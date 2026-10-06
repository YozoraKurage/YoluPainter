//! レイの当たり・最近点・三角形ごとの式（C# の TryRaycast・TryFindClosestPoint・IntersectTriangle・TryUvBarycentric・ClosestPoint・
//! Barycentric・AtEdge・UvFootprintBounds）。式の順は C# と同じ。

use glam::{Vec2, Vec3};

use super::unity::{
    cross, dot, finite3, fmax, fmin, magnitude, mix2, normalized, sqr_magnitude, v2max, v2min, Ray,
};
use super::{SurfaceGeometry, SurfaceHit, SurfaceTriangle};

/// 1 本のレイの仕事の上限（節点を見る数・三角形を試す数）。超えたら `exceeded`。
pub(crate) struct RayQueryBudget {
    pub remaining_triangle_tests: i32,
    pub remaining_node_visits: i32,
    pub exceeded: bool,
}

struct RayState {
    origin: Vec3,
    direction: Vec3,
    cull: bool,
    nearest: f32,
    triangle: i64,
    barycentric: Vec3,
}

impl SurfaceGeometry {
    /// レイのいちばん近い当たり（cull_backfaces なら裏から当たる面は見ない。max_distance より遠い当たりは無いとみなす）。
    pub fn raycast(&self, ray: Ray, cull_backfaces: bool, max_distance: f32) -> Option<SurfaceHit> {
        self.raycast_internal(ray, cull_backfaces, max_distance, None)
    }

    pub(crate) fn raycast_internal(
        &self,
        ray: Ray,
        cull: bool,
        max_distance: f32,
        work: Option<&mut RayQueryBudget>,
    ) -> Option<SurfaceHit> {
        if self.triangles.is_empty()
            || !finite3(ray.origin)
            || !finite3(ray.direction)
            || sqr_magnitude(ray.direction) < 1e-20
        {
            return None;
        }
        // C# の ray.direction = ray.direction.normalized（右辺で 1 回、Ray の setter でもう 1 回）
        let direction = normalized(normalized(ray.direction));
        let mut s = RayState {
            origin: ray.origin,
            direction,
            cull,
            nearest: max_distance,
            triangle: -1,
            barycentric: Vec3::ZERO,
        };
        let mut work = work;
        self.raycast_node(0, &mut s, &mut work);
        if s.triangle < 0 {
            return None;
        }
        let index = s.triangle as u32;
        let t = &self.triangles[index as usize];
        let w = s.barycentric;
        Some(SurfaceHit {
            revision: self.revision,
            renderer: t.renderer,
            material_slot: t.material_slot,
            material: t.material,
            triangle: index,
            position: s.origin + s.direction * s.nearest,
            normal: t.normal(),
            barycentric: w,
            uv: mix2(t.uv_a, t.uv_b, t.uv_c, w),
            distance: s.nearest,
        })
    }

    fn raycast_node(&self, id: u32, s: &mut RayState, work: &mut Option<&mut RayQueryBudget>) {
        if let Some(w) = work.as_deref_mut() {
            if w.exceeded {
                return;
            }
            let before = w.remaining_node_visits;
            w.remaining_node_visits -= 1;
            if before <= 0 {
                w.exceeded = true;
                return;
            }
        }
        let node = self.nodes[id as usize];
        if !intersects_bounds(node.lo, node.hi, s.origin, s.direction, s.nearest) {
            return;
        }
        if node.count == 0 {
            self.raycast_node(node.left, s, work);
            self.raycast_node(node.right, s, work);
            return;
        }
        for i in node.start..node.start + node.count {
            if let Some(w) = work.as_deref_mut() {
                let before = w.remaining_triangle_tests;
                w.remaining_triangle_tests -= 1;
                if before <= 0 {
                    w.exceeded = true;
                    return;
                }
            }
            let index = self.indices[i as usize];
            if let Some((distance, weights)) = intersect_triangle(
                s.origin,
                s.direction,
                &self.triangles[index as usize],
                s.cull,
            ) {
                if distance < s.nearest {
                    s.nearest = distance;
                    s.triangle = index as i64;
                    s.barycentric = weights;
                }
            }
        }
    }

    /// point にいちばん近い面の上の点（C# の TryFindClosestPoint）。BVH を近い枝から辿り、今の最短より遠い枝は刈る。
    /// facing が 0 でなければ法線が facing と同じ側を向く三角形だけ、material が 0 以上ならその組の三角形だけ。同じ距離なら番号の
    /// 小さい三角形。節点を見る数が max_node_visits を超えたら `Err`（重なり合った幾何で止まらないように）。届く面が無ければ Ok(None)。
    pub fn find_closest_point(
        &self,
        point: Vec3,
        max_distance: f32,
        facing: Vec3,
        max_node_visits: i32,
        material: i32,
    ) -> Result<Option<SurfaceHit>, NodeBudgetExceeded> {
        if self.triangles.is_empty()
            || !finite3(point)
            || !finite3(facing)
            || max_distance.is_nan()
            || max_distance < 0.0
        {
            return Ok(None);
        }
        let mut best_sq = if max_distance == f32::INFINITY {
            f32::INFINITY
        } else {
            max_distance * max_distance
        };
        let mut best: i64 = -1;
        let mut best_point = Vec3::ZERO;
        let mut visits = 0i32;
        let use_facing = sqr_magnitude(facing) > 0.0;
        let mut stack: Vec<u32> = vec![0];
        while let Some(id) = stack.pop() {
            visits += 1;
            if visits > max_node_visits {
                return Err(NodeBudgetExceeded);
            }
            let node = self.nodes[id as usize];
            if node.bounds.sqr_distance(point) > best_sq {
                continue;
            }
            if node.count == 0 {
                let left = self.nodes[node.left as usize].bounds.sqr_distance(point);
                let right = self.nodes[node.right as usize].bounds.sqr_distance(point);
                if left <= right {
                    stack.push(node.right);
                    stack.push(node.left);
                } else {
                    stack.push(node.left);
                    stack.push(node.right);
                }
                continue;
            }
            for i in node.start..node.start + node.count {
                let index = self.indices[i as usize];
                let t = &self.triangles[index as usize];
                if use_facing && dot(t.normal(), facing) <= 0.0 {
                    continue;
                }
                if material >= 0 && t.material != material {
                    continue;
                }
                let closest = closest_point(point, t);
                let squared = sqr_magnitude(closest - point);
                if squared < best_sq || (squared == best_sq && (best < 0 || (index as i64) < best))
                {
                    best_sq = squared;
                    best = index as i64;
                    best_point = closest;
                }
            }
        }
        if best < 0 {
            return Ok(None);
        }
        let found = &self.triangles[best as usize];
        let w = barycentric(best_point, found);
        Ok(Some(SurfaceHit {
            revision: self.revision,
            renderer: found.renderer,
            material_slot: found.material_slot,
            material: found.material,
            triangle: best as u32,
            position: best_point,
            normal: found.normal(),
            barycentric: w,
            uv: mix2(found.uv_a, found.uv_b, found.uv_c, w),
            distance: best_sq.sqrt(),
        }))
    }

    /// 三角形の上の点（重心座標）が辺の上にあるか: いちばん小さい重みが 1e-6 以下か、3D でいちばん近い辺までの距離が
    /// 遮蔽の許し以下（C# の AtEdge）。
    pub(crate) fn at_edge(&self, triangle: u32, bary: Vec3) -> bool {
        if fmin(bary.x, fmin(bary.y, bary.z)) <= 1e-6 {
            return true;
        }
        let t = &self.triangles[triangle as usize];
        let twice_area = magnitude(cross(t.b - t.a, t.c - t.a));
        let to_bc = bary.x * twice_area / fmax(1e-30, magnitude(t.c - t.b));
        let to_ca = bary.y * twice_area / fmax(1e-30, magnitude(t.a - t.c));
        let to_ab = bary.z * twice_area / fmax(1e-30, magnitude(t.b - t.a));
        fmin(to_bc, fmin(to_ca, to_ab)) <= self.visibility_epsilon
    }
}

/// 最近点の探索が節点を見る数の上限を超えた。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeBudgetExceeded;

impl std::fmt::Display for NodeBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("最近点の探索が上限を超えました（重なった面を減らしてください）")
    }
}

impl std::error::Error for NodeBudgetExceeded {}

/// レイの区間 [0, max] が箱を通るか（C# の IntersectsBounds。lo・hi は箱の min・max）。
#[inline(always)]
pub(crate) fn intersects_bounds(
    lo: Vec3,
    hi: Vec3,
    origin: Vec3,
    direction: Vec3,
    max_distance: f32,
) -> bool {
    let mut min = 0.0f32;
    let mut max = max_distance;
    for axis in 0..3 {
        let d = direction[axis];
        let o = origin[axis];
        if d.abs() < 1e-12 {
            if o < lo[axis] || o > hi[axis] {
                return false;
            }
            continue;
        }
        let mut a = (lo[axis] - o) / d;
        let mut b = (hi[axis] - o) / d;
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        min = fmax(min, a);
        max = fmin(max, b);
        if max < min {
            return false;
        }
    }
    true
}

/// レイと三角形（Möller–Trumbore。C# の IntersectTriangle）。当たれば (距離, 重心座標 (1 − u − v, u, v))。
#[allow(clippy::manual_range_contains)] // C# と同じ比較で書く（NaN の扱いも同じ）
pub fn intersect_triangle(
    origin: Vec3,
    direction: Vec3,
    t: &SurfaceTriangle,
    cull_backfaces: bool,
) -> Option<(f32, Vec3)> {
    let e1 = t.b - t.a;
    let e2 = t.c - t.a;
    let p = cross(direction, e2);
    let determinant = dot(e1, p);
    let epsilon = fmax(1e-20, (sqr_magnitude(e1) * sqr_magnitude(e2)).sqrt() * 1e-7);
    if if cull_backfaces {
        determinant <= epsilon
    } else {
        determinant.abs() <= epsilon
    } {
        return None;
    }
    let inverse = 1.0 / determinant;
    let offset = origin - t.a;
    let u = dot(offset, p) * inverse;
    if u < -1e-6 || u > 1.000001 {
        return None;
    }
    let q = cross(offset, e1);
    let v = dot(direction, q) * inverse;
    if v < -1e-6 || u + v > 1.000001 {
        return None;
    }
    let distance = dot(e2, q) * inverse;
    if distance < 0.0 {
        return None;
    }
    Some((distance, Vec3::new(1.0 - u - v, u, v)))
}

/// UV の点の重心座標（C# の TryUvBarycentric）。三角形の中（−1e-6 の許しつき）なら Some。
pub fn uv_barycentric(point: Vec2, t: &SurfaceTriangle) -> Option<Vec3> {
    let a = t.uv_b - t.uv_a;
    let b = t.uv_c - t.uv_a;
    let p = point - t.uv_a;
    let determinant = a.x * b.y - a.y * b.x;
    if determinant.abs() <= 1e-12 {
        return None;
    }
    let u = (p.x * b.y - p.y * b.x) / determinant;
    let v = (a.x * p.y - a.y * p.x) / determinant;
    let w = Vec3::new(1.0 - u - v, u, v);
    if w.x >= -1e-6 && u >= -1e-6 && v >= -1e-6 {
        Some(w)
    } else {
        None
    }
}

/// 三角形の上で p にいちばん近い点（Ericson の Voronoi 領域の式。C# の ClosestPoint）。
pub fn closest_point(p: Vec3, t: &SurfaceTriangle) -> Vec3 {
    let ab = t.b - t.a;
    let ac = t.c - t.a;
    let ap = p - t.a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return t.a;
    }
    let bp = p - t.b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return t.b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return t.a + ab * (d1 / (d1 - d3));
    }
    let cp = p - t.c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return t.c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return t.a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return t.b + (t.c - t.b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let sum = va + vb + vc;
    if sum.abs() < 1e-30 {
        return t.a;
    }
    t.a + ab * (vb / sum) + ac * (vc / sum)
}

/// 三角形の上の点の重心座標（丸めの負は 0 にして、足して 1 に戻す。C# の Barycentric）。
pub fn barycentric(p: Vec3, t: &SurfaceTriangle) -> Vec3 {
    let e0 = t.b - t.a;
    let e1 = t.c - t.a;
    let e2 = p - t.a;
    let d00 = dot(e0, e0);
    let d01 = dot(e0, e1);
    let d11 = dot(e1, e1);
    let d20 = dot(e2, e0);
    let d21 = dot(e2, e1);
    let denominator = d00 * d11 - d01 * d01;
    if denominator.abs() < 1e-30 {
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / denominator;
    let w = (d00 * d21 - d01 * d20) / denominator;
    let weights = Vec3::new(fmax(0.0, 1.0 - v - w), fmax(0.0, v), fmax(0.0, w));
    let sum = weights.x + weights.y + weights.z;
    if sum > 0.0 {
        Vec3::new(weights.x / sum, weights.y / sum, weights.z / sum)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    }
}

/// 中心から半径の球が三角形の UV で覆う範囲の外接の箱（三角形の UV の箱と 0〜1 で切る。C# の UvFootprintBounds）。
/// 三角形の面積が 0 に近ければ None。
pub(crate) fn uv_footprint_bounds(
    t: &SurfaceTriangle,
    center: Vec3,
    radius: f32,
) -> Option<(Vec2, Vec2)> {
    let e1 = t.b - t.a;
    let e2 = t.c - t.a;
    let a = dot(e1, e1);
    let b = dot(e1, e2);
    let c = dot(e2, e2);
    let det = a * c - b * b;
    if det <= fmax(1e-30, a * c * 1e-10) {
        return None;
    }
    let duv1 = t.uv_b - t.uv_a;
    let duv2 = t.uv_c - t.uv_a;
    let gu = (e1 * (duv1.x * c - duv2.x * b) + e2 * (duv2.x * a - duv1.x * b)) / det;
    let gv = (e1 * (duv1.y * c - duv2.y * b) + e2 * (duv2.y * a - duv1.y * b)) / det;
    let delta = center - t.a;
    let projected = t.uv_a + Vec2::new(dot(gu, delta), dot(gv, delta));
    let uv_radius = Vec2::new(magnitude(gu), magnitude(gv)) * radius;
    let min = v2max(v2min(t.uv_a, v2min(t.uv_b, t.uv_c)), projected - uv_radius);
    let max = v2min(v2max(t.uv_a, v2max(t.uv_b, t.uv_c)), projected + uv_radius);
    // 0〜1 の外の UV は切る（繰り返し・UDIM にはしない）
    Some((v2max(Vec2::ZERO, min), v2min(Vec2::ONE, max)))
}

/// 被覆率（C# の式: 距離 ≤ 硬さ か 硬さ ≥ 0.9999 なら 1、ほかは max(0, 1 − SmoothStep((d − h) / (1 − h)))）。
/// 単精度の丸めで縁が −2.4e−7 ほどになるのを 0 で押さえる（Unity 版の SurfaceGeometry.EdgeCoverage と同じ）。
#[inline]
pub(crate) fn coverage(distance: f32, hardness: f32) -> f32 {
    if distance <= hardness || hardness >= 0.9999 {
        1.0
    } else {
        (1.0 - super::unity::smooth_step(0.0, 1.0, (distance - hardness) / (1.0 - hardness)))
            .max(0.0)
    }
}
