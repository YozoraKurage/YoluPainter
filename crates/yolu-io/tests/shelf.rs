use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};
use yolu_core::{
    smart::{SmartPlacement, SmartResampling},
    Channel, ChannelInfo, ChannelKind, ColorSpace, CoreError, Document, Rgba8, TileCoord,
};
use yolu_io::{shelf::Shelf, smart::SmartFile, Archive, Project, WriterInfo, UNITY_NATIVE_VERSION};
fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter".into(),
        version: "test".into(),
        unity: "none".into(),
    }
}
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smart")
}
fn files() -> BTreeMap<String, Arc<[u8]>> {
    let index = std::fs::read(root().join("resources.json")).unwrap();
    let value: Value = serde_json::from_slice(&index).unwrap();
    let mut f = BTreeMap::from([("resources.json".into(), Arc::from(index))]);
    for r in value["resources"].as_array().unwrap() {
        let name = format!("resources/{}.png", r["content"].as_str().unwrap());
        f.insert(
            name.clone(),
            Arc::from(std::fs::read(root().join(name)).unwrap()),
        );
    }
    f
}
#[test]
fn csharp_index_and_embedded_copies_are_byte_exact() {
    let f = files();
    let shelf = Shelf::read(&f, 1024).unwrap();
    assert_eq!(
        shelf.canonical_index().unwrap(),
        f["resources.json"].as_ref()
    );
    assert_eq!(shelf.entries(), &f);
    assert_eq!(shelf.used_bytes(), 12);
}
#[test]
fn csharp_smart_files_keep_every_byte() {
    for name in ["raster", "mask", "multi"] {
        let bytes = std::fs::read(root().join(format!("{name}.ylsmart"))).unwrap();
        let smart = SmartFile::read(&bytes).unwrap();
        assert_eq!(smart.file_bytes(), bytes);
        assert_eq!(
            smart.fragment().to_bytes(),
            smart.entries()["layers.utpaint"].as_ref()
        );
    }
}
#[test]
fn csharp_placement_resampling_matches_twelve_cases() {
    let mut d = Document::with_tile_size(3, 2, 8).unwrap();
    let l = d.add_layer("画素").unwrap();
    for y in 0..2 {
        for x in 0..3 {
            d.set_pixel(
                l,
                x,
                y,
                Rgba8::new(
                    (x * 71 + y * 13) as u8,
                    (x * 9 + y * 81) as u8,
                    (x * 37 + y * 11) as u8,
                    if (x + y) % 3 == 0 {
                        0
                    } else {
                        (50 + x * 60 + y * 20) as u8
                    },
                ),
            )
            .unwrap();
        }
    }
    let material = d.capture_smart_material(&[l], "試験素材").unwrap();
    for (w, h) in [(3, 2), (7, 5), (2, 1), (2, 5)] {
        for (i, how) in [
            SmartResampling::Nearest,
            SmartResampling::Bilinear,
            SmartResampling::Area,
        ]
        .iter()
        .enumerate()
        {
            let mut target = Document::with_tile_size(w, h, 16).unwrap();
            let placed = target
                .place_smart_material(
                    &material,
                    &SmartPlacement {
                        resampling: Some(*how),
                        ..Default::default()
                    },
                )
                .unwrap();
            let actual = target
                .layer(placed.layer_id)
                .unwrap()
                .surface(Channel::Color)
                .unwrap()
                .to_canvas_bytes();
            let expected = std::fs::read(root().join(format!("resize-{w}-{h}-{i}.bin"))).unwrap();
            assert_eq!(actual, expected, "{w}x{h}/{i}");
        }
    }
}
#[test]
fn shelf_add_deduplicates_and_budget_refusal_changes_nothing() {
    let f = files();
    let source = Shelf::read(&f, 0).unwrap();
    let r = &source.resources()[0];
    let bytes = source.content_bytes(&r.id).unwrap();
    let mut shelf = Shelf::new(7);
    assert!(shelf.add(r.metadata.clone(), bytes).is_err());
    assert!(shelf.resources().is_empty());
    shelf.set_budget_bytes(8);
    let id = shelf.add(r.metadata.clone(), bytes).unwrap();
    shelf.set_budget_bytes(0);
    let mut same = r.metadata.clone();
    same["id"] = json!("20000000-0000-0000-0000-000000000001");
    assert_eq!(shelf.add(same, bytes).unwrap(), id);
    assert_eq!(shelf.used_bytes(), 8);
    let before = shelf.entries().clone();
    let other = &source.resources()[1];
    assert!(shelf
        .add(
            other.metadata.clone(),
            source.content_bytes(&other.id).unwrap()
        )
        .is_err());
    assert_eq!(shelf.entries(), &before);
    assert!(shelf.remove(&id).unwrap());
    assert_eq!(shelf.used_bytes(), 0);
    // 最後の 1 件を消した棚は resources.json を持たない（C# の ResourceIndex.AddTo と同じ）。最初の 1 件を足すと作る
    assert!(shelf.resources().is_empty());
    assert!(!shelf.entries().contains_key("resources.json"));
    assert!(shelf.entries().is_empty());
    shelf.set_budget_bytes(8);
    shelf.add(r.metadata.clone(), bytes).unwrap();
    assert!(shelf.entries().contains_key("resources.json"));
}
#[test]
fn project_shelf_roundtrip_preserves_resource_payloads() {
    let p = Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format4.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let shelf = Shelf::read(&files(), 1024).unwrap();
    let next = p.with_shelf(&shelf, writer()).unwrap();
    let reopened = Project::read(&next.to_bytes().unwrap()).unwrap();
    assert_eq!(reopened.shelf(1024).unwrap().entries(), shelf.entries());
    assert_eq!(
        p.sets()[0].document.to_bytes(),
        reopened.sets()[0].document.to_bytes()
    );
    assert_eq!(
        Archive::read(&p.to_bytes().unwrap()).unwrap().entries(),
        p.original_archive().entries()
    );
}
#[test]
fn malformed_hash_index_and_origins_are_refused() {
    let f = files();
    let shelf = Shelf::read(&f, 1024).unwrap();
    let r = &shelf.resources()[0];
    let b = shelf.content_bytes(&r.id).unwrap();
    for (key, value) in [
        ("content", json!("0".repeat(64))),
        ("kind", json!("future")),
        ("width", json!(8193)),
        (
            "origin",
            json!({"type":"library","file":"../x","sha256":"a".repeat(64),"length":1}),
        ),
    ] {
        let mut m = r.metadata.clone();
        m[key] = value;
        assert!(Shelf::new(1024).add(m, b).is_err(), "{key}");
    }
    let mut bad = f.clone();
    bad.insert(
        "resources.json".into(),
        Arc::from(b"{\"resources\":[],\"resources\":[]}".as_slice()),
    );
    assert!(Shelf::read(&bad, 1024).is_err());
}
#[test]
fn raster_smart_core_roundtrip() {
    let bytes = std::fs::read(root().join("raster.ylsmart")).unwrap();
    let file = SmartFile::read(&bytes).unwrap();
    let material = file.to_core().unwrap();
    let encoded = SmartFile::from_core(
        &material,
        &WriterInfo {
            app: "YoluPainter".into(),
            version: "test".into(),
            unity: "none".into(),
        },
    )
    .unwrap();
    assert_eq!(
        file.entries()["smart.json"],
        encoded.entries()["smart.json"]
    );
    let restored = encoded.to_core().unwrap();
    assert_eq!(
        material.layers()[0]
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        restored.layers()[0]
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    );
}
#[test]
fn smart_future_version_broken_metadata_and_missing_entries_are_rejected() {
    let file = SmartFile::read(&std::fs::read(root().join("raster.ylsmart")).unwrap()).unwrap();
    for (key, value) in [
        ("format", json!(2)),
        ("kind", json!("future")),
        ("channels", json!(["Color", "Color"])),
        ("repin", json!(["10000000-0000-0000-0000-000000000001"])),
        ("layers", json!(0)),
        ("width", json!(12)),
    ] {
        let mut files = file.entries().clone();
        let mut info = file.info().clone();
        info[key] = value;
        files.insert(
            "smart.json".into(),
            Arc::from(serde_json::to_vec(&info).unwrap()),
        );
        assert!(SmartFile::from_entries(files).is_err(), "{key}");
    }
    let mut files = file.entries().clone();
    files.remove("layers.utpaint");
    assert!(SmartFile::from_entries(files).is_err());
}
#[test]
fn image_hash_matches_csharp_and_transparent_rgb_is_significant() {
    let f = files();
    let s = Shelf::read(&f, 1024).unwrap();
    let expected = &s.resources()[0];
    let pixels = [42, 3, 9, 0, 4, 5, 6, 200];
    assert_eq!(
        yolu_io::shelf::image_hash(&pixels, 2, 1).unwrap(),
        expected.content
    );
    let mut t = Shelf::new(8);
    t.add_image(
        &expected.id,
        &expected.name,
        &pixels,
        2,
        1,
        "srgb",
        expected.metadata["origin"].clone(),
    )
    .unwrap();
    assert_eq!(t.canonical_index().unwrap(), {
        let mut v: Value = serde_json::from_slice(&f["resources.json"]).unwrap();
        v["resources"].as_array_mut().unwrap().pop();
        let mut one = f.clone();
        one.insert(
            "resources.json".into(),
            Arc::from(serde_json::to_vec(&v).unwrap()),
        );
        Shelf::read(&one, 1024).unwrap().canonical_index().unwrap()
    });
    let mut changed = pixels;
    changed[0] = 0;
    assert_ne!(
        yolu_io::shelf::image_hash(&changed, 2, 1).unwrap(),
        expected.content
    );
}
#[test]
fn mask_and_group_smart_core_roundtrip_keeps_layers_and_pixels() {
    for name in ["mask", "multi"] {
        let bytes = std::fs::read(root().join(format!("{name}.ylsmart"))).unwrap();
        let file = SmartFile::read(&bytes).unwrap();
        let material = file.to_core().unwrap();
        let saved = SmartFile::from_core(
            &material,
            &WriterInfo {
                app: "YoluPainter".into(),
                version: "test".into(),
                unity: "none".into(),
            },
        )
        .unwrap();
        assert_eq!(file.entries()["smart.json"], saved.entries()["smart.json"]);
        let before = material.fragment_document().unwrap();
        let after = saved.to_core().unwrap().fragment_document().unwrap();
        for channel in Channel::ALL {
            assert_eq!(
                before.composite_channel(channel, before.bounds()).unwrap(),
                after.composite_channel(channel, after.bounds()).unwrap()
            );
        }
        assert_eq!(before.layers().len(), after.layers().len());
    }
}
#[test]
fn all_file_kinds_roundtrip_and_count_the_embedded_pixels() {
    use yolu_io::shelf::ResourceKind;
    let mut shelf = Shelf::new(1024 * 1024);
    for (i, name, kind) in [
        (1, "raster", ResourceKind::SmartMaterial),
        (2, "mask", ResourceKind::SmartMask),
        (3, "multi", ResourceKind::Material),
    ] {
        let bytes = std::fs::read(root().join(format!("{name}.ylsmart"))).unwrap();
        let id = format!("10000000-0000-0000-0000-{i:012}");
        shelf
            .add_file(&id, name, kind, &bytes, json!({"type":"none"}))
            .unwrap();
        assert_eq!(shelf.content_bytes(&id).unwrap(), bytes);
    }
    let restored = Shelf::read(shelf.entries(), 0).unwrap();
    assert_eq!(restored.used_bytes(), shelf.used_bytes());
    assert_eq!(restored.resources().len(), 3);
}
#[test]
fn csharp_all_kinds_index_payloads_and_memory_agree() {
    let index = std::fs::read(root().join("all/resources.json")).unwrap();
    let value: Value = serde_json::from_slice(&index).unwrap();
    let mut files = BTreeMap::from([("resources.json".into(), Arc::from(index))]);
    for r in value["resources"].as_array().unwrap() {
        let ext = match r["kind"].as_str().unwrap() {
            "image" => "png",
            "brush" => "ylbrush",
            _ => "ylsmart",
        };
        let name = format!("resources/{}.{ext}", r["content"].as_str().unwrap());
        files.insert(
            name.clone(),
            Arc::from(std::fs::read(root().join("all").join(name)).unwrap()),
        );
    }
    let shelf = Shelf::read(&files, 1024 * 1024).unwrap();
    assert_eq!(
        shelf.canonical_index().unwrap(),
        files["resources.json"].as_ref()
    );
    assert_eq!(shelf.entries(), &files);
    let expected: u64 = std::fs::read_to_string(root().join("used.txt"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(shelf.used_bytes(), expected);
    let mut rebuilt = Shelf::new(expected);
    for r in shelf.resources() {
        rebuilt
            .add(r.metadata.clone(), shelf.content_bytes(&r.id).unwrap())
            .unwrap();
    }
    assert_eq!(
        rebuilt.canonical_index().unwrap(),
        files["resources.json"].as_ref()
    );
    assert_eq!(rebuilt.entries(), &files);
}
#[test]
fn csharp_smart_native_roundtrip_is_byte_exact() {
    for name in ["raster", "mask", "multi"] {
        let file = SmartFile::read(&std::fs::read(root().join(format!("{name}.ylsmart"))).unwrap())
            .unwrap();
        let encoded = SmartFile::from_core(
            &file.to_core().unwrap(),
            &WriterInfo {
                app: "YoluPainter".into(),
                version: "test".into(),
                unity: "none".into(),
            },
        )
        .unwrap();
        assert_eq!(
            encoded.entries()["smart.json"],
            file.entries()["smart.json"],
            "{name}"
        );
        assert_eq!(
            encoded.entries()["layers.utpaint"],
            file.entries()["layers.utpaint"],
            "{name}"
        );
    }
}
#[test]
fn embedded_images_add_atomically_and_remap_colliding_ids() {
    let file = SmartFile::read(&std::fs::read(root().join("images.ylsmart")).unwrap()).unwrap();
    let refusal = file.to_core().unwrap_err().to_string();
    assert!(refusal.contains("画像を含む素材"), "{refusal}");
    let mut shelf = Shelf::new(11);
    assert!(file.add_images_to(&mut shelf).is_err());
    assert!(shelf.resources().is_empty());
    shelf.set_budget_bytes(100);
    let id = "10000000-0000-0000-0000-000000000001";
    shelf
        .add_image(
            id,
            "既存",
            &[1, 2, 3, 255],
            1,
            1,
            "srgb",
            json!({"type":"none"}),
        )
        .unwrap();
    let map = file.add_images_to(&mut shelf).unwrap();
    assert_ne!(map[id], id);
    assert_eq!(map.len(), 2);
    assert_eq!(shelf.resources().len(), 3);
    let bytes = shelf.used_bytes();
    file.add_images_to(&mut shelf).unwrap();
    assert_eq!(shelf.resources().len(), 3);
    assert_eq!(shelf.used_bytes(), bytes);
}
#[test]
fn unsupported_filter_keeps_original_and_refuses_core_conversion() {
    let bytes = std::fs::read(root().join("filtered.ylsmart")).unwrap();
    let file = SmartFile::read(&bytes).unwrap();
    assert_eq!(file.file_bytes(), bytes);
    let refusal = file.to_core().unwrap_err().to_string();
    assert!(refusal.contains("フィルター・Generator"), "{refusal}");
    assert!(refusal.contains("layers[0].filters"), "{refusal}");
}

#[test]
fn with_shelf_records_the_writer_that_saved_and_keeps_who_created() {
    let p = Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format4.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let before = p.info().clone();
    assert_ne!(before.saved_by, Some(writer()));
    let shelf = p.shelf(1 << 20).unwrap();
    let saved =
        Project::read(&p.with_shelf(&shelf, writer()).unwrap().to_bytes().unwrap()).unwrap();
    assert_eq!(saved.info().format, 7);
    assert_eq!(saved.info().saved_by, Some(writer()));
    assert_eq!(saved.info().created_by, before.created_by);
    // 書き手の違いだけが変わり、棚と正本は変わらない
    assert_eq!(saved.shelf(1 << 20).unwrap().entries(), shelf.entries());
    assert_eq!(
        saved.sets()[0].document.to_bytes(),
        p.sets()[0].document.to_bytes()
    );
}
#[test]
fn an_emptied_shelf_leaves_no_resources_json_in_the_project() {
    let p = Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/format4.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let mut shelf = p.shelf(1 << 20).unwrap();
    assert!(!shelf.resources().is_empty());
    for id in shelf
        .resources()
        .iter()
        .map(|r| r.id.clone())
        .collect::<Vec<_>>()
    {
        assert!(shelf.remove(&id).unwrap());
    }
    let saved =
        Project::read(&p.with_shelf(&shelf, writer()).unwrap().to_bytes().unwrap()).unwrap();
    assert!(saved.resources().is_empty());
    assert!(!saved.migrated_entries().contains_key("resources.json"));
    assert!(saved
        .migrated_entries()
        .keys()
        .all(|name| !name.starts_with("resources/")));
}

// ───────── .ylsmart と core の境 ─────────

fn user(name: &str) -> ChannelInfo {
    ChannelInfo {
        name: name.into(),
        kind: ChannelKind::Color,
        color_space: ColorSpace::Srgb,
        default: Rgba8::TRANSPARENT,
    }
}
#[test]
fn a_fragment_that_uses_no_user_channel_is_written_in_the_unity_version() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    d.add_channel(user("どの層も使わない")).unwrap();
    let l = d.add_layer("画素").unwrap();
    d.set_pixel(l, 1, 1, Rgba8::new(1, 2, 3, 255)).unwrap();
    let material = d.capture_smart_material(&[l], "素材").unwrap();
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    assert_eq!(file.fragment().version(), UNITY_NATIVE_VERSION);
    let back = file.to_core().unwrap();
    assert_eq!(
        back.layers()[0]
            .surface(Channel::Color)
            .unwrap()
            .pixel(1, 1)
            .unwrap(),
        Rgba8::new(1, 2, 3, 255)
    );
    // 空のマスクだけの素材も同じ
    d.add_layer_mask(l).unwrap();
    let mask = d.capture_smart_mask(l, "マスク").unwrap();
    assert_eq!(
        SmartFile::from_core(&mask, &writer())
            .unwrap()
            .fragment()
            .version(),
        UNITY_NATIVE_VERSION
    );
}
#[test]
fn user_channel_content_is_refused_on_save_even_when_the_channel_is_off() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let ch = d.add_channel(user("使う")).unwrap();
    let l = d.add_layer("画素").unwrap();
    d.set_channel_pixel(l, ch, 0, 0, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    for off in [false, true] {
        if off {
            d.set_channel_enabled(l, ch, false).unwrap();
        }
        let material = d.capture_smart_material(&[l], "素材").unwrap();
        let message = SmartFile::from_core(&material, &writer())
            .unwrap_err()
            .to_string();
        assert!(message.contains("標準チャンネルだけ"), "{off}: {message}");
    }
}
#[test]
fn a_v22_fragment_with_an_unused_user_channel_opens_and_saves_as_the_unity_version() {
    // 版 22（ユーザーチャンネル付き）の断片を持つ .ylsmart。使っていない定義は、開くときに外れる
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    d.add_channel(user("どの層も使わない")).unwrap();
    let l = d.add_layer("画素").unwrap();
    d.set_pixel(l, 2, 2, Rgba8::new(7, 7, 7, 255)).unwrap();
    let fragment = yolu_io::NativeDocument::from_core(&d).unwrap();
    assert_eq!(fragment.version(), 22);
    let plain = d.capture_smart_material(&[l], "素材").unwrap();
    let base = SmartFile::from_core(&plain, &writer()).unwrap();
    let mut entries = base.entries().clone();
    entries.insert("layers.utpaint".into(), Arc::from(fragment.to_bytes()));
    let file = SmartFile::from_entries(entries).unwrap();
    assert_eq!(file.fragment().version(), 22);
    let material = file.to_core().unwrap();
    let saved = SmartFile::from_core(&material, &writer()).unwrap();
    assert_eq!(saved.fragment().version(), UNITY_NATIVE_VERSION);
}
#[test]
fn to_core_has_no_pixel_budget_of_its_own_and_within_is_exact() {
    let file = SmartFile::read(&std::fs::read(root().join("multi.ylsmart")).unwrap()).unwrap();
    let need = file.to_core().unwrap().pixel_bytes();
    assert!(need > 0);
    // 予算ちょうどなら開け、1 バイト足りなければ、どの層のどのタイルかを添えて断る
    assert_eq!(file.to_core_within(need).unwrap().pixel_bytes(), need);
    for budget in [need - 1, need / 2, 0] {
        let message = file.to_core_within(budget).unwrap_err().to_string();
        assert!(message.contains("予算"), "{budget}: {message}");
        assert!(message.contains("layers["), "{budget}: {message}");
    }
    assert_eq!(file.to_core_within(u64::MAX).unwrap().pixel_bytes(), need);
}
#[test]
#[ignore = "256 MiB を超える断片を組むので 2 GB 近く使う。手で `cargo test -p yolu-io --test shelf -- --ignored` で回す"]
fn a_fragment_over_the_default_pixel_budget_opens_and_is_refused_only_where_it_is_placed() {
    // 8192² の 1 チャンネルは、タイル寸法 512 でちょうど 256 MiB（既定の予算）。もう 1 つのチャンネルの 1 タイルで超える
    const MIB: u64 = 1 << 20;
    let mut source = Document::with_tile_size(8192, 8192, 512).unwrap();
    source.set_source_budget_bytes(1 << 30).unwrap();
    let l = source.add_layer("大きい").unwrap();
    let tile = |seed: u8| -> Vec<u8> {
        (0..512 * 512u32)
            .flat_map(|i| [(i % 251) as u8 ^ seed, 8, 9, 255])
            .collect()
    };
    for y in 0..16 {
        for x in 0..16 {
            source
                .import_tile(
                    l,
                    Channel::Color,
                    TileCoord { x, y },
                    &tile((x * 16 + y) as u8),
                )
                .unwrap();
        }
    }
    assert_eq!(source.allocated_bytes(), 256 * MIB);
    source
        .import_tile(l, Channel::Roughness, TileCoord { x: 0, y: 0 }, &tile(3))
        .unwrap();
    assert_eq!(source.allocated_bytes(), 257 * MIB);
    let material = source.capture_smart_material(&[l], "大きい素材").unwrap();
    drop(source);
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    drop(material);
    // 開く（既定の 256 MiB で断らない）。予算を付けて開けば、その予算で断る
    let opened = file.to_core().unwrap();
    assert_eq!(opened.pixel_bytes(), 257 * MIB);
    assert!(file.to_core_within(256 * MIB).is_err());
    assert_eq!(
        file.to_core_within(257 * MIB).unwrap().pixel_bytes(),
        257 * MIB
    );
    // 断るのは置く文書の予算: 既定のままなら断り、広げれば置ける
    let mut target = Document::with_tile_size(8192, 8192, 512).unwrap();
    assert!(matches!(
        target.place_smart_material(&opened, &SmartPlacement::default()),
        Err(CoreError::SourceBudgetExceeded)
    ));
    assert!(target.layers().is_empty());
    target.set_source_budget_bytes(257 * MIB).unwrap();
    target
        .place_smart_material(&opened, &SmartPlacement::default())
        .unwrap();
    assert_eq!(target.allocated_bytes(), 257 * MIB);
}
