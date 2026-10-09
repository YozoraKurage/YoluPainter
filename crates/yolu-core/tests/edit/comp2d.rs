//! 2D の合成を速くする仕組み（散らばったタイルの合成・描いている間の下と上の覚え・歩幅つきの粗い合成）の、
//! 全レイヤーを下から重ねた今までの合成とバイトで同じことの試験。

use crate::comp2d_support;

use comp2d_support::*;
use yolu_core::{Channel, Document, Rect, RowOrder, TileCoord};

fn reference(d: &Document, ch: Channel, r: Rect) -> Vec<u8> {
    let mut out = vec![0u8; (r.width * r.height * 4) as usize];
    d.composite_into(ch, r, &mut out, RowOrder::BottomUp)
        .unwrap();
    out
}

#[test]
fn tiles_composited_in_a_batch_match_the_rectangle_composite() {
    for seed in 0..40u64 {
        let (d, _) = random_doc(seed, 70, 50, 16);
        for ch in [Channel::Color, Channel::Normal] {
            // 全部・飛び飛び・1 枚だけ
            let all = all_tiles(&d);
            let sparse: Vec<TileCoord> = all.iter().copied().step_by(3).collect();
            for coords in [
                all.clone(),
                sparse,
                all[..1].to_vec(),
                all[all.len() - 2..].to_vec(),
            ] {
                let tiles = d.composite_tiles(ch, &coords).unwrap();
                assert_eq!(tiles.len(), coords.len());
                for (t, c) in tiles.iter().zip(&coords) {
                    assert_eq!(t.coord, *c);
                    assert_eq!(
                        t.pixels,
                        reference(&d, ch, t.rect),
                        "seed {seed} {ch:?} {c:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn tiles_outside_the_canvas_are_skipped_and_edge_tiles_are_trimmed() {
    let (d, _) = random_doc(3, 70, 50, 16);
    let coords = [
        TileCoord::new(0, 0),
        TileCoord::new(40, 40),
        TileCoord::new(4, 3),
    ];
    let tiles = d.composite_tiles(Channel::Color, &coords).unwrap();
    assert_eq!(tiles.len(), 2);
    let edge = &tiles[1];
    assert_eq!((edge.rect.width, edge.rect.height), (70 - 64, 50 - 48));
    assert_eq!(edge.pixels.len(), (6 * 2 * 4) as usize);
}

// ───────── 描いている間の下の覚え ─────────

use yolu_core::glam::DVec2;
use yolu_core::{BrushSettings, Rgba8};

fn brush(rng: &mut Rng, radius: f64) -> BrushSettings {
    BrushSettings {
        radius,
        hardness: 0.7,
        spacing: 0.2,
        opacity: 0.5 + rng.below(50) as f64 / 100.0,
        flow: 1.0,
        color: Rgba8::new(rng.bits() as u8, rng.bits() as u8, rng.bits() as u8, 255),
        pressure_size: false,
        pressure_opacity: false,
        pressure_flow: false,
        erase: rng.chance(15),
        anti_alias: yolu_core::AntiAlias::None,
    }
}

/// 2 つの文書（覚えを使う・使わない）の合成が、全チャンネル・全体・タイルの束・一部の矩形でバイトまで同じか。
fn assert_same(a: &Document, b: &Document, what: &str) {
    for ch in a.channels() {
        let whole = a.composite_channel(ch, a.bounds()).unwrap();
        assert_eq!(
            whole,
            b.composite_channel(ch, b.bounds()).unwrap(),
            "{what} {ch:?} 全体"
        );
        let coords = all_tiles(a);
        let (ta, tb) = (
            a.composite_tiles(ch, &coords).unwrap(),
            b.composite_tiles(ch, &coords).unwrap(),
        );
        for (x, y) in ta.iter().zip(&tb) {
            assert_eq!(x.pixels, y.pixels, "{what} {ch:?} タイル {:?}", x.coord);
        }
        let r = Rect::new(5, 3, 40, 33);
        assert_eq!(
            a.composite_channel(ch, r).unwrap(),
            b.composite_channel(ch, r).unwrap(),
            "{what} {ch:?} 矩形"
        );
    }
}

/// seed の文書で、`pick` 番目に描けるレイヤーへストロークを描き、途中・確定・Undo のどこでも、覚えを使う文書が使わない文書と同じ合成か。
/// 戻り値は覚えの状況（使った・作った・予算で飛ばした数）。
fn memo_case(seed: u64, pick: usize, budget: Option<u64>) -> Option<yolu_core::MemoStats> {
    memo_case_in(seed, pick, budget, (70, 50, 16))
}

fn memo_case_in(
    seed: u64,
    pick: usize,
    budget: Option<u64>,
    (width, height, tile): (u32, u32, u32),
) -> Option<yolu_core::MemoStats> {
    let (mut with, rasters) = random_doc(seed, width, height, tile);
    let (mut without, _) = random_doc(seed, width, height, tile);
    without.set_composite_memo(false);
    let candidates = paintable(&with, &rasters);
    if candidates.is_empty() {
        return None;
    }
    let index = with
        .layer_index(candidates[pick % candidates.len()])
        .unwrap();
    let (la, lb) = (with.layers()[index].id(), without.layers()[index].id());
    if let Some(b) = budget {
        with.set_stroke_budget_bytes(b).unwrap();
        without.set_stroke_budget_bytes(b).unwrap();
    }
    let mut rng = Rng(seed ^ 0xABCD);
    let radius = 3.0 + rng.below(8) as f64;
    let br = brush(&mut rng, radius);
    let (mut sa, mut sb) = (
        with.begin_stroke(la, &br).ok()?,
        without.begin_stroke(lb, &br).ok()?,
    );
    assert_same(&with, &without, &format!("seed {seed} 始め"));
    let mut x = rng.below(width as u64 - 10) as f64;
    let mut y = rng.below(height as u64 - 10) as f64;
    for step in 0..7 {
        x = (x + rng.below(14) as f64 - 5.0).clamp(0.0, width as f64 - 1.0);
        y = (y + rng.below(10) as f64 - 4.0).clamp(0.0, height as f64 - 1.0);
        let (ra, rb) = (
            sa.add_point(&mut with, x, y, 1.0, DVec2::ZERO),
            sb.add_point(&mut without, x, y, 1.0, DVec2::ZERO),
        );
        assert_eq!(ra.is_ok(), rb.is_ok(), "seed {seed} 点 {step}");
        if ra.is_err() {
            return Some(with.composite_memo_stats());
        }
        assert_same(&with, &without, &format!("seed {seed} 点 {step}"));
    }
    let stats = with.composite_memo_stats();
    with.end_stroke(sa).unwrap();
    without.end_stroke(sb).unwrap();
    assert_eq!(
        with.composite_memo_stats().tiles,
        0,
        "確定したら覚えを手放す"
    );
    assert_same(&with, &without, &format!("seed {seed} 確定"));
    with.undo().unwrap();
    without.undo().unwrap();
    assert_same(&with, &without, &format!("seed {seed} Undo"));
    Some(stats)
}

#[test]
fn the_remembered_composite_matches_the_full_composite_through_a_stroke() {
    let (mut hits, mut built) = (0, 0);
    for seed in 0..60u64 {
        for pick in 0..3 {
            if let Some(s) = memo_case(seed, pick * 5 + seed as usize, None) {
                hits += s.hits;
                built += s.built;
            }
        }
    }
    assert!(built > 0, "覚えを作った場面がある");
    assert!(hits > 0, "覚えから続けた場面がある");
}

#[test]
fn the_remembered_composite_matches_when_the_work_is_split_into_bands_across_threads() {
    // 128² のタイルに詰まったレイヤー: 仕事が大きく、タイルが行の帯に割れてワーカーへ分かれる（帯ごとに覚えの切り出しの位置が違う）
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let hits: u64 = pool.install(|| {
        (0..6u64)
            .filter_map(|seed| memo_case_in(seed, seed as usize, None, (256, 200, 128)))
            .map(|s| s.hits)
            .sum()
    });
    assert!(hits > 0);
}

#[test]
fn a_tight_memory_budget_skips_the_memory_and_gives_the_same_bytes() {
    let (mut skipped, mut built) = (0, 0);
    for seed in 0..40u64 {
        for budget in [3_000u64, 9_000, 30_000] {
            if let Some(s) = memo_case(seed, seed as usize * 3, Some(budget)) {
                skipped += s.skipped;
                built += s.built;
            }
        }
    }
    assert!(skipped > 0, "予算に収まらず飛ばしたタイルがある");
    assert!(built > 0, "収まるタイルは覚える");
}

/// 長いストロークを、巻き戻しが予算いっぱいまで育つまで描き、点ごとに（描いた直後と合成のあと）覚えのバイト + 巻き戻しが予算に収まり、
/// 覚えを使う文書の合成が使わない文書とバイトで同じか。戻り値は、覚えが手放したタイルの数と、予算で止まったか。
fn memo_long_stroke(seed: u64, budget: u64) -> Option<(u64, bool)> {
    let (width, height, tile) = (256u32, 96u32, 16u32);
    let (mut with, rasters) = random_doc(seed, width, height, tile);
    let (mut without, _) = random_doc(seed, width, height, tile);
    without.set_composite_memo(false);
    let candidates = paintable(&with, &rasters);
    if candidates.is_empty() {
        return None;
    }
    let index = with
        .layer_index(candidates[seed as usize % candidates.len()])
        .unwrap();
    let (la, lb) = (with.layers()[index].id(), without.layers()[index].id());
    with.set_stroke_budget_bytes(budget).unwrap();
    without.set_stroke_budget_bytes(budget).unwrap();
    let mut rng = Rng(seed ^ 0x5EED);
    let br = brush(&mut rng, 6.0);
    let (mut sa, mut sb) = (
        with.begin_stroke(la, &br).ok()?,
        without.begin_stroke(lb, &br).ok()?,
    );
    let fits = |d: &Document, what: &str| {
        let (memo, rollback) = (
            d.composite_memo_stats().bytes,
            d.active_stroke_stats().map_or(0, |s| s.rollback_bytes),
        );
        assert!(
            memo + rollback <= budget,
            "{what}: 覚え {memo} + 巻き戻し {rollback} が予算 {budget} を超えた"
        );
    };
    let mut stopped = false;
    let mut y = 10.0;
    for step in 0..60 {
        let x = 4.0 + (step as f64 * 9.0) % (width as f64 - 8.0);
        y = (y + 5.0) % (height as f64 - 12.0) + 6.0;
        let (ra, rb) = (
            sa.add_point(&mut with, x, y, 1.0, DVec2::ZERO),
            sb.add_point(&mut without, x, y, 1.0, DVec2::ZERO),
        );
        assert_eq!(ra.is_ok(), rb.is_ok(), "seed {seed} 点 {step}");
        if ra.is_err() {
            // 予算で取り消された: 覚えは手放す
            assert_eq!(with.composite_memo_stats().tiles, 0, "seed {seed} 取り消し");
            stopped = true;
            break;
        }
        fits(&with, &format!("seed {seed} 点 {step} 描いた直後"));
        assert_same(&with, &without, &format!("seed {seed} 点 {step}"));
        fits(&with, &format!("seed {seed} 点 {step} 合成のあと"));
    }
    let evicted = with.composite_memo_stats().evicted;
    if !stopped {
        with.end_stroke(sa).unwrap();
        without.end_stroke(sb).unwrap();
    }
    Some((evicted, stopped))
}

#[test]
fn the_memory_gives_way_when_the_rollback_grows_so_memory_plus_rollback_stays_in_the_budget() {
    let (mut evicted, mut stopped, mut ran) = (0, 0, 0);
    for seed in 0..24u64 {
        for budget in [24_000u64, 48_000, 96_000] {
            if let Some((e, s)) = memo_long_stroke(seed, budget) {
                ran += 1;
                evicted += e;
                stopped += s as u64;
            }
        }
    }
    assert!(ran > 0);
    assert!(stopped > 0, "巻き戻しが予算いっぱいまで育った場面がある");
    assert!(evicted > 0, "巻き戻しが育って、覚えが手放した場面がある");
}

#[test]
fn the_memory_is_not_used_when_it_is_switched_off() {
    let (mut d, rasters) = random_doc(5, 70, 50, 16);
    d.set_composite_memo(false);
    let id = paintable(&d, &rasters)[0];
    let mut rng = Rng(1);
    let br = brush(&mut rng, 6.0);
    let mut s = d.begin_stroke(id, &br).unwrap();
    s.add_point(&mut d, 30.0, 20.0, 1.0, DVec2::ZERO).unwrap();
    let _ = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    assert_eq!(d.composite_memo_stats().built, 0);
    d.end_stroke(s).unwrap();
}

// ───────── 歩幅つきの粗い合成 ─────────

use yolu_core::effects::{EffectSettings, FilterSpec};
use yolu_core::FilterTarget;

/// 粗いタイルの画素 (i, j) が、全体の合成の (x0 + i·歩幅, y0 + j·歩幅) の画素とバイトで同じか。
fn assert_samples_of(d: &Document, ch: Channel, stride: u32, what: &str) {
    let whole = d.composite_channel(ch, d.bounds()).unwrap();
    let w = d.width() as usize;
    let coords = all_tiles(d);
    for t in d.composite_coarse_tiles(ch, &coords, stride).unwrap() {
        let (cw, ch_) = t.size();
        assert_eq!(t.pixels.len(), (cw * ch_ * 4) as usize);
        for j in 0..ch_ {
            for i in 0..cw {
                let (x, y) = (t.rect.x + i * stride, t.rect.y + j * stride);
                let want = &whole[(y as usize * w + x as usize) * 4..][..4];
                let got = &t.pixels[((j * cw + i) * 4) as usize..][..4];
                assert_eq!(
                    got, want,
                    "{what} {ch:?} 歩幅 {stride} タイル {:?} ({i},{j})",
                    t.coord
                );
            }
        }
    }
}

#[test]
fn a_coarse_composite_is_the_exact_composite_taken_every_stride_pixels() {
    for seed in 0..40u64 {
        let (d, _) = random_doc(seed, 70, 50, 16);
        for ch in [Channel::Color, Channel::Normal] {
            for stride in [1, 2, 4, 8, 16] {
                assert_samples_of(&d, ch, stride, &format!("seed {seed}"));
            }
        }
    }
}

#[test]
fn a_stride_that_does_not_divide_the_tile_is_refused() {
    let (d, _) = random_doc(1, 70, 50, 16);
    for stride in [0, 3, 5, 32] {
        assert!(
            d.composite_coarse_tiles(Channel::Color, &all_tiles(&d), stride)
                .is_err(),
            "{stride}"
        );
    }
}

/// 点ごとの効果（レベル補正・反転）を足した文書。
fn with_point_effects(seed: u64) -> Document {
    let (mut d, rasters) = random_doc(seed, 70, 50, 16);
    for (k, id) in paintable(&d, &rasters).into_iter().enumerate().take(3) {
        let fx = if k % 2 == 0 {
            EffectSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95)
        } else {
            EffectSettings::invert()
        };
        d.add_filter(id, FilterTarget::Content, FilterSpec::new(fx))
            .unwrap();
    }
    d
}

#[test]
fn coarse_effects_of_pointwise_filters_equal_the_exact_output_at_the_sample_points() {
    for seed in 0..25u64 {
        let d = with_point_effects(seed);
        // 評価していない（粗く評価する）道
        assert!(
            d.effects_pending(Channel::Color, &all_tiles(&d))
                || d.layers()
                    .iter()
                    .all(|l| l.surface(Channel::Color).is_none())
        );
        let before = d.effect_counters().blocks_evaluated;
        for stride in [2, 4, 8] {
            let coords = all_tiles(&d);
            // 先に粗く（キャッシュは空）、そのあと正確に（キャッシュが埋まる）
            let coarse = d
                .composite_coarse_tiles(Channel::Color, &coords, stride)
                .unwrap();
            if stride == 2 {
                assert_eq!(
                    d.effect_counters().blocks_evaluated,
                    before,
                    "粗い評価はキャッシュに入れない"
                );
            }
            let whole = d.composite_channel(Channel::Color, d.bounds()).unwrap();
            let w = d.width() as usize;
            for t in &coarse {
                let (cw, chh) = t.size();
                for j in 0..chh {
                    for i in 0..cw {
                        let (x, y) = (t.rect.x + i * stride, t.rect.y + j * stride);
                        assert_eq!(
                            &t.pixels[((j * cw + i) * 4) as usize..][..4],
                            &whole[(y as usize * w + x as usize) * 4..][..4],
                            "seed {seed} 歩幅 {stride} {:?} ({i},{j})",
                            t.coord
                        );
                    }
                }
            }
            d.release_effect_cache();
        }
    }
}

#[test]
fn with_every_block_evaluated_a_coarse_composite_picks_the_exact_output() {
    let d = with_point_effects(3);
    let coords = all_tiles(&d);
    let _ = d.composite_tiles(Channel::Color, &coords).unwrap(); // キャッシュを埋める
    assert!(!d.effects_pending(Channel::Color, &coords));
    let blocks = d.effect_counters().blocks_evaluated;
    assert_samples_of(&d, Channel::Color, 4, "評価済み");
    assert_eq!(
        d.effect_counters().blocks_evaluated,
        blocks,
        "評価し直さない"
    );
}

#[test]
fn effects_are_pending_until_their_blocks_are_evaluated_and_again_after_a_change() {
    let (mut d, rasters) = random_doc(11, 70, 50, 16);
    let id = paintable(&d, &rasters)[0];
    let fx = d
        .add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(4)),
        )
        .unwrap();
    let coords = all_tiles(&d);
    assert!(d.effects_pending(Channel::Color, &coords));
    let _ = d.composite_tiles(Channel::Color, &coords).unwrap();
    assert!(!d.effects_pending(Channel::Color, &coords));
    d.set_filter_settings(id, fx, EffectSettings::blur(6), true)
        .unwrap();
    assert!(
        d.effects_pending(Channel::Color, &coords),
        "設定を変えたら評価し直しが要る"
    );
    assert!(d.is_coalescing());
    d.end_coalescing();
    assert!(!d.is_coalescing());
}

#[test]
fn a_coarse_blur_stays_close_to_the_exact_blur_on_a_smooth_picture() {
    let mut d = Document::with_tile_size(96, 64, 32).unwrap();
    let id = d.add_layer("grad").unwrap();
    for y in 0..64u32 {
        for x in 0..96u32 {
            d.set_pixel(id, x, y, Rgba8::new((x * 2) as u8, (y * 3) as u8, 128, 255))
                .unwrap();
        }
    }
    d.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(8)),
    )
    .unwrap();
    let coords = all_tiles(&d);
    let coarse = d
        .composite_coarse_tiles(Channel::Color, &coords, 4)
        .unwrap();
    let exact = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let (mut worst, mut count) = (0i32, 0);
    for t in &coarse {
        let (cw, chh) = t.size();
        for j in 0..chh {
            for i in 0..cw {
                let (x, y) = ((t.rect.x + i * 4) as usize, (t.rect.y + j * 4) as usize);
                // 端（ぼかしが端の画素を繰り返す所）は除く
                if x < 16 || y < 16 || x + 16 >= 96 || y + 16 >= 64 {
                    continue;
                }
                for c in 0..3 {
                    let a = t.pixels[((j * cw + i) * 4) as usize + c] as i32;
                    let b = exact[(y * 96 + x) * 4 + c] as i32;
                    worst = worst.max((a - b).abs());
                    count += 1;
                }
            }
        }
    }
    assert!(count > 100);
    assert!(
        worst <= 6,
        "粗いぼかしは正確なぼかしに近い: 最大の差 {worst}"
    );
}
