use std::collections::BTreeMap;
use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, ChannelBlend, Document, HeightEdgeMode, NormalSettings,
    NormalYDirection,
};
use yolu_io::{Archive, NativeDocument, NativeValue, Project, Selection, WriterInfo};

use crate::legacy_layout;
use legacy_layout::{as_version, attribute_byte, NORMAL_SETTINGS_OFFSET};
fn files(n: usize) -> BTreeMap<String, Vec<u8>> {
    Archive::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format{n}.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
    .entries()
    .iter()
    .map(|(n, b)| (n.clone(), b.to_vec()))
    .collect()
}
fn open(f: BTreeMap<String, Vec<u8>>) -> yolu_io::Result<Project> {
    Project::read(&Archive::from_entries(f)?.to_bytes()?)
}
fn mutate_json(
    f: &mut BTreeMap<String, Vec<u8>>,
    name: &str,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let mut j = serde_json::from_slice(&f[name]).unwrap();
    change(&mut j);
    f.insert(name.into(), serde_json::to_vec(&j).unwrap());
}
#[test]
fn future_format_error_identifies_writer() {
    let mut f = files(6);
    mutate_json(&mut f, "ylp.json", |j| {
        j["format"] = 99.into();
        j["savedBy"]["app"] = "FuturePainter".into();
        j["savedBy"]["version"] = "9.0".into();
    });
    let e = open(f).unwrap_err().to_string();
    assert!(e.contains("99") && e.contains("FuturePainter") && e.contains("9.0"));
}
#[test]
fn invalid_json_and_duplicate_keys_are_refused() {
    for b in [
        b"{\"format\":2,\"format\":6}".to_vec(),
        b"{\xff}".to_vec(),
        vec![b' '; 65537],
        b"{\"format\":6.0}".to_vec(),
    ] {
        let mut f = files(6);
        f.insert("ylp.json".into(), b);
        assert!(open(f).is_err());
    }
}
#[test]
fn missing_document_bad_current_and_duplicate_sets_are_refused() {
    for case in 0..4 {
        let mut f = files(3);
        mutate_json(&mut f, "project.json", |j| match case {
            0 => j["current"] = "00000000-0000-0000-0000-000000000000".into(),
            1 => {
                let v = j["sets"][0].clone();
                j["sets"].as_array_mut().unwrap().push(v);
            }
            2 => j["sets"][0]["materialSlot"] = (-1).into(),
            _ => j["sets"][0]["id"] = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        });
        assert!(open(f).is_err());
    }
}
#[test]
fn unknown_entries_and_json_keys_survive_both_save_modes() {
    let mut f = files(3);
    f.insert("future.bin".into(), vec![7, 3, 9]);
    mutate_json(&mut f, "project.json", |j| j["future"] = "保持する".into());
    mutate_json(&mut f, "ylp.json", |j| {
        j["future"] = serde_json::json!({"mode":13})
    });
    let p = open(f.clone()).unwrap();
    assert!(p.unknown_entries().contains(&"future.bin".into()));
    // 知らせは種類で持つ（画面が言語ごとの文を作る）。形式 3 の読み込みはマテリアル参照のメモリ上の移行も知らせる
    assert!(p
        .notes()
        .contains(&yolu_io::Note::UnknownEntryKept("future.bin".into())));
    assert!(p
        .notes()
        .contains(&yolu_io::Note::MaterialRefsMigrated { format: 3 }));
    let a = Archive::read(&p.to_bytes().unwrap()).unwrap();
    for (k, v) in &f {
        assert_eq!(a.entries()[k].as_ref(), v);
    }
    let u = p
        .upgraded(WriterInfo {
            app: "test".into(),
            version: "1".into(),
            unity: "none".into(),
        })
        .unwrap();
    let a = Archive::read(&u.to_bytes().unwrap()).unwrap();
    assert_eq!(a.entries()["future.bin"].as_ref(), [7, 3, 9]);
    let project: serde_json::Value = serde_json::from_slice(&a.entries()["project.json"]).unwrap();
    assert_eq!(project["future"], "保持する");
    assert!(project["sets"][0].get("materialSlot").is_none());
    assert!(project["sets"][0].get("material").is_some());
    let info: serde_json::Value = serde_json::from_slice(&a.entries()["ylp.json"]).unwrap();
    assert_eq!(info["future"]["mode"], 13);
}
#[test]
fn legacy_material_slot_fallback_is_reported() {
    let mut f = files(1);
    f.insert("view.json".into(), b"{\"materialSlot\":-1}".to_vec());
    let p = open(f).unwrap();
    assert_eq!(p.sets()[0].material, yolu_io::MaterialRef::PendingSlot(0));
    assert!(p
        .notes()
        .iter()
        .any(|n| matches!(n, yolu_io::Note::ViewSlotUnreadable(_))
            && n.to_string().contains("スロット")));
}
#[test]
fn resource_unknown_kind_origin_and_missing_pixels_are_refused() {
    for case in 0..4 {
        let mut f = files(4);
        mutate_json(&mut f, "resources.json", |j| match case {
            0 => j["resources"][0]["kind"] = "future".into(),
            1 => j["resources"][0]["origin"] = serde_json::json!({"type":"future"}),
            2 => j["resources"][0]["content"] = "0".repeat(64).into(),
            _ => j["resources"][0]["width"] = 8193.into(),
        });
        assert!(open(f).is_err());
    }
}
#[test]
fn resource_pixels_hash_and_dimensions_are_verified() {
    for case in 0..2 {
        let mut f = files(4);
        if case == 0 {
            mutate_json(&mut f, "resources.json", |j| {
                j["resources"][0]["width"] = 8192.into()
            });
        } else {
            let png = f
                .keys()
                .find(|k| k.starts_with("resources/") && k.ends_with(".png"))
                .unwrap()
                .clone();
            f.get_mut(&png).unwrap()[0] = 0;
        }
        assert!(open(f).is_err());
    }
}
#[test]
fn resource_local_file_id_is_exact_and_unsafe_library_path_is_refused() {
    let mut f = files(4);
    mutate_json(
        &mut f,
        "resources.json",
        |j| j["resources"][0]["origin"] = serde_json::json!({"type":"unityAsset","guid":"0123456789abcdef0123456789abcdef","path":"Assets/Textures/Sample.png","localFileID":i64::MIN+37}),
    );
    let p = open(f.clone()).unwrap();
    assert_eq!(
        p.resources()[0].metadata["origin"]["localFileID"].as_i64(),
        Some(i64::MIN + 37)
    );
    mutate_json(
        &mut f,
        "resources.json",
        |j| j["resources"][0]["origin"] = serde_json::json!({"type":"library","file":"../bad","sha256":"a".repeat(64),"length":0}),
    );
    assert!(open(f).is_err());
}
#[test]
fn shared_smart_material_bytes_cannot_be_claimed_as_mask() {
    let mut f = files(5);
    mutate_json(&mut f, "resources.json", |j| {
        let resources = j["resources"].as_array_mut().unwrap();
        let mut duplicate = resources
            .iter()
            .find(|r| r["kind"] == "smartMaterial")
            .unwrap()
            .clone();
        duplicate["id"] = "aabbccdd-0000-4000-8000-000000000001".into();
        duplicate["kind"] = "smartMask".into();
        resources.push(duplicate);
    });
    assert!(open(f).is_err());
}
#[test]
fn native_unknown_values_and_invalid_ranges_are_refused() {
    let d = NativeDocument::read(include_bytes!("../fixtures/native-rich-v21.utpaint")).unwrap();
    let mut checked = 0;
    for f in d.fields() {
        let invalid = match &f.value {
            NativeValue::Float(_) => Some(NativeValue::Float(f64::NAN)),
            NativeValue::Int(_)
                if [
                    ".type",
                    ".algorithm",
                    ".blend",
                    ".kind",
                    ".channel",
                    ".anchor_read",
                    ".wrap",
                    ".mode",
                    ".shape",
                ]
                .iter()
                .any(|s| f.path.ends_with(s)) =>
            {
                Some(NativeValue::Int(9999))
            }
            _ => None,
        };
        if let Some(value) = invalid {
            assert!(d.with_value(&f.path, value).is_err(), "{}", f.path);
            checked += 1;
        }
    }
    assert!(checked > 150);
    for (path, v) in [
        ("layers[0].attributes", NativeValue::Byte(128)),
        ("layers[0].anchor_flags", NativeValue::Byte(4)),
        ("layers[0].locks", NativeValue::Int(16)),
        ("layers[1].gradients[0].blend", NativeValue::Int(0)),
    ] {
        assert!(d.with_value(path, v).is_err(), "{path}");
    }
}
#[test]
fn duplicate_layer_filter_anchor_and_parent_cycles_are_refused() {
    let d = NativeDocument::read(include_bytes!("../fixtures/native-rich-v21.utpaint")).unwrap();
    for (source, dest) in [
        ("layers[0].id", "layers[1].id"),
        (
            "layers[0].filters.items[0].id",
            "layers[0].mask.filters.items[0].id",
        ),
        ("layers[0].anchor.id", "layers[0].mask.anchor.id"),
        ("layers[0].id", "layers[0].parent"),
    ] {
        assert!(
            d.with_value(dest, d.field(source).unwrap().clone())
                .is_err(),
            "{dest}"
        );
    }
}
#[test]
fn every_truncated_rich_document_is_refused() {
    let b = include_bytes!("../fixtures/native-rich-v21.utpaint");
    let manual_offset = b.windows(4).rposition(|v| v == b"YLID").unwrap();
    for end in 0..b.len() {
        if end == manual_offset {
            assert!(NativeDocument::read(&b[..end]).is_ok());
        } else {
            assert!(NativeDocument::read(&b[..end]).is_err(), "{end}");
        }
    }
}
#[test]
fn selection_version_order_empty_padding_and_size_are_checked() {
    let d = NativeDocument::read(include_bytes!("../fixtures/native-v21.utpaint")).unwrap();
    let mut b = b"YLSL".to_vec();
    for v in [1i32, 9, 10, 8, 1, 1, 1] {
        b.extend(v.to_le_bytes());
    }
    let mut tile = vec![0; 64];
    tile[0] = 7;
    b.extend(tile);
    let good = Selection::read(&b, &d).unwrap();
    assert_eq!(good.to_bytes(), b);
    for (offset, value) in [(4, 2), (8, 16), (24, 2), (32, 0), (33, 1)] {
        let mut bad = b.clone();
        bad[offset] = value;
        assert!(Selection::read(&bad, &d).is_err(), "{offset}");
    }
    b.push(0);
    assert!(Selection::read(&b, &d).is_err());
}

/// アルファ以外が 0 でないマスクのタイルは正本の読み込みで断る（ネイティブの straight RGBA8 の約束。C# の
/// MaskTests.NativeReaderRefusesMaskTilesWithColour）。core の `import_mask_tile` も同じ約束を持つ（yolu-core/tests/edit/layers.rs）が、
/// 正本の読み手が先に断るので、壊れた正本は core の文書にならない。隠す量（アルファ）の変更は断らない。
#[test]
fn a_native_mask_tile_with_colour_is_refused() {
    let d = NativeDocument::read(include_bytes!("../fixtures/m2-masks.utpaint")).unwrap();
    d.to_core().unwrap();
    let paths: Vec<String> = d
        .fields()
        .iter()
        .filter(|f| f.path.contains(".mask.tiles[") && f.path.ends_with(".rgba"))
        .map(|f| f.path.clone())
        .collect();
    assert!(paths.len() >= 10, "マスクのタイルのあるレイヤーが 2 つ");
    let mut refused = 0;
    for path in &paths {
        let NativeValue::Bytes(bytes) = d.field(path).unwrap() else {
            panic!("{path}")
        };
        let pixels = bytes.len() / 4;
        for pixel in [0, 77, pixels - 1] {
            for channel in 0..3 {
                assert_eq!(
                    bytes[pixel * 4 + channel],
                    0,
                    "{path}: 正本のマスクの RGB は 0"
                );
                let mut bad = bytes.to_vec();
                bad[pixel * 4 + channel] = 1;
                let err = d
                    .with_value(path, NativeValue::Bytes(bad.into()))
                    .expect_err(&format!("{path} 画素 {pixel} の {channel}"))
                    .to_string();
                assert!(
                    err.contains("マスク") && err.contains("RGB"),
                    "{path}: {err}"
                );
                refused += 1;
            }
        }
        // アルファ（隠す量）は変えてよい: 読めて、core の文書になり、書き戻しも同じ
        let mut alpha = bytes.to_vec();
        alpha[3] = 77; // 先頭の画素（キャンバスの中。右と上の余白はアルファも 0 でなければならない）
        let changed = d
            .with_value(path, NativeValue::Bytes(alpha.into()))
            .unwrap();
        assert_eq!(
            NativeDocument::from_core(&changed.to_core().unwrap())
                .unwrap()
                .to_bytes(),
            changed.to_bytes(),
            "{path}"
        );
    }
    assert_eq!(refused, paths.len() * 9);
}

/// 選択範囲の読み手が断るもののうち、識別子の違い・途中で切れたファイル・タイル数・同じ座標や逆順の 2 枚目（C# の
/// SelectionPersistenceTests.AnythingItDoesNotWriteIsRefused の magic・count・order・truncated・short）。断る理由は種類ごとに違う文で言う。
#[test]
fn selection_magic_truncation_tile_count_and_tile_order_are_refused() {
    let d = NativeDocument::read(include_bytes!("../fixtures/native-v21.utpaint")).unwrap();
    assert_eq!(
        (d.width(), d.height(), d.tile_size()),
        (9, 10, 8),
        "2 × 2 のタイル"
    );
    let selection = |tiles: &[(i32, i32)]| {
        let mut b = b"YLSL".to_vec();
        for v in [1i32, 9, 10, 8, tiles.len() as i32] {
            b.extend(v.to_le_bytes());
        }
        for &(x, y) in tiles {
            b.extend(x.to_le_bytes());
            b.extend(y.to_le_bytes());
            let mut amounts = vec![0u8; 64];
            // キャンバスの中（幅 9・高さ 10）の画素にだけ量を入れる
            for (i, a) in amounts.iter_mut().enumerate() {
                if x * 8 + (i as i32 % 8) < 9 && y * 8 + (i as i32 / 8) < 10 {
                    *a = 7 + i as u8;
                }
            }
            b.extend(amounts);
        }
        b
    };
    let refuse = |bytes: &[u8], what: &str, message: &str| {
        let err = Selection::read(bytes, &d).expect_err(what).to_string();
        assert!(err.contains(message), "{what}: {err}");
    };
    let good = selection(&[(0, 0), (1, 0)]);
    let read = Selection::read(&good, &d).unwrap();
    assert_eq!(read.to_bytes(), good);
    assert_eq!(read.tiles().len(), 2);
    // 識別子
    let mut bad = good.clone();
    bad[0] = b'X';
    refuse(&bad, "識別子の違い", "識別子");
    refuse(&[], "空", "識別子");
    refuse(&good[..3], "識別子より短い", "識別子");
    // 途中で切れた・長すぎる: どの長さでも断る。ヘッダーの途中は「途中で切れています」、そのあとは長さの不一致
    for n in 0..good.len() {
        assert!(
            Selection::read(&good[..n], &d).is_err(),
            "{n} バイトで切れたもの"
        );
    }
    refuse(&good[..10], "ヘッダーの途中", "途中で切れて");
    refuse(&good[..good.len() - 1], "最後の 1 バイトを欠く", "長さ");
    refuse(&good[..24 + 72], "2 枚目をまるごと欠く", "長さ");
    // タイルの数: キャンバスのタイル数（2 × 2）より多い・負・宣言と中身が合わない
    for count in [5i32, 21, i32::MAX, -1, i32::MIN] {
        let mut bad = good.clone();
        bad[20..24].copy_from_slice(&count.to_le_bytes());
        refuse(&bad, &format!("タイル数 {count}"), "タイル数");
    }
    for count in [1i32, 3] {
        let mut bad = good.clone();
        bad[20..24].copy_from_slice(&count.to_le_bytes());
        refuse(&bad, &format!("宣言 {count} と中身（2 枚）"), "長さ");
    }
    // 並び: 2 枚目が 1 枚目と同じ座標・逆順・行をまたいで逆順は断る。(y, x) の昇順は通る
    let mut same = good.clone();
    same[24 + 72..24 + 72 + 8].copy_from_slice(&good[24..32]);
    refuse(&same, "同じ座標の 2 枚目", "位置または並び");
    refuse(&selection(&[(1, 0), (0, 0)]), "逆順", "位置または並び");
    refuse(
        &selection(&[(0, 1), (1, 0)]),
        "行をまたいで逆順",
        "位置または並び",
    );
    refuse(&selection(&[(1, 1), (1, 1)]), "同じ座標", "位置または並び");
    for ok in [
        vec![(0, 0), (1, 1)],
        vec![(1, 0), (0, 1)],
        vec![(0, 0), (1, 0), (0, 1), (1, 1)],
    ] {
        assert_eq!(
            Selection::read(&selection(&ok), &d).unwrap().tiles().len(),
            ok.len()
        );
    }
}

// ───────── 壊れた設定は落とさずに断る・旧版の並びの断り ─────────

fn save(doc: &Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}
/// 読めないバイト列の断りの文（読めてしまったら、そのことだけを言って落ちる）。
fn refusal(bytes: &[u8], what: &str) -> String {
    match NativeDocument::read(bytes) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("{what}: 断るはずが読めてしまった"),
    }
}

/// チャンネルごとの合成の設定が壊れていたら、黙って落とさず断る（C# の ChannelBlendTests.BrokenSettingsAreRefusedInsteadOfDropped）。
/// 並び: レイヤーの属性の 1 バイト（ビット 2 が設定あり）、設定の数 1 バイト、設定ごとに チャンネル 4・部品の印 1・モード 4・不透明度 8。
#[test]
fn broken_channel_blend_settings_are_refused_instead_of_dropped() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let l = d.add_layer("L").unwrap();
    d.set_channel_blend(
        l,
        Channel::Roughness,
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.25)),
        false,
    )
    .unwrap();
    let bytes = save(&d);
    let at = attribute_byte("L");
    assert_eq!(bytes[at], 4, "属性のビット 2");
    assert_eq!(bytes[at + 1], 1, "設定は 1 つ");
    assert_eq!(
        i32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap()),
        Channel::Roughness.index() as i32
    );
    assert_eq!(bytes[at + 6], 3, "モードと不透明度");
    assert_eq!(
        i32::from_le_bytes(bytes[at + 7..at + 11].try_into().unwrap()),
        BlendMode::Multiply as i32
    );
    assert_eq!(
        f64::from_le_bytes(bytes[at + 11..at + 19].try_into().unwrap()),
        0.25
    );
    NativeDocument::read(&bytes).unwrap();
    let with = |change: &dyn Fn(&mut Vec<u8>)| {
        let mut b = bytes.clone();
        change(&mut b);
        b
    };
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "空の一覧は書かれない",
            with(&|b| b[at + 1] = 0),
            "チャンネル合成数",
        ),
        (
            "チャンネルの数より多い",
            with(&|b| b[at + 1] = 7),
            "チャンネル合成数",
        ),
        (
            "未知のチャンネル",
            with(&|b| b[at + 2..at + 6].copy_from_slice(&42i32.to_le_bytes())),
            "channel",
        ),
        ("部品が無い", with(&|b| b[at + 6] = 0), "合成属性"),
        ("未知の部品の印", with(&|b| b[at + 6] = 7), "合成属性"),
        (
            "未知のモード",
            with(&|b| b[at + 7..at + 11].copy_from_slice(&99i32.to_le_bytes())),
            "mode",
        ),
        (
            "通過をレイヤーに",
            with(&|b| {
                b[at + 7..at + 11].copy_from_slice(&(BlendMode::PassThrough as i32).to_le_bytes())
            }),
            "通過合成はグループだけ",
        ),
        (
            "不透明度が 1 より大きい",
            with(&|b| b[at + 11..at + 19].copy_from_slice(&1.5f64.to_le_bytes())),
            "opacity",
        ),
        (
            "不透明度が NaN",
            with(&|b| b[at + 11..at + 19].copy_from_slice(&f64::NAN.to_le_bytes())),
            "opacity",
        ),
        ("途中で切れた", bytes[..at + 9].to_vec(), ""),
    ];
    for (what, b, message) in cases {
        let err = refusal(&b, what);
        assert!(err.contains(message), "{what}: {err}");
    }
    // 同じチャンネルが 2 回
    d.set_channel_blend(
        l,
        Channel::Height,
        ChannelBlend::new(None, Some(0.5)),
        false,
    )
    .unwrap();
    let two = save(&d);
    assert_eq!(two[at + 1], 2);
    let second = at + 2 + 4 + 1 + 4 + 8;
    assert_eq!(
        i32::from_le_bytes(two[second..second + 4].try_into().unwrap()),
        Channel::Height.index() as i32
    );
    let mut dup = two.clone();
    dup[second..second + 4].copy_from_slice(&(Channel::Roughness.index() as i32).to_le_bytes());
    assert!(refusal(&dup, "同じチャンネルが 2 回").contains("重複"));
}

/// Normal の出力設定が壊れていたら断る（アルゴリズム版・強さの範囲と非数・端・向き・途中で切れた設定。C# の NativeArchiveRoundTripsNormalSettings の後半）。
#[test]
fn broken_normal_settings_are_refused() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    d.add_layer("p").unwrap();
    d.set_normal_settings(
        NormalSettings::new(true, 12.5, HeightEdgeMode::Wrap, NormalYDirection::DirectX).unwrap(),
        false,
    )
    .unwrap();
    let bytes = save(&d);
    NativeDocument::read(&bytes).unwrap();
    let at = NORMAL_SETTINGS_OFFSET;
    let tampered = |offset: usize, value: &[u8]| {
        let mut b = bytes.clone();
        b[at + offset..at + offset + value.len()].copy_from_slice(value);
        b
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("アルゴリズムの版が新しい", tampered(0, &2i32.to_le_bytes())),
        ("アルゴリズムの版が 0", tampered(0, &0i32.to_le_bytes())),
        ("強さが NaN", tampered(5, &f64::NAN.to_le_bytes())),
        ("強さが無限大", tampered(5, &f64::INFINITY.to_le_bytes())),
        ("強さが範囲外", tampered(5, &1e6f64.to_le_bytes())),
        ("強さが範囲のすぐ外", tampered(5, &256.5f64.to_le_bytes())),
        ("未知の端の扱い", tampered(13, &7i32.to_le_bytes())),
        ("未知の向き", tampered(17, &(-1i32).to_le_bytes())),
        ("途中で切れた設定", bytes[..at + 10].to_vec()),
    ];
    for (what, b) in cases {
        refusal(&b, what);
    }
    // 範囲の端は読める
    for strength in [-256.0f64, 0.0, 256.0] {
        NativeDocument::read(&tampered(5, &strength.to_le_bytes())).unwrap();
    }
}

/// 版の数だけ変えた並びは、その版が持たない中身があれば断る（読み手は版ごとの並びで読み、持たない印を黙って無視しない）。
/// チャンネルごとの合成（版 14）は版 13 以前、調整（版 4）は版 3 の並びでは読めない。
#[test]
fn a_layout_older_than_what_the_document_holds_is_refused() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let l = d.add_layer("L").unwrap();
    d.set_channel_blend(
        l,
        Channel::Roughness,
        ChannelBlend::new(Some(BlendMode::Screen), None),
        false,
    )
    .unwrap();
    let bytes = save(&d);
    assert_eq!(NativeDocument::read(&bytes).unwrap().version(), 21);
    for version in [20, 19, 15, 14] {
        let older = as_version(&bytes, "L", version);
        assert_eq!(
            NativeDocument::read(&older).unwrap().version(),
            version,
            "版 {version}はまだ設定を持つ"
        );
    }
    for version in [13i32, 12, 11, 10, 9] {
        let mut older = bytes.clone();
        older[8..12].copy_from_slice(&version.to_le_bytes());
        let err = refusal(&older, &format!("版 {version}"));
        if version >= 12 {
            // 属性の 1 バイトの並び。設定の印（ビット 2）は版 14 から
            assert!(err.contains("属性"), "版 {version}: {err}");
        }
    }
    // 調整レイヤーは版 3 の並び（種類が塗りつぶしまで）では読めない
    let mut adjusted = Document::with_tile_size(16, 16, 8).unwrap();
    adjusted
        .add_adjustment_layer("A", AdjustmentSettings::invert(), None, None)
        .unwrap();
    adjusted.clear_history().unwrap();
    let bytes = save(&adjusted);
    NativeDocument::read(&bytes).unwrap();
    let v3 = as_version(&bytes, "A", 3);
    assert!(refusal(&v3, "版 3 に調整レイヤー").contains("kind"));
    // 4 以上なら調整を持てる
    NativeDocument::read(&as_version(&bytes, "A", 4)).unwrap();
    // 設定を持たない文書は、版の数だけ古くても読める（読み書きは compatibility.rs）
    let mut plain = Document::with_tile_size(16, 16, 8).unwrap();
    plain.add_layer("L").unwrap();
    let plain = save(&plain);
    for version in [13, 12] {
        NativeDocument::read(&as_version(&plain, "L", version)).unwrap();
    }
}

/// 全部入りの版 21 の正本（デカール・ID の色の Generator・形のグラデーション・塗りつぶしの画像・パス・Anchor・ロックなど）は、どの古い版の
/// 並びとしても読めない（版の数だけを変えても、その版が持たない中身を黙って無視して読まない）。
#[test]
fn the_rich_document_is_refused_under_every_older_version_label() {
    let rich = NativeDocument::read(include_bytes!("../fixtures/native-rich-v21.utpaint")).unwrap();
    for version in 1..=20 {
        assert!(
            rich.with_value("version", NativeValue::Int(version))
                .is_err(),
            "版 {version}"
        );
    }
    rich.with_value("version", NativeValue::Int(21)).unwrap();
}
