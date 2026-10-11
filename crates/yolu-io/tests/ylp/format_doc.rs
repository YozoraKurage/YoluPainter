//! 形式の仕様（`docs/YLP_FORMAT.md`）が、書き手と読み手に追いついているか。エントリ・版・正本の欄・JSON のキーを足して仕様を直し忘れると落ちる。
//! 仕様の中の節への道（`#…`）が、見出しに当たっているかも見る（見出しを変えると黙って切れるため）。
use yolu_core::{look::MaterialLook, Document, SelectionMask};
use yolu_io::{
    entry_form, mesh_map, pose, saved_selections, MaterialRef, NativeDocument, Project, Selection,
    SetSpec, WriterInfo, ADJUST_VERSION, ANTI_ALIAS_VERSION, BAKE_PRIORITY_VERSION,
    EFFECTS_VERSION, MAX_FORMAT, MAX_NATIVE_VERSION, MIXING_VERSION, PATHS_VERSION,
    POINT_GRADIENT_VERSION, PROCEDURAL_VERSION, RESOURCE_ENTRIES, ROOT_ENTRIES, RULERS_VERSION,
    SAVED_SELECTIONS_FORMAT, SEAMS_VERSION, SET_ENTRIES, SPLIT_VERSION, TEXT_VERSION,
    UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION,
};

const SPEC: &str = include_str!("../../../../docs/YLP_FORMAT.md");
const DECISIONS: &str = include_str!("../../../../docs/YLP_DECISIONS.md");

/// 節を絞らず、仕様の全体と照らす印。
const WHOLE: &str = "";

/// 見出し `heading` の節（次の同じ深さか浅い見出しまで）。
fn section(heading: &str) -> &'static str {
    let level = heading.bytes().take_while(|b| *b == b'#').count();
    let start = SPEC
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("仕様に節「{heading}」がありません"));
    let body = &SPEC[start + heading.len() + 2..];
    let end = body
        .match_indices("\n#")
        .find(|(i, _)| body[i + 1..].bytes().take_while(|b| *b == b'#').count() <= level)
        .map_or(body.len(), |(i, _)| i);
    &body[..end]
}

/// 表の行のうち、最初の欄が `name` で始まるもの。
fn row<'a>(text: &'a str, name: &str) -> &'a str {
    let key = format!("| {name}");
    text.lines()
        .find(|l| l.starts_with(&key))
        .unwrap_or_else(|| panic!("表に行「{name}」がありません"))
}

/// 表の行の欄（前後の空白を除く）。
fn cells(row: &str) -> Vec<&str> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// 文書の見出しの道（GitHub と同じ規則: 英数字・日本語の字・`-`・`_` だけを残し、空白は `-` にする。
/// 同じ見出しは後ろに `-1`…が付く）。コードの囲みの中は見ない。
fn heading_anchors(doc: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let Some(rest) = line.strip_prefix('#').filter(|_| !fenced) else {
            continue;
        };
        let Some(text) = rest.trim_start_matches('#').strip_prefix(' ') else {
            continue;
        };
        let base: String = text
            .trim()
            .chars()
            .filter(|c| *c != '`')
            .flat_map(char::to_lowercase)
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        let mut anchor = base.clone();
        let mut n = 0;
        while out.contains(&anchor) {
            n += 1;
            anchor = format!("{base}-{n}");
        }
        out.push(anchor);
    }
    out
}

/// 文書の中のリンク `](…)` の行き先（コードの囲みの中は見ない）。
fn link_targets(doc: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let mut rest = line;
        while let Some(i) = rest.find("](").filter(|_| !fenced) {
            let after = &rest[i + 2..];
            let Some(end) = after.find(')') else { break };
            out.push(&after[..end]);
            rest = &after[end + 1..];
        }
    }
    out
}

/// `name` が英数字に挟まれずに現れるか（`_`・`.`・`` ` `` は区切りとみなす）。
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// 読み手のソースの、試験より前の部分の文字列の定数のうち、識別子の形のもの（正本の欄の名前・JSON のキーと値）。
fn identifiers(source: &str, camel: bool) -> Vec<String> {
    let body = source.split("#[cfg(test)]").next().unwrap();
    let mut out: Vec<String> = Vec::new();
    for piece in body.split('"').skip(1).step_by(2) {
        let ok = piece.starts_with(|c: char| c.is_ascii_lowercase())
            && piece.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '_'
                    || camel && c.is_ascii_uppercase()
            });
        if ok && !out.iter().any(|s| s == piece) {
            out.push(piece.to_owned());
        }
    }
    out
}

#[test]
fn every_entry_the_reader_knows_is_in_the_spec() {
    // 表の行の最初の欄（エントリの名前）で照らす。ほかの欄の文に名前が出ていても、行があるとは言えない
    let table = section("## エントリの一覧");
    let names: Vec<&str> = table
        .lines()
        .filter(|l| l.starts_with("| "))
        .filter_map(|l| cells(l).first().copied())
        .collect();
    let missing: Vec<_> = ROOT_ENTRIES
        .iter()
        .chain(&SET_ENTRIES)
        .filter(|f| !names.iter().any(|n| n.contains(&format!("`{f}`"))))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/YLP_FORMAT.md の「エントリの一覧」に無い: {missing:?}"
    );
    let assets = names
        .iter()
        .find(|n| n.contains("`resources/<content>.png`"))
        .expect("アセットの中身の行");
    for f in RESOURCE_ENTRIES {
        let ext = f.rsplit_once('.').unwrap().1;
        assert!(
            assets.contains(&format!(".{ext}`")),
            "アセットの中身 {f} が「エントリの一覧」に無い"
        );
    }
}

#[test]
fn every_version_is_in_the_spec() {
    // コードの定数の表（形式を変えるときの節）
    let constants = section("### コードの定数");
    for (name, value) in [
        ("MAX_FORMAT", MAX_FORMAT),
        ("SAVED_SELECTIONS_FORMAT", SAVED_SELECTIONS_FORMAT),
        ("UNITY_NATIVE_VERSION", UNITY_NATIVE_VERSION),
        ("USER_CHANNELS_VERSION", USER_CHANNELS_VERSION),
        ("PROCEDURAL_VERSION", PROCEDURAL_VERSION),
        ("ADJUST_VERSION", ADJUST_VERSION),
        ("MIXING_VERSION", MIXING_VERSION),
        ("PATHS_VERSION", PATHS_VERSION),
        ("EFFECTS_VERSION", EFFECTS_VERSION),
        ("POINT_GRADIENT_VERSION", POINT_GRADIENT_VERSION),
        ("SEAMS_VERSION", SEAMS_VERSION),
        ("TEXT_VERSION", TEXT_VERSION),
        ("BAKE_PRIORITY_VERSION", BAKE_PRIORITY_VERSION),
        ("ANTI_ALIAS_VERSION", ANTI_ALIAS_VERSION),
        ("RULERS_VERSION", RULERS_VERSION),
        ("MAX_NATIVE_VERSION", MAX_NATIVE_VERSION),
        ("SPLIT_VERSION", SPLIT_VERSION),
        ("FORMAT_VERSION", mesh_map::FORMAT_VERSION),
    ] {
        assert!(
            constants.contains(&format!("::{name}` | {value} |")),
            "コードの定数の表の {name} が {value} でない"
        );
    }
    for (name, value) in [
        ("look::FORMAT", yolu_io::look::FORMAT),
        ("pose::FORMAT", pose::FORMAT),
        ("saved_selections::FORMAT", saved_selections::FORMAT),
        ("livelink::FORMAT", yolu_io::livelink::FORMAT),
    ] {
        assert!(
            constants.contains(&format!("`{name}` | {value} |")),
            "コードの定数の表の {name} が {value} でない"
        );
    }
    // 「版の数」の表。行の名前ごとに、範囲の欄と、このアプリが書く値の欄を照らす
    let summary = section("### 版の数");
    let range = |name: &str| cells(row(summary, name))[3];
    let written = |name: &str| cells(row(summary, name))[4];
    assert_eq!(range("外側の版"), "1〜4", "外側の版の範囲");
    assert!(written("外側の版").starts_with("3。"), "外側の版の書く値");
    assert_eq!(
        range("中身の形式"),
        format!("1〜{MAX_FORMAT}"),
        "中身の形式の範囲"
    );
    assert!(
        written("中身の形式").starts_with("7。"),
        "中身の形式の書く値"
    );
    assert_eq!(
        range("正本の版"),
        format!(
            "1〜{TEXT_VERSION}・{SEAMS_VERSION}〜{MAX_NATIVE_VERSION}（{} は欠番）",
            TEXT_VERSION + 1
        ),
        "正本の版の範囲（末尾が MAX_NATIVE_VERSION）"
    );
    let native_written = format!(
        "{UNITY_NATIVE_VERSION}〜{MIXING_VERSION}・{PATHS_VERSION}〜{TEXT_VERSION}・{SEAMS_VERSION}〜{MAX_NATIVE_VERSION}"
    );
    assert!(
        written("正本の版").contains(&native_written),
        "正本の版の書く値: {native_written}"
    );
    assert!(
        range("メッシュマップの版").starts_with(&format!("1〜{}。", mesh_map::FORMAT_VERSION)),
        "メッシュマップの版の範囲"
    );
    assert_eq!(
        written("メッシュマップの版"),
        mesh_map::FORMAT_VERSION.to_string(),
        "メッシュマップの版の書く値"
    );
    for (name, value) in [
        ("選択範囲の版", "1"),
        ("JSON の `format`", "1"),
        ("`.ylsmart` の版", "1"),
    ] {
        assert_eq!(range(name), value, "{name}の範囲");
        assert_eq!(written(name), value, "{name}の書く値");
    }
    // 読み手ごとの範囲のこのアプリの行
    let readers = section("### 読み手ごとの範囲");
    let app = cells(row(readers, "このアプリ"));
    assert_eq!(app[1], "`YLP-1`〜`4`", "読み手ごとの範囲: 外側の版");
    assert_eq!(
        app[2],
        format!("1〜{MAX_FORMAT}"),
        "読み手ごとの範囲: 中身の形式"
    );
    assert_eq!(
        app[3],
        format!("1〜{TEXT_VERSION}・{SEAMS_VERSION}〜{MAX_NATIVE_VERSION}"),
        "読み手ごとの範囲: 正本の版"
    );
    // 表の行が版ごとにある
    let formats = section("### 中身の形式の版");
    for v in 1..=MAX_FORMAT {
        assert!(
            formats.contains(&format!("\n| {v} |")),
            "中身の形式 {v} の行が無い"
        );
    }
    let natives = section("### 正本の版ごとの追加");
    for v in (1..=TEXT_VERSION).chain([
        SEAMS_VERSION,
        BAKE_PRIORITY_VERSION,
        ANTI_ALIAS_VERSION,
        RULERS_VERSION,
    ]) {
        assert!(
            natives.contains(&format!("\n| {v} |")),
            "正本の版 {v} の行が無い"
        );
    }
    // 意味の決まっていない版（31）は表に行が無い
    for v in (TEXT_VERSION + 1)..SEAMS_VERSION {
        assert!(
            !natives.contains(&format!("\n| {v} |")),
            "読めない版 {v} の行がある"
        );
    }
}

/// 仕様の中の `#…` の道が、全部見出しに当たる（YLP_FORMAT.md と YLP_DECISIONS.md の相互の道を含む）。
#[test]
fn every_link_to_a_section_reaches_a_heading() {
    let docs = [
        ("YLP_FORMAT.md", SPEC, heading_anchors(SPEC)),
        ("YLP_DECISIONS.md", DECISIONS, heading_anchors(DECISIONS)),
    ];
    let mut broken = Vec::new();
    for (name, doc, _) in &docs {
        for target in link_targets(doc) {
            let (file, fragment) = match target.split_once('#') {
                Some(("", fragment)) => (*name, fragment),
                Some((file, fragment)) if docs.iter().any(|d| d.0 == file) => (file, fragment),
                _ => continue,
            };
            let anchors = &docs.iter().find(|d| d.0 == file).unwrap().2;
            if !anchors.iter().any(|a| a == fragment) {
                broken.push(format!("{name} -> {target}"));
            }
        }
    }
    assert!(broken.is_empty(), "見出しに当たらない道: {broken:?}");
    // 見出しの道の作り方が崩れていないこと（道が 1 つも無い文書では、上の確かめが何も見ない）
    assert!(docs.iter().all(|d| d.2.len() > 10));
    assert!(link_targets(SPEC).iter().any(|t| t.starts_with('#')));
    assert!(link_targets(SPEC)
        .iter()
        .any(|t| t.starts_with("YLP_DECISIONS.md#")));
}

#[test]
fn every_field_of_the_document_is_in_the_spec() {
    let spec = section("## document.utpaint");
    let names = identifiers(include_str!("../../src/native.rs"), false);
    assert!(names.len() > 150, "欄の名前を拾えていない: {}", names.len());
    let missing: Vec<_> = names.iter().filter(|n| !mentions(spec, n)).collect();
    assert!(
        missing.is_empty(),
        "docs/YLP_FORMAT.md の「document.utpaint」に無い欄: {missing:?}"
    );
}

#[test]
fn every_json_key_is_in_the_spec() {
    for (source, heading) in [
        (include_str!("../../src/project.rs"), WHOLE),
        (include_str!("../../src/look.rs"), "### look.json"),
        (include_str!("../../src/pose.rs"), "### pose.json"),
        (
            include_str!("../../src/saved_selections.rs"),
            "### selections.json と残した選択範囲",
        ),
    ] {
        // project.rs の読み手は、ylp.json・project.json・resources.json・view.json・.ylsmart・.ylbrush を読むので、仕様の全体と照らす
        let text = if heading == WHOLE {
            SPEC
        } else {
            section(heading)
        };
        let missing: Vec<_> = identifiers(source, true)
            .into_iter()
            .filter(|n| !mentions(text, n))
            .collect();
        assert!(
            missing.is_empty(),
            "docs/YLP_FORMAT.md の「{}」に無いキー・値: {missing:?}",
            if heading == WHOLE { "全体" } else { heading }
        );
    }
}

/// 書き手が出せるエントリ（セット・選択範囲・残した選択範囲・見た目・ポーズ・モデルの参照・メッシュマップ・合成・アセットの画像）が、
/// どれも仕様の一覧の形に入り、開き直して知らないエントリにならない。
#[test]
fn every_entry_the_writer_makes_has_a_form_in_the_spec() {
    const SET: &str = "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c01";
    let writer = WriterInfo {
        app: "試験".into(),
        version: "1".into(),
        unity: "なし".into(),
    };
    let mut core = Document::with_tile_size(64, 64, 32).unwrap();
    let layer = core.add_layer("レイヤー 1").unwrap();
    core.import_tile(
        layer,
        yolu_core::Channel::Color,
        yolu_core::TileCoord::new(0, 0),
        &[200; 32 * 32 * 4],
    )
    .unwrap();
    let spec = SetSpec {
        id: SET.into(),
        name: "Body".into(),
        material: MaterialRef::Material {
            name: "Skin".into(),
            asset: None,
        },
        document: Some(NativeDocument::from_core(&core).unwrap().into()),
        composites: yolu_io::composite_pngs(&core).unwrap(),
    };
    let mut project = Project::create(writer.clone(), &[spec], SET).unwrap();
    let selection = Selection::from_core(&SelectionMask::rectangle(&core, 0, 0, 10, 10)).unwrap();
    project = project.with_selection(SET, Some(&selection)).unwrap();
    project = project
        .with_saved_selections(
            SET,
            &[saved_selections::SavedSelection {
                name: "前".into(),
                selection: selection.clone(),
            }],
        )
        .unwrap();
    let look = MaterialLook {
        kind: yolu_core::look::LookKind::LilToon,
        ..MaterialLook::default()
    };
    project = project.with_look(SET, Some(&look)).unwrap();
    let pose = pose::StoredPose {
        bones: Vec::new(),
        shapes: vec![pose::StoredShape {
            mesh: "Face".into(),
            name: "Smile".into(),
            weight: 10.0,
        }],
        take: Some(pose::StoredTake {
            name: "Take 001".into(),
            frame: 12,
        }),
    };
    project = project.with_pose(Some(&pose)).unwrap();
    project = project.with_view_model(Some("model.fbx")).unwrap();
    let map = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/mesh/Position.v3.bin"
    ))
    .unwrap();
    project = project
        .with_mesh_map(SET, &mesh_map::read(&map).unwrap())
        .unwrap();
    let mut shelf = project.shelf(u64::MAX).unwrap();
    shelf
        .add_image(
            "aaaaaaaa-0000-4000-8000-000000000001",
            "Scratches",
            &[9; 4 * 4 * 4],
            4,
            4,
            "srgb",
            serde_json::json!({"type": "none"}),
        )
        .unwrap();
    project = project.with_shelf(&shelf, writer).unwrap();

    let names: Vec<String> = project
        .original_archive()
        .entries()
        .keys()
        .cloned()
        .collect();
    for want in [
        "selections.json",
        "look.json",
        "meshmap-Position.bin",
        "composite/Color.png",
        "selection.bin",
    ] {
        assert!(
            names.contains(&format!("sets/{SET}/{want}")),
            "書いたはずの {want} が無い: {names:?}"
        );
    }
    for want in ["pose.json", "view.json", "resources.json"] {
        assert!(
            names.contains(&want.to_owned()),
            "書いたはずの {want} が無い: {names:?}"
        );
    }
    let unformed: Vec<_> = names
        .iter()
        .filter(|n| entry_form(n).is_none() && *n != "ylp.json")
        .collect();
    assert!(
        unformed.is_empty(),
        "仕様の一覧の形に入らないエントリ: {unformed:?}"
    );
    let reopened = Project::read(&project.to_bytes().unwrap()).unwrap();
    assert!(
        reopened.unknown_entries().is_empty(),
        "{:?}",
        reopened.unknown_entries()
    );
    assert_eq!(reopened.info().format, SAVED_SELECTIONS_FORMAT);
}

#[test]
fn entry_forms_recognise_their_names() {
    let set = "sets/5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c01/";
    let hash = "0".repeat(64);
    for (name, form) in [
        (format!("{set}document.utpaint"), "document.utpaint"),
        (format!("{set}document.utpaint.12"), "document.utpaint.<n>"),
        (
            format!("{set}selection-{}.bin", "a".repeat(32)),
            "selection-<印>.bin",
        ),
        (
            format!("{set}composite/Normal.png"),
            "composite/<チャンネル>.png",
        ),
        (format!("{set}meshmap-Id.bin"), "meshmap-<種類>.bin"),
        (
            format!("{set}imported-original.psd"),
            "imported-original.psd",
        ),
        (
            format!("resources/{hash}.ylbrush"),
            "resources/<content>.ylbrush",
        ),
        ("model.json".into(), "model.json"),
    ] {
        assert_eq!(entry_form(&name), Some(form), "{name}");
    }
    for name in [
        format!("{set}document.utpaint.0"),
        format!("{set}notes.txt"),
        format!("{set}selection-{}.bin", "a".repeat(64)),
        "resources/x.png".into(),
        "extra.dat".into(),
    ] {
        assert_eq!(entry_form(&name), None, "{name}");
    }
}
