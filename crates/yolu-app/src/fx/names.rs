//! 効果の名前と一覧の 1 行の文字（日本語は Unity 版の ja.po と同じ言葉、英語は Unity 版の原文）。名前・値・短い状態だけで、説明は置かない。

use yolu_core::effects::{generator_kind_name, EffectSettings};
use yolu_core::filter::Settings as Filter;
use yolu_core::generator::{anchor::ReadMode, Blend, Kind, NoiseSpace, Shape};
use yolu_core::{Anchor, AnchorPlacement, Channel, FilterEffect, FilterTarget};

use crate::lang::Lang;

/// 足せるフィルターの種類（足すときの既定値は Unity 版のメニューと同じ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Blur,
    Sharpen,
    NoiseMono,
    NoiseColor,
    Levels,
    Invert,
    Normalize,
}

impl FilterKind {
    pub const ALL: [FilterKind; 7] = [
        FilterKind::Blur,
        FilterKind::Sharpen,
        FilterKind::NoiseMono,
        FilterKind::NoiseColor,
        FilterKind::Levels,
        FilterKind::Invert,
        FilterKind::Normalize,
    ];

    /// 足すときの設定（ぼかし 4 px・シャープ 2 px ×1・ノイズ 25 %・レベル補正は何も変えない値）。
    pub fn settings(self) -> EffectSettings {
        match self {
            FilterKind::Blur => EffectSettings::blur(4),
            FilterKind::Sharpen => EffectSettings::sharpen(2, 1.0, 0),
            FilterKind::NoiseMono => EffectSettings::noise(0.25, 0, true),
            FilterKind::NoiseColor => EffectSettings::noise(0.25, 0, false),
            FilterKind::Levels => EffectSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0),
            FilterKind::Invert => EffectSettings::invert(),
            FilterKind::Normalize => EffectSettings::normalize(),
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            FilterKind::Blur => lang.pick("ぼかし（ガウス）", "Gaussian Blur"),
            FilterKind::Sharpen => lang.pick("シャープ", "Sharpen"),
            FilterKind::NoiseMono => lang.pick("ノイズ（モノクロ）", "Noise (Mono)"),
            FilterKind::NoiseColor => lang.pick("ノイズ（カラー）", "Noise (Color)"),
            FilterKind::Levels => lang.pick("レベル補正", "Levels"),
            FilterKind::Invert => lang.pick("階調の反転", "Invert"),
            FilterKind::Normalize => lang.pick("正規化（レイヤー全体）", "Normalize (Layer)"),
        }
    }
}

/// 足せる Generator の種類（メニューの並び）。
pub const GENERATOR_KINDS: [Kind; 8] = [
    Kind::EdgeWear,
    Kind::Dirt,
    Kind::PositionGradient,
    Kind::ShapeGradient,
    Kind::Thickness,
    Kind::Direction,
    Kind::IdColor,
    Kind::Anchor,
];

pub fn generator_name(lang: Lang, kind: Kind) -> &'static str {
    match kind {
        Kind::EdgeWear => lang.pick("エッジの摩耗", "Edge Wear"),
        Kind::Dirt => lang.pick("汚れ・隙間", "Dirt"),
        Kind::PositionGradient => lang.pick("位置の勾配", "Position Gradient"),
        Kind::Thickness => lang.pick("厚み", "Thickness"),
        Kind::Direction => lang.pick("向き", "Direction"),
        Kind::ShapeGradient => lang.pick("形のグラデーション", "Shape Gradient"),
        Kind::IdColor => lang.pick("ID の色", "ID Color"),
        Kind::Anchor => lang.pick("アンカー", "Anchor"),
        Kind::Noise => lang.pick("ノイズ", "Noise"),
        Kind::Grunge => lang.pick("グランジ", "Grunge"),
    }
}

/// 段の名前（フィルターも Generator も）。
pub fn effect_name(lang: Lang, settings: &EffectSettings) -> &'static str {
    match settings {
        EffectSettings::Generator(g) => generator_name(lang, g.kind),
        EffectSettings::Filter(f) => match f {
            Filter::GaussianBlur { .. } => FilterKind::Blur.name(lang),
            Filter::Sharpen { .. } => FilterKind::Sharpen.name(lang),
            Filter::Noise {
                monochrome: true, ..
            } => FilterKind::NoiseMono.name(lang),
            Filter::Noise { .. } => FilterKind::NoiseColor.name(lang),
            Filter::Levels { .. } => FilterKind::Levels.name(lang),
            Filter::Invert => FilterKind::Invert.name(lang),
            Filter::Normalize => FilterKind::Normalize.name(lang),
            Filter::Generator { .. } => generator_kind_name(Kind::EdgeWear), // 文書には置かれない形
        },
    }
}

pub fn effect_icon(settings: &EffectSettings) -> &'static str {
    if settings.is_generator() {
        "texture"
    } else {
        "auto_awesome"
    }
}

pub fn blend_name(lang: Lang, blend: Blend) -> &'static str {
    match blend {
        Blend::Multiply => lang.pick("乗算", "Multiply"),
        Blend::Replace => lang.pick("置き換え", "Replace"),
        Blend::Screen => lang.pick("スクリーン", "Screen"),
        Blend::Max => lang.pick("最大", "Max"),
        Blend::Min => lang.pick("最小", "Min"),
        Blend::Add => lang.pick("加算", "Add"),
        Blend::Subtract => lang.pick("減算", "Subtract"),
    }
}

pub const BLENDS: [Blend; 7] = [
    Blend::Multiply,
    Blend::Replace,
    Blend::Screen,
    Blend::Max,
    Blend::Min,
    Blend::Add,
    Blend::Subtract,
];

pub fn noise_space_name(lang: Lang, space: NoiseSpace) -> &'static str {
    match space {
        NoiseSpace::Model => lang.pick("モデルの上（3D）", "On the model (3D)"),
        NoiseSpace::Uv => lang.pick("UV（継ぎ目が出る）", "UV (seams show)"),
    }
}

pub fn axis_name(axis: usize) -> &'static str {
    ["X", "Y", "Z"].get(axis).copied().unwrap_or("Y")
}

/// 向きの選択肢（ワールドの軸の 6 方向）。
pub const DIRECTIONS: [[f64; 3]; 6] = [
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

pub fn direction_name(lang: Lang, d: [f64; 3]) -> String {
    if d == [0.0, 1.0, 0.0] {
        lang.pick("上（+Y）", "Up (+Y)").into()
    } else if d == [0.0, -1.0, 0.0] {
        lang.pick("下（−Y）", "Down (−Y)").into()
    } else if d == [1.0, 0.0, 0.0] {
        "+X".into()
    } else if d == [-1.0, 0.0, 0.0] {
        "−X".into()
    } else if d == [0.0, 0.0, 1.0] {
        "+Z".into()
    } else if d == [0.0, 0.0, -1.0] {
        "−Z".into()
    } else {
        format!("({:.2}, {:.2}, {:.2})", d[0], d[1], d[2])
    }
}

pub fn shape_name(lang: Lang, shape: Shape) -> &'static str {
    match shape {
        Shape::Box => lang.pick("ボックス", "Box"),
        Shape::Sphere => lang.pick("球", "Sphere"),
        Shape::Plane => lang.pick("平面", "Plane"),
    }
}

pub const SHAPES: [Shape; 3] = [Shape::Box, Shape::Sphere, Shape::Plane];

pub fn read_mode_name(lang: Lang, mode: ReadMode) -> &'static str {
    match mode {
        ReadMode::Value => lang.pick("値", "Value"),
        ReadMode::Coverage => lang.pick("覆い", "Coverage"),
    }
}

/// Anchor の Generator が読めるチャンネル（Normal は 1 画素 1 値ではないので無い）。
pub const ANCHOR_CHANNELS: [Channel; 5] = [
    Channel::Color,
    Channel::Roughness,
    Channel::Metallic,
    Channel::Height,
    Channel::Emission,
];

/// Anchor の一覧での名前（マスクの Anchor で名前が「）」で終わらないものは「（マスク）」を足す）。
pub fn anchor_label(lang: Lang, anchor: &Anchor, placement: AnchorPlacement) -> String {
    if placement == AnchorPlacement::Mask && !anchor.name().ends_with(')') && !anchor.name().ends_with('）') {
        lang.pick(
            format!("{}（マスク）", anchor.name()),
            format!("{} (mask)", anchor.name()),
        )
    } else {
        anchor.name().to_owned()
    }
}

/// 一覧の 1 行の文字: 名前と主な値（効かないチャンネルなら効くチャンネル、強さが 1 でなければ強さ）。Generator の効いていない印と
/// Anchor の名前は呼ぶ側が足す。
pub fn effect_label(
    lang: Lang,
    effect: &FilterEffect,
    target: FilterTarget,
    paint_channel: Channel,
    channel_name: impl Fn(Channel) -> String,
) -> String {
    let mut text = effect_name(lang, effect.settings()).to_owned();
    match effect.settings() {
        EffectSettings::Filter(Filter::GaussianBlur { radius }) => {
            text += &format!("  {radius} px");
        }
        EffectSettings::Filter(Filter::Sharpen { radius, amount, .. }) => {
            text += &format!("  {radius} px ×{}", trim(*amount, 2));
        }
        EffectSettings::Filter(Filter::Noise { amount, .. }) => {
            text += &format!("  {}%", (amount * 100.0).round() as i32);
        }
        EffectSettings::Generator(g) => {
            text += &format!("  {}", blend_name(lang, g.blend));
        }
        _ => {}
    }
    if target == FilterTarget::Content && !effect.applies_to(paint_channel) {
        let names: Vec<String> = effect.channels().iter().map(|c| channel_name(*c)).collect();
        text += &format!("  [{}]", names.join(", "));
    }
    if effect.strength() < 1.0 {
        text += &format!("  · {}%", (effect.strength() * 100.0).round() as i32);
    }
    text
}

/// 小数 `places` 桁までで、後ろの 0 を落とした文字（Unity の "0.##"）。
pub fn trim(v: f64, places: usize) -> String {
    let mut s = format!("{v:.places$}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}
