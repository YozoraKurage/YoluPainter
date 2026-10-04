//! 三角形の一部だけを残したスナップショット（3D ビューで一部の面を見せない・当てない）。
//!
//! - 隣り合わせは元のスナップショットのものを写す（残した三角形どうしの隣だけ。溶接し直さない）。ポーズで継ぎ目がずれた形から
//!   組み直すと、離れた継ぎ目でダブが止まるので、つながりは元の形のものを使う（`reposition` と同じ考え方）。
//! - BVH は残した三角形で作り直す。ブラシの大きさの基準（`brush_scale`）は元の値を引き継ぐ（面を隠してもブラシの大きさは変わらない）。

use super::build::{self, Cancel};
use super::{GeometryError, SurfaceGeometry, SurfaceTriangle};

impl SurfaceGeometry {
    /// `keep`（元の三角形ごと）が true の三角形だけを残した、新しい世代のスナップショット。
    /// `triangles` は残した三角形（元の並びのまま。数は `keep` の true の数）で、位置はポーズで元と違ってよく、スロット・マテリアルの
    /// 番号は残した側で組み直した番号でよい。レンダラーと UV は元と同じでなければ `Mismatch`。
    pub fn restrict(
        &self,
        keep: &[bool],
        triangles: Vec<SurfaceTriangle>,
        revision: u32,
    ) -> Result<SurfaceGeometry, GeometryError> {
        let n = self.triangles.len();
        if keep.len() != n {
            return Err(GeometryError::Mismatch);
        }
        let mut new_index = vec![u32::MAX; n];
        let mut kept = 0usize;
        for (i, &k) in keep.iter().enumerate() {
            if k {
                new_index[i] = kept as u32;
                kept += 1;
            }
        }
        if kept != triangles.len() {
            return Err(GeometryError::Mismatch);
        }
        for (i, old) in self.triangles.iter().enumerate() {
            if !keep[i] {
                continue;
            }
            let new = &triangles[new_index[i] as usize];
            if new.renderer != old.renderer
                || new.uv_a != old.uv_a
                || new.uv_b != old.uv_b
                || new.uv_c != old.uv_c
            {
                return Err(GeometryError::Mismatch);
            }
        }
        let mut offsets = Vec::with_capacity(kept + 1);
        let mut neighbors = Vec::new();
        offsets.push(0u32);
        for i in 0..n {
            if !keep[i] {
                continue;
            }
            neighbors.extend(
                self.neighbors(i)
                    .iter()
                    .filter(|&&j| keep[j as usize])
                    .map(|&j| new_index[j as usize]),
            );
            offsets.push(neighbors.len() as u32);
        }
        let mut g = build::build(
            triangles,
            revision,
            self.seam_tolerance,
            Cancel(None),
            Some((offsets, neighbors, self.non_manifold_edge_count)),
        )?;
        g.brush_scale = self.brush_scale;
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{cube_sphere, model_triangles, world_radius, Ray};

    fn sphere() -> (Vec<SurfaceTriangle>, SurfaceGeometry) {
        let tri = model_triangles(&[cube_sphere(6, 1.0)]).unwrap();
        let g = SurfaceGeometry::new(tri.clone(), 1, super::super::DEFAULT_WELD_TOLERANCE).unwrap();
        (tri, g)
    }

    /// y が 0 より上の三角形を残す。
    fn upper(tri: &[SurfaceTriangle]) -> Vec<bool> {
        tri.iter()
            .map(|t| (t.a.y + t.b.y + t.c.y) / 3.0 > 0.0)
            .collect()
    }

    fn kept(tri: &[SurfaceTriangle], keep: &[bool]) -> Vec<SurfaceTriangle> {
        tri.iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|(t, _)| *t)
            .collect()
    }

    #[test]
    fn keeping_everything_matches_the_original() {
        let (tri, g) = sphere();
        let all = vec![true; tri.len()];
        let same = g.restrict(&all, tri.clone(), 2).unwrap();
        assert_eq!(same.revision(), 2);
        assert_eq!(same.triangle_count(), g.triangle_count());
        for i in 0..g.triangle_count() {
            assert_eq!(same.neighbors(i), g.neighbors(i));
        }
        assert_eq!(same.bounds(), g.bounds());
    }

    #[test]
    fn the_rest_keeps_only_its_own_neighbours_renumbered() {
        let (tri, g) = sphere();
        let keep = upper(&tri);
        let part = g.restrict(&keep, kept(&tri, &keep), 2).unwrap();
        let count = keep.iter().filter(|k| **k).count();
        assert!(count > 0 && count < tri.len());
        assert_eq!(part.triangle_count(), count);
        // 新しい番号 → 元の番号
        let old: Vec<usize> = (0..tri.len()).filter(|&i| keep[i]).collect();
        for (new, &o) in old.iter().enumerate() {
            let want: Vec<u32> = g
                .neighbors(o)
                .iter()
                .filter(|&&j| keep[j as usize])
                .map(|&j| old.iter().position(|&x| x == j as usize).unwrap() as u32)
                .collect();
            assert_eq!(part.neighbors(new), &want[..], "三角形 {o}");
        }
        // 隠した側の隣は残っていない（境目の三角形は隣が減る）
        let lost = old
            .iter()
            .enumerate()
            .filter(|(new, &o)| part.neighbors(*new).len() < g.neighbors(o).len())
            .count();
        assert!(lost > 0, "境目の三角形の隣が減る");
    }

    #[test]
    fn removed_triangles_are_not_hit_and_the_brush_stays_the_same_size() {
        let (tri, g) = sphere();
        let keep = upper(&tri);
        let part = g.restrict(&keep, kept(&tri, &keep), 2).unwrap();
        // 真上から撃つと、残した側の表に当たる。真下から撃つと、隠した側の面は無く、内側の上の面（裏）には当たらない（裏面は除く）
        let from_above = Ray::new(glam::Vec3::new(0.0, 5.0, 0.0), -glam::Vec3::Y);
        assert!(part.raycast(from_above, true, f32::INFINITY).is_some());
        let from_below = Ray::new(glam::Vec3::new(0.0, -5.0, 0.0), glam::Vec3::Y);
        assert!(g.raycast(from_below, true, f32::INFINITY).is_some());
        assert!(
            part.raycast(from_below, true, f32::INFINITY).is_none(),
            "隠した側の面には当たらない"
        );
        // 当たった三角形は、残した三角形の番号
        let hit = part.raycast(from_above, true, f32::INFINITY).unwrap();
        assert!((hit.triangle as usize) < part.triangle_count());
        // 箱は小さくなるが、ブラシの大きさの基準は元のまま
        assert!(part.bounds().size().y < g.bounds().size().y);
        assert_eq!(part.brush_scale(), g.brush_scale());
        assert_eq!(
            world_radius(&part, 16.0, 2048),
            world_radius(&g, 16.0, 2048)
        );
    }

    #[test]
    fn restrict_refuses_a_different_selection() {
        let (tri, g) = sphere();
        let keep = upper(&tri);
        let mut some = kept(&tri, &keep);
        some.pop();
        assert_eq!(
            g.restrict(&keep, some, 2).err(),
            Some(GeometryError::Mismatch),
            "数が合わない"
        );
        assert_eq!(
            g.restrict(&keep[1..], kept(&tri, &keep), 2).err(),
            Some(GeometryError::Mismatch),
            "印の長さが合わない"
        );
        let mut moved_uv = kept(&tri, &keep);
        moved_uv[3].uv_a.x += 0.25;
        assert_eq!(
            g.restrict(&keep, moved_uv, 2).err(),
            Some(GeometryError::Mismatch),
            "UV が違う"
        );
        let mut broken = kept(&tri, &keep);
        broken[2].b.y = f32::NAN;
        assert_eq!(
            g.restrict(&keep, broken, 2).err(),
            Some(GeometryError::NonFinite)
        );
    }
}
