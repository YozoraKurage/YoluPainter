//! UDIM: 1 つの文書（シート）にタイルのセルを格子に並べ、UV の (u + i, v + j) のタイルをそのセルへ写す。
//!
//! - タイルの番号は `1001 + floor(u) + 10 × floor(v)`（u のタイルは 0〜9、v のタイルは 0〜99。番号は 1001〜2000）。
//! - シート: タイルを番号の順に、列の数 = ceil(√n)、行の数 = ceil(n / 列) の格子へ、下の行から左から右へ詰める（UV の位置どおりには並べない。
//!   1001〜1006 の横並びで辺が上限を超えるため）。余ったセルは「空のセル」（書き出さない）。セルはどれも同じ大きさ（W × H）で、シートは
//!   列 × W と 行 × H。
//! - UV の写し（[`map_triangle`]）: 三角形の UV の重心のタイル (floor(u), floor(v)) のセルへ、uv' = (セルの原点 + (uv − タイルの原点)) / シート。
//!   重心のタイルから外れる頂点を持つ三角形（タイルをまたぐ三角形）も重心のタイルに入れ、呼び手が数える。表に無いタイルの三角形は、シートの
//!   0〜1 の外（u − 2 の側）へ置き、描けない三角形にする（UDIM でないセットの 0〜1 の外と同じ扱い）。
//! - [`remap_meshes`]: 索引つきのメッシュを写す。違うセルの三角形が頂点を共有していたら頂点を複製する（元の UV のメッシュは呼び手が別に持つ）。
//!
//! 画素の並びは文書と同じ左下原点（行 0 が下）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec2;

use crate::geometry::{ModelMesh, SurfaceTriangle};
use crate::{CoreError, Reason, Rect};

/// 一番小さいタイルの番号。
pub const FIRST_TILE: u16 = 1001;
/// 一番大きいタイルの番号（u のタイル 9・v のタイル 99）。
pub const LAST_TILE: u16 = 2000;
/// 1 つのシートに置けるタイルの数の上限。
pub const MAX_TILES: usize = 100;
/// タイルをまたぐかを見るときの UV の許し（頂点がちょうどタイルの境にあるのはまたがない）。
const STRADDLE_EPSILON: f32 = 1e-4;

/// UV の点のタイルの番号（範囲の外・有限でなければ None）。
pub fn tile_of(uv: Vec2) -> Option<u16> {
    if !uv.x.is_finite() || !uv.y.is_finite() {
        return None;
    }
    let (u, v) = (uv.x.floor(), uv.y.floor());
    if !(0.0..10.0).contains(&u) || !(0.0..100.0).contains(&v) {
        return None;
    }
    Some(FIRST_TILE + u as u16 + 10 * v as u16)
}

/// タイルの UV の原点（左下の角）。
pub fn tile_origin(tile: u16) -> Vec2 {
    let k = tile.saturating_sub(FIRST_TILE);
    Vec2::new((k % 10) as f32, (k / 10) as f32)
}

/// タイルの番号として正しいか（1001〜2000）。
pub fn is_tile(tile: u16) -> bool {
    (FIRST_TILE..=LAST_TILE).contains(&tile)
}

/// セットのシートの並び（タイルの番号の並びと列の数とセルの大きさ）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UdimLayout {
    tiles: Vec<u16>,
    columns: u32,
    cell_width: u32,
    cell_height: u32,
}

impl UdimLayout {
    /// 並びを確かめて作る: タイルは 1〜[`MAX_TILES`] 個の狭義の昇順の正しい番号、列は 1〜タイルの数、セルの辺は 1 以上。シートの辺の
    /// 上限は呼び手（文書の辺の上限）が確かめる。
    pub fn new(
        tiles: Vec<u16>,
        columns: u32,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<UdimLayout, CoreError> {
        let ordered = tiles.windows(2).all(|w| w[0] < w[1]);
        if tiles.is_empty()
            || tiles.len() > MAX_TILES
            || !ordered
            || !tiles.iter().all(|t| is_tile(*t))
        {
            return Err(CoreError::InvalidArgument(Reason::UdimTiles));
        }
        if columns == 0 || columns as usize > tiles.len() || cell_width == 0 || cell_height == 0 {
            return Err(CoreError::InvalidArgument(Reason::UdimLayout));
        }
        Ok(UdimLayout {
            tiles,
            columns,
            cell_width,
            cell_height,
        })
    }

    /// 決まりの並び（列の数 = ceil(√n)）。タイルは並べ替え・重なりを除いてから並べる。
    pub fn packed(
        mut tiles: Vec<u16>,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<UdimLayout, CoreError> {
        tiles.sort_unstable();
        tiles.dedup();
        let columns = packed_columns(tiles.len());
        UdimLayout::new(tiles, columns, cell_width, cell_height)
    }

    pub fn tiles(&self) -> &[u16] {
        &self.tiles
    }
    pub fn columns(&self) -> u32 {
        self.columns
    }
    pub fn rows(&self) -> u32 {
        (self.tiles.len() as u32).div_ceil(self.columns)
    }
    /// セル（タイル 1 つ）の大きさ（画素）。
    pub fn cell_size(&self) -> (u32, u32) {
        (self.cell_width, self.cell_height)
    }
    /// シートの大きさ（画素。u64 で、上限を超えるかは呼び手が見る）。
    pub fn sheet_size(&self) -> (u64, u64) {
        (
            u64::from(self.columns) * u64::from(self.cell_width),
            u64::from(self.rows()) * u64::from(self.cell_height),
        )
    }
    /// 格子のセルの数（空のセルを含む）。
    pub fn cell_count(&self) -> usize {
        (self.columns * self.rows()) as usize
    }
    /// セル i（0 からの番号。下の行から左から右）の矩形。
    pub fn cell_rect(&self, index: usize) -> Rect {
        let (c, r) = (index as u32 % self.columns, index as u32 / self.columns);
        Rect::new(
            c * self.cell_width,
            r * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }
    /// タイルのセルの番号（表に無ければ None）。
    pub fn cell_of_tile(&self, tile: u16) -> Option<usize> {
        self.tiles.binary_search(&tile).ok()
    }
    /// セル i のタイル（空のセルは None）。
    pub fn tile_of_cell(&self, index: usize) -> Option<u16> {
        self.tiles.get(index).copied()
    }
    /// 画素 (x, y) のセルの番号（シートの外は None。空のセルも番号を返す）。
    pub fn cell_at(&self, x: u32, y: u32) -> Option<usize> {
        let (c, r) = (x / self.cell_width, y / self.cell_height);
        (c < self.columns && r < self.rows()).then_some((r * self.columns + c) as usize)
    }
    /// 同じタイルの並びで、セルの大きさだけ替えたもの。
    pub fn with_cell_size(&self, width: u32, height: u32) -> Result<UdimLayout, CoreError> {
        UdimLayout::new(self.tiles.clone(), self.columns, width, height)
    }
    /// タイルを足した決まりの並び（今のタイルは消さない）。
    pub fn with_tiles(&self, more: impl IntoIterator<Item = u16>) -> Result<UdimLayout, CoreError> {
        let mut tiles = self.tiles.clone();
        tiles.extend(more);
        UdimLayout::packed(tiles, self.cell_width, self.cell_height)
    }
    /// UV の点（そのタイル `tile` のセルへ写す）のシートの UV。
    fn sheet_uv(&self, cell: usize, tile: u16, uv: Vec2) -> Vec2 {
        let (c, r) = (cell as u32 % self.columns, cell as u32 / self.columns);
        let local = uv - tile_origin(tile);
        Vec2::new(
            (c as f32 + local.x) / self.columns as f32,
            (r as f32 + local.y) / self.rows() as f32,
        )
    }
}

/// 決まりの列の数（ceil(√n)。n は 1 以上）。
pub fn packed_columns(n: usize) -> u32 {
    let mut c = 1u32;
    while (c as usize) * (c as usize) < n {
        c += 1;
    }
    c
}

/// セル（タイル）の格子の大きさ。近くの画素を読む効果・UV の外への塗り広げが、セルの外を読まない・セルの外へ出さないための区切り
/// （UDIM でない文書は、文書の大きさ 1 つのセル）。シートはセルの整数倍なので、画素 (x, y) のセルは (x / 幅, y / 高さ)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellGrid {
    pub width: u32,
    pub height: u32,
}

impl CellGrid {
    /// 画素 (x, y) を含むセルの矩形（画像 `size` の中へ切る）。
    pub fn cell_containing(&self, x: u32, y: u32, size: (u32, u32)) -> Rect {
        let (cx, cy) = (
            (x / self.width) * self.width,
            (y / self.height) * self.height,
        );
        Rect::new(
            cx,
            cy,
            self.width.min(size.0.saturating_sub(cx)),
            self.height.min(size.1.saturating_sub(cy)),
        )
    }
    /// 2 つの画素が同じセルか。
    #[inline]
    pub fn same_cell(&self, a: (u32, u32), b: (u32, u32)) -> bool {
        a.0 / self.width == b.0 / self.width && a.1 / self.height == b.1 / self.height
    }
    /// 矩形を、セルの境で分けた矩形の並び（下の行から左から右）。
    pub fn split(&self, r: Rect) -> Vec<Rect> {
        let mut out = Vec::new();
        if r.width == 0 || r.height == 0 {
            return out;
        }
        let (x1, y1) = (r.x + r.width, r.y + r.height);
        let mut y = r.y;
        while y < y1 {
            let ye = ((y / self.height + 1) * self.height).min(y1);
            let mut x = r.x;
            while x < x1 {
                let xe = ((x / self.width + 1) * self.width).min(x1);
                out.push(Rect::new(x, y, xe - x, ye - y));
                x = xe;
            }
            y = ye;
        }
        out
    }
    /// 歩幅 `stride` で縮めた画像のセル（粗い評価。端数は切り上げ）。
    pub fn coarse(&self, stride: u32) -> CellGrid {
        let s = stride.max(1);
        CellGrid {
            width: self.width.div_ceil(s).max(1),
            height: self.height.div_ceil(s).max(1),
        }
    }
}

/// 三角形の写しの結果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mapped {
    /// シートの UV（`cell` は入れたセルの番号、`straddles` はタイルをまたぐ三角形）。
    Cell {
        uv: [Vec2; 3],
        cell: usize,
        straddles: bool,
    },
    /// 表に無いタイル（UV の重心が範囲の外も含む）。シートの 0〜1 の外へ置いた UV。
    Outside([Vec2; 3]),
}

/// 三角形の UV（元の UV）をシートの UV へ写す。
pub fn map_triangle(layout: &UdimLayout, uv: [Vec2; 3]) -> Mapped {
    let center = (uv[0] + uv[1] + uv[2]) / 3.0;
    let outside = || Mapped::Outside(uv.map(|p| Vec2::new(p.x.fract() - 2.0, p.y.fract())));
    let Some(tile) = tile_of(center) else {
        return outside();
    };
    let Some(cell) = layout.cell_of_tile(tile) else {
        return outside();
    };
    let o = tile_origin(tile);
    let straddles = uv.iter().any(|p| {
        let l = *p - o;
        l.x < -STRADDLE_EPSILON
            || l.y < -STRADDLE_EPSILON
            || l.x > 1.0 + STRADDLE_EPSILON
            || l.y > 1.0 + STRADDLE_EPSILON
    });
    Mapped::Cell {
        uv: uv.map(|p| layout.sheet_uv(cell, tile, p)),
        cell,
        straddles,
    }
}

/// マテリアルの組 → シートの並び（UDIM のセットが受け持つマテリアルだけ。ほかのマテリアルの三角形は写さない）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UdimMapping {
    by_material: Vec<(i32, Arc<UdimLayout>)>,
}

impl UdimMapping {
    pub fn new() -> UdimMapping {
        UdimMapping::default()
    }
    /// マテリアルの組 `material` を `layout` で写す（同じ組を 2 度渡せば後のもの）。
    pub fn insert(&mut self, material: i32, layout: Arc<UdimLayout>) {
        self.by_material.retain(|(m, _)| *m != material);
        self.by_material.push((material, layout));
        self.by_material.sort_by_key(|(m, _)| *m);
    }
    pub fn is_empty(&self) -> bool {
        self.by_material.is_empty()
    }
    /// 組の並び（写さない組は None）。
    pub fn layout(&self, material: i32) -> Option<&Arc<UdimLayout>> {
        self.by_material
            .iter()
            .find(|(m, _)| *m == material)
            .map(|(_, l)| l)
    }
    fn slot(&self, material: i32) -> Option<usize> {
        self.by_material.iter().position(|(m, _)| *m == material)
    }
}

/// 写した三角形の数え（呼び手が知らせる）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapCounts {
    /// 写した三角形。
    pub mapped: usize,
    /// そのうちタイルをまたぐ三角形（重心のタイルへ入れた）。
    pub straddling: usize,
    /// 表に無いタイルの三角形（描けない）。
    pub outside: usize,
}

impl MapCounts {
    fn add(&mut self, m: &Mapped) {
        match m {
            Mapped::Cell { straddles, .. } => {
                self.mapped += 1;
                self.straddling += usize::from(*straddles);
            }
            Mapped::Outside(_) => self.outside += 1,
        }
    }
}

/// 索引つきのメッシュの UV をシートへ写す（`mapping` に無いマテリアルのサブメッシュは今のまま）。違うセル（か写さない側）の三角形が頂点を
/// 共有していたら頂点を複製し、複製は頂点の並びの後ろへ足す（元の頂点の番号はそのまま）。UV の無いメッシュは写さない。三角形の並び・数は変えない。
pub fn remap_meshes(meshes: &[ModelMesh], mapping: &UdimMapping) -> (Vec<ModelMesh>, MapCounts) {
    let mut counts = MapCounts::default();
    let out = meshes
        .iter()
        .map(|m| remap_mesh(m, mapping, &mut counts))
        .collect();
    (out, counts)
}

/// 頂点の行き先の鍵: 写さない（u32::MAX, u32::MAX）か、並びの番号とセル（表に無いタイルは u32::MAX）。
type VertexKey = (u32, u32, u32);

fn remap_mesh(m: &ModelMesh, mapping: &UdimMapping, counts: &mut MapCounts) -> ModelMesh {
    let mut out = m.clone();
    if m.uvs.len() != m.positions.len()
        || !m
            .submeshes
            .iter()
            .any(|s| mapping.layout(s.material).is_some())
    {
        return out;
    }
    let n = m.positions.len();
    // 頂点ごとに、最初に使った鍵（元の番号を使う）と、ほかの鍵の複製の番号
    let mut first: Vec<Option<(u32, u32)>> = vec![None; n];
    let mut extra: HashMap<VertexKey, u32> = HashMap::new();
    let mut uvs = m.uvs.clone();
    let mut positions = m.positions.clone();
    let mut normals = m.normals.clone();
    for (si, s) in m.submeshes.iter().enumerate() {
        let layout = mapping.layout(s.material).cloned();
        let slot = mapping.slot(s.material).map_or(u32::MAX, |k| k as u32);
        let mut indices = s.indices.clone();
        for t in indices.chunks_exact_mut(3) {
            if t.iter().any(|&i| i as usize >= n) {
                continue;
            }
            let (key, new_uv) = match &layout {
                None => ((u32::MAX, u32::MAX), None),
                Some(l) => {
                    let mapped = map_triangle(l, [0, 1, 2].map(|k| m.uvs[t[k] as usize]));
                    counts.add(&mapped);
                    match mapped {
                        Mapped::Cell { uv, cell, .. } => ((slot, cell as u32), Some(uv)),
                        Mapped::Outside(uv) => ((slot, u32::MAX), Some(uv)),
                    }
                }
            };
            for k in 0..3 {
                let v = t[k] as usize;
                let target = match first[v] {
                    None => {
                        first[v] = Some(key);
                        v as u32
                    }
                    Some(f) if f == key => v as u32,
                    Some(_) => *extra.entry((v as u32, key.0, key.1)).or_insert_with(|| {
                        positions.push(m.positions[v]);
                        if !m.normals.is_empty() {
                            normals.push(m.normals[v]);
                        }
                        uvs.push(m.uvs[v]);
                        (positions.len() - 1) as u32
                    }),
                };
                if let Some(uv) = new_uv {
                    uvs[target as usize] = uv[k];
                }
                t[k] = target;
            }
        }
        out.submeshes[si].indices = indices;
    }
    out.positions = positions;
    out.normals = normals;
    out.uvs = uvs;
    out
}

/// 当たりの三角形（索引の無いスープ）の UV をその場で写す（`mapping` に無いマテリアルの三角形は今のまま）。
pub fn remap_triangles(triangles: &mut [SurfaceTriangle], mapping: &UdimMapping) -> MapCounts {
    let mut counts = MapCounts::default();
    for t in triangles.iter_mut() {
        let Some(layout) = mapping.layout(t.material) else {
            continue;
        };
        let mapped = map_triangle(layout, [t.uv_a, t.uv_b, t.uv_c]);
        counts.add(&mapped);
        let uv = match mapped {
            Mapped::Cell { uv, .. } | Mapped::Outside(uv) => uv,
        };
        t.uv_a = uv[0];
        t.uv_b = uv[1];
        t.uv_c = uv[2];
    }
    counts
}

/// マテリアルの組 `materials` の三角形が使っているタイル（UV の重心のタイル。範囲の外は数えない）。昇順・重なり無し。
pub fn used_tiles<'a>(
    triangles: impl IntoIterator<Item = &'a SurfaceTriangle>,
    materials: impl Fn(i32) -> bool,
) -> Vec<u16> {
    let mut seen = [false; (LAST_TILE - FIRST_TILE + 1) as usize];
    for t in triangles {
        if !materials(t.material) {
            continue;
        }
        if let Some(tile) = tile_of((t.uv_a + t.uv_b + t.uv_c) / 3.0) {
            seen[(tile - FIRST_TILE) as usize] = true;
        }
    }
    seen.iter()
        .enumerate()
        .filter(|(_, s)| **s)
        .map(|(i, _)| FIRST_TILE + i as u16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Submesh;
    use glam::Vec3;

    #[test]
    fn tile_numbers_follow_the_udim_rule() {
        assert_eq!(tile_of(Vec2::new(0.5, 0.5)), Some(1001));
        assert_eq!(tile_of(Vec2::new(1.5, 0.2)), Some(1002));
        assert_eq!(tile_of(Vec2::new(0.5, 1.5)), Some(1011));
        assert_eq!(tile_of(Vec2::new(9.9, 99.9)), Some(2000));
        assert_eq!(tile_of(Vec2::new(10.1, 0.5)), None);
        assert_eq!(tile_of(Vec2::new(-0.1, 0.5)), None);
        assert_eq!(tile_origin(1012), Vec2::new(1.0, 1.0));
    }

    #[test]
    fn packed_layout_fills_rows_from_the_bottom() {
        let l = UdimLayout::packed(vec![1011, 1001, 1002, 1002], 64, 32).unwrap();
        assert_eq!(l.tiles(), &[1001, 1002, 1011]);
        assert_eq!((l.columns(), l.rows()), (2, 2));
        assert_eq!(l.sheet_size(), (128, 64));
        assert_eq!(l.cell_rect(0), Rect::new(0, 0, 64, 32));
        assert_eq!(l.cell_rect(2), Rect::new(0, 32, 64, 32));
        assert_eq!(l.tile_of_cell(3), None, "余ったセルは空");
        assert_eq!(l.cell_at(70, 40), Some(3));
        assert_eq!(packed_columns(6), 3);
        assert_eq!(packed_columns(1), 1);
        assert!(UdimLayout::new(vec![1002, 1001], 1, 8, 8).is_err());
        assert!(UdimLayout::new(vec![999], 1, 8, 8).is_err());
        assert!(UdimLayout::new(vec![1001], 2, 8, 8).is_err());
    }

    #[test]
    fn triangles_go_to_the_cell_of_their_centroid() {
        let l = UdimLayout::packed(vec![1001, 1002, 1011], 64, 64).unwrap();
        let tri = |o: Vec2| {
            [
                o + Vec2::new(0.1, 0.1),
                o + Vec2::new(0.9, 0.1),
                o + Vec2::new(0.1, 0.9),
            ]
        };
        match map_triangle(&l, tri(Vec2::new(1.0, 0.0))) {
            Mapped::Cell { uv, straddles, .. } => {
                assert!(!straddles);
                assert!((uv[0] - Vec2::new(0.55, 0.05)).length() < 1e-6, "{uv:?}");
            }
            m => panic!("{m:?}"),
        }
        match map_triangle(&l, tri(Vec2::new(0.0, 1.0))) {
            Mapped::Cell { uv, .. } => assert!((uv[0] - Vec2::new(0.05, 0.55)).length() < 1e-6),
            m => panic!("{m:?}"),
        }
        // またぐ三角形は重心のタイルへ
        let across = [
            Vec2::new(0.8, 0.2),
            Vec2::new(1.4, 0.2),
            Vec2::new(0.8, 0.5),
        ];
        assert!(matches!(
            map_triangle(&l, across),
            Mapped::Cell {
                straddles: true,
                ..
            }
        ));
        // 表に無いタイルは 0〜1 の外
        match map_triangle(&l, tri(Vec2::new(5.0, 0.0))) {
            Mapped::Outside(uv) => assert!(uv.iter().all(|p| p.x < 0.0)),
            m => panic!("{m:?}"),
        }
    }

    #[test]
    fn shared_vertices_across_cells_are_duplicated() {
        // 2 つの三角形が頂点 1・2 を共有し、片方は 1001、もう片方は 1002 の重心
        let mesh = ModelMesh {
            name: "板".into(),
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::ONE],
            normals: Vec::new(),
            uvs: vec![
                Vec2::new(0.2, 0.2),
                Vec2::new(1.0, 0.2),
                Vec2::new(1.0, 0.8),
                Vec2::new(1.8, 0.5),
            ],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 1, 2, 1, 3, 2],
            }],
        };
        let mut mapping = UdimMapping::new();
        mapping.insert(
            0,
            Arc::new(UdimLayout::packed(vec![1001, 1002], 32, 32).unwrap()),
        );
        let (out, counts) = remap_meshes(std::slice::from_ref(&mesh), &mapping);
        assert_eq!(counts.mapped, 2);
        assert_eq!(counts.straddling, 0);
        let m = &out[0];
        assert_eq!(m.positions.len(), 6, "共有の 2 頂点を複製");
        let idx = &m.submeshes[0].indices;
        assert_eq!(&idx[..3], &[0, 1, 2]);
        assert_eq!(&idx[3..], &[4, 3, 5]);
        // 1001 側の頂点 1 は右の端（シートの u = 0.5）、1002 側の複製は左の端（u = 0.5 でもセル 1 の原点）
        assert!((m.uvs[1].x - 0.5).abs() < 1e-6);
        assert!((m.uvs[4].x - 0.5).abs() < 1e-6);
        assert!((m.uvs[3].x - 0.9).abs() < 1e-6);
        // 写さないマテリアルはそのまま
        let (same, c) = remap_meshes(std::slice::from_ref(&mesh), &UdimMapping::new());
        assert_eq!(same[0], mesh);
        assert_eq!(c, MapCounts::default());
    }

    #[test]
    fn cell_grid_splits_at_borders() {
        let g = CellGrid {
            width: 10,
            height: 10,
        };
        let parts = g.split(Rect::new(5, 8, 10, 4));
        assert_eq!(
            parts,
            vec![
                Rect::new(5, 8, 5, 2),
                Rect::new(10, 8, 5, 2),
                Rect::new(5, 10, 5, 2),
                Rect::new(10, 10, 5, 2)
            ]
        );
        assert_eq!(g.cell_containing(12, 3, (15, 15)), Rect::new(10, 0, 5, 10));
        assert!(g.same_cell((9, 9), (0, 0)));
        assert!(!g.same_cell((9, 9), (10, 9)));
    }
}
