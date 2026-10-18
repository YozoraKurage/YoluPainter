//! 3D の面のストロークの連続吹き付けとリボン: 同じ点で時刻だけ進む入力でダブが時刻の間隔ごとに増え、時刻が進まない入力では増えない。
//! 押し続けた所は濃くなる。リボンは、まっすぐな線が筆先の帯の模様を線の長さのとおりに続けて写す（帯の継ぎ目に抜けが無い）。
//! カメラに正面を向けた平らな四角（UV が文書の全体、文書の 1 画素が画面の 1 点）に描く。Tangent | Wrap は、斜めに見た四角でも
//! リボンの帯が途切れず、線の向きに沿わせる細長い筆先が面の上の線の向きに沿う。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;

use yolu_core::geometry::{
    CameraView, OrbitCamera, ProjectionSettings, SurfaceAlignment, SurfaceGeometry, SurfaceInput,
    SurfaceStroke, SurfaceStrokeError, SurfaceStrokeOptions, SurfaceTriangle,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{
    Brush, BrushSettings, BrushTip, Channel, CoreError, Document, DualBrush, DualBrushMode,
    LayerId, Rgba8, Spray, SprayDirection, TipSelection, CONTINUOUS_INTERVAL,
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
    draw_with(brush, points, ProjectionSettings::default())
}

/// `draw` の、3D の塗りの設定（アライメントなど）を選べる形。
fn draw_with(
    brush: &Brush,
    points: &[(f64, f64, f64)],
    projection: ProjectionSettings,
) -> (usize, Vec<u8>) {
    draw_seen(quad(), brush, points, projection, front(), screen)
}

/// 四角を、横に yaw 度回したカメラから見て描く（文書の点 (x, y) は四角の点 (x, y, 0)）。
fn draw_slanted(
    brush: &Brush,
    points: &[(f64, f64, f64)],
    projection: ProjectionSettings,
    yaw: f32,
) -> (usize, Vec<u8>) {
    draw_far(brush, points, projection, yaw, 1.0)
}

/// `draw_slanted` の、カメラの距離を far 倍にした形。
fn draw_far(
    brush: &Brush,
    points: &[(f64, f64, f64)],
    projection: ProjectionSettings,
    yaw: f32,
    far: f32,
) -> (usize, Vec<u8>) {
    let camera = slanted_camera();
    let camera = OrbitCamera {
        yaw,
        distance: camera.distance * far,
        ..camera
    };
    draw_on(quad(), brush, points, projection, camera, |x, y| {
        Vec3::new(x as f32, y as f32, 0.0)
    })
}

/// 面 g を camera から見て描く（文書の点 (x, y) は面の点 place(x, y)）。
fn draw_on(
    g: Arc<SurfaceGeometry>,
    brush: &Brush,
    points: &[(f64, f64, f64)],
    projection: ProjectionSettings,
    camera: OrbitCamera,
    place: impl Fn(f64, f64) -> Vec3,
) -> (usize, Vec<u8>) {
    let view = camera.view(W as f32 + 2.0 * MARGIN, H as f32 + 2.0 * MARGIN);
    let to = move |x: f64, y: f64| view.to_screen(place(x, y)).unwrap();
    draw_seen(g, brush, points, projection, view, to)
}

fn slanted_camera() -> OrbitCamera {
    let sh = H as f32 + 2.0 * MARGIN;
    OrbitCamera {
        target: Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance: sh / (2.0 * (15.0f32).to_radians().tan()),
        model_radius: W as f32,
        ..Default::default()
    }
}

/// 面 g を、カメラ view から、文書の点を画面の点へ写す to で描く。
fn draw_seen(
    g: Arc<SurfaceGeometry>,
    brush: &Brush,
    points: &[(f64, f64, f64)],
    projection: ProjectionSettings,
    view: CameraView,
    to: impl Fn(f64, f64) -> Vec2,
) -> (usize, Vec<u8>) {
    let b = on_quad(&g, brush.clone());
    let (mut d, l) = document();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let input = |(x, y, t): (f64, f64, f64)| SurfaceInput {
        time: t,
        ..SurfaceInput::new(to(x, y), 1.0)
    };
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        input(points[0]),
        SurfaceStrokeOptions {
            projection,
            ..SurfaceStrokeOptions::default()
        },
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

fn tangent_wrap() -> ProjectionSettings {
    ProjectionSettings {
        alignment: SurfaceAlignment::TangentWrap,
        ..ProjectionSettings::default()
    }
}

#[test]
fn tangent_wrap_reads_the_ribbon_band_by_the_same_cover_as_the_camera_alignment() {
    let line: Vec<(f64, f64, f64)> = (0..=28)
        .map(|i| (20.0 + 7.0 * i as f64, 48.0, 0.0))
        .collect();
    let (_, px) = draw_with(&ribbon(true), &line, tangent_wrap());
    // カメラの向きの塗りと同じ帯の模様が、線の長さのとおりに続く（抜けが無い）
    for x in 22..216u32 {
        let along = x - 20;
        let got = alpha(&px, x, 48);
        assert!(got > 0, "x {x}: 抜けが無い");
        if (2..14).contains(&(along % 16)) {
            let want = [60u8, 120, 180, 240][((along % 64) / 16) as usize];
            assert!(got.abs_diff(want) <= 3, "x {x}: {got} ≠ {want}");
        }
    }
    // リボンが切なら別の線
    assert_ne!(draw_with(&ribbon(false), &line, tangent_wrap()).1, px);
}

#[test]
fn tangent_wrap_keeps_the_ribbon_band_unbroken_on_a_slanted_quad() {
    // 60 度回して見た四角を横に引く: 画面の上の長さは面に沿った長さの半分ほど。帯は面に沿った長さで切るので、模様は正面と同じ
    // 文書の長さで続き、抜けが無い
    let line: Vec<(f64, f64, f64)> = (0..=56)
        .map(|i| (20.0 + 3.5 * i as f64, 48.0, 0.0))
        .collect();
    let (_, px) = draw_slanted(&ribbon(true), &line, tangent_wrap(), 60.0);
    for x in 22..212u32 {
        let along = x - 20;
        let got = alpha(&px, x, 48);
        assert!(got > 0, "x {x}: 抜けが無い");
        if (3..13).contains(&(along % 16)) {
            let want = [60u8, 120, 180, 240][((along % 64) / 16) as usize];
            assert!(got.abs_diff(want) <= 6, "x {x}: {got} ≠ {want}");
        }
    }
}

/// 横の線（文書の x 20〜216、y 48）の点の列。
fn across(step: f64) -> Vec<(f64, f64, f64)> {
    let n = (196.0 / step).round() as usize;
    (0..=n)
        .map(|i| (20.0 + step * i as f64, 48.0, 0.0))
        .collect()
}

/// 文書の行 y 48 の x 22〜211 で、塗られていない画素の x。
fn holes(px: &[u8]) -> Vec<u32> {
    (22..212u32).filter(|&x| alpha(px, x, 48) == 0).collect()
}

#[test]
fn tangent_wrap_keeps_the_ribbon_band_unbroken_on_a_slanted_quad_seen_from_far() {
    // 遠くから斜めに見ると、ダブの間隔は画面の上で 0.5 点に止まり、面に沿った描点の間隔が帯の間隔の見込みより広がる。帯の間隔は
    // 実際の描点の間隔より短くならないので、抜けが無い
    for (far, yaw) in [(7.0, 60.0), (7.0, 70.0), (10.0, 70.0), (4.0, 70.0)] {
        let (_, px) = draw_far(&ribbon(true), &across(3.5), tangent_wrap(), yaw, far);
        assert_eq!(holes(&px), Vec::<u32>::new(), "×{far}・yaw {yaw}");
    }
}

/// 文書の x 128 で 90 度に折った 2 枚の四角（x 128 から先は奥へ。UV は文書の全体のまま）の、文書の点 (x, y) の面の点。
fn fold_at(x: f64, y: f64) -> Vec3 {
    let (x, y) = (x as f32, y as f32);
    if x <= 128.0 {
        Vec3::new(x, y, 0.0)
    } else {
        Vec3::new(128.0, y, FOLD_SIDE * (x - 128.0))
    }
}

/// 折った側の z の向き（カメラから離れる側）。
const FOLD_SIDE: f32 = 1.0;

fn folded() -> Arc<SurfaceGeometry> {
    let (w, h) = (W as f32, H as f32);
    let uv = |x: f32, y: f32| Vec2::new(x / w, y / h);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        let p = |q: (f32, f32)| fold_at(q.0 as f64, q.1 as f64);
        SurfaceTriangle::new(p(a), p(b), p(c), uv(a.0, a.1), uv(b.0, b.1), uv(c.0, c.1))
            .with_slot(0, 0, 0)
    };
    let m = 128.0;
    Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((0.0, 0.0), (0.0, h), (m, 0.0)),
                tri((m, 0.0), (0.0, h), (m, h)),
                tri((m, 0.0), (m, h), (w, 0.0)),
                tri((w, 0.0), (m, h), (w, h)),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

#[test]
fn tangent_wrap_keeps_the_ribbon_band_unbroken_across_a_folded_edge() {
    // 90 度に折った辺をまたいで引く: 描点の中心の間の弦は面に沿った距離より短い。線の長さを今の描点の展開の上で測るので、折り目にも
    // 抜けが無い
    let camera = OrbitCamera {
        target: Vec3::new(128.0, H as f32 / 2.0, 0.0),
        yaw: FOLD_YAW,
        ..slanted_camera()
    };
    let (_, px) = draw_on(
        folded(),
        &ribbon(true),
        &across(3.5),
        tangent_wrap(),
        camera,
        fold_at,
    );
    assert_eq!(holes(&px), Vec::<u32>::new());
}

/// 折った 2 枚がどちらも見えるカメラの横の回し。
const FOLD_YAW: f32 = -45.0;

/// 線の向きに沿わせる、細長い丸の筆先（長い半径 12・短い半径 1.2）。
fn thin_follow() -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 12.0,
        hardness: 1.0,
        spacing: 0.05,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.tip.roundness = 0.1;
    b.tip.follow_direction = true;
    b
}

#[test]
fn tangent_wrap_turns_a_direction_following_tip_along_the_line_on_the_surface() {
    // 60 度回して見た四角に、面の上で 45 度の線を引く（画面の上では約 63 度）。筆先が面の上の線の向きに沿えば、線の幅は短い半径の
    // 2 倍ほど。画面の角度のまま回すと、長い半径が線から 18 度ほど外へ出て、幅は 3 倍より広くなる
    let line: Vec<(f64, f64, f64)> = (0..=40)
        .map(|i| (70.0 + 2.0 * i as f64, 8.0 + 2.0 * i as f64, 0.0))
        .collect();
    let (_, px) = draw_slanted(&thin_follow(), &line, tangent_wrap(), 60.0);
    // 線の真ん中の行（y 48）で、塗った画素の数（線の幅 × √2）。沿えば 3、画面の角度のままでは 11
    let across = (80..140u32).filter(|&x| alpha(&px, x, 48) > 0).count();
    assert!((2..=6).contains(&across), "{across}");
}

/// 面の上で 45 度の線（`tangent_wrap_turns_a_direction_following_tip_along_the_line_on_the_surface` と同じ）を 60 度回して見た四角に引き、
/// 線の真ん中の行（y 48）で塗った画素の数と、画素。
fn width_on_slant(brush: &Brush) -> (usize, Vec<u8>) {
    let line: Vec<(f64, f64, f64)> = (0..=40)
        .map(|i| (70.0 + 2.0 * i as f64, 8.0 + 2.0 * i as f64, 0.0))
        .collect();
    let (_, px) = draw_slanted(brush, &line, tangent_wrap(), 60.0);
    ((80..140u32).filter(|&x| alpha(&px, x, 48) > 0).count(), px)
}

/// 半径 1 のブラシに、細長い筆先（`thin_follow`）の粒子を 1 つずつ、ほぼ描点の中心に置く散布。
fn sprayed(follow: bool, direction: SprayDirection) -> Brush {
    let mut b = thin_follow();
    b.base.radius = 1.0;
    b.tip.follow_direction = follow;
    b.spray = Some(Spray {
        size: 24.0,
        density: 1.0,
        bias: 1.0,
        direction,
        ..Spray::default()
    });
    b
}

#[test]
fn tangent_wrap_turns_a_sprayed_particle_only_when_the_spray_follows_the_line() {
    // 散布の粒子の角度は散布の向きで決まる: 向きが「線の向き」なら、線の向きに沿わせる設定が切でも面の上の線に沿う
    let (line, _) = width_on_slant(&sprayed(false, SprayDirection::Line));
    assert!((2..=6).contains(&line), "{line}");
    assert_eq!(
        width_on_slant(&sprayed(true, SprayDirection::Line)).1,
        width_on_slant(&sprayed(false, SprayDirection::Line)).1
    );
    // 向きが「散布全体の向き」なら、線の向きに沿わせる設定が入りでも回さない
    let (whole, px) = width_on_slant(&sprayed(true, SprayDirection::Whole));
    assert_eq!(px, width_on_slant(&sprayed(false, SprayDirection::Whole)).1);
    assert!(whole > 6, "{whole}");
}

/// 主は丸い大きな筆先、2 つ目は細長い筆先（線の向きに沿わせる）で、乗算で重ねたデュアルブラシ。
fn dual_thin(angle_line: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 12.0,
        hardness: 1.0,
        spacing: 0.05,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.dual = Some(DualBrush {
        radius: 12.0,
        roundness: 0.1,
        spacing: 0.05,
        mode: DualBrushMode::Multiply,
        angle_line,
        ..DualBrush::default()
    });
    b
}

#[test]
fn tangent_wrap_turns_the_second_tip_of_a_dual_brush_along_the_line_on_the_surface() {
    // 2 つ目の筆先も主の筆先と同じく、面の上の線の向きに沿う（乗算なので、塗れるのは 2 つ目の細い帯の内だけ）
    let (along, _) = width_on_slant(&dual_thin(true));
    assert!((2..=6).contains(&along), "{along}");
    let (fixed, _) = width_on_slant(&dual_thin(false));
    assert!(fixed > 6, "{fixed}");
}

#[test]
fn tangent_wrap_turns_a_sprayed_second_tip_only_when_its_spray_follows_the_line() {
    // 2 つ目に散布が入ると粒子の角度は散布の向きで決まる: 向きが「線の向き」なら、線の向きに沿わせる設定が切でも面の上の線に沿い、
    // 「散布全体の向き」なら、設定が入りでも回さない
    let dual_sprayed = |angle_line: bool, direction: SprayDirection| {
        let mut b = dual_thin(angle_line);
        let dual = b.dual.as_mut().unwrap();
        // 粒子はほぼ描点の中心に 1 つずつ置く（主の散布のテストと同じ）
        dual.radius = 1.0;
        dual.spray = Some(Spray {
            size: 24.0,
            density: 1.0,
            bias: 1.0,
            direction,
            ..Spray::default()
        });
        b
    };
    let (line, _) = width_on_slant(&dual_sprayed(false, SprayDirection::Line));
    assert!((2..=8).contains(&line), "{line}");
    assert_eq!(
        width_on_slant(&dual_sprayed(true, SprayDirection::Line)).1,
        width_on_slant(&dual_sprayed(false, SprayDirection::Line)).1
    );
    let (whole, px) = width_on_slant(&dual_sprayed(true, SprayDirection::Whole));
    assert_eq!(
        px,
        width_on_slant(&dual_sprayed(false, SprayDirection::Whole)).1
    );
    assert!(whole > 12, "{whole}");
}

#[test]
fn tangent_wrap_turns_a_direction_following_tip_along_the_line_under_an_orthographic_camera() {
    // 正投影でも、面の上の 45 度の線に筆先が沿う
    let line: Vec<(f64, f64, f64)> = (0..=40)
        .map(|i| (70.0 + 2.0 * i as f64, 8.0 + 2.0 * i as f64, 0.0))
        .collect();
    let mut camera = OrbitCamera {
        yaw: 60.0,
        ..slanted_camera()
    };
    camera.set_orthographic(true);
    let (_, px) = draw_on(
        quad(),
        &thin_follow(),
        &line,
        tangent_wrap(),
        camera,
        |x, y| Vec3::new(x as f32, y as f32, 0.0),
    );
    let across = (80..140u32).filter(|&x| alpha(&px, x, 48) > 0).count();
    assert!((2..=6).contains(&across), "{across}");
}

#[test]
fn tangent_wrap_turned_tip_bytes_do_not_depend_on_threads() {
    // 候補が並列の下限を超える大きな細長い筆先を、斜めに見た四角に斜めに引く（枠を回す道を通る）
    let mut b = thin_follow();
    b.base.radius = 150.0;
    b.base.spacing = 0.02;
    let line: Vec<(f64, f64, f64)> = (0..=10)
        .map(|i| (90.0 + 8.0 * i as f64, 20.0 + 5.0 * i as f64, 0.0))
        .collect();
    let run = || draw_slanted(&b, &line, tangent_wrap(), 60.0).1;
    let base = run();
    assert!(base.chunks_exact(4).filter(|c| c[3] > 0).count() > 2_000);
    for threads in [1usize, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        assert!(
            pool.install(run) == base,
            "{threads} スレッド: バイトが違う"
        );
    }
}

#[test]
fn tangent_wrap_paints_the_dabs_added_by_holding_the_pen() {
    for n in [1usize, 7, 30] {
        let (on, px_on) = draw_with(&soft(true), &held(n), tangent_wrap());
        assert_eq!(on, 1 + n, "{n}");
        let (off, px_off) = draw_with(&soft(false), &held(n), tangent_wrap());
        assert_eq!(off, 1, "{n}");
        assert!(alpha(&px_on, 60, 40) > alpha(&px_off, 60, 40), "{n}");
    }
}
