//! 自己診断のスタンドアロン: ブリッジと同じプロセスの中で待ち受け、モデルが来るとマテリアルごとに試しの模様のテクスチャセットを返す。
//! Unity の試験（外のプロセスを起こさずに、本物のソケットと共有メモリを通す）と、スタンドアロン無しでの確かめに使う。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use yolu_protocol::host::PublishedSet;
use yolu_protocol::link::{accept_as, error_message, wrong_direction, HANDSHAKE_TIMEOUT};
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
    /// 受けたマテリアルの値（MaterialValues）の数と、最後の値のマテリアルの番号・種類（0 値なし・1 lilToon）・プロパティの数・
    /// キーワードの数・スロットの数・シェーダー名の長さ。
    pub values: u32,
    pub last_values_material: u32,
    pub last_values_kind: u32,
    pub last_values_properties: u32,
    pub last_values_keywords: u32,
    pub last_values_slots: u32,
    pub last_values_shader_len: u32,
    /// 受けた描いていないスロットの絵（MaterialTexture）の数と、その画素のバイトの合計（KiB、切り上げ）。
    pub textures: u32,
    pub texture_kib: u32,
    /// 受けた元の絵（MaterialOriginal）の数（絵の付かない様子も数える）と、絵の付いたものの画素のバイトの合計（KiB、切り上げ）。
    pub originals: u32,
    pub original_kib: u32,
    /// 元の絵が揃うまで出さずに待たせているセットの数（機能の印 ORIGINAL_TEXTURES を名乗っているときだけ待たせる）。
    pub held_sets: u32,
    /// 送った頼み（MaterialRequest。印 MATERIAL_REQUEST が双方にあるとき、待たせたセットの元の絵を自分から頼む分と、試験が頼ませた分）の数。
    pub requests: u32,
    /// 手元の絵を使った数（Cached の印が手元の絵と合った）と、使えなかった数（手元に無い・印か大きさが違う。頼み直す）。
    pub cached_used: u32,
    pub cached_missed: u32,
    /// 最後に送った頼みの、項目の数。
    pub last_request_items: u32,
    /// 最後に受けた元の絵（線の上で受けた様子。手元の絵に置き換える前）の様子（0 絵が付く・1 読めない・2 大きすぎる・3 予算・4 手元の絵を使う）と
    /// マテリアルの番号。
    pub last_original_state: u32,
    pub last_original_material: u32,
}

/// 自己診断のスタンドアロンが受けた、元の絵 1 つの様子（`ylb_test_server_original`）。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct YlbTestServerOriginal {
    /// 0 絵が付く・1 読めない・2 辺が上限を超える・3 予算を超える・4 手元の絵を使う（画素なし）。
    pub state: u32,
    /// 0 原本のファイル・1 取り込んだ絵・2 GPU を通して。
    pub read: u32,
    /// 0 でなければ圧縮されたテクスチャから読んだ。
    pub compressed: u32,
    /// 0 でなければ sRGB。
    pub srgb: u32,
    pub width: u32,
    pub height: u32,
    /// 真ん中の画素（(幅 / 2, 高さ / 2)。行は下から）と、一番下の左の画素の RGBA を r | g << 8 | b << 16 | a << 24 に詰めたもの（絵が付かなければ 0）。
    pub center: u32,
    pub corner: u32,
    /// 絵の印（0 は印なし）。
    pub stamp: u64,
}

/// 自己診断のスタンドアロンが受けた、描いていないスロットの絵 1 つの様子（`ylb_test_server_texture`）。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct YlbTestServerTexture {
    pub width: u32,
    pub height: u32,
    /// 0 でなければ sRGB。
    pub srgb: u32,
    /// 真ん中の画素（(幅 / 2, 高さ / 2)。行は下から）の RGBA を r | g << 8 | b << 16 | a << 24 に詰めたもの。
    pub center: u32,
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
    /// 挨拶で名乗る内容（`configure` で替える。次につなぐブリッジから効く）。
    identity: Identity,
    conn: Option<Connection>,
    session: u64,
    generation: u32,
    next_set: u32,
    sets: Vec<PublishedSet>,
    stats: YlbTestServerStats,
    /// マテリアルの番号ごとの最後の値と、(番号, スロット) ごとの最後の絵（試験が名前で引く）。
    values: std::collections::BTreeMap<u32, MaterialValues>,
    textures: std::collections::BTreeMap<(u32, String), MaterialTexture>,
    /// (番号, スロット) ごとの最後の元の絵。
    originals: std::collections::BTreeMap<(u32, String), MaterialOriginal>,
    /// 元の絵が揃うまで出さないセット（マテリアルの番号ごと。スタンドアロンが、新しく作ったセットを元の絵が入るまで Unity に出さないのと同じ）。
    held: std::collections::BTreeMap<u32, Held>,
    /// 今のモデルのマテリアルの名前（番号順）。
    names: Vec<String>,
    /// 手元の元の絵（マテリアルの名前・スロットごと。印の付いた絵だけ。モデルを替えても残す）。頼みの `have` と、Cached の答えに使う。
    cache: std::collections::BTreeMap<(String, String), MaterialOriginal>,
}

/// 元の絵を待たせているセット 1 つ。
struct Held {
    name: String,
    channels: Vec<u8>,
    /// まだ来ていないスロット。
    expected: std::collections::BTreeSet<String>,
    /// 来た元の絵（絵が付いたものだけ。Color の模様の代わりにする）。
    image: Option<MaterialOriginal>,
}

pub struct TestServer {
    name: String,
    stop: Arc<AtomicBool>,
    /// 読むのを止めている間は、つながりから何も読まない（試験用。相手が読まないときのブリッジの送りの列を確かめる）。
    paused: Arc<AtomicBool>,
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
            identity: Identity::standalone("YoluPainter bridge test server")
                .with_version(AppVersion::parse(env!("CARGO_PKG_VERSION"))),
            conn: None,
            session: 0,
            generation: 0,
            next_set: 0,
            sets: Vec::new(),
            stats: YlbTestServerStats::default(),
            values: Default::default(),
            textures: Default::default(),
            originals: Default::default(),
            held: Default::default(),
            names: Vec::new(),
            cache: Default::default(),
        }));
        let paused = Arc::new(AtomicBool::new(false));
        let (s, sh, pause) = (stop.clone(), shared.clone(), paused.clone());
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
                    let (session, identity) = {
                        let mut g = lock(&sh);
                        g.stats.connections += 1;
                        g.session += 1;
                        (g.session, g.identity.clone())
                    };
                    let Ok((conn, mut reader, _)) = accept_as(
                        stream,
                        &identity,
                        session,
                        &key,
                        HANDSHAKE_TIMEOUT,
                        &|_| Ok(()),
                    ) else {
                        continue;
                    };
                    lock(&sh).conn = Some(conn.clone());
                    reader.set_timeout(Some(Duration::from_millis(100)));
                    while !s.load(Ordering::Relaxed) {
                        if pause.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
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
                    g.held.clear();
                    g.stats.held_sets = 0;
                }
            })?;
        Ok(TestServer {
            name: name.to_owned(),
            stop,
            paused,
            shared,
            thread: Some(thread),
        })
    }

    /// つながりから読むのを止める・再開する（止めている間も、止める合図には応える）。
    pub fn pause_reading(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
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

    /// 挨拶で名乗る版・求める相手の版・機能の印を決める（版が None なら版を名乗らない古い役）。読めるプロトコルの版の範囲は変えない。
    pub fn configure(&self, app_version: Option<AppVersion>, min_peer: AppVersion, features: u64) {
        let mut g = lock(&self.shared);
        let (min, max) = g.identity.protocol_range();
        g.identity = Identity::standalone("YoluPainter bridge test server")
            .with_version(app_version)
            .with_min_peer(min_peer)
            .with_features(features)
            .with_protocol_range(min, max);
    }

    /// 読めるプロトコルの版の範囲を決める（つなぐブリッジの範囲と重ならなければ、版の範囲の断りを返す）。
    pub fn set_protocol_range(&self, min: u16, max: u16) {
        let mut g = lock(&self.shared);
        g.identity = g.identity.clone().with_protocol_range(min, max);
    }

    pub fn stats(&self) -> YlbTestServerStats {
        lock(&self.shared).stats
    }

    /// 最後に受けた、マテリアル `material` の値のプロパティ `name`（無ければ None）。
    pub fn value(&self, material: u32, name: &str) -> Option<PropertyValue> {
        lock(&self.shared)
            .values
            .get(&material)?
            .properties
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.value)
    }

    /// 最後に受けた、マテリアル `material` の値のスロット `name` の様子（無ければ None）。
    pub fn slot(&self, material: u32, name: &str) -> Option<SlotState> {
        lock(&self.shared)
            .values
            .get(&material)?
            .slots
            .iter()
            .find(|x| x.name == name)
            .map(|x| x.state)
    }

    /// 最後に受けた、マテリアル `material` のキーワード `name` があるか。
    pub fn has_keyword(&self, material: u32, name: &str) -> bool {
        lock(&self.shared)
            .values
            .get(&material)
            .is_some_and(|v| v.keywords.iter().any(|k| k == name))
    }

    /// 最後に受けた、マテリアル `material` のスロット `slot` の元の絵の様子。
    pub fn original(&self, material: u32, slot: &str) -> Option<YlbTestServerOriginal> {
        let g = lock(&self.shared);
        let o = g.originals.get(&(material, slot.to_owned()))?;
        let pixel = |x: u32, y: u32| {
            if o.pixels.is_empty() {
                return 0;
            }
            let at = (y as usize * o.width as usize + x as usize) * 4;
            u32::from_le_bytes([o.pixels[at], o.pixels[at + 1], o.pixels[at + 2], o.pixels[at + 3]])
        };
        Some(YlbTestServerOriginal {
            state: o.state as u32,
            read: o.read as u32,
            compressed: o.compressed as u32,
            srgb: o.srgb as u32,
            width: o.width,
            height: o.height,
            center: pixel(o.width / 2, o.height / 2),
            corner: pixel(0, 0),
            stamp: o.stamp,
        })
    }

    /// 最後に受けた、マテリアル `material` のスロット `slot` の絵の様子。
    pub fn texture(&self, material: u32, slot: &str) -> Option<YlbTestServerTexture> {
        let g = lock(&self.shared);
        let t = g.textures.get(&(material, slot.to_owned()))?;
        let (x, y) = (t.width / 2, t.height / 2);
        let at = (y as usize * t.width as usize + x as usize) * 4;
        let p = &t.pixels[at..at + 4];
        Some(YlbTestServerTexture {
            width: t.width,
            height: t.height,
            srgb: t.srgb as u32,
            center: u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
        })
    }

    /// 頼みを 1 つ送る（スタンドアロンが Unity に頼む。`generation` が 0 なら今のモデルの世代）。相手に印が無い・つながっていないなら負。
    pub fn request(
        &self,
        generation: u32,
        material: u32,
        wants: u8,
        slot: &str,
        have: u64,
    ) -> i32 {
        let mut g = lock(&self.shared);
        let Some(conn) = g.conn.clone() else {
            return crate::ffi::YLB_E_STATE;
        };
        let generation = if generation == 0 { g.generation } else { generation };
        let message = Message::MaterialRequest(MaterialRequest {
            generation,
            items: vec![MaterialWant {
                material,
                wants,
                slot: slot.to_owned(),
                have,
            }],
        });
        match conn.send_gated(&message) {
            Ok(true) => {
                g.stats.requests += 1;
                g.stats.last_request_items = 1;
                0
            }
            Ok(false) => crate::ffi::YLB_E_STATE,
            Err(_) => crate::ffi::YLB_E_STATE,
        }
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
            g.values.clear();
            g.textures.clear();
            // 前のモデルのセットを片付ける
            for old in std::mem::take(&mut g.sets) {
                let _ = conn.send(&Message::TextureSetRemoved { set: old.set });
            }
            g.originals.clear();
            g.held.clear();
            g.stats.held_sets = 0;
            g.names = m
                .materials
                .iter()
                .map(|mat| match &mat.key {
                    MaterialKey::Unassigned => "Unassigned".to_owned(),
                    MaterialKey::Material { name, .. } => name.clone(),
                })
                .collect();
            let mut wanted: Vec<MaterialWant> = Vec::new();
            // 元の絵を送ると名乗る相手とつながっているときは、Color の流し込み先に絵の入っているマテリアルのセットを、元の絵が揃うまで
            // 出さない（スタンドアロンが、新しく作ったセットを元の絵が入るまで Unity に出さないのと同じ。出た後の Color は元の絵）
            let waits = conn
                .link_info()
                .is_some_and(|l| l.common_features() & feature::ORIGINAL_TEXTURES != 0);
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
                let expected: std::collections::BTreeSet<String> = mat
                    .routes
                    .iter()
                    .filter(|r| r.channel == channel::COLOR)
                    .filter(|r| {
                        mat.textures
                            .iter()
                            .any(|t| t.name == r.property && t.width > 0 && t.height > 0)
                    })
                    .map(|r| r.property.clone())
                    .collect();
                if waits && !expected.is_empty() {
                    for slot in &expected {
                        let have = g.cache.get(&(name.clone(), slot.clone())).map_or(0, |o| o.stamp);
                        wanted.push(MaterialWant::original(i as u32, slot.clone(), have));
                    }
                    g.held.insert(
                        i as u32,
                        Held {
                            name,
                            channels,
                            expected,
                            image: None,
                        },
                    );
                    g.stats.held_sets = g.held.len() as u32;
                    continue;
                }
                publish_set(&mut g, conn, i as u32, &name, &channels, size, tile_size, None);
            }
            // 印が双方にあれば、待たせたセットの元の絵を自分から頼む（実際のスタンドアロンと同じ。Unity は頼まれない元の絵を送らない）
            if !wanted.is_empty() && conn.common_features() & feature::MATERIAL_REQUEST != 0 {
                let items = wanted.len() as u32;
                let message = Message::MaterialRequest(MaterialRequest {
                    generation: m.generation,
                    items: wanted,
                });
                if conn.send_gated(&message).unwrap_or(false) {
                    g.stats.requests += 1;
                    g.stats.last_request_items = items;
                }
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
                g.held.clear();
                g.stats.held_sets = 0;
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
        Message::MaterialValues(v) => {
            if v.generation != g.generation || v.material >= g.stats.materials {
                g.stats.refused += 1;
                let _ = conn.send(&error_message(
                    ErrorCode::Refused,
                    Kind::MaterialValues as u16,
                    "古い世代か、無いマテリアルの値です".into(),
                ));
                return;
            }
            g.stats.values += 1;
            g.stats.last_values_material = v.material;
            g.stats.last_values_kind = v.kind as u32;
            g.stats.last_values_properties = v.properties.len() as u32;
            g.stats.last_values_keywords = v.keywords.len() as u32;
            g.stats.last_values_slots = v.slots.len() as u32;
            g.stats.last_values_shader_len = v.shader.len() as u32;
            g.values.insert(v.material, v);
        }
        Message::MaterialTexture(t) => {
            if t.generation != g.generation || t.material >= g.stats.materials {
                g.stats.refused += 1;
                return;
            }
            g.stats.textures += 1;
            g.stats.texture_kib = g
                .stats
                .texture_kib
                .saturating_add(t.pixels.len().div_ceil(1024) as u32);
            g.textures.insert((t.material, t.slot.clone()), t);
        }
        Message::MaterialOriginal(o) => {
            if o.generation != g.generation || o.material >= g.stats.materials {
                g.stats.refused += 1;
                let _ = conn.send(&error_message(
                    ErrorCode::Refused,
                    Kind::MaterialOriginal as u16,
                    "古い世代か、無いマテリアルの元の絵です".into(),
                ));
                return;
            }
            let name = g.names.get(o.material as usize).cloned().unwrap_or_default();
            g.stats.last_original_state = o.state as u32;
            g.stats.last_original_material = o.material;
            // 線の上の画素のバイト（手元の絵に置き換える前）
            let wire_pixels = o.pixels.len();
            // 手元の絵を使う答え: 印と大きさが手元の絵と合えば、その画素を使う。合わなければ使わず、印なしで頼み直す
            let mut o = o;
            if o.state == OriginalState::Cached {
                let hit = g
                    .cache
                    .get(&(name.clone(), o.slot.clone()))
                    .filter(|c| c.stamp == o.stamp && (c.width, c.height) == (o.width, o.height))
                    .cloned();
                match hit {
                    Some(c) => {
                        g.stats.cached_used += 1;
                        o = MaterialOriginal {
                            generation: o.generation,
                            material: o.material,
                            ..c
                        };
                    }
                    None => {
                        g.stats.cached_missed += 1;
                        let again = Message::MaterialRequest(MaterialRequest {
                            generation: o.generation,
                            items: vec![MaterialWant::original(o.material, o.slot.clone(), 0)],
                        });
                        if conn.send_gated(&again).unwrap_or(false) {
                            g.stats.requests += 1;
                            g.stats.last_request_items = 1;
                        }
                        return;
                    }
                }
            } else if o.state == OriginalState::Image && o.stamp != 0 {
                if g.cache.len() >= 64 {
                    g.cache.pop_first();
                }
                g.cache.insert((name.clone(), o.slot.clone()), o.clone());
            }
            g.stats.originals += 1;
            g.stats.original_kib = g
                .stats
                .original_kib
                .saturating_add(wire_pixels.div_ceil(1024) as u32);
            let (material, slot) = (o.material, o.slot.clone());
            g.originals.insert((material, slot.clone()), o.clone());
            let done = match g.held.get_mut(&material) {
                Some(h) => {
                    if h.expected.remove(&slot) && o.state == OriginalState::Image {
                        h.image = Some(o);
                    }
                    h.expected.is_empty()
                }
                None => false,
            };
            if done {
                let h = g.held.remove(&material).expect("上で見た");
                g.stats.held_sets = g.held.len() as u32;
                publish_set(
                    &mut g,
                    conn,
                    material,
                    &h.name,
                    &h.channels,
                    size,
                    tile_size,
                    h.image.as_ref(),
                );
            }
        }
        Message::Error(_) => {}
        _ => {}
    }
}

/// マテリアル `material` のセットを作って知らせる。Color は `original` があればその絵（最近傍で size × size に合わせる）、無ければ試しの模様、
/// ほかのチャンネルは灰色。
#[allow(clippy::too_many_arguments)]
fn publish_set(
    g: &mut Shared,
    conn: &Connection,
    material: u32,
    name: &str,
    channels: &[u8],
    size: u32,
    tile_size: u32,
    original: Option<&MaterialOriginal>,
) {
    g.next_set = g.next_set.wrapping_add(1);
    let Ok(mut set) = PublishedSet::create(
        g.session,
        g.next_set,
        g.generation,
        material,
        name,
        size,
        size,
        tile_size,
        channels,
    ) else {
        let _ = conn.send(&error_message(
            ErrorCode::Other,
            Kind::Model as u16,
            format!("共有メモリを作れません（{name}）"),
        ));
        return;
    };
    let (tx, ty) = (size.div_ceil(tile_size), size.div_ceil(tile_size));
    let mut tiles = Vec::new();
    for &ch in channels {
        let img = set.image_mut(ch).unwrap();
        for y in 0..ty {
            for x in 0..tx {
                let _ = img.write_tile(x, y, |slot| match (ch, original) {
                    (channel::COLOR, Some(o)) => {
                        for (i, p) in slot.chunks_exact_mut(4).enumerate() {
                            let (px, py) = (
                                x * tile_size + i as u32 % tile_size,
                                y * tile_size + i as u32 / tile_size,
                            );
                            let (sx, sy) = (
                                (px as u64 * o.width as u64 / size as u64) as usize,
                                (py as u64 * o.height as u64 / size as u64) as usize,
                            );
                            let at = (sy * o.width as usize + sx) * 4;
                            p.copy_from_slice(&o.pixels[at..at + 4]);
                        }
                    }
                    _ => {
                        let px = if ch == channel::COLOR {
                            pattern(material, x, y)
                        } else {
                            [200, 200, 200, 255]
                        };
                        for p in slot.chunks_exact_mut(4) {
                            p.copy_from_slice(&px);
                        }
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
