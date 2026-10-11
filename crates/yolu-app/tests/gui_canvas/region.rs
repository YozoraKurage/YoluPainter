//! マテリアルで塗る・範囲のツール（バケツ・ポリゴン塗りつぶし・ID の色で選択）・手動の ID の色の画面の振る舞いと見た目。
//! どれも「操作 → 文書が変わる → Undo で戻る」と、日本語と英語、断られた理由。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{pos2, vec2, Key, Modifiers, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::Arc;
use yolu_app::bake::BakeAction;
use yolu_app::engine::{composite_pixel, Channel, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::matpaint::MatAction;
use yolu_app::region::tools::{begin_polygon, bucket, drag_to, finish_drag, update_hover, Where};
use yolu_app::region::{IdColorOp, RegionAction};
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, Submesh, SurfaceRegionKind};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::mesh_maps::{MeshIdSource, MeshMapKind};
use yolu_core::SelectionMask;

// ───────── 試験用のモデルと座標 ─────────

/// 部品 A（UV の継ぎ目で左右の 2 つのアイランドに分かれた板。位置は継ぎ目でつながる）と、離れた部品 B。マテリアルは 1 つ。
/// 三角形: A の左 0・1、A の右 2・3、B 4・5。UV は 0〜1 の中で、左 0.05〜0.25、右 0.30〜0.45、B 0.55〜0.95（縦は 0.05〜0.45）。
fn two_parts_model() -> ViewModel {
    let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(x, y);
    let a = ModelMesh {
        name: "A".into(),
        positions: vec![
            v(0.0, 0.0),
            v(0.5, 0.0),
            v(0.0, 1.0),
            v(0.5, 1.0),
            v(0.5, 0.0),
            v(1.0, 0.0),
            v(0.5, 1.0),
            v(1.0, 1.0),
        ],
        normals: Vec::new(),
        uvs: vec![
            uv(0.05, 0.05),
            uv(0.25, 0.05),
            uv(0.05, 0.45),
            uv(0.25, 0.45),
            uv(0.30, 0.05),
            uv(0.45, 0.05),
            uv(0.30, 0.45),
            uv(0.45, 0.45),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1, 4, 6, 5, 6, 7, 5],
        }],
    };
    let b = ModelMesh {
        name: "B".into(),
        positions: vec![v(3.0, 0.0), v(4.0, 0.0), v(3.0, 1.0), v(4.0, 1.0)],
        normals: Vec::new(),
        uvs: vec![
            uv(0.55, 0.05),
            uv(0.95, 0.05),
            uv(0.55, 0.45),
            uv(0.95, 0.45),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    };
    ViewModel::new("二つの部品", vec![a, b], vec![Some("材".into())], 1).expect("モデル")
}

/// 試験の UV の点（(u, v)）。三角形ごとの中ほど。
const TRI0: (f32, f32) = (0.08, 0.10); // A の左の下の三角形
const TRI1: (f32, f32) = (0.22, 0.40); // A の左の上の三角形
const TRI2: (f32, f32) = (0.32, 0.10); // A の右の下の三角形
const TRI3: (f32, f32) = (0.43, 0.40); // A の右の上の三角形
const TRI4: (f32, f32) = (0.58, 0.10); // B

/// モデルを読んだ状態（画面を使わない）。3D の見せ方は、正面から（yaw・pitch 0）。
fn with_model(size: u32) -> (AppState, Rect) {
    let mut s = AppState::new(size, size);
    s.view3d.set_model(two_parts_model());
    s.view3d.material = 0;
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    (
        s,
        Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0)),
    )
}

/// 2D キャンバスで UV の点が見える画面の点。
fn at_uv(s: &AppState, rect: Rect, uv: (f32, f32)) -> Pos2 {
    let view = s.view.view(rect, s.doc.width(), s.doc.height());
    view.to_screen(
        uv.0 as f64 * s.doc.width() as f64,
        uv.1 as f64 * s.doc.height() as f64,
    )
}

/// 3D ビューでモデルの点が見える画面の点。
fn at_model(s: &AppState, rect: Rect, p: Vec3) -> Pos2 {
    let view = s.view3d.camera.view(rect.width(), rect.height());
    let q = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + q.x, rect.top() + q.y)
}

/// 文書の UV の点の合成の画素（Color）。
fn px(s: &AppState, uv: (f32, f32)) -> [u8; 4] {
    composite_pixel(
        &s.doc,
        (uv.0 * s.doc.width() as f32) as u32,
        (uv.1 * s.doc.height() as f32) as u32,
    )
}

fn painted(s: &AppState, uv: (f32, f32)) -> bool {
    px(s, uv)[3] > 0
}

fn canvas_view(s: &AppState, rect: Rect) -> yolu_app::canvas::view::CanvasView {
    s.view.view(rect, s.doc.width(), s.doc.height())
}

// ───────── マテリアルで塗る ─────────

/// 画面を使わずにストロークで塗る（2D の点を 2 つ）。
fn stroke(s: &mut AppState, from: (f64, f64), to: (f64, f64)) {
    use yolu_app::engine::DVec2;
    let layer = s.selected_layer.unwrap();
    let mut stroke = s.begin_paint_stroke(layer, false).expect("始められる");
    for p in [from, to] {
        stroke
            .add_point(&mut s.doc, p.0, p.1, 1.0, DVec2::ZERO)
            .unwrap();
    }
    s.doc.end_stroke(stroke).unwrap();
}

/// マスクの画素の隠す量（0 は見せる、255 は隠す）。
fn mask_hide(s: &AppState, id: yolu_app::engine::LayerId, x: u32, y: u32) -> u8 {
    s.doc
        .layer(id)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .pixel(x, y)
        .expect("マスクの画素を読める（読めない失敗を 0 = 見せる に見せない）")
        .a
}

fn channel_pixel(s: &AppState, channel: Channel, x: u32, y: u32) -> Rgba8 {
    let layer = s.doc.layer(s.selected_layer.unwrap()).unwrap();
    layer
        .surface(channel)
        .and_then(|surface| surface.pixel(x, y).ok())
        .unwrap_or(Rgba8::TRANSPARENT)
}

#[test]
fn headless_a_material_stroke_paints_every_channel_in_one_undo() {
    let mut s = AppState::new(64, 64);
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(
        Channel::Color,
    )));
    s.apply(Action::Mat(MatAction::Enabled(true)));
    assert_eq!(
        s.mat.included(),
        vec![Channel::Color],
        "描くチャンネル 1 つから始まる"
    );
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Emission, true)));
    s.mat.set_scalar(Channel::Roughness, 0.25);
    s.apply(Action::Mat(MatAction::Emission([0.0, 1.0, 0.0])));
    let before = s.doc.undo_count();
    stroke(&mut s, (10.0, 32.0), (50.0, 32.0));
    assert_eq!(s.doc.undo_count(), before + 1, "全チャンネルで 1 回の Undo");
    assert_eq!(
        channel_pixel(&s, Channel::Color, 30, 32),
        Rgba8::new(255, 0, 0, 255)
    );
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, 30, 32),
        Rgba8::new(64, 64, 64, 255)
    );
    assert_eq!(
        channel_pixel(&s, Channel::Emission, 30, 32),
        Rgba8::new(0, 255, 0, 255)
    );
    assert_eq!(
        channel_pixel(&s, Channel::Metallic, 30, 32),
        Rgba8::TRANSPARENT,
        "組に無いチャンネルは塗らない"
    );
    s.apply(Action::Undo);
    for c in [Channel::Color, Channel::Roughness, Channel::Emission] {
        assert_eq!(channel_pixel(&s, c, 30, 32), Rgba8::TRANSPARENT, "{c:?}");
    }
    s.apply(Action::Redo);
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, 30, 32),
        Rgba8::new(64, 64, 64, 255),
        "やり直しで戻る"
    );
}

#[test]
fn headless_material_off_paints_only_the_paint_channel_and_the_mask_ignores_the_material() {
    let mut s = AppState::new(64, 64);
    s.color.set_main([0.0, 0.0, 1.0, 1.0]);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    s.apply(Action::Mat(MatAction::Enabled(false)));
    assert!(
        s.mat.included().contains(&Channel::Roughness),
        "組はオフにしても残る"
    );
    stroke(&mut s, (10.0, 20.0), (50.0, 20.0));
    assert_eq!(
        channel_pixel(&s, Channel::Color, 30, 20),
        Rgba8::new(0, 0, 255, 255)
    );
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, 30, 20),
        Rgba8::TRANSPARENT,
        "オフなら描くチャンネルだけ"
    );
    // マスクに描くあいだは、オンでもマスクだけを塗る
    s.apply(Action::Mat(MatAction::Enabled(true)));
    let id = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    assert!(s.m2.edit_mask);
    assert!(!s.paints_material());
    let layer_before = channel_pixel(&s, Channel::Roughness, 30, 40);
    stroke(&mut s, (10.0, 40.0), (50.0, 40.0));
    assert_eq!(channel_pixel(&s, Channel::Roughness, 30, 40), layer_before);
    assert_ne!(mask_hide(&s, id, 30, 40), 0, "マスクは塗った");
}

#[test]
fn headless_enabling_the_material_asks_for_a_channel_first_and_keeps_one() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(
        Channel::Metallic,
    )));
    s.apply(Action::Mat(MatAction::Enabled(true)));
    assert_eq!(s.mat.included(), vec![Channel::Metallic]);
    s.apply(Action::Mat(MatAction::Channel(Channel::Metallic, false)));
    assert_eq!(
        s.mat.included(),
        vec![Channel::Metallic],
        "最後の 1 つは外せない"
    );
    assert!(s.message.contains("1 つ以上"), "{}", s.message);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Metallic, false)));
    assert!(s.message.contains("at least one"), "{}", s.message);
}

#[test]
fn headless_material_actions_set_the_values_and_refuse_while_drawing() {
    let mut s = AppState::new(64, 64);
    s.color.set_main([0.2, 0.4, 0.6, 1.0]);
    s.apply(Action::Mat(MatAction::EmissionFromPaint));
    assert_eq!(s.mat.emission, [0.2, 0.4, 0.6]);
    s.apply(Action::Mat(MatAction::Emission([2.0, -1.0, 0.5])));
    assert_eq!(s.mat.emission, [1.0, 0.0, 0.5], "0〜1 に丸める");
    s.mat.set_normal(0.5, 0.5);
    s.apply(Action::Mat(MatAction::NormalFlat));
    assert_eq!(s.mat.normal, [0.0, 0.0]);
    // 描いている間は変えない
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Mat(MatAction::Enabled(true)));
    assert!(!s.mat.enabled, "描いている間はできない");
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.apply(Action::Region(RegionAction::Erase(true)));
    assert!(!s.region.erase);
    s.doc.cancel_stroke(stroke);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    assert!(s.mat.enabled);
}

#[test]
fn headless_refusals_are_short_reasons_in_both_languages() {
    use yolu_app::engine::CoreError;
    for lang in Lang::ALL {
        for e in [
            CoreError::StrokeActive,
            CoreError::LayerNotFound,
            CoreError::ChannelNotFound,
            CoreError::SourceBudgetExceeded,
            CoreError::WorkingBudgetExceeded,
        ] {
            let text = lang.core_error(&e);
            assert!(
                !text.is_empty() && text.chars().count() < 40,
                "{lang:?} {e:?}: {text}"
            );
        }
    }
    // 同じ誤りは、どの操作でも同じ文（英語は「Layer not found」の 1 つ）
    assert_eq!(
        Lang::En.core_error(&CoreError::LayerNotFound),
        "Layer not found"
    );
    assert_eq!(
        Lang::Ja.core_error(&CoreError::LayerNotFound),
        "レイヤーがありません"
    );
}

// ───────── バケツ ─────────

fn fill_state(kind: SurfaceRegionKind) -> (AppState, Rect) {
    let (mut s, rect) = with_model(128);
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.tool = Tool::Fill;
    s.apply(Action::Region(RegionAction::FillRange(kind)));
    (s, rect)
}

#[test]
fn headless_bucket_fills_the_clicked_region_in_2d_for_each_kind_with_one_undo() {
    // (範囲の種類, 押した点, 塗られるはずの点, 塗られないはずの点)
    let cases = [
        (
            SurfaceRegionKind::Triangle,
            TRI0,
            vec![TRI0],
            vec![TRI1, TRI2, TRI4],
        ),
        (
            SurfaceRegionKind::UvIsland,
            TRI0,
            vec![TRI0, TRI1],
            vec![TRI2, TRI3, TRI4],
        ),
        (
            SurfaceRegionKind::MeshPart,
            TRI1,
            vec![TRI0, TRI1, TRI2, TRI3],
            vec![TRI4],
        ),
        (
            SurfaceRegionKind::Material,
            TRI3,
            vec![TRI0, TRI1, TRI2, TRI3, TRI4],
            vec![(0.7, 0.8), (0.1, 0.8)],
        ),
    ];
    for (kind, press, filled, empty) in cases {
        let (mut s, rect) = fill_state(kind);
        let view = canvas_view(&s, rect);
        let before = s.doc.undo_count();
        let at = at_uv(&s, rect, press);
        bucket(&mut s, Where::Canvas(&view), at);
        assert_eq!(s.doc.undo_count(), before + 1, "{kind:?}: 1 回の Undo");
        assert!(s.modified);
        for p in &filled {
            assert_eq!(px(&s, *p), [255, 0, 0, 255], "{kind:?} {p:?}");
        }
        for p in &empty {
            assert!(!painted(&s, *p), "{kind:?} {p:?}");
        }
        s.apply(Action::Undo);
        for p in filled.iter().chain(&empty) {
            assert!(!painted(&s, *p), "{kind:?}: Undo で戻る {p:?}");
        }
    }
}

/// 縦の軸（キャンバスの中心）の線対称 2 本のとき、モデルの範囲のバケツは押した点の写しの下の範囲も塗る（UV の左半分の押下は、
/// 軸の向こうの部品 B にも当たる）。切にすると押した範囲だけ。
#[test]
fn headless_bucket_by_model_range_also_fills_the_range_under_each_copy_in_2d() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    common::rulers::vertical(&mut s, 64.0);
    let view = canvas_view(&s, rect);
    let before = s.doc.undo_count();
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), before + 1, "1 回の Undo");
    // 押した A の左のアイランドと、写し (0.92, 0.10) の下の B（UV アイランド）
    for p in [TRI0, TRI1, TRI4, (0.92, 0.10), (0.78, 0.40)] {
        assert_eq!(px(&s, p), [255, 0, 0, 255], "{p:?}");
    }
    for p in [TRI2, TRI3] {
        assert!(!painted(&s, p), "写しの下にない A の右のアイランド {p:?}");
    }
    s.apply(Action::Undo);
    for p in [TRI0, TRI1, TRI4, (0.92, 0.10)] {
        assert!(!painted(&s, p), "Undo で戻る {p:?}");
    }
    // 切: 押した範囲だけ
    s.region.snap_symmetry = false;
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(painted(&s, TRI0) && painted(&s, TRI1));
    assert!(!painted(&s, TRI4) && !painted(&s, (0.92, 0.10)));
}

/// 押した所に三角形が無くても、写しの下に三角形があればその範囲を塗る。どの写しの下にも無ければ断る。
#[test]
fn headless_bucket_by_model_range_uses_only_the_copies_that_have_a_triangle() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Triangle);
    common::rulers::vertical(&mut s, 64.0);
    let view = canvas_view(&s, rect);
    let base = s.doc.undo_count();
    // どちらにも三角形が無い (0.27, 0.70) と、その写し (0.73, 0.70): 断る
    let at = at_uv(&s, rect, (0.27, 0.70));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), base);
    assert!(s.message.contains("三角形がありません"), "{}", s.message);
    // 押した (0.27, 0.20) は A の 2 つのアイランドの間で三角形が無く、写し (0.73, 0.20) は B の最初の三角形の下
    let at = at_uv(&s, rect, (0.27, 0.20));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), base + 1, "{}", s.message);
    assert!(painted(&s, TRI4), "写しの下の三角形");
    assert!(
        !painted(&s, (0.92, 0.10)) && !painted(&s, TRI0),
        "ほかの三角形は塗らない"
    );
}

/// 3D ビューのバケツは対称定規の写しを使わない（2D だけ）。
#[test]
fn headless_bucket_in_3d_ignores_the_symmetry_ruler() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    common::rulers::vertical(&mut s, 64.0);
    let before = s.doc.undo_count();
    let at = at_model(&s, rect, Vec3::new(0.25, 0.25, 0.0));
    bucket(&mut s, Where::Surface(rect), at);
    assert_eq!(s.doc.undo_count(), before + 1);
    assert!(
        painted(&s, TRI0) && painted(&s, TRI1),
        "押した面のアイランド"
    );
    assert!(
        !painted(&s, TRI4) && !painted(&s, (0.92, 0.10)),
        "写しの下の部品 B は塗らない"
    );
}

#[test]
fn headless_bucket_in_3d_picks_the_face_under_the_pointer() {
    for (kind, press, filled, empty) in [
        (
            SurfaceRegionKind::UvIsland,
            Vec3::new(0.1, 0.1, 0.0),
            vec![TRI0, TRI1],
            vec![TRI2, TRI4],
        ),
        (
            SurfaceRegionKind::MeshPart,
            Vec3::new(0.9, 0.5, 0.0),
            vec![TRI0, TRI3],
            vec![TRI4],
        ),
        (
            SurfaceRegionKind::Material,
            Vec3::new(3.5, 0.5, 0.0),
            vec![TRI0, TRI4],
            vec![],
        ),
    ] {
        let (mut s, rect) = fill_state(kind);
        let at = at_model(&s, rect, press);
        bucket(&mut s, Where::Surface(rect), at);
        assert_eq!(s.doc.undo_count(), 1, "{kind:?}: {}", s.message);
        for p in &filled {
            assert!(painted(&s, *p), "{kind:?} {p:?}");
        }
        for p in &empty {
            assert!(!painted(&s, *p), "{kind:?} {p:?}");
        }
    }
}

#[test]
fn headless_bucket_stays_inside_the_selection_and_the_material_paints_every_channel() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Material);
    // 文書の左半分だけ選ぶ（UV の x < 0.5）
    let w = s.doc.width() as i64;
    let selection = SelectionMask::rectangle(&s.doc, 0, 0, w / 2, s.doc.height() as i64);
    s.doc.set_selection(Some(selection)).unwrap();
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    s.mat.set_scalar(Channel::Roughness, 1.0);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    let undo = s.doc.undo_count();
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(
        s.doc.undo_count(),
        undo + 1,
        "選択範囲と塗りで別々の段、塗りは 1 段"
    );
    assert!(
        painted(&s, TRI0) && !painted(&s, TRI4),
        "選択範囲の外は塗らない"
    );
    let (x, y) = ((TRI0.0 * 128.0) as u32, (TRI0.1 * 128.0) as u32);
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, x, y),
        Rgba8::new(255, 255, 255, 255)
    );
    s.apply(Action::Undo);
    assert!(!painted(&s, TRI0));
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, x, y),
        Rgba8::TRANSPARENT,
        "Undo で全チャンネルが戻る"
    );
}

#[test]
fn headless_bucket_erase_and_the_mask_follow_paint_and_erase() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(painted(&s, TRI0));
    s.apply(Action::Region(RegionAction::Erase(true)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(!painted(&s, TRI0), "消すで透明に");
    // マスク: 塗る = 白（見せる）= 隠す量 0、消す = 黒（隠す）
    let id = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    s.apply(Action::Region(RegionAction::Erase(true)));
    bucket(&mut s, Where::Canvas(&view), at);
    let (x, y) = ((TRI0.0 * 128.0) as u32, (TRI0.1 * 128.0) as u32);
    assert_eq!(mask_hide(&s, id, x, y), 255, "黒で隠す");
    s.apply(Action::Region(RegionAction::Erase(false)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(mask_hide(&s, id, x, y), 0, "白で見せる");
}

#[test]
fn headless_bucket_on_an_inverted_mask_writes_the_opposite_value() {
    // 白・黒はマスクのサムネイルの色。反転したマスクでは、塗る（白 = 見せる）が隠す量 255、消す（黒 = 隠す）が 0
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    let id = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    s.apply(Action::M2(yolu_app::m2::Edit::MaskInverted(id, true)));
    assert!(s.m2.edit_mask);
    let (x, y) = ((TRI0.0 * 128.0) as u32, (TRI0.1 * 128.0) as u32);
    assert_eq!(mask_hide(&s, id, x, y), 0, "初めは 0");
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(mask_hide(&s, id, x, y), 255, "反転したマスクの塗る（白）");
    s.apply(Action::Region(RegionAction::Erase(true)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(mask_hide(&s, id, x, y), 0, "反転したマスクの消す（黒）");
    // 反転しないマスクは逆
    s.apply(Action::M2(yolu_app::m2::Edit::MaskInverted(id, false)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(
        mask_hide(&s, id, x, y),
        255,
        "反転しないマスクの消す（黒 = 隠す）"
    );
    s.apply(Action::Region(RegionAction::Erase(false)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(
        mask_hide(&s, id, x, y),
        0,
        "反転しないマスクの塗る（白 = 見せる）"
    );
    // ポリゴン塗りつぶしも同じ向き
    s.apply(Action::M2(yolu_app::m2::Edit::MaskInverted(id, true)));
    s.tool = Tool::PolygonFill;
    assert!(begin_polygon(&mut s, Where::Canvas(&view), at));
    assert!(finish_drag(&mut s, false));
    assert_eq!(
        mask_hide(&s, id, x, y),
        255,
        "反転したマスクのポリゴン塗りつぶし（白）"
    );
}

#[test]
fn headless_bucket_by_color_fills_the_similar_area_in_2d_and_3d() {
    let mut s = AppState::new(64, 64);
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s.tool = Tool::Fill;
    s.apply(Action::Region(RegionAction::ByColor(true)));
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 512.0));
    let view = canvas_view(&s, rect);
    let undo = s.doc.undo_count();
    bucket(&mut s, Where::Canvas(&view), rect.center());
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(
        composite_pixel(&s.doc, 5, 5),
        [0, 255, 0, 255],
        "空のレイヤーは全体が近い色"
    );
    // 3D でも使える: 押した面の UV が指す画素から、2D と同じ近い色の範囲を求める（空のレイヤーは全体が近い色）
    let (mut s3, rect3) = with_model(64);
    s3.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s3.tool = Tool::Fill;
    s3.apply(Action::Region(RegionAction::ByColor(true)));
    let on_face = at_model(&s3, rect3, Vec3::new(0.1, 0.1, 0.0));
    bucket(&mut s3, Where::Surface(rect3), on_face);
    assert_eq!(s3.doc.undo_count(), 1);
    assert_eq!(
        composite_pixel(&s3.doc, 5, 5),
        [0, 255, 0, 255],
        "3D でも空のレイヤーは全体が近い色"
    );
    // 面の無い所では塗らず、理由を出す
    let mut empty = with_model(64).0;
    empty.tool = Tool::Fill;
    empty.apply(Action::Region(RegionAction::ByColor(true)));
    bucket(
        &mut empty,
        Where::Surface(rect3),
        pos2(rect3.left() + 1.0, rect3.top() + 1.0),
    );
    assert_eq!(empty.doc.undo_count(), 0);
    assert!(
        empty.message.contains("三角形がありません"),
        "{}",
        empty.message
    );
}

#[test]
fn headless_bucket_says_why_it_cannot_fill() {
    let mut s = AppState::new(64, 64);
    s.tool = Tool::Fill;
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 512.0));
    let view = canvas_view(&s, rect);
    bucket(&mut s, Where::Canvas(&view), rect.center());
    assert_eq!(
        s.message, "モデルがありません",
        "モデルが無いと範囲が決まらない"
    );
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    bucket(&mut s, Where::Canvas(&view), rect.center());
    assert_eq!(s.message, "No model");
    // 塗れないレイヤー（塗りつぶしレイヤー）
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    let undo = s.doc.undo_count();
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), undo);
    assert!(
        s.message.contains("このレイヤーには描けません"),
        "{}",
        s.message
    );
    // モデルの面でない所
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, (0.8, 0.9));
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(s.message.contains("三角形がありません"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), 0);
}

#[test]
fn headless_a_loaded_model_without_the_current_set_says_so_instead_of_no_model() {
    let (mut s, rect) = with_model(64);
    s.view3d.material = -1; // モデルはあるが、今のテクスチャセットはモデルのマテリアルに付いていない
    let name = s.sets.current().name.clone();
    let ja = format!("今のテクスチャセット「{name}」はこのモデルにありません。");
    assert_eq!(s.region_missing_reason(), ja);
    s.tool = Tool::Fill;
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.message, ja, "バケツ");
    s.message.clear();
    s.tool = Tool::PolygonFill;
    assert!(!begin_polygon(&mut s, Where::Canvas(&view), at));
    assert_eq!(s.message, ja, "ポリゴン塗りつぶし");
    assert_eq!(s.doc.undo_count(), 0);
    assert!(!s.is_stroking());
    s.message.clear();
    s.tool = Tool::IdSelect;
    let on_face = at_model(
        &s,
        Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0)),
        Vec3::new(0.1, 0.1, 0.0),
    );
    yolu_app::region::idcolor::select_by_id(
        &mut s,
        Where::Surface(Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0))),
        on_face,
    );
    assert!(s.doc.selection().is_none());
    // モデルが無いときは、これまでどおり「モデルがありません」
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    // 既定の名前は言語で変わるので、切り替えたあとの名前で確かめる
    let name = s.sets.current().name.clone();
    let en = format!("Texture set “{name}” is not in this model.");
    assert_eq!(s.region_missing_reason(), en);
    s.tool = Tool::Fill;
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.message, en, "英語");
    let mut none = AppState::new(64, 64);
    assert_eq!(none.region_missing_reason(), "モデルがありません");
    none.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    assert_eq!(none.region_missing_reason(), "No model");
}

/// Live Link と同じ形の、2 つのマテリアルのモデル（マテリアル 0 に部品 A、マテリアル 1 に離れた部品 B と C）。
/// マテリアル 1 のセットの部品はモデル全体の 1 と 2 で、0 から始まらない。
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
            width: 128,
            height: 128,
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
            indices: vec![0, 1, 2],
        }],
    };
    Model {
        generation: 1,
        name: "二つのセット".into(),
        materials: vec![material("MatA"), material("MatB")],
        meshes: vec![mesh("A", 0.0, 0), mesh("B", 3.0, 1), mesh("C", 6.0, 1)],
    }
}

/// そのモデルを読み、2 つ目のテクスチャセットを今のセットにした状態（画面を使わない）。
fn second_set_state() -> AppState {
    let mut s = AppState::new(64, 64);
    let (_, shape) = s.receive_link_model(&two_materials_link_model());
    shape.expect("3D に読める");
    let uid = s.sets.iter().nth(1).expect("2 つ目のセット").uid;
    s.apply(Action::SelectSet(uid));
    assert_eq!(s.view3d.material, 1, "2 つ目のセットはマテリアル 1");
    s
}

#[test]
fn headless_the_part_switcher_counts_inside_the_current_set_not_the_whole_model() {
    use yolu_app::panels::region_props::part_position;
    let mut s = second_set_state();
    s.tool = Tool::IdSelect;
    let parts = ready_parts(&mut s);
    assert_eq!(parts, vec![1, 2], "セット 1 の部品はモデル全体の 1 と 2");
    for (at, ja, en) in [
        (0, "部品 1 / 2", "Part 1 / 2"),
        (1, "部品 2 / 2", "Part 2 / 2"),
    ] {
        assert_eq!(part_position(Lang::Ja, at, parts.len()), ja);
        assert_eq!(part_position(Lang::En, at, parts.len()), en);
    }
    let uid = s.sets.iter().next().unwrap().uid;
    s.apply(Action::SelectSet(uid));
    assert_eq!(ready_parts(&mut s), vec![0], "セット 0 は部品 A だけ");
}

#[test]
fn headless_a_locked_layer_refuses_every_way_of_painting_with_a_short_reason() {
    use yolu_core::LayerLocks;
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    let id = s.selected_layer.unwrap();
    s.doc.set_layer_locks(id, LayerLocks::PIXELS).unwrap();
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    let undo = s.doc.undo_count();
    // バケツ
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), undo);
    assert!(!painted(&s, TRI0));
    assert_eq!(s.message, "レイヤーの「画素」がロックされています");
    // ポリゴン塗りつぶし（始められない）
    s.message.clear();
    assert!(!begin_polygon(&mut s, Where::Canvas(&view), at));
    assert_eq!(s.message, "レイヤーの「画素」がロックされています");
    assert!(!s.is_stroking());
    // ブラシ
    assert!(s.begin_paint_stroke(id, false).is_err());
    // マテリアルで塗るときも
    s.apply(Action::Mat(MatAction::Enabled(true)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(s.message.contains("ロック"), "{}", s.message);
    // 英語
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.message, "The layer has \"Image pixels\" locked");
    // すべてのロック・親グループのロック
    s.doc.set_layer_locks(id, LayerLocks::ALL).unwrap();
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.message, "The layer has \"All\" locked");
    s.doc.set_layer_locks(id, LayerLocks::NONE).unwrap();
    let group = s.doc.group_layers(&[id], "g").unwrap();
    s.doc
        .set_layer_locks(group, LayerLocks::PIXELS | LayerLocks::POSITION)
        .unwrap();
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(s.message.starts_with("A parent group has"), "{}", s.message);
    assert!(s.message.contains("Image pixels"), "{}", s.message);
    // ロックを外せば塗れる
    s.doc.set_layer_locks(group, LayerLocks::NONE).unwrap();
    bucket(&mut s, Where::Canvas(&view), at);
    assert!(painted(&s, TRI0), "{}", s.message);
}

#[test]
fn headless_the_bucket_and_the_brush_write_the_same_value_into_a_scalar_channel() {
    // 描くチャンネルが Roughness（スカラー）で描画色が赤。ブラシのストロークと、バケツ・ポリゴン塗りつぶしの値が同じ（描画色のまま）
    let red = Rgba8::new(255, 0, 0, 255);
    let prepare = |s: &mut AppState| {
        s.color.set_main([1.0, 0.0, 0.0, 1.0]);
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(yolu_app::m2::Edit::ChannelEnabled {
            id,
            channel: Channel::Roughness,
            enabled: true,
        }));
        s.apply(Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(
            Channel::Roughness,
        )));
        assert!(!s.mat.enabled, "マテリアルはオフ");
    };
    // ブラシ
    let (mut brush, _) = with_model(128);
    prepare(&mut brush);
    stroke(&mut brush, (10.0, 12.0), (30.0, 12.0));
    let by_brush = channel_pixel(&brush, Channel::Roughness, 20, 12);
    assert_eq!(by_brush, red, "ブラシは描画色の RGB のまま");
    // バケツ
    let (mut s, rect) = with_model(128);
    s.tool = Tool::Fill;
    prepare(&mut s);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    let (x, y) = ((TRI0.0 * 128.0) as u32, (TRI0.1 * 128.0) as u32);
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, x, y),
        by_brush,
        "バケツはブラシと同じ値: {}",
        s.message
    );
    // ポリゴン塗りつぶし
    let (mut p, rect) = with_model(128);
    p.tool = Tool::PolygonFill;
    prepare(&mut p);
    let view = canvas_view(&p, rect);
    let at = at_uv(&p, rect, TRI0);
    assert!(begin_polygon(&mut p, Where::Canvas(&view), at));
    assert!(finish_drag(&mut p, false));
    assert_eq!(
        channel_pixel(&p, Channel::Roughness, x, y),
        by_brush,
        "ポリゴン塗りつぶしも同じ値"
    );
    // 塗りつぶしレイヤーの値も、同じ変換（アルファは 255）
    assert_eq!(yolu_app::m2::fill_from_color([1.0, 0.0, 0.0, 0.3]), red);
    assert_eq!(
        yolu_app::matpaint::single_value([1.0, 0.0, 0.0, 0.3]).a,
        77,
        "1 チャンネルの値は描画色のアルファのまま"
    );
}

#[test]
fn headless_a_read_only_set_refuses_the_bucket_the_polygon_fill_and_the_id_select() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    bake_id(&mut s);
    let uid_index = s.sets.current_index();
    s.sets.get_mut(uid_index).unwrap().read_only =
        Some("フィルターのあるレイヤーがあります".into());
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let at = at_uv(&s, rect, TRI0);
    let undo = s.doc.undo_count();
    let reason = "このテクスチャセットは読むだけです（フィルターのあるレイヤーがあります）。";
    // バケツ
    bucket(&mut s, w, at);
    assert_eq!(s.message, reason);
    assert!(!painted(&s, TRI0));
    // ポリゴン塗りつぶし
    s.message.clear();
    assert!(!begin_polygon(&mut s, w, at));
    assert_eq!(s.message, reason);
    assert!(s.region.drag.is_none() && !s.is_stroking());
    // ID の色で選択（選択範囲も変えない。変えても Undo が断られて戻せない）
    s.message.clear();
    s.tool = Tool::IdSelect;
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(s.message, reason);
    assert!(s.doc.selection().is_none());
    assert_eq!(s.doc.undo_count(), undo, "履歴に何も積まない");
    // 英語
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    s.sets.get_mut(uid_index).unwrap().read_only = Some("has filters".into());
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(s.message, "This texture set is read-only (has filters).");
    // 読むだけでなくなれば選べる
    s.sets.get_mut(uid_index).unwrap().read_only = None;
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert!(s.doc.selection().is_some(), "{}", s.message);
}

/// 小さい予算の文書（タイル 4 枚分の覆いまで。512 の文書で A のアイランドは 2〜4 枚、B のアイランドは別の 4 枚）。
fn four_tile_budget(s: &mut AppState) -> u64 {
    let ts = s.doc.tile_size() as u64;
    let budget = 4 * (16 + 2 * ts * ts);
    s.doc.set_stroke_budget_bytes(budget).unwrap();
    budget
}

#[test]
fn headless_polygon_fill_over_the_budget_is_refused_and_leaves_the_document_as_it_was() {
    // 最初の範囲で超える: 始まらない
    let (mut s, rect) = with_model(512);
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.tool = Tool::PolygonFill;
    s.apply(Action::Region(RegionAction::FillRange(
        SurfaceRegionKind::UvIsland,
    )));
    s.doc.set_stroke_budget_bytes(1).unwrap();
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let (a, b) = (at_uv(&s, rect, TRI0), at_uv(&s, rect, TRI4));
    assert!(
        !begin_polygon(&mut s, w, a),
        "最初の範囲で予算を超えたら始まらない"
    );
    assert!(s.region.drag.is_none() && !s.is_stroking() && !s.doc.has_active_stroke());
    assert!(s.message.contains("予算"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), 0);
    assert!(!painted(&s, TRI0));
    // 途中で超える: 最初のアイランドは塗れて、別の部品を足すと予算を超え、そこまでの塗りも戻す
    let budget = four_tile_budget(&mut s);
    assert!(begin_polygon(&mut s, w, a), "{}", s.message);
    assert!(
        painted(&s, TRI0),
        "最初のアイランドは塗れている（予算 {budget}）"
    );
    drag_to(&mut s, w, b);
    assert!(s.region.drag.is_none(), "超えたら札を手放す");
    assert!(!s.is_stroking() && !s.doc.has_active_stroke());
    assert!(s.message.contains("予算"), "{}", s.message);
    assert!(!painted(&s, TRI0) && !painted(&s, TRI4), "文書は元のまま");
    assert_eq!(s.doc.undo_count(), 0, "履歴に何も積まない");
    assert!(!finish_drag(&mut s, false), "離しても終えるものが無い");
    // 英語
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    s.doc.set_stroke_budget_bytes(1).unwrap();
    assert!(!begin_polygon(&mut s, w, a));
    assert_eq!(
        s.message,
        "Over the memory budget of one operation (cancelled)"
    );
}

#[test]
fn headless_bucket_over_the_budget_is_refused_and_leaves_the_document_as_it_was() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Material);
    let allocated = s.doc.allocated_bytes();
    s.doc.set_source_budget_bytes(allocated).unwrap();
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.message, "レイヤーのメモリの予算を超えます");
    assert_eq!(s.doc.undo_count(), 0);
    assert!(!painted(&s, TRI0) && !painted(&s, TRI4), "文書は元のまま");
    assert!(!s.modified);
}

// ───────── ポリゴン塗りつぶし ─────────

#[test]
fn headless_polygon_fill_adds_the_regions_it_passes_and_commits_as_one_undo() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    s.tool = Tool::PolygonFill;
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let a = at_uv(&s, rect, TRI0);
    let b = at_uv(&s, rect, TRI2);
    let c = at_uv(&s, rect, TRI4);
    let undo = s.doc.undo_count();
    assert!(begin_polygon(&mut s, w, a));
    assert!(s.is_stroking(), "ドラッグの間は描いている扱い");
    assert!(
        painted(&s, TRI0) && painted(&s, TRI1),
        "押したアイランドはすぐ塗る"
    );
    drag_to(&mut s, w, b);
    drag_to(&mut s, w, c);
    assert_eq!(
        s.region.drag.as_ref().unwrap().regions(),
        3,
        "通ったアイランドの数"
    );
    assert!(
        painted(&s, TRI2) && painted(&s, TRI4),
        "動かすと足す。間のアイランドも飛ばさない"
    );
    assert_eq!(s.doc.undo_count(), undo, "離すまで履歴に積まない");
    // 描いている間は文書を変える操作を断る
    s.apply(Action::Undo);
    assert!(s.message.contains("描いている間"), "{}", s.message);
    assert!(finish_drag(&mut s, false));
    assert_eq!(s.doc.undo_count(), undo + 1, "離して 1 回の Undo");
    assert!(!s.is_stroking());
    assert!(s.message.contains("UV アイランド × 3"), "{}", s.message);
    s.apply(Action::Undo);
    for p in [TRI0, TRI1, TRI2, TRI3, TRI4] {
        assert!(!painted(&s, p), "{p:?}");
    }
}

#[test]
fn headless_polygon_fill_cancel_leaves_nothing_and_erase_removes() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Triangle);
    s.tool = Tool::PolygonFill;
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let a = at_uv(&s, rect, TRI0);
    assert!(begin_polygon(&mut s, w, a));
    assert!(painted(&s, TRI0));
    assert!(finish_drag(&mut s, true));
    assert!(!painted(&s, TRI0), "取り消すと戻る");
    assert_eq!(s.doc.undo_count(), 0, "履歴に何も残さない");
    assert!(s.region.drag.is_none());
    // 塗ってから消す
    assert!(begin_polygon(&mut s, w, a));
    assert!(finish_drag(&mut s, false));
    s.apply(Action::Region(RegionAction::Erase(true)));
    assert!(begin_polygon(&mut s, w, a));
    assert!(finish_drag(&mut s, false));
    assert!(!painted(&s, TRI0), "消すで透明に");
    assert!(s.message.contains("消しました"), "{}", s.message);
    // 何も無い所からのドラッグ: 何も塗らず、理由を出す
    let nothing = at_uv(&s, rect, (0.8, 0.9));
    assert!(begin_polygon(&mut s, w, nothing));
    assert!(finish_drag(&mut s, false));
    assert!(s.message.contains("三角形がありません"), "{}", s.message);
}

#[test]
fn headless_polygon_fill_in_3d_follows_the_pointer_across_faces() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::MeshPart);
    s.tool = Tool::PolygonFill;
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    let w = Where::Surface(rect);
    let on_a = at_model(&s, rect, Vec3::new(0.2, 0.5, 0.0));
    let on_b = at_model(&s, rect, Vec3::new(3.5, 0.5, 0.0));
    assert!(begin_polygon(&mut s, w, on_a));
    // 2D の入力は、3D で始めたドラッグには効かない
    let view = canvas_view(&s, rect);
    let b_point = at_uv(&s, rect, TRI4);
    drag_to(&mut s, Where::Canvas(&view), b_point);
    assert!(!painted(&s, TRI4));
    drag_to(&mut s, w, on_b);
    assert!(painted(&s, TRI0) && painted(&s, TRI4));
    assert!(finish_drag(&mut s, false));
    assert_eq!(s.doc.undo_count(), 1);
    let (x, y) = ((TRI4.0 * 128.0) as u32, (TRI4.1 * 128.0) as u32);
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, x, y),
        Rgba8::new(128, 128, 128, 255),
        "マテリアルの全チャンネル"
    );
    s.apply(Action::Undo);
    assert_eq!(
        channel_pixel(&s, Channel::Roughness, x, y),
        Rgba8::TRANSPARENT
    );
}

// ───────── 強調 ─────────

#[test]
fn headless_hover_finds_the_region_under_the_pointer_and_clears_outside() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Triangle);
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    for (kind, tris, outline) in [
        (SurfaceRegionKind::Triangle, 1, 3),
        (SurfaceRegionKind::UvIsland, 2, 4),
        (SurfaceRegionKind::MeshPart, 4, 8),
        (SurfaceRegionKind::Material, 6, 12),
    ] {
        s.apply(Action::Region(RegionAction::FillRange(kind)));
        {
            let at = at_uv(&s, rect, TRI0);
            update_hover(&mut s, w, Some(at));
        }
        assert_eq!(s.region_hover_len(), Some(tris), "{kind:?}");
        assert_eq!(
            s.region.hover.as_ref().unwrap().outline.len(),
            outline,
            "{kind:?} の UV の輪郭"
        );
    }
    // 三角形が変わらなければ同じ強調のまま、モデルの面でない所では消える
    s.apply(Action::Region(RegionAction::FillRange(
        SurfaceRegionKind::UvIsland,
    )));
    {
        let at = at_uv(&s, rect, TRI0);
        update_hover(&mut s, w, Some(at));
    }
    let key = s.region.hover.as_ref().unwrap().key;
    {
        let at = at_uv(&s, rect, TRI1);
        update_hover(&mut s, w, Some(at));
    }
    assert_eq!(
        s.region.hover.as_ref().unwrap().key,
        key,
        "同じアイランドなら引き直さない"
    );
    {
        let at = at_uv(&s, rect, (0.8, 0.9));
        update_hover(&mut s, w, Some(at));
    }
    assert!(s.region.hover.is_none());
    // 3D
    let rect3 = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    let on_face = at_model(&s, rect3, Vec3::new(0.1, 0.1, 0.0));
    update_hover(&mut s, Where::Surface(rect3), Some(on_face));
    assert_eq!(s.region_hover_len(), Some(2));
    assert!(s.region.hover.as_ref().unwrap().on_surface);
    // ブラシでは出さない
    s.tool = Tool::Brush;
    {
        let at = at_uv(&s, rect, TRI0);
        update_hover(&mut s, w, Some(at));
    }
    assert!(s.region.hover.is_none());
    // 近い色のバケツは範囲を持たない
    s.tool = Tool::Fill;
    s.apply(Action::Region(RegionAction::ByColor(true)));
    {
        let at = at_uv(&s, rect, TRI0);
        update_hover(&mut s, w, Some(at));
    }
    assert!(s.region.hover.is_none());
}

#[test]
fn headless_a_panel_without_the_pointer_clears_only_its_own_hover() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    let view = canvas_view(&s, rect);
    let canvas = Where::Canvas(&view);
    let surface = Where::Surface(rect);
    let at = at_uv(&s, rect, TRI0);
    update_hover(&mut s, canvas, Some(at));
    let made = s.region.hover.as_ref().expect("2D の強調").tris.clone();
    // 3D のパネル（ポインタが無い）が毎フレーム呼んでも、2D の強調は消さず、2D は引き直さない
    for _ in 0..3 {
        update_hover(&mut s, surface, None);
        update_hover(&mut s, canvas, Some(at));
        let now = &s.region.hover.as_ref().expect("残っている").tris;
        assert!(Arc::ptr_eq(&made, now), "同じ範囲を作り直していない");
    }
    // 持ち主（2D）のポインタが無くなれば消す
    update_hover(&mut s, canvas, None);
    assert!(s.region.hover.is_none());
    // 3D の強調も同じ（2D のパネルのポインタが無いのでは消えない）
    let on_face = at_model(&s, rect, Vec3::new(0.1, 0.1, 0.0));
    update_hover(&mut s, surface, Some(on_face));
    assert!(s.region.hover.as_ref().is_some_and(|h| h.on_surface));
    update_hover(&mut s, canvas, None);
    assert!(
        s.region.hover.is_some(),
        "2D のパネルは 3D の強調を消さない"
    );
    update_hover(&mut s, surface, None);
    assert!(s.region.hover.is_none());
    // ツールが範囲を出さないときは、どの画面のものも消す
    update_hover(&mut s, canvas, Some(at));
    s.tool = Tool::Brush;
    update_hover(&mut s, surface, None);
    assert!(s.region.hover.is_none());
}

#[test]
fn the_hover_under_the_pointer_is_not_rebuilt_every_frame_with_the_canvas_and_the_3d_view_side_by_side(
) {
    use egui_dock::{DockState, NodeIndex};
    let mut h = app_with_model(1280.0, 800.0, 256);
    // キャンバスと 3D ビューを左右に並べる
    let mut dock = DockState::new(vec![yolu_app::Tab::Canvas]);
    dock.main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![yolu_app::Tab::View3d]);
    h.state_mut().dock = dock;
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    let rect = h.state().view3d_rect().expect("3D も出ている");
    let canvas = canvas_rect(&h);
    assert!(canvas.right() <= rect.left() + 1.0, "左右に並んでいる");
    let [a, ..] = face_points(&h, rect);
    // 3D にポインタを置く
    move_to(&h, a);
    h.run();
    let first = h
        .state()
        .state
        .region
        .hover
        .as_ref()
        .expect("3D の強調")
        .tris
        .clone();
    assert!(h.state().state.region.hover.as_ref().unwrap().on_surface);
    // 動かさずに何フレーム回しても、同じ範囲（2D のパネルがポインタが無いと消さない）
    for _ in 0..3 {
        h.step();
        let hover = h.state().state.region.hover.as_ref().expect("残っている");
        assert!(
            Arc::ptr_eq(&first, &hover.tris),
            "毎フレーム引き直していない"
        );
    }
    // 2D にポインタを移すと、2D の強調になる
    let at = canvas_at(&h, TRI0);
    move_to(&h, at);
    h.run();
    let hover = h.state().state.region.hover.as_ref().expect("2D の強調");
    assert!(!hover.on_surface);
}

// ───────── 手動の ID の色・ID の色で選択 ─────────

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-region-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 今のモデルの部品（毎フレームの表示用の口は、モデルの入力を別のスレッドで作り終えるまで None なので、できるまで待つ）。
fn ready_parts(s: &mut AppState) -> Vec<usize> {
    let start = std::time::Instant::now();
    loop {
        if let Some(parts) = s.id_set_parts() {
            return parts;
        }
        assert!(start.elapsed().as_secs() < 120, "部品が求まらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn headless_manual_id_colors_are_one_undo_each_and_are_saved_and_come_back_when_reopened() {
    let (mut s, _) = with_model(64);
    let parts = ready_parts(&mut s);
    assert_eq!(parts, vec![0, 1], "A と B の 2 つのメッシュの塊");
    let undo = s.doc.undo_count();
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 1,
        rgb: Some(0x336699),
    })));
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(s.doc.id_colors().colors().get(&1), Some(&0x336699));
    assert!(s.modified);
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0xff0000),
    })));
    assert_eq!(s.doc.undo_count(), undo + 2);
    // 保存できる（黙って落とさない）。開き直すと同じ色が戻り、取り消しの履歴は空
    let dir = temp_dir("save");
    s.apply(Action::SaveProjectAs(dir.join("manual.ylp")));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(dir.join("manual.ylp").exists());
    assert!(!s.modified);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(dir.join("manual.ylp")));
    assert_eq!(again.doc.id_colors().colors(), s.doc.id_colors().colors());
    assert_eq!(again.doc.id_colors().binding(), s.doc.id_colors().binding());
    assert_eq!(again.doc.id_colors().colors().len(), 2);
    assert!(!again.doc.can_undo() && !again.doc.can_redo());
    assert!(again.sets.get(0).unwrap().read_only.is_none());
    // 自動に戻す・全部戻す・Undo
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: None,
    })));
    assert_eq!(s.doc.id_colors().colors().len(), 1);
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::ResetAll)));
    assert!(s.doc.id_colors().colors().is_empty());
    s.apply(Action::Undo);
    assert_eq!(
        s.doc.id_colors().colors().len(),
        1,
        "全部戻したのも 1 回の Undo"
    );
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert!(s.doc.id_colors().colors().is_empty());
    // 手動の色が無ければ保存できる（塊を書かない）
    s.apply(Action::SaveProjectAs(dir.join("plain.ylp")));
    assert!(dir.join("plain.ylp").exists(), "{}", s.message);
    let mut plain = AppState::new(64, 64);
    plain.apply(Action::OpenProject(dir.join("plain.ylp")));
    assert!(plain.doc.id_colors().colors().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 開き直した手動の ID の色は指紋ごと戻る: 同じモデルを読み込めば合い、別のモデルなら別のモデルのものとして編集を断る（色は捨てず、
/// その状態で保存し直しても残る）。
#[test]
fn headless_manual_id_colors_come_back_with_their_model_fingerprint_and_are_kept_for_another_model()
{
    let (mut s, _) = with_model(64);
    ready_parts(&mut s);
    for (part, rgb) in [(1, 0x336699), (0, 0xff0000)] {
        s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
            part,
            rgb: Some(rgb),
        })));
    }
    let before = s.doc.id_colors().clone();
    assert_eq!(before.colors().len(), 2);
    let dir = temp_dir("fingerprint");
    s.apply(Action::SaveProjectAs(dir.join("manual.ylp")));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // 開き直す（モデルはファイルに無い）: 色は指紋ごと元のまま
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(dir.join("manual.ylp")));
    assert_eq!(again.doc.id_colors().binding(), before.binding());
    assert_eq!(again.doc.id_colors().colors(), before.colors());
    assert!(!again.doc.can_undo());
    // 同じモデルなら指紋が合い、別のモデルのものではない
    again.view3d.set_model(two_parts_model());
    again.view3d.material = 0;
    ready_parts(&mut again);
    assert!(!again.id_colors_foreign());
    // 別のモデルなら、色は残したまま別のモデルのものとして編集を断る
    again
        .view3d
        .set_model(yolu_app::view3d::model::ViewModel::demo(2));
    ready_parts(&mut again);
    assert!(again.id_colors_foreign());
    assert_eq!(again.doc.id_colors().colors(), before.colors());
    // その状態で保存し直しても、色は消えない
    again.modified = true;
    again.apply(Action::SaveProjectAs(dir.join("kept.ylp")));
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    let mut third = AppState::new(64, 64);
    third.apply(Action::OpenProject(dir.join("kept.ylp")));
    assert_eq!(third.doc.id_colors().binding(), before.binding());
    assert_eq!(third.doc.id_colors().colors(), before.colors());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 開いたプロジェクトで手動の ID の色だけを変えても、そのセットは保存で書き直され、色が残る（画素は変わらないので、書き直しの要否を
/// 画素の変更だけで決めていると、色の変更が保存から漏れる）。
#[test]
fn headless_changing_only_the_manual_id_colors_of_an_opened_project_is_saved() {
    let (mut s, _) = with_model(64);
    ready_parts(&mut s);
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 1,
        rgb: Some(0x336699),
    })));
    let dir = temp_dir("edit-saved");
    let path = dir.join("manual.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    again.view3d.set_model(two_parts_model());
    again.view3d.material = 0;
    ready_parts(&mut again);
    assert!(!again.modified);
    // 開いたあとに色だけを変える（画素は変えない）
    again.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0xff0000),
    })));
    assert!(again.modified);
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    assert_eq!(again.rewritten_sets, 1, "色だけを変えたセットも書き直す");
    let mut third = AppState::new(64, 64);
    third.apply(Action::OpenProject(path));
    assert_eq!(
        third.doc.id_colors().colors(),
        &std::collections::BTreeMap::from([(0, 0xff0000), (1, 0x336699)])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_manual_id_colors_refuse_unknown_parts_and_other_models() {
    let (mut s, _) = with_model(64);
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 9,
        rgb: Some(1),
    })));
    assert!(s.message.contains("その部品はありません"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), 0);
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0x123456),
    })));
    // モデルを替えると、手動の色は別のモデルのものになり、編集を断る
    s.view3d
        .set_model(yolu_app::view3d::model::ViewModel::demo(2));
    ready_parts(&mut s);
    assert!(s.id_colors_foreign());
    let undo = s.doc.undo_count();
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0x654321),
    })));
    assert_eq!(s.doc.undo_count(), undo);
    assert!(s.message.contains("別のモデル"), "{}", s.message);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0x654321),
    })));
    assert!(s.message.contains("another model"), "{}", s.message);
    // 全部戻すはできる
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::ResetAll)));
    assert!(s.doc.id_colors().colors().is_empty());
}

#[test]
fn headless_the_id_panel_does_not_wait_for_the_model_input_and_shows_checking() {
    let (mut s, _) = with_model(64);
    s.tool = Tool::IdSelect;
    // モデルが替わった直後のフレーム: 入力を別のスレッドで作らせ、待たずに None（画面は「確かめています」）
    assert!(s.id_set_parts().is_none(), "待たない");
    assert!(s.bake.is_checking(), "別のスレッドで作っている");
    assert!(!s.id_colors_foreign(), "待たずに false");
    assert_eq!(s.id_part_hint(0), None);
    // ID のツールを選んでいる間は、ウィンドウが無くても作っている最中のものを手放さないので、いつかは求まる
    let start = std::time::Instant::now();
    let parts = loop {
        if let Some(parts) = s.id_set_parts() {
            break parts;
        }
        s.release_idle_bake_input();
        assert!(start.elapsed().as_secs() < 120, "部品が求まらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(parts, vec![0, 1]);
    assert!(!s.bake.is_checking());
    // 同じモデルなら、次のフレームからは作り直さない
    assert_eq!(s.id_set_parts(), Some(vec![0, 1]));
    assert!(!s.bake.is_checking());
    // 別のモデルに替わると、また待たずに None
    s.view3d
        .set_model(yolu_app::view3d::model::ViewModel::demo(2));
    assert!(s.id_set_parts().is_none());
    // 押して直すときは、作り終えるまで待つ
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 0,
        rgb: Some(0x445566),
    })));
    assert_eq!(
        s.doc.id_colors().colors().get(&0),
        Some(&0x445566),
        "{}",
        s.message
    );
}

/// 今のセットの ID マップを、アプリのベイク（ベイクのウィンドウと同じ道）で焼く。UV アイランドごとに色が付く。
fn bake_id(s: &mut AppState) {
    s.bake.settings.maps = vec![MeshMapKind::Id];
    s.bake.settings.id_source = MeshIdSource::UvIsland;
    s.bake.settings.padding = 0;
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    assert!(
        s.sets.current().mesh_maps.get(MeshMapKind::Id).is_some(),
        "焼けた: {}",
        s.message
    );
}

fn selected(s: &AppState, uv: (f32, f32)) -> u8 {
    s.doc.selection().map_or(0, |m| {
        m.amount(
            (uv.0 * s.doc.width() as f32) as u32,
            (uv.1 * s.doc.height() as f32) as u32,
        )
    })
}

#[test]
fn headless_id_select_picks_the_part_under_the_pointer_with_combine_modes() {
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    bake_id(&mut s);
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let press = |s: &mut AppState, uv: (f32, f32), shift: bool, ctrl: bool| {
        s.region.modifiers = Modifiers {
            shift,
            ctrl,
            command: ctrl,
            ..Default::default()
        };
        let at = at_uv(s, rect, uv);
        yolu_app::region::idcolor::select_by_id(s, w, at);
    };
    press(&mut s, TRI0, false, false);
    assert_eq!(selected(&s, TRI0), 255);
    assert_eq!(selected(&s, TRI1), 255, "同じアイランド");
    assert_eq!(selected(&s, TRI2), 0, "別のアイランド");
    assert_eq!(selected(&s, TRI4), 0);
    assert!(s.message.contains("ID の色 #"), "{}", s.message);
    assert!(s.message.contains("新規"), "{}", s.message);
    let undo = s.doc.undo_count();
    press(&mut s, TRI4, true, false);
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(
        (selected(&s, TRI0), selected(&s, TRI4)),
        (255, 255),
        "Shift で足す"
    );
    press(&mut s, TRI0, false, true);
    assert_eq!(
        (selected(&s, TRI0), selected(&s, TRI4)),
        (0, 255),
        "Ctrl で引く"
    );
    press(&mut s, TRI4, true, true);
    assert_eq!(
        selected(&s, TRI4),
        255,
        "Shift+Ctrl で重ねる（今の選択の中の同じ色）"
    );
    // 選択の 1 回ごとが Undo の 1 段
    s.apply(Action::Undo);
    assert_eq!(selected(&s, TRI4), 255);
    // ID の色のない所（UV の外）は選ばず、理由を出す
    let before = s.doc.undo_count();
    press(&mut s, (0.8, 0.9), false, false);
    assert_eq!(s.doc.undo_count(), before);
    assert!(s.message.contains("部品がありません"), "{}", s.message);
}

#[test]
fn headless_id_select_follows_the_combine_mode_of_the_selection_tools() {
    use yolu_app::selection::{SelAction, SelUiOp};
    use yolu_core::SelectionCombine;
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    bake_id(&mut s);
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    s.region.modifiers = Modifiers::NONE;
    let press = |s: &mut AppState, uv: (f32, f32)| {
        let at = at_uv(s, rect, uv);
        yolu_app::region::idcolor::select_by_id(s, w, at);
    };
    // オプションバーで「足す」を選ぶと、キーの修飾が無くても足す（選択のツールと同じ値）
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(
        SelectionCombine::Add,
    ))));
    press(&mut s, TRI0);
    press(&mut s, TRI4);
    assert_eq!((selected(&s, TRI0), selected(&s, TRI4)), (255, 255));
    assert!(s.message.contains("追加"), "{}", s.message);
    // 「引く」なら、押した部品だけが外れる
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(
        SelectionCombine::Subtract,
    ))));
    press(&mut s, TRI0);
    assert_eq!((selected(&s, TRI0), selected(&s, TRI4)), (0, 255));
    // Shift は選んでいる方に関わらず足す
    s.region.modifiers = Modifiers {
        shift: true,
        ..Default::default()
    };
    press(&mut s, TRI0);
    assert_eq!((selected(&s, TRI0), selected(&s, TRI4)), (255, 255));
}

#[test]
fn headless_id_select_in_3d_reads_the_texel_of_the_face_under_the_pointer() {
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    bake_id(&mut s);
    let at = at_model(&s, rect, Vec3::new(3.5, 0.5, 0.0));
    yolu_app::region::idcolor::select_by_id(&mut s, Where::Surface(rect), at);
    assert_eq!(selected(&s, TRI4), 255);
    assert_eq!(selected(&s, TRI0), 0);
}

#[test]
fn headless_id_select_refuses_a_missing_or_stale_map_with_a_reason() {
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let at = at_uv(&s, rect, TRI0);
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(s.message, "ID マップがありません");
    assert!(s.doc.selection().is_none());
    // 焼いた後に設定が変わった（ID の分け方）
    bake_id(&mut s);
    s.bake.settings.id_source = MeshIdSource::MaterialSlot;
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(s.message, "ID マップが古いです（ベイクし直し）");
    assert!(s.doc.selection().is_none(), "古いマップでは選ばない");
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(s.message, "The ID map is stale (bake again)");
    // 焼いた後にモデルが替わった
    s.bake.settings.id_source = MeshIdSource::UvIsland;
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert!(
        s.doc.selection().is_some(),
        "設定を戻せば使える: {}",
        s.message
    );
    let mut other = two_parts_model();
    other.meshes[1].uvs[0] = Vec2::new(0.56, 0.05);
    let other = ViewModel::new("別", other.meshes.clone(), vec![Some("材".into())], 3).unwrap();
    s.view3d.set_model(other);
    s.apply(Action::Undo);
    assert!(s.doc.selection().is_none());
    yolu_app::region::idcolor::select_by_id(&mut s, w, at);
    assert_eq!(
        s.message, "The ID map is stale (bake again)",
        "モデルが替わった"
    );
    assert!(s.doc.selection().is_none());
}

#[test]
fn headless_manual_id_colors_reach_the_baked_id_map_and_make_the_old_one_stale() {
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    bake_id(&mut s);
    assert!(s.usable_id_map().is_ok());
    // 手動の色を直すと、前の ID マップは古い
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
        part: 1,
        rgb: Some(0x123456),
    })));
    assert!(
        s.usable_id_map().unwrap_err().contains("古い"),
        "{:?}",
        s.usable_id_map().err()
    );
    // 焼き直すと、B のアイランド（部品 1）がその色になり、押すとその色で選ぶ
    bake_id(&mut s);
    assert!(s.usable_id_map().is_ok());
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI4);
    yolu_app::region::idcolor::select_by_id(&mut s, Where::Canvas(&view), at);
    assert!(s.message.contains("#123456"), "{}", s.message);
    assert_eq!(selected(&s, TRI4), 255);
    // 自動に戻すと、また古い
    s.apply(Action::Region(RegionAction::IdColor(IdColorOp::ResetAll)));
    assert!(s.usable_id_map().is_err());
}

#[test]
fn headless_id_hover_highlights_the_triangles_of_the_color_under_the_pointer() {
    let (mut s, rect) = with_model(64);
    s.tool = Tool::IdSelect;
    bake_id(&mut s);
    let view = canvas_view(&s, rect);
    {
        let at = at_uv(&s, rect, TRI2);
        update_hover(&mut s, Where::Canvas(&view), Some(at));
    }
    assert_eq!(s.region_hover_len(), Some(2), "A の右のアイランドの三角形");
    {
        let at = at_uv(&s, rect, TRI4);
        update_hover(&mut s, Where::Canvas(&view), Some(at));
    }
    assert_eq!(s.region_hover_len(), Some(2), "B");
    {
        let at = at_uv(&s, rect, (0.8, 0.9));
        update_hover(&mut s, Where::Canvas(&view), Some(at));
    }
    assert!(s.region.hover.is_none());
}

#[test]
fn headless_id_hover_is_not_reused_after_another_tool_replaced_it() {
    let (mut s, rect) = with_model(64);
    bake_id(&mut s);
    let view = canvas_view(&s, rect);
    let w = Where::Canvas(&view);
    let at = at_uv(&s, rect, TRI2);
    // ID の色の強調 → バケツ（メッシュの塊）→ ID の色。ポインタは動かさない
    s.tool = Tool::IdSelect;
    update_hover(&mut s, w, Some(at));
    assert_eq!(
        s.region_hover_len(),
        Some(2),
        "A の右のアイランド（ID の色の範囲）"
    );
    s.tool = Tool::Fill;
    s.apply(Action::Region(RegionAction::FillRange(
        SurfaceRegionKind::MeshPart,
    )));
    update_hover(&mut s, w, Some(at));
    assert_eq!(s.region_hover_len(), Some(4), "バケツの範囲は A の全体");
    s.tool = Tool::IdSelect;
    update_hover(&mut s, w, Some(at));
    assert_eq!(
        s.region_hover_len(),
        Some(2),
        "ID の色に戻したら ID の色の範囲"
    );
    // 強調が消えたあとも、前の条件で作ったことにしない（バケツ → 消える → ID の色）
    s.tool = Tool::Brush;
    update_hover(&mut s, w, Some(at));
    assert!(s.region.hover.is_none());
    s.tool = Tool::IdSelect;
    update_hover(&mut s, w, Some(at));
    assert_eq!(s.region_hover_len(), Some(2));
    // 3D でも同じ（ID の色 → バケツ → ID の色）
    let rect3 = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    let on_face = at_model(&s, rect3, Vec3::new(0.1, 0.1, 0.0));
    let w3 = Where::Surface(rect3);
    update_hover(&mut s, w3, Some(on_face));
    let id_tris = s.region_hover_len().expect("3D の ID の色の強調");
    s.tool = Tool::Fill;
    update_hover(&mut s, w3, Some(on_face));
    assert_eq!(s.region_hover_len(), Some(4), "3D のバケツ（メッシュの塊）");
    s.tool = Tool::IdSelect;
    update_hover(&mut s, w3, Some(on_face));
    assert_eq!(s.region_hover_len(), Some(id_tris), "3D の ID の色に戻る");
}

// ───────── 画面（egui_kittest） ─────────

fn app_with_model(width: f32, height: f32, size: u32) -> Harness<'static, YoluApp> {
    model_app_from(app(width, height, size))
}

fn model_app_from(mut h: Harness<'static, YoluApp>) -> Harness<'static, YoluApp> {
    {
        let s = &mut h.state_mut().state;
        s.view3d.set_model(two_parts_model());
        s.view3d.material = 0;
        s.view3d.camera.yaw = 0.0;
        s.view3d.camera.pitch = 0.0;
        s.color.set_main([0.9, 0.3, 0.1, 1.0]);
    }
    h.run();
    h
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

#[test]
fn the_tools_have_buttons_keys_and_menu_entries() {
    let mut h = app(1280.0, 800.0, 128);
    for (key_, tool, modifiers) in [
        (Key::G, Tool::Fill, Modifiers::NONE),
        (Key::Num4, Tool::PolygonFill, Modifiers::NONE),
        (Key::W, Tool::IdSelect, Modifiers::SHIFT),
        // Shift なしの W は自動選択のまま（Shift+W を先に取っている）
        (Key::W, Tool::Wand, Modifiers::NONE),
        (Key::B, Tool::Brush, Modifiers::NONE),
    ] {
        key(&h, key_, modifiers);
        h.run();
        assert_eq!(h.state().state.tool, tool, "{tool:?}");
    }
    // ツールの帯のボタン（名前はツールチップ）
    for tool in [Tool::Fill, Tool::PolygonFill, Tool::IdSelect] {
        let label = format!("{}（{}）", tool.name_in(Lang::Ja), tool.key());
        h.get_by_label(&label).click();
        h.run();
        assert_eq!(h.state().state.tool, tool);
    }
    // 編集メニューの項目（描くツール）と、選択範囲メニューの項目（選ぶツールの ID の色で選択）
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    for tool in [Tool::Fill, Tool::PolygonFill] {
        let _ = popup_item(&h, tool.name_in(Lang::Ja));
    }
    click(&mut h, at);
    let at = menu_title(&h, "選択範囲").center();
    click(&mut h, at);
    let item = popup_item(&h, Tool::IdSelect.name_in(Lang::Ja)).center();
    click(&mut h, item);
    assert_eq!(h.state().state.tool, Tool::IdSelect);
}

#[test]
fn paint_channels_toggle_chips_and_values_change_the_state() {
    // ツールプロパティの「塗るチャンネル」（初めは閉じている）。入る高さで
    let mut h = app_with_model(1280.0, 1500.0, 128);
    let props = h.state().tab_rects[&yolu_app::Tab::ToolProperties];
    let size = h.state().tab_rects[&yolu_app::Tab::BrushSize];
    let in_props = move |r: Rect| {
        r.left() < 340.0 && r.top() > props.bottom() && r.bottom() < size.top() + 2.0
    };
    let header = rect_of(&h, "塗るチャンネル", in_props);
    click(&mut h, header.center());
    assert!(!h.state().state.mat.enabled);
    paint_channels_shot(&mut h, "region_paint_channels_off");
    let toggle = h.get_by_label("複数のチャンネルを一度に塗る");
    toggle.click();
    h.run();
    assert!(h.state().state.mat.enabled);
    assert_eq!(h.state().state.mat.included(), vec![Channel::Color]);
    // チップ（右の列にあるラフネス）で組に足す。最後の 1 つは外せない
    let chip = rect_of(&h, "ラフネス", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    assert_eq!(
        h.state().state.mat.included(),
        vec![Channel::Color, Channel::Roughness]
    );
    let chip = rect_of(&h, "カラー", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    let at = rect_of(&h, "ラフネス", |r| in_props(r) && r.height() < 30.0).center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.mat.included(),
        vec![Channel::Roughness],
        "最後の 1 つは外せない"
    );
    assert!(h.state().state.message.contains("1 つ以上"));
    // 値と見た目
    for c in [
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Normal,
        Channel::Emission,
    ] {
        let s = &mut h.state_mut().state;
        s.mat.set_enabled(true, Channel::Color);
        s.mat.set_channel(c, true);
    }
    h.state_mut().state.mat.set_normal(0.3, -0.2);
    h.state_mut().state.mat.emission = [0.2, 0.8, 0.4];
    h.run();
    paint_channels_shot(&mut h, "region_paint_channels_on");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    // 知らせは作ったときの言語の文のまま残るので、英語の見た目からは外す
    h.state_mut().state.message.clear();
    h.run();
    paint_channels_shot(&mut h, "region_paint_channels_on_english");
}

/// ツールプロパティのパネルの中だけを撮って、正解の絵と比べる。
fn paint_channels_shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    h.event(egui::Event::PointerGone);
    h.step();
    let props = h.state().tab_rects[&yolu_app::Tab::ToolProperties];
    let size = h.state().tab_rects[&yolu_app::Tab::BrushSize];
    let rect = Rect::from_min_max(
        pos2(props.left() - 2.0, props.top()),
        pos2(props.left() + 300.0, size.top()),
    );
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left() as u32,
        rect.top() as u32,
        rect.width() as u32,
        rect.height() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// ペンの 1 点（pointer_id は固定。位置はウィンドウの画素 = 点）。
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

/// 2D キャンバスで UV の点が見える画面の点（画面を回した後も）。
fn canvas_at(h: &Harness<'_, YoluApp>, uv: (f32, f32)) -> Pos2 {
    at_uv(&h.state().state, canvas_rect(h), uv)
}

/// 3D のタブを前に出し、モデルの点が見える画面の点を返す口を作る。
fn show_3d(h: &mut Harness<'_, YoluApp>) -> Rect {
    click_tab(h, yolu_app::Tab::View3d);
    h.run();
    h.state().view3d_rect().expect("3D のタブ")
}

/// 3D の A の左のアイランド・A の右のアイランド・B の上の点。
fn face_points(h: &Harness<'_, YoluApp>, rect: Rect) -> [Pos2; 3] {
    let s = &h.state().state;
    [
        at_model(s, rect, Vec3::new(0.1, 0.1, 0.0)),
        at_model(s, rect, Vec3::new(0.9, 0.5, 0.0)),
        at_model(s, rect, Vec3::new(3.5, 0.5, 0.0)),
    ]
}

/// バケツを 1 回押したのと同じ結果（不透明度 0.5 の A の左のアイランドの塗り: 画素のアルファ）。押し直したかどうかが重なりで見える。
fn single_fill_alpha() -> u8 {
    let (mut s, rect) = fill_state(SurfaceRegionKind::UvIsland);
    s.brush.opacity = 0.5;
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    bucket(&mut s, Where::Canvas(&view), at);
    px(&s, TRI0)[3]
}

fn bucket_app() -> Harness<'static, YoluApp> {
    let mut h = app_with_model(1280.0, 800.0, 256);
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    h.state_mut().state.brush.opacity = 0.5;
    h
}

fn assert_filled_once(h: &Harness<'_, YoluApp>, alpha: u8, what: &str) {
    let s = &h.state().state;
    assert_eq!(
        s.doc.undo_count(),
        1,
        "{what}: 1 回の Undo（{}）",
        s.message
    );
    assert_eq!(px(s, TRI0)[3], alpha, "{what}: 押し直して重ねていない");
    assert!(painted(s, TRI1), "{what}: 押したアイランド");
    assert!(
        !painted(s, TRI2) && !painted(s, TRI4),
        "{what}: 押したまま滑らせた先の範囲は足さない"
    );
    assert!(!s.is_stroking(), "{what}");
}

#[test]
fn bucket_click_in_the_canvas_fills_one_undo() {
    let mut h = app_with_model(1280.0, 800.0, 256);
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    h.snapshot("region_bucket_options");
    // A の左のアイランドを押す
    let at = canvas_at(&h, (0.15, 0.25));
    click(&mut h, at);
    assert_eq!(
        h.state().state.doc.undo_count(),
        1,
        "{}",
        h.state().state.message
    );
    assert!(painted(&h.state().state, TRI0) && painted(&h.state().state, TRI1));
    assert!(
        !painted(&h.state().state, TRI2),
        "UV アイランド: 継ぎ目の向こうは塗らない"
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!painted(&h.state().state, TRI0));
}

/// 線対称 6 本の定規を置き、近い色のバケツで 1 回押すと、6 つの写しの四角が塗られる（画面の絵は日英。ツールプロパティの切り替えも写る）。
#[test]
fn bucket_with_a_six_line_symmetry_ruler_fills_every_copy_and_snapshots_in_both_languages() {
    let mut h = app(1280.0, 1000.0, 256);
    let center = (128.0_f64, 128.0_f64);
    let at_polar = |degrees: f64| {
        let a = degrees.to_radians();
        (center.0 + 80.0 * a.cos(), center.1 + 80.0 * a.sin())
    };
    // 押した点（20 度）の写し 6 つと、写しでない四角（50 度）。四角は 24 画素四方
    let images: Vec<(f64, f64)> = [20.0, 140.0, 260.0, -20.0, 100.0, 220.0]
        .map(at_polar)
        .to_vec();
    let decoy = at_polar(50.0);
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        for &(x, y) in images.iter().chain([&decoy]) {
            for py in y as u32 - 12..y as u32 + 12 {
                for px in x as u32 - 12..x as u32 + 12 {
                    s.doc
                        .set_pixel(layer, px, py, Rgba8::new(230, 60, 60, 255))
                        .unwrap();
                }
            }
        }
        common::rulers::symmetry_2d(s, center, (1.0, 0.0), 6, true);
        s.color.set_main([0.2, 0.7, 0.3, 1.0]);
        s.apply(Action::SelectTool(Tool::Fill));
        s.apply(Action::SubTool(yolu_app::subtool::SubToolAction::Select(
            Tool::Fill,
            yolu_app::subtool::Key::Builtin("similar-colors"),
        )));
        s.rulers.selected = None;
    }
    h.run();
    let r = canvas_rect(&h);
    let view = h.state().state.view.view(r, 256, 256);
    let before = h.state().state.doc.undo_count();
    click(&mut h, view.to_screen(images[0].0, images[0].1));
    assert_eq!(
        h.state().state.doc.undo_count(),
        before + 1,
        "1 回の取り消し"
    );
    let green = |s: &AppState, (x, y): (f64, f64)| {
        let p = composite_pixel(&s.doc, x as u32, y as u32);
        p[1] > p[0] && p[3] == 255
    };
    for &p in &images {
        assert!(green(&h.state().state, p), "{p:?}");
    }
    assert!(!green(&h.state().state, decoy), "写しでない四角は塗らない");
    h.state_mut().state.message.clear();
    h.run();
    h.snapshot("region_bucket_symmetry_lines6");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    h.snapshot("region_bucket_symmetry_lines6_english");
}

/// 自動選択でも、線対称 6 本の写しの全部が 1 回で選ばれる（絵は日英）。
#[test]
fn wand_with_a_six_line_symmetry_ruler_selects_every_copy_and_snapshots_in_both_languages() {
    let mut h = app(1280.0, 800.0, 256);
    let center = (128.0_f64, 128.0_f64);
    let at_polar = |degrees: f64| {
        let a = degrees.to_radians();
        (center.0 + 80.0 * a.cos(), center.1 + 80.0 * a.sin())
    };
    let images: Vec<(f64, f64)> = [20.0, 140.0, 260.0, -20.0, 100.0, 220.0]
        .map(at_polar)
        .to_vec();
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        for &(x, y) in images.iter().chain([&at_polar(50.0)]) {
            for py in y as u32 - 12..y as u32 + 12 {
                for px in x as u32 - 12..x as u32 + 12 {
                    s.doc
                        .set_pixel(layer, px, py, Rgba8::new(230, 60, 60, 255))
                        .unwrap();
                }
            }
        }
        common::rulers::symmetry_2d(s, center, (1.0, 0.0), 6, true);
        s.apply(Action::SelectTool(Tool::Wand));
        s.sel.tolerance = 0;
        s.rulers.selected = None;
    }
    h.run();
    let r = canvas_rect(&h);
    let view = h.state().state.view.view(r, 256, 256);
    let before = h.state().state.doc.undo_count();
    click(&mut h, view.to_screen(images[0].0, images[0].1));
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), before + 1, "1 回の取り消し");
    let mask = s.doc.selection().expect("選択範囲");
    for &(x, y) in &images {
        assert_eq!(mask.amount(x as u32, y as u32), 255, "{x},{y}");
    }
    let (x, y) = at_polar(50.0);
    assert_eq!(mask.amount(x as u32, y as u32), 0, "写しでない四角");
    h.state_mut().state.message.clear();
    h.run();
    h.snapshot("selection_wand_symmetry_lines6");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    h.snapshot("selection_wand_symmetry_lines6_english");
}

#[test]
fn bucket_click_in_the_3d_view_fills_one_undo() {
    let mut h = bucket_app();
    let single = single_fill_alpha();
    let rect = show_3d(&mut h);
    let [a, _, _] = face_points(&h, rect);
    click(&mut h, a);
    assert_filled_once(&h, single, "3D のマウス");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!painted(&h.state().state, TRI0), "Undo で戻る");
}

#[test]
fn a_pen_held_on_the_bucket_fills_once_in_the_canvas_even_when_it_slides_over_other_regions() {
    let single = single_fill_alpha();
    let mut h = bucket_app();
    let (a, b, c) = (
        canvas_at(&h, TRI0),
        canvas_at(&h, TRI2),
        canvas_at(&h, TRI4),
    );
    // 押したまま 4 点（同じ所・別のアイランド・別の部品）。別のフレームで
    pen_frames(
        &mut h,
        &[(a, true), (a, true), (b, true), (c, true), (c, false)],
    );
    assert_filled_once(&h, single, "ペン（別のフレーム）");
    assert!(h.state().state.region.drag.is_none());
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!painted(&h.state().state, TRI0), "Undo で戻る");
    // 1 フレームにまとめて来ても同じ
    pen_one_frame(&mut h, &[(a, true), (b, true), (c, true), (c, false)]);
    assert_filled_once(&h, single, "ペン（1 フレーム）");
    // 離したあとに触れ直せば、また 1 回押したことになる（印は離したときに下りる）
    pen_frames(&mut h, &[(b, true), (b, true), (b, false)]);
    let s = &h.state().state;
    assert_eq!(
        s.doc.undo_count(),
        2,
        "触れ直すと次のアイランドを塗る: {}",
        s.message
    );
    assert!(painted(s, TRI2) && painted(s, TRI3));
    assert!(!painted(s, TRI4));
}

#[test]
fn a_pen_held_on_the_bucket_fills_once_in_the_3d_view_even_when_it_slides_over_other_regions() {
    let single = single_fill_alpha();
    let mut h = bucket_app();
    let rect = show_3d(&mut h);
    let [a, b, c] = face_points(&h, rect);
    pen_frames(
        &mut h,
        &[(a, true), (a, true), (b, true), (c, true), (c, false)],
    );
    assert_filled_once(&h, single, "3D のペン（別のフレーム）");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!painted(&h.state().state, TRI0), "Undo で戻る");
    pen_one_frame(&mut h, &[(a, true), (b, true), (c, true), (c, false)]);
    assert_filled_once(&h, single, "3D のペン（1 フレーム）");
    pen_frames(&mut h, &[(b, true), (b, true), (b, false)]);
    let s = &h.state().state;
    assert_eq!(
        s.doc.undo_count(),
        2,
        "触れ直すと次のアイランドを塗る: {}",
        s.message
    );
    assert!(painted(s, TRI2) && painted(s, TRI3));
}

#[derive(Clone, Copy, Debug)]
enum Source {
    Mouse,
    Pen,
}

/// 押して、点を 1 点ずつ別のフレームでなぞる。押したままの状態で返る。
fn hold_over(h: &mut Harness<'_, YoluApp>, source: Source, points: &[Pos2]) {
    match source {
        Source::Mouse => {
            press(h, points[0], egui::PointerButton::Primary);
            h.step();
            for p in &points[1..] {
                move_to(h, *p);
                h.step();
            }
        }
        Source::Pen => {
            for p in points {
                h.state().pen().push(pen_sample(*p, true));
                h.step();
            }
        }
    }
}

/// 押していたのを離す。
fn lift_at(h: &mut Harness<'_, YoluApp>, source: Source, at: Pos2) {
    match source {
        Source::Mouse => release(h, at, egui::PointerButton::Primary),
        Source::Pen => h.state().pen().push(pen_sample(at, false)),
    }
    h.step();
    h.run();
}

fn polygon_app() -> Harness<'static, YoluApp> {
    let mut h = app_with_model(1280.0, 800.0, 256);
    key(&h, Key::Num4, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::PolygonFill);
    h
}

/// 画面の 3 点（A の左のアイランド・A の右のアイランド・B）。3D なら 3D のタブを前に出す。
fn polygon_points(h: &mut Harness<'_, YoluApp>, in_3d: bool) -> [Pos2; 3] {
    if in_3d {
        let rect = show_3d(h);
        face_points(h, rect)
    } else {
        [canvas_at(h, TRI0), canvas_at(h, TRI2), canvas_at(h, TRI4)]
    }
}

/// ドラッグもストロークも残らず、Undo の段が `undo` であること。
fn assert_polygon_idle(h: &Harness<'_, YoluApp>, undo: usize, what: &str) {
    let s = &h.state().state;
    assert!(s.region.drag.is_none(), "{what}: ドラッグが残っている");
    assert!(!s.is_stroking(), "{what}: ストロークが残っている");
    assert_eq!(
        s.doc.undo_count(),
        undo,
        "{what}: Undo の段（{}）",
        s.message
    );
}

#[test]
fn polygon_fill_ends_cleanly_with_the_mouse_and_the_pen_in_the_canvas_and_the_3d_view() {
    let polygon_app = || {
        let mut h = model_app_from(common::app(1280.0, 800.0, 256));
        key(&h, Key::Num4, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.tool, Tool::PolygonFill);
        h
    };
    for in_3d in [false, true] {
        for source in [Source::Mouse, Source::Pen] {
            let what = format!("{} {source:?}", if in_3d { "3D" } else { "2D" });
            // なぞって離す（別のフレーム）: 1 回の Undo。途中はドラッグ中で、塗れている
            let mut h = polygon_app();
            let [a, b, c] = polygon_points(&mut h, in_3d);
            hold_over(&mut h, source, &[a, b, c]);
            assert!(
                h.state().state.region.drag.is_some(),
                "{what}: ドラッグの途中"
            );
            assert!(h.state().state.is_stroking(), "{what}");
            assert!(
                painted(&h.state().state, TRI2),
                "{what}: 途中でも塗れている"
            );
            lift_at(&mut h, source, c);
            assert_polygon_idle(&h, 1, &format!("{what} 離す"));
            let s = &h.state().state;
            assert!(
                painted(s, TRI0) && painted(s, TRI2) && painted(s, TRI4),
                "{what}: 通った範囲"
            );
            key(&h, Key::Z, Modifiers::COMMAND);
            h.run();
            assert!(
                !painted(&h.state().state, TRI0) && !painted(&h.state().state, TRI4),
                "{what}: Undo で戻る"
            );
            assert_polygon_idle(&h, 0, &format!("{what} Undo"));

            // 押したまま Esc: 捨てる（Undo 0 回）。そのあと離しても何も起きない
            let mut h = polygon_app();
            let [a, b, c] = polygon_points(&mut h, in_3d);
            hold_over(&mut h, source, &[a, b]);
            assert!(painted(&h.state().state, TRI2), "{what}: 途中は塗れている");
            key(&h, Key::Escape, Modifiers::NONE);
            h.step();
            assert_polygon_idle(&h, 0, &format!("{what} Esc"));
            assert!(
                !painted(&h.state().state, TRI0) && !painted(&h.state().state, TRI2),
                "{what}: Esc で捨てる"
            );
            lift_at(&mut h, source, c);
            assert_polygon_idle(&h, 0, &format!("{what} Esc のあと離す"));
            assert!(!painted(&h.state().state, TRI0), "{what}");

            // ウィンドウのフォーカスを失う: そこまでを確定する（1 回の Undo）
            let mut h = polygon_app();
            let [a, b, c] = polygon_points(&mut h, in_3d);
            hold_over(&mut h, source, &[a, b]);
            h.event(egui::Event::WindowFocused(false));
            h.step();
            assert_polygon_idle(&h, 1, &format!("{what} フォーカス喪失"));
            let s = &h.state().state;
            assert!(
                painted(s, TRI0) && painted(s, TRI2),
                "{what}: そこまでを確定"
            );
            lift_at(&mut h, source, c);
            assert_polygon_idle(&h, 1, &format!("{what} フォーカス喪失のあと離す"));
            assert!(
                !painted(&h.state().state, TRI4),
                "{what}: 離す前に確定した分だけ"
            );
        }
    }
}

#[test]
fn polygon_fill_pressed_and_released_within_one_frame_is_one_undo() {
    for in_3d in [false, true] {
        // マウス: 押す・動く・離すが 1 フレームに来ても、取り残さず 1 回の Undo
        let mut h = polygon_app();
        let [a, b, c] = polygon_points(&mut h, in_3d);
        press(&h, a, egui::PointerButton::Primary);
        move_to(&h, b);
        move_to(&h, c);
        release(&h, c, egui::PointerButton::Primary);
        h.step();
        h.run();
        assert_polygon_idle(&h, 1, &format!("マウス（1 フレーム） 3D={in_3d}"));
        assert!(painted(&h.state().state, TRI4), "押した所から離した所まで");
        // ペン: 1 フレームにまとめた点
        let mut h = polygon_app();
        let [a, b, c] = polygon_points(&mut h, in_3d);
        pen_one_frame(&mut h, &[(a, true), (b, true), (c, true), (c, false)]);
        assert_polygon_idle(&h, 1, &format!("ペン（1 フレーム） 3D={in_3d}"));
        let s = &h.state().state;
        assert!(painted(s, TRI0) && painted(s, TRI2) && painted(s, TRI4));
        // 離したあとに触れ直せば、また 1 つのドラッグ（印は離したときに下りる）
        pen_frames(&mut h, &[(b, true), (b, true), (b, false)]);
        assert_polygon_idle(&h, 2, &format!("ペンの触れ直し 3D={in_3d}"));
    }
}

fn id_app() -> Harness<'static, YoluApp> {
    let mut h = app_with_model(1280.0, 800.0, 256);
    bake_id(&mut h.state_mut().state);
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.tool, Tool::IdSelect);
    h
}

fn assert_selected_once(h: &Harness<'_, YoluApp>, what: &str) {
    let s = &h.state().state;
    assert_eq!(
        s.doc.undo_count(),
        1,
        "{what}: 1 回の Undo（{}）",
        s.message
    );
    assert_eq!(selected(s, TRI4), 255, "{what}: 押した部品");
    assert_eq!(selected(s, TRI0), 0, "{what}: 滑らせた先へ選び直さない");
    assert_eq!(selected(s, TRI2), 0, "{what}");
}

#[test]
fn id_select_by_the_mouse_and_the_held_pen_selects_once_in_the_canvas() {
    // マウス: 押した部品（B）を選ぶ
    let mut h = id_app();
    let at = canvas_at(&h, TRI4);
    click(&mut h, at);
    assert_selected_once(&h, "2D のマウス");
    // ペン: B を押したまま A へ滑らせても、選び直さない（Undo も 1 段）
    let mut h = id_app();
    let (b, a) = (canvas_at(&h, TRI4), canvas_at(&h, TRI0));
    pen_frames(
        &mut h,
        &[(b, true), (b, true), (a, true), (a, true), (a, false)],
    );
    assert_selected_once(&h, "2D のペン（別のフレーム）");
    let mut h = id_app();
    let (b, a) = (canvas_at(&h, TRI4), canvas_at(&h, TRI0));
    pen_one_frame(&mut h, &[(b, true), (a, true), (a, false)]);
    assert_selected_once(&h, "2D のペン（1 フレーム）");
    // 離したあとに触れ直せば、また選ぶ
    pen_frames(&mut h, &[(a, true), (a, true), (a, false)]);
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), 2, "{}", s.message);
    assert_eq!(
        (selected(s, TRI0), selected(s, TRI4)),
        (255, 0),
        "触れ直して A を選ぶ"
    );
}

#[test]
fn id_select_by_the_mouse_and_the_held_pen_selects_once_in_the_3d_view() {
    let mut h = id_app();
    let rect = show_3d(&mut h);
    let [_, _, b] = face_points(&h, rect);
    click(&mut h, b);
    assert_selected_once(&h, "3D のマウス");
    let mut h = id_app();
    let rect = show_3d(&mut h);
    let [a, _, b] = face_points(&h, rect);
    pen_frames(
        &mut h,
        &[(b, true), (b, true), (a, true), (a, true), (a, false)],
    );
    assert_selected_once(&h, "3D のペン（別のフレーム）");
    let mut h = id_app();
    let rect = show_3d(&mut h);
    let [a, _, b] = face_points(&h, rect);
    pen_one_frame(&mut h, &[(b, true), (a, true), (a, false)]);
    assert_selected_once(&h, "3D のペン（1 フレーム）");
    pen_frames(&mut h, &[(a, true), (a, true), (a, false)]);
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), 2, "{}", s.message);
    assert_eq!(
        (selected(s, TRI0), selected(s, TRI4)),
        (255, 0),
        "触れ直して A を選ぶ"
    );
}

/// レイヤーの Color と Roughness の、塗られた画素（アルファが 0 でない）の集合が同じで、数が多少ある。
fn painted_pixels(s: &AppState, channel: Channel) -> Vec<(u32, u32)> {
    let (w, hh) = (s.doc.width(), s.doc.height());
    let mut out = Vec::new();
    for y in 0..hh {
        for x in 0..w {
            if channel_pixel(s, channel, x, y).a > 0 {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn a_material_stroke_in_the_canvas_and_the_3d_view_paints_every_channel_in_one_undo() {
    let mut h = app_with_model(1280.0, 800.0, 256);
    {
        let s = &mut h.state_mut().state;
        s.apply(Action::Mat(MatAction::Enabled(true)));
        s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
        s.mat.set_scalar(Channel::Roughness, 1.0);
    }
    // 2D
    let c = canvas_rect(&h).center();
    drag(
        &mut h,
        &[
            offset(c, -60.0, 0.0),
            offset(c, -20.0, 0.0),
            offset(c, 40.0, 10.0),
        ],
    );
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), 1, "{}", s.message);
    let color = painted_pixels(s, Channel::Color);
    assert!(color.len() > 200, "{}", color.len());
    assert_eq!(
        color,
        painted_pixels(s, Channel::Roughness),
        "同じダブで両方のチャンネル（2D）"
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted_pixels(&h.state().state, Channel::Roughness).is_empty());
    // 3D（A の面をなぞる）
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let s = &h.state().state;
    let a = at_model(s, rect, Vec3::new(0.15, 0.3, 0.0));
    let b = at_model(s, rect, Vec3::new(0.85, 0.6, 0.0));
    drag(&mut h, &[a, b]);
    let s = &h.state().state;
    assert_eq!(
        s.doc.undo_count(),
        1,
        "2D の分は取り消した後。3D も 1 回の Undo: {}",
        s.message
    );
    let color = painted_pixels(s, Channel::Color);
    assert!(color.len() > 50, "{}", color.len());
    assert_eq!(
        color,
        painted_pixels(s, Channel::Roughness),
        "同じダブで両方のチャンネル（3D）"
    );
    assert_eq!(
        channel_pixel(s, Channel::Roughness, color[0].0, color[0].1).r,
        255
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    let s = &h.state().state;
    assert!(
        painted_pixels(s, Channel::Color).is_empty()
            && painted_pixels(s, Channel::Roughness).is_empty()
    );
}

#[test]
fn a_locked_layer_says_why_the_brush_cannot_start() {
    let mut h = app_with_model(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .doc
        .set_layer_locks(id, yolu_core::LayerLocks::PIXELS)
        .unwrap();
    let before = h.state().state.doc.undo_count();
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -40.0, 0.0), offset(c, 40.0, 0.0)]);
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), before, "何も描いていない");
    assert_eq!(
        s.message,
        "描けません（レイヤーの「画素」がロックされています）。"
    );
    h.snapshot("region_locked_refusal");
}

#[test]
fn polygon_fill_dragging_in_the_3d_view_is_one_undo_and_escape_cancels() {
    let mut h = app_with_model(1280.0, 800.0, 256);
    key(&h, Key::Num4, Modifiers::NONE);
    h.run();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let s = &h.state().state;
    let a = at_model(s, rect, Vec3::new(0.1, 0.1, 0.0));
    let b = at_model(s, rect, Vec3::new(0.9, 0.5, 0.0));
    let c = at_model(s, rect, Vec3::new(3.5, 0.5, 0.0));
    // なぞって離す
    drag(&mut h, &[a, b, c]);
    let s = &h.state().state;
    assert_eq!(s.doc.undo_count(), 1, "{}", s.message);
    for p in [TRI0, TRI2, TRI4] {
        assert!(painted(s, p), "{p:?}");
    }
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(!painted(&h.state().state, TRI4));
    // 押したまま Esc: 捨てる
    press(&h, a, egui::PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    assert!(
        painted(&h.state().state, TRI2),
        "ドラッグの途中は塗れている"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, b, egui::PointerButton::Primary);
    h.run();
    let s = &h.state().state;
    assert!(!painted(s, TRI0) && !painted(s, TRI2), "Esc で捨てる");
    assert_eq!(s.doc.undo_count(), 0);
    assert!(!s.is_stroking());
    // ウィンドウのフォーカスを失う: そこまでを確定する
    press(&h, a, egui::PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    h.event(egui::Event::WindowFocused(false));
    h.run();
    release(&h, b, egui::PointerButton::Primary);
    h.run();
    let s = &h.state().state;
    assert!(
        painted(s, TRI0) && painted(s, TRI2),
        "フォーカスを失うとそこまでを確定"
    );
    assert_eq!(s.doc.undo_count(), 1);
    assert!(!s.is_stroking());
}

#[test]
fn hover_highlight_in_the_3d_view_and_the_canvas_snapshots() {
    let mut h = app_with_model(1280.0, 800.0, 256);
    key(&h, Key::Num4, Modifiers::NONE);
    h.run();
    // 2D: ポインタの下の UV アイランドの輪郭
    let rect = canvas_rect(&h);
    let at = {
        let s = &h.state().state;
        s.view
            .view(rect, 256, 256)
            .to_screen(0.15 * 256.0, 0.25 * 256.0)
    };
    move_to(&h, at);
    h.run();
    assert_eq!(h.state().state.region_hover_len(), Some(2));
    h.snapshot("region_hover_canvas");
    // 3D
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let at = at_model(&h.state().state, rect, Vec3::new(0.1, 0.2, 0.0));
    move_to(&h, at);
    h.run();
    assert_eq!(h.state().state.region_hover_len(), Some(2));
    h.snapshot("region_hover_3d");
    // 消すなら桃色
    apply(&mut h, Action::Region(RegionAction::Erase(true)));
    h.snapshot("region_hover_3d_erase");
    // 面でない所では消える
    move_to(&h, rect.min + vec2(5.0, 5.0));
    h.run();
    assert!(h.state().state.region.hover.is_none());
}

#[test]
fn the_id_select_panel_shows_the_same_creation_band_as_the_selection_tools() {
    let mut h = app_with_model(1280.0, 800.0, 128);
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.tool, Tool::IdSelect);
    // 新規・追加・削除に「⋯」（共通は畳んでいる）。選択のツールと同じ帯
    for label in ["新規", "追加", "削除", "すべての作成方法"] {
        dock_rect(&h, label);
    }
    // （オプションバーの文字のボタンには「共通」がある。ここはドックの帯だけを見る）
    assert!(h
        .query_all_by_label("共通")
        .all(|n| n.rect().top() < 62.0 || n.rect().left() > 390.0));
    // 「⋯」で共通が出て、帯のボタンで作成方法が替わる（オプションバーの文字のボタンと同じ値）
    let more = dock_rect(&h, "すべての作成方法");
    click(&mut h, more.center());
    let intersect = dock_rect(&h, "共通");
    click(&mut h, intersect.center());
    assert_eq!(
        h.state().state.sel.combine,
        yolu_app::engine::SelectionCombine::Intersect
    );
}

#[test]
fn id_select_panel_and_the_options_bar_snapshots_in_both_languages() {
    let mut h = app_with_model(1280.0, 800.0, 128);
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    // 部品の欄はモデルの入力を別のスレッドで作り終えるまで「確かめています」なので、撮る前に作り終えさせる（撮る時機に頼らない）
    ready_parts(&mut h.state_mut().state);
    h.run();
    h.snapshot("region_idselect_no_map");
    {
        let s = &mut h.state_mut().state;
        s.apply(Action::Region(RegionAction::IdColor(IdColorOp::Set {
            part: 1,
            rgb: Some(0x3a7bd5),
        })));
        bake_id(s);
        // 焼いた時間（秒）が撮るたびに変わるので、ステータスバーの知らせは空にする
        s.message.clear();
    }
    h.run();
    h.snapshot("region_idselect_manual");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    h.snapshot("region_idselect_english");
    // ポリゴン塗りつぶしの帯とプロパティ（英語）
    key(&h, Key::Num4, Modifiers::NONE);
    h.run();
    h.snapshot("region_polygon_english");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::Ja)));
    h.snapshot("region_polygon");
}

#[test]
fn the_sub_tool_list_has_the_ranges_and_the_bucket_adds_similar_colors() {
    let mut h = app_with_model(1280.0, 800.0, 128);
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    // サブツールの一覧に範囲の種類が並ぶ（近い色はバケツだけ）。初めは UV アイランド
    for name in [
        "近い色",
        "三角形",
        "メッシュの塊",
        "UV アイランド",
        "マテリアル",
    ] {
        let _ = dock_rect(&h, name);
    }
    assert_eq!(h.state().state.region.kind, SurfaceRegionKind::UvIsland);
    let at = dock_rect(&h, "近い色").center();
    click(&mut h, at);
    assert!(h.state().state.region.by_color);
    // 範囲の種類を選ぶと、近い色はやめる
    let at = dock_rect(&h, "メッシュの塊").center();
    click(&mut h, at);
    assert!(!h.state().state.region.by_color);
    assert_eq!(h.state().state.region.kind, SurfaceRegionKind::MeshPart);
    // ポリゴン塗りつぶしには近い色が無く、自分の一覧の選び（初めは UV アイランド）
    key(&h, Key::Num4, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.region.kind, SurfaceRegionKind::UvIsland);
    let names: Vec<_> = ["近い色", "三角形"]
        .iter()
        .map(|n| h.query_all_by_label(n).count())
        .collect();
    assert!(names[1] > 0);
    assert_eq!(names[0], 0, "ポリゴン塗りつぶしの一覧に近い色は出ない");
    // バケツへ戻ると、バケツが最後に選んだ範囲
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.region.kind, SurfaceRegionKind::MeshPart);
}

/// ID の色で選択のツールを選び、部品が求まるまで待った画面。
fn id_panel(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app_with_model(1280.0, 800.0, 128);
    h.state_mut().state.lang = lang;
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.tool, Tool::IdSelect);
    ready_parts(&mut h.state_mut().state);
    h.run();
    h
}

fn manual_color(h: &Harness<'_, YoluApp>, part: usize) -> Option<u32> {
    h.state().state.doc.id_colors().colors().get(&part).copied()
}

/// 部品の ID の色の見本を押すと色のウィンドウが出て、円・四角のドラッグがその場で入り、1 回の取り消しになる（ドラッグの間は知らせを出さず、
/// 離して 1 回）。自動の色を見せているときに開いて Esc を押すと自動へ戻し、手動の色から開いたときは、その色へ戻す。ウィンドウが部品の色を
/// 相手にしている間に部品を替えると、ウィンドウはそのままで相手が新しい部品へ替わる。
#[test]
fn the_part_id_color_opens_the_color_window_and_escape_goes_back_to_automatic() {
    use egui::PointerButton;
    use yolu_app::panels::color::wheel_square;
    use yolu_app::panels::color_window::{self, wheel_of};
    let mut h = id_panel(Lang::Ja);
    let steps = h.state().state.doc.undo_count();
    let doc = h.state().state.doc.id();
    let target = |part: usize| egui::Id::new(("id.part.color", doc, part));
    let swatch = h.get_by_label("この部品の ID の色").rect();
    click(&mut h, swatch.center());
    assert!(color_window::is_target(&h.ctx, target(0)));
    let window = color_window::rect(&h.ctx).expect("色のウィンドウ");
    let sq = wheel_square(wheel_of(window));
    // ドラッグの途中: その場で入るが、知らせは出さない
    h.state_mut().state.message.clear();
    let end = sq.right_bottom() - vec2(8.0, 30.0);
    press(&h, sq.left_top() + vec2(8.0, 8.0), PointerButton::Primary);
    h.step();
    for p in [sq.center(), end] {
        move_to(&h, p);
        h.step();
    }
    h.step();
    let dragged = manual_color(&h, 0);
    assert!(dragged.is_some(), "その場で入る");
    assert!(
        !h.state().state.message.contains("変えました"),
        "ドラッグの間は知らせない: {}",
        h.state().state.message
    );
    release(&h, end, PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(manual_color(&h, 0), dragged);
    assert_eq!(
        h.state().state.doc.undo_count(),
        steps + 1,
        "ドラッグは 1 回の取り消し"
    );
    assert!(
        h.state()
            .state
            .message
            .contains("手動の ID の色を変えました"),
        "離して 1 回知らせる: {}",
        h.state().state.message
    );
    // Esc: 開いたときの「自動」へ戻して閉じる（戻すのも 1 回の取り消し）
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(manual_color(&h, 0), None, "自動へ戻る");
    assert!(!color_window::is_open(&h.ctx));
    assert_eq!(h.state().state.doc.undo_count(), steps + 2);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(manual_color(&h, 0), dragged);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(manual_color(&h, 0), None);
    // 手動の色から開いて Esc: その手動の色へ戻す（自動にはしない）
    apply(
        &mut h,
        Action::Region(RegionAction::IdColor(IdColorOp::Set {
            part: 0,
            rgb: Some(0x336699),
        })),
    );
    click(&mut h, swatch.center());
    click(&mut h, sq.left_top() + vec2(6.0, 6.0));
    assert_ne!(manual_color(&h, 0), Some(0x336699));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(manual_color(&h, 0), Some(0x336699));
    // 部品を替えると、ウィンドウはそのままで相手が新しい部品へ替わる
    click(&mut h, swatch.center());
    assert!(color_window::is_target(&h.ctx, target(0)));
    let placed = color_window::rect(&h.ctx).expect("色のウィンドウ");
    apply(&mut h, Action::Region(RegionAction::IdPart(1)));
    h.run();
    assert!(color_window::is_target(&h.ctx, target(1)));
    assert_eq!(
        color_window::rect(&h.ctx),
        Some(placed),
        "ウィンドウは動かない"
    );
    click(&mut h, sq.center());
    assert!(manual_color(&h, 1).is_some());
    assert_eq!(manual_color(&h, 0), Some(0x336699), "前の部品は変えない");
}

/// 部品の ID の色のウィンドウの円を押したまま Esc: 押したあとのドラッグは取り消され（まとめていた段ごと捨てる）、「取り消しました。」が残る。
/// ウィンドウの戻しが次のフレームに届いても、変えていないのに「手動の ID の色を変えました。」を出して上書きしない。
#[test]
fn escape_while_dragging_the_part_id_color_in_the_window_does_not_report_a_change() {
    use egui::PointerButton;
    use yolu_app::panels::color::wheel_square;
    use yolu_app::panels::color_window::{self, wheel_of};
    let mut h = id_panel(Lang::Ja);
    let steps = h.state().state.doc.undo_count();
    let swatch = h.get_by_label("この部品の ID の色").rect();
    click(&mut h, swatch.center());
    let sq = wheel_square(wheel_of(
        color_window::rect(&h.ctx).expect("色のウィンドウ"),
    ));
    h.state_mut().state.message.clear();
    press(&h, sq.left_top() + vec2(8.0, 8.0), PointerButton::Primary);
    h.step();
    for p in [sq.center(), sq.right_bottom() - vec2(8.0, 30.0)] {
        move_to(&h, p);
        h.step();
    }
    h.step();
    assert!(
        manual_color(&h, 0).is_some(),
        "ドラッグの途中はその場で入る"
    );
    // 押したまま Esc
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    h.step();
    release(
        &h,
        sq.right_bottom() - vec2(8.0, 30.0),
        PointerButton::Primary,
    );
    h.run();
    assert_eq!(manual_color(&h, 0), None, "自動へ戻る");
    assert_eq!(h.state().state.doc.undo_count(), steps, "段は残さない");
    assert!(
        !h.state().state.message.contains("変えました"),
        "変えていないので知らせない: {}",
        h.state().state.message
    );
    assert!(
        h.state().state.message.contains("取り消しました"),
        "取り消した知らせが残る: {}",
        h.state().state.message
    );
}

/// 部品の ID の色から開いた色のウィンドウの絵（日英）。
#[test]
fn snapshot_the_colour_window_on_a_part_id_color() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name, label) in [
        (Lang::Ja, "ja", "この部品の ID の色"),
        (Lang::En, "en", "This part's ID color"),
    ] {
        let mut h = id_panel(lang);
        let swatch = h.get_by_label(label).rect();
        click(&mut h, swatch.center());
        assert!(yolu_app::panels::color_window::is_open(&h.ctx));
        h.state_mut().state.message.clear();
        h.run();
        h.snapshot(format!("color_window_part_id_{name}"));
        results.extend_harness(&mut h);
    }
}

#[test]
fn the_id_panel_shows_the_part_position_of_the_second_texture_set() {
    let mut h = app_with_model(1280.0, 800.0, 128);
    h.state_mut()
        .load_live_link_model(&two_materials_link_model())
        .unwrap();
    h.run();
    {
        let s = &mut h.state_mut().state;
        let uid = s.sets.iter().nth(1).expect("2 つ目のセット").uid;
        s.apply(Action::SelectSet(uid));
    }
    h.run();
    assert_eq!(h.state().state.view3d.material, 1);
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.tool, Tool::IdSelect);
    ready_parts(&mut h.state_mut().state);
    apply(&mut h, Action::Region(RegionAction::IdPart(1)));
    h.state_mut().state.message.clear();
    h.run();
    h.snapshot("region_idselect_second_set");
}

#[test]
fn the_options_bar_of_each_range_tool_fits_a_narrow_window_in_both_languages() {
    let mut h = app_with_model(1000.0, 700.0, 128);
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    apply(&mut h, Action::Region(RegionAction::ByColor(true)));
    h.snapshot("region_bucket_by_color_narrow");
    apply(&mut h, Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    h.snapshot("region_bucket_by_color_narrow_english");
    key(&h, Key::W, Modifiers::SHIFT);
    h.run();
    ready_parts(&mut h.state_mut().state);
    h.run();
    h.snapshot("region_idselect_narrow_english");
}

#[test]
fn headless_bucket_surface_region_ignores_color_reference_and_gap_but_scales_area() {
    let (mut s, rect) = fill_state(SurfaceRegionKind::Triangle);
    let view = canvas_view(&s, rect);
    let at = at_uv(&s, rect, TRI0);
    s.region.color.reference = yolu_app::region::color::Reference::Marked;
    s.region.color.gap = 32;
    s.region.color.margin = -200;
    let before = s.doc.undo_count();
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), before);
    s.region.color.margin = 0;
    bucket(&mut s, Where::Canvas(&view), at);
    assert_eq!(s.doc.undo_count(), before + 1);
    assert!(painted(&s, TRI0));
}
