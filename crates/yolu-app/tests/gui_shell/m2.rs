//! M2 の画面の操作（egui_kittest）: レイヤーのパネル（グループ・マスク・塗りつぶし・調整・ドラッグ）、チャンネルのパネル、
//! プロパティの欄（ブラシの M2 の設定・レイヤーの欄）、ペンの時刻と回転。どれも「パネルの操作 → 文書が変わる → Undo で戻る」。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{pos2, Key, Modifiers, PointerButton};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{BlendMode, Channel, LayerKind};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, AppState, PopupKind};
use yolu_app::{Tab, YoluApp};

fn undo(h: &mut Harness<'_, YoluApp>) {
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
}

fn doc_layers(h: &Harness<'_, YoluApp>) -> usize {
    h.state().state.doc.layers().len()
}

fn popup_kind(h: &Harness<'_, YoluApp>) -> Option<PopupKind> {
    h.state().state.popup.as_ref().map(|p| p.kind)
}

/// 帯の塗りつぶしのボタンを押して、ポップアップの「単色」を選ぶ（押しただけでは作らない）。
fn toolbar_new_fill(h: &mut Harness<'_, YoluApp>) {
    h.get_by_label("新規塗りつぶしレイヤー").click();
    h.run();
    assert_eq!(
        popup_kind(h),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::NewFill))
    );
    let at = popup_item(h, "単色").center();
    click(h, at);
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 描いた線（キャンバスの中央を横切る）。
fn stroke_across(h: &mut Harness<'_, YoluApp>, dy: f32) {
    let c = canvas_rect(h).center();
    drag(
        h,
        &[
            offset(c, -60.0, dy),
            offset(c, -20.0, dy),
            offset(c, 20.0, dy),
            offset(c, 60.0, dy),
        ],
    );
}

#[test]
fn layer_toolbar_makes_groups_fills_adjustments_and_masks_one_undo_each() {
    let mut h = app(1280.0, 800.0, 128);
    let first = h.state().state.selected_layer.unwrap();
    // グループ化: 選んでいるレイヤーが新しいグループに入る
    h.get_by_label("レイヤーをグループ化").click();
    h.run();
    let group = h.state().state.selected_layer.unwrap();
    assert!(h.state().state.doc.layer(group).unwrap().is_group());
    assert_eq!(
        h.state().state.doc.layer(first).unwrap().parent(),
        Some(group)
    );
    undo(&mut h);
    assert_eq!(doc_layers(&h), 1, "グループ化は 1 回の取り消しで戻る");
    // 塗りつぶし
    toolbar_new_fill(&mut h);
    let fill = h.state().state.selected_layer.unwrap();
    assert_eq!(
        h.state().state.doc.layer(fill).unwrap().kind(),
        LayerKind::Fill
    );
    undo(&mut h);
    assert_eq!(doc_layers(&h), 1);
    // 調整（自前のメニュー）
    h.get_by_label("新規調整レイヤー").click();
    h.run();
    assert_eq!(
        popup_kind(&h),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::NewAdjustment))
    );
    let at = popup_item(&h, "レベル補正").center();
    click(&mut h, at);
    let adj = h.state().state.selected_layer.unwrap();
    assert_eq!(
        h.state().state.doc.layer(adj).unwrap().kind(),
        LayerKind::Adjustment
    );
    undo(&mut h);
    assert_eq!(doc_layers(&h), 1);
    // マスク: 足すと描く先がマスクになり、もう一度押すとレイヤーへ戻る
    h.get_by_label("レイヤーマスクを追加").click();
    h.run();
    assert!(h.state().state.doc.layer(first).unwrap().mask().is_some());
    assert!(h.state().state.m2.edit_mask);
    h.get_by_label("レイヤーマスクを編集").click();
    h.run();
    assert!(!h.state().state.m2.edit_mask);
    undo(&mut h);
    assert!(h.state().state.doc.layer(first).unwrap().mask().is_none());
}

#[test]
fn layer_tree_collapses_and_drag_moves_a_layer_into_a_group() {
    let mut h = app(1280.0, 800.0, 128);
    apply(&mut h, Action::M2(Edit::GroupSelected)); // グループ 1 の中にレイヤー 1
    apply(&mut h, Action::NewLayer); // グループの上にレイヤー 2
    let group = h
        .state()
        .state
        .doc
        .layers()
        .iter()
        .find(|l| l.is_group())
        .unwrap()
        .id();
    let top = h.state().state.selected_layer.unwrap();
    assert_eq!(h.state().state.doc.layer(top).unwrap().parent(), None);
    // 開閉
    assert!(h.query_by_label("レイヤー 1").is_some());
    h.get_by_label("グループを閉じる").click();
    h.run();
    assert!(h.state().state.m2.collapsed.contains(&group));
    assert!(
        h.query_by_label("レイヤー 1").is_none(),
        "閉じたグループの中身は出ない"
    );
    h.get_by_label("グループを開く").click();
    h.run();
    assert!(h.query_by_label("レイヤー 1").is_some());
    // 一番上の行を、グループの行の中ほどへドラッグ → グループの中へ
    let top_name = h.state().state.doc.layer(top).unwrap().name().to_owned();
    let from = rect_of(&h, &top_name, |_| true);
    let into = rect_of(&h, "グループ 1", |_| true);
    let start = pos2(from.left() + 90.0, from.center().y);
    drag(
        &mut h,
        &[
            start,
            offset(start, 0.0, 12.0),
            pos2(start.x, into.center().y + 1.0),
        ],
    );
    assert_eq!(
        h.state().state.doc.layer(top).unwrap().parent(),
        Some(group)
    );
    undo(&mut h);
    assert_eq!(h.state().state.doc.layer(top).unwrap().parent(), None);
}

/// ドラッグしている行がホイールで一覧の外へ出ても、ボタンを離せばドラッグは終わり、落とす先の線は残らず、落とす操作は起きる。
#[test]
fn a_layer_drag_ends_cleanly_when_its_row_scrolls_out_of_view() {
    let mut h = app(1280.0, 800.0, 128);
    for _ in 0..30 {
        apply(&mut h, Action::NewLayer);
    }
    let top = h.state().state.selected_layer.unwrap();
    let name = h.state().state.doc.layer(top).unwrap().name().to_owned();
    let before = h.state().state.doc.layers().len();
    let from = rect_of(&h, &name, |_| true);
    let start = pos2(from.left() + 90.0, from.center().y);
    press(&h, start, PointerButton::Primary);
    h.step();
    for dy in [12.0, 40.0] {
        move_to(&h, offset(start, 0.0, dy));
        h.step();
    }
    assert!(
        h.state().state.ui.layer_drag.is_some(),
        "ドラッグが始まった"
    );
    // ホイールで一覧を送る（最後まで）。ドラッグしている行は見えなくなる
    h.state_mut().state.ui.layer_scroll = 100_000.0;
    h.step();
    h.step();
    assert!(h.query_by_label(&name).is_none(), "行は一覧の外");
    assert!(
        h.state()
            .state
            .ui
            .layer_drag
            .is_some_and(|d| d.target.is_some()),
        "ボタンを押しているあいだは、落とす先をポインタに追わせる"
    );
    release(&h, offset(start, 0.0, 40.0), PointerButton::Primary);
    h.step();
    h.step();
    assert!(
        h.state().state.ui.layer_drag.is_none(),
        "離したらドラッグを手放す（線が残り続けない）"
    );
    assert_eq!(h.state().state.doc.layers().len(), before);
    assert!(
        h.state().state.doc.layers().last().map(|l| l.id()) != Some(top),
        "落とす操作は起きる（一番上にいたレイヤーが動いた）"
    );
    undo(&mut h);
    assert_eq!(
        h.state().state.doc.layers().last().map(|l| l.id()),
        Some(top),
        "1 回の取り消しで戻る"
    );
}

/// レイヤーの欄の上の合成モードと不透明度は、欄が狭くて 1 行に収まらないとき（ウィンドウの最小の幅など）は、名前を詰めずに 2 行に積む。
/// ふつうの幅では横に並べる。どちらでも一覧は不透明度の下から始まる。
#[test]
fn the_blend_mode_and_opacity_stack_when_the_layers_panel_is_too_narrow() {
    for lang in Lang::ALL {
        for (width, stacked) in [(1280.0, false), (960.0, true)] {
            let mut h = app(width, 800.0, 128);
            if stacked {
                // 狭い列（今の既定の右の列は最小のウィンドウでも 232 点あるので、前の版の最小のウィンドウと同じ 170 点ほどの幅に動かす）にする
                set_center_share(&mut h, 0.755);
            }
            apply(&mut h, Action::M2Ui(UiOp::Language(lang)));
            let body = layers_body(&h);
            let panel = move |r: egui::Rect| body.contains(r.center());
            let slider = rect_of(&h, lang.pick("不透明度", "Opacity"), panel);
            let blend = rect_of(&h, lang.pick("通常", "Normal"), panel);
            if stacked {
                assert!(
                    slider.top() >= blend.bottom(),
                    "{lang:?} {width}: 積む {slider:?} {blend:?}"
                );
                assert!(
                    slider.left() <= blend.left() + 1.0,
                    "{lang:?} {width}: 合成モードの下に揃える"
                );
            } else {
                assert!(
                    slider.left() >= blend.right(),
                    "{lang:?} {width}: 横に並ぶ {slider:?} {blend:?}"
                );
                assert_eq!(slider.top(), blend.top(), "{lang:?} {width}");
            }
            // 名前は詰めずに出る（最小の幅でも「…」にならない）
            yolu_app::ui::widgets::record_truncations(true);
            yolu_app::ui::widgets::take_truncations();
            h.step();
            let truncated = yolu_app::ui::widgets::take_truncations();
            yolu_app::ui::widgets::record_truncations(false);
            assert!(
                !truncated
                    .iter()
                    .any(|t| t == lang.pick("不透明度", "Opacity")),
                "{lang:?} {width}: {truncated:?}"
            );
            // 一覧は、積んだ分だけ下から始まる
            let layer = h
                .state()
                .state
                .doc
                .layers()
                .first()
                .map(|l| l.name().to_owned())
                .unwrap();
            let row = rect_of(&h, &layer, |r| panel(r) && r.top() >= slider.top());
            assert!(
                row.top() >= slider.bottom(),
                "{lang:?} {width}: 一覧 {row:?} は不透明度 {slider:?} の下"
            );
        }
    }
}

/// スライダーのドラッグを押したまま Esc で止めると、値は押し始めに戻り、Undo の段は残らず、そのドラッグの残りは受けない。
/// 離したあとの次のドラッグは普通に 1 回の Undo になる。
#[test]
fn escape_during_a_slider_drag_restores_the_value_and_leaves_no_undo_step() {
    let mut h = app(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    let opacity = |h: &Harness<'_, YoluApp>| h.state().state.doc.layer(id).unwrap().opacity();
    let steps = h.state().state.doc.undo_count();
    // レイヤーの欄の不透明度（オプションバーの同じ名前より右）
    let slider = rect_of(&h, "不透明度", |r| r.left() > 900.0);
    let at = |f: f32| pos2(slider.left() + slider.width() * f, slider.center().y);
    press(&h, at(0.7), PointerButton::Primary);
    h.step();
    move_to(&h, at(0.4));
    h.step();
    assert!(
        (opacity(&h) - 0.4).abs() < 0.05,
        "動いている: {}",
        opacity(&h)
    );
    assert!(h.state().state.doc.undo_count() > steps);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    h.step();
    assert_eq!(opacity(&h), 1.0, "押し始めの値へ戻る");
    assert_eq!(h.state().state.doc.undo_count(), steps, "段を残さない");
    move_to(&h, at(0.2));
    h.step();
    assert_eq!(opacity(&h), 1.0, "そのドラッグの残りは受けない");
    release(&h, at(0.2), PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(opacity(&h), 1.0);
    assert_eq!(h.state().state.doc.undo_count(), steps);
    // 次のドラッグは普通に効き、1 回の取り消しで戻る
    drag(&mut h, &[at(0.7), at(0.5)]);
    assert!((opacity(&h) - 0.5).abs() < 0.05, "{}", opacity(&h));
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    undo(&mut h);
    assert_eq!(opacity(&h), 1.0);
    // 文書に触らないスライダー（オプションバーの直径）も、押し始めの値へ戻る
    let radius = h.state().state.brush.radius;
    let bar = rect_of(&h, "直径", |r| r.top() < 60.0);
    let at = |f: f32| pos2(bar.left() + bar.width() * f, bar.center().y);
    press(&h, at(0.8), PointerButton::Primary);
    h.step();
    move_to(&h, at(0.6));
    h.step();
    assert!(h.state().state.brush.radius != radius);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    h.step();
    assert_eq!(h.state().state.brush.radius, radius);
    release(&h, at(0.6), PointerButton::Primary);
    h.step();
    h.run();
    assert_eq!(h.state().state.brush.radius, radius);
}

#[test]
fn channels_panel_adds_renames_changes_kind_and_deletes() {
    let mut h = app(1280.0, 800.0, 128);
    click_tab(&mut h, Tab::Channels);
    h.run();
    assert_eq!(h.state().state.doc.channels().len(), 6);
    h.get_by_label("チャンネルを追加").click();
    h.run();
    let at = popup_item(&h, "L8").center();
    click(&mut h, at);
    let ao = h.state().state.m2.paint_channel;
    assert!(!ao.is_standard(), "足したチャンネルを描く先にする");
    assert_eq!(h.state().state.m2.display_channel, ao);
    assert_eq!(
        h.state().state.doc.channel_info(ao).unwrap().name,
        "スカラー 1"
    );
    // 名前（ダブルクリックで変える）
    let row = rect_of(&h, "スカラー 1", |r| r.height() < 40.0);
    let name_at = pos2(row.left() + 100.0, row.center().y);

    for _ in 0..2 {
        press(&h, name_at, egui::PointerButton::Primary);
        release(&h, name_at, egui::PointerButton::Primary);
        h.step();
    }
    h.run();
    assert_eq!(h.state().state.m2.renaming_channel, Some(ao));
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(egui::Event::Text("AO".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.doc.channel_info(ao).unwrap().name, "AO");
    undo(&mut h);
    assert_eq!(
        h.state().state.doc.channel_info(ao).unwrap().name,
        "スカラー 1",
        "名前の変更も 1 回の取り消し"
    );
    // 種類（行の右端の箱。形式の名前）
    h.get_by_label("L8").click();
    h.run();
    let at = popup_item(&h, "RGB8").center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.doc.channel_info(ao).unwrap().kind,
        yolu_app::engine::ChannelKind::Normal
    );
    // 目で 2D の表示を替える
    h.get_by_label("カラー をキャンバスに出す").click();
    h.run();
    assert_eq!(h.state().state.m2.display_channel, Channel::Color);
    assert_eq!(
        h.state().state.m2.paint_channel,
        ao,
        "表示は描く先を変えない"
    );
    // 消す（標準のチャンネルは消せない）
    h.get_by_label("チャンネルを削除").click();
    h.run();
    assert!(h.state().state.doc.channel_info(ao).is_none());
    assert_eq!(h.state().state.m2.paint_channel, Channel::Color);
    undo(&mut h);
    assert!(h.state().state.doc.channel_info(ao).is_some());
}

/// 開いているポップアップの項目のチェックの印。
fn popup_item_checked(h: &Harness<'_, YoluApp>, label: &str) -> bool {
    let at = popup_item(h, label);
    h.get_all_by_label(label)
        .find(|n| n.rect() == at)
        .unwrap()
        .accesskit_node()
        .toggled()
        == Some(egui::accesskit::Toggled::True)
}

/// 合成モードの箱の上の縁の、描くチャンネルだけの値の印（チャンネルの名前の小さな文字。チャンネルの行など、同じ名前のほかの部品より低い）。
fn own_legend(h: &Harness<'_, YoluApp>, name: &str) -> Option<egui::Rect> {
    h.query_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| r.height() < 14.0)
}

/// 描くチャンネルだけの合成モードと不透明度は、合成モードのメニューの頭の項目（チェック）で切り替える。そのチャンネルだけの値のときは、
/// 合成モードの箱の上にチャンネルの名前が出て、合成モードを替えてもレイヤーの値は変わらない。切り替えも 1 回の Undo。
#[test]
fn per_channel_blend_leaves_the_layer_value_alone() {
    let mut h = app(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    click_tab(&mut h, Tab::Channels);
    h.run();
    h.get_by_label("ラフネス を描くチャンネルにする").click();
    h.run();
    assert_eq!(h.state().state.m2.paint_channel, Channel::Roughness);
    let own = |h: &Harness<'_, YoluApp>| {
        !h.state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .channel_blend(Channel::Roughness)
            .is_empty()
    };
    assert!(
        own_legend(&h, "ラフネス").is_none(),
        "レイヤーの値に従う間は名前を出さない"
    );
    // メニューの頭の項目で、そのチャンネル専用の合成にする
    let blend_box = h.get_by_label("通常").rect().center();
    click(&mut h, blend_box);
    h.run();
    assert!(!popup_item_checked(&h, "ラフネス だけの値"));
    let at = popup_item(&h, "ラフネス だけの値").center();
    click(&mut h, at);
    assert!(own(&h));
    assert!(popup_kind(&h).is_none(), "選ぶとメニューは閉じる");
    assert!(
        own_legend(&h, "ラフネス").is_some(),
        "専用のときは箱の上に名前"
    );
    // 合成モードを替える
    h.get_by_label("通常").click();
    h.run();
    assert!(popup_item_checked(&h, "ラフネス だけの値"));
    let at = popup_item(&h, "乗算").center();
    click(&mut h, at);
    let layer = h.state().state.doc.layer(id).unwrap();
    assert_eq!(layer.blend_mode_in(Channel::Roughness), BlendMode::Multiply);
    assert_eq!(
        layer.blend_mode(),
        BlendMode::Normal,
        "レイヤーの値は変わらない"
    );
    undo(&mut h);
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .blend_mode_in(Channel::Roughness),
        BlendMode::Normal
    );
    assert!(own(&h), "合成モードの取り消しは専用のまま");
    // レイヤーの値に戻す（同じ項目のチェックを外す）。戻すのも 1 回の Undo
    h.get_by_label("通常").click();
    h.run();
    let at = popup_item(&h, "ラフネス だけの値").center();
    click(&mut h, at);
    assert!(!own(&h));
    assert!(own_legend(&h, "ラフネス").is_none());
    undo(&mut h);
    assert!(own(&h));
    undo(&mut h);
    assert!(!own(&h), "専用にしたのも 1 回の Undo");
    // 英語: 項目は「<名前> only」、印はチャンネルの名前
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    click(&mut h, blend_box);
    let at = popup_item(&h, "Roughness only").center();
    click(&mut h, at);
    assert!(own(&h));
    assert!(own_legend(&h, "Roughness").is_some());
}

#[test]
fn painting_goes_to_the_paint_channel_and_the_canvas_shows_the_display_channel() {
    let mut h = app(1280.0, 800.0, 128);
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    apply(&mut h, Action::M2Ui(UiOp::DisplayChannel(Channel::Color)));
    stroke_across(&mut h, 0.0);
    let c = canvas_rect(&h).center();
    assert_eq!(canvas_pixel(&h, c)[3], 0, "カラーには描いていない");
    let doc = &h.state().state.doc;
    let (x, y) = {
        let r = canvas_rect(&h);
        h.state().state.view.view(r, 128, 128).to_canvas(c)
    };
    let rough = doc
        .composite_pixel(Channel::Roughness, x as u32, y as u32)
        .unwrap();
    assert_eq!(rough.a, 255, "描くチャンネルに描いた");
    let before = h.state().display().stats.total_tiles;
    apply(
        &mut h,
        Action::M2Ui(UiOp::DisplayChannel(Channel::Roughness)),
    );
    assert!(
        h.state().display().stats.total_tiles > before,
        "表示するチャンネルを替えると全部を作り直す"
    );
    undo(&mut h);
    assert!(h.state().state.doc.layers().iter().all(|l| l
        .surface(Channel::Roughness)
        .is_none_or(|s| s.tile_count() == 0)));
}

#[test]
fn a_layer_that_cannot_be_painted_says_why() {
    let mut h = app(1280.0, 800.0, 128);
    toolbar_new_fill(&mut h);
    stroke_across(&mut h, 0.0);
    let undo_steps = h.state().state.doc.undo_count();
    stroke_across(&mut h, 0.0);
    let state = &h.state().state;
    assert!(state.message.contains("塗りつぶし"), "{}", state.message);
    assert_eq!(state.doc.undo_count(), undo_steps, "何も描いていない");
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    stroke_across(&mut h, 0.0);
    assert!(
        h.state().state.message.contains("Fill"),
        "{}",
        h.state().state.message
    );
}

#[test]
fn headless_m2_edits_are_one_undo_each() {
    use yolu_app::state::AppState;
    let mut s = AppState::new(64, 64);
    let id = s.selected_layer.unwrap();
    for edit in [
        Edit::AddMask(id),
        Edit::Clipping(id, true),
        Edit::Duplicate(id),
    ] {
        let before = s.doc.undo_count();
        s.apply(Action::M2(edit));
        assert_eq!(s.doc.undo_count(), before + 1);
    }
}

/// マスクに描くあいだも、プロパティの欄のタブ（ステンシル・レイヤーの 2 つ）は替えない。マスクの欄はツールプロパティの「塗るチャンネル」の区分に出る。
#[test]
fn headless_painting_on_a_mask_leaves_the_properties_tab_alone() {
    for tab in [0, 1] {
        let mut s = AppState::new(64, 64);
        let id = s.selected_layer.unwrap();
        s.ui.property_tab = tab;
        // マスクを足す → マスクに描く。やめる → もう一度 → マスクを消す
        s.apply(Action::M2(Edit::AddMask(id)));
        assert!(s.m2.edit_mask);
        assert_eq!(s.ui.property_tab, tab, "マスクを足す");
        s.apply(Action::M2Ui(UiOp::EditMask(false)));
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (false, tab));
        s.apply(Action::M2Ui(UiOp::EditMask(true)));
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (true, tab));
        s.apply(Action::M2(Edit::RemoveMask(id)));
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (false, tab));
        // マスクを足したあとの取り消し
        s.apply(Action::M2(Edit::AddMask(id)));
        s.apply(Action::Undo);
        s.apply(Action::Undo);
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (false, tab));
        // マスクの編集中に新しいレイヤーを足す・マスクのあるレイヤーを消す
        s.apply(Action::M2(Edit::AddMask(id)));
        s.apply(Action::NewLayer);
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (false, tab));
        s.selected_layer = Some(id);
        s.apply(Action::M2Ui(UiOp::EditMask(true)));
        s.apply(Action::DeleteLayer);
        assert_eq!((s.m2.edit_mask, s.ui.property_tab), (false, tab));
    }
}

#[test]
fn selecting_another_layer_row_leaves_mask_editing_and_the_properties_tab() {
    let mut h = app(1280.0, 800.0, 128);
    let first = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::NewLayer);
    let top = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(Edit::AddMask(top)));
    assert!(h.state().state.m2.edit_mask);
    let tab = h.state().state.ui.property_tab;
    let name = h.state().state.doc.layer(first).unwrap().name().to_owned();
    let row = rect_of(&h, &name, |_| true);
    click(&mut h, pos2(row.left() + 90.0, row.center().y));
    assert_eq!(h.state().state.selected_layer, Some(first));
    assert!(!h.state().state.m2.edit_mask);
    assert_eq!(h.state().state.ui.property_tab, tab, "タブは替えない");
}

// ───────── グループを含む削除と上へ・下へ ─────────

use yolu_app::engine::LayerId;

/// 入れ子のグループの文書。下から `G1 { G2 { A }, C }`、その上に `B`（一番上の段は G1 と B）。
struct Nest {
    a: LayerId,
    g2: LayerId,
    c: LayerId,
    g1: LayerId,
    b: LayerId,
}

fn nest(s: &mut AppState) -> Nest {
    let a = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::NewGroup));
    let g2 = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::Move {
        id: a,
        parent: Some(g2),
        position: 0,
    }));
    s.apply(Action::M2(Edit::NewGroup));
    let g1 = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::Move {
        id: g2,
        parent: Some(g1),
        position: 0,
    }));
    s.apply(Action::NewLayer);
    let b = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let c = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::Move {
        id: c,
        parent: Some(g1),
        position: 1,
    }));
    Nest { a, g2, c, g1, b }
}

/// レイヤーの並び（下から）と、それぞれの親。
fn structure(s: &AppState) -> Vec<(LayerId, Option<LayerId>)> {
    s.doc
        .layers()
        .iter()
        .map(|l| (l.id(), l.parent()))
        .collect()
}

#[test]
fn headless_nest_is_built_as_documented() {
    let mut s = AppState::new(64, 64);
    let n = nest(&mut s);
    assert_eq!(
        structure(&s),
        [
            (n.a, Some(n.g2)),
            (n.g2, Some(n.g1)),
            (n.c, Some(n.g1)),
            (n.g1, None),
            (n.b, None)
        ]
    );
}

#[test]
fn headless_deleting_a_group_takes_its_contents_and_undo_brings_them_back() {
    let mut s = AppState::new(64, 64);
    let n = nest(&mut s);
    // 一番下に別のレイヤー D（G1 の塊の下）
    s.apply(Action::NewLayer);
    let d = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::Move {
        id: d,
        parent: None,
        position: 0,
    }));
    let before = structure(&s);
    assert_eq!(before.len(), 6);
    let steps = s.doc.undo_count();
    s.selected_layer = Some(n.g1);
    s.apply(Action::DeleteLayer);
    assert_eq!(
        structure(&s),
        [(d, None), (n.b, None)],
        "入れ子の中身ごと消える"
    );
    assert_eq!(s.doc.undo_count(), steps + 1, "1 回の取り消し");
    assert_eq!(
        s.selected_layer,
        Some(d),
        "消えた塊のすぐ下のレイヤーを選ぶ"
    );
    s.apply(Action::Undo);
    assert_eq!(structure(&s), before, "中身も入れ子も元どおり");
    assert!(s.selected_layer.is_some_and(|id| s.doc.layer(id).is_some()));
    // グループの中の一番上のレイヤーを消すと、すぐ下を選ぶ。グループは残る
    s.selected_layer = Some(n.c);
    s.apply(Action::DeleteLayer);
    assert_eq!(s.selected_layer, Some(n.g2));
    assert!(s.doc.layer(n.c).is_none() && s.doc.layer(n.g1).is_some());
}

#[test]
fn headless_a_document_that_is_only_one_group_cannot_lose_it() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let a = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::NewGroup));
        let g2 = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Move {
            id: a,
            parent: Some(g2),
            position: 0,
        }));
        s.apply(Action::M2(Edit::NewGroup));
        let g1 = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Move {
            id: g2,
            parent: Some(g1),
            position: 0,
        }));
        assert_eq!(s.doc.layers().len(), 3);
        let (before, steps) = (structure(&s), s.doc.undo_count());
        s.selected_layer = Some(g1);
        s.message.clear();
        s.apply(Action::DeleteLayer);
        assert_eq!(
            s.message,
            lang.pick(
                "最後のレイヤーは消せません。",
                "Cannot delete the last layer."
            )
        );
        assert_eq!((structure(&s), s.doc.undo_count()), (before, steps));
        // 中のグループを消すと、外のグループは残る（空のグループになる）
        s.selected_layer = Some(g2);
        s.apply(Action::DeleteLayer);
        assert_eq!(structure(&s), [(g1, None)]);
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), 3);
    }
}

#[test]
fn headless_layer_up_and_down_stop_at_the_group_edge_and_move_a_group_whole() {
    let mut s = AppState::new(64, 64);
    let n = nest(&mut s);
    let before = structure(&s);
    let steps = s.doc.undo_count();
    // G1 の中の一番上（C）は上へ動かない・一番下（G2）は下へ動かない（外へ出ない）
    s.selected_layer = Some(n.c);
    s.apply(Action::LayerUp);
    s.selected_layer = Some(n.g2);
    s.apply(Action::LayerDown);
    assert_eq!((structure(&s), s.doc.undo_count()), (before.clone(), steps));
    // 兄弟の中では動く（G2 が C の上へ。親は G1 のまま）
    s.apply(Action::LayerUp);
    assert_eq!(s.doc.children_of(Some(n.g1)).unwrap(), [n.c, n.g2]);
    assert_eq!(s.doc.layer(n.g2).unwrap().parent(), Some(n.g1));
    assert_eq!(
        s.doc.layer(n.a).unwrap().parent(),
        Some(n.g2),
        "中身は付いていく"
    );
    assert_eq!(s.doc.undo_count(), steps + 1, "1 回の取り消し");
    s.apply(Action::Undo);
    assert_eq!(structure(&s), before);
    // 一番上の段では G1 ごと B の上へ（中身は G1 の中のまま）。一番上の B はそれ以上上がらない
    s.selected_layer = Some(n.g1);
    s.apply(Action::LayerUp);
    assert_eq!(s.doc.children_of(None).unwrap(), [n.b, n.g1]);
    assert_eq!(s.doc.children_of(Some(n.g1)).unwrap(), [n.g2, n.c]);
    let steps = s.doc.undo_count();
    s.apply(Action::LayerUp);
    assert_eq!(s.doc.undo_count(), steps, "一番上の段の一番上は動かない");
    s.apply(Action::Undo);
    assert_eq!(structure(&s), before);
}

/// 木・マスク・塗りつぶし・調整の行を並べた文書。
fn tree_document(h: &mut Harness<'_, YoluApp>) {
    apply(h, Action::M2(Edit::GroupSelected));
    apply(h, Action::NewLayer);
    let layer = h.state().state.selected_layer.unwrap();
    apply(h, Action::M2(Edit::AddMask(layer)));
    apply(h, Action::M2Ui(UiOp::EditMask(false)));
    apply(h, Action::M2(Edit::NewFill));
    apply(
        h,
        Action::M2(Edit::NewAdjustment(yolu_app::m2::AdjustmentKind::Invert)),
    );
}

#[test]
fn snapshot_layers_tree() {
    let mut h = app(1280.0, 800.0, 256);
    stroke_across(&mut h, 0.0);
    tree_document(&mut h);
    h.snapshot("m2_layers_tree");
}

#[test]
fn snapshot_channels_panel() {
    let mut h = app(1280.0, 800.0, 256);
    click_tab(&mut h, Tab::Channels);
    apply(
        &mut h,
        Action::M2(Edit::AddChannel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))),
    );
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    h.snapshot("m2_channels_panel");
}

/// 見た目を lilToon にして、影・発光・マットキャップ・リムライト・ラメを入にしてひな形のチャンネルを作り、ラメだけを切る。どのスロットも
/// 読まないユーザーチャンネルを 1 つ追加する（「そのほか」）。
fn liltoon_channels(lang: Lang) -> Harness<'static, YoluApp> {
    use yolu_core::look::{LookKind, LookValue};
    // （チャンネルの組の全部の行が入る高さで）
    let mut h = app(1280.0, 1700.0, 256);
    apply(&mut h, Action::M2Ui(UiOp::Language(lang)));
    {
        let doc = &mut h.state_mut().state.doc;
        let mut look = doc.look().clone();
        look.kind = LookKind::LilToon;
        for p in [
            "_UseShadow",
            "_UseEmission",
            "_UseMatCap",
            "_UseRim",
            "_UseGlitter",
        ] {
            look.properties.insert(p.into(), LookValue::Float(1.0));
        }
        doc.set_look(look, false).unwrap();
        yolu_app::look::apply_template(doc, lang).unwrap();
        let mut look = doc.look().clone();
        look.properties
            .insert("_UseGlitter".into(), LookValue::Float(0.0));
        doc.set_look(look, false).unwrap();
    }
    apply(
        &mut h,
        Action::M2(Edit::AddChannel(yolu_app::m2::new_channel_info(
            lang.pick("スカラー 1", "Scalar 1").into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))),
    );
    click_tab(&mut h, Tab::Channels);
    h.run();
    h
}

/// チャンネルの欄のまとまりの見出し（上から）と、開いているか。見出しは開閉の印を持つ低いボタン（行の部品より低い）。
fn channel_groups(h: &Harness<'_, YoluApp>) -> Vec<(String, bool)> {
    // チャンネルの組の中だけ（右の列の上の組。同じ列のレイヤーなどのボタンは含めない）
    let body = top_right_body(h);
    let mut out: Vec<(f32, String, bool)> = h
        .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::Button))
        .filter(|n| {
            let r = n.rect();
            body.contains(r.center()) && (r.height() - 20.0).abs() < 1.0
        })
        .filter_map(|n| {
            let node = n.accesskit_node();
            let open = node.toggled()? == egui::accesskit::Toggled::True;
            Some((n.rect().top(), node.label()?.to_string(), open))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, l, o)| (l, o)).collect()
}

/// 見た目が lilToon なら、ユーザーチャンネルは読むスロットの部位ごとのまとまり（lilToon のインスペクターの節の並び、最後に「そのほか」）。
/// 標準の 6 つは上のまま。見た目がその部位の機能を切っているまとまりは、開いたことが無ければたたむ。見出しを押すと開閉し、描くチャンネルが
/// たたんだまとまりにあると見出しの左に印。
#[test]
fn liltoon_user_channels_are_grouped_by_the_part_that_reads_them() {
    let mut h = liltoon_channels(Lang::Ja);
    let groups = channel_groups(&h);
    assert_eq!(
        groups,
        [
            ("影", true),
            ("発光", true),
            ("マットキャップ", true),
            ("リムライト", true),
            ("ラメ", false),
            ("そのほか", true),
        ]
        .map(|(l, o)| (l.to_string(), o)),
        "インスペクターの並び。切っている部位（ラメ）はたたむ"
    );
    let row = |h: &Harness<'_, YoluApp>, name: &str| {
        let body = top_right_body(h);
        h.query_all_by_label(name)
            .map(|n| n.rect())
            .find(|r| body.contains(r.center()) && (r.height() - 28.0).abs() < 1.0)
    };
    let header = |h: &Harness<'_, YoluApp>, name: &str| {
        let body = top_right_body(h);
        h.get_all_by_label(name)
            .map(|n| n.rect())
            .find(|r| body.contains(r.center()) && r.height() < 24.0)
            .unwrap()
    };
    // 標準の 6 つは見出しより上
    let first = header(&h, "影");
    for name in [
        "カラー",
        "ラフネス",
        "メタリック",
        "ハイト",
        "ノーマル",
        "エミッション",
    ] {
        assert!(
            row(&h, name).unwrap().bottom() <= first.top() + 0.5,
            "{name}"
        );
    }
    // 開いたまとまりの行はその見出しの下、たたんだまとまりの行は出ない
    let shadow = row(&h, "影の強度").unwrap();
    assert!(shadow.top() >= first.bottom() - 0.5);
    assert!(row(&h, "スカラー 1").unwrap().top() > header(&h, "そのほか").top());
    assert!(row(&h, "ラメのマスク").is_none());
    // 押すと開閉（画面の状態。文書は変わらない）
    let steps = h.state().state.doc.undo_count();
    let at = header(&h, "影").center();
    click(&mut h, at);
    assert!(row(&h, "影の強度").is_none());
    assert_eq!(channel_groups(&h)[0], ("影".to_string(), false));
    let at = header(&h, "ラメ").center();
    click(&mut h, at);
    assert!(row(&h, "ラメのマスク").is_some());
    let at = header(&h, "影").center();
    click(&mut h, at);
    assert!(row(&h, "影の強度").is_some());
    assert_eq!(h.state().state.doc.undo_count(), steps);
    // 描くチャンネルがたたんだまとまりにあると、見出しの左に印（開いていれば行が見えるので出さない）
    let accent_at = |h: &mut Harness<'_, YoluApp>, name: &str| {
        let r = header(h, name);
        let img = h.render().unwrap();
        let px = img.get_pixel((r.left() - 8.0 + 1.0) as u32, r.center().y as u32);
        [px[0], px[1], px[2]] == [0x3D, 0x8E, 0xF0]
    };
    let glitter = h
        .state()
        .state
        .doc
        .channels()
        .into_iter()
        .find(|c| h.state().state.doc.channel_info(*c).unwrap().name == "ラメのマスク")
        .unwrap();
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(glitter)));
    assert!(!accent_at(&mut h, "ラメ"), "開いているときは印なし");
    let at = header(&h, "ラメ").center();
    click(&mut h, at);
    assert!(accent_at(&mut h, "ラメ"));
    assert!(!accent_at(&mut h, "影"));
    h.snapshot("channels_groups_ja");
    // 英語
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    let names: Vec<String> = channel_groups(&h).into_iter().map(|(l, _)| l).collect();
    assert_eq!(
        names,
        [
            "Shadow",
            "Emission",
            "MatCap",
            "Rim Light",
            "Glitter",
            "Other"
        ]
    );
    h.snapshot("channels_groups_en");
}

/// 見た目が lilToon でないとき・どのスロットも読まないユーザーチャンネルだけのときは、まとめない（見出しを出さない）。
#[test]
fn channels_are_not_grouped_without_liltoon_parts() {
    use yolu_core::look::LookKind;
    // lilToon でも、どのスロットも読まないユーザーチャンネルだけ
    let mut h = app(1280.0, 1000.0, 256);
    assert_eq!(h.state().state.doc.look().kind, LookKind::LilToon);
    apply(
        &mut h,
        Action::M2(Edit::AddChannel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))),
    );
    click_tab(&mut h, Tab::Channels);
    h.run();
    assert!(channel_groups(&h).is_empty());
    assert!(h.query_by_label("AO").is_some());
    // 見た目が標準（PBR）なら、スロットを読むチャンネルがあってもまとめない
    let mut h = liltoon_channels(Lang::Ja);
    assert!(!channel_groups(&h).is_empty());
    {
        let doc = &mut h.state_mut().state.doc;
        let mut look = doc.look().clone();
        look.kind = LookKind::Standard;
        doc.set_look(look, false).unwrap();
    }
    h.run();
    assert!(channel_groups(&h).is_empty());
    for name in ["影の強度", "ラメのマスク", "スカラー 1"] {
        assert!(h.query_by_label(name).is_some(), "{name}");
    }
}

#[test]
fn snapshot_properties_layer_kinds() {
    let mut h = app(1280.0, 1000.0, 256);
    apply(&mut h, Action::M2(Edit::NewFill));
    h.snapshot("m2_properties_fill");
    apply(
        &mut h,
        Action::M2(Edit::NewAdjustment(yolu_app::m2::AdjustmentKind::Levels)),
    );
    h.snapshot("m2_properties_levels");
    apply(&mut h, Action::M2(Edit::NewGroup));
    h.snapshot("m2_properties_group");
}

#[test]
fn snapshot_properties_mask_and_english() {
    let mut h = app(1280.0, 1000.0, 256);
    let id = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(Edit::AddMask(id)));
    // プロパティのレイヤーのタブ（レイヤーマスクの節はここにもある）
    h.state_mut().state.ui.property_tab = 1;
    h.run();
    h.snapshot("m2_properties_mask");
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    h.snapshot("m2_properties_mask_english");
}

fn canvas_alpha(h: &Harness<'_, YoluApp>, dx: f32) -> u8 {
    let c = canvas_rect(h).center();
    canvas_pixel(h, offset(c, dx, 0.0))[3]
}

#[test]
fn the_mask_hides_inverts_and_switches_off_with_one_undo_each() {
    // （マスクの欄は、マスクに描くあいだ、ツールプロパティのブラシの欄の下に出る。入る高さで）
    let mut h = app(1280.0, 1500.0, 128);
    {
        let b = &mut h.state_mut().state.brush;
        b.radius = 3.0;
        b.hardness = 1.0;
    }
    stroke_across(&mut h, 0.0); // レイヤーに長い線
    assert_eq!(canvas_alpha(&h, 0.0), 255);
    assert_eq!(canvas_alpha(&h, 50.0), 255);
    h.get_by_label("レイヤーマスクを追加").click();
    h.run();
    // マスクに短く描く（塗ると隠す）
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -10.0, 0.0), offset(c, 10.0, 0.0)]);
    let id = h.state().state.selected_layer.unwrap();
    assert!(
        h.state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .tile_count()
            > 0
    );
    assert_eq!(canvas_alpha(&h, 0.0), 0, "マスクを塗った所は隠れる");
    assert_eq!(canvas_alpha(&h, 50.0), 255, "ほかは見えたまま");
    // ツールプロパティのマスクの欄の反転・有効
    h.get_by_label("反転").click();
    h.run();
    assert_eq!(canvas_alpha(&h, 0.0), 255, "反転すると塗った所だけが見える");
    assert_eq!(canvas_alpha(&h, 50.0), 0);
    h.get_by_label("有効").click();
    h.run();
    assert_eq!(canvas_alpha(&h, 50.0), 255, "切るとマスクなしと同じ");
    undo(&mut h);
    assert_eq!(canvas_alpha(&h, 50.0), 0, "有効の切り替えは 1 回の取り消し");
    undo(&mut h);
    assert_eq!(canvas_alpha(&h, 0.0), 0, "反転も 1 回の取り消し");
    // マスクを消すと見える
    h.get_by_label("レイヤーマスクを削除").click();
    h.run();
    assert!(h.state().state.doc.layer(id).unwrap().mask().is_none());
    assert_eq!(canvas_alpha(&h, 0.0), 255);
    undo(&mut h);
    assert!(h.state().state.doc.layer(id).unwrap().mask().is_some());
}

#[test]
fn fill_and_adjustment_properties_edit_and_undo() {
    let mut h = app(1280.0, 1600.0, 128);
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    toolbar_new_fill(&mut h);
    let fill = h.state().state.selected_layer.unwrap();
    // 作ったときは描画色
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(fill)
            .unwrap()
            .fill_value(Channel::Color),
        Some(yolu_app::engine::Rgba8::new(255, 0, 0, 255))
    );
    // 描画色を替えて、値の見本を押して色のウィンドウの「描画色を入れる」を押すと、値が描画色になる
    h.state_mut().state.color.set_main([0.0, 0.0, 1.0, 1.0]);
    h.get_by_label("カラー の値").click();
    h.run();
    h.get_by_label("描画色を入れる").click();
    h.run();
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(fill)
            .unwrap()
            .fill_value(Channel::Color),
        Some(yolu_app::engine::Rgba8::new(0, 0, 255, 255))
    );
    undo(&mut h);
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(fill)
            .unwrap()
            .fill_value(Channel::Color),
        Some(yolu_app::engine::Rgba8::new(255, 0, 0, 255))
    );
    // ほかのチャンネルの値を足す・外す
    h.get_by_label_contains("ラフネス の値を追加").click();
    h.run();
    assert!(h
        .state()
        .state
        .doc
        .layer(fill)
        .unwrap()
        .fill_value(Channel::Roughness)
        .is_some());
    h.get_by_label_contains("ラフネス の値を外す").click();
    h.run();
    assert!(h
        .state()
        .state
        .doc
        .layer(fill)
        .unwrap()
        .fill_value(Channel::Roughness)
        .is_none());
    // 調整: ガンマのドラッグは 1 回の取り消しにまとまる
    h.get_by_label("新規調整レイヤー").click();
    h.run();
    let at = popup_item(&h, "レベル補正").center();
    click(&mut h, at);
    let adj = h.state().state.selected_layer.unwrap();
    let slider = h.get_by_label("ガンマ").rect();
    drag(
        &mut h,
        &[
            pos2(slider.left() + slider.width() * 0.2, slider.center().y),
            pos2(slider.left() + slider.width() * 0.5, slider.center().y),
            pos2(slider.left() + slider.width() * 0.8, slider.center().y),
        ],
    );
    let gamma = h
        .state()
        .state
        .doc
        .layer(adj)
        .unwrap()
        .adjustment()
        .unwrap()
        .gamma();
    assert!(gamma > 2.0, "{gamma}");
    undo(&mut h);
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(adj)
            .unwrap()
            .adjustment()
            .unwrap()
            .gamma(),
        1.0
    );
}

#[test]
fn the_ui_stroke_is_the_core_stroke_with_the_same_brush() {
    use yolu_app::engine::{BrushSample, DVec2, Document};
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.m2.random_seed = false;
    let chalk = yolu_app::m2::presets()
        .iter()
        .position(|p| p.id == "chalk")
        .unwrap();
    apply(&mut h, Action::M2Ui(UiOp::Preset(chalk)));
    {
        let s = &mut h.state_mut().state;
        s.m2.brush.jitter.scatter = 0.5;
        s.m2.brush.assist.stabilizer = 6.0;
        s.m2.brush.assist.curve = true;
        s.color.set_main([0.2, 0.6, 0.3, 1.0]);
    }
    let brush = h.state_mut().state.stroke_brush(false);
    let r = canvas_rect(&h);
    let c = r.center();
    let points = [
        offset(c, -50.0, -10.0),
        offset(c, -20.0, 15.0),
        offset(c, 10.0, -12.0),
        offset(c, 45.0, 8.0),
    ];
    drag(&mut h, &points);
    let view = h.state().state.view.view(r, 128, 128);
    let mut doc = Document::new(128, 128).unwrap();
    let id = doc.add_layer("x").unwrap();
    let mut stroke = doc.begin_brush_stroke(id, &brush).unwrap();
    for p in points {
        let (x, y) = view.to_canvas(p);
        stroke
            .add_sample(
                &mut doc,
                BrushSample::new(x, y, 1.0, 0.0, DVec2::ZERO).unwrap(),
            )
            .unwrap();
    }
    doc.end_stroke(stroke).unwrap();
    let want = doc.composite(doc.bounds()).unwrap();
    let got = h.state().state.doc.composite(doc.bounds()).unwrap();
    assert!(want.chunks(4).any(|p| p[3] > 0), "何か描けている");
    assert!(
        want == got,
        "画面から描いた線は、同じブラシを core へ直に渡した線と同じ"
    );
    // プリセットを替えれば線も替わる（照合が空でない）
    h.state_mut().state.apply(Action::Undo);
    apply(&mut h, Action::M2Ui(UiOp::Preset(0)));
    drag(&mut h, &points);
    assert!(h.state().state.doc.composite(doc.bounds()).unwrap() != want);
}

fn pen(
    pos: egui::Pos2,
    contact: bool,
    rotation: Option<f32>,
    time_ms: u32,
) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [pos.x, pos.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 9,
        time_ms,
    }
}

#[test]
fn the_pens_rotation_and_time_reach_the_brush() {
    use yolu_app::engine::{BrushSample, DVec2, Document};
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.m2.random_seed = false;
    {
        let s = &mut h.state_mut().state;
        s.brush.hardness = 1.0;
        s.brush.radius = 10.0;
        s.m2.brush.tip.roundness = 0.25;
        s.m2.brush.controls.rotation_angle = true;
        s.m2.brush.controls.speed_size = true;
        s.m2.brush.controls.speed_max = 400.0;
    }
    let brush = h.state_mut().state.stroke_brush(false);
    let r = canvas_rect(&h);
    let view = h.state().state.view.view(r, 128, 128);
    let c = r.center();
    let line = [
        offset(c, -40.0, 0.0),
        offset(c, 0.0, 0.0),
        offset(c, 40.0, 0.0),
    ];
    let reference = |rotation: f64, times: [f64; 3]| {
        let mut doc = Document::new(128, 128).unwrap();
        let id = doc.add_layer("x").unwrap();
        let mut stroke = doc.begin_brush_stroke(id, &brush).unwrap();
        for (p, t) in line.iter().zip(times) {
            let (x, y) = view.to_canvas(*p);
            let sample = BrushSample::new(x, y, 1.0, t, DVec2::ZERO)
                .unwrap()
                .with_rotation(rotation)
                .unwrap();
            stroke.add_sample(&mut doc, sample).unwrap();
        }
        doc.end_stroke(stroke).unwrap();
        doc.composite(doc.bounds()).unwrap()
    };
    let paint = |h: &mut Harness<'_, YoluApp>, rotation: Option<f32>, times: [u32; 3]| {
        for (i, p) in line.iter().enumerate() {
            h.state().pen().push(pen(*p, true, rotation, times[i]));
        }
        h.state()
            .pen()
            .push(pen(line[2], false, rotation, times[2]));
        h.run();
        let bounds = h.state().state.doc.bounds();
        let pixels = h.state().state.doc.composite(bounds).unwrap();
        h.state_mut().state.apply(Action::Undo);
        h.run();
        pixels
    };
    // ペンの軸の回転は、画面で時計回りの度 → キャンバスで反時計回りのラジアン（−π/4）で core へ渡る
    let rotated = paint(&mut h, Some(45.0), [0, 100, 200]);
    assert!(
        rotated == reference(-std::f64::consts::FRAC_PI_4, [0.0, 0.1, 0.2]),
        "時計回り 45° は反時計回り −45°"
    );
    assert!(
        rotated != paint(&mut h, None, [0, 100, 200]),
        "回転で絵が変わる"
    );
    // 時刻（ミリ秒 → 秒）で筆の速さが決まる: 速く動かすと小さく
    let slow = paint(&mut h, None, [0, 2000, 4000]);
    let fast = paint(&mut h, None, [0, 20, 40]);
    assert!(fast == reference(0.0, [0.0, 0.02, 0.04]));
    let ink = |p: &[u8]| p.chunks(4).map(|c| c[3] as u64).sum::<u64>();
    assert!(ink(&fast) < ink(&slow), "{} < {}", ink(&fast), ink(&slow));
}

/// 1 本の線を core へ直に渡した絵（画面の点を `view` でキャンバスへ。回転は Some のときだけ付ける。時刻は秒）。
fn core_line(
    brush: &yolu_app::engine::Brush,
    view: &yolu_app::canvas::view::CanvasView,
    points: &[egui::Pos2],
    rotation: Option<f64>,
    times: &[f64],
) -> Vec<u8> {
    use yolu_app::engine::{BrushSample, DVec2, Document};
    let mut doc = Document::new(128, 128).unwrap();
    let id = doc.add_layer("x").unwrap();
    let mut stroke = doc.begin_brush_stroke(id, brush).unwrap();
    for (p, t) in points.iter().zip(times) {
        let (x, y) = view.to_canvas(*p);
        let mut sample = BrushSample::new(x, y, 1.0, *t, DVec2::ZERO).unwrap();
        if let Some(r) = rotation {
            sample = sample.with_rotation(r).unwrap();
        }
        stroke.add_sample(&mut doc, sample).unwrap();
    }
    doc.end_stroke(stroke).unwrap();
    doc.composite(doc.bounds()).unwrap()
}

/// 回転の情報が無い入力（マウス・回転を送れないペン）は、表示を回していても反転していても、core へ回転を渡さない。
/// 回転を送るペンの 0° は、表示の向きに合わせて直した角度で渡る。
#[test]
fn input_without_rotation_gets_none_on_a_rotated_flipped_view() {
    let mut h = app(1280.0, 800.0, 128);
    {
        let s = &mut h.state_mut().state;
        s.m2.random_seed = false;
        s.brush.hardness = 1.0;
        s.brush.radius = 10.0;
        s.m2.brush.tip.roundness = 0.25;
        s.m2.brush.controls.rotation_angle = true;
        s.view.angle = 90.0;
        s.view.flip = true;
    }
    h.run();
    let brush = h.state_mut().state.stroke_brush(false);
    let r = canvas_rect(&h);
    let view = h.state().state.view.view(r, 128, 128);
    let c = r.center();
    let line = [
        offset(c, -40.0, -10.0),
        offset(c, 0.0, 0.0),
        offset(c, 40.0, 10.0),
    ];
    let canvas_zero = view.rotation_to_canvas(0.0);
    assert!(
        canvas_zero.abs() > 0.1,
        "この表示では 0° が 0 にならない: {canvas_zero}"
    );
    let want_none = core_line(&brush, &view, &line, None, &[0.0; 3]);
    let want_zero = core_line(&brush, &view, &line, Some(canvas_zero), &[0.0; 3]);
    assert!(want_none != want_zero, "回転が絵に出る（照合が空でない）");
    let pixels = |h: &Harness<'_, YoluApp>| {
        let bounds = h.state().state.doc.bounds();
        h.state().state.doc.composite(bounds).unwrap()
    };
    // マウス
    drag(&mut h, &line);
    assert!(pixels(&h) == want_none, "マウスは回転を渡さない");
    undo(&mut h);
    // 回転を送れないペン
    for (i, p) in line.iter().enumerate() {
        h.state().pen().push(pen(*p, true, None, i as u32));
    }
    h.state().pen().push(pen(line[2], false, None, 3));
    h.run();
    assert!(
        pixels(&h) == want_none,
        "回転を送れないペンは回転を渡さない"
    );
    undo(&mut h);
    // 回転を送るペンの 0°（情報あり）
    for (i, p) in line.iter().enumerate() {
        h.state().pen().push(pen(*p, true, Some(0.0), i as u32));
    }
    h.state().pen().push(pen(line[2], false, Some(0.0), 3));
    h.run();
    assert!(
        pixels(&h) == want_zero,
        "回転を送るペンの 0° は、表示の向きを直した角度で渡る"
    );
}

/// マウスの速さの制御: 同じ速さで動かしたなら、1 フレームに来るイベントの数（マウスの報告の頻度）によらず同じ絵になる
/// （イベントごとの時刻はフレームの間を等分する）。
#[test]
fn mouse_speed_does_not_depend_on_how_many_events_arrive_per_frame() {
    let mut h = app(1280.0, 800.0, 128);
    {
        let s = &mut h.state_mut().state;
        s.m2.random_seed = false;
        s.brush.hardness = 1.0;
        s.brush.radius = 8.0;
        s.m2.brush.controls.speed_size = true;
        s.m2.brush.controls.speed_max = 150.0;
    }
    let r = canvas_rect(&h);
    let c = r.center();
    let ink = |h: &Harness<'_, YoluApp>| {
        let bounds = h.state().state.doc.bounds();
        h.state()
            .state
            .doc
            .composite(bounds)
            .unwrap()
            .chunks(4)
            .map(|p| p[3] as u64)
            .sum::<u64>()
    };
    // 1 フレームに 8 点（画面の点）ずつ進む線を、1 フレーム 1 イベント・2 イベント・4 イベントで
    let frames = 14;
    let per_frame = 8.0;
    let run = |h: &mut Harness<'_, YoluApp>, events: usize| {
        let at = |i: usize| offset(c, -60.0 + per_frame * i as f32 / events as f32, 0.0);
        press(h, at(0), PointerButton::Primary);
        h.step();
        for frame in 0..frames {
            // 1 フレームに複数のイベント（ハーネスの step は、待たせたイベントを 1 つずつ別のフレームにするので、直に入れる）
            for e in 1..=events {
                h.input_mut()
                    .events
                    .push(egui::Event::PointerMoved(at(frame * events + e)));
            }
            h.step();
        }
        release(h, at(frames * events), PointerButton::Primary);
        h.step();
        h.run();
        let v = ink(h);
        undo(h);
        v
    };
    let one = run(&mut h, 1);
    let two = run(&mut h, 2);
    let four = run(&mut h, 4);
    assert!(one > 0);
    for (name, v) in [("2", two), ("4", four)] {
        let diff = (v as f64 - one as f64).abs() / one as f64;
        assert!(
            diff < 0.08,
            "1 フレーム {name} イベントでも同じ速さ: {v} と {one}（{diff:.3}）"
        );
    }
    // 速さは実際に効いている: 倍の速さで動かすと絵が変わる
    let at = |i: usize| offset(c, -60.0 + 2.0 * per_frame * i as f32, 0.0);
    press(&h, at(0), PointerButton::Primary);
    h.step();
    for frame in 1..=frames / 2 {
        move_to(&h, at(frame));
        h.step();
    }
    release(&h, at(frames / 2), PointerButton::Primary);
    h.step();
    h.run();
    let fast = ink(&h);
    assert!(fast != one, "{fast} != {one}");
}

#[test]
fn the_two_languages_name_the_panels() {
    let mut h = app(1280.0, 800.0, 128);
    assert!(h.query_by_label("新規レイヤー").is_some());
    assert!(h.query_by_label("レイヤーをグループ化").is_some());
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    assert!(h.query_by_label("New Layer").is_some());
    assert!(h.query_by_label("Group Layers").is_some());
    assert!(h.query_by_label("Add Layer Mask").is_some());
    assert!(h.query_by_label("新規レイヤー").is_none());
    click_tab(&mut h, Tab::Channels);
    h.run();
    assert!(h.query_by_label("Add Channel").is_some());
    assert!(h.query_by_label("Roughness").is_some());
}

// ───────── 保存（.ylp） ─────────

/// 保存と読み直しで変わってはいけない中身の全部（レイヤーごとの id・種類・親・名前・表示・不透明度・合成・クリッピング・
/// 有効なチャンネル・面の画素・塗りつぶしの値・調整・マスクの状態と画素・チャンネルごとの合成、文書のチャンネルの一覧）。
fn content(s: &AppState) -> Vec<String> {
    use std::hash::{Hash, Hasher};
    let doc = &s.doc;
    let pixels = |surface: &yolu_core::Surface| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for y in 0..doc.height() {
            for x in 0..doc.width() {
                surface.pixel(x, y).unwrap().hash(&mut hasher);
            }
        }
        hasher.finish()
    };
    let mut out = vec![format!("channels {:?}", doc.channels())];
    for c in doc.channels() {
        out.push(format!("channel {c:?} {:?}", doc.channel_info(c)));
    }
    out.push(format!("normal {:?}", doc.normal_settings()));
    for l in doc.layers() {
        let surfaces: Vec<_> = l
            .surface_channels()
            .into_iter()
            .map(|c| (c, pixels(l.surface(c).unwrap())))
            .collect();
        let mask = l
            .mask()
            .map(|m| (m.enabled(), m.inverted(), m.density(), pixels(m.surface())));
        out.push(format!(
            "{:?} {:?} {:?} {:?} visible={} opacity={} blend={:?} clip={} enabled={:?} surfaces={surfaces:?} fills={:?} adjustment={:?} mask={mask:?} blends={:?}",
            l.id(),
            l.kind(),
            l.parent(),
            l.name(),
            l.visible(),
            l.opacity(),
            l.blend_mode(),
            l.clipping(),
            l.enabled_channels(),
            l.fill_values().collect::<Vec<_>>(),
            l.adjustment(),
            l.channel_blends().collect::<Vec<_>>(),
        ));
    }
    out
}

/// 左下のタイル（0, 0）の、キャンバスの中の画素を単色にしてレイヤーのチャンネルへ入れる（透明の画素の RGB も保つ読み込み。キャンバスの外は 0）。
fn put_tile(s: &mut AppState, id: yolu_app::engine::LayerId, channel: Channel, rgba: [u8; 4]) {
    let ts = s.doc.tile_size() as usize;
    let (w, h) = (s.doc.width() as usize, s.doc.height() as usize);
    let mut bytes = vec![0u8; ts * ts * 4];
    for y in 0..h.min(ts) {
        for x in 0..w.min(ts) {
            bytes[(y * ts + x) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    s.doc
        .import_tile(id, channel, yolu_app::engine::TileCoord::new(0, 0), &bytes)
        .unwrap();
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-m2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 保存の往復で、今の yolu-io が書ける範囲（ラスターレイヤー・Color だけ）の中身がそのまま戻る。比較は `content` の全部。
#[test]
fn headless_saveable_documents_survive_save_and_reopen() {
    let dir = temp_dir("plain");
    let path = dir.join("plain.ylp");
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    put_tile(&mut s, base, Channel::Color, [200, 40, 30, 255]);
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    put_tile(&mut s, top, Channel::Color, [10, 20, 30, 0]);
    s.doc.set_layer_opacity(top, 0.5, false).unwrap();
    s.doc
        .set_layer_blend_mode(top, BlendMode::Multiply)
        .unwrap();
    s.doc.set_layer_clipping(top, true).unwrap();
    s.doc.set_layer_visible(base, false).unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert_eq!(content(&again), content(&s), "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

/// 今の yolu-io が書けない M2 の中身は、黙って捨てずに保存を断る: 断った理由が出る・文書も印も変わらない・ファイルを作らない。
#[test]
fn headless_each_m2_content_saves_and_reopens() {
    use yolu_app::engine::{ChannelInfo, ChannelKind, ColorSpace, Rgba8};
    let user_channel = || ChannelInfo {
        name: "AO2".into(),
        kind: ChannelKind::Scalar,
        color_space: ColorSpace::Linear,
        default: Rgba8::new(255, 255, 255, 255),
    };
    type Make = Box<dyn Fn(&mut AppState)>;
    let cases: Vec<(&str, Make)> = vec![
        (
            "マスクだけ",
            Box::new(|s| {
                let id = s.selected_layer.unwrap();
                s.apply(Action::M2(Edit::AddMask(id)));
            }),
        ),
        (
            "空のユーザーチャンネルだけ",
            Box::new(move |s| s.apply(Action::M2(Edit::AddChannel(user_channel())))),
        ),
        (
            "画素のあるユーザーチャンネル",
            Box::new(move |s| {
                s.apply(Action::M2(Edit::AddChannel(user_channel())));
                let channel = *s.doc.channels().last().unwrap();
                let id = s.selected_layer.unwrap();
                put_tile(s, id, channel, [90, 90, 90, 255]);
            }),
        ),
        (
            "Color のレイヤーごとの合成だけ",
            Box::new(|s| {
                let id = s.selected_layer.unwrap();
                s.apply(Action::M2(Edit::OwnBlend {
                    id,
                    channel: Channel::Color,
                    own: true,
                }));
            }),
        ),
        (
            "グループだけ",
            Box::new(|s| s.apply(Action::M2(Edit::GroupSelected))),
        ),
        (
            "塗りつぶしだけ",
            Box::new(|s| s.apply(Action::M2(Edit::NewFill))),
        ),
        (
            "調整だけ",
            Box::new(|s| {
                s.apply(Action::M2(Edit::NewAdjustment(
                    yolu_app::m2::AdjustmentKind::Levels,
                )))
            }),
        ),
        (
            "Normal の出力の設定だけ",
            Box::new(|s| {
                let settings = yolu_core::NormalSettings::DEFAULT.with_derive(true);
                s.doc.set_normal_settings(settings, false).unwrap();
            }),
        ),
    ];
    let dir = temp_dir("each");
    for (name, make) in &cases {
        let path = dir.join(format!("{name}.ylp"));
        let mut s = AppState::new(64, 64);
        make(&mut s);
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(
            s.message.starts_with("保存しました"),
            "{name}: {}",
            s.message
        );
        assert!(!s.modified, "{name}: 保存したら変更の印は下りる");
        let mut again = AppState::new(64, 64);
        again.apply(Action::OpenProject(path));
        assert_eq!(content(&again), content(&s), "{name}: {}", again.message);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// M2 の中身（マスク）を足して同じファイルへ上書き保存すると、前の版は -backups~ に残り、開き直すとマスクまで戻る。
#[test]
fn headless_overwriting_with_m2_content_keeps_the_previous_version() {
    let dir = temp_dir("overwrite");
    let path = dir.join("keep.ylp");
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    put_tile(&mut s, base, Channel::Color, [1, 2, 3, 255]);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let first = std::fs::read(&path).unwrap();
    s.apply(Action::M2(Edit::AddMask(base)));
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(!s.modified);
    assert_ne!(std::fs::read(&path).unwrap(), first);
    let backups = dir.join("keep.ylp-backups~");
    let kept: Vec<_> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| std::fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(kept.contains(&first), "前の版は -backups~ に残る");
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert_eq!(content(&again), content(&s), "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

/// M2 の中身（グループ・マスク・塗りつぶし・調整・ユーザーチャンネル・チャンネルごとの合成）を全部入れた文書を保存して開き直す。
/// 比較は `content` の全部。
#[test]
fn headless_m2_documents_survive_save_and_reopen() {
    use yolu_app::engine::{ChannelInfo, ChannelKind, ColorSpace, Rgba8};
    let dir = temp_dir("m2");
    let path = dir.join("m2.ylp");
    let mut s = AppState::new(64, 64);
    s.apply(Action::M2(Edit::GroupSelected));
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::AddMask(top)));
    s.apply(Action::M2(Edit::MaskDensity(top, 0.5)));
    s.apply(Action::M2(Edit::MaskInverted(top, true)));
    s.apply(Action::M2(Edit::NewFill));
    s.apply(Action::M2(Edit::NewAdjustment(
        yolu_app::m2::AdjustmentKind::Levels,
    )));
    s.apply(Action::M2(Edit::AddChannel(ChannelInfo {
        name: "AO2".into(),
        kind: ChannelKind::Scalar,
        color_space: ColorSpace::Linear,
        default: Rgba8::new(255, 255, 255, 255),
    })));
    s.apply(Action::M2(Edit::OwnBlend {
        id: top,
        channel: Channel::Roughness,
        own: true,
    }));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert_eq!(content(&again), content(&s), "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 3D ビュー ─────────

/// 3D のタブを出し、試しの立方体を読み、右（+X）と手前（−Z）の面が見えるカメラにする。
fn cube_view(doc: u32) -> (Harness<'static, YoluApp>, egui::Rect) {
    let mut h = app(1100.0, 760.0, doc);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

fn cube_stroke(h: &mut Harness<'_, YoluApp>, rect: egui::Rect) {
    use yolu_core::glam::Vec3;
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let at = |p: Vec3| {
        let s = view.to_screen(p).expect("カメラの前");
        pos2(rect.left() + s.x, rect.top() + s.y)
    };
    let (from, to) = (
        at(Vec3::new(0.15, 0.1, -0.5)),
        at(Vec3::new(0.45, 0.1, -0.5)),
    );
    let points: Vec<egui::Pos2> = (0..=8)
        .map(|i| from + (to - from) * (i as f32 / 8.0))
        .collect();
    drag(h, &points);
}

#[test]
fn the_cube_stroke_uses_the_paint_channel_the_base_brush_and_the_mask() {
    let (mut h, rect) = cube_view(256);
    h.state_mut().state.m2.random_seed = false;
    h.state_mut().state.color.set_main([0.9, 0.2, 0.1, 1.0]);
    let composite = |h: &Harness<'_, YoluApp>| {
        h.state()
            .state
            .doc
            .composite(h.state().state.doc.bounds())
            .unwrap()
    };
    // 描くチャンネル（Roughness）へ
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    cube_stroke(&mut h, rect);
    let id = h.state().state.selected_layer.unwrap();
    let layer = h.state().state.doc.layer(id).unwrap();
    assert!(layer.surface(Channel::Roughness).unwrap().tile_count() > 0);
    assert_eq!(layer.surface(Channel::Color).unwrap().tile_count(), 0);
    undo(&mut h);
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(Channel::Color)));
    cube_stroke(&mut h, rect);
    let plain = composite(&h);
    undo(&mut h);
    // プリセット（チョーク）は基本の値（直径・流量・不透明度・間隔・硬さ）を替える。基本の値は 3D でも効く
    let chalk = yolu_app::m2::presets()
        .iter()
        .position(|p| p.id == "chalk")
        .unwrap();
    apply(&mut h, Action::M2Ui(UiOp::Preset(chalk)));
    cube_stroke(&mut h, rect);
    let chalky = composite(&h);
    assert!(plain.chunks(4).any(|p| p[3] > 0) && chalky.chunks(4).any(|p| p[3] > 0));
    assert!(plain != chalky, "基本の値は 3D でも効く");
    undo(&mut h);
    // 筆先の画像と質感も 3D の面のダブに効く（外すと 3D の絵が変わる）
    {
        let brush = &mut h.state_mut().state.m2.brush;
        assert!(brush.tip.image.is_some() && brush.texture.is_some());
        brush.tip.image = None;
        brush.texture = None;
    }
    cube_stroke(&mut h, rect);
    assert!(composite(&h) != chalky, "3D でも筆先・質感が効く");
    undo(&mut h);
    // 色の変化は 3D でも効く
    apply(&mut h, Action::M2Ui(UiOp::Preset(0)));
    h.state_mut().state.m2.brush.color.hue = 0.5;
    cube_stroke(&mut h, rect);
    assert!(composite(&h) != plain, "色の変化は 3D でも効く");
    undo(&mut h);
    h.state_mut().state.m2.brush.color.hue = 0.0;
    // マスクを選んでいれば、面に描いたものはマスクへ
    apply(&mut h, Action::M2(Edit::AddMask(id)));
    cube_stroke(&mut h, rect);
    let layer = h.state().state.doc.layer(id).unwrap();
    assert!(layer.mask().unwrap().surface().tile_count() > 0);
    assert_eq!(layer.surface(Channel::Color).unwrap().tile_count(), 0);
    undo(&mut h);
    assert_eq!(
        h.state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .tile_count(),
        0
    );
}

#[test]
fn the_cube_runs_effect_brushes_asks_for_a_clone_source_and_the_eraser_still_works() {
    use yolu_app::engine::BrushEffect;
    use yolu_app::state::Tool;
    let (mut h, rect) = cube_view(256);
    h.state_mut().state.m2.random_seed = false;
    let ink = |h: &Harness<'_, YoluApp>| {
        h.state()
            .state
            .doc
            .composite(h.state().state.doc.bounds())
            .unwrap()
            .chunks(4)
            .filter(|p| p[3] > 0)
            .count()
    };
    cube_stroke(&mut h, rect);
    assert!(ink(&h) > 0);
    // ぼかしと指先は 3D の面でも走る（始める前に断らない。ストロークは終わっている）
    for (effect, name) in [(BrushEffect::BLUR, "ぼかし"), (BrushEffect::SMUDGE, "指先")] {
        h.state_mut().state.m2.brush.effect = effect;
        h.state_mut().state.message.clear();
        cube_stroke(&mut h, rect);
        let state = &h.state().state;
        assert!(
            !state.message.contains("効果のブラシ") && !state.is_stroking(),
            "{name}: 断らずに走る: {}",
            state.message
        );
    }
    // クローンは元（Alt を押したクリック）が要る。無ければ始めず、短い状態を出す
    h.state_mut().state.m2.brush.effect = BrushEffect::Clone {
        offset: yolu_app::engine::DVec2::new(8.0, 0.0),
    };
    h.state_mut().state.message.clear();
    let (steps, revision) = (
        h.state().state.doc.undo_count(),
        h.state().state.doc.revision(),
    );
    cube_stroke(&mut h, rect);
    let state = &h.state().state;
    assert_eq!(state.message, "クローンの元がありません");
    assert!(!state.is_stroking(), "ストロークを始めない");
    assert_eq!(
        (state.doc.undo_count(), state.doc.revision()),
        (steps, revision),
        "文書は変わらない"
    );
    // 英語でも短い状態
    apply(&mut h, Action::M2Ui(UiOp::Language(Lang::En)));
    h.state_mut().state.message.clear();
    cube_stroke(&mut h, rect);
    assert_eq!(h.state().state.message, "No clone source");
    // 消しゴムは効果を使わない（ペイントとして消す）ので、効果が選ばれたままでも 3D で消せる
    let painted = ink(&h);
    h.state_mut().state.tool = Tool::Eraser;
    h.state_mut().state.message.clear();
    cube_stroke(&mut h, rect);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    assert!(ink(&h) < painted, "消しゴムは消す: {} < {painted}", ink(&h));
}

#[test]
fn the_3d_notes_follow_the_tab_that_is_on_screen() {
    let (mut h, _) = cube_view(256);
    assert!(
        h.state().state.view3d.paintable_on_screen(),
        "モデルがあり 3D のタブが出ている"
    );
    click_tab(&mut h, Tab::Canvas);
    h.run();
    assert!(
        !h.state().state.view3d.paintable_on_screen(),
        "キャンバスの裏では出さない"
    );
    click_tab(&mut h, Tab::View3d);
    h.run();
    assert!(h.state().state.view3d.paintable_on_screen());
    let mut empty = app(1100.0, 760.0, 128);
    click_tab(&mut empty, Tab::View3d);
    empty.run();
    assert!(
        !empty.state().state.view3d.paintable_on_screen(),
        "モデルが無ければ描けないので出さない"
    );
}

/// 試しの人形（マテリアル 2 つ）を読んだウィンドウと 3D の表示域。
fn figure_window() -> (Harness<'static, YoluApp>, egui::Rect) {
    let mut h = app(1280.0, 860.0, 128);
    h.state_mut()
        .state
        .apply(Action::Pose(yolu_app::view3d::pose::PoseAction::LoadFigure));
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブが前に出た");
    (h, rect)
}

/// マテリアル material の三角形のうち、カメラにまっすぐ向いたものの真ん中の画面の点。
fn point_on_material(h: &Harness<'_, YoluApp>, rect: egui::Rect, material: i32) -> egui::Pos2 {
    let app = &h.state().state;
    let model = app.view3d.model.as_ref().unwrap();
    let cam = app.view3d.camera.position();
    let view = app.view3d.camera.view(rect.width(), rect.height());
    model
        .geometry
        .triangles()
        .iter()
        .filter(|t| t.material == material)
        .find_map(|t| {
            let c = (t.a + t.b + t.c) / 3.0;
            if t.normal().dot((cam - c).normalize()) > 0.8 {
                let s = view.to_screen(c)?;
                Some(pos2(rect.left() + s.x, rect.top() + s.y))
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("マテリアル {material} のカメラに向いた三角形が無い"))
}

/// FBX・試しの人形のモデルはマテリアルごとにテクスチャセットが付き、今のセットのマテリアルにだけ描け、セットを替えれば別のマテリアルに描ける
/// （前はいつも 1 つ目のマテリアルだけで、2 つ目以降の面は断られていた）。
#[test]
fn a_rig_model_paints_each_material_into_its_own_texture_set() {
    let (mut h, rect) = figure_window();
    h.state_mut().state.color.set_main([0.9, 0.1, 0.1, 1.0]);
    h.state_mut().state.brush.radius = 3.0;
    assert_eq!(h.state().state.sets.len(), 2, "{}", h.state().state.message);
    let current = h.state().state.sets.current_index();
    let a = h.state().state.sets.current().bound.unwrap() as i32;
    let b = 1 - a;
    let other = (0..2).find(|i| *i != current).unwrap();
    assert_eq!(
        h.state().state.sets.get(other).unwrap().bound,
        Some(b as u32)
    );
    let tiles = |h: &Harness<'_, YoluApp>, set: usize| {
        let app = &h.state().state;
        app.set_doc(set)
            .layers()
            .iter()
            .map(|l| l.surface(Channel::Color).map_or(0, |s| s.tile_count()))
            .sum::<usize>()
    };
    // 今のセットのマテリアルには描ける
    let at = point_on_material(&h, rect, a);
    click(&mut h, at);
    assert!(tiles(&h, current) > 0, "{}", h.state().state.message);
    assert_eq!(tiles(&h, other), 0);
    // ほかのセットのマテリアルの面は断る（文書は変わらない）
    let steps = h.state().state.doc.undo_count();
    let at = point_on_material(&h, rect, b);
    click(&mut h, at);
    assert_eq!(h.state().state.doc.undo_count(), steps);
    assert!(
        h.state().state.message.contains("ほかのテクスチャセット"),
        "{}",
        h.state().state.message
    );
    // セットを替えると、描く先のマテリアルも替わり、そちらの文書へ描く
    h.state_mut().state.switch_set(other).unwrap();
    h.run();
    assert_eq!(h.state().state.view3d.material, b);
    let at = point_on_material(&h, rect, b);
    click(&mut h, at);
    assert!(tiles(&h, other) > 0, "{}", h.state().state.message);
    let first_tiles = tiles(&h, current);
    assert!(first_tiles > 0, "最初のセットはそのまま");
    // 最初のセットの目を閉じると、そのマテリアルの面は 3D から消える
    let uid = h.state().state.sets.get(current).unwrap().uid;
    h.state_mut().state.toggle_set_visible(uid);
    h.run();
    let shown = h.state().state.view3d.model.clone().unwrap();
    assert!(shown.geometry.triangles().iter().all(|t| t.material == b));
}

#[test]
fn the_cube_refuses_a_layer_that_cannot_be_painted_and_says_why() {
    let (mut h, rect) = cube_view(256);
    apply(&mut h, Action::M2(Edit::NewGroup));
    let steps = h.state().state.doc.undo_count();
    cube_stroke(&mut h, rect);
    assert_eq!(h.state().state.doc.undo_count(), steps);
    assert!(
        h.state().state.message.contains("グループ"),
        "{}",
        h.state().state.message
    );
}

/// 開いている種類のメニューの 3 つの形式の行を上から並べる（文字・印）。欄の箱にも同じ文字があるので、メニューの中の行だけを数える。
fn kind_menu_rows(h: &Harness<'_, YoluApp>) -> Vec<(String, bool)> {
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect;
    let mut rows: Vec<(f32, String, bool)> = Vec::new();
    for label in ["sRGB8", "L8", "RGB8"] {
        for node in h.query_all_by_label(label) {
            let rect = node.rect();
            if body.contains_rect(rect) {
                let marked =
                    node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
                rows.push((rect.top(), label.to_owned(), marked));
            }
        }
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, l, m)| (l, m)).collect()
}

/// 開いている種類のメニューの中の、この形式の行の箱。
fn kind_menu_row(h: &Harness<'_, YoluApp>, label: &str) -> egui::Rect {
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect;
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| body.contains_rect(*r))
        .unwrap_or_else(|| panic!("メニューに {label} の行が無い"))
}

/// 種類のメニューの印は、欄が出す形式と同じ文字の項目に付く。ほかで作った文書にあるリニアのカラー（欄は RGB8）はノーマルの項目に付く。
/// 印の付いた項目を押しても、種類も色空間も Undo の段も変わらず、ほかの項目を選ぶと 1 回の Undo で戻る。絵は日英。
#[test]
fn snapshot_channel_kind_menu_marks_the_row_format() {
    use yolu_app::engine::{ChannelInfo, ChannelKind, ColorSpace, Rgba8};
    let mut h = app(1280.0, 800.0, 128);
    click_tab(&mut h, Tab::Channels);
    apply(
        &mut h,
        Action::M2(Edit::AddChannel(ChannelInfo {
            name: "Tint".into(),
            kind: ChannelKind::Color,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(255, 255, 255, 255),
        })),
    );
    let tint = h.state().state.m2.paint_channel;
    let before = h.state().state.doc.channel_info(tint).cloned();
    let steps = h.state().state.doc.undo_count();
    for (lang, name) in [
        (Lang::Ja, "channel_kind_menu_ja"),
        (Lang::En, "channel_kind_menu_en"),
    ] {
        apply(&mut h, Action::M2Ui(UiOp::Language(lang)));
        h.get_by_label("RGB8").click();
        h.run();
        assert_eq!(
            popup_kind(&h),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::ChannelKind(tint)))
        );
        assert_eq!(
            kind_menu_rows(&h),
            [
                ("sRGB8".to_owned(), false),
                ("L8".to_owned(), false),
                ("RGB8".to_owned(), true),
            ],
            "{name}"
        );
        h.snapshot(name);
        h.state_mut().state.popup = None;
        h.run();
    }
    // 印の付いた項目（RGB8）を押しても何も変わらない
    h.get_by_label("RGB8").click();
    h.run();
    let at = kind_menu_row(&h, "RGB8").center();
    click(&mut h, at);
    assert_eq!(h.state().state.doc.channel_info(tint).cloned(), before);
    assert_eq!(h.state().state.doc.undo_count(), steps);
    assert!(h.state().state.popup.is_none());
    // ほかの項目（sRGB8）を選ぶと、カラー（sRGB）になって 1 回の Undo で戻る
    h.get_by_label("RGB8").click();
    h.run();
    let at = kind_menu_row(&h, "sRGB8").center();
    click(&mut h, at);
    let after = h.state().state.doc.channel_info(tint).cloned().unwrap();
    assert_eq!(after.kind, ChannelKind::Color);
    assert_eq!(after.color_space, ColorSpace::Srgb);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    apply(&mut h, Action::Undo);
    assert_eq!(h.state().state.doc.channel_info(tint).cloned(), before);
}
