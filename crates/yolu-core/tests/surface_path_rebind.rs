//! 面のパスの付け直し（Unity 版の SurfacePathRebindTests。値は C# の試験の期待値そのもの）: 同じ位置の面へ置き直し（三角形の割り方・UV・
//! 並びが違っても）、許す距離の外・別のマテリアルの組の面・向きが逆の面には置かない、前のモデルに結び付いていないパスは付け直さない。
#![allow(clippy::chunks_exact_to_as_chunks)]

use yolu_core::geometry::{Ray, SurfaceGeometry, SurfaceTriangle};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, point_of, point_position, rebind_surface_path, render_surface, Options, PathBrush,
    PathPoint, RebindError, SurfacePath,
};
use yolu_core::{BrushSettings, Channel, Rgba8};

/// z = 0 の 1 × 1 の板（法線 +z）を n × n に割ったもの。UV は (u0, v0) から s 倍。material はマテリアルの組。
#[allow(clippy::too_many_arguments)]
fn board(
    n: i32,
    u0: f32,
    v0: f32,
    s: f32,
    slot: i32,
    material: i32,
    z: f32,
    flip: bool,
) -> Vec<SurfaceTriangle> {
    let mut list = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let (x0, x1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            let (y0, y1) = (j as f32 / n as f32, (j + 1) as f32 / n as f32);
            let p = |x: f32, y: f32| Vec3::new(x, y, z);
            let u = |x: f32, y: f32| Vec2::new(u0 + x * s, v0 + y * s);
            let t = |a, b, c, ua, ub, uc| {
                SurfaceTriangle::new(a, b, c, ua, ub, uc).with_slot(0, slot, material)
            };
            if !flip {
                list.push(t(
                    p(x0, y0),
                    p(x1, y0),
                    p(x1, y1),
                    u(x0, y0),
                    u(x1, y0),
                    u(x1, y1),
                ));
                list.push(t(
                    p(x0, y0),
                    p(x1, y1),
                    p(x0, y1),
                    u(x0, y0),
                    u(x1, y1),
                    u(x0, y1),
                ));
            } else {
                list.push(t(
                    p(x0, y0),
                    p(x1, y1),
                    p(x1, y0),
                    u(x0, y0),
                    u(x1, y1),
                    u(x1, y0),
                ));
                list.push(t(
                    p(x0, y0),
                    p(x0, y1),
                    p(x1, y1),
                    u(x0, y0),
                    u(x0, y1),
                    u(x1, y1),
                ));
            }
        }
    }
    list
}
fn plain(n: i32) -> Vec<SurfaceTriangle> {
    board(n, 0.0, 0.0, 1.0, 0, -1, 0.0, false)
}
fn geometry(t: Vec<SurfaceTriangle>) -> SurfaceGeometry {
    SurfaceGeometry::new(t, 1, 0.000_001).unwrap()
}
fn at(g: &SurfaceGeometry, x: f32, y: f32) -> PathPoint {
    let hit = g
        .raycast(
            Ray::new(Vec3::new(x, y, 1.0), Vec3::NEG_Z),
            true,
            f32::INFINITY,
        )
        .expect("当たる");
    point_of(&hit, 0.7).unwrap()
}
fn path_on(g: &SurfaceGeometry) -> SurfacePath {
    SurfacePath {
        id: 0x1234_5678_9abc_def0,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 0.005,
            color: Rgba8::new(1, 2, 3, 255),
            ..BrushSettings::default()
        }),
        points: vec![at(g, 0.2, 0.3), at(g, 0.7, 0.6)],
        model_fingerprint: fingerprint(g),
        material: None,
    }
}

#[test]
fn a_path_moves_to_the_same_place_on_another_mesh() {
    let from = geometry(plain(1));
    let to = geometry(board(4, 0.5, 0.5, 0.5, 0, -1, 0.0, false)); // 割り方・UV・三角形の数が違う
    let path = path_on(&from);
    let rebound = rebind_surface_path(&path, &from, &to, 0).unwrap();
    assert_eq!(rebound.id, path.id);
    assert_eq!(rebound.brush, path.brush);
    assert_eq!(rebound.channel, path.channel);
    assert_eq!(rebound.model_fingerprint, fingerprint(&to));
    assert_eq!(
        rebound
            .points
            .iter()
            .map(|p| p.pressure)
            .collect::<Vec<_>>(),
        path.points.iter().map(|p| p.pressure).collect::<Vec<_>>(),
        "筆圧はそのまま"
    );
    for (new, old) in rebound.points.iter().zip(&path.points) {
        let a = point_position(&to, new).unwrap().0;
        let b = point_position(&from, old).unwrap().0;
        assert!(a.distance(b) < 1e-5);
    }
    // 描くと新しい UV の所に描ける
    let options = Options {
        width: 64,
        height: 64,
        tile_size: 16,
        ..Options::default()
    };
    let drawn = render_surface(&rebound, &to, &options).unwrap();
    assert_eq!(drawn.gaps, 0);
    let (_, surface) = &drawn.channels[0];
    assert!(
        surface.tile_coords().iter().all(|c| c.x >= 2 && c.y >= 2),
        "新しい UV の正方形（右上の 4 分の 1）だけ"
    );
    assert!(!surface.tile_coords().is_empty());
}

#[test]
fn a_point_without_a_near_surface_of_its_material_is_not_placed() {
    let from = geometry(plain(1));
    let path = path_on(&from);
    let far = geometry(board(1, 0.0, 0.0, 1.0, 0, -1, 0.5, false));
    assert!(matches!(
        rebind_surface_path(&path, &from, &far, 0),
        Err(RebindError::NoSurface { point: 0, .. })
    ));
    let near = geometry(board(1, 0.0, 0.0, 1.0, 0, -1, 0.001, false));
    assert!(
        rebind_surface_path(&path, &from, &near, 0).is_ok(),
        "許す距離（対角線の 1%）より少し近い"
    );
    // 同じ所に別のマテリアルの組の板と、自分の組の裏向きの板: どちらにも置かない
    let mut both = board(1, 0.0, 0.0, 1.0, 1, 1, 0.0, false);
    both.extend(board(1, 0.0, 0.0, 1.0, 0, 0, 0.0, true));
    let other = geometry(both);
    assert!(
        rebind_surface_path(&path, &from, &other, 0).is_err(),
        "向きの合うのは別の組の板で、自分の組の板は裏向き"
    );
    let on_other = rebind_surface_path(&path, &from, &other, 1).unwrap();
    assert!(on_other
        .points
        .iter()
        .all(|p| other.triangles()[p.triangle as usize].material == 1));
    assert_eq!(
        rebind_surface_path(&path, &from, &other, -1).err(),
        Some(RebindError::NoMaterial)
    );
    let stranger = path_on(&geometry(plain(2)));
    assert_eq!(
        rebind_surface_path(&stranger, &from, &other, 1).err(),
        Some(RebindError::OtherModel)
    );
}

#[test]
fn the_reason_names_the_point_and_the_distance() {
    let from = geometry(plain(1));
    let path = path_on(&from);
    let far = geometry(board(1, 0.0, 0.0, 1.0, 0, -1, 0.5, false));
    let e = rebind_surface_path(&path, &from, &far, 0).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("点 1"), "{text}");
    // 点の指す三角形がモデルに無いとき（前のモデルの指紋は合っていても、点が壊れている）
    let mut broken = path.clone();
    broken.points[1].triangle = 99;
    let near = geometry(board(1, 0.0, 0.0, 1.0, 0, -1, 0.001, false));
    assert_eq!(
        rebind_surface_path(&broken, &from, &near, 0).err(),
        Some(RebindError::MissingTriangle { point: 1 })
    );
}

#[test]
fn a_hit_on_a_triangle_edge_becomes_a_valid_control_point() {
    // 重心座標の丸めで u + v が 1 をわずかに超えても、制御点にできる
    let g = geometry(plain(1));
    let hit = g
        .raycast(
            Ray::new(Vec3::new(1.0, 0.0, 1.0), Vec3::NEG_Z),
            true,
            f32::INFINITY,
        )
        .expect("角に当たる");
    let p = point_of(&hit, 1.0).unwrap();
    assert!(p.u + p.v <= 1.0 + 1e-9);
}
