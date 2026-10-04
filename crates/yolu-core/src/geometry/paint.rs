//! 3D ビューのストローク（Unity 版の TexturePaintWindow の PaintAt の面の部分）: 画面の点で当て、ブラシの半径をモデルの大きさに
//! 合わせ、面の上のダブの画素を文書のストロークへ `apply_pixel` で塗る。
//!
//! - ブラシの半径（文書の画素）をモデルの単位に直す式は Unity 版と同じ: max(1e-6, 箱の対角線) × 半径 / 文書の幅（× 筆圧）。
//! - ストロークの間はカメラもモデルも動かない前提（遮蔽の結果を覚えて、重なる次のダブで撃ち直さない）。
//! - ほかのテクスチャセット（マテリアルの組）の面に当たった点は塗らない。面に当たらない点も塗らない。
//! - ダブが予算を超えた・1 回の入力のダブが多すぎるときは `Err` を返す（呼ぶ側がストロークを取り消す。途中まで塗った画素も戻る）。
//! - ダブの縁の丸めで 0 以下になった覆いは塗らない（Unity 版はそれを ApplyPixel に渡して断られ、ストロークごと取り消していた）。

use std::sync::Arc;

use glam::Vec2;

use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceBrushBudget, SurfaceVisibilityCache};
use super::stencil::SurfaceStencil;
use super::stroke::{ScreenStrokeSampler, TooManyDabs};
use super::unity::fmax;
use super::{SurfaceGeometry, SurfaceHit};
use crate::{BrushSettings, CoreError, Document, Stroke};

/// 3D のストロークを止めた理由（どれもストロークを取り消す）。
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceStrokeError {
    /// ダブが予算を超えた。
    Dab(DabRefusal),
    /// 1 回の入力のダブが多すぎる。
    TooManyDabs,
    /// 文書が断った（core はストロークを取り消してから返す）。
    Core(CoreError),
}

impl std::fmt::Display for SurfaceStrokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceStrokeError::Dab(r) => r.fmt(f),
            SurfaceStrokeError::TooManyDabs => TooManyDabs.fmt(f),
            SurfaceStrokeError::Core(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SurfaceStrokeError {}

impl From<CoreError> for SurfaceStrokeError {
    fn from(e: CoreError) -> Self {
        SurfaceStrokeError::Core(e)
    }
}

/// ストロークの数（試験・知らせ用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceStrokeStats {
    /// 置いたダブ（面に当たって画素を集めたもの）。
    pub dabs: usize,
    /// 塗った画素の延べ数。
    pub pixels: usize,
    /// 面に当たらない・ほかのテクスチャセットの面で飛ばした点。
    pub missed: usize,
    /// 世代違いなどで作らなかったダブ（予算以外の理由）。
    pub refused: usize,
}

/// 進行中の 3D のストローク（文書のストロークの札と一緒に持つ）。
pub struct SurfaceStroke {
    geometry: Arc<SurfaceGeometry>,
    view: CameraView,
    material: Option<i32>,
    sampler: ScreenStrokeSampler,
    cache: SurfaceVisibilityCache,
    budget: SurfaceBrushBudget,
    /// 筆圧を掛ける前のモデルの単位の半径。
    world_radius: f32,
    hardness: f32,
    spacing: f32,
    pressure_size: bool,
    width: i32,
    height: i32,
    /// ステンシルを通して塗るなら、その置き場（ストロークの `Brush` のステンシルと対。無ければ画素ごとの点は渡さない）。
    stencil: Option<SurfaceStencil>,
    pub stats: SurfaceStrokeStats,
    /// 最後に知らせたい理由（予算以外で作らなかったダブ）。
    pub note: Option<DabRefusal>,
}

impl SurfaceStroke {
    /// ストロークを始め、押した点に 1 つ目のダブを置く。material は描くテクスチャセット（None ならどの面にも描く）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        doc: &mut Document,
        stroke: &mut Stroke,
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        brush: &BrushSettings,
        material: Option<i32>,
        at: Vec2,
        pressure: f32,
    ) -> Result<SurfaceStroke, SurfaceStrokeError> {
        Self::begin_with_stencil(
            doc, stroke, geometry, view, brush, material, at, pressure, None,
        )
    }

    /// [`SurfaceStroke::begin`] に、ステンシルの置き場を足したもの。ストロークの `Brush` がステンシルを持つときは、ここで置き場を渡す
    /// （渡さないと、画素ごとにステンシルの上の点が無いので文書が断る）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_with_stencil(
        doc: &mut Document,
        stroke: &mut Stroke,
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        brush: &BrushSettings,
        material: Option<i32>,
        at: Vec2,
        pressure: f32,
        stencil: Option<SurfaceStencil>,
    ) -> Result<SurfaceStroke, SurfaceStrokeError> {
        let world_radius = world_radius(&geometry, brush.radius, doc.width());
        let mut s = SurfaceStroke {
            geometry,
            view,
            material,
            sampler: ScreenStrokeSampler::new(at, pressure),
            cache: SurfaceVisibilityCache::new(),
            budget: SurfaceBrushBudget::default(),
            world_radius,
            hardness: brush.hardness as f32,
            spacing: brush.spacing as f32,
            pressure_size: brush.pressure_size,
            width: doc.width() as i32,
            height: doc.height() as i32,
            stencil,
            stats: SurfaceStrokeStats::default(),
            note: None,
        };
        s.paint_at(doc, stroke, at, pressure)?;
        Ok(s)
    }

    /// ダブの予算を変える（試験用）。
    pub fn set_budget(&mut self, budget: SurfaceBrushBudget) {
        self.budget = budget;
    }

    /// 遮蔽の結果を覚えた数・使った数（試験用）。
    pub fn cache(&self) -> &SurfaceVisibilityCache {
        &self.cache
    }

    /// 新しい入力の点（画面の座標。区間を描けるようになった分だけダブを置く）。
    pub fn add(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        at: Vec2,
        pressure: f32,
    ) -> Result<(), SurfaceStrokeError> {
        let mut points = Vec::new();
        let (geometry, view, radius, spacing) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.spacing,
        );
        self.sampler
            .add(
                at,
                pressure,
                |a| gap(&geometry, &view, radius, spacing, a),
                &mut points,
            )
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        for (p, pressure) in points {
            self.paint_at(doc, stroke, p, pressure)?;
        }
        Ok(())
    }

    /// 離したとき: 待たせている最後の区間を描く（この後に文書のストロークを確定する）。
    pub fn finish(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
    ) -> Result<(), SurfaceStrokeError> {
        let mut points = Vec::new();
        let (geometry, view, radius, spacing) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.spacing,
        );
        self.sampler
            .finish(|a| gap(&geometry, &view, radius, spacing, a), &mut points)
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        for (p, pressure) in points {
            self.paint_at(doc, stroke, p, pressure)?;
        }
        Ok(())
    }

    /// 1 つのダブ（C# の PaintAt の面の経路）。
    fn paint_at(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        at: Vec2,
        pressure: f32,
    ) -> Result<(), SurfaceStrokeError> {
        if self.pressure_size && pressure <= 0.0 {
            return Ok(());
        }
        let Some(hit) = pick(&self.geometry, &self.view, at) else {
            self.stats.missed += 1;
            return Ok(());
        };
        if self.material.is_some_and(|m| m != hit.material) {
            self.stats.missed += 1;
            return Ok(());
        }
        let radius = self.world_radius
            * if self.pressure_size {
                fmax(0.001, pressure)
            } else {
                1.0
            };
        let dab = self.geometry.build_surface_dabs(
            &hit,
            radius,
            self.width,
            self.height,
            self.view.position,
            self.hardness,
            &self.budget,
            Some(&mut self.cache),
            false,
        );
        if let Some(why) = dab.refusal {
            if why.cancels_stroke() {
                return Err(SurfaceStrokeError::Dab(why));
            }
            self.stats.refused += 1;
            self.note = Some(why);
            return Ok(());
        }
        self.stats.dabs += 1;
        let footprint = self
            .stencil
            .map(|st| st.footprint(&self.geometry, &self.view, &hit, self.width, self.height));
        for p in &dab.pixels {
            // ダブの縁では 1 − SmoothStep が単精度の丸めで −2.4e−7 などになる（C# の BuildSurfaceDabs も同じ値）。apply_pixel は
            // 0〜1 の外を断ってストロークを取り消すので、0 以下は塗らない（覆い 0 は何も変えないので、0 に丸めるのと同じ）
            if p.coverage <= 0.0 {
                continue;
            }
            let coverage = p.coverage.min(1.0) as f64;
            match (self.stencil, footprint) {
                (Some(st), Some(footprint)) => stroke.apply_pixel_at(
                    doc,
                    p.x as i64,
                    p.y as i64,
                    coverage,
                    pressure as f64,
                    st.point(&self.view, p.position, footprint),
                )?,
                _ => stroke.apply_pixel(doc, p.x as i64, p.y as i64, coverage, pressure as f64)?,
            };
        }
        self.stats.pixels += dab.pixels.len();
        Ok(())
    }
}

/// 画面の点の下の面（表の面だけ。表示域の外は None。Unity 版の TryPick）。
pub fn pick(geometry: &SurfaceGeometry, view: &CameraView, at: Vec2) -> Option<SurfaceHit> {
    if !(at.x >= 0.0 && at.x < view.width && at.y >= 0.0 && at.y < view.height) {
        return None;
    }
    geometry.raycast(view.ray(at), true, f32::INFINITY)
}

/// ブラシの半径（文書の画素）のモデルの単位での大きさ（筆圧の前。Unity 版と同じ式: max(1e-6, 箱の対角線) × 半径 / 文書の幅。
/// 箱の対角線は `brush_scale`（ポーズを付けたスナップショットでも元の形の値）。
pub fn world_radius(geometry: &SurfaceGeometry, brush_radius: f64, document_width: u32) -> f32 {
    fmax(0.000001, geometry.brush_scale()) * brush_radius as f32 / document_width as f32
}

/// 区間の始まりの点でのダブの間隔（面の上の直径 × 間隔を画面に直す。0.5 以上。面に当たらなければ 1）。
fn gap(
    geometry: &SurfaceGeometry,
    view: &CameraView,
    world_radius: f32,
    spacing: f32,
    a: Vec2,
) -> f32 {
    match pick(geometry, view, a) {
        Some(hit) => fmax(
            0.5,
            2.0 * view.world_radius_to_screen(hit.position, world_radius) * spacing,
        ),
        None => 1.0,
    }
}
