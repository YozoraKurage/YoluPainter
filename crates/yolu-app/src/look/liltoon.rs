//! lilToon（2.3.4、MIT・Copyright (c) 2020-present lilxyzw）のマテリアルの表: 3D ビューで再現するプロパティの名前・既定・範囲と、
//! テクスチャのスロット、シェーダーの名前と描画モードの対応。既定と範囲はシェーダーの `Properties`（`Shader/lts_o.shader`）の値、
//! 節の分け方・並び・名前はインスペクター（`Editor/lilInspector/*.cs` と `Editor/Localization/ja-JP.po`・`en-US.po`）に合わせる。
//!
//! 値は [`yolu_core::look::MaterialLook`] にプロパティの名前で持ち、無い名前はここの既定で描く。ここに無い名前（ファー・宝石・
//! AudioLink など、再現しない機能の値や Live Link で受けた値）も設定には残り、描かないだけ。

use yolu_core::look::{MaterialLook, LILTOON_SHADER};

use super::Section;
use crate::lang::Lang;

/// 値の欄の種類。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// 0 / 1 の入切（Unity の `[lilToggle]` の Int）。
    Toggle,
    /// 範囲のある数（`Range`。`power` は Unity の `PowerSlider` の指数、1 なら等間隔）。
    Slider { min: f32, max: f32, power: f32 },
    /// 範囲の無い数（`Float`。欄は `min`〜`max` で動かす）。
    Number { min: f32, max: f32 },
    /// 選ぶ（値は番号）。名前は日英。
    Choice(&'static [(&'static str, &'static str)]),
    /// 色（`hdr` なら 1 を超えてよい）。
    Color { hdr: bool },
    /// 角度（`[lilAngle]`。値はラジアン、欄は −180〜180 度）。
    Angle,
    /// ミップの段（`[lilLOD]`。欄は値の 4 乗根を 0〜1 で動かす）。
    Lod,
    /// ベクトル（欄は成分ごとに、節の作りが決める）。
    Vector,
}

/// 1 つのプロパティ。
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub name: &'static str,
    /// 既定（数は x。色はガンマの空間の RGBA）。
    pub default: [f32; 4],
    pub kind: Kind,
    pub ja: &'static str,
    pub en: &'static str,
}

impl Prop {
    pub fn label(&self, lang: Lang) -> &'static str {
        lang.pick(self.ja, self.en)
    }
}

const BLEND: &[(&str, &str)] = &[
    ("通常", "Normal"),
    ("加算", "Add"),
    ("スクリーン", "Screen"),
    ("乗算", "Multiply"),
];
pub const CULL: &[(&str, &str)] = &[
    ("Off（両面を描画）", "Off"),
    ("Front（裏面のみ描画）", "Front"),
    ("Back（前面のみ描画）", "Back"),
];
const ALPHA_MASK: &[(&str, &str)] = &[
    ("None", "None"),
    ("置き換え", "Replace"),
    ("乗算", "Multiply"),
    ("加算", "Add"),
    ("減算", "Subtract"),
];
const SHADOW_MASK: &[(&str, &str)] = &[("強度", "Strength"), ("平面化", "Flat"), ("SDF", "SDF")];
const SHADOW_COLOR_TYPE: &[(&str, &str)] = &[("通常", "Normal"), ("LUT", "LUT")];
const EMISSION_UV: &[(&str, &str)] = &[
    ("UV0", "UV0"),
    ("UV1", "UV1"),
    ("UV2", "UV2"),
    ("UV3", "UV3"),
    ("リム", "Rim"),
];
const LAYER_UV: &[(&str, &str)] = &[
    ("UV0", "UV0"),
    ("UV1", "UV1"),
    ("UV2", "UV2"),
    ("UV3", "UV3"),
    ("MatCap", "MatCap"),
];
const UV4: &[(&str, &str)] = &[
    ("UV0", "UV0"),
    ("UV1", "UV1"),
    ("UV2", "UV2"),
    ("UV3", "UV3"),
];
const UV2: &[(&str, &str)] = &[("UV0", "UV0"), ("UV1", "UV1")];
const OUTLINE_VERTEX: &[(&str, &str)] = &[
    ("None", "None"),
    ("R -> Width", "R -> Width"),
    ("RGBA -> Normal & Width", "RGBA -> Normal & Width"),
];
const DISTANCE_FADE_MODE: &[(&str, &str)] = &[("頂点", "Vertex"), ("座標", "Position")];

const fn f(
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
    ja: &'static str,
    en: &'static str,
) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Slider {
            min,
            max,
            power: 1.0,
        },
        ja,
        en,
    }
}

const fn p3(
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
    ja: &'static str,
    en: &'static str,
) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Slider {
            min,
            max,
            power: 3.0,
        },
        ja,
        en,
    }
}

const fn n(
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
    ja: &'static str,
    en: &'static str,
) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Number { min, max },
        ja,
        en,
    }
}

const fn t(name: &'static str, default: f32, ja: &'static str, en: &'static str) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Toggle,
        ja,
        en,
    }
}

const fn e(
    name: &'static str,
    default: f32,
    options: &'static [(&'static str, &'static str)],
    ja: &'static str,
    en: &'static str,
) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Choice(options),
        ja,
        en,
    }
}

const fn c(
    name: &'static str,
    default: [f32; 4],
    hdr: bool,
    ja: &'static str,
    en: &'static str,
) -> Prop {
    Prop {
        name,
        default,
        kind: Kind::Color { hdr },
        ja,
        en,
    }
}

const fn v(name: &'static str, default: [f32; 4]) -> Prop {
    Prop {
        name,
        default,
        kind: Kind::Vector,
        ja: name,
        en: name,
    }
}

const fn a(name: &'static str, ja: &'static str, en: &'static str) -> Prop {
    Prop {
        name,
        default: [0.0; 4],
        kind: Kind::Angle,
        ja,
        en,
    }
}

const fn lod(name: &'static str) -> Prop {
    Prop {
        name,
        default: [0.0; 4],
        kind: Kind::Lod,
        ja: "LOD",
        en: "LOD",
    }
}

const BASE: &[Prop] = &[
    f(
        "_Cutoff",
        0.5,
        -0.001,
        1.001,
        "Cutoff（完全透明にする範囲）",
        "Cutoff",
    ),
    e("_Cull", 2.0, CULL, "Cull Mode", "Cull Mode"),
    t(
        "_FlipNormal",
        0.0,
        "裏面の法線を反転",
        "Flip Backface Normal",
    ),
    f(
        "_BackfaceForceShadow",
        0.0,
        0.0,
        1.0,
        "裏面を影にする",
        "Backface Force Shadow",
    ),
    c("_BackfaceColor", [0.0, 0.0, 0.0, 0.0], true, "色", "Color"),
    t("_Invisible", 0.0, "非表示", "Invisible"),
    f(
        "_AAStrength",
        1.0,
        0.0,
        1.0,
        "アンチエイリアスシェーディング",
        "Anti-aliasing shading",
    ),
];

const LIGHTING: &[Prop] = &[
    f(
        "_LightMinLimit",
        0.05,
        0.0,
        1.0,
        "明るさの下限",
        "Lower brightness limit",
    ),
    f(
        "_LightMaxLimit",
        1.0,
        0.0,
        10.0,
        "明るさの上限",
        "Upper brightness limit",
    ),
    f(
        "_MonochromeLighting",
        0.0,
        0.0,
        1.0,
        "ライトのモノクロ化",
        "Monochrome Lighting",
    ),
    f(
        "_ShadowEnvStrength",
        0.0,
        0.0,
        1.0,
        "影色への環境光影響度",
        "Environment strength on shadow color",
    ),
    f("_AsUnlit", 0.0, 0.0, 1.0, "Unlit化", "As Unlit"),
    v("_LightDirectionOverride", [0.001, 0.002, 0.001, 0.0]),
];

const UV: &[Prop] = &[
    v("_MainTex_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_MainTex_ScrollRotate", [0.0; 4]),
    t(
        "_ShiftBackfaceUV",
        0.0,
        "裏面のUVをずらす",
        "Shift Backface UV",
    ),
];

/// メインカラー 2nd・3rd の同じ形の値（`_Use<名>Tex`・`_Color<n>`・`_<名>Tex…`）。
macro_rules! layer {
    ($l:literal, $n:literal, $ja:literal, $en:literal) => {
        [
            t(concat!("_Use", $l, "Tex"), 0.0, $ja, $en),
            c(concat!("_Color", $n), [1.0; 4], true, "色", "Color"),
            v(concat!("_", $l, "Tex_ST"), [1.0, 1.0, 0.0, 0.0]),
            a(concat!("_", $l, "TexAngle"), "角度", "Angle"),
            e(
                concat!("_", $l, "Tex_UVMode"),
                0.0,
                LAYER_UV,
                "UV Mode",
                "UV Mode",
            ),
            e(
                concat!("_", $l, "Tex_Cull"),
                0.0,
                CULL,
                "Cull Mode",
                "Cull Mode",
            ),
            t(
                concat!("_", $l, "TexIsMSDF"),
                0.0,
                "MSDFテクスチャ",
                "MSDF Texture",
            ),
            f(
                concat!("_", $l, "EnableLighting"),
                1.0,
                0.0,
                1.0,
                "ライトの明るさを反映",
                "Enable Lighting",
            ),
            e(
                concat!("_", $l, "TexBlendMode"),
                0.0,
                BLEND,
                "合成モード",
                "Blending Mode",
            ),
            e(
                concat!("_", $l, "TexAlphaMode"),
                0.0,
                ALPHA_MASK,
                "透過モード",
                "Transparent Mode",
            ),
            t(
                concat!("_", $l, "TexIsDecal"),
                0.0,
                "デカール化",
                "As Decal",
            ),
            t(
                concat!("_", $l, "TexIsLeftOnly"),
                0.0,
                "Left Only",
                "Left Only",
            ),
            t(
                concat!("_", $l, "TexIsRightOnly"),
                0.0,
                "Right Only",
                "Right Only",
            ),
            t(concat!("_", $l, "TexShouldCopy"), 0.0, "Copy", "Copy"),
            t(
                concat!("_", $l, "TexShouldFlipMirror"),
                0.0,
                "Flip Mirror",
                "Flip Mirror",
            ),
            t(
                concat!("_", $l, "TexShouldFlipCopy"),
                0.0,
                "Flip Copy",
                "Flip Copy",
            ),
            v(concat!("_", $l, "TexDecalAnimation"), [1.0, 1.0, 1.0, 30.0]),
            v(concat!("_", $l, "TexDecalSubParam"), [1.0, 1.0, 0.0, 1.0]),
            v(concat!("_", $l, "DistanceFade"), [0.1, 0.01, 0.0, 0.0]),
        ]
    };
}

const MAIN2ND: [Prop; 19] = layer!("Main2nd", "2nd", "メインカラー2nd", "Main Color 2nd");
const MAIN3RD: [Prop; 19] = layer!("Main3rd", "3rd", "メインカラー3rd", "Main Color 3rd");

const MAIN: &[Prop] = &[
    c("_Color", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_MainTexHSVG", [0.0, 1.0, 1.0, 1.0]),
    e(
        "_AlphaMaskMode",
        0.0,
        ALPHA_MASK,
        "アルファマスク",
        "Alpha Mask",
    ),
    v("_AlphaMask_ST", [1.0, 1.0, 0.0, 0.0]),
    n("_AlphaMaskScale", 1.0, -1.0, 1.0, "Scale", "Scale"),
    n("_AlphaMaskValue", 0.0, -1.0, 1.0, "Offset", "Offset"),
];

const SHADOW: &[Prop] = &[
    t("_UseShadow", 0.0, "影", "Shadow"),
    e(
        "_ShadowMaskType",
        0.0,
        SHADOW_MASK,
        "マスクタイプ",
        "Mask Type",
    ),
    f("_ShadowStrength", 1.0, 0.0, 1.0, "強度", "Strength"),
    lod("_ShadowStrengthMaskLOD"),
    f("_ShadowFlatBorder", 1.0, -2.0, 2.0, "範囲", "Border"),
    f("_ShadowFlatBlur", 1.0, 0.001, 2.0, "ぼかし", "Blur"),
    e(
        "_ShadowColorType",
        0.0,
        SHADOW_COLOR_TYPE,
        "カラータイプ",
        "Color Type",
    ),
    c(
        "_ShadowColor",
        [0.82, 0.76, 0.85, 1.0],
        false,
        "影色1",
        "1st Color",
    ),
    f("_ShadowBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_ShadowBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f(
        "_ShadowNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f(
        "_ShadowReceive",
        0.0,
        0.0,
        1.0,
        "影を受け取る",
        "Receive Shadow",
    ),
    c(
        "_Shadow2ndColor",
        [0.68, 0.66, 0.79, 1.0],
        false,
        "影色2",
        "2nd Color",
    ),
    f("_Shadow2ndBorder", 0.15, 0.0, 1.0, "範囲", "Border"),
    f("_Shadow2ndBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f(
        "_Shadow2ndNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f(
        "_Shadow2ndReceive",
        0.0,
        0.0,
        1.0,
        "影を受け取る",
        "Receive Shadow",
    ),
    c(
        "_Shadow3rdColor",
        [0.0, 0.0, 0.0, 0.0],
        false,
        "影色3",
        "3rd Color",
    ),
    f("_Shadow3rdBorder", 0.25, 0.0, 1.0, "範囲", "Border"),
    f("_Shadow3rdBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f(
        "_Shadow3rdNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f(
        "_Shadow3rdReceive",
        0.0,
        0.0,
        1.0,
        "影を受け取る",
        "Receive Shadow",
    ),
    c(
        "_ShadowBorderColor",
        [1.0, 0.1, 0.0, 1.0],
        false,
        "境界の色",
        "Border Color",
    ),
    f(
        "_ShadowBorderRange",
        0.08,
        0.0,
        1.0,
        "境界の幅",
        "Border Range",
    ),
    f(
        "_ShadowMainStrength",
        0.0,
        0.0,
        1.0,
        "コントラスト",
        "Contrast",
    ),
    lod("_ShadowBlurMaskLOD"),
    lod("_ShadowBorderMaskLOD"),
    t(
        "_ShadowPostAO",
        0.0,
        "影範囲設定を無視して適用",
        "Ignore border properties",
    ),
    v("_ShadowAOShift", [1.0, 0.0, 1.0, 0.0]),
    v("_ShadowAOShift2", [1.0, 0.0, 1.0, 0.0]),
];

const RIM_SHADE: &[Prop] = &[
    t("_UseRimShade", 0.0, "リムシェード", "RimShade"),
    c("_RimShadeColor", [0.5, 0.5, 0.5, 1.0], false, "色", "Color"),
    f(
        "_RimShadeNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f("_RimShadeBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_RimShadeBlur", 1.0, 0.0, 1.0, "ぼかし", "Blur"),
    p3(
        "_RimShadeFresnelPower",
        1.0,
        0.01,
        50.0,
        "リムライトの細さ",
        "Fresnel Power",
    ),
];

const EMISSION: &[Prop] = &[
    t("_UseEmission", 0.0, "発光テクスチャ", "Emission"),
    c("_EmissionColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_EmissionMap_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_EmissionMap_ScrollRotate", [0.0; 4]),
    e(
        "_EmissionMap_UVMode",
        0.0,
        EMISSION_UV,
        "UV Mode",
        "UV Mode",
    ),
    f(
        "_EmissionMainStrength",
        0.0,
        0.0,
        1.0,
        "メインカラーの強度",
        "Main Color Power",
    ),
    f("_EmissionBlend", 1.0, 0.0, 1.0, "Blend", "Blend"),
    v("_EmissionBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_EmissionBlendMask_ScrollRotate", [0.0; 4]),
    e(
        "_EmissionBlendMode",
        1.0,
        BLEND,
        "合成モード",
        "Blending Mode",
    ),
    f(
        "_EmissionFluorescence",
        0.0,
        0.0,
        1.0,
        "蛍光",
        "Fluorescence",
    ),
    t("_UseEmission2nd", 0.0, "発光テクスチャ2nd", "Emission 2nd"),
    c(
        "_Emission2ndColor",
        [1.0, 1.0, 1.0, 1.0],
        true,
        "色",
        "Color",
    ),
    v("_Emission2ndMap_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_Emission2ndMap_ScrollRotate", [0.0; 4]),
    e(
        "_Emission2ndMap_UVMode",
        0.0,
        EMISSION_UV,
        "UV Mode",
        "UV Mode",
    ),
    f(
        "_Emission2ndMainStrength",
        0.0,
        0.0,
        1.0,
        "メインカラーの強度",
        "Main Color Power",
    ),
    f("_Emission2ndBlend", 1.0, 0.0, 1.0, "Blend", "Blend"),
    v("_Emission2ndBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_Emission2ndBlendMask_ScrollRotate", [0.0; 4]),
    e(
        "_Emission2ndBlendMode",
        1.0,
        BLEND,
        "合成モード",
        "Blending Mode",
    ),
    f(
        "_Emission2ndFluorescence",
        0.0,
        0.0,
        1.0,
        "蛍光",
        "Fluorescence",
    ),
];

const NORMAL: &[Prop] = &[
    t("_UseBumpMap", 0.0, "ノーマルマップ", "Normal Map"),
    v("_BumpMap_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_BumpScale", 1.0, -10.0, 10.0, "Scale", "Scale"),
    t("_UseBump2ndMap", 0.0, "ノーマルマップ2nd", "Normal Map 2nd"),
    v("_Bump2ndMap_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_Bump2ndScale", 1.0, -10.0, 10.0, "Scale", "Scale"),
    e("_Bump2ndMap_UVMode", 0.0, UV4, "UV Mode", "UV Mode"),
    t("_UseAnisotropy", 0.0, "異方性反射", "Anisotropy"),
    v("_AnisotropyTangentMap_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_AnisotropyScale", 1.0, -1.0, 1.0, "Scale", "Scale"),
    v("_AnisotropyScaleMask_ST", [1.0, 1.0, 0.0, 0.0]),
    t("_Anisotropy2Reflection", 0.0, "反射", "Reflection"),
    f(
        "_AnisotropyTangentWidth",
        1.0,
        0.0,
        10.0,
        "タンジェント方向の幅",
        "Tangent Width",
    ),
    f(
        "_AnisotropyBitangentWidth",
        1.0,
        0.0,
        10.0,
        "バイタンジェント方向の幅",
        "Bitangent Width",
    ),
    f("_AnisotropyShift", 0.0, -10.0, 10.0, "Offset", "Offset"),
    f(
        "_AnisotropyShiftNoiseScale",
        0.0,
        -1.0,
        1.0,
        "ノイズの強度",
        "Noise Strength",
    ),
    f(
        "_AnisotropySpecularStrength",
        1.0,
        0.0,
        10.0,
        "強度",
        "Strength",
    ),
    f(
        "_Anisotropy2ndTangentWidth",
        1.0,
        0.0,
        10.0,
        "タンジェント方向の幅",
        "Tangent Width",
    ),
    f(
        "_Anisotropy2ndBitangentWidth",
        1.0,
        0.0,
        10.0,
        "バイタンジェント方向の幅",
        "Bitangent Width",
    ),
    f("_Anisotropy2ndShift", 0.0, -10.0, 10.0, "Offset", "Offset"),
    f(
        "_Anisotropy2ndShiftNoiseScale",
        0.0,
        -1.0,
        1.0,
        "ノイズの強度",
        "Noise Strength",
    ),
    f(
        "_Anisotropy2ndSpecularStrength",
        0.0,
        0.0,
        10.0,
        "強度",
        "Strength",
    ),
    v("_AnisotropyShiftNoiseMask_ST", [1.0, 1.0, 0.0, 0.0]),
    t("_Anisotropy2MatCap", 0.0, "マットキャップ", "MatCap"),
    t(
        "_Anisotropy2MatCap2nd",
        0.0,
        "マットキャップ2nd",
        "MatCap 2nd",
    ),
];

const BACKLIGHT: &[Prop] = &[
    t("_UseBacklight", 0.0, "逆光ライト", "Backlight"),
    c(
        "_BacklightColor",
        [0.85, 0.8, 0.7, 1.0],
        true,
        "色",
        "Color",
    ),
    v("_BacklightColorTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f(
        "_BacklightMainStrength",
        0.0,
        0.0,
        1.0,
        "メインカラーの強度",
        "Main Color Power",
    ),
    t(
        "_BacklightReceiveShadow",
        1.0,
        "影を受け取る",
        "Receive Shadow",
    ),
    t(
        "_BacklightBackfaceMask",
        1.0,
        "裏面で無効化",
        "Backface Mask",
    ),
    f(
        "_BacklightNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f("_BacklightBorder", 0.35, 0.0, 1.0, "範囲", "Border"),
    f("_BacklightBlur", 0.05, 0.0, 1.0, "ぼかし", "Blur"),
    n(
        "_BacklightDirectivity",
        5.0,
        0.0,
        20.0,
        "指向性",
        "Directivity",
    ),
    f(
        "_BacklightViewStrength",
        1.0,
        0.0,
        1.0,
        "視線方向の影響度",
        "View direction strength",
    ),
];

const REFLECTION: &[Prop] = &[
    t("_UseReflection", 0.0, "反射", "Reflection"),
    f("_Smoothness", 1.0, 0.0, 1.0, "滑らかさ", "Smoothness"),
    v("_SmoothnessTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_GSAAStrength", 0.0, 0.0, 1.0, "GSAA", "GSAA"),
    f("_Metallic", 0.0, 0.0, 1.0, "金属度", "Metallic"),
    v("_MetallicGlossMap_ST", [1.0, 1.0, 0.0, 0.0]),
    c("_ReflectionColor", [1.0; 4], true, "色", "Color"),
    v("_ReflectionColorTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_Reflectance", 0.04, 0.0, 1.0, "反射率", "Reflectance"),
    t("_ApplySpecular", 1.0, "Apply Specular", "Apply Specular"),
    t("_SpecularToon", 1.0, "Specular Toon", "Specular Toon"),
    f(
        "_SpecularNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    f("_SpecularBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_SpecularBlur", 0.0, 0.0, 1.0, "ぼかし", "Blur"),
    t(
        "_ApplyReflection",
        0.0,
        "環境光の反射",
        "Environment Reflections",
    ),
    f(
        "_ReflectionNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    t(
        "_ReflectionApplyTransparency",
        1.0,
        "透明度を適用",
        "Apply Transparency",
    ),
    e(
        "_ReflectionBlendMode",
        1.0,
        BLEND,
        "合成モード",
        "Blending Mode",
    ),
];

/// マットキャップ（1st・2nd）の同じ形の値。
macro_rules! matcap {
    ($m:literal, $ja:literal, $en:literal) => {
        [
            t(concat!("_Use", $m), 0.0, $ja, $en),
            c(concat!("_", $m, "Color"), [1.0; 4], true, "色", "Color"),
            v(concat!("_", $m, "Tex_ST"), [1.0, 1.0, 0.0, 0.0]),
            t(
                concat!("_", $m, "ZRotCancel"),
                1.0,
                "Z軸回転キャンセル",
                "Z-axis rotation cancellation",
            ),
            t(
                concat!("_", $m, "Perspective"),
                1.0,
                "パース補正",
                "Fix Perspective",
            ),
            f(
                concat!("_", $m, "MainStrength"),
                0.0,
                0.0,
                1.0,
                "メインカラーの強度",
                "Main Color Power",
            ),
            f(
                concat!("_", $m, "NormalStrength"),
                1.0,
                0.0,
                1.0,
                "ノーマルマップ強度",
                "Normal Map Strength",
            ),
            f(concat!("_", $m, "Blend"), 1.0, 0.0, 1.0, "Blend", "Blend"),
            v(concat!("_", $m, "BlendMask_ST"), [1.0, 1.0, 0.0, 0.0]),
            f(
                concat!("_", $m, "EnableLighting"),
                1.0,
                0.0,
                1.0,
                "ライトの明るさを反映",
                "Enable Lighting",
            ),
            f(
                concat!("_", $m, "ShadowMask"),
                0.0,
                0.0,
                1.0,
                "影部分で無効化",
                "Shadow Mask",
            ),
            t(
                concat!("_", $m, "BackfaceMask"),
                0.0,
                "裏面で無効化",
                "Backface Mask",
            ),
            f(concat!("_", $m, "Lod"), 0.0, 0.0, 10.0, "ぼかし", "Blur"),
            e(
                concat!("_", $m, "BlendMode"),
                1.0,
                BLEND,
                "合成モード",
                "Blending Mode",
            ),
            t(
                concat!("_", $m, "ApplyTransparency"),
                1.0,
                "透明度を適用",
                "Apply Transparency",
            ),
            t(
                concat!("_", $m, "CustomNormal"),
                0.0,
                "カスタムノーマルマップ",
                "Custom normal map",
            ),
            v(concat!("_", $m, "BumpMap_ST"), [1.0, 1.0, 0.0, 0.0]),
            f(
                concat!("_", $m, "BumpScale"),
                1.0,
                -10.0,
                10.0,
                "Scale",
                "Scale",
            ),
        ]
    };
}

const MATCAP1: [Prop; 18] = matcap!("MatCap", "マットキャップ", "MatCap");
const MATCAP2: [Prop; 18] = matcap!("MatCap2nd", "マットキャップ2nd", "MatCap 2nd");

const RIM: &[Prop] = &[
    t("_UseRim", 0.0, "リムライト", "Rim Light"),
    c("_RimColor", [0.66, 0.5, 0.48, 1.0], true, "色", "Color"),
    v("_RimColorTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f(
        "_RimMainStrength",
        0.0,
        0.0,
        1.0,
        "メインカラーの強度",
        "Main Color Power",
    ),
    f(
        "_RimEnableLighting",
        1.0,
        0.0,
        1.0,
        "ライトの明るさを反映",
        "Enable Lighting",
    ),
    f(
        "_RimShadowMask",
        0.5,
        0.0,
        1.0,
        "影部分で無効化",
        "Shadow Mask",
    ),
    t("_RimBackfaceMask", 1.0, "裏面で無効化", "Backface Mask"),
    t(
        "_RimApplyTransparency",
        1.0,
        "透明度を適用",
        "Apply Transparency",
    ),
    e("_RimBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    f(
        "_RimDirStrength",
        0.0,
        0.0,
        1.0,
        "ライト方向の影響度",
        "Light direction strength",
    ),
    f(
        "_RimDirRange",
        0.0,
        -1.0,
        1.0,
        "直接光の幅",
        "Direct light width",
    ),
    f("_RimBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_RimBlur", 0.65, 0.0, 1.0, "ぼかし", "Blur"),
    f(
        "_RimIndirRange",
        0.0,
        -1.0,
        1.0,
        "間接光の幅",
        "Indirect light width",
    ),
    c("_RimIndirColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    f("_RimIndirBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_RimIndirBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f(
        "_RimNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    p3(
        "_RimFresnelPower",
        3.5,
        0.01,
        50.0,
        "リムライトの細さ",
        "Fresnel Power",
    ),
];

const GLITTER: &[Prop] = &[
    t("_UseGlitter", 0.0, "ラメ", "Glitter"),
    e("_GlitterUVMode", 0.0, UV2, "UV Mode", "UV Mode"),
    c("_GlitterColor", [1.0; 4], true, "色", "Color"),
    v("_GlitterColorTex_ST", [1.0, 1.0, 0.0, 0.0]),
    e("_GlitterColorTex_UVMode", 0.0, UV4, "UV Mode", "UV Mode"),
    f(
        "_GlitterMainStrength",
        0.0,
        0.0,
        1.0,
        "メインカラーの強度",
        "Main Color Power",
    ),
    f(
        "_GlitterEnableLighting",
        1.0,
        0.0,
        1.0,
        "ライトの明るさを反映",
        "Enable Lighting",
    ),
    f(
        "_GlitterShadowMask",
        0.0,
        0.0,
        1.0,
        "影部分で無効化",
        "Shadow Mask",
    ),
    t("_GlitterBackfaceMask", 0.0, "裏面で無効化", "Backface Mask"),
    t(
        "_GlitterApplyTransparency",
        1.0,
        "透明度を適用",
        "Apply Transparency",
    ),
    v("_GlitterParams1", [256.0, 256.0, 0.16, 50.0]),
    f(
        "_GlitterScaleRandomize",
        0.0,
        0.0,
        1.0,
        "ランダム化 (Size)",
        "Randomize (Size)",
    ),
    n(
        "_GlitterSensitivity",
        0.25,
        0.25,
        100.0,
        "感度",
        "Sensitivity",
    ),
    v("_GlitterParams2", [0.25, 0.0, 0.0, 0.0]),
    f(
        "_GlitterNormalStrength",
        1.0,
        0.0,
        1.0,
        "ノーマルマップ強度",
        "Normal Map Strength",
    ),
    n(
        "_GlitterPostContrast",
        1.0,
        0.0,
        10.0,
        "コントラスト（後処理）",
        "Post Contrast",
    ),
];

const OUTLINE: &[Prop] = &[
    c("_OutlineColor", [0.6, 0.56, 0.73, 1.0], true, "色", "Color"),
    v("_OutlineTex_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_OutlineTex_ScrollRotate", [0.0; 4]),
    v("_OutlineTexHSVG", [0.0, 1.0, 1.0, 1.0]),
    c(
        "_OutlineLitColor",
        [1.0, 0.2, 0.0, 0.0],
        true,
        "色",
        "Color",
    ),
    t(
        "_OutlineLitApplyTex",
        0.0,
        "メインカラーから色を取得",
        "Get color from Main",
    ),
    n("_OutlineLitScale", 10.0, -20.0, 20.0, "Scale", "Scale"),
    n("_OutlineLitOffset", -8.0, -20.0, 20.0, "Offset", "Offset"),
    t(
        "_OutlineLitShadowReceive",
        0.0,
        "影を受け取る",
        "Receive Shadow",
    ),
    f(
        "_OutlineEnableLighting",
        1.0,
        0.0,
        1.0,
        "ライトの明るさを反映",
        "Enable Lighting",
    ),
    f(
        "_OutlineWidth",
        0.08,
        0.0,
        1.0,
        "マスクと太さ",
        "Mask & Width",
    ),
    f(
        "_OutlineFixWidth",
        0.5,
        0.0,
        1.0,
        "距離に応じた太さ補正",
        "Fix width by distance",
    ),
    e(
        "_OutlineVertexR2Width",
        0.0,
        OUTLINE_VERTEX,
        "頂点カラー",
        "Vertex Color",
    ),
    t(
        "_OutlineDeleteMesh",
        0.0,
        "太さ0の頂点を削除",
        "Remove zero width vertices",
    ),
    n("_OutlineZBias", 0.0, -1.0, 1.0, "Z Bias", "Z Bias"),
];

const DISTANCE_FADE: &[Prop] = &[
    c(
        "_DistanceFadeColor",
        [0.0, 0.0, 0.0, 1.0],
        true,
        "色",
        "Color",
    ),
    v("_DistanceFade", [0.1, 0.01, 0.0, 0.0]),
    e(
        "_DistanceFadeMode",
        0.0,
        DISTANCE_FADE_MODE,
        "モード",
        "Mode",
    ),
    c("_DistanceFadeRimColor", [0.0; 4], true, "色", "Color"),
    p3(
        "_DistanceFadeRimFresnelPower",
        5.0,
        0.01,
        50.0,
        "リムライトの細さ",
        "Fresnel Power",
    ),
];

/// 節ごとのプロパティ（インスペクターの節の並び。既定に戻す単位）。
const GROUPS: &[(Section, &[Prop])] = &[
    (Section::Base, BASE),
    (Section::Lighting, LIGHTING),
    (Section::Uv, UV),
    (Section::Main, MAIN),
    (Section::Main, &MAIN2ND),
    (Section::Main, &MAIN3RD),
    (Section::Shadow, SHADOW),
    (Section::RimShade, RIM_SHADE),
    (Section::Emission, EMISSION),
    (Section::Normal, NORMAL),
    (Section::Backlight, BACKLIGHT),
    (Section::Reflection, REFLECTION),
    (Section::MatCap, &MATCAP1),
    (Section::MatCap, &MATCAP2),
    (Section::Rim, RIM),
    (Section::Glitter, GLITTER),
    (Section::Outline, OUTLINE),
    (Section::DistanceFade, DISTANCE_FADE),
];

/// 再現するプロパティの全部（節の並び）。
pub fn props() -> impl Iterator<Item = &'static Prop> {
    GROUPS.iter().flat_map(|(_, p)| p.iter())
}

/// 節のプロパティの名前（既定に戻すと設定から外す名前）。影色への環境光影響度は、インスペクターと同じくライティングの節と影設定の
/// 両方に出るが、値の持ち主はライティングの節。
pub fn section_props(section: Section) -> impl Iterator<Item = &'static str> {
    GROUPS
        .iter()
        .filter(move |(s, _)| *s == section)
        .flat_map(|(_, p)| p.iter().map(|p| p.name))
}

/// Unity の `[HDR]` の色（Unity はマテリアルの値をリニアのまま渡す。ほかの色はガンマの値を sRGB → リニアにして渡す）。lilToon では
/// 発光の色の 2 つ（`[lilHDR]` だけの色は普通の色と同じ）。
pub fn is_linear_color(name: &str) -> bool {
    matches!(name, "_EmissionColor" | "_Emission2ndColor")
}

/// プロパティの表から名前で（無ければ None）。
pub fn prop(name: &str) -> Option<&'static Prop> {
    props().find(|p| p.name == name)
}

/// プロパティの値（設定に無ければ既定）。4 つの値（数は x）。表に無い名前は 0。
pub fn value(look: &MaterialLook, name: &str) -> [f32; 4] {
    let default = prop(name).map_or([0.0; 4], |p| p.default);
    look.vec4(name, default)
}

/// プロパティの数の値（設定に無ければ既定）。
pub fn number(look: &MaterialLook, name: &str) -> f32 {
    value(look, name)[0]
}

/// 入切のプロパティが入か。
pub fn on(look: &MaterialLook, name: &str) -> bool {
    number(look, name) > 0.5
}

/// テクスチャのスロットの既定（Unity の `"white"`・`"black"`・`"bump"`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotDefault {
    White,
    Black,
    Bump,
}

impl SlotDefault {
    /// 既定の RGBA（Unity の既定のテクスチャの値。`"bump"` は `Texture2D.normalTexture` と同じ (127, 127, 255, 255) / 255: 法線は
    /// ほぼ平らで、XY がわずかに負。異方性反射の接線のマップを割り当てないと、lilToon はこのわずかな傾きから接線を斜めに作る）。
    pub fn rgba(self) -> [f32; 4] {
        match self {
            SlotDefault::White => [1.0; 4],
            SlotDefault::Black => [0.0; 4],
            SlotDefault::Bump => [127.0 / 255.0, 127.0 / 255.0, 1.0, 1.0],
        }
    }
}

/// スロットの読み方（書き出しの詰め方と、割り当ての候補）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotUse {
    /// 色（sRGB のチャンネル。RGBA）。
    Color,
    /// マスク（スカラー。R、または RGB の成分ごとに意味がある）。
    Mask,
    /// 法線。
    Normal,
    /// 絵（マットキャップ。プロジェクトの画像）。
    Image,
}

/// テクスチャのスロット。
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub name: &'static str,
    pub default: SlotDefault,
    pub usage: SlotUse,
    pub ja: &'static str,
    pub en: &'static str,
    /// 書き出しのファイルの名前の末尾（`<名前>_<末尾>.png`）。
    pub suffix: &'static str,
}

impl Slot {
    pub fn label(&self, lang: Lang) -> &'static str {
        lang.pick(self.ja, self.en)
    }
}

const fn s(
    name: &'static str,
    default: SlotDefault,
    usage: SlotUse,
    ja: &'static str,
    en: &'static str,
    suffix: &'static str,
) -> Slot {
    Slot {
        name,
        default,
        usage,
        ja,
        en,
        suffix,
    }
}

/// 再現するスロット（シェーダーへ渡す番号の順。`shaders/liltoon.wgsl` の `SLOT_*` と同じ並び）。並びは Live Link で受けた絵を
/// 3D ビューに持つ順（先の 16 枚）でもあるので、足すスロットは後ろに足す。
pub const SLOTS: &[Slot] = &[
    s(
        "_MainTex",
        SlotDefault::White,
        SlotUse::Color,
        "メインカラー",
        "Main Color",
        "Main",
    ),
    s(
        "_MainColorAdjustMask",
        SlotDefault::White,
        SlotUse::Mask,
        "色調補正のマスク",
        "Color Adjust Mask",
        "MainAdjustMask",
    ),
    s(
        "_AlphaMask",
        SlotDefault::White,
        SlotUse::Mask,
        "アルファマスク",
        "Alpha Mask",
        "AlphaMask",
    ),
    s(
        "_BumpMap",
        SlotDefault::Bump,
        SlotUse::Normal,
        "ノーマルマップ",
        "Normal Map",
        "Normal",
    ),
    s(
        "_ShadowStrengthMask",
        SlotDefault::White,
        SlotUse::Mask,
        "影の強度マスク",
        "Shadow Strength Mask",
        "ShadowStrengthMask",
    ),
    s(
        "_ShadowBorderMask",
        SlotDefault::White,
        SlotUse::Mask,
        "AO Map",
        "AO Map",
        "ShadowBorderMask",
    ),
    s(
        "_ShadowBlurMask",
        SlotDefault::White,
        SlotUse::Mask,
        "ぼかし量マスク",
        "Blur Mask",
        "ShadowBlurMask",
    ),
    s(
        "_ShadowColorTex",
        SlotDefault::Black,
        SlotUse::Color,
        "影色1",
        "1st Shadow Color",
        "ShadowColor",
    ),
    s(
        "_Shadow2ndColorTex",
        SlotDefault::Black,
        SlotUse::Color,
        "影色2",
        "2nd Shadow Color",
        "Shadow2ndColor",
    ),
    s(
        "_Shadow3rdColorTex",
        SlotDefault::Black,
        SlotUse::Color,
        "影色3",
        "3rd Shadow Color",
        "Shadow3rdColor",
    ),
    s(
        "_EmissionMap",
        SlotDefault::White,
        SlotUse::Color,
        "発光テクスチャ",
        "Emission",
        "Emission",
    ),
    s(
        "_EmissionBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "発光のマスク",
        "Emission Mask",
        "EmissionMask",
    ),
    s(
        "_Emission2ndMap",
        SlotDefault::White,
        SlotUse::Color,
        "発光テクスチャ2nd",
        "Emission 2nd",
        "Emission2nd",
    ),
    s(
        "_Emission2ndBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "発光2nd のマスク",
        "Emission 2nd Mask",
        "Emission2ndMask",
    ),
    s(
        "_MatCapTex",
        SlotDefault::White,
        SlotUse::Image,
        "マットキャップ",
        "MatCap",
        "MatCap",
    ),
    s(
        "_MatCapBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "マットキャップのマスク",
        "MatCap Mask",
        "MatCapMask",
    ),
    s(
        "_MatCap2ndTex",
        SlotDefault::White,
        SlotUse::Image,
        "マットキャップ2nd",
        "MatCap 2nd",
        "MatCap2nd",
    ),
    s(
        "_MatCap2ndBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "マットキャップ2nd のマスク",
        "MatCap 2nd Mask",
        "MatCap2ndMask",
    ),
    s(
        "_RimColorTex",
        SlotDefault::White,
        SlotUse::Color,
        "リムライトの色",
        "Rim Light Color",
        "RimColor",
    ),
    s(
        "_OutlineTex",
        SlotDefault::White,
        SlotUse::Color,
        "輪郭線の色",
        "Outline Color",
        "OutlineColor",
    ),
    s(
        "_OutlineWidthMask",
        SlotDefault::White,
        SlotUse::Mask,
        "輪郭線の太さ",
        "Outline Width",
        "OutlineWidth",
    ),
    s(
        "_Main2ndTex",
        SlotDefault::White,
        SlotUse::Color,
        "メインカラー2nd",
        "Main Color 2nd",
        "Main2nd",
    ),
    s(
        "_Main2ndBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "メインカラー2nd のマスク",
        "Main Color 2nd Mask",
        "Main2ndMask",
    ),
    s(
        "_Main3rdTex",
        SlotDefault::White,
        SlotUse::Color,
        "メインカラー3rd",
        "Main Color 3rd",
        "Main3rd",
    ),
    s(
        "_Main3rdBlendMask",
        SlotDefault::White,
        SlotUse::Mask,
        "メインカラー3rd のマスク",
        "Main Color 3rd Mask",
        "Main3rdMask",
    ),
    s(
        "_Bump2ndMap",
        SlotDefault::Bump,
        SlotUse::Normal,
        "ノーマルマップ2nd",
        "Normal Map 2nd",
        "Normal2nd",
    ),
    s(
        "_Bump2ndScaleMask",
        SlotDefault::White,
        SlotUse::Mask,
        "ノーマルマップ2nd のマスク",
        "Normal Map 2nd Mask",
        "Normal2ndMask",
    ),
    s(
        "_RimShadeMask",
        SlotDefault::White,
        SlotUse::Mask,
        "リムシェードのマスク",
        "RimShade Mask",
        "RimShadeMask",
    ),
    s(
        "_BacklightColorTex",
        SlotDefault::White,
        SlotUse::Color,
        "逆光ライトの色",
        "Backlight Color",
        "BacklightColor",
    ),
    s(
        "_SmoothnessTex",
        SlotDefault::White,
        SlotUse::Mask,
        "滑らかさ",
        "Smoothness",
        "Smoothness",
    ),
    s(
        "_MetallicGlossMap",
        SlotDefault::White,
        SlotUse::Mask,
        "金属度",
        "Metallic",
        "Metallic",
    ),
    s(
        "_ReflectionColorTex",
        SlotDefault::White,
        SlotUse::Color,
        "光沢の色",
        "Reflection Color",
        "ReflectionColor",
    ),
    s(
        "_GlitterColorTex",
        SlotDefault::White,
        SlotUse::Color,
        "ラメの色",
        "Glitter Color",
        "GlitterColor",
    ),
    s(
        "_AnisotropyTangentMap",
        SlotDefault::Bump,
        SlotUse::Normal,
        "異方性反射のノーマルマップ",
        "Anisotropy Tangent Map",
        "AnisotropyTangent",
    ),
    s(
        "_AnisotropyScaleMask",
        SlotDefault::White,
        SlotUse::Mask,
        "異方性反射のマスク",
        "Anisotropy Mask",
        "AnisotropyMask",
    ),
    s(
        "_AnisotropyShiftNoiseMask",
        SlotDefault::White,
        SlotUse::Mask,
        "異方性反射のノイズ",
        "Anisotropy Noise",
        "AnisotropyNoise",
    ),
    s(
        "_MatCapBumpMap",
        SlotDefault::Bump,
        SlotUse::Normal,
        "マットキャップのノーマルマップ",
        "MatCap Normal Map",
        "MatCapNormal",
    ),
    s(
        "_MatCap2ndBumpMap",
        SlotDefault::Bump,
        SlotUse::Normal,
        "マットキャップ2nd のノーマルマップ",
        "MatCap 2nd Normal Map",
        "MatCap2ndNormal",
    ),
];

/// スロットの番号（シェーダーの並び）。
pub fn slot_index(name: &str) -> Option<usize> {
    SLOTS.iter().position(|s| s.name == name)
}

pub fn slot(name: &str) -> Option<&'static Slot> {
    SLOTS.iter().find(|s| s.name == name)
}

/// 後から足したスロット（`SLOTS` の 22 番目から）を読む機能: 機能の入切のプロパティと、その機能が読むスロット。Unity のパッケージの
/// Live Link が送るスロットの一覧（`LiveLinkMaterialValues.Slots`）がこれらのスロットを知らない版でも機能の入切と値は届くので、受けた
/// 見た目は、スロットが知らされた機能だけを描く（`look::link::received_look`）。割り当ても絵も無いスロットは既定（白）で読むので、
/// 知らないまま描くと、Unity でデカールを置いたメインカラー 2nd が全面に重なる。
pub const FEATURES_OF_LATER_SLOTS: &[(&str, &[&str])] = &[
    ("_UseMain2ndTex", &["_Main2ndTex", "_Main2ndBlendMask"]),
    ("_UseMain3rdTex", &["_Main3rdTex", "_Main3rdBlendMask"]),
    ("_UseBump2ndMap", &["_Bump2ndMap", "_Bump2ndScaleMask"]),
    ("_UseRimShade", &["_RimShadeMask"]),
    ("_UseBacklight", &["_BacklightColorTex"]),
    (
        "_UseReflection",
        &["_SmoothnessTex", "_MetallicGlossMap", "_ReflectionColorTex"],
    ),
    ("_UseGlitter", &["_GlitterColorTex"]),
    (
        "_UseAnisotropy",
        &[
            "_AnisotropyTangentMap",
            "_AnisotropyScaleMask",
            "_AnisotropyShiftNoiseMask",
        ],
    ),
    ("_MatCapCustomNormal", &["_MatCapBumpMap"]),
    ("_MatCap2ndCustomNormal", &["_MatCap2ndBumpMap"]),
];

/// 描画モード。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderMode {
    Opaque,
    Cutout,
    Transparent,
}

impl RenderMode {
    pub const ALL: [RenderMode; 3] = [
        RenderMode::Opaque,
        RenderMode::Cutout,
        RenderMode::Transparent,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            RenderMode::Opaque => lang.pick("不透明", "Opaque"),
            RenderMode::Cutout => lang.pick("カットアウト", "Cutout"),
            RenderMode::Transparent => lang.pick("半透明", "Transparent"),
        }
    }
}

/// シェーダーの名前から分かること。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderInfo {
    pub mode: RenderMode,
    pub outline: bool,
    /// このシェーダーの見た目を再現の範囲で描けるか（Lite・テッセレーション・ファー・宝石・屈折・知らない名前は false。近い描き方で描く）。
    pub exact: bool,
}

/// シェーダーの名前を読む（lilToon 2.3 の名前。マルチの版は `_TransparentMode`・`_UseOutline` の値で決める）。
pub fn shader_info(look: &MaterialLook) -> ShaderInfo {
    let name = look.shader_name();
    let base = name
        .strip_prefix("Hidden/")
        .or_else(|| name.strip_prefix("_lil/"))
        .unwrap_or(name);
    if base == "lilToonMulti" || base == "lilToonMultiOutline" {
        let mode = match look.float("_TransparentMode", 0.0).round() as i32 {
            1 => RenderMode::Cutout,
            2 => RenderMode::Transparent,
            _ => RenderMode::Opaque,
        };
        let exact = matches!(look.float("_TransparentMode", 0.0).round() as i32, 0..=2);
        return ShaderInfo {
            mode,
            outline: base.ends_with("Outline"),
            exact,
        };
    }
    let Some(rest) = base.strip_prefix("lilToon") else {
        return ShaderInfo {
            mode: RenderMode::Opaque,
            outline: false,
            exact: false,
        };
    };
    let outline = rest.ends_with("Outline");
    let rest = rest.strip_suffix("Outline").unwrap_or(rest);
    let (rest, variant) = match rest {
        r if r.starts_with("Lite") => (&r[4..], false),
        r if r.starts_with("Tessellation") => (&r[12..], false),
        r => (r, true),
    };
    let (mode, known) = match rest {
        "" => (RenderMode::Opaque, true),
        "Cutout" => (RenderMode::Cutout, true),
        "Transparent" | "OnePassTransparent" | "TwoPassTransparent" => {
            (RenderMode::Transparent, true)
        }
        _ => (RenderMode::Opaque, false),
    };
    ShaderInfo {
        mode,
        outline,
        exact: variant && known && rest != "TwoPassTransparent",
    }
}

/// 描画モードと輪郭線からシェーダーの名前（lilToon の通常の版）。
pub fn shader_name(mode: RenderMode, outline: bool) -> String {
    let mode = match mode {
        RenderMode::Opaque => "",
        RenderMode::Cutout => "Cutout",
        RenderMode::Transparent => "Transparent",
    };
    if mode.is_empty() && !outline {
        return LILTOON_SHADER.to_owned();
    }
    format!(
        "Hidden/lilToon{mode}{}",
        if outline { "Outline" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::look::{LookKind, LookValue};

    fn with_shader(name: &str) -> MaterialLook {
        MaterialLook {
            kind: LookKind::LilToon,
            shader: name.into(),
            ..MaterialLook::default()
        }
    }

    #[test]
    fn shader_names_round_trip_through_mode_and_outline() {
        for mode in RenderMode::ALL {
            for outline in [false, true] {
                let name = shader_name(mode, outline);
                let info = shader_info(&with_shader(&name));
                assert_eq!(
                    (info.mode, info.outline, info.exact),
                    (mode, outline, true),
                    "{name}"
                );
            }
        }
        assert_eq!(shader_name(RenderMode::Opaque, false), "lilToon");
        assert_eq!(
            shader_name(RenderMode::Cutout, true),
            "Hidden/lilToonCutoutOutline"
        );
        // 空の名前は lilToon
        assert_eq!(shader_info(&with_shader("")).mode, RenderMode::Opaque);
    }

    #[test]
    fn every_later_slot_belongs_to_a_feature_with_a_known_toggle() {
        // 前の 21 のスロットは Unity のパッケージが前から送る。後から足したスロットは、どれも受けた見た目で切る機能に入っている
        let listed: Vec<&str> = FEATURES_OF_LATER_SLOTS
            .iter()
            .flat_map(|(_, s)| s.iter().copied())
            .collect();
        let later: Vec<&str> = SLOTS[21..].iter().map(|s| s.name).collect();
        let mut a = listed.clone();
        let mut b = later.clone();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        for (toggle, _) in FEATURES_OF_LATER_SLOTS {
            assert!(prop(toggle).is_some(), "{toggle}");
        }
    }

    #[test]
    fn variants_outside_the_scope_are_drawn_approximately() {
        let lite = shader_info(&with_shader("Hidden/lilToonLiteTransparentOutline"));
        assert_eq!(
            (lite.mode, lite.outline, lite.exact),
            (RenderMode::Transparent, true, false)
        );
        let fur = shader_info(&with_shader("Hidden/lilToonFur"));
        assert!(!fur.exact);
        let other = shader_info(&with_shader("Standard"));
        assert_eq!((other.mode, other.exact), (RenderMode::Opaque, false));
        let two = shader_info(&with_shader("Hidden/lilToonTwoPassTransparent"));
        assert_eq!((two.mode, two.exact), (RenderMode::Transparent, false));
        let mut multi = with_shader("_lil/lilToonMulti");
        multi
            .properties
            .insert("_TransparentMode".into(), LookValue::Float(1.0));
        let m = shader_info(&multi);
        assert_eq!(
            (m.mode, m.outline, m.exact),
            (RenderMode::Cutout, false, true)
        );
    }

    #[test]
    fn the_tables_have_unique_names_and_defaults_inside_their_ranges() {
        let mut names: Vec<&str> = props().map(|p| p.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "プロパティの名前が重なる");
        for p in props() {
            if let Kind::Slider { min, max, .. } = p.kind {
                assert!((min..=max).contains(&p.default[0]), "{}", p.name);
            }
            if let Kind::Choice(options) = p.kind {
                assert!((p.default[0] as usize) < options.len(), "{}", p.name);
            }
        }
        let mut slots: Vec<&str> = SLOTS.iter().map(|s| s.name).collect();
        slots.sort_unstable();
        let before = slots.len();
        slots.dedup();
        assert_eq!(slots.len(), before);
        assert_eq!(slot_index("_MainTex"), Some(0));
        // 前の版の並び（受けた絵を持つ順・書き出しの取り決め）は変えない
        assert_eq!(slot_index("_OutlineWidthMask"), Some(20));
        // 既定の値の読み出し
        let look = with_shader("lilToon");
        assert_eq!(number(&look, "_ShadowBorder"), 0.5);
        assert_eq!(value(&look, "_ShadowColor"), [0.82, 0.76, 0.85, 1.0]);
        assert!(!on(&look, "_UseShadow"));
        assert!(on(&look, "_RimBackfaceMask"));
        assert_eq!(number(&look, "_RimShadeFresnelPower"), 1.0);
        // 節の値: ライティングの節に影色への環境光影響度が入る、メインカラーの節に 2nd・3rd
        assert!(section_props(Section::Lighting).any(|n| n == "_ShadowEnvStrength"));
        assert!(section_props(Section::Main).any(|n| n == "_Main3rdTexAngle"));
        assert!(section_props(Section::RimShade)
            .all(|n| n.starts_with("_UseRimShade") || n.starts_with("_RimShade")));
    }
}
