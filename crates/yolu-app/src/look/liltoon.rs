//! lilToon（2.3.4、MIT・Copyright (c) 2020-present lilxyzw）のマテリアルの表: 3D ビューで再現するプロパティの名前・既定・範囲と、
//! テクスチャのスロット、シェーダーの名前と描画モードの対応。既定と範囲はシェーダーの `Properties`（`Shader/lts_o.shader`）の値、
//! 欄の並びと名前の付け方はインスペクター（`Editor/lilInspector/*.cs` と `Editor/Localization/ja-JP.po`）に合わせる。
//!
//! 値は [`yolu_core::look::MaterialLook`] にプロパティの名前で持ち、無い名前はここの既定で描く。ここに無い名前（ファー・宝石・
//! AudioLink など、再現しない機能の値や Live Link で受けた値）も設定には残り、描かないだけ。

use yolu_core::look::{MaterialLook, LILTOON_SHADER};

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

    pub fn is_color(&self) -> bool {
        matches!(self.kind, Kind::Color { .. })
    }
}

const BLEND: &[(&str, &str)] = &[("通常", "Normal"), ("加算", "Add"), ("スクリーン", "Screen"), ("乗算", "Multiply")];
const CULL: &[(&str, &str)] = &[
    ("Off（両面を描画）", "Off"),
    ("Front（裏面のみ描画）", "Front"),
    ("Back（前面のみ描画）", "Back"),
];
const ALPHA_MASK: &[(&str, &str)] = &[
    ("なし", "None"),
    ("置き換え", "Replace"),
    ("乗算", "Multiply"),
    ("加算", "Add"),
    ("減算", "Subtract"),
];
const SHADOW_MASK: &[(&str, &str)] = &[("強度", "Strength"), ("平面", "Flat"), ("SDF", "SDF")];
const SHADOW_COLOR_TYPE: &[(&str, &str)] = &[("通常", "Normal"), ("LUT", "LUT")];
const EMISSION_UV: &[(&str, &str)] = &[("UV0", "UV0"), ("UV1", "UV1"), ("UV2", "UV2"), ("UV3", "UV3"), ("リム", "Rim")];
const OUTLINE_VERTEX: &[(&str, &str)] = &[
    ("なし", "None"),
    ("R を太さに", "R as width"),
    ("RGBA を法線と太さに", "RGBA as normal & width"),
];

const fn f(name: &'static str, default: f32, min: f32, max: f32, ja: &'static str, en: &'static str) -> Prop {
    Prop {
        name,
        default: [default, 0.0, 0.0, 0.0],
        kind: Kind::Slider { min, max, power: 1.0 },
        ja,
        en,
    }
}

const fn n(name: &'static str, default: f32, min: f32, max: f32, ja: &'static str, en: &'static str) -> Prop {
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

const fn c(name: &'static str, default: [f32; 4], hdr: bool, ja: &'static str, en: &'static str) -> Prop {
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
        kind: Kind::Number { min: -100.0, max: 100.0 },
        ja: name,
        en: name,
    }
}

/// 再現するプロパティ（シェーダーの `Properties` の並び）。
pub const PROPS: &[Prop] = &[
    // 基本
    f("_Cutoff", 0.5, -0.001, 1.001, "Cutoff", "Cutoff"),
    e("_Cull", 2.0, CULL, "Cull Mode", "Cull Mode"),
    t("_FlipNormal", 0.0, "裏面の法線を反転", "Flip Backface Normal"),
    f("_BackfaceForceShadow", 0.0, 0.0, 1.0, "裏面を影にする", "Backface Force Shadow"),
    c("_BackfaceColor", [0.0, 0.0, 0.0, 0.0], true, "裏面の色", "Backface Color"),
    t("_Invisible", 0.0, "非表示", "Invisible"),
    // ライティング
    f("_LightMinLimit", 0.05, 0.0, 1.0, "明るさの下限", "Lower Brightness Limit"),
    f("_LightMaxLimit", 1.0, 0.0, 10.0, "明るさの上限", "Upper Brightness Limit"),
    f("_MonochromeLighting", 0.0, 0.0, 1.0, "ライトのモノクロ化", "Monochrome Lighting"),
    f("_ShadowEnvStrength", 0.0, 0.0, 1.0, "影色への環境光影響度", "Environment Strength on Shadow"),
    f("_AsUnlit", 0.0, 0.0, 1.0, "Unlit 化", "As Unlit"),
    f("_AAStrength", 1.0, 0.0, 1.0, "アンチエイリアスシェーディング", "Anti-aliasing Shading"),
    v("_LightDirectionOverride", [0.001, 0.002, 0.001, 0.0]),
    // メインカラー
    c("_Color", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_MainTex_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_MainTexHSVG", [0.0, 1.0, 1.0, 1.0]),
    e("_AlphaMaskMode", 0.0, ALPHA_MASK, "アルファマスク", "Alpha Mask"),
    n("_AlphaMaskScale", 1.0, -1.0, 1.0, "スケール", "Scale"),
    n("_AlphaMaskValue", 0.0, -1.0, 1.0, "オフセット", "Offset"),
    // 影
    t("_UseShadow", 0.0, "影", "Shadow"),
    f("_ShadowStrength", 1.0, 0.0, 1.0, "強度", "Strength"),
    f("_ShadowStrengthMaskLOD", 0.0, 0.0, 1.0, "マスクのぼかし", "Mask LOD"),
    f("_ShadowBorderMaskLOD", 0.0, 0.0, 1.0, "AO のぼかし", "AO LOD"),
    f("_ShadowBlurMaskLOD", 0.0, 0.0, 1.0, "ぼかしマスクのぼかし", "Blur Mask LOD"),
    v("_ShadowAOShift", [1.0, 0.0, 1.0, 0.0]),
    v("_ShadowAOShift2", [1.0, 0.0, 1.0, 0.0]),
    t("_ShadowPostAO", 0.0, "影範囲設定を無視して適用", "Ignore Border Properties"),
    e("_ShadowColorType", 0.0, SHADOW_COLOR_TYPE, "影色の種類", "Shadow Color Type"),
    c("_ShadowColor", [0.82, 0.76, 0.85, 1.0], false, "影色", "Shadow Color"),
    f("_ShadowNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    f("_ShadowBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_ShadowBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f("_ShadowReceive", 0.0, 0.0, 1.0, "影を受け取る", "Receive Shadow"),
    c("_Shadow2ndColor", [0.68, 0.66, 0.79, 1.0], false, "2影色", "2nd Color"),
    f("_Shadow2ndNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    f("_Shadow2ndBorder", 0.15, 0.0, 1.0, "範囲", "Border"),
    f("_Shadow2ndBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f("_Shadow2ndReceive", 0.0, 0.0, 1.0, "影を受け取る", "Receive Shadow"),
    c("_Shadow3rdColor", [0.0, 0.0, 0.0, 0.0], false, "3影色", "3rd Color"),
    f("_Shadow3rdNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    f("_Shadow3rdBorder", 0.25, 0.0, 1.0, "範囲", "Border"),
    f("_Shadow3rdBlur", 0.1, 0.0, 1.0, "ぼかし", "Blur"),
    f("_Shadow3rdReceive", 0.0, 0.0, 1.0, "影を受け取る", "Receive Shadow"),
    c("_ShadowBorderColor", [1.0, 0.1, 0.0, 1.0], false, "境界の色", "Border Color"),
    f("_ShadowBorderRange", 0.08, 0.0, 1.0, "境界の幅", "Border Range"),
    f("_ShadowMainStrength", 0.0, 0.0, 1.0, "コントラスト", "Contrast"),
    e("_ShadowMaskType", 0.0, SHADOW_MASK, "マスクの種類", "Mask Type"),
    f("_ShadowFlatBorder", 1.0, -2.0, 2.0, "範囲", "Border"),
    f("_ShadowFlatBlur", 1.0, 0.001, 2.0, "ぼかし", "Blur"),
    // 発光
    t("_UseEmission", 0.0, "発光", "Emission"),
    c("_EmissionColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_EmissionMap_ST", [1.0, 1.0, 0.0, 0.0]),
    e("_EmissionMap_UVMode", 0.0, EMISSION_UV, "UV", "UV Mode"),
    f("_EmissionMainStrength", 0.0, 0.0, 1.0, "メインカラーの強度", "Main Color Power"),
    f("_EmissionBlend", 1.0, 0.0, 1.0, "強度", "Blend"),
    v("_EmissionBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    e("_EmissionBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    f("_EmissionFluorescence", 0.0, 0.0, 1.0, "蛍光", "Fluorescence"),
    t("_UseEmission2nd", 0.0, "発光 2nd", "Emission 2nd"),
    c("_Emission2ndColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_Emission2ndMap_ST", [1.0, 1.0, 0.0, 0.0]),
    e("_Emission2ndMap_UVMode", 0.0, EMISSION_UV, "UV", "UV Mode"),
    f("_Emission2ndMainStrength", 0.0, 0.0, 1.0, "メインカラーの強度", "Main Color Power"),
    f("_Emission2ndBlend", 1.0, 0.0, 1.0, "強度", "Blend"),
    v("_Emission2ndBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    e("_Emission2ndBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    f("_Emission2ndFluorescence", 0.0, 0.0, 1.0, "蛍光", "Fluorescence"),
    // ノーマルマップ
    t("_UseBumpMap", 0.0, "ノーマルマップ", "Normal Map"),
    v("_BumpMap_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_BumpScale", 1.0, -10.0, 10.0, "強度", "Scale"),
    // マットキャップ
    t("_UseMatCap", 0.0, "マットキャップ", "MatCap"),
    c("_MatCapColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_MatCapTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_MatCapMainStrength", 0.0, 0.0, 1.0, "メインカラーの強度", "Main Color Power"),
    t("_MatCapZRotCancel", 1.0, "Z 軸回転キャンセル", "Z-axis Rotation Cancellation"),
    t("_MatCapPerspective", 1.0, "パース補正", "Fix Perspective"),
    f("_MatCapBlend", 1.0, 0.0, 1.0, "強度", "Blend"),
    v("_MatCapBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_MatCapEnableLighting", 1.0, 0.0, 1.0, "ライトの明るさを反映", "Enable Lighting"),
    f("_MatCapShadowMask", 0.0, 0.0, 1.0, "影部分で無効化", "Shadow Mask"),
    t("_MatCapBackfaceMask", 0.0, "裏面で無効化", "Backface Mask"),
    f("_MatCapLod", 0.0, 0.0, 10.0, "ぼかし", "Blur"),
    e("_MatCapBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    t("_MatCapApplyTransparency", 1.0, "透明度を適用", "Apply Transparency"),
    f("_MatCapNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    t("_UseMatCap2nd", 0.0, "マットキャップ 2nd", "MatCap 2nd"),
    c("_MatCap2ndColor", [1.0, 1.0, 1.0, 1.0], true, "色", "Color"),
    v("_MatCap2ndTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_MatCap2ndMainStrength", 0.0, 0.0, 1.0, "メインカラーの強度", "Main Color Power"),
    t("_MatCap2ndZRotCancel", 1.0, "Z 軸回転キャンセル", "Z-axis Rotation Cancellation"),
    t("_MatCap2ndPerspective", 1.0, "パース補正", "Fix Perspective"),
    f("_MatCap2ndBlend", 1.0, 0.0, 1.0, "強度", "Blend"),
    v("_MatCap2ndBlendMask_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_MatCap2ndEnableLighting", 1.0, 0.0, 1.0, "ライトの明るさを反映", "Enable Lighting"),
    f("_MatCap2ndShadowMask", 0.0, 0.0, 1.0, "影部分で無効化", "Shadow Mask"),
    t("_MatCap2ndBackfaceMask", 0.0, "裏面で無効化", "Backface Mask"),
    f("_MatCap2ndLod", 0.0, 0.0, 10.0, "ぼかし", "Blur"),
    e("_MatCap2ndBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    t("_MatCap2ndApplyTransparency", 1.0, "透明度を適用", "Apply Transparency"),
    f("_MatCap2ndNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    // リムライト
    t("_UseRim", 0.0, "リムライト", "Rim Light"),
    c("_RimColor", [0.66, 0.5, 0.48, 1.0], true, "色", "Color"),
    v("_RimColorTex_ST", [1.0, 1.0, 0.0, 0.0]),
    f("_RimMainStrength", 0.0, 0.0, 1.0, "メインカラーの強度", "Main Color Power"),
    f("_RimNormalStrength", 1.0, 0.0, 1.0, "ノーマルマップ強度", "Normal Map Strength"),
    f("_RimBorder", 0.5, 0.0, 1.0, "範囲", "Border"),
    f("_RimBlur", 0.65, 0.0, 1.0, "ぼかし", "Blur"),
    Prop {
        name: "_RimFresnelPower",
        default: [3.5, 0.0, 0.0, 0.0],
        kind: Kind::Slider { min: 0.01, max: 50.0, power: 3.0 },
        ja: "リムライトの細さ",
        en: "Fresnel Power",
    },
    f("_RimEnableLighting", 1.0, 0.0, 1.0, "ライトの明るさを反映", "Enable Lighting"),
    f("_RimShadowMask", 0.5, 0.0, 1.0, "影部分で無効化", "Shadow Mask"),
    t("_RimBackfaceMask", 1.0, "裏面で無効化", "Backface Mask"),
    t("_RimApplyTransparency", 1.0, "透明度を適用", "Apply Transparency"),
    f("_RimDirStrength", 0.0, 0.0, 1.0, "ライト方向の影響度", "Light Direction Strength"),
    f("_RimDirRange", 0.0, -1.0, 1.0, "直接光の幅", "Direct Light Width"),
    f("_RimIndirRange", 0.0, -1.0, 1.0, "間接光の幅", "Indirect Light Width"),
    c("_RimIndirColor", [1.0, 1.0, 1.0, 1.0], true, "間接光の色", "Indirect Light Color"),
    f("_RimIndirBorder", 0.5, 0.0, 1.0, "間接光の範囲", "Indirect Border"),
    f("_RimIndirBlur", 0.1, 0.0, 1.0, "間接光のぼかし", "Indirect Blur"),
    e("_RimBlendMode", 1.0, BLEND, "合成モード", "Blending Mode"),
    // 輪郭線
    c("_OutlineColor", [0.6, 0.56, 0.73, 1.0], true, "色", "Color"),
    v("_OutlineTex_ST", [1.0, 1.0, 0.0, 0.0]),
    v("_OutlineTexHSVG", [0.0, 1.0, 1.0, 1.0]),
    c("_OutlineLitColor", [1.0, 0.2, 0.0, 0.0], true, "ライトの色", "Lit Color"),
    t("_OutlineLitApplyTex", 0.0, "メインカラーから色を取得", "Get Color from Main"),
    n("_OutlineLitScale", 10.0, -20.0, 20.0, "スケール", "Scale"),
    n("_OutlineLitOffset", -8.0, -20.0, 20.0, "オフセット", "Offset"),
    t("_OutlineLitShadowReceive", 0.0, "影を受け取る", "Receive Shadow"),
    f("_OutlineWidth", 0.08, 0.0, 1.0, "太さ", "Width"),
    f("_OutlineFixWidth", 0.5, 0.0, 1.0, "距離に応じた太さ補正", "Fix Width by Distance"),
    e("_OutlineVertexR2Width", 0.0, OUTLINE_VERTEX, "頂点カラー", "Vertex Color"),
    t("_OutlineDeleteMesh", 0.0, "太さ 0 の頂点を削除", "Remove Zero-width Vertices"),
    f("_OutlineEnableLighting", 1.0, 0.0, 1.0, "ライトの明るさを反映", "Enable Lighting"),
    n("_OutlineZBias", 0.0, -1.0, 1.0, "Z バイアス", "Z Bias"),
];

/// Unity の `[HDR]` の色（Unity はマテリアルの値をリニアのまま渡す。ほかの色はガンマの値を sRGB → リニアにして渡す）。lilToon では
/// 発光の色の 2 つ（`[lilHDR]` だけの色は普通の色と同じ）。
pub fn is_linear_color(name: &str) -> bool {
    matches!(name, "_EmissionColor" | "_Emission2ndColor")
}

/// プロパティの表から名前で（無ければ None）。
pub fn prop(name: &str) -> Option<&'static Prop> {
    PROPS.iter().find(|p| p.name == name)
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
    /// 既定の RGBA（Unity の既定のテクスチャの値）。
    pub fn rgba(self) -> [f32; 4] {
        match self {
            SlotDefault::White => [1.0; 4],
            SlotDefault::Black => [0.0; 4],
            SlotDefault::Bump => [0.5, 0.5, 1.0, 0.5],
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

/// 再現するスロット（シェーダーへ渡す番号の順。`shaders/liltoon.wgsl` の `SLOT_*` と同じ並び）。
pub const SLOTS: &[Slot] = &[
    s("_MainTex", SlotDefault::White, SlotUse::Color, "メインカラー", "Main Color", "Main"),
    s("_MainColorAdjustMask", SlotDefault::White, SlotUse::Mask, "色調補正マスク", "Adjust Mask", "MainAdjustMask"),
    s("_AlphaMask", SlotDefault::White, SlotUse::Mask, "アルファマスク", "Alpha Mask", "AlphaMask"),
    s("_BumpMap", SlotDefault::Bump, SlotUse::Normal, "ノーマルマップ", "Normal Map", "Normal"),
    s("_ShadowStrengthMask", SlotDefault::White, SlotUse::Mask, "影の強度マスク", "Shadow Strength Mask", "ShadowStrengthMask"),
    s("_ShadowBorderMask", SlotDefault::White, SlotUse::Mask, "AO マップ", "AO Map", "ShadowBorderMask"),
    s("_ShadowBlurMask", SlotDefault::White, SlotUse::Mask, "ぼかしマスク", "Blur Mask", "ShadowBlurMask"),
    s("_ShadowColorTex", SlotDefault::Black, SlotUse::Color, "影色", "Shadow Color", "ShadowColor"),
    s("_Shadow2ndColorTex", SlotDefault::Black, SlotUse::Color, "2影色", "2nd Shadow Color", "Shadow2ndColor"),
    s("_Shadow3rdColorTex", SlotDefault::Black, SlotUse::Color, "3影色", "3rd Shadow Color", "Shadow3rdColor"),
    s("_EmissionMap", SlotDefault::White, SlotUse::Color, "発光", "Emission", "Emission"),
    s("_EmissionBlendMask", SlotDefault::White, SlotUse::Mask, "発光のマスク", "Emission Mask", "EmissionMask"),
    s("_Emission2ndMap", SlotDefault::White, SlotUse::Color, "発光 2nd", "Emission 2nd", "Emission2nd"),
    s("_Emission2ndBlendMask", SlotDefault::White, SlotUse::Mask, "発光 2nd のマスク", "Emission 2nd Mask", "Emission2ndMask"),
    s("_MatCapTex", SlotDefault::White, SlotUse::Image, "マットキャップ", "MatCap", "MatCap"),
    s("_MatCapBlendMask", SlotDefault::White, SlotUse::Mask, "マットキャップのマスク", "MatCap Mask", "MatCapMask"),
    s("_MatCap2ndTex", SlotDefault::White, SlotUse::Image, "マットキャップ 2nd", "MatCap 2nd", "MatCap2nd"),
    s("_MatCap2ndBlendMask", SlotDefault::White, SlotUse::Mask, "マットキャップ 2nd のマスク", "MatCap 2nd Mask", "MatCap2ndMask"),
    s("_RimColorTex", SlotDefault::White, SlotUse::Color, "リムライトの色", "Rim Light Color", "RimColor"),
    s("_OutlineTex", SlotDefault::White, SlotUse::Color, "輪郭線の色", "Outline Color", "OutlineColor"),
    s("_OutlineWidthMask", SlotDefault::White, SlotUse::Mask, "輪郭線の太さ", "Outline Width", "OutlineWidth"),
];

/// スロットの番号（シェーダーの並び）。
pub fn slot_index(name: &str) -> Option<usize> {
    SLOTS.iter().position(|s| s.name == name)
}

pub fn slot(name: &str) -> Option<&'static Slot> {
    SLOTS.iter().find(|s| s.name == name)
}

/// 描画モード。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderMode {
    Opaque,
    Cutout,
    Transparent,
}

impl RenderMode {
    pub const ALL: [RenderMode; 3] = [RenderMode::Opaque, RenderMode::Cutout, RenderMode::Transparent];

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
        "Transparent" | "OnePassTransparent" | "TwoPassTransparent" => (RenderMode::Transparent, true),
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
    format!("Hidden/lilToon{mode}{}", if outline { "Outline" } else { "" })
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
                assert_eq!((info.mode, info.outline, info.exact), (mode, outline, true), "{name}");
            }
        }
        assert_eq!(shader_name(RenderMode::Opaque, false), "lilToon");
        assert_eq!(shader_name(RenderMode::Cutout, true), "Hidden/lilToonCutoutOutline");
        // 空の名前は lilToon
        assert_eq!(shader_info(&with_shader("")).mode, RenderMode::Opaque);
    }

    #[test]
    fn variants_outside_the_scope_are_drawn_approximately() {
        let lite = shader_info(&with_shader("Hidden/lilToonLiteTransparentOutline"));
        assert_eq!((lite.mode, lite.outline, lite.exact), (RenderMode::Transparent, true, false));
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
        assert_eq!((m.mode, m.outline, m.exact), (RenderMode::Cutout, false, true));
    }

    #[test]
    fn the_tables_have_unique_names_and_defaults_inside_their_ranges() {
        let mut names: Vec<&str> = PROPS.iter().map(|p| p.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "プロパティの名前が重なる");
        for p in PROPS {
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
        // 既定の値の読み出し
        let look = with_shader("lilToon");
        assert_eq!(number(&look, "_ShadowBorder"), 0.5);
        assert_eq!(value(&look, "_ShadowColor"), [0.82, 0.76, 0.85, 1.0]);
        assert!(!on(&look, "_UseShadow"));
        assert!(on(&look, "_RimBackfaceMask"));
    }
}
