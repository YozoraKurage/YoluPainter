//! 効果の名前と一覧の 1 行の文字（日本語は Unity 版の ja.po と同じ言葉、英語は Unity 版の原文）。名前・値・短い状態だけで、説明は置かない。

use yolu_core::effects::{generator_kind_name, EffectSettings};
use yolu_core::fill_image::{ProjectionMode, Wrap};
use yolu_core::filter::Settings as Filter;
use yolu_core::generator::{
    anchor::ReadMode, Blend, CellOutput, FractalMode, ImageComponent, Kind, NoiseBasis, NoiseSpace,
    ProceduralSpace, Shape,
};
use yolu_core::{Anchor, AnchorPlacement, Channel, FilterEffect, FilterTarget};

use crate::lang::Lang;

/// 足せるフィルターの種類（足すときの既定値は Unity 版のメニューと同じ。0.5.0 の種類は効果の目録の既定）。
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
    HistogramScan,
    HistogramRange,
    SlopeBlur,
    DirectionalBlur,
    Warp,
    Morphology,
    EdgeDetect,
    HighPass,
    Median,
    Glow,
}

impl FilterKind {
    /// メニューの並び（ぼかしの仲間 → 輪郭・形 → ノイズ → 値の調整 → 色調補正）。
    pub const ALL: [FilterKind; 23] = [
        FilterKind::Blur,
        FilterKind::DirectionalBlur,
        FilterKind::SlopeBlur,
        FilterKind::Warp,
        FilterKind::Median,
        FilterKind::Sharpen,
        FilterKind::HighPass,
        FilterKind::Glow,
        FilterKind::EdgeDetect,
        FilterKind::Morphology,
        FilterKind::NoiseMono,
        FilterKind::NoiseColor,
        FilterKind::Levels,
        FilterKind::HistogramScan,
        FilterKind::HistogramRange,
        FilterKind::Invert,
        FilterKind::Normalize,
        FilterKind::GradientMap,
        FilterKind::ToneCurve,
        FilterKind::ColorBalance,
        FilterKind::BrightnessContrast,
        FilterKind::Threshold,
        FilterKind::Posterize,
    ];

    /// 0.5.0 の種類の、効果の目録の名前（値の欄は目録から作る）。
    pub fn catalog_id(self) -> Option<&'static str> {
        Some(match self {
            FilterKind::HistogramScan => "histogram_scan",
            FilterKind::HistogramRange => "histogram_range",
            FilterKind::SlopeBlur => "slope_blur",
            FilterKind::DirectionalBlur => "directional_blur",
            FilterKind::Warp => "warp",
            FilterKind::Morphology => "morphology",
            FilterKind::EdgeDetect => "edge_detect",
            FilterKind::HighPass => "high_pass",
            FilterKind::Median => "median",
            FilterKind::Glow => "glow",
            _ => return None,
        })
    }

    /// 色調補正の 6 種（調整レイヤーと同じ値。Rust 版だけの種類）なら、その種類。
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
            other if other.catalog_id().is_some() => EffectSettings::from_catalog(
                other.catalog_id().expect("目録の種類"),
                &Default::default(),
            )
            .expect("目録の既定は作れる"),
            other => EffectSettings::from_color_adjust(
                yolu_core::ColorAdjust::default_for(other.color_adjust().expect("色調補正の種類"))
                    .expect("色調補正の既定値"),
            ),
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            FilterKind::Blur => lang.t("fx-gaussian-blur"),
            FilterKind::Sharpen => lang.t("fx-sharpen-unsharp-mask"),
            FilterKind::NoiseMono => lang.t("fx-noise-mono"),
            FilterKind::NoiseColor => lang.t("fx-noise-color"),
            FilterKind::Levels => lang.t("fx-levels"),
            FilterKind::Invert => lang.t("fx-invert-2"),
            FilterKind::Normalize => lang.t("fx-normalize-layer"),
            FilterKind::GradientMap => lang.t("fx-gradient-map"),
            FilterKind::ToneCurve => lang.t("fx-tone-curve"),
            FilterKind::ColorBalance => lang.t("fx-color-balance"),
            FilterKind::BrightnessContrast => lang.t("fx-brightness-contrast"),
            FilterKind::Threshold => lang.t("fx-threshold"),
            FilterKind::Posterize => lang.t("fx-posterize"),
            FilterKind::HistogramScan => lang.t("fx-histogram-scan"),
            FilterKind::HistogramRange => lang.t("fx-histogram-range"),
            FilterKind::SlopeBlur => lang.t("fx-slope-blur"),
            FilterKind::DirectionalBlur => lang.t("fx-directional-blur"),
            FilterKind::Warp => lang.t("fx-warp"),
            FilterKind::Morphology => lang.t("fx-dilate-erode"),
            FilterKind::EdgeDetect => lang.t("fx-edge-detect"),
            FilterKind::HighPass => lang.t("fx-high-pass"),
            FilterKind::Median => lang.t("fx-median"),
            FilterKind::Glow => lang.t("fx-glow"),
        }
    }

    /// 段の設定の種類。
    pub fn of(settings: &Filter) -> Option<FilterKind> {
        Some(match settings {
            Filter::GaussianBlur { .. } => FilterKind::Blur,
            Filter::Sharpen { .. } => FilterKind::Sharpen,
            Filter::Noise {
                monochrome: true, ..
            } => FilterKind::NoiseMono,
            Filter::Noise { .. } => FilterKind::NoiseColor,
            Filter::Levels { .. } => FilterKind::Levels,
            Filter::Invert => FilterKind::Invert,
            Filter::Normalize => FilterKind::Normalize,
            Filter::GradientMap(_) => FilterKind::GradientMap,
            Filter::ToneCurve(_) => FilterKind::ToneCurve,
            Filter::ColorBalance(_) => FilterKind::ColorBalance,
            Filter::BrightnessContrast(_) => FilterKind::BrightnessContrast,
            Filter::Threshold(_) => FilterKind::Threshold,
            Filter::Posterize(_) => FilterKind::Posterize,
            Filter::HistogramScan { .. } => FilterKind::HistogramScan,
            Filter::HistogramRange { .. } => FilterKind::HistogramRange,
            Filter::SlopeBlur { .. } => FilterKind::SlopeBlur,
            Filter::DirectionalBlur { .. } => FilterKind::DirectionalBlur,
            Filter::Warp { .. } => FilterKind::Warp,
            Filter::Morphology { .. } => FilterKind::Morphology,
            Filter::EdgeDetect { .. } => FilterKind::EdgeDetect,
            Filter::HighPass { .. } => FilterKind::HighPass,
            Filter::Median { .. } => FilterKind::Median,
            Filter::Glow { .. } => FilterKind::Glow,
            Filter::Generator { .. } => return None,
        })
    }
}

/// 目録の値の欄の名前（0.5.0 のフィルター。`kind` は目録の種類の名前、`param` は欄の名前）。
pub fn param_label(lang: Lang, kind: &str, param: &str) -> &'static str {
    match (kind, param) {
        ("pattern", "shape") => lang.t("fx-shape"),
        ("pattern", "scale") => lang.t("fx-repeat"),
        ("pattern", "width") => lang.t("fx-width"),
        ("pattern", "softness") => lang.t("fx-blur"),
        ("pattern", "offset_u") => lang.t("fx-offset-u"),
        ("pattern", "offset_v") => lang.t("fx-offset-v"),
        ("light", "azimuth") => lang.t("fx-azimuth"),
        ("light", "elevation") => lang.t("fx-elevation"),
        ("light", "softness") => lang.t("fx-wrap"),
        ("light", "ambient") => lang.t("fx-ambient"),
        ("mask_builder", "combine") => lang.t("fx-combine"),
        ("mask_builder", p) if p.ends_with("_weight") => lang.t("fx-weight"),
        ("mask_builder", p) if p.ends_with("_level") => lang.t("fx-level"),
        ("mask_builder", p) if p.ends_with("_contrast") => lang.t("fx-contrast"),
        ("mask_builder", p) if p.ends_with("_invert") => lang.t("fx-invert"),
        ("uv_island_variation", "min") => lang.t("fx-min"),
        ("uv_island_variation", "max") => lang.t("fx-max"),
        (_, "position") => lang.t("fx-position"),
        (_, "contrast") => lang.t("fx-contrast"),
        (_, "range") => lang.t("fx-range"),
        ("warp", "intensity") => lang.t("fx-amount"),
        ("glow", "intensity") => lang.t("fx-intensity"),
        (_, "intensity") => lang.t("fx-length"),
        (_, "samples") => lang.t("fx-samples"),
        ("morphology", "mode") => lang.t("fx-mode"),
        (_, "mode") => lang.t("fx-mode-2"),
        (_, "scale") => lang.t("fx-noise-size"),
        (_, "seed") => lang.t("fx-seed"),
        (_, "angle") => lang.t("fx-angle"),
        (_, "distance") => lang.t("fx-distance"),
        (_, "width") => lang.t("fx-width-2"),
        (_, "threshold") => lang.t("fx-threshold-2"),
        _ => lang.t("fx-radius"),
    }
}

/// 目録の値の欄の前に置く小さな見出し（マスクの組み立てのマップの名前。無ければ None）。
pub fn param_group(lang: Lang, kind: &str, param: &str) -> Option<&'static str> {
    if kind != "mask_builder" {
        return None;
    }
    Some(match param {
        "curvature_weight" => lang.t("fx-curvature"),
        "ambient_occlusion_weight" => lang.t("fx-ambient-occlusion"),
        "position_weight" => lang.t("fx-height-position"),
        "thickness_weight" => lang.t("fx-thickness"),
        _ => return None,
    })
}

/// 目録の値の欄のツールチップ（無ければ None）。
pub fn param_hint(lang: Lang, kind: &str, param: &str) -> Option<&'static str> {
    Some(match (kind, param) {
        ("pattern", "scale") => lang.t("fx-how-many-times-the-pattern-tip"),
        ("pattern", "width") => lang.t("fx-width-of-stripes-dots-and-tip"),
        ("pattern", "softness") => lang.t("fx-blur-of-the-edges-tip"),
        ("light", "azimuth") => lang.t("fx-horizontal-direction-the-light-comes-tip"),
        ("light", "elevation") => lang.t("fx-height-of-the-light-0-tip"),
        ("light", "softness") => lang.t("fx-how-far-the-light-wraps-tip"),
        ("light", "ambient") => lang.t("fx-brightness-of-the-dark-side-tip"),
        ("mask_builder", p) if p.ends_with("_weight") => lang.t("fx-how-much-this-map-counts-tip"),
        ("mask_builder", p) if p.ends_with("_level") => {
            lang.t("fx-where-values-start-turning-to-tip")
        }
        ("mask_builder", "combine") => lang.t("fx-how-the-maps-are-combined-tip"),
        ("uv_island_variation", "seed") => lang.t("fx-the-same-seed-and-model-tip"),
        ("uv_island_variation", "min") => lang.t("fx-lowest-value-an-island-can-tip"),
        ("uv_island_variation", "max") => lang.t("fx-highest-value-an-island-can-tip"),
        ("histogram_scan", "position") => lang.t("fx-where-values-start-turning-to-2-tip"),
        ("histogram_scan", "contrast") => lang.t("fx-how-sharp-the-cut-is-tip"),
        ("histogram_range", "range") => lang.t("fx-how-far-the-values-spread-tip"),
        ("histogram_range", "position") => lang.t("fx-the-middle-value-tip"),
        ("slope_blur", "intensity") => lang.t("fx-how-far-along-the-noise-tip"),
        ("slope_blur", "mode") => lang.t("fx-average-minimum-or-maximum-of-tip"),
        ("slope_blur" | "warp", "scale") => lang.t("fx-size-of-one-blob-of-tip"),
        ("warp", "intensity") => lang.t("fx-how-far-the-pixels-are-tip"),
        ("directional_blur", "distance") => lang.t("fx-length-to-each-side-pixels-tip"),
        ("morphology", "radius") => lang.t("fx-round-window-radius-pixels-tip"),
        ("edge_detect", "width") => lang.t("fx-blur-before-finding-edges-pixels-tip"),
        ("edge_detect", "threshold") => lang.t("fx-edges-weaker-than-this-become-tip"),
        ("glow", "threshold") => lang.t("fx-only-parts-brighter-than-this-tip"),
        ("median", "radius") => lang.t("fx-square-window-radius-pixels-tip"),
        (_, "seed") => lang.t("fx-the-same-seed-gives-the-tip"),
        _ => return None,
    })
}

/// 目録の選択肢の名前（0.5.0 のフィルター）。
pub fn option_label(lang: Lang, option: &str) -> &'static str {
    match option {
        "blur" => lang.t("fx-average"),
        "min" => lang.t("fx-min"),
        "max" => lang.t("fx-max"),
        "dilate" => lang.t("fx-dilate"),
        "erode" => lang.t("fx-erode"),
        "stripes" => lang.t("fx-stripes"),
        "checker" => lang.t("fx-checker"),
        "dots" => lang.t("fx-dots"),
        "border" => lang.t("fx-border"),
        "grid" => lang.t("fx-grid"),
        "multiply" => lang.t("fx-multiply"),
        "add" => lang.t("fx-add"),
        _ => "?",
    }
}

/// 足せる Generator の種類（メニューの並び: 焼いたマップ・モデルを読む種類 → 読まない種類 → 画像）。
pub const GENERATOR_KINDS: [Kind; 15] = [
    Kind::EdgeWear,
    Kind::Dirt,
    Kind::PositionGradient,
    Kind::ShapeGradient,
    Kind::Thickness,
    Kind::Direction,
    Kind::Light,
    Kind::MaskBuilder,
    Kind::UvIslandVariation,
    Kind::IdColor,
    Kind::Anchor,
    Kind::Noise,
    Kind::Grunge,
    Kind::Pattern,
    Kind::Image,
];

/// 画像の段の投影の種類（塗りつぶしレイヤーの投影の、デカールを除いたもの。並びも同じ）。
pub const IMAGE_PROJECTIONS: [ProjectionMode; 5] = [
    ProjectionMode::Uv,
    ProjectionMode::Triplanar,
    ProjectionMode::Planar,
    ProjectionMode::Spherical,
    ProjectionMode::Cylindrical,
];

/// 画像の段の投影の外側（塗りつぶしレイヤーと同じ並び）。
pub const IMAGE_WRAPS: [Wrap; 3] = [Wrap::Repeat, Wrap::Clamp, Wrap::None];

/// 画像の段がマスク・スカラーで値にする成分の名前。
pub fn image_component_name(lang: Lang, c: ImageComponent) -> &'static str {
    match c {
        ImageComponent::Red => lang.t("fx-r"),
        ImageComponent::Green => lang.t("fx-g"),
        ImageComponent::Blue => lang.t("fx-b"),
        ImageComponent::Alpha => lang.t("fx-alpha"),
        ImageComponent::Luminance => lang.t("fx-luminance"),
    }
}

pub fn generator_name(lang: Lang, kind: Kind) -> &'static str {
    match kind {
        Kind::EdgeWear => lang.t("fx-edge-wear"),
        Kind::Dirt => lang.t("fx-dirt"),
        Kind::PositionGradient => lang.t("fx-position-gradient"),
        Kind::Thickness => lang.t("fx-thickness"),
        Kind::Direction => lang.t("fx-direction"),
        Kind::ShapeGradient => lang.t("fx-shape-gradient"),
        Kind::IdColor => lang.t("fx-id-color"),
        Kind::Anchor => lang.t("fx-anchor"),
        Kind::Noise => lang.t("fx-noise"),
        Kind::Grunge => lang.t("fx-grunge"),
        Kind::Image => lang.t("fx-image-2"),
        Kind::Pattern => lang.t("fx-pattern"),
        Kind::Light => lang.t("fx-light"),
        Kind::MaskBuilder => lang.t("fx-mask-builder"),
        Kind::UvIslandVariation => lang.t("fx-uv-island-variation"),
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
            other => FilterKind::of(other).map_or("?", |k| k.name(lang)),
        },
    }
}

pub fn effect_icon(settings: &EffectSettings) -> &'static str {
    if settings.is_generator() {
        "generator"
    } else {
        "auto_awesome"
    }
}

pub fn blend_name(lang: Lang, blend: Blend) -> &'static str {
    match blend {
        Blend::Multiply => lang.t("fx-multiply"),
        Blend::Replace => lang.t("fx-replace"),
        Blend::Screen => lang.t("fx-screen"),
        Blend::Max => lang.t("fx-max"),
        Blend::Min => lang.t("fx-min"),
        Blend::Add => lang.t("fx-add"),
        Blend::Subtract => lang.t("fx-subtract"),
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
        NoiseSpace::Model => lang.t("fx-on-the-model-3d"),
        NoiseSpace::Uv => lang.t("fx-uv-seams-show"),
    }
}

pub const PROCEDURAL_SPACES: [ProceduralSpace; 3] = [
    ProceduralSpace::Position,
    ProceduralSpace::Uv,
    ProceduralSpace::Triplanar,
];

pub fn procedural_space_name(lang: Lang, space: ProceduralSpace) -> &'static str {
    match space {
        ProceduralSpace::Position => lang.t("fx-position"),
        ProceduralSpace::Uv => "UV",
        ProceduralSpace::Triplanar => lang.t("fx-triplanar"),
    }
}

pub const NOISE_BASES: [NoiseBasis; 3] =
    [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley];

pub fn noise_basis_name(lang: Lang, basis: NoiseBasis) -> &'static str {
    match basis {
        NoiseBasis::Value => lang.t("fx-value"),
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
        lang.t("fx-up-y").into()
    } else if d == [0.0, -1.0, 0.0] {
        lang.t("fx-down-y").into()
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

/// 形の名前（グラデーションデカールの形。塗りつぶしの欄の「形」・新規塗りつぶしレイヤーのメニュー・効果の欄が同じ名前を使う）。
/// 平面は後ろが 0・前が 1 の、1 方向のグラデーション（線形）。
pub fn shape_name(lang: Lang, shape: Shape) -> &'static str {
    match shape {
        Shape::Box => lang.t("fx-box"),
        Shape::Sphere => lang.t("fx-sphere"),
        Shape::Plane => lang.t("fx-plane-linear"),
    }
}

/// 形の短い説明（メニューの項目のツールチップ）。
pub fn shape_hint(lang: Lang, shape: Shape) -> &'static str {
    match shape {
        Shape::Box => lang.t("fx-1-inside-the-box-fading-tip"),
        Shape::Sphere => lang.t("fx-1-inside-the-sphere-fading-tip"),
        Shape::Plane => lang.t("fx-0-behind-the-plane-to-tip"),
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
        ReadMode::Value => lang.t("fx-value"),
        ReadMode::Coverage => lang.t("fx-coverage"),
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
        lang.f("fx-mask", &[("name", &anchor.name())])
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
        EffectSettings::Filter(
            Filter::Morphology { radius, .. }
            | Filter::HighPass { radius }
            | Filter::Median { radius }
            | Filter::Glow { radius, .. },
        ) => {
            text += &format!("  {radius} px");
        }
        EffectSettings::Filter(
            Filter::SlopeBlur { intensity, .. } | Filter::Warp { intensity, .. },
        ) => {
            text += &format!("  {} px", trim(*intensity, 1));
        }
        EffectSettings::Filter(Filter::DirectionalBlur { angle, distance }) => {
            text += &format!("  {}° {} px", trim(*angle, 1), trim(*distance, 1));
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

#[cfg(test)]
mod icon_tests {
    use super::*;
    use yolu_core::generator::{Kind, Settings};

    #[test]
    fn a_generator_row_has_the_generator_icon_and_a_filter_row_the_filter_icon() {
        let generator = EffectSettings::generator(Settings::new(Kind::Noise));
        assert_eq!(effect_icon(&generator), "generator");
        let filter = EffectSettings::Filter(Filter::Invert);
        assert_eq!(effect_icon(&filter), "auto_awesome");
    }
}
