//! 命令ごとの試験: FileHost で .ylp を開いて命令を当て、1 回の取り消しで戻り、保存して開き直すと結果が残る。

mod common;

use common::*;
use serde_json::{json, Value};
use yolu_ops::reply::*;
use yolu_ops::{FileHost, OpHost};

/// 命令を当てて、(1) 取り消しの 1 段が 1 つ増える (2) 取り消し 1 回で元の状態に戻り、やり直しで戻る
/// (3) 保存して開き直すと、当てた後の状態が残る、を確かめる。当てた後の返事を返す。
fn check_one_command(fx: &Fixture, host: &mut FileHost, command: Value) -> Reply {
    let before_state = state(host);
    let before_undo = undo_count(host);
    let reply = ok(host, command.clone());
    let after_state = state(host);
    assert_ne!(before_state, after_state, "{command} が何も変えていない");
    assert_eq!(
        undo_count(host),
        before_undo + 1,
        "{command} は取り消しの 1 段になる"
    );
    // 取り消し 1 回で元へ、やり直し 1 回でまた当たる
    let Reply::Undone(undone) = ok(host, json!({"command": "undo"})) else {
        panic!()
    };
    assert_eq!(undone.steps, 1);
    assert_eq!(state(host), before_state, "{command} の取り消し");
    let Reply::Undone(redone) = ok(host, json!({"command": "redo"})) else {
        panic!()
    };
    assert_eq!(redone.steps, 1);
    assert_eq!(state(host), after_state, "{command} のやり直し");
    // 保存して開き直す
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let file = format!(
        "saved-{}.ylp",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    ok(
        host,
        json!({"command": "save_as", "args": {"path": file, "confirm": true}}),
    );
    let mut reopened = FileHost::new(fx.policy());
    reopened.open(&fx.path(&file), true).unwrap();
    assert_eq!(
        state(&mut reopened),
        after_state,
        "{command} を保存して開き直した状態"
    );
    reply
}

fn edited(reply: Reply) -> Edited {
    match reply {
        Reply::Edited(e) => e,
        other => panic!("{other:?}"),
    }
}

#[test]
fn layer_add_makes_every_kind_in_one_undo_step() {
    let fx = Fixture::new("layer-add");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "New paint"}}),
    ));
    assert!(e.layer.is_some() && e.effect.is_none() && !e.unchanged);
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Rough", "fill": {"roughness": "#808080", "Metallic": "#ff000080"}}}),
    ));
    let Reply::Layer(fill) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": e.layer.unwrap()}}),
    ) else {
        panic!()
    };
    assert_eq!(fill.summary.kind, LayerKindName::Fill);
    let color = |name: &str| fill.channels.iter().find(|c| c.channel == name).unwrap();
    assert_eq!(color("Roughness").fill.as_deref(), Some("#808080"));
    assert_eq!(color("Metallic").fill.as_deref(), Some("#ff000080"));
    assert!(color("Metallic").enabled && !color("Color").enabled);
    edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group", "name": "Folder"}}),
    ));
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "adjustment", "name": "Curve", "adjustment": {"kind": "levels", "values": {"gamma": 2.0}}}}),
    ));
    let Reply::Layer(adj) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": e.layer.unwrap()}}),
    ) else {
        panic!()
    };
    let a = adj.adjustment.unwrap();
    assert_eq!(a.kind, "levels");
    assert_eq!(a.values["gamma"], yolu_ops::Value::Number(2.0));
    // 足す位置: Base の上（同じ階層）
    let base = layer_id(&mut host, "Base");
    let e = edited(ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "Above base", "above": base}}),
    ));
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    let names: Vec<&str> = info.layers.iter().map(|l| l.name.as_str()).collect();
    let at = names.iter().position(|n| *n == "Above base").unwrap();
    assert_eq!(names[at + 1], "Base", "{names:?}");
    assert_eq!(e.layer.as_deref(), Some(info.layers[at].id.as_str()));
}

#[test]
fn layer_delete_removes_a_group_with_its_contents() {
    let fx = Fixture::new("layer-delete");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let group = layer_id(&mut host, "Group");
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.delete", "args": {"layer": group, "confirm": true}}),
    );
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert!(
        info.layers
            .iter()
            .all(|l| l.name != "Group" && l.name != "Inner"),
        "{:?}",
        info.layers
    );
}

#[test]
fn layer_move_reorders_enters_a_group_and_leaves_it() {
    let fx = Fixture::new("layer-move");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    // 一番下へ（兄弟の中で 0）
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.move", "args": {"layer": "Tint", "index": 0}}),
    );
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert_eq!(info.layers.last().unwrap().name, "Tint");
    // グループの中へ
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.move", "args": {"layer": "Base", "parent": "Group"}}),
    );
    let Reply::Layer(base) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    assert_eq!(base.summary.depth, 1);
    assert!(base.summary.parent.is_some());
    // 外へ
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.move", "args": {"layer": "Base", "to_root": true, "index": 0}}),
    );
    let Reply::Layer(base) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    assert_eq!(base.summary.depth, 0);
}

#[test]
fn layer_set_changes_many_attributes_as_one_undo_step() {
    let fx = Fixture::new("layer-set");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {
            "layer": "Base", "name": "Renamed", "visible": false, "opacity": 0.5, "blend_mode": "multiply",
            "clipping": true, "locks": {"pixels": true, "position": true},
            "channels": {"Roughness": true}
        }}),
    );
    let Reply::Layer(l) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Renamed"}}),
    ) else {
        panic!()
    };
    assert!(!l.summary.visible && l.summary.clipping);
    assert_eq!(l.summary.opacity, 0.5);
    assert_eq!(l.summary.blend_mode, "Multiply");
    assert!(l.locks.pixels && l.locks.position && !l.locks.transparency && !l.locks.all);
    assert!(
        l.channels
            .iter()
            .find(|c| c.channel == "Roughness")
            .unwrap()
            .enabled
    );
    // 塗りつぶしの値を変える・消す
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Tint", "fill": {"Color": "#102030"}}}),
    );
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Tint", "fill": {"Roughness": "#404040"}}}),
    );
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Tint", "fill": {"Roughness": null}}}),
    );
    // 調整の値: 同じ種類は欄ごとに、別の種類は作り直す
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "adjustment", "name": "Adj", "adjustment": {"kind": "levels", "values": {"gamma": 2.0, "output_white": 0.8}}}}),
    );
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Adj", "adjustment": {"kind": "levels", "values": {"gamma": 1.5}}}}),
    );
    let Reply::Layer(adj) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Adj"}}),
    ) else {
        panic!()
    };
    let values = adj.adjustment.unwrap().values;
    assert_eq!(values["gamma"], yolu_ops::Value::Number(1.5));
    assert_eq!(
        values["output_white"],
        yolu_ops::Value::Number(0.8),
        "渡さなかった欄は変わらない"
    );
    // 色のチャンネルだけに使える調整へ変えるには、使えないチャンネルを無効にする（言わなければ、どのチャンネルかを言って断る）
    let e = err(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Adj", "adjustment": {"kind": "hue_saturation", "values": {"hue": 30}}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::Unsupported);
    assert!(e.message.en.contains("Roughness"), "{}", e.message.en);
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {
            "layer": "Adj", "adjustment": {"kind": "hue_saturation", "values": {"hue": 30}},
            "channels": {"Roughness": false, "Metallic": false, "Height": false, "Normal": false}
        }}),
    );
    // 逆向き: 色のチャンネルだけに使える調整レイヤーを、どのチャンネルにも使える調整へ替えながら、使えなかったチャンネルを有効にする
    // （核は有効にするチャンネルに今の調整が使えるかを段ごとに見るので、調整を替えたあとで有効にする順で 1 段になる）
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {
            "layer": "Adj", "adjustment": {"kind": "levels", "values": {"gamma": 1.25}}, "channels": {"Roughness": true}
        }}),
    );
    let Reply::Layer(adj) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Adj"}}),
    ) else {
        panic!()
    };
    assert_eq!(adj.adjustment.as_ref().unwrap().kind, "levels");
    let enabled = |name: &str| {
        adj.channels
            .iter()
            .find(|c| c.channel == name)
            .unwrap()
            .enabled
    };
    assert!(enabled("Roughness") && enabled("Color") && !enabled("Metallic"));
}

#[test]
fn mask_commands_add_change_and_delete() {
    let fx = Fixture::new("mask");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "mask.add", "args": {"layer": "Base"}}),
    );
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "mask.set", "args": {"layer": "Base", "enabled": false, "inverted": true, "density": 0.25}}),
    );
    let Reply::Layer(l) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    let m = l.mask.unwrap();
    assert!(!m.enabled && m.inverted && m.density == 0.25);
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "mask.delete", "args": {"layer": "Base", "confirm": true}}),
    );
    let Reply::Layer(l) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    assert!(l.mask.is_none());
}

#[test]
fn effect_commands_add_get_change_and_delete() {
    let fx = Fixture::new("effect");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    // 足す（内容のスタック）
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 3}, "strength": 0.5}}),
    ));
    let blur = e.effect.clone().unwrap();
    let Reply::Effects(list) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    assert_eq!(list.effects.len(), 1);
    assert_eq!(list.effects[0].kind, "blur");
    assert_eq!(
        list.effects[0].values["radius"],
        yolu_ops::Value::Number(3.0)
    );
    assert_eq!(list.effects[0].strength, 0.5);
    assert_eq!(
        list.effects[0].target,
        yolu_ops::command::EffectTarget::Content
    );
    // 種類の違う 2 つ目を先頭へ
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "levels", "values": {"gamma": 2.2}, "index": 0, "channels": ["Color"]}}),
    ));
    let levels = e.effect.unwrap();
    let Reply::Effects(list) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    assert_eq!(list.effects[0].id, levels);
    assert_eq!(list.effects[0].channels, vec!["Color".to_string()]);
    // 値だけ変える（渡さない欄は残る）
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.set", "args": {"layer": "Base", "effect": blur, "values": {"radius": 9}}}),
    );
    let Reply::Effects(one) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base", "effect": blur}}),
    ) else {
        panic!()
    };
    assert_eq!(
        one.effects[0].values["radius"],
        yolu_ops::Value::Number(9.0)
    );
    assert_eq!(one.effects[0].strength, 0.5);
    // 強さ・有効・位置をまとめて 1 段で
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.set", "args": {"layer": "Base", "effect": blur, "strength": 0.25, "enabled": false, "index": 0}}),
    );
    // 種類を変える
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.set", "args": {"layer": "Base", "effect": blur, "kind": "sharpen", "values": {"amount": 2}, "channels": ["Color", "Roughness"]}}),
    );
    let Reply::Effects(one) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base", "effect": blur}}),
    ) else {
        panic!()
    };
    assert_eq!(one.effects[0].kind, "sharpen");
    // マスクのスタック
    ok(
        &mut host,
        json!({"command": "mask.add", "args": {"layer": "Base"}}),
    );
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "target": "mask", "kind": "blur", "values": {"radius": 2}}}),
    ));
    let Reply::Effects(all) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base"}}),
    ) else {
        panic!()
    };
    let on_mask: Vec<_> = all
        .effects
        .iter()
        .filter(|x| x.target == yolu_ops::command::EffectTarget::Mask)
        .collect();
    assert_eq!(on_mask.len(), 1);
    assert_eq!(Some(&on_mask[0].id), e.effect.as_ref());
    // 消す
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.delete", "args": {"layer": "Base", "effect": levels, "confirm": true}}),
    );
    // Generator は設定が文書に残り、画面なしでは効かない効果として知らせる
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "edge_wear", "values": {"low": 0.1, "blend": "screen"}}}),
    ));
    let Reply::Effects(one) = ok(
        &mut host,
        json!({"command": "effect.get", "args": {"layer": "Base", "effect": e.effect.unwrap()}}),
    ) else {
        panic!()
    };
    assert_eq!(
        one.effects[0].values["blend"],
        yolu_ops::Value::Text("screen".into())
    );
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert!(!info.inactive_effects.is_empty());
}

#[test]
fn undo_and_redo_take_several_steps_and_history_info_counts_them() {
    let fx = Fixture::new("history");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let Reply::History(h) = ok(&mut host, json!({"command": "history.info"})) else {
        panic!()
    };
    assert_eq!(
        (h.undo_count, h.redo_count, h.can_undo, h.can_redo),
        (0, 0, false, false)
    );
    let start = state(&mut host);
    for name in ["A", "B", "C"] {
        ok(
            &mut host,
            json!({"command": "layer.add", "args": {"kind": "paint", "name": name}}),
        );
    }
    let Reply::History(h) = ok(&mut host, json!({"command": "history.info"})) else {
        panic!()
    };
    assert_eq!((h.undo_count, h.redo_count), (3, 0));
    let Reply::Undone(u) = ok(&mut host, json!({"command": "undo", "args": {"steps": 5}})) else {
        panic!()
    };
    assert_eq!(
        (u.steps, u.can_undo, u.can_redo),
        (3, false, true),
        "履歴が尽きたら、取り消せた数だけ"
    );
    assert_eq!(state(&mut host), start);
    let Reply::Undone(r) = ok(&mut host, json!({"command": "redo", "args": {"steps": 2}})) else {
        panic!()
    };
    assert_eq!(r.steps, 2);
    let Reply::History(h) = ok(&mut host, json!({"command": "history.info"})) else {
        panic!()
    };
    assert_eq!((h.undo_count, h.redo_count), (2, 1));
}

#[test]
fn doc_info_and_set_info_describe_the_file() {
    let fx = Fixture::new("info");
    fx.project_with(
        "a.ylp",
        vec![
            Set::doc(
                "11111111-1111-4111-8111-111111111111",
                "Body",
                sample_document(),
            ),
            Set::doc("22222222-2222-4222-8222-222222222222", "Face", {
                let mut d = yolu_core::Document::new(32, 32).unwrap();
                d.add_layer("Only").unwrap();
                d.clear_history().unwrap();
                d
            }),
        ],
    );
    let mut host = fx.host("a.ylp");
    let Reply::Doc(info) = ok(&mut host, json!({"command": "doc.info"})) else {
        panic!()
    };
    assert_eq!(info.format, 7);
    assert_eq!(info.sets.len(), 2);
    assert_eq!(
        (
            info.sets[0].width,
            info.sets[0].height,
            info.sets[0].layer_count
        ),
        (64, 48, 4)
    );
    assert_eq!(info.current_set, "11111111-1111-4111-8111-111111111111");
    assert!(!info.unsaved);
    // セットの指定は ID でも名前でもよい
    let Reply::Set(face) = ok(
        &mut host,
        json!({"command": "set.info", "args": {"set": "Face"}}),
    ) else {
        panic!()
    };
    assert_eq!((face.width, face.height, face.layers.len()), (32, 32, 1));
    let Reply::Set(same) = ok(
        &mut host,
        json!({"command": "set.info", "args": {"set": "22222222-2222-4222-8222-222222222222"}}),
    ) else {
        panic!()
    };
    assert_eq!(face, same);
    // レイヤーの並びは上から下で、グループが中身の前に来る
    let Reply::Set(body) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    let names: Vec<&str> = body.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Group", "Inner", "Tint", "Base"]);
    assert_eq!(body.layers[1].depth, 1);
    assert_eq!(
        body.layers[1].parent.as_deref(),
        Some(body.layers[0].id.as_str())
    );
    assert_eq!(body.channels.len(), 6);
    // 編集すると文書は「保存していない」
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group", "set": "Face"}}),
    );
    let Reply::Doc(info) = ok(&mut host, json!({"command": "doc.info"})) else {
        panic!()
    };
    assert!(info.unsaved);
    assert!(info.sets[1].unsaved && !info.sets[0].unsaved);
}

#[test]
fn saving_rewrites_only_the_edited_set_and_keeps_the_others_byte_for_byte() {
    let fx = Fixture::new("save-partial");
    fx.project_with(
        "a.ylp",
        vec![
            Set::doc(
                "11111111-1111-4111-8111-111111111111",
                "Body",
                sample_document(),
            ),
            Set::doc(
                "22222222-2222-4222-8222-222222222222",
                "Face",
                sample_document(),
            ),
        ],
    );
    let entries = |path: &std::path::Path| -> std::collections::BTreeMap<String, Vec<u8>> {
        let project = yolu_io::Project::read(&read(path)).unwrap();
        project
            .migrated_entries()
            .iter()
            .map(|(name, blob)| (name.clone(), blob.bytes().unwrap().to_vec()))
            .collect()
    };
    let before = entries(&fx.path("a.ylp"));
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"set": "Face", "layer": "Base", "opacity": 0.5}}),
    );
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    ) else {
        panic!()
    };
    assert!(saved.written);
    assert_eq!(
        saved.sets_written,
        vec!["22222222-2222-4222-8222-222222222222".to_string()]
    );
    assert!(saved.backup.is_some(), "前の版は退避に残る");
    let after = entries(&fx.path("a.ylp"));
    let body = "sets/11111111-1111-4111-8111-111111111111/";
    let face = "sets/22222222-2222-4222-8222-222222222222/";
    for (name, bytes) in &before {
        if name.starts_with(body) {
            assert_eq!(
                after.get(name),
                Some(bytes),
                "編集していないセットの {name} は同じバイト"
            );
        }
    }
    let changed: Vec<_> = before
        .iter()
        .filter(|(n, b)| n.starts_with(face) && after.get(*n) != Some(*b))
        .map(|(n, _)| n.clone())
        .collect();
    assert!(
        changed.iter().any(|n| n.ends_with("document.utpaint")),
        "{changed:?}"
    );
    // 何も編集していないときは書かない
    let Reply::Saved(again) = ok(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    ) else {
        panic!()
    };
    assert!(!again.written);
}

/// 形式 7 未満のファイル（yolu-io の試験の台にある形式 6）は、保存で今の形式（7）へ上がる。編集したセットの結果が開き直しても残り、
/// 編集していないセットの中身（エントリのバイト列）は変わらず、前の版は退避に残る。
#[test]
fn saving_an_old_format_file_moves_it_to_format_7_and_keeps_the_untouched_sets() {
    let fx = Fixture::new("upgrade");
    let old_bytes = read(std::path::Path::new(&format!(
        "{}/../yolu-io/tests/fixtures/format6.ylp",
        env!("CARGO_MANIFEST_DIR")
    )));
    std::fs::write(fx.path("old.ylp"), &old_bytes).unwrap();
    let entries = |bytes: &[u8]| -> std::collections::BTreeMap<String, Vec<u8>> {
        yolu_io::Project::read(bytes)
            .unwrap()
            .migrated_entries()
            .iter()
            .map(|(name, blob)| (name.clone(), blob.bytes().unwrap().to_vec()))
            .collect()
    };
    let before = entries(&old_bytes);
    let mut host = fx.host("old.ylp");
    let Reply::Doc(info) = ok(&mut host, json!({"command": "doc.info"})) else {
        panic!()
    };
    assert!(info.format < 7, "試験の台は旧形式: {}", info.format);
    let editable: Vec<&SetSummary> = info
        .sets
        .iter()
        .filter(|s| s.state == SetState::Editable)
        .collect();
    assert!(
        editable.len() >= 2,
        "編集できるセットが 2 つ以上要る: {:?}",
        info.sets
    );
    let (edited, untouched) = (
        editable[0].id.clone(),
        editable[1..]
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>(),
    );
    let layers_before = {
        let Reply::Set(set) = ok(
            &mut host,
            json!({"command": "set.info", "args": {"set": edited}}),
        ) else {
            panic!()
        };
        set.layers.len()
    };
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"set": edited, "kind": "group", "name": "Added by ops"}}),
    );
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save", "args": {"confirm": true}}),
    ) else {
        panic!()
    };
    // (1) 上げたことが返事に出る
    assert!(saved.written);
    assert_eq!((saved.upgraded_from, saved.format), (Some(info.format), 7));
    assert_eq!(saved.sets_written, vec![edited.clone()]);
    let backup = saved.backup.expect("前の版は退避に残る");
    assert_eq!(
        read(std::path::Path::new(&backup)),
        old_bytes,
        "退避は元のファイルのバイト列"
    );
    // (2) 開き直すと、形式 7 で編集の結果が残っている
    let mut reopened = FileHost::new(fx.policy());
    let again = reopened.open(&fx.path("old.ylp"), true).unwrap();
    assert_eq!(again.format, 7);
    assert_eq!(
        again.sets.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        info.sets.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
    );
    let Reply::Set(set) = ok(
        &mut reopened,
        json!({"command": "set.info", "args": {"set": edited}}),
    ) else {
        panic!()
    };
    assert_eq!(set.layers.len(), layers_before + 1);
    assert!(set.layers.iter().any(|l| l.name == "Added by ops"));
    // (3) 編集していないセットは、エントリのバイト列が上げる前（形式 7 への読み替え）と同じ
    let after = entries(&read(&fx.path("old.ylp")));
    for id in &untouched {
        let prefix = format!("sets/{id}/");
        let names: Vec<&String> = before.keys().filter(|n| n.starts_with(&prefix)).collect();
        assert!(!names.is_empty(), "{prefix}");
        for name in names {
            assert_eq!(
                after.get(name),
                before.get(name),
                "編集していないセットの {name} は同じバイト"
            );
        }
    }
    assert_ne!(
        after.get(&format!("sets/{edited}/document.utpaint")),
        before.get(&format!("sets/{edited}/document.utpaint")),
        "編集したセットの正本は書き直される"
    );
}

/// スタンドアロンだけの効果・調整を使ったセットを保存すると、0.4.x までの Unity 版が開けない版になる。そのことを知らせる（使っていなければ知らせない）。
#[test]
fn saving_tells_when_a_set_uses_something_only_this_editor_has() {
    let fx = Fixture::new("unity-note");
    fx.project("a.ylp");
    let Reply::Kinds(kinds) = ({
        let mut host = fx.host("a.ylp");
        ok(&mut host, json!({"command": "effect.list_kinds"}))
    }) else {
        panic!()
    };
    let mut n = 0;
    for kind in kinds.kinds.iter().filter(|k| k.addable) {
        let mut host = fx.host("a.ylp");
        if kind.used_as.contains(&KindUse::EffectStack) {
            // 色のチャンネルに置けない種類（スカラーとマスクだけのフィルター）は Roughness へ
            let color = json!({"command": "effect.add", "args": {"layer": "Base", "kind": kind.id, "channels": ["Color"]}});
            if run(&mut host, color).is_err() {
                ok(
                    &mut host,
                    json!({"command": "effect.add", "args": {"layer": "Base", "kind": kind.id, "channels": ["Roughness"]}}),
                );
            }
        } else {
            ok(
                &mut host,
                json!({"command": "layer.add", "args": {"kind": "adjustment", "adjustment": {"kind": kind.id}}}),
            );
        }
        n += 1;
        let Reply::Saved(saved) = ok(
            &mut host,
            json!({"command": "save_as", "args": {"path": format!("u{n}.ylp")}}),
        ) else {
            panic!()
        };
        let noted = saved
            .notes
            .iter()
            .any(|t| t.en.contains("Unity version up to 0.4.x"));
        assert_eq!(noted, kind.rust_only, "{}: {:?}", kind.id, saved.notes);
        if noted {
            let note = saved
                .notes
                .iter()
                .find(|t| t.en.contains("Unity version up to 0.4.x"))
                .unwrap();
            assert!(
                note.ja.contains("Unity 版") && note.en.contains("version"),
                "{note:?}"
            );
        }
    }
    assert!(n >= 15);
    // 何も足さない保存には出ない
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "group"}}),
    );
    let Reply::Saved(saved) = ok(
        &mut host,
        json!({"command": "save_as", "args": {"path": "plain.ylp"}}),
    ) else {
        panic!()
    };
    assert!(saved.notes.is_empty(), "{:?}", saved.notes);
}

/// 0.5.0 のフィルター（種類 70〜79）と Generator（模様・ライト・マスクの組み立て・アイランドごとのばらつき）が `effect.list_kinds` に出て、
/// `effect.add` で値つきで足せ、`effect.get` で同じ値が読める。
#[test]
fn the_new_filters_are_listed_and_added_with_values() {
    let fx = Fixture::new("new-filters");
    fx.project("a.ylp");
    let Reply::Kinds(kinds) = ({
        let mut host = fx.host("a.ylp");
        ok(&mut host, json!({"command": "effect.list_kinds"}))
    }) else {
        panic!()
    };
    let cases = [
        (
            "histogram_scan",
            "Roughness",
            json!({"position": 0.25, "contrast": 0.75}),
        ),
        (
            "histogram_range",
            "Height",
            json!({"range": 0.4, "position": 0.6}),
        ),
        (
            "slope_blur",
            "Color",
            json!({"intensity": 12.5, "samples": 6, "mode": "max", "scale": 20.0, "seed": 4}),
        ),
        (
            "directional_blur",
            "Color",
            json!({"angle": 45.0, "distance": 10.0}),
        ),
        (
            "warp",
            "Roughness",
            json!({"intensity": 8.0, "scale": 16.0, "seed": -3}),
        ),
        (
            "morphology",
            "Metallic",
            json!({"mode": "erode", "radius": 5}),
        ),
        (
            "edge_detect",
            "Height",
            json!({"width": 2, "threshold": 0.2}),
        ),
        ("high_pass", "Color", json!({"radius": 12})),
        ("median", "Emission", json!({"radius": 2})),
        (
            "glow",
            "Color",
            json!({"threshold": 0.5, "radius": 24, "intensity": 2.0}),
        ),
        (
            "pattern",
            "Color",
            json!({"shape": "dots", "scale": 12.0, "width": 0.4, "softness": 0.2}),
        ),
        (
            "light",
            "Roughness",
            json!({"azimuth": 120.0, "elevation": 30.0, "ambient": 0.1}),
        ),
        (
            "mask_builder",
            "Height",
            json!({"curvature_weight": 0.5, "ambient_occlusion_weight": 1.0, "combine": "max", "thickness_invert": true}),
        ),
        (
            "uv_island_variation",
            "Roughness",
            json!({"seed": -5, "min": 0.25, "max": 0.25, "softness": 0.5}),
        ),
    ];
    let mut host = fx.host("a.ylp");
    for (id, channel, values) in &cases {
        let kind = kinds
            .kinds
            .iter()
            .find(|k| k.id == *id)
            .unwrap_or_else(|| panic!("{id} が一覧に無い"));
        assert!(kind.addable && kind.rust_only, "{id}");
        assert_eq!(
            kind.generator,
            ["pattern", "light", "mask_builder", "uv_island_variation"].contains(id),
            "{id}"
        );
        assert!(kind.used_as.contains(&KindUse::EffectStack), "{id}");
        assert!(
            !kind.title.ja.is_empty() && kind.title.en != *id,
            "{id}: 名前"
        );
        for p in &kind.params {
            assert!(!p.description.en.is_empty(), "{id}.{}: 説明", p.name);
        }
        let e = edited(ok(
            &mut host,
            json!({"command": "effect.add", "args": {"layer": "Base", "kind": id, "values": values, "channels": [channel]}}),
        ));
        let effect = e.effect.unwrap();
        let Reply::Effects(list) = ok(
            &mut host,
            json!({"command": "effect.get", "args": {"layer": "Base"}}),
        ) else {
            panic!()
        };
        let got = list.effects.iter().find(|x| x.id == effect).unwrap();
        assert_eq!(got.kind, *id);
        for (name, v) in values.as_object().unwrap() {
            let want = match v {
                serde_json::Value::String(s) => yolu_ops::Value::Text(s.clone()),
                serde_json::Value::Bool(b) => yolu_ops::Value::Bool(*b),
                other => yolu_ops::Value::Number(other.as_f64().unwrap()),
            };
            assert_eq!(got.values[name], want, "{id}.{name}");
        }
    }
    // 置けないチャンネルは理由つきで断る（グローは Roughness に、太らせる・細らせるは Color に置けない）
    err(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "glow", "channels": ["Roughness"]}}),
    );
    err(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "morphology", "channels": ["Color"]}}),
    );
}

#[test]
fn layer_set_places_and_removes_a_point_gradient_in_one_undo_step() {
    let fx = Fixture::new("layer-points");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Dots", "fill": {"Color": "#808080", "Roughness": "#404040"}}}),
    );
    // UV の空間の 2 点（色と不透明度）。保存して開き直しても残る
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Dots", "points": {"Color": {"space": "uv", "spread": 0.25, "points": [
            {"position": [0.1, 0.5], "color": "#ff0000"},
            {"position": [0.9, 0.5], "color": "#0000ff80"}
        ]}}}}),
    );
    let Reply::Layer(layer) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": "Dots"}}),
    ) else {
        panic!()
    };
    let color = layer
        .channels
        .iter()
        .find(|c| c.channel == "Color")
        .unwrap();
    let points = color.points.as_ref().expect("点のグラデーション");
    assert_eq!(points.space, yolu_ops::command::PointSpaceName::Uv);
    assert_eq!(points.spread, Some(0.25));
    assert_eq!(points.points.len(), 2);
    assert_eq!(points.points[1].position, vec![0.9, 0.5]);
    assert_eq!(points.points[1].color, "#0000ff80");
    // 外す
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Dots", "points": {"Color": null}}}),
    );
    // 断る: 位置の数が空間と合わない・点が無い・広がりの範囲の外
    for bad in [
        json!({"Color": {"space": "model", "points": [{"position": [0.1, 0.5], "color": "#ff0000"}]}}),
        json!({"Color": {"space": "uv", "points": []}}),
        json!({"Roughness": {"space": "uv", "spread": 2.0, "points": [{"position": [0.1, 0.5], "color": "#ff0000"}]}}),
    ] {
        let e = err(
            &mut host,
            json!({"command": "layer.set", "args": {"layer": "Dots", "points": bad}}),
        );
        assert_eq!(e.code, yolu_ops::ErrorCode::InvalidValue, "{e:?}");
    }
}

/// 試験に使うフォントのファイル（アプリに同梱の BIZ UDPGothic を作業のフォルダへ写す）。
fn font_file(fx: &Fixture, name: &str) -> String {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");
    std::fs::copy(source, fx.path(name)).unwrap();
    name.to_owned()
}

#[test]
fn text_layers_are_added_and_redrawn_in_one_undo_step_with_a_font_file() {
    let fx = Fixture::new("text-layer");
    fx.project("a.ylp");
    let file = font_file(&fx, "font.ttf");
    let mut host = fx.host("a.ylp");
    let e = edited(check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "name": "Label",
            "text": {"content": "文字 Text", "font_file": file, "size": 14, "color": "#204080", "x": 2, "y": 40}}}),
    ));
    let id = e.layer.unwrap();
    let get = |host: &mut FileHost| {
        let Reply::Layer(l) = ok(host, json!({"command": "layer.get", "args": {"layer": id}}))
        else {
            panic!()
        };
        l
    };
    let layer = get(&mut host);
    assert_eq!(layer.summary.kind, LayerKindName::Text);
    let text = layer.text.unwrap();
    assert_eq!(text.content, "文字 Text");
    assert_eq!(text.size, 14.0);
    assert_eq!(text.color, "#204080");
    assert!(text.font.is_none());
    assert!(text.font_file.unwrap().ends_with("font.ttf"));
    // 値を変えると描き直す（フォントはレイヤーが覚えたファイルから読む）
    check_one_command(
        &fx,
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"content": "二行\nの文", "align": "center", "rotation": 15}}}),
    );
    let text = get(&mut host).text.unwrap();
    assert_eq!(text.content, "二行\nの文");
    assert_eq!(text.align, yolu_ops::command::TextAlignName::Center);
    assert_eq!(text.size, 14.0, "書かなかった値はそのまま");
}

#[test]
fn text_layers_refuse_unavailable_fonts_and_misplaced_values_without_changes() {
    let fx = Fixture::new("text-refusals");
    fx.project("a.ylp");
    let file = font_file(&fx, "font.ttf");
    // OS のフォントは空の一覧（この PC に入っているフォントで結果が変わらないように）
    let mut host = WithFonts(fx.host("a.ylp"), Default::default());
    let before = state(&mut host);
    // 画面なしのホストは同梱のフォントを持たない（理由で、起動中のアプリなら描けることが分かる）
    let e = err(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "x"}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::Unsupported);
    assert!(e.message.ja.contains("起動中のアプリ") && e.message.en.contains("running app"));
    for (command, code) in [
        (
            json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "x", "font": "nope"}}}),
            yolu_ops::ErrorCode::NotFound,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "x", "font": "biz-udpgothic", "font_file": file}}}),
            yolu_ops::ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "text", "text": {"font_file": file}}}),
            yolu_ops::ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "paint", "text": {"content": "x", "font_file": file}}}),
            yolu_ops::ErrorCode::InvalidRequest,
        ),
        (
            json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "x", "font_file": file, "size": 0}}}),
            yolu_ops::ErrorCode::InvalidValue,
        ),
        (
            json!({"command": "layer.set", "args": {"layer": "Base", "text": {"content": "x"}}}),
            yolu_ops::ErrorCode::Unsupported,
        ),
    ] {
        let e = err(&mut host, command.clone());
        assert_eq!(e.code, code, "{command}");
    }
    assert_eq!(state(&mut host), before);
    // レイヤーが覚えたフォントのファイルの中身が変わったら、見つからない扱いで描き直さない
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "name": "Label", "text": {"content": "x", "font_file": file}}}),
    );
    let added = state(&mut host);
    std::fs::write(fx.path("font.ttf"), b"changed").unwrap();
    let e = err(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"content": "y"}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::NotFound);
    assert_eq!(state(&mut host), added);
}

/// 同梱のフォントを持つホスト（起動中のアプリの代わり）。ほかは画面なしのホストへ渡す。
struct WithBundled(FileHost);
impl OpHost for WithBundled {
    fn policy(&self) -> &yolu_ops::PathPolicy {
        self.0.policy()
    }
    fn doc_info(&mut self) -> Result<DocInfo, yolu_ops::OpError> {
        self.0.doc_info()
    }
    fn open(&mut self, p: &std::path::Path, c: bool) -> Result<DocInfo, yolu_ops::OpError> {
        self.0.open(p, c)
    }
    fn read_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(&yolu_ops::SetView<'_>) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        self.0.read_set(set, f)
    }
    fn write_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(
            yolu_ops::doc_ops::SetFacts<'_>,
            &mut yolu_core::Document,
        ) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        self.0.write_set(set, f)
    }
    fn save(&mut self, job: &yolu_ops::SaveJob) -> Result<Reply, yolu_ops::OpError> {
        self.0.save(job)
    }
    fn bundled_font(&self, name: &str) -> Option<std::sync::Arc<[u8]>> {
        (name == "biz-udpgothic").then(|| {
            std::sync::Arc::from(
                &include_bytes!("../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf")[..],
            )
        })
    }
}

#[test]
fn a_host_with_bundled_fonts_draws_the_default_font() {
    let fx = Fixture::new("text-bundled");
    fx.project("a.ylp");
    let mut host = WithBundled(fx.host("a.ylp"));
    let e = edited(ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "既定のフォント"}}}),
    ));
    let Reply::Layer(l) = ok(
        &mut host,
        json!({"command": "layer.get", "args": {"layer": e.layer.unwrap()}}),
    ) else {
        panic!()
    };
    let text = l.text.unwrap();
    assert_eq!(text.font.as_deref(), Some("biz-udpgothic"));
    assert_eq!(
        (text.x, text.y),
        (0.0, 48.0),
        "既定の基準の点はキャンバスの左上"
    );
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Text", "text": {"size": 20}}}),
    );
    // 画面なしのホストは、同梱のフォントの文字を描き直せない
    let e = err(
        &mut host.0,
        json!({"command": "layer.set", "args": {"layer": "Text", "text": {"size": 30}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::Unsupported);
}

/// OS のフォントの一覧を決めたフォルダのものにしたホスト。ほかは画面なしのホストへ渡す。
struct WithFonts(FileHost, std::sync::Arc<yolu_io::fonts::SystemFonts>);
impl OpHost for WithFonts {
    fn policy(&self) -> &yolu_ops::PathPolicy {
        self.0.policy()
    }
    fn doc_info(&mut self) -> Result<DocInfo, yolu_ops::OpError> {
        self.0.doc_info()
    }
    fn open(&mut self, p: &std::path::Path, c: bool) -> Result<DocInfo, yolu_ops::OpError> {
        self.0.open(p, c)
    }
    fn read_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(&yolu_ops::SetView<'_>) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        self.0.read_set(set, f)
    }
    fn write_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(
            yolu_ops::doc_ops::SetFacts<'_>,
            &mut yolu_core::Document,
        ) -> Result<Reply, yolu_ops::OpError>,
    ) -> Result<Reply, yolu_ops::OpError> {
        self.0.write_set(set, f)
    }
    fn save(&mut self, job: &yolu_ops::SaveJob) -> Result<Reply, yolu_ops::OpError> {
        self.0.save(job)
    }
    fn system_fonts(
        &mut self,
    ) -> Result<std::sync::Arc<yolu_io::fonts::SystemFonts>, yolu_ops::OpError> {
        Ok(self.1.clone())
    }
}

/// 返事の知らせ（`edited` の `notes`）の日本語の文。
fn note_texts(reply: Reply) -> Vec<String> {
    edited(reply).notes.into_iter().map(|n| n.ja).collect()
}

#[test]
fn a_font_found_with_other_contents_is_redrawn_with_a_note_in_the_reply() {
    let fx = Fixture::new("text-different");
    fx.project("a.ylp");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-app/assets/fonts");
    let dir = fx.path("fonts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(source.join("BIZUDPGothic-Bold.ttf"), dir.join("b.ttf")).unwrap();
    let list = || std::sync::Arc::new(yolu_io::fonts::SystemFonts::from_dirs(&[&dir]));
    let mut host = WithFonts(fx.host("a.ylp"), list());
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "name": "Label", "text": {"content": "OS", "font": "BIZUDPGothic-Bold"}}}),
    );
    // 同じ中身が見つかれば、知らせは無い
    let same = ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"size": 20}}}),
    );
    assert!(note_texts(same).is_empty());
    // 同じ名前・同じ道のファイルが別の中身（末尾に 1 バイト足した、読めるフォント）になった
    let mut bytes = std::fs::read(dir.join("b.ttf")).unwrap();
    bytes.push(0);
    std::fs::write(dir.join("b.ttf"), &bytes).unwrap();
    host.1 = list();
    // 色だけを変える命令でも、描き直すのは見つけたフォントなので、黙って入れ替えない
    let different = ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"color": "#336699"}}}),
    );
    let notes = note_texts(different.clone());
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("フォントが違います"), "{}", notes[0]);
    assert!(notes[0].contains("BIZUDPGothic-Bold"), "{}", notes[0]);
    let Reply::Edited(e) = different else {
        panic!()
    };
    assert!(e.notes[0].en.contains("differs"), "{}", e.notes[0].en);
    // 描き直した値は見つけたフォントの印になり、次からは同じ中身
    let again = ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"size": 22}}}),
    );
    assert!(note_texts(again).is_empty());
    // フォントを指定した命令は、利用者が選んだものなので知らせない
    let chosen = ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"font": "BIZUDPGothic-Bold"}}}),
    );
    assert!(note_texts(chosen).is_empty());
}

#[test]
fn installed_fonts_are_chosen_by_name_and_found_again_after_moving() {
    let fx = Fixture::new("text-installed");
    fx.project("a.ylp");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-app/assets/fonts");
    let (first, second) = (fx.path("fonts"), fx.path("moved"));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::copy(source.join("BIZUDPGothic-Regular.ttf"), first.join("r.ttf")).unwrap();
    std::fs::copy(source.join("BIZUDPGothic-Bold.ttf"), first.join("b.ttf")).unwrap();
    let list =
        |dir: &std::path::Path| std::sync::Arc::new(yolu_io::fonts::SystemFonts::from_dirs(&[dir]));
    let mut host = WithFonts(fx.host("a.ylp"), list(&first));
    let get = |host: &mut WithFonts| {
        let Reply::Layer(l) = ok(
            host,
            json!({"command": "layer.get", "args": {"layer": "Label"}}),
        ) else {
            panic!()
        };
        l.text.unwrap()
    };
    // ファミリー名は太さ 400 の斜体でないスタイル（画面なしのホストでも描ける）
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "name": "Label", "text": {"content": "OS", "font": "BIZ UDPGothic"}}}),
    );
    let text = get(&mut host);
    assert_eq!(text.font, None);
    assert_eq!(text.font_family.as_deref(), Some("BIZ UDPGothic"));
    assert_eq!(
        text.font_postscript.as_deref(),
        Some("BIZUDPGothic-Regular")
    );
    assert!(text.font_file.unwrap().ends_with("r.ttf"));
    // PostScript 名で 1 つのスタイル
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"font": "BIZUDPGothic-Bold"}}}),
    );
    assert_eq!(
        get(&mut host).font_postscript.as_deref(),
        Some("BIZUDPGothic-Bold")
    );
    // フォントが別のフォルダへ移っても、名前で同じ中身を見つけて描き直す（値の道は見つけた所）
    std::fs::rename(first.join("b.ttf"), second.join("bold.ttf")).unwrap();
    host.1 = list(&second);
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"size": 20}}}),
    );
    let text = get(&mut host);
    assert_eq!(text.size, 20.0);
    assert!(text.font_file.unwrap().ends_with("bold.ttf"));
    // どこにも無ければ、何も変えずに断る
    std::fs::remove_file(second.join("bold.ttf")).unwrap();
    host.1 = list(&second);
    let before = state(&mut host);
    let e = err(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Label", "text": {"size": 30}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::NotFound);
    assert!(
        e.message.ja.contains("フォントが見つかりません"),
        "{}",
        e.message.ja
    );
    assert_eq!(state(&mut host), before);
    // 一覧に無い名前
    let e = err(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "text", "text": {"content": "x", "font": "No Such Family"}}}),
    );
    assert_eq!(e.code, yolu_ops::ErrorCode::NotFound);
}

/// アイランドごとのばらつきはモデルの UV アイランドを読む: 画面なしのホストにはモデルが無いので、足せるが入力のまま通し、`inactive_effects` に
/// モデルが無いことを言う（`needs_baked_maps` の印も立つ）。
#[test]
fn the_uv_island_variation_needs_a_model_and_is_listed_as_inactive_without_one() {
    let fx = Fixture::new("island-variation");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let Reply::Kinds(kinds) = ok(&mut host, json!({"command": "effect.list_kinds"})) else {
        panic!()
    };
    let kind = kinds
        .kinds
        .iter()
        .find(|k| k.id == "uv_island_variation")
        .unwrap();
    assert!(kind.generator && kind.needs_baked_maps && kind.addable && kind.rust_only);
    assert_eq!(kind.title.en, "UV Island Variation");
    let names: Vec<&str> = kind.params.iter().map(|p| p.name.as_str()).collect();
    for name in [
        "seed", "min", "max", "low", "high", "softness", "invert", "blend",
    ] {
        assert!(names.contains(&name), "{name}: {names:?}");
    }
    // 最小が最大を超えるのは断る
    err(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "uv_island_variation", "values": {"min": 0.8, "max": 0.2}}}),
    );
    edited(ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "uv_island_variation", "values": {"seed": 9}}}),
    ));
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert_eq!(info.inactive_effects.len(), 1);
    let note = &info.inactive_effects[0];
    assert!(
        note.en.contains("UV island variation") && note.en.contains("no model"),
        "{note:?}"
    );
    assert!(note.ja.contains("モデルがありません"), "{note:?}");
}
