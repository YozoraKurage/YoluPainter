//! core の文書への明示変換（C# の `PsdBridge.Export`・`Import` と対）。名前によるレイヤー対応付けはしない。
//!
//! - 書き出しは [`super::bake`]（焼き込み・チャンネルごと）と、ここの `from_core`（厳密: Color の写しで、そのままは書けないものは
//!   平らにも黙って落とすこともせず、レイヤーの名前と理由で断る。C# の `PsdBridge.Export` の断りと対）。PSD の形で書くのは、ラスター・
//!   グループ（入れ子・通過/分離）・単色の塗りつぶし・調整（反転・レベル補正・色相/彩度・色調補正の 6 種。PSD の刻みに収まるものだけ）・クリッピング・
//!   ラスターマスク（有効/無効・濃度）・レイヤーのロック。
//! - 取り込み（`to_core`）は逆で、並びも ID も C# の取り込みと同じ（PSD のレイヤー ID と区切りの ID を core のレイヤー ID の中に持ち、
//!   書き出し直すと同じ ID になる）。
use super::*;
use crate::{check, Error, Result};
use std::collections::HashSet;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, AdjustmentType, BrightnessContrast, Channel, ColorBalance,
    Document as CoreDocument, GradientMap, LayerId, LayerLocks, Posterize, Rgba8, Threshold,
    TileCoord, ToneChannel, ToneCurves,
};

/// PSD の画素の合計の予算（C# と同じ。マスクはキャンバス 1 枚ぶんを数える）。厳密な書き出し（`from_core`）と、予算を決めない焼き込みの書き出し（`ExportControl::default()`）の上限。
/// アプリの書き出しは、設定の「レイヤーのメモリ」から決まる予算（`ExportControl::source_budget`）で書く。
pub(super) const PIXEL_BUDGET: u64 = 128 * 1024 * 1024;

/// core のレイヤーのロック（1:透明部分・2:画素・4:位置・8:すべて）を PSD の lspf のビット（0:透明部分・1:画素・2:位置・31:すべて）へ。
/// すべては 0x80000000 だけにする（その下の個別のビットは足さない。効くロックは同じで、読み直した PSD も同じ形になる）。
pub(super) fn psd_locks(locks: LayerLocks) -> u32 {
    if locks.contains(LayerLocks::ALL) {
        return 0x8000_0000;
    }
    let mut bits = 0;
    for (lock, bit) in [
        (LayerLocks::TRANSPARENCY, 1),
        (LayerLocks::PIXELS, 2),
        (LayerLocks::POSITION, 4),
    ] {
        if locks.contains(lock) {
            bits |= bit;
        }
    }
    bits
}
/// `psd_locks` の逆。
pub(super) fn core_locks(bits: u32) -> LayerLocks {
    let mut locks = LayerLocks::NONE;
    for (bit, lock) in [
        (1, LayerLocks::TRANSPARENCY),
        (2, LayerLocks::PIXELS),
        (4, LayerLocks::POSITION),
        (0x8000_0000, LayerLocks::ALL),
    ] {
        if bits & bit != 0 {
            locks = locks | lock;
        }
    }
    locks
}
pub(super) fn channel_label(d: &CoreDocument, c: Channel) -> String {
    d.channel_info(c)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| format!("番号 {}", c.index()))
}
fn whole(v: f64) -> Option<i32> {
    let r = v.round_ties_even();
    ((v - r).abs() < 1e-9).then_some(r as i32)
}
/// core の調整を PSD の刻みの整数で。刻みの間は丸めず断る（C# の `AdjustmentRefusal` と同じ条件）。
pub(super) fn psd_adjustment(s: &AdjustmentSettings) -> std::result::Result<Adjustment, Refusal> {
    match s.kind() {
        AdjustmentType::Invert => Ok(Adjustment::Invert),
        AdjustmentType::Levels => {
            let steps = [
                whole(s.input_black() * 255.0),
                whole(s.input_white() * 255.0),
                whole(s.output_black() * 255.0),
                whole(s.output_white() * 255.0),
                whole(s.gamma() * 100.0),
            ];
            let [Some(ib), Some(iw), Some(ob), Some(ow), Some(g)] = steps else {
                return Err(Refusal::LevelsBetweenSteps);
            };
            if !(ib <= 253 && iw >= 2 && iw > ib) {
                return Err(Refusal::LevelsRange);
            }
            Ok(Adjustment::Levels {
                input_black: ib as u16,
                input_white: iw as u16,
                output_black: ob as u16,
                output_white: ow as u16,
                gamma: g as u16,
            })
        }
        AdjustmentType::HueSaturation => {
            let steps = [
                whole(s.hue()),
                whole(s.saturation() * 100.0),
                whole(s.lightness() * 100.0),
            ];
            let [Some(hue), Some(saturation), Some(lightness)] = steps else {
                return Err(Refusal::HueSaturationBetweenSteps);
            };
            Ok(Adjustment::HueSaturation {
                hue: hue as i16,
                saturation: saturation as i16,
                lightness: lightness as i16,
            })
        }
        AdjustmentType::GradientMap => psd_gradient_map(
            s.gradient_map_value()
                .expect("グラデーションマップは値を持つ"),
        ),
        AdjustmentType::ToneCurve => {
            let curves = s.tone_curve_value().expect("トーンカーブは値を持つ");
            let points = |channel: ToneChannel| -> std::result::Result<Vec<[u8; 2]>, Refusal> {
                curves
                    .curve(channel)
                    .points()
                    .iter()
                    .map(|p| match (whole(p.x * 255.0), whole(p.y * 255.0)) {
                        (Some(x), Some(y)) => Ok([x as u8, y as u8]),
                        _ => Err(Refusal::ToneCurveBetweenSteps),
                    })
                    .collect()
            };
            Ok(Adjustment::ToneCurve {
                composite: points(ToneChannel::Composite)?,
                red: points(ToneChannel::Red)?,
                green: points(ToneChannel::Green)?,
                blue: points(ToneChannel::Blue)?,
            })
        }
        AdjustmentType::ColorBalance => {
            let v = s.color_balance_value().expect("カラーバランスは値を持つ");
            let mut ranges = [[0i16; 3]; 3];
            for (range, which) in yolu_core::BalanceRange::ALL.into_iter().enumerate() {
                for (axis, value) in v.values(which).into_iter().enumerate() {
                    ranges[range][axis] =
                        whole(value).ok_or(Refusal::ColorBalanceBetweenSteps)? as i16;
                }
            }
            Ok(Adjustment::ColorBalance {
                shadows: ranges[0],
                midtones: ranges[1],
                highlights: ranges[2],
                preserve_luminosity: v.preserve_luminosity(),
            })
        }
        AdjustmentType::BrightnessContrast => {
            let v = s
                .brightness_contrast_value()
                .expect("明るさ・コントラストは値を持つ");
            let (Some(brightness), Some(contrast)) = (whole(v.brightness()), whole(v.contrast()))
            else {
                return Err(Refusal::BrightnessContrastBetweenSteps);
            };
            Ok(Adjustment::BrightnessContrast {
                brightness: brightness as i16,
                contrast: contrast as i16,
            })
        }
        AdjustmentType::Threshold => Ok(Adjustment::Threshold {
            level: s.threshold_value().expect("2 値化は値を持つ").level() as u16,
        }),
        AdjustmentType::Posterize => Ok(Adjustment::Posterize {
            levels: s
                .posterize_value()
                .expect("ポスタリゼーションは値を持つ")
                .levels() as u16,
        }),
    }
}
/// グラデーションマップを PSD の刻み（位置 0〜4096・中点 %・不透明度 0〜255）へ。値のカーブが直線でないもの・刻みの間は断る。
/// 最後の分岐点の中点は使われないので 50 で書く。
fn psd_gradient_map(g: &GradientMap) -> std::result::Result<Adjustment, Refusal> {
    let ramp = g.ramp();
    if !ramp.value_curve().is_identity() {
        return Err(Refusal::GradientMapCurve);
    }
    if ramp.uses_mixing() {
        return Err(Refusal::GradientMapMixing);
    }
    let step = |position: f64, midpoint: f64, last: bool| -> Option<(u16, u8)> {
        let location = whole(position * 4096.0)?;
        let mid = if last { 50 } else { whole(midpoint * 100.0)? };
        ((0..=4096).contains(&location) && (1..=99).contains(&mid))
            .then_some((location as u16, mid as u8))
    };
    let mut colors = Vec::new();
    for (i, c) in ramp.colors().iter().enumerate() {
        let (location, midpoint) = step(c.position, c.midpoint, i + 1 == ramp.colors().len())
            .ok_or(Refusal::GradientMapBetweenSteps)?;
        colors.push(GradientColorStop {
            location,
            midpoint,
            rgb: [c.color.r, c.color.g, c.color.b],
        });
    }
    let mut opacities = Vec::new();
    for (i, o) in ramp.opacities().iter().enumerate() {
        let (location, midpoint) = step(o.position, o.midpoint, i + 1 == ramp.opacities().len())
            .ok_or(Refusal::GradientMapBetweenSteps)?;
        let opacity = whole(o.opacity * 255.0).ok_or(Refusal::GradientMapBetweenSteps)?;
        opacities.push(GradientOpacityStop {
            location,
            midpoint,
            opacity: opacity as u8,
        });
    }
    Ok(Adjustment::GradientMap {
        reverse: g.reverse(),
        colors,
        opacities,
    })
}
/// PSD の調整を core の設定へ（C# の取り込みと同じ割り算。刻みの整数をそのまま写す）。
pub(super) fn core_adjustment(a: &Adjustment) -> Result<AdjustmentSettings> {
    let invalid = |e: yolu_core::CoreError| Error::InvalidData(e.to_string());
    Ok(match a {
        Adjustment::Invert => AdjustmentSettings::invert(),
        &Adjustment::Levels {
            input_black,
            input_white,
            output_black,
            output_white,
            gamma,
        } => AdjustmentSettings::levels(
            f64::from(input_black) / 255.0,
            f64::from(input_white) / 255.0,
            f64::from(gamma) / 100.0,
            f64::from(output_black) / 255.0,
            f64::from(output_white) / 255.0,
        )?,
        &Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => AdjustmentSettings::hue_saturation(
            f64::from(hue),
            f64::from(saturation) / 100.0,
            f64::from(lightness) / 100.0,
        )?,
        Adjustment::GradientMap {
            reverse,
            colors,
            opacities,
        } => {
            let ramp = Ramp::new(
                colors
                    .iter()
                    .map(|c| ColorStop {
                        position: f64::from(c.location) / 4096.0,
                        color: Rgba8::new(c.rgb[0], c.rgb[1], c.rgb[2], 255),
                        midpoint: f64::from(c.midpoint) / 100.0,
                    })
                    .collect(),
                opacities
                    .iter()
                    .map(|o| OpacityStop {
                        position: f64::from(o.location) / 4096.0,
                        opacity: f64::from(o.opacity) / 255.0,
                        midpoint: f64::from(o.midpoint) / 100.0,
                    })
                    .collect(),
                None,
            )
            .map_err(|e| Error::InvalidData(e.to_string()))?;
            AdjustmentSettings::gradient_map(GradientMap::new(ramp, *reverse))
        }
        Adjustment::ToneCurve {
            composite,
            red,
            green,
            blue,
        } => {
            let curve = |points: &Vec<[u8; 2]>| {
                Curve::new(
                    points
                        .iter()
                        .map(|p| CurvePoint {
                            x: f64::from(p[0]) / 255.0,
                            y: f64::from(p[1]) / 255.0,
                        })
                        .collect(),
                )
                .map_err(|e| Error::InvalidData(e.to_string()))
            };
            AdjustmentSettings::tone_curve(ToneCurves::new(
                curve(composite)?,
                curve(red)?,
                curve(green)?,
                curve(blue)?,
            ))
        }
        Adjustment::ColorBalance {
            shadows,
            midtones,
            highlights,
            preserve_luminosity,
        } => {
            let f = |v: &[i16; 3]| v.map(f64::from);
            AdjustmentSettings::color_balance(
                ColorBalance::new(f(shadows), f(midtones), f(highlights), *preserve_luminosity)
                    .map_err(invalid)?,
            )
        }
        &Adjustment::BrightnessContrast {
            brightness,
            contrast,
        } => AdjustmentSettings::brightness_contrast(
            BrightnessContrast::new(f64::from(brightness), f64::from(contrast)).map_err(invalid)?,
        ),
        &Adjustment::Threshold { level } => {
            AdjustmentSettings::threshold(Threshold::new(u32::from(level)).map_err(invalid)?)
        }
        &Adjustment::Posterize { levels } => {
            AdjustmentSettings::posterize(Posterize::new(u32::from(levels)).map_err(invalid)?)
        }
    })
}
/// 厳密に（焼かず・丸めず・落とさずに）PSD へ書けない理由（レイヤーごと）。`from_core` は 1 つでもあれば断る。焼き込みの書き出し（[`super::plan_export`]）は、
/// これらを機能ごとに焼く・丸める・落とすで書き、書けないものだけを断りとして返す。画面は種類から画面の言語の文を作る。`message` は日本語の
/// 診断（`from_core` の断りの文）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// クリッピングされたグループ（Photoshop が正しく扱うかを確かめられない）。
    ClippedGroup,
    /// 反転したマスク（PSD に非破壊の反転が無い）。
    InvertedMask,
    /// 半透明の塗りつぶし（PSD の単色の塗りつぶしは不透明だけ）。
    FillTranslucent,
    /// 塗りつぶしの画像・投影・デカール・グラデーション（PSD の単色の塗りつぶしに形が無い）。
    FillPixels,
    /// レベル補正が PSD の刻み（0〜255 の整数・ガンマは 1/100）の間にある。
    LevelsBetweenSteps,
    /// レベル補正の入力が PSD の範囲（黒 0〜253、白はその上の 2〜255）に収まらない。
    LevelsRange,
    /// 色相・彩度が PSD の刻み（1 度・1%）の間にある。
    HueSaturationBetweenSteps,
    /// グラデーションマップの位置・中点・不透明度が PSD の刻み（位置 1/4096・中点 1%・不透明度 1/255）の間にある。
    GradientMapBetweenSteps,
    /// グラデーションマップのランプに値のカーブがある（PSD のグラデーションに形が無い）。
    GradientMapCurve,
    /// グラデーションマップの値のカーブを、停止点の上限（32）の中で、レイヤーの出力の差 4 以下（通常の合成モード・不透明度 100%・下の色によらない最悪）に
    /// 展開できない。焼き込みの書き出しだけの断りで、合成に出るレイヤーにだけ付く（合成に出ないレイヤーは最良の展開を隠して書く）。厳密な書き出しは展開をしないので、
    /// 値のカーブがあれば収まる曲線も含めて `GradientMapCurve` で断る。
    GradientMapCurveStops,
    /// グラデーションマップのランプが混色（混色モードが通常でない）か区間の混合率曲線を使っている（PSD のグラデーションに形が無い）。
    GradientMapMixing,
    /// グラデーションマップの混色・混合率曲線を、停止点の上限（32）の中で、レイヤーの出力の差 4 以下に展開できない。`GradientMapCurveStops` と同じ決めで、
    /// 焼き込みの書き出しだけの断り（厳密な書き出しは混色があれば `GradientMapMixing` で断る）。
    GradientMapMixingStops,
    /// トーンカーブの点が PSD の刻み（入力・出力とも 1/255）の間にある。
    ToneCurveBetweenSteps,
    /// カラーバランスのスライダーが PSD の刻み（整数）の間にある。
    ColorBalanceBetweenSteps,
    /// 明るさ・コントラストが PSD の刻み（整数）の間にある。
    BrightnessContrastBetweenSteps,
    /// レイヤーかマスクのフィルター・Generator（効いていない段・無効の段も。設定が PSD に残らず、レイヤーの画素と統合画像が食い違う）。
    Effects,
    /// Anchor（PSD に形が無い）。
    Anchor,
    /// パス（PSD に形が無い）。
    Path,
    /// ファイルの向きが DirectX の Normal のレベル補正（緑だけ別の曲線になり、PSD の 1 つのレベル補正では書けない）。
    NormalLevels,
    /// テキストレイヤーの値（このアプリのテキストの値は PSD のテキストレイヤーの形に無い）。
    Text,
}
/// レイヤー 1 枚の断りの理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocker {
    pub layer: String,
    pub refusal: Refusal,
}
impl Blocker {
    /// 日本語の診断（レイヤーの名前つき）。
    pub fn message(&self) -> String {
        let name = &self.layer;
        match &self.refusal {
            Refusal::ClippedGroup => format!("グループ「{name}」はクリッピングされています。クリッピングされたフォルダーを Photoshop が正しく扱うかは確かめられていないので、PSD に書きません"),
            Refusal::InvertedMask => format!("レイヤー「{name}」のマスクは反転しています。PSD には非破壊のマスクの反転がないので、画素に焼かず断ります"),
            Refusal::FillTranslucent => format!("塗りつぶし「{name}」の Color は半透明です。PSD の単色の塗りつぶしは不透明だけです"),
            Refusal::FillPixels => format!("塗りつぶし「{name}」は画像・投影・デカール・グラデーションを使っています。PSD の単色の塗りつぶしに形が無いので、画素に焼かず断ります"),
            Refusal::LevelsBetweenSteps => format!("調整「{name}」のレベル補正は PSD の刻み（0〜255 の整数・ガンマは 1/100）の間にあります。丸めて書かず断ります"),
            Refusal::LevelsRange => format!("調整「{name}」のレベル補正は PSD に書けません。入力の黒は 0〜253、入力の白はそれより上の 2〜255 です"),
            Refusal::HueSaturationBetweenSteps => format!("調整「{name}」の色相・彩度は PSD の刻み（1 度・1%）の間にあります。丸めて書かず断ります"),
            Refusal::GradientMapBetweenSteps => format!("調整「{name}」のグラデーションマップは PSD の刻み（位置 1/4096・中点 1%・不透明度 1/255）の間にあります。丸めて書かず断ります"),
            Refusal::GradientMapCurve => format!("調整「{name}」のグラデーションマップのランプに値のカーブがあります。PSD のグラデーションに値のカーブはないので、書かず断ります"),
            Refusal::GradientMapCurveStops => format!("調整「{name}」のグラデーションマップの値のカーブを、PSD の停止点の上限（32 個）の中で、レイヤーの出力の差 4 以下（通常の合成・不透明度 100%）に展開できません。書かず断ります"),
            Refusal::GradientMapMixing => format!("調整「{name}」のグラデーションマップのランプに混色（混色モード・混合率曲線）があります。PSD のグラデーションに形はないので、書かず断ります"),
            Refusal::GradientMapMixingStops => format!("調整「{name}」のグラデーションマップの混色（混色モード・混合率曲線）を、PSD の停止点の上限（32 個）の中で、レイヤーの出力の差 4 以下（通常の合成・不透明度 100%）に展開できません。書かず断ります"),
            Refusal::ToneCurveBetweenSteps => format!("調整「{name}」のトーンカーブの点は PSD の刻み（入力・出力とも 0〜255 の整数）の間にあります。丸めて書かず断ります"),
            Refusal::ColorBalanceBetweenSteps => format!("調整「{name}」のカラーバランスは PSD の刻み（整数）の間にあります。丸めて書かず断ります"),
            Refusal::BrightnessContrastBetweenSteps => format!("調整「{name}」の明るさ・コントラストは PSD の刻み（整数）の間にあります。丸めて書かず断ります"),
            Refusal::Effects => format!("レイヤー「{name}」にフィルターかジェネレーターがあります。効果は PSD に書けません"),
            Refusal::Anchor => format!("レイヤー「{name}」に Anchor があります。Anchor は PSD に書けません"),
            Refusal::Path => format!("レイヤー「{name}」にパスがあります。パスは PSD に書けません"),
            Refusal::NormalLevels => format!("調整「{name}」のレベル補正は、ファイルの向きが DirectX の Normal の PSD に書けません。緑だけ別の曲線になります"),
            Refusal::Text => format!("「{name}」はテキストレイヤーです。テキストの値は PSD に書けません"),
        }
    }
}

/// PSD 側の ID（正の整数）。core のレイヤー ID の上位 32 bit（C# の Guid の先頭 4 バイト）から取り、重なれば次の空きへ。
pub(super) fn unique_id(raw: i32, used: &mut HashSet<i32>) -> i32 {
    let mut id = raw & i32::MAX;
    if id == 0 {
        id = 1
    }
    while !used.insert(id) {
        id = if id == i32::MAX { 1 } else { id + 1 }
    }
    id
}
pub(super) fn layer_part(id: LayerId) -> i32 {
    (id.0 >> 96) as i32
}
/// グループの区切りの ID（C# の Guid の 4〜7 バイト目。core のレイヤー ID では 64〜95 bit 目の 2 つの 16 bit）。
pub(super) fn divider_part(id: LayerId) -> i32 {
    ((((id.0 >> 80) & 0xffff) as u32) | ((((id.0 >> 64) & 0xffff) as u32) << 16)) as i32
}
/// `divider_part` の逆（取り込みで区切りの ID を core のレイヤー ID に持つ）。
pub(super) fn divider_bits(divider: i32) -> u128 {
    let d = divider as u32;
    (u128::from(d & 0xffff) << 80) | (u128::from(d >> 16) << 64)
}

impl ReadResult {
    pub fn to_core(&self) -> Result<CoreDocument> {
        check(
            self.mode == CompatibilityMode::EditableRaster,
            "PSD 原本は編集できません",
        )?;
        self.document
            .as_ref()
            .ok_or_else(|| Error::InvalidData("編集用のプロジェクトがありません".into()))?
            .to_core()
    }
}
/// レイヤーの並び（下から上）。グループの中身はグループのすぐ下に続き、親はグループの並びの位置。
struct Item<'a> {
    layer: &'a Layer,
    parent: Option<usize>,
}
fn count(l: &Layer) -> usize {
    match &l.kind {
        LayerKind::Group { children, .. } => 1 + children.iter().map(count).sum::<usize>(),
        _ => 1,
    }
}
fn order<'a>(top_down: &'a [Layer], parent: Option<usize>, out: &mut Vec<Item<'a>>) {
    for l in top_down.iter().rev() {
        if let LayerKind::Group { children, .. } = &l.kind {
            let at = out.len() + children.iter().map(count).sum::<usize>();
            order(children, Some(at), out);
        }
        out.push(Item { layer: l, parent })
    }
}
pub(super) fn mask_off_canvas(m: &Mask, width: u32, height: u32) -> bool {
    let (w, h) = (i64::from(width), i64::from(height));
    (0..m.height).any(|y| {
        (0..m.width).any(|x| {
            let cx = i64::from(m.left) + i64::from(x);
            let cy = i64::from(m.top) + i64::from(y);
            (cx < 0 || cy < 0 || cx >= w || cy >= h)
                && m.pixels[y as usize * m.width as usize + x as usize] != m.default_color
        })
    })
}
impl Document {
    /// core に入れられない内容を、レイヤーごとに 1 つ（`layers[2].children[0]: …` の形）返す。キャンバス外の画素・マスクを切り捨てて黙って
    /// 捨てることはしない。グループ・調整・塗りつぶし・マスク・ロックは core が持てる。
    pub fn core_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        self.collect_core_issues(&self.layers, "layers", &mut issues);
        issues
    }
    fn collect_core_issues(&self, layers: &[Layer], path: &str, issues: &mut Vec<String>) {
        for (i, l) in layers.iter().enumerate() {
            let p = format!("{path}[{i}]");
            match &l.kind {
                LayerKind::Group { children, .. } => {
                    self.collect_core_issues(children, &format!("{p}.children"), issues)
                }
                LayerKind::Raster
                    if l.left < 0
                        || l.top < 0
                        || i64::from(l.left) + i64::from(l.width) > i64::from(self.width)
                        || i64::from(l.top) + i64::from(l.height) > i64::from(self.height) =>
                {
                    issues.push(format!("{p}: キャンバス外の画素を切り捨てられません"))
                }
                _ => {}
            }
            if l.mask
                .as_ref()
                .is_some_and(|m| mask_off_canvas(m, self.width, self.height))
            {
                issues.push(format!(
                    "{p}.mask: キャンバス外にあるマスクの値を切り捨てられません"
                ))
            }
        }
    }
    /// ラスターレイヤーの画素を core の Color の面へ（PSD は上から下、core は下から上）。
    fn import_pixels(&self, d: &mut CoreDocument, id: LayerId, l: &Layer) -> Result<()> {
        let ts = d.tile_size();
        let x0 = l.left as u32;
        let y0 = self.height - l.top as u32 - l.height;
        let x1 = x0 + l.width;
        let y1 = y0 + l.height;
        for ty in y0 / ts..y1.div_ceil(ts) {
            for tx in x0 / ts..x1.div_ceil(ts) {
                let mut tile = vec![0; (ts * ts * 4) as usize];
                for y in (ty * ts).max(y0)..((ty + 1) * ts).min(y1) {
                    for x in (tx * ts).max(x0)..((tx + 1) * ts).min(x1) {
                        let src =
                            ((self.height - 1 - y - l.top as u32) * l.width + x - x0) as usize * 4;
                        let dest = ((y % ts) * ts + x % ts) as usize * 4;
                        tile[dest..dest + 4].copy_from_slice(&l.pixels_rgba[src..src + 4])
                    }
                }
                d.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &tile)?;
            }
        }
        Ok(())
    }
    /// ラスターレイヤーの画素を core の Color の面へ。`import_pixels` と違い、キャンバスからはみ出すレイヤーは、はみ出した所を入れずに切る
    /// （アルファが 0 でない画素を切ったら true を返す。透明の画素は、切っても何も失わない）。レイヤーの画素が空のレイヤーは何もしない。
    pub(super) fn import_pixels_clipped(
        &self,
        d: &mut CoreDocument,
        id: LayerId,
        l: &Layer,
    ) -> Result<bool> {
        if l.width == 0 || l.height == 0 {
            return Ok(false);
        }
        let (cw, ch) = (i64::from(self.width), i64::from(self.height));
        let (left, top) = (i64::from(l.left), i64::from(l.top));
        let (right, bottom) = (left + i64::from(l.width), top + i64::from(l.height));
        let (x0, x1) = (left.max(0), right.min(cw));
        let (y0, y1) = (top.max(0), bottom.min(ch));
        let mut clipped = false;
        if x0 != left || x1 != right || y0 != top || y1 != bottom {
            for y in 0..i64::from(l.height) {
                let row_outside = top + y < y0 || top + y >= y1;
                for x in 0..i64::from(l.width) {
                    if (row_outside || left + x < x0 || left + x >= x1)
                        && l.pixels_rgba[(y as usize * l.width as usize + x as usize) * 4 + 3] != 0
                    {
                        clipped = true;
                        break;
                    }
                }
                if clipped {
                    break;
                }
            }
        }
        if x0 >= x1 || y0 >= y1 {
            return Ok(clipped);
        }
        let ts = d.tile_size();
        // core の行は下から。PSD の行 y0..y1 は core の行 (高さ − y1)..(高さ − y0)
        let (cx0, cx1) = (x0 as u32, x1 as u32);
        let (cy0, cy1) = ((ch - y1) as u32, (ch - y0) as u32);
        let coords: Vec<TileCoord> = (cy0 / ts..cy1.div_ceil(ts))
            .flat_map(|ty| (cx0 / ts..cx1.div_ceil(ts)).map(move |tx| TileCoord::new(tx, ty)))
            .collect();
        // タイルの画素を行ごとに写す（タイルはワーカーが作り、core へはタイルの順に入れる）
        d.import_tiles_with(id, Channel::Color, &coords, |c, tile| {
            tile.fill(0);
            let (xa, xb) = ((c.x * ts).max(cx0), ((c.x + 1) * ts).min(cx1));
            let n = (xb - xa) as usize * 4;
            for y in (c.y * ts).max(cy0)..((c.y + 1) * ts).min(cy1) {
                let row = (ch - 1 - i64::from(y) - top) as usize;
                let src = (row * l.width as usize + (i64::from(xa) - left) as usize) * 4;
                let dest = ((y % ts) * ts + xa % ts) as usize * 4;
                tile[dest..dest + n].copy_from_slice(&l.pixels_rgba[src..src + n]);
            }
        })?;
        Ok(clipped)
    }
    /// マスクを core へ。core は隠す量（255 − PSD の値）を持ち、何も隠さないタイルは持たないので、隠す所のあるタイルだけを入れる。
    pub(super) fn import_mask(&self, d: &mut CoreDocument, id: LayerId, m: &Mask) -> Result<()> {
        d.add_layer_mask(id)?;
        let (w, h, ts) = (self.width, self.height, d.tile_size());
        let mut tile = vec![0u8; (ts * ts * 4) as usize];
        for ty in 0..h.div_ceil(ts) {
            for tx in 0..w.div_ceil(ts) {
                let (x0, y0) = (tx * ts, ty * ts);
                // 既定の値が 255（隠さない）なら、矩形に触れないタイルは何も隠さない
                if m.default_color == 255 {
                    let (top, bottom) = (h - (y0 + ts).min(h), h - y0);
                    let outside = i64::from(m.left) >= i64::from((x0 + ts).min(w))
                        || i64::from(m.left) + i64::from(m.width) <= i64::from(x0)
                        || i64::from(m.top) >= i64::from(bottom)
                        || i64::from(m.top) + i64::from(m.height) <= i64::from(top);
                    if outside {
                        continue;
                    }
                }
                tile.fill(0);
                let mut hides = false;
                for row in 0..ts.min(h - y0) {
                    let psd_y = i64::from(h - 1 - (y0 + row));
                    for col in 0..ts.min(w - x0) {
                        let hide = 255 - m.at(i64::from(x0 + col), psd_y);
                        if hide != 0 {
                            tile[((row * ts + col) * 4 + 3) as usize] = hide;
                            hides = true
                        }
                    }
                }
                if hides {
                    d.import_mask_tile(id, TileCoord::new(tx, ty), &tile)?;
                }
            }
        }
        d.set_layer_mask_enabled(id, m.enabled)?;
        d.set_layer_mask_density(id, f64::from(m.density) / 255.0, false)?;
        Ok(())
    }
    pub fn to_core(&self) -> Result<CoreDocument> {
        super::write::validate(self, &Limits::default())?;
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("core 変換を拒否しました: {}", issues.join("、")),
        )?;
        let mut d = CoreDocument::new(self.width, self.height)?;
        let salt = d.id() & ((1u128 << 64) - 1);
        let mut items = Vec::new();
        order(&self.layers, None, &mut items);
        let mut made: Vec<LayerId> = Vec::with_capacity(items.len());
        for item in &items {
            let l = item.layer;
            let id = match &l.kind {
                LayerKind::Raster => {
                    let id = d.add_layer(&l.name)?;
                    self.import_pixels(&mut d, id, l)?;
                    id
                }
                LayerKind::Group { .. } => d.add_group(&l.name, None)?,
                LayerKind::SolidColor([r, g, b]) => d.add_fill_layer(
                    &l.name,
                    &[(Channel::Color, Rgba8::new(*r, *g, *b, 255))],
                    None,
                )?,
                LayerKind::Adjustment(a) => {
                    d.add_adjustment_layer(&l.name, core_adjustment(a)?, None, None)?
                }
            };
            d.set_layer_visible(id, l.visible)?;
            d.set_layer_opacity(id, f64::from(l.opacity) / 255.0, false)?;
            d.set_layer_blend_mode(
                id,
                yolu_core::BlendMode::from_index(l.blend_mode as u8).unwrap(),
            )?;
            d.set_layer_clipping(id, l.clipping)?;
            if let Some(m) = &l.mask {
                self.import_mask(&mut d, id, m)?;
            }
            made.push(id);
        }
        let parents: Vec<Option<LayerId>> =
            items.iter().map(|i| i.parent.map(|p| made[p])).collect();
        if parents.iter().any(Option::is_some) {
            d.set_structure_for_load(&parents)?;
        }
        // ロックは最後に付ける（取り込みの設定をロックが断らないように。履歴には残らない）
        for (item, id) in items.iter().zip(&made) {
            if item.layer.locks != 0 {
                d.set_locks_for_load(*id, core_locks(item.layer.locks))?;
            }
        }
        // レイヤー ID に PSD のレイヤー ID（上位 32 bit）とグループの区切りの ID を持たせ、書き出し直すと同じ ID になる。残りは文書ごとに違う
        let ids: Vec<LayerId> = items
            .iter()
            .map(|i| {
                let divider = match i.layer.kind {
                    LayerKind::Group { divider_id, .. } => divider_bits(divider_id),
                    _ => 0,
                };
                LayerId(((i.layer.id as u128) << 96) | divider | salt)
            })
            .collect();
        let doc_id = d.id();
        Ok(d.with_persistent_ids(doc_id, &ids)?)
    }

    /// 厳密な新規投影（Color）。そのままは書けないもの（焼く・丸める・落とすが要るもの）が 1 つでもあれば、レイヤーの名前と理由で断る。
    /// 焼き込みで書くには [`export_core`](super::export_core)。インポート原本の編集保存には、この結果と `write_edited` を使う。
    pub fn from_core(d: &CoreDocument) -> Result<Self> {
        Self::from_core_with(d, &ExportControl::default())
    }

    /// [`from_core`](Self::from_core) の、上限の値を呼び手が決める形（`ctl.source_budget` は設定の「レイヤーのメモリ」。レイヤーの記録の数・キャンバス・
    /// 全レイヤーの画素の合計がこの予算から決まる）。何を断るかは `from_core` と同じ。
    pub fn from_core_with(d: &CoreDocument, ctl: &ExportControl) -> Result<Self> {
        super::bake::from_core_strict(d, ctl)
    }
}
