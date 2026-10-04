use yolu_core::{BlendMode, Channel, Document, Rect, Rgba8, TileCoord};
use yolu_gpu::{GpuPainter, Options, ResidentCompositor, ResidentOptions};
fn gpu(options: ResidentOptions) -> Option<ResidentCompositor> {
    let g = match GpuPainter::new(Options::default()) {
        Ok(g) => g,
        Err(e) => {
            assert!(e.to_string().starts_with("GPU 利用不可:"), "{e}");
            eprintln!("GPU 常駐試験をスキップ: {e}");
            return None;
        }
    };
    eprintln!("GPU 常駐試験: {:?}", g.adapter_info());
    Some(ResidentCompositor::with_gpu(g, options).unwrap())
}
fn document(w: u32, h: u32) -> Document {
    let mut d = Document::with_tile_size(w, h, 16).unwrap();
    for k in 0..3 {
        let l = d.add_layer("層").unwrap();
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
    assert_eq!(g.pending_readback_bytes(), 0);
    compare(&mut g, &d);
    let generation = g.display().unwrap().generation;
    let idle = g.update(&d, Channel::Color).unwrap();
    assert_eq!(idle.uploaded_bytes, 0);
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
    // 48×16表示3072、作業域2176、入力のGPU+CPUが2048/タイル。常駐はちょうど2枚。
    let options = ResidentOptions {
        resident_budget_bytes: 9344,
        readback_budget_bytes: 8192,
        batch_tiles: 1,
        ..Default::default()
    };
    let Some(mut g) = gpu(options) else { return };
    let mut d = Document::with_tile_size(48, 16, 16).unwrap();
    let l = d.add_layer("層").unwrap();
    for x in [0, 16, 32] {
        d.set_pixel(l, x, 0, Rgba8::new(99, 100, 101, 255)).unwrap();
    }
    let first = g.update(&d, Channel::Color).unwrap();
    assert_eq!(first.uploaded_tiles, 3);
    assert_eq!(first.evicted_tiles, 1);
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
