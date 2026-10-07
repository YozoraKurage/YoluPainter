//! 文書の外から渡す効果の入力: 焼いたメッシュマップ・モデルのルートの位置・プロジェクトの画像。
//! 保存も Undo もしない（派生の入力）。渡し直すと、それを読むレイヤーの合成が作り直される。
//!
//! 使えないマップ（無い・古い・条件を照合できない・大きさが違う）は、使う段が入力をそのまま通して理由を出す。黒として読まない。

use std::collections::HashMap;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::ImageId;
use crate::brush::ImageColorSpace;
use crate::error::CoreError;
use crate::fill_image;
use crate::generator::{self, MapKind, MapState};
use crate::mesh_maps::BakedMeshMap;

/// generator の `MapKind` を保存の番号（メッシュマップの種類の並びと同じ）から。
pub(crate) fn map_kind_from_index(index: i32) -> Option<MapKind> {
    use MapKind::*;
    [
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
    ]
    .get(usize::try_from(index).ok()?)
    .copied()
}

/// 焼いたメッシュマップ 1 枚（16 bit の成分と被覆、Position の境界箱、焼いた条件の鍵、文書の今の条件に合っているか）。
/// 成分は色のマップ（World Normal・Position・Tangent Normal・ID・Bent Normal）が画素ごとに 3 つ、ほかは 1 つ。左下原点・行優先。
#[derive(Clone, Debug)]
pub struct MapInput {
    pub kind: MapKind,
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u16]>,
    /// 画素ごとの被覆（0 は何も無い。1 以上は値がある）。
    pub coverage: Arc<[u8]>,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    /// 焼いた条件の鍵（小文字 16 進 64 桁。Generator のピンと比べる）。
    pub condition_key: String,
    pub state: MapState,
}

impl MapInput {
    /// 長さと値を確かめて作る。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MapKind,
        width: u32,
        height: u32,
        data: Vec<u16>,
        coverage: Vec<u8>,
        bounds_min: [f64; 3],
        bounds_max: [f64; 3],
        condition_key: &str,
        state: MapState,
    ) -> Result<Self, CoreError> {
        let input = MapInput {
            kind,
            width,
            height,
            data: data.into(),
            coverage: coverage.into(),
            bounds_min,
            bounds_max,
            condition_key: condition_key.to_owned(),
            state,
        };
        input.validate()?;
        Ok(input)
    }

    /// 焼いたマップ（[`BakedMeshMap`]）から。条件の鍵は焼いたときの由来のもの。
    pub fn from_baked(map: &BakedMeshMap, state: MapState) -> Result<Self, CoreError> {
        let p = map.provenance();
        let kind = map_kind_from_index(p.kind as i32)
            .ok_or(CoreError::InvalidArgument("メッシュマップの種類"))?;
        Self::new(
            kind,
            u32::try_from(p.width).map_err(|_| CoreError::InvalidArgument("マップの幅"))?,
            u32::try_from(p.height).map_err(|_| CoreError::InvalidArgument("マップの高さ"))?,
            map.data().to_vec(),
            map.coverage().to_vec(),
            p.bounds_min,
            p.bounds_max,
            &p.condition_key(),
            state,
        )
    }

    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        let n = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || n != self.coverage.len() as u64
            || n * self.kind.channels() as u64 != self.data.len() as u64
        {
            return Err(CoreError::InvalidArgument("マップの大きさと長さが合わない"));
        }
        let key = &self.condition_key;
        if key.len() != 64
            || !key
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(CoreError::InvalidArgument("マップの条件の鍵"));
        }
        if (0..3).any(|i| {
            !self.bounds_min[i].is_finite()
                || !self.bounds_max[i].is_finite()
                || self.bounds_max[i] < self.bounds_min[i]
                || !(self.bounds_max[i] - self.bounds_min[i]).is_finite()
        }) {
            return Err(CoreError::InvalidArgument("マップの境界箱"));
        }
        Ok(())
    }

    pub(crate) fn as_generator(&self) -> generator::Map<'_> {
        generator::Map {
            kind: self.kind,
            width: self.width,
            height: self.height,
            data: &self.data,
            coverage: &self.coverage,
            bounds_min: self.bounds_min,
            bounds_max: self.bounds_max,
            condition_key: &self.condition_key,
            state: self.state,
        }
    }
    pub(crate) fn as_fill(&self) -> fill_image::Map<'_> {
        fill_image::Map {
            width: self.width,
            height: self.height,
            values: &self.data,
            coverage: &self.coverage,
        }
    }
}

/// モデルのルートの位置と向き（クォータニオン x, y, z, w）。形のグラデーション・塗りつぶしの投影が、焼いた Position を
/// モデルの空間へ戻すのに使う。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelFrame {
    pub position: [f64; 3],
    pub rotation: [f64; 4],
}

impl Default for ModelFrame {
    fn default() -> Self {
        ModelFrame {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl ModelFrame {
    pub fn new(position: [f64; 3], rotation: [f64; 4]) -> Result<Self, CoreError> {
        let len = rotation.iter().map(|v| v * v).sum::<f64>().sqrt();
        if position.iter().chain(&rotation).any(|v| !v.is_finite())
            || !len.is_finite()
            || len <= 1e-9
        {
            return Err(CoreError::InvalidArgument("モデルの位置・回転"));
        }
        Ok(ModelFrame { position, rotation })
    }
    pub(crate) fn for_generator(&self) -> Result<generator::ModelFrame, generator::Error> {
        generator::ModelFrame::new(self.position, self.rotation)
    }
    pub(crate) fn for_fill(&self) -> fill_image::ModelFrame {
        fill_image::ModelFrame {
            position: self.position,
            rotation: self.rotation,
        }
    }
}

/// プロジェクトの画像リソース 1 枚（straight RGBA8、左下原点、1〜8192）。
#[derive(Clone, Debug)]
pub struct ImageInput {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub color_space: ImageColorSpace,
    /// 中身の鍵（C# の `ImageContent.ComputeHash` と同じ: `YLPRGBA8`・幅・高さ（32 bit の little-endian）・画素の SHA-256）。
    /// 同じ画素は同じ鍵（ミップマップの共有に使う）。
    pub hash: String,
}

impl ImageInput {
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        color_space: ImageColorSpace,
    ) -> Result<Self, CoreError> {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || pixels.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err(CoreError::InvalidArgument("画像の大きさと画素の長さ"));
        }
        let mut sha = Sha256::new();
        sha.update(b"YLPRGBA8");
        sha.update(width.to_le_bytes());
        sha.update(height.to_le_bytes());
        sha.update(&pixels);
        let hash = sha
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Ok(ImageInput {
            width,
            height,
            pixels: pixels.into(),
            color_space,
            hash,
        })
    }
}

/// 文書の外から渡す入力の全部。`Default` はマップも画像も無く、モデルのルートは原点・回転なし。
#[derive(Clone, Debug)]
pub struct EffectInputs {
    pub(crate) maps: Vec<MapInput>,
    /// None はモデルのルートの位置が分からない（形のグラデーション・位置を読む投影は入力のまま通す）。
    pub(crate) frame: Option<ModelFrame>,
    pub(crate) images: HashMap<ImageId, ImageInput>,
    /// モデルのこのテクスチャセットの UV の位相（アイランド・継ぎ目の対応）。レイヤーのフィルターが UV の継ぎ目をまたぐのに使う（None はモデルが無い）。
    pub(crate) topology: Option<Arc<crate::geometry::UvTopology>>,
}

impl Default for EffectInputs {
    fn default() -> Self {
        EffectInputs {
            maps: Vec::new(),
            frame: Some(ModelFrame::default()),
            images: HashMap::new(),
            topology: None,
        }
    }
}

impl EffectInputs {
    pub fn new() -> Self {
        Self::default()
    }
    /// マップを足す（同じ種類は置き換える）。
    pub fn with_map(mut self, map: MapInput) -> Result<Self, CoreError> {
        map.validate()?;
        self.maps.retain(|m| m.kind != map.kind);
        self.maps.push(map);
        Ok(self)
    }
    /// モデルのルートの位置と向き（None は分からない）。
    pub fn with_frame(mut self, frame: Option<ModelFrame>) -> Self {
        self.frame = frame;
        self
    }
    pub fn with_image(mut self, id: ImageId, image: ImageInput) -> Self {
        self.images.insert(id, image);
        self
    }
    /// モデルのこのテクスチャセットの UV の位相（None はモデルが無い）。
    pub fn with_topology(mut self, topology: Option<Arc<crate::geometry::UvTopology>>) -> Self {
        self.topology = topology;
        self
    }
    /// モデルの UV の位相（アイランドの図・継ぎ目の対応）。
    pub fn topology(&self) -> Option<&Arc<crate::geometry::UvTopology>> {
        self.topology.as_ref()
    }
    /// 同じ UV の位相か（同じ物を共有している、または同じモデルの組・三角形の並び・UV・スロット・隣り合わせ。位置は見ない）。
    /// アイランドの図・帯の写しは UV だけで決まるので、位相の同じさにベイクの条件（余白など）・マップ・画像は関わらない。
    pub(crate) fn same_topology(&self, other: &EffectInputs) -> bool {
        match (&self.topology, &other.topology) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                Arc::ptr_eq(a, b)
                    || (a.material() == b.material()
                        && (Arc::ptr_eq(a.geometry(), b.geometry()) || a.same_layout(b.geometry())))
            }
            _ => false,
        }
    }
    /// プロジェクトの画像（ID ごと）。
    pub fn images(&self) -> &HashMap<ImageId, ImageInput> {
        &self.images
    }
    /// 位相以外の入力（マップ・モデルのルート・画像）が同じか（画素・画像は共有していれば中身を見ずに同じとする）。
    pub(crate) fn same_data(&self, other: &EffectInputs) -> bool {
        self.frame == other.frame
            && self.maps.len() == other.maps.len()
            && self.maps.iter().all(|m| {
                other.map(m.kind).is_some_and(|o| {
                    o.width == m.width
                        && o.height == m.height
                        && o.state == m.state
                        && o.condition_key == m.condition_key
                        && o.bounds_min == m.bounds_min
                        && o.bounds_max == m.bounds_max
                        && (Arc::ptr_eq(&o.data, &m.data) || o.data == m.data)
                        && (Arc::ptr_eq(&o.coverage, &m.coverage) || o.coverage == m.coverage)
                })
            })
            && self.images.len() == other.images.len()
            && self.images.iter().all(|(id, i)| {
                other
                    .images
                    .get(id)
                    .is_some_and(|o| o.hash == i.hash && o.color_space == i.color_space)
            })
    }
    /// 同じ入力か（渡し直しで合成を作り直すかの判断に使う）。
    pub fn same_as(&self, other: &EffectInputs) -> bool {
        self.same_topology(other) && self.same_data(other)
    }
    pub fn map(&self, kind: MapKind) -> Option<&MapInput> {
        self.maps.iter().find(|m| m.kind == kind)
    }
    pub fn image(&self, id: ImageId) -> Option<&ImageInput> {
        self.images.get(&id)
    }
}
