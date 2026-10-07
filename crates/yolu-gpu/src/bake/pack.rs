//! `MeshBakeScene`（core が準備した平らな入力）を、シェーダーの読む配列とパラメーターに詰める。
//! 浮動小数と整数が混ざる配列は u32 で持ち（`bake.wgsl` の `bf` で読む）、純粋な浮動小数の配列だけ f32。
use super::GpuBakeError;
use bytemuck::{Pod, Zeroable};
use yolu_core::mesh_maps::{FlatBvh, FlatCurvature, MeshBakeScene, SCENE_NONE};

pub(crate) const NONE: u32 = SCENE_NONE;
pub(crate) const LU: usize = 16;
pub(crate) const HF: usize = 21;
pub(crate) const TILE: usize = 8;

pub(crate) const F_NORMALS: u32 = 1;
pub(crate) const F_PROJ: u32 = 2;
pub(crate) const F_BYNAME: u32 = 4;
pub(crate) const F_FRAMES: u32 = 8;
pub(crate) const F_IDS: u32 = 16;
pub(crate) const F_UVISLAND: u32 = 32;
pub(crate) const F_CURV: u32 = 64;
pub(crate) const F_RAYS: u32 = 128;
pub(crate) const F_AO: u32 = 256;
pub(crate) const F_TH: u32 = 512;
pub(crate) const F_ANYHIT: u32 = 1024;
pub(crate) const F_BACK: u32 = 2048;
pub(crate) const F_HI_NORMALS: u32 = 4096;
/// ray query の道だけが見る旗: 加速構造の頂点の並び（辺 1 と辺 2）を入れ替えたので、重心座標の u と v を戻す。
pub(crate) const F_RQ_FLIP: u32 = 8192;

/// `bake.wgsl` の `Params` と同じ並び（先頭の配列は 16 バイト境界）。
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Params {
    pub map_offsets: [[u32; 4]; 3],
    pub cvf: [[f32; 4]; 4],
    pub cvu: [[u32; 4]; 2],
    pub width: u32,
    pub height: u32,
    pub aa: u32,
    pub band_first: u32,
    pub band_texels: u32,
    pub local_start: u32,
    pub count: u32,
    pub out_stride: u32,
    pub flags: u32,
    pub tiles_x: u32,
    pub tile_off_off: u32,
    pub tile_tri_off: u32,
    pub bvh_table_off: u32,
    pub low_bvh: u32,
    pub high_bvh: u32,
    pub ao_count: u32,
    pub th_count: u32,
    pub ao_dirs_off: u32,
    pub th_dirs_off: u32,
    pub lo_f_stride: u32,
    pub lo_cage_off: u32,
    pub lo_frame_off: u32,
    pub pad0: u32,
    pub pad1: u32,
    pub ao_cos2: f32,
    pub th_cos2: f32,
    pub ray_offset: f32,
    pub ao_max: f32,
    pub th_max: f32,
    pub frontal: f32,
    pub rear: f32,
    pub range: f32,
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub pad2: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale_z: f32,
    pub pad3: f32,
}

pub(crate) struct Packed {
    pub lo_f: Vec<f32>,
    pub lo_u: Vec<u32>,
    pub hi_f: Vec<f32>,
    pub hi_u: Vec<u32>,
    pub idx: Vec<u32>,
    pub bvh_nodes: Vec<u32>,
    pub bvh_tris: Vec<u32>,
    pub misc: Vec<u32>,
    pub params: Params,
    /// レイを飛ばすテクセルあたりの本数の上限の見積もり（サンプル × (AO + 厚み + 投影)）。
    pub rays_per_texel: u64,
    /// 三角形の BVH の数（低ポリ・高ポリ・名前ごと）。
    #[allow(dead_code)]
    pub bvh_count: usize,
    /// 名前ごとの BVH（名前の対応で投影する）があるか。
    pub has_groups: bool,
}
impl Packed {
    /// シェーダーに渡す配列の大きさ（バイト）。空の配列はダミーの 16 バイトで数える。
    pub fn bytes(&self) -> u64 {
        [
            self.lo_f.len(),
            self.lo_u.len(),
            self.hi_f.len(),
            self.hi_u.len(),
            self.idx.len(),
            self.bvh_nodes.len(),
            self.bvh_tris.len(),
            self.misc.len(),
        ]
        .iter()
        .map(|n| (*n as u64 * 4).max(16))
        .sum::<u64>()
            + std::mem::size_of::<Params>() as u64
    }
    /// 配列ごとの大きさの最大（バイト）。1 つの束縛の上限と比べる。
    pub fn largest_binding(&self) -> u64 {
        [
            self.lo_f.len(),
            self.lo_u.len(),
            self.hi_f.len(),
            self.hi_u.len(),
            self.idx.len(),
            self.bvh_nodes.len(),
            self.bvh_tris.len(),
            self.misc.len(),
        ]
        .iter()
        .map(|n| *n as u64 * 4)
        .max()
        .unwrap_or(0)
    }
}

/// `bake.wgsl` の `hash32` と同じ整数のハッシュ（曲率のセルの表の位置）。
pub(crate) fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}
fn cell_slot(x: u32, y: u32, z: u32) -> u32 {
    hash32(x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ z.wrapping_mul(83_492_791))
}

fn too_big(what: &str) -> GpuBakeError {
    GpuBakeError::Budget(format!("{what}が GPU の添字の範囲を超えます"))
}
fn word_count(v: usize, what: &str) -> Result<u32, GpuBakeError> {
    u32::try_from(v).map_err(|_| too_big(what))
}

/// UV の 8×8 テクセルのタイルごとに、覆う三角形（受け手の番号、昇順）の一覧を作る。
/// 受け手の番号の昇順に並べるのは、同じテクセルを覆う三角形のうち受け手の並び（core が決めた優先の順。既定は元の番号の昇順）で
/// 先のものが所有するという core の規則のため。
fn bin_tiles(
    scene: &MeshBakeScene,
    max_entries: usize,
) -> Result<(Vec<u32>, Vec<u32>, u32), GpuBakeError> {
    let tiles_x = (scene.width as usize).div_ceil(TILE);
    let tiles_y = (scene.height as usize).div_ceil(TILE);
    let n = scene.receivers.len();
    let range = |r: usize| {
        let b = &scene.raster_bounds[r * 4..r * 4 + 4];
        (b[1] >= b[0] && b[3] >= b[2]).then(|| {
            (
                b[0] as usize / TILE,
                b[1] as usize / TILE,
                b[2] as usize / TILE,
                b[3] as usize / TILE,
            )
        })
    };
    let mut offsets = vec![0u32; tiles_x * tiles_y + 1];
    let mut entries = 0usize;
    for r in 0..n {
        if let Some((tx0, tx1, ty0, ty1)) = range(r) {
            entries += (tx1 - tx0 + 1) * (ty1 - ty0 + 1);
            if entries > max_entries {
                return Err(GpuBakeError::Budget(
                    "UV のタイルごとの三角形の一覧が GPU の予算を超えます".into(),
                ));
            }
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    offsets[ty * tiles_x + tx + 1] += 1;
                }
            }
        }
    }
    for i in 1..offsets.len() {
        offsets[i] += offsets[i - 1];
    }
    let mut cursor = offsets.clone();
    let mut list = vec![0u32; entries];
    for r in 0..n {
        if let Some((tx0, tx1, ty0, ty1)) = range(r) {
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    let tile = ty * tiles_x + tx;
                    list[cursor[tile] as usize] = r as u32;
                    cursor[tile] += 1;
                }
            }
        }
    }
    Ok((offsets, list, tiles_x as u32))
}

fn push_bvh(
    flat: &FlatBvh,
    nodes: &mut Vec<u32>,
    tris: &mut Vec<u32>,
    table: &mut Vec<u32>,
) -> Result<(), GpuBakeError> {
    let node_base = word_count(nodes.len() / 8, "BVH の節")?;
    let tri_base = word_count(tris.len() / 12, "BVH の面")?;
    table.extend_from_slice(&[
        node_base,
        tri_base,
        flat.node_count() as u32,
        flat.triangle_count() as u32,
    ]);
    for n in 0..flat.node_count() {
        let b = &flat.bounds[n * 6..n * 6 + 6];
        nodes.extend([
            b[0].to_bits(),
            b[1].to_bits(),
            b[2].to_bits(),
            flat.first[n],
            b[3].to_bits(),
            b[4].to_bits(),
            b[5].to_bits(),
            flat.count[n],
        ]);
    }
    for t in 0..flat.triangle_count() {
        let v = &flat.tris[t * 10..t * 10 + 10];
        tris.extend([
            v[0].to_bits(),
            v[1].to_bits(),
            v[2].to_bits(),
            flat.original[t],
            v[3].to_bits(),
            v[4].to_bits(),
            v[5].to_bits(),
            v[9].to_bits(),
            v[6].to_bits(),
            v[7].to_bits(),
            v[8].to_bits(),
            0,
        ]);
    }
    Ok(())
}

/// 曲率のセルの開番地法の表（`bake.wgsl` の `cell_lookup` が引く）。1 つの項目は [x, y, z, 線分の先頭, 本数]、空は x = NONE。
fn cell_table(curvature: &FlatCurvature) -> Vec<u32> {
    let capacity = (curvature.cells.len() * 2).max(2).next_power_of_two();
    let mut table = vec![NONE; capacity * 5];
    let mask = capacity as u32 - 1;
    for c in &curvature.cells {
        let mut slot = cell_slot(c[0], c[1], c[2]) & mask;
        while table[slot as usize * 5] != NONE {
            slot = (slot + 1) & mask;
        }
        table[slot as usize * 5..slot as usize * 5 + 5].copy_from_slice(c);
    }
    table
}

fn push_segments(curvature: &FlatCurvature, misc: &mut Vec<u32>) {
    for (s, comp) in curvature
        .segments
        .as_chunks::<7>()
        .0
        .iter()
        .zip(&curvature.segment_component)
    {
        misc.extend(s.iter().map(|v| v.to_bits()));
        misc.push(*comp);
    }
}

impl Packed {
    /// `budget_bytes` は、タイルごとの一覧が使ってよい上限の目安（全体の予算の判断は呼び手）。
    pub fn new(scene: &MeshBakeScene, budget_bytes: u64) -> Result<Self, GpuBakeError> {
        let n = scene.receivers.len();
        let projection = scene.projection.as_ref();
        let has_proj = projection.is_some();
        let has_frames = scene.frames.is_some();
        let lo_cage_off = 27usize;
        let lo_frame_off = lo_cage_off + if has_proj { 9 } else { 0 };
        let lo_stride = lo_frame_off + if has_frames { 27 } else { 0 };
        word_count(n * lo_stride.max(LU), "受け手の配列")?;

        let mut lo_f = Vec::with_capacity(n * lo_stride);
        let mut lo_u = Vec::with_capacity(n * LU);
        for r in 0..n {
            lo_f.extend_from_slice(&scene.corners[r * 9..r * 9 + 9]);
            match &scene.normals {
                Some(v) => lo_f.extend_from_slice(&v[r * 9..r * 9 + 9]),
                None => lo_f.extend([0.0; 9]),
            }
            lo_f.extend_from_slice(&scene.face[r * 3..r * 3 + 3]);
            let a = &scene.raster_affine[r * 6..r * 6 + 6];
            let (ax, ay) = (a[0].floor(), a[1].floor());
            lo_f.extend([
                (a[0] - ax) as f32,
                (a[1] - ay) as f32,
                a[2] as f32,
                a[3] as f32,
                a[4] as f32,
                a[5] as f32,
            ]);
            if let Some(p) = projection {
                lo_f.extend_from_slice(&p.cage[r * 9..r * 9 + 9]);
            }
            if let Some(f) = &scene.frames {
                lo_f.extend_from_slice(&f[r * 27..r * 27 + 27]);
            }
            let ids = scene.ids.as_ref();
            let b = &scene.raster_bounds[r * 4..r * 4 + 4];
            lo_u.extend([
                scene.receivers[r],
                ids.map_or(0, |i| i.low[r]),
                ids.map_or(NONE, |i| i.manual[r]),
                projection.map_or(NONE, |p| p.target[r]),
                scene.low_component.get(r).copied().unwrap_or(0),
                0,
                0,
                0,
                ax as i32 as u32,
                ay as i32 as u32,
                b[0] as u32,
                b[1] as u32,
                b[2] as u32,
                b[3] as u32,
                0,
                0,
            ]);
        }

        let (mut hi_f, mut hi_u) = (vec![], vec![]);
        if let Some(p) = projection {
            let m = p.high_corners.len() / 9;
            word_count(m * HF, "高ポリの配列")?;
            hi_f.reserve(m * HF);
            for t in 0..m {
                hi_f.extend_from_slice(&p.high_corners[t * 9..t * 9 + 9]);
                match p.high_normals {
                    Some(v) => hi_f.extend_from_slice(&v[t * 9..t * 9 + 9]),
                    None => hi_f.extend([0.0; 9]),
                }
                hi_f.extend_from_slice(&p.high_face[t * 3..t * 3 + 3]);
                hi_u.push(
                    scene
                        .ids
                        .as_ref()
                        .and_then(|i| i.high.get(t))
                        .copied()
                        .unwrap_or(0),
                );
                hi_u.push(
                    p.curvature
                        .as_ref()
                        .and_then(|c| c.components.get(t))
                        .copied()
                        .unwrap_or(0),
                );
            }
        }

        // 配列の並び: [タイルの先頭 | タイルの三角形 | BVH の表 | 曲率のセルの表 0 | 曲率のセルの表 1]
        let max_entries = (budget_bytes / 16).min(u32::MAX as u64 / 2) as usize;
        let (tile_offsets, tile_list, tiles_x) = bin_tiles(scene, max_entries)?;
        let mut idx = tile_offsets;
        let tile_off_off = 0u32;
        let tile_tri_off = word_count(idx.len(), "タイルの一覧")?;
        idx.extend_from_slice(&tile_list);
        drop(tile_list);

        let (mut bvh_nodes, mut bvh_tris, mut table) = (vec![], vec![], vec![]);
        let mut bvh_count = 0usize;
        let mut low_bvh = NONE;
        if let Some(b) = &scene.low_bvh {
            low_bvh = bvh_count as u32;
            push_bvh(b, &mut bvh_nodes, &mut bvh_tris, &mut table)?;
            bvh_count += 1;
        }
        let mut high_bvh = NONE;
        if let Some(p) = projection {
            high_bvh = bvh_count as u32;
            push_bvh(&p.bvh, &mut bvh_nodes, &mut bvh_tris, &mut table)?;
            bvh_count += 1;
            for g in &p.groups {
                push_bvh(g, &mut bvh_nodes, &mut bvh_tris, &mut table)?;
                bvh_count += 1;
            }
        }
        let bvh_table_off = word_count(idx.len(), "BVH の表")?;
        idx.extend_from_slice(&table);

        let mut params = Params::zeroed();
        let mut misc = vec![];
        let curvatures = [
            scene.curvature.as_ref(),
            projection.and_then(|p| p.curvature.as_ref()),
        ];
        for (ci, c) in curvatures.iter().enumerate() {
            let Some(c) = c else { continue };
            let seg_base = word_count(misc.len() / 8, "曲率の線分")?;
            push_segments(c, &mut misc);
            let cells = cell_table(c);
            let cell_off = word_count(idx.len(), "曲率のセル")?;
            idx.extend_from_slice(&cells);
            params.cvf[ci * 2] = [c.radius as f32, c.cell as f32, c.norm as f32, 0.0];
            params.cvf[ci * 2 + 1] = [c.min[0] as f32, c.min[1] as f32, c.min[2] as f32, 0.0];
            params.cvu[ci] = [
                seg_base,
                cell_off,
                (cells.len() / 5) as u32 - 1,
                u32::from(!c.cells.is_empty()),
            ];
        }
        let ao_dirs_off = word_count(misc.len(), "レイの方向")?;
        for d in &scene.rays.ao {
            misc.extend([
                (d[0] as f32).to_bits(),
                (d[1] as f32).to_bits(),
                (d[2] as f32).to_bits(),
                0,
            ]);
        }
        let th_dirs_off = word_count(misc.len(), "レイの方向")?;
        for d in &scene.rays.thickness {
            misc.extend([
                (d[0] as f32).to_bits(),
                (d[1] as f32).to_bits(),
                (d[2] as f32).to_bits(),
                0,
            ]);
        }
        word_count(idx.len(), "添字の配列")?;
        word_count(misc.len(), "補助の配列")?;
        word_count(bvh_nodes.len(), "BVH の節の配列")?;
        word_count(bvh_tris.len(), "BVH の面の配列")?;

        let mut out_stride = 0u32;
        let mut offsets = [NONE; 12];
        for k in &scene.maps {
            offsets[*k as usize] = out_stride;
            out_stride += k.channels() as u32;
        }
        for (i, o) in offsets.iter().enumerate() {
            params.map_offsets[i / 4][i % 4] = *o;
        }
        let mut flags = 0;
        for (on, bit) in [
            (scene.normals.is_some(), F_NORMALS),
            (has_proj, F_PROJ),
            (projection.is_some_and(|p| p.by_name), F_BYNAME),
            (has_frames, F_FRAMES),
            (scene.ids.is_some(), F_IDS),
            (scene.ids.as_ref().is_some_and(|i| i.uv_island), F_UVISLAND),
            (scene.curvature.is_some(), F_CURV),
            (scene.low_bvh.is_some(), F_RAYS),
            (scene.rays.want_ao, F_AO),
            (scene.rays.want_thickness, F_TH),
            (scene.rays.any_hit, F_ANYHIT),
            (scene.rays.ignore_backfaces, F_BACK),
            (
                projection.is_some_and(|p| p.high_normals.is_some()),
                F_HI_NORMALS,
            ),
        ] {
            if on {
                flags |= bit;
            }
        }
        params.width = scene.width;
        params.height = scene.height;
        params.aa = scene.antialiasing;
        params.out_stride = out_stride;
        params.flags = flags;
        params.tiles_x = tiles_x;
        params.tile_off_off = tile_off_off;
        params.tile_tri_off = tile_tri_off;
        params.bvh_table_off = bvh_table_off;
        params.low_bvh = low_bvh;
        params.high_bvh = high_bvh;
        params.ao_count = scene.rays.ao.len() as u32;
        params.th_count = scene.rays.thickness.len() as u32;
        params.ao_dirs_off = ao_dirs_off;
        params.th_dirs_off = th_dirs_off;
        params.lo_f_stride = lo_stride as u32;
        params.lo_cage_off = lo_cage_off as u32;
        params.lo_frame_off = lo_frame_off as u32;
        params.ao_cos2 = scene.rays.ao_cos2 as f32;
        params.th_cos2 = scene.rays.thickness_cos2 as f32;
        params.ray_offset = scene.rays.offset as f32;
        params.ao_max = scene.rays.ao_max as f32;
        params.th_max = scene.rays.thickness_max as f32;
        if let Some(p) = projection {
            params.frontal = p.frontal as f32;
            params.rear = p.rear as f32;
        }
        params.range = scene.height_range as f32;
        params.min_x = scene.min[0] as f32;
        params.min_y = scene.min[1] as f32;
        params.min_z = scene.min[2] as f32;
        params.scale_x = scene.scale[0] as f32;
        params.scale_y = scene.scale[1] as f32;
        params.scale_z = scene.scale[2] as f32;

        let samples = u64::from(scene.antialiasing) * u64::from(scene.antialiasing);
        let per_sample = u64::from(has_proj)
            + if scene.low_bvh.is_some() {
                u64::from(scene.rays.want_ao) * scene.rays.ao.len() as u64
                    + u64::from(scene.rays.want_thickness) * scene.rays.thickness.len() as u64
            } else {
                0
            };
        Ok(Self {
            lo_f,
            lo_u,
            hi_f,
            hi_u,
            idx,
            bvh_nodes,
            bvh_tris,
            misc,
            params,
            rays_per_texel: (samples * per_sample).max(1),
            bvh_count,
            has_groups: projection.is_some_and(|p| !p.groups.is_empty()),
        })
    }
}
