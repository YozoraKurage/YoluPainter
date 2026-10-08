//! グランジのプリセット。ノイズのレイヤー（[`Layer`]）としきい値（smoothstep）・三角波の組み合わせで、値は 1 が「汚れ・傷などがある」。
//! 式は + − × ÷ sqrt floor と整数だけ。座標 `b` は基本のセル（`Procedural::scale`）が 1 の単位。
//!
//! プリセットごとに使うレイヤー・セルの枠・線分の枠を [`Spec`] に並べ、束縛のときに計画（`procedural::Plan`）が乱数と座標の写し方を先に出す。
//! 式は番号でそれを呼ぶ。式の中の演算とその順は、レイヤーを式の中で作っていた頃と同じ。
//!
//! 各式には、N 画素を 1 組で計算する版（`*_lanes`）がある。1 画素の式と同じ演算を同じ順に並べ、分岐はレーンごとの選択に置き換えたもので、
//! 結果のビットは変わらない。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::noisefn::{sin_cos_deg_lanes, unit24_lanes};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::procedural::GenLanes;
use super::{
    noisefn::{cell_hash, hash_unit, sin_cos_deg, unit24, wrap},
    procedural::{Ctx, FractalMode::*, GrungePreset, Layer, NoiseBasis::*},
};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};

/// セルの枠（Worley の特徴点を引く座標の写し方）。
pub(super) struct CellSpec {
    pub freq: [f64; 3],
    pub slot: u32,
}
/// 線分の枠: セルごとに向き・長さ・太さが乱数の線分（端が細る）。`len` は枠の単位（セルの間隔が 1）の半分の長さ、`width` は基本のセルの単位の太さ。
#[derive(Clone, Copy)]
pub(super) struct SegSpec {
    pub freq: f64,
    pub slot: u32,
    pub chance: f64,
    pub len: f64,
    pub width: f64,
}
/// プリセットが使うレイヤー・セルの枠・線分の枠（式が番号で呼ぶ）。
pub(super) struct Spec {
    pub layers: &'static [Layer],
    pub cells: &'static [CellSpec],
    pub segments: &'static [SegSpec],
}

/// 織りの 1 つの基本のセルの本数（偶数。UV では周期が奇数にならず、市松の向きが巻く）。
const WEAVE_THREADS: f64 = 6.;

static STAIN: Spec = Spec {
    layers: &[
        Layer::new(Value, Fbm, 5, 0.7, 0),
        Layer::new(Perlin, Fbm, 4, 2.2, 1),
        Layer::new(Value, Fbm, 2, 24., 2),
    ],
    cells: &[],
    segments: &[],
};
static RUST: Spec = Spec {
    layers: &[
        Layer::new(Perlin, Fbm, 5, 0.8, 0),
        Layer::new(Perlin, Ridged, 4, 4., 1),
        Layer::new(Worley, Fbm, 1, 12., 2),
    ],
    cells: &[],
    segments: &[],
};
static SCRATCHES: Spec = Spec {
    layers: &[Layer::new(Perlin, Fbm, 3, 0.8, 9)],
    cells: &[],
    segments: &[
        SegSpec {
            freq: 1.,
            slot: 0,
            chance: 0.7,
            len: 0.9,
            width: 0.016,
        },
        SegSpec {
            freq: 2.3,
            slot: 1,
            chance: 0.75,
            len: 0.8,
            width: 0.009,
        },
        SegSpec {
            freq: 5.,
            slot: 2,
            chance: 0.7,
            len: 0.6,
            width: 0.005,
        },
    ],
};
static DUST: Spec = Spec {
    layers: &[
        Layer::new(Value, Fbm, 3, 18., 0),
        Layer::new(Perlin, Fbm, 4, 1.2, 1),
    ],
    cells: &[],
    segments: &[],
};
static FINGERPRINTS: Spec = Spec {
    layers: &[
        Layer::new(Perlin, Fbm, 3, 3., 1),
        Layer::new(Perlin, Fbm, 2, 4., 2),
        Layer::new(Perlin, Fbm, 3, 2., 3),
    ],
    cells: &[CellSpec {
        freq: [0.6, 0.6, 0.],
        slot: 20,
    }],
    segments: &[],
};
static WEAVE: Spec = Spec {
    layers: &[Layer::new(Value, Fbm, 2, WEAVE_THREADS * 5., 0)],
    cells: &[],
    segments: &[],
};
static CRACKS: Spec = Spec {
    layers: &[
        Layer::new(Perlin, Fbm, 2, 1.5, 0),
        Layer::new(Perlin, Fbm, 2, 1.5, 1),
        Layer::new(Perlin, Fbm, 2, 1.5, 2),
        Layer::new(Perlin, Fbm, 2, 3., 4),
        Layer::new(Perlin, Fbm, 2, 2., 6),
    ],
    cells: &[
        CellSpec {
            freq: [1.2; 3],
            slot: 3,
        },
        CellSpec {
            freq: [3.4; 3],
            slot: 5,
        },
    ],
    segments: &[],
};
static SPLATTER: Spec = Spec {
    layers: &[],
    cells: &[
        CellSpec {
            freq: [1.; 3],
            slot: 0,
        },
        CellSpec {
            freq: [3.1; 3],
            slot: 1,
        },
        CellSpec {
            freq: [9.; 3],
            slot: 2,
        },
    ],
    segments: &[],
};
static PEELING: Spec = Spec {
    layers: &[
        Layer::new(Perlin, Fbm, 5, 1., 0),
        Layer::new(Perlin, Ridged, 3, 5., 1),
    ],
    cells: &[],
    segments: &[],
};
static WOOD_GRAIN: Spec = Spec {
    layers: &[
        Layer::new(Perlin, Fbm, 3, 0.45, 0),
        Layer::new(Value, Fbm, 2, 6., 1),
        Layer::new(Value, Fbm, 3, 1., 2).aniso([9., 0.5, 9.]),
    ],
    cells: &[],
    segments: &[],
};
static PEBBLES: Spec = Spec {
    layers: &[],
    cells: &[CellSpec {
        freq: [1.; 3],
        slot: 0,
    }],
    segments: &[],
};

pub(super) fn spec(preset: GrungePreset) -> &'static Spec {
    match preset {
        GrungePreset::Stain => &STAIN,
        GrungePreset::Rust => &RUST,
        GrungePreset::Scratches => &SCRATCHES,
        GrungePreset::Dust => &DUST,
        GrungePreset::Fingerprints => &FINGERPRINTS,
        GrungePreset::Weave => &WEAVE,
        GrungePreset::Cracks => &CRACKS,
        GrungePreset::Splatter => &SPLATTER,
        GrungePreset::Peeling => &PEELING,
        GrungePreset::WoodGrain => &WOOD_GRAIN,
        GrungePreset::Pebbles => &PEBBLES,
    }
}

#[inline]
fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
/// 整数で 1、半分で 0 の三角波。
#[inline]
fn tri(x: f64) -> f64 {
    ((x - x.floor()) * 2. - 1.).abs()
}

pub(super) fn eval(cx: &mut Ctx<'_>, b: [f64; 3], preset: GrungePreset) -> f64 {
    match preset {
        GrungePreset::Stain => stain(cx, b),
        GrungePreset::Rust => rust(cx, b),
        GrungePreset::Scratches => scratches(cx, b),
        GrungePreset::Dust => dust(cx, b),
        GrungePreset::Fingerprints => fingerprints(cx, b),
        GrungePreset::Weave => weave(cx, b),
        GrungePreset::Cracks => cracks(cx, b),
        GrungePreset::Splatter => splatter(cx, b),
        GrungePreset::Peeling => peeling(cx, b),
        GrungePreset::WoodGrain => wood_grain(cx, b),
        GrungePreset::Pebbles => pebbles(cx, b),
    }
}

fn stain(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let a = cx.layer(b, 0);
    let c = cx.layer(b, 1);
    let g = cx.layer(b, 2);
    smooth(0.38, 0.78, a * 0.6 + c * 0.4) * (0.7 + 0.6 * g)
}

fn rust(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let patch = cx.layer(b, 0);
    let blot = cx.layer(b, 1);
    let pits = 1. - smooth(0., 0.2, cx.layer(b, 2));
    let spread = smooth(0.42, 0.60, patch);
    let halo = smooth(0.32, 0.46, patch) * 0.3 * blot;
    (spread * (0.45 + 0.55 * blot) + 0.4 * spread * pits).max(halo)
}

/// 1 つのセルの線分（そのセルが線分を持つ場合）。
#[derive(Clone, Copy)]
struct Segment {
    center: [f64; 2],
    sin: f64,
    cos: f64,
    half: f64,
    width: f64,
    strength: f64,
}
const NO_SEGMENT: Segment = Segment {
    center: [0.; 2],
    sin: 0.,
    cos: 0.,
    half: 0.,
    width: 0.,
    strength: 0.,
};
/// 線分の枠の、直前の格子とその周りの 9 セルの線分（線分を持つセルだけを、元の走査と同じ順に並べる）。
#[derive(Clone, Copy)]
pub(super) struct SegCell {
    valid: bool,
    key: [i32; 2],
    count: usize,
    items: [Segment; 9],
}
impl SegCell {
    pub(super) const EMPTY: Self = Self {
        valid: false,
        key: [0; 2],
        count: 0,
        items: [NO_SEGMENT; 9],
    };
}
impl SegCell {
    /// 格子 `key` とその周りの 9 セルの線分を引く（直前と同じ格子なら引き直さない）。
    fn fill(&mut self, key: [i32; 2], seed: u32, per: [i32; 2], spec: &SegSpec) {
        if self.valid && self.key == key {
            return;
        }
        let mut count = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (key[0] + dx, key[1] + dy);
                let h = cell_hash(seed, wrap(cx, per[0]), wrap(cy, per[1]), 5);
                if hash_unit(h ^ 0x0a5b_3c91) >= spec.chance {
                    continue;
                }
                let (sin, cos) = sin_cos_deg(hash_unit(h ^ 0x02e5_be93) * 180.);
                self.items[count] = Segment {
                    center: [
                        cx as f64 + unit24(h),
                        cy as f64 + hash_unit(h ^ 0x68bc_21eb),
                    ],
                    sin,
                    cos,
                    half: spec.len * (0.4 + 0.6 * hash_unit(h ^ 0x9e37_79b1)),
                    width: spec.width * spec.freq * (0.6 + 0.8 * hash_unit(h ^ 0x85eb_ca6b)),
                    strength: 0.55 + 0.45 * hash_unit(h ^ 0x27d4_eb2f),
                };
                count += 1;
            }
        }
        self.count = count;
        self.key = key;
        self.valid = true;
    }
}
/// 傷の筋: セルごとに向き・長さ・太さが乱数の線分（端が細る）。2D の模様（位置ではトライプラナー）。`fp` は枠の格子の座標。
pub(super) fn segments(
    fp: [f64; 3],
    seed: u32,
    per: [i32; 2],
    spec: &SegSpec,
    cell: &mut SegCell,
) -> f64 {
    cell.fill(
        [fp[0].floor() as i32, fp[1].floor() as i32],
        seed,
        per,
        spec,
    );
    let mut best: f64 = 0.;
    for sg in &cell.items[..cell.count] {
        let (rx, ry) = (fp[0] - sg.center[0], fp[1] - sg.center[1]);
        let (along, perp) = ((sg.cos * rx + sg.sin * ry).abs(), sg.cos * ry - sg.sin * rx);
        // 線から幅以上離れた画素は、減衰の項が丸めまで含めて 0 になり、`best` を変えない
        // （距離は `|perp|` より小さくなり得るが、丸めの 1 ULP 程度。余裕を持って切る）
        if perp.abs() >= sg.width * (1. + 1e-9) {
            continue;
        }
        let dist = if along <= sg.half {
            perp.abs()
        } else {
            ((along - sg.half) * (along - sg.half) + perp * perp).sqrt()
        };
        let taper = 1. - smooth(sg.half * 0.5, sg.half, along);
        let v = (1. - smooth(sg.width * 0.4, sg.width, dist)) * (0.5 + 0.5 * taper);
        best = best.max(v * sg.strength);
    }
    best
}
fn scratches(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let wear = smooth(0.25, 0.6, cx.layer(b, 0));
    let long = cx.segments(b, 0);
    let mid = cx.segments(b, 1);
    let short = cx.segments(b, 2);
    long.max(mid).max(short) * (0.35 + 0.65 * wear)
}

fn dust(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let fine = cx.layer(b, 0);
    let m = cx.layer(b, 1);
    let cover = smooth(0.30, 0.65, m);
    cover * (0.3 + 0.7 * smooth(0.5, 0.85, fine))
}

fn fingerprints(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let (c, fp) = cx.cells(b, 0);
    let d = [fp[0] - c.point[0], fp[1] - c.point[1]];
    let (s, co) = sin_cos_deg(unit24(c.id) * 180.);
    let dx = co * d[0] + s * d[1];
    let dy = (co * d[1] - s * d[0]) * 1.3;
    let r = (dx * dx + dy * dy).sqrt();
    let n1 = cx.layer(b, 0) - 0.5;
    let n2 = cx.layer(b, 1) - 0.5;
    let vis = smooth(0.25, 0.6, cx.layer(b, 2));
    let ridge = smooth(0.30, 0.55, tri(r * 11. + n1 * 2.4));
    let patch = 1. - smooth(0.22, 0.46, r + 0.1 * n2);
    ridge * patch * (0.5 + 0.5 * vis)
}

fn weave(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    const T: f64 = WEAVE_THREADS;
    let (x, y) = (b[0] * T, b[1] * T);
    let (jx, jy) = (x.floor(), y.floor());
    let (fx, fy) = (x - jx, y - jy);
    let (jx, jy) = (jx as i32, jy as i32);
    let p = cx.plan;
    let per = p.period(T as i32);
    let prof = |f: f64| 1. - (2. * f - 1.) * (2. * f - 1.);
    let over_vertical = (jx + jy).rem_euclid(2) == 0;
    let (over, thread) = if over_vertical {
        (prof(fx), p.lattice(21, jx, 0, per))
    } else {
        (prof(fy), p.lattice(22, 0, jy, per))
    };
    let fiber = cx.layer(b, 0);
    (0.3 + 0.7 * over) * (0.8 + 0.2 * thread) * (0.88 + 0.24 * fiber)
}

fn cracks(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let w = |cx: &mut Ctx<'_>, k| (cx.layer(b, k) - 0.5) * 0.45;
    let wx = w(cx, 0);
    let wy = w(cx, 1);
    let wb = [
        b[0] + wx,
        b[1] + wy,
        if cx.plan.is_uv() {
            b[2]
        } else {
            b[2] + w(cx, 2)
        },
    ];
    let (c, _) = cx.cells(wb, 0);
    let width = 0.012 + 0.02 * cx.layer(b, 3);
    let main = 1. - smooth(width * 0.4, width * 1.2, c.f2 - c.f1);
    let (c2, _) = cx.cells(wb, 1);
    let fine = (1. - smooth(0.01, 0.03, c2.f2 - c2.f1)) * smooth(0.4, 0.6, cx.layer(b, 4));
    main.max(0.6 * fine)
}

fn drops(cx: &mut Ctx<'_>, b: [f64; 3], frame: usize, biggest: f64) -> f64 {
    let (c, _) = cx.cells(b, frame);
    let (r1, r2) = (unit24(c.id), hash_unit(c.id ^ 0x7f4a_7c15));
    if r2 < 0.45 {
        return 0.;
    }
    let radius = 0.04 + biggest * r1 * r1;
    1. - smooth(radius * 0.75, radius, c.f1)
}
fn splatter(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let big = drops(cx, b, 0, 0.32);
    let mid = drops(cx, b, 1, 0.3);
    let small = drops(cx, b, 2, 0.28) * 0.9;
    big.max(mid).max(small)
}

/// 塗装の剥げ: 剥げの場の値（大きいほど剥げる）。既定のレベル（`GrungePreset::default_levels`）が境目を作り、レベルを動かすと
/// 剥げの割合が変わる。
fn peeling(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let base = cx.layer(b, 0);
    let chip = cx.layer(b, 1);
    base + 0.12 * (chip - 0.5)
}

fn wood_grain(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    // 年輪: 位置の空間では局所の y 軸のまわり、UV では x 方向の縞（周期が整数の本数になる）
    let r = if cx.plan.is_uv() {
        b[0] * 4.
    } else {
        (b[0] * b[0] + b[2] * b[2] * 0.85).sqrt() * 2.5
    };
    let warp = (cx.layer(b, 0) - 0.5) * 1.6 + (cx.layer(b, 1) - 0.5) * 0.25;
    let rings = r + warp;
    let f = rings - rings.floor();
    let ring = if f < 0.78 { f / 0.78 } else { (1. - f) / 0.22 };
    let fiber = cx.layer(b, 2);
    (0.2 + 0.8 * ring) * (0.8 + 0.4 * fiber)
}

fn pebbles(cx: &mut Ctx<'_>, b: [f64; 3]) -> f64 {
    let (c, _) = cx.cells(b, 0);
    let border = smooth(0., 0.14, c.f2 - c.f1);
    let dome = 1. - smooth(0., 0.6, c.f1);
    border * (0.5 + 0.5 * dome) * (0.82 + 0.18 * unit24(c.id))
}

// ───────── SIMD の道（N 画素を 1 組で） ─────────

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn smooth_lanes<V: Lanes>(e0: f64, e1: f64, x: V::F) -> V::F {
    let t = simd::clamp01::<V>(V::div(V::sub(x, V::splat(e0)), V::splat(e1 - e0)));
    V::mul(V::mul(t, t), V::sub(V::splat(3.), V::mul(V::splat(2.), t)))
}
/// 端もレーンごとに違う `smooth`。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn smooth_edges_lanes<V: Lanes>(e0: V::F, e1: V::F, x: V::F) -> V::F {
    let t = simd::clamp01::<V>(V::div(V::sub(x, e0), V::sub(e1, e0)));
    V::mul(V::mul(t, t), V::sub(V::splat(3.), V::mul(V::splat(2.), t)))
}
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn tri_lanes<V: Lanes>(x: V::F) -> V::F {
    V::abs(V::sub(
        V::mul(V::sub(x, V::floor(x)), V::splat(2.)),
        V::splat(1.),
    ))
}
/// 整数の値を持つ N 個の f64（ハッシュなど）の各レーンに `f` を引く（`from_fn` に渡すので、レーンの演算は含めない）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn per_hash<V: Lanes>(ids: V::F, f: impl Fn(u32) -> f64) -> V::F {
    let mut at = [0.; 4];
    V::store_f64(&mut at, ids);
    V::from_fn(|k| f(at[k] as u32))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn eval_lanes<V: GenLanes>(
    cx: &mut Ctx<'_>,
    b: [V::F; 3],
    preset: GrungePreset,
) -> V::F {
    V::grunge(cx, b, preset)
}

/// 模様ごとの、道の命令を有効にした展開しない入口（`$m::dispatch` が模様で選ぶ）。模様の中のノイズのレイヤー・セル・線分は展開したまま
/// （模様ごとの関数は数個のレイヤーの大きさに収まる）、模様どうしは別の関数にする。
macro_rules! preset_entries {
    ($m:ident, $ty:ident, $feature:literal, $($preset:ident => $f:ident),* $(,)?) => {
        pub(super) mod $m {
            use super::*;
            use crate::math::simd::$ty;
            $(
                #[target_feature(enable = $feature)]
                #[inline(never)]
                unsafe fn $f(cx: &mut Ctx<'_>, b: [<$ty as Lanes>::F; 3]) -> <$ty as Lanes>::F {
                    super::$f::<$ty>(cx, b)
                }
            )*
            #[inline(always)]
            pub(in crate::generator) unsafe fn dispatch(
                cx: &mut Ctx<'_>,
                b: [<$ty as Lanes>::F; 3],
                preset: GrungePreset,
            ) -> <$ty as Lanes>::F {
                match preset {
                    $(GrungePreset::$preset => $f(cx, b),)*
                }
            }
        }
    };
}
macro_rules! presets {
    ($($args:tt)*) => {
        preset_entries!($($args)*,
            Stain => stain_lanes, Rust => rust_lanes, Scratches => scratches_lanes, Dust => dust_lanes,
            Fingerprints => fingerprints_lanes, Weave => weave_lanes, Cracks => cracks_lanes,
            Splatter => splatter_lanes, Peeling => peeling_lanes, WoodGrain => wood_grain_lanes,
            Pebbles => pebbles_lanes);
    };
}
#[cfg(target_arch = "x86_64")]
presets!(avx2, Avx2, "avx2,fma");
#[cfg(target_arch = "x86_64")]
presets!(sse41, Sse41, "sse4.1");
#[cfg(target_arch = "aarch64")]
presets!(neon, Neon, "neon");

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn stain_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let a = cx.layer_body::<V>(b, 0);
    let c = cx.layer_body::<V>(b, 1);
    let g = cx.layer_body::<V>(b, 2);
    V::mul(
        smooth_lanes::<V>(
            0.38,
            0.78,
            V::add(V::mul(a, V::splat(0.6)), V::mul(c, V::splat(0.4))),
        ),
        V::add(V::splat(0.7), V::mul(V::splat(0.6), g)),
    )
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn rust_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let patch = cx.layer_body::<V>(b, 0);
    let blot = cx.layer_body::<V>(b, 1);
    let pits = V::sub(
        V::splat(1.),
        smooth_lanes::<V>(0., 0.2, cx.layer_body::<V>(b, 2)),
    );
    let spread = smooth_lanes::<V>(0.42, 0.60, patch);
    let halo = V::mul(
        V::mul(smooth_lanes::<V>(0.32, 0.46, patch), V::splat(0.3)),
        blot,
    );
    V::max(
        V::add(
            V::mul(spread, V::add(V::splat(0.45), V::mul(V::splat(0.55), blot))),
            V::mul(V::mul(V::splat(0.4), spread), pits),
        ),
        halo,
    )
}

/// `segments` の N 画素ぶん。N 画素が同じ格子に入れば線分を共有して計算し、またぐ組は 1 画素ずつ。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(super) unsafe fn segments_lanes<V: Lanes>(
    fp: [V::F; 3],
    seed: u32,
    per: [i32; 2],
    spec: &SegSpec,
    cell: &mut SegCell,
) -> V::F {
    let (fx, fy) = (V::floor(fp[0]), V::floor(fp[1]));
    let mut floors = [[0.; 4]; 2];
    V::store_f64(&mut floors[0], fx);
    V::store_f64(&mut floors[1], fy);
    let key = [floors[0][0] as i32, floors[1][0] as i32];
    let same = (1..V::N).all(|k| floors[0][k] == floors[0][0] && floors[1][k] == floors[1][0]);
    if !same {
        let mut at = [[0.; 4]; 3];
        for (axis, lanes) in at.iter_mut().enumerate() {
            V::store_f64(lanes, fp[axis]);
        }
        let mut out = [0.; 4];
        for (k, o) in out.iter_mut().enumerate().take(V::N) {
            *o = segments([at[0][k], at[1][k], at[2][k]], seed, per, spec, cell);
        }
        return V::load_f64(&out);
    }
    cell.fill(key, seed, per, spec);
    let mut best = V::splat(0.);
    for sg in &cell.items[..cell.count] {
        let (rx, ry) = (
            V::sub(fp[0], V::splat(sg.center[0])),
            V::sub(fp[1], V::splat(sg.center[1])),
        );
        let (sin, cos) = (V::splat(sg.sin), V::splat(sg.cos));
        let along = V::abs(V::add(V::mul(cos, rx), V::mul(sin, ry)));
        let perp = V::sub(V::mul(cos, ry), V::mul(sin, rx));
        // 線から幅以上離れた画素は、減衰の項が丸めまで含めて 0 になり、`best` を変えない。全部のレーンがそうなら飛ばす
        if V::all(V::ge(V::abs(perp), V::splat(sg.width * (1. + 1e-9)))) {
            continue;
        }
        let half = V::splat(sg.half);
        let past = V::sub(along, half);
        let dist = V::select(
            V::le(along, half),
            V::abs(perp),
            V::sqrt(V::add(V::mul(past, past), V::mul(perp, perp))),
        );
        let taper = V::sub(
            V::splat(1.),
            smooth_lanes::<V>(sg.half * 0.5, sg.half, along),
        );
        let v = V::mul(
            V::sub(
                V::splat(1.),
                smooth_lanes::<V>(sg.width * 0.4, sg.width, dist),
            ),
            V::add(V::splat(0.5), V::mul(V::splat(0.5), taper)),
        );
        best = V::max(best, V::mul(v, V::splat(sg.strength)));
    }
    best
}
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn scratches_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let wear = smooth_lanes::<V>(0.25, 0.6, cx.layer_body::<V>(b, 0));
    let long = cx.segments_body::<V>(b, 0);
    let mid = cx.segments_body::<V>(b, 1);
    let short = cx.segments_body::<V>(b, 2);
    V::mul(
        V::max(V::max(long, mid), short),
        V::add(V::splat(0.35), V::mul(V::splat(0.65), wear)),
    )
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn dust_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let fine = cx.layer_body::<V>(b, 0);
    let m = cx.layer_body::<V>(b, 1);
    let cover = smooth_lanes::<V>(0.30, 0.65, m);
    V::mul(
        cover,
        V::add(
            V::splat(0.3),
            V::mul(V::splat(0.7), smooth_lanes::<V>(0.5, 0.85, fine)),
        ),
    )
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn fingerprints_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let (c, fp) = cx.cells_body::<V>(b, 0);
    let d = [V::sub(fp[0], c.point[0]), V::sub(fp[1], c.point[1])];
    let (s, co) = sin_cos_deg_lanes::<V>(V::mul(unit24_lanes::<V>(c.id), V::splat(180.)));
    let dx = V::add(V::mul(co, d[0]), V::mul(s, d[1]));
    let dy = V::mul(V::sub(V::mul(co, d[1]), V::mul(s, d[0])), V::splat(1.3));
    let r = V::sqrt(V::add(V::mul(dx, dx), V::mul(dy, dy)));
    let half = V::splat(0.5);
    let n1 = V::sub(cx.layer_body::<V>(b, 0), half);
    let n2 = V::sub(cx.layer_body::<V>(b, 1), half);
    let vis = smooth_lanes::<V>(0.25, 0.6, cx.layer_body::<V>(b, 2));
    let ridge = smooth_lanes::<V>(
        0.30,
        0.55,
        tri_lanes::<V>(V::add(V::mul(r, V::splat(11.)), V::mul(n1, V::splat(2.4)))),
    );
    let patch = V::sub(
        V::splat(1.),
        smooth_lanes::<V>(0.22, 0.46, V::add(r, V::mul(V::splat(0.1), n2))),
    );
    V::mul(V::mul(ridge, patch), V::add(half, V::mul(half, vis)))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn weave_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    const T: f64 = WEAVE_THREADS;
    let (x, y) = (V::mul(b[0], V::splat(T)), V::mul(b[1], V::splat(T)));
    let (jx, jy) = (V::floor(x), V::floor(y));
    let (fx, fy) = (V::sub(x, jx), V::sub(y, jy));
    let p = cx.plan;
    let per = p.period(T as i32);
    let (mut jxs, mut jys) = ([0.; 4], [0.; 4]);
    V::store_f64(&mut jxs, jx);
    V::store_f64(&mut jys, jy);
    // 画素ごとに、縦の糸が上か（市松）と、その糸の乱数
    let mut vertical = [0.; 4];
    let mut thread = [0.; 4];
    for k in 0..V::N {
        let (jx, jy) = (jxs[k] as i32, jys[k] as i32);
        let over_vertical = (jx + jy).rem_euclid(2) == 0;
        vertical[k] = if over_vertical { 1. } else { 0. };
        thread[k] = if over_vertical {
            p.lattice(21, jx, 0, per)
        } else {
            p.lattice(22, 0, jy, per)
        };
    }
    let over = V::select(
        V::gt(V::load_f64(&vertical), V::splat(0.)),
        prof_lanes::<V>(fx),
        prof_lanes::<V>(fy),
    );
    let fiber = cx.layer_body::<V>(b, 0);
    V::mul(
        V::mul(
            V::add(V::splat(0.3), V::mul(V::splat(0.7), over)),
            V::add(V::splat(0.8), V::mul(V::splat(0.2), V::load_f64(&thread))),
        ),
        V::add(V::splat(0.88), V::mul(V::splat(0.24), fiber)),
    )
}
/// 織りの糸の断面（`1 - (2f - 1)²`）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn prof_lanes<V: Lanes>(f: V::F) -> V::F {
    let u = V::sub(V::mul(V::splat(2.), f), V::splat(1.));
    V::sub(V::splat(1.), V::mul(u, u))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn cracks_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let half = V::splat(0.5);
    let k = V::splat(0.45);
    let wx = V::mul(V::sub(cx.layer_body::<V>(b, 0), half), k);
    let wy = V::mul(V::sub(cx.layer_body::<V>(b, 1), half), k);
    let wz = if cx.plan.is_uv() {
        b[2]
    } else {
        V::add(b[2], V::mul(V::sub(cx.layer_body::<V>(b, 2), half), k))
    };
    let wb = [V::add(b[0], wx), V::add(b[1], wy), wz];
    let (f1, f2) = cx.distances_body::<V>(wb, 0);
    let width = V::add(
        V::splat(0.012),
        V::mul(V::splat(0.02), cx.layer_body::<V>(b, 3)),
    );
    let main = V::sub(
        V::splat(1.),
        smooth_edges_lanes::<V>(
            V::mul(width, V::splat(0.4)),
            V::mul(width, V::splat(1.2)),
            V::sub(f2, f1),
        ),
    );
    let (f1b, f2b) = cx.distances_body::<V>(wb, 1);
    let fine = V::mul(
        V::sub(
            V::splat(1.),
            smooth_lanes::<V>(0.01, 0.03, V::sub(f2b, f1b)),
        ),
        smooth_lanes::<V>(0.4, 0.6, cx.layer_body::<V>(b, 4)),
    );
    V::max(main, V::mul(V::splat(0.6), fine))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn drops_lanes<V: Lanes>(
    cx: &mut Ctx<'_>,
    b: [V::F; 3],
    frame: usize,
    biggest: f64,
) -> V::F {
    let (c, _) = cx.cells_body::<V>(b, frame);
    let r1 = unit24_lanes::<V>(c.id);
    let r2 = per_hash::<V>(c.id, |id| hash_unit(id ^ 0x7f4a_7c15));
    let radius = V::add(V::splat(0.04), V::mul(V::mul(V::splat(biggest), r1), r1));
    let v = V::sub(
        V::splat(1.),
        smooth_edges_lanes::<V>(V::mul(radius, V::splat(0.75)), radius, c.f1),
    );
    V::select(V::lt(r2, V::splat(0.45)), V::splat(0.), v)
}
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn splatter_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let big = drops_lanes::<V>(cx, b, 0, 0.32);
    let mid = drops_lanes::<V>(cx, b, 1, 0.3);
    let small = V::mul(drops_lanes::<V>(cx, b, 2, 0.28), V::splat(0.9));
    V::max(V::max(big, mid), small)
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn peeling_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let base = cx.layer_body::<V>(b, 0);
    let chip = cx.layer_body::<V>(b, 1);
    V::add(base, V::mul(V::splat(0.12), V::sub(chip, V::splat(0.5))))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn wood_grain_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let r = if cx.plan.is_uv() {
        V::mul(b[0], V::splat(4.))
    } else {
        V::mul(
            V::sqrt(V::add(
                V::mul(b[0], b[0]),
                V::mul(V::mul(b[2], b[2]), V::splat(0.85)),
            )),
            V::splat(2.5),
        )
    };
    let half = V::splat(0.5);
    let warp = V::add(
        V::mul(V::sub(cx.layer_body::<V>(b, 0), half), V::splat(1.6)),
        V::mul(V::sub(cx.layer_body::<V>(b, 1), half), V::splat(0.25)),
    );
    let rings = V::add(r, warp);
    let f = V::sub(rings, V::floor(rings));
    let ring = V::select(
        V::lt(f, V::splat(0.78)),
        V::div(f, V::splat(0.78)),
        V::div(V::sub(V::splat(1.), f), V::splat(0.22)),
    );
    let fiber = cx.layer_body::<V>(b, 2);
    V::mul(
        V::add(V::splat(0.2), V::mul(V::splat(0.8), ring)),
        V::add(V::splat(0.8), V::mul(V::splat(0.4), fiber)),
    )
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn pebbles_lanes<V: Lanes>(cx: &mut Ctx<'_>, b: [V::F; 3]) -> V::F {
    let (c, _) = cx.cells_body::<V>(b, 0);
    let border = smooth_lanes::<V>(0., 0.14, V::sub(c.f2, c.f1));
    let dome = V::sub(V::splat(1.), smooth_lanes::<V>(0., 0.6, c.f1));
    V::mul(
        V::mul(border, V::add(V::splat(0.5), V::mul(V::splat(0.5), dome))),
        V::add(
            V::splat(0.82),
            V::mul(V::splat(0.18), unit24_lanes::<V>(c.id)),
        ),
    )
}
