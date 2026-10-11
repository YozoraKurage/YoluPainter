use yolu_core::{BlendMode, Channel, Document, Rect, Rgba8, TileCoord};
#[path = "support/gpu_lease.rs"]
mod gpu_lease;
#[path = "support/require_gpu.rs"]
mod require_gpu;
use yolu_gpu::{GpuPainter, Options, ResidentCompositor, ResidentOptions};
fn gpu(options: ResidentOptions) -> Option<ResidentCompositor> {
    gpu_lease::lease();
    let g = match GpuPainter::new(Options::default()) {
        Ok(g) => g,
        Err(e) => {
            assert!(e.to_string().starts_with("GPU 利用不可:"), "{e}");
            require_gpu::skipped("GPU 常駐試験", &e.to_string());
            return None;
        }
    };
    eprintln!("GPU 常駐試験: {:?}", g.adapter_info());
    Some(ResidentCompositor::with_gpu(g, options).unwrap())
}
fn document(w: u32, h: u32) -> Document {
    let mut d = Document::with_tile_size(w, h, 16).unwrap();
    for k in 0..3 {
        let l = d.add_layer("レイヤー").unwrap();
        for y in 0..h {
            for x in 0..w {
                d.set_pixel(
                    l,
                    x,
                    y,
                    Rgba8::new(
                        (x * 7 + k * 37) as u8,
                        (y * 11) as u8,
                        83,
                        if k == 0 { 255 } else { 128 },
                    ),
                )
                .unwrap();
            }
        }
    }
    d
}
fn read(g: &mut ResidentCompositor, d: &Document) -> Vec<u8> {
    let request = g.request_readback(d.bounds()).unwrap();
    g.finish_readback(request).unwrap()
}
fn compare(g: &mut ResidentCompositor, d: &Document) -> u8 {
    let expected = d.composite(d.bounds()).unwrap();
    let actual = read(g, d);
    assert_eq!(expected.len(), actual.len());
    let max = expected
        .iter()
        .zip(actual)
        .map(|(a, b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert!(max <= 1, "最大差 {max}");
    max
}
#[test]
fn resident_uploads_only_changed_layer_and_keeps_texture() {
    let Some(mut g) = gpu(ResidentOptions::default()) else {
        return;
    };
    let mut d = document(35, 19);
    let first = g.update(&d, Channel::Color).unwrap();
    assert_eq!(first.updated_tiles, 6);
    assert_eq!(first.uploaded_tiles, 18);
    // 18 枚のタイルと命令の並び・束の定数を、1 回の書き込みにまとめる
    assert_eq!(first.transfers, 1);
    assert_eq!(g.pending_readback_bytes(), 0);
    compare(&mut g, &d);
    let generation = g.display().unwrap().generation;
    let idle = g.update(&d, Channel::Color).unwrap();
    assert_eq!(idle.uploaded_bytes, 0);
    assert_eq!(idle.transfers, 0);
    assert_eq!(idle.updated_tiles, 0);
    assert_eq!(g.display().unwrap().generation, generation);
    let l = d.layers()[1].id();
    d.set_pixel(l, 17, 2, Rgba8::new(17, 39, 61, 123)).unwrap();
    let changed = g.update(&d, Channel::Color).unwrap();
    assert_eq!(changed.updated_tiles, 1);
    assert_eq!(changed.uploaded_tiles, 1);
    assert_eq!(changed.cache_hits, 2);
    assert_eq!(changed.evicted_tiles, 0);
    compare(&mut g, &d);
    d.set_layer_opacity(l, 0.7, false).unwrap();
    let property = g.update(&d, Channel::Color).unwrap();
    assert_eq!(property.uploaded_tiles, 0);
    // タイルは上げないが、束の定数・座標・命令の列は 1 回で書く
    assert_eq!(property.transfers, 1);
    assert_eq!(property.updated_tiles, 6);
    compare(&mut g, &d);
    d.set_layer_clipping(l, true).unwrap();
    g.update(&d, Channel::Color).unwrap();
    compare(&mut g, &d);
    d.undo().unwrap();
    g.update(&d, Channel::Color).unwrap();
    compare(&mut g, &d);
    d.remove_layer(l).unwrap();
    g.update(&d, Channel::Color).unwrap();
    compare(&mut g, &d);
}
#[test]
fn lru_evicts_oldest_and_remains_inside_budget() {
    // 48×16表示3072、作業域2192（入力・命令・タイルの有無の印・命令の番号の列を、GPU と転送用で 2 つ）、入力のGPU+CPUが2048/タイル。
    // 常駐はちょうど2枚。
    let options = ResidentOptions {
        resident_budget_bytes: 9376,
        readback_budget_bytes: 8192,
        batch_tiles: 1,
        ..Default::default()
    };
    let Some(mut g) = gpu(options) else { return };
    let mut d = Document::with_tile_size(48, 16, 16).unwrap();
    let l = d.add_layer("レイヤー").unwrap();
    for x in [0, 16, 32] {
        d.set_pixel(l, x, 0, Rgba8::new(99, 100, 101, 255)).unwrap();
    }
    let first = g.update(&d, Channel::Color).unwrap();
    assert_eq!(first.uploaded_tiles, 3);
    assert_eq!(first.evicted_tiles, 1);
    // 予算が狭いので、上げの上限は下限の 1 束ぶんになり、束（1 タイル）ごとに流す。3 枚目を入れるときに追い出す 1 枚目は、もう流してある
    assert_eq!(first.transfers, 3);
    assert_eq!(first.cached_tiles, 2);
    assert!(first.resident_bytes <= options.resident_budget_bytes);
    compare(&mut g, &d);
    d.set_pixel(l, 17, 1, Rgba8::new(2, 3, 4, 255)).unwrap();
    let hit = g.update(&d, Channel::Color).unwrap();
    assert_eq!(hit.evicted_tiles, 0);
    assert_eq!(hit.uploaded_tiles, 1);
    d.set_pixel(l, 1, 1, Rgba8::new(3, 4, 5, 255)).unwrap();
    let missing = g.update(&d, Channel::Color).unwrap();
    assert_eq!(missing.evicted_tiles, 1);
    assert!(missing.resident_bytes <= options.resident_budget_bytes);
    // タイル1は直前に触ったので残り、古いタイル2が追い出された。
    d.set_pixel(l, 18, 1, Rgba8::new(4, 5, 6, 255)).unwrap();
    assert_eq!(g.update(&d, Channel::Color).unwrap().evicted_tiles, 0);
    d.set_pixel(l, 33, 1, Rgba8::new(5, 6, 7, 255)).unwrap();
    assert_eq!(g.update(&d, Channel::Color).unwrap().evicted_tiles, 1);
    compare(&mut g, &d);
    let Some(mut denied) = gpu(ResidentOptions {
        resident_budget_bytes: 1,
        ..options
    }) else {
        return;
    };
    assert!(denied
        .update(&d, Channel::Color)
        .unwrap_err()
        .to_string()
        .contains("予算"));
    assert!(denied.display().is_err());
}
/// 上げの上限で分けて流す道: 上限を小さくすると、1 回の更新の上げを束の区切りで何度かに分けて流す（書き込みが 2 回以上）。絵は CPU の合成と差 1 以内のまま。
#[test]
fn a_small_transfer_limit_splits_the_upload_and_keeps_the_pixels() {
    let options = ResidentOptions {
        batch_tiles: 2,
        // 1 バイト: 上限は下限の 1 束ぶんになり、束ごとに流す
        transfer_limit_bytes: Some(1),
        ..Default::default()
    };
    let Some(mut g) = gpu(options) else { return };
    let mut d = document(35, 19);
    let first = g.update(&d, Channel::Color).unwrap();
    assert_eq!(first.updated_tiles, 6);
    assert_eq!(first.uploaded_tiles, 18);
    assert!(first.transfers >= 2, "分けて流す: {}", first.transfers);
    compare(&mut g, &d);
    // 何枚かのタイルだけが変わる回（上げが 1 束ぶんに収まれば 1 回）も、絵は同じ
    let l = d.layers()[2].id();
    for (x, y) in [(1, 1), (20, 3), (33, 17)] {
        d.set_pixel(l, x, y, Rgba8::new(200, 10, 50, 255)).unwrap();
    }
    let changed = g.update(&d, Channel::Color).unwrap();
    assert_eq!(changed.uploaded_tiles, 3);
    assert!(changed.transfers >= 1);
    compare(&mut g, &d);
    // 既定（予算から決める上限）では 1 回
    let Some(mut whole) = gpu(ResidentOptions::default()) else {
        return;
    };
    assert_eq!(whole.update(&d, Channel::Color).unwrap().transfers, 1);
    compare(&mut whole, &d);
}
#[test]
fn readback_budget_generation_cancel_and_owner_lifetime() {
    let options = ResidentOptions {
        readback_budget_bytes: 256 * 19,
        ..Default::default()
    };
    let Some(mut g) = gpu(options) else { return };
    let mut d = document(35, 19);
    g.update(&d, Channel::Color).unwrap();
    let old = g.request_readback(d.bounds()).unwrap();
    assert_eq!(g.pending_readback_bytes(), 256 * 19);
    assert!(g.request_readback(d.bounds()).is_err());
    let l = d.layers()[0].id();
    d.set_pixel(l, 0, 0, Rgba8::new(200, 1, 2, 255)).unwrap();
    g.update(&d, Channel::Color).unwrap();
    assert!(g
        .finish_readback(old)
        .unwrap_err()
        .to_string()
        .contains("古い世代"));
    assert_eq!(g.pending_readback_bytes(), 0);
    let canceled = g.request_readback(d.bounds()).unwrap();
    drop(canceled);
    g.reset().unwrap();
    assert_eq!(g.pending_readback_bytes(), 0);
    g.update(&d, Channel::Color).unwrap();
    let old = g.request_readback(d.bounds()).unwrap();
    // 完了を待たずに差し替える。reset 自身が旧テクスチャのコピー寿命を守る。
    g.reset().unwrap();
    assert!(g.finish_readback(old).is_err());
    assert_eq!(g.pending_readback_bytes(), 0);
    g.update(&d, Channel::Color).unwrap();
    compare(&mut g, &d);
    let foreign = g.request_readback(d.bounds()).unwrap();
    foreign.wait_ready().unwrap();
    let Some(mut other) = gpu(options) else {
        return;
    };
    other.update(&d, Channel::Color).unwrap();
    assert!(other.finish_readback(foreign).is_err());
    assert_eq!(g.pending_readback_bytes(), 0);
    let ownerless = g.request_readback(d.bounds()).unwrap();
    drop(g);
    ownerless.wait_ready().unwrap();
    drop(ownerless);
}
#[test]
fn display_all_modes_and_document_switch() {
    let Some(mut g) = gpu(ResidentOptions::default()) else {
        return;
    };
    let mut d = document(35, 19);
    let l = d.layers()[2].id();
    for mode in BlendMode::LAYER_MODES {
        d.set_layer_blend_mode(l, mode).unwrap();
        for clip in [false, true] {
            d.set_layer_clipping(l, clip).unwrap();
            g.update(&d, Channel::Color).unwrap();
            eprintln!(
                "常駐表示 {mode:?} clip={clip}: 最大差 {}",
                compare(&mut g, &d)
            );
        }
    }
    let old = g.request_readback(Rect::new(1, 1, 3, 2)).unwrap();
    let fresh = Document::new(17, 9).unwrap();
    g.update(&fresh, Channel::Color).unwrap();
    assert!(g.finish_readback(old).is_err());
    assert_eq!(read(&mut g, &fresh), vec![0; 17 * 9 * 4]);
    g.update(&d, Channel::Emission).unwrap();
    assert!(read(&mut g, &d).iter().all(|&b| b == 0));
    let mut tile = vec![0; 16 * 16 * 4];
    for p in tile.as_chunks_mut::<4>().0 {
        *p = [80, 90, 100, 255];
    }
    d.import_tile(
        d.layers()[0].id(),
        Channel::Emission,
        TileCoord::new(0, 0),
        &tile,
    )
    .unwrap();
    assert_eq!(g.update(&d, Channel::Emission).unwrap().uploaded_tiles, 1);
    let request = g.request_readback(Rect::new(1, 1, 3, 2)).unwrap();
    assert_eq!(
        g.finish_readback(request).unwrap(),
        [80, 90, 100, 255].repeat(6)
    );
}

/// 計測: 常駐の合成の更新（`update`）の 1 回の時間と上げの量を、変更の種類ごとに（不透明度だけ・ぼかしの半径（効果の出力が全部変わる）・
/// 一部の画素）。文書は `RESIDENT_SIZE`（既定 2048）四方・128² のタイルで、模様のラスターと、同じ模様にぼかしを掛けたラスターの 2 枚。
/// 回数 `RESIDENT_FRAMES`（既定 16。前に 3 回回して外す）。GPU は wgpu の環境変数（`WGPU_BACKEND` など）で選ぶ。
/// `cargo test -p yolu-gpu --test resident measure_resident_update_frames -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_resident_update_frames() {
    use std::time::Instant;
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let var = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u32>().ok());
    let size = var("RESIDENT_SIZE").unwrap_or(2048);
    let frames = var("RESIDENT_FRAMES").unwrap_or(16) as usize;
    let Some(mut g) = gpu(ResidentOptions {
        resident_budget_bytes: 512 << 20,
        readback_budget_bytes: 1 << 20,
        premultiplied_display: true,
        ..Default::default()
    }) else {
        return;
    };
    let mut d = Document::new(size, size).unwrap();
    let ts = d.tile_size();
    let pattern = |x: u32, y: u32| -> [u8; 4] {
        let c = ((x / 37 + y / 53) % 7) as u8;
        [40 + c * 30, 200 - c * 20, 60 + ((x ^ y) & 63) as u8, 255]
    };
    let fill = |d: &mut Document, layer| {
        let coords: Vec<TileCoord> = d.canvas_tiles().collect();
        for c in coords {
            let r = d.tile_rect(c).unwrap();
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..r.height {
                for x in 0..r.width {
                    let i = ((y * ts + x) * 4) as usize;
                    bytes[i..i + 4].copy_from_slice(&pattern(r.x + x, r.y + y));
                }
            }
            d.import_tile(layer, Channel::Color, c, &bytes).unwrap();
        }
    };
    let base = d.add_layer("模様").unwrap();
    fill(&mut d, base);
    let blurred = d.add_layer("ぼかし").unwrap();
    fill(&mut d, blurred);
    let blur = d
        .add_filter(
            blurred,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(6)).channels(&[Channel::Color]),
        )
        .unwrap();
    d.set_layer_opacity(blurred, 0.6, false).unwrap();
    g.update(&d, Channel::Color).unwrap();
    println!("GPU: {:?}、文書 {size}²", g.adapter_info().name);
    for kind in ["opacity", "blur", "paint"] {
        d.release_effect_cache();
        let mut times = Vec::new();
        let (mut tiles, mut bytes) = (0usize, 0u64);
        for frame in 0..frames + 3 {
            match kind {
                "opacity" => d
                    .set_layer_opacity(base, 0.5 + 0.4 * ((frame as f64) * 0.35).sin().abs(), true)
                    .unwrap(),
                "blur" => d
                    .set_filter_settings(
                        blurred,
                        blur,
                        EffectSettings::blur(4 + (frame % 8) as u32),
                        true,
                    )
                    .unwrap(),
                _ => {
                    // 64² の画素を、毎回違う所・違う色で
                    let (x0, y0) = (
                        (frame as u32 * 97) % (size - 64),
                        (frame as u32 * 151) % (size - 64),
                    );
                    for y in y0..y0 + 64 {
                        for x in x0..x0 + 64 {
                            d.set_pixel(base, x, y, Rgba8::new(frame as u8 * 13, 90, 200, 255))
                                .unwrap();
                        }
                    }
                }
            }
            let t = Instant::now();
            let s = g.update(&d, Channel::Color).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if frame >= 3 {
                times.push(ms);
                tiles += s.uploaded_tiles;
                bytes += s.uploaded_bytes;
            }
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        println!(
            "{kind:<8}: update 平均 {mean:.1} ms（中央 {:.1}・最小 {:.1}・最大 {:.1}）、1 回の上げ {:.0} タイル・{:.1} MiB",
            times[times.len() / 2],
            times[0],
            times[times.len() - 1],
            tiles as f64 / times.len() as f64,
            bytes as f64 / times.len() as f64 / (1u64 << 20) as f64
        );
    }
}
