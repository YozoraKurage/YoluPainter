//! 3D の投影の塗り（`SurfaceProjector` と、それを使う `SurfaceStroke`）の不変条件: 見える所・遮蔽・裏の面・面の向きの弱め・
//! 継ぎ目のにじみ・並列の度合いと覚えの捨てによらない決定性・予算でストロークを取り消さないこと・取り消しとカメラの変更・
//! 対称の写し（写したカメラの表裏・元のカメラから見えるか・放射状の回転の向き・中心が面の外のダブ・遅れて出る写しの予算）。
//! 期待は、テクセルを UV の重心座標で 3D の点へ戻し、カメラで画面へ写した素朴な計算（凸の形は表の面が見える）から出す。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::BTreeSet;
use std::sync::Arc;

use yolu_core::geometry::{
    cube_sphere, demo_cube, model_triangles, pick, CameraView, DabRefusal, MirrorOutcome,
    MirrorPlane, OrbitCamera, ProjectionSettings, RadialSymmetry, SurfaceBrushBudget,
    SurfaceGeometry, SurfaceProjector, SurfaceStroke, SurfaceStrokeOptions, SurfaceSymmetrySetup,
    SurfaceTriangle, SymmetryAxis, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::{BrushSettings, Channel, Document, LayerId, Rgba8};

fn geometry(triangles: Vec<SurfaceTriangle>) -> Arc<SurfaceGeometry> {
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// z の高さの四角（x0..x1 × y0..y1）。UV は uv0..uv1 の長方形。toward が負なら法線は −Z（−Z の側のカメラを向く）。
#[allow(clippy::too_many_arguments)]
fn quad(
    out: &mut Vec<SurfaceTriangle>,
    z: f32,
    (x0, x1): (f32, f32),
    (y0, y1): (f32, f32),
    (uv0, uv1): (Vec2, Vec2),
    material: i32,
    toward: f32,
) {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    let uv = |x: f32, y: f32| {
        Vec2::new(
            uv0.x + (uv1.x - uv0.x) * (x - x0) / (x1 - x0),
            uv0.y + (uv1.y - uv0.y) * (y - y0) / (y1 - y0),
        )
    };
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
    // (a, a + Y, a + X) の巻きで法線が −Z
    out.push(tri((x0, y0), (x0, y1), (x1, y0)));
    out.push(tri((x1, y0), (x0, y1), (x1, y1)));
}

/// −Z の側から +Z を見るカメラ（注視点 target、距離 distance）。
fn front_view(target: Vec3, distance: f32, size: f32) -> CameraView {
    OrbitCamera {
        target,
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: 1.0,
        ..Default::default()
    }
    .view(size, size)
}

/// 文書のテクセルの中心の 3D の点（塗るマテリアルの三角形の UV の中のもの。三角形の番号つき）。
fn texel_points(
    g: &SurfaceGeometry,
    width: i32,
    height: i32,
    material: i32,
) -> Vec<(i32, i32, Vec3, usize)> {
    let mut out = Vec::new();
    for (i, t) in g.triangles().iter().enumerate() {
        if t.material != material {
            continue;
        }
        let s = Vec2::new(width as f32, height as f32);
        let (a, b, c) = (t.uv_a * s, t.uv_b * s, t.uv_c * s);
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        let (e1, e2) = ((b - a).as_dvec2(), (c - a).as_dvec2());
        let det = e1.x * e2.y - e1.y * e2.x;
        if det.abs() < 1e-12 {
            continue;
        }
        for y in (lo.y.floor() as i32).max(0)..=(hi.y.ceil() as i32).min(height - 1) {
            for x in (lo.x.floor() as i32).max(0)..=(hi.x.ceil() as i32).min(width - 1) {
                let d = yolu_core::glam::DVec2::new(x as f64 + 0.5, y as f64 + 0.5) - a.as_dvec2();
                let b1 = (e2.y * d.x - e2.x * d.y) / det;
                let b2 = (-e1.y * d.x + e1.x * d.y) / det;
                let b0 = 1.0 - b1 - b2;
                if b0 < -1e-9 || b1 < -1e-9 || b2 < -1e-9 {
                    continue;
                }
                let p = t.a.as_dvec3() * b0 + t.b.as_dvec3() * b1 + t.c.as_dvec3() * b2;
                out.push((x, y, p.as_vec3(), i));
            }
        }
    }
    out
}

/// ダブの画素の (x, y) の集まり。
fn painted(pixels: &[yolu_core::geometry::SurfacePixel]) -> BTreeSet<(i32, i32)> {
    pixels.iter().map(|p| (p.x, p.y)).collect()
}

/// 面と視線の角度（度）。
fn view_angle(view: &CameraView, t: &SurfaceTriangle, p: Vec3) -> f32 {
    let n = t.normal();
    let to = (view.position - p).normalize();
    n.dot(to).abs().clamp(0.0, 1.0).acos().to_degrees()
}

fn settings() -> ProjectionSettings {
    ProjectionSettings::default()
}

/// 凸の形: 表の面の、画面の円の中のテクセルは塗られ（面の向きで弱める角度より手前）、裏の面と円の外は塗られない。
fn check_convex(g: &Arc<SurfaceGeometry>, view: &CameraView, center: Vec2, radius: f32, size: i32) {
    let mut p =
        SurfaceProjector::new(g.clone(), view, Some(0), size, size, radius, settings()).unwrap();
    let dab = p.dab(center, radius, 1.0, u64::MAX);
    assert!(dab.refusal.is_none());
    let got = painted(&dab.pixels);
    let (mut must, mut must_not) = (0, 0);
    for (x, y, point, i) in texel_points(g, size, size, 0) {
        let t = &g.triangles()[i];
        let front = t.normal().dot(view.position - point) > 0.0;
        let s = view.to_screen(point).unwrap();
        let d = (s - center).length();
        let angle = view_angle(view, t, point);
        if !front || d > radius + 0.25 || angle > 85.5 {
            must_not += 1;
            // 継ぎ目のにじみは、隣のアイランドのテクセルへ出ない（塗るセットの UV が覆うテクセルは、その三角形が塗るときだけ）。裏の面・円の外は
            // 同じテクセルを表の面・円の中の三角形が覆う（UV の辺の上）ときだけ塗られる
            if got.contains(&(x, y)) {
                let shared = texel_points(g, size, size, 0)
                    .into_iter()
                    .any(|(x2, y2, q, j)| {
                        (x2, y2) == (x, y)
                            && j != i
                            && g.triangles()[j].normal().dot(view.position - q) > 0.0
                            && (view.to_screen(q).unwrap() - center).length() < radius + 0.25
                    });
                assert!(
                    shared,
                    "塗らない所を塗った: ({x}, {y}) 表 {front} 距離 {d} 角度 {angle}"
                );
            }
        } else if d < radius - 0.25 && angle < 79.5 {
            must += 1;
            assert!(
                got.contains(&(x, y)),
                "塗る所を塗らない: ({x}, {y}) 距離 {d} 角度 {angle}"
            );
        }
    }
    assert!(must > 100 && must_not > 100, "{must} {must_not}");
    for px in &dab.pixels {
        assert!(px.coverage > 0.0 && px.coverage <= 1.0);
    }
}

#[test]
fn a_flat_plane_is_painted_exactly_inside_the_screen_circle() {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-1.0, 1.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        0,
        -1.0,
    );
    let g = geometry(t);
    let view = front_view(Vec3::ZERO, 4.0, 400.0);
    let center = Vec2::new(200.0, 200.0);
    let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 64, 64, 60.0, settings()).unwrap();
    let dab = p.dab(center, 60.0, 1.0, u64::MAX);
    let got = painted(&dab.pixels);
    for y in 0..64 {
        for x in 0..64 {
            let w = Vec3::new(
                (x as f32 + 0.5) / 32.0 - 1.0,
                (y as f32 + 0.5) / 32.0 - 1.0,
                0.0,
            );
            let d = (view.to_screen(w).unwrap() - center).length();
            if d < 59.5 {
                assert!(got.contains(&(x, y)), "({x}, {y}) {d}");
            } else if d > 60.5 {
                assert!(!got.contains(&(x, y)), "({x}, {y}) {d}");
            }
        }
    }
    // 硬さ 0 なら中心から縁へ弱まる（中心がいちばん濃い）
    let soft = p.dab(center, 60.0, 0.0, u64::MAX);
    let max = soft.pixels.iter().map(|p| p.coverage).fold(0.0, f32::max);
    let at_center = soft
        .pixels
        .iter()
        .find(|p| (p.x, p.y) == (32, 32))
        .unwrap()
        .coverage;
    assert_eq!(at_center, max);
    assert!(soft.pixels.iter().any(|p| p.coverage < 0.1));
}

#[test]
fn a_sphere_paints_its_front_inside_the_circle_and_never_its_back() {
    let g = geometry(model_triangles(&[cube_sphere(12, 0.5)]).unwrap());
    let view = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 25.0,
        pitch: 20.0,
        distance: 2.5,
        model_radius: 0.5,
        ..Default::default()
    }
    .view(320.0, 320.0);
    let center = view.to_screen(Vec3::ZERO).unwrap();
    // 球の画面の半径の 9 割の円（縁の近くの浅い角度も入る）
    let edge = view.world_radius_to_screen(Vec3::ZERO, 0.5);
    check_convex(&g, &view, center, edge * 0.9, 256);
    check_convex(&g, &view, center + Vec2::new(20.0, -10.0), edge * 0.4, 256);
}

#[test]
fn a_cube_from_a_corner_never_paints_the_three_hidden_faces() {
    let g = geometry(model_triangles(&[demo_cube()]).unwrap());
    let view = OrbitCamera {
        target: Vec3::ZERO,
        yaw: -40.0,
        pitch: 25.0,
        distance: 3.0,
        model_radius: 0.9,
        ..Default::default()
    }
    .view(300.0, 300.0);
    let center = view.to_screen(Vec3::ZERO).unwrap();
    check_convex(&g, &view, center, 140.0, 192);
}

/// 奥の板（マテリアル 0、z = 0）と、手前（z = −1）で右半分を遮る板（マテリアル 1）。
fn plate_and_blocker() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-1.0, 1.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        0,
        -1.0,
    );
    quad(
        &mut t,
        -1.0,
        (0.0, 1.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        1,
        -1.0,
    );
    geometry(t)
}

#[test]
fn nearer_faces_of_another_texture_set_hide_texels_unless_hidden_areas_are_painted() {
    let g = plate_and_blocker();
    let view = front_view(Vec3::ZERO, 4.0, 400.0);
    let center = Vec2::new(200.0, 200.0);
    let radius = 150.0;
    for hidden in [false, true] {
        let s = ProjectionSettings {
            paint_hidden: hidden,
            ..settings()
        };
        let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 64, 64, radius, s).unwrap();
        let got = painted(&p.dab(center, radius, 1.0, u64::MAX).pixels);
        let (mut seen, mut behind) = (0, 0);
        for (x, y, point, _) in texel_points(&g, 64, 64, 0) {
            let d = (view.to_screen(point).unwrap() - center).length();
            if d > radius - 1.0 {
                continue;
            }
            // カメラ (0, 0, −4) から奥の板の点へのレイが z = −1 を通る所の x（遮る板は 0〜1）
            let at = point.x * 0.75;
            let y_at = point.y * 0.75;
            let covered = at > 0.01 && at < 0.99 && y_at.abs() < 0.99;
            let open = at < -0.01;
            if covered {
                behind += 1;
                assert_eq!(got.contains(&(x, y)), hidden, "隠れた ({x}, {y})");
            } else if open {
                seen += 1;
                assert!(got.contains(&(x, y)), "見える ({x}, {y})");
            }
        }
        assert!(seen > 100 && behind > 100, "{seen} {behind}");
    }
}

#[test]
fn back_faces_are_painted_only_when_asked() {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-1.0, 1.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        0,
        1.0,
    );
    let g = geometry(t);
    let view = front_view(Vec3::ZERO, 4.0, 400.0);
    let center = Vec2::new(200.0, 200.0);
    let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 32, 32, 80.0, settings()).unwrap();
    assert!(p.dab(center, 80.0, 1.0, u64::MAX).pixels.is_empty());
    let s = ProjectionSettings {
        paint_backfaces: true,
        ..settings()
    };
    let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 32, 32, 80.0, s).unwrap();
    let dab = p.dab(center, 80.0, 1.0, u64::MAX);
    assert!(dab.pixels.len() > 100, "{}", dab.pixels.len());
    // 裏から見た角度で弱める（正面から見ているので弱めない）
    assert!(dab.pixels.iter().all(|p| p.coverage == 1.0));
}

#[test]
fn the_angle_falloff_is_monotone_in_the_angle_and_follows_its_range() {
    let mut last = f32::INFINITY;
    for degrees in [
        0.0f32, 30.0, 60.0, 75.0, 79.0, 80.5, 81.0, 82.0, 83.0, 84.0, 84.8, 86.0, 89.0,
    ] {
        // 板を Y 軸のまわりに回す（中心は原点。カメラは正面）
        let r = yolu_core::glam::Quat::from_rotation_y(degrees.to_radians());
        let mut t = Vec::new();
        quad(
            &mut t,
            0.0,
            (-1.0, 1.0),
            (-1.0, 1.0),
            (Vec2::ZERO, Vec2::ONE),
            0,
            -1.0,
        );
        for tri in &mut t {
            tri.a = r * tri.a;
            tri.b = r * tri.b;
            tri.c = r * tri.c;
        }
        let g = geometry(t);
        let view = front_view(Vec3::ZERO, 4.0, 400.0);
        let mut p =
            SurfaceProjector::new(g.clone(), &view, Some(0), 64, 64, 100.0, settings()).unwrap();
        let dab = p.dab(Vec2::new(200.0, 200.0), 100.0, 1.0, u64::MAX);
        // 中心の 2 × 2 のテクセルの覆いの最大（無ければ 0）
        let at = dab
            .pixels
            .iter()
            .filter(|p| (31..=32).contains(&p.x) && (31..=32).contains(&p.y))
            .map(|p| p.coverage)
            .fold(0.0f32, f32::max);
        assert!(at <= last + 1e-6, "{degrees}°: {at} > {last}");
        if degrees <= 79.0 {
            assert_eq!(at, 1.0, "{degrees}°");
        } else if degrees >= 86.0 {
            assert_eq!(at, 0.0, "{degrees}°");
        } else if (80.5..=84.8).contains(&degrees) {
            assert!(at > 0.0 && at < 1.0, "{degrees}°: {at}");
        }
        last = at;
        // 弱めを切れば、潰れる手前まで 1
        let s = ProjectionSettings {
            angle_falloff: false,
            ..settings()
        };
        let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 64, 64, 100.0, s).unwrap();
        let flat = p.dab(Vec2::new(200.0, 200.0), 100.0, 1.0, u64::MAX);
        if degrees <= 86.0 {
            assert!(flat.pixels.iter().all(|p| p.coverage == 1.0), "{degrees}°");
            assert!(!flat.pixels.is_empty(), "{degrees}°");
        }
    }
}

/// 左右に 1 枚ずつの板（世界の x が 0..1 と 1..2、辺 x = 1 を共有）。UV は左が x 0〜12 画素、右が 20〜32 画素の離れたアイランド（32 × 16）。
fn two_islands() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (0.0, 1.0),
        (0.0, 1.0),
        (Vec2::new(0.0, 0.125), Vec2::new(0.375, 0.875)),
        0,
        -1.0,
    );
    quad(
        &mut t,
        0.0,
        (1.0, 2.0),
        (0.0, 1.0),
        (Vec2::new(0.625, 0.125), Vec2::new(1.0, 0.875)),
        0,
        -1.0,
    );
    geometry(t)
}

#[test]
fn seam_bleed_paints_n_texels_outside_the_island_edges_and_no_further() {
    let g = two_islands();
    let view = front_view(Vec3::new(1.0, 0.5, 0.0), 5.0, 400.0);
    let center = view.to_screen(Vec3::new(1.0, 0.5, 0.0)).unwrap();
    for bleed in [0u32, 1, 2, 3] {
        let s = ProjectionSettings {
            seam_bleed: bleed,
            ..settings()
        };
        let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 32, 16, 150.0, s).unwrap();
        let dab = p.dab(center, 150.0, 1.0, u64::MAX);
        let row: BTreeSet<i32> = dab
            .pixels
            .iter()
            .filter(|p| p.y == 8)
            .map(|p| p.x)
            .collect();
        let b = bleed as i32;
        for x in 0..32 {
            let left = x < 12 + b;
            let right = x >= 20 - b;
            assert_eq!(row.contains(&x), left || right, "にじみ {bleed}: x {x}");
        }
        // にじみのテクセルは辺の上の点の値（硬さ 1 で 1）。三角形と位置は辺の上の点
        for px in dab
            .pixels
            .iter()
            .filter(|p| p.y == 8 && (12..20).contains(&p.x))
        {
            assert_eq!(px.coverage, 1.0);
            let x = px.position.x;
            assert!((x - 1.0).abs() < 1e-5, "辺の上の点: {}", px.position);
        }
        // 上と下の縁（行 0・1 と 14・15）にも出る
        let rows: BTreeSet<i32> = dab.pixels.iter().map(|p| p.y).collect();
        assert_eq!(rows.contains(&1), bleed >= 1, "{bleed}");
        assert_eq!(rows.contains(&0), bleed >= 2, "{bleed}");
    }
}

#[test]
fn seam_bleed_never_writes_into_another_island_of_the_same_set() {
    // 左の板は見え、右の板はカメラに背を向ける（塗らない）。右のアイランドは左のアイランドの 1 テクセル右から始まる
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (0.0, 1.0),
        (0.0, 1.0),
        (Vec2::new(0.0, 0.0), Vec2::new(0.375, 1.0)),
        0,
        -1.0,
    );
    quad(
        &mut t,
        0.0,
        (1.0, 2.0),
        (0.0, 1.0),
        (Vec2::new(0.40625, 0.0), Vec2::new(1.0, 1.0)),
        0,
        1.0,
    );
    let g = geometry(t);
    let view = front_view(Vec3::new(1.0, 0.5, 0.0), 5.0, 400.0);
    let center = view.to_screen(Vec3::new(0.6, 0.5, 0.0)).unwrap();
    let s = ProjectionSettings {
        seam_bleed: 4,
        ..settings()
    };
    let mut p = SurfaceProjector::new(g.clone(), &view, Some(0), 32, 16, 150.0, s).unwrap();
    let dab = p.dab(center, 150.0, 1.0, u64::MAX);
    let row: BTreeSet<i32> = dab
        .pixels
        .iter()
        .filter(|p| p.y == 8)
        .map(|p| p.x)
        .collect();
    assert!(row.contains(&12), "隙間のテクセル（12）はにじむ");
    for x in 13..32 {
        assert!(!row.contains(&x), "右のアイランドの x {x} へ出ない");
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

/// 重なった面の多いモデル: 球と、その手前で細かく折り返す蛇腹（全部つながった 1 つのメッシュ。同じマテリアル）。蛇腹の幅（x の 0.3）は
/// ブラシの球より狭いので、前の作りの面を辿るダブは、折り返しを越えて奥のレイヤーまで進む。
fn crowded() -> Arc<SurfaceGeometry> {
    let mut t = model_triangles(&[cube_sphere(24, 0.5)]).unwrap();
    let folds = 40;
    let (cols, rows) = (12, 20);
    for f in 0..folds {
        let z0 = -0.6 - f as f32 * 0.01;
        let z1 = z0 - 0.01;
        for j in 0..rows {
            for i in 0..cols {
                let (u0, u1) = (i as f32 / cols as f32, (i + 1) as f32 / cols as f32);
                let (v0, v1) = (j as f32 / rows as f32, (j + 1) as f32 / rows as f32);
                // 折り返す板: x が −0.15..0.15 を行き来し、z が少しずつ手前へ
                let x = |u: f32| {
                    if f % 2 == 0 {
                        -0.15 + 0.3 * u
                    } else {
                        0.15 - 0.3 * u
                    }
                };
                let z = |u: f32| z0 + (z1 - z0) * u;
                let uv = |u: f32, v: f32| {
                    Vec2::new((f as f32 + u) / folds as f32 * 0.5 + 0.5, v * 0.5 + 0.5)
                };
                let p = |u: f32, v: f32| Vec3::new(x(u), -0.25 + 0.5 * v, z(u));
                // 手前（−Z）を向く巻き（折り返しの向きで入れ替える）
                let (a, b, c, d) = (p(u0, v0), p(u0, v1), p(u1, v0), p(u1, v1));
                let (ua, ub, uc, ud) = (uv(u0, v0), uv(u0, v1), uv(u1, v0), uv(u1, v1));
                if f % 2 == 0 {
                    t.push(SurfaceTriangle::new(a, b, c, ua, ub, uc));
                    t.push(SurfaceTriangle::new(c, b, d, uc, ub, ud));
                } else {
                    t.push(SurfaceTriangle::new(a, c, b, ua, uc, ub));
                    t.push(SurfaceTriangle::new(c, d, b, uc, ud, ub));
                }
            }
        }
    }
    geometry(t)
}

fn crowded_view() -> CameraView {
    OrbitCamera {
        target: Vec3::ZERO,
        yaw: 15.0,
        pitch: 10.0,
        distance: 3.0,
        model_radius: 0.8,
        ..Default::default()
    }
    .view(480.0, 360.0)
}

/// 重なった所を横切るストローク（終わったら確定する）。radius は文書の画素のブラシの半径。
fn crowded_stroke(
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    memory: Option<u64>,
    radius: f64,
) -> (Document, LayerId, SurfaceStroke) {
    let (mut d, l) = document(512, 512);
    let brush = BrushSettings {
        radius,
        hardness: 0.6,
        color: Rgba8::new(200, 60, 40, 255),
        ..BrushSettings::default()
    };
    let mut stroke = d.begin_stroke(l, &brush).unwrap();
    let c = view.to_screen(Vec3::new(0.0, 0.0, -0.99)).unwrap();
    let (from, to) = (c - Vec2::new(200.0, 10.0), c + Vec2::new(200.0, 20.0));
    let mut s = SurfaceStroke::begin(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        Some(0),
        from,
        1.0,
    )
    .unwrap();
    s.set_projection_memory(memory);
    // 右へ横切り、少し下を左へ戻る
    let back = Vec2::new(0.0, 40.0);
    let points: Vec<Vec2> = (1..=6)
        .map(|i| from + (to - from) * (i as f32 / 6.0))
        .chain((0..=6).map(|i| to + back + (from - to) * (i as f32 / 6.0)))
        .collect();
    for p in points {
        s.add(&mut d, &mut stroke, p, 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    assert!(d.end_stroke(stroke).unwrap().changed);
    (d, l, s)
}

#[test]
fn the_painted_bytes_do_not_depend_on_threads_or_on_dropping_buckets() {
    let g = crowded();
    let view = crowded_view();
    let mut results = Vec::new();
    for threads in [1usize, 8, 32] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let (d, l, _) = pool.install(|| crowded_stroke(&g, view, None, 60.0));
        results.push((format!("{threads} スレッド"), layer_bytes(&d, l)));
    }
    let (d, l, _) = crowded_stroke(&g, view, None, 60.0);
    let base = layer_bytes(&d, l);
    assert!(base.chunks_exact(4).any(|c| c[3] > 0));
    for (name, bytes) in results {
        assert!(bytes == base, "{name}: バイトが違う");
    }
    // 覚えを小さくして、区画を捨てては作り直す（小さいブラシで長く引くと、ダブ 1 つの区画は全体より十分小さい）
    let (d, l, s) = crowded_stroke(&g, view, None, 16.0);
    let stats = s.projection_stats();
    let fixed = s.projection_bytes() - stats.cached_bytes;
    // どのダブも入るが、ストローク全体は入らない
    assert!(
        stats.peak_bytes > stats.largest_dab_bytes * 5 / 4,
        "{stats:?}"
    );
    let tight = fixed + stats.largest_dab_bytes;
    let (d2, l2, s2) = crowded_stroke(&g, view, Some(tight), 16.0);
    let stats = s2.projection_stats();
    assert!(stats.evictions > 0, "{stats:?}");
    assert!(
        s2.projection_bytes() <= tight,
        "{} > {tight}",
        s2.projection_bytes()
    );
    assert!(fixed + stats.peak_bytes <= tight);
    assert!(
        layer_bytes(&d2, l2) == layer_bytes(&d, l),
        "捨てて作り直すとバイトが違う"
    );
}

#[test]
fn a_big_brush_over_overlapping_faces_paints_instead_of_cancelling() {
    let g = crowded();
    let view = crowded_view();
    // 前の作り（ダブごとに面を辿ってレイを撃つ）は、この場面のダブを上限で断っていた（ストロークを取り消していた）
    let at = view.to_screen(Vec3::new(0.0, 0.0, -0.99)).unwrap();
    let hit = pick(&g, &view, at).expect("当たる");
    assert!(
        hit.position.z < -0.9,
        "蛇腹の手前のレイヤー: {}",
        hit.position
    );
    let radius = yolu_core::geometry::world_radius(&g, 60.0, 512);
    let old = g.build_surface_dabs(
        &hit,
        radius,
        512,
        512,
        view.position,
        0.6,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    assert!(
        old.refusal.is_some_and(|r| r.is_limit()),
        "{:?}",
        old.refusal
    );
    // 投影の塗りは取り消さずに塗る。覚えは 1 回の操作のメモリ（既定 64 MiB）の中
    let (d, l, s) = crowded_stroke(&g, view, None, 60.0);
    assert!(s.stats.dabs >= 6, "{:?}", s.stats);
    assert!(s.projection_bytes() <= d.stroke_budget_bytes());
    let count = layer_bytes(&d, l)
        .chunks_exact(4)
        .filter(|c| c[3] > 0)
        .count();
    assert!(count > 2000, "{count}");
}

#[test]
fn a_dab_whose_buckets_do_not_fit_cancels_the_stroke_like_2d() {
    let g = crowded();
    let view = crowded_view();
    let (mut d, l) = document(512, 512);
    let brush = BrushSettings {
        radius: 60.0,
        ..BrushSettings::default()
    };
    let mut stroke = d.begin_stroke(l, &brush).unwrap();
    let mut s = SurfaceStroke::begin(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        Some(0),
        Vec2::new(200.0, 180.0),
        1.0,
    )
    .unwrap();
    assert_eq!(s.stats.dabs, 1);
    // 区画の一覧の分しか無い: 次のダブは入らないので、ストロークごと断る（呼び手が取り消し、最初のダブも戻る。塗り残しを作らない）
    let fixed = s.projection_bytes() - s.projection_stats().cached_bytes;
    s.set_projection_memory(Some(fixed));
    let refused = [Vec2::new(260.0, 180.0), Vec2::new(270.0, 182.0)]
        .into_iter()
        .find_map(|at| s.add(&mut d, &mut stroke, at, 1.0).err())
        .or_else(|| s.finish(&mut d, &mut stroke).err());
    assert_eq!(
        refused,
        Some(yolu_core::geometry::SurfaceStrokeError::Dab(
            DabRefusal::MemoryBudget
        ))
    );
    d.cancel_stroke(stroke);
    assert!(!d.has_active_stroke());
    assert!(layer_bytes(&d, l).chunks_exact(4).all(|c| c[3] == 0));
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn one_undo_takes_a_stroke_back_and_a_new_camera_builds_new_buckets() {
    let g = geometry(model_triangles(&[demo_cube()]).unwrap());
    let (mut d, l) = document(192, 128);
    let brush = BrushSettings {
        radius: 10.0,
        hardness: 1.0,
        color: Rgba8::new(10, 200, 30, 255),
        ..BrushSettings::default()
    };
    let paint = |d: &mut Document, view: CameraView| -> (BTreeSet<usize>, SurfaceStroke) {
        let mut stroke = d.begin_stroke(l, &brush).unwrap();
        let at = view.to_screen(Vec3::ZERO).unwrap();
        let mut s = SurfaceStroke::begin(d, &mut stroke, g.clone(), view, &brush, Some(0), at, 1.0)
            .unwrap();
        // ダブは線の長さで間隔ごとに置くので、間隔より長く動かす（重なるダブが前の区画を使い回す）
        for i in 1..=6 {
            s.add(d, &mut stroke, at + Vec2::new(4.0 * i as f32, 0.0), 1.0)
                .unwrap();
        }
        s.finish(d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
        // 塗られた面（UV アイランド 3 × 2）
        let mut faces = BTreeSet::new();
        for y in 0..128 {
            for x in 0..192 {
                if d.composite_pixel(Channel::Color, x, y).unwrap().a > 0 {
                    faces.insert((x as usize * 3 / 192) + (y as usize * 2 / 128) * 3);
                }
            }
        }
        (faces, s)
    };
    let a = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 3.0,
        model_radius: 0.9,
        ..Default::default()
    };
    let (first, s) = paint(&mut d, a.view(300.0, 300.0));
    assert_eq!(first, [0].into_iter().collect(), "−Z の面");
    assert!(s.projection_stats().bucket_reuses > 0);
    d.undo().unwrap();
    assert_eq!(d.undo_count(), 0);
    assert!(
        (0..128).all(|y| (0..192).all(|x| d.composite_pixel(Channel::Color, x, y).unwrap().a == 0))
    );
    // 反対から見たカメラ: 前のストロークの覚えは使わず、見える面（+Z）を塗る
    let b = OrbitCamera { yaw: 180.0, ..a };
    let (second, s) = paint(&mut d, b.view(300.0, 300.0));
    assert_eq!(second, [1].into_iter().collect(), "+Z の面");
    assert!(s.projection_stats().buckets_built > 0);
}

/// 左のアイランドは黒、右のアイランドは白（どちらも不透明。余白は透明）の 32 × 16 の文書で、継ぎ目（世界の x = 1）に沿ってぼかす。budget は文書の
/// 1 回の操作の予算（投影の塗りのメモリは別に十分に取る）。通ったら確定し、断られたら取り消して、結果と文書を返す。
fn blur_along_the_seam(
    budget: Option<u64>,
) -> (
    Document,
    LayerId,
    Result<(), yolu_core::geometry::SurfaceStrokeError>,
) {
    let g = two_islands();
    let view = front_view(Vec3::new(1.0, 0.5, 0.0), 5.0, 400.0);
    let mut d = Document::with_tile_size(32, 16, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    for y in 2..14 {
        for x in 0..12 {
            d.set_pixel(l, x, y, Rgba8::new(0, 0, 0, 255)).unwrap();
        }
        for x in 20..32 {
            d.set_pixel(l, x, y, Rgba8::new(255, 255, 255, 255))
                .unwrap();
        }
    }
    d.clear_history().unwrap();
    if let Some(b) = budget {
        d.set_stroke_budget_bytes(b).unwrap();
    }
    // 間隔は、線の長さで置くダブが 9 個ほどになるように（入力の 8 区間のそれぞれに 1 つずつ置いていた前と同じくらいの、ぼかしの重なり）
    let brush = yolu_core::Brush {
        effect: yolu_core::BrushEffect::Blur { radius: 3 },
        ..yolu_core::Brush::from(BrushSettings {
            radius: 6.0,
            hardness: 1.0,
            spacing: 0.08,
            ..BrushSettings::default()
        })
    };
    let mut stroke = d.begin_brush_stroke(l, &brush).unwrap();
    let a = view.to_screen(Vec3::new(1.0, 0.2, 0.0)).unwrap();
    let b = view.to_screen(Vec3::new(1.0, 0.8, 0.0)).unwrap();
    let result = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &brush.base,
        Some(0),
        a,
        1.0,
        SurfaceStrokeOptions {
            effect: yolu_core::geometry::SurfaceEffect::Blur,
            projection_memory: Some(64 << 20),
            ..SurfaceStrokeOptions::default()
        },
    )
    .and_then(|mut s| {
        for i in 1..=8 {
            s.add(&mut d, &mut stroke, a + (b - a) * (i as f32 / 8.0), 1.0)?;
        }
        s.finish(&mut d, &mut stroke)?;
        // ダブは線の長さで間隔ごと（入力の点の数ではない）。どれも塗れた（断れば Err）
        assert!(s.stats.dabs >= 8, "{:?}", s.stats);
        Ok(())
    });
    match result {
        Ok(()) => assert!(d.end_stroke(stroke).unwrap().changed),
        Err(_) => d.cancel_stroke(stroke),
    }
    (d, l, result)
}

#[test]
fn a_blur_across_a_seam_mixes_both_islands_and_leaves_no_line() {
    let (mut d, l, result) = blur_along_the_seam(None);
    result.unwrap();
    let value = |d: &Document, x: u32, y: u32| {
        d.layer(l).unwrap().pixel(Channel::Color, x, y).unwrap().r as i32
    };
    // 継ぎ目の両側の縁のテクセルの差が小さい（線が出ない）。アイランドの奥は元のまま
    for y in 5..11 {
        let (left, right) = (value(&d, 11, y), value(&d, 20, y));
        assert!((right - left).abs() <= 48, "行 {y}: 左 {left} 右 {right}");
        assert!(
            left > 40 && right < 215,
            "行 {y}: 両側が混ざる {left} {right}"
        );
        assert_eq!((value(&d, 2, y), value(&d, 29, y)), (0, 255), "行 {y}");
    }
    d.undo().unwrap();
    assert_eq!(value(&d, 11, 8), 0, "1 回の Undo で戻る");
}

#[test]
fn a_seam_blur_whose_chart_does_not_fit_the_budget_still_paints() {
    // 展開の図が 1 回の操作の予算に入らないダブは、縁の画素も UV の画像の上の箱の平均で塗る（図のためにストロークを取り消さない）。
    // 予算を細かく変えて、取り消すのは塗る画素そのものが予算に入らないとき（core の StrokeBudgetExceeded）だけ
    let (wide, l, result) = blur_along_the_seam(None);
    result.unwrap();
    let wide = layer_bytes(&wide, l);
    let mut fallback = 0;
    for budget in (16u64..=34).map(|k| k << 10) {
        let (d, l, result) = blur_along_the_seam(Some(budget));
        match result {
            Ok(()) => {
                assert_eq!(d.undo_count(), 1, "{budget}");
                if layer_bytes(&d, l) != wide {
                    fallback += 1;
                }
            }
            Err(yolu_core::geometry::SurfaceStrokeError::Core(
                yolu_core::CoreError::StrokeBudgetExceeded,
            )) => assert_eq!(d.undo_count(), 0, "{budget}"),
            Err(e) => panic!("{budget}: 図のために取り消した {e:?}"),
        }
    }
    assert!(fallback > 0, "図に入らない予算で塗った場面が無い");
}

// ───────── 対称の写し（SurfaceStroke から） ─────────

/// 画素の不透明度（文書の画素の座標。UV × 大きさと同じ）。
fn alpha(d: &Document, l: LayerId, x: i32, y: i32) -> u8 {
    d.layer(l)
        .unwrap()
        .pixel(Channel::Color, x as u32, y as u32)
        .unwrap()
        .a
}

/// 塗られた画素（不透明度が 0 でない）の集まり。
fn painted_texels(d: &Document, l: LayerId) -> BTreeSet<(i32, i32)> {
    let (w, h) = (d.width() as i32, d.height() as i32);
    let mut out = BTreeSet::new();
    for y in 0..h {
        for x in 0..w {
            if alpha(d, l, x, y) > 0 {
                out.insert((x, y));
            }
        }
    }
    out
}

/// x = 0 の面で写す対称。
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

/// 世界の点の列を画面の点で引くストローク（確定する）。最初の点で始め、残りを足す。
#[allow(clippy::too_many_arguments)]
fn stroke_through(
    d: &mut Document,
    l: LayerId,
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    brush: &BrushSettings,
    world: &[Vec3],
    options: SurfaceStrokeOptions,
    memory_after_begin: Option<u64>,
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
    if memory_after_begin.is_some() {
        s.set_projection_memory(memory_after_begin);
    }
    for p in &screen[1..] {
        s.add(d, &mut stroke, *p, 1.0).unwrap();
    }
    s.finish(d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    s
}

#[test]
fn a_mirror_copy_that_first_appears_late_in_a_tight_budget_is_still_painted() {
    // 右の板（x 0.1〜1.9）と、左の板（x −1.9〜−1.0。右の x 1.0〜1.9 を写した所）。x = 0 で写す。UV は右が u 0.5〜1、左が 0〜0.25 で、
    // 右の画素 i を写した所が左の画素 511 − i
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (0.1, 1.9),
        (-1.0, 1.0),
        (Vec2::new(0.5, 0.0), Vec2::new(1.0, 1.0)),
        0,
        -1.0,
    );
    quad(
        &mut t,
        0.0,
        (-1.9, -1.0),
        (-1.0, 1.0),
        (Vec2::new(0.0, 0.0), Vec2::new(0.25, 1.0)),
        0,
        -1.0,
    );
    let g = geometry(t);
    let view = front_view(Vec3::ZERO, 8.0, 480.0);
    let brush = BrushSettings {
        radius: 16.0,
        hardness: 0.7,
        color: Rgba8::new(30, 90, 220, 255),
        ..BrushSettings::default()
    };
    // 写しの無い所（x 0.3・0.6）を広く塗ってから、写しのある所（x 1.0 より右）へ
    let path = [
        Vec3::new(0.3, -0.8, 0.0),
        Vec3::new(0.3, 0.8, 0.0),
        Vec3::new(0.6, 0.8, 0.0),
        Vec3::new(0.6, -0.8, 0.0),
        Vec3::new(1.5, -0.8, 0.0),
        Vec3::new(1.5, 0.8, 0.0),
    ];
    let options = SurfaceStrokeOptions {
        symmetry: Some(mirror_x()),
        ..SurfaceStrokeOptions::default()
    };
    let (mut d, l) = document(512, 128);
    let open = stroke_through(&mut d, l, &g, view, &brush, &path, options, None);
    let largest = open.projection_stats().largest_dab_bytes;
    let fixed = open.projection_fixed_bytes();
    let wide = layer_bytes(&d, l);
    // 写しの側にも塗れている（右の (1.5, 0) を写した (−1.5, 0)）
    assert!(alpha(&d, l, 56, 64) > 0 && alpha(&d, l, 455, 64) > 0);
    // 一覧と 3 つのダブの分だけ: 元の側は、写しが出る前に覚えを上限まで溜める。写しが出てからも、古い区画を捨てて両方を塗る
    let tight = fixed + largest * 3;
    let (mut d2, l2) = document(512, 128);
    let s = stroke_through(&mut d2, l2, &g, view, &brush, &path, options, Some(tight));
    let stats = s.projection_stats();
    assert!(stats.evictions > 0, "{stats:?}");
    assert!(
        s.projection_bytes() <= tight,
        "{} > {tight}",
        s.projection_bytes()
    );
    assert!(alpha(&d2, l2, 56, 64) > 0, "写しの側が塗られない");
    assert!(layer_bytes(&d2, l2) == wide, "覚えを捨てるとバイトが違う");
}

#[test]
fn a_dab_centred_off_the_surface_paints_its_mirror_copy_too() {
    // 板（x −1〜1。UV は全体）。x = 0 で写すと画素 i が 63 − i。板の右の縁の外に中心を置いたダブは、縁の内側を塗り、写しの側も塗る
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-1.0, 1.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        0,
        -1.0,
    );
    let g = geometry(t);
    let view = front_view(Vec3::ZERO, 5.0, 400.0);
    let brush = BrushSettings {
        radius: 6.0,
        hardness: 0.6,
        color: Rgba8::new(200, 40, 40, 255),
        ..BrushSettings::default()
    };
    let (mut d, l) = document(64, 64);
    let s = stroke_through(
        &mut d,
        l,
        &g,
        view,
        &brush,
        &[
            Vec3::new(0.85, -0.7, 0.0),
            Vec3::new(1.12, -0.6, 0.0),
            Vec3::new(1.12, 0.7, 0.0),
        ],
        SurfaceStrokeOptions {
            symmetry: Some(mirror_x()),
            ..SurfaceStrokeOptions::default()
        },
        None,
    );
    assert!(s.stats.dabs >= 5, "{:?}", s.stats);
    assert_eq!(s.stats.copies, s.stats.dabs, "{:?}", s.stats);
    assert_eq!(s.symmetry_note(), None);
    let got = painted_texels(&d, l);
    // 縁の外のダブが塗った所（上のほう）が両側にある
    assert!(got.contains(&(62, 48)) && got.contains(&(1, 48)), "{got:?}");
    let mut differ = 0;
    for &(x, y) in &got {
        let a = alpha(&d, l, x, y) as i32;
        let b = alpha(&d, l, 63 - x, y) as i32;
        if (a - b).abs() > 2 {
            differ += 1;
        }
    }
    assert!(differ <= 2, "左右が揃わない画素 {differ} / {}", got.len());
}

/// 板（マテリアル 0、x −2〜2・y −1〜1、UV は全体。128 × 64 で x = 0 で写すと画素 i が 127 − i）と、カメラ（x = 1）と板の間の
/// z = −5 で、元のカメラから左の板の下半分（y < 0）への視線だけを遮る板（マテリアル 1）。写したカメラ（x = −1）からの視線は遮らない。
fn plate_with_one_sided_blocker() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-2.0, 2.0),
        (-1.0, 1.0),
        (Vec2::ZERO, Vec2::ONE),
        0,
        -1.0,
    );
    quad(
        &mut t,
        -5.0,
        (-0.4, 0.4),
        (-2.0, 0.0),
        (Vec2::ZERO, Vec2::ONE),
        1,
        -1.0,
    );
    geometry(t)
}

/// 1 つのダブ（世界の点 at の画面の点）を押して離す。
fn one_dab_at(
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    size: (u32, u32),
    radius: f64,
    at: Vec3,
    options: SurfaceStrokeOptions,
) -> (Document, LayerId, SurfaceStroke) {
    let (mut d, l) = document(size.0, size.1);
    let brush = BrushSettings {
        radius,
        hardness: 0.8,
        color: Rgba8::new(240, 200, 20, 255),
        ..BrushSettings::default()
    };
    let s = stroke_through(&mut d, l, g, view, &brush, &[at], options, None);
    (d, l, s)
}

#[test]
fn a_mirror_copy_keeps_only_texels_the_real_camera_can_see() {
    let g = plate_with_one_sided_blocker();
    let view = front_view(Vec3::new(1.0, 0.0, 0.0), 10.0, 400.0);
    let row_y = |y: i32| (y as f32 + 0.5) / 64.0 * 2.0 - 1.0;
    let sym = SurfaceStrokeOptions {
        symmetry: Some(mirror_x()),
        ..SurfaceStrokeOptions::default()
    };
    // 中心 (1, 0): 写しの側は、元のカメラから見える上半分だけ。そこは元の側を写した画素と同じ
    let (d, l, s) = one_dab_at(&g, view, (128, 64), 6.0, Vec3::new(1.0, 0.0, 0.0), sym);
    assert_eq!(s.stats.copies, 1);
    assert_eq!(s.symmetry_note(), None);
    let got = painted_texels(&d, l);
    let main: BTreeSet<(i32, i32)> = got.iter().copied().filter(|p| p.0 >= 64).collect();
    let copy: BTreeSet<(i32, i32)> = got.iter().copied().filter(|p| p.0 < 64).collect();
    assert!(main.len() > 200, "{}", main.len());
    let (mut upper, mut differ) = (0, 0);
    for &(x, y) in &main {
        let mirrored = (127 - x, y);
        if row_y(y) > 0.05 {
            upper += 1;
            if !copy.contains(&mirrored) || alpha(&d, l, x, y) != alpha(&d, l, mirrored.0, y) {
                differ += 1;
            }
        }
    }
    assert!(upper > 80, "{upper}");
    assert!(differ <= 2, "上半分で元の側と違う {differ} / {upper}");
    for &(x, y) in &copy {
        assert!(row_y(y) > -0.05, "元のカメラから隠れた ({x}, {y}) を塗った");
    }
    // 中心 (1, −0.5): 写しの側は全部隠れる。塗らずに知らせる
    let (d, l, s) = one_dab_at(&g, view, (128, 64), 6.0, Vec3::new(1.0, -0.5, 0.0), sym);
    assert_eq!(s.symmetry_note(), Some(MirrorOutcome::Hidden));
    let got = painted_texels(&d, l);
    assert!(got.iter().any(|p| p.0 >= 64));
    assert!(got.iter().all(|p| p.0 >= 64), "隠れた写しを塗った");
    // 隠れた所も塗る: 写しの側は元の側を写した画素と同じ（写したカメラの表裏も正しい）
    let hidden = SurfaceStrokeOptions {
        projection: ProjectionSettings {
            paint_hidden: true,
            ..settings()
        },
        ..sym
    };
    let (d, l, s) = one_dab_at(&g, view, (128, 64), 6.0, Vec3::new(1.0, 0.0, 0.0), hidden);
    assert_eq!(s.symmetry_note(), None);
    let got = painted_texels(&d, l);
    let main: Vec<(i32, i32)> = got.iter().copied().filter(|p| p.0 >= 64).collect();
    let differ = main
        .iter()
        .filter(|&&(x, y)| alpha(&d, l, x, y) != alpha(&d, l, 127 - x, y))
        .count();
    assert!(main.len() > 200 && differ <= 2, "{differ} / {}", main.len());
    assert_eq!(got.len(), main.len() * 2, "写しの側の画素の数");
}

#[test]
fn a_mirror_copy_on_the_far_side_of_a_closed_shape_is_hidden_unless_hidden_areas_are_painted() {
    let g = geometry(model_triangles(&[cube_sphere(12, 0.5)]).unwrap());
    let view = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 90.0,
        pitch: 0.0,
        distance: 2.5,
        model_radius: 0.5,
        ..Default::default()
    }
    .view(320.0, 320.0);
    // カメラに向いた側（x の符号）と、その中心の点
    let side = view.position.x.signum();
    let near = Vec3::new(0.5 * side, 0.0, 0.0);
    let points = texel_points(&g, 256, 256, 0);
    let count_on = |d: &Document, l: LayerId, sign: f32| {
        points
            .iter()
            .filter(|(x, y, p, _)| p.x * sign > 0.05 && alpha(d, l, *x, *y) > 0)
            .map(|(x, y, _, _)| (*x, *y))
            .collect::<BTreeSet<_>>()
            .len()
    };
    let sym = SurfaceStrokeOptions {
        symmetry: Some(mirror_x()),
        ..SurfaceStrokeOptions::default()
    };
    // 写しの側は球の裏（元のカメラから見えない）: 塗らずに知らせる
    let (d, l, s) = one_dab_at(&g, view, (256, 256), 12.0, near, sym);
    assert_eq!(s.symmetry_note(), Some(MirrorOutcome::Hidden));
    let front = count_on(&d, l, side);
    assert!(front > 100, "{front}");
    assert_eq!(count_on(&d, l, -side), 0, "球の裏の写しを塗った");
    // 裏の面も塗る だけでは、手前の面に隠れるので塗らない
    let backfaces = SurfaceStrokeOptions {
        projection: ProjectionSettings {
            paint_backfaces: true,
            ..settings()
        },
        ..sym
    };
    let (d, l, _) = one_dab_at(&g, view, (256, 256), 12.0, near, backfaces);
    assert_eq!(count_on(&d, l, -side), 0, "手前の面に隠れた写しを塗った");
    // 隠れた所も塗る: 写しの側も、元の側と同じくらい塗る
    let hidden = SurfaceStrokeOptions {
        projection: ProjectionSettings {
            paint_hidden: true,
            ..settings()
        },
        ..sym
    };
    let (d, l, s) = one_dab_at(&g, view, (256, 256), 12.0, near, hidden);
    assert_eq!(s.symmetry_note(), None);
    let (a, b) = (count_on(&d, l, side), count_on(&d, l, -side));
    assert!(
        b * 10 >= a * 9 && b * 10 <= a * 11,
        "元の側 {a} 写しの側 {b}"
    );
}

#[test]
fn a_radial_copy_paints_the_surface_its_rotation_reaches() {
    // 軸 Z のまわりの 4 つの放射状。板 A（x 0.6〜1.4・y −0.4〜0.4、UV は左半分）と、A を 1 番目の写しの回転で回した板 B（UV は右半分）。
    // 2・3 番目の写しの所には面が無い
    let radial = RadialSymmetry::new(Vec3::ZERO, Vec3::Z, 4).unwrap();
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (0.6, 1.4),
        (-0.4, 0.4),
        (Vec2::new(0.0, 0.0), Vec2::new(0.5, 1.0)),
        0,
        -1.0,
    );
    let turned: Vec<SurfaceTriangle> = t
        .iter()
        .map(|a| {
            let mut b = *a;
            b.a = radial.rotate_point(a.a, 1);
            b.b = radial.rotate_point(a.b, 1);
            b.c = radial.rotate_point(a.c, 1);
            b.uv_a.x += 0.5;
            b.uv_b.x += 0.5;
            b.uv_c.x += 0.5;
            b
        })
        .collect();
    t.extend(turned);
    let g = geometry(t);
    // 回転の軸から外したカメラ（写しのカメラは元のカメラを回したもの）
    let view = front_view(Vec3::new(0.5, 0.5, 0.0), 6.0, 400.0);
    let (d, l, s) = one_dab_at(
        &g,
        view,
        (64, 32),
        4.0,
        Vec3::new(1.0, 0.0, 0.0),
        SurfaceStrokeOptions {
            symmetry: Some(SurfaceSymmetrySetup {
                mirror: None,
                radial: Some(radial),
                ignore_visibility: false,
            }),
            ..SurfaceStrokeOptions::default()
        },
    );
    assert_eq!(s.stats.copies, 1, "{:?}", s.stats);
    assert_eq!(s.symmetry_note(), Some(MirrorOutcome::NoSurface));
    let got = painted_texels(&d, l);
    let a: BTreeSet<(i32, i32)> = got.iter().copied().filter(|p| p.0 < 32).collect();
    let b: BTreeSet<(i32, i32)> = got
        .iter()
        .copied()
        .filter(|p| p.0 >= 32)
        .map(|(x, y)| (x - 32, y))
        .collect();
    assert!(a.len() > 60, "{}", a.len());
    // B は A を回した所（UV は A の右へずらしたもの）に、同じ形で塗られる
    let differ = a.symmetric_difference(&b).count();
    assert!(differ * 50 <= a.len(), "{differ} / {}", a.len());
}
