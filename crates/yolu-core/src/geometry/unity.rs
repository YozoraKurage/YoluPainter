//! Unity の Vector2・Vector3・Mathf・Bounds・Ray と同じ単精度の式（Unity 版の面の計算とビットまで同じ値を出す土台）。
//!
//! - 演算の順は UnityCsReference の C# と同じに書く（`Vector3.Dot` は x・y・z の順の和、`normalized` は長さで割る、など）。
//!   Unity のエディタの Mono は float の計算を単精度で行う（2026-10-04、同梱の Mono で 16777216 + 1 − 16777216 が 0 になるのを確かめた）。
//!   Rust も f32 のまま、FMA に縮めない（Rust は `mul_add` を書かない限り縮めない）。
//! - `Mathf.Min`・`Mathf.Max` は `a < b ? a : b` の比較（f32::min とは −0 と NaN の扱いが違う）。
//! - `Bounds` は中心と半分の大きさで持つ（min・max は毎回 中心 ∓ 半分）。これも Unity と同じ丸めにするため。
//! - `Bounds.SqrDistance` は Unity ではネイティブの関数なので、その C++ の式（Wild Magic の点と箱の距離）を写した。ダブの三角形の
//!   ふるい（箱の距離が半径を超えたら飛ばす）と最近点の探索の枝刈りにしか使わないので、境目のぴったりの値でしか結果に効かない。

use glam::{Vec2, Vec3};

/// `Vector3.kEpsilon`（`normalized` の下限）。
pub(crate) const K_EPSILON: f32 = 0.00001;

#[inline(always)]
pub(crate) fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

#[inline(always)]
pub(crate) fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

#[inline(always)]
pub(crate) fn sqr_magnitude(a: Vec3) -> f32 {
    a.x * a.x + a.y * a.y + a.z * a.z
}

/// `(float)Math.Sqrt(x*x + y*y + z*z)`。和は単精度で、倍精度の平方根を単精度に丸めた値は f32 の平方根と同じ（正しく丸めた値同士）。
#[inline(always)]
pub(crate) fn magnitude(a: Vec3) -> f32 {
    sqr_magnitude(a).sqrt()
}

/// `Vector3.normalized`（長さが kEpsilon 以下なら 0。逆数を掛けずに長さで割る）。
#[inline]
pub(crate) fn normalized(a: Vec3) -> Vec3 {
    let m = magnitude(a);
    if m > K_EPSILON {
        Vec3::new(a.x / m, a.y / m, a.z / m)
    } else {
        Vec3::ZERO
    }
}

#[inline(always)]
pub(crate) fn v2_magnitude(a: Vec2) -> f32 {
    (a.x * a.x + a.y * a.y).sqrt()
}

/// `Mathf.Min`。
#[inline(always)]
pub(crate) fn fmin(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `Mathf.Max`。
#[inline(always)]
pub(crate) fn fmax(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

#[inline(always)]
pub(crate) fn vmin(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(fmin(a.x, b.x), fmin(a.y, b.y), fmin(a.z, b.z))
}

#[inline(always)]
pub(crate) fn vmax(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(fmax(a.x, b.x), fmax(a.y, b.y), fmax(a.z, b.z))
}

#[inline(always)]
pub(crate) fn v2min(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(fmin(a.x, b.x), fmin(a.y, b.y))
}

#[inline(always)]
pub(crate) fn v2max(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(fmax(a.x, b.x), fmax(a.y, b.y))
}

/// `Mathf.Clamp01`。
#[inline(always)]
pub(crate) fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// `Mathf.SmoothStep(from, to, t)`。
#[inline]
pub(crate) fn smooth_step(from: f32, to: f32, t: f32) -> f32 {
    let t = clamp01(t);
    let t = -2.0 * t * t * t + 3.0 * t * t;
    to * t + from * (1.0 - t)
}

#[inline(always)]
pub(crate) fn finite(v: f32) -> bool {
    v.is_finite()
}

#[inline(always)]
pub(crate) fn finite2(v: Vec2) -> bool {
    v.x.is_finite() && v.y.is_finite()
}

#[inline(always)]
pub(crate) fn finite3(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// `a * bx + b * by + c * bz`（重心座標の補間。Unity の式の順: ((a·x + b·y) + c·z)）。
#[inline(always)]
pub(crate) fn mix3(a: Vec3, b: Vec3, c: Vec3, w: Vec3) -> Vec3 {
    a * w.x + b * w.y + c * w.z
}

#[inline(always)]
pub(crate) fn mix2(a: Vec2, b: Vec2, c: Vec2, w: Vec3) -> Vec2 {
    a * w.x + b * w.y + c * w.z
}

/// 軸の並びの箱（Unity の `Bounds`。中心と半分の大きさで持つ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub center: Vec3,
    pub extents: Vec3,
}

impl Bounds {
    /// `new Bounds(center, size)`。
    pub fn new(center: Vec3, size: Vec3) -> Bounds {
        Bounds {
            center,
            extents: size * 0.5,
        }
    }
    /// 1 点の箱（`new Bounds(p, Vector3.zero)`）。
    pub fn point(p: Vec3) -> Bounds {
        Bounds::new(p, Vec3::ZERO)
    }
    pub fn min(&self) -> Vec3 {
        self.center - self.extents
    }
    pub fn max(&self) -> Vec3 {
        self.center + self.extents
    }
    pub fn size(&self) -> Vec3 {
        self.extents * 2.0
    }
    /// `SetMinMax`。
    pub fn set_min_max(&mut self, min: Vec3, max: Vec3) {
        self.extents = (max - min) * 0.5;
        self.center = min + self.extents;
    }
    /// `Encapsulate(Vector3)`。
    pub fn encapsulate_point(&mut self, p: Vec3) {
        let (min, max) = (self.min(), self.max());
        self.set_min_max(vmin(min, p), vmax(max, p));
    }
    /// `Encapsulate(Bounds)`。
    pub fn encapsulate(&mut self, b: &Bounds) {
        self.encapsulate_point(b.center - b.extents);
        self.encapsulate_point(b.center + b.extents);
    }
    /// 点から箱までの距離の 2 乗（箱の中なら 0。Unity のネイティブの `CalculateSqrDistance` の式）。
    pub fn sqr_distance(&self, p: Vec3) -> f32 {
        let closest = p - self.center;
        let mut sum = 0.0f32;
        for (c, e) in [
            (closest.x, self.extents.x),
            (closest.y, self.extents.y),
            (closest.z, self.extents.z),
        ] {
            if c < -e {
                let d = c + e;
                sum += d * d;
            } else if c > e {
                let d = c - e;
                sum += d * d;
            }
        }
        sum
    }
}

/// 半直線（Unity の `Ray`。向きは作るときに `normalized` する）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub(crate) origin: Vec3,
    pub(crate) direction: Vec3,
}

impl Ray {
    /// `new Ray(origin, direction)`（向きは `normalized`。短すぎる向きは 0 になり、何にも当たらない）。
    pub fn new(origin: Vec3, direction: Vec3) -> Ray {
        Ray {
            origin,
            direction: normalized(direction),
        }
    }
    pub fn origin(&self) -> Vec3 {
        self.origin
    }
    pub fn direction(&self) -> Vec3 {
        self.direction
    }
    /// `GetPoint`。
    pub fn point(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::excessive_precision)] // 同梱の Mono が "R" で書いた値をそのまま
    fn single_precision_like_mono() {
        // 同梱の Mono で求めた値（tools/csharp-golden の確かめ。0.1f × 0.2f + 0.3f を FMA に縮めない）
        let (x, y, z) = (0.1f32, 0.2f32, 0.3f32);
        assert_eq!((x * y + z).to_bits(), 0x3ea3d70b);
        assert_eq!(normalized(Vec3::new(x, y, z)).x, 0.2672612);
        assert_eq!(smooth_step(0.0, 1.0, x), 0.028);
        let mut b = Bounds::point(Vec3::new(x, y, z));
        b.encapsulate_point(Vec3::new(1.1, -2.3, 0.7));
        assert_eq!(b.min(), Vec3::new(0.100000024, -2.3, 0.3));
        assert_eq!(b.max(), Vec3::new(1.1, 0.200000048, 0.7));
        let r = Ray::new(Vec3::new(x, y, z), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(r.direction(), Vec3::new(0.267261237, 0.5345225, 0.8017837));
        assert_eq!(r.point(2.0), Vec3::new(0.6345225, 1.269045, 1.90356731));
    }

    #[test]
    fn sqr_distance_is_zero_inside_and_squared_outside() {
        let b = Bounds::new(Vec3::ZERO, Vec3::splat(2.0));
        assert_eq!(b.sqr_distance(Vec3::new(0.5, -0.5, 1.0)), 0.0);
        assert_eq!(b.sqr_distance(Vec3::new(3.0, 0.0, 0.0)), 4.0);
        assert_eq!(b.sqr_distance(Vec3::new(2.0, -2.0, 0.0)), 2.0);
    }

    #[test]
    fn min_max_keep_the_first_on_ties_like_mathf() {
        assert_eq!(fmin(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(fmax(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
    }
}
