//! 設定のウィンドウの「ショートカット」の区分の画面の試験（egui_kittest。描画は wgpu のソフトの描画）: 欄を押してキーを入れる・外す・やめる、待っている間の
//! キーがキーの表・キャンバス・ほかのウィンドウへ漏れない（Esc・Tab・修飾だけ・繰り返し）、ぶつかりの赤と保存の断り、行ごとに既定へ戻す、名前と
//! キーで探す、マウスの組み合わせ、パイメニューの編集と実行。絵は日英。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::keyconfig::GroupKey;
use yolu_app::keymap::{self, Scope};
use yolu_app::lang::Lang;
use yolu_app::prefs::PrefsAction;
use yolu_app::shortcuts::editor::{Section, Target};
use yolu_app::state::{Action, PopupKind, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

/// 設定のウィンドウを「ショートカット」の区分で開いたアプリ。
fn editor(lang: Lang) -> H {
    let mut h = app(1280.0, 860.0, 64);
    h.app_mut().set_language(lang);
    h.act(Action::ShowShortcuts);
    h.run();
    h
}

fn window(h: &H) -> Rect {
    yolu_app::prefs::last_rect(&h.ctx).expect("ウィンドウを描いた")
}

/// 設定のウィンドウを閉じる（キーがキャンバスなどへ届くようにする）。
fn close_window(h: &mut H) {
    h.act(Action::Prefs(PrefsAction::Close));
    h.run();
}

fn section(h: &mut H, s: Section) {
    let label = s.label(h.lang());
    let w = window(h);
    let r = rect_of(h, label, |r| {
        w.contains_rect(r) && r.left() < w.left() + 220.0
    });
    click(h, r.center());
}

/// 名前で探す（表の行を絞る。欄に打つのと同じ）。
fn search(h: &mut H, text: &str) {
    h.shortcuts_mut().search = text.into();
    h.run();
}

fn chip(h: &H, target: Target) -> Rect {
    h.shortcuts()
        .chip_rect(target)
        .unwrap_or_else(|| panic!("{target:?}"))
}

fn click_chip(h: &mut H, target: Target) {
    let at = chip(h, target).center();
    click(h, at);
}

fn hover_chip(h: &mut H, target: Target) {
    let at = chip(h, target).center();
    hover_and_wait(h, at);
}

fn key_chip(group: GroupKey, i: usize) -> Target {
    Target::Key {
        group,
        chip: Some(i),
    }
}

fn capturing(h: &H) -> bool {
    let app = h.app();
    let open = app.shortcuts().capturing();
    assert_eq!(
        open,
        matches!(
            app.popup().as_ref().map(|p| p.kind),
            Some(PopupKind::KeyCapture)
        ),
        "待っている間と受け皿は一緒"
    );
    open
}

fn key_event(h: &H, key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    });
}

fn triggers(h: &H, group: GroupKey) -> Vec<keymap::Trigger> {
    h.keys().triggers(group)
}

fn trigger(modifiers: Modifiers, key: Key) -> keymap::Trigger {
    keymap::Trigger::Key {
        modifiers: modifiers.into(),
        key: key.into(),
    }
}

const FLIP: GroupKey = ("view.flip", Scope::Everywhere);
const BRUSH: GroupKey = ("tool.brush", Scope::Paint);

#[test]
fn pressing_a_key_cell_takes_the_next_key_and_the_key_works_and_follows_into_the_menu() {
    let mut h = editor(Lang::Ja);
    section(&mut h, Section::Paint);
    let at = chip(&h, key_chip(BRUSH, 0));
    click(&mut h, at.center());
    assert!(capturing(&h));
    // 修飾だけを押しても決めない（Ctrl を押している間は「Ctrl+…」）
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.run();
    assert!(capturing(&h));
    key(&h, Key::K, Modifiers::COMMAND);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert!(!capturing(&h));
    assert_eq!(
        triggers(&h, BRUSH),
        vec![trigger(Modifiers::COMMAND, Key::K)]
    );
    // 割り当てたキーで、ブラシに替わる（ウィンドウを閉じて）
    close_window(&mut h);
    h.act(Action::SelectTool(Tool::Eraser));
    h.run();
    key(&h, Key::K, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.tool(), Tool::Brush);
    // 前のキー（B）では替わらない
    h.act(Action::SelectTool(Tool::Eraser));
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert_eq!(h.tool(), Tool::Eraser);
    // ツールの名前に添える文字も付いてくる
    assert_eq!(yolu_app::shortcuts::tool_key(Tool::Brush), "Ctrl+K");
}

#[test]
fn escape_cancels_backspace_removes_and_keys_taken_while_waiting_do_not_leak() {
    let mut h = editor(Lang::Ja);
    // 選択範囲（Esc がキャンバスへ漏れれば解除される）
    h.act(Action::Sel(yolu_app::selection::SelAction::Edit(
        yolu_app::selection::SelEdit::All,
    )));
    h.run();
    search(&mut h, "左右反転");
    let at = chip(&h, key_chip(FLIP, 0));
    click(&mut h, at.center());
    assert!(capturing(&h));
    // Esc: やめるだけ（割り当ては変えない・ウィンドウは閉じない・選択範囲は残る）
    move_to(&h, at.center());
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(!capturing(&h));
    assert_eq!(triggers(&h, FLIP), vec![trigger(Modifiers::NONE, Key::H)]);
    assert!(h.prefs().open, "ウィンドウは閉じない");
    assert!(h.doc().selection().is_some(), "選択範囲はそのまま");
    // 待っている間の X は、色の入れ替え（キーの表）へ行かずに割り当てになる
    let colors = h.color().main;
    click(&mut h, at.center());
    key(&h, Key::X, Modifiers::NONE);
    h.run();
    assert_eq!(h.color().main, colors, "色は入れ替わらない");
    assert_eq!(triggers(&h, FLIP), vec![trigger(Modifiers::NONE, Key::X)]);
    // 待っている間の Ctrl+Tab は、モードのパイを開かずに割り当てになる
    click_chip(&mut h, key_chip(FLIP, 0));
    key(&h, Key::Tab, Modifiers::CTRL | Modifiers::COMMAND);
    h.run();
    assert!(h.pie().open.is_none(), "パイを開かない");
    assert_eq!(
        triggers(&h, FLIP),
        vec![trigger(Modifiers::COMMAND, Key::Tab)]
    );
    // Backspace で外す
    click_chip(&mut h, key_chip(FLIP, 0));
    key(&h, Key::Backspace, Modifiers::NONE);
    h.run();
    assert!(triggers(&h, FLIP).is_empty());
    // Tab も 1 つのキーとして入る（入力欄へフォーカスが移らない）
    click_chip(
        &mut h,
        Target::Key {
            group: FLIP,
            chip: None,
        },
    );
    key(&h, Key::Tab, Modifiers::NONE);
    h.run();
    assert_eq!(triggers(&h, FLIP), vec![trigger(Modifiers::NONE, Key::Tab)]);
    assert!(
        h.ctx.memory(|m| m.focused()).is_none(),
        "フォーカスは移らない"
    );
}

#[test]
fn holding_the_captured_key_does_not_run_the_new_binding_by_repeat() {
    let mut h = editor(Lang::Ja);
    search(&mut h, "左右反転");
    let at = chip(&h, key_chip(FLIP, 0));
    click(&mut h, at.center());
    // K を押したまま（割り当てに使い、OS の繰り返しの押しが続く）
    key_event(&h, Key::K, true, false, Modifiers::NONE);
    h.step();
    assert_eq!(triggers(&h, FLIP), vec![trigger(Modifiers::NONE, Key::K)]);
    for _ in 0..4 {
        key_event(&h, Key::K, true, true, Modifiers::NONE);
        h.step();
    }
    assert!(!h.view().flip, "繰り返しで左右反転しない");
    key_event(&h, Key::K, false, false, Modifiers::NONE);
    h.run();
    // 離して押し直せば効く
    close_window(&mut h);
    key(&h, Key::K, Modifiers::NONE);
    h.run();
    assert!(h.view().flip);
}

#[test]
fn a_conflict_turns_the_row_red_names_the_other_side_and_is_not_saved_until_fixed() {
    let mut h = editor(Lang::Ja);
    section(&mut h, Section::Paint);
    let eraser: GroupKey = ("tool.eraser", Scope::Paint);
    click_chip(&mut h, key_chip(eraser, 0));
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert!(h.keys().has_conflicts());
    assert!(h.keys().unsaved);
    assert!(
        h.query_by_label_contains("ぶつかるキーがあるので")
            .is_some(),
        "保存しない理由を出す"
    );
    assert!(!h.keys().path().is_some_and(|p| p.exists()));
    // 相手の名前はツールチップに
    hover_chip(&mut h, key_chip(eraser, 0));
    assert!(h.query_by_label_contains("と同じキー").is_some());
    // 行の右端の印で既定に戻す
    let row = chip(&h, key_chip(eraser, 0));
    let w = window(&h);
    let reset = rect_of(&h, "既定に戻す", |r| {
        (r.center().y - row.center().y).abs() < 12.0 && w.contains_rect(r)
    });
    click(&mut h, reset.center());
    assert!(!h.keys().has_conflicts());
    assert_eq!(triggers(&h, eraser), vec![trigger(Modifiers::NONE, Key::E)]);
}

#[test]
fn search_by_name_and_find_by_key_narrow_the_table() {
    let mut h = editor(Lang::Ja);
    // キーで探す: H の行だけ（どこでもの左右反転と、編集の印を隠す）
    let find = rect_of(&h, "キーで探す", |_| true);
    click(&mut h, find.center());
    assert!(capturing(&h));
    key(&h, Key::H, Modifiers::NONE);
    h.run();
    let shown: Vec<GroupKey> = h
        .shortcuts()
        .chips
        .iter()
        .filter_map(|(t, _)| match t {
            Target::Key {
                group,
                chip: Some(_),
            } => Some(*group),
            _ => None,
        })
        .collect();
    assert!(
        shown.contains(&FLIP) && shown.contains(&("object.hide", Scope::Edit)),
        "{shown:?}"
    );
    assert!(!shown.contains(&BRUSH));
    // もう一度押すと全部に戻る
    let find = rect_of(&h, "H", |r| (r.center().y - find.center().y).abs() < 4.0);
    click(&mut h, find.center());
    assert!(h.shortcuts().key_filter.is_none());
    // 名前で探す（欄を押して打つ）
    section(&mut h, Section::Everywhere);
    let field = h.shortcuts().search_rect.expect("探す欄");
    click(&mut h, field.center());
    h.event(Event::Text("左右反転".into()));
    h.run();
    let shown: Vec<GroupKey> = h
        .shortcuts()
        .chips
        .iter()
        .filter_map(|(t, _)| match t {
            Target::Key { group, .. } => Some(*group),
            _ => None,
        })
        .collect();
    assert!(shown.iter().all(|g| *g == FLIP), "{shown:?}");
}

#[test]
fn a_mouse_cell_takes_the_next_button_pressed_on_it_with_its_modifiers() {
    let mut h = editor(Lang::Ja);
    let orbit = keymap::GESTURES
        .iter()
        .find(|g| g.scope == "view3d" && g.operation == keymap::Operation::Orbit)
        .unwrap()
        .index;
    search(&mut h, "回転（3D）");
    let at = chip(&h, Target::Mouse(orbit)).center();
    click(&mut h, at);
    assert!(capturing(&h));
    // キーは数えない
    key(&h, Key::K, Modifiers::NONE);
    h.run();
    assert!(capturing(&h));
    // Alt を押したまま右ボタン
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    assert!(!capturing(&h), "離しのクリックで待ち直さない");
    let combo = h.keys().combo(orbit).unwrap();
    assert_eq!(
        (
            PointerButton::from(combo.button),
            combo.alt,
            combo.shift,
            combo.ctrl
        ),
        (PointerButton::Secondary, true, false, false)
    );
    assert_eq!(
        keymap::gesture(
            "view3d",
            PointerButton::Secondary.into(),
            &Modifiers::ALT.into(),
            false
        ),
        Some(keymap::Operation::Orbit)
    );
}

#[test]
fn a_new_pie_menu_gets_items_and_a_key_and_opens_and_runs() {
    let mut h = editor(Lang::Ja);
    section(&mut h, Section::Pies);
    let add = rect_of(&h, "パイメニューを追加", |_| true);
    click(&mut h, add.center());
    let id = h.pie().menus.last().unwrap().id.clone();
    // 上の項目: 操作を名前で探して入れる（消しゴム）
    let slot = h.shortcuts().slot_rects[0];
    click(&mut h, slot.center());
    assert!(h.shortcuts().picker.is_some());
    let query = h.shortcuts().pick_rect.expect("項目を探す欄");
    click(&mut h, query.center());
    h.event(Event::Text("消しゴム".into()));
    h.run();
    let candidate = rect_of(&h, "消しゴム", |r| r.top() > query.bottom());
    click(&mut h, candidate.center());
    let menu = h.pie().menu(&id).unwrap().clone();
    assert_eq!(
        menu.slots[0],
        Some(yolu_app::pie::PieItem::Command("tool.eraser".into()))
    );
    // パイのキー: F8
    let command = menu.command.unwrap();
    click_chip(
        &mut h,
        Target::Key {
            group: (command, Scope::Everywhere),
            chip: None,
        },
    );
    key(&h, Key::F8, Modifiers::NONE);
    h.run();
    // ウィンドウを閉じて、キャンバスの上で F8 → パイが開き、上の項目で消しゴムに替わる
    close_window(&mut h);
    let c = canvas_rect(&h).center();
    move_to(&h, c);
    h.run();
    key(&h, Key::F8, Modifiers::NONE);
    h.run();
    assert_eq!(h.pie().open.as_ref().map(|o| o.menu.clone()), Some(id));
    key(&h, Key::Num1, Modifiers::NONE);
    h.run();
    assert_eq!(h.tool(), Tool::Eraser);
}

/// パイの編集の行（上・右上…）と、操作の一覧（タブ・探す欄・一覧）は、設定の区分の右端（上の「設定を探す」と同じ端）まで伸びる。長い候補は「…」で詰める。
#[test]
fn the_pie_editor_runs_to_the_right_edge_of_the_search_field_and_a_long_candidate_is_cut_with_dots()
{
    for lang in Lang::ALL {
        let mut h = editor(lang);
        section(&mut h, Section::Pies);
        // 名前の長い別のパイを足しておく（「別のパイ」の候補になる）
        let long = "あ".repeat(90);
        {
            let s = h.app_mut();
            let id = s.pie_mut().add_user(lang);
            let at = s.pie().menus.iter().position(|m| m.id == id).unwrap();
            s.pie_mut().menus[at].name = yolu_app::pie::PieName::User(long.clone());
        }
        h.run();
        let w = window(&h);
        let first = h.pie().menus[0].name(lang);
        let tab = rect_of(&h, &first, |r| w.contains_rect(r));
        click(&mut h, tab.center());
        let slot = h.shortcuts().slot_rects[0];
        click(&mut h, slot.center());
        // 上の探す欄の右端（ウィンドウの右の端から 16）まで
        let edge = w.right() - 16.0;
        let pick = h.shortcuts().pick_rect.expect("項目を探す欄");
        assert!(
            (pick.right() - edge).abs() < 0.5,
            "{lang:?}: 一覧の右端 {} と探す欄の右端 {edge}",
            pick.right()
        );
        for (i, slot) in h.shortcuts().slot_rects.clone().iter().enumerate() {
            // 項目の箱は、右の「空にする」の印（30）を除いた所まで
            assert!(
                (slot.right() - (edge - 30.0)).abs() < 0.5,
                "{lang:?}: 項目 {i} の右端 {}",
                slot.right()
            );
        }
        // 「別のパイ」のタブの候補: 長い名前は「…」で詰め、一覧の中に収まる
        let other = lang.pick("別のパイ", "Another Pie");
        let tab = rect_of(&h, other, |r| w.contains_rect(r) && r.top() > slot.bottom());
        click(&mut h, tab.center());
        // 上の並び（パイの名前の押しボタン）は除き、操作の一覧の中だけを見る
        let texts: Vec<(String, Rect)> = painted_texts(&h)
            .into_iter()
            .filter(|(_, r)| r.top() > pick.bottom())
            .collect();
        let cut = texts
            .iter()
            .find(|(t, _)| t.starts_with("あ") && t.ends_with('…'))
            .unwrap_or_else(|| panic!("{lang:?}: 詰めた候補が無い: {texts:?}"));
        assert!(cut.1.right() <= edge, "{lang:?}: {:?}", cut.1);
        assert!(
            !texts.iter().any(|(t, _)| *t == long),
            "{lang:?}: 詰めずに描いた"
        );
    }
}

/// 描いた文字（文字・矩形）。
fn painted_texts(h: &H) -> Vec<(String, Rect)> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(t) => out.push((
                t.galley.job.text.clone(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &h.output().shapes {
        walk(&s.shape, &mut out);
    }
    out
}

// ───────── 絵 ─────────

#[test]
fn snapshot_the_shortcut_editor_with_a_conflict_in_both_languages() {
    for (lang, name) in [
        (Lang::Ja, "shortcut_editor_ja"),
        (Lang::En, "shortcut_editor_en"),
    ] {
        let mut h = editor(lang);
        section(&mut h, Section::Edit);
        // 編集の Alt+H（隠した印を出す）を H に: 印を隠す H とぶつかる
        let reveal: GroupKey = ("object.reveal", Scope::Edit);
        click_chip(&mut h, key_chip(reveal, 0));
        key(&h, Key::H, Modifiers::NONE);
        h.run();
        // スナップの切り替えの欄は、待っている所を見せる
        click_chip(&mut h, key_chip(("object.snap_toggle", Scope::Edit), 0));
        hover_chip(&mut h, key_chip(reveal, 0));
        let w = window(&h);
        common::shot::region(&mut h, name, w);
    }
}

#[test]
fn snapshot_the_pie_menu_editor_in_both_languages() {
    for (lang, name) in [
        (Lang::Ja, "shortcut_pies_ja"),
        (Lang::En, "shortcut_pies_en"),
    ] {
        let mut h = editor(lang);
        section(&mut h, Section::Pies);
        // 視点のパイを選び、左下の項目を選ぶ所を開く
        let view = h.pie().menus[1].name(lang);
        let w = window(&h);
        let tab = rect_of(&h, &view, |r| w.contains_rect(r));
        click(&mut h, tab.center());
        let slot = h.shortcuts().slot_rects[5];
        click(&mut h, slot.center());
        move_to(&h, w.right_bottom() - egui::vec2(20.0, 80.0));
        h.run();
        common::shot::region(&mut h, name, w);
    }
}

// ───────── ツールのキーの動き方 ─────────

fn mode_box(h: &H, command: &'static str) -> Rect {
    h.shortcuts()
        .mode_boxes
        .iter()
        .find(|(c, _)| *c == command)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| panic!("{command}"))
}

fn click_mode_item(h: &mut H, label: &str) {
    let item = popup_item(h, label);
    click(h, item.center());
}

#[test]
fn tool_rows_have_a_mode_box_whose_list_changes_the_mode_and_the_row_reset_returns_it() {
    let mut h = editor(Lang::Ja);
    section(&mut h, Section::Paint);
    // 箱はツールを選ぶ操作の行だけ
    let boxes = h.shortcuts().mode_boxes.clone();
    assert!(boxes.len() >= 8, "ツールの行が並ぶ: {}", boxes.len());
    for (command, _) in &boxes {
        assert!(command.starts_with("tool."), "{command}");
    }
    assert!(boxes.iter().any(|(c, _)| *c == "tool.eraser"));
    assert_eq!(
        h.keys().tool_mode("tool.eraser"),
        yolu_app::toolkeys::ToolKeyMode::Tap
    );
    // 箱を押すと、動き方の一覧が開く（今の動き方に印）
    let at = mode_box(&h, "tool.eraser").center();
    click(&mut h, at);
    assert!(matches!(
        h.popup().as_ref().map(|p| p.kind),
        Some(PopupKind::ToolKeyMode("tool.eraser"))
    ));
    for label in [
        "押すと切り替え",
        "押している間だけ",
        "短く押すと切り替え・長押しで押している間だけ",
    ] {
        popup_item(&h, label);
    }
    click_mode_item(&mut h, "押している間だけ");
    assert!(h.popup().is_none());
    assert_eq!(
        h.keys().tool_mode("tool.eraser"),
        yolu_app::toolkeys::ToolKeyMode::Hold
    );
    // 行に「既定に戻す」の印が出て、押すと押すと切り替えに戻る
    let row = mode_box(&h, "tool.eraser");
    let w = window(&h);
    let reset = rect_of(&h, "既定に戻す", |r| {
        (r.center().y - row.center().y).abs() < 12.0 && w.contains_rect(r)
    });
    // 実際の割り当てで効く（設定のウィンドウを閉じて、E を押している間だけ消しゴム）
    close_window(&mut h);
    key_event(&h, Key::E, true, false, Modifiers::NONE);
    h.run();
    assert_eq!(h.tool(), Tool::Eraser);
    key_event(&h, Key::E, false, false, Modifiers::NONE);
    h.run();
    assert_eq!(h.tool(), Tool::Brush, "設定が、実際のキーに効く");
    h.act(Action::ShowShortcuts);
    h.run();
    section(&mut h, Section::Paint);
    click(&mut h, reset.center());
    assert_eq!(
        h.keys().tool_mode("tool.eraser"),
        yolu_app::toolkeys::ToolKeyMode::Tap
    );
    assert!(h.keys().is_default());
    // 英語の一覧と、3 つ目の動き方（箱には短い名前）
    h.app_mut().set_language(Lang::En);
    h.run();
    let at = mode_box(&h, "tool.fill").center();
    click(&mut h, at);
    click_mode_item(&mut h, "Tap to Switch, Hold for Temporary");
    assert_eq!(
        h.keys().tool_mode("tool.fill"),
        yolu_app::toolkeys::ToolKeyMode::TapOrHold
    );
    let r = mode_box(&h, "tool.fill");
    assert!(
        h.query_all_by_label_contains("Tap / Hold")
            .any(|n| r.contains(n.rect().center())),
        "箱の短い名前"
    );
}

#[test]
fn the_mode_list_closes_without_a_change_when_the_box_is_pressed_again() {
    let mut h = editor(Lang::Ja);
    section(&mut h, Section::Paint);
    let at = mode_box(&h, "tool.brush").center();
    click(&mut h, at);
    assert!(h.popup().is_some());
    let at = mode_box(&h, "tool.brush").center();
    click(&mut h, at);
    assert!(h.popup().is_none());
    assert!(h.keys().is_default());
}

fn tool_mode_scene(lang: Lang, open: &'static str) -> H {
    use yolu_app::toolkeys::ToolKeyMode;
    let mut h = editor(lang);
    {
        let keys = h.keys_mut();
        keys.set_tool_mode("tool.eraser", ToolKeyMode::Hold);
        keys.set_tool_mode("tool.fill", ToolKeyMode::TapOrHold);
    }
    h.run();
    section(&mut h, Section::Paint);
    let at = mode_box(&h, open).center();
    click(&mut h, at);
    move_to(&h, window(&h).right_bottom() - egui::vec2(8.0, 8.0));
    h.run();
    h
}

#[test]
fn snapshot_the_tool_key_mode_list_in_both_languages() {
    for (lang, name) in [
        (Lang::Ja, "shortcut_tool_mode_ja"),
        (Lang::En, "shortcut_tool_mode_en"),
    ] {
        let mut h = tool_mode_scene(lang, "tool.shape");
        let w = window(&h);
        let popup = h
            .popup()
            .as_ref()
            .map(|p| p.state.rect)
            .unwrap_or(Rect::NOTHING);
        common::shot::region(&mut h, name, w.union(popup));
    }
}

#[test]
fn starting_to_type_in_a_text_field_while_a_tool_key_is_held_counts_as_releasing_it() {
    use yolu_app::toolkeys::ToolKeyMode;
    let mut h = editor(Lang::Ja);
    h.act(Action::ToolKeyMode("tool.eraser", ToolKeyMode::Hold));
    // ウィンドウの外のキャンバスにフォーカスがあるまま E を押す（押している間だけ消しゴム）
    key_event(&h, Key::E, true, false, Modifiers::NONE);
    h.run();
    assert_eq!(h.tool(), Tool::Eraser);
    assert!(h.temp_tool().is_active());
    // 名前で探す欄を押して、文字を打ち始める（押している間の E は、離したものとして扱う）
    let search = h.shortcuts().search_rect.expect("探す欄");
    click(&mut h, search.center());
    h.run();
    assert_eq!(h.tool(), Tool::Brush);
    assert!(!h.temp_tool().is_active());
}
