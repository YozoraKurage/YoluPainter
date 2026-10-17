//! 3D の塗りのアライメント（カメラ・Tangent | Wrap）: ブラシの詳細のストロークの行で選び、3D ビューの線に効く（egui_kittest）。
//! ダブの計算は core の `tests/surface/surface_tangent.rs` が見る。ここは、行・設定・入力の道がつながっていることと、同じ線をカメラと
//! Tangent | Wrap で描いた見本（3D ビューと 2D のキャンバスを並べたスクリーンショット）。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::{Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::Category;
use yolu_app::m2::UiOp;
use yolu_app::state::Action;
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{cube_sphere, SurfaceAlignment};
use yolu_core::glam::Vec3;

const SIZE: u32 = 256;

/// 既定の並び（3D ビューが左・キャンバスが右）で、試しの立方体か球を読む。
fn scene(sphere: bool, yaw: f32, pitch: f32) -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app_default(1600.0, 860.0, SIZE);
    if sphere {
        let revision = h.view3d_mut().next_revision();
        let model = ViewModel::new(
            "球",
            vec![cube_sphere(24, 0.5)],
            vec![Some("球".to_string())],
            revision,
        )
        .unwrap();
        h.view3d_mut().material = yolu_core::geometry::MaterialSet::one(0);
        h.view3d_mut().set_model(model);
    } else {
        h.view3d_mut().load_demo();
    }
    h.view3d_mut().view.camera.yaw = yaw;
    h.view3d_mut().view.camera.pitch = pitch;
    // 半分の幅の縦長のビューでも形の全体が入る遠さ
    h.view3d_mut().view.camera.distance = if sphere { 3.6 } else { 5.5 };
    h.run();
    let rect = h.view3d_rect().expect("3D のビューを描いた");
    (h, rect)
}

fn screen_of(h: &Harness<'_, YoluApp>, rect: Rect, p: Vec3) -> Pos2 {
    let view = h.view3d().view.camera.view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    egui::pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 世界の点の列を通る線（区間ごとに 6 点）。
fn paint_through(h: &mut Harness<'_, YoluApp>, rect: Rect, world: &[Vec3]) {
    let screen: Vec<Pos2> = world.iter().map(|p| screen_of(h, rect, *p)).collect();
    let mut points = vec![screen[0]];
    for pair in screen.windows(2) {
        for i in 1..=6 {
            points.push(pair[0] + (pair[1] - pair[0]) * (i as f32 / 6.0));
        }
    }
    drag(h, &points);
    h.run();
}

fn painted(h: &Harness<'_, YoluApp>) -> usize {
    let doc = h.doc();
    doc.composite(doc.bounds())
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|c| c[3] > 0)
        .count()
}

fn open_stroke_detail(h: &mut Harness<'_, YoluApp>) {
    let ui = &mut h.brushes_mut().ui;
    ui.detail.open = true;
    ui.detail.category = Category::Stroke;
    ui.detail.scroll = 0.0;
    h.run();
}

fn detail_rect(h: &Harness<'_, YoluApp>) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

/// 同じ線を、カメラと Tangent | Wrap で描いて、3D ビューとキャンバスを並べて撮る。描いた画素の数を返す（カメラ・Tangent | Wrap）。
fn compare(sphere: bool, yaw: f32, pitch: f32, line: &[Vec3], name: &str) -> (usize, usize) {
    let mut counts = Vec::new();
    for (alignment, suffix) in [
        (SurfaceAlignment::Camera, "camera"),
        (SurfaceAlignment::TangentWrap, "tangent_wrap"),
    ] {
        let (mut h, rect) = scene(sphere, yaw, pitch);
        h.view3d_mut().projection.alignment = alignment;
        h.brush_mut().radius = 14.0;
        h.brush_mut().hardness = 0.9;
        h.color_mut().set_main([0.85, 0.15, 0.1, 1.0]);
        paint_through(&mut h, rect, line);
        assert!(h.message().is_empty(), "{}", h.message());
        assert_eq!(h.doc().undo_count(), 1, "1 本の線");
        counts.push(painted(&h));
        let both = rect.union(canvas_rect(&h));
        crate::common::shot::clean_region(&mut h, &format!("{name}_{suffix}"), both);
    }
    (counts[0], counts[1])
}

/// 立方体の角をまたぐ線: カメラは斜めに見た右の面で線が太り、Tangent | Wrap は面に沿った同じ太さで続く。
#[test]
fn view3d_alignment_cube_corner() {
    let line = [
        Vec3::new(0.05, 0.15, -0.5),
        Vec3::new(0.5, 0.15, -0.5),
        Vec3::new(0.5, 0.1, -0.05),
    ];
    let (camera, tangent) = compare(false, -60.0, 15.0, &line, "view3d_alignment_cube");
    assert!(camera > 0 && tangent > 0);
    assert_ne!(camera, tangent, "アライメントで塗る形が変わる");
}

/// 球を斜めに見た線: カメラは縁の近くで画面の円を引き伸ばし、Tangent | Wrap は面の上で丸いダブのまま。
#[test]
fn view3d_alignment_sphere_slant() {
    let line = [Vec3::new(0.4, -0.15, -0.26), Vec3::new(0.4, 0.15, -0.26)];
    let (camera, tangent) = compare(true, 0.0, 0.0, &line, "view3d_alignment_sphere");
    assert!(
        camera > tangent,
        "カメラは斜めの面で広く塗る: {camera} {tangent}"
    );
}

/// ストロークの欄のアライメントの行: 3D のビューが出ているときだけ出て、Tangent | Wrap にすると、カメラから投影する塗りだけの切り替え
/// （隠れた所・面の向きの弱め）は使えない形になる（裏の面・継ぎ目のにじみは使える）。
#[test]
fn view3d_alignment_rows_follow_the_choice() {
    let (mut h, _) = scene(false, -40.0, 15.0);
    open_stroke_detail(&mut h);
    let window = detail_rect(&h);
    let node = |h: &Harness<'_, YoluApp>, label: &str| -> bool {
        h.query_all_by_label(label)
            .find(|n| window.contains(n.rect().center()))
            .unwrap_or_else(|| panic!("{label}"))
            .accesskit_node()
            .is_disabled()
    };
    assert!(!node(&h, "隠れた所も塗る"));
    // 「アライメント」の箱から選ぶ（既定はカメラ）
    h.get_by_label("アライメント: カメラ").click();
    h.run();
    let at = popup_item(&h, "Tangent | Wrap").center();
    click(&mut h, at);
    // 選んだ所の下の行にポインタを残さない（スクリーンショットにホバーを写さない）
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    assert_eq!(
        h.view3d().projection.alignment,
        SurfaceAlignment::TangentWrap
    );
    assert!(node(&h, "隠れた所も塗る"));
    assert!(node(&h, "面の向きで弱める"));
    assert!(!node(&h, "裏の面も塗る"));
    assert!(!node(&h, "継ぎ目のにじみ"));
    let rect = detail_rect(&h);
    crate::common::shot::clean_region(&mut h, "view3d_alignment_rows_ja", rect);
    // 英語
    h.act(Action::M2Ui(UiOp::Language(yolu_app::lang::Lang::En)));
    h.run();
    let rect = detail_rect(&h);
    crate::common::shot::clean_region(&mut h, "view3d_alignment_rows_en", rect);
}
