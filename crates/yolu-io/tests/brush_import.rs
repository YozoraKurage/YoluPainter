//! ブラシ形式の取り込み（GBR・GIH・VBR・ABR・PAT・PNG）。C# の GimpBrushTests・PhotoshopBrushTests と BrushDynamicsTests の
//! ABR/PAT、BrushLibraryTests の拡張子の読み分け・PNG の約束に当たる試験。試験のファイルは試験の中で一から組む。

mod brush_files;

use brush_files::*;
use std::sync::Arc;
use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushTip, Document, DualBrushMode, Rgba8, TextureMode, TipSelection};
use yolu_io::brushes::{
    import, import_bytes, pretty_name, AbrKind, BrushImportError, ControlKind, DualNote, Fault,
    FileKind, ImportedBrush, ImportedSet, PatternMode, PatternNote, PatternRefusal, Setting,
    SkipReason, Source, TextureNote, Unrepresented, UnsupportedFile, VbrShape,
};

fn read(kind: FileKind, bytes: &[u8]) -> Result<ImportedSet, BrushImportError> {
    import_bytes(kind, bytes, None)
}
fn one(kind: FileKind, bytes: &[u8]) -> ImportedBrush {
    let mut set = read(kind, bytes).unwrap();
    assert_eq!(set.brushes.len(), 1);
    set.brushes.remove(0)
}
fn fault(result: Result<ImportedSet, BrushImportError>) -> Fault {
    match result {
        Err(BrushImportError::Fault(f)) => f,
        other => panic!("形式の中身の失敗のはず: {other:?}"),
    }
}
fn tip(b: &ImportedBrush) -> &Arc<BrushTip> {
    b.brush.tip.image.as_ref().expect("画像の筆先")
}

// ---------------- GIMP ----------------

#[test]
fn gbr_grayscale_is_flipped_to_the_canvas_origin() {
    // 2x2: 上の行 = (255, 0)、下の行 = (0, 128)
    let brush = one(
        FileKind::Gbr,
        &gbr_gray(2, 2, &[255, 0, 0, 128], "Pixel 筆", 50),
    );
    let t = tip(&brush);
    assert_eq!(brush.name, "Pixel 筆");
    assert_eq!(brush.source, Source::GimpGbr);
    assert_eq!(brush.source.label(), "GIMP GBR");
    assert_eq!(t.at(0, 1), 255, "ファイルの 1 行目は筆先の一番上の行");
    assert_eq!(t.at(1, 0), 128);
    assert_eq!(brush.brush.base.radius, 1.0);
    assert_eq!(brush.brush.base.spacing, 0.5);
    assert!(brush.unrepresented.is_empty());
}

#[test]
fn gbr_version1_and_colour_brushes_are_read_with_honest_notes() {
    let v1 = one(FileKind::Gbr, &gbr(1, 1, &[200], 1, "old", 25, 1));
    assert_eq!(
        v1.brush.base.spacing, 0.25,
        "版 1 に間隔の欄は無く、GIMP は 25% にする"
    );
    let rgba = one(
        FileKind::Gbr,
        &gbr(1, 1, &[10, 20, 30, 77], 4, "Test", 25, 2),
    );
    assert_eq!(tip(&rgba).at(0, 0), 77, "アルファのチャンネルが筆先になる");
    assert_eq!(rgba.unrepresented, vec![Unrepresented::ColorTipAsMask]);
    // 横長: 間隔は幅に対する % なので、直径（長い辺）に対する割合に直す
    let wide = one(FileKind::Gbr, &gbr_gray(4, 2, &[0; 8], "Test", 100));
    assert_eq!(wide.brush.base.spacing, 1.0);
    assert_eq!(wide.brush.base.radius, 2.0);
    let tall = one(FileKind::Gbr, &gbr_gray(2, 4, &[0; 8], "Test", 100));
    assert_eq!(tall.brush.base.spacing, 0.5);
    // 間隔の範囲（0.01〜4）と、名前の無い筆は代わりの名前
    assert_eq!(
        one(FileKind::Gbr, &gbr_gray(1, 1, &[0], "Test", 0))
            .brush
            .base
            .spacing,
        0.01
    );
    assert_eq!(
        one(FileKind::Gbr, &gbr_gray(1, 1, &[0], "Test", u32::MAX))
            .brush
            .base
            .spacing,
        4.0
    );
    let unnamed = import_bytes(
        FileKind::Gbr,
        &gbr_gray(1, 1, &[0], "", 25),
        Some("Fallback"),
    )
    .unwrap();
    assert_eq!(unnamed.brushes[0].name, "Fallback");
    assert_eq!(
        read(FileKind::Gbr, &gbr_gray(1, 1, &[0], "", 25))
            .unwrap()
            .brushes[0]
            .name,
        "Untitled"
    );
}

#[test]
fn gbr_keeps_trailing_data_visible() {
    let mut file = gbr_gray(1, 1, &[9], "T", 25);
    file.extend_from_slice(&[1, 2, 3]);
    assert_eq!(
        one(FileKind::Gbr, &file).unrepresented,
        vec![Unrepresented::GbrTrailingData { bytes: 3 }]
    );
}

#[test]
fn malformed_gbr_is_refused_with_a_reason() {
    let good = gbr_gray(2, 2, &[0; 4], "Test", 25);
    assert!(
        matches!(
            fault(read(FileKind::Gbr, &good[..good.len() - 1])),
            Fault::Truncated { .. } | Fault::BadCount { .. }
        ),
        "画素が 1 バイト足りない"
    );
    assert!(
        matches!(
            fault(read(FileKind::Gbr, &good[..10])),
            Fault::Truncated { offset: 8 }
        ),
        "ヘッダーの途中で切れる"
    );
    let mut bad_magic = good.clone();
    bad_magic[20] = b'X';
    assert_eq!(fault(read(FileKind::Gbr, &bad_magic)), Fault::GbrSignature);
    assert_eq!(
        fault(read(
            FileKind::Gbr,
            &gbr(2, 2, &[0; 4 * 18], 18, "T", 25, 2)
        )),
        Fault::GbrCinePaint
    );
    assert_eq!(
        fault(read(FileKind::Gbr, &gbr(2, 2, &[0; 8], 2, "T", 25, 2))),
        Fault::GbrPixelSize(2)
    );
    assert!(matches!(
        fault(read(FileKind::Gbr, &gbr_gray(2049, 1, &[0; 2049], "T", 25))),
        Fault::SizeOutOfRange {
            width: 2049,
            height: 1,
            ..
        }
    ));
    assert!(matches!(
        fault(read(FileKind::Gbr, &gbr_gray(0, 1, &[], "T", 25))),
        Fault::SizeOutOfRange { .. }
    ));
    assert_eq!(
        fault(read(FileKind::Gbr, &gbr(1, 1, &[0], 1, "T", 25, 9))),
        Fault::GbrVersion(9)
    );
    let long_name = "n".repeat(300);
    assert_eq!(
        fault(read(FileKind::Gbr, &gbr_gray(1, 1, &[0], &long_name, 25))),
        Fault::GbrNameTooLong
    );
    // ヘッダーの大きさが小さすぎる
    let mut small_header = good.clone();
    small_header[3] = 10;
    assert!(matches!(
        fault(read(FileKind::Gbr, &small_header)),
        Fault::GbrHeaderSize { .. }
    ));
    // 宣言した画素数がファイルに入らない
    let mut declared = gbr_gray(2, 2, &[0; 4], "T", 25);
    declared.truncate(declared.len() - 2);
    assert!(matches!(
        fault(read(FileKind::Gbr, &declared)),
        Fault::BadCount { .. }
    ));
}

fn cells() -> Vec<Vec<u8>> {
    vec![
        gbr_gray(2, 2, &[255, 255, 0, 0], "a", 25),
        gbr_gray(3, 1, &[1, 2, 3], "b", 25),
        gbr_gray(2, 2, &[0; 4], "c", 25),
    ]
}

#[test]
fn gih_becomes_one_brush_with_several_tips() {
    let header = "Sparks\n3 ncells:3 cellwidth:2 cellheight:2 step:100 dim:1 cols:1 rows:1 placement:constant rank0:3 sel0:random\n";
    let hose = one(FileKind::Gih, &gih(header, &cells()));
    assert_eq!(hose.name, "Sparks");
    assert_eq!(hose.source, Source::GimpGih);
    assert_eq!(hose.brush.tip.images.len(), 3);
    assert!(hose.brush.tip.image.is_none());
    assert_eq!(hose.brush.tip.selection, TipSelection::Random);
    assert_eq!(
        hose.brush.tip.images[1].width(),
        3,
        "セルは大きさが違ってよい"
    );
    assert!(hose.unrepresented.is_empty());

    let incremental = one(FileKind::Gih, &gih("Seq\n2\n", &cells()[..2]));
    assert_eq!(
        incremental.brush.tip.selection,
        TipSelection::Sequential,
        "パラメーターが空なら順番"
    );

    let pressure = one(
        FileKind::Gih,
        &gih(
            "Felt\n2 ncells:2 dim:3 rank0:2 sel0:pressure rank1:1 sel1:ytilt rank2:1 sel2:xtilt\n",
            &cells()[..2],
        ),
    );
    assert_eq!(
        pressure.unrepresented,
        vec![
            Unrepresented::HoseSelection {
                mode: "pressure".into()
            },
            Unrepresented::HoseDimensions { dim: "3".into() }
        ],
        "筆圧でのセルの選び方と 3 次元の並びの両方を知らせる"
    );

    let short = one(FileKind::Gih, &gih("Short\n3\n", &cells()[..2]));
    assert_eq!(
        short.brush.tip.images.len(),
        2,
        "途中で終わっただけのホースは、あるセルで使う"
    );
    assert_eq!(
        short.unrepresented,
        vec![Unrepresented::HoseShort {
            declared: 3,
            present: 2
        }]
    );

    let mut cut = gih("Cut\n2\n", &cells()[..2]);
    cut.pop();
    assert!(
        matches!(
            fault(read(FileKind::Gih, &cut)),
            Fault::Truncated { .. } | Fault::BadCount { .. }
        ),
        "セルの途中で切れたものは断る"
    );

    // 1 つのセルは単独の筆先
    let single = one(FileKind::Gih, &gih("One\n1\n", &cells()[..1]));
    assert!(single.brush.tip.image.is_some() && single.brush.tip.images.is_empty());
}

#[test]
fn malformed_gih_headers_are_refused() {
    for header in [
        "",
        "NoNewline",
        "Name\n",
        "Name\n0\n",
        "Name\n257\n",
        "Name\nabc\n",
        "Name\n-3\n",
        &format!("{}\n2\n", "n".repeat(5000)),
    ] {
        let result = read(FileKind::Gih, &gih(header, &cells()));
        assert!(
            matches!(fault(result), Fault::GihHeader | Fault::GihCellCount),
            "{header:?}"
        );
    }
}

#[test]
fn vbr_circle_maps_to_the_round_tip_and_stars_are_rendered() {
    let soft = one(
        FileKind::Vbr,
        b"GIMP-VBR\n1.0\nHardness 050\n10.000000\n25.000000\n0.500000\n1.000000\n0.000000\n",
    );
    assert!(soft.brush.tip.image.is_none());
    assert_eq!(soft.brush.base.radius, 25.0);
    assert_eq!(soft.brush.base.hardness, 0.5);
    assert_eq!(soft.brush.base.spacing, 0.1);
    assert!(soft.unrepresented.is_empty());
    assert_eq!(soft.source.label(), "GIMP VBR 1.0");

    let star = one(
        FileKind::Vbr,
        b"GIMP-VBR\r\n1.5\r\nStar\r\ndiamond\r\n50.000000\r\n25.000000\r\n5\r\n1.000000\r\n2.500000\r\n17.500000\r\n",
    );
    assert_eq!(star.brush.tip.roundness, 0.4);
    assert_eq!(star.brush.tip.angle, 17.5);
    let t = tip(&star);
    assert!((t.sample(0.5, 0.5) - 1.0).abs() < 1e-9, "中心は塗りつぶし");
    assert!(t.sample(0.99, 0.5) > 0.0, "スパイクは軸に沿って縁まで届く");
    assert_eq!(
        star.unrepresented,
        vec![Unrepresented::VbrShapeRendered {
            shape: VbrShape::Diamond,
            spikes: 5
        }]
    );

    let square = one(
        FileKind::Vbr,
        b"GIMP-VBR\n1.5\nBox\nsquare\n50\n25\n2\n1\n1\n0\n",
    );
    assert_eq!(
        square.unrepresented,
        vec![Unrepresented::VbrShapeRendered {
            shape: VbrShape::Square,
            spikes: 2
        }]
    );
    assert!(tip(&square).sample(0.97, 0.97) > 0.0, "正方形は角まで塗る");

    assert!(matches!(
        fault(read(
            FileKind::Vbr,
            b"GIMP-VBR\n1.5\nBad\nhexagon\n1\n1\n2\n1\n1\n0\n"
        )),
        Fault::VbrShape(_)
    ));
    assert!(
        matches!(
            fault(read(
                FileKind::Vbr,
                b"GIMP-VBR\n1.5\nBad\ncircle\n1\n1\n21\n1\n1\n0\n"
            )),
            Fault::VbrNumber { line: 7, .. }
        ),
        "GIMP と同じく、スパイクが 20 を超えるものは断る"
    );
    assert!(matches!(
        fault(read(FileKind::Vbr, b"GIMP-VBR\n1.0\nShort\n10")),
        Fault::VbrEndsEarly { line: 5 }
    ));
    assert!(
        matches!(
            fault(read(FileKind::Vbr, b"GIMP-VBR\n1.0\nShort\n10\n")),
            Fault::VbrNumber { line: 5, .. }
        ),
        "最後の改行の後ろの空の行は数ではない"
    );
    assert_eq!(fault(read(FileKind::Vbr, b"NOT-VBR\n1.0\n")), Fault::NotVbr);
    assert!(matches!(
        fault(read(FileKind::Vbr, b"GIMP-VBR\n2.0\nX\n")),
        Fault::VbrVersion(_)
    ));
    assert!(
        matches!(
            fault(read(
                FileKind::Vbr,
                b"GIMP-VBR\n1.0\nNaN\nnan\n1\n1\n1\n0\n"
            )),
            Fault::VbrNumber { line: 4, .. }
        ),
        "NaN は数として通さない"
    );
    assert!(matches!(
        fault(read(
            FileKind::Vbr,
            b"GIMP-VBR\n1.0\nBig\n10\n1e999\n1\n1\n0\n"
        )),
        Fault::VbrNumber { line: 5, .. }
    ));
    // BOM つきの UTF-8 と空の名前
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(b"GIMP-VBR\n1.0\n\n10\n8\n1\n1\n0\n");
    assert_eq!(
        import_bytes(FileKind::Vbr, &bom, Some("From file"))
            .unwrap()
            .brushes[0]
            .name,
        "From file"
    );
}

// ---------------- Photoshop ABR ----------------

fn assert_tip(t: &BrushTip) {
    assert_eq!((t.width(), t.height()), (3, 2));
    assert_eq!(t.at(0, 1), 255, "ファイルの一番上の行は筆先の一番上の行");
    assert_eq!(t.at(2, 0), 128);
}

#[test]
fn version1_computed_and_sampled_brushes() {
    let computed = W::new()
        .i32(0)
        .i16(30)
        .i16(40)
        .i16(50)
        .i16(-30)
        .i16(80)
        .done(); // misc・間隔・直径・真円率・角度・硬さ
    let sampled = raw_bitmap(
        W::new()
            .i32(0)
            .i16(25)
            .u8(1)
            .i16(0)
            .i16(0)
            .i16(2)
            .i16(3)
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(8),
    )
    .done();
    let file = W::new()
        .i16(1)
        .i16(2)
        .i16(1)
        .i32(computed.len() as i64)
        .bytes(&computed)
        .i16(2)
        .i32(sampled.len() as i64)
        .bytes(&sampled)
        .done();
    let set = import_bytes(FileKind::Abr, &file, Some("Old")).unwrap();
    assert_eq!(set.brushes.len(), 2);
    let c = &set.brushes[0];
    assert!(c.brush.tip.image.is_none());
    assert_eq!(
        c.source,
        Source::PhotoshopAbr {
            version: 1,
            kind: AbrKind::Computed
        }
    );
    assert_eq!(c.source.label(), "Photoshop ABR v1 computed");
    assert_eq!(
        (
            c.brush.base.radius,
            c.brush.base.spacing,
            c.brush.tip.roundness,
            c.brush.tip.angle,
            c.brush.base.hardness
        ),
        (20.0, 0.3, 0.5, -30.0, 0.8)
    );
    assert!(!c.brush.base.pressure_opacity);
    assert_eq!(c.name, "Old 1");
    assert_tip(tip(&set.brushes[1]));
    assert_eq!(set.brushes[1].brush.base.spacing, 0.25);
    assert_eq!(set.brushes[1].source.label(), "Photoshop ABR v1 sampled");
}

#[test]
fn version2_sampled_brush_with_name_and_packbits() {
    let sampled = rle_bitmap(
        W::new()
            .i32(0)
            .i16(10)
            .unicode("Grass 草")
            .u8(1)
            .i16(0)
            .i16(0)
            .i16(2)
            .i16(3)
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(8),
    )
    .done();
    let file = W::new()
        .i16(2)
        .i16(1)
        .i16(2)
        .i32(sampled.len() as i64)
        .bytes(&sampled)
        .done();
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(set.brushes.len(), 1);
    assert_eq!(set.brushes[0].name, "Grass 草");
    assert_tip(tip(&set.brushes[0]));
}

#[test]
fn version6_tips_with_preset_dynamics() {
    let leaves = preset(&[
        ("Nm  ", text("Leaves")),
        (
            "Brsh",
            obj(
                "sampledBrush",
                &[
                    ("Dmtr", unit("#Pxl", 60.0)),
                    ("Angl", unit("#Ang", 45.0)),
                    ("Rndn", unit("#Prc", 70.0)),
                    ("Spcn", unit("#Prc", 35.0)),
                    ("sampledData", text(ID)),
                ],
            ),
        ),
        ("useTipDynamics", boolean(true)),
        ("szVr", dynamics(2, 40.0)),
        ("angleDynamics", dynamics(0, 100.0)),
        ("roundnessDynamics", dynamics(0, 20.0)),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, 150.0)),
        ("bothAxes", boolean(true)),
        ("Cnt ", long(3)),
        ("usePaintDynamics", boolean(true)),
        ("opVr", dynamics(2, 10.0)),
        ("prVr", dynamics(1, 30.0)),
        ("useTexture", boolean(true)),
        ("Wtdg", boolean(true)),
    ]);
    let file = abr_v6(&[
        section("samp", &samp_record(1, ID, false)),
        section("desc", &desc_body(&[brush_list(&[leaves])])),
        section("patt", &[0; 8]),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(set.brushes.len(), 1);
    let brush = &set.brushes[0];
    let b = &brush.brush;
    assert_eq!(brush.name, "Leaves");
    assert_eq!(
        brush.source,
        Source::PhotoshopAbr {
            version: 6,
            kind: AbrKind::Preset
        }
    );
    assert_tip(tip(brush));
    assert_eq!(
        (b.base.radius, b.tip.angle, b.tip.roundness, b.base.spacing),
        (30.0, 45.0, 0.7, 0.35)
    );
    assert_eq!(b.jitter.size, 0.4);
    assert!(b.base.pressure_size, "大きさを筆圧で制御");
    assert_eq!((b.jitter.angle, b.jitter.roundness), (1.0, 0.2));
    assert_eq!((b.jitter.scatter, b.jitter.count), (1.5, 3));
    assert_eq!(b.jitter.opacity, 0.1);
    assert!(b.base.pressure_opacity);
    assert_eq!(b.jitter.flow, 0.3);
    assert!(!b.base.pressure_flow, "フェードは筆圧ではない");
    assert_eq!(
        b.controls.fade_flow, 25,
        "コントロール 1 は fStp 描点のフェード"
    );
    assert!(brush.unrepresented.iter().any(|n| matches!(
        n,
        Unrepresented::Texture(TextureNote::PatternMissing { .. })
    )));
    assert!(brush.unrepresented.contains(&Unrepresented::WetEdges));
    assert!(!brush.unrepresented.iter().any(|n| matches!(
        n,
        Unrepresented::Control {
            setting: Setting::Flow,
            ..
        }
    )));
    assert!(
        set.notes
            .iter()
            .any(|n| matches!(n, Unrepresented::PatternsUnreadable(_))),
        "模様の節が空の記録だけで壊れている（ファイル全体の注記）"
    );
    assert!(
        !brush
            .unrepresented
            .iter()
            .any(|n| matches!(n, Unrepresented::PatternsUnreadable(_))),
        "ファイル全体の注記はブラシに複製しない"
    );
}

/// 混合ブラシのウェット・混合のゆらぎ（`wetnessControl`・`mixControl`）を持つプリセットは「表せなかった項目」に載り、持たない・ゆらぎもコントロールも
/// 無いプリセットには載らない。
#[test]
fn a_mixer_brush_preset_notes_wetness_and_mix_jitter_and_others_do_not() {
    let preset_with = |wet: Option<Vec<u8>>, mix: Option<Vec<u8>>, paint_dynamics: bool| {
        let mut items = vec![
            ("Nm  ", text("Mixer")),
            (
                "Brsh",
                obj(
                    "computedBrush",
                    &[
                        ("Dmtr", unit("#Pxl", 40.0)),
                        ("Hrdn", unit("#Prc", 50.0)),
                        ("Spcn", unit("#Prc", 25.0)),
                    ],
                ),
            ),
            ("usePaintDynamics", boolean(paint_dynamics)),
            ("opVr", dynamics(2, 0.0)),
        ];
        if let Some(w) = wet {
            items.push(("wetnessControl", w));
        }
        if let Some(m) = mix {
            items.push(("mixControl", m));
        }
        let file = abr_v6(&[
            section("desc", &desc_body(&[brush_list(&[preset(&items)])])),
            section("patt", &[0; 8]),
        ]);
        let set = read(FileKind::Abr, &file).unwrap();
        assert_eq!(set.brushes.len(), 1);
        set.brushes[0].unrepresented.clone()
    };
    let noted = |n: &Vec<Unrepresented>| n.contains(&Unrepresented::MixerBrush);
    assert!(
        noted(&preset_with(Some(dynamics(0, 40.0)), None, true)),
        "ウェットのゆらぎ"
    );
    assert!(
        noted(&preset_with(None, Some(dynamics(2, 0.0)), true)),
        "混合のコントロール（筆圧）"
    );
    assert!(
        !noted(&preset_with(
            Some(dynamics(0, 0.0)),
            Some(dynamics(0, 0.0)),
            true
        )),
        "ゆらぎもコントロールも無い"
    );
    assert!(!noted(&preset_with(None, None, true)));
    // 「トランスファー」を使っていないプリセットの値は効いていないので載せない
    assert!(!noted(&preset_with(Some(dynamics(0, 40.0)), None, false)));
    assert_eq!(
        Unrepresented::MixerBrush.to_string(),
        "混合ブラシのウェット・混合のゆらぎは未対応"
    );
    assert_eq!(
        Unrepresented::MixerBrush.english(),
        "Mixer brush wetness and mix jitter are not supported."
    );
}

#[test]
fn version6_subversion2_tips_without_presets_still_import() {
    let mut samples = samp_record(2, "a", true);
    samples.extend(samp_record(2, "b", false));
    let file = W::new()
        .i16(10)
        .i16(2)
        .bytes(&section("samp", &samples))
        .done();
    let set = import_bytes(FileKind::Abr, &file, Some("Set")).unwrap();
    assert_eq!(set.brushes.len(), 2);
    assert_eq!(
        set.brushes
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["Set 1", "Set 2"]
    );
    for b in &set.brushes {
        assert_tip(tip(b));
        assert_eq!(
            b.source,
            Source::PhotoshopAbr {
                version: 10,
                kind: AbrKind::Tip
            }
        );
        assert_eq!(b.brush.base.spacing, 0.25);
        assert_eq!(b.brush.base.radius, 1.5);
    }
}

#[test]
fn an_unreadable_preset_section_falls_back_to_the_tips_with_a_note() {
    let bad_desc = W::new()
        .i32(16)
        .unicode("")
        .key("null")
        .i32(1)
        .key("Brsh")
        .ascii("ObAr")
        .done(); // 未対応の型
    let file = abr_v6(&[
        section("samp", &samp_record(1, "x", false)),
        section("desc", &bad_desc),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(set.brushes.len(), 1);
    assert_tip(tip(&set.brushes[0]));
    assert!(matches!(
        set.notes.as_slice(),
        [Unrepresented::PresetsUnreadable(Fault::DescriptorType { kind, .. })] if kind == "ObAr"
    ));
    assert!(set.brushes[0].unrepresented.is_empty());
}

#[test]
fn unknown_sections_are_reported_once() {
    let file = abr_v6(&[
        section("samp", &samp_record(1, "x", false)),
        section("zzzz", &[1, 2, 3]),
        section("zzzz", &[4]),
        section("yyyy", &[]),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(
        set.notes,
        vec![
            Unrepresented::SectionSkipped("zzzz".into()),
            Unrepresented::SectionSkipped("yyyy".into())
        ]
    );
    assert!(set.brushes[0].unrepresented.is_empty());
}

#[test]
fn malformed_abr_files_are_refused_with_a_reason() {
    assert_eq!(
        fault(read(FileKind::Abr, &W::new().i16(3).done())),
        Fault::AbrVersion(3)
    );
    assert_eq!(
        fault(read(
            FileKind::Abr,
            &W::new().i16(1).i16(1).i16(2).i32(100).done()
        )),
        Fault::AbrBrushTruncated { index: 1 }
    );
    let huge = abr_v6(&[section(
        "samp",
        &W::new()
            .i32(30)
            .u8(1)
            .ascii("z")
            .bytes(&[0; 10])
            .i32(0)
            .i32(0)
            .i32(5000)
            .i32(5000)
            .i16(8)
            .done(),
    )]);
    assert!(matches!(
        fault(read(FileKind::Abr, &huge)),
        Fault::SizeOutOfRange {
            width: 5000,
            height: 5000,
            ..
        }
    ));
    assert_eq!(
        fault(read(FileKind::Abr, &W::new().i16(6).i16(1).done())),
        Fault::AbrNoBrushes
    );
    assert_eq!(
        fault(read(FileKind::Abr, &W::new().i16(6).i16(7).done())),
        Fault::AbrSubversion(7)
    );
    assert_eq!(
        fault(read(FileKind::Abr, &W::new().i16(1).i16(-1).done())),
        Fault::AbrBrushCount(-1)
    );
    assert_eq!(
        fault(read(
            FileKind::Abr,
            &W::new().i16(1).i16(1).i16(7).i32(0).done()
        )),
        Fault::AbrBrushType { index: 1, kind: 7 }
    );
    assert!(matches!(
        fault(read(
            FileKind::Abr,
            &W::new().i16(6).i16(1).ascii("XXXXsamp").i32(0).done()
        )),
        Fault::AbrSection { offset: 4 }
    ));
    assert!(matches!(
        fault(read(FileKind::Abr, &[])),
        Fault::Truncated { .. }
    ));
    // 深さ・圧縮方式
    let bad_depth = abr_v6(&[section(
        "samp",
        &W::new()
            .i32(30)
            .u8(1)
            .ascii("z")
            .bytes(&[0; 10])
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(4)
            .done(),
    )]);
    assert_eq!(fault(read(FileKind::Abr, &bad_depth)), Fault::TipDepth(4));
    let bad_compression = abr_v6(&[section(
        "samp",
        &W::new()
            .i32(30)
            .u8(1)
            .ascii("z")
            .bytes(&[0; 10])
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(8)
            .u8(2)
            .done(),
    )]);
    assert_eq!(
        fault(read(FileKind::Abr, &bad_compression)),
        Fault::TipCompression(2)
    );
    let deep_rle = abr_v6(&[section(
        "samp",
        &W::new()
            .i32(30)
            .u8(1)
            .ascii("z")
            .bytes(&[0; 10])
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(16)
            .u8(1)
            .done(),
    )]);
    assert_eq!(
        fault(read(FileKind::Abr, &deep_rle)),
        Fault::Tip16BitCompressed
    );
}

#[test]
fn a_16_bit_tip_keeps_the_high_byte_and_says_so() {
    // 2x1 の 16 bit: 0x80 0x01 / 0xFF 0xFF
    let record = W::new()
        .u8(1)
        .ascii("z")
        .bytes(&[0; 10])
        .i32(0)
        .i32(0)
        .i32(1)
        .i32(2)
        .i16(16)
        .u8(0)
        .bytes(&[0x80, 0x01, 0xFF, 0xFF])
        .done();
    let samp = W::new()
        .i32(record.len() as i64)
        .bytes(&record)
        .pad4()
        .done();
    let set = read(FileKind::Abr, &abr_v6(&[section("samp", &samp)])).unwrap();
    let t = tip(&set.brushes[0]);
    assert_eq!((t.at(0, 0), t.at(1, 0)), (0x80, 0xFF));
    assert_eq!(set.brushes[0].unrepresented, vec![Unrepresented::Tip16Bit]);
}

#[test]
fn abr_colour_dual_texture_and_controls_are_mapped() {
    let set = read(FileKind::Abr, &dual_file(120.0)).unwrap();
    assert_eq!(
        set.brushes.len(),
        2,
        "デュアルの筆先はプリセットが使うので、それだけのブラシにならない"
    );
    let b = &set.brushes[0].brush;
    assert_eq!(b.controls.fade_size, 25);
    assert!(b.tip.follow_direction, "角度のコントロール 7 は方向");
    assert!(b.controls.tilt_opacity);
    let n = &set.brushes[0].unrepresented;
    assert!(n.contains(&Unrepresented::Control {
        setting: Setting::Roundness,
        control: ControlKind::PenTilt
    }));
    assert!(n.contains(&Unrepresented::Control {
        setting: Setting::Flow,
        control: ControlKind::StylusWheel
    }));
    assert_eq!(
        (
            b.color.foreground_background,
            b.color.hue,
            b.color.saturation,
            b.color.brightness,
            b.color.purity
        ),
        (0.4, 0.2, 0.3, 0.1, -0.5)
    );
    assert!(b.color.per_tip);
    let dual = b.dual.as_ref().expect("デュアルブラシ");
    let dual_tip = dual.tip.as_ref().unwrap();
    assert_eq!(dual_tip.width(), 3);
    assert_eq!(dual_tip.at(0, 1), 255);
    assert_eq!(
        (
            dual.radius,
            dual.angle,
            dual.spacing,
            dual.mode,
            dual.count,
            dual.scatter
        ),
        (12.0, 30.0, 0.4, DualBrushMode::ColorBurn, 3, 1.2)
    );
    // 模様: 上の行 (0, 255)、下の行 (100, 200) → 左下原点で [0,0]=100 [1,0]=200 [0,1]=0 [1,1]=255。反転して 155, 55, 255, 0
    let tex = b.texture.as_ref().expect("質感");
    assert_eq!(
        [
            tex.image.at(0, 0),
            tex.image.at(1, 0),
            tex.image.at(0, 1),
            tex.image.at(1, 1)
        ],
        [155, 55, 255, 0]
    );
    assert_eq!(
        (tex.depth, tex.scale, tex.mode),
        (0.6, 2.0, TextureMode::Multiply)
    );
    assert!(
        !n.iter().any(|x| matches!(
            x,
            Unrepresented::Texture(_) | Unrepresented::TexturePattern(_) | Unrepresented::Dual(_)
        )),
        "グレーの 8 bit の乗算の模様は正確に写る: {n:?}"
    );
    let m = &set.brushes[1];
    assert!(m.brush.texture.is_none());
    assert!(
        m.unrepresented
            .contains(&Unrepresented::Texture(TextureNote::PatternMissing {
                name: "Paper".into()
            })),
        "模様は ID だけで探す（名前では決して対応づけない）"
    );
}

#[test]
fn presets_that_use_the_same_tip_share_one_image() {
    let a = preset(&[
        ("Nm  ", text("A")),
        ("Brsh", obj("sampledBrush", &[("sampledData", text(ID))])),
    ]);
    let b = preset(&[
        ("Nm  ", text("B")),
        (
            "Brsh",
            obj(
                "sampledBrush",
                &[("sampledData", text(ID)), ("Dmtr", unit("#Pxl", 10.0))],
            ),
        ),
    ]);
    let file = abr_v6(&[
        section("samp", &samp_record(1, ID, false)),
        section("desc", &desc_body(&[brush_list(&[a, b])])),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(
        set.brushes.len(),
        2,
        "筆先を使うプリセットがあるので、その筆先だけのブラシは増えない"
    );
    assert!(
        Arc::ptr_eq(tip(&set.brushes[0]), tip(&set.brushes[1])),
        "同じ筆先の画像は共有する（保存で 1 つにまとめられる）"
    );
}

#[test]
fn texture_modes_flips_and_rotation_map_to_the_core_extensions() {
    let pat = pattern_gray("Paper", "p", 2, 2, &[10, 20, 30, 40]);
    let make = |mode: &str, scale: f64| {
        preset(&[
            ("Nm  ", text(mode)),
            (
                "Brsh",
                obj(
                    "sampledBrush",
                    &[
                        ("sampledData", text(ID)),
                        ("flipX", boolean(true)),
                        ("flipY", boolean(false)),
                    ],
                ),
            ),
            ("useTipDynamics", boolean(true)),
            ("angleDynamics", dynamics(5, 0.0)),
            ("useTexture", boolean(true)),
            (
                "Txtr",
                obj("Ptrn", &[("Nm  ", text("Paper")), ("Idnt", text("p"))]),
            ),
            ("textureScale", unit("#Prc", scale)),
            ("textureBlendMode", enum_value("BlnM", mode)),
        ])
    };
    let modes = [
        "Sbtr",
        "blendSubtraction",
        "Drkn",
        "Ovrl",
        "CDdg",
        "CBrn",
        "linearBurn",
        "hardMix",
        "Mltp",
    ];
    let expected = [
        TextureMode::Subtract,
        TextureMode::Subtract,
        TextureMode::Darken,
        TextureMode::Overlay,
        TextureMode::ColorDodge,
        TextureMode::ColorBurn,
        TextureMode::LinearBurn,
        TextureMode::HardMix,
        TextureMode::Multiply,
    ];
    let presets: Vec<Vec<u8>> = modes
        .iter()
        .map(|m| make(m, 100.0))
        .chain([make("weird", 100.0), make("Mltp", 10000.0)])
        .collect();
    let file = abr_v6(&[
        section("samp", &samp_record(1, ID, false)),
        section("desc", &desc_body(&[brush_list(&presets)])),
        section("patt", &framed(&[pat])),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(set.brushes.len(), 11);
    for (i, mode) in expected.iter().enumerate() {
        let b = &set.brushes[i];
        assert_eq!(
            b.brush.texture.as_ref().unwrap().mode,
            *mode,
            "{}",
            modes[i]
        );
        assert!(
            !b.unrepresented
                .iter()
                .any(|n| matches!(n, Unrepresented::Texture(_))),
            "{}: {:?}",
            modes[i],
            b.unrepresented
        );
        assert!(
            b.brush.tip.flip_x && !b.brush.tip.flip_y,
            "筆先の反転は core の拡張で表す"
        );
        assert!(
            b.brush.controls.rotation_angle,
            "角度のコントロール 5 はペンの軸の回転"
        );
        assert!(!b
            .unrepresented
            .iter()
            .any(|n| matches!(n, Unrepresented::Control { .. })));
    }
    let weird = &set.brushes[9];
    assert_eq!(
        weird.brush.texture.as_ref().unwrap().mode,
        TextureMode::Multiply
    );
    assert!(weird
        .unrepresented
        .contains(&Unrepresented::Texture(TextureNote::Mode("weird".into()))));
    let clamped = &set.brushes[10];
    assert_eq!(clamped.brush.texture.as_ref().unwrap().scale, 64.0);
    assert!(clamped
        .unrepresented
        .contains(&Unrepresented::Texture(TextureNote::ScaleClamped {
            percent: 10000.0,
            used_percent: 6400.0
        })));
}

#[test]
fn presets_that_cannot_be_imported_are_reported_not_dropped() {
    let no_shape = preset(&[("Nm  ", text("No shape"))]);
    let missing_tip = preset(&[
        ("Nm  ", text("Lost tip")),
        (
            "Brsh",
            obj("sampledBrush", &[("sampledData", text("not-there"))]),
        ),
    ]);
    let ok = preset(&[("Nm  ", text("Fine")), ("Brsh", obj("computedBrush", &[]))]);
    let file = abr_v6(&[
        section("samp", &samp_record(1, "orphan", false)),
        section(
            "desc",
            &desc_body(&[brush_list(&[no_shape, missing_tip, ok])]),
        ),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert_eq!(
        set.skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason))
            .collect::<Vec<_>>(),
        [
            ("No shape", SkipReason::NoTipShape),
            ("Lost tip", SkipReason::TipNotInFile)
        ]
    );
    assert_eq!(
        set.brushes
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["Fine", "Tip 1"],
        "どのプリセットも使わない筆先も、ブラシとして残る"
    );
}

#[test]
fn the_minimum_diameter_becomes_the_size_minimum_only_when_pressure_drives_the_size() {
    let with_control = |name: &str, control: i32, min: f64| {
        preset(&[
            ("Nm  ", text(name)),
            ("Brsh", obj("computedBrush", &[])),
            ("useTipDynamics", boolean(true)),
            ("minimumDiameter", unit("#Prc", min)),
            ("szVr", dynamics(control, 0.0)),
        ])
    };
    let file = abr_v6(&[section(
        "desc",
        &desc_body(&[brush_list(&[
            with_control("Pressure", 2, 25.0),
            with_control("Fade", 1, 25.0),
            with_control("Off", 0, 25.0),
            with_control("Zero", 2, 0.0),
            with_control("Full", 2, 100.0),
        ])]),
    )]);
    let set = read(FileKind::Abr, &file).unwrap();
    let by = |name: &str| set.brushes.iter().find(|b| b.name == name).unwrap();
    // 筆圧で大きさを変える: 最小の直径が応えの最小値になり、表せなかった項目に残らない
    let pressure = by("Pressure");
    assert!(pressure.brush.base.pressure_size);
    assert_eq!(pressure.brush.pressure.size.min(), 0.25);
    assert!(pressure.brush.pressure.size.curve().is_empty());
    assert!(!pressure
        .unrepresented
        .iter()
        .any(|n| matches!(n, Unrepresented::MinimumDiameter(_))));
    assert!(pressure.brush.pressure.opacity.is_identity());
    assert!(pressure.brush.pressure.flow.is_identity());
    // フェード・切の最小は表せない（今までどおり知らせる）
    for name in ["Fade", "Off"] {
        let b = by(name);
        assert!(b.brush.pressure.is_identity(), "{name}");
        assert!(
            b.unrepresented
                .contains(&Unrepresented::MinimumDiameter(25.0)),
            "{name}"
        );
    }
    // 最小 0 は何も変えない・100% は筆圧を無視して 1
    assert!(by("Zero").brush.pressure.is_identity());
    assert_eq!(by("Full").brush.pressure.size.min(), 1.0);
    assert_eq!(by("Full").brush.pressure.size.apply(0.0), 1.0);
}

#[test]
fn fade_and_dual_notes_are_reported() {
    let weird = preset(&[
        ("Nm  ", text("Odd")),
        ("Brsh", obj("mysteryBrush", &[])),
        ("useTipDynamics", boolean(true)),
        ("minimumDiameter", unit("#Prc", 25.0)),
        (
            "szVr",
            obj(
                "brVr",
                &[
                    ("bVTy", long(1)),
                    ("fStp", long(0)),
                    ("jitter", unit("#Prc", 0.0)),
                ],
            ),
        ),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, 50.0)),
        ("countDynamics", dynamics(0, 10.0)),
        ("useColorDynamics", boolean(true)),
        ("clVr", dynamics(2, 0.0)),
        ("colorDynamicsPerTip", boolean(false)),
        ("useDualBrush", boolean(true)),
        (
            "dualBrush",
            obj(
                "dualBrush",
                &[
                    ("Brsh", obj("mysteryBrush", &[])),
                    ("BlnM", enum_value("BlnM", "Dvsn")),
                    ("scatterDynamics", dynamics(0, 30.0)),
                    ("bothAxes", boolean(false)),
                    ("countDynamics", dynamics(0, 10.0)),
                    ("Flip", boolean(true)),
                ],
            ),
        ),
        ("Nose", boolean(true)),
    ]);
    let no_dual = preset(&[
        ("Nm  ", text("No dual")),
        ("Brsh", obj("computedBrush", &[])),
        ("useDualBrush", boolean(true)),
    ]);
    let lost_dual = preset(&[
        ("Nm  ", text("Lost dual")),
        ("Brsh", obj("computedBrush", &[])),
        ("useDualBrush", boolean(true)),
        (
            "dualBrush",
            obj(
                "dualBrush",
                &[(
                    "Brsh",
                    obj("sampledBrush", &[("sampledData", text("gone"))]),
                )],
            ),
        ),
    ]);
    let file = abr_v6(&[
        section(
            "desc",
            &desc_body(&[brush_list(&[weird, no_dual, lost_dual])]),
        ),
        section("samp", &samp_record(1, "t", false)),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    let n = &set.brushes[0].unrepresented;
    for expected in [
        Unrepresented::UnknownTipKind("mysteryBrush".into()),
        Unrepresented::MinimumDiameter(25.0),
        Unrepresented::FadeRange {
            setting: Setting::Size,
            steps: 0.0,
        },
        Unrepresented::ScatterOneAxis,
        Unrepresented::CountJitter,
        Unrepresented::ForegroundBackgroundControl(ControlKind::PenPressure),
        Unrepresented::Dual(DualNote::UnknownTipKind("mysteryBrush".into())),
        Unrepresented::Dual(DualNote::Mode("Dvsn".into())),
        Unrepresented::Dual(DualNote::ScatterOneAxis),
        Unrepresented::Dual(DualNote::CountJitter),
        Unrepresented::Dual(DualNote::Flip),
        Unrepresented::Noise,
    ] {
        assert!(n.contains(&expected), "{expected:?} が無い: {n:?}");
    }
    assert!(!set.brushes[0].brush.color.per_tip);
    assert_eq!(set.brushes[0].brush.controls.fade_size, 0);
    assert!(set.brushes[1]
        .unrepresented
        .contains(&Unrepresented::Dual(DualNote::MissingTip)));
    assert!(set.brushes[1].brush.dual.is_none());
    assert!(set.brushes[2]
        .unrepresented
        .contains(&Unrepresented::Dual(DualNote::TipNotInFile)));
    assert!(set.brushes[2].brush.dual.is_none());
}

#[test]
fn tool_options_and_numeric_ranges_are_clamped_not_rejected() {
    let p = preset(&[
        ("Nm  ", text("Clamp")),
        (
            "Brsh",
            obj(
                "computedBrush",
                &[
                    ("Dmtr", unit("#Pxl", 99999.0)),
                    ("Angl", unit("#Ang", -720.0)),
                    ("Rndn", unit("#Prc", -5.0)),
                    ("Spcn", unit("#Prc", 99999.0)),
                    ("Hrdn", unit("#Prc", 900.0)),
                ],
            ),
        ),
        (
            "toolOptions",
            obj(
                "toolOptions",
                &[("Opct", unit("#Prc", 150.0)), ("flow", unit("#Prc", -10.0))],
            ),
        ),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, -50.0)),
        ("Cnt ", long(1000)),
        ("bothAxes", boolean(true)),
        ("useColorDynamics", boolean(true)),
        ("purity", unit("#Prc", 900.0)),
    ]);
    let file = abr_v6(&[
        section("desc", &desc_body(&[brush_list(&[p])])),
        section("samp", &samp_record(1, "t", false)),
    ]);
    let b = &read(FileKind::Abr, &file).unwrap().brushes[0].brush;
    assert_eq!(
        (
            b.base.radius,
            b.tip.angle,
            b.tip.roundness,
            b.base.spacing,
            b.base.hardness
        ),
        (1000.0, -180.0, 0.01, 4.0, 1.0)
    );
    assert_eq!(
        (
            b.base.opacity,
            b.base.flow,
            b.jitter.scatter,
            b.jitter.count,
            b.color.purity
        ),
        (1.0, 0.0, 0.0, 16, 1.0)
    );
    assert!(b.validate().is_ok());
}

#[test]
fn a_non_finite_descriptor_number_is_an_unreadable_preset_section() {
    let p = preset(&[
        ("Nm  ", text("NaN")),
        (
            "Brsh",
            obj("computedBrush", &[("Dmtr", unit("#Pxl", f64::NAN))]),
        ),
    ]);
    let file = abr_v6(&[
        section("desc", &desc_body(&[brush_list(&[p])])),
        section("samp", &samp_record(1, "t", false)),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert!(set.notes.contains(&Unrepresented::PresetsUnreadable(
        Fault::DescriptorNotFinite
    )));
}

#[test]
fn abr_patterns_that_cannot_be_used_are_reported_per_brush() {
    let lab = pattern(
        9,
        "Lab",
        "lab-1",
        1,
        1,
        &[vec![1], vec![2], vec![3]],
        false,
        8,
        None,
    );
    let rgb = pattern(
        3,
        "Colour",
        "rgb-1",
        2,
        1,
        &[vec![255, 0], vec![0, 0], vec![0, 255]],
        true,
        8,
        None,
    );
    let a = preset(&[
        ("Nm  ", text("A")),
        ("Brsh", obj("computedBrush", &[])),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj("Ptrn", &[("Nm  ", text("Lab")), ("Idnt", text("lab-1"))]),
        ),
    ]);
    let b = preset(&[
        ("Nm  ", text("B")),
        ("Brsh", obj("computedBrush", &[])),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj("Ptrn", &[("Nm  ", text("Colour")), ("Idnt", text("rgb-1"))]),
        ),
    ]);
    let desc = desc_body(&[brush_list(&[a, b])]);
    let file = abr_v6(&[
        section("desc", &desc),
        section("patt", &framed(&[lab, rgb])),
    ]);
    let set = read(FileKind::Abr, &file).unwrap();
    assert!(set.brushes[0].brush.texture.is_none());
    assert!(set.brushes[0]
        .unrepresented
        .contains(&Unrepresented::Texture(TextureNote::PatternRefused {
            name: "Lab".into(),
            reason: PatternRefusal::Mode(PatternMode::Lab)
        })));
    let t = &set.brushes[1].brush.texture.as_ref().unwrap().image;
    assert_eq!(
        [t.at(0, 0), t.at(1, 0)],
        [76, 29],
        "赤と青の Rec. 601 のグレー"
    );
    assert!(set.brushes[1]
        .unrepresented
        .contains(&Unrepresented::TexturePattern(
            PatternNote::GreyConversion { indexed: false }
        )));
    let broken = abr_v6(&[
        section("desc", &desc),
        section("patt", &W::new().i32(9999).done()),
    ]);
    let set = read(FileKind::Abr, &broken).unwrap();
    assert!(
        set.notes
            .iter()
            .any(|n| matches!(n, Unrepresented::PatternsUnreadable(Fault::BadCount { .. }))),
        "壊れた模様の節でもブラシは取り込む"
    );
    assert!(
        set.brushes.iter().all(|b| !b
            .unrepresented
            .iter()
            .any(|n| matches!(n, Unrepresented::PatternsUnreadable(_)))),
        "ファイル全体の注記はブラシに複製しない"
    );
}

#[test]
fn pat_files_become_texture_brushes() {
    let g = pattern(
        1,
        "Grain",
        "g",
        2,
        2,
        &[vec![10, 20, 30, 40]],
        true,
        8,
        None,
    );
    let deep = pattern(
        1,
        "Deep",
        "d",
        1,
        2,
        &[vec![0x80, 0x01, 0x40, 0x02]],
        false,
        16,
        None,
    );
    let mut palette = vec![0u8; 768];
    palette[3 * 7 + 1] = 255;
    let indexed = pattern(
        2,
        "Indexed",
        "i",
        1,
        1,
        &[vec![7]],
        false,
        8,
        Some(&palette),
    );
    let cmyk = pattern(
        4,
        "Print",
        "c",
        1,
        1,
        &[vec![0], vec![0], vec![0], vec![0]],
        false,
        8,
        None,
    );
    let pat = |patterns: &[&Vec<u8>]| {
        let mut w = W::new().ascii("8BPT").i16(1).i32(patterns.len() as i64);
        for p in patterns {
            w = w.bytes(p);
        }
        w.done()
    };
    let set = read(FileKind::Pat, &pat(&[&g, &deep, &indexed, &cmyk])).unwrap();
    assert_eq!(
        set.brushes
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["Grain", "Deep", "Indexed"]
    );
    assert_eq!(set.brushes[0].source, Source::PhotoshopPattern);
    let t = &set.brushes[0].brush.texture.as_ref().unwrap();
    assert_eq!(
        [
            t.image.at(0, 0),
            t.image.at(1, 0),
            t.image.at(0, 1),
            t.image.at(1, 1)
        ],
        [30, 40, 10, 20],
        "行を下から並べ替える"
    );
    assert_eq!(t.depth, 1.0);
    assert!(set.brushes[0].brush.tip.image.is_none());
    assert!(
        set.notes.contains(&Unrepresented::PatternSkipped {
            name: "Print".into(),
            reason: PatternRefusal::Mode(PatternMode::Cmyk)
        }),
        "使えなかった模様はファイル全体の注記に 1 回"
    );
    assert!(
        set.brushes.iter().all(|b| !b
            .unrepresented
            .iter()
            .any(|n| matches!(n, Unrepresented::PatternSkipped { .. }))),
        "ブラシごとには複製しない"
    );
    let d = set.brushes[1].brush.texture.as_ref().unwrap();
    assert_eq!([d.image.at(0, 0), d.image.at(0, 1)], [0x40, 0x80]);
    assert!(set.brushes[1]
        .unrepresented
        .contains(&Unrepresented::Pattern(PatternNote::Reduced16Bit)));
    assert_eq!(
        set.brushes[2]
            .brush
            .texture
            .as_ref()
            .unwrap()
            .image
            .at(0, 0),
        150,
        "パレットの緑 → (255 × 587 + 500) / 1000"
    );

    assert_eq!(
        fault(read(
            FileKind::Pat,
            &W::new().ascii("8BPS").i16(1).i32(0).done()
        )),
        Fault::NotPat
    );
    assert_eq!(
        fault(read(
            FileKind::Pat,
            &W::new().ascii("8BPT").i16(2).i32(0).done()
        )),
        Fault::PatVersion(2)
    );
    assert_eq!(
        fault(read(
            FileKind::Pat,
            &W::new().ascii("8BPT").i16(1).i32(10001).done()
        )),
        Fault::PatCount(10001)
    );
    match read(FileKind::Pat, &pat(&[&cmyk])) {
        Err(BrushImportError::NoUsablePattern(skipped)) => {
            assert_eq!(skipped[0].reason, PatternRefusal::Mode(PatternMode::Cmyk))
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(read(FileKind::Pat, &W::new().ascii("8BPT").i16(1).i32(0).done()), Err(BrushImportError::NoUsablePattern(s)) if s.is_empty())
    );
    assert!(matches!(
        fault(read(FileKind::Pat, &pat(&[&g[..g.len() - 3].to_vec()]))),
        Fault::Truncated { .. } | Fault::BadCount { .. }
    ));
    let big = pattern(1, "Big", "b", 3000, 1, &[vec![0; 3000]], false, 8, None);
    match read(FileKind::Pat, &pat(&[&big])) {
        Err(BrushImportError::NoUsablePattern(skipped)) => assert_eq!(
            skipped[0].reason,
            PatternRefusal::TooLarge {
                width: 3000,
                height: 1
            }
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn pattern_notes_carry_a_stated_size_that_differs_and_extra_channels() {
    // 宣言の大きさ（4×4）とデータ（2×1）が違い、使わない透明度のチャンネルが 1 つ
    let mut body = W::new()
        .i32(1)
        .i32(1)
        .i16(4)
        .i16(4)
        .unicode("Odd")
        .u8(1)
        .ascii("o")
        .i32(3);
    let channel = |plane: &[u8]| {
        W::new()
            .i32(8)
            .i32(0)
            .i32(0)
            .i32(1)
            .i32(2)
            .i16(8)
            .u8(0)
            .bytes(plane)
            .done()
    };
    let (c0, c1) = (channel(&[5, 6]), channel(&[255, 255]));
    let list = W::new()
        .i32(0)
        .i32(0)
        .i32(1)
        .i32(2)
        .i32(1)
        .i32(1)
        .i32(c0.len() as i64)
        .bytes(&c0)
        .i32(1)
        .i32(c1.len() as i64)
        .bytes(&c1)
        .i32(0)
        .i32(0)
        .done();
    body = body.i32(list.len() as i64).bytes(&list);
    let file = W::new()
        .ascii("8BPT")
        .i16(1)
        .i32(1)
        .bytes(&body.done())
        .done();
    let set = read(FileKind::Pat, &file).unwrap();
    assert_eq!(
        set.brushes[0].unrepresented,
        vec![
            Unrepresented::Pattern(PatternNote::SizeDiffers {
                stated: (4, 4),
                data: (2, 1)
            }),
            Unrepresented::Pattern(PatternNote::ExtraChannelsIgnored(1))
        ]
    );
}

// ---------------- PNG ----------------

#[test]
fn png_tips_follow_the_dark_is_paint_convention() {
    use png::{BitDepth, ColorType};
    let rgba = png_file(
        3,
        1,
        ColorType::Rgba,
        BitDepth::Eight,
        &[0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0],
        None,
        None,
    );
    let b = import_bytes(FileKind::Png, &rgba, Some("Dot"))
        .unwrap()
        .brushes
        .remove(0);
    assert_eq!(b.name, "Dot");
    assert_eq!(b.source, Source::PngTip);
    let t = tip(&b);
    assert_eq!(
        [t.at(0, 0), t.at(1, 0), t.at(2, 0)],
        [255, 0, 0],
        "黒は塗り、白と透明は塗らない"
    );
    assert_eq!((b.brush.base.radius, b.brush.base.spacing), (1.5, 0.1));

    let gray = png_file(
        3,
        1,
        ColorType::Grayscale,
        BitDepth::Eight,
        &[0, 255, 128],
        None,
        None,
    );
    let t = read_tip(&gray);
    assert_eq!([t.at(0, 0), t.at(1, 0), t.at(2, 0)], [255, 0, 127]);
    // 半透明の黒は、アルファの割合だけ塗る
    let half = png_file(
        1,
        1,
        ColorType::GrayscaleAlpha,
        BitDepth::Eight,
        &[0, 128],
        None,
        None,
    );
    assert_eq!(read_tip(&half).at(0, 0), 128);
    // 行は上から。筆先は下の行が先
    let rows = png_file(
        1,
        2,
        ColorType::Grayscale,
        BitDepth::Eight,
        &[0, 255],
        None,
        None,
    );
    let t = read_tip(&rows);
    assert_eq!((t.at(0, 0), t.at(0, 1)), (0, 255));
    // 16 bit は上位バイト
    let deep = png_file(
        1,
        1,
        ColorType::Grayscale,
        BitDepth::Sixteen,
        &[0x40, 0xFF],
        None,
        None,
    );
    assert_eq!(read_tip(&deep).at(0, 0), 255 - 0x40);
    // パレットと tRNS（透明）
    let indexed = png_file(
        2,
        1,
        ColorType::Indexed,
        BitDepth::Eight,
        &[0, 1],
        Some(&[0, 0, 0, 255, 255, 255]),
        Some(&[255, 0]),
    );
    let t = read_tip(&indexed);
    assert_eq!([t.at(0, 0), t.at(1, 0)], [255, 0]);
    // 1 bit のグレー
    let bits = png_file(
        8,
        1,
        ColorType::Grayscale,
        BitDepth::One,
        &[0b1010_0000],
        None,
        None,
    );
    let t = read_tip(&bits);
    assert_eq!(
        (0..8).map(|x| t.at(x, 0)).collect::<Vec<_>>(),
        [0, 255, 0, 255, 255, 255, 255, 255]
    );
}

fn read_tip(png: &[u8]) -> BrushTip {
    yolu_io::brushes::read_png_tip(png, "t").unwrap()
}

#[test]
fn png_tips_that_are_too_large_or_not_png_are_refused_before_decoding() {
    use png::{BitDepth, ColorType};
    let wide = png_file(
        2049,
        1,
        ColorType::Grayscale,
        BitDepth::Eight,
        &vec![0; 2049],
        None,
        None,
    );
    assert!(matches!(
        fault(read(FileKind::Png, &wide)),
        Fault::SizeOutOfRange {
            width: 2049,
            height: 1,
            ..
        }
    ));
    let tall = png_file(
        1,
        100_000,
        ColorType::Grayscale,
        BitDepth::One,
        &vec![0; 100_000],
        None,
        None,
    );
    assert!(matches!(
        fault(read(FileKind::Png, &tall)),
        Fault::SizeOutOfRange {
            height: 100_000,
            ..
        }
    ));
    assert_eq!(
        fault(read(FileKind::Png, b"not a png at all")),
        Fault::NotPng
    );
    assert_eq!(fault(read(FileKind::Png, &[])), Fault::NotPng);
    let good = png_file(
        4,
        4,
        ColorType::Grayscale,
        BitDepth::Eight,
        &[7; 16],
        None,
        None,
    );
    for cut in 0..good.len() {
        let result = read(FileKind::Png, &good[..cut]);
        if cut < 41 {
            assert_eq!(
                fault(result),
                Fault::NotPng,
                "{cut} バイトで切る（画素に届く前）"
            );
        } else if let Err(e) = result {
            assert!(
                matches!(e, BrushImportError::Fault(Fault::NotPng)),
                "{cut}: {e:?}"
            );
        }
    }
    assert!(read(FileKind::Png, &good).is_ok());
}

// ---------------- 拡張子での読み分け ----------------

struct Dir(std::path::PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let path = std::env::temp_dir().join(
            format!(
                "yolu-brush-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )
            .replace(['(', ')', ' '], ""),
        );
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn files_are_read_by_extension_and_unsupported_ones_are_refused() {
    let dir = Dir::new("ext");
    let gbr_path = dir.file("soft_round.gbr", &gbr_gray(2, 2, &[255, 255, 0, 0], "", 25));
    let brush = import(&gbr_path).unwrap().brushes.remove(0);
    assert_eq!(brush.name, "Soft round", "名前の無いブラシはファイル名から");
    let vbr = dir.file("v.VBR", b"GIMP-VBR\n1.0\nHard\n10\n8\n1\n1\n0\n");
    assert_eq!(
        import(&vbr).unwrap().brushes[0].brush.base.radius,
        8.0,
        "拡張子は大文字小文字を区別しない"
    );
    for (file, expected) in [
        ("a.kpp", UnsupportedFile::KritaPreset),
        ("a.myb", UnsupportedFile::Extension("myb".into())),
        ("noextension", UnsupportedFile::Extension(String::new())),
    ] {
        let p = dir.file(file, &[0; 16]);
        assert!(
            matches!(import(&p), Err(BrushImportError::Unsupported(e)) if e == expected),
            "{file}"
        );
    }
    // .sut は読む形式（SQLite でなければ理由つきで断る）
    let sut = dir.file("a.sut", &[0; 16]);
    assert!(matches!(
        import(&sut),
        Err(BrushImportError::Fault(Fault::SutNotDatabase))
    ));
    assert!(
        matches!(import(&dir.0.join("missing.abr")), Err(BrushImportError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound)
    );
    let abr = dir.file(
        "set.ABR",
        &abr_v6(&[section("samp", &samp_record(1, "x", false))]),
    );
    assert_eq!(import(&abr).unwrap().brushes[0].name, "Set 1");
    let png = dir.file(
        "my-dot_2.png",
        &png_file(
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &[0],
            None,
            None,
        ),
    );
    assert_eq!(import(&png).unwrap().brushes[0].name, "My dot 2");
    let pat = dir.file(
        "p.pat",
        &W::new()
            .ascii("8BPT")
            .i16(1)
            .i32(1)
            .bytes(&pattern_gray("Tex", "t", 1, 1, &[9]))
            .done(),
    );
    assert_eq!(import(&pat).unwrap().brushes[0].name, "Tex");
    // 名前を持つ GIH
    let gih_path = dir.file("Hose.gih", &gih("Named\n1\n", &cells()[..1]));
    assert_eq!(import(&gih_path).unwrap().brushes[0].name, "Named");
}

#[test]
fn a_file_over_the_size_limit_is_refused_before_reading() {
    let dir = Dir::new("big");
    let path = dir.0.join("huge.abr");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(yolu_io::brushes::MAX_FILE_BYTES + 1).unwrap(); // 疎なファイル（ディスクは使わない）
    drop(file);
    assert!(
        matches!(import(&path), Err(BrushImportError::FileTooLarge { limit }) if limit == yolu_io::brushes::MAX_FILE_BYTES)
    );
}

#[test]
fn a_vbr_over_its_own_size_limit_is_refused_before_reading() {
    // パラメトリックブラシは 10 行ほどの文字。ABR なら許す大きさでも、VBR は読み込む前に断る
    let dir = Dir::new("bigvbr");
    for size in [
        yolu_io::brushes::MAX_VBR_BYTES + 1,
        yolu_io::brushes::MAX_FILE_BYTES,
    ] {
        let path = dir.0.join("huge.vbr");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(size).unwrap(); // 疎なファイル（ディスクは使わない）
        drop(file);
        assert!(
            matches!(import(&path), Err(BrushImportError::FileTooLarge { limit }) if limit == yolu_io::brushes::MAX_VBR_BYTES),
            "{size}"
        );
    }
    assert_eq!(
        FileKind::Vbr.max_bytes(),
        yolu_io::brushes::MAX_VBR_BYTES,
        "上限は種類ごと"
    );
    assert_eq!(FileKind::Abr.max_bytes(), yolu_io::brushes::MAX_FILE_BYTES);
}

#[test]
fn pretty_names_come_from_file_names() {
    assert_eq!(pretty_name("chalk_grainy-01.gbr"), "Chalk grainy 01");
    assert_eq!(
        pretty_name("__.gbr"),
        "__.gbr",
        "名前が空になるときはファイル名のまま"
    );
    assert_eq!(pretty_name("éclair.png"), "Éclair");
    assert_eq!(
        pretty_name("ßeta.png"),
        "ßeta",
        "大文字が 2 文字になる文字はそのまま"
    );
}

#[test]
fn file_kinds_list_the_supported_extensions() {
    for e in FileKind::EXTENSIONS {
        assert!(FileKind::from_extension(e).is_ok());
        assert!(FileKind::from_extension(&e.to_uppercase()).is_ok());
    }
}

// ---------------- core で描く ----------------

fn paint(brush: &Brush, size: u32, points: &[(f64, f64)]) -> Vec<u8> {
    let mut doc = Document::new(size, size).unwrap();
    let layer = doc.add_layer("L").unwrap();
    let mut b = brush.clone();
    b.base.color = Rgba8::new(0, 0, 0, 255);
    b.base.pressure_size = false;
    b.base.pressure_opacity = false;
    let mut stroke = doc.begin_brush_stroke(layer, &b).unwrap();
    for (i, (x, y)) in points.iter().enumerate() {
        stroke
            .add_point(&mut doc, *x, *y, 1.0, DVec2::ZERO)
            .unwrap();
        let _ = i;
    }
    assert!(doc.end_stroke(stroke).unwrap().changed);
    doc.composite(doc.bounds()).unwrap()
}
fn alpha_at(pixels: &[u8], size: u32, x: u32, y: u32) -> u8 {
    pixels[((y * size + x) * 4 + 3) as usize]
}

#[test]
fn an_imported_tip_paints_upright_on_the_canvas() {
    // 4x4 の筆先: ファイルの上の 2 行が 255、下の 2 行が 0（上の行はキャンバスの上 = y が大きい側）
    let mut rows = vec![255u8; 8];
    rows.extend([0u8; 8]);
    let mut brush = one(FileKind::Gbr, &gbr_gray(4, 4, &rows, "Half", 25)).brush;
    brush.base.radius = 8.0;
    let px = paint(&brush, 64, &[(32.0, 32.0), (33.0, 32.0)]);
    assert!(
        alpha_at(&px, 64, 32, 38) > 200,
        "筆先の上半分はキャンバスの上側に塗る"
    );
    assert_eq!(alpha_at(&px, 64, 32, 26), 0, "下半分は塗らない");
}

/// 横一列に並べたダブ（間隔 8 画素）を、左半分だけ塗るセル（L）・右半分だけ塗るセル（R）のどちらで塗ったかの並びにする。
fn hose_cells_used(header: &str) -> String {
    // 2x2 のセル: 左の列だけ 255 / 右の列だけ 255（行は上から。どの行も同じ）
    let left = gbr_gray(2, 2, &[255, 0, 255, 0], "left", 25);
    let right = gbr_gray(2, 2, &[0, 255, 0, 255], "right", 25);
    let hose = one(FileKind::Gih, &gih(header, &[left, right]));
    let mut brush = hose.brush;
    brush.base.radius = 4.0; // 直径 8 画素。セルの 1 画素が 4 画素
    brush.base.spacing = 1.0; // 8 画素ごと
    let px = paint(&brush, 128, &[(10.0, 32.0), (90.0, 32.0)]);
    (0..=10)
        .map(|k| {
            let centre = 10 + 8 * k;
            let (l, r) = (
                alpha_at(&px, 128, centre - 3, 32),
                alpha_at(&px, 128, centre + 3, 32),
            );
            // 塗った側は半分以上の濃さ、塗らない側はほぼ 0（縁のぼかしで 255 にはならない）
            match (l > 100 && r < 30, r > 100 && l < 30) {
                (true, false) => 'L',
                (false, true) => 'R',
                other => panic!("{k} 番目のダブがどちらのセルか決まらない: {other:?} ({l}, {r})"),
            }
        })
        .collect()
}

#[test]
fn an_imported_hose_paints_in_turn() {
    // incremental（sel0 が無い既定も同じ）は、ダブごとに 1 つ目・2 つ目のセルを交互に使う。見分けのつくセルで、塗られた画素から確かめる
    assert_eq!(hose_cells_used("Turn\n2 sel0:incremental\n"), "LRLRLRLRLRL");
    assert_eq!(hose_cells_used("Turn\n2\n"), "LRLRLRLRLRL");
    // random は同じ入力でも順番にならない（両方のセルが使われる）
    let random = hose_cells_used("Turn\n2 sel0:random\n");
    assert_ne!(random, "LRLRLRLRLRL");
    assert!(random.contains('L') && random.contains('R'), "{random}");
}

#[test]
fn every_imported_brush_validates_and_paints() {
    let set = read(FileKind::Abr, &dual_file(120.0)).unwrap();
    for b in &set.brushes {
        assert!(b.brush.validate().is_ok(), "{}", b.name);
    }
    let mut vbr = one(
        FileKind::Vbr,
        b"GIMP-VBR\n1.5\nStar\ndiamond\n50\n25\n5\n1\n2.5\n17.5\n",
    )
    .brush;
    vbr.base.radius = 10.0;
    let px = paint(&vbr, 64, &[(20.0, 32.0), (40.0, 32.0)]);
    assert!((0..64).any(|y| alpha_at(&px, 64, 32, y) > 0));
}

// ---------------- 文（日本語・英語） ----------------

#[test]
fn every_message_exists_in_both_languages() {
    let faults = vec![
        Fault::Truncated { offset: 3 },
        Fault::BadCount {
            what: yolu_io::brushes::Counted::PixelData,
            value: 9,
            offset: 1,
        },
        Fault::Budget,
        Fault::SizeOutOfRange {
            what: yolu_io::brushes::SizedItem::Tip,
            width: 5000,
            height: 1,
        },
        Fault::GbrHeaderSize {
            version: 2,
            size: 3,
        },
        Fault::GbrSignature,
        Fault::GbrNameTooLong,
        Fault::GbrHeaderMismatch,
        Fault::GbrCinePaint,
        Fault::GbrPixelSize(2),
        Fault::GbrVersion(9),
        Fault::GihHeader,
        Fault::GihCellCount,
        Fault::NotVbr,
        Fault::VbrEndsEarly { line: 4 },
        Fault::VbrNumber {
            line: 4,
            min: 0.0,
            max: 1.0,
        },
        Fault::VbrShape("hex".into()),
        Fault::VbrVersion("9".into()),
        Fault::AbrVersion(3),
        Fault::AbrSubversion(3),
        Fault::AbrBrushCount(-1),
        Fault::AbrBrushTruncated { index: 1 },
        Fault::AbrBrushType { index: 1, kind: 9 },
        Fault::AbrTipCount(10_000),
        Fault::AbrSection { offset: 4 },
        Fault::TipDepth(4),
        Fault::Tip16BitCompressed,
        Fault::TipCompression(2),
        Fault::PackBits(yolu_io::brushes::PackBitsFault::EndedEarly),
        Fault::PackBits(yolu_io::brushes::PackBitsFault::Overflow),
        Fault::AbrNoBrushes,
        Fault::DescriptorDepth,
        Fault::DescriptorItems,
        Fault::DescriptorType {
            kind: "ObAr".into(),
            offset: 9,
        },
        Fault::DescriptorReference("zzzz".into()),
        Fault::DescriptorNotFinite,
        Fault::NotPat,
        Fault::PatVersion(2),
        Fault::PatCount(99999),
        Fault::PatternVersion(2),
        Fault::PatternDataVersion(2),
        Fault::PatternChannelCount(99),
        Fault::PatternChannelLengthTooShort(3),
        Fault::PatternEmptyRecord,
        Fault::NoPatterns,
        Fault::NotPng,
        Fault::PngLimits,
        Fault::SutNotDatabase,
        Fault::SutNoNodeTable,
        Fault::SutNoBrushes,
        Fault::SutLimits,
    ];
    let refusals = vec![
        PatternRefusal::ChannelsDiffer,
        PatternRefusal::TooLarge {
            width: 3000,
            height: 1,
        },
        PatternRefusal::Depth(32),
        PatternRefusal::Zip,
        PatternRefusal::Mode(PatternMode::Bitmap),
        PatternRefusal::Mode(PatternMode::Cmyk),
        PatternRefusal::Mode(PatternMode::Multichannel),
        PatternRefusal::Mode(PatternMode::Duotone),
        PatternRefusal::Mode(PatternMode::Lab),
        PatternRefusal::Mode(PatternMode::Other(12)),
        PatternRefusal::ChannelsMissing,
    ];
    let mut errors: Vec<BrushImportError> = faults
        .iter()
        .cloned()
        .map(BrushImportError::Fault)
        .collect();
    errors.push(BrushImportError::Io(std::io::Error::from(
        std::io::ErrorKind::NotFound,
    )));
    errors.push(BrushImportError::FileTooLarge { limit: 1 });
    errors.push(BrushImportError::Unsupported(UnsupportedFile::KritaPreset));
    errors.push(BrushImportError::Unsupported(UnsupportedFile::Extension(
        "xyz".into(),
    )));
    errors.push(BrushImportError::NoUsablePattern(Vec::new()));
    errors.push(BrushImportError::NoUsablePattern(
        refusals
            .iter()
            .map(|r| yolu_io::brushes::SkippedPattern {
                name: "P".into(),
                reason: r.clone(),
            })
            .collect(),
    ));
    errors.push(BrushImportError::Core(
        yolu_core::CoreError::InvalidArgument("x"),
    ));
    for e in &errors {
        let (ja, en) = (e.to_string(), e.english());
        assert!(
            !ja.is_empty() && !ja.is_ascii(),
            "日本語の文が日本語でない: {en}"
        );
        assert!(
            !en.is_empty() && en.is_ascii(),
            "英語の文に日本語が混ざる: {ja}"
        );
        // 画面の文は名前・状態・短い理由だけ。使い方の指示は書かない
        assert!(!ja.contains("ください"), "{ja}");
        let lower = en.to_lowercase();
        assert!(
            !lower.contains("please") && !lower.contains("instead"),
            "{en}"
        );
    }
    for f in &faults {
        assert!(!f.to_string().is_empty() && f.english().is_ascii());
    }
    let notes = vec![
        Unrepresented::ColorTipAsMask,
        Unrepresented::HoseSelection {
            mode: "pressure".into(),
        },
        Unrepresented::HoseDimensions { dim: "3".into() },
        Unrepresented::HoseShort {
            declared: 3,
            present: 2,
        },
        Unrepresented::GbrTrailingData { bytes: 3 },
        Unrepresented::VbrShapeRendered {
            shape: VbrShape::Circle,
            spikes: 5,
        },
        Unrepresented::VbrShapeRendered {
            shape: VbrShape::Square,
            spikes: 2,
        },
        Unrepresented::Tip16Bit,
        Unrepresented::PresetsUnreadable(Fault::NotPng),
        Unrepresented::PatternsUnreadable(Fault::NotPng),
        Unrepresented::SectionSkipped("zzzz".into()),
        Unrepresented::MoreSectionsSkipped(5),
        Unrepresented::UnknownTipKind("k".into()),
        Unrepresented::MinimumDiameter(25.0),
        Unrepresented::Control {
            setting: Setting::DualScatter,
            control: ControlKind::Unknown(99),
        },
        Unrepresented::ForegroundBackgroundControl(ControlKind::Rotation),
        Unrepresented::FadeRange {
            setting: Setting::Opacity,
            steps: 0.0,
        },
        Unrepresented::ScatterOneAxis,
        Unrepresented::CountJitter,
        Unrepresented::Noise,
        Unrepresented::WetEdges,
        Unrepresented::Texture(TextureNote::PatternMissing { name: "p".into() }),
        Unrepresented::Texture(TextureNote::PatternRefused {
            name: "p".into(),
            reason: PatternRefusal::Zip,
        }),
        Unrepresented::Texture(TextureNote::ScaleClamped {
            percent: 1.0,
            used_percent: 5.0,
        }),
        Unrepresented::Texture(TextureNote::Mode("m".into())),
        Unrepresented::Texture(TextureNote::EachTip),
        Unrepresented::Texture(TextureNote::DepthDynamics),
        Unrepresented::Texture(TextureNote::Brightness),
        Unrepresented::Texture(TextureNote::Contrast),
        Unrepresented::TexturePattern(PatternNote::Reduced16Bit),
        Unrepresented::TexturePattern(PatternNote::GreyConversion { indexed: true }),
        Unrepresented::Pattern(PatternNote::SizeDiffers {
            stated: (1, 2),
            data: (3, 4),
        }),
        Unrepresented::Pattern(PatternNote::ExtraChannelsIgnored(2)),
        Unrepresented::Dual(DualNote::MissingTip),
        Unrepresented::Dual(DualNote::TipNotInFile),
        Unrepresented::Dual(DualNote::UnknownTipKind("k".into())),
        Unrepresented::Dual(DualNote::Mode("m".into())),
        Unrepresented::Dual(DualNote::ScatterOneAxis),
        Unrepresented::Dual(DualNote::CountJitter),
        Unrepresented::Dual(DualNote::Flip),
        Unrepresented::PatternSkipped {
            name: "p".into(),
            reason: PatternRefusal::Depth(32),
        },
    ];
    for n in notes {
        let (ja, en) = (n.to_string(), n.english());
        assert!(!ja.is_empty() && !ja.is_ascii(), "{n:?}: {en}");
        assert!(!en.is_empty() && en.is_ascii(), "{n:?}: {ja}");
    }
    for c in [
        ControlKind::Fade,
        ControlKind::PenPressure,
        ControlKind::PenTilt,
        ControlKind::StylusWheel,
        ControlKind::InitialDirection,
        ControlKind::Direction,
    ] {
        let n = Unrepresented::Control {
            setting: Setting::Size,
            control: c,
        };
        assert!(!n.to_string().is_empty() && n.english().is_ascii());
    }
    for s in [
        Setting::Angle,
        Setting::Roundness,
        Setting::Flow,
        Setting::Scatter,
    ] {
        let n = Unrepresented::Control {
            setting: s,
            control: ControlKind::Fade,
        };
        assert!(n.english().is_ascii() && !n.to_string().is_ascii());
    }
}

#[test]
fn messages_do_not_show_byte_offsets() {
    // オフセットなどの内部の数は Fault の値のまま運び（ログ・試験は Debug で読める）、画面の文には出さない
    let faults = [
        Fault::Truncated { offset: 4711 },
        Fault::BadCount {
            what: yolu_io::brushes::Counted::PixelData,
            value: 4712,
            offset: 4713,
        },
        Fault::AbrSection { offset: 4714 },
        Fault::DescriptorType {
            kind: "ObAr".into(),
            offset: 4715,
        },
    ];
    for f in &faults {
        for text in [f.to_string(), f.english()] {
            assert!(!text.contains("471"), "{text}");
        }
        assert!(format!("{f:?}").contains("471"), "値は型に残る");
    }
    let kpp = BrushImportError::Unsupported(UnsupportedFile::KritaPreset);
    assert_eq!(
        kpp.to_string(),
        "Krita のブラシプリセット（.kpp）は未対応です"
    );
    assert_eq!(
        kpp.english(),
        "Krita brush presets (.kpp) are not supported."
    );
    // 実際の壊れ方でも（読む途中で尽きる・宣言が残りに入らない）
    let good = gbr_gray(2, 2, &[0; 4], "Test", 25);
    for result in [
        read(FileKind::Gbr, &good[..good.len() - 1]),
        read(
            FileKind::Abr,
            &W::new().i16(6).i16(1).ascii("8BIMsamp").i32(-1).done(),
        ),
    ] {
        let e = result.unwrap_err();
        for text in [e.to_string(), e.english()] {
            assert!(!text.chars().any(|c| c.is_ascii_digit()), "{text}");
        }
    }
}

#[test]
fn notes_from_a_file_are_short_and_have_no_control_characters() {
    // 画像ホースのヘッダー: 名前にも、選び方・次元の値にも、改行以外の制御文字（BEL・SOH・CR）が混じる長い値
    let noisy = "p\u{7}r\u{1}e\rs".repeat(50);
    let header = format!("N\u{7}ame\r\n2 ncells:2 dim:3\u{7} sel0:{noisy}\n");
    let set = import_bytes(FileKind::Gih, &gih(&header, &cells()[..2]), None)
        .expect("改行を含まない値なので、ホースとして読める");
    let hose = &set.brushes[0];
    assert_eq!(hose.name, "Name");
    assert_eq!(
        hose.brush.tip.selection,
        TipSelection::Random,
        "知らない選び方はランダム"
    );
    assert!(hose.unrepresented.contains(&Unrepresented::HoseSelection {
        mode: "pres".repeat(8)
    }));
    assert!(hose
        .unrepresented
        .contains(&Unrepresented::HoseDimensions { dim: "3".into() }));
    for n in &hose.unrepresented {
        for text in [n.to_string(), n.english()] {
            assert!(!text.chars().any(|c| c.is_control()), "{text:?}");
            assert!(text.chars().count() < 200, "{text:?}");
        }
    }
    let named = one(
        FileKind::Gbr,
        &gbr_gray(1, 1, &[0], "Line1\nLine2\u{7}", 25),
    );
    assert_eq!(named.name, "Line1Line2");
}
