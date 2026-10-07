//! 色調補正の追加の 6 種（グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション）の値と式。
//!
//! **Rust 版だけの種類**（.ylp の種類の番号は 64 から。C# の 0〜2 とは重ならず、Unity 版は読めない）。調整レイヤー（[`super::AdjustmentSettings`]）と
//! フィルターの段（`filter::Settings`）が同じ型を使うので、式を二重に持たない。値は作るときに検査し、作った後は変えない。
//!
//! 式はこのツールの定義で、Photoshop・CLIP STUDIO の同じ名前の調整と一致するとは言わない。画素ごとの点の処理で、符号化したままの
//! （ガンマをかけたままの）RGB のバイトに当て、アルファは変えない。どれも決定的で、スレッド数・タイルの区切りで結果は変わらない。
//! 重い計算（ランプ・曲線・べき乗）は作るときに 256 の表へ引き、画素ごとの処理は表を引くだけの整数の計算にする。
//!
//! - **輝度**（[`luminance`]）: Rec.709 の重み（0.2126・0.7152・0.0722）を符号化したままの RGB のバイトに整数で当てた 0〜255。灰色（R=G=B）は値のまま
//!   返る（スカラーのチャンネルで 2 値化・グラデーションマップの入力が値そのものになる）。Rec.709 にしたのは、ランプの「データのチャンネルを輝度の灰色で
//!   見せる」計算（`generator::Ramp` の `scalar`）と同じ重みで、アプリの中で輝度が 2 通りにならないため。
//! - **グラデーションマップ**: 輝度 → ランプ（値のカーブ・色・不透明度）。「逆向き」は輝度を 255 − 輝度にする。ランプの不透明度は元の色へ戻す割合
//!   （結果 = 元の色 × (1 − 不透明度) + ランプの色 × 不透明度）。
//! - **トーンカーブ**: 各チャンネルの曲線を先に、RGB 全体の曲線を後に当てる（`合成(チャンネル(値))`）。スカラーのチャンネルは RGB 全体の曲線だけ。
//! - **カラーバランス**: 画素の輝度 y で 3 つの範囲の重み（シャドウ (1 − y)²・中間 4y(1 − y)・ハイライト y²）を決め、範囲ごとの
//!   シアン/レッド・マゼンタ/グリーン・イエロー/ブルー（−100〜100）の重ね合わせ × 0.3 を R・G・B に足す。「輝度を保つ」は足した後の輝度を元の輝度へ
//!   同じ量だけ戻す（0〜1 へ収める前に戻すので、収めて飽和した分は戻らない）。
//! - **明るさ/コントラスト**（明るさ −150〜150・コントラスト −50〜100、Photoshop の幅。旧式は持たない）: まずコントラスト（0.5 を中心に傾き 2^(コントラスト/50) で
//!   伸ばし、0〜1 に収める）、次に明るさ（べき乗 v^(2^(−明るさ/75))。0 と 1 は動かさず、中間を持ち上げる・沈める）。
//! - **2 値化**（しきい値 1〜255）: 輝度 ≥ しきい値なら白、そうでなければ黒。
//! - **ポスタリゼーション**（階調 2〜255）: チャンネルごとに 0〜255 を n 段へ分け（段 = min(floor(v × n / 255), n − 1)）、段の値を 0〜255 に均等に置く。

use crate::curve::Curve;
use crate::error::CoreError;
use crate::generator::Ramp;
use crate::math::{clamp01, require_finite, to_byte, UNIT};
use crate::ranges;
use crate::types::Rgba8;
use std::sync::Arc;

/// 輝度（0〜255）。Rec.709 の重みを整数（2126・7152・722 / 10000、四捨五入）で当てる。灰色（R=G=B=v）は v。
#[inline]
pub fn luminance(c: Rgba8) -> u8 {
    ((2126 * u32::from(c.r) + 7152 * u32::from(c.g) + 722 * u32::from(c.b) + 5000) / 10000) as u8
}

// ───────── グラデーションマップ ─────────

/// グラデーションマップ（輝度 → ランプ）。作るとき 256 の輝度ぶんの表（ランプの値のカーブと色・不透明度を引いた結果）を作る。
#[derive(Clone, Debug)]
pub struct GradientMap {
    ramp: Ramp,
    reverse: bool,
    /// 輝度 → ランプの色（R, G, B）と不透明度（A）。
    table: Arc<[[u8; 4]; 256]>,
}

impl PartialEq for GradientMap {
    fn eq(&self, other: &Self) -> bool {
        self.ramp == other.ramp && self.reverse == other.reverse
    }
}

impl GradientMap {
    pub fn new(ramp: Ramp, reverse: bool) -> Self {
        let mut table = [[0u8; 4]; 256];
        for (l, e) in table.iter_mut().enumerate() {
            let t = if reverse { UNIT[255 - l] } else { UNIT[l] };
            let c = ramp
                .evaluate(t, false)
                .expect("輝度は有限なのでランプの評価は失敗しない");
            *e = [c.r, c.g, c.b, c.a];
        }
        Self {
            ramp,
            reverse,
            table: Arc::new(table),
        }
    }
    pub fn ramp(&self) -> &Ramp {
        &self.ramp
    }
    pub fn reverse(&self) -> bool {
        self.reverse
    }
    /// 調整した色（アルファはそのまま）。ランプの不透明度で元の色へ戻す。
    #[inline]
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        let [r, g, b, a] = self.table[luminance(c) as usize];
        let (a, inv) = (u32::from(a), 255 - u32::from(a));
        let mix = |orig: u8, mapped: u8| {
            ((u32::from(orig) * inv + u32::from(mapped) * a + 127) / 255) as u8
        };
        Rgba8::new(mix(c.r, r), mix(c.g, g), mix(c.b, b), c.a)
    }
    /// 履歴に積む大きさ（ランプ・表・本体）。
    pub fn byte_size(&self) -> u64 {
        32 + 1024 + self.ramp.byte_size()
    }
}

// ───────── トーンカーブ ─────────

/// トーンカーブの曲線の選び（RGB 全体と R・G・B）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToneChannel {
    Composite,
    Red,
    Green,
    Blue,
}

impl ToneChannel {
    pub const ALL: [ToneChannel; 4] = [Self::Composite, Self::Red, Self::Green, Self::Blue];
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
struct ToneTables {
    /// RGB 全体の曲線だけ（スカラーのチャンネル用）。
    composite: [u8; 256],
    /// チャンネルの曲線の後に RGB 全体の曲線を当てた表。
    red: [u8; 256],
    green: [u8; 256],
    blue: [u8; 256],
}

fn curve_table(curve: &Curve) -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = to_byte(curve.value_unchecked(UNIT[i]));
    }
    t
}

/// トーンカーブ（RGB 全体と R・G・B の 4 本の曲線）。
#[derive(Clone, Debug)]
pub struct ToneCurves {
    curves: [Curve; 4],
    tables: Arc<ToneTables>,
}

impl PartialEq for ToneCurves {
    fn eq(&self, other: &Self) -> bool {
        self.curves == other.curves
    }
}

impl Default for ToneCurves {
    fn default() -> Self {
        Self::identity()
    }
}

impl ToneCurves {
    pub fn new(composite: Curve, red: Curve, green: Curve, blue: Curve) -> Self {
        let curves = [composite, red, green, blue];
        let comp = curve_table(&curves[0]);
        let chain = |channel: &Curve| {
            let first = curve_table(channel);
            let mut t = [0u8; 256];
            for (v, f) in t.iter_mut().zip(first) {
                *v = comp[f as usize];
            }
            t
        };
        let tables = ToneTables {
            composite: comp,
            red: chain(&curves[1]),
            green: chain(&curves[2]),
            blue: chain(&curves[3]),
        };
        Self {
            curves,
            tables: Arc::new(tables),
        }
    }
    /// 4 本とも直線（何も変えない）。
    pub fn identity() -> Self {
        Self::new(
            Curve::identity(),
            Curve::identity(),
            Curve::identity(),
            Curve::identity(),
        )
    }
    pub fn curve(&self, channel: ToneChannel) -> &Curve {
        &self.curves[channel.index()]
    }
    /// 1 本だけ差し替えた新しい値。
    pub fn with_curve(&self, channel: ToneChannel, curve: Curve) -> Self {
        let mut curves = self.curves.clone();
        curves[channel.index()] = curve;
        let [c, r, g, b] = curves;
        Self::new(c, r, g, b)
    }
    /// 4 本とも直線か。
    pub fn is_identity(&self) -> bool {
        self.curves.iter().all(Curve::is_identity)
    }
    /// 色のチャンネル: R・G・B の曲線を先に、RGB 全体の曲線を後に当てる（アルファはそのまま）。
    #[inline]
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        let t = &self.tables;
        Rgba8::new(
            t.red[c.r as usize],
            t.green[c.g as usize],
            t.blue[c.b as usize],
            c.a,
        )
    }
    /// スカラー（灰色）のチャンネル: RGB 全体の曲線だけ。
    #[inline]
    pub fn apply_scalar(&self, c: Rgba8) -> Rgba8 {
        let t = &self.tables.composite;
        Rgba8::new(t[c.r as usize], t[c.g as usize], t[c.b as usize], c.a)
    }
    /// 履歴に積む大きさ（4 本の曲線と表）。
    pub fn byte_size(&self) -> u64 {
        32 + 1024 + self.curves.iter().map(Curve::byte_size).sum::<u64>()
    }
}

// ───────── カラーバランス ─────────

/// カラーバランスの範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BalanceRange {
    Shadows,
    Midtones,
    Highlights,
}

impl BalanceRange {
    pub const ALL: [BalanceRange; 3] = [Self::Shadows, Self::Midtones, Self::Highlights];
}

/// カラーバランス: 範囲ごとの [シアン(−)/レッド(+), マゼンタ(−)/グリーン(+), イエロー(−)/ブルー(+)]（−100〜100）と、輝度を保つか。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorBalance {
    values: [[f64; 3]; 3],
    preserve_luminosity: bool,
}

impl ColorBalance {
    /// スライダー 100 が重み 1 の所で動かす量（0〜1 の値に対して）。
    pub(super) const SCALE: f64 = 0.3;
    pub const RANGE: f64 = 100.0;

    pub fn new(
        shadows: [f64; 3],
        midtones: [f64; 3],
        highlights: [f64; 3],
        preserve_luminosity: bool,
    ) -> Result<Self, CoreError> {
        let values = [shadows, midtones, highlights];
        for v in values.iter().flatten() {
            require_finite(*v, "カラーバランス")?;
            if !(-Self::RANGE..=Self::RANGE).contains(v) {
                return Err(CoreError::InvalidArgument("カラーバランス（−100〜100）"));
            }
        }
        Ok(Self {
            values,
            preserve_luminosity,
        })
    }
    /// 何も動かさない（輝度を保つ）。
    pub fn neutral() -> Self {
        Self {
            values: [[0.0; 3]; 3],
            preserve_luminosity: true,
        }
    }
    pub fn values(&self, range: BalanceRange) -> [f64; 3] {
        self.values[range as usize]
    }
    pub fn preserve_luminosity(&self) -> bool {
        self.preserve_luminosity
    }
    /// 1 つの範囲だけ差し替えた新しい値（範囲外は断る）。
    pub fn with_range(&self, range: BalanceRange, values: [f64; 3]) -> Result<Self, CoreError> {
        let mut all = self.values;
        all[range as usize] = values;
        Self::new(all[0], all[1], all[2], self.preserve_luminosity)
    }
    pub fn with_preserve_luminosity(&self, preserve: bool) -> Self {
        Self {
            preserve_luminosity: preserve,
            ..*self
        }
    }
    /// 何も動かさない値か（輝度を保つかは問わない）。
    pub fn is_neutral(&self) -> bool {
        self.values.iter().flatten().all(|v| *v == 0.0)
    }
    /// 調整した色（アルファはそのまま）。f32 の式（[`super::rows`]、行の核と同じ関数）。
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        if self.is_neutral() {
            return c;
        }
        super::rows::color_balance_pixel(&self.values, self.preserve_luminosity, c)
    }
    pub fn byte_size(&self) -> u64 {
        96
    }
    /// 行の核（SIMD）が同じ式をレーンで計算するために、範囲ごとの値と「輝度を保つ」を渡す。
    pub(super) fn parts(&self) -> (&[[f64; 3]; 3], bool) {
        (&self.values, self.preserve_luminosity)
    }
}

// ───────── 明るさ/コントラスト ─────────

/// 明るさ/コントラスト。作るとき 256 の表を作る。
#[derive(Clone, Debug)]
pub struct BrightnessContrast {
    brightness: f64,
    contrast: f64,
    table: Arc<[u8; 256]>,
}

impl PartialEq for BrightnessContrast {
    fn eq(&self, other: &Self) -> bool {
        self.brightness == other.brightness && self.contrast == other.contrast
    }
}

impl BrightnessContrast {
    pub const BRIGHTNESS: std::ops::RangeInclusive<f64> = -150.0..=150.0;
    pub const CONTRAST: std::ops::RangeInclusive<f64> = -50.0..=100.0;

    pub fn new(brightness: f64, contrast: f64) -> Result<Self, CoreError> {
        require_finite(brightness, "明るさ")?;
        require_finite(contrast, "コントラスト")?;
        if !Self::BRIGHTNESS.contains(&brightness) || !Self::CONTRAST.contains(&contrast) {
            return Err(CoreError::InvalidArgument(
                "明るさ（−150〜150）・コントラスト（−50〜100）",
            ));
        }
        let slope = (contrast / 50.0).exp2();
        let exponent = (-brightness / 75.0).exp2();
        let mut table = [0u8; 256];
        for (i, e) in table.iter_mut().enumerate() {
            let stretched = clamp01((UNIT[i] - 0.5) * slope + 0.5);
            *e = to_byte(stretched.powf(exponent));
        }
        Ok(Self {
            brightness,
            contrast,
            table: Arc::new(table),
        })
    }
    pub fn brightness(&self) -> f64 {
        self.brightness
    }
    pub fn contrast(&self) -> f64 {
        self.contrast
    }
    #[inline]
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        let t = &self.table;
        Rgba8::new(t[c.r as usize], t[c.g as usize], t[c.b as usize], c.a)
    }
    pub fn byte_size(&self) -> u64 {
        32 + 256
    }
}

// ───────── 2 値化 ─────────

/// 2 値化（輝度 ≥ しきい値なら白、そうでなければ黒）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threshold {
    level: u8,
}

impl Threshold {
    pub fn new(level: u32) -> Result<Self, CoreError> {
        if !ranges::THRESHOLD_LEVEL.contains(&level) {
            return Err(CoreError::InvalidArgument("しきい値（1〜255）"));
        }
        Ok(Self { level: level as u8 })
    }
    pub fn level(&self) -> u32 {
        u32::from(self.level)
    }
    #[inline]
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        let v = if luminance(c) >= self.level { 255 } else { 0 };
        Rgba8::new(v, v, v, c.a)
    }
    pub fn byte_size(&self) -> u64 {
        32
    }
}

// ───────── ポスタリゼーション ─────────

/// ポスタリゼーション（チャンネルごとに n 段）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Posterize {
    levels: u8,
}

impl Posterize {
    pub fn new(levels: u32) -> Result<Self, CoreError> {
        if !ranges::POSTERIZE_LEVELS.contains(&levels) {
            return Err(CoreError::InvalidArgument("階調（2〜255）"));
        }
        Ok(Self {
            levels: levels as u8,
        })
    }
    pub fn levels(&self) -> u32 {
        u32::from(self.levels)
    }
    #[inline]
    fn channel(&self, v: u8) -> u8 {
        let n = u32::from(self.levels);
        let step = (u32::from(v) * n / 255).min(n - 1);
        ((step * 255 + (n - 1) / 2) / (n - 1)) as u8
    }
    #[inline]
    pub fn apply(&self, c: Rgba8) -> Rgba8 {
        Rgba8::new(self.channel(c.r), self.channel(c.g), self.channel(c.b), c.a)
    }
    /// 0〜255 → 結果の 256 の表（`apply` の 1 チャンネルの式を全値で引いたもの）。
    pub(super) fn table(&self) -> [u8; 256] {
        let mut t = [0u8; 256];
        for (v, e) in t.iter_mut().enumerate() {
            *e = self.channel(v as u8);
        }
        t
    }
    pub fn byte_size(&self) -> u64 {
        32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{ColorStop, OpacityStop};

    fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
        Rgba8::new(r, g, b, a)
    }

    fn two_stop(first: Rgba8, last: Rgba8, alpha_end: f64) -> Ramp {
        Ramp::new(
            vec![
                ColorStop {
                    position: 0.,
                    color: first,
                    midpoint: 0.5,
                },
                ColorStop {
                    position: 1.,
                    color: last,
                    midpoint: 0.5,
                },
            ],
            vec![
                OpacityStop {
                    position: 0.,
                    opacity: 1.,
                    midpoint: 0.5,
                },
                OpacityStop {
                    position: 1.,
                    opacity: alpha_end,
                    midpoint: 0.5,
                },
            ],
            None,
        )
        .unwrap()
    }

    #[test]
    fn luminance_uses_rec709_and_keeps_greys() {
        assert_eq!(luminance(rgba(0, 0, 0, 255)), 0);
        assert_eq!(luminance(rgba(255, 255, 255, 255)), 255);
        // 純色: 0.2126・0.7152・0.0722（× 255 を四捨五入）
        assert_eq!(luminance(rgba(255, 0, 0, 255)), 54);
        assert_eq!(luminance(rgba(0, 255, 0, 255)), 182);
        assert_eq!(luminance(rgba(0, 0, 255, 255)), 18);
        for v in 0..=255u8 {
            assert_eq!(luminance(rgba(v, v, v, 7)), v, "灰色は値のまま");
        }
    }

    #[test]
    fn gradient_map_maps_luminance_through_the_ramp_and_reverses() {
        let black_white = GradientMap::new(Ramp::default(), false);
        for v in 0..=255u8 {
            assert_eq!(black_white.apply(rgba(v, v, v, 200)), rgba(v, v, v, 200));
        }
        let reversed = GradientMap::new(Ramp::default(), true);
        for v in 0..=255u8 {
            assert_eq!(
                reversed.apply(rgba(v, v, v, 9)),
                rgba(255 - v, 255 - v, 255 - v, 9)
            );
        }
        // 赤→青: 輝度 0 は赤、255 は青、アルファは変えない
        let rb = GradientMap::new(
            two_stop(rgba(255, 0, 0, 255), rgba(0, 0, 255, 255), 1.0),
            false,
        );
        assert_eq!(rb.apply(rgba(0, 0, 0, 77)), rgba(255, 0, 0, 77));
        assert_eq!(rb.apply(rgba(255, 255, 255, 77)), rgba(0, 0, 255, 77));
        // 中間の輝度 128 は赤と青の間（t = 128/255）
        let mid = rb.apply(rgba(128, 128, 128, 255));
        assert_eq!((mid.r, mid.g, mid.b), (127, 0, 128));
    }

    #[test]
    fn gradient_map_opacity_blends_back_towards_the_original() {
        // 黒→白で、終わりの不透明度が 0: 白い画素は元の色のまま、黒い画素はランプの色（黒）
        let fade = GradientMap::new(
            two_stop(rgba(255, 0, 0, 255), rgba(0, 255, 0, 255), 0.0),
            false,
        );
        let original = rgba(250, 250, 250, 255);
        let out = fade.apply(original);
        // 輝度 250 → 不透明度 = 1 − 250/255 ≈ 5/255。ランプの色はほぼ緑。元の色が大半
        assert!(out.r > 240 && out.g > 240 && out.b > 240, "{out:?}");
        let dark = fade.apply(rgba(0, 0, 0, 255));
        assert_eq!(dark, rgba(255, 0, 0, 255));
        // 不透明度 0 の端: 元の色がそのまま
        assert_eq!(
            fade.apply(rgba(255, 255, 255, 255)),
            rgba(255, 255, 255, 255)
        );
    }

    #[test]
    fn gradient_map_equality_ignores_the_table_but_not_the_ramp_or_direction() {
        let a = GradientMap::new(Ramp::default(), false);
        assert_eq!(a, GradientMap::new(Ramp::default(), false));
        assert_ne!(a, GradientMap::new(Ramp::default(), true));
        assert_ne!(
            a,
            GradientMap::new(two_stop(rgba(1, 2, 3, 255), rgba(9, 9, 9, 255), 1.0), false)
        );
    }

    #[test]
    fn tone_curves_apply_channels_first_then_the_composite() {
        let identity = ToneCurves::identity();
        assert!(identity.is_identity());
        for v in [0u8, 1, 37, 128, 254, 255] {
            assert_eq!(
                identity.apply(rgba(v, 255 - v, v / 2, 3)),
                rgba(v, 255 - v, v / 2, 3)
            );
        }
        let invert = Curve::new(vec![
            crate::curve::CurvePoint { x: 0., y: 1. },
            crate::curve::CurvePoint { x: 1., y: 0. },
        ])
        .unwrap();
        // RGB 全体の曲線だけ反転: 3 チャンネルとも反転
        let comp = ToneCurves::identity().with_curve(ToneChannel::Composite, invert.clone());
        assert_eq!(comp.apply(rgba(10, 20, 30, 5)), rgba(245, 235, 225, 5));
        assert_eq!(
            comp.apply_scalar(rgba(10, 10, 10, 5)),
            rgba(245, 245, 245, 5)
        );
        // R の曲線だけ反転: R だけ反転。スカラーでは R・G・B の曲線は効かない
        let red = ToneCurves::identity().with_curve(ToneChannel::Red, invert.clone());
        assert_eq!(red.apply(rgba(10, 20, 30, 255)), rgba(245, 20, 30, 255));
        assert_eq!(
            red.apply_scalar(rgba(10, 10, 10, 255)),
            rgba(10, 10, 10, 255)
        );
        // チャンネルが先、全体が後: R を反転してから全体も反転すると R は元に戻る
        let both = red.with_curve(ToneChannel::Composite, invert);
        assert_eq!(both.apply(rgba(10, 20, 30, 255)), rgba(10, 235, 225, 255));
        assert!(!both.is_identity());
        assert_eq!(both.curve(ToneChannel::Red).points().len(), 2);
    }

    #[test]
    fn tone_curves_follow_the_curve_value_at_every_byte() {
        let c = Curve::new(vec![
            crate::curve::CurvePoint { x: 0., y: 0.1 },
            crate::curve::CurvePoint { x: 0.4, y: 0.6 },
            crate::curve::CurvePoint { x: 1., y: 0.9 },
        ])
        .unwrap();
        let t = ToneCurves::identity().with_curve(ToneChannel::Composite, c.clone());
        for v in 0..=255u8 {
            let want = to_byte(c.value(f64::from(v) / 255.0).unwrap());
            assert_eq!(t.apply(rgba(v, v, v, 255)), rgba(want, want, want, 255));
        }
    }

    #[test]
    fn color_balance_is_neutral_at_zero_and_moves_the_chosen_range() {
        let neutral = ColorBalance::neutral();
        assert!(neutral.is_neutral() && neutral.preserve_luminosity());
        for v in [0u8, 50, 128, 255] {
            assert_eq!(
                neutral.apply(rgba(v, 255 - v, v / 3, 9)),
                rgba(v, 255 - v, v / 3, 9)
            );
        }
        // シャドウのレッド +100（輝度を保たない）: 黒は R が 0.3 上がる（= 77）。白には効かない（重み 0）
        let s = ColorBalance::new([100.0, 0.0, 0.0], [0.0; 3], [0.0; 3], false).unwrap();
        assert_eq!(s.apply(rgba(0, 0, 0, 255)), rgba(77, 0, 0, 255));
        assert_eq!(s.apply(rgba(255, 255, 255, 255)), rgba(255, 255, 255, 255));
        // ハイライトのブルー +100: 白は B が上限で変わらず、黒には効かない。中間の灰色（重み 0.25）は B が 0.075 上がる
        let h = ColorBalance::new([0.0; 3], [0.0; 3], [0.0, 0.0, 100.0], false).unwrap();
        assert_eq!(h.apply(rgba(0, 0, 0, 255)), rgba(0, 0, 0, 255));
        let gray = h.apply(rgba(128, 128, 128, 255));
        assert_eq!((gray.r, gray.g), (128, 128));
        assert!(gray.b > 128 && gray.b <= 128 + 20, "{gray:?}");
        // 中間のマゼンタ(−)/グリーン(+) −100: 中間の灰色は G が下がる
        let m = ColorBalance::new([0.0; 3], [0.0, -100.0, 0.0], [0.0; 3], false).unwrap();
        let g = m.apply(rgba(128, 128, 128, 255));
        assert_eq!((g.r, g.b), (128, 128));
        assert!(g.g < 128 - 50, "{g:?}");
    }

    #[test]
    fn color_balance_preserve_luminosity_restores_it_before_clamping() {
        let moved = ColorBalance::new([0.0; 3], [60.0, 0.0, -60.0], [0.0; 3], false).unwrap();
        let kept = moved.with_preserve_luminosity(true);
        let c = rgba(120, 130, 140, 255);
        let y = i32::from(luminance(c));
        let lum_moved = i32::from(luminance(moved.apply(c)));
        let lum_kept = i32::from(luminance(kept.apply(c)));
        assert!(
            (lum_kept - y).abs() <= 1,
            "保つと輝度は元のまま: {y} {lum_kept}"
        );
        assert!((lum_moved - y).abs() > 3, "保たないと動く: {y} {lum_moved}");
        // 色は動いている（灰色にはならない）
        let out = kept.apply(c);
        assert!(out.r > out.b + 10, "{out:?}");
    }

    #[test]
    fn color_balance_refuses_out_of_range_and_non_finite() {
        assert!(ColorBalance::new([100.5, 0.0, 0.0], [0.0; 3], [0.0; 3], true).is_err());
        assert!(ColorBalance::new([0.0; 3], [0.0, -101.0, 0.0], [0.0; 3], true).is_err());
        assert!(ColorBalance::new([0.0; 3], [0.0; 3], [0.0, 0.0, f64::NAN], true).is_err());
        assert!(ColorBalance::new([100.0; 3], [-100.0; 3], [100.0; 3], false).is_ok());
        let ok = ColorBalance::neutral();
        assert!(ok
            .with_range(BalanceRange::Midtones, [0.0, 200.0, 0.0])
            .is_err());
        let set = ok
            .with_range(BalanceRange::Highlights, [1.0, 2.0, 3.0])
            .unwrap();
        assert_eq!(set.values(BalanceRange::Highlights), [1.0, 2.0, 3.0]);
        assert_eq!(set.values(BalanceRange::Shadows), [0.0; 3]);
    }

    #[test]
    fn brightness_contrast_is_identity_at_zero_and_fixes_black_and_white() {
        let zero = BrightnessContrast::new(0.0, 0.0).unwrap();
        for v in 0..=255u8 {
            assert_eq!(zero.apply(rgba(v, v, v, 4)), rgba(v, v, v, 4));
        }
        // 明るさだけ: 黒と白は動かず、中間は明るさの向きに動く
        let up = BrightnessContrast::new(100.0, 0.0).unwrap();
        let down = BrightnessContrast::new(-100.0, 0.0).unwrap();
        assert_eq!(up.apply(rgba(0, 0, 0, 255)), rgba(0, 0, 0, 255));
        assert_eq!(up.apply(rgba(255, 255, 255, 255)), rgba(255, 255, 255, 255));
        assert!(up.apply(rgba(128, 128, 128, 255)).r > 128);
        assert!(down.apply(rgba(128, 128, 128, 255)).r < 128);
        // 広がるほどコントラストが強い: 暗い値はより暗く、明るい値はより明るく。コントラスト 0 の中心（127.5 付近）は動かない
        let hard = BrightnessContrast::new(0.0, 50.0).unwrap();
        assert!(hard.apply(rgba(64, 64, 64, 255)).r < 64);
        assert!(hard.apply(rgba(192, 192, 192, 255)).r > 192);
        assert!((i32::from(hard.apply(rgba(128, 128, 128, 255)).r) - 128).abs() <= 1);
        // 最大のコントラスト（傾き 4）は、0.375 未満を 0 に、0.625 超を 1 に潰す
        let max = BrightnessContrast::new(0.0, 100.0).unwrap();
        assert_eq!(max.apply(rgba(90, 90, 90, 255)).r, 0);
        assert_eq!(max.apply(rgba(170, 170, 170, 255)).r, 255);
        // 弱める（傾き 0.5）と黒は 0.25 に持ち上がる
        let soft = BrightnessContrast::new(0.0, -50.0).unwrap();
        assert_eq!(soft.apply(rgba(0, 0, 0, 255)).r, 64);
        assert_eq!(soft.apply(rgba(255, 255, 255, 255)).r, 191);
    }

    #[test]
    fn brightness_contrast_is_monotone_and_refuses_the_wrong_range() {
        for (b, c) in [
            (150.0, 100.0),
            (-150.0, -50.0),
            (150.0, -50.0),
            (-150.0, 100.0),
            (33.0, 17.0),
        ] {
            let op = BrightnessContrast::new(b, c).unwrap();
            let mut prev = 0u8;
            for v in 0..=255u8 {
                let out = op.apply(rgba(v, v, v, 255)).r;
                assert!(out >= prev, "{b} {c} {v}");
                prev = out;
            }
        }
        assert!(BrightnessContrast::new(150.1, 0.0).is_err());
        assert!(BrightnessContrast::new(-150.1, 0.0).is_err());
        assert!(BrightnessContrast::new(0.0, 100.1).is_err());
        assert!(BrightnessContrast::new(0.0, -50.1).is_err());
        assert!(BrightnessContrast::new(f64::NAN, 0.0).is_err());
        assert!(BrightnessContrast::new(0.0, f64::INFINITY).is_err());
    }

    #[test]
    fn threshold_splits_on_luminance_and_keeps_alpha() {
        let t = Threshold::new(128).unwrap();
        assert_eq!(t.apply(rgba(127, 127, 127, 40)), rgba(0, 0, 0, 40));
        assert_eq!(t.apply(rgba(128, 128, 128, 40)), rgba(255, 255, 255, 40));
        // 純色は輝度で決まる: 緑（182）は白、赤（54）・青（18）は黒
        assert_eq!(t.apply(rgba(0, 255, 0, 255)), rgba(255, 255, 255, 255));
        assert_eq!(t.apply(rgba(255, 0, 0, 255)), rgba(0, 0, 0, 255));
        assert_eq!(t.apply(rgba(0, 0, 255, 255)), rgba(0, 0, 0, 255));
        // 端: 1 は黒だけが黒、255 は白だけが白
        let low = Threshold::new(1).unwrap();
        assert_eq!(low.apply(rgba(0, 0, 0, 255)).r, 0);
        assert_eq!(low.apply(rgba(1, 1, 1, 255)).r, 255);
        let high = Threshold::new(255).unwrap();
        assert_eq!(high.apply(rgba(254, 254, 254, 255)).r, 0);
        assert_eq!(high.apply(rgba(255, 255, 255, 255)).r, 255);
        assert!(Threshold::new(0).is_err());
        assert!(Threshold::new(256).is_err());
    }

    #[test]
    fn posterize_levels_are_even_and_cover_the_ends() {
        let two = Posterize::new(2).unwrap();
        assert_eq!(two.apply(rgba(127, 128, 0, 9)), rgba(0, 255, 0, 9));
        let four = Posterize::new(4).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for v in 0..=255u8 {
            seen.insert(four.apply(rgba(v, v, v, 255)).r);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![0, 85, 170, 255]);
        // 255 段は全ての値をそのまま通す
        let all = Posterize::new(255).unwrap();
        for v in 0..=255u8 {
            let out = all.apply(rgba(v, v, v, 255)).r;
            assert!((i32::from(out) - i32::from(v)).abs() <= 1, "{v} {out}");
        }
        // 単調
        for n in [2u32, 3, 5, 16, 255] {
            let p = Posterize::new(n).unwrap();
            let mut prev = 0;
            for v in 0..=255u8 {
                let out = p.apply(rgba(v, 0, 0, 255)).r;
                assert!(out >= prev, "{n} {v}");
                prev = out;
            }
            assert_eq!(p.apply(rgba(255, 0, 0, 255)).r, 255);
            assert_eq!(p.apply(rgba(0, 0, 0, 255)).r, 0);
        }
        assert!(Posterize::new(1).is_err());
        assert!(Posterize::new(256).is_err());
    }
}
