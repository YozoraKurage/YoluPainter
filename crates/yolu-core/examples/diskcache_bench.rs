//! 大きな文書（既定 4096² × 20 レイヤー、全タイルが画素あり）の合成・ストローク・取り消し・やり直しの時間。
//!   cargo run --release -p yolu-core --example diskcache_bench -- [--size 4096] [--layers 20] [--runs 3]
//!       [--only composite,stroke,merge,transform] [--cache-mib N] [--cold]
//! `--cache-mib N` はディスクキャッシュを入にし、メモリの上限を N MiB にする（超えた分を裏の書き手が一時フォルダのファイルへ逃がす）。
//! `--cold` は、測る操作の前ごとに読んでいない中身を全部ディスクへ逃がす（全部を読み戻す最悪の形）。
//! 準備（レイヤーを埋める）は計測の外。各操作は `--runs` 回の中央値（ミリ秒、壁時計）。
//! - composite: キャンバスの全面の Color の合成。
//! - stroke_small / stroke_large: 一番上のレイヤーへの硬い丸筆（半径 32・256）の 1 本（始め・点・確定）。
//! - undo / redo: その 1 本の取り消し・やり直し。
//! - merge_down: 一番上のレイヤーを下のレイヤーへ結合（画素ごとの式の道）。transform: 一番上のレイヤーを 10° 回す（双線形）。どちらも直後に取り消す。
use std::time::Instant;

use yolu_core::glam::DVec2;
use yolu_core::tile_cache::{self, CacheSettings};
use yolu_core::{
    Affine2D, BrushSettings, Channel, Document, LayerId, Rect, Resampling, Rgba8, TileCoord,
};

fn arg(name: &str, default: u32) -> u32 {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.parse().expect("数"))
        .unwrap_or(default)
}

fn document(side: u32, layers: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(side, side, 128).unwrap();
    d.set_source_budget_bytes(u64::MAX / 4).unwrap();
    d.set_undo_budget_bytes(4 << 30).unwrap();
    d.set_stroke_budget_bytes(4 << 30).unwrap();
    let mut last = LayerId(0);
    let tiles = side / 128;
    let mut bytes = vec![0u8; 128 * 128 * 4];
    for l in 0..layers {
        last = d.add_layer("レイヤー").unwrap();
        for ty in 0..tiles {
            for tx in 0..tiles {
                for (i, p) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let n = (i as u32)
                        .wrapping_mul(17)
                        .wrapping_add((ty * tiles + tx) * 31 + l * 7);
                    p.copy_from_slice(&[
                        (n * 3 + 1) as u8,
                        (n * 5 + 2) as u8,
                        (n * 7 + 3) as u8,
                        96 + (n % 128) as u8,
                    ]);
                }
                d.import_tile(last, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    (d, last)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn stroke(d: &mut Document, layer: LayerId, side: u32, radius: f64) {
    let b = BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        color: Rgba8::new(200, 60, 30, 255),
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke(layer, &b).unwrap();
    let n = 300;
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let x = side as f64 * (0.05 + 0.9 * t);
        let y = side as f64 * (0.5 + 0.35 * (t * std::f64::consts::TAU * 1.5).sin());
        s.add_point(d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let only: Option<Vec<String>> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(str::to_owned).collect());
    let want = |name: &str| only.as_ref().is_none_or(|o| o.iter().any(|x| x == name));
    let side = arg("--size", 4096);
    let layers = arg("--layers", 20);
    let runs = arg("--runs", 3) as usize;
    let cache_mib = arg("--cache-mib", 0) as u64;
    let cold = args.iter().any(|a| a == "--cold");
    if cache_mib > 0 || cold {
        tile_cache::configure(&CacheSettings {
            enabled: cache_mib > 0,
            folder: None,
            memory_limit: cache_mib << 20,
            disk_limit: 64 << 30,
        });
    }
    let start = Instant::now();
    let (mut d, top) = document(side, layers);
    eprintln!(
        "準備: {side}² × {layers} レイヤー・{} MiB・{:.0} ms",
        d.allocated_bytes() >> 20,
        start.elapsed().as_secs_f64() * 1000.0
    );
    let rect = Rect::new(0, 0, side, side);
    let time = |f: &mut dyn FnMut()| {
        if cold {
            tile_cache::evict_now(0);
        }
        let t = Instant::now();
        f();
        t.elapsed().as_secs_f64() * 1000.0
    };
    if want("composite") {
        let mut composite = Vec::new();
        for _ in 0..runs {
            composite.push(time(&mut || {
                std::hint::black_box(d.composite(rect).unwrap());
            }));
        }
        println!("composite\t{:.1}", median(composite));
    }
    for (name, radius) in [("stroke_small", 32.0), ("stroke_large", 256.0)] {
        if !want("stroke") {
            continue;
        }
        let (mut s, mut u, mut r) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..runs {
            s.push(time(&mut || stroke(&mut d, top, side, radius)));
            u.push(time(&mut || assert!(d.undo().unwrap())));
            r.push(time(&mut || assert!(d.redo().unwrap())));
            assert!(d.undo().unwrap());
        }
        println!("{name}\t{:.1}", median(s));
        println!("{name}_undo\t{:.1}", median(u));
        println!("{name}_redo\t{:.1}", median(r));
    }
    let (mut m, mut t) = (Vec::new(), Vec::new());
    let (c, s) = (10f64.to_radians().cos(), 10f64.to_radians().sin());
    let half = side as f64 / 2.0;
    let rotate = Affine2D {
        a: c,
        b: -s,
        c: s,
        d: c,
        tx: half - c * half + s * half,
        ty: half - s * half - c * half,
    };
    for _ in 0..runs {
        if want("merge") {
            m.push(time(&mut || {
                d.merge_down(top, 255).unwrap();
            }));
            assert!(d.undo().unwrap());
        }
        if want("transform") {
            t.push(time(&mut || {
                assert!(d
                    .transform_layer(top, rotate, Resampling::Bilinear, false)
                    .unwrap());
            }));
            assert!(d.undo().unwrap());
        }
    }
    if want("merge") {
        println!("merge_down\t{:.1}", median(m));
    }
    if want("transform") {
        println!("transform\t{:.1}", median(t));
    }
    if want("composite") {
        let mut after = Vec::new();
        for _ in 0..runs {
            after.push(time(&mut || {
                std::hint::black_box(d.composite(rect).unwrap());
            }));
        }
        println!("composite_after\t{:.1}", median(after));
    }
    if cache_mib > 0 || cold {
        let st = tile_cache::status();
        eprintln!(
            "キャッシュ: メモリ {} MiB・ディスク {} MiB・読めなかった {}",
            st.resident_bytes >> 20,
            st.disk_bytes >> 20,
            st.read_failures
        );
    }
}
