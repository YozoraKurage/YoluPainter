//! 形式の仕様（`docs/YLP_FORMAT.md`）が、書き手と読み手に追いついているか。エントリ・版・正本の欄・JSON のキーを足して仕様を直し忘れると落ちる。
use yolu_core::{look::MaterialLook, Document, SelectionMask};
use yolu_io::{
    entry_form, mesh_map, pose, saved_selections, MaterialRef, NativeDocument, Project, Selection,
    SetSpec, WriterInfo, ADJUST_VERSION, EFFECTS_VERSION, MAX_FORMAT, MAX_NATIVE_VERSION,
    MIXING_VERSION, PATHS_VERSION, POINT_GRADIENT_VERSION, PROCEDURAL_VERSION, RESOURCE_ENTRIES,
    ROOT_ENTRIES, SAVED_SELECTIONS_FORMAT, SEAMS_VERSION, SET_ENTRIES, SPLIT_VERSION,
    UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION,
};

const SPEC: &str = include_str!("../../../../docs/YLP_FORMAT.md");

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
    let table = section("## エントリ");
    let missing: Vec<_> = ROOT_ENTRIES
        .iter()
        .chain(&SET_ENTRIES)
        .filter(|f| !table.contains(&format!("`{f}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/YLP_FORMAT.md の「エントリ」に無い: {missing:?}"
    );
    for f in RESOURCE_ENTRIES {
        let ext = f.rsplit_once('.').unwrap().1;
        assert!(
            table.contains(&format!(".{ext}`")),
            "棚の中身 {f} が「エントリ」に無い"
        );
    }
}

#[test]
fn every_version_is_in_the_spec() {
    // 早見と、コードの定数の値
    let summary = section("## 版の早見");
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
        ("MAX_NATIVE_VERSION", MAX_NATIVE_VERSION),
        ("SPLIT_VERSION", SPLIT_VERSION),
        ("FORMAT_VERSION", mesh_map::FORMAT_VERSION),
    ] {
        assert!(
            summary.contains(&format!("{name}`（{value}）")),
            "版の早見の {name} が {value} でない"
        );
    }
    for (name, value) in [
        ("look::FORMAT", yolu_io::look::FORMAT),
        ("pose::FORMAT", pose::FORMAT),
        ("saved_selections::FORMAT", saved_selections::FORMAT),
    ] {
        assert!(
            summary.contains(&format!("`{name}`")) && value == 1,
            "版の早見の {name} が {value} でない"
        );
    }
    assert!(
        summary.contains(&format!("1〜{MAX_FORMAT}")),
        "中身の形式の範囲"
    );
    assert!(
        summary.contains(&format!("1〜{POINT_GRADIENT_VERSION}")),
        "正本の版の範囲"
    );
    // 読める版の一番新しいもの（機能の版の末尾）は、範囲の末尾に載る
    assert!(
        summary.contains(&format!("・{MAX_NATIVE_VERSION} |")),
        "正本の版の範囲の末尾が MAX_NATIVE_VERSION でない"
    );
    assert!(
        summary.contains(&format!("版 1〜{}", mesh_map::FORMAT_VERSION)),
        "メッシュマップの版"
    );
    assert!(summary.contains("YOLUPAINTER-YLP-1`〜`4"), "外側の版");
    let ranges = format!(
        "| `YLP-1`〜`4` | 1〜{MAX_FORMAT} | 1〜{POINT_GRADIENT_VERSION}・{SEAMS_VERSION} |"
    );
    assert!(
        summary.contains(&ranges),
        "読み手ごとの範囲のこのアプリの行: {ranges}"
    );
    // 表の行が版ごとにある
    let formats = section("## 中身の形式の版");
    for v in 1..=MAX_FORMAT {
        assert!(
            formats.contains(&format!("\n| {v} |")),
            "中身の形式 {v} の行が無い"
        );
    }
    let natives = section("### 正本の版");
    for v in (1..=POINT_GRADIENT_VERSION).chain([SEAMS_VERSION]) {
        assert!(
            natives.contains(&format!("\n| {v} |")),
            "正本の版 {v} の行が無い"
        );
    }
    // 意味の決まっていない版（30・31）は表に行が無い
    for v in (POINT_GRADIENT_VERSION + 1)..SEAMS_VERSION {
        assert!(
            !natives.contains(&format!("\n| {v} |")),
            "読めない版 {v} の行がある"
        );
    }
}

#[test]
fn every_field_of_the_document_is_in_the_spec() {
    let spec = section("## 正本（document.utpaint）");
    let names = identifiers(include_str!("../../src/native.rs"), false);
    assert!(names.len() > 150, "欄の名前を拾えていない: {}", names.len());
    let missing: Vec<_> = names.iter().filter(|n| !mentions(spec, n)).collect();
    assert!(
        missing.is_empty(),
        "docs/YLP_FORMAT.md の「正本」に無い欄: {missing:?}"
    );
}

#[test]
fn every_json_key_is_in_the_spec() {
    for (source, heading) in [
        (include_str!("../../src/project.rs"), "## エントリ"),
        (
            include_str!("../../src/look.rs"),
            "## look.json（状態。セットの下）",
        ),
        (
            include_str!("../../src/pose.rs"),
            "## pose.json（状態。根）",
        ),
        (
            include_str!("../../src/saved_selections.rs"),
            "### selections.json と selection-<印>.bin（形式 8）",
        ),
    ] {
        // project.rs の読み手は、ylp.json・project.json・棚・view.json・.ylsmart・.ylbrush を読むので、仕様の全体と照らす
        let text = if heading == "## エントリ" {
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
            "docs/YLP_FORMAT.md の「{heading}」に無いキー・値: {missing:?}"
        );
    }
}

/// 書き手が出せるエントリ（セット・選択範囲・残した選択範囲・見た目・ポーズ・モデルの参照・メッシュマップ・合成・棚の画像）が、
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
