//! 3D の面のストロークの連続吹き付けとリボン: 同じ点で時刻だけ進む入力でダブが時刻の間隔ごとに増え、時刻が進まない入力では増えない。
//! 押し続けた所は濃くなる。リボンは、まっすぐな線が筆先の帯の模様を線の長さのとおりに続けて写す（帯の継ぎ目に抜けが無い）。
//! カメラに正面を向けた平らな四角（UV が文書の全体、文書の 1 画素が画面の 1 点）に描く。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;

use yolu_core::geometry::{
    CameraView, OrbitCamera, SurfaceGeometry, SurfaceInput, SurfaceStroke, SurfaceStrokeError,
    SurfaceStrokeOptions, SurfaceTriangle, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{
    Brush, BrushSettings, BrushTip, Channel, CoreError, Document, LayerId, Rgba8, TipSelection,
    CONTINUOUS_INTERVAL,
};

const W: u32 = 256;
const H: u32 = 96;
const MARGIN: f32 = 16.0;

fn quad() -> Arc<SurfaceGeometry> {
    let (w, h) = (W as f32, H as f32);
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(x / w, y / h);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
        .with_slot(0, 0, 0)
    };
    Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((0.0, 0.0), (0.0, h), (w, 0.0)),
                tri((w, 0.0), (0.0, h), (w, h)),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

fn front() -> CameraView {
    let (sw, sh) = (W as f32 + 2.0 * MARGIN, H as f32 + 2.0 * MARGIN);
    let distance = sh / (2.0 * (15.0f32).to_radians().tan());
    OrbitCamera {
        target: Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: W as f32,
        ..Default::default()
    }
    .view(sw, sh)
}

fn screen(x: f64, y: f64) -> Vec2 {
    Vec2::new(
        (x + MARGIN as f64) as f32,
        (H as f64 + MARGIN as f64 - y) as f32,
    )
}

fn document() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
}

/// 画面の半径を文書の半径にそろえたブラシ。
fn on_quad(g: &SurfaceGeometry, mut b: Brush) -> Brush {
    b.base.radius *= W as f64 / g.brush_scale() as f64;
    b
}

/// 文書の点 (x, y, 時刻) の列を 3D に描いて、塗ったダブの数と画素。
fn draw(brush: &Brush, points: &[(f64, f64, f64)]) -> (usize, Vec<u8>) {
    let g = quad();
    let b = on_quad(&g, brush.clone());
    let (mut d, l) = document();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let input = |(x, y, t): (f64, f64, f64)| SurfaceInput {
        time: t,
        ..SurfaceInput::new(screen(x, y), 1.0)
    };
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        input(points[0]),
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &p in &points[1..] {
        s.add_input(&mut d, &mut stroke, input(p)).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    let dabs = s.stats.dabs;
    d.end_stroke(stroke).unwrap();
    (dabs, bytes(&d, l))
}

fn alpha(px: &[u8], x: u32, y: u32) -> u8 {
    px[((y * W + x) * 4 + 3) as usize]
}

fn soft(continuous: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 6.0,
        hardness: 0.5,
        spacing: 0.25,
        flow: 0.08,
        color: Rgba8::new(20, 40, 160, 255),
        ..BrushSettings::default()
    });
    b.tip.continuous = continuous;
    b
}

fn held(n: usize) -> Vec<(f64, f64, f64)> {
    (0..=n)
        .map(|i| (60.0, 40.0, i as f64 * CONTINUOUS_INTERVAL + 1e-9 * i as f64))
        .collect()
}

#[test]
fn holding_the_pen_on_the_surface_adds_a_dab_per_interval() {
    for n in [0usize, 1, 7, 30] {
        let (on, _) = draw(&soft(true), &held(n));
        assert_eq!(on, 1 + n, "{n}");
        let (off, _) = draw(&soft(false), &held(n));
        assert_eq!(off, 1, "{n}");
    }
    // 時刻の進まない入力は増やさない
    let same: Vec<(f64, f64, f64)> = (0..10).map(|_| (60.0, 40.0, 0.25)).collect();
    assert_eq!(draw(&soft(true), &same).0, 1);
}

#[test]
fn holding_darkens_the_spot_and_repeats_the_same_bytes() {
    let (_, on) = draw(&soft(true), &held(40));
    let (_, off) = draw(&soft(false), &held(40));
    assert!(alpha(&on, 60, 40) > alpha(&off, 60, 40));
    assert_eq!(draw(&soft(true), &held(40)).1, on);
    // 後補正が入りなら切と同じ
    let mut post = soft(true);
    post.assist.post.on = true;
    let mut post_off = soft(false);
    post_off.assist.post.on = true;
    assert_eq!(draw(&post, &held(40)).1, draw(&post_off, &held(40)).1);
}

fn bands() -> Arc<BrushTip> {
    let alpha = (0..16)
        .flat_map(|_| (0..64u32).map(|x| [60u8, 120, 180, 240][(x / 16) as usize]))
        .collect();
    Arc::new(BrushTip::new("band", 64, 16, alpha).unwrap())
}

fn ribbon(on: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 8.0,
        hardness: 1.0,
        spacing: 0.25,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.tip.images = vec![bands()];
    b.tip.selection = TipSelection::Sequential;
    b.tip.ribbon = on;
    b
}

#[test]
fn a_straight_line_on_the_surface_copies_the_pattern_by_arc_length() {
    let line: Vec<(f64, f64, f64)> = (0..=28)
        .map(|i| (20.0 + 7.0 * i as f64, 48.0, 0.0))
        .collect();
    let (_, px) = draw(&ribbon(true), &line);
    // 帯の中は筆先の濃さ（テクセルの中心を画面へ写す丸めの分だけ許す）、継ぎ目にも抜けが無い
    for x in 22..216u32 {
        let along = x - 20;
        let got = alpha(&px, x, 48);
        assert!(got > 0, "x {x}: 抜けが無い");
        if (2..14).contains(&(along % 16)) {
            let want = [60u8, 120, 180, 240][((along % 64) / 16) as usize];
            assert!(got.abs_diff(want) <= 3, "x {x}: {got} ≠ {want}");
        }
    }
    // 切なら別の線（画像を 1 枚ずつ置く）
    assert_ne!(draw(&ribbon(false), &line).1, px);
}

/// 幅 1・高さ h の細長い筆先のリボン（半径 8・間隔 4 点。2D の `thin` と同じ）。
fn thin_ribbon(h: u32) -> Brush {
    let mut b = ribbon(true);
    b.tip.images = vec![Arc::new(
        BrushTip::new("thin", 1, h, vec![200; h as usize]).unwrap(),
    )];
    b
}

/// `draw` の、断られるかもしれない形: 点の列を 3D に描き、断られたら文書のストロークを取り消す。`budget` はストロークの予算（バイト）。
fn try_draw(
    brush: &Brush,
    points: &[(f64, f64, f64)],
    budget: Option<u64>,
) -> (Document, LayerId, Result<(), SurfaceStrokeError>) {
    let g = quad();
    let b = on_quad(&g, brush.clone());
    let (mut d, l) = document();
    if let Some(bytes) = budget {
        d.set_stroke_budget_bytes(bytes).unwrap();
    }
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let input = |(x, y, t): (f64, f64, f64)| SurfaceInput {
        time: t,
        ..SurfaceInput::new(screen(x, y), 1.0)
    };
    let result = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        input(points[0]),
        SurfaceStrokeOptions::default(),
    )
    .and_then(|mut s| {
        for &p in &points[1..] {
            s.add_input(&mut d, &mut stroke, input(p))?;
        }
        s.finish(&mut d, &mut stroke)
    });
    match &result {
        Ok(()) => {
            d.end_stroke(stroke).unwrap();
        }
        Err(_) => d.cancel_stroke(stroke),
    }
    (d, l, result)
}

fn line() -> Vec<(f64, f64, f64)> {
    (0..=28)
        .map(|i| (20.0 + 7.0 * i as f64, 48.0, 0.0))
        .collect()
}

#[test]
fn a_tall_tip_at_or_over_the_piece_limit_leaves_no_gap_on_the_surface() {
    // 描点 1 つが新しく写す帯は 0.25 × h 枚: 240 で 60 枚（上限 64 の内）、300 で 75 枚（残りを次の描点が続けて写す）
    for h in [240, 300] {
        let (d, l, result) = try_draw(&thin_ribbon(h), &line(), None);
        assert_eq!(result, Ok(()), "h {h}");
        let px = bytes(&d, l);
        for x in 22..170u32 {
            assert!(alpha(&px, x, 48) > 0, "h {h}: x {x} に抜けが無い");
        }
    }
}

#[test]
fn a_ribbon_that_cannot_keep_up_on_the_surface_is_refused_and_leaves_nothing() {
    let (d, l, result) = try_draw(&thin_ribbon(1024), &line(), None);
    assert_eq!(
        result,
        Err(SurfaceStrokeError::Core(CoreError::StrokeBudgetExceeded))
    );
    assert!(!d.has_active_stroke());
    assert_eq!(d.undo_count(), 0);
    let (blank, blank_layer) = document();
    assert_eq!(bytes(&d, l), bytes(&blank, blank_layer), "画素は元のまま");
}

#[test]
fn dabs_held_for_the_taper_on_the_surface_count_against_the_stroke_budget() {
    let mut tapered = soft(true);
    tapered.assist.taper_out = 24.0;
    // 予算の内: 止めて押している間に待たせたダブは、動かした後に塗れる
    let mut points = held(30);
    let t = 30.0 * CONTINUOUS_INTERVAL;
    points.extend((1..=10).map(|i| (60.0 + 6.0 * i as f64, 40.0, t + i as f64 * 0.01)));
    let (d, l, result) = try_draw(&tapered, &points, None);
    assert_eq!(result, Ok(()));
    assert!(!d.has_active_stroke());
    let (blank, blank_layer) = document();
    assert_ne!(bytes(&d, l), bytes(&blank, blank_layer));
    assert_eq!(d.undo_count(), 1);
    // 予算の外: 待たせるダブが増え続けるので、予算を超えたところでストロークごと取り消す
    let (d, l, result) = try_draw(&tapered, &held(200), Some(4000));
    assert_eq!(
        result,
        Err(SurfaceStrokeError::Core(CoreError::StrokeBudgetExceeded))
    );
    assert!(!d.has_active_stroke());
    assert_eq!(d.undo_count(), 0);
    let (blank, blank_layer) = document();
    assert_eq!(bytes(&d, l), bytes(&blank, blank_layer), "画素は元のまま");
}
