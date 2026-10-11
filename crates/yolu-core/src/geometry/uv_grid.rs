//! UV の点の下の三角形を引く格子（1 つのマテリアル（テクスチャセット）の三角形だけ）。2D のキャンバスで押した UV の下の面を引く
//! 範囲のツール・ベイクの見取り図と、2D のストロークに 3D の対称を当てるとき（ダブの中心の UV の下の面の点）に使う。

use std::sync::Arc;

use glam::{DVec2, Vec2, Vec3};

use super::query::uv_barycentric;
use super::{SurfaceGeometry, SurfaceTriangle};

/// UV の点の下の三角形を引く格子（1 つのマテリアルの三角形だけ）。
pub struct UvGrid {
    geometry: Arc<SurfaceGeometry>,
    material: i32,
    cells: Vec<Vec<u32>>,
    n: usize,
}

impl UvGrid {
    pub fn new(geometry: &Arc<SurfaceGeometry>, material: i32) -> UvGrid {
        let triangles = geometry.triangles();
        let count = triangles.iter().filter(|t| t.material == material).count();
        // 1 マスにおよそ数個の三角形が来る大きさ（16〜256）
        let n = ((count as f64 / 4.0).sqrt().ceil() as usize).clamp(16, 256);
        let mut cells = vec![Vec::new(); n * n];
        let bin = |v: f32| ((v.clamp(0.0, 1.0) * n as f32) as usize).min(n - 1);
        for (i, t) in triangles.iter().enumerate() {
            if t.material != material {
                continue;
            }
            let (x0, x1) = (
                t.uv_a.x.min(t.uv_b.x).min(t.uv_c.x),
                t.uv_a.x.max(t.uv_b.x).max(t.uv_c.x),
            );
            let (y0, y1) = (
                t.uv_a.y.min(t.uv_b.y).min(t.uv_c.y),
                t.uv_a.y.max(t.uv_b.y).max(t.uv_c.y),
            );
            if x1 < 0.0 || y1 < 0.0 || x0 > 1.0 || y0 > 1.0 {
                continue;
            }
            for y in bin(y0)..=bin(y1) {
                for x in bin(x0)..=bin(x1) {
                    cells[y * n + x].push(i as u32);
                }
            }
        }
        UvGrid {
            geometry: geometry.clone(),
            material,
            cells,
            n,
        }
    }

    pub fn is_for(&self, geometry: &Arc<SurfaceGeometry>, material: i32) -> bool {
        self.material == material && Arc::ptr_eq(&self.geometry, geometry)
    }

    pub fn geometry(&self) -> &Arc<SurfaceGeometry> {
        &self.geometry
    }

    pub fn material(&self) -> i32 {
        self.material
    }

    /// UV の点を含む三角形（重なっていれば番号の小さいもの）。UV の 0〜1 の外は None。
    pub fn find(&self, uv: Vec2) -> Option<u32> {
        self.find_all(uv).first().copied()
    }

    /// UV の点を含む三角形の全部（重なった UV では 2 つ以上。番号の昇順）。UV の 0〜1 の外は空。
    pub fn find_all(&self, uv: Vec2) -> Vec<u32> {
        if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
            return Vec::new();
        }
        let bin = |v: f32| ((v * self.n as f32) as usize).min(self.n - 1);
        let cell = &self.cells[bin(uv.y) * self.n + bin(uv.x)];
        let triangles = self.geometry.triangles();
        let mut out: Vec<u32> = cell
            .iter()
            .copied()
            .filter(|i| uv_barycentric(uv, &triangles[*i as usize]).is_some())
            .collect();
        out.sort_unstable();
        out
    }

    /// 文書の画素の点 `p`（UV × 文書の大きさ `size`）を含む三角形と、その重心座標（UV が重なっていれば番号のいちばん小さいもの）。
    /// 含む三角形が無ければ、`reach`（文書の画素）の内でいちばん近い三角形の、いちばん近い点（距離が同じなら番号の小さい三角形）。
    /// どちらも無ければ None。
    pub fn locate(&self, p: DVec2, size: DVec2, reach: f64) -> Option<(u32, Vec3)> {
        self.locate_all(p, size, reach).into_iter().next()
    }

    /// [`UvGrid::locate`] の、UV が重なった所では点を含む三角形の全部（番号の昇順）を返す形。含む三角形が無ければ、`reach` の内で
    /// いちばん近い三角形 1 つ。
    pub fn locate_all(&self, p: DVec2, size: DVec2, reach: f64) -> Vec<(u32, Vec3)> {
        let uv = Vec2::new((p.x / size.x) as f32, (p.y / size.y) as f32);
        let triangles = self.geometry.triangles();
        let inside: Vec<(u32, Vec3)> = self
            .find_all(uv)
            .into_iter()
            .filter_map(|t| uv_barycentric(uv, &triangles[t as usize]).map(|w| (t, w)))
            .collect();
        if !inside.is_empty() {
            return inside;
        }
        self.nearest(p, size, reach).into_iter().collect()
    }

    /// `reach`（文書の画素）の内でいちばん近い三角形の、いちばん近い点の重心座標。
    fn nearest(&self, p: DVec2, size: DVec2, reach: f64) -> Option<(u32, Vec3)> {
        let triangles = self.geometry.triangles();
        if !(reach > 0.0 && reach.is_finite() && size.x > 0.0 && size.y > 0.0) {
            return None;
        }
        let n = self.n as f64;
        let bin = |v: f64| (v.clamp(0.0, 1.0) * n).floor().min(n - 1.0) as usize;
        let (x0, x1) = (bin((p.x - reach) / size.x), bin((p.x + reach) / size.x));
        let (y0, y1) = (bin((p.y - reach) / size.y), bin((p.y + reach) / size.y));
        let mut best: Option<(f64, u32, Vec3)> = None;
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &i in &self.cells[y * self.n + x] {
                    let (d, w) = closest_in_pixels(p, &triangles[i as usize], size);
                    if d > reach * reach {
                        continue;
                    }
                    let better = match best {
                        None => true,
                        Some((bd, bi, _)) => d < bd || (d == bd && i < bi),
                    };
                    if better {
                        best = Some((d, i, w));
                    }
                }
            }
        }
        best.map(|(_, i, w)| (i, w))
    }
}

/// 文書の画素の点 p から、三角形の UV（× size）の上のいちばん近い点までの距離の 2 乗と、その点の重心座標。
fn closest_in_pixels(p: DVec2, t: &SurfaceTriangle, size: DVec2) -> (f64, Vec3) {
    let s = |v: Vec2| DVec2::new(v.x as f64 * size.x, v.y as f64 * size.y);
    let (a, b, c) = (s(t.uv_a), s(t.uv_b), s(t.uv_c));
    // Ericson の Voronoi 領域の式（2 次元）
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    let at = |w: DVec2| -> (f64, Vec3) {
        let q = a + ab * w.x + ac * w.y;
        (
            (p - q).length_squared(),
            Vec3::new((1.0 - w.x - w.y) as f32, w.x as f32, w.y as f32),
        )
    };
    if d1 <= 0.0 && d2 <= 0.0 {
        return at(DVec2::ZERO);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return at(DVec2::new(1.0, 0.0));
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return at(DVec2::new(v, 0.0));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return at(DVec2::new(0.0, 1.0));
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return at(DVec2::new(0.0, w));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return at(DVec2::new(1.0 - w, w));
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-300 || !denom.is_finite() {
        return (f64::INFINITY, Vec3::new(1.0, 0.0, 0.0));
    }
    at(DVec2::new(vb / denom, vc / denom))
}
