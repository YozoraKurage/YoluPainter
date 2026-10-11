//! 投影・デカール・形のグラデーションの置き場の決め方（モデルのルートの空間。ルートの位置と向きは原点・回転なしとして、モデルの
//! 空間と同じに扱う）: モデルの外形に合わせる、当たった面にデカールを向ける。Unity 版の `ModelPlacement`・`DecalPlacement`・
//! `DecalFitPlacement` と同じ式。

use yolu_core::fill_image::{Placement, Projection, ProjectionMode, Wrap};
use yolu_core::generator::{Settings, Shape, Volume};
use yolu_core::geometry::{Bounds, CameraView, SurfaceHit};
use yolu_core::glam::{Mat3, Quat, Vec2, Vec3};

use crate::view3d::shape_gizmo::{euler_degrees, MAX_SIZE, MIN_SIZE};

/// 落とした画像の長い辺が、見えている高さ（落とした所の奥行きで）のこの割合になる。
pub const DECAL_VIEW_FRACTION: f32 = 0.3;
/// デカールの奥行き（箱の Z）の、長い辺に対する割合（面から手前と奥へ半分ずつ）。
pub const DECAL_DEPTH_FRACTION: f32 = 0.5;
/// 新しい平面（線形）のグラデーションで、高さがこの割合（一番長い辺に対して）より小さい外形は「薄い」として、高さではなく水平の長い辺に沿わせる。
pub const THIN_HEIGHT_FRACTION: f32 = 0.01;

fn size(s: f32) -> f64 {
    f64::from(s.abs() * 1.02).clamp(MIN_SIZE, MAX_SIZE)
}

fn v3(v: Vec3) -> [f64; 3] {
    [f64::from(v.x), f64::from(v.y), f64::from(v.z)]
}

/// モデルの外形に合わせた置き場: 中心は外形の中心、大きさは外形（トライプラナーと球は最も長い辺の立方体で、画像の縦横比を崩さない）。
/// デカールは `decal_fit` の仕事で、ここへ来たら箱のままにする。
pub fn fit_to_bounds(bounds: &Bounds, mode: ProjectionMode) -> Placement {
    let full = bounds.extents * 2.0;
    let (mut sx, mut sy, mut sz) = (size(full.x), size(full.y), size(full.z));
    if matches!(mode, ProjectionMode::Triplanar | ProjectionMode::Spherical) {
        let side = sx.max(sy).max(sz);
        (sx, sy, sz) = (side, side, side);
    }
    Placement {
        center: v3(bounds.center),
        rotation: [0.0; 3],
        size: [sx, sy, sz],
    }
}

/// 新しく作る塗りつぶしレイヤーの投影: 種類を `mode` にし、置き場があればそれにする（無ければ初めのまま）。デカールは画像を 1 回だけ置く
/// （繰り返さない）。デカールを置く道とメニューが画像・デカールのレイヤーを作る道が、同じ投影を作るための 1 か所。
pub fn new_projection(mode: ProjectionMode, placement: Option<Placement>) -> Projection {
    let mut projection = Projection {
        mode,
        ..Projection::default()
    };
    if mode == ProjectionMode::Decal {
        projection.wrap = Wrap::None;
    }
    if let Some(placement) = placement {
        projection.placement = placement;
    }
    projection
}

/// Unity の `Quaternion.LookRotation(forward, up)`（Z が forward、Y が up に近い向き）。
pub fn look_rotation(forward: Vec3, up: Vec3) -> Quat {
    let z = forward.normalize_or_zero();
    let z = if z == Vec3::ZERO { Vec3::Z } else { z };
    let mut x = up.cross(z).normalize_or_zero();
    if x == Vec3::ZERO {
        x = z.any_orthonormal_vector();
    }
    let y = z.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, z))
}

fn rotation_degrees(q: Quat) -> [f64; 3] {
    euler_degrees(q)
}

/// 当たった面に向けたデカールの置き場: 中心は当たった点、−Z の面が面の外を向く（+Z は法線の逆）、+Y は 3D ビューの上を面に写した向き
/// （ビューから見て画像が立つ）。長い辺は見えている高さの `DECAL_VIEW_FRACTION`、短い辺は画像の縦横比、奥行きは長い辺の
/// `DECAL_DEPTH_FRACTION`。`gui` は 3D ビューの表示域の左上からの点、`image` は画像の大きさ。
pub fn decal_at(hit: &SurfaceHit, view: &CameraView, gui: Vec2, image: (u32, u32)) -> Placement {
    let mut normal = hit.normal.normalize_or_zero();
    if normal.length_squared() < 0.5 {
        normal = Vec3::NEG_Z;
    }
    let ray = view.ray(gui);
    let above = view.ray(gui - Vec2::new(0.0, 1.0));
    // 画面の上の向きと、当たった所で見えている高さ（正投影の視線は平行なので、向きでなく始めの点の差と、見える高さそのもの）
    let (up, height) = match view.orthographic_height() {
        Some(height) => (above.origin() - ray.origin(), height),
        None => {
            let top = view.ray(Vec2::new(gui.x, 0.0));
            let bottom = view.ray(Vec2::new(gui.x, view.height));
            let distance = hit.distance.max(1e-4);
            (
                above.direction() - ray.direction(),
                (top.direction().normalize_or_zero() - bottom.direction().normalize_or_zero())
                    .length()
                    * distance,
            )
        }
    };
    let forward = -normal; // +Z は面の中へ
    let mut up = up - forward * up.dot(forward);
    if up.length_squared() < 1e-10 {
        let helper = if forward.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        up = forward.cross(helper);
        up -= forward * up.dot(forward);
    }
    let rotation = rotation_degrees(look_rotation(forward, up.normalize_or_zero()));
    let longest = f64::from(height * DECAL_VIEW_FRACTION).clamp(MIN_SIZE, MAX_SIZE);
    let (w, h) = (f64::from(image.0.max(1)), f64::from(image.1.max(1)));
    let sx = if w >= h { longest } else { longest * w / h };
    let sy = if h >= w { longest } else { longest * h / w };
    Placement {
        center: v3(hit.position),
        rotation,
        size: [
            sx.max(MIN_SIZE),
            sy.max(MIN_SIZE),
            (longest * f64::from(DECAL_DEPTH_FRACTION)).max(MIN_SIZE),
        ],
    }
}

/// デカールをモデルに合わせる: 外形の中心に、3D ビューから見て正面を向け（`forward`・`up` はカメラの向き）、外形の短い辺の半分の大きさ
/// （形の画像の縦横比）、奥行きは外形を突き抜ける長さ。
pub fn decal_fit(bounds: &Bounds, forward: Vec3, up: Vec3, image: Option<(u32, u32)>) -> Placement {
    let full = bounds.extents * 2.0;
    let up = up - forward * up.dot(forward);
    let up = if up.length_squared() < 1e-10 {
        forward.cross(if forward.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        })
    } else {
        up
    };
    let rotation = rotation_degrees(look_rotation(forward, up.normalize_or_zero()));
    let side = f64::from(full.x.min(full.y).min(full.z) * 0.5).max(MIN_SIZE);
    let depth = f64::from(full.length() * 1.02).max(MIN_SIZE);
    let (mut sx, mut sy) = (side, side);
    if let Some((w, h)) = image.filter(|(w, h)| *w > 0 && *h > 0) {
        if w >= h {
            sy = side * f64::from(h) / f64::from(w);
        } else {
            sx = side * f64::from(w) / f64::from(h);
        }
    }
    Placement {
        center: v3(bounds.center),
        rotation,
        size: [sx.max(MIN_SIZE), sy.max(MIN_SIZE), depth.min(MAX_SIZE)],
    }
}

/// 新しい形のグラデーション（形のグラデーションの Generator に、色の階調と置き換えの合成）: モデルがあれば外形の中央に、幅と奥行きは
/// モデルに合わせ、高さは半分の箱で始める（どこに効くかすぐ見えるように。やわらかさ 50 %）。
pub fn new_shape_gradient(bounds: Option<&Bounds>) -> Settings {
    use yolu_core::generator::{Blend, Kind, Ramp};
    let mut g = Settings::new(Kind::ShapeGradient);
    g.blend = Blend::Replace;
    g.ramp = Some(Ramp::default());
    if let Some(b) = bounds {
        let full = b.extents * 2.0;
        let clamp = |s: f32| f64::from(s.abs()).clamp(MIN_SIZE, MAX_SIZE);
        g.volume = Volume {
            shape: Shape::Box,
            center: v3(b.center),
            rotation: [0.0; 3],
            size: [
                clamp(full.x * 1.05),
                clamp(full.y * 0.5),
                clamp(full.z * 1.05),
            ],
            falloff: 0.5,
        };
    }
    g
}

/// 形を選んで作る新しいグラデーションデカール（新規塗りつぶしレイヤーのメニュー）。ランプと置き換えの合成は `new_shape_gradient` と
/// 同じで、置き場は形ごとにモデルの外形から決める（モデルが無ければ形の既定の置き場）:
/// ボックスは `new_shape_gradient` のまま、球は外形の中央に一番長い辺の 4 分の 3 の直径（どこに効くかすぐ見えるように）、
/// 平面（線形）は外形の中央で、下の端が 0・上の端が 1 になる幅（モデルの高さ）。高さが一番長い辺の `THIN_HEIGHT_FRACTION` 未満の外形
/// （地面のような平らなモデル）では、高さに沿うと面の値が 0.5 のまま分かれないので、平面を横へ回して、水平の長い方の辺に沿わせる
/// （長いのが X なら Z のまわりに −90°、Z なら X のまわりに 90°。平面の局所の Y がその軸を向く）。
pub fn new_shape_gradient_of(shape: Shape, bounds: Option<&Bounds>) -> Settings {
    let mut g = new_shape_gradient(bounds);
    g.volume.shape = shape;
    let Some(b) = bounds else {
        return g;
    };
    let full = b.extents * 2.0;
    let clamp = |s: f32| f64::from(s.abs()).clamp(MIN_SIZE, MAX_SIZE);
    match shape {
        Shape::Box => {}
        Shape::Sphere => {
            let diameter = clamp(full.x.abs().max(full.y.abs()).max(full.z.abs()) * 0.75);
            g.volume.size = [diameter; 3];
        }
        Shape::Plane => {
            let longest = full.x.abs().max(full.y.abs()).max(full.z.abs());
            if full.y.abs() < longest * THIN_HEIGHT_FRACTION {
                if full.x.abs() >= full.z.abs() {
                    // 局所の Y = ワールドの X。局所の X・Z は、ワールドの Y（薄い）ではなく短い水平の辺で見せる
                    g.volume.rotation = [0.0, 0.0, -90.0];
                    let lateral = clamp(full.z * 1.05);
                    g.volume.size = [lateral, clamp(full.x), lateral];
                } else {
                    // 局所の Y = ワールドの Z
                    g.volume.rotation = [90.0, 0.0, 0.0];
                    let lateral = clamp(full.x * 1.05);
                    g.volume.size = [lateral, clamp(full.z), lateral];
                }
            } else {
                g.volume.size = [clamp(full.x * 1.05), clamp(full.y), clamp(full.z * 1.05)];
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::OrbitCamera;

    fn bounds() -> Bounds {
        Bounds::new(Vec3::new(0.5, 1.0, 0.0), Vec3::new(2.0, 4.0, 1.0))
    }

    #[test]
    fn a_model_fit_follows_the_bounds_and_cubes_the_projections_that_need_it() {
        let b = bounds();
        let p = fit_to_bounds(&b, ProjectionMode::Planar);
        assert_eq!(p.center, [0.5, 1.0, 0.0]);
        assert!(
            (p.size[0] - 2.04).abs() < 1e-5
                && (p.size[1] - 4.08).abs() < 1e-5
                && (p.size[2] - 1.02).abs() < 1e-5
        );
        for mode in [ProjectionMode::Triplanar, ProjectionMode::Spherical] {
            let c = fit_to_bounds(&b, mode);
            assert_eq!(c.size[0], c.size[1]);
            assert_eq!(c.size[1], c.size[2]);
            assert!((c.size[0] - 4.08).abs() < 1e-5, "最も長い辺の立方体");
        }
        let cylinder = fit_to_bounds(&b, ProjectionMode::Cylindrical);
        assert_eq!(cylinder.size[0], p.size[0]);
    }

    #[test]
    fn look_rotation_matches_unity() {
        // Unity: LookRotation(forward = +X, up = +Y) は Y のまわりに 90°
        let q = look_rotation(Vec3::X, Vec3::Y);
        assert!((q * Vec3::Z - Vec3::X).length() < 1e-5);
        assert!((q * Vec3::Y - Vec3::Y).length() < 1e-5);
        // 平行な up でも壊れない
        let q = look_rotation(Vec3::Y, Vec3::Y);
        assert!((q * Vec3::Z - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn a_decal_faces_the_surface_with_the_images_aspect() {
        let cam = OrbitCamera::default();
        let view = cam.view(800.0, 600.0);
        let gui = Vec2::new(400.0, 300.0);
        let ray = view.ray(gui);
        // 正面の面（法線はカメラへ向く）に当たった
        let hit = SurfaceHit {
            revision: 0,
            renderer: 0,
            material_slot: 0,
            material: 0,
            triangle: 0,
            position: cam.target,
            normal: -view.forward,
            barycentric: Vec3::ZERO,
            uv: Vec2::ZERO,
            distance: (cam.target - ray.origin()).length(),
        };
        let p = decal_at(&hit, &view, gui, (200, 100));
        assert_eq!(
            p.center,
            [
                f64::from(cam.target.x),
                f64::from(cam.target.y),
                f64::from(cam.target.z)
            ]
        );
        // 箱の +Z は面の中（法線の逆）を向く
        let q = {
            let r = p.rotation;
            let s = crate::view3d::shape_gizmo::Shape {
                kind: crate::view3d::shape_gizmo::Kind::Box,
                center: [0.0; 3],
                rotation: r,
                size: [1.0; 3],
                falloff: 0.0,
            };
            crate::view3d::shape_gizmo::rotation_of(&s)
        };
        assert!(
            (q * Vec3::Z - view.forward).length() < 1e-3,
            "{:?}",
            q * Vec3::Z
        );
        // ビューの上が箱の上
        let up = view.rotation * Vec3::Y;
        assert!((q * Vec3::Y).dot(up) > 0.99);
        // 画像の縦横比（2:1）と、奥行きは長い辺の半分
        assert!((p.size[0] / p.size[1] - 2.0).abs() < 1e-6, "{:?}", p.size);
        assert!((p.size[2] - p.size[0] * 0.5).abs() < 1e-9);
        // 縦長の画像は縦が長い辺
        let tall = decal_at(&hit, &view, gui, (100, 400));
        assert!(tall.size[1] > tall.size[0] && (tall.size[1] / tall.size[0] - 4.0).abs() < 1e-6);
        // 面の向きが壊れていても置ける
        let bad = SurfaceHit {
            normal: Vec3::ZERO,
            ..hit
        };
        let p = decal_at(&bad, &view, gui, (10, 10));
        assert!(p.size.iter().all(|s| *s >= MIN_SIZE && s.is_finite()));
    }

    #[test]
    fn a_decal_fit_faces_the_camera_and_goes_through_the_model() {
        let cam = OrbitCamera::default();
        let view = cam.view(800.0, 600.0);
        let b = bounds();
        let p = decal_fit(&b, view.forward, view.rotation * Vec3::Y, Some((100, 50)));
        assert_eq!(p.center, [0.5, 1.0, 0.0]);
        // 外形の最も短い辺（1.0）の半分が長い辺、縦横比 2:1
        assert!(
            (p.size[0] - 0.5).abs() < 1e-6 && (p.size[1] - 0.25).abs() < 1e-6,
            "{:?}",
            p.size
        );
        assert!(p.size[2] > (b.extents * 2.0).length() as f64);
        let none = decal_fit(&b, view.forward, view.rotation * Vec3::Y, None);
        assert_eq!(none.size[0], none.size[1]);
    }

    #[test]
    fn a_new_shape_gradient_starts_as_half_the_model_with_a_ramp() {
        let g = new_shape_gradient(Some(&bounds()));
        assert_eq!(g.kind, yolu_core::generator::Kind::ShapeGradient);
        assert!(g.ramp.is_some());
        assert_eq!(g.blend, yolu_core::generator::Blend::Replace);
        assert!((g.volume.size[1] - 2.0).abs() < 1e-6, "高さは外形の半分");
        assert!((g.volume.size[0] - 2.1).abs() < 1e-5);
        assert_eq!(g.volume.falloff, 0.5);
        assert!(g.validate().is_ok());
        let bare = new_shape_gradient(None);
        assert_eq!(bare.volume, Volume::default());
        assert!(bare.validate().is_ok());
    }

    #[test]
    fn a_new_gradient_of_each_shape_fits_the_model_and_is_valid() {
        let b = bounds();
        // ボックスは今までと同じ
        assert_eq!(
            new_shape_gradient_of(Shape::Box, Some(&b)),
            new_shape_gradient(Some(&b))
        );
        // 球: 一番長い辺（高さ 4）の 4 分の 3 を直径に、外形の中央へ
        let sphere = new_shape_gradient_of(Shape::Sphere, Some(&b));
        assert_eq!(sphere.volume.shape, Shape::Sphere);
        assert_eq!(sphere.volume.center, [0.5, 1.0, 0.0]);
        assert!(sphere.volume.size.iter().all(|s| (s - 3.0).abs() < 1e-5));
        // 平面: 幅は外形の高さ（4）。下の端（-2）が 0・上の端（+2）が 1
        let plane = new_shape_gradient_of(Shape::Plane, Some(&b));
        assert_eq!(plane.volume.shape, Shape::Plane);
        assert!((plane.volume.size[1] - 4.0).abs() < 1e-5);
        let at = |y: f64| plane.volume.value_at([0.5, 1.0 + y, 0.0]).unwrap();
        assert!(
            at(-2.0).abs() < 1e-9 && (at(2.0) - 1.0).abs() < 1e-9 && (at(0.0) - 0.5).abs() < 1e-9
        );
        assert_eq!(
            plane.volume.rotation, [0.0; 3],
            "ふつうの高さの外形は回さない"
        );
        for g in [&sphere, &plane] {
            assert!(g.ramp.is_some());
            assert_eq!(g.blend, yolu_core::generator::Blend::Replace);
            assert!(g.validate().is_ok());
        }
        // モデルが無ければ、形だけ替えた既定の置き場
        for shape in [Shape::Box, Shape::Sphere, Shape::Plane] {
            let bare = new_shape_gradient_of(shape, None);
            assert_eq!(
                bare.volume,
                Volume {
                    shape,
                    ..Volume::default()
                }
            );
            assert!(bare.validate().is_ok());
        }
    }

    /// 面の上（外形の中心の高さ）の、中心から (dx, dz) の点の値。
    fn on_surface(g: &Settings, b: &Bounds, dx: f64, dz: f64) -> f64 {
        let c = b.center;
        g.volume
            .value_at([f64::from(c.x) + dx, f64::from(c.y), f64::from(c.z) + dz])
            .unwrap()
    }

    #[test]
    fn a_new_plane_gradient_on_a_flat_model_runs_along_its_longest_horizontal_side() {
        // 地面のような高さ 0 のモデル。X が長い（4）と Z が長い（6）の両方
        for (size, along_x) in [
            (Vec3::new(4.0, 0.0, 2.0), true),
            (Vec3::new(2.0, 0.0, 6.0), false),
        ] {
            let b = Bounds::new(Vec3::new(0.5, 1.0, 0.0), size);
            let plane = new_shape_gradient_of(Shape::Plane, Some(&b));
            assert!(plane.validate().is_ok(), "{size:?}");
            assert_eq!(plane.volume.shape, Shape::Plane);
            assert_eq!(plane.volume.center, [0.5, 1.0, 0.0]);
            let long = f64::from(size.x.max(size.z));
            assert!(
                (plane.volume.size[1] - long).abs() < 1e-5,
                "{size:?}: {:?}",
                plane.volume.size
            );
            let at = |t: f64| {
                if along_x {
                    on_surface(&plane, &b, t * long / 2.0, 0.0)
                } else {
                    on_surface(&plane, &b, 0.0, t * long / 2.0)
                }
            };
            // 面の値が 0.5 の一定ではなく、片端が 0・もう片端が 1 に分かれる
            assert!(at(-1.0).abs() < 1e-9, "{size:?}: {}", at(-1.0));
            assert!((at(1.0) - 1.0).abs() < 1e-9, "{size:?}: {}", at(1.0));
            assert!((at(0.0) - 0.5).abs() < 1e-9);
            assert!(
                at(-0.5) > at(-1.0) && at(0.5) > at(0.0),
                "途中の値は段階的に"
            );
            // 平面の欄で見せる大きさは、薄いワールドの Y ではなく短い水平の辺
            assert!(
                plane.volume.size[0] > 1.0 && plane.volume.size[2] > 1.0,
                "{:?}",
                plane.volume.size
            );
        }
    }

    #[test]
    fn the_plane_turns_only_when_the_height_is_thin_against_the_longest_side() {
        // 一番長い辺 4、高さが 1 % 未満（0.039）と 1 % 以上（0.041）
        let thin = Bounds::new(Vec3::ZERO, Vec3::new(4.0, 0.039, 2.0));
        let thick = Bounds::new(Vec3::ZERO, Vec3::new(4.0, 0.041, 2.0));
        let p = new_shape_gradient_of(Shape::Plane, Some(&thin));
        assert_eq!(p.volume.rotation, [0.0, 0.0, -90.0]);
        assert!((p.volume.size[1] - 4.0).abs() < 1e-5);
        let p = new_shape_gradient_of(Shape::Plane, Some(&thick));
        assert_eq!(p.volume.rotation, [0.0; 3]);
        assert!(
            (p.volume.size[1] - 0.041).abs() < 1e-5,
            "高さに沿う: {:?}",
            p.volume.size
        );
        // ごく薄いが 0 ではない外形でも、面の上の値は分かれる
        let sheet = Bounds::new(Vec3::ZERO, Vec3::new(4.0, 1e-4, 4.0));
        let p = new_shape_gradient_of(Shape::Plane, Some(&sheet));
        assert!(
            on_surface(&p, &sheet, -2.0, 0.0) < 0.01 && on_surface(&p, &sheet, 2.0, 0.0) > 0.99
        );
        // 一番長い辺が高さのとき（背の高い外形）は今までどおり回さない
        let tall = Bounds::new(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0));
        let p = new_shape_gradient_of(Shape::Plane, Some(&tall));
        assert_eq!(p.volume.rotation, [0.0; 3]);
        assert!((p.volume.size[1] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn a_flat_or_point_model_gets_valid_gradients_of_every_shape() {
        let flat = Bounds::new(Vec3::new(0.0, 2.0, 0.0), Vec3::new(4.0, 0.0, 4.0));
        let point = Bounds::new(Vec3::ONE, Vec3::ZERO);
        for b in [&flat, &point] {
            for shape in [Shape::Box, Shape::Sphere, Shape::Plane] {
                let g = new_shape_gradient_of(shape, Some(b));
                assert!(g.validate().is_ok(), "{shape:?} {b:?}");
                assert!(g.volume.value_at(v3(b.center)).is_ok());
            }
        }
        // 球は平らな外形でも一番長い辺から直径を決めるので、面の上で 1（中心）から 0（縁）へ分かれる
        let sphere = new_shape_gradient_of(Shape::Sphere, Some(&flat));
        assert!((sphere.volume.size[0] - 3.0).abs() < 1e-5);
        assert!(on_surface(&sphere, &flat, 0.0, 0.0) > 0.99);
        assert!(on_surface(&sphere, &flat, 2.0, 0.0) < 1e-9);
    }
}
