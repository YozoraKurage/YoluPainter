//! 塗りつぶしレイヤーの画像と投影・デカール・形のギズモ・グラデーションのツールを、ウィンドウ（egui_kittest。描画は wgpu のソフトの描画）で本物のポインタとキーで
//! 操作する試験。画面なしの試験は `fillfx.rs`。マップ（位置・法線）は、立方体を CPU で焼いた結果を文書の効果の入力へ足して使う。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::engine::{Channel, LayerId};
use yolu_app::fillfx::{gizmo, inputs, FillOp};
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::view3d::shape_gizmo::Handle;
use yolu_app::{Tab, YoluApp};
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::MapState;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{ImageId, MapInput};

fn quad() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ]
}

fn st<'a>(h: &'a Harness<'_, YoluApp>) -> &'a AppState {
    &h.state().state
}

fn apply(h: &mut Harness<'_, YoluApp>, a: Action) {
    h.state_mut().state.apply(a);
    h.run();
}

fn undo(h: &mut Harness<'_, YoluApp>) {
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
}

/// 試しの立方体を焼いて（位置・法線）文書の入力へ足し、3D のタブを前へ出したウィンドウ。右（+X）と手前（−Z）の面が見えるカメラ。
fn window() -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app(1280.0, 1500.0, 64);
    {
        let s = &mut h.state_mut().state;
        s.bake.backend = BakeBackend::Cpu;
        s.apply(Action::LoadDemoModel);
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
        s.bake.settings.padding = 4;
        s.apply(Action::Bake(BakeAction::Start));
        s.wait_bake();
        let mut inputs = s.doc.effect_inputs().clone();
        for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
            let map = s.sets.current().mesh_maps.get(kind).unwrap().clone();
            inputs = inputs
                .with_map(MapInput::from_baked(&map, MapState::Current).unwrap())
                .unwrap();
        }
        s.doc.set_effect_inputs(inputs).unwrap();
        s.view3d.camera.yaw = -40.0;
        s.view3d.camera.pitch = 15.0;
    }
    click_tab(&mut h, Tab::View3d);
    // 右上の組はテクスチャセットが前なので、棚（アセット）のタブを前へ出す
    click_tab(&mut h, Tab::Assets);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

fn shelf_image(h: &mut Harness<'_, YoluApp>, name: &str) -> (String, ImageId) {
    let rid = h
        .state_mut()
        .state
        .shelf
        .add_image(yolu_app::lang::Lang::Ja, name, &quad(), 2, 2)
        .unwrap();
    h.run();
    (rid.clone(), inputs::image_id(&rid).unwrap())
}

fn layer_projection(h: &Harness<'_, YoluApp>, id: LayerId) -> Projection {
    *st(h).doc.layer(id).unwrap().projection()
}

/// 右上の組（アセット）の格子の中の素材。
fn card(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
    let body = top_right_body(h);
    rect_of(h, name, |r| body.contains(r.center()))
}

#[test]
fn dragging_a_shelf_image_onto_the_3d_model_places_a_decal_and_one_undo_removes_it() {
    let (mut h, rect) = window();
    let (_, image) = shelf_image(&mut h, "石の模様");
    let layers = st(&h).doc.layers().len();
    let from = card(&h, "石の模様").center();
    let to = rect.center();
    drag(
        &mut h,
        &[from, from + vec2(30.0, 10.0), to + vec2(0.0, -40.0), to],
    );
    assert_eq!(st(&h).doc.layers().len(), layers + 1, "{}", st(&h).message);
    let id = st(&h).selected_layer.unwrap();
    let layer = st(&h).doc.layer(id).unwrap();
    assert_eq!(layer.fill_image(Channel::Color), Some(image));
    assert_eq!(layer.projection().mode, ProjectionMode::Decal);
    assert!(st(&h).message.contains("石の模様"), "{}", st(&h).message);
    assert!(
        !egui::DragAndDrop::has_any_payload(&h.ctx),
        "ドラッグの荷物は残さない"
    );
    // 置いたレイヤーのギズモ（デカールの箱）が 3D ビューに出ている
    assert_eq!(gizmo::target(st(&h)), Some(gizmo::Target::Projection(id)));
    assert!(gizmo::handle_point(st(&h), rect, Handle::MoveFree).is_some());
    undo(&mut h);
    assert_eq!(st(&h).doc.layers().len(), layers);
    // 3D ビューの外へ落としても何も置かない
    let canvas_tab = h.state().tab_rects.get(&Tab::Canvas).unwrap().center();
    let outside = pos2(canvas_tab.x, rect.bottom() + 5.0);
    drag(&mut h, &[from, from + vec2(30.0, 10.0), outside]);
    assert_eq!(st(&h).doc.layers().len(), layers);
}

#[test]
fn dragging_a_shelf_image_onto_the_image_box_sets_it_and_the_box_opens_the_list() {
    let (mut h, _) = window();
    let (rid, image) = shelf_image(&mut h, "石の模様");
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    assert!(st(&h).fill_image_problem(layer, Channel::Color).is_none());
    // 画像の箱（右の列。同じ名前の格子の素材は右上の組）。まだ画像が無いので名前は「画像」。欄は縦に長く、箱がウィンドウの下端の外に
    // 出ることがあるので、右の列を箱が見える所まで送ってから位置を取る
    let in_column = |r: Rect| r.left() > rx() && r.height() < 40.0 && r.width() > 100.0;
    let first = rect_of(&h, "画像", in_column);
    let scroll = st(&h).m2.props_scroll + (first.top() - props_mid(&h)).max(0.0);
    h.state_mut().state.m2.props_scroll = scroll;
    h.run();
    let boxed = rect_of(&h, "画像", in_column);
    // 押すと一覧
    click(&mut h, boxed.center());
    assert!(st(&h).popup.is_some(), "画像の一覧が開く");
    let item = popup_item(&h, "石の模様  (2 × 2)");
    click(&mut h, item.center());
    assert_eq!(
        st(&h).doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(image),
        "{}",
        st(&h).message
    );
    undo(&mut h);
    assert_eq!(
        st(&h).doc.layer(layer).unwrap().fill_image(Channel::Color),
        None
    );
    // 棚の画像をドラッグして落とす
    let boxed = rect_of(&h, "画像", |r| {
        r.left() > rx() && r.height() < 40.0 && r.width() > 100.0
    });
    let from = card(&h, "石の模様").center();
    drag(
        &mut h,
        &[
            from,
            from + vec2(30.0, 10.0),
            boxed.center() + vec2(0.0, -30.0),
            boxed.center(),
        ],
    );
    assert_eq!(
        st(&h).doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(image),
        "{}",
        st(&h).message
    );
    assert!(!egui::DragAndDrop::has_any_payload(&h.ctx));
    // 画像の箱の名前は画像の名前になり、外すボタンで戻る
    let boxed = rect_of(&h, "石の模様", |r| {
        r.left() > rx() && r.height() < 40.0 && r.width() > 100.0
    });
    let clear = pos2(boxed.right() + 14.0, boxed.center().y);
    click(&mut h, clear);
    assert_eq!(
        st(&h).doc.layer(layer).unwrap().fill_image(Channel::Color),
        None
    );
    let _ = rid;
}

#[test]
fn the_fill_image_box_gets_its_picture_from_another_thread_even_with_the_shelf_tab_closed() {
    let (mut h, _) = window();
    // 右上の組をテクスチャセットへ（棚の格子が絵を頼まない）。別のスレッドは、頼まれても止めておく
    click_tab(&mut h, Tab::TextureSets);
    h.run();
    h.state_mut().state.shelf.hold_inspections(true);
    let (rid, image) = shelf_image(&mut h, "石の模様");
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::Fill(FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        }),
    );
    // 画像の箱は名前を出し、絵はまだ作っていない（画面のスレッドでは作らない）。頼んだ仕事が別のスレッドに積まれている
    let _ = rect_of(&h, "石の模様", |r| {
        r.left() > rx() && r.height() < 40.0 && r.width() > 100.0
    });
    for _ in 0..5 {
        h.step();
    }
    assert!(
        st(&h).shelf.info(&rid).is_none(),
        "画面のスレッドで展開しない"
    );
    assert!(st(&h).shelf.inspections_pending(), "別のスレッドへ頼んだ");
    // 別のスレッドが終わると、箱の絵になる（受け取りは棚のタブが閉じていても毎フレーム）
    h.state_mut().state.shelf.hold_inspections(false);
    let started = std::time::Instant::now();
    while st(&h).shelf.info(&rid).is_none() {
        assert!(started.elapsed().as_secs() < 30, "絵ができない");
        std::thread::sleep(std::time::Duration::from_millis(2));
        h.step();
    }
    h.run();
    assert!(!st(&h).shelf.inspections_pending());
    let ctx = h.ctx.clone();
    assert!(h.state_mut().state.shelf.texture(&ctx, &rid).is_some());
}

#[test]
fn dragging_a_gizmo_handle_in_the_3d_view_moves_the_box_in_one_undo_and_escape_puts_it_back() {
    let (mut h, rect) = window();
    let (_, image) = shelf_image(&mut h, "石の模様");
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::Fill(FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        }),
    );
    apply(
        &mut h,
        Action::Fill(FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Planar,
        }),
    );
    let start = layer_projection(&h, layer);
    let from = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    let steps = st(&h).doc.undo_count();
    let target = from + vec2(60.0, 0.0);
    drag(
        &mut h,
        &[from, from + vec2(20.0, 0.0), from + vec2(40.0, 0.0), target],
    );
    let moved = layer_projection(&h, layer);
    assert!(
        moved.placement.center[0].abs() > 0.01,
        "{:?}",
        moved.placement.center
    );
    assert!(moved.placement.center[1].abs() < 1e-6, "X の軸だけ動く");
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "1 回の Undo");
    assert!(!gizmo::dragging(st(&h)));
    undo(&mut h);
    assert_eq!(layer_projection(&h, layer), start);
    // Esc: ドラッグの途中で押すと、前へ戻して履歴にも残さない
    let steps = st(&h).doc.undo_count();
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, from + vec2(30.0, 0.0));
    h.step();
    move_to(&h, from + vec2(55.0, 0.0));
    h.step();
    assert!(gizmo::dragging(st(&h)));
    assert_ne!(layer_projection(&h, layer), start);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(layer_projection(&h, layer), start);
    assert!(!gizmo::dragging(st(&h)));
    release(&h, from + vec2(55.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(layer_projection(&h, layer), start, "離しても動かない");
    assert_eq!(st(&h).doc.undo_count(), steps);
    // ウィンドウがフォーカスを失ったら、そこまでを捨てる
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, from + vec2(30.0, 0.0));
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(layer_projection(&h, layer), start);
    assert!(!gizmo::dragging(st(&h)));
    release(&h, from + vec2(30.0, 0.0), PointerButton::Primary);
    h.run();
    // ハンドルの無い所の押下は今のツールへ（塗りつぶしレイヤーには描けないので理由が出る）
    let away = rect.min + vec2(8.0, 8.0);
    click(&mut h, away);
    assert_eq!(layer_projection(&h, layer), start);
}

#[test]
fn while_dragging_the_gizmo_the_3d_view_shows_a_coarse_picture_and_release_makes_it_exact() {
    use yolu_app::view3d::paint::{Slot, DRAG_STRIDE};
    let (mut h, rect) = window();
    let (_, image) = shelf_image(&mut h, "石の模様");
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::Fill(FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        }),
    );
    apply(
        &mut h,
        Action::Fill(FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Planar,
        }),
    );
    let level0 = |h: &Harness<'_, YoluApp>| {
        h.state()
            .view3d_read_paint_level(Slot::Color, 0)
            .expect("3D の絵")
    };
    let before = level0(&h);
    assert_eq!(h.state().view3d_stats().unwrap().paint_coarse_tiles, 0);
    let from = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, from + vec2(25.0, 0.0));
    h.step();
    h.step();
    assert!(gizmo::dragging(st(&h)));
    assert!(st(&h).doc.is_coalescing());
    let stats = h.state().view3d_stats().unwrap();
    assert!(
        stats.paint_coarse_tiles > 0,
        "ドラッグの間は粗い絵: {stats:?}"
    );
    // 粗い絵は歩幅の箱ごとに同じテクセル（1/4 の大きさで合成して広げた）
    let (coarse, size) = level0(&h);
    assert_ne!(coarse, before.0, "ドラッグで絵が動く");
    let s = DRAG_STRIDE as usize;
    for y in 0..size[1] as usize {
        for x in 0..size[0] as usize {
            let at = |x: usize, y: usize| &coarse[(y * size[0] as usize + x) * 4..][..4];
            assert_eq!(at(x, y), at(x / s * s, y / s * s), "({x}, {y})");
        }
    }
    release(&h, from + vec2(25.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(!gizmo::dragging(st(&h)));
    let stats = h.state().view3d_stats().unwrap();
    assert_eq!(
        stats.paint_coarse_tiles, 0,
        "離したら正確に上げ直す: {stats:?}"
    );
    let exact = level0(&h);
    assert_ne!(exact.0, coarse);
    // 作り直した絵と同じバイト
    h.state_mut().view3d_invalidate_paint();
    h.run();
    assert_eq!(level0(&h), exact, "離した後の絵は全部を作り直した絵と同じ");
}

#[test]
fn q_hides_and_shows_the_handles_and_the_gizmo_shows_move_and_rotate_together() {
    let (mut h, rect) = window();
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::Fill(FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Triplanar,
        }),
    );
    assert!(gizmo::target(st(&h)).is_some());
    key(&h, Key::Q, Modifiers::NONE);
    h.run();
    assert!(st(&h).fillfx.handles_hidden && gizmo::target(st(&h)).is_none());
    key(&h, Key::Q, Modifiers::NONE);
    h.run();
    assert!(gizmo::target(st(&h)).is_some());
    // 移動と回転のハンドルは 1 つのギズモに同時に出て、切り替えのボタンは無い
    for handle in [
        Handle::MoveX,
        Handle::MoveFree,
        Handle::RotateY,
        Handle::SizeXPos,
    ] {
        assert!(
            gizmo::handle_point(st(&h), rect, handle).is_some(),
            "{handle:?}"
        );
    }
    h.state_mut().state.m2.props_scroll = 330.0;
    h.run();
    for label in ["移動", "回転"] {
        assert!(
            !h.query_all_by_label(label).any(|n| n.rect().left() > rx()
                && n.accesskit_node().role() == egui::accesskit::Role::Button),
            "欄に「{label}」の切り替えが残っている"
        );
    }
    // 3D ビューを描いた絵に形の線が重なる（外形の橙がある）
    let image = h.render().expect("描ける");
    let mut orange = 0;
    for y in (rect.top() as u32..rect.bottom() as u32).step_by(2) {
        for x in (rect.left() as u32..rect.right() as u32).step_by(2) {
            let p = image.get_pixel(x, y).0;
            if p[0] > 200 && p[1] > 100 && p[1] < 190 && p[2] < 90 {
                orange += 1;
            }
        }
    }
    assert!(orange > 30, "形の外形の線が出る: {orange}");
    // ポインタを何も無い所へ置いて、押したあとのホバーの動き・ツールチップが絵に入らないようにしてから撮る
    move_to(&h, rect.min + vec2(14.0, 40.0));
    // 焼いた時間を載せた知らせは撮るたびに変わる（状態の帯に出る）ので消す
    h.state_mut().state.message.clear();
    h.run();
    // 棚の素材の絵は別のスレッドで作る。できるまで待ってから撮る
    h.state_mut().state.shelf.wait_inspections();
    h.run();
    h.snapshot("fillfx_gizmo_3d");
}

#[test]
fn the_gradient_tool_paints_with_a_real_drag_on_the_canvas_and_shift_g_selects_it() {
    let mut h = app(1280.0, 1400.0, 64);
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    key(&h, Key::G, Modifiers::SHIFT);
    h.run();
    assert_eq!(st(&h).tool, Tool::Gradient);
    let canvas = canvas_rect(&h);
    let a = canvas.center() + vec2(-120.0, 0.0);
    let b = canvas.center() + vec2(120.0, 0.0);
    let steps = st(&h).doc.undo_count();
    drag(&mut h, &[a, a + vec2(30.0, 0.0), a + vec2(100.0, 0.0), b]);
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "{}", st(&h).message);
    let left = canvas_pixel(&h, a + vec2(2.0, 0.0));
    let right = canvas_pixel(&h, b - vec2(2.0, 0.0));
    assert!(left[0] > 200 && left[3] > 200, "始点の側は描画色: {left:?}");
    assert!(right[3] < 60, "終点の側は透明: {right:?}");
    assert!(st(&h).gradient.drag.is_none());
    undo(&mut h);
    assert_eq!(canvas_pixel(&h, a)[3], 0);
    // Esc: ドラッグの途中でやめる（何も塗らない）
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, a + vec2(80.0, 0.0));
    h.step();
    assert!(st(&h).gradient.drag.is_some());
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert!(st(&h).gradient.drag.is_none());
    release(&h, a + vec2(80.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), steps, "何も塗らない");
    // ツールの帯のボタンと英語の名前
    apply(&mut h, Action::SelectTool(Tool::Brush));
    h.get_by_label("グラデーション（Shift+G）").click();
    h.run();
    assert_eq!(st(&h).tool, Tool::Gradient);
    apply(
        &mut h,
        Action::M2Ui(yolu_app::m2::UiOp::Language(yolu_app::lang::Lang::En)),
    );
    assert!(h.query_by_label("Gradient (Shift+G)").is_some());
    h.snapshot("fillfx_gradient_tool_options");
    // 3D ビューでは使わない
    click_tab(&mut h, Tab::View3d);
    st(&h);
    let _: Pos2 = pos2(0.0, 0.0);
}

#[test]
fn the_fill_panel_draws_in_both_languages_without_clipped_text() {
    let mut results = SnapshotResults::new();
    for lang in yolu_app::lang::Lang::ALL {
        let (mut h, _) = window();
        {
            let s = &mut h.state_mut().state;
            s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
            s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
            let layer = s.selected_layer.unwrap();
            let rid = s.shelf.add_image(lang, "tile", &quad(), 2, 2).unwrap();
            let id = inputs::image_id(&rid).unwrap();
            s.apply(Action::Fill(FillOp::Image {
                layer,
                channel: Channel::Color,
                image: Some(id),
            }));
            s.apply(Action::Fill(FillOp::ProjectionMode {
                layer,
                mode: ProjectionMode::Decal,
            }));
            s.apply(Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(
                Channel::Roughness,
            )));
            s.apply(Action::Fill(FillOp::AddGradient {
                layer,
                channel: Channel::Roughness,
            }));
            for key in ["fill-image", "fill-projection", "fill-gradient"] {
                s.ui.sections.insert(key, true);
            }
            s.m2.props_scroll = 0.0;
        }
        h.run();
        let want = lang.pick("投影", "Projection");
        assert!(
            h.query_all_by_label(want).next().is_some(),
            "{lang:?}: {want}"
        );
        // 描いた全部の文字が、その部品の描く範囲の中に収まる
        let mut clipped = Vec::new();
        for shape in &h.output().shapes {
            collect_clipped(&shape.shape, shape.clip_rect, &mut clipped);
        }
        assert!(clipped.is_empty(), "{lang:?}: {clipped:?}");
        // 英語の画面に日本語が残らない（画面の文言。ユーザーの付けた名前を除く）
        if lang == yolu_app::lang::Lang::En {
            for shape in &h.output().shapes {
                let mut texts = Vec::new();
                collect_texts(&shape.shape, &mut texts);
                for t in texts {
                    assert!(
                        !common::has_japanese(&t) || t.contains("tile"),
                        "英語の画面に日本語: {t}"
                    );
                }
            }
        }
        for (k, scroll) in [0.0f32, 420.0, 900.0].into_iter().enumerate() {
            h.state_mut().state.message.clear();
            h.state_mut().state.m2.props_scroll = scroll;
            h.run();
            // 右の列（プロパティ）に描いた文字は、行の名前・状態・短い理由だけ（使い方の説明・開発用の数・長い文が無い）
            let mut shown = 0;
            for shape in &h.output().shapes {
                let mut texts = Vec::new();
                collect_texts_at(&shape.shape, &mut texts);
                for (at, text) in texts {
                    if at.x > rx() + 20.0
                        && shape
                            .clip_rect
                            .intersects(Rect::from_min_size(at, vec2(1.0, 1.0)))
                    {
                        common::assert_plain(&format!("{lang:?} 欄 {k}"), &text);
                        shown += 1;
                    }
                }
            }
            assert!(
                shown > 15,
                "{lang:?} {k}: 欄の文字を集められていない（{shown}）"
            );
            // 棚の素材の絵は別のスレッドで作る。できるまで待ってから撮る
            h.state_mut().state.shelf.wait_inspections();
            h.run();
            h.snapshot(format!("fillfx_panel_{}_{k}", lang.pick("ja", "en")));
        }
        results.extend_harness(&mut h);
    }
}

fn collect_texts_at(shape: &egui::epaint::Shape, out: &mut Vec<(Pos2, String)>) {
    match shape {
        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect_texts_at(s, out)),
        egui::epaint::Shape::Text(t) => out.push((t.pos, t.galley.job.text.clone())),
        _ => {}
    }
}

fn collect_texts(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
        egui::epaint::Shape::Text(t) => out.push(t.galley.job.text.clone()),
        _ => {}
    }
}

fn collect_clipped(shape: &egui::epaint::Shape, clip: Rect, out: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| collect_clipped(s, clip, out)),
        egui::epaint::Shape::Text(t) => {
            let bounds = Rect::from_min_size(t.pos, t.galley.size());
            // 横に切れる文字を見る（縦はスクロールで欄の端に半分かかることがある）。見える行の文字は、全部が横に収まっていること
            let visible = clip.intersects(bounds);
            if visible
                && bounds.width() > 0.0
                && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
            {
                out.push(format!("{} {:?} {:?}", t.galley.job.text, bounds, clip));
            }
        }
        _ => {}
    }
}

#[test]
fn dragging_a_ramp_slider_makes_one_undo_step_and_a_stop_click_makes_another() {
    let (mut h, _) = window();
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::M2Ui(yolu_app::m2::UiOp::PaintChannel(Channel::Roughness)),
    );
    apply(
        &mut h,
        Action::Fill(FillOp::AddGradient {
            layer,
            channel: Channel::Roughness,
        }),
    );
    h.state_mut()
        .state
        .ui
        .sections
        .insert("fill-gradient", true);
    // 欄のいちばん下（階調の分岐点の欄とカーブ）まで送る
    h.state_mut().state.m2.props_scroll = 100_000.0;
    h.run();
    let stops = |h: &Harness<'_, YoluApp>| {
        st(h)
            .doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Roughness)
            .unwrap()
            .ramp
            .clone()
            .unwrap()
    };
    // 分岐点の位置のスライダー（欄の下の方にある「位置」）が、欄の見える範囲の真ん中あたりに来るように送る
    let position = |h: &Harness<'_, YoluApp>| {
        use egui_kittest::kittest::Queryable;
        h.get_all_by_label("位置")
            .map(|n| n.rect())
            .filter(|r| r.left() > rx())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("分岐点の位置")
    };
    let found = position(&h);
    let scroll = st(&h).m2.props_scroll + (found.top() - props_mid(&h));
    h.state_mut().state.m2.props_scroll = scroll;
    h.run();
    let slider = position(&h);
    assert!(
        slider.top() > props_body(&h).top() && slider.bottom() < props_body(&h).bottom(),
        "{slider:?}"
    );
    let y = slider.bottom() - 6.0;
    let steps = st(&h).doc.undo_count();
    let x0 = slider.left() + 20.0;
    drag(
        &mut h,
        &[
            pos2(x0, y),
            pos2(x0 + 40.0, y),
            pos2(x0 + 100.0, y),
            pos2(x0 + 140.0, y),
        ],
    );
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "スライダーのドラッグは 1 回の取り消し"
    );
    assert!(
        stops(&h).colors()[0].position > 0.1,
        "{:?}",
        stops(&h).colors()
    );
    undo(&mut h);
    assert_eq!(stops(&h).colors()[0].position, 0.0);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

fn pen_sample(at: Pos2, contact: bool) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 9,
        time_ms: 0,
    }
}

/// 投影の欄を開いた塗りつぶしレイヤー（UV の投影）と、そのタイルの U の欄の中心（欄の見える範囲の真ん中あたりへ送ってある）。
fn projection_window() -> (Harness<'static, YoluApp>, LayerId, Pos2) {
    let (mut h, _) = window();
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .ui
        .sections
        .insert("fill-projection", true);
    h.state_mut().state.m2.props_scroll = 0.0;
    h.run();
    let tile_u = |h: &Harness<'_, YoluApp>| {
        let mut us: Vec<Rect> = h
            .get_all_by_label("U")
            .map(|n| n.rect())
            .filter(|r| r.left() > rx())
            .collect();
        us.sort_by(|a, b| a.top().total_cmp(&b.top()));
        us.first().copied().expect("タイルの U の欄")
    };
    let top = tile_u(&h).top();
    let scroll = st(&h).m2.props_scroll + (top - props_mid(&h));
    h.state_mut().state.m2.props_scroll = scroll;
    h.run();
    let field = tile_u(&h);
    assert!(
        field.top() > props_body(&h).top() && field.bottom() < props_body(&h).bottom(),
        "{field:?}"
    );
    (h, layer, field.center())
}

fn tiles(h: &Harness<'_, YoluApp>, layer: LayerId) -> [f64; 2] {
    layer_projection(h, layer).tiles
}

#[test]
fn dragging_a_number_field_in_the_panel_is_one_undo_and_escape_leaves_the_value_and_no_step() {
    let (mut h, layer, c) = projection_window();
    assert_eq!(tiles(&h, layer), [1.0, 1.0]);
    let steps = st(&h).doc.undo_count();
    // 離すまでの変更は 1 回の Undo
    drag(
        &mut h,
        &[
            c,
            c + vec2(10.0, 0.0),
            c + vec2(30.0, 0.0),
            c + vec2(50.0, 0.0),
        ],
    );
    let dragged = tiles(&h, layer)[0];
    assert!((dragged - 1.5).abs() < 0.011, "50 画素 × 0.01: {dragged}");
    assert_eq!(tiles(&h, layer)[1], 1.0, "V は動かさない");
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "ドラッグは 1 回の Undo");
    undo(&mut h);
    assert_eq!(tiles(&h, layer), [1.0, 1.0]);
    assert_eq!(st(&h).doc.undo_count(), steps);
    // 押したまま Esc: 押し始めの値へ戻り、履歴にも残らない（離しても動かない）
    press(&h, c, PointerButton::Primary);
    h.step();
    move_to(&h, c + vec2(20.0, 0.0));
    h.step();
    move_to(&h, c + vec2(45.0, 0.0));
    h.step();
    assert!(tiles(&h, layer)[0] > 1.3, "ドラッグの途中は文書に出る");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(tiles(&h, layer), [1.0, 1.0]);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps,
        "Esc で止めた分は履歴に残さない"
    );
    move_to(&h, c + vec2(70.0, 0.0));
    h.step();
    release(&h, c + vec2(70.0, 0.0), PointerButton::Primary);
    h.run();
    assert_eq!(tiles(&h, layer), [1.0, 1.0], "離しても動かない");
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn clicking_a_number_field_types_a_value_clamped_to_its_range_and_escape_throws_it_away() {
    let (mut h, layer, c) = projection_window();
    let steps = st(&h).doc.undo_count();
    // クリックして打つ（範囲の外は端へ。タイルは 1e-3〜1e4）
    click(&mut h, c);
    key(&h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text("2.5".into()));
    h.step();
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(tiles(&h, layer)[0], 2.5);
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "打った値も 1 回の Undo");
    click(&mut h, c);
    key(&h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text("1e9".into()));
    h.step();
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(tiles(&h, layer)[0], 1e4, "範囲の上の端");
    assert_eq!(st(&h).doc.undo_count(), steps + 2);
    // Esc で捨てる
    click(&mut h, c);
    key(&h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text("7".into()));
    h.step();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(tiles(&h, layer)[0], 1e4);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 2,
        "捨てたものは履歴に残さない"
    );
    undo(&mut h);
    assert_eq!(tiles(&h, layer)[0], 2.5);
    undo(&mut h);
    assert_eq!(tiles(&h, layer)[0], 1.0);
}

/// ペンの点を 1 点ずつ別のフレームで流す。
fn pen_frames(h: &mut Harness<'_, YoluApp>, steps: &[(Pos2, bool)]) {
    for (at, contact) in steps {
        h.state().pen().push(pen_sample(*at, *contact));
        h.step();
    }
}

#[test]
fn a_pen_drag_on_a_gizmo_handle_is_one_undo_with_the_property_fields_drawn_and_escape_and_focus_loss_put_it_back(
) {
    let (mut h, rect) = window();
    let (_, image) = shelf_image(&mut h, "石の模様");
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = st(&h).selected_layer.unwrap();
    apply(
        &mut h,
        Action::Fill(FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        }),
    );
    apply(
        &mut h,
        Action::Fill(FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Planar,
        }),
    );
    // 右の列のプロパティ（投影の欄）を描いたまま動かす。欄は egui のポインタの押下が偽なら毎フレームまとめを終える
    // （ペンの接触は egui のポインタの押下にならない）ので、ギズモのドラッグの途中で終えないことを確かめる
    assert!(
        h.get_all_by_label("投影").any(|n| n.rect().left() > rx()),
        "プロパティの投影の欄が描かれている"
    );
    let start = layer_projection(&h, layer);
    let from = gizmo::handle_point(st(&h), rect, Handle::MoveX).expect("X の矢印");
    let steps = st(&h).doc.undo_count();
    // 接触・動く・離れる
    for (k, p) in [
        from,
        from + vec2(20.0, 0.0),
        from + vec2(40.0, 0.0),
        from + vec2(60.0, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        pen_frames(&mut h, &[(p, true)]);
        assert!(gizmo::dragging(st(&h)), "{k}: ペンで掴んでいる");
        assert!(
            st(&h).doc.undo_count() <= steps + 1,
            "{k}: ドラッグの途中で別の Undo の段にならない（{} 段）",
            st(&h).doc.undo_count() - steps
        );
    }
    let moved = layer_projection(&h, layer);
    assert!(
        moved.placement.center[0].abs() > 0.01,
        "{:?}",
        moved.placement.center
    );
    assert!(moved.placement.center[1].abs() < 1e-6, "X の軸だけ動く");
    pen_frames(&mut h, &[(from + vec2(60.0, 0.0), false)]);
    h.run();
    assert!(!gizmo::dragging(st(&h)));
    assert_eq!(layer_projection(&h, layer), moved);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "ペンのドラッグは 1 回の Undo"
    );
    undo(&mut h);
    assert_eq!(layer_projection(&h, layer), start);
    // Esc: ペンの途中で押すと、前へ戻して履歴にも残さない。ペンが触れたままの点では、掴み直さず・描き始めもしない
    let steps = st(&h).doc.undo_count();
    pen_frames(
        &mut h,
        &[
            (from, true),
            (from + vec2(30.0, 0.0), true),
            (from + vec2(55.0, 0.0), true),
        ],
    );
    assert!(gizmo::dragging(st(&h)));
    assert_ne!(layer_projection(&h, layer), start);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(layer_projection(&h, layer), start);
    assert!(!gizmo::dragging(st(&h)));
    pen_frames(&mut h, &[(from + vec2(70.0, 0.0), true), (from, true)]);
    assert!(
        !gizmo::dragging(st(&h)),
        "Esc のあと、触れたままでは掴み直さない"
    );
    assert_eq!(layer_projection(&h, layer), start);
    pen_frames(&mut h, &[(from, false)]);
    h.run();
    assert_eq!(layer_projection(&h, layer), start, "離しても動かない");
    assert_eq!(st(&h).doc.undo_count(), steps);
    // 離したあとの触れ直しは、また掴める
    pen_frames(&mut h, &[(from, true)]);
    assert!(gizmo::dragging(st(&h)));
    pen_frames(&mut h, &[(from, false)]);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), steps, "動かさなければ段を足さない");
    // ウィンドウがフォーカスを失ったら、そこまでを捨てる
    pen_frames(&mut h, &[(from, true), (from + vec2(30.0, 0.0), true)]);
    assert!(gizmo::dragging(st(&h)));
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(layer_projection(&h, layer), start);
    assert!(!gizmo::dragging(st(&h)));
    assert_eq!(st(&h).doc.undo_count(), steps);
    pen_frames(&mut h, &[(from + vec2(30.0, 0.0), false)]);
    h.run();
    assert_eq!(layer_projection(&h, layer), start);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

/// 点のグラデーションのレイヤーを作る（点 2 つ）。右の列のプロパティが描かれていることも確かめる。
fn points_layer(h: &mut Harness<'_, YoluApp>) -> LayerId {
    let layer = {
        let s = &mut h.state_mut().state;
        s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Fill(FillOp::AddPoints {
            layer,
            channel: Channel::Color,
        }));
        layer
    };
    h.run();
    assert!(
        h.get_all_by_label("点のグラデーション")
            .any(|n| n.rect().left() > rx()),
        "プロパティの点のグラデーションの欄が描かれている"
    );
    layer
}

fn gradient_points(
    h: &Harness<'_, YoluApp>,
    layer: LayerId,
) -> yolu_core::fill_points::PointGradient {
    st(h)
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Color)
        .unwrap()
        .clone()
}

/// ペンの途中の 1 点ごとに、まとめが切れて別の Undo の段にならないことを見ながら動かす。
fn pen_drag_keeping_one_step(h: &mut Harness<'_, YoluApp>, path: &[Pos2], steps: usize) {
    for (k, p) in path.iter().enumerate() {
        pen_frames(h, &[(*p, true)]);
        assert!(
            yolu_app::fillfx::points::dragging(st(h)),
            "{k}: ペンで点を掴んでいる"
        );
        assert!(
            st(h).doc.undo_count() <= steps + 1,
            "{k}: ドラッグの途中で別の Undo の段にならない（{} 段）",
            st(h).doc.undo_count() - steps
        );
    }
}

#[test]
fn a_pen_drag_on_a_point_in_the_3d_view_is_one_undo_with_the_property_fields_drawn_and_escape_and_focus_loss_put_it_back(
) {
    use yolu_app::fillfx::points;
    let (mut h, rect) = window();
    let layer = points_layer(&mut h);
    let start = gradient_points(&h, layer);
    let view = st(&h).view3d.camera.view(rect.width(), rect.height());
    let c = view.to_screen(yolu_core::glam::Vec3::ZERO).unwrap();
    let at = pos2(rect.left() + c.x, rect.top() + c.y);
    // 面を押して点を追加し、そのままドラッグ。右の列の欄は、ペンの接触では毎フレームまとめを終えようとする
    let steps = st(&h).doc.undo_count();
    pen_drag_keeping_one_step(
        &mut h,
        &[
            at,
            at + vec2(12.0, 4.0),
            at + vec2(24.0, 8.0),
            at + vec2(36.0, 12.0),
        ],
        steps,
    );
    let moved = gradient_points(&h, layer);
    assert_eq!(moved.points.len(), 3);
    pen_frames(&mut h, &[(at + vec2(36.0, 12.0), false)]);
    h.run();
    assert!(!points::dragging(st(&h)));
    assert_eq!(gradient_points(&h, layer), moved);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "ペンの追加とドラッグで 1 回の Undo"
    );
    undo(&mut h);
    assert_eq!(gradient_points(&h, layer), start);
    key(&h, Key::Y, Modifiers::COMMAND);
    h.run();
    assert_eq!(gradient_points(&h, layer), moved);
    // 追加した点の印を掴んで動かし、Esc でドラッグの前へ戻す（履歴にも残さない）
    let mark = {
        let p = moved.points[2].position;
        let g = st(&h)
            .view3d
            .camera
            .view(rect.width(), rect.height())
            .to_screen(yolu_core::glam::Vec3::new(
                p[0] as f32,
                p[1] as f32,
                p[2] as f32,
            ))
            .unwrap();
        pos2(rect.left() + g.x, rect.top() + g.y)
    };
    let steps = st(&h).doc.undo_count();
    pen_drag_keeping_one_step(
        &mut h,
        &[mark, mark + vec2(-15.0, 0.0), mark + vec2(-30.0, 0.0)],
        steps,
    );
    assert_eq!(
        gradient_points(&h, layer).points.len(),
        3,
        "掴んだだけでは足さない"
    );
    assert_ne!(gradient_points(&h, layer), moved);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(gradient_points(&h, layer), moved);
    assert!(!points::dragging(st(&h)));
    assert_eq!(st(&h).doc.undo_count(), steps, "Esc は履歴に残さない");
    pen_frames(&mut h, &[(mark + vec2(-40.0, 0.0), true)]);
    assert!(
        !points::dragging(st(&h)),
        "Esc のあと、触れたままでは掴み直さない"
    );
    pen_frames(&mut h, &[(mark + vec2(-40.0, 0.0), false)]);
    h.run();
    assert_eq!(gradient_points(&h, layer), moved);
    assert_eq!(st(&h).doc.undo_count(), steps);
    // ウィンドウがフォーカスを失ったら、そこまでを捨てる
    pen_drag_keeping_one_step(
        &mut h,
        &[mark, mark + vec2(-15.0, 0.0), mark + vec2(-30.0, 0.0)],
        steps,
    );
    h.event(Event::WindowFocused(false));
    h.step();
    assert_eq!(gradient_points(&h, layer), moved);
    assert!(!points::dragging(st(&h)));
    assert_eq!(st(&h).doc.undo_count(), steps);
    pen_frames(&mut h, &[(mark + vec2(-30.0, 0.0), false)]);
    h.run();
    assert_eq!(gradient_points(&h, layer), moved);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn a_pen_drag_on_a_point_in_the_2d_canvas_is_one_undo_and_escape_puts_it_back() {
    use yolu_app::fillfx::points;
    let (mut h, _) = window();
    let layer = points_layer(&mut h);
    // UV の空間の 1 点だけの勾配にして、2D のキャンバスで操作する
    let uv = yolu_core::fill_points::PointGradient {
        space: yolu_core::fill_points::PointSpace::Uv,
        spread: 0.1,
        points: vec![yolu_core::fill_points::GradientPoint {
            position: [0.5, 0.5, 0.0],
            color: yolu_app::engine::Rgba8::new(10, 20, 30, 255),
        }],
    };
    apply(
        &mut h,
        Action::Fill(FillOp::Points {
            layer,
            channel: Channel::Color,
            points: Some(Box::new(uv)),
            coalesce: false,
        }),
    );
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let r = canvas_rect(&h);
    let (w, hh) = (st(&h).doc.width(), st(&h).doc.height());
    let view = st(&h).view.view(r, w, hh);
    let at = view.to_screen(w as f64 * 0.25, hh as f64 * 0.75);
    let start = gradient_points(&h, layer);
    let steps = st(&h).doc.undo_count();
    pen_drag_keeping_one_step(
        &mut h,
        &[
            at,
            at + vec2(10.0, -4.0),
            at + vec2(20.0, -8.0),
            at + vec2(30.0, -12.0),
        ],
        steps,
    );
    let moved = gradient_points(&h, layer);
    assert_eq!(moved.points.len(), 2);
    pen_frames(&mut h, &[(at + vec2(30.0, -12.0), false)]);
    h.run();
    assert!(!points::dragging(st(&h)));
    assert_eq!(gradient_points(&h, layer), moved);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "ペンの追加とドラッグで 1 回の Undo"
    );
    undo(&mut h);
    assert_eq!(gradient_points(&h, layer), start);
    key(&h, Key::Y, Modifiers::COMMAND);
    h.run();
    // 掴んで動かし、Esc でドラッグの前へ戻す
    let mark = {
        let p = moved.points[1].position;
        view.to_screen(p[0] * w as f64, p[1] * hh as f64)
    };
    let steps = st(&h).doc.undo_count();
    pen_drag_keeping_one_step(
        &mut h,
        &[mark, mark + vec2(-12.0, 0.0), mark + vec2(-24.0, 0.0)],
        steps,
    );
    assert_ne!(gradient_points(&h, layer), moved);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(gradient_points(&h, layer), moved);
    assert!(!points::dragging(st(&h)));
    assert_eq!(st(&h).doc.undo_count(), steps, "Esc は履歴に残さない");
    pen_frames(&mut h, &[(mark, false)]);
    h.run();
    assert_eq!(gradient_points(&h, layer), moved);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn the_point_gradient_panel_and_its_markers_draw_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in yolu_app::lang::Lang::ALL {
        let (mut h, rect) = window();
        let layer = {
            let s = &mut h.state_mut().state;
            s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
            s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
            let layer = s.selected_layer.unwrap();
            s.color.main = [0.9, 0.2, 0.1, 1.0];
            s.color.sub = [0.1, 0.3, 0.9, 1.0];
            s.apply(Action::Fill(FillOp::AddPoints {
                layer,
                channel: Channel::Color,
            }));
            s.apply(Action::Fill(FillOp::SelectPoint(Some(1))));
            for key in ["fill-image", "fill-projection", "fill-gradient"] {
                s.ui.sections.insert(key, false);
            }
            s.ui.sections.insert("fill-points", true);
            layer
        };
        h.run();
        // 塗りつぶしレイヤーの欄の下の、点のグラデーションの欄まで送る
        h.state_mut().state.m2.props_scroll = 490.0;
        h.run();
        // 3D ビューに点の印（点の色）が出る
        let g = st(&h)
            .doc
            .layer(layer)
            .unwrap()
            .fill_points(Channel::Color)
            .unwrap()
            .clone();
        assert_eq!(g.points.len(), 2);
        let want = lang.pick("点のグラデーション", "Point Gradient");
        assert!(
            h.query_all_by_label(want).next().is_some(),
            "{lang:?}: {want}"
        );
        let mut clipped = Vec::new();
        for shape in &h.output().shapes {
            collect_clipped(&shape.shape, shape.clip_rect, &mut clipped);
        }
        assert!(clipped.is_empty(), "{lang:?}: {clipped:?}");
        let mut shown = 0;
        for shape in &h.output().shapes {
            let mut texts = Vec::new();
            collect_texts_at(&shape.shape, &mut texts);
            for (at, text) in texts {
                if at.x > rx() + 20.0
                    && shape
                        .clip_rect
                        .intersects(Rect::from_min_size(at, vec2(1.0, 1.0)))
                {
                    common::assert_plain(&format!("{lang:?} 点の欄"), &text);
                    if lang == yolu_app::lang::Lang::En {
                        assert!(!common::has_japanese(&text), "英語の画面に日本語: {text}");
                    }
                    shown += 1;
                }
            }
        }
        assert!(shown > 8, "{lang:?}: 欄の文字を集められていない（{shown}）");
        let _ = rect;
        h.state_mut().state.shelf.wait_inspections();
        h.run();
        h.state_mut().state.message.clear();
        h.run();
        h.snapshot(format!("fillfx_points_{}", lang.pick("ja", "en")));
        // UV の空間: 立方体の UV の継ぎ目で色が切れる（モデルの空間は切れない）。2D のキャンバスにも印が出る
        let uv = yolu_core::fill_points::PointGradient {
            space: yolu_core::fill_points::PointSpace::Uv,
            spread: 0.1,
            points: vec![
                yolu_core::fill_points::GradientPoint {
                    position: [0.2, 0.5, 0.0],
                    color: yolu_app::engine::Rgba8::new(230, 51, 26, 255),
                },
                yolu_core::fill_points::GradientPoint {
                    position: [0.8, 0.5, 0.0],
                    color: yolu_app::engine::Rgba8::new(26, 77, 230, 255),
                },
            ],
        };
        apply(
            &mut h,
            Action::Fill(FillOp::Points {
                layer,
                channel: Channel::Color,
                points: Some(Box::new(uv)),
                coalesce: false,
            }),
        );
        h.state_mut().state.m2.props_scroll = 490.0;
        h.run();
        h.state_mut().state.message.clear();
        h.run();
        h.snapshot(format!("fillfx_points_uv_{}", lang.pick("ja", "en")));
        click_tab(&mut h, Tab::Canvas);
        // 焼いたメッシュマップの重ね表示を外して、塗った絵を見せる
        h.state_mut().state.bake.view = yolu_app::bake::MeshMapView::None;
        h.run();
        h.state_mut().state.message.clear();
        h.run();
        h.snapshot(format!("fillfx_points_canvas_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
    }
}

#[test]
fn clicking_the_model_while_editing_points_adds_a_point_and_delete_removes_it() {
    let (mut h, rect) = window();
    let layer = {
        let s = &mut h.state_mut().state;
        s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Fill(FillOp::AddPoints {
            layer,
            channel: Channel::Color,
        }));
        layer
    };
    h.run();
    let count = |h: &Harness<'_, YoluApp>| {
        st(h)
            .doc
            .layer(layer)
            .unwrap()
            .fill_points(Channel::Color)
            .unwrap()
            .points
            .len()
    };
    assert_eq!(count(&h), 2);
    let view = st(&h).view3d.camera.view(rect.width(), rect.height());
    let c = view.to_screen(yolu_core::glam::Vec3::ZERO).unwrap();
    let at = pos2(rect.left() + c.x, rect.top() + c.y);
    click(&mut h, at);
    assert_eq!(count(&h), 3, "{}", st(&h).message);
    assert_eq!(st(&h).fillfx.point_selected, Some(2));
    // 描かない（塗りつぶしレイヤーに描こうとした断りが出ない）
    assert!(!st(&h).is_stroking());
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(count(&h), 2);
    // 1 回の取り消しで、追加した点が戻る
    undo(&mut h);
    assert_eq!(count(&h), 3);
    // 2D のキャンバス: UV の空間にして、画素を押すとその UV の点が足される
    let uv = yolu_core::fill_points::PointGradient {
        space: yolu_core::fill_points::PointSpace::Uv,
        spread: 0.1,
        points: vec![yolu_core::fill_points::GradientPoint {
            position: [0.5, 0.5, 0.0],
            color: yolu_app::engine::Rgba8::new(10, 20, 30, 255),
        }],
    };
    apply(
        &mut h,
        Action::Fill(FillOp::Points {
            layer,
            channel: Channel::Color,
            points: Some(Box::new(uv)),
            coalesce: false,
        }),
    );
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let r = canvas_rect(&h);
    let (w, hh) = (st(&h).doc.width(), st(&h).doc.height());
    let v = st(&h).view.view(r, w, hh);
    let at = v.to_screen(w as f64 * 0.25, hh as f64 * 0.75);
    click(&mut h, at);
    let g = st(&h)
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Color)
        .unwrap()
        .clone();
    assert_eq!(g.points.len(), 2, "{}", st(&h).message);
    let p = g.points[1].position;
    assert!(
        (p[0] - 0.25).abs() < 0.03 && (p[1] - 0.75).abs() < 0.03 && p[2] == 0.0,
        "{p:?}"
    );
    assert!(!st(&h).is_stroking());
}

#[test]
fn the_point_colour_opens_the_colour_window_and_one_drag_in_it_is_one_undo() {
    use yolu_app::panels::color_window;
    let (mut h, _) = window();
    let layer = {
        let s = &mut h.state_mut().state;
        s.color.main = [0.9, 0.2, 0.1, 1.0];
        s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Fill(FillOp::AddPoints {
            layer,
            channel: Channel::Color,
        }));
        s.apply(Action::Fill(FillOp::SelectPoint(Some(0))));
        for key in ["fill-image", "fill-projection", "fill-gradient"] {
            s.ui.sections.insert(key, false);
        }
        s.ui.sections.insert("fill-points", true);
        layer
    };
    h.run();
    h.state_mut().state.m2.props_scroll = 490.0;
    h.run();
    let colors = |h: &Harness<'_, YoluApp>| {
        st(h)
            .doc
            .layer(layer)
            .unwrap()
            .fill_points(Channel::Color)
            .unwrap()
            .points
            .iter()
            .map(|p| p.color)
            .collect::<Vec<_>>()
    };
    let doc = st(&h).doc.id();
    let target =
        |i: usize| egui::Id::new(("fill.points.color", doc, layer.0, Channel::Color.index(), i));
    let first = colors(&h);
    let steps = st(&h).doc.undo_count();
    // 点 1 の色の欄を押すと、色のウィンドウがその点を相手に開く（描画色は入れない）
    let swatch = h.get_by_label("点の色").rect();
    click(&mut h, swatch.center());
    assert!(color_window::is_target(&h.ctx, target(0)));
    assert_eq!(colors(&h), first, "押しただけでは変えない");
    // ウィンドウの四角の中の 1 回のドラッグ: その場で点 1 の色が変わり、不透明度とほかの点はそのまま、1 回の取り消し
    let w = color_window::rect(&h.ctx).expect("色のウィンドウが開いている");
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(w));
    drag(
        &mut h,
        &[
            pos2(sq.left() + 8.0, sq.top() + 8.0),
            sq.center(),
            pos2(sq.right() - 8.0, sq.bottom() - 30.0),
        ],
    );
    let dragged = colors(&h);
    assert_ne!(
        [dragged[0].r, dragged[0].g, dragged[0].b],
        [first[0].r, first[0].g, first[0].b],
        "その場で色が変わる"
    );
    assert_eq!(dragged[0].a, first[0].a, "不透明度はそのまま");
    assert_eq!(dragged[1], first[1], "ほかの点はそのまま");
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "ドラッグは 1 回の取り消し"
    );
    assert_eq!(st(&h).color.main, [0.9, 0.2, 0.1, 1.0], "描画色は動かない");
    // 一覧で点 2 を選ぶと、ウィンドウはそのままで相手が点 2 へ替わる
    let second = h.get_by_label("点 2").rect();
    click(&mut h, second.center());
    assert_eq!(st(&h).fillfx.point_selected, Some(1));
    assert!(color_window::is_target(&h.ctx, target(1)));
    // 1 回の取り消しで、ドラッグの前の色へ戻る
    color_window::close(&h.ctx);
    h.run();
    undo(&mut h);
    assert_eq!(colors(&h), first);
}

#[test]
fn the_image_row_has_an_anisotropic_toggle_that_is_one_undo_step() {
    let mut results = SnapshotResults::new();
    for lang in yolu_app::lang::Lang::ALL {
        let (mut h, _) = window();
        let layer = {
            let s = &mut h.state_mut().state;
            s.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
            s.apply(Action::M2(yolu_app::m2::Edit::NewFill));
            let layer = s.selected_layer.unwrap();
            let rid = s.shelf.add_image(lang, "tile", &quad(), 2, 2).unwrap();
            let id = inputs::image_id(&rid).unwrap();
            s.apply(Action::Fill(FillOp::Image {
                layer,
                channel: Channel::Color,
                image: Some(id),
            }));
            s.apply(Action::Fill(FillOp::ProjectionMode {
                layer,
                mode: ProjectionMode::Triplanar,
            }));
            for key in ["fill-projection", "fill-gradient", "fill-points"] {
                s.ui.sections.insert(key, false);
            }
            s.ui.sections.insert("fill-image", true);
            layer
        };
        h.run();
        h.state_mut().state.m2.props_scroll = 300.0;
        h.run();
        let label = lang.pick("異方性フィルター", "Anisotropic filtering");
        let toggle = rect_of(&h, label, |r| r.left() > rx());
        let on = |h: &Harness<'_, YoluApp>| {
            st(h)
                .doc
                .layer(layer)
                .unwrap()
                .fill_anisotropic(Channel::Color)
        };
        assert!(on(&h), "既定は入");
        h.state_mut().state.shelf.wait_inspections();
        h.run();
        h.state_mut().state.message.clear();
        h.run();
        h.snapshot(format!(
            "fillfx_image_anisotropic_{}",
            lang.pick("ja", "en")
        ));
        let steps = st(&h).doc.undo_count();
        click(&mut h, toggle.center());
        assert!(!on(&h));
        assert_eq!(st(&h).doc.undo_count(), steps + 1);
        undo(&mut h);
        assert!(on(&h));
        results.extend_harness(&mut h);
    }
}
