//! 3D ビューのストローク: 画面の点の円の中の、カメラから見える面のテクセルを、投影の塗り（[`SurfaceProjector`]）で集めて、文書の
//! ストロークへ `apply_dab`（画素の並びをまとめて）で塗る。
//!
//! - ブラシの半径（文書の画素）をモデルの単位に直す式: max(1e-6, 箱の対角線) × 半径 / 文書の幅（× 筆圧の応え）。画面の円の半径は、
//!   中心の下の塗るテクスチャセットの面の奥行きで、その半径を画面へ直したもの。中心がほかのセットの面・背景にあるダブは、直前に
//!   面に当たったダブの奥行きで直す（縁で途切れない）。まだ一度も面に当たっていなければ塗らない。
//! - ストロークの間はカメラもモデルも動かない前提（区画の投影の画素を覚えて、重なる次のダブで作り直さない）。
//! - 投影の画素の覚えは 1 回の操作のメモリ（文書のストロークの予算から巻き戻しの分を除いたもの）に収め、超えたら長く使っていない
//!   区画から捨てる（要るときに同じものを作り直す）。元の側と対称の写しの投影の塗りは 1 つの予算を分け、どれの区画でも古いものから
//!   捨てる（元の側が覚えを溜めても、後から作る写しが締め出されない）。1 つのダブ（写し・デュアルブラシの 2 つ目のダブも）に要る区画
//!   だけで入らないとき、ダブ 1 つの上限（三角形・画素・見え方の判定・写しの点の探索）を超えたとき、区間のダブがありえない数
//!   （[`SURFACE_DABS_PER_SEGMENT`]）のとき、指先・クローン・伸ばす・色の混ぜの読み元が 1 回の操作のメモリに入らないときは、2D の
//!   ストロークと同じく `Err`（呼ぶ側がストロークごと取り消す。ダブを飛ばして塗り残しを作らない）。
//! - 速い入力: 区間のダブは、位置・面の当たり・画面の大きさを先に決めて待ち行列に並べるだけで（[`SurfaceStroke::add`]。塗らない）、
//!   塗るのは呼ぶ側がフレームごとに時間の枠まで（[`SurfaceStroke::paint_queued_until`]・[`SurfaceStroke::paint_queued_while`]。最初の
//!   1 つは必ず塗るので進む）。大きいブラシは 1 つのダブが重く、ダブの数で区切ると 1 フレームが長くなるため。離したとき
//!   （[`SurfaceStroke::end_input`]）は残りを塗らずに並べ、呼ぶ側が同じ枠で塗り続けて、空になってから文書のストロークを確定する。
//!   その場で確定させる呼び手は [`SurfaceStroke::finish`]（残りを全部塗る）。並べる時に当たりと大きさを決めるので、次の区間の間隔も、
//!   塗る時も、いつ塗っても、すぐ塗ったときと同じ（塗った結果は入力のまとまり方・フレームの区切り・時間の枠によらない）。
//! - 3D の対称（[`SurfaceSymmetrySetup`]）: 写しの点が、向きの合う同じテクスチャセットの面の近くにあるときだけ、写しの側を塗る。
//!   写しの点は、中心の下の面の点か、中心がほかのセット・背景にあるダブでは元の側の画素の点を写して探す。
//!   見えない面にも塗らない設定では、写したカメラ（鏡映・回転したカメラ）から同じ画面の円で投影の塗りをする（元の側の見え方を写した
//!   ものになる）。見えない面にも塗る設定では、写しの点のまわりの球の中の面（カメラによらない足跡）を塗る。全ての写しを画素ごとに
//!   大きい方の覆いで 1 つにして塗る。写しの点に向きの合う面が無い・別のテクスチャセット・見えないときは、その写しだけ飛ばして知らせる
//!   （`MirrorOutcome`）。ぼかしは写しも含めて 1 つのダブとして読み元を凍結する。指先・クローンは写しごとの読み元と動きが要るので
//!   対称とは組めない（ストロークの始めに断る）。
//! - 2D の対称（[`SurfaceStrokeOptions::canvas_symmetry`]）: 3D の写しの後に、元と 3D の写しの全部の UV の画素を、UV の平面の上で
//!   写す（`uv_symmetry`。2D のキャンバスのストロークに 3D の対称を当てるときと同じ順）。指先・クローンとは組めない。
//! - 色の混ぜ（ストロークの `Brush` の `mix`）は、混ぜるならぼかしと同じく面のダブを `apply_dab` へ（下地を書く前に、UV の離れたアイランドごとの塊に分けて凍結する）。伸ばす
//!   （対称なし）は指先と同じ写像されたダブで、展開の図で UV の継ぎ目をまたいで前のダブの側を読む（最初のダブ・動いていないダブ・図に入らない
//!   ダブも、ずれ 0 で塗る）。対称（3D・2D）と組むときは、写しごとの読み元が要るので `apply_dab`（動きの向きなし。2D のキャンバスの
//!   対称のダブと同じ）へ。
//! - 効果のブラシ（[`SurfaceEffect`]）: ぼかしは画素ごとに面の上のまわりの 4 点を読み元にする。指先は直前のダブの面の点から今の点へ
//!   引きずり、クローンは固定した元の面の点から、ストロークの最初の面の点に対応させて写す。どれも展開の図（[`super::SamplingChart`]）で
//!   UV アイランドの継ぎ目をまたいで読み元の画素を決め、全ての読みを書く前に凍結する（`Stroke::apply_mapped_dab`）。画面の円に入った、図に
//!   つながらない面の画素は、指先・クローンでは塗らず、ぼかしでは UV の画像の上で読む。ぼかしは、図を作れないとき（1 回の操作の
//!   メモリ・参照を探す回数）も UV の画像の上の箱の平均で塗る（取り消さない）。
//! - 画面の円の半径と中心の下の面の点は、カメラ（[`CameraView`]）の単精度の tan で出す（投影の塗りの中は四則と平方根だけ）。
//! - 筆先は 2D のストロークと同じ: 描点ごとのフェード・傾き・速さ・筆圧の係数と、ダブ（数の分）ごとの大きさ・散布・角度・真円率・
//!   不透明度・流量のゆらぎと筆先の選びを、2D と同じ関数（`brush::plan`）と同じ乱数の列（同じ種・同じ順）で決める。座標は画面の点を
//!   y が上向きになるように直した枠（2D の文書の画素と同じ向き）で、長さ（手ぶれ補正・入り抜き・散布の届く長さ・速さ）は画面の点。
//!   ダブの形は画面の上の形で、投影の画素ごとに中心からの画面のずれで 2D と同じ覆いの式（`ShapeCoverage`）を読み、面の向きの弱めを
//!   掛ける。丸い筆先（画像なし・真円率 1）は今までの丸の式のまま。ダブごとの色・ゆらぎの不透明度と流量の係数・紙の質感（塗る
//!   テクセルの文書の画素の位置で読む）は、文書のストロークが面の画素を塗るときに当てる（`Stroke::begin_surface_dab`）。
//! - デュアルブラシ: 2 つ目のダブを同じ道筋に先に並べ、描点の線の長さまでのものを、画面の上の形でテクセルごとの最大として溜める
//!   （面の向きでは弱めない。対称の写しにも置く）。主のダブの覆いは、形の覆いと溜まりを 2D と同じ 8 つの合わせ方で合わせてから、
//!   面の向きで弱める。
//! - 手ぶれ補正（引っ張る糸）は入力の点に、入り抜きは描点の線の長さにかける（2D と同じ式）。抜きの長さの内の描点は、線が伸びるか
//!   離すまで待たせる（待っている描点は [`SurfaceStroke::queued`] に数えない）。

use std::collections::VecDeque;
use std::sync::Arc;

use glam::{DVec2, Quat, Vec2, Vec3};

use super::build::FastMap;
use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceBrushBudget, SurfaceDabResult};
use super::project::{
    evict_least_recent, CopyTransform, ProjectionSettings, ProjectionStats, RoundCover,
    ScreenCover, SurfaceProjector,
};
use super::query::coverage;
use super::sampling::SamplingError;
use super::stencil::SurfaceStencil;
use super::stroke::{
    ScreenDab, ScreenPoint, ScreenStrokeSampler, SegmentGaps, TooManyDabs, SURFACE_DABS_PER_SEGMENT,
};
use super::symmetry::{find_copy, union_dabs, CopyHit, MirrorOutcome, MirrorPlane, RadialSymmetry};
use super::unity::{dot, fmax, magnitude, sqr_magnitude, v2_magnitude as magnitude2};
use super::{SurfaceGeometry, SurfaceHit};
use crate::brush::profile;
use crate::brush::random::NetRandom;
use crate::brush::{
    combine_dual, AntiAlias, BrushMappedPixel, BrushSourceTap, DabPlan, DabShape, PendingDab,
    ShapeCoverage, StampControls, SurfaceDabLook, TexelMetric, DUAL_STREAM,
};
use crate::{
    Brush, BrushPixel, BrushSettings, CanvasSymmetry, CoreError, Document, DualBrushMode, MixMode,
    PressureResponse, StencilPoint, Stroke, SymmetryTransform,
};

/// 3D のストロークを止めた理由（どれもストロークを取り消す）。
#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceStrokeError {
    /// ダブを作れなかった（投影の塗りの準備が範囲外の値を断った）。
    Dab(DabRefusal),
    /// 1 つの区間のダブがありえない数（[`SURFACE_DABS_PER_SEGMENT`] を超える）。
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
    /// 2D の対称（UV の平面の写し。文書の画素の座標の中心）。3D の対称の写しの後に、元と写しの全部の画素へ当てる
    /// （`uv_symmetry`）。None か「なし」なら当てない。
    pub canvas_symmetry: Option<CanvasSymmetry>,
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
    /// 対称の写しを塗ったダブの数のべ（元を除く）。
    pub copies: usize,
    /// 指先が、直前の点から読めず（つながらない面）に飛ばしたダブ。
    pub lost: usize,
}

/// 待ち行列に置けるダブの数（超えた分は、入力のときにその場で塗る）。1 つの区間の上限と同じ。待ち行列のメモリの上限で（1 つ約 150 バイト、
/// 上限でおよそ 10 MiB）、時間の見込みでは決めない: 見込みでその場で塗る形にすると、塗りが追いつかない入力のとき入力の処理が塗りの時間で止まり、
/// 固まる元の形に戻る。時間の枠は呼ぶ側（`paint_queued_until`。アプリは溜まった見込みで枠を広げる）が持ち、ここに届くのは、塗りがまったく
/// 追いつかないまま 65,536 ダブ溜まった異常な長さの線だけ。
pub const MAX_QUEUED_DABS: usize = SURFACE_DABS_PER_SEGMENT;

/// 3D のストロークの入力の点（画面の点。表示域の左上が原点で、下が +y）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInput {
    pub at: Vec2,
    /// 筆圧 0〜1。
    pub pressure: f32,
    /// ペンの傾き（ラジアン。直立からの角度を、画面の右と上へ倒れる向きの軸に分けたもの。0 は直立か情報なし）。
    pub tilt: DVec2,
    /// ペンの軸の回転（ラジアン、画面で反時計回り。0 は回転なしか情報なし）。
    pub rotation: f64,
    /// 時刻（秒。筆の速さに使う。前の点より前の時刻は前の点の時刻とみなす）。
    pub time: f64,
}

impl SurfaceInput {
    /// 位置と筆圧だけの入力（傾き・回転なし、時刻 0）。
    pub fn new(at: Vec2, pressure: f32) -> SurfaceInput {
        SurfaceInput {
            at,
            pressure,
            tilt: DVec2::ZERO,
            rotation: 0.0,
            time: 0.0,
        }
    }
}

/// 位置と当たりを決めて、まだ塗っていない描点。
#[derive(Clone, Copy, Debug)]
struct QueuedDab {
    at: Vec2,
    pressure: f32,
    /// 押した点からの線の長さ（画面の点。入り抜き）と、その所の線の向き・ペンの傾き・回転・速さ。
    arc: f64,
    direction: f64,
    tilt: DVec2,
    rotation: f64,
    speed: f64,
    /// 中心の下の、塗るセットの面の当たり。
    hit: Option<SurfaceHit>,
    /// このダブを置いた時の、モデルの単位 1 の画面の大きさ（まだ一度も面に当たっていなければ None）。
    scale: Option<f32>,
    /// 筆圧で大きさ 0 のダブ（塗らない・数えない。ゆらぎの乱数と色は 2D と同じく進める）。
    skip: bool,
}

/// デュアルブラシの 2 つ目のダブの溜まり（テクセルごとの最大の覆い。64 × 64 のタイルで持つ）。
#[derive(Default)]
struct DualCells {
    tiles: FastMap<u32, Box<[f32]>>,
}

const DUAL_TILE: u32 = 64;

impl DualCells {
    fn key(x: u32, y: u32) -> u32 {
        ((y / DUAL_TILE) << 16) | (x / DUAL_TILE)
    }
    #[inline]
    fn get(&self, x: u16, y: u16) -> f32 {
        let (x, y) = (x as u32, y as u32);
        self.tiles.get(&Self::key(x, y)).map_or(0.0, |t| {
            t[((y % DUAL_TILE) * DUAL_TILE + x % DUAL_TILE) as usize]
        })
    }
    fn raise(&mut self, x: i32, y: i32, v: f32) {
        if x < 0 || y < 0 || v <= 0.0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        let tile = self
            .tiles
            .entry(Self::key(x, y))
            .or_insert_with(|| vec![0.0; (DUAL_TILE * DUAL_TILE) as usize].into_boxed_slice());
        let cell = &mut tile[((y % DUAL_TILE) * DUAL_TILE + x % DUAL_TILE) as usize];
        if v > *cell {
            *cell = v;
        }
    }
    fn bytes(&self) -> u64 {
        self.tiles.len() as u64 * (64 + DUAL_TILE as u64 * DUAL_TILE as u64 * 4)
    }
}

/// デュアルブラシ（2D と同じく、同じ道筋に自分の間隔・散布・数で 2 つ目のダブを先に並べ、テクセルごとの最大を溜める）。
struct DualState {
    random: NetRandom,
    /// 並べて、まだ置いていない 2 つ目のダブ: 画面の点・線の長さ・その時の画面の大きさ。
    pending: VecDeque<(Vec2, f64, Option<f32>)>,
    /// 2 つ目の筆先の半径（モデルの単位。大きさは筆圧・ゆらぎ・入り抜きの影響を受けない）。
    world_radius: f32,
    cells: DualCells,
}

/// 画面の上のダブの形の種類。
enum Form<'a> {
    /// 丸い筆先（画像なし・真円率 1。今までの 3D の式）。
    Round { hardness: f32 },
    /// 筆先の画像・潰した丸（2D と同じ式。半径は画面の点）。
    Shaped(DabShape<'a>),
}

/// 投影の画素の覆い: 丸か 2D と同じ形の覆いに、デュアルブラシの溜まりを合わせたもの。
struct DabCover<'a> {
    form: CoverForm<'a>,
    dual: Option<(DualBrushMode, &'a DualCells)>,
}

enum CoverForm<'a> {
    Round(RoundCover),
    Shaped(ShapeCoverage<'a>, f64),
}

impl ScreenCover for DabCover<'_> {
    fn reach(&self) -> f64 {
        match &self.form {
            CoverForm::Round(r) => r.reach(),
            CoverForm::Shaped(_, reach) => *reach,
        }
    }
    #[inline]
    fn cover(&self, dx: f64, dy: f64, x: u16, y: u16, metric: [f32; 3]) -> f32 {
        let c = match &self.form {
            CoverForm::Round(r) => r.cover(dx, dy, x, y, metric),
            // 形は y を上向きに測る（2D の文書の画素と同じ向き）
            CoverForm::Shaped(s, _) => s.coverage(dx as f32, (-dy) as f32),
        };
        match self.dual {
            Some((mode, cells)) => combine_dual(mode, c, cells.get(x, y)),
            None => c,
        }
    }
}

/// 進行中の 3D のストローク（文書のストロークの札と一緒に持つ）。
pub struct SurfaceStroke {
    geometry: Arc<SurfaceGeometry>,
    view: CameraView,
    material: Option<i32>,
    sampler: ScreenStrokeSampler,
    /// 位置と当たりを決めて、まだ塗っていない描点（古い順）。
    queue: std::collections::VecDeque<QueuedDab>,
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
    /// 丸い筆先の縁のアンチエイリアス（[`crate::BrushSettings::anti_alias`]）と、直前に面に当たったダブの中心のテクセルの画面の
    /// 大きさ（中心が面に無いダブの見積もりに使う）。
    anti_alias: AntiAlias,
    last_metric: Option<TexelMetric>,
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
    /// 2D の対称の変換（恒等を含む。2D の対称が無ければ空）。
    canvas: Vec<SymmetryTransform>,
    effect: SurfaceEffect,
    /// 指先: 直前のダブの面の点。
    previous_hit: Option<SurfaceHit>,
    /// クローン: 先の基準の点（最初のダブの面の点か、揃えるなら前のストロークの先）。
    clone_destination: Option<SurfaceHit>,
    /// 対称の写しが塗られなかった理由の最後のもの（塗れた写しだけなら None。Painted は入れない）。
    symmetry_note: Option<MirrorOutcome>,
    /// ストロークのブラシ（文書のストロークが始めに固めたもの。筆先・ゆらぎ・入り抜き・手ぶれ補正・デュアルブラシ）。
    brush: Arc<Brush>,
    /// ゆらぎ・散布・筆先の選びの乱数の列（2D のストロークと同じ種で、同じ順に引く）と、順に使う筆先の番号。
    random: NetRandom,
    tip_index: u64,
    /// 置いた描点の数（フェード）。
    stamps: u64,
    /// 手ぶれ補正の後の筆の点（速さはその点での筆の速さ）と、その時刻。最後の入力の点。
    pen: ScreenPoint,
    pen_time: f64,
    last_input: SurfaceInput,
    /// 離した時の線の長さ（抜きの終わり）。離すまでは None。
    end: Option<f64>,
    /// デュアルブラシ（無ければ None）と、その溜まりのバイト。
    dual: Option<DualState>,
    dual_bytes: u64,
    pub stats: SurfaceStrokeStats,
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
        Self::begin_input(
            doc,
            stroke,
            geometry,
            view,
            brush,
            material,
            SurfaceInput::new(at, pressure),
            options,
        )
    }

    /// [`SurfaceStroke::begin_with_options`] の、ペンの傾き・回転・時刻つきの入力の形。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_input(
        doc: &mut Document,
        stroke: &mut Stroke,
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        brush: &BrushSettings,
        material: Option<i32>,
        input: SurfaceInput,
        options: SurfaceStrokeOptions,
    ) -> Result<SurfaceStroke, SurfaceStrokeError> {
        let symmetry = options.symmetry.filter(|s| s.enabled());
        let canvas = match options.canvas_symmetry.filter(|s| s.enabled()) {
            Some(c) => c.transforms()?,
            None => Vec::new(),
        };
        if (symmetry.is_some() || !canvas.is_empty())
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
        // 伸ばすは、対称（3D・2D）と組むと写しごとの読み元が要るので、2D の対称のダブと同じく動きの向きを使わない読み方へ
        let smears = mixes
            && stroke_brush.mix.mode == MixMode::Smear
            && symmetry.is_none()
            && canvas.is_empty();
        let first = ScreenPoint {
            at: input.at,
            pressure: input.pressure,
            tilt: input.tilt,
            rotation: input.rotation,
            speed: 0.0,
        };
        let dual = stroke_brush.dual.as_ref().map(|d| DualState {
            random: NetRandom::new(stroke_brush.seed ^ DUAL_STREAM),
            pending: VecDeque::new(),
            world_radius: self::world_radius(&geometry, d.radius, doc.width()),
            cells: DualCells::default(),
        });
        let mut s = SurfaceStroke {
            geometry,
            view,
            material,
            sampler: ScreenStrokeSampler::with_curve(first, stroke_brush.assist.curve),
            queue: std::collections::VecDeque::new(),
            budget: SurfaceBrushBudget::default(),
            projection: options.projection.sanitized(),
            projector: None,
            copy_projectors: Vec::new(),
            clock: 0,
            screen_scale: None,
            memory_override: options.projection_memory,
            world_radius,
            hardness: brush.hardness as f32,
            anti_alias: brush.anti_alias,
            last_metric: None,
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
            canvas,
            effect: options.effect,
            previous_hit: None,
            clone_destination,
            symmetry_note: None,
            random: NetRandom::new(stroke_brush.seed),
            tip_index: 0,
            stamps: 0,
            pen: first,
            pen_time: input.time,
            last_input: input,
            end: None,
            dual,
            dual_bytes: 0,
            brush: stroke_brush,
            stats: SurfaceStrokeStats::default(),
        };
        // 押した点の描点（と、2 つ目の筆先のダブ）。抜きがあれば、線が伸びるか離すまで待つ
        let dab = s.locate(ScreenDab {
            at: input.at,
            pressure: input.pressure,
            arc: 0.0,
            direction: 0.0,
            tilt: input.tilt,
            rotation: input.rotation,
            speed: 0.0,
        });
        s.queue.push_back(dab);
        s.enqueue_dual(&[(input.at, 0.0)]);
        // 押した点のダブだけが並んでいる（抜きがあれば待たせる）
        s.paint_queued(doc, stroke, usize::MAX)?;
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

    /// 新しい入力の点（画面の座標）。[`SurfaceStroke::add_input`] の、位置と筆圧だけの形（時刻は前の点と同じ）。
    pub fn add(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        at: Vec2,
        pressure: f32,
    ) -> Result<(), SurfaceStrokeError> {
        let input = SurfaceInput {
            time: self.last_input.time,
            ..SurfaceInput::new(at, pressure)
        };
        self.add_input(doc, stroke, input)
    }

    /// 新しい入力の点。手ぶれ補正の糸で筆を引き（2D と同じ式、長さは画面の点）、描けるようになった区間のダブを待ち行列に並べるだけで、
    /// 塗らない（塗るのは呼ぶ側がフレームごとに [`SurfaceStroke::paint_queued_until`] で時間の枠まで）。入力が重なっても 1 回の入力は
    /// 並べる仕事（点の数に比例）しかしないので、塗りの重さが入力の処理に積み上がらない。待ちが [`MAX_QUEUED_DABS`] を超える分だけは、
    /// その場で塗る（覚えるメモリを抑える安全弁）。抜きの長さの内の描点は、線が伸びるか離すまで待つ。
    pub fn add_input(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        input: SurfaceInput,
    ) -> Result<(), SurfaceStrokeError> {
        let input = SurfaceInput {
            time: input.time.max(self.last_input.time),
            ..input
        };
        self.last_input = input;
        let assist = self.brush.assist;
        let pen = if assist.stabilizer > 0.0 {
            assist
                .pull(
                    (self.pen.at.x as f64, self.pen.at.y as f64),
                    (input.at.x as f64, input.at.y as f64),
                )
                .map(|(x, y)| Vec2::new(x as f32, y as f32))
        } else {
            Some(input.at)
        };
        if let Some(at) = pen {
            self.move_pen(at, input)?;
        }
        let over = self.queue.len().saturating_sub(MAX_QUEUED_DABS);
        if over > 0 {
            self.paint_queued(doc, stroke, over)?;
        }
        Ok(())
    }

    /// 筆を at へ動かす（速さは前の筆の点からの距離 ÷ 時刻の差。時刻が進まなければ前の速さのまま）。描けるようになった区間のダブを並べる。
    fn move_pen(&mut self, at: Vec2, input: SurfaceInput) -> Result<(), SurfaceStrokeError> {
        let dt = input.time - self.pen_time;
        let speed = if dt > 0.0 {
            magnitude2(at - self.pen.at) as f64 / dt
        } else {
            self.pen.speed
        };
        let point = ScreenPoint {
            at,
            pressure: input.pressure,
            tilt: input.tilt,
            rotation: input.rotation,
            speed,
        };
        self.pen = point;
        self.pen_time = input.time;
        let (mut points, mut duals) = (Vec::new(), Vec::new());
        let gaps = self.gaps();
        self.sampler
            .add_point(point, gaps, &mut points, &mut duals)
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        self.enqueue(points, &duals);
        Ok(())
    }

    /// 区間の始まりの点での間隔（ダブと、デュアルブラシの 2 つ目のダブ）を出す関数。
    fn gaps(&self) -> impl FnMut(Vec2) -> SegmentGaps + use<> {
        let (geometry, view, radius, spacing, scale) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.spacing,
            self.screen_scale,
        );
        let dual = self
            .dual
            .as_ref()
            .zip(self.brush.dual.as_ref())
            .map(|(d, b)| (d.world_radius, b.spacing as f32));
        move |a| {
            (
                gap(&geometry, &view, radius, spacing, scale, a),
                dual.map(|(r, s)| gap(&geometry, &view, r, s, scale, a)),
            )
        }
    }

    /// 描点が今塗れるか（抜きのある線は、終わりから抜きの長さの内の描点を、線が伸びるか離すまで待たせる）。
    fn paintable(&self, dab: &QueuedDab) -> bool {
        let taper = self.brush.assist.taper_out;
        self.end.is_some() || taper <= 0.0 || dab.arc <= self.sampler.length() - taper
    }

    /// 待ち行列のダブを、古い順に max まで塗る（塗った数を返す。数で区切る形で、時間で区切るのは
    /// [`SurfaceStroke::paint_queued_while`]・[`SurfaceStroke::paint_queued_until`]）。
    pub fn paint_queued(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        max: usize,
    ) -> Result<usize, SurfaceStrokeError> {
        let mut painted = 0;
        while painted < max && self.paint_front(doc, stroke)? {
            painted += 1;
        }
        Ok(painted)
    }

    /// 待ち行列のダブを、古い順に、`more` が true を返す間塗る。最初の 1 つは（塗れるなら）必ず塗り（時間の枠が小さくても進む）、
    /// 1 つ塗るごとに、これまでに塗った数を `more` へ渡して続けるかを聞く。呼ぶ側は時計（フレームの締め切り）か、試験では決まった数を
    /// 渡す。塗った数を返す。どこで区切っても、塗る結果は同じ（当たりと大きさは並べるときに決めてある）。
    pub fn paint_queued_while(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        mut more: impl FnMut(usize) -> bool,
    ) -> Result<usize, SurfaceStrokeError> {
        let mut painted = 0;
        while self.paint_front(doc, stroke)? {
            painted += 1;
            if !more(painted) {
                break;
            }
        }
        Ok(painted)
    }

    /// [`SurfaceStroke::paint_queued_while`] の、締め切りの時刻まで。次の 1 つが、ここまでに塗った 1 つの平均と同じ長さかかっても締め切りに
    /// 収まるときだけ続ける（ダブ 1 つが重いとき、締め切りを過ぎてから止めると、1 フレームがダブ 1 つ分だけ必ず延びるため）。最初の 1 つは
    /// 締め切りを過ぎていても塗る。
    pub fn paint_queued_until(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        deadline: std::time::Instant,
    ) -> Result<usize, SurfaceStrokeError> {
        let start = std::time::Instant::now();
        self.paint_queued_while(doc, stroke, |painted| {
            let now = std::time::Instant::now();
            now + (now - start) / painted as u32 <= deadline
        })
    }

    /// 待ち行列の先頭のダブが塗れるなら塗って true（空・抜きのために線の続きを待っているときは false）。
    fn paint_front(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
    ) -> Result<bool, SurfaceStrokeError> {
        let Some(&dab) = self.queue.front() else {
            return Ok(false);
        };
        if !self.paintable(&dab) {
            return Ok(false);
        }
        self.queue.pop_front();
        self.paint_located(doc, stroke, dab)?;
        Ok(true)
    }

    /// 今塗れる、まだ塗っていないダブの数（抜きのために線の続きを待っているダブは数えない）。
    pub fn queued(&self) -> usize {
        self.queue.iter().take_while(|d| self.paintable(d)).count()
    }

    /// 離したとき（塗らない形）: 手ぶれ補正の筆を最後の入力の点まで描き、待たせている最後の区間を並べて、抜きの終わりの長さを決める。
    /// 残りのダブは塗らずに待ち行列に残す（呼ぶ側が [`SurfaceStroke::paint_queued_until`] などでフレームごとに塗り、
    /// [`SurfaceStroke::queued`] が 0 になったら文書のストロークを確定する）。2 回目からは何もしない。
    pub fn end_input(&mut self) -> Result<(), SurfaceStrokeError> {
        if self.end.is_some() {
            return Ok(());
        }
        if self.brush.assist.stabilizer > 0.0 && self.pen.at != self.last_input.at {
            self.move_pen(self.last_input.at, self.last_input)?;
        }
        let (mut points, mut duals) = (Vec::new(), Vec::new());
        let gaps = self.gaps();
        self.sampler
            .finish_points(gaps, &mut points, &mut duals)
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        self.enqueue(points, &duals);
        self.end = Some(self.sampler.length());
        Ok(())
    }

    /// 離したあとか（[`SurfaceStroke::end_input`] 済み）。
    pub fn input_ended(&self) -> bool {
        self.end.is_some()
    }

    /// 離したとき: [`SurfaceStroke::end_input`] をして、待ち行列を全部塗る（この後に文書のストロークを確定する）。その場で確定させる
    /// 呼び手（保存・閉じる・GPU を失ったときなど）の形で、時間の枠は使わない。
    pub fn finish(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
    ) -> Result<(), SurfaceStrokeError> {
        self.end_input()?;
        self.paint_queued(doc, stroke, usize::MAX)?;
        Ok(())
    }

    /// 区間のダブの位置を、当たりと画面の大きさを決めて待ち行列の後ろに並べる（2 つ目の筆先のダブも）。
    fn enqueue(&mut self, points: Vec<ScreenDab>, duals: &[(Vec2, f64)]) {
        self.queue.reserve(points.len());
        for p in points {
            let dab = self.locate(p);
            self.queue.push_back(dab);
        }
        self.enqueue_dual(duals);
    }

    /// 2 つ目の筆先のダブを、その所の画面の大きさを決めて並べる（面に当たらなければ直前の大きさ）。
    fn enqueue_dual(&mut self, duals: &[(Vec2, f64)]) {
        if self.dual.is_none() {
            return;
        }
        for &(at, arc) in duals {
            let scale = pick(&self.geometry, &self.view, at)
                .filter(|h| self.material.is_none_or(|m| m == h.material))
                .map(|h| self.view.world_radius_to_screen(h.position, 1.0))
                .filter(|s| s.is_finite() && *s > 0.0)
                .or(self.screen_scale);
            if let Some(d) = self.dual.as_mut() {
                d.pending.push_back((at, arc, scale));
            }
        }
    }

    /// 筆圧を応えに通した大きさの係数（既定の応えは筆圧そのもの）。
    fn size_pressure(&self, pressure: f32) -> f32 {
        match &self.size_response {
            Some(r) => r.apply(pressure as f64) as f32,
            None => pressure,
        }
    }

    /// ダブの面の当たりを決め、画面の大きさ（直前に塗るセットの面に当たった所の奥行き）を進める。大きさ 0 のダブは当てない
    /// （塗らず、大きさも進めない）。次の区間の間隔はこの大きさで決まるので、並べるときに決める。
    fn locate(&mut self, p: ScreenDab) -> QueuedDab {
        let mut dab = QueuedDab {
            at: p.at,
            pressure: p.pressure,
            arc: p.arc,
            direction: p.direction,
            tilt: p.tilt,
            rotation: p.rotation,
            speed: p.speed,
            hit: None,
            scale: None,
            skip: false,
        };
        if self.pressure_size && self.size_pressure(p.pressure) <= 0.0 {
            dab.skip = true;
            return dab;
        }
        let hit = pick(&self.geometry, &self.view, p.at)
            .filter(|h| self.material.is_none_or(|m| m == h.material));
        if let Some(h) = &hit {
            let scale = self.view.world_radius_to_screen(h.position, 1.0);
            if scale.is_finite() && scale > 0.0 {
                self.screen_scale = Some(scale);
            }
        }
        dab.hit = hit;
        dab.scale = self.screen_scale;
        dab
    }

    /// 画面の y（下向き）を、ダブの置き方の枠の y（上向き）へ。
    fn up(&self, y: f32) -> f64 {
        self.view.height as f64 - y as f64
    }

    /// ダブの置き方の枠の y（上向き）を、画面の y へ。
    fn down(&self, y: f64) -> f32 {
        (self.view.height as f64 - y) as f32
    }

    /// 描点 1 つ（2D の Stamp と同じ順）: 2 つ目の筆先のダブをこの描点の線の長さまで置き、フェード・傾き・速さ・筆圧の係数を決めて、
    /// 数の分のダブを、2D と同じ式（大きさ・散布・角度・真円率・不透明度・流量のゆらぎ、筆先の選び、ダブごとの色）で置く。
    fn paint_located(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        dab: QueuedDab,
    ) -> Result<(), SurfaceStrokeError> {
        let index = self.stamps;
        self.stamps += 1;
        if self.dual.is_some() {
            self.stamp_dual(doc, dab.arc, dab.scale)?;
        }
        let brush = self.brush.clone();
        let pending = PendingDab {
            x: dab.at.x as f64,
            y: self.up(dab.at.y),
            pressure: dab.pressure as f64,
            arc: dab.arc,
            direction: dab.direction,
            tilt_x: dab.tilt.x,
            tilt_y: dab.tilt.y,
            rotation: dab.rotation,
            speed: dab.speed,
        };
        let controls = brush.stamp_controls(&pending, index);
        // 大きさの係数（入り抜き）。筆圧の大きさとモデルの単位の半径は塗るときに掛ける（今までの 3D の式のまま）
        let factor = if dab.skip {
            0.0
        } else {
            brush
                .assist
                .taper(dab.arc, self.end.unwrap_or(f64::INFINITY))
        };
        // 散布の届く長さの元: 筆圧の前の半径を画面へ直したもの
        let nominal = dab.scale.map_or(0.0, |s| (s * self.world_radius) as f64);
        for _ in 0..brush.jitter.count {
            let plan = brush.next_dab(
                &controls,
                &pending,
                factor,
                nominal,
                &mut self.random,
                &mut self.tip_index,
            );
            stroke.begin_surface_dab(
                doc,
                plan.as_ref().map(|p| SurfaceDabLook {
                    opacity: p.opacity_scale,
                    flow: p.flow_scale,
                }),
            )?;
            let Some(plan) = plan else {
                continue;
            };
            self.paint_plan(doc, stroke, &dab, &pending, &plan, &controls)?;
        }
        Ok(())
    }

    /// 置き方を決めたダブを塗る。`plan.radius` は大きさの係数（入り抜き・フェード・傾き・速さ・ゆらぎ）。
    fn paint_plan(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        dab: &QueuedDab,
        pending: &PendingDab,
        plan: &DabPlan<'_>,
        controls: &StampControls,
    ) -> Result<(), SurfaceStrokeError> {
        let _profile = profile::scope(profile::Stage::SurfaceDab);
        let pressure = dab.pressure;
        // 散布で動いたダブは、その中心の下の面を当て直す
        let (at, hit) = if plan.x == pending.x && plan.y == pending.y {
            (dab.at, dab.hit)
        } else {
            let at = Vec2::new(plan.x as f32, self.down(plan.y));
            let hit = pick(&self.geometry, &self.view, at)
                .filter(|h| self.material.is_none_or(|m| m == h.material));
            (at, hit)
        };
        let size_pressure = self.size_pressure(pressure);
        let inside =
            at.x >= 0.0 && at.x < self.view.width && at.y >= 0.0 && at.y < self.view.height;
        // 指先・クローン・色の混ぜの伸ばすは、面の上の中心から読み元を決めるので、中心が塗るセットの面に無いダブは飛ばす
        let needs_hit = matches!(self.effect, SurfaceEffect::Smudge | SurfaceEffect::Clone(_))
            || (self.smears && matches!(self.effect, SurfaceEffect::Paint));
        let Some(scale) = dab
            .scale
            .filter(|_| inside && (hit.is_some() || !needs_hit))
        else {
            self.stats.missed += 1;
            return Ok(());
        };
        let mut radius = self.world_radius
            * if self.pressure_size {
                fmax(0.001, size_pressure)
            } else {
                1.0
            };
        if plan.radius != 1.0 {
            radius *= plan.radius as f32;
        }
        let hardness = match &self.hardness_response {
            Some(r) => (self.hardness as f64 * r.apply(pressure as f64)) as f32,
            None => self.hardness,
        };
        // 丸い筆先（画像なし・真円率 1。角度は形を変えない）は今までの式、そのほかは 2D と同じ形の式
        let brush = self.brush.clone();
        let form = if plan.tip.is_none() && plan.roundness == 1.0 {
            Form::Round { hardness }
        } else {
            let mut shape = brush.dab_shape(plan, controls);
            shape.radius = (scale * radius) as f64;
            Form::Shaped(shape)
        };
        let dual = self.dual.take();
        let mode = brush.dual.as_ref().map(|d| d.mode);
        let (dab, hit) = self.build_dab(
            doc,
            hit,
            at,
            scale,
            radius,
            &form,
            mode.zip(dual.as_ref().map(|d| &d.cells)),
            true,
        );
        self.dual = dual;
        // 1 つのダブ（写しも）でも塗れなければ、2D と同じくストロークごと取り消す（塗り残しを作らない）
        if let Some(why) = dab.refusal {
            return Err(SurfaceStrokeError::Dab(why));
        }
        let Some(hit) = hit else {
            return Ok(());
        };
        self.stats.dabs += 1;
        let footprint = self
            .stencil
            .map(|st| st.footprint(&self.geometry, &self.view, &hit, self.width, self.height));
        // apply_dab は覆いが 0〜1 の外だとストロークを取り消すので、0 以下は塗らない（覆い 0 は何も変えない）
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
            // 色を塗るだけ: ダブの画素をまとめて渡す（画素ごとに文書へ入ると、変わったタイルの印やメモの調整が画素の数だけ繰り返される）。
            // 塗る式・順は画素ごとに渡したときと同じ
            SurfaceEffect::Paint if !self.mixes => {
                let _profile = profile::scope(profile::Stage::SurfaceApply);
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

    /// 2 つ目の筆先のダブを、線の長さ limit まで置いて溜める（2D の StampDual と同じ順。散布の乱数は 2 つ目の筆先の列）。画面の大きさは
    /// 並べたときに決めたもの（面にまだ当たっていなければ置かない）。投影の塗りがまだ無ければ、今の描点の大きさ（scale）で作る。
    fn stamp_dual(
        &mut self,
        doc: &Document,
        limit: f64,
        scale: Option<f32>,
    ) -> Result<(), SurfaceStrokeError> {
        let Some(dual_brush) = self.brush.dual.clone() else {
            return Ok(());
        };
        while let Some(&(at, arc, dual_scale)) = self.dual.as_ref().and_then(|d| d.pending.front())
        {
            if arc > limit {
                break;
            }
            let world = {
                let d = self.dual.as_mut().expect("デュアルブラシ");
                d.pending.pop_front();
                d.world_radius
            };
            for _ in 0..dual_brush.count {
                let (mut x, mut y) = (at.x as f64, self.up(at.y));
                if dual_brush.scatter > 0.0 {
                    let reach =
                        dual_scale.map_or(0.0, |s| (s * world) as f64) * 2.0 * dual_brush.scatter;
                    let r = &mut self.dual.as_mut().expect("デュアルブラシ").random;
                    x += (r.next_double() * 2.0 - 1.0) * reach;
                    y += (r.next_double() * 2.0 - 1.0) * reach;
                }
                let Some(s) = dual_scale.or(scale) else {
                    continue;
                };
                let center = if x == at.x as f64 && dual_brush.scatter <= 0.0 {
                    at
                } else {
                    Vec2::new(x as f32, self.down(y))
                };
                self.dual_dab(doc, &dual_brush, center, s, world)?;
            }
        }
        Ok(())
    }

    /// 2 つ目の筆先のダブ 1 つ: 画面の上の形（2D と同じ式、筆圧・ゆらぎなし）で投影の画素の覆いを出し（面の向きでは弱めない）、
    /// テクセルごとの最大を溜める。対称の写しにも置く。作れなかったら（主のダブと同じく）ストロークを取り消す `Err`。
    fn dual_dab(
        &mut self,
        doc: &Document,
        dual_brush: &crate::DualBrush,
        at: Vec2,
        scale: f32,
        world: f32,
    ) -> Result<(), SurfaceStrokeError> {
        if !(at.x >= 0.0 && at.x < self.view.width && at.y >= 0.0 && at.y < self.view.height) {
            return Ok(());
        }
        let hit = pick(&self.geometry, &self.view, at)
            .filter(|h| self.material.is_none_or(|m| m == h.material));
        let shape = dual_brush.dab_shape(0.0, 0.0, (scale * world) as f64);
        let (stats, symmetry_note) = (self.stats, self.symmetry_note);
        let (result, _) = self.build_dab(
            doc,
            hit,
            at,
            scale,
            world,
            &Form::Shaped(shape),
            None,
            false,
        );
        (self.stats, self.symmetry_note) = (stats, symmetry_note);
        if let Some(why) = result.refusal {
            return Err(SurfaceStrokeError::Dab(why));
        }
        if let Some(d) = self.dual.as_mut() {
            for p in &result.pixels {
                d.cells.raise(p.x, p.y, p.coverage);
            }
            self.dual_bytes = d.cells.bytes();
        }
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
    /// 区画の一覧・デュアルブラシの溜まりと、ほかの投影の塗りがこのダブで使った区画を引いたもの。前のダブの区画は捨てて空けるので
    /// 引かない（どの投影の塗りの区画でも、古いものから捨てる）。
    fn room(&self, total: u64, except: Option<usize>) -> u64 {
        let clock = self.clock;
        let mut used = self.dual_bytes
            + self.projector.as_ref().map_or(0, |p| {
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

    /// 全部の投影の塗りの覚えを、予算から一覧（とデュアルブラシの溜まり）の分を引いた残りに収める（このダブで使った区画は残す）。
    fn evict(&mut self, total: u64) {
        let lists = self.projection_fixed_bytes() + self.dual_bytes;
        let clock = self.clock;
        let mut all: Vec<&mut SurfaceProjector> = self
            .projector
            .iter_mut()
            .chain(self.copy_projectors.iter_mut().flatten())
            .collect();
        evict_least_recent(&mut all, total.saturating_sub(lists), clock);
    }

    /// ダブの画素（元と、3D の対称の写しと、それら全部の 2D の対称の写しを 1 つにしたもの）と、ダブの面の点を作る。3D の写しまでは
    /// [`SurfaceStroke::build_model_dab`]、2D の写しは UV の平面の上で写す（`uv_symmetry::copy_by_canvas`。写しが多すぎればダブを断る）。
    #[allow(clippy::too_many_arguments)]
    fn build_dab(
        &mut self,
        doc: &Document,
        hit: Option<SurfaceHit>,
        at: Vec2,
        scale: f32,
        radius: f32,
        form: &Form<'_>,
        dual: Option<(DualBrushMode, &DualCells)>,
        weighted: bool,
    ) -> (SurfaceDabResult, Option<SurfaceHit>) {
        let _profile = profile::scope(profile::Stage::SurfaceProject);
        let (result, hit) = self.build_model_dab(doc, hit, at, scale, radius, form, dual, weighted);
        if self.canvas.is_empty() {
            return (result, hit);
        }
        (
            super::uv_symmetry::copy_by_canvas(result, &self.canvas, self.width, self.height),
            hit,
        )
    }

    /// ダブの画素（元と 3D の対称の写しを 1 つにしたもの）と、ダブの面の点を作る。at・scale は画面の中心と、モデルの単位 1 の画面の大きさ、radius は
    /// モデルの単位の半径（丸の大きさ・写しを探す距離）。面の点は、中心の下の塗るセットの面の点（hit）か、それが無ければ元の側の画素の点
    /// （[`SurfaceStroke::fallback_hit`]。元の側が何も塗らなければ None）。対称の写しは、その点を写して探す（中心がほかのセット・背景に
    /// あるダブでも、元の側が塗る縁を写しの側にも塗る）。dual はデュアルブラシの合わせ方と溜まり、weighted は面の向きで弱めるか。
    #[allow(clippy::too_many_arguments)]
    fn build_model_dab(
        &mut self,
        doc: &Document,
        hit: Option<SurfaceHit>,
        at: Vec2,
        scale: f32,
        radius: f32,
        form: &Form<'_>,
        dual: Option<(DualBrushMode, &DualCells)>,
        weighted: bool,
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
                // テクセルの画面の大きさは、アンチエイリアスの段があるストロークだけが持つ（なし は今と同じ大きさ・予算）
                Ok(p) => {
                    self.projector = Some(p.with_texel_metrics(self.anti_alias != AntiAlias::None))
                }
                Err(why) => return (SurfaceDabResult::default().reject(why), hit),
            }
        }
        // アンチエイリアスの帯の見積もりと画像の筆先の小さなダブ: ダブの中心のテクセルの画面の大きさ（中心が面に無ければ直前の値）
        let metric = if self.anti_alias == AntiAlias::None {
            None
        } else {
            let at_hit = hit.and_then(|h| self.projector.as_ref().and_then(|p| p.metric_at(&h)));
            if at_hit.is_some() {
                self.last_metric = at_hit;
            }
            at_hit.or(self.last_metric)
        };
        let cover = DabCover {
            form: match form {
                Form::Round { hardness } => CoverForm::Round(
                    RoundCover::new(screen_radius, *hardness)
                        .with_anti_alias(self.anti_alias, metric),
                ),
                Form::Shaped(shape) => {
                    // 形は y を上向きに測るので、画面（y は下向き）の値の xy の符号を反す
                    let shape = match metric {
                        Some(m) => {
                            shape.with_edge_in(self.anti_alias, &TexelMetric { xy: -m.xy, ..m })
                        }
                        None => *shape,
                    };
                    CoverForm::Shaped(shape.coverage_fn(), shape.reach())
                }
            },
            dual,
        };
        self.clock += 1;
        let total = self.memory_total(doc);
        let room = self.room(total, None);
        let clock = self.clock;
        let mut result = self
            .projector
            .as_mut()
            .expect("作った")
            .dab_with(at, &cover, weighted, room, clock);
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
                    // 写しの点を探す仕事が上限を超えた: ダブごと断る（呼び手がストロークを取り消す）
                    CopyHit::BudgetExceeded => {
                        return (result.reject(DabRefusal::BvhBudget), Some(hit));
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
                let transform = CopyTransform {
                    mirror: if reflected { sym.mirror } else { None },
                    radial: sym.radial.map(|r| (r, copy)),
                };
                let dab = if sym.ignore_visibility {
                    // 見えない面にも塗る写しは、写しの点のまわりの球の中の面（カメラによらない足跡）
                    match (&cover.form, cover.dual) {
                        (CoverForm::Round(round), None) => {
                            self.geometry.build_surface_dabs_anti_aliased(
                                &found,
                                radius,
                                self.width,
                                self.height,
                                self.view.position,
                                round.hardness(),
                                self.anti_alias,
                                &self.budget,
                                None,
                                true,
                            )
                        }
                        // 形のあるダブ・デュアルブラシ: 足跡のテクセルの点を元の側へ戻して元のカメラの画面へ写し、元のダブの形で読む
                        // （鏡映は左右が、放射状は向きが、写しに合わせて変わる）。丸は今までどおり球の中の距離で
                        (form, _) => {
                            // 丸の縁のアンチエイリアスは、写しの点の三角形のテクセルの大きさで（向きによらない帯）
                            let sphere = match form {
                                CoverForm::Round(round) => super::dab::SphereEdge::new(
                                    &self.geometry,
                                    &found,
                                    radius,
                                    round.hardness(),
                                    self.anti_alias,
                                    self.width,
                                    self.height,
                                ),
                                CoverForm::Shaped(..) => None,
                            };
                            let reach = match form {
                                CoverForm::Round(_) => {
                                    radius * sphere.map_or(1.0, |e| e.outer as f32)
                                }
                                CoverForm::Shaped(_, reach) => (*reach / scale as f64) as f32,
                            };
                            let view = self.view;
                            let center = found.position;
                            let cover_at = |position: Vec3, x: i32, y: i32| -> f32 {
                                let c = match form {
                                    CoverForm::Round(round) => match &sphere {
                                        Some(e) => e.cover_at_hit(
                                            (magnitude(position - center) / radius) as f64,
                                        ),
                                        None => coverage(
                                            magnitude(position - center) / radius,
                                            round.hardness(),
                                        ),
                                    },
                                    CoverForm::Shaped(shape, _) => {
                                        let back = transform.inverse_point(position.as_dvec3());
                                        match view.to_screen(back.as_vec3()) {
                                            Some(s) => shape.coverage(s.x - at.x, at.y - s.y),
                                            None => 0.0,
                                        }
                                    }
                                };
                                match cover.dual {
                                    Some((mode, cells)) => {
                                        combine_dual(mode, c, cells.get(x as u16, y as u16))
                                    }
                                    None => c,
                                }
                            };
                            self.geometry.build_surface_footprint(
                                &found,
                                reach,
                                self.width,
                                self.height,
                                self.view.position,
                                &self.budget,
                                &cover_at,
                            )
                        }
                    }
                } else {
                    let slot = copy as usize * 2 + reflected as usize;
                    if self.copy_projectors.len() <= slot {
                        self.copy_projectors.resize_with(slot + 1, || None);
                    }
                    if self.copy_projectors[slot].is_none() {
                        let p = self.projector.as_ref().expect("作った").copy(transform);
                        self.copy_projectors[slot] = Some(p);
                    }
                    let room = self.room(total, Some(slot));
                    let dab = self.copy_projectors[slot]
                        .as_mut()
                        .expect("作った")
                        .dab_with(at, &cover, weighted, room, clock);
                    if dab.refusal.is_none() {
                        self.evict(total);
                    }
                    dab
                };
                if let Some(why) = dab.refusal {
                    return (result.reject(why), Some(hit));
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

    /// ぼかしの 1 つのダブ。箱の範囲（半径 r）が塗るセットの UV の中に収まる画素は、今までどおり UV の画像の上の箱の平均（`apply_dab`）。
    /// ウィンドウが UV アイランドの縁にかかる画素は、面の上でまわりの 4 点（展開の図で UV の継ぎ目をまたぐ）を読み元にする（`apply_mapped_dab`）。
    /// 4 点は画素の点から、箱の平均と同じ広がり（1 軸の標準偏差）の所の 4 方向で、向きはダブごとに回す（重なるダブで方向の偏りが
    /// 残らない）。図に入らない縁の画素（つながらない面・対称の写しの側）は、UV の画像の上の同じ広がりの 4 点を読む。2 つは同じ
    /// ストロークの中で続けて塗るので 1 回の取り消しで戻る（縁の画素の読みは、アイランドの中の画素を書いた後の値）。展開の図が 1 回の操作の
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
        // ウィンドウがアイランドの縁にかかる画素（ウィンドウの中に、塗るセットの UV の外のテクセルがある）。ダブの箱の上の外のテクセルの累積和で数える
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
        // UV の画像の上の箱の平均で塗る（アイランドの中の画素と、図で読めなかった縁の画素）
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

/// 三角形の上のテクセル 1 つの、モデルの単位の大きさ（[`super::SurfaceTriangle::texel_size`]）。
fn texel_size(t: &super::SurfaceTriangle, width: i32, height: i32) -> f32 {
    t.texel_size(width, height)
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
pub(crate) fn gap(
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
