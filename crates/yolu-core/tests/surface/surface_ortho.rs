//! 正投影のカメラでの 3D の塗り（`SurfaceProjector`・`SurfaceStroke`・玉のダブの遮蔽・当たり）の不変条件: 画面の円が奥行きによらない
//! 大きさで UV に落ちる・遮蔽は平行な視線で・裏の面・面の向きの弱めは前の向きとの角度（面の上で一定）・対称の写しは元のカメラから
//! 見えるテクセルだけ・取り消し・不正な見える高さは断る。期待は、テクセルを UV の重心座標で 3D の点へ戻し、カメラで画面へ写した素朴な
//! 計算から出す。

use std::collections::BTreeSet;
use std::sync::Arc;

use yolu_core::geometry::{
    pick, CameraView, DabRefusal, MirrorOutcome, MirrorPlane, OrbitCamera, Projection,
    ProjectionSettings, SurfaceBrushBudget, SurfaceGeometry, SurfaceProjector, SurfaceStroke,
    SurfaceStrokeOptions, SurfaceSymmetrySetup, SurfaceTriangle, SymmetryAxis,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::{AntiAlias, BrushSettings, Channel, Document, LayerId, Rgba8};

fn geometry(triangles: Vec<SurfaceTriangle>) -> Arc<SurfaceGeometry> {
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// z の高さの四角（x0..x1 × y0..y1、UV は 0〜1）。toward が負なら法線は −Z（−Z の側のカメラを向く）。
fn quad(
    out: &mut Vec<SurfaceTriangle>,
    z: f32,
    (x0, x1): (f32, f32),
    (y0, y1): (f32, f32),
    material: i32,
    toward: f32,
) {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    let uv = |x: f32, y: f32| Vec2::new((x - x0) / (x1 - x0), (y - y0) / (y1 - y0));
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        let (b, c) = if toward < 0.0 { (b, c) } else { (c, b) };
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
        .with_slot(material, material, material)
    };
    out.push(tri((x0, y0), (x0, y1), (x1, y0)));
    out.push(tri((x1, y0), (x0, y1), (x1, y1)));
}

/// −Z の側から +Z を見る正投影のカメラ（注視点 target、見える高さ height、表示域 size × size）。
fn ortho_front(target: Vec3, height: f32, size: f32) -> CameraView {
    OrbitCamera {
        target,
        yaw: 0.0,
        pitch: 0.0,
        distance: 4.0,
        model_radius: 1.0,
        projection: Projection::Orthographic { height },
    }
    .view(size, size)
}

/// 同じ向き・距離の透視のカメラ。
fn persp_front(target: Vec3, size: f32) -> CameraView {
    OrbitCamera {
        target,
        yaw: 0.0,
        pitch: 0.0,
        distance: 4.0,
        model_radius: 1.0,
        ..OrbitCamera::default()
    }
    .view(size, size)
}

fn painted(pixels: &[yolu_core::geometry::SurfacePixel]) -> BTreeSet<(i32, i32)> {
    pixels.iter().map(|p| (p.x, p.y)).collect()
}

/// 四角（x・y が −1〜1、UV 0〜1）のテクセルの中心の世界の点（z は四角の高さ）。
fn texel(x: i32, y: i32, size: i32, z: f32) -> Vec3 {
    Vec3::new(
        (x as f32 + 0.5) / size as f32 * 2.0 - 1.0,
        (y as f32 + 0.5) / size as f32 * 2.0 - 1.0,
        z,
    )
}

#[test]
fn an_axis_view_dab_lands_in_uv_at_the_screen_circles_size_at_any_depth() {
    // 見える高さ 4 を 400 点で: 1 点 = 0.01。半径 60 点 = 0.6 = UV の 19.2 テクセル（64 テクセルで 2）
    let mut sets = Vec::new();
    for z in [-0.5f32, 0.0, 0.7] {
        let mut t = Vec::new();
        quad(&mut t, z, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
        let g = geometry(t);
        let view = ortho_front(Vec3::ZERO, 4.0, 400.0);
        let center = Vec2::new(200.0, 200.0);
        let mut p = SurfaceProjector::new(
            g.clone(),
            &view,
            Some(0),
            64,
            64,
            60.0,
            ProjectionSettings::default(),
        )
        .unwrap();
        let dab = p.dab(center, 60.0, 1.0, u64::MAX);
        assert!(dab.refusal.is_none());
        let got = painted(&dab.pixels);
        for y in 0..64 {
            for x in 0..64 {
                let w = texel(x, y, 64, z);
                // 正投影の写しは奥行きによらない: 画面の距離は世界の xy の距離 × 100
                let d = (view.to_screen(w).unwrap() - center).length();
                assert!((d - Vec2::new(w.x, w.y).length() * 100.0).abs() < 0.01);
                if d < 59.5 {
                    assert!(got.contains(&(x, y)), "z {z}: ({x}, {y}) {d}");
                } else if d > 60.5 {
                    assert!(!got.contains(&(x, y)), "z {z}: ({x}, {y}) {d}");
                }
            }
        }
        // 正面から見ているので弱めない
        assert!(dab.pixels.iter().all(|p| p.coverage == 1.0));
        let area = std::f32::consts::PI * 19.2 * 19.2;
        assert!(
            (got.len() as f32 - area).abs() < 0.05 * area,
            "{} {area}",
            got.len()
        );
        sets.push(got);
    }
    assert!(sets.windows(2).all(|w| w[0] == w[1]), "奥行きによらず同じ");
    // 透視では奥行きで大きさが変わる（比べのため）
    let sizes: Vec<usize> = [-0.5f32, 0.7]
        .into_iter()
        .map(|z| {
            let mut t = Vec::new();
            quad(&mut t, z, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
            let view = persp_front(Vec3::ZERO, 400.0);
            let mut p = SurfaceProjector::new(
                geometry(t),
                &view,
                Some(0),
                64,
                64,
                60.0,
                ProjectionSettings::default(),
            )
            .unwrap();
            p.dab(Vec2::new(200.0, 200.0), 60.0, 1.0, u64::MAX)
                .pixels
                .len()
        })
        .collect();
    assert!(sizes[0] < sizes[1], "{sizes:?}");
}

/// 奥の板（z = 0、マテリアル 0）と、手前で x が 0〜1 を覆う板（z = −1、マテリアル 1）。
fn plate_and_blocker() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(&mut t, 0.0, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
    quad(&mut t, -1.0, (0.0, 1.0), (-1.0, 1.0), 1, -1.0);
    geometry(t)
}

#[test]
fn parallel_sight_lines_hide_exactly_what_the_nearer_plate_covers() {
    let g = plate_and_blocker();
    let view = ortho_front(Vec3::ZERO, 3.0, 300.0);
    let center = Vec2::new(150.0, 150.0);
    let radius = 120.0;
    for hidden in [false, true] {
        let s = ProjectionSettings {
            paint_hidden: hidden,
            ..ProjectionSettings::default()
        };
        let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 64, 64, radius, s).unwrap();
        let got = painted(&p.dab(center, radius, 1.0, u64::MAX).pixels);
        let (mut seen, mut behind) = (0, 0);
        for y in 0..64 {
            for x in 0..64 {
                let w = texel(x, y, 64, 0.0);
                if (view.to_screen(w).unwrap() - center).length() > radius - 1.0 {
                    continue;
                }
                // 視線は平行なので、手前の板が覆うのは同じ x・y（透視なら 3/4 に縮む）
                if w.x > 0.02 && w.x < 0.98 && w.y.abs() < 0.98 {
                    behind += 1;
                    assert_eq!(got.contains(&(x, y)), hidden, "隠れた ({x}, {y})");
                } else if w.x < -0.02 {
                    seen += 1;
                    assert!(got.contains(&(x, y)), "見える ({x}, {y})");
                }
            }
        }
        assert!(seen > 300 && behind > 300, "{seen} {behind}");
    }
    // 当たり（スポイト・選び・描き始め）も平行な視線: 手前の板、無ければ奥の板。点は画面の点の真下
    let at = |p: Vec3| view.to_screen(p).unwrap();
    let hit = pick(&g, &view, at(Vec3::new(0.5, 0.25, 0.0))).unwrap();
    assert_eq!(hit.material, 1);
    assert!((hit.position - Vec3::new(0.5, 0.25, -1.0)).length() < 1e-4);
    let hit = pick(&g, &view, at(Vec3::new(-0.5, -0.25, 0.0))).unwrap();
    assert_eq!(hit.material, 0);
    assert!((hit.position - Vec3::new(-0.5, -0.25, 0.0)).length() < 1e-4);
    assert!(pick(&g, &view, at(Vec3::new(-1.2, 0.0, 0.0))).is_none());
}

#[test]
fn faces_away_from_the_view_are_painted_only_when_asked() {
    let mut t = Vec::new();
    quad(&mut t, 0.0, (-1.0, 1.0), (-1.0, 1.0), 0, 1.0);
    let g = geometry(t);
    let view = ortho_front(Vec3::ZERO, 4.0, 400.0);
    let center = Vec2::new(200.0, 200.0);
    let mut p = SurfaceProjector::new(
        g.clone(),
        &view,
        Some(0),
        32,
        32,
        80.0,
        ProjectionSettings::default(),
    )
    .unwrap();
    assert!(p.dab(center, 80.0, 1.0, u64::MAX).pixels.is_empty());
    let s = ProjectionSettings {
        paint_backfaces: true,
        ..ProjectionSettings::default()
    };
    let mut p = SurfaceProjector::new(g, &view, Some(0), 32, 32, 80.0, s).unwrap();
    let dab = p.dab(center, 80.0, 1.0, u64::MAX);
    assert!(dab.pixels.len() > 100, "{}", dab.pixels.len());
    assert!(dab.pixels.iter().all(|p| p.coverage == 1.0));
}

#[test]
fn the_angle_falloff_uses_the_view_direction_and_is_even_over_a_flat_face() {
    for degrees in [30.0f32, 81.0, 82.5, 84.0, 86.0] {
        let r = Quat::from_rotation_y(degrees.to_radians());
        let mut t = Vec::new();
        quad(&mut t, 0.0, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
        for tri in &mut t {
            tri.a = r * tri.a;
            tri.b = r * tri.b;
            tri.c = r * tri.c;
        }
        let g = geometry(t);
        let view = ortho_front(Vec3::ZERO, 4.0, 400.0);
        let mut p = SurfaceProjector::new(
            g,
            &view,
            Some(0),
            64,
            64,
            150.0,
            ProjectionSettings::default(),
        )
        .unwrap();
        let dab = p.dab(Vec2::new(200.0, 200.0), 150.0, 1.0, u64::MAX);
        // 弱めは法線と前の向きの角度だけで決まる（始まり 80°・終わり 85°）
        let cos = degrees.to_radians().cos() as f64;
        let (start, end) = (80f64.to_radians().cos(), 85f64.to_radians().cos());
        let expected = ((cos - end) / (start - end)).clamp(0.0, 1.0) as f32;
        if expected == 0.0 {
            assert!(dab.pixels.is_empty(), "{degrees}°");
            continue;
        }
        assert!(!dab.pixels.is_empty(), "{degrees}°");
        for px in &dab.pixels {
            assert!(
                (px.coverage - expected).abs() < 1e-4,
                "{degrees}°: {} {expected}",
                px.coverage
            );
        }
    }
}

fn document(width: u32, height: u32) -> (Document, LayerId) {
    let mut d = Document::new(width, height).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn layer_bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(Vec::new, |s| s.to_canvas_bytes())
}

fn alpha(d: &Document, l: LayerId, x: i32, y: i32) -> u8 {
    d.layer(l)
        .unwrap()
        .pixel(Channel::Color, x as u32, y as u32)
        .unwrap()
        .a
}

/// 世界の点の列を画面の点で引くストローク（確定する）。
fn stroke_through(
    d: &mut Document,
    l: LayerId,
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    brush: &BrushSettings,
    world: &[Vec3],
    options: SurfaceStrokeOptions,
) -> SurfaceStroke {
    let mut stroke = d.begin_stroke(l, brush).unwrap();
    let screen: Vec<Vec2> = world.iter().map(|p| view.to_screen(*p).unwrap()).collect();
    let mut s = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        g.clone(),
        view,
        brush,
        Some(0),
        screen[0],
        1.0,
        options,
    )
    .unwrap();
    for p in &screen[1..] {
        s.add(d, &mut stroke, *p, 1.0).unwrap();
    }
    s.finish(d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    s
}

#[test]
fn an_anti_aliased_stroke_paints_the_same_bytes_at_any_depth_and_undoes_in_one_step() {
    let brush = BrushSettings {
        radius: 5.0,
        hardness: 1.0,
        anti_alias: AntiAlias::Strong,
        color: Rgba8::new(40, 120, 230, 255),
        ..BrushSettings::default()
    };
    let mut results = Vec::new();
    for z in [-0.4f32, 0.6] {
        let mut t = Vec::new();
        quad(&mut t, z, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
        let g = geometry(t);
        let view = ortho_front(Vec3::ZERO, 2.5, 320.0);
        let (mut d, l) = document(96, 96);
        let line = [
            Vec3::new(-0.6, -0.3, z),
            Vec3::new(0.0, 0.2, z),
            Vec3::new(0.5, -0.1, z),
        ];
        stroke_through(
            &mut d,
            l,
            &g,
            view,
            &brush,
            &line,
            SurfaceStrokeOptions::default(),
        );
        let bytes = layer_bytes(&d, l);
        assert!(bytes.chunks(4).any(|p| p[3] > 0 && p[3] < 255), "縁の帯");
        // 1 回の取り消しで空に、やり直しで同じバイトに
        assert!(d.undo().unwrap());
        assert!(layer_bytes(&d, l).chunks(4).all(|p| p[3] == 0));
        assert!(d.redo().unwrap());
        assert_eq!(layer_bytes(&d, l), bytes);
        results.push(bytes);
    }
    assert_eq!(results[0], results[1], "奥行きによらず同じ");
}

/// 奥の大きい板（x −2〜2、マテリアル 0。128 × 64 の文書で x の 1 が 32 テクセル）と、x −0.6〜−0.2 を手前（カメラとの間）で覆う板
/// （マテリアル 1）。
fn plate_with_blocker_on_the_left() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(&mut t, 0.0, (-2.0, 2.0), (-1.0, 1.0), 0, -1.0);
    quad(&mut t, -2.0, (-0.6, -0.2), (-1.0, 1.0), 1, -1.0);
    geometry(t)
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

#[test]
fn a_mirror_copy_in_orthographic_keeps_only_texels_the_real_view_can_see() {
    let g = plate_with_blocker_on_the_left();
    let view = ortho_front(Vec3::ZERO, 5.0, 400.0);
    let sym = SurfaceStrokeOptions {
        symmetry: Some(mirror_x()),
        ..SurfaceStrokeOptions::default()
    };
    let brush = BrushSettings {
        radius: 4.0,
        hardness: 0.8,
        color: Rgba8::new(240, 200, 20, 255),
        ..BrushSettings::default()
    };
    // 右の 0.9 を塗る: 写しの −0.9 は覆いの外で、元の側を写した画素と同じ
    let (mut d, l) = document(128, 64);
    let s = stroke_through(
        &mut d,
        l,
        &g,
        view,
        &brush,
        &[Vec3::new(0.9, 0.0, 0.0)],
        sym,
    );
    assert_eq!(s.stats.copies, 1);
    assert_eq!(s.symmetry_note(), None);
    let main: Vec<(i32, i32)> = (0..64)
        .flat_map(|y| (64..128).map(move |x| (x, y)))
        .filter(|&(x, y)| alpha(&d, l, x, y) > 0)
        .collect();
    assert!(main.len() > 30, "{}", main.len());
    for &(x, y) in &main {
        assert_eq!(alpha(&d, l, x, y), alpha(&d, l, 127 - x, y), "({x}, {y})");
    }
    // 右の 0.4 を塗る: 写しの −0.4（半径 4 テクセル = 0.125）は、平行な視線で手前の板に全部隠れる。塗らずに知らせる
    let (mut d, l) = document(128, 64);
    let s = stroke_through(
        &mut d,
        l,
        &g,
        view,
        &brush,
        &[Vec3::new(0.4, 0.0, 0.0)],
        sym,
    );
    assert_eq!(s.symmetry_note(), Some(MirrorOutcome::Hidden));
    let any_left = (0..64).any(|y| (0..64).any(|x| alpha(&d, l, x, y) > 0));
    assert!(!any_left, "隠れた写しを塗った");
}

#[test]
fn the_ball_dab_shoots_parallel_occlusion_rays_from_the_near_plane() {
    let g = plate_and_blocker();
    let view = ortho_front(Vec3::ZERO, 3.0, 300.0);
    let hit = pick(
        &g,
        &view,
        view.to_screen(Vec3::new(-0.2, 0.0, 0.0)).unwrap(),
    )
    .unwrap();
    assert_eq!(hit.material, 0);
    // 半径 0.5 の球: 奥の板の x −0.7〜0.3。x が 0 より右は手前の板の陰（平行な視線では、ちょうど x = 0 から）
    let budget = SurfaceBrushBudget::default();
    let dab = g.build_surface_dabs(&hit, 0.5, 64, 64, view.viewer(), 1.0, &budget, None, false);
    assert!(dab.refusal.is_none());
    let got = painted(&dab.pixels);
    let (mut seen, mut behind) = (0, 0);
    for y in 0..64 {
        for x in 0..64 {
            let w = texel(x, y, 64, 0.0);
            if (w - hit.position).length() > 0.49 {
                continue;
            }
            if w.x > 0.02 {
                behind += 1;
                assert!(!got.contains(&(x, y)), "隠れた ({x}, {y})");
            } else if w.x < -0.02 {
                seen += 1;
                assert!(got.contains(&(x, y)), "見える ({x}, {y})");
            }
        }
    }
    assert!(seen > 50 && behind > 20, "{seen} {behind}");
    // 隠れていても、見える判定を切れば塗る（カメラによらない足跡）
    let all = g.build_surface_dabs(&hit, 0.5, 64, 64, view.viewer(), 1.0, &budget, None, true);
    assert!(all.pixels.len() > got.len());
    // 前の向きに背く面（裏の面）は辿らない
    let mut t = Vec::new();
    quad(&mut t, 0.0, (-1.0, 1.0), (-1.0, 1.0), 0, 1.0);
    let back = geometry(t);
    let hit = yolu_core::geometry::SurfaceHit {
        position: Vec3::ZERO,
        normal: Vec3::Z,
        ..hit
    };
    let hit = yolu_core::geometry::SurfaceHit {
        revision: back.revision(),
        triangle: 0,
        material: 0,
        renderer: back.triangles()[0].renderer,
        material_slot: back.triangles()[0].material_slot,
        ..hit
    };
    let none = back.build_surface_dabs(&hit, 0.5, 64, 64, view.viewer(), 1.0, &budget, None, false);
    assert!(none.pixels.is_empty());
}

#[test]
fn an_orthographic_height_that_is_not_a_positive_number_is_refused() {
    let mut t = Vec::new();
    quad(&mut t, 0.0, (-1.0, 1.0), (-1.0, 1.0), 0, -1.0);
    let g = geometry(t);
    for height in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
        let mut view = ortho_front(Vec3::ZERO, 4.0, 400.0);
        view.projection = Projection::Orthographic { height };
        let err = SurfaceProjector::new(
            g.clone(),
            &view,
            Some(0),
            64,
            64,
            30.0,
            ProjectionSettings::default(),
        )
        .err();
        assert_eq!(err, Some(DabRefusal::InvalidArguments), "{height}");
    }
    // カメラの見える高さが壊れていても、ビューを作るときに距離から直す
    let view = OrbitCamera {
        projection: Projection::Orthographic { height: f32::NAN },
        ..OrbitCamera::default()
    }
    .view(100.0, 100.0);
    assert!(matches!(
        view.projection,
        Projection::Orthographic { height } if height.is_finite() && height > 0.0
    ));
}
