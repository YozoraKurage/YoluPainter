//! 影響元（`Brush::influences`）の 2D のストローク: 傾きの最小値と曲線・硬さへの傾き・真円率と質感の濃さの影響元が、同じ値を固定の設定に
//! 入れたブラシと同じ画素になること、ランダムの影響元は同じ種で同じ画素・ゆらぎの乱数の列を変えないこと。1 つのダブは「1 点だけの
//! ストローク」で置く（間隔・フェードに関わらない）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::brush::pen_tilt;
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushSample, BrushSettings, Document, InputResponse, InputUse, LayerId,
    PaperTexture, PressureSwitches, Rgba8,
};

const SIZE: u32 = 64;
const INK: Rgba8 = Rgba8::new(20, 30, 40, 255);

fn canvas() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(SIZE, SIZE, 16).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

/// 筆圧を使わない丸いブラシ（半径 10・硬さ 0.8）。
fn round() -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 10.0,
            hardness: 0.8,
            spacing: 0.1,
            color: INK,
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    )
}

fn used(min: f64) -> InputUse {
    InputUse {
        on: true,
        response: InputResponse::new(min, vec![]).unwrap(),
    }
}

/// 1 点だけのストローク（筆圧・傾き）の合成した画素。
fn dot(brush: &Brush, pressure: f64, tilt: DVec2) -> Vec<u8> {
    let (mut d, l) = canvas();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    s.add_sample(
        &mut d,
        BrushSample::new(32.0, 32.0, pressure, 0.0, tilt).unwrap(),
    )
    .unwrap();
    d.end_stroke(s).unwrap();
    d.composite(d.bounds()).unwrap()
}

/// 点の並び（筆圧が変わる）のストローク。
fn line(brush: &Brush) -> Vec<u8> {
    let (mut d, l) = canvas();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for i in 0..=24 {
        let t = i as f64 / 24.0;
        let sample = BrushSample::new(
            8.0 + 48.0 * t,
            32.0 + 6.0 * (t * 7.0).sin(),
            0.2 + 0.8 * t,
            i as f64 * 0.01,
            DVec2::ZERO,
        )
        .unwrap();
        s.add_sample(&mut d, sample).unwrap();
    }
    d.end_stroke(s).unwrap();
    d.composite(d.bounds()).unwrap()
}

fn painted(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
}

#[test]
fn a_tilt_minimum_and_curve_set_the_size_like_a_fixed_radius() {
    let tilt = DVec2::new(0.6, 0.2);
    let upright = 1.0 - pen_tilt::amount(tilt.x, tilt.y);
    let curve = InputResponse::new(
        0.3,
        vec![
            yolu_core::generator::CurvePoint { x: 0.0, y: 0.0 },
            yolu_core::generator::CurvePoint { x: 0.5, y: 0.8 },
            yolu_core::generator::CurvePoint { x: 1.0, y: 1.0 },
        ],
    )
    .unwrap();
    let mut b = round();
    b.influences.size.tilt = InputUse {
        on: true,
        response: curve.clone(),
    };
    let mut fixed = round();
    fixed.base.radius = 10.0 * curve.apply(upright);
    assert_eq!(dot(&b, 1.0, tilt), dot(&fixed, 1.0, DVec2::ZERO));
    // 切った入力の応えは使わない（今までの丸いブラシと同じ画素）
    b.influences.size.tilt.on = false;
    assert_eq!(dot(&b, 1.0, tilt), dot(&round(), 1.0, tilt));
}

#[test]
fn a_tilt_maximum_maps_the_tilt_between_the_minimum_and_the_maximum() {
    let (min, max) = (0.2, 0.7);
    let response = InputResponse::new(min, vec![])
        .unwrap()
        .with_max(max)
        .unwrap();
    assert_eq!((response.min(), response.max()), (min, max));
    let mut b = round();
    b.influences.size.tilt = InputUse {
        on: true,
        response: response.clone(),
    };
    // 傾きの量 t のとき 入力 x = 1 − t。係数は 最小値 + (最大値 − 最小値) × x（寝かせきった x=0 で最小値、直立の x=1 で最大値）
    for tilt in [
        DVec2::ZERO,
        DVec2::new(0.6, 0.2),
        DVec2::new(0.0, 1.4),
        DVec2::new(1.2, 1.2),
    ] {
        let upright = 1.0 - pen_tilt::amount(tilt.x, tilt.y);
        let mut fixed = round();
        fixed.base.radius = 10.0 * (min + (max - min) * upright);
        assert_eq!(
            dot(&b, 1.0, tilt),
            dot(&fixed, 1.0, DVec2::ZERO),
            "{tilt:?}"
        );
    }
    // 直立（傾きなし）は最大値の大きさ。最大値 1 の応えは、最大値を持つ前と同じ画素
    let mut fixed = round();
    fixed.base.radius = 10.0 * max;
    assert_eq!(dot(&b, 1.0, DVec2::ZERO), dot(&fixed, 1.0, DVec2::ZERO));
    let tilted = DVec2::new(0.6, 0.2);
    let mut full = round();
    full.influences.size.tilt = InputUse {
        on: true,
        response: InputResponse::new(min, vec![]).unwrap(),
    };
    let mut full_max = full.clone();
    full_max.influences.size.tilt.response = response.with_max(1.0).unwrap();
    assert_eq!(dot(&full, 1.0, tilted), dot(&full_max, 1.0, tilted));
    // 最大値は曲線のあとに効く: 係数 = 最小値 + (最大値 − 最小値) × 曲線(x)
    let curve = vec![
        yolu_core::generator::CurvePoint { x: 0.0, y: 0.0 },
        yolu_core::generator::CurvePoint { x: 0.5, y: 0.8 },
        yolu_core::generator::CurvePoint { x: 1.0, y: 1.0 },
    ];
    let shaped = InputResponse::new(min, curve)
        .unwrap()
        .with_max(max)
        .unwrap();
    let upright = 1.0 - pen_tilt::amount(tilted.x, tilted.y);
    let mut curved = round();
    curved.influences.size.tilt = InputUse {
        on: true,
        response: shaped.clone(),
    };
    let mut fixed = round();
    fixed.base.radius = 10.0 * shaped.apply(upright);
    assert_eq!(dot(&curved, 1.0, tilted), dot(&fixed, 1.0, DVec2::ZERO));
    // 範囲の外・有限でない最大値は作れない
    for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(response.with_max(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_hardness_follows_the_tilt_like_a_fixed_hardness() {
    let tilt = DVec2::new(0.0, 0.7);
    let upright = 1.0 - pen_tilt::amount(tilt.x, tilt.y);
    let mut b = round();
    b.base.radius = 20.0;
    b.influences.hardness.tilt = used(0.0);
    let mut fixed = round();
    fixed.base.radius = 20.0;
    fixed.base.hardness = 0.8 * upright;
    assert_eq!(dot(&b, 1.0, tilt), dot(&fixed, 1.0, DVec2::ZERO));
}

#[test]
fn the_roundness_influence_squashes_the_tip_before_the_jitter() {
    let mut b = round();
    b.tip.angle = 30.0;
    b.influences.roundness.pressure = used(0.0);
    for p in [0.3, 0.75] {
        let mut fixed = round();
        fixed.tip.angle = 30.0;
        fixed.tip.roundness = p;
        assert_eq!(
            dot(&b, p, DVec2::ZERO),
            dot(&fixed, 1.0, DVec2::ZERO),
            "{p}"
        );
    }
    // 下限は 0.01（筆圧 0 でも消えない）
    let mut thin = round();
    thin.tip.angle = 30.0;
    thin.tip.roundness = 0.01;
    assert_eq!(dot(&b, 0.0, DVec2::ZERO), dot(&thin, 1.0, DVec2::ZERO));
    assert!(painted(&dot(&b, 0.0, DVec2::ZERO)) > 0);
}

#[test]
fn the_texture_density_influence_scales_the_depth_of_that_dab() {
    let texture = |depth: f64| {
        let mut t = PaperTexture::new(builtin_tip("grain").unwrap(), depth);
        t.scale = 2.0;
        Some(t)
    };
    let mut b = round();
    b.texture = texture(0.8);
    b.influences.texture_density.pressure = used(0.0);
    let p = 0.5;
    let mut fixed = round();
    fixed.texture = texture(0.8 * p);
    assert_eq!(dot(&b, p, DVec2::ZERO), dot(&fixed, 1.0, DVec2::ZERO));
    assert_ne!(dot(&b, p, DVec2::ZERO), dot(&b, 1.0, DVec2::ZERO));
}

#[test]
fn a_random_influence_repeats_with_the_seed_and_leaves_the_jitter_sequence_alone() {
    let mut b = round();
    b.seed = 5;
    b.jitter.size = 0.4;
    b.jitter.angle = 0.5;
    b.tip.image = builtin_tip("charcoal");
    let plain = line(&b);
    // ランダムを使う項目があっても、効く所の無い項目（質感の無いブラシの質感の濃さ）なら、ゆらぎの乱数の列は同じで画素も同じ
    let mut idle = b.clone();
    idle.influences.texture_density.random = used(0.0);
    assert_eq!(line(&idle), plain);
    // 不透明度のランダム: 同じ種なら同じ画素、種が違えば違う画素、使わないブラシとも違う
    let mut r = b.clone();
    r.influences.opacity.random = used(0.2);
    let first = line(&r);
    assert_eq!(first, line(&r));
    assert_ne!(first, plain);
    let mut other = r.clone();
    other.seed = 6;
    let mut other_plain = b.clone();
    other_plain.seed = 6;
    assert_ne!(line(&other), first);
    assert_ne!(line(&other), line(&other_plain));
}
