//! 色のウィンドウ（`panels::color_window`）をアプリの中で: 色の値の欄（塗りつぶしレイヤーの値）を押すと動かせるウィンドウが出て、円の変更がその場で欄へ入り、
//! ドラッグ 1 回・ボタン 1 回が 1 回の取り消しになる。別の色の欄を押すと相手が替わり、相手のレイヤーを消すと閉じる。Esc は開いたときの色へ戻す。
//! ウィンドウの絵（日英）。分岐点の色のウィンドウの細かい振る舞いは `gui_canvas/ramp_panel.rs`。
use crate::common;

use common::*;
use egui::{pos2, vec2, Key, Modifiers, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::engine::{Channel, LayerId, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::panels::color_window;
use yolu_app::state::Action;
use yolu_app::YoluApp;

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 描画色を赤にして、塗りつぶしレイヤーを作る（値は描画色）。
fn fill_layer(h: &mut Harness<'_, YoluApp>) -> LayerId {
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    apply(h, Action::M2(Edit::NewFill));
    h.state()
        .state
        .selected_layer
        .expect("作ったレイヤーを選ぶ")
}

fn value(h: &Harness<'_, YoluApp>, id: LayerId, channel: Channel) -> Option<Rgba8> {
    h.state()
        .state
        .doc
        .layer(id)
        .and_then(|l| l.fill_value(channel))
}

fn undo_count(h: &Harness<'_, YoluApp>) -> usize {
    h.state().state.doc.undo_count()
}

fn window(h: &Harness<'_, YoluApp>) -> Rect {
    color_window::rect(&h.ctx).expect("色のウィンドウが開いている")
}

fn target(id: LayerId, channel: Channel) -> egui::Id {
    egui::Id::new(("fill.value", id.0, channel.index()))
}

#[test]
fn a_fill_value_opens_the_window_and_one_drag_in_it_is_one_undo_and_escape_goes_back() {
    let mut h = app(1280.0, 1700.0, 64);
    let id = fill_layer(&mut h);
    let first = value(&h, id, Channel::Color);
    assert_eq!(first, Some(Rgba8::new(255, 0, 0, 255)));
    let steps = undo_count(&h);
    let swatch = h.get_by_label("カラー の値").rect();
    click(&mut h, swatch.center());
    assert!(color_window::is_target(&h.ctx, target(id, Channel::Color)));
    let w = window(&h);
    assert!(w.right() <= swatch.left(), "欄の左に出る: {w:?} {swatch:?}");
    // 四角の中で何度か動かす（1 回のドラッグ）: その場で値が変わり、1 回の取り消し
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(w));
    drag(
        &mut h,
        &[
            pos2(sq.left() + 8.0, sq.top() + 8.0),
            sq.center(),
            pos2(sq.right() - 8.0, sq.bottom() - 30.0),
        ],
    );
    let dragged = value(&h, id, Channel::Color);
    assert_ne!(dragged, first, "その場で値が変わる");
    assert_eq!(undo_count(&h), steps + 1, "ドラッグは 1 回の取り消し");
    assert!(color_window::is_open(&h.ctx), "ドラッグのあとも開いたまま");
    // 描画色は動かない
    assert_eq!(h.state().state.color.main, [1.0, 0.0, 0.0, 1.0]);
    // Esc: 開いたときの色へ戻して閉じる（戻すのも 1 回の取り消し）
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(value(&h, id, Channel::Color), first);
    assert!(!color_window::is_open(&h.ctx));
    assert_eq!(undo_count(&h), steps + 2);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(value(&h, id, Channel::Color), dragged);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(value(&h, id, Channel::Color), first);
}

/// 16 進の欄を打っている途中の Esc は、その入力をやめるだけ: ウィンドウは開いたままで、相手の値は戻さない（ここまでの変更を残す）。
/// egui は Esc を受けたフレームの初めに欄のフォーカスを外すので、ウィンドウは前のフレームの入力中かどうかで見分ける。次の Esc で、開いたときの色へ戻して閉じる。
#[test]
fn escape_while_typing_in_the_hex_field_only_stops_typing_and_the_next_one_goes_back() {
    let mut h = app(1280.0, 1700.0, 64);
    let id = fill_layer(&mut h);
    let first = value(&h, id, Channel::Color);
    let swatch = h.get_by_label("カラー の値").rect();
    click(&mut h, swatch.center());
    let w = window(&h);
    // 円の中の四角を押して、色を変える（1 回の取り消し）
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(w));
    click(&mut h, sq.left_top() + vec2(6.0, 6.0));
    let changed = value(&h, id, Channel::Color);
    assert_ne!(changed, first);
    let steps = undo_count(&h);
    // 16 進の欄を押して打つ
    click(&mut h, color_window::hex_of(w, false).center());
    h.event(egui::Event::Text("12".into()));
    h.run();
    assert!(
        h.ctx.memory(|m| m.focused().is_some()),
        "16 進の欄を打っている"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(
        color_window::is_target(&h.ctx, target(id, Channel::Color)),
        "ウィンドウは閉じない"
    );
    assert_eq!(
        value(&h, id, Channel::Color),
        changed,
        "ここまでの変更は戻さない"
    );
    assert_eq!(undo_count(&h), steps, "取り消しも積まない");
    assert!(h.ctx.memory(|m| m.focused().is_none()), "入力はやめた");
    // 次の Esc: 開いたときの色へ戻して閉じる
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!color_window::is_open(&h.ctx));
    assert_eq!(value(&h, id, Channel::Color), first);
}

/// ウィンドウより先に描く部品がこのフレームの Esc を使った（塗りつぶしの仕事の取消）ときは、ウィンドウは同じ Esc で相手の値を戻したり閉じたりしない。
/// 次の Esc がウィンドウのもの。
#[test]
fn an_escape_another_part_already_used_neither_closes_the_window_nor_reverts_its_target() {
    use yolu_app::matpaint::MatAction;
    // 65536 画素を超える文書の塗りつぶしは別のスレッドの仕事になる
    let mut h = app(1500.0, 1600.0, 512);
    open_paint_channels(&mut h);
    apply(&mut h, Action::Mat(MatAction::Enabled(true)));
    apply(
        &mut h,
        Action::Mat(MatAction::Channel(Channel::Emission, true)),
    );
    let first = h.state().state.mat.emission;
    let at = h
        .get_by_label("ストロークが塗るエミッションの色")
        .rect()
        .center();
    click(&mut h, at);
    assert!(color_window::is_target(
        &h.ctx,
        egui::Id::new("mat.emission")
    ));
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(window(&h)));
    click(&mut h, sq.left_top() + vec2(6.0, 6.0));
    let changed = h.state().state.mat.emission;
    assert_ne!(changed, first);
    // 塗りつぶしを走らせて、Esc をその取消に使わせる
    yolu_app::region::bucket::start(&mut h.state_mut().state, vec![(1.0, 1.0)]);
    assert!(
        h.state().state.region.job.is_some(),
        "塗りつぶしの仕事が走っている"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(h.state().state.region.job.is_none(), "仕事は取り消された");
    assert!(color_window::is_open(&h.ctx), "ウィンドウは閉じない");
    assert_eq!(h.state().state.mat.emission, changed, "相手の値は戻さない");
    // 次の Esc がウィンドウのもの
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!color_window::is_open(&h.ctx));
    assert_eq!(h.state().state.mat.emission, first);
}

#[test]
fn the_paint_colour_and_the_colour_set_each_make_one_undo_and_another_field_takes_over_the_window()
{
    let mut h = app(1280.0, 1700.0, 64);
    let id = fill_layer(&mut h);
    // エミッションの値も持たせる（色の欄が 2 つ）
    apply(
        &mut h,
        Action::M2(Edit::FillValue {
            id,
            channel: Channel::Emission,
            value: Some(Rgba8::new(10, 20, 30, 255)),
        }),
    );
    let emission = value(&h, id, Channel::Emission);
    assert!(emission.is_some());
    let at = h.get_by_label("カラー の値").rect().center();
    click(&mut h, at);
    let placed = window(&h);
    // 描画色を入れる（1 回の取り消し）
    h.state_mut().state.color.set_main([0.0, 0.0, 1.0, 1.0]);
    h.run();
    let steps = undo_count(&h);
    h.get_by_label("描画色を入れる").click();
    h.run();
    assert_eq!(
        value(&h, id, Channel::Color),
        Some(Rgba8::new(0, 0, 255, 255))
    );
    assert_eq!(undo_count(&h), steps + 1);
    // 今のカラーセットの色（1 回の取り消し）
    let first = h.state().state.colorsets.palette().colors[0].clone();
    let cell = h
        .get_all_by_label(&first.label())
        .map(|n| n.rect())
        .find(|r| placed.contains_rect(*r))
        .expect("ウィンドウの中のカラーセットの色");
    click(&mut h, cell.center());
    let c = first.rgba.map(yolu_app::ui::widgets::to_byte);
    assert_eq!(
        value(&h, id, Channel::Color),
        Some(Rgba8::new(c[0], c[1], c[2], 255))
    );
    assert_eq!(undo_count(&h), steps + 2);
    // 別の色の欄を押すと、ウィンドウはそのままで相手が替わる
    let at = h.get_by_label("エミッション の値").rect().center();
    click(&mut h, at);
    assert!(color_window::is_target(
        &h.ctx,
        target(id, Channel::Emission)
    ));
    assert_eq!(window(&h), placed, "ウィンドウは動かない");
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(placed));
    click(&mut h, sq.left_top() + vec2(6.0, 6.0));
    assert_ne!(value(&h, id, Channel::Emission), emission);
    // 相手のレイヤーを消すと閉じる
    apply(&mut h, Action::DeleteLayer);
    h.run();
    assert!(h.state().state.doc.layer(id).is_none());
    assert!(!color_window::is_open(&h.ctx), "相手が無くなったら閉じる");
}

#[test]
fn the_window_moves_by_its_title_keeps_its_place_and_does_not_close_on_an_outside_click() {
    let mut h = app(1280.0, 1700.0, 64);
    let id = fill_layer(&mut h);
    let swatch = h.get_by_label("カラー の値").rect();
    click(&mut h, swatch.center());
    let first = window(&h);
    let grab = pos2(first.left() + 60.0, first.top() + 12.0);
    drag(
        &mut h,
        &[grab, grab + vec2(-40.0, -20.0), grab + vec2(-80.0, -40.0)],
    );
    let moved = window(&h);
    assert!(
        (moved.min - (first.min + vec2(-80.0, -40.0))).length() < 1.5,
        "{first:?} → {moved:?}"
    );
    // 外（下の状態の帯）を押しても閉じない
    click(&mut h, pos2(640.0, 1192.0));
    assert!(color_window::is_target(&h.ctx, target(id, Channel::Color)));
    // 閉じて開き直すと、前の位置
    h.get_by_label("閉じる").click();
    h.run();
    assert!(!color_window::is_open(&h.ctx));
    click(&mut h, swatch.center());
    assert_eq!(window(&h).min, moved.min);
}

/// ウィンドウの絵（日英）: 塗りつぶしレイヤーのカラーの値を押して開いたところ。
#[test]
fn snapshot_the_colour_window_on_a_fill_value() {
    let mut results = SnapshotResults::new();
    for (lang, name, label) in [
        (Lang::Ja, "ja", "カラー の値"),
        (Lang::En, "en", "Value of Color"),
    ] {
        let mut h = app(1280.0, 1500.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        fill_layer(&mut h);
        let swatch = h.get_by_label(label).rect();
        click(&mut h, swatch.center());
        assert!(color_window::is_open(&h.ctx));
        h.snapshot(format!("color_window_fill_{name}"));
        results.extend_harness(&mut h);
    }
}

/// マテリアルのパネル: lilToon の色（文書の見た目。1 回のドラッグが 1 回の取り消し）と、ツールプロパティの塗るチャンネルのエミッション（ブラシの設定。取り消しに積まない）。
/// lilToon の色から押し替えると、ウィンドウはそのままで相手が替わる。
#[test]
fn a_liltoon_colour_and_the_brush_emission_share_the_window() {
    use yolu_app::matpaint::MatAction;
    let mut h = app(1500.0, 1600.0, 64);
    // 右の「マテリアル」のパネルと、ツールプロパティの「塗るチャンネル」。塗るチャンネルでエミッションも塗る
    open_material(&mut h);
    open_paint_channels(&mut h);
    apply(&mut h, Action::Mat(MatAction::Enabled(true)));
    apply(
        &mut h,
        Action::Mat(MatAction::Channel(Channel::Emission, true)),
    );
    let doc = h.state().state.doc.id();
    let main_color = |h: &Harness<'_, YoluApp>| {
        yolu_app::look::liltoon::value(h.state().state.doc.drawn_look(), "_Color")
    };
    // メインカラーの節を開く
    let header = h
        .get_all_by_label("メインカラー / 透過設定")
        .find(|n| n.accesskit_node().role() != egui::accesskit::Role::CheckBox)
        .expect("節の見出し")
        .rect();
    click(&mut h, header.center());
    let before = main_color(&h);
    let steps = undo_count(&h);
    let at = h.get_by_label("_Color").rect().center();
    click(&mut h, at);
    assert!(color_window::is_target(
        &h.ctx,
        egui::Id::new(("look.color", doc, "_Color"))
    ));
    let placed = window(&h);
    let sq = yolu_app::panels::color::wheel_square(color_window::wheel_of(placed));
    drag(
        &mut h,
        &[
            sq.left_top() + vec2(10.0, 10.0),
            sq.center(),
            sq.right_bottom() - vec2(20.0, 30.0),
        ],
    );
    assert_ne!(main_color(&h), before, "その場で値が変わる");
    assert_eq!(undo_count(&h), steps + 1, "ドラッグは 1 回の取り消し");
    // ブラシのエミッションの見本を押すと、ウィンドウはそのままで相手が替わる
    let emission = h.state().state.mat.emission;
    let at = h
        .get_by_label("ストロークが塗るエミッションの色")
        .rect()
        .center();
    click(&mut h, at);
    assert!(color_window::is_target(
        &h.ctx,
        egui::Id::new("mat.emission")
    ));
    assert_eq!(window(&h), placed);
    h.state_mut().state.color.set_main([0.2, 0.6, 0.4, 1.0]);
    h.run();
    h.get_by_label("描画色を入れる").click();
    h.run();
    let main = h.state().state.color.main;
    assert_ne!(h.state().state.mat.emission, emission);
    for (e, m) in h.state().state.mat.emission.iter().zip(main) {
        assert!((e - m).abs() < 1.0 / 255.0);
    }
    assert_eq!(
        undo_count(&h),
        steps + 1,
        "ブラシの設定は取り消しに積まない"
    );
}
