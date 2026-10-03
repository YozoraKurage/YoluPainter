//! Live Link の試しのスタンドアロン（yolu-app が載るまでの代わり）。Unity のブリッジを待ち受け、受けたモデルのマテリアルの組ごとに
//! yolu-core の文書を 1 つ持ち、試しの模様を描いて、描いた所（変わったタイル）だけを共有メモリへ合成して知らせる。
//!
//! 使い方: `yolu-link-demo [--name 名前] [--size N] [--animate] [--bench] [--once]`
//! - `--name`: つなぎ先の名前（既定 yolupainter-livelink）。
//! - `--size`: テクスチャセットの大きさ（既定はマテリアルの流し込み先のテクスチャの大きさ、無ければ 1024。256〜4096 に丸める）。
//! - `--animate`: つながっている間、模様の上に線を描き足し続ける（変わったタイルだけを返す流れを見る）。
//! - `--bench`: 1 つ目のセットで、1 秒ごとに「全面」と「1 タイル」を交互に書き換える（共有メモリ → Unity の上げの時間を測る）。
//! - `--once`: 1 つのつながりが終わったら終わる（試験用）。

// 画素（4 バイト）を chunks_exact で回すのは読みやすさのため（yolu-core と同じ）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Listener as _;
use yolu_core::glam::DVec2;
use yolu_core::{BrushSettings, Channel, Document, LayerId, Rgba8, RowOrder, TileCoord};
use yolu_protocol::host::PublishedSet;
use yolu_protocol::link::{self, accept, wrong_direction};
use yolu_protocol::*;

struct Args {
    name: String,
    size: Option<u32>,
    animate: bool,
    bench: bool,
    once: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        name: DEFAULT_LINK_NAME.to_owned(),
        size: None,
        animate: false,
        bench: false,
        once: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" => a.name = it.next().ok_or("--name の後に名前")?,
            "--size" => {
                a.size = Some(
                    it.next()
                        .and_then(|s| s.parse().ok())
                        .ok_or("--size の後に数")?,
                )
            }
            "--animate" => a.animate = true,
            "--bench" => a.bench = true,
            "--once" => a.once = true,
            "-h" | "--help" => {
                println!("yolu-link-demo [--name 名前] [--size N] [--animate] [--bench] [--once]");
                std::process::exit(0);
            }
            other => return Err(format!("知らない引数: {other}")),
        }
    }
    Ok(a)
}

const PALETTE: [[u8; 3]; 6] = [
    [220, 50, 60],
    [40, 120, 220],
    [40, 170, 80],
    [230, 170, 20],
    [150, 60, 200],
    [20, 170, 170],
];

/// マテリアル 1 つの文書と、その共有メモリ。
struct Painter {
    doc: Document,
    ink: LayerId,
    since: u64,
    set: PublishedSet,
    color: Rgba8,
    t: f64,
}

impl Painter {
    fn new(
        session: u64,
        set_id: u32,
        generation: u32,
        material: u32,
        name: &str,
        size: u32,
    ) -> Result<Painter, String> {
        let mut doc = Document::new(size, size).map_err(|e| e.to_string())?;
        let base = doc.add_layer("下地").map_err(|e| e.to_string())?;
        let ink = doc.add_layer("模様").map_err(|e| e.to_string())?;
        let [r, g, b] = PALETTE[material as usize % PALETTE.len()];
        let color = Rgba8::new(r, g, b, 255);
        // 下地: マテリアルの色を薄めた不透明の色（タイルを丸ごと読み込む）
        let ts = doc.tile_size();
        let tint = [
            (255 - (255 - r as u32) / 4) as u8,
            (255 - (255 - g as u32) / 4) as u8,
            (255 - (255 - b as u32) / 4) as u8,
            255,
        ];
        let tile: Vec<u8> = std::iter::repeat_n(tint, (ts * ts) as usize)
            .flatten()
            .collect();
        for y in 0..size.div_ceil(ts) {
            for x in 0..size.div_ceil(ts) {
                doc.import_tile(base, Channel::Color, TileCoord::new(x, y), &tile)
                    .map_err(|e| e.to_string())?;
            }
        }
        // 模様: UV の 8 等分の格子と、対角の線
        let s = size as f64;
        let thin = BrushSettings {
            radius: (s / 512.0).max(1.5),
            hardness: 0.9,
            color: Rgba8::new(r / 2, g / 2, b / 2, 255),
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        };
        for i in 0..=8 {
            let v = (i as f64 / 8.0 * s).clamp(1.0, s - 1.0);
            line(&mut doc, ink, &thin, (v, 1.0), (v, s - 1.0))?;
            line(&mut doc, ink, &thin, (1.0, v), (s - 1.0, v))?;
        }
        let thick = BrushSettings {
            radius: (s / 64.0).max(3.0),
            color,
            ..thin
        };
        line(
            &mut doc,
            ink,
            &thick,
            (s * 0.1, s * 0.1),
            (s * 0.9, s * 0.9),
        )?;
        let set = PublishedSet::create(
            session,
            set_id,
            generation,
            material,
            name,
            size,
            size,
            ts,
            &[channel::COLOR],
        )
        .map_err(|e| e.to_string())?;
        Ok(Painter {
            doc,
            ink,
            since: 0,
            set,
            color,
            t: 0.0,
        })
    }

    /// 前に知らせた後に変わったタイルだけを合成して共有メモリへ書き、知らせる。返すのは書いたタイルの数。
    fn publish(&mut self, conn: &Connection) -> Result<usize, String> {
        let coords = match self.doc.changed_tiles(Channel::Color, self.since) {
            Some(c) => c,
            None => {
                let ts = self.doc.tile_size();
                let (tx, ty) = (
                    self.doc.width().div_ceil(ts),
                    self.doc.height().div_ceil(ts),
                );
                (0..ty)
                    .flat_map(|y| (0..tx).map(move |x| TileCoord::new(x, y)))
                    .collect()
            }
        };
        self.since = self.doc.change_serial();
        if coords.is_empty() {
            return Ok(0);
        }
        let ts = self.doc.tile_size() as usize;
        let mut buf = Vec::new();
        let mut tiles = Vec::with_capacity(coords.len());
        for c in &coords {
            let Some(rect) = self.doc.tile_rect(*c) else {
                continue;
            };
            buf.resize(rect.width as usize * rect.height as usize * 4, 0);
            self.doc
                .composite_into(Channel::Color, rect, &mut buf, RowOrder::BottomUp)
                .map_err(|e| e.to_string())?;
            let img = self.set.image_mut(channel::COLOR).expect("Color を作った");
            img.write_tile(c.x, c.y, |slot| {
                let row = rect.width as usize * 4;
                for r in 0..rect.height as usize {
                    slot[r * ts * 4..r * ts * 4 + row]
                        .copy_from_slice(&buf[r * row..(r + 1) * row]);
                }
            })
            .map_err(|e| e.to_string())?;
            tiles.push(Tile {
                x: c.x as u16,
                y: c.y as u16,
            });
        }
        for m in self.set.tiles_changed(channel::COLOR, &tiles) {
            conn.send(&m).map_err(|e| e.to_string())?;
        }
        Ok(tiles.len())
    }

    /// リサージュの曲線に沿って線を少し描き足す。
    fn animate(&mut self) -> Result<(), String> {
        let s = self.doc.width() as f64;
        let at = |t: f64| {
            (
                s * (0.5 + 0.4 * (3.0 * t).sin()),
                s * (0.5 + 0.4 * (2.0 * t).cos()),
            )
        };
        let brush = BrushSettings {
            radius: (s / 128.0).max(2.0),
            hardness: 0.7,
            color: Rgba8::new(
                255 - self.color.r,
                255 - self.color.g,
                255 - self.color.b,
                255,
            ),
            pressure_size: false,
            ..BrushSettings::default()
        };
        let (a, b) = (at(self.t), at(self.t + 0.05));
        self.t += 0.05;
        line(&mut self.doc, self.ink, &brush, a, b)
    }
}

fn line(
    doc: &mut Document,
    layer: LayerId,
    brush: &BrushSettings,
    a: (f64, f64),
    b: (f64, f64),
) -> Result<(), String> {
    let mut stroke = doc.begin_stroke(layer, brush).map_err(|e| e.to_string())?;
    stroke
        .add_point(doc, a.0, a.1, 1.0, DVec2::ZERO)
        .map_err(|e| e.to_string())?;
    stroke
        .add_point(doc, b.0, b.1, 1.0, DVec2::ZERO)
        .map_err(|e| e.to_string())?;
    doc.end_stroke(stroke).map_err(|e| e.to_string())?;
    Ok(())
}

/// マテリアルの大きさ: 流し込み先（Color）のテクスチャの大きさ、無ければ 1024。256〜4096 に丸めて 2 の冪へ。
fn size_for(info: &MaterialInfo, forced: Option<u32>) -> u32 {
    let wanted = forced.unwrap_or_else(|| {
        let prop = info
            .routes
            .iter()
            .find(|r| r.channel == channel::COLOR)
            .map(|r| r.property.as_str());
        info.textures
            .iter()
            .find(|t| Some(t.name.as_str()) == prop)
            .map(|t| t.width.max(t.height))
            .filter(|&w| w > 0)
            .unwrap_or(1024)
    });
    wanted.clamp(256, 4096).next_power_of_two().min(4096)
}

fn material_name(info: &MaterialInfo) -> String {
    match &info.key {
        MaterialKey::Unassigned => "Unassigned".into(),
        MaterialKey::Material { name, .. } => name.clone(),
    }
}

fn serve(
    stream: interprocess::local_socket::Stream,
    session: u64,
    args: &Args,
) -> Result<(), String> {
    let (conn, mut reader, hello) =
        accept(stream, "yolu-link-demo", session).map_err(|e| e.to_string())?;
    println!(
        "つながりました: {}（版 {}〜{}）",
        hello.agent, hello.min_version, hello.max_version
    );
    let (tx, rx) = mpsc::channel();
    let reply = conn.clone();
    let reading = thread::spawn(move || loop {
        let got = reader.next(&reply);
        let bye = matches!(got, Ok(Received::Message(Message::Bye)) | Err(_));
        if !matches!(got, Ok(Received::Idle)) && tx.send(got).is_err() {
            break;
        }
        // Bye を受けたら自分から抜けて、つながりを閉じる（Windows は受けの時間切れが無いので、待ち合わない）
        if bye {
            break;
        }
    });
    let mut painters: Vec<Painter> = Vec::new();
    let mut next_set = 0u32;
    let mut last_bench = Instant::now();
    let mut bench_full = true;
    let mut bench_value = 0u8;
    loop {
        let got = rx.recv_timeout(Duration::from_millis(50));
        match got {
            Ok(Ok(Received::Message(m))) => {
                if let Some(reply) = wrong_direction(&m, true) {
                    let _ = conn.send(&reply);
                    continue;
                }
                match m {
                    Message::Model(model) => {
                        let vertices: usize = model.meshes.iter().map(|m| m.positions.len()).sum();
                        println!(
                            "モデル「{}」（世代 {}）: マテリアル {}・メッシュ {}・頂点 {vertices}",
                            model.name,
                            model.generation,
                            model.materials.len(),
                            model.meshes.len()
                        );
                        for old in painters.drain(..) {
                            let _ = conn.send(&Message::TextureSetRemoved { set: old.set.set });
                        }
                        for (i, info) in model.materials.iter().enumerate() {
                            if !info.routes.iter().any(|r| r.channel == channel::COLOR) {
                                println!(
                                    "  {}: Unity 側に Color の流し込み先が無いので描かない",
                                    material_name(info)
                                );
                                continue;
                            }
                            next_set += 1;
                            let size = size_for(info, args.size);
                            let started = Instant::now();
                            match Painter::new(
                                session,
                                next_set,
                                model.generation,
                                i as u32,
                                &material_name(info),
                                size,
                            ) {
                                Ok(mut p) => {
                                    conn.send(&p.set.announce()).map_err(|e| e.to_string())?;
                                    let n = p.publish(&conn)?;
                                    println!(
                                        "  {}（{}）: {size}² のセット {} を描いて {n} タイルを送った（{:.1} ms）",
                                        material_name(info),
                                        info.shader,
                                        p.set.set,
                                        started.elapsed().as_secs_f64() * 1000.0
                                    );
                                    painters.push(p);
                                }
                                Err(e) => {
                                    println!("  {}: セットを作れません: {e}", material_name(info))
                                }
                            }
                        }
                    }
                    Message::Pose(p) => {
                        let vertices: usize = p.meshes.iter().map(|m| m.positions.len()).sum();
                        println!(
                            "ポーズ（世代 {}）: メッシュ {}・頂点 {vertices}",
                            p.generation,
                            p.meshes.len()
                        );
                    }
                    Message::Materials(m) => println!(
                        "マテリアルの更新（世代 {}）: {}",
                        m.generation,
                        m.materials.len()
                    ),
                    Message::ModelClosed { generation } => {
                        println!("モデルを閉じた（世代 {generation}）");
                        for old in painters.drain(..) {
                            let _ = conn.send(&Message::TextureSetRemoved { set: old.set.set });
                        }
                    }
                    Message::Bye => {
                        println!("Unity が切りました");
                        break;
                    }
                    Message::Error(e) => println!("Unity からの誤りの知らせ: {}", e.text),
                    _ => {}
                }
            }
            Ok(Ok(Received::Unknown(kind))) => println!("知らない命令（0x{kind:04x}）を断りました"),
            Ok(Ok(Received::Malformed(kind, e))) => println!("読めない命令（0x{kind:04x}）: {e}"),
            Ok(Ok(Received::Idle)) => {}
            Ok(Err(e)) => {
                println!("つながりが切れました: {e}");
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if args.animate {
            for p in &mut painters {
                p.animate()?;
                p.publish(&conn)?;
            }
        }
        if args.bench && last_bench.elapsed() >= Duration::from_secs(1) {
            if let Some(p) = painters.first_mut() {
                last_bench = Instant::now();
                bench_value = bench_value.wrapping_add(37);
                let ts = p.set.tile_size();
                let (tx, ty) = (p.set.width().div_ceil(ts), p.set.height().div_ceil(ts));
                let px = [bench_value, 255 - bench_value, 128, 255];
                let img = p.set.image_mut(channel::COLOR).unwrap();
                let mut tiles = Vec::new();
                let started = Instant::now();
                let (xr, yr) = if bench_full {
                    (0..tx, 0..ty)
                } else {
                    (tx / 2..tx / 2 + 1, ty / 2..ty / 2 + 1)
                };
                for y in yr {
                    for x in xr.clone() {
                        img.write_tile(x, y, |slot| {
                            for q in slot.chunks_exact_mut(4) {
                                q.copy_from_slice(&px);
                            }
                        })
                        .map_err(|e| e.to_string())?;
                        tiles.push(Tile {
                            x: x as u16,
                            y: y as u16,
                        });
                    }
                }
                let wrote = started.elapsed();
                for m in p.set.tiles_changed(channel::COLOR, &tiles) {
                    conn.send(&m).map_err(|e| e.to_string())?;
                }
                println!(
                    "計測: {} タイル（{}²）を共有メモリへ {:.2} ms で書いて知らせた",
                    tiles.len(),
                    p.set.width(),
                    wrote.as_secs_f64() * 1000.0
                );
                bench_full = !bench_full;
            }
        }
    }
    drop(painters);
    drop(conn);
    let _ = reading.join();
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let listener = match link::listen(&args.name) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{} で待ち受けられません: {e}", args.name);
            std::process::exit(1);
        }
    };
    println!("{} で Unity のブリッジを待っています", args.name);
    let mut session = 0u64;
    loop {
        let stream = match listener.accept() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("受けられません: {e}");
                continue;
            }
        };
        session += 1;
        if let Err(e) = serve(stream, session, &args) {
            println!("つながりを終えました: {e}");
        }
        if args.once {
            break;
        }
    }
}
