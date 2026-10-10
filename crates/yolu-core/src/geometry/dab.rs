//! 面の上のダブ: ブラシの半径の中の、カメラから見えるテクスチャの画素と覆い（C# の SurfaceGeometry.BuildSurfaceDabs）。
//!
//! 1. 当たった三角形から幅優先で辿り（同じレンダラー・同じスロット・半径の中・カメラを向く面だけ進む）、三角形ごとに UV の足跡の
//!    箱の中のテクセルの中心を重心座標で 3D へ戻し、半径の中のものを候補として並びのまま集める。三角形の数と候補の画素の予算はここで数える。
//! 2. 候補ごとにカメラから遮蔽のレイを撃つ（4096 本ずつの組で並列。BVH は読むだけ）。正投影のカメラ（[`Viewer::Parallel`]）は、
//!    近い面から前の向きに平行なレイを撃ち、面の向きも前の向きと比べる。
//! 3. 組ごとに並びのとおりに予算を数えて受け入れる。予算を超えたら画素を返さずに断る（呼ぶ側はストロークを取り消す）。
//!    同じ画素は覆いの大きいほう（同じなら先に訪れた三角形）を残し、下の行から順に並べる。
//!
//! 画素・覆い・数・断る理由は、並列の度合いによらず逐次で撃ったときと同じ（C# と同じ作り）。

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use super::build::FastMap;
use super::camera::Viewer;
use super::query::{coverage, uv_barycentric, uv_footprint_bounds, RayQueryBudget};
use super::unity::{clamp01, dot, finite, finite3, magnitude, mix3, sqr_magnitude, Ray};
use super::{SurfaceGeometry, SurfaceHit, SurfaceTriangle};
use crate::brush::edge;
use crate::brush::AntiAlias;

/// ダブ 1 つの仕事の上限（C# の SurfaceBrushBudget と同じ既定値）。どれかを超えたダブは画素を返さずに断る。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceBrushBudget {
    pub max_triangles: i32,
    pub max_candidate_pixels: i32,
    pub max_visibility_rays: i32,
    pub max_ray_triangle_tests: i32,
    pub max_ray_node_visits: i32,
}

impl Default for SurfaceBrushBudget {
    fn default() -> Self {
        SurfaceBrushBudget {
            max_triangles: 2048,
            max_candidate_pixels: 262_144,
            max_visibility_rays: 131_072,
            max_ray_triangle_tests: 2_000_000,
            max_ray_node_visits: 4_000_000,
        }
    }
}

/// ダブの画素 1 つ（画素の座標は左下原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePixel {
    pub x: i32,
    pub y: i32,
    pub coverage: f32,
    /// 最大の覆いを与えた三角形（同じなら先に訪れた三角形）。カメラによらない足跡（対称の側）では None。
    pub triangle: Option<u32>,
    /// その三角形の上のテクセルの中心の位置（None の三角形なら 0）。
    pub position: Vec3,
}

/// ダブを作らなかった・断った理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DabRefusal {
    /// 当たりのスナップショットの世代が違う・三角形の番号が範囲外（新しいストロークで描き直す）。
    SnapshotChanged,
    /// 半径・解像度・カメラ・当たりの位置が範囲外か有限でない。
    InvalidArguments,
    /// 当たりのレンダラー・スロットがスナップショットの三角形と合わない。
    BindingMismatch,
    /// 辿る三角形の数の予算を超えた（ストロークを取り消す）。
    TriangleBudget,
    /// 候補の画素の予算を超えた（ストロークを取り消す）。
    PixelBudget,
    /// 遮蔽のレイの本数の予算を超えた（ストロークを取り消す）。
    VisibilityBudget,
    /// 遮蔽のレイの BVH の仕事量の予算を超えた（ストロークを取り消す）。
    BvhBudget,
    /// 投影の塗りで、このダブに要る区画の投影の画素が、1 回の操作のメモリに入らない（ストロークを取り消す）。
    MemoryBudget,
}

impl DabRefusal {
    /// 上限を超えた断りか（ダブ 1 つの 4 つの上限と、投影の塗りのメモリ）。3D のストロークは、どの断りでも（2D のストロークと
    /// 同じく）ストロークごと取り消す。パスの評価は、どの断りでも評価ごと失敗する。
    pub fn is_limit(self) -> bool {
        matches!(
            self,
            DabRefusal::TriangleBudget
                | DabRefusal::PixelBudget
                | DabRefusal::VisibilityBudget
                | DabRefusal::BvhBudget
                | DabRefusal::MemoryBudget
        )
    }

    /// C# の Diagnostic と同じ英語の文（照合用）。投影の塗りのメモリ（`MemoryBudget`）は C# に無い断りで、ストロークを取り消すと言う。
    pub fn csharp_message(self) -> &'static str {
        match self {
            DabRefusal::SnapshotChanged => "The model snapshot changed. Start a new stroke.",
            DabRefusal::InvalidArguments => "Invalid surface brush size, resolution or camera.",
            DabRefusal::BindingMismatch => "Surface binding does not match the current snapshot.",
            DabRefusal::TriangleBudget => "Surface dab exceeded the triangle budget. No pixels were changed; reduce the brush radius.",
            DabRefusal::PixelBudget => "Surface dab exceeded the pixel budget. No pixels were changed; reduce brush radius or use a smaller document.",
            DabRefusal::VisibilityBudget => "Surface dab exceeded the visibility budget. No pixels were changed; reduce the brush radius.",
            DabRefusal::BvhBudget => "Surface visibility exceeded the BVH work budget. No pixels were changed; reduce the radius or simplify overlapping geometry.",
            DabRefusal::MemoryBudget => "The projected brush area does not fit in the stroke memory budget. The stroke was cancelled.",
        }
    }
}

impl std::fmt::Display for DabRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DabRefusal::SnapshotChanged => "モデルが変わりました。描き直してください",
            DabRefusal::InvalidArguments => "ブラシの大きさ・解像度・カメラが範囲外です",
            DabRefusal::BindingMismatch => "当たった面がモデルと合いません",
            DabRefusal::TriangleBudget => "ブラシが広すぎるので、ストロークを取り消しました。ブラシを小さくしてください",
            DabRefusal::PixelBudget => "ブラシが大きすぎるので、ストロークを取り消しました。ブラシを小さくするか、キャンバスを小さくしてください",
            DabRefusal::VisibilityBudget => "見え方の確認が多すぎるので、ストロークを取り消しました。ブラシを小さくしてください",
            DabRefusal::BvhBudget => "見え方の確認が重すぎるので、ストロークを取り消しました。ブラシを小さくするか、重なった面を減らしてください",
            DabRefusal::MemoryBudget => "ブラシの範囲が 1 回の操作のメモリに入らないので、ストロークを取り消しました",
        })
    }
}

/// ダブの結果と、使った仕事の数（試験と計測用。断ったときも、断った時点の数）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceDabResult {
    /// 画素（行優先・下の行から）。断ったときは空。
    pub pixels: Vec<SurfacePixel>,
    pub refusal: Option<DabRefusal>,
    pub candidate_pixels: i32,
    pub visibility_rays: i32,
    pub ray_triangle_tests: i32,
    pub visited_triangles: i32,
    pub ray_node_visits: i64,
}

impl SurfaceDabResult {
    /// 上限で断ったか（C# の WasClipped）。
    pub fn was_clipped(&self) -> bool {
        self.refusal.is_some_and(|r| r.is_limit())
    }
    pub(crate) fn reject(mut self, why: DabRefusal) -> SurfaceDabResult {
        self.pixels.clear();
        self.refusal = Some(why);
        self
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CachedRay {
    skipped: bool,
    has_hit: bool,
    hit_triangle: u32,
    hit_distance: f32,
    camera_distance: f32,
    hit_position: Vec3,
}

/// 1 回のストロークのあいだ、テクセルがカメラから見えるか（遮蔽のレイの結果）を覚えておく（C# の SurfaceVisibilityCache）。
/// ストロークの間はカメラもスナップショットも動かないので、重なり合う次のダブで同じテクセルのレイを撃ち直さない。カメラ・世代・
/// 解像度が変わったら空にする。覚えたレイは撃たないので BVH の仕事量の予算にも数えない（画素は変わらない）。上限を超えたら空にする。
#[derive(Debug)]
pub struct SurfaceVisibilityCache {
    rays: HashMap<i64, CachedRay>,
    revision: i64,
    width: i32,
    height: i32,
    camera: Viewer,
    /// 覚えていた結果を使った数（試験と計測用）。
    pub hits: i64,
}

impl Default for SurfaceVisibilityCache {
    fn default() -> Self {
        SurfaceVisibilityCache {
            rays: HashMap::new(),
            revision: -1,
            width: 0,
            height: 0,
            camera: Viewer::Point(Vec3::ZERO),
            hits: 0,
        }
    }
}

impl SurfaceVisibilityCache {
    pub const MAX_ENTRIES: usize = 1 << 21;

    pub fn new() -> SurfaceVisibilityCache {
        SurfaceVisibilityCache::default()
    }
    pub fn len(&self) -> usize {
        self.rays.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rays.is_empty()
    }
    fn prepare(&mut self, revision: u32, camera: Viewer, width: i32, height: i32) {
        // カメラは Unity の Vector3 の == （差の長さの 2 乗が 1e-10 未満）で比べる（正投影は前の向きと近い面の位置）
        let same = |a: Vec3, b: Vec3| sqr_magnitude(a - b) < 0.00001f32 * 0.00001f32;
        let same_camera = match (camera, self.camera) {
            (Viewer::Point(a), Viewer::Point(b)) => same(a, b),
            (
                Viewer::Parallel { forward, near },
                Viewer::Parallel {
                    forward: f,
                    near: n,
                },
            ) => same(forward, f) && (near - n).abs() < 0.00001,
            _ => false,
        };
        if revision as i64 == self.revision
            && same_camera
            && width == self.width
            && height == self.height
            && self.rays.len() < Self::MAX_ENTRIES
        {
            return;
        }
        self.rays.clear();
        self.revision = revision as i64;
        self.camera = camera;
        self.width = width;
        self.height = height;
    }
}

#[derive(Clone, Copy)]
struct DabCandidate {
    triangle: u32,
    x: i32,
    y: i32,
    candidates_so_far: i32,
    bary: Vec3,
    position: Vec3,
    distance: f32,
}

#[derive(Clone, Copy, Default)]
struct RayOutcome {
    skipped: bool,
    has_hit: bool,
    exceeded: bool,
    cached: bool,
    hit_triangle: u32,
    hit_distance: f32,
    hit_position: Vec3,
    camera_distance: f32,
    tests: i64,
    visits: i64,
}

/// テクセルの x・y の 1 つ分が、三角形の上でモデルの空間でどれだけ動くか（解像度 width × height。UV が潰れていれば None）。
pub(crate) fn texel_steps_world(
    t: &SurfaceTriangle,
    width: i32,
    height: i32,
) -> Option<[glam::DVec3; 2]> {
    let scale = glam::DVec2::new(width as f64, height as f64);
    let (p0, p1, p2) = (
        t.uv_a.as_dvec2() * scale,
        t.uv_b.as_dvec2() * scale,
        t.uv_c.as_dvec2() * scale,
    );
    let (e1, e2) = (p1 - p0, p2 - p0);
    let det = e1.x * e2.y - e1.y * e2.x;
    if !det.is_finite() || det.abs() <= 1e-12 {
        return None;
    }
    let (c11, c12, c21, c22) = (e2.y / det, -e2.x / det, -e1.y / det, e1.x / det);
    let (a, b, c) = (t.a.as_dvec3(), t.b.as_dvec3(), t.c.as_dvec3());
    let (f1, f2) = (b - a, c - a);
    let steps = [f1 * c11 + f2 * c21, f1 * c12 + f2 * c22];
    (steps[0].is_finite() && steps[1].is_finite()).then_some(steps)
}

/// テクセルの動き steps（モデルの空間）の、いちばん長い・短い向きの長さ（StepsᵀSteps の固有値の平方根）。
fn step_lengths(steps: &[glam::DVec3; 2]) -> (f64, f64) {
    let m = edge::TexelMetric {
        xx: steps[0].length_squared(),
        xy: steps[0].dot(steps[1]),
        yy: steps[1].length_squared(),
    };
    let (big, small) = m.eigen();
    (big.sqrt(), small.sqrt())
}

/// 三角形 1 つのテクセルの動き（モデルの空間）・いちばん長い向きの長さ・帯の幅と濃さ。
struct TriangleTexels {
    steps: [glam::DVec3; 2],
    long: f64,
    e: edge::Edge,
}

/// 面のダブ（モデルの空間の球）の縁のアンチエイリアス（[`crate::brush::AntiAlias`]）。帯（テクセル）は、テクセルの候補の三角形の
/// テクセルの動きで、球の距離の勾配をテクセルの空間へ直して当てる。届く半径は、当たった三角形のテクセルの 2 倍の大きさまで帯が収まる
/// ように取り、それより大きいテクセルでは帯をその半径に収まる幅で止める。
#[derive(Clone, Copy, Debug)]
pub(crate) struct SphereEdge {
    band: f64,
    hardness: f64,
    radius: f64,
    /// 届く距離（規格化）と、その中に収まる帯の上限（規格化した単位）。
    pub outer: f64,
    cap: f64,
    /// 三角形が分からない所（カメラによらない足跡の覆いを呼び手が決める形）で使う、当たった三角形の値の帯（規格化した単位）と濃さ。
    pub band_at_hit: f64,
    pub density_at_hit: f64,
}

impl SphereEdge {
    /// 当たり hit の三角形で見積もる（段がなし・UV が潰れていれば None）。
    pub(crate) fn new(
        geometry: &SurfaceGeometry,
        hit: &SurfaceHit,
        radius_world: f32,
        hardness: f32,
        level: AntiAlias,
        width: i32,
        height: i32,
    ) -> Option<SphereEdge> {
        let w = level.band();
        let t = geometry.triangles().get(hit.triangle as usize)?;
        if w <= 0.0 || !edge::above(radius_world as f64, 0.0) {
            return None;
        }
        let steps = texel_steps_world(t, width, height)?;
        let (long, short) = step_lengths(&steps);
        if !edge::above(short, 0.0) {
            return None;
        }
        let (r, h) = (radius_world as f64, clamp01(hardness) as f64);
        let estimate = 2.0 * w.max(1.0) * long / r;
        let (_, outer) = edge::bounds(h, estimate);
        let e = edge::band_and_density(w, h, r / short, r / long);
        Some(SphereEdge {
            band: w,
            hardness: h,
            radius: r,
            outer,
            cap: edge::band_for_outer(h, outer),
            band_at_hit: (e.band * long / r).min(edge::band_for_outer(h, outer)),
            density_at_hit: e.density,
        })
    }

    /// 三角形 t のテクセルの動きと、その三角形での帯の幅・濃さ（UV が潰れていれば None。ダブの中で三角形ごとに 1 回）。
    fn texels(&self, t: &SurfaceTriangle, width: i32, height: i32) -> Option<TriangleTexels> {
        let steps = texel_steps_world(t, width, height)?;
        let (long, short) = step_lengths(&steps);
        if !edge::above(short, 0.0) {
            return None;
        }
        let e = edge::band_and_density(
            self.band,
            self.hardness,
            self.radius / short,
            self.radius / long,
        );
        Some(TriangleTexels { steps, long, e })
    }

    /// 三角形（`texels`）の上の点 position（球の中心 center から、規格化した距離 distance）の覆い。帯がぼかしの幅以下で濃さ 1 なら今の式。
    fn cover_with(
        &self,
        texels: Option<&TriangleTexels>,
        position: Vec3,
        center: Vec3,
        distance: f32,
    ) -> f32 {
        let Some(tt) = texels else {
            return coverage(distance, self.hardness as f32);
        };
        let r = (position - center).as_dvec3();
        let len = r.length();
        // 規格化した距離のテクセルあたりの勾配（中心は、テクセルがいちばん長い向きの値）
        let g = if len > 0.0 {
            let n = r / len;
            (tt.steps[0].dot(n).powi(2) + tt.steps[1].dot(n).powi(2)).sqrt() / self.radius
        } else {
            tt.long / self.radius
        };
        let band = (tt.e.band * g).min(self.cap);
        if tt.e.density == 1.0 && !edge::above(band, 1.0 - self.hardness) {
            return coverage(distance, self.hardness as f32);
        }
        edge::cover64(distance as f64, self.hardness, band, tt.e.density) as f32
    }

    /// 三角形の分からない所の覆い（当たった三角形の値で、向きによらない帯）。
    pub(crate) fn cover_at_hit(&self, distance: f64) -> f32 {
        if self.density_at_hit == 1.0 && !edge::above(self.band_at_hit, 1.0 - self.hardness) {
            return coverage(distance as f32, self.hardness as f32);
        }
        edge::cover64(
            distance,
            self.hardness,
            self.band_at_hit,
            self.density_at_hit,
        ) as f32
    }
}

/// 組の大きさ（C# と同じ 4096。予算を超えたときに無駄に撃つのは多くても 1 組）。64 本未満の組は呼んだスレッドで撃つ。
const CHUNK: usize = 4096;
/// 1 つのワーカーが続けて撃つレイの最小の数。細かく分けると、32 スレッドでは起こす手間が勝って 1 本のスレッドより遅くなった
/// （69,312 三角形の球・2048²・半径 0.15・57,023 画素で、分けない 43.9 ms・64 本 34.2 ms・256 本 23.0 ms・512 本 18.5 ms、
/// 1 本のスレッド 28.9 ms。2026-10-04、ほかの作業と並行の計測）。
const MIN_RAYS_PER_TASK: usize = 512;

impl SurfaceGeometry {
    /// 半径 radius_world（モデルの単位）の球が面の上で覆う、カメラから見えるテクセルと覆い（解像度 width × height）。
    /// hardness は C# と同じく 0〜1 に収める。cache はストロークの間の遮蔽の結果（ストロークごとに 1 つ。None なら覚えない）。
    /// ignore_visibility なら遮蔽のレイを撃たず、面の向きは当たりの法線と比べる（対称の側のカメラによらない足跡）。
    /// camera はカメラの位置（透視）か [`Viewer`]（正投影は [`super::CameraView::viewer`]）。
    #[allow(clippy::too_many_arguments)]
    pub fn build_surface_dabs(
        &self,
        hit: &SurfaceHit,
        radius_world: f32,
        width: i32,
        height: i32,
        camera: impl Into<Viewer>,
        hardness: f32,
        budget: &SurfaceBrushBudget,
        cache: Option<&mut SurfaceVisibilityCache>,
        ignore_visibility: bool,
    ) -> SurfaceDabResult {
        self.build_surface_dabs_with(
            hit,
            radius_world,
            width,
            height,
            camera.into(),
            hardness,
            budget,
            cache,
            ignore_visibility,
            None,
            AntiAlias::None,
        )
    }

    /// [`SurfaceGeometry::build_surface_dabs`] に、丸の縁のアンチエイリアスの段 anti_alias を加えたもの（なしなら同じ結果）。
    #[allow(clippy::too_many_arguments)]
    pub fn build_surface_dabs_anti_aliased(
        &self,
        hit: &SurfaceHit,
        radius_world: f32,
        width: i32,
        height: i32,
        camera: impl Into<Viewer>,
        hardness: f32,
        anti_alias: AntiAlias,
        budget: &SurfaceBrushBudget,
        cache: Option<&mut SurfaceVisibilityCache>,
        ignore_visibility: bool,
    ) -> SurfaceDabResult {
        self.build_surface_dabs_with(
            hit,
            radius_world,
            width,
            height,
            camera.into(),
            hardness,
            budget,
            cache,
            ignore_visibility,
            None,
            anti_alias,
        )
    }

    /// カメラによらない足跡（対称の写しの側、[`SurfaceGeometry::build_surface_dabs`] の ignore_visibility）の、覆いを呼び手が決める形:
    /// 半径 radius_world の球に入るテクセルごとに、`cover(テクセルの点, x, y)` を覆いにする（同じテクセルは大きいほう）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_surface_footprint(
        &self,
        hit: &SurfaceHit,
        radius_world: f32,
        width: i32,
        height: i32,
        camera: Vec3,
        budget: &SurfaceBrushBudget,
        cover: &dyn Fn(Vec3, i32, i32) -> f32,
    ) -> SurfaceDabResult {
        self.build_surface_dabs_with(
            hit,
            radius_world,
            width,
            height,
            Viewer::Point(camera),
            1.0,
            budget,
            None,
            true,
            Some(cover),
            AntiAlias::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_surface_dabs_with(
        &self,
        hit: &SurfaceHit,
        radius_world: f32,
        width: i32,
        height: i32,
        camera: Viewer,
        hardness: f32,
        budget: &SurfaceBrushBudget,
        cache: Option<&mut SurfaceVisibilityCache>,
        ignore_visibility: bool,
        cover: Option<&dyn Fn(Vec3, i32, i32) -> f32>,
        anti_alias: AntiAlias,
    ) -> SurfaceDabResult {
        let mut result = SurfaceDabResult::default();
        if hit.revision != self.revision || hit.triangle as usize >= self.triangles.len() {
            result.refusal = Some(DabRefusal::SnapshotChanged);
            return result;
        }
        if width <= 0
            || height <= 0
            || width > 32768
            || height > 32768
            || !finite(radius_world)
            || radius_world <= 0.0
            || !camera.is_finite()
            || !finite3(hit.position)
        {
            result.refusal = Some(DabRefusal::InvalidArguments);
            return result;
        }
        let seed = &self.triangles[hit.triangle as usize];
        if seed.renderer != hit.renderer || seed.material_slot != hit.material_slot {
            result.refusal = Some(DabRefusal::BindingMismatch);
            return result;
        }
        let hardness = clamp01(hardness);
        // アンチエイリアスは、球の外へ帯の分だけ届く（候補はその半径の中のテクセル。覆いを呼び手が決める形には使わない）
        let sphere = cover
            .is_none()
            .then(|| SphereEdge::new(self, hit, radius_world, hardness, anti_alias, width, height))
            .flatten();
        // 候補を集める半径（帯の外の端まで）と、規格化した距離の打ち切り。距離はいつも元の半径で割る（帯の無い画素は今と同じビット）
        let (reach_world, cutoff) = match &sphere {
            Some(e) => (radius_world * e.outer as f32, e.outer as f32),
            None => (radius_world, 1.0),
        };

        // 帯の三角形ごとの値（テクセルの動き・帯の幅・濃さ）の覚え
        let mut texel_cache: FastMap<u32, Option<TriangleTexels>> = FastMap::default();

        // 1. 幅優先で候補のテクセルを並びのまま集める
        let mut queue: VecDeque<u32> = VecDeque::new();
        let mut visited: HashSet<u32, std::hash::BuildHasherDefault<super::build::MixHasher>> =
            HashSet::default();
        let mut candidates: Vec<DabCandidate> = Vec::new();
        queue.push_back(hit.triangle);
        visited.insert(hit.triangle);
        let radius_sq = reach_world * reach_world;
        let mut processed = 0i32;
        let mut stop: Option<DabRefusal> = None;
        let (wf, hf) = (width as f32, height as f32);
        while let Some(index) = queue.pop_front() {
            let t = &self.triangles[index as usize];
            processed += 1;
            result.visited_triangles = processed;
            if processed > budget.max_triangles {
                stop = Some(DabRefusal::TriangleBudget);
                break;
            }
            if t.renderer != hit.renderer
                || t.material_slot != hit.material_slot
                || t.bounds().sqr_distance(hit.position) > radius_sq
            {
                continue;
            }
            if sqr_magnitude(super::query::closest_point(hit.position, t) - hit.position)
                > radius_sq
            {
                continue;
            }
            let faces = if ignore_visibility {
                dot(t.normal(), hit.normal) <= 0.0
            } else {
                dot(t.normal(), camera.to_viewer((t.a + t.b + t.c) / 3.0)) <= 0.0
            };
            if faces {
                continue;
            }
            for &n in self.neighbors(index as usize) {
                if visited.insert(n) {
                    queue.push_back(n);
                }
            }
            let Some((uv_min, uv_max)) = uv_footprint_bounds(t, hit.position, reach_world) else {
                continue;
            };
            let min_x = 0.max((uv_min.x * wf - 0.5).ceil() as i32);
            let max_x = (width - 1).min((uv_max.x * wf - 0.5).floor() as i32);
            let min_y = 0.max((uv_min.y * hf - 0.5).ceil() as i32);
            let max_y = (height - 1).min((uv_max.y * hf - 0.5).floor() as i32);
            if max_x < min_x || max_y < min_y {
                continue;
            }
            let count = (max_x - min_x + 1) as i64 * (max_y - min_y + 1) as i64;
            if result.candidate_pixels as i64 + count > budget.max_candidate_pixels as i64 {
                stop = Some(DabRefusal::PixelBudget);
                break;
            }
            result.candidate_pixels += count as i32;
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let uv = Vec2::new((x as f32 + 0.5) / wf, (y as f32 + 0.5) / hf);
                    let Some(bary) = uv_barycentric(uv, t) else {
                        continue;
                    };
                    let position = mix3(t.a, t.b, t.c, bary);
                    let distance = magnitude(position - hit.position) / radius_world;
                    if distance >= cutoff {
                        continue;
                    }
                    candidates.push(DabCandidate {
                        triangle: index,
                        x,
                        y,
                        candidates_so_far: result.candidate_pixels,
                        bary,
                        position,
                        distance,
                    });
                }
            }
        }

        // 対称の側: 足跡はカメラによらない。連結・同じスロット・法線の向き・候補の画素の予算は守る
        if ignore_visibility {
            if let Some(why) = stop {
                return result.reject(why);
            }
            let mut merged: FastMap<i32, f32> = FastMap::default();
            for c in &candidates {
                let cov = match (cover, &sphere) {
                    (Some(f), _) => f(c.position, c.x, c.y),
                    (None, Some(e)) => {
                        let tt = texel_cache.entry(c.triangle).or_insert_with(|| {
                            e.texels(&self.triangles[c.triangle as usize], width, height)
                        });
                        e.cover_with(tt.as_ref(), c.position, hit.position, c.distance)
                    }
                    (None, None) => coverage(c.distance, hardness),
                };
                if sphere.is_some() && cov <= 0.0 {
                    continue;
                }
                let key = c.y * width + c.x;
                // C# の !TryGetValue || coverage > old
                if merged.get(&key).is_none_or(|&old| cov > old) {
                    merged.insert(key, cov);
                }
            }
            let mut keys: Vec<i32> = merged.keys().copied().collect();
            keys.sort_unstable();
            result.pixels = keys
                .into_iter()
                .map(|k| SurfacePixel {
                    x: k % width,
                    y: k / width,
                    coverage: merged[&k],
                    triangle: None,
                    position: Vec3::ZERO,
                })
                .collect();
            return result;
        }

        // 2・3. 組ごとに並列に撃ち、並びのとおりに予算を数えて受け入れる
        let rays = candidates
            .len()
            .min(budget.max_visibility_rays.max(0) as usize);
        let mut outcomes: Vec<RayOutcome> = Vec::with_capacity(rays.min(CHUNK));
        let mut cache = cache;
        if let Some(c) = cache.as_deref_mut() {
            c.prepare(self.revision, camera, width, height);
        }
        let key_of = |c: &DabCandidate| -> i64 {
            ((c.triangle as i64) << 31) | (c.y as i64 * width as i64 + c.x as i64)
        };
        let epsilon = self.visibility_epsilon;
        let (mut chunk_start, mut chunk_end) = (0usize, 0usize);
        let (mut tests, mut visits) = (0i64, 0i64);
        let mut pixels: FastMap<i32, SurfacePixel> = FastMap::default();
        let collected = result.candidate_pixels;
        for i in 0..candidates.len() {
            result.candidate_pixels = candidates[i].candidates_so_far;
            result.visibility_rays += 1;
            if result.visibility_rays > budget.max_visibility_rays {
                return result.reject(DabRefusal::VisibilityBudget);
            }
            if i >= chunk_end {
                chunk_start = i;
                let count = CHUNK.min(rays - i);
                chunk_end = i + count;
                let known = cache.as_deref().map(|c| &c.rays);
                let shoot = |k: usize| -> RayOutcome {
                    self.shoot(
                        &candidates[chunk_start + k],
                        camera,
                        epsilon,
                        budget,
                        known,
                        &key_of,
                    )
                };
                outcomes.clear();
                if count < 64 || rayon::current_num_threads() <= 1 {
                    outcomes.extend((0..count).map(shoot));
                } else {
                    (0..count)
                        .into_par_iter()
                        .with_min_len(MIN_RAYS_PER_TASK)
                        .map(shoot)
                        .collect_into_vec(&mut outcomes);
                }
                if let Some(c) = cache.as_deref_mut() {
                    for (k, o) in outcomes.iter().enumerate() {
                        if o.cached {
                            c.hits += 1;
                            continue;
                        }
                        if o.exceeded {
                            continue; // 予算で途中まで撃ったレイは覚えない
                        }
                        c.rays.insert(
                            key_of(&candidates[chunk_start + k]),
                            CachedRay {
                                skipped: o.skipped,
                                has_hit: o.has_hit,
                                hit_triangle: o.hit_triangle,
                                hit_distance: o.hit_distance,
                                camera_distance: o.camera_distance,
                                hit_position: o.hit_position,
                            },
                        );
                    }
                }
            }
            let o = outcomes[i - chunk_start];
            if o.skipped {
                continue;
            }
            tests += o.tests;
            visits += o.visits;
            result.ray_node_visits = visits;
            let exceeded = o.exceeded
                || tests > budget.max_ray_triangle_tests as i64
                || visits > budget.max_ray_node_visits as i64;
            result.ray_triangle_tests = tests.min(budget.max_ray_triangle_tests as i64 + 1) as i32;
            if exceeded {
                return result.reject(DabRefusal::BvhBudget);
            }
            let c = &candidates[i];
            if !o.has_hit || o.hit_distance < o.camera_distance - epsilon {
                continue;
            }
            if o.hit_triangle != c.triangle {
                // 距離の許しだけでは、ごく近い薄い服を透けてしまう。この三角形の辺の上で、実際に隣の三角形に当たったときだけ受け入れる
                if !self.at_edge(c.triangle, c.bary)
                    || sqr_magnitude(o.hit_position - c.position) > epsilon * epsilon
                    || !self
                        .neighbors(c.triangle as usize)
                        .contains(&o.hit_triangle)
                {
                    continue;
                }
            }
            let cov = match &sphere {
                Some(e) => {
                    let tt = texel_cache.entry(c.triangle).or_insert_with(|| {
                        e.texels(&self.triangles[c.triangle as usize], width, height)
                    });
                    e.cover_with(tt.as_ref(), c.position, hit.position, c.distance)
                }
                None => coverage(c.distance, hardness),
            };
            if sphere.is_some() && cov <= 0.0 {
                continue;
            }
            let key = c.y * width + c.x;
            if pixels.get(&key).is_none_or(|p| cov > p.coverage) {
                pixels.insert(
                    key,
                    SurfacePixel {
                        x: c.x,
                        y: c.y,
                        coverage: cov,
                        triangle: Some(c.triangle),
                        position: c.position,
                    },
                );
            }
        }
        result.candidate_pixels = collected;
        if let Some(why) = stop {
            return result.reject(why);
        }
        // 下の行からの決まった並びと、覆いの大きいほうの和で、共有の辺を二重に塗らない
        let mut keys: Vec<i32> = pixels.keys().copied().collect();
        keys.sort_unstable();
        result.pixels = keys.into_iter().map(|k| pixels[&k]).collect();
        result
    }

    /// 候補 1 つの遮蔽のレイ（覚えていればそれを返す）。
    fn shoot(
        &self,
        c: &DabCandidate,
        camera: Viewer,
        epsilon: f32,
        budget: &SurfaceBrushBudget,
        known: Option<&HashMap<i64, CachedRay>>,
        key_of: &impl Fn(&DabCandidate) -> i64,
    ) -> RayOutcome {
        if let Some(r) = known.and_then(|m| m.get(&key_of(c))) {
            return RayOutcome {
                cached: true,
                skipped: r.skipped,
                has_hit: r.has_hit,
                camera_distance: r.camera_distance,
                hit_triangle: r.hit_triangle,
                hit_distance: r.hit_distance,
                hit_position: r.hit_position,
                ..RayOutcome::default()
            };
        }
        // 透視はカメラの位置から点へ、正投影は点を通る視線の近い面の上から前の向きへ
        let (origin, direction, distance) = match camera {
            Viewer::Point(camera) => {
                let direction = c.position - camera;
                let distance = magnitude(direction);
                (camera, direction, distance)
            }
            Viewer::Parallel { forward, near } => {
                let distance = dot(c.position, forward) - near;
                (
                    c.position - forward * distance,
                    forward * distance,
                    distance,
                )
            }
        };
        if distance <= epsilon {
            return RayOutcome {
                skipped: true,
                ..RayOutcome::default()
            };
        }
        let mut work = RayQueryBudget {
            remaining_triangle_tests: budget.max_ray_triangle_tests,
            remaining_node_visits: budget.max_ray_node_visits,
            exceeded: false,
        };
        let ray = Ray::new(origin, direction / distance);
        let hit = self.raycast_internal(ray, false, distance + epsilon * 2.0, Some(&mut work));
        RayOutcome {
            has_hit: hit.is_some(),
            hit_triangle: hit.map_or(0, |h| h.triangle),
            hit_distance: hit.map_or(0.0, |h| h.distance),
            hit_position: hit.map_or(Vec3::ZERO, |h| h.position),
            camera_distance: distance,
            tests: budget.max_ray_triangle_tests as i64 - work.remaining_triangle_tests as i64,
            visits: budget.max_ray_node_visits as i64 - work.remaining_node_visits as i64,
            exceeded: work.exceeded,
            ..RayOutcome::default()
        }
    }
}
