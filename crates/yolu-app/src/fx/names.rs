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
            FilterKind::Blur => lang.pick("ぼかし（ガウス）", "Gaussian Blur"),
            FilterKind::Sharpen => {
                lang.pick("シャープ（アンシャープマスク）", "Sharpen (Unsharp Mask)")
            }
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
            FilterKind::HistogramScan => lang.pick("値の切り出し", "Histogram Scan"),
            FilterKind::HistogramRange => lang.pick("値の幅", "Histogram Range"),
            FilterKind::SlopeBlur => lang.pick("ノイズに沿ったぼかし", "Slope Blur"),
            FilterKind::DirectionalBlur => lang.pick("方向ぼかし", "Directional Blur"),
            FilterKind::Warp => lang.pick("ゆがみ", "Warp"),
            FilterKind::Morphology => lang.pick("太らせる・細らせる", "Dilate / Erode"),
            FilterKind::EdgeDetect => lang.pick("輪郭の検出", "Edge Detect"),
            FilterKind::HighPass => lang.pick("ハイパス", "High Pass"),
            FilterKind::Median => lang.pick("メディアン", "Median"),
            FilterKind::Glow => lang.pick("グロー", "Glow"),
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
        ("pattern", "shape") => lang.pick("形", "Shape"),
        ("pattern", "scale") => lang.pick("繰り返し", "Repeat"),
        ("pattern", "width") => lang.pick("太さ", "Width"),
        ("pattern", "softness") => lang.pick("ぼかし", "Blur"),
        ("pattern", "offset_u") => lang.pick("ずらす U", "Offset U"),
        ("pattern", "offset_v") => lang.pick("ずらす V", "Offset V"),
        ("light", "azimuth") => lang.pick("水平の角度", "Azimuth"),
        ("light", "elevation") => lang.pick("高さ", "Elevation"),
        ("light", "softness") => lang.pick("回り込み", "Wrap"),
        ("light", "ambient") => lang.pick("底上げ", "Ambient"),
        ("mask_builder", "combine") => lang.pick("合わせ方", "Combine"),
        ("mask_builder", p) if p.ends_with("_weight") => lang.pick("重み", "Weight"),
        ("mask_builder", p) if p.ends_with("_level") => lang.pick("位置", "Level"),
        ("mask_builder", p) if p.ends_with("_contrast") => lang.pick("コントラスト", "Contrast"),
        ("mask_builder", p) if p.ends_with("_invert") => lang.pick("反転", "Invert"),
        (_, "position") => lang.pick("位置", "Position"),
        (_, "contrast") => lang.pick("コントラスト", "Contrast"),
        (_, "range") => lang.pick("幅", "Range"),
        ("warp", "intensity") => lang.pick("ずらす量", "Amount"),
        ("glow", "intensity") => lang.pick("明るさ", "Intensity"),
        (_, "intensity") => lang.pick("長さ", "Length"),
        (_, "samples") => lang.pick("取る数", "Samples"),
        ("morphology", "mode") => lang.pick("向き", "Mode"),
        (_, "mode") => lang.pick("合わせ方", "Mode"),
        (_, "scale") => lang.pick("ノイズの大きさ", "Noise Size"),
        (_, "seed") => lang.pick("シード", "Seed"),
        (_, "angle") => lang.pick("角度", "Angle"),
        (_, "distance") => lang.pick("長さ", "Distance"),
        (_, "width") => lang.pick("ぼかす幅", "Width"),
        (_, "threshold") => lang.pick("しきい値", "Threshold"),
        _ => lang.pick("半径", "Radius"),
    }
}

/// 目録の値の欄の前に置く小さな見出し（マスクの組み立てのマップの名前。無ければ None）。
pub fn param_group(lang: Lang, kind: &str, param: &str) -> Option<&'static str> {
    if kind != "mask_builder" {
        return None;
    }
    Some(match param {
        "curvature_weight" => lang.pick("曲率", "Curvature"),
        "ambient_occlusion_weight" => lang.pick("AO", "Ambient Occlusion"),
        "position_weight" => lang.pick("位置の高さ", "Height (Position)"),
        "thickness_weight" => lang.pick("厚み", "Thickness"),
        _ => return None,
    })
}

/// 目録の値の欄のツールチップ（無ければ None）。
pub fn param_hint(lang: Lang, kind: &str, param: &str) -> Option<&'static str> {
    Some(match (kind, param) {
        ("pattern", "scale") => lang.pick(
            "UV の 0〜1 に繰り返す回数",
            "How many times the pattern repeats across UV 0–1",
        ),
        ("pattern", "width") => lang.pick(
            "縞・水玉・格子の太さ、縁の幅（繰り返しの 1 つに対する割合）",
            "Width of stripes, dots and grid lines, or of the border",
        ),
        ("pattern", "softness") => lang.pick("境目のぼかし", "Blur of the edges"),
        ("light", "azimuth") => lang.pick(
            "光の来る水平の向き（0° が +Z、90° が +X）",
            "Horizontal direction the light comes from (0° = +Z, 90° = +X)",
        ),
        ("light", "elevation") => lang.pick(
            "光の高さ（0° が水平、90° が真上）",
            "Height of the light (0° = horizon, 90° = straight above)",
        ),
        ("light", "softness") => lang.pick(
            "明暗の境を裏側へ回り込ませる量",
            "How far the light wraps around past the terminator",
        ),
        ("light", "ambient") => lang.pick("暗い所の明るさ", "Brightness of the dark side"),
        ("mask_builder", p) if p.ends_with("_weight") => lang.pick(
            "このマップをどれだけ使うか（0 % は読まない）",
            "How much this map counts (0% = not read)",
        ),
        ("mask_builder", p) if p.ends_with("_level") => lang.pick(
            "どの値から上を 1 へ寄せるか",
            "Where values start turning to 1",
        ),
        ("mask_builder", "combine") => {
            lang.pick("マップの値の合わせ方", "How the maps are combined")
        }
        ("histogram_scan", "position") => lang.pick(
            "どの値から上を 1 にするか",
            "Where values start turning to 1",
        ),
        ("histogram_scan", "contrast") => lang.pick(
            "境目の鋭さ（1 で 2 値）",
            "How sharp the cut is (1 = two values)",
        ),
        ("histogram_range", "range") => lang.pick(
            "値の広がり（0 で位置の値 1 つ）",
            "How far the values spread (0 = only the position)",
        ),
        ("histogram_range", "position") => lang.pick("真ん中の値", "The middle value"),
        ("slope_blur", "intensity") => lang.pick(
            "ノイズの坂の向きへ伸ばす長さ（画素）",
            "How far along the noise slope (pixels)",
        ),
        ("slope_blur", "mode") => lang.pick(
            "取った値の平均・最小・最大",
            "Average, minimum or maximum of the samples",
        ),
        ("slope_blur" | "warp", "scale") => lang.pick(
            "内蔵のノイズの 1 つの塊の大きさ（画素）",
            "Size of one blob of the built-in noise (pixels)",
        ),
        ("warp", "intensity") => lang.pick(
            "読む位置をずらす長さ（画素）",
            "How far the pixels are moved (pixels)",
        ),
        ("directional_blur", "distance") => lang.pick(
            "片側の長さ（画素。両側へぼかす）",
            "Length to each side (pixels)",
        ),
        ("morphology", "radius") => {
            lang.pick("丸い窓の半径（画素）", "Round window radius (pixels)")
        }
        ("edge_detect", "width") => lang.pick(
            "輪郭を探す前にぼかす幅（画素）",
            "Blur before finding edges (pixels)",
        ),
        ("edge_detect", "threshold") => {
            lang.pick("これ以下の弱い輪郭は 0", "Edges weaker than this become 0")
        }
        ("glow", "threshold") => lang.pick(
            "これより明るい所だけ光る",
            "Only parts brighter than this glow",
        ),
        ("median", "radius") => {
            lang.pick("正方形の窓の半径（画素）", "Square window radius (pixels)")
        }
        (_, "seed") => lang.pick(
            "同じシードなら同じノイズ",
            "The same seed gives the same noise.",
        ),
        _ => return None,
    })
}

/// 目録の選択肢の名前（0.5.0 のフィルター）。
pub fn option_label(lang: Lang, option: &str) -> &'static str {
    match option {
        "blur" => lang.pick("平均", "Average"),
        "min" => lang.pick("最小", "Min"),
        "max" => lang.pick("最大", "Max"),
        "dilate" => lang.pick("太らせる", "Dilate"),
        "erode" => lang.pick("細らせる", "Erode"),
        "stripes" => lang.pick("縞", "Stripes"),
        "checker" => lang.pick("市松", "Checker"),
        "dots" => lang.pick("水玉", "Dots"),
        "border" => lang.pick("縁", "Border"),
        "grid" => lang.pick("格子", "Grid"),
        "multiply" => lang.pick("乗算", "Multiply"),
        "add" => lang.pick("加算", "Add"),
        _ => "?",
    }
}

/// 足せる Generator の種類（メニューの並び: 焼いたマップを読む種類 → 読まない種類 → 画像）。
pub const GENERATOR_KINDS: [Kind; 14] = [
    Kind::EdgeWear,
    Kind::Dirt,
    Kind::PositionGradient,
    Kind::ShapeGradient,
    Kind::Thickness,
    Kind::Direction,
    Kind::Light,
    Kind::MaskBuilder,
    Kind::IdColor,
    Kind::Anchor,
    Kind::Noise,
    Kind::Grunge,
    Kind::Pattern,
    Kind::Image,
];

/// 画像の段の投影の種類（塗りつぶしの層の投影の、デカールを除いたもの。並びも同じ）。
pub const IMAGE_PROJECTIONS: [ProjectionMode; 5] = [
    ProjectionMode::Uv,
    ProjectionMode::Triplanar,
    ProjectionMode::Planar,
    ProjectionMode::Spherical,
    ProjectionMode::Cylindrical,
];

/// 画像の段の投影の外側（塗りつぶしの層と同じ並び）。
pub const IMAGE_WRAPS: [Wrap; 3] = [Wrap::Repeat, Wrap::Clamp, Wrap::None];

/// 画像の段がマスク・スカラーで値にする成分の名前。
pub fn image_component_name(lang: Lang, c: ImageComponent) -> &'static str {
    match c {
        ImageComponent::Red => lang.pick("R", "R"),
        ImageComponent::Green => lang.pick("G", "G"),
        ImageComponent::Blue => lang.pick("B", "B"),
        ImageComponent::Alpha => lang.pick("アルファ", "Alpha"),
        ImageComponent::Luminance => lang.pick("輝度", "Luminance"),
    }
}

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
        Kind::Image => lang.pick("画像", "Image"),
        Kind::Pattern => lang.pick("模様", "Pattern"),
        Kind::Light => lang.pick("光", "Light"),
        Kind::MaskBuilder => lang.pick("マスクの組み立て", "Mask Builder"),
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

/// 形の名前（グラデーションデカールの形。塗りつぶしの欄の「形」・新規塗りつぶしレイヤーのメニュー・効果の欄が同じ名前を使う）。
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
