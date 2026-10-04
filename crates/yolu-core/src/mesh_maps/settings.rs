use super::{check, input::hash_text, MeshMapKind, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshIdSource {
    MaterialSlot,
    Mesh,
    VertexColor,
    UvIsland,
    MeshPart,
    MaterialAsset,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshOccluders {
    WholeModel,
    TargetSlotOnly,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshOcclusionFalloff {
    None,
    Linear,
}
#[derive(Clone, Debug)]
pub struct MeshBakeSettings {
    pub width: i32,
    pub height: i32,
    pub target_slot: i32,
    pub target_slots: Vec<i32>,
    pub padding: i32,
    pub maps: Vec<MeshMapKind>,
    pub antialiasing: i32,
    pub occluders: MeshOccluders,
    pub ao_samples: i32,
    pub ao_max_distance: f64,
    pub ao_spread_degrees: f64,
    pub ao_falloff: MeshOcclusionFalloff,
    pub ao_ignore_backfaces: bool,
    pub thickness_samples: i32,
    pub thickness_max_distance: f64,
    pub thickness_spread_degrees: f64,
    pub curvature_radius: f64,
    pub id_source: MeshIdSource,
    pub reference_frontal: f64,
    pub reference_rear: f64,
    pub reference_average_normals: bool,
    pub reference_match_by_name: bool,
    pub manual_id_colors: super::IdColorAssignments,
}
impl Default for MeshBakeSettings {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 1024,
            target_slot: 0,
            target_slots: vec![],
            padding: 16,
            maps: MeshMapKind::ALL[..5].to_vec(),
            antialiasing: 1,
            occluders: MeshOccluders::WholeModel,
            ao_samples: 64,
            ao_max_distance: 0.1,
            ao_spread_degrees: 180.,
            ao_falloff: MeshOcclusionFalloff::Linear,
            ao_ignore_backfaces: false,
            thickness_samples: 64,
            thickness_max_distance: 0.1,
            thickness_spread_degrees: 90.,
            curvature_radius: 0.02,
            id_source: MeshIdSource::MaterialSlot,
            reference_frontal: 0.01,
            reference_rear: 0.01,
            reference_average_normals: true,
            reference_match_by_name: false,
            manual_id_colors: super::IdColorAssignments::default(),
        }
    }
}
impl MeshBakeSettings {
    pub fn validate(&self) -> Result<()> {
        check(
            (1..=8192).contains(&self.width) && (1..=8192).contains(&self.height),
            "ベイクの大きさが範囲外です",
        )?;
        check(
            self.target_slot >= -1
                && self.target_slots.iter().all(|s| *s >= 0)
                && self.target_slots.windows(2).all(|s| s[0] < s[1])
                && self
                    .target_slots
                    .first()
                    .is_none_or(|s| *s == self.target_slot),
            "対象スロットが不正です",
        )?;
        check(
            (0..=64).contains(&self.padding) && (1..=4).contains(&self.antialiasing),
            "余白またはアンチエイリアスが範囲外です",
        )?;
        check(
            !self.maps.is_empty()
                && self
                    .maps
                    .iter()
                    .enumerate()
                    .all(|(i, k)| !self.maps[..i].contains(k)),
            "マップの種類が空または重複しています",
        )?;
        check(
            [self.ao_samples, self.thickness_samples]
                .iter()
                .all(|v| (1..=1024).contains(v)),
            "レイのサンプル数が範囲外です",
        )?;
        check(
            [
                self.ao_max_distance,
                self.thickness_max_distance,
                self.reference_frontal,
                self.reference_rear,
            ]
            .iter()
            .all(|v| *v > 0. && *v <= 4.),
            "距離は対角線比で0より大きく4以下が必要です",
        )?;
        check(
            [self.ao_spread_degrees, self.thickness_spread_degrees]
                .iter()
                .all(|v| (1. ..=180.).contains(v)),
            "レイの広がりは1〜180度が必要です",
        )?;
        check(
            (0.001..=0.5).contains(&self.curvature_radius),
            "曲率半径が範囲外です",
        )
    }
    pub fn targets(&self) -> Vec<i32> {
        if !self.target_slots.is_empty() {
            self.target_slots.clone()
        } else if self.target_slot >= 0 {
            vec![self.target_slot]
        } else {
            vec![]
        }
    }
    pub(crate) fn targeted(&self, slot: i32) -> bool {
        slot >= 0
            && (if !self.target_slots.is_empty() {
                self.target_slots.binary_search(&slot).is_ok()
            } else {
                self.target_slot < 0 || self.target_slot == slot
            })
    }
    pub fn includes(&self, k: MeshMapKind) -> bool {
        self.maps.contains(&k)
    }
    pub fn kind_key(&self, k: MeshMapKind, material_identity: &str) -> String {
        let back = if self.ao_ignore_backfaces {
            "ignore"
        } else {
            "occlude"
        };
        match k {
            MeshMapKind::WorldNormal => "source=vertex-normals".into(),
            MeshMapKind::Position => "normalize=bounding-box".into(),
            MeshMapKind::AmbientOcclusion => format!(
                "samples={};max={};spread={};falloff={:?};backfaces={back};occluders={:?}",
                self.ao_samples,
                roundtrip(self.ao_max_distance),
                roundtrip(self.ao_spread_degrees),
                self.ao_falloff,
                self.occluders
            ),
            MeshMapKind::Curvature => format!("radius={}", roundtrip(self.curvature_radius)),
            MeshMapKind::Thickness => format!(
                "samples={};max={};spread={};occluders={:?}",
                self.thickness_samples,
                roundtrip(self.thickness_max_distance),
                roundtrip(self.thickness_spread_degrees),
                self.occluders
            ),
            MeshMapKind::TangentNormal => "frame=unity-vertex-tangents;y=up".into(),
            MeshMapKind::Height => "normalize=max-ray-distance".into(),
            MeshMapKind::Opacity => "hit=reference".into(),
            MeshMapKind::BentNormal => format!(
                "samples={};max={};spread={};backfaces={back};occluders={:?}",
                self.ao_samples,
                roundtrip(self.ao_max_distance),
                roundtrip(self.ao_spread_degrees),
                self.occluders
            ),
            MeshMapKind::Id => format!(
                "source={:?};algorithm=2{}{}",
                self.id_source,
                if self.id_source == MeshIdSource::MaterialAsset {
                    format!(";materials={material_identity}")
                } else {
                    String::new()
                },
                if self.manual_id_colors.colors.is_empty() {
                    String::new()
                } else {
                    format!(";manual={}", self.manual_id_colors.key())
                }
            ),
        }
    }
    pub fn source_key(&self, reference_hash: Option<&str>) -> String {
        match reference_hash {
            Some(h) if !h.is_empty() => format!(
                "Reference:{h};frontal={};rear={};cage={};match={}",
                roundtrip(self.reference_frontal),
                roundtrip(self.reference_rear),
                if self.reference_average_normals {
                    "average"
                } else {
                    "vertex"
                },
                if self.reference_match_by_name {
                    "name"
                } else {
                    "all"
                }
            ),
            _ => "Self".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct MeshBakeBudget {
    pub max_bytes: u64,
    pub max_seconds: f64,
    pub max_degree_of_parallelism: usize,
}
impl Default for MeshBakeBudget {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024,
            max_seconds: 0.,
            max_degree_of_parallelism: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshBakeStatus {
    Completed,
    Canceled,
    TimedOut,
}
#[derive(Clone, Debug, Default)]
pub struct MeshBakeReport {
    pub covered_texels: u64,
    pub overlap_texels: u64,
    pub padded_texels: u64,
    pub empty_texels: u64,
    pub rays: u64,
    pub estimated_bytes: u64,
    pub projected_samples: u64,
    pub missed_samples: u64,
    pub receiving_triangles: usize,
    pub zero_uv_area_triangles: usize,
    pub degenerate_triangles: usize,
    pub occluder_triangles: usize,
    pub reference_triangles: usize,
    pub id_parts: usize,
    pub boundary_edges: usize,
    pub non_manifold_edges: usize,
    pub inconsistent_winding_edges: usize,
    pub curvature_segments: usize,
    /// 接線が無く UV から作った三角形の数（接空間法線を焼いたときだけ数える）。
    pub fallback_triangles: usize,
    pub prepare_seconds: f64,
    pub raster_seconds: f64,
    pub padding_seconds: f64,
    pub total_seconds: f64,
    /// 結果が平ら・白・欠けになる理由と、焼いた条件の記録。`diagnostics` はこれを文にしたもの。
    pub notes: Vec<MeshBakeNote>,
    pub diagnostics: Vec<String>,
}
/// ベイクの記録のうち、結果の見え方に関わるもの。並びは C# の MeshBaker の診断と同じ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshBakeNote {
    /// UV が重なるテクセルの数（番号の小さい三角形の値が入る）。
    OverlappingTexels(u64),
    /// 対象スロットの UV 面積 0 の三角形の数（焼かないが遮る）。
    ZeroUvAreaTriangles(usize),
    /// 頂点法線が無く、面の法線を使った。
    FaceNormals,
    /// 頂点法線を形から作り直した（由来の名前）。
    ReconstructedNormals(String),
    /// 接線が無く UV から作った三角形の数（接空間法線）。
    TangentFallback(usize),
    /// 非多様体または巻きの合わない辺の数（曲率は無視する）。
    IgnoredEdges(usize),
    /// 高ポリに当たらず低ポリで焼いたサンプルの数。
    MissedSamples {
        missed: u64,
        projected: u64,
        by_name: bool,
    },
    /// 手動 ID 色が元の色分けを上書きした部品の数。
    ManualIdColors(usize),
    /// 頂点カラーを選んだが頂点カラーが無く、ID が白になる。
    NoVertexColors,
    /// ID の部品の数と、どの 2 色も 1 チャンネルで離れている幅（8 bit）。
    IdParts {
        parts: usize,
        source: MeshIdSource,
        separation: u32,
    },
    /// 高ポリが無く、接空間法線は平ら・高さは 0.5。
    NoReference,
}
impl MeshBakeNote {
    /// 機械が読む短い記号（C# の診断との照合に使う。文言は言語ごとに違う）。
    pub fn code(&self) -> String {
        match self {
            Self::OverlappingTexels(n) => format!("overlap={n}"),
            Self::ZeroUvAreaTriangles(n) => format!("zero_uv={n}"),
            Self::FaceNormals => "normals=face".into(),
            Self::ReconstructedNormals(source) => format!("normals={source}"),
            Self::TangentFallback(n) => format!("tangent_fallback={n}"),
            Self::IgnoredEdges(n) => format!("bad_edges={n}"),
            Self::MissedSamples {
                missed,
                projected,
                by_name,
            } => format!(
                "missed={missed}/{projected}{}",
                if *by_name { "/name" } else { "" }
            ),
            Self::ManualIdColors(n) => format!("manual_parts={n}"),
            Self::NoVertexColors => "no_vertex_colors".into(),
            Self::IdParts {
                parts,
                source,
                separation,
            } => format!("id_parts={parts}/{source:?}/sep={separation}"),
            Self::NoReference => "no_reference".into(),
        }
    }
}
impl std::fmt::Display for MeshBakeNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverlappingTexels(n) => {
                write!(f, "UVが重なるテクセル {n} 個（番号の小さい三角形の値）")
            }
            Self::ZeroUvAreaTriangles(n) => {
                write!(f, "UV面積0の三角形 {n} 個（焼かない・遮蔽には使う）")
            }
            Self::FaceNormals => f.write_str("頂点法線なし（法線とレイの向きは面の法線）"),
            Self::ReconstructedNormals(source) => {
                write!(f, "頂点法線は形から再構築（{source}）。編集した法線は再現しない")
            }
            Self::TangentFallback(n) => write!(f, "接線のない三角形 {n} 個（UVから接線を作成）"),
            Self::IgnoredEdges(n) => {
                write!(f, "非多様体または巻きの合わない辺 {n} 本（曲率は無視）")
            }
            Self::MissedSamples {
                missed,
                projected,
                by_name,
            } => write!(
                f,
                "高ポリに当たらないサンプル {missed}/{projected}{}。低ポリの値で代用（接空間法線は平ら・高さ0.5・不透明度0）",
                if *by_name { "（名前の一致なしを含む）" } else { "" }
            ),
            Self::ManualIdColors(n) => write!(
                f,
                "手動ID色が{n}個の部品で元の色分けを上書き（色の離れは保証しない）"
            ),
            Self::NoVertexColors => f.write_str("頂点カラーなし（IDは白）"),
            Self::IdParts {
                parts,
                source,
                separation,
            } => write!(
                f,
                "IDの部品 {parts}（{source:?}）、どの2色も8 bitの1チャンネルで {separation} 以上離れる"
            ),
            Self::NoReference => f.write_str("高ポリなし（接空間法線は平ら、高さは0.5）"),
        }
    }
}
pub struct MeshBakeResult {
    pub status: MeshBakeStatus,
    pub maps: Vec<super::BakedMeshMap>,
    pub report: MeshBakeReport,
}
impl super::MeshMapProvenance {
    pub fn condition_key(&self) -> String {
        condition_key_text(
            self.kind,
            self.engine_version,
            &self.mesh_hash,
            self.uv_channel,
            self.width,
            self.height,
            self.target_slot,
            self.padding,
            self.antialiasing,
            &self.settings_key,
            &self.space,
            &self.pose,
            &self.source,
            &self.target_slots,
        )
    }
}
/// スロットが 2 つ以上なら並びも鍵に入れる（1 つなら前と同じ鍵: マテリアルごとのセットにする前に焼いたマップが古くならない）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn condition_key_text(
    kind: MeshMapKind,
    engine_version: i32,
    mesh_hash: &str,
    uv_channel: i32,
    width: i32,
    height: i32,
    target_slot: i32,
    padding: i32,
    antialiasing: i32,
    settings_key: &str,
    space: &str,
    pose: &str,
    source: &str,
    target_slots: &[i32],
) -> String {
    let slots = if target_slots.len() > 1 {
        format!(
            "slots={}\n",
            target_slots
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        String::new()
    };
    hash_text(&format!(
        "kind={}\nengine={engine_version}\nmesh={mesh_hash}\nuv={uv_channel}\nsize={width}x{height}\nslot={target_slot}\npadding={padding}\nantialiasing={antialiasing}\nsettings={settings_key}\nspace={space}\npose={pose}\nsource={source}\n{slots}",
        kind.name()
    ))
}

// Unity Mono の Double.ToString("R", InvariantCulture): 15桁で往復できなければ17桁。
fn roundtrip(value: f64) -> String {
    if value == 0. {
        return "0".into();
    }
    let first = format!("{value:.14e}");
    let (text, precision) = if first.parse::<f64>().ok() == Some(value) {
        (first, 15)
    } else {
        (format!("{value:.16e}"), 17)
    };
    let (mantissa, exponent) = text.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    let negative = mantissa.starts_with('-');
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    let digits = digits.trim_end_matches('0');
    let sign = if negative { "-" } else { "" };
    if exponent < -4 || exponent >= precision {
        let fraction = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        format!(
            "{sign}{}{fraction}E{}{:02}",
            &digits[..1],
            if exponent < 0 { "-" } else { "+" },
            exponent.abs()
        )
    } else {
        let point = exponent + 1;
        if point <= 0 {
            format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!(
                "{sign}{digits}{}",
                "0".repeat(point as usize - digits.len())
            )
        } else {
            format!(
                "{sign}{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        }
    }
}
