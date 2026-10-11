//! アイランドごとのばらつき（Generator の種類 67）: アイランドの中は同じ値・アイランドごとに違う値・シードで変わる・最小と最大の範囲（等しいのも）・
//! アイランドの外は入力のまま・重なったテクセルは番号の小さい三角形のアイランドの値・1 画素の式と行の式が同じ・領域・スレッドの数・タイルの大きさ・
//! 解像度によらない・モデルが無い・予算で断られた・図を渡す前は理由つきで入力のまま・モデルを替えたら評価し直す・欄の検査・
//! SIMD の道（`YOLU_SIMD`）によらないバイト・アイランドの図を初めて作る間に評価が固まらない。
//!
//! モデルは試験で組む: 3D で離れた 4 つの四角。A・B・C は UV の別の所、D は B と UV の一部が重なる（アイランドの番号は三角形の順で A 1・B 2・C 3・D 4）。
use std::sync::Arc;

use crate::rayon_support::{assert_finishes_alike, atlas};

use yolu_core::generator::*;
use yolu_core::geometry::{
    IslandMap, SurfaceGeometry, SurfaceTriangle, UvTopology, DEFAULT_BUDGET, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterId, FilterSpec, FilterTarget,
    InactiveReason, LayerId, Rect, Rgba8,
};

const SIZE: u32 = 64;
const BASE: Rgba8 = Rgba8::new(90, 140, 200, 255);

/// 3D の四角（x 0..1, y 0..1, z）の 2 つの三角形。uv は四角の中の (x, y)（0..1）から UV へ。
fn quad(z: f32, uv: impl Fn(f32, f32) -> Vec2) -> [SurfaceTriangle; 2] {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    [
        SurfaceTriangle::new(
            p(0., 0.),
            p(1., 0.),
            p(1., 1.),
            uv(0., 0.),
            uv(1., 0.),
            uv(1., 1.),
        ),
        SurfaceTriangle::new(
            p(0., 0.),
            p(1., 1.),
            p(0., 1.),
            uv(0., 0.),
            uv(1., 1.),
            uv(0., 1.),
        ),
    ]
}

fn lower_left(x: f32, y: f32) -> Vec2 {
    Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y)
}
fn upper_left(x: f32, y: f32) -> Vec2 {
    Vec2::new(0.1 + 0.3 * x, 0.6 + 0.3 * y)
}

/// 四角の中の (x, y) から UV へ。
type UvOf = fn(f32, f32) -> Vec2;

/// A・B・C・D の 4 つのアイランド。`swapped` なら A と C の UV を入れ替える（A はアイランド 1 のまま上に、C はアイランド 3 のまま下に）。
fn islands(swapped: bool) -> Arc<UvTopology> {
    let (a, c): (UvOf, UvOf) = if swapped {
        (upper_left, lower_left)
    } else {
        (lower_left, upper_left)
    };
    let mut t = quad(0., a).to_vec();
    t.extend(quad(5., |x, y| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y)));
    t.extend(quad(10., c));
    t.extend(quad(15., |x, y| Vec2::new(0.75 + 0.2 * x, 0.3 + 0.3 * y)));
    let g = SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    Arc::new(UvTopology::new(Arc::new(g), Some(0)))
}

/// 64² のアイランドごとの代表のテクセル（A・B・C・D の重ならない所）と、B と D が重なるテクセル・アイランドの外のテクセル。
const A: (u32, u32) = (16, 16);
const B: (u32, u32) = (45, 12);
const C: (u32, u32) = (16, 48);
const D: (u32, u32) = (60, 30);
const OVERLAP: (u32, u32) = (52, 22);
const OUTSIDE: (u32, u32) = (40, 40);

fn no_anchor() -> Result<&'static dyn anchor::ValueSource, anchor::Issue> {
    Err(anchor::Issue::NotChosen)
}

fn bound<'a>(s: &'a Settings, map: &Arc<IslandMap>) -> BoundGenerator<'a> {
    BoundGenerator::bind(s, &[], None, (map.width(), map.height()), no_anchor())
        .unwrap()
        .with_islands(map.clone())
}

/// 基底の値からレベル・減衰・反転を当てた値（`BoundGenerator::value` の後半と同じ式）。
fn levelled(s: &Settings, base: f64) -> f64 {
    let mut t = ((base - s.low) / (s.high - s.low)).clamp(0., 1.);
    if s.softness > 0. {
        t += s.softness * (t * t * (3. - 2. * t) - t);
    }
    if s.invert {
        1. - t
    } else {
        t
    }
}

fn settings(seed: i32) -> Settings {
    let mut s = Settings::new(Kind::UvIslandVariation);
    s.island.seed = seed;
    s
}

/// レベル・減衰・反転・最小と最大の変種。
fn variants() -> Vec<Settings> {
    (0..6)
        .map(|v: usize| {
            let mut s = settings([0, 7, -123_456_789][v % 3]);
            s.low = [0., 0.1, 0.25][v % 3];
            s.high = [1., 0.9, 0.8][v % 3];
            s.softness = [0., 0.5, 1.][(v + 1) % 3];
            s.invert = v % 2 == 1;
            s.blend = [Blend::Replace, Blend::Multiply, Blend::Screen][v % 3];
            (s.island.min, s.island.max) = [(0., 1.), (0.2, 0.3), (0.6, 0.6)][v % 3];
            s.validate().unwrap();
            s
        })
        .collect()
}

#[test]
fn each_island_has_one_value_from_its_number_and_the_seed() {
    let model = islands(false);
    assert_eq!(model.island_count(), 4);
    let map = model.island_map(SIZE, SIZE).unwrap();
    for (i, (x, y)) in [A, B, C, D].into_iter().enumerate() {
        assert_eq!(map.island(x, y), i as u32 + 1, "({x}, {y})");
    }
    for s in variants() {
        let b = bound(&s, &map);
        assert!(b.inactive().is_none());
        for y in 0..SIZE {
            for x in 0..SIZE {
                let island = map.island(x, y);
                let want = (island != 0).then(|| levelled(&s, s.island.value(island)));
                assert_eq!(
                    b.value(x, y).map(f64::to_bits),
                    want.map(f64::to_bits),
                    "({x}, {y})"
                );
            }
        }
    }
    // アイランドごとに違う値（既定の最小 0・最大 1）、シードを変えると全部のアイランドで変わる
    let s = settings(0);
    let values: Vec<f64> = (1..=4).map(|i| s.island.value(i)).collect();
    for (i, v) in values.iter().enumerate() {
        assert!((0. ..1.).contains(v), "{v}");
        assert!(values[i + 1..].iter().all(|o| o != v), "{values:?}");
    }
    let other = settings(1);
    for i in 1..=4 {
        assert_ne!(other.island.value(i), s.island.value(i), "アイランド {i}");
    }
    // 最小・最大は u を線形に写す（同じシードの u から、丸めまで同じ式）。等しければアイランドによらず同じ値
    let mut narrow = settings(0);
    (narrow.island.min, narrow.island.max) = (0.2, 0.3);
    let mut flat = settings(0);
    (flat.island.min, flat.island.max) = (0.6, 0.6);
    for i in 1..=4 {
        let u = s.island.value(i);
        assert_eq!(
            narrow.island.value(i).to_bits(),
            (0.2 + (0.3 - 0.2) * u).to_bits()
        );
        assert!((0.2..0.3).contains(&narrow.island.value(i)));
        assert_eq!(flat.island.value(i), 0.6);
    }
}

#[test]
fn outside_texels_keep_the_input_and_overlaps_take_the_lower_triangles_island() {
    let map = islands(false).island_map(SIZE, SIZE).unwrap();
    let s = settings(42);
    let b = bound(&s, &map);
    // アイランドの外は値を持たない
    assert_eq!(b.value(OUTSIDE.0, OUTSIDE.1), None);
    assert_eq!(b.value(0, 0), None);
    // B と D が重なるテクセルは、番号の小さい三角形（B）のアイランドの値
    assert!(map.overlapped(OVERLAP.0, OVERLAP.1));
    assert_eq!(map.island(OVERLAP.0, OVERLAP.1), 2);
    assert_eq!(b.value(OVERLAP.0, OVERLAP.1), b.value(B.0, B.1));
    assert_ne!(b.value(OVERLAP.0, OVERLAP.1), b.value(D.0, D.1));
    // 評価してもアイランドの外の画素は入力のバイトのまま
    let source = source_image();
    let image = Image::new(&source, SIZE, SIZE).unwrap();
    for target in [Target::Color, Target::Scalar, Target::Mask] {
        let out = evaluate(
            &image,
            &b,
            Rect::new(0, 0, SIZE, SIZE),
            target,
            1.,
            &Options::default(),
        )
        .unwrap();
        assert!(out.inactive.is_none());
        let at = |p: &[u8], (x, y): (u32, u32)| {
            let i = ((y * SIZE + x) * 4) as usize;
            <[u8; 4]>::try_from(&p[i..i + 4]).unwrap()
        };
        let mut want = at(&source, OUTSIDE);
        if target == Target::Mask {
            want[..3].fill(0);
        }
        assert_eq!(at(&out.pixels, OUTSIDE), want, "{target:?}");
    }
}

fn source_image() -> Vec<u8> {
    (0..SIZE * SIZE)
        .flat_map(|i| [(i * 7) as u8, (i * 13) as u8, (i * 3) as u8, 255])
        .collect()
}

#[test]
fn rows_regions_and_threads_give_the_same_bytes_as_single_pixels() {
    let map = islands(false).island_map(SIZE, SIZE).unwrap();
    let source = source_image();
    let image = Image::new(&source, SIZE, SIZE).unwrap();
    for s in variants() {
        let b = bound(&s, &map);
        for scalar in [false, true] {
            for y in 0..SIZE {
                // 行の途中から始める（連なりの途中で切れる行）
                for x0 in [0, 13] {
                    let mut row = vec![None; (SIZE - x0) as usize];
                    b.sample_row(x0, y, scalar, &mut row);
                    for (k, got) in row.into_iter().enumerate() {
                        let x = x0 + k as u32;
                        assert_eq!(got, b.sample(x, y, scalar), "({x}, {y})");
                    }
                }
            }
        }
        for target in [Target::Color, Target::Scalar, Target::Mask] {
            let whole = |threads: usize| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap()
                    .install(|| {
                        evaluate(
                            &image,
                            &b,
                            Rect::new(0, 0, SIZE, SIZE),
                            target,
                            0.8,
                            &Options::default(),
                        )
                        .unwrap()
                        .pixels
                    })
            };
            let all = whole(1);
            assert_eq!(whole(4), all, "{target:?}");
            let r = Rect::new(5, 7, 41, 33);
            let part = evaluate(&image, &b, r, target, 0.8, &Options::default())
                .unwrap()
                .pixels;
            for y in 0..r.height {
                let a = (((r.y + y) * SIZE + r.x) * 4) as usize;
                let p = (y * r.width * 4) as usize;
                assert_eq!(
                    &all[a..a + (r.width * 4) as usize],
                    &part[p..p + (r.width * 4) as usize],
                    "{target:?}"
                );
            }
        }
    }
}

#[test]
fn island_values_do_not_depend_on_the_resolution() {
    let model = islands(false);
    let s = settings(-9);
    let small = model.island_map(SIZE, SIZE).unwrap();
    let large = model.island_map(SIZE * 2, SIZE * 2).unwrap();
    let wide = model.island_map(SIZE * 3, SIZE / 2 * 3).unwrap();
    let (bs, bl) = (bound(&s, &small), bound(&s, &large));
    let bw = bound(&s, &wide);
    for (x, y) in [A, B, C, D] {
        let v = bs.value(x, y).unwrap();
        assert_eq!(large.island(x * 2, y * 2), small.island(x, y));
        assert_eq!(bl.value(x * 2, y * 2), Some(v), "({x}, {y})");
        assert_eq!(wide.island(x * 3, y * 3 / 2), small.island(x, y));
        assert_eq!(bw.value(x * 3, y * 3 / 2), Some(v), "({x}, {y})");
    }
}

#[test]
fn before_the_map_is_given_and_when_it_cannot_be_used_the_stage_passes_its_input_through() {
    let s = settings(0);
    // アイランドの図を渡す前は、モデルが無いのと同じ
    let b = BoundGenerator::bind(&s, &[], None, (SIZE, SIZE), no_anchor()).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::NoModel));
    assert_eq!(b.value(A.0, A.1), None);
    // 予算で断られた
    let b = BoundGenerator::bind(&s, &[], None, (SIZE, SIZE), no_anchor())
        .unwrap()
        .islands_refused();
    assert_eq!(b.inactive(), Some(&Inactive::IslandMap));
    // 大きさの違う図は使わない
    let map = islands(false).island_map(SIZE * 2, SIZE).unwrap();
    let b = BoundGenerator::bind(&s, &[], None, (SIZE, SIZE), no_anchor())
        .unwrap()
        .with_islands(map.clone());
    assert_eq!(b.inactive(), Some(&Inactive::IslandMap));
    // ほかの種類には何もしない
    let pattern = Settings::new(Kind::Pattern);
    let b = BoundGenerator::bind(&pattern, &[], None, (SIZE * 2, SIZE), no_anchor())
        .unwrap()
        .with_islands(map)
        .islands_refused();
    assert!(b.inactive().is_none());
}

/// 全面を BASE で塗ったレイヤーに、アイランドごとのばらつきの段（色・置き換え）を足した文書。
fn document(tile: u32, s: &Settings) -> (Document, LayerId, FilterId) {
    let mut doc = Document::with_tile_size(SIZE, SIZE, tile).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            doc.set_pixel(layer, x, y, BASE).unwrap();
        }
    }
    let id = doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(s.clone())).channels(&[Channel::Color]),
        )
        .unwrap();
    (doc, layer, id)
}

fn with_model(doc: &mut Document, model: &Arc<UvTopology>) {
    doc.set_effect_inputs(EffectInputs::new().with_topology(Some(model.clone())))
        .unwrap();
}

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, SIZE, SIZE))
        .unwrap()
}

fn at(pixels: &[u8], (x, y): (u32, u32)) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

fn replace(seed: i32) -> Settings {
    let mut s = settings(seed);
    s.blend = Blend::Replace;
    s
}

#[test]
fn a_document_without_a_model_or_over_the_budget_keeps_the_input_and_says_why() {
    let s = replace(3);
    let (mut doc, layer, id) = document(16, &s);
    let reason = |doc: &Document| doc.generator_inactive(layer, id).unwrap();
    let base: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| BASE.to_array()).collect();
    // モデルが無い: 入力のまま、理由はモデルが無いこと（効かない効果の一覧にも出る）
    assert_eq!(whole(&doc), base);
    assert_eq!(
        reason(&doc),
        Some(InactiveReason::Generator(Inactive::NoModel))
    );
    let list = doc.inactive_effect_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, InactiveReason::Generator(Inactive::NoModel));
    // アイランドの図の予算に収まらない: 入力のまま、理由はアイランドの図
    doc.set_seam_cache_budget_bytes(64);
    with_model(&mut doc, &islands(false));
    assert_eq!(whole(&doc), base);
    assert_eq!(
        reason(&doc),
        Some(InactiveReason::Generator(Inactive::IslandMap))
    );
    // 予算を戻すと評価し直す（描き直す印も付く）
    let since = doc.change_serial();
    doc.set_seam_cache_budget_bytes(DEFAULT_BUDGET);
    assert!(!doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
    let now = whole(&doc);
    assert_eq!(reason(&doc), None);
    assert!(doc.inactive_effect_list().is_empty());
    // アイランドの中はアイランドごとに 1 つの灰色、アイランドの外は入力のまま
    let map = islands(false).island_map(SIZE, SIZE).unwrap();
    let mut colours = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = at(&now, (x, y));
            match map.island(x, y) {
                0 => assert_eq!(p, BASE.to_array(), "({x}, {y})"),
                i => {
                    assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 255, "{p:?}");
                    let slot = i as usize - 1;
                    if colours.len() <= slot {
                        colours.resize(slot + 1, None);
                    }
                    assert_eq!(
                        *colours[slot].get_or_insert(p),
                        p,
                        "アイランド {i} ({x}, {y})"
                    );
                }
            }
        }
    }
    assert_eq!(colours.len(), 4);
}

#[test]
fn another_model_re_evaluates_and_the_same_one_does_not() {
    let s = replace(5);
    let (mut doc, _, _) = document(16, &s);
    with_model(&mut doc, &islands(false));
    let first = whole(&doc);
    // A と C の UV を入れ替えたモデル: 左下のアイランドは C（アイランド 3）になり、値が変わる
    let since = doc.change_serial();
    with_model(&mut doc, &islands(true));
    assert!(!doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
    let second = whole(&doc);
    assert_ne!(at(&second, A), at(&first, A));
    assert_eq!(at(&second, A), at(&first, C));
    assert_eq!(at(&second, C), at(&first, A));
    // 覚えていた評価を使わず、初めからそのモデルで評価した文書と同じ
    let (mut fresh, _, _) = document(16, &s);
    with_model(&mut fresh, &islands(true));
    assert_eq!(second, whole(&fresh));
    // 同じ位相を渡し直しても何も変わらない
    let since = doc.change_serial();
    with_model(&mut doc, &islands(true));
    assert!(doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
}

#[test]
fn tile_sizes_do_not_change_the_bytes() {
    let mut s = replace(11);
    s.low = 0.2;
    s.softness = 0.7;
    let model = islands(false);
    let run = |tile: u32| {
        let (mut doc, _, _) = document(tile, &s);
        with_model(&mut doc, &model);
        whole(&doc)
    };
    let reference = run(64);
    assert_eq!(run(8), reference);
    assert_eq!(run(16), reference);
    assert_eq!(run(32), reference);
}

#[test]
fn fields_outside_the_kind_are_refused() {
    let ok = settings(i32::MIN);
    ok.validate().unwrap();
    let mut s = settings(0);
    (s.island.min, s.island.max) = (0.7, 0.3);
    assert!(s.validate().is_err());
    assert!(BoundGenerator::bind(&s, &[], None, (SIZE, SIZE), no_anchor()).is_err());
    (s.island.min, s.island.max) = (0., 1.5);
    assert!(s.validate().is_err());
    // 重ねるノイズは持たない
    for f in [
        |s: &mut Settings| s.noise_amount = 0.3,
        |s: &mut Settings| s.noise_seed = 1,
        |s: &mut Settings| s.noise_scale = 0.1,
        |s: &mut Settings| s.noise_space = NoiseSpace::Uv,
    ] {
        let mut s = settings(0);
        f(&mut s);
        assert!(s.validate().is_err());
    }
    // 共通の減衰は効かせる（模様・ライトと違って自分のぼかしを持たない）
    let mut soft = settings(0);
    soft.softness = 0.5;
    soft.validate().unwrap();
    // ピンは持たない、ほかの種類はアイランドの設定を持たない
    assert!(Settings::new(Kind::UvIslandVariation)
        .candidate_maps()
        .is_empty());
    let mut pattern = Settings::new(Kind::Pattern);
    pattern.island.seed = 1;
    assert!(pattern.validate().is_err());
    assert_eq!(Kind::from_index(67), Some(Kind::UvIslandVariation));
    assert!(Kind::UvIslandVariation.is_050() && Kind::UvIslandVariation.is_rust_only());
}

#[test]
fn the_bytes_are_pinned_on_every_simd_path() {
    // `YOLU_SIMD=scalar|sse41|avx2|neon`（その CPU にある道）で同じ値になる（レベル・減衰・反転は SIMD の道、アイランドの値は整数の hash と丸めの無い掛け算）
    let mut s = settings(77);
    s.low = 0.15;
    s.high = 0.85;
    s.softness = 0.6;
    s.invert = true;
    s.blend = Blend::Screen;
    (s.island.min, s.island.max) = (0.1, 0.95);
    let (mut doc, _, _) = document(16, &s);
    with_model(&mut doc, &islands(false));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in whole(&doc) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    assert_eq!(h, PINNED, "{h:#018x}");
}

const PINNED: u64 = 0xfb62_7427_4389_6d1f;

// ── 固まらない ──────────────────────────────────────────────────────────────────────────────────────────────────────────────
//
// アイランドの図は、初めて要るときに rayon の並列で作る。作る間は門を持つので、評価のブロックの中から初めて要ると、作っている最中のスレッドが、待つ間に
// 別のブロックを拾って同じ門を待つと止まる。モデルを渡した直後（アイランドの図が無い）の、ブロックが何個にもなる評価で確かめる。

/// 全面を塗った塗りつぶしレイヤーにアイランドごとのばらつきの段（置き換え）を重ねた、512² の文書（タイル 16²・ブロック 32² の 256 個）と、全部のタイル
/// （ブロックがスレッドより十分多いほど、固まりやすい）。
/// 継ぎ目の縁もアイランドも多いモデルを渡しただけで、アイランドの図はまだ作っていない。
fn cold_big_document() -> (Document, Vec<yolu_core::TileCoord>) {
    const BIG: u32 = 512;
    let mut doc = Document::with_tile_size(BIG, BIG, 16).unwrap();
    doc.set_filter_block_pixels(32).unwrap();
    let layer = doc
        .add_fill_layer("塗り", &[(Channel::Color, BASE)], None)
        .unwrap();
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(replace(3))).channels(&[Channel::Color]),
    )
    .unwrap();
    with_model(&mut doc, &atlas(12));
    let tiles = (0..BIG / 16)
        .flat_map(|y| (0..BIG / 16).map(move |x| yolu_core::TileCoord::new(x, y)))
        .collect();
    (doc, tiles)
}

/// 固まりは、仕事を拾うタイミングで決まる（出ない回もある）ので、冷えた文書を作り直して何回か。
const ROUNDS: usize = 4;

#[test]
fn the_first_evaluation_of_island_variation_does_not_hang() {
    for round in 0..ROUNDS {
        assert_finishes_alike(
            &format!("アイランドごとのばらつきの評価（{round}）"),
            cold_big_document,
            |(doc, tiles)| {
                let composed = doc.composite_tiles(Channel::Color, tiles).unwrap();
                assert_eq!(composed.len(), tiles.len());
                composed.into_iter().map(|t| t.pixels).collect::<Vec<_>>()
            },
        );
    }
}

#[test]
fn the_coarse_evaluation_of_island_variation_does_not_hang() {
    for round in 0..ROUNDS {
        assert_finishes_alike(
            &format!("アイランドごとのばらつきの粗い評価（{round}）"),
            cold_big_document,
            |(doc, tiles)| {
                let composed = doc
                    .composite_coarse_tiles(Channel::Color, tiles, 2)
                    .unwrap();
                assert_eq!(composed.len(), tiles.len());
                composed.into_iter().map(|t| t.pixels).collect::<Vec<_>>()
            },
        );
    }
}
