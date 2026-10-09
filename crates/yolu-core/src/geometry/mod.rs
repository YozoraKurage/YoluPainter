//! 3D の面の計算（Unity 版の Editor にあった SurfaceGeometry・SurfaceRegions を Core へ移したもの）: 三角形のスープ・頂点の溶接・
//! 隣り合わせ・BVH・レイの当たり（重心座標・UV）・最近点・ブラシの半径の中の面のテクセル（面の上のダブ）・範囲（UV アイランドなど）・
//! 3D ビューのカメラ・画面のストロークの点の並べ方。
//!
//! - 参照の写像（`sampling`）: クローン・指先が読む画素を、辺でつながった三角形の局所の展開（UV アイランドの継ぎ目をまたぐ）で決める。
//!   対称（`symmetry`）: ダブの中心をモデルの軸に直交する面で映す・軸のまわりに回して面へ投げ直し、写しの画素を大きい方の覆いで 1 つに
//!   する。`SurfaceStroke` の options（`paint`）が、ぼかし・指先・クローンと 3D の対称をストロークに通す。
//! - 位置・UV は Unity と同じ単精度で、式の順も同じにする（`unity` の写し）。C# の SurfaceGeometry に同じ入力を通した結果と、
//!   当たり・隣り合わせ・BVH・ダブの画素と覆いまでビットで一致することを `tests/reference/surface_golden.rs` で確かめる。
//! - 座標は Unity と同じ左手系（Y が上）。UV (0, 0) がテクスチャの左下（文書の画素 (0, 0)）。
//! - 三角形の番号は入力の並びのまま変わらない。`revision`（スナップショットの世代）が違う当たりでダブは作らない。
//! - BVH は読むだけなので、レイは並列に撃てる（ダブの遮蔽のレイ）。並列でも結果・数・断る理由は逐次と同じ。

mod build;
mod camera;
mod cover;
mod dab;
mod model;
mod paint;
mod project;
mod query;
mod refit;
mod regions;
mod restrict;
mod sampling;
mod screen;
pub(crate) mod seam_band;
mod stencil;
mod stroke;
mod symmetry;
pub(crate) mod unity;
mod uv_grid;
mod uv_symmetry;
mod uv_topology;

use std::sync::atomic::AtomicBool;

use glam::{Vec2, Vec3};

pub use camera::{
    aligned_axis_view, nearest_axis_view, orbited, snap_orientation, visible_height, AxisView,
    CameraView, OrbitCamera, Projection, Sight, Viewer, DEFAULT_PITCH, DEFAULT_YAW,
    ORBIT_DEGREES_PER_POINT, SNAP_ANGLE,
};
pub use cover::{CoverParams, CoverPixel, SurfaceCoverStroke};
pub use dab::{
    DabRefusal, SurfaceBrushBudget, SurfaceDabResult, SurfacePixel, SurfaceVisibilityCache,
};
pub use model::{cube_sphere, demo_cube, model_triangles, ModelMesh, Submesh};
pub use paint::{
    pick, world_radius, SurfaceCloneSource, SurfaceEffect, SurfaceInput, SurfaceStroke,
    SurfaceStrokeError, SurfaceStrokeOptions, SurfaceStrokeStats, SurfaceSymmetrySetup,
    MAX_QUEUED_DABS,
};
pub use project::{
    set_parallel_projection_candidates, CopyTransform, ProjectionSettings, ProjectionStats,
    SurfaceProjector, MAX_BUCKET, MAX_SEAM_BLEED, MIN_BUCKET,
};
pub use query::{
    barycentric, closest_point, intersect_triangle, uv_barycentric, NodeBudgetExceeded,
};
pub use refit::BvhUpdate;
pub use regions::{region, SurfaceRegionKind};
pub use sampling::{
    SamplingChart, SamplingError, SAMPLING_CHART_MAX_TRIANGLES, SAMPLING_CHART_TRIANGLE_BYTES,
};
pub use screen::{cover_screen, ScreenCoverSettings, ScreenCoverage, ScreenShape};
pub use seam_band::{seam_band_width, SeamBand, SeamBandStats, MAX_CHART_TRIANGLES};
pub use stencil::SurfaceStencil;
pub use stroke::{
    ScreenDab, ScreenPoint, ScreenStrokeSampler, SegmentGaps, StrokeCurve, TooManyDabs,
    SURFACE_DABS_PER_SEGMENT,
};
pub use symmetry::{
    build_expanded, build_mirrored, copy_count, copy_hits, find_copy, search_distance, union_dabs,
    CopyHit, DabSide, ExpandedSurfaceDab, MirrorOutcome, MirrorPlane, RadialSymmetry,
    SymmetricSurfaceDab, SymmetryAxis, SymmetryError, MAX_CLOSEST_POINT_NODE_VISITS,
    ON_PLANE_FRACTION,
};
pub use unity::{Bounds, Ray};
pub use uv_grid::UvGrid;
pub use uv_symmetry::{ModelCopies, ModelSymmetry, UvCopy, MAX_CANVAS_COPY_PIXELS};
pub use uv_topology::{
    IslandMap, IslandRun, UvTopology, UvTopologyError, DEFAULT_BUDGET, MAX_SEAM_BAND,
    MAX_TOPOLOGY_EDGE,
};

/// スナップショットの三角形 1 つ（位置はモデルの空間、UV は 0 番）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTriangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub uv_a: Vec2,
    pub uv_b: Vec2,
    pub uv_c: Vec2,
    /// レンダラー（メッシュ）の番号。
    pub renderer: i32,
    /// レンダラー × サブメッシュのスロットの番号（モデル全体の通し番号）。隣り合わせはこれが同じ三角形の間だけ。
    pub material_slot: i32,
    /// マテリアルの組（同じマテリアルを使うスロットの組。テクスチャセット 1 つ）。
    pub material: i32,
}

impl SurfaceTriangle {
    /// レンダラー 0・スロット 0・マテリアル 0 の三角形。
    pub fn new(a: Vec3, b: Vec3, c: Vec3, uv_a: Vec2, uv_b: Vec2, uv_c: Vec2) -> SurfaceTriangle {
        SurfaceTriangle {
            a,
            b,
            c,
            uv_a,
            uv_b,
            uv_c,
            renderer: 0,
            material_slot: 0,
            material: 0,
        }
    }

    /// レンダラー・スロット・マテリアルの組を付ける（material が負ならスロットと同じ。C# の既定と同じ）。
    pub fn with_slot(
        mut self,
        renderer: i32,
        material_slot: i32,
        material: i32,
    ) -> SurfaceTriangle {
        self.renderer = renderer;
        self.material_slot = material_slot;
        self.material = if material >= 0 {
            material
        } else {
            material_slot
        };
        self
    }

    /// 面の法線（B − A と C − A の外積を長さで割る。面積が 0 なら 0）。
    pub fn normal(&self) -> Vec3 {
        let c = unity::cross(self.b - self.a, self.c - self.a);
        let length = unity::magnitude(c);
        if length > 1e-20 {
            Vec3::new(c.x / length, c.y / length, c.z / length)
        } else {
            Vec3::ZERO
        }
    }

    /// テクセル 1 つ（文書の大きさ width × height）の、モデルの単位の大きさ（面積の比の平方根。潰れていれば 0）。
    pub fn texel_size(&self, width: i32, height: i32) -> f32 {
        let area = unity::magnitude(unity::cross(self.b - self.a, self.c - self.a)) as f64 * 0.5;
        let (e1, e2) = (self.uv_b - self.uv_a, self.uv_c - self.uv_a);
        let uv = (e1.x as f64 * e2.y as f64 - e1.y as f64 * e2.x as f64).abs()
            * 0.5
            * width as f64
            * height as f64;
        if area > 0.0 && uv > 0.0 && area.is_finite() && uv.is_finite() {
            (area / uv).sqrt() as f32
        } else {
            0.0
        }
    }

    /// 3 頂点の箱（C# の SurfaceTriangle.Bounds と同じ丸め）。
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::point(self.a);
        b.encapsulate_point(self.b);
        b.encapsulate_point(self.c);
        b
    }
}

/// レイや最近点の当たり（位置はモデルの空間）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    /// 当てたスナップショットの世代。
    pub revision: u32,
    pub renderer: i32,
    pub material_slot: i32,
    pub material: i32,
    pub triangle: u32,
    pub position: Vec3,
    /// 三角形の面の法線。
    pub normal: Vec3,
    /// A・B・C の重み。
    pub barycentric: Vec3,
    pub uv: Vec2,
    /// レイの始点（最近点なら問い合わせの点）からの距離。
    pub distance: f32,
}

/// 組み立てを断った理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    /// 位置・UV・法線に有限でない値がある。
    NonFinite,
    /// 全体の箱が単精度の範囲を超える。
    BoundsOverflow,
    /// 溶接の許しが正の有限の値でない。
    InvalidTolerance,
    /// 三角形が多すぎる（番号が 32 bit に収まらない）。
    TooManyTriangles,
    /// 取り消した（`SurfaceGeometry::build_cancelable` の印）。
    Canceled,
    /// 位置を変えた三角形の並び・スロット・UV が元のスナップショットと違う（`SurfaceGeometry::reposition`）。
    Mismatch,
}

impl std::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GeometryError::NonFinite => "メッシュの位置か UV に有限でない値があります",
            GeometryError::BoundsOverflow => "メッシュの大きさが扱える範囲を超えています",
            GeometryError::InvalidTolerance => "溶接の許しは正の値にしてください",
            GeometryError::TooManyTriangles => "三角形が多すぎます",
            GeometryError::Canceled => "取り消しました",
            GeometryError::Mismatch => "三角形の並びが元のスナップショットと違います",
        })
    }
}

impl std::error::Error for GeometryError {}

/// 組み立ての時間（ミリ秒。計測用）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildTimings {
    /// 三角形の写しと箱。
    pub snapshot_ms: f64,
    /// 溶接と隣り合わせ。
    pub adjacency_ms: f64,
    pub bvh_ms: f64,
}

/// 変わらない三角形のスナップショットと、その隣り合わせ・BVH。作った後は読むだけ（`Send + Sync`）。
pub struct SurfaceGeometry {
    pub(crate) triangles: Vec<SurfaceTriangle>,
    /// 三角形 i の隣は `adjacency[adjacency_offsets[i]..adjacency_offsets[i + 1]]`。
    pub(crate) adjacency_offsets: Vec<u32>,
    pub(crate) adjacency: Vec<u32>,
    /// BVH の葉が指す三角形の番号の並び。
    pub(crate) indices: Vec<u32>,
    pub(crate) nodes: Vec<build::BvhNode>,
    pub(crate) visibility_epsilon: f32,
    pub(crate) seam_tolerance: f32,
    pub(crate) revision: u32,
    pub(crate) bounds: Bounds,
    pub(crate) non_manifold_edge_count: u32,
    /// ブラシの半径をモデルの単位に直す基準（箱の対角線。`reposition` は元の値を引き継ぐ）。
    pub(crate) brush_scale: f32,
    pub(crate) timings: BuildTimings,
    /// 投影の塗りの、カメラによらない一覧（UV の覆い・アイランドの縁）の覚え（テクスチャセットと文書の大きさごと。新しい 2 つまで）。
    /// 位置だけ変えたスナップショット（ポーズ）は、UV と隣り合わせが同じなので引き継ぐ。
    pub(crate) projection_cache: std::sync::Mutex<Vec<std::sync::Arc<project::Shared>>>,
}

/// 溶接の既定の許し（C# と同じ 1e-6。モデルの単位）。
pub const DEFAULT_WELD_TOLERANCE: f32 = 0.000_001;

impl SurfaceGeometry {
    /// 三角形から作る（溶接の許しは座標の量子化の幅。その格子で同じ点になる頂点の辺がつながる）。
    pub fn new(
        triangles: Vec<SurfaceTriangle>,
        revision: u32,
        weld_tolerance: f32,
    ) -> Result<SurfaceGeometry, GeometryError> {
        build::build(
            triangles,
            revision,
            weld_tolerance,
            build::Cancel(None),
            None,
        )
    }

    /// 別のスレッドで組むとき: cancel を立てると途中でやめて `Canceled` を返す（作りかけは返さない）。
    pub fn build_cancelable(
        triangles: Vec<SurfaceTriangle>,
        revision: u32,
        weld_tolerance: f32,
        cancel: &AtomicBool,
    ) -> Result<SurfaceGeometry, GeometryError> {
        build::build(
            triangles,
            revision,
            weld_tolerance,
            build::Cancel(Some(cancel)),
            None,
        )
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
    /// 三角形（並びは番号と同じ）。
    pub fn triangles(&self) -> &[SurfaceTriangle] {
        &self.triangles
    }
    /// 三角形 i と辺を共有する三角形（辺の初出の順）。
    pub fn neighbors(&self, i: usize) -> &[u32] {
        let (s, e) = (
            self.adjacency_offsets[i] as usize,
            self.adjacency_offsets[i + 1] as usize,
        );
        &self.adjacency[s..e]
    }
    /// 3 つ以上の三角形が使う辺の数（その辺ではダブが隣へ進まない）。
    pub fn non_manifold_edge_count(&self) -> u32 {
        self.non_manifold_edge_count
    }
    /// 全体の箱。
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    /// ブラシの半径をモデルの単位に直す基準の長さ（作ったときの箱の対角線。ポーズで位置だけ変えたスナップショットは、元の形の値を
    /// 引き継ぐので、ポーズでブラシの大きさが変わらない）。
    pub fn brush_scale(&self) -> f32 {
        self.brush_scale
    }
    /// モデルの半径の目安（箱の半分の対角線。0.0001 以上。カメラの距離と寄る範囲に使う）。
    pub fn model_radius(&self) -> f32 {
        unity::fmax(0.0001, unity::magnitude(self.bounds.extents))
    }
    /// 遮蔽の判定の許し（モデルの対角線の 1e-6、1e-7 以上）。
    pub fn visibility_epsilon(&self) -> f32 {
        self.visibility_epsilon
    }
    pub fn weld_tolerance(&self) -> f32 {
        self.seam_tolerance
    }
    pub fn timings(&self) -> BuildTimings {
        self.timings
    }
    /// BVH の節点の数（試験・計測用）。
    pub fn bvh_node_count(&self) -> usize {
        self.nodes.len()
    }

    /// BVH の中身の指紋を作る関数に、節点（前順）ごとに 箱の中心・半分・左・右・始め・数、続けて葉の三角形の並びを渡す（照合用）。
    pub fn visit_bvh(
        &self,
        mut node: impl FnMut(Bounds, i64, i64, u32, u32),
        mut index: impl FnMut(u32),
    ) {
        for n in &self.nodes {
            let (l, r) = if n.count == 0 {
                (n.left as i64, n.right as i64)
            } else {
                (-1, -1)
            };
            node(n.bounds, l, r, n.start, n.count);
        }
        for &i in &self.indices {
            index(i);
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod uv_parallel_tests;
