//! ブラシの画面（左のブラシのパネルと、ブラシの詳細のウィンドウ）の操作と見た目（egui_kittest）。一覧の操作そのものの試験（画面を描かない）は
//! `brush_list.rs`。見た目の試験は、パネルやウィンドウの中だけを撮る（ほかのパネルの変更で壊れない）。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{BrushAction, BrushKey, Category, Group, Row};
use yolu_app::engine::BrushEffect;
use yolu_app::lang::Lang;
use yolu_app::m2::{BrushOp, UiOp};
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    h.app()
}

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn language(h: &mut H, lang: Lang) {
    h.act(Action::M2Ui(UiOp::Language(lang)));
    h.run();
}

/// 左のドックの上の列（ブラシのパネルのある所）。
fn in_panel(r: Rect) -> bool {
    r.left() < 390.0 && r.top() > 80.0 && r.top() < 700.0
}

/// 一覧の行（名前で探す）の、押せる所。
fn row_of(h: &H, name: &str) -> Rect {
    rect_of(h, name, |r| {
        r.left() < 340.0 && r.top() > 100.0 && r.top() < 900.0 && r.width() > 200.0
    })
}

/// 一覧を先頭へ戻す（新しいブラシを作ると、一覧はその行へ送られる。組の高さに入りきらない行は、先頭へ戻さないと見えない）。
fn list_to_top(h: &mut H) {
    h.brushes_mut().ui.list_scroll = 0.0;
    h.run();
}

fn click_row(h: &mut H, name: &str) {
    let r = row_of(h, name);
    click(h, pos2(r.left() + 30.0, r.center().y));
}

fn group_tab(h: &H, name: &str) -> Rect {
    rect_of(h, name, |r| {
        r.left() < 340.0 && r.top() > 80.0 && r.top() < 120.0
    })
}

fn open_detail(h: &mut H, category: Category) {
    let ui = &mut h.brushes_mut().ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn detail_rect(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

/// ウィンドウの中の部品（ウィンドウと同じ名前の左のカテゴリと区別して、ウィンドウの右側の欄から探す）。
fn in_pane(h: &H, label: &str) -> Rect {
    let window = detail_rect(h);
    rect_of(h, label, |r| {
        r.left() > window.left() + 168.0 && window.contains(r.center())
    })
}

/// 詳細のウィンドウの欄を、その名前の部品が見える所まで送る（筆先の一覧はブラシ先端の区分の終わりにある）。
fn scroll_to(h: &mut H, label: &str) {
    let window = detail_rect(h);
    let view = Rect::from_min_max(
        pos2(window.left() + 168.0, window.top() + 140.0),
        window.max,
    );
    for _ in 0..40 {
        let r = h.get_by_label(label).rect();
        if view.contains(r.center()) {
            return;
        }
        let step = r.center().y - view.center().y;
        h.brushes_mut().ui.detail.scroll += step;
        h.run();
    }
}

/// 影響元設定のウィンドウ（開いていれば）と、その本体（見出しの帯の下）。
fn dynamics_window(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_rows::dynamics_id())
        .expect("影響元設定のウィンドウを描いている")
}

fn dynamics_body(h: &H) -> Rect {
    let w = dynamics_window(h);
    Rect::from_min_max(
        pos2(w.left(), w.top() + yolu_app::ui::window::HEADER_HEIGHT),
        w.max,
    )
}

/// 影響元設定のウィンドウの中の部品（同じ名前のものを上から並べる）。
fn in_dynamics(h: &H, label: &str) -> Vec<(Rect, bool)> {
    let w = dynamics_window(h);
    let mut found: Vec<(Rect, bool)> = h
        .query_all_by_label(label)
        .map(|n| (n.rect(), n.accesskit_node().is_disabled()))
        .filter(|(r, _)| w.contains(r.center()))
        .collect();
    found.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
    found
}

fn open_dynamics(h: &mut H, row: Row) {
    h.brushes_mut().ui.detail.dynamics = Some(row);
    h.run();
    h.run();
}

// ───────── パネルの場所 ─────────

#[test]
fn the_brush_panel_is_the_first_tab_of_the_left_dock_and_the_properties_lose_the_brush_tab() {
    let mut h = app(1600.0, 900.0, 128);
    let tab = h.state().tab_rects[&Tab::SubTools];
    assert!(tab.left() < 340.0 && tab.top() < 80.0);
    assert!(
        tab.left() < h.state().tab_rects[&Tab::Assets].left(),
        "アセットより前"
    );
    // ツールプロパティとブラシサイズは、サブツールの下に縦に並ぶ別のパネル
    let props_tab = h.state().tab_rects[&Tab::ToolProperties];
    let size_tab = h.state().tab_rects[&Tab::BrushSize];
    assert!(tab.top() < props_tab.top() && props_tab.top() < size_tab.top());
    // 右のプロパティのタブはステンシル・レイヤー（ブラシも、マテリアルも、筆先の形のアルファも無い）
    let properties = h.state().tab_rects[&Tab::Properties];
    let side = move |r: Rect| {
        r.left() > properties.left() - 2.0
            && r.top() > properties.bottom()
            && r.top() < properties.bottom() + 28.0
    };
    for label in ["ステンシル", "レイヤー"] {
        rect_of(&h, label, side);
    }
    assert!(
        h.query_all_by_label("ブラシ").all(|n| !side(n.rect())),
        "プロパティにブラシのタブは無い"
    );
    assert!(
        h.query_all_by_label("マテリアル").all(|n| !side(n.rect())),
        "プロパティにマテリアルのタブは無い"
    );
    assert!(
        h.query_all_by_label("アルファ").next().is_none(),
        "筆先の形は詳細のウィンドウの「形状」にあり、プロパティにアルファのタブは無い"
    );
    assert_eq!(st(&h).ui().property_tab, 1, "初めはレイヤーのタブ");
    // 英語
    language(&mut h, Lang::En);
    assert!(h.query_all_by_label("Alpha").next().is_none());
    assert!(h.query_by_label("Hardness").is_some());
    click_tab(&mut h, Tab::Assets);
    click_tab(&mut h, Tab::SubTools);
}

// ───────── 一覧 ─────────

#[test]
fn clicking_a_row_switches_the_brush_and_the_group_tabs_only_change_the_list() {
    let mut h = app(1600.0, 1200.0, 128);
    click_row(&mut h, "ハード円");
    assert_eq!(st(&h).brushes().lib.current(), b("hard-round"));
    assert_eq!(st(&h).brush().hardness, 0.95);
    // グループのタブを押しても、ブラシは替わらない（見ている一覧だけ）
    let tab = group_tab(&h, "筆").center();
    click(&mut h, tab);
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Brush));
    assert_eq!(st(&h).brushes().lib.current(), b("hard-round"));
    assert_eq!(st(&h).tool(), Tool::Brush);
    click_row(&mut h, "チョーク");
    assert_eq!(st(&h).brushes().lib.current(), b("chalk"));
    assert!(st(&h).m2().brush.texture.is_some());
    // ブラシのツールの一覧に、消しゴムのグループのタブも行も無い（消しゴムは消しゴムのツールの一覧）
    assert!(
        h.query_all_by_label("消しゴム")
            .all(|n| !(n.rect().left() < 340.0 && n.rect().top() > 80.0 && n.rect().top() < 120.0)),
        "グループのタブに消しゴムは無い"
    );
    assert!(h.query_all_by_label("ソフト消しゴム").next().is_none());
    // 消しゴムのツールに替えると、一覧は消しゴムだけになる（グループのタブも無い）。行を押してもツールは消しゴムのまま
    h.act(Action::SelectTool(Tool::Eraser));
    h.run();
    assert!(
        h.query_all_by_label("ハード円").next().is_none(),
        "消しゴムの一覧にブラシは無い"
    );
    assert!(
        h.query_all_by_label("筆").next().is_none(),
        "グループのタブは出ない"
    );
    click_row(&mut h, "ソフト消しゴム");
    assert_eq!(st(&h).tool(), Tool::Eraser);
    assert_eq!(st(&h).brush().radius, 24.0);
    // ブラシを替えても Undo の段は増えない（ブラシの設定は文書ではない）
    let steps = st(&h).doc().undo_count();
    click_row(&mut h, "ハード消しゴム");
    assert_eq!(st(&h).brushes().lib.current(), b("hard-eraser"));
    assert_eq!(st(&h).doc().undo_count(), steps);
    // 選んだ行が青く、変更ありの印は無い。キーボードでツールを替えると、ツールごとのブラシに戻る
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes().lib.current(), b("chalk"));
    assert_eq!(
        st(&h).shown_brush_group(),
        Some(Group::Brush),
        "一覧もそのブラシのグループへ"
    );
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes().lib.current(), b("hard-eraser"));
}

#[test]
fn the_eraser_tool_erases_with_the_eraser_groups_brush() {
    let mut h = app(1600.0, 900.0, 256);
    click_row(&mut h, "ハード円");
    h.brush_mut().radius = 30.0;
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -60.0, 0.0), offset(c, 60.0, 0.0)]);
    let off = offset(c, 0.0, 60.0);
    assert_eq!(canvas_pixel(&h, c)[3], 255);
    assert_eq!(canvas_pixel(&h, off)[3], 255);
    // E: 消しゴムのグループの、最後に使ったブラシ（標準の消しゴム）。それを細くして、中ほどを消す
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).tool(), Tool::Eraser);
    assert_eq!(st(&h).brushes().lib.current(), b("standard-eraser"));
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Eraser));
    assert_eq!(
        st(&h).brush().radius,
        16.0,
        "描くツールの大きさ（30）ではなく、消しゴムの設定"
    );
    h.brush_mut().radius = 4.0;
    let steps = st(&h).doc().undo_count();
    drag(&mut h, &[offset(c, -40.0, 0.0), offset(c, 40.0, 0.0)]);
    assert_eq!(st(&h).doc().undo_count(), steps + 1);
    assert!(canvas_pixel(&h, c)[3] < 255, "消しゴムで消えた");
    assert_eq!(canvas_pixel(&h, off)[3], 255, "細い消しゴムの外は残る");
    // 消しゴムの一覧のブラシを選び直す: ソフトな消しゴムは縁がなだらか
    click_row(&mut h, "ソフト消しゴム");
    assert_eq!(st(&h).tool(), Tool::Eraser);
    // B で描くツールへ戻ると、描くツールの最後のブラシ（変えた設定ごと）
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes().lib.current(), b("hard-round"));
    assert_eq!(st(&h).brush().radius, 30.0);
}

#[test]
fn the_list_footer_adds_duplicates_reverts_and_deletes() {
    let mut h = app(1600.0, 1200.0, 128);
    click_row(&mut h, "ハード円");
    h.brush_mut().radius = 30.0;
    h.run();
    // 変えると、行に変更ありの印（行のツールチップにも）。元に戻すが押せる
    let row = row_of(&h, "ハード円");
    assert!(st(&h).brush_is_modified(b("hard-round")));
    h.get_by_label_contains("元の設定に戻す").click();
    h.run();
    assert_eq!(st(&h).brush().radius, 12.0);
    assert!(!st(&h).brush_is_modified(b("hard-round")));
    let _ = row;
    // 追加
    h.get_by_label("今の設定を新しいブラシに").click();
    h.run();
    let key = st(&h).brushes().lib.current();
    assert!(key.is_user());
    assert_eq!(st(&h).message(), "ブラシを追加しました: ブラシ");
    // 新しい行が一覧に出て選ばれている。複製
    row_of(&h, "ブラシ");
    h.get_by_label("ブラシを複製").click();
    h.run();
    row_of(&h, "ブラシ のコピー");
    assert_ne!(st(&h).brushes().lib.current(), key);
    // 削除（今のブラシ。並びから外すだけで、ファイルは残る）
    let copy = st(&h).brushes().lib.current();
    h.get_by_label("ブラシを削除").click();
    h.run();
    assert!(!st(&h).toolset().set.contains(copy));
    assert!(st(&h).brushes().lib.entry(copy).is_some());
    assert_eq!(st(&h).brushes().lib.current(), key, "前の行へ");
    // 組み込みも並びから外せる（「＋」のウィンドウから戻せる）
    list_to_top(&mut h);
    click_row(&mut h, "ハード円");
    h.get_by_label("ブラシを削除").click();
    h.run();
    assert!(!st(&h).toolset().set.contains(b("hard-round")));
    assert!(st(&h).brushes().lib.entry(b("hard-round")).is_some());
}

#[test]
fn double_click_renames_a_user_brush_and_built_in_names_do_not_change() {
    let mut h = app(1600.0, 1200.0, 128);
    h.get_by_label("今の設定を新しいブラシに").click();
    h.run();
    let key = st(&h).brushes().lib.current();
    let row = row_of(&h, "ブラシ");
    let at = pos2(row.left() + 30.0, row.center().y);
    for _ in 0..2 {
        press(&h, at, PointerButton::Primary);
        release(&h, at, PointerButton::Primary);
        h.step();
    }
    h.run();
    assert_eq!(st(&h).brushes().ui.renaming, Some(key));
    self::key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("太い線".into()));
    self::key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes().lib.entry(key).unwrap().name, "太い線");
    assert_eq!(st(&h).brushes().ui.renaming, None);
    row_of(&h, "太い線");
    // Esc でやめる（名前は変わらない）
    let row = row_of(&h, "太い線");
    let at = pos2(row.left() + 30.0, row.center().y);
    for _ in 0..2 {
        press(&h, at, PointerButton::Primary);
        release(&h, at, PointerButton::Primary);
        h.step();
    }
    h.run();
    h.event(Event::Text("あ".into()));
    self::key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).brushes().lib.entry(key).unwrap().name, "太い線");
    assert_eq!(st(&h).brushes().ui.renaming, None);
    // 組み込みもダブルクリックで名前を変えられる（変えると、同じ場所のファイルの写しになる）。一覧は新しいブラシの所へ送られているので、先頭へ戻す
    h.brushes_mut().ui.list_scroll = 0.0;
    h.run();
    let row = row_of(&h, "標準");
    let at = pos2(row.left() + 30.0, row.center().y);
    for _ in 0..2 {
        press(&h, at, PointerButton::Primary);
        release(&h, at, PointerButton::Primary);
        h.step();
    }
    h.run();
    assert_eq!(st(&h).brushes().ui.renaming, Some(b("standard")));
    self::key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("細い線".into()));
    self::key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(!st(&h).toolset().set.contains(b("standard")));
    let copy = st(&h).brushes().lib.current();
    assert!(copy.is_user());
    assert_eq!(st(&h).brushes().lib.entry(copy).unwrap().name, "細い線");
    row_of(&h, "細い線");
}

#[test]
fn the_row_menu_renames_duplicates_registers_reverts_and_deletes() {
    let mut h = app(1600.0, 1200.0, 128);
    click_row(&mut h, "鉛筆");
    let row = row_of(&h, "鉛筆");
    press(&h, row.center(), PointerButton::Secondary);
    h.step();
    release(&h, row.center(), PointerButton::Secondary);
    h.run();
    assert!(st(&h).popup().is_some());
    // 変更が無ければ、登録と元に戻すは押せない（組み込みも、名前の変更・削除・複製はできる）
    for label in ["この設定で登録", "元に戻す"] {
        let item = popup_item(&h, label);
        click(&mut h, item.center());
        assert!(st(&h).popup().is_some(), "{label}: 押せない項目は閉じない");
        assert_eq!(st(&h).brushes().ui.renaming, None);
        assert!(st(&h).brushes().lib.entry(b("pencil")).is_some());
    }
    let at = popup_item(&h, "複製").center();
    click(&mut h, at);
    let copy = st(&h).brushes().lib.current();
    assert!(copy.is_user());
    assert_eq!(
        st(&h).brushes().lib.entry(copy).unwrap().name,
        "鉛筆 のコピー"
    );
    // 利用者のブラシ: 変えて、登録（新しい元になる）。名前の変更はメニューから
    h.brush_mut().radius = 55.0;
    h.run();
    let row = row_of(&h, "鉛筆 のコピー");
    press(&h, row.center(), PointerButton::Secondary);
    h.step();
    release(&h, row.center(), PointerButton::Secondary);
    h.run();
    let at = popup_item(&h, "この設定で登録").center();
    click(&mut h, at);
    assert!(!st(&h).brush_is_modified(copy));
    assert_eq!(
        st(&h)
            .brushes()
            .lib
            .entry(copy)
            .unwrap()
            .baseline
            .base
            .radius,
        55.0
    );
    let row = row_of(&h, "鉛筆 のコピー");
    press(&h, row.center(), PointerButton::Secondary);
    h.step();
    release(&h, row.center(), PointerButton::Secondary);
    h.run();
    let at = popup_item(&h, "名前を変更").center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.renaming, Some(copy));
    self::key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    // 削除
    let row = row_of(&h, "鉛筆 のコピー");
    press(&h, row.center(), PointerButton::Secondary);
    h.step();
    release(&h, row.center(), PointerButton::Secondary);
    h.run();
    let at = popup_item(&h, "削除").center();
    click(&mut h, at);
    assert!(!st(&h).toolset().set.contains(copy), "並びから外す");
    assert!(st(&h).brushes().lib.entry(copy).is_some(), "ファイルは残る");
}

#[test]
fn dragging_a_row_reorders_the_list_inside_its_group() {
    // （一覧の行が全部入る高さで）
    let mut h = app(1600.0, 1400.0, 128);
    // 利用者のブラシを 2 つ足して、並び替える
    for _ in 0..2 {
        h.get_by_label("今の設定を新しいブラシに").click();
        h.run();
    }
    let pen = |h: &H| -> Vec<BrushKey> {
        st(h)
            .brush_entries_in(Group::Pen)
            .iter()
            .map(|e| e.key)
            .collect()
    };
    let before = pen(&h);
    let (first, second) = (before[before.len() - 2], before[before.len() - 1]);
    // 2 つ目の行を、1 つ目の行の上へドラッグ
    let a = row_of(&h, "ブラシ");
    let b2 = row_of(&h, "ブラシ 2");
    drag(
        &mut h,
        &[
            pos2(b2.left() + 30.0, b2.center().y),
            pos2(b2.left() + 30.0, b2.center().y - 8.0),
            pos2(a.left() + 30.0, a.top() + 4.0),
        ],
    );
    let after = pen(&h);
    assert_eq!(after.len(), before.len());
    assert_eq!(after[after.len() - 2], second);
    assert_eq!(after[after.len() - 1], first);
    // 動かしても、今のブラシは替わらない。ドラッグは Undo の段を作らない
    assert_eq!(st(&h).toolset().ui.drag, None);
    assert!(!st(&h).doc().can_undo());
    // 一番下の空白へ落とすと、グループの一番後ろ
    list_to_top(&mut h);
    let top = row_of(&h, "標準");
    drag(
        &mut h,
        &[
            pos2(top.left() + 30.0, top.center().y),
            pos2(top.left() + 30.0, top.center().y + 20.0),
            pos2(top.left() + 30.0, top.center().y + 120.0),
        ],
    );
    let moved = pen(&h);
    assert_ne!(moved[0], before[0], "標準が動いた");
}

// ───────── ツールプロパティとブラシサイズ ─────────

#[test]
fn the_tool_properties_change_the_live_brush_and_the_dynamics_buttons_open_the_window() {
    let mut h = app(1600.0, 900.0, 128);
    let slider = |h: &H, label: &str| {
        rect_of(h, label, |r| {
            r.left() < 340.0 && r.top() > 200.0 && r.height() < 30.0
        })
    };
    // 直径（1 行の形のスライダー）: 左の端を押すと最小、右の端で最大。右の端は影響元のボタンの左
    let size = slider(&h, "直径");
    click(&mut h, pos2(size.left() + 1.0, size.center().y));
    assert!(st(&h).brush().radius < 1.5, "{}", st(&h).brush().radius);
    click(&mut h, pos2(size.right() - 1.0, size.center().y));
    assert!(st(&h).brush().radius >= 127.0);
    let hardness = slider(&h, "硬さ");
    click(&mut h, pos2(hardness.left() + 1.0, hardness.center().y));
    assert!(st(&h).brush().hardness < 0.02);
    // 手ぶれ補正は描き手の設定（一覧の変更ありの印にならない）
    let stab = slider(&h, "手ぶれ補正");
    let before = st(&h).brush_is_modified(st(&h).brushes().lib.current());
    click(
        &mut h,
        pos2(stab.left() + stab.width() * 0.5, stab.center().y),
    );
    assert!((90.0..110.0).contains(&(st(&h).m2().brush.assist.stabilizer as f32)));
    assert_eq!(
        st(&h).brush_is_modified(st(&h).brushes().lib.current()),
        before
    );
    // 影響元のボタン（直径・不透明度・流量・硬さの行の右）。押すと設定のウィンドウが開き、もう一度押すと閉じる
    let buttons: Vec<Rect> = h
        .query_all_by_label("影響元")
        .map(|n| n.rect())
        .filter(|r| in_panel(*r))
        .collect();
    assert_eq!(buttons.len(), 4, "{buttons:?}");
    for (button, row) in buttons
        .iter()
        .zip([Row::Size, Row::Opacity, Row::Flow, Row::Hardness])
    {
        click(&mut h, button.center());
        assert_eq!(st(&h).brushes().ui.detail.dynamics, Some(row), "{row:?}");
        assert!(yolu_app::ui::window::last_rect(
            &h.ctx,
            yolu_app::panels::brush_rows::dynamics_id()
        )
        .is_some());
    }
    click(&mut h, buttons[3].center());
    assert_eq!(st(&h).brushes().ui.detail.dynamics, None);
    // 流量・間隔・不透明度のスライダーも同じ設定を動かす
    let flow = slider(&h, "流量");
    click(
        &mut h,
        pos2(flow.left() + flow.width() * 0.5, flow.center().y),
    );
    assert!((0.4..0.6).contains(&st(&h).brush().flow));
    // 値の箱ではなく、行の中をダブルクリックして数を打てる（直前の押下から時間を空ける。直前の別の所の押下と 3 回押しにならないように）
    for _ in 0..40 {
        h.step();
    }
    let spacing = slider(&h, "間隔");
    for _ in 0..2 {
        press(&h, spacing.center(), PointerButton::Primary);
        release(&h, spacing.center(), PointerButton::Primary);
        h.step();
    }
    h.run();
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("40".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(
        (st(&h).brush().spacing - 0.4).abs() < 1e-6,
        "{}",
        st(&h).brush().spacing
    );
}

#[test]
fn the_size_circles_set_the_diameter_and_mark_the_nearest_one() {
    // （丸が 4 段とも入る高さで）
    let mut h = app(1600.0, 1400.0, 128);
    // 既定の直径 32 に近い 30 の丸が今の大きさ
    let cell = |h: &H, size: f32| {
        rect_of(h, &yolu_app::panels::brushes::size_name(size), |r| {
            r.left() < 400.0 && r.top() > 100.0 && r.width() < 60.0
        })
    };
    for size in [1.0f32, 3.0, 8.0, 25.0, 60.0, 120.0, 250.0] {
        let at = cell(&h, size).center();
        click(&mut h, at);
        assert_eq!(st(&h).brush().radius, size / 2.0, "{size}");
        let near = yolu_app::panels::brushes::SIZES
            [yolu_app::panels::brushes::nearest_size(st(&h).brush().radius * 2.0)];
        assert_eq!(near, size);
    }
    // 間の大きさは、近い丸に印（比で近いほう）
    h.brush_mut().radius = 18.0; // 直径 36
    h.run();
    assert_eq!(
        yolu_app::panels::brushes::SIZES[yolu_app::panels::brushes::nearest_size(36.0)],
        40.0
    );
    // 大きさを持たないツールでは、パネルは空（丸は出ない）
    h.act(Action::SelectTool(Tool::Fill));
    h.run();
    assert!(h.query_by_label("30 px").is_none());
    // 英語（丸の名前は同じ）
    h.act(Action::SelectTool(Tool::Brush));
    language(&mut h, Lang::En);
    assert!(h.query_by_label("30 px").is_some());
}

// ───────── ブラシの詳細のウィンドウ ─────────

#[test]
fn the_wrench_opens_the_detail_window_and_it_lists_every_category() {
    let mut h = app(1600.0, 900.0, 128);
    assert!(!st(&h).brushes().ui.detail.open);
    h.get_by_label("ブラシの詳細").click();
    h.run();
    assert!(st(&h).brushes().ui.detail.open);
    let window = detail_rect(&h);
    // ウィンドウはキャンバスの真ん中を空けた所に出る
    assert!(window.left() > 340.0 && window.top() > 100.0, "{window:?}");
    // 左のカテゴリは今のブラシの欄の全部（対称は定規の欄にあり、ここには無い）
    for category in Category::ALL {
        let name = category.name(Lang::Ja);
        assert!(
            h.query_all_by_label(name)
                .any(|n| window.contains(n.rect().center())),
            "{name}"
        );
    }
    // もう一度押すと閉じる。ウィンドウの閉じるボタンでも閉じる
    h.get_by_label("ブラシの詳細").click();
    h.run();
    assert!(!st(&h).brushes().ui.detail.open);
    h.get_by_label("ブラシの詳細").click();
    h.run();
    h.get_by_label("閉じる").click();
    h.run();
    assert!(!st(&h).brushes().ui.detail.open);
}

/// 影響元設定のウィンドウの曲線の枠（共通の編集の部品）: 入力ごとの曲線を点で直し、ドラッグは離したとき 1 回で入り（Esc でやめられる）、切っている入力では
/// 触れない。ほかの項目の応えは変わらず、区分の「既定に戻す」で直線へ戻る。
#[test]
fn the_curve_in_the_dynamics_window_is_edited_per_input_and_cancels_with_escape() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Size);
    open_dynamics(&mut h, Row::Size);
    let window = dynamics_window(&h);
    // 筆圧の曲線の枠（ウィンドウの下の左）
    let frame = yolu_app::panels::brush_rows::dynamics_curve_frame(dynamics_body(&h), 0);
    let g = frame.shrink(6.0);
    let at = |x: f32, y: f32| pos2(g.left() + x * g.width(), g.bottom() - y * g.height());
    let primary = PointerButton::Primary;
    let size = |h: &H| st(h).m2().brush.influences.size.pressure.response.clone();
    assert!(size(&h).is_identity());
    assert!(window.contains_rect(frame), "{window:?} {frame:?}");

    // 押して動かす間は下書き（ブラシは変わらない）。離すと (0.5, 0.8) の点が 1 回で入る
    move_to(&h, at(0.5, 0.5));
    h.step();
    press(&h, at(0.5, 0.5), primary);
    h.step();
    move_to(&h, at(0.5, 0.8));
    h.step();
    assert!(size(&h).is_identity(), "ドラッグの間は書かない");
    assert!(!st(&h).brush_is_modified(b("standard")));
    release(&h, at(0.5, 0.8), primary);
    h.step();
    h.run();
    let curve = size(&h).curve_shape();
    assert_eq!(curve.points().len(), 3);
    let p = curve.points()[1];
    assert!(
        (p.x - 0.5).abs() < 0.02 && (p.y - 0.8).abs() < 0.02,
        "{p:?}"
    );
    assert!((size(&h).apply(0.5) - 0.8).abs() < 0.03);
    assert!(st(&h).brush_is_modified(b("standard")), "変えたら変更あり");
    // ほかの項目・ほかの入力は変わらない
    for item in [
        yolu_core::brush::Item::Opacity,
        yolu_core::brush::Item::Flow,
        yolu_core::brush::Item::Hardness,
    ] {
        assert!(
            st(&h)
                .m2()
                .brush
                .influences
                .get(item)
                .pressure
                .response
                .is_identity(),
            "{item:?}"
        );
    }
    assert!(st(&h)
        .m2()
        .brush
        .influences
        .size
        .tilt
        .response
        .is_identity());
    // 最小値は曲線の後に効く: 曲線を保ち、最小だけ変わる（筆圧の「最小値」は上から 1 つ目）
    let minimum = in_dynamics(&h, "最小値")[0].0;
    let min_at = pos2(minimum.left() + minimum.width() * 0.4, minimum.center().y);
    click(&mut h, min_at);
    let both = size(&h);
    assert!((0.3..0.5).contains(&both.min()), "{}", both.min());
    assert_eq!(both.curve_shape(), curve, "最小値を変えても曲線はそのまま");

    // Esc でやめると、押す前のまま
    let before = size(&h);
    let mid = curve.points()[1];
    let mid_at = at(mid.x as f32, mid.y as f32);
    move_to(&h, mid_at);
    h.step();
    press(&h, mid_at, primary);
    h.step();
    move_to(&h, at(0.3, 0.1));
    h.step();
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at(0.3, 0.1), primary);
    h.step();
    h.run();
    assert_eq!(size(&h), before);

    // 右クリックで点を消すと直線へ（最小値は残る）
    move_to(&h, mid_at);
    h.step();
    press(&h, mid_at, PointerButton::Secondary);
    h.step();
    release(&h, mid_at, PointerButton::Secondary);
    h.step();
    h.run();
    assert!(size(&h).curve().is_empty());
    assert_eq!(size(&h).min(), before.min());

    // 筆圧を使わないときは、曲線は触れない（切り替えを切ると使えなくなる）
    let toggle = in_dynamics(&h, "筆圧")[0].0;
    click(&mut h, toggle.center());
    assert!(!st(&h).brush().pressure.size);
    let off = size(&h);
    move_to(&h, at(0.5, 0.5));
    h.step();
    press(&h, at(0.5, 0.5), primary);
    h.step();
    move_to(&h, at(0.5, 0.9));
    h.step();
    release(&h, at(0.5, 0.9), primary);
    h.step();
    h.run();
    assert_eq!(size(&h), off);
    assert!(
        in_dynamics(&h, "最小値")[0].1,
        "切った入力の最小値は押せない"
    );
    click(&mut h, toggle.center());
    assert!(st(&h).brush().pressure.size);

    // 項目ごと: 不透明度の影響元の最小と曲線を直しても、直径は今のまま。不透明度の側だけが変わる
    let size_now = size(&h);
    open_dynamics(&mut h, Row::Opacity);
    let opacity_min = in_dynamics(&h, "最小値")[0].0;
    click(
        &mut h,
        pos2(
            opacity_min.left() + opacity_min.width() * 0.6,
            opacity_min.center().y,
        ),
    );
    let opacity = |h: &H| {
        st(h)
            .m2()
            .brush
            .influences
            .opacity
            .pressure
            .response
            .clone()
    };
    assert!(
        (0.5..0.7).contains(&opacity(&h).min()),
        "{}",
        opacity(&h).min()
    );
    let frame = yolu_app::panels::brush_rows::dynamics_curve_frame(dynamics_body(&h), 0);
    let og = frame.shrink(6.0);
    let spot = pos2(
        og.left() + 0.5 * og.width(),
        og.bottom() - 0.8 * og.height(),
    );
    click(&mut h, spot);
    let bent = opacity(&h);
    assert_eq!(bent.curve().len(), 3, "{bent:?}");
    assert!((bent.curve_shape().points()[1].y - 0.8).abs() < 0.02);
    assert_eq!(size(&h), size_now, "直径の応えは変わらない");
    // 不透明度の点を右クリックで消すと直線へ戻る（直径は触れない）
    move_to(&h, spot);
    h.step();
    press(&h, spot, PointerButton::Secondary);
    h.step();
    release(&h, spot, PointerButton::Secondary);
    h.step();
    h.run();
    assert!(opacity(&h).curve().is_empty());
    assert_eq!(size(&h), size_now);

    // 直径の曲線を足してから、区分の「既定に戻す」: 直線・最小 0・筆圧を使う（既定）へ。ウィンドウで直した不透明度はブラシサイズの区分の外
    open_dynamics(&mut h, Row::Size);
    let frame = yolu_app::panels::brush_rows::dynamics_curve_frame(dynamics_body(&h), 0);
    let g = frame.shrink(6.0);
    click(
        &mut h,
        pos2(g.left() + 0.4 * g.width(), g.bottom() - 0.2 * g.height()),
    );
    assert_eq!(size(&h).curve().len(), 3);
    h.get_by_label("ブラシサイズを既定に戻す").click();
    h.run();
    assert!(size(&h).is_identity());
    assert!(st(&h).brush().pressure.size);
    assert!(!opacity(&h).is_identity(), "ほかの区分の設定は残る");
}

/// 傾きの最大値（最小値と最大値の間に写る）と、傾き・速度・ランダムの入力の切り替え（色相のランダムは、はじめて入れるとき ±値にする）。
#[test]
fn the_dynamics_window_sets_the_tilt_maximum_and_the_inputs_of_a_color_item() {
    let mut h = app(1600.0, 1000.0, 128);
    open_dynamics(&mut h, Row::Size);
    let tilt = |h: &H| st(h).m2().brush.influences.size.tilt.clone();
    // 傾きを入れる → 最小値と最大値（上から 2 つ目の最小値の下の最大値）
    let tilt_toggle = in_dynamics(&h, "傾き")[0].0;
    click(&mut h, tilt_toggle.center());
    assert!(tilt(&h).on);
    let max = in_dynamics(&h, "最大値")[0].0;
    assert!(
        !in_dynamics(&h, "最大値")[0].1,
        "傾きを使うと最大値が押せる"
    );
    click(&mut h, pos2(max.left() + max.width() * 0.5, max.center().y));
    let response = tilt(&h).response;
    assert!((0.4..0.6).contains(&response.max()), "{}", response.max());
    assert_eq!(response.min(), 0.0, "最小値はそのまま");
    assert!(st(&h).brush_is_modified(b("standard")));
    // 最大値は傾きだけにある（1 つだけ）
    assert_eq!(in_dynamics(&h, "最大値").len(), 1);
    // 傾きを切ると最大値は押せない
    click(&mut h, tilt_toggle.center());
    assert!(!tilt(&h).on && in_dynamics(&h, "最大値")[0].1);
    assert_eq!(tilt(&h).response, response, "切っても最大値は残る");
    // 色相: ランダムをはじめて入れると最小値 −100%（±値）になる。もう一度切って入れても、決めた最小値のまま
    open_dynamics(&mut h, Row::ColorHue);
    let random = in_dynamics(&h, "ランダム")[0].0;
    click(&mut h, random.center());
    let hue = |h: &H| st(h).m2().brush.influences.color_hue.random.clone();
    assert!(hue(&h).on);
    assert_eq!(hue(&h).response.min(), -1.0);
    click(&mut h, random.center());
    click(&mut h, random.center());
    assert!(hue(&h).on && hue(&h).response.min() == -1.0);
    // 速度も同じ形で入る（最小値は 0 のまま）
    let speed = in_dynamics(&h, "速度")[0].0;
    click(&mut h, speed.center());
    assert!(st(&h).m2().brush.influences.color_hue.speed.on);
    assert_eq!(
        st(&h).m2().brush.influences.color_hue.speed.response.min(),
        0.0
    );
}

#[test]
fn the_detail_window_edits_the_live_brush_by_category_and_resets_each_one() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Size);
    // ブラシサイズ: 大きさのゆらぎのスライダーを 60% あたりまで
    let slider = in_pane(&h, "大きさのゆらぎ");
    drag(
        &mut h,
        &[
            pos2(slider.left() + 2.0, slider.center().y),
            pos2(slider.left() + slider.width() * 0.6, slider.center().y),
        ],
    );
    let size = st(&h).m2().brush.jitter.size;
    assert!((0.5..0.7).contains(&size), "{size}");
    assert!(st(&h).brush_is_modified(b("standard")), "変えたら変更あり");
    // 見出しの右の既定に戻す
    h.get_by_label("ブラシサイズを既定に戻す").click();
    h.run();
    assert_eq!(st(&h).m2().brush.jitter.size, 0.0);
    assert!(!st(&h).brush_is_modified(b("standard")));
    // 最低 1 ピクセルと、画面上のサイズで指定（ブラシサイズの欄のチェック）。入れると変更ありになり、既定に戻すと切に戻る
    for (label, get) in [
        (
            "最低 1 ピクセル",
            (|s: &AppState| s.m2().brush.controls.min_one_pixel) as fn(&AppState) -> bool,
        ),
        ("画面上のサイズで指定", |s| {
            s.m2().brush.controls.size_by_screen
        }),
    ] {
        assert!(!get(st(&h)), "{label}");
        h.get_by_label(label).click();
        h.run();
        assert!(get(st(&h)), "{label}");
        assert!(st(&h).brush_is_modified(b("standard")), "変えたら変更あり");
        h.get_by_label("ブラシサイズを既定に戻す").click();
        h.run();
        assert!(!get(st(&h)), "{label}");
        // パスのツールの間は押せない（パスは `Controls` を持たない）。ほかのツールでは押せる
        assert!(!h.get_by_label(label).accesskit_node().is_disabled());
        h.act(Action::SelectTool(Tool::Path));
        h.run();
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
        h.act(Action::SelectTool(Tool::Brush));
        h.run();
        assert!(!h.get_by_label(label).accesskit_node().is_disabled());
    }
    // 区分を替える（左の一覧を押す）
    let window = detail_rect(&h);
    let at = rect_of(&h, "補正", |r| window.contains(r.center())).center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.detail.category, Category::Stabilization);
    // 補正: 手ぶれ補正と曲線
    let stab = in_pane(&h, "手ぶれ補正");
    click(
        &mut h,
        pos2(stab.left() + stab.width() * 0.5, stab.center().y),
    );
    assert!(st(&h).m2().brush.assist.stabilizer > 50.0);
    h.get_by_label("曲線").click();
    h.run();
    assert!(st(&h).m2().brush.assist.curve);
    h.get_by_label("補正を既定に戻す").click();
    h.run();
    assert_eq!(st(&h).m2().brush.assist.stabilizer, 0.0);
    assert!(!st(&h).m2().brush.assist.curve);
    // 入り抜き: 入り・抜き・影響先
    let at = rect_of(&h, "入り抜き", |r| {
        window.contains(r.center()) && r.left() < window.left() + 168.0
    })
    .center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.detail.category, Category::Taper);
    let taper = in_pane(&h, "入りの長さ");
    click(
        &mut h,
        pos2(taper.left() + taper.width() * 0.4, taper.center().y),
    );
    assert!(st(&h).m2().brush.assist.taper_in > 100.0);
    // 影響先: 不透明度にも掛ける（チェックを入れると最小値 0 で入る）
    assert!(st(&h)
        .m2()
        .brush
        .assist
        .taper_targets
        .min(yolu_core::brush::Item::Opacity)
        .is_none());
    let target = in_pane(&h, "不透明度");
    click(&mut h, pos2(target.left() + 10.0, target.center().y));
    assert_eq!(
        st(&h)
            .m2()
            .brush
            .assist
            .taper_targets
            .min(yolu_core::brush::Item::Opacity),
        Some(0.0)
    );
    // 筆圧で硬さを変える切り替えは入り抜きの外のもの: 入り抜きを既定に戻しても残る
    h.m2_mut().brush.influences.hardness.pressure.on = true;
    h.get_by_label("入り抜きを既定に戻す").click();
    h.run();
    assert_eq!(st(&h).m2().brush.assist.taper_in, 0.0);
    assert!(st(&h).m2().brush.assist.taper_targets.is_default());
    assert!(st(&h).m2().brush.influences.hardness.pressure.on);
    h.m2_mut().brush.influences.hardness.pressure.on = false;
    // 2-ブラシ形状: 使う → 2-直径などが使える → 既定に戻すで外れる
    let at = rect_of(&h, "2-ブラシ形状", |r| {
        window.contains(r.center()) && r.left() < window.left() + 168.0
    })
    .center();
    click(&mut h, at);
    assert_eq!(
        st(&h).brushes().ui.detail.category,
        Category::DualTip,
        "見出しは最初の子へ移る"
    );
    assert!(
        h.get_by_label("2-直径").accesskit_node().is_disabled(),
        "使う前は薄い"
    );
    h.get_by_label("デュアルブラシを使う").click();
    h.run();
    assert!(st(&h).m2().brush.dual.is_some());
    assert!(!h.get_by_label("2-直径").accesskit_node().is_disabled());
    in_pane(&h, "2-モード: 乗算");
    h.get_by_label("2-ブラシ先端を既定に戻す").click();
    h.run();
    assert!(st(&h).m2().brush.dual.is_none());
    // どの設定も Undo の段を作らない
    assert!(!st(&h).doc().can_undo());
}

#[test]
fn the_detail_window_picks_tips_textures_and_effects_from_its_menus() {
    let mut h = app(1600.0, 1000.0, 128);
    // ブラシ先端: 筆先の画像を選ぶ・丸へ戻す
    open_detail(&mut h, Category::Tip);
    let window = detail_rect(&h);
    // 筆先の一覧は欄の下にあるので、見える所まで送ってから押す
    scroll_to(&mut h, "ドット");
    let dots = rect_of(&h, "ドット", |r| window.contains(r.center()));
    click(&mut h, dots.center());
    assert_eq!(
        st(&h).m2().brush.tip.image.as_ref().map(|t| t.name()),
        Some("dots")
    );
    let round = rect_of(&h, "丸（硬さ）", |r| {
        window.contains(r.center()) && r.width() < 60.0
    });
    click(&mut h, round.center());
    assert!(st(&h).m2().brush.tip.image.is_none());
    // 角度と線の向き（欄の上へ戻す）
    h.brushes_mut().ui.detail.scroll = 0.0;
    h.run();
    let angle = in_pane(&h, "角度");
    click(&mut h, pos2(angle.right() - 1.0, angle.center().y));
    assert!(st(&h).m2().brush.tip.angle > 170.0);
    h.get_by_label("線の向きに従う").click();
    h.run();
    assert!(st(&h).m2().brush.tip.follow_direction);
    // 繰り返し方法・左右反転（自前のメニュー）: 画像の筆先のときだけ反転は選べる
    use yolu_core::{FlipRule, TipSelection};
    assert!(
        h.get_by_label("左右反転: なし")
            .accesskit_node()
            .is_disabled(),
        "丸い筆先は反転しても同じ"
    );
    h.get_by_label("繰り返し方法: ランダム").click();
    h.run();
    let at = popup_item(&h, "折り返し").center();
    click(&mut h, at);
    assert_eq!(st(&h).m2().brush.tip.selection, TipSelection::PingPong);
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("dots")))));
    h.run();
    for (item, rule) in [
        ("反転", FlipRule::On),
        ("ランダム", FlipRule::Random),
        ("折り返し時に反転", FlipRule::OnFold),
        ("なし", FlipRule::Off),
    ] {
        let flip = format!(
            "左右反転: {}",
            match st(&h).m2().brush.tip.flip_x {
                FlipRule::Off => "なし",
                FlipRule::On => "反転",
                FlipRule::Random => "ランダム",
                FlipRule::OnFold => "折り返し時に反転",
            }
        );
        h.get_by_label(flip.as_str()).click();
        h.run();
        let at = popup_item(&h, item).center();
        click(&mut h, at);
        assert_eq!(st(&h).m2().brush.tip.flip_x, rule, "{item}");
    }
    // 紙質: 画像の箱から選ぶ
    let at = rect_of(&h, "紙質", |r| {
        window.contains(r.center()) && r.left() < window.left() + 168.0
    })
    .center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.detail.category, Category::Paper);
    // 同じ名前の箱は、右のプロパティのステンシルのタブにもある（ウィンドウの中の箱を押す）
    let image = rect_of(&h, "紙質の画像: なし", |r| {
        window.contains(r.center())
    });
    click(&mut h, image.center());
    let at = popup_item(&h, "粒子").center();
    click(&mut h, at);
    assert!(st(&h).m2().brush.texture.is_some());
    in_pane(&h, "質感の濃さ");
    // 回転（−180〜180°）
    let rotation = in_pane(&h, "紙質の回転");
    click(&mut h, pos2(rotation.right() - 1.0, rotation.center().y));
    assert!(st(&h).m2().brush.texture.as_ref().unwrap().angle > 170.0);
    // 効果（インク）: ぼかしを選ぶと描くツールのまま、ストロークは効果のブラシで始まる
    let at = rect_of(&h, "インク", |r| {
        window.contains(r.center()) && r.left() < window.left() + 168.0
    })
    .center();
    click(&mut h, at);
    h.get_by_label("効果: ペイント").click();
    h.run();
    let at = popup_item(&h, "ぼかし").center();
    click(&mut h, at);
    assert!(matches!(st(&h).m2().brush.effect, BrushEffect::Blur { .. }));
    in_pane(&h, "ぼかしの半径");
    // ツールプロパティにもぼかしの半径が出る
    rect_of(&h, "ぼかしの半径", in_panel);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    assert!(st(&h).message().is_empty(), "{}", st(&h).message());
    // 既定に戻す: ペイントへ
    h.get_by_label("インクを既定に戻す").click();
    h.run();
    assert_eq!(st(&h).m2().brush.effect, BrushEffect::Paint);
}

#[test]
fn the_effect_type_is_unavailable_for_the_eraser_with_a_reason() {
    let mut h = app(1600.0, 1000.0, 128);
    h.act(Action::SelectTool(Tool::Eraser));
    open_detail(&mut h, Category::Ink);
    let type_box = h.get_by_label("効果: ペイント");
    assert!(type_box.accesskit_node().is_disabled());
    type_box.click();
    h.run();
    assert!(st(&h).popup().is_none(), "消しゴムでは効果を選ばせない");
    // 注記の行は出さない（理由はツールチップ）
    for note in ["消しゴムでは使えません", "3D では使えません"] {
        assert!(h.query_by_label(note).is_none(), "{note}");
    }
}

#[test]
fn the_detail_window_moves_with_its_header_and_the_canvas_still_paints_beside_it() {
    let mut h = app(1600.0, 900.0, 256);
    open_detail(&mut h, Category::Tip);
    let before = detail_rect(&h);
    // 見出しをドラッグして動かす
    let grab = pos2(before.center().x, before.top() + 14.0);
    drag(
        &mut h,
        &[grab, offset(grab, 30.0, 10.0), offset(grab, 120.0, 40.0)],
    );
    let after = detail_rect(&h);
    assert!(
        (after.left() - before.left() - 120.0).abs() < 2.0,
        "{before:?} {after:?}"
    );
    assert!((after.top() - before.top() - 40.0).abs() < 2.0);
    // ウィンドウを開いたまま、ウィンドウの外のキャンバスに描ける
    let canvas = canvas_rect(&h);
    let spot = pos2(canvas.right() - 40.0, canvas.bottom() - 40.0);
    assert!(!after.contains(spot));
    let steps = st(&h).doc().undo_count();
    drag(&mut h, &[spot, offset(spot, -20.0, 0.0)]);
    assert_eq!(st(&h).doc().undo_count(), steps + 1);
    assert!(st(&h).brushes().ui.detail.open);
    // ウィンドウの上の押下はキャンバスへ通さない
    let inside = after.center();
    let steps = st(&h).doc().undo_count();
    drag(
        &mut h,
        &[offset(inside, 0.0, 200.0), offset(inside, 30.0, 200.0)],
    );
    assert_eq!(st(&h).doc().undo_count(), steps);
}

#[test]
fn the_brush_fields_stay_enabled_while_only_the_3d_view_can_be_painted() {
    use yolu_app::engine::Channel;
    let mut h = app(1600.0, 1000.0, 256);
    click_tab(&mut h, Tab::View3d);
    h.act(Action::LoadDemoModel);
    h.run();
    assert!(st(&h).view3d().paintable_on_screen());
    // ツールプロパティ: 手ぶれ補正も 3D の面のダブに効く（直径・不透明度・硬さ・流量・間隔も）
    assert!(!h.get_by_label("手ぶれ補正").accesskit_node().is_disabled());
    for label in ["直径", "不透明度", "硬さ", "流量", "間隔"] {
        assert!(!rect_of_enabled(&h, label), "{label} は 3D でも効く");
    }
    // 詳細のウィンドウ: ブラシサイズ・ブラシ先端・散布効果・ストローク・補正・入り抜きも 3D で効く。注記の行は出さない
    for (category, enabled) in [
        (
            Category::Size,
            vec![
                "直径",
                "最低 1 ピクセル",
                "画面上のサイズで指定",
                "大きさのゆらぎ",
            ],
        ),
        (Category::Tip, vec!["真円率", "角度"]),
        (Category::Spray, vec!["散布", "数"]),
        (Category::Stroke, vec!["間隔"]),
        (Category::Stabilization, vec!["手ぶれ補正", "曲線"]),
        (Category::Taper, vec!["入りの長さ", "抜きの長さ"]),
    ] {
        open_detail(&mut h, category);
        let window = detail_rect(&h);
        for label in enabled {
            let node = h
                .query_all_by_label(label)
                .find(|n| {
                    window.contains(n.rect().center()) && n.rect().left() > window.left() + 168.0
                })
                .unwrap_or_else(|| panic!("{category:?} {label}"));
            assert!(!node.accesskit_node().is_disabled(), "{category:?} {label}");
        }
        for note in [
            "3D では効きません",
            "3D では使えません",
            "角度は 3D では効きません",
            "硬さ以外は 3D では効きません",
        ] {
            assert!(h.query_by_label(note).is_none(), "注記は出さない: {note}");
        }
    }
    // 3D のビューが出ているあいだは、ストロークの欄に 3D の塗りの切り替えが出る
    open_detail(&mut h, Category::Stroke);
    in_pane(&h, "隠れた所も塗る");
    // ブラシ先端の硬さは 3D でも効く（丸い先端）
    open_detail(&mut h, Category::Tip);
    let hardness = in_pane_node(&h, "硬さ");
    assert!(!hardness.accesskit_node().is_disabled());
    // 色の変化のゆらぎは 3D でも効く。ただし色を持たないチャンネルでは無効
    open_detail(&mut h, Category::ColorChange);
    assert!(!in_pane_node(&h, "色相のゆらぎ")
        .accesskit_node()
        .is_disabled());
    h.act(Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    h.run();
    assert!(in_pane_node(&h, "色相のゆらぎ")
        .accesskit_node()
        .is_disabled());
    assert!(in_pane_node(&h, "ブラシ先端色の変化")
        .accesskit_node()
        .is_disabled());
    // 効果のブラシ（ぼかし・指先・クローン）は 3D の面でも効く（3D の面のブラシ）
    h.act(Action::M2Ui(UiOp::PaintChannel(Channel::Color)));
    h.m2_mut().brush.effect = BrushEffect::BLUR;
    open_detail(&mut h, Category::Ink);
    assert!(!in_pane_node(&h, "ぼかしの半径")
        .accesskit_node()
        .is_disabled());
}

fn in_pane_node<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
    let window = detail_rect(h);
    h.query_all_by_label(label)
        .find(|n| window.contains(n.rect().center()) && n.rect().left() > window.left() + 168.0)
        .unwrap_or_else(|| panic!("{label}"))
}

/// 左のパネルの、その名前のスライダーが無効か。
fn rect_of_enabled(h: &H, label: &str) -> bool {
    h.query_all_by_label(label)
        .find(|n| in_panel(n.rect()) && n.rect().height() < 30.0)
        .map(|n| n.accesskit_node().is_disabled())
        .unwrap_or(true)
}

// ───────── 日英・はみ出し・見た目 ─────────

/// 描いた文字がどれも、描いた範囲（クリップ）の中に収まっている（横に切れていない）。
fn clipped_texts(h: &H) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            egui::epaint::Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                let visible = clip.y_range().contains(bounds.center().y);
                if visible
                    && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
                {
                    out.push(format!(
                        "{}: {bounds:?} clip={clip:?}",
                        text.galley.job.text
                    ));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}

fn drawn_texts(h: &H) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            egui::epaint::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

#[test]
fn the_brush_panel_and_the_detail_window_speak_both_languages_without_clipped_text() {
    for lang in Lang::ALL {
        for (width, height) in [(1600.0, 960.0), (1280.0, 800.0)] {
            let mut h = app(width, height, 128);
            language(&mut h, lang);
            // 一覧は全グループ・利用者のブラシ 1 つ
            h.act(Action::Brush(BrushAction::Add));
            for group in Group::ALL {
                h.app_mut().show_brush_group(group);
                h.run();
                let clipped = clipped_texts(&h);
                assert!(
                    clipped.is_empty(),
                    "{lang:?} {width} {group:?}: {clipped:#?}"
                );
            }
            for category in Category::ALL {
                open_detail(&mut h, category);
                let clipped = clipped_texts(&h);
                assert!(
                    clipped.is_empty(),
                    "{lang:?} {width} {category:?}: {clipped:#?}"
                );
                if lang == Lang::En {
                    let japanese: Vec<String> = drawn_texts(&h)
                        .into_iter()
                        .filter(|t| has_japanese(t))
                        .collect();
                    assert!(
                        japanese.is_empty(),
                        "英語の画面に日本語: {category:?} {japanese:?}"
                    );
                }
            }
        }
    }
    // 英語のパネルに日本語が無い（組み込みの名前・グループ・ツールプロパティ・ブラシサイズ）
    let mut h = app(1600.0, 900.0, 128);
    language(&mut h, Lang::En);
    for group in Group::ALL {
        h.app_mut().show_brush_group(group);
        h.run();
        let japanese: Vec<String> = drawn_texts(&h)
            .into_iter()
            .filter(|t| has_japanese(t))
            .collect();
        assert!(japanese.is_empty(), "{group:?}: {japanese:?}");
    }
}

#[test]
fn snapshot_brush_panel() {
    let mut h = app(1600.0, 900.0, 128);
    click_row(&mut h, "インクペン");
    h.run();
    let rect =
        crate::common::shot::group_span(&h, yolu_app::Tab::SubTools, Some(yolu_app::Tab::Color));
    crate::common::shot::clean_region(&mut h, "brushes_panel", rect);
}

#[test]
fn snapshot_brush_panel_modified_and_user_brush() {
    // ブラシサイズの格子が 2 段になっても一覧の「鉛筆」まで見える高さ
    let mut h = app(1600.0, 1300.0, 128);
    click_row(&mut h, "鉛筆");
    h.brush_mut().radius = 9.0;
    h.act(Action::Brush(BrushAction::Duplicate(b("pencil"))));
    h.brush_mut().hardness = 0.2;
    h.m2_mut().brush.tip.angle = 40.0;
    h.run();
    h.run();
    let rect =
        crate::common::shot::group_span(&h, yolu_app::Tab::SubTools, Some(yolu_app::Tab::Color));
    crate::common::shot::clean_region(&mut h, "brushes_panel_modified", rect);
}

#[test]
fn snapshot_brush_panel_effects_and_english() {
    let mut h = app(1600.0, 900.0, 128);
    language(&mut h, Lang::En);
    h.act(Action::Brush(BrushAction::Select(b("smudge"))));
    h.run();
    h.run();
    let rect =
        crate::common::shot::group_span(&h, yolu_app::Tab::SubTools, Some(yolu_app::Tab::Color));
    crate::common::shot::clean_region(&mut h, "brushes_panel_effects_english", rect);
}

#[test]
fn snapshot_brush_panel_eraser_group() {
    let mut h = app(1600.0, 900.0, 128);
    h.act(Action::Brush(BrushAction::Select(b("soft-eraser"))));
    h.run();
    h.run();
    let rect =
        crate::common::shot::group_span(&h, yolu_app::Tab::SubTools, Some(yolu_app::Tab::Color));
    crate::common::shot::clean_region(&mut h, "brushes_panel_eraser", rect);
}

#[test]
fn snapshot_brush_detail_window() {
    let mut h = app(1600.0, 1000.0, 128);
    h.act(Action::Brush(BrushAction::Select(b("chalk"))));
    for (category, name) in [
        (Category::Size, "brushes_detail_size"),
        (Category::Ink, "brushes_detail_ink"),
        (Category::ColorChange, "brushes_detail_color_change"),
        (Category::Tip, "brushes_detail_tip"),
        (Category::Stroke, "brushes_detail_stroke"),
        (Category::Paper, "brushes_detail_paper"),
        (Category::Taper, "brushes_detail_taper"),
    ] {
        open_detail(&mut h, category);
        let rect = detail_rect(&h);
        crate::common::shot::clean_region(&mut h, name, rect);
    }
}

#[test]
fn snapshot_brush_dynamics_window() {
    let mut h = app(1600.0, 1000.0, 128);
    h.act(Action::Brush(BrushAction::Select(b("chalk"))));
    open_detail(&mut h, Category::Size);
    h.m2_mut().brush.influences.size.tilt = yolu_core::InputUse {
        on: true,
        response: yolu_core::InputResponse::new(0.2, vec![])
            .unwrap()
            .with_max(0.8)
            .unwrap(),
    };
    open_dynamics(&mut h, Row::Size);
    let both = detail_rect(&h).union(dynamics_window(&h));
    crate::common::shot::clean_region(&mut h, "brushes_dynamics_window", both);
}

#[test]
fn snapshot_brush_detail_window_english_ink() {
    let mut h = app(1600.0, 1000.0, 128);
    language(&mut h, Lang::En);
    h.act(Action::Brush(BrushAction::Select(b("clone"))));
    open_detail(&mut h, Category::Ink);
    let rect = detail_rect(&h);
    crate::common::shot::clean_region(&mut h, "brushes_detail_ink_english", rect);
}

#[test]
fn snapshot_brush_detail_window_english_size() {
    let mut h = app(1600.0, 1000.0, 128);
    language(&mut h, Lang::En);
    h.act(Action::Brush(BrushAction::Select(b("chalk"))));
    open_detail(&mut h, Category::Size);
    assert!(h.query_by_label("At least 1 pixel").is_some());
    let rect = detail_rect(&h);
    crate::common::shot::clean_region(&mut h, "brushes_detail_size_english", rect);
}

#[test]
fn each_tool_panel_scrolls_on_its_own_when_its_group_is_too_short_and_the_list_scrolls_on_its_own()
{
    // 低いウィンドウ: ツールプロパティの組には、ブラシの設定が全部は入らないので、その中だけがスクロールする
    let mut h = app(1280.0, 640.0, 128);
    let props_tab = h.state().tab_rects[&Tab::ToolProperties];
    let size_tab = h.state().tab_rects[&Tab::BrushSize];
    let body = Rect::from_min_max(
        pos2(props_tab.left() - 2.0, props_tab.bottom()),
        pos2(props_tab.left() + 300.0, size_tab.top()),
    );
    let brush = Tool::Brush as usize;
    let content = st(&h).subtools().ui.props_content[brush];
    assert!(
        content > body.height(),
        "中身 {content} はパネルの高さ {} を超える",
        body.height()
    );
    assert_eq!(st(&h).subtools().ui.props_scroll[brush], 0.0);
    let wheel = |h: &mut H, at: egui::Pos2, dy: f32| {
        move_to(h, at);
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    };
    // ツールプロパティの上のホイールは、ツールプロパティだけを送る。一覧は動かない
    let list_before = st(&h).brushes().ui.list_scroll;
    wheel(&mut h, body.center(), -400.0);
    let scrolled = st(&h).subtools().ui.props_scroll[brush];
    assert!(
        scrolled > 0.0,
        "ツールプロパティがスクロールする: {scrolled}"
    );
    assert!(scrolled < content, "中身の高さまでは送らない: {scrolled}");
    assert_eq!(
        st(&h).brushes().ui.list_scroll,
        list_before,
        "一覧は動かさない"
    );
    // 逆向きに戻せて、先頭より前には行かない
    wheel(&mut h, body.center(), 100_000.0);
    assert_eq!(st(&h).subtools().ui.props_scroll[brush], 0.0);
    // 一覧の上のホイールは、一覧だけを送る（ブラシの行が多いとき）
    for _ in 0..30 {
        h.get_by_label("今の設定を新しいブラシに").click();
        h.run();
    }
    let sub_tab = h.state().tab_rects[&Tab::SubTools];
    let list_at = pos2(sub_tab.left() + 80.0, sub_tab.bottom() + 70.0);
    wheel(&mut h, list_at, -200.0);
    assert!(
        st(&h).brushes().ui.list_scroll > 0.0,
        "一覧がスクロールする"
    );
    assert_eq!(
        st(&h).subtools().ui.props_scroll[brush],
        0.0,
        "ツールプロパティは動かさない"
    );
    // 十分に高いウィンドウでは、ツールプロパティは送れない
    let mut tall = app(1280.0, 1400.0, 128);
    let props_tab = tall.state().tab_rects[&Tab::ToolProperties];
    wheel(
        &mut tall,
        pos2(props_tab.left() + 100.0, props_tab.bottom() + 60.0),
        -400.0,
    );
    assert_eq!(st(&tall).subtools().ui.props_scroll[brush], 0.0);
    // ブラシサイズの組も、入りきらなければ自分の中だけをスクロールし、スクロールしても丸は押せる（最後の段の丸が見える所まで送って、押す）
    let size_tab = h.state().tab_rects[&Tab::BrushSize];
    let color_tab = h.state().tab_rects[&Tab::Color];
    let body = Rect::from_min_max(
        size_tab.left_bottom(),
        pos2(size_tab.left() + 300.0, color_tab.top()),
    );
    let last = *yolu_app::panels::brushes::SIZES.last().unwrap();
    let cell = |h: &H| {
        rect_of(h, &format!("{last} px"), |r| {
            r.left() < 400.0 && r.width() < 60.0
        })
    };
    assert!(!body.contains_rect(cell(&h)), "初めは最後の段が見えない");
    h.subtools_mut().ui.sizes_scroll = 100_000.0;
    h.run();
    let scrolled = st(&h).subtools().ui.sizes_scroll;
    assert!(
        scrolled > 0.0 && scrolled < 1_000.0,
        "丸の高さまで: {scrolled}"
    );
    let at = cell(&h);
    assert!(
        body.contains_rect(at),
        "送ると最後の段が見える {at:?} {body:?}"
    );
    click(&mut h, at.center());
    assert_eq!(st(&h).brush().radius, last / 2.0);
}

/// ブラシサイズのパネル（既定の高さ）の見える範囲と、丸の名前（`30 px` など）で探した丸の範囲。
fn size_panel(h: &H) -> Rect {
    let size_tab = h.state().tab_rects[&Tab::BrushSize];
    let color_tab = h.state().tab_rects[&Tab::Color];
    Rect::from_min_max(
        size_tab.left_bottom(),
        pos2(size_tab.left() + 300.0, color_tab.top()),
    )
}

fn size_cell(h: &H, label: &str) -> Rect {
    rect_of(h, &format!("{label} px"), |r| {
        r.left() < 400.0 && r.width() < 60.0
    })
}

#[test]
fn the_size_panel_opens_scrolled_to_the_chosen_size_and_follows_it() {
    let mut h = app(1280.0, 640.0, 128);
    let panel = size_panel(&h);
    // 既定の大きさ（直径 32）に近い段は、初めから見える範囲にある（1 段目だけが見えるのではない）
    assert_eq!(h.brush().radius * 2.0, 32.0);
    assert!(h.subtools().ui.sizes_scroll > 0.0, "初めから送ってある");
    let at = size_cell(&h, "30");
    assert!(
        panel.contains_rect(at),
        "30 px の丸が見える {at:?} {panel:?}"
    );
    // 大きさが変わると、新しい段が見える所に来る（キー・スライダー・ブラシの切り替えも、同じ半径の欄を変える）
    h.brush_mut().radius = 100.0;
    h.run();
    let at = size_cell(&h, "200");
    assert!(
        panel.contains_rect(at),
        "200 px の丸が見える {at:?} {panel:?}"
    );
    h.brush_mut().radius = 0.5;
    h.run();
    assert!(panel.contains_rect(size_cell(&h, "1")), "1 px の丸が見える");
    // 段を押して変えても、押した丸が見える所のまま（見えている段は動かさない）
    h.brush_mut().radius = 10.0;
    h.run();
    let before = h.subtools().ui.sizes_scroll;
    let at = size_cell(&h, "30");
    assert!(panel.contains_rect(at), "30 px が見える {at:?}");
    click(&mut h, at.center());
    assert_eq!(h.brush().radius, 15.0);
    assert_eq!(
        h.subtools().ui.sizes_scroll,
        before,
        "見えている段へは動かさない"
    );
}

#[test]
fn the_size_panel_keeps_a_scroll_by_the_user_until_the_size_changes() {
    let mut h = app(1280.0, 640.0, 128);
    let panel = size_panel(&h);
    let followed = h.subtools().ui.sizes_scroll;
    assert!(followed > 0.0);
    // ホイールで先頭へ戻す
    move_to(&h, panel.center());
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 100_000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(h.subtools().ui.sizes_scroll, 0.0, "利用者が戻した");
    // 大きさが変わらないあいだは、何度描いても動かさない
    for _ in 0..5 {
        h.run();
    }
    assert_eq!(h.subtools().ui.sizes_scroll, 0.0, "大きさが同じなら保つ");
    // 同じ段のまま半径が少し変わっても（直径 32 → 33）、段は替わらないので保つ
    h.brush_mut().radius = 16.5;
    h.run();
    assert_eq!(h.subtools().ui.sizes_scroll, 0.0);
    // 段が替わると、また送る
    h.brush_mut().radius = 100.0;
    h.run();
    assert!(h.subtools().ui.sizes_scroll > 0.0, "段が替わった");
    assert!(panel.contains_rect(size_cell(&h, "200")));
}

#[test]
fn escape_cancels_a_row_drag_and_nothing_moves() {
    let mut h = app(1600.0, 1200.0, 128);
    for _ in 0..2 {
        h.get_by_label("今の設定を新しいブラシに").click();
        h.run();
    }
    let order = st(&h).toolset().set.brushes();
    let a = row_of(&h, "ブラシ");
    let b2 = row_of(&h, "ブラシ 2");
    let from = pos2(b2.left() + 30.0, b2.center().y);
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, pos2(from.x, from.y - 8.0));
    h.step();
    move_to(&h, pos2(a.left() + 30.0, a.top() + 4.0));
    h.step();
    assert!(st(&h).toolset().ui.drag.is_some(), "動かしている");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert_eq!(st(&h).toolset().ui.drag, None);
    release(
        &h,
        pos2(a.left() + 30.0, a.top() + 4.0),
        PointerButton::Primary,
    );
    h.run();
    assert_eq!(st(&h).toolset().set.brushes(), order, "落とさない");
}

#[test]
fn the_hardness_in_the_tool_properties_follows_the_tip_image() {
    let mut h = app(1600.0, 900.0, 128);
    let hardness_disabled = |h: &H| {
        h.query_all_by_label("硬さ")
            .find(|n| in_panel(n.rect()) && n.rect().height() < 30.0)
            .expect("ツールプロパティの硬さ")
            .accesskit_node()
            .is_disabled()
    };
    assert!(!hardness_disabled(&h), "丸い先端なら硬さが効く");
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("dots")))));
    h.run();
    assert!(hardness_disabled(&h), "画像の先端は画像の縁のまま");
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(None))));
    h.run();
    assert!(!hardness_disabled(&h));
}

#[test]
fn the_hardness_is_decided_in_one_place_for_the_tool_properties_and_the_tip_category() {
    use yolu_app::panels::brush_props::hardness_applies;
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Tip);
    let tool_off = |h: &H| rect_of_enabled(h, "硬さ");
    let shape_off = |h: &H| in_pane_node(h, "硬さ").accesskit_node().is_disabled();
    // 2D: 丸い先端なら両方が効き、画像の先端なら両方が効かない（画像の縁のまま）
    assert!(!tool_off(&h) && !shape_off(&h));
    assert!(hardness_applies(st(&h)));
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("dots")))));
    h.run();
    assert!(!hardness_applies(st(&h)));
    assert!(
        tool_off(&h) && shape_off(&h),
        "画像の先端は 2D では効かない"
    );
    // 3D: 面のダブも画像の筆先を使うので、2D と同じく画像の先端では両方が効かない
    click_tab(&mut h, Tab::View3d);
    h.act(Action::LoadDemoModel);
    h.run();
    assert!(st(&h).view3d().paintable_on_screen());
    assert!(st(&h).m2().brush.tip.image.is_some());
    assert!(!hardness_applies(st(&h)));
    assert!(
        tool_off(&h) && shape_off(&h),
        "画像の先端は 3D でも効かない（同じブラシで食い違わない）"
    );
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(None))));
    h.run();
    assert!(!tool_off(&h) && !shape_off(&h));
}

#[test]
fn the_dynamics_button_beside_the_tool_hardness_is_off_for_an_image_tip() {
    let mut h = app(1600.0, 900.0, 128);
    // 硬さの行は、ツールプロパティの 4 つの影響元のボタンの 4 つ目
    let button = |h: &H| {
        let mut nodes: Vec<(Rect, bool)> = h
            .query_all_by_label("影響元")
            .filter(|n| in_panel(n.rect()))
            .map(|n| (n.rect(), n.accesskit_node().is_disabled()))
            .collect();
        nodes.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        nodes[3]
    };
    assert!(!button(&h).1);
    // 丸い筆先では押せる。押すと硬さの設定のウィンドウが開く
    let at = button(&h).0.center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.detail.dynamics, Some(Row::Hardness));
    let at = button(&h).0.center();
    click(&mut h, at);
    assert_eq!(st(&h).brushes().ui.detail.dynamics, None);
    // 画像の筆先では硬さが効かないので、ボタンも押せない
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("dots")))));
    h.run();
    assert!(button(&h).1);
    let at = button(&h).0.center();
    click(&mut h, at);
    assert_eq!(
        st(&h).brushes().ui.detail.dynamics,
        None,
        "押せない所では何も起きない"
    );
    // 丸い筆先へ戻すとまた押せる
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(None))));
    h.run();
    assert!(!button(&h).1);
}

#[test]
fn the_hardness_dynamics_follow_the_tip_and_reset_with_the_tip_category() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Tip);
    open_dynamics(&mut h, Row::Hardness);
    let pressure = |h: &H| st(h).m2().brush.influences.hardness.pressure.clone();
    // 丸い筆先: 筆圧の切り替えは押せる。切っているので最小値は押せない
    assert!(!in_dynamics(&h, "筆圧")[0].1);
    assert!(in_dynamics(&h, "最小値")[0].1);
    // 切り替えを入れると、最小値が使える。ほかの項目は変わらない
    let toggle = in_dynamics(&h, "筆圧")[0].0;
    click(&mut h, toggle.center());
    assert!(pressure(&h).on);
    assert!(
        st(&h).brush().pressure.size
            && st(&h).brush().pressure.opacity
            && !st(&h).brush().pressure.flow
    );
    assert!(!in_dynamics(&h, "最小値")[0].1);
    assert!(st(&h).brush_is_modified(b("standard")));
    // 画像の筆先では、硬さの影響元は触れない（入っていても）
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("dots")))));
    h.run();
    assert!(in_dynamics(&h, "筆圧")[0].1 && in_dynamics(&h, "最小値")[0].1);
    click(&mut h, toggle.center());
    assert!(pressure(&h).on, "押せない所では何も起きない");
    let before = pressure(&h).response;
    let min = in_dynamics(&h, "最小値")[0].0;
    click(&mut h, pos2(min.left() + min.width() * 0.5, min.center().y));
    assert_eq!(pressure(&h).response, before, "押せない所では何も起きない");
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::Tip(None))));
    h.run();
    assert!(!in_dynamics(&h, "筆圧")[0].1);
    // 「ブラシ先端を既定に戻す」は、硬さの影響元も切る（最小値・曲線と一緒に）
    let min = in_dynamics(&h, "最小値")[0].0;
    click(&mut h, pos2(min.left() + min.width() * 0.4, min.center().y));
    assert!(!pressure(&h).response.is_identity());
    h.get_by_label("ブラシ先端を既定に戻す").click();
    h.run();
    assert!(!pressure(&h).on);
    assert!(pressure_identity(&st(&h).m2().brush));
    assert!(!st(&h).brush_is_modified(b("standard")));
}

// ───────── 見本の点滅 ─────────

/// ツールのプロパティのつまみを動かした直後のフレームでも、ツールの見本は前の絵を出し続ける（紙だけになって点滅しない）。新しい見本が
/// できたら替わる（見本は実際のウィンドウと同じく別のスレッドで描く）。
#[test]
fn the_tool_sample_keeps_its_picture_while_the_changed_brush_is_redrawn() {
    use yolu_app::brushes::sample::{key_of, SampleSpec};
    let mut h = app(1600.0, 900.0, 128);
    let ctx = h.ctx.clone();
    h.ui_mut().samples.render_in_background(&ctx);
    let key = |h: &H| {
        let s = st(h);
        let mut brush = s.m2().brush.clone();
        brush.base = s.brush().settings([1.0; 4], false);
        key_of(&brush, SampleSpec::tool(false))
    };
    // この絵を最後のフレームに描いたか
    let drawn = |h: &mut H, key: u64| {
        let Some(id) = h.ui_mut().samples.texture(&ctx, key) else {
            return false;
        };
        h.output()
            .shapes
            .iter()
            .any(|c| matches!(&c.shape, egui::Shape::Mesh(m) if m.texture_id == id))
    };
    let wait = |h: &mut H, key: u64| {
        let start = std::time::Instant::now();
        while !drawn(h, key) {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(30),
                "見本が描かれない"
            );
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    let old = key(&h);
    wait(&mut h, old);
    // つまみを動かす（半径のつまみと同じく、今の設定を変える）
    h.brush_mut().radius = 40.0;
    h.step();
    let new = key(&h);
    assert_ne!(new, old);
    let fresh = st(&h).ui().samples.image(new).is_some() && drawn(&mut h, new);
    assert!(
        drawn(&mut h, old) || fresh,
        "動かした直後のフレームも、見本は紙だけにならない"
    );
    wait(&mut h, new);
    assert!(!drawn(&mut h, old), "新しい見本ができたら替わる");
}

/// 大きさ・不透明度・流量・硬さの筆圧の応えが、どれも既定（最小値 0・直線）か。
fn pressure_identity(b: &yolu_core::Brush) -> bool {
    use yolu_core::brush::Item;
    [Item::Size, Item::Opacity, Item::Flow, Item::Hardness]
        .into_iter()
        .all(|i| b.influences.get(i).pressure.response.is_identity())
}
