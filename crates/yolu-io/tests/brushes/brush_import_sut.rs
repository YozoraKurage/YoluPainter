//! CLIP STUDIO の `.sut`（SQLite）の取り込み: 設定の写し・筆先と質感の画像・影響元・表せなかった項目・表の欠け・上限・
//! 悪意のあるスキーマ。試験のファイルは本物の .sut ではなく、公開の解析で分かっている形を試験の中で組む（`brush_files::sut`）。
//! 本物の .sut での確かめは別（docs/BRUSH_IMPORT.md の「確かめた範囲」）。

use crate::brush_files;

use std::time::{Duration, Instant};

use brush_files::sut::*;
use yolu_io::brushes::{
    import, import_bytes, BrushImportError, Fault, FileKind, ImportedBrush, ImportedSet,
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
        !s.pressure_size && !s.pressure_opacity && !s.pressure_flow,
        "影響元が無ければ筆圧は使わない"
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
fn texture_settings_the_engine_cannot_apply_are_reported_only_when_set() {
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
                    ("TextureCompositeMode", int(on * 2)),
                    ("TextureForPlot", int(on)),
                ],
            )
            .build()
    };
    let on = &ok(&build(1)).brushes[0];
    for n in [
        SutNote::TextureRotation,
        SutNote::TextureBrightness,
        SutNote::TextureContrast,
        SutNote::TextureMode,
        SutNote::TextureEachTip,
    ] {
        assert!(has(on, n.clone()), "{n:?}");
    }
    let off = &ok(&build(0)).brushes[0];
    assert!(off.brush.texture.is_some());
    assert_eq!(off.unrepresented, vec![note(SutNote::PreviewImage)]);
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
        assert!(b.brush.base.pressure_size && !b.brush.base.pressure_opacity);
        let r = &b.brush.pressure.size;
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
        b.brush.base.pressure_opacity && b.brush.base.pressure_flow && !b.brush.base.pressure_size
    );
    assert_eq!(b.brush.pressure.opacity.min(), 0.1);
    assert!(
        b.brush.pressure.opacity.curve().is_empty(),
        "直線は曲線なし"
    );
    assert_eq!(b.brush.pressure.flow.curve().len(), 3);
    assert!(b.brush.pressure.size.is_identity());
}

#[test]
fn inputs_other_than_pressure_are_reported_and_left_off() {
    let file = SutBuilder::new()
        .brush(
            "Inputs",
            1,
            &[
                (
                    "BrushSizeEffector",
                    blob(effector(44, 0x10 | 0x40 | 0x80, 0, &[])),
                ),
                (
                    "OpacityEffector",
                    blob(effector(44, 0x20, 0, &[&pressure_curve()])),
                ),
                (
                    "BrushFlowEffector",
                    blob(effector(44, 0, 0, &[&pressure_curve()])),
                ),
                ("BrushThicknessEffector", blob(effector(44, 0x10, 0, &[]))),
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert!(b.brush.base.pressure_size, "筆圧は使う");
    assert!(!b.brush.base.pressure_opacity && !b.brush.base.pressure_flow);
    for n in [
        SutNote::Influence {
            target: SutTarget::Size,
            input: SutInput::Speed,
        },
        SutNote::Influence {
            target: SutTarget::Size,
            input: SutInput::Random,
        },
        SutNote::Influence {
            target: SutTarget::Opacity,
            input: SutInput::Tilt,
        },
        SutNote::ThicknessPressure,
    ] {
        assert!(has(b, n.clone()), "{n:?}");
    }
    assert_eq!(
        b.unrepresented.len(),
        4,
        "旗の無い流量・ほかの組は注記しない: {:?}",
        b.unrepresented
    );
    assert_eq!(b.brush.tip.roundness, 1.0);
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
    assert!(has(b, SutNote::CurveSimplified(SutTarget::Size)));
    assert_eq!(b.brush.pressure.size.curve().len(), 16);
    assert!(has(b, SutNote::InfluenceUnreadable(SutTarget::Opacity)));
    assert!(
        has(b, SutNote::InfluenceUnreadable(SutTarget::Flow)),
        "大きすぎる値も黙って捨てない"
    );
    assert!(!b.brush.base.pressure_opacity && !b.brush.base.pressure_flow);
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
                // 向きに筆圧を使う（旗 0x10。下の 2 ビットは読まない）
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
    for n in [
        SutNote::Direction,
        SutNote::Spray,
        SutNote::DualBrush,
        SutNote::StartEnd,
        SutNote::Stabilizer,
        SutNote::ColorChange,
        SutNote::BlendMode,
    ] {
        assert!(has(b, n.clone()), "{n:?}");
    }
    assert!(
        b.unrepresented.contains(&Unrepresented::WetEdges),
        "ウェットエッジは ABR と同じ注記"
    );
    assert_eq!(b.unrepresented.len(), 8);
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
        SutNote::Influence {
            target: SutTarget::Size,
            input: SutInput::Tilt,
        },
        SutNote::Influence {
            target: SutTarget::Thickness,
            input: SutInput::Speed,
        },
        SutNote::Influence {
            target: SutTarget::Opacity,
            input: SutInput::Random,
        },
        SutNote::InfluenceUnreadable(SutTarget::Flow),
        SutNote::ThicknessPressure,
        SutNote::CurveSimplified(SutTarget::Opacity),
        SutNote::TiltCurve(SutTarget::Flow),
        SutNote::StartEndDetail,
        SutNote::StabilizerStrength,
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
            ],
        )
        .build();
    let b = &ok(&file).brushes[0];
    assert_eq!(b.unrepresented, vec![]);
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
    // 単位が画素でない・影響先の種類が既定でない・影響先の旗が立っている・影響先の BLOB の形が違う・長さが負・長さの列が無い
    let refused = [
        on(&[("BrushInLengthUnit", int(1))]),
        on(&[("BrushInOutType", int(1))]),
        on(&[("BrushInOutTarget", blob(in_out_targets(&[1001])))]),
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

fn falling() -> [(f64, f64); 2] {
    [(0.0, 1.0), (1.0, 0.0)]
}

#[test]
fn a_tilt_that_shrinks_size_opacity_or_flow_becomes_the_tilt_controls() {
    let fall = falling();
    let press = [(0.0, 0.0), (1.0, 1.0)];
    let b = one(&[
        (
            "BrushSizeEffector",
            blob(effector_slots(0x30, 10, Some(&press), Some(&fall))),
        ),
        (
            "OpacityEffector",
            blob(effector_slots(0x20, 0, None, Some(&fall))),
        ),
        (
            "BrushFlowEffector",
            blob(effector_slots(0x20, 0, None, Some(&fall))),
        ),
    ]);
    let c = &b.brush.controls;
    assert!(c.tilt_size && c.tilt_opacity && c.tilt_flow);
    assert!(mapped(&b, SutMapped::Tilt) && mapped(&b, SutMapped::Pressure));
    assert!(
        !b.unrepresented.iter().any(|u| matches!(
            u,
            Unrepresented::ClipStudio(SutNote::TiltCurve(_) | SutNote::Influence { .. })
        )),
        "直線は近似の注記も要らない: {:?}",
        b.unrepresented
    );
    // 傾きだけの設定の曲線を、筆圧の曲線に取り違えない（筆圧の旗が無い）
    assert!(!b.brush.base.pressure_opacity && !b.brush.base.pressure_flow);
    assert!(b.brush.base.pressure_size);
    assert_eq!(b.brush.pressure.size.min(), 0.1);
    assert!(b.brush.pressure.size.curve().is_empty(), "筆圧の曲線は直線");
}

#[test]
fn a_tilt_curve_that_is_not_a_straight_line_is_mapped_and_reported_as_approximate() {
    let plateau = [(0.0, 1.0), (0.364, 1.0), (0.609, 0.0), (1.0, 0.0)];
    let b = one(&[(
        "BrushSizeEffector",
        blob(effector_slots(0x20, 0, None, Some(&plateau))),
    )]);
    assert!(b.brush.controls.tilt_size);
    assert!(mapped(&b, SutMapped::Tilt));
    assert!(has(&b, SutNote::TiltCurve(SutTarget::Size)));
    assert!(!has(
        &b,
        SutNote::Influence {
            target: SutTarget::Size,
            input: SutInput::Tilt
        }
    ));
}

#[test]
fn a_tilt_that_cannot_be_represented_stays_a_note_and_leaves_the_controls_off() {
    let rising = [(0.0, 0.0), (1.0, 1.0)];
    let half = [(0.0, 1.0), (1.0, 0.5)];
    let fall = falling();
    let tilt = |target: SutTarget| SutNote::Influence {
        target,
        input: SutInput::Tilt,
    };
    let b = one(&[
        // 傾くほど大きくなる・半分までしか下がらない・太さ（真円率）には傾きの影響が無い
        (
            "BrushSizeEffector",
            blob(effector_slots(0x20, 0, None, Some(&rising))),
        ),
        (
            "OpacityEffector",
            blob(effector_slots(0x20, 0, None, Some(&half))),
        ),
        (
            "BrushThicknessEffector",
            blob(effector_slots(0x20, 0, None, Some(&fall))),
        ),
        // 速さも使う設定は、2 つ目の曲線がどちらのものか決められない
        (
            "BrushFlowEffector",
            blob(effector_slots(0x60, 0, None, Some(&fall))),
        ),
    ]);
    let c = &b.brush.controls;
    assert!(!c.tilt_size && !c.tilt_opacity && !c.tilt_flow);
    for target in [
        SutTarget::Size,
        SutTarget::Opacity,
        SutTarget::Thickness,
        SutTarget::Flow,
    ] {
        assert!(has(&b, tilt(target)), "{target:?}");
    }
    assert!(has(
        &b,
        SutNote::Influence {
            target: SutTarget::Flow,
            input: SutInput::Speed
        }
    ));
    assert!(!mapped(&b, SutMapped::Tilt));
    // 曲線の枠が無い（旧形式のヘッダー）・曲線が枠の長さと合わない設定も、傾きは写さない
    let b = one(&[("BrushSizeEffector", blob(effector(44, 0x20, 0, &[&fall])))]);
    assert!(has(&b, tilt(SutTarget::Size)) && !b.brush.controls.tilt_size);
    let mut wrong = effector_slots(0x20, 0, None, Some(&fall));
    wrong[36..40].copy_from_slice(&999u32.to_be_bytes());
    let b = one(&[("BrushSizeEffector", blob(wrong))]);
    assert!(has(&b, tilt(SutTarget::Size)) && !b.brush.controls.tilt_size);
}

#[test]
fn the_pressure_curve_follows_the_header_slots_not_just_the_first_curve() {
    // 筆圧の曲線が無く（枠が 0）、最初の曲線が傾きのもの: 筆圧の旗が立っていても、傾きの曲線を筆圧に使わない
    let fall = falling();
    let b = one(&[(
        "BrushSizeEffector",
        blob(effector_slots(0x30, 20, None, Some(&fall))),
    )]);
    assert!(b.brush.base.pressure_size);
    assert!(b.brush.pressure.size.curve().is_empty(), "直線のまま");
    assert_eq!(b.brush.pressure.size.min(), 0.2);
    assert!(b.brush.controls.tilt_size);
    // 枠を読めない（旧形式の試験用ヘッダー）ときは、従来どおり最初の曲線が筆圧
    let bend = [(0.0, 0.0), (0.5, 0.9), (1.0, 1.0)];
    let b = one(&[("BrushSizeEffector", blob(effector(44, 0x10, 0, &[&bend])))]);
    assert_eq!(b.brush.pressure.size.curve().len(), 3);
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
fn a_direction_driven_by_pressure_tilt_or_speed_is_reported() {
    for flag in [0x10, 0x20, 0x40] {
        let b = one(&[("BrushRotationEffector", int(flag | 3))]);
        assert!(has(&b, SutNote::Direction), "{flag:#x}");
    }
    // 旗の下の 2 ビット・強さの読めるランダムだけなら知らせない
    let b = one(&[
        ("BrushRotationEffector", int(0x83)),
        ("BrushRotationRandomScale", int(50)),
    ]);
    assert!(!has(&b, SutNote::Direction));
    // ランダムの旗があって強さの列が無い版は、写せないので知らせる（黙って捨てない）
    let b = one(&[("BrushRotationEffector", int(0x83))]);
    assert!(has(&b, SutNote::Direction));
    assert_eq!(b.brush.jitter.angle, 0.0);
}

#[test]
fn a_stabilizer_level_becomes_the_string_length_and_an_unreadable_level_is_reported() {
    let b = one(&[("BrushUseRevision", int(1)), ("BrushRevision", int(6))]);
    assert_eq!(b.brush.assist.stabilizer, 6.0);
    assert!(mapped(&b, SutMapped::Stabilizer));
    assert!(has(&b, SutNote::StabilizerStrength));
    assert!(!has(&b, SutNote::Stabilizer));
    let b = one(&[
        ("BrushUseRevision", int(1)),
        ("BrushRevision", int(1_000_000)),
    ]);
    assert_eq!(
        b.brush.assist.stabilizer,
        yolu_core::brush::MAX_STROKE_ASSIST
    );
    for cells in [
        vec![("BrushUseRevision", int(1))],
        vec![("BrushUseRevision", int(1)), ("BrushRevision", int(0))],
    ] {
        let b = one(&cells);
        assert_eq!(b.brush.assist.stabilizer, 0.0);
        assert!(has(&b, SutNote::Stabilizer) && !mapped(&b, SutMapped::Stabilizer));
    }
    let b = one(&[("BrushUseRevision", int(0)), ("BrushRevision", int(30))]);
    assert_eq!(b.brush.assist.stabilizer, 0.0);
    assert!(b.unrepresented.is_empty());
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
                ("BrushUseRevision", int(1)),
                ("BrushRevision", int(3)),
                ("BrushRotation", real(15.0)),
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
            SutMapped::ColorMixing,
        ],
        "SutMapped の並びで、重ならない"
    );
    assert!(has(b, SutNote::Spray), "写せなかったものは別に載る");
    // 写せたものの名前は両方の言語で作れる
    for m in &b.mapped {
        let (ja, en) = m.texts();
        assert!(!ja.is_empty() && en.is_ascii() && !ja.is_ascii(), "{m:?}");
    }
}
