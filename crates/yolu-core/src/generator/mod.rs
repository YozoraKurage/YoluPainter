//! 文書非依存の Generator。マップは焼いた u16 画像、Anchor は評価済みのスナップショットを読む。
pub mod anchor;
mod evaluate;
mod grunge;
mod mixing;
mod noise;
mod noisefn;
mod preview;
mod procedural;
mod ramp;
mod shape;
pub use evaluate::{evaluate, BoundGenerator, Generated, Options, Output, Target};
pub use preview::preview;
pub use procedural::{
    CellOutput, FractalMode, GrungePreset, NoiseBasis, Procedural, ProceduralSpace,
};
pub use crate::curve::CurvePoint;
pub use mixing::{LuminanceCorrection, MixMode};
pub use ramp::{ColorStop, OpacityStop, Preset, Ramp};
pub use shape::{ModelFrame, Shape, Volume};
pub(crate) use noisefn::MAX_OCTAVES;
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Budget { needed: u64, budget: u64 },
    Cancelled,
    Allocation,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Budget { needed, budget } => write!(
                f,
                "ジェネレーターの作業メモリが予算を超えます: {needed} > {budget} バイト"
            ),
            Self::Cancelled => f.write_str("ジェネレーターの評価を取り消しました"),
            Self::Allocation => f.write_str("ジェネレーターの作業メモリを確保できません"),
        }
    }
}
impl std::error::Error for Error {}
pub(super) fn unit(x: f64) -> bool {
    x.is_finite() && crate::ranges::UNIT.contains(&x)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    EdgeWear = 0,
    Dirt = 1,
    PositionGradient = 2,
    Thickness = 3,
    Direction = 4,
    ShapeGradient = 5,
    IdColor = 6,
    Anchor = 7,
    /// ノイズ（値・Perlin・Worley の基底と fBm・ridged・turbulence の重ね）。**Rust 版だけの種類**。C# の種類（0〜7）と重ならない
    /// 64 から振る。Unity 版は読めない（正本の版 23 で断る）。
    Noise = 64,
    /// グランジ（ノイズの組み合わせのプリセット）。Rust 版だけの種類。
    Grunge = 65,
}
impl Kind {
    /// 正本の種類の番号から。知らない番号は None。
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::EdgeWear,
            1 => Self::Dirt,
            2 => Self::PositionGradient,
            3 => Self::Thickness,
            4 => Self::Direction,
            5 => Self::ShapeGradient,
            6 => Self::IdColor,
            7 => Self::Anchor,
            64 => Self::Noise,
            65 => Self::Grunge,
            _ => return None,
        })
    }
    /// Rust 版だけの種類か（マップを読まず位置・UV から値を作る。Unity 版は読めない）。
    pub fn is_procedural(self) -> bool {
        matches!(self, Self::Noise | Self::Grunge)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Blend {
    Multiply = 0,
    Replace = 1,
    Screen = 2,
    Max = 3,
    Min = 4,
    Add = 5,
    Subtract = 6,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NoiseSpace {
    Model = 0,
    Uv = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum MapKind {
    WorldNormal = 0,
    Position = 1,
    AmbientOcclusion = 2,
    Curvature = 3,
    Thickness = 4,
    TangentNormal = 5,
    Height = 6,
    Id = 7,
    BentNormal = 8,
    Opacity = 9,
}
impl MapKind {
    pub fn channels(self) -> usize {
        if matches!(
            self,
            Self::WorldNormal | Self::Position | Self::TangentNormal | Self::Id | Self::BentNormal
        ) {
            3
        } else {
            1
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapState {
    Current,
    Stale,
    Unverified,
}
/// 由来の現在性はベイク/文書側が照合する。内容は評価の間固定し、空の被覆は値を読まない。
#[derive(Clone, Debug)]
pub struct Map<'a> {
    pub kind: MapKind,
    pub width: u32,
    pub height: u32,
    pub data: &'a [u16],
    pub coverage: &'a [u8],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub condition_key: &'a str,
    pub state: MapState,
}
impl Map<'_> {
    fn validate(&self) -> Result<(), Error> {
        let n = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || n != self.coverage.len() as u64
            || n.checked_mul(self.kind.channels() as u64) != Some(self.data.len() as u64)
            || !key_valid(self.condition_key)
            || self
                .bounds_min
                .iter()
                .zip(self.bounds_max)
                .any(|(a, b)| !a.is_finite() || !b.is_finite() || b < *a || !(b - *a).is_finite())
        {
            return Err(Error::Invalid("マップの画像・境界箱・由来が不正です"));
        }
        Ok(())
    }
}
fn key_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub kind: Kind,
    pub low: f64,
    pub high: f64,
    pub softness: f64,
    pub invert: bool,
    pub noise_amount: f64,
    pub noise_scale: f64,
    pub noise_seed: i32,
    pub noise_space: NoiseSpace,
    pub blend: Blend,
    pub balance: f64,
    pub axis: usize,
    pub direction: [f64; 3],
    pub use_bent_normal: bool,
    pub volume: Volume,
    pub ramp: Option<Ramp>,
    pub id_colors: Vec<u32>,
    pub id_tolerance: u8,
    /// Anchor の参照。Anchor 以外の種類は既定値のまま（[`anchor::Reference`]）。
    pub anchor: anchor::Reference,
    pub pins: BTreeMap<MapKind, String>,
    /// ノイズ・グランジの設定（[`Procedural`]）。Noise・Grunge 以外の種類は既定のまま。
    pub procedural: Procedural,
}
impl Settings {
    pub fn new(kind: Kind) -> Self {
        let mut g = Self {
            kind,
            low: 0.,
            high: 1.,
            softness: 0.,
            invert: false,
            noise_amount: 0.,
            noise_scale: 0.05,
            noise_seed: 0,
            noise_space: NoiseSpace::Model,
            blend: Blend::Multiply,
            balance: 0.5,
            axis: 1,
            direction: [0., 1., 0.],
            use_bent_normal: false,
            volume: Volume::default(),
            ramp: None,
            id_colors: vec![],
            id_tolerance: 8,
            anchor: anchor::Reference::default_for(kind),
            pins: BTreeMap::new(),
            procedural: Procedural::default(),
        };
        match kind {
            Kind::EdgeWear => {
                g.low = 0.04;
                g.high = 0.3;
                g.softness = 0.5;
                g.noise_amount = 0.6;
            }
            Kind::Dirt => {
                g.low = 0.15;
                g.high = 0.6;
                g.softness = 0.5;
                g.noise_amount = 0.4;
            }
            Kind::Direction => {
                g.low = 0.6;
                g.high = 0.95;
                g.softness = 0.5;
                g.noise_amount = 0.3;
            }
            Kind::Grunge => g.procedural = Procedural::for_preset(GrungePreset::Stain),
            _ => {}
        }
        g
    }
    /// 指定のプリセットのグランジ（模様の大きさ・レベルはプリセットの既定）。
    pub fn grunge(preset: GrungePreset) -> Self {
        let mut g = Self::new(Kind::Grunge);
        g.set_preset(preset);
        g
    }
    /// グランジのプリセットを選び直す（模様の大きさ・レベルをプリセットの既定にする。ほかの設定は残す）。
    pub fn set_preset(&mut self, preset: GrungePreset) {
        self.procedural.preset = preset;
        self.procedural.scale = preset.default_scale();
        (self.low, self.high) = preset.default_levels();
    }
    pub fn algorithm_version(&self) -> u32 {
        if self.ramp.is_some() {
            2
        } else {
            1
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        if !unit(self.low)
            || !unit(self.high)
            || self.high - self.low < 0.001
            || !unit(self.softness)
            || !unit(self.noise_amount)
            || !self.noise_scale.is_finite()
            || !crate::ranges::NOISE_SCALE.contains(&self.noise_scale)
        {
            return Err(Error::Invalid(
                "ジェネレーターのレベル・減衰・ノイズが範囲外です",
            ));
        }
        if !unit(self.balance)
            || (self.kind != Kind::Dirt && self.balance != 0.5)
            || self.axis > 2
            || (self.kind != Kind::PositionGradient && self.axis != 1)
        {
            return Err(Error::Invalid("ジェネレーターの種類に対して割合・軸が不正です"));
        }
        let [x, y, z] = self.direction;
        if self
            .direction
            .iter()
            .any(|x| !x.is_finite() || !crate::ranges::DIRECTION_COMPONENT.contains(x))
            || x * x + y * y + z * z < 1e-12
            || (self.kind != Kind::Direction
                && (self.direction != [0., 1., 0.] || self.use_bent_normal))
        {
            return Err(Error::Invalid("ジェネレーターの方向が不正です"));
        }
        self.volume.validate()?;
        if self.kind != Kind::ShapeGradient
            && (self.volume != Volume::default() || self.ramp.is_some())
        {
            return Err(Error::Invalid("形とランプは形状グラデーション専用です"));
        }
        if self.id_colors.len() > 32
            || self
                .id_colors
                .iter()
                .enumerate()
                .any(|(i, c)| *c > 0xffffff || self.id_colors[..i].contains(c))
            || (self.kind != Kind::IdColor
                && (!self.id_colors.is_empty() || self.id_tolerance != 8))
        {
            return Err(Error::Invalid("ID 色の数・値・重複・種類が不正です"));
        }
        if self.kind == Kind::Anchor {
            if self.anchor.channel == crate::Channel::Normal {
                return Err(Error::Invalid(
                    "Anchor は Normal チャンネルを読めません。Height を読んでください",
                ));
            }
        } else if self.anchor != anchor::Reference::default_for(self.kind) {
            return Err(Error::Invalid("Anchor の参照は Anchor 専用です"));
        }
        if self.kind.is_procedural() {
            self.procedural.validate(self.kind)?;
            if self.noise_amount != 0.
                || self.noise_scale != 0.05
                || self.noise_seed != 0
                || self.noise_space != NoiseSpace::Model
            {
                return Err(Error::Invalid(
                    "ノイズ・グランジは重ねるノイズを持たず、大きさ・シードは自身の設定で決めます",
                ));
            }
        } else if self.procedural != Procedural::default() {
            return Err(Error::Invalid(
                "ノイズ・グランジの設定はノイズ・グランジ専用です",
            ));
        }
        for (kind, key) in &self.pins {
            if !self.candidate_maps().contains(kind) || !key_valid(key) {
                return Err(Error::Invalid(
                    "ピンは対応するマップの小文字16進64桁が必要です",
                ));
            }
        }
        Ok(())
    }
    pub fn candidate_maps(&self) -> Vec<MapKind> {
        use MapKind::*;
        match self.kind {
            Kind::EdgeWear => vec![Curvature, Position],
            Kind::Dirt => vec![AmbientOcclusion, Curvature, Position],
            Kind::PositionGradient | Kind::ShapeGradient | Kind::Anchor => vec![Position],
            Kind::Thickness => vec![Thickness, Position],
            Kind::Direction => vec![WorldNormal, BentNormal, Position],
            Kind::IdColor => vec![Id, Position],
            Kind::Noise | Kind::Grunge => vec![Position, WorldNormal],
        }
    }
    pub fn used_maps(&self) -> Vec<MapKind> {
        use MapKind::*;
        let mut v = match self.kind {
            Kind::EdgeWear => vec![Curvature],
            Kind::Dirt => {
                let mut v = vec![];
                if self.balance < 1. {
                    v.push(AmbientOcclusion)
                }
                if self.balance > 0. {
                    v.push(Curvature)
                }
                v
            }
            Kind::PositionGradient | Kind::ShapeGradient => vec![Position],
            Kind::Thickness => vec![Thickness],
            Kind::Direction => vec![if self.use_bent_normal {
                BentNormal
            } else {
                WorldNormal
            }],
            Kind::IdColor => vec![Id],
            Kind::Anchor => vec![],
            // 位置のマップが使えないときは UV に落とす（入力のまま通さない）ので、読むマップは設定だけで決まる
            Kind::Noise | Kind::Grunge => return self.procedural.maps(self.kind),
        };
        if self.noise_amount > 0. && self.noise_space == NoiseSpace::Model && !v.contains(&Position)
        {
            v.push(Position);
        }
        v
    }
}
/// マップが使えない段は入力を保つ。壊れた設定とは区別する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inactive {
    MissingMap(MapKind),
    StaleMap(MapKind),
    UnverifiedMap(MapKind),
    MapSize(MapKind),
    PinMismatch(MapKind),
    MissingFrame,
    EmptyBounds,
    NoIdColors,
    Anchor(anchor::Issue),
}
/// 左下原点の読み取り専用 RGBA8。タイルはこの口を実装する。
pub trait Source: Sync {
    fn dimensions(&self) -> (u32, u32);
    fn pixel(&self, x: u32, y: u32) -> crate::Rgba8;
    /// 行 `y` の `x0` から `out.len() / 4` 画素を RGBA8 で `out` に書く。既定は `pixel` の繰り返し（読む順は左から右）で、連続した画像は
    /// 行をまとめてコピーできる。
    fn read_row(&self, x0: u32, y: u32, out: &mut [u8]) {
        for (i, d) in out.chunks_exact_mut(4).enumerate() {
            d.copy_from_slice(&self.pixel(x0 + i as u32, y).to_array());
        }
    }
}
#[derive(Clone, Copy)]
pub struct Image<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
}
impl<'a> Image<'a> {
    pub fn new(data: &'a [u8], width: u32, height: u32) -> Result<Self, Error> {
        if width == 0
            || height == 0
            || u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                != Some(data.len() as u64)
        {
            return Err(Error::Invalid("画像は幅×高さ×4バイト必要です"));
        }
        Ok(Self {
            data,
            width,
            height,
        })
    }
}
impl Source for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> crate::Rgba8 {
        crate::Rgba8::from_slice(&self.data[(y as usize * self.width as usize + x as usize) * 4..])
    }
    fn read_row(&self, x0: u32, y: u32, out: &mut [u8]) {
        let start = (y as usize * self.width as usize + x0 as usize) * 4;
        out.copy_from_slice(&self.data[start..start + out.len()]);
    }
}
