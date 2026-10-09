//! ブラシのストローク（C# の BrushStroke と BrushStroke.Effects の、1 つの面へ描く部分）。
//!
//! - 入力の点（手ぶれ補正の後、曲線なら曲線を 1 画素ほどの折れ線に刻んだ点）を結んだ線の上に、直径 × 間隔ごとの弧長でダブを
//!   置く（入力の区切り方によらない）。筆圧・傾きは各ダブの位置で線形に補間する。同じ位置の点はダブを増やさない。確定のときに
//!   終点へ余分なダブを置かない。抜きのあるストロークは、終わりから抜きの長さの内のダブを確定（か続きの入力）まで待たせる。
//! - 塗りはストロークの中で画素ごとに溜まる（Photoshop・CLIP STUDIO と同じ）: 各ダブは画素の「ストロークの覆い」を、
//!   ダブの天井（不透明度 × 筆圧 × ゆらぎ × 紙の質感）へ流量（流量 × 覆い × 筆圧 × ゆらぎ）の割合だけ寄せ、画素はストロークの
//!   前の色から毎回計算し直す。重なったダブがストロークの不透明度を超えることはない。
//! - ダブごとの色（色の変化の「描点ごと」）は、画素ごとにストロークの色（straight RGBA 0〜1、float）を持ち、各ダブが自分の色へ
//!   流量の割合だけ寄せる。デュアルブラシは同じ道筋に自分のダブを先に並べ、主のダブは自分の線の長さまでのものを使う。
//!   色とデュアルの乱数は位置のゆらぎと別の列なので、足してもダブの位置は動かない。
//! - 効果（ぼかし・指先・クローン）は、ダブの前に読む範囲を凍結し（[`effects`]）、覆いの割合でその色へ寄せる。
//! - 色の混ぜ（厚塗り。拡張: C# に無い）は、効果と同じくダブの前に下地を凍結し、画素ごとに下の色と筆の荷を混ぜた色を、ダブごとの色と同じ道
//!   （画素ごとの色の積み）で置く（`mix`・`mix_stroke`）。混ぜ方が切のブラシの画素は混ぜを足す前とバイト単位で同じ。
//! - 覆いは float（単精度）で持ち、画素の計算（覆い・天井と流量・覆いの寄せ・画素ごとの色・合成・効果の読み）も f32 の式
//!   （`rows`。合成は `crate::blend` の Normal とフェード）。ダブの位置・形・筆圧・ゆらぎは倍精度で求め、画素の式へ渡すときに f32 へ丸める。
//!   回転・傾き・線の向きは libm の三角関数（cos・sin・tan・atan・atan2）を通るので、ダブの形は libm（OS）によって 1 ULP ずれ得る。
//!
//! - ステンシルを使わないブラシの画素は、行ごとにレーンで描く（`rows`。道は AVX2・SSE4.1・NEON・スカラー。選択範囲・透明部分のロックは、
//!   色を塗る・消すだけのブラシなら行の核、画素ごとの色・効果のブラシでは画素ごとの式）。どの道も同じ式なので、結果のバイトは道と
//!   スレッド数によらず、画素ごとの式（[`apply_at`]）とも同じ。ワーカーで描くかは、箱の大きさに画素ごとの時間の見積もりを掛けて決める。
//!
//! 1 つのストロークは 1 つの面（レイヤーの 1 つのチャンネル）へ描く。始めたときの文書の選択範囲の内側だけを、選ばれた量の割合で
//! 変える（[`apply_at`] の 1 か所）。2D の対称（[`crate::CanvasSymmetry`]）は各ダブを写しへも置く（`symmetric`）。透明部分の
//! ロックは `keep_alpha`（ストロークを作るときに必ず決める）で、描く画素のアルファと透明画素の RGB を守る。C# の
//! マテリアルは文書がチャンネルごとの状態へ同じ入力を渡す。クローンは、見えているレイヤーの重なりを凍結した参照元（`sources`）も読め、
//! 3D の面のクローン・指先は、画素ごとの参照を呼び手が決める写像されたダブ（`sources`）で塗る。
//!
//! ```
//! use yolu_core::{builtin_tip, Brush, BrushSettings, Document, DualBrush, PaperTexture, Rgba8};
//! use yolu_core::glam::DVec2;
//! let mut doc = Document::new(256, 256).unwrap();
//! let layer = doc.add_layer("レイヤー 1").unwrap();
//! let mut brush = Brush::from(BrushSettings { radius: 12.0, color: Rgba8::new(40, 40, 160, 255), ..BrushSettings::default() });
//! brush.tip.image = builtin_tip("charcoal");      // 筆先の画像
//! brush.tip.follow_direction = true;              // 線の向きに沿わせる
//! brush.jitter.size = 0.3;                        // 大きさのゆらぎ
//! brush.seed = 7;                                 // 同じ種なら同じ線
//! brush.texture = Some(PaperTexture::new(builtin_tip("grain").unwrap(), 0.5));
//! brush.dual = Some(DualBrush { radius: 4.0, ..DualBrush::default() });
//! brush.color.hue = 0.2;                          // 色相のゆらぎ（ダブごと）
//! brush.assist.taper_in = 20.0;                   // 入り
//! brush.assist.curve = true;                      // 点の間を曲線で
//! let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
//! for (i, (x, y)) in [(20.0, 30.0), (90.0, 120.0), (200.0, 80.0)].into_iter().enumerate() {
//!     stroke.add_point(&mut doc, x, y, 0.4 + 0.3 * i as f64, DVec2::ZERO).unwrap();
//! }
//! assert!(doc.end_stroke(stroke).unwrap().changed); // 確定で、待たせた曲線の最後の区間も描く
//! ```

pub mod curve;
mod dynamics;
pub(crate) mod edge;
mod effects;
mod mix;
mod mix_stroke;
mod plan;
mod presets;
mod pressure;
#[doc(hidden)]
pub mod profile;
pub mod random;
mod rows;
mod settings;
mod sources;
mod stencil;
mod symmetric;
mod tip;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use glam::DVec2;
use rayon::prelude::*;

pub use dynamics::{hsv_to_rgb, pen_tilt, rgb_to_hsv};
pub use edge::AntiAlias;
pub(crate) use edge::{Edge, TexelMetric};
pub use mix::{ColorMix, MixGround, MixMode};
pub(crate) use plan::{DabPlan, StampControls};
pub use presets::{builtin_presets, BrushPreset};
pub(crate) use pressure::PressureScale;
pub use pressure::{PressureResponse, PressureResponses, MAX_CURVE_POINTS, STRAIGHT};
pub use settings::{
    Brush, BrushEffect, ColorDynamics, Controls, DualBrush, DualBrushMode, Jitter, PaperTexture,
    StrokeAssist, TextureMode, TipSelection, TipShape, MAX_FADE, MAX_STROKE_ASSIST,
};
pub use sources::{BrushMappedPixel, BrushSourceTap};
pub(crate) use sources::{CloneSource, CompositeTile};
pub use stencil::{
    linear_to_srgb, luminance, BrushStencil, ImageColorSpace, StencilImage, StencilMapping,
    StencilMode, StencilPoint, StencilSample, StencilTexel, StencilTiling,
};
pub use tip::{builtin_tip, BrushTip, BUILTIN_TIPS};

use crate::error::CoreError;
use crate::math::{clamp01, require_finite};
use crate::selection::{Amounts, SelectionMask};
use crate::surface::{Growth, LiveTile, Pixels, Surface, Tile};
use crate::types::{Channel, ChannelKind, Rgba8, TileCoord};
use crate::LayerId;
use dynamics::{f64_max, f64_min};
use effects::EffectFrame;
use mix::{MixDab, MixRun, MixTally};
use random::NetRandom;

/// ブラシの基本の設定（M1）。ストロークを始めたときに写して固定する（途中で変えても、そのストロークには効かない）。
/// 筆先・ゆらぎ・質感などを足すときは [`Brush`]（これを `base` に持つ）で始める。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSettings {
    /// 半径（画素）。0 < r ≤ 65536。
    pub radius: f64,
    /// 硬さ 0〜1。覆いが 1 のまま続く半径の割合（その外は smoothstep で 0 へ）。
    pub hardness: f64,
    /// ダブの間隔（公称の直径に対する割合、0.01〜4）。筆圧や入力の間隔によらない。
    pub spacing: f64,
    /// ストロークの不透明度（重なっても超えない天井）0〜1。
    pub opacity: f64,
    /// 流量（1 つのダブが天井へ寄せる割合）0〜1。
    pub flow: f64,
    /// 塗る色（straight）。消しゴムではアルファだけを消す強さに使う。
    pub color: Rgba8,
    /// 筆圧で大きさ・不透明度・流量を変えるか（それぞれ線形）。
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub pressure_flow: bool,
    /// 消しゴム（アルファを消す。色のアルファの割合だけ）。
    pub erase: bool,
    /// 拡張（C# に無い）: 丸い筆先の縁のアンチエイリアスと、小さなダブの濃さ（[`AntiAlias`]）。既定は なし（今の式）。
    pub anti_alias: AntiAlias,
}

impl Default for BrushSettings {
    /// C# の BrushSettings の既定値と同じ。
    fn default() -> Self {
        BrushSettings {
            radius: 16.0,
            hardness: 0.8,
            spacing: 0.15,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(255, 255, 255, 255),
            pressure_size: true,
            pressure_opacity: true,
            pressure_flow: false,
            erase: false,
            anti_alias: AntiAlias::None,
        }
    }
}

impl BrushSettings {
    /// C# の BrushSettings.Validate の M1 の部分。
    pub fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.radius, "radius")?;
        require_finite(self.hardness, "hardness")?;
        require_finite(self.spacing, "spacing")?;
        require_finite(self.opacity, "opacity")?;
        require_finite(self.flow, "flow")?;
        if self.radius <= 0.0 || self.radius > 65536.0 {
            return Err(CoreError::InvalidArgument("radius"));
        }
        if !(0.0..=1.0).contains(&self.hardness) {
            return Err(CoreError::InvalidArgument("hardness"));
        }
        if !(0.01..=4.0).contains(&self.spacing) {
            return Err(CoreError::InvalidArgument("spacing"));
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(CoreError::InvalidArgument("opacity"));
        }
        if !(0.0..=1.0).contains(&self.flow) {
            return Err(CoreError::InvalidArgument("flow"));
        }
        Ok(())
    }
}

/// ペンの入力 1 つ。画素の座標（左下原点、画素の中心は n + 0.5）。時刻は減ってはならない（速さの制御は秒を勧める）。
/// 傾きはペンの直立からの角度（ラジアン、キャンバスの X・Y 軸に沿って。0 は直立か傾きの情報なし）。[`Controls`] の傾きで使う。
/// 作るときは [`BrushSample::new`]（回転は [`BrushSample::with_rotation`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSample {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub time: f64,
    pub tilt: DVec2,
    /// ペンの軸の回転（ラジアン、キャンバスで反時計回り。0 は回転なしか情報なし）。拡張（C# に無い）: [`Controls::rotation_angle`] で使う。
    pub rotation: f64,
    /// 筆の速さ（画素 / 時刻の単位）。ストロークが手ぶれ補正の後の点の間から求めて入れる（渡した値は使わない）。
    pub(crate) speed: f64,
}

impl BrushSample {
    /// 有限でない値は断る。筆圧は 0〜1、傾きは ±π/2 に収める（C# の BrushSample と同じ）。
    pub fn new(x: f64, y: f64, pressure: f64, time: f64, tilt: DVec2) -> Result<Self, CoreError> {
        require_finite(x, "x")?;
        require_finite(y, "y")?;
        require_finite(pressure, "pressure")?;
        require_finite(time, "time")?;
        require_finite(tilt.x, "tilt")?;
        require_finite(tilt.y, "tilt")?;
        let max = std::f64::consts::FRAC_PI_2;
        Ok(BrushSample {
            x,
            y,
            pressure: clamp01(pressure),
            time,
            tilt: DVec2::new(tilt.x.clamp(-max, max), tilt.y.clamp(-max, max)),
            rotation: 0.0,
            speed: 0.0,
        })
    }
    /// ペンの軸の回転（ラジアン、キャンバスで反時計回り）を付けた写し。有限でない値は断る。
    pub fn with_rotation(self, rotation: f64) -> Result<Self, CoreError> {
        require_finite(rotation, "rotation")?;
        Ok(BrushSample { rotation, ..self })
    }
    const ZERO: BrushSample = BrushSample {
        x: 0.0,
        y: 0.0,
        pressure: 0.0,
        time: 0.0,
        tilt: DVec2::ZERO,
        rotation: 0.0,
        speed: 0.0,
    };
}

/// 角度（ラジアン）の cos と sin。角度が +0 のときは（libm の結果と同じ）1 と 0 をそのまま返す（回転しない筆先が大半で、三角関数を省く）。
#[inline]
fn cos_sin(angle: f64) -> (f64, f64) {
    if angle.to_bits() == 0 {
        (1.0, 0.0)
    } else {
        (angle.cos(), angle.sin())
    }
}

/// 角度 a から b へ短い向きに t だけ進んだ角度（回転の補間。差を ±π に折り返す）。
#[inline]
pub(crate) fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    if a == b {
        return a;
    }
    let tau = std::f64::consts::TAU;
    let mut d = (b - a) % tau;
    if d > std::f64::consts::PI {
        d -= tau;
    } else if d < -std::f64::consts::PI {
        d += tau;
    }
    a + d * t
}

/// 色の変化が意味を持つチャンネル（色の種類のもの。C# の BrushSettings.CarriesColor、標準では Color と Emission）。スカラーはデータ、
/// Normal はベクトルなので、色相や描画色/背景色で値を揺らすとデータを壊すだけになる。
pub fn carries_color(kind: ChannelKind) -> bool {
    kind == ChannelKind::Color
}

/// 3D の面などから渡す、ダブの中で重なりをまとめた画素と被覆率（C# の BrushPixel）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushPixel {
    pub x: i64,
    pub y: i64,
    pub coverage: f64,
}

/// ストロークが手を付けたタイル 1 枚: 画素ごとのストロークの覆い（0〜1）、ストロークの前のタイル（巻き戻し用の写し）、
/// ダブごとの色ならストロークの色（straight RGBA 0〜1。R・G・B・A の面を TileSize² ずつ並べる）。
pub(crate) struct StrokeTile {
    wash: Vec<f32>,
    pub before: Option<Tile>,
    /// `before` の読んだ中身（ストロークの間は持ったまま。画素ごとに読む所はこれを読む）。
    pub before_px: Option<Pixels>,
    paint: Option<Vec<f32>>,
    /// ステンシルがあるとき: ダブが初めて届いたときに読んだ量（NaN は未読）と色。ストロークの間、画素のステンシルの上の位置は
    /// 変わらないので 1 回だけ読む（重なったダブが何度も読まない）。
    stencil_amount: Option<Vec<f64>>,
    stencil_color: Option<Vec<Rgba8>>,
}

/// 予算（文書の設定をストロークの始めに写す。ストロークの間は、ほかの面は変わらない）。
#[derive(Clone, Copy)]
pub(crate) struct Budgets {
    pub growth: Growth,
    pub stroke: u64,
}

/// 道筋の上の描点（位置・筆圧・線の長さ・その時の線の向き・傾き）。抜きのために待たせるダブもこれ。3D の面のストロークも同じ形で
/// ダブの置き方（[`plan`]）に渡す（座標は y を上向きに直した画面の点）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingDab {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub arc: f64,
    pub direction: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
    /// 拡張: ペンの回転と筆の速さ。
    pub rotation: f64,
    pub speed: f64,
}

/// デュアルブラシのダブ（道筋の上の位置と線の長さ）。
#[derive(Clone, Copy)]
struct DualDab {
    x: f64,
    y: f64,
    arc: f64,
}

/// 効果のダブの状態（指先の前の位置、読み元の位置のずれ、枠の大きさ）。
#[derive(Default)]
struct EffectState {
    has_position: bool,
    x: f64,
    y: f64,
    offset_x: f64,
    offset_y: f64,
    /// 今のダブの読み元の枠のバイト（ダブの間だけ予算に数える。C# の effectScratchBytes）。
    scratch: u64,
    /// 効果のダブを 1 つでも始めたか（合成の参照元は最初のダブの前にしか決められない。C# の effectDabStarted）。
    started: bool,
}

/// 進行中のストロークの中身（文書が持つ）。
pub(crate) struct StrokeState {
    /// 透明部分のロック: 描く画素のアルファと透明画素の RGB を守る。作るときに必ず決める（[`StrokeState::new`]）。
    keep_alpha: bool,
    pub id: u64,
    pub layer: LayerId,
    pub layer_index: usize,
    pub channel: Channel,
    brush: Arc<Brush>,
    budgets: Budgets,
    width: i64,
    height: i64,
    tile_size: i64,
    // 道筋
    has_sample: bool,
    previous: BrushSample,
    distance_since_stamp: f64,
    /// 今の入力の区間の向き（ラジアン。線の向きに沿わせる筆先が使う）。
    direction: f64,
    // 手ぶれ補正: 糸の先（実際に描く点）。入り抜き: ここまでの線の長さと、抜きのために待たせているダブ
    has_pen: bool,
    pen: BrushSample,
    last_input: BrushSample,
    /// 速さ: 最後の筆の点（手ぶれ補正の後）と、その点での速さ。
    last_pen_point: Option<BrushSample>,
    last_speed: f64,
    stroke_length: f64,
    pending: VecDeque<PendingDab>,
    random: NetRandom,
    // 色の変化: ストロークの色（ダブごとでないとき・パスの面のダブ）、今のダブの色、色の乱数の列
    stroke_color: Rgba8,
    dab_color: Rgba8,
    color_random: Option<NetRandom>,
    tip_colors: bool,
    /// ステンシルの色を受けるチャンネル（色のモードで、ステンシルが名指すチャンネルへ色を塗るとき）。
    stencil_kind: Option<ChannelKind>,
    // デュアルブラシ: 2 つ目の筆先のダブと、画素ごとの最大の被覆率
    dual_random: Option<NetRandom>,
    dual_pending: VecDeque<DualDab>,
    dual_coverage: HashMap<TileCoord, Vec<f32>>,
    dual_since_stamp: f64,
    /// 次の筆先（順に使うとき）。
    tip_index: u64,
    // 曲線: 描いた点（curve_from）と、その前の点（curve_before）、まだ描いていない最新の点（curve_to）
    curve_points: u8,
    has_curve_before: bool,
    curve_before: BrushSample,
    curve_from: BrushSample,
    curve_to: BrushSample,
    effect: EffectState,
    /// クローンが読む合成（見えているレイヤーの重なりを、最初のダブの前に凍結したもの。None は今のレイヤーから読む）。
    source: Option<sources::CloneSource>,
    /// 効果の読み元の枠の領域（ダブの間で使い回す。予算に数えるのはダブの間だけ、C# と同じ）。
    frame_cache: Option<EffectFrame>,
    /// 画素ごとに筆圧を渡す呼び出し（3D の面）で、同じ筆圧の応えを何度も計算しないための、直前の筆圧とその係数。
    pressure_memo: Option<(f64, PressureScale)>,
    /// 色の混ぜが効くか（混ぜ方が切でなく、色のチャンネルを色で塗る: 消しゴム・効果のブラシ・データのチャンネル・マスクでは効かない）。
    mix_on: bool,
    /// 混ぜの状態（筆の荷・今の打点の値と下地の合計）。
    mix_run: MixRun,
    pub stamp_count: u64,
    pub sample_count: u64,
    /// 安全なタイルをワーカーで描いたダブの数。
    pub parallel_dabs: u64,
    pub tiles: HashMap<TileCoord, StrokeTile>,
    /// 巻き戻しの写し・覆い・ダブごとの色・デュアルの溜まりのバイト数（C# の RollbackBytes の、効果の枠を除く分）。
    pub rollback_bytes: u64,
    /// ストロークを始めたときの選択範囲（None は選択なし）。画素は選ばれた量の割合でだけ変わる（[`apply_at`]）。
    selection: Option<SelectionMask>,
    /// 3D の面のダブの、今のダブのゆらぎの係数（[`StrokeState::begin_surface_dab`] で覚える）。None は面のダブを始めていない
    /// （パスなど: 係数 1・紙の質感なしで塗る）。
    surface_look: Option<SurfaceDabLook>,
    /// 3D の対称（[`Brush::model_symmetry`]）の写しを作れなかった最後の理由。
    copy_note: Option<crate::geometry::MirrorOutcome>,
}

/// 面の画素の不透明度・流量の係数と、乗算でない紙の質感（合わせ方・質感の値・深さ）。
type SurfaceScales = ((f32, f32), Option<(DualBrushMode, f32, f32)>);

/// 3D の面のダブ 1 つの、ゆらぎ・フェード・傾き・速さの不透明度と流量の係数（2D のダブの形の `opacity_scale`・`flow_scale` と同じ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfaceDabLook {
    pub opacity: f64,
    pub flow: f64,
}

/// これを超える長さの区間（ダブの数）は断る（C# と同じ百万）。
const MAX_STAMPS_PER_SEGMENT: f64 = 1_000_000.0;
/// 入力の座標の範囲（C# と同じ）。
const MAX_COORDINATE: f64 = 10_000_000.0;
/// 曲線の区間を刻む折れ線の 1 本の長さ（画素）。弧と弦のずれは半径 R の曲がりで 1/(8R) px 以下（C# の CurvePieceLength）。
pub const CURVE_PIECE_LENGTH: f64 = 1.0;
/// 曲線の 1 区間を刻む数の上限（C# の MaxCurvePieces）。
pub const MAX_CURVE_PIECES: u32 = 65536;
/// 色の乱数の列（C# の ColorStream）とデュアルブラシの乱数の列（DualStream）の種へ混ぜる値。
const COLOR_STREAM: i32 = 0x2545F491;
pub(crate) const DUAL_STREAM: i32 = 0x5DEECE6;
/// 回した四角い筆先の角は √2·r まで届く（C# の 1.4142135623730951 と同じ double）。
const SQRT_2: f64 = std::f64::consts::SQRT_2;

impl StrokeState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: u64,
        layer: LayerId,
        layer_index: usize,
        channel: Channel,
        kind: ChannelKind,
        brush: Brush,
        budgets: Budgets,
        size: (u32, u32, u32),
        keep_alpha: bool,
    ) -> Self {
        let mut stroke_color = brush.base.color;
        let mut color_random = None;
        let mut tip_colors = false;
        if brush.effect.is_paint() && brush.color.is_active() && !brush.base.erase {
            // ストロークの色を最初に 1 回引く（ダブごとでないときはこの色で塗る。ダブごとなら、2D の描点と 3D の面のダブ
            // （`begin_surface_dab`）がダブごとに次の色を引く。面のダブを始めないパスの塗りは、この色のまま）
            let mut r = NetRandom::new(brush.seed ^ COLOR_STREAM);
            stroke_color = brush.color.next(brush.base.color, &mut r);
            color_random = Some(r);
            tip_colors = brush.color.per_tip;
        }
        let dual_random = brush
            .dual
            .as_ref()
            .map(|_| NetRandom::new(brush.seed ^ DUAL_STREAM));
        // ステンシルの色: 色のモードで、ステンシルが名指すチャンネルを塗る面だけ（消す・マスク・効果は量だけ）
        let stencil_kind = brush
            .stencil
            .as_ref()
            .filter(|st| {
                brush.effect.is_paint() && !brush.base.erase && st.paints_color_into(channel)
            })
            .map(|_| kind);
        let mix_on = brush.mix.is_active()
            && brush.effect.is_paint()
            && !brush.base.erase
            && carries_color(kind);
        StrokeState {
            keep_alpha,
            id,
            layer,
            layer_index,
            channel,
            random: NetRandom::new(brush.seed),
            budgets,
            width: size.0 as i64,
            height: size.1 as i64,
            tile_size: size.2 as i64,
            has_sample: false,
            previous: BrushSample::ZERO,
            distance_since_stamp: 0.0,
            direction: 0.0,
            has_pen: false,
            pen: BrushSample::ZERO,
            last_input: BrushSample::ZERO,
            last_pen_point: None,
            last_speed: 0.0,
            stroke_length: 0.0,
            pending: VecDeque::new(),
            stroke_color,
            dab_color: stroke_color,
            color_random,
            tip_colors,
            stencil_kind,
            dual_random,
            dual_pending: VecDeque::new(),
            dual_coverage: HashMap::new(),
            dual_since_stamp: 0.0,
            tip_index: 0,
            curve_points: 0,
            has_curve_before: false,
            curve_before: BrushSample::ZERO,
            curve_from: BrushSample::ZERO,
            curve_to: BrushSample::ZERO,
            effect: EffectState::default(),
            source: None,
            frame_cache: None,
            pressure_memo: None,
            mix_on,
            mix_run: MixRun::default(),
            stamp_count: 0,
            sample_count: 0,
            parallel_dabs: 0,
            tiles: HashMap::new(),
            rollback_bytes: 0,
            selection: None,
            surface_look: None,
            copy_note: None,
            brush: Arc::new(brush),
        }
    }

    /// 文書の今の選択範囲の内側だけに描く（C# の BrushStroke が始めに document.Selection を覚えるのと同じ）。
    pub(crate) fn with_selection(mut self, selection: Option<SelectionMask>) -> Self {
        self.selection = selection;
        self
    }

    /// マスクへのストローク（色を持たない面）にする: ステンシルの色を受けない（C# の ForChannel(null) の PaintedChannel = null）。
    #[allow(dead_code)]
    pub(crate) fn without_stencil_colour(mut self) -> Self {
        self.stencil_kind = None;
        self
    }

    pub(crate) fn set_budgets(&mut self, budgets: Budgets) {
        self.budgets = budgets;
    }

    /// このストロークが使うブラシ（始めたときに写して固定したもの）。
    pub(crate) fn brush(&self) -> &Arc<Brush> {
        &self.brush
    }

    pub(crate) fn last_time(&self) -> f64 {
        if self.has_pen {
            self.last_input.time
        } else {
            0.0
        }
    }

    /// 進行中の巻き戻しのバイト数（写し・覆い・ダブごとの色・デュアルの溜まり。C# の RollbackBytes）。
    pub(crate) fn rollback_total(&self) -> u64 {
        self.rollback_bytes + self.held_bytes()
    }

    /// 巻き戻しの外でストロークが持っているバイト: 今の効果のダブの枠と、凍結した合成の参照元（C# の effectScratchBytes と
    /// cloneSourceBytes）。タイルの写しなどを数える予算の確かめは、これも足す。
    fn held_bytes(&self) -> u64 {
        self.effect.scratch + self.source_bytes()
    }

    /// 凍結した合成の参照元のバイト（無ければ 0）。
    fn source_bytes(&self) -> u64 {
        self.source.as_ref().map_or(0, |s| s.bytes())
    }

    /// ストロークの予算（C# の EnsureEffectBudget: 見込みのバイトと今の効果の枠の和が予算を超えたら断る）。
    fn ensure_budget(&self, bytes: u64) -> Result<(), CoreError> {
        if bytes.saturating_add(self.held_bytes()) > self.budgets.stroke {
            Err(CoreError::StrokeBudgetExceeded)
        } else {
            Ok(())
        }
    }

    /// 入力の点を 1 つ足す（C# の Add）。changed に、画素の変わったタイルを足す。画素が変わったら true。
    pub(crate) fn add(
        &mut self,
        surface: &mut Surface,
        sample: BrushSample,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        if self.has_pen && sample.time < self.last_input.time {
            return Err(CoreError::InvalidArgument("入力の時刻が戻った"));
        }
        if sample.x.abs() > MAX_COORDINATE || sample.y.abs() > MAX_COORDINATE {
            return Err(CoreError::InvalidArgument("入力の座標が範囲外"));
        }
        self.last_input = sample;
        let stabilizer = self.brush.assist.stabilizer;
        if stabilizer <= 0.0 || !self.has_pen {
            self.has_pen = true;
            self.pen = sample;
            return self.add_pen_point(surface, sample, changed);
        }
        let Some((x, y)) = self
            .brush
            .assist
            .pull((self.pen.x, self.pen.y), (sample.x, sample.y))
        else {
            return Ok(false);
        };
        self.pen = BrushSample { x, y, ..sample };
        self.add_pen_point(surface, self.pen, changed)
    }

    /// 筆の点（手ぶれ補正の後）: そのまま道へ、または曲線の向きが分かるまで待たせる（C# の AddPenPoint）。
    /// 筆の速さ（拡張）はここで前の筆の点との距離 ÷ 時刻の差として入れる（時刻が進まなければ前の速さのまま、最初の点は 0）。
    fn add_pen_point(
        &mut self,
        surface: &mut Surface,
        mut sample: BrushSample,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        sample.speed = match self.last_pen_point {
            None => 0.0,
            Some(p) => {
                let dt = sample.time - p.time;
                if dt > 0.0 {
                    let (dx, dy) = (sample.x - p.x, sample.y - p.y);
                    (dx * dx + dy * dy).sqrt() / dt
                } else {
                    self.last_speed
                }
            }
        };
        self.last_pen_point = Some(sample);
        self.last_speed = sample.speed;
        if !self.brush.assist.curve {
            return self.add_path_point(surface, sample, true, changed);
        }
        match self.curve_points {
            0 => {
                let any = self.add_path_point(surface, sample, true, changed)?;
                self.curve_from = sample;
                self.curve_points = 1;
                Ok(any)
            }
            1 => {
                // 描いた点に重なる点は、直線のときと同じく筆圧などだけを次の区間の始まりに入れる
                let f = self.curve_from;
                if curve::coincident(f.x, f.y, sample.x, sample.y) {
                    let any = self.add_path_point(surface, sample, true, changed)?;
                    self.curve_from = sample;
                    Ok(any)
                } else {
                    self.curve_to = sample;
                    self.curve_points = 2;
                    Ok(false)
                }
            }
            _ => {
                // 待たせている点に重なる点は、その点の筆圧・傾き・時刻を新しくするだけ（長さ 0 の区間は作らない）
                let t = self.curve_to;
                if curve::coincident(t.x, t.y, sample.x, sample.y) {
                    self.curve_to = sample;
                    return Ok(false);
                }
                let any = self.draw_curve_segment(surface, sample.x, sample.y, changed)?;
                self.curve_to = sample;
                Ok(any)
            }
        }
    }

    /// 待たせている区間 curve_from → curve_to を、その前の点と次の点 (next_x, next_y) で形を決めて描く（C# の DrawCurveSegment）。
    fn draw_curve_segment(
        &mut self,
        surface: &mut Surface,
        next_x: f64,
        next_y: f64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let (a, b) = (self.curve_from, self.curve_to);
        let (before_x, before_y) = if self.has_curve_before {
            (self.curve_before.x, self.curve_before.y)
        } else {
            curve::reflect(b.x, b.y, a.x, a.y)
        };
        // 折れ線に刻む（弦の長さから数を決める）。ダブの数の上限は直線のときと同じく区間ごとに確かめる（刻んだ後では 1 本ずつは短い）
        let chord = ((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y)).sqrt();
        let count = f64_min(
            MAX_CURVE_PIECES as f64,
            f64_max(1.0, (chord / CURVE_PIECE_LENGTH).ceil()),
        ) as u32;
        let mut pieces = Vec::with_capacity(count as usize);
        let (mut length, mut last_x, mut last_y) = (0.0, a.x, a.y);
        for i in 1..=count {
            let t = i as f64 / count as f64;
            let (x, y) = if i == count {
                (b.x, b.y) // 端は制御点そのもの（丸めで揺らさない）
            } else {
                curve::point(before_x, before_y, a.x, a.y, b.x, b.y, next_x, next_y, t)
            };
            length += ((x - last_x) * (x - last_x) + (y - last_y) * (y - last_y)).sqrt();
            last_x = x;
            last_y = y;
            pieces.push(BrushSample {
                x,
                y,
                pressure: clamp01(a.pressure + (b.pressure - a.pressure) * t),
                time: a.time + (b.time - a.time) * t,
                tilt: DVec2::new(
                    a.tilt.x + (b.tilt.x - a.tilt.x) * t,
                    a.tilt.y + (b.tilt.y - a.tilt.y) * t,
                ),
                rotation: lerp_angle(a.rotation, b.rotation, t),
                speed: a.speed + (b.speed - a.speed) * t,
            });
        }
        let s = &self.brush.base;
        let spacing = f64_max(0.01, s.radius * 2.0 * s.spacing);
        if length / spacing > MAX_STAMPS_PER_SEGMENT {
            return Err(CoreError::InvalidArgument("1 区間のダブが百万を超える"));
        }
        if let Some(d) = &self.brush.dual {
            if length / f64_max(0.01, d.radius * 2.0 * d.spacing) > MAX_STAMPS_PER_SEGMENT {
                return Err(CoreError::InvalidArgument(
                    "1 区間のデュアルブラシのダブが百万を超える",
                ));
            }
        }
        let mut any = false;
        let last = pieces.len() - 1;
        for (i, piece) in pieces.into_iter().enumerate() {
            any |= self.add_path_point(surface, piece, i == last, changed)?;
        }
        self.curve_before = a;
        self.has_curve_before = true;
        self.curve_from = b;
        Ok(any)
    }

    /// 筆が実際に通る道の点: 間隔ごとにダブを置く（C# の AddPathPoint）。counted は曲線の途中の点では false
    /// （入力の点の数は曲線が通る点を数える）。
    fn add_path_point(
        &mut self,
        surface: &mut Surface,
        sample: BrushSample,
        counted: bool,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let mut any = false;
        let brush = self.brush.clone();
        if !self.has_sample {
            if brush.dual.is_some() {
                self.dual_pending.push_back(DualDab {
                    x: sample.x,
                    y: sample.y,
                    arc: 0.0,
                });
            }
            any |= self.emit(
                surface,
                PendingDab {
                    x: sample.x,
                    y: sample.y,
                    pressure: sample.pressure,
                    arc: 0.0,
                    direction: self.direction,
                    tilt_x: sample.tilt.x,
                    tilt_y: sample.tilt.y,
                    rotation: sample.rotation,
                    speed: sample.speed,
                },
                changed,
            )?;
            self.has_sample = true;
        } else {
            let p = self.previous;
            let dx = sample.x - p.x;
            let dy = sample.y - p.y;
            let length = (dx * dx + dy * dy).sqrt();
            if length > 0.0 {
                self.direction = dy.atan2(dx);
            }
            let s = &brush.base;
            let spacing = f64_max(0.01, s.radius * 2.0 * s.spacing);
            if length / spacing > MAX_STAMPS_PER_SEGMENT {
                return Err(CoreError::InvalidArgument("1 区間のダブが百万を超える"));
            }
            if length > 0.0 {
                if let Some(d) = &brush.dual {
                    // 2 つ目の筆先のダブは自分の間隔で先に並べておき、主のダブが自分の線の長さまでのものを使う（入力の区切り方によらない）
                    let dual_spacing = f64_max(0.01, d.radius * 2.0 * d.spacing);
                    if length / dual_spacing > MAX_STAMPS_PER_SEGMENT {
                        return Err(CoreError::InvalidArgument(
                            "1 区間のデュアルブラシのダブが百万を超える",
                        ));
                    }
                    let mut at = dual_spacing - self.dual_since_stamp;
                    while at <= length + 1e-9 {
                        let t = f64_min(1.0, at / length);
                        self.dual_pending.push_back(DualDab {
                            x: p.x + dx * t,
                            y: p.y + dy * t,
                            arc: self.stroke_length + at,
                        });
                        at += dual_spacing;
                    }
                    self.dual_since_stamp = length - (at - dual_spacing);
                    if self.dual_since_stamp < 1e-9 {
                        self.dual_since_stamp = 0.0;
                    }
                    if self.dual_since_stamp >= dual_spacing {
                        self.dual_since_stamp %= dual_spacing;
                    }
                }
                let mut position = spacing - self.distance_since_stamp;
                // 許しは入力の境での丸めを吸うだけ。溜めた距離は下で詰める
                while position <= length + 1e-9 {
                    let t = f64_min(1.0, position / length);
                    any |= self.emit(
                        surface,
                        PendingDab {
                            x: p.x + dx * t,
                            y: p.y + dy * t,
                            pressure: p.pressure + (sample.pressure - p.pressure) * t,
                            arc: self.stroke_length + position,
                            direction: self.direction,
                            tilt_x: p.tilt.x + (sample.tilt.x - p.tilt.x) * t,
                            tilt_y: p.tilt.y + (sample.tilt.y - p.tilt.y) * t,
                            rotation: lerp_angle(p.rotation, sample.rotation, t),
                            speed: p.speed + (sample.speed - p.speed) * t,
                        },
                        changed,
                    )?;
                    position += spacing;
                }
                self.stroke_length += length;
                any |= self.flush_pending(
                    surface,
                    self.stroke_length - brush.assist.taper_out,
                    None,
                    changed,
                )?;
                self.distance_since_stamp = length - (position - spacing);
                if self.distance_since_stamp < 1e-9 {
                    self.distance_since_stamp = 0.0;
                }
                if self.distance_since_stamp >= spacing {
                    self.distance_since_stamp %= spacing;
                }
            }
        }
        self.previous = sample;
        if counted {
            self.sample_count += 1;
        }
        Ok(any)
    }

    /// 線の長さ arc の所のダブ: すぐ置くか、終わりから抜きの長さの内なら待たせる（C# の Emit）。
    fn emit(
        &mut self,
        surface: &mut Surface,
        dab: PendingDab,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        if self.brush.assist.taper_out > 0.0 {
            self.pending.push_back(dab);
            return Ok(false);
        }
        let factor = self.taper(dab.arc, f64::INFINITY);
        self.stamp(surface, dab, factor, changed)
    }

    /// 待たせたダブを線の長さ limit まで置く。end（確定の時の線の長さ）があれば、そこへ向けて細る（C# の FlushPending）。
    fn flush_pending(
        &mut self,
        surface: &mut Surface,
        limit: f64,
        end: Option<f64>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let mut any = false;
        while let Some(&dab) = self.pending.front() {
            if dab.arc > limit {
                break;
            }
            self.pending.pop_front();
            let factor = self.taper(dab.arc, end.unwrap_or(f64::INFINITY));
            any |= self.stamp(surface, dab, factor, changed)?;
        }
        Ok(any)
    }

    /// 長さ end のストロークの、線の長さ arc の所のダブの大きさの係数（[`StrokeAssist::taper`]）。
    fn taper(&self, arc: f64, end: f64) -> f64 {
        self.brush.assist.taper(arc, end)
    }

    /// 入力の終わり: 手ぶれ補正の筆を最後の入力の点まで描き、待たせている曲線の区間と抜きのダブを置く（C# の FinishInput）。
    pub(crate) fn finish_input(
        &mut self,
        surface: &mut Surface,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let mut any = false;
        if self.has_pen
            && self.brush.assist.stabilizer > 0.0
            && (self.pen.x != self.last_input.x || self.pen.y != self.last_input.y)
        {
            self.pen = self.last_input;
            any |= self.add_pen_point(surface, self.last_input, changed)?;
        }
        if self.curve_points == 2 {
            // 最後の区間: その先は無いので、終わりの点の向こうへ折り返した点で向きを決める
            let (f, t) = (self.curve_from, self.curve_to);
            let (end_x, end_y) = curve::reflect(f.x, f.y, t.x, t.y);
            any |= self.draw_curve_segment(surface, end_x, end_y, changed)?;
            self.curve_points = 1;
        }
        any |= self.flush_pending(surface, f64::INFINITY, Some(self.stroke_length), changed)?;
        Ok(any)
    }

    /// 描点 1 つ（C# の Stamp）: フェード・傾き・ゆらぎ・散布・数・筆先の選び方を当てて、ダブを置く。
    fn stamp(
        &mut self,
        surface: &mut Surface,
        dab: PendingDab,
        size_factor: f64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let _profile = profile::scope(profile::Stage::Stamp);
        let index = self.stamp_count;
        self.stamp_count += 1;
        let brush = self.brush.clone();
        if brush.dual.is_some() {
            self.stamp_dual(dab.arc)?;
        }
        let c = brush.stamp_controls(&dab, index);
        let s = &brush.base;
        let mut any = false;
        for _ in 0..brush.jitter.count {
            if self.tip_colors {
                let r = self.color_random.as_mut().expect("色の乱数");
                self.dab_color = brush.color.next(s.color, r);
            }
            let Some(plan) = brush.next_dab(
                &c,
                &dab,
                s.radius * c.size_pressure * size_factor,
                s.radius,
                &mut self.random,
                &mut self.tip_index,
            ) else {
                continue;
            };
            let shape = brush.dab_shape(&plan, &c).with_edge(s.anti_alias);
            any |= self.dab(surface, &brush, &shape, changed)?;
        }
        Ok(any)
    }

    /// 2 つ目の筆先のダブを線の長さ limit まで、画素ごとの溜まり（最大）へ置く（C# の StampDual）。
    fn stamp_dual(&mut self, limit: f64) -> Result<(), CoreError> {
        let _profile = profile::scope(profile::Stage::Dual);
        let brush = self.brush.clone();
        let dual = brush.dual.as_ref().expect("デュアルブラシ");
        while let Some(&d) = self.dual_pending.front() {
            if d.arc > limit {
                break;
            }
            self.dual_pending.pop_front();
            for _ in 0..dual.count {
                let (mut cx, mut cy) = (d.x, d.y);
                if dual.scatter > 0.0 {
                    let reach = dual.radius * 2.0 * dual.scatter;
                    let r = self.dual_random.as_mut().expect("デュアルの乱数");
                    cx += (r.next_double() * 2.0 - 1.0) * reach;
                    cy += (r.next_double() * 2.0 - 1.0) * reach;
                }
                self.dual_dab_at(dual, cx, cy)?;
            }
        }
        Ok(())
    }

    /// 2 つ目の筆先のダブ 1 つ（C# の DualDabAt）。丸い筆先も回転の式で測る（C# と同じ。角度 0 でも主の丸の近道とは丸めが違う）。
    fn dual_dab_at(&mut self, dual: &DualBrush, x: f64, y: f64) -> Result<(), CoreError> {
        let radius = dual.radius;
        let shape = plan::dual_shape(dual, x, y, radius, self.brush.base.anti_alias);
        let extent = if !shape.edge.is_off() {
            rows::dual_cover_shape(&shape).reach()
        } else if dual.tip.is_none() {
            radius
        } else {
            radius * SQRT_2
        };
        let min_x = ((x - extent - 0.5).ceil() as i64).max(0);
        let max_x = ((x + extent - 0.5).floor() as i64).min(self.width - 1);
        let min_y = ((y - extent - 0.5).ceil() as i64).max(0);
        let max_y = ((y + extent - 0.5).floor() as i64).min(self.height - 1);
        if self.brush.symmetry.enabled() || self.brush.model_symmetry.is_some() {
            // 写しは元のダブがキャンバスの外でもキャンバスにかかり得る（C# も外接の箱を見る前に分ける）。3D の対称だけで写しの無い
            // ダブ（中心が面に無い）は、普通のダブ
            if let Some(maps) = self.copy_maps(x, y, radius)? {
                return self.symmetric_dual_dab(&shape, extent, &maps);
            }
        }
        if min_x > max_x || min_y > max_y {
            return Ok(());
        }
        let ts = self.tile_size_i64();
        // タイルごとに 1 回だけ溜まりを引く（画素の順は C# の行の順と違うが、各画素は自分の値と最大を取るだけなので結果は同じ。
        // 新しいタイルの予算は足していく一方なので、超えるかどうかも順によらない。超えたらストロークごと取り消す）
        for ty in min_y / ts..=max_y / ts {
            for tx in min_x / ts..=max_x / ts {
                let coord = TileCoord::new(tx as u32, ty as u32);
                let xs = (min_x.max(tx * ts), max_x.min(tx * ts + ts - 1));
                let ys = (min_y.max(ty * ts), max_y.min(ty * ts + ts - 1));
                let (origin_x, origin_y) = (tx * ts, ty * ts);
                let mut cells = self.dual_coverage.remove(&coord);
                let result = {
                    let mut alloc = || {
                        let next = self.rollback_bytes + 64 + (ts * ts * 4) as u64;
                        self.ensure_budget(next)?;
                        self.rollback_bytes = next;
                        Ok(vec![0.0; (ts * ts) as usize])
                    };
                    rows::dual_tile(
                        &shape,
                        xs,
                        ys,
                        (origin_x, origin_y, ts as usize),
                        &mut cells,
                        &mut alloc,
                    )
                };
                if let Some(c) = cells {
                    self.dual_coverage.insert(coord, c);
                }
                result?;
            }
        }
        Ok(())
    }

    fn tile_size_i64(&self) -> i64 {
        self.tile_size
    }

    /// ダブ 1 つ（C# の Dab と DabPixels）。タイルごとに処理する（各画素は自分の入力だけで決まるので、画素を回る順は結果を変えない。
    /// 予算の確かめは増える一方なので、超えるかどうかも順によらない）。
    ///
    /// 外接の箱が [`PARALLEL_DAB_PIXELS`] 以上で複数のタイルにかかるダブは、このダブで起こり得る写しと確保を全部足しても予算に
    /// 収まるとき（普段はいつも）、タイルごとにワーカーで描き、増えたバイトを後で足す。どの確かめも失敗し得ないので、写すタイル・
    /// 確保・画素は呼んだスレッドだけで描いたときと同じになる。収まらないかもしれないときは、C# と同じく呼んだスレッドで順に描く。
    fn dab(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        shape: &DabShape<'_>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let extent = shape.reach();
        let (w, h) = (self.width, self.height);
        let (x, y) = (shape.x, shape.y);
        let min_x = ((x - extent - 0.5).ceil() as i64).max(0);
        let max_x = ((x + extent - 0.5).floor() as i64).min(w - 1);
        let min_y = ((y - extent - 0.5).ceil() as i64).max(0);
        let max_y = ((y + extent - 0.5).floor() as i64).min(h - 1);
        if brush.symmetry.enabled() || brush.model_symmetry.is_some() {
            // 写しは元のダブがキャンバスの外でもキャンバスにかかり得る（C# も外接の箱を見る前に分ける）。3D の対称だけで写しの無い
            // ダブ（中心が面に無い）は、普通のダブ
            if let Some(maps) = self.copy_maps(x, y, shape.radius)? {
                return self.symmetric_dab(surface, brush, shape, extent, &maps, changed);
            }
        }
        if min_x > max_x || min_y > max_y {
            return Ok(false);
        }
        if brush
            .stencil
            .as_ref()
            .is_some_and(|st| st.canvas_to_image().is_none())
        {
            return Err(CoreError::Unsupported(
                "ステンシルにキャンバスからの写しが無いので、2D のダブは読めない",
            ));
        }
        let prepared = {
            let _profile = profile::scope(profile::Stage::Prepare);
            self.prepare_dab(
                surface,
                brush,
                (min_x, max_x),
                (min_y, max_y),
                x,
                y,
                shape.pressure,
                true,
            )?
        };
        let frame = match prepared {
            Prepared::Skip => return Ok(false),
            Prepared::Paint => None,
            Prepared::Effect(f) => Some(f),
        };
        let result = {
            let _profile = profile::scope(profile::Stage::Pixels);
            self.dab_pixels(
                surface,
                brush,
                shape,
                frame.as_ref(),
                (min_x, max_x),
                (min_y, max_y),
                changed,
            )
        };
        self.effect.scratch = 0; // ReleaseEffectDab
        self.frame_cache = frame; // 領域は次のダブで使い回す（中身は作り直す）
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn dab_pixels(
        &mut self,
        surface: &mut Surface,
        brush: &Brush,
        shape: &DabShape<'_>,
        frame: Option<&EffectFrame>,
        xr: (i64, i64),
        yr: (i64, i64),
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let ts = surface.tile_size() as i64;
        let (min_x, max_x) = xr;
        let (min_y, max_y) = yr;
        // ダブが触るタイル（小さなダブは 4 枚までなので、置き場はスタック）
        let tiles_across = max_x / ts - min_x / ts + 1;
        let tile_count = (tiles_across * (max_y / ts - min_y / ts + 1)) as usize;
        let mut inline_spans = [(TileCoord::new(0, 0), (0, 0), (0, 0)); 4];
        let mut heap_spans: Vec<TileSpan> = Vec::new();
        let spans: &mut [TileSpan] = if tile_count <= inline_spans.len() {
            &mut inline_spans[..tile_count]
        } else {
            heap_spans.resize(tile_count, inline_spans[0]);
            &mut heap_spans
        };
        let mut k = 0;
        for ty in min_y / ts..=max_y / ts {
            for tx in min_x / ts..=max_x / ts {
                let xs = (min_x.max(tx * ts), max_x.min(tx * ts + ts - 1));
                let ys = (min_y.max(ty * ts), max_y.min(ty * ts + ts - 1));
                spans[k] = (TileCoord::new(tx as u32, ty as u32), xs, ys);
                k += 1;
            }
        }
        let spans: &[TileSpan] = spans;
        let paint = self.paint(brush, frame);
        let parallel = spans.len() > 1
            && rayon::current_num_threads() > 1
            && worth_parallel(
                (max_x - min_x + 1) * (max_y - min_y + 1),
                rows::cost_per_pixel(&paint, shape, self.selection.is_none(), ts as usize),
            )
            && self.fits_any_order(surface, spans, &paint);
        if parallel {
            self.parallel_dabs += 1;
            return self.dab_parallel(surface, &paint, shape, spans, changed);
        }
        let mut any = false;
        for &(coord, xs, ys) in spans {
            let dual = self.dual_coverage.remove(&coord);
            let r = self.with_tile(surface, &paint, coord, |cx, held, live| {
                let _profile = profile::scope(profile::Stage::Kernel);
                dab_tile(cx, held, live, dual.as_deref(), shape, coord, xs, ys)
            });
            if let Some(d) = dual {
                self.dual_coverage.insert(coord, d);
            }
            if r? {
                changed.push(coord);
                any = true;
            }
        }
        Ok(any)
    }

    /// 画素の処理が読む、ストロークとダブの値。
    fn paint<'a>(&self, brush: &'a Brush, frame: Option<&'a EffectFrame>) -> Paint<'a> {
        let effect = match brush.effect {
            BrushEffect::Paint => EffectKind::Paint,
            BrushEffect::Blur { radius } => EffectKind::Blur(radius as i64),
            BrushEffect::Smudge { strength } => EffectKind::Smudge(strength),
            BrushEffect::Clone { .. } => EffectKind::Clone,
        };
        let mix = if self.mix_on { self.mix_run.dab } else { None };
        Paint {
            keep_alpha: self.keep_alpha,
            s: &brush.base,
            effect,
            stroke_color: self.stroke_color,
            dab_color: self.dab_color,
            // 画素ごとの色（ダブごとの色か、色の混ぜ）。天井に届いた画素も、色は寄せ続ける
            tip_colors: self.tip_colors || mix.is_some(),
            stop_at_ceiling: !(self.tip_colors || mix.is_some()) && brush.effect.is_paint(),
            mix,
            stencil: brush.stencil.as_deref(),
            stencil_kind: self.stencil_kind,
            frame,
            mapped: None,
            offset_x: self.effect.offset_x,
            offset_y: self.effect.offset_y,
            width: self.width,
            height: self.height,
            scratch: self.held_bytes(),
        }
    }

    /// 効果のダブの読み元を凍結する（C# の PrepareEffectDab）。指先の最初のダブ（と止まったままのダブ）は位置を覚えるだけ。
    fn prepare_effect_dab(
        &mut self,
        surface: &Surface,
        brush: &Brush,
        xr: (i64, i64),
        yr: (i64, i64),
        x: f64,
        y: f64,
    ) -> Result<Prepared, CoreError> {
        let (radius, clone) = match brush.effect {
            BrushEffect::Paint => return Ok(Prepared::Paint),
            BrushEffect::Blur { radius } => (radius as i64, false),
            BrushEffect::Smudge { .. } => (0, false),
            BrushEffect::Clone { .. } => (0, true),
        };
        self.effect.scratch = 0;
        self.effect.started = true;
        match brush.effect {
            BrushEffect::Smudge { .. } => {
                let (dx, dy) = (self.effect.x - x, self.effect.y - y);
                let first = !self.effect.has_position;
                self.effect.has_position = true;
                self.effect.x = x;
                self.effect.y = y;
                if first || (dx == 0.0 && dy == 0.0) {
                    return Ok(Prepared::Skip);
                }
                self.effect.offset_x = dx;
                self.effect.offset_y = dy;
            }
            BrushEffect::Clone { offset } => {
                self.effect.offset_x = offset.x;
                self.effect.offset_y = offset.y;
            }
            _ => {
                self.effect.offset_x = 0.0;
                self.effect.offset_y = 0.0;
            }
        }
        let (ox, oy) = if radius > 0 {
            (0.0, 0.0)
        } else {
            (self.effect.offset_x, self.effect.offset_y)
        };
        let edge = if radius > 0 { 0 } else { 1 };
        let x0 = 0.max((xr.0 as f64 + ox).floor() as i64 - radius);
        let y0 = 0.max((yr.0 as f64 + oy).floor() as i64 - radius);
        let x1 = (self.width - 1).min((xr.1 as f64 + ox).ceil() as i64 + radius + edge);
        let y1 = (self.height - 1).min((yr.1 as f64 + oy).ceil() as i64 + radius + edge);
        if x1 < x0 || y1 < y0 {
            return Ok(Prepared::Skip);
        }
        let cells = ((x1 - x0 + 1) * (y1 - y0 + 1)) as u64;
        let bytes = cells * 4
            + if radius > 0 {
                ((x1 - x0 + 2) * (y1 - y0 + 2) * 32) as u64
            } else {
                0
            };
        if self
            .rollback_bytes
            .saturating_add(self.source_bytes())
            .saturating_add(bytes)
            > self.budgets.stroke
        {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        self.effect.scratch = bytes;
        let ts = surface.tile_size() as i64;
        let (fw, fh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut frame = match self.frame_cache.take() {
            Some(f) => f.reset(x0, y0, fw, fh),
            None => EffectFrame::new(x0, y0, fw, fh),
        };
        // 合成の参照元があれば、凍結した合成を読む（書き込み中の面は読まない）
        if let Some(source) = &self.source {
            for py in y0..=y1 {
                let mut px = x0;
                while px <= x1 {
                    let tx = px / ts;
                    let end = x1.min(tx * ts + ts - 1);
                    let out = frame.row_mut(py, px, (end - px + 1) as usize);
                    source.read_row(py, px, out);
                    px = end + 1;
                }
            }
            if radius > 0 {
                frame.build_integral();
            }
            return Ok(Prepared::Effect(frame));
        }
        // タイルは読む行の区間ごとに引く。クローンは既に触ったタイルだけ巻き戻しの写し（ストロークの前）を読む
        for py in y0..=y1 {
            let ty = py / ts;
            let row = ((py - ty * ts) * ts) as usize;
            let mut px = x0;
            while px <= x1 {
                let tx = px / ts;
                let coord = TileCoord::new(tx as u32, ty as u32);
                let tile: Option<Pixels> = match (clone, self.tiles.get(&coord)) {
                    (true, Some(held)) => held.before_px.clone(),
                    _ => surface.read(coord)?,
                };
                let end = x1.min(tx * ts + ts - 1);
                let len = (end - px + 1) as usize;
                let out = frame.row_mut(py, px, len);
                match &tile {
                    None => {} // 枠は透明で始まる
                    Some(Pixels::Uniform(c)) => out.fill(*c),
                    Some(Pixels::Data(d)) => {
                        let start = (row + (px - tx * ts) as usize) * 4;
                        for (o, p) in out
                            .iter_mut()
                            .zip(d[start..start + len * 4].chunks_exact(4))
                        {
                            *o = Rgba8::from_slice(p);
                        }
                    }
                }
                px = end + 1;
            }
        }
        if radius > 0 {
            frame.build_integral();
        }
        Ok(Prepared::Effect(frame))
    }

    /// このダブのタイルで起こり得る写し（覆い・巻き戻しの写し・ダブごとの色）と面の確保を全部足しても、どちらの予算にも
    /// 収まるか（控えめな見積もり）。
    fn fits_any_order(&self, surface: &Surface, spans: &[TileSpan], paint: &Paint<'_>) -> bool {
        let full = surface.tile_bytes() as u64;
        let tip = if paint.tip_colors { full * 4 } else { 0 };
        // ステンシルがあれば画素ごとに読んだ量と色（12 バイト）も（覆いと合わせて 16 バイト）
        let tip = tip + if paint.stencil.is_some() { full * 3 } else { 0 };
        let (mut capture, mut growth) = (0u64, 0u64);
        for &(coord, _, _) in spans {
            let live = surface.tile(coord);
            if !self.tiles.contains_key(&coord) {
                capture += 64 + full + tip + live.map_or(0, |t| t.byte_size());
            }
            growth += match live {
                None => full,
                Some(Tile::Uniform(_)) => full - 4,
                Some(Tile::Data(_)) => 0,
            };
        }
        self.rollback_bytes + capture + self.held_bytes() <= self.budgets.stroke
            && self
                .budgets
                .growth
                .ensure(surface.allocated, growth as i64)
                .is_ok()
    }

    /// ダブのタイルをワーカーで 1 枚ずつ描く（予算は fits_any_order で確かめ済み）。各ワーカーは自分のタイルの持ち分と面のタイルだけを
    /// 書き、増えたバイトを返す。
    fn dab_parallel(
        &mut self,
        surface: &mut Surface,
        paint: &Paint<'_>,
        shape: &DabShape<'_>,
        spans: &[TileSpan],
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let ts = surface.tile_size() as usize;
        let mut work: Vec<(TileSpan, Option<StrokeTile>, LiveTile)> =
            Vec::with_capacity(spans.len());
        for &span in spans {
            let coord = span.0;
            match LiveTile::take(surface, coord) {
                Ok(live) => work.push((span, self.tiles.remove(&coord), live)),
                Err(e) => {
                    // 読めないタイルがあれば何も描かずに戻す（取り出したタイルは元の場所へ）
                    for ((coord, _, _), held, live) in work {
                        if let Some(h) = held {
                            self.tiles.insert(coord, h);
                        }
                        live.put_back(surface, coord);
                    }
                    return Err(e);
                }
            }
        }
        let dual = &self.dual_coverage;
        let selection = self.selection.as_ref();
        let unlimited = Budgets {
            growth: Growth {
                budget: u64::MAX,
                others: 0,
            },
            stroke: u64::MAX,
        };
        let results: Vec<(bool, u64, u64, MixTally)> = work
            .par_iter_mut()
            .map(|((coord, xs, ys), held, live)| {
                let (mut rollback, mut allocated) = (0u64, 0u64);
                let mut cx = PixelContext {
                    paint,
                    budgets: unlimited,
                    rollback_bytes: &mut rollback,
                    allocated: &mut allocated,
                    tile_size: ts,
                    selected: Selected::of(selection, *coord),
                    tally: MixTally::default(),
                };
                let cells = dual.get(coord).map(|v| &v[..]);
                let painted = dab_tile(&mut cx, held, live, cells, shape, *coord, *xs, *ys)
                    .expect("予算は確かめ済みなので失敗しない");
                let tally = cx.tally;
                (painted, rollback, allocated, tally)
            })
            .collect();
        let mut any = false;
        for (((coord, _, _), held, live), (painted, rollback, allocated, tally)) in
            work.into_iter().zip(results)
        {
            // 下地の合計は、順に描くときと同じくタイルの順に足す（どの経路も同じ足し算の列）
            self.mix_run.tally.merge(tally);
            if let Some(h) = held {
                self.tiles.insert(coord, h);
            }
            live.put_back(surface, coord);
            self.rollback_bytes += rollback;
            surface.allocated += allocated;
            if painted {
                changed.push(coord);
                any = true;
            }
        }
        Ok(any)
    }

    /// 画素ごとに筆圧を渡す呼び出し用: 筆圧を応えに通した係数。同じ筆圧が続く間（1 つのダブの画素）は直前の結果を使う。
    fn shaped_pressure(&mut self, brush: &Brush, pressure: f64) -> PressureScale {
        match self.pressure_memo {
            Some((raw, scale)) if raw == pressure => scale,
            _ => {
                let scale = brush.pressure_scale(pressure);
                self.pressure_memo = Some((pressure, scale));
                scale
            }
        }
    }

    /// 3D の面のダブを 1 つ始める（2D の描点のダブの数のループの 1 回にあたる）: ダブごとの色（色の変化の「描点ごと」）を次の色へ進め、
    /// 塗るダブなら、そのゆらぎの係数を覚える。覚えた係数と紙の質感は、この後の面の画素の塗り（`apply_pixel`・`apply_dab`・
    /// `apply_mapped_dab`）が使う。大きさが 0 で塗らないダブ（`look` が None）も、2D と同じく色の乱数は 1 つ進める。
    pub(crate) fn begin_surface_dab(&mut self, look: Option<SurfaceDabLook>) {
        if self.tip_colors {
            let r = self.color_random.as_mut().expect("色の乱数");
            self.dab_color = self.brush.color.next(self.brush.base.color, r);
        }
        if look.is_some() {
            self.surface_look = look;
        }
    }

    /// 面の画素 (x, y) の不透明度・流量の係数と、乗算でない紙の質感（合わせ方・質感の値・深さ）。面のダブを始めていなければ係数 1・質感なし
    /// （今までの面の塗りと同じ）。紙の質感は 2D と同じく文書の画素の座標で読み、乗算は天井の係数に掛ける（2D の行の核と同じ f32 の式）。
    /// 乗算の質感で天井の係数が 0 以下の画素は None（塗らない）。
    fn surface_scales(&self, x: i64, y: i64) -> Option<SurfaceScales> {
        let Some(look) = self.surface_look else {
            return Some(((1.0, 1.0), None));
        };
        let (opacity, flow) = (look.opacity as f32, look.flow as f32);
        let Some(tex) = self.brush.texture.as_ref().filter(|t| t.depth > 0.0) else {
            return Some(((opacity, flow), None));
        };
        let grain = rows::Grain::new(tex, y);
        let depth = tex.depth as f32;
        match tex.mode.as_blend() {
            None => {
                let ceiling = rows::texture_scale_one(&grain, depth, opacity, x);
                (ceiling > 0.0).then_some(((ceiling, flow), None))
            }
            Some(mode) => Some((
                (opacity, flow),
                Some((mode, rows::grain_one(&grain, x), depth)),
            )),
        }
    }

    /// 与えた覆いを 1 画素に塗る（C# の ApplyPixel。メッシュのダブ向け。筆圧で大きさは変えない。色は今のダブの色で、3D の面のダブは
    /// `begin_surface_dab` でダブごとに進める）。
    /// キャンバスの外は何もしない。効果のブラシは断る（読み元を凍結するには [`StrokeState::apply_dab`]）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_pixel(
        &mut self,
        surface: &mut Surface,
        x: i64,
        y: i64,
        coverage: f64,
        pressure: f64,
        at: Option<StencilPoint>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        require_finite(coverage, "coverage")?;
        require_finite(pressure, "pressure")?;
        if !(0.0..=1.0).contains(&coverage) || !(0.0..=1.0).contains(&pressure) {
            return Err(CoreError::InvalidArgument("coverage/pressure"));
        }
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return Ok(false);
        }
        if !self.brush.effect.is_paint() {
            return Err(CoreError::Unsupported(
                "効果のブラシは画素ごとには塗れない（apply_dab で読み元を凍結する）",
            ));
        }
        if self.mix_on {
            return Err(CoreError::Unsupported(
                "混ぜるブラシは画素ごとには塗れない（apply_dab で下地を凍結する）",
            ));
        }
        let Some((scales, paper)) = self.surface_scales(x, y) else {
            return Ok(false);
        };
        let brush = self.brush.clone();
        let pressure = self.shaped_pressure(&brush, pressure);
        let paint = self.paint(&brush, None);
        let ts = surface.tile_size() as i64;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        let local = ((y % ts) * ts + x % ts) as usize;
        let done = self.with_tile(surface, &paint, coord, |cx, held, live| {
            apply_at::<false>(
                cx,
                held,
                live,
                coord,
                local,
                coverage as f32,
                pressure,
                scales,
                paper,
                at,
            )
        })?;
        if done {
            changed.push(coord);
        }
        Ok(done)
    }

    /// 面のダブを丸ごと塗る（C# の ApplyDab）。効果の読み元は、どの画素を書くより前に凍結する。指先の中心はキャンバスの画素の座標で、
    /// 継ぎ目をまたぐときは呼び手が [`StrokeState::reset_effect_direction`] する。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_dab(
        &mut self,
        surface: &mut Surface,
        pixels: &[BrushPixel],
        center: DVec2,
        pressure: f64,
        points: Option<&[StencilPoint]>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        if points.is_some_and(|p| p.len() != pixels.len()) {
            return Err(CoreError::InvalidArgument(
                "ステンシルの点は画素ごとに 1 つ",
            ));
        }
        require_finite(center.x, "center")?;
        require_finite(center.y, "center")?;
        require_finite(pressure, "pressure")?;
        if center.x.abs() > MAX_COORDINATE
            || center.y.abs() > MAX_COORDINATE
            || !(0.0..=1.0).contains(&pressure)
        {
            return Err(CoreError::InvalidArgument("dab"));
        }
        let (mut x0, mut y0, mut x1, mut y1) = (self.width, self.height, -1i64, -1i64);
        for p in pixels {
            require_finite(p.coverage, "coverage")?;
            if !(0.0..=1.0).contains(&p.coverage) {
                return Err(CoreError::InvalidArgument("coverage"));
            }
            if p.x < 0 || p.y < 0 || p.x >= self.width || p.y >= self.height {
                continue;
            }
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
        }
        if x1 < x0 {
            return Ok(false);
        }
        let brush = self.brush.clone();
        let pressure = brush.pressure_scale(pressure);
        if self.mix_on {
            // 色の混ぜ: ダブの画素を、下地を読む範囲が離れた塊へ分け、塊ごとに枠を凍結して塗る
            return self.apply_mix_dab(surface, &brush, pixels, pressure, points, changed);
        }
        // 面のダブの中心は UV の継ぎ目で飛ぶので、伸ばすの動きの向きは使わない（canvas_shift = false）
        let frame = match self.prepare_dab(
            surface,
            &brush,
            (x0, x1),
            (y0, y1),
            center.x,
            center.y,
            pressure,
            false,
        )? {
            Prepared::Skip => return Ok(false),
            Prepared::Paint => None,
            Prepared::Effect(f) => Some(f),
        };
        let paint = self.paint(&brush, frame.as_ref());
        let result = self.paint_listed_pixels(
            surface,
            &paint,
            pressure,
            pixels,
            points,
            0..pixels.len(),
            changed,
        );
        self.effect.scratch = 0;
        self.frame_cache = frame;
        result
    }

    /// 面のダブの画素のうち `order` の番号のものを、この順に塗る（キャンバスの外は飛ばす）。
    #[allow(clippy::too_many_arguments)]
    fn paint_listed_pixels(
        &mut self,
        surface: &mut Surface,
        paint: &Paint<'_>,
        pressure: PressureScale,
        pixels: &[BrushPixel],
        points: Option<&[StencilPoint]>,
        order: impl Iterator<Item = usize>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let ts = surface.tile_size() as i64;
        let mut cursor = TileCursor::default();
        let mut any = false;
        let mut result = Ok(());
        for i in order {
            let p = &pixels[i];
            if p.x < 0 || p.y < 0 || p.x >= self.width || p.y >= self.height {
                continue;
            }
            let Some((scales, paper)) = self.surface_scales(p.x, p.y) else {
                continue;
            };
            let point = points.map(|v| v[i]);
            let coord = TileCoord::new((p.x / ts) as u32, (p.y / ts) as u32);
            let local = ((p.y % ts) * ts + p.x % ts) as usize;
            if let Err(e) = cursor.move_to(self, surface, coord) {
                result = Err(e);
                break;
            }
            let r = cursor.with(self, surface, paint, |cx, held, live| {
                apply_at::<false>(
                    cx,
                    held,
                    live,
                    coord,
                    local,
                    p.coverage as f32,
                    pressure,
                    scales,
                    paper,
                    point,
                )
            });
            match r {
                Ok(true) => {
                    if changed.last() != Some(&coord) {
                        changed.push(coord);
                    }
                    any = true;
                }
                Ok(false) => {}
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        cursor.release(self, surface);
        result.map(|_| any)
    }

    /// 指先の前の位置を忘れる（3D の面で継ぎ目をまたぐとき。C# の ResetEffectDirection）。
    pub(crate) fn reset_effect_direction(&mut self) {
        self.effect.has_position = false;
    }

    /// 1 枚のタイルの、ストロークの持ち分と面のタイルを取り出して f に渡し、終わったら（失敗しても）戻す。
    /// 面のタイルは、そのタイルで初めて書くときに 1 回だけ自分のものにし（写しと共有なら複製）、後の画素はそのまま書く。
    fn with_tile<F>(
        &mut self,
        surface: &mut Surface,
        paint: &Paint<'_>,
        coord: TileCoord,
        f: F,
    ) -> Result<bool, CoreError>
    where
        F: FnOnce(
            &mut PixelContext<'_>,
            &mut Option<StrokeTile>,
            &mut LiveTile,
        ) -> Result<bool, CoreError>,
    {
        let mut live = LiveTile::take(surface, coord)?;
        let mut held = self.tiles.remove(&coord);
        let ts = surface.tile_size() as usize;
        let mut cx = PixelContext {
            paint,
            budgets: self.budgets,
            rollback_bytes: &mut self.rollback_bytes,
            allocated: &mut surface.allocated,
            tile_size: ts,
            selected: Selected::of(self.selection.as_ref(), coord),
            tally: MixTally::default(),
        };
        let result = f(&mut cx, &mut held, &mut live);
        let tally = cx.tally;
        self.mix_run.tally.merge(tally);
        if let Some(h) = held {
            self.tiles.insert(coord, h);
        }
        live.put_back(surface, coord);
        result
    }
}

/// 画素を 1 つずつ渡すダブ（面のダブ）で、今のタイルを取り出したまま持つ（C# の TileCursor）。タイルが変わったときだけ戻して引き直す。
#[derive(Default)]
struct TileCursor {
    coord: Option<TileCoord>,
    held: Option<StrokeTile>,
    live: Option<LiveTile>,
}

impl TileCursor {
    fn move_to(
        &mut self,
        state: &mut StrokeState,
        surface: &mut Surface,
        coord: TileCoord,
    ) -> Result<(), CoreError> {
        if self.coord == Some(coord) {
            return Ok(());
        }
        self.release(state, surface);
        let live = LiveTile::take(surface, coord)?;
        self.coord = Some(coord);
        self.held = state.tiles.remove(&coord);
        self.live = Some(live);
        Ok(())
    }

    fn with<F>(
        &mut self,
        state: &mut StrokeState,
        surface: &mut Surface,
        paint: &Paint<'_>,
        f: F,
    ) -> Result<bool, CoreError>
    where
        F: FnOnce(
            &mut PixelContext<'_>,
            &mut Option<StrokeTile>,
            &mut LiveTile,
        ) -> Result<bool, CoreError>,
    {
        let ts = surface.tile_size() as usize;
        let coord = self.coord.expect("move_to の後");
        let mut cx = PixelContext {
            paint,
            budgets: state.budgets,
            rollback_bytes: &mut state.rollback_bytes,
            allocated: &mut surface.allocated,
            tile_size: ts,
            selected: Selected::of(state.selection.as_ref(), coord),
            tally: MixTally::default(),
        };
        let result = f(
            &mut cx,
            &mut self.held,
            self.live.as_mut().expect("move_to の後"),
        );
        let tally = cx.tally;
        state.mix_run.tally.merge(tally);
        result
    }

    fn release(&mut self, state: &mut StrokeState, surface: &mut Surface) {
        if let Some(coord) = self.coord.take() {
            if let Some(h) = self.held.take() {
                state.tiles.insert(coord, h);
            }
            if let Some(l) = self.live.take() {
                l.put_back(surface, coord);
            }
        }
    }
}

/// 2 つ目の筆先のダブの形（C# の DualDabAt の式。丸い筆先も回転の式で測る）。
struct DualShape<'a> {
    x: f64,
    y: f64,
    radius: f64,
    cos: f64,
    sin: f64,
    roundness: f64,
    hardness: f64,
    aspect_x: f64,
    aspect_y: f64,
    tip: Option<&'a BrushTip>,
    /// 縁のアンチエイリアス（画素。主の筆先と同じ [`DabShape::with_edge`] の値）。
    edge: Edge,
}

impl DualShape<'_> {
    /// 中心からのずれ (dx, dy) の被覆率（対称の写しは、画素の中心を元へ戻した位置で測る）。
    #[inline(always)]
    fn coverage_at(&self, dx: f64, dy: f64) -> f64 {
        let u = (self.cos * dx + self.sin * dy) / self.radius;
        let v = (-self.sin * dx + self.cos * dy) / (self.radius * self.roundness);
        match self.tip {
            None if !self.edge.is_off() => {
                let dist = (u * u + v * v).sqrt();
                let minor = self.radius * self.roundness;
                let g = edge::ellipse_gradient(u, v, dist, self.radius, minor);
                let mid = 1.0 - (1.0 - self.hardness) * 0.5;
                let floored = edge::box_floor(
                    dist,
                    g,
                    mid,
                    (u * self.radius, v * minor),
                    (self.radius * mid, minor * mid),
                );
                edge::cover64_floored(
                    dist,
                    floored,
                    self.hardness,
                    self.edge.band * g,
                    self.edge.density,
                )
            }
            None => {
                let dist = (u * u + v * v).sqrt();
                if dist > 1.0 {
                    return 0.0;
                }
                let mut c = 1.0;
                if dist > self.hardness {
                    let t = (1.0 - dist) / (1.0 - self.hardness);
                    c = t * t * (3.0 - 2.0 * t);
                }
                c
            }
            Some(t) => {
                let c = t.sample(
                    (u / self.aspect_x + 1.0) * 0.5,
                    (v / self.aspect_y + 1.0) * 0.5,
                );
                if self.edge.density != 1.0 {
                    c * self.edge.density
                } else {
                    c
                }
            }
        }
    }
}

/// prepare_effect_dab の結果。
enum Prepared {
    /// このダブは何も塗らない（指先の最初のダブ、読み元がキャンバスの外）。
    Skip,
    /// 色を塗る（読み元は要らない）。
    Paint,
    Effect(EffectFrame),
}

/// ダブの形（中心・半径・角度・真円率・筆先・硬さ）と、塗りの係数（筆圧・ゆらぎの不透明度と流量・紙の質感・デュアルの合わせ方）。
/// 座標の枠はダブの持ち主のもの（2D は文書の画素、3D の面のダブは y を上向きに直した画面の点）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DabShape<'a> {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub cos: f64,
    pub sin: f64,
    pub roundness: f64,
    pub aspect_x: f64,
    pub aspect_y: f64,
    pub hardness: f64,
    pub pressure: PressureScale,
    pub opacity_scale: f64,
    pub flow_scale: f64,
    pub tip: Option<&'a BrushTip>,
    /// 回転も潰しも無い丸（元の式そのもので測る）。
    pub plain: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub texture: Option<&'a PaperTexture>,
    pub dual: Option<DualBrushMode>,
    /// 縁のアンチエイリアス（帯の幅はダブの座標の単位。[`DabShape::with_edge`]。既定は今の式）。
    pub edge: Edge,
}

/// ダブの覆いを、中心からのずれ（y は上向き）ごとに 1 つずつ測る形（[`DabShape::coverage_fn`]。2D の行の核と同じ f32 の式・同じ値を
/// 前もって作ったもの）。3D の面のダブが、投影の画素ごとに呼ぶ。
pub(crate) struct ShapeCoverage<'a>(rows::Shape32<'a>);

impl ShapeCoverage<'_> {
    /// 中心からのずれ (dx, dy) の覆い（0〜1。外は 0）。2D の画素ごとの覆いの行（`rows::cover_row`）と同じ式の 1 本のレーン。
    #[inline]
    pub(crate) fn coverage(&self, dx: f32, dy: f32) -> f32 {
        rows::cover_one(&self.0, dx, dy)
    }
}

/// デュアルブラシの合わせ（主の覆いとデュアルの溜まり。2D の行の核と同じ f32 の式）。
#[inline]
pub(crate) fn combine_dual(mode: DualBrushMode, main: f32, dual: f32) -> f32 {
    rows::combine32(mode, main, dual)
}

impl<'a> DabShape<'a> {
    /// ダブが届く、中心からの距離（丸は半径、筆先の画像は回した四角の外接円の √2 × 半径）。2D のダブの外接の箱と、3D の面のダブが
    /// 投影の画素を集める画面の円は、この半径。
    pub(crate) fn reach(&self) -> f64 {
        if self.edge.is_off() {
            return if self.tip.is_none() {
                self.radius
            } else {
                self.radius * SQRT_2
            };
        }
        match self.tip {
            // 真円は帯の外の端。潰した丸は、それと、覆いが半分になる楕円の外接の箱を帯の分だけ広げた箱の角までの、近い方（帯の式は
            // 箱の外の距離で押さえるので、その外は 0）
            None if self.plain || self.roundness >= 1.0 => self.radius * self.round_bounds().1,
            None => {
                let mid = 1.0 - (1.0 - self.hardness) * 0.5;
                let grow = (self.radius * (1.0 - self.hardness)).max(self.edge.band) * 0.5 + 1e-3;
                let (eu, ev) = (
                    self.radius * mid + grow,
                    self.radius * self.roundness * mid + grow,
                );
                (self.radius * self.round_bounds().1).min((eu * eu + ev * ev).sqrt())
            }
            // 広げた筆先は、縦が横より長くなりうる
            Some(_) => self.radius * self.roundness.max(1.0) * SQRT_2,
        }
    }

    /// 丸の縁の帯の内の端・外の端（規格化した距離。潰した丸は、帯がいちばん広くなる短い軸の向きの値）。
    pub(crate) fn round_bounds(&self) -> (f64, f64) {
        if self.edge.is_off() {
            return (self.hardness, 1.0);
        }
        let minor = self.radius * self.roundness.min(1.0);
        edge::bounds(self.hardness, self.edge.band / minor)
    }

    /// アンチエイリアスの段 level の縁を付ける（2D。ダブの座標は描く先の画素）。丸は帯の幅と濃さ、画像の筆先は小さなダブの
    /// 広げと濃さだけ（画像の補間は今のまま）。段がなしなら今の式のまま。
    pub(crate) fn with_edge(self, level: AntiAlias) -> Self {
        self.with_edge_in(level, &TexelMetric::IDENTITY)
    }

    /// [`DabShape::with_edge`] の、テクセル 1 つの枠での大きさ metric を渡す形（3D の面のダブ: 枠は画面の点、metric はダブの中心の値）。
    pub(crate) fn with_edge_in(mut self, level: AntiAlias, metric: &TexelMetric) -> Self {
        let w = level.band();
        self.edge = Edge::OFF;
        if w <= 0.0 || !metric.usable() {
            return self;
        }
        let identity = *metric == TexelMetric::IDENTITY;
        let (c, s) = (self.cos, self.sin);
        match self.tip {
            None => {
                let (a, b) = (self.radius, self.radius * self.roundness);
                // 軸の向きのテクセルの半径
                let (ta, tb) = if identity {
                    (a, b)
                } else {
                    (
                        metric.texel_length(c * a, s * a),
                        metric.texel_length(-s * b, c * b),
                    )
                };
                let e = edge::band_and_density(w, self.hardness, ta, tb);
                // 帯の半分より細い軸は帯の半分まで広げ（濃さで面積を保つ）、その形に帯を付ける
                let (fa, fb) = edge::widen_axes(&e, self.hardness, ta, tb);
                if fa != 1.0 || fb != 1.0 {
                    self.radius *= fa;
                    self.roundness = self.roundness * fb / fa;
                }
                // 帯はテクセルの幅。枠の単位へは、2D はそのまま、3D はテクセルがいちばん長く写る向きの長さを掛ける（ダブの中で
                // 1 つの値。どの向きでも帯がテクセル 1 つ分より細くならない側）
                self.edge = if identity {
                    e
                } else {
                    Edge {
                        band: e.band * metric.eigen().0.sqrt(),
                        density: e.density,
                    }
                };
            }
            Some(_) => {
                let hu = self.radius * self.aspect_x;
                let hv = self.radius * self.roundness * self.aspect_y;
                let (tu, tv) = if identity {
                    (hu, hv)
                } else {
                    (
                        metric.texel_length(c * hu, s * hu),
                        metric.texel_length(-s * hv, c * hv),
                    )
                };
                let e = edge::band_and_density(w, 1.0, tu, tv);
                let rho = e.band * 0.5;
                let fu = if tu < rho { rho / tu } else { 1.0 };
                let fv = if tv < rho { rho / tv } else { 1.0 };
                if fu != 1.0 || fv != 1.0 {
                    self.radius *= fu;
                    self.roundness = self.roundness * fv / fu;
                }
                self.edge = Edge {
                    band: 0.0,
                    density: e.density,
                };
            }
        }
        self
    }

    /// 覆いを 1 画素ずつ測る形を作る（丸・筆先の画像・回転・真円率・反転。2D の行の核と同じ値）。
    pub(crate) fn coverage_fn(&self) -> ShapeCoverage<'a> {
        ShapeCoverage(rows::Shape32::new(self))
    }

    /// 筆先の画像の縦横比（長い辺が直径にかかる）。
    fn with_aspect(mut self) -> Self {
        if let Some(t) = self.tip {
            if t.width() >= t.height() {
                self.aspect_y = t.height() as f64 / t.width() as f64;
            } else {
                self.aspect_x = t.width() as f64 / t.height() as f64;
            }
        }
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
enum EffectKind {
    Paint,
    Blur(i64),
    Smudge(f64),
    Clone,
}

/// 画素の処理が読む、ストロークとダブの値（ワーカーからも読む）。
struct Paint<'a> {
    keep_alpha: bool,
    s: &'a BrushSettings,
    effect: EffectKind,
    stroke_color: Rgba8,
    dab_color: Rgba8,
    tip_colors: bool,
    /// 天井に届いた画素は何もしない（ダブごとの色・効果では、天井でも色は寄せる）。
    stop_at_ceiling: bool,
    stencil: Option<&'a BrushStencil>,
    stencil_kind: Option<ChannelKind>,
    frame: Option<&'a EffectFrame>,
    /// 色の混ぜの今の打点の値（読む下地は `frame`）。あれば画素の色はここから決まる。
    mix: Option<MixDab>,
    /// 面のダブ（[`StrokeState::apply_mapped_dab`]）が先に読んで混ぜた参照の色（画素ごと）。あれば `frame` の代わりに読む。
    mapped: Option<&'a sources::MappedColors>,
    offset_x: f64,
    offset_y: f64,
    width: i64,
    height: i64,
    /// 今のダブの読み元の枠のバイト（タイルを初めて触るときの予算の確かめに足す）。
    scratch: u64,
}

/// 画素の処理が使う、ストロークと面の状態。
pub(crate) struct PixelContext<'a> {
    paint: &'a Paint<'a>,
    budgets: Budgets,
    rollback_bytes: &'a mut u64,
    allocated: &'a mut u64,
    tile_size: usize,
    /// このタイルの選択範囲の量。
    selected: Selected<'a>,
    /// 色の混ぜ: このタイルの打点の下地の合計（呼び手が打点のタイルの順に足す）。
    tally: MixTally,
}

/// ストロークの選択範囲の、1 枚のタイルの量（C# の ApplyPixelAt の selected）。
#[derive(Clone, Copy)]
enum Selected<'a> {
    /// 選択範囲が無い（どこでも 1）。
    Everywhere,
    /// 選択範囲はあるが、このタイルには何も選ばれていない（どこでも 0）。
    Nothing,
    Tile(&'a Amounts),
}

impl<'a> Selected<'a> {
    fn of(selection: Option<&'a SelectionMask>, coord: TileCoord) -> Selected<'a> {
        match selection {
            None => Selected::Everywhere,
            Some(s) => s.tile(coord).map_or(Selected::Nothing, Selected::Tile),
        }
    }
    /// タイルの中の画素の番号の量（0〜1。量 / 255 を f32 で。行の核と同じ値）。
    #[inline(always)]
    fn amount(&self, local: usize) -> f32 {
        match self {
            Selected::Everywhere => 1.0,
            Selected::Nothing => 0.0,
            Selected::Tile(t) => rows::unit32(t.get(local)),
        }
    }
}

/// 外接の箱がこれ以上（画素）のダブは、タイルごとにワーカーで描く候補になる（C# と同じ 128²）。ただし画素ごとの時間はブラシで 15 倍ほど
/// 違うので、箱の大きさに画素ごとの時間の見積もり（[`rows::cost_per_pixel`]、ナノ秒）を掛けて、画素ごとの式（30 ナノ秒）で描く箱の
/// 大きさへ換算してから比べる。ワーカーを起こす費用は、1 つのダブの直列の時間がおよそ 0.5 ミリ秒を超えて初めて勝つ
/// （幅 128 の硬い丸の 150 ダブは、直列で 4 ミリ秒、ワーカーを使うと 43 ミリ秒。幅 512 の指先・色の混ぜは、ワーカーで 2.4〜3.4 倍速い。1 つのダブが直列で 0.6〜0.8 ミリ秒を超えるあたりが損得の境）。
pub(crate) const PARALLEL_DAB_PIXELS: i64 = 128 * 128;
static PARALLEL_THRESHOLD: AtomicI64 = AtomicI64::new(PARALLEL_DAB_PIXELS);
/// 画素ごとの式（`apply_at`）の画素ごとの時間の見積もり（ナノ秒）。しきい値の換算の基準。
const SCALAR_PIXEL_NANOS: i64 = 30;

/// 箱が `box_pixels` 画素で、1 画素 `cost` ナノ秒かかるダブを、ワーカーで描く価値があるか。しきい値が既定（128²）未満のときは、
/// 箱の画素数そのものの下限（試験が小さな箱でもワーカーの経路を通すため）。
fn worth_parallel(box_pixels: i64, cost: i64) -> bool {
    worth_parallel_for(PARALLEL_THRESHOLD.load(Ordering::Relaxed), box_pixels, cost)
}

fn worth_parallel_for(limit: i64, box_pixels: i64, cost: i64) -> bool {
    if limit < PARALLEL_DAB_PIXELS {
        box_pixels >= limit
    } else {
        box_pixels.saturating_mul(cost) / SCALAR_PIXEL_NANOS >= limit
    }
}

/// 試験のための口（C# の BrushStroke.ParallelDabPixels と同じ役目）: ワーカーで描くダブの外接の箱の下限（既定の 128² 以上の値は、
/// 画素ごとの式で描く箱の大きさへ換算した下限）を変え、前の値を返す。小さなキャンバスでもワーカーの経路を通すために使う。
/// どの値でも画素の結果は同じ（経路の選び方だけが変わる）。
#[doc(hidden)]
pub fn set_parallel_dab_pixels(pixels: i64) -> i64 {
    PARALLEL_THRESHOLD.swap(pixels, Ordering::Relaxed)
}

/// ダブが触るタイルと、その中のキャンバスの画素の範囲（x の両端、y の両端）。
type TileSpan = (TileCoord, (i64, i64), (i64, i64));

/// 1 枚のタイルの中のダブの画素（C# の DabPixels。xs・ys はキャンバスの画素の範囲、両端を含む）。ステンシルを使わないブラシの多くは
/// 行の核（[`rows`]、どの道でも）で、ほかは画素ごとの式（[`dab_tile_with`]）で描く。
#[allow(clippy::too_many_arguments)]
fn dab_tile(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    if matches!(cx.selected, Selected::Nothing) {
        return Ok(false); // 選択範囲がこのタイルに何も選んでいない: どの画素も `apply_at` の最初で断られる
    }
    if let Some(level) = rows::usable(cx, s) {
        return rows::dab_tile(level, cx, held, live, dual, s, coord, xs, ys);
    }
    let simple =
        cx.paint.effect == EffectKind::Paint && !cx.paint.tip_colors && cx.paint.stencil.is_none();
    if simple {
        dab_tile_with::<true>(cx, held, live, dual, s, coord, xs, ys)
    } else {
        dab_tile_with::<false>(cx, held, live, dual, s, coord, xs, ys)
    }
}

/// 画素ごとの式で 1 枚のタイルのダブの画素を描く。覆い・デュアル・乗算の紙の質感は行の核と同じ式を 1 本のレーンで求め
/// （[`rows::cover_row`] など）、画素は [`apply_at`]。SIMPLE は色を塗るだけ（ダブごとの色・効果・ステンシルなし）。
#[allow(clippy::too_many_arguments)]
fn dab_tile_with<const SIMPLE: bool>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    dual: Option<&[f32]>,
    s: &DabShape<'_>,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    use crate::math::simd::Scalar1;
    let ts = cx.tile_size as i64;
    let (ox, oy) = (coord.x as i64 * ts, coord.y as i64 * ts);
    let shape = rows::Shape32::new(s);
    let (opacity_scale, flow_scale) = (s.opacity_scale as f32, s.flow_scale as f32);
    let n = (xs.1 - xs.0 + 1) as usize;
    let mut cov = vec![0.0f32; n];
    let mut changed = false;
    for py in ys.0..=ys.1 {
        let row = ((py - oy) * ts) as usize;
        let x0 = (xs.0 - ox) as usize;
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        let (lo, hi) = unsafe { rows::cover_row::<Scalar1>(s, &shape, py, xs, &mut cov) };
        if lo >= hi {
            continue;
        }
        if let Some(mode) = s.dual {
            // SAFETY: 同上
            unsafe { rows::dual_row::<Scalar1>(mode, dual, &mut cov, lo, hi, row + x0) };
        }
        // 紙の質感の y の側は行で同じ
        let grain = s.texture.map(|t| (t, rows::Grain::new(t, py)));
        for (i, &coverage) in cov.iter().enumerate().take(hi).skip(lo) {
            if coverage <= 0.0 {
                continue;
            }
            let px = xs.0 + i as i64;
            let mut ceiling_scale = opacity_scale;
            let mut paper = None;
            if let Some((tex, g)) = &grain {
                // 紙の質感は流量ではなく天井に効かせる（Photoshop の「描点ごとに適用」オフと同じ）。流量に効かせると、
                // 間隔の細かいブラシでは重なったダブが溜まって質感が消えてしまう
                let depth = tex.depth as f32;
                match tex.mode.as_blend() {
                    None => {
                        ceiling_scale = rows::texture_scale_one(g, depth, opacity_scale, px);
                        if ceiling_scale <= 0.0 {
                            continue;
                        }
                    }
                    Some(mode) => paper = Some((mode, rows::grain_one(g, px), depth)),
                }
            }
            changed |= apply_at::<SIMPLE>(
                cx,
                held,
                live,
                coord,
                row + x0 + i,
                coverage,
                s.pressure,
                (ceiling_scale, flow_scale),
                paper,
                None,
            )?;
        }
    }
    Ok(changed)
}

/// タイルを初めて触る: 覆い（float × TileSize²）と写し（とダブごとの色）を合わせて予算と比べ（写しは共有でも全体を数える）、
/// ストロークのタイルを作って巻き戻しの数を増やす。ステンシルがあれば、画素ごとに読んだ値（double と色で 12 バイト）も持つ。
/// `simple` は色を塗るだけ（ダブごとの色・ステンシルなし）と分かっているとき。
fn new_stroke_tile(
    cx: &mut PixelContext<'_>,
    live: &mut LiveTile,
    simple: bool,
) -> Result<StrokeTile, CoreError> {
    let _profile = profile::scope(profile::Stage::FirstTouch);
    let p = cx.paint;
    let ts = cx.tile_size;
    let paint_bytes = if !simple && p.tip_colors {
        (ts * ts * 16) as u64
    } else {
        0
    };
    let per_pixel = if !simple && p.stencil.is_some() {
        16
    } else {
        4
    };
    let next =
        *cx.rollback_bytes + 64 + (ts * ts * per_pixel) as u64 + live.byte_size() + paint_bytes;
    if next.saturating_add(p.scratch) > cx.budgets.stroke {
        return Err(CoreError::StrokeBudgetExceeded);
    }
    let stencil = !simple && p.stencil.is_some();
    let (before, before_px) = live.snapshot();
    let tile = StrokeTile {
        wash: vec![0.0; ts * ts],
        before,
        before_px,
        paint: (!simple && p.tip_colors).then(|| vec![0.0; ts * ts * 4]),
        stencil_amount: stencil.then(|| vec![f64::NAN; ts * ts]),
        stencil_color: stencil.then(|| vec![Rgba8::TRANSPARENT; ts * ts]),
    };
    *cx.rollback_bytes = next;
    Ok(tile)
}

/// 1 画素（C# の ApplyPixelAt の 1 チャンネルの経路。選択範囲の量と、透明部分のロックの `keep_alpha`（アルファと透明画素の RGB を
/// 守る）はここだけで掛ける）。計算は行の核（[`rows`]）と同じ f32 の式を 1 本のレーンで通るので、行の核が描くブラシでは同じバイトになる。
/// SIMPLE は色を塗るだけ（ダブごとの色・効果なし）と分かっているとき（分岐を除いた同じ式）。`scales` は不透明度と流量の係数、
/// paper は乗算以外の紙の質感（拡張）: 合わせ方・質感の値・深さ。
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn apply_at<const SIMPLE: bool>(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    coord: TileCoord,
    local: usize,
    coverage: f32,
    pressure: PressureScale,
    (opacity_scale, flow_scale): (f32, f32),
    paper: Option<(DualBrushMode, f32, f32)>,
    at: Option<StencilPoint>,
) -> Result<bool, CoreError> {
    let p = cx.paint;
    let s = p.s;
    let ts = cx.tile_size;
    // 選択範囲: 選ばれていない画素は何もしない（写しも取らない）。半分選ばれた画素は、描く前の画素から半分までしか変わらない
    let selected = cx.selected.amount(local);
    if selected <= 0.0 || p.keep_alpha && live.get(local * 4).a == 0 {
        return Ok(false);
    }
    let mut opacity_scale = opacity_scale;
    let mut through = StencilSample::default();
    let mut stencil_read = false;
    if !SIMPLE {
        if let Some(stencil) = p.stencil {
            // ステンシルも紙の質感と同じく天井に効かせる（重なったダブでステンシルの量を越えない）。画素ごとに初めの 1 回だけ読む
            let known = held
                .as_ref()
                .and_then(|t| t.stencil_amount.as_ref().map(|a| a[local]))
                .filter(|a| !a.is_nan());
            match known {
                Some(amount) => {
                    let c = held
                        .as_ref()
                        .expect("読んだタイル")
                        .stencil_color
                        .as_ref()
                        .expect("色")[local];
                    through = StencilSample {
                        amount,
                        color: c,
                        has_color: stencil.mode() == StencilMode::Color,
                    };
                }
                None => {
                    through = match at {
                        Some(point) => stencil.sample_at(point),
                        None => {
                            let px = coord.x as i64 * ts as i64 + (local % ts) as i64;
                            let py = coord.y as i64 * ts as i64 + (local / ts) as i64;
                            stencil.sample_canvas(px, py).ok_or(CoreError::Unsupported(
                                "ステンシルにキャンバスからの写しが無い（画素ごとにステンシルの上の点を渡す）",
                            ))?
                        }
                    };
                    stencil_read = true;
                    if let Some(t) = held.as_mut() {
                        if let (Some(a), Some(c)) =
                            (t.stencil_amount.as_mut(), t.stencil_color.as_mut())
                        {
                            a[local] = through.amount;
                            c[local] = through.color;
                        }
                    }
                }
            }
            if through.amount <= 0.0 {
                return Ok(false);
            }
            opacity_scale *= through.amount as f32;
        }
    }
    // 色の混ぜ: 打点の前に凍結した下地の色を読み、荷と混ぜた色をこの画素の色にする。下地の合計（荷の更新の元）はここで足す
    let mut mixed = None;
    if !SIMPLE {
        if let Some(mix) = p.mix {
            let px = coord.x as i64 * ts as i64 + (local % ts) as i64;
            let py = coord.y as i64 * ts as i64 + (local / ts) as i64;
            let ground = match p.mapped {
                // 面のダブの伸ばし: 参照は書く前にまとめて読んである（指先と同じ）
                Some(mapped) => mapped.get(p.width, px, py),
                None => mix.ground_at(p.frame.expect("混ぜの下地"), px, py, p.width, p.height),
            };
            rows::tally32(&mut cx.tally, coverage, ground);
            match rows::mixed32(&mix, ground) {
                Some(color) => mixed = Some(color),
                None => return Ok(false),
            }
        }
    }
    // SIMPLE（色を塗るだけの丸いブラシ）は、混ぜがあれば画素ごとの色を持つので選ばれない（混ぜ・指先の分岐を持ち込まない）
    let (density, smudge) = if SIMPLE {
        (None, None)
    } else {
        (
            p.mix.map(|m| m.density),
            match p.effect {
                EffectKind::Smudge(strength) => Some(strength),
                _ => None,
            },
        )
    };
    let (mut ceiling, flow) = rows::ceiling_and_flow32(
        p,
        pressure,
        (opacity_scale, flow_scale),
        coverage,
        density,
        smudge,
    );
    if let Some((mode, grain, depth)) = paper {
        ceiling += (rows::combine32(mode, ceiling, grain) - ceiling) * depth;
    }
    if flow <= 0.0 || ceiling <= 0.0 {
        return Ok(false);
    }
    if let Some(st) = held.as_ref() {
        if st.wash[local] >= ceiling && (SIMPLE || p.stop_at_ceiling) {
            return Ok(false);
        }
    }
    if held.is_none() {
        let mut tile = new_stroke_tile(cx, live, SIMPLE)?;
        if stencil_read {
            if let (Some(a), Some(c)) = (tile.stencil_amount.as_mut(), tile.stencil_color.as_mut())
            {
                a[local] = through.amount;
                c[local] = through.color;
            }
        }
        *held = Some(tile);
    }
    let st = held.as_mut().expect("直前に作った");
    let previous = st.wash[local];
    // 天井に届いた画素も、ダブごとの色ならその色へは寄せる（濃さは天井のまま）
    let accumulated = if previous >= ceiling {
        previous
    } else {
        rows::accumulate32(previous, ceiling, flow)
    };
    st.wash[local] = accumulated;
    let mut color = p.stroke_color;
    if !SIMPLE && p.tip_colors {
        // 画素ごとの色は、チャンネルごとの面（R の面・G の面・B の面・A の面の順、1 面は TileSize²）に持つ
        let pc = st.paint.as_mut().expect("ダブごとの色");
        let plane = ts * ts;
        let w = rows::min32(1.0, flow);
        let d = mixed.unwrap_or(p.dab_color).to_array();
        let fresh = previous <= 0.0;
        let mut v = [0u8; 4];
        for (c, out) in v.iter_mut().enumerate() {
            let at = c * plane + local;
            pc[at] = rows::plane_step32(pc[at], rows::unit32(d[c]), w, fresh);
            *out = rows::byte32(pc[at]);
        }
        color = Rgba8::new(v[0], v[1], v[2], v[3]);
    }
    if !SIMPLE {
        // ステンシルの色（色のモード）: その画素のステンシルの色を、塗りつぶしの画像と同じ読み方でこのチャンネルの値にする
        // （アルファは描画色のもの）
        if let (Some(kind), Some(stencil)) = (p.stencil_kind, p.stencil) {
            if through.has_color {
                color = stencil.paint_for(kind, through.color, p.stroke_color.a);
            }
        }
    }
    let start = st
        .before_px
        .as_ref()
        .map_or(Rgba8::TRANSPARENT, |t| t.get(local * 4));
    let effect = if SIMPLE { EffectKind::Paint } else { p.effect };
    let amount = rows::min32(1.0, accumulated);
    let next = match effect {
        EffectKind::Paint => {
            let next = if s.erase {
                rows::erase32(start, accumulated, s.color.a)
            } else if p.keep_alpha {
                rows::keeping32(start, color, amount * selected)
            } else {
                rows::normal32(start, color, amount)
            };
            // 選択範囲の量だけ、描く前の画素から寄せる（塗りも消しゴムも。C# の Fade(start, next, selected)）
            if selected < 1.0 && !p.keep_alpha {
                rows::fade32(start, next, selected)
            } else {
                next
            }
        }
        effect => {
            let px = coord.x as i64 * ts as i64 + (local % ts) as i64;
            let py = coord.y as i64 * ts as i64 + (local / ts) as i64;
            let sampled = match (effect, p.mapped) {
                // 面のダブ: 参照は書く前にまとめて読んで混ぜてある（キャンバスの外の判定も、そのとき済んでいる）
                (_, Some(mapped)) => mapped.get(p.width, px, py),
                (EffectKind::Blur(radius), None) => rows::blur32(
                    p.frame.expect("効果の読み元"),
                    px,
                    py,
                    radius,
                    p.width,
                    p.height,
                ),
                _ => {
                    let frame = p.frame.expect("効果の読み元");
                    let sx = px as f64 + p.offset_x;
                    let sy = py as f64 + p.offset_y;
                    if sx < 0.0
                        || sy < 0.0
                        || sx > (p.width - 1) as f64
                        || sy > (p.height - 1) as f64
                    {
                        return Ok(false);
                    }
                    rows::sample32(frame, sx, sy, p.width, p.height)
                }
            };
            // ぼかしは透明部分に色を広げない。指先とクローンは透明な場所へ描ける
            if matches!(effect, EffectKind::Blur(_)) && start.a == 0 {
                return Ok(false);
            }
            // 選択範囲の量も寄せ方に入れる（クローンは塗りと同じく Fade で）
            let amount = amount * selected;
            let mut next = if p.keep_alpha {
                if effect == EffectKind::Clone {
                    rows::keeping32(start, sampled, amount)
                } else {
                    rows::keeping32(
                        start,
                        Rgba8::new(sampled.r, sampled.g, sampled.b, 255),
                        amount * f32::from(sampled.a) / 255.0,
                    )
                }
            } else if effect == EffectKind::Clone {
                rows::fade32(
                    start,
                    rows::normal32(start, sampled, rows::min32(1.0, accumulated)),
                    selected,
                )
            } else {
                rows::mix_effect32(start, sampled, amount)
            };
            if next.a == 0 {
                next = Rgba8::new(start.r, start.g, start.b, 0);
            }
            next
        }
    };
    live.write(
        local * 4,
        next,
        cx.allocated,
        ts * ts * 4,
        cx.budgets.growth,
    )
}

#[cfg(test)]
mod tests {
    use super::{cos_sin, lerp_angle, worth_parallel_for, PARALLEL_DAB_PIXELS, SCALAR_PIXEL_NANOS};
    use std::f64::consts::PI;

    #[test]
    fn the_worker_threshold_follows_the_pixel_cost() {
        let limit = PARALLEL_DAB_PIXELS;
        // 画素ごとの式（30 ナノ秒）の箱は、今までと同じ 128² から
        assert!(!worth_parallel_for(limit, limit - 1, SCALAR_PIXEL_NANOS));
        assert!(worth_parallel_for(limit, limit, SCALAR_PIXEL_NANOS));
        // 速いブラシは、同じ時間になる大きな箱から（硬い丸は 30 倍）
        assert!(!worth_parallel_for(limit, limit * 30 - 1, 1));
        assert!(worth_parallel_for(limit, limit * 30, 1));
        assert!(!worth_parallel_for(limit, 100_000, 3));
        assert!(worth_parallel_for(limit, 200_000, 3));
        assert!(worth_parallel_for(limit, 40_000, 14));
        // 既定より小さい値は箱の画素数そのものの下限（試験が小さな箱でもワーカーの経路を通す）。0 は常に、最大は決して
        assert!(worth_parallel_for(1, 1, 1));
        assert!(worth_parallel_for(0, 0, 1));
        assert!(!worth_parallel_for(64, 63, 30));
        assert!(worth_parallel_for(64, 64, 1));
        assert!(!worth_parallel_for(i64::MAX, i64::MAX, 30));
    }

    #[test]
    fn cos_sin_keeps_the_libm_bits() {
        for a in [0.0f64, -0.0, 0.3, -1.2, PI, 7.0, f64::MIN_POSITIVE] {
            let (c, s) = cos_sin(a);
            assert_eq!(c.to_bits(), a.cos().to_bits(), "{a}");
            assert_eq!(s.to_bits(), a.sin().to_bits(), "{a}");
        }
    }

    #[test]
    fn rotation_is_interpolated_the_short_way() {
        let (a, b) = (170f64.to_radians(), (-170f64).to_radians());
        let mid = lerp_angle(a, b, 0.5);
        assert!((mid - PI).abs() < 1e-12, "{mid}");
        assert_eq!(lerp_angle(0.3, 0.3, 0.7), 0.3);
        assert!((lerp_angle(0.0, 1.0, 0.25) - 0.25).abs() < 1e-15);
        assert!(
            (lerp_angle(-3.0, 3.0, 0.5).cos() + 1.0).abs() < 0.01,
            "±3 の間は π の側"
        );
    }
}
