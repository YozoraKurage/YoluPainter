//! 効果の名前と一覧の 1 行の文字（日本語は Unity 版の ja.po と同じ言葉、英語は Unity 版の原文）。名前・値・短い状態だけで、説明は置かない。

use yolu_core::effects::{generator_kind_name, EffectSettings};
use yolu_core::filter::Settings as Filter;
use yolu_core::generator::{
    anchor::ReadMode, Blend, CellOutput, FractalMode, Kind, NoiseBasis, NoiseSpace,
    ProceduralSpace, Shape,
};
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
    GradientMap,
    ToneCurve,
    ColorBalance,
    BrightnessContrast,
    Threshold,
    Posterize,
}

impl FilterKind {
    pub const ALL: [FilterKind; 13] = [
        FilterKind::Blur,
        FilterKind::Sharpen,
        FilterKind::NoiseMono,
        FilterKind::NoiseColor,
        FilterKind::Levels,
        FilterKind::Invert,
        FilterKind::Normalize,
        FilterKind::GradientMap,
        FilterKind::ToneCurve,
        FilterKind::ColorBalance,
        FilterKind::BrightnessContrast,
        FilterKind::Threshold,
        FilterKind::Posterize,
    ];

    /// 色調補正の 6 種（調整の層と同じ値。Rust 版だけの種類）なら、その種類。
    fn color_adjust(self) -> Option<yolu_core::AdjustmentType> {
        use yolu_core::AdjustmentType as T;
        Some(match self {
            FilterKind::GradientMap => T::GradientMap,
            FilterKind::ToneCurve => T::ToneCurve,
            FilterKind::ColorBalance => T::ColorBalance,
            FilterKind::BrightnessContrast => T::BrightnessContrast,
            FilterKind::Threshold => T::Threshold,
            FilterKind::Posterize => T::Posterize,
            _ => return None,
        })
    }

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
            other => EffectSettings::from_color_adjust(
                yolu_core::ColorAdjust::default_for(other.color_adjust().expect("色調補正の種類"))
                    .expect("色調補正の既定値"),
            ),
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
            FilterKind::GradientMap => lang.pick("グラデーションマップ", "Gradient Map"),
            FilterKind::ToneCurve => lang.pick("トーンカーブ", "Tone Curve"),
            FilterKind::ColorBalance => lang.pick("カラーバランス", "Color Balance"),
            FilterKind::BrightnessContrast => {
                lang.pick("明るさ・コントラスト", "Brightness / Contrast")
            }
            FilterKind::Threshold => lang.pick("2 値化", "Threshold"),
            FilterKind::Posterize => lang.pick("ポスタリゼーション", "Posterize"),
        }
    }
}

/// 足せる Generator の種類（メニューの並び）。
pub const GENERATOR_KINDS: [Kind; 10] = [
    Kind::EdgeWear,
    Kind::Dirt,
    Kind::PositionGradient,
    Kind::ShapeGradient,
    Kind::Thickness,
    Kind::Direction,
    Kind::IdColor,
    Kind::Anchor,
    Kind::Noise,
    Kind::Grunge,
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
            Filter::GradientMap(_) => FilterKind::GradientMap.name(lang),
            Filter::ToneCurve(_) => FilterKind::ToneCurve.name(lang),
            Filter::ColorBalance(_) => FilterKind::ColorBalance.name(lang),
            Filter::BrightnessContrast(_) => FilterKind::BrightnessContrast.name(lang),
            Filter::Threshold(_) => FilterKind::Threshold.name(lang),
            Filter::Posterize(_) => FilterKind::Posterize.name(lang),
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

pub const PROCEDURAL_SPACES: [ProceduralSpace; 3] = [
    ProceduralSpace::Position,
    ProceduralSpace::Uv,
    ProceduralSpace::Triplanar,
];

pub fn procedural_space_name(lang: Lang, space: ProceduralSpace) -> &'static str {
    match space {
        ProceduralSpace::Position => lang.pick("位置", "Position"),
        ProceduralSpace::Uv => "UV",
        ProceduralSpace::Triplanar => lang.pick("トライプラナー", "Triplanar"),
    }
}

pub const NOISE_BASES: [NoiseBasis; 3] =
    [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley];

pub fn noise_basis_name(lang: Lang, basis: NoiseBasis) -> &'static str {
    match basis {
        NoiseBasis::Value => lang.pick("値", "Value"),
        NoiseBasis::Perlin => "Perlin",
        NoiseBasis::Worley => "Worley",
    }
}

pub const CELL_OUTPUTS: [CellOutput; 3] = [CellOutput::F1, CellOutput::F2, CellOutput::F2MinusF1];

pub fn cell_output_name(output: CellOutput) -> &'static str {
    match output {
        CellOutput::F1 => "F1",
        CellOutput::F2 => "F2",
        CellOutput::F2MinusF1 => "F2−F1",
    }
}

pub const FRACTAL_MODES: [FractalMode; 3] = [
    FractalMode::Fbm,
    FractalMode::Ridged,
    FractalMode::Turbulence,
];

pub fn fractal_mode_name(mode: FractalMode) -> &'static str {
    match mode {
        FractalMode::Fbm => "fBm",
        FractalMode::Ridged => "ridged",
        FractalMode::Turbulence => "turbulence",
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

/// 形の名前（ワールドスペースのグラデーションの形。塗りつぶしの欄の「形」・新規塗りつぶしレイヤーのメニュー・効果の欄が同じ名前を使う）。
/// 平面は後ろが 0・前が 1 の、1 方向のグラデーション（線形）。
pub fn shape_name(lang: Lang, shape: Shape) -> &'static str {
    match shape {
        Shape::Box => lang.pick("ボックス", "Box"),
        Shape::Sphere => lang.pick("球", "Sphere"),
        Shape::Plane => lang.pick("平面（線形）", "Plane (Linear)"),
    }
}

/// 形の短い説明（メニューの項目のツールチップ）。
pub fn shape_hint(lang: Lang, shape: Shape) -> &'static str {
    match shape {
        Shape::Box => lang.pick(
            "箱の中は 1、面へ向かって 0 に消える",
            "1 inside the box, fading to 0 at its faces",
        ),
        Shape::Sphere => lang.pick(
            "球の中は 1、表面へ向かって 0 に消える",
            "1 inside the sphere, fading to 0 at its surface",
        ),
        Shape::Plane => lang.pick(
            "平面の後ろが 0、前が 1（1 方向のグラデーション）",
            "0 behind the plane to 1 in front of it (a one-direction gradient)",
        ),
    }
}

pub const SHAPES: [Shape; 3] = [Shape::Box, Shape::Sphere, Shape::Plane];

/// 形の欄（塗りつぶしの欄・効果の欄）のツールチップ: 3 つの形の「名前: 説明」を 1 行ずつ。メニューの項目と同じ名前と説明の文。
pub fn shape_tooltip(lang: Lang) -> String {
    SHAPES
        .iter()
        .map(|s| format!("{}: {}", shape_name(lang, *s), shape_hint(lang, *s)))
        .collect::<Vec<_>>()
        .join("\n")
}

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
    if placement == AnchorPlacement::Mask
        && !anchor.name().ends_with(')')
        && !anchor.name().ends_with('）')
    {
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
