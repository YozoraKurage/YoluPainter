//! ハイドのモードで隠す単位（UV アイランド）の索引: UV と 3D の位置の両方で辺を共有してつながる面の組（同じスロットの中だけ。UV は
//! 1e-6・位置は 1e-5 に丸めた頂点で比べる。バケツの「UV アイランド」と同じ丸めで、位置でもつながることを足したもの）。UV だけで
//! つなぐと、左右の UV を重ねたモデルで反対側の面まで 1 つになって隠れるので、位置でも分ける（左右を重ねていないモデルでは、
//! バケツの UV アイランドと同じ組になる）。三角形の番号は受けたままの形のもの。形（休みの形）ごとに 1 回だけ作る。

use std::collections::HashMap;
use std::sync::Arc;

use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle};

/// 頂点の鍵（UV の x・y と位置の x・y・z を丸めた整数）。
type Vertex = [i64; 5];

fn quantize(v: f32, scale: f64) -> i64 {
    // バケツの UV アイランド（core の `region`）と同じ丸め（倍精度、偶数への丸め）
    (v as f64 * scale).round_ties_even() as i64
}

fn corners(t: &SurfaceTriangle) -> [Vertex; 3] {
    let key = |uv: yolu_core::glam::Vec2, p: yolu_core::glam::Vec3| {
        [
            quantize(uv.x, 1e6),
            quantize(uv.y, 1e6),
            quantize(p.x, 1e5),
            quantize(p.y, 1e5),
            quantize(p.z, 1e5),
        ]
    };
    [key(t.uv_a, t.a), key(t.uv_b, t.b), key(t.uv_c, t.c)]
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        parent[i as usize] = parent[parent[i as usize] as usize];
        i = parent[i as usize];
    }
    i
}

/// アイランドの索引（三角形 → アイランドの番号、アイランド → 三角形。番号は、いちばん小さい三角形の番号の順）。
#[derive(Debug)]
pub struct IslandIndex {
    /// 作った形（アドレスと世代）。
    key: (usize, u32),
    of: Vec<u32>,
    members: Vec<Arc<[u32]>>,
}

impl IslandIndex {
    pub fn new(g: &Arc<SurfaceGeometry>) -> IslandIndex {
        let triangles = g.triangles();
        let n = triangles.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        let mut edges: HashMap<(i32, Vertex, Vertex), u32> = HashMap::with_capacity(n * 2);
        for (i, t) in triangles.iter().enumerate() {
            let [a, b, c] = corners(t);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let edge = if p <= q {
                    (t.material_slot, p, q)
                } else {
                    (t.material_slot, q, p)
                };
                match edges.entry(edge) {
                    std::collections::hash_map::Entry::Occupied(o) => {
                        let (x, y) = (find(&mut parent, i as u32), find(&mut parent, *o.get()));
                        if x != y {
                            parent[x.max(y) as usize] = x.min(y);
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(v) => {
                        v.insert(i as u32);
                    }
                }
            }
        }
        let mut of = vec![0u32; n];
        let mut members: Vec<Vec<u32>> = Vec::new();
        let mut ids: HashMap<u32, u32> = HashMap::new();
        for i in 0..n as u32 {
            let root = find(&mut parent, i);
            let id = *ids.entry(root).or_insert_with(|| {
                members.push(Vec::new());
                (members.len() - 1) as u32
            });
            of[i as usize] = id;
            members[id as usize].push(i);
        }
        IslandIndex {
            key: (Arc::as_ptr(g) as usize, g.revision()),
            of,
            members: members.into_iter().map(Arc::from).collect(),
        }
    }

    /// この形から作った索引か。
    pub fn is_for(&self, g: &Arc<SurfaceGeometry>) -> bool {
        self.key == (Arc::as_ptr(g) as usize, g.revision())
    }

    /// アイランドの数。
    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 三角形の数。
    pub fn triangle_count(&self) -> usize {
        self.of.len()
    }

    /// この三角形（受けたままの形の番号）のアイランド。
    pub fn island_of(&self, triangle: u32) -> Option<u32> {
        self.of.get(triangle as usize).copied()
    }

    /// アイランドの三角形（昇順）。
    pub fn triangles(&self, island: u32) -> &[u32] {
        self.members
            .get(island as usize)
            .map(|m| &m[..])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::DEFAULT_WELD_TOLERANCE;
    use yolu_core::glam::{Vec2, Vec3};

    /// 板 1 枚（三角形 2 つ）。`x` だけずらし、UV は `u0` から。
    fn quad(x: f32, u0: f32) -> [SurfaceTriangle; 2] {
        let (a, b, c, d) = (
            Vec3::new(x, 0.0, 0.0),
            Vec3::new(x + 1.0, 0.0, 0.0),
            Vec3::new(x + 1.0, 1.0, 0.0),
            Vec3::new(x, 1.0, 0.0),
        );
        let (ua, ub, uc, ud) = (
            Vec2::new(u0, 0.0),
            Vec2::new(u0 + 0.4, 0.0),
            Vec2::new(u0 + 0.4, 1.0),
            Vec2::new(u0, 1.0),
        );
        [
            SurfaceTriangle::new(a, b, c, ua, ub, uc),
            SurfaceTriangle::new(a, c, d, ua, uc, ud),
        ]
    }

    #[test]
    fn faces_joined_in_uv_and_in_position_are_one_island_and_mirrored_uvs_are_split() {
        // 左右の板が同じ UV を重ねて使う（鏡のモデル）: UV だけなら 1 つだが、位置が離れているので 2 つ
        let mut t = quad(0.0, 0.0).to_vec();
        t.extend(quad(3.0, 0.0));
        // UV の別の所にある 3 枚目
        t.extend(quad(6.0, 0.5));
        let g = Arc::new(SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap());
        let index = IslandIndex::new(&g);
        assert_eq!(index.len(), 3);
        assert_eq!(index.triangles(0), &[0, 1]);
        assert_eq!(index.triangles(1), &[2, 3]);
        assert_eq!(index.island_of(5), Some(2));
        assert!(index.is_for(&g));
        assert_eq!(index.triangle_count(), 6);
        // UV の継ぎ目で分かれた板は、位置でつながっていても別のアイランド（バケツの UV アイランドと同じ）
        let bucket =
            yolu_core::geometry::region(&g, 4, yolu_core::geometry::SurfaceRegionKind::UvIsland);
        assert_eq!(bucket, index.triangles(2));
    }
}
