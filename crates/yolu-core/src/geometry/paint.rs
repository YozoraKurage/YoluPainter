//! 3D ビューのストローク: 画面の点の円の中の、カメラから見える面のテクセルを、投影の塗り（[`SurfaceProjector`]）で集めて、文書の
//! ストロークへ `apply_pixel` で塗る。
//!
//! - ブラシの半径（文書の画素）をモデルの単位に直す式: max(1e-6, 箱の対角線) × 半径 / 文書の幅（× 筆圧の応え）。画面の円の半径は、
//!   中心の下の塗るテクスチャセットの面の奥行きで、その半径を画面へ直したもの。中心がほかのセットの面・背景にあるダブは、直前に
//!   面に当たったダブの奥行きで直す（縁で途切れない）。まだ一度も面に当たっていなければ塗らない。
//! - ストロークの間はカメラもモデルも動かない前提（区画の投影の画素を覚えて、重なる次のダブで作り直さない）。
//! - 投影の画素の覚えは 1 回の操作のメモリ（文書のストロークの予算から巻き戻しの分を除いたもの）に収め、超えたら長く使っていない
//!   区画から捨てる（要るときに同じものを作り直す）。元の側と対称の写しの投影の塗りは 1 つの予算を分け、どれの区画でも古いものから
//!   捨てる（元の側が覚えを溜めても、後から作る写しが締め出されない）。1 つのダブに要る区画だけで入らないときは、そのダブ（写しなら
//!   その写し）を飛ばして理由を `note` に残す（ストロークは取り消さない）。1 回の入力のダブが多すぎるときと、指先・クローン・伸ばす・
//!   色の混ぜの読み元が 1 回の操作のメモリに入らないときは `Err`（呼ぶ側がストロークを取り消す）。
//! - 3D の対称（[`SurfaceSymmetrySetup`]）: 写しの点が、向きの合う同じテクスチャセットの面の近くにあるときだけ、写しの側を塗る。
//!   写しの点は、中心の下の面の点か、中心がほかのセット・背景にあるダブでは元の側の画素の点を写して探す。
//!   見えない面にも塗らない設定では、写したカメラ（鏡映・回転したカメラ）から同じ画面の円で投影の塗りをする（元の側の見え方を写した
//!   ものになる）。見えない面にも塗る設定では、写しの点のまわりの球の中の面（カメラによらない足跡）を塗る。全ての写しを画素ごとに
//!   大きい方の覆いで 1 つにして塗る。写しが作れなかったときは、その写しだけ飛ばして知らせる。ぼかしは写しも含めて 1 つのダブとして
//!   読み元を凍結する。指先・クローンは写しごとの読み元と動きが要るので対称とは組めない（ストロークの始めに断る）。
//! - 色の混ぜ（ストロークの `Brush` の `mix`）は、混ぜるならぼかしと同じく面のダブを `apply_dab` へ（下地を書く前に、UV の離れた島ごとの塊に分けて凍結する）。伸ばす
//!   （対称なし）は指先と同じ写像されたダブで、展開の図で UV の継ぎ目をまたいで前のダブの側を読む（最初のダブ・動いていないダブ・図に入らない
//!   ダブも、ずれ 0 で塗る）。対称と組むときは、写しごとの読み元が要るので `apply_dab`（動きの向きなし）へ。
//! - 効果のブラシ（[`SurfaceEffect`]）: ぼかしは画素ごとに面の上のまわりの 4 点を読み元にする。指先は直前のダブの面の点から今の点へ
//!   引きずり、クローンは固定した元の面の点から、ストロークの最初の面の点に対応させて写す。どれも展開の図（[`super::SamplingChart`]）で
//!   UV の島の継ぎ目をまたいで読み元の画素を決め、全ての読みを書く前に凍結する（`Stroke::apply_mapped_dab`）。画面の円に入った、図に
//!   つながらない面の画素は、指先・クローンでは塗らず、ぼかしでは UV の画像の上で読む。ぼかしは、図を作れないとき（1 回の操作の
//!   メモリ・参照を探す回数）も UV の画像の上の箱の平均で塗る（取り消さない）。
//! - 画面の円の半径と中心の下の面の点は、カメラ（[`CameraView`]）の単精度の tan で出す（投影の塗りの中は四則と平方根だけ）。

use std::sync::Arc;

use glam::Vec2;

use glam::{DVec2, Quat, Vec3};

use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceBrushBudget, SurfaceDabResult};
use super::project::{
    evict_least_recent, CopyTransform, ProjectionSettings, ProjectionStats, SurfaceProjector,
};
use super::sampling::SamplingError;
use super::stencil::SurfaceStencil;
use super::stroke::{ScreenStrokeSampler, TooManyDabs};
use super::symmetry::{find_copy, union_dabs, CopyHit, MirrorOutcome, MirrorPlane, RadialSymmetry};
use super::unity::{dot, fmax, magnitude, sqr_magnitude};
use super::{SurfaceGeometry, SurfaceHit};
use crate::brush::{BrushMappedPixel, BrushSourceTap};
use crate::{
    BrushPixel, BrushSettings, CoreError, Document, MixMode, PressureResponse, StencilPoint, Stroke,
};

/// 3D のストロークを止めた理由（どれもストロークを取り消す）。
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceStrokeError {
    /// ダブを作れなかった（投影の塗りの準備が範囲外の値を断った）。
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
    /// 写しの側は、カメラから見えない面にも塗る（元の側は投影の塗りの切り替えのまま）。
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

/// 面のストロークの追加の設定（既定は、ステンシルも対称も効果も無く、投影の塗りは既定の切り替え）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceStrokeOptions {
    /// ステンシルを通して塗るなら、その置き場（ストロークの `Brush` のステンシルと対）。
    pub stencil: Option<SurfaceStencil>,
    pub symmetry: Option<SurfaceSymmetrySetup>,
    pub effect: SurfaceEffect,
    /// 投影の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ）。
    pub projection: ProjectionSettings,
    /// 投影の塗りに使ってよいバイト（試験用。None なら文書の 1 回の操作の予算から巻き戻しの分を引いたもの。
    /// [`SurfaceStroke::set_projection_memory`] と同じものを、最初のダブから効かせる）。
    pub projection_memory: Option<u64>,
}

/// ストロークの数（試験・知らせ用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceStrokeStats {
    /// 置いたダブ（画素を集めたもの）。
    pub dabs: usize,
    /// 塗った画素の延べ数。
    pub pixels: usize,
    /// 飛ばした点（まだ面に当たっていない・面の上の点が要る効果で、中心が塗るセットの面に無い）。
    pub missed: usize,
    /// 作らなかったダブ・写し（メモリに入らない区画・写しの側の上限など。理由は `note`）。
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
    /// 見えない面にも塗る写し（カメラによらない足跡）の、ダブごとの上限。
    budget: SurfaceBrushBudget,
    projection: ProjectionSettings,
    /// 元の側の投影の塗り（最初に面に当たったダブで作る）。
    projector: Option<SurfaceProjector>,
    /// 対称の写しの投影の塗り（写しの番号 × 2 ＋ 鏡映。使うときに作る）。
    copy_projectors: Vec<Option<SurfaceProjector>>,
    /// ダブの時刻（投影の塗りの区画に付けて、全部の投影の塗りで古い区画から捨てる）。
    clock: u64,
    /// モデルの単位 1 が画面で何単位か（直前に塗るセットの面に当たった所の奥行きで）。
    screen_scale: Option<f32>,
    /// 投影の塗りに使ってよいバイト（試験用。None なら文書の 1 回の操作の予算から巻き戻しの分を引いたもの）。
    memory_override: Option<u64>,
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
    /// ぼかしの半径（文書の画素。ぼかしのブラシのときだけ使う）と、ダブごとに回す 4 点の参照の向きの番号。
    blur_radius: u32,
    blur_turn: usize,
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
    /// 最後に知らせたい理由（作らなかったダブ・写し）。
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
            budget: SurfaceBrushBudget::default(),
            projection: options.projection.sanitized(),
            projector: None,
            copy_projectors: Vec::new(),
            clock: 0,
            screen_scale: None,
            memory_override: options.projection_memory,
            world_radius,
            hardness: brush.hardness as f32,
            spacing: brush.spacing as f32,
            pressure_size: brush.pressure_size,
            size_response,
            hardness_response,
            mixes,
            smears,
            stretch: stroke_brush.mix.stretch as f32,
            blur_radius: match stroke_brush.effect {
                crate::BrushEffect::Blur { radius } => radius,
                _ => 3,
            },
            blur_turn: 0,
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

    /// 見えない面にも塗る写しの、ダブごとの上限を変える（試験用）。
    pub fn set_budget(&mut self, budget: SurfaceBrushBudget) {
        self.budget = budget;
    }

    /// 投影の塗りに使ってよいバイトを決める（試験用。区画の一覧・UV の覆い・覚えた投影の画素の合計。None で文書の予算に戻す）。
    pub fn set_projection_memory(&mut self, bytes: Option<u64>) {
        self.memory_override = bytes;
    }

    /// 元の側の投影の塗りの覚えの数（試験用。まだ作っていなければ既定値）。
    pub fn projection_stats(&self) -> ProjectionStats {
        self.projector
            .as_ref()
            .map_or_else(ProjectionStats::default, |p| p.stats())
    }

    /// 投影の塗りが今持っているバイト（区画の一覧・UV の覆い・覚えた投影の画素。写しの分も）。
    pub fn projection_bytes(&self) -> u64 {
        self.projectors()
            .map(|p| p.fixed_bytes() + p.cached_bytes())
            .sum::<u64>()
            + self.projector.as_ref().map_or(0, |p| p.shared_bytes())
    }

    /// 投影の塗りのうち、ストロークの間ずっと持つ一覧（共有の一覧と、元の側・写しの区画の一覧）のバイト。
    pub fn projection_fixed_bytes(&self) -> u64 {
        self.projector.as_ref().map_or(0, |p| p.shared_bytes())
            + self.projectors().map(|p| p.fixed_bytes()).sum::<u64>()
    }

    fn projectors(&self) -> impl Iterator<Item = &SurfaceProjector> {
        self.projector
            .iter()
            .chain(self.copy_projectors.iter().flatten())
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
        let (geometry, view, radius, spacing, scale) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.spacing,
            self.screen_scale,
        );
        self.sampler
            .add(
                at,
                pressure,
                |a| gap(&geometry, &view, radius, spacing, scale, a),
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
        let (geometry, view, radius, spacing, scale) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.spacing,
            self.screen_scale,
        );
        self.sampler
            .finish(
                |a| gap(&geometry, &view, radius, spacing, scale, a),
                &mut points,
            )
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        for (p, pressure) in points {
            self.paint_at(doc, stroke, p, pressure)?;
        }
        Ok(())
    }

    /// 1 つのダブ。
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
        let inside =
            at.x >= 0.0 && at.x < self.view.width && at.y >= 0.0 && at.y < self.view.height;
        let hit = pick(&self.geometry, &self.view, at)
            .filter(|h| self.material.is_none_or(|m| m == h.material));
        if let Some(h) = &hit {
            let scale = self.view.world_radius_to_screen(h.position, 1.0);
            if scale.is_finite() && scale > 0.0 {
                self.screen_scale = Some(scale);
            }
        }
        // 指先・クローン・色の混ぜの伸ばすは、面の上の中心から読み元を決めるので、中心が塗るセットの面に無いダブは飛ばす
        let needs_hit = matches!(self.effect, SurfaceEffect::Smudge | SurfaceEffect::Clone(_))
            || (self.smears && matches!(self.effect, SurfaceEffect::Paint));
        let Some(scale) = self
            .screen_scale
            .filter(|_| inside && (hit.is_some() || !needs_hit))
        else {
            self.stats.missed += 1;
            return Ok(());
        };
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
        let (dab, hit) = self.build_dab(doc, hit, at, scale, radius, hardness);
        if let Some(why) = dab.refusal {
            self.stats.refused += 1;
            self.note = Some(why);
            return Ok(());
        }
        let Some(hit) = hit else {
            return Ok(());
        };
        self.stats.dabs += 1;
        let footprint = self
            .stencil
            .map(|st| st.footprint(&self.geometry, &self.view, &hit, self.width, self.height));
        // apply_pixel は覆いが 0〜1 の外だとストロークを取り消すので、0 以下は塗らない（覆い 0 は何も変えない）
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
            SurfaceEffect::Blur => {
                let points: Option<Vec<StencilPoint>> = painted
                    .iter()
                    .map(|p| point_of(self, p))
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty());
                self.blur_dab(doc, stroke, &hit, pressure, &painted, points)?;
            }
            // 色の混ぜ（ダブの画素をまとめて渡し、読み元を書く前に凍結する）
            SurfaceEffect::Paint => {
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

    /// 中心が塗るセットの面に無いダブで、面の点の代わりにする当たり（覆いのいちばん大きい画素の三角形の上の点）。
    fn fallback_hit(&self, dab: &SurfaceDabResult) -> Option<SurfaceHit> {
        let best = dab
            .pixels
            .iter()
            .filter(|p| p.triangle.is_some())
            .max_by(|a, b| a.coverage.total_cmp(&b.coverage))?;
        let triangle = best.triangle?;
        let t = self.geometry.triangles().get(triangle as usize)?;
        let barycentric = super::query::barycentric(best.position, t);
        Some(SurfaceHit {
            revision: self.geometry.revision(),
            renderer: t.renderer,
            material_slot: t.material_slot,
            material: t.material,
            triangle,
            position: best.position,
            normal: t.normal(),
            barycentric,
            uv: t.uv_a * barycentric.x + t.uv_b * barycentric.y + t.uv_c * barycentric.z,
            distance: 0.0,
        })
    }

    /// 投影の塗りに使ってよいバイト（共有の一覧・区画の一覧・覚えた投影の画素の合計。元の側と写しで 1 つ）。
    fn memory_total(&self, doc: &Document) -> u64 {
        self.memory_override.unwrap_or_else(|| {
            let rollback = doc.active_stroke_stats().map_or(0, |s| s.rollback_bytes);
            doc.stroke_budget_bytes().saturating_sub(rollback)
        })
    }

    /// 今のダブで、`except`（None は元の側、Some(i) は写しの i 番）の投影の塗りの区画に使ってよいバイト: 全体から、共有の一覧・全部の
    /// 区画の一覧と、ほかの投影の塗りがこのダブで使った区画を引いたもの。前のダブの区画は捨てて空けるので引かない（どの投影の塗りの
    /// 区画でも、古いものから捨てる）。
    fn room(&self, total: u64, except: Option<usize>) -> u64 {
        let clock = self.clock;
        let mut used = self.projector.as_ref().map_or(0, |p| {
            p.shared_bytes()
                + p.fixed_bytes()
                + if except.is_none() {
                    0
                } else {
                    p.bytes_used_at(clock)
                }
        });
        for (i, p) in self.copy_projectors.iter().enumerate() {
            if let Some(p) = p {
                used += p.fixed_bytes()
                    + if except == Some(i) {
                        0
                    } else {
                        p.bytes_used_at(clock)
                    };
            }
        }
        total.saturating_sub(used)
    }

    /// 全部の投影の塗りの覚えを、予算から一覧の分を引いた残りに収める（このダブで使った区画は残す）。
    fn evict(&mut self, total: u64) {
        let lists = self.projection_fixed_bytes();
        let clock = self.clock;
        let mut all: Vec<&mut SurfaceProjector> = self
            .projector
            .iter_mut()
            .chain(self.copy_projectors.iter_mut().flatten())
            .collect();
        evict_least_recent(&mut all, total.saturating_sub(lists), clock);
    }

    /// ダブの画素（元と写しを 1 つにしたもの）と、ダブの面の点を作る。at・scale は画面の中心と、モデルの単位 1 の画面の大きさ。
    /// 面の点は、中心の下の塗るセットの面の点（hit）か、それが無ければ元の側の画素の点（[`SurfaceStroke::fallback_hit`]。元の側が
    /// 何も塗らなければ None）。対称の写しは、その点を写して探す（中心がほかのセット・背景にあるダブでも、元の側が塗る縁を写しの側にも
    /// 塗る）。
    fn build_dab(
        &mut self,
        doc: &Document,
        hit: Option<SurfaceHit>,
        at: Vec2,
        scale: f32,
        radius: f32,
        hardness: f32,
    ) -> (SurfaceDabResult, Option<SurfaceHit>) {
        let screen_radius = scale * radius;
        if self.projector.is_none() {
            match SurfaceProjector::new(
                self.geometry.clone(),
                &self.view,
                self.material,
                self.width,
                self.height,
                scale * self.world_radius,
                self.projection,
            ) {
                Ok(p) => self.projector = Some(p),
                Err(why) => return (SurfaceDabResult::default().reject(why), hit),
            }
        }
        self.clock += 1;
        let total = self.memory_total(doc);
        let room = self.room(total, None);
        let clock = self.clock;
        let mut result = self.projector.as_mut().expect("作った").dab_at(
            at,
            screen_radius,
            hardness,
            room,
            clock,
        );
        if result.refusal.is_some() {
            return (result, hit);
        }
        self.evict(total);
        let hit = hit.or_else(|| self.fallback_hit(&result));
        let (Some(sym), Some(hit)) = (self.symmetry, hit) else {
            return (result, hit);
        };
        let mut outcome = MirrorOutcome::OnPlane;
        let mut positions = vec![hit.position];
        let count = sym.radial.map_or(1, |r| r.count);
        for copy in 0..count {
            for reflected in [false, true] {
                if (copy == 0 && !reflected) || (reflected && sym.mirror.is_none()) {
                    continue;
                }
                let found = match find_copy(
                    &self.geometry,
                    &hit,
                    sym.mirror.as_ref(),
                    sym.radial.as_ref(),
                    copy,
                    reflected,
                    radius,
                    &mut positions,
                ) {
                    CopyHit::Duplicate => continue,
                    CopyHit::BudgetExceeded => {
                        self.stats.refused += 1;
                        self.note = Some(DabRefusal::BvhBudget);
                        continue;
                    }
                    CopyHit::NoSurface => {
                        outcome = MirrorOutcome::NoSurface;
                        continue;
                    }
                    CopyHit::OtherMaterial => {
                        outcome = MirrorOutcome::OtherSlot;
                        continue;
                    }
                    CopyHit::Found(h) => h,
                };
                self.stats.copies += 1;
                let dab = if sym.ignore_visibility {
                    // 見えない面にも塗る写しは、写しの点のまわりの球の中の面（カメラによらない足跡）
                    self.geometry.build_surface_dabs(
                        &found,
                        radius,
                        self.width,
                        self.height,
                        self.view.position,
                        hardness,
                        &self.budget,
                        None,
                        true,
                    )
                } else {
                    let slot = copy as usize * 2 + reflected as usize;
                    if self.copy_projectors.len() <= slot {
                        self.copy_projectors.resize_with(slot + 1, || None);
                    }
                    if self.copy_projectors[slot].is_none() {
                        let transform = CopyTransform {
                            mirror: if reflected { sym.mirror } else { None },
                            radial: sym.radial.map(|r| (r, copy)),
                        };
                        let p = self.projector.as_ref().expect("作った").copy(transform);
                        self.copy_projectors[slot] = Some(p);
                    }
                    let room = self.room(total, Some(slot));
                    let dab = self.copy_projectors[slot].as_mut().expect("作った").dab_at(
                        at,
                        screen_radius,
                        hardness,
                        room,
                        clock,
                    );
                    if dab.refusal.is_none() {
                        self.evict(total);
                    }
                    dab
                };
                if let Some(why) = dab.refusal {
                    self.stats.refused += 1;
                    self.note = Some(why);
                    continue;
                }
                if dab.pixels.is_empty() {
                    outcome = MirrorOutcome::Hidden;
                } else if outcome == MirrorOutcome::OnPlane {
                    outcome = MirrorOutcome::Painted;
                }
                result = union_dabs(result, dab, self.width);
            }
        }
        if matches!(
            outcome,
            MirrorOutcome::NoSurface | MirrorOutcome::OtherSlot | MirrorOutcome::Hidden
        ) {
            self.symmetry_note = Some(outcome);
        }
        (result, Some(hit))
    }

    /// 指先・クローン・伸ばすの 1 つのダブ: 先の面の点の展開の図と、元の面の点の展開の図で、ダブの画素ごとの
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
        let reach_scale = if smears {
            1.0 + 2.0 * self.stretch
        } else {
            1.0
        };
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
        let reach = spread(hit, pixels, radius)
            + magnitude(source.position - destination.position)
                * if smudge { 2.0 * reach_scale } else { 0.0 }
            + magnitude(hit.position - destination.position) * 2.0;
        let geometry = self.geometry.clone();
        // 図の三角形の数は、1 回の操作のメモリ（バイト）だけで抑える
        let mut dest_chart =
            geometry.build_sampling_chart(&destination, reach, Vec3::ZERO, i32::MAX, available)?;
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
            let chart =
                geometry.build_sampling_chart(&source, reach, tangent, i32::MAX, available)?;
            bytes += chart.nominal_bytes();
            source_chart = Some(chart);
        }
        let mut plan = Vec::with_capacity(pixels.len());
        let mut kept = Vec::new();
        for (i, p) in pixels.iter().enumerate() {
            // 画面の円には、つながらない面（別の房など）のテクセルも入る。図に入らない画素は読み元を決められないので塗らない
            let Some(point) = dest_chart.pixel_coordinates(p) else {
                continue;
            };
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

    /// ぼかしの 1 つのダブ。箱の窓（半径 r）が塗るセットの UV の中に収まる画素は、今までどおり UV の画像の上の箱の平均（`apply_dab`）。
    /// 窓が UV の島の縁にかかる画素は、面の上でまわりの 4 点（展開の図で UV の継ぎ目をまたぐ）を読み元にする（`apply_mapped_dab`）。
    /// 4 点は画素の点から、箱の平均と同じ広がり（1 軸の標準偏差）の所の 4 方向で、向きはダブごとに回す（重なるダブで方向の偏りが
    /// 残らない）。図に入らない縁の画素（つながらない面・対称の写しの側）は、UV の画像の上の同じ広がりの 4 点を読む。2 つは同じ
    /// ストロークの中で続けて塗るので 1 回の取り消しで戻る（縁の画素の読みは、島の中の画素を書いた後の値）。展開の図が 1 回の操作の
    /// メモリに入らない・参照を探す回数を超えたときなど、図で読めないときは、縁の画素も UV の画像の上の箱の平均で塗る（継ぎ目は
    /// またがないが、図のためにストロークを取り消さない）。
    #[allow(clippy::too_many_arguments)]
    fn blur_dab(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        hit: &SurfaceHit,
        pressure: f32,
        pixels: &[&super::SurfacePixel],
        points: Option<Vec<StencilPoint>>,
    ) -> Result<(), SurfaceStrokeError> {
        if pixels.is_empty() {
            return Ok(());
        }
        let r = self.blur_radius as i64;
        let (width, height) = (self.width, self.height);
        // 窓が島の縁にかかる画素（窓の中に、塗るセットの UV の外のテクセルがある）。ダブの箱の上の外のテクセルの累積和で数える
        let border: Vec<bool> = {
            let projector = self.projector.as_ref().expect("ダブがあれば作ってある");
            let x0 = pixels.iter().map(|p| p.x as i64).min().unwrap_or(0) - r - 1;
            let x1 = pixels.iter().map(|p| p.x as i64).max().unwrap_or(0) + r + 1;
            let y0 = pixels.iter().map(|p| p.y as i64).min().unwrap_or(0) - r - 1;
            let y1 = pixels.iter().map(|p| p.y as i64).max().unwrap_or(0) + r + 1;
            let (bw, bh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
            let mut sum = vec![0u32; (bw + 1) * (bh + 1)];
            for j in 0..bh {
                let mut row = 0u32;
                for i in 0..bw {
                    let (x, y) = (x0 + i as i64, y0 + j as i64);
                    row += !projector.covered(x as i32, y as i32) as u32;
                    sum[(j + 1) * (bw + 1) + i + 1] = sum[j * (bw + 1) + i + 1] + row;
                }
            }
            let at = |x: i64, y: i64| sum[(y - y0) as usize * (bw + 1) + (x - x0) as usize];
            pixels
                .iter()
                .map(|p| {
                    let (lx, ly) = (p.x as i64 - r, p.y as i64 - r);
                    let (hx, hy) = (p.x as i64 + r + 1, p.y as i64 + r + 1);
                    at(hx, hy) + at(lx, ly) > at(lx, hy) + at(hx, ly)
                })
                .collect()
        };
        // UV の画像の上の箱の平均で塗る（島の中の画素と、図で読めなかった縁の画素）
        let center = DVec2::new(
            hit.uv.x as f64 * width as f64,
            hit.uv.y as f64 * height as f64,
        );
        let box_blur = |doc: &mut Document,
                        stroke: &mut Stroke,
                        which: &[usize]|
         -> Result<(), SurfaceStrokeError> {
            if which.is_empty() {
                return Ok(());
            }
            let list: Vec<BrushPixel> = which
                .iter()
                .map(|&i| BrushPixel {
                    x: pixels[i].x as i64,
                    y: pixels[i].y as i64,
                    coverage: pixels[i].coverage.min(1.0) as f64,
                })
                .collect();
            match &points {
                Some(points) => {
                    let at: Vec<StencilPoint> = which.iter().map(|&i| points[i]).collect();
                    stroke.apply_dab_at(doc, &list, center, pressure as f64, &at)?
                }
                None => stroke.apply_dab(doc, &list, center, pressure as f64)?,
            };
            Ok(())
        };
        let inner: Vec<usize> = (0..pixels.len()).filter(|&i| !border[i]).collect();
        box_blur(doc, stroke, &inner)?;
        let edge: Vec<usize> = (0..pixels.len()).filter(|&i| border[i]).collect();
        if edge.is_empty() {
            return Ok(());
        }
        match self.blur_plan(doc, hit, pixels, &edge, points.is_some()) {
            Ok((plan, bytes)) => {
                let edge_points: Option<Vec<StencilPoint>> = points
                    .as_ref()
                    .map(|v| edge.iter().map(|&i| v[i]).collect());
                stroke.apply_mapped_dab(
                    doc,
                    &plan,
                    pressure as f64,
                    bytes,
                    edge_points.as_deref(),
                )?;
                Ok(())
            }
            Err(_) => box_blur(doc, stroke, &edge),
        }
    }

    /// ぼかしの縁の画素の読み元（展開の図の上の 4 点）と、図と計画の名目のバイト。図が 1 回の操作のメモリに入らない・参照を探す
    /// 回数を超えたときなどは Err（呼び手は UV の画像の上の箱の平均で塗る）。
    fn blur_plan(
        &mut self,
        doc: &Document,
        hit: &SurfaceHit,
        pixels: &[&super::SurfacePixel],
        edge: &[usize],
        stencil: bool,
    ) -> Result<(Vec<BrushMappedPixel>, u64), SamplingError> {
        let (width, height) = (self.width, self.height);
        let targets = doc.active_stroke_stats().map_or(1, |s| s.targets.max(1)) as i64;
        let per_pixel = 160 + targets * 4 + if stencil { 24 } else { 0 };
        let mut bytes = edge.len() as i64 * 28;
        let rollback = doc.active_stroke_stats().map_or(0, |s| s.rollback_bytes) as i64;
        let available = doc.stroke_budget_bytes().min(i64::MAX as u64) as i64
            - rollback
            - bytes
            - edge.len() as i64 * per_pixel;
        if available <= 0 {
            return Err(SamplingError::ChartBudget);
        }
        // 箱の平均（半径 r、一辺 2r + 1）の 1 軸の標準偏差（テクセル）
        let side = (2 * self.blur_radius + 1) as f32;
        let sigma = ((side * side - 1.0) / 12.0).sqrt();
        let geometry = self.geometry.clone();
        let mut texel = super::build::FastMap::<u32, f32>::default();
        let mut texel_of = |t: u32| -> f32 {
            *texel
                .entry(t)
                .or_insert_with(|| texel_size(&geometry.triangles()[t as usize], width, height))
        };
        // 図は、画素の広がりと、いちばん粗いテクセルでの 4 点の届く距離まで
        let coarsest = edge
            .iter()
            .filter_map(|&i| pixels[i].triangle)
            .map(&mut texel_of)
            .fold(0.0f32, f32::max);
        let reach = spread(hit, pixels, self.world_radius) + sigma * coarsest * 2.0;
        let mut chart =
            geometry.build_sampling_chart(hit, reach, Vec3::ZERO, i32::MAX, available)?;
        bytes += chart.nominal_bytes();
        // 4 方向の向き（cos・sin）。斜めの格子にそろわないよう 11.25° からずらし、ダブごとに 22.5° ずつ回す（90° で一周）
        const TURNS: [(f32, f32); 4] = [
            (0.980_785_3, 0.195_090_32),
            (0.831_469_6, 0.555_570_24),
            (0.555_570_24, 0.831_469_6),
            (0.195_090_32, 0.980_785_3),
        ];
        let (cos, sin) = TURNS[self.blur_turn % TURNS.len()];
        self.blur_turn += 1;
        let diagonal = [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)];
        let (w, h) = (width as i64, height as i64);
        let mut plan = Vec::with_capacity(edge.len());
        for &i in edge {
            let p = pixels[i];
            let pixel = BrushPixel {
                x: p.x as i64,
                y: p.y as i64,
                coverage: p.coverage.min(1.0) as f64,
            };
            let mut taps: Vec<(i64, i64)> = Vec::with_capacity(4);
            let size = p.triangle.map(&mut texel_of).unwrap_or(0.0);
            let on_chart = (size > 0.0 && size.is_finite())
                .then(|| chart.pixel_coordinates(p))
                .flatten();
            match on_chart {
                Some(q) => {
                    let reach = sigma * size;
                    for (dx, dy) in diagonal {
                        let o = Vec2::new(dx * cos - dy * sin, dx * sin + dy * cos) * reach;
                        if let Some(t) = chart.nearest_texel(q + o, width, height)? {
                            taps.push(t);
                        }
                    }
                }
                None => {
                    for (dx, dy) in diagonal {
                        let o = Vec2::new(dx * cos - dy * sin, dx * sin + dy * cos) * sigma;
                        let (x, y) = (
                            (p.x as f32 + 0.5 + o.x).floor() as i64,
                            (p.y as f32 + 0.5 + o.y).floor() as i64,
                        );
                        if x >= 0 && y >= 0 && x < w && y < h {
                            taps.push((x, y));
                        }
                    }
                }
            }
            if taps.is_empty() {
                taps.push((p.x as i64, p.y as i64));
            }
            let weight = 1.0 / taps.len() as f64;
            let tap = |i: usize| {
                taps.get(i).map_or(BrushSourceTap::NONE, |&(x, y)| {
                    BrushSourceTap::new(x, y, weight)
                })
            };
            plan.push(BrushMappedPixel::new(pixel, tap(0), tap(1), tap(2), tap(3)));
        }
        Ok((plan, bytes.max(0) as u64))
    }
}

/// 展開の図の届く距離: 打点の直径（モデルの単位）か、ダブの画素の当たりからの広がり（画面の円は傾いた面の上で長く伸びる。半径の 4 倍まで）
/// の大きいほう。
fn spread(hit: &SurfaceHit, pixels: &[&super::SurfacePixel], radius: f32) -> f32 {
    let far = pixels
        .iter()
        .filter(|p| p.triangle.is_some())
        .map(|p| magnitude(p.position - hit.position))
        .fold(0.0f32, f32::max);
    fmax(radius * 2.0, (far * 1.05).min(radius * 4.0))
}

/// 三角形の上のテクセル 1 つの、モデルの単位の大きさ（面積の比の平方根。潰れていれば 0）。
fn texel_size(t: &super::SurfaceTriangle, width: i32, height: i32) -> f32 {
    let area = magnitude(super::unity::cross(t.b - t.a, t.c - t.a)) as f64 * 0.5;
    let (e1, e2) = (t.uv_b - t.uv_a, t.uv_c - t.uv_a);
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

/// 区間の始まりの点でのダブの間隔（面の上の直径 × 間隔を画面に直す。0.5 以上）。面に当たらなければ、直前に面に当たった所の
/// 大きさ（scale はモデルの単位 1 の画面の大きさ）で、それも無ければ 1。
fn gap(
    geometry: &SurfaceGeometry,
    view: &CameraView,
    world_radius: f32,
    spacing: f32,
    scale: Option<f32>,
    a: Vec2,
) -> f32 {
    match pick(geometry, view, a) {
        Some(hit) => fmax(
            0.5,
            2.0 * view.world_radius_to_screen(hit.position, world_radius) * spacing,
        ),
        None => scale.map_or(1.0, |s| fmax(0.5, 2.0 * s * world_radius * spacing)),
    }
}
