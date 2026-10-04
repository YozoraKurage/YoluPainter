//! 選択範囲の selection.bin: C# の `SelectionBinary` が書いたバイト列との一致、core の選択範囲との往復、プロジェクトへの出し入れ。
use yolu_core::glam::DVec2;
use yolu_core::{Document, SelectionCombine, SelectionMask};
use yolu_io::{Archive, NativeDocument, Project, Selection};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/selection");

/// 70×50・タイル 16 の文書（C# の選択範囲の正解と同じ大きさ）。
fn document() -> Document {
    let mut doc = Document::with_tile_size(70, 50, 16).unwrap();
    doc.add_layer("レイヤー 1").unwrap();
    doc
}

fn native(doc: &Document) -> NativeDocument {
    NativeDocument::from_core(doc).unwrap()
}

/// tools/csharp-golden/Golden.cs の SelectionFixtures と同じ作り方。
fn shapes(doc: &Document) -> Vec<(&'static str, SelectionMask)> {
    let ellipse = |cx, cy, rx, ry| SelectionMask::ellipse(doc, cx, cy, rx, ry).unwrap();
    let budget = yolu_core::selection::DEFAULT_WORKING_BUDGET_BYTES;
    let combine = ellipse(30.25, 20.75, 18.5, 11.5)
        .combine(
            &SelectionMask::rectangle(doc, 50, 30, 80, 60),
            SelectionCombine::Add,
        )
        .unwrap()
        .combine(&ellipse(40.0, 25.0, 8.0, 6.0), SelectionCombine::Subtract)
        .unwrap();
    let polygon = SelectionMask::polygon(
        doc,
        &[
            DVec2::new(5.5, 4.25),
            DVec2::new(58.0, 9.75),
            DVec2::new(31.5, 44.0),
            DVec2::new(40.0, 20.0),
            DVec2::new(12.0, 38.5),
        ],
    )
    .unwrap();
    vec![
        ("combine", combine),
        ("polygon", polygon),
        (
            "feather",
            ellipse(35.0, 25.0, 20.0, 12.0)
                .feather(4.0, false, budget)
                .unwrap(),
        ),
        ("all", SelectionMask::all(doc)),
        ("invert", ellipse(35.0, 25.0, 20.0, 12.0).invert()),
        ("none", SelectionMask::none(doc)),
    ]
}

fn fixture(name: &str, ext: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/selection-{name}.{ext}"))
        .unwrap_or_else(|e| panic!("{name}.{ext}: {e}"))
}

#[test]
fn rust_selections_write_the_same_bytes_as_the_csharp_codec() {
    let doc = document();
    for (name, mask) in shapes(&doc) {
        // C# の量と同じ（作った選択範囲そのものが一致していること）
        assert_eq!(
            mask.to_canvas_bytes(),
            fixture(name, "amounts"),
            "{name}: 量"
        );
        // C# の SelectionBinary.Write と全バイト一致
        let selection = Selection::from_core(&mask).unwrap();
        assert_eq!(
            selection.to_bytes(),
            fixture(name, "bin"),
            "{name}: selection.bin"
        );
    }
}

#[test]
fn csharp_selection_files_read_back_into_the_same_core_selection() {
    let doc = document();
    let native = native(&doc);
    for (name, mask) in shapes(&doc) {
        let bytes = fixture(name, "bin");
        let selection = Selection::read(&bytes, &native).unwrap();
        assert_eq!(selection.to_bytes(), bytes, "{name}: 読んで書き直して同じ");
        let core = selection.to_core().unwrap();
        assert_eq!(
            core.to_canvas_bytes(),
            fixture(name, "amounts"),
            "{name}: 量"
        );
        assert_eq!(core, mask, "{name}: Rust で作ったものと同じ中身");
        // 読み込んだ直後の文書へ戻す: 空は選択なし、履歴は増えない
        let mut fresh = native.to_core().unwrap();
        assert_eq!(fresh.undo_count(), 0);
        fresh.restore_selection(Some(core)).unwrap();
        assert_eq!(fresh.selection().is_some(), name != "none", "{name}");
        assert_eq!(fresh.undo_count(), 0, "{name}");
    }
}

#[test]
fn the_selection_goes_into_and_out_of_a_project_without_touching_the_rest() {
    let doc = document();
    let native = native(&doc);
    let spec = yolu_io::SetSpec {
        id: "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c01".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone()),
        composites: Vec::new(),
    };
    let writer = yolu_io::WriterInfo {
        app: "試験".into(),
        version: "1".into(),
        unity: "なし".into(),
    };
    let project = Project::create(writer, &[spec], "5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c01").unwrap();
    let id = project.current_set().to_string();
    assert!(project.sets()[0].selection.is_none());

    let mask = shapes(&doc).remove(0).1;
    let with = project
        .with_selection(&id, Some(&Selection::from_core(&mask).unwrap()))
        .unwrap();
    let reopened = Project::read(&with.to_bytes().unwrap()).unwrap();
    let stored = reopened.sets()[0]
        .selection
        .as_ref()
        .expect("選択範囲が残る");
    assert_eq!(stored.to_core().unwrap(), mask);
    assert_eq!(
        reopened.sets()[0].document.to_bytes(),
        native.to_bytes(),
        "正本は変わらない"
    );

    // 選択なしへ（エントリを消す）。ほかのエントリは変わらない
    let without = reopened.with_selection(&id, None).unwrap();
    let again = Project::read(&without.to_bytes().unwrap()).unwrap();
    assert!(again.sets()[0].selection.is_none());
    assert_eq!(again.sets()[0].document.to_bytes(), native.to_bytes());

    // 知らないセットと、正本と大きさの合わない選択範囲は断る
    assert!(with
        .with_selection("5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c02", None)
        .is_err());
    let other = Document::with_tile_size(40, 40, 16).unwrap();
    let wrong = Selection::from_core(&SelectionMask::all(&other)).unwrap();
    assert!(project.with_selection(&id, Some(&wrong)).is_err());
}

#[test]
fn a_selection_for_another_size_or_a_tile_outside_it_does_not_become_a_core_selection() {
    let doc = document();
    let native = native(&doc);
    let good = fixture("combine", "bin");
    // 大きさが文書と違う（幅を 1 増やす）
    let mut wide = good.clone();
    wide[8] += 1;
    assert!(Selection::read(&wide, &native).is_err());
    // タイルの座標が文書の外
    let mut outside = good.clone();
    outside[24] = 9;
    assert!(Selection::read(&outside, &native).is_err());
    // 切り詰め・後ろの余り
    assert!(Selection::read(&good[..good.len() - 1], &native).is_err());
    let mut extra = good;
    extra.push(0);
    assert!(Selection::read(&extra, &native).is_err());
}

fn older_fixture(n: usize) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/format{n}.ylp",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("format{n}.ylp: {e}"))
}

/// 形式 1・2 は選択範囲がルートの `selection.bin`、3 以降はセットの下。
fn selection_entry(format: i32, set_id: &str) -> String {
    if format < 3 {
        "selection.bin".into()
    } else {
        format!("sets/{set_id}/selection.bin")
    }
}

/// 文書と同じ大きさの、中身のある選択範囲（書いたものが元の選択範囲と区別できる形）。
fn mask_for(native: &NativeDocument) -> SelectionMask {
    let doc = Document::with_tile_size(
        native.width() as u32,
        native.height() as u32,
        native.tile_size() as u32,
    )
    .unwrap();
    let (w, h) = (native.width() as f64, native.height() as f64);
    SelectionMask::ellipse(&doc, w * 0.4, h * 0.45, w * 0.25, h * 0.3).unwrap()
}

/// 旧い形式（Unity 版が書いた形式 1〜6 の実物）の選択範囲の出し入れ。形式 < 3 はルートの `selection.bin` を書く枝で、
/// どの形式でも、選択範囲のエントリのほかは 1 バイトも変わらず、形式も変わらず、ほかのセットの選択範囲も変わらない。
#[test]
fn the_selection_goes_into_and_out_of_every_older_format_without_touching_the_rest() {
    let writer = || yolu_io::WriterInfo {
        app: "試験".into(),
        version: "1".into(),
        unity: "なし".into(),
    };
    for n in 1..=6usize {
        let bytes = older_fixture(n);
        let project = Project::read(&bytes).unwrap_or_else(|e| panic!("形式{n}: {e}"));
        let format = project.info().format;
        assert_eq!(format, n as i32);
        let before = Archive::read(&bytes).unwrap();
        let set = &project.sets()[0];
        let id = set.id.clone();
        let entry = selection_entry(format, &id);
        let mask = mask_for(&set.document);
        let selection = Selection::from_core(&mask).unwrap();
        let had_before = before.entries().contains_key(&entry);
        assert_ne!(
            set.selection.as_ref().map(|s| s.to_bytes()),
            Some(selection.to_bytes()),
            "形式{n}: 元の選択範囲と区別できる形にする"
        );

        // 選択範囲を置く: そのエントリだけが変わり、読み直しても同じ。形式は旧いまま
        let with = project.with_selection(&id, Some(&selection)).unwrap();
        let reopened = Project::read(&with.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.info().format, format, "形式{n}: 形式は変わらない");
        let after = Archive::read(&with.to_bytes().unwrap()).unwrap();
        assert_eq!(
            after.entries().get(&entry).map(|b| b.to_vec()),
            Some(selection.to_bytes()),
            "形式{n}: {entry}"
        );
        for (name, data) in before.entries() {
            if *name != entry {
                assert_eq!(after.entries().get(name), Some(data), "形式{n}: {name}");
            }
        }
        assert_eq!(
            after.entries().len(),
            before.entries().len() + usize::from(!had_before),
            "形式{n}: エントリの数"
        );
        assert_eq!(
            reopened.sets()[0]
                .selection
                .as_ref()
                .unwrap()
                .to_core()
                .unwrap(),
            mask,
            "形式{n}: 読み直した選択範囲"
        );
        for (a, b) in project.sets().iter().zip(reopened.sets()) {
            assert_eq!(
                a.document.to_bytes(),
                b.document.to_bytes(),
                "形式{n}: 文書"
            );
            if a.id != id {
                assert_eq!(
                    a.selection.as_ref().map(|s| s.to_bytes()),
                    b.selection.as_ref().map(|s| s.to_bytes()),
                    "形式{n}: ほかのセットの選択範囲"
                );
            }
        }
        // 新しい形式へ移しても選択範囲は付いてくる
        let upgraded = with.upgraded(writer()).unwrap();
        assert_eq!(upgraded.info().format, 7);
        assert_eq!(
            upgraded.sets()[0]
                .selection
                .as_ref()
                .unwrap()
                .to_core()
                .unwrap(),
            mask,
            "形式{n}: 形式 7 へ移した後"
        );

        // 選択なしへ: そのエントリが消え、ほかは元のまま（元から無いなら何も変わらない）
        let without = reopened.with_selection(&id, None).unwrap();
        let none = Archive::read(&without.to_bytes().unwrap()).unwrap();
        assert!(!none.entries().contains_key(&entry), "形式{n}: {entry}");
        for (name, data) in before.entries() {
            if *name != entry {
                assert_eq!(none.entries().get(name), Some(data), "形式{n}: {name}");
            }
        }
        assert_eq!(
            none.entries().len(),
            before.entries().len() - usize::from(had_before),
            "形式{n}: エントリの数（選択なし）"
        );
        let again = Project::read(&without.to_bytes().unwrap()).unwrap();
        assert!(again.sets()[0].selection.is_none(), "形式{n}");
        assert_eq!(again.info().format, format);

        // 知らないセットと、大きさの合わない選択範囲は断る（旧い形式でも）
        assert!(project
            .with_selection("5f7f1e2e-8d52-4b8e-9a31-0c0c0c0c0c02", Some(&selection))
            .is_err());
        let other = Document::with_tile_size(
            set.document.width() as u32 + 1,
            set.document.height() as u32,
            set.document.tile_size() as u32,
        )
        .unwrap();
        let wrong = Selection::from_core(&SelectionMask::all(&other)).unwrap();
        assert!(
            project.with_selection(&id, Some(&wrong)).is_err(),
            "形式{n}"
        );
    }
}
