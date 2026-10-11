//! 範囲の索引: クリックした三角形から辿る範囲（三角形・メッシュの塊・UV アイランド・マテリアル）を、ジオメトリごとに 1 回の連結成分の
//! 計算で作っておき、カーソルが動くたびに引き直さない（core の `region` は呼ぶたびに全三角形の辺の表を作るので、ホバーのたびには使えない）。
//! 範囲の定義は core の `region` と同じ（同じスロットの中だけ、UV は 1e-6・位置は 1e-5 に量子化した頂点で辺を共有するもの）で、試験が
//! 全三角形・全種類で core と一致することを確かめる。
//!
//! 2D キャンバスで UV の点の下の三角形を引くための格子（`UvGrid`）と、範囲の UV の輪郭（`outline`）もここ。

use std::collections::HashMap;
use std::sync::Arc;

use yolu_core::geometry::{SurfaceGeometry, SurfaceRegionKind, SurfaceTriangle};
use yolu_core::glam::Vec2;

type Vertex = (i64, i64, i64);
type UvPoint = (i64, i64);
type EdgeKey = (i32, Vertex, Vertex);

fn quantize(v: f32, scale: f64) -> i64 {
    // C# の (long)Math.Round(x * scale)（float × double は倍精度、偶数への丸め）
    (v as f64 * scale).round_ties_even() as i64
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        parent[i as usize] = parent[parent[i as usize] as usize];
        i = parent[i as usize];
    }
    i
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[a.max(b) as usize] = a.min(b);
    }
}

/// 連結成分の番号（最小の三角形の番号の順）と、成分ごとの三角形（昇順）。
fn components(parent: &mut [u32]) -> (Vec<u32>, Vec<Vec<u32>>) {
    let n = parent.len();
    let mut of = vec![0u32; n];
    let mut members: Vec<Vec<u32>> = Vec::new();
    let mut ids: HashMap<u32, u32> = HashMap::new();
    for i in 0..n as u32 {
        let root = find(parent, i);
        let id = *ids.entry(root).or_insert_with(|| {
            members.push(Vec::new());
            (members.len() - 1) as u32
        });
        of[i as usize] = id;
        members[id as usize].push(i);
    }
    (of, members)
}

/// 辺を共有する三角形をつなぐ。`key` は三角形の 3 頂点を量子化した頂点にする。
fn connect(
    triangles: &[SurfaceTriangle],
    key: impl Fn(&SurfaceTriangle) -> [Vertex; 3],
) -> (Vec<u32>, Vec<Vec<u32>>) {
    let mut parent: Vec<u32> = (0..triangles.len() as u32).collect();
    let mut edges: HashMap<EdgeKey, u32> = HashMap::with_capacity(triangles.len() * 2);
    for (i, t) in triangles.iter().enumerate() {
        let [a, b, c] = key(t);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let edge = if p <= q {
                (t.material_slot, p, q)
            } else {
                (t.material_slot, q, p)
            };
            match edges.entry(edge) {
                std::collections::hash_map::Entry::Occupied(o) => {
                    union(&mut parent, i as u32, *o.get())
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(i as u32);
                }
            }
        }
    }
    components(&mut parent)
}

/// 範囲の索引（ジオメトリごと）。
pub struct RegionIndex {
    geometry: Arc<SurfaceGeometry>,
    island: Vec<u32>,
    island_members: Vec<Vec<u32>>,
    part: Vec<u32>,
    part_members: Vec<Vec<u32>>,
    materials: Vec<(i32, Vec<u32>)>,
    /// 0, 1, 2, …（三角形 1 つの範囲を借りて返すための並び）。
    identity: Vec<u32>,
}

impl RegionIndex {
    pub fn new(geometry: &Arc<SurfaceGeometry>) -> RegionIndex {
        let triangles = geometry.triangles();
        let (island, island_members) = connect(triangles, |t| {
            [t.uv_a, t.uv_b, t.uv_c].map(|uv| (quantize(uv.x, 1e6), quantize(uv.y, 1e6), 0))
        });
        let (part, part_members) = connect(triangles, |t| {
            [t.a, t.b, t.c].map(|p| (quantize(p.x, 1e5), quantize(p.y, 1e5), quantize(p.z, 1e5)))
        });
        let mut by_material: std::collections::BTreeMap<i32, Vec<u32>> = Default::default();
        for (i, t) in triangles.iter().enumerate() {
            by_material.entry(t.material).or_default().push(i as u32);
        }
        RegionIndex {
            geometry: geometry.clone(),
            island,
            island_members,
            part,
            part_members,
            materials: by_material.into_iter().collect(),
            identity: (0..triangles.len() as u32).collect(),
        }
    }

    /// この索引が `geometry` のものか。
    pub fn is_for(&self, geometry: &Arc<SurfaceGeometry>) -> bool {
        Arc::ptr_eq(&self.geometry, geometry)
    }

    pub fn geometry(&self) -> &Arc<SurfaceGeometry> {
        &self.geometry
    }

    /// 三角形 `triangle` を含む範囲の三角形の番号（昇順）。
    pub fn region(&self, triangle: u32, kind: SurfaceRegionKind) -> &[u32] {
        let i = triangle as usize;
        match kind {
            SurfaceRegionKind::Triangle => &self.identity[i..=i],
            SurfaceRegionKind::UvIsland => &self.island_members[self.island[i] as usize],
            SurfaceRegionKind::MeshPart => &self.part_members[self.part[i] as usize],
            SurfaceRegionKind::Material => {
                let m = self.geometry.triangles()[i].material;
                self.materials
                    .iter()
                    .find(|(k, _)| *k == m)
                    .map(|(_, v)| v.as_slice())
                    .unwrap_or_default()
            }
        }
    }

    /// 範囲の同一性の鍵（同じ範囲なら同じ値。ホバーの強調を引き直すかの判定）。
    pub fn key(&self, triangle: u32, kind: SurfaceRegionKind) -> u64 {
        let i = triangle as usize;
        match kind {
            SurfaceRegionKind::Triangle => (1 << 60) | triangle as u64,
            SurfaceRegionKind::UvIsland => (2 << 60) | self.island[i] as u64,
            SurfaceRegionKind::MeshPart => (3 << 60) | self.part[i] as u64,
            SurfaceRegionKind::Material => {
                (4 << 60) | self.geometry.triangles()[i].material as u32 as u64
            }
        }
    }

    /// 範囲の UV の輪郭（範囲の中で 1 つの三角形にしか属さない辺。UV の座標。三角形・UV アイランドは UV 上の外周、
    /// メッシュの塊・マテリアルは複数のアイランドの外周）。
    pub fn outline(&self, region: &[u32]) -> Vec<[Vec2; 2]> {
        let triangles = self.geometry.triangles();
        let mut count: HashMap<(UvPoint, UvPoint), (u32, [Vec2; 2])> = HashMap::new();
        for &i in region {
            let t = &triangles[i as usize];
            let q = |v: Vec2| (quantize(v.x, 1e6), quantize(v.y, 1e6));
            for (a, b) in [(t.uv_a, t.uv_b), (t.uv_b, t.uv_c), (t.uv_c, t.uv_a)] {
                let key = if q(a) <= q(b) {
                    (q(a), q(b))
                } else {
                    (q(b), q(a))
                };
                count.entry(key).or_insert((0, [a, b])).0 += 1;
            }
        }
        let mut out: Vec<_> = count
            .into_values()
            .filter(|(n, _)| *n == 1)
            .map(|(_, e)| e)
            .collect();
        // 並びを決める（同じ範囲は同じ線の列）
        out.sort_by(|a, b| {
            let key = |e: &[Vec2; 2]| {
                (
                    e[0].x.to_bits(),
                    e[0].y.to_bits(),
                    e[1].x.to_bits(),
                    e[1].y.to_bits(),
                )
            };
            key(a).cmp(&key(b))
        });
        out
    }
}

/// UV の点の下の三角形を引く格子（core のもの。2D のストロークの 3D の対称も同じ格子を使う）。
pub use yolu_core::geometry::UvGrid;

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::{demo_cube, model_triangles, ModelMesh, Submesh};
    use yolu_core::glam::Vec3;

    /// 2 つの板（別のスロットとマテリアル）と、UV の継ぎ目で切れた折れた板を 1 つのモデルにする。
    fn model(with_cube: bool) -> Arc<SurfaceGeometry> {
        let quad = |x: f32, uv0: f32, slots: [i32; 2]| ModelMesh {
            name: "板".into(),
            positions: vec![
                Vec3::new(x, 0.0, 0.0),
                Vec3::new(x + 1.0, 0.0, 0.0),
                Vec3::new(x, 1.0, 0.0),
                Vec3::new(x + 1.0, 1.0, 0.0),
                Vec3::new(x + 2.0, 0.0, 0.0),
                Vec3::new(x + 2.0, 1.0, 0.0),
            ],
            normals: Vec::new(),
            // 頂点 4・5 は位置が同じ x + 1 の頂点 1・3 と別の UV（UV の継ぎ目）
            uvs: vec![
                Vec2::new(uv0, 0.0),
                Vec2::new(uv0 + 0.2, 0.0),
                Vec2::new(uv0, 0.4),
                Vec2::new(uv0 + 0.2, 0.4),
                Vec2::new(uv0 + 0.3, 0.0),
                Vec2::new(uv0 + 0.3, 0.4),
            ],
            submeshes: vec![
                Submesh {
                    material: slots[0],
                    indices: vec![0, 1, 2, 2, 1, 3],
                },
                Submesh {
                    material: slots[1],
                    indices: vec![1, 4, 3, 3, 4, 5],
                },
            ],
        };
        let mut meshes = vec![quad(0.0, 0.0, [0, 0]), quad(5.0, 0.5, [0, 1])];
        if with_cube {
            meshes.push(demo_cube());
        }
        let triangles = model_triangles(&meshes).unwrap();
        Arc::new(SurfaceGeometry::new(triangles, 1, 1e-6).unwrap())
    }

    #[test]
    fn index_matches_the_core_region_for_every_triangle_and_kind() {
        let g = model(true);
        let index = RegionIndex::new(&g);
        for kind in [
            SurfaceRegionKind::Triangle,
            SurfaceRegionKind::UvIsland,
            SurfaceRegionKind::MeshPart,
            SurfaceRegionKind::Material,
        ] {
            for i in 0..g.triangle_count() as u32 {
                assert_eq!(
                    index.region(i, kind),
                    yolu_core::geometry::region(&g, i, kind).as_slice(),
                    "{kind:?} {i}"
                );
            }
        }
    }

    #[test]
    fn keys_are_the_same_exactly_inside_one_region() {
        let g = model(true);
        let index = RegionIndex::new(&g);
        for kind in [
            SurfaceRegionKind::Triangle,
            SurfaceRegionKind::UvIsland,
            SurfaceRegionKind::MeshPart,
            SurfaceRegionKind::Material,
        ] {
            for i in 0..g.triangle_count() as u32 {
                let region = index.region(i, kind);
                for j in 0..g.triangle_count() as u32 {
                    assert_eq!(
                        index.key(i, kind) == index.key(j, kind),
                        region.contains(&j),
                        "{kind:?} {i} {j}"
                    );
                }
            }
        }
    }

    #[test]
    fn outline_is_the_boundary_of_the_uv_region() {
        let g = model(true);
        let index = RegionIndex::new(&g);
        // 三角形 1 つは 3 辺
        assert_eq!(
            index
                .outline(index.region(0, SurfaceRegionKind::Triangle))
                .len(),
            3
        );
        // 2 枚の三角形でできた四角の UV アイランドは外周の 4 辺（共有する対角線は輪郭ではない）
        let island = index.region(0, SurfaceRegionKind::UvIsland);
        assert_eq!(island.len(), 2);
        assert_eq!(index.outline(island).len(), 4);
        // 立方体の 1 つの面（2 三角形）も 4 辺
        let cube_first = (g.triangle_count() - 12) as u32;
        let face = index.region(cube_first, SurfaceRegionKind::UvIsland);
        assert_eq!(index.outline(face).len(), 4);
    }

    #[test]
    fn uv_grid_finds_the_lowest_triangle_under_the_point_of_one_material() {
        let g = model(false);
        let grid0 = UvGrid::new(&g, 0);
        // 最初の板の下の三角形（頂点 0・1・2）の中
        let hit = grid0.find(Vec2::new(0.04, 0.05)).expect("三角形の中");
        assert!(g.triangles()[hit as usize].material == 0);
        assert_eq!(hit, 0);
        // どの三角形の UV でもない点・UV の外
        assert_eq!(grid0.find(Vec2::new(0.95, 0.95)), None);
        assert_eq!(grid0.find(Vec2::new(-0.1, 0.5)), None);
        // 別のマテリアル（スロット 1 の板）の UV は、マテリアル 0 の格子には出ない
        let grid1 = UvGrid::new(&g, 1);
        let other = grid1.find(Vec2::new(0.5 + 0.25, 0.1));
        assert!(other.is_some_and(|i| g.triangles()[i as usize].material == 1));
        assert_eq!(
            grid0.find(Vec2::new(0.5 + 0.25, 0.1)),
            None,
            "別のマテリアルの三角形は引かない"
        );
    }
}
