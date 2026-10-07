//! テクスチャセットの見た目の設定（`MaterialLook`）を GPU へ渡す: lilToon の値の一様バッファ（`shaders/liltoon.wgsl` の `Lil`）、
//! スロットが読むユーザーチャンネルの配列（`user_layers`）、マットキャップの絵（プロジェクトの画像）。セットごとに 1 つ持ち、
//! 値が変わったときだけ書き直す。
//!
//! 値の約束（lilToon と Unity のリニアの色空間と同じ）: 色のプロパティは sRGB からリニアへ（Unity がマテリアルの色をシェーダーへ渡すときと
//! 同じ）、数とベクトルはそのまま。スロットの元は、標準のチャンネルは 3D ビューの絵（`paint`）、ユーザーチャンネルは配列の層、画像は
//! 束ねの 9・10。色空間が sRGB の元を 1 つだけ読むスロットは、読んだ RGB をリニアへ直す（Unity が sRGB のテクスチャを読むときと同じ）。
//! 成分ごとの詰め合わせはリニアのまま（書き出しの詰めた画像はリニアで取り込む）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eframe::egui_wgpu::wgpu;
use yolu_core::look::{LookKind, MaterialLook, PlaneSource, TextureSource};
use yolu_core::{Channel, ColorSpace, Document, ImageColorSpace, ImageId};

use super::brdf;
use super::paint::{ImageTexture, Paint, Slot as PaintSlot};
use super::received_layers::{self, ReceivedLayers, MAX_RECEIVED_LAYERS};
use super::user_layers::{UserLayers, MAX_LAYERS};
use crate::look::liltoon::{self, RenderMode, SlotUse, SLOTS};

/// 描画モード（描き方の選びで使う）。
pub use crate::look::liltoon::RenderMode as RenderModeAlias;

/// `Lil` の値の数（vec4 の数）。
pub const NP: usize = 122;
/// スロットの数。
pub const NS: usize = 38;
/// 一様バッファのバイト数（`p`・`slot_src`・`slot_def`・`slot_flags`・`user_default`）。
pub const LIL_BYTES: u64 = ((NP + 3 * NS + MAX_LAYERS) * 16) as u64;

const IMAGE_SOURCES: [i32; 2] = [32, 33];
/// Unity から受けた絵の配列の元の番号の始まり（40〜55）。
const RECEIVED_SOURCE: i32 = 40;

/// lilToon のパイプラインの定数（`shaders/liltoon.wgsl` の `LIL_FEATURES`・`LIL_SINGLE*`・`LIL_LOOP*`）: 使う機能のビットと、スロットの
/// 読み方（1 つの元をそのまま・成分ごと）のビット。ソフトの描画（llvmpipe）でだけパイプラインに入れ、使わない機能とスロットの読み方を
/// 作らない（一様な分岐の先も全部実行するので）。実機は全部入り（[`LilSpec::ALL`]）の 1 本。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LilSpec {
    pub features: u32,
    pub single: u64,
    pub looped: u64,
}

impl LilSpec {
    pub const ALL: LilSpec = LilSpec {
        features: u32::MAX,
        single: u64::MAX,
        looped: u64::MAX,
    };

    /// パイプラインの定数（名前と値）。
    pub fn constants(&self) -> [(&'static str, f64); 5] {
        [
            ("LIL_FEATURES", f64::from(self.features)),
            ("LIL_SINGLE0", f64::from(self.single as u32)),
            ("LIL_SINGLE1", f64::from((self.single >> 32) as u32)),
            ("LIL_LOOP0", f64::from(self.looped as u32)),
            ("LIL_LOOP1", f64::from((self.looped >> 32) as u32)),
        ]
    }
}

/// `shaders/liltoon.wgsl` の機能のビット（`F_*`）。
pub mod feature {
    pub const SHADOW: u32 = 0;
    pub const BUMP: u32 = 1;
    pub const BUMP2: u32 = 2;
    pub const MAIN2: u32 = 3;
    pub const MAIN3: u32 = 4;
    pub const ALPHA_MASK: u32 = 5;
    pub const RIM_SHADE: u32 = 6;
    pub const BACKLIGHT: u32 = 7;
    pub const REFLECTION: u32 = 8;
    pub const MATCAP: u32 = 9;
    pub const MATCAP2: u32 = 10;
    pub const RIM: u32 = 11;
    pub const GLITTER: u32 = 12;
    pub const EMISSION: u32 = 13;
    pub const EMISSION2: u32 = 14;
    pub const ANISO: u32 = 15;
    pub const DISTANCE_FADE: u32 = 16;
    pub const BACKFACE: u32 = 17;
}

/// 見た目の設定で使う機能のビット（入にしている機能。描画モード・Cull・距離フェードの強さで効かないものは外す）。
pub fn features_of(look: &MaterialLook) -> u32 {
    use feature::*;
    let on = |name: &str| liltoon::on(look, name);
    let info = liltoon::shader_info(look);
    let mut bits = 0u32;
    let mut set = |bit: u32, wanted: bool| {
        if wanted {
            bits |= 1 << bit;
        }
    };
    set(SHADOW, on("_UseShadow"));
    set(BUMP, on("_UseBumpMap"));
    set(BUMP2, on("_UseBump2ndMap"));
    set(MAIN2, on("_UseMain2ndTex"));
    set(MAIN3, on("_UseMain3rdTex"));
    set(
        ALPHA_MASK,
        info.mode != RenderMode::Opaque && liltoon::number(look, "_AlphaMaskMode").round() >= 1.0,
    );
    set(RIM_SHADE, on("_UseRimShade"));
    set(BACKLIGHT, on("_UseBacklight"));
    set(REFLECTION, on("_UseReflection"));
    set(MATCAP, on("_UseMatCap"));
    set(MATCAP2, on("_UseMatCap2nd"));
    set(RIM, on("_UseRim"));
    set(GLITTER, on("_UseGlitter"));
    set(EMISSION, on("_UseEmission"));
    set(EMISSION2, on("_UseEmission2nd"));
    set(ANISO, on("_UseAnisotropy"));
    set(
        DISTANCE_FADE,
        liltoon::value(look, "_DistanceFade")[2] != 0.0,
    );
    set(
        BACKFACE,
        liltoon::number(look, "_Cull").round() <= 1.0
            && liltoon::value(look, "_BackfaceColor")[3] != 0.0,
    );
    bits
}

/// 値（[`params`] の中身）から、スロットの読み方のビット（1 つの元・成分ごと。どちらでもないスロットは既定の値）。
pub fn slot_reads(values: &[f32]) -> (u64, u64) {
    let (mut single, mut looped) = (0u64, 0u64);
    for i in 0..NS {
        let src_at = NP * 4 + i * 4;
        let flags_at = (NP + 2 * NS) * 4 + i * 4;
        let src: [i32; 4] = std::array::from_fn(|k| values[src_at + k].to_bits() as i32);
        let one = values[flags_at + 1] > 0.5;
        if one {
            single |= 1 << i;
        } else if src.iter().any(|c| *c != -1) {
            looped |= 1 << i;
        }
    }
    (single, looped)
}

/// セットの描き方（描くパイプラインの選び）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SetDraw {
    /// lilToon で描くか（false なら標準の PBR）。
    pub lil: bool,
    pub mode: RenderMode,
    pub outline: bool,
    /// Cull（0 Off・1 Front・2 Back）。
    pub cull: u8,
    pub invisible: bool,
    /// 使う機能とスロットの読み方（ソフトの描画のパイプラインの定数）。
    pub spec: LilSpec,
}

impl SetDraw {
    pub const STANDARD: SetDraw = SetDraw {
        lil: false,
        mode: RenderMode::Opaque,
        outline: false,
        cull: 2,
        invisible: false,
        spec: LilSpec::ALL,
    };

    pub fn of(look: &MaterialLook) -> SetDraw {
        if look.kind != LookKind::LilToon {
            return SetDraw::STANDARD;
        }
        let info = liltoon::shader_info(look);
        SetDraw {
            lil: true,
            mode: info.mode,
            outline: info.outline,
            cull: (liltoon::number(look, "_Cull").round() as i32).clamp(0, 2) as u8,
            invisible: liltoon::on(look, "_Invisible"),
            spec: LilSpec {
                features: features_of(look),
                single: 0,
                looped: 0,
            },
        }
    }
}

/// スロットが読むユーザーチャンネル（文書にあるもの。スロットの並びで最初に出てきた順、[`MAX_LAYERS`] まで）。
pub fn wanted_users(doc: &Document, look: &MaterialLook) -> Vec<Channel> {
    let mut all = all_users(doc, look);
    all.truncate(MAX_LAYERS);
    all
}

/// スロットが読むユーザーチャンネルのうち、配列の層の上限（[`MAX_LAYERS`]）を超えて持たないもの（17 個目から。そのスロットは
/// 割り当てのない既定で描く。欄が理由を出す）。
pub fn dropped_users(doc: &Document, look: &MaterialLook) -> Vec<Channel> {
    let all = all_users(doc, look);
    all.get(MAX_LAYERS..)
        .map(<[Channel]>::to_vec)
        .unwrap_or_default()
}

fn all_users(doc: &Document, look: &MaterialLook) -> Vec<Channel> {
    if look.kind != LookKind::LilToon {
        return Vec::new();
    }
    let mut out: Vec<Channel> = Vec::new();
    for slot in SLOTS {
        let Some(source) = look.textures.get(slot.name) else {
            continue;
        };
        for c in source.channels() {
            if !c.is_standard() && doc.channel_info(c).is_some() && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// スロットの割り当てが、Unity から受けた絵に譲るものか: どのレイヤーも使っていない標準のチャンネル（新しいセットの既定の
/// 割り当て。メインカラー → Color・ノーマルマップ → Normal・発光 → Emission）。書き出しもそのチャンネルの画像を書かない（使って
/// いないチャンネルは既定の値）ので、Unity の元のテクスチャで描くのが Unity の見え方と同じ。
pub fn yields_to_received(doc: &Document, source: Option<&TextureSource>) -> bool {
    matches!(source, Some(TextureSource::Channel(c)) if c.is_standard() && !yolu_core::export::uses(doc, *c))
}

/// 受けた見た目があり、このスロットが Unity の流し込み先でない（Unity が元のテクスチャのまま見せる）か。流し込み先は、受けた見た目が
/// スタンドアロンの出すチャンネル（今は Color）を自分で割り当てているスロットで、Unity はそこだけ描いた絵で見せる。
pub fn follows_unity_texture(doc: &Document, slot: &str) -> bool {
    doc.received_look()
        .is_some_and(|r| !r.look.textures.contains_key(slot))
}

/// スロットの割り当てが、Unity の元のテクスチャに譲るものか: 受けた見た目があり、流し込み先でないスロットが、使っていない標準のチャンネルを
/// 読むとき。Unity はそのスロットを元のテクスチャで、空なら既定のテクスチャで読むので、受けた絵が来ない間（空と知らせた・読めない・
/// まだ届かない・層の上限を超えた）も、使っていないチャンネルの値（発光なら黒）ではなく、スロットの既定のテクスチャで描く。
pub fn yields_to_unity_texture(doc: &Document, slot: &str, source: Option<&TextureSource>) -> bool {
    yields_to_received(doc, source) && follows_unity_texture(doc, slot)
}

/// 描く見た目で割り当てていない（か、受けた絵に譲る割り当ての）スロットのうち、Unity から受けた絵のあるもの（スロットの並びの順。
/// 層の上限を超えたものも含む）。
pub fn received_images(
    doc: &Document,
    look: &MaterialLook,
) -> Vec<(String, Arc<yolu_core::look::ReceivedImage>)> {
    let Some(received) = doc.received_look() else {
        return Vec::new();
    };
    if look.kind != LookKind::LilToon {
        return Vec::new();
    }
    SLOTS
        .iter()
        .filter(|s| {
            let source = look.textures.get(s.name);
            source.is_none() || yields_to_received(doc, source)
        })
        .filter_map(|s| {
            received
                .images
                .get(s.name)
                .map(|i| (s.name.to_owned(), i.clone()))
        })
        .collect()
}

/// 3D ビューが受けた絵の配列に持つもの（[`received_images`] の先から [`MAX_RECEIVED_LAYERS`] 枚）。
pub fn received_layered(
    doc: &Document,
    look: &MaterialLook,
) -> Vec<(String, Arc<yolu_core::look::ReceivedImage>)> {
    let mut all = received_images(doc, look);
    all.truncate(MAX_RECEIVED_LAYERS);
    all
}

/// Unity から受けた絵のあるスロットのうち、配列の層の上限（[`MAX_RECEIVED_LAYERS`]）を超えて持たないもの（17 枚目から。そのスロットは
/// 割り当てのない既定で描く。欄が理由を出す）。
pub fn dropped_received(doc: &Document, look: &MaterialLook) -> Vec<String> {
    received_images(doc, look)
        .into_iter()
        .skip(MAX_RECEIVED_LAYERS)
        .map(|(slot, _)| slot)
        .collect()
}

/// この文書の描く見た目が持つ、受けた絵の配列のバイト数（ミップ込み。`limit` は辺の上限、`budget` はバイトの予算）。3D ビューの全体の
/// 予算の計画に使う（[`received_layers::plan`]）。
pub fn planned_received_bytes(doc: &Document, limit: u32, budget: u64) -> u64 {
    received_layers::plan(&received_layered(doc, doc.drawn_look()), limit, budget).1
}

/// この文書の見た目が持つユーザーチャンネルの配列のバイト数（ミップ込み。`paint_shift` は標準のチャンネルの縮め）。3D ビューの
/// 全体の予算の計画に使う（[`super::user_layers::plan`]）。
pub fn planned_user_bytes(doc: &Document, paint_shift: u32, limit: u32, budget: u64) -> u64 {
    let count = wanted_users(doc, doc.drawn_look()).len();
    super::user_layers::plan(
        [doc.width(), doc.height()],
        count,
        paint_shift,
        limit,
        budget,
    )
    .1
}

/// 色のプロパティをリニアへ（Unity と同じ `GammaToLinearSpace`。1 を超える色は pow 2.2）。
fn linear_color(c: [f32; 4]) -> [f32; 4] {
    [
        brdf::unity_gamma_to_linear(c[0]),
        brdf::unity_gamma_to_linear(c[1]),
        brdf::unity_gamma_to_linear(c[2]),
        c[3],
    ]
}

/// GPU が読むときにリニアへ直すチャンネル（sRGB の形式で持つ Color・Emission。`paint`）。
fn decoded_by_gpu(channel: Channel) -> bool {
    matches!(channel, Channel::Color | Channel::Emission)
}

/// 標準のチャンネルの元の番号（3D ビューの絵の束ねの番号）。
fn paint_source(channel: Channel) -> Option<i32> {
    PaintSlot::of(channel).map(|s| s.index() as i32)
}

/// lilToon の値の一様バッファの中身（f32 の並び。整数の並びはビットのまま）。`users` は配列の層の並び、`images` は画像の元の
/// 色空間（スロット `_MatCapTex`・`_MatCap2ndTex` の順。持っていなければ None）。
pub fn params(
    doc: &Document,
    look: &MaterialLook,
    users: &[Channel],
    images: [Option<ImageColorSpace>; 2],
) -> Vec<f32> {
    params_with(doc, look, users, images, &[])
}

/// `params` に、Unity から受けた絵（`received`: スロットの名前・配列の層・sRGB か）を足したもの。受けた絵は、割り当てていない
/// スロットだけが読む（割り当てたチャンネル・画像が勝つ）。
pub fn params_with(
    doc: &Document,
    look: &MaterialLook,
    users: &[Channel],
    images: [Option<ImageColorSpace>; 2],
    received: &[(&str, usize, bool)],
) -> Vec<f32> {
    let v = |name: &str| liltoon::value(look, name);
    let x = |name: &str| liltoon::number(look, name);
    // 色は Unity と同じ: `[HDR]` の色はリニアのまま、ほかはガンマ → リニア
    let color = |name: &str| {
        if liltoon::is_linear_color(name) {
            v(name)
        } else {
            linear_color(v(name))
        }
    };
    let info = liltoon::shader_info(look);
    let mode = match info.mode {
        RenderMode::Opaque => 0.0,
        RenderMode::Cutout => 1.0,
        RenderMode::Transparent => 2.0,
    };
    let emission_uv = |name: &str| {
        let m = x(name).round();
        // UV1〜3 はモデルが持たない（UV0 で読む）
        if m == 4.0 {
            4.0
        } else {
            0.0
        }
    };
    let mut p: Vec<[f32; 4]> = vec![[0.0; 4]; NP];
    p[0] = [mode, x("_Cutoff"), x("_Cull"), x("_Invisible")];
    p[1] = color("_Color");
    p[2] = v("_MainTex_ST");
    p[3] = v("_MainTexHSVG");
    p[4] = [
        x("_LightMinLimit"),
        x("_LightMaxLimit"),
        x("_MonochromeLighting"),
        x("_AsUnlit"),
    ];
    p[5] = [
        x("_ShadowEnvStrength"),
        x("_AAStrength"),
        x("_BackfaceForceShadow"),
        x("_FlipNormal"),
    ];
    p[6] = v("_LightDirectionOverride");
    p[7] = color("_BackfaceColor");
    p[8] = [
        x("_AlphaMaskMode"),
        x("_AlphaMaskScale"),
        x("_AlphaMaskValue"),
        0.0,
    ];
    p[9] = [
        x("_UseShadow"),
        x("_ShadowStrength"),
        x("_ShadowMaskType"),
        x("_ShadowPostAO"),
    ];
    p[10] = [
        x("_ShadowStrengthMaskLOD"),
        x("_ShadowBorderMaskLOD"),
        x("_ShadowBlurMaskLOD"),
        x("_ShadowColorType"),
    ];
    p[11] = [
        x("_ShadowFlatBorder"),
        x("_ShadowFlatBlur"),
        x("_ShadowBorderRange"),
        x("_ShadowMainStrength"),
    ];
    for (k, prefix) in ["_Shadow", "_Shadow2nd", "_Shadow3rd"].iter().enumerate() {
        p[12 + 2 * k] = color(&format!("{prefix}Color"));
        p[13 + 2 * k] = [
            x(&format!("{prefix}Border")),
            x(&format!("{prefix}Blur")),
            x(&format!("{prefix}NormalStrength")),
            x(&format!("{prefix}Receive")),
        ];
    }
    p[18] = color("_ShadowBorderColor");
    p[19] = v("_ShadowAOShift");
    p[20] = v("_ShadowAOShift2");
    for (k, e) in ["_Emission", "_Emission2nd"].iter().enumerate() {
        let base = 21 + 5 * k;
        p[base] = [
            x(&format!("_Use{}", &e[1..])),
            x(&format!("{e}Blend")),
            x(&format!("{e}BlendMode")),
            x(&format!("{e}MainStrength")),
        ];
        p[base + 1] = color(&format!("{e}Color"));
        p[base + 2] = v(&format!("{e}Map_ST"));
        p[base + 3] = v(&format!("{e}BlendMask_ST"));
        p[base + 4] = [
            x(&format!("{e}Fluorescence")),
            emission_uv(&format!("{e}Map_UVMode")),
            v(&format!("{e}Map_ScrollRotate"))[2],
            v(&format!("{e}BlendMask_ScrollRotate"))[2],
        ];
    }
    p[31] = [x("_UseBumpMap"), x("_BumpScale"), 0.0, 0.0];
    p[32] = v("_BumpMap_ST");
    for (k, m) in ["_MatCap", "_MatCap2nd"].iter().enumerate() {
        let base = 33 + 6 * k;
        p[base] = [
            x(&format!("_Use{}", &m[1..])),
            x(&format!("{m}Blend")),
            x(&format!("{m}BlendMode")),
            x(&format!("{m}MainStrength")),
        ];
        p[base + 1] = color(&format!("{m}Color"));
        p[base + 2] = v(&format!("{m}Tex_ST"));
        p[base + 3] = v(&format!("{m}BlendMask_ST"));
        p[base + 4] = [
            x(&format!("{m}EnableLighting")),
            x(&format!("{m}ShadowMask")),
            x(&format!("{m}BackfaceMask")),
            x(&format!("{m}Lod")),
        ];
        p[base + 5] = [
            x(&format!("{m}NormalStrength")),
            x(&format!("{m}ZRotCancel")),
            x(&format!("{m}Perspective")),
            x(&format!("{m}ApplyTransparency")),
        ];
    }
    p[45] = [
        x("_UseRim"),
        x("_RimBlendMode"),
        x("_RimMainStrength"),
        x("_RimNormalStrength"),
    ];
    p[46] = color("_RimColor");
    p[47] = v("_RimColorTex_ST");
    p[48] = [
        x("_RimBorder"),
        x("_RimBlur"),
        x("_RimFresnelPower"),
        x("_RimEnableLighting"),
    ];
    p[49] = [
        x("_RimShadowMask"),
        x("_RimBackfaceMask"),
        x("_RimApplyTransparency"),
        x("_RimDirStrength"),
    ];
    p[50] = [
        x("_RimDirRange"),
        x("_RimIndirRange"),
        x("_RimIndirBorder"),
        x("_RimIndirBlur"),
    ];
    p[51] = color("_RimIndirColor");
    p[52] = [
        f32::from(info.outline),
        x("_OutlineWidth"),
        x("_OutlineFixWidth"),
        x("_OutlineEnableLighting"),
    ];
    p[53] = color("_OutlineColor");
    p[54] = v("_OutlineTex_ST");
    p[55] = v("_OutlineTexHSVG");
    p[56] = color("_OutlineLitColor");
    p[57] = [
        x("_OutlineLitScale"),
        x("_OutlineLitOffset"),
        x("_OutlineLitApplyTex"),
        x("_OutlineLitShadowReceive"),
    ];
    p[58] = [
        x("_OutlineZBias"),
        x("_OutlineDeleteMesh"),
        v("_OutlineTex_ScrollRotate")[2],
        0.0,
    ];
    p[59] = [1.0, f32::from(info.exact), 0.0, 0.0];
    p[60] = [
        v("_MainTex_ScrollRotate")[2],
        x("_ShiftBackfaceUV"),
        0.0,
        0.0,
    ];
    // メインカラー 2nd・3rd（liltoon.wgsl の L_*）
    for (base, l, n) in [(61, "Main2nd", "2nd"), (70, "Main3rd", "3rd")] {
        let uv_mode = if x(&format!("_{l}Tex_UVMode")).round() == 4.0 {
            4.0
        } else {
            0.0
        };
        p[base] = [
            x(&format!("_Use{l}Tex")),
            x(&format!("_{l}EnableLighting")),
            x(&format!("_{l}TexBlendMode")),
            x(&format!("_{l}TexAlphaMode")),
        ];
        p[base + 1] = color(&format!("_Color{n}"));
        p[base + 2] = v(&format!("_{l}Tex_ST"));
        p[base + 3] = [
            x(&format!("_{l}TexAngle")),
            uv_mode,
            x(&format!("_{l}Tex_Cull")),
            x(&format!("_{l}TexIsMSDF")),
        ];
        p[base + 4] = [
            x(&format!("_{l}TexIsDecal")),
            x(&format!("_{l}TexIsLeftOnly")),
            x(&format!("_{l}TexIsRightOnly")),
            x(&format!("_{l}TexShouldCopy")),
        ];
        p[base + 5] = [
            x(&format!("_{l}TexShouldFlipMirror")),
            x(&format!("_{l}TexShouldFlipCopy")),
            0.0,
            0.0,
        ];
        p[base + 6] = v(&format!("_{l}TexDecalAnimation"));
        p[base + 7] = v(&format!("_{l}TexDecalSubParam"));
        p[base + 8] = v(&format!("_{l}DistanceFade"));
    }
    p[79] = [x("_UseBump2ndMap"), x("_Bump2ndScale"), 0.0, 0.0];
    p[80] = v("_Bump2ndMap_ST");
    p[81] = [
        x("_UseRimShade"),
        x("_RimShadeNormalStrength"),
        x("_RimShadeBorder"),
        x("_RimShadeBlur"),
    ];
    p[82] = color("_RimShadeColor");
    p[83] = [x("_RimShadeFresnelPower"), 0.0, 0.0, 0.0];
    p[84] = [
        x("_UseBacklight"),
        x("_BacklightMainStrength"),
        x("_BacklightReceiveShadow"),
        x("_BacklightBackfaceMask"),
    ];
    p[85] = color("_BacklightColor");
    p[86] = [
        x("_BacklightNormalStrength"),
        x("_BacklightBorder"),
        x("_BacklightBlur"),
        x("_BacklightDirectivity"),
    ];
    p[87] = [x("_BacklightViewStrength"), 0.0, 0.0, 0.0];
    p[88] = v("_BacklightColorTex_ST");
    // [Gamma] の数（金属度・反射率）は、Unity と同じく sRGB → リニア（GammaToLinearSpace）
    let gamma = |name: &str| brdf::unity_gamma_to_linear(x(name));
    p[89] = [
        x("_UseReflection"),
        x("_Smoothness"),
        gamma("_Metallic"),
        gamma("_Reflectance"),
    ];
    p[90] = color("_ReflectionColor");
    p[91] = [
        x("_ApplySpecular"),
        x("_SpecularToon"),
        x("_SpecularNormalStrength"),
        x("_SpecularBorder"),
    ];
    p[92] = [
        x("_SpecularBlur"),
        x("_ApplyReflection"),
        x("_ReflectionNormalStrength"),
        x("_ReflectionApplyTransparency"),
    ];
    p[93] = [x("_ReflectionBlendMode"), x("_GSAAStrength"), 0.0, 0.0];
    p[94] = v("_SmoothnessTex_ST");
    p[95] = v("_MetallicGlossMap_ST");
    p[96] = v("_ReflectionColorTex_ST");
    p[97] = [
        x("_UseGlitter"),
        x("_GlitterUVMode"),
        x("_GlitterMainStrength"),
        x("_GlitterEnableLighting"),
    ];
    p[98] = color("_GlitterColor");
    p[99] = v("_GlitterColorTex_ST");
    p[100] = v("_GlitterParams1");
    p[101] = v("_GlitterParams2");
    p[102] = [
        x("_GlitterShadowMask"),
        x("_GlitterBackfaceMask"),
        x("_GlitterApplyTransparency"),
        x("_GlitterNormalStrength"),
    ];
    p[103] = [
        x("_GlitterPostContrast"),
        x("_GlitterSensitivity"),
        x("_GlitterScaleRandomize"),
        0.0,
    ];
    p[104] = [
        x("_UseAnisotropy"),
        x("_AnisotropyScale"),
        x("_Anisotropy2Reflection"),
        x("_Anisotropy2MatCap"),
    ];
    p[105] = [x("_Anisotropy2MatCap2nd"), 0.0, 0.0, 0.0];
    for (k, a) in ["_Anisotropy", "_Anisotropy2nd"].iter().enumerate() {
        p[106 + k] = [
            x(&format!("{a}TangentWidth")),
            x(&format!("{a}BitangentWidth")),
            x(&format!("{a}Shift")),
            x(&format!("{a}ShiftNoiseScale")),
        ];
    }
    p[108] = [
        x("_AnisotropySpecularStrength"),
        x("_Anisotropy2ndSpecularStrength"),
        0.0,
        0.0,
    ];
    p[109] = v("_AnisotropyTangentMap_ST");
    p[110] = v("_AnisotropyScaleMask_ST");
    p[111] = v("_AnisotropyShiftNoiseMask_ST");
    p[112] = color("_DistanceFadeColor");
    p[113] = v("_DistanceFade");
    p[114] = color("_DistanceFadeRimColor");
    p[115] = [
        x("_DistanceFadeMode"),
        x("_DistanceFadeRimFresnelPower"),
        0.0,
        0.0,
    ];
    p[116] = [
        x("_MatCapCustomNormal"),
        x("_MatCapBumpScale"),
        x("_MatCap2ndCustomNormal"),
        x("_MatCap2ndBumpScale"),
    ];
    p[117] = v("_MatCapBumpMap_ST");
    p[118] = v("_MatCap2ndBumpMap_ST");
    p[119] = v("_AlphaMask_ST");
    // 座標のモードの距離フェードの原点（モデルは世界の空間で、物の原点は世界の原点）
    p[120] = [0.0, 0.0, 0.0, 1.0];

    let mut src: Vec<[i32; 4]> = vec![[-1; 4]; NS];
    let mut def: Vec<[f32; 4]> = vec![[0.0; 4]; NS];
    let mut flags: Vec<[f32; 4]> = vec![[0.0; 4]; NS];
    let user_source = |c: Channel| -> Option<i32> {
        if c.is_standard() {
            paint_source(c)
        } else {
            users.iter().position(|u| *u == c).map(|i| 6 + i as i32)
        }
    };
    let srgb = |c: Channel| -> bool {
        doc.channel_info(c)
            .is_some_and(|i| i.color_space == ColorSpace::Srgb)
    };
    let scalar = |c: Channel| -> bool {
        doc.channel_info(c)
            .is_some_and(|i| i.kind == yolu_core::ChannelKind::Scalar)
    };
    for (i, slot) in SLOTS.iter().enumerate() {
        def[i] = slot.default.rgba();
        // 使っていない標準のチャンネルの割り当ては、Unity の元のテクスチャに譲る: 受けた絵があればそれで、無ければスロットの既定で
        // （Unity は空のテクスチャを既定で読む）。流し込み先と、受けた見た目の無いセットは割り当てのチャンネルを読む
        let assigned = look.textures.get(slot.name).filter(|s| {
            !(yields_to_received(doc, Some(s))
                && (received.iter().any(|r| r.0 == slot.name)
                    || follows_unity_texture(doc, slot.name)))
        });
        let Some(source) = assigned else {
            if let Some((_, layer, srgb)) = received.iter().find(|r| r.0 == slot.name) {
                let s = RECEIVED_SOURCE + *layer as i32;
                src[i] = [s * 4, s * 4 + 1, s * 4 + 2, s * 4 + 3];
                flags[i] = [f32::from(*srgb), 1.0, s as f32, 0.0];
            }
            continue;
        };
        match source {
            TextureSource::Channel(c) => {
                let Some(s) = user_source(*c).filter(|_| doc.channel_info(*c).is_some()) else {
                    continue;
                };
                if !c.is_standard() && scalar(*c) {
                    // スカラーのユーザーチャンネルは値を RGB に、A は 1
                    src[i] = [s * 4, s * 4, s * 4, -3];
                } else {
                    src[i] = [s * 4, s * 4 + 1, s * 4 + 2, s * 4 + 3];
                    flags[i][1] = 1.0;
                    flags[i][2] = s as f32;
                }
                // Color・Emission は sRGB の形式のテクスチャで、GPU が読むときにリニアにしてある（`paint`）
                flags[i][0] = f32::from(srgb(*c) && !decoded_by_gpu(*c));
            }
            TextureSource::Packed(planes) => {
                for (k, plane) in planes.iter().enumerate() {
                    src[i][k] = match plane {
                        PlaneSource::Zero => -2,
                        PlaneSource::One => -3,
                        PlaneSource::Channel { channel, component } => {
                            match user_source(*channel)
                                .filter(|_| doc.channel_info(*channel).is_some())
                            {
                                Some(s) => s * 4 + i32::from(*component),
                                None => -1,
                            }
                        }
                    };
                }
            }
            TextureSource::Image(_) => {
                let which = match slot.name {
                    "_MatCapTex" => 0,
                    "_MatCap2ndTex" => 1,
                    _ => continue,
                };
                if images[which].is_none() {
                    continue;
                }
                // sRGB の画像は sRGB の形式で持ち、GPU が読むときにリニアにしてある（直さない）。リニアの画像はそのまま
                let s = IMAGE_SOURCES[which];
                src[i] = [s * 4, s * 4 + 1, s * 4 + 2, s * 4 + 3];
                flags[i] = [0.0, 1.0, s as f32, 0.0];
            }
        }
        debug_assert!(slot.usage != SlotUse::Image || i == 14 || i == 16);
    }
    let mut user_default = vec![[0.0f32; 4]; MAX_LAYERS];
    for (k, c) in users.iter().enumerate() {
        if let Some(info) = doc.channel_info(*c) {
            let d = info.default;
            user_default[k] = [
                d.r as f32 / 255.0,
                d.g as f32 / 255.0,
                d.b as f32 / 255.0,
                d.a as f32 / 255.0,
            ];
        }
    }
    let mut out: Vec<f32> = Vec::with_capacity(LIL_BYTES as usize / 4);
    for v in &p {
        out.extend_from_slice(v);
    }
    for s in &src {
        out.extend(s.iter().map(|i| f32::from_bits(*i as u32)));
    }
    for d in &def {
        out.extend_from_slice(d);
    }
    for f in &flags {
        out.extend_from_slice(f);
    }
    for d in &user_default {
        out.extend_from_slice(d);
    }
    debug_assert_eq!(out.len() * 4, LIL_BYTES as usize);
    out
}

// ───────── 環境光の SH ─────────

fn sh_basis(d: yolu_core::glam::Vec3) -> [f32; 9] {
    [
        1.0,
        d.y,
        d.z,
        d.x,
        d.x * d.y,
        d.y * d.z,
        3.0 * d.z * d.z - 1.0,
        d.x * d.z,
        d.x * d.x - d.y * d.y,
    ]
}

/// 環境を上の軸のまわりに θ 回した SH（`scene.wgsl` の `to_source` と同じ回し方: 回した SH を n で読む値 = 元の SH を R(−θ) n で読む値）。
/// 2 次までの SH は回しても 2 次までの SH なので、26 の向きで読んだ値に最小二乗で当てはめる（誤差は浮動小数の丸めだけ）。
pub fn rotate_sh_y(sh: &[yolu_core::glam::Vec3; 9], radians: f32) -> [yolu_core::glam::Vec3; 9] {
    use yolu_core::glam::Vec3;
    if radians == 0.0 {
        return *sh;
    }
    let (s, c) = radians.sin_cos();
    let eval = |d: Vec3| -> Vec3 {
        let b = sh_basis(d);
        sh.iter().zip(b).map(|(k, v)| *k * v).sum()
    };
    let mut dirs: Vec<Vec3> = Vec::with_capacity(26);
    for x in -1i32..=1 {
        for y in -1i32..=1 {
            for z in -1i32..=1 {
                if (x, y, z) != (0, 0, 0) {
                    dirs.push(Vec3::new(x as f32, y as f32, z as f32).normalize());
                }
            }
        }
    }
    // 正規方程式（9 × 9）
    let mut ata = [[0.0f64; 9]; 9];
    let mut atb = [[0.0f64; 3]; 9];
    for d in &dirs {
        let b = sh_basis(*d);
        let source = Vec3::new(d.x * c - d.z * s, d.y, d.x * s + d.z * c);
        let value = eval(source);
        for i in 0..9 {
            for j in 0..9 {
                ata[i][j] += f64::from(b[i] * b[j]);
            }
            atb[i][0] += f64::from(b[i] * value.x);
            atb[i][1] += f64::from(b[i] * value.y);
            atb[i][2] += f64::from(b[i] * value.z);
        }
    }
    // ガウスの消去（部分ピボット）
    for col in 0..9 {
        let pivot = (col..9)
            .max_by(|a, b| ata[*a][col].abs().total_cmp(&ata[*b][col].abs()))
            .expect("行がある");
        ata.swap(col, pivot);
        atb.swap(col, pivot);
        let p = ata[col][col];
        for row in 0..9 {
            if row == col {
                continue;
            }
            let f = ata[row][col] / p;
            let (pivot_row, pivot_b) = (ata[col], atb[col]);
            for (a, c) in ata[row].iter_mut().zip(pivot_row).skip(col) {
                *a -= f * c;
            }
            for (b, c) in atb[row].iter_mut().zip(pivot_b) {
                *b -= f * c;
            }
        }
    }
    let mut out = [Vec3::ZERO; 9];
    for (i, o) in out.iter_mut().enumerate() {
        let p = ata[i][i];
        *o = Vec3::new(
            (atb[i][0] / p) as f32,
            (atb[i][1] / p) as f32,
            (atb[i][2] / p) as f32,
        );
    }
    out
}

/// SH（`scene.wgsl` の `evaluate_sh` の係数）を Unity の unity_SHAr・SHAg・SHAb・SHBr・SHBg・SHBb・SHC の形に。
pub fn unity_sh(sh: &[yolu_core::glam::Vec3; 9]) -> [[f32; 4]; 7] {
    let ch = |k: usize| -> [f32; 9] { sh.map(|v| v[k]) };
    let mut out = [[0.0f32; 4]; 7];
    for k in 0..3 {
        let c = ch(k);
        out[k] = [c[3], c[1], c[2], c[0] - c[6]];
        out[3 + k] = [c[4], c[5], 3.0 * c[6], c[7]];
        out[6][k] = c[8];
    }
    out
}

/// 持っているマットキャップの絵（画像の ID と中身の鍵と色空間）。
struct HeldImage {
    id: ImageId,
    hash: String,
    space: ImageColorSpace,
    texture: ImageTexture,
}

/// 見た目の配列の辺の上限とバイトの予算（[`LookGpu::sync`]。3D ビューの全体の予算の計画が決める）。
#[derive(Clone, Copy, Debug)]
pub struct LookBudget {
    /// ユーザーチャンネルの配列の辺の上限（受けた絵の配列は、これと [`received_layers::MAX_LAYER_SIZE`] の小さいほう）。
    pub limit: u32,
    /// ユーザーチャンネルの配列のバイトの予算。
    pub users: u64,
    /// Unity から受けた絵の配列のバイトの予算。
    pub received: u64,
}

/// 1 つのセットの見た目の GPU の持ち物。
pub struct LookGpu {
    pub buffer: wgpu::Buffer,
    pub users: UserLayers,
    /// Unity から受けた、描いていないスロットの絵（束ねの 11）。
    pub received: ReceivedLayers,
    images: [Option<HeldImage>; 2],
    written: Vec<f32>,
    /// 中身が変わるたびに増える（描き直しの鍵）。
    version: u64,
    /// 画像を入れ替えるたびに増える（束ねの鍵）。
    image_version: u64,
    pub draw: SetDraw,
    /// 法線マップを使う（lilToon のノーマルマップが入）。接線を作るかの決めに使う。
    pub wants_tangents: bool,
    /// 最後に値を作ったときの鍵（文書・版・見た目の設定の入れ物・層の並び・画像の色空間）。同じなら値を作り直さない。
    params_key: Option<ParamsKey>,
    /// 値（`params`）を作った回数（試験・計測用。同じ道具の兄弟の持ち物（ほかのセット）と共有する）。
    builds: Arc<AtomicU64>,
}

#[derive(Clone, PartialEq)]
struct ParamsKey {
    doc: u128,
    revision: u64,
    /// 描く見た目の番号（利用者の設定・受けた見た目のどちらが変わっても進む）。
    look_serial: u64,
    /// 受けた絵の層の並び（スロットの名前と sRGB か）。
    received: Vec<(String, bool)>,
    /// 見た目の設定の入れ物の番地（文書は設定を替えるたびに新しい入れ物にする。番地の使い回しは版で分ける）。
    look: usize,
    users: Vec<Channel>,
    spaces: [Option<ImageColorSpace>; 2],
}

impl LookGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> LookGpu {
        LookGpu::with_users(
            device,
            UserLayers::new(device, queue),
            ReceivedLayers::new(device, queue),
        )
    }

    fn with_users(device: &wgpu::Device, users: UserLayers, received: ReceivedLayers) -> LookGpu {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-liltoon"),
            size: LIL_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        LookGpu {
            buffer,
            users,
            received,
            images: [None, None],
            written: Vec::new(),
            version: 0,
            image_version: 0,
            draw: SetDraw::STANDARD,
            wants_tangents: false,
            params_key: None,
            builds: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 同じ道具の、まっさらな別の持ち物（ほかのセット用）。
    pub fn sibling(&self, device: &wgpu::Device) -> LookGpu {
        let mut look = LookGpu::with_users(device, self.users.sibling(), self.received.sibling());
        look.builds = self.builds.clone();
        look
    }

    /// 値（一様バッファの中身）を作った回数（この持ち物と兄弟の合計）。
    pub fn params_builds(&self) -> u64 {
        self.builds.load(Ordering::Relaxed)
    }

    /// 描き直しの鍵。
    pub fn key(&self) -> u64 {
        self.version.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ self.users.version().rotate_left(17)
            ^ self.image_version.rotate_left(33)
    }

    /// 束ねを作り直す鍵（配列の作り直し・画像の入れ替え）。
    pub fn bind_key(&self) -> (u64, u64, u64) {
        let (uid, layout) = self.users.bind_key();
        (uid, layout, self.image_version)
    }

    /// マットキャップの絵の見え方（持っていなければ None。束ねは既定の白を使う）。
    pub fn image_view(&self, which: usize) -> Option<&wgpu::TextureView> {
        self.images[which].as_ref().map(|h| h.texture.view())
    }

    /// GPU のバイト数（ユーザーチャンネルの配列と、Unity から受けた絵の配列。画像は数えない）。
    pub fn bytes(&self) -> u64 {
        self.users.bytes() + self.received.bytes()
    }

    /// 文書の見た目の設定に合わせる（値・ユーザーチャンネルの配列・画像・受けた絵の配列）。`paint` はそのセットの標準のチャンネルの絵（縮めと
    /// 画像の作り方）、`budget` は配列の辺の上限とバイトの予算。標準の見た目のセットは、持ち物を手放すだけで値を作らない
    /// （描き方は PBR で、値を読まない）。lilToon のセットも、文書・版・設定が前と同じなら値を作り直さない（描くたびに回るので）。
    pub fn sync(
        &mut self,
        doc: &Document,
        paint: &Paint,
        budget: LookBudget,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let LookBudget {
            limit,
            users: budget,
            received: received_budget,
        } = budget;
        // 描く見た目（Unity から受けた値があれば、その上に利用者の設定を重ねたもの）
        let look = doc.drawn_look();
        let mut draw = SetDraw::of(look);
        // スロットの読み方は値を作ったときに決まる（下）。それまでは前の読み方
        draw.spec.single = self.draw.spec.single;
        draw.spec.looped = self.draw.spec.looped;
        if !draw.lil {
            if self.draw != draw {
                self.draw = draw;
                self.version += 1;
            }
            self.wants_tangents = false;
            self.params_key = None;
            self.users.sync(doc, &[], 0, limit, budget, encoder);
            if self.images.iter_mut().any(|i| i.take().is_some()) {
                self.image_version += 1;
            }
            if self.received.sync(&[], limit, received_budget) {
                self.image_version += 1;
            }
            return;
        }
        if self.draw != draw {
            // 描き方（パイプライン）が替わった: 値が前と同じでも描き直す
            self.draw = draw;
            self.version += 1;
        }
        // 接線を使う機能（ノーマルマップ・2nd・異方性反射・マットキャップのカスタムノーマル・左右で分けるデカール（接線の向きで右手か））
        let decal_sides = |l: &str| {
            liltoon::on(look, &format!("_Use{l}Tex"))
                && ["IsLeftOnly", "IsRightOnly", "ShouldFlipMirror"]
                    .iter()
                    .any(|t| liltoon::on(look, &format!("_{l}Tex{t}")))
        };
        self.wants_tangents = ["_UseBumpMap", "_UseBump2ndMap", "_UseAnisotropy"]
            .iter()
            .any(|t| liltoon::on(look, t))
            || (liltoon::on(look, "_UseMatCap") && liltoon::on(look, "_MatCapCustomNormal"))
            || (liltoon::on(look, "_UseMatCap2nd") && liltoon::on(look, "_MatCap2ndCustomNormal"))
            || decal_sides("Main2nd")
            || decal_sides("Main3rd");
        let users = wanted_users(doc, look);
        self.users
            .sync(doc, &users, paint.level(), limit, budget, encoder);
        // マットキャップの絵（プロジェクトの画像。文書の効果の入力にある画像を使う）
        let mut spaces: [Option<ImageColorSpace>; 2] = [None, None];
        for (which, name) in ["_MatCapTex", "_MatCap2ndTex"].iter().enumerate() {
            let wanted = match look.textures.get(*name) {
                Some(TextureSource::Image(id)) if self.draw.lil => {
                    doc.effect_inputs().image(*id).map(|i| (*id, i))
                }
                _ => None,
            };
            match wanted {
                Some((id, input)) => {
                    let same = self.images[which].as_ref().is_some_and(|h| {
                        h.id == id && h.hash == input.hash && h.space == input.color_space
                    });
                    if !same {
                        let srgb = input.color_space != ImageColorSpace::Linear;
                        self.images[which] = paint
                            .create_image(&input.pixels, [input.width, input.height], srgb, encoder)
                            .map(|texture| HeldImage {
                                id,
                                hash: input.hash.clone(),
                                space: input.color_space,
                                texture,
                            });
                        self.image_version += 1;
                    }
                }
                None => {
                    if self.images[which].take().is_some() {
                        self.image_version += 1;
                    }
                }
            }
            spaces[which] = self.images[which].as_ref().map(|h| h.space);
        }
        // Unity から受けた絵（割り当てていないスロットのもの。スロットの並びで先から、層の上限まで）
        let wanted = received_layered(doc, look);
        if self.received.sync(&wanted, limit, received_budget) {
            self.image_version += 1;
        }
        let received: Vec<(String, bool)> = wanted
            .iter()
            .map(|(slot, image)| (slot.clone(), image.srgb))
            .collect();
        let key = ParamsKey {
            doc: doc.id(),
            revision: doc.revision(),
            look_serial: doc.look_serial(),
            received: received.clone(),
            look: look as *const MaterialLook as usize,
            users: self.users.channels().to_vec(),
            spaces,
        };
        if self.params_key.as_ref() == Some(&key) {
            return;
        }
        self.params_key = Some(key);
        self.builds.fetch_add(1, Ordering::Relaxed);
        let layers: Vec<(&str, usize, bool)> = received
            .iter()
            .filter_map(|(slot, srgb)| Some((slot.as_str(), self.received.layer_of(slot)?, *srgb)))
            .collect();
        let values = params_with(doc, look, self.users.channels(), spaces, &layers);
        // ビットで比べる（整数の並び -1 などはビットのまま f32 に入れていて、NaN になる）
        let same = values.len() == self.written.len()
            && values
                .iter()
                .zip(&self.written)
                .all(|(a, b)| a.to_bits() == b.to_bits());
        if !same {
            let (single, looped) = slot_reads(&values);
            if (single, looped) != (self.draw.spec.single, self.draw.spec.looped) {
                self.draw.spec.single = single;
                self.draw.spec.looped = looped;
            }
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            queue.write_buffer(&self.buffer, 0, &bytes);
            self.written = values;
            self.version += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::look::LookValue;
    use yolu_core::{ChannelInfo, ChannelKind, Rgba8};

    fn lil() -> MaterialLook {
        MaterialLook {
            kind: LookKind::LilToon,
            ..MaterialLook::default()
        }
    }

    fn slot_src(values: &[f32], i: usize) -> [i32; 4] {
        let at = NP * 4 + i * 4;
        [0, 1, 2, 3].map(|k| values[at + k].to_bits() as i32)
    }

    fn slot_flags(values: &[f32], i: usize) -> [f32; 4] {
        let at = (NP + 2 * NS) * 4 + i * 4;
        [values[at], values[at + 1], values[at + 2], values[at + 3]]
    }

    #[test]
    fn colors_are_linearized_and_numbers_kept() {
        let doc = Document::new(8, 8).unwrap();
        let mut look = lil();
        look.properties
            .insert("_Color".into(), LookValue::Color([0.5, 1.0, 0.0, 0.25]));
        look.properties
            .insert("_ShadowBorder".into(), LookValue::Float(0.3));
        let v = params(&doc, &look, &[], [None, None]);
        assert_eq!(v.len() * 4, LIL_BYTES as usize);
        assert!((v[4] - brdf::srgb_to_linear(0.5)).abs() < 1e-6);
        assert_eq!(v[5], 1.0);
        assert_eq!(v[7], 0.25, "アルファは直さない");
        // _ShadowBorder は P_SHADOW1.x（13 番）
        assert_eq!(v[13 * 4], 0.3);
        // 既定: _ShadowColor (0.82, 0.76, 0.85) をリニアへ
        assert!((v[12 * 4] - brdf::srgb_to_linear(0.82)).abs() < 1e-6);
        // 描画モードと Cutoff
        assert_eq!(&v[0..2], &[0.0, 0.5]);
    }

    #[test]
    fn colors_above_one_follow_unity_and_hdr_colors_stay_linear() {
        // lilToon の欄で 1 を超えて選べる色（[lilHDR]。Unity の [HDR] ではない）は、リニアの Unity と同じ GammaToLinearSpace
        // （1 以上は pow 2.2）。Unity 2022.3 で測った値: 16.948 → 505.9、2.119 → 5.218。[HDR] の発光の色はそのまま
        let doc = Document::new(8, 8).unwrap();
        let mut look = lil();
        look.properties.insert(
            "_BacklightColor".into(),
            LookValue::Color([16.948, 2.119, 0.5, 1.0]),
        );
        look.properties.insert(
            "_MatCapColor".into(),
            LookValue::Color([2.119, 1.895, 1.789, 1.0]),
        );
        look.properties.insert(
            "_EmissionColor".into(),
            LookValue::Color([2.5, 0.5, 16.948, 1.0]),
        );
        let v = params(&doc, &look, &[], [None, None]);
        let at = |i: usize| &v[i * 4..i * 4 + 4];
        // P_BACKLIGHT_COLOR は 85 番、P_MATCAP_COLOR は 34 番、P_EMISSION_COLOR は 22 番
        let bl = at(85);
        assert!((bl[0] - 505.895_33).abs() < 0.01, "{bl:?}");
        assert!((bl[1] - 5.217_808).abs() < 1e-4, "{bl:?}");
        assert!((bl[2] - brdf::srgb_to_linear(0.5)).abs() < 1e-6);
        assert_eq!(bl[3], 1.0);
        let mc = at(34);
        assert!((mc[0] - 2.119f32.powf(2.2)).abs() < 1e-4, "{mc:?}");
        assert_eq!(at(22), &[2.5, 0.5, 16.948, 1.0]);
    }

    #[test]
    fn slots_encode_channels_packing_and_defaults() {
        let mut doc = Document::new(8, 8).unwrap();
        let mask = doc
            .add_channel(ChannelInfo {
                name: "影".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })
            .unwrap();
        let tint = doc
            .add_channel(ChannelInfo {
                name: "影色".into(),
                kind: ChannelKind::Color,
                color_space: ColorSpace::Srgb,
                default: Rgba8::new(0, 0, 0, 0),
            })
            .unwrap();
        let mut look = lil();
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        look.textures
            .insert("_ShadowStrengthMask".into(), TextureSource::Channel(mask));
        look.textures
            .insert("_ShadowColorTex".into(), TextureSource::Channel(tint));
        look.textures.insert(
            "_ShadowBorderMask".into(),
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: mask,
                    component: 0,
                },
                PlaneSource::One,
                PlaneSource::Zero,
                PlaneSource::Channel {
                    channel: Channel::Roughness,
                    component: 0,
                },
            ]),
        );
        let users = wanted_users(&doc, &look);
        assert_eq!(users, vec![mask, tint], "スロットの並びで最初に出てきた順");
        let v = params(&doc, &look, &users, [None, None]);
        // _MainTex: Color（絵の束ね 0）を全部。sRGB の形式のテクスチャで GPU がリニアにして読むので、シェーダーでは直さない
        assert_eq!(slot_src(&v, 0), [0, 1, 2, 3]);
        assert_eq!(slot_flags(&v, 0), [0.0, 1.0, 0.0, 0.0]);
        // スカラーのユーザーチャンネル（層 0 = 元 6）は値を RGB に
        assert_eq!(slot_src(&v, 4), [24, 24, 24, -3]);
        // 色のユーザーチャンネル（層 1 = 元 7）は全部、sRGB
        assert_eq!(slot_src(&v, 7), [28, 29, 30, 31]);
        assert_eq!(slot_flags(&v, 7)[0], 1.0);
        // 詰め合わせ: R ← 層 0 の R、G ← 1、B ← 0、A ← Roughness（絵の束ね 2）の R
        assert_eq!(slot_src(&v, 5), [24, -3, -2, 8]);
        // 割り当てていないスロットは既定（_ShadowColorTex 2nd は黒）
        assert_eq!(slot_src(&v, 8), [-1; 4]);
        let def_at = (NP + NS) * 4 + 8 * 4;
        assert_eq!(&v[def_at..def_at + 4], &[0.0; 4]);
        // ユーザーチャンネルの既定の値
        let ud = (NP + 3 * NS) * 4;
        assert_eq!(&v[ud..ud + 4], &[1.0; 4]);
        assert_eq!(&v[ud + 4..ud + 8], &[0.0; 4]);
        // 消したチャンネルは割り当てなし
        let mut gone = doc;
        gone.remove_channel(mask).unwrap();
        let users = wanted_users(&gone, &look);
        assert_eq!(users, vec![tint]);
        let v = params(&gone, &look, &users, [None, None]);
        assert_eq!(slot_src(&v, 4), [-1; 4]);
        assert_eq!(slot_src(&v, 5), [-1, -3, -2, 8]);
    }

    fn received_image(srgb: bool) -> Arc<yolu_core::look::ReceivedImage> {
        Arc::new(yolu_core::look::ReceivedImage {
            width: 2,
            height: 2,
            srgb,
            pixels: vec![255; 16].into(),
        })
    }

    /// 受けた見た目: _MainTex は流し込み先（Color）、ほかの全部のスロットに Unity から受けた絵（色・マットキャップのスロットは sRGB）。
    fn received_everywhere(doc: &mut Document) {
        let mut look = lil();
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        let mut r = yolu_core::look::ReceivedLook {
            look,
            ..Default::default()
        };
        for slot in &SLOTS[1..] {
            r.images.insert(
                slot.name.into(),
                received_image(matches!(slot.usage, SlotUse::Color | SlotUse::Image)),
            );
        }
        doc.set_received_look(Some(r)).unwrap();
    }

    #[test]
    fn received_images_fill_the_unassigned_slots_up_to_the_layer_limit() {
        let mut doc = Document::new(8, 8).unwrap();
        received_everywhere(&mut doc);
        let drawn = doc.drawn_look().clone();
        let all = received_images(&doc, &drawn);
        assert_eq!(
            all.len(),
            SLOTS.len() - 1,
            "流し込み先（_MainTex）は受けた絵を読まない"
        );
        let layered = received_layered(&doc, &drawn);
        assert_eq!(layered.len(), MAX_RECEIVED_LAYERS);
        // スロットの並びで先の 16 枚が層、17 枚目からは持たない
        assert_eq!(
            dropped_received(&doc, &drawn),
            SLOTS[1 + MAX_RECEIVED_LAYERS..]
                .iter()
                .map(|s| s.name.to_owned())
                .collect::<Vec<_>>()
        );
        // LookGpu と同じく、層の番号は並びの順
        let layers: Vec<(&str, usize, bool)> = layered
            .iter()
            .enumerate()
            .map(|(k, (slot, image))| (slot.as_str(), k, image.srgb))
            .collect();
        let v = params_with(&doc, &drawn, &[], [None, None], &layers);
        // 流し込み先は 3D ビューの絵（Color。GPU がリニアにして読む）
        assert_eq!(slot_src(&v, 0), [0, 1, 2, 3]);
        assert_eq!(slot_flags(&v, 0), [0.0, 1.0, 0.0, 0.0]);
        // 色調補正マスク（層 0 = 元 40）: 1 つの元をそのまま読み、リニアのまま
        assert_eq!(slot_src(&v, 1), [160, 161, 162, 163]);
        assert_eq!(slot_flags(&v, 1), [0.0, 1.0, 40.0, 0.0]);
        // ノーマルマップ（層 2 = 元 42）: シェーダーは元が 40 以上なら X を A × R で読む
        assert_eq!(slot_flags(&v, 3), [0.0, 1.0, 42.0, 0.0]);
        // 影色（層 6 = 元 46）は sRGB
        assert_eq!(slot_flags(&v, 7), [1.0, 1.0, 46.0, 0.0]);
        // マットキャップの絵のスロットも受けた絵で描く（層 13 = 元 53）
        assert_eq!(slot_flags(&v, 14), [1.0, 1.0, 53.0, 0.0]);
        // 16 枚目（マットキャップ 2nd、層 15 = 元 55）まで
        assert_eq!(slot_flags(&v, 16), [1.0, 1.0, 55.0, 0.0]);
        // 17 枚目からは割り当てのない既定
        for (i, slot) in SLOTS.iter().enumerate().skip(1 + MAX_RECEIVED_LAYERS) {
            assert_eq!(slot_src(&v, i), [-1; 4], "{}", slot.name);
            assert_eq!(slot_flags(&v, i), [0.0; 4], "{}", slot.name);
        }
    }

    #[test]
    fn a_slot_assigned_here_wins_over_the_received_image() {
        let mut doc = Document::new(8, 8).unwrap();
        received_everywhere(&mut doc);
        // 使っている Color（レイヤーがある）を、利用者が影色に割り当てる（受けた絵より勝つ）
        doc.add_layer("a").unwrap();
        let mut mine = lil();
        mine.textures.insert(
            "_ShadowColorTex".into(),
            TextureSource::Channel(Channel::Color),
        );
        doc.set_look(mine, false).unwrap();
        let drawn = doc.drawn_look().clone();
        let layered = received_layered(&doc, &drawn);
        assert!(layered.iter().all(|(slot, _)| slot != "_ShadowColorTex"));
        // 影色が抜けた分、17 枚目（マットキャップ 2nd のマスク）が層に入る
        assert!(layered
            .iter()
            .any(|(slot, _)| slot == "_MatCap2ndBlendMask"));
        assert!(!dropped_received(&doc, &drawn).contains(&"_MatCap2ndBlendMask".to_owned()));
        let layers: Vec<(&str, usize, bool)> = layered
            .iter()
            .enumerate()
            .map(|(k, (slot, image))| (slot.as_str(), k, image.srgb))
            .collect();
        let v = params_with(&doc, &drawn, &[], [None, None], &layers);
        assert_eq!(slot_src(&v, 7), [0, 1, 2, 3], "割り当てたチャンネル");
        assert_eq!(slot_flags(&v, 7), [0.0, 1.0, 0.0, 0.0]);
        // 標準の見た目を選ぶと、受けた絵は読まない
        let mut standard = doc.look().clone();
        standard.kind = LookKind::Standard;
        standard.kind_chosen = true;
        doc.set_look(standard, false).unwrap();
        assert!(received_images(&doc, doc.drawn_look()).is_empty());
        assert_eq!(planned_received_bytes(&doc, 1024, u64::MAX), 0);
    }

    #[test]
    fn an_unused_standard_channel_yields_to_the_received_image() {
        // 新しいセットの既定の割り当て（ノーマルマップ → Normal・発光 → Emission）は、どのレイヤーも使っていなければ Unity の絵に譲る
        let mut doc = Document::new(8, 8).unwrap();
        received_everywhere(&mut doc);
        let layer = doc.add_layer("a").unwrap();
        let mut mine = lil();
        crate::look::default_textures(&mut mine);
        doc.set_look(mine, false).unwrap();
        let drawn = doc.drawn_look().clone();
        let received: Vec<String> = received_images(&doc, &drawn)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert!(
            received.iter().any(|s| s == "_BumpMap"),
            "Normal は使っていない: {received:?}"
        );
        assert!(received.iter().any(|s| s == "_EmissionMap"));
        assert!(
            !received.iter().any(|s| s == "_MainTex"),
            "Color はレイヤーが使う"
        );
        // Normal を使い始めると、割り当てたチャンネルで描く
        doc.set_channel_enabled(layer, Channel::Normal, true)
            .unwrap();
        let drawn = doc.drawn_look().clone();
        let received: Vec<String> = received_images(&doc, &drawn)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert!(!received.iter().any(|s| s == "_BumpMap"));
        assert!(yields_to_received(
            &doc,
            Some(&TextureSource::Channel(Channel::Emission))
        ));
        assert!(!yields_to_received(
            &doc,
            Some(&TextureSource::Channel(Channel::Normal))
        ));
    }

    #[test]
    fn an_unused_standard_channel_reads_the_slot_default_when_unity_sends_no_picture() {
        // Unity が発光・ノーマルマップのテクスチャを空と知らせた（絵も理由も来ない）セット。新しいセットの既定の割り当て（発光 → Emission・
        // ノーマルマップ → Normal）はどのレイヤーも使っていないので、使っていないチャンネルの値（発光なら黒）ではなく、Unity と同じに
        // スロットの既定のテクスチャで読む
        let (emission, bump) = (
            liltoon::slot_index("_EmissionMap").unwrap(),
            liltoon::slot_index("_BumpMap").unwrap(),
        );
        let mut sink = lil();
        sink.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        let received = yolu_core::look::ReceivedLook {
            look: sink,
            ..Default::default()
        };
        let mut doc = Document::new(8, 8).unwrap();
        let mut mine = lil();
        crate::look::default_textures(&mut mine);
        doc.set_look(mine, false).unwrap();
        // 受けた見た目が無いとき（Live Link でつないでいない）は今までどおり、割り当てたチャンネルを読む
        let v = params_with(&doc, &doc.drawn_look().clone(), &[], [None, None], &[]);
        assert_eq!(
            slot_src(&v, emission)[0] / 4,
            paint_source(Channel::Emission).unwrap()
        );
        assert_eq!(
            slot_src(&v, bump)[0] / 4,
            paint_source(Channel::Normal).unwrap()
        );
        doc.set_received_look(Some(received)).unwrap();
        let drawn = doc.drawn_look().clone();
        assert!(yields_to_unity_texture(
            &doc,
            "_EmissionMap",
            drawn.textures.get("_EmissionMap")
        ));
        assert!(received_images(&doc, &drawn).is_empty(), "来る絵は無い");
        let v = params_with(&doc, &drawn, &[], [None, None], &[]);
        for i in [emission, bump] {
            assert_eq!(
                slot_src(&v, i),
                [-1; 4],
                "{}: 既定のテクスチャ",
                SLOTS[i].name
            );
            assert_eq!(slot_flags(&v, i), [0.0; 4], "{}", SLOTS[i].name);
        }
        // 流し込み先（受けた見た目が Color を割り当てる _MainTex）は、Color を使うレイヤーが無くても描いた絵（3D ビューの Color）で読む
        assert!(!yields_to_unity_texture(
            &doc,
            "_MainTex",
            drawn.textures.get("_MainTex")
        ));
        assert_eq!(slot_src(&v, 0), [0, 1, 2, 3]);
        assert_eq!(slot_flags(&v, 0), [0.0, 1.0, 0.0, 0.0]);
        // Normal を使い始めると、割り当てたチャンネルで描く（Emission は使っていないまま既定）
        let layer = doc.add_layer("a").unwrap();
        doc.set_channel_enabled(layer, Channel::Normal, true)
            .unwrap();
        let v = params_with(&doc, &doc.drawn_look().clone(), &[], [None, None], &[]);
        assert_eq!(
            slot_src(&v, bump)[0] / 4,
            paint_source(Channel::Normal).unwrap()
        );
        assert_eq!(slot_src(&v, emission), [-1; 4]);
        // 受けた絵が来れば、その絵が勝つ（既定ではなく）
        let mut received = doc.received_look().unwrap().clone();
        received
            .images
            .insert("_EmissionMap".into(), received_image(true));
        doc.set_received_look(Some(received)).unwrap();
        let drawn = doc.drawn_look().clone();
        let layered = received_layered(&doc, &drawn);
        let layers: Vec<(&str, usize, bool)> = layered
            .iter()
            .enumerate()
            .map(|(k, (slot, image))| (slot.as_str(), k, image.srgb))
            .collect();
        let v = params_with(&doc, &drawn, &[], [None, None], &layers);
        assert_eq!(
            slot_flags(&v, emission),
            [1.0, 1.0, RECEIVED_SOURCE as f32, 0.0]
        );
        // 読めない・まだ届かない理由（missing）が付いたスロットも、絵が来ない間は既定で描く
        let mut pending = doc.received_look().unwrap().clone();
        pending.images.clear();
        pending.missing.insert(
            "_EmissionMap".into(),
            yolu_core::look::MissingImage::Unreadable,
        );
        doc.set_received_look(Some(pending)).unwrap();
        let v = params_with(&doc, &doc.drawn_look().clone(), &[], [None, None], &[]);
        assert_eq!(slot_src(&v, emission), [-1; 4]);
    }

    #[test]
    fn rotating_the_sh_matches_reading_the_source_direction() {
        use yolu_core::glam::Vec3;
        let sh: [Vec3; 9] = std::array::from_fn(|i| {
            Vec3::new(
                0.3 + i as f32 * 0.1,
                -0.2 + i as f32 * 0.05,
                0.1 * (i as f32).sin(),
            )
        });
        let theta = 0.7f32;
        let rotated = rotate_sh_y(&sh, theta);
        let (s, c) = theta.sin_cos();
        let eval = |k: &[Vec3; 9], d: Vec3| -> Vec3 {
            k.iter().zip(sh_basis(d)).map(|(a, b)| *a * b).sum()
        };
        for i in 0..40 {
            let a = i as f32 * 0.37;
            let d = Vec3::new(
                a.sin() * (a * 1.7).cos(),
                (a * 0.9).cos(),
                a.cos() * (a * 1.3).sin(),
            )
            .normalize();
            let source = Vec3::new(d.x * c - d.z * s, d.y, d.x * s + d.z * c);
            assert!(
                (eval(&rotated, d) - eval(&sh, source)).length() < 1e-4,
                "{d:?}"
            );
        }
        // Unity の形で読んでも同じ値（ShadeSH9 の L0 + L1 + L2）
        let u = unity_sh(&rotated);
        let d = Vec3::new(0.3, 0.8, -0.52).normalize();
        let vb = [d.x * d.y, d.y * d.z, d.z * d.z, d.z * d.x];
        let shade = |k: usize| -> f32 {
            u[k][0] * d.x
                + u[k][1] * d.y
                + u[k][2] * d.z
                + u[k][3]
                + (0..4).map(|j| u[3 + k][j] * vb[j]).sum::<f32>()
                + u[6][k] * (d.x * d.x - d.y * d.y)
        };
        let expect = eval(&rotated, d);
        assert!(
            (shade(0) - expect.x).abs() < 1e-5
                && (shade(1) - expect.y).abs() < 1e-5
                && (shade(2) - expect.z).abs() < 1e-5
        );
    }

    #[test]
    fn the_standard_look_needs_nothing() {
        let doc = Document::new(8, 8).unwrap();
        let mut look = MaterialLook::default();
        look.textures.insert(
            "_ShadowStrengthMask".into(),
            TextureSource::Channel(Channel::from_index(9).unwrap()),
        );
        assert!(wanted_users(&doc, &look).is_empty());
        assert_eq!(SetDraw::of(&look), SetDraw::STANDARD);
        let mut l = lil();
        l.shader = "Hidden/lilToonTransparentOutline".into();
        l.properties.insert("_Cull".into(), LookValue::Float(0.0));
        let d = SetDraw::of(&l);
        assert_eq!(
            (d.lil, d.mode, d.outline, d.cull),
            (true, RenderMode::Transparent, true, 0)
        );
    }

    #[test]
    fn every_template_mask_fits_the_layers() {
        // 全部の機能を入にしてひな形を当てると、マスクのチャンネルが 9 つ（影の SDF なら 10）。どれも GPU の層に入る
        let mut doc = Document::new(8, 8).unwrap();
        let mut look = lil();
        for t in [
            "_UseShadow",
            "_UseRim",
            "_UseMatCap",
            "_UseMatCap2nd",
            "_UseEmission",
            "_UseEmission2nd",
        ] {
            look.properties.insert(t.into(), LookValue::Float(1.0));
        }
        look.properties
            .insert("_ShadowMaskType".into(), LookValue::Float(2.0));
        look.shader = "lilToonOutline".into();
        doc.set_look(look, false).unwrap();
        let made = crate::look::apply_template(&mut doc, crate::lang::Lang::Ja).unwrap();
        assert_eq!(made, 10);
        let users = wanted_users(&doc, doc.look());
        assert_eq!(users.len(), 10);
        assert!(users.len() <= MAX_LAYERS);
    }

    #[test]
    fn slots_read_up_to_sixteen_user_channels_and_the_rest_draw_their_default() {
        let mut doc = Document::new(8, 8).unwrap();
        let channels: Vec<Channel> = (0..18)
            .map(|i| {
                doc.add_channel(ChannelInfo {
                    name: format!("マスク {i}"),
                    kind: ChannelKind::Scalar,
                    color_space: ColorSpace::Linear,
                    default: Rgba8::new(255, 255, 255, 255),
                })
                .unwrap()
            })
            .collect();
        let mut look = lil();
        for (k, slot) in [
            "_ShadowStrengthMask",
            "_ShadowBorderMask",
            "_ShadowBlurMask",
            "_ShadowColorTex",
        ]
        .iter()
        .enumerate()
        {
            let planes = [0, 1, 2, 3].map(|j| PlaneSource::Channel {
                channel: channels[k * 4 + j],
                component: 0,
            });
            look.textures
                .insert((*slot).into(), TextureSource::Packed(planes));
        }
        look.textures.insert(
            "_Shadow2ndColorTex".into(),
            TextureSource::Channel(channels[16]),
        );
        look.textures.insert(
            "_Shadow3rdColorTex".into(),
            TextureSource::Channel(channels[17]),
        );
        let users = wanted_users(&doc, &look);
        assert_eq!(users, channels[..16].to_vec());
        assert_eq!(dropped_users(&doc, &look), channels[16..].to_vec());
        // 層に無いチャンネルのスロットは、割り当てのない既定で描く
        let v = params(&doc, &look, &users, [None, None]);
        // 詰め合わせの成分の元は（層の元の番号 × 4 + 成分）。層 12〜15 = 元 18〜21
        assert_eq!(slot_src(&v, 7), [18 * 4, 19 * 4, 20 * 4, 21 * 4]);
        assert_eq!(slot_src(&v, 8), [-1; 4]);
        assert_eq!(slot_src(&v, 9), [-1; 4]);
        // 標準の見た目では読まない
        assert!(dropped_users(&doc, &MaterialLook::default()).is_empty());
    }
}
