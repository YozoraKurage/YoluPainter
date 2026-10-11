//! 2D の対称と 3D の対称を、もう片方のビューのストロークにも当てる（UV の平面と、モデルの空間の写しの橋渡し）。
//!
//! 写す順は、どちらのビューでも同じ: 3D の写し（モデルの空間のミラー・放射状）を先に、2D の写し（UV の平面の縦・横・両方・放射状）を
//! 後に当てる（写しの数は 2 つの数の掛け算。2D の写しは、元と 3D の写しの全部を写す）。
//!
//! - 3D のストロークに 2D の対称（[`copy_by_canvas`]）: 面のダブ（元と 3D の写しを画素ごとの大きい方で合わせたもの）の UV の画素を、
//!   2D の対称の変換で写す。写しの画素の中心を元へ戻した点で、元の画素の覆いを双線形に読む（元の画素の中心へ戻る写し、つまり軸が
//!   画素の中心か境にある縦・横・両方と四分の一周の放射状は、覆いをそのまま写す）。写しの画素は三角形を持たない（ぼかしの縁の
//!   読み元は UV の画像の上の 4 点、ステンシルは元の画素の点で読む）。写しが重なる画素は大きい方の覆い。
//! - 2D のストロークに 3D の対称（[`ModelSymmetry`]）: ダブの中心の UV の下の面の点（中心がどのアイランドにも無ければ、ダブの半径の内で
//!   いちばん近い三角形の点）を 3D の対称で写し、3D のストロークと同じ探し方（[`find_copy`]: 同じテクスチャセットの、向きの合う面の
//!   いちばん近い点）で写しの面の点を決める。元の三角形の UV → 面 → 写す → 写しの三角形の面 → その UV の 1 次の写像（三角形の組ごとの
//!   2×2 の係数、[`UvCopy`]）で、元のダブの形を写しの UV へ置く（写しの側のテクセルの細かさと、UV の向き（鏡に写した UV の並べ方など）も
//!   写す）。写しの面が見つからない・別のテクスチャセット・UV が潰れた三角形のときは、その写しだけ飛ばして理由を返す。2D には
//!   カメラが無いので、見えない面にも塗る（3D の「見えない面にも」によらない）。
//!
//! 保証の射程: 1 つのダブは、中心の三角形の組の写像で丸ごと写す（ダブが三角形やアイランドをまたいでも、写しの側で折り返さない）。
//! 写しのダブは、元のダブと同じく UV の平面の上の形で、写しの側の面の上で 3D の写しの形と画素まで同じにはならない。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glam::{DMat3, DVec2, DVec3, Vec2, Vec3};

use super::build::FastMap;
use super::dab::{DabRefusal, SurfaceDabResult, SurfacePixel};
use super::paint::{world_radius, SurfaceSymmetrySetup};
use super::symmetry::{find_copy, CopyHit, MirrorOutcome, MirrorPlane, RadialSymmetry};
use super::uv_grid::UvGrid;
use super::{SurfaceHit, SurfaceTriangle};
use crate::error::CoreError;
use crate::symmetry::SymmetryTransform;

/// 1 つのダブ（3D の面のダブの元と写しの全部）を 2D の対称で写すときに調べる、写しの画素の候補の上限（2D の対称の 1 ダブの上限
/// と同じ 4 M）。超えたらダブを断る（ストロークを取り消す）。
pub const MAX_CANVAS_COPY_PIXELS: i64 = 4 << 20;

/// 写しの写像の伸び縮みの上限（1 軸あたり。写しの側のテクセルが元の 64 倍より細かい・粗いときは、UV が潰れた三角形として飛ばす）。
const MAX_COPY_SCALE: f64 = 64.0;

/// 文書の画素の点の 1 次の写像: p を `target + m (p − origin)` へ写す（m は 2×2、行の順: m00 m01 / m10 m11）。逆も持つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvCopy {
    pub origin: DVec2,
    pub target: DVec2,
    pub m: [f64; 4],
    inv: [f64; 4],
}

impl UvCopy {
    /// 係数の逆が作れない（行列式が 0・有限でない）なら None。
    pub fn new(origin: DVec2, target: DVec2, m: [f64; 4]) -> Option<UvCopy> {
        let det = m[0] * m[3] - m[1] * m[2];
        if !(det.is_finite() && det != 0.0 && origin.is_finite() && target.is_finite()) {
            return None;
        }
        let inv = [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det];
        inv.iter().all(|v| v.is_finite()).then_some(UvCopy {
            origin,
            target,
            m,
            inv,
        })
    }

    /// 点を写す。
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.origin.x, y - self.origin.y);
        (
            self.target.x + self.m[0] * dx + self.m[1] * dy,
            self.target.y + self.m[2] * dx + self.m[3] * dy,
        )
    }

    /// 写した点を元へ戻す。
    #[inline]
    pub fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.target.x, y - self.target.y);
        (
            self.origin.x + self.inv[0] * dx + self.inv[1] * dy,
            self.origin.y + self.inv[2] * dx + self.inv[3] * dy,
        )
    }

    /// 元の中心のまわりの半径 r の円を写した形の、外接の箱の半分の幅と高さ。
    pub fn reach(&self, r: f64) -> (f64, f64) {
        (
            r * (self.m[0] * self.m[0] + self.m[1] * self.m[1]).sqrt(),
            r * (self.m[2] * self.m[2] + self.m[3] * self.m[3]).sqrt(),
        )
    }

    /// この写しの後に 2D の対称の変換 t を当てたもの（t ∘ self）。逆は、t の逆（転置）をこの写しの逆の前に当てる。
    pub fn then(&self, t: &SymmetryTransform) -> UvCopy {
        let (c, r) = t.parts();
        let (tx, ty) = (self.target.x - c.x, self.target.y - c.y);
        let target = DVec2::new(c.x + r[0] * tx + r[1] * ty, c.y + r[2] * tx + r[3] * ty);
        let m = mul(r, self.m);
        // (r m)⁻¹ = m⁻¹ rᵀ
        let inv = mul(self.inv, [r[0], r[2], r[1], r[3]]);
        UvCopy {
            origin: self.origin,
            target,
            m,
            inv,
        }
    }
}

fn mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
    ]
}

/// 1 つのダブの 3D の写しの写像（元を除く）と、写せなかった写しの最後の理由（面が無い・別のテクスチャセット。全部写せたら None）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelCopies {
    pub copies: Vec<UvCopy>,
    pub outcome: Option<MirrorOutcome>,
}

/// 2D のストロークに当てる 3D の対称（ストロークの始めに固める。ストロークの間はモデルも設定も動かない）。
pub struct ModelSymmetry {
    grid: Arc<UvGrid>,
    mirror: Option<MirrorPlane>,
    radial: Option<RadialSymmetry>,
    /// 求めたダブの写しの覚え（複数チャンネルのストロークは、同じダブをチャンネルごとに塗るので、2 つ目からはここを読む）。
    memo: Mutex<CopyMemo>,
    /// 写しを求めた数（覚えから読んだものは数えない。試験用）。
    computed: AtomicU64,
}

/// ダブの中心・半径・文書の大きさ（ビットのまま）。
type CopyKey = (u64, u64, u64, u32, u32);

/// 覚えるダブの数（超えたら古いものから捨てる。捨てても同じ値を求め直すだけ）。
const COPY_MEMO: usize = 4096;

#[derive(Default)]
struct CopyMemo {
    map: HashMap<CopyKey, ModelCopies>,
    order: VecDeque<CopyKey>,
}

impl std::fmt::Debug for ModelSymmetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSymmetry")
            .field("material", &self.grid.material())
            .field("mirror", &self.mirror)
            .field("radial", &self.radial)
            .finish()
    }
}

impl PartialEq for ModelSymmetry {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.grid, &other.grid)
            && self.mirror == other.mirror
            && self.radial == other.radial
    }
}

impl ModelSymmetry {
    /// grid の今のモデルとテクスチャセット（マテリアルの組）に、setup のミラー・放射状を当てる。写しが無い設定なら None。
    pub fn new(grid: Arc<UvGrid>, setup: &SurfaceSymmetrySetup) -> Option<ModelSymmetry> {
        setup.enabled().then(|| ModelSymmetry {
            grid,
            mirror: setup.mirror,
            radial: setup.radial,
            memo: Mutex::new(CopyMemo::default()),
            computed: AtomicU64::new(0),
        })
    }

    /// 写しを求めた数（同じダブを覚えから読んだものは数えない。試験用）。
    pub fn computed(&self) -> u64 {
        self.computed.load(Ordering::Relaxed)
    }

    /// 文書の画素の点 (x, y) を中心とする半径 radius（文書の画素）のダブの、3D の写しの写像（width・height は文書の大きさ）。
    /// 中心の UV が重なった面（同じ UV を持つ別の面）の上にあれば、その全部の面の点から写す（3D ビューでどちらの面を塗っても写しが
    /// 出るのと同じ）。同じ写し先の三角形へ同じ 2×2 で写す写しと、元の UV に重なる写しは 1 つにまとめる。中心がテクスチャセットの
    /// どの三角形にも、半径の内にも無いダブは、写しを作らない（理由も返さない。面の無い所の画素は写せない）。写しの点を面へ
    /// 投げ直す探索が上限を超えたら `WorkingBudgetExceeded`（ストロークを取り消す）。同じダブ（中心・半径・大きさが同じ）の
    /// 2 度目からは、覚えた結果を返す。
    pub fn copies(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        width: u32,
        height: u32,
    ) -> Result<ModelCopies, CoreError> {
        let key = (x.to_bits(), y.to_bits(), radius.to_bits(), width, height);
        if let Some(found) = self.memo.lock().ok().and_then(|m| m.map.get(&key).cloned()) {
            return Ok(found);
        }
        let out = self.compute(x, y, radius, width, height)?;
        self.computed.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut m) = self.memo.lock() {
            if m.order.len() >= COPY_MEMO {
                if let Some(old) = m.order.pop_front() {
                    m.map.remove(&old);
                }
            }
            m.order.push_back(key);
            m.map.insert(key, out.clone());
        }
        Ok(out)
    }

    fn compute(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        width: u32,
        height: u32,
    ) -> Result<ModelCopies, CoreError> {
        let mut out = ModelCopies::default();
        let size = DVec2::new(width as f64, height as f64);
        let anchors = self.grid.locate_all(DVec2::new(x, y), size, radius);
        if anchors.is_empty() {
            return Ok(out);
        }
        let geometry = self.grid.geometry();
        let hits: Vec<SurfaceHit> = anchors
            .iter()
            .map(|&(t, w)| {
                surface_hit(geometry.revision(), t, &geometry.triangles()[t as usize], w)
            })
            .collect();
        let world = world_radius(geometry, radius, width);
        // 元の面の点の全部（重なった UV の面どうしは、互いの写しとして作らない）
        let mut positions: Vec<Vec3> = hits.iter().map(|h| h.position).collect();
        // 作った写しの写し先の三角形と写像（同じものは 1 つに）。元の UV に重なる写しは作らない
        let mut made: Vec<(u32, UvCopy)> = Vec::new();
        let mut outcome = MirrorOutcome::OnPlane;
        let count = self.radial.map_or(1, |r| r.count);
        for hit in &hits {
            let triangle = &geometry.triangles()[hit.triangle as usize];
            let anchor = DVec2::new(hit.uv.x as f64 * size.x, hit.uv.y as f64 * size.y);
            for copy in 0..count {
                for reflected in [false, true] {
                    if (copy == 0 && !reflected) || (reflected && self.mirror.is_none()) {
                        continue;
                    }
                    let found = match find_copy(
                        geometry,
                        hit,
                        self.mirror.as_ref(),
                        self.radial.as_ref(),
                        copy,
                        reflected,
                        world,
                        &mut positions,
                    ) {
                        CopyHit::Duplicate => continue,
                        CopyHit::BudgetExceeded => return Err(CoreError::WorkingBudgetExceeded),
                        CopyHit::NoSurface => {
                            outcome = MirrorOutcome::NoSurface;
                            continue;
                        }
                        CopyHit::OtherMaterial => {
                            outcome = MirrorOutcome::OtherSlot;
                            continue;
                        }
                        CopyHit::Found(h) => h,
                    };
                    let linear = self.linear(copy, reflected);
                    let to = &geometry.triangles()[found.triangle as usize];
                    let target = DVec2::new(found.uv.x as f64 * size.x, found.uv.y as f64 * size.y);
                    match jacobian(triangle, to, linear, size)
                        .and_then(|m| UvCopy::new(anchor, target, m))
                    {
                        Some(c) => {
                            if outcome == MirrorOutcome::OnPlane {
                                outcome = MirrorOutcome::Painted;
                            }
                            let same = |a: &UvCopy| {
                                (a.target - c.target).length() < 1e-6
                                    && (a.origin - c.origin).length() < 1e-6
                                    && a.m.iter().zip(&c.m).all(|(p, q)| (p - q).abs() < 1e-9)
                            };
                            let identity = (c.target - c.origin).length() < 1e-6
                                && [1.0, 0.0, 0.0, 1.0]
                                    .iter()
                                    .zip(&c.m)
                                    .all(|(p, q)| (p - q).abs() < 1e-9);
                            if identity || made.iter().any(|(t, a)| *t == found.triangle && same(a))
                            {
                                continue;
                            }
                            made.push((found.triangle, c));
                        }
                        // UV が潰れた三角形・ありえない伸び縮み: その写しだけ飛ばす
                        None => outcome = MirrorOutcome::UvMismatch,
                    }
                }
            }
        }
        out.copies = made.into_iter().map(|(_, c)| c).collect();
        if matches!(
            outcome,
            MirrorOutcome::NoSurface | MirrorOutcome::OtherSlot | MirrorOutcome::UvMismatch
        ) {
            out.outcome = Some(outcome);
        }
        Ok(out)
    }

    /// copy 番目の回転（reflected ならミラーの後）の、モデルの空間の 1 次の部分（鏡映を先に、回転を後に）。
    fn linear(&self, copy: u32, reflected: bool) -> DMat3 {
        let mirror = match (reflected, &self.mirror) {
            (true, Some(m)) => {
                let n = m.normal.as_dvec3();
                DMat3::IDENTITY
                    - DMat3::from_cols(n * (2.0 * n.x), n * (2.0 * n.y), n * (2.0 * n.z))
            }
            _ => DMat3::IDENTITY,
        };
        let rotation = match &self.radial {
            Some(r) if copy > 0 => DMat3::from_quat(r.rotation(copy).as_dquat()),
            _ => DMat3::IDENTITY,
        };
        rotation * mirror
    }
}

/// 三角形の重心座標 w の点の当たり（2D のダブの中心の面の点）。
fn surface_hit(revision: u32, index: u32, t: &SurfaceTriangle, w: Vec3) -> SurfaceHit {
    SurfaceHit {
        revision,
        renderer: t.renderer,
        material_slot: t.material_slot,
        material: t.material,
        triangle: index,
        position: t.a * w.x + t.b * w.y + t.c * w.z,
        normal: t.normal(),
        barycentric: w,
        uv: t.uv_a * w.x + t.uv_b * w.y + t.uv_c * w.z,
        distance: 0.0,
    }
}

/// 元の三角形の UV（文書の画素）→ 面 → linear → 写しの三角形の面（その平面へ直交に落とす）→ その UV（文書の画素）の 1 次の係数。
/// UV か面が潰れた三角形・伸び縮みが [`MAX_COPY_SCALE`] を超えるときは None。
fn jacobian(
    from: &SurfaceTriangle,
    to: &SurfaceTriangle,
    linear: DMat3,
    size: DVec2,
) -> Option<[f64; 4]> {
    let d3 = |v: Vec3| v.as_dvec3();
    let px = |v: Vec2| DVec2::new(v.x as f64 * size.x, v.y as f64 * size.y);
    // 元: 画素のずれ → 面のずれ（E · U⁻¹ の 2 つの列）
    let (e1, e2) = (d3(from.b - from.a), d3(from.c - from.a));
    let (u1, u2) = (px(from.uv_b) - px(from.uv_a), px(from.uv_c) - px(from.uv_a));
    let det = u1.x * u2.y - u2.x * u1.y;
    // 潰れた UV（NaN も）
    if det.is_nan() || det.abs() <= 1e-9 * u1.length() * u2.length() {
        return None;
    }
    let a0: DVec3 = (e1 * u2.y - e2 * u1.y) / det;
    let a1: DVec3 = (e2 * u1.x - e1 * u2.x) / det;
    let (b0, b1) = (linear * a0, linear * a1);
    // 写し: 面のずれ → 写しの三角形の平面の重み（最小二乗）→ 画素のずれ
    let (f1, f2) = (d3(to.b - to.a), d3(to.c - to.a));
    let (v1, v2) = (px(to.uv_b) - px(to.uv_a), px(to.uv_c) - px(to.uv_a));
    let (g11, g12, g22) = (f1.dot(f1), f1.dot(f2), f2.dot(f2));
    let gd = g11 * g22 - g12 * g12;
    if gd.is_nan() || gd <= 1e-12 * g11 * g22 {
        return None;
    }
    let to_px = |d: DVec3| -> DVec2 {
        let (r1, r2) = (f1.dot(d), f2.dot(d));
        let s = (g22 * r1 - g12 * r2) / gd;
        let t = (g11 * r2 - g12 * r1) / gd;
        v1 * s + v2 * t
    };
    let (c0, c1) = (to_px(b0), to_px(b1));
    let m = [c0.x, c1.x, c0.y, c1.y];
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    // 伸び縮み（特異値）の範囲: 列の長さと行列式で見る
    let (s0, s1) = (c0.length(), c1.length());
    let area = (m[0] * m[3] - m[1] * m[2]).abs();
    let limit = MAX_COPY_SCALE;
    if s0 > limit || s1 > limit || area < 1.0 / (limit * limit) || area > limit * limit {
        return None;
    }
    Some(m)
}

/// 64 × 64 の区画で持つ、画素ごとの値（区画の中は行の順）。
struct Tiles<T: Copy + Default> {
    tiles: FastMap<u32, Box<[T]>>,
}

const TILE: i64 = 64;

impl<T: Copy + Default> Tiles<T> {
    fn new() -> Self {
        Tiles {
            tiles: FastMap::default(),
        }
    }
    fn key(x: i64, y: i64) -> u32 {
        (((y / TILE) as u32) << 16) | (x / TILE) as u32
    }
    fn at(x: i64, y: i64) -> usize {
        ((y % TILE) * TILE + x % TILE) as usize
    }
    #[inline]
    fn get(&self, x: i64, y: i64) -> T {
        if x < 0 || y < 0 {
            return T::default();
        }
        self.tiles
            .get(&Self::key(x, y))
            .map_or_else(T::default, |t| t[Self::at(x, y)])
    }
    fn slot(&mut self, x: i64, y: i64) -> &mut T {
        let tile = self
            .tiles
            .entry(Self::key(x, y))
            .or_insert_with(|| vec![T::default(); (TILE * TILE) as usize].into_boxed_slice());
        &mut tile[Self::at(x, y)]
    }
}

/// 3D の面のダブ（元と 3D の写しを合わせたもの）を、2D の対称の変換（恒等を含んでよい）で UV の平面の上に写して合わせる（同じ画素は
/// 大きい方の覆い、並びは左下からの行の順）。候補の画素が [`MAX_CANVAS_COPY_PIXELS`] を超えたら `PixelBudget` で断る。
pub(crate) fn copy_by_canvas(
    result: SurfaceDabResult,
    transforms: &[SymmetryTransform],
    width: i32,
    height: i32,
) -> SurfaceDabResult {
    if result.pixels.is_empty() || result.refusal.is_some() {
        return result;
    }
    // 元の画素の番号（+1。0 は無し）
    let mut lookup: Tiles<u32> = Tiles::new();
    for (i, p) in result.pixels.iter().enumerate() {
        if p.x >= 0 && p.y >= 0 {
            *lookup.slot(p.x as i64, p.y as i64) = i as u32 + 1;
        }
    }
    let Some((mut out, work)) = canvas_copies(&result.pixels, &lookup, transforms, width, height)
    else {
        return result.reject(DabRefusal::PixelBudget);
    };
    let w = width as i64;
    // 並べ替えは安定なので、同じ画素は元の画素が先（覆いが同じなら元の画素の三角形を残す）
    out.sort_by_key(|p| p.y as i64 * w + p.x as i64);
    out.dedup_by(|later, kept| {
        if (later.x, later.y) != (kept.x, kept.y) {
            return false;
        }
        if later.coverage > kept.coverage {
            *kept = *later;
        }
        true
    });
    SurfaceDabResult {
        pixels: out,
        candidate_pixels: result
            .candidate_pixels
            .saturating_add(work.min(i32::MAX as i64) as i32),
        ..result
    }
}

/// 元の画素と、写しの画素の全部（並べ替え・重ね合わせの前）と、調べた候補の数。候補が上限を超えたら None。
fn canvas_copies(
    source: &[SurfacePixel],
    lookup: &Tiles<u32>,
    transforms: &[SymmetryTransform],
    width: i32,
    height: i32,
) -> Option<(Vec<SurfacePixel>, i64)> {
    let coverage_of = |i: u32| source[(i - 1) as usize].coverage as f64;
    let mut out: Vec<SurfacePixel> = source.to_vec();
    let mut work = 0i64;
    for t in transforms.iter().filter(|t| !t.is_identity()) {
        let (_, r) = t.parts();
        // 元の画素の双線形の支え（中心のまわりの 1 × 1 の箱）を写した形の外接の箱の半分
        let (hx, hy) = (r[0].abs() + r[1].abs(), r[2].abs() + r[3].abs());
        let mut seen: Tiles<bool> = Tiles::new();
        for p in source {
            let (qx, qy) = t.map(p.x as f64 + 0.5, p.y as f64 + 0.5);
            let x0 = ((qx - 0.5 - hx).floor() as i64 + 1).max(0);
            let x1 = ((qx - 0.5 + hx).ceil() as i64 - 1).min(width as i64 - 1);
            let y0 = ((qy - 0.5 - hy).floor() as i64 + 1).max(0);
            let y1 = ((qy - 0.5 + hy).ceil() as i64 - 1).min(height as i64 - 1);
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    let flag = seen.slot(tx, ty);
                    if *flag {
                        continue;
                    }
                    *flag = true;
                    work += 1;
                    if work > MAX_CANVAS_COPY_PIXELS {
                        return None;
                    }
                    let (sx, sy) = t.inverse(tx as f64 + 0.5, ty as f64 + 0.5);
                    if let Some((c, nearest)) = bilinear(lookup, &coverage_of, sx, sy) {
                        out.push(SurfacePixel {
                            x: tx as i32,
                            y: ty as i32,
                            coverage: c,
                            triangle: None,
                            position: source[(nearest - 1) as usize].position,
                        });
                    }
                }
            }
        }
    }
    Some((out, work))
}

/// 元の画素の覆いを、点 (sx, sy)（文書の画素の座標、画素の中心は n + 0.5）で双線形に読む。覆いが 0 なら None。重みのいちばん大きい
/// 元の画素の番号（+1）も返す。点が画素の中心から 1e-9 以内なら、その画素の値をそのまま読む。
fn bilinear(
    lookup: &Tiles<u32>,
    coverage_of: &impl Fn(u32) -> f64,
    sx: f64,
    sy: f64,
) -> Option<(f32, u32)> {
    let snap = |f: f64| -> (i64, f64) {
        let base = f.floor();
        let a = f - base;
        if a < 1e-9 {
            (base as i64, 0.0)
        } else if a > 1.0 - 1e-9 {
            (base as i64 + 1, 0.0)
        } else {
            (base as i64, a)
        }
    };
    let (x0, ax) = snap(sx - 0.5);
    let (y0, ay) = snap(sy - 0.5);
    let mut sum = 0.0f64;
    let mut best = (0.0f64, 0u32);
    for (dx, dy, w) in [
        (0, 0, (1.0 - ax) * (1.0 - ay)),
        (1, 0, ax * (1.0 - ay)),
        (0, 1, (1.0 - ax) * ay),
        (1, 1, ax * ay),
    ] {
        if w <= 0.0 {
            continue;
        }
        let i = lookup.get(x0 + dx, y0 + dy);
        if i == 0 {
            continue;
        }
        sum += w * coverage_of(i);
        if w > best.0 {
            best = (w, i);
        }
    }
    (sum > 0.0 && best.1 != 0).then_some((sum as f32, best.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasSymmetry, SymmetryMode};

    fn pixel(x: i32, y: i32, coverage: f32) -> SurfacePixel {
        SurfacePixel {
            x,
            y,
            coverage,
            triangle: Some(0),
            position: Vec3::new(x as f32, y as f32, 0.0),
        }
    }

    fn dab(pixels: Vec<SurfacePixel>) -> SurfaceDabResult {
        SurfaceDabResult {
            pixels,
            ..SurfaceDabResult::default()
        }
    }

    #[test]
    fn a_vertical_mirror_with_the_axis_on_a_pixel_border_copies_coverage_exactly() {
        let s = CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(8.0, 8.0), 2).unwrap();
        let out = copy_by_canvas(
            dab(vec![pixel(2, 3, 0.25), pixel(3, 3, 0.75)]),
            &s.transforms().unwrap(),
            16,
            16,
        );
        let got: Vec<(i32, i32, f32, bool)> = out
            .pixels
            .iter()
            .map(|p| (p.x, p.y, p.coverage, p.triangle.is_some()))
            .collect();
        assert_eq!(
            got,
            vec![
                (2, 3, 0.25, true),
                (3, 3, 0.75, true),
                (12, 3, 0.75, false),
                (13, 3, 0.25, false)
            ]
        );
        // 写しの画素の点は、元の画素の点（ステンシルを同じ所で読む）
        assert_eq!(out.pixels[2].position, Vec3::new(3.0, 3.0, 0.0));
    }

    #[test]
    fn a_diagonal_mirror_through_a_pixel_corner_copies_coverage_exactly_and_an_oblique_one_leaves_no_holes(
    ) {
        // 45 度の鏡（中心 (8, 8) を通る y = x）: 画素 (x, y) は (y, x) へ写る。軸が画素の角を通るので、覆いはそのまま写る
        let s = CanvasSymmetry::lines(DVec2::new(8.0, 8.0), 2, 45.0).unwrap();
        let out = copy_by_canvas(
            dab(vec![pixel(2, 3, 0.25), pixel(3, 3, 0.75)]),
            &s.transforms().unwrap(),
            16,
            16,
        );
        let mut got: Vec<(i32, i32, f32)> =
            out.pixels.iter().map(|p| (p.x, p.y, p.coverage)).collect();
        got.sort_by_key(|p| (p.1, p.0));
        assert_eq!(
            got,
            vec![(3, 2, 0.25), (2, 3, 0.25), (3, 3, 0.75)],
            "(2,3) の写しは (3,2)、軸の上の (3,3) は自分自身"
        );
        // 30 度の鏡: 中を満たした円板の写しにも穴が無い
        let s = CanvasSymmetry::lines(DVec2::new(64.0, 64.0), 2, 30.0).unwrap();
        let mut disc = Vec::new();
        for y in 70..90 {
            for x in 90..110 {
                let (dx, dy) = (x as f64 + 0.5 - 100.0, y as f64 + 0.5 - 80.0);
                if dx * dx + dy * dy <= 64.0 {
                    disc.push(pixel(x, y, 1.0));
                }
            }
        }
        let out = copy_by_canvas(dab(disc), &s.transforms().unwrap(), 128, 128);
        let (cx, cy) = s.transforms().unwrap()[1].map(100.0, 80.0);
        for y in 0..128 {
            for x in 0..128 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                if dx * dx + dy * dy <= 36.0 {
                    let p = out
                        .pixels
                        .iter()
                        .find(|p| (p.x, p.y) == (x, y))
                        .unwrap_or_else(|| panic!("写しの中の画素 ({x}, {y}) が抜けた"));
                    assert!(p.coverage > 0.99, "{p:?}");
                }
            }
        }
    }

    /// 覆いが同じ画素に重なる写しは先に置いた物が残る（`position` が変わる）ので、写しの並びが今の「両方」と違う線対称 4 本（0 度）をそのまま使うと、
    /// ステンシルを読む位置が変わりうる。対称定規は、今と同じ写しになる向きでは今のモードを返すので、並びまで同じになる。
    #[test]
    fn a_four_line_ruler_orders_its_copies_like_the_old_both_so_tied_pixels_keep_the_same_position()
    {
        use crate::{Ruler, RulerId, RulerKind};
        let center = DVec2::new(8.0, 8.0);
        let mut ruler = Ruler::canvas(RulerId(1), RulerKind::Symmetry, center, center + DVec2::X);
        ruler.lines = 4;
        let from_ruler = ruler.canvas_symmetry().unwrap();
        let both = CanvasSymmetry::new(SymmetryMode::Both, center, 2).unwrap();
        let lines = CanvasSymmetry::lines(center, 4, 0.0).unwrap();
        // (3, 5) の縦の写しと (12, 10) の横の写しは、どちらも (12, 5) に同じ覆いで重なる
        let source = || vec![pixel(3, 5, 1.0), pixel(12, 10, 1.0)];
        let run = |s: &CanvasSymmetry| {
            let out = copy_by_canvas(dab(source()), &s.transforms().unwrap(), 16, 16);
            out.pixels
                .iter()
                .map(|p| (p.x, p.y, p.coverage, p.position.to_array()))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&from_ruler), run(&both), "今の「両方」と位置まで同じ");
        let at = |v: &[(i32, i32, f32, [f32; 3])]| {
            v.iter()
                .find(|p| (p.0, p.1) == (12, 5))
                .map(|p| p.3)
                .unwrap()
        };
        assert_eq!(at(&run(&both)), [3.0, 5.0, 0.0], "縦の写しが先に残る");
        // 線対称そのものは写しの集合は同じでも、並びが違うので重なりの勝ちが変わる
        assert_ne!(at(&run(&lines)), at(&run(&both)));
    }

    #[test]
    fn overlapping_copies_keep_the_larger_coverage_and_rotations_have_no_holes() {
        // 中心の上の画素は、写しと重なって大きい方
        let s = CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(4.0, 4.0), 2).unwrap();
        let out = copy_by_canvas(
            dab(vec![pixel(3, 1, 0.5), pixel(4, 1, 0.9)]),
            &s.transforms().unwrap(),
            8,
            8,
        );
        let got: Vec<(i32, f32)> = out.pixels.iter().map(|p| (p.x, p.coverage)).collect();
        assert_eq!(got, vec![(3, 0.9), (4, 0.9)]);
        // 30° 回す放射状: 中を満たした円板の写しにも穴が無い
        let r = CanvasSymmetry::new(SymmetryMode::Radial, DVec2::new(64.0, 64.0), 12).unwrap();
        let mut disc = Vec::new();
        for y in 70..90 {
            for x in 90..110 {
                let (dx, dy) = (x as f64 + 0.5 - 100.0, y as f64 + 0.5 - 80.0);
                if dx * dx + dy * dy <= 64.0 {
                    disc.push(pixel(x, y, 1.0));
                }
            }
        }
        let n = disc.len();
        let out = copy_by_canvas(dab(disc), &r.transforms().unwrap(), 128, 128);
        let t = r.transforms().unwrap()[1];
        let (cx, cy) = t.map(100.0, 80.0);
        for y in 0..128 {
            for x in 0..128 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                if dx * dx + dy * dy <= 36.0 {
                    let p = out
                        .pixels
                        .iter()
                        .find(|p| (p.x, p.y) == (x, y))
                        .unwrap_or_else(|| panic!("写しの中の画素 ({x}, {y}) が抜けた"));
                    assert!(p.coverage > 0.99, "{:?}", p);
                }
            }
        }
        assert!(out.pixels.len() > n * 11, "{} {}", out.pixels.len(), n);
    }

    #[test]
    fn too_many_candidates_refuse_the_dab() {
        let s = CanvasSymmetry::new(SymmetryMode::Radial, DVec2::new(2048.0, 2048.0), 16).unwrap();
        let pixels: Vec<SurfacePixel> = (0..400 * 1100)
            .map(|i| pixel(1500 + i % 1100, 1900 + i / 1100, 1.0))
            .collect();
        let out = copy_by_canvas(dab(pixels), &s.transforms().unwrap(), 4096, 4096);
        assert_eq!(out.refusal, Some(DabRefusal::PixelBudget));
        assert!(out.pixels.is_empty());
    }

    #[test]
    fn a_copy_after_a_canvas_transform_maps_and_inverts() {
        let c = UvCopy::new(
            DVec2::new(10.0, 20.0),
            DVec2::new(50.0, 40.0),
            [-2.0, 0.5, 0.25, 1.5],
        )
        .unwrap();
        let s = CanvasSymmetry::new(SymmetryMode::Radial, DVec2::new(32.0, 32.0), 3).unwrap();
        let t = s.transforms().unwrap()[1];
        let both = c.then(&t);
        for (x, y) in [(10.0, 20.0), (13.5, 18.25), (-4.0, 7.0)] {
            let (mx, my) = c.map(x, y);
            let (ex, ey) = t.map(mx, my);
            let (gx, gy) = both.map(x, y);
            assert!((gx - ex).abs() < 1e-9 && (gy - ey).abs() < 1e-9);
            let (bx, by) = both.inverse(gx, gy);
            assert!((bx - x).abs() < 1e-9 && (by - y).abs() < 1e-9);
        }
        assert!(UvCopy::new(DVec2::ZERO, DVec2::ZERO, [1.0, 2.0, 2.0, 4.0]).is_none());
    }
}
