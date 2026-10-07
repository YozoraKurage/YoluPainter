//! 位置だけが変わったスナップショット（ポーズを付けた形）: 三角形の並び・UV・スロットは同じで、隣り合わせは元の形のものを使い、
//! BVH は木の形を保って箱だけ当て直す（refit）か、作り直す。
//!
//! - 隣り合わせを元の形のまま使うのは、つながりがポーズで変わらないため（ポーズで継ぎ目の頂点のウェイトがわずかに違って離れても、
//!   腕が体に触れても、面のつながりは元の形のもの）。溶接し直すと、ポーズによってダブが継ぎ目で止まったり触れた面へ漏れたりする。
//! - refit の箱は、同じ木の形で一から組んだときと同じ値になる（葉はその三角形の箱の min・max、内側は子の min・max の和。
//!   一から組むときも部分木の三角形の箱の min・max なので、集合が同じなら同じ値）。木の形が元の形に合っているほど速い。
//! - ブラシの大きさの基準（`brush_scale`）は元のスナップショットのものを引き継ぐ（ポーズで箱が変わってもブラシは同じ大きさ）。

use std::time::Instant;

use super::unity::{finite2, finite3, fmax, magnitude, vmax, vmin, Bounds};
use super::{BuildTimings, GeometryError, SurfaceGeometry, SurfaceTriangle};

/// 位置を変えたときの BVH の扱い。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BvhUpdate {
    /// 木の形はそのままで箱だけ当て直す（速い。大きく形が変わると箱が重なって当たりが遅くなる）。
    Refit,
    /// 新しい位置で作り直す（隣り合わせは元のまま）。
    Rebuild,
}

impl SurfaceGeometry {
    /// 同じ三角形（並び・レンダラー・スロット・マテリアル・UV）の位置だけを変えた、新しい世代のスナップショット。
    /// 三角形の数・スロット・UV が違えば `Mismatch`、位置が有限でなければ `NonFinite`。
    pub fn reposition(
        &self,
        triangles: Vec<SurfaceTriangle>,
        revision: u32,
        bvh: BvhUpdate,
    ) -> Result<SurfaceGeometry, GeometryError> {
        if triangles.len() != self.triangles.len()
            || triangles.iter().zip(&self.triangles).any(|(a, b)| {
                a.renderer != b.renderer
                    || a.material_slot != b.material_slot
                    || a.material != b.material
                    || a.uv_a != b.uv_a
                    || a.uv_b != b.uv_b
                    || a.uv_c != b.uv_c
            })
        {
            return Err(GeometryError::Mismatch);
        }
        if bvh == BvhUpdate::Rebuild {
            let mut g = super::build::build(
                triangles,
                revision,
                self.seam_tolerance,
                super::build::Cancel(None),
                Some((
                    self.adjacency_offsets.clone(),
                    self.adjacency.clone(),
                    self.non_manifold_edge_count,
                )),
            )?;
            g.brush_scale = self.brush_scale;
            return Ok(g);
        }
        let clock = Instant::now();
        let mut boxes = Vec::with_capacity(triangles.len());
        let mut bounds = match triangles.first() {
            Some(t) => t.bounds(),
            None => Bounds::new(glam::Vec3::ZERO, glam::Vec3::ZERO),
        };
        for t in &triangles {
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
            boxes.push((b.min(), b.max()));
            bounds.encapsulate(&b);
        }
        if !finite3(bounds.size()) {
            return Err(GeometryError::BoundsOverflow);
        }
        let snapshot_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let clock = Instant::now();
        let mut nodes = self.nodes.clone();
        // 前順なので子は親より後ろ。後ろから回せば子が先に決まる
        let mut raw = vec![(glam::Vec3::ZERO, glam::Vec3::ZERO); nodes.len()];
        for id in (0..nodes.len()).rev() {
            let n = nodes[id];
            let (min, max) = if n.count == 0 {
                let (l, r) = (raw[n.left as usize], raw[n.right as usize]);
                (vmin(l.0, r.0), vmax(l.1, r.1))
            } else {
                let first = boxes[self.indices[n.start as usize] as usize];
                let (mut min, mut max) = first;
                for i in n.start + 1..n.start + n.count {
                    let b = boxes[self.indices[i as usize] as usize];
                    min = vmin(min, b.0);
                    max = vmax(max, b.1);
                }
                (min, max)
            };
            raw[id] = (min, max);
            let node = &mut nodes[id];
            node.bounds.set_min_max(min, max);
            node.lo = node.bounds.min();
            node.hi = node.bounds.max();
        }
        let bvh_ms = clock.elapsed().as_secs_f64() * 1000.0;
        Ok(SurfaceGeometry {
            triangles,
            adjacency_offsets: self.adjacency_offsets.clone(),
            adjacency: self.adjacency.clone(),
            indices: self.indices.clone(),
            nodes,
            visibility_epsilon: fmax(0.000_000_1, magnitude(bounds.size()) * 0.000_001),
            seam_tolerance: self.seam_tolerance,
            revision,
            bounds,
            non_manifold_edge_count: self.non_manifold_edge_count,
            brush_scale: self.brush_scale,
            timings: BuildTimings {
                snapshot_ms,
                adjacency_ms: 0.0,
                bvh_ms,
            },
            projection_cache: std::sync::Mutex::new(
                self.projection_cache
                    .lock()
                    .map(|c| c.clone())
                    .unwrap_or_default(),
            ),
        })
    }

    /// BVH の箱の表面積の和（葉と内側。refit で木の形が合わなくなった度合いを見る目安。計測用）。
    pub fn bvh_surface_area(&self) -> f64 {
        self.nodes
            .iter()
            .map(|n| {
                let s = n.bounds.size();
                2.0 * (s.x as f64 * s.y as f64 + s.y as f64 * s.z as f64 + s.z as f64 * s.x as f64)
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;
    use crate::geometry::{
        intersect_triangle, model_triangles, world_radius, Ray, DEFAULT_WELD_TOLERANCE,
    };
    use crate::skin::{demo_figure, FigureDetail, Pose, Rig};

    fn figure() -> (Rig, SurfaceGeometry) {
        let rig = demo_figure(FigureDetail::SMALL);
        let rest = rig.deform(&rig.rest_pose()).unwrap();
        let g = SurfaceGeometry::new(model_triangles(&rest).unwrap(), 1, DEFAULT_WELD_TOLERANCE)
            .unwrap();
        (rig, g)
    }

    fn bent(rig: &Rig) -> Pose {
        let mut pose = rig.rest_pose();
        let find = |name: &str| rig.bones().iter().position(|b| b.name == name).unwrap();
        // 右腕を下ろして肘を曲げ、左脚を前へ、背を少し丸める
        pose.locals[find("右上腕")].rotation = Quat::from_rotation_z(-1.3);
        pose.locals[find("右前腕")].rotation = Quat::from_rotation_y(1.2);
        pose.locals[find("左太もも")].rotation = Quat::from_rotation_x(-0.9);
        pose.locals[find("背骨")].rotation = Quat::from_rotation_x(0.35);
        pose.blend_weights[0][0] = 100.0;
        pose
    }

    fn posed(rig: &Rig, pose: &Pose) -> Vec<SurfaceTriangle> {
        model_triangles(&rig.deform(pose).unwrap()).unwrap()
    }

    /// 全部の三角形を総当たりした、いちばん近い当たり（表の面だけ）。
    fn brute(triangles: &[SurfaceTriangle], ray: Ray) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        for (i, t) in triangles.iter().enumerate() {
            if let Some((d, _)) = intersect_triangle(ray.origin(), ray.direction(), t, true) {
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((i as u32, d));
                }
            }
        }
        best
    }

    fn rays(g: &SurfaceGeometry, n: usize) -> Vec<Ray> {
        let mut s = 0x9E37_79B9u64;
        let mut r = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
        };
        let (c, e) = (g.bounds().center, g.bounds().extents.length());
        (0..n)
            .map(|_| {
                let o = c + Vec3::new(r(), r(), r()) * e * 2.0;
                let t = c + Vec3::new(r(), r(), r()) * e * 0.5;
                Ray::new(o, t - o)
            })
            .collect()
    }

    #[test]
    fn refit_with_the_same_positions_keeps_every_box() {
        let (_, g) = figure();
        let same = g
            .reposition(g.triangles().to_vec(), 2, BvhUpdate::Refit)
            .unwrap();
        assert_eq!(same.revision(), 2);
        assert_eq!(same.bounds(), g.bounds());
        assert_eq!(same.nodes.len(), g.nodes.len());
        for (a, b) in same.nodes.iter().zip(&g.nodes) {
            assert_eq!(a.bounds, b.bounds);
            assert_eq!((a.lo, a.hi), (b.lo, b.hi));
        }
        assert_eq!(same.visibility_epsilon(), g.visibility_epsilon());
    }

    #[test]
    fn posed_snapshot_hits_like_brute_force_with_refit_and_rebuild() {
        let (rig, g) = figure();
        let tri = posed(&rig, &bent(&rig));
        let refit = g.reposition(tri.clone(), 2, BvhUpdate::Refit).unwrap();
        let rebuilt = g.reposition(tri.clone(), 3, BvhUpdate::Rebuild).unwrap();
        assert!(refit.bounds() != g.bounds(), "ポーズで箱が変わる");
        let mut hits = 0;
        for ray in rays(&refit, 2000) {
            let want = brute(&tri, ray);
            let a = refit.raycast(ray, true, f32::INFINITY);
            let b = rebuilt.raycast(ray, true, f32::INFINITY);
            // 三角形は同じ。距離は raycast が向きをもう 1 度 normalized する（C# と同じ）分の丸めだけ違ってよい
            assert_eq!(a.map(|h| h.triangle), want.map(|w| w.0));
            assert_eq!(b.map(|h| h.triangle), want.map(|w| w.0));
            if let (Some(a), Some((_, d))) = (a, want) {
                assert!((a.distance - d).abs() <= d * 1e-6, "{} と {d}", a.distance);
            }
            assert_eq!(a.map(|h| (h.distance, h.uv)), b.map(|h| (h.distance, h.uv)));
            hits += want.is_some() as usize;
        }
        assert!(hits > 200, "当たりが少なすぎる: {hits}");
        // 隣り合わせは元の形のまま、作り直しでも同じ
        for i in 0..g.triangle_count() {
            assert_eq!(refit.neighbors(i), g.neighbors(i));
            assert_eq!(rebuilt.neighbors(i), g.neighbors(i));
        }
        // ブラシの大きさの基準も元のまま
        assert_eq!(refit.brush_scale(), g.brush_scale());
        assert_eq!(rebuilt.brush_scale(), g.brush_scale());
        assert_eq!(
            world_radius(&refit, 16.0, 2048),
            world_radius(&g, 16.0, 2048)
        );
    }

    #[test]
    fn a_ray_at_a_posed_triangle_finds_its_uv() {
        let (rig, g) = figure();
        let pose = bent(&rig);
        let tri = posed(&rig, &pose);
        let refit = g.reposition(tri.clone(), 2, BvhUpdate::Refit).unwrap();
        // 曲げた右の前腕の三角形（UV アイランドは 2 番目）の真ん中へ、面の法線の向きの外から撃つ
        let arm = &rig.meshes()[1];
        assert_eq!(arm.mesh.name, "右腕");
        let first = rig.meshes()[0].mesh.triangle_count();
        let count = arm.mesh.triangle_count();
        let mut checked = 0;
        for i in (first + count / 2..first + count).step_by(97) {
            let t = &tri[i];
            let center = (t.a + t.b + t.c) / 3.0;
            let n = t.normal();
            let hit = refit
                .raycast(Ray::new(center + n * 0.02, -n), true, f32::INFINITY)
                .expect("当たる");
            assert_eq!(hit.triangle as usize, i);
            let uv = (t.uv_a + t.uv_b + t.uv_c) / 3.0;
            assert!((hit.uv - uv).length() < 1e-4, "UV {} と {}", hit.uv, uv);
            // 休みの形では、同じ三角形はずっと離れた所にある（ポーズで動いた）
            let rest = &g.triangles()[i];
            assert!(((rest.a + rest.b + rest.c) / 3.0 - center).length() > 0.05);
            checked += 1;
        }
        assert!(checked >= 3);
    }

    #[test]
    fn reposition_refuses_other_triangles() {
        let (rig, g) = figure();
        let mut tri = posed(&rig, &bent(&rig));
        tri.pop();
        assert_eq!(
            g.reposition(tri, 2, BvhUpdate::Refit).err(),
            Some(GeometryError::Mismatch)
        );
        let mut tri = posed(&rig, &bent(&rig));
        tri[5].uv_a.x += 0.25;
        assert_eq!(
            g.reposition(tri, 2, BvhUpdate::Refit).err(),
            Some(GeometryError::Mismatch)
        );
        let mut tri = posed(&rig, &bent(&rig));
        tri[3].material_slot += 1;
        assert_eq!(
            g.reposition(tri, 2, BvhUpdate::Rebuild).err(),
            Some(GeometryError::Mismatch)
        );
        let mut tri = posed(&rig, &bent(&rig));
        tri[7].b.y = f32::NAN;
        assert_eq!(
            g.reposition(tri.clone(), 2, BvhUpdate::Refit).err(),
            Some(GeometryError::NonFinite)
        );
        assert_eq!(
            g.reposition(tri, 2, BvhUpdate::Rebuild).err(),
            Some(GeometryError::NonFinite)
        );
    }
}
