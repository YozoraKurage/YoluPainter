//! レイヤーのロック（正本の版 12。属性の印のビット 1 と、直後の int）の読み書き。正解は C# の実際の書き手（`tools/io-fixtures/M2Fixture.cs` の
//! `--locks`）が作った `locks-v21.utpaint` と、Rust が書いて C# の読み手に読ませた `rust-written-locks-v21.*`。ロックは合成を変えない。
use yolu_core::{
    BlendMode, Channel, ChannelBlend, CoreError, Document, LayerId, LayerKind, LayerLocks, Rect,
};
use yolu_io::{NativeDocument, NativeValue, Project, SetSpec, WriterInfo};

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn id_of(doc: &Document, name: &str) -> LayerId {
    doc.layers()
        .iter()
        .find(|l| l.name() == name)
        .unwrap_or_else(|| panic!("レイヤー「{name}」がありません"))
        .id()
}

/// C# の `M2Fixture.Composites` と同じ並び: 全チャンネル（番号の順）の合成、続けて Normal のファイル出力。
fn composites(doc: &Document) -> Vec<u8> {
    let rect = Rect {
        x: 0,
        y: 0,
        width: doc.width(),
        height: doc.height(),
    };
    let mut out = Vec::new();
    for c in Channel::ALL {
        out.extend(doc.composite_channel(c, rect).unwrap());
    }
    out.extend(doc.normal_file_output(u64::MAX).unwrap());
    out
}

fn bits(n: u8) -> LayerLocks {
    LayerLocks::from_bits(n).unwrap()
}

/// （レイヤーの名前, 自分のロック）。`M2Fixture.LockedLayers` の並び。
fn expected() -> Vec<(&'static str, LayerLocks)> {
    vec![
        ("ロック無し", LayerLocks::NONE),
        ("透明部分", LayerLocks::TRANSPARENCY),
        ("画素", LayerLocks::PIXELS),
        ("位置", LayerLocks::POSITION),
        ("すべて", LayerLocks::ALL),
        ("透明部分と位置（クリップ）", bits(1 | 4)),
        ("個別 3 つ + すべて", bits(15)),
        ("ロックした塗り", LayerLocks::PIXELS),
        ("ロックした調整", LayerLocks::POSITION),
        ("ロックしたグループの中", LayerLocks::NONE),
        ("ロックしたグループ", bits(2 | 4)),
        ("ロックの無い層（ロックの層の隣）", LayerLocks::NONE),
    ]
}

#[test]
fn a_native_document_written_by_csharp_with_locks_reads_into_core_and_writes_back_the_same_bytes() {
    let original = read("locks-v21.utpaint");
    let native = NativeDocument::read(&original).unwrap();
    assert_eq!(native.version(), 21);
    // ロックを持つレイヤーが正本の中にいる（読み手が通した項目）
    let locked: Vec<_> = (0..native.layer_count())
        .filter(|i| native.field(&format!("layers[{i}].locks")).is_some())
        .collect();
    assert_eq!(
        locked.len(),
        9,
        "ロックを持つレイヤーの数（グループと塗り・調整を含む）"
    );
    assert!(
        native.core_issues().is_empty(),
        "{:?}",
        native.core_issues()
    );
    let core = native.to_core().unwrap();
    assert!(
        !core.can_undo() && !core.can_redo(),
        "読み込みは Undo の履歴に残さない"
    );
    for (name, locks) in expected() {
        assert_eq!(
            core.layer(id_of(&core, name)).unwrap().locks(),
            locks,
            "{name}"
        );
    }
    assert_eq!(core.layers().len(), expected().len());
    // 属性の印のビット 1 と 2 が両方立つレイヤーは、クリッピングもチャンネルごとの合成もロックと一緒に戻る
    let mixed = core
        .layer(id_of(&core, "透明部分と位置（クリップ）"))
        .unwrap();
    assert!(mixed.clipping());
    assert_eq!(
        mixed.channel_blend(Channel::Color),
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5))
    );
    assert!(mixed.surface(Channel::Height).is_some());
    // グループのロックは中身に効く（自分のロックは持たず、効くロックだけがある）
    let inside = id_of(&core, "ロックしたグループの中");
    assert_eq!(core.layer(inside).unwrap().locks(), LayerLocks::NONE);
    assert_eq!(core.effective_locks(inside).unwrap(), bits(2 | 4));
    assert!(matches!(
        core.ensure_pixels_editable(inside, false),
        Err(CoreError::LayerLocked { layer, holder, lock })
            if layer == inside && holder == id_of(&core, "ロックしたグループ") && lock == LayerLocks::PIXELS
    ));
    // すべてのロックは個別の 3 種を含んで効く
    assert_eq!(
        core.effective_locks(id_of(&core, "すべて")).unwrap(),
        bits(15)
    );
    // ロックが効いているのは、ロックの無いレイヤーと違って画素を書く入口が断るから
    assert!(core
        .ensure_pixels_editable(id_of(&core, "ロック無し"), true)
        .is_ok());
    assert!(core
        .ensure_pixels_editable(id_of(&core, "画素"), false)
        .is_err());
    assert!(core
        .ensure_pixels_editable(id_of(&core, "透明部分"), false)
        .is_ok());
    assert!(core
        .ensure_pixels_editable(id_of(&core, "透明部分"), true)
        .is_err());
    // C# の書き手の並びと全バイト一致する書き戻し、合成はロックに影響されない
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        original,
        "C# の正本を core にして書き戻すとバイト一致"
    );
    assert_eq!(composites(&core), read("locks-v21.composite"));
    // ロックを全部外した合成も同じ（ロックは見た目を変えない）
    let mut open = native.to_core().unwrap();
    for (name, _) in expected() {
        open.set_locks_for_load(id_of(&open, name), LayerLocks::NONE)
            .unwrap();
    }
    assert_eq!(composites(&open), read("locks-v21.composite"));
    assert_ne!(
        NativeDocument::from_core(&open).unwrap().to_bytes(),
        original
    );
}

/// Rust が編集してロックを付け外しした版 21。固定の ID で作り、ファイルと同じバイト列になる。作り直すときは
/// `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp layer_locks::rust_written`、続けて
/// `python3 tools/io-fixtures/generate.py --source <Unity 版> --rust-written-locks` で Unity 版の読み手の記録を取り直す。
fn edited_locked_document() -> Document {
    let mut core = NativeDocument::read(&read("locks-v21.utpaint"))
        .unwrap()
        .to_core()
        .unwrap();
    let plain = id_of(&core, "ロック無し");
    let pixels = id_of(&core, "画素");
    let group = id_of(&core, "ロックしたグループ");
    let fill = id_of(&core, "ロックした塗り");
    // 付ける・外す・変える・複数のレイヤーへまとめて（ロックは 1 回の Undo。履歴に残るので、読み込んだ直後の文書とは別の文書になる）
    core.set_layer_locks(plain, bits(1 | 2)).unwrap();
    core.set_layer_locks(pixels, LayerLocks::NONE).unwrap();
    core.change_layer_locks(&[fill, group], LayerLocks::TRANSPARENCY, true)
        .unwrap();
    core.change_layer_locks(&[group], LayerLocks::POSITION, false)
        .unwrap();
    // 新しいレイヤーと、ロックしたグループの複製（複製もロックを持つ）
    let added = core.add_layer("新しい層").unwrap();
    core.set_layer_locks(added, LayerLocks::ALL).unwrap();
    core.duplicate_layer(group, Some("グループの写し")).unwrap();
    let ids: Vec<LayerId> = (0..core.layers().len())
        .map(|i| LayerId(0x7100 + i as u128))
        .collect();
    core.with_persistent_ids(0x7777, &ids).unwrap()
}

#[test]
fn rust_written_locks_are_what_this_writer_produces_and_unity_reads_and_resaves_them() {
    let edited = edited_locked_document();
    let bytes = NativeDocument::from_core(&edited).unwrap().to_bytes();
    let path = format!(
        "{}/tests/fixtures/rust-written-locks-v21.utpaint",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("YOLU_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(path.replace(".utpaint", ".composite"), composites(&edited)).unwrap();
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "記録した正本が今の書き手の出力と同じ"
    );
    assert_eq!(
        bytes[8..12],
        21i32.to_le_bytes(),
        "ユーザーチャンネルが無い文書は版 21（Unity 0.2.0 が読める）"
    );
    // 書いたものを読み戻すと、自分のロックも効くロックも同じ
    let back = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
    for l in edited.layers() {
        assert_eq!(
            back.layer(l.id()).unwrap().locks(),
            l.locks(),
            "{}",
            l.name()
        );
        assert_eq!(
            back.effective_locks(l.id()).unwrap(),
            edited.effective_locks(l.id()).unwrap(),
            "{}",
            l.name()
        );
    }
    // Unity 0.2.0 の読み手（`DocumentBinary`）に読ませた記録: 読めて、書き直すと同じバイト列になり、自分のロックと効くロックが Rust と同じ
    let record = String::from_utf8(read("rust-written-locks-v21.unity.txt")).unwrap();
    let own_and_effective = edited
        .layers()
        .iter()
        .map(|l| {
            format!(
                "{}/{}",
                l.locks().bits(),
                edited.effective_locks(l.id()).unwrap().bits()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        record.lines().collect::<Vec<_>>(),
        [
            "DocumentBinary.CurrentVersion: 21".to_string(),
            "DocumentBinary.ReadId: OK".to_string(),
            "DocumentBinary.Read: OK".to_string(),
            "DocumentBinary.Write(Read) == input: True".to_string(),
            format!("Layers: {}", edited.layers().len()),
            format!("Locks (own/effective): {own_and_effective}"),
        ]
    );
    assert_eq!(
        composites(&edited),
        read("rust-written-locks-v21.composite"),
        "C# が読んだ文書の全チャンネルの合成が Rust の合成と全バイト一致"
    );
}

/// ロックの印の規則（C# の LayerLockTests と同じ）: 印のビット 1 が立てば直後に int のロック（1〜15）が続く。0・知らないビットは断る。
#[test]
fn a_lock_value_of_zero_or_with_an_unknown_bit_is_refused_and_a_lockless_layer_has_no_lock_field() {
    let native = NativeDocument::read(&read("locks-v21.utpaint")).unwrap();
    let locked = (0..native.layer_count())
        .find(|i| native.field(&format!("layers[{i}].locks")).is_some())
        .unwrap();
    for bad in [0, 16, 17, 255, -1, i32::MAX] {
        assert!(
            native
                .with_value(&format!("layers[{locked}].locks"), NativeValue::Int(bad))
                .is_err(),
            "{bad}"
        );
    }
    for good in 1..=15 {
        let changed = native
            .with_value(&format!("layers[{locked}].locks"), NativeValue::Int(good))
            .unwrap();
        let core = changed.to_core().unwrap();
        assert_eq!(
            core.layers()[locked].locks().bits(),
            good as u8,
            "{good}: レイヤーの並びのとおりに入る"
        );
    }
    // ロックの無いレイヤーには locks の項目が無い（属性の印は 0 か 1 だけ）
    let unlocked = (0..native.layer_count())
        .find(|i| native.field(&format!("layers[{i}].locks")).is_none())
        .unwrap();
    assert!(native.field(&format!("layers[{unlocked}].locks")).is_none());
    assert!(matches!(
        native.field(&format!("layers[{unlocked}].attributes")),
        Some(NativeValue::Byte(0 | 1))
    ));
}

/// ロックの無い文書は、ロックを覚えていた前と同じバイト列（版の数も同じ）。ロックを付けてから外すと元のバイト列に戻る。
#[test]
fn an_unlocked_document_writes_the_same_bytes_and_locking_then_unlocking_returns_to_them() {
    let original = read("m2-groups.utpaint");
    let native = NativeDocument::read(&original).unwrap();
    let mut core = native.to_core().unwrap();
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        original
    );
    let ids: Vec<LayerId> = core.layers().iter().map(|l| l.id()).collect();
    for (i, id) in ids.iter().enumerate() {
        core.set_layer_locks(*id, bits(1 + (i % 15) as u8)).unwrap();
    }
    let locked = NativeDocument::from_core(&core).unwrap();
    assert_ne!(locked.to_bytes(), original);
    assert!(locked.core_issues().is_empty());
    // 全部のレイヤーのロックは、元と同じ画素・属性の上に付く
    let back = locked.to_core().unwrap();
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(back.layer(*id).unwrap().locks().bits(), 1 + (i % 15) as u8);
    }
    for id in &ids {
        core.set_layer_locks(*id, LayerLocks::NONE).unwrap();
    }
    assert_eq!(
        NativeDocument::from_core(&core).unwrap().to_bytes(),
        original
    );
}

/// 版 12 は、ロックの印（属性の印のビット 1 と直後の int）を足した版。ロックのある単純な文書（版 12 より新しい項目を使わない）は、
/// 版の数だけを 12 にしても同じ並びで読める。版 11 には属性の印が無く（クリッピングの 1 バイトだけ）、ロックを持つ正本を版 11 と
/// 偽ると断る（黙って落とさない）。
#[test]
fn a_document_with_locks_reads_as_version_12_and_is_refused_as_version_11() {
    let mut doc = Document::with_tile_size(16, 12, 8).unwrap();
    let a = doc.add_layer("a").unwrap();
    let b = doc.add_layer("b").unwrap();
    doc.set_channel_pixel(a, Channel::Color, 1, 1, yolu_core::Rgba8::new(9, 8, 7, 255))
        .unwrap();
    doc.set_layer_locks(a, bits(1 | 4)).unwrap();
    doc.set_layer_clipping(b, true).unwrap();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    assert_eq!(bytes[8..12], 21i32.to_le_bytes());
    let relabel = |version: i32| {
        let mut patched = bytes.clone();
        patched[8..12].copy_from_slice(&version.to_le_bytes());
        patched
    };
    for version in [12, 13] {
        let native = NativeDocument::read(&relabel(version)).unwrap();
        assert_eq!(native.version(), version);
        let core = native.to_core().unwrap();
        assert_eq!(
            core.layer(id_of(&core, "a")).unwrap().locks(),
            bits(1 | 4),
            "版 {version}"
        );
        assert!(
            core.layer(id_of(&core, "b")).unwrap().clipping(),
            "版 {version}"
        );
        assert_eq!(
            core.layer(id_of(&core, "b")).unwrap().locks(),
            LayerLocks::NONE
        );
    }
    for version in [5, 6, 11] {
        assert!(
            NativeDocument::read(&relabel(version)).is_err(),
            "ロックの印を持つ正本を版 {version} と偽ると断る"
        );
    }
    // ロックの無い同じ文書は、版 11 と偽っても読める（ロックの無い文書は版 11 と同じ並び）
    doc.set_layer_locks(a, LayerLocks::NONE).unwrap();
    let plain = NativeDocument::from_core(&doc).unwrap().to_bytes();
    let mut v11 = plain.clone();
    v11[8..12].copy_from_slice(&11i32.to_le_bytes());
    assert!(NativeDocument::read(&v11).is_ok());
}

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

/// .ylp に保存して開き直しても、ロック（自分のもの・グループのもの）は同じ。
#[test]
fn locks_survive_a_project_save_and_open() {
    let core = edited_locked_document();
    const SET: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
    let project = Project::create(
        writer(),
        &[SetSpec {
            id: SET.into(),
            name: "ロック".into(),
            material: yolu_io::MaterialRef::Unassigned,
            document: Some(NativeDocument::from_core(&core).unwrap().into()),
            composites: yolu_io::composite_pngs(&core).unwrap(),
        }],
        SET,
    )
    .unwrap();
    let opened = Project::read(&project.to_bytes().unwrap()).unwrap();
    let back = opened.sets()[0].document.to_core().unwrap();
    assert_eq!(back.layers().len(), core.layers().len());
    for l in core.layers() {
        let b = back.layer(l.id()).unwrap();
        assert_eq!(b.locks(), l.locks(), "{}", l.name());
        assert_eq!(b.name(), l.name());
    }
    // 開き直した文書への編集は、ロックを守る（画素を書く入口が断る）
    let locked_group = id_of(&back, "ロックしたグループ");
    assert!(back
        .effective_locks(locked_group)
        .unwrap()
        .contains(LayerLocks::PIXELS));
    assert!(matches!(
        back.ensure_pixels_editable(id_of(&back, "ロックしたグループの中"), false),
        Err(CoreError::LayerLocked { .. })
    ));
}

/// ロックのあるレイヤーを .ylsmart（スマートマテリアル）に保存でき、開くとロックも戻る（C# の `CloneLayer` が `Locks` を写すのと同じ）。
#[test]
fn a_smart_material_keeps_the_locks_of_its_layers() {
    use yolu_io::smart::SmartFile;
    let core = edited_locked_document();
    let source = id_of(&core, "ロックしたグループ");
    let material = core
        .capture_smart_material(&[source], "ロックの素材")
        .unwrap();
    let file = SmartFile::from_core(&material, &writer()).unwrap();
    let again = SmartFile::read(file.file_bytes())
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(again.layers().len(), material.layers().len());
    for (a, b) in again.layers().iter().zip(material.layers()) {
        assert_eq!(a.locks(), b.locks(), "{}", b.name());
        assert_eq!(a.kind() == LayerKind::Group, b.kind() == LayerKind::Group);
    }
    assert!(again.layers().iter().any(|l| l.locks() != LayerLocks::NONE));
}

// ───────── PSD（lspf）の往復 ─────────

use yolu_io::psd::{self, CompatibilityMode, Limits};

fn painted(names: &[&str]) -> Document {
    let mut doc = Document::with_tile_size(16, 12, 8).unwrap();
    for (i, name) in names.iter().enumerate() {
        let id = doc.add_layer(name).unwrap();
        for y in 0..12 {
            for x in 0..16 {
                let v = (x * 31 + y * 17 + i as u32 * 53) as u8;
                doc.set_channel_pixel(
                    id,
                    Channel::Color,
                    x,
                    y,
                    yolu_core::Rgba8::new(v, v.wrapping_mul(3), v.wrapping_add(90), v | 1),
                )
                .unwrap();
            }
        }
    }
    doc
}

fn lspf_values(bytes: &[u8]) -> Vec<u32> {
    bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"lspf")
        .map(|(at, _)| u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()))
        .collect()
}

/// core → PSD → core → PSD。ロックは PSD の lspf を通って同じ効くロックで core へ戻り、書き直した PSD は最初と全バイト一致する
/// （ロックが取り込みで落ちないこと・足されないこと）。取り込みは履歴を残さない。
#[test]
fn psd_locks_come_back_into_core_and_write_the_same_bytes_again() {
    let mut doc = painted(&["下", "中", "上", "グループなし"]);
    let all = [
        ("下", LayerLocks::TRANSPARENCY),
        ("中", bits(2 | 4)),
        ("上", LayerLocks::ALL),
        ("グループなし", bits(1 | 2 | 4)),
    ];
    for (name, lock) in all {
        doc.set_layer_locks(id_of(&doc, name), lock).unwrap();
    }
    let first = psd::write(&psd::Document::from_core(&doc).unwrap(), &Limits::default()).unwrap();
    assert_eq!(lspf_values(&first), vec![1, 6, 0x8000_0000, 7]);
    let read = psd::read(&first, &Limits::default()).unwrap();
    assert_eq!(read.mode(), CompatibilityMode::EditableRaster);
    assert!(read.diagnostics().is_empty(), "{:?}", read.diagnostics());
    let back = read.to_core().unwrap();
    assert!(!back.can_undo(), "取り込みは Undo の履歴に残さない");
    for (name, lock) in all {
        assert_eq!(
            back.effective_locks(id_of(&back, name)).unwrap(),
            doc.effective_locks(id_of(&doc, name)).unwrap(),
            "{name}"
        );
        // すべてだけは個別のビットを足さないので、自分のロックは「すべて」だけに畳まれる
        let own = back.layer(id_of(&back, name)).unwrap().locks();
        assert_eq!(
            own,
            if lock.contains(LayerLocks::ALL) {
                LayerLocks::ALL
            } else {
                lock
            },
            "{name}"
        );
    }
    let second = psd::write(
        &psd::Document::from_core(&back).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(second, first, "取り込んで書き出し直しても全バイト同じ");
}

/// Photoshop の書き方（すべてに個別のビットを重ねた 0x80000007、レイヤーの記録の印のビット 0 だけの透明部分）も取り込む。
/// lspf のビット 3（アートボードへの入れ子の禁止）は core のロックに無いので、取り込みを断らずロックを足さない（読み込みの通知は psd の側）。
#[test]
fn photoshop_style_lspf_bits_import_as_the_matching_core_locks() {
    let base = painted(&["a", "b", "c", "d"]);
    let mut doc = psd::Document::from_core(&base).unwrap();
    // 上から d, c, b, a
    doc.layers[3].locks = 0x8000_0007; // a: すべて + 個別（Photoshop）
    doc.layers[2].locks = 4; // b: 位置
    doc.layers[1].locks = 0; // c: ロックなし
    doc.layers[0].locks = 2 | 1; // d: 透明部分と画素
    let core = doc.to_core().unwrap();
    let locks = |name: &str| core.layer(id_of(&core, name)).unwrap().locks();
    assert_eq!(locks("a"), bits(15));
    assert_eq!(locks("b"), LayerLocks::POSITION);
    assert_eq!(locks("c"), LayerLocks::NONE);
    assert_eq!(locks("d"), bits(1 | 2));
    assert_eq!(core.effective_locks(id_of(&core, "a")).unwrap(), bits(15));
    // 書き出し直すと、すべては 0x80000000 だけ
    let written = psd::write(
        &psd::Document::from_core(&core).unwrap(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(lspf_values(&written), vec![0x8000_0000, 4, 3]);
}
