mod rows;
use super::*;
use crate::{
    math::{clamp01, to_byte, UNIT},
    Rect,
};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Color,
    Scalar,
    Mask,
}
/// 1 画素の Generator の出力。`Scalar` は有限の 0..1。`Mapped` はランプ評価済みの straight RGBA8（[r, g, b, a]）。
/// rsfilter の `filter::Generated` と同じ形なので、`GeneratorInput` の実装は `BoundGenerator::sample` の結果をそのまま返せる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Generated {
    Scalar(f64),
    Mapped([u8; 4]),
}
/// 入力は外部所有。予算は返却する RGBA8 のみ（作業用画素バッファは持たない）。
pub struct Options<'a> {
    pub budget_bytes: u64,
    pub cancel: Option<&'a AtomicBool>,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            budget_bytes: 256 * 1024 * 1024,
            cancel: None,
        }
    }
}
impl Options<'_> {
    pub(super) fn check(&self) -> Result<(), Error> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
pub struct Output {
    pub pixels: Vec<u8>,
    pub inactive: Option<Inactive>,
}
/// 検査済みの入力と前計算。設定と画像を借用するため、評価中には変更できない。
pub struct BoundGenerator<'a> {
    g: &'a Settings,
    width: u32,
    height: u32,
    maps: [Option<&'a Map<'a>>; 10],
    anchor: Option<&'a dyn anchor::ValueSource>,
    inactive: Option<Inactive>,
    direction: [f64; 3],
    noise_scale: [f64; 3],
    seeds: [u32; 4],
    matrix: [f64; 9],
    offset: [f64; 3],
    /// ノイズ・グランジの評価の計画（それ以外の種類は None）。
    plan: Option<procedural::Plan>,
}
impl<'a> BoundGenerator<'a> {
    pub fn bind(
        g: &'a Settings,
        maps: &'a [Map<'a>],
        frame: Option<ModelFrame>,
        dimensions: (u32, u32),
        anchor: Result<&'a dyn anchor::ValueSource, anchor::Issue>,
    ) -> Result<Self, Error> {
        g.validate()?;
        let (width, height) = dimensions;
        if width == 0 || height == 0 {
            return Err(Error::Invalid("ジェネレーターの大きさは正数が必要です"));
        }
        let mut table = [None; 10];
        for map in maps {
            map.validate()?;
            if table[map.kind as usize].replace(map).is_some() {
                return Err(Error::Invalid("マップの種類が重複しています"));
            }
        }
        let mut inactive = None;
        // ノイズ・グランジはマップが使えなくても UV に落として評価する（入力のまま通さない。理由は `fallback`）
        let plan = g
            .kind
            .is_procedural()
            .then(|| procedural::Plan::new(g, &table, (width, height)))
            .transpose()?;
        let wanted = if plan.is_some() {
            vec![]
        } else {
            g.used_maps()
        };
        for kind in wanted {
            let why = match table[kind as usize] {
                None => Some(Inactive::MissingMap(kind)),
                Some(m) => {
                    if m.state == MapState::Stale {
                        Some(Inactive::StaleMap(kind))
                    } else if m.state == MapState::Unverified {
                        Some(Inactive::UnverifiedMap(kind))
                    } else if (m.width, m.height) != (width, height) {
                        Some(Inactive::MapSize(kind))
                    } else if g.pins.get(&kind).is_some_and(|p| p != m.condition_key) {
                        Some(Inactive::PinMismatch(kind))
                    } else {
                        None
                    }
                }
            };
            if why.is_some() {
                inactive = why;
                break;
            }
        }
        if inactive.is_none() {
            inactive = match g.kind {
                Kind::ShapeGradient if frame.is_none() => Some(Inactive::MissingFrame),
                Kind::IdColor if g.id_colors.is_empty() => Some(Inactive::NoIdColors),
                Kind::Anchor => anchor.as_ref().err().map(|e| Inactive::Anchor(e.clone())),
                _ => None,
            };
        }
        if g.kind == Kind::Anchor {
            if let Ok(a) = anchor {
                if a.dimensions() != (width, height) {
                    return Err(Error::Invalid(
                        "Anchor とジェネレーターの画像サイズが違います",
                    ));
                }
            }
        }
        let [x, y, z] = g.direction;
        let len = (x * x + y * y + z * z).sqrt();
        let mut b = Self {
            g,
            width,
            height,
            maps: table,
            anchor: anchor.ok(),
            inactive,
            direction: [x / len, y / len, z / len],
            noise_scale: [0.; 3],
            seeds: noise::seeds(g.noise_seed),
            matrix: [0.; 9],
            offset: [0.; 3],
            plan,
        };
        if b.inactive.is_some() {
            return Ok(b);
        }
        if g.noise_amount > 0. && g.noise_space == NoiseSpace::Model {
            let p = b.map(MapKind::Position);
            let e = std::array::from_fn::<_, 3, _>(|i| p.bounds_max[i] - p.bounds_min[i]);
            let diag = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
            if diag <= 0. {
                b.inactive = Some(Inactive::EmptyBounds);
                return Ok(b);
            }
            if !diag.is_finite() {
                return Err(Error::Invalid("Position の境界箱が大きすぎます"));
            }
            b.noise_scale = e.map(|e| e / diag / g.noise_scale / 65535.);
        }
        if g.kind == Kind::ShapeGradient {
            let p = b.map(MapKind::Position);
            let frame = frame.unwrap();
            let r0 = frame.rotation_matrix();
            let rs = g.volume.rotation_matrix();
            let mut a = [0.; 9];
            for i in 0..3 {
                for j in 0..3 {
                    a[i * 3 + j] =
                        rs[i] * r0[j * 3] + rs[3 + i] * r0[j * 3 + 1] + rs[6 + i] * r0[j * 3 + 2];
                }
            }
            let t = frame.position();
            let root: [f64; 3] = std::array::from_fn(|k| {
                r0[k] * t[0] + r0[3 + k] * t[1] + r0[6 + k] * t[2] + g.volume.center[k]
            });
            for i in 0..3 {
                let mut offset = 0.;
                for j in 0..3 {
                    b.matrix[i * 3 + j] =
                        a[i * 3 + j] * ((p.bounds_max[j] - p.bounds_min[j]) / 65535.);
                    offset += a[i * 3 + j] * p.bounds_min[j];
                }
                b.offset[i] =
                    offset - (rs[i] * root[0] + rs[3 + i] * root[1] + rs[6 + i] * root[2]);
            }
            if b.matrix
                .iter()
                .chain(b.offset.iter())
                .any(|v| !v.is_finite())
            {
                return Err(Error::Invalid("形の座標変換が有限値を超えます"));
            }
        }
        Ok(b)
    }
    fn map(&self, k: MapKind) -> &'a Map<'a> {
        self.maps[k as usize].unwrap()
    }
    pub fn inactive(&self) -> Option<&Inactive> {
        self.inactive.as_ref()
    }
    /// ノイズ・グランジが、位置のマップが使えなくて UV 空間に落としている理由。マップが使えている・ほかの種類なら None。
    /// 入力のまま通す `inactive` とは別で、値は出ている（UV では周期を巻き、位置の継ぎ目の無さと回転は効かない）。
    pub fn fallback(&self) -> Option<&Inactive> {
        self.plan.as_ref().and_then(|p| p.fallback.as_ref())
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn scalar_at(&self, k: MapKind, i: usize) -> Option<f64> {
        let m = self.map(k);
        if m.coverage[i] == 0 {
            None
        } else {
            Some(m.data[i] as f64 / 65535.)
        }
    }
    fn vector_at(&self, k: MapKind, i: usize) -> Option<[f64; 3]> {
        let m = self.map(k);
        if m.coverage[i] == 0 {
            None
        } else {
            Some([
                m.data[i * 3] as f64,
                m.data[i * 3 + 1] as f64,
                m.data[i * 3 + 2] as f64,
            ])
        }
    }
    /// 1 画素の値。行の評価（`value_row`）はこの式と同じ値を出す（試験で全画素を比べる）。
    pub fn value(&self, x: u32, y: u32) -> Option<f64> {
        if x >= self.width || y >= self.height || self.inactive.is_some() {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        let g = self.g;
        let scalar = |k| self.scalar_at(k, i);
        let vector = |k| self.vector_at(k, i);
        let base = match g.kind {
            Kind::EdgeWear => clamp01(2. * (scalar(MapKind::Curvature)? - 0.5)),
            Kind::Dirt => {
                let mut b = 0.;
                if g.balance < 1. {
                    b += (1. - g.balance) * (1. - scalar(MapKind::AmbientOcclusion)?);
                }
                if g.balance > 0. {
                    b += g.balance * clamp01(2. * (0.5 - scalar(MapKind::Curvature)?));
                }
                b
            }
            Kind::PositionGradient => vector(MapKind::Position)?[g.axis] / 65535.,
            Kind::Thickness => scalar(MapKind::Thickness)?,
            Kind::Direction => {
                let [nx, ny, nz] = vector(if g.use_bent_normal {
                    MapKind::BentNormal
                } else {
                    MapKind::WorldNormal
                })?
                .map(|v| v / 65535. * 2. - 1.);
                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                let [dx, dy, dz] = self.direction;
                let dot = if len > 1e-9 {
                    (nx * dx + ny * dy + nz * dz) / len
                } else {
                    0.
                };
                clamp01((dot + 1.) * 0.5)
            }
            Kind::IdColor => {
                let rgb = vector(MapKind::Id)?.map(|v| ((v as u32 + 128) / 257) as i32);
                if g.id_colors.iter().any(|c| {
                    [
                        ((*c >> 16) & 255) as i32,
                        ((*c >> 8) & 255) as i32,
                        (*c & 255) as i32,
                    ]
                    .iter()
                    .zip(rgb)
                    .all(|(c, v)| (c - v).abs() <= g.id_tolerance as i32)
                }) {
                    1.
                } else {
                    0.
                }
            }
            Kind::Anchor => self.anchor?.value(x, y)?,
            Kind::Noise | Kind::Grunge => self.procedural_value(x, y, i)?,
            Kind::ShapeGradient => {
                let [x, y, z] = vector(MapKind::Position)?;
                let m = self.matrix;
                let k = self.offset;
                g.volume.local_value([
                    m[0] * x + m[1] * y + m[2] * z + k[0],
                    m[3] * x + m[4] * y + m[5] * z + k[1],
                    m[6] * x + m[7] * y + m[8] * z + k[2],
                ])
            }
        };
        let mut t = clamp01((base - g.low) / (g.high - g.low));
        if g.softness > 0. {
            t += g.softness * (t * t * (3. - 2. * t) - t);
        }
        if g.invert {
            t = 1. - t;
        }
        if g.noise_amount > 0. {
            let p = if g.noise_space == NoiseSpace::Uv {
                [
                    (x as f64 + 0.5) * (1. / self.width as f64 / g.noise_scale),
                    (y as f64 + 0.5) * (1. / self.height as f64 / g.noise_scale),
                    0.,
                ]
            } else {
                let p = vector(MapKind::Position)?;
                std::array::from_fn(|i| p[i] * self.noise_scale[i])
            };
            let m = clamp01((noise::fractal_pixel(p, self.seeds) - 0.3) / 0.4);
            t *= 1. - g.noise_amount * (m * m * (3. - 2. * m));
        }
        Some(t)
    }
    /// ノイズ・グランジの 1 画素の基底の値（レベル・反転の前）。`i` は画素の添字。格子の覚えはスレッドごとに持ち、同じ計画の呼びどうしで
    /// 使い回す（画素を隣へ進める呼びでは同じ格子の中の hash を引き直さない。計画が変われば捨てる）。
    fn procedural_value(&self, x: u32, y: u32, i: usize) -> Option<f64> {
        use std::cell::RefCell;
        thread_local! {
            static SCRATCH: RefCell<Option<procedural::PlanScratch>> = const { RefCell::new(None) };
        }
        let plan = self.plan.as_ref().expect("束縛済み");
        SCRATCH.with(|cell| {
            let mut slot = cell.borrow_mut();
            let scratch = match &mut *slot {
                Some(scratch) if scratch.fits(plan) => scratch,
                other => other.insert(plan.scratch()),
            };
            self.procedural_value_with(x, y, i, scratch)
        })
    }
    fn procedural_value_with(
        &self,
        x: u32,
        y: u32,
        i: usize,
        scratch: &mut procedural::PlanScratch,
    ) -> Option<f64> {
        let plan = self.plan.as_ref().expect("束縛済み");
        let position = if plan.needs_position() {
            Some(self.vector_at(MapKind::Position, i)?)
        } else {
            None
        };
        let normal = if plan.mode == procedural::Mode::Triplanar {
            Some(self.vector_at(MapKind::WorldNormal, i)?)
        } else {
            None
        };
        plan.value(x, y, (self.width, self.height), position, normal, scratch)
    }
    pub fn sample(&self, x: u32, y: u32, scalar: bool) -> Option<Generated> {
        let value = self.value(x, y)?;
        Some(match &self.g.ramp {
            None => Generated::Scalar(value),
            Some(r) => Generated::Mapped(r.evaluate_unchecked(value, scalar).to_array()),
        })
    }
}
pub(super) fn allocate(
    region: Rect,
    dimensions: (u32, u32),
    options: &Options<'_>,
) -> Result<Vec<u8>, Error> {
    options.check()?;
    if region.is_empty()
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|v| v > dimensions.0)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|v| v > dimensions.1)
    {
        return Err(Error::Invalid("評価領域が画像の範囲外です"));
    }
    let needed = (u64::from(region.width) * u64::from(region.height))
        .checked_mul(4)
        .ok_or(Error::Invalid("評価画像が大きすぎます"))?;
    if needed > options.budget_bytes {
        return Err(Error::Budget {
            needed,
            budget: options.budget_bytes,
        });
    }
    zeroed(usize::try_from(needed).map_err(|_| Error::Allocation)?)
}
/// 0 で埋めた `size` バイト。確保できなければ `Error::Allocation`。大きい確保は OS から 0 のページをそのまま受け取る（`alloc_zeroed`）ので、
/// 確保のスレッドで 0 を書き込む列を通らず、ページへの最初の書き込み（行の評価）がそれぞれのスレッドで進む。
/// `vec![0; n]` は確保に失敗すると中断して `Error::Allocation` を返せず、失敗を返す 0 埋めの確保は安定版の標準ライブラリに無いので、`unsafe` を使う。
fn zeroed(size: usize) -> Result<Vec<u8>, Error> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let layout = std::alloc::Layout::array::<u8>(size).map_err(|_| Error::Allocation)?;
    // SAFETY: layout の大きさは 0 でない。確保できなければ null が返る
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return Err(Error::Allocation);
    }
    // SAFETY: ptr は大域の割り当て器から、`Vec<u8>` と同じ配置（大きさ `size`・整列 1）で確保した、0 で初期化済みのメモリ。
    // 長さ・容量はその確保の大きさに一致する
    Ok(unsafe { Vec::from_raw_parts(ptr, size, size) })
}
pub fn evaluate(
    source: &dyn Source,
    g: &BoundGenerator<'_>,
    region: Rect,
    target: Target,
    strength: f64,
    options: &Options<'_>,
) -> Result<Output, Error> {
    if !unit(strength) || source.dimensions() != g.dimensions() {
        return Err(Error::Invalid(
            "ジェネレーターの強さまたは画像の大きさが不正です",
        ));
    }
    let mut pixels = allocate(region, source.dimensions(), options)?;
    let width = region.width as usize;
    pixels
        .par_chunks_mut(width * 4)
        .enumerate()
        .try_for_each_init(
            || rows::Scratch::new(g, width),
            |scratch, (row, bytes)| -> Result<(), Error> {
                options.check()?;
                let y = region.y + row as u32;
                source.read_row(region.x, y, bytes);
                if target == Target::Mask {
                    for p in bytes.chunks_exact_mut(4) {
                        p[..3].fill(0);
                    }
                }
                // 強さ 0・マップが使えない段は入力のまま（値を作らない）
                if strength != 0. && g.inactive.is_none() {
                    g.value_row(region.x, y, &mut scratch.values, &mut scratch.aux);
                    g.apply_row(bytes, &scratch.values, target, strength, scratch.aux.level);
                }
                Ok(())
            },
        )?;
    options.check()?;
    Ok(Output {
        pixels,
        inactive: g.inactive.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroed_gives_zeros_and_refuses_what_cannot_be_allocated() {
        assert!(zeroed(0).unwrap().is_empty());
        for size in [1, 7, 4096, 5_000_003, 40 << 20] {
            let v = zeroed(size).unwrap();
            assert_eq!((v.len(), v.capacity()), (size, size));
            assert!(v.iter().all(|b| *b == 0), "{size}");
        }
        // 書き込めて、解放しても壊れない
        let mut v = zeroed(1 << 20).unwrap();
        v.iter_mut().enumerate().for_each(|(i, b)| *b = i as u8);
        assert_eq!(v[255], 255);
        drop(v);
        assert_eq!(zeroed(usize::MAX), Err(Error::Allocation));
        assert_eq!(zeroed(isize::MAX as usize), Err(Error::Allocation));
    }
}
