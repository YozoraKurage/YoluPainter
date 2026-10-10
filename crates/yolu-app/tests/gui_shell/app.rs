//! ウィンドウの全体の振る舞いと見た目（egui_kittest。描画は wgpu のソフトの描画）。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton};
use yolu_app::engine::{BlendMode, Tilt};
use yolu_app::panels::view3d::{RecordingHost, View3dEvent};
use yolu_app::pen::PenSample;
use yolu_app::state::{PopupKind, Tool};

fn popup_kind(h: &egui_kittest::Harness<'_, yolu_app::YoluApp>) -> Option<PopupKind> {
    h.state().state.popup.as_ref().map(|p| p.kind)
}

#[test]
fn default_layout_snapshot() {
    // 既定の並び（3D ビューが左・キャンバスが右）
    let mut h = app_default(1280.0, 800.0, 512);
    let c = canvas_rect(&h);
    drag(
        &mut h,
        &[
            offset(c.center(), -80.0, -20.0),
            offset(c.center(), -20.0, 20.0),
            offset(c.center(), 60.0, -10.0),
        ],
    );
    h.snapshot("app_default");
}

#[test]
fn menu_opens_on_press_and_switches_on_hover() {
    let mut h = app(1280.0, 800.0, 256);
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    assert_eq!(popup_kind(&h), Some(PopupKind::MenuBar(0)));
    move_to(&h, menu_title(&h, "編集").center());
    h.run();
    assert_eq!(
        popup_kind(&h),
        Some(PopupKind::MenuBar(1)),
        "開いているあいだはホバーで切り替わる"
    );
    h.snapshot("menu_edit");
    // ←→ で隣の見出しへ、Esc で閉じる
    key(&h, Key::ArrowRight, Modifiers::NONE);
    h.run();
    assert_eq!(popup_kind(&h), Some(PopupKind::MenuBar(2)));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(popup_kind(&h), None);
    // 同じ見出しをもう一度押すと閉じる
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    assert_eq!(popup_kind(&h), None);
}

#[test]
fn menu_item_runs_its_action() {
    let mut h = app(1280.0, 800.0, 256);
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    let at = popup_item(&h, "表示を左右反転").center();
    click(&mut h, at);
    assert!(h.state().state.view.flip);
    assert_eq!(popup_kind(&h), None, "選んだら閉じる");
    let at = menu_title(&h, "レイヤー").center();
    click(&mut h, at);
    let at = popup_item(&h, "新規レイヤー").center();
    click(&mut h, at);
    assert_eq!(h.state().state.doc.layers().len(), 2);
    // 無効の項目は押しても閉じない・何もしない（やり直すものが無いときのやり直し）
    let at = menu_title(&h, "編集").center();
    click(&mut h, at);
    let at = popup_item(&h, "やり直し").center();
    click(&mut h, at);
    assert_eq!(popup_kind(&h), Some(PopupKind::MenuBar(1)));
}

#[test]
fn click_outside_closes_the_menu_without_painting() {
    let mut h = app(1280.0, 800.0, 256);
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert_eq!(popup_kind(&h), None);
    assert!(
        !h.state().state.doc.can_undo(),
        "閉じるための押下は描かない"
    );
    assert_eq!(canvas_pixel(&h, c)[3], 0);
}

#[test]
fn stroke_uploads_only_the_changed_tiles() {
    let mut h = app(1280.0, 800.0, 1024); // 8 × 8 = 64 タイル
    assert!(
        h.state().display().stats.total_tiles >= 64,
        "初めは全部を作る"
    );
    let before = h.state().display().stats.total_tiles;
    let c = canvas_rect(&h).center();
    let start = offset(c, 30.0, 30.0); // 文書の中心から少し右下（1 つか 2 つのタイル）
    press(&h, start, PointerButton::Primary);
    move_to(&h, offset(start, 6.0, 0.0));
    release(&h, offset(start, 6.0, 0.0), PointerButton::Primary);
    h.run();
    let stats = h.state().display().stats;
    let uploaded = stats.total_tiles - before;
    assert!(!stats.last_rebuilt);
    assert!(
        (1..=4).contains(&uploaded),
        "変わったタイルだけを上げる: {uploaded}"
    );
    assert_eq!(canvas_pixel(&h, start), [0, 0, 0, 255]);
    assert!(h.state().state.doc.can_undo());
}

#[test]
fn every_pointer_move_in_a_frame_becomes_a_point() {
    let mut h = app(1280.0, 800.0, 512);
    let c = canvas_rect(&h).center();
    press(&h, c, PointerButton::Primary);
    for i in 1..=10 {
        move_to(&h, offset(c, i as f32 * 3.0, 0.0));
    }
    h.step(); // 1 フレームに 11 の点
    assert_eq!(h.state().state.canvas.stroke_points, 11);
    release(&h, offset(c, 30.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(h.state().state.canvas.stroke.is_none());
}

#[test]
fn wheel_zooms_around_the_pointer() {
    let mut h = app(1280.0, 800.0, 512);
    let r = canvas_rect(&h);
    let p = offset(r.center(), 50.0, 30.0);
    let before = h.state().state.view.view(r, 512, 512).to_canvas(p);
    move_to(&h, p);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 2.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run();
    let view = h.state().state.view;
    assert!(view.zoom > 1.4, "{}", view.zoom);
    let after = view.view(r, 512, 512).to_canvas(p);
    assert!((before.0 - after.0).abs() < 0.05 && (before.1 - after.1).abs() < 0.05);
}

#[test]
fn view_keys_rotate_and_flip_and_the_corner_icons_reset_them() {
    let mut h = app(1280.0, 800.0, 512);
    key(&h, Key::Minus, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.view.angle, -15.0);
    h.event(Event::Text("^".into())); // JIS の ^ のキー
    h.event(Event::Text("^".into()));
    h.run();
    assert_eq!(h.state().state.view.angle, 15.0);
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    assert!(h.state().state.view.flip);
    assert_eq!(
        h.state().state.view.angle,
        -15.0,
        "反転すると角度の符号が変わる"
    );
    let c = canvas_rect(&h);
    drag(
        &mut h,
        &[
            offset(c.center(), -60.0, 0.0),
            offset(c.center(), 60.0, 40.0),
        ],
    );
    // 回転した頁のテクスチャの補間は、GPU のドライバのレイヤーで数画素ゆれる（同じ木で 0〜13 画素）。それ以外の差は落とす
    h.snapshot_options(
        "canvas_rotated_flipped",
        &egui_kittest::SnapshotOptions::new().max_failed_pixels(32),
    );
    use egui_kittest::kittest::Queryable;
    // 隅のアイコン（文字なし。名前と角度はツールチップ）で、回転と反転を戻す
    h.get_by_label("表示を回しています（-15°）。押すと回転を戻します（Shift+R）")
        .click();
    h.run();
    assert_eq!(h.state().state.view.angle, 0.0);
    h.get_by_label("表示を左右反転しています。押すと戻します（H）")
        .click();
    h.run();
    assert!(!h.state().state.view.flip);
}

#[test]
fn rotate_drag_with_alt_held_turns_the_view_in_15_degree_steps_and_never_paints() {
    let mut h = app(1280.0, 800.0, 512);
    let r = canvas_rect(&h);
    let alt = Modifiers::ALT;
    h.event(Event::ModifiersChanged(alt));
    h.step();
    // 中心の右から真下へ回す = 時計回りに 90°（15° の倍数）。押す・動く・離すに Alt を添える（実際のウィンドウのイベントもそうなる）
    let start = offset(r.center(), 100.0, 0.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: alt,
    });
    h.step();
    for p in [
        offset(r.center(), 70.7, 70.7),
        offset(r.center(), 0.0, 100.0),
    ] {
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: offset(r.center(), 0.0, 100.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: alt,
    });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert!(
        (h.state().state.view.angle - 90.0).abs() < 0.5,
        "{}",
        h.state().state.view.angle
    );
    assert!(!h.state().state.doc.can_undo(), "回すドラッグでは描かない");
}

#[test]
fn escape_cancels_the_stroke_and_focus_loss_commits_it() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    press(&h, c, PointerButton::Primary);
    move_to(&h, offset(c, 20.0, 0.0));
    h.step();
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, offset(c, 20.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(h.state().state.canvas.stroke.is_none());
    assert!(!h.state().state.doc.can_undo());
    assert_eq!(canvas_pixel(&h, c)[3], 0);

    press(&h, c, PointerButton::Primary);
    move_to(&h, offset(c, 20.0, 0.0));
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(
        h.state().state.canvas.stroke.is_none(),
        "フォーカスを失ったらストロークを終える"
    );
    assert!(h.state().state.doc.can_undo());
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 255]);
    release(&h, offset(c, 20.0, 0.0), PointerButton::Primary);
    h.run();
}

fn pen(pos: egui::Pos2, pressure: f32, contact: bool, eraser: bool) -> PenSample {
    PenSample {
        pos: [pos.x, pos.y],
        pressure,
        tilt: Tilt { x: 10.0, y: -5.0 },
        rotation: None,
        contact,
        eraser,
        barrel: false,
        pointer_id: 7,
        time_ms: 0,
    }
}

#[test]
fn pen_samples_paint_with_pressure_and_the_eraser_end_erases() {
    let mut h = app(1280.0, 800.0, 256);
    let r = canvas_rect(&h);
    let c = r.center();
    let pixel = h.state().state.view.view(r, 256, 256).pixel_size();
    // 半径 16 の筆圧 0.5 = 半径 8（画素）、不透明度も 0.5。中心から 12 画素の所は塗られず、5 画素の所は半分の覆い
    for (p, contact) in [
        (c, true),
        (offset(c, 0.5, 0.0), true),
        (offset(c, 0.5, 0.0), false),
    ] {
        h.state().pen().push(pen(p, 0.5, contact, false));
    }
    h.run();
    assert!(!h.state().state.doc.has_active_stroke());
    assert_eq!(canvas_pixel(&h, offset(c, 5.0 * pixel, 0.0))[3], 128);
    assert_eq!(canvas_pixel(&h, offset(c, 12.0 * pixel, 0.0))[3], 0);
    // 消しゴムの端
    for (p, contact) in [(c, true), (c, false)] {
        h.state().pen().push(pen(p, 1.0, contact, true));
    }
    h.run();
    assert_eq!(canvas_pixel(&h, c)[3], 0);
    assert_eq!(
        h.state().state.tool,
        Tool::Brush,
        "消しゴムの端はツールを変えない"
    );
}

/// 離しが届かないまま OS にペンの押しを奪われたとき: 離しが補われるまでは、その押しがキャンバスに残って別のポインタの番号の押しを使わない。補われた離しで
/// ストロークが終わり、次の押し（別のポインタの番号）は新しい押しとして描ける。
#[test]
fn a_taken_pen_press_is_finished_by_the_supplied_lift_and_the_next_press_paints() {
    use yolu_app::pen::capture::{seize, Loss};
    use yolu_app::pen::wintab::{PressOwner, Touch};
    let mut h = app(1280.0, 800.0, 256);
    let r = canvas_rect(&h);
    let c = r.center();
    let pixel = h.state().state.view.view(r, 256, 256).pixel_size();
    let (mut touch, mut owner) = (Touch::default(), PressOwner::default());
    // 押して、少し動かす（離しは来ない）
    for p in [c, offset(c, 20.0 * pixel, 0.0)] {
        let s = pen(p, 0.5, true, false);
        touch.note(&s);
        h.state().pen().push(s);
    }
    h.run();
    assert!(
        h.state().state.doc.has_active_stroke(),
        "離しが来ないあいだは描いている"
    );
    // 別のポインタの番号の押しは、前の押しが残っているあいだは使われない
    let elsewhere = offset(c, 0.0, 60.0 * pixel);
    let other = |contact: bool| PenSample {
        pointer_id: 9,
        ..pen(elsewhere, 0.5, contact, false)
    };
    h.state().pen().push(other(true));
    h.state().pen().push(other(false));
    h.run();
    assert_eq!(
        canvas_pixel(&h, elsewhere)[3],
        0,
        "前の押しが残っていると、次の押しは描けない"
    );
    // 押しを奪われた → 離しが補われ、ストロークが終わる
    let lift = seize(&mut touch, &mut owner, Loss::Pointer { id: 7, kept: false })
        .expect("触れていた押しの離しが補われる");
    h.state().pen().push_lost(lift);
    h.run();
    assert!(!h.state().state.doc.has_active_stroke());
    assert!(
        canvas_pixel(&h, c)[3] > 0,
        "終わったストロークは描かれている"
    );
    // 次の押しは新しい押しとして描ける
    h.state().pen().push(other(true));
    h.state().pen().push(other(false));
    h.run();
    assert!(
        canvas_pixel(&h, elsewhere)[3] > 0,
        "次の押しは新しい押しとして描ける"
    );
    assert!(!h.state().state.doc.has_active_stroke());
}

#[test]
fn layer_panel_add_hide_blend_and_opacity() {
    use egui_kittest::kittest::Queryable;
    let mut h = app(1280.0, 800.0, 256);
    h.get_by_label("新規レイヤー").click();
    h.run();
    assert_eq!(h.state().state.doc.layers().len(), 2);
    let top = h.state().state.selected_layer.unwrap();
    assert_eq!(h.state().state.doc.layers()[1].id(), top);
    // 一番上の行の目
    let eye = rect_of(&h, "非表示にする", |_| true);
    click(&mut h, eye.center());
    assert!(!h.state().state.doc.layer(top).unwrap().visible());
    // 合成モード（自前のドロップダウン）
    h.get_by_label("通常").click();
    h.run();
    assert_eq!(popup_kind(&h), Some(PopupKind::BlendMode(top)));
    h.snapshot("layers_blend_dropdown");
    let at = popup_item(&h, "乗算").center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.doc.layer(top).unwrap().blend_mode(),
        BlendMode::Multiply
    );
    // 不透明度のドラッグは 1 回の取り消しにまとまる（レイヤーのパネルの中の不透明度。オプションバーとプロパティにも同じ名前がある）
    let layers_tab = h.state().tab_rects[&yolu_app::Tab::Layers];
    let properties_tab = h.state().tab_rects[&yolu_app::Tab::Properties];
    let slider = rect_of(&h, "不透明度", |r| {
        r.left() > 1000.0 && r.top() > layers_tab.bottom() && r.top() < properties_tab.top()
    });
    drag(
        &mut h,
        &[
            pos2(slider.right() - 2.0, slider.center().y),
            pos2(slider.center().x, slider.center().y),
            pos2(slider.left() + slider.width() * 0.25, slider.center().y),
        ],
    );
    let opacity = h.state().state.doc.layer(top).unwrap().opacity();
    assert!((0.2..0.3).contains(&opacity), "{opacity}");
    h.state_mut().state.apply(yolu_app::state::Action::Undo);
    assert_eq!(h.state().state.doc.layer(top).unwrap().opacity(), 1.0);
}

#[test]
fn layer_rename_by_double_click() {
    let mut h = app(1280.0, 800.0, 256);
    let row = rect_of(&h, "レイヤー 1", |_| true);
    let at = pos2(row.left() + 70.0, row.center().y);
    for _ in 0..2 {
        press(&h, at, PointerButton::Primary);
        release(&h, at, PointerButton::Primary);
        h.step();
    }
    h.run();
    assert!(h.state().state.ui.renaming.is_some());
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("背景".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.doc.layers()[0].name(), "背景");
    assert!(h.state().state.ui.renaming.is_none());
}

#[test]
fn layer_drag_reorders() {
    let mut h = app(1280.0, 800.0, 256);
    for _ in 0..2 {
        h.state_mut().state.apply(yolu_app::state::Action::NewLayer);
    }
    h.run();
    let top = rect_of(&h, "レイヤー 3", |_| true);
    let bottom = rect_of(&h, "レイヤー 1", |_| true);
    let start = pos2(top.left() + 80.0, top.center().y);
    drag(
        &mut h,
        &[
            start,
            offset(start, 0.0, 10.0),
            offset(start, 0.0, 30.0),
            pos2(start.x, bottom.bottom() + 4.0),
        ],
    );
    let names: Vec<String> = h
        .state()
        .state
        .doc
        .layers()
        .iter()
        .map(|l| l.name().to_owned())
        .collect();
    assert_eq!(
        names,
        ["レイヤー 3", "レイヤー 1", "レイヤー 2"],
        "一番上を一番下へ"
    );
}

#[test]
fn color_panel_picks_hex_swap_and_wheel() {
    use egui_kittest::kittest::Queryable;
    let mut h = app(1280.0, 800.0, 256);
    // 既定は色相の円と中の四角。四角と色相の帯へ切り替えてから選ぶ（終わりに円へ戻す）
    assert!(h.state().state.color.wheel);
    h.get_by_label("四角と色相の帯").click();
    h.run();
    assert!(!h.state().state.color.wheel);
    let sv = h.get_by_label("彩度と明度").rect();
    click(&mut h, pos2(sv.right() - 0.5, sv.top() + 0.5));
    let c = h.state().state.color.main;
    assert!(c[0] > 0.97 && c[1] < 0.03 && c[2] < 0.03, "{c:?}");
    let hue = h.get_by_label("色相").rect();
    click(
        &mut h,
        pos2(hue.center().x, hue.top() + hue.height() * 2.0 / 3.0),
    ); // 色相 1/3 = 緑
    let c = h.state().state.color.main;
    assert!(c[1] > 0.95 && c[0] < 0.05, "{c:?}");
    // 16 進
    // 16 進の欄（アセットの検索の欄も文字の入力なので、カラーのパネルの中のものを選ぶ）
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| r.top() > 300.0)
        .expect("16 進の欄");
    click(&mut h, field.center());
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("#3366cc".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(
        yolu_app::state::to_hex(h.state().state.color.main),
        "3366CC"
    );
    h.get_by_label("メインとサブの色を入れ替え（X）").click();
    h.run();
    assert_eq!(
        yolu_app::state::to_hex(h.state().state.color.main),
        "FFFFFF"
    );
    assert_eq!(yolu_app::state::to_hex(h.state().state.color.sub), "3366CC");
    h.get_by_label("色相の円").click();
    h.run();
    assert!(h.state().state.color.wheel);
    h.snapshot("color_wheel");
}

#[test]
fn properties_tabs_and_pen_toggles() {
    use egui_kittest::kittest::Queryable;
    let mut h = app(1600.0, 900.0, 256);
    // 筆圧の切り替えは左のサブツールのパネル（ツールプロパティ）にある（オプションバーには直径と不透明度だけ）
    let in_panel = |r: egui::Rect| r.left() < 390.0 && r.top() > 80.0;
    let pen = rect_of(&h, "筆圧で直径を変える", in_panel);
    click(&mut h, pen.center());
    assert!(!h.state().state.brush.pressure_size);
    let flow = rect_of(&h, "筆圧で流量を変える", in_panel);
    click(&mut h, flow.center());
    assert!(h.state().state.brush.pressure_flow);
    // 右のプロパティはステンシルのタブから始まる。タブはステンシルとレイヤーの 2 つだけ（ブラシのタブも、マテリアルのタブも、筆先の形のアルファの
    // タブも無い。マテリアルは別のパネル、筆先の形は詳細のウィンドウの「形状」）
    assert_eq!(h.state().state.ui.property_tab, 0);
    assert!(h.query_all_by_label("アルファ").next().is_none());
    assert!(
        h.query_all_by_label("マテリアル").next().is_none(),
        "マテリアルはプロパティのタブではない"
    );
    // （「レイヤー」の名前は、右のプロパティのタブのほかにも出る。プロパティの組の中のもの）
    let props = h.state().tab_rects[&yolu_app::Tab::Properties];
    let tab = rect_of(&h, "レイヤー", |r| {
        r.left() > props.left() - 2.0 && r.top() > props.top()
    });
    click(&mut h, tab.center());
    assert_eq!(h.state().state.ui.property_tab, 1);
    h.snapshot("properties_layer_tab");
    h.get_by_label("ステンシル").click();
    h.run();
    assert_eq!(h.state().state.ui.property_tab, 0);
    h.snapshot("properties_stencil_tab");
}

#[test]
fn view3d_host_hears_placement_hidden_and_covered() {
    let mut h = app(1000.0, 700.0, 256);
    let host = RecordingHost::default();
    let events = host.events.clone();
    h.state_mut().set_view3d_host(Box::new(host));
    h.run();
    assert!(
        events.lock().unwrap().is_empty(),
        "隠れたままなら知らせない"
    );
    click_tab(&mut h, yolu_app::Tab::View3d);
    let placed = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| {
            if let View3dEvent::Placed(p) = e {
                Some(*p)
            } else {
                None
            }
        })
        .expect("placed");
    assert!(placed.rect_px[2] > 100 && placed.rect_px[3] > 100 && !placed.floating);
    h.snapshot("view3d_placeholder");
    // メニューが重なったら知らせる
    let at = menu_title(&h, "表示").center();
    click(&mut h, at);
    assert!(events.lock().unwrap().contains(&View3dEvent::Covered(true)));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(events
        .lock()
        .unwrap()
        .contains(&View3dEvent::Covered(false)));
    // 中の入力を渡す
    let at = pos2(
        (placed.rect_px[0] + 40) as f32,
        (placed.rect_px[1] + 30) as f32,
    );
    move_to(&h, at);
    h.run();
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, View3dEvent::Input(i) if (i.pos[0] - 40.0).abs() < 1.0 && (i.pos[1] - 30.0).abs() < 1.0)));
    // キャンバスのタブに戻すと隠れる
    click_tab(&mut h, yolu_app::Tab::Canvas);
    assert_eq!(events.lock().unwrap().last(), Some(&View3dEvent::Hidden));
}

#[test]
fn shortcuts_switch_tools_colors_size_and_history() {
    let mut h = app(1280.0, 800.0, 256);
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Eraser);
    key(&h, Key::B, Modifiers::NONE);
    key(&h, Key::X, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Brush);
    assert_eq!(h.state().state.color.main, [1.0, 1.0, 1.0, 1.0]);
    key(&h, Key::D, Modifiers::NONE);
    key(&h, Key::CloseBracket, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.color.main, [0.0, 0.0, 0.0, 1.0]);
    assert!((h.state().state.brush.radius - 16.0 * 1.15).abs() < 1e-4);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 10.0, 0.0)]);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(canvas_pixel(&h, c)[3], 0);
    key(&h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(canvas_pixel(&h, c)[3], 255);
}

#[test]
fn view3d_host_hears_when_the_tab_floats_in_a_window() {
    let mut h = app(1280.0, 800.0, 256);
    let host = RecordingHost::default();
    let events = host.events.clone();
    h.state_mut().set_view3d_host(Box::new(host));
    // 3D ビューのタブをドックから外して浮いたウィンドウへ（ドラッグで外したのと同じ形）
    {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&yolu_app::Tab::View3d).expect("3D tab");
        dock.detach_tab(
            path,
            egui::Rect::from_min_size(pos2(400.0, 200.0), vec2(360.0, 280.0)),
        );
    }
    h.run();
    let placed = events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|e| {
            if let View3dEvent::Placed(p) = e {
                Some(*p)
            } else {
                None
            }
        })
        .expect("placed");
    assert!(placed.floating, "浮いたウィンドウの中");
    assert!(
        placed.rect_px[2] > 100 && placed.rect_px[2] < 400,
        "{:?}",
        placed.rect_px
    );
    // ウィンドウを動かすと置き場所が変わったと知らせる（浮かせたウィンドウは別ウィンドウになる。試験のウィンドウでは、別ウィンドウは記録の位置に描く）
    let before = placed.rect_px;
    {
        let detached = &mut h.state_mut().detached;
        assert!(detached.contains(yolu_app::Tab::View3d));
        let record = detached.windows[0].record.as_mut().expect("置き場所");
        record.position = [500.0, 260.0];
    }
    h.run();
    let moved = events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|e| {
            if let View3dEvent::Placed(p) = e {
                Some(*p)
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(moved.rect_px, before);
}
