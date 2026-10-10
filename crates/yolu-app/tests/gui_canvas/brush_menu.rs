//! ブラシ・消しゴムの一覧の行の右クリックのメニュー: 先頭は「ブラシの設定…」（そのブラシの詳細のウィンドウ）、名前を変更（行の中でその場で）・複製・
//! グループへ移す ▸・削除。押せない項目はその理由をツールチップに。ブラシを引いている間は落とせるグループのタブが光る（egui_kittest）。
//! メニューの項目そのものの試験（画面を描かない）は `headless/brush_list.rs`。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::brushes::BrushKey;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::toolset::ui::Dragged;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

/// ブラシの一覧の行が全部入る高さのウィンドウ。
fn window() -> H {
    app(1600.0, 1400.0, 128)
}

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
    h.run();
}

/// 一覧の行（名前で探す）の、押せる所。
fn row_of(h: &H, name: &str) -> Rect {
    rect_of(h, name, |r| {
        r.left() < 340.0 && r.top() > 100.0 && r.top() < 900.0 && r.width() > 200.0
    })
}

fn right_click(h: &mut H, at: egui::Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
    release(h, at, PointerButton::Secondary);
    h.run();
}

fn open_menu(h: &mut H, name: &str) {
    let row = row_of(h, name);
    right_click(h, row.center());
    assert!(st(h).popup.is_some(), "{name}: メニューが開く");
}

fn group_tab(h: &H, name: &str) -> Rect {
    rect_of(h, name, |r| {
        r.left() < 340.0 && r.top() > 80.0 && r.top() < 130.0
    })
}

#[test]
fn the_row_menu_starts_with_the_brush_settings_and_opens_the_detail_of_that_brush() {
    for lang in Lang::ALL {
        let mut h = window();
        language(&mut h, lang);
        let (settings, rename, duplicate) = lang.pick(
            ("ブラシの設定…", "名前を変更", "複製"),
            ("Brush Settings…", "Rename", "Duplicate"),
        );
        let pencil = lang.pick("鉛筆", "Pencil");
        assert_eq!(st(&h).brushes.lib.current(), b("standard"));
        assert!(!st(&h).brushes.ui.detail.open);
        open_menu(&mut h, pencil);
        // 先頭に「ブラシの設定…」。名前の変更・複製はその下
        let first = popup_item(&h, settings);
        assert!(first.top() < popup_item(&h, rename).top(), "{lang:?}");
        assert!(popup_item(&h, rename).top() < popup_item(&h, duplicate).top());
        // 押すと、そのブラシに替わって詳細のウィンドウが開く（今のブラシではなかったブラシでも）
        click(&mut h, first.center());
        assert!(st(&h).popup.is_none(), "{lang:?}: 閉じる");
        assert_eq!(st(&h).brushes.lib.current(), b("pencil"), "{lang:?}");
        assert!(st(&h).brushes.ui.detail.open, "{lang:?}");
        let window = yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
            .expect("ブラシの詳細のウィンドウを描いている");
        assert!(window.width() > 300.0);
    }
}

#[test]
fn the_row_menu_renames_in_the_row_and_escape_leaves_the_name_alone() {
    let mut h = window();
    // 利用者のブラシを足して、メニューから名前を変える（行の中の入力欄。確定は Enter、Esc でやめる）
    h.get_by_label("今の設定を新しいブラシに").click();
    h.run();
    let copy = st(&h).brushes.lib.current();
    assert!(copy.is_user());
    let name = st(&h).brushes.lib.entry(copy).unwrap().name.clone();
    open_menu(&mut h, &name);
    let at = popup_item(&h, "名前を変更").center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes.ui.renaming, Some(copy), "行の中でその場で");
    h.event(Event::Text("線画".into()));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes.ui.renaming, None);
    assert_eq!(
        st(&h).brushes.lib.entry(copy).unwrap().name,
        name,
        "Esc では変えない"
    );
    // もう一度: 打って Enter で確定
    open_menu(&mut h, &name);
    let at = popup_item(&h, "名前を変更").center();
    click(&mut h, at);
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("線画".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes.lib.entry(copy).unwrap().name, "線画");
    row_of(&h, "線画");
}

#[test]
fn the_row_menu_moves_a_brush_to_another_group_from_a_nested_list() {
    for lang in Lang::ALL {
        let mut h = window();
        language(&mut h, lang);
        let pencil = lang.pick("鉛筆", "Pencil");
        let slot = st(&h).toolset.set.slot_of(b("pencil")).unwrap();
        let own = st(&h).toolset.set.group_of(b("pencil")).unwrap();
        let target = st(&h).toolset.set.slot(slot).unwrap().groups[1].id;
        let target_name = st(&h).toolset.set.group(target).unwrap().1.name_in(lang);
        open_menu(&mut h, pencil);
        h.get_by_label(lang.pick("グループへ移す", "Move to Group"))
            .click();
        h.run();
        // 今のグループは押せない（印だけ）。別のグループを選ぶと、そこへ動く
        let own_name = st(&h).toolset.set.group(own).unwrap().1.name_in(lang);
        let own_item = popup_item(&h, &own_name);
        click(&mut h, own_item.center());
        assert!(
            st(&h).popup.is_some(),
            "{lang:?}: 今のグループの項目では閉じない"
        );
        let item = popup_item(&h, &target_name);
        click(&mut h, item.center());
        assert!(st(&h).popup.is_none());
        assert_eq!(
            st(&h).toolset.set.group_of(b("pencil")),
            Some(target),
            "{lang:?}"
        );
        assert!(!st(&h).doc.can_undo(), "ブラシの並びは文書ではない");
    }
}

#[test]
fn a_blocked_row_menu_keeps_its_items_closed_and_names_the_reason() {
    let mut h = window();
    h.state_mut().state.toolset.set.locked = Some(yolu_app::toolset::Lock::Newer(9));
    h.run();
    open_menu(&mut h, "鉛筆");
    // 押せない項目は、押しても閉じない
    for label in ["複製", "グループへ移す", "削除"] {
        let item = popup_item(&h, label);
        click(&mut h, item.center());
        assert!(st(&h).popup.is_some(), "{label}: 押せない項目は閉じない");
    }
    assert!(!st(&h).doc.can_undo());
    // 理由は、乗せると出るツールチップの文（ラベルには続けない）
    let reason = yolu_app::toolset::Refusal::Newer(9).describe(Lang::Ja);
    let entries = yolu_app::toolset::ui::brush_menu(st(&h));
    use yolu_app::ui::menu::Entry;
    for entry in &entries {
        let (label, tooltip) = match entry {
            Entry::Item {
                label,
                enabled: false,
                tooltip,
                ..
            }
            | Entry::Submenu {
                label,
                enabled: false,
                tooltip,
                ..
            } => (label, tooltip),
            _ => continue,
        };
        if label == "元に戻す" {
            // 変更が無いから押せない（並びの理由ではない）
            assert_eq!(tooltip, &None);
            continue;
        }
        assert_eq!(tooltip.as_deref(), Some(reason.as_str()), "{label}");
        assert!(!label.contains(&reason), "ラベルに理由を続けない: {label}");
    }
}

#[test]
fn the_eraser_rows_have_the_same_menu() {
    let mut h = window();
    h.state_mut().state.apply(Action::SelectTool(Tool::Eraser));
    h.run();
    open_menu(&mut h, "ソフト消しゴム");
    let settings = popup_item(&h, "ブラシの設定…");
    for label in ["名前を変更", "複製", "グループへ移す", "削除"] {
        assert!(settings.top() < popup_item(&h, label).top(), "{label}");
    }
    click(&mut h, settings.center());
    assert_eq!(st(&h).brushes.lib.current(), b("soft-eraser"));
    assert!(st(&h).brushes.ui.detail.open);
}

#[test]
fn dragging_a_brush_lights_the_group_tabs_it_can_drop_on_and_stops_when_released() {
    let mut h = window();
    let row = row_of(&h, "鉛筆");
    let start = pos2(row.left() + 30.0, row.center().y);
    let slot = st(&h).toolset.set.slot_of(b("pencil")).unwrap();
    let own = st(&h).toolset.set.group_of(b("pencil")).unwrap();
    // 引く前は光らない
    assert!(yolu_app::toolset::ui::droppable_groups(st(&h), b("pencil")).len() > 1);
    press(&h, start, PointerButton::Primary);
    for dy in [3.0, 8.0, 14.0] {
        move_to(&h, pos2(start.x, start.y + dy));
        h.step();
    }
    assert_eq!(
        st(&h).toolset.ui.dragging(),
        Some(Dragged::Brush(b("pencil")))
    );
    let lit = yolu_app::toolset::ui::droppable_groups(st(&h), b("pencil"));
    assert!(!lit.contains(&own), "今のグループは落とす先ではない");
    for g in &st(&h).toolset.set.slot(slot).unwrap().groups {
        assert_eq!(lit.contains(&g.id), g.id != own);
    }
    // 光った所のタブの絵: 引いている間と、離した後の違い
    let tabs = {
        let tab = h.state().tab_rects[&yolu_app::Tab::SubTools];
        Rect::from_min_max(
            pos2(tab.left() - 2.0, tab.bottom()),
            pos2(tab.left() + 300.0, tab.bottom() + 32.0),
        )
    };
    h.step();
    let during = h.render().expect("描画");
    key(&h, Key::Escape, Modifiers::NONE);
    release(&h, pos2(start.x, start.y + 14.0), PointerButton::Primary);
    h.run();
    assert_eq!(st(&h).toolset.ui.dragging(), None);
    let after = h.render().expect("描画");
    let crop = |image: &image::RgbaImage| {
        image::imageops::crop_imm(
            image,
            tabs.left() as u32,
            tabs.top() as u32,
            tabs.width() as u32,
            tabs.height() as u32,
        )
        .to_image()
    };
    assert_ne!(
        crop(&during).as_raw(),
        crop(&after).as_raw(),
        "引いている間だけ、タブが光る"
    );
    egui_kittest::image_snapshot(&crop(&during), "brush_drag_lights_group_tabs");
    // 落とすと、そのグループへ動く（今までの動き）
    let target = st(&h).toolset.set.slot(slot).unwrap().groups[2].id;
    let tab_name = st(&h)
        .toolset
        .set
        .group(target)
        .unwrap()
        .1
        .name_in(Lang::Ja);
    let tab = group_tab(&h, &tab_name);
    drag(
        &mut h,
        &[
            start,
            pos2(start.x, start.y + 8.0),
            pos2(start.x, start.y + 14.0),
            tab.center(),
        ],
    );
    assert_eq!(st(&h).toolset.set.group_of(b("pencil")), Some(target));
}

#[test]
fn snapshots_of_the_row_menu_with_the_group_list_in_both_languages() {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = window();
        language(&mut h, lang);
        open_menu(&mut h, lang.pick("鉛筆", "Pencil"));
        h.get_by_label(lang.pick("グループへ移す", "Move to Group"))
            .click();
        h.run();
        let body = st(&h).popup.as_ref().unwrap().state.rect;
        let rect = Rect::from_min_max(
            pos2((body.left() - 30.0).max(0.0), (body.top() - 30.0).max(0.0)),
            pos2(body.right() + 260.0, body.bottom() + 30.0),
        );
        h.event(Event::PointerGone);
        h.step();
        let image = h.render().expect("描画");
        let cropped = image::imageops::crop_imm(
            &image,
            rect.left() as u32,
            rect.top() as u32,
            rect.width() as u32,
            rect.height() as u32,
        )
        .to_image();
        egui_kittest::image_snapshot(&cropped, format!("brush_row_menu_{name}"));
    }
}
