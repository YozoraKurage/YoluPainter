//! レイヤーのフィルターが UV の継ぎ目をまたぐ（文書の設定、モデルの UV の位相が効果の入力にあるときだけ）: 縁の向こうのアイランドの色が入る・
//! アイランドの外は段の入力のまま・設定を切ると（モデルが無いのと）同じバイト・1 回の Undo・相手のアイランドに描くと縁の向こうの出力と変化の印が
//! 変わる・ブロックやタイルの大きさによらない・接空間の法線は継ぎ目の向きで回る・作業メモリの予算で断る・取り消せる・アイランドの図と帯の写しが
//! 予算に収まらなければ 2D のまま評価して、それが分かる。
//! 初めて要る帯の写しを rayon の並列で作る間に評価が固まらない（ドラッグ中の粗い評価・文書を開いた直後の評価・Anchor が読む出力。レイヤーの内容もマスクも）。
//!
//! モデルは試験で組む: 3D で 1 辺を共有する 2 つの四角（A は x 0..1、B は x 1..2）を、64² の UV の別の所に置く。A は赤、B は青で塗る。
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::rayon_support::{assert_finishes_alike, atlas};
use yolu_core::filter::{self, Options, Settings, Source, Stage, ValueType};
use yolu_core::generator::{self, MapKind, MapState};
use yolu_core::geometry::{
    SurfaceGeometry, SurfaceTriangle, UvTopology, UvTopologyError, DEFAULT_BUDGET,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterSpec, FilterTarget, LayerId, MapInput,
    Rect, Rgba8, SeamFallback, TileCoord,
};

const SIZE: u32 = 64;
const RED: Rgba8 = Rgba8::new(220, 20, 20, 255);
const BLUE: Rgba8 = Rgba8::new(20, 20, 220, 255);

fn quad(x0: f32, z: f32, uv: impl Fn(f32, f32) -> Vec2) -> [SurfaceTriangle; 2] {
    let p = |x: f32, y: f32| Vec3::new(x0 + x, y, z);
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

fn model(b: impl Fn(f32, f32) -> Vec2) -> Arc<UvTopology> {
    let mut t = quad(0., 0., |x, y| Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y)).to_vec();
    t.extend(quad(1., 0., b));
    let g = SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    Arc::new(UvTopology::new(Arc::new(g), Some(0)))
}

fn straight() -> Arc<UvTopology> {
    model(|x, y| Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y))
}

/// A（アイランド 1）を赤、B（アイランド 2）を青で塗ったレイヤー（tile はタイルの大きさ）。
fn painted(topology: &UvTopology, tile: u32) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(SIZE, SIZE, tile).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    let map = topology.island_map(SIZE, SIZE).unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            match map.island(x, y) {
                1 => doc.set_pixel(layer, x, y, RED).unwrap(),
                2 => doc.set_pixel(layer, x, y, BLUE).unwrap(),
                _ => false,
            };
        }
    }
    (doc, layer)
}

fn blur(doc: &mut Document, layer: LayerId, radius: u32) {
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(radius)).channels(&[Channel::Color]),
    )
    .unwrap();
}

fn with_model(doc: &mut Document, topology: &Arc<UvTopology>) {
    doc.set_effect_inputs(EffectInputs::new().with_topology(Some(topology.clone())))
        .unwrap();
}

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, SIZE, SIZE))
        .unwrap()
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn a_blur_reads_the_other_island_across_the_seam() {
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 4);
    let flat = whole(&doc);
    with_model(&mut doc, &topology);
    assert!(doc.seams_active());
    let across = whole(&doc);
    // A の右の縁の内側: 2D では外の透明と混ざり、またぐと B の青が入る（不透明のまま）
    let (f, a) = (at(&flat, 25, 16), at(&across, 25, 16));
    assert!(f[3] < 255 && f[2] < 40, "2D: {f:?}");
    assert!(a[3] == 255 && a[2] > 40, "またぐ: {a:?}");
    // B の左の縁の内側にも A の赤が入る
    assert!(at(&across, 38, 16)[0] > 40);
    // アイランドの外は段の入力（塗っていない透明）のまま
    for x in 26..38 {
        assert_eq!(at(&across, x, 16), [0, 0, 0, 0], "x {x}");
    }
    assert_ne!(
        at(&flat, 26, 16)[3],
        0,
        "2D のぼかしはアイランドの外へも広がる"
    );
    // 継ぎ目から半径より遠いアイランドの中は 2D と同じ（A の左は開いた縁）
    for y in 7..26 {
        for x in 6..20 {
            assert_eq!(at(&across, x, y), at(&flat, x, y), "({x}, {y})");
        }
    }
}

#[test]
fn turning_the_setting_off_is_the_same_as_no_model_and_one_undo() {
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 4);
    let flat = whole(&doc);
    with_model(&mut doc, &topology);
    let across = whole(&doc);
    assert!(doc.filter_seams());
    let count = doc.undo_count();
    doc.set_filter_seams(false).unwrap();
    assert_eq!(doc.undo_count(), count + 1);
    assert!(!doc.seams_active());
    assert_eq!(whole(&doc), flat);
    assert!(doc.undo().unwrap());
    assert!(doc.filter_seams());
    assert_eq!(whole(&doc), across);
    assert!(doc.redo().unwrap());
    assert_eq!(whole(&doc), flat);
    // 同じ値にしても段は増えない
    let count = doc.undo_count();
    doc.set_filter_seams(false).unwrap();
    assert_eq!(doc.undo_count(), count);
}

fn green(doc: &mut Document, layer: LayerId) {
    let green = Rgba8::new(20, 220, 20, 255);
    for y in 14..19 {
        doc.set_pixel(layer, 38, y, green).unwrap();
        doc.set_pixel(layer, 39, y, green).unwrap();
    }
}

#[test]
fn painting_the_other_island_changes_the_output_across_the_seam() {
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 4);
    with_model(&mut doc, &topology);
    let _ = whole(&doc); // 評価を覚えさせる
    let since = doc.change_serial();
    // B の左の縁の内側（タイル (2, 1)）を緑にする
    green(&mut doc, layer);
    // 縁の向こうの A のタイル (1, 1) も変わり得る所に入る
    let changed = doc.changed_tiles(Channel::Color, since).unwrap();
    assert!(changed.contains(&TileCoord::new(1, 1)), "{changed:?}");
    // 覚えていた評価を使わず、同じ画素から作り直した文書と同じ
    let (mut fresh, l) = painted(&topology, 16);
    green(&mut fresh, l);
    blur(&mut fresh, l, 4);
    with_model(&mut fresh, &topology);
    let now = whole(&doc);
    assert_eq!(now, whole(&fresh));
    assert!(
        at(&now, 25, 16)[1] > 40,
        "A の縁に緑: {:?}",
        at(&now, 25, 16)
    );
}

#[test]
fn blocks_tiles_and_stacked_neighbourhoods_do_not_change_the_bytes() {
    let topology = straight();
    let run = |tile: u32, block: u32| {
        let (mut doc, layer) = painted(&topology, tile);
        doc.set_filter_block_pixels(block).unwrap();
        // 近傍の段を 2 つ重ねる（後の段の帯は、前の段の出力を相手のアイランドで読み直す）
        blur(&mut doc, layer, 3);
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::sharpen(2, 1.0, 0)).channels(&[Channel::Color]),
        )
        .unwrap();
        with_model(&mut doc, &topology);
        whole(&doc)
    };
    let reference = run(64, 64);
    assert_eq!(run(16, 16), reference);
    assert_eq!(run(8, 32), reference);
    assert_eq!(run(32, 256), reference);
}

#[test]
fn another_model_gives_another_result() {
    let straight = straight();
    let rotated = model(|x, y| Vec2::new(0.6 + 0.3 * y, 0.7 - 0.3 * x));
    let (mut doc, layer) = painted(&straight, 16);
    blur(&mut doc, layer, 4);
    with_model(&mut doc, &straight);
    let first = whole(&doc);
    let since = doc.change_serial();
    with_model(&mut doc, &rotated);
    assert!(!doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
    let second = whole(&doc);
    assert_ne!(first, second);
    // 回した B は塗った所が違う（B の青は右のアイランドの矩形）ので、縁の向こうはアイランドの外の透明も読む
    assert_eq!(at(&second, 30, 16), [0, 0, 0, 0]);
    // 同じ位相を渡し直しても何も変わらない
    let since = doc.change_serial();
    with_model(&mut doc, &rotated);
    assert!(doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
}

#[test]
fn a_mask_blur_crosses_the_seam_too() {
    let topology = straight();
    let mut doc = Document::with_tile_size(SIZE, SIZE, 16).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            doc.set_pixel(layer, x, y, RED).unwrap();
        }
    }
    doc.add_layer_mask(layer).unwrap();
    let map = topology.island_map(SIZE, SIZE).unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if map.island(x, y) == 2 {
                doc.set_mask_pixel(layer, x, y, 255).unwrap(); // B を隠す
            }
        }
    }
    doc.add_filter(
        layer,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(4)),
    )
    .unwrap();
    let flat = whole(&doc);
    with_model(&mut doc, &topology);
    let across = whole(&doc);
    // A の右の縁の内側は、縁の向こうの B の隠す量を読んで薄くなる
    assert_eq!(at(&flat, 25, 16)[3], 255);
    assert!(at(&across, 25, 16)[3] < 255, "{:?}", at(&across, 25, 16));
}

#[test]
fn tangent_normals_turn_with_a_mirrored_seam() {
    // B は左右を返したアイランド。B の法線は B の接空間で +X に傾く → A の側から見ると −X
    let topology = model(|x, y| Vec2::new(0.9 - 0.3 * x, 0.1 + 0.3 * y));
    let map = topology.island_map(SIZE, SIZE).unwrap();
    let band = topology.seam_band(SIZE, SIZE, 8).unwrap();
    let mut image = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = match map.island(x, y) {
                1 => [128, 128, 255, 255],
                2 => [204, 128, 230, 255], // (0.6, 0, 0.8) くらい
                _ => [128, 128, 255, 0],
            };
            image[((y * SIZE + x) * 4) as usize..][..4].copy_from_slice(&p);
        }
    }
    let source = filter::Image::new(&image, SIZE, SIZE).unwrap();
    let stages = [Stage::new(Settings::GaussianBlur { radius: 3 })];
    let region = Rect::new(0, 0, SIZE, SIZE);
    let run = |seams| {
        let options = Options {
            seams,
            ..Options::default()
        };
        filter::evaluate(&source, ValueType::TangentNormal, &stages, region, &options).unwrap()
    };
    let across = run(Some(&band));
    let flat = run(None);
    let x = |p: &[u8]| at(p, 25, 16)[0];
    assert!(x(&across) < 120, "−X に傾く: {:?}", at(&across, 25, 16));
    assert!(
        x(&flat) >= 126,
        "2D は外の平らな法線だけ: {:?}",
        at(&flat, 25, 16)
    );
}

#[test]
fn the_seam_bytes_are_pinned_on_every_simd_path() {
    // `YOLU_SIMD=scalar|sse41|avx2|neon`（その CPU にある道）で同じ値になる（近傍の段は SIMD の道、帯を埋めるのは整数の 1 本の道）
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 5);
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::sharpen(3, 0.8, 2)).channels(&[Channel::Color]),
    )
    .unwrap();
    with_model(&mut doc, &topology);
    let bytes = whole(&doc);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    assert_eq!(h, PINNED, "{h:#018x}");
}

const PINNED: u64 = 0x7847_9b0c_9534_1d35;

/// 読んだ回数が `after` に達したら取消の印を立てる読み元（評価の途中で取り消す）。
struct Tripwire<'a> {
    inner: filter::Image<'a>,
    flag: &'a AtomicBool,
    reads: AtomicUsize,
    after: usize,
}

impl Source for Tripwire<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.inner.dimensions()
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if self.reads.fetch_add(1, Ordering::Relaxed) + 1 >= self.after {
            self.flag.store(true, Ordering::Relaxed);
        }
        self.inner.pixel(x, y)
    }
}

fn striped() -> Vec<u8> {
    let mut image = vec![0u8; (SIZE * SIZE * 4) as usize];
    for (i, p) in image.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let v = (i as u32 % 251) as u8;
        *p = [v, 255 - v, v / 2, 255];
    }
    image
}

#[test]
fn crossing_the_seam_is_refused_by_the_budget_when_2d_alone_fits() {
    let topology = straight();
    let band = topology.seam_band(SIZE, SIZE, 8).unwrap();
    let image = striped();
    let source = filter::Image::new(&image, SIZE, SIZE).unwrap();
    let stages = [Stage::new(Settings::GaussianBlur { radius: 3 })];
    let region = Rect::new(0, 0, SIZE, SIZE);
    let block = 32;
    let output = u64::from(SIZE) * u64::from(SIZE) * 4;
    let base = filter::block_working_bytes(&stages, block, SIZE, SIZE).unwrap();
    let seam = filter::seam_working_bytes(&stages, block, SIZE, SIZE, band.texel_count() as u64);
    assert!(
        seam > base,
        "帯の分は 2D の見積りに足される: {seam} / {base}"
    );
    let run = |budget, seams| {
        let options = Options {
            working_budget: budget,
            block_size: block,
            seams,
            ..Options::default()
        };
        filter::evaluate(&source, ValueType::Color, &stages, region, &options)
    };
    // 2D だけなら収まる予算で、またぐと断る（作業メモリの見積りに帯の分が入っている。返す画像の予算はそれとは別に足される）
    let tight = base + output;
    let flat = run(tight, None).unwrap();
    assert_eq!(
        run(tight, Some(&band)).unwrap_err(),
        filter::Error::Budget {
            needed: base + seam + output,
            budget: tight
        }
    );
    // 見積りどおりの予算なら通り、またがない評価とは値が違う。1 バイト足りなければ断る
    let exact = base + seam + output;
    assert_ne!(run(exact, Some(&band)).unwrap(), flat);
    assert!(matches!(
        run(exact - 1, Some(&band)),
        Err(filter::Error::Budget { .. })
    ));
    // 全域の統計（Normalize）を求める道も、同じ見積りで断る
    let stages = [Stage::new(Settings::Normalize), stages[0].clone()];
    let base = filter::block_working_bytes(&stages, block, SIZE, SIZE).unwrap();
    let seam = filter::seam_working_bytes(&stages, block, SIZE, SIZE, band.texel_count() as u64);
    let stats = |budget, seams| {
        let options = Options {
            working_budget: budget,
            block_size: block,
            seams,
            ..Options::default()
        };
        filter::statistics(&source, ValueType::Color, &stages, &options)
    };
    assert!(stats(base, None).is_ok());
    assert_eq!(
        stats(base, Some(&band)).unwrap_err(),
        filter::Error::Budget {
            needed: base + seam,
            budget: base
        }
    );
    assert!(stats(base + seam, Some(&band)).is_ok());
}

#[test]
fn a_cancel_during_a_seam_crossing_evaluation_stops_it() {
    let topology = straight();
    let band = topology.seam_band(SIZE, SIZE, 8).unwrap();
    let image = striped();
    let stages = [
        Stage::new(Settings::GaussianBlur { radius: 3 }),
        Stage::new(Settings::GaussianBlur { radius: 3 }),
    ];
    let region = Rect::new(0, 0, SIZE, SIZE);
    let run = |after: usize| {
        let flag = AtomicBool::new(false);
        let source = Tripwire {
            inner: filter::Image::new(&image, SIZE, SIZE).unwrap(),
            flag: &flag,
            reads: AtomicUsize::new(0),
            after,
        };
        let options = Options {
            block_size: 32,
            cancel: Some(&flag),
            seams: Some(&band),
            ..Options::default()
        };
        filter::evaluate(&source, ValueType::Color, &stages, region, &options)
    };
    // 取消の印が立たなければ最後まで評価し、評価の途中で立てば取り消される（返す画像は無い）
    assert!(run(usize::MAX).is_ok());
    for after in [1, 500, 5000] {
        match run(after) {
            Err(e) => assert_eq!(e, filter::Error::Cancelled, "読み {after} 回で立てた"),
            Ok(_) => panic!("読み {after} 回で立てたが、取り消されなかった"),
        }
    }
    // 始める前から立っていれば、何も読まずに取り消す
    let flag = AtomicBool::new(true);
    let source = Tripwire {
        inner: filter::Image::new(&image, SIZE, SIZE).unwrap(),
        flag: &flag,
        reads: AtomicUsize::new(0),
        after: usize::MAX,
    };
    let options = Options {
        cancel: Some(&flag),
        seams: Some(&band),
        ..Options::default()
    };
    assert_eq!(
        filter::evaluate(&source, ValueType::Color, &stages, region, &options).unwrap_err(),
        filter::Error::Cancelled
    );
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn a_table_that_does_not_fit_the_budget_is_evaluated_in_2d_and_says_so() {
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 4);
    let flat = whole(&doc);
    assert_eq!(doc.seam_cache_budget_bytes(), DEFAULT_BUDGET);
    doc.set_seam_cache_budget_bytes(100);
    assert_eq!(
        doc.seam_fallback(),
        None,
        "モデルが無ければまたがない（2D が普通）"
    );
    with_model(&mut doc, &topology);
    assert!(doc.seams_active());
    // 帯の写しが予算に収まらなければ、またがずに 2D のまま評価し、そのことが分かる（モデルを渡したとき、またぐ所に印を付けるために
    // 作ろうとして、もう断られている）
    assert_eq!(whole(&doc), flat);
    assert_eq!(
        doc.seam_fallback(),
        Some(SeamFallback::Tables(UvTopologyError::Budget {
            budget: 100
        }))
    );
    // 予算を戻すと、またいで評価し直し（前の予算の評価を使わない）、断りの印は消える
    doc.set_seam_cache_budget_bytes(DEFAULT_BUDGET);
    let across = whole(&doc);
    assert_ne!(across, flat);
    assert_eq!(doc.seam_fallback(), None);
    // 予算を小さくすると、収まらない覚えを捨てて、作り直した文書と同じ 2D に戻る
    doc.set_seam_cache_budget_bytes(100);
    assert_eq!(whole(&doc), flat);
    assert!(doc.seam_fallback().is_some());
    // またがない設定・近傍の段が無いレイヤーだけなら、断った印は出ない
    doc.set_filter_seams(false).unwrap();
    assert_eq!(doc.seam_fallback(), None);
}

#[test]
fn the_seam_budget_starts_at_the_default_and_can_be_set() {
    let mut doc = Document::with_tile_size(8, 8, 8).unwrap();
    assert_eq!(doc.seam_cache_budget_bytes(), DEFAULT_BUDGET);
    doc.set_seam_cache_budget_bytes(12345);
    assert_eq!(doc.seam_cache_budget_bytes(), 12345);
    // 同じ値にしても何も変わらない（Undo の段も作らない設定）
    let count = doc.undo_count();
    doc.set_seam_cache_budget_bytes(12345);
    assert_eq!(doc.undo_count(), count);
}

#[test]
fn a_filter_is_refused_when_only_the_seam_estimate_exceeds_the_working_budget() {
    let topology = straight();
    let stages = [Stage::new(Settings::GaussianBlur { radius: 3 })];
    let block = 32;
    let base = filter::block_working_bytes(&stages, block, SIZE, SIZE).unwrap();
    let seam = filter::seam_working_bytes(&stages, block, SIZE, SIZE, u64::MAX);
    let output = u64::from(block) * u64::from(block) * 4;
    let try_add = |model: bool, budget: u64| {
        let mut doc = Document::with_tile_size(SIZE, SIZE, 16).unwrap();
        let layer = doc.add_layer("塗り").unwrap();
        doc.set_filter_block_pixels(block).unwrap();
        doc.set_filter_working_budget_bytes(budget).unwrap();
        if model {
            with_model(&mut doc, &topology);
        }
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
    };
    // モデルが無ければ（2D だけ）今までの見積りで足りる
    assert!(try_add(false, base + output).is_ok());
    // モデルがあって継ぎ目をまたぐなら、帯の分も数える: 2D だけで足りる予算では断る
    assert!(matches!(
        try_add(true, base + output),
        Err(yolu_core::CoreError::WorkingBudgetExceeded)
    ));
    assert!(matches!(
        try_add(true, base + seam + output - 1),
        Err(yolu_core::CoreError::WorkingBudgetExceeded)
    ));
    assert!(try_add(true, base + seam + output).is_ok());
}

#[test]
fn a_filter_that_only_fits_in_2d_is_evaluated_in_2d_when_the_model_comes_later() {
    // 先に段を足し（2D の見積りだけで足りる予算）、あとからモデルを渡す: 帯の写しを使うと予算を超えるので、またがずに 2D で評価し、
    // 評価の失敗にはならない
    let topology = straight();
    let stages = [Stage::new(Settings::GaussianBlur { radius: 4 })];
    let block = 32;
    let base = filter::block_working_bytes(&stages, block, SIZE, SIZE).unwrap();
    let output = u64::from(block) * u64::from(block) * 4;
    let (mut doc, layer) = painted(&topology, 16);
    doc.set_filter_block_pixels(block).unwrap();
    doc.set_filter_working_budget_bytes(base + output).unwrap();
    blur(&mut doc, layer, 4);
    let flat = whole(&doc);
    with_model(&mut doc, &topology);
    assert!(doc.seams_active());
    assert_eq!(
        whole(&doc),
        flat,
        "予算を超えるので 2D のまま（エラーにならない）"
    );
    let Some(SeamFallback::Working { needed, budget }) = doc.seam_fallback() else {
        panic!("{:?}", doc.seam_fallback());
    };
    assert_eq!(budget, base + output);
    assert!(needed > budget);
    // 予算を上げると（設定できる下限は、帯の写しの大きさが分からない最悪の見積り）、またいで評価し直し（前の予算の評価を使わず、描き直す印も
    // 付く）、理由は消える。上げても、実際の見積りが予算に収まる限りの話
    let worst = base + filter::seam_working_bytes(&stages, block, SIZE, SIZE, u64::MAX) + output;
    assert!(needed <= worst);
    let since = doc.change_serial();
    doc.set_filter_working_budget_bytes(worst).unwrap();
    assert!(!doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
    let across = whole(&doc);
    assert_ne!(across, flat);
    assert_eq!(doc.seam_fallback(), None);
    // 戻すことはできない（今のスタックが要る量より小さい予算にはできない）
    assert!(doc.set_filter_working_budget_bytes(base + output).is_err());
}

// ───────── 入力の差し替えと位相の鍵 ─────────

/// 位置のマップ（seed を変えると別の中身）。
fn position_map(seed: u32) -> MapInput {
    let n = (SIZE * SIZE) as usize;
    let data: Vec<u16> = (0..n * 3)
        .map(|i| ((i as u32).wrapping_mul(977).wrapping_add(seed * 7919) % 65536) as u16)
        .collect();
    MapInput::new(
        MapKind::Position,
        SIZE,
        SIZE,
        data,
        vec![1; n],
        [-1.0; 3],
        [1.0; 3],
        &"ab".repeat(32),
        MapState::Current,
    )
    .unwrap()
}

/// モデルの UV の位相と、位置のマップ（seed）を渡す入力。
fn inputs_with(topology: &Arc<UvTopology>, seed: u32) -> EffectInputs {
    EffectInputs::new()
        .with_map(position_map(seed))
        .unwrap()
        .with_topology(Some(topology.clone()))
}

/// 位置のマップを読むレイヤー（全面を塗り、位置のグラデーションを色に足す）。
fn map_reader(doc: &mut Document) -> LayerId {
    let layer = doc.add_layer("マップを読む").unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            doc.set_pixel(layer, x, y, Rgba8::new(128, 128, 128, 255))
                .unwrap();
        }
    }
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(generator::Settings::new(
            generator::Kind::PositionGradient,
        )))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    layer
}

fn evaluated(doc: &Document) -> u64 {
    doc.effect_counters().blocks_evaluated
}

/// 評価のキャッシュを捨てて作り直した合成と同じか（古い出力を返していないか）。
fn assert_cache_is_honest(doc: &Document, what: &str) {
    let now = whole(doc);
    doc.release_effect_cache();
    assert!(
        whole(doc) == now,
        "{what}: キャッシュの出力が作り直した出力と違う"
    );
}

/// ぼかしのレイヤー（継ぎ目をまたぐ）と、マップを読むレイヤーの 2 レイヤーの文書（どちらも 1 ブロック）。
fn blur_and_reader(topology: &Arc<UvTopology>) -> Document {
    let (mut doc, layer) = painted(topology, 16);
    blur(&mut doc, layer, 4);
    map_reader(&mut doc);
    doc.set_effect_inputs(inputs_with(topology, 1)).unwrap();
    doc
}

#[test]
fn replacing_the_maps_redraws_the_map_reader_and_not_the_seam_crossing_blur() {
    let topology = straight();
    let mut doc = blur_and_reader(&topology);
    assert!(doc.seams_active());
    let first = whole(&doc);
    let settled = evaluated(&doc);
    assert_eq!(settled, 2, "2 レイヤーが 1 ブロックずつ");
    assert_eq!(whole(&doc), first);
    assert_eq!(evaluated(&doc), settled, "入力が同じなら評価し直さない");
    // マップだけ差し替える（位相は同じ物のまま）: 読むレイヤーの 1 ブロックだけ。ぼかしは位相にだけ依るので前の評価を使う
    doc.set_effect_inputs(inputs_with(&topology, 2)).unwrap();
    let second = whole(&doc);
    assert_ne!(second, first, "マップを読むレイヤーは変わる");
    assert_eq!(evaluated(&doc) - settled, 1);
    assert_cache_is_honest(&doc, "マップの差し替え");
    // 何度差し替えても同じ
    let settled = evaluated(&doc);
    doc.set_effect_inputs(inputs_with(&topology, 3)).unwrap();
    whole(&doc);
    assert_eq!(evaluated(&doc) - settled, 1);
    // 位相も要らなくなる（モデルを外す）と、またがない評価になるので、ぼかしのレイヤーも評価し直す
    let settled = evaluated(&doc);
    doc.set_effect_inputs(EffectInputs::new().with_map(position_map(3)).unwrap())
        .unwrap();
    assert!(!doc.seams_active());
    whole(&doc);
    assert_eq!(
        evaluated(&doc) - settled,
        1,
        "ぼかしのレイヤーだけ（マップは同じ）"
    );
    assert_cache_is_honest(&doc, "モデルを外す");
}

#[test]
fn the_same_layout_as_another_object_keeps_the_topology_and_evaluates_nothing_again() {
    let topology = straight();
    let mut doc = blur_and_reader(&topology);
    let first = whole(&doc);
    let settled = evaluated(&doc);
    // 別に作った同じ UV・隣り合わせの位相: 今の物を使い続け、何も変わらない
    let twin = straight();
    assert!(!Arc::ptr_eq(&topology, &twin));
    let since = doc.change_serial();
    doc.set_effect_inputs(inputs_with(&twin, 1)).unwrap();
    assert!(Arc::ptr_eq(
        doc.effect_inputs().topology().unwrap(),
        &topology
    ));
    assert_eq!(doc.change_serial(), since);
    assert_eq!(whole(&doc), first);
    assert_eq!(evaluated(&doc), settled);
    // ポーズで位置だけが変わったモデル（UV は同じ）も同じ位相
    let posed = {
        let mut t = quad(0., 0.5, |x, y| Vec2::new(0.1 + 0.3 * x, 0.1 + 0.3 * y)).to_vec();
        t.extend(quad(1., 0.5, |x, y| {
            Vec2::new(0.6 + 0.3 * x, 0.1 + 0.3 * y)
        }));
        let g = SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap();
        Arc::new(UvTopology::new(Arc::new(g), Some(0)))
    };
    doc.set_effect_inputs(inputs_with(&posed, 1)).unwrap();
    assert!(Arc::ptr_eq(
        doc.effect_inputs().topology().unwrap(),
        &topology
    ));
    assert_eq!(whole(&doc), first);
    assert_eq!(evaluated(&doc), settled);
    // マップも一緒に替わっても、ぼかしのレイヤーは評価し直さない（読むレイヤーの 1 ブロックだけ）
    doc.set_effect_inputs(inputs_with(&twin, 2)).unwrap();
    whole(&doc);
    assert!(Arc::ptr_eq(
        doc.effect_inputs().topology().unwrap(),
        &topology
    ));
    assert_eq!(evaluated(&doc) - settled, 1);
    assert_cache_is_honest(&doc, "同じ位相とマップの差し替え");
}

#[test]
fn another_uv_layout_redraws_the_seam_crossing_blur_and_not_the_map_reader() {
    let topology = straight();
    let mut doc = blur_and_reader(&topology);
    whole(&doc);
    let settled = evaluated(&doc);
    // 別の UV（B を回した）: 縁の向こうの対応が変わるので、ぼかしのレイヤーは評価し直す。マップは同じなので読むレイヤーは前のまま
    let rotated = model(|x, y| Vec2::new(0.6 + 0.3 * y, 0.7 - 0.3 * x));
    let since = doc.change_serial();
    doc.set_effect_inputs(inputs_with(&rotated, 1)).unwrap();
    assert!(Arc::ptr_eq(
        doc.effect_inputs().topology().unwrap(),
        &rotated
    ));
    assert!(!doc.changed_tiles(Channel::Color, since).unwrap().is_empty());
    whole(&doc);
    assert_eq!(
        evaluated(&doc) - settled,
        1,
        "ぼかしのレイヤーの 1 ブロックだけ"
    );
    assert_cache_is_honest(&doc, "別の UV");
    // ぼかしだけの文書では、出力も変わる
    let (mut alone, layer) = painted(&topology, 16);
    blur(&mut alone, layer, 4);
    with_model(&mut alone, &topology);
    let before = whole(&alone);
    with_model(&mut alone, &rotated);
    assert_ne!(whole(&alone), before);
    // 同じ UV でスロットのマテリアルの組だけが違う位相も別の物
    let other_material = Arc::new(UvTopology::new(rotated.geometry().clone(), Some(1)));
    let settled = evaluated(&doc);
    doc.set_effect_inputs(inputs_with(&other_material, 1))
        .unwrap();
    assert!(Arc::ptr_eq(
        doc.effect_inputs().topology().unwrap(),
        &other_material
    ));
    whole(&doc);
    assert_eq!(evaluated(&doc) - settled, 1);
}

#[test]
fn another_resolution_redraws_the_seam_crossing_blur() {
    let topology = straight();
    let (mut doc, layer) = painted(&topology, 16);
    blur(&mut doc, layer, 4);
    with_model(&mut doc, &topology);
    let first = whole(&doc);
    let settled = evaluated(&doc);
    // キャンバスを広げる（画素は同じ位置）。UV の同じ点は別の画素に当たるので、帯の写しもアイランドの図も大きさごとに作り直す
    doc.resize_canvas(96, 80, (0, 0)).unwrap();
    assert!(doc.seams_active());
    let grown = doc
        .composite_channel(Channel::Color, Rect::new(0, 0, 96, 80))
        .unwrap();
    assert!(evaluated(&doc) > settled, "大きさが変わったら評価し直す");
    doc.release_effect_cache();
    assert!(
        doc.composite_channel(Channel::Color, Rect::new(0, 0, 96, 80))
            .unwrap()
            == grown,
        "キャッシュの出力が作り直した出力と違う"
    );
    // 戻すと、前の大きさの評価と同じ
    assert!(doc.undo().unwrap());
    assert_eq!(whole(&doc), first);
}

// ── 固まらない（ドラッグ中の粗い評価・文書を開いた直後の評価・専用のプールの中）─────────────────────────────────────────
//
// 帯の写し・アイランドの図・位相の対応は、初めて要るときに rayon の並列で作る。作る間は門を持つので、作っている最中のスレッドが、待つ間に別のブロックの
// 評価を拾い、同じ門を待つと止まる。モデルを渡した直後（何も作っていない）に、ブロックが何個にもなる評価を始める形で、道ごとに確かめる。
// プールのスレッドが多いほど出やすいので、グローバルのプール（スレッド数は環境による）のほかに、スレッドが多い専用のプールの中でも回す。

/// 固まりの試験の文書の辺。タイル 16² のブロック 32²（256 個）に分ける（ブロックがスレッドより十分多いほど、固まりやすい）。
const BIG: u32 = 512;
const TILE: u32 = 16;

fn tint(x: u32, y: u32) -> [u8; 4] {
    let h = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
    [(h >> 8) as u8, (h >> 16) as u8, (h >> 24) as u8, 255]
}

/// タイル `coord` の画素（TileSize² × 4、下の行から）。`pixel(x, y)` は文書の座標。
fn tile_bytes(coord: TileCoord, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((TILE * TILE * 4) as usize);
    for y in 0..TILE {
        for x in 0..TILE {
            bytes.extend(pixel(coord.x * TILE + x, coord.y * TILE + y));
        }
    }
    bytes
}

/// 継ぎ目の縁が多いモデル（`atlas`）を渡した直後の文書（帯の写しもアイランドの図もまだ無い）と、全部のタイル。画素のあるレイヤー 1 枚（タイルを
/// 読み込んで置く。文書を開いたときと同じ）に、`target` へ `stage` を重ねる。マスクなら、マスクにも（間引いた）画素を置く。
fn cold_document(target: FilterTarget, stage: EffectSettings) -> (Document, Vec<TileCoord>) {
    let mut doc = Document::with_tile_size(BIG, BIG, TILE).unwrap();
    doc.set_filter_block_pixels(32).unwrap();
    let layer = doc.add_layer("塗り").unwrap();
    let tiles: Vec<TileCoord> = (0..BIG / TILE)
        .flat_map(|y| (0..BIG / TILE).map(move |x| TileCoord::new(x, y)))
        .collect();
    for &coord in &tiles {
        doc.import_tile(layer, Channel::Color, coord, &tile_bytes(coord, tint))
            .unwrap();
    }
    let spec = match target {
        FilterTarget::Mask => {
            doc.add_layer_mask(layer).unwrap();
            for &coord in &tiles {
                let hide = tile_bytes(coord, |x, y| {
                    [0, 0, 0, if x % 3 == 0 && y % 3 == 0 { 255 } else { 0 }]
                });
                doc.import_mask_tile(layer, coord, &hide).unwrap();
            }
            FilterSpec::new(stage)
        }
        _ => FilterSpec::new(stage).channels(&[Channel::Color]),
    };
    doc.add_filter(layer, target, spec).unwrap();
    with_model(&mut doc, &atlas(12));
    assert!(doc.seams_active());
    (doc, tiles)
}

/// (名前, 重ねる所, 段): ガウスぼかしと方向ぼかしを、レイヤーの内容とマスクの両方で。
fn seam_stages() -> Vec<(&'static str, FilterTarget, EffectSettings)> {
    let directional = EffectSettings::Filter(Settings::DirectionalBlur {
        angle: 0.7,
        distance: 12.0,
    });
    vec![
        (
            "内容のガウスぼかし",
            FilterTarget::Content,
            EffectSettings::blur(8),
        ),
        (
            "内容の方向ぼかし",
            FilterTarget::Content,
            directional.clone(),
        ),
        (
            "マスクのガウスぼかし",
            FilterTarget::Mask,
            EffectSettings::blur(8),
        ),
        ("マスクの方向ぼかし", FilterTarget::Mask, directional),
    ]
}

/// 評価の入り口（アプリが呼ぶ形）。
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// ドラッグ中の粗い合成（歩幅 2）。
    Coarse,
    /// 正確な合成（開いた直後に全タイルを描くのと同じ）。
    Exact,
}

fn pixels_of(doc: &Document, tiles: &[TileCoord], entry: Entry) -> Vec<Vec<u8>> {
    let composed = match entry {
        Entry::Coarse => doc.composite_coarse_tiles(Channel::Color, tiles, 2),
        Entry::Exact => doc.composite_tiles(Channel::Color, tiles),
    }
    .unwrap();
    assert_eq!(composed.len(), tiles.len());
    composed.into_iter().map(|t| t.pixels).collect()
}

fn check(entry: Entry) {
    for (name, target, stage) in seam_stages() {
        assert_finishes_alike(
            &format!("{name}（{entry:?}）"),
            move || cold_document(target, stage.clone()),
            move |(doc, tiles)| pixels_of(doc, tiles, entry),
        );
    }
}

/// ぼかしのスライダーをドラッグしている間の粗い評価（歩幅 2）。縮めた大きさの帯の写しを、評価を並べる前に作る。
#[test]
fn the_coarse_evaluation_of_a_blur_that_crosses_seams_does_not_hang() {
    check(Entry::Coarse);
}

/// 文書を開いた直後の評価（全部のタイルの正確な合成）。マスクのぼかしも、レイヤーの内容のぼかしと同じ。
#[test]
fn the_first_evaluation_of_a_document_with_a_blur_that_crosses_seams_does_not_hang() {
    check(Entry::Exact);
}

/// Anchor が読むレイヤー（ぼかしのレイヤー）の出力は、Anchor を読む側のブロックの評価の最中に（そのレイヤーのブロックを評価して）初めて要る。
/// 土台のレイヤー（画素とぼかし）の Anchor を、その上の塗りつぶしレイヤーの Anchor のジェネレーター（置き換え）が読む文書。
fn cold_anchor_document() -> (Document, Vec<TileCoord>) {
    let mut doc = Document::with_tile_size(BIG, BIG, TILE).unwrap();
    doc.set_filter_block_pixels(32).unwrap();
    let base = doc.add_layer("土台").unwrap();
    let tiles: Vec<TileCoord> = (0..BIG / TILE)
        .flat_map(|y| (0..BIG / TILE).map(move |x| TileCoord::new(x, y)))
        .collect();
    for &coord in &tiles {
        doc.import_tile(base, Channel::Color, coord, &tile_bytes(coord, tint))
            .unwrap();
    }
    doc.add_filter(
        base,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(8)).channels(&[Channel::Color]),
    )
    .unwrap();
    let anchor = doc
        .add_anchor(base, yolu_core::AnchorPlacement::Layer, Some("土台"), None)
        .unwrap();
    let reader = doc
        .add_fill_layer("読む側", &[(Channel::Color, BLUE)], None)
        .unwrap();
    let mut g = generator::Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let stage = doc
        .add_filter(
            reader,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
    doc.set_generator_anchor(
        reader,
        stage,
        Some(anchor),
        Channel::Color,
        generator::anchor::ReadMode::Value,
        false,
    )
    .unwrap();
    with_model(&mut doc, &atlas(12));
    assert!(doc.seams_active());
    (doc, tiles)
}

/// Anchor が読むぼかしのレイヤー（継ぎ目をまたぐ）の出力を、Anchor を読む側の評価の最中に初めて作っても固まらない。
#[test]
fn an_anchor_reading_a_blur_that_crosses_seams_does_not_hang() {
    // 読む側は、塗りつぶしの色ではなく、Anchor の（ぼかした）土台の値を出す
    let (doc, tiles) = cold_anchor_document();
    let read = pixels_of(&doc, &tiles, Entry::Exact);
    assert!(read.iter().any(|t| t
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[..3] != BLUE.to_array()[..3])));
    for round in 0..4 {
        for entry in [Entry::Coarse, Entry::Exact] {
            assert_finishes_alike(
                &format!("Anchor が読むぼかし（{entry:?}・{round}）"),
                cold_anchor_document,
                move |(doc, tiles)| pixels_of(doc, tiles, entry),
            );
        }
    }
}
