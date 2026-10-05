//! 3D ビューのストローク（Unity 版の TexturePaintWindow の PaintAt の面の部分）: 画面の点で当て、ブラシの半径をモデルの大きさに
//! 合わせ、面の上のダブの画素を文書のストロークへ `apply_pixel` で塗る。
//!
//! - ブラシの半径（文書の画素）をモデルの単位に直す式は Unity 版と同じ: max(1e-6, 箱の対角線) × 半径 / 文書の幅（× 筆圧の応え）。
//! - ストロークの間はカメラもモデルも動かない前提（遮蔽の結果を覚えて、重なる次のダブで撃ち直さない）。
//! - ほかのテクスチャセット（マテリアルの組）の面に当たった点は塗らない。面に当たらない点も塗らない。
//! - ダブが予算を超えた・1 回の入力のダブが多すぎるときは `Err` を返す（呼ぶ側がストロークを取り消す。途中まで塗った画素も戻る）。
//! - ダブの縁の丸めで 0 以下になった覆いは塗らない（Unity 版はそれを ApplyPixel に渡して断られ、ストロークごと取り消していた）。
//! - 3D の対称（[`SurfaceSymmetrySetup`]）は、ダブの中心を面の上で映す・回すダブへ置き換え（`symmetry`）、全ての写しを画素ごとに
//!   大きい方の覆いで 1 つにして塗る。ぼかしは写しも含めて 1 つのダブとして読み元を凍結する。指先・クローンは写しごとの読み元と
//!   動きが要るので対称とは組めない（C# と同じ。ストロークの始めに断る）。
//! - 色の混ぜ（ストロークの `Brush` の `mix`）は、混ぜるならぼかしと同じく面のダブを `apply_dab` へ（下地を書く前に、UV の離れた島ごとの塊に分けて凍結する）。伸ばす
//!   （対称なし）は指先と同じ写像されたダブで、展開の図で UV の継ぎ目をまたいで前のダブの側を読む（最初のダブ・動いていないダブ・図に入らない
//!   ダブも、ずれ 0 で塗る）。対称と組むときは、写しごとの読み元が要るので `apply_dab`（動きの向きなし）へ。
//! - 効果のブラシ（[`SurfaceEffect`]）: ぼかしは面のダブを `apply_dab` へ。指先は直前のダブの面の点から今の点へ引きずり、クローンは
//!   固定した元の面の点から、ストロークの最初の面の点に対応させて写す。どちらも展開の図（[`super::SamplingChart`]）で UV の島の
//!   継ぎ目をまたいで読み元の画素を決め、全ての読みを書く前に凍結する（`Stroke::apply_mapped_dab`）。

use std::sync::Arc;

use glam::Vec2;

use glam::{DVec2, Quat, Vec3};

use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceBrushBudget, SurfaceDabResult, SurfaceVisibilityCache};
use super::sampling::SamplingError;
use super::stencil::SurfaceStencil;
use super::stroke::{ScreenStrokeSampler, TooManyDabs};
use super::symmetry::{build_expanded, MirrorOutcome, MirrorPlane, RadialSymmetry};
use super::unity::{dot, fmax, magnitude, sqr_magnitude};
use super::{SurfaceGeometry, SurfaceHit};
use crate::{
    BrushPixel, BrushSettings, CoreError, Document, MixMode, PressureResponse, StencilPoint,
    Stroke,
};

/// 3D のストロークを止めた理由（どれもストロークを取り消す）。
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceStrokeError {
    /// ダブが予算を超えた。
    Dab(DabRefusal),
    /// 1 回の入力のダブが多すぎる。
    TooManyDabs,
    /// 文書が断った（core はストロークを取り消してから返す）。
    Core(CoreError),
    /// 指先・クローンの読み元の画素を決められなかった（予算・探索の上限・図に入らない画素）。
    Sampling(SamplingError),
    /// 指先・クローンは 3D の対称と組めない。
    EffectWithSymmetry,
    /// クローンの元の面の点が、今のモデルの面ではない（モデルが替わった・別のテクスチャセット）。
    CloneSource,
}

impl std::fmt::Display for SurfaceStrokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceStrokeError::Dab(r) => r.fmt(f),
            SurfaceStrokeError::TooManyDabs => TooManyDabs.fmt(f),
            SurfaceStrokeError::Core(e) => e.fmt(f),
            SurfaceStrokeError::Sampling(e) => e.fmt(f),
            SurfaceStrokeError::EffectWithSymmetry => {
                f.write_str("指先・クローンは対称と一緒に使えません")
            }
            SurfaceStrokeError::CloneSource => {
                f.write_str("クローンの元が今のモデルの面ではありません")
            }
        }
    }
}

impl std::error::Error for SurfaceStrokeError {}

impl From<CoreError> for SurfaceStrokeError {
    fn from(e: CoreError) -> Self {
        SurfaceStrokeError::Core(e)
    }
}

impl From<SamplingError> for SurfaceStrokeError {
    fn from(e: SamplingError) -> Self {
        SurfaceStrokeError::Sampling(e)
    }
}

/// 3D の対称（ストロークの始めに固める。ストロークの間はモデルも設定も動かない）。ミラーと放射状は一緒に使え（鏡映を先に、回転を
/// 後に当てる）、どちらも無ければ対称なし。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSymmetrySetup {
    pub mirror: Option<MirrorPlane>,
    pub radial: Option<RadialSymmetry>,
    /// 写しの側は、カメラから見えない面にも塗る（元の側はいつも見える面だけ）。
    pub ignore_visibility: bool,
}

impl SurfaceSymmetrySetup {
    /// 写しがあるか。
    pub fn enabled(&self) -> bool {
        self.mirror.is_some() || self.radial.is_some()
    }
}

/// クローンの元（モデルの面の上の点）と、先の基準の点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceCloneSource {
    /// 写し元の面の点（Alt を押して決めた点）。
    pub source: SurfaceHit,
    /// 先の基準の点。None ならこのストロークの最初のダブの面の点（「揃える」ときは前のストロークの先をそのまま渡す）。
    pub destination: Option<SurfaceHit>,
}

/// 面のダブの効果。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SurfaceEffect {
    /// 色を塗る（消しゴムもこちら）。
    #[default]
    Paint,
    /// ぼかし（ストロークの `Brush` の効果も `Blur` にする）。
    Blur,
    /// 指先（直前のダブの面の点から今の点へ引きずる）。
    Smudge,
    /// クローン。
    Clone(SurfaceCloneSource),
}

/// 面のストロークの追加の設定（既定は、ステンシルも対称も効果も無し）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceStrokeOptions {
    /// ステンシルを通して塗るなら、その置き場（ストロークの `Brush` のステンシルと対）。
    pub stencil: Option<SurfaceStencil>,
    pub symmetry: Option<SurfaceSymmetrySetup>,
    pub effect: SurfaceEffect,
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
    /// 対称の写しを塗ったダブの数のべ（元を除く）。
    pub copies: usize,
    /// 指先が、直前の点から読めず（つながらない面）に飛ばしたダブ。
    pub lost: usize,
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
    /// 筆圧の応え（ストロークのブラシと同じもの）。大きさは、切っていない・応えが既定（筆圧そのもの）のとき None。硬さは切っているとき
    /// だけ持つ（既定の応えでも、硬さ × 筆圧になる）。
    size_response: Option<PressureResponse>,
    hardness_response: Option<PressureResponse>,
    /// 色の混ぜるブラシ（ストロークのブラシが混ぜ、色を塗る）: ダブの画素をまとめて渡し、下地を凍結して塗る（画素ごとには塗れない）。
    mixes: bool,
    /// 色の混ぜの伸ばす（対称なし）: 指先と同じく、直前のダブの面の点から今の点へ、展開の図で UV の継ぎ目をまたいで下地を読む。
    smears: bool,
    /// 色延び（伸ばすで、読む位置の遅れの長さに効く。1 + 2 × 色延び 倍）。
    stretch: f32,
    width: i32,
    height: i32,
    /// ステンシルを通して塗るなら、その置き場（ストロークの `Brush` のステンシルと対。無ければ画素ごとの点は渡さない）。
    stencil: Option<SurfaceStencil>,
    symmetry: Option<SurfaceSymmetrySetup>,
    effect: SurfaceEffect,
    /// 指先: 直前のダブの面の点。
    previous_hit: Option<SurfaceHit>,
    /// クローン: 先の基準の点（最初のダブの面の点か、揃えるなら前のストロークの先）。
    clone_destination: Option<SurfaceHit>,
    /// 対称の写しが塗られなかった理由の最後のもの（塗れた写しだけなら None。Painted は入れない）。
    symmetry_note: Option<MirrorOutcome>,
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
        Self::begin_with_options(
            doc,
            stroke,
            geometry,
            view,
            brush,
            material,
            at,
            pressure,
            SurfaceStrokeOptions::default(),
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
        Self::begin_with_options(
            doc,
            stroke,
            geometry,
            view,
            brush,
            material,
            at,
            pressure,
            SurfaceStrokeOptions {
                stencil,
                ..SurfaceStrokeOptions::default()
            },
        )
    }

    /// [`SurfaceStroke::begin`] に、ステンシル・3D の対称・効果を足したもの。効果を使うときは、文書のストロークも同じ効果の `Brush` で
    /// 始めておく（クローンの合成の参照元は、最初のダブの前に `Stroke::use_composite_clone_source` で決める）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_with_options(
        doc: &mut Document,
        stroke: &mut Stroke,
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        brush: &BrushSettings,
        material: Option<i32>,
        at: Vec2,
        pressure: f32,
        options: SurfaceStrokeOptions,
    ) -> Result<SurfaceStroke, SurfaceStrokeError> {
        let symmetry = options.symmetry.filter(|s| s.enabled());
        if symmetry.is_some()
            && matches!(
                options.effect,
                SurfaceEffect::Smudge | SurfaceEffect::Clone(_)
            )
        {
            return Err(SurfaceStrokeError::EffectWithSymmetry);
        }
        let clone_destination = match options.effect {
            SurfaceEffect::Clone(c) => {
                // 元が今のモデルの面でなければ、先に断る（世代違いの当たりで、ダブごとに断られ続けるのを避ける）
                let t = geometry.triangles().get(c.source.triangle as usize);
                if c.source.revision != geometry.revision()
                    || t.is_none_or(|t| material.is_some_and(|m| m != t.material))
                {
                    return Err(SurfaceStrokeError::CloneSource);
                }
                c.destination
            }
            _ => None,
        };
        let world_radius = world_radius(&geometry, brush.radius, doc.width());
        // 筆圧の応えは、文書のストロークのブラシ（始めたときに固定したもの）から取る。切り替えは渡された設定のもの
        let stroke_brush = stroke.brush(doc)?;
        let size_response = (brush.pressure_size && !stroke_brush.pressure.size.is_identity())
            .then(|| stroke_brush.pressure.size.clone());
        let hardness_response = stroke_brush
            .controls
            .pressure_hardness
            .then(|| stroke_brush.pressure.hardness.clone());
        let mixes = stroke_brush.mix.is_active()
            && stroke_brush.effect.is_paint()
            && !stroke_brush.base.erase;
        let smears = mixes && stroke_brush.mix.mode == MixMode::Smear && symmetry.is_none();
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
            size_response,
            hardness_response,
            mixes,
            smears,
            stretch: stroke_brush.mix.stretch as f32,
            width: doc.width() as i32,
            height: doc.height() as i32,
            stencil: options.stencil,
            symmetry,
            effect: options.effect,
            previous_hit: None,
            clone_destination,
            symmetry_note: None,
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

    /// クローンの先の基準の点（最初のダブで決まる。「揃える」ときは、ストロークが確定したらこれを次のストロークへ渡す）。
    pub fn clone_destination(&self) -> Option<SurfaceHit> {
        self.clone_destination
    }

    /// 対称の写しが塗られなかった最後の理由（知らせる文にする。全部塗れていれば None）。
    pub fn symmetry_note(&self) -> Option<MirrorOutcome> {
        self.symmetry_note
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
        // 筆圧を応えに通した大きさの係数（既定の応えは筆圧そのもの）
        let size_pressure = match &self.size_response {
            Some(r) => r.apply(pressure as f64) as f32,
            None => pressure,
        };
        if self.pressure_size && size_pressure <= 0.0 {
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
                fmax(0.001, size_pressure)
            } else {
                1.0
            };
        let hardness = match &self.hardness_response {
            Some(r) => (self.hardness as f64 * r.apply(pressure as f64)) as f32,
            None => self.hardness,
        };
        let dab = self.build_dab(&hit, radius, hardness);
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
        // ダブの縁では 1 − SmoothStep が単精度の丸めで −2.4e−7 などになる（C# の BuildSurfaceDabs も同じ値）。apply_pixel は
        // 0〜1 の外を断ってストロークを取り消すので、0 以下は塗らない（覆い 0 は何も変えないので、0 に丸めるのと同じ）
        let painted: Vec<&super::SurfacePixel> =
            dab.pixels.iter().filter(|p| p.coverage > 0.0).collect();
        let point_of = |this: &SurfaceStroke, p: &super::SurfacePixel| -> Option<StencilPoint> {
            match (this.stencil, footprint) {
                (Some(st), Some(footprint)) => Some(st.point(&this.view, p.position, footprint)),
                _ => None,
            }
        };
        match self.effect {
            SurfaceEffect::Paint if self.smears => {
                let points: Option<Vec<StencilPoint>> = painted
                    .iter()
                    .map(|p| point_of(self, p))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty());
                self.mapped_dab(doc, stroke, &hit, pressure, &painted, points)?;
            }
            SurfaceEffect::Paint if !self.mixes => {
                for p in &painted {
                    let coverage = p.coverage.min(1.0) as f64;
                    match point_of(self, p) {
                        Some(at) => stroke.apply_pixel_at(
                            doc,
                            p.x as i64,
                            p.y as i64,
                            coverage,
                            pressure as f64,
                            at,
                        )?,
                        None => stroke.apply_pixel(
                            doc,
                            p.x as i64,
                            p.y as i64,
                            coverage,
                            pressure as f64,
                        )?,
                    };
                }
            }
            // 色の混ぜ（ぼかしと同じく、ダブの画素をまとめて渡し、読み元を書く前に凍結する）
            SurfaceEffect::Paint | SurfaceEffect::Blur => {
                let pixels: Vec<BrushPixel> = painted
                    .iter()
                    .map(|p| BrushPixel {
                        x: p.x as i64,
                        y: p.y as i64,
                        coverage: p.coverage.min(1.0) as f64,
                    })
                    .collect();
                let center = DVec2::new(
                    hit.uv.x as f64 * self.width as f64,
                    hit.uv.y as f64 * self.height as f64,
                );
                let points: Option<Vec<StencilPoint>> = painted
                    .iter()
                    .map(|p| point_of(self, p))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty());
                match &points {
                    Some(points) => {
                        stroke.apply_dab_at(doc, &pixels, center, pressure as f64, points)?
                    }
                    None => stroke.apply_dab(doc, &pixels, center, pressure as f64)?,
                };
            }
            SurfaceEffect::Smudge | SurfaceEffect::Clone(_) => {
                let points: Option<Vec<StencilPoint>> = painted
                    .iter()
                    .map(|p| point_of(self, p))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty());
                self.mapped_dab(doc, stroke, &hit, pressure, &painted, points)?;
            }
        }
        self.stats.pixels += dab.pixels.len();
        Ok(())
    }

    /// ダブの画素（元と写しを 1 つにしたもの）を作る。対称があれば、写しも面へ投げ直して合わせる。
    fn build_dab(&mut self, hit: &SurfaceHit, radius: f32, hardness: f32) -> SurfaceDabResult {
        let Some(sym) = self.symmetry else {
            return self.geometry.build_surface_dabs(
                hit,
                radius,
                self.width,
                self.height,
                self.view.position,
                hardness,
                &self.budget,
                Some(&mut self.cache),
                false,
            );
        };
        let expanded = build_expanded(
            &self.geometry,
            hit,
            sym.mirror.as_ref(),
            sym.radial.as_ref(),
            sym.ignore_visibility,
            radius,
            self.width,
            self.height,
            self.view.position,
            hardness,
            &self.budget,
            Some(&mut self.cache),
        );
        self.stats.copies += expanded.copies.len();
        if matches!(
            expanded.outcome,
            MirrorOutcome::NoSurface | MirrorOutcome::OtherSlot | MirrorOutcome::Hidden
        ) {
            self.symmetry_note = Some(expanded.outcome);
        }
        expanded.result
    }

    /// 指先・クローンの 1 つのダブ（C# の ApplySurfaceEffectCore）: 先の面の点の展開の図と、元の面の点の展開の図で、ダブの画素ごとの
    /// 読み元を決め、全ての読みを凍結して塗る。
    fn mapped_dab(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        hit: &SurfaceHit,
        pressure: f32,
        pixels: &[&super::SurfacePixel],
        points: Option<Vec<StencilPoint>>,
    ) -> Result<(), SurfaceStrokeError> {
        let smears = matches!(self.effect, SurfaceEffect::Paint);
        let smudge = matches!(self.effect, SurfaceEffect::Smudge) || smears;
        // 伸ばすは、前の打点への動きを (1 + 2 × 色延び) 倍だけ後ろを読む（長さは打点の直径まで）
        let reach_scale = if smears { 1.0 + 2.0 * self.stretch } else { 1.0 };
        let (destination, source) = match self.effect {
            SurfaceEffect::Smudge => {
                // 最初のダブは位置を覚えるだけ。動いていなければ、覚え直すだけで塗らない
                let Some(previous) = self.previous_hit.replace(*hit) else {
                    return Ok(());
                };
                if sqr_magnitude(previous.position - hit.position) < 1e-20 {
                    return Ok(());
                }
                (*hit, previous)
            }
            // 色の混ぜの伸ばす: 指先と同じ読み方だが、塗るブラシなので、最初のダブ・動いていないダブも塗る（読む位置のずれは 0）
            SurfaceEffect::Paint => (*hit, self.previous_hit.replace(*hit).unwrap_or(*hit)),
            SurfaceEffect::Clone(c) => {
                let destination = *self.clone_destination.get_or_insert(*hit);
                (destination, c.source)
            }
            _ => return Ok(()),
        };
        // 面の来歴と全画素の参照を予算に数え、書く前にまとめて凍結する
        let targets = doc.active_stroke_stats().map_or(1, |s| s.targets.max(1)) as i64;
        let per_pixel = 160 + targets * 4 + if points.is_some() { 24 } else { 0 };
        let mut bytes = pixels.len() as i64 * 28;
        let rollback = doc.active_stroke_stats().map_or(0, |s| s.rollback_bytes) as i64;
        let mut available = doc.stroke_budget_bytes().min(i64::MAX as u64) as i64
            - rollback
            - bytes
            - pixels.len() as i64 * per_pixel;
        if available <= 0 {
            return Err(SamplingError::ChartBudget.into());
        }
        let radius = self.world_radius;
        let reach = radius * 2.0
            + magnitude(source.position - destination.position)
                * if smudge { 2.0 * reach_scale } else { 0.0 }
            + magnitude(hit.position - destination.position) * 2.0;
        let geometry = self.geometry.clone();
        let mut dest_chart = geometry.build_sampling_chart(
            &destination,
            reach,
            Vec3::ZERO,
            self.budget.max_triangles,
            available,
        )?;
        bytes += dest_chart.nominal_bytes();
        available -= dest_chart.nominal_bytes();
        let mut source_chart = None;
        let mut offset = glam::Vec2::ZERO;
        if smudge {
            match dest_chart.coordinates(&source) {
                Some(o) => offset = o,
                // 直前の点が展開の図に入らない（つながらない面に移った）: 指先はこのダブは塗らず、今の点から拾い直す。伸ばすは塗るブラシなので、
                // ずれ 0（同じ画素を読む）で塗る
                None if smears => offset = glam::Vec2::ZERO,
                None => {
                    self.stats.lost += 1;
                    return Ok(());
                }
            }
            if smears {
                offset *= reach_scale;
                let length = offset.length();
                if length > radius * 2.0 {
                    offset *= radius * 2.0 / length;
                }
            }
        } else {
            let mut tangent = project_on_plane(Vec3::X, destination.normal);
            if sqr_magnitude(tangent) < 1e-12 {
                tangent = project_on_plane(Vec3::Y, destination.normal);
            }
            if let (Some(from), Some(to)) = (
                destination.normal.try_normalize(),
                source.normal.try_normalize(),
            ) {
                tangent = Quat::from_rotation_arc(from, to) * tangent;
            }
            let chart = geometry.build_sampling_chart(
                &source,
                reach,
                tangent,
                self.budget.max_triangles,
                available,
            )?;
            bytes += chart.nominal_bytes();
            source_chart = Some(chart);
        }
        let mut plan = Vec::with_capacity(pixels.len());
        let mut kept = Vec::new();
        for (i, p) in pixels.iter().enumerate() {
            let point = dest_chart
                .pixel_coordinates(p)
                .ok_or(SamplingError::Unreachable)?;
            let pixel = BrushPixel {
                x: p.x as i64,
                y: p.y as i64,
                coverage: p.coverage.min(1.0) as f64,
            };
            let mapped = match source_chart.as_mut() {
                Some(chart) => chart.try_sample(point + offset, pixel, self.width, self.height)?,
                None => dest_chart.try_sample(point + offset, pixel, self.width, self.height)?,
            };
            if let Some(mapped) = mapped {
                plan.push(mapped);
                kept.push(i);
            }
        }
        let kept_points: Option<Vec<StencilPoint>> =
            points.map(|v| kept.iter().map(|&i| v[i]).collect());
        stroke.apply_mapped_dab(
            doc,
            &plan,
            pressure as f64,
            bytes.max(0) as u64,
            kept_points.as_deref(),
        )?;
        Ok(())
    }
}

/// Unity の `Vector3.ProjectOnPlane`。
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    let sqr = dot(normal, normal);
    if sqr < f32::MIN_POSITIVE {
        return v;
    }
    let d = dot(v, normal);
    Vec3::new(
        v.x - normal.x * d / sqr,
        v.y - normal.y * d / sqr,
        v.z - normal.z * d / sqr,
    )
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
