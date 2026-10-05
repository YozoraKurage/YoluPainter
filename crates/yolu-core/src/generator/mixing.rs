//! ランプの隣り合う色の分岐点の間の混ぜ方（混色モード）と、知覚的な混色の輝度の補正。
//!
//! CLIP STUDIO PAINT のグラデーションの混色モードの考え方（通常 = 昔の混ぜ方、知覚的 = 鮮やかで絵の具に近い混ぜ方、リニア）と、
//! 輝度の補正 5 段階（知覚的のときだけ。補正が強いほど混ぜた色が明るくなる）に合わせた**このアプリの式**で、CLIP STUDIO や Photoshop の
//! 同名のモードと画素まで一致するとは言わない（あちらの式は公開されていない）。
//!
//! - [`MixMode::Standard`]: sRGB の 8 bit の値をそのまま線形に補間する。これまでの（版 24 までの）混ぜ方で、PSD のグラデーションと同じ。
//! - [`MixMode::Linear`]: sRGB を線形の光へ戻して補間し、sRGB へ戻す（暗い色どうしを混ぜても沈まない）。
//! - [`MixMode::Perceptual`]: 線形の光から Oklab（Björn Ottosson）へ移して L・a・b を補間し、戻す。色相が濁りにくく、補色どうしの
//!   途中が灰色へ沈む量は Oklab の a・b の補間が決める。補間した色が sRGB の外へ出たら、各チャンネルを 0〜1 に収める（彩度を残す
//!   ガマットの写像はしない）。
//!
//! 輝度の補正は、知覚的の混色で「両端の彩度の混ぜ合わせ」より実際の彩度が落ちた量（ΔC = (1−t)·C0 + t·C1 − C）に比例して L を持ち上げる:
//! `L' = min(1, L + k·ΔC)`、k は段階ごとに 0・0.25・0.5・0.75・1。補色どうしの真ん中のように彩度が落ちる所ほど明るくなり、端（t = 0 と 1）と、
//! 彩度が落ちない混ぜ（同じ色相どうし）では何も変わらない。
use crate::math::clamp01;
use crate::Rgba8;

/// 色の混ぜ方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MixMode {
    /// sRGB の値を線形に補間する（版 24 までの混ぜ方）。
    #[default]
    Standard,
    /// 線形の光で補間する。
    Linear,
    /// Oklab で補間する（輝度の補正つき）。
    Perceptual,
}

impl MixMode {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Perceptual, Self::Linear];

    /// 保存の値（0 通常・1 知覚的・2 リニア）。
    pub fn index(self) -> u8 {
        match self {
            Self::Standard => 0,
            Self::Perceptual => 1,
            Self::Linear => 2,
        }
    }

    pub fn from_index(index: i64) -> Option<Self> {
        match index {
            0 => Some(Self::Standard),
            1 => Some(Self::Perceptual),
            2 => Some(Self::Linear),
            _ => None,
        }
    }
}

/// 輝度の補正の強さ（5 段階。知覚的な混色のときだけ働く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LuminanceCorrection {
    None,
    Low,
    Medium,
    High,
    Max,
}

impl Default for LuminanceCorrection {
    /// CLIP STUDIO の知覚的の既定に合わせて「高」。
    fn default() -> Self {
        Self::High
    }
}

impl LuminanceCorrection {
    pub const ALL: [Self; 5] = [Self::None, Self::Low, Self::Medium, Self::High, Self::Max];

    /// 保存の値（0〜4）。
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(index: i64) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// 彩度の落ちた量に掛ける係数（0・0.25・0.5・0.75・1）。
    pub fn strength(self) -> f64 {
        f64::from(self.index()) * 0.25
    }
}

fn to_linear(byte: f64) -> f64 {
    let c = byte / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(linear: f64) -> f64 {
    let c = clamp01(linear);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    v * 255.0
}

/// 線形の sRGB から Oklab（L, a, b）。
pub(crate) fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Oklab から線形の sRGB（範囲の外も出る）。
pub(crate) fn oklab_to_linear([lightness, a, b]: [f64; 3]) -> [f64; 3] {
    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// 色 `a`（重み 0）から `b`（重み 1）へ、重み `t` の所の色（R・G・B を 0〜255 の実数で。丸めは呼び手）。
/// `Standard` は昔のままの sRGB の値の線形補間。両端（t ≤ 0・t ≥ 1）は、どの混ぜ方でも端の色そのもの。
pub(crate) fn mix(
    a: Rgba8,
    b: Rgba8,
    t: f64,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [f64; 3] {
    let pick = |c: Rgba8| [f64::from(c.r), f64::from(c.g), f64::from(c.b)];
    let (ca, cb) = (pick(a), pick(b));
    if t <= 0.0 {
        return ca;
    }
    if t >= 1.0 {
        return cb;
    }
    match mode {
        MixMode::Standard => std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * t),
        MixMode::Linear => {
            let (la, lb) = (ca.map(to_linear), cb.map(to_linear));
            std::array::from_fn(|i| to_srgb(la[i] + (lb[i] - la[i]) * t))
        }
        MixMode::Perceptual => {
            let oa = linear_to_oklab(ca.map(to_linear));
            let ob = linear_to_oklab(cb.map(to_linear));
            let mut m: [f64; 3] = std::array::from_fn(|i| oa[i] + (ob[i] - oa[i]) * t);
            let k = correction.strength();
            if k > 0.0 {
                let chroma = |o: [f64; 3]| o[1].hypot(o[2]);
                let lost = (1.0 - t) * chroma(oa) + t * chroma(ob) - chroma(m);
                m[0] = (m[0] + k * lost.max(0.0)).min(1.0);
            }
            oklab_to_linear(m).map(to_srgb)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, g: u8, b: u8) -> Rgba8 {
        Rgba8::new(r, g, b, 255)
    }

    #[test]
    fn oklab_round_trips_and_has_the_known_anchors() {
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.2, 0.5, 0.8],
        ] {
            let back = oklab_to_linear(linear_to_oklab(rgb));
            for i in 0..3 {
                assert!((back[i] - rgb[i]).abs() < 1e-6, "{rgb:?} {back:?}");
            }
        }
        // 白は L = 1・a = b = 0、黒は 0
        let white = linear_to_oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-4 && white[1].abs() < 1e-4 && white[2].abs() < 1e-4);
        assert!(linear_to_oklab([0.0; 3]).iter().all(|v| v.abs() < 1e-9));
        // 赤は Oklab の公開の値（L 0.628、a 0.2249、b 0.1258）
        let red = linear_to_oklab([1.0, 0.0, 0.0]);
        assert!((red[0] - 0.6280).abs() < 1e-3, "{red:?}");
        assert!((red[1] - 0.2249).abs() < 1e-3, "{red:?}");
        assert!((red[2] - 0.1258).abs() < 1e-3, "{red:?}");
    }

    #[test]
    fn every_mode_returns_the_end_colours_exactly() {
        let (a, b) = (c(12, 200, 99), c(250, 3, 180));
        for mode in MixMode::ALL {
            for correction in LuminanceCorrection::ALL {
                assert_eq!(mix(a, b, 0.0, mode, correction), [12.0, 200.0, 99.0]);
                assert_eq!(mix(a, b, 1.0, mode, correction), [250.0, 3.0, 180.0]);
            }
        }
    }

    #[test]
    fn standard_is_the_old_straight_srgb_interpolation() {
        let m = mix(
            c(0, 100, 255),
            c(100, 200, 55),
            0.25,
            MixMode::Standard,
            LuminanceCorrection::High,
        );
        assert_eq!(m, [25.0, 125.0, 205.0]);
    }

    #[test]
    fn linear_light_mixing_is_brighter_than_srgb_mixing_for_black_and_white() {
        let m = mix(
            c(0, 0, 0),
            c(255, 255, 255),
            0.5,
            MixMode::Linear,
            LuminanceCorrection::None,
        );
        // 線形の 0.5 は sRGB で約 188
        assert!((m[0] - 188.0).abs() < 1.0, "{m:?}");
        assert!(m[0] > 127.5);
    }

    #[test]
    fn perceptual_mixing_keeps_a_grey_ramp_grey_and_lighter_correction_lightens_complements() {
        // 灰色どうしは a = b = 0 のまま（色が付かない）
        let g = mix(
            c(20, 20, 20),
            c(230, 230, 230),
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        );
        assert!(
            (g[0] - g[1]).abs() < 0.5 && (g[1] - g[2]).abs() < 0.5,
            "{g:?}"
        );
        // 補色どうし（青と黄）の真ん中は、補正が強いほど明るい
        let (blue, yellow) = (c(20, 40, 220), c(250, 230, 30));
        let lum = |m: [f64; 3]| 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2];
        let mut last = -1.0;
        for level in LuminanceCorrection::ALL {
            let m = mix(blue, yellow, 0.5, MixMode::Perceptual, level);
            let l = lum(m);
            assert!(l >= last, "{level:?}: {l} < {last}");
            last = l;
        }
        let none = lum(mix(
            blue,
            yellow,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        ));
        let max = lum(mix(
            blue,
            yellow,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::Max,
        ));
        assert!(max > none + 5.0, "{none} {max}");
        // 同じ色相どうし（彩度が落ちない）では、補正しても変わらない
        let (dark, light) = (c(40, 0, 0), c(255, 120, 120));
        let a = mix(
            dark,
            light,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        );
        let b = mix(
            dark,
            light,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::Max,
        );
        assert!((0..3).all(|i| (a[i] - b[i]).abs() < 12.0), "{a:?} {b:?}");
    }

    #[test]
    fn out_of_gamut_results_are_clamped_to_the_byte_range() {
        for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
            for mode in MixMode::ALL {
                let m = mix(
                    c(0, 255, 255),
                    c(255, 0, 255),
                    t,
                    mode,
                    LuminanceCorrection::Max,
                );
                assert!(
                    m.iter().all(|v| (0.0..=255.0).contains(v)),
                    "{mode:?} {t}: {m:?}"
                );
            }
        }
    }

    #[test]
    fn saved_indices_round_trip_and_unknown_ones_are_refused() {
        for mode in MixMode::ALL {
            assert_eq!(MixMode::from_index(i64::from(mode.index())), Some(mode));
        }
        assert_eq!(MixMode::from_index(3), None);
        assert_eq!(MixMode::from_index(-1), None);
        for level in LuminanceCorrection::ALL {
            assert_eq!(
                LuminanceCorrection::from_index(i64::from(level.index())),
                Some(level)
            );
        }
        assert_eq!(LuminanceCorrection::from_index(5), None);
        assert_eq!(LuminanceCorrection::default(), LuminanceCorrection::High);
    }
}
