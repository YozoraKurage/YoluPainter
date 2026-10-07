//! スポイト（I と、2D のブラシ中の Alt）: 2D・3D・レイヤーだけ・全体・描くチャンネル・マテリアルで塗るの 6 チャンネル、断り、キー・ボタン・
//! オプションバー、日英。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{Channel, LayerId, Rgba8};
use yolu_app::eyedrop::{pick_canvas, pick_surface};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::matpaint::MatAction;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{Vec2, Vec3};

fn rect() -> Rect {
    Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0))
}

/// 画面を使わない 2D の状態（文書は 64 × 64、スポイトのツール）。
fn state() -> AppState {
    let mut s = AppState::new(64, 64);
    s.tool = Tool::Eyedropper;
    s
}

/// 2D キャンバスで画素 (x, y) の中心が見える画面の点。
fn at_pixel(s: &AppState, x: u32, y: u32) -> Pos2 {
    s.view
        .view(rect(), s.doc.width(), s.doc.height())
        .to_screen(x as f64 + 0.5, y as f64 + 0.5)
}

fn pick2d(s: &mut AppState, x: u32, y: u32) -> bool {
    let view = s.view.view(rect(), s.doc.width(), s.doc.height());
    let at = at_pixel(s, x, y);
    pick_canvas(s, &view, at)
}

fn put(s: &mut AppState, id: LayerId, channel: Channel, x: u32, y: u32, px: [u8; 4]) {
    s.doc
        .set_channel_pixel(id, channel, x, y, Rgba8::new(px[0], px[1], px[2], px[3]))
        .unwrap();
}

fn main_rgb(s: &AppState) -> [u8; 3] {
    let b = |v: f32| (v * 255.0).round() as u8;
    [b(s.color.main[0]), b(s.color.main[1]), b(s.color.main[2])]
}

fn byte(v: u8) -> f32 {
    v as f32 / 255.0
}

#[test]
fn headless_picks_the_selected_layer_or_the_whole_composite() {
    let mut s = state();
    let bottom = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    put(&mut s, bottom, Channel::Color, 10, 10, [200, 0, 0, 255]);
    put(&mut s, top, Channel::Color, 10, 10, [0, 0, 200, 128]);
    let (revision, modified) = (s.doc.revision(), s.modified);
    // 選んでいるレイヤー（上）だけ。アルファは描画色に持ち込まない
    assert!(pick2d(&mut s, 10, 10), "{}", s.message);
    assert_eq!(main_rgb(&s), [0, 0, 200]);
    assert_eq!(s.color.main[3], 1.0);
    assert_eq!(s.message, "取得: R 0 G 0 B 200");
    // 下のレイヤーを選べば下の色
    s.selected_layer = Some(bottom);
    assert!(pick2d(&mut s, 10, 10));
    assert_eq!(main_rgb(&s), [200, 0, 0]);
    // 全レイヤーなら合成（下のレイヤーを選んでいても）
    s.eyedrop.all_layers = true;
    assert!(pick2d(&mut s, 10, 10));
    let composite = s.doc.composite_pixel(Channel::Color, 10, 10).unwrap();
    assert_eq!(main_rgb(&s), [composite.r, composite.g, composite.b]);
    assert_ne!(main_rgb(&s), [200, 0, 0], "上のレイヤーが混ざっている");
    assert_ne!(main_rgb(&s), [0, 0, 200]);
    // グループを選んでいれば、レイヤーだけにしていても合成
    s.eyedrop.all_layers = false;
    assert_eq!(s.doc.revision(), revision, "取るだけで文書は変わらない");
    assert_eq!(s.modified, modified);
    let group = s.doc.group_layers(&[bottom, top], "グループ").unwrap();
    s.selected_layer = Some(group);
    let revision = s.doc.revision();
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    assert!(pick2d(&mut s, 10, 10));
    assert_eq!(main_rgb(&s), [composite.r, composite.g, composite.b]);
    assert_eq!(s.doc.revision(), revision);
}

#[test]
fn headless_a_transparent_texel_changes_nothing_and_outside_the_canvas_is_ignored() {
    let mut s = state();
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 5, 5, [9, 8, 7, 255]);
    s.color.set_main([0.25, 0.5, 0.75, 1.0]);
    let before = s.color.clone();
    assert!(!pick2d(&mut s, 40, 40));
    assert_eq!(s.color, before);
    assert_eq!(s.message, "そこには何もありません（透明）。");
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert!(!pick2d(&mut s, 40, 40));
    assert_eq!(s.message, "Nothing to pick there (transparent).");
    // キャンバスの外は黙って何もしない（知らせも変えない）
    s.message = "前の知らせ".into();
    let view = s.view.view(rect(), 64, 64);
    for at in [
        view.to_screen(-3.0, 10.0),
        view.to_screen(10.0, -3.0),
        view.to_screen(64.0, 10.0),
        view.to_screen(10.0, 64.5),
    ] {
        assert!(!pick_canvas(&mut s, &view, at));
    }
    assert_eq!(s.message, "前の知らせ");
    assert_eq!(s.color, before);
    // 画素の端（最後の画素）は取れる
    assert!(!pick_canvas(&mut s, &view, view.to_screen(63.5, 63.5)));
    put(&mut s, id, Channel::Color, 63, 63, [1, 2, 3, 255]);
    assert!(pick_canvas(&mut s, &view, view.to_screen(63.5, 63.5)));
    assert_eq!(main_rgb(&s), [1, 2, 3]);
}

#[test]
fn headless_it_reads_the_channel_that_will_be_painted() {
    let mut s = state();
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 20, 20, [255, 0, 0, 255]);
    put(&mut s, id, Channel::Roughness, 20, 20, [180, 180, 180, 255]);
    put(&mut s, id, Channel::Normal, 20, 20, [200, 100, 220, 255]);
    // 描くチャンネル = Color
    assert!(pick2d(&mut s, 20, 20));
    assert_eq!(main_rgb(&s), [255, 0, 0]);
    // Roughness を描くなら、Roughness の値。描画色にして塗れば同じ値になる
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    assert!(pick2d(&mut s, 20, 20));
    assert_eq!(main_rgb(&s), [180, 180, 180]);
    // 表示のチャンネルが違っても、読むのは描くチャンネル
    s.apply(Action::M2Ui(UiOp::DisplayChannel(Channel::Color)));
    assert!(pick2d(&mut s, 20, 20));
    assert_eq!(main_rgb(&s), [180, 180, 180]);
    // Normal は R・G・B のまま描画色に
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Normal)));
    assert!(pick2d(&mut s, 20, 20));
    assert_eq!(main_rgb(&s), [200, 100, 220]);
    // そのチャンネルが透明な所では取れない（Color があっても）
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Metallic)));
    assert!(!pick2d(&mut s, 20, 20));
    assert_eq!(main_rgb(&s), [200, 100, 220]);
}

#[test]
fn headless_material_mode_reads_all_six_channels_and_keeps_what_it_cannot_read() {
    let mut s = state();
    let id = s.selected_layer.unwrap();
    for (channel, px) in [
        (Channel::Color, [10, 20, 30, 255]),
        (Channel::Roughness, [100, 100, 100, 255]),
        (Channel::Height, [60, 60, 60, 255]),
        (Channel::Normal, [200, 100, 220, 255]),
        (Channel::Emission, [5, 6, 7, 255]),
    ] {
        put(&mut s, id, channel, 12, 34, px);
    }
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Color)));
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Metallic, true)));
    s.mat.set_scalar(Channel::Metallic, 0.25);
    let set_before = s.mat.channels;
    assert!(pick2d(&mut s, 12, 34), "{}", s.message);
    assert_eq!(main_rgb(&s), [10, 20, 30]);
    assert!((s.mat.roughness - byte(100)).abs() < 1e-6);
    assert!((s.mat.height - byte(60)).abs() < 1e-6);
    assert!((s.mat.normal[0] - (byte(200) * 2.0 - 1.0)).abs() < 1e-6);
    assert!((s.mat.normal[1] - (byte(100) * 2.0 - 1.0)).abs() < 1e-6);
    assert_eq!(s.mat.emission, [byte(5), byte(6), byte(7)]);
    assert_eq!(s.mat.metallic, 0.25, "透明なチャンネルの値は変えない");
    assert_eq!(s.mat.channels, set_before, "塗るチャンネルの組は変えない");
    assert!(s.mat.enabled);
    assert_eq!(
        s.message,
        "取得: カラー・ラフネス・ハイト・ノーマル・エミッション"
    );
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert!(pick2d(&mut s, 12, 34));
    assert_eq!(
        s.message,
        "Picked Color, Roughness, Height, Normal, Emission"
    );
    // 1 つも読めない所は何も変えない
    let (mat, color) = (s.mat.clone(), s.color.clone());
    assert!(!pick2d(&mut s, 50, 50));
    assert_eq!((s.mat.clone(), s.color.clone()), (mat, color));
    // レイヤーだけ・全体: レイヤーだけのとき別のレイヤーの値は読まない
    let other = {
        s.apply(Action::NewLayer);
        s.selected_layer.unwrap()
    };
    put(
        &mut s,
        other,
        Channel::Roughness,
        12,
        34,
        [250, 250, 250, 255],
    );
    assert!(pick2d(&mut s, 12, 34));
    assert!(
        (s.mat.roughness - byte(250)).abs() < 1e-6,
        "選んだレイヤーの値"
    );
    assert_eq!(
        main_rgb(&s),
        [10, 20, 30],
        "選んだレイヤーに Color は無いので変えない"
    );
}

#[test]
fn headless_material_mode_is_ignored_while_painting_the_mask() {
    let mut s = state();
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 8, 8, [50, 60, 70, 255]);
    put(&mut s, id, Channel::Roughness, 8, 8, [120, 120, 120, 255]);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::M2(Edit::AddMask(id)));
    assert!(s.m2.edit_mask);
    let rough = s.mat.roughness;
    assert!(pick2d(&mut s, 8, 8));
    assert_eq!(
        main_rgb(&s),
        [50, 60, 70],
        "描くチャンネル（Color）を描画色に"
    );
    assert_eq!(
        s.mat.roughness, rough,
        "マスクに描くあいだはマテリアルの値を変えない"
    );
}

#[test]
fn headless_a_read_only_set_can_still_be_picked_from() {
    let mut s = state();
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 3, 3, [11, 22, 33, 255]);
    let index = s.sets.current_index();
    s.sets.get_mut(index).unwrap().read_only = Some("フィルターのあるレイヤーがあります".into());
    assert!(pick2d(&mut s, 3, 3));
    assert_eq!(main_rgb(&s), [11, 22, 33]);
}

// ───────── 3D ─────────

/// 正面の板（マテリアル 0）。UV は四隅の `uv0`〜`uv1`。
fn plate_with_uv(uv0: f32, uv1: f32) -> ViewModel {
    let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let mesh = ModelMesh {
        name: "板".into(),
        positions: vec![v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0), v(1.0, 1.0)],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(uv0, uv0),
            Vec2::new(uv1, uv0),
            Vec2::new(uv0, uv1),
            Vec2::new(uv1, uv1),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    };
    ViewModel::new("板", vec![mesh], vec![Some("材".into())], 1).expect("モデル")
}

/// 正面の板（UV は 0.1〜0.9、マテリアル 0）。
fn plate() -> ViewModel {
    plate_with_uv(0.1, 0.9)
}

fn at_model(s: &AppState, p: Vec3) -> Pos2 {
    let r = rect();
    let view = s.view3d.camera.view(r.width(), r.height());
    let q = view.to_screen(p).expect("カメラの前");
    pos2(r.left() + q.x, r.top() + q.y)
}

fn plate_state(size: u32) -> AppState {
    plate_state_of(size, plate())
}

fn plate_state_of(size: u32, model: ViewModel) -> AppState {
    let mut s = AppState::new(size, size);
    s.tool = Tool::Eyedropper;
    s.view3d.set_model(model);
    s.view3d.material = 0;
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    s
}

#[test]
fn headless_3d_reads_the_texel_under_the_surface_uv() {
    let mut s = plate_state(128);
    let id = s.selected_layer.unwrap();
    // 位置 (0.55, 0.35) の UV は (0.1 + 0.8 × 0.55, 0.1 + 0.8 × 0.35) = (0.54, 0.38) → テクセル (69, 48)
    put(&mut s, id, Channel::Color, 69, 48, [10, 200, 30, 255]);
    for (x, y) in [(68, 48), (70, 48), (69, 47), (69, 49)] {
        put(&mut s, id, Channel::Color, x, y, [1, 1, 1, 255]);
    }
    let at = at_model(&s, Vec3::new(0.55, 0.35, 0.0));
    assert!(pick_surface(&mut s, rect(), at), "{}", s.message);
    assert_eq!(main_rgb(&s), [10, 200, 30]);
    assert_eq!(s.message, "取得: R 10 G 200 B 30");
    // レイヤーだけ・全体は 2D と同じ
    let top = {
        s.apply(Action::NewLayer);
        s.selected_layer.unwrap()
    };
    put(&mut s, top, Channel::Color, 69, 48, [0, 0, 255, 255]);
    assert!(pick_surface(&mut s, rect(), at));
    assert_eq!(main_rgb(&s), [0, 0, 255]);
    s.selected_layer = Some(id);
    assert!(pick_surface(&mut s, rect(), at));
    assert_eq!(main_rgb(&s), [10, 200, 30]);
    s.eyedrop.all_layers = true;
    assert!(pick_surface(&mut s, rect(), at));
    assert_eq!(
        main_rgb(&s),
        [0, 0, 255],
        "上の不透明なレイヤーが合成の結果"
    );
    // モデルの外・板の外
    s.message.clear();
    let before = s.color.clone();
    assert!(!pick_surface(
        &mut s,
        rect(),
        pos2(rect().left() + 2.0, rect().top() + 2.0)
    ));
    assert_eq!(s.message, "ポインタの下にモデルがありません");
    assert_eq!(s.color, before);
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert!(!pick_surface(
        &mut s,
        rect(),
        pos2(rect().left() + 2.0, rect().top() + 2.0)
    ));
    assert_eq!(s.message, "Nothing of the model under the pointer");
    // 透明な所
    let empty = at_model(&s, Vec3::new(0.2, 0.8, 0.0));
    assert!(!pick_surface(&mut s, rect(), empty));
    assert_eq!(s.message, "Nothing to pick there (transparent).");
}

#[test]
fn headless_3d_material_mode_reads_the_texel_in_every_channel() {
    let mut s = plate_state(128);
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 69, 48, [10, 200, 30, 255]);
    put(&mut s, id, Channel::Metallic, 69, 48, [220, 220, 220, 255]);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    let at = at_model(&s, Vec3::new(0.55, 0.35, 0.0));
    assert!(pick_surface(&mut s, rect(), at));
    assert_eq!(main_rgb(&s), [10, 200, 30]);
    assert!((s.mat.metallic - byte(220)).abs() < 1e-6);
    assert_eq!(s.mat.roughness, 0.5, "透明なチャンネルは変えない");
}

#[test]
fn headless_3d_reads_the_first_and_last_row_and_column_at_the_uv_edges() {
    // UV が 0〜1 いっぱいの板: 端のすぐ内側の点が、最初・最後の列と行のテクセルを読む
    let mut s = plate_state_of(128, plate_with_uv(0.0, 1.0));
    let id = s.selected_layer.unwrap();
    for (x, y, rgb) in [
        (0, 64, [1, 0, 0]),
        (127, 64, [2, 0, 0]),
        (64, 0, [3, 0, 0]),
        (64, 127, [4, 0, 0]),
        (0, 0, [5, 0, 0]),
        (127, 127, [6, 0, 0]),
    ] {
        put(
            &mut s,
            id,
            Channel::Color,
            x,
            y,
            [rgb[0], rgb[1], rgb[2], 255],
        );
    }
    for (at, want, what) in [
        (Vec3::new(0.0005, 0.5, 0.0), 1, "左端の列"),
        (
            Vec3::new(0.9995, 0.5, 0.0),
            2,
            "右端の列（u が 1 に近くても幅の外へ出ない）",
        ),
        (Vec3::new(0.5, 0.0005, 0.0), 3, "最初の行"),
        (Vec3::new(0.5, 0.9995, 0.0), 4, "最後の行"),
        (Vec3::new(0.0005, 0.0005, 0.0), 5, "左上の隅"),
        (Vec3::new(0.9995, 0.9995, 0.0), 6, "右下の隅"),
    ] {
        s.color.set_main([0.0, 0.0, 0.0, 1.0]);
        let at = at_model(&s, at);
        assert!(pick_surface(&mut s, rect(), at), "{what}: {}", s.message);
        assert_eq!(main_rgb(&s)[0], want, "{what}");
    }
}

#[test]
fn headless_3d_refuses_a_surface_whose_uv_is_outside_the_texture() {
    // 板の UV が 0.5〜1.5: 右半分は 1 を超える（繰り返しの面。読まない）
    let mut s = plate_state_of(128, plate_with_uv(0.5, 1.5));
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 70, 70, [10, 20, 30, 255]);
    s.color.set_main([0.25, 0.5, 0.75, 1.0]);
    let before = s.color.clone();
    // u = v = 0.5 + 0.1 = 0.6 → テクセル (76, 76) の近く。画素は (70, 70) なので透明
    let inside = at_model(&s, Vec3::new(0.1, 0.1, 0.0));
    assert!(!pick_surface(&mut s, rect(), inside));
    assert_eq!(
        s.message, "そこには何もありません（透明）。",
        "範囲の中なら読む"
    );
    // u = 0.5 + 0.8 = 1.3: 1 を超える
    let outside = at_model(&s, Vec3::new(0.8, 0.2, 0.0));
    assert!(!pick_surface(&mut s, rect(), outside));
    assert_eq!(s.message, "この面の UV はテクスチャの外です");
    assert_eq!(s.color, before);
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert!(!pick_surface(&mut s, rect(), outside));
    assert_eq!(s.message, "This surface's UV is outside the texture");
    // v だけが 1 を超える場合も同じ
    let above = at_model(&s, Vec3::new(0.2, 0.8, 0.0));
    assert!(!pick_surface(&mut s, rect(), above));
    assert_eq!(s.message, "This surface's UV is outside the texture");
    assert_eq!(s.color, before);
}

/// 2 つのマテリアル（0 に部品 A、1 に離れた部品 B と C）の Live Link のモデル。
fn two_materials_link_model() -> yolu_protocol::Model {
    use yolu_protocol::{
        MaterialInfo, MaterialKey, MeshData, Model, Submesh as LinkSubmesh, TextureProperty,
    };
    let material = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: vec![],
    };
    let mesh = |name: &str, x: f32, material: u32| MeshData {
        key: name.into(),
        name: name.into(),
        skinned: false,
        positions: vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]],
        normals: vec![],
        uv0: vec![[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]],
        submeshes: vec![LinkSubmesh {
            material,
            indices: vec![0, 2, 1],
        }],
    };
    Model {
        generation: 1,
        name: "二つのセット".into(),
        materials: vec![material("MatA"), material("MatB")],
        meshes: vec![mesh("A", 0.0, 0), mesh("B", 3.0, 1)],
    }
}

#[test]
fn headless_3d_reads_the_composite_of_another_texture_set_without_switching() {
    let mut s = AppState::new(64, 64);
    let (_, shape) = s.receive_link_model(&two_materials_link_model());
    shape.expect("3D に読める");
    s.tool = Tool::Eyedropper;
    // 全体を表示してから向きを決める（frame_model は既定の向き＝モデルの前の +Z 側へ戻す。試しの三角形は -Z 側が表なので、正面から見る）
    s.view3d.frame_model();
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    let current = s.sets.current_index();
    assert_eq!(current, 0);
    let other = 1;
    // 部品 B（マテリアル 1 = 2 つ目のセット。文書の大きさは今のセットと違う）の UV (0.2, 0.2) のテクセル。位置は辺を 1/3 ずつ進んだ所
    let layer = s.set_doc(other).layers()[0].id();
    let (w, h) = (s.set_doc(other).width(), s.set_doc(other).height());
    assert_ne!(w, s.doc.width());
    let (x, y) = (
        (0.2 * w as f32).floor() as u32,
        (0.2 * h as f32).floor() as u32,
    );
    s.set_doc_mut(other)
        .set_channel_pixel(layer, Channel::Color, x, y, Rgba8::new(90, 80, 70, 255))
        .unwrap();
    let at = at_model(&s, Vec3::new(3.0 + 1.0 / 3.0, 1.0 / 3.0, 0.0));
    assert!(pick_surface(&mut s, rect(), at), "{}", s.message);
    assert_eq!(main_rgb(&s), [90, 80, 70]);
    assert_eq!(s.sets.current_index(), current, "今のセットは切り替えない");
    // 今のセット（マテリアル 0）の面は、今の文書から
    let own = s.selected_layer.unwrap();
    put(&mut s, own, Channel::Color, 12, 12, [1, 2, 3, 255]);
    let at = at_model(&s, Vec3::new(1.0 / 3.0, 1.0 / 3.0, 0.0));
    assert!(pick_surface(&mut s, rect(), at));
    assert_eq!(main_rgb(&s), [1, 2, 3]);
}

// ───────── 画面（キー・ボタン・オプションバー・Alt） ─────────

fn press_with(h: &Harness<'_, YoluApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
}

fn release_with(h: &Harness<'_, YoluApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
}

/// キャンバスの画素 (x, y) の中心の画面の点。
fn screen_of(h: &Harness<'_, YoluApp>, x: u32, y: u32) -> Pos2 {
    let r = canvas_rect(h);
    let s = &h.state().state;
    s.view
        .view(r, s.doc.width(), s.doc.height())
        .to_screen(x as f64 + 0.5, y as f64 + 0.5)
}

#[test]
fn the_eyedropper_has_a_button_a_key_and_a_menu_entry() {
    let mut h = app(1280.0, 800.0, 128);
    key(&h, Key::I, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Eyedropper);
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Brush);
    h.get_by_label("スポイト（I）").click();
    h.run();
    assert_eq!(h.state().state.tool, Tool::Eyedropper);
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    let _ = popup_item(&h, "スポイト");
    click(&mut h, at);
    // 英語
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    let _ = h.get_by_label("Eyedropper (I)");
    // Ctrl+Shift+I の選択範囲の反転は、I のスポイトに取られない
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    key(&h, Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Brush);
}

#[test]
fn clicking_with_the_eyedropper_picks_and_the_option_bar_chooses_the_layer_or_all() {
    let mut h = app(1280.0, 800.0, 128);
    let (bottom, top, modified) = {
        let s = &mut h.state_mut().state;
        let bottom = s.selected_layer.unwrap();
        s.apply(Action::NewLayer);
        let top = s.selected_layer.unwrap();
        put(s, bottom, Channel::Color, 40, 40, [200, 0, 0, 255]);
        put(s, top, Channel::Color, 40, 40, [0, 0, 200, 255]);
        s.selected_layer = Some(bottom);
        s.apply(Action::SelectTool(Tool::Eyedropper));
        (bottom, top, s.modified)
    };
    h.run();
    let at = screen_of(&h, 40, 40);
    click(&mut h, at);
    assert_eq!(
        main_rgb(&h.state().state),
        [200, 0, 0],
        "{}",
        h.state().state.message
    );
    assert!(!h.state().state.is_stroking());
    assert_eq!(h.state().state.modified, modified);
    // オプションバーの「全レイヤーを対象」（ツールプロパティにも同じ値が出る）
    let bar = bar_rect(&h, "全レイヤーを対象");
    click(&mut h, bar.center());
    assert!(h.state().state.eyedrop.all_layers);
    let _ = dock_rect(&h, "全レイヤーを対象");
    click(&mut h, at);
    assert_eq!(main_rgb(&h.state().state), [0, 0, 200]);
    assert_eq!(h.state().state.selected_layer, Some(bottom));
    let _ = top;
    // 英語の名前
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    let _ = bar_rect(&h, "Sample All Layers");
    let _ = dock_rect(&h, "Sample All Layers");
}

#[test]
fn alt_with_a_paint_tool_picks_instead_of_painting_in_2d() {
    let mut h = app(1280.0, 800.0, 128);
    {
        let s = &mut h.state_mut().state;
        let id = s.selected_layer.unwrap();
        put(s, id, Channel::Color, 60, 60, [30, 90, 150, 255]);
        s.color.set_main([1.0, 0.0, 0.0, 1.0]);
        s.apply(Action::SelectTool(Tool::Brush));
    }
    h.run();
    let on_pixel = screen_of(&h, 60, 60);
    let elsewhere = screen_of(&h, 20, 20);
    // Alt を押したクリック: 色を取り、描かない（文書も変更の印も変わらない）
    let revision = h.state().state.doc.revision();
    press_with(&h, on_pixel, Modifiers::ALT);
    h.step();
    release_with(&h, on_pixel, Modifiers::ALT);
    h.run();
    let s = &h.state().state;
    assert_eq!(main_rgb(s), [30, 90, 150], "{}", s.message);
    assert_eq!(s.tool, Tool::Brush, "ツールは替わらない");
    assert_eq!(s.doc.revision(), revision);
    assert!(!s.modified);
    assert!(!s.is_stroking());
    // Alt なしのクリックは描く
    drag(&mut h, &[elsewhere, offset(elsewhere, 12.0, 0.0)]);
    assert!(h.state().state.modified, "ふつうのクリックは描く");
    assert_ne!(canvas_pixel(&h, elsewhere)[3], 0);
    // バケツ・消しゴムでも
    for tool in [Tool::Fill, Tool::Eraser, Tool::PolygonFill] {
        h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
        h.state_mut().state.apply(Action::SelectTool(tool));
        h.run();
        let revision = h.state().state.doc.revision();
        press_with(&h, on_pixel, Modifiers::ALT);
        h.step();
        release_with(&h, on_pixel, Modifiers::ALT);
        h.run();
        let s = &h.state().state;
        assert_eq!(main_rgb(s), [30, 90, 150], "{tool:?}");
        assert_eq!(s.doc.revision(), revision, "{tool:?} は描かない");
    }
    // 選択のツールでは、Alt は引く（スポイトにしない）
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    h.state_mut().state.apply(Action::SelectTool(Tool::Lasso));
    h.run();
    press_with(&h, on_pixel, Modifiers::ALT);
    h.step();
    release_with(&h, on_pixel, Modifiers::ALT);
    h.run();
    assert_eq!(main_rgb(&h.state().state), [255, 0, 0]);
}

#[test]
fn headless_the_selected_layer_is_read_through_its_effects() {
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let mut s = state();
    let id = s.selected_layer.unwrap();
    put(&mut s, id, Channel::Color, 7, 7, [10, 20, 30, 255]);
    s.doc
        .add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
        )
        .unwrap();
    // レイヤーの画素そのものは (10, 20, 30)。レイヤーだけで取るのは、反転を通した出力（表示と同じ）
    assert_eq!(
        s.doc
            .layer(id)
            .unwrap()
            .pixel(Channel::Color, 7, 7)
            .unwrap(),
        Rgba8::new(10, 20, 30, 255)
    );
    assert!(pick2d(&mut s, 7, 7), "{}", s.message);
    assert_eq!(main_rgb(&s), [245, 235, 225]);
    let composite = s.doc.composite_pixel(Channel::Color, 7, 7).unwrap();
    assert_eq!(
        [composite.r, composite.g, composite.b],
        [245, 235, 225],
        "全レイヤーでも同じ"
    );
    s.eyedrop.all_layers = true;
    s.color.set_main([0.0, 0.0, 0.0, 1.0]);
    assert!(pick2d(&mut s, 7, 7));
    assert_eq!(main_rgb(&s), [245, 235, 225]);
}

// ───────── ペンの押しっぱなし（押した瞬間に終わるツール。滑らせた先を取り直さない） ─────────

fn pen_sample(at: Pos2, contact: bool) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    }
}

/// ペンの点を 1 点ずつ別のフレームで流す（押しっぱなしの間も、点ごとにフレームが進む）。
fn pen_frames(h: &mut Harness<'_, YoluApp>, steps: &[(Pos2, bool)]) {
    for (at, contact) in steps {
        h.state().pen().push(pen_sample(*at, *contact));
        h.step();
    }
    h.run();
}

/// ペンの点を 1 フレームにまとめて流す。
fn pen_one_frame(h: &mut Harness<'_, YoluApp>, steps: &[(Pos2, bool)]) {
    for (at, contact) in steps {
        h.state().pen().push(pen_sample(*at, *contact));
    }
    h.step();
    h.run();
}

const RED: [u8; 3] = [200, 10, 10];
const GREEN: [u8; 3] = [10, 200, 10];
const BLUE: [u8; 3] = [10, 10, 200];

/// 色の違う 3 か所（赤・緑・青のブロック）を持つ 2D のアプリ。画面の点（赤・緑・青の中心）を返す。
fn three_colors_app(tool: Tool) -> (Harness<'static, YoluApp>, [Pos2; 3]) {
    let mut h = app(1280.0, 800.0, 128);
    {
        let s = &mut h.state_mut().state;
        let id = s.selected_layer.unwrap();
        for ((cx, cy), rgb) in [((30, 64), RED), ((64, 64), GREEN), ((98, 64), BLUE)] {
            for y in cy - 3..=cy + 3 {
                for x in cx - 3..=cx + 3 {
                    put(s, id, Channel::Color, x, y, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        s.color.set_main([0.0, 0.0, 0.0, 1.0]);
        s.apply(Action::SelectTool(tool));
    }
    h.run();
    let points = [
        screen_of(&h, 30, 64),
        screen_of(&h, 64, 64),
        screen_of(&h, 98, 64),
    ];
    (h, points)
}

#[test]
fn a_pen_held_on_the_eyedropper_picks_once_in_the_canvas_even_when_it_slides_over_other_colors() {
    let (mut h, [a, b, c]) = three_colors_app(Tool::Eyedropper);
    // 押したまま 4 点（同じ所・別の色・別の色）。別のフレームで
    pen_frames(
        &mut h,
        &[(a, true), (a, true), (b, true), (c, true), (c, false)],
    );
    assert_eq!(
        main_rgb(&h.state().state),
        RED,
        "最初に押した所の色のまま: {}",
        h.state().state.message
    );
    assert!(!h.state().state.is_stroking());
    // 1 フレームにまとめて来ても同じ
    h.state_mut().state.color.set_main([0.0, 0.0, 0.0, 1.0]);
    pen_one_frame(&mut h, &[(b, true), (a, true), (c, true), (c, false)]);
    assert_eq!(main_rgb(&h.state().state), GREEN);
    // 離したあとに触れ直せば、また 1 回押したことになる（印は離したときに下りる）
    pen_frames(&mut h, &[(c, true), (c, true), (a, true), (a, false)]);
    assert_eq!(main_rgb(&h.state().state), BLUE);
    assert!(!h.state().state.modified, "取るだけで文書は変わらない");
}

#[test]
fn a_pen_held_with_alt_on_a_paint_tool_picks_once_instead_of_following_the_slide() {
    let (mut h, [a, b, c]) = three_colors_app(Tool::Brush);
    let revision = h.state().state.doc.revision();
    // Alt を押したまま、ペンを赤から緑・青へ滑らせる
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.step();
    pen_frames(
        &mut h,
        &[(a, true), (a, true), (b, true), (c, true), (c, false)],
    );
    let s = &h.state().state;
    assert_eq!(main_rgb(s), RED, "最初に押した所の色のまま: {}", s.message);
    assert_eq!(s.tool, Tool::Brush, "ツールは替わらない");
    assert_eq!(s.doc.revision(), revision, "描かない");
    assert!(!s.is_stroking());
    // Alt を離してペンで押せば、ふつうに描く
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    pen_frames(
        &mut h,
        &[
            (b, true),
            (offset(b, 20.0, 0.0), true),
            (offset(b, 20.0, 0.0), false),
        ],
    );
    let s = &h.state().state;
    assert_eq!(main_rgb(s), RED, "描いても色は取らない");
    assert!(s.modified, "Alt なしのペンは描く");
}

/// 3D の板に 3 色のブロック（板の左・中・右）を置いた、3D のスポイトのアプリ。
fn three_colors_3d_app() -> (Harness<'static, YoluApp>, [Pos2; 3]) {
    let mut h = app(1280.0, 800.0, 128);
    {
        let s = &mut h.state_mut().state;
        s.view3d.set_model(plate());
        s.view3d.material = 0;
        s.view3d.camera.yaw = 0.0;
        s.view3d.camera.pitch = 0.0;
        let id = s.selected_layer.unwrap();
        // 板の位置 x = 0.2・0.5・0.8 の UV（0.26・0.5・0.74）の、縦は y = 0.5（v = 0.5）のテクセルの近く
        for ((cx, cy), rgb) in [((33, 64), RED), ((64, 64), GREEN), ((94, 64), BLUE)] {
            for y in cy - 4..=cy + 4 {
                for x in cx - 4..=cx + 4 {
                    put(s, id, Channel::Color, x, y, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        s.color.set_main([0.0, 0.0, 0.0, 1.0]);
        s.apply(Action::SelectTool(Tool::Eyedropper));
    }
    h.run();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let points = {
        let s = &h.state().state;
        [0.2, 0.5, 0.8].map(|x| {
            let view = s.view3d.camera.view(rect.width(), rect.height());
            let q = view.to_screen(Vec3::new(x, 0.5, 0.0)).expect("カメラの前");
            pos2(rect.left() + q.x, rect.top() + q.y)
        })
    };
    (h, points)
}

#[test]
fn a_pen_held_on_the_eyedropper_picks_once_in_the_3d_view_even_when_it_slides_over_other_colors() {
    let (mut h, [a, b, c]) = three_colors_3d_app();
    // まず 1 回押して、3D の点が板の色を読めることを確かめる
    pen_frames(&mut h, &[(b, true), (b, false)]);
    assert_eq!(
        main_rgb(&h.state().state),
        GREEN,
        "{}",
        h.state().state.message
    );
    h.state_mut().state.color.set_main([0.0, 0.0, 0.0, 1.0]);
    pen_frames(
        &mut h,
        &[(a, true), (a, true), (b, true), (c, true), (c, false)],
    );
    assert_eq!(
        main_rgb(&h.state().state),
        RED,
        "最初に押した所の色のまま: {}",
        h.state().state.message
    );
    h.state_mut().state.color.set_main([0.0, 0.0, 0.0, 1.0]);
    pen_one_frame(&mut h, &[(c, true), (a, true), (b, true), (b, false)]);
    assert_eq!(main_rgb(&h.state().state), BLUE);
    // 離したあとに触れ直せば、また 1 回押したことになる
    pen_frames(&mut h, &[(b, true), (b, true), (a, true), (a, false)]);
    assert_eq!(main_rgb(&h.state().state), GREEN);
    assert!(!h.state().state.modified);
}
