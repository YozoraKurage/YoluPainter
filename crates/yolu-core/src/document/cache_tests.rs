//! タイルの中身をディスクへ逃がしても、文書の操作が同じバイトになること（`tile_cache`）。読めない中身は誤りになり、黙って透明に
//! しないこと。
//!
//! 逃がすのはプロセスの全体の係（`tile_cache::evict_now`）なので、同じ試験の実行ファイルのほかの試験のタイルも逃がす。逃がしても
//! 同じバイトになるのが約束なので、ほかの試験はそのまま通る（裏の書き手は動かさない: 設定は切のまま、逃がすのは試験が呼ぶ時だけ）。

use super::*;
use crate::brush::{Brush, BrushEffect, ColorMix, MixMode};
use crate::glam::DVec2;
use crate::math::simd::{forced, Level};
use crate::tile_cache::{self, CacheSettings};

/// 全体の係の設定を変える試験を 1 つずつ走らせる鍵。
static CONFIGURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 全体の係にディスクを使わせる（裏の書き手は起こさない）。返した鍵を持っている間、ほかの試験は設定を変えない。
fn use_disk() -> std::sync::MutexGuard<'static, ()> {
    let one = CONFIGURING.lock().unwrap_or_else(|e| e.into_inner());
    tile_cache::configure(&CacheSettings {
        enabled: false,
        folder: None,
        memory_limit: u64::MAX,
        disk_limit: 1 << 40,
    });
    one
}

/// 読んでいない中身を全部ディスクへ逃がす。
fn evict_all() {
    tile_cache::evict_now(0);
}

/// 乱数の画素の層（全タイルが画素あり）。
fn noise(d: &mut Document, id: LayerId, seed: u32) {
    let ts = d.tile_size();
    let n = (ts * ts) as usize;
    for ty in 0..d.height().div_ceil(ts) {
        for tx in 0..d.width().div_ceil(ts) {
            let mut bytes = vec![0u8; n * 4];
            for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
                let (x, y) = (tx * ts + (i as u32) % ts, ty * ts + (i as u32) / ts);
                if x >= d.width() || y >= d.height() {
                    continue; // 画布の外の余白は 0
                }
                let v = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ seed)
                    .wrapping_mul(2246822519);
                p.copy_from_slice(&[
                    v as u8,
                    (v >> 8) as u8,
                    (v >> 16) as u8,
                    if v >> 30 == 0 { 0 } else { (v >> 24) as u8 | 1 },
                ]);
            }
            d.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
}

fn stroke(d: &mut Document, id: LayerId, brush: &Brush, points: &[(f64, f64)]) {
    let mut s = d.begin_brush_stroke(id, brush).unwrap();
    for &(x, y) in points {
        s.add_point(d, x, y, 0.8, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
}

fn brush(color: Rgba8, radius: f64) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 0.6,
        spacing: 0.15,
        color,
        ..BrushSettings::default()
    });
    b.seed = 3;
    b
}

/// 文書の見える状態（全体の合成と、層ごとの Color とマスクの全画素）。
fn state(d: &Document) -> Vec<Vec<u8>> {
    let mut out = vec![d.composite(d.bounds()).unwrap()];
    for l in d.layers() {
        if let Some(s) = l.surface(Channel::Color) {
            out.push(s.canvas_bytes().unwrap());
        }
        if let Some(m) = l.mask() {
            out.push(m.surface().canvas_bytes().unwrap());
        }
    }
    out
}

/// 決まった手順の操作を順に行い、操作ごとの見える状態を返す。`evict` なら操作の前ごとに、全部の中身を逃がす（層・取り消しの写し・
/// 保存の写しのどれも）。
fn scenario(evict: bool) -> Vec<Vec<Vec<u8>>> {
    let spill = |d: &Document| {
        if evict {
            evict_all();
            let on_disk: usize = d
                .layers()
                .iter()
                .filter_map(|l| l.surface(Channel::Color))
                .map(Surface::evicted_tile_count)
                .sum();
            assert!(on_disk > 0, "層のタイルがディスクにある");
        }
    };
    let mut d = Document::with_tile_size(70, 52, 16).unwrap();
    let base = d.add_layer("base").unwrap();
    let mid = d.add_layer("mid").unwrap();
    let top = d.add_layer("top").unwrap();
    noise(&mut d, base, 1);
    noise(&mut d, mid, 2);
    d.clear_history().unwrap();
    let mut states = vec![state(&d)];
    let mut step = |d: &mut Document, f: &mut dyn FnMut(&mut Document)| {
        spill(d);
        f(d);
        states.push(state(d));
    };
    // 塗る・消す・ぼかし・指先・クローン・色の混ぜ
    step(&mut d, &mut |d| {
        stroke(
            d,
            top,
            &brush(Rgba8::new(200, 40, 30, 255), 6.0),
            &[(5.0, 5.0), (40.0, 30.0), (65.0, 8.0)],
        )
    });
    step(&mut d, &mut |d| {
        let mut b = brush(Rgba8::new(0, 0, 0, 255), 5.0);
        b.base.erase = true;
        stroke(d, mid, &b, &[(10.0, 40.0), (50.0, 12.0)]);
    });
    for effect in [
        BrushEffect::Blur { radius: 2 },
        BrushEffect::Smudge { strength: 0.7 },
        BrushEffect::Clone {
            offset: DVec2::new(9.0, -4.0),
        },
    ] {
        step(&mut d, &mut |d| {
            let mut b = brush(Rgba8::new(10, 200, 90, 255), 7.0);
            b.effect = effect;
            stroke(d, mid, &b, &[(20.0, 20.0), (45.0, 35.0), (60.0, 20.0)]);
        });
    }
    step(&mut d, &mut |d| {
        let mut b = brush(Rgba8::new(30, 60, 220, 255), 6.0);
        b.mix = ColorMix {
            mode: MixMode::Mix,
            ..ColorMix::default()
        };
        stroke(d, mid, &b, &[(8.0, 10.0), (30.0, 44.0)]);
    });
    // 選択範囲の中の塗りつぶし・自動選択・マスク
    step(&mut d, &mut |d| {
        let r = SelectionMask::rectangle(d, 12, 9, 50, 41);
        d.set_selection(Some(r)).unwrap();
        d.fill(
            mid,
            Channel::Color,
            Rgba8::new(250, 250, 0, 255),
            0.5,
            None,
            false,
        )
        .unwrap();
        d.clear_selection().unwrap();
    });
    step(&mut d, &mut |d| {
        let wand =
            SelectionMask::magic_wand(d, Some(base), Channel::Color, 30, 20, 90, false, u64::MAX)
                .unwrap();
        d.fill(
            top,
            Channel::Color,
            Rgba8::new(0, 120, 255, 255),
            0.7,
            Some(&wand),
            false,
        )
        .unwrap();
    });
    step(&mut d, &mut |d| {
        d.add_layer_mask(mid).unwrap();
        d.fill_mask(
            mid,
            0.6,
            Some(&SelectionMask::rectangle(d, 0, 0, 33, 52)),
            false,
        )
        .unwrap();
    });
    // 取り消し・やり直し
    step(&mut d, &mut |d| {
        assert!(d.undo().unwrap());
        assert!(d.undo().unwrap());
    });
    step(&mut d, &mut |d| assert!(d.redo().unwrap()));
    // 変形・結合
    step(&mut d, &mut |d| {
        let (c, s) = (0.3f64.cos(), 0.3f64.sin());
        let t = Affine2D {
            a: c,
            b: -s,
            c: s,
            d: c,
            tx: 10.0,
            ty: -6.0,
        };
        assert!(d
            .transform_layer(top, t, Resampling::Bilinear, false)
            .unwrap());
    });
    step(&mut d, &mut |d| {
        d.merge_down(top, 255).unwrap();
    });
    step(&mut d, &mut |d| assert!(d.undo().unwrap()));
    // 保存の写しは、元が書いた後も写した時の画素
    let snapshot = d.capture_snapshot().unwrap();
    let frozen = state(&snapshot);
    step(&mut d, &mut |d| {
        stroke(
            d,
            base,
            &brush(Rgba8::new(255, 255, 255, 255), 9.0),
            &[(0.0, 0.0), (69.0, 51.0)],
        )
    });
    spill(&d);
    assert_eq!(state(&snapshot), frozen, "保存の写しは動かない");
    states.push(frozen);
    states
}

#[test]
fn every_operation_gives_the_same_bytes_when_the_tiles_live_on_disk() {
    let _one = use_disk();
    let expected = scenario(false);
    for level in forced::supported() {
        let got = forced::with_level(level, || scenario(true));
        assert_eq!(got.len(), expected.len());
        for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
            assert!(g == e, "{level:?}: 手順 {i} の画素が違う");
        }
    }
    assert!(forced::supported().contains(&Level::Scalar));
}

#[test]
fn an_unreadable_tile_is_an_error_everywhere_and_changes_nothing() {
    let _one = use_disk();
    let mut d = Document::with_tile_size(40, 40, 16).unwrap();
    let below = d.add_layer("below").unwrap();
    let id = d.add_layer("layer").unwrap();
    noise(&mut d, below, 4);
    noise(&mut d, id, 5);
    d.clear_history().unwrap();
    let before_revision = d.revision();
    let surface = d.layer(id).unwrap().surface(Channel::Color).unwrap();
    assert!(surface.evict_tiles_now() > 0);
    surface.fail_tile_reads_for_test();
    let unreadable = Err(CoreError::TileUnreadable);
    assert_eq!(d.composite(d.bounds()).map(|_| ()), unreadable);
    assert_eq!(
        d.composite_pixel(Channel::Color, 3, 3).map(|_| ()),
        unreadable
    );
    let mut buf = vec![0u8; 16 * 16 * 4];
    assert_eq!(
        surface
            .copy_tile(TileCoord::new(0, 0), &mut buf)
            .map(|_| ()),
        unreadable
    );
    assert_eq!(surface.canvas_bytes().map(|_| ()), unreadable);
    assert!(surface.has_unreadable_tiles());
    // 書く操作は断り、何も変えない
    let mut s = d
        .begin_brush_stroke(id, &brush(Rgba8::new(1, 2, 3, 255), 4.0))
        .unwrap();
    assert_eq!(s.add_point(&mut d, 8.0, 8.0, 1.0, DVec2::ZERO), unreadable);
    assert!(!d.has_active_stroke(), "失敗したストロークは取り消す");
    assert_eq!(d.merge_down(id, 255).map(|_| ()), unreadable);
    assert_eq!(
        d.transform_layer(
            id,
            Affine2D::translation(3.0, 1.0),
            Resampling::Nearest,
            false
        )
        .map(|_| ()),
        unreadable
    );
    assert_eq!(
        d.fill(
            id,
            Channel::Color,
            Rgba8::new(9, 9, 9, 255),
            1.0,
            None,
            false
        )
        .map(|_| ()),
        unreadable
    );
    assert_eq!(
        SelectionMask::magic_wand(&d, Some(id), Channel::Color, 1, 1, 10, false, u64::MAX)
            .map(|_| ()),
        unreadable
    );
    assert_eq!(d.revision(), before_revision);
    assert_eq!(d.undo_count(), 0);
    // 読める層は読める
    let other = d.layer(below).unwrap().surface(Channel::Color).unwrap();
    assert!(other.canvas_bytes().is_ok());
    assert!(!other.has_unreadable_tiles());
}

/// 選択範囲の全タイルの量（座標つき）。
fn amounts(mask: &SelectionMask) -> Vec<(TileCoord, Vec<u8>)> {
    let n = (mask.tile_size() * mask.tile_size()) as usize;
    mask.tile_coords()
        .into_iter()
        .map(|coord| {
            let mut a = vec![0; n];
            mask.copy_tile(coord, &mut a).unwrap();
            (coord, a)
        })
        .collect()
}

#[test]
fn a_selection_made_from_an_unreadable_surface_is_an_error_not_an_empty_one() {
    let _one = use_disk();
    let d = Document::with_tile_size(40, 40, 16).unwrap();
    let mask = SelectionMask::rectangle(&d, 4, 4, 30, 30);
    assert!(!amounts(&mask).is_empty());
    let surface = super::transform::selection_surface(&mask);
    // ディスクへ逃がしていても、読み戻して同じ選択範囲になる
    assert!(surface.evict_tiles_now() > 0);
    let back = super::resize::selection_from_surface(&surface).unwrap();
    assert_eq!(amounts(&back), amounts(&mask));
    // 読めないタイルは、選ばれていないことにせず誤りで返す
    assert!(surface.evict_tiles_now() > 0);
    surface.fail_tile_reads_for_test();
    assert_eq!(
        super::resize::selection_from_surface(&surface).map(|_| ()),
        Err(CoreError::TileUnreadable)
    );
}

#[test]
fn the_background_writer_moves_what_exceeds_the_limit_and_the_bytes_stay_the_same() {
    let _one = use_disk();
    let mut d = Document::with_tile_size(96, 96, 16).unwrap();
    let id = d.add_layer("layer").unwrap();
    noise(&mut d, id, 9);
    d.clear_history().unwrap();
    let expected = state(&d);
    let surface = |d: &Document| {
        d.layer(id)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .clone()
    };
    // 入にして上限を小さくすると、裏の書き手が使っていない中身から逃がす（ほかの試験の中身も逃がすが、どれも同じバイトのまま）
    tile_cache::configure(&CacheSettings {
        enabled: true,
        folder: None,
        memory_limit: 16 * 16 * 4,
        disk_limit: 1 << 40,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while surface(&d).evicted_tile_count() == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "裏の書き手が逃がさない"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // 切ると逃がすのを止める（逃がした分は読むときに戻る）
    tile_cache::configure(&CacheSettings {
        enabled: false,
        folder: None,
        memory_limit: u64::MAX,
        disk_limit: 1 << 40,
    });
    assert!(state(&d) == expected, "逃がしても同じバイト");
    assert!(tile_cache::status().disk_bytes > 0);
}
