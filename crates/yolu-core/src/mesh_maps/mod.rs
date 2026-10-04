//! メッシュマップの16 bit 正本と由来。座標は左下原点、行優先。
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshMapError(pub String);
impl fmt::Display for MeshMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MeshMapError {}
pub type Result<T> = std::result::Result<T, MeshMapError>;
pub(crate) fn check(ok: bool, why: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(MeshMapError(why.into()))
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshMapKind {
    WorldNormal,
    Position,
    AmbientOcclusion,
    Curvature,
    Thickness,
    TangentNormal,
    Height,
    Id,
    BentNormal,
    Opacity,
}
impl MeshMapKind {
    pub const ALL: [Self; 10] = [
        Self::WorldNormal,
        Self::Position,
        Self::AmbientOcclusion,
        Self::Curvature,
        Self::Thickness,
        Self::TangentNormal,
        Self::Height,
        Self::Id,
        Self::BentNormal,
        Self::Opacity,
    ];
    pub fn channels(self) -> usize {
        match self {
            Self::WorldNormal
            | Self::Position
            | Self::TangentNormal
            | Self::Id
            | Self::BentNormal => 3,
            _ => 1,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::WorldNormal => "WorldNormal",
            Self::Position => "Position",
            Self::AmbientOcclusion => "AmbientOcclusion",
            Self::Curvature => "Curvature",
            Self::Thickness => "Thickness",
            Self::TangentNormal => "TangentNormal",
            Self::Height => "Height",
            Self::Id => "Id",
            Self::BentNormal => "BentNormal",
            Self::Opacity => "Opacity",
        }
    }
}
impl TryFrom<i32> for MeshMapKind {
    type Error = MeshMapError;
    fn try_from(value: i32) -> Result<Self> {
        Self::ALL
            .get(value as usize)
            .copied()
            .ok_or_else(|| MeshMapError("メッシュマップの種類が不明です".into()))
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshTexelCoverage {
    Empty,
    Covered,
    Overlap,
    Padding,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshMapProvenance {
    pub kind: MeshMapKind,
    pub engine_version: i32,
    pub mesh_hash: String,
    pub topology_hash: String,
    pub uv_channel: i32,
    pub width: i32,
    pub height: i32,
    pub target_slot: i32,
    pub target_slots: Vec<i32>,
    pub padding: i32,
    pub antialiasing: i32,
    pub settings_key: String,
    pub space: String,
    pub pose: String,
    pub source: String,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}
impl MeshMapProvenance {
    pub fn validate(&self) -> Result<()> {
        check(
            (1..=8192).contains(&self.width) && (1..=8192).contains(&self.height),
            "メッシュマップの大きさが範囲外です",
        )?;
        check(
            self.engine_version >= 1
                && (0..=7).contains(&self.uv_channel)
                && self.target_slot >= -1
                && (0..=64).contains(&self.padding)
                && (1..=4).contains(&self.antialiasing),
            "メッシュマップの由来が範囲外です",
        )?;
        check(
            self.target_slots.len() <= 65536
                && self.target_slots.iter().all(|s| *s >= 0)
                && self.target_slots.windows(2).all(|p| p[0] < p[1])
                && self.target_slots.first().copied().unwrap_or(-1) == self.target_slot,
            "スロットは昇順で重複せず先頭が対象スロットと一致する必要があります",
        )?;
        check(
            (0..3).all(|a| {
                self.bounds_min[a].is_finite()
                    && self.bounds_max[a].is_finite()
                    && self.bounds_min[a] <= self.bounds_max[a]
            }),
            "境界箱が不正です",
        )?;
        check(
            [
                &self.mesh_hash,
                &self.topology_hash,
                &self.settings_key,
                &self.space,
                &self.pose,
                &self.source,
            ]
            .iter()
            .all(|s| s.len() <= 4096),
            "メッシュマップの文字列が長すぎます",
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BakedMeshMap {
    provenance: MeshMapProvenance,
    data: Vec<u16>,
    coverage: std::sync::Arc<Vec<u8>>,
}
impl BakedMeshMap {
    pub fn new(provenance: MeshMapProvenance, data: Vec<u16>, coverage: Vec<u8>) -> Result<Self> {
        Self::with_coverage(provenance, data, std::sync::Arc::new(coverage))
    }
    pub(crate) fn with_coverage(
        provenance: MeshMapProvenance,
        data: Vec<u16>,
        coverage: std::sync::Arc<Vec<u8>>,
    ) -> Result<Self> {
        provenance.validate()?;
        let texels = provenance.width as usize * provenance.height as usize;
        check(
            coverage.len() == texels && data.len() == texels * provenance.kind.channels(),
            "正本の長さが大きさと一致しません",
        )?;
        check(coverage.iter().all(|b| *b <= 3), "未知のテクセルの由来です")?;
        Ok(Self {
            provenance,
            data,
            coverage,
        })
    }
    pub fn provenance(&self) -> &MeshMapProvenance {
        &self.provenance
    }
    pub fn data(&self) -> &[u16] {
        &self.data
    }
    pub fn coverage(&self) -> &[u8] {
        &self.coverage
    }
    pub fn kind(&self) -> MeshMapKind {
        self.provenance.kind
    }
    pub fn width(&self) -> usize {
        self.provenance.width as usize
    }
    pub fn height(&self) -> usize {
        self.provenance.height as usize
    }
    pub fn channels(&self) -> usize {
        self.kind().channels()
    }
    pub fn raw_value(&self, x: i32, y: i32, channel: usize) -> Result<u16> {
        check(channel < self.channels(), "チャンネルが範囲外です")?;
        Ok(
            if x < 0 || y < 0 || x as usize >= self.width() || y as usize >= self.height() {
                0
            } else {
                self.data[(y as usize * self.width() + x as usize) * self.channels() + channel]
            },
        )
    }
    pub fn value(&self, x: i32, y: i32, channel: usize) -> Result<f32> {
        Ok(self.raw_value(x, y, channel)? as f32 * (1.0f32 / 65535.0))
    }
    pub fn to_rgba8(&self, coverage_view: bool) -> Vec<u8> {
        let mut out = vec![0; self.coverage.len() * 4];
        for (i, p) in out.chunks_exact_mut(4).enumerate() {
            if self.coverage[i] == 0 {
                continue;
            }
            if coverage_view {
                p[..3].copy_from_slice(match self.coverage[i] {
                    1 => &[40, 170, 70],
                    2 => &[230, 40, 40],
                    _ => &[60, 100, 220],
                });
            } else {
                for (c, v) in p[..3].iter_mut().enumerate() {
                    *v = ((self.data[i * self.channels() + if self.channels() == 1 { 0 } else { c }]
                        as u32
                        * 255
                        + 32767)
                        / 65535) as u8;
                }
            }
            p[3] = 255;
        }
        out
    }
}

mod input;
pub use input::{MeshBakeAttributes, MeshBakeInput};
mod bvh;
pub use bvh::{MeshRayBvh, MeshRayHit};
mod settings;
pub use settings::*;
mod ids;
pub use ids::{
    id_palette, id_palette_levels, id_palette_separation, id_part_binding, mesh_regions,
    IdColorAssignments,
};
mod surface;
pub use surface::reconstruct_normals;
mod bake;
mod curvature;
mod raster;
mod rays;
pub use bake::{
    bake, base_name, condition_key, estimate_bytes, material_identity, ENGINE_VERSION,
    MAX_PARALLELISM,
};
mod freshness;
pub use freshness::{MeshMapCheck, MeshMapExpectation, MeshMapStaleReason, MeshMapState};
