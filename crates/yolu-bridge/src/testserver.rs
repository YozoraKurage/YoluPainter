//! 自己診断のスタンドアロン: ブリッジと同じプロセスの中で待ち受け、モデルが来るとマテリアルごとに試しの模様のテクスチャセットを返す。
//! Unity の試験（外のプロセスを起こさずに、本物のソケットと共有メモリを通す）と、スタンドアロン無しでの確かめに使う。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use yolu_protocol::host::PublishedSet;
use yolu_protocol::link::{accept, error_message, wrong_direction};
use yolu_protocol::*;

/// 自己診断のスタンドアロンが受けたものの数。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct YlbTestServerStats {
    pub connections: u32,
    pub models: u32,
    pub poses: u32,
    /// 受けたポーズのメッシュの数の合計。
    pub pose_meshes: u32,
    /// 最後のモデルの世代・マテリアルの数・メッシュの数・頂点の数の合計。
    pub generation: u32,
    pub materials: u32,
    pub meshes: u32,
    pub vertices: u32,
    pub models_closed: u32,
    /// 断った（古い世代のポーズなど）・知らない・読めない命令の数。
    pub refused: u32,
    pub unknown: u32,
    /// 最後のポーズの、一番目のメッシュの一番目の頂点の位置（試験で見る）。
    pub last_pose_x: f32,
    pub last_pose_y: f32,
    pub last_pose_z: f32,
    /// 受けたマテリアルの更新の数と、最後の更新のマテリアルの数・流し込み先の数の合計・一番目のマテリアルのシェーダー名の長さ。
    pub materials_updates: u32,
    pub last_materials_count: u32,
    pub last_materials_routes: u32,
    pub last_materials_shader_len: u32,
}

const PALETTE: [[u8; 3]; 8] = [
    [230, 60, 60],
    [60, 160, 230],
    [80, 200, 90],
    [240, 200, 40],
    [170, 80, 220],
    [240, 130, 40],
    [60, 200, 200],
    [200, 200, 200],
];

/// 試しの模様（マテリアルごとの色の市松。奇数のタイルは半分の明るさ。不透明）。
pub fn pattern(material: u32, tile_x: u32, tile_y: u32) -> [u8; 4] {
    let [r, g, b] = PALETTE[material as usize % PALETTE.len()];
    if (tile_x + tile_y).is_multiple_of(2) {
        [r, g, b, 255]
    } else {
        [r / 2, g / 2, b / 2, 255]
    }
}

struct Shared {
    conn: Option<Connection>,
    session: u64,
    generation: u32,
    next_set: u32,
    sets: Vec<PublishedSet>,
    stats: YlbTestServerStats,
}

pub struct TestServer {
    name: String,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

fn lock(m: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl TestServer {
    pub fn start(name: &str, size: u32, tile_size: u32) -> std::io::Result<TestServer> {
        let listener = Server::bind(name, true)?;
        let key = listener.key();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Shared {
            conn: None,
            session: 0,
            generation: 0,
            next_set: 0,
            sets: Vec::new(),
            stats: YlbTestServerStats::default(),
        }));
        let (s, sh) = (stop.clone(), shared.clone());
        let thread = thread::Builder::new()
            .name("yolu-bridge-testserver".into())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    let stream = match listener.accept() {
                        Ok(stream) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(_) => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                    };
                    let session = {
                        let mut g = lock(&sh);
                        g.stats.connections += 1;
                        g.session += 1;
                        g.session
                    };
                    let Ok((conn, mut reader, _)) =
                        accept(stream, "YoluPainter bridge test server", session, &key)
                    else {
                        continue;
                    };
                    lock(&sh).conn = Some(conn.clone());
                    reader.set_timeout(Some(Duration::from_millis(100)));
                    while !s.load(Ordering::Relaxed) {
                        match reader.next(&conn) {
                            Ok(Received::Idle) => continue,
                            Ok(Received::Message(Message::Bye)) | Err(_) => break,
                            Ok(Received::Message(m)) => {
                                if let Some(reply) = wrong_direction(&m, true) {
                                    let _ = conn.send(&reply);
                                    lock(&sh).stats.refused += 1;
                                    continue;
                                }
                                handle(&sh, &conn, m, size, tile_size);
                            }
                            Ok(Received::Unknown(_)) | Ok(Received::Malformed(..)) => {
                                lock(&sh).stats.unknown += 1
                            }
                        }
                    }
                    let mut g = lock(&sh);
                    g.conn = None;
                    g.sets.clear();
                }
            })?;
        Ok(TestServer {
            name: name.to_owned(),
            stop,
            shared,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Linux は読みの時間切れで止まる。Windows は相手が切るまで待つことがあるので、待たずに手放す
        if cfg!(unix) {
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
        lock(&self.shared).sets.clear();
    }

    /// 鍵のファイルを別の鍵に差し替える（古い・別の待ち受けの鍵が残っている状態。つなぐブリッジは鍵の断りを受ける）。
    pub fn replace_key(&self) -> i32 {
        let replaced = yolu_protocol::auth::key_path(&self.name)
            .and_then(|path| yolu_protocol::LinkKey::generate()?.write_file(&path));
        if replaced.is_ok() {
            0
        } else {
            crate::ffi::YLB_E_STATE
        }
    }

    pub fn stats(&self) -> YlbTestServerStats {
        lock(&self.shared).stats
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        material: u32,
        channel: u8,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        rgba: [u8; 4],
    ) -> i32 {
        let mut g = lock(&self.shared);
        let Some(conn) = g.conn.clone() else {
            return crate::ffi::YLB_E_STATE;
        };
        let Some(set) = g.sets.iter_mut().find(|s| s.material == material) else {
            return crate::ffi::YLB_E_ARGUMENT;
        };
        let ts = set.tile_size();
        let (tx, ty) = (set.width().div_ceil(ts), set.height().div_ceil(ts));
        let Some(img) = set.image_mut(channel) else {
            return crate::ffi::YLB_E_ARGUMENT;
        };
        let mut tiles = Vec::new();
        for y in y0..y1.min(ty) {
            for x in x0..x1.min(tx) {
                let _ = img.write_tile(x, y, |slot| {
                    for p in slot.chunks_exact_mut(4) {
                        p.copy_from_slice(&rgba);
                    }
                });
                tiles.push(Tile {
                    x: x as u16,
                    y: y as u16,
                });
            }
        }
        for m in set.tiles_changed(channel, &tiles) {
            if conn.send(&m).is_err() {
                return crate::ffi::YLB_E_STATE;
            }
        }
        tiles.len() as i32
    }
}

fn handle(shared: &Mutex<Shared>, conn: &Connection, message: Message, size: u32, tile_size: u32) {
    let mut g = lock(shared);
    match message {
        Message::Model(m) => {
            g.stats.models += 1;
            g.stats.generation = m.generation;
            g.stats.materials = m.materials.len() as u32;
            g.stats.meshes = m.meshes.len() as u32;
            g.stats.vertices = m.meshes.iter().map(|x| x.positions.len() as u32).sum();
            g.generation = m.generation;
            // 前のモデルのセットを片付ける
            for old in std::mem::take(&mut g.sets) {
                let _ = conn.send(&Message::TextureSetRemoved { set: old.set });
            }
            let session = g.session;
            for (i, mat) in m.materials.iter().enumerate() {
                let mut channels = vec![channel::COLOR];
                for r in &mat.routes {
                    if !channels.contains(&r.channel) {
                        channels.push(r.channel);
                    }
                }
                let name = match &mat.key {
                    MaterialKey::Unassigned => "Unassigned".to_owned(),
                    MaterialKey::Material { name, .. } => name.clone(),
                };
                g.next_set = g.next_set.wrapping_add(1);
                let Ok(mut set) = PublishedSet::create(
                    session,
                    g.next_set,
                    m.generation,
                    i as u32,
                    &name,
                    size,
                    size,
                    tile_size,
                    &channels,
                ) else {
                    let _ = conn.send(&error_message(
                        ErrorCode::Other,
                        Kind::Model as u16,
                        format!("共有メモリを作れません（{name}）"),
                    ));
                    continue;
                };
                let (tx, ty) = (size.div_ceil(tile_size), size.div_ceil(tile_size));
                let mut tiles = Vec::new();
                for &ch in &channels {
                    let img = set.image_mut(ch).unwrap();
                    for y in 0..ty {
                        for x in 0..tx {
                            let px = if ch == channel::COLOR {
                                pattern(i as u32, x, y)
                            } else {
                                [200, 200, 200, 255]
                            };
                            let _ = img.write_tile(x, y, |slot| {
                                for p in slot.chunks_exact_mut(4) {
                                    p.copy_from_slice(&px);
                                }
                            });
                            if ch == channel::COLOR {
                                tiles.push(Tile {
                                    x: x as u16,
                                    y: y as u16,
                                });
                            }
                        }
                    }
                }
                let _ = conn.send(&set.announce());
                // 知らせた時の中身はブリッジが全部写すが、描いた所を知らせる流れも通す
                for msg in set.tiles_changed(channel::COLOR, &tiles) {
                    let _ = conn.send(&msg);
                }
                g.sets.push(set);
            }
        }
        Message::Pose(p) => {
            if p.generation != g.generation {
                g.stats.refused += 1;
                let _ = conn.send(&error_message(
                    ErrorCode::Refused,
                    Kind::Pose as u16,
                    "古い世代のポーズです".into(),
                ));
                return;
            }
            g.stats.poses += 1;
            g.stats.pose_meshes += p.meshes.len() as u32;
            if let Some(first) = p.meshes.first().and_then(|m| m.positions.first()) {
                g.stats.last_pose_x = first[0];
                g.stats.last_pose_y = first[1];
                g.stats.last_pose_z = first[2];
            }
        }
        Message::ModelClosed { generation } => {
            if generation == g.generation {
                g.stats.models_closed += 1;
                for old in std::mem::take(&mut g.sets) {
                    let _ = conn.send(&Message::TextureSetRemoved { set: old.set });
                }
            }
        }
        Message::Materials(m) => {
            if m.generation != g.generation || m.materials.len() != g.stats.materials as usize {
                g.stats.refused += 1;
                let _ = conn.send(&error_message(
                    ErrorCode::Refused,
                    Kind::Materials as u16,
                    "古い世代か、数の違うマテリアルの更新です".into(),
                ));
                return;
            }
            g.stats.materials_updates += 1;
            g.stats.last_materials_count = m.materials.len() as u32;
            g.stats.last_materials_routes = m.materials.iter().map(|x| x.routes.len() as u32).sum();
            g.stats.last_materials_shader_len = m
                .materials
                .first()
                .map(|x| x.shader.len() as u32)
                .unwrap_or(0);
        }
        Message::Error(_) => {}
        _ => {}
    }
}
