//! 3D のストロークの Tangent | Wrap のアライメント（`ProjectionSettings::alignment`）: ダブの形を面に沿った局所の展開で読むこと。
//! 立方体の角をまたいで面に沿った距離で塗る・斜めに見た球でも足跡が丸い・中心の法線と反対を向く面は「裏の面を塗る」の時だけ・
//! 予算で断るとストロークごと何も残らない・取消と Undo・並列の度合いによらない・鏡映の写し。
//! 期待は、テクセルを UV の重心座標で 3D の点へ戻した素朴な計算（平らな面の距離・立方体の辺で開いた距離・球の弦の長さ）から出す。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::BTreeSet;
use std::sync::Arc;

use yolu_core::geometry::{
    cube_sphere, demo_cube, model_triangles, world_radius, CameraView, DabRefusal, MaterialSet,
    MirrorPlane, OrbitCamera, ProjectionSettings, SurfaceAlignment, SurfaceGeometry, SurfaceStroke,
    SurfaceStrokeError, SurfaceStrokeOptions, SurfaceSymmetrySetup, SurfaceTriangle, SymmetryAxis,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::{BrushSettings, Channel, Document, LayerId, Rgba8};

fn geometry(triangles: Vec<SurfaceTriangle>) -> Arc<SurfaceGeometry> {
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
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

/// 文書のテクセルの中心の 3D の点と三角形の番号（UV の中のもの）。
fn texel_points(g: &SurfaceGeometry, width: i32, height: i32) -> Vec<(i32, i32, Vec3, usize)> {
    let mut out = Vec::new();
    for (i, t) in g.triangles().iter().enumerate() {
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

fn view(yaw: f32, pitch: f32) -> CameraView {
    OrbitCamera {
        target: Vec3::ZERO,
        yaw,
        pitch,
        distance: 3.0,
        model_radius: 1.0,
        ..Default::default()
    }
    .view(400.0, 400.0)
}

fn tangent() -> ProjectionSettings {
    ProjectionSettings {
        alignment: SurfaceAlignment::TangentWrap,
        // にじみは UV の外のテクセルを塗るので、距離の比べから外す
        seam_bleed: 0,
        ..ProjectionSettings::default()
    }
}

fn camera() -> ProjectionSettings {
    ProjectionSettings {
        seam_bleed: 0,
        ..ProjectionSettings::default()
    }
}

/// 世界の点の列を画面の点で引くストローク（確定する）。radius_world はモデルの単位のブラシの半径。
#[allow(clippy::too_many_arguments)]
fn stroke(
    d: &mut Document,
    l: LayerId,
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    radius_world: f32,
    world: &[Vec3],
    projection: ProjectionSettings,
    symmetry: Option<SurfaceSymmetrySetup>,
) -> SurfaceStroke {
    stroke_in(
        d,
        l,
        g,
        view,
        radius_world,
        world,
        projection,
        symmetry,
        &MaterialSet::one(0),
    )
}

/// `stroke` の、描くテクスチャセットのマテリアルの組を選べる形。
#[allow(clippy::too_many_arguments)]
fn stroke_in(
    d: &mut Document,
    l: LayerId,
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    radius_world: f32,
    world: &[Vec3],
    projection: ProjectionSettings,
    symmetry: Option<SurfaceSymmetrySetup>,
    material: &MaterialSet,
) -> SurfaceStroke {
    let brush = brush(g, d.width(), radius_world);
    let mut stroke = d.begin_stroke(l, &brush).unwrap();
    let screen: Vec<Vec2> = world.iter().map(|p| view.to_screen(*p).unwrap()).collect();
    let mut s = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        g.clone(),
        view,
        &brush,
        Some(material),
        screen[0],
        1.0,
        SurfaceStrokeOptions {
            projection,
            symmetry,
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    for p in &screen[1..] {
        s.add(d, &mut stroke, *p, 1.0).unwrap();
    }
    s.finish(d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    s
}

/// モデルの単位の半径が radius_world になる、硬さ 1 のブラシ（文書の画素の半径へ直す）。
fn brush(g: &SurfaceGeometry, width: u32, radius_world: f32) -> BrushSettings {
    let per_pixel = world_radius(g, 1.0, width);
    BrushSettings {
        radius: (radius_world / per_pixel) as f64,
        hardness: 1.0,
        color: Rgba8::new(200, 60, 40, 255),
        ..BrushSettings::default()
    }
}

/// 試しの立方体の、+X の面と −Z の面の距離（−Z の面の点 c から、+X の面へ x = 0.5 の辺で開いた平らな距離）。face は 6 面の番号。
fn cube_distance(face: usize, p: Vec3, c: Vec3) -> Option<f32> {
    match face {
        // −Z の面
        0 => Some((p - c).truncate().length()),
        // +X の面: 辺 x = 0.5 で −Z の面の平面へ開く（z = −0.5 からの奥行きが x の続き）
        3 => {
            let unfolded = Vec2::new(0.5 + (p.z + 0.5), p.y);
            Some((unfolded - c.truncate()).length())
        }
        _ => None,
    }
}

#[test]
fn tangent_wrap_crosses_a_cube_edge_by_the_distance_along_the_surface() {
    let g = geometry(model_triangles(&[demo_cube()]).unwrap());
    // 右前から見る: −Z の面も +X の面も見える
    let v = view(-35.0, 0.0);
    let c = Vec3::new(0.35, 0.0, -0.5);
    let r = 0.25;
    let side = &g.triangles()[6];
    assert!(side.normal().dot(Vec3::X) > 0.99, "6 番は +X の面");
    assert!(
        side.normal().dot(v.position - side.a) > 0.0,
        "+X の面が見える"
    );
    let (w, h) = (384, 256);
    let points = texel_points(&g, w, h);
    let check = |projection: ProjectionSettings| -> (usize, usize, f32) {
        let (mut d, l) = document(w as u32, h as u32);
        let s = stroke(&mut d, l, &g, v, r, &[c], projection, None);
        assert_eq!(s.stats.dabs, 1);
        let painted = painted_texels(&d, l);
        let (mut on_side, mut wrong, mut farthest) = (0, 0, 0.0f32);
        for &(x, y, p, i) in &points {
            let face = i / 2;
            let hit = painted.contains(&(x, y));
            match cube_distance(face, p, c) {
                Some(dist) => {
                    if hit {
                        farthest = farthest.max(dist / r);
                        on_side += usize::from(face == 3);
                    } else if dist < 0.95 * r {
                        wrong += 1;
                    }
                }
                None => wrong += usize::from(hit),
            }
        }
        (on_side, wrong, farthest)
    };
    let (on_side, wrong, farthest) = check(tangent());
    assert!(on_side > 200, "+X の面にも塗る: {on_side}");
    assert_eq!(
        wrong, 0,
        "面に沿った距離の内側は全部塗り、ほかの面は塗らない"
    );
    assert!(farthest < 1.02, "面に沿った距離で半径まで: {farthest}");
    // カメラ: +X の面は画面の円を投影した形（斜めに見た面では、面に沿った距離で半径より遠くまで）
    let (on_side, _, farthest) = check(camera());
    assert!(on_side > 0);
    assert!(
        farthest > 1.2,
        "カメラは +X の面を投影の形で塗る: {farthest}"
    );
}

#[test]
fn tangent_wrap_paints_a_round_footprint_on_a_sphere_seen_at_a_slant() {
    let g = geometry(model_triangles(&[cube_sphere(32, 0.5)]).unwrap());
    let v = view(0.0, 0.0);
    // 視線から 60 度ほど傾いた所
    let c = Vec3::new(0.433, 0.0, -0.25);
    let hit = yolu_core::geometry::pick(&g, &v, v.to_screen(c).unwrap()).expect("当たる");
    let center = hit.position;
    let r = 0.1;
    let (w, h) = (768, 512);
    let points = texel_points(&g, w, h);
    let measure = |projection: ProjectionSettings| -> (usize, f32) {
        let (mut d, l) = document(w as u32, h as u32);
        stroke(&mut d, l, &g, v, r, &[center], projection, None);
        let painted = painted_texels(&d, l);
        let (mut missing, mut farthest) = (0, 0.0f32);
        for &(x, y, p, _) in &points {
            let dist = (p - center).length() / r;
            if painted.contains(&(x, y)) {
                farthest = farthest.max(dist);
            } else if dist < 0.9 {
                missing += 1;
            }
        }
        (missing, farthest)
    };
    let (missing, farthest) = measure(tangent());
    assert_eq!(missing, 0, "中心から半径の内側は全部塗る");
    assert!(
        farthest < 1.03,
        "足跡は丸い（弦の長さで半径まで）: {farthest}"
    );
    let (_, farthest) = measure(camera());
    assert!(
        farthest > 1.4,
        "カメラは斜めの所で画面の円を引き伸ばす: {farthest}"
    );
    // 縁のアンチエイリアス: 帯（テクセルの数）は展開の上のテクセルの大きさで当てるので、足跡は丸いまま少しだけ外へ届く
    let (mut d, l) = document(w as u32, h as u32);
    let brush = BrushSettings {
        anti_alias: yolu_core::AntiAlias::Strong,
        ..brush(&g, w as u32, r)
    };
    let mut st = d.begin_stroke(l, &brush).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut st,
        g.clone(),
        v,
        &brush,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        v.to_screen(center).unwrap(),
        1.0,
        SurfaceStrokeOptions {
            projection: tangent(),
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    s.finish(&mut d, &mut st).unwrap();
    d.end_stroke(st).unwrap();
    let painted = painted_texels(&d, l);
    let reach = points
        .iter()
        .filter(|(x, y, _, _)| painted.contains(&(*x, *y)))
        .map(|(_, _, p, _)| (*p - center).length() / r)
        .fold(0.0f32, f32::max);
    assert!((1.0..1.15).contains(&reach), "{reach}");
}

/// ぼかし・指先も、Tangent | Wrap のダブの画素で読み元を決めて塗る（中心の面の展開の画素。投影の塗りを作らない）。
#[test]
fn blur_and_smudge_paint_through_tangent_dabs() {
    let g = geometry(model_triangles(&[demo_cube()]).unwrap());
    let v = view(-35.0, 0.0);
    let (w, h) = (192, 128);
    let (mut d, l) = document(w, h);
    let line = [Vec3::new(0.1, 0.0, -0.5), Vec3::new(0.3, 0.0, -0.5)];
    stroke(&mut d, l, &g, v, 0.12, &line, tangent(), None);
    let before = layer_bytes(&d, l);
    for (effect, kind) in [
        (
            yolu_core::BrushEffect::Blur { radius: 3 },
            yolu_core::geometry::SurfaceEffect::Blur,
        ),
        (
            yolu_core::BrushEffect::SMUDGE,
            yolu_core::geometry::SurfaceEffect::Smudge,
        ),
    ] {
        let brush = yolu_core::Brush {
            effect,
            ..yolu_core::Brush::from(brush(&g, w, 0.12))
        };
        let mut st = d.begin_brush_stroke(l, &brush).unwrap();
        let a = v.to_screen(Vec3::new(0.2, -0.15, -0.5)).unwrap();
        let b = v.to_screen(Vec3::new(0.45, 0.15, -0.5)).unwrap();
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut st,
            g.clone(),
            v,
            &brush.base,
            Some(&yolu_core::geometry::MaterialSet::one(0)),
            a,
            1.0,
            SurfaceStrokeOptions {
                effect: kind,
                projection: tangent(),
                ..SurfaceStrokeOptions::default()
            },
        )
        .unwrap();
        for i in 1..=6 {
            s.add(&mut d, &mut st, a + (b - a) * (i as f32 / 6.0), 1.0)
                .unwrap();
        }
        s.finish(&mut d, &mut st).unwrap();
        assert!(s.stats.dabs > 1, "{:?}", s.stats);
        d.end_stroke(st).unwrap();
    }
    assert!(layer_bytes(&d, l) != before, "ぼかし・指先で変わる");
}

/// Tangent | Wrap のダブの間隔は面に沿った長さ: 斜めの面へ向かう線では、画面の間隔で置くカメラより多くのダブを置く。
#[test]
fn tangent_dabs_are_spaced_along_the_surface() {
    let g = geometry(model_triangles(&[cube_sphere(24, 0.5)]).unwrap());
    let v = view(0.0, 0.0);
    // 球の縁の近くを、縁へ向かって（面と視線が 45 度から 80 度ほど。面に沿った長さは画面の長さの 2 倍ほど）
    let line: Vec<Vec3> = (0..=8)
        .map(|i| {
            let x = 0.35 + 0.0175 * i as f32;
            Vec3::new(x, 0.0, -(0.25 - x * x).sqrt())
        })
        .collect();
    let dabs = |projection: ProjectionSettings| {
        let (mut d, l) = document(256, 256);
        stroke(&mut d, l, &g, v, 0.05, &line, projection, None)
            .stats
            .dabs
    };
    let (camera, tangent) = (dabs(camera()), dabs(tangent()));
    assert!(tangent * 2 > camera * 3, "{camera} {tangent}");
}

/// 1 × 1 × 0.04 の薄い板（試しの立方体を z で潰したもの。前は −Z、裏は +Z）。
fn thin_plate() -> Arc<SurfaceGeometry> {
    let mut t = model_triangles(&[demo_cube()]).unwrap();
    for tri in &mut t {
        for p in [&mut tri.a, &mut tri.b, &mut tri.c] {
            p.z *= 0.04;
        }
    }
    geometry(t)
}

#[test]
fn faces_opposite_to_the_centre_normal_are_painted_only_with_back_faces() {
    let g = thin_plate();
    let v = view(0.0, 0.0);
    let c = Vec3::new(0.42, 0.0, -0.02);
    let (w, h) = (384, 256);
    let faces = |projection: ProjectionSettings| -> BTreeSet<usize> {
        let (mut d, l) = document(w as u32, h as u32);
        stroke(&mut d, l, &g, v, 0.15, &[c], projection, None);
        let painted = painted_texels(&d, l);
        texel_points(&g, w, h)
            .into_iter()
            .filter(|(x, y, _, _)| painted.contains(&(*x, *y)))
            .map(|(_, _, _, i)| i / 2)
            .collect()
    };
    // 前（0）と、直角の縁（+X の 3）は塗り、裏（+Z の 1）は塗らない
    assert_eq!(faces(tangent()), [0, 3].into_iter().collect());
    let back = ProjectionSettings {
        paint_backfaces: true,
        ..tangent()
    };
    assert_eq!(faces(back), [0, 1, 3].into_iter().collect());
}

#[test]
fn a_refused_tangent_dab_cancels_the_stroke_and_leaves_nothing() {
    let g = geometry(model_triangles(&[cube_sphere(16, 0.5)]).unwrap());
    let v = view(0.0, 0.0);
    let (w, h) = (256, 256);
    let brush = brush(&g, w, 0.2);
    let at = v.to_screen(Vec3::new(0.0, 0.0, -0.5)).unwrap();
    let options = SurfaceStrokeOptions {
        projection: tangent(),
        ..SurfaceStrokeOptions::default()
    };
    // 押した点のダブは塗れ、1 回の操作のメモリを下げた次のダブで断る（展開の図と候補が入らない）
    let (mut d, l) = document(w, h);
    let mut st = d.begin_stroke(l, &brush).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut st,
        g.clone(),
        v,
        &brush,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at,
        1.0,
        options,
    )
    .unwrap();
    assert_eq!(s.stats.dabs, 1);
    assert!(layer_bytes(&d, l).chunks_exact(4).any(|c| c[3] > 0));
    // 一覧（UV の覆い・アイランドの縁）は入り、ダブ 1 つ分は入らない
    s.set_projection_memory(Some(64 * 1024));
    let refused = (1..=8)
        .find_map(|i| {
            s.add(&mut d, &mut st, at + Vec2::new(6.0 * i as f32, 0.0), 1.0)
                .err()
        })
        .or_else(|| s.finish(&mut d, &mut st).err());
    assert_eq!(
        refused,
        Some(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
    );
    d.cancel_stroke(st);
    assert!(!d.has_active_stroke());
    assert!(layer_bytes(&d, l).chunks_exact(4).all(|c| c[3] == 0));
    assert_eq!(d.undo_count(), 0);
    // 最初のダブから入らない
    let mut st = d.begin_stroke(l, &brush).unwrap();
    let refused = SurfaceStroke::begin_with_options(
        &mut d,
        &mut st,
        g.clone(),
        v,
        &brush,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        at,
        1.0,
        SurfaceStrokeOptions {
            projection_memory: Some(1024),
            ..options
        },
    )
    .err();
    assert_eq!(
        refused,
        Some(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
    );
    d.cancel_stroke(st);
    assert!(layer_bytes(&d, l).chunks_exact(4).all(|c| c[3] == 0));
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn one_undo_takes_a_tangent_stroke_back() {
    let g = geometry(model_triangles(&[demo_cube()]).unwrap());
    let (mut d, l) = document(192, 128);
    let v = view(-35.0, 0.0);
    let line = [Vec3::new(0.2, 0.0, -0.5), Vec3::new(0.45, 0.05, -0.5)];
    stroke(&mut d, l, &g, v, 0.15, &line, tangent(), None);
    assert!(!painted_texels(&d, l).is_empty());
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(painted_texels(&d, l).is_empty());
}

#[test]
fn the_tangent_bytes_do_not_depend_on_threads() {
    let g = geometry(model_triangles(&[cube_sphere(24, 0.5)]).unwrap());
    let v = view(20.0, 10.0);
    // 大きなダブ（候補が並列の下限を超える）で、球の縁をまたいで引く。にじみも入れる
    let line = [
        Vec3::new(-0.3, 0.1, -0.4),
        Vec3::new(0.0, 0.15, -0.48),
        Vec3::new(0.3, 0.1, -0.4),
    ];
    let projection = ProjectionSettings {
        seam_bleed: 3,
        ..tangent()
    };
    let run = || {
        let (mut d, l) = document(1024, 1024);
        let s = stroke(&mut d, l, &g, v, 0.3, &line, projection, None);
        assert!(s.stats.dabs > 3, "{:?}", s.stats);
        layer_bytes(&d, l)
    };
    let base = run();
    assert!(base.chunks_exact(4).filter(|c| c[3] > 0).count() > 50_000);
    for threads in [1usize, 8, 32] {
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
fn a_mirror_copy_paints_the_same_round_footprint_on_the_other_side() {
    let g = geometry(model_triangles(&[cube_sphere(32, 0.5)]).unwrap());
    let v = view(0.0, 0.0);
    let hit = yolu_core::geometry::pick(&g, &v, v.to_screen(Vec3::new(0.25, 0.1, -0.42)).unwrap())
        .expect("当たる");
    let center = hit.position;
    let mirrored = Vec3::new(-center.x, center.y, center.z);
    let symmetry = SurfaceSymmetrySetup {
        mirror: Some(MirrorPlane::from_model(
            Vec3::ZERO,
            Quat::IDENTITY,
            SymmetryAxis::X,
            0.0,
        )),
        radial: None,
        ignore_visibility: false,
    };
    let (w, h) = (768, 512);
    let r = 0.1;
    let (mut d, l) = document(w as u32, h as u32);
    let s = stroke(&mut d, l, &g, v, r, &[center], tangent(), Some(symmetry));
    assert_eq!(s.stats.copies, 1);
    let painted = painted_texels(&d, l);
    let (mut right, mut left) = (0usize, 0usize);
    for (x, y, p, _) in texel_points(&g, w, h) {
        let (a, b) = ((p - center).length() / r, (p - mirrored).length() / r);
        if painted.contains(&(x, y)) {
            assert!(a.min(b) < 1.03, "どちらかの中心から半径の内側: {a} {b}");
            if p.x > 0.0 {
                right += 1;
            } else {
                left += 1;
            }
        } else {
            assert!(a.min(b) >= 0.9, "半径の内側は塗る: {a} {b}");
        }
    }
    assert!(right > 100 && left > 100, "{right} {left}");
    assert!(
        right.abs_diff(left) * 20 < right,
        "両側が同じ大きさ: {right} {left}"
    );
}

/// 試しの立方体の、−Z の面（0）をマテリアル 0・+X の面（3）をマテリアル 1・残りをマテリアル 2（どれも別のスロット）にしたもの。
fn three_materials() -> Arc<SurfaceGeometry> {
    let mut t = model_triangles(&[demo_cube()]).unwrap();
    for (i, tri) in t.iter_mut().enumerate() {
        let material = match i / 2 {
            0 => 0,
            3 => 1,
            _ => 2,
        };
        *tri = tri.with_slot(0, material, material);
    }
    geometry(t)
}

#[test]
fn tangent_wrap_paints_the_faces_of_every_material_in_the_set() {
    let g = three_materials();
    let v = view(-35.0, 0.0);
    let (w, h) = (384, 256);
    let faces = |material: &MaterialSet| -> BTreeSet<usize> {
        let (mut d, l) = document(w as u32, h as u32);
        for c in [Vec3::new(0.0, 0.0, -0.5), Vec3::new(0.5, 0.0, 0.0)] {
            stroke_in(&mut d, l, &g, v, 0.1, &[c], tangent(), None, material);
        }
        let painted = painted_texels(&d, l);
        texel_points(&g, w, h)
            .into_iter()
            .filter(|(x, y, _, _)| painted.contains(&(*x, *y)))
            .map(|(_, _, _, i)| i / 2)
            .collect()
    };
    // 組の 2 つのマテリアルの面で塗れる。組に無いマテリアルの面と、1 つだけの組に無い面は塗らない
    assert_eq!(
        faces(&MaterialSet::new([0, 1])),
        [0, 3].into_iter().collect()
    );
    assert_eq!(faces(&MaterialSet::one(0)), [0].into_iter().collect());
    assert_eq!(faces(&MaterialSet::one(1)), [3].into_iter().collect());
}
