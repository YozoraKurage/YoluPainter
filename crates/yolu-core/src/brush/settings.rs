//! 全部入りのブラシの設定（C# の BrushSettings の全体）。M1 の [`BrushSettings`]（大きさ・硬さ・間隔・不透明度・流量・色・筆圧・
//! 消しゴム）に、筆先の形・ゆらぎ・紙の質感・デュアルブラシ・色の変化・フェードと傾き・手ぶれ補正と入り抜き・曲線・効果を足したもの。
//! 部分の分け方は Photoshop のブラシのウィンドウの節に合わせた（画面の節にそのまま当てられる）。
//!
//! 既定値はどれも C# の既定値と同じで、`Brush::simple(settings, switches)`（`Brush::from(settings)` は既定の切り替え）は M1 の丸いブラシと
//! バイト単位で同じストロークになる。
//! C# に無いもの（拡張）: 筆先の反転（[`TipShape::flip_x`]・[`TipShape::flip_y`]。なし・反転・ランダム・折り返し時に反転の 1 つの列挙 [`FlipRule`]）、筆先の繰り返し方法の折り返し・
//! 繰り返さない・一巡のみ・ランダムに一巡（[`TipSelection`]）、ダブの重なりの比較暗（[`TipShape::darken_overlap`]）、紙の質感のモード（[`TextureMode`] の
//! 乗算以外）と回転（[`PaperTexture::angle`]）。どれも既定では C# と同じ。

use crate::Reason;
use std::sync::Arc;

use glam::DVec2;

use super::edge::AntiAlias;
use super::influence::{Influences, Item, PressureSwitches};
use super::mix::ColorMix;
use super::spray::Spray;
use super::stencil::BrushStencil;
use super::tip::BrushTip;
use super::BrushSettings;
use crate::error::CoreError;
use crate::geometry::ModelSymmetry;
use crate::math::require_finite;
use crate::symmetry::CanvasSymmetry;
use crate::types::Rgba8;

/// 全部入りのブラシ。ストロークを始めたときに写して固定する（筆先の画像は共有する。画像は変わらない）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Brush {
    /// 大きさ・硬さ・間隔・不透明度・流量・色・消しゴム（M1 の設定）。
    pub base: BrushSettings,
    /// ゆらぎ・色の変化・デュアルブラシの乱数の種（同じ種なら同じストローク）。C# の Seed。
    pub seed: i32,
    pub tip: TipShape,
    pub jitter: Jitter,
    /// 紙の質感（None は無し）。
    pub texture: Option<PaperTexture>,
    /// デュアルブラシ（None は無し）。
    pub dual: Option<DualBrush>,
    pub color: ColorDynamics,
    pub controls: Controls,
    /// 影響元（項目ごとの筆圧・傾き・速度・ランダムと、その最小値と曲線。[`Influences`]）。項目を入力で変えるかの唯一の正本。
    /// 既定は筆圧で大きさと不透明度を変える（応えは直線。C# の既定と同じ）。
    pub influences: Influences,
    /// 色の混ぜ（厚塗り。拡張: C# に無い。既定は混ぜない）。色のチャンネルを色で塗るブラシだけに効く（[`ColorMix`]）。
    pub mix: ColorMix,
    pub assist: StrokeAssist,
    pub effect: BrushEffect,
    /// ステンシル（None は無し）。共有する（写さない）。
    pub stencil: Option<Arc<BrushStencil>>,
    /// 2D の対称（既定は無し）。指先・クローンとは組めない（写しごとに読み元と動きが要る）。
    pub symmetry: CanvasSymmetry,
    /// 3D の対称（モデルの空間のミラー・放射状）を、2D のキャンバスのストロークのダブにも当てる（既定は無し。共有する）。2D の対称と
    /// 両方あれば、3D の写しの後に 2D の写しを当てる。指先・クローンとは組めない。3D の面のストロークは見ない（面のストロークは
    /// 自分の対称を持つ）。
    pub model_symmetry: Option<Arc<ModelSymmetry>>,
    /// 散布効果（None は無し）。入っている間、描点のダブは [`Jitter::scatter`]・[`Jitter::count`] の代わりに粒子の道を通る（[`Spray`]）。
    pub spray: Option<Spray>,
    /// ブラシと一緒に覚える、ツールプロパティに出す行の名前（ブラシの詳細の行の名前。何が行かは画面の側が決め、描き方には関わらないので、
    /// core は文字列を見ない）。None は、そのブラシの既定の行（画面の側が決める。前の形のブラシはこれ）。
    pub shown_rows: Option<Vec<String>>,
}

/// 表示倍率（画面の点 ÷ 文書の画素）の下限と上限の外の値は、そのまま割ると半径が範囲を外れるので、ここへ収める。
const MIN_VIEW_SCALE: f64 = 1.0 / 65536.0;
const MAX_VIEW_SCALE: f64 = 65536.0;

impl Brush {
    /// ストロークを始める表示倍率 `scale`（文書の画素 1 が画面で何点か。2D のキャンバスは表示の倍率、3D の面は始めの点の下の
    /// 画素 1 つの大きさ）での、そのストロークのブラシ。[`Controls::size_by_screen`] が切なら、`self` と同じ。入りなら、ブラシの大きさ（半径）を
    /// 画面の点の値として、文書の画素へ直した半径（`半径 ÷ scale`。0 より大きく 65536 まで）にする。ストロークの始めに 1 回だけ呼び、その後は
    /// 渡した値を変えない（途中の表示の拡大・縮小は効かない）。表示倍率の受け渡しはこの 1 か所（補正の「表示倍率による調整」もここの値を使う）。
    /// 有限でない・0 以下の倍率は 1 とみなす。
    pub fn at_view_scale(&self, scale: f64) -> Brush {
        let mut brush = self.clone();
        if self.controls.size_by_screen {
            let scale = if scale.is_finite() && scale > 0.0 {
                scale.clamp(MIN_VIEW_SCALE, MAX_VIEW_SCALE)
            } else {
                1.0
            };
            brush.base.radius = (self.base.radius / scale).clamp(MIN_VIEW_RADIUS, 65536.0);
        }
        brush
    }
}

/// 画面上のサイズで指定の半径の下限（文書の画素。ブラシの設定の範囲の「0 より大きい」を満たす小ささ）。
const MIN_VIEW_RADIUS: f64 = 1e-6;

impl From<BrushSettings> for Brush {
    /// 既定の筆圧の切り替え（大きさと不透明度。[`PressureSwitches::default`]）の簡単なブラシ。
    fn from(base: BrushSettings) -> Brush {
        Brush::simple(base, PressureSwitches::default())
    }
}

impl Brush {
    /// 簡単な丸いブラシ（設定と筆圧の切り替え）から。切り替えは影響元の筆圧（応えは直線）になり、ほかは既定。
    pub fn simple(base: BrushSettings, pressure: PressureSwitches) -> Brush {
        Brush {
            base,
            influences: Influences::from_switches(pressure),
            ..Brush::default()
        }
    }
}

/// 複数の筆先から各ダブの筆先を選ぶ方法（C# の TipSelection に、CLIP STUDIO PAINT の「繰り返し方法」の 6 種を加えた）。
/// 並びの何番目を使うかは、ストロークの最初のダブから数えた順（`k`）で決まる（筆先が 1 枚だけ・`image` に置いたときは選ばない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TipSelection {
    /// 乱数で選ぶ（ストロークの乱数の列から 1 つ引く）。
    #[default]
    Random,
    /// 順に使い、最後の次は最初へ戻る（0, 1, 2, 0, 1, 2 …）。
    Sequential,
    /// 順に使い、端で折り返す。端の筆先は重ねて使わない（0, 1, 2, 1, 0, 1, 2 …）。
    PingPong,
    /// 順に使い、最後の筆先に着いたら、その筆先のまま続ける（0, 1, 2, 2, 2 …）。
    Hold,
    /// 順に 1 回だけ使い、最後の筆先のあとは、そのストロークでは何も置かない（0, 1, 2, 置かない …）。
    Once,
    /// 並びをストロークの種から決めた順に並べ替えて 1 回だけ使い、そのあとは何も置かない。
    Shuffle,
}

/// 筆先の画像の反転のしかた（1 つの軸ごと。CLIP STUDIO PAINT の「左右反転」「上下反転」の 4 つの選びに当たる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlipRule {
    /// 反転しない。
    #[default]
    Off,
    /// いつも反転する。
    On,
    /// ダブごとに反転するかを乱数で決める。引く乱数は、ほかの乱数の列（ゆらぎ・色・筆先の選び・影響元・デュアル）と別の列。
    Random,
    /// 折り返しで戻る向きのダブだけ反転する（[`TipSelection::PingPong`] のときだけ起こる）。
    OnFold,
}

impl FlipRule {
    /// すべての反転のしかた（並びは画面の選びの順）。
    pub const ALL: [FlipRule; 4] = [
        FlipRule::Off,
        FlipRule::On,
        FlipRule::Random,
        FlipRule::OnFold,
    ];
}

/// 筆先の形（Photoshop の「ブラシ先端のシェイプ」）。
#[derive(Clone, Debug, PartialEq)]
pub struct TipShape {
    /// 筆先の画像（None は丸。[`BrushSettings::hardness`] で縁が決まる）。画像の長い辺が直径にかかる。
    pub image: Option<Arc<BrushTip>>,
    /// 順に使う複数の筆先（GIMP の画像ホース・CLIP STUDIO の筆先の並び。1〜256 枚）。空でなければ `image` に代わる。
    /// 1 枚でも乱数で選ぶときは乱数を 1 つ引く（C# と同じ。`image` に置いたときとは乱数の列が違う）。
    pub images: Vec<Arc<BrushTip>>,
    pub selection: TipSelection,
    /// 回転（度、反時計回り。キャンバスの Y は上向き）。
    pub angle: f64,
    /// 真円率: 回した縦の軸の向きに潰す（1 = 潰さない、0.01〜1）。
    pub roundness: f64,
    /// 線の向きを角度に足す（毛の筆先・カリグラフィー）。
    pub follow_direction: bool,
    /// 拡張（C# に無い）: 筆先の画像を左右に反転するしかた（回す前の筆先の横の軸で）。丸い筆先には効かない。
    pub flip_x: FlipRule,
    /// 拡張（C# に無い）: 筆先の画像を上下に反転するしかた。
    pub flip_y: FlipRule,
    /// 拡張: 1 本のストロークの中で、ダブが重なる所は足していかずに、覆いの大きい方を残す（CLIP STUDIO PAINT の「ブラシ先端同士を比較暗で合成」）。
    pub darken_overlap: bool,
}

impl Default for TipShape {
    fn default() -> Self {
        TipShape {
            image: None,
            images: Vec::new(),
            selection: TipSelection::Random,
            angle: 0.0,
            roundness: 1.0,
            follow_direction: false,
            flip_x: FlipRule::Off,
            flip_y: FlipRule::Off,
            darken_overlap: false,
        }
    }
}

/// ダブごとのゆらぎと散布（Photoshop の「シェイプ」「散布」。0 は無し）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jitter {
    /// 大きさを最大でこの割合だけ小さくする（0〜1）。
    pub size: f64,
    /// 角度を最大 ±180° × これだけ回す（0〜1）。
    pub angle: f64,
    /// 真円率を最大でこの割合だけ潰す（0〜1。下限 0.01）。
    pub roundness: f64,
    /// 不透明度・流量を最大でこの割合だけ下げる（0〜1）。
    pub opacity: f64,
    pub flow: f64,
    /// 位置を、両方の軸に沿って最大でブラシの直径 × これだけずらす（0〜10）。
    pub scatter: f64,
    /// 1 つの間隔に置くダブの数（1〜16。散布と使う）。
    pub count: u32,
}

impl Default for Jitter {
    fn default() -> Self {
        Jitter {
            size: 0.0,
            angle: 0.0,
            roundness: 0.0,
            opacity: 0.0,
            flow: 0.0,
            scatter: 0.0,
            count: 1,
        }
    }
}

/// 紙の質感の合わせ方。C# は乗算だけ（Photoshop の ABR の他のモードは読み込みで警告して乗算にする）。
/// 乗算以外は拡張（C# に無い）: 画素の天井 c（不透明度 × 筆圧 × ゆらぎ × フェード）と質感の値 g（0〜1）を、デュアルブラシと同じ
/// 式（[`DualBrushMode::combine`]、Photoshop のテクスチャのモードと同じ名前の合わせ方）で合わせ、深さ d で元の天井との間を取る:
/// c + (合わせた値 − c) × d。乗算はこの式と数学的に同じだが、C# と同じ演算の順（倍率 1 − d·(1 − g) を先に掛ける）で計算する。
/// 乗算・比較（暗）・焼き込みの類は白い所ほど塗れ、減算は黒い所ほど塗れる（Photoshop と同じく、向きは画像の反転で選ぶ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureMode {
    /// 天井 × (1 − 深さ × (1 − 質感))（C# と同じ式・同じ演算の順）。
    #[default]
    Multiply,
    Subtract,
    Darken,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
}

/// 紙の質感（Photoshop の「テクスチャ」。描点ごとに適用はしない: 質感はキャンバスに固定され、天井に効く）。
#[derive(Clone, Debug, PartialEq)]
pub struct PaperTexture {
    /// 並べて使う質感の画像（覆い 0〜255、白が塗れる）。
    pub image: Arc<BrushTip>,
    /// 0 = 効かない、1 = 質感にそのまま従う。0 なら質感を読まない。
    pub depth: f64,
    /// 質感の 1 画素あたりのキャンバスの画素（2 = 2 倍に大きく。0.05〜64）。
    pub scale: f64,
    pub mode: TextureMode,
    /// 質感の回転（度、反時計回り。キャンバスの Y は上向き。0 = 回さない）。回した向きでも継ぎ目なく並ぶよう、画像は回さず、読む位置を回す。
    pub angle: f64,
}

impl TextureMode {
    /// すべてのモード（並びは保存の値の順）。
    pub const ALL: [TextureMode; 8] = [
        TextureMode::Multiply,
        TextureMode::Subtract,
        TextureMode::Darken,
        TextureMode::Overlay,
        TextureMode::ColorDodge,
        TextureMode::ColorBurn,
        TextureMode::LinearBurn,
        TextureMode::HardMix,
    ];
    /// 乗算以外の合わせ方（乗算は None。C# と同じ倍率の式で計算する）。
    pub(crate) fn as_blend(self) -> Option<DualBrushMode> {
        match self {
            TextureMode::Multiply => None,
            TextureMode::Subtract => Some(DualBrushMode::Subtract),
            TextureMode::Darken => Some(DualBrushMode::Darken),
            TextureMode::Overlay => Some(DualBrushMode::Overlay),
            TextureMode::ColorDodge => Some(DualBrushMode::ColorDodge),
            TextureMode::ColorBurn => Some(DualBrushMode::ColorBurn),
            TextureMode::LinearBurn => Some(DualBrushMode::LinearBurn),
            TextureMode::HardMix => Some(DualBrushMode::HardMix),
        }
    }
}

impl PaperTexture {
    /// キャンバスの座標 (x, y)（画素の中心は i + 0.5）の質感（0〜1）。角度の分だけ回して、並べた画像を読む（f64。行の核の 1 画素ずつの
    /// 式とは丸めが違うので、対称の写しの画素ごとの式だけが使う）。
    pub(crate) fn grain_at(&self, x: f64, y: f64) -> f64 {
        let Some((cos, sin)) = self.turn() else {
            return self.image.sample_tiled(x / self.scale, y / self.scale);
        };
        self.image.sample_tiled(
            (cos * x + sin * y) / self.scale,
            (cos * y - sin * x) / self.scale,
        )
    }

    /// 回した読みの (cos, sin)。0 度は回さない（None）。大きな有限の角度（`f64::MAX` など）はラジアンへの掛け算で溢れるので、
    /// 360 度で割った余り（符号は保つ。360 度未満の角度は値が変わらない）にしてから変換する。
    pub(crate) fn turn(&self) -> Option<(f64, f64)> {
        (self.angle != 0.0).then(|| {
            let (sin, cos) = ((self.angle % 360.0) * std::f64::consts::PI / 180.0).sin_cos();
            (cos, sin)
        })
    }

    /// 深さ・倍率・角度の範囲を確かめる（主の質感と 2 つ目の質感で同じ）。
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.depth, Reason::TextureDepth)?;
        require_finite(self.scale, Reason::TextureScale)?;
        require_finite(self.angle, Reason::TextureAngle)?;
        if !(0.0..=1.0).contains(&self.depth) {
            return Err(CoreError::InvalidArgument(Reason::TextureDepthRange));
        }
        if !(0.05..=64.0).contains(&self.scale) {
            return Err(CoreError::InvalidArgument(Reason::TextureScaleRange));
        }
        Ok(())
    }

    /// 乗算・大きさ 1 の質感。
    pub fn new(image: Arc<BrushTip>, depth: f64) -> PaperTexture {
        PaperTexture {
            image,
            depth,
            scale: 1.0,
            mode: TextureMode::Multiply,
            angle: 0.0,
        }
    }
}

/// デュアルブラシの合わせ方（C# の DualBrushMode。Photoshop のデュアルブラシのモードの名前と並び）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DualBrushMode {
    #[default]
    Multiply,
    Darken,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
    Subtract,
    /// 通常: 2 つ目の溜まりを使わず、主の覆いのまま。
    Normal,
    /// 加算(発光): min(1, 主 + 2 つ目)。
    Add,
    /// 比較(明): max(主, 2 つ目)。
    Lighten,
    /// スクリーン: 1 − (1 − 主)(1 − 2 つ目)。
    Screen,
    /// 高さ(リニア): 主 − 2 つ目 × (1 − 主)（減算に近く、減算より濃い。[`DualBrushMode::combine`]）。
    LinearHeight,
}

impl DualBrushMode {
    /// [`DualBrushMode::ALL`] の後に足したモード（CLIP STUDIO PAINT のデュアルブラシにあって Photoshop に無い 5 つ）。
    pub const EXTENDED: [DualBrushMode; 5] = [
        DualBrushMode::Normal,
        DualBrushMode::Add,
        DualBrushMode::Lighten,
        DualBrushMode::Screen,
        DualBrushMode::LinearHeight,
    ];

    /// すべてのモード（[`DualBrushMode::ALL`] の 8 つの後に [`DualBrushMode::EXTENDED`] の 5 つ）。
    pub const EVERY: [DualBrushMode; 13] = [
        DualBrushMode::Multiply,
        DualBrushMode::Darken,
        DualBrushMode::Overlay,
        DualBrushMode::ColorDodge,
        DualBrushMode::ColorBurn,
        DualBrushMode::LinearBurn,
        DualBrushMode::HardMix,
        DualBrushMode::Subtract,
        DualBrushMode::Normal,
        DualBrushMode::Add,
        DualBrushMode::Lighten,
        DualBrushMode::Screen,
        DualBrushMode::LinearHeight,
    ];

    /// [`DualBrushMode::EXTENDED`] のモードか（Photoshop の 8 つに無い）。
    pub fn is_extended(self) -> bool {
        DualBrushMode::EXTENDED.contains(&self)
    }

    /// 保存の値の順（C# の値と同じ）。
    pub const ALL: [DualBrushMode; 8] = [
        DualBrushMode::Multiply,
        DualBrushMode::Darken,
        DualBrushMode::Overlay,
        DualBrushMode::ColorDodge,
        DualBrushMode::ColorBurn,
        DualBrushMode::LinearBurn,
        DualBrushMode::HardMix,
        DualBrushMode::Subtract,
    ];
}

/// デュアルブラシ（2 つ目の筆先、C# の DualBrush）。主の筆先と同じ道筋に自分の間隔・散布・数でダブを置き、ストロークの間
/// 画素ごとの最大の被覆率を溜める。主のダブは、そのダブの位置（線の長さ）までに置かれた 2 つ目のダブの溜まりと `mode` で
/// 合わせた被覆率で塗る。大きさは、影響元（[`super::DualItem`]）とサイズの連動（`link_size`）を使わなければ、筆圧・ゆらぎ・入り抜きの
/// 影響を受けない（Photoshop と同じ）。
#[derive(Clone, Debug, PartialEq)]
pub struct DualBrush {
    /// 筆先の画像（None は丸。`hardness` で縁が決まる）。
    pub tip: Option<Arc<BrushTip>>,
    /// 半径（画素、0.5〜65536）。
    pub radius: f64,
    pub hardness: f64,
    /// 間隔（2 つ目の筆先の直径に対する割合、0.01〜4）。
    pub spacing: f64,
    /// 回転（度、反時計回り）。
    pub angle: f64,
    pub roundness: f64,
    /// 散布（2 つ目の筆先の直径の倍数、0〜10）。
    pub scatter: f64,
    /// 1 つの間隔に置く数（1〜16）。
    pub count: u32,
    pub mode: DualBrushMode,
    /// メインのブラシサイズに連動: 2 つ目の半径に、その描点の主のブラシの半径 ÷ 設定の主の半径を掛ける。
    pub link_size: bool,
    /// 濃度（0〜1）: 2 つ目のダブが溜まりへ置く覆いに掛ける。影響元は [`super::DualItem::Flow`]。
    pub flow: f64,
    /// 2 つ目だけのアンチエイリアス（None は主の [`BrushSettings::anti_alias`] と同じ）。
    pub anti_alias: Option<AntiAlias>,
    /// 向きに、ペンが倒れた向き・ペン軸の回転・線の方向を足すか（主の `controls.tilt_angle`・`controls.rotation_angle`・
    /// `tip.follow_direction` と同じ規則）。
    pub angle_tilt: bool,
    pub angle_rotation: bool,
    pub angle_line: bool,
    /// 向きのランダムの強さ（0〜1。1 で ±180°。0 は無し。主の `jitter.angle` と同じ規則）。
    pub angle_random: f64,
    /// 順に使う複数の筆先（1〜256 枚）。空でなければ `tip` に代わる（主の [`TipShape::images`] と同じ意味）。
    pub tips: Vec<Arc<BrushTip>>,
    /// 複数の筆先から 2 つ目のダブごとに選ぶ方法（主の [`TipShape::selection`] と同じ規則・同じ関数）。
    pub selection: TipSelection,
    /// 筆先の画像の左右・上下の反転のしかた（主の [`TipShape::flip_x`]・[`TipShape::flip_y`] と同じ意味）。
    pub flip_x: FlipRule,
    pub flip_y: FlipRule,
    /// 先端同士を比較暗で合成。2 つ目の溜まりはいつも画素ごとに覆いの大きい方を残すので、この旗で描き方は変わらない（取り込みと
    /// 保存のために持つ）。
    pub darken_overlap: bool,
    /// 2 つ目の紙の質感（None は無し）。主の天井と同じ式（[`TextureMode`]）を、2 つ目のダブが溜まりへ置く覆いに掛ける。
    pub texture: Option<PaperTexture>,
    /// 2 つ目の散布効果（None は無し）。入っている間、2 つ目のダブは `scatter`・`count` の代わりに粒子の道を通る（主の [`Spray`] と
    /// 同じ規則・同じ関数。影響元は持たない）。
    pub spray: Option<Spray>,
}

impl Default for DualBrush {
    fn default() -> Self {
        DualBrush {
            tip: None,
            radius: 8.0,
            hardness: 1.0,
            spacing: 0.25,
            angle: 0.0,
            roundness: 1.0,
            scatter: 0.0,
            count: 1,
            mode: DualBrushMode::Multiply,
            link_size: false,
            flow: 1.0,
            anti_alias: None,
            angle_tilt: false,
            angle_rotation: false,
            angle_line: false,
            angle_random: 0.0,
            tips: Vec::new(),
            selection: TipSelection::Random,
            flip_x: FlipRule::Off,
            flip_y: FlipRule::Off,
            darken_overlap: false,
            texture: None,
            spray: None,
        }
    }
}

/// 色の変化（C# のカラーダイナミクス。Color・Emission のチャンネルだけ。0 は無し）。色は保存された値のまま HSV で動かす。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorDynamics {
    /// 背景色（描画色/背景色のゆらぎの相手）。
    pub secondary: Rgba8,
    /// 各ダブが背景色へ 0〜これだけ寄る（0〜1）。
    pub foreground_background: f64,
    /// 色相を最大 ±これ × 180°、彩度・明るさを最大 ±これだけ動かす（0〜1）。
    pub hue: f64,
    pub saturation: f64,
    pub brightness: f64,
    /// 純度: −1（灰色）〜 0（そのまま）〜 1（彩度いっぱい）。ゆらぎの前に当てる。
    pub purity: f64,
    /// true: ダブごとに新しい色（Photoshop の「描点ごとに適用」）。false: ストロークに 1 色。
    pub per_tip: bool,
    /// 先端の色の変化（描画色を色相・彩度・明度で動かす。切は None）。変化量 = 値 × 影響元の係数の積（[`ColorItem`]）。
    pub tip: Option<TipColorChange>,
    /// サブ描画色混合率（0〜1。描画色をサブ描画色へこの割合だけ寄せる。影響元は [`ColorItem::SubMix`]）。先端の色の変化の切り替えによらず効く。
    pub sub_mix: f64,
    /// ストロークごとの変化（ストロークの始めに 1 回だけランダムに引く。切は None）。
    pub stroke: Option<StrokeColorChange>,
    /// 先端とストロークごとの色相・彩度・明度・サブを当てる描画色。
    pub target: ColorTarget,
}

impl Default for ColorDynamics {
    fn default() -> Self {
        ColorDynamics {
            secondary: Rgba8::new(0, 0, 0, 255),
            foreground_background: 0.0,
            hue: 0.0,
            saturation: 0.0,
            brightness: 0.0,
            purity: 0.0,
            per_tip: true,
            tip: None,
            sub_mix: 0.0,
            stroke: None,
            target: ColorTarget::Main,
        }
    }
}

/// 先端の色の変化の値（影響元の係数を掛ける前）。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct TipColorChange {
    /// 色相（度。−360〜360）。
    pub hue: f64,
    /// 彩度・明度（−1〜1）。
    pub saturation: f64,
    pub value: f64,
}

/// ストロークごとの変化の大きさ。色相・彩度・明度は ±値、サブ描画色混合率は 0〜値の一様なばらつき。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StrokeColorChange {
    /// 色相（度。0〜180）。
    pub hue: f64,
    /// 彩度・明度・サブ描画色混合率（0〜1）。
    pub saturation: f64,
    pub value: f64,
    pub sub_mix: f64,
}

/// 色の変化を当てる描画色。このアプリはいつもメイン描画色で描くので、「サブ」だけのときは色相・彩度・明度・ストロークごとの変化を当てない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorTarget {
    /// メイン描画色。
    #[default]
    Main,
    /// メインとサブ描画色。
    MainAndSub,
    /// サブ描画色だけ。
    Sub,
}

impl ColorTarget {
    /// すべての変化対象（並びは保存の順）。
    pub const ALL: [ColorTarget; 3] =
        [ColorTarget::Main, ColorTarget::MainAndSub, ColorTarget::Sub];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            ColorTarget::Main => "main",
            ColorTarget::MainAndSub => "main_and_sub",
            ColorTarget::Sub => "sub",
        }
    }
}

impl ColorDynamics {
    /// 先端の色相・彩度・明度の変化を当てるか（変化対象が「サブ」だけなら当てない）。
    pub fn tip_active(&self) -> bool {
        self.tip.is_some() && self.target != ColorTarget::Sub
    }

    /// ストロークごとの変化を当てるか（変化対象が「サブ」だけなら当てない）。
    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some() && self.target != ColorTarget::Sub
    }

    /// 色の変化（先端・サブ描画色混合率・ストロークごと）のどれかが効くか。
    pub fn change_active(&self) -> bool {
        self.tip_active() || self.sub_mix > 0.0 || self.stroke_active()
    }

    /// 色の変化の値を確かめる（有限で範囲の中）。
    pub(crate) fn validate_change(&self) -> Result<(), CoreError> {
        let mut checks: Vec<(f64, f64, f64)> = vec![(self.sub_mix, 0.0, 1.0)];
        if let Some(t) = &self.tip {
            checks.extend([
                (t.hue, -360.0, 360.0),
                (t.saturation, -1.0, 1.0),
                (t.value, -1.0, 1.0),
            ]);
        }
        if let Some(s) = &self.stroke {
            checks.extend([
                (s.hue, 0.0, 180.0),
                (s.saturation, 0.0, 1.0),
                (s.value, 0.0, 1.0),
                (s.sub_mix, 0.0, 1.0),
            ]);
        }
        for (v, lo, hi) in checks {
            require_finite(v, Reason::ColorChange)?;
            if !(lo..=hi).contains(&v) {
                return Err(CoreError::InvalidArgument(Reason::ColorChangeRange));
            }
        }
        Ok(())
    }
}

/// 影響元のほかの操作: フェード（ストロークの何番目の描点か）とペンの傾きの向き（Photoshop の「コントロール」）。
/// 拡張（C# に無い）: ペンの軸の回転を角度へ。項目を傾き・速度・筆圧で変える影響元は [`Brush::influences`]。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Controls {
    /// 大きさ・不透明度・流量が、この数の描点（間隔の刻み）にわたって 1 から 0 へ下がる（0 は切、1〜10000）。
    pub fade_size: u32,
    pub fade_opacity: u32,
    pub fade_flow: u32,
    /// ペンが倒れている向きを筆先の角度に足す。
    pub tilt_angle: bool,
    /// 拡張: ペンの軸の回転（[`super::BrushSample::rotation`]）を筆先の角度に足す。回転の情報の無い入力は 0。
    pub rotation_angle: bool,
    /// 拡張: 最低 1 ピクセル。筆圧・フェード・傾き・速さ・入り抜き・大きさのゆらぎで細くなった後のダブの直径に、下限 1 画素（半径 0.5）を
    /// 掛ける（直径 0 になるダブも 1 画素で残る）。1 画素は文書の画素（3D の面の塗りも、ブラシの大きさと同じ換算の文書の画素）。
    /// 間隔は公称の大きさで決まるので変わらない。縁のアンチエイリアスの小さなダブの広げと濃さは、この下限の後の大きさに掛かる。切（既定）は
    /// 今までと同じストローク。
    pub min_one_pixel: bool,
    /// 拡張: 画面上のサイズで指定（CLIP STUDIO PAINT の「画面上のサイズで指定」）。入れると、ブラシの大きさ（半径）は文書の画素でなく画面の点で
    /// 決める値になり、ストロークの始めに 半径 = 画面上の大きさ ÷ 表示倍率（[`Brush::at_view_scale`]）の文書の画素へ直して固定する（途中で
    /// 表示を拡大・縮小しても、そのストロークは変えない）。切（既定）は今までと同じストローク。
    pub size_by_screen: bool,
}

/// 線の補助（画素。0 は切）。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StrokeAssist {
    /// 手ぶれ補正の糸の長さ: 筆は入力がこれより離れたときだけ、この距離だけ遅れて付いていく（Krita・Lazy Nezumi の「引っ張る糸」）。
    /// 確定のとき最後の入力の点まで描く。入力の頻度によらない。0〜10000。
    pub stabilizer: f64,
    /// 入り・抜き: 線の長さの最初の taper_in 画素で大きさが 0 から育ち、最後の taper_out 画素で 0 へ細る。抜きの範囲のダブは、
    /// 線が続くか終わるまで待つ（終わりはそのときに分かる）。0〜10000。
    pub taper_in: f64,
    pub taper_out: f64,
    /// 入り・抜きを掛ける項目と、項目ごとの最小値（既定は大きさだけ・最小値 0）。
    pub taper_targets: TaperTargets,
    /// 道の点（手ぶれ補正の後）を centripetal Catmull-Rom の曲線で結ぶ。区間は次の点が来たとき（最後の区間は確定のとき）に描く。
    pub curve: bool,
}

/// 入り・抜きを掛ける項目と、項目ごとの最小値（0〜1）。項目の係数は `最小値 + (1 − 最小値) × 入り抜きの係数`（入りの先頭と抜きの終わりで最小値、
/// 範囲の外で 1）。掛ける項目は [`Item`] の 8 つ。既定は大きさだけ・最小値 0（入り抜きが大きさにだけ効いていた今までと同じ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaperTargets {
    /// [`Item::ALL`] の並びで、掛けるなら最小値。
    mins: [Option<f64>; 8],
}

impl Default for TaperTargets {
    /// 大きさだけ・最小値 0。
    fn default() -> Self {
        TaperTargets::SIZE_ONLY
    }
}

impl TaperTargets {
    /// 大きさだけ・最小値 0（既定）。
    pub const SIZE_ONLY: TaperTargets = TaperTargets {
        mins: [Some(0.0), None, None, None, None, None, None, None],
    };

    /// どの項目にも掛けない。
    pub const NONE: TaperTargets = TaperTargets { mins: [None; 8] };

    fn slot(item: Item) -> usize {
        match item {
            Item::Size => 0,
            Item::Opacity => 1,
            Item::Flow => 2,
            Item::Hardness => 3,
            Item::Roundness => 4,
            Item::TextureDensity => 5,
            Item::MixPaint => 6,
            Item::MixDensity => 7,
        }
    }

    /// 項目に掛けるなら、その最小値（0〜1）。
    pub fn min(&self, item: Item) -> Option<f64> {
        self.mins[Self::slot(item)]
    }

    /// 項目へ掛けるか（`Some(最小値)`）・掛けないか（`None`）を変えた写し。範囲外・有限でない最小値は断る。
    pub fn with(mut self, item: Item, min: Option<f64>) -> Result<TaperTargets, CoreError> {
        if let Some(m) = min {
            Self::check(m)?;
        }
        self.mins[Self::slot(item)] = min;
        Ok(self)
    }

    /// 既定（大きさだけ・最小値 0）か。
    pub fn is_default(&self) -> bool {
        *self == TaperTargets::SIZE_ONLY
    }

    fn check(min: f64) -> Result<(), CoreError> {
        require_finite(min, crate::Reason::TaperMinimum)?;
        if !(0.0..=1.0).contains(&min) {
            return Err(CoreError::InvalidArgument(crate::Reason::TaperMinimumRange));
        }
        Ok(())
    }

    /// 持っている最小値を検査する。
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        self.mins.iter().flatten().try_for_each(|m| Self::check(*m))
    }
}

/// 効果のブラシ（C# の BrushEffect）。効果は色を塗らず、レイヤーの今の画素から読んだ色をストロークの覆いで混ぜる。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BrushEffect {
    /// 色を塗る（消しゴムは [`BrushSettings::erase`]）。
    #[default]
    Paint,
    /// ぼかし: 半径（1〜64 画素）の箱の平均（プリマルチプライド）へ寄せる。透明な画素には色を広げない。
    Blur { radius: u32 },
    /// 指先: 前のダブの位置の色を引きずる。強さ（0〜1）を流量に掛ける。
    Smudge { strength: f64 },
    /// クローン（今のレイヤーから）: offset だけ離れた所の、ストロークの前の画素を写す（±1e7 画素）。
    Clone { offset: DVec2 },
}

impl BrushEffect {
    /// ぼかしの既定（C# の BlurRadius = 3）。
    pub const BLUR: BrushEffect = BrushEffect::Blur { radius: 3 };
    /// 指先の既定（C# の SmudgeStrength = 0.5）。
    pub const SMUDGE: BrushEffect = BrushEffect::Smudge { strength: 0.5 };
    pub fn is_paint(&self) -> bool {
        matches!(self, BrushEffect::Paint)
    }
}

/// フェードの長さの上限（C# の MaxFade）。
pub const MAX_FADE: u32 = 10000;
/// 最低 1 ピクセル（[`Controls::min_one_pixel`]）の下限の半径（画素。直径 1 画素）。
pub const MIN_ONE_PIXEL_RADIUS: f64 = 0.5;
/// 手ぶれ補正・入り抜きの上限（C# の MaxStrokeAssist）。
pub const MAX_STROKE_ASSIST: f64 = 10000.0;

impl Brush {
    /// C# の BrushSettings.Validate と同じ範囲を確かめる（断ったら何も変えない）。
    pub fn validate(&self) -> Result<(), CoreError> {
        self.base.validate()?;
        let t = &self.tip;
        let j = &self.jitter;
        for (v, what) in [
            (t.angle, Reason::Angle),
            (t.roundness, Reason::Roundness),
            (j.size, Reason::Jitter),
            (j.angle, Reason::Jitter),
            (j.roundness, Reason::Jitter),
            (j.opacity, Reason::Jitter),
            (j.flow, Reason::Jitter),
            (j.scatter, Reason::Scatter),
        ] {
            require_finite(v, what)?;
        }
        let a = &self.assist;
        for v in [a.stabilizer, a.taper_in, a.taper_out] {
            require_finite(v, Reason::StrokeAssistance)?;
            if !(0.0..=MAX_STROKE_ASSIST).contains(&v) {
                return Err(CoreError::InvalidArgument(Reason::StabilizerAndTaperRange));
            }
        }
        a.taper_targets.validate()?;
        if !(0.01..=1.0).contains(&t.roundness) {
            return Err(CoreError::InvalidArgument(Reason::Roundness));
        }
        for v in [j.size, j.angle, j.roundness, j.opacity, j.flow] {
            if !(0.0..=1.0).contains(&v) {
                return Err(CoreError::InvalidArgument(Reason::JitterRange));
            }
        }
        if !(0.0..=10.0).contains(&j.scatter) {
            return Err(CoreError::InvalidArgument(Reason::Scatter));
        }
        if !(1..=16).contains(&j.count) {
            return Err(CoreError::InvalidArgument(Reason::Count));
        }
        if let Some(tex) = &self.texture {
            tex.validate()?;
        }
        if t.images.len() > 256 {
            return Err(CoreError::InvalidArgument(Reason::BrushTipSequenceRange));
        }
        let c = &self.color;
        for v in [c.foreground_background, c.hue, c.saturation, c.brightness] {
            require_finite(v, Reason::ColourDynamics)?;
            if !(0.0..=1.0).contains(&v) {
                return Err(CoreError::InvalidArgument(Reason::ColorDynamicsRange));
            }
        }
        require_finite(c.purity, Reason::Purity)?;
        if !(-1.0..=1.0).contains(&c.purity) {
            return Err(CoreError::InvalidArgument(Reason::PurityRange));
        }
        c.validate_change()?;
        let k = &self.controls;
        for f in [k.fade_size, k.fade_opacity, k.fade_flow] {
            if f > MAX_FADE {
                return Err(CoreError::InvalidArgument(Reason::FadeRange));
            }
        }
        self.influences.validate()?;
        if let Some(d) = &self.dual {
            d.validate()?;
        }
        self.spray.as_ref().map_or(Ok(()), Spray::validate)?;
        self.mix.validate()?;
        match self.effect {
            BrushEffect::Paint => {}
            BrushEffect::Blur { radius } => {
                if !(1..=64).contains(&radius) {
                    return Err(CoreError::InvalidArgument(Reason::BlurRadiusRange));
                }
            }
            BrushEffect::Smudge { strength } => {
                require_finite(strength, Reason::SmudgeStrength)?;
                if !(0.0..=1.0).contains(&strength) {
                    return Err(CoreError::InvalidArgument(Reason::SmudgeStrengthRange));
                }
            }
            BrushEffect::Clone { offset } => {
                for v in [offset.x, offset.y] {
                    require_finite(v, Reason::CloneOffset)?;
                    if v.abs() > 10_000_000.0 {
                        return Err(CoreError::InvalidArgument(Reason::ClonePositionRange));
                    }
                }
            }
        }
        if !self.effect.is_paint() && self.base.erase {
            return Err(CoreError::InvalidArgument(Reason::EffectBrushesCannotErase));
        }
        self.symmetry.validate()?;
        if (self.symmetry.enabled() || self.model_symmetry.is_some())
            && matches!(
                self.effect,
                BrushEffect::Smudge { .. } | BrushEffect::Clone { .. }
            )
        {
            return Err(CoreError::Unsupported(
                Reason::SmudgeAndCloneWithoutSymmetry,
            ));
        }
        Ok(())
    }

    /// 色の変化を持たないチャンネル（Roughness・Metallic・Height・Normal・マスク）へ描く写し（C# の ForChannel）。
    /// データのチャンネルを色相などで揺らすと値を壊すだけなので、色の変化を外す。
    pub(crate) fn without_color_dynamics(&self) -> Brush {
        let mut b = self.clone();
        let c = &mut b.color;
        c.foreground_background = 0.0;
        c.hue = 0.0;
        c.saturation = 0.0;
        c.brightness = 0.0;
        c.purity = 0.0;
        c.tip = None;
        c.sub_mix = 0.0;
        c.stroke = None;
        b
    }

    /// 筆圧を、不透明度・流量・絵の具の量と濃さの影響元の筆圧の応え（最小値と曲線）に通した係数。筆圧を使っていない項目は 1。
    /// 応えが既定なら筆圧そのもの（C# と同じ値）。
    pub(crate) fn pressure_scale(&self, pressure: f64) -> super::PressureScale {
        let inf = &self.influences;
        let f = |i: &super::Influence| i.pressure_factor(pressure).unwrap_or(1.0);
        super::PressureScale {
            opacity: f(&inf.opacity),
            flow: f(&inf.flow),
            mix_paint: f(&inf.mix_paint),
            mix_density: f(&inf.mix_density),
        }
    }

    /// 今のダブで使う筆先の並び（`images` が空でなければそれ、空なら None）。
    pub(crate) fn tip_list(&self) -> Option<&[Arc<BrushTip>]> {
        if self.tip.images.is_empty() {
            None
        } else {
            Some(&self.tip.images)
        }
    }
}

impl DualBrush {
    /// C# の DualBrush.Validate。
    pub fn validate(&self) -> Result<(), CoreError> {
        for v in [
            self.radius,
            self.hardness,
            self.spacing,
            self.angle,
            self.roundness,
            self.scatter,
            self.flow,
            self.angle_random,
        ] {
            require_finite(v, Reason::DualBrush)?;
        }
        if !(0.0..=1.0).contains(&self.flow) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushFlow));
        }
        if !(0.0..=1.0).contains(&self.angle_random) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushAngleRandom));
        }
        if !(0.5..=65536.0).contains(&self.radius) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushRadiusRange));
        }
        if !(0.0..=1.0).contains(&self.hardness) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushHardness));
        }
        if !(0.01..=4.0).contains(&self.spacing) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushSpacing));
        }
        if !(0.01..=1.0).contains(&self.roundness) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushRoundness));
        }
        if !(0.0..=10.0).contains(&self.scatter) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushScatter));
        }
        if !(1..=16).contains(&self.count) {
            return Err(CoreError::InvalidArgument(Reason::DualBrushCount));
        }
        if self.tips.len() > 256 {
            return Err(CoreError::InvalidArgument(Reason::DualBrushTips));
        }
        if let Some(tex) = &self.texture {
            tex.validate()?;
        }
        self.spray.as_ref().map_or(Ok(()), Spray::validate)
    }

    /// 反転のしかた（主の [`TipShape::flip_rules`] と同じ形）。
    pub(crate) fn flip_rules(&self) -> super::plan::FlipRules {
        super::plan::FlipRules {
            x: self.flip_x,
            y: self.flip_y,
        }
    }

    /// 溜まりへ掛ける紙の質感（深さ 0 は無しと同じ）。
    pub(crate) fn paper(&self) -> Option<&PaperTexture> {
        self.texture.as_ref().filter(|t| t.depth > 0.0)
    }
}
