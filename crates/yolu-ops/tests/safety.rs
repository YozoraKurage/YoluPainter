//! 安全の試験: 壊す操作の確認・範囲外の値・知らない種類・読むだけのセット・道の決まり・外からの書き換え・版。

mod common;

use common::*;
use serde_json::json;
use yolu_ops::reply::*;
use yolu_ops::{ErrorCode, FileHost, OpHost};

fn code(e: &yolu_ops::OpError) -> ErrorCode {
    e.code
}

#[test]
fn destructive_commands_are_refused_without_confirm_and_change_nothing() {
    let fx = Fixture::new("confirm");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "mask.add", "args": {"layer": "Base"}}),
    );
    let eff = match ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur"}}),
    ) {
        Reply::Edited(e) => e.effect.unwrap(),
        _ => panic!(),
    };
    let before = state(&mut host);
    let steps = undo_count(&mut host);
    for command in [
        json!({"command": "layer.delete", "args": {"layer": "Base"}}),
        json!({"command": "layer.delete", "args": {"layer": "Base", "confirm": false}}),
        json!({"command": "mask.delete", "args": {"layer": "Base"}}),
        json!({"command": "effect.delete", "args": {"layer": "Base", "effect": eff}}),
        json!({"command": "save"}),
    ] {
        let e = err(&mut host, command.clone());
        assert_eq!(code(&e), ErrorCode::ConfirmRequired, "{command}");
        assert!(
            e.message.ja.contains("confirm") && e.message.en.contains("confirm"),
            "{command}"
        );
    }
    assert_eq!(state(&mut host), before);
    assert_eq!(undo_count(&mut host), steps);
    assert!(host.has_unsaved_changes());
}

#[test]
fn save_over_the_opened_file_needs_confirm_and_keeps_the_previous_version() {
    let fx = Fixture::new("save");
    fx.project("a.ylp");
    let original = read(&fx.path("a.ylp"));
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    assert_eq!(
        code(&err(&mut host, json!({"command": "save"}))),
        ErrorCode::ConfirmRequired
    );
    assert_eq!(
        read(&fx.path("a.ylp")),
        original,
        "確認が無ければファイルは変わらない"
    );
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    ) else {
        panic!()
    };
    assert!(saved.written);
    let backups = yolu_io::backups(&fx.path("a.ylp")).unwrap();
    assert_eq!(backups.len(), 1, "前の版は退避のフォルダに残る");
    assert_eq!(read(&backups[0]), original);
    assert!(!host.has_unsaved_changes());
}

#[test]
fn save_as_to_an_existing_file_needs_confirm_and_only_replaces_a_valid_ylp() {
    let fx = Fixture::new("save-as");
    fx.project("a.ylp");
    fx.project("other.ylp");
    let other = read(&fx.path("other.ylp"));
    std::fs::write(fx.path("notes.ylp"), b"not a project").unwrap();
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    let e = err(
        &mut host,
        json!({"command": "save_as", "args": {"path": "other.ylp"}}),
    );
    assert_eq!(code(&e), ErrorCode::ConfirmRequired);
    assert_eq!(read(&fx.path("other.ylp")), other);
    // 有効でないファイルは、confirm があっても置き換えない
    let e = err(
        &mut host,
        json!({"command": "save_as", "args": {"path": "notes.ylp", "confirm": true}}),
    );
    assert_ne!(code(&e), ErrorCode::ConfirmRequired);
    assert_eq!(read(&fx.path("notes.ylp")), b"not a project");
    // 名前が .ylp で終わらない
    let e = err(
        &mut host,
        json!({"command": "save_as", "args": {"path": "x.txt"}}),
    );
    assert_eq!(code(&e), ErrorCode::PathRefused);
    assert!(!fx.path("x.txt").exists());
    // confirm があれば、有効な .ylp は置き換え、前の版は退避に残る
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": "other.ylp", "confirm": true}}),
    ) else {
        panic!()
    };
    assert!(saved.written && saved.backup.is_some());
    assert_eq!(
        read(&yolu_io::backups(&fx.path("other.ylp")).unwrap()[0]),
        other
    );
    // 新しいファイルは確認が要らない
    let Reply::Saved(fresh) = ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": "sub/new.ylp"}}),
    ) else {
        panic!()
    };
    assert!(fresh.written && fresh.backup.is_none());
    assert!(fx.path("sub/new.ylp").exists());
}

#[test]
fn out_of_range_values_unknown_kinds_and_wrong_types_are_refused_without_changing_the_document() {
    let fx = Fixture::new("values");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = state(&mut host);
    let steps = undo_count(&mut host);
    let cases = [
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 1.5}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "opacity": -0.1}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "blend_mode": "Glow"}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "blend_mode": "PassThrough"}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "name": ""}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "name": "ok", "opacity": 9}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Tint", "fill": {"Color": "red"}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "channels": {"Gloss": true}}}),
            ErrorCode::NotFound,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "fill"}}),
            ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "paint", "fill": {"Color": "#fff000"}}}),
            ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "adjustment"}}),
            ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.move", "args": {"layer": "Base", "index": 99}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.move", "args": {"layer": "Base", "parent": "Group", "to_root": true}}),
            ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.move", "args": {"layer": "Base", "parent": "Tint"}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "mask.set", "args": {"layer": "Base", "density": 0.5}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "mask.delete", "args": {"layer": "Base", "confirm": true}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 0}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 257}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 2.5}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": "big"}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"size": 3}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "strength": 2}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "levels", "values": {"input_black": 0.5, "input_white": 0.5}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "edge_wear", "values": {"blend": "xor"}}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "teleport"}}),
            ErrorCode::NotFound,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "tone_curve"}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "hue_saturation"}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "target": "mask", "kind": "blur"}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "channels": ["Gloss"]}}),
            ErrorCode::NotFound,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Tint", "kind": "color_balance", "channels": ["Roughness"]}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "effect.add", "args": {"layer": "Group", "kind": "blur"}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"command": "effect.set", "args": {"layer": "Base", "effect": "0".repeat(31) + "1", "strength": 0.5}}),
            ErrorCode::NotFound,
        ),
        (
            json!({"command": "effect.set", "args": {"layer": "Base", "effect": "xyz"}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "undo", "args": {"steps": 0}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "redo", "args": {"steps": 101}}),
            ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.get", "args": {"layer": "Nothing"}}),
            ErrorCode::NotFound,
        ),
        (
            json!({"command": "set.info", "args": {"set": "Nothing"}}),
            ErrorCode::NotFound,
        ),
    ];
    for (command, expected) in cases {
        let e = err(&mut host, command.clone());
        assert_eq!(code(&e), expected, "{command}: {}", e.message.en);
        assert!(
            !e.message.ja.is_empty() && !e.message.en.is_empty(),
            "{command}"
        );
    }
    assert_eq!(state(&mut host), before, "断った命令は何も変えない");
    assert_eq!(undo_count(&mut host), steps);
}

#[test]
fn effect_set_that_cannot_apply_to_the_current_channels_says_which_ones() {
    let fx = Fixture::new("effect-channels");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let Reply::Edited(e) = ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur"}}),
    ) else {
        panic!()
    };
    let before = state(&mut host);
    let e = err(
        &mut host,
        json!({"command": "effect.set", "args": {"layer": "Base", "effect": e.effect.unwrap(), "kind": "sharpen"}}),
    );
    assert_eq!(code(&e), ErrorCode::InvalidValue);
    assert!(e.message.en.contains("Normal"), "{}", e.message.en);
    assert_eq!(state(&mut host), before);
}

#[test]
fn request_errors_tell_unknown_commands_bad_arguments_and_versions_apart() {
    let fx = Fixture::new("request");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let e = err(&mut host, json!({"command": "layer.explode"}));
    assert_eq!(code(&e), ErrorCode::UnknownCommand);
    assert!(e.data.unwrap()["commands"].as_array().unwrap().len() >= 20);
    let e = err(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Base", "colour": 1}}),
    );
    assert_eq!(code(&e), ErrorCode::InvalidRequest, "知らない欄は断る");
    let e = err(&mut host, json!({"command": "layer.get", "args": {}}));
    assert_eq!(code(&e), ErrorCode::InvalidRequest, "足りない欄");
    let e = err(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": 3}}),
    );
    assert_eq!(code(&e), ErrorCode::InvalidRequest, "型が違う");
    let e = err(&mut host, json!({"args": {}}));
    assert_eq!(code(&e), ErrorCode::InvalidRequest);
    let e = err(&mut host, json!([1, 2]));
    assert_eq!(code(&e), ErrorCode::InvalidRequest);
    for v in [json!(2), json!(0), json!("1"), json!(null)] {
        let e = err(&mut host, json!({"v": v, "command": "doc.info"}));
        assert_eq!(code(&e), ErrorCode::UnsupportedVersion, "v = {v}");
        assert_eq!(e.data.unwrap()["supported"], json!([1]));
    }
    // 版を省くと今の版、args を省くと空
    ok(&mut host, json!({"command": "doc.info"}));
    ok(
        &mut host,
        json!({"v": 1, "command": "doc.info", "args": {}}),
    );
    // 文字列からも同じ入口
    assert_eq!(
        yolu_ops::parse_command_str("{").unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn an_ambiguous_layer_name_is_refused_with_candidates_and_an_id_still_works() {
    let fx = Fixture::new("ambiguous");
    let mut doc = sample_document();
    doc.add_layer("Dup").unwrap();
    doc.add_layer("Dup").unwrap();
    doc.clear_history().unwrap();
    fx.project_with(
        "a.ylp",
        vec![Set::doc(
            "11111111-1111-4111-8111-111111111111",
            "Body",
            doc,
        )],
    );
    let mut host = fx.host("a.ylp");
    let e = err(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Dup"}}),
    );
    assert_eq!(code(&e), ErrorCode::Ambiguous);
    let candidates = e.data.unwrap()["candidates"].as_array().unwrap().clone();
    assert_eq!(candidates.len(), 2);
    let id = candidates[0]["id"].as_str().unwrap();
    ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": id}}),
    );
}

/// ユーザーチャンネルの名前は大文字小文字を区別して重ならなければよいので、「Mask」と「mask」は両方あり得る。
/// 大文字小文字の違う指定は、どちらか決められないので断り、候補を層と同じ鍵（`candidates`）で返す。
#[test]
fn an_ambiguous_channel_name_is_refused_with_candidates_and_an_exact_name_still_works() {
    let fx = Fixture::new("ambiguous-channel");
    let mut doc = sample_document();
    for name in ["Mask", "mask"] {
        doc.add_channel(yolu_core::ChannelInfo {
            name: name.into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: yolu_core::Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    }
    doc.clear_history().unwrap();
    fx.project_with(
        "a.ylp",
        vec![Set::doc(
            "11111111-1111-4111-8111-111111111111",
            "Body",
            doc,
        )],
    );
    let mut host = fx.host("a.ylp");
    let e = err(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Base", "channels": {"MASK": true}}}),
    );
    assert_eq!(code(&e), ErrorCode::Ambiguous);
    let data = e.data.unwrap();
    let names: Vec<&str> = data["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Mask", "mask"]);
    let indexes: Vec<u64> = data["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["index"].as_u64().unwrap())
        .collect();
    assert_eq!(indexes, [6, 7]);
    // 名前どおりの指定と、候補の番号なら通る
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Base", "channels": {"mask": true}}}),
    );
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Base", "channels": {"6": true}}}),
    );
}

#[test]
fn a_read_only_set_is_listed_with_its_reason_and_is_kept_byte_for_byte_by_a_save() {
    let fx = Fixture::new("read-only");
    fx.project_with(
        "a.ylp",
        vec![
            Set::doc(
                "11111111-1111-4111-8111-111111111111",
                "Body",
                sample_document(),
            ),
            Set::unsupported("22222222-2222-4222-8222-222222222222", "Odd"),
        ],
    );
    let entry = |path: &std::path::Path| {
        let p = yolu_io::Project::read(&read(path)).unwrap();
        p.migrated_entries()
            .iter()
            .filter(|(n, _)| n.starts_with("sets/22222222-2222-4222-8222-222222222222/"))
            .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
            .collect::<Vec<_>>()
    };
    let before = entry(&fx.path("a.ylp"));
    assert!(!before.is_empty());
    let mut host = fx.host("a.ylp");
    let Reply::Doc(info) = ok(&mut host, json!({"command": "doc.info"})) else {
        panic!()
    };
    assert_eq!(info.sets[0].state, SetState::Editable);
    assert_eq!(info.sets[1].state, SetState::ReadOnly);
    let reason = info.sets[1].reason.clone().unwrap();
    assert!(!reason.ja.is_empty() && !reason.en.is_empty());
    let Reply::Set(odd) = ok(
        &mut host,
        json!({"command": "set.info", "args": {"set": "Odd"}}),
    ) else {
        panic!()
    };
    assert_eq!(
        (odd.state, odd.width, odd.height),
        (SetState::ReadOnly, 16, 16)
    );
    assert!(odd.layers.is_empty() && odd.reason.is_some());
    for command in [
        json!({"command": "layer.add", "args": {"set": "Odd", "kind": "paint"}}),
        json!({"command": "layer.get", "args": {"set": "Odd", "layer": "Adj"}}),
        json!({"command": "effect.get", "args": {"set": "Odd", "layer": "Adj"}}),
        json!({"command": "history.info", "args": {"set": "Odd"}}),
        json!({"command": "preview", "args": {"set": "Odd"}}),
        json!({"command": "export.channels", "args": {"set": "Odd", "dir": "out"}}),
        json!({"command": "undo", "args": {"set": "Odd"}}),
    ] {
        let e = err(&mut host, command.clone());
        assert_eq!(code(&e), ErrorCode::ReadOnly, "{command}");
        assert!(
            e.message.en.contains("read-only") && e.message.ja.contains("読むだけ"),
            "{command}"
        );
    }
    // 読めるセットを編集して保存しても、読むだけのセットのエントリは同じバイト
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    ok(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    );
    assert_eq!(
        entry(&fx.path("a.ylp")),
        before,
        "読むだけのセットは書き換えない"
    );
}

#[test]
fn paths_that_climb_out_of_the_working_folder_are_refused_by_every_command_that_takes_one() {
    let fx = Fixture::new("paths");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    for command in [
        json!({"command": "doc.open", "args": {"path": "../a.ylp", "confirm": true}}),
        json!({"command": "save_as", "args": {"path": "../escape.ylp"}}),
        json!({"command": "export.channels", "args": {"dir": "../out"}}),
        json!({"command": "export.textures", "args": {"dir": "x/../../out"}}),
        json!({"command": "export.psd", "args": {"path": "../escape.psd"}}),
    ] {
        let e = err(&mut host, command.clone());
        assert_eq!(code(&e), ErrorCode::PathRefused, "{command}");
        assert!(
            e.message.ja.contains("外へ出ます") && e.message.en.contains("leaves"),
            "{command}"
        );
    }
    assert!(!fx.dir.parent().unwrap().join("escape.ylp").exists());
    // 絶対パスは許す
    let abs = fx.path("abs.ylp");
    ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": abs.to_str().unwrap()}}),
    );
    assert!(abs.exists());
}

#[test]
fn opening_another_document_with_unsaved_changes_needs_confirm() {
    let fx = Fixture::new("open");
    fx.project("a.ylp");
    fx.project("b.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    let e = err(
        &mut host,
        json!({"command": "doc.open", "args": {"path": "b.ylp"}}),
    );
    assert_eq!(code(&e), ErrorCode::ConfirmRequired);
    assert_eq!(e.data.unwrap()["reason"], "unsaved_changes");
    assert!(
        host.path().unwrap().ends_with("a.ylp"),
        "断ったら今の文書のまま"
    );
    assert!(host.has_unsaved_changes());
    let Reply::Doc(info) = ok(
        &mut host,
        json!({"command": "doc.open", "args": {"path": "b.ylp", "confirm": true}}),
    ) else {
        panic!()
    };
    assert!(info.path.ends_with("b.ylp") && !info.unsaved);
    // 変更が無ければ確認は要らない
    ok(
        &mut host,
        json!({"command": "doc.open", "args": {"path": "a.ylp"}}),
    );
    // 開けないファイルは、今の文書をそのままにして断る
    std::fs::write(fx.path("bad.ylp"), b"junk").unwrap();
    let e = err(
        &mut host,
        json!({"command": "doc.open", "args": {"path": "bad.ylp"}}),
    );
    assert!(
        matches!(code(&e), ErrorCode::InvalidProject | ErrorCode::Io),
        "{:?}",
        e.code
    );
    assert!(host.path().unwrap().ends_with("a.ylp"));
    let e = err(
        &mut host,
        json!({"command": "doc.open", "args": {"path": "missing.ylp"}}),
    );
    assert_eq!(code(&e), ErrorCode::Io);
}

#[test]
fn without_an_open_document_commands_say_so() {
    let fx = Fixture::new("none");
    let mut host = FileHost::new(fx.policy());
    for command in [
        json!({"command": "doc.info"}),
        json!({"command": "set.info"}),
        json!({"command": "layer.add", "args": {"kind": "group"}}),
        json!({"command": "preview"}),
        json!({"command": "save", "args": {"confirm": true}}),
        json!({"command": "history.info"}),
    ] {
        assert_eq!(
            code(&err(&mut host, command.clone())),
            ErrorCode::NoDocument,
            "{command}"
        );
    }
    // 文書の無い命令は通る
    ok(&mut host, json!({"command": "effect.list_kinds"}));
}

#[test]
fn a_file_changed_from_outside_after_opening_is_not_overwritten() {
    let fx = Fixture::new("conflict");
    fx.project("a.ylp");
    fx.project("b.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    // 開いたあとに、別のプログラムがファイルを書き換えた
    std::fs::copy(fx.path("b.ylp"), fx.path("a.ylp")).unwrap();
    std::fs::write(fx.path("a.ylp"), {
        let mut bytes = read(&fx.path("b.ylp"));
        bytes.extend_from_slice(b"x");
        bytes
    })
    .unwrap();
    let outside = read(&fx.path("a.ylp"));
    let e = err(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    );
    assert_eq!(code(&e), ErrorCode::Conflict);
    assert_eq!(
        read(&fx.path("a.ylp")),
        outside,
        "外で変わったファイルは上書きしない"
    );
    assert!(host.has_unsaved_changes(), "断られたら、編集は失わない");
}

/// 開いたファイルを別の書き方（`..`・リンク・ハードリンク・Windows の大文字小文字）で保存先に指しても、同じファイルとして扱う:
/// 開いたときの印で外からの書き換えを見つけ、書き換えがなければ開いたファイルの場所へ書く。
#[test]
fn save_as_naming_the_opened_file_another_way_is_still_the_same_file() {
    let fx = Fixture::new("conflict-spelling");
    fx.project("a.ylp");
    fx.project("b.ylp");
    std::fs::create_dir(fx.path("sub")).unwrap();
    let mut spellings = vec![fx.path("sub").join("..").join("a.ylp")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&fx.dir, fx.path("dirlink")).unwrap();
        spellings.push(fx.path("dirlink").join("a.ylp"));
    }
    #[cfg(windows)]
    spellings.push(std::path::PathBuf::from(
        fx.path("a.ylp").to_string_lossy().to_uppercase(),
    ));
    let mut host = fx.host("a.ylp");
    let opened = host.path().unwrap().to_path_buf();
    // 外の書き換えが無ければ、別の書き方でも開いたファイルへ書き、開いている場所の表記は変わらない
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": spellings[0].to_str().unwrap(), "confirm": true}}),
    ) else {
        panic!()
    };
    assert!(
        saved.written && saved.backup.is_some(),
        "同じファイルとして、前の版を残して置き換える"
    );
    assert_eq!(saved.path, opened.display().to_string());
    assert_eq!(host.path().unwrap(), opened);
    // 開いたあとに、別のプログラムがファイルを書き換えた（保存は置き換えで新しいファイルになるので、ハードリンクは保存のあとに張る）
    #[cfg(unix)]
    {
        std::fs::hard_link(fx.path("a.ylp"), fx.path("hard.ylp")).unwrap();
        spellings.push(fx.path("hard.ylp"));
    }
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    let mut bytes = read(&fx.path("b.ylp"));
    bytes.extend_from_slice(b"x");
    std::fs::write(fx.path("a.ylp"), &bytes).unwrap();
    for path in &spellings {
        let e = err(
            &mut host,
            json!({"command": "save_as", "args": {"path": path.to_str().unwrap(), "confirm": true}}),
        );
        assert_eq!(code(&e), ErrorCode::Conflict, "{}", path.display());
        assert_eq!(
            read(&fx.path("a.ylp")),
            bytes,
            "{} で、外で変わったファイルを上書きしない",
            path.display()
        );
    }
    assert!(host.has_unsaved_changes(), "断られたら、編集は失わない");
    // 別のファイルは、もうあっても開き直した印で置き換えられる（同じものとは扱わない）
    ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": "b.ylp", "confirm": true}}),
    );
    assert!(host.path().unwrap().ends_with("b.ylp"));
}

#[test]
fn a_budget_that_the_set_does_not_fit_in_is_refused_when_the_set_is_touched() {
    let fx = Fixture::new("budget");
    fx.project("a.ylp");
    let mut host = FileHost::with_config(
        fx.policy(),
        yolu_ops::FileHostConfig {
            source_budget: 64,
            ..yolu_ops::FileHostConfig::default()
        },
    );
    host.open(&fx.path("a.ylp"), true).unwrap();
    // 開くだけ・一覧は通る（セットの文書は触るまで読まない）
    ok(&mut host, json!({"command": "doc.info"}));
    let e = err(&mut host, json!({"command": "set.info"}));
    assert_eq!(code(&e), ErrorCode::Budget);
    // 断っても状態は変わらず、何度でも同じ理由で断る
    let e = err(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    assert_eq!(code(&e), ErrorCode::Budget);
}

/// 途中で panic するホスト（核・ホストの想定外の失敗）。
struct Boom;
impl OpHost for Boom {
    fn policy(&self) -> &yolu_ops::PathPolicy {
        unreachable!("使わない")
    }
    fn doc_info(&mut self) -> Result<DocInfo, yolu_ops::OpError> {
        panic!("boom");
    }
    fn open(&mut self, _: &std::path::Path, _: bool) -> Result<DocInfo, yolu_ops::OpError> {
        unreachable!("使わない")
    }
    fn read_set(
        &mut self,
        _: Option<&str>,
        _: &mut dyn FnMut(&yolu_ops::SetView<'_>) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        panic!("{}", String::from("read boom"));
    }
    fn write_set(
        &mut self,
        _: Option<&str>,
        _: &mut dyn FnMut(
            yolu_ops::doc_ops::SetFacts<'_>,
            &mut yolu_core::Document,
        ) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        unreachable!("使わない")
    }
    fn save(&mut self, _: &yolu_ops::SaveJob) -> Result<Reply, yolu_ops::OpError> {
        unreachable!("使わない")
    }
}

#[test]
fn a_panic_inside_a_host_comes_back_as_an_internal_error() {
    let mut host = Boom;
    let e = err(&mut host, json!({"command": "doc.info"}));
    assert_eq!(code(&e), ErrorCode::Internal);
    assert_eq!(e.data.as_ref().unwrap()["detail"], "boom");
    assert!(e.message.en.contains("doc.info") && !e.message.ja.is_empty());
    let e = err(&mut host, json!({"command": "set.info"}));
    assert_eq!(code(&e), ErrorCode::Internal);
    assert_eq!(e.data.unwrap()["detail"], "read boom");
    // 確認の断りは、ホストに触れる前に返る（panic しない）
    let e = err(
        &mut host,
        json!({"command": "layer.delete", "args": {"layer": "x"}}),
    );
    assert_eq!(code(&e), ErrorCode::ConfirmRequired);
}
