//! CLIP STUDIO の `.sut`（SQLite）の取り込み: 設定の写し・筆先と質感の画像・影響元・表せなかった項目・表の欠け・上限・
//! 悪意のあるスキーマ。試験のファイルは本物の .sut ではなく、公開の解析で分かっている形を試験の中で組む（`brush_files::sut`）。
//! 本物の .sut での確かめは別（docs/BRUSH_IMPORT.md の「確かめた範囲」）。

use crate::brush_files;

use std::time::{Duration, Instant};

use brush_files::sut::*;
use yolu_io::brushes::{
    import, import_bytes, BrushImportError, ColorNote, Fault, FileKind, ImportedBrush, ImportedSet,
    SkipReason, Source, SutInput, SutMapped, SutNote, SutTarget, Unrepresented,
};

fn read(bytes: &[u8]) -> Result<ImportedSet, BrushImportError> {
    import_bytes(FileKind::Sut, bytes, Some("File name"))
}

fn ok(bytes: &[u8]) -> ImportedSet {
    read(bytes).unwrap_or_else(|e| panic!("取り込めない: {e:?}"))
}

fn note(n: SutNote) -> Unrepresented {
    Unrepresented::ClipStudio(n)
}

fn has(brush: &ImportedBrush, n: SutNote) -> bool {
    brush.unrepresented.contains(&note(n))
}

/// 筆先の画素の覆い（行は下から）。
fn alpha(brush: &ImportedBrush) -> Vec<u8> {
    brush
        .brush
        .tip
        .image
        .as_ref()
        .expect("筆先の画像")
        .alpha()
        .to_vec()
}

fn faulted(result: Result<ImportedSet, BrushImportError>) -> Fault {
    match result {
        Err(BrushImportError::Fault(f)) => f,
        Err(other) => panic!("Fault のはず: {other:?}"),
        Ok(set) => panic!("断られるはず: {} 個取り込めた", set.brushes.len()),
    }
}

fn tip_png() -> Vec<u8> {
    // 2×2。輝度 0 = 塗る
    png_gray(2, 2, &[0, 255, 255, 0])
}

// ---------------- 設定 ----------------

#[test]
fn plain_settings_are_read_from_the_variant_row() {
    let file = SutBuilder::new()
        .brush(
            "G-pen",
            1,
            &[
                ("BrushSize", real(40.0)),
                ("Opacity", int(80)),
                ("BrushFlow", real(60.0)),
                ("BrushHardness", int(50)),
                ("BrushInterval", int(25)),
                ("BrushThickness", int(70)),
            ],
        )
        .build();
    let set = ok(&file);
    assert_eq!(set.brushes.len(), 1);
    assert!(set.skipped.is_empty() && set.notes.is_empty());
    let b = &set.brushes[0];
    assert_eq!(b.name, "G-pen");
    assert_eq!(b.source, Source::ClipStudioSut);
    assert_eq!(b.source.label(), "CLIP STUDIO SUT");
    assert_eq!(b.unrepresented, vec![], "表せないものは無い");
    let s = &b.brush.base;
    assert_eq!(s.radius, 20.0, "直径 40 → 半径 20");
    assert_eq!(s.opacity, 0.8);
    assert_eq!(s.flow, 0.6);
    assert_eq!(s.hardness, 0.5);
    assert_eq!(s.spacing, 0.25);
    assert_eq!(b.brush.tip.roundness, 0.7);
    assert!(b.brush.tip.image.is_none() && b.brush.tip.images.is_empty());
    assert!(
        b.brush.influences == yolu_core::Influences::none(),
        "影響元が無ければ何も使わない"
    );
    assert!(b.brush.texture.is_none());
}

#[test]
fn ratios_may_be_stored_as_fractions_and_everything_is_clamped_to_the_engines_range() {
    let file = SutBuilder::new()
        .brush(
            "Soft",
            3,
            &[
                ("Opacity", real(0.5)),
                ("BrushFlow", real(250.0)),
                ("BrushHardness", real(-5.0)),
                ("BrushInterval", int(100000)),
                ("BrushThickness", int(0)),
                ("BrushSize", real(1e9)),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    let s = &b.brush.base;
    assert_eq!((s.opacity, s.flow, s.hardness), (0.5, 1.0, 0.0));
    assert_eq!(s.spacing, 4.0);
    assert_eq!(b.brush.tip.roundness, 0.01);
    assert_eq!(s.radius, 1000.0, "直径は 2000 まで");
    assert!(b.brush.validate().is_ok());
}

#[test]
fn a_file_without_known_columns_still_gives_a_default_brush_named_after_the_node() {
    let file = SutBuilder::new()
        .brush("Plain", 1, &[("SomethingNew", int(1))])
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(b.name, "Plain");
    assert_eq!(b.brush.base.radius, yolu_core::Brush::default().base.radius);
    assert!(b.unrepresented.is_empty());
}

// ---------------- 筆先の画像 ----------------

#[test]
fn a_tip_stored_only_in_the_proprietary_format_is_left_out_and_reported() {
    let file = SutBuilder::new()
        .material(
            Some("tip_a"),
            tar(&[("data/material.layer", b"\x89C2F\r\n\x1a\nbody")]),
        )
        .brush(
            "Stamp",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["C:\\mats\\tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert!(
        b.brush.tip.image.is_none() && b.brush.tip.images.is_empty(),
        "丸い筆先"
    );
    assert_eq!(b.brush.base.radius, 15.0, "設定は取り込む");
    assert!(has(b, SutNote::TipMissing));
    assert!(
        has(b, SutNote::ProprietaryImage),
        "独自の形式で読めないことを知らせる"
    );
}

#[test]
fn the_tip_is_the_preview_png_of_the_referenced_material() {
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip_png()))
        .brush(
            "Stamp",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["C:\\mats\\tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    let tip = b.brush.tip.image.as_ref().unwrap();
    assert_eq!((tip.width(), tip.height()), (2, 2));
    // PNG の上の行は筆先では後ろの行。暗い画素が塗る
    assert_eq!(alpha(b), [0, 255, 255, 0]);
    assert!(b.brush.tip.images.is_empty());
    assert_eq!(b.brush.base.radius, 15.0);
    assert!(
        has(b, SutNote::PreviewImage),
        "プレビューの画像なので知らせる"
    );
    assert!(!has(b, SutNote::TipMissing) && !has(b, SutNote::TipOrder));
    assert!(
        !has(b, SutNote::TipGuessed),
        "名前で決まった筆先は推定ではない"
    );
}

#[test]
fn the_tip_follows_the_pngs_gray_levels_not_just_black_and_white() {
    let png = png_gray(2, 2, &[255, 200, 100, 0]);
    let file = SutBuilder::new()
        .material(Some("t"), material_with_thumbnail(&png))
        .brush(
            "Levels",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["t.png", "c/t", "t"]])),
                ),
            ],
        )
        .build();
    // 下の行 = PNG の 2 行目 [100, 0] → 被覆率 (255 − 輝度)、上の行 [255, 200] → [0, 55]
    assert_eq!(alpha(&ok(&file).brushes[0]), [155, 255, 0, 55]);
    // 大きさの列が無ければ、筆先の長い辺が直径
    assert_eq!(ok(&file).brushes[0].brush.base.radius, 1.0);
}

#[test]
fn several_tips_keep_the_references_order_and_say_the_selection_order_is_not_read() {
    let a = png_gray(2, 1, &[0, 0]);
    let b = png_gray(1, 3, &[0, 0, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .material(Some("tip_b"), material_with_thumbnail(&b))
        .brush(
            "Multi",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["x/tip_b.png", "c/b", "tip_b"],
                        ["x/tip_a.png", "c/a", "tip_a"],
                    ])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    let dims: Vec<(u32, u32)> = br
        .brush
        .tip
        .images
        .iter()
        .map(|t| (t.width(), t.height()))
        .collect();
    assert_eq!(dims, [(1, 3), (2, 1)], "参照の順（b, a）");
    assert!(br.brush.tip.image.is_none());
    assert_eq!(br.brush.tip.selection, yolu_core::TipSelection::Random);
    assert!(has(br, SutNote::TipOrder));
    assert_eq!(br.brush.base.radius, 1.5, "直径は長い辺の最大（3）");
}

#[test]
fn materials_without_names_are_matched_by_order_when_the_references_are_complete() {
    let a = png_gray(1, 1, &[0]);
    let b = png_gray(2, 2, &[0; 4]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&a))
        .material(None, material_with_thumbnail(&b))
        .brush(
            "ByOrder",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["unknown-1.png", "c/u1", "u1"],
                        ["unknown-2.png", "c/u2", "u2"],
                    ])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert_eq!(br.brush.tip.images.len(), 2);
    assert_eq!(br.brush.tip.images[0].width(), 1);
    assert_eq!(br.brush.tip.images[1].width(), 2);
    assert!(
        has(br, SutNote::TipGuessed),
        "並びで当てたことを知らせる（別の画像かもしれない）"
    );
    assert!(!has(br, SutNote::TipMissing));
}

#[test]
fn unreadable_references_use_all_materials_when_the_brush_has_no_texture() {
    let a = png_gray(1, 1, &[0]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&a))
        .brush(
            "Garbage",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                ("BrushPatternImageArray", blob(vec![1, 2, 3])),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert!(br.brush.tip.image.is_some());
    assert!(
        has(br, SutNote::TipGuessed),
        "参照を読めずに素材すべてを筆先にしたことを知らせる"
    );
}

#[test]
fn an_unidentifiable_material_gives_a_round_tip_and_says_so() {
    // 筆先と質感の両方を使うブラシで、名前が当たらない: 並びでは決めない
    let a = png_gray(1, 1, &[0]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&a))
        .material(None, material_with_thumbnail(&a))
        .brush(
            "Lost",
            1,
            &[
                ("BrushSize", real(24.0)),
                ("BrushHardness", int(30)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["nowhere.png", "c/n", "n"]])),
                ),
                ("TextureImage", blob(refs(&[["elsewhere.png", "c/e", "e"]]))),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert!(
        br.brush.tip.image.is_none() && br.brush.tip.images.is_empty(),
        "丸い筆先"
    );
    assert_eq!(br.brush.base.hardness, 0.3, "丸い筆先は硬さが効く");
    assert!(has(br, SutNote::TipMissing));
    assert!(has(br, SutNote::TextureMissing));
    assert!(br.brush.texture.is_none());
}

#[test]
fn references_that_name_nothing_and_cannot_be_matched_by_order_give_no_tip() {
    // 参照が 2 つなのに素材が 1 つ: 名前も当たらず並びでも足りないので、当てずっぽうで 1 つの素材を使わない
    let a = png_gray(1, 1, &[0]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&a))
        .brush(
            "Short",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["a.png", "c/a", "a"], ["b.png", "c/b", "b"]])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert!(br.brush.tip.image.is_none() && br.brush.tip.images.is_empty());
    assert!(has(br, SutNote::TipMissing));
}

#[test]
fn more_materials_than_references_are_not_matched_by_order() {
    // 参照 2 つの名前はどの素材にも当たらず、素材は 3 つ（別の設定の素材も入りうる）: 先頭の素材を当てずっぽうで使わない
    let a = png_gray(1, 1, &[0]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&a))
        .material(None, material_with_thumbnail(&a))
        .material(None, material_with_thumbnail(&a))
        .brush(
            "Extra",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["a.png", "c/a", "a"], ["b.png", "c/b", "b"]])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert!(br.brush.tip.image.is_none() && br.brush.tip.images.is_empty());
    assert!(has(br, SutNote::TipMissing));
    assert!(!has(br, SutNote::TipGuessed));
}

#[test]
fn a_brush_where_only_some_tips_are_found_says_a_tip_is_missing() {
    // 参照 2 つのうち 1 つだけ名前が当たり、もう 1 つは当たらない。素材は 1 つで、並びでは決められない
    let a = png_gray(2, 2, &[0, 255, 255, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .brush(
            "Half",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["x/tip_a.png", "c/a", "tip_a"],
                        ["x/tip_b.png", "c/b", "tip_b"],
                    ])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert_eq!(alpha(br), [0, 255, 255, 0], "当たった筆先は使う");
    assert!(
        has(br, SutNote::TipMissing),
        "欠けた筆先を黙って捨てない: {:?}",
        br.unrepresented
    );
    assert!(!has(br, SutNote::TipGuessed));
}

#[test]
fn a_brush_with_one_broken_tip_image_keeps_the_other_and_says_a_tip_is_missing() {
    let a = png_gray(2, 2, &[0, 255, 255, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .material(Some("tip_b"), b"not a tar and not a png".to_vec())
        .brush(
            "Broken",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["x/tip_a.png", "c/a", "tip_a"],
                        ["x/tip_b.png", "c/b", "tip_b"],
                    ])),
                ),
            ],
        )
        .build();
    let set = ok(&file);
    let br = &set.brushes[0];
    assert_eq!(alpha(br), [0, 255, 255, 0]);
    assert!(has(br, SutNote::TipMissing));
    assert_eq!(set.notes, vec![note(SutNote::MaterialsUnreadable(1))]);
}

#[test]
fn a_reference_list_that_could_not_be_read_to_the_end_says_a_tip_may_be_missing() {
    let a = png_gray(2, 2, &[0, 255, 255, 0]);
    let full = refs(&[
        ["x/tip_a.png", "c/a", "tip_a"],
        ["x/tip_b.png", "c/b", "tip_b"],
    ]);
    let cut = full[..full.len() - 12].to_vec();
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .brush(
            "Cut",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                ("BrushPatternImageArray", blob(cut)),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert_eq!(alpha(br), [0, 255, 255, 0], "読めた参照の筆先は使う");
    assert!(has(br, SutNote::TipMissing));
}

#[test]
fn hundreds_of_references_to_one_material_stay_within_the_engines_tip_limit() {
    // 同じ素材を指す参照 300 個: 筆先は 256 枚で打ち切り、使わなかった分を知らせる（core の検証で全体が断られない）
    let a = png_gray(2, 2, &[0, 255, 255, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .brush(
            "Many",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&vec![["x/tip_a.png", "c/a", "tip_a"]; 300])),
                ),
            ],
        )
        .brush("Other", 2, &[("BrushSize", real(10.0))])
        .build();
    let set = ok(&file);
    assert_eq!(
        set.brushes.len(),
        2,
        "同じファイルのほかのブラシも取り込める"
    );
    let br = &set.brushes[0];
    assert_eq!(br.brush.tip.images.len(), 256);
    assert!(br.brush.validate().is_ok());
    assert!(has(br, SutNote::TipMissing));
    assert!(has(br, SutNote::TipOrder));
}

#[test]
fn node_numbers_and_names_may_be_stored_as_text_or_blob() {
    use rusqlite::Connection;
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE Node (NodeName BLOB, NodeVariantId TEXT, NodeInitVariantId TEXT);
         INSERT INTO Node VALUES (x'4661726d', '7', '0');
         CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
         INSERT INTO Variant VALUES (7, 12);",
    )
    .unwrap();
    let set = ok(&conn.serialize("main").unwrap());
    assert_eq!(set.brushes[0].name, "Farm");
    assert_eq!(set.brushes[0].brush.base.radius, 6.0);
}

#[test]
fn a_material_without_a_readable_image_is_counted_once_for_the_file() {
    let wide = png_gray(3000, 1, &vec![0u8; 3000]);
    let file = SutBuilder::new()
        .material(Some("junk"), b"not a tar and not a png".to_vec())
        .material(Some("wide"), material_with_thumbnail(&wide))
        .brush(
            "A",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["junk.png", "c/j", "junk"],
                        ["wide.png", "c/w", "wide"],
                    ])),
                ),
            ],
        )
        .brush(
            "B",
            2,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["wide.png", "c/w", "wide"]])),
                ),
            ],
        )
        .build();
    let set = ok(&file);
    for b in &set.brushes {
        assert!(has(b, SutNote::TipMissing), "{}", b.name);
        assert!(b.brush.tip.image.is_none());
    }
    // 画像の無い素材（junk）と、大きすぎる画像（wide）で 2 つ。ファイル全体に 1 回だけ
    assert_eq!(set.notes, vec![note(SutNote::MaterialsUnreadable(2))]);
}

#[test]
fn a_plain_png_or_an_embedded_one_is_also_taken_as_the_materials_image() {
    let png = tip_png();
    let mut wrapped = b"C2F-HEADER".to_vec();
    wrapped.extend_from_slice(&png);
    wrapped.extend_from_slice(b"tail");
    let file = SutBuilder::new()
        .material(Some("plain"), png.clone())
        .material(Some("wrapped"), wrapped)
        .brush(
            "P",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[
                        ["plain.png", "c/p", "plain"],
                        ["wrapped.png", "c/w", "wrapped"],
                    ])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert_eq!(br.brush.tip.images.len(), 2);
    assert_eq!(br.brush.tip.images[0].alpha(), [0, 255, 255, 0]);
    assert_eq!(br.brush.tip.images[1].alpha(), [0, 255, 255, 0]);
}

// ---------------- 質感 ----------------

#[test]
fn the_texture_is_the_referenced_image_where_white_paints() {
    let png = png_gray(2, 2, &[255, 200, 100, 0]);
    let build = |reverse: i64| {
        SutBuilder::new()
            .material(Some("grain"), material_with_thumbnail(&png))
            .brush(
                "Textured",
                1,
                &[
                    (
                        "TextureImage",
                        blob(refs(&[["g/grain.png", "c/grain", "grain"]])),
                    ),
                    ("TextureDensity", int(40)),
                    ("TextureScale2", int(200)),
                    ("TextureReverseDensity", int(reverse)),
                ],
            )
            .build()
    };
    let br = &ok(&build(0)).brushes[0];
    let t = br.brush.texture.as_ref().unwrap();
    assert_eq!(
        t.image.alpha(),
        [100, 0, 255, 200],
        "白（輝度が高い所）が塗れる。行は下から"
    );
    assert_eq!((t.depth, t.scale), (0.4, 2.0));
    assert_eq!(t.mode, yolu_core::TextureMode::Multiply);
    assert!(has(br, SutNote::PreviewImage));
    assert!(br.brush.tip.image.is_none(), "質感の素材は筆先にしない");
    let reversed = &ok(&build(1)).brushes[0];
    assert_eq!(
        reversed.brush.texture.as_ref().unwrap().image.alpha(),
        [155, 255, 0, 55]
    );
}

#[test]
fn a_texture_alone_with_one_material_is_found_even_when_the_reference_does_not_name_it() {
    let png = png_gray(1, 1, &[128]);
    let mut builder = SutBuilder::new();
    builder.material_names = false;
    let file = builder
        .material(None, material_with_thumbnail(&png))
        .brush("T", 1, &[("TextureImage", blob(vec![9, 9, 9]))])
        .build();
    let br = &ok(&file).brushes[0];
    let t = br
        .brush
        .texture
        .as_ref()
        .expect("素材が 1 つで筆先に使っていないので、それ");
    assert_eq!((t.depth, t.scale), (1.0, 1.0));
    assert!(has(br, SutNote::TextureGuessed), "推定なので知らせる");
}

#[test]
fn a_texture_whose_first_reference_is_not_found_does_not_borrow_the_second() {
    // 質感の参照が 2 つで、1 つ目の名前は当たらず 2 つ目だけ当たる。筆先も使うので並びでは決めず、2 つ目の素材を質感にしない
    let tip = png_gray(2, 2, &[0, 255, 255, 0]);
    let grain = png_gray(2, 2, &[255, 200, 100, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip))
        .material(Some("grain"), material_with_thumbnail(&grain))
        .brush(
            "Second",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["x/tip_a.png", "c/a", "tip_a"]])),
                ),
                (
                    "TextureImage",
                    blob(refs(&[
                        ["nowhere.png", "c/n", "nowhere"],
                        ["g/grain.png", "c/g", "grain"],
                    ])),
                ),
            ],
        )
        .build();
    let br = &ok(&file).brushes[0];
    assert!(br.brush.texture.is_none(), "別の素材を質感にしない");
    assert!(has(br, SutNote::TextureMissing));
    assert!(!has(br, SutNote::TextureGuessed));
}

#[test]
fn texture_each_tip_is_reported_only_when_set() {
    // 描点ごとに質感は表せない。回転・明るさ・コントラストは写すので、知らせない
    let png = png_gray(1, 1, &[128]);
    let build = |on: i64| {
        SutBuilder::new()
            .material(Some("grain"), material_with_thumbnail(&png))
            .brush(
                "T",
                1,
                &[
                    ("TextureImage", blob(refs(&[["grain.png", "c/g", "grain"]]))),
                    ("TextureRotate", int(on * 45)),
                    ("TextureBrightness", int(on * 10)),
                    ("TextureContrast", int(on * -10)),
                    ("TextureForPlot", int(on)),
                ],
            )
            .build()
    };
    let on = &ok(&build(1)).brushes[0];
    assert!(has(on, SutNote::TextureEachTip));
    assert!(
        has(on, SutNote::TextureToneApprox),
        "明るさ・コントラストは近い式なので知らせる"
    );
    for n in [
        SutNote::TextureRotation,
        SutNote::TextureBrightness,
        SutNote::TextureContrast,
    ] {
        assert!(!has(on, n.clone()), "{n:?} は写したので知らせない");
    }
    for m in [SutMapped::TextureRotation, SutMapped::TextureTone] {
        assert!(mapped(on, m), "{m:?}");
    }
    let off = &ok(&build(0)).brushes[0];
    assert!(off.brush.texture.is_some());
    assert_eq!(off.unrepresented, vec![note(SutNote::PreviewImage)]);
    assert!(!mapped(off, SutMapped::TextureRotation) && !mapped(off, SutMapped::TextureTone));
}

/// 4 画素の質感（輝度 255・200・100・0。覆いは行が下から [100, 0, 255, 200]）に、回転・明るさ・コントラストを付けた 1 本のブラシ。
fn toned(reverse: i64, columns: &[(&'static str, i64)]) -> ImportedBrush {
    let png = png_gray(2, 2, &[255, 200, 100, 0]);
    let mut cells = vec![
        (
            "TextureImage",
            blob(refs(&[["g/grain.png", "c/g", "grain"]])),
        ),
        ("TextureReverseDensity", int(reverse)),
    ];
    cells.extend(columns.iter().map(|&(name, v)| (name, int(v))));
    let file = SutBuilder::new()
        .material(Some("grain"), material_with_thumbnail(&png))
        .brush("Toned", 1, &cells)
        .build();
    ok(&file).brushes[0].clone()
}

fn texture_alpha(b: &ImportedBrush) -> Vec<u8> {
    b.brush.texture.as_ref().unwrap().image.alpha().to_vec()
}

#[test]
fn texture_rotation_is_carried_as_an_angle_and_does_not_touch_the_image() {
    let plain = toned(0, &[]);
    let turned = toned(0, &[("TextureRotate", 143)]);
    assert_eq!(plain.brush.texture.as_ref().unwrap().angle, 0.0);
    assert_eq!(turned.brush.texture.as_ref().unwrap().angle, 143.0);
    // 画像は回さない（継ぎ目が崩れない）
    assert_eq!(texture_alpha(&plain), texture_alpha(&turned));
    assert!(mapped(&turned, SutMapped::TextureRotation));
    assert!(!mapped(&plain, SutMapped::TextureRotation));
    // 負の角度は 0〜360 に直す
    assert_eq!(
        toned(0, &[("TextureRotate", -30)])
            .brush
            .texture
            .as_ref()
            .unwrap()
            .angle,
        330.0
    );
}

#[test]
fn texture_brightness_and_contrast_are_baked_into_the_image() {
    // 補正なしは今までと同じ画素
    assert_eq!(texture_alpha(&toned(0, &[])), [100, 0, 255, 200]);
    // 明るさ +20: 値に 0.2 を足して 0〜1 に収める
    let bright = toned(0, &[("TextureBrightness", 20)]);
    assert_eq!(texture_alpha(&bright), [151, 51, 255, 251]);
    assert!(mapped(&bright, SutMapped::TextureTone) && has(&bright, SutNote::TextureToneApprox));
    // コントラスト +50: 0.5 を中心に 1.5 倍に広げる。-100 は 0.5 の一様
    assert_eq!(
        texture_alpha(&toned(0, &[("TextureContrast", 50)])),
        [86, 0, 255, 236]
    );
    assert_eq!(
        texture_alpha(&toned(0, &[("TextureContrast", -100)])),
        [128, 128, 128, 128]
    );
    // 反転は補正のあと（補正した画像を反転する）
    assert_eq!(
        texture_alpha(&toned(1, &[("TextureBrightness", 20)])),
        [104, 204, 0, 4]
    );
    // 範囲の外は -100〜100 に収める。0 は補正なし（印も付けない）
    assert_eq!(
        texture_alpha(&toned(0, &[("TextureBrightness", 500)])),
        [255, 255, 255, 255]
    );
    let none = toned(0, &[("TextureBrightness", 0)]);
    assert!(!mapped(&none, SutMapped::TextureTone) && !has(&none, SutNote::TextureToneApprox));
}

#[test]
fn the_same_material_with_different_tones_gives_different_images() {
    // 同じ素材でも補正が違えば別の画像（取り込みの間の使い回しは、補正も見て決める）
    let png = png_gray(2, 2, &[255, 200, 100, 0]);
    let brush = |name: &str, id: i64, brightness: i64| (name.to_string(), id, brightness);
    let mut builder = SutBuilder::new().material(Some("grain"), material_with_thumbnail(&png));
    for (name, id, brightness) in [brush("A", 1, 0), brush("B", 2, 30), brush("C", 3, 30)] {
        builder = builder.brush(
            &name,
            id,
            &[
                (
                    "TextureImage",
                    blob(refs(&[["g/grain.png", "c/g", "grain"]])),
                ),
                ("TextureBrightness", int(brightness)),
            ],
        );
    }
    let set = ok(&builder.build());
    let images: Vec<Vec<u8>> = set.brushes.iter().map(texture_alpha).collect();
    assert_ne!(images[0], images[1]);
    assert_eq!(images[1], images[2]);
}

// ---------------- 筆先の繰り返し方法・反転・比較暗 ----------------

/// 2 枚の筆先を持つブラシに、列を足した 1 本。
fn tipped(columns: &[(&'static str, i64)]) -> ImportedBrush {
    let a = png_gray(2, 1, &[0, 0]);
    let b = png_gray(1, 3, &[0, 0, 0]);
    let mut cells = vec![
        ("BrushUsePatternImage", int(1)),
        (
            "BrushPatternImageArray",
            blob(refs(&[
                ["x/tip_a.png", "c/a", "tip_a"],
                ["x/tip_b.png", "c/b", "tip_b"],
            ])),
        ),
    ];
    cells.extend(columns.iter().map(|&(name, v)| (name, int(v))));
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .material(Some("tip_b"), material_with_thumbnail(&b))
        .brush("Multi", 1, &cells)
        .build();
    ok(&file).brushes[0].clone()
}

#[test]
fn the_six_repeat_methods_are_carried_and_the_close_ones_are_said_so() {
    use yolu_core::TipSelection as T;
    for (value, expected, approximate) in [
        (0, T::Sequential, false),
        (1, T::PingPong, true),
        (2, T::Hold, false),
        (3, T::Random, false),
        (4, T::Once, false),
        (5, T::Shuffle, true),
    ] {
        // `…Type2` はいつも同じ値で、読むのは 1 つ目だけ
        let b = tipped(&[
            ("BrushPatternOrderType", value),
            ("BrushPatternOrderType2", 3),
        ]);
        assert_eq!(b.brush.tip.selection, expected, "{value}");
        assert!(mapped(&b, SutMapped::TipOrder), "{value}");
        assert!(!has(&b, SutNote::TipOrder), "{value}");
        assert_eq!(has(&b, SutNote::TipOrderApprox), approximate, "{value}");
    }
    // 列が無い・知らない値は、これまでどおりランダムにして知らせる
    for columns in [vec![], vec![("BrushPatternOrderType", 9)]] {
        let b = tipped(&columns);
        assert_eq!(b.brush.tip.selection, T::Random);
        assert!(has(&b, SutNote::TipOrder) && !mapped(&b, SutMapped::TipOrder));
    }
}

/// 筆先 1 枚のブラシに、列を足した 1 本。
fn single_tipped(columns: &[(&'static str, i64)]) -> ImportedBrush {
    let a = png_gray(2, 1, &[0, 0]);
    let mut cells = vec![
        ("BrushUsePatternImage", int(1)),
        (
            "BrushPatternImageArray",
            blob(refs(&[["x/tip_a.png", "c/a", "tip_a"]])),
        ),
    ];
    cells.extend(columns.iter().map(|&(name, v)| (name, int(v))));
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .brush("Single", 1, &cells)
        .build();
    ok(&file).brushes[0].clone()
}

/// 12 画素おきの 10 ダブの線を描いて、k 番目のダブの中心が塗れているかを並べる。
fn dabs_painted(brush: &yolu_core::Brush) -> Vec<bool> {
    use yolu_core::glam::DVec2;
    let mut b = brush.clone();
    (b.base.radius, b.base.hardness, b.base.spacing) = (4.0, 1.0, 1.5);
    b.base.color = yolu_core::Rgba8::new(0, 0, 0, 255);
    let mut d = yolu_core::Document::with_tile_size(160, 32, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    s.add_point(&mut d, 10.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut d, 118.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
    (0..10)
        .map(|k| {
            let x = 10 + 12 * k;
            d.composite_pixel(yolu_core::Channel::Color, x, 16)
                .unwrap()
                .a
                > 0
        })
        .collect()
}

#[test]
fn a_single_tip_stops_after_the_first_dab_when_the_repeat_method_says_so() {
    use yolu_core::TipSelection as T;
    // 一巡のみ・ランダムに一巡は、1 枚でも使い切ったら置かない。1 枚の並びとして持ち、最初のダブだけが描かれる
    for (value, expected) in [(4, T::Once), (5, T::Shuffle)] {
        let b = single_tipped(&[("BrushPatternOrderType", value)]);
        assert!(b.brush.tip.image.is_none(), "{value}");
        assert_eq!(b.brush.tip.images.len(), 1, "{value}");
        assert_eq!(b.brush.tip.selection, expected, "{value}");
        assert!(mapped(&b, SutMapped::TipOrder), "{value}");
        assert!(!has(&b, SutNote::TipOrder), "{value}");
        assert!(!has(&b, SutNote::TipOrderApprox), "{value}");
        assert!(b.brush.validate().is_ok(), "{value}");
        let painted = dabs_painted(&b.brush);
        assert!(painted[0], "{value}");
        assert!(painted[1..].iter().all(|p| !p), "{value}: {painted:?}");
    }
    // 止まらない方法は、これまでどおり 1 枚の筆先（`image`）のまま描き続ける
    for value in 0..=3 {
        let b = single_tipped(&[("BrushPatternOrderType", value)]);
        assert!(b.brush.tip.image.is_some() && b.brush.tip.images.is_empty());
        assert!(!mapped(&b, SutMapped::TipOrder) && !has(&b, SutNote::TipOrder));
        assert!(dabs_painted(&b.brush).iter().all(|p| *p), "{value}");
    }
}

#[test]
fn the_four_flip_methods_are_carried_for_each_axis() {
    use yolu_core::FlipRule as R;
    let tip = |b: &ImportedBrush| {
        let t = &b.brush.tip;
        (t.flip_x, t.flip_x_rule, t.flip_y, t.flip_y_rule)
    };
    for (value, expected) in [
        (0, (false, R::Fixed)),
        (1, (true, R::Fixed)),
        (2, (false, R::Random)),
        (3, (false, R::OnFold)),
    ] {
        let h = tipped(&[("BrushPatternReverseHorizontal", value)]);
        assert_eq!(
            tip(&h),
            (expected.0, expected.1, false, R::Fixed),
            "{value}"
        );
        let v = tipped(&[("BrushPatternReverseVertical", value)]);
        assert_eq!(
            tip(&v),
            (false, R::Fixed, expected.0, expected.1),
            "{value}"
        );
        assert_eq!(mapped(&h, SutMapped::TipFlip), value != 0, "{value}");
        assert!(!has(&h, SutNote::TipFlip));
    }
    // 知らない値は反転せずに知らせる
    let odd = tipped(&[("BrushPatternReverseHorizontal", 7)]);
    assert_eq!(tip(&odd), (false, R::Fixed, false, R::Fixed));
    assert!(has(&odd, SutNote::TipFlip) && !mapped(&odd, SutMapped::TipFlip));
    // 1 枚の筆先にも、丸い筆先でない限り効く。丸い筆先には写さない（反転しても同じ）
    let round = SutBuilder::new()
        .brush("Round", 1, &[("BrushPatternReverseHorizontal", int(1))])
        .build();
    let round = &ok(&round).brushes[0];
    assert!(!round.brush.tip.flip_x && !mapped(round, SutMapped::TipFlip));
}

#[test]
fn blending_tips_with_darken_is_carried_for_round_and_image_tips() {
    assert!(
        tipped(&[("BrushBlendPatternByDarken", 1)])
            .brush
            .tip
            .darken_overlap
    );
    assert!(
        !tipped(&[("BrushBlendPatternByDarken", 0)])
            .brush
            .tip
            .darken_overlap
    );
    assert!(!tipped(&[]).brush.tip.darken_overlap);
    let round = SutBuilder::new()
        .brush("Round", 1, &[("BrushBlendPatternByDarken", int(1))])
        .build();
    let round = &ok(&round).brushes[0];
    assert!(round.brush.tip.darken_overlap && mapped(round, SutMapped::TipDarken));
    assert!(round.unrepresented.is_empty());
}

// ---------------- 影響元 ----------------

fn pressure_curve() -> Vec<(f64, f64)> {
    vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]
}

#[test]
fn pressure_becomes_a_minimum_and_a_curve() {
    let curve = pressure_curve();
    for header in [40usize, 44] {
        let file = SutBuilder::new()
            .brush(
                "Pressure",
                1,
                &[(
                    "BrushSizeEffector",
                    blob(effector(header, 0x10, 30, &[&curve])),
                )],
            )
            .build();
        let b = &ok(&file).brushes[0];
        assert!(b.brush.influences.size.pressure.on && !b.brush.influences.opacity.pressure.on);
        let r = &b.brush.influences.size.pressure.response;
        assert_eq!(r.min(), 0.3);
        let points: Vec<(f64, f64)> = r.curve().iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(points, curve);
        assert!(b.unrepresented.is_empty());
        // 最小 0.3 + 0.7 × 曲線(0.5) = 0.3 + 0.7 × 0.8
        assert!((r.apply(0.5) - (0.3 + 0.7 * 0.8)).abs() < 1e-9);
    }
}

#[test]
fn opacity_and_flow_effectors_map_to_their_own_responses() {
    let file = SutBuilder::new()
        .brush(
            "Both",
            1,
            &[
                (
                    "OpacityEffector",
                    blob(effector(44, 0x10, 10, &[&[(0.0, 0.0), (1.0, 1.0)]])),
                ),
                (
                    "BrushFlowEffector",
                    blob(effector(44, 0x10, 0, &[&pressure_curve()])),
                ),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert!(
        b.brush.influences.opacity.pressure.on
            && b.brush.influences.flow.pressure.on
            && !b.brush.influences.size.pressure.on
    );
    assert_eq!(b.brush.influences.opacity.pressure.response.min(), 0.1);
    assert!(
        b.brush
            .influences
            .opacity
            .pressure
            .response
            .curve()
            .is_empty(),
        "直線は曲線なし"
    );
    assert_eq!(b.brush.influences.flow.pressure.response.curve().len(), 3);
    assert!(b.brush.influences.size.pressure.response.is_identity());
}

#[test]
fn every_input_of_an_influence_is_mapped_with_its_minimum() {
    let rising = [(0.0, 0.0), (1.0, 1.0)];
    let file = SutBuilder::new()
        .brush(
            "Inputs",
            1,
            &[
                (
                    "BrushSizeEffector",
                    blob(effector_full(
                        44,
                        0x10 | 0x40 | 0x80,
                        [30, 100, 45, 60],
                        None,
                        None,
                    )),
                ),
                (
                    "OpacityEffector",
                    blob(effector_full(48, 0x20, [0, 25, 0, 0], None, Some(&rising))),
                ),
                (
                    "BrushFlowEffector",
                    blob(effector(44, 0, 0, &[&pressure_curve()])),
                ),
                ("BrushThicknessEffector", blob(effector(44, 0x10, 40, &[]))),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    let i = &b.brush.influences;
    // 大きさ: 筆圧・速さ・ランダムを使い、最小値はヘッダーの 4・6・7 番目。傾きは旗が無いので使わない
    assert!(i.size.pressure.on && i.size.speed.on && i.size.random.on && !i.size.tilt.on);
    assert_eq!(
        [
            i.size.pressure.response.min(),
            i.size.speed.response.min(),
            i.size.random.response.min()
        ],
        [0.3, 0.45, 0.6]
    );
    assert!(
        i.size.tilt.response.is_identity(),
        "使わない入力は既定のまま"
    );
    // 不透明度: 頭が 48 バイトでも傾きの最小値と曲線を読む。曲線の x は直立が 0 なので、直立 1 の入力へ反転する
    assert!(i.opacity.tilt.on && !i.opacity.pressure.on);
    assert_eq!(i.opacity.tilt.response.min(), 0.25);
    let points: Vec<(f64, f64)> = i
        .opacity
        .tilt
        .response
        .curve()
        .iter()
        .map(|p| (p.x, p.y))
        .collect();
    assert_eq!(points, [(0.0, 1.0), (1.0, 0.0)]);
    // 旗の無い流量は使わない。厚さの筆圧は真円率へ
    assert!(!i.flow.is_on());
    assert!(i.roundness.pressure.on);
    assert_eq!(i.roundness.pressure.response.min(), 0.4);
    for item in [
        SutMapped::Pressure,
        SutMapped::Tilt,
        SutMapped::Speed,
        SutMapped::Random,
    ] {
        assert!(b.mapped.contains(&item), "{item:?}");
    }
    // 速さ・ランダムの曲線は読めないので、直線にして知らせる（ほかの組は注記しない）
    for input in [SutInput::Speed, SutInput::Random] {
        assert!(has(
            b,
            SutNote::InfluenceCurve {
                target: SutTarget::Size,
                input,
            }
        ));
    }
    assert_eq!(b.unrepresented.len(), 2, "{:?}", b.unrepresented);
    assert_eq!(b.brush.tip.roundness, 1.0);
}

#[test]
fn texture_density_and_paint_influences_are_mapped_only_where_they_apply() {
    let fall = falling();
    let cells = |mixing: i64| {
        vec![
            (
                "TextureImage",
                blob(refs(&[["g/grain.png", "c/grain", "grain"]])),
            ),
            (
                "TextureDensityEffector",
                blob(effector_full(44, 0x30, [20, 10, 0, 0], None, Some(&fall))),
            ),
            ("BrushUseWaterColor", int(mixing)),
            ("BrushMixColor", int(60)),
            ("BrushMixAlpha", int(80)),
            (
                "BrushMixColorEffector",
                blob(effector_full(44, 0x10, [35, 0, 0, 0], None, None)),
            ),
            (
                "BrushMixAlphaEffector",
                blob(effector_full(44, 0x80, [0, 0, 0, 50], None, None)),
            ),
        ]
    };
    let file = SutBuilder::new()
        .material(
            Some("grain"),
            material_with_thumbnail(&png_gray(2, 2, &[255, 200, 100, 0])),
        )
        .brush("Mixing", 1, &cells(1))
        .brush("Plain", 2, &cells(0))
        .build();
    let set = ok(&file);
    let mixing = &set.brushes[0].brush;
    assert!(mixing.texture.is_some() && mixing.mix.is_active());
    let i = &mixing.influences;
    assert!(i.texture_density.pressure.on && i.texture_density.tilt.on);
    assert_eq!(i.texture_density.pressure.response.min(), 0.2);
    assert_eq!(i.texture_density.tilt.response.min(), 0.1);
    assert!(i.mix_paint.pressure.on && !i.mix_paint.random.on);
    assert_eq!(i.mix_paint.pressure.response.min(), 0.35);
    assert!(i.mix_density.random.on && !i.mix_density.pressure.on);
    assert_eq!(i.mix_density.random.response.min(), 0.5);
    // 混ぜないブラシには、絵の具の影響元を写さない（効く所が無い）
    let plain = &set.brushes[1].brush;
    assert!(!plain.mix.is_active());
    assert!(!plain.influences.mix_paint.is_on() && !plain.influences.mix_density.is_on());
    assert!(plain.influences.texture_density.is_on());
}

#[test]
fn a_curve_with_too_many_points_is_resampled_and_unreadable_effectors_are_reported() {
    let many: Vec<(f64, f64)> = (0..40)
        .map(|i| (i as f64 / 39.0, (i as f64 / 39.0).powi(2)))
        .collect();
    let file = SutBuilder::new()
        .brush(
            "Many",
            1,
            &[
                ("BrushSizeEffector", blob(effector(44, 0x10, 0, &[&many]))),
                ("OpacityEffector", blob(vec![1, 2, 3])),
                ("BrushFlowEffector", blob(vec![0; 300 * 1024])),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert!(has(
        b,
        SutNote::CurveSimplified(SutTarget::Size, SutInput::Pressure)
    ));
    assert_eq!(b.brush.influences.size.pressure.response.curve().len(), 16);
    assert!(has(b, SutNote::InfluenceUnreadable(SutTarget::Opacity)));
    assert!(
        has(b, SutNote::InfluenceUnreadable(SutTarget::Flow)),
        "大きすぎる値も黙って捨てない"
    );
    assert!(!b.brush.influences.opacity.pressure.on && !b.brush.influences.flow.pressure.on);
}

// ---------------- 表せない設定 ----------------

#[test]
fn the_default_mixing_values_of_a_brush_with_mixing_off_do_not_turn_mixing_on() {
    // CLIP STUDIO は混色が切のブラシ（G ペンなど）にも絵の具量・濃度・色延びの既定の値を書く
    let file = SutBuilder::new()
        .brush(
            "Pen",
            1,
            &[
                ("BrushUseWaterColor", int(0)),
                ("BrushUseWaterColor2", int(0)),
                ("BrushMixColor", int(50)),
                ("BrushMixAlpha", int(50)),
                ("BrushMixColorExtension", int(10)),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(b.brush.mix.mode, yolu_core::brush::MixMode::default());
    assert!(
        !b.unrepresented
            .iter()
            .any(|u| matches!(u, Unrepresented::ClipStudio(SutNote::ColorMixing { .. }))),
        "混色が切なら知らせもしない"
    );
}

#[test]
fn settings_the_engine_cannot_represent_are_reported_when_flagged() {
    let file = SutBuilder::new()
        .brush(
            "Painter",
            1,
            &[
                ("BrushThickness", int(50)),
                ("BrushRotation", real(30.0)),
                // 向きの影響元にペンの向きを使う（旗 0x10。下の 2 ビットは読まない）。これは写る
                ("BrushRotationEffector", int(0x13)),
                ("BrushUseWaterColor", int(1)),
                ("BrushMixColor", int(30)),
                ("BrushMixAlpha", int(20)),
                ("BrushMixColorExtension", int(10)),
                ("BrushUseWaterEdge", int(1)),
                ("BrushUseSpray", int(1)),
                ("UseDualBrush", int(1)),
                ("BrushUseIn", int(1)),
                ("BrushUseOut", int(0)),
                ("BrushUseRevision", int(1)),
                ("BrushHueChange", int(12)),
                ("CompositeMode", int(3)),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    // 色の混ぜの値は、絵の具で混ぜるブラシとして写る（知らせない）
    let mix = &b.brush.mix;
    assert_eq!(mix.mode, yolu_core::brush::MixMode::Mix);
    assert_eq!((mix.paint, mix.density, mix.stretch), (0.3, 0.2, 0.1));
    assert_eq!(b.brush.tip.angle, 30.0, "角度は写る");
    assert!(b.brush.controls.tilt_angle, "ペンの向きは写る");
    assert!(!has(b, SutNote::Direction));
    for n in [
        SutNote::Spray,
        SutNote::DualBrush,
        SutNote::StartEnd,
        SutNote::PostCorrection,
        SutNote::ColorChange,
        SutNote::BlendMode,
    ] {
        assert!(has(b, n.clone()), "{n:?}");
    }
    assert!(
        b.unrepresented.contains(&Unrepresented::WetEdges),
        "ウェットエッジは ABR と同じ注記"
    );
    assert_eq!(b.unrepresented.len(), 7);
    // 注記の文は両方の言語で空でなく、制御文字を含まない
    for n in &b.unrepresented {
        let (ja, en) = (n.to_string(), n.english());
        assert!(!ja.is_empty() && !en.is_empty() && !ja.contains('\n') && !en.contains('\n'));
    }
}

/// 注記の文は、どの種類でも両方の言語で作れ、英語に日本語が混ざらず、制御文字を含まない（種類を足したらここへも足す）。
#[test]
fn every_note_has_a_sentence_in_both_languages() {
    let all = [
        SutNote::PreviewImage,
        SutNote::TipMissing,
        SutNote::TipGuessed,
        SutNote::TipOrder,
        SutNote::TipOrderApprox,
        SutNote::TipFlip,
        SutNote::TextureToneApprox,
        SutNote::TextureMissing,
        SutNote::TextureGuessed,
        SutNote::TextureRotation,
        SutNote::TextureBrightness,
        SutNote::TextureContrast,
        SutNote::TextureMode,
        SutNote::TextureEachTip,
        SutNote::Direction,
        SutNote::ColorMixing {
            paint: 30.0,
            density: 20.0,
            stretch: 10.0,
        },
        SutNote::Spray,
        SutNote::DualBrush,
        SutNote::StartEnd,
        SutNote::Stabilizer,
        SutNote::ColorChange,
        SutNote::BlendMode,
        SutNote::InfluenceCurve {
            target: SutTarget::Thickness,
            input: SutInput::Speed,
        },
        SutNote::InfluenceCurve {
            target: SutTarget::MixPaint,
            input: SutInput::Random,
        },
        SutNote::InfluenceUnreadable(SutTarget::Flow),
        SutNote::InfluenceUnreadable(SutTarget::TextureDensity),
        SutNote::CurveSimplified(SutTarget::Opacity, SutInput::Pressure),
        SutNote::CurveSimplified(SutTarget::MixDensity, SutInput::Tilt),
        SutNote::TiltCurve(SutTarget::Flow),
        SutNote::StartEndDetail,
        SutNote::StabilizerStrength,
        SutNote::ThicknessDirection,
        SutNote::ColorMixingMethod(1.0),
        SutNote::ColorMixingMethod(2.0),
        SutNote::ColorMixingMethod(7.0),
        SutNote::PostCorrection,
        SutNote::VectorTools,
        SutNote::LengthUnit,
        SutNote::IntervalType(9.0),
        SutNote::SettingsMissing,
        SutNote::MaterialsUnreadable(3),
        SutNote::BrushesCapped(7),
        SutNote::MaterialsCapped,
        SutNote::AntiAliasing(5.0),
    ];
    for n in all {
        let note = Unrepresented::ClipStudio(n.clone());
        let (ja, en) = (note.to_string(), note.english());
        assert!(!ja.is_empty() && !en.is_empty(), "{n:?}");
        assert!(en.is_ascii(), "英語に日本語: {n:?}: {en}");
        assert!(!ja.is_ascii(), "日本語の文: {n:?}: {ja}");
        for text in [&ja, &en] {
            assert!(!text.chars().any(|c| c.is_control()), "{n:?}");
        }
    }
}

#[test]
fn the_anti_alias_column_maps_to_the_four_levels() {
    use yolu_core::AntiAlias;
    // `AntiAlias` の 0〜3 は、画面の選びの順（なし・弱・中・強）
    for (value, level) in [
        (0, AntiAlias::None),
        (1, AntiAlias::Weak),
        (2, AntiAlias::Medium),
        (3, AntiAlias::Strong),
    ] {
        let file = SutBuilder::new()
            .brush(
                "Pen",
                1,
                &[("BrushHardness", int(100)), ("AntiAlias", int(value))],
            )
            .build();
        let b = &ok(&file).brushes[0];
        assert_eq!(b.brush.base.anti_alias, level, "{value}");
        assert!(mapped(b, SutMapped::AntiAliasing), "{value}");
        assert_eq!(b.unrepresented, vec![], "{value}");
    }
    // 列が無いブラシは今の見た目のまま（なし）で、写した項目にも載らない
    let file = SutBuilder::new()
        .brush("Pen", 1, &[("BrushHardness", int(100))])
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(b.brush.base.anti_alias, AntiAlias::None);
    assert!(!mapped(b, SutMapped::AntiAliasing));
    // 知らない値は写さず（なし）、値を添えて知らせる
    for (cell, value) in [(int(4), 4.0), (int(-1), -1.0), (real(1.5), 1.5)] {
        let file = SutBuilder::new()
            .brush("Pen", 1, &[("AntiAlias", cell)])
            .build();
        let b = &ok(&file).brushes[0];
        assert_eq!(b.brush.base.anti_alias, AntiAlias::None, "{value}");
        assert!(!mapped(b, SutMapped::AntiAliasing), "{value}");
        assert!(has(b, SutNote::AntiAliasing(value)), "{value}");
    }
}

#[test]
fn flags_that_are_off_report_nothing() {
    let file = SutBuilder::new()
        .brush(
            "Quiet",
            1,
            &[
                ("BrushRotation", real(0.0)),
                ("BrushUseWaterColor", int(0)),
                ("BrushMixColor", int(0)),
                ("BrushUseWaterEdge", int(0)),
                ("BrushUseSpray", int(0)),
                ("UseDualBrush", int(0)),
                ("BrushUseIn", int(0)),
                ("BrushUseOut", int(0)),
                ("BrushUseRevision", int(0)),
                ("BrushHueChange", int(0)),
                ("CompositeMode", int(0)),
                ("FlickerReduction", int(0)),
                ("FlickerReductionBySpeed", int(1)),
                ("BrushRevisionBySpeed", int(1)),
                ("BrushUseVectorEraser", int(0)),
                ("BrushUseVectorMagnet", int(0)),
                ("BrushSizeUnit", int(0)),
                ("BrushAtLeast1Pixel", int(0)),
                ("BrushWaterColor", int(0)),
                ("BrushAutoIntervalType", int(0)),
                ("BrushVerticalThicknes", int(1)),
                ("BrushRotationEffector", int(0x03)),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(b.unrepresented, vec![]);
    assert!(!b.brush.controls.min_one_pixel);
    assert_eq!(b.mapped, vec![], "旗が切なら、写した項目も無い");
}

// ---------------- ノードと表 ----------------

#[test]
fn only_nodes_with_variants_are_brushes_and_the_current_settings_win() {
    let file = SutBuilder::new()
        .node("Tool", 0, 0)
        .node("Group", 0, 0)
        .node("A", 1, 2)
        .variant(1, &[("BrushSize", real(10.0))])
        .variant(2, &[("BrushSize", real(20.0))])
        .node("B", 0, 4)
        .variant(4, &[("BrushSize", real(40.0))])
        .node("", 5, 6)
        .variant(6, &[("BrushSize", real(60.0))])
        .node("Gone", 7, 8)
        .build();
    let set = ok(&file);
    let got: Vec<(&str, f64)> = set
        .brushes
        .iter()
        .map(|b| (b.name.as_str(), b.brush.base.radius))
        .collect();
    assert_eq!(
        got,
        [("A", 5.0), ("B", 20.0), ("File name", 30.0)],
        "現在の設定を使う・無ければ既定の設定・名前が空ならファイル名"
    );
    assert_eq!(set.skipped.len(), 1);
    assert_eq!(set.skipped[0].name, "Gone");
    assert_eq!(set.skipped[0].reason, SkipReason::SettingsNotInFile);
}

#[test]
fn a_missing_variant_table_keeps_the_names_with_default_settings_and_says_so() {
    let mut builder = SutBuilder::new().node("Named", 1, 1);
    builder.no_variant = true;
    let set = ok(&builder.build());
    assert_eq!(set.brushes[0].name, "Named");
    assert_eq!(
        set.brushes[0].unrepresented,
        vec![note(SutNote::SettingsMissing)]
    );
}

#[test]
fn settings_are_found_by_variant_id_and_never_by_the_row_number() {
    // `_PW_ID`（行の連番）は 1・2、`VariantID` は 10・20。ブラシが指す番号は `VariantID`
    let file = SutBuilder::new()
        .node("Second", 20, 20)
        .node("ByRowNumber", 2, 2)
        .variant(10, &[("BrushSize", real(10.0))])
        .variant(20, &[("BrushSize", real(40.0))])
        .build();
    let set = ok(&file);
    assert_eq!(set.brushes.len(), 1);
    assert_eq!(set.brushes[0].name, "Second");
    assert_eq!(set.brushes[0].brush.base.radius, 20.0, "VariantID 20 の行");
    assert_eq!(
        set.skipped,
        vec![yolu_io::brushes::SkippedBrush {
            name: "ByRowNumber".into(),
            reason: SkipReason::SettingsNotInFile,
        }],
        "行の連番 2 の行を、番号 2 の設定として読まない"
    );
}

#[test]
fn a_variant_table_without_variant_id_gives_default_settings_and_says_so() {
    use rusqlite::Connection;
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
         INSERT INTO Node VALUES ('NoKey', 1, 1);
         CREATE TABLE Variant (_PW_ID INTEGER PRIMARY KEY, BrushSize REAL);
         INSERT INTO Variant VALUES (1, 99);",
    )
    .unwrap();
    let set = ok(&conn.serialize("main").unwrap());
    let b = &set.brushes[0];
    assert_eq!(b.name, "NoKey");
    assert_eq!(
        b.brush.base.radius,
        yolu_core::Brush::default().base.radius,
        "行の連番の行の値を使わない"
    );
    assert_eq!(b.unrepresented, vec![note(SutNote::SettingsMissing)]);
}

#[test]
fn a_file_that_is_not_a_clip_studio_brush_is_refused_with_a_reason() {
    let mut no_node = SutBuilder::new().brush("A", 1, &[]);
    no_node.no_node = true;
    assert_eq!(faulted(read(&no_node.build())), Fault::SutNoNodeTable);
    let groups_only = SutBuilder::new().node("Tool", 0, 0).build();
    assert_eq!(faulted(read(&groups_only)), Fault::SutNoBrushes);
    assert_eq!(faulted(read(&[])), Fault::SutNotDatabase);
    assert_eq!(
        faulted(read(b"hello, this is not sqlite")),
        Fault::SutNotDatabase
    );
    let mut empty = b"SQLite format 3\0".to_vec();
    empty.extend_from_slice(&[0u8; 2000]);
    assert_eq!(faulted(read(&empty)), Fault::SutNotDatabase);
    // 署名が先頭の少し後ろにあっても読む（先頭に別のヘッダーがある形）。遠すぎれば読まない
    let file = SutBuilder::new().brush("Shifted", 1, &[]).build();
    let mut shifted = vec![0x55u8; 100];
    shifted.extend_from_slice(&file);
    assert_eq!(ok(&shifted).brushes[0].name, "Shifted");
    let mut far = vec![0x55u8; 9000];
    far.extend_from_slice(&file);
    assert_eq!(faulted(read(&far)), Fault::SutNotDatabase);
    // 画面の文に制御文字・内部の数が出ない
    for f in [
        Fault::SutNoNodeTable,
        Fault::SutNoBrushes,
        Fault::SutNotDatabase,
        Fault::SutLimits,
    ] {
        assert!(!f.to_string().is_empty() && !f.english().is_empty());
    }
}

#[test]
fn a_missing_material_table_means_no_tip_not_a_crash() {
    let mut builder = SutBuilder::new().brush(
        "NoMaterials",
        1,
        &[
            ("BrushUsePatternImage", int(1)),
            (
                "BrushPatternImageArray",
                blob(refs(&[["a.png", "c/a", "a"]])),
            ),
        ],
    );
    builder.no_material = true;
    let set = ok(&builder.build());
    assert!(has(&set.brushes[0], SutNote::TipMissing));
    assert!(set.notes.is_empty());
}

#[test]
fn a_file_is_read_from_disk_through_the_common_entry_and_the_name_falls_back_to_the_file_name() {
    let dir = std::env::temp_dir().join(format!("yolu-sut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("my_pen-brush.sut");
    let file = SutBuilder::new()
        .node("", 1, 1)
        .variant(1, &[("BrushSize", real(8.0))])
        .build();
    std::fs::write(&path, &file).unwrap();
    let set = import(&path).unwrap();
    assert_eq!(set.brushes[0].name, "My pen brush");
    assert_eq!(set.brushes[0].brush.base.radius, 4.0);
    // 読んだあとにファイルのそばへ何も作られていない（ジャーナル・一時ファイル）
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["my_pen-brush.sut"]);
    // 元のファイルは変わらない
    assert_eq!(std::fs::read(&path).unwrap(), file);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------- 上限と悪意のある入力 ----------------

#[test]
fn too_many_brushes_are_capped_and_the_file_says_how_many_were_not_read() {
    let mut builder = SutBuilder::new();
    builder.filler = 0;
    for i in 1..=300 {
        builder = builder.brush(&format!("B{i}"), i, &[("BrushSize", real(10.0))]);
    }
    let set = ok(&builder.build());
    assert_eq!(set.brushes.len(), 256);
    assert_eq!(set.notes, vec![note(SutNote::BrushesCapped(44))]);
}

#[test]
fn too_many_materials_are_capped_and_reported() {
    let mut builder = SutBuilder::new().brush(
        "M",
        1,
        &[
            ("BrushSize", real(10.0)),
            ("TextureImage", blob(vec![0; 4])),
        ],
    );
    builder.filler = 0;
    let png = png_gray(1, 1, &[0]);
    for i in 0..300 {
        builder = builder.material(Some(&format!("m{i}")), material_with_thumbnail(&png));
    }
    let set = ok(&builder.build());
    assert!(
        set.notes.contains(&note(SutNote::MaterialsCapped)),
        "{:?}",
        set.notes
    );
}

#[test]
fn a_small_file_cannot_expand_into_an_unbounded_amount_of_tips() {
    // 2048×2048 の全部 0 の PNG（数 KB）を 70 個の素材として持ち、全部を筆先にする（4 MiB × 70 = 280 MiB > 予算 256 MiB）
    let big = png_gray(2048, 2048, &vec![0u8; 2048 * 2048]);
    assert!(big.len() < 64 * 1024, "PNG は小さい: {}", big.len());
    let data = material_with_thumbnail(&big);
    let mut builder = SutBuilder::new().brush(
        "Bomb",
        1,
        &[
            ("BrushUsePatternImage", int(1)),
            ("BrushPatternImageArray", blob(vec![0; 4])),
        ],
    );
    builder.material_names = false;
    for _ in 0..70 {
        builder = builder.material(None, data.clone());
    }
    let file = builder.build();
    assert!(
        file.len() < 8 * 1024 * 1024,
        "ファイルは小さい: {}",
        file.len()
    );
    let started = Instant::now();
    assert_eq!(faulted(read(&file)), Fault::Budget);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "予算で早く断る: {:?}",
        started.elapsed()
    );
    // 予算の中なら通る（10 個）
    let mut ten = SutBuilder::new().brush(
        "Ten",
        1,
        &[
            ("BrushUsePatternImage", int(1)),
            ("BrushPatternImageArray", blob(vec![0; 4])),
        ],
    );
    ten.material_names = false;
    for _ in 0..10 {
        ten = ten.material(None, data.clone());
    }
    assert_eq!(ok(&ten.build()).brushes[0].brush.tip.images.len(), 10);
}

#[test]
fn a_view_or_virtual_table_cannot_stand_in_for_a_real_table() {
    use rusqlite::Connection;
    let make = |variant_sql: &str| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
             INSERT INTO Node VALUES ('Fake', 1, 1);",
        )
        .unwrap();
        conn.execute_batch(variant_sql).unwrap();
        conn.serialize("main").unwrap().to_vec()
    };
    let view = make("CREATE VIEW Variant AS SELECT 1 AS VariantID, 99 AS BrushSize;");
    let set = ok(&view);
    assert_eq!(
        set.brushes[0].unrepresented,
        vec![note(SutNote::SettingsMissing)],
        "ビューは表としない"
    );
    assert_eq!(
        set.brushes[0].brush.base.radius,
        yolu_core::Brush::default().base.radius,
        "ビューの値は使われていない"
    );
    let virtual_table = make("CREATE VIRTUAL TABLE Variant USING rtree(VariantID, BrushSize, b);");
    let set = ok(&virtual_table);
    assert_eq!(
        set.brushes[0].unrepresented,
        vec![note(SutNote::SettingsMissing)],
        "仮想表は表としない"
    );
}

#[test]
fn columns_that_are_not_read_are_never_evaluated() {
    use rusqlite::Connection;
    // 読まない列が、評価すると重い生成列（3000 万バイトの文字列を作る）でも、読まないので速い
    let conn = Connection::open_in_memory().unwrap();
    let mut sql = String::from("CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL");
    for i in 0..30 {
        sql.push_str(&format!(
            ", Heavy{i} INTEGER GENERATED ALWAYS AS (length(hex(zeroblob(30000000)))) VIRTUAL"
        ));
    }
    sql.push_str(");");
    conn.execute_batch(&sql).unwrap();
    conn.execute_batch(
        "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
         INSERT INTO Node VALUES ('Heavy', 1, 1);
         INSERT INTO Variant (VariantID, BrushSize) VALUES (1, 32);",
    )
    .unwrap();
    let file = conn.serialize("main").unwrap().to_vec();
    let started = Instant::now();
    let set = ok(&file);
    assert_eq!(set.brushes[0].brush.base.radius, 16.0);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn brushes_far_past_the_cap_are_all_counted_as_not_read() {
    // Node の行が 5000（ブラシでない行も 3 つ）。256 個を読み、残りのブラシの数を全部数える（見ない行を作らない）
    let mut builder = SutBuilder::new().variant(1, &[("BrushSize", real(10.0))]);
    builder.filler = 0;
    for i in 0..5000 {
        builder = builder.node(&format!("B{i}"), 1, 1);
    }
    for i in 0..3 {
        builder = builder.node(&format!("Folder{i}"), 0, 0);
    }
    let set = ok(&builder.build());
    assert_eq!(set.brushes.len(), 256);
    assert_eq!(set.notes, vec![note(SutNote::BrushesCapped(5000 - 256))]);
}

#[test]
fn queries_that_run_too_long_refuse_the_file_as_a_limit() {
    use rusqlite::Connection;
    // 番号の列に索引の無い 20 万行の `Variant` を、存在しない番号で何度も引かせる（引くたびに全件の走査）。命令の数の上限で断られる
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 256)
         INSERT INTO Node SELECT 'B' || x, 900000 + x, 800000 + x FROM c;
         CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 200000)
         INSERT INTO Variant SELECT x, 10 FROM c;",
    )
    .unwrap();
    let file = conn.serialize("main").unwrap().to_vec();
    assert!(
        file.len() < 16 * 1024 * 1024,
        "ファイルは小さい: {}",
        file.len()
    );
    let started = Instant::now();
    assert_eq!(faulted(read(&file)), Fault::SutLimits);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "上限で早く断る: {:?}",
        started.elapsed()
    );
}

#[test]
fn an_expression_that_loads_an_extension_never_loads_one() {
    use rusqlite::Connection;
    // 生成列に `load_extension` を置いたスキーマ（SQLite 自身は作らせないので、スキーマの文を書き換えて作る）。読む列の名前
    // （Opacity・BrushFlow）でも、拡張は開かれない。SQL から呼べないことは `sut::db` の試験（`load_extension_is_refused_from_sql`）
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
         INSERT INTO Node VALUES ('Ext', 1, 1);
         CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
         INSERT INTO Variant (VariantID, BrushSize) VALUES (1, 32);
         PRAGMA writable_schema = ON;
         UPDATE sqlite_master SET sql = replace(sql, 'BrushSize REAL)',
           'BrushSize REAL, Opacity INTEGER GENERATED ALWAYS AS (load_extension(''/nonexistent/extension'')) VIRTUAL, BrushFlow INTEGER GENERATED ALWAYS AS (load_extension(''/nonexistent/extension'', ''entry'')) VIRTUAL)')
           WHERE name = 'Variant';",
    )
    .unwrap();
    let file = conn.serialize("main").unwrap().to_vec();
    // 実測では、SQLite がこのスキーマ自体を断る（`load_extension` は生成列などのスキーマの中では使えない関数）。読み手はそれを
    // 「SQLite として読めない」として断る
    assert_eq!(faulted(read(&file)), Fault::SutNotDatabase);
}

#[test]
fn a_database_saved_in_wal_mode_is_read_as_a_plain_file() {
    use rusqlite::Connection;
    // WAL の印がヘッダーにあるファイル（チェックポイント済みの 1 つのファイル）。メモリからは開けない形なので、読み手が普通の形式として読む
    let dir = std::env::temp_dir().join(format!("yolu-sut-wal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.sqlite");
    {
        let conn = Connection::open(&path).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        conn.execute_batch(
            "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
             INSERT INTO Node VALUES ('Wal', 1, 1);
             CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
             INSERT INTO Variant VALUES (1, 22);
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!((bytes[18], bytes[19]), (2, 2), "WAL の印");
    let set = ok(&bytes);
    assert_eq!(set.brushes[0].name, "Wal");
    assert_eq!(set.brushes[0].brush.base.radius, 11.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_schema_with_too_many_tables_is_refused() {
    use rusqlite::Connection;
    let file = |tables: usize| {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE Node (NodeName TEXT, NodeVariantId INTEGER, NodeInitVariantId INTEGER);
             INSERT INTO Node VALUES ('Many', 1, 1);
             CREATE TABLE Variant (VariantID INTEGER, BrushSize REAL);
             INSERT INTO Variant VALUES (1, 10);",
        )
        .unwrap();
        for i in 0..tables {
            conn.execute_batch(&format!("CREATE TABLE T{i} (a INTEGER);"))
                .unwrap();
        }
        conn.serialize("main").unwrap().to_vec()
    };
    assert_eq!(ok(&file(100)).brushes.len(), 1);
    assert_eq!(faulted(read(&file(1100))), Fault::SutLimits);
}

#[test]
fn a_very_wide_variant_table_is_read() {
    let mut builder = SutBuilder::new().brush("Wide", 1, &[("BrushSize", real(50.0))]);
    builder.filler = 1900;
    let b = ok(&builder.build()).brushes.remove(0);
    assert_eq!(b.brush.base.radius, 25.0);
}

#[test]
fn text_from_the_file_cannot_put_control_characters_on_screen() {
    let name = "Evil\nname\u{7}\u{1b}[31m".to_string() + &"x".repeat(500);
    let file = SutBuilder::new().brush(&name, 1, &[]).build();
    let b = &ok(&file).brushes[0];
    assert!(!b.name.chars().any(|c| c.is_control()));
    assert!(b.name.chars().count() <= 128);
}

// ---------------- 入り抜き・傾き・筆先の向き・手ぶれ補正 ----------------

fn one(cells: &[(&'static str, rusqlite::types::Value)]) -> ImportedBrush {
    ok(&SutBuilder::new().brush("B", 1, cells).build())
        .brushes
        .remove(0)
}

fn mapped(b: &ImportedBrush, item: SutMapped) -> bool {
    b.mapped.contains(&item)
}

#[test]
fn start_and_end_lengths_become_the_stroke_taper_in_pixels() {
    let b = one(&[
        ("BrushUseIn", int(1)),
        ("BrushInLength", real(25.0)),
        ("BrushInLengthUnit", int(0)),
        ("BrushUseOut", int(1)),
        ("BrushOutLength", real(12.5)),
        ("BrushOutLengthUnit", int(0)),
        ("BrushInOutBySpeed", int(1)),
        ("BrushInRatio", real(30.0)),
        ("BrushInOutTarget", blob(in_out_targets(&[]))),
    ]);
    assert_eq!(
        (b.brush.assist.taper_in, b.brush.assist.taper_out),
        (25.0, 12.5)
    );
    assert!(mapped(&b, SutMapped::StartEnd));
    assert!(
        has(&b, SutNote::StartEndDetail),
        "速さ・割合を読まないことを知らせる"
    );
    assert!(
        !has(&b, SutNote::StartEnd),
        "写せたものを「未対応」とは言わない"
    );
    assert!(b.brush.validate().is_ok());
}

#[test]
fn only_the_sides_that_are_on_are_mapped_and_a_zero_length_is_nothing_to_map() {
    let b = one(&[
        ("BrushUseIn", int(0)),
        ("BrushInLength", real(25.0)),
        ("BrushUseOut", int(1)),
        ("BrushOutLength", real(40.0)),
    ]);
    assert_eq!(
        (b.brush.assist.taper_in, b.brush.assist.taper_out),
        (0.0, 40.0)
    );
    let b = one(&[("BrushUseIn", int(1)), ("BrushInLength", real(0.0))]);
    assert_eq!(b.brush.assist.taper_in, 0.0);
    assert!(
        b.mapped.is_empty() && b.unrepresented.is_empty(),
        "長さ 0 は入り抜きなし"
    );
    // 巨大な長さはエンジンの上限に収める
    let b = one(&[("BrushUseIn", int(1)), ("BrushInLength", real(1e9))]);
    assert_eq!(b.brush.assist.taper_in, yolu_core::brush::MAX_STROKE_ASSIST);
    assert!(b.brush.validate().is_ok());
}

#[test]
fn a_taper_that_is_not_about_pixels_of_size_is_reported_instead_of_mapped() {
    let on = |extra: &[(&'static str, rusqlite::types::Value)]| {
        let mut cells = vec![("BrushUseIn", int(1)), ("BrushInLength", real(20.0))];
        cells.extend_from_slice(extra);
        one(&cells)
    };
    // 単位が画素でない・パーセント指定・フェード・知らない指定方法・最小値が 100 を超える・影響先の BLOB の形が違う・長さが負・長さの列が無い
    let refused = [
        on(&[("BrushInLengthUnit", int(1))]),
        on(&[("BrushInOutType", int(1))]),
        on(&[("BrushInOutType", int(2))]),
        on(&[("BrushInOutType", int(7))]),
        on(&[("BrushInOutTarget", blob(in_out_targets_on(&[(1001, 101)])))]),
        on(&[("BrushInOutTarget", blob(vec![0, 0, 0, 12, 1, 2, 3]))]),
        on(&[("BrushInOutTarget", blob(vec![0; 40]))]),
        one(&[("BrushUseIn", int(1)), ("BrushInLength", real(-5.0))]),
        one(&[("BrushUseOut", int(1))]),
    ];
    for b in &refused {
        assert!(has(b, SutNote::StartEnd), "{:?}", b.unrepresented);
        assert_eq!(b.brush.assist.taper_in, 0.0);
        assert!(!mapped(b, SutMapped::StartEnd));
    }
    // 片方だけ写せない: 入りは写り、抜き（単位が違う）は知らせる
    let b = one(&[
        ("BrushUseIn", int(1)),
        ("BrushInLength", real(20.0)),
        ("BrushUseOut", int(1)),
        ("BrushOutLength", real(20.0)),
        ("BrushOutLengthUnit", int(2)),
    ]);
    assert_eq!(
        (b.brush.assist.taper_in, b.brush.assist.taper_out),
        (20.0, 0.0)
    );
    assert!(has(&b, SutNote::StartEnd) && has(&b, SutNote::StartEndDetail));
    assert!(mapped(&b, SutMapped::StartEnd));
}

/// 長さ 20 の入り・抜きの行に、影響先の BLOB（旗を立てた項目の「番号・最小値（%）」）をつける。
fn tapered(
    targets: &[(u32, u32)],
    extra: &[(&'static str, rusqlite::types::Value)],
) -> ImportedBrush {
    let mut cells = vec![
        ("BrushUseIn", int(1)),
        ("BrushInLength", real(20.0)),
        ("BrushUseOut", int(1)),
        ("BrushOutLength", real(10.0)),
        ("BrushInOutTarget", blob(in_out_targets_on(targets))),
    ];
    cells.extend_from_slice(extra);
    one(&cells)
}

fn target_min(b: &ImportedBrush, item: yolu_core::brush::Item) -> Option<f64> {
    b.brush.assist.taper_targets.min(item)
}

#[test]
fn the_six_taper_targets_that_the_engine_has_are_mapped_with_their_minimums() {
    use yolu_core::brush::Item;
    // 番号 → 項目、最小値（%）→ 0〜1
    let cases = [
        (1001, Item::Size),
        (1021, Item::Flow),
        (1051, Item::Roundness),
        (112, Item::TextureDensity),
        (1082, Item::MixPaint),
        (1084, Item::MixDensity),
    ];
    for (id, item) in cases {
        let b = tapered(&[(id, 30)], &[]);
        assert_eq!(
            (b.brush.assist.taper_in, b.brush.assist.taper_out),
            (20.0, 10.0)
        );
        for other in Item::ALL {
            let want = (other == item).then_some(0.3);
            assert_eq!(target_min(&b, other), want, "{id} → {other:?}");
        }
        assert!(mapped(&b, SutMapped::StartEnd));
        assert!(has(&b, SutNote::StartEndDetail));
        assert!(b.unrepresented.iter().all(|n| !matches!(
            n,
            yolu_io::brushes::Unrepresented::ClipStudio(
                SutNote::StartEnd | SutNote::StartEndTargets(_)
            )
        )));
        assert!(b.brush.validate().is_ok());
    }
    // 複数の項目: 立てた項目だけ、それぞれの最小値
    let b = tapered(&[(1001, 0), (1021, 50), (1051, 100)], &[]);
    assert_eq!(target_min(&b, Item::Size), Some(0.0));
    assert_eq!(target_min(&b, Item::Flow), Some(0.5));
    assert_eq!(target_min(&b, Item::Roundness), Some(1.0));
    assert_eq!(target_min(&b, Item::Opacity), None);
}

#[test]
fn a_taper_whose_flags_are_all_off_still_tapers_the_size_only() {
    use yolu_core::brush::Item;
    let b = tapered(&[], &[]);
    assert!(b.brush.assist.taper_targets.is_default());
    assert_eq!(target_min(&b, Item::Size), Some(0.0));
    assert!(mapped(&b, SutMapped::StartEnd));
}

#[test]
fn taper_targets_the_engine_does_not_have_are_reported_and_the_rest_are_mapped() {
    use yolu_core::brush::Item;
    use yolu_io::brushes::Unrepresented;
    // 間隔・色相・粒子密度・デュアル側のブラシサイズ・画面に無い番号 1093 は写さずに知らせる（並びは BLOB の項目の並び）。ブラシ濃度は写る
    let b = tapered(
        &[
            (1021, 20),
            (1041, 0),
            (1104, 0),
            (1134, 0),
            (51001, 0),
            (1093, 0),
        ],
        &[],
    );
    assert_eq!(target_min(&b, Item::Flow), Some(0.2));
    assert_eq!(
        target_min(&b, Item::Size),
        None,
        "旗が 0 の大きさは掛けない"
    );
    assert_eq!(b.brush.assist.taper_in, 20.0);
    let dropped: Vec<&Vec<u32>> = b
        .unrepresented
        .iter()
        .filter_map(|n| match n {
            Unrepresented::ClipStudio(SutNote::StartEndTargets(ids)) => Some(ids),
            _ => None,
        })
        .collect();
    assert_eq!(dropped, vec![&vec![1041, 1093, 1104, 1134, 51001]]);
    assert!(has(&b, SutNote::StartEndDetail));
    // 写せる項目が 1 つも無いときは、入り抜きを写さない（知らせだけ）
    let b = tapered(&[(1041, 0), (1104, 0)], &[]);
    assert_eq!(
        (b.brush.assist.taper_in, b.brush.assist.taper_out),
        (0.0, 0.0)
    );
    assert!(b.brush.assist.taper_targets.is_default());
    assert!(!mapped(&b, SutMapped::StartEnd));
    assert!(b.unrepresented.iter().any(|n| matches!(
        n,
        Unrepresented::ClipStudio(SutNote::StartEndTargets(ids)) if ids == &vec![1041, 1104]
    )));
    // 知らせの文は、写せなかった項目の名前を日本語と英語で言う
    let note = Unrepresented::ClipStudio(SutNote::StartEndTargets(vec![1041, 51001, 9]));
    let (ja, en) = (note.to_string(), note.english());
    assert!(
        ja.contains("間隔") && ja.contains("デュアルブラシのブラシサイズ") && ja.contains("番号 9"),
        "{ja}"
    );
    assert!(
        en.contains("spacing") && en.contains("dual brush brush size") && en.contains("number 9"),
        "{en}"
    );
}

#[test]
fn percent_and_fade_tapers_are_reported_and_not_mapped_even_with_targets() {
    for kind in [1, 2] {
        let b = tapered(&[(1021, 20)], &[("BrushInOutType", int(kind))]);
        assert!(has(&b, SutNote::StartEnd), "型 {kind}");
        assert_eq!(b.brush.assist.taper_in, 0.0, "型 {kind}");
        assert!(b.brush.assist.taper_targets.is_default(), "型 {kind}");
        assert!(!mapped(&b, SutMapped::StartEnd), "型 {kind}");
    }
    // 型 0 は写る
    let b = tapered(&[(1021, 20)], &[("BrushInOutType", int(0))]);
    assert_eq!(b.brush.assist.taper_in, 20.0);
}

fn falling() -> [(f64, f64); 2] {
    [(0.0, 1.0), (1.0, 0.0)]
}

/// 傾きの影響元の曲線の点（x・y）。
fn tilt_points(b: &ImportedBrush, item: yolu_core::brush::Item) -> Vec<(f64, f64)> {
    b.brush
        .influences
        .get(item)
        .tilt
        .response
        .curve()
        .iter()
        .map(|p| (p.x, p.y))
        .collect()
}

#[test]
fn a_tilt_that_falls_from_upright_to_flat_becomes_the_straight_tilt_input() {
    use yolu_core::brush::Item;
    let fall = falling();
    let press = [(0.0, 0.0), (1.0, 1.0)];
    let b = one(&[
        (
            "BrushSizeEffector",
            blob(effector_slots(0x30, 10, Some(&press), Some(&fall))),
        ),
        (
            "OpacityEffector",
            blob(effector_full(44, 0x20, [0, 0, 0, 0], None, Some(&fall))),
        ),
        (
            "BrushFlowEffector",
            blob(effector_full(48, 0x20, [0, 30, 0, 0], None, Some(&fall))),
        ),
    ]);
    let i = &b.brush.influences;
    assert!(i.size.tilt.on && i.opacity.tilt.on && i.flow.tilt.on);
    // 直立で 1・寝かせきって 0 の直線は、直立 1 の入力をそのまま使う直線（曲線なし）
    for item in [Item::Size, Item::Opacity, Item::Flow] {
        assert!(tilt_points(&b, item).is_empty(), "{item:?}");
    }
    // 最小値はヘッダーの 5 番目の整数（実物の形の見本は 100 を持つ）
    assert_eq!(i.size.tilt.response.min(), 1.0);
    assert_eq!(i.opacity.tilt.response.min(), 0.0);
    assert_eq!(i.flow.tilt.response.min(), 0.3);
    assert!(mapped(&b, SutMapped::Tilt) && mapped(&b, SutMapped::Pressure));
    assert!(
        !b.unrepresented
            .iter()
            .any(|u| matches!(u, Unrepresented::ClipStudio(SutNote::TiltCurve(_)))),
        "曲線を写せたので注記は要らない: {:?}",
        b.unrepresented
    );
    // 傾きだけの設定の曲線を、筆圧の曲線に取り違えない（筆圧の旗が無い）
    assert!(!i.opacity.pressure.on && !i.flow.pressure.on);
    assert!(i.size.pressure.on);
    assert_eq!(i.size.pressure.response.min(), 0.1);
    assert!(
        i.size.pressure.response.curve().is_empty(),
        "筆圧の曲線は直線"
    );
}

#[test]
fn a_tilt_curve_keeps_its_shape_with_the_upright_end_at_one() {
    use yolu_core::brush::Item;
    // 直立から途中まで 1 のまま、寝かせると 0 へ落ちる曲線は、x を反転した同じ形
    let plateau = [(0.0, 1.0), (0.364, 1.0), (0.609, 0.0), (1.0, 0.0)];
    let rising = [(0.0, 0.0), (1.0, 1.0)];
    let half = [(0.0, 1.0), (1.0, 0.5)];
    let b = one(&[
        (
            "BrushSizeEffector",
            blob(effector_full(44, 0x20, [0; 4], None, Some(&plateau))),
        ),
        // 傾けるほど大きくなる・半分までしか下がらない・太さ（真円率）への傾き
        (
            "OpacityEffector",
            blob(effector_full(44, 0x20, [0; 4], None, Some(&rising))),
        ),
        (
            "BrushFlowEffector",
            blob(effector_full(44, 0x20, [0; 4], None, Some(&half))),
        ),
        (
            "BrushThicknessEffector",
            blob(effector_full(44, 0x20, [0; 4], None, Some(&falling()))),
        ),
    ]);
    let flipped = |p: &[(f64, f64)]| {
        let mut v: Vec<(f64, f64)> = p.iter().map(|&(x, y)| (1.0 - x, y)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    };
    let near = |a: &[(f64, f64)], b: &[(f64, f64)]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(p, q)| (p.0 - q.0).abs() < 1e-12 && (p.1 - q.1).abs() < 1e-12)
    };
    for (item, curve) in [
        (Item::Size, &plateau[..]),
        (Item::Opacity, &rising[..]),
        (Item::Flow, &half[..]),
    ] {
        let got = tilt_points(&b, item);
        assert!(near(&got, &flipped(curve)), "{item:?}: {got:?}");
    }
    assert!(b.brush.influences.roundness.tilt.on);
    assert!(tilt_points(&b, Item::Roundness).is_empty());
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
}

#[test]
fn a_tilt_curve_that_cannot_be_located_is_straight_and_reported() {
    let fall = falling();
    let tilt = SutNote::TiltCurve(SutTarget::Size);
    // 曲線の枠が無い（旧形式のヘッダー）・曲線が枠の長さと合わない設定は、傾きを直線で使って知らせる
    let b = one(&[("BrushSizeEffector", blob(effector(44, 0x20, 0, &[&fall])))]);
    assert!(has(&b, tilt.clone()) && b.brush.influences.size.tilt.on);
    assert!(b.brush.influences.size.tilt.response.is_identity());
    let mut wrong = effector_slots(0x20, 0, None, Some(&fall));
    wrong[36..40].copy_from_slice(&999u32.to_be_bytes());
    let b = one(&[("BrushSizeEffector", blob(wrong))]);
    assert!(has(&b, tilt) && b.brush.influences.size.tilt.on);
    assert!(b.brush.influences.size.tilt.response.curve().is_empty());
    // 速さも使う設定も、傾きの曲線は 2 つ目の枠から読み、速さの曲線は直線にして知らせる
    let b = one(&[(
        "BrushFlowEffector",
        blob(effector_full(44, 0x60, [0, 0, 20, 0], None, Some(&fall))),
    )]);
    let flow = &b.brush.influences.flow;
    assert!(flow.tilt.on && flow.speed.on);
    assert_eq!(flow.speed.response.min(), 0.2);
    assert!(has(
        &b,
        SutNote::InfluenceCurve {
            target: SutTarget::Flow,
            input: SutInput::Speed
        }
    ));
    assert!(!has(&b, SutNote::TiltCurve(SutTarget::Flow)));
}

#[test]
fn the_pressure_curve_follows_the_header_slots_not_just_the_first_curve() {
    // 筆圧の曲線が無く（枠が 0）、最初の曲線が傾きのもの: 筆圧の旗が立っていても、傾きの曲線を筆圧に使わない
    let fall = falling();
    let b = one(&[(
        "BrushSizeEffector",
        blob(effector_slots(0x30, 20, None, Some(&fall))),
    )]);
    assert!(b.brush.influences.size.pressure.on);
    assert!(
        b.brush.influences.size.pressure.response.curve().is_empty(),
        "直線のまま"
    );
    assert_eq!(b.brush.influences.size.pressure.response.min(), 0.2);
    assert!(b.brush.influences.size.tilt.on);
    // 枠を読めない（旧形式の試験用ヘッダー）ときは、従来どおり最初の曲線が筆圧
    let bend = [(0.0, 0.0), (0.5, 0.9), (1.0, 1.0)];
    let b = one(&[("BrushSizeEffector", blob(effector(44, 0x10, 0, &[&bend])))]);
    assert_eq!(b.brush.influences.size.pressure.response.curve().len(), 3);
}

#[test]
fn the_tip_angle_and_its_random_range_are_mapped() {
    let b = one(&[
        ("BrushRotation", real(30.0)),
        ("BrushRotationEffector", int(0x83)),
        ("BrushRotationRandomScale", int(40)),
    ]);
    assert_eq!(b.brush.tip.angle, 30.0);
    assert_eq!(b.brush.jitter.angle, 0.4);
    assert!(mapped(&b, SutMapped::TipAngle) && mapped(&b, SutMapped::AngleRandom));
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    // 角度は -180〜180 に巻く。0 は何もしない
    for (given, wrapped) in [
        (270.0, -90.0),
        (180.0, -180.0),
        (-190.0, 170.0),
        (720.0, 0.0),
    ] {
        let b = one(&[("BrushRotation", real(given))]);
        assert_eq!(b.brush.tip.angle, wrapped, "{given}");
    }
    let b = one(&[("BrushRotation", real(0.0))]);
    assert!(!mapped(&b, SutMapped::TipAngle));
    // ランダムの旗が無ければ、強さが入っていても使わない。強さ 0 も使わない
    let b = one(&[
        ("BrushRotationEffector", int(0x03)),
        ("BrushRotationRandomScale", int(80)),
    ]);
    assert_eq!(b.brush.jitter.angle, 0.0);
    assert!(b.mapped.is_empty() && b.unrepresented.is_empty());
    let b = one(&[
        ("BrushRotationEffector", int(0x83)),
        ("BrushRotationRandomScale", int(0)),
    ]);
    assert!(!mapped(&b, SutMapped::AngleRandom));
    // 範囲外の強さは収める。非有限の角度（実数の列の NaN は SQLite では NULL）は無い扱い
    let b = one(&[
        ("BrushRotationEffector", int(0x80)),
        ("BrushRotationRandomScale", int(900)),
    ]);
    assert_eq!(b.brush.jitter.angle, 1.0);
    assert!(b.brush.validate().is_ok());
}

#[test]
fn the_angle_sources_map_to_the_engines_direction_settings() {
    // 下位 0x03 は常に立つので読まない。0x10 ペンの向き・0x20 ペン軸の回転・0x40 線の方向は、それぞれエンジンの設定へ
    let b = one(&[("BrushRotationEffector", int(0x03))]);
    assert!(!b.brush.controls.tilt_angle && !b.brush.controls.rotation_angle);
    assert!(!b.brush.tip.follow_direction);
    assert!(b.mapped.is_empty() && b.unrepresented.is_empty());
    for (flag, which) in [(0x10, "tilt"), (0x20, "rotation"), (0x40, "line")] {
        let b = one(&[("BrushRotationEffector", int(flag | 0x03))]);
        let (tilt, rotation, line) = (
            b.brush.controls.tilt_angle,
            b.brush.controls.rotation_angle,
            b.brush.tip.follow_direction,
        );
        assert_eq!(
            (tilt, rotation, line),
            (which == "tilt", which == "rotation", which == "line"),
            "{flag:#x}"
        );
        assert!(mapped(&b, SutMapped::AngleSource), "{flag:#x}");
        assert!(
            b.unrepresented.is_empty(),
            "{flag:#x}: {:?}",
            b.unrepresented
        );
    }
    // ランダム（0x80）と同時でも、全部が写る
    let b = one(&[
        ("BrushRotationEffector", int(0xF3)),
        ("BrushRotationRandomScale", int(50)),
    ]);
    assert!(b.brush.controls.tilt_angle && b.brush.controls.rotation_angle);
    assert!(b.brush.tip.follow_direction);
    assert_eq!(b.brush.jitter.angle, 0.5);
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    // 強さの読めるランダムだけなら知らせない。強さの列が無い版は写せないので知らせる（黙って捨てない）
    let b = one(&[
        ("BrushRotationEffector", int(0x83)),
        ("BrushRotationRandomScale", int(50)),
    ]);
    assert!(!has(&b, SutNote::Direction));
    let b = one(&[("BrushRotationEffector", int(0x83))]);
    assert!(has(&b, SutNote::Direction));
    assert_eq!(b.brush.jitter.angle, 0.0);
    // 知らない旗（0x04・0x08・0x100 など）は表せないので知らせる。知っている旗は写る
    for unknown in [0x04, 0x08, 0x100] {
        let b = one(&[("BrushRotationEffector", int(unknown | 0x13))]);
        assert!(has(&b, SutNote::Direction), "{unknown:#x}");
        assert!(b.brush.controls.tilt_angle);
    }
}

#[test]
fn the_direction_settings_agree_with_the_engines_plan_of_the_dab_angle() {
    // 写した設定が、エンジンのダブの角度の計画（筆先の角度 + 線の方向 + ペンの倒れた向き + ペン軸の回転）にそのまま足されること:
    // 同じ入力の列で、取り込んだブラシが、手で同じ設定にしたブラシと同じストロークを描く
    use yolu_core::glam::DVec2;
    let b = one(&[
        ("BrushRotation", real(20.0)),
        ("BrushRotationEffector", int(0x73)),
        ("BrushThickness", int(40)),
    ]);
    let mut by_hand = yolu_core::Brush::default();
    by_hand.influences.size.pressure.on = false;
    by_hand.influences.opacity.pressure.on = false;
    by_hand.influences.flow.pressure.on = false;
    by_hand.tip.angle = 20.0;
    by_hand.tip.roundness = 0.4;
    by_hand.tip.follow_direction = true;
    by_hand.controls.tilt_angle = true;
    by_hand.controls.rotation_angle = true;
    let draw = |brush: &yolu_core::Brush| {
        let mut doc = yolu_core::Document::with_tile_size(64, 64, 32).unwrap();
        let layer = doc.add_layer("a").unwrap();
        let mut brush = brush.clone();
        brush.base.radius = 6.0;
        brush.base.hardness = 1.0;
        brush.base.spacing = 0.3;
        let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
        for (x, y) in [(10.0, 10.0), (30.0, 22.0), (50.0, 12.0)] {
            stroke
                .add_point(&mut doc, x, y, 1.0, DVec2::new(0.3, 0.2))
                .unwrap();
        }
        doc.end_stroke(stroke).unwrap();
        doc.layers()[0]
            .surface(yolu_core::Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    };
    let imported = draw(&b.brush);
    assert!(imported.iter().any(|&v| v != 0));
    assert_eq!(imported, draw(&by_hand));
}

#[test]
fn a_stabilizer_strength_becomes_the_string_length_and_post_correction_is_reported_apart() {
    // 手ブレ補正は `FlickerReduction`（画面の値そのまま）。後補正（`BrushUseRevision`・`BrushRevision`）は手ぶれ補正に写さない
    let b = one(&[("FlickerReduction", int(30))]);
    assert_eq!(b.brush.assist.stabilizer, 30.0);
    assert!(mapped(&b, SutMapped::Stabilizer));
    assert!(has(&b, SutNote::StabilizerStrength));
    assert!(!has(&b, SutNote::Stabilizer) && !has(&b, SutNote::PostCorrection));
    let b = one(&[("FlickerReduction", int(1_000_000))]);
    assert_eq!(
        b.brush.assist.stabilizer,
        yolu_core::brush::MAX_STROKE_ASSIST
    );
    // 強さ 0（と、無い・負の値）は写すものが無い。速度による手ブレ補正の旗は、強さ 0 の行にも既定で入るので知らせない
    for cells in [
        vec![
            ("FlickerReduction", int(0)),
            ("FlickerReductionBySpeed", int(1)),
        ],
        vec![("FlickerReductionBySpeed", int(1))],
        vec![("FlickerReduction", int(-5))],
    ] {
        let b = one(&cells);
        assert_eq!(b.brush.assist.stabilizer, 0.0);
        assert!(b.mapped.is_empty() && b.unrepresented.is_empty());
    }
    // 速度による手ブレ補正（旗）は、強さがあるときだけ表せないと知らせる。強さは写る
    let b = one(&[
        ("FlickerReduction", int(20)),
        ("FlickerReductionBySpeed", int(1)),
    ]);
    assert_eq!(b.brush.assist.stabilizer, 20.0);
    assert!(has(&b, SutNote::Stabilizer));
    // 後補正: 旗が立っているときだけ知らせ、強さは手ぶれ補正に入れない（強さ・速度・表示倍率・ベジェの列は読まない）
    let b = one(&[
        ("BrushUseRevision", int(1)),
        ("BrushRevision", int(6)),
        ("BrushRevisionBySpeed", int(1)),
        ("BrushRevisionByViewScale", int(1)),
        ("BrushRevisionBezier", int(1)),
    ]);
    assert_eq!(b.brush.assist.stabilizer, 0.0);
    assert!(!mapped(&b, SutMapped::Stabilizer));
    assert!(has(&b, SutNote::PostCorrection));
    assert_eq!(b.unrepresented.len(), 1, "{:?}", b.unrepresented);
    let b = one(&[
        ("BrushUseRevision", int(0)),
        ("BrushRevision", int(30)),
        ("BrushRevisionBySpeed", int(1)),
        ("BrushRevisionByViewScale", int(1)),
    ]);
    assert!(b.unrepresented.is_empty() && b.mapped.is_empty());
    // 両方あれば、手ぶれ補正は写り、後補正は知らせる
    let b = one(&[("FlickerReduction", int(5)), ("BrushUseRevision", int(1))]);
    assert_eq!(b.brush.assist.stabilizer, 5.0);
    assert!(has(&b, SutNote::PostCorrection) && has(&b, SutNote::StabilizerStrength));
}

#[test]
fn the_interval_follows_its_type_and_the_fixed_value_is_a_percent_of_the_diameter() {
    // 種類 0（固定）は `BrushInterval`（直径に対する %）。整数でも実数でも %（1 は 1%）
    for (cells, expected) in [
        (
            vec![
                ("BrushAutoIntervalType", int(0)),
                ("BrushInterval", real(10.0)),
            ],
            0.10,
        ),
        (
            vec![
                ("BrushAutoIntervalType", int(0)),
                ("BrushInterval", int(37)),
            ],
            0.37,
        ),
        (
            vec![
                ("BrushAutoIntervalType", int(0)),
                ("BrushInterval", real(1.0)),
            ],
            0.01,
        ),
        (
            vec![
                ("BrushAutoIntervalType", int(0)),
                ("BrushInterval", real(100.0)),
            ],
            1.0,
        ),
        // 種類の列が無い版は、固定として読む（今まで通り）
        (vec![("BrushInterval", real(25.0))], 0.25),
    ] {
        let b = one(&cells);
        assert_eq!(b.brush.base.spacing, expected, "{cells:?}");
        assert!(b.unrepresented.is_empty());
    }
    // 広い・普通・狭いは数値を使わず、測った 12%・8%・4%
    for (kind, expected) in [(1, 0.12), (2, 0.08), (3, 0.04)] {
        for value in [10.0, 25.0, 100.0] {
            let b = one(&[
                ("BrushAutoIntervalType", int(kind)),
                ("BrushInterval", real(value)),
            ]);
            assert_eq!(b.brush.base.spacing, expected, "{kind} {value}");
            assert!(b.unrepresented.is_empty());
        }
    }
    // 数値の列が無くても種類で決まる。固定で数値が無ければ既定のまま
    let b = one(&[("BrushAutoIntervalType", int(2))]);
    assert_eq!(b.brush.base.spacing, 0.08);
    let b = one(&[("BrushAutoIntervalType", int(0))]);
    assert_eq!(
        b.brush.base.spacing,
        yolu_core::Brush::default().base.spacing
    );
    // 知らない種類は写さず（既定の間隔のまま）、値を添えて知らせる
    for (cell, value) in [(int(4), 4.0), (int(-1), -1.0), (real(1.5), 1.5)] {
        let b = one(&[
            ("BrushAutoIntervalType", cell),
            ("BrushInterval", real(30.0)),
        ]);
        assert_eq!(
            b.brush.base.spacing,
            yolu_core::Brush::default().base.spacing,
            "{value}"
        );
        assert!(has(&b, SutNote::IntervalType(value)), "{value}");
    }
}

#[test]
fn a_horizontal_thickness_turns_a_round_tip_by_ninety_degrees() {
    // 垂直（1。既定）はエンジンの真円率の潰す軸と同じ向きで、何もしない。水平（0）は潰す軸が 90 度違うので、丸い筆先は角度に足す
    let b = one(&[
        ("BrushThickness", int(40)),
        ("BrushVerticalThicknes", int(1)),
    ]);
    assert_eq!(b.brush.tip.angle, 0.0);
    assert!(!mapped(&b, SutMapped::ThicknessDirection));
    let b = one(&[
        ("BrushThickness", int(40)),
        ("BrushVerticalThicknes", int(0)),
    ]);
    assert_eq!((b.brush.tip.angle, b.brush.tip.roundness), (90.0, 0.4));
    assert!(mapped(&b, SutMapped::ThicknessDirection));
    assert!(b.unrepresented.is_empty());
    // 列が無い版は何もしない。潰していない（厚さ 100）ときは向きの違いが形に出ないので何もしない
    let b = one(&[("BrushThickness", int(40))]);
    assert_eq!(b.brush.tip.angle, 0.0);
    let b = one(&[
        ("BrushThickness", int(100)),
        ("BrushVerticalThicknes", int(0)),
    ]);
    assert_eq!(b.brush.tip.angle, 0.0);
    assert!(b.mapped.is_empty());
    // 筆先の角度に足して、-180〜180 に巻く（ほかの向きの設定と同じ角度に足される）
    for (given, wrapped) in [(30.0, 120.0), (100.0, -170.0), (-120.0, -30.0)] {
        let b = one(&[
            ("BrushThickness", int(50)),
            ("BrushVerticalThicknes", int(0)),
            ("BrushRotation", real(given)),
        ]);
        assert_eq!(b.brush.tip.angle, wrapped, "{given}");
        assert!(mapped(&b, SutMapped::TipAngle) && mapped(&b, SutMapped::ThicknessDirection));
    }
    // 向きの影響元（線の方向）と組んでも、同じ角度に足されるだけ
    let b = one(&[
        ("BrushThickness", int(50)),
        ("BrushVerticalThicknes", int(0)),
        ("BrushRotationEffector", int(0x43)),
    ]);
    assert!(b.brush.tip.follow_direction && b.brush.tip.angle == 90.0);
    // 水平が丸い筆先で 90 度回した楕円と同じ形になること: 水平は、縦長の楕円（角度 0 の横長を 90 度回したもの）
    use yolu_core::glam::DVec2;
    let draw = |angle: f64, roundness: f64| {
        let mut doc = yolu_core::Document::with_tile_size(48, 48, 32).unwrap();
        let layer = doc.add_layer("a").unwrap();
        let mut brush = yolu_core::Brush::default();
        brush.base.radius = 10.0;
        brush.base.hardness = 1.0;
        brush.influences.size.pressure.on = false;
        brush.tip.angle = angle;
        brush.tip.roundness = roundness;
        let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
        stroke
            .add_point(&mut doc, 24.5, 24.5, 1.0, DVec2::ZERO)
            .unwrap();
        doc.end_stroke(stroke).unwrap();
        let bytes = doc.layers()[0]
            .surface(yolu_core::Channel::Color)
            .unwrap()
            .to_canvas_bytes();
        let alpha = |x: usize, y: usize| bytes[(y * 48 + x) * 4 + 3];
        // 中心から横へ 8 画素・縦へ 8 画素の覆い
        (alpha(32, 24), alpha(24, 32))
    };
    let (vertical_wide, vertical_tall) = draw(0.0, 0.4);
    assert!(vertical_wide > 0 && vertical_tall == 0, "垂直は横長");
    let (horizontal_wide, horizontal_tall) = draw(
        b_angle(&one(&[
            ("BrushThickness", int(40)),
            ("BrushVerticalThicknes", int(0)),
        ])),
        0.4,
    );
    assert!(horizontal_wide == 0 && horizontal_tall > 0, "水平は縦長");
}

fn b_angle(b: &ImportedBrush) -> f64 {
    b.brush.tip.angle
}

#[test]
fn a_horizontal_thickness_on_an_image_tip_is_reported_not_mapped() {
    let build = |direction: i64, thickness: i64| {
        SutBuilder::new()
            .material(Some("tip_a"), material_with_thumbnail(&tip_png()))
            .brush(
                "T",
                1,
                &[
                    ("BrushUsePatternImage", int(1)),
                    (
                        "BrushPatternImageArray",
                        blob(refs(&[["tip_a.png", "cat/aaaa", "tip_a"]])),
                    ),
                    ("BrushThickness", int(thickness)),
                    ("BrushVerticalThicknes", int(direction)),
                ],
            )
            .build()
    };
    let b = &ok(&build(0, 40)).brushes[0];
    assert!(b.brush.tip.image.is_some());
    assert_eq!(b.brush.tip.angle, 0.0, "画像は回さない");
    assert!(has(b, SutNote::ThicknessDirection));
    assert!(!mapped(b, SutMapped::ThicknessDirection));
    // 垂直・潰していないときは知らせない
    for (direction, thickness) in [(1, 40), (0, 100)] {
        let b = &ok(&build(direction, thickness)).brushes[0];
        assert!(
            !has(b, SutNote::ThicknessDirection),
            "{direction} {thickness}"
        );
    }
}

#[test]
fn a_horizontal_full_thickness_with_a_thickness_influence_still_turns_the_tip() {
    // 厚さ 100 でも、厚さの影響元（筆圧）が入っていれば筆圧の弱いところで潰れて楕円になるので、向きの違いが形に出る
    let influence = || ("BrushThicknessEffector", blob(effector(44, 0x10, 40, &[])));
    let b = one(&[
        ("BrushThickness", int(100)),
        ("BrushVerticalThicknes", int(0)),
        influence(),
    ]);
    assert!(b.brush.influences.roundness.pressure.on);
    assert_eq!((b.brush.tip.angle, b.brush.tip.roundness), (90.0, 1.0));
    assert!(mapped(&b, SutMapped::ThicknessDirection));
    // 垂直なら向きは足さない
    let b = one(&[
        ("BrushThickness", int(100)),
        ("BrushVerticalThicknes", int(1)),
        influence(),
    ]);
    assert_eq!(b.brush.tip.angle, 0.0);
    // 画像の筆先は回さずに知らせる
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip_png()))
        .brush(
            "T",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
                ("BrushThickness", int(100)),
                ("BrushVerticalThicknes", int(0)),
                influence(),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert!(b.brush.tip.image.is_some());
    assert_eq!(b.brush.tip.angle, 0.0, "画像は回さない");
    assert!(has(b, SutNote::ThicknessDirection));
}

#[test]
fn texture_composite_modes_map_by_name_and_the_others_are_reported() {
    use yolu_core::TextureMode as M;
    let png = png_gray(1, 1, &[128]);
    let build = |mode: Option<i64>| {
        let mut cells = vec![("TextureImage", blob(refs(&[["grain.png", "c/g", "grain"]])))];
        if let Some(mode) = mode {
            cells.push(("TextureCompositeMode", int(mode)));
        }
        SutBuilder::new()
            .material(Some("grain"), material_with_thumbnail(&png))
            .brush("T", 1, &cells)
            .build()
    };
    // 0 通常・1 乗算・2 減算・3 比較(暗)・4 輪郭・5 オーバーレイ・6 覆い焼きカラー・7 焼き込みカラー・8 ハードミックス・9 高さ
    for (value, mode) in [
        (1, Some(M::Multiply)),
        (2, Some(M::Subtract)),
        (3, Some(M::Darken)),
        (5, Some(M::Overlay)),
        (6, Some(M::ColorDodge)),
        (7, Some(M::ColorBurn)),
        (8, Some(M::HardMix)),
        (0, None),
        (4, None),
        (9, None),
        (10, None),
        (-1, None),
    ] {
        let b = &ok(&build(Some(value))).brushes[0];
        let texture = b.brush.texture.as_ref().expect("質感");
        match mode {
            Some(mode) => {
                assert_eq!(texture.mode, mode, "{value}");
                assert!(!has(b, SutNote::TextureMode), "{value}");
            }
            None => {
                assert_eq!(texture.mode, M::Multiply, "{value}: 乗算にする");
                assert!(has(b, SutNote::TextureMode), "{value}: 知らせる");
            }
        }
    }
    // 列が無い版は乗算（知らせない）
    let b = &ok(&build(None)).brushes[0];
    assert_eq!(b.brush.texture.as_ref().unwrap().mode, M::Multiply);
    assert!(!has(b, SutNote::TextureMode));
    // 質感が無ければ、合わせ方の列があっても何もしない
    let b = one(&[("TextureCompositeMode", int(4))]);
    assert!(b.brush.texture.is_none() && !has(&b, SutNote::TextureMode));
}

#[test]
fn the_ink_blend_mode_is_reported_for_every_value_but_normal() {
    // 0 通常 のほかは、ブラシ単位の合成モードがエンジンに無いので知らせる（27 消去も、消しゴムはツールの切り替えで、取り込んだ設定に入らない）
    let b = one(&[("CompositeMode", int(0))]);
    assert!(!has(&b, SutNote::BlendMode));
    for value in 1..=36 {
        let b = one(&[("CompositeMode", int(value))]);
        assert!(has(&b, SutNote::BlendMode), "{value}");
        assert!(!b.brush.base.erase, "{value}");
    }
}

#[test]
fn only_the_blend_method_of_color_mixing_equal_to_mixing_is_carried() {
    // 下地混色の方式: 0 色混ぜ はエンジンの「混ぜる」、1 にじみ・2 色重ね は同じ振る舞いと言えないので写さずに知らせる
    let mixing = |kind: Option<i64>| {
        let mut cells = vec![
            ("BrushUseWaterColor", int(1)),
            ("BrushUseWaterColor2", int(1)),
            ("BrushMixColor", int(30)),
            ("BrushMixAlpha", int(20)),
            ("BrushMixColorExtension", int(10)),
        ];
        if let Some(kind) = kind {
            cells.push(("BrushWaterColor", int(kind)));
        }
        one(&cells)
    };
    for kind in [None, Some(0)] {
        let b = mixing(kind);
        assert_eq!(b.brush.mix.mode, yolu_core::brush::MixMode::Mix, "{kind:?}");
        assert!(mapped(&b, SutMapped::ColorMixing));
        assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    }
    for kind in [1, 2] {
        let b = mixing(Some(kind));
        assert_eq!(b.brush.mix.mode, yolu_core::brush::MixMode::Off, "{kind}");
        assert!(!mapped(&b, SutMapped::ColorMixing));
        assert!(has(&b, SutNote::ColorMixingMethod(kind as f64)), "{kind}");
    }
    // 下地混色が切なら、方式の列があっても知らせない
    let b = one(&[("BrushUseWaterColor", int(0)), ("BrushWaterColor", int(1))]);
    assert!(b.unrepresented.is_empty() && b.mapped.is_empty());
}

#[test]
fn vector_flags_and_a_size_unit_are_reported_and_never_dropped_silently() {
    for column in ["BrushUseVectorEraser", "BrushUseVectorMagnet"] {
        let b = one(&[(column, int(1))]);
        assert!(has(&b, SutNote::VectorTools), "{column}");
        assert_eq!(b.unrepresented.len(), 1);
        let b = one(&[(column, int(0))]);
        assert!(b.unrepresented.is_empty(), "{column}");
    }
    // 実物の行は両方の列を持つ。片方が 0 でも、もう片方の旗を見落とさない
    for (eraser, magnet) in [(0, 1), (1, 0), (1, 1)] {
        let b = one(&[
            ("BrushUseVectorEraser", int(eraser)),
            ("BrushUseVectorMagnet", int(magnet)),
        ]);
        assert!(has(&b, SutNote::VectorTools), "{eraser} {magnet}");
        assert_eq!(b.unrepresented.len(), 1, "{eraser} {magnet}");
    }
    let b = one(&[
        ("BrushUseVectorEraser", int(0)),
        ("BrushUseVectorMagnet", int(0)),
    ]);
    assert!(b.unrepresented.is_empty());
    // 単位が 0 でなければ知らせ、値は px として読む
    let b = one(&[("BrushSize", real(40.0)), ("BrushSizeUnit", int(1))]);
    assert_eq!(b.brush.base.radius, 20.0);
    assert!(has(&b, SutNote::LengthUnit));
    let b = one(&[("BrushSize", real(40.0)), ("BrushSizeUnit", int(0))]);
    assert!(!has(&b, SutNote::LengthUnit));
}

#[test]
fn at_least_one_pixel_maps_to_the_floor_of_the_dab_diameter() {
    let b = one(&[("BrushAtLeast1Pixel", int(1))]);
    assert!(b.brush.controls.min_one_pixel);
    assert!(mapped(&b, SutMapped::MinOnePixel));
    assert!(b.unrepresented.is_empty());
    let b = one(&[("BrushAtLeast1Pixel", int(0))]);
    assert!(!b.brush.controls.min_one_pixel);
    assert!(!mapped(&b, SutMapped::MinOnePixel));
    // 列が無い版は切（今まで通り）
    let b = one(&[("BrushSize", real(10.0))]);
    assert!(!b.brush.controls.min_one_pixel);
    // 写した設定はエンジンで効く: 入り抜きの先で太さが 0 になっても、線が 1 画素で続く
    use yolu_core::glam::DVec2;
    let draw = |brush: &yolu_core::Brush| {
        let mut doc = yolu_core::Document::with_tile_size(48, 48, 32).unwrap();
        let layer = doc.add_layer("a").unwrap();
        let mut brush = brush.clone();
        brush.base.hardness = 1.0;
        brush.base.spacing = 0.0625;
        let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
        stroke
            .add_point(&mut doc, 8.5, 20.5, 0.0, DVec2::ZERO)
            .unwrap();
        stroke
            .add_point(&mut doc, 40.5, 20.5, 0.0, DVec2::ZERO)
            .unwrap();
        doc.end_stroke(stroke).unwrap();
        doc.layers()[0]
            .surface(yolu_core::Channel::Color)
            .map_or(0, |s| {
                s.to_canvas_bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] > 0)
                    .count()
            })
    };
    let pressure = |on: i64| {
        let mut b = one(&[
            ("BrushSize", real(16.0)),
            ("BrushAtLeast1Pixel", int(on)),
            (
                "BrushSizeEffector",
                blob(effector_slots(
                    0x10,
                    0,
                    Some(&[(0.0, 0.0), (1.0, 1.0)]),
                    None,
                )),
            ),
        ]);
        b.brush.influences.size.pressure.on = true;
        b.brush
    };
    assert_eq!(draw(&pressure(0)), 0, "切: 筆圧 0 は塗らない");
    assert!(draw(&pressure(1)) >= 30, "入: 筆圧 0 でも 1 画素の線が続く");
}

#[test]
fn integer_columns_are_always_percent_so_a_hardness_of_one_is_one_percent() {
    let b = one(&[
        ("BrushHardness", int(1)),
        ("Opacity", int(1)),
        ("BrushFlow", int(1)),
        ("BrushThickness", int(1)),
    ]);
    let s = &b.brush.base;
    assert_eq!((s.hardness, s.opacity, s.flow), (0.01, 0.01, 0.01));
    assert_eq!(b.brush.tip.roundness, 0.01);
    // 実数の 1 は割合の 1（従来どおり）
    let b = one(&[("BrushHardness", real(1.0)), ("Opacity", real(1.0))]);
    assert_eq!((b.brush.base.hardness, b.brush.base.opacity), (1.0, 1.0));
}

#[test]
fn what_was_mapped_is_listed_apart_from_what_was_not() {
    let fall = falling();
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip_png()))
        .brush(
            "All",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["C:\\mats\\tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
                (
                    "BrushSizeEffector",
                    blob(effector_slots(
                        0x30,
                        0,
                        Some(&[(0.0, 0.0), (1.0, 1.0)]),
                        Some(&fall),
                    )),
                ),
                ("BrushUseIn", int(1)),
                ("BrushInLength", real(10.0)),
                ("FlickerReduction", int(3)),
                ("BrushRotation", real(15.0)),
                ("BrushRotationEffector", int(0x73)),
                ("BrushAtLeast1Pixel", int(1)),
                ("BrushUseWaterColor", int(1)),
                ("BrushMixColor", int(40)),
                ("BrushUseSpray", int(1)),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(
        b.mapped,
        vec![
            SutMapped::TipImage,
            SutMapped::Pressure,
            SutMapped::Tilt,
            SutMapped::StartEnd,
            SutMapped::Stabilizer,
            SutMapped::TipAngle,
            SutMapped::AngleSource,
            SutMapped::MinOnePixel,
            SutMapped::ColorMixing,
        ],
        "SutMapped の並びで、重ならない"
    );
    assert!(has(b, SutNote::Spray), "写せなかったものは別に載る");
    // 写せたものの名前は、どの種類でも両方の言語で作れる（種類を足したらここへも足す）
    for m in [
        SutMapped::TipImage,
        SutMapped::Texture,
        SutMapped::Pressure,
        SutMapped::Tilt,
        SutMapped::StartEnd,
        SutMapped::Stabilizer,
        SutMapped::TipAngle,
        SutMapped::AngleRandom,
        SutMapped::AngleSource,
        SutMapped::ThicknessDirection,
        SutMapped::MinOnePixel,
        SutMapped::ColorMixing,
        SutMapped::AntiAliasing,
    ] {
        let (ja, en) = m.texts();
        assert!(!ja.is_empty() && en.is_ascii() && !ja.is_ascii(), "{m:?}");
    }
}

// ---------------- 色の変化 ----------------

/// 色の変化の列（先端の旗と値・サブ描画色混合率・ストロークごとの旗と値）。
fn color_cells(tip: bool, stroke: bool) -> Vec<(&'static str, rusqlite::types::Value)> {
    vec![
        ("BrushChangePatternColor", int(i64::from(tip))),
        ("BrushHueChange", int(90)),
        ("BrushSaturationChange", int(-50)),
        ("BrushValueChange", int(25)),
        ("BrushSubColor", int(40)),
        ("BrushChangeStrokeColor", int(i64::from(stroke))),
        ("BrushStrokeHueChange", int(45)),
        ("BrushStrokeSaturationChange", int(20)),
        ("BrushStrokeValueChange", int(10)),
        ("BrushStrokeSubColor", int(60)),
    ]
}

fn one_color(cells: &[(&'static str, rusqlite::types::Value)]) -> ImportedBrush {
    ok(&SutBuilder::new().brush("Color", 1, cells).build())
        .brushes
        .remove(0)
}

/// 実物の既定の影響元（色相・彩度・明度はランダム・最小値 −100、サブ描画色混合率は何も使わない）。
fn default_color_effector() -> Vec<u8> {
    effector_full(48, 0x80, [0, 100, 0, (-100i32) as u32], None, None)
}

#[test]
fn the_color_change_columns_become_the_tip_sub_mix_and_stroke_changes() {
    let b = one_color(&color_cells(true, true));
    let c = &b.brush.color;
    assert_eq!(
        c.tip,
        Some(yolu_core::TipColorChange {
            hue: 90.0,
            saturation: -0.5,
            value: 0.25
        })
    );
    assert_eq!(c.sub_mix, 0.4);
    assert_eq!(
        c.stroke,
        Some(yolu_core::StrokeColorChange {
            hue: 45.0,
            saturation: 0.2,
            value: 0.1,
            sub_mix: 0.6
        })
    );
    assert_eq!(c.target, yolu_core::ColorTarget::Main, "列が無ければメイン");
    assert!(mapped(&b, SutMapped::ColorChange));
    assert!(!has(&b, SutNote::ColorChange), "写せた物には知らせない");
    // 先端とストロークごとの色相を両方使うと、足し算の上限を確かめていないと知らせる
    assert!(has(&b, SutNote::ColorApprox(ColorNote::HueSum)));
    b.brush.validate().unwrap();
    // 旗が切の側は写さない（値があれば使っていないと知らせる）。サブ描画色混合率は先端の旗によらず写す
    let b = one_color(&color_cells(false, true));
    assert!(b.brush.color.tip.is_none() && b.brush.color.stroke.is_some());
    assert_eq!(b.brush.color.sub_mix, 0.4);
    assert!(has(&b, SutNote::ColorChange) && !has(&b, SutNote::ColorApprox(ColorNote::HueSum)));
    let b = one_color(&color_cells(true, false));
    assert!(b.brush.color.tip.is_some() && b.brush.color.stroke.is_none());
    assert!(has(&b, SutNote::ColorChange));
    let b = one_color(&color_cells(false, false));
    assert!(b.brush.color.tip.is_none() && b.brush.color.stroke.is_none());
    assert_eq!(b.brush.color.sub_mix, 0.4);
    assert!(
        mapped(&b, SutMapped::ColorChange),
        "サブ描画色混合率だけでも写せた"
    );
    // 何も入っていない行は写さず、知らせもしない
    let plain = one_color(&[("BrushChangePatternColor", int(0))]);
    assert_eq!(plain.brush.color, yolu_core::ColorDynamics::default());
    assert!(!mapped(&plain, SutMapped::ColorChange) && plain.unrepresented.is_empty());
}

#[test]
fn the_color_change_target_and_out_of_range_values_are_read() {
    for (value, target, note) in [
        (0, yolu_core::ColorTarget::MainAndSub, false),
        (1, yolu_core::ColorTarget::Main, false),
        (2, yolu_core::ColorTarget::Sub, true),
    ] {
        let mut cells = color_cells(true, false);
        cells.push(("BrushChangeColorTarget", int(value)));
        let b = one_color(&cells);
        assert_eq!(b.brush.color.target, target);
        assert_eq!(
            has(&b, SutNote::ColorApprox(ColorNote::TargetSub)),
            note,
            "{value}"
        );
    }
    // サブだけでも、先端もストロークごとも無ければ知らせない
    let b = one_color(&[("BrushChangeColorTarget", int(2))]);
    assert!(!has(&b, SutNote::ColorApprox(ColorNote::TargetSub)));
    // 範囲の外の値は範囲へ収める
    let b = one_color(&[
        ("BrushChangePatternColor", int(1)),
        ("BrushHueChange", int(500)),
        ("BrushSaturationChange", int(-300)),
        ("BrushValueChange", int(300)),
        ("BrushSubColor", int(250)),
        ("BrushChangeStrokeColor", int(1)),
        ("BrushStrokeHueChange", int(400)),
        ("BrushStrokeSaturationChange", int(-5)),
    ]);
    let c = &b.brush.color;
    assert_eq!(
        c.tip,
        Some(yolu_core::TipColorChange {
            hue: 360.0,
            saturation: -1.0,
            value: 1.0
        })
    );
    assert_eq!(c.sub_mix, 1.0);
    assert_eq!(c.stroke.unwrap().hue, 180.0);
    assert_eq!(c.stroke.unwrap().saturation, 0.0);
    b.brush.validate().unwrap();
}

#[test]
fn a_color_target_without_a_tip_or_stroke_change_is_not_carried() {
    // 先端もストロークごとも切のブラシは、変化対象が メインとサブ（0）・サブ（2）でもメインのまま。色の変化を使わない
    for value in [0, 2] {
        let b = one_color(&[
            ("BrushChangePatternColor", int(0)),
            ("BrushChangeStrokeColor", int(0)),
            ("BrushChangeColorTarget", int(value)),
        ]);
        assert_eq!(
            b.brush.color.target,
            yolu_core::ColorTarget::Main,
            "{value}"
        );
        assert_eq!(
            b.brush.color,
            yolu_core::ColorDynamics::default(),
            "{value}"
        );
        assert!(!mapped(&b, SutMapped::ColorChange), "{value}");
        assert!(
            !has(&b, SutNote::ColorApprox(ColorNote::TargetSub)),
            "{value}"
        );
    }
}

#[test]
fn the_color_change_influences_are_read_with_a_signed_minimum() {
    let mut cells = color_cells(true, false);
    cells.extend([
        ("BrushHueChangeEffector", blob(default_color_effector())),
        // 彩度は筆圧（最小値 0）、明度は筆圧（最小値 −50）、サブ描画色混合率はランダム（最小値 20）
        (
            "BrushSaturationChangeEffector",
            blob(effector_full(48, 0x10, [0, 100, 0, 0], None, None)),
        ),
        (
            "BrushValueChangeEffector",
            blob(effector_full(
                48,
                0x10,
                [(-50i32) as u32, 100, 0, 0],
                None,
                None,
            )),
        ),
        (
            "BrushSubColorEffector",
            blob(effector_full(48, 0x80, [0, 0, 0, 20], None, None)),
        ),
    ]);
    let b = one_color(&cells);
    let i = &b.brush.influences;
    assert!(i.color_hue.random.on && !i.color_hue.pressure.on);
    assert_eq!(i.color_hue.random.response.min(), -1.0);
    assert!(i.color_saturation.pressure.on && i.color_saturation.pressure.response.min() == 0.0);
    assert!(i.color_value.pressure.on);
    assert_eq!(i.color_value.pressure.response.min(), -0.5);
    assert!(i.color_sub_mix.random.on);
    assert_eq!(i.color_sub_mix.random.response.min(), 0.2);
    // 既定の（曲線の付かない）ランダムには、曲線の知らせを出さない。傾き・速度を使っていなければ近い動きの知らせも無い
    assert!(!b.unrepresented.iter().any(|n| matches!(
        n,
        Unrepresented::ClipStudio(
            SutNote::InfluenceCurve { .. } | SutNote::ColorApprox(ColorNote::MotionInfluence)
        )
    )));
    b.brush.validate().unwrap();
    // 色の項目でない影響元（大きさ）の読み方は変えない: 負の最小値は 0〜1 に丸まる（−100 は 1）
    let b = one_color(&[(
        "BrushSizeEffector",
        blob(effector_full(
            48,
            0x80,
            [0, 100, 0, (-100i32) as u32],
            None,
            None,
        )),
    )]);
    assert_eq!(b.brush.influences.size.random.response.min(), 1.0);
    // 傾きと速度を使うと近い動きと知らせる。曲線の付く BLOB なら、速さ・ランダムの曲線を読めないと知らせる
    let mut cells = color_cells(true, false);
    let mut with_curve = effector_full(48, 0x80 | 0x40, [0, 100, 0, 0], None, None);
    with_curve.extend_from_slice(&[0; 44]);
    cells.extend([
        (
            "BrushValueChangeEffector",
            blob(effector_full(48, 0x20, [0, 100, 0, 0], None, None)),
        ),
        ("BrushHueChangeEffector", blob(with_curve)),
    ]);
    let b = one_color(&cells);
    assert!(b.brush.influences.color_value.tilt.on);
    assert!(has(&b, SutNote::ColorApprox(ColorNote::MotionInfluence)));
    for input in [SutInput::Speed, SutInput::Random] {
        assert!(
            has(
                &b,
                SutNote::InfluenceCurve {
                    target: SutTarget::ColorHue,
                    input
                }
            ),
            "{input:?}"
        );
    }
    // 切の側の影響元は写さない（旗が切のとき色相・彩度・明度の影響元は残さない）
    let mut cells = color_cells(false, false);
    cells.push(("BrushHueChangeEffector", blob(default_color_effector())));
    let b = one_color(&cells);
    assert!(!b.brush.influences.color_hue.is_on());
    // 読めない BLOB は外して知らせる
    let mut cells = color_cells(true, false);
    cells.push(("BrushSaturationChangeEffector", blob(vec![1, 2, 3])));
    let b = one_color(&cells);
    assert!(has(
        &b,
        SutNote::InfluenceUnreadable(SutTarget::ColorSaturation)
    ));
    assert!(!b.brush.influences.color_saturation.is_on());
}

#[test]
fn the_color_change_notes_have_sentences_in_both_languages() {
    for n in [
        SutNote::ColorChange,
        SutNote::ColorApprox(ColorNote::TargetSub),
        SutNote::ColorApprox(ColorNote::MotionInfluence),
        SutNote::ColorApprox(ColorNote::HueSum),
        SutNote::InfluenceUnreadable(SutTarget::ColorHue),
        SutNote::InfluenceCurve {
            target: SutTarget::ColorSubMix,
            input: SutInput::Speed,
        },
        SutNote::CurveSimplified(SutTarget::ColorValue, SutInput::Tilt),
    ] {
        let note = Unrepresented::ClipStudio(n.clone());
        let (ja, en) = (note.to_string(), note.english());
        assert!(
            !ja.is_empty() && !en.is_empty() && en.is_ascii() && !ja.is_ascii(),
            "{n:?}"
        );
    }
    assert_eq!(SutMapped::ColorChange.texts(), ("色の変化", "color change"));
}

// ---------------- 散布効果 ----------------

/// 散布効果の列（旗・粒子サイズ・粒子密度と、足す列）。
fn spray_cells(
    extra: &[(&'static str, rusqlite::types::Value)],
) -> Vec<(&'static str, rusqlite::types::Value)> {
    let mut cells = vec![
        ("BrushUseSpray", int(1)),
        ("BrushSpraySize", real(14.5)),
        ("BrushSpraySizeUnit", int(0)),
        ("BrushSprayDensity", int(4)),
    ];
    cells.extend_from_slice(extra);
    cells
}

fn one_spray(cells: &[(&'static str, rusqlite::types::Value)]) -> ImportedBrush {
    ok(&SutBuilder::new().brush("Spray", 1, cells).build())
        .brushes
        .remove(0)
}

#[test]
fn the_spray_columns_become_the_spray_effect() {
    use yolu_io::brushes::SprayNote;
    let b = one_spray(&spray_cells(&[
        ("BrushSpraySizeSyncBrushSize", int(1)),
        ("BrushSprayBias", int(-100)),
        ("BrushRotationInSpray", real(154.5)),
    ]));
    assert_eq!(
        b.brush.spray,
        Some(yolu_core::Spray {
            size: 14.5,
            link_size: true,
            density: 4.0,
            bias: -1.0,
            angle: 154.5,
            direction: yolu_core::SprayDirection::None,
            random_angle: None,
        })
    );
    assert!(mapped(&b, SutMapped::Spray));
    assert!(
        !has(&b, SutNote::Spray),
        "写せた物には「写していない」を出さない"
    );
    // 密度の換算と偏向の式は近い動きと知らせる
    assert!(has(&b, SutNote::SprayApprox(SprayNote::Density)));
    assert!(has(&b, SutNote::SprayApprox(SprayNote::Bias)));
    b.brush.validate().unwrap();
    // 偏向 0 は知らせない
    let b = one_spray(&spray_cells(&[("BrushSprayBias", int(0))]));
    assert_eq!(b.brush.spray.unwrap().bias, 0.0);
    assert!(!has(&b, SutNote::SprayApprox(SprayNote::Bias)));
    // 旗が切なら写さず、知らせもしない
    let mut off = spray_cells(&[]);
    off[0] = ("BrushUseSpray", int(0));
    let b = one_spray(&off);
    assert!(b.brush.spray.is_none() && !mapped(&b, SutMapped::Spray));
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
}

#[test]
fn a_spray_without_its_values_or_with_another_unit_is_not_carried_over() {
    for cells in [
        vec![("BrushUseSpray", int(1)), ("BrushSprayDensity", int(4))],
        vec![("BrushUseSpray", int(1)), ("BrushSpraySize", real(10.0))],
        {
            let mut cells = spray_cells(&[]);
            cells[2] = ("BrushSpraySizeUnit", int(1));
            cells
        },
    ] {
        let b = one_spray(&cells);
        assert!(b.brush.spray.is_none(), "{cells:?}");
        assert!(!mapped(&b, SutMapped::Spray));
        assert!(has(&b, SutNote::Spray), "写していないと知らせる");
    }
}

#[test]
fn a_spray_density_below_one_is_not_carried_over_instead_of_being_raised_to_one() {
    for density in [real(0.0), real(0.5)] {
        let mut cells = spray_cells(&[]);
        cells[3] = ("BrushSprayDensity", density);
        let b = one_spray(&cells);
        assert!(b.brush.spray.is_none(), "{cells:?}");
        assert!(!mapped(&b, SutMapped::Spray));
        assert!(has(&b, SutNote::Spray), "写していないと知らせる");
    }
    // 1 は写す
    let mut cells = spray_cells(&[]);
    cells[3] = ("BrushSprayDensity", real(1.0));
    assert_eq!(one_spray(&cells).brush.spray.unwrap().density, 1.0);
}

#[test]
fn the_particle_direction_flags_map_random_and_report_unknown_bits() {
    use yolu_io::brushes::SprayNote;
    for (flags, random, note) in [
        (0x1, None, false),
        (0x3, None, true),
        (0x81, Some(0.4), false),
        (0x83, Some(0.4), true),
        (0x0, None, false),
    ] {
        let b = one_spray(&spray_cells(&[
            ("BrushRotationEffectorInSpray", int(flags)),
            ("BrushRotationRandomInSpray", int(40)),
        ]));
        let s = b.brush.spray.unwrap();
        assert_eq!(s.random_angle, random, "{flags:#x}");
        assert_eq!(s.direction, yolu_core::SprayDirection::None, "{flags:#x}");
        assert_eq!(
            has(&b, SutNote::SprayApprox(SprayNote::Direction)),
            note,
            "{flags:#x}"
        );
    }
    // 強さの列が無ければ一杯（1）
    let b = one_spray(&spray_cells(&[("BrushRotationEffectorInSpray", int(0x81))]));
    assert_eq!(b.brush.spray.unwrap().random_angle, Some(1.0));
}

#[test]
fn the_particle_direction_flags_pick_the_direction_to_add() {
    use yolu_core::SprayDirection::{Center, Line, None as Plain, Whole};
    use yolu_io::brushes::SprayNote;
    for (flags, direction, random, note) in [
        (0x41, Line, None, false),
        (0x101, Whole, None, false),
        (0x201, Center, None, false),
        (0xC1, Line, Some(0.4), false),
        // 向きのビットが 2 つ・知らないビットは、向きを足さずに知らせる
        (0x141, Plain, None, true),
        (0x43, Line, None, true),
    ] {
        let b = one_spray(&spray_cells(&[
            ("BrushRotationEffectorInSpray", int(flags)),
            ("BrushRotationRandomInSpray", int(40)),
        ]));
        let s = b.brush.spray.unwrap();
        assert_eq!(s.direction, direction, "{flags:#x}");
        assert_eq!(s.random_angle, random, "{flags:#x}");
        assert_eq!(
            has(&b, SutNote::SprayApprox(SprayNote::Direction)),
            note,
            "{flags:#x}"
        );
    }
}

#[test]
fn the_particle_size_and_density_influences_are_carried() {
    let b = one_spray(&spray_cells(&[
        (
            "BrushSpraySizeEffector",
            blob(effector_full(44, 0x80, [0, 0, 0, 30], None, None)),
        ),
        (
            "BrushSprayDensityEffector",
            blob(effector_full(
                44,
                0x10,
                [20, 0, 0, 0],
                Some(&[(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]),
                None,
            )),
        ),
    ]));
    let i = &b.brush.influences;
    assert!(i.spray_size.random.on && !i.spray_size.pressure.on);
    assert!((i.spray_size.random.response.min() - 0.3).abs() < 1e-12);
    assert!(i.spray_density.pressure.on);
    assert!((i.spray_density.pressure.response.min() - 0.2).abs() < 1e-12);
    assert_eq!(i.spray_density.pressure.response.curve().len(), 3);
    assert!(mapped(&b, SutMapped::Pressure) && mapped(&b, SutMapped::Random));
    b.brush.validate().unwrap();
    // 散布を写さないブラシには、影響元も写さない
    let mut off = spray_cells(&[(
        "BrushSpraySizeEffector",
        blob(effector_full(44, 0x80, [0, 0, 0, 30], None, None)),
    )]);
    off[0] = ("BrushUseSpray", int(0));
    let b = one_spray(&off);
    assert!(!b.brush.influences.spray_size.is_on());
}

#[test]
fn the_spray_notes_have_sentences_in_both_languages() {
    use yolu_io::brushes::SprayNote;
    for n in [
        SutNote::Spray,
        SutNote::SprayApprox(SprayNote::Density),
        SutNote::SprayApprox(SprayNote::Bias),
        SutNote::SprayApprox(SprayNote::Direction),
        SutNote::InfluenceUnreadable(SutTarget::SpraySize),
        SutNote::TiltCurve(SutTarget::SprayDensity),
    ] {
        let note = Unrepresented::ClipStudio(n.clone());
        let (ja, en) = (note.to_string(), note.english());
        assert!(
            !ja.is_empty() && !en.is_empty() && en.is_ascii() && !ja.is_ascii(),
            "{n:?}"
        );
    }
    assert_eq!(SutMapped::Spray.texts(), ("散布効果", "spraying effect"));
}

// ---------------- デュアルブラシ ----------------

/// デュアルブラシの列（旗・2 つ目のサイズと、足す列）。
fn dual_cells(
    extra: &[(&'static str, rusqlite::types::Value)],
) -> Vec<(&'static str, rusqlite::types::Value)> {
    let mut cells = vec![("UseDualBrush", int(1)), ("DualSize", real(30.0))];
    cells.extend_from_slice(extra);
    cells
}

fn one_dual(cells: &[(&'static str, rusqlite::types::Value)]) -> ImportedBrush {
    ok(&SutBuilder::new().brush("Dual", 1, cells).build())
        .brushes
        .remove(0)
}

fn dual_of(b: &ImportedBrush) -> yolu_core::DualBrush {
    b.brush.dual.clone().expect("デュアルブラシ")
}

#[test]
fn the_dual_columns_become_the_dual_brush() {
    let b = one_dual(&dual_cells(&[
        ("SyncDualBrushSize", int(1)),
        ("DualFlow", int(60)),
        ("DualHardness", int(80)),
        ("DualAutoIntervalType", int(0)),
        ("DualInterval", real(25.0)),
        ("DualThickness", int(50)),
        ("DualRotation", real(30.0)),
        ("DualBrushCompositeMode", int(6)),
        ("DualAntiAlias", int(2)),
        ("AntiAlias", int(0)),
        ("DualUsePatternImage", int(0)),
    ]));
    let d = dual_of(&b);
    assert_eq!(d.radius, 15.0);
    assert!(d.tip.is_none(), "円形");
    assert!(d.link_size);
    assert!((d.flow - 0.6).abs() < 1e-12);
    assert!((d.hardness - 0.8).abs() < 1e-12);
    assert!((d.spacing - 0.25).abs() < 1e-12);
    assert!((d.roundness - 0.5).abs() < 1e-12);
    assert_eq!(d.angle, 30.0);
    assert_eq!(d.mode, yolu_core::DualBrushMode::Screen);
    assert_eq!(d.anti_alias, Some(yolu_core::AntiAlias::Medium));
    // 散布の旧形は使わない
    assert_eq!((d.scatter, d.count), (0.0, 1));
    assert!(mapped(&b, SutMapped::DualBrush));
    assert!(
        !has(&b, SutNote::DualBrush),
        "写せた物には「写していない」を出さない"
    );
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    b.brush.validate().unwrap();
    // 主と同じアンチエイリアスは持たない（主に従う）
    let b = one_dual(&dual_cells(&[
        ("DualAntiAlias", int(2)),
        ("AntiAlias", int(2)),
    ]));
    assert_eq!(dual_of(&b).anti_alias, None);
    // 旗が切なら写さず、知らせもしない
    let mut off = dual_cells(&[]);
    off[0] = ("UseDualBrush", int(0));
    let b = one_dual(&off);
    assert!(b.brush.dual.is_none() && !mapped(&b, SutMapped::DualBrush));
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
}

#[test]
fn a_dual_brush_without_its_size_is_not_carried_over() {
    let b = one_dual(&[("UseDualBrush", int(1)), ("DualFlow", int(50))]);
    assert!(b.brush.dual.is_none());
    assert!(!mapped(&b, SutMapped::DualBrush));
    assert!(has(&b, SutNote::DualBrush), "写していないと知らせる");
}

#[test]
fn the_thirteen_dual_modes_map_by_their_values() {
    use yolu_core::DualBrushMode as M;
    use yolu_io::brushes::SutDualNote;
    let want = [
        M::Normal,
        M::Multiply,
        M::Add,
        M::Subtract,
        M::Darken,
        M::Lighten,
        M::Screen,
        M::Overlay,
        M::ColorDodge,
        M::ColorBurn,
        M::LinearBurn,
        M::HardMix,
        M::LinearHeight,
    ];
    for (value, mode) in want.into_iter().enumerate() {
        let b = one_dual(&dual_cells(&[(
            "DualBrushCompositeMode",
            int(value as i64),
        )]));
        assert_eq!(dual_of(&b).mode, mode, "{value}");
        assert_eq!(
            has(&b, SutNote::DualApprox(SutDualNote::LinearHeight)),
            mode == M::LinearHeight,
            "{value}"
        );
    }
    // 列が無ければ乗算（知らせない）。知らない値は乗算にして知らせる
    let b = one_dual(&dual_cells(&[]));
    assert_eq!(dual_of(&b).mode, M::Multiply);
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    let b = one_dual(&dual_cells(&[("DualBrushCompositeMode", int(13))]));
    assert_eq!(dual_of(&b).mode, M::Multiply);
    assert!(has(&b, SutNote::DualApprox(SutDualNote::Mode(13.0))));
}

#[test]
fn the_dual_interval_type_uses_the_same_table_as_the_main_brush() {
    use yolu_io::brushes::SutDualNote;
    for (kind, spacing) in [(1, 0.12), (2, 0.08), (3, 0.04)] {
        let b = one_dual(&dual_cells(&[
            ("DualAutoIntervalType", int(kind)),
            ("DualInterval", real(100.0)),
        ]));
        assert!((dual_of(&b).spacing - spacing).abs() < 1e-12, "{kind}");
        // 主の同じ種類と同じ値
        let main = one(&[("BrushAutoIntervalType", int(kind))]);
        assert_eq!(main.brush.base.spacing, dual_of(&b).spacing, "{kind}");
    }
    let b = one_dual(&dual_cells(&[("DualAutoIntervalType", int(9))]));
    assert_eq!(dual_of(&b).spacing, yolu_core::DualBrush::default().spacing);
    assert!(has(&b, SutNote::DualApprox(SutDualNote::IntervalType(9.0))));
}

#[test]
fn the_dual_angle_flags_follow_the_main_brushs_bits() {
    use yolu_io::brushes::SutDualNote;
    for (flags, tilt, rotation, line, random, note) in [
        (0x03, false, false, false, 0.0, false),
        (0x13, true, false, false, 0.0, false),
        (0x23, false, true, false, 0.0, false),
        (0x43, false, false, true, 0.0, false),
        (0x83, false, false, false, 0.4, false),
        (0xF3, true, true, true, 0.4, false),
        (0x403, false, false, false, 0.0, true),
    ] {
        let b = one_dual(&dual_cells(&[
            ("DualRotationEffector", int(flags)),
            ("DualRotationRandomScale", int(40)),
        ]));
        let d = dual_of(&b);
        assert_eq!(
            (d.angle_tilt, d.angle_rotation, d.angle_line),
            (tilt, rotation, line),
            "{flags:#x}"
        );
        assert!((d.angle_random - random).abs() < 1e-12, "{flags:#x}");
        assert_eq!(
            has(&b, SutNote::DualApprox(SutDualNote::Direction)),
            note,
            "{flags:#x}"
        );
    }
    // 強さの読めないランダムは足さずに知らせる
    let b = one_dual(&dual_cells(&[("DualRotationEffector", int(0x83))]));
    assert_eq!(dual_of(&b).angle_random, 0.0);
    assert!(has(&b, SutNote::DualApprox(SutDualNote::Direction)));
}

#[test]
fn a_horizontal_dual_thickness_turns_a_round_secondary_by_ninety_degrees() {
    let b = one_dual(&dual_cells(&[
        ("DualThickness", int(50)),
        ("DualVerticalThicknes", int(0)),
        ("DualRotation", real(10.0)),
    ]));
    assert_eq!(dual_of(&b).angle, 100.0);
    // 垂直・潰していない 2 つ目は変えない
    for cells in [
        dual_cells(&[("DualThickness", int(50)), ("DualVerticalThicknes", int(1))]),
        dual_cells(&[
            ("DualThickness", int(100)),
            ("DualVerticalThicknes", int(0)),
        ]),
    ] {
        assert_eq!(dual_of(&one_dual(&cells)).angle, 0.0);
    }
}

#[test]
fn the_four_dual_influences_are_carried() {
    let b = one_dual(&dual_cells(&[
        (
            "DualSizeEffector",
            blob(effector_full(44, 0x10, [20, 0, 0, 0], None, None)),
        ),
        (
            "DualFlowEffector",
            blob(effector_full(44, 0x30, [0, 10, 0, 0], None, None)),
        ),
        (
            "DualIntervalEffector",
            blob(effector_full(44, 0x80, [0, 0, 0, 30], None, None)),
        ),
        (
            "DualThicknessEffector",
            blob(effector_full(44, 0x10, [50, 0, 0, 0], None, None)),
        ),
    ]));
    let i = &b.brush.influences;
    assert!(i.dual_size.pressure.on && (i.dual_size.pressure.response.min() - 0.2).abs() < 1e-12);
    assert!(i.dual_flow.pressure.on && i.dual_flow.tilt.on);
    assert!(i.dual_spacing.random.on && (i.dual_spacing.random.response.min() - 0.3).abs() < 1e-12);
    assert!(i.dual_thickness.pressure.on);
    assert!(mapped(&b, SutMapped::Pressure) && mapped(&b, SutMapped::Random));
    // 主の項目には写さない
    assert!(!i.size.is_on() && !i.flow.is_on() && !i.roundness.is_on());
    b.brush.validate().unwrap();
    // デュアルを写さないブラシには、影響元も写さない
    let mut off = dual_cells(&[(
        "DualSizeEffector",
        blob(effector_full(44, 0x10, [20, 0, 0, 0], None, None)),
    )]);
    off[0] = ("UseDualBrush", int(0));
    assert!(!one_dual(&off).brush.influences.dual_size.is_on());
}

#[test]
fn the_secondary_tips_are_the_referenced_materials() {
    use yolu_io::brushes::SutDualNote;
    let build = |names: &[[&str; 3]]| {
        let cells = dual_cells(&[
            ("DualUsePatternImage", int(1)),
            ("DualPatternImageArray", blob(refs(names))),
        ]);
        let file = SutBuilder::new()
            .material(Some("dual_a"), material_with_thumbnail(&tip_png()))
            .material(
                Some("dual_b"),
                material_with_thumbnail(&png_gray(1, 1, &[0])),
            )
            .brush("Dual", 1, &cells)
            .build();
        ok(&file).brushes.remove(0)
    };
    let b = build(&[["x/dual_a.png", "c/a", "dual_a"]]);
    let tip = dual_of(&b).tip.expect("2 つ目の筆先");
    assert_eq!((tip.width(), tip.height()), (2, 2));
    assert!(mapped(&b, SutMapped::DualBrush));
    assert!(!has(&b, SutNote::DualApprox(SutDualNote::Tips)));
    // 複数の筆先は全部を並びに（1 つ目は筆先にも置く）
    let b = build(&[
        ["x/dual_b.png", "c/b", "dual_b"],
        ["x/dual_a.png", "c/a", "dual_a"],
    ]);
    let tip = dual_of(&b).tip.expect("2 つ目の筆先");
    assert_eq!((tip.width(), tip.height()), (1, 1));
    let sizes: Vec<_> = dual_of(&b)
        .tips
        .iter()
        .map(|t| (t.width(), t.height()))
        .collect();
    assert_eq!(sizes, [(1, 1), (2, 2)]);
    assert!(!has(&b, SutNote::DualApprox(SutDualNote::Tips)));
    // 素材を決められなければ、デュアルブラシごと写さずに知らせる
    let b = build(&[["x/unknown.png", "c/u", "unknown"]]);
    assert!(b.brush.dual.is_none());
    assert!(has(&b, SutNote::DualBrush));
}

#[test]
fn dual_settings_the_engine_does_not_have_are_reported() {
    use yolu_io::brushes::SutDualNote;
    for (column, value, n) in [
        ("ChangeRGBByDual", int(1), SutDualNote::Rgb),
        ("DualContinuousPlot", int(1), SutDualNote::Continuous),
        ("DualRibbon", int(1), SutDualNote::Ribbon),
        ("DualUseSpray", int(1), SutDualNote::Spray),
        (
            "DualSprayUseFixedPoint",
            int(1),
            SutDualNote::SprayFixedPoint,
        ),
        (
            "DualTextureImage",
            blob(refs(&[["x/t.png", "c/t", "t"]])),
            SutDualNote::Texture,
        ),
    ] {
        let b = one_dual(&dual_cells(&[(column, value.clone())]));
        assert!(b.brush.dual.is_some(), "{column}");
        assert!(has(&b, SutNote::DualApprox(n)), "{column}");
        // 切（0）なら知らせない
        if column != "DualTextureImage" {
            let b = one_dual(&dual_cells(&[(column, int(0))]));
            assert!(
                b.unrepresented.is_empty(),
                "{column}: {:?}",
                b.unrepresented
            );
        }
    }
}

#[test]
fn the_dual_notes_have_sentences_in_both_languages() {
    use yolu_io::brushes::SutDualNote;
    for n in [
        SutNote::DualBrush,
        SutNote::DualApprox(SutDualNote::LinearHeight),
        SutNote::DualApprox(SutDualNote::Mode(13.0)),
        SutNote::DualApprox(SutDualNote::IntervalType(9.0)),
        SutNote::DualApprox(SutDualNote::SizeUnit),
        SutNote::DualApprox(SutDualNote::Direction),
        SutNote::DualApprox(SutDualNote::ThicknessDirection),
        SutNote::DualApprox(SutDualNote::Tips),
        SutNote::DualApprox(SutDualNote::TipRules),
        SutNote::DualApprox(SutDualNote::Rgb),
        SutNote::DualApprox(SutDualNote::Continuous),
        SutNote::DualApprox(SutDualNote::Ribbon),
        SutNote::DualApprox(SutDualNote::Texture),
        SutNote::DualApprox(SutDualNote::Spray),
        SutNote::DualApprox(SutDualNote::Darken),
        SutNote::DualApprox(SutDualNote::AntiAlias(4.0)),
        SutNote::InfluenceUnreadable(SutTarget::DualSize),
        SutNote::TiltCurve(SutTarget::DualSpacing),
    ] {
        let note = Unrepresented::ClipStudio(n.clone());
        let (ja, en) = (note.to_string(), note.english());
        assert!(
            !ja.is_empty() && !en.is_empty() && en.is_ascii() && !ja.is_ascii(),
            "{n:?}"
        );
        for text in [&ja, &en] {
            assert!(!text.chars().any(|c| c.is_control()), "{n:?}");
        }
    }
    assert_eq!(
        SutMapped::DualBrush.texts(),
        ("デュアルブラシ", "dual brush")
    );
}

#[test]
fn the_dual_darken_between_tips_is_reported_when_on() {
    use yolu_io::brushes::SutDualNote;
    // `DualBlendPatternByDarken` は写す（`the_secondary_darken_flag_is_carried_without_a_note`）
    #[allow(clippy::single_element_loop)]
    for column in ["DualBlendPatternByDarken2"] {
        let b = one_dual(&dual_cells(&[(column, int(1))]));
        assert!(b.brush.dual.is_some(), "{column}");
        assert!(mapped(&b, SutMapped::DualBrush), "{column}");
        assert!(
            has(&b, SutNote::DualApprox(SutDualNote::Darken)),
            "{column}"
        );
        // 主の比較暗は写していない（2 つ目の列は主の旗を立てない）
        assert!(!b.brush.tip.darken_overlap, "{column}");
        assert!(!mapped(&b, SutMapped::TipDarken), "{column}");
        // 切（0）なら知らせない
        let b = one_dual(&dual_cells(&[(column, int(0))]));
        assert!(
            b.unrepresented.is_empty(),
            "{column}: {:?}",
            b.unrepresented
        );
    }
}

#[test]
fn an_unknown_dual_anti_aliasing_value_is_reported_and_follows_the_main_brush() {
    use yolu_io::brushes::SutDualNote;
    for value in [real(4.0), real(1.5), real(-1.0)] {
        let b = one_dual(&dual_cells(&[("DualAntiAlias", value.clone())]));
        assert_eq!(dual_of(&b).anti_alias, None, "{value:?}");
        let rusqlite::types::Value::Real(v) = value else {
            unreachable!()
        };
        assert!(
            has(&b, SutNote::DualApprox(SutDualNote::AntiAlias(v))),
            "{v}"
        );
    }
    // 整数の 4 も同じ
    let b = one_dual(&dual_cells(&[("DualAntiAlias", int(4))]));
    assert_eq!(dual_of(&b).anti_alias, None);
    assert!(has(&b, SutNote::DualApprox(SutDualNote::AntiAlias(4.0))));
    // 列が無いとき、0〜3 のときは知らせない
    let b = one_dual(&dual_cells(&[]));
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    for level in 0..=3 {
        let b = one_dual(&dual_cells(&[("DualAntiAlias", int(level))]));
        assert!(b.unrepresented.is_empty(), "{level}: {:?}", b.unrepresented);
    }
}

// ---------------- デュアルブラシの 2 つ目の複数の筆先・繰り返し方法・反転・比較暗・紙質・散布効果 ----------------

/// 2 枚の 2 つ目の筆先（主の `tipped` と同じ 2 つの素材）を持つデュアルブラシに、列を足した 1 本。
fn dual_tipped(columns: &[(&'static str, i64)]) -> ImportedBrush {
    let a = png_gray(2, 1, &[0, 0]);
    let b = png_gray(1, 3, &[0, 0, 0]);
    let mut cells = dual_cells(&[
        ("DualUsePatternImage", int(1)),
        (
            "DualPatternImageArray",
            blob(refs(&[
                ["x/tip_a.png", "c/a", "tip_a"],
                ["x/tip_b.png", "c/b", "tip_b"],
            ])),
        ),
    ]);
    cells.extend(columns.iter().map(|&(name, v)| (name, int(v))));
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&a))
        .material(Some("tip_b"), material_with_thumbnail(&b))
        .brush("Dual", 1, &cells)
        .build();
    ok(&file).brushes[0].clone()
}

#[test]
fn the_secondary_repeat_methods_and_flips_use_the_main_brushs_table() {
    use yolu_io::brushes::SutDualNote;
    // 主の同じ値と同じ選び方・同じ近い動きの知らせ
    for value in 0..=5 {
        let main = tipped(&[("BrushPatternOrderType", value)]);
        let dual = dual_tipped(&[("DualPatternOrderType", value)]);
        let d = dual_of(&dual);
        assert_eq!(d.tips.len(), 2, "{value}");
        assert_eq!(d.selection, main.brush.tip.selection, "{value}");
        assert_eq!(
            has(&dual, SutNote::DualApprox(SutDualNote::TipOrderApprox)),
            has(&main, SutNote::TipOrderApprox),
            "{value}"
        );
        assert!(!has(&dual, SutNote::DualApprox(SutDualNote::TipRules)));
        // 主の筆先には写さない
        assert_eq!(dual.brush.tip.selection, yolu_core::TipSelection::Random);
        dual.brush.validate().unwrap();
    }
    for value in 0..=3 {
        let main = tipped(&[
            ("BrushPatternReverseHorizontal", value),
            ("BrushPatternReverseVertical", 3 - value),
        ]);
        let dual = dual_tipped(&[
            ("DualPatternReverseHorizontal", value),
            ("DualPatternReverseVertical", 3 - value),
        ]);
        let (t, d) = (&main.brush.tip, dual_of(&dual));
        assert_eq!(
            (d.flip_x, d.flip_x_rule, d.flip_y, d.flip_y_rule),
            (t.flip_x, t.flip_x_rule, t.flip_y, t.flip_y_rule),
            "{value}"
        );
        assert!(!dual.brush.tip.flip_x && !dual.brush.tip.flip_y);
    }
    // 知らない値は、ランダム・反転なしにして知らせる
    for columns in [
        vec![("DualPatternOrderType", 9)],
        vec![("DualPatternReverseHorizontal", 7)],
    ] {
        let b = dual_tipped(&columns);
        let d = dual_of(&b);
        assert_eq!(d.selection, yolu_core::TipSelection::Random);
        assert!(!d.flip_x && d.flip_x_rule == yolu_core::FlipRule::Fixed);
        assert!(
            has(&b, SutNote::DualApprox(SutDualNote::TipRules)),
            "{columns:?}"
        );
    }
    // 筆先 1 枚の一巡のみは、主と同じく 1 枚の並びになる
    let single = one_dual(&dual_cells(&[("DualPatternOrderType", int(4))]));
    assert!(dual_of(&single).tips.is_empty(), "丸い 2 つ目には効かない");
    assert!(
        single.unrepresented.is_empty(),
        "{:?}",
        single.unrepresented
    );
}

#[test]
fn the_secondary_darken_flag_is_carried_without_a_note() {
    let b = one_dual(&dual_cells(&[("DualBlendPatternByDarken", int(1))]));
    assert!(dual_of(&b).darken_overlap);
    assert!(mapped(&b, SutMapped::DualBrush));
    assert!(!b.brush.tip.darken_overlap, "主の比較暗には写さない");
    assert!(!mapped(&b, SutMapped::TipDarken));
    assert!(b.unrepresented.is_empty(), "{:?}", b.unrepresented);
    let off = one_dual(&dual_cells(&[("DualBlendPatternByDarken", int(0))]));
    assert!(!dual_of(&off).darken_overlap);
    assert!(off.unrepresented.is_empty(), "{:?}", off.unrepresented);
}

/// 紙質の素材 1 つ（`grain`）のファイルで、主か 2 つ目の紙質の列を入れた 1 本。
fn textured_brush(cells: &[(&'static str, rusqlite::types::Value)]) -> ImportedBrush {
    let png = png_gray(2, 2, &[255, 200, 100, 0]);
    let file = SutBuilder::new()
        .material(Some("grain"), material_with_thumbnail(&png))
        .brush("T", 1, cells)
        .build();
    ok(&file).brushes[0].clone()
}

#[test]
fn the_secondary_texture_is_read_like_the_main_texture() {
    use yolu_io::brushes::SutDualNote;
    let grain = || blob(refs(&[["g/grain.png", "c/grain", "grain"]]));
    for (mode, reverse, rotate, tone) in [(1, 0, 0, 0), (5, 1, 90, 0), (7, 0, 0, 10)] {
        let main = textured_brush(&[
            ("TextureImage", grain()),
            ("TextureDensity", int(40)),
            ("TextureScale2", int(200)),
            ("TextureReverseDensity", int(reverse)),
            ("TextureRotate", int(rotate)),
            ("TextureCompositeMode", int(mode)),
            ("TextureBrightness", int(tone)),
        ]);
        let dual = textured_brush(&dual_cells(&[
            ("DualTextureImage", grain()),
            ("DualTextureDensity", int(40)),
            ("DualTextureScale", int(200)),
            ("DualTextureReverseDensity", int(reverse)),
            ("DualTextureRotate", int(rotate)),
            ("DualTextureCompositeMode", int(mode)),
            ("DualTextureBrightness", int(tone)),
        ]));
        let t = main.brush.texture.as_ref().unwrap();
        let d = dual_of(&dual).texture.expect("2 つ目の紙質");
        assert_eq!(d.image.alpha(), t.image.alpha(), "{mode}");
        assert_eq!(
            (d.depth, d.scale, d.mode, d.angle),
            (t.depth, t.scale, t.mode, t.angle),
            "{mode}"
        );
        assert!(dual.brush.texture.is_none(), "主の紙質には写さない");
        assert_eq!(
            has(&dual, SutNote::DualApprox(SutDualNote::TextureTone)),
            has(&main, SutNote::TextureToneApprox),
            "{mode}"
        );
        assert!(!has(&dual, SutNote::DualApprox(SutDualNote::Texture)));
        dual.brush.validate().unwrap();
    }
    // 主でも写せない物は知らせる: 同じ物の無い適用方法（輪郭）・濃度強調・描点ごとに適用・紙質濃度の影響元
    for (column, value, note) in [
        ("DualTextureCompositeMode", int(4), SutDualNote::TextureMode),
        (
            "DualTextureStressDensity",
            int(1),
            SutDualNote::TextureEmphasis,
        ),
        ("DualTextureForPlot", int(1), SutDualNote::TextureEachTip),
        (
            "DualTextureDensityEffector",
            blob(effector_full(44, 0x10, [20, 0, 0, 0], None, None)),
            SutDualNote::Sources,
        ),
    ] {
        let b = textured_brush(&dual_cells(&[
            ("DualTextureImage", grain()),
            (column, value),
        ]));
        assert!(dual_of(&b).texture.is_some(), "{column}");
        assert!(has(&b, SutNote::DualApprox(note)), "{column}");
    }
    let outline = textured_brush(&dual_cells(&[
        ("DualTextureImage", grain()),
        ("DualTextureCompositeMode", int(4)),
    ]));
    assert_eq!(
        dual_of(&outline).texture.unwrap().mode,
        yolu_core::TextureMode::Multiply
    );
    // 素材を決められなければ 2 つ目の紙質なしで知らせる（デュアルブラシは写す）
    let missing = textured_brush(&dual_cells(&[(
        "DualTextureImage",
        blob(refs(&[["x/other.png", "c/o", "other"]])),
    )]));
    assert!(dual_of(&missing).texture.is_none());
    assert!(has(&missing, SutNote::DualApprox(SutDualNote::Texture)));
    assert!(mapped(&missing, SutMapped::DualBrush));
}

#[test]
fn the_secondary_spray_is_read_like_the_main_spray() {
    use yolu_io::brushes::{SprayNote, SutDualNote};
    let main = one_spray(&spray_cells(&[
        ("BrushSpraySizeSyncBrushSize", int(1)),
        ("BrushSprayBias", int(-100)),
        ("BrushRotationInSpray", real(154.5)),
        ("BrushRotationEffectorInSpray", int(0xC1)),
        ("BrushRotationRandomInSpray", int(50)),
    ]));
    let dual = one_dual(&dual_cells(&[
        ("DualUseSpray", int(1)),
        ("DualSpraySize", real(14.5)),
        ("DualSpraySizeUnit", int(0)),
        ("DualSprayDensity", int(4)),
        ("DualSpraySizeSyncBrushSize", int(1)),
        ("DualSprayBias", int(-100)),
        ("DualRotationInSpray", real(154.5)),
        ("DualRotationEffectorInSpray", int(0xC1)),
        ("DualRotationRandomInSpray", int(50)),
    ]));
    let s = dual_of(&dual).spray.expect("2 つ目の散布効果");
    assert_eq!(Some(s), main.brush.spray);
    assert_eq!(s.direction, yolu_core::SprayDirection::Line);
    assert!(dual.brush.spray.is_none(), "主の散布効果には写さない");
    for n in [SprayNote::Density, SprayNote::Bias] {
        assert!(has(&main, SutNote::SprayApprox(n)));
        assert!(has(&dual, SutNote::DualApprox(SutDualNote::SprayApprox(n))));
    }
    assert!(!has(&dual, SutNote::DualApprox(SutDualNote::Spray)));
    assert!(!mapped(&dual, SutMapped::Spray));
    dual.brush.validate().unwrap();
    let base = || {
        dual_cells(&[
            ("DualUseSpray", int(1)),
            ("DualSpraySize", real(10.0)),
            ("DualSprayDensity", int(5)),
        ])
    };
    // 粒子サイズ・粒子密度の影響元は 2 つ目には無いので知らせる
    let mut cells = base();
    cells.push((
        "DualSprayDensityEffector",
        blob(effector_full(44, 0x80, [0, 0, 0, 30], None, None)),
    ));
    let b = one_dual(&cells);
    assert!(dual_of(&b).spray.is_some());
    assert!(has(&b, SutNote::DualApprox(SutDualNote::Sources)));
    // 写せない（粒子密度が 1 未満）・固定点は、今まで通り知らせる
    let mut thin = base();
    thin[4] = ("DualSprayDensity", real(0.5));
    let b = one_dual(&thin);
    assert!(dual_of(&b).spray.is_none());
    assert!(has(&b, SutNote::DualApprox(SutDualNote::Spray)));
    let mut fixed = base();
    fixed.push(("DualSprayUseFixedPoint", int(1)));
    let b = one_dual(&fixed);
    assert!(dual_of(&b).spray.is_some());
    assert!(has(&b, SutNote::DualApprox(SutDualNote::SprayFixedPoint)));
}

#[test]
fn the_secondary_sprays_fixed_point_is_reported_apart_from_the_spray_itself() {
    use yolu_io::brushes::SutDualNote;
    let mut cells = dual_cells(&[
        ("DualUseSpray", int(1)),
        ("DualSpraySize", real(10.0)),
        ("DualSprayDensity", int(5)),
    ]);
    cells.push(("DualSprayUseFixedPoint", int(1)));
    let b = one_dual(&cells);
    // 散布は写り、知らせは固定点の 1 つだけ
    assert!(dual_of(&b).spray.is_some());
    assert!(has(&b, SutNote::DualApprox(SutDualNote::SprayFixedPoint)));
    assert!(!has(&b, SutNote::DualApprox(SutDualNote::Spray)));
    // 近い動きの知らせ（粒子密度の換算）は散布を写したときの知らせで、固定点の知らせと別
    let rest: Vec<_> = b
        .unrepresented
        .iter()
        .filter(|n| {
            !matches!(
                n,
                Unrepresented::ClipStudio(SutNote::DualApprox(
                    SutDualNote::SprayApprox(_) | SutDualNote::SprayFixedPoint
                ))
            )
        })
        .collect();
    assert!(rest.is_empty(), "{rest:?}");
}

#[test]
fn the_new_dual_notes_have_sentences_in_both_languages() {
    use yolu_io::brushes::{SprayNote, SutDualNote};
    for n in [
        SutDualNote::TipOrderApprox,
        SutDualNote::TextureGuessed,
        SutDualNote::TextureMode,
        SutDualNote::TextureTone,
        SutDualNote::TextureEachTip,
        SutDualNote::TextureEmphasis,
        SutDualNote::SprayApprox(SprayNote::Density),
        SutDualNote::SprayApprox(SprayNote::Bias),
        SutDualNote::SprayApprox(SprayNote::Direction),
        SutDualNote::Sources,
        SutDualNote::SprayFixedPoint,
    ] {
        let note = Unrepresented::ClipStudio(SutNote::DualApprox(n));
        let (ja, en) = (note.to_string(), note.english());
        assert!(
            !ja.is_empty() && !en.is_empty() && en.is_ascii() && !ja.is_ascii(),
            "{n:?}"
        );
        for text in [&ja, &en] {
            assert!(!text.chars().any(|c| c.is_control()), "{n:?}");
        }
    }
}

#[test]
fn a_single_image_secondary_moved_into_its_list_keeps_its_horizontal_thickness_note() {
    use yolu_io::brushes::SutDualNote;
    // 一巡のみの 1 枚は筆先から並びへ移るが、画像の 2 つ目なので、水平の厚さで角度を回さず知らせる
    let cells = dual_cells(&[
        ("DualUsePatternImage", int(1)),
        (
            "DualPatternImageArray",
            blob(refs(&[["x/dual_a.png", "c/a", "dual_a"]])),
        ),
        ("DualPatternOrderType", int(4)),
        ("DualThickness", int(50)),
        ("DualVerticalThicknes", int(0)),
        ("DualRotation", real(10.0)),
    ]);
    let file = SutBuilder::new()
        .material(Some("dual_a"), material_with_thumbnail(&tip_png()))
        .brush("Dual", 1, &cells)
        .build();
    let b = ok(&file).brushes.remove(0);
    let d = dual_of(&b);
    assert!(d.tip.is_none() && d.tips.len() == 1);
    assert_eq!(d.selection, yolu_core::TipSelection::Once);
    assert_eq!(d.angle, 10.0);
    assert!(has(
        &b,
        SutNote::DualApprox(SutDualNote::ThicknessDirection)
    ));
}
