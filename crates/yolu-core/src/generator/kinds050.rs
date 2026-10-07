//! 0.5.0 の Generator の種類（Rust 版だけ）: 模様（66）・アイランドごとのばらつき（67）・ライト（68）・マスクの組み立て（69）。
//!
//! - 模様は UV の空間の繰り返し（縞・市松・水玉・縁・格子）。マップを読まない。
//! - ライトは焼いたワールドの法線と光の向きの内積（`softness` で明暗の境を回り込ませる、`ambient` で底上げ）。
//! - マスクの組み立ては、焼いた曲率・AO・位置の高さ・厚みを、それぞれ位置とコントラストで 0〜1 にし（ヒストグラムスキャンと同じ式）、
//!   重みを付けて掛け合わせる・最大・足す。重みが 0 のマップは読まない（無くても断らない）。
//! - アイランドごとのばらつきは、モデルの UV アイランドの番号とシードから決まる一様な乱数の値（アイランドの中は同じ値）。アイランドの番号は
//!   `geometry::IslandMap`（モデルが同じなら解像度によらず同じ番号）から読む。
//!
//! 式は + − × ÷ sqrt floor と多項式の sin・cos（`noisefn::sin_cos_deg`）だけで、画素ごとに座標・マップの値だけから決まる。SIMD にしない
//! （行の評価も 1 画素ずつこの式を呼ぶ）ので、道によらず同じ値。

use super::noisefn::{cell_hash, sin_cos_deg};
use super::{unit, Error, MapKind};
use crate::math::clamp01;
use crate::ranges;

/// 模様の形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PatternShape {
    Stripes = 0,
    Checker = 1,
    Dots = 2,
    Border = 3,
    Grid = 4,
}
impl PatternShape {
    pub const ALL: [PatternShape; 5] = [
        PatternShape::Stripes,
        PatternShape::Checker,
        PatternShape::Dots,
        PatternShape::Border,
        PatternShape::Grid,
    ];
    pub fn from_index(i: i64) -> Option<Self> {
        Self::ALL.get(usize::try_from(i).ok()?).copied()
    }
}

/// 模様（`Kind::Pattern`）の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    pub shape: PatternShape,
    /// UV の 0〜1 に繰り返す回数（1〜512）。縁は使わない。
    pub scale: f64,
    /// 回転（度。0〜360）。縁は使わない。
    pub angle: f64,
    /// 縞・水玉・格子の太さ、縁の幅（1 つの繰り返しに対する割合。0〜1）。市松は使わない。
    pub width: f64,
    /// 境目のぼかし（0〜1。1 つの繰り返しの半分まで）。
    pub softness: f64,
    /// 模様をずらす量（1 つの繰り返しに対する割合。0〜1）。
    pub offset: [f64; 2],
}
impl Default for Pattern {
    fn default() -> Self {
        Self {
            shape: PatternShape::Stripes,
            scale: 8.,
            angle: 0.,
            width: 0.5,
            softness: 0.,
            offset: [0.; 2],
        }
    }
}
impl Pattern {
    pub(super) fn validate(&self) -> Result<(), Error> {
        let within = |v: f64, r: std::ops::RangeInclusive<f64>| v.is_finite() && r.contains(&v);
        if !within(self.scale, ranges::PATTERN_SCALE)
            || !within(self.angle, ranges::TURN_DEGREES)
            || !unit(self.width)
            || !unit(self.softness)
            || !self.offset.iter().all(|v| unit(*v))
        {
            return Err(Error::Invalid(
                "模様の大きさ・角度・太さ・ぼかし・ずれが範囲外です",
            ));
        }
        Ok(())
    }
    /// UV（0〜1。左下が原点）での値（0〜1）。
    pub fn value(&self, u: f64, v: f64) -> f64 {
        // 内側を正とする距離（1 つの繰り返しの単位。縁は UV の単位）
        let distance = if self.shape == PatternShape::Border {
            self.width * 0.5 - u.min(1. - u).min(v).min(1. - v)
        } else {
            let (sin, cos) = sin_cos_deg(self.angle);
            let (cu, cv) = (u - 0.5, v - 0.5);
            let px = (cos * cu - sin * cv) * self.scale + 0.5 * self.scale + self.offset[0];
            let py = (sin * cu + cos * cv) * self.scale + 0.5 * self.scale + self.offset[1];
            let (fx, fy) = (px - px.floor(), py - py.floor());
            match self.shape {
                PatternShape::Stripes => self.width * 0.5 - (fx - 0.5).abs(),
                PatternShape::Dots => {
                    let (dx, dy) = (fx - 0.5, fy - 0.5);
                    self.width * 0.5 - (dx * dx + dy * dy).sqrt()
                }
                PatternShape::Grid => {
                    let edge = fx.min(1. - fx).min(fy).min(1. - fy);
                    self.width * 0.5 - edge
                }
                // 市松: 升の色で内外、升の境からの距離
                _ => {
                    let edge = fx.min(1. - fx).min(fy).min(1. - fy);
                    let odd = (px.floor() + py.floor()).rem_euclid(2.) != 0.;
                    if odd {
                        edge
                    } else {
                        -edge
                    }
                }
            }
        };
        let half = self.softness * 0.5;
        if half <= 0. {
            return if distance > 0. { 1. } else { 0. };
        }
        let t = clamp01((distance + half) / (2. * half));
        t * t * (3. - 2. * t)
    }
}

/// ライト（`Kind::Light`）の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// 光の来る向きの水平の角度（度。0〜360。0 が +Z、90 が +X）。
    pub azimuth: f64,
    /// 光の高さ（度。0 が水平、90 が真上 +Y）。
    pub elevation: f64,
    /// 明暗の境の回り込み（0〜1。0 は内積の 0 で切り、1 は裏側まで半分の明るさで回り込む）。
    pub softness: f64,
    /// 暗い所の底上げ（0〜1）。
    pub ambient: f64,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            azimuth: 45.,
            elevation: 45.,
            softness: 0.2,
            ambient: 0.,
        }
    }
}
impl Light {
    pub(super) fn validate(&self) -> Result<(), Error> {
        let within = |v: f64, r: std::ops::RangeInclusive<f64>| v.is_finite() && r.contains(&v);
        if !within(self.azimuth, ranges::TURN_DEGREES)
            || !within(self.elevation, ranges::LIGHT_ELEVATION)
            || !unit(self.softness)
            || !unit(self.ambient)
        {
            return Err(Error::Invalid("ライトの向き・回り込み・底上げが範囲外です"));
        }
        Ok(())
    }
    /// 光の来る向き（単位ベクトル。Y が上）。
    pub fn direction(&self) -> [f64; 3] {
        let (sin_az, cos_az) = sin_cos_deg(self.azimuth);
        let (sin_el, cos_el) = sin_cos_deg(self.elevation);
        [cos_el * sin_az, sin_el, cos_el * cos_az]
    }
    /// 法線（焼いたマップの u16 の 3 成分）での値。向きの定まらない法線は None。
    pub fn value(&self, normal: [f64; 3], direction: [f64; 3]) -> Option<f64> {
        let [nx, ny, nz] = normal.map(|v| v / 65535. * 2. - 1.);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len <= 1e-9 {
            return None;
        }
        let dot = (nx * direction[0] + ny * direction[1] + nz * direction[2]) / len;
        let lit = clamp01((dot + self.softness) / (1. + self.softness));
        Some(self.ambient + (1. - self.ambient) * lit)
    }
}

/// マスクの組み立ての合わせ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MaskCombine {
    /// 重みを付けて掛ける（重み 0 は 1 を掛ける）。
    Multiply = 0,
    /// 重みを掛けた値の最大。
    Max = 1,
    /// 重みを掛けた値の和（1 まで）。
    Add = 2,
}
impl MaskCombine {
    pub const ALL: [MaskCombine; 3] = [MaskCombine::Multiply, MaskCombine::Max, MaskCombine::Add];
    pub fn from_index(i: i64) -> Option<Self> {
        Self::ALL.get(usize::try_from(i).ok()?).copied()
    }
}

/// マスクの組み立ての 1 つのマップの読み方。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskInput {
    /// 重み（0〜1。0 はこのマップを読まない）。
    pub weight: f64,
    /// どの値から上を 1 へ寄せるか（0〜1）。
    pub level: f64,
    /// 境目の鋭さ（0〜1。0 は値のまま）。
    pub contrast: f64,
    pub invert: bool,
}
impl MaskInput {
    const fn new(weight: f64) -> Self {
        Self {
            weight,
            level: 0.5,
            contrast: 0.,
            invert: false,
        }
    }
    /// 0〜1 の値をレベルで 0〜1 にする（ヒストグラムスキャンと同じ: 幅 w = max(1 − contrast, 1/255)）。
    fn level(&self, v: f64) -> f64 {
        let w = (1. - self.contrast).max(1. / 255.);
        let t = clamp01((v - (self.level - w / 2.)) / w);
        if self.invert {
            1. - t
        } else {
            t
        }
    }
}

/// マスクの組み立て（`Kind::MaskBuilder`）の設定。マップの並びは `MaskBuilder::MAPS`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskBuilder {
    /// 曲率・AO・位置の高さ・厚みの順。
    pub inputs: [MaskInput; 4],
    pub combine: MaskCombine,
}
impl Default for MaskBuilder {
    fn default() -> Self {
        Self {
            inputs: [
                MaskInput::new(1.),
                MaskInput::new(0.),
                MaskInput::new(0.),
                MaskInput::new(0.),
            ],
            combine: MaskCombine::Multiply,
        }
    }
}
impl MaskBuilder {
    /// 読むマップ（`inputs` と同じ並び）。
    pub const MAPS: [MapKind; 4] = [
        MapKind::Curvature,
        MapKind::AmbientOcclusion,
        MapKind::Position,
        MapKind::Thickness,
    ];
    pub(super) fn validate(&self) -> Result<(), Error> {
        if !self
            .inputs
            .iter()
            .all(|i| unit(i.weight) && unit(i.level) && unit(i.contrast))
        {
            return Err(Error::Invalid(
                "マスクの組み立ての重み・位置・コントラストが範囲外です",
            ));
        }
        Ok(())
    }
    /// 重みが 0 でないマップ。
    pub fn used(&self) -> Vec<MapKind> {
        Self::MAPS
            .iter()
            .zip(&self.inputs)
            .filter(|(_, i)| i.weight > 0.)
            .map(|(k, _)| *k)
            .collect()
    }
    /// マップの値（0〜1。位置は高さ Y）からの値。`read` は読むマップ（重みが 0 でないもの）の値を返し、被覆の無い画素は None。
    pub fn value(&self, mut read: impl FnMut(MapKind) -> Option<f64>) -> Option<f64> {
        let mut out = match self.combine {
            MaskCombine::Multiply => 1.,
            _ => 0.,
        };
        for (kind, input) in Self::MAPS.iter().zip(&self.inputs) {
            if input.weight <= 0. {
                continue;
            }
            let t = input.level(read(*kind)?);
            out = match self.combine {
                MaskCombine::Multiply => out * (1. - input.weight + input.weight * t),
                MaskCombine::Max => out.max(input.weight * t),
                MaskCombine::Add => out + input.weight * t,
            };
        }
        Some(clamp01(out))
    }
}

/// アイランドごとのばらつき（`Kind::UvIslandVariation`）の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IslandVariation {
    /// 乱数の種（同じシード・同じモデルなら同じ値）。
    pub seed: i32,
    /// 値の下端（0〜1）。
    pub min: f64,
    /// 値の上端（0〜1。`min` 以上。等しければアイランドによらず同じ値）。
    pub max: f64,
}
impl Default for IslandVariation {
    fn default() -> Self {
        Self {
            seed: 0,
            min: 0.,
            max: 1.,
        }
    }
}
impl IslandVariation {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if !unit(self.min) || !unit(self.max) || self.min > self.max {
            return Err(Error::Invalid(
                "アイランドごとのばらつきの最小・最大が範囲外か、最小が最大を超えています",
            ));
        }
        Ok(())
    }
    /// シードから引く乱数の種（重ねるノイズと同じ混ぜ方の 1 つ目）。アイランドごとの値はこれとアイランドの番号で決まる。
    pub(super) fn stream(&self) -> u32 {
        super::noise::seeds(self.seed)[0]
    }
    /// アイランドの番号 `island`（1 から）の基底の値（`min + (max − min) × u`。レベル・反転の前）。u はアイランドの番号とシードの一様な乱数（[0, 1)）。
    pub fn value(&self, island: u32) -> f64 {
        self.value_in(self.stream(), island)
    }
    /// [`Self::value`] を、前もって引いた乱数の種 `stream`（[`Self::stream`]）で。u は格子の hash（ノイズの格子の角と同じ `cell_hash`）の
    /// 32 bit を 2^-32 倍したもの（u32 → f64 と 2 の冪の掛け算は丸めが無いので、道・環境によらず同じ値）。
    pub(super) fn value_in(&self, stream: u32, island: u32) -> f64 {
        let u = f64::from(cell_hash(stream, island as i32, 0, 0)) * (1. / 4_294_967_296.);
        self.min + (self.max - self.min) * u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripes_dots_grid_checker_and_border_follow_their_shapes() {
        let p = |shape, width| Pattern {
            shape,
            scale: 4.,
            width,
            ..Pattern::default()
        };
        // 縞: 1 つの繰り返し（u の 0.25）の真ん中が 1、端が 0
        let stripes = p(PatternShape::Stripes, 0.5);
        assert_eq!(stripes.value(0.125, 0.3), 1.);
        assert_eq!(stripes.value(0.01, 0.3), 0.);
        assert_eq!(stripes.value(0.25 + 0.125, 0.9), 1.);
        // 水玉: 升の真ん中が 1、角が 0
        let dots = p(PatternShape::Dots, 0.6);
        assert_eq!(dots.value(0.125, 0.125), 1.);
        assert_eq!(dots.value(0.01, 0.01), 0.);
        // 格子: 升の境が 1、真ん中が 0
        let grid = p(PatternShape::Grid, 0.2);
        assert_eq!(grid.value(0.25, 0.1), 1.);
        assert_eq!(grid.value(0.125, 0.125), 0.);
        // 市松: 隣の升は反対
        let checker = p(PatternShape::Checker, 0.5);
        assert_ne!(checker.value(0.125, 0.125), checker.value(0.375, 0.125));
        assert_eq!(checker.value(0.125, 0.125), checker.value(0.375, 0.375));
        // 縁: UV の縁から幅の半分まで 1
        let border = p(PatternShape::Border, 0.2);
        assert_eq!(border.value(0.05, 0.5), 1.);
        assert_eq!(border.value(0.5, 0.95), 1.);
        assert_eq!(border.value(0.5, 0.5), 0.);
        // ぼかしは 0〜1 の間をなめらかに
        let soft = Pattern {
            softness: 1.,
            ..stripes
        };
        let v = soft.value(0.0625, 0.3);
        assert!(v > 0. && v < 1., "{v}");
        // ずれ・回転
        let shifted = Pattern {
            offset: [0.5, 0.],
            ..stripes
        };
        assert_eq!(shifted.value(0.01, 0.3), 1.);
        let turned = Pattern {
            angle: 90.,
            ..stripes
        };
        assert_eq!(turned.value(0.3, 0.125), stripes.value(0.125, 0.3));
    }

    #[test]
    fn light_is_the_wrapped_lambert_with_ambient() {
        let up = Light {
            azimuth: 0.,
            elevation: 90.,
            softness: 0.,
            ambient: 0.,
        };
        let d = up.direction();
        assert!((d[1] - 1.).abs() < 1e-12);
        let n = |x: f64, y: f64, z: f64| [x, y, z].map(|v| (v * 0.5 + 0.5) * 65535.);
        assert!((up.value(n(0., 1., 0.), d).unwrap() - 1.).abs() < 1e-4);
        assert_eq!(up.value(n(0., -1., 0.), d), Some(0.));
        assert!(up.value(n(1., 0., 0.), d).unwrap() < 1e-4);
        let soft = Light {
            softness: 1.,
            ambient: 0.25,
            ..up
        };
        // 横向きは回り込みで半分、底上げで 0.25 + 0.75 × 0.5
        assert!((soft.value(n(1., 0., 0.), d).unwrap() - 0.625).abs() < 1e-4);
        assert_eq!(soft.value([32767.5; 3], d), None);
    }

    #[test]
    fn the_new_kinds_give_the_same_bytes_on_every_simd_path() {
        use crate::generator::{evaluate, BoundGenerator, Image, Kind, Options, Settings, Target};
        use crate::math::simd::forced;
        let (w, h) = (23u32, 17u32);
        let mut source = Vec::new();
        for i in 0..w * h {
            source.extend_from_slice(&[(i * 5) as u8, (i * 9) as u8, (i * 2) as u8, (i * 7) as u8]);
        }
        let image = Image::new(&source, w, h).unwrap();
        let mut s = Settings::new(Kind::Pattern);
        s.pattern.shape = PatternShape::Dots;
        s.pattern.softness = 0.3;
        s.low = 0.1;
        s.invert = true;
        let b = BoundGenerator::bind(
            &s,
            &[],
            None,
            (w, h),
            Err(super::super::anchor::Issue::NotChosen),
        )
        .unwrap();
        for target in [Target::Color, Target::Scalar, Target::Mask] {
            let run = || {
                evaluate(
                    &image,
                    &b,
                    crate::Rect::new(0, 0, w, h),
                    target,
                    0.7,
                    &Options::default(),
                )
                .unwrap()
                .pixels
            };
            let expected = forced::with_level(crate::math::simd::Level::Scalar, run);
            for level in forced::supported() {
                assert!(
                    forced::with_level(level, run) == expected,
                    "{target:?} {level:?}"
                );
            }
        }
    }

    #[test]
    fn mask_builder_levels_weights_and_combines() {
        let mut m = MaskBuilder::default();
        // 既定は曲率だけ、値のまま
        assert_eq!(m.used(), vec![MapKind::Curvature]);
        assert_eq!(m.value(|_| Some(0.3)), Some(0.3));
        m.inputs[1] = MaskInput {
            weight: 0.5,
            level: 0.5,
            contrast: 1.,
            invert: true,
        };
        let read = |k: MapKind| match k {
            MapKind::Curvature => Some(0.8),
            MapKind::AmbientOcclusion => Some(0.9),
            _ => None,
        };
        // AO 0.9 は 2 値で 1、反転して 0。掛ける: 0.8 × (1 − 0.5 + 0) = 0.4
        assert!((m.value(read).unwrap() - 0.4).abs() < 1e-12);
        m.combine = MaskCombine::Max;
        assert!((m.value(read).unwrap() - 0.8).abs() < 1e-12);
        m.combine = MaskCombine::Add;
        assert!((m.value(read).unwrap() - 0.8).abs() < 1e-12);
        // 被覆の無い画素は None、重み 0 のマップは読まない
        m.inputs[2].weight = 0.;
        assert_eq!(m.value(|k| (k != MapKind::Curvature).then_some(0.5)), None);
    }
}
