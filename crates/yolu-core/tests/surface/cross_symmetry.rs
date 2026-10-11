//! 2D の対称と 3D の対称を、もう片方のビューのストロークにも当てる（`geometry::uv_symmetry`）: 3D のストロークに 2D の対称（UV の平面で
//! 写す）、2D のストロークに 3D の対称（ダブの中心の面の点を写して、写しの面の UV へ置く）、両方のときは 3D の写しの後に 2D の写し、
//! 写しの面が無いときの理由、指先・クローンの断り、3D の選択ペンのストローク（`SurfaceCoverStroke`）。

use std::sync::Arc;

use yolu_core::geometry::{
    CoverParams, MirrorOutcome, MirrorPlane, OrbitCamera, ProjectionSettings, SurfaceCoverStroke,
    SurfaceEffect, SurfaceGeometry, SurfaceStroke, SurfaceStrokeError, SurfaceStrokeOptions,
    SurfaceSymmetrySetup, SurfaceTriangle, SymmetryAxis, UvGrid, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{DVec2, Quat, Vec2, Vec3};
use yolu_core::{
    Brush, BrushEffect, BrushSettings, CanvasSymmetry, CoreError, Document, LayerId, Rgba8,
    SymmetryMode,
};

const SIZE: u32 = 128;

/// x −1..1・y −1..1 の板（z = 0、法線は −Z）。UV は u = (x + 1) / 4（テクスチャの左半分）、v = (y + 1) / 2。half なら x ≤ 0 の半分だけ。
fn plate(half: bool) -> Arc<SurfaceGeometry> {
    let x1 = if half { 0.0 } else { 1.0 };
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new((x + 1.0) / 4.0, (y + 1.0) / 2.0);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
    };
    let t = vec![
        tri((-1.0, -1.0), (-1.0, 1.0), (x1, -1.0)),
        tri((x1, -1.0), (-1.0, 1.0), (x1, 1.0)),
    ];
    Arc::new(SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// −Z の側から板を見るカメラ。
fn front() -> yolu_core::geometry::CameraView {
    OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 4.0,
        model_radius: 1.0,
        ..Default::default()
    }
    .view(400.0, 400.0)
}

fn mirror_x() -> SurfaceSymmetrySetup {
    SurfaceSymmetrySetup {
        mirror: Some(MirrorPlane::from_model(
            Vec3::ZERO,
            Quat::IDENTITY,
            SymmetryAxis::X,
            0.0,
        )),
        radial: None,
        ignore_visibility: false,
    }
}

fn document() -> (Document, LayerId) {
    let mut d = Document::new(SIZE, SIZE).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn alpha(d: &Document, l: LayerId, x: u32, y: u32) -> u8 {
    d.layer(l)
        .unwrap()
        .surface(yolu_core::Channel::Color)
        .unwrap()
        .pixel(x, y)
        .unwrap()
        .a
}

fn painted(d: &Document, l: LayerId) -> Vec<(u32, u32, u8)> {
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = alpha(d, l, x, y);
            if a > 0 {
                out.push((x, y, a));
            }
        }
    }
    out
}

fn brush() -> BrushSettings {
    BrushSettings {
        radius: 6.0,
        hardness: 0.6,
        color: Rgba8::new(200, 40, 40, 255),
        ..BrushSettings::default()
    }
}

/// 板の点 (x, y) を画面へ。
fn screen(x: f32, y: f32) -> Vec2 {
    front().to_screen(Vec3::new(x, y, 0.0)).unwrap()
}

/// 3D の面のストロークで、板の点 (x, y) を 1 回押して離す。
fn surface_dot(
    d: &mut Document,
    l: LayerId,
    g: &Arc<SurfaceGeometry>,
    at: (f32, f32),
    options: SurfaceStrokeOptions,
) -> Result<(), SurfaceStrokeError> {
    let b = brush();
    let mut stroke = d.begin_stroke(l, &b).unwrap();
    let result = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        g.clone(),
        front(),
        &b,
        Some(0),
        screen(at.0, at.1),
        1.0,
        options,
    )
    .and_then(|mut s| s.finish(d, &mut stroke));
    match result {
        Ok(()) => {
            d.end_stroke(stroke).unwrap();
        }
        Err(_) => d.cancel_stroke(stroke),
    }
    result
}

/// 2D のキャンバスのストロークで、文書の点 (x, y) に 1 つ置く。
fn canvas_dot(
    d: &mut Document,
    l: LayerId,
    brush: &Brush,
    x: f64,
    y: f64,
) -> Result<(), CoreError> {
    let mut stroke = d.begin_brush_stroke(l, brush)?;
    stroke.add_point(d, x, y, 1.0, DVec2::ZERO)?;
    d.end_stroke(stroke).map(|_| ())
}

/// 縦の軸（文書の x = 64）の 2D の対称。
fn canvas_vertical() -> CanvasSymmetry {
    CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(64.0, 64.0), 2).unwrap()
}

#[test]
fn a_3d_stroke_with_the_2d_mirror_also_paints_the_mirrored_uv_pixels() {
    let g = plate(false);
    let (mut d, l) = document();
    surface_dot(
        &mut d,
        l,
        &g,
        (-0.5, 0.0),
        SurfaceStrokeOptions {
            canvas_symmetry: Some(canvas_vertical()),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    // 板の (−0.5, 0) は UV (0.125, 0.5) → 文書の (16, 64)。縦の軸 x = 64 で (112, 64) へ写る（板の UV の外でも、2D と同じく UV の平面で写す）
    let dots = painted(&d, l);
    assert!(dots
        .iter()
        .any(|&(x, y, _)| x < 32 && (56..72).contains(&y)));
    assert!(dots
        .iter()
        .any(|&(x, y, _)| x > 96 && (56..72).contains(&y)));
    // 軸が画素の境にあるので、写しの画素は元の画素の左右反転と同じ量
    for &(x, y, a) in &dots {
        assert_eq!(alpha(&d, l, SIZE - 1 - x, y), a, "({x}, {y})");
    }
    // 2D の対称が無ければ、元の側だけ
    let (mut plain, pl) = document();
    surface_dot(
        &mut plain,
        pl,
        &g,
        (-0.5, 0.0),
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    let left: Vec<_> = dots.iter().filter(|p| p.0 < 64).copied().collect();
    assert_eq!(
        painted(&plain, pl),
        left,
        "元の側は 2D の対称の有無で変わらない"
    );
}

#[test]
fn a_3d_stroke_with_an_oblique_2d_mirror_paints_the_mirrored_uv_pixels() {
    let g = plate(false);
    // 45 度の鏡（中心を通る y = x の線）: 文書の画素 (x, y) は (y, x) へ写る。軸が画素の中心を通るので、写しの覆いは元の覆いと画素ごとに同じ
    let (mut d, l) = document();
    surface_dot(
        &mut d,
        l,
        &g,
        (-0.5, 0.0),
        SurfaceStrokeOptions {
            canvas_symmetry: Some(CanvasSymmetry::lines(DVec2::new(64.0, 64.0), 2, 45.0).unwrap()),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    let dots = painted(&d, l);
    // 板の (−0.5, 0) は UV (0.125, 0.5) → 文書の (16, 64)。鏡の写しは (64, 16)
    assert!(dots
        .iter()
        .any(|&(x, y, _)| x < 32 && (56..72).contains(&y)));
    assert!(dots
        .iter()
        .any(|&(x, y, _)| y < 32 && (56..72).contains(&x)));
    for &(x, y, a) in &dots {
        assert_eq!(alpha(&d, l, y, x), a, "({x}, {y}) と ({y}, {x}) は同じ覆い");
    }
    // 軸の角度が 45 度でない鏡（30 度）は、軸に直交する向きへ鏡に写す
    let (mut d, l) = document();
    let center = DVec2::new(64.0, 64.0);
    surface_dot(
        &mut d,
        l,
        &g,
        (-0.5, 0.0),
        SurfaceStrokeOptions {
            canvas_symmetry: Some(CanvasSymmetry::lines(center, 2, 30.0).unwrap()),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    let mirror = CanvasSymmetry::lines(center, 2, 30.0)
        .unwrap()
        .transforms()
        .unwrap()[1];
    let (mx, my) = mirror.map(16.0, 64.0);
    let near = |cx: f64, cy: f64| {
        painted(&d, l).iter().any(|&(x, y, _)| {
            (x as f64 + 0.5 - cx).abs() <= 2.0 && (y as f64 + 0.5 - cy).abs() <= 2.0
        })
    };
    assert!(near(16.5, 64.5), "元");
    assert!(near(mx, my), "30 度の鏡の写し ({mx:.1}, {my:.1})");
}

#[test]
fn a_four_line_ruler_in_a_3d_stroke_paints_the_same_bytes_as_the_old_both() {
    use yolu_core::{Ruler, RulerId, RulerKind};
    let g = plate(false);
    let center = DVec2::new(64.0, 64.0);
    let mut ruler = Ruler::canvas(RulerId(1), RulerKind::Symmetry, center, center + DVec2::X);
    ruler.lines = 4;
    let from_ruler = ruler.canvas_symmetry().unwrap();
    let both = CanvasSymmetry::new(SymmetryMode::Both, center, 2).unwrap();
    // 軸にまたがる点（元と写しが重なる）と、離れた点
    for at in [(-0.5, 0.4), (0.0, 0.0), (-0.03, 0.04), (0.0, -0.5)] {
        let paint = |s: CanvasSymmetry| {
            let (mut d, l) = document();
            surface_dot(
                &mut d,
                l,
                &g,
                at,
                SurfaceStrokeOptions {
                    canvas_symmetry: Some(s),
                    ..SurfaceStrokeOptions::default()
                },
            )
            .unwrap();
            painted(&d, l)
        };
        let old = paint(both);
        assert!(!old.is_empty());
        assert_eq!(paint(from_ruler), old, "{at:?}");
    }
}

#[test]
fn a_3d_stroke_with_both_symmetries_paints_the_3d_copy_and_both_2d_copies() {
    let g = plate(false);
    let (mut d, l) = document();
    surface_dot(
        &mut d,
        l,
        &g,
        (-0.5, 0.4),
        SurfaceStrokeOptions {
            symmetry: Some(mirror_x()),
            canvas_symmetry: Some(
                CanvasSymmetry::new(SymmetryMode::Horizontal, DVec2::new(64.0, 64.0), 2).unwrap(),
            ),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    // 元 (−0.5, 0.4) → 文書 (16, 89.6)。3D のミラー → (0.5, 0.4) → (48, 89.6)。横の軸 y = 64 の写し → (16, 38.4)・(48, 38.4)
    let dots = painted(&d, l);
    for (cx, cy) in [(16u32, 89u32), (48, 89), (16, 38), (48, 38)] {
        assert!(
            dots.iter()
                .any(|&(x, y, _)| x.abs_diff(cx) <= 2 && y.abs_diff(cy) <= 2),
            "({cx}, {cy}) に塗られる"
        );
    }
}

#[test]
fn smudge_and_clone_refuse_the_2d_symmetry_in_3d_too() {
    let g = plate(false);
    let (mut d, l) = document();
    let b = brush();
    let smudge = Brush {
        effect: BrushEffect::Smudge { strength: 1.0 },
        ..Brush::from(b)
    };
    let mut stroke = d.begin_brush_stroke(l, &smudge).unwrap();
    let result = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b,
        Some(0),
        screen(-0.5, 0.0),
        1.0,
        SurfaceStrokeOptions {
            effect: SurfaceEffect::Smudge,
            canvas_symmetry: Some(canvas_vertical()),
            ..SurfaceStrokeOptions::default()
        },
    );
    assert_eq!(result.err(), Some(SurfaceStrokeError::EffectWithSymmetry));
    d.cancel_stroke(stroke);
}

/// 板の UV の格子と、X のミラーの 3D の対称を 2D のブラシに付ける。
fn with_model_symmetry(g: &Arc<SurfaceGeometry>, mut b: Brush) -> Brush {
    let grid = Arc::new(UvGrid::new(g, 0));
    b.model_symmetry = yolu_core::geometry::ModelSymmetry::new(grid, &mirror_x()).map(Arc::new);
    assert!(b.model_symmetry.is_some());
    b
}

#[test]
fn a_2d_stroke_with_the_3d_mirror_paints_the_mirrored_surface_in_uv() {
    let g = plate(false);
    let (mut d, l) = document();
    let b = with_model_symmetry(&g, Brush::from(brush()));
    // 文書の (16.5, 64.5) は板の (−0.48…, 0.0…)。X のミラーの面の点は UV で u → 0.5 − u、文書の x → 64 − x（画素 x → 63 − x）
    canvas_dot(&mut d, l, &b, 16.5, 64.5).unwrap();
    let dots = painted(&d, l);
    assert!(dots.iter().any(|&(x, _, _)| x < 32));
    assert!(dots.iter().any(|&(x, _, _)| (32..64).contains(&x)));
    for &(x, y, a) in dots.iter().filter(|p| p.0 < 32) {
        let m = alpha(&d, l, 63 - x, y);
        assert!(m.abs_diff(a) <= 1, "({x}, {y}) {a} と写し {m}");
    }
    assert_eq!(
        d.undo_count(),
        1,
        "写しも 1 つのストローク（1 回の取り消し）"
    );
    // 写しの面が無い（板の右半分が無い）: 写しは飛ばし、理由を残す
    let half = plate(true);
    let (mut d2, l2) = document();
    let b2 = with_model_symmetry(&half, Brush::from(brush()));
    let mut stroke = d2.begin_brush_stroke(l2, &b2).unwrap();
    stroke
        .add_point(&mut d2, 16.5, 64.5, 1.0, DVec2::ZERO)
        .unwrap();
    assert_eq!(
        d2.active_stroke_stats().unwrap().copy_note,
        Some(MirrorOutcome::NoSurface)
    );
    d2.end_stroke(stroke).unwrap();
    assert!(painted(&d2, l2).iter().all(|p| p.0 < 32), "元の側だけ");
}

#[test]
fn a_2d_stroke_with_both_symmetries_applies_the_3d_copy_first_then_the_2d_copies() {
    let g = plate(false);
    let (mut d, l) = document();
    let mut b = with_model_symmetry(&g, Brush::from(brush()));
    b.symmetry = CanvasSymmetry::new(SymmetryMode::Horizontal, DVec2::new(64.0, 64.0), 2).unwrap();
    canvas_dot(&mut d, l, &b, 16.5, 40.5).unwrap();
    let dots = painted(&d, l);
    // 元 (16.5, 40.5)、3D の写し (47.5, 40.5)、2D の写し (16.5, 87.5)・(47.5, 87.5)
    for (cx, cy) in [(16u32, 40u32), (47, 40), (16, 87), (47, 87)] {
        assert!(
            dots.iter()
                .any(|&(x, y, _)| x.abs_diff(cx) <= 1 && y.abs_diff(cy) <= 1),
            "({cx}, {cy}) に塗られる"
        );
    }
    // 元の側がどの面にも無いダブ（UV の空き）は、3D の写しを作らない（2D の写しだけ）
    let (mut d2, l2) = document();
    canvas_dot(&mut d2, l2, &b, 100.5, 40.5).unwrap();
    let dots = painted(&d2, l2);
    assert!(dots.iter().all(|p| p.0 >= 90), "{dots:?}");
    assert!(dots.iter().any(|p| p.1 > 64) && dots.iter().any(|p| p.1 < 64));
}

#[test]
fn smudge_and_clone_refuse_the_3d_symmetry_on_the_2d_canvas() {
    let g = plate(false);
    let (mut d, l) = document();
    for effect in [
        BrushEffect::Smudge { strength: 1.0 },
        BrushEffect::Clone {
            offset: DVec2::new(4.0, 0.0),
        },
    ] {
        let b = with_model_symmetry(
            &g,
            Brush {
                effect,
                ..Brush::from(brush())
            },
        );
        assert!(
            matches!(d.begin_brush_stroke(l, &b), Err(CoreError::Unsupported(_))),
            "{effect:?}"
        );
    }
    assert!(!d.has_active_stroke());
}

#[test]
fn the_3d_selection_pen_returns_the_projected_amounts_and_refuses_when_out_of_memory() {
    let g = plate(false);
    let params = CoverParams {
        radius: 6.0,
        hardness: 0.6,
        opacity: 1.0,
        pressure_size: false,
        pressure_opacity: true,
    };
    let (mut s, first) = SurfaceCoverStroke::begin(
        g.clone(),
        front(),
        Some(0),
        params,
        ProjectionSettings::default(),
        SIZE,
        SIZE,
        screen(-0.5, 0.0),
        1.0,
        64 << 20,
    )
    .unwrap();
    // 押した点の下（文書の (16, 64) のまわり）が満量
    assert!(first
        .iter()
        .any(|&(x, y, a)| (15..=17).contains(&x) && (63..=65).contains(&y) && a == 255));
    assert!(first.iter().all(|&(x, _, _)| x < 32));
    // 線は間隔ごとのダブ。筆圧は前の点から補間し、筆圧で量が減る（最後の点の筆圧 0.5 の近くは半分ほど）
    let line = s.add(screen(-0.3, 0.0), 0.5, 64 << 20).unwrap();
    assert!(s.dabs() > 3, "{}", s.dabs());
    let end = s.add(screen(-0.2, 0.0), 0.5, 64 << 20).unwrap();
    assert!(!line.is_empty() && !end.is_empty());
    assert!(end.iter().all(|&(_, _, a)| a <= 128), "{end:?}");
    assert!(end.iter().any(|&(_, _, a)| a >= 120));
    // 投影の塗りのメモリが入らなければ断る（呼ぶ側がストロークを捨てる）
    let refused = SurfaceCoverStroke::begin(
        g,
        front(),
        Some(0),
        params,
        ProjectionSettings::default(),
        SIZE,
        SIZE,
        screen(-0.5, 0.0),
        1.0,
        0,
    );
    assert!(matches!(
        refused.err(),
        Some(SurfaceStrokeError::Dab(
            yolu_core::geometry::DabRefusal::MemoryBudget
        ))
    ));
}

/// x0..x1 × y −1..1、高さ z の板（法線は −Z）。UV は u = u0 + (x − x0) / (x1 − x0) × (u1 − u0)、v = (y + 1) / 2。
fn quad(out: &mut Vec<SurfaceTriangle>, (x0, x1): (f32, f32), z: f32, (u0, u1): (f32, f32)) {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    let uv = |x: f32, y: f32| Vec2::new(u0 + (x - x0) / (x1 - x0) * (u1 - u0), (y + 1.0) / 2.0);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
    };
    out.push(tri((x0, -1.0), (x0, 1.0), (x1, -1.0)));
    out.push(tri((x1, -1.0), (x0, 1.0), (x1, 1.0)));
}

/// 面 A（z = 0）と B（z = 2）が同じ UV（u 0〜0.2）を持ち、X のミラーの先の A′・B′ は別の UV（u 0.4〜0.6 と 0.7〜0.9）。
/// flat なら B′ の UV を 1 点に潰す。
fn overlapped(flat: bool) -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(&mut t, (-2.0, -1.0), 0.0, (0.0, 0.2));
    quad(&mut t, (-2.0, -1.0), 2.0, (0.0, 0.2));
    quad(&mut t, (1.0, 2.0), 0.0, (0.4, 0.6));
    quad(&mut t, (1.0, 2.0), 2.0, (0.7, 0.9));
    if flat {
        for tri in t.iter_mut().skip(6) {
            (tri.uv_a, tri.uv_b, tri.uv_c) = (
                Vec2::new(0.8, 0.5),
                Vec2::new(0.8, 0.5),
                Vec2::new(0.8, 0.5),
            );
        }
    }
    Arc::new(SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

fn ink_near(d: &Document, l: LayerId, (x, y): (u32, u32), r: u32) -> bool {
    (y.saturating_sub(r)..=(y + r).min(SIZE - 1))
        .any(|yy| (x.saturating_sub(r)..=(x + r).min(SIZE - 1)).any(|xx| alpha(d, l, xx, yy) > 0))
}

#[test]
fn a_2d_stroke_on_overlapping_uvs_copies_from_every_face_that_shares_the_uv() {
    // 2D の文書の (12.8, 64) は A と B の両方の UV。3D ビューで A を塗れば A′ に、B を塗れば B′ に写しが出るので、2D でも両方に出す
    let g = overlapped(false);
    let (mut d, l) = document();
    let b = with_model_symmetry(&g, Brush::from(brush()));
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    stroke
        .add_point(&mut d, 12.8, 64.0, 1.0, DVec2::ZERO)
        .unwrap();
    let result = d.end_stroke(stroke).unwrap();
    assert_eq!(result.copy_note, None, "どの写しも面に届く");
    assert!(ink_near(&d, l, (12, 64), 1), "元");
    assert!(ink_near(&d, l, (64, 64), 1), "A の写し（A′）");
    assert!(ink_near(&d, l, (102, 64), 1), "B の写し（B′）");
    // 写しは、写し先と写し方が同じものを 1 つにまとめる（A と B の写しは別。(12.8, 64) は四角の対角線の上で、A と B の
    // それぞれ 2 つの三角形に入るが、同じ面の隣の三角形どうしの写しは 1 つ）
    let m = b.model_symmetry.as_ref().unwrap();
    let edge = m.copies(12.8, 64.0, 6.0, SIZE, SIZE).unwrap();
    assert_eq!(edge.copies.len(), 2, "{edge:?}");
    let inside = m.copies(12.8, 60.0, 6.0, SIZE, SIZE).unwrap();
    assert_eq!(inside.copies.len(), 2, "三角形の中でも同じ数: {inside:?}");
}

#[test]
fn a_copy_onto_a_flat_uv_is_skipped_with_its_own_reason() {
    let g = overlapped(true);
    let (mut d, l) = document();
    let b = with_model_symmetry(&g, Brush::from(brush()));
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    stroke
        .add_point(&mut d, 12.8, 64.0, 1.0, DVec2::ZERO)
        .unwrap();
    let result = d.end_stroke(stroke).unwrap();
    assert_eq!(result.copy_note, Some(MirrorOutcome::UvMismatch));
    assert!(ink_near(&d, l, (64, 64), 1), "潰れていない A′ には写す");
}

#[test]
fn a_multi_channel_stroke_finds_each_dab_copy_once_for_all_channels() {
    use yolu_core::material::ChannelPaint;
    use yolu_core::Channel;
    let g = plate(false);
    let points = [(16.5, 64.5), (24.5, 60.5), (30.5, 70.5)];
    // 1 チャンネル
    let (mut d, l) = document();
    let one = with_model_symmetry(&g, Brush::from(brush()));
    let mut stroke = d.begin_brush_stroke(l, &one).unwrap();
    for (x, y) in points {
        stroke.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    d.end_stroke(stroke).unwrap();
    let single = one.model_symmetry.as_ref().unwrap().computed();
    assert!(single > 3, "{single}");
    // 3 チャンネル: 同じダブの写しを、チャンネルごとに求め直さない
    let (mut d, l) = document();
    let three = with_model_symmetry(&g, Brush::from(brush()));
    let channels = [Channel::Color, Channel::Roughness, Channel::Metallic]
        .map(|c| ChannelPaint::new(c, Rgba8::new(200, 40, 40, 255)));
    let mut stroke = d.begin_material_brush_stroke(l, &channels, &three).unwrap();
    for (x, y) in points {
        stroke.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    assert!(d.end_stroke(stroke).unwrap().changed);
    assert_eq!(three.model_symmetry.as_ref().unwrap().computed(), single);
    // どのチャンネルにも写しが塗られている
    for c in [Channel::Roughness, Channel::Metallic] {
        let s = d.layer(l).unwrap().surface(c).unwrap();
        assert!(s.pixel(47, 64).unwrap().a > 0, "{c:?}");
    }
}
