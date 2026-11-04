//! UDIM のテクスチャセットの画面: テクスチャセットの右クリックのメニューの「UDIM」、一覧の大きさの欄（タイルの大きさ ×タイルの数）、2D の
//! キャンバスのセルの境の線とタイルの番号（空のセルは暗く）、3D ビューで 2 つのタイルをまたいで描いた後の見た目。日本語と英語の絵。
//! 操作の中身（写し・Undo・書き出し・保存）は `headless/udim.rs`。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::{pos2, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::udim::UdimAction;
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{Vec2, Vec3};

const DOC: u32 = 128;

/// 一辺 1 の立方体。面ごとに頂点は別で、面を UDIM のタイル 1001・1002・1003・1011・1012 に置く（手前 −Z が 1001、右 +X が 1002、上 +Y が
/// 1011。左と下の面は 1012 の左右の半分）。タイルは 5 つなので、3 × 2 のシートの 1 つは空のセル。
fn udim_cube() -> ViewModel {
    let h = 0.5f32;
    let v = Vec3::new;
    // 面の 4 隅（左下・右下・左上・右上）、面のタイルの UV の原点、UV の u の範囲
    let faces: [([Vec3; 4], Vec2, [f32; 2]); 6] = [
        (
            [v(-h, -h, -h), v(h, -h, -h), v(-h, h, -h), v(h, h, -h)],
            Vec2::new(0.0, 0.0),
            [0.05, 0.95],
        ),
        (
            [v(h, -h, -h), v(h, -h, h), v(h, h, -h), v(h, h, h)],
            Vec2::new(1.0, 0.0),
            [0.05, 0.95],
        ),
        (
            [v(h, -h, h), v(-h, -h, h), v(h, h, h), v(-h, h, h)],
            Vec2::new(2.0, 0.0),
            [0.05, 0.95],
        ),
        (
            [v(-h, h, -h), v(h, h, -h), v(-h, h, h), v(h, h, h)],
            Vec2::new(0.0, 1.0),
            [0.05, 0.95],
        ),
        (
            [v(-h, -h, h), v(-h, -h, -h), v(-h, h, h), v(-h, h, -h)],
            Vec2::new(1.0, 1.0),
            [0.05, 0.45],
        ),
        (
            [v(-h, -h, h), v(h, -h, h), v(-h, -h, -h), v(h, -h, -h)],
            Vec2::new(1.0, 1.0),
            [0.55, 0.95],
        ),
    ];
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (corners, origin, [u0, u1]) in faces {
        let base = positions.len() as u32;
        for (k, (u, w)) in [(u0, 0.05), (u1, 0.05), (u0, 0.95), (u1, 0.95)]
            .into_iter()
            .enumerate()
        {
            positions.push(corners[k]);
            uvs.push(origin + Vec2::new(u, w));
        }
        indices.extend([0, 2, 1, 2, 3, 1].map(|i| base + i));
    }
    let mesh = ModelMesh {
        name: "箱".into(),
        positions,
        normals: Vec::new(),
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    };
    ViewModel::new("箱", vec![mesh], vec![Some("材".into())], 1).unwrap()
}

/// UDIM の立方体を読んで、テクスチャセットを UDIM にした画面（3D のタブを前に）。
fn udim_app(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, DOC);
    h.app_mut().set_language(lang);
    h.view3d_mut().set_model(udim_cube());
    h.view3d_mut().view.camera.yaw = -40.0;
    h.view3d_mut().view.camera.pitch = 15.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let uid = h.sets().current().uid;
    h.act(Action::Udim(UdimAction::Toggle(uid)));
    h.run();
    assert!(h.doc().udim().is_some(), "{}", h.message());
    // 見せる形（描画・当たり）の UV はシートの 0〜1
    let shown = h.view3d().model.clone().expect("見せる形");
    assert!(shown.meshes[0]
        .uvs
        .iter()
        .all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
    h
}

fn screen_of(h: &Harness<'_, YoluApp>, p: Vec3) -> Pos2 {
    let rect = h.view3d_rect().expect("3D のタブを描いた");
    let view = h.view3d().view.camera.view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 手前の面（1001）から右の面（1002）へ、縁をまたいで描く。
fn paint_across(h: &mut Harness<'_, YoluApp>) {
    h.color_mut().set_main([0.85, 0.15, 0.1, 1.0]);
    let from = screen_of(h, Vec3::new(0.1, 0.0, -0.5));
    let mid = screen_of(h, Vec3::new(0.5, 0.0, -0.5));
    let to = screen_of(h, Vec3::new(0.5, 0.0, -0.1));
    let points: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            if t < 0.5 {
                from + (mid - from) * (t * 2.0)
            } else {
                mid + (to - mid) * ((t - 0.5) * 2.0)
            }
        })
        .collect();
    drag(h, &points);
}

/// テクスチャセットの一覧（行の上下を少し含む）。
fn set_list(h: &Harness<'_, YoluApp>, lang: Lang) -> Rect {
    let row = h
        .get_by_label(lang.pick("テクスチャセット 1", "Texture Set 1"))
        .rect();
    Rect::from_min_max(
        pos2(row.left() - 40.0, row.top() - 6.0),
        pos2(row.right() + 220.0, row.bottom() + 6.0),
    )
    .intersect(h.ctx.content_rect())
}

#[test]
fn snapshot_the_set_menu_and_the_size_column() {
    for lang in Lang::JA_EN {
        let mut h = udim_app(lang);
        let suffix = lang.pick("ja", "en");
        let list = set_list(&h, lang);
        crate::common::shot::clean_region(&mut h, &format!("udim_set_list_{suffix}"), list);
        // 右クリックのメニュー（UDIM の印つき）
        let row = h
            .get_by_label(lang.pick("テクスチャセット 1", "Texture Set 1"))
            .rect()
            .center();
        press(&h, row, egui::PointerButton::Secondary);
        h.step();
        release(&h, row, egui::PointerButton::Secondary);
        h.run();
        let item = popup_item(&h, "UDIM");
        move_to(&h, item.center());
        h.run();
        let rect = crate::common::shot::popup(&h);
        crate::common::shot::region(&mut h, &format!("udim_set_menu_{suffix}"), rect);
        // 押すと UDIM をやめる（1 回の Undo で戻る）
        click(&mut h, item.center());
        h.run();
        assert!(h.doc().udim().is_none());
        h.act(Action::Undo);
        h.run();
        assert!(h.doc().udim().is_some());
    }
}

#[test]
fn snapshot_painting_across_two_tiles_in_3d_and_the_sheet_on_the_canvas() {
    let mut h = udim_app(Lang::Ja);
    paint_across(&mut h);
    assert!(h.doc().can_undo());
    // 描いた画素は 1001 と 1002 のセルに入り、ほかのセルには無い
    let layout = h.doc().udim().unwrap().clone();
    let painted_cells: std::collections::BTreeSet<usize> = {
        let doc = h.doc();
        let mut cells = std::collections::BTreeSet::new();
        for y in 0..doc.height() {
            for x in 0..doc.width() {
                if yolu_app::engine::composite_pixel(doc, x, y)[3] > 0 {
                    cells.insert(layout.cell_at(x, y).unwrap());
                }
            }
        }
        cells
    };
    let want: std::collections::BTreeSet<usize> = [1001, 1002]
        .into_iter()
        .map(|t| layout.cell_of_tile(t).unwrap())
        .collect();
    assert_eq!(painted_cells, want);
    h.event(egui::Event::PointerGone);
    h.step();
    let view = h.view3d_rect().unwrap();
    crate::common::shot::region(&mut h, "udim_view3d_painted", view);
    // 2D のキャンバス: セルの境の線と番号、空のセルは暗い
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.act(Action::FitView);
    h.run();
    let rect = canvas_rect(&h);
    crate::common::shot::clean_region(&mut h, "udim_canvas_cells", rect);
}
