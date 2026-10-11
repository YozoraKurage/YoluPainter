//! 行ごとの評価。`BoundGenerator::value`・`apply` の 1 画素ずつの式と同じ値・同じバイトを、行の単位で出す。
//!
//! 画素ごとに変わらないもの（種類・ブレンド・ランプの有無・形・ノイズの大きさ）はループの外で決め、マップの行は添字の計算なしで読み、
//! ノイズの格子の値は同じ格子の中で使い回す。式の演算とその順は 1 画素ずつの式と同じで、結果のビットは変わらない（試験で全画素を比べる）。
//! 値を持たない画素（`Option::None`）は、有限の 0..1 の値と取り違えない印（[`none`]）で表す。
use super::*;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::generator::mixing::MixLanes;
use crate::generator::shape::{self, BOX, PLANE, SPHERE};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::Lanes;
#[cfg(target_arch = "aarch64")]
use crate::math::simd::Neon;
use crate::math::simd::{self, Level};
#[cfg(target_arch = "x86_64")]
use crate::math::simd::{Avx2, Sse41};

/// 「値なし」の印。画素の値は有限の 0..1 なので、この NaN とビットが一致することはない（式の途中で出る NaN は既定の NaN で、別のビット）。
const NONE: u64 = 0x7ff8_0000_0000_5eed;
#[inline(always)]
pub(super) fn none() -> f64 {
    f64::from_bits(NONE)
}
#[inline(always)]
pub(super) fn is_none(v: f64) -> bool {
    v.to_bits() == NONE
}

/// 1 本のスレッドが行をたどるあいだ使い回す作業領域。
pub(super) struct Scratch {
    pub(super) values: Vec<f64>,
    pub(super) aux: Aux,
}
impl Scratch {
    pub(super) fn new(g: &BoundGenerator<'_>, width: usize) -> Self {
        Self {
            values: vec![0.; width],
            aux: Aux::new(g),
        }
    }
}
/// 値の行を作るあいだの格子の覚え。
pub(super) struct Aux {
    /// 使う計算の道（SIMD の幅）。
    pub(super) level: Level,
    /// 重ねるノイズ（C# の種類）。
    noise: noise::Cache,
    /// ノイズ・グランジ。
    plan: Option<procedural::PlanScratch>,
}
impl Aux {
    pub(super) fn new(g: &BoundGenerator<'_>) -> Self {
        Self::with_level(g, simd::level())
    }
    /// この生成器の計画のために作ったものか（ノイズ・グランジの格子の覚えは計画の乱数で引いたもの）。道は今の `simd::level()` と同じか。
    fn fits(&self, g: &BoundGenerator<'_>) -> bool {
        self.level == simd::level().min(simd::detect())
            && match (&g.plan, &self.plan) {
                (None, None) => true,
                (Some(plan), Some(scratch)) => scratch.fits(plan),
                _ => false,
            }
    }
    /// 道を指定する（試験で道ごとに同じバイトになることを確かめる）。この CPU が持たない道は持つ道まで下げる。
    pub(super) fn with_level(g: &BoundGenerator<'_>, level: Level) -> Self {
        Self {
            level: level.min(simd::detect()),
            noise: noise::Cache::new(),
            plan: g.plan.as_ref().map(procedural::Plan::scratch),
        }
    }
}

/// 1 チャンネルのマップを行で読み、被覆 0 の画素は「値なし」、ほかは `f(値)`。
#[inline(always)]
fn scalar_row(m: &Map<'_>, at: usize, out: &mut [f64], f: impl Fn(f64) -> f64) {
    let n = out.len();
    let (cover, data) = (&m.coverage[at..at + n], &m.data[at..at + n]);
    for ((o, c), d) in out.iter_mut().zip(cover).zip(data) {
        *o = if *c == 0 {
            none()
        } else {
            f(*d as f64 / 65535.)
        };
    }
}
/// 3 チャンネルのマップを行で読み、被覆 0 の画素は「値なし」、ほかは `f(生の値の 3 つ)`。
#[inline(always)]
fn vector_row(m: &Map<'_>, at: usize, out: &mut [f64], f: impl Fn([f64; 3]) -> f64) {
    let n = out.len();
    let cover = &m.coverage[at..at + n];
    let data = &m.data[at * 3..(at + n) * 3];
    for ((o, c), d) in out.iter_mut().zip(cover).zip(data.chunks_exact(3)) {
        *o = if *c == 0 {
            none()
        } else {
            f([d[0] as f64, d[1] as f64, d[2] as f64])
        };
    }
}

impl BoundGenerator<'_> {
    /// 行 `y` の `x0` から `out.len()` 画素の値（`value` と同じ。値を持たない画素・範囲外・マップが使えない段は「値なし」）。
    /// `aux` のうち重ねるノイズの覚え（`aux.noise`）は呼ぶたびに空から始める。ノイズ・グランジの格子の覚え（`aux.plan`）は計画の番号と格子の位置で
    /// 引き当てるので、行をまたいで・呼びをまたいで使い回す（格子の角の値は hash だけで決まるので、使い回しても値は変わらない）。
    pub(super) fn value_row(&self, x0: u32, y: u32, out: &mut [f64], aux: &mut Aux) {
        let n = if y >= self.height || x0 >= self.width {
            0
        } else {
            out.len().min((self.width - x0) as usize)
        };
        let (head, tail) = out.split_at_mut(n);
        tail.fill(none());
        if self.inactive.is_some() {
            head.fill(none());
            return;
        }
        if n == 0 {
            return;
        }
        let at = y as usize * self.width as usize + x0 as usize;
        self.base_row(x0, y, at, head, aux.plan.as_mut(), aux.level);
        self.level_row(head, aux.level);
        if self.g.noise_amount > 0. {
            aux.noise = noise::Cache::new();
            self.noise_row(x0, y, at, head, &mut aux.noise, aux.level);
        }
    }

    /// 種類ごとの基底の値（レベルの前）。`at` は行の先頭の画素の添字。
    fn base_row(
        &self,
        x0: u32,
        y: u32,
        at: usize,
        out: &mut [f64],
        plan: Option<&mut procedural::PlanScratch>,
        level: Level,
    ) {
        let g = self.g;
        match g.kind {
            Kind::EdgeWear => scalar_row(self.map(MapKind::Curvature), at, out, |s| {
                clamp01(2. * (s - 0.5))
            }),
            Kind::Dirt => self.dirt_row(at, out),
            Kind::PositionGradient => {
                let axis = g.axis;
                vector_row(self.map(MapKind::Position), at, out, |v| v[axis] / 65535.)
            }
            Kind::Thickness => scalar_row(self.map(MapKind::Thickness), at, out, |s| s),
            Kind::Direction => {
                let [dx, dy, dz] = self.direction;
                let map = self.map(if g.use_bent_normal {
                    MapKind::BentNormal
                } else {
                    MapKind::WorldNormal
                });
                vector_row(map, at, out, |v| {
                    let [nx, ny, nz] = v.map(|v| v / 65535. * 2. - 1.);
                    let len = (nx * nx + ny * ny + nz * nz).sqrt();
                    let dot = if len > 1e-9 {
                        (nx * dx + ny * dy + nz * dz) / len
                    } else {
                        0.
                    };
                    clamp01((dot + 1.) * 0.5)
                })
            }
            Kind::IdColor => {
                let mut colors = [[0i32; 3]; 32];
                for (slot, c) in colors.iter_mut().zip(&g.id_colors) {
                    *slot = [
                        ((*c >> 16) & 255) as i32,
                        ((*c >> 8) & 255) as i32,
                        (*c & 255) as i32,
                    ];
                }
                let colors = &colors[..g.id_colors.len().min(32)];
                let tolerance = g.id_tolerance as i32;
                vector_row(self.map(MapKind::Id), at, out, |v| {
                    let rgb = v.map(|v| ((v as u32 + 128) / 257) as i32);
                    if colors
                        .iter()
                        .any(|c| c.iter().zip(rgb).all(|(c, v)| (c - v).abs() <= tolerance))
                    {
                        1.
                    } else {
                        0.
                    }
                })
            }
            Kind::Anchor => match self.anchor {
                Some(a) => {
                    a.value_row(x0, y, out);
                    for v in out.iter_mut() {
                        if v.is_nan() {
                            *v = none();
                        }
                    }
                }
                None => out.fill(none()),
            },
            Kind::Noise | Kind::Grunge => {
                self.procedural_row(x0, y, at, out, plan.expect("束縛済み"), level)
            }
            Kind::Image => {
                for (k, o) in out.iter_mut().enumerate() {
                    *o = self.image_value(x0 + k as u32, y).unwrap_or_else(none);
                }
            }
            Kind::UvIslandVariation => self.island_row(x0, y, out),
            // 模様・ライト・マスクの組み立ては 1 画素ずつ（SIMD にしない）
            Kind::Pattern | Kind::Light | Kind::MaskBuilder => {
                for (k, o) in out.iter_mut().enumerate() {
                    *o = self.base_050(x0 + k as u32, y, at + k).unwrap_or_else(none);
                }
            }
            Kind::ShapeGradient => {
                let local = g.volume.local();
                match local.shape() {
                    Shape::Box => self.shape_row::<BOX>(&local, at, out),
                    Shape::Sphere => self.shape_row::<SPHERE>(&local, at, out),
                    Shape::Plane => self.shape_row::<PLANE>(&local, at, out),
                }
            }
        }
    }

    /// ノイズ・グランジの行の基底の値。
    fn procedural_row(
        &self,
        x0: u32,
        y: u32,
        at: usize,
        out: &mut [f64],
        scratch: &mut procedural::PlanScratch,
        level: Level,
    ) {
        match level {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            Level::Avx2 => unsafe { procedural_row_avx2(self, x0, y, at, out, scratch) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            Level::Sse41 => unsafe { procedural_row_sse41(self, x0, y, at, out, scratch) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
            Level::Neon => unsafe { procedural_row_neon(self, x0, y, at, out, scratch) },
            Level::Scalar => self.procedural_scalar(x0, y, at, out, scratch),
        }
    }
    fn procedural_scalar(
        &self,
        x0: u32,
        y: u32,
        at: usize,
        out: &mut [f64],
        scratch: &mut procedural::PlanScratch,
    ) {
        for (k, o) in out.iter_mut().enumerate() {
            *o = self
                .procedural_value_with(x0 + k as u32, y, at + k, scratch)
                .unwrap_or_else(none);
        }
    }

    /// アイランドごとのばらつきの行の基底の値。アイランドの図の行の連なりを歩き、連なりごとに 1 回だけ値を求めて埋める（`island_value` と同じ式）。
    fn island_row(&self, x0: u32, y: u32, out: &mut [f64]) {
        out.fill(none());
        let Some(map) = &self.islands else {
            return;
        };
        let x1 = x0 + out.len() as u32;
        let row = map.row(y);
        let first = row.partition_point(|r| r.end <= x0);
        for r in &row[first..] {
            if r.start >= x1 {
                break;
            }
            if r.island == 0 {
                continue;
            }
            let v = self.g.island.value_in(self.island_stream, r.island);
            let (a, b) = (r.start.max(x0), r.end.min(x1));
            out[(a - x0) as usize..(b - x0) as usize].fill(v);
        }
    }

    fn dirt_row(&self, at: usize, out: &mut [f64]) {
        let g = self.g;
        let ao = (g.balance < 1.).then(|| self.map(MapKind::AmbientOcclusion));
        let curvature = (g.balance > 0.).then(|| self.map(MapKind::Curvature));
        for (k, o) in out.iter_mut().enumerate() {
            let i = at + k;
            let mut b = 0.;
            let mut seen = true;
            if let Some(m) = ao {
                if m.coverage[i] == 0 {
                    seen = false;
                } else {
                    b += (1. - g.balance) * (1. - m.data[i] as f64 / 65535.);
                }
            }
            if let Some(m) = curvature {
                if m.coverage[i] == 0 {
                    seen = false;
                } else {
                    b += g.balance * clamp01(2. * (0.5 - m.data[i] as f64 / 65535.));
                }
            }
            *o = if seen { b } else { none() };
        }
    }

    fn shape_row<const SHAPE: u8>(&self, local: &shape::Local, at: usize, out: &mut [f64]) {
        let m = self.matrix;
        let k = self.offset;
        vector_row(self.map(MapKind::Position), at, out, |[x, y, z]| {
            local.eval::<SHAPE>([
                m[0] * x + m[1] * y + m[2] * z + k[0],
                m[3] * x + m[4] * y + m[5] * z + k[1],
                m[6] * x + m[7] * y + m[8] * z + k[2],
            ])
        });
    }

    /// レベル（low・high）・減衰・反転。
    fn level_row(&self, out: &mut [f64], level: Level) {
        match level {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            Level::Avx2 => unsafe { level_row_avx2(self, out) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            Level::Sse41 => unsafe { level_row_sse41(self, out) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
            Level::Neon => unsafe { level_row_neon(self, out) },
            Level::Scalar => self.level_scalar(out),
        }
    }
    fn level_scalar(&self, out: &mut [f64]) {
        let g = self.g;
        for v in out.iter_mut() {
            if is_none(*v) {
                continue;
            }
            let mut t = clamp01((*v - g.low) / (g.high - g.low));
            if g.softness > 0. {
                t += g.softness * (t * t * (3. - 2. * t) - t);
            }
            if g.invert {
                t = 1. - t;
            }
            *v = t;
        }
    }

    /// 重ねるノイズ（UV の空間は画素の位置から、モデルの空間は Position から）。
    fn noise_row(
        &self,
        x0: u32,
        y: u32,
        at: usize,
        out: &mut [f64],
        cache: &mut noise::Cache,
        level: Level,
    ) {
        match level {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            Level::Avx2 => unsafe { noise_row_avx2(self, x0, y, at, out, cache) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            Level::Sse41 => unsafe { noise_row_sse41(self, x0, y, at, out, cache) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
            Level::Neon => unsafe { noise_row_neon(self, x0, y, at, out, cache) },
            Level::Scalar => self.noise_scalar(x0, y, at, out, cache),
        }
    }
    fn noise_scalar(&self, x0: u32, y: u32, at: usize, out: &mut [f64], cache: &mut noise::Cache) {
        let g = self.g;
        let amount = g.noise_amount;
        let seeds = self.seeds;
        let gain = |t: &mut f64, p: [f64; 3], cache: &mut noise::Cache| {
            let m = clamp01((noise::fractal(p, seeds, cache) - 0.3) / 0.4);
            *t *= 1. - amount * (m * m * (3. - 2. * m));
        };
        if g.noise_space == NoiseSpace::Uv {
            let kx = 1. / self.width as f64 / g.noise_scale;
            let ky = 1. / self.height as f64 / g.noise_scale;
            let py = (y as f64 + 0.5) * ky;
            for (k, t) in out.iter_mut().enumerate() {
                if is_none(*t) {
                    continue;
                }
                gain(t, [((x0 + k as u32) as f64 + 0.5) * kx, py, 0.], cache);
            }
        } else {
            let m = self.map(MapKind::Position);
            let scale = self.noise_scale;
            for (k, t) in out.iter_mut().enumerate() {
                if is_none(*t) {
                    continue;
                }
                let i = at + k;
                if m.coverage[i] == 0 {
                    *t = none();
                    continue;
                }
                let d = &m.data[i * 3..i * 3 + 3];
                let p = [
                    d[0] as f64 * scale[0],
                    d[1] as f64 * scale[1],
                    d[2] as f64 * scale[2],
                ];
                gain(t, p, cache);
            }
        }
    }

    /// 画像の段の色の対象: 行の画素に、画素ごとの画像の色を合成する（`apply_with` のランプの色と同じ式。透明な画素・値の無い画素は入力のまま）。
    pub(super) fn apply_image_row(&self, dst: &mut [u8], x0: u32, y: u32, strength: f64) {
        match self.g.blend {
            Blend::Multiply => self.apply_image_with::<Multiply>(dst, x0, y, strength),
            Blend::Replace => self.apply_image_with::<Replace>(dst, x0, y, strength),
            Blend::Screen => self.apply_image_with::<Screen>(dst, x0, y, strength),
            Blend::Max => self.apply_image_with::<Max>(dst, x0, y, strength),
            Blend::Min => self.apply_image_with::<Min>(dst, x0, y, strength),
            Blend::Add => self.apply_image_with::<Add>(dst, x0, y, strength),
            Blend::Subtract => self.apply_image_with::<Subtract>(dst, x0, y, strength),
        }
    }
    fn apply_image_with<B: Op>(&self, dst: &mut [u8], x0: u32, y: u32, strength: f64) {
        let u = |b: u8| UNIT[b as usize];
        for (k, p) in dst.chunks_exact_mut(4).enumerate() {
            if p[3] == 0 {
                continue;
            }
            let Some(m) = self.image_color(x0 + k as u32, y) else {
                continue;
            };
            for c in 0..3 {
                p[c] = to_byte(mix::<B>(u(p[c]), u(m[c]), strength));
            }
            p[3] = to_byte(u(p[3]) * (1. - strength + strength * u(m[3])));
        }
    }

    /// 行の画素を `values`（`value_row` の結果）で合成する。`dst` は入力の行（Mask の対象では RGB が 0）で、その場で書き換える。
    pub(super) fn apply_row(
        &self,
        dst: &mut [u8],
        values: &[f64],
        target: Target,
        strength: f64,
        level: Level,
    ) {
        match self.g.blend {
            Blend::Multiply => self.apply_at::<Multiply>(dst, values, target, strength, level),
            Blend::Replace => self.apply_at::<Replace>(dst, values, target, strength, level),
            Blend::Screen => self.apply_at::<Screen>(dst, values, target, strength, level),
            Blend::Max => self.apply_at::<Max>(dst, values, target, strength, level),
            Blend::Min => self.apply_at::<Min>(dst, values, target, strength, level),
            Blend::Add => self.apply_at::<Add>(dst, values, target, strength, level),
            Blend::Subtract => self.apply_at::<Subtract>(dst, values, target, strength, level),
        }
    }
    fn apply_at<B: Op>(
        &self,
        dst: &mut [u8],
        values: &[f64],
        target: Target,
        strength: f64,
        level: Level,
    ) {
        match level {
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
            Level::Avx2 => unsafe { apply_avx2::<B>(self, dst, values, target, strength) },
            #[cfg(target_arch = "x86_64")]
            // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
            Level::Sse41 => unsafe { apply_sse41::<B>(self, dst, values, target, strength) },
            #[cfg(target_arch = "aarch64")]
            // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
            Level::Neon => unsafe { apply_neon::<B>(self, dst, values, target, strength) },
            Level::Scalar => self.apply_with::<B>(dst, values, target, strength),
        }
    }

    fn apply_with<B: Op>(&self, dst: &mut [u8], values: &[f64], target: Target, strength: f64) {
        let ramp = self.g.ramp.as_ref();
        let scalar = target != Target::Color;
        let u = |b: u8| UNIT[b as usize];
        for (p, &v) in dst.chunks_exact_mut(4).zip(values) {
            if is_none(v) {
                continue;
            }
            if target == Target::Mask {
                let value = match ramp {
                    None => v,
                    Some(r) => {
                        let m = r.evaluate_unchecked(v, scalar).to_array();
                        u(m[0]) * u(m[3])
                    }
                };
                p[3] = 255 - to_byte(mix::<B>(1. - u(p[3]), value, strength));
                continue;
            }
            if p[3] == 0 {
                continue;
            }
            let (rgb, a) = match ramp {
                None => ([v; 3], p[3]),
                Some(r) => {
                    let m = r.evaluate_unchecked(v, scalar).to_array();
                    (
                        [u(m[0]), u(m[1]), u(m[2])],
                        to_byte(u(p[3]) * (1. - strength + strength * u(m[3]))),
                    )
                }
            };
            p[0] = to_byte(mix::<B>(u(p[0]), rgb[0], strength));
            p[1] = to_byte(mix::<B>(u(p[1]), rgb[1], strength));
            p[2] = to_byte(mix::<B>(u(p[2]), rgb[2], strength));
            p[3] = a;
        }
    }

    /// 行 `y` の `x0` から `out.len()` 画素の `sample` と同じ結果。範囲外の画素は `None`。1 画素ずつ `sample` を呼ぶより速い（同じ行の
    /// 値を SIMD でまとめて作る）。作業領域はスレッドごとに持って使い回すので、行ごとに呼んでも確保し直さない。
    pub fn sample_row(&self, x0: u32, y: u32, scalar: bool, out: &mut [Option<Generated>]) {
        // 画像の段の色は画素ごとの画像の色（値の行を作らない）
        if self.g.kind == Kind::Image && !scalar {
            for (i, o) in out.iter_mut().enumerate() {
                *o = self.sample(x0 + i as u32, y, false);
            }
            return;
        }
        use std::cell::RefCell;
        thread_local! {
            static ROW: RefCell<(Vec<f64>, Option<Aux>)> = const { RefCell::new((Vec::new(), None)) };
        }
        ROW.with(|cell| {
            let (values, aux) = &mut *cell.borrow_mut();
            let aux = match aux {
                Some(aux) if aux.fits(self) => aux,
                other => other.insert(Aux::new(self)),
            };
            values.clear();
            values.resize(out.len(), 0.);
            self.value_row(x0, y, values, aux);
            match &self.g.ramp {
                None => {
                    for (o, v) in out.iter_mut().zip(values.iter()) {
                        *o = (!is_none(*v)).then_some(Generated::Scalar(*v));
                    }
                }
                Some(r) => ramp_row(r, values, scalar, out, aux.level),
            }
        });
    }
}

/// 値の行をランプの色にして `Generated::Mapped` に（値なしは `None`）。
fn ramp_row(r: &Ramp, values: &[f64], scalar: bool, out: &mut [Option<Generated>], level: Level) {
    match level {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、AVX2 と FMA を持つ
        Level::Avx2 => unsafe { ramp_row_avx2(r, values, scalar, out) },
        #[cfg(target_arch = "x86_64")]
        // SAFETY: level は detect() 以下なので、SSE4.1 を持つ
        Level::Sse41 => unsafe { ramp_row_sse41(r, values, scalar, out) },
        #[cfg(target_arch = "aarch64")]
        // SAFETY: level は detect() 以下なので、NEON を持つ（aarch64 の基本の命令）
        Level::Neon => unsafe { ramp_row_neon(r, values, scalar, out) },
        Level::Scalar => ramp_scalar(r, values, scalar, out),
    }
}
fn ramp_scalar(r: &Ramp, values: &[f64], scalar: bool, out: &mut [Option<Generated>]) {
    for (o, v) in out.iter_mut().zip(values) {
        *o = (!is_none(*v)).then(|| Generated::Mapped(r.evaluate_unchecked(*v, scalar).to_array()));
    }
}

// ───────── SIMD の道（N 画素を 1 組で） ─────────
//
// 1 組の N 画素が全部「値あり」（被覆も 0 でない）なら N 画素を同時に計算し、そうでない組は 1 画素ずつの式で計算する。
// レーンの式は 1 画素の式と同じ演算を同じ順に並べたもの。

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn level_row_avx2(g: &BoundGenerator<'_>, out: &mut [f64]) {
    level_row_lanes::<Avx2>(g, out)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn level_row_sse41(g: &BoundGenerator<'_>, out: &mut [f64]) {
    level_row_lanes::<Sse41>(g, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn level_row_neon(g: &BoundGenerator<'_>, out: &mut [f64]) {
    level_row_lanes::<Neon>(g, out)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn noise_row_avx2(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    cache: &mut noise::Cache,
) {
    noise_row_lanes::<Avx2>(g, x0, y, at, out, cache)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn noise_row_sse41(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    cache: &mut noise::Cache,
) {
    noise_row_lanes::<Sse41>(g, x0, y, at, out, cache)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn noise_row_neon(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    cache: &mut noise::Cache,
) {
    noise_row_lanes::<Neon>(g, x0, y, at, out, cache)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn procedural_row_avx2(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    scratch: &mut procedural::PlanScratch,
) {
    procedural_row_lanes::<Avx2>(g, x0, y, at, out, scratch)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn procedural_row_sse41(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    scratch: &mut procedural::PlanScratch,
) {
    procedural_row_lanes::<Sse41>(g, x0, y, at, out, scratch)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn procedural_row_neon(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    scratch: &mut procedural::PlanScratch,
) {
    procedural_row_lanes::<Neon>(g, x0, y, at, out, scratch)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn apply_avx2<B: Op>(
    g: &BoundGenerator<'_>,
    dst: &mut [u8],
    values: &[f64],
    target: Target,
    strength: f64,
) {
    apply_lanes::<Avx2, B>(g, dst, values, target, strength)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn apply_sse41<B: Op>(
    g: &BoundGenerator<'_>,
    dst: &mut [u8],
    values: &[f64],
    target: Target,
    strength: f64,
) {
    apply_lanes::<Sse41, B>(g, dst, values, target, strength)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn apply_neon<B: Op>(
    g: &BoundGenerator<'_>,
    dst: &mut [u8],
    values: &[f64],
    target: Target,
    strength: f64,
) {
    apply_lanes::<Neon, B>(g, dst, values, target, strength)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn ramp_row_avx2(r: &Ramp, values: &[f64], scalar: bool, out: &mut [Option<Generated>]) {
    ramp_lanes::<Avx2>(r, values, scalar, out)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn ramp_row_sse41(r: &Ramp, values: &[f64], scalar: bool, out: &mut [Option<Generated>]) {
    ramp_lanes::<Sse41>(r, values, scalar, out)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn ramp_row_neon(r: &Ramp, values: &[f64], scalar: bool, out: &mut [Option<Generated>]) {
    ramp_lanes::<Neon>(r, values, scalar, out)
}

/// 組の値が全部「値あり」か。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn present(group: &[f64]) -> bool {
    group.iter().all(|v| !is_none(*v))
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn level_row_lanes<V: Lanes>(g: &BoundGenerator<'_>, out: &mut [f64]) {
    let s = g.g;
    let (low, span) = (V::splat(s.low), V::splat(s.high - s.low));
    let (one, two, three) = (V::splat(1.), V::splat(2.), V::splat(3.));
    let softness = V::splat(s.softness);
    let mut k = 0;
    while k + V::N <= out.len() {
        let group = &mut out[k..k + V::N];
        if present(group) {
            let v = V::load_f64(group);
            let mut t = simd::clamp01::<V>(V::div(V::sub(v, low), span));
            if s.softness > 0. {
                let smooth = V::mul(V::mul(t, t), V::sub(three, V::mul(two, t)));
                t = V::add(t, V::mul(softness, V::sub(smooth, t)));
            }
            if s.invert {
                t = V::sub(one, t);
            }
            V::store_f64(group, t);
        } else {
            g.level_scalar(group);
        }
        k += V::N;
    }
    g.level_scalar(&mut out[k..]);
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn noise_row_lanes<V: Lanes>(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    cache: &mut noise::Cache,
) {
    let s = g.g;
    let uv = s.noise_space == NoiseSpace::Uv;
    let (kx, ky) = (
        1. / g.width as f64 / s.noise_scale,
        1. / g.height as f64 / s.noise_scale,
    );
    let py = V::splat((y as f64 + 0.5) * ky);
    let (zero, one, two, three) = (V::splat(0.), V::splat(1.), V::splat(2.), V::splat(3.));
    let (amount, offset, span) = (V::splat(s.noise_amount), V::splat(0.3), V::splat(0.4));
    let position = if uv {
        None
    } else {
        Some(g.map(MapKind::Position))
    };
    let scale = g.noise_scale;
    let mut k = 0;
    while k + V::N <= out.len() {
        let group = &mut out[k..k + V::N];
        let i0 = at + k;
        let ready = present(group)
            && position.is_none_or(|m| m.coverage[i0..i0 + V::N].iter().all(|c| *c != 0));
        if !ready {
            g.noise_scalar(x0 + k as u32, y, i0, group, cache);
            k += V::N;
            continue;
        }
        let p = match position {
            None => {
                let x = x0 + k as u32;
                [V::from_fn(|j| ((x + j as u32) as f64 + 0.5) * kx), py, zero]
            }
            Some(m) => {
                let d = &m.data[i0 * 3..(i0 + V::N) * 3];
                [
                    V::from_fn(|j| d[3 * j] as f64 * scale[0]),
                    V::from_fn(|j| d[3 * j + 1] as f64 * scale[1]),
                    V::from_fn(|j| d[3 * j + 2] as f64 * scale[2]),
                ]
            }
        };
        let f = noise::fractal_lanes::<V>(p, g.seeds, cache);
        let m = simd::clamp01::<V>(V::div(V::sub(f, offset), span));
        let smooth = V::mul(V::mul(m, m), V::sub(three, V::mul(two, m)));
        let t = V::load_f64(group);
        V::store_f64(group, V::mul(t, V::sub(one, V::mul(amount, smooth))));
        k += V::N;
    }
    g.noise_scalar(x0 + k as u32, y, at + k, &mut out[k..], cache);
}

/// 3 チャンネルのマップの N 画素（添字 `i0` から）を、チャンネルごとのレーンの生の値に（マップが無ければ `none`）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn gather_lanes<V: Lanes>(m: Option<&Map<'_>>, i0: usize, none: [V::F; 3]) -> [V::F; 3] {
    match m {
        None => none,
        Some(m) => {
            let d = &m.data[i0 * 3..(i0 + V::N) * 3];
            [
                V::from_fn(|j| d[3 * j] as f64),
                V::from_fn(|j| d[3 * j + 1] as f64),
                V::from_fn(|j| d[3 * j + 2] as f64),
            ]
        }
    }
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn procedural_row_lanes<V: procedural::GenLanes>(
    g: &BoundGenerator<'_>,
    x0: u32,
    y: u32,
    at: usize,
    out: &mut [f64],
    scratch: &mut procedural::PlanScratch,
) {
    let plan = g.plan.as_ref().expect("束縛済み");
    let size = (g.width, g.height);
    let position = plan.needs_position().then(|| g.map(MapKind::Position));
    let normal = (plan.mode == procedural::Mode::Triplanar).then(|| g.map(MapKind::WorldNormal));
    let zero = [V::splat(0.); 3];
    let mut k = 0;
    while k + V::N <= out.len() {
        let i0 = at + k;
        let covered = |m: Option<&Map<'_>>| {
            m.is_none_or(|m| m.coverage[i0..i0 + V::N].iter().all(|c| *c != 0))
        };
        if covered(position) && covered(normal) {
            if let Some(v) = plan.value_lanes::<V>(
                x0 + k as u32,
                y,
                size,
                gather_lanes::<V>(position, i0, zero),
                gather_lanes::<V>(normal, i0, zero),
                scratch,
            ) {
                V::store_f64(&mut out[k..k + V::N], v);
                k += V::N;
                continue;
            }
        }
        // 被覆の無い画素・向きの定まらない画素を含む組は 1 画素ずつ
        g.procedural_scalar(x0 + k as u32, y, i0, &mut out[k..k + V::N], scratch);
        k += V::N;
    }
    g.procedural_scalar(x0 + k as u32, y, at + k, &mut out[k..], scratch);
}

#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn ramp_lanes<V: MixLanes>(
    r: &Ramp,
    values: &[f64],
    scalar: bool,
    out: &mut [Option<Generated>],
) {
    let n = values.len().min(out.len());
    let mut k = 0;
    while k + V::N <= n {
        let group = &values[k..k + V::N];
        if present(group) {
            let m = r.evaluate_lanes::<V>(V::load_f64(group), scalar);
            let mut bytes = [[0.; 4]; 4];
            for (channel, lane) in m.into_iter().enumerate() {
                V::store_f64(&mut bytes[channel], lane);
            }
            for (j, o) in out[k..k + V::N].iter_mut().enumerate() {
                *o = Some(Generated::Mapped(bytes.map(|c| c[j] as u8)));
            }
        } else {
            ramp_scalar(r, group, scalar, &mut out[k..k + V::N]);
        }
        k += V::N;
    }
    ramp_scalar(r, &values[k..n], scalar, &mut out[k..n]);
}

/// 元の値 `s` に `v` を式で重ねて、強さで元の値との間を取る（`mix` の N 画素ぶん）。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn mix_lanes<V: Lanes, B: Op>(s: V::F, v: V::F, strength: f64) -> V::F {
    let c = B::op_lanes::<V>(s, v);
    if strength >= 1. {
        c
    } else {
        V::add(s, V::mul(V::sub(c, s), V::splat(strength)))
    }
}

/// 行の合成（`apply_with` の N 画素ぶん）。組の値が全部「値あり」なら N 画素を同時に、そうでない組は 1 画素ずつ。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn apply_lanes<V: MixLanes, B: Op>(
    g: &BoundGenerator<'_>,
    dst: &mut [u8],
    values: &[f64],
    target: Target,
    strength: f64,
) {
    let ramp = g.g.ramp.as_ref();
    let scalar = target != Target::Color;
    let (zero, one, top) = (V::splat(0.), V::splat(1.), V::splat(255.));
    let n = values.len().min(dst.len() / 4);
    let mut k = 0;
    while k + V::N <= n {
        let (group, px) = (&values[k..k + V::N], &mut dst[k * 4..(k + V::N) * 4]);
        if !present(group) {
            g.apply_with::<B>(px, group, target, strength);
            k += V::N;
            continue;
        }
        let v = V::load_f64(group);
        let src = V::load(px);
        if target == Target::Mask {
            let value = match ramp {
                None => v,
                Some(r) => {
                    let m = r.evaluate_lanes::<V>(v, scalar);
                    V::mul(V::unit(m[0]), V::unit(m[3]))
                }
            };
            let covered = mix_lanes::<V, B>(V::sub(one, V::unit(src[3])), value, strength);
            V::store(
                px,
                [zero, zero, zero, V::sub(top, simd::to_byte::<V>(covered))],
            );
        } else {
            let (rgb, alpha) = match ramp {
                None => ([v; 3], src[3]),
                Some(r) => {
                    let m = r.evaluate_lanes::<V>(v, scalar);
                    (
                        [V::unit(m[0]), V::unit(m[1]), V::unit(m[2])],
                        simd::to_byte::<V>(V::mul(
                            V::unit(src[3]),
                            V::add(
                                V::splat(1. - strength),
                                V::mul(V::splat(strength), V::unit(m[3])),
                            ),
                        )),
                    )
                }
            };
            let out = [
                simd::to_byte::<V>(mix_lanes::<V, B>(V::unit(src[0]), rgb[0], strength)),
                simd::to_byte::<V>(mix_lanes::<V, B>(V::unit(src[1]), rgb[1], strength)),
                simd::to_byte::<V>(mix_lanes::<V, B>(V::unit(src[2]), rgb[2], strength)),
                alpha,
            ];
            // 完全に透明な画素は入力のまま
            let keep = V::eq(src[3], zero);
            V::store(
                px,
                [
                    V::select(keep, src[0], out[0]),
                    V::select(keep, src[1], out[1]),
                    V::select(keep, src[2], out[2]),
                    V::select(keep, src[3], out[3]),
                ],
            );
        }
        k += V::N;
    }
    g.apply_with::<B>(&mut dst[k * 4..n * 4], &values[k..n], target, strength);
}

/// ブレンドの式（種類ごとに型にして、行のループの外で選ぶ）。`op_lanes` は N 画素ぶん（同じ式をレーンで）。
pub(super) trait Op {
    fn op(s: f64, v: f64) -> f64;
    /// # Safety
    /// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    unsafe fn op_lanes<V: Lanes>(s: V::F, v: V::F) -> V::F;
}
macro_rules! ops {
    ($($name:ident($s:ident, $v:ident) => $e:expr, $lanes:expr;)*) => {$(
        pub(super) struct $name;
        impl Op for $name {
            #[inline(always)]
            fn op($s: f64, $v: f64) -> f64 { $e }
            #[inline(always)]
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            unsafe fn op_lanes<V: Lanes>($s: V::F, $v: V::F) -> V::F { $lanes }
        }
    )*};
}
ops! {
    Multiply(s, v) => s * v, V::mul(s, v);
    Replace(_s, v) => v, v;
    Screen(s, v) => 1. - (1. - s) * (1. - v),
        V::sub(V::splat(1.), V::mul(V::sub(V::splat(1.), s), V::sub(V::splat(1.), v)));
    Max(s, v) => s.max(v), V::max(s, v);
    Min(s, v) => s.min(v), V::min(s, v);
    Add(s, v) => (s + v).min(1.), V::min(V::add(s, v), V::splat(1.));
    Subtract(s, v) => (s - v).max(0.), V::max(V::sub(s, v), V::splat(0.));
}
/// 元の値 `s` に `v` を式で重ねて、強さで元の値との間を取る。
#[inline(always)]
pub(super) fn mix<B: Op>(s: f64, v: f64, strength: f64) -> f64 {
    let c = B::op(s, v);
    if strength >= 1. {
        c
    } else {
        s + (c - s) * strength
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{ColorStop, MixMode, OpacityStop, ProceduralSpace, Ramp};
    use crate::Rgba8;

    const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const W: u32 = 37;
    const H: u32 = 21;

    /// 位置・向き・曲率の合成のマップ（値がなめらかに動く所と飛ぶ所があり、被覆 0 の画素もある）と、入力の画像。
    struct Scene {
        position: Vec<u16>,
        normal: Vec<u16>,
        curvature: Vec<u16>,
        cover: Vec<u8>,
        input: Vec<u8>,
    }
    impl Scene {
        fn new() -> Self {
            let (mut position, mut normal, mut curvature, mut cover, mut input) =
                (vec![], vec![], vec![], vec![], vec![]);
            let mut state = 0x1234_5678u32;
            let mut next = move || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state >> 8
            };
            for y in 0..H {
                for x in 0..W {
                    let jump = (x + y) % 9 == 0;
                    position.extend([
                        (if jump {
                            next() % 65536
                        } else {
                            x * 1700 + y * 90
                        }) as u16,
                        (if jump {
                            next() % 65536
                        } else {
                            y * 2900 + x * 40
                        }) as u16,
                        ((x * 11 + y * 17) % 60 * 1000) as u16,
                    ]);
                    normal.extend([
                        (30000 + (x * 700) % 9000) as u16,
                        (40000 + (y * 500) % 9000) as u16,
                        (20000 + ((x + y) * 300) % 20000) as u16,
                    ]);
                    curvature.push(((x * 1900 + y * 300) % 65536) as u16);
                    cover.push(u8::from((x * 3 + y * 5) % 13 != 0));
                    input.extend([
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                        match (x + 2 * y) % 11 {
                            0 => 0,
                            1 => 255,
                            _ => (next() % 256) as u8,
                        },
                    ]);
                }
            }
            Self {
                position,
                normal,
                curvature,
                cover,
                input,
            }
        }
        fn maps(&self) -> Vec<Map<'_>> {
            let map = |kind, data| Map {
                kind,
                width: W,
                height: H,
                data,
                coverage: &self.cover,
                bounds_min: [-1., -2., -3.],
                bounds_max: [2., 3., 1.],
                condition_key: KEY,
                state: MapState::Current,
            };
            vec![
                map(MapKind::Position, &self.position),
                map(MapKind::WorldNormal, &self.normal),
                map(MapKind::Curvature, &self.curvature),
            ]
        }
    }

    fn two_stop_ramp(mode: MixMode) -> Ramp {
        let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
            position,
            color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            midpoint,
        };
        Ramp::new(
            vec![
                stop(0.05, [200, 30, 10], 0.3),
                stop(0.5, [20, 220, 60], 0.7),
                stop(0.95, [10, 40, 250], 0.5),
            ],
            vec![
                OpacityStop {
                    position: 0.,
                    opacity: 0.9,
                    midpoint: 0.4,
                },
                OpacityStop {
                    position: 1.,
                    opacity: 0.2,
                    midpoint: 0.5,
                },
            ],
            None,
        )
        .unwrap()
        .with_mixing(mode, crate::generator::LuminanceCorrection::High)
    }

    fn settings() -> Vec<(String, Settings)> {
        let mut all = Vec::new();
        for (label, amount, space) in [
            ("none", 0., NoiseSpace::Model),
            ("model", 0.6, NoiseSpace::Model),
            ("uv", 0.6, NoiseSpace::Uv),
        ] {
            let mut s = Settings::new(Kind::EdgeWear);
            s.noise_amount = amount;
            s.noise_space = space;
            s.noise_scale = 0.08;
            s.softness = 0.4;
            all.push((format!("edge-{label}"), s));
        }
        let mut direction = Settings::new(Kind::Direction);
        direction.invert = true;
        all.push(("direction".into(), direction));
        for (label, ramp) in [
            ("plain", None),
            ("standard", Some(two_stop_ramp(MixMode::Standard))),
            ("perceptual", Some(two_stop_ramp(MixMode::Perceptual))),
        ] {
            let mut s = Settings::new(Kind::ShapeGradient);
            s.ramp = ramp;
            s.volume.rotation = [10., 20., 30.];
            s.noise_amount = 0.3;
            all.push((format!("shape-{label}"), s));
        }
        let mut noise = Settings::new(Kind::Noise);
        noise.procedural.seed = 3;
        noise.procedural.scale = 0.1;
        all.push(("noise".into(), noise));
        let mut grunge = Settings::grunge(crate::generator::GrungePreset::Cracks);
        grunge.procedural.bleed = 0.4;
        grunge.procedural.space = ProceduralSpace::Triplanar;
        all.push(("grunge".into(), grunge));
        all
    }

    /// AVX2・SSE4.1・NEON・スカラーのどの道でも、全部のブレンド・対象・強さ・種類・ランプで `evaluate` のバイトが同じ。
    #[test]
    fn every_simd_level_gives_the_same_bytes() {
        let scene = Scene::new();
        let maps = scene.maps();
        let image = Image::new(&scene.input, W, H).unwrap();
        let blends = [
            Blend::Multiply,
            Blend::Replace,
            Blend::Screen,
            Blend::Max,
            Blend::Min,
            Blend::Add,
            Blend::Subtract,
        ];
        // この CPU が持つ道（x86_64 は最大 3 つ、aarch64 は NEON とスカラー。ほかの CPU はスカラーだけで、スカラーどうしを比べるので違いは出ない）
        let levels = simd::forced::supported();
        assert!(levels.contains(&Level::Scalar), "{levels:?}");
        let (mut cases, mut compared) = (0, 0);
        for (name, base) in settings() {
            for (i, blend) in blends.into_iter().enumerate() {
                for target in [Target::Color, Target::Scalar, Target::Mask] {
                    for strength in [1., 0.43] {
                        let mut s = base.clone();
                        s.blend = blend;
                        let bound = BoundGenerator::bind(
                            &s,
                            &maps,
                            Some(crate::generator::ModelFrame::default()),
                            (W, H),
                            Err(anchor::Issue::NotChosen),
                        )
                        .unwrap();
                        let run = |level| {
                            simd::forced::with_level(level, || {
                                evaluate(
                                    &image,
                                    &bound,
                                    Rect::new(0, 0, W, H),
                                    target,
                                    strength,
                                    &Options::default(),
                                )
                                .unwrap()
                                .pixels
                            })
                        };
                        cases += 1;
                        let reference = run(Level::Scalar);
                        for &level in &levels {
                            assert!(
                                run(level) == reference,
                                "{name} {blend:?} {target:?} 強さ {strength} {level:?} がスカラーと違う（{i}）"
                            );
                            compared += 1;
                        }
                    }
                }
            }
        }
        // 組み合わせの数は道の数によらない。比べた回数は、組み合わせ × この CPU の道の数（決め打ちの数と比べない）
        assert!(cases > 300, "{cases}");
        assert_eq!(compared, cases * levels.len());
    }
}
