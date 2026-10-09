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
