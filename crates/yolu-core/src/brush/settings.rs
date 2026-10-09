//! 全部入りのブラシの設定（C# の BrushSettings の全体）。M1 の [`BrushSettings`]（大きさ・硬さ・間隔・不透明度・流量・色・筆圧・
//! 消しゴム）に、筆先の形・ゆらぎ・紙の質感・デュアルブラシ・色の変化・フェードと傾き・手ぶれ補正と入り抜き・曲線・効果を足したもの。
//! 部分の分け方は Photoshop のブラシのウィンドウの節に合わせた（画面の節にそのまま当てられる）。
//!
//! 既定値はどれも C# の既定値と同じで、`Brush::from(settings)` は M1 の丸いブラシとバイト単位で同じストロークになる。
//! C# に無いもの（拡張）: 筆先の反転（[`TipShape::flip_x`]・[`TipShape::flip_y`]）と、紙の質感のモード（[`TextureMode`] の
//! 乗算以外）。どれも既定では C# と同じ。

use std::sync::Arc;

use glam::DVec2;

use super::mix::ColorMix;
use super::pressure::PressureResponses;
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
    /// 大きさ・硬さ・間隔・不透明度・流量・色・筆圧・消しゴム（M1 の設定）。
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
    /// 筆圧の応え（項目ごとの最小値と曲線。拡張: C# に無い）。切り替えは [`BrushSettings`] の `pressure_*` と
    /// [`Controls::pressure_hardness`]。既定（最小値 0・直線）は、切り替えが真なら筆圧をそのまま使う（C# と同じ）。
    pub pressure: PressureResponses,
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
}

impl From<BrushSettings> for Brush {
    fn from(base: BrushSettings) -> Brush {
        Brush {
            base,
            ..Brush::default()
        }
    }
}

/// 複数の筆先から各ダブの筆先を選ぶ方法（C# の TipSelection）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TipSelection {
    /// 乱数で選ぶ（ストロークの乱数の列から 1 つ引く）。
    #[default]
    Random,
    /// 順に使う。
    Sequential,
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
    /// 拡張（C# に無い）: 筆先の画像を左右に反転する（回す前の筆先の横の軸で）。丸い筆先には効かない。
    pub flip_x: bool,
    /// 拡張（C# に無い）: 筆先の画像を上下に反転する。
    pub flip_y: bool,
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
            flip_x: false,
            flip_y: false,
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
    /// 乗算・大きさ 1 の質感。
    pub fn new(image: Arc<BrushTip>, depth: f64) -> PaperTexture {
        PaperTexture {
            image,
            depth,
            scale: 1.0,
            mode: TextureMode::Multiply,
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
}

impl DualBrushMode {
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
/// 合わせた被覆率で塗る。大きさは筆圧・ゆらぎ・入り抜きの影響を受けない（Photoshop と同じ）。
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
        }
    }
}

/// 筆圧のほかの操作: フェード（ストロークの何番目の描点か）とペンの傾き（Photoshop の「コントロール」）。
/// 拡張（C# に無い）: ペンの軸の回転を角度へ、筆の速さを大きさ・不透明度・流量へ、筆圧を硬さへ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// 大きさ・不透明度・流量が、この数の描点（間隔の刻み）にわたって 1 から 0 へ下がる（0 は切、1〜10000）。
    pub fade_size: u32,
    pub fade_opacity: u32,
    pub fade_flow: u32,
    /// 大きさ・不透明度・流量に 1 − 傾き（直立 0〜寝かせきって 1）を掛ける。傾きの無い入力（マウス）は直立。
    pub tilt_size: bool,
    pub tilt_opacity: bool,
    pub tilt_flow: bool,
    /// ペンが倒れている向きを筆先の角度に足す。
    pub tilt_angle: bool,
    /// 拡張: ペンの軸の回転（[`super::BrushSample::rotation`]）を筆先の角度に足す。回転の情報の無い入力は 0。
    pub rotation_angle: bool,
    /// 拡張: 大きさ・不透明度・流量に 1 − min(1, 速さ ÷ speed_max) を掛ける（速いほど小さく・薄く）。速さは手ぶれ補正の後の
    /// 筆の点の間の距離 ÷ 時刻の差で、点の間は線形に補間する。時刻が進まない入力（時刻を渡さない）では最初の 0 のまま。
    pub speed_size: bool,
    pub speed_opacity: bool,
    pub speed_flow: bool,
    /// 効きが一杯になる速さ（画素 / 時刻の単位。秒なら画素毎秒）。0 より大きい。
    pub speed_max: f64,
    /// 拡張: 筆圧で硬さを変える（硬さ × 筆圧の応え [`Brush::pressure`] の硬さ）。大きさ・不透明度・流量の切り替えは
    /// [`BrushSettings`] が持つ（C# と同じ）。
    pub pressure_hardness: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            fade_size: 0,
            fade_opacity: 0,
            fade_flow: 0,
            tilt_size: false,
            tilt_opacity: false,
            tilt_flow: false,
            tilt_angle: false,
            rotation_angle: false,
            speed_size: false,
            speed_opacity: false,
            speed_flow: false,
            speed_max: 3000.0,
            pressure_hardness: false,
        }
    }
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
    /// 道の点（手ぶれ補正の後）を centripetal Catmull-Rom の曲線で結ぶ。区間は次の点が来たとき（最後の区間は確定のとき）に描く。
    pub curve: bool,
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
/// 手ぶれ補正・入り抜きの上限（C# の MaxStrokeAssist）。
pub const MAX_STROKE_ASSIST: f64 = 10000.0;

impl Brush {
    /// C# の BrushSettings.Validate と同じ範囲を確かめる（断ったら何も変えない）。
    pub fn validate(&self) -> Result<(), CoreError> {
        self.base.validate()?;
        let t = &self.tip;
        let j = &self.jitter;
        for (v, what) in [
            (t.angle, "angle"),
            (t.roundness, "roundness"),
            (j.size, "jitter"),
            (j.angle, "jitter"),
            (j.roundness, "jitter"),
            (j.opacity, "jitter"),
            (j.flow, "jitter"),
            (j.scatter, "scatter"),
        ] {
            require_finite(v, what)?;
        }
        let a = &self.assist;
        for v in [a.stabilizer, a.taper_in, a.taper_out] {
            require_finite(v, "stroke assistance")?;
            if !(0.0..=MAX_STROKE_ASSIST).contains(&v) {
                return Err(CoreError::InvalidArgument(
                    "手ぶれ補正・入り抜き（0〜10000）",
                ));
            }
        }
        if !(0.01..=1.0).contains(&t.roundness) {
            return Err(CoreError::InvalidArgument("roundness"));
        }
        for v in [j.size, j.angle, j.roundness, j.opacity, j.flow] {
            if !(0.0..=1.0).contains(&v) {
                return Err(CoreError::InvalidArgument("jitter（0〜1）"));
            }
        }
        if !(0.0..=10.0).contains(&j.scatter) {
            return Err(CoreError::InvalidArgument("scatter"));
        }
        if !(1..=16).contains(&j.count) {
            return Err(CoreError::InvalidArgument("count"));
        }
        if let Some(tex) = &self.texture {
            require_finite(tex.depth, "texture depth")?;
            require_finite(tex.scale, "texture scale")?;
            if !(0.0..=1.0).contains(&tex.depth) {
                return Err(CoreError::InvalidArgument("texture depth（0〜1）"));
            }
            if !(0.05..=64.0).contains(&tex.scale) {
                return Err(CoreError::InvalidArgument("texture scale（0.05〜64）"));
            }
        }
        if t.images.len() > 256 {
            return Err(CoreError::InvalidArgument("筆先の並び（1〜256 枚）"));
        }
        let c = &self.color;
        for v in [c.foreground_background, c.hue, c.saturation, c.brightness] {
            require_finite(v, "colour dynamics")?;
            if !(0.0..=1.0).contains(&v) {
                return Err(CoreError::InvalidArgument("色のゆらぎ（0〜1）"));
            }
        }
        require_finite(c.purity, "purity")?;
        if !(-1.0..=1.0).contains(&c.purity) {
            return Err(CoreError::InvalidArgument("purity（−1〜1）"));
        }
        let k = &self.controls;
        for f in [k.fade_size, k.fade_opacity, k.fade_flow] {
            if f > MAX_FADE {
                return Err(CoreError::InvalidArgument("フェード（0〜10000）"));
            }
        }
        require_finite(k.speed_max, "speed_max")?;
        if k.speed_max <= 0.0 {
            return Err(CoreError::InvalidArgument("速さの上限（0 より大きい）"));
        }
        if let Some(d) = &self.dual {
            d.validate()?;
        }
        self.mix.validate()?;
        match self.effect {
            BrushEffect::Paint => {}
            BrushEffect::Blur { radius } => {
                if !(1..=64).contains(&radius) {
                    return Err(CoreError::InvalidArgument("ぼかしの半径（1〜64）"));
                }
            }
            BrushEffect::Smudge { strength } => {
                require_finite(strength, "smudge strength")?;
                if !(0.0..=1.0).contains(&strength) {
                    return Err(CoreError::InvalidArgument("指先の強さ（0〜1）"));
                }
            }
            BrushEffect::Clone { offset } => {
                for v in [offset.x, offset.y] {
                    require_finite(v, "clone offset")?;
                    if v.abs() > 10_000_000.0 {
                        return Err(CoreError::InvalidArgument("クローンの位置（±1e7）"));
                    }
                }
            }
        }
        if !self.effect.is_paint() && self.base.erase {
            return Err(CoreError::InvalidArgument(
                "効果のブラシは消しゴムにできない",
            ));
        }
        self.symmetry.validate()?;
        if (self.symmetry.enabled() || self.model_symmetry.is_some())
            && matches!(
                self.effect,
                BrushEffect::Smudge { .. } | BrushEffect::Clone { .. }
            )
        {
            return Err(CoreError::Unsupported(
                "指先・クローンは対称と組めない（写しごとに読み元と動きが要る）",
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
        b
    }

    /// 筆圧を、不透明度・流量の応え（最小値と曲線）に通した係数。切っている項目は 1。応えが既定なら筆圧そのもの（C# と同じ値）。
    pub(crate) fn pressure_scale(&self, pressure: f64) -> super::PressureScale {
        let (mix_paint, mix_density) = self.mix.pressure_factors(pressure);
        super::PressureScale {
            opacity: if self.base.pressure_opacity {
                self.pressure.opacity.apply(pressure)
            } else {
                1.0
            },
            flow: if self.base.pressure_flow {
                self.pressure.flow.apply(pressure)
            } else {
                1.0
            },
            mix_paint,
            mix_density,
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
        ] {
            require_finite(v, "dual brush")?;
        }
        if !(0.5..=65536.0).contains(&self.radius) {
            return Err(CoreError::InvalidArgument(
                "デュアルブラシの半径（0.5〜65536）",
            ));
        }
        if !(0.0..=1.0).contains(&self.hardness) {
            return Err(CoreError::InvalidArgument("デュアルブラシの硬さ"));
        }
        if !(0.01..=4.0).contains(&self.spacing) {
            return Err(CoreError::InvalidArgument("デュアルブラシの間隔"));
        }
        if !(0.01..=1.0).contains(&self.roundness) {
            return Err(CoreError::InvalidArgument("デュアルブラシの真円率"));
        }
        if !(0.0..=10.0).contains(&self.scatter) {
            return Err(CoreError::InvalidArgument("デュアルブラシの散布"));
        }
        if !(1..=16).contains(&self.count) {
            return Err(CoreError::InvalidArgument("デュアルブラシの数"));
        }
        Ok(())
    }
}
