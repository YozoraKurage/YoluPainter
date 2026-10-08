//! 手続き型の Generator（ノイズ・グランジ）の設定と評価の計画。Rust 版だけの種類（`Kind::Noise`・`Kind::Grunge`。C# の番号と重ならない
//! 64 から）で、マップを読まずに位置から値を作る。
//!
//! - 位置（メッシュマップの Position）で 3D のまま評価すると、UV アイランドの継ぎ目で模様がずれない。2D の模様（布目・指紋）は
//!   トライプラナー（面の向きで 3 方向の平面の模様を混ぜる。塗りつぶしレイヤーの投影と同じ重み）で評価する。
//! - 位置のマップが使えない（無い・古い・大きさが違う・ピンと違う・境界箱が 0）ときは入力のまま通さず、UV 空間に落とす。
//!   UV では x・y の格子を周期で巻くので、テクスチャの端で継ぎ目が出ない（回転は効かない）。理由は [`super::BoundGenerator::fallback`]。
//! - 式は + − × ÷ sqrt floor と整数だけ（libm を使わない）。同じ設定・シード・マップなら、スレッド数・領域の切り方に依らず同じバイト。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::noisefn::{
    perlin3_lanes, value3_lanes, worley3_lanes, worley_distances_lanes, CellsLanes,
};
use super::{
    grunge,
    noisefn::{
        cell_hash, perlin3, sin_cos_deg, unit24, value3, worley3, Cells, PerlinCell, ValueCell,
        WorleyCell, MAX_OCTAVES,
    },
    unit, Error, Inactive, Kind, Map, MapKind, MapState,
};
#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neon;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

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
        if !self.scale.is_finite() || !crate::ranges::NOISE_SCALE.contains(&self.scale) {
            return bad("ノイズの大きさが範囲外です");
        }
        if self
            .rotation
            .iter()
            .any(|r| !r.is_finite() || !crate::ranges::PROCEDURAL_ROTATION.contains(r))
        {
            return bad("ノイズの回転が範囲外です");
        }
        if !unit(self.bleed) || !unit(self.blend_width) {
            return bad("ノイズのにじみ・境目の幅が範囲外です");
        }
        if !(1..=MAX_OCTAVES).contains(&self.octaves)
            || !self.lacunarity.is_finite()
            || !crate::ranges::LACUNARITY.contains(&self.lacunarity)
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

/// レイヤー（基底とオクターブの重ね）の指定。`freq` は基本の模様の大きさに対する周波数の倍率（軸ごと）、`slot` はレイヤーごとに違う乱数の区別。
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
/// Worley の出力の正規化（0..1 に広げる倍率）。
const F1_SPREAD: f64 = 1.05;
const F2_SPREAD: f64 = 0.7;
const F21_SPREAD: f64 = 1.5;

/// 1 回の基底の評価の座標の写し方・周期・乱数と、直前の格子を覚える場所の番号（`CellSet` の中の添字）。
/// レイヤーのオクターブごと・セルの枠ごとに、束縛のときに 1 回だけ作る（画素ごとには変わらない値）。
#[derive(Clone, Copy)]
pub(super) struct OctavePlan {
    seed: u32,
    per: [i32; 2],
    /// 座標 `b`（基本のセル単位）から格子の座標への倍率。
    mul: [f64; 3],
    /// UV では z（格子の面を避けた定数）、位置の空間では 3 軸のずらし。
    add: [f64; 3],
    slot: usize,
}
impl OctavePlan {
    const EMPTY: Self = Self {
        seed: 0,
        per: [0; 2],
        mul: [0.; 3],
        add: [0.; 3],
        slot: 0,
    };
    /// `b`（基本のセル単位）の格子の座標。
    #[inline(always)]
    pub(super) fn point(&self, b: [f64; 3], uv: bool) -> [f64; 3] {
        if uv {
            [b[0] * self.mul[0], b[1] * self.mul[1], self.add[2]]
        } else {
            [
                b[0] * self.mul[0] + self.add[0],
                b[1] * self.mul[1] + self.add[1],
                b[2] * self.mul[2] + self.add[2],
            ]
        }
    }
}
impl OctavePlan {
    /// `point` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn point_lanes<V: Lanes>(&self, b: [V::F; 3], uv: bool) -> [V::F; 3] {
        if uv {
            [
                V::mul(b[0], V::splat(self.mul[0])),
                V::mul(b[1], V::splat(self.mul[1])),
                V::splat(self.add[2]),
            ]
        } else {
            [
                V::add(V::mul(b[0], V::splat(self.mul[0])), V::splat(self.add[0])),
                V::add(V::mul(b[1], V::splat(self.mul[1])), V::splat(self.add[1])),
                V::add(V::mul(b[2], V::splat(self.mul[2])), V::splat(self.add[2])),
            ]
        }
    }
}
/// レイヤーの計画（オクターブごとの `OctavePlan` と、重みの並び・その合計）。
pub(super) struct LayerPlan {
    basis: NoiseBasis,
    cell: CellOutput,
    fractal: FractalMode,
    n: usize,
    octaves: [OctavePlan; MAX_OCTAVES as usize],
    amps: [f64; MAX_OCTAVES as usize],
    total: f64,
}
/// 直前の格子を覚える場所の数（基底ごと）。
#[derive(Clone, Copy, Default)]
struct Slots {
    value: usize,
    perlin: usize,
    worley: usize,
    segments: usize,
}

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
    layers: Vec<LayerPlan>,
    cells: Vec<OctavePlan>,
    segments: Vec<(OctavePlan, grunge::SegSpec)>,
    /// にじみの 3 レイヤーの最初の番号（にじみが無ければ使わない）。
    warp: usize,
    slots: Slots,
    /// 計画ごとに違う番号（格子の覚えが、別の計画の乱数で引いた値を使い回さないための印）。
    id: u64,
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
        let mut plan = Self {
            p,
            kind: g.kind,
            mode,
            fallback,
            rot,
            unit: unit3,
            counts,
            seed: super::noisefn::hash(p.seed as u32 ^ 0x9e37_79b9),
            layers: Vec::new(),
            cells: Vec::new(),
            segments: Vec::new(),
            warp: usize::MAX,
            slots: Slots::default(),
            id: NEXT_PLAN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        };
        plan.prepare();
        Ok(plan)
    }

    /// 種類・プリセットが使うレイヤー・セルの枠・線分の枠の計画を作る（画素ごとには変わらない乱数・座標の写し方を先に出す）。
    fn prepare(&mut self) {
        let p = self.p;
        if self.kind == Kind::Noise {
            self.add_layer(&Layer {
                basis: p.basis,
                cell: p.cell_output,
                fractal: p.fractal,
                octaves: p.octaves,
                lacunarity: p.lacunarity,
                gain: p.gain,
                freq: [1.; 3],
                slot: 0,
            });
        } else {
            let spec = grunge::spec(p.preset);
            for l in spec.layers {
                self.add_layer(l);
            }
            for c in spec.cells {
                let slot = self.slots.worley;
                self.slots.worley += 1;
                let oc = self.octave(c.freq, c.slot, 0, slot);
                self.cells.push(oc);
            }
            for s in spec.segments {
                let slot = self.slots.segments;
                self.slots.segments += 1;
                let oc = self.octave([s.freq, s.freq, 0.], s.slot, 0, slot);
                self.segments.push((oc, *s));
            }
        }
        if p.bleed > 0. {
            self.warp = self.layers.len();
            for k in 0..3 {
                self.add_layer(&Layer::new(
                    NoiseBasis::Value,
                    FractalMode::Fbm,
                    2,
                    0.5,
                    WARP_SLOT + k,
                ));
            }
        }
    }
    fn add_layer(&mut self, l: &Layer) {
        let mut octaves = [OctavePlan::EMPTY; MAX_OCTAVES as usize];
        let mut amps = [0.; MAX_OCTAVES as usize];
        let (mut total, mut amp, mut f) = (0., 1., 1.);
        for o in 0..l.octaves {
            let slot = match l.basis {
                NoiseBasis::Value => &mut self.slots.value,
                NoiseBasis::Perlin => &mut self.slots.perlin,
                NoiseBasis::Worley => &mut self.slots.worley,
            };
            let slot_index = *slot;
            *slot += 1;
            octaves[o as usize] = self.octave(
                [l.freq[0] * f, l.freq[1] * f, l.freq[2] * f],
                l.slot,
                o,
                slot_index,
            );
            amps[o as usize] = amp;
            total += amp;
            amp *= l.gain;
            f *= l.lacunarity;
        }
        self.layers.push(LayerPlan {
            basis: l.basis,
            cell: l.cell,
            fractal: l.fractal,
            n: l.octaves as usize,
            octaves,
            amps,
            total,
        });
    }
    /// 周波数 `freq`・レイヤーの区別 `slot`・オクターブ `octave` の格子の座標の写し方と乱数。
    fn octave(&self, freq: [f64; 3], slot: u32, octave: u32, cache_slot: usize) -> OctavePlan {
        use super::noisefn::{hash, unit24};
        let seed = hash(
            self.seed
                .wrapping_add(slot.wrapping_mul(0x632b_e5ab))
                .wrapping_add(octave.wrapping_mul(0x85eb_ca6b)),
        );
        if self.mode == Mode::Uv {
            let c = |a: usize| (self.counts[a] as f64 * freq[a]).round().max(1.);
            let (cx, cy) = (c(0), c(1));
            // z は格子の面（整数）を避ける: 面の上では 2D の勾配が軸に揃って、格子の筋が出る
            let z = 0.25 + 0.5 * unit24(hash(seed ^ 0x2545_f491));
            OctavePlan {
                seed,
                per: [cx as i32, cy as i32],
                mul: [cx / self.counts[0] as f64, cy / self.counts[1] as f64, 0.],
                add: [0., 0., z],
                slot: cache_slot,
            }
        } else {
            let h = hash(seed ^ 0x51ed_270b);
            OctavePlan {
                seed,
                per: [0, 0],
                mul: freq,
                add: [
                    unit24(h) * 128.,
                    unit24(hash(h ^ 0x1b87_3593)) * 128.,
                    unit24(hash(h ^ 0xe654_3a2d)) * 128.,
                ],
                slot: cache_slot,
            }
        }
    }

    /// 直前の格子を覚える場所の組の数（トライプラナーは軸ごとに別の座標をたどるので 3 組、ほかは 1 組）。
    fn sets(&self) -> usize {
        if self.mode == Mode::Triplanar {
            3
        } else {
            1
        }
    }
    /// 評価に使う作業領域（格子の覚え）。行・領域をたどるあいだ使い回す。
    pub fn scratch(&self) -> PlanScratch {
        PlanScratch {
            plan: self.id,
            sets: (0..self.sets())
                .map(|_| CellSet {
                    value: vec![ValueCell::EMPTY; self.slots.value],
                    perlin: vec![PerlinCell::EMPTY; self.slots.perlin],
                    worley: vec![WorleyCell::EMPTY; self.slots.worley],
                    segments: vec![grunge::SegCell::EMPTY; self.slots.segments],
                })
                .collect(),
        }
    }

    pub fn needs_position(&self) -> bool {
        self.mode != Mode::Uv
    }

    /// 位置（Position マップの生の値）と向き（WorldNormal の生の値）、画素の位置から、0..1 の値。
    /// `scratch` は同じ計画の `scratch()`（画素を隣へ進めるときは使い回す。覚えは格子の位置で照合するので、飛んだ画素でも値は変わらない）。
    pub fn value(
        &self,
        x: u32,
        y: u32,
        size: (u32, u32),
        position: Option<[f64; 3]>,
        normal: Option<[f64; 3]>,
        scratch: &mut PlanScratch,
    ) -> Option<f64> {
        let v = match self.mode {
            Mode::Uv => self.recipe(
                [
                    (x as f64 + 0.5) / size.0 as f64 * self.counts[0] as f64,
                    (y as f64 + 0.5) / size.1 as f64 * self.counts[1] as f64,
                    0.,
                ],
                &mut scratch.sets[0],
            ),
            Mode::Space3 => self.recipe(self.rotated(position?), &mut scratch.sets[0]),
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
                    sum += weight * self.recipe([s, t, 17. * axis as f64], &mut scratch.sets[axis]);
                }
                sum
            }
        };
        Some(v.clamp(0., 1.))
    }

    /// `value` の N 画素ぶん。画素 `x..x + N` は行 `y` の連続した画素（`position`・`normal` は生の値のレーン。位置を使わない空間では無視）。
    /// 三角面の向きが定まらない画素（`value` が `None` を返す画素）が 1 つでもあれば `None`（呼び手が 1 画素ずつ引く）。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn value_lanes<V: GenLanes>(
        &self,
        x: u32,
        y: u32,
        size: (u32, u32),
        position: [V::F; 3],
        normal: [V::F; 3],
        scratch: &mut PlanScratch,
    ) -> Option<V::F> {
        let v = match self.mode {
            Mode::Uv => {
                let (w, c) = (size.0 as f64, self.counts[0] as f64);
                let bx = V::from_fn(|k| ((x + k as u32) as f64 + 0.5) / w * c);
                let by = V::splat((y as f64 + 0.5) / size.1 as f64 * self.counts[1] as f64);
                V::recipe(self, [bx, by, V::splat(0.)], &mut scratch.sets[0])
            }
            Mode::Space3 => V::recipe(
                self,
                self.rotated_lanes::<V>(position),
                &mut scratch.sets[0],
            ),
            Mode::Triplanar => {
                let r = self.rotated_lanes::<V>(position);
                let (zero, one, two) = (V::splat(0.), V::splat(1.), V::splat(2.));
                let n = [
                    V::sub(V::mul(V::div(normal[0], V::splat(65535.)), two), one),
                    V::sub(V::mul(V::div(normal[1], V::splat(65535.)), two), one),
                    V::sub(V::mul(V::div(normal[2], V::splat(65535.)), two), one),
                ];
                let a = [V::abs(n[0]), V::abs(n[1]), V::abs(n[2])];
                let most = V::max(a[0], V::max(a[1], a[2]));
                if V::any(V::le(most, V::splat(1e-9))) {
                    return None;
                }
                let cut = V::mul(V::splat(1. - self.p.blend_width), most);
                let mut weights = [
                    V::max(V::sub(a[0], cut), zero),
                    V::max(V::sub(a[1], cut), zero),
                    V::max(V::sub(a[2], cut), zero),
                ];
                let mut total = V::add(V::add(weights[0], weights[1]), weights[2]);
                // 重みが全部 0 なら、いちばん大きい軸だけ
                let none = V::le(total, zero);
                if V::any(none) {
                    let top = [
                        V::select(V::ge(a[0], most), one, zero),
                        V::select(V::ge(a[1], most), one, zero),
                        V::select(V::ge(a[2], most), one, zero),
                    ];
                    let top_total = V::add(V::add(top[0], top[1]), top[2]);
                    for axis in 0..3 {
                        weights[axis] = V::select(none, top[axis], weights[axis]);
                    }
                    total = V::select(none, top_total, total);
                }
                let mut sum = zero;
                for axis in 0..3 {
                    let weight = V::div(weights[axis], total);
                    let used = V::gt(weight, zero);
                    if !V::any(used) {
                        continue;
                    }
                    let positive = V::ge(n[axis], zero);
                    let [rx, ry, rz] = r;
                    let (s, t) = match axis {
                        0 => (V::select(positive, rz, V::neg(rz)), ry),
                        1 => (V::select(positive, rx, V::neg(rx)), rz),
                        _ => (V::select(positive, V::neg(rx), rx), ry),
                    };
                    let value = V::recipe(
                        self,
                        [s, t, V::splat(17. * axis as f64)],
                        &mut scratch.sets[axis],
                    );
                    sum = V::select(used, V::add(sum, V::mul(weight, value)), sum);
                }
                sum
            }
        };
        Some(simd::clamp01::<V>(v))
    }

    /// `rotated` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    unsafe fn rotated_lanes<V: Lanes>(&self, p: [V::F; 3]) -> [V::F; 3] {
        let w = [
            V::mul(p[0], V::splat(self.unit[0])),
            V::mul(p[1], V::splat(self.unit[1])),
            V::mul(p[2], V::splat(self.unit[2])),
        ];
        let r = &self.rot;
        let mut out = [w[0]; 3];
        for (row, o) in out.iter_mut().enumerate() {
            *o = V::add(
                V::add(
                    V::mul(V::splat(r[row * 3]), w[0]),
                    V::mul(V::splat(r[row * 3 + 1]), w[1]),
                ),
                V::mul(V::splat(r[row * 3 + 2]), w[2]),
            );
        }
        out
    }

    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn recipe_body<V: GenLanes>(&self, b: [V::F; 3], set: &mut CellSet) -> V::F {
        let mut cx = Ctx { plan: self, set };
        let b = cx.warp_lanes::<V>(b);
        match self.kind {
            Kind::Noise => cx.layer_lanes::<V>(b, 0),
            _ => grunge::eval_lanes::<V>(&mut cx, b, self.p.preset),
        }
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

    fn recipe(&self, b: [f64; 3], set: &mut CellSet) -> f64 {
        let mut cx = Ctx { plan: self, set };
        let b = cx.warp(b);
        match self.kind {
            Kind::Noise => cx.layer(b, 0),
            _ => grunge::eval(&mut cx, b, self.p.preset),
        }
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
static NEXT_PLAN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// にじみのずらし量 `(レイヤーの値 - 0.5) * a`。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn shift_lanes<V: Lanes>(layer: V::F, a: V::F) -> V::F {
    V::mul(V::sub(layer, V::splat(0.5)), a)
}

/// Generator の重い式（ノイズのレイヤー・グランジの模様・1 つの値の式）の、道（AVX2・SSE4.1・NEON）ごとの入口。
///
/// 式の本体（`*_body`）は `#[inline(always)]` のレーンの式で、呼んだ所に丸ごと展開される。ノイズのレイヤーは 1 つの値の式の中で何十回も
/// 呼ばれ（にじみ・種類・グランジの各模様）、その全部を 1 つの関数に展開すると、LLVM の最適化が 1 関数で数十秒かかっていた。
/// ここで道ごとに `#[target_feature]` 付きの展開しない関数を 1 つずつ置き、呼び出しの側は関数の呼び出しになる（式も演算の順も
/// 変わらないので、値は同じ bit）。呼び出しは 1 回で N 画素ぶん（AVX2 は 4、SSE4.1・NEON は 2）。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) trait GenLanes: Lanes {
    unsafe fn layer(cx: &mut Ctx<'_>, b: [Self::F; 3], index: usize) -> Self::F;
    unsafe fn recipe(plan: &Plan, b: [Self::F; 3], set: &mut CellSet) -> Self::F;
    /// グランジの模様（模様ごとに 1 つの展開しない関数）。
    unsafe fn grunge(cx: &mut Ctx<'_>, b: [Self::F; 3], preset: GrungePreset) -> Self::F;
}

/// `GenLanes` を道の型に実装する（入口の関数は、その道の命令を有効にした展開しない関数）。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
macro_rules! gen_lanes {
    ($ty:ident, $feature:literal, $m:ident) => {
        impl GenLanes for $ty {
            #[inline(always)]
            unsafe fn grunge(cx: &mut Ctx<'_>, b: [Self::F; 3], preset: GrungePreset) -> Self::F {
                grunge::$m::dispatch(cx, b, preset)
            }
            #[inline(always)]
            unsafe fn layer(cx: &mut Ctx<'_>, b: [Self::F; 3], index: usize) -> Self::F {
                #[target_feature(enable = $feature)]
                #[inline(never)]
                unsafe fn entry(
                    cx: &mut Ctx<'_>,
                    b: [<$ty as Lanes>::F; 3],
                    index: usize,
                ) -> <$ty as Lanes>::F {
                    cx.layer_body::<$ty>(b, index)
                }
                entry(cx, b, index)
            }
            #[inline(always)]
            unsafe fn recipe(plan: &Plan, b: [Self::F; 3], set: &mut CellSet) -> Self::F {
                #[target_feature(enable = $feature)]
                #[inline(never)]
                unsafe fn entry(
                    plan: &Plan,
                    b: [<$ty as Lanes>::F; 3],
                    set: &mut CellSet,
                ) -> <$ty as Lanes>::F {
                    plan.recipe_body::<$ty>(b, set)
                }
                entry(plan, b, set)
            }
        }
    };
}
#[cfg(target_arch = "x86_64")]
gen_lanes!(Avx2, "avx2,fma", avx2);
#[cfg(target_arch = "x86_64")]
gen_lanes!(Sse41, "sse4.1", sse41);
#[cfg(target_arch = "aarch64")]
gen_lanes!(Neon, "neon", neon);

/// 1 組の格子の覚え（基底ごとに、レイヤーのオクターブ・セルの枠の数だけ）。
pub(super) struct CellSet {
    value: Vec<ValueCell>,
    perlin: Vec<PerlinCell>,
    worley: Vec<WorleyCell>,
    pub(super) segments: Vec<grunge::SegCell>,
}
/// `Plan::value` が使う作業領域。同じ計画の `Plan::scratch` から作り、隣の画素へ進むあいだ使い回す。
pub(super) struct PlanScratch {
    /// 作った計画の番号。
    plan: u64,
    sets: Vec<CellSet>,
}
impl PlanScratch {
    /// この計画のために作ったものか（覚えた格子は計画の乱数で引いたものなので、別の計画では使えない）。
    pub fn fits(&self, plan: &Plan) -> bool {
        self.plan == plan.id
    }
}

/// 式を評価する間の文脈（計画と、その軸の格子の覚え）。
pub(super) struct Ctx<'a> {
    pub(super) plan: &'a Plan,
    set: &'a mut CellSet,
}
impl Ctx<'_> {
    /// レイヤー（オクターブの重ね）の値。0..1。
    pub fn layer(&mut self, b: [f64; 3], index: usize) -> f64 {
        let plan = self.plan;
        let layer = &plan.layers[index];
        let uv = plan.mode == Mode::Uv;
        let mut sum = 0.;
        for o in 0..layer.n {
            let oc = &layer.octaves[o];
            let q = oc.point(b, uv);
            let n = match layer.basis {
                NoiseBasis::Value => value3(q, oc.seed, oc.per, &mut self.set.value[oc.slot]),
                NoiseBasis::Perlin => perlin3(q, oc.seed, oc.per, &mut self.set.perlin[oc.slot]),
                NoiseBasis::Worley => {
                    let c = worley3(q, oc.seed, oc.per, &mut self.set.worley[oc.slot]);
                    match layer.cell {
                        CellOutput::F1 => (c.f1 * F1_SPREAD).min(1.),
                        CellOutput::F2 => (c.f2 * F2_SPREAD).min(1.),
                        CellOutput::F2MinusF1 => ((c.f2 - c.f1) * F21_SPREAD).min(1.),
                    }
                }
            };
            let s = match layer.fractal {
                FractalMode::Fbm => n,
                FractalMode::Ridged => {
                    let r = 1. - (2. * n - 1.).abs();
                    r * r
                }
                FractalMode::Turbulence => (2. * n - 1.).abs(),
            };
            sum += layer.amps[o] * s;
        }
        sum / layer.total
    }

    /// セルの枠 `index` の Worley の結果と、その枠の格子の座標。
    pub fn cells(&mut self, b: [f64; 3], index: usize) -> (Cells, [f64; 3]) {
        let plan = self.plan;
        let oc = &plan.cells[index];
        let q = oc.point(b, plan.mode == Mode::Uv);
        let c = worley3(q, oc.seed, oc.per, &mut self.set.worley[oc.slot]);
        (c, q)
    }

    /// 線分の枠 `index` の、画素 `b` で最も濃い線分の値（0..1）。
    pub fn segments(&mut self, b: [f64; 3], index: usize) -> f64 {
        let plan = self.plan;
        let (oc, spec) = &plan.segments[index];
        let q = oc.point(b, plan.mode == Mode::Uv);
        grunge::segments(q, oc.seed, oc.per, spec, &mut self.set.segments[oc.slot])
    }

    /// `layer` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn layer_body<V: Lanes>(&mut self, b: [V::F; 3], index: usize) -> V::F {
        let plan = self.plan;
        let layer = &plan.layers[index];
        let uv = plan.mode == Mode::Uv;
        let mut sum = V::splat(0.);
        for o in 0..layer.n {
            let oc = &layer.octaves[o];
            let q = oc.point_lanes::<V>(b, uv);
            let n = match layer.basis {
                NoiseBasis::Value => {
                    value3_lanes::<V>(q, oc.seed, oc.per, &mut self.set.value[oc.slot])
                }
                NoiseBasis::Perlin => {
                    perlin3_lanes::<V>(q, oc.seed, oc.per, &mut self.set.perlin[oc.slot])
                }
                NoiseBasis::Worley => {
                    let (f1, f2) = worley_distances_lanes::<V>(
                        q,
                        oc.seed,
                        oc.per,
                        &mut self.set.worley[oc.slot],
                    );
                    let one = V::splat(1.);
                    match layer.cell {
                        CellOutput::F1 => V::min(V::mul(f1, V::splat(F1_SPREAD)), one),
                        CellOutput::F2 => V::min(V::mul(f2, V::splat(F2_SPREAD)), one),
                        CellOutput::F2MinusF1 => {
                            V::min(V::mul(V::sub(f2, f1), V::splat(F21_SPREAD)), one)
                        }
                    }
                }
            };
            let two = V::splat(2.);
            let s = match layer.fractal {
                FractalMode::Fbm => n,
                FractalMode::Ridged => {
                    let r = V::sub(V::splat(1.), V::abs(V::sub(V::mul(two, n), V::splat(1.))));
                    V::mul(r, r)
                }
                FractalMode::Turbulence => V::abs(V::sub(V::mul(two, n), V::splat(1.))),
            };
            sum = V::add(sum, V::mul(V::splat(layer.amps[o]), s));
        }
        V::div(sum, V::splat(layer.total))
    }

    /// `cells` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn cells_body<V: Lanes>(
        &mut self,
        b: [V::F; 3],
        index: usize,
    ) -> (CellsLanes<V>, [V::F; 3]) {
        let plan = self.plan;
        let oc = &plan.cells[index];
        let q = oc.point_lanes::<V>(b, plan.mode == Mode::Uv);
        let c = worley3_lanes::<V>(q, oc.seed, oc.per, &mut self.set.worley[oc.slot]);
        (c, q)
    }

    /// セルの枠 `index` の最寄りと 2 番目の距離だけ（`cells_lanes` の `f1`・`f2` と同じ値。ID・点が要らない呼び手用）。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn distances_body<V: Lanes>(
        &mut self,
        b: [V::F; 3],
        index: usize,
    ) -> (V::F, V::F) {
        let plan = self.plan;
        let oc = &plan.cells[index];
        let q = oc.point_lanes::<V>(b, plan.mode == Mode::Uv);
        worley_distances_lanes::<V>(q, oc.seed, oc.per, &mut self.set.worley[oc.slot])
    }

    /// `segments` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub(super) unsafe fn segments_body<V: Lanes>(&mut self, b: [V::F; 3], index: usize) -> V::F {
        let plan = self.plan;
        let (oc, spec) = &plan.segments[index];
        let q = oc.point_lanes::<V>(b, plan.mode == Mode::Uv);
        grunge::segments_lanes::<V>(q, oc.seed, oc.per, spec, &mut self.set.segments[oc.slot])
    }

    /// `layer` の N 画素ぶん。道ごとの入口（[`GenLanes::layer`]）を呼び、ノイズの式の展開はそこで止まる。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    pub unsafe fn layer_lanes<V: GenLanes>(&mut self, b: [V::F; 3], index: usize) -> V::F {
        V::layer(self, b, index)
    }

    /// `warp` の N 画素ぶん。
    #[inline(always)]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    unsafe fn warp_lanes<V: GenLanes>(&mut self, b: [V::F; 3]) -> [V::F; 3] {
        let amount = self.plan.p.bleed;
        if amount <= 0. {
            return b;
        }
        let a = V::splat(3.2 * amount);
        let w = self.plan.warp;
        let x = V::add(b[0], shift_lanes::<V>(self.layer_lanes::<V>(b, w), a));
        let y = V::add(b[1], shift_lanes::<V>(self.layer_lanes::<V>(b, w + 1), a));
        let z = if self.plan.mode == Mode::Uv {
            b[2]
        } else {
            V::add(b[2], shift_lanes::<V>(self.layer_lanes::<V>(b, w + 2), a))
        };
        [x, y, z]
    }

    /// にじみ: 座標を低周波のノイズでずらす（UV では周期を保つ）。
    fn warp(&mut self, b: [f64; 3]) -> [f64; 3] {
        let amount = self.plan.p.bleed;
        if amount <= 0. {
            return b;
        }
        let a = 3.2 * amount;
        let w = self.plan.warp;
        [
            b[0] + (self.layer(b, w) - 0.5) * a,
            b[1] + (self.layer(b, w + 1) - 0.5) * a,
            if self.plan.mode == Mode::Uv {
                b[2]
            } else {
                b[2] + (self.layer(b, w + 2) - 0.5) * a
            },
        ]
    }
}

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

    /// 格子の覚えを使わずに（呼ぶたびに空の領域で）式を評価する。
    fn recipe(plan: &Plan, b: [f64; 3]) -> f64 {
        let mut scratch = plan.scratch();
        plan.recipe(b, &mut scratch.sets[0])
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
                            let here = recipe(&plan, b);
                            let across_x = recipe(&plan, [b[0] + nx, b[1], 0.]);
                            let across_y = recipe(&plan, [b[0], b[1] + ny, 0.]);
                            let across_both = recipe(&plan, [b[0] + nx, b[1] + ny, 0.]);
                            for (what, v) in [("x", across_x), ("y", across_y), ("xy", across_both)]
                            {
                                // 足し算の丸めで小数部が 1 ULP ずれ得るので、ビットでなく近さで比べる
                                assert!(
                                    (here - v).abs() < 1e-6,
                                    "{name} scale {scale} bleed {bleed} {dimensions:?}: {what} に 1 周期ずらすと {here} が {v} になる（{b:?}）"
                                );
                            }
                            for half in [[nx * 0.5, 0.], [0., ny * 0.5]] {
                                let moved = recipe(&plan, [b[0] + half[0], b[1] + half[1], 0.]);
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

    /// 隣の画素へ進めながら格子の覚えを使い回した値は、画素ごとに空の領域で評価した値とビットまで同じ（覚えが違う格子の値を返さない）。
    /// 位置・向き・UV の座標が、同じ格子に居続ける・隣の格子へ越える・飛ぶ、のどれもを通る。
    #[test]
    fn reusing_the_cells_along_a_scanline_never_changes_a_value() {
        const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let data = [0u16; 16 * 16 * 3];
        let cover = [1u8; 16 * 16];
        let map = |kind| Map {
            kind,
            width: 16,
            height: 16,
            data: &data,
            coverage: &cover,
            bounds_min: [-1., -2., -3.],
            bounds_max: [2., 3., 1.],
            condition_key: KEY,
            state: MapState::Current,
        };
        let (position, normal) = (map(MapKind::Position), map(MapKind::WorldNormal));
        let mut maps: [Option<&Map<'_>>; 10] = [None; 10];
        maps[MapKind::Position as usize] = Some(&position);
        maps[MapKind::WorldNormal as usize] = Some(&normal);
        let mut checked = 0;
        let settings = every_setting();
        for (name, base) in &settings {
            for space in [
                ProceduralSpace::Position,
                ProceduralSpace::Triplanar,
                ProceduralSpace::Uv,
            ] {
                for bleed in [0., 0.4] {
                    let mut s = base.clone();
                    s.procedural.space = space;
                    s.procedural.bleed = bleed;
                    s.procedural.rotation = [13., -27., 41.];
                    s.procedural.scale = 0.03;
                    let plan = Plan::new(&s, &maps, (16, 16)).unwrap();
                    let mut shared = plan.scratch();
                    for i in 0..400u32 {
                        let t = i as f64;
                        // 3 つの区間: ゆっくり進む（同じ格子が続く）・速く進む（毎画素ちがう格子）・向きが回って重みが入れ替わる
                        let (pos, n) = match i / 100 {
                            0 => (
                                [10000. + 9. * t, 20000. + 3. * t, 30000. + t],
                                [0.2, 0.9, 0.3],
                            ),
                            1 => (
                                [5000. + 631. * t, 40000. - 377. * t, 700. * t],
                                [0.9, 0.1, 0.2],
                            ),
                            _ => (
                                [30000. + t, 31000. + 2. * t, 32000. + 3. * t],
                                [(t * 0.07).cos(), (t * 0.05).sin(), 0.3 + (t * 0.03).sin()],
                            ),
                        };
                        let n = n.map(|v: f64| (v * 0.5 + 0.5) * 65535.);
                        let (x, y) = (i % 16, i / 16 % 16);
                        let mut fresh = plan.scratch();
                        let a = plan.value(x, y, (16, 16), Some(pos), Some(n), &mut shared);
                        let b = plan.value(x, y, (16, 16), Some(pos), Some(n), &mut fresh);
                        assert_eq!(
                            a.map(f64::to_bits),
                            b.map(f64::to_bits),
                            "{name} {space:?} bleed {bleed} i {i}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, settings.len() * 3 * 2 * 400);
    }

    /// SIMD の道（AVX2・SSE4.1・NEON）の N 画素ぶんの値は、1 画素ずつ（`Plan::value`）の値とビットまで同じ。全部のノイズの種類・プリセット・空間・
    /// にじみ・回転で、位置・向き・UV の座標が同じ格子に居続ける・隣の格子へ越える・飛ぶ、のどれもを通り、向きの定まらない画素
    /// （`None`）の組は、1 つでも `None` があれば組全体が `None` になる。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn lane_values_equal_single_pixel_values_on_every_simd_level() {
        const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        #[inline(never)]
        unsafe fn value_lanes<V: GenLanes>(
            plan: &Plan,
            x: u32,
            y: u32,
            position: [V::F; 3],
            normal: [V::F; 3],
            scratch: &mut PlanScratch,
        ) -> Option<V::F> {
            plan.value_lanes::<V>(x, y, (16, 16), position, normal, scratch)
        }
        #[allow(clippy::needless_range_loop)]
        unsafe fn check<V: GenLanes>() {
            let data = [0u16; 16 * 16 * 3];
            let cover = [1u8; 16 * 16];
            let map = |kind| Map {
                kind,
                width: 16,
                height: 16,
                data: &data,
                coverage: &cover,
                bounds_min: [-1., -2., -3.],
                bounds_max: [2., 3., 1.],
                condition_key: KEY,
                state: MapState::Current,
            };
            let (position, normal) = (map(MapKind::Position), map(MapKind::WorldNormal));
            let mut maps: [Option<&Map<'_>>; 10] = [None; 10];
            maps[MapKind::Position as usize] = Some(&position);
            maps[MapKind::WorldNormal as usize] = Some(&normal);
            let mut lanes_used = 0;
            for (name, base) in every_setting() {
                for space in [
                    ProceduralSpace::Position,
                    ProceduralSpace::Triplanar,
                    ProceduralSpace::Uv,
                ] {
                    for bleed in [0., 0.4] {
                        let mut s = base.clone();
                        s.procedural.space = space;
                        s.procedural.bleed = bleed;
                        s.procedural.rotation = [13., -27., 41.];
                        s.procedural.scale = 0.03;
                        let plan = Plan::new(&s, &maps, (16, 16)).unwrap();
                        let mut shared = plan.scratch();
                        for i in 0..150u32 {
                            let mut pos = [[0.; 4]; 3];
                            let mut nrm = [[0.; 4]; 3];
                            for k in 0..V::N {
                                let t = (i * V::N as u32 + k as u32) as f64;
                                let (p, n) = match i / 50 {
                                    0 => (
                                        [10000. + 9. * t, 20000. + 3. * t, 30000. + t],
                                        [0.2, 0.9, 0.3],
                                    ),
                                    1 => (
                                        [5000. + 631. * t, 40000. - 377. * t, 700. * t],
                                        [0.9, 0.1, 0.2],
                                    ),
                                    _ => (
                                        [30000. + t, 31000. + 2. * t, 32000. + 3. * t],
                                        [
                                            (t * 0.07).cos(),
                                            (t * 0.05).sin(),
                                            0.3 + (t * 0.03).sin(),
                                        ],
                                    ),
                                };
                                for axis in 0..3 {
                                    pos[axis][k] = p[axis];
                                    nrm[axis][k] = (n[axis] * 0.5 + 0.5) * 65535.;
                                }
                            }
                            let x = i % 8 * 2;
                            let y = i / 8 % 16;
                            let lanes = |a: &[[f64; 4]; 3]| {
                                [V::load_f64(&a[0]), V::load_f64(&a[1]), V::load_f64(&a[2])]
                            };
                            let got = value_lanes::<V>(
                                &plan,
                                x,
                                y,
                                lanes(&pos),
                                lanes(&nrm),
                                &mut shared,
                            );
                            let mut want = [None; 4];
                            for k in 0..V::N {
                                let mut fresh = plan.scratch();
                                want[k] = plan.value(
                                    x + k as u32,
                                    y,
                                    (16, 16),
                                    Some([pos[0][k], pos[1][k], pos[2][k]]),
                                    Some([nrm[0][k], nrm[1][k], nrm[2][k]]),
                                    &mut fresh,
                                );
                            }
                            match got {
                                Some(v) => {
                                    let mut out = [0.; 4];
                                    V::store_f64(&mut out, v);
                                    for k in 0..V::N {
                                        assert_eq!(
                                            Some(out[k].to_bits()),
                                            want[k].map(f64::to_bits),
                                            "{name} {space:?} bleed {bleed} i {i} lane {k}"
                                        );
                                    }
                                    lanes_used += 1;
                                }
                                None => assert!(
                                    want[..V::N].iter().any(Option::is_none),
                                    "{name} {space:?}: 組が None なのに、どの画素も値を持つ"
                                ),
                            }
                        }
                    }
                }
            }
            assert!(lanes_used > 1000, "{lanes_used}");
        }
        crate::math::simd::on_each_level!(check);
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
