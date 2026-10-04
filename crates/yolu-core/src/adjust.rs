//! 調整レイヤーの式（C# の Adjustments.cs、AlgorithmVersion 1）。画素ごとの点の調整で、保存したままの（符号化した）RGB に
//! この道具の定義の式を当てる。Photoshop・CLIP STUDIO の同じ名前の調整と一致するとは言わない。アルファは変えない。

use crate::blend::mix_rgb;
use crate::error::CoreError;
use crate::math::{require_finite, to_byte, UNIT};
use crate::types::{BlendMode, ChannelKind, Rgba8};

/// 調整の種類（値は保存形式に入る）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum AdjustmentType {
    Invert = 0,
    Levels = 1,
    HueSaturation = 2,
}

/// 調整の値（変えない値。作るときに検査する）。
#[derive(Clone, Copy, PartialEq, Debug)]
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
        };
        s.validate()?;
        Ok(s)
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
        if self.input_black < 0.0
            || self.input_white > 1.0
            || self.input_white - self.input_black < 1.0 / 255.0
        {
            return Err(CoreError::InvalidArgument("レベル補正の入力の範囲"));
        }
        if self.output_black < 0.0
            || self.output_black > 1.0
            || self.output_white < 0.0
            || self.output_white > 1.0
        {
            return Err(CoreError::InvalidArgument("レベル補正の出力の範囲"));
        }
        if self.gamma < 0.1 || self.gamma > 9.99 {
            return Err(CoreError::InvalidArgument("ガンマ（0.1〜9.99）"));
        }
        if self.hue < -180.0
            || self.hue > 180.0
            || self.saturation < -1.0
            || self.saturation > 1.0
            || self.lightness < -1.0
            || self.lightness > 1.0
        {
            return Err(CoreError::InvalidArgument("色相・彩度・明度"));
        }
        Ok(())
    }

    /// そのチャンネルに使えるか。色相/彩度は色のチャンネルだけ（スカラーのデータや法線に当てると値を壊す）。
    pub fn applies_to(&self, kind: ChannelKind) -> bool {
        self.kind != AdjustmentType::HueSaturation || kind == ChannelKind::Color
    }

    /// 調整した色（アルファはそのまま）。
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        match self.kind {
            AdjustmentType::Invert => Rgba8::new(255 - c.r, 255 - c.g, 255 - c.b, c.a),
            AdjustmentType::Levels => {
                Rgba8::new(self.level(c.r), self.level(c.g), self.level(c.b), c.a)
            }
            AdjustmentType::HueSaturation => {
                let (r, g, b) = self.hue_saturation_lightness(
                    UNIT[c.r as usize],
                    UNIT[c.g as usize],
                    UNIT[c.b as usize],
                );
                Rgba8::new(to_byte(r), to_byte(g), to_byte(b), c.a)
            }
        }
    }

    fn level(&self, component: u8) -> u8 {
        let mut v =
            (UNIT[component as usize] - self.input_black) / (self.input_white - self.input_black);
        v = max(0.0, min(1.0, v));
        v = v.powf(1.0 / self.gamma);
        to_byte(self.output_black + v * (self.output_white - self.output_black))
    }

    fn hue_saturation_lightness(&self, r: f64, g: f64, b: f64) -> (f64, f64, f64) {
        // RGB → HSL
        let mx = max(r, max(g, b));
        let mn = min(r, min(g, b));
        let mut l = (mx + mn) / 2.0;
        let mut h = 0.0;
        let mut s = 0.0;
        let d = mx - mn;
        if d > 1e-12 {
            s = if l > 0.5 {
                d / (2.0 - mx - mn)
            } else {
                d / (mx + mn)
            };
            h = if mx == r {
                (g - b) / d + if g < b { 6.0 } else { 0.0 }
            } else if mx == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
            h /= 6.0;
        }
        h += self.hue / 360.0;
        h -= h.floor();
        s = max(0.0, min(1.0, s * (1.0 + self.saturation)));
        l = if self.lightness >= 0.0 {
            l + (1.0 - l) * self.lightness
        } else {
            l * (1.0 + self.lightness)
        };
        // HSL → RGB
        if s <= 0.0 {
            return (l, l, l);
        }
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue_to_rgb(p, q, h + 1.0 / 3.0),
            hue_to_rgb(p, q, h),
            hue_to_rgb(p, q, h - 1.0 / 3.0),
        )
    }

    /// 調整レイヤーの合成の 1 段: 調整した色を下の色とモードで組み合わせ、量（不透明度 × マスク）で戻す。アルファは下のまま
    /// （調整は半透明の画素を濃くしない）。完全に透明な画素はそのまま。
    #[inline]
    pub fn composite(&self, below: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
        if amount <= 0.0 || below.a == 0 {
            return below;
        }
        mix_rgb(below, self.apply(below), amount, mode)
    }

    /// 合成のための速い形（レベル補正の 256 の表を 1 回だけ作る）。
    pub(crate) fn kernel(&self) -> AdjustKernel {
        let table = if self.kind == AdjustmentType::Levels {
            let mut t = [0u8; 256];
            for (i, v) in t.iter_mut().enumerate() {
                *v = self.level(i as u8);
            }
            Some(Box::new(t))
        } else {
            None
        };
        AdjustKernel {
            settings: *self,
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

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// 合成で使う調整（レベル補正は表を引く。表の値は `level` そのもの）。
pub(crate) struct AdjustKernel {
    settings: AdjustmentSettings,
    table: Option<Box<[u8; 256]>>,
}

impl AdjustKernel {
    #[inline]
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
            None => self.settings.apply(below),
        };
        mix_rgb(below, adjusted, amount, mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let k = s.kernel();
        for v in 0..=255u8 {
            let c = Rgba8::new(v, 255 - v, v / 2, 200);
            for mode in [BlendMode::Normal, BlendMode::Screen, BlendMode::Hue] {
                assert_eq!(k.composite(c, 0.7, mode), s.composite(c, 0.7, mode));
            }
        }
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
