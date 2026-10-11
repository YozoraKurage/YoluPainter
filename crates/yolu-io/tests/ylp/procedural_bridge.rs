//! Rust 版だけの Generator の種類（ノイズ 64・グランジ 65）の保存・復元（正本の版 23）。C# に対応する書き手は無いので、
//! 正解ファイルは無く、往復・版の選び方・読み手の拒否・.ylp への保存を試す。
use yolu_core::generator::{
    Blend, CellOutput, FractalMode, GrungePreset, Kind, MapKind, NoiseBasis, ProceduralSpace,
    Settings,
};
use yolu_core::{
    Channel, ChannelInfo, ChannelKind, ColorSpace, Document, EffectSettings, FilterId, FilterSpec,
    FilterTarget, LayerId, Rgba8,
};
use yolu_io::smart::{SmartFile, REFUSAL_RUST_GENERATORS};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, MAX_NATIVE_VERSION,
    PROCEDURAL_VERSION, UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn busy_noise() -> Settings {
    let mut g = Settings::new(Kind::Noise);
    g.blend = Blend::Replace;
    g.low = 0.2;
    g.high = 0.9;
    g.softness = 0.3;
    g.invert = true;
    g.procedural.space = ProceduralSpace::Triplanar;
    g.procedural.scale = 0.37;
    g.procedural.seed = -123456;
    g.procedural.rotation = [10.5, -20.25, 359.];
    g.procedural.bleed = 0.625;
    g.procedural.blend_width = 0.8;
    g.procedural.basis = NoiseBasis::Worley;
    g.procedural.cell_output = CellOutput::F2MinusF1;
    g.procedural.fractal = FractalMode::Turbulence;
    g.procedural.octaves = 7;
    g.procedural.lacunarity = 2.75;
    g.procedural.gain = 0.41;
    g.pins.insert(MapKind::Position, "c".repeat(64));
    g.pins.insert(MapKind::WorldNormal, "d".repeat(64));
    g
}
fn busy_grunge() -> Settings {
    let mut g = Settings::grunge(GrungePreset::Fingerprints);
    g.blend = Blend::Multiply;
    g.procedural.space = ProceduralSpace::Uv;
    g.procedural.scale = 0.0625;
    g.procedural.seed = i32::MAX;
    g.procedural.bleed = 1.;
    g
}

/// 40×28 の文書: 塗りつぶしレイヤーの内容にノイズ（Color）、マスクにグランジ。UV の設定にして、入力なしでも合成が決まる。
fn document(uv_only: bool) -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(180, 90, 40, 255))],
            None,
        )
        .unwrap();
    let mut noise = busy_noise();
    let mut grunge = busy_grunge();
    if uv_only {
        noise.procedural.space = ProceduralSpace::Uv;
        noise.pins.clear();
    }
    grunge.procedural.space = ProceduralSpace::Uv;
    doc.add_filter(
        fill,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(noise))
            .channels(&[Channel::Color])
            .with_id(FilterId(0xF001)),
    )
    .unwrap();
    doc.add_layer_mask(fill).unwrap();
    doc.add_filter(
        fill,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::generator(grunge)).with_id(FilterId(0xF002)),
    )
    .unwrap();
    // 固定の ID（正解の正本が毎回同じバイト列になる）
    let ids: Vec<LayerId> = (0..doc.layers().len())
        .map(|i| LayerId(0x7000 + i as u128))
        .collect();
    doc.with_persistent_ids(0x7777, &ids).unwrap()
}
fn plain() -> Document {
    let mut doc = Document::with_tile_size(40, 28, 8).unwrap();
    doc.add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    doc
}
fn user_channel(doc: &mut Document) {
    doc.add_channel(ChannelInfo {
        name: "傷".into(),
        kind: ChannelKind::Scalar,
        color_space: ColorSpace::Linear,
        default: Rgba8::new(0, 0, 0, 255),
    })
    .unwrap();
}
fn stages(doc: &Document) -> Vec<(bool, Settings)> {
    let mut out = Vec::new();
    for l in doc.layers() {
        for e in l.filters() {
            out.push((false, e.settings().generator_settings().unwrap().clone()));
        }
        for e in l.mask().into_iter().flat_map(|m| m.filters()) {
            out.push((true, e.settings().generator_settings().unwrap().clone()));
        }
    }
    out
}

#[test]
fn the_version_follows_the_features_used() {
    assert_eq!(UNITY_NATIVE_VERSION, 21);
    assert_eq!(USER_CHANNELS_VERSION, 22);
    assert_eq!(PROCEDURAL_VERSION, 23);
    // 版 24（色調補正の 6 種）は adjust_bridge の試験が固定する
    assert_eq!(yolu_io::EFFECTS_VERSION, 28);
    assert_eq!(MAX_NATIVE_VERSION, yolu_io::RULERS_VERSION);
    // Unity 版と同じ機能だけなら版 21 のまま（Unity 0.2.0 が読める）
    assert_eq!(
        NativeDocument::from_core(&plain()).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
    // ユーザーチャンネルだけなら 22
    let mut with_user = plain();
    user_channel(&mut with_user);
    assert_eq!(
        NativeDocument::from_core(&with_user).unwrap().version(),
        USER_CHANNELS_VERSION
    );
    // Rust 版だけの Generator があれば 23（ユーザーチャンネルがあっても無くても）
    let doc = document(false);
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        PROCEDURAL_VERSION
    );
    let mut both = document(false);
    user_channel(&mut both);
    let native = NativeDocument::from_core(&both).unwrap();
    assert_eq!(native.version(), PROCEDURAL_VERSION);
    assert_eq!(
        native.to_core().unwrap().channels().len(),
        7,
        "ユーザーチャンネルも版 23 で残る"
    );
    // 既存の種類の Generator だけの文書は版 21
    let mut old = plain();
    let id = old.layers()[0].id();
    old.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(Settings::new(
            Kind::PositionGradient,
        )))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    assert_eq!(
        NativeDocument::from_core(&old).unwrap().version(),
        UNITY_NATIVE_VERSION
    );
}

#[test]
fn every_setting_survives_the_native_round_trip_byte_for_byte() {
    let doc = document(false);
    let native = NativeDocument::from_core(&doc).unwrap();
    let back = native.to_core().unwrap();
    assert_eq!(stages(&back), stages(&doc), "設定が往復で変わった");
    // 書き直しても同じバイト（正本 → core → 正本）
    let again = NativeDocument::from_core(&back).unwrap();
    assert_eq!(again.to_bytes(), native.to_bytes());
    // 読み直し（バイト列 → 正本）も同じ
    let reread = NativeDocument::read(&native.to_bytes()).unwrap();
    assert_eq!(reread.to_bytes(), native.to_bytes());
    assert_eq!(reread.version(), PROCEDURAL_VERSION);
    // 無効にした段・強さ・ID も残る
    let mut edited = document(false);
    let layer = edited.layers()[0].id();
    let stage = edited.layers()[0].filters()[0].id();
    edited.set_filter_enabled(layer, stage, false).unwrap();
    edited
        .set_filter_strength(layer, stage, 0.375, false)
        .unwrap();
    let back = NativeDocument::from_core(&edited)
        .unwrap()
        .to_core()
        .unwrap();
    assert!(!back.layers()[0].filters()[0].enabled());
    assert_eq!(back.layers()[0].filters()[0].strength(), 0.375);
    assert_eq!(back.layers()[0].filters()[0].id(), stage);
}

#[test]
fn composites_are_identical_after_a_round_trip() {
    let doc = document(true);
    let back = NativeDocument::from_core(&doc).unwrap().to_core().unwrap();
    let all = yolu_core::Rect::new(0, 0, 40, 28);
    assert_eq!(
        doc.composite_channel(Channel::Color, all).unwrap(),
        back.composite_channel(Channel::Color, all).unwrap()
    );
    // 効果が入っている（段を外した文書と違う）
    let mut bare = document(true);
    let layer = bare.layers()[0].id();
    let ids: Vec<_> = bare.layers()[0].filters().iter().map(|e| e.id()).collect();
    for id in ids {
        bare.remove_filter(layer, id).unwrap();
    }
    assert_ne!(
        doc.composite_channel(Channel::Color, all).unwrap(),
        bare.composite_channel(Channel::Color, all).unwrap()
    );
}

fn field_paths(native: &NativeDocument, tail: &str) -> Vec<String> {
    native
        .fields()
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| p.ends_with(tail))
        .collect()
}

#[test]
fn the_reader_refuses_what_the_format_does_not_allow() {
    let native = NativeDocument::from_core(&document(false)).unwrap();
    let edit = |tail: &str, value: NativeValue| {
        let path = field_paths(&native, tail)
            .into_iter()
            .find(|p| p.contains("filters") && p.contains("generator"))
            .unwrap_or_else(|| panic!("{tail}"));
        native.with_value(&path, value)
    };
    // 空けてある番号・知らない番号
    for kind in [8, 9, 30, 63, 66, 100, -1] {
        assert!(
            edit("generator.type", NativeValue::Int(kind)).is_err(),
            "種類 {kind}"
        );
    }
    // 範囲外の欄
    for (tail, v) in [
        ("procedural.space", NativeValue::Int(3)),
        ("procedural.scale", NativeValue::Float(0.0)),
        ("procedural.scale", NativeValue::Float(2.0)),
        ("procedural.rotation_x", NativeValue::Float(361.0)),
        ("procedural.bleed", NativeValue::Float(1.5)),
        ("procedural.blend_width", NativeValue::Float(-0.1)),
        ("procedural.basis", NativeValue::Int(3)),
        ("procedural.cell_output", NativeValue::Int(3)),
        ("procedural.fractal", NativeValue::Int(3)),
        ("procedural.octaves", NativeValue::Int(0)),
        ("procedural.octaves", NativeValue::Int(9)),
        ("procedural.lacunarity", NativeValue::Float(0.9)),
        ("procedural.lacunarity", NativeValue::Float(4.1)),
        ("procedural.gain", NativeValue::Float(1.1)),
        ("procedural.gain", NativeValue::Float(f64::NAN)),
    ] {
        assert!(edit(tail, v.clone()).is_err(), "{tail} {v:?}");
    }
    // 種類を入れ替えると欄の並びが合わなくなる（Anchor に変えるなど）ので断る
    assert!(edit("generator.type", NativeValue::Int(7)).is_err());
    assert!(edit("generator.type", NativeValue::Int(65)).is_err());
    // 範囲内の変更は通る
    assert!(edit("procedural.octaves", NativeValue::Int(8)).is_ok());
    assert!(edit("procedural.gain", NativeValue::Float(0.0)).is_ok());
}

#[test]
fn a_version_21_document_cannot_carry_the_new_kinds() {
    // Unity 版が書く版 21 の正本は、種類 64・65 を持たない。版 21 のまま種類だけを変えても通らない
    let mut old = plain();
    let id = old.layers()[0].id();
    old.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(Settings::new(
            Kind::PositionGradient,
        )))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    let native = NativeDocument::from_core(&old).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    let path = field_paths(&native, "generator.type").remove(0);
    for kind in [64, 65] {
        assert!(native.with_value(&path, NativeValue::Int(kind)).is_err());
    }
    // 版の数だけを 23 に書き換えた正本は、並び（ユーザーチャンネルの数が増える）が合わず読めない
    let mut bytes = native.to_bytes();
    bytes[8..12].copy_from_slice(&PROCEDURAL_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 22 のまま種類 64 を書いた正本も断る（版 22 の意味は変えない）
    let doc = document(false);
    let mut bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    bytes[8..12].copy_from_slice(&USER_CHANNELS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
}

#[test]
fn a_ylp_keeps_the_stages_and_the_version() {
    let doc = document(false);
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000001".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-procedural-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("procedural.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), PROCEDURAL_VERSION);
    assert_eq!(reopened.to_bytes().unwrap(), native.to_bytes());
    assert_eq!(stages(&reopened.to_core().unwrap()), stages(&doc));
    // 開いたまま何も変えずに書き直しても、正本のバイトは変わらない
    SaveTarget::create(dir.join("again.ylp"))
        .unwrap()
        .save(&again)
        .unwrap();
    let (third, _) = SaveTarget::open(dir.join("again.ylp")).unwrap();
    assert_eq!(
        third.sets()[0].document.to_bytes().unwrap(),
        native.to_bytes()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn smart_files_do_not_carry_rust_only_generators() {
    // .ylsmart の形式 1 は Unity 版と共有なので、Rust 版だけの種類は保存せず、理由を言って断る（ユーザーチャンネルと同じ扱い）
    let mut doc = document(true);
    let layer = doc.layers()[0].id();
    let material = doc.capture_smart_material(&[layer], "素材").unwrap();
    let err = SmartFile::from_core(&material, &writer()).unwrap_err();
    assert!(err.to_string().contains(REFUSAL_RUST_GENERATORS), "{err}");
    // 無効にした段も同じ（段が残っている限り、開いた Unity 版が読めない版になる）
    let stage = doc.layers()[0].filters()[0].id();
    doc.set_filter_enabled(layer, stage, false).unwrap();
    let material = doc.capture_smart_material(&[layer], "素材").unwrap();
    assert!(SmartFile::from_core(&material, &writer()).is_err());
    // 既存の種類だけの素材は今までどおり書ける
    let old = plain();
    let id = old.layers()[0].id();
    let material = old.capture_smart_material(&[id], "素材").unwrap();
    assert!(SmartFile::from_core(&material, &writer()).is_ok());
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// 版 23 の正解の正本（Rust が書いたもの。固定の ID で作り、ファイルと同じバイト列になる）。
/// 作り直すときは `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp procedural_bridge::`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --procedural` で Unity 版の読み手の記録を取り直す。
#[test]
fn version_23_fixture_is_what_this_writer_produces() {
    let bytes = NativeDocument::from_core(&document(false))
        .unwrap()
        .to_bytes();
    let path = format!(
        "{}/tests/fixtures/procedural-v23.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    assert_eq!(bytes[8..12], 23i32.to_le_bytes());
}

#[test]
fn unity_0_2_0_reader_refuses_version_23_before_touching_the_file() {
    // Unity 0.2.0 の DocumentBinary・YlpFormat に Rust が書いた版 23 を読ませた記録（tools/io-fixtures --procedural）。
    // 版 22 と同じく「Unsupported archive version」で断り、Unity 版は種類 64・65 を別の意味で読まない
    let record = String::from_utf8(read_fixture("procedural-v23.unity.txt")).unwrap();
    let lines: Vec<&str> = record.lines().collect();
    assert_eq!(lines[0], "DocumentBinary.CurrentVersion: 21");
    assert_eq!(lines[1], "YlpFormat.Current: 7");
    let refusal = "InvalidDataException: Unsupported archive version; source retained unchanged.";
    assert_eq!(lines[2], format!("DocumentBinary.Read: {refusal}"));
    assert_eq!(lines[3], format!("DocumentBinary.ReadId: {refusal}"));
    assert_eq!(lines[4], "YlpFormat.Open: OK");
    assert!(
        lines[5].starts_with("TexturePaintWindow.ReadTextureSets（文を組み立てたもの。ウィンドウは未実行）: InvalidDataException: Texture set \"")
            && lines[5].ends_with("\": Unsupported archive version; source retained unchanged."),
        "{}",
        lines[5]
    );
    assert_eq!(lines.len(), 6);
    // 記録した正本は、この書き手が今書くものと同じ（記録が古くなっていない）
    assert_eq!(
        read_fixture("procedural-v23.utpaint"),
        NativeDocument::from_core(&document(false))
            .unwrap()
            .to_bytes()
    );
}

/// 同梱のスマートマテリアルを置いた文書は、保存して読み直すと、レイヤー・Generator の設定・全チャンネルの合成が同じ（版 23）。
#[test]
fn bundled_materials_survive_save_and_reopen() {
    use yolu_core::smart::SmartPlacement;
    use yolu_core::smart_library;
    let all = yolu_core::Rect::new(0, 0, 96, 64);
    for e in smart_library::entries() {
        let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
        let m = e.build(e.en, false).unwrap();
        doc.place_smart_material(&m, &SmartPlacement::default())
            .unwrap();
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), PROCEDURAL_VERSION, "{}", e.id);
        let back = native.to_core().unwrap();
        assert_eq!(stages(&back), stages(&doc), "{}", e.id);
        // 標準の 6 チャンネルすべて
        for c in Channel::ALL {
            assert_eq!(
                doc.composite_channel(c, all).unwrap(),
                back.composite_channel(c, all).unwrap(),
                "{} {c:?}",
                e.id
            );
        }
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes(),
            "{}: 正本の往復",
            e.id
        );
    }
}

/// 同梱の素材の絵（`YOLU_LIBRARY_THUMBS=<ディレクトリ>`。256 四方に置いた Color の PNG）。ふだんは何もしない。
#[test]
fn dump_bundled_thumbnails() {
    use yolu_core::smart::SmartPlacement;
    use yolu_core::smart_library;
    let Ok(dir) = std::env::var("YOLU_LIBRARY_THUMBS") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for e in smart_library::entries() {
        let mut doc = Document::with_tile_size(256, 256, 64).unwrap();
        let m = e.build(e.en, false).unwrap();
        doc.place_smart_material(&m, &SmartPlacement::default())
            .unwrap();
        let png = yolu_io::composite_png(&doc).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("{}.png", e.id)),
            png,
        )
        .unwrap();
    }
}
