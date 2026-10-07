mod rows;
use super::*;
use crate::{
    math::{clamp01, to_byte, UNIT},
    Rect,
};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    /// 画像の段が読む、投影を束縛済みの画像（[`Self::with_image`]。ほかの種類と、まだ渡していない画像の段は None）。
    image: Option<&'a crate::fill_image::FillSampler<'a>>,
    /// アイランドごとのばらつきが読む島の図（[`Self::with_islands`]。ほかの種類と、まだ渡していない段は None）。
    islands: Option<Arc<crate::geometry::IslandMap>>,
    /// アイランドごとのばらつきの乱数の種（[`IslandVariation::stream`]。ほかの種類は 0）。
    island_stream: u32,
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
        // 画像の段は、画像を選んでいないことを先に言う（マップより直しやすい）
        let mut inactive =
            (g.kind == Kind::Image && g.image.image == 0).then_some(Inactive::NoImage);
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
            if inactive.is_some() {
                break;
            }
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
                Kind::Image
                    if frame.is_none()
                        && g.image.projection.mode != crate::fill_image::ProjectionMode::Uv =>
                {
                    Some(Inactive::MissingFrame)
                }
                // 画像（`with_image`）を渡すまでは使えない
                Kind::Image => Some(Inactive::MissingImage),
                // 島の図（`with_islands`）を渡すまでは、モデルが無いのと同じ
                Kind::UvIslandVariation => Some(Inactive::NoModel),
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
            image: None,
            islands: None,
            island_stream: if g.kind == Kind::UvIslandVariation {
                g.island.stream()
            } else {
                0
            },
        };
        if b.inactive.is_some() {
            return Ok(b);
        }
        // 光は向きの欄の代わりに、光の来る向きを持つ
        if g.kind == Kind::Light {
            b.direction = g.light.direction();
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
    /// 画像の段が読む画像を渡す（`FillSampler::bind` で、この段の投影・文書の大きさ・位置と向きのマップ・モデルのルートを束縛したもの）。
    /// 色のチャンネルに使う段は色として読むミップマップ（リニアの画像は sRGB に直す）、マスク・スカラーは値のままのミップマップのサンプラーを渡す。
    /// 画像の段で、マップ・モデルのルートが揃っていて画像だけを待っていたときにだけ効く（ほかは何もしない）。サンプラー自身が
    /// 使えなければ（位置・法線のマップが無いなど）その理由で入力のまま通す。
    pub fn with_image(mut self, sampler: &'a crate::fill_image::FillSampler<'a>) -> Self {
        use crate::fill_image::InactiveReason as R;
        if self.g.kind != Kind::Image || self.inactive != Some(Inactive::MissingImage) {
            return self;
        }
        self.inactive = match sampler.reason() {
            None => {
                self.image = Some(sampler);
                None
            }
            Some(R::MissingImage) => Some(Inactive::MissingImage),
            Some(R::MissingPosition) => Some(Inactive::MissingMap(MapKind::Position)),
            Some(R::MissingNormal) => Some(Inactive::MissingMap(MapKind::WorldNormal)),
            Some(R::PositionSize) => Some(Inactive::MapSize(MapKind::Position)),
            Some(R::NormalSize) => Some(Inactive::MapSize(MapKind::WorldNormal)),
            Some(R::StalePosition) => Some(Inactive::StaleMap(MapKind::Position)),
            Some(R::StaleNormal) => Some(Inactive::StaleMap(MapKind::WorldNormal)),
            Some(R::UnknownModelFrame) => Some(Inactive::MissingFrame),
        };
        self
    }
    /// アイランドごとのばらつきが読む島の図を渡す（モデルのこのテクスチャセットの `UvTopology::island_map_within` の、ジェネレーターと
    /// 同じ大きさの図）。島の図を待っている アイランドごとのばらつきの段にだけ効く（ほかは何もしない）。大きさの違う図は使わず、島の図を
    /// 作れなかったことにする（入力のまま通す）。
    pub fn with_islands(mut self, islands: Arc<crate::geometry::IslandMap>) -> Self {
        if self.g.kind != Kind::UvIslandVariation || self.inactive != Some(Inactive::NoModel) {
            return self;
        }
        if (islands.width(), islands.height()) != (self.width, self.height) {
            self.inactive = Some(Inactive::IslandMap);
            return self;
        }
        self.islands = Some(islands);
        self.inactive = None;
        self
    }
    /// アイランドごとのばらつきの島の図を作れなかった（作業メモリの予算で断られた）ことを渡す。島の図を待っている段にだけ効く。
    pub fn islands_refused(mut self) -> Self {
        if self.g.kind == Kind::UvIslandVariation && self.inactive == Some(Inactive::NoModel) {
            self.inactive = Some(Inactive::IslandMap);
        }
        self
    }
    /// アイランドごとのばらつきの 1 画素の基底の値（島の外は None）。
    fn island_value(&self, x: u32, y: u32) -> Option<f64> {
        let island = self.islands.as_ref()?.island(x, y);
        (island != 0).then(|| self.g.island.value_in(self.island_stream, island))
    }
    /// 画像の段の 1 画素の画像の色（straight RGBA8、反転は RGB だけ）。値の無い画素（投影の外・位置の無い画素）・範囲外・使えない段は None。
    fn image_color(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height || self.inactive.is_some() {
            return None;
        }
        let c = self.image?.projected(x, y)?;
        Some(if self.g.invert {
            [255 - c.r, 255 - c.g, 255 - c.b, c.a]
        } else {
            c.to_array()
        })
    }
    /// 画像の段の 1 画素の基底の値（選んだ成分。レベル・反転の前）。
    fn image_value(&self, x: u32, y: u32) -> Option<f64> {
        let c = self.image?.projected(x, y)?;
        let u = |b: u8| UNIT[b as usize];
        Some(match self.g.image.component {
            ImageComponent::Red => u(c.r),
            ImageComponent::Green => u(c.g),
            ImageComponent::Blue => u(c.b),
            ImageComponent::Alpha => u(c.a),
            ImageComponent::Luminance => {
                clamp01(0.2126 * u(c.r) + 0.7152 * u(c.g) + 0.0722 * u(c.b))
            }
        })
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
            Kind::Image => self.image_value(x, y)?,
            Kind::Pattern | Kind::Light | Kind::MaskBuilder => self.base_050(x, y, i)?,
            Kind::UvIslandVariation => self.island_value(x, y)?,
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
    /// 模様・ライト・マスクの組み立ての 1 画素の基底の値（レベル・反転の前）。`i` は画素の添字。行の評価もこの式を 1 画素ずつ呼ぶ。
    fn base_050(&self, x: u32, y: u32, i: usize) -> Option<f64> {
        let g = self.g;
        match g.kind {
            Kind::Pattern => Some(g.pattern.value(
                (f64::from(x) + 0.5) / f64::from(self.width),
                (f64::from(y) + 0.5) / f64::from(self.height),
            )),
            Kind::Light => g
                .light
                .value(self.vector_at(MapKind::WorldNormal, i)?, self.direction),
            _ => g.mask_builder.value(|k| match k {
                // 位置は高さ（Y。境界箱の中の 0〜1）
                MapKind::Position => Some(self.vector_at(MapKind::Position, i)?[1] / 65535.),
                other => self.scalar_at(other, i),
            }),
        }
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
    /// 1 画素の出力。画像の段は、色の対象（`scalar` が偽）では画素の色（`Mapped`）、マスク・スカラーでは選んだ成分の値。
    pub fn sample(&self, x: u32, y: u32, scalar: bool) -> Option<Generated> {
        if self.g.kind == Kind::Image && !scalar {
            return self.image_color(x, y).map(Generated::Mapped);
        }
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
                    if g.g.kind == Kind::Image && target == Target::Color {
                        g.apply_image_row(bytes, region.x, y, strength);
                    } else {
                        g.value_row(region.x, y, &mut scratch.values, &mut scratch.aux);
                        g.apply_row(bytes, &scratch.values, target, strength, scratch.aux.level);
                    }
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
