//! 3D ビューでクリックした三角形から辿る範囲（C# の SurfaceRegions.Region）。選択範囲・ポリゴン塗りつぶしに使う三角形の番号を返す
//! （三角形の UV を画素の選択範囲にするのは、選択範囲が core に入ってから）。

use super::build::FastMap;
use super::SurfaceGeometry;
use glam::{Vec2, Vec3};

/// クリックした三角形から選ぶ範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceRegionKind {
    /// その三角形だけ。
    Triangle,
    /// UV で辺を共有してつながる三角形（UV アイランド。UV を 1e-6 で量子化して比べる）。
    UvIsland,
    /// 3D の位置で辺を共有してつながる三角形（UV の継ぎ目をまたぐメッシュの塊。位置を 1e-5 で量子化して比べる）。
    MeshPart,
    /// 同じマテリアルの組の三角形すべて。
    Material,
}

type Key = (i64, i64, i64);
type EdgeKey = (i64, i64, i64, i64, i64, i64);

/// 三角形 start を含む範囲の三角形の番号（昇順）。UV アイランド・メッシュの塊は同じスロットの中だけを辿る。
pub fn region(geometry: &SurfaceGeometry, start: u32, kind: SurfaceRegionKind) -> Vec<u32> {
    let triangles = &geometry.triangles;
    let s = start as usize;
    assert!(s < triangles.len(), "三角形の番号が範囲外");
    let slot = triangles[s].material_slot;
    let material = triangles[s].material;
    match kind {
        SurfaceRegionKind::Triangle => return vec![start],
        SurfaceRegionKind::Material => {
            return (0..triangles.len() as u32)
                .filter(|&i| triangles[i as usize].material == material)
                .collect()
        }
        _ => {}
    }
    let key = |uv: Vec2, p: Vec3| -> Key {
        // C# の (long)Math.Round(uv.x * 1e6)（float × double は倍精度、偶数への丸め）
        if kind == SurfaceRegionKind::UvIsland {
            (
                (uv.x as f64 * 1e6).round_ties_even() as i64,
                (uv.y as f64 * 1e6).round_ties_even() as i64,
                0,
            )
        } else {
            (
                (p.x as f64 * 1e5).round_ties_even() as i64,
                (p.y as f64 * 1e5).round_ties_even() as i64,
                (p.z as f64 * 1e5).round_ties_even() as i64,
            )
        }
    };
    let edge = |a: Key, b: Key| -> EdgeKey {
        if a <= b {
            (a.0, a.1, a.2, b.0, b.1, b.2)
        } else {
            (b.0, b.1, b.2, a.0, a.1, a.2)
        }
    };
    let edges_of = |i: usize| -> [EdgeKey; 3] {
        let t = &triangles[i];
        let (a, b, c) = (key(t.uv_a, t.a), key(t.uv_b, t.b), key(t.uv_c, t.c));
        [edge(a, b), edge(b, c), edge(c, a)]
    };
    let mut edges: FastMap<EdgeKey, Vec<u32>> = FastMap::default();
    for (i, t) in triangles.iter().enumerate() {
        if t.material_slot != slot {
            continue;
        }
        for e in edges_of(i) {
            edges.entry(e).or_default().push(i as u32);
        }
    }
    let mut seen = vec![false; triangles.len()];
    seen[s] = true;
    let mut queue = std::collections::VecDeque::from([start]);
    let mut out = vec![start];
    while let Some(t) = queue.pop_front() {
        for e in edges_of(t as usize) {
            for &n in &edges[&e] {
                if !seen[n as usize] {
                    seen[n as usize] = true;
                    out.push(n);
                    queue.push_back(n);
                }
            }
        }
    }
    out.sort_unstable();
    out
}
