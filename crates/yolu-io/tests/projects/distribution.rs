//! 配布用の写し（`Project::for_distribution`）: 種類ごとに除かれる・使っている棚の物は参照の種類ごとに残る・レイヤーの画素は元と同じ・写しをもう一度開ける・
//! 開いているプロジェクトは変わらない。Unity 版が作った .ylp（PSD の原本・file の出どころ・unityAsset・thumbnail・brush.json）は試験の中で組み立てる。
use std::collections::{BTreeMap, BTreeSet};

use yolu_core::{
    look::{LookKind, MaterialLook, ReceivedLook, TextureSource},
    Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, Rgba8,
};
use yolu_io::{
    composite_pngs,
    shelf::{ResourceKind, Shelf},
    Archive, MaterialRef, NativeDocument, Project, Removal, SetSpec, WriterInfo,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

const SET_A: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
const SET_B: &str = "11111111-2222-4333-8444-555555555555";
const SENTINELS: [&str; 5] = [
    "C:\\Users\\tester\\textures\\scratch.png",
    "Assets/Textures/Rust.png",
    "library/stone/brick.png",
    "C:\\Users\\tester\\models\\Prop.fbx",
    "0123456789abcdef0123456789abcdef",
];

/// 棚の項目の ID（n から決まる、小文字の GUID）。
fn rid(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-{n:012x}")
}
fn image_id(id: &str) -> ImageId {
    ImageId(u128::from_str_radix(&id.replace('-', ""), 16).unwrap())
}
/// 2 × 2 の画素（seed ごとに違う）。
fn pixels(seed: u32) -> Vec<u8> {
    (0..16u32).map(|i| (seed * 37 + i * 11) as u8 | 1).collect()
}

/// 塗りつぶしレイヤーが、指定したチャンネルで棚の画像を読む文書。
fn doc_reading(images: &[(u32, Channel)]) -> Document {
    let mut doc = Document::new(64, 64).unwrap();
    let mut inputs = EffectInputs::new();
    for (n, _) in images {
        inputs = inputs.with_image(
            image_id(&rid(*n)),
            ImageInput::new(2, 2, pixels(*n), ImageColorSpace::Srgb).unwrap(),
        );
    }
    doc.set_effect_inputs(inputs).unwrap();
    let layer = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(10, 20, 30, 255)),
                (Channel::Height, Rgba8::new(255, 255, 255, 255)),
            ],
            None,
        )
        .unwrap();
    for (n, channel) in images {
        doc.set_fill_image(layer, *channel, Some(image_id(&rid(*n))))
            .unwrap();
    }
    doc
}

fn spec(id: &str, name: &str, doc: &Document) -> SetSpec {
    SetSpec {
        id: id.into(),
        name: name.into(),
        material: MaterialRef::Material {
            name: name.into(),
            asset: None,
        },
        document: Some(NativeDocument::from_core(doc).unwrap().into()),
        composites: composite_pngs(doc).unwrap(),
    }
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/smart/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn entries(project: &Project) -> BTreeMap<String, Vec<u8>> {
    project
        .original_archive()
        .entries()
        .iter()
        .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
        .collect()
}
fn reopened(entries: BTreeMap<String, Vec<u8>>) -> Project {
    Project::read(&Archive::from_entries(entries).unwrap().to_bytes().unwrap()).unwrap()
}

fn origin_file(path: &str) -> serde_json::Value {
    serde_json::json!({"type":"file","path":path,"sha256":"a".repeat(64),"length":10})
}
fn origin_unity(path: &str) -> serde_json::Value {
    serde_json::json!({"type":"unityAsset","guid":"0123456789abcdef0123456789abcdef","path":path,"stamp":"s","readThroughGpu":false})
}
fn origin_library(file: &str) -> serde_json::Value {
    serde_json::json!({"type":"library","file":file,"sha256":"b".repeat(64),"length":10})
}

/// Unity 版が作ったような .ylp: 2 つのセット（A は塗りつぶしが棚の画像 1 を色・画像 2 を高さで読み、B は画像 3 を読む）、PSD の原本、
/// メッシュマップ、見た目（B は画像 4 を割り当て、A・B とも Unity の値があり、A の Unity の値は画像 5 を指す）、状態のエントリ、知らないエントリ、
/// 棚（使っている物 1〜5、使っていない物 6〜8・スマートマテリアル・ブラシ）。
fn unity_style() -> Project {
    let a = doc_reading(&[(1, Channel::Color), (2, Channel::Height)]);
    let b = doc_reading(&[(3, Channel::Color)]);
    let base = Project::create(
        writer(),
        &[spec(SET_A, "Body", &a), spec(SET_B, "Prop", &b)],
        SET_A,
    )
    .unwrap();
    let mut shelf = Shelf::new(1 << 30);
    let image = |shelf: &mut Shelf, n: u32, name: &str, origin: serde_json::Value| {
        shelf
            .add_image(&rid(n), name, &pixels(n), 2, 2, "srgb", origin)
            .unwrap();
    };
    image(&mut shelf, 1, "Scratch", origin_file(SENTINELS[0]));
    image(&mut shelf, 2, "Rust", origin_unity(SENTINELS[1]));
    image(&mut shelf, 3, "Brick", origin_library(SENTINELS[2]));
    image(&mut shelf, 4, "MatCap", serde_json::json!({"type":"none"}));
    image(
        &mut shelf,
        5,
        "Received",
        origin_unity("Assets/Textures/Rcv.png"),
    );
    image(
        &mut shelf,
        6,
        "Spare",
        origin_unity("Assets/Textures/Spare.png"),
    );
    image(&mut shelf, 7, "Sketch", serde_json::json!({"type":"none"}));
    image(
        &mut shelf,
        8,
        "Bundled",
        serde_json::json!({"type":"builtIn","key":"noise","version":1}),
    );
    let smart = fixture("raster.ylsmart");
    shelf
        .add_file(
            &rid(20),
            "Smart",
            ResourceKind::SmartMaterial,
            &smart,
            origin_library("smart/Raster.ylsmart"),
        )
        .unwrap();
    let brush = fixture("brush.ylbrush");
    shelf
        .add_file(
            &rid(21),
            "Brush",
            ResourceKind::Brush,
            &brush,
            serde_json::json!({"type":"none"}),
        )
        .unwrap();
    let project = base.with_shelf(&shelf, writer()).unwrap();
    let mut files = entries(&project);
    let set = |id: &str, leaf: &str| format!("sets/{id}/{leaf}");
    files.insert(
        set(SET_A, "imported-original.psd"),
        b"8BPS-fake-original".to_vec(),
    );
    files.insert(set(SET_A, "meshmap-Id.bin"), b"mesh-a".to_vec());
    files.insert(set(SET_B, "meshmap-Ao.bin"), b"mesh-b".to_vec());
    files.insert(set(SET_A, "notes.txt"), b"unknown in a set".to_vec());
    files.insert("extra.dat".into(), b"unknown at root".to_vec());
    files.insert("thumbnail.png".into(), b"old-thumbnail".to_vec());
    files.insert("brush.json".into(), b"{\"schema\":3}".to_vec());
    files.insert("model.json".into(), b"{}".to_vec());
    let view = serde_json::json!({
        "modelAssetGuid": SENTINELS[4],
        "selectedChannel": 2,
        "standaloneModel": { "path": SENTINELS[3] },
        "visibility": { "hiddenSets": [SET_B], "hiddenRenderers": ["000001/000000:0"] }
    });
    files.insert(
        "view.json".into(),
        serde_json::to_vec_pretty(&view).unwrap(),
    );
    // 見た目: B は利用者の設定（画像 4）と Unity の値、A は Unity の値だけ（利用者の設定は既定。Unity の値は画像 5 を指す）
    let received = |image: Option<u32>| ReceivedLook {
        look: MaterialLook {
            kind: LookKind::LilToon,
            textures: image
                .map(|n| {
                    (
                        "_MatCapTex".to_string(),
                        TextureSource::Image(image_id(&rid(n))),
                    )
                })
                .into_iter()
                .collect(),
            ..MaterialLook::default()
        },
        source: "lilToon 2.3.4".into(),
        images: Default::default(),
        missing: Default::default(),
    };
    let user_b = MaterialLook {
        kind: LookKind::LilToon,
        kind_chosen: true,
        textures: [(
            "_MainTex".to_string(),
            TextureSource::Image(image_id(&rid(4))),
        )]
        .into(),
        ..MaterialLook::default()
    };
    let look_b = yolu_io::look::write(&user_b, None).unwrap();
    let look_b = yolu_io::look::write_received(Some(&received(None)), Some(&look_b))
        .unwrap()
        .unwrap();
    files.insert(set(SET_B, "look.json"), look_b);
    let look_a = yolu_io::look::write_received(Some(&received(Some(5))), None)
        .unwrap()
        .unwrap();
    files.insert(set(SET_A, "look.json"), look_a);
    reopened(files)
}

/// Unity 版が作った .ylp にある種類（名前を付けて残した選択範囲は、スタンドアロン版だけが書く形式 8 のエントリ）。
fn unity_kinds() -> Vec<Removal> {
    Removal::ALL
        .into_iter()
        .filter(|r| *r != Removal::SavedSelections)
        .collect()
}

fn names(project: &Project, removal: Removal) -> Vec<String> {
    project
        .distribution_inventory(&Removal::ALL)
        .get(removal)
        .map(|f| f.names.clone())
        .unwrap_or_default()
}

#[test]
fn the_fixture_is_what_a_unity_made_project_leaves_behind() {
    let project = unity_style();
    let files = entries(&project);
    for entry in [
        "sets/0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0/imported-original.psd",
        "thumbnail.png",
        "brush.json",
        "view.json",
        "resources.json",
    ] {
        assert!(files.contains_key(entry), "{entry}");
    }
    assert_eq!(project.resources().len(), 10);
    assert_eq!(
        project.unknown_entries().len(),
        2,
        "{:?}",
        project.unknown_entries()
    );
    // 元の中身に、取り込んだ元のパスとモデルの場所が見える
    let all: Vec<u8> = files.values().flatten().copied().collect();
    let text = String::from_utf8_lossy(&all);
    for sentinel in SENTINELS {
        assert!(
            text.contains(&sentinel.replace('\\', "\\\\")) || text.contains(sentinel),
            "{sentinel}"
        );
    }
}

#[test]
fn the_inventory_lists_each_kind_with_names_and_leaves_out_empty_kinds() {
    let project = unity_style();
    let inventory = project.distribution_inventory(&Removal::ALL);
    // ウィンドウに並べる順（Unity 版が作った .ylp は、名前を付けて残した選択範囲を持たない）
    assert_eq!(inventory.kinds(), unity_kinds());
    assert_eq!(names(&project, Removal::PsdOriginals), ["Body"]);
    assert_eq!(
        names(&project, Removal::UnusedShelf),
        ["Received", "Spare", "Sketch", "Bundled", "Smart", "Brush"],
        "Unity の値も除くので、その中の割り当てだけが指す画像（Received）も使っていない"
    );
    assert_eq!(
        names(&project, Removal::SourcePaths),
        ["Scratch", "Rust", "Brick", "Received", "Spare", "Smart"],
        "file・unityAsset・library の出どころ。none と builtIn は含まない"
    );
    assert_eq!(names(&project, Removal::ModelReference), ["Prop.fbx"]);
    assert_eq!(names(&project, Removal::MeshMaps), ["Body", "Prop"]);
    assert_eq!(names(&project, Removal::UnityValues), ["Body", "Prop"]);
    assert_eq!(
        names(&project, Removal::StaleEntries),
        ["thumbnail.png", "brush.json", "model.json"]
    );
    assert_eq!(
        names(&project, Removal::UnknownEntries),
        project.unknown_entries()
    );
    // 何も無い写しの目録は空（0 件の種類は出さない）
    let clean = project.for_distribution(writer(), &Removal::ALL).unwrap();
    let left = clean.distribution_inventory(&Removal::ALL);
    assert!(left.is_empty(), "{left:?}");
    let plain =
        Project::create(writer(), &[spec(SET_A, "Body", &doc_reading(&[]))], SET_A).unwrap();
    assert!(plain.distribution_inventory(&Removal::ALL).is_empty());
}

#[test]
fn what_counts_as_unused_follows_the_kinds_that_are_chosen() {
    let project = unity_style();
    let unused = |remove: &[Removal]| {
        project
            .distribution_inventory(remove)
            .get(Removal::UnusedShelf)
            .map(|f| f.names.clone())
            .unwrap_or_default()
    };
    // Unity の値を残す（除かない）なら、その中の割り当てが指す画像は使っている
    assert_eq!(
        unused(&[Removal::UnusedShelf, Removal::SourcePaths]),
        ["Spare", "Sketch", "Bundled", "Smart", "Brush"]
    );
    // 除くなら使っていない。写しも同じ結果になる（目録と写しで食い違わない）
    for remove in [
        vec![Removal::UnusedShelf],
        vec![Removal::UnusedShelf, Removal::UnityValues],
        Removal::ALL.to_vec(),
    ] {
        let listed = unused(&remove);
        let copy = project.for_distribution(writer(), &remove).unwrap();
        let kept: BTreeSet<String> = copy.resources().iter().map(|r| r.name.clone()).collect();
        let all: BTreeSet<String> = project.resources().iter().map(|r| r.name.clone()).collect();
        let dropped: BTreeSet<String> = all.difference(&kept).cloned().collect();
        assert_eq!(
            dropped,
            listed.into_iter().collect::<BTreeSet<_>>(),
            "{remove:?}"
        );
    }
    // 使っていない物の名前の種類そのものも、選びで出入りしてよい: 何も除かないなら、除く物は目録に出るが残る
    let none = project.for_distribution(writer(), &[]).unwrap();
    assert_eq!(none.resources().len(), project.resources().len());
}

#[test]
fn a_shelf_item_that_a_layer_or_a_look_points_at_is_used_whatever_the_kind_of_reference() {
    let project = unity_style();
    let used: BTreeSet<String> = project.used_resource_ids();
    let want: BTreeSet<String> = (1..=5).map(rid).collect();
    assert_eq!(
        used, want,
        "1: 色の画像、2: 高さの画像（チャンネル違い）、3: 別のセットのレイヤー、4: 利用者の見た目の割り当て、5: Unity の値の中の割り当て"
    );
    // 使っていない物は含まれない
    for n in [6, 7, 8, 20, 21] {
        assert!(!used.contains(&rid(n)), "{n}");
    }
}

/// 棚に使っていない画像 1 つ（`rid(9)`）だけを持つプロジェクトで、`name`（レイヤーの名前の Text の項目）と `look`（`look.json` の中身。無ければ置かない）を
/// 差し替えたもの。ID が正本の Guid の項目や look.json の引用符で区切られた値としてではなく、文字列の中の ID として現れる場合の確かめに使う。
fn text_project(name: &str, look: Option<Vec<u8>>) -> Project {
    let doc = doc_reading(&[]);
    let native = NativeDocument::from_core(&doc).unwrap();
    let path = native
        .fields()
        .iter()
        .find(|f| {
            f.path.ends_with(".name")
                && matches!(&f.value, yolu_io::NativeValue::Text(t) if t == "塗り")
        })
        .expect("レイヤーの名前の項目")
        .path
        .clone();
    let native = native
        .with_value(&path, yolu_io::NativeValue::Text(name.into()))
        .unwrap();
    let base = Project::create(
        writer(),
        &[SetSpec {
            id: SET_A.into(),
            name: "Body".into(),
            material: MaterialRef::Material {
                name: "Body".into(),
                asset: None,
            },
            document: Some(native.into()),
            composites: composite_pngs(&doc).unwrap(),
        }],
        SET_A,
    )
    .unwrap();
    let mut shelf = Shelf::new(1 << 30);
    shelf
        .add_image(
            &rid(9),
            "Hidden",
            &pixels(9),
            2,
            2,
            "srgb",
            serde_json::json!({"type":"none"}),
        )
        .unwrap();
    let project = base.with_shelf(&shelf, writer()).unwrap();
    let mut files = entries(&project);
    if let Some(look) = look {
        files.insert(format!("sets/{SET_A}/look.json"), look);
    }
    reopened(files)
}

#[test]
fn an_id_inside_a_string_is_found_even_with_hex_characters_or_hyphens_beside_it() {
    let id = rid(9);
    let key = id.replace('-', "");
    // レイヤーの名前のような Text の項目: ID の前後に 16 進の文字・ハイフン・数字が付いても、使っている物として数える
    for name in [
        format!("e-{key}"),
        format!("{key}-1"),
        format!("face{key}"),
        format!("{key}beef"),
        format!("x{key}"),
        format!("face{id}"),
        format!("{id}-1"),
        format!("a-{id}-b"),
        key.clone(),
        id.clone(),
    ] {
        let project = text_project(&name, None);
        assert!(
            project.used_resource_ids().contains(&id),
            "{name} の中の ID を見落とした"
        );
        let copy = project
            .for_distribution(writer(), &[Removal::UnusedShelf])
            .unwrap();
        assert_eq!(
            copy.resources().len(),
            1,
            "{name}: 使っている物が写しから消えた"
        );
    }
    // ID でない文字列（32 桁に足りない・16 進でない文字で切れる）は数えない
    for name in [
        key[1..].to_string(),
        format!("{}g{}", &key[..16], &key[16..]),
    ] {
        let project = text_project(&name, None);
        assert!(
            project.used_resource_ids().is_empty(),
            "{name} を ID と読んだ"
        );
    }
    // look.json: 画像の割り当ての ID の前後に 16 進の文字が付いていても数える（構造は見ず、文字列として当てる）
    let look = MaterialLook {
        kind: LookKind::LilToon,
        kind_chosen: true,
        textures: [("_MainTex".to_string(), TextureSource::Image(image_id(&id)))].into(),
        ..MaterialLook::default()
    };
    let bytes = yolu_io::look::write(&look, None).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains(&key), "look.json の形が変わった: {text}");
    // まず引用符で区切られた普通の形
    let plain = text_project("塗り", Some(text.clone().into_bytes()));
    assert!(plain.used_resource_ids().contains(&id));
    for glued in [
        format!("face{key}"),
        format!("{key}0"),
        format!("e-{key}"),
        format!("{key}-1"),
    ] {
        let edited = text.replace(&key, &glued);
        let project = text_project("塗り", Some(edited.into_bytes()));
        assert!(
            project.used_resource_ids().contains(&id),
            "look.json の {glued} の中の ID を見落とした"
        );
    }
}

/// 元のエントリのうち、写しで消えた・中身が変わったものの名前（`ylp.json` は保存したアプリを書き直すので数えない）。
fn changed(before: &Project, after: &Project) -> (BTreeSet<String>, BTreeSet<String>) {
    let (b, a) = (entries(before), entries(after));
    let removed = b.keys().filter(|n| !a.contains_key(*n)).cloned().collect();
    let modified = b
        .iter()
        .filter(|(n, v)| n.as_str() != "ylp.json" && a.get(*n).is_some_and(|x| x != *v))
        .map(|(n, _)| n.clone())
        .collect();
    (removed, modified)
}

#[test]
fn each_kind_removes_only_its_own_things_and_the_toggle_keeps_them() {
    let project = unity_style();
    let a = |leaf: &str| format!("sets/{SET_A}/{leaf}");
    let b = |leaf: &str| format!("sets/{SET_B}/{leaf}");
    let content = |n: u32| {
        let r = project.resources().iter().find(|r| r.id == rid(n)).unwrap();
        r.entry.clone()
    };
    let cases: Vec<(Removal, Vec<String>, Vec<String>)> = vec![
        (
            Removal::PsdOriginals,
            vec![a("imported-original.psd")],
            vec![],
        ),
        (
            Removal::UnusedShelf,
            [6, 7, 8, 20, 21].map(content).to_vec(),
            vec!["resources.json".into()],
        ),
        (Removal::SourcePaths, vec![], vec!["resources.json".into()]),
        (Removal::ModelReference, vec![], vec!["view.json".into()]),
        (
            Removal::MeshMaps,
            vec![a("meshmap-Id.bin"), b("meshmap-Ao.bin")],
            vec![],
        ),
        (
            Removal::UnityValues,
            vec![a("look.json")],
            vec![b("look.json")],
        ),
        (
            Removal::StaleEntries,
            vec![
                "thumbnail.png".into(),
                "brush.json".into(),
                "model.json".into(),
            ],
            vec![],
        ),
        (
            Removal::UnknownEntries,
            vec![a("notes.txt"), "extra.dat".into()],
            vec![],
        ),
    ];
    assert_eq!(cases.len(), unity_kinds().len());
    for (removal, removed, modified) in cases {
        let copy = project.for_distribution(writer(), &[removal]).unwrap();
        let (gone, replaced) = changed(&project, &copy);
        let want_gone: BTreeSet<String> = removed.into_iter().collect();
        let want_replaced: BTreeSet<String> = modified.into_iter().collect();
        assert_eq!(gone, want_gone, "{removal:?} で消えるもの");
        assert_eq!(replaced, want_replaced, "{removal:?} で書き直すもの");
        // 切り替えを外した（何も除かない）写しは、保存したアプリ以外が元と同じ
        let kept = project.for_distribution(writer(), &[]).unwrap();
        let (gone, replaced) = changed(&project, &kept);
        assert!(gone.is_empty(), "{gone:?}");
        assert!(replaced.is_empty(), "{replaced:?}");
    }
}

#[test]
fn removing_the_used_shelf_items_never_happens_but_the_unused_ones_and_their_pixels_go() {
    let project = unity_style();
    let copy = project
        .for_distribution(writer(), &[Removal::UnusedShelf])
        .unwrap();
    let ids: BTreeSet<String> = copy.resources().iter().map(|r| r.id.clone()).collect();
    assert_eq!(
        ids,
        (1..=5).map(rid).collect::<BTreeSet<_>>(),
        "Unity の値を残すので、その中の割り当てが指す画像 5 も残る"
    );
    // 使っている物は中身も出どころも元のまま
    for n in 1..=5 {
        let (before, after) = (
            project.resources().iter().find(|r| r.id == rid(n)).unwrap(),
            copy.resources().iter().find(|r| r.id == rid(n)).unwrap(),
        );
        assert_eq!(before.content, after.content);
        assert_eq!(
            entries(&project)[&before.entry],
            entries(&copy)[&after.entry],
            "{n}"
        );
        for key in ["type", "path", "file"] {
            assert_eq!(
                before.metadata["origin"][key], after.metadata["origin"][key],
                "{n}: 出どころは別の種類"
            );
        }
    }
}

#[test]
fn source_paths_are_cleared_from_every_kept_item_but_built_in_keys_stay() {
    let project = unity_style();
    let copy = project
        .for_distribution(writer(), &[Removal::SourcePaths])
        .unwrap();
    for r in copy.resources() {
        let kind = r.metadata["origin"]["type"].as_str().unwrap();
        let before = project.resources().iter().find(|b| b.id == r.id).unwrap();
        let was = before.metadata["origin"]["type"].as_str().unwrap();
        match was {
            "file" | "unityAsset" | "library" => assert_eq!(kind, "none", "{}", r.name),
            other => assert_eq!(kind, other, "{}", r.name),
        }
    }
    // 画素は変わらない
    for r in copy.resources() {
        let before = project.resources().iter().find(|b| b.id == r.id).unwrap();
        assert_eq!(before.content, r.content);
    }
}

#[test]
fn the_full_copy_has_no_path_original_or_unity_value_and_opens_again() {
    let project = unity_style();
    let before = entries(&project);
    let copy = project.for_distribution(writer(), &Removal::ALL).unwrap();
    let bytes = copy.to_bytes().unwrap();
    // もう一度開ける（Unity 版の読み手と同じ決まり: 並びの物は全部そろい、知らないエントリも無い）
    let again = Project::read(&bytes).unwrap();
    assert!(again.unknown_entries().is_empty());
    assert_eq!(again.info().format, 7);
    assert_eq!(again.info().saved_by.as_ref().unwrap().app, "試験の書き手");
    assert_eq!(again.sets().len(), 2);
    assert_eq!(again.current_set(), SET_A);
    // 取り込んだ元のパス・モデルの場所・PSD・古い状態は、どこにも残らない
    let all = entries(&again);
    let text =
        String::from_utf8_lossy(&all.values().flatten().copied().collect::<Vec<u8>>()).into_owned();
    for sentinel in SENTINELS {
        assert!(
            !text.contains(&sentinel.replace('\\', "\\\\")) && !text.contains(sentinel),
            "{sentinel}"
        );
    }
    for n in all.keys() {
        assert!(
            !n.ends_with(".psd")
                && !n.contains("meshmap-")
                && !["thumbnail.png", "brush.json", "model.json", "extra.dat"]
                    .contains(&n.as_str()),
            "{n}"
        );
    }
    // レイヤー（正本）・選択・合成は元と同じバイト列
    for set in [SET_A, SET_B] {
        for leaf in ["document.utpaint", "composite/Color.png"] {
            let name = format!("sets/{set}/{leaf}");
            assert_eq!(before[&name], all[&name], "{name}");
        }
    }
    for (x, y) in project.sets().iter().zip(again.sets()) {
        assert_eq!(
            x.document.to_bytes().unwrap(),
            y.document.to_bytes().unwrap(),
            "{}",
            x.name
        );
        assert_eq!(
            (x.id.as_str(), x.name.as_str()),
            (y.id.as_str(), y.name.as_str())
        );
    }
    // 使っている棚の画像の画素は同じ（画像の入力として復号して比べる）
    let (a, b) = (
        project.image_inputs().unwrap(),
        again.image_inputs().unwrap(),
    );
    for (id, image) in &b {
        let original = a.iter().find(|(i, _)| i == id).expect("元にもある");
        assert_eq!(original.1.hash, image.hash);
    }
    assert_eq!(
        b.len(),
        4,
        "Unity の値の中の割り当てだけが指していた 5 は、Unity の値と一緒に外れる"
    );
    // 利用者の見た目の設定（B の画像 4 の割り当て）は残り、Unity の値だけが消える
    let look = again.look(SET_B).unwrap().expect("B の設定は残る");
    assert!(look.kind_chosen);
    assert_eq!(
        look.textures["_MainTex"],
        TextureSource::Image(image_id(&rid(4)))
    );
    assert!(again.received_look(SET_B).unwrap().is_none());
    assert!(
        again.look(SET_A).unwrap().is_none(),
        "A は Unity の値だけだったのでエントリごと無い"
    );
    // 表示の状態は残り、モデルの参照だけが空になる（セットの目と選んだチャンネルは残る）
    let view: serde_json::Value = serde_json::from_slice(&all["view.json"]).unwrap();
    assert_eq!(view["selectedChannel"], 2);
    assert_eq!(view["modelAssetGuid"], "");
    assert!(view.get("standaloneModel").is_none());
    assert_eq!(view["visibility"]["hiddenSets"][0], SET_B);
    assert_eq!(
        view["visibility"]["hiddenRenderers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(again.view_model().unwrap(), None);
    // 写しを写しても同じ（もう除くものは無い）
    let twice = again.for_distribution(writer(), &Removal::ALL).unwrap();
    assert_eq!(
        entries(&twice).keys().collect::<Vec<_>>(),
        all.keys().collect::<Vec<_>>()
    );
}

#[test]
fn the_open_project_does_not_change() {
    let project = unity_style();
    let before = entries(&project);
    let bytes = project.to_bytes().unwrap();
    let _ = project.for_distribution(writer(), &Removal::ALL).unwrap();
    let _ = project.distribution_inventory(&Removal::ALL);
    let _ = project.used_resource_ids();
    assert_eq!(entries(&project), before);
    assert_eq!(project.to_bytes().unwrap(), bytes);
}

#[test]
fn an_unused_only_shelf_leaves_no_index_and_no_resource_entries() {
    let doc = doc_reading(&[]);
    let base = Project::create(writer(), &[spec(SET_A, "Body", &doc)], SET_A).unwrap();
    let mut shelf = Shelf::new(1 << 30);
    shelf
        .add_image_without_origin(&rid(1), "Spare", &pixels(1), 2, 2, "srgb")
        .unwrap();
    let project = base.with_shelf(&shelf, writer()).unwrap();
    assert!(entries(&project).contains_key("resources.json"));
    let copy = project
        .for_distribution(writer(), &[Removal::UnusedShelf])
        .unwrap();
    let files = entries(&copy);
    assert!(!files.contains_key("resources.json"));
    assert!(
        files.keys().all(|n| !n.starts_with("resources/")),
        "{:?}",
        files.keys()
    );
    Project::read(&copy.to_bytes().unwrap()).unwrap();
}

#[test]
fn a_content_shared_with_a_kept_item_stays_when_the_unused_item_goes() {
    // 同じ画素の項目が 2 つ（片方はレイヤーが読み、片方は使っていない）。PNG は 1 つで、使っている方が残る
    let doc = doc_reading(&[(1, Channel::Color)]);
    let base = Project::create(writer(), &[spec(SET_A, "Body", &doc)], SET_A).unwrap();
    let mut shelf = Shelf::new(1 << 30);
    shelf
        .add_image_without_origin(&rid(1), "Used", &pixels(1), 2, 2, "srgb")
        .unwrap();
    // 同じ中身は add が同じ ID を返す（棚では 1 つ）。索引を組み替えて 2 つ目の項目を作る
    let project = base.with_shelf(&shelf, writer()).unwrap();
    let mut files = entries(&project);
    let mut index: serde_json::Value = serde_json::from_slice(&files["resources.json"]).unwrap();
    let mut twin = index["resources"][0].clone();
    twin["id"] = serde_json::Value::from(rid(2));
    twin["name"] = serde_json::Value::from("Twin");
    index["resources"].as_array_mut().unwrap().push(twin);
    files.insert("resources.json".into(), serde_json::to_vec(&index).unwrap());
    let project = reopened(files);
    assert_eq!(project.resources().len(), 2);
    assert_eq!(project.resources()[0].entry, project.resources()[1].entry);
    let copy = project
        .for_distribution(writer(), &[Removal::UnusedShelf])
        .unwrap();
    assert_eq!(copy.resources().len(), 1);
    assert_eq!(copy.resources()[0].name, "Used");
    assert!(entries(&copy).contains_key(&copy.resources()[0].entry));
    assert_eq!(copy.image_inputs().unwrap().len(), 1);
}

#[test]
fn imported_original_is_dropped_per_set_and_only_that_one() {
    let project = unity_style();
    let copy = project.without_imported_original(SET_A).unwrap();
    let (gone, replaced) = changed(&project, &copy);
    assert_eq!(
        gone,
        [format!("sets/{SET_A}/imported-original.psd")]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    assert!(replaced.is_empty(), "{replaced:?}");
    assert!(project.without_imported_original("no-such-set").is_err());
    // 無ければそのまま
    let again = copy.without_imported_original(SET_A).unwrap();
    assert_eq!(entries(&again), entries(&copy));
    let other = project.without_imported_original(SET_B).unwrap();
    assert_eq!(entries(&other), entries(&project));
}

#[test]
fn a_view_that_cannot_be_read_goes_with_the_model_reference_and_an_unreadable_look_refuses() {
    let project = unity_style();
    let mut files = entries(&project);
    files.insert("view.json".into(), b"{ not json".to_vec());
    let broken = reopened(files);
    assert_eq!(names(&broken, Removal::ModelReference), ["view.json"]);
    let copy = broken
        .for_distribution(writer(), &[Removal::ModelReference])
        .unwrap();
    assert!(!entries(&copy).contains_key("view.json"));
    // 読めない見た目に Unity の値があれば、中を確かめられないので除けない（利用者の設定を黙って消さない）。理由はセットの名前つき
    let mut files = entries(&project);
    let mut look: serde_json::Value =
        serde_json::from_slice(&files[&format!("sets/{SET_B}/look.json")]).unwrap();
    look["format"] = serde_json::Value::from(2);
    files.insert(
        format!("sets/{SET_B}/look.json"),
        serde_json::to_vec(&look).unwrap(),
    );
    let unreadable = reopened(files);
    let error = unreadable
        .for_distribution(writer(), &[Removal::UnityValues])
        .unwrap_err()
        .to_string();
    assert!(error.contains("Prop"), "{error}");
    // 切り替えを外せば、そのまま写せる
    unreadable.for_distribution(writer(), &[]).unwrap();
}

#[test]
fn only_a_format_7_project_can_be_copied() {
    let old = Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format6.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(old.for_distribution(writer(), &Removal::ALL).is_err());
    let upgraded = old.upgraded(writer()).unwrap();
    upgraded.for_distribution(writer(), &Removal::ALL).unwrap();
}

/// スタンドアロン版が書く、名前を付けて残した選択範囲とポーズを持つプロジェクト。
fn with_remembered() -> Project {
    let a = doc_reading(&[(1, Channel::Color)]);
    let b = doc_reading(&[]);
    let base = Project::create(
        writer(),
        &[spec(SET_A, "Body", &a), spec(SET_B, "Prop", &b)],
        SET_A,
    )
    .unwrap();
    let selection = |x1: i64| {
        yolu_io::Selection::from_core(&yolu_core::SelectionMask::rectangle(&a, 0, 0, x1, 30))
            .unwrap()
    };
    let saved = |name: &str, x1| yolu_io::saved_selections::SavedSelection {
        name: name.into(),
        selection: selection(x1),
    };
    let pose = yolu_io::pose::StoredPose {
        bones: vec![yolu_io::pose::StoredBone {
            path: vec!["Root".into()],
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }],
        shapes: Vec::new(),
        take: None,
    };
    base.with_selection(SET_A, Some(&selection(20)))
        .unwrap()
        .with_saved_selections(SET_A, &[saved("a", 10), saved("b", 30)])
        .unwrap()
        .with_saved_selections(SET_B, &[saved("c", 5)])
        .unwrap()
        .with_view_model(Some("models/Prop.fbx"))
        .unwrap()
        .with_pose(Some(&pose))
        .unwrap()
}

#[test]
fn the_remembered_selections_are_their_own_kind_and_removing_them_takes_the_copy_back_to_format_7()
{
    let project = with_remembered();
    assert_eq!(project.info().format, 8);
    assert_eq!(names(&project, Removal::SavedSelections), ["Body", "Prop"]);
    // 残す写し: 形式 8 のまま、残した選択範囲も今の選択範囲も元のとおり
    let kept = project
        .for_distribution(writer(), &[Removal::ModelReference])
        .unwrap();
    assert_eq!(kept.info().format, 8);
    assert_eq!(kept.saved_selections(SET_A).unwrap().items.len(), 2);
    assert_eq!(kept.saved_selections(SET_B).unwrap().items.len(), 1);
    // 除く写し: 残した選択範囲のエントリが全部無くなり、形式は 7。今の選択範囲（絵の一部）は残る
    let copy = project
        .for_distribution(writer(), &[Removal::SavedSelections])
        .unwrap();
    assert_eq!(copy.info().format, 7, "Unity 版が開ける形式に戻る");
    assert!(copy.saved_selections(SET_A).unwrap().items.is_empty());
    assert!(!entries(&copy)
        .keys()
        .any(|n| n.ends_with("selections.json") || n.contains("/selection-")));
    assert!(copy.sets()[0].selection.is_some(), "今の選択範囲は絵の一部");
    // 元は変わらない。写しに残る物と目録が一致する（除いた写しの目録に、この種類は出ない）
    assert_eq!(project.info().format, 8);
    assert!(copy
        .distribution_inventory(&Removal::ALL)
        .get(Removal::SavedSelections)
        .is_none());
    // 読み直せる
    let again = Project::read(&copy.to_bytes().unwrap()).unwrap();
    assert_eq!(again.info().format, 7);
}

#[test]
fn the_pose_goes_with_the_model_reference_and_stays_otherwise() {
    let project = with_remembered();
    assert!(entries(&project).contains_key("pose.json"));
    // モデルの参照を残すなら、ポーズも残る
    let kept = project
        .for_distribution(writer(), &[Removal::SavedSelections])
        .unwrap();
    assert!(entries(&kept).contains_key("pose.json"));
    assert!(kept.view_model().unwrap().is_some());
    // モデルの参照を除くなら、ポーズも除く（どのモデルのポーズか分からなくなる）
    let copy = project
        .for_distribution(writer(), &[Removal::ModelReference])
        .unwrap();
    assert!(!entries(&copy).contains_key("pose.json"));
    assert!(copy.view_model().unwrap().is_none());
}

/// Live Link で開いたモデルの記録（根の `livelink.json`）。FBX と絵のファイルの絶対の場所・Unity のプロジェクトの場所・書き出しの置き場を持つ。
const LIVE_LINK: &str = r#"{"format":1,"kind":"open","target":{"key":"GlobalObjectId_V1-2-0123-4567-0","name":"Prop","export":"C:\\Users\\tester\\export"},"models":[{"path":"C:\\Users\\tester\\models\\Prop.fbx"}],"unityProject":"C:\\Users\\tester\\UnityProject"}"#;

#[test]
fn the_live_link_record_goes_with_the_model_reference_and_stays_otherwise() {
    let project = with_remembered()
        .with_livelink(Some(LIVE_LINK.as_bytes()))
        .unwrap();
    assert!(entries(&project).contains_key("livelink.json"));
    // 目録: モデルのファイルの名前に並べて、記録のエントリの名前も挙げる
    assert_eq!(
        names(&project, Removal::ModelReference),
        ["Prop.fbx", "livelink.json"]
    );
    // モデルの参照を残すなら、記録も残る（バイト列のまま）
    let kept = project
        .for_distribution(writer(), &[Removal::SavedSelections])
        .unwrap();
    assert_eq!(
        kept.livelink().unwrap().as_deref(),
        Some(LIVE_LINK.as_bytes())
    );
    // モデルの参照を除くなら、記録も除く（絶対の場所とポーズが写しに残らない）
    let copy = project
        .for_distribution(writer(), &[Removal::ModelReference])
        .unwrap();
    assert!(!entries(&copy).contains_key("livelink.json"));
    assert_eq!(copy.livelink().unwrap(), None);
    let text = String::from_utf8_lossy(
        &entries(&copy)
            .values()
            .flatten()
            .copied()
            .collect::<Vec<u8>>(),
    )
    .into_owned();
    assert!(
        !text.contains("tester") && !text.contains("GlobalObjectId"),
        "作った人の場所や Unity のオブジェクトの鍵が残る"
    );
    // 読み直せて、除く物はもう無い
    let again = Project::read(&copy.to_bytes().unwrap()).unwrap();
    assert!(again.unknown_entries().is_empty());
    assert!(!again
        .distribution_inventory(&Removal::ALL)
        .kinds()
        .contains(&Removal::ModelReference));
    // 開いているプロジェクトは変わらない
    assert_eq!(
        project.livelink().unwrap().as_deref(),
        Some(LIVE_LINK.as_bytes())
    );
}

#[test]
fn a_live_link_record_without_a_view_still_counts_as_a_model_reference() {
    // view.json が無い（モデルのファイルの参照が無い）プロジェクトでも、記録だけで種類が当たる
    let base = Project::create(writer(), &[spec(SET_A, "Body", &doc_reading(&[]))], SET_A).unwrap();
    assert!(!entries(&base).contains_key("view.json"));
    assert!(names(&base, Removal::ModelReference).is_empty());
    let project = base.with_livelink(Some(LIVE_LINK.as_bytes())).unwrap();
    assert_eq!(names(&project, Removal::ModelReference), ["livelink.json"]);
    let copy = project
        .for_distribution(writer(), &[Removal::ModelReference])
        .unwrap();
    assert_eq!(copy.livelink().unwrap(), None);
    assert!(copy.distribution_inventory(&Removal::ALL).is_empty());
}

// ───────── テキストレイヤーのフォントの場所 ─────────

const FONT: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");
/// 利用者が入れたフォントの場所（Windows。ユーザー名を含む）と、ホームの隠しフォルダー（Linux）。
const FONT_WINDOWS: &str =
    "C:\\Users\\tester\\AppData\\Local\\Microsoft\\Windows\\Fonts\\Example Sans.ttf";
const FONT_LINUX: &str = "/home/tester/.fonts/Other Face.ttf";

fn file_font(path: &str, seed: u8) -> yolu_core::text::TextFont {
    yolu_core::text::TextFont::File {
        path: path.into(),
        index: 0,
        sha256: std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)),
        names: yolu_core::text::FontNames {
            family: format!("Family {seed}"),
            postscript: format!("Family{seed}-BoldItalic"),
            weight: 700,
            italic: true,
        },
    }
}

/// セット A はテキストレイヤーを 3 枚（Windows の道のフォント・Linux の道のフォント・同梱のフォント）、セット B は文字なし。
fn font_text_project() -> Project {
    use yolu_core::text::{TextFont, TextSettings};
    let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
    for (name, font) in [
        ("甲", file_font(FONT_WINDOWS, 1)),
        ("乙", file_font(FONT_LINUX, 2)),
        ("丙", TextFont::Bundled("biz-udpgothic".into())),
    ] {
        doc.add_text_layer(
            name,
            TextSettings::new("Text", font, 4.0, 40.0),
            FONT,
            None,
            false,
        )
        .unwrap();
    }
    Project::create(
        writer(),
        &[
            spec(SET_A, "Body", &doc),
            spec(SET_B, "Prop", &doc_reading(&[])),
        ],
        SET_A,
    )
    .unwrap()
}

/// セットの正本の、テキストレイヤーのフォントの項目（`font_kind`・`font_path`・番号・SHA-256・名前・太さ・斜体など。正本の並びのまま）。
fn fonts_of(project: &Project, set: &str) -> Vec<(String, yolu_io::NativeValue)> {
    project
        .sets()
        .iter()
        .find(|s| s.id == set)
        .unwrap()
        .document
        .to_native()
        .unwrap()
        .fields()
        .iter()
        .filter(|f| f.path.contains(".text.font_"))
        .map(|f| (f.path.clone(), f.value.clone()))
        .collect()
}

#[test]
fn the_font_path_of_a_text_layer_is_cut_to_the_file_name_and_the_rest_of_the_font_stays() {
    let project = font_text_project();
    let before = fonts_of(&project, SET_A);
    assert_eq!(
        before
            .iter()
            .filter(|(p, _)| p.ends_with(".font_path"))
            .count(),
        2,
        "ファイルのフォントの 2 枚"
    );
    // 目録: 素材の出どころのパスの種類に、フォントのファイル名が挙がる（道は挙げない）
    let listed = names(&project, Removal::SourcePaths);
    assert_eq!(listed, ["Example Sans.ttf", "Other Face.ttf"]);
    let copy = project
        .for_distribution(writer(), &[Removal::SourcePaths])
        .unwrap();
    let after = fonts_of(&copy, SET_A);
    assert_eq!(after.len(), before.len());
    for ((bp, bv), (ap, av)) in before.iter().zip(&after) {
        assert_eq!(bp, ap);
        if bp.ends_with(".font_path") {
            let (yolu_io::NativeValue::Text(was), yolu_io::NativeValue::Text(now)) = (bv, av)
            else {
                panic!()
            };
            assert!(!now.contains(['/', '\\']), "{now}");
            assert!(was.ends_with(now.as_str()), "{was} {now}");
        } else {
            // 番号・SHA-256・ファミリー名・PostScript 名・太さ・斜体と、同梱のフォントの名前は残る
            assert_eq!(bv, av, "{bp}");
        }
    }
    // 絵（画素・レイヤー）は同じ: 道のほかの項目は全部同じ
    let (was, now) = (
        project.sets()[0].document.to_native().unwrap(),
        copy.sets()[0].document.to_native().unwrap(),
    );
    assert_eq!(was.fields().len(), now.fields().len());
    for (a, b) in was.fields().iter().zip(now.fields()) {
        assert_eq!(a.path, b.path);
        if !a.path.ends_with(".text.font_path") {
            assert_eq!(a.value, b.value, "{}", a.path);
        }
    }
    // セット B（文字なし）の正本・合成は元のバイト列のまま
    let (x, y) = (entries(&project), entries(&copy));
    for leaf in ["document.utpaint", "composite/Color.png"] {
        let name = format!("sets/{SET_B}/{leaf}");
        assert_eq!(x[&name], y[&name], "{name}");
    }
    // 読み直せる
    let again = Project::read(&copy.to_bytes().unwrap()).unwrap();
    assert!(again.unknown_entries().is_empty());
    assert_eq!(fonts_of(&again, SET_A), after);
    // 写しを写しても、もう外す物は無い
    assert!(names(&again, Removal::SourcePaths).is_empty());
    // 外さない選びなら、道はそのまま
    let kept = project
        .for_distribution(writer(), &[Removal::UnityValues])
        .unwrap();
    assert_eq!(fonts_of(&kept, SET_A), before);
    // 開いているプロジェクトは変わらない
    assert_eq!(fonts_of(&project, SET_A), before);
}

#[test]
fn a_copy_has_no_path_of_the_author_in_any_entry() {
    let project = unity_style_with_text();
    let copy = project.for_distribution(writer(), &Removal::ALL).unwrap();
    let all = entries(&Project::read(&copy.to_bytes().unwrap()).unwrap());
    let mut text = String::new();
    for (name, bytes) in &all {
        text += name;
        text += &String::from_utf8_lossy(bytes);
    }
    for needle in [
        "tester",
        "C:\\\\Users",
        "C:\\Users",
        "/home/",
        "AppData",
        ".fonts",
        "Windows\\\\Fonts",
        "Unity Project",
    ] {
        assert!(!text.contains(needle), "{needle} が写しに残る");
    }
    for sentinel in SENTINELS {
        assert!(
            !text.contains(sentinel) && !text.contains(&sentinel.replace('\\', "\\\\")),
            "{sentinel}"
        );
    }
}

/// Unity 版が作ったような .ylp（`unity_style`）のセット A を、フォントのファイルを持つテキストレイヤーのある文書に替えたもの。
fn unity_style_with_text() -> Project {
    use yolu_core::text::TextSettings;
    let base = unity_style();
    let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
    doc.add_text_layer(
        "文字",
        TextSettings::new("Text", file_font(FONT_WINDOWS, 3), 4.0, 40.0),
        FONT,
        None,
        false,
    )
    .unwrap();
    base.with_document(SET_A, &NativeDocument::from_core(&doc).unwrap())
        .unwrap()
}
