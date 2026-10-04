//! 三角形のスープから、頂点の溶接・辺の隣り合わせ・BVH を作る（C# の SurfaceGeometry のコンストラクター・BuildAdjacency・BuildBvh）。
//!
//! 並びは C# と同じにする（隣り合わせは辺の初出の順、BVH は前順の節点・中央値の選び方・ヒープソートへの切り替え）。並びが同じなら、
//! 幅優先で辿る順・レイの当たりの同じ距離のときの選び方・ダブの予算の使い方も同じになる。

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use glam::Vec3;

use super::unity::{finite2, finite3, magnitude, vmax, vmin, Bounds};
use super::{GeometryError, SurfaceGeometry, SurfaceTriangle};

/// 64 bit の値を混ぜるだけの速いハッシュ（溶接と辺の鍵。鍵は整数だけで、外から選ばれる値ではない）。
#[derive(Default)]
pub(crate) struct MixHasher(u64);

impl Hasher for MixHasher {
    fn finish(&self) -> u64 {
        mix(self.0)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = mix(self.0 ^ *b as u64);
        }
    }
    fn write_u32(&mut self, v: u32) {
        self.0 = mix(self.0 ^ v as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    }
    fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = mix(self.0 ^ v).wrapping_add(0x3c6e_f372_fe94_f82a);
    }
    fn write_i64(&mut self, v: i64) {
        self.write_u64(v as u64);
    }
}

/// splitmix64 の仕上げ（C# の PositionKey の Mix と同じ）。
#[inline(always)]
pub(crate) fn mix(mut v: u64) -> u64 {
    v ^= v >> 30;
    v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^ (v >> 31)
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<MixHasher>>;

/// 溶接の鍵（C# の PositionKey: 座標 ÷ 許し（倍精度）を偶数への丸めで整数に）。
#[inline]
fn position_key(p: Vec3, tolerance: f64) -> (i64, i64, i64) {
    (
        (p.x as f64 / tolerance).round_ties_even() as i64,
        (p.y as f64 / tolerance).round_ties_even() as i64,
        (p.z as f64 / tolerance).round_ties_even() as i64,
    )
}

/// BVH の節点（Count が 0 なら内側の節点で Left・Right を持つ）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct BvhNode {
    pub bounds: Bounds,
    /// 箱の min・max（中心 ∓ 半分。レイの度に計算し直さないよう先に求めておく。値は毎回求めるのと同じ）。
    pub lo: Vec3,
    pub hi: Vec3,
    pub left: u32,
    pub right: u32,
    pub start: u32,
    pub count: u32,
}

#[derive(Clone, Copy)]
struct BuildBox {
    min: Vec3,
    max: Vec3,
    center: Vec3,
}

/// 組み立てを途中でやめる印（別のスレッドで組むとき。立てたら `GeometryError::Canceled`）。
pub(crate) struct Cancel<'a>(pub Option<&'a AtomicBool>);

impl Cancel<'_> {
    #[inline]
    fn check(&self) -> Result<(), GeometryError> {
        match self.0 {
            Some(flag) if flag.load(Ordering::Relaxed) => Err(GeometryError::Canceled),
            _ => Ok(()),
        }
    }
}

/// 作り直さずに使う隣り合わせ（CSR の offsets・並び・非多様体の辺の数。位置だけ変えたスナップショットのため）。
pub(crate) type Adjacency = (Vec<u32>, Vec<u32>, u32);

pub(crate) fn build(
    source: Vec<SurfaceTriangle>,
    revision: u32,
    weld_tolerance: f32,
    cancel: Cancel<'_>,
    reuse: Option<Adjacency>,
) -> Result<SurfaceGeometry, GeometryError> {
    if !weld_tolerance.is_finite() || weld_tolerance <= 0.0 {
        return Err(GeometryError::InvalidTolerance);
    }
    cancel.check()?;
    let clock = Instant::now();
    let triangles = source;
    let n = triangles.len();
    if n > u32::MAX as usize / 4 {
        return Err(GeometryError::TooManyTriangles);
    }
    let mut boxes = Vec::with_capacity(n);
    let mut centers = Vec::with_capacity(n);
    let mut bounds = match triangles.first() {
        Some(t) => t.bounds(),
        None => Bounds::new(Vec3::ZERO, Vec3::ZERO),
    };
    for (i, t) in triangles.iter().enumerate() {
        if i & 255 == 0 {
            cancel.check()?;
        }
        if !finite3(t.a)
            || !finite3(t.b)
            || !finite3(t.c)
            || !finite2(t.uv_a)
            || !finite2(t.uv_b)
            || !finite2(t.uv_c)
            || !finite3(super::unity::cross(t.b - t.a, t.c - t.a))
        {
            return Err(GeometryError::NonFinite);
        }
        let b = t.bounds();
        centers.push(b.center);
        boxes.push(BuildBox {
            min: b.min(),
            max: b.max(),
            center: b.center,
        });
        bounds.encapsulate(&b);
    }
    if !finite3(bounds.size()) {
        return Err(GeometryError::BoundsOverflow);
    }
    let visibility_epsilon = super::unity::fmax(0.000_000_1, magnitude(bounds.size()) * 0.000_001);
    let snapshot_ms = clock.elapsed().as_secs_f64() * 1000.0;

    let clock = Instant::now();
    let (offsets, neighbors, non_manifold) = match reuse {
        Some(a) => a,
        None => build_adjacency(&triangles, weld_tolerance, &cancel)?,
    };
    let adjacency_ms = clock.elapsed().as_secs_f64() * 1000.0;

    let clock = Instant::now();
    let mut indices: Vec<u32> = (0..n as u32).collect();
    let mut nodes = Vec::new();
    if n != 0 {
        let mut b = BvhBuilder {
            indices: &mut indices,
            nodes: &mut nodes,
            boxes: &boxes,
            centers: &centers,
            cancel: &cancel,
        };
        b.build(0, n)?;
    }
    let bvh_ms = clock.elapsed().as_secs_f64() * 1000.0;
    cancel.check()?;

    Ok(SurfaceGeometry {
        triangles,
        adjacency_offsets: offsets,
        adjacency: neighbors,
        indices,
        nodes,
        visibility_epsilon,
        seam_tolerance: weld_tolerance,
        revision,
        bounds,
        non_manifold_edge_count: non_manifold,
        brush_scale: magnitude(bounds.size()),
        timings: super::BuildTimings {
            snapshot_ms,
            adjacency_ms,
            bvh_ms,
        },
    })
}

/// 隣り合わせ（C# の BuildAdjacency）: 頂点を量子化して溶接し、同じレンダラー・同じスロットで 2 つの三角形が使う辺だけを隣とする。
/// 3 つ以上が使う辺（非多様体）は隣にしない。並びは辺の初出の順。返すのは CSR（offsets と並び）と非多様体の辺の数。
#[allow(clippy::type_complexity)]
fn build_adjacency(
    triangles: &[SurfaceTriangle],
    tolerance: f32,
    cancel: &Cancel<'_>,
) -> Result<(Vec<u32>, Vec<u32>, u32), GeometryError> {
    #[derive(Clone, Copy)]
    struct EdgeUse {
        first: u32,
        second: u32,
        count: u32,
    }
    let n = triangles.len();
    let tol = tolerance as f64;
    let mut vertices: FastMap<(i64, i64, i64), u32> = FastMap::default();
    vertices.reserve(n);
    let mut edges: FastMap<(u32, u32, i32, i32), u32> = FastMap::default();
    edges.reserve(n * 2);
    let mut uses: Vec<EdgeUse> = Vec::with_capacity(n * 2);
    let mut weld = |p: Vec3| -> u32 {
        let next = vertices.len() as u32;
        *vertices.entry(position_key(p, tol)).or_insert(next)
    };
    for (i, t) in triangles.iter().enumerate() {
        if i & 255 == 0 {
            cancel.check()?;
        }
        let (a, b, c) = (weld(t.a), weld(t.b), weld(t.c));
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let key = (p.min(q), p.max(q), t.renderer, t.material_slot);
            match edges.get(&key) {
                None => {
                    edges.insert(key, uses.len() as u32);
                    uses.push(EdgeUse {
                        first: i as u32,
                        second: 0,
                        count: 1,
                    });
                }
                Some(&id) => {
                    let e = &mut uses[id as usize];
                    if e.count == 1 {
                        e.second = i as u32;
                    }
                    e.count += 1;
                }
            }
        }
    }
    let mut counts = vec![0u32; n];
    let mut non_manifold = 0u32;
    for (i, e) in uses.iter().enumerate() {
        if i & 1023 == 0 {
            cancel.check()?;
        }
        if e.count == 2 && e.first != e.second {
            counts[e.first as usize] += 1;
            counts[e.second as usize] += 1;
        } else if e.count > 2 {
            non_manifold += 1;
        }
    }
    let mut offsets = Vec::with_capacity(n + 1);
    let mut total = 0u32;
    offsets.push(0);
    for c in &counts {
        total += c;
        offsets.push(total);
    }
    let mut fill: Vec<u32> = offsets[..n].to_vec();
    let mut list = vec![0u32; total as usize];
    for (i, e) in uses.iter().enumerate() {
        if i & 1023 == 0 {
            cancel.check()?;
        }
        if e.count == 2 && e.first != e.second {
            let (f, s) = (e.first as usize, e.second as usize);
            list[fill[f] as usize] = e.second;
            fill[f] += 1;
            list[fill[s] as usize] = e.first;
            fill[s] += 1;
        }
    }
    Ok((offsets, list, non_manifold))
}

struct BvhBuilder<'a, 'c> {
    indices: &'a mut Vec<u32>,
    nodes: &'a mut Vec<BvhNode>,
    boxes: &'a [BuildBox],
    centers: &'a [Vec3],
    cancel: &'a Cancel<'c>,
}

impl BvhBuilder<'_, '_> {
    /// 前順で節点を足す（C# の BuildBvh）。8 つ以下で葉、ほかは中心の広がりの一番長い軸の中央値で分ける。
    fn build(&mut self, start: usize, count: usize) -> Result<u32, GeometryError> {
        self.cancel.check()?;
        let first = self.boxes[self.indices[start] as usize];
        let (mut min, mut max, mut cmin, mut cmax) =
            (first.min, first.max, first.center, first.center);
        for i in start + 1..start + count {
            if i & 1023 == 0 {
                self.cancel.check()?;
            }
            let b = self.boxes[self.indices[i] as usize];
            min = vmin(min, b.min);
            max = vmax(max, b.max);
            cmin = vmin(cmin, b.center);
            cmax = vmax(cmax, b.center);
        }
        let mut bounds = Bounds::new(Vec3::ZERO, Vec3::ZERO);
        bounds.set_min_max(min, max);
        let size = cmax - cmin;
        let id = self.nodes.len() as u32;
        self.nodes.push(BvhNode {
            bounds,
            lo: bounds.min(),
            hi: bounds.max(),
            left: u32::MAX,
            right: u32::MAX,
            start: start as u32,
            count: count as u32,
        });
        if count <= 8 {
            return Ok(id);
        }
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        let half = count / 2;
        self.select_median(start, start + count - 1, start + half, axis)?;
        let left = self.build(start, half)?;
        let right = self.build(start + half, count - half)?;
        let node = &mut self.nodes[id as usize];
        node.start = start as u32;
        node.count = 0;
        node.left = left;
        node.right = right;
        Ok(id)
    }

    #[inline(always)]
    fn key(&self, i: usize, axis: usize) -> f32 {
        self.centers[self.indices[i] as usize][axis]
    }

    /// 中央値の選び出し（C# の SelectMedian: 3 方向の分割の quickselect。深さを使い切ったらヒープソート）。
    fn select_median(
        &mut self,
        mut low: usize,
        mut high: usize,
        middle: usize,
        axis: usize,
    ) -> Result<(), GeometryError> {
        // C# の 2 * (int)Math.Log(n, 2) + 1（Math.Log(a, b) は ln(a) / ln(b)）
        let mut depth = 2 * (((high - low + 1) as f64).ln() / 2f64.ln()) as i64 + 1;
        while low < high {
            self.cancel.check()?;
            if depth == 0 {
                return self.heap_sort(low, high, axis);
            }
            depth -= 1;
            let a = self.key(low, axis);
            let b = self.key((low + high) / 2, axis);
            let c = self.key(high, axis);
            // Math.Max(Math.Min(a, b), Math.Min(Math.Max(a, b), c))（System.Math の float 版。値は有限）
            let pivot = math_max(math_min(a, b), math_min(math_max(a, b), c));
            let (mut lt, mut scan, mut gt) = (low as i64, low as i64, high as i64);
            while scan <= gt {
                if scan & 1023 == 0 {
                    self.cancel.check()?;
                }
                let value = self.key(scan as usize, axis);
                if value < pivot {
                    self.indices.swap(lt as usize, scan as usize);
                    lt += 1;
                    scan += 1;
                } else if value > pivot {
                    self.indices.swap(scan as usize, gt as usize);
                    gt -= 1;
                } else {
                    scan += 1;
                }
            }
            if (middle as i64) < lt {
                high = (lt - 1) as usize;
            } else if (middle as i64) > gt {
                low = (gt + 1) as usize;
            } else {
                return Ok(());
            }
        }
        Ok(())
    }

    fn heap_sort(&mut self, low: usize, high: usize, axis: usize) -> Result<(), GeometryError> {
        let count = high - low + 1;
        let mut i = count as i64 / 2 - 1;
        while i >= 0 {
            self.sift(low, i as usize, count, axis);
            i -= 1;
        }
        let mut end = count - 1;
        while end > 0 {
            if end & 255 == 0 {
                self.cancel.check()?;
            }
            self.indices.swap(low, low + end);
            self.sift(low, 0, end, axis);
            end -= 1;
        }
        Ok(())
    }

    fn sift(&mut self, low: usize, mut i: usize, length: usize, axis: usize) {
        while i * 2 + 1 < length {
            let mut child = i * 2 + 1;
            if child + 1 < length && self.key(low + child, axis) < self.key(low + child + 1, axis) {
                child += 1;
            }
            if self.key(low + i, axis) >= self.key(low + child, axis) {
                break;
            }
            self.indices.swap(low + i, low + child);
            i = child;
        }
    }
}

/// System.Math.Max(float, float)（NaN は来ない）。
#[inline(always)]
fn math_max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

#[inline(always)]
fn math_min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
