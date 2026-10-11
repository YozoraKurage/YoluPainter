//! ツールの並びの画面（左のツールの列の右クリック・ドラッグ、ブラシのグループのタブ、「＋」のウィンドウ）の操作と見た目（egui_kittest）。
//! 並びの操作そのもの（画面を描かない）は `headless/tool_layout.rs`。
use crate::common;

use common::*;
use egui::{pos2, Event, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{BrushAction, BrushKey};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::toolset::catalog::{CatalogItem, Kind};
use yolu_app::toolset::{SlotId, ToolsetAction};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(lang)));
    h.run();
}

fn tools(h: &mut H, action: ToolsetAction) {
    h.state_mut().state.apply(Action::Tools(action));
    h.run();
}

/// ツールの列のボタンの矩形（前のフレームに描いた所）。
fn button(h: &H, slot: SlotId) -> Rect {
    st(h)
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == slot)
        .map(|(_, r)| *r)
        .expect("ツールの列のボタン")
}

fn right_click(h: &mut H, at: egui::Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
    release(h, at, PointerButton::Secondary);
    h.run();
}

/// ウィンドウの全体を撮る（直前に押した所のポインタが絵に残らないように）。
fn shot(h: &mut H, name: &str) {
    h.event(Event::PointerGone);
    h.step();
    h.snapshot(name);
}

/// 試しの並び: インクのツール（ペンのアイコン・前に区切り・2 つのグループ）と、名前を変えたグループ・足したグループ。
fn customize(h: &mut H) {
    let s = &mut h.state_mut().state;
    let brush = s.toolset.set.first_of(Tool::Brush).unwrap();
    s.apply(Action::Tools(ToolsetAction::Add {
        tool: Tool::Brush,
        after: Some(s.toolset.set.first_of(Tool::Eraser).unwrap()),
    }));
    let ink = s.toolset.set.active().unwrap();
    s.apply(Action::Tools(ToolsetAction::Rename(
        ink,
        s.lang.pick("インク", "Ink").into(),
    )));
    s.apply(Action::Tools(ToolsetAction::SetIcon(ink, "stylus")));
    s.apply(Action::Tools(ToolsetAction::ToggleGap(ink)));
    let first = s.toolset.set.slot(ink).unwrap().groups[0].id;
    s.apply(Action::Tools(ToolsetAction::RenameGroup(
        first,
        s.lang.pick("線画", "Line Art").into(),
    )));
    s.apply(Action::Brush(BrushAction::AddFrom(vec![
        CatalogItem::Builtin("ink-pen"),
        CatalogItem::Builtin("pencil"),
        CatalogItem::Builtin("marker"),
    ])));
    s.apply(Action::Tools(ToolsetAction::AddGroup(ink)));
    s.toolset.ui.renaming = None;
    s.apply(Action::Tools(ToolsetAction::ShowGroup(first)));
    s.apply(Action::Tools(ToolsetAction::Select(ink)));
    // ブラシのツールにもグループを足す（タブが 2 段になる）
    for _ in 0..3 {
        s.apply(Action::Tools(ToolsetAction::AddGroup(brush)));
    }
    s.toolset.ui.renaming = None;
    h.run();
}

#[test]
fn the_strip_follows_the_layout_and_its_menu_adds_renames_and_removes_tools() {
    let mut h = app(1280.0, 800.0, 128);
    let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
    // ツールの列のボタンは、ツールの並びの数だけ。押すとツールが替わる
    assert_eq!(st(&h).toolset.ui.slot_rects.len(), Tool::ALL.len());
    let fill = st(&h).toolset.set.first_of(Tool::Fill).unwrap();
    let at = button(&h, fill).center();
    click(&mut h, at);
    assert_eq!(st(&h).tool, Tool::Fill);
    assert_eq!(st(&h).toolset.set.active(), Some(fill));
    // 右クリックのメニュー: ツールを追加 ▸ ブラシ
    let at = button(&h, brush).center();
    right_click(&mut h, at);
    h.get_by_label("ツールを追加").click();
    h.run();
    h.get_by_label("ブラシ").click();
    h.run();
    let added = st(&h).toolset.set.active().unwrap();
    assert_ne!(added, brush);
    assert_eq!(st(&h).toolset.set.slot_index(added), Some(1));
    assert_eq!(st(&h).toolset.ui.slot_rects.len(), Tool::ALL.len() + 1);
    // 名前を変更: 列の右に入力欄が出る
    let at = button(&h, added).center();
    right_click(&mut h, at);
    h.get_by_label("名前を変更").click();
    h.run();
    assert_eq!(
        st(&h).toolset.ui.renaming,
        Some(yolu_app::toolset::ui::Renaming::Slot(added))
    );
    h.event(Event::Text("水彩".into()));
    h.run();
    key(&h, egui::Key::Enter, Modifiers::NONE);
    h.run();
    let renamed = st(&h).toolset.set.slot(added).unwrap().name.clone();
    assert!(
        renamed.as_deref().is_some_and(|n| n.contains("水彩")),
        "{renamed:?}"
    );
    // このツールを削除
    let at = button(&h, added).center();
    right_click(&mut h, at);
    h.get_by_label("このツールを削除").click();
    h.run();
    assert!(st(&h).toolset.set.slot(added).is_none());
    // 最後のブラシのツールは削除できない（押せない）
    let at = button(&h, brush).center();
    right_click(&mut h, at);
    let item = h.get_by_label("このツールを削除");
    assert!(item.accesskit_node().is_disabled());
    key(&h, egui::Key::Escape, Modifiers::NONE);
    h.run();
}

#[test]
fn dragging_a_tool_button_moves_it_in_the_strip() {
    let mut h = app(1280.0, 800.0, 128);
    let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
    let fill = st(&h).toolset.set.first_of(Tool::Fill).unwrap();
    let from = button(&h, fill).center();
    let to = button(&h, brush);
    drag(
        &mut h,
        &[
            from,
            pos2(from.x, from.y - 8.0),
            pos2(to.center().x, to.top() + 3.0),
        ],
    );
    assert_eq!(st(&h).toolset.set.slot_index(fill), Some(0));
    assert!(!st(&h).doc.can_undo(), "並べ替えは文書ではない");
}

#[test]
fn the_plus_tab_adds_a_group_and_a_brush_dragged_onto_a_tab_moves_there() {
    // （一覧の行が全部入る高さで）
    let mut h = app(1280.0, 1100.0, 128);
    let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
    let before = st(&h).toolset.set.slot(brush).unwrap().groups.len();
    h.get_by_label("グループを追加").click();
    h.run();
    let groups = st(&h).toolset.set.slot(brush).unwrap().groups.clone();
    assert_eq!(groups.len(), before + 1);
    let added = groups.last().unwrap().id;
    // 足したグループの名前の入力欄が出る（Enter で決める）
    h.event(Event::Text("2".into()));
    h.run();
    key(&h, egui::Key::Enter, Modifiers::NONE);
    h.run();
    let renamed = st(&h)
        .toolset
        .set
        .group(added)
        .unwrap()
        .1
        .name
        .clone()
        .unwrap();
    assert!(renamed.contains('2'), "{renamed}");
    // ペンのタブへ戻り、インクペンの行を足したグループのタブへドラッグ
    tools(&mut h, ToolsetAction::ShowGroup(groups[0].id));
    let row = rect_of(&h, "インクペン", |r| {
        r.left() < 340.0 && r.width() > 200.0
    });
    let tab = rect_of(&h, &renamed, |r| r.left() < 340.0 && r.top() < 140.0);
    drag(
        &mut h,
        &[
            pos2(row.left() + 30.0, row.center().y),
            pos2(row.left() + 30.0, row.center().y - 8.0),
            tab.center(),
        ],
    );
    assert_eq!(st(&h).toolset.set.group_of(b("ink-pen")), Some(added));
}

#[test]
fn the_catalog_window_lists_kinds_searches_and_adds_the_selected_brushes() {
    let mut h = app(1280.0, 800.0, 128);
    h.get_by_label("ブラシを追加").click();
    h.run();
    assert!(st(&h).toolset.catalog.open);
    h.get_by_label("組み込み").click();
    h.run();
    for name in ["チョーク", "マーカー"] {
        let at = in_catalog(&h, name).center();
        click(&mut h, at);
    }
    assert_eq!(st(&h).toolset.catalog.selected.len(), 2);
    h.get_by_label("2 個を追加").click();
    h.run();
    let current = st(&h).brushes.lib.current();
    assert!(current.is_user(), "並びにある組み込みは写しになる");
    assert!(st(&h).toolset.catalog.selected.is_empty());
    // 自分のブラシに、写しが出る
    h.get_by_label("自分のブラシ").click();
    h.run();
    assert_eq!(st(&h).toolset.catalog.kind, Kind::Mine);
    let _ = in_catalog(&h, "チョーク");
}

/// 「＋」のウィンドウの中の、この名前の部品。
fn in_catalog(h: &H, label: &str) -> Rect {
    let window = yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_catalog::id())
        .expect("「＋」のウィンドウ");
    rect_of(h, label, |r| window.contains(r.center()))
}

#[test]
fn snapshot_tool_strip_menu_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [
        (Lang::Ja, "tool_layout_menu_ja"),
        (Lang::En, "tool_layout_menu_en"),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
        let at = button(&h, brush).center();
        right_click(&mut h, at);
        let add = rect_of(&h, lang.pick("ツールを追加", "Add Tool"), |_| true);
        move_to(&h, add.center());
        for _ in 0..10 {
            h.step();
        }
        h.snapshot(name);
        results.extend(h.take_snapshot_results());
    }
}

#[test]
fn snapshot_a_customized_layout_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [
        (Lang::Ja, "tool_layout_custom_ja"),
        (Lang::En, "tool_layout_custom_en"),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        customize(&mut h);
        shot(&mut h, name);
        results.extend(h.take_snapshot_results());
    }
}

#[test]
fn snapshot_the_group_tab_menu_and_rename_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [
        (Lang::Ja, "tool_layout_group_menu_ja"),
        (Lang::En, "tool_layout_group_menu_en"),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        let tab = rect_of(&h, lang.pick("筆", "Brush"), |r| {
            r.left() < 340.0 && r.top() < 140.0 && r.width() < 120.0
        });
        right_click(&mut h, tab.center());
        h.snapshot(name);
        results.extend(h.take_snapshot_results());
    }
}

#[test]
fn snapshot_the_icon_grid_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [
        (Lang::Ja, "tool_layout_icons_ja"),
        (Lang::En, "tool_layout_icons_en"),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        let brush = st(&h).toolset.set.first_of(Tool::Brush).unwrap();
        tools(&mut h, ToolsetAction::PickIcon(Some(brush)));
        shot(&mut h, name);
        results.extend(h.take_snapshot_results());
    }
}

#[test]
fn snapshot_the_catalog_window_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name, kind) in [
        (Lang::Ja, "tool_layout_catalog_ja", Kind::Builtin),
        (Lang::En, "tool_layout_catalog_en", Kind::Builtin),
        (Lang::Ja, "tool_layout_catalog_krita_ja", Kind::Krita),
        (Lang::En, "tool_layout_catalog_krita_en", Kind::Krita),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        {
            let s = &mut h.state_mut().state;
            s.toolset.catalog.open = true;
            s.toolset.catalog.kind = kind;
            match kind {
                Kind::Krita => {
                    let _ = yolu_app::brushes::store::krita();
                    s.toolset.catalog.selected = vec![CatalogItem::Krita(1), CatalogItem::Krita(3)];
                }
                _ => {
                    s.toolset.catalog.selected =
                        vec![CatalogItem::Builtin("pencil"), CatalogItem::Builtin("oil")];
                }
            }
        }
        // 見本のストロークは別のスレッドで描くので、出そろうまで回す
        for _ in 0..40 {
            h.run();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        shot(&mut h, name);
        results.extend(h.take_snapshot_results());
    }
}

#[test]
fn snapshot_wrapped_group_tabs_and_a_rename_field_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [
        (Lang::Ja, "tool_layout_tabs_ja"),
        (Lang::En, "tool_layout_tabs_en"),
    ] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        {
            let s = &mut h.state_mut().state;
            let brush = s.toolset.set.first_of(Tool::Brush).unwrap();
            s.apply(Action::Tools(ToolsetAction::AddGroup(brush)));
            let thick = *s
                .toolset
                .set
                .slot(brush)
                .unwrap()
                .groups
                .last()
                .map(|g| &g.id)
                .unwrap();
            s.apply(Action::Tools(ToolsetAction::RenameGroup(
                thick,
                lang.pick("厚塗りと質感", "Thick Paint").into(),
            )));
            s.apply(Action::Brush(BrushAction::AddFrom(vec![
                CatalogItem::Builtin("oil"),
                CatalogItem::Builtin("gouache"),
            ])));
            s.apply(Action::Tools(ToolsetAction::AddGroup(brush)));
        }
        h.run();
        h.run();
        shot(&mut h, name);
        results.extend(h.take_snapshot_results());
    }
}
