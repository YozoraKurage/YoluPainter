//! 効果の種類と値の表（読むだけ）。外から名前と値（数・真偽・選択肢）で効果を作る・読む道（コマンド・設定の入れ口）が使う。
//!
//! - **表は検査と同じ定義から作る**: 値の範囲は [`crate::ranges`]（各 `validate` とコンストラクタが使うのと同じ const）、
//!   既定の値は新しく足すときの設定そのもの（`Settings::new`・画面の既定）を読み出したもの。範囲の検査は表の型と範囲で 1 欄ずつ行い
//!   （どの欄がなぜ断られたかを返す）、欄の組み合わせの条件（レベル補正の入力の幅など）は組んだ設定の `validate` が断る。
//! - **足せる種類（`addable`）は、値だけで組める種類**。リスト・曲線・参照を持つ種類（グラデーションマップのランプ・トーンカーブ・
//!   形のグラデーション・ID の色・Anchor）は欄を持たず、足せない。すでにある段は読めて（`opaque_parts` にその部分を挙げる）、
//!   強さ・有効・適用するチャンネルは変えられ、値の変更は断る（その部分を黙って作り直さない）。
//! - 欄の名前と選択肢の綴りは外へ出す名前（保存形式ではない）。変えるときは版の扱いに気を付ける。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::adjust::{
    AdjustmentSettings, AdjustmentType, BalanceRange, BrightnessContrast, ColorAdjust,
    ColorBalance, Posterize, Threshold,
};
use crate::effects::EffectSettings;
use crate::error::CoreError;
use crate::filter;
use crate::generator::{
    self, Blend, CellOutput, FractalMode, GrungePreset, NoiseBasis, NoiseSpace, ProceduralSpace,
};
use crate::ranges;

/// 欄の型と範囲。
#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    /// 整数（両端を含む）。
    Integer {
        min: i64,
        max: i64,
    },
    /// 実数（両端を含む）。
    Number {
        min: f64,
        max: f64,
    },
    Bool,
    /// 選択肢のどれか（綴りは小文字の英数字と `_`）。
    Choice {
        options: Vec<&'static str>,
    },
}

/// 欄の値。整数も `Number`（整数かどうかは型が見る）。
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Number(f64),
    Bool(bool),
    Choice(String),
}

/// 1 つの欄。
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub ty: ParamType,
    /// 新しく足すときの値。
    pub default: ParamValue,
}

/// 効果の種類 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct EffectKind {
    pub id: &'static str,
    /// フィルターのスタックの段にできる（フィルターと Generator）。
    pub stack: bool,
    /// 調整の層の設定にできる。
    pub adjustment: bool,
    /// Generator（フィルターのスタックの Generator の段）。
    pub generator: bool,
    /// 焼いたメッシュマップ・モデルを読む Generator（マップ・モデルの入力が無ければ入力のまま通す）。手続き型のノイズ・グランジは、
    /// 位置のマップが無ければ UV で評価するので、これは偽。
    pub needs_maps: bool,
    /// 値だけで足せる。足せない種類は `params` が空。
    pub addable: bool,
    /// Rust 版だけの種類（保存形式の種類の番号が 64 から、または手続き型の Generator）。使う文書は新しい版で保存され、Unity 版（0.2.0）は開けない。
    pub rust_only: bool,
    pub params: Vec<Param>,
    /// 値の欄では変えられない中身（すでにある段に残る部分）。
    pub opaque: &'static [&'static str],
}

impl EffectKind {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// 値から効果を組む・読む道の失敗。
#[derive(Clone, Debug, PartialEq)]
pub enum ParamError {
    UnknownKind(String),
    /// その使い方（フィルターの段・調整の層）にできない種類。
    WrongTarget {
        kind: &'static str,
        adjustment: bool,
    },
    /// 値だけでは足せない・値を変えられない種類（リスト・曲線・参照を持つ）。
    NotEditable {
        kind: &'static str,
    },
    UnknownParam {
        kind: &'static str,
        name: String,
    },
    WrongType {
        name: &'static str,
        expected: &'static str,
    },
    OutOfRange {
        name: &'static str,
        min: f64,
        max: f64,
    },
    NotInteger {
        name: &'static str,
    },
    UnknownOption {
        name: &'static str,
        options: Vec<&'static str>,
    },
    /// 欄の組み合わせが、組んだ設定の検査で断られた。
    Refused(CoreError),
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKind(k) => write!(f, "知らない効果の種類: {k}"),
            Self::WrongTarget { kind, adjustment } => write!(
                f,
                "{kind} は{}にできない",
                if *adjustment {
                    "調整の層の設定"
                } else {
                    "フィルターの段"
                }
            ),
            Self::NotEditable { kind } => write!(f, "{kind} は値の欄では足せない・変えられない"),
            Self::UnknownParam { kind, name } => write!(f, "{kind} に欄 {name} は無い"),
            Self::WrongType { name, expected } => write!(f, "{name} は{expected}"),
            Self::OutOfRange { name, min, max } => write!(f, "{name} は {min}〜{max}"),
            Self::NotInteger { name } => write!(f, "{name} は整数"),
            Self::UnknownOption { name, options } => {
                write!(f, "{name} は {} のどれか", options.join(" / "))
            }
            Self::Refused(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for ParamError {}

type Bag = BTreeMap<&'static str, ParamValue>;

// ───────── 欄の定義 ─────────

fn int<T: Into<i64> + Copy>(r: &std::ops::RangeInclusive<T>) -> ParamType {
    ParamType::Integer {
        min: (*r.start()).into(),
        max: (*r.end()).into(),
    }
}
fn num(r: &std::ops::RangeInclusive<f64>) -> ParamType {
    ParamType::Number {
        min: *r.start(),
        max: *r.end(),
    }
}
fn choice(options: &[&'static str]) -> ParamType {
    ParamType::Choice {
        options: options.to_vec(),
    }
}

const BLEND_OPTIONS: [&str; 7] = [
    "multiply", "replace", "screen", "max", "min", "add", "subtract",
];
const NOISE_SPACE_OPTIONS: [&str; 2] = ["model", "uv"];
const AXIS_OPTIONS: [&str; 3] = ["x", "y", "z"];
const SPACE_OPTIONS: [&str; 3] = ["position", "triplanar", "uv"];
const BASIS_OPTIONS: [&str; 3] = ["value", "perlin", "worley"];
const CELL_OPTIONS: [&str; 3] = ["f1", "f2", "f2_minus_f1"];
const FRACTAL_OPTIONS: [&str; 3] = ["fbm", "ridged", "turbulence"];
const SLOPE_MODE_OPTIONS: [&str; 3] = ["blur", "min", "max"];
const PATTERN_SHAPE_OPTIONS: [&str; 5] = ["stripes", "checker", "dots", "border", "grid"];
const MASK_COMBINE_OPTIONS: [&str; 3] = ["multiply", "max", "add"];
/// マスクの組み立てのマップ（欄の名前の頭。`generator::MaskBuilder::MAPS` と同じ並び）。
const MASK_MAPS: [&str; 4] = ["curvature", "ambient_occlusion", "position", "thickness"];
/// マスクの組み立ての 1 つのマップの欄（`{マップ}_weight` など）。
const MASK_FIELDS: [&str; 4] = ["weight", "level", "contrast", "invert"];
const MORPHOLOGY_MODE_OPTIONS: [&str; 2] = ["dilate", "erode"];

fn preset_options() -> Vec<&'static str> {
    GrungePreset::ALL.iter().map(|p| p.id()).collect()
}

/// `(i32::MIN, i32::MAX)` の整数（シード）。
fn seed_type() -> ParamType {
    ParamType::Integer {
        min: i64::from(i32::MIN),
        max: i64::from(i32::MAX),
    }
}

/// 色調補正の欄の名前（カラーバランス）。
const BALANCE_NAMES: [&str; 9] = [
    "shadows_cyan_red",
    "shadows_magenta_green",
    "shadows_yellow_blue",
    "midtones_cyan_red",
    "midtones_magenta_green",
    "midtones_yellow_blue",
    "highlights_cyan_red",
    "highlights_magenta_green",
    "highlights_yellow_blue",
];

fn generator_common() -> Vec<(&'static str, ParamType)> {
    vec![
        ("low", num(&ranges::UNIT)),
        ("high", num(&ranges::UNIT)),
        ("softness", num(&ranges::UNIT)),
        ("invert", ParamType::Bool),
        ("blend", choice(&BLEND_OPTIONS)),
    ]
}
fn generator_overlay_noise() -> Vec<(&'static str, ParamType)> {
    vec![
        ("noise_amount", num(&ranges::UNIT)),
        ("noise_scale", num(&ranges::NOISE_SCALE)),
        ("noise_seed", seed_type()),
        ("noise_space", choice(&NOISE_SPACE_OPTIONS)),
    ]
}
fn procedural_common() -> Vec<(&'static str, ParamType)> {
    vec![
        ("space", choice(&SPACE_OPTIONS)),
        ("scale", num(&ranges::NOISE_SCALE)),
        ("seed", seed_type()),
        ("rotation_x", num(&ranges::PROCEDURAL_ROTATION)),
        ("rotation_y", num(&ranges::PROCEDURAL_ROTATION)),
        ("rotation_z", num(&ranges::PROCEDURAL_ROTATION)),
        ("bleed", num(&ranges::UNIT)),
        ("blend_width", num(&ranges::UNIT)),
    ]
}

/// 種類ごとの欄の名前と型（足せない種類は空）。
fn param_types(id: &str) -> Vec<(&'static str, ParamType)> {
    let mut v: Vec<(&'static str, ParamType)> = Vec::new();
    match id {
        "blur" => v.push(("radius", int(&ranges::BLUR_RADIUS))),
        "sharpen" => v.extend([
            ("radius", int(&ranges::SHARPEN_RADIUS)),
            ("amount", num(&ranges::SHARPEN_AMOUNT)),
            ("threshold", int(&ranges::SHARPEN_THRESHOLD)),
        ]),
        "noise" => v.extend([
            ("amount", num(&ranges::NOISE_AMOUNT)),
            ("seed", seed_type()),
            ("monochrome", ParamType::Bool),
        ]),
        "levels" => v.extend([
            ("input_black", num(&ranges::LEVELS_UNIT)),
            ("input_white", num(&ranges::LEVELS_UNIT)),
            ("gamma", num(&ranges::GAMMA)),
            ("output_black", num(&ranges::LEVELS_UNIT)),
            ("output_white", num(&ranges::LEVELS_UNIT)),
        ]),
        "hue_saturation" => v.extend([
            ("hue", num(&ranges::HUE)),
            ("saturation", num(&ranges::SATURATION)),
            ("lightness", num(&ranges::SATURATION)),
        ]),
        "color_balance" => {
            let range = ColorBalance::RANGE;
            for name in BALANCE_NAMES {
                v.push((
                    name,
                    ParamType::Number {
                        min: -range,
                        max: range,
                    },
                ));
            }
            v.push(("preserve_luminosity", ParamType::Bool));
        }
        "brightness_contrast" => v.extend([
            ("brightness", num(&BrightnessContrast::BRIGHTNESS)),
            ("contrast", num(&BrightnessContrast::CONTRAST)),
        ]),
        "threshold" => v.push(("level", int(&ranges::THRESHOLD_LEVEL))),
        "posterize" => v.push(("levels", int(&ranges::POSTERIZE_LEVELS))),
        "edge_wear" | "thickness" => {
            v.extend(generator_common());
            v.extend(generator_overlay_noise());
        }
        "dirt" => {
            v.extend(generator_common());
            v.extend(generator_overlay_noise());
            v.push(("balance", num(&ranges::UNIT)));
        }
        "position_gradient" => {
            v.extend(generator_common());
            v.extend(generator_overlay_noise());
            v.push(("axis", choice(&AXIS_OPTIONS)));
        }
        "direction" => {
            v.extend(generator_common());
            v.extend(generator_overlay_noise());
            v.extend([
                ("direction_x", num(&ranges::DIRECTION_COMPONENT)),
                ("direction_y", num(&ranges::DIRECTION_COMPONENT)),
                ("direction_z", num(&ranges::DIRECTION_COMPONENT)),
                ("use_bent_normal", ParamType::Bool),
            ]);
        }
        "procedural_noise" => {
            v.extend(generator_common());
            v.extend(procedural_common());
            v.extend([
                ("basis", choice(&BASIS_OPTIONS)),
                ("cell_output", choice(&CELL_OPTIONS)),
                ("fractal", choice(&FRACTAL_OPTIONS)),
                (
                    "octaves",
                    ParamType::Integer {
                        min: 1,
                        max: i64::from(generator::MAX_OCTAVES),
                    },
                ),
                ("lacunarity", num(&ranges::LACUNARITY)),
                ("gain", num(&ranges::UNIT)),
            ]);
        }
        "grunge" => {
            v.extend(generator_common());
            v.extend(procedural_common());
            v.push((
                "preset",
                ParamType::Choice {
                    options: preset_options(),
                },
            ));
        }
        "histogram_scan" => v.extend([
            ("position", num(&ranges::UNIT)),
            ("contrast", num(&ranges::UNIT)),
        ]),
        "histogram_range" => v.extend([
            ("range", num(&ranges::UNIT)),
            ("position", num(&ranges::UNIT)),
        ]),
        "slope_blur" => v.extend([
            ("intensity", num(&ranges::SLOPE_INTENSITY)),
            ("samples", int(&ranges::SLOPE_SAMPLES)),
            ("mode", choice(&SLOPE_MODE_OPTIONS)),
            ("scale", num(&ranges::FILTER_NOISE_SCALE)),
            ("seed", seed_type()),
        ]),
        "directional_blur" => v.extend([
            ("angle", num(&ranges::DIRECTIONAL_ANGLE)),
            ("distance", num(&ranges::DIRECTIONAL_DISTANCE)),
        ]),
        "warp" => v.extend([
            ("intensity", num(&ranges::WARP_INTENSITY)),
            ("scale", num(&ranges::FILTER_NOISE_SCALE)),
            ("seed", seed_type()),
        ]),
        "morphology" => v.extend([
            ("mode", choice(&MORPHOLOGY_MODE_OPTIONS)),
            ("radius", int(&ranges::MORPHOLOGY_RADIUS)),
        ]),
        "edge_detect" => v.extend([
            ("width", int(&ranges::EDGE_WIDTH)),
            ("threshold", num(&ranges::UNIT)),
        ]),
        "high_pass" => v.push(("radius", int(&ranges::HIGH_PASS_RADIUS))),
        "median" => v.push(("radius", int(&ranges::MEDIAN_RADIUS))),
        "glow" => v.extend([
            ("threshold", num(&ranges::UNIT)),
            ("radius", int(&ranges::GLOW_RADIUS)),
            ("intensity", num(&ranges::GLOW_INTENSITY)),
        ]),
        // 模様・光は境目のぼかしを自分の欄（softness）で持つので、共通の減衰を置かない
        "pattern" => {
            v.extend(
                generator_common()
                    .into_iter()
                    .filter(|(n, _)| *n != "softness"),
            );
            v.extend([
                ("shape", choice(&PATTERN_SHAPE_OPTIONS)),
                ("scale", num(&ranges::PATTERN_SCALE)),
                ("angle", num(&ranges::TURN_DEGREES)),
                ("width", num(&ranges::UNIT)),
                ("softness", num(&ranges::UNIT)),
                ("offset_u", num(&ranges::UNIT)),
                ("offset_v", num(&ranges::UNIT)),
            ]);
        }
        "light" => {
            v.extend(
                generator_common()
                    .into_iter()
                    .filter(|(n, _)| *n != "softness"),
            );
            v.extend([
                ("azimuth", num(&ranges::TURN_DEGREES)),
                ("elevation", num(&ranges::LIGHT_ELEVATION)),
                ("softness", num(&ranges::UNIT)),
                ("ambient", num(&ranges::UNIT)),
            ]);
        }
        "mask_builder" => {
            v.extend(generator_common());
            for map in MASK_MAPS {
                for field in MASK_FIELDS {
                    v.push((
                        mask_param(map, field),
                        if field == "invert" {
                            ParamType::Bool
                        } else {
                            num(&ranges::UNIT)
                        },
                    ));
                }
            }
            v.push(("combine", choice(&MASK_COMBINE_OPTIONS)));
        }
        _ => {}
    }
    v
}

/// マスクの組み立ての欄の名前（`curvature_weight` など。表に載る名前なので 'static）。
fn mask_param(map: &str, field: &str) -> &'static str {
    const NAMES: [[&str; 4]; 4] = [
        [
            "curvature_weight",
            "curvature_level",
            "curvature_contrast",
            "curvature_invert",
        ],
        [
            "ambient_occlusion_weight",
            "ambient_occlusion_level",
            "ambient_occlusion_contrast",
            "ambient_occlusion_invert",
        ],
        [
            "position_weight",
            "position_level",
            "position_contrast",
            "position_invert",
        ],
        [
            "thickness_weight",
            "thickness_level",
            "thickness_contrast",
            "thickness_invert",
        ],
    ];
    let m = MASK_MAPS
        .iter()
        .position(|m| *m == map)
        .expect("マップの名前");
    let f = MASK_FIELDS
        .iter()
        .position(|f| *f == field)
        .expect("欄の名前");
    NAMES[m][f]
}

/// 一覧の 1 行の元: (id, フィルターのスタック, 調整の層, Generator, 足せる, 値の欄で変えられない中身)。
type Row = (
    &'static str,
    bool,
    bool,
    bool,
    bool,
    &'static [&'static str],
);
const KIND_ROWS: [Row; 37] = [
    ("blur", true, false, false, true, &[]),
    ("sharpen", true, false, false, true, &[]),
    ("noise", true, false, false, true, &[]),
    ("levels", true, true, false, true, &[]),
    ("invert", true, true, false, true, &[]),
    ("normalize", true, false, false, true, &[]),
    ("hue_saturation", false, true, false, true, &[]),
    ("gradient_map", true, true, false, false, &["ramp"]),
    ("tone_curve", true, true, false, false, &["curves"]),
    ("color_balance", true, true, false, true, &[]),
    ("brightness_contrast", true, true, false, true, &[]),
    ("threshold", true, true, false, true, &[]),
    ("posterize", true, true, false, true, &[]),
    ("edge_wear", true, false, true, true, &["pins"]),
    ("dirt", true, false, true, true, &["pins"]),
    ("position_gradient", true, false, true, true, &["pins"]),
    ("thickness", true, false, true, true, &["pins"]),
    ("direction", true, false, true, true, &["pins"]),
    ("procedural_noise", true, false, true, true, &["pins"]),
    ("grunge", true, false, true, true, &["pins"]),
    (
        "shape_gradient",
        true,
        false,
        true,
        false,
        &["volume", "ramp", "pins"],
    ),
    ("id_color", true, false, true, false, &["id_colors", "pins"]),
    ("anchor", true, false, true, false, &["anchor"]),
    ("histogram_scan", true, false, false, true, &[]),
    ("histogram_range", true, false, false, true, &[]),
    ("slope_blur", true, false, false, true, &[]),
    ("directional_blur", true, false, false, true, &[]),
    ("warp", true, false, false, true, &[]),
    ("morphology", true, false, false, true, &[]),
    ("edge_detect", true, false, false, true, &[]),
    ("high_pass", true, false, false, true, &[]),
    ("median", true, false, false, true, &[]),
    ("glow", true, false, false, true, &[]),
    ("pattern", true, false, true, true, &["pins"]),
    ("light", true, false, true, true, &["pins"]),
    ("mask_builder", true, false, true, true, &["pins"]),
    (
        "image",
        true,
        false,
        true,
        false,
        &["image", "projection", "component"],
    ),
];

/// 効果の種類の一覧（フィルター・調整・Generator。足せない種類も含む）。
pub fn kinds() -> &'static [EffectKind] {
    static KINDS: OnceLock<Vec<EffectKind>> = OnceLock::new();
    KINDS.get_or_init(|| {
        KIND_ROWS
            .iter()
            .map(|&(id, stack, adjustment, generator, addable, opaque)| {
                let defaults = default_bag(id);
                let params = param_types(id)
                    .into_iter()
                    .map(|(name, ty)| Param {
                        name,
                        default: defaults
                            .get(name)
                            .cloned()
                            .expect("既定の値は全部の欄にある（表の試験が確かめる）"),
                        ty,
                    })
                    .collect();
                // 画像の段は投影しだい（UV は読まない）なので、ここでは偽
                let needs_maps =
                    generator && !matches!(id, "procedural_noise" | "grunge" | "pattern" | "image");
                EffectKind {
                    id,
                    stack,
                    adjustment,
                    generator,
                    needs_maps,
                    addable,
                    rust_only: rust_only(id),
                    params,
                    opaque,
                }
            })
            .collect()
    })
}

/// Rust 版だけの種類か（保存形式の種類の番号・Generator の種類で決める）。
fn rust_only(id: &str) -> bool {
    let stack = default_stack(id).or_else(|| match id {
        "gradient_map" => ColorAdjust::default_for(AdjustmentType::GradientMap)
            .map(EffectSettings::from_color_adjust),
        "tone_curve" => ColorAdjust::default_for(AdjustmentType::ToneCurve)
            .map(EffectSettings::from_color_adjust),
        _ => None,
    });
    match stack {
        Some(s) => {
            s.type_index() >= 64
                || s.generator_settings()
                    .is_some_and(|g| g.kind.is_rust_only())
        }
        None => default_adjustment(id).is_some_and(|a| a.kind().is_rust_only()),
    }
}

pub fn kind(id: &str) -> Option<&'static EffectKind> {
    kinds().iter().find(|k| k.id == id)
}

// ───────── 既定の設定 ─────────

fn default_stack(id: &str) -> Option<EffectSettings> {
    use generator::Kind as G;
    Some(match id {
        "blur" => EffectSettings::blur(4),
        "sharpen" => EffectSettings::sharpen(2, 1.0, 0),
        "noise" => EffectSettings::noise(0.25, 0, true),
        "levels" => EffectSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0),
        "invert" => EffectSettings::invert(),
        "normalize" => EffectSettings::normalize(),
        "color_balance" => EffectSettings::color_balance(ColorBalance::neutral()),
        "brightness_contrast" => EffectSettings::from_color_adjust(ColorAdjust::default_for(
            AdjustmentType::BrightnessContrast,
        )?),
        "threshold" => {
            EffectSettings::from_color_adjust(ColorAdjust::default_for(AdjustmentType::Threshold)?)
        }
        "posterize" => {
            EffectSettings::from_color_adjust(ColorAdjust::default_for(AdjustmentType::Posterize)?)
        }
        "edge_wear" => EffectSettings::generator(generator::Settings::new(G::EdgeWear)),
        "dirt" => EffectSettings::generator(generator::Settings::new(G::Dirt)),
        "position_gradient" => {
            EffectSettings::generator(generator::Settings::new(G::PositionGradient))
        }
        "thickness" => EffectSettings::generator(generator::Settings::new(G::Thickness)),
        "direction" => EffectSettings::generator(generator::Settings::new(G::Direction)),
        "procedural_noise" => EffectSettings::generator(generator::Settings::new(G::Noise)),
        "grunge" => EffectSettings::generator(generator::Settings::new(G::Grunge)),
        "pattern" => EffectSettings::generator(generator::Settings::new(G::Pattern)),
        "light" => EffectSettings::generator(generator::Settings::new(G::Light)),
        "mask_builder" => EffectSettings::generator(generator::Settings::new(G::MaskBuilder)),
        // 0.5.0 の 10 種の既定（アプリのメニューで足すときもこの値）
        "histogram_scan" => EffectSettings::Filter(filter::Settings::HistogramScan {
            position: 0.5,
            contrast: 0.0,
        }),
        "histogram_range" => EffectSettings::Filter(filter::Settings::HistogramRange {
            range: 0.5,
            position: 0.5,
        }),
        "slope_blur" => EffectSettings::Filter(filter::Settings::SlopeBlur {
            intensity: 8.0,
            samples: 8,
            mode: filter::SlopeMode::Blur,
            scale: 16.0,
            seed: 0,
        }),
        "directional_blur" => EffectSettings::Filter(filter::Settings::DirectionalBlur {
            angle: 0.0,
            distance: 8.0,
        }),
        "warp" => EffectSettings::Filter(filter::Settings::Warp {
            intensity: 16.0,
            scale: 32.0,
            seed: 0,
        }),
        "morphology" => EffectSettings::Filter(filter::Settings::Morphology {
            mode: filter::MorphologyMode::Dilate,
            radius: 2,
        }),
        "edge_detect" => EffectSettings::Filter(filter::Settings::EdgeDetect {
            width: 1,
            threshold: 0.1,
        }),
        "high_pass" => EffectSettings::Filter(filter::Settings::HighPass { radius: 8 }),
        "median" => EffectSettings::Filter(filter::Settings::Median { radius: 1 }),
        "glow" => EffectSettings::Filter(filter::Settings::Glow {
            threshold: 0.7,
            radius: 16,
            intensity: 1.0,
        }),
        "image" => EffectSettings::generator(generator::Settings::new(G::Image)),
        _ => return None,
    })
}

fn default_adjustment(id: &str) -> Option<AdjustmentSettings> {
    Some(match id {
        "invert" => AdjustmentSettings::invert(),
        "levels" => AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).ok()?,
        "hue_saturation" => AdjustmentSettings::hue_saturation(0.0, 0.0, 0.0).ok()?,
        "color_balance" => AdjustmentSettings::color_balance(ColorBalance::neutral()),
        "brightness_contrast" => {
            ColorAdjust::default_for(AdjustmentType::BrightnessContrast)?.into_settings()
        }
        "threshold" => ColorAdjust::default_for(AdjustmentType::Threshold)?.into_settings(),
        "posterize" => ColorAdjust::default_for(AdjustmentType::Posterize)?.into_settings(),
        _ => return None,
    })
}

/// 種類の既定の値（足せない種類は空）。
fn default_bag(id: &str) -> Bag {
    if let Some(s) = default_stack(id) {
        return read_stack(&s).map(|(_, bag)| bag).unwrap_or_default();
    }
    if let Some(a) = default_adjustment(id) {
        return read_adjustment(&a).1;
    }
    Bag::new()
}

// ───────── 読む ─────────

fn n(v: f64) -> ParamValue {
    ParamValue::Number(v)
}
fn c(v: &str) -> ParamValue {
    ParamValue::Choice(v.to_owned())
}

fn blend_id(b: Blend) -> &'static str {
    match b {
        Blend::Multiply => "multiply",
        Blend::Replace => "replace",
        Blend::Screen => "screen",
        Blend::Max => "max",
        Blend::Min => "min",
        Blend::Add => "add",
        Blend::Subtract => "subtract",
    }
}
fn blend_from(id: &str) -> Blend {
    match id {
        "replace" => Blend::Replace,
        "screen" => Blend::Screen,
        "max" => Blend::Max,
        "min" => Blend::Min,
        "add" => Blend::Add,
        "subtract" => Blend::Subtract,
        _ => Blend::Multiply,
    }
}

fn generator_kind_id(k: generator::Kind) -> &'static str {
    use generator::Kind as G;
    match k {
        G::EdgeWear => "edge_wear",
        G::Dirt => "dirt",
        G::PositionGradient => "position_gradient",
        G::Thickness => "thickness",
        G::Direction => "direction",
        G::ShapeGradient => "shape_gradient",
        G::IdColor => "id_color",
        G::Anchor => "anchor",
        G::Noise => "procedural_noise",
        G::Grunge => "grunge",
        G::Pattern => "pattern",
        G::Light => "light",
        G::MaskBuilder => "mask_builder",
        G::Image => "image",
    }
}

fn read_generator(g: &generator::Settings) -> Bag {
    use generator::Kind as G;
    let mut b = Bag::new();
    if !matches!(
        g.kind,
        G::EdgeWear
            | G::Dirt
            | G::PositionGradient
            | G::Thickness
            | G::Direction
            | G::Noise
            | G::Grunge
            | G::Pattern
            | G::Light
            | G::MaskBuilder
    ) {
        return b;
    }
    b.insert("low", n(g.low));
    b.insert("high", n(g.high));
    b.insert("softness", n(g.softness));
    b.insert("invert", ParamValue::Bool(g.invert));
    b.insert("blend", c(blend_id(g.blend)));
    match g.kind {
        G::Pattern => {
            let p = &g.pattern;
            b.insert("shape", c(PATTERN_SHAPE_OPTIONS[p.shape as usize]));
            b.insert("scale", n(p.scale));
            b.insert("angle", n(p.angle));
            b.insert("width", n(p.width));
            b.insert("softness", n(p.softness));
            b.insert("offset_u", n(p.offset[0]));
            b.insert("offset_v", n(p.offset[1]));
            return b;
        }
        G::Light => {
            let l = &g.light;
            b.insert("azimuth", n(l.azimuth));
            b.insert("elevation", n(l.elevation));
            b.insert("softness", n(l.softness));
            b.insert("ambient", n(l.ambient));
            return b;
        }
        G::MaskBuilder => {
            let m = &g.mask_builder;
            for (map, input) in MASK_MAPS.iter().zip(&m.inputs) {
                b.insert(mask_param(map, "weight"), n(input.weight));
                b.insert(mask_param(map, "level"), n(input.level));
                b.insert(mask_param(map, "contrast"), n(input.contrast));
                b.insert(mask_param(map, "invert"), ParamValue::Bool(input.invert));
            }
            b.insert("combine", c(MASK_COMBINE_OPTIONS[m.combine as usize]));
            return b;
        }
        _ => {}
    }
    if !g.kind.is_procedural() {
        b.insert("noise_amount", n(g.noise_amount));
        b.insert("noise_scale", n(g.noise_scale));
        b.insert("noise_seed", n(f64::from(g.noise_seed)));
        b.insert(
            "noise_space",
            c(match g.noise_space {
                NoiseSpace::Model => "model",
                NoiseSpace::Uv => "uv",
            }),
        );
    }
    match g.kind {
        G::Dirt => {
            b.insert("balance", n(g.balance));
        }
        G::PositionGradient => {
            b.insert("axis", c(AXIS_OPTIONS.get(g.axis).copied().unwrap_or("y")));
        }
        G::Direction => {
            b.insert("direction_x", n(g.direction[0]));
            b.insert("direction_y", n(g.direction[1]));
            b.insert("direction_z", n(g.direction[2]));
            b.insert("use_bent_normal", ParamValue::Bool(g.use_bent_normal));
        }
        _ => {}
    }
    if g.kind.is_procedural() {
        let p = &g.procedural;
        b.insert(
            "space",
            c(match p.space {
                ProceduralSpace::Position => "position",
                ProceduralSpace::Triplanar => "triplanar",
                ProceduralSpace::Uv => "uv",
            }),
        );
        b.insert("scale", n(p.scale));
        b.insert("seed", n(f64::from(p.seed)));
        b.insert("rotation_x", n(p.rotation[0]));
        b.insert("rotation_y", n(p.rotation[1]));
        b.insert("rotation_z", n(p.rotation[2]));
        b.insert("bleed", n(p.bleed));
        b.insert("blend_width", n(p.blend_width));
        if g.kind == G::Noise {
            b.insert(
                "basis",
                c(match p.basis {
                    NoiseBasis::Value => "value",
                    NoiseBasis::Perlin => "perlin",
                    NoiseBasis::Worley => "worley",
                }),
            );
            b.insert(
                "cell_output",
                c(match p.cell_output {
                    CellOutput::F1 => "f1",
                    CellOutput::F2 => "f2",
                    CellOutput::F2MinusF1 => "f2_minus_f1",
                }),
            );
            b.insert(
                "fractal",
                c(match p.fractal {
                    FractalMode::Fbm => "fbm",
                    FractalMode::Ridged => "ridged",
                    FractalMode::Turbulence => "turbulence",
                }),
            );
            b.insert("octaves", n(f64::from(p.octaves)));
            b.insert("lacunarity", n(p.lacunarity));
            b.insert("gain", n(p.gain));
        } else {
            b.insert("preset", c(p.preset.id()));
        }
    }
    b
}

fn read_color_adjust(a: &ColorAdjust) -> Bag {
    let mut b = Bag::new();
    match a {
        ColorAdjust::ColorBalance(cb) => {
            for (i, range) in BalanceRange::ALL.iter().enumerate() {
                let v = cb.values(*range);
                for (k, value) in v.iter().enumerate() {
                    b.insert(BALANCE_NAMES[i * 3 + k], n(*value));
                }
            }
            b.insert(
                "preserve_luminosity",
                ParamValue::Bool(cb.preserve_luminosity()),
            );
        }
        ColorAdjust::BrightnessContrast(v) => {
            b.insert("brightness", n(v.brightness()));
            b.insert("contrast", n(v.contrast()));
        }
        ColorAdjust::Threshold(v) => {
            b.insert("level", n(f64::from(v.level())));
        }
        ColorAdjust::Posterize(v) => {
            b.insert("levels", n(f64::from(v.levels())));
        }
        ColorAdjust::GradientMap(_) | ColorAdjust::ToneCurve(_) => {}
    }
    b
}

fn color_adjust_id(a: &ColorAdjust) -> &'static str {
    match a {
        ColorAdjust::GradientMap(_) => "gradient_map",
        ColorAdjust::ToneCurve(_) => "tone_curve",
        ColorAdjust::ColorBalance(_) => "color_balance",
        ColorAdjust::BrightnessContrast(_) => "brightness_contrast",
        ColorAdjust::Threshold(_) => "threshold",
        ColorAdjust::Posterize(_) => "posterize",
    }
}

/// フィルターのスタックの段の、種類と値。
fn read_stack(s: &EffectSettings) -> Option<(&'static str, Bag)> {
    use filter::Settings as F;
    if let Some(a) = s.color_adjust() {
        return Some((color_adjust_id(&a), read_color_adjust(&a)));
    }
    let mut b = Bag::new();
    let id = match s {
        EffectSettings::Generator(g) => {
            return Some((generator_kind_id(g.kind), read_generator(g)));
        }
        EffectSettings::Filter(f) => match f {
            F::GaussianBlur { radius } => {
                b.insert("radius", n(f64::from(*radius)));
                "blur"
            }
            F::Sharpen {
                radius,
                amount,
                threshold,
            } => {
                b.insert("radius", n(f64::from(*radius)));
                b.insert("amount", n(*amount));
                b.insert("threshold", n(f64::from(*threshold)));
                "sharpen"
            }
            F::Noise {
                amount,
                seed,
                monochrome,
            } => {
                b.insert("amount", n(*amount));
                b.insert("seed", n(f64::from(*seed)));
                b.insert("monochrome", ParamValue::Bool(*monochrome));
                "noise"
            }
            F::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
            } => {
                b.insert("input_black", n(*input_black));
                b.insert("input_white", n(*input_white));
                b.insert("gamma", n(*gamma));
                b.insert("output_black", n(*output_black));
                b.insert("output_white", n(*output_white));
                "levels"
            }
            F::Invert => "invert",
            F::Normalize => "normalize",
            // 評価器の中の段は文書では Generator として持つ（ここへは来ない）
            F::Generator { .. } => return None,
            // 色調補正は上で読んだ
            F::GradientMap(_)
            | F::ToneCurve(_)
            | F::ColorBalance(_)
            | F::BrightnessContrast(_)
            | F::Threshold(_)
            | F::Posterize(_) => return None,
            F::HistogramScan { position, contrast } => {
                b.insert("position", n(*position));
                b.insert("contrast", n(*contrast));
                "histogram_scan"
            }
            F::HistogramRange { range, position } => {
                b.insert("range", n(*range));
                b.insert("position", n(*position));
                "histogram_range"
            }
            F::SlopeBlur {
                intensity,
                samples,
                mode,
                scale,
                seed,
            } => {
                b.insert("intensity", n(*intensity));
                b.insert("samples", n(f64::from(*samples)));
                b.insert(
                    "mode",
                    c(match mode {
                        filter::SlopeMode::Blur => "blur",
                        filter::SlopeMode::Min => "min",
                        filter::SlopeMode::Max => "max",
                    }),
                );
                b.insert("scale", n(*scale));
                b.insert("seed", n(f64::from(*seed)));
                "slope_blur"
            }
            F::DirectionalBlur { angle, distance } => {
                b.insert("angle", n(*angle));
                b.insert("distance", n(*distance));
                "directional_blur"
            }
            F::Warp {
                intensity,
                scale,
                seed,
            } => {
                b.insert("intensity", n(*intensity));
                b.insert("scale", n(*scale));
                b.insert("seed", n(f64::from(*seed)));
                "warp"
            }
            F::Morphology { mode, radius } => {
                b.insert(
                    "mode",
                    c(match mode {
                        filter::MorphologyMode::Dilate => "dilate",
                        filter::MorphologyMode::Erode => "erode",
                    }),
                );
                b.insert("radius", n(f64::from(*radius)));
                "morphology"
            }
            F::EdgeDetect { width, threshold } => {
                b.insert("width", n(f64::from(*width)));
                b.insert("threshold", n(*threshold));
                "edge_detect"
            }
            F::HighPass { radius } => {
                b.insert("radius", n(f64::from(*radius)));
                "high_pass"
            }
            F::Median { radius } => {
                b.insert("radius", n(f64::from(*radius)));
                "median"
            }
            F::Glow {
                threshold,
                radius,
                intensity,
            } => {
                b.insert("threshold", n(*threshold));
                b.insert("radius", n(f64::from(*radius)));
                b.insert("intensity", n(*intensity));
                "glow"
            }
        },
    };
    Some((id, b))
}

/// 調整の層の設定の、種類と値。
fn read_adjustment(a: &AdjustmentSettings) -> (&'static str, Bag) {
    let mut b = Bag::new();
    if let Some(ca) = a.color_adjust() {
        return (color_adjust_id(&ca), read_color_adjust(&ca));
    }
    let id = match a.kind() {
        AdjustmentType::Invert => "invert",
        AdjustmentType::Levels => {
            b.insert("input_black", n(a.input_black()));
            b.insert("input_white", n(a.input_white()));
            b.insert("gamma", n(a.gamma()));
            b.insert("output_black", n(a.output_black()));
            b.insert("output_white", n(a.output_white()));
            "levels"
        }
        AdjustmentType::HueSaturation => {
            b.insert("hue", n(a.hue()));
            b.insert("saturation", n(a.saturation()));
            b.insert("lightness", n(a.lightness()));
            "hue_saturation"
        }
        // 64 からの種類は上で読んだ
        _ => "unknown",
    };
    (id, b)
}

// ───────── 組む ─────────

fn get_n(bag: &Bag, name: &str) -> f64 {
    match bag.get(name) {
        Some(ParamValue::Number(v)) => *v,
        _ => 0.0,
    }
}
fn get_b(bag: &Bag, name: &str) -> bool {
    matches!(bag.get(name), Some(ParamValue::Bool(true)))
}
fn get_c<'a>(bag: &'a Bag, name: &str) -> &'a str {
    match bag.get(name) {
        Some(ParamValue::Choice(v)) => v,
        _ => "",
    }
}
fn refused(e: CoreError) -> ParamError {
    ParamError::Refused(e)
}

fn build_color_adjust(id: &str, bag: &Bag) -> Result<ColorAdjust, ParamError> {
    Ok(match id {
        "color_balance" => {
            let v = |i: usize| -> [f64; 3] {
                [
                    get_n(bag, BALANCE_NAMES[i * 3]),
                    get_n(bag, BALANCE_NAMES[i * 3 + 1]),
                    get_n(bag, BALANCE_NAMES[i * 3 + 2]),
                ]
            };
            ColorAdjust::ColorBalance(
                ColorBalance::new(v(0), v(1), v(2), get_b(bag, "preserve_luminosity"))
                    .map_err(refused)?,
            )
        }
        "brightness_contrast" => ColorAdjust::BrightnessContrast(
            BrightnessContrast::new(get_n(bag, "brightness"), get_n(bag, "contrast"))
                .map_err(refused)?,
        ),
        "threshold" => {
            ColorAdjust::Threshold(Threshold::new(get_n(bag, "level") as u32).map_err(refused)?)
        }
        "posterize" => {
            ColorAdjust::Posterize(Posterize::new(get_n(bag, "levels") as u32).map_err(refused)?)
        }
        _ => unreachable!("色調補正の種類だけ"),
    })
}

fn build_generator(
    id: &str,
    mut g: generator::Settings,
    bag: &Bag,
) -> Result<EffectSettings, ParamError> {
    g.low = get_n(bag, "low");
    g.high = get_n(bag, "high");
    g.softness = get_n(bag, "softness");
    g.invert = get_b(bag, "invert");
    g.blend = blend_from(get_c(bag, "blend"));
    match id {
        // 模様・光の softness は種類の欄（共通の減衰は 0 のまま）
        "pattern" => {
            g.softness = 0.;
            g.pattern = generator::Pattern {
                shape: PATTERN_SHAPE_OPTIONS
                    .iter()
                    .position(|o| *o == get_c(bag, "shape"))
                    .and_then(|i| generator::PatternShape::from_index(i as i64))
                    .unwrap_or(generator::PatternShape::Stripes),
                scale: get_n(bag, "scale"),
                angle: get_n(bag, "angle"),
                width: get_n(bag, "width"),
                softness: get_n(bag, "softness"),
                offset: [get_n(bag, "offset_u"), get_n(bag, "offset_v")],
            };
        }
        "light" => {
            g.softness = 0.;
            g.light = generator::Light {
                azimuth: get_n(bag, "azimuth"),
                elevation: get_n(bag, "elevation"),
                softness: get_n(bag, "softness"),
                ambient: get_n(bag, "ambient"),
            };
        }
        "mask_builder" => {
            for (map, input) in MASK_MAPS.iter().zip(g.mask_builder.inputs.iter_mut()) {
                *input = generator::MaskInput {
                    weight: get_n(bag, mask_param(map, "weight")),
                    level: get_n(bag, mask_param(map, "level")),
                    contrast: get_n(bag, mask_param(map, "contrast")),
                    invert: get_b(bag, mask_param(map, "invert")),
                };
            }
            g.mask_builder.combine = MASK_COMBINE_OPTIONS
                .iter()
                .position(|o| *o == get_c(bag, "combine"))
                .and_then(|i| generator::MaskCombine::from_index(i as i64))
                .unwrap_or(generator::MaskCombine::Multiply);
        }
        _ => {}
    }
    if !g.kind.is_procedural() && !g.kind.is_050() {
        g.noise_amount = get_n(bag, "noise_amount");
        g.noise_scale = get_n(bag, "noise_scale");
        g.noise_seed = get_n(bag, "noise_seed") as i32;
        g.noise_space = if get_c(bag, "noise_space") == "uv" {
            NoiseSpace::Uv
        } else {
            NoiseSpace::Model
        };
    }
    match id {
        "dirt" => g.balance = get_n(bag, "balance"),
        "position_gradient" => {
            g.axis = AXIS_OPTIONS
                .iter()
                .position(|a| *a == get_c(bag, "axis"))
                .unwrap_or(1);
        }
        "direction" => {
            g.direction = [
                get_n(bag, "direction_x"),
                get_n(bag, "direction_y"),
                get_n(bag, "direction_z"),
            ];
            g.use_bent_normal = get_b(bag, "use_bent_normal");
        }
        _ => {}
    }
    if g.kind.is_procedural() {
        let p = &mut g.procedural;
        p.space = match get_c(bag, "space") {
            "triplanar" => ProceduralSpace::Triplanar,
            "uv" => ProceduralSpace::Uv,
            _ => ProceduralSpace::Position,
        };
        p.scale = get_n(bag, "scale");
        p.seed = get_n(bag, "seed") as i32;
        p.rotation = [
            get_n(bag, "rotation_x"),
            get_n(bag, "rotation_y"),
            get_n(bag, "rotation_z"),
        ];
        p.bleed = get_n(bag, "bleed");
        p.blend_width = get_n(bag, "blend_width");
        if id == "procedural_noise" {
            p.basis = match get_c(bag, "basis") {
                "value" => NoiseBasis::Value,
                "worley" => NoiseBasis::Worley,
                _ => NoiseBasis::Perlin,
            };
            p.cell_output = match get_c(bag, "cell_output") {
                "f2" => CellOutput::F2,
                "f2_minus_f1" => CellOutput::F2MinusF1,
                _ => CellOutput::F1,
            };
            p.fractal = match get_c(bag, "fractal") {
                "ridged" => FractalMode::Ridged,
                "turbulence" => FractalMode::Turbulence,
                _ => FractalMode::Fbm,
            };
            p.octaves = get_n(bag, "octaves") as u32;
            p.lacunarity = get_n(bag, "lacunarity");
            p.gain = get_n(bag, "gain");
        } else {
            let want = get_c(bag, "preset");
            p.preset = GrungePreset::ALL
                .iter()
                .copied()
                .find(|x| x.id() == want)
                .unwrap_or(p.preset);
        }
    }
    g.validate().map_err(|e| match e {
        generator::Error::Invalid(why) => refused(CoreError::InvalidArgument(why)),
        _ => refused(CoreError::InvalidArgument("ジェネレーターの設定")),
    })?;
    Ok(EffectSettings::generator(g))
}

/// フィルターのスタックの段を、欄の値から組む。`base` は変える前の段（Generator のピンなど値の欄に無い中身を残すため。足すときは None）。
fn build_stack(
    id: &str,
    bag: &Bag,
    base: Option<&EffectSettings>,
) -> Result<EffectSettings, ParamError> {
    use generator::Kind as G;
    let settings = match id {
        "blur" => EffectSettings::blur(get_n(bag, "radius") as u32),
        "sharpen" => EffectSettings::sharpen(
            get_n(bag, "radius") as u32,
            get_n(bag, "amount"),
            get_n(bag, "threshold") as u32,
        ),
        "noise" => EffectSettings::noise(
            get_n(bag, "amount"),
            get_n(bag, "seed") as i32,
            get_b(bag, "monochrome"),
        ),
        "levels" => EffectSettings::levels(
            get_n(bag, "input_black"),
            get_n(bag, "input_white"),
            get_n(bag, "gamma"),
            get_n(bag, "output_black"),
            get_n(bag, "output_white"),
        ),
        "invert" => EffectSettings::invert(),
        "normalize" => EffectSettings::normalize(),
        "color_balance" | "brightness_contrast" | "threshold" | "posterize" => {
            EffectSettings::from_color_adjust(build_color_adjust(id, bag)?)
        }
        "histogram_scan" => EffectSettings::Filter(filter::Settings::HistogramScan {
            position: get_n(bag, "position"),
            contrast: get_n(bag, "contrast"),
        }),
        "histogram_range" => EffectSettings::Filter(filter::Settings::HistogramRange {
            range: get_n(bag, "range"),
            position: get_n(bag, "position"),
        }),
        "slope_blur" => EffectSettings::Filter(filter::Settings::SlopeBlur {
            intensity: get_n(bag, "intensity"),
            samples: get_n(bag, "samples") as u32,
            mode: match get_c(bag, "mode") {
                "min" => filter::SlopeMode::Min,
                "max" => filter::SlopeMode::Max,
                _ => filter::SlopeMode::Blur,
            },
            scale: get_n(bag, "scale"),
            seed: get_n(bag, "seed") as i32,
        }),
        "directional_blur" => EffectSettings::Filter(filter::Settings::DirectionalBlur {
            angle: get_n(bag, "angle"),
            distance: get_n(bag, "distance"),
        }),
        "warp" => EffectSettings::Filter(filter::Settings::Warp {
            intensity: get_n(bag, "intensity"),
            scale: get_n(bag, "scale"),
            seed: get_n(bag, "seed") as i32,
        }),
        "morphology" => EffectSettings::Filter(filter::Settings::Morphology {
            mode: if get_c(bag, "mode") == "erode" {
                filter::MorphologyMode::Erode
            } else {
                filter::MorphologyMode::Dilate
            },
            radius: get_n(bag, "radius") as u32,
        }),
        "edge_detect" => EffectSettings::Filter(filter::Settings::EdgeDetect {
            width: get_n(bag, "width") as u32,
            threshold: get_n(bag, "threshold"),
        }),
        "high_pass" => EffectSettings::Filter(filter::Settings::HighPass {
            radius: get_n(bag, "radius") as u32,
        }),
        "median" => EffectSettings::Filter(filter::Settings::Median {
            radius: get_n(bag, "radius") as u32,
        }),
        "glow" => EffectSettings::Filter(filter::Settings::Glow {
            threshold: get_n(bag, "threshold"),
            radius: get_n(bag, "radius") as u32,
            intensity: get_n(bag, "intensity"),
        }),
        "edge_wear" | "dirt" | "position_gradient" | "thickness" | "direction"
        | "procedural_noise" | "grunge" | "pattern" | "light" | "mask_builder" => {
            let kind = match id {
                "edge_wear" => G::EdgeWear,
                "dirt" => G::Dirt,
                "position_gradient" => G::PositionGradient,
                "thickness" => G::Thickness,
                "direction" => G::Direction,
                "procedural_noise" => G::Noise,
                "pattern" => G::Pattern,
                "light" => G::Light,
                "mask_builder" => G::MaskBuilder,
                _ => G::Grunge,
            };
            let start = match base.and_then(EffectSettings::generator_settings) {
                Some(g) if g.kind == kind => g.clone(),
                _ => generator::Settings::new(kind),
            };
            return build_generator(id, start, bag);
        }
        _ => {
            return Err(ParamError::NotEditable {
                kind: static_id(id),
            })
        }
    };
    // 組んだ設定の検査（欄の組み合わせの条件。チャンネルの種類に依る条件は文書に足すときに見る）
    match &settings {
        EffectSettings::Filter(f) => f
            .validate_values()
            .map_err(|_| refused(CoreError::InvalidArgument("フィルターの設定")))?,
        EffectSettings::Generator(_) => {}
    }
    Ok(settings)
}

fn build_adjustment(id: &str, bag: &Bag) -> Result<AdjustmentSettings, ParamError> {
    let settings = match id {
        "invert" => AdjustmentSettings::invert(),
        "levels" => AdjustmentSettings::levels(
            get_n(bag, "input_black"),
            get_n(bag, "input_white"),
            get_n(bag, "gamma"),
            get_n(bag, "output_black"),
            get_n(bag, "output_white"),
        )
        .map_err(refused)?,
        "hue_saturation" => AdjustmentSettings::hue_saturation(
            get_n(bag, "hue"),
            get_n(bag, "saturation"),
            get_n(bag, "lightness"),
        )
        .map_err(refused)?,
        "color_balance" | "brightness_contrast" | "threshold" | "posterize" => {
            build_color_adjust(id, bag)?.into_settings()
        }
        _ => {
            return Err(ParamError::NotEditable {
                kind: static_id(id),
            })
        }
    };
    settings.validate().map_err(refused)?;
    Ok(settings)
}

/// 読んだ値を、表の欄の並びにする。
fn ordered(id: &str, mut bag: Bag) -> Vec<(&'static str, ParamValue)> {
    kind(id)
        .map(|k| {
            k.params
                .iter()
                .filter_map(|p| bag.remove(p.name).map(|v| (p.name, v)))
                .collect()
        })
        .unwrap_or_default()
}

fn static_id(id: &str) -> &'static str {
    kind(id).map_or("unknown", |k| k.id)
}

// ───────── 値の重ね合わせ ─────────

/// 欄の名前・型・範囲を確かめて、`base` の上に重ねる。グランジのプリセットが変わるときは、プリセットの既定（模様の大きさ・レベル）を先に
/// 入れてから、渡された値で上書きする。
fn overlay(
    kind: &'static EffectKind,
    mut base: Bag,
    given: &BTreeMap<String, ParamValue>,
) -> Result<Bag, ParamError> {
    for (name, value) in given {
        let Some(param) = kind.param(name) else {
            return Err(ParamError::UnknownParam {
                kind: kind.id,
                name: name.clone(),
            });
        };
        check_value(param, value)?;
    }
    if kind.id == "grunge" {
        if let Some(ParamValue::Choice(want)) = given.get("preset") {
            if base.get("preset") != Some(&ParamValue::Choice(want.clone())) {
                if let Some(preset) = GrungePreset::ALL.iter().find(|p| p.id() == want) {
                    let (low, high) = preset.default_levels();
                    base.insert("scale", n(preset.default_scale()));
                    base.insert("low", n(low));
                    base.insert("high", n(high));
                }
            }
        }
    }
    for param in &kind.params {
        if let Some(value) = given.get(param.name) {
            base.insert(param.name, value.clone());
        }
    }
    Ok(base)
}

fn check_value(param: &Param, value: &ParamValue) -> Result<(), ParamError> {
    match (&param.ty, value) {
        (ParamType::Integer { min, max }, ParamValue::Number(v)) => {
            if !v.is_finite() || v.fract() != 0.0 {
                return Err(ParamError::NotInteger { name: param.name });
            }
            if *v < *min as f64 || *v > *max as f64 {
                return Err(ParamError::OutOfRange {
                    name: param.name,
                    min: *min as f64,
                    max: *max as f64,
                });
            }
            Ok(())
        }
        (ParamType::Number { min, max }, ParamValue::Number(v)) => {
            if !v.is_finite() || *v < *min || *v > *max {
                return Err(ParamError::OutOfRange {
                    name: param.name,
                    min: *min,
                    max: *max,
                });
            }
            Ok(())
        }
        (ParamType::Bool, ParamValue::Bool(_)) => Ok(()),
        (ParamType::Choice { options }, ParamValue::Choice(v)) => {
            if options.iter().any(|o| *o == v) {
                Ok(())
            } else {
                Err(ParamError::UnknownOption {
                    name: param.name,
                    options: options.clone(),
                })
            }
        }
        (ty, _) => Err(ParamError::WrongType {
            name: param.name,
            expected: match ty {
                ParamType::Integer { .. } => "整数（数）",
                ParamType::Number { .. } => "数",
                ParamType::Bool => "真偽",
                ParamType::Choice { .. } => "選択肢の文字列",
            },
        }),
    }
}

fn editable(id: &str, adjustment: bool) -> Result<&'static EffectKind, ParamError> {
    let kind = kind(id).ok_or_else(|| ParamError::UnknownKind(id.to_owned()))?;
    if (adjustment && !kind.adjustment) || (!adjustment && !kind.stack) {
        return Err(ParamError::WrongTarget {
            kind: kind.id,
            adjustment,
        });
    }
    if !kind.addable {
        return Err(ParamError::NotEditable { kind: kind.id });
    }
    Ok(kind)
}

impl EffectSettings {
    /// 効果の種類の名前（[`kinds`] の `id`）。
    pub fn kind_id(&self) -> &'static str {
        read_stack(self).map_or("generator", |(id, _)| id)
    }
    /// 値の欄の値（欄を持たない種類は空。並びは表の欄の並び）。
    pub fn catalog_values(&self) -> Vec<(&'static str, ParamValue)> {
        read_stack(self)
            .map(|(id, bag)| ordered(id, bag))
            .unwrap_or_default()
    }
    /// 値の欄では変えられない中身（グラデーションマップのランプなど。無ければ空）。
    pub fn opaque_parts(&self) -> &'static [&'static str] {
        kind(self.kind_id()).map_or(&[], |k| k.opaque)
    }
    /// 種類の名前と値から、新しい設定を組む（渡さない欄は既定）。
    pub fn from_catalog(
        kind_id: &str,
        values: &BTreeMap<String, ParamValue>,
    ) -> Result<Self, ParamError> {
        let kind = editable(kind_id, false)?;
        let bag = overlay(kind, default_bag(kind.id), values)?;
        build_stack(kind.id, &bag, None)
    }
    /// 今の設定に、渡した欄の値だけを重ねた設定（同じ種類のまま。値の欄に無い中身は残す）。
    pub fn with_catalog_values(
        &self,
        values: &BTreeMap<String, ParamValue>,
    ) -> Result<Self, ParamError> {
        let id = self.kind_id();
        let kind = kind(id).ok_or_else(|| ParamError::UnknownKind(id.to_owned()))?;
        if !kind.addable {
            return if values.is_empty() {
                Ok(self.clone())
            } else {
                Err(ParamError::NotEditable { kind: kind.id })
            };
        }
        let base = read_stack(self).map(|(_, bag)| bag).unwrap_or_default();
        let bag = overlay(kind, base, values)?;
        build_stack(kind.id, &bag, Some(self))
    }
}

impl AdjustmentSettings {
    /// 調整の種類の名前（[`kinds`] の `id`）。
    pub fn kind_id(&self) -> &'static str {
        read_adjustment(self).0
    }
    /// 値の欄の値（欄を持たない種類は空。並びは表の欄の並び）。
    pub fn catalog_values(&self) -> Vec<(&'static str, ParamValue)> {
        let (id, bag) = read_adjustment(self);
        ordered(id, bag)
    }
    /// 値の欄では変えられない中身（グラデーションマップのランプ・トーンカーブ。無ければ空）。
    pub fn opaque_parts(&self) -> &'static [&'static str] {
        kind(self.kind_id()).map_or(&[], |k| k.opaque)
    }
    /// 種類の名前と値から、新しい調整の設定を組む（渡さない欄は既定）。
    pub fn from_catalog(
        kind_id: &str,
        values: &BTreeMap<String, ParamValue>,
    ) -> Result<Self, ParamError> {
        let kind = editable(kind_id, true)?;
        let bag = overlay(kind, default_bag(kind.id), values)?;
        build_adjustment(kind.id, &bag)
    }
    /// 今の設定に、渡した欄の値だけを重ねた設定（同じ種類のまま）。
    pub fn with_catalog_values(
        &self,
        values: &BTreeMap<String, ParamValue>,
    ) -> Result<Self, ParamError> {
        let id = self.kind_id();
        let kind = kind(id).ok_or_else(|| ParamError::UnknownKind(id.to_owned()))?;
        if !kind.addable {
            return if values.is_empty() {
                Ok(self.clone())
            } else {
                Err(ParamError::NotEditable { kind: kind.id })
            };
        }
        let bag = overlay(kind, read_adjustment(self).1, values)?;
        build_adjustment(kind.id, &bag)
    }
}

#[cfg(test)]
mod tests;
