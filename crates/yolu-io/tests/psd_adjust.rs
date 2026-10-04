//! PSD の調整レイヤーとしての、色調補正の 6 種（グラデーションマップ `grdm`・トーンカーブ `curv`・カラーバランス `blnc`・明るさ/コントラスト `brit`・
//! 2 値化 `thrs`・ポスタリゼーション `post`）の読み書き。PSD の刻みに乗る値はそのまま往復し、刻みの間の値は丸めず層ごとの理由で断り、
//! 読み込みで表せないものは PreserveOnly と診断する。Photoshop・CLIP STUDIO の実物のファイルは持ち込まず、試験の中で一から組む
//! （書いた PSD を psd-tools で読み直す確かめは `tools/io-fixtures/psd_tools_check.py`。Photoshop の実機は未確認）。
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, BlendMode as CoreBlend, BrightnessContrast, Channel, ColorBalance,
    Document, GradientMap, Posterize, Rgba8, Threshold, ToneChannel, ToneCurves,
};
use yolu_io::psd::{
    self, Adjustment, Blocker, CompatibilityMode as Mode, Document as Psd, GradientColorStop,
    GradientOpacityStop, LayerKind, Limits, Refusal,
};

const W: u32 = 24;
const H: u32 = 16;

fn new_doc() -> Document {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let id = d.add_layer("bg").unwrap();
    for y in 0..H {
        for x in 0..W {
            d.set_pixel(
                id,
                x,
                y,
                Rgba8::new(
                    (x * 10 + 15) as u8,
                    (y * 14 + 20) as u8,
                    (200 - x * 5) as u8,
                    255,
                ),
            )
            .unwrap();
        }
    }
    d
}

fn curve(list: &[(u32, u32)]) -> Curve {
    Curve::new(
        list.iter()
            .map(|&(x, y)| CurvePoint {
                x: f64::from(x) / 255.0,
                y: f64::from(y) / 255.0,
            })
            .collect(),
    )
    .unwrap()
}

/// PSD の刻みに乗る値の 6 種。
fn exact() -> Vec<(&'static str, AdjustmentSettings)> {
    let stop = |position: f64, rgb: [u8; 3], midpoint| ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    };
    let opacity = |position, byte: u8, midpoint| OpacityStop {
        position,
        opacity: f64::from(byte) / 255.0,
        midpoint,
    };
    let ramp = Ramp::new(
        vec![
            stop(0.0, [10, 20, 90], 0.35),
            stop(1000.0 / 4096.0, [200, 60, 30], 0.6),
            stop(1.0, [250, 240, 200], 0.5),
        ],
        vec![
            opacity(0.0, 255, 0.5),
            opacity(0.5, 204, 0.25),
            opacity(1.0, 128, 0.5),
        ],
        None,
    )
    .unwrap();
    vec![
        (
            "グラデーションマップ",
            AdjustmentSettings::gradient_map(GradientMap::new(ramp, true)),
        ),
        (
            "トーンカーブ",
            AdjustmentSettings::tone_curve(ToneCurves::new(
                curve(&[(0, 0), (64, 80), (180, 150), (255, 255)]),
                curve(&[(0, 10), (255, 230)]),
                Curve::identity(),
                curve(&[(0, 0), (128, 170), (255, 255)]),
            )),
        ),
        (
            "カラーバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [-100.0, 12.0, 3.0],
                    [0.0, -40.0, 100.0],
                    [20.0, 0.0, -8.0],
                    false,
                )
                .unwrap(),
            ),
        ),
        (
            "明るさ・コントラスト",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(-37.0, 81.0).unwrap()),
        ),
        (
            "2 値化",
            AdjustmentSettings::threshold(Threshold::new(77).unwrap()),
        ),
        (
            "ポスタリゼーション",
            AdjustmentSettings::posterize(Posterize::new(7).unwrap()),
        ),
    ]
}

/// core → PSD → バイト列 → 読み → core。診断が無い・原本の書き戻しが同じバイト列・書き出し直すと同じ PSD、まで確かめる。
fn round_trip(core: &Document) -> (Document, Vec<u8>) {
    let projected = Psd::from_core(core).unwrap();
    let bytes = psd::write(&projected, &Limits::default()).unwrap();
    let read = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        read.mode(),
        Mode::EditableRaster,
        "{:?}",
        read.diagnostics()
    );
    assert!(
        read.diagnostics().iter().all(|d| d.is_informational()),
        "{:?}",
        read.diagnostics()
    );
    let doc = read.document().unwrap();
    assert_eq!(doc.layers, projected.layers, "読み直した層は書いた層と同じ");
    assert_eq!(
        psd::write_edited(&read, doc, &Limits::default()).unwrap(),
        bytes,
        "原本の書き戻しは同じバイト列"
    );
    let back = read.to_core().unwrap();
    assert_eq!(
        Psd::from_core(&back).unwrap(),
        projected,
        "書き出し直すと同じ PSD"
    );
    (back, bytes)
}

fn adjustments(d: &Document) -> Vec<AdjustmentSettings> {
    d.layers()
        .iter()
        .filter_map(|l| l.adjustment().cloned())
        .collect()
}

#[test]
fn each_kind_round_trips_with_the_same_settings_and_composite() {
    for (name, s) in exact() {
        let mut d = new_doc();
        d.add_adjustment_layer(name, s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        let (back, _) = round_trip(&d);
        assert_eq!(adjustments(&back), vec![s.clone()], "{name}: 設定");
        assert_eq!(
            back.composite(back.bounds()).unwrap(),
            d.composite(d.bounds()).unwrap(),
            "{name}: 合成"
        );
        assert_ne!(
            d.composite(d.bounds()).unwrap(),
            new_doc().composite(d.bounds()).unwrap(),
            "{name}: 絵が変わっている"
        );
    }
}

#[test]
fn each_kind_round_trips_with_opacity_mask_clipping_blend_mode_and_hidden() {
    for (name, s) in exact() {
        let mut d = new_doc();
        let plain = d
            .add_adjustment_layer("plain", s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        d.set_layer_opacity(plain, 160.0 / 255.0, false).unwrap();
        d.add_layer_mask(plain).unwrap();
        for x in 0..W {
            d.set_mask_pixel(plain, x, 5, 200).unwrap();
        }
        let shape = d.add_layer("shape").unwrap();
        // 画布いっぱいの絵（書き出しは各層の画素の範囲を書くので、疎な層は読み直すと範囲が変わる）
        for y in 0..H {
            for x in 0..W {
                let a = if x < 3 { 0 } else { (60 + x * 8 + y * 3) as u8 };
                d.set_pixel(shape, x, y, Rgba8::new(240, (x * 9) as u8, 60, a))
                    .unwrap();
            }
        }
        let clipped = d
            .add_adjustment_layer("clipped", s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        d.set_layer_clipping(clipped, true).unwrap();
        d.set_layer_blend_mode(clipped, CoreBlend::Overlay).unwrap();
        let hidden = d
            .add_adjustment_layer("hidden", s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        d.set_layer_visible(hidden, false).unwrap();
        let (back, bytes) = round_trip(&d);
        assert_eq!(adjustments(&back).len(), 3, "{name}");
        assert!(adjustments(&back).iter().all(|a| *a == s), "{name}");
        assert_eq!(
            back.composite(back.bounds()).unwrap(),
            d.composite(d.bounds()).unwrap(),
            "{name}"
        );
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        let layers = &read.document().unwrap().layers;
        assert!(matches!(layers[3].kind, LayerKind::Adjustment(_)), "{name}");
        assert!(layers[3].mask.is_some(), "{name}");
        assert!(layers[1].clipping && !layers[0].visible, "{name}");
    }
}

#[test]
fn the_psd_carries_the_steps_of_each_record() {
    let read_adjustment = |s: &AdjustmentSettings| {
        let mut d = new_doc();
        d.add_adjustment_layer("a", s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        let (_, bytes) = round_trip(&d);
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        match &read.document().unwrap().layers[0].kind {
            LayerKind::Adjustment(a) => a.clone(),
            other => panic!("{other:?}"),
        }
    };
    let all = exact();
    assert_eq!(
        read_adjustment(&all[0].1),
        Adjustment::GradientMap {
            reverse: true,
            colors: vec![
                GradientColorStop {
                    location: 0,
                    midpoint: 35,
                    rgb: [10, 20, 90]
                },
                GradientColorStop {
                    location: 1000,
                    midpoint: 60,
                    rgb: [200, 60, 30]
                },
                // 最後の分岐点の中点は使われないので 50 で書く
                GradientColorStop {
                    location: 4096,
                    midpoint: 50,
                    rgb: [250, 240, 200]
                },
            ],
            opacities: vec![
                GradientOpacityStop {
                    location: 0,
                    midpoint: 50,
                    opacity: 255
                },
                GradientOpacityStop {
                    location: 2048,
                    midpoint: 25,
                    opacity: 204
                },
                GradientOpacityStop {
                    location: 4096,
                    midpoint: 50,
                    opacity: 128
                },
            ],
        }
    );
    assert_eq!(
        read_adjustment(&all[1].1),
        Adjustment::ToneCurve {
            composite: vec![[0, 0], [64, 80], [180, 150], [255, 255]],
            red: vec![[0, 10], [255, 230]],
            green: vec![[0, 0], [255, 255]],
            blue: vec![[0, 0], [128, 170], [255, 255]],
        }
    );
    assert_eq!(
        read_adjustment(&all[2].1),
        Adjustment::ColorBalance {
            shadows: [-100, 12, 3],
            midtones: [0, -40, 100],
            highlights: [20, 0, -8],
            preserve_luminosity: false,
        }
    );
    assert_eq!(
        read_adjustment(&all[3].1),
        Adjustment::BrightnessContrast {
            brightness: -37,
            contrast: 81
        }
    );
    assert_eq!(
        read_adjustment(&all[4].1),
        Adjustment::Threshold { level: 77 }
    );
    assert_eq!(
        read_adjustment(&all[5].1),
        Adjustment::Posterize { levels: 7 }
    );
}

fn blockers(d: &Document) -> Vec<Blocker> {
    psd::export_blockers(d)
}
fn adjustment_document(s: AdjustmentSettings) -> Document {
    let mut d = new_doc();
    d.add_adjustment_layer("調整", s, Some(&[Channel::Color]), None)
        .unwrap();
    d
}
fn only(refusal: Refusal, d: &Document) {
    let list = blockers(d);
    assert_eq!(
        list.iter().map(|b| b.refusal.clone()).collect::<Vec<_>>(),
        vec![refusal],
        "{list:?}"
    );
    let err = Psd::from_core(d).unwrap_err().to_string();
    assert!(err.contains("調整") || err.contains("「"), "{err}");
}

#[test]
fn values_between_the_psd_steps_are_refused_one_reason_each_and_never_rounded() {
    let stop = |position, midpoint| ColorStop {
        position,
        color: Rgba8::new(10, 20, 30, 255),
        midpoint,
    };
    let opacity = |position, opacity, midpoint| OpacityStop {
        position,
        opacity,
        midpoint,
    };
    let gradient =
        |colors: Vec<ColorStop>, opacities: Vec<OpacityStop>, curve: Option<Vec<CurvePoint>>| {
            adjustment_document(AdjustmentSettings::gradient_map(GradientMap::new(
                Ramp::new(colors, opacities, curve).unwrap(),
                false,
            )))
        };
    let flat = || vec![opacity(0.0, 1.0, 0.5), opacity(1.0, 1.0, 0.5)];
    // 位置（1/4096 の刻みの間）
    only(
        Refusal::GradientMapBetweenSteps,
        &gradient(vec![stop(0.0, 0.5), stop(0.3, 0.5)], flat(), None),
    );
    // 中点（1% の刻みの間）
    only(
        Refusal::GradientMapBetweenSteps,
        &gradient(vec![stop(0.0, 0.333), stop(1.0, 0.5)], flat(), None),
    );
    // 不透明度（1/255 の刻みの間。0.5 は 127.5）
    only(
        Refusal::GradientMapBetweenSteps,
        &gradient(
            vec![stop(0.0, 0.5), stop(1.0, 0.5)],
            vec![opacity(0.0, 0.5, 0.5), opacity(1.0, 1.0, 0.5)],
            None,
        ),
    );
    // 値のカーブがある（PSD のグラデーションに形が無い）
    only(
        Refusal::GradientMapCurve,
        &gradient(
            vec![stop(0.0, 0.5), stop(1.0, 0.5)],
            flat(),
            Some(vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 0.5, y: 0.7 },
                CurvePoint { x: 1.0, y: 1.0 },
            ]),
        ),
    );
    // 最後の分岐点の中点は使われないので、刻みの間でも断らない
    let ok = gradient(vec![stop(0.0, 0.5), stop(1.0, 0.37)], flat(), None);
    assert!(blockers(&ok).is_empty());
    // トーンカーブの点（1/255 の刻みの間）
    let off = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.0 },
        CurvePoint { x: 0.3, y: 0.3 },
        CurvePoint { x: 1.0, y: 1.0 },
    ])
    .unwrap();
    for channel in ToneChannel::ALL {
        only(
            Refusal::ToneCurveBetweenSteps,
            &adjustment_document(AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(channel, off.clone()),
            )),
        );
    }
    // カラーバランス・明るさ/コントラスト（整数の刻みの間）
    only(
        Refusal::ColorBalanceBetweenSteps,
        &adjustment_document(AdjustmentSettings::color_balance(
            ColorBalance::new([0.0, 0.0, 0.5], [0.0; 3], [0.0; 3], true).unwrap(),
        )),
    );
    only(
        Refusal::BrightnessContrastBetweenSteps,
        &adjustment_document(AdjustmentSettings::brightness_contrast(
            BrightnessContrast::new(10.5, 0.0).unwrap(),
        )),
    );
    only(
        Refusal::BrightnessContrastBetweenSteps,
        &adjustment_document(AdjustmentSettings::brightness_contrast(
            BrightnessContrast::new(10.0, -0.25).unwrap(),
        )),
    );
    // 整数だけの 2 値化・ポスタリゼーションは刻みの間が無い（端の値も通る）
    for s in [
        AdjustmentSettings::threshold(Threshold::new(1).unwrap()),
        AdjustmentSettings::threshold(Threshold::new(255).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(2).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(255).unwrap()),
    ] {
        let d = adjustment_document(s.clone());
        assert!(blockers(&d).is_empty());
        let (back, _) = round_trip(&d);
        assert_eq!(adjustments(&back), vec![s]);
    }
}

#[test]
fn a_new_adjustment_that_does_not_act_on_color_is_refused_like_the_others() {
    let mut d = new_doc();
    let s = AdjustmentSettings::tone_curve(ToneCurves::identity());
    let id = d
        .add_adjustment_layer("調整", s, Some(&[Channel::Roughness]), None)
        .unwrap();
    assert!(!d.layer(id).unwrap().is_channel_enabled(Channel::Color));
    only(Refusal::AdjustmentColorDisabled, &d);
}

/// 書いた PSD の指定のタグの本体の `at` 番目のバイトから `with` で書き換える。
fn patch(bytes: &[u8], key: &[u8; 4], at: usize, with: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let tag: Vec<u8> = [b"8BIM".as_slice(), key.as_slice()].concat();
    let start = out
        .windows(8)
        .position(|w| w == tag.as_slice())
        .unwrap_or_else(|| panic!("{key:?}"));
    let body = start + 12;
    out[body + at..body + at + with.len()].copy_from_slice(with);
    out
}
fn written(s: &AdjustmentSettings) -> Vec<u8> {
    let (_, bytes) = round_trip(&adjustment_document(s.clone()));
    bytes
}
fn read_back(bytes: &[u8]) -> (Mode, Vec<String>) {
    let read = psd::read(bytes, &Limits::default()).unwrap();
    (
        read.mode(),
        read.diagnostics().iter().map(|d| d.code.clone()).collect(),
    )
}
#[track_caller]
fn preserved(bytes: &[u8], code: &str) {
    let (mode, codes) = read_back(bytes);
    assert_eq!(mode, Mode::PreserveOnly, "{code}: {codes:?}");
    assert!(codes.iter().any(|c| c == code), "{code}: {codes:?}");
}

#[test]
fn what_the_tool_cannot_represent_is_preserved_not_edited() {
    let all = exact();
    // しきい値 0・256 / 階調 1・256 / 未知の末尾
    let thrs = written(&all[4].1);
    preserved(&patch(&thrs, b"thrs", 0, &[0, 0]), "TaggedBlock");
    preserved(&patch(&thrs, b"thrs", 0, &[1, 0]), "TaggedBlock");
    preserved(&patch(&thrs, b"thrs", 2, &[0, 9]), "TaggedBlock");
    let post = written(&all[5].1);
    preserved(&patch(&post, b"post", 0, &[0, 1]), "TaggedBlock");
    preserved(&patch(&post, b"post", 0, &[1, 0]), "TaggedBlock");
    // 明るさ・コントラストは、旧式の brit と新しい式の CgEd の両方を書く。編集できるのは新しい式（旧式・自動・Lab でない）で、
    // brit と CgEd が同じ値のもの。範囲外（明るさ 151・コントラスト −51・101）・旧式・自動・Lab・版・食い違いは保護する。
    // CgEd の本体: 版 16（4）・名前（6）・クラス（8）・項目数（4）の後、項目は Vrsn・Brgh・Cntr が 16 バイトずつ（値は 12 バイト目から）、
    // means（17）・Lab（13）・useLegacy（18）・auto（13）。値の位置は Vrsn 34・Brgh 50・Cntr 66・Lab 99・useLegacy 117・auto 130
    let brit = written(&all[3].1);
    let both = |brightness: Option<i16>, contrast: Option<i16>| {
        let mut b = brit.clone();
        if let Some(v) = brightness {
            b = patch(&b, b"brit", 0, &v.to_be_bytes());
            b = patch(&b, b"CgEd", 50, &i32::from(v).to_be_bytes());
        }
        if let Some(v) = contrast {
            b = patch(&b, b"brit", 2, &v.to_be_bytes());
            b = patch(&b, b"CgEd", 66, &i32::from(v).to_be_bytes());
        }
        b
    };
    for (brightness, contrast) in [(151, 0), (-151, 0), (0, 101), (0, -51)] {
        let (b, c) = (
            (brightness != 0).then_some(brightness),
            (contrast != 0).then_some(contrast),
        );
        preserved(&both(b, c), "TaggedBlock");
    }
    // 範囲の端（明るさ ±150・コントラスト −50 と 100）は通る
    for (brightness, contrast) in [(150, 100), (-150, -50)] {
        let (mode, codes) = read_back(&both(Some(brightness), Some(contrast)));
        assert_eq!(
            mode,
            Mode::EditableRaster,
            "{brightness} {contrast} {codes:?}"
        );
    }
    // brit だけ違う（食い違い）・CgEd の旧式・自動・Lab・版
    preserved(
        &patch(&brit, b"brit", 0, &9i16.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(&patch(&brit, b"brit", 6, &[1]), "TaggedBlock");
    preserved(&patch(&brit, b"CgEd", 117, &[1]), "TaggedBlock");
    preserved(&patch(&brit, b"CgEd", 130, &[1]), "TaggedBlock");
    preserved(&patch(&brit, b"CgEd", 99, &[1]), "TaggedBlock");
    preserved(
        &patch(&brit, b"CgEd", 34, &2i32.to_be_bytes()),
        "TaggedBlock",
    );
    // 項目の型の違い（Brgh の型 long を bool に。値の長さが合わなくなる）・知らない項目名（Brgh → Brgx）は断る
    preserved(&patch(&brit, b"CgEd", 46, b"bool"), "TaggedBlock");
    preserved(&patch(&brit, b"CgEd", 42, b"Brgx"), "TaggedBlock");
    // カラーバランス: 範囲外・輝度の印が 0 か 1 でない
    let blnc = written(&all[2].1);
    preserved(
        &patch(&blnc, b"blnc", 0, &101i16.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&blnc, b"blnc", 16, &(-101i16).to_be_bytes()),
        "TaggedBlock",
    );
    preserved(&patch(&blnc, b"blnc", 18, &[2]), "TaggedBlock");
    // トーンカーブ: 画素の表の形式・版 4・知らないチャンネルのビット・点が 0〜255 の外・入力が昇順でない
    let curv = written(&all[1].1);
    preserved(&patch(&curv, b"curv", 0, &[1]), "TaggedBlock");
    preserved(
        &patch(&curv, b"curv", 1, &4u16.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&curv, b"curv", 3, &0x1fu32.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&curv, b"curv", 3, &0u32.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&curv, b"curv", 9, &256u16.to_be_bytes()),
        "TaggedBlock",
    );
    // 合成の曲線の 2 点目の入力を 1 点目（0）と同じにする（昇順でない）。点は 出力・入力 の順で、2 点目の入力は 15 バイト目
    preserved(
        &patch(&curv, b"curv", 15, &0u16.to_be_bytes()),
        "TaggedBlock",
    );
    // グラデーションマップ: ディザ・未知の版・補間・中点の端・RGB でない色・ノイズ型
    let grdm = written(&all[0].1);
    preserved(&patch(&grdm, b"grdm", 3, &[1]), "TaggedBlock");
    preserved(
        &patch(&grdm, b"grdm", 0, &3u16.to_be_bytes()),
        "TaggedBlock",
    );
    // 1 つ目の色の分岐点: 位置 4 バイト・中点 4 バイト（10 バイト目から 4 バイト）・モード 2 バイト
    // 本体: 版 2・逆向き 1・ディザ 1・名前 4（空）・数 2 の後に分岐点が始まる（10 バイト目）
    preserved(
        &patch(&grdm, b"grdm", 14, &0u32.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&grdm, b"grdm", 14, &100u32.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&grdm, b"grdm", 18, &3u16.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&grdm, b"grdm", 10, &5000u32.to_be_bytes()),
        "TaggedBlock",
    );
    // 補間の欄（なめらかさ 4096 = 100%）を 2048 にする: 色 3 つ（20 バイト × 3）と不透明度 3 つ（10 バイト × 3）の後
    let smooth = 10 + 3 * 20 + 2 + 3 * 10 + 2;
    preserved(
        &patch(&grdm, b"grdm", smooth, &2048u16.to_be_bytes()),
        "TaggedBlock",
    );
    preserved(
        &patch(&grdm, b"grdm", smooth + 4, &1u16.to_be_bytes()),
        "TaggedBlock",
    );
    // 同じ値のまま（何も書き換えない）なら通る
    assert_eq!(read_back(&grdm).0, Mode::EditableRaster);
    // 色の 16 bit が 257 の倍数でなければ、丸めて通し「端数」を知らせる
    let (mode, codes) = read_back(&patch(&grdm, b"grdm", 20, &(100u16).to_be_bytes()));
    assert_eq!(mode, Mode::EditableRaster, "{codes:?}");
}

#[test]
fn a_curve_that_is_valid_as_psd_but_not_for_the_tool_is_preserved_not_edited() {
    // Photoshop の「自動」は両端の点を動かす。PSD として正しくても、この道具の曲線（点は 16 まで・両端の入力は 0 と 255・隣は 6 刻み以上）で
    // 表せないものは、編集できる層にせず（取り込みで理由が分からないまま断らず）原本を保つ
    let with_composite = |points: Vec<[u8; 2]>| {
        let mut projected = Psd::from_core(&adjustment_document(AdjustmentSettings::tone_curve(
            ToneCurves::identity(),
        )))
        .unwrap();
        let mut found = false;
        for layer in &mut projected.layers {
            if let LayerKind::Adjustment(Adjustment::ToneCurve { composite, .. }) = &mut layer.kind
            {
                *composite = points.clone();
                found = true;
            }
        }
        assert!(found);
        psd::write(&projected, &Limits::default()).unwrap()
    };
    // 隣が 15 刻みで、最後が 255 の点の並び（15 点 + 端 = 16 点、16 点 + 端 = 17 点）
    let spaced = |n: u8| -> Vec<[u8; 2]> {
        (0..n)
            .map(|i| [i * 15, i * 15])
            .chain([[255, 255]])
            .collect()
    };
    for (name, points) in [
        ("黒点を内側へ（入力 5）", vec![[5, 0], [255, 255]]),
        ("白点を内側へ（入力 250）", vec![[0, 0], [250, 255]]),
        ("隣が 4 刻み", vec![[0, 0], [4, 10], [255, 255]]),
        ("点が 17", spaced(16)),
    ] {
        let (mode, codes) = read_back(&with_composite(points));
        assert_eq!(mode, Mode::PreserveOnly, "{name}: {codes:?}");
        assert!(
            codes.iter().any(|c| c == "TaggedBlock"),
            "{name}: {codes:?}"
        );
    }
    // 表せる曲線（隣が 6 刻み・16 点）は編集できる層のまま
    for (name, points) in [
        ("隣が 6 刻み", vec![[0, 0], [6, 10], [255, 255]]),
        ("点が 16", spaced(15)),
    ] {
        let (mode, codes) = read_back(&with_composite(points));
        assert_eq!(mode, Mode::EditableRaster, "{name}: {codes:?}");
    }
}

#[test]
fn gradient_map_fields_the_tool_rewrites_are_reported_not_dropped_silently() {
    let grdm = written(&exact()[0].1);
    let (mode, codes) = read_back(&grdm);
    assert_eq!(mode, Mode::EditableRaster);
    assert!(
        !codes.iter().any(|c| c == "NotCarriedIntoExport"),
        "書き戻す値と同じなら知らせない: {codes:?}"
    );
    // 補間の欄（なめらかさ）の位置から、乱数 4・透明の表示 2・ベクトルの色 2・粗さ 4・色モデル 2・色の範囲 16・余白 2 バイトが続く
    let smooth = 10 + 3 * 20 + 2 + 3 * 10 + 2;
    for (name, at, with) in [
        ("乱数", smooth + 6, 7u32.to_be_bytes().to_vec()),
        ("透明の表示", smooth + 10, vec![0, 0]),
        ("粗さ", smooth + 14, 2048u32.to_be_bytes().to_vec()),
        ("色モデル", smooth + 18, vec![0, 1]),
        ("色の範囲", smooth + 30, vec![0, 1]),
        ("余白", smooth + 37, vec![1]),
    ] {
        let (mode, codes) = read_back(&patch(&grdm, b"grdm", at, &with));
        assert_eq!(mode, Mode::EditableRaster, "{name}: {codes:?}");
        assert!(
            codes.iter().any(|c| c == "NotCarriedIntoExport"),
            "{name}: {codes:?}"
        );
    }
}

#[test]
fn the_tool_and_psd_agree_on_every_composite_pixel() {
    // PSD の参照合成（core の式を表にしたもの）と core の合成が、統合画像も層ごとの合成も同じ（読みが CompositeDiffers を言わない）
    for (name, s) in exact() {
        let mut d = new_doc();
        let a = d
            .add_adjustment_layer(name, s.clone(), Some(&[Channel::Color]), None)
            .unwrap();
        d.set_layer_blend_mode(a, CoreBlend::Multiply).unwrap();
        d.set_layer_opacity(a, 0.5, false).unwrap();
        let projected = Psd::from_core(&d).unwrap();
        let bytes = psd::write(&projected, &Limits::default()).unwrap();
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        assert!(
            read.diagnostics()
                .iter()
                .all(|d| d.code != "CompositeDiffers"),
            "{name}: {:?}",
            read.diagnostics()
        );
    }
}

/// 書いた PSD を psd-tools で読み直すための出力（`YOLU_PSD_DUMP=<ディレクトリ>`。ふだんは何もしない）。
/// `python3 tools/io-fixtures/psd_tools_check.py <ディレクトリ>` が値を照らす。
#[test]
fn dump_psd_for_psd_tools() {
    let Ok(dir) = std::env::var("YOLU_PSD_DUMP") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for (i, (_, s)) in exact().into_iter().enumerate() {
        let d = adjustment_document(s);
        let bytes = psd::write(&Psd::from_core(&d).unwrap(), &Limits::default()).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join(format!("adjust{i}.psd")),
            bytes,
        )
        .unwrap();
    }
}
