//! 名前を付けて残した選択範囲（`sets/<ID>/selections.json` と `selection-<SHA-256>.bin`。.ylp の形式 8）と、モデルの今のポーズ
//! （根の `pose.json`）。使う文書だけが形式 8 になること、壊れた項目を理由つきで飛ばすこと、旧形式・知らない形式の扱い。
use yolu_core::{Document, SelectionMask};
use yolu_io::saved_selections::{SavedSelection, SkipReason, INDEX, MAX_NAME_CHARS, MAX_SAVED};
use yolu_io::{Blob, MaterialRef, Project, Selection, SetSpec, WriterInfo, MAX_FORMAT};

const A: &str = "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c01";
const B: &str = "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c02";
const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/selection");

fn writer() -> WriterInfo {
    WriterInfo { app: "試験".into(), version: "1".into(), unity: "なし".into() }
}

fn document() -> Document {
    let mut doc = Document::with_tile_size(70, 50, 16).unwrap();
    doc.add_layer("レイヤー 1").unwrap();
    doc
}

/// 70×50 のセット 2 つのプロジェクト（形式 7）。
fn project() -> Project {
    let native = yolu_io::NativeDocument::from_core(&document()).unwrap();
    let spec = |id: &str, name: &str| SetSpec {
        id: id.into(),
        name: name.into(),
        material: MaterialRef::Material { name: name.into(), asset: None },
        document: Some(native.clone().into()),
        composites: Vec::new(),
    };
    Project::create(writer(), &[spec(A, "Body"), spec(B, "Hair")], A).unwrap()
}

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Selection {
    Selection::from_core(&SelectionMask::rectangle(&document(), x0, y0, x1, y1)).unwrap()
}

fn item(name: &str, selection: Selection) -> SavedSelection {
    SavedSelection { name: name.into(), selection }
}

fn reopened(p: &Project) -> Project {
    Project::read(&p.to_bytes().unwrap()).unwrap()
}

fn format_of(p: &Project) -> i32 {
    p.info().format
}

fn entries_of(p: &Project, id: &str) -> Vec<String> {
    let prefix = format!("sets/{id}/");
    p.original_archive()
        .entries()
        .keys()
        .filter_map(|n| n.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}

#[test]
fn only_a_document_that_uses_them_becomes_format_8_and_it_goes_back_to_7_when_none_is_left() {
    let plain = project();
    assert_eq!(format_of(&plain), 7);
    assert!(plain.saved_selections(A).unwrap().items.is_empty());
    let ylp_json_before = plain.original_archive().entries()["ylp.json"].bytes().unwrap();

    let with = plain
        .with_saved_selections(A, &[item("前髪", rect(2, 2, 20, 20))])
        .unwrap();
    assert_eq!(format_of(&with), 8, "使う文書だけ 8");
    let again = reopened(&with);
    assert_eq!(format_of(&again), 8);
    assert_eq!(MAX_FORMAT, 8);
    // 外側の版は変わらない（名前の決まりは同じ）
    assert_eq!(again.original_archive().manifest_version(), plain.original_archive().manifest_version());
    // ほかのセットの有無に関わらず、1 つでも使えば 8、全部なくせば 7
    let both = again.with_saved_selections(B, &[item("x", rect(0, 0, 5, 5))]).unwrap();
    let one_gone = both.with_saved_selections(A, &[]).unwrap();
    assert_eq!(format_of(&one_gone), 8, "ほかのセットがまだ使っている");
    let none = one_gone.with_saved_selections(B, &[]).unwrap();
    assert_eq!(format_of(&none), 7);
    assert_eq!(reopened(&none).info().format, 7);
    assert_eq!(entries_of(&none, A), entries_of(&plain, A), "使わなくなったエントリを残さない");
    assert_eq!(entries_of(&none, B), entries_of(&plain, B));
    // ylp.json は、形式の数字のほか（書いたアプリなど）変わらない
    let after = none.original_archive().entries()["ylp.json"].bytes().unwrap();
    let parse = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).unwrap();
    assert_eq!(parse(&after), parse(&ylp_json_before));
}

#[test]
fn the_entries_are_the_selection_bin_shape_named_by_their_hash_and_identical_masks_share_one() {
    let golden = std::fs::read(format!("{DIR}/selection-combine.bin")).unwrap();
    let native = yolu_io::NativeDocument::from_core(&document()).unwrap();
    let selection = Selection::read(&golden, &native).unwrap();
    let p = project()
        .with_saved_selections(
            A,
            &[
                item("a", selection.clone()),
                item("b", selection.clone()),
                item("c", rect(1, 1, 9, 9)),
            ],
        )
        .unwrap();
    let entries = entries_of(&p, A);
    let contents: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("selection-") && n.ends_with(".bin"))
        .collect();
    assert_eq!(contents.len(), 2, "同じ中身は 1 つ: {entries:?}");
    assert!(entries.contains(&INDEX.to_owned()));
    // 中身は C# の SelectionBinary と同じバイト列（名前は SHA-256）
    let name = format!("sets/{A}/selection-{}.bin", content_id(&golden));
    assert_eq!(p.original_archive().entries()[&name].bytes().unwrap().as_ref(), golden.as_slice());
    // 索引は並びの順
    let index: serde_json::Value =
        serde_json::from_slice(&p.original_archive().entries()[&format!("sets/{A}/{INDEX}")].bytes().unwrap()).unwrap();
    let names: Vec<&str> = index["selections"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(index["format"], 1);
    // 同じ並びはいつも同じバイト列
    let again = project()
        .with_saved_selections(
            A,
            &[item("a", selection.clone()), item("b", selection), item("c", rect(1, 1, 9, 9))],
        )
        .unwrap();
    assert_eq!(
        again.original_archive().entries()[&format!("sets/{A}/{INDEX}")].bytes().unwrap(),
        p.original_archive().entries()[&format!("sets/{A}/{INDEX}")].bytes().unwrap()
    );
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// 中身の印（SHA-256 の先頭 32 桁）。
fn content_id(bytes: &[u8]) -> String {
    sha256(bytes)[..32].to_owned()
}

#[test]
fn saved_selections_round_trip_with_the_rest_of_the_set_untouched() {
    let base = project();
    let doc_a = base.sets()[0].document.to_bytes().unwrap();
    let doc_b = base.sets()[1].document.to_bytes().unwrap();
    let current = rect(30, 20, 60, 45);
    let base = base.with_selection(A, Some(&current)).unwrap();
    let list = vec![item("髪", rect(2, 2, 20, 20)), item("服 / Clothes", rect(10, 10, 50, 40))];
    let p = reopened(&base.with_saved_selections(A, &list).unwrap());
    let read = p.saved_selections(A).unwrap();
    assert!(read.skipped.is_empty());
    assert_eq!(read.items, list, "名前・並び・中身");
    assert!(p.saved_selections(B).unwrap().items.is_empty(), "ほかのセットは無し");
    assert_eq!(p.sets()[0].selection.as_ref(), Some(&current), "今の選択範囲は別のエントリのまま");
    assert_eq!(p.sets()[0].document.to_bytes().unwrap(), doc_a);
    assert_eq!(p.sets()[1].document.to_bytes().unwrap(), doc_b);
    // 書き直すと前の中身は消え、残した選択範囲と今の選択範囲は互いに触らない
    let rewritten = p.with_saved_selections(A, &[item("髪", rect(3, 3, 8, 8))]).unwrap();
    let names = entries_of(&rewritten, A);
    assert_eq!(names.iter().filter(|n| n.starts_with("selection-")).count(), 1, "{names:?}");
    assert_eq!(rewritten.sets()[0].selection.as_ref(), Some(&current));
    // 取り消した並び（空）はエントリを消す
    let cleared = rewritten.with_saved_selections(A, &[]).unwrap();
    assert!(!entries_of(&cleared, A).iter().any(|n| n == INDEX || n.starts_with("selection-")));
    assert_eq!(cleared.sets()[0].selection.as_ref(), Some(&current));
}

#[test]
fn unrelated_edits_keep_the_saved_selection_entries_byte_for_byte() {
    let p = project().with_saved_selections(A, &[item("a", rect(2, 2, 20, 20))]).unwrap();
    let before: Vec<(String, Vec<u8>)> = p
        .original_archive()
        .entries()
        .iter()
        .filter(|(n, _)| n.contains("/selection-") || n.ends_with(INDEX))
        .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
        .collect();
    assert_eq!(before.len(), 2);
    // 今の選択範囲・モデルの参照・見た目・ポーズを替えても、残した選択範囲は動かない
    let edited = p
        .with_selection(A, Some(&rect(0, 0, 4, 4)))
        .unwrap()
        .with_view_model(Some("models/x.fbx"))
        .unwrap()
        .with_pose(Some(&yolu_io::pose::StoredPose::default()))
        .unwrap();
    for (name, bytes) in &before {
        assert_eq!(edited.original_archive().entries()[name].bytes().unwrap().as_ref(), bytes.as_slice(), "{name}");
    }
    assert_eq!(format_of(&edited), 8);
    // 配布用の写しも残す（除く種類に入れていない）
    let copy = edited.for_distribution(writer(), &yolu_io::Removal::ALL).unwrap();
    assert_eq!(copy.saved_selections(A).unwrap().items.len(), 1);
    assert_eq!(format_of(&copy), 8);
}

#[test]
fn writing_refuses_what_the_rules_forbid_and_changes_nothing() {
    let p = project();
    let ok = rect(0, 0, 5, 5);
    // 数の上限
    let many: Vec<_> = (0..=MAX_SAVED).map(|i| item(&format!("n{i}"), ok.clone())).collect();
    assert!(p.with_saved_selections(A, &many).is_err());
    assert!(p.with_saved_selections(A, &many[..MAX_SAVED]).is_ok(), "上限ちょうどは通る");
    // 名前: 空・前後の空白・制御文字・長すぎる・重なり
    for bad in ["", " a", "a ", "a\nb", &"あ".repeat(MAX_NAME_CHARS + 1)] {
        assert!(p.with_saved_selections(A, &[item(bad, ok.clone())]).is_err(), "{bad:?}");
    }
    assert!(p.with_saved_selections(A, &[item(&"あ".repeat(MAX_NAME_CHARS), ok.clone())]).is_ok());
    assert!(p.with_saved_selections(A, &[item("a", ok.clone()), item("a", ok.clone())]).is_err());
    assert!(p.with_saved_selections(A, &[item("a", ok.clone()), item("A", ok.clone())]).is_ok(), "大文字小文字は別の名前");
    // 文書と大きさが違う
    let other = Document::with_tile_size(40, 40, 16).unwrap();
    let wrong = Selection::from_core(&SelectionMask::all(&other)).unwrap();
    assert!(p.with_saved_selections(A, &[item("w", wrong)]).is_err());
    // 知らないセット
    assert!(p.with_saved_selections("5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c09", &[]).is_err());
    assert!(p.saved_selections("5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c09").is_err());
    assert_eq!(format_of(&p), 7, "断ったら何も変わらない");
}

/// 壊れたエントリを作る: 書いた .ylp の中のエントリを書き換えて、検証つきで読み直す。
fn tampered(p: &Project, edit: impl FnOnce(&mut yolu_io::Files)) -> Project {
    let mut files = p.original_archive().entries().clone();
    edit(&mut files);
    Project::from_entries(files).unwrap()
}

fn index_name(id: &str) -> String {
    format!("sets/{id}/{INDEX}")
}

fn content_name(id: &str, selection: &Selection) -> String {
    format!("sets/{id}/selection-{}.bin", content_id(&selection.to_bytes()))
}

fn reasons(p: &Project, id: &str) -> Vec<(usize, Option<String>, SkipReason)> {
    p.saved_selections(id)
        .unwrap()
        .skipped
        .into_iter()
        .map(|s| (s.index, s.name, s.reason))
        .collect()
}

#[test]
fn a_broken_item_is_skipped_with_its_reason_and_the_others_are_read() {
    let (s1, s2, s3, s4) = (rect(1, 1, 9, 9), rect(10, 10, 20, 20), rect(20, 20, 30, 30), rect(30, 30, 40, 40));
    let p = project()
        .with_saved_selections(A, &[item("one", s1.clone()), item("two", s2.clone()), item("three", s3.clone()), item("four", s4.clone())])
        .unwrap();
    // two の中身が無い・three の中身が壊れている（索引の中身の印と合わない）・four は大きさが違う中身
    let broken = tampered(&p, |files| {
        files.remove(&content_name(A, &s2));
        let name = content_name(A, &s3);
        let mut bytes = files[&name].bytes().unwrap().to_vec();
        *bytes.last_mut().unwrap() ^= 1;
        files.insert(name, Blob::from(bytes));
        let other = Document::with_tile_size(40, 40, 16).unwrap();
        let wrong = Selection::from_core(&SelectionMask::all(&other)).unwrap();
        let wrong_bytes = wrong.to_bytes();
        // 索引の four の中身を、大きさの違う選択範囲に差し替える
        let index_name = index_name(A);
        let mut index: serde_json::Value = serde_json::from_slice(&files[&index_name].bytes().unwrap()).unwrap();
        index["selections"][3]["content"] = serde_json::Value::from(content_id(&wrong_bytes));
        files.insert(index_name, Blob::from(serde_json::to_vec(&index).unwrap()));
        files.insert(format!("sets/{A}/selection-{}.bin", content_id(&wrong_bytes)), Blob::from(wrong_bytes));
    });
    let read = broken.saved_selections(A).unwrap();
    assert_eq!(read.items.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["one"]);
    assert_eq!(read.items[0].selection, s1);
    let why = reasons(&broken, A);
    assert_eq!(why.len(), 3, "{why:?}");
    assert_eq!(why[0], (1, Some("two".into()), SkipReason::MissingContent));
    assert!(matches!(&why[1], (2, Some(n), SkipReason::UnreadableContent(_)) if n == "three"), "{why:?}");
    assert_eq!(why[2], (3, Some("four".into()), SkipReason::WrongSize), "壊れてはいないが文書と大きさが違う");
    // 文書は開ける（ほかのセットは影響を受けない）。飛ばしたエントリはファイルにバイト列のまま残る
    assert_eq!(broken.sets().len(), 2);
    assert!(broken.saved_selections(B).unwrap().skipped.is_empty());
    let kept = broken.with_selection(A, Some(&rect(0, 0, 3, 3))).unwrap();
    assert!(kept.original_archive().entries().contains_key(&content_name(A, &s3)));
    assert_eq!(format_of(&kept), 8);
}

#[test]
fn a_bad_index_or_item_is_told_by_what_is_wrong() {
    let ok = rect(1, 1, 9, 9);
    let p = project().with_saved_selections(A, &[item("one", ok.clone())]).unwrap();
    let hash = content_id(&ok.to_bytes());
    let with_index = |text: &str| {
        tampered(&p, |files| {
            files.insert(index_name(A), Blob::from(text.as_bytes().to_vec()));
        })
    };
    // 索引そのものが読めない: 全部を飛ばす
    for text in [
        "{ not json",
        "[]",
        "{\"format\":1}",
        "{\"format\":1,\"selections\":{}}",
        "{\"format\":2,\"selections\":[]}",
        "{\"selections\":[]}",
    ] {
        let q = with_index(text);
        let why = reasons(&q, A);
        assert!(matches!(why.as_slice(), [(0, None, SkipReason::Index(_))]), "{text}: {why:?}");
        assert!(q.saved_selections(A).unwrap().items.is_empty());
        assert_eq!(q.sets().len(), 2, "{text}: 文書は開ける");
    }
    // 項目: 名前の決まり・中身の印・重なり・上限
    let item_json = |name: &str| format!("{{\"name\":{},\"content\":\"{hash}\"}}", serde_json::to_string(name).unwrap());
    let q = with_index(&format!(
        "{{\"format\":1,\"selections\":[{},{{\"name\":\"x\",\"content\":\"nothash\"}},{{\"content\":\"{hash}\"}},{},{},{}],\"future\":true}}",
        item_json("ok"),
        item_json(" padded "),
        item_json("ok"),
        item_json("fine"),
    ));
    let read = q.saved_selections(A).unwrap();
    assert_eq!(read.items.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["ok", "fine"], "知らないキーは読み飛ばす");
    let why = reasons(&q, A);
    assert_eq!(why.len(), 4, "{why:?}");
    assert!(matches!(&why[0], (1, Some(n), SkipReason::Item(w)) if n == "x" && w == "content"));
    assert!(matches!(&why[1], (2, None, SkipReason::Item(w)) if w == "name"));
    assert!(matches!(&why[2], (3, Some(_), SkipReason::Item(w)) if w == "name"));
    assert_eq!(why[3], (4, Some("ok".into()), SkipReason::DuplicateName));
    // 数の上限を超えた項目は飛ばす
    let many: Vec<String> = (0..MAX_SAVED + 3).map(|i| item_json(&format!("n{i}"))).collect();
    let q = with_index(&format!("{{\"format\":1,\"selections\":[{}]}}", many.join(",")));
    let read = q.saved_selections(A).unwrap();
    assert_eq!(read.items.len(), MAX_SAVED);
    assert_eq!(read.skipped.len(), 3);
    assert!(read.skipped.iter().all(|s| s.reason == SkipReason::TooMany));
}

#[test]
fn older_formats_open_without_saved_selections_and_are_not_raised_unless_they_use_them() {
    for n in 1..=6 {
        let bytes = std::fs::read(format!("{}/tests/fixtures/format{n}.ylp", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let old = Project::read(&bytes).unwrap();
        assert_eq!(format_of(&old), n);
        for set in old.sets() {
            let read = old.saved_selections(&set.id).unwrap();
            assert!(read.items.is_empty() && read.skipped.is_empty(), "形式 {n}");
        }
        // 旧形式のまま書き込むことはできない（先に upgraded で 7 にする）
        let set = &old.sets()[0];
        assert!(old.with_saved_selections(&set.id, &[]).is_err(), "形式 {n}");
        // 開いて保存し直す（7 へ上げる）だけでは、新しいエントリを足さず、8 にもならない
        let upgraded = old.upgraded(writer()).unwrap();
        assert_eq!(format_of(&upgraded), 7, "形式 {n}");
        for set in upgraded.sets() {
            assert!(!entries_of(&upgraded, &set.id).iter().any(|e| e == INDEX || e.starts_with("selection-")), "形式 {n}");
        }
        assert!(!upgraded.original_archive().entries().contains_key("pose.json"), "形式 {n}");
        // 使った文書だけが 8 になる
        let id = upgraded.sets()[0].id.clone();
        let native = upgraded.sets()[0].document.to_native().unwrap();
        let doc = Document::with_tile_size(native.width() as u32, native.height() as u32, native.tile_size() as u32).unwrap();
        let mask = Selection::from_core(&SelectionMask::rectangle(&doc, 0, 0, 3, 3)).unwrap();
        let used = upgraded.with_saved_selections(&id, &[item("a", mask)]).unwrap();
        assert_eq!(format_of(&used), 8, "形式 {n}");
        assert_eq!(reopened(&used).saved_selections(&id).unwrap().items.len(), 1);
    }
}

#[test]
fn a_newer_format_than_this_reader_knows_is_refused_with_the_writer_and_format_8_is_read() {
    let p = project().with_saved_selections(A, &[item("a", rect(1, 1, 9, 9))]).unwrap();
    // 形式 9 は読まない（書いたアプリを添えて断る。黙って捨てない）
    let mut files = p.original_archive().entries().clone();
    let mut info: serde_json::Value = serde_json::from_slice(&files["ylp.json"].bytes().unwrap()).unwrap();
    info["format"] = serde_json::Value::from(MAX_FORMAT + 1);
    files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info).unwrap()));
    match Project::from_entries(files) {
        Err(yolu_io::Error::UnsupportedFormat { format, app, .. }) => {
            assert_eq!(format, MAX_FORMAT + 1);
            assert_eq!(app, "試験");
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    let text = yolu_io::Error::UnsupportedFormat { format: 9, app: "A".into(), version: "1".into() }.to_string();
    assert!(text.contains("形式8"), "{text}");
    assert_eq!(format_of(&reopened(&p)), 8);
}

#[test]
fn a_file_with_the_entries_but_the_old_format_number_still_reads_them_and_settles_on_the_next_write() {
    // 形式の数字だけを 7 にした（他の書き手が作った）ファイルも、エントリがあれば読め、次の書き込みで 8 になる
    let p = project().with_saved_selections(A, &[item("a", rect(1, 1, 9, 9))]).unwrap();
    let q = tampered(&p, |files| {
        let mut info: serde_json::Value = serde_json::from_slice(&files["ylp.json"].bytes().unwrap()).unwrap();
        info["format"] = serde_json::Value::from(7);
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info).unwrap()));
    });
    assert_eq!(format_of(&q), 7);
    assert_eq!(q.saved_selections(A).unwrap().items.len(), 1);
    assert!(q.unknown_entries().is_empty(), "知らないエントリではない");
    let written = q.with_selection(A, None).unwrap();
    assert_eq!(format_of(&written), 8);
}

// ───────── pose.json ─────────

fn bone(path: &[&str], t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> yolu_io::pose::StoredBone {
    yolu_io::pose::StoredBone { path: path.iter().map(|s| (*s).into()).collect(), translation: t, rotation: r, scale: s }
}

fn sample_pose() -> yolu_io::pose::StoredPose {
    // 回転は単位クォータニオン（w は 0 以上）
    let r = {
        let (x, y, z) = (0.1f32, -0.2f32, 0.3f32);
        let w = (1.0 - x * x - y * y - z * z).sqrt();
        [x, y, z, w]
    };
    yolu_io::pose::StoredPose {
        bones: vec![
            bone(&["腰", "背骨", "胸"], [0.0, 0.01, -0.02], r, [1.0, 1.25, 0.8]),
            bone(&["腰", "左足"], [0.1, 0.2, 0.3], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        ],
        shapes: vec![yolu_io::pose::StoredShape { mesh: "Face".into(), name: "Smile".into(), weight: 40.5 }],
    }
}

#[test]
fn the_pose_round_trips_exactly_and_adds_no_format_change() {
    let base = project();
    assert!(base.pose().unwrap().is_none());
    let pose = sample_pose();
    let p = base.with_pose(Some(&pose)).unwrap();
    assert_eq!(format_of(&p), 7, "ポーズは形式を上げない状態のエントリ");
    let again = reopened(&p);
    assert_eq!(again.pose().unwrap().unwrap(), pose, "値は 1 ビットも変わらない");
    assert!(again.unknown_entries().is_empty(), "知らないエントリではない");
    // 同じポーズはいつも同じバイト列
    let bytes = |p: &Project| p.original_archive().entries()["pose.json"].bytes().unwrap();
    assert_eq!(bytes(&p), bytes(&base.with_pose(Some(&pose)).unwrap()));
    // 休みの形（項目なし）も書ける。None はエントリを消す
    let rest = p.with_pose(Some(&yolu_io::pose::StoredPose::default())).unwrap();
    assert!(rest.pose().unwrap().unwrap().is_rest());
    let none = rest.with_pose(None).unwrap();
    assert!(none.pose().unwrap().is_none());
    assert!(!none.original_archive().entries().contains_key("pose.json"));
    // 残した選択範囲と同じファイルで、互いに触らない
    let both = p.with_saved_selections(A, &[item("a", rect(1, 1, 9, 9))]).unwrap();
    assert_eq!(both.pose().unwrap().unwrap(), pose);
    assert_eq!(format_of(&both), 8);
    assert_eq!(both.with_pose(None).unwrap().saved_selections(A).unwrap().items.len(), 1);
}

#[test]
fn a_bad_pose_is_refused_whole_and_the_entry_stays_as_bytes() {
    let good = project().with_pose(Some(&sample_pose())).unwrap();
    let bytes = good.original_archive().entries()["pose.json"].bytes().unwrap();
    let text = std::str::from_utf8(&bytes).unwrap().to_owned();
    let with = |t: &str| {
        let mut files = good.original_archive().entries().clone();
        files.insert("pose.json".into(), Blob::from(t.as_bytes().to_vec()));
        Project::from_entries(files).unwrap()
    };
    for (what, broken) in [
        ("版", text.replace("\"format\":1", "\"format\":2")),
        ("JSON", "{ not json".to_owned()),
        ("配列でない", text.replace("\"bones\":[", "\"bones\":{\"x\":[")),
        ("回転が単位でない", text.replace("0.3,0.9", "0.3,2.9")),
        ("数でない", text.replace("1.25", "\"a\"")),
        ("道が空", text.replace("[\"腰\",\"左足\"]", "[]")),
        ("道の名前に制御文字", text.replace("左足", "左\\u0001足")),
        ("骨の重なり", text.replace("[\"腰\",\"左足\"]", "[\"腰\",\"背骨\",\"胸\"]")),
        ("範囲外", text.replace("0.1,0.2,0.3", "1e30,0,0")),
    ] {
        let q = with(&broken);
        assert!(q.pose().is_err(), "{what}: {broken}");
        // 読めなくても文書は開け、エントリはバイト列のまま残る
        assert_eq!(q.original_archive().entries()["pose.json"].bytes().unwrap().as_ref(), broken.as_bytes(), "{what}");
        let kept = q.with_selection(A, None).unwrap();
        assert_eq!(kept.original_archive().entries()["pose.json"].bytes().unwrap().as_ref(), broken.as_bytes(), "{what}");
    }
    // 書くときも同じ決まりで断る（数・名前・有限）
    let mut bad = sample_pose();
    bad.bones[0].rotation = [0.0, 0.0, 0.0, 2.0];
    assert!(project().with_pose(Some(&bad)).is_err());
    let mut bad = sample_pose();
    bad.shapes[0].weight = f32::NAN;
    assert!(project().with_pose(Some(&bad)).is_err());
    let mut bad = sample_pose();
    bad.bones.push(bad.bones[0].clone());
    assert!(project().with_pose(Some(&bad)).is_err());
    let mut bad = sample_pose();
    bad.bones = (0..yolu_io::pose::MAX_BONES + 1).map(|i| bone(&[&format!("b{i}")], [0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3])).collect();
    assert!(project().with_pose(Some(&bad)).is_err());
}
