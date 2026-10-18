//! 1 つのテクスチャセットが複数のマテリアルを持つとき、3D ビューはその全部の面を今のセットとして扱う: 1 本のストロークで組の面の
//! どれにも描け、組に無いマテリアルの面には描かない。スポイト・範囲のツールも組の面を今のセットの面として読む。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::Pos2;
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, Channel, LayerId, Rgba8};
use yolu_app::newproject::{DraftOp, NpAction};
use yolu_app::pathtool::PathAction;
use yolu_app::region::tools::{under, Under, Where};
use yolu_app::state::{Action, Tool};
use yolu_app::view3d::ViewId;
use yolu_app::YoluApp;
use yolu_core::geometry::{MaterialSet, OrbitCamera};
use yolu_core::glam::Vec3;
use yolu_core::paths::PathSymmetry;
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};

const SIZE: u32 = 96;

/// 3 枚の板（左から M0・M1・M2。板 i の UV は横の 3 分の 1 ずつ、i 番目の列）。
fn three_quads() -> Model {
    let materials = (0..3)
        .map(|i| MaterialInfo {
            key: MaterialKey::Material {
                name: format!("M{i}"),
                asset: None,
            },
            shader: "Standard".into(),
            textures: vec![],
            routes: vec![],
        })
        .collect();
    let meshes = (0..3)
        .map(|i| {
            let x = (i as f32 - 1.0) * 1.3;
            let (u0, u1) = (i as f32 / 3.0, (i + 1) as f32 / 3.0);
            MeshData {
                key: format!("{i}"),
                name: format!("板{i}"),
                skinned: false,
                positions: vec![
                    [x - 0.5, -0.5, 0.0],
                    [x + 0.5, -0.5, 0.0],
                    [x - 0.5, 0.5, 0.0],
                    [x + 0.5, 0.5, 0.0],
                ],
                normals: vec![],
                uv0: vec![[u0, 0.0], [u1, 0.0], [u0, 1.0], [u1, 1.0]],
                submeshes: vec![Submesh {
                    material: i,
                    indices: vec![0, 2, 1, 2, 3, 1],
                }],
            }
        })
        .collect();
    Model {
        generation: 1,
        name: "板".into(),
        materials,
        meshes,
    }
}

/// 3 枚の板を 3D に出し、M1 のマテリアルを M0 のセットへ移す（セット 0 = M0・M1、セット 1 = M2）。
fn merged() -> Harness<'static, YoluApp> {
    let mut h = app(1474.0, 700.0, SIZE);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    let (_, loaded) = h.app_mut().receive_link_model(&three_quads());
    loaded.unwrap();
    h.run();
    h.view3d_mut().view.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 6.0,
        model_radius: 1.0,
        ..Default::default()
    };
    assert_eq!(h.sets().len(), 3);
    // プロジェクト設定で M1 を M0 のセットへ移し、マテリアルの無くなった行を消す
    np(&mut h, NpAction::OpenConfigure);
    np(&mut h, NpAction::Draft(0, DraftOp::Material(1, true)));
    np(&mut h, NpAction::Draft(1, DraftOp::Remove));
    np(&mut h, NpAction::Submit);
    np(&mut h, NpAction::ConfirmApply);
    h.run();
    assert_eq!(h.sets().get(0).unwrap().bound, [0, 1]);
    assert_eq!(h.sets().get(1).unwrap().bound, [2]);
    assert_eq!(h.sets().current_index(), 0);
    assert_eq!(h.view3d().material, MaterialSet::new([0, 1]));
    h
}

fn np(h: &mut Harness<'_, YoluApp>, action: NpAction) {
    h.act(Action::Project(action));
    h.run();
}

fn view_rect(h: &Harness<'_, YoluApp>) -> egui::Rect {
    h.view3d_rect_of(ViewId::FIRST).unwrap()
}

fn screen_of(h: &Harness<'_, YoluApp>, world: Vec3) -> Pos2 {
    let area = view_rect(h);
    let view = h.view3d().view.camera.view(area.width(), area.height());
    let p = view.to_screen(world).expect("カメラの前");
    area.min + egui::vec2(p.x, p.y)
}

fn quad_center(i: usize) -> Vec3 {
    Vec3::new((i as f32 - 1.0) * 1.3, 0.0, 0.0)
}

/// 文書の UV の列 i（3 分の 1）の真ん中の行の、縁から離れた所（継ぎ目のにじみの幅の外）に、描いた画素があるか。
fn painted_column(doc: &yolu_core::Document, column: u32) -> bool {
    let y = SIZE / 2;
    let margin = 6;
    (column * SIZE / 3 + margin..(column + 1) * SIZE / 3 - margin)
        .any(|x| composite_pixel(doc, x, y)[3] > 0)
}

#[test]
fn one_stroke_paints_both_materials_of_the_set_and_not_a_material_outside_it() {
    let mut h = merged();
    h.brush_mut().radius = 4.0;
    h.color_mut().set_main([0.8, 0.1, 0.1, 1.0]);
    let points: Vec<Pos2> = (0..=40)
        .map(|k| {
            let t = k as f32 / 40.0;
            screen_of(&h, quad_center(0).lerp(quad_center(2), t))
        })
        .collect();
    drag(&mut h, &points);
    let doc = h.doc();
    assert!(painted_column(doc, 0), "M0 の面に描いた");
    assert!(painted_column(doc, 1), "同じセットの M1 の面にも描いた");
    assert!(
        !painted_column(doc, 2),
        "組に無い M2 の UV の所には描かない（M2 はほかのセット）"
    );
    let other = h.app().set_doc(1);
    assert!(
        other
            .layers()
            .iter()
            .all(|l| !yolu_app::engine::layer_has_pixels(l)),
        "ほかのセットの文書には描かない"
    );
    assert_eq!(h.doc().undo_count(), 1, "1 本のストロークは 1 回の Undo");
    // 1 回の Undo で、両方のマテリアルの面に描いた物がそろって消える
    h.act(Action::Undo);
    h.run();
    let doc = h.doc();
    assert!(!painted_column(doc, 0), "Undo で M0 の面の線が消える");
    assert!(!painted_column(doc, 1), "Undo で M1 の面の線も消える");
}

#[test]
fn the_eyedropper_and_the_region_tools_take_the_faces_of_the_set_as_the_current_set() {
    let mut h = merged();
    let rect = view_rect(&h);
    // 範囲のツール: 組の M1 の面は今のセットの三角形、M2 の面はほかのセット
    let on_m1 = screen_of(&h, quad_center(1));
    let on_m2 = screen_of(&h, quad_center(2));
    assert!(matches!(
        under(h.app_mut(), Where::Surface(rect), on_m1),
        Under::Triangle(_)
    ));
    assert!(matches!(
        under(h.app_mut(), Where::Surface(rect), on_m2),
        Under::OtherSet(_)
    ));
    // スポイト: M1 の面は今のセット（セット 0）の文書から読む
    let doc = h.app_mut().set_doc_mut(0);
    let layer = doc.layers().last().unwrap().id();
    for x in SIZE / 3..2 * SIZE / 3 {
        for y in 0..SIZE {
            doc.set_pixel(layer, x, y, Rgba8::new(10, 220, 40, 255))
                .unwrap();
        }
    }
    h.run();
    h.color_mut().set_main([0.0, 0.0, 0.0, 1.0]);
    assert!(
        yolu_app::eyedrop::pick_surface(h.app_mut(), rect, on_m1),
        "{}",
        h.message()
    );
    let main = h.color().main;
    let b = |v: f32| (v * 255.0).round() as u8;
    assert_eq!([b(main[0]), b(main[1]), b(main[2])], [10, 220, 40]);
    // 範囲のマテリアルの範囲は、組の全部のマテリアルの三角形
    let index = h.app_mut().region_index().unwrap();
    let set = h.view3d().material.clone();
    let Under::Triangle(t) = under(h.app_mut(), Where::Surface(rect), on_m1) else {
        unreachable!()
    };
    let region = index.region_in(t, yolu_core::geometry::SurfaceRegionKind::Material, &set);
    let model = h.view3d().model.clone().unwrap();
    let materials: std::collections::BTreeSet<i32> = region
        .iter()
        .map(|&t| model.geometry.triangles()[t as usize].material)
        .collect();
    assert_eq!(materials.into_iter().collect::<Vec<_>>(), [0, 1]);
}

/// レイヤー自身の Color の画素に、UV の列 `column`（3 分の 1）の縁から離れた所で描いた画素があるか。
fn layer_has_column(h: &Harness<'_, YoluApp>, layer: LayerId, column: u32) -> bool {
    let l = h.doc().layer(layer).expect("パスのレイヤー");
    let (y, margin) = (SIZE / 2, 6);
    (column * SIZE / 3 + margin..(column + 1) * SIZE / 3 - margin)
        .any(|x| l.pixel(Channel::Color, x, y).is_ok_and(|p| p.a > 0))
}

#[test]
fn a_mirrored_3d_path_reaches_the_other_material_of_the_set_when_drawn_redrawn_and_rebound() {
    let mut h = merged();
    h.act(Action::SelectTool(Tool::Path));
    h.run();
    // M1 の板（中心 x = 0）の上の 2 点。x = −0.65 の面で映すと、M0 の板（中心 x = −1.3）の上へ落ちる
    h.path_mut().next_style.symmetry = PathSymmetry::Mirror {
        point: Vec3::new(-0.65, 0.0, 0.0),
        normal: Vec3::X,
    };
    let at = screen_of(&h, Vec3::new(0.15, -0.2, 0.0));
    click(&mut h, at);
    let layer = h.selected_layer().expect("パスのレイヤー");
    // 新しいレイヤーに描く道（最初の点）
    assert!(layer_has_column(&h, layer, 1), "元の点（M1）");
    assert!(
        layer_has_column(&h, layer, 0),
        "映した先（組のもう 1 つのマテリアル M0）: {}",
        h.message()
    );
    let at = screen_of(&h, Vec3::new(0.35, 0.2, 0.0));
    click(&mut h, at);
    assert!(layer_has_column(&h, layer, 0) && layer_has_column(&h, layer, 1));
    // 一覧の描き直しの道
    h.act(Action::Path(PathAction::Reverse));
    h.run();
    assert!(layer_has_column(&h, layer, 1));
    assert!(
        layer_has_column(&h, layer, 0),
        "描き直しても M0 へ映る: {}",
        h.message()
    );
    // モデルの差し替えの描き直しの道（V を反転した新しいモデルで、モデルの指紋が替わる）
    let mut flipped = three_quads();
    flipped.generation = 2;
    for mesh in &mut flipped.meshes {
        for uv in &mut mesh.uv0 {
            uv[1] = 1.0 - uv[1];
        }
    }
    let (_, loaded) = h.app_mut().receive_link_model(&flipped);
    loaded.unwrap();
    h.run();
    assert!(layer_has_column(&h, layer, 1));
    assert!(
        layer_has_column(&h, layer, 0),
        "モデルを替えて描き直しても M0 へ映る: {}",
        h.message()
    );
}
