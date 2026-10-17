//! ブラシの詳細の「区分と行の表」。左の区分の一覧（クリスタのサブツール詳細の並び）と、区分ごとの行（設定 1 つ）を、1 つの表に持つ。
//! 行は、詳細のウィンドウの欄にも、ツールプロパティのパネルにも出る（どの行をツールプロパティに出すかは、ブラシごとに覚える目の印。
//! `Brush::shown_rows`）。行を足すときは、[`Row`] に 1 つ足し、`id`・`category`・`default_shown` を決め、`panels::brush_rows` の区分の関数に 1 つ足す
//! （足し忘れは、全部の行を描いてみるテストが落とす）。区分を足すときは [`Category`] に足す。エンジンの無い区分
//! （水彩境界・はみ出し防止・後補正など）は、エンジンが入るときにここへ足す。
//!
//! ツールプロパティの行の並びは、詳細の区分の順（区分の中は [`Row::ALL`] の順）。目の印の初めの形（`Brush::shown_rows` が None）は、
//! 前のツールプロパティが出していた行（[`Row::default_shown`]）。

use crate::engine::{Brush, ColorItem, Influence, Influences};
use crate::lang::Lang;
use yolu_core::brush::Item;
use yolu_core::{DualItem, SprayItem};

/// 詳細のウィンドウの左の区分。並びは [`Category::ALL`]（左の一覧の順）。`Shape`・`Dual` は子の区分を持つ見出し（中に行は無い）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Size,
    Ink,
    ColorChange,
    AntiAlias,
    /// ブラシ形状（見出し）。
    Shape,
    Tip,
    Spray,
    Stroke,
    Paper,
    /// 2-ブラシ形状（見出し）。
    Dual,
    DualTip,
    DualSpray,
    DualStroke,
    DualPaper,
    Erase,
    Stabilization,
    Taper,
}

impl Category {
    /// 左の一覧の並び。
    pub const ALL: [Category; 17] = [
        Category::Size,
        Category::Ink,
        Category::ColorChange,
        Category::AntiAlias,
        Category::Shape,
        Category::Tip,
        Category::Spray,
        Category::Stroke,
        Category::Paper,
        Category::Dual,
        Category::DualTip,
        Category::DualSpray,
        Category::DualStroke,
        Category::DualPaper,
        Category::Erase,
        Category::Stabilization,
        Category::Taper,
    ];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Category::Size => lang.t("brushrows-brush-size"),
            Category::Ink => lang.t("brushrows-ink"),
            Category::ColorChange => lang.t("brushrows-color-change"),
            Category::AntiAlias => lang.t("brushrows-anti-aliasing"),
            Category::Shape => lang.t("brushrows-brush-shape"),
            Category::Tip => lang.t("brushrows-brush-tip"),
            Category::Spray => lang.t("brushrows-spray"),
            Category::Stroke => lang.t("brushrows-stroke"),
            Category::Paper => lang.t("brushrows-paper"),
            Category::Dual => lang.t("brushrows-2-brush-shape"),
            Category::DualTip => lang.t("brushrows-2-brush-tip"),
            Category::DualSpray => lang.t("brushrows-2-spray"),
            Category::DualStroke => lang.t("brushrows-2-stroke"),
            Category::DualPaper => lang.t("brushrows-2-paper"),
            Category::Erase => lang.t("brushrows-erase"),
            Category::Stabilization => lang.t("brushrows-stabilization"),
            Category::Taper => lang.t("brushrows-taper"),
        }
    }

    /// 子の区分を持つ見出しか（中に行は無い。押すと最初の子へ移る）。
    pub fn is_group(self) -> bool {
        matches!(self, Category::Shape | Category::Dual)
    }

    /// 子の区分（字下げして一覧に出る）の見出し。
    pub fn parent(self) -> Option<Category> {
        match self {
            Category::Tip | Category::Spray | Category::Stroke | Category::Paper => {
                Some(Category::Shape)
            }
            Category::DualTip
            | Category::DualSpray
            | Category::DualStroke
            | Category::DualPaper => Some(Category::Dual),
            _ => None,
        }
    }

    /// 見出しの最初の子（見出しでなければ自分）。
    pub fn first_page(self) -> Category {
        match self {
            Category::Shape => Category::Tip,
            Category::Dual => Category::DualTip,
            other => other,
        }
    }
}

/// 影響元を持つ項目 1 つ（[`Influences`] の項目。影響元設定のウィンドウが、どの項目の物かを持つ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dynamics {
    Item(Item),
    Color(ColorItem),
    Spray(SprayItem),
    Dual(DualItem),
}

impl Dynamics {
    pub fn get(self, influences: &Influences) -> &Influence {
        match self {
            Dynamics::Item(i) => influences.get(i),
            Dynamics::Color(i) => influences.color(i),
            Dynamics::Spray(i) => influences.spray(i),
            Dynamics::Dual(i) => influences.dual(i),
        }
    }

    pub fn get_mut(self, influences: &mut Influences) -> &mut Influence {
        match self {
            Dynamics::Item(i) => influences.get_mut(i),
            Dynamics::Color(i) => influences.color_mut(i),
            Dynamics::Spray(i) => influences.spray_mut(i),
            Dynamics::Dual(i) => influences.dual_mut(i),
        }
    }

    /// 応えの最小値が −1〜1 か（色相・彩度・明度）。
    pub fn signed(self) -> bool {
        matches!(self, Dynamics::Color(i) if i.signed())
    }
}

/// 行（設定 1 つ）。詳細の欄とツールプロパティの両方が、同じ行を出す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    // ブラシサイズ
    Size,
    SizeByScreen,
    MinOnePixel,
    JitterSize,
    // インク
    Opacity,
    Flow,
    JitterOpacity,
    JitterFlow,
    MixMode,
    MixPaint,
    MixDensity,
    MixStretch,
    MixGround,
    EffectKind,
    EffectBlur,
    EffectSmudge,
    EffectCloneX,
    EffectCloneY,
    EffectCloneAllLayers,
    EffectCloneAligned,
    // 色の変化
    ColorTip,
    ColorHue,
    ColorSaturation,
    ColorValue,
    ColorSubMix,
    ColorStroke,
    ColorStrokeHue,
    ColorStrokeSaturation,
    ColorStrokeValue,
    ColorStrokeSubMix,
    ColorTarget,
    ColorBackground,
    ColorJitterForeBack,
    ColorJitterHue,
    ColorJitterSaturation,
    ColorJitterBrightness,
    ColorPurity,
    ColorPerTip,
    // アンチエイリアス
    AntiAlias,
    // ブラシ先端
    Hardness,
    Roundness,
    TipAngle,
    FollowDirection,
    TiltAngle,
    RotationAngle,
    JitterAngle,
    JitterRoundness,
    Repeat,
    FlipX,
    FlipY,
    // 散布効果
    Scatter,
    ScatterCount,
    // ストローク
    Spacing,
    DarkenOverlap,
    SpeedMax,
    ProjectionHidden,
    ProjectionBackfaces,
    ProjectionFalloff,
    ProjectionAngleStart,
    ProjectionAngleEnd,
    ProjectionBleed,
    // 紙質
    PaperImage,
    PaperDepth,
    PaperScale,
    PaperAngle,
    PaperMode,
    // 2-ブラシ形状
    DualEnabled,
    DualTip,
    DualMode,
    DualSize,
    DualHardness,
    DualAngle,
    DualRoundness,
    DualScatter,
    DualCount,
    DualSpacing,
    DualPaperImage,
    DualPaperDepth,
    DualPaperScale,
    DualPaperAngle,
    DualPaperMode,
    // 消去
    Erase,
    // 補正
    Stabilizer,
    Curve,
    // 入り抜き
    TaperIn,
    TaperOut,
    FadeSize,
    FadeOpacity,
    FadeFlow,
}

impl Row {
    /// 全部の行（区分の中の並び。ツールプロパティも、区分の順に、この並びで出す）。
    pub const ALL: [Row; 89] = [
        Row::Size,
        Row::SizeByScreen,
        Row::MinOnePixel,
        Row::JitterSize,
        Row::Opacity,
        Row::Flow,
        Row::JitterOpacity,
        Row::JitterFlow,
        Row::MixMode,
        Row::MixPaint,
        Row::MixDensity,
        Row::MixStretch,
        Row::MixGround,
        Row::EffectKind,
        Row::EffectBlur,
        Row::EffectSmudge,
        Row::EffectCloneX,
        Row::EffectCloneY,
        Row::EffectCloneAllLayers,
        Row::EffectCloneAligned,
        Row::ColorTip,
        Row::ColorHue,
        Row::ColorSaturation,
        Row::ColorValue,
        Row::ColorSubMix,
        Row::ColorStroke,
        Row::ColorStrokeHue,
        Row::ColorStrokeSaturation,
        Row::ColorStrokeValue,
        Row::ColorStrokeSubMix,
        Row::ColorTarget,
        Row::ColorBackground,
        Row::ColorJitterForeBack,
        Row::ColorJitterHue,
        Row::ColorJitterSaturation,
        Row::ColorJitterBrightness,
        Row::ColorPurity,
        Row::ColorPerTip,
        Row::AntiAlias,
        Row::Hardness,
        Row::Roundness,
        Row::TipAngle,
        Row::FollowDirection,
        Row::TiltAngle,
        Row::RotationAngle,
        Row::JitterAngle,
        Row::JitterRoundness,
        Row::Repeat,
        Row::FlipX,
        Row::FlipY,
        Row::Scatter,
        Row::ScatterCount,
        Row::Spacing,
        Row::DarkenOverlap,
        Row::SpeedMax,
        Row::ProjectionHidden,
        Row::ProjectionBackfaces,
        Row::ProjectionFalloff,
        Row::ProjectionAngleStart,
        Row::ProjectionAngleEnd,
        Row::ProjectionBleed,
        Row::PaperImage,
        Row::PaperDepth,
        Row::PaperScale,
        Row::PaperAngle,
        Row::PaperMode,
        Row::DualEnabled,
        Row::DualTip,
        Row::DualMode,
        Row::DualSize,
        Row::DualHardness,
        Row::DualAngle,
        Row::DualRoundness,
        Row::DualScatter,
        Row::DualCount,
        Row::DualSpacing,
        Row::DualPaperImage,
        Row::DualPaperDepth,
        Row::DualPaperScale,
        Row::DualPaperAngle,
        Row::DualPaperMode,
        Row::Erase,
        Row::Stabilizer,
        Row::Curve,
        Row::TaperIn,
        Row::TaperOut,
        Row::FadeSize,
        Row::FadeOpacity,
        Row::FadeFlow,
    ];

    /// ファイルに書く名前（ブラシの `rows.shown`）。変えない。
    pub fn id(self) -> &'static str {
        match self {
            Row::Size => "size",
            Row::SizeByScreen => "size.by_screen",
            Row::MinOnePixel => "size.min_one_pixel",
            Row::JitterSize => "size.jitter",
            Row::Opacity => "opacity",
            Row::Flow => "flow",
            Row::JitterOpacity => "opacity.jitter",
            Row::JitterFlow => "flow.jitter",
            Row::MixMode => "mix.mode",
            Row::MixPaint => "mix.paint",
            Row::MixDensity => "mix.density",
            Row::MixStretch => "mix.stretch",
            Row::MixGround => "mix.all_layers",
            Row::EffectKind => "effect.kind",
            Row::EffectBlur => "effect.blur",
            Row::EffectSmudge => "effect.smudge",
            Row::EffectCloneX => "effect.clone_x",
            Row::EffectCloneY => "effect.clone_y",
            Row::EffectCloneAllLayers => "effect.clone_all_layers",
            Row::EffectCloneAligned => "effect.clone_aligned",
            Row::ColorTip => "color.tip",
            Row::ColorHue => "color.hue",
            Row::ColorSaturation => "color.saturation",
            Row::ColorValue => "color.value",
            Row::ColorSubMix => "color.sub_mix",
            Row::ColorStroke => "color.stroke",
            Row::ColorStrokeHue => "color.stroke_hue",
            Row::ColorStrokeSaturation => "color.stroke_saturation",
            Row::ColorStrokeValue => "color.stroke_value",
            Row::ColorStrokeSubMix => "color.stroke_sub_mix",
            Row::ColorTarget => "color.target",
            Row::ColorBackground => "color.background",
            Row::ColorJitterForeBack => "color.jitter_fg_bg",
            Row::ColorJitterHue => "color.jitter_hue",
            Row::ColorJitterSaturation => "color.jitter_saturation",
            Row::ColorJitterBrightness => "color.jitter_brightness",
            Row::ColorPurity => "color.purity",
            Row::ColorPerTip => "color.per_tip",
            Row::AntiAlias => "anti_alias",
            Row::Hardness => "tip.hardness",
            Row::Roundness => "tip.roundness",
            Row::TipAngle => "tip.angle",
            Row::FollowDirection => "tip.follow_direction",
            Row::TiltAngle => "tip.tilt_angle",
            Row::RotationAngle => "tip.rotation_angle",
            Row::JitterAngle => "tip.angle_jitter",
            Row::JitterRoundness => "tip.roundness_jitter",
            Row::Repeat => "tip.repeat",
            Row::FlipX => "tip.flip_x",
            Row::FlipY => "tip.flip_y",
            Row::Scatter => "spray.scatter",
            Row::ScatterCount => "spray.count",
            Row::Spacing => "stroke.spacing",
            Row::DarkenOverlap => "stroke.darken",
            Row::SpeedMax => "stroke.speed_max",
            Row::ProjectionHidden => "stroke.projection_hidden",
            Row::ProjectionBackfaces => "stroke.projection_backfaces",
            Row::ProjectionFalloff => "stroke.projection_falloff",
            Row::ProjectionAngleStart => "stroke.projection_angle_start",
            Row::ProjectionAngleEnd => "stroke.projection_angle_end",
            Row::ProjectionBleed => "stroke.projection_bleed",
            Row::PaperImage => "paper.image",
            Row::PaperDepth => "paper.depth",
            Row::PaperScale => "paper.scale",
            Row::PaperAngle => "paper.angle",
            Row::PaperMode => "paper.mode",
            Row::DualEnabled => "dual.enabled",
            Row::DualTip => "dual.tip",
            Row::DualMode => "dual.mode",
            Row::DualSize => "dual.size",
            Row::DualHardness => "dual.hardness",
            Row::DualAngle => "dual.angle",
            Row::DualRoundness => "dual.roundness",
            Row::DualScatter => "dual.scatter",
            Row::DualCount => "dual.count",
            Row::DualSpacing => "dual.spacing",
            Row::DualPaperImage => "dual.paper.image",
            Row::DualPaperDepth => "dual.paper.depth",
            Row::DualPaperScale => "dual.paper.scale",
            Row::DualPaperAngle => "dual.paper.angle",
            Row::DualPaperMode => "dual.paper.mode",
            Row::Erase => "erase",
            Row::Stabilizer => "stabilizer",
            Row::Curve => "stabilizer.curve",
            Row::TaperIn => "taper.in",
            Row::TaperOut => "taper.out",
            Row::FadeSize => "fade.size",
            Row::FadeOpacity => "fade.opacity",
            Row::FadeFlow => "fade.flow",
        }
    }

    pub fn from_id(id: &str) -> Option<Row> {
        Row::ALL.into_iter().find(|r| r.id() == id)
    }

    /// 行が属する区分（見出しの区分は持たない）。
    pub fn category(self) -> Category {
        match self {
            Row::Size | Row::SizeByScreen | Row::MinOnePixel | Row::JitterSize => Category::Size,
            Row::Opacity
            | Row::Flow
            | Row::JitterOpacity
            | Row::JitterFlow
            | Row::MixMode
            | Row::MixPaint
            | Row::MixDensity
            | Row::MixStretch
            | Row::MixGround
            | Row::EffectKind
            | Row::EffectBlur
            | Row::EffectSmudge
            | Row::EffectCloneX
            | Row::EffectCloneY
            | Row::EffectCloneAllLayers
            | Row::EffectCloneAligned => Category::Ink,
            Row::ColorTip
            | Row::ColorHue
            | Row::ColorSaturation
            | Row::ColorValue
            | Row::ColorSubMix
            | Row::ColorStroke
            | Row::ColorStrokeHue
            | Row::ColorStrokeSaturation
            | Row::ColorStrokeValue
            | Row::ColorStrokeSubMix
            | Row::ColorTarget
            | Row::ColorBackground
            | Row::ColorJitterForeBack
            | Row::ColorJitterHue
            | Row::ColorJitterSaturation
            | Row::ColorJitterBrightness
            | Row::ColorPurity
            | Row::ColorPerTip => Category::ColorChange,
            Row::AntiAlias => Category::AntiAlias,
            Row::Hardness
            | Row::Roundness
            | Row::TipAngle
            | Row::FollowDirection
            | Row::TiltAngle
            | Row::RotationAngle
            | Row::JitterAngle
            | Row::JitterRoundness
            | Row::Repeat
            | Row::FlipX
            | Row::FlipY => Category::Tip,
            Row::Scatter | Row::ScatterCount => Category::Spray,
            Row::Spacing
            | Row::DarkenOverlap
            | Row::SpeedMax
            | Row::ProjectionHidden
            | Row::ProjectionBackfaces
            | Row::ProjectionFalloff
            | Row::ProjectionAngleStart
            | Row::ProjectionAngleEnd
            | Row::ProjectionBleed => Category::Stroke,
            Row::PaperImage
            | Row::PaperDepth
            | Row::PaperScale
            | Row::PaperAngle
            | Row::PaperMode => Category::Paper,
            Row::DualEnabled
            | Row::DualTip
            | Row::DualMode
            | Row::DualSize
            | Row::DualHardness
            | Row::DualAngle
            | Row::DualRoundness => Category::DualTip,
            Row::DualScatter | Row::DualCount => Category::DualSpray,
            Row::DualSpacing => Category::DualStroke,
            Row::DualPaperImage
            | Row::DualPaperDepth
            | Row::DualPaperScale
            | Row::DualPaperAngle
            | Row::DualPaperMode => Category::DualPaper,
            Row::Erase => Category::Erase,
            Row::Stabilizer | Row::Curve => Category::Stabilization,
            Row::TaperIn | Row::TaperOut | Row::FadeSize | Row::FadeOpacity | Row::FadeFlow => {
                Category::Taper
            }
        }
    }

    /// 行の名前（詳細の欄の行の名前、ツールプロパティの溝の中の名前、影響元設定のウィンドウの名前に使う）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Row::Size => lang.t("panels-size"),
            Row::SizeByScreen => lang.t("brushrows-size-on-screen"),
            Row::MinOnePixel => lang.t("panels-at-least-1-pixel"),
            Row::JitterSize => lang.t("brushrows-size-jitter"),
            Row::Opacity => lang.t("panels-opacity"),
            Row::Flow => lang.t("panels-flow"),
            Row::JitterOpacity => lang.t("brushrows-opacity-jitter"),
            Row::JitterFlow => lang.t("brushrows-flow-jitter"),
            Row::MixMode => lang.t("brushrows-mixing"),
            Row::MixPaint => lang.t("panels-paint-amount"),
            Row::MixDensity => lang.t("panels-paint-density"),
            Row::MixStretch => lang.t("panels-color-stretch"),
            Row::MixGround => lang.t("panels-all-layers"),
            Row::EffectCloneAllLayers => lang.t("brushrows-read-all-layers"),
            Row::EffectKind => lang.t("brushrows-effect"),
            Row::EffectBlur => lang.t("panels-blur-radius"),
            Row::EffectSmudge => lang.t("panels-smudge-strength"),
            Row::EffectCloneX => lang.t("panels-offset-x"),
            Row::EffectCloneY => lang.t("panels-offset-y"),
            Row::EffectCloneAligned => lang.t("panels-aligned"),
            Row::ColorTip => lang.t("brushrows-change-tip-color"),
            Row::ColorHue => lang.t("panels-hue"),
            Row::ColorSaturation => lang.t("panels-saturation"),
            Row::ColorValue => lang.t("brushrows-value"),
            Row::ColorSubMix => lang.t("brushrows-sub-color"),
            Row::ColorStroke => lang.t("brushrows-randomize-per-stroke"),
            Row::ColorStrokeHue => lang.t("brushrows-stroke-hue"),
            Row::ColorStrokeSaturation => lang.t("brushrows-stroke-saturation"),
            Row::ColorStrokeValue => lang.t("brushrows-stroke-value"),
            Row::ColorStrokeSubMix => lang.t("brushrows-stroke-sub-color"),
            Row::ColorTarget => lang.t("brushrows-change-target"),
            Row::ColorBackground => lang.t("panels-background"),
            Row::ColorJitterForeBack => lang.t("brushrows-fg-bg-jitter"),
            Row::ColorJitterHue => lang.t("brushrows-hue-jitter"),
            Row::ColorJitterSaturation => lang.t("brushrows-saturation-jitter"),
            Row::ColorJitterBrightness => lang.t("brushrows-brightness-jitter"),
            Row::ColorPurity => lang.t("panels-purity"),
            Row::ColorPerTip => lang.t("panels-apply-per-tip-text"),
            Row::AntiAlias => lang.t("panels-anti-aliasing-tip"),
            Row::Hardness => lang.t("panels-hardness"),
            Row::Roundness => lang.t("panels-roundness"),
            Row::TipAngle => lang.t("panels-angle"),
            Row::FollowDirection => lang.t("panels-follow-direction-tip"),
            Row::TiltAngle => lang.t("brushrows-add-tilt-to-angle"),
            Row::RotationAngle => lang.t("brushrows-add-rotation-to-angle"),
            Row::JitterAngle => lang.t("brushrows-angle-jitter"),
            Row::JitterRoundness => lang.t("brushrows-roundness-jitter"),
            Row::Repeat => lang.t("brushrows-repeat-method"),
            Row::FlipX => lang.t("panels-flip-x-tip"),
            Row::FlipY => lang.t("panels-flip-y-tip"),
            Row::Scatter => lang.t("panels-scatter"),
            Row::ScatterCount => lang.t("panels-count"),
            Row::Spacing => lang.t("panels-spacing"),
            Row::DarkenOverlap => lang.t("brushrows-blend-tips-with-darken"),
            Row::SpeedMax => lang.t("panels-full-effect-speed"),
            Row::ProjectionHidden => lang.t("panels-paint-hidden-areas"),
            Row::ProjectionBackfaces => lang.t("panels-paint-back-faces"),
            Row::ProjectionFalloff => lang.t("panels-fade-by-angle"),
            Row::ProjectionAngleStart => lang.t("panels-fade-start"),
            Row::ProjectionAngleEnd => lang.t("panels-fade-end"),
            Row::ProjectionBleed => lang.t("panels-seam-bleed"),
            Row::PaperImage => lang.t("brushrows-paper-image"),
            Row::PaperDepth => lang.t("brushrows-paper-depth"),
            Row::PaperScale => lang.t("panels-scale"),
            Row::PaperAngle => lang.t("brushrows-rotation"),
            Row::PaperMode => lang.t("panels-mode"),
            Row::DualEnabled => lang.t("panels-use-a-dual-brush"),
            Row::DualTip => lang.t("brushrows-2-tip"),
            Row::DualMode => lang.t("brushrows-2-mode"),
            Row::DualSize => lang.t("brushrows-2-size"),
            Row::DualHardness => lang.t("brushrows-2-hardness"),
            Row::DualAngle => lang.t("brushrows-2-angle"),
            Row::DualRoundness => lang.t("brushrows-2-roundness"),
            Row::DualScatter => lang.t("brushrows-2-scatter"),
            Row::DualCount => lang.t("brushrows-2-count"),
            Row::DualSpacing => lang.t("brushrows-2-spacing"),
            Row::DualPaperImage => lang.t("brushrows-2-paper-image"),
            Row::DualPaperDepth => lang.t("brushrows-2-paper-depth"),
            Row::DualPaperScale => lang.t("brushrows-2-paper-scale"),
            Row::DualPaperAngle => lang.t("brushrows-2-paper-rotation"),
            Row::DualPaperMode => lang.t("brushrows-2-paper-mode"),
            Row::Erase => lang.t("brushrows-erase"),
            Row::Stabilizer => lang.t("panels-stabilizer"),
            Row::Curve => lang.t("panels-curve"),
            Row::TaperIn => lang.t("brushrows-start-length"),
            Row::TaperOut => lang.t("brushrows-end-length"),
            Row::FadeSize => lang.t("brushrows-size-fade"),
            Row::FadeOpacity => lang.t("brushrows-opacity-fade"),
            Row::FadeFlow => lang.t("brushrows-flow-fade"),
        }
    }

    /// 行が影響元（筆圧・傾き・速度・ランダム）を持つなら、その項目（行の右に影響元のボタンが出る）。
    pub fn dynamics(self) -> Option<Dynamics> {
        Some(match self {
            Row::Size => Dynamics::Item(Item::Size),
            Row::Opacity => Dynamics::Item(Item::Opacity),
            Row::Flow => Dynamics::Item(Item::Flow),
            Row::MixPaint => Dynamics::Item(Item::MixPaint),
            Row::MixDensity => Dynamics::Item(Item::MixDensity),
            Row::Hardness => Dynamics::Item(Item::Hardness),
            Row::Roundness => Dynamics::Item(Item::Roundness),
            Row::PaperDepth => Dynamics::Item(Item::TextureDensity),
            Row::ColorHue => Dynamics::Color(ColorItem::Hue),
            Row::ColorSaturation => Dynamics::Color(ColorItem::Saturation),
            Row::ColorValue => Dynamics::Color(ColorItem::Value),
            Row::ColorSubMix => Dynamics::Color(ColorItem::SubMix),
            _ => return None,
        })
    }

    /// 前のツールプロパティが出していた行（目の印を持たない前の形のブラシと、既定のブラシが、この行に目が入った形で開く）。
    /// ぼかしの半径・指先の強さは、効果がそのブラシのときだけ出る。
    pub fn default_shown(self) -> bool {
        matches!(
            self,
            Row::Size
                | Row::Opacity
                | Row::Flow
                | Row::EffectBlur
                | Row::EffectSmudge
                | Row::Hardness
                | Row::Spacing
                | Row::Stabilizer
        )
    }

    /// 区分の行（[`Row::ALL`] の並び）。
    pub fn of(category: Category) -> impl Iterator<Item = Row> {
        Row::ALL
            .into_iter()
            .filter(move |r| r.category() == category)
    }
}

/// 目の印の初めの形（前のツールプロパティの行）の名前の並び。
pub fn default_shown_ids() -> Vec<&'static str> {
    Row::ALL
        .into_iter()
        .filter(|r| r.default_shown())
        .map(Row::id)
        .collect()
}

/// 保存・比べる形にそろえた目の印: 知らない名前を落とし、行の並びにし、初めの形（前のツールプロパティの行）と同じなら None。
pub fn normalize(shown: Option<&[String]>) -> Option<Vec<String>> {
    let shown = shown?;
    let rows: Vec<&'static str> = Row::ALL
        .into_iter()
        .map(Row::id)
        .filter(|id| shown.iter().any(|s| s == id))
        .collect();
    (rows != default_shown_ids()).then(|| rows.into_iter().map(str::to_owned).collect())
}

/// この行をツールプロパティに出す目が入っているか。
pub fn is_shown(brush: &Brush, row: Row) -> bool {
    match &brush.shown_rows {
        Some(rows) => rows.iter().any(|r| r == row.id()),
        None => row.default_shown(),
    }
}

/// この行の目を入れる・切る（初めの形と同じになれば、印を持たない形へ戻す）。
pub fn set_shown(brush: &mut Brush, row: Row, on: bool) {
    let mut rows: Vec<String> = Row::ALL
        .into_iter()
        .filter(|r| is_shown(brush, *r) && (*r != row || on))
        .map(|r| r.id().to_owned())
        .collect();
    if on && !rows.iter().any(|r| r == row.id()) {
        rows.push(row.id().to_owned());
    }
    brush.shown_rows = normalize(Some(&rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_one_id_and_a_page_category() {
        let mut ids: Vec<&str> = Row::ALL.iter().map(|r| r.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Row::ALL.len());
        for row in Row::ALL {
            assert_eq!(Row::from_id(row.id()), Some(row));
            assert!(
                !row.category().is_group(),
                "{row:?}: 見出しの区分に行は置かない"
            );
            assert!(Category::ALL.contains(&row.category()));
        }
        // 区分の並びの中に、行を持つ区分と見出しだけが並ぶ
        for category in Category::ALL {
            assert_eq!(
                Row::of(category).count() == 0,
                category.is_group(),
                "{category:?}"
            );
        }
    }

    #[test]
    fn the_default_form_is_the_rows_the_old_tool_properties_showed() {
        let brush = Brush::default();
        for row in Row::ALL {
            assert_eq!(is_shown(&brush, row), row.default_shown(), "{row:?}");
        }
        // 順は区分の順（大きさ・不透明度・流量・ぼかしと指先・硬さ・間隔・手ぶれ補正）
        let order: Vec<Row> = Category::ALL
            .into_iter()
            .flat_map(Row::of)
            .filter(|r| r.default_shown())
            .collect();
        assert_eq!(
            order,
            [
                Row::Size,
                Row::Opacity,
                Row::Flow,
                Row::EffectBlur,
                Row::EffectSmudge,
                Row::Hardness,
                Row::Spacing,
                Row::Stabilizer
            ]
        );
    }

    #[test]
    fn setting_the_eye_keeps_the_default_form_when_it_returns_to_it() {
        let mut brush = Brush::default();
        set_shown(&mut brush, Row::Size, false);
        assert!(!is_shown(&brush, Row::Size));
        assert!(brush.shown_rows.is_some());
        set_shown(&mut brush, Row::Size, true);
        assert_eq!(brush.shown_rows, None, "初めの形と同じなら印を持たない");
        set_shown(&mut brush, Row::Erase, true);
        assert!(is_shown(&brush, Row::Erase) && is_shown(&brush, Row::Size));
        assert_eq!(
            brush.shown_rows.as_ref().unwrap().len(),
            default_shown_ids().len() + 1
        );
    }

    #[test]
    fn normalizing_drops_unknown_names_and_follows_the_row_order() {
        let shown = vec![
            "flow".to_owned(),
            "no_such_row".to_owned(),
            "size".to_owned(),
        ];
        assert_eq!(
            normalize(Some(&shown)),
            Some(vec!["size".to_owned(), "flow".to_owned()])
        );
        assert_eq!(normalize(None), None);
        let default: Vec<String> = default_shown_ids().into_iter().map(str::to_owned).collect();
        assert_eq!(normalize(Some(&default)), None);
    }
}
