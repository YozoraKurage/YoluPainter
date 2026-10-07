//! 調整レイヤーの式（C# の Adjustments.cs、AlgorithmVersion 1）。画素ごとの点の調整で、保存したままの（符号化した）RGB に
//! このツールの定義の式を当てる。Photoshop・CLIP STUDIO の同じ名前の調整と一致するとは言わない。アルファは変えない。
//!
//! 種類 0〜2（反転・レベル補正・色相/彩度）は C# と同じ式で、反転・レベル補正は C# と全バイト一致。色相/彩度は同じ式を f32 で
//! 計算する（`rows`。f64 の式とは 1 段ずれる値がある）。64 からは Rust 版だけの種類（グラデーションマップ・トーンカーブ・
//! カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション。値と式は [`ops`]）で、Unity 版は読めない。

mod ops;
mod rows;

pub use ops::{
    luminance, BalanceRange, BrightnessContrast, ColorBalance, GradientMap, Posterize, Threshold,
    ToneChannel, ToneCurves,
};
pub(crate) use rows::color_balance_rgba;

use crate::blend::mix_rgb;
use crate::error::CoreError;
use crate::math::{require_finite, to_byte, UNIT};
use crate::ranges;
use crate::types::{BlendMode, ChannelKind, Rgba8};
use std::sync::Arc;

/// 調整の種類（値は保存形式に入る）。0〜2 は C# と共有。Rust 版だけの種類は 64 から（手続き型の Generator の 64・65 と同じ流儀）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum AdjustmentType {
    Invert = 0,
    Levels = 1,
    HueSaturation = 2,
    GradientMap = 64,
    ToneCurve = 65,
    ColorBalance = 66,
    BrightnessContrast = 67,
    Threshold = 68,
    Posterize = 69,
}

impl AdjustmentType {
    /// 保存形式の番号から。知らない番号は None。
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::Invert,
            1 => Self::Levels,
            2 => Self::HueSaturation,
            64 => Self::GradientMap,
            65 => Self::ToneCurve,
            66 => Self::ColorBalance,
            67 => Self::BrightnessContrast,
            68 => Self::Threshold,
            69 => Self::Posterize,
            _ => return None,
        })
    }
    /// Rust 版だけの種類か（C# の 0〜2 でない。Unity 版は読めない）。
    pub fn is_rust_only(self) -> bool {
        (self as u8) >= 64
    }
}

/// Rust 版だけの色調補正 6 種のどれか 1 つの値（調整レイヤーとフィルターの段で共通の入れ物。保存・画面が種類ごとの分岐を 1 か所で持てる）。
#[derive(Clone, Debug, PartialEq)]
pub enum ColorAdjust {
    GradientMap(GradientMap),
    ToneCurve(ToneCurves),
    ColorBalance(ColorBalance),
    BrightnessContrast(BrightnessContrast),
    Threshold(Threshold),
    Posterize(Posterize),
}

impl ColorAdjust {
    /// 新しく足すときの既定の値（グラデーションマップは黒から白・トーンカーブは直線・カラーバランスは何も動かさない・
    /// 明るさ/コントラストは 0・2 値化は 128・ポスタリゼーションは 4）。64 より前の種類は None。
    pub fn default_for(kind: AdjustmentType) -> Option<Self> {
        Some(match kind {
            AdjustmentType::GradientMap => {
                Self::GradientMap(GradientMap::new(crate::generator::Ramp::default(), false))
            }
            AdjustmentType::ToneCurve => Self::ToneCurve(ToneCurves::identity()),
            AdjustmentType::ColorBalance => Self::ColorBalance(ColorBalance::neutral()),
            AdjustmentType::BrightnessContrast => Self::BrightnessContrast(
                BrightnessContrast::new(0.0, 0.0).expect("0 と 0 は範囲内"),
            ),
            AdjustmentType::Threshold => {
                Self::Threshold(Threshold::new(128).expect("128 は範囲内"))
            }
            AdjustmentType::Posterize => Self::Posterize(Posterize::new(4).expect("4 は範囲内")),
            AdjustmentType::Invert | AdjustmentType::Levels | AdjustmentType::HueSaturation => {
                return None
            }
        })
    }
    pub fn kind(&self) -> AdjustmentType {
        match self {
            Self::GradientMap(_) => AdjustmentType::GradientMap,
            Self::ToneCurve(_) => AdjustmentType::ToneCurve,
            Self::ColorBalance(_) => AdjustmentType::ColorBalance,
            Self::BrightnessContrast(_) => AdjustmentType::BrightnessContrast,
            Self::Threshold(_) => AdjustmentType::Threshold,
            Self::Posterize(_) => AdjustmentType::Posterize,
        }
    }
    /// そのチャンネルの種類に使えるか（`AdjustmentSettings::applies_to` と同じ）。
    pub fn applies_to(&self, kind: ChannelKind) -> bool {
        self.clone().into_settings().applies_to(kind)
    }
    /// 調整レイヤーの設定にする。
    pub fn into_settings(self) -> AdjustmentSettings {
        match self {
            Self::GradientMap(v) => AdjustmentSettings::gradient_map(v),
            Self::ToneCurve(v) => AdjustmentSettings::tone_curve(v),
            Self::ColorBalance(v) => AdjustmentSettings::color_balance(v),
            Self::BrightnessContrast(v) => AdjustmentSettings::brightness_contrast(v),
            Self::Threshold(v) => AdjustmentSettings::threshold(v),
            Self::Posterize(v) => AdjustmentSettings::posterize(v),
        }
    }
    /// 履歴に積む大きさ。
    pub fn byte_size(&self) -> u64 {
        match self {
            Self::GradientMap(v) => v.byte_size(),
            Self::ToneCurve(v) => v.byte_size(),
            Self::ColorBalance(v) => v.byte_size(),
            Self::BrightnessContrast(v) => v.byte_size(),
            Self::Threshold(v) => v.byte_size(),
            Self::Posterize(v) => v.byte_size(),
        }
    }
}

/// 64 からの種類の値（種類ごとに 1 つ。重い中身は `Arc` の表を持つので複製は安い）。
#[derive(Clone, PartialEq, Debug)]
enum More {
    None,
    GradientMap(Arc<GradientMap>),
    ToneCurve(Arc<ToneCurves>),
    ColorBalance(ColorBalance),
    BrightnessContrast(BrightnessContrast),
    Threshold(Threshold),
    Posterize(Posterize),
}

/// 調整の値（変えない値。作るときに検査する）。
#[derive(Clone, PartialEq, Debug)]
pub struct AdjustmentSettings {
    kind: AdjustmentType,
    input_black: f64,
    input_white: f64,
    gamma: f64,
    output_black: f64,
    output_white: f64,
    hue: f64,
    saturation: f64,
    lightness: f64,
    more: More,
}

impl AdjustmentSettings {
    /// 式の版（保存形式に入る）。
    pub const ALGORITHM_VERSION: i32 = 1;

    fn base(kind: AdjustmentType) -> Self {
        AdjustmentSettings {
            kind,
            input_black: 0.0,
            input_white: 1.0,
            gamma: 1.0,
            output_black: 0.0,
            output_white: 1.0,
            hue: 0.0,
            saturation: 0.0,
            lightness: 0.0,
            more: More::None,
        }
    }
    fn with_more(kind: AdjustmentType, more: More) -> Self {
        AdjustmentSettings {
            more,
            ..Self::base(kind)
        }
    }
    /// 反転（255 − 値）。
    pub fn invert() -> Self {
        Self::base(AdjustmentType::Invert)
    }
    /// レベル補正: 入力の範囲（0〜1、幅は 1/255 以上）・ガンマ（0.1〜9.99）・出力の範囲（0〜1）。
    pub fn levels(
        input_black: f64,
        input_white: f64,
        gamma: f64,
        output_black: f64,
        output_white: f64,
    ) -> Result<Self, CoreError> {
        let s = AdjustmentSettings {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
            ..Self::base(AdjustmentType::Levels)
        };
        s.validate()?;
        Ok(s)
    }
    /// 色相（−180〜180 度）・彩度（−1〜1）・明度（−1〜1）。Color の種類のチャンネルにだけ使える。
    pub fn hue_saturation(hue: f64, saturation: f64, lightness: f64) -> Result<Self, CoreError> {
        let s = AdjustmentSettings {
            hue,
            saturation,
            lightness,
            ..Self::base(AdjustmentType::HueSaturation)
        };
        s.validate()?;
        Ok(s)
    }
    /// 保存形式の値から（読み込み用。値は検査する）。
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        kind: AdjustmentType,
        input_black: f64,
        input_white: f64,
        gamma: f64,
        output_black: f64,
        output_white: f64,
        hue: f64,
        saturation: f64,
        lightness: f64,
    ) -> Result<Self, CoreError> {
        if kind.is_rust_only() {
            return Err(CoreError::InvalidArgument(
                "この種類は 8 つの値では組み立てられない（種類ごとの組み立てを使う）",
            ));
        }
        let s = AdjustmentSettings {
            kind,
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
            hue,
            saturation,
            lightness,
            more: More::None,
        };
        s.validate()?;
        Ok(s)
    }
    /// グラデーションマップ（Color のチャンネルだけ）。
    pub fn gradient_map(map: GradientMap) -> Self {
        Self::with_more(
            AdjustmentType::GradientMap,
            More::GradientMap(Arc::new(map)),
        )
    }
    /// トーンカーブ（Color と Scalar のチャンネル。Scalar では RGB 全体の曲線だけ）。
    pub fn tone_curve(curves: ToneCurves) -> Self {
        Self::with_more(AdjustmentType::ToneCurve, More::ToneCurve(Arc::new(curves)))
    }
    /// カラーバランス（Color のチャンネルだけ）。
    pub fn color_balance(balance: ColorBalance) -> Self {
        Self::with_more(AdjustmentType::ColorBalance, More::ColorBalance(balance))
    }
    /// 明るさ/コントラスト（Color と Scalar のチャンネル）。
    pub fn brightness_contrast(value: BrightnessContrast) -> Self {
        Self::with_more(
            AdjustmentType::BrightnessContrast,
            More::BrightnessContrast(value),
        )
    }
    /// 2 値化（Color と Scalar のチャンネル）。
    pub fn threshold(value: Threshold) -> Self {
        Self::with_more(AdjustmentType::Threshold, More::Threshold(value))
    }
    /// ポスタリゼーション（Color と Scalar のチャンネル）。
    pub fn posterize(value: Posterize) -> Self {
        Self::with_more(AdjustmentType::Posterize, More::Posterize(value))
    }

    pub fn kind(&self) -> AdjustmentType {
        self.kind
    }
    pub fn input_black(&self) -> f64 {
        self.input_black
    }
    pub fn input_white(&self) -> f64 {
        self.input_white
    }
    pub fn gamma(&self) -> f64 {
        self.gamma
    }
    pub fn output_black(&self) -> f64 {
        self.output_black
    }
    pub fn output_white(&self) -> f64 {
        self.output_white
    }
    pub fn hue(&self) -> f64 {
        self.hue
    }
    pub fn saturation(&self) -> f64 {
        self.saturation
    }
    pub fn lightness(&self) -> f64 {
        self.lightness
    }
    pub fn gradient_map_value(&self) -> Option<&GradientMap> {
        match &self.more {
            More::GradientMap(v) => Some(v.as_ref()),
            _ => None,
        }
    }
    pub fn tone_curve_value(&self) -> Option<&ToneCurves> {
        match &self.more {
            More::ToneCurve(v) => Some(v.as_ref()),
            _ => None,
        }
    }
    pub fn color_balance_value(&self) -> Option<&ColorBalance> {
        match &self.more {
            More::ColorBalance(v) => Some(v),
            _ => None,
        }
    }
    pub fn brightness_contrast_value(&self) -> Option<&BrightnessContrast> {
        match &self.more {
            More::BrightnessContrast(v) => Some(v),
            _ => None,
        }
    }
    pub fn threshold_value(&self) -> Option<&Threshold> {
        match &self.more {
            More::Threshold(v) => Some(v),
            _ => None,
        }
    }
    pub fn posterize_value(&self) -> Option<&Posterize> {
        match &self.more {
            More::Posterize(v) => Some(v),
            _ => None,
        }
    }
    /// 64 からの種類なら、その値（調整レイヤーの設定から取り出す）。反転・レベル補正・色相/彩度は None。
    pub fn color_adjust(&self) -> Option<ColorAdjust> {
        Some(match &self.more {
            More::None => return None,
            More::GradientMap(v) => ColorAdjust::GradientMap(v.as_ref().clone()),
            More::ToneCurve(v) => ColorAdjust::ToneCurve(v.as_ref().clone()),
            More::ColorBalance(v) => ColorAdjust::ColorBalance(*v),
            More::BrightnessContrast(v) => ColorAdjust::BrightnessContrast(v.clone()),
            More::Threshold(v) => ColorAdjust::Threshold(*v),
            More::Posterize(v) => ColorAdjust::Posterize(*v),
        })
    }
    /// 履歴に積む大きさ（固定の欄 64 と、種類ごとの中身）。反転・レベル補正・色相/彩度は中身が無いので 64 で、前の値と後の値の和（編集 1 回の見積り）は
    /// 旧来の 128 のまま。ランプ・曲線・表を持つ種類は、その分だけ大きい。
    pub fn byte_size(&self) -> u64 {
        64 + match &self.more {
            More::None => 0,
            More::GradientMap(v) => v.byte_size(),
            More::ToneCurve(v) => v.byte_size(),
            More::ColorBalance(v) => v.byte_size(),
            More::BrightnessContrast(v) => v.byte_size(),
            More::Threshold(v) => v.byte_size(),
            More::Posterize(v) => v.byte_size(),
        }
    }

    /// C# の Validate と同じ検査。
    pub fn validate(&self) -> Result<(), CoreError> {
        for v in [
            self.input_black,
            self.input_white,
            self.gamma,
            self.output_black,
            self.output_white,
            self.hue,
            self.saturation,
            self.lightness,
        ] {
            require_finite(v, "adjustment")?;
        }
        if self.input_black < *ranges::LEVELS_UNIT.start()
            || self.input_white > *ranges::LEVELS_UNIT.end()
            || self.input_white - self.input_black < 1.0 / 255.0
        {
            return Err(CoreError::InvalidArgument("レベル補正の入力の範囲"));
        }
        if !ranges::LEVELS_UNIT.contains(&self.output_black)
            || !ranges::LEVELS_UNIT.contains(&self.output_white)
        {
            return Err(CoreError::InvalidArgument("レベル補正の出力の範囲"));
        }
        if !ranges::GAMMA.contains(&self.gamma) {
            return Err(CoreError::InvalidArgument("ガンマ（0.1〜9.99）"));
        }
        if !ranges::HUE.contains(&self.hue)
            || !ranges::SATURATION.contains(&self.saturation)
            || !ranges::SATURATION.contains(&self.lightness)
        {
            return Err(CoreError::InvalidArgument("色相・彩度・明度"));
        }
        let matches_kind = matches!(
            (self.kind, &self.more),
            (
                AdjustmentType::Invert | AdjustmentType::Levels | AdjustmentType::HueSaturation,
                More::None
            ) | (AdjustmentType::GradientMap, More::GradientMap(_))
                | (AdjustmentType::ToneCurve, More::ToneCurve(_))
                | (AdjustmentType::ColorBalance, More::ColorBalance(_))
                | (
                    AdjustmentType::BrightnessContrast,
                    More::BrightnessContrast(_)
                )
                | (AdjustmentType::Threshold, More::Threshold(_))
                | (AdjustmentType::Posterize, More::Posterize(_))
        );
        if !matches_kind {
            return Err(CoreError::InvalidArgument("調整の種類と値が合わない"));
        }
        Ok(())
    }

    /// そのチャンネルに使えるか。色相/彩度・グラデーションマップ・カラーバランスは色のチャンネルだけ（スカラーのデータや法線に当てると
    /// 値を壊す）。トーンカーブ・明るさ/コントラスト・2 値化・ポスタリゼーションは法線に当てない（単位ベクトルを壊す）。
    pub fn applies_to(&self, kind: ChannelKind) -> bool {
        match self.kind {
            AdjustmentType::Invert | AdjustmentType::Levels => true,
            AdjustmentType::HueSaturation
            | AdjustmentType::GradientMap
            | AdjustmentType::ColorBalance => kind == ChannelKind::Color,
            AdjustmentType::ToneCurve
            | AdjustmentType::BrightnessContrast
            | AdjustmentType::Threshold
            | AdjustmentType::Posterize => kind != ChannelKind::Normal,
        }
    }

    /// 調整した色（アルファはそのまま）。色のチャンネルでの結果（`apply_in(ChannelKind::Color, c)`）。
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        self.apply_in(ChannelKind::Color, c)
    }

    /// そのチャンネルの種類での調整結果。トーンカーブだけ種類で変わる（スカラーは RGB 全体の曲線だけ）。
    pub fn apply_in(&self, channel: ChannelKind, c: Rgba8) -> Rgba8 {
        match self.kind {
            AdjustmentType::Invert => Rgba8::new(255 - c.r, 255 - c.g, 255 - c.b, c.a),
            AdjustmentType::Levels => {
                Rgba8::new(self.level(c.r), self.level(c.g), self.level(c.b), c.a)
            }
            AdjustmentType::HueSaturation => {
                rows::hue_saturation_pixel(self.hue, self.saturation, self.lightness, c)
            }
            _ => match &self.more {
                More::GradientMap(v) => v.apply(c),
                More::ToneCurve(v) if channel == ChannelKind::Scalar => v.apply_scalar(c),
                More::ToneCurve(v) => v.apply(c),
                More::ColorBalance(v) => v.apply(c),
                More::BrightnessContrast(v) => v.apply(c),
                More::Threshold(v) => v.apply(c),
                More::Posterize(v) => v.apply(c),
                More::None => c,
            },
        }
    }

    fn level(&self, component: u8) -> u8 {
        let mut v =
            (UNIT[component as usize] - self.input_black) / (self.input_white - self.input_black);
        v = max(0.0, min(1.0, v));
        v = v.powf(1.0 / self.gamma);
        to_byte(self.output_black + v * (self.output_white - self.output_black))
    }

    /// 調整レイヤーの合成の 1 段: 調整した色を下の色とモードで組み合わせ、量（不透明度 × マスク）で戻す。アルファは下のまま
    /// （調整は半透明の画素を濃くしない）。完全に透明な画素はそのまま。色のチャンネルでの結果。
    #[inline]
    pub fn composite(&self, below: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
        self.composite_in(ChannelKind::Color, below, amount, mode)
    }

    /// `composite` の、チャンネルの種類を指定する版（トーンカーブがスカラーで変わる）。
    #[inline]
    pub fn composite_in(
        &self,
        channel: ChannelKind,
        below: Rgba8,
        amount: f64,
        mode: BlendMode,
    ) -> Rgba8 {
        if amount <= 0.0 || below.a == 0 {
            return below;
        }
        mix_rgb(below, self.apply_in(channel, below), amount, mode)
    }

    /// 1 行（RGBA）に調整レイヤーを重ねる（各画素は `composite_in` と同じバイト。行の途中の量は `amount` が持つ）。表を使う種類は呼ぶたびに
    /// 表を作るので、細かく刻まず行や塊でまとめて呼ぶ。
    pub fn composite_row(
        &self,
        channel: ChannelKind,
        row: &mut [u8],
        amount: crate::blend::RowAmount<'_>,
        mode: BlendMode,
    ) {
        self.kernel(channel).composite_row(row, amount, mode);
    }

    /// 合成のための速い形（レベル補正の 256 の表を 1 回だけ作る）。
    pub(crate) fn kernel(&self, channel: ChannelKind) -> AdjustKernel {
        // チャンネルごとに値だけで決まる種類は 256 の表にする（表の値は式そのもの）
        let table = if self.kind == AdjustmentType::Levels {
            let mut t = [0u8; 256];
            for (i, v) in t.iter_mut().enumerate() {
                *v = self.level(i as u8);
            }
            Some(Box::new(t))
        } else if let More::Posterize(p) = &self.more {
            Some(Box::new(p.table()))
        } else {
            None
        };
        AdjustKernel {
            settings: self.clone(),
            channel,
            table,
        }
    }
}

/// C# の Math.Max / Math.Min（NaN の来ない値だけを比べる）。
#[inline(always)]
fn max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}
#[inline(always)]
fn min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

/// 合成で使う調整（レベル補正・ポスタリゼーションは表を引く。表の値は式そのもの）。
pub(crate) struct AdjustKernel {
    settings: AdjustmentSettings,
    channel: ChannelKind,
    table: Option<Box<[u8; 256]>>,
}

impl AdjustKernel {
    /// 1 画素の式。行の核（`composite_row`）はこれと同じバイトを出す（試験が比べる基準）。
    #[inline]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn composite(&self, below: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
        if amount <= 0.0 || below.a == 0 {
            return below;
        }
        let adjusted = match &self.table {
            Some(t) => Rgba8::new(
                t[below.r as usize],
                t[below.g as usize],
                t[below.b as usize],
                below.a,
            ),
            None => self.settings.apply_in(self.channel, below),
        };
        mix_rgb(below, adjusted, amount, mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::Ramp;

    #[test]
    fn formulas_produce_the_documented_values() {
        // C# AdjustmentTests.FormulasProduceTheDocumentedValues
        assert_eq!(
            AdjustmentSettings::invert().apply(Rgba8::new(10, 20, 30, 128)),
            Rgba8::new(245, 235, 225, 128)
        );
        let levels = AdjustmentSettings::levels(0.2, 0.8, 2.0, 0.0, 1.0).unwrap();
        assert_eq!(
            levels.apply(Rgba8::new(127, 0, 255, 77)),
            Rgba8::new(180, 0, 255, 77)
        );
        assert_eq!(
            AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.2, 0.6)
                .unwrap()
                .apply(Rgba8::new(255, 0, 128, 255)),
            Rgba8::new(153, 51, 102, 255)
        );
        let hs = |h, s, l| AdjustmentSettings::hue_saturation(h, s, l).unwrap();
        assert_eq!(
            hs(120.0, 0.0, 0.0).apply(Rgba8::new(255, 0, 0, 9)),
            Rgba8::new(0, 255, 0, 9)
        );
        assert_eq!(
            hs(0.0, -1.0, 0.0).apply(Rgba8::new(255, 0, 0, 255)),
            Rgba8::new(128, 128, 128, 255)
        );
        assert_eq!(
            hs(0.0, 0.0, 1.0).apply(Rgba8::new(40, 90, 200, 255)),
            Rgba8::new(255, 255, 255, 255)
        );
        assert_eq!(
            hs(0.0, 0.0, -1.0).apply(Rgba8::new(40, 90, 200, 255)),
            Rgba8::new(0, 0, 0, 255)
        );
        assert!(AdjustmentSettings::levels(0.5, 0.5, 1.0, 0.0, 1.0).is_err());
        assert!(AdjustmentSettings::hue_saturation(181.0, 0.0, 0.0).is_err());
        assert!(AdjustmentSettings::levels(0.0, 1.0, f64::NAN, 0.0, 1.0).is_err());
    }

    #[test]
    fn the_levels_table_is_the_formula() {
        let s = AdjustmentSettings::levels(0.1, 0.9, 1.7, 0.05, 0.95).unwrap();
        let k = s.kernel(ChannelKind::Color);
        for v in 0..=255u8 {
            let c = Rgba8::new(v, 255 - v, v / 2, 200);
            for mode in [BlendMode::Normal, BlendMode::Screen, BlendMode::Hue] {
                assert_eq!(k.composite(c, 0.7, mode), s.composite(c, 0.7, mode));
            }
        }
    }

    fn every_new_kind() -> Vec<AdjustmentSettings> {
        vec![
            AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), true)),
            AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(
                    ToneChannel::Composite,
                    crate::curve::Curve::new(vec![
                        crate::curve::CurvePoint { x: 0., y: 0.2 },
                        crate::curve::CurvePoint { x: 1., y: 0.8 },
                    ])
                    .unwrap(),
                ),
            ),
            AdjustmentSettings::color_balance(
                ColorBalance::new([10.0, 0.0, 0.0], [0.0, 20.0, 0.0], [0.0, 0.0, -30.0], true)
                    .unwrap(),
            ),
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(40.0, 20.0).unwrap()),
            AdjustmentSettings::threshold(Threshold::new(100).unwrap()),
            AdjustmentSettings::posterize(Posterize::new(5).unwrap()),
        ]
    }

    #[test]
    fn the_new_kinds_take_numbers_from_64_and_the_old_ones_keep_theirs() {
        let kinds: Vec<AdjustmentType> = every_new_kind().iter().map(|s| s.kind()).collect();
        assert_eq!(
            kinds.iter().map(|k| *k as u8).collect::<Vec<_>>(),
            vec![64, 65, 66, 67, 68, 69]
        );
        for (i, k) in [
            (0, AdjustmentType::Invert),
            (1, AdjustmentType::Levels),
            (2, AdjustmentType::HueSaturation),
        ] {
            assert_eq!(AdjustmentType::from_index(i), Some(k));
            assert_eq!(k as u8, i as u8);
            assert!(!k.is_rust_only());
        }
        for k in kinds {
            assert_eq!(AdjustmentType::from_index(k as i64), Some(k));
            assert!(k.is_rust_only());
        }
        for unknown in [3, 63, 70, 255, -1] {
            assert_eq!(AdjustmentType::from_index(unknown), None, "{unknown}");
        }
        // 8 つの値からは、新しい種類は組み立てられない
        assert!(AdjustmentSettings::from_parts(
            AdjustmentType::Threshold,
            0.0,
            1.0,
            1.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0
        )
        .is_err());
    }

    #[test]
    fn the_new_kinds_validate_and_keep_what_they_were_made_with() {
        for s in every_new_kind() {
            s.validate().unwrap();
            assert_eq!(s, s.clone());
        }
        let all = every_new_kind();
        assert!(all[0].gradient_map_value().unwrap().reverse());
        assert!(all[1].tone_curve_value().is_some() && all[0].tone_curve_value().is_none());
        assert_eq!(
            all[2]
                .color_balance_value()
                .unwrap()
                .values(BalanceRange::Midtones),
            [0.0, 20.0, 0.0]
        );
        assert_eq!(all[3].brightness_contrast_value().unwrap().contrast(), 20.0);
        assert_eq!(all[4].threshold_value().unwrap().level(), 100);
        assert_eq!(all[5].posterize_value().unwrap().levels(), 5);
        // 種類が違えば等しくない（同じ値でも）
        assert_ne!(all[4], all[5]);
        assert_ne!(
            all[4],
            AdjustmentSettings::threshold(Threshold::new(101).unwrap())
        );
    }

    #[test]
    fn the_new_kinds_never_touch_normal_and_colour_only_ones_skip_scalars() {
        let all = every_new_kind();
        let applies = |s: &AdjustmentSettings| {
            (
                s.applies_to(ChannelKind::Color),
                s.applies_to(ChannelKind::Scalar),
                s.applies_to(ChannelKind::Normal),
            )
        };
        assert_eq!(
            applies(&all[0]),
            (true, false, false),
            "グラデーションマップ"
        );
        assert_eq!(applies(&all[1]), (true, true, false), "トーンカーブ");
        assert_eq!(applies(&all[2]), (true, false, false), "カラーバランス");
        assert_eq!(
            applies(&all[3]),
            (true, true, false),
            "明るさ・コントラスト"
        );
        assert_eq!(applies(&all[4]), (true, true, false), "2 値化");
        assert_eq!(applies(&all[5]), (true, true, false), "ポスタリゼーション");
        // 今の 3 種は今のまま（反転・レベル補正は全部、色相/彩度は色だけ）
        assert_eq!(applies(&AdjustmentSettings::invert()), (true, true, true));
        assert_eq!(
            applies(&AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).unwrap()),
            (true, true, true)
        );
        assert_eq!(
            applies(&AdjustmentSettings::hue_saturation(0.0, 0.0, 0.0).unwrap()),
            (true, false, false)
        );
    }

    #[test]
    fn the_kernel_gives_the_same_pixels_as_the_settings_for_every_kind_and_channel() {
        let mut all = every_new_kind();
        all.push(AdjustmentSettings::invert());
        for s in &all {
            for kind in [ChannelKind::Color, ChannelKind::Scalar] {
                if !s.applies_to(kind) {
                    continue;
                }
                let k = s.kernel(kind);
                for v in (0..=255u8).step_by(5) {
                    let c = Rgba8::new(v, 255 - v, v / 3, 180);
                    for mode in [BlendMode::Normal, BlendMode::Multiply] {
                        assert_eq!(
                            k.composite(c, 0.6, mode),
                            s.composite_in(kind, c, 0.6, mode),
                            "{:?} {kind:?} {v}",
                            s.kind()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_tone_curve_uses_only_the_composite_curve_on_scalars() {
        let invert = crate::curve::Curve::new(vec![
            crate::curve::CurvePoint { x: 0., y: 1. },
            crate::curve::CurvePoint { x: 1., y: 0. },
        ])
        .unwrap();
        let s = AdjustmentSettings::tone_curve(
            ToneCurves::identity().with_curve(ToneChannel::Red, invert),
        );
        let grey = Rgba8::new(40, 40, 40, 255);
        assert_eq!(s.apply_in(ChannelKind::Scalar, grey), grey);
        assert_eq!(
            s.apply_in(ChannelKind::Color, grey),
            Rgba8::new(215, 40, 40, 255)
        );
        assert_eq!(s.apply(grey), s.apply_in(ChannelKind::Color, grey));
    }

    #[test]
    fn history_size_grows_with_what_a_kind_carries() {
        let base = AdjustmentSettings::invert().byte_size();
        assert_eq!(
            base, 64,
            "旧 3 種は固定の欄だけ。前の値と後の値の和は旧来の 128（文書の試験で固定）"
        );
        for s in every_new_kind() {
            assert!(s.byte_size() >= 96, "{:?}", s.kind());
        }
        let gm = AdjustmentSettings::gradient_map(GradientMap::new(Ramp::default(), false));
        assert!(gm.byte_size() > base);
    }

    #[test]
    fn only_colour_channels_take_hue_saturation() {
        let hs = AdjustmentSettings::hue_saturation(30.0, 0.0, 0.0).unwrap();
        assert!(hs.applies_to(ChannelKind::Color));
        assert!(!hs.applies_to(ChannelKind::Scalar));
        assert!(!hs.applies_to(ChannelKind::Normal));
        assert!(AdjustmentSettings::invert().applies_to(ChannelKind::Scalar));
    }
}
