//! UDIM のシートでの 3D の選択ペンと、3D の画面の形（グラデーション・図形）の塗り: 継ぎ目のにじみが、三角形のタイルのセルの外（隣のセル）へ出ない。
//! UDIM でない同じ大きさの文書では、同じ面のにじみが右の画素へ出る（このテストが効いているかの確かめ）。
use std::sync::Arc;

use yolu_core::geometry::{
    cover_screen, CameraView, CoverParams, MaterialSet, OrbitCamera, ProjectionSettings,
    ScreenCoverSettings, ScreenShape, SurfaceCoverStroke, SurfaceGeometry, SurfaceTriangle,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::udim::UdimLayout;
use yolu_core::Document;

/// セルの大きさ。シートは横に 2 セル（1001 と 1002）。
const S: u32 = 64;

/// −Z の側から見る板（−1〜1）。UV はシートの左のセル（1001）の右の端まで（u = 0.5 がセルの境）。
fn plate() -> Arc<SurfaceGeometry> {
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(0.3 + 0.2 * (x + 1.0) / 2.0, (y + 1.0) / 2.0 * 0.9 + 0.05);
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
    Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0)),
                tri((1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

fn camera() -> CameraView {
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

/// 2 セル分の横長の文書（UDIM なら 1001・1002）。
fn sheet(udim: bool) -> Document {
    let mut doc = Document::with_tile_size(2 * S, S, 16).unwrap();
    if udim {
        doc.set_udim_for_load(Some(UdimLayout::packed(vec![1001, 1002], S, S).unwrap()))
            .unwrap();
    }
    doc
}

/// 右のセル（x ≥ S）で量のある画素の数。
fn right_cell_pixels(amounts: impl Iterator<Item = (u32, u8)>) -> usize {
    amounts.filter(|&(x, a)| x >= S && a > 0).count()
}

#[test]
fn the_screen_shape_fill_does_not_bleed_into_the_next_cell() {
    let count = |udim: bool| {
        let doc = sheet(udim);
        let settings = ScreenCoverSettings {
            material: Some(&MaterialSet::one(0)),
            projection: ProjectionSettings::default(),
            shape: ScreenShape::View,
            positions: false,
            room: 512 << 20,
        };
        let c = cover_screen(&doc, plate(), &camera(), &settings).unwrap();
        let mut all = 0;
        let mut right = 0;
        for y in 0..S {
            for x in 0..2 * S {
                let a = c.mask().amount(x, y);
                all += usize::from(a > 0 && x < S);
                right += right_cell_pixels(std::iter::once((x, a)));
            }
        }
        assert!(all > 100, "左のセルの面は塗れている: {all}");
        right
    };
    assert!(count(false) > 0, "UDIM でなければ、にじみは右の画素へ出る");
    assert_eq!(count(true), 0, "UDIM なら隣のセルへ出ない");
}

#[test]
fn the_selection_pen_does_not_bleed_into_the_next_cell() {
    let count = |udim: bool| {
        let doc = sheet(udim);
        let params = CoverParams {
            radius: 8.0,
            hardness: 1.0,
            opacity: 1.0,
            pressure: Default::default(),
        };
        let material = MaterialSet::one(0);
        let at = camera().to_screen(Vec3::new(0.9, 0.0, 0.0)).unwrap();
        let (mut stroke, first) = SurfaceCoverStroke::begin(
            plate(),
            camera(),
            Some(&material),
            params,
            ProjectionSettings::default(),
            doc.width(),
            doc.height(),
            doc.udim_cells(),
            at,
            1.0,
            64 << 20,
        )
        .unwrap();
        let more = stroke
            .add(at + Vec2::new(0.0, 40.0), 1.0, 64 << 20)
            .unwrap();
        let all = first.iter().chain(&more).collect::<Vec<_>>();
        assert!(all.iter().any(|&&(x, _, a)| x < S as i32 && a > 0));
        right_cell_pixels(all.iter().map(|&&(x, _, a)| (x as u32, a)))
    };
    assert!(count(false) > 0, "UDIM でなければ、にじみは右の画素へ出る");
    assert_eq!(count(true), 0, "UDIM なら隣のセルへ出ない");
}
