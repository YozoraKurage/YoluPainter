//! 手続き型の Generator（ノイズ・グランジ）の設定と評価の計画。Rust 版だけの種類（`Kind::Noise`・`Kind::Grunge`。C# の番号と重ならない
//! 64 から）で、マップを読まずに位置から値を作る。
//!
//! - 位置（メッシュマップの Position）で 3D のまま評価すると、UV の島の継ぎ目で模様がずれない。2D の模様（布目・指紋）は
//!   トライプラナー（面の向きで 3 方向の平面の模様を混ぜる。塗りつぶしの層の投影と同じ重み）で評価する。
//! - 位置のマップが使えない（無い・古い・大きさが違う・ピンと違う・境界箱が 0）ときは入力のまま通さず、UV 空間に落とす。
//!   UV では x・y の格子を周期で巻くので、テクスチャの端で継ぎ目が出ない（回転は効かない）。理由は [`super::BoundGenerator::fallback`]。
//! - 式は + − × ÷ sqrt floor と整数だけ（libm を使わない）。同じ設定・シード・マップなら、スレッド数・領域の切り方に依らず同じバイト。
use super::{
    grunge,
    noisefn::{cell_hash, perlin3, sin_cos_deg, unit24, value3, worley3, Cells, MAX_OCTAVES},
    unit, Error, Inactive, Kind, Map, MapKind, MapState,
};

/// 評価する空間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProceduralSpace {
    /// 位置のマップから 3D で評価する（継ぎ目が出ない。2D の模様のプリセットは自動でトライプラナーになる）。
    Position = 0,
    /// 位置と向きのマップから、面の向きで 3 つの平面の評価を混ぜる。
    Triplanar = 1,
    /// UV（テクスチャ）の空間で、周期を巻いて評価する。
    Uv = 2,
}
impl ProceduralSpace {
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::Position,
            1 => Self::Triplanar,
            2 => Self::Uv,
            _ => return None,
        })
    }
}
/// ノイズの基底。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NoiseBasis {
    Value = 0,
    Perlin = 1,
    Worley = 2,
}
impl NoiseBasis {
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::Value,
            1 => Self::Perlin,
            2 => Self::Worley,
            _ => return None,
        })
    }
}
/// Worley が返す量。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CellOutput {
    /// 最も近い点までの距離。
    F1 = 0,
    /// 2 番目に近い点までの距離。
    F2 = 1,
    /// 2 番目と最も近い点の距離の差（セルの境目が 0）。
    F2MinusF1 = 2,
}
impl CellOutput {
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::F1,
            1 => Self::F2,
            2 => Self::F2MinusF1,
            _ => return None,
        })
    }
}
/// オクターブの重ね方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FractalMode {
    /// 基底をそのまま重ねる（fBm）。
    Fbm = 0,
    /// 中央からの隔たりを反転して尾根にする（ridged）。
    Ridged = 1,
    /// 中央からの隔たりをそのまま重ねる（turbulence）。
    Turbulence = 2,
}
impl FractalMode {
    pub fn from_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::Fbm,
            1 => Self::Ridged,
            2 => Self::Turbulence,
            _ => return None,
        })
    }
}
/// グランジのプリセット（ノイズの組み合わせ）。値は 1 が「汚れ・傷などがある」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GrungePreset {
    /// 汚れの斑。
    Stain = 0,
    /// 錆の斑。
    Rust = 1,
    /// 傷の筋（2D の模様）。
    Scratches = 2,
    /// ほこり。
    Dust = 3,
    /// 指紋（2D の模様）。
    Fingerprints = 4,
    /// 布目（2D の模様）。
    Weave = 5,
    /// ひび。
    Cracks = 6,
    /// 飛沫。
    Splatter = 7,
    /// 塗装の剥げ。
    Peeling = 8,
    /// 木目（年輪と繊維）。
    WoodGrain = 9,
    /// 革のしぼ。
    Pebbles = 10,
}
impl GrungePreset {
    pub const ALL: [GrungePreset; 11] = [
        Self::Stain,
        Self::Rust,
        Self::Scratches,
        Self::Dust,
        Self::Fingerprints,
        Self::Weave,
        Self::Cracks,
        Self::Splatter,
        Self::Peeling,
        Self::WoodGrain,
        Self::Pebbles,
    ];
    pub fn from_index(i: i64) -> Option<Self> {
        usize::try_from(i)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }
    /// 2D の模様か（位置で評価するときは自動でトライプラナーにする）。
    pub fn is_planar(self) -> bool {
        matches!(self, Self::Fingerprints | Self::Weave | Self::Scratches)
    }
    /// プリセットを選んだときの模様の大きさ（モデルの境界箱の対角線・UV の幅に対する割合）。
    pub fn default_scale(self) -> f64 {
        match self {
            Self::Stain => 0.25,
            Self::Rust => 0.2,
            Self::Scratches => 0.2,
            Self::Dust => 0.1,
            Self::Fingerprints => 0.12,
            Self::Weave => 0.1,
            Self::Cracks => 0.15,
            Self::Splatter => 0.15,
            Self::Peeling => 0.25,
            Self::WoodGrain => 0.2,
            Self::Pebbles => 0.04,
        }
    }
    /// プリセットを選んだときのレベル（low・high。しきい値）。剥げは場の値を返すので、既定で境目のしきい値を持つ。
    pub fn default_levels(self) -> (f64, f64) {
        match self {
            Self::Peeling => (0.52, 0.535),
            _ => (0., 1.),
        }
    }
    /// 保存・画面の名前に使う英語の識別子。
    pub fn id(self) -> &'static str {
        match self {
            Self::Stain => "stain",
            Self::Rust => "rust",
            Self::Scratches => "scratches",
            Self::Dust => "dust",
            Self::Fingerprints => "fingerprints",
            Self::Weave => "weave",
            Self::Cracks => "cracks",
            Self::Splatter => "splatter",
            Self::Peeling => "peeling",
            Self::WoodGrain => "wood_grain",
            Self::Pebbles => "pebbles",
        }
    }
}

/// ノイズ・グランジの設定（`Settings::procedural`）。レベル（low・high・softness・invert）と合成（blend）は `Settings` のものを使う。
/// ノイズの欄（基底・重ね方・オクターブ・ラクナリティ・ゲイン）は `Kind::Noise` だけ、`preset` は `Kind::Grunge` だけが使い、
/// 使わない種類では既定のままにする（`Settings::validate`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Procedural {
    pub space: ProceduralSpace,
    /// 模様の大きさ（モデルの境界箱の対角線・UV のテクスチャの長い辺に対する 1 セルの割合。0.001〜1）。
    pub scale: f64,
    pub seed: i32,
    /// 位置の空間での回転（度。x・y・z の順に適用）。UV では効かない。
    pub rotation: [f64; 3],
    /// にじみ（0〜1）。模様の座標をゆがめて、縁を不規則にする。
    pub bleed: f64,
    /// トライプラナーの境目の幅（0〜1。塗りつぶしの投影と同じ意味）。
    pub blend_width: f64,
    pub basis: NoiseBasis,
    pub cell_output: CellOutput,
    pub fractal: FractalMode,
    /// 重ねる数（1〜8）。
    pub octaves: u32,
    /// 次のオクターブの周波数の倍率（1〜4）。
    pub lacunarity: f64,
    /// 次のオクターブの振幅の倍率（0〜1）。
    pub gain: f64,
    pub preset: GrungePreset,
}
impl Default for Procedural {
    fn default() -> Self {
        Self {
            space: ProceduralSpace::Position,
            scale: 0.1,
            seed: 0,
            rotation: [0.; 3],
            bleed: 0.,
            blend_width: 0.3,
            basis: NoiseBasis::Perlin,
            cell_output: CellOutput::F1,
            fractal: FractalMode::Fbm,
            octaves: 5,
            lacunarity: 2.,
            gain: 0.5,
            preset: GrungePreset::Stain,
        }
    }
}
impl Procedural {
    /// プリセットを選んだ状態（模様の大きさはプリセットの既定、ほかは既定）。
    pub fn for_preset(preset: GrungePreset) -> Self {
        Self {
            preset,
            scale: preset.default_scale(),
            ..Self::default()
        }
    }
    pub(super) fn validate(&self, kind: Kind) -> Result<(), Error> {
        let bad = |why| Err(Error::Invalid(why));
        if !self.scale.is_finite() || !(0.001..=1.).contains(&self.scale) {
            return bad("ノイズの大きさが範囲外です");
        }
        if self
            .rotation
            .iter()
            .any(|r| !r.is_finite() || r.abs() > 360.)
        {
            return bad("ノイズの回転が範囲外です");
        }
        if !unit(self.bleed) || !unit(self.blend_width) {
            return bad("ノイズのにじみ・境目の幅が範囲外です");
        }
        if !(1..=MAX_OCTAVES).contains(&self.octaves)
            || !self.lacunarity.is_finite()
            || !(1. ..=4.).contains(&self.lacunarity)
            || !unit(self.gain)
        {
            return bad("ノイズのオクターブ・ラクナリティ・ゲインが範囲外です");
        }
        let d = Self::default();
        if kind != Kind::Noise
            && (self.basis != d.basis
                || self.cell_output != d.cell_output
                || self.fractal != d.fractal
                || self.octaves != d.octaves
                || self.lacunarity != d.lacunarity
                || self.gain != d.gain)
        {
            return bad("基底・重ね方・オクターブ・ラクナリティ・ゲインはノイズ専用です");
        }
        if kind == Kind::Noise && self.preset != d.preset {
            return bad("プリセットはグランジ専用です");
        }
        if self.basis != NoiseBasis::Worley && self.cell_output != d.cell_output {
            return bad("セルの出力は Worley 専用です");
        }
        Ok(())
    }
    /// 評価が読むマップ（空間の設定だけで決まる。使えないときは UV に落とす）。
    pub(super) fn maps(&self, kind: Kind) -> Vec<MapKind> {
        match self.effective_space(kind) {
            ProceduralSpace::Uv => vec![],
            ProceduralSpace::Position => vec![MapKind::Position],
            ProceduralSpace::Triplanar => vec![MapKind::Position, MapKind::WorldNormal],
        }
    }
    /// 実際に使う空間（2D の模様のプリセットを位置で評価するときはトライプラナー）。
    pub(super) fn effective_space(&self, kind: Kind) -> ProceduralSpace {
        if self.space == ProceduralSpace::Position
            && kind == Kind::Grunge
            && self.preset.is_planar()
        {
            ProceduralSpace::Triplanar
        } else {
            self.space
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    Space3,
    Triplanar,
    Uv,
}

/// 層（基底とオクターブの重ね）の指定。`freq` は基本の模様の大きさに対する周波数の倍率（軸ごと）、`slot` は層ごとに違う乱数の区別。
#[derive(Clone, Copy)]
pub(super) struct Layer {
    pub basis: NoiseBasis,
    pub cell: CellOutput,
    pub fractal: FractalMode,
    pub octaves: u32,
    pub lacunarity: f64,
    pub gain: f64,
    pub freq: [f64; 3],
    pub slot: u32,
}
impl Layer {
    pub const fn new(
        basis: NoiseBasis,
        fractal: FractalMode,
        octaves: u32,
        freq: f64,
        slot: u32,
    ) -> Self {
        Self {
            basis,
            cell: CellOutput::F1,
            fractal,
            octaves,
            lacunarity: 2.,
            gain: 0.5,
            freq: [freq; 3],
            slot,
        }
    }
    pub const fn aniso(mut self, freq: [f64; 3]) -> Self {
        self.freq = freq;
        self
    }
}
/// 1 回の基底の評価の座標・周期・乱数。
pub(super) struct Frame {
    pub p: [f64; 3],
    pub per: [i32; 2],
    pub seed: u32,
}
/// Worley の出力の正規化（0..1 に広げる倍率）。
const F1_SPREAD: f64 = 1.05;
const F2_SPREAD: f64 = 0.7;
const F21_SPREAD: f64 = 1.5;

/// 評価の計画。設定とマップから束縛のときに作る。
pub(super) struct Plan {
    pub p: Procedural,
    pub kind: Kind,
    pub mode: Mode,
    /// 位置のマップが使えなくて UV に落としている理由。
    pub fallback: Option<Inactive>,
    rot: [f64; 9],
    unit: [f64; 3],
    counts: [i32; 2],
    seed: u32,
}
impl Plan {
    pub fn new(
        g: &super::Settings,
        maps: &[Option<&Map<'_>>; 10],
        dimensions: (u32, u32),
    ) -> Result<Self, Error> {
        let p = g.procedural;
        let wanted = p.effective_space(g.kind);
        let mut mode = match wanted {
            ProceduralSpace::Position => Mode::Space3,
            ProceduralSpace::Triplanar => Mode::Triplanar,
            ProceduralSpace::Uv => Mode::Uv,
        };
        let mut fallback = None;
        let mut unit3 = [0.; 3];
        if mode != Mode::Uv {
            for kind in p.maps(g.kind) {
                let why = match maps[kind as usize] {
                    None => Some(Inactive::MissingMap(kind)),
                    Some(m) if m.state == MapState::Stale => Some(Inactive::StaleMap(kind)),
                    Some(m) if m.state == MapState::Unverified => {
                        Some(Inactive::UnverifiedMap(kind))
                    }
                    Some(m) if (m.width, m.height) != dimensions => Some(Inactive::MapSize(kind)),
                    Some(m) if g.pins.get(&kind).is_some_and(|k| k != m.condition_key) => {
                        Some(Inactive::PinMismatch(kind))
                    }
                    Some(_) => None,
                };
                if why.is_some() {
                    fallback = why;
                    break;
                }
            }
            if fallback.is_none() {
                let m = maps[MapKind::Position as usize].expect("検査済み");
                let e = std::array::from_fn::<_, 3, _>(|i| m.bounds_max[i] - m.bounds_min[i]);
                let diag = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
                if !diag.is_finite() {
                    return Err(Error::Invalid("Position の境界箱が大きすぎます"));
                }
                if diag <= 0. {
                    fallback = Some(Inactive::EmptyBounds);
                } else {
                    unit3 = e.map(|e| e / diag / p.scale / 65535.);
                }
            }
            if fallback.is_some() {
                mode = Mode::Uv;
            }
        }
        // UV の周期: 長い辺を 1/scale 個（四捨五入）に、短い辺を同じ大きさのセルになる個数に
        let (w, h) = (dimensions.0 as f64, dimensions.1 as f64);
        let n = (1. / p.scale).round().max(1.);
        let short = |s: f64, l: f64| (n * s / l).round().max(1.);
        let counts = if w >= h {
            [n as i32, short(h, w) as i32]
        } else {
            [short(w, h) as i32, n as i32]
        };
        let (sx, cx) = sin_cos_deg(p.rotation[0]);
        let (sy, cy) = sin_cos_deg(p.rotation[1]);
        let (sz, cz) = sin_cos_deg(p.rotation[2]);
        // R = Rz · Ry · Rx
        let rot = [
            cz * cy,
            cz * sy * sx - sz * cx,
            cz * sy * cx + sz * sx,
            sz * cy,
            sz * sy * sx + cz * cx,
            sz * sy * cx - cz * sx,
            -sy,
            cy * sx,
            cy * cx,
        ];
        Ok(Self {
            p,
            kind: g.kind,
            mode,
            fallback,
            rot,
            unit: unit3,
            counts,
            seed: super::noisefn::hash(p.seed as u32 ^ 0x9e37_79b9),
        })
    }

    pub fn needs_position(&self) -> bool {
        self.mode != Mode::Uv
    }

    /// 位置（Position マップの生の値）と向き（WorldNormal の生の値）、画素の位置から、0..1 の値。
    pub fn value(
        &self,
        x: u32,
        y: u32,
        size: (u32, u32),
        position: Option<[f64; 3]>,
        normal: Option<[f64; 3]>,
    ) -> Option<f64> {
        let v = match self.mode {
            Mode::Uv => self.recipe([
                (x as f64 + 0.5) / size.0 as f64 * self.counts[0] as f64,
                (y as f64 + 0.5) / size.1 as f64 * self.counts[1] as f64,
                0.,
            ]),
            Mode::Space3 => self.recipe(self.rotated(position?)),
            Mode::Triplanar => {
                let r = self.rotated(position?);
                let n = normal?.map(|v| v / 65535. * 2. - 1.);
                let a = n.map(f64::abs);
                let most = a[0].max(a[1].max(a[2]));
                if most <= 1e-9 {
                    return None;
                }
                let cut = (1. - self.p.blend_width) * most;
                let mut weights = a.map(|v| (v - cut).max(0.));
                let mut total = weights[0] + weights[1] + weights[2];
                if total <= 0. {
                    weights = a.map(|v| if v >= most { 1. } else { 0. });
                    total = weights[0] + weights[1] + weights[2];
                }
                let mut sum = 0.;
                for axis in 0..3 {
                    let weight = weights[axis] / total;
                    if weight <= 0. {
                        continue;
                    }
                    let positive = n[axis] >= 0.;
                    let [x, y, z] = r;
                    let (s, t) = match axis {
                        0 => (if positive { z } else { -z }, y),
                        1 => (if positive { x } else { -x }, z),
                        _ => (if positive { -x } else { x }, y),
                    };
                    sum += weight * self.recipe([s, t, 17. * axis as f64]);
                }
                sum
            }
        };
        Some(v.clamp(0., 1.))
    }

    /// Position の生の値を、モデルの大きさに依らない模様の単位（1 = 基本のセル）の回転した座標に。
    fn rotated(&self, p: [f64; 3]) -> [f64; 3] {
        let w = [
            p[0] * self.unit[0],
            p[1] * self.unit[1],
            p[2] * self.unit[2],
        ];
        let r = &self.rot;
        [
            r[0] * w[0] + r[1] * w[1] + r[2] * w[2],
            r[3] * w[0] + r[4] * w[1] + r[5] * w[2],
            r[6] * w[0] + r[7] * w[1] + r[8] * w[2],
        ]
    }

    fn recipe(&self, b: [f64; 3]) -> f64 {
        let b = self.warp(b);
        match self.kind {
            Kind::Noise => {
                let l = Layer {
                    basis: self.p.basis,
                    cell: self.p.cell_output,
                    fractal: self.p.fractal,
                    octaves: self.p.octaves,
                    lacunarity: self.p.lacunarity,
                    gain: self.p.gain,
                    freq: [1.; 3],
                    slot: 0,
                };
                self.layer(b, &l)
            }
            _ => grunge::eval(self, b, self.p.preset),
        }
    }

    /// にじみ: 座標を低周波のノイズでずらす（UV では周期を保つ）。
    fn warp(&self, b: [f64; 3]) -> [f64; 3] {
        let amount = self.p.bleed;
        if amount <= 0. {
            return b;
        }
        let a = 3.2 * amount;
        let l = |slot| Layer::new(NoiseBasis::Value, FractalMode::Fbm, 2, 0.5, slot);
        [
            b[0] + (self.layer(b, &l(WARP_SLOT)) - 0.5) * a,
            b[1] + (self.layer(b, &l(WARP_SLOT + 1)) - 0.5) * a,
            if self.mode == Mode::Uv {
                b[2]
            } else {
                b[2] + (self.layer(b, &l(WARP_SLOT + 2)) - 0.5) * a
            },
        ]
    }

    /// UV での周期を `m` 倍した格子（位置の空間では巻かない）。
    pub fn period(&self, m: i32) -> [i32; 2] {
        if self.mode == Mode::Uv {
            [self.counts[0] * m, self.counts[1] * m]
        } else {
            [0, 0]
        }
    }
    pub fn is_uv(&self) -> bool {
        self.mode == Mode::Uv
    }

    /// `b`（基本のセル単位）を `freq`・`slot`・`octave` の格子の座標へ。
    pub fn frame(&self, b: [f64; 3], freq: [f64; 3], slot: u32, octave: u32) -> Frame {
        let seed = super::noisefn::hash(
            self.seed
                .wrapping_add(slot.wrapping_mul(0x632b_e5ab))
                .wrapping_add(octave.wrapping_mul(0x85eb_ca6b)),
        );
        if self.mode == Mode::Uv {
            let c = |a: usize| (self.counts[a] as f64 * freq[a]).round().max(1.);
            let (cx, cy) = (c(0), c(1));
            // z は格子の面（整数）を避ける: 面の上では 2D の勾配が軸に揃って、格子の筋が出る
            let z = 0.25 + 0.5 * unit24(super::noisefn::hash(seed ^ 0x2545_f491));
            Frame {
                p: [
                    b[0] * (cx / self.counts[0] as f64),
                    b[1] * (cy / self.counts[1] as f64),
                    z,
                ],
                per: [cx as i32, cy as i32],
                seed,
            }
        } else {
            let h = super::noisefn::hash(seed ^ 0x51ed_270b);
            let off = [
                unit24(h) * 128.,
                unit24(super::noisefn::hash(h ^ 0x1b87_3593)) * 128.,
                unit24(super::noisefn::hash(h ^ 0xe654_3a2d)) * 128.,
            ];
            Frame {
                p: [
                    b[0] * freq[0] + off[0],
                    b[1] * freq[1] + off[1],
                    b[2] * freq[2] + off[2],
                ],
                per: [0, 0],
                seed,
            }
        }
    }

    pub fn cells(&self, f: &Frame) -> Cells {
        worley3(f.p, f.seed, f.per)
    }

    /// 基底 1 回の値（0..1）。
    pub fn basis_value(&self, basis: NoiseBasis, cell: CellOutput, f: &Frame) -> f64 {
        match basis {
            NoiseBasis::Value => value3(f.p, f.seed, f.per),
            NoiseBasis::Perlin => perlin3(f.p, f.seed, f.per),
            NoiseBasis::Worley => {
                let c = worley3(f.p, f.seed, f.per);
                match cell {
                    CellOutput::F1 => (c.f1 * F1_SPREAD).min(1.),
                    CellOutput::F2 => (c.f2 * F2_SPREAD).min(1.),
                    CellOutput::F2MinusF1 => ((c.f2 - c.f1) * F21_SPREAD).min(1.),
                }
            }
        }
    }

    /// 層（オクターブの重ね）の値。0..1。
    pub fn layer(&self, b: [f64; 3], l: &Layer) -> f64 {
        let (mut sum, mut total, mut amp, mut f) = (0., 0., 1., 1.);
        for o in 0..l.octaves {
            let fr = self.frame(b, [l.freq[0] * f, l.freq[1] * f, l.freq[2] * f], l.slot, o);
            let n = self.basis_value(l.basis, l.cell, &fr);
            let s = match l.fractal {
                FractalMode::Fbm => n,
                FractalMode::Ridged => {
                    let r = 1. - (2. * n - 1.).abs();
                    r * r
                }
                FractalMode::Turbulence => (2. * n - 1.).abs(),
            };
            sum += amp * s;
            total += amp;
            amp *= l.gain;
            f *= l.lacunarity;
        }
        sum / total
    }

    /// 整数の格子の乱数（0..1）。UV では周期で巻く。
    pub fn lattice(&self, slot: u32, x: i32, y: i32, per: [i32; 2]) -> f64 {
        let seed = self.seed.wrapping_add(slot.wrapping_mul(0x632b_e5ab));
        unit24(cell_hash(
            seed,
            super::noisefn::wrap(x, per[0]),
            super::noisefn::wrap(y, per[1]),
            3,
        ))
    }
}
const WARP_SLOT: u32 = 40;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::Settings;

    fn every_setting() -> Vec<(String, Settings)> {
        let mut v = Vec::new();
        for basis in [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley] {
            for fractal in [
                FractalMode::Fbm,
                FractalMode::Ridged,
                FractalMode::Turbulence,
            ] {
                for cell in [CellOutput::F1, CellOutput::F2, CellOutput::F2MinusF1] {
                    let mut s = Settings::new(Kind::Noise);
                    s.procedural.basis = basis;
                    s.procedural.fractal = fractal;
                    s.procedural.cell_output = cell;
                    s.procedural.seed = 7;
                    v.push((format!("noise-{basis:?}-{fractal:?}-{cell:?}"), s));
                }
            }
        }
        for preset in GrungePreset::ALL {
            let mut s = Settings::grunge(preset);
            s.procedural.seed = 7;
            v.push((format!("grunge-{}", preset.id()), s));
        }
        v
    }

    fn uv_plan(s: &Settings, dimensions: (u32, u32)) -> Plan {
        let mut s = s.clone();
        s.procedural.space = ProceduralSpace::Uv;
        let plan = Plan::new(&s, &[None; 10], dimensions).unwrap();
        assert!(plan.is_uv() && plan.fallback.is_none());
        plan
    }

    /// UV の模様は周期が `counts`（長い辺に 1/scale 個）で、x にも y にも 1 周期ずらすと同じ値になる（継ぎ目が出ない）。
    /// 画素の差の平均で見ると、細かい模様では巻き忘れが「もともと大きい差」に埋もれるので、式の値そのものを比べる。
    #[test]
    fn every_recipe_is_periodic_over_the_uv_counts_for_every_basis_and_preset() {
        let mut state = 0x2545_f491u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f64 / (1u32 << 24) as f64
        };
        for (name, base) in every_setting() {
            // 比べる相手: 半周期ずらすと、どこかでは大きく違う値になる（式が定数や、周期より短い繰り返しではない）
            let mut most_different = 0f64;
            for scale in [0.07, 0.1, 0.25, 0.5] {
                for bleed in [0., 0.5] {
                    for dimensions in [(64u32, 64u32), (96, 40), (40, 96)] {
                        let mut s = base.clone();
                        s.procedural.scale = scale;
                        s.procedural.bleed = bleed;
                        let plan = uv_plan(&s, dimensions);
                        let (nx, ny) = (plan.counts[0] as f64, plan.counts[1] as f64);
                        for _ in 0..40 {
                            let b = [next() * nx, next() * ny, 0.];
                            let here = plan.recipe(b);
                            let across_x = plan.recipe([b[0] + nx, b[1], 0.]);
                            let across_y = plan.recipe([b[0], b[1] + ny, 0.]);
                            let across_both = plan.recipe([b[0] + nx, b[1] + ny, 0.]);
                            for (what, v) in [("x", across_x), ("y", across_y), ("xy", across_both)]
                            {
                                // 足し算の丸めで小数部が 1 ULP ずれ得るので、ビットでなく近さで比べる
                                assert!(
                                    (here - v).abs() < 1e-6,
                                    "{name} scale {scale} bleed {bleed} {dimensions:?}: {what} に 1 周期ずらすと {here} が {v} になる（{b:?}）"
                                );
                            }
                            for half in [[nx * 0.5, 0.], [0., ny * 0.5]] {
                                let moved = plan.recipe([b[0] + half[0], b[1] + half[1], 0.]);
                                most_different = most_different.max((here - moved).abs());
                            }
                        }
                    }
                }
            }
            assert!(
                most_different > 0.2,
                "{name}: 半周期ずらしても変わらない（{most_different}）"
            );
        }
    }

    /// 位置の空間の回転は、R = Rz · Ry · Rx。z まわりに 90 度で x と y が入れ替わる（x' = −y、y' = x）。360 度は 0 度と同じ。
    #[test]
    fn rotation_by_90_degrees_around_z_swaps_x_and_y() {
        let plan = |rotation: [f64; 3]| {
            let mut s = Settings::new(Kind::Noise);
            s.procedural.rotation = rotation;
            let mut plan = uv_plan(&s, (32, 32));
            // UV の計画は位置の単位を持たない（0）ので、回転だけを見られるように 1 にする
            plan.unit = [1.; 3];
            plan
        };
        let samples = [[0.3, -1.2, 2.5], [4.0, 0.5, -0.75], [-2.0, 3.0, 1.0]];
        let (turned, back, same) = (
            plan([0., 0., 90.]),
            plan([0., 0., 360.]),
            plan([0., 0., 0.]),
        );
        for p in samples {
            let r = turned.rotated(p);
            for (got, want) in r.iter().zip([-p[1], p[0], p[2]]) {
                assert!((got - want).abs() < 1e-12, "{p:?} → {r:?}");
            }
            for (got, want) in back.rotated(p).iter().zip(same.rotated(p)) {
                assert!((got - want).abs() < 1e-12, "{p:?}");
            }
        }
    }
}
