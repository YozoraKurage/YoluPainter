//! キー・マウス・パイの設定（keymap.json）の試験: 文字の往復、置き換えと既定に戻す、ぶつかりと保存の断り、書き出しと読み込みの往復（知らない物の数）、
//! 壊れたファイル、メニューの文字とキーの処理が変更に付いてくる、利用者のパイとそのキー、マウスの組み合わせ。

use egui::{Event, Key, Modifiers, PointerButton};

use super::*;
use crate::mode::EditorMode;
use crate::pie::PieAction;
use crate::state::{Action, Tool};

/// 試験の一時フォルダー（OS の一時フォルダーの下。捨てると消す）。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("yolu-keymap-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> TempDir {
    TempDir::new(tag)
}

fn key(modifiers: Modifiers, key: Key) -> Trigger {
    Trigger::Key { modifiers, key }
}

/// 1 つの押しを、今このスレッドで効いている割り当てで判定する。
fn dispatched(app: &AppState, k: Key, modifiers: Modifiers) -> Vec<Action> {
    let ctx = egui::Context::default();
    let mut got = Vec::new();
    let input = egui::RawInput {
        events: vec![Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        ui.input_mut(|i| got = keymap::dispatch(i, app));
    });
    output.textures_delta.clear();
    got
}

#[test]
fn every_default_input_and_mouse_combination_survives_the_file_text() {
    for row in keymap::bindings() {
        let text = trigger_text(&row.trigger);
        let back = parse_trigger(&text).unwrap_or_else(|| panic!("{text}"));
        assert!(keymap::same_input(&back, &row.trigger), "{text}");
        assert_eq!(back, row.trigger, "{text}");
    }
    for g in GESTURES {
        let c = Combo::of(&g);
        assert_eq!(parse_combo(&combo_text(c)), Some(c));
    }
    for button in [PointerButton::Extra1, PointerButton::Extra2] {
        let c = Combo {
            button,
            alt: true,
            shift: false,
            ctrl: true,
        };
        assert_eq!(parse_combo(&combo_text(c)), Some(c));
    }
    assert_eq!(
        parse_trigger("Ctrl+Shift+Plus"),
        Some(key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus))
    );
    assert_eq!(
        parse_trigger("Control+Tab"),
        Some(key(Modifiers::CTRL, Key::Tab))
    );
    assert_eq!(parse_trigger("text:^"), Some(Trigger::Text("^")));
    assert_eq!(parse_trigger("Hyper+Q"), None);
    assert_eq!(parse_trigger(""), None);
    // 押したキーの割り当て: Windows・Linux の Ctrl（ctrl と command が立つ）は Command の割り当て
    let pressed = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    assert_eq!(
        trigger_of(Key::K, &pressed),
        key(Modifiers::COMMAND, Key::K)
    );
}

#[test]
fn a_changed_key_takes_effect_follows_into_the_menu_and_resetting_forgets_it() {
    let mut app = AppState::new(32, 32);
    let flip = ("view.flip", Scope::Everywhere);
    assert_eq!(
        dispatched(&app, Key::H, Modifiers::NONE),
        vec![Action::FlipView]
    );
    app.keys.set(flip, vec![key(Modifiers::SHIFT, Key::H)]);
    assert!(app.keys.is_changed(flip));
    assert!(dispatched(&app, Key::H, Modifiers::NONE).is_empty());
    assert_eq!(
        dispatched(&app, Key::H, Modifiers::SHIFT),
        vec![Action::FlipView]
    );
    // メニューの文字・ツールチップも、今の表から
    assert_eq!(
        crate::shortcuts::menu_key_in("view.flip", EditorMode::Paint).as_deref(),
        Some("Shift+H")
    );
    // ツールのキーを外すと、ツールの帯の文字も消える
    let brush = ("tool.brush", Scope::Paint);
    app.keys.set(brush, Vec::new());
    assert_eq!(crate::shortcuts::tool_key(Tool::Brush), "");
    assert!(dispatched(&app, Key::B, Modifiers::NONE).is_empty());
    // 既定と同じ並びに戻すと、変えた所から消える
    app.keys.set(flip, vec![key(Modifiers::NONE, Key::H)]);
    assert!(!app.keys.is_changed(flip));
    app.keys.reset(brush);
    assert!(app.keys.is_default());
    assert_eq!(crate::shortcuts::tool_key(Tool::Brush), "B");
    // 選択範囲の帯の消去の札・メニューの H も、変えた割り当てとモードに付いてくる
    let erase = ("selection.erase", Scope::Paint);
    app.keys
        .set(erase, vec![key(Modifiers::SHIFT, Key::Delete)]);
    assert_eq!(
        crate::shortcuts::key_in("selection.erase", EditorMode::Paint).as_deref(),
        Some("Shift+Delete")
    );
    assert_eq!(
        crate::shortcuts::key_in("selection.erase", EditorMode::Edit),
        None
    );
    app.keys.reset(erase);
    let hide = ("object.hide", Scope::Edit);
    app.keys
        .set(hide, vec![key(Modifiers::ALT | Modifiers::SHIFT, Key::H)]);
    assert_eq!(
        crate::shortcuts::key_in("view.flip", EditorMode::Edit).as_deref(),
        Some("H"),
        "編集の段が H を使わなくなれば、編集のモードでも H を添える"
    );
    app.keys.reset(hide);
    assert_eq!(
        crate::shortcuts::key_in("view.flip", EditorMode::Edit),
        None
    );
    // 既定の行が無い操作にも入れられる（視点のパイ）
    let pie = ("view3d.pie", Scope::Everywhere);
    app.keys.set(pie, vec![key(Modifiers::NONE, Key::F9)]);
    assert_eq!(
        dispatched(&app, Key::F9, Modifiers::NONE),
        vec![Action::Pie(PieAction::Open("view".into()))]
    );
}

#[test]
fn a_conflict_is_reported_and_blocks_saving_until_it_is_gone() {
    let dir = temp_dir("conflict");
    let path = dir.join(FILE_NAME);
    let mut app = AppState::new(32, 32);
    let mut pies = app.pie.menus.clone();
    assert_eq!(app.keys.attach(path.clone(), &mut pies, Lang::Ja), None);
    // 消しゴムを B に: ブラシとぶつかる（同じ範囲・同じ場面）
    let eraser = ("tool.eraser", Scope::Paint);
    app.keys.set(eraser, vec![key(Modifiers::NONE, Key::B)]);
    assert_eq!(
        app.keys.conflict(eraser, 0),
        Some(("tool.brush", Scope::Paint))
    );
    assert_eq!(
        app.keys.conflict(("tool.brush", Scope::Paint), 0),
        Some(eraser)
    );
    assert!(app.keys.has_conflicts());
    app.keys_changed();
    assert!(app.keys.unsaved);
    assert!(!path.exists(), "ぶつかりが残っている間は書かない");
    // ぶつかりの無い相手（どこでもの段の H と、編集の段の H）はぶつかりではない（先に取るだけ）
    assert_eq!(app.keys.conflict(("view.flip", Scope::Everywhere), 0), None);
    assert_eq!(
        app.keys.shadowed_by(("view.flip", Scope::Everywhere), 0),
        Some(("object.hide", Scope::Edit))
    );
    // 直すと書く（変えた所だけ）
    app.keys.set(eraser, vec![key(Modifiers::NONE, Key::K)]);
    app.keys_changed();
    assert!(!app.keys.unsaved);
    let text = std::fs::read_to_string(&path).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let keys = v["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1, "{text}");
    assert_eq!(keys[0]["command"], "tool.eraser");
    assert_eq!(keys[0]["keys"][0], "K");
    assert_eq!(v["mouse"].as_array().unwrap().len(), 0);
    assert_eq!(v["pies"].as_array().unwrap().len(), 0);
    // 既定に戻すと、変えた所の無いファイルになる
    app.keys_reset_all();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(v["keys"].as_array().unwrap().is_empty());
}

#[test]
fn export_and_import_round_trip_keys_mouse_and_pies_and_count_unknown_items() {
    let dir = temp_dir("roundtrip");
    let mut app = AppState::new(32, 32);
    // キー・マウス・最初からあるパイの中身・利用者のパイとそのキー
    app.keys.set(
        ("view.flip", Scope::Everywhere),
        vec![key(Modifiers::SHIFT | Modifiers::ALT, Key::H)],
    );
    let orbit = GESTURES
        .iter()
        .find(|g| g.scope == "view3d" && g.operation == keymap::Operation::Orbit)
        .unwrap()
        .index;
    app.keys.set_combo(
        orbit,
        Some(Combo {
            button: PointerButton::Secondary,
            alt: true,
            shift: false,
            ctrl: false,
        }),
    );
    // 横のボタン（戻る・進む）も書いて読める
    let stencil_move = GESTURES
        .iter()
        .find(|g| g.scope == "stencil" && g.operation == keymap::Operation::MoveStencil)
        .unwrap()
        .index;
    let pan3d = GESTURES
        .iter()
        .find(|g| g.scope == "view3d" && g.operation == keymap::Operation::Pan)
        .unwrap()
        .index;
    for (index, button) in [
        (stencil_move, PointerButton::Extra1),
        (pan3d, PointerButton::Extra2),
    ] {
        app.keys.set_combo(
            index,
            Some(Combo {
                button,
                alt: false,
                shift: false,
                ctrl: false,
            }),
        );
    }
    app.pie.menus[1].slots[0] = Some(PieItem::Command("view.fit".into()));
    let id = app.pie.add_user(Lang::Ja);
    let user = app.pie.menu(&id).unwrap().clone();
    let command = user.command.unwrap();
    app.pie.menus.last_mut().unwrap().slots[2] = Some(PieItem::Pie("mode".into()));
    app.keys.set(
        (command, Scope::Everywhere),
        vec![key(Modifiers::COMMAND | Modifiers::SHIFT, Key::P)],
    );
    let file = dir.join("mine.json");
    app.keys_export(&file);
    assert!(file.exists(), "{}", app.message);
    // 別のアプリで読む
    let mut other = AppState::new(32, 32);
    other.keys_import(&file);
    assert_eq!(
        other.keys.to_json(&other.pie.menus),
        app.keys.to_json(&app.pie.menus)
    );
    assert_eq!(other.pie.menus, app.pie.menus);
    assert_eq!(
        dispatched(&other, Key::P, Modifiers::COMMAND | Modifiers::SHIFT),
        vec![Action::Pie(PieAction::Open(id.clone()))]
    );
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Secondary, &Modifiers::ALT, false),
        Some(keymap::Operation::Orbit)
    );
    assert_eq!(
        other.keys.combo(stencil_move).unwrap().button,
        PointerButton::Extra1
    );
    assert_eq!(
        other.keys.combo(pan3d).unwrap().button,
        PointerButton::Extra2
    );
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Extra2, &Modifiers::NONE, false),
        Some(keymap::Operation::Pan)
    );
    // 知らない操作・範囲・キー・組み合わせ・パイの項目は飛ばして数える
    let mut v = app.keys.to_json(&app.pie.menus);
    v["keys"].as_array_mut().unwrap().extend([
        json!({"command": "no.such.command", "scope": "paint", "keys": ["K"]}),
        json!({"command": "tool.brush", "scope": "nowhere", "keys": ["K"]}),
        json!({"command": "tool.brush", "scope": "paint", "keys": ["Hyper+K", "N"]}),
        json!({"command": "clip.copy", "scope": "everywhere", "keys": ["Ctrl+K"]}),
    ]);
    v["mouse"].as_array_mut().unwrap().push(json!({
        "scope": "view3d", "operation": "teleport", "click": false, "index": 0, "combo": "Left"
    }));
    v["pies"].as_array_mut().unwrap().push(json!({
        "id": "u2", "name": "x", "slots": [{"command": "no.such"}, null]
    }));
    std::fs::write(&file, serde_json::to_string(&v).unwrap()).unwrap();
    let mut third = AppState::new(32, 32);
    third.keys_import(&file);
    assert!(third.message.contains("6 個"), "{}", third.message);
    assert_eq!(
        third.keys.triggers(("tool.brush", Scope::Paint)),
        vec![key(Modifiers::NONE, Key::N)],
        "読めたキーは入る"
    );
    assert!(
        !third.keys.is_changed(("clip.copy", Scope::Everywhere)),
        "変えられない操作"
    );
    // 読めないファイルは、何も変えずに理由を出す
    std::fs::write(&file, "{").unwrap();
    let before = third.keys.to_json(&third.pie.menus);
    third.keys_import(&file);
    assert_eq!(third.keys.to_json(&third.pie.menus), before);
    assert!(
        third.message.contains("読み込めません"),
        "{}",
        third.message
    );
}

#[test]
fn a_broken_keymap_file_starts_with_the_defaults_and_is_kept_until_it_is_replaced() {
    let dir = temp_dir("broken");
    let path = dir.join(FILE_NAME);
    std::fs::write(&path, "{ not json").unwrap();
    let mut app = AppState::new(32, 32);
    let mut pies = app.pie.menus.clone();
    let message = app
        .keys
        .attach(path.clone(), &mut pies, Lang::Ja)
        .expect("知らせる");
    assert!(message.contains("既定のキー"), "{message}");
    assert!(app.keys.is_default());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{ not json",
        "消さない"
    );
    // 次に書くとき、別の名前へ移してから書く
    app.keys.set(
        ("view.flip", Scope::Everywhere),
        vec![key(Modifiers::SHIFT, Key::H)],
    );
    app.keys_changed();
    assert_eq!(
        std::fs::read_to_string(dir.join("keymap.broken.json")).unwrap(),
        "{ not json"
    );
    assert!(parse(&std::fs::read_to_string(&path).unwrap(), Lang::Ja).is_ok());
    // 版の違うファイルも既定で始める
    std::fs::write(
        &path,
        r#"{"format":"yolupainter-keymap","version":99,"keys":[]}"#,
    )
    .unwrap();
    let mut app = AppState::new(32, 32);
    let mut pies = app.pie.menus.clone();
    assert!(app.keys.attach(path, &mut pies, Lang::En).is_some());
}

#[test]
fn a_user_pie_opens_with_its_key_and_removing_it_forgets_the_key_and_the_links_to_it() {
    let mut app = AppState::new(32, 32);
    let id = app.pie.add_user(Lang::Ja);
    let command = app.pie.menu(&id).unwrap().command.unwrap();
    assert_eq!(command, format!("pie.{id}"));
    app.pie.menus[0].slots[1] = Some(PieItem::Pie(id.clone()));
    app.keys.set(
        (command, Scope::Everywhere),
        vec![key(Modifiers::NONE, Key::F8)],
    );
    assert_eq!(
        dispatched(&app, Key::F8, Modifiers::NONE),
        vec![Action::Pie(PieAction::Open(id.clone()))]
    );
    // 繰り返しの押しでは開き直さない
    assert!(!keymap::repeats(command));
    app.keys.forget_command(command);
    assert!(app.pie.remove_user(&id));
    assert!(dispatched(&app, Key::F8, Modifiers::NONE).is_empty());
    assert_eq!(
        app.pie.menus[0].slots[1], None,
        "消したパイを開く項目は空に"
    );
    // 最初からあるパイは消せない
    assert!(!app.pie.remove_user("mode"));
}

#[test]
fn a_changed_mouse_combination_takes_effect_can_be_turned_off_and_conflicts_are_found() {
    let mut app = AppState::new(32, 32);
    let find = |scope: &str, op| {
        GESTURES
            .iter()
            .find(|g| g.scope == scope && g.operation == op && g.starts)
            .unwrap()
            .index
    };
    let orbit = find("view3d", keymap::Operation::Orbit);
    let pan = find("view3d", keymap::Operation::Pan);
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Middle, &Modifiers::NONE, false),
        Some(keymap::Operation::Pan)
    );
    // パンを右ボタンへ: 回転とぶつかる
    app.keys.set_combo(
        pan,
        Some(Combo {
            button: PointerButton::Secondary,
            alt: false,
            shift: false,
            ctrl: false,
        }),
    );
    assert_eq!(
        app.keys.combo_conflict(pan),
        Some(ComboConflict::With(orbit))
    );
    assert!(app.keys.has_conflicts());
    // Shift＋右ボタンなら、修飾の多い方が先に当たる（回転に隠れない）
    app.keys.set_combo(
        pan,
        Some(Combo {
            button: PointerButton::Secondary,
            alt: false,
            shift: true,
            ctrl: false,
        }),
    );
    assert!(!app.keys.has_conflicts());
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Secondary, &Modifiers::SHIFT, false),
        Some(keymap::Operation::Pan)
    );
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Secondary, &Modifiers::NONE, false),
        Some(keymap::Operation::Orbit)
    );
    // 外す
    app.keys.set_combo(orbit, None);
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Secondary, &Modifiers::NONE, false),
        None
    );
    app.keys.reset_all();
    assert_eq!(
        keymap::gesture("view3d", PointerButton::Middle, &Modifiers::NONE, false),
        Some(keymap::Operation::Pan)
    );
}

#[test]
fn locked_commands_cannot_be_read_from_a_file() {
    let text = r#"{"format":"yolupainter-keymap","version":1,"keys":[{"command":"clip.paste","scope":"paint","keys":["Ctrl+B"]}]}"#;
    let parsed = parse(text, Lang::Ja).unwrap();
    assert!(parsed.keys.is_empty());
    assert_eq!(parsed.skipped, 1);
}

#[test]
fn a_row_of_only_unreadable_keys_and_a_repeated_row_are_skipped_and_only_an_empty_list_removes_keys(
) {
    let text = r#"{"format":"yolupainter-keymap","version":1,"keys":[
        {"command":"view.flip","scope":"everywhere","keys":["Hyper+H"]},
        {"command":"tool.brush","scope":"paint","keys":[]},
        {"command":"tool.eraser","scope":"paint","keys":["K"]},
        {"command":"tool.eraser","scope":"paint","keys":["J"]}
    ],"mouse":[
        {"scope":"view3d","operation":"pan","click":false,"index":0,"combo":"Shift+Middle"},
        {"scope":"view3d","operation":"pan","click":false,"index":0,"combo":"Alt+Middle"},
        {"scope":"stencil","operation":"snap_stencil_rotation","click":false,"index":0,"combo":"Right"},
        {"scope":"stencil","operation":"snap_stencil_rotation","click":false,"index":1,"combo":"Ctrl+Right"}
    ]}"#;
    let parsed = parse(text, Lang::Ja).unwrap();
    assert!(
        !parsed.keys.contains_key(&("view.flip", Scope::Everywhere)),
        "読めない入力だけの行は既定のまま"
    );
    assert_eq!(
        parsed.keys.get(&("tool.brush", Scope::Paint)),
        Some(&Vec::new()),
        "空の並びは外す"
    );
    assert_eq!(
        parsed.keys.get(&("tool.eraser", Scope::Paint)),
        Some(&vec![key(Modifiers::NONE, Key::K)]),
        "同じ行の 2 つ目は飛ばす"
    );
    // 回転の刻みの行は修飾だけ: 修飾の無い物は読めず、ボタンは回す操作のボタンのまま（2 つ目の「index 1」は無い行）
    let snap = GESTURES
        .iter()
        .find(|g| g.operation == keymap::Operation::SnapStencilRotation)
        .unwrap();
    assert_eq!(parsed.mouse.get(&snap.index), None);
    assert_eq!(parsed.mouse.len(), 1);
    assert_eq!(parsed.skipped, 5);
    let text = r#"{"format":"yolupainter-keymap","version":1,"mouse":[
        {"scope":"stencil","operation":"snap_stencil_rotation","click":false,"index":0,"combo":"Ctrl+Right"}
    ]}"#;
    let parsed = parse(text, Lang::Ja).unwrap();
    assert_eq!(
        parsed.mouse.get(&snap.index),
        Some(&Some(Combo {
            button: snap.button,
            alt: false,
            shift: false,
            ctrl: true,
        }))
    );
    assert!(skipped_message(Lang::Ja, 3).contains("3 個"));
    assert!(skipped_message(Lang::En, 3).contains("combinations"));
}

#[test]
fn a_view_combination_on_the_plain_left_button_or_one_shared_by_the_canvas_and_the_selection_tools_conflicts(
) {
    let mut app = AppState::new(32, 32);
    let find = |scope: &str, op| {
        GESTURES
            .iter()
            .find(|g| g.scope == scope && g.operation == op && g.starts)
            .unwrap()
            .index
    };
    let plain_left = Combo {
        button: PointerButton::Primary,
        alt: false,
        shift: false,
        ctrl: false,
    };
    // 3D のパンを修飾なしの左ボタンに: 描く押しとぶつかる（保存しない）
    let pan = find("view3d", keymap::Operation::Pan);
    app.keys.set_combo(pan, Some(plain_left));
    assert_eq!(app.keys.combo_conflict(pan), Some(ComboConflict::Painting));
    assert!(app.keys.has_conflicts());
    app.keys.reset_all();
    // 2D の回転を Shift＋左に: 選択のツールの「追加」と同じ入力（2D で同時に効く）
    let rotate = find("canvas", keymap::Operation::Rotate);
    let add = find("selection", keymap::Operation::SelectionAdd);
    app.keys.set_combo(
        rotate,
        Some(Combo {
            shift: true,
            ..plain_left
        }),
    );
    // 回転は、追加と同じで、重ねる（Shift＋Ctrl＋左）も隠す
    let intersect = find("selection", keymap::Operation::SelectionIntersect);
    assert!(matches!(
        app.keys.combo_conflict(rotate),
        Some(ComboConflict::With(i)) if i == add || i == intersect
    ));
    assert_eq!(
        app.keys.combo_conflict(add),
        Some(ComboConflict::With(rotate))
    );
    assert_eq!(
        app.keys.combo_conflict(intersect),
        Some(ComboConflict::With(rotate))
    );
    // 選択の行を修飾なしの左ボタンにしても、描く押しとのぶつかりではない（選択のツールの押しそのもの）
    app.keys.reset_all();
    app.keys.set_combo(add, Some(plain_left));
    assert_eq!(app.keys.combo_conflict(add), None);
    // 2D の押しは視点の行を先に引く: 回転（Alt＋左）に修飾を足しただけの選択の行は、回転に隠れる
    app.keys.reset_all();
    app.keys.set_combo(
        add,
        Some(Combo {
            alt: true,
            shift: true,
            ..plain_left
        }),
    );
    assert_eq!(
        app.keys.combo_conflict(add),
        Some(ComboConflict::With(rotate))
    );
    assert_eq!(
        app.keys.combo_conflict(rotate),
        Some(ComboConflict::With(add))
    );
    // スポイト（右ボタン）を Shift＋中ボタンにすると、パン（中ボタン）に隠れる。パンを Shift＋中にしても、スポイトの Shift＋右とはぶつからない
    app.keys.reset_all();
    let pick = find("canvas", keymap::Operation::Pick);
    let shift_middle = Combo {
        button: PointerButton::Middle,
        shift: true,
        ..plain_left
    };
    app.keys.set_combo(pick, Some(shift_middle));
    assert_eq!(
        app.keys.combo_conflict(pick),
        Some(ComboConflict::With(find("canvas", keymap::Operation::Pan)))
    );
    app.keys.reset_all();
    app.keys
        .set_combo(find("canvas", keymap::Operation::Pan), Some(shift_middle));
    assert!(!app.keys.has_conflicts());
    // 既定の表にはぶつかりが無い
    app.keys.reset_all();
    assert!(!app.keys.has_conflicts());
}

// ───────── ツールのキーの動き方（tool_keys） ─────────

use crate::toolkeys::ToolKeyMode;

const ERASER: GroupKey = ("tool.eraser", Scope::Paint);

/// 1 つの押しを、今効いている割り当てで判定する（動き方つき）。`repeat` なら、先に押した（離していない）あとの繰り返しの押し
/// （egui は、押されているキーの 2 度目の押しを繰り返しにする）。
fn fired(app: &AppState, k: Key, repeat: bool) -> Vec<keymap::Fired> {
    let ctx = egui::Context::default();
    let mut got = Vec::new();
    let press = |ctx: &egui::Context, got: &mut Vec<keymap::Fired>, repeat: bool| {
        let input = egui::RawInput {
            events: vec![Event::Key {
                key: k,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.input_mut(|i| *got = keymap::dispatch_fired(&keymap::current(), i, app));
        });
        output.textures_delta.clear();
    };
    if repeat {
        press(&ctx, &mut got, false);
    }
    press(&ctx, &mut got, repeat);
    got
}

#[test]
fn tool_key_modes_are_written_only_when_changed_and_come_back_from_the_file() {
    let dir = temp_dir("toolkeys");
    let path = dir.join(FILE_NAME);
    let mut app = AppState::new(32, 32);
    let mut pies = app.pie.menus.clone();
    assert!(app.keys.attach(path.clone(), &mut pies, Lang::Ja).is_none());
    // 既定（押すと切り替え）のまま: tool_keys を書かず、ファイルも作らない
    assert!(app.keys.is_default());
    assert_eq!(app.keys.tool_mode("tool.eraser"), ToolKeyMode::Tap);
    assert!(app.keys.to_json(&app.pie.menus).get("tool_keys").is_none());
    app.keys_changed();
    assert!(!path.exists());
    // 変えた物だけ書く
    app.keys.set_tool_mode("tool.eraser", ToolKeyMode::Hold);
    app.keys.set_tool_mode("tool.fill", ToolKeyMode::TapOrHold);
    app.keys.set_tool_mode("tool.brush", ToolKeyMode::Tap);
    assert!(!app.keys.is_default());
    assert!(app.keys.is_changed(ERASER));
    assert!(!app.keys.is_changed(("tool.brush", Scope::Paint)));
    app.keys_changed();
    let text = std::fs::read_to_string(&path).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["tool_keys"],
        json!({"tool.eraser": "hold", "tool.fill": "tap_or_hold"})
    );
    assert_eq!(v["version"], 1, "形式の版は上げない");
    // 次の起動で読み直す
    let mut again = AppState::new(32, 32);
    let mut pies = again.pie.menus.clone();
    assert!(again
        .keys
        .attach(path.clone(), &mut pies, Lang::Ja)
        .is_none());
    assert_eq!(again.keys.tool_mode("tool.eraser"), ToolKeyMode::Hold);
    assert_eq!(again.keys.tool_mode("tool.fill"), ToolKeyMode::TapOrHold);
    assert_eq!(again.keys.tool_mode("tool.brush"), ToolKeyMode::Tap);
    // 表の判定にも付いてくる（押しは動き方つきで返る。割り当ては変えていない）
    let hit = fired(&again, Key::E, false);
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].action, Action::SelectTool(Tool::Eraser));
    assert_eq!((hit[0].mode, hit[0].key), (ToolKeyMode::Hold, Some(Key::E)));
    assert_eq!(fired(&again, Key::B, false)[0].mode, ToolKeyMode::Tap);
    // 押すと切り替えに戻せば、変えた所から外れ、tool_keys も消える
    again.keys.set_tool_mode("tool.eraser", ToolKeyMode::Tap);
    again.keys.set_tool_mode("tool.fill", ToolKeyMode::Tap);
    assert!(again.keys.is_default());
    assert!(again
        .keys
        .to_json(&again.pie.menus)
        .get("tool_keys")
        .is_none());
}

#[test]
fn hold_modes_ignore_key_repeat_and_the_tap_mode_keeps_repeating() {
    let mut app = AppState::new(32, 32);
    // 押すと切り替え: 繰り返しの押しも今までどおり実行する
    assert_eq!(fired(&app, Key::E, true).len(), 1);
    app.keys.set_tool_mode("tool.eraser", ToolKeyMode::Hold);
    assert_eq!(fired(&app, Key::E, false).len(), 1);
    assert!(fired(&app, Key::E, true).is_empty(), "繰り返しは何もしない");
    app.keys
        .set_tool_mode("tool.eraser", ToolKeyMode::TapOrHold);
    assert!(fired(&app, Key::E, true).is_empty());
    // ツール以外の操作は、動き方が無い
    assert_eq!(fired(&app, Key::X, true)[0].mode, ToolKeyMode::Tap);
}

#[test]
fn a_tool_row_reset_returns_the_keys_and_the_mode_and_reset_all_returns_every_mode() {
    let mut app = AppState::new(32, 32);
    app.keys.set(ERASER, vec![key(Modifiers::NONE, Key::K)]);
    app.keys.set_tool_mode("tool.eraser", ToolKeyMode::Hold);
    app.keys.set_tool_mode("tool.fill", ToolKeyMode::Hold);
    app.keys.reset(ERASER);
    assert_eq!(app.keys.triggers(ERASER), default_triggers(ERASER));
    assert_eq!(app.keys.tool_mode("tool.eraser"), ToolKeyMode::Tap);
    assert_eq!(
        app.keys.tool_mode("tool.fill"),
        ToolKeyMode::Hold,
        "ほかの行はそのまま"
    );
    app.keys_reset_all();
    assert!(app.keys.is_default());
    assert_eq!(app.keys.tool_mode("tool.fill"), ToolKeyMode::Tap);
    // 動き方だけを変えた行も「変えた行」（既定に戻す印が出る）
    app.keys.set_tool_mode("tool.fill", ToolKeyMode::Hold);
    assert!(app.keys.is_changed(("tool.fill", Scope::Paint)));
    assert_eq!(
        app.keys.changed_groups().count(),
        0,
        "追加の行は増えない（キーは既定のまま）"
    );
}

#[test]
fn export_and_import_carry_the_tool_key_modes_and_import_replaces_them() {
    let dir = temp_dir("toolkeys-roundtrip");
    let file = dir.join("mine.json");
    let mut app = AppState::new(32, 32);
    app.keys.set_tool_mode("tool.eraser", ToolKeyMode::Hold);
    app.keys.set_tool_mode("tool.move", ToolKeyMode::TapOrHold);
    app.keys_export(&file);
    let mut other = AppState::new(32, 32);
    other.keys.set_tool_mode("tool.fill", ToolKeyMode::Hold);
    other.keys_import(&file);
    assert_eq!(other.keys.tool_mode("tool.eraser"), ToolKeyMode::Hold);
    assert_eq!(other.keys.tool_mode("tool.move"), ToolKeyMode::TapOrHold);
    assert_eq!(
        other.keys.tool_mode("tool.fill"),
        ToolKeyMode::Tap,
        "読み込みは今の変更を置き換える"
    );
    assert_eq!(
        other.keys.to_json(&other.pie.menus),
        app.keys.to_json(&app.pie.menus)
    );
    // tool_keys の無いファイル（今までの keymap.json）を読み込むと、動き方は押すと切り替えに戻る
    let old = r#"{"format":"yolupainter-keymap","version":1,"keys":[],"mouse":[],"pies":[]}"#;
    std::fs::write(&file, old).unwrap();
    other.keys_import(&file);
    assert!(other.keys.is_default());
    assert!(!other.message.contains("飛ばしました"), "{}", other.message);
}

#[test]
fn unknown_tool_key_entries_are_skipped_and_counted() {
    let text = r#"{"format":"yolupainter-keymap","version":1,"tool_keys":{
        "tool.fill":"hold",
        "tool.eraser":"toggle",
        "tool.brush":"tap",
        "view.flip":"hold",
        "no.such":"hold",
        "clip.copy":"hold",
        "tool.move":7
    },"future_field":{"x":1}}"#;
    let parsed = parse(text, Lang::Ja).unwrap();
    assert_eq!(
        parsed.tool_modes.into_iter().collect::<Vec<_>>(),
        vec![("tool.fill", ToolKeyMode::Hold)],
        "知っている動き方の、ツールを選ぶ操作だけ。押すと切り替えは変えた所に入れない"
    );
    assert_eq!(
        parsed.skipped, 5,
        "知らない動き方・ツールでない操作・知らない操作・値の形が違う物"
    );
    // 形が違う tool_keys 全体は 1 個として数える
    let text = r#"{"format":"yolupainter-keymap","version":1,"tool_keys":["tool.fill"]}"#;
    let parsed = parse(text, Lang::Ja).unwrap();
    assert!(parsed.tool_modes.is_empty());
    assert_eq!(parsed.skipped, 1);
    // 取り込みの知らせにも数が出る
    let dir = temp_dir("toolkeys-unknown");
    let file = dir.join("x.json");
    std::fs::write(
        &file,
        r#"{"format":"yolupainter-keymap","version":1,"tool_keys":{"tool.fill":"hold","tool.eraser":"x"}}"#,
    )
    .unwrap();
    let mut app = AppState::new(32, 32);
    app.keys_import(&file);
    assert_eq!(app.keys.tool_mode("tool.fill"), ToolKeyMode::Hold);
    assert!(app.message.contains("1 個"), "{}", app.message);
}

#[test]
fn only_tool_commands_take_a_mode_and_locked_or_unknown_ones_are_ignored() {
    let mut app = AppState::new(32, 32);
    app.keys.set_tool_mode("clip.copy", ToolKeyMode::Hold);
    app.keys.set_tool_mode("view.flip", ToolKeyMode::TapOrHold);
    app.keys.set_tool_mode("pie.nothing", ToolKeyMode::Hold);
    assert!(app.keys.is_default());
}
