// lilToon の再現。scene.wgsl の後ろにつないで 1 つのモジュールにする（一様バッファ・束ね・色の変換・影は scene.wgsl のもの）。
//
// 式は lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の Shader/Includes から移した: lil_pass_forward_normal.hlsl の frag の順、
// lil_common_frag.hlsl の lilGetMain2nd・lilGetMain3rd・lilGetShading・lilGetRimShade・lilBacklight・lilReflection・lilCalcSpecular・
// lilGetMatCap・lilGetMatCap2nd・lilGetRim・lilGlitter・lilEmission・lilEmission2nd・lilDistanceFade・アルファマスク・ノーマルマップ 2nd・
// 異方性反射・輪郭線の色、lil_common_functions.hlsl の lilTooning*・lilToneCorrection・lilBlendColor・lilBlendNormal・lilCalcMatCapUV・
// lilUnpackNormalScale・lilCalcDecalUV・lilCalcAtlasAnimation・lilGetSubTex・lilIsIn0to1・lilMSDF・lilVoronoi・lilCalcGlitter・
// lilGetAnisotropyNormalWS・lilFresnelTerm・lilFresnelLerp・GSAA・lilGetOutlineWidth・lilCalcOutlinePosition、
// lil_common_functions_thirdparty.hlsl の lilHashRGB4、ビルトインのレンダーパイプラインの光（openlit_core.hlsl の ComputeLights。
// OpenLit は CC0）と lil_common_macro.hlsl の LIL_CORRECT_LIGHTCOLOR。機能の入切は、エディタの既定（全部の機能を組み込んだ版）と同じ。
// 時間で動く値（スクロール・回転の速さ・点滅・デカールのアニメーション・ラメの点滅）は時刻 0 の値で描く。
//
// 再現しないもの（値は持つが描かない）: グラデーションマップ、ディゾルブ、ディザー、視差、AudioLink、ID マスク、ファー・宝石・屈折、
// 影色の LUT、UV1〜UV3（モデルは UV0 だけ。UV0 で読む）、頂点カラー、ラメの形のテクスチャ、環境光の反射のキューブマップの差し替え、
// 追加のライト（頂点ライト・ForwardAdd）、ライトマップ、霧。
//
// 色の約束: lilToon はリニアで解き、最後に scene.wgsl と同じくガンマにして書く。半透明は、描き先を sRGB の見え方にして（`*_linear` の
// 入り口）リニアの乗算済みで重ねる（Unity と同じ重ね方）。トーンマッピングのとき（描き先が HDR）は、ガンマの値のまま重ねる。
//
// パイプラインの定数（`LIL_FEATURES`・`LIL_SINGLE*`・`LIL_LOOP*`）: ソフトの描画（llvmpipe）は一様な分岐の先も全部実行するので、
// 入にしていない機能と、割り当ての無いスロットの読み方を、パイプラインを作るときに外す（render.rs）。既定は全部入りで、実機は
// その 1 本を一様な分岐で描く（入切のたびにシェーダーを作り直さない）。どちらも同じ値の分岐を通るので、描く絵は同じ。

// ───────── 束ね（group 1 の 7〜10） ─────────

const NP: i32 = 122;
const NS: i32 = 38;
const NU: i32 = 16;

struct Lil {
    p: array<vec4<f32>, 122>,
    // スロットごとの成分の元（-1 既定・-2 は 0・-3 は 1・0 以上は 元 × 4 + 成分。元は 0〜5 標準のチャンネル（Slot の番号）・
    // 6〜21 ユーザーチャンネルの配列の層・32〜33 画像・40〜55 Unity から受けた絵の配列の層）
    slot_src: array<vec4<i32>, 38>,
    // スロットの既定の値（割り当てていない成分）
    slot_def: array<vec4<f32>, 38>,
    // x: RGB を sRGB からリニアへ（1）、y: 1 つの元の RGBA をそのまま（1）、z: その元の番号
    slot_flags: array<vec4<f32>, 38>,
    // ユーザーチャンネルの配列の層ごとの、何も描いていない所の値（ガンマのまま。スカラーは R）
    user_default: array<vec4<f32>, 16>,
};

@group(1) @binding(7) var<uniform> lil: Lil;
@group(1) @binding(8) var user_tex: texture_2d_array<f32>;
@group(1) @binding(9) var image1_tex: texture_2d<f32>;
@group(1) @binding(10) var image2_tex: texture_2d<f32>;
// Live Link で Unity から受けた、描いていないスロットの絵（層 1 つがスロット 1 つ。元の番号 40〜55。Unity が読むのと同じ straight で、
// 割り戻さない。A は不透明度とは限らない: DXT5nm のノーマルマップは A に X を持つ）
@group(1) @binding(11) var received_tex: texture_2d_array<f32>;
const NR: i32 = 16;

// ───────── パイプラインの定数（既定は全部入り） ─────────

// 機能のビット（F_*）。0 のビットの機能はパイプラインに作らない
override LIL_FEATURES: u32 = 0xffffffffu;
// スロットの読み方のビット（スロットの番号。0〜31 と 32〜63）: 1 つの元をそのまま読む・成分ごとに読む。どちらも 0 のスロットは既定の値
override LIL_SINGLE0: u32 = 0xffffffffu;
override LIL_SINGLE1: u32 = 0xffffffffu;
override LIL_LOOP0: u32 = 0xffffffffu;
override LIL_LOOP1: u32 = 0xffffffffu;

const F_SHADOW: u32 = 0u;
const F_BUMP: u32 = 1u;
const F_BUMP2: u32 = 2u;
const F_MAIN2: u32 = 3u;
const F_MAIN3: u32 = 4u;
const F_ALPHA_MASK: u32 = 5u;
const F_RIM_SHADE: u32 = 6u;
const F_BACKLIGHT: u32 = 7u;
const F_REFLECTION: u32 = 8u;
const F_MATCAP: u32 = 9u;
const F_MATCAP2: u32 = 10u;
const F_RIM: u32 = 11u;
const F_GLITTER: u32 = 12u;
const F_EMISSION: u32 = 13u;
const F_EMISSION2: u32 = 14u;
const F_ANISO: u32 = 15u;
const F_DISTANCE_FADE: u32 = 16u;
const F_BACKFACE: u32 = 17u;

fn feat(bit: u32) -> bool {
    return (LIL_FEATURES & (1u << bit)) != 0u;
}

fn slot_bit(lo: u32, hi: u32, i: i32) -> bool {
    if (i < 32) {
        return (lo & (1u << u32(i))) != 0u;
    }
    return (hi & (1u << u32(i - 32))) != 0u;
}

// 値の並び（look_gpu.rs の `params` と同じ）
const P_MODE: i32 = 0;            // x 描画モード（0 不透明・1 カットアウト・2 半透明）、y Cutoff、z Cull、w 非表示
const P_COLOR: i32 = 1;           // _Color（リニア）
const P_MAIN_ST: i32 = 2;
const P_HSVG: i32 = 3;
const P_LIGHT: i32 = 4;           // x 下限、y 上限、z モノクロ化、w Unlit 化
const P_LIGHT2: i32 = 5;          // x 影色への環境光、y AA、z 裏面を影に、w 裏面の法線を反転
const P_LIGHT_DIR: i32 = 6;       // _LightDirectionOverride
const P_BACKFACE: i32 = 7;        // _BackfaceColor（リニア）
const P_ALPHA_MASK: i32 = 8;      // x モード、y スケール、z オフセット
const P_SHADOW: i32 = 9;          // x 入、y 強度、z マスクの種類、w 影範囲設定を無視
const P_SHADOW_LOD: i32 = 10;     // x 強度マスク、y AO、z ぼかしマスク
const P_SHADOW_FLAT: i32 = 11;    // x 平面の範囲、y 平面のぼかし、z 境界の幅、w コントラスト
const P_SHADOW1_COLOR: i32 = 12;
const P_SHADOW1: i32 = 13;        // x 範囲、y ぼかし、z ノーマルマップ強度、w 影を受け取る
const P_SHADOW2_COLOR: i32 = 14;
const P_SHADOW2: i32 = 15;
const P_SHADOW3_COLOR: i32 = 16;
const P_SHADOW3: i32 = 17;
const P_SHADOW_BORDER_COLOR: i32 = 18;
const P_AO_SHIFT: i32 = 19;
const P_AO_SHIFT2: i32 = 20;
const P_EMISSION: i32 = 21;       // x 入、y 強度、z 合成モード、w メインカラーの強度
const P_EMISSION_COLOR: i32 = 22;
const P_EMISSION_ST: i32 = 23;
const P_EMISSION_MASK_ST: i32 = 24;
const P_EMISSION_X: i32 = 25;     // x 蛍光、y UV、z テクスチャの角度、w マスクの角度
const P_EMISSION2: i32 = 26;
const P_EMISSION2_COLOR: i32 = 27;
const P_EMISSION2_ST: i32 = 28;
const P_EMISSION2_MASK_ST: i32 = 29;
const P_EMISSION2_X: i32 = 30;
const P_BUMP: i32 = 31;           // x 入、y 強度
const P_BUMP_ST: i32 = 32;
const P_MATCAP: i32 = 33;         // x 入、y 強度、z 合成モード、w メインカラーの強度
const P_MATCAP_COLOR: i32 = 34;
const P_MATCAP_ST: i32 = 35;
const P_MATCAP_MASK_ST: i32 = 36;
const P_MATCAP_P: i32 = 37;       // x ライトの明るさ、y 影部分で無効化、z 裏面で無効化、w ぼかし
const P_MATCAP_Q: i32 = 38;       // x ノーマルマップ強度、y Z 軸回転キャンセル、z パース補正、w 透明度を適用
const P_MATCAP2: i32 = 39;
const P_MATCAP2_COLOR: i32 = 40;
const P_MATCAP2_ST: i32 = 41;
const P_MATCAP2_MASK_ST: i32 = 42;
const P_MATCAP2_P: i32 = 43;
const P_MATCAP2_Q: i32 = 44;
const P_RIM: i32 = 45;            // x 入、y 合成モード、z メインカラーの強度、w ノーマルマップ強度
const P_RIM_COLOR: i32 = 46;
const P_RIM_ST: i32 = 47;
const P_RIM_P: i32 = 48;          // x 範囲、y ぼかし、z 細さ、w ライトの明るさ
const P_RIM_Q: i32 = 49;          // x 影部分で無効化、y 裏面で無効化、z 透明度を適用、w ライト方向の影響度
const P_RIM_R: i32 = 50;          // x 直接光の幅、y 間接光の幅、z 間接光の範囲、w 間接光のぼかし
const P_RIM_INDIR_COLOR: i32 = 51;
const P_OUTLINE: i32 = 52;        // x 入、y 太さ、z 距離補正、w ライトの明るさ
const P_OUTLINE_COLOR: i32 = 53;
const P_OUTLINE_ST: i32 = 54;
const P_OUTLINE_HSVG: i32 = 55;
const P_OUTLINE_LIT_COLOR: i32 = 56;
const P_OUTLINE_P: i32 = 57;      // x スケール、y オフセット、z メインカラーから、w 影を受け取る
const P_OUTLINE_Q: i32 = 58;      // x Z バイアス、y 太さ 0 を消す、z テクスチャの角度
const P_FLAGS: i32 = 59;          // x 法線マップを読める（接線がある）
const P_UV: i32 = 60;             // x メインの UV の角度、y 裏面の UV をずらす
// メインカラー 2nd（61〜69）・3rd（70〜78）: 同じ並び（L_*）
const P_LAYER2: i32 = 61;
const P_LAYER3: i32 = 70;
const L_P: i32 = 0;               // x 入、y ライトの明るさ、z 合成モード、w 透過モード
const L_COLOR: i32 = 1;           // 色（リニア）
const L_ST: i32 = 2;
const L_Q: i32 = 3;               // x 角度、y UV（0 UV0・4 マットキャップ）、z Cull、w MSDF
const L_DECAL: i32 = 4;           // x デカール、y 左のみ、z 右のみ、w 複製
const L_DECAL2: i32 = 5;          // x ミラーの反転、y 複製の反転
const L_ANIM: i32 = 6;            // _<名>TexDecalAnimation
const L_SUB: i32 = 7;             // _<名>TexDecalSubParam
const L_FADE: i32 = 8;            // _<名>DistanceFade
const P_BUMP2: i32 = 79;          // x 入、y 強度
const P_BUMP2_ST: i32 = 80;
const P_RIM_SHADE: i32 = 81;      // x 入、y ノーマルマップ強度、z 範囲、w ぼかし
const P_RIM_SHADE_COLOR: i32 = 82;
const P_RIM_SHADE2: i32 = 83;     // x 細さ
const P_BACKLIGHT: i32 = 84;      // x 入、y メインカラーの強度、z 影を受け取る、w 裏面で無効化
const P_BACKLIGHT_COLOR: i32 = 85;
const P_BACKLIGHT_P: i32 = 86;    // x ノーマルマップ強度、y 範囲、z ぼかし、w 指向性
const P_BACKLIGHT_Q: i32 = 87;    // x 視線方向の影響度
const P_BACKLIGHT_ST: i32 = 88;
const P_REFL: i32 = 89;           // x 入、y 滑らかさ、z 金属度（リニア）、w 反射率（リニア）
const P_REFL_COLOR: i32 = 90;
const P_REFL_P: i32 = 91;         // x 光沢を出す、y トゥーン、z 光沢のノーマルマップ強度、w 光沢の範囲
const P_REFL_Q: i32 = 92;         // x 光沢のぼかし、y 環境光の反射、z 反射のノーマルマップ強度、w 透明度を適用
const P_REFL_R: i32 = 93;         // x 合成モード、y GSAA
const P_SMOOTH_ST: i32 = 94;
const P_METAL_ST: i32 = 95;
const P_REFL_COLOR_ST: i32 = 96;
const P_GLITTER: i32 = 97;        // x 入、y UV、z メインカラーの強度、w ライトの明るさ
const P_GLITTER_COLOR: i32 = 98;
const P_GLITTER_ST: i32 = 99;
const P_GLITTER_P1: i32 = 100;    // _GlitterParams1
const P_GLITTER_P2: i32 = 101;    // _GlitterParams2
const P_GLITTER_Q: i32 = 102;     // x 影部分で無効化、y 裏面で無効化、z 透明度を適用、w ノーマルマップ強度
const P_GLITTER_R: i32 = 103;     // x コントラスト（後処理）、y 感度、z 大きさのランダム化
const P_ANISO: i32 = 104;         // x 入、y スケール、z 反射へ、w マットキャップへ
const P_ANISO_Q: i32 = 105;       // x マットキャップ 2nd へ
const P_ANISO1: i32 = 106;        // タンジェント方向の幅・バイタンジェント方向の幅・オフセット・ノイズの強度（1st）
const P_ANISO2: i32 = 107;        // 同じ（2nd）
const P_ANISO_S: i32 = 108;       // x 1st の強度、y 2nd の強度
const P_ANISO_ST: i32 = 109;
const P_ANISO_MASK_ST: i32 = 110;
const P_ANISO_NOISE_ST: i32 = 111;
const P_DFADE_COLOR: i32 = 112;
const P_DFADE: i32 = 113;         // _DistanceFade
const P_DFADE_RIM_COLOR: i32 = 114;
const P_DFADE_P: i32 = 115;       // x モード、y リムの細さ
const P_MC_BUMP: i32 = 116;       // x 1st のカスタムノーマル、y その強度、z 2nd のカスタムノーマル、w その強度
const P_MC_BUMP_ST: i32 = 117;
const P_MC2_BUMP_ST: i32 = 118;
const P_ALPHA_MASK_ST: i32 = 119;
const P_DFADE_ORIGIN: i32 = 120;  // モデルの原点（座標のモードの距離）

// スロット（liltoon.rs の SLOTS と同じ並び）
const SLOT_MAIN: i32 = 0;
const SLOT_ADJUST_MASK: i32 = 1;
const SLOT_ALPHA_MASK: i32 = 2;
const SLOT_BUMP: i32 = 3;
const SLOT_SHADOW_STRENGTH: i32 = 4;
const SLOT_SHADOW_BORDER: i32 = 5;
const SLOT_SHADOW_BLUR: i32 = 6;
const SLOT_SHADOW1_TEX: i32 = 7;
const SLOT_SHADOW2_TEX: i32 = 8;
const SLOT_SHADOW3_TEX: i32 = 9;
const SLOT_EMISSION: i32 = 10;
const SLOT_EMISSION_MASK: i32 = 11;
const SLOT_EMISSION2: i32 = 12;
const SLOT_EMISSION2_MASK: i32 = 13;
const SLOT_MATCAP: i32 = 14;
const SLOT_MATCAP_MASK: i32 = 15;
const SLOT_MATCAP2: i32 = 16;
const SLOT_MATCAP2_MASK: i32 = 17;
const SLOT_RIM: i32 = 18;
const SLOT_OUTLINE: i32 = 19;
const SLOT_OUTLINE_WIDTH: i32 = 20;
const SLOT_MAIN2: i32 = 21;
const SLOT_MAIN2_MASK: i32 = 22;
const SLOT_MAIN3: i32 = 23;
const SLOT_MAIN3_MASK: i32 = 24;
const SLOT_BUMP2: i32 = 25;
const SLOT_BUMP2_MASK: i32 = 26;
const SLOT_RIM_SHADE_MASK: i32 = 27;
const SLOT_BACKLIGHT: i32 = 28;
const SLOT_SMOOTHNESS: i32 = 29;
const SLOT_METALLIC: i32 = 30;
const SLOT_REFL_COLOR: i32 = 31;
const SLOT_GLITTER: i32 = 32;
const SLOT_ANISO_TANGENT: i32 = 33;
const SLOT_ANISO_MASK: i32 = 34;
const SLOT_ANISO_NOISE: i32 = 35;
const SLOT_MATCAP_BUMP: i32 = 36;
const SLOT_MATCAP2_BUMP: i32 = 37;

// ───────── テクスチャのスロットを読む ─────────

fn unpremultiply(c: vec4<f32>) -> vec4<f32> {
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}

// 元を 1 つ読む（明示の勾配。標準のチャンネルは 3D ビューの絵の約束（render.rs）、ユーザーチャンネルは何も描いていない所の値に重ねた値）。
fn fetch_grad(src: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
    if (src == 0) {
        return unpremultiply(textureSampleGrad(color_tex, paint_sampler, uv, gx, gy));
    } else if (src == 1) {
        let v = textureSampleGrad(metallic_tex, paint_sampler, uv, gx, gy).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src == 2) {
        let v = textureSampleGrad(roughness_tex, paint_sampler, uv, gx, gy).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src == 3) {
        return vec4<f32>(textureSampleGrad(normal_tex, paint_sampler, uv, gx, gy).rgb, 1.0);
    } else if (src == 4) {
        return vec4<f32>(textureSampleGrad(emission_tex, paint_sampler, uv, gx, gy).rgb, 1.0);
    } else if (src == 5) {
        let v = textureSampleGrad(height_tex, paint_sampler, uv, gx, gy).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src >= 6 && src < 6 + NU) {
        let layer = src - 6;
        let t = textureSampleGrad(user_tex, paint_sampler, uv, layer, gx, gy);
        let d = lil.user_default[layer];
        return t + d * (1.0 - t.a);
    } else if (src == 32) {
        return unpremultiply(textureSampleGrad(image1_tex, paint_sampler, uv, gx, gy));
    } else if (src == 33) {
        return unpremultiply(textureSampleGrad(image2_tex, paint_sampler, uv, gx, gy));
    } else if (src >= 40 && src < 40 + NR) {
        return textureSampleGrad(received_tex, paint_sampler, uv, src - 40, gx, gy);
    }
    return vec4<f32>(0.0);
}

// 同じ、段（mip）を決めて読む（マットキャップのぼかし・頂点の輪郭線の太さ）。
fn fetch_level(src: i32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    if (src == 0) {
        return unpremultiply(textureSampleLevel(color_tex, paint_sampler, uv, lod));
    } else if (src == 1) {
        let v = textureSampleLevel(metallic_tex, paint_sampler, uv, lod).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src == 2) {
        let v = textureSampleLevel(roughness_tex, paint_sampler, uv, lod).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src == 3) {
        return vec4<f32>(textureSampleLevel(normal_tex, paint_sampler, uv, lod).rgb, 1.0);
    } else if (src == 4) {
        return vec4<f32>(textureSampleLevel(emission_tex, paint_sampler, uv, lod).rgb, 1.0);
    } else if (src == 5) {
        let v = textureSampleLevel(height_tex, paint_sampler, uv, lod).r;
        return vec4<f32>(v, v, v, 1.0);
    } else if (src >= 6 && src < 6 + NU) {
        let layer = src - 6;
        let t = textureSampleLevel(user_tex, paint_sampler, uv, layer, lod);
        let d = lil.user_default[layer];
        return t + d * (1.0 - t.a);
    } else if (src == 32) {
        return unpremultiply(textureSampleLevel(image1_tex, paint_sampler, uv, lod));
    } else if (src == 33) {
        return unpremultiply(textureSampleLevel(image2_tex, paint_sampler, uv, lod));
    } else if (src >= 40 && src < 40 + NR) {
        return textureSampleLevel(received_tex, paint_sampler, uv, src - 40, lod);
    }
    return vec4<f32>(0.0);
}

// 詰め合わせ（成分ごと）で読む値: 書き出しの「lilToon の詰め方」と同じく、チャンネルの値のまま。Color・Emission は sRGB の形式で
// GPU がリニアにして読むので、ガンマの値へ戻す
fn raw_value(src: i32, v: vec4<f32>) -> vec4<f32> {
    if (src == 0 || src == 4) {
        return vec4<f32>(linear_to_srgb(v.rgb), v.a);
    }
    return v;
}

fn component(v: vec4<f32>, k: i32) -> f32 {
    if (k == 0) {
        return v.x;
    } else if (k == 1) {
        return v.y;
    } else if (k == 2) {
        return v.z;
    }
    return v.w;
}

fn finish_slot(i: i32, v: vec4<f32>) -> vec4<f32> {
    if (lil.slot_flags[i].x > 0.5) {
        return vec4<f32>(srgb_to_linear(v.rgb), v.a);
    }
    return v;
}

// スロットを読む（割り当てていない成分は既定）。
fn slot_grad(i: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
    let flags = lil.slot_flags[i];
    if (slot_bit(LIL_SINGLE0, LIL_SINGLE1, i) && flags.y > 0.5) {
        return finish_slot(i, fetch_grad(i32(flags.z + 0.5), uv, gx, gy));
    }
    if (slot_bit(LIL_LOOP0, LIL_LOOP1, i) && flags.y < 0.5) {
        let src = lil.slot_src[i];
        var out = lil.slot_def[i];
        for (var k = 0; k < 4; k = k + 1) {
            let code = src[k];
            if (code >= 0) {
                out[k] = component(raw_value(code / 4, fetch_grad(code / 4, uv, gx, gy)), code % 4);
            } else if (code == -2) {
                out[k] = 0.0;
            } else if (code == -3) {
                out[k] = 1.0;
            }
        }
        return finish_slot(i, out);
    }
    return lil.slot_def[i];
}

fn slot_level(i: i32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let flags = lil.slot_flags[i];
    if (slot_bit(LIL_SINGLE0, LIL_SINGLE1, i) && flags.y > 0.5) {
        return finish_slot(i, fetch_level(i32(flags.z + 0.5), uv, lod));
    }
    if (slot_bit(LIL_LOOP0, LIL_LOOP1, i) && flags.y < 0.5) {
        let src = lil.slot_src[i];
        var out = lil.slot_def[i];
        for (var k = 0; k < 4; k = k + 1) {
            let code = src[k];
            if (code >= 0) {
                out[k] = component(raw_value(code / 4, fetch_level(code / 4, uv, lod)), code % 4);
            } else if (code == -2) {
                out[k] = 0.0;
            } else if (code == -3) {
                out[k] = 1.0;
            }
        }
        return finish_slot(i, out);
    }
    return lil.slot_def[i];
}

// Unity から受けた絵を 1 つの元として読むスロットか（ノーマルマップの X を A × R で読む）。
fn slot_received(i: i32) -> bool {
    let flags = lil.slot_flags[i];
    return flags.y > 0.5 && flags.z >= 40.0;
}

// ───────── lilToon の関数 ─────────

// scene.wgsl の saturate は数だけ（同じ名前の組み込みを隠す）なので、ベクトルはこちら
fn sat3(v: vec3<f32>) -> vec3<f32> {
    return clamp(v, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn lil_tooning_ns(aa: f32, value: f32, border: f32, blur: f32, fw: f32) -> f32 {
    let bmin = saturate(border - blur * 0.5);
    let bmax = saturate(border + blur * 0.5);
    return (value - bmin) / saturate(bmax - bmin + fw * aa);
}

fn lil_tooning_ns_range(aa: f32, value: f32, border: f32, blur: f32, range: f32, fw: f32) -> f32 {
    let bmin = saturate(border - blur * 0.5 - range);
    let bmax = saturate(border + blur * 0.5);
    return (value - bmin) / saturate(bmax - bmin + fw * aa);
}

// lilTooningScale(aa, value, border)（ぼかしの無い形）
fn lil_tooning_step(aa: f32, value: f32, border: f32, fw: f32) -> f32 {
    return saturate((value - border) / clamp(fw * aa, 0.0001, 1.0));
}

fn lil_blend(dst: vec3<f32>, src: vec3<f32>, a: vec3<f32>, mode: i32) -> vec3<f32> {
    let ad = dst + src;
    let mu = dst * src;
    var out = src;
    if (mode == 1) {
        out = ad;
    } else if (mode == 2) {
        out = max(ad - mu, dst);
    } else if (mode == 3) {
        out = mu;
    }
    return mix(dst, out, a);
}

fn lil_tone(c_in: vec3<f32>, hsvg: vec4<f32>) -> vec3<f32> {
    let c = pow(abs(c_in), vec3<f32>(hsvg.w));
    var p = vec4<f32>(c.g, c.b, 0.0, -1.0 / 3.0);
    if (c.b > c.g) {
        p = vec4<f32>(c.b, c.g, -1.0, 2.0 / 3.0);
    }
    var q = vec4<f32>(c.r, p.y, p.z, p.x);
    if (p.x > c.r) {
        q = vec4<f32>(p.x, p.y, p.w, c.r);
    }
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    var hsv = vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
    hsv = vec3<f32>(hsv.x + hsvg.x, saturate(hsv.y * hsvg.y), saturate(hsv.z * hsvg.z));
    let k = sat3(abs(fract(vec3<f32>(hsv.x) + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0);
    return vec3<f32>(hsv.z - hsv.z * hsv.y) + hsv.z * hsv.y * k;
}

fn lil_gray(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(1.0 / 3.0));
}

fn lil_unpack_normal(t: vec4<f32>, scale: f32, ag: bool) -> vec3<f32> {
    // Unity のノーマルマップの取り込み（DXT5nm）と同じく、XY から Z を作り直す。`ag` は Unity から受けた絵: X は lilUnpackNormalScale と
    // 同じく A × R（RGB の絵は A が 1 で R、DXT5nm の絵は R が 1 で A）
    var x = t.x;
    if (ag) {
        x = t.w * t.x;
    }
    let xy = (vec2<f32>(x, t.y) * 2.0 - vec2<f32>(1.0)) * scale;
    return vec3<f32>(xy, sqrt(1.0 - saturate(dot(xy, xy))));
}

// lilBlendNormal（接空間の法線を重ねる）
fn lil_blend_normal(dst: vec3<f32>, src: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dst.xy + src.xy, dst.z * src.z);
}

fn ortho_normalize(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return normalize(t - n * dot(n, t));
}

// lilRotateUV（UV の中心 0.5 のまわりに回す）
fn rotate_uv(uv: vec2<f32>, angle: f32) -> vec2<f32> {
    let s = sin(angle);
    let c = cos(angle);
    let o = uv - vec2<f32>(0.5);
    return vec2<f32>(o.x * c - o.y * s, o.x * s + o.y * c) + vec2<f32>(0.5);
}

fn matcap_uv(n: vec3<f32>, v: vec3<f32>, st: vec4<f32>, zrot_cancel: bool, perspective: bool) -> vec2<f32> {
    var nvd = v;
    if (!perspective) {
        nvd = u.lil_camera_front.xyz;
    }
    var bvd = u.lil_camera_up.xyz;
    if (zrot_cancel) {
        bvd = vec3<f32>(0.0, 1.0, 0.0);
    }
    bvd = ortho_normalize(bvd, nvd);
    let tvd = cross(nvd, bvd);
    var uv = vec2<f32>(dot(tvd, n), dot(bvd, n));
    uv = uv * st.xy + st.zw;
    return uv * 0.5 + 0.5;
}

// lilIsIn0to1(f, nv)（アンチエイリアスつき。`fw` は f の fwidth）
fn lil_in01(f: f32, fw: f32, nv: f32) -> f32 {
    let value = 0.5 - abs(f - 0.5);
    return saturate(value / clamp(fw, 0.0001, nv));
}

// ビルトインのレンダーパイプラインの主な光（OpenLit の ComputeLights。SH は Unity の unity_SH* の形）。
struct LilLight {
    dir: vec3<f32>,
    color: vec3<f32>,
    ind: vec3<f32>,
};

fn lil_sh_l0l2(n: vec3<f32>) -> vec3<f32> {
    let vb = n.xyzz * n.yzzx;
    var res = vec3<f32>(u.lil_sh[0].w, u.lil_sh[1].w, u.lil_sh[2].w);
    res = res + vec3<f32>(dot(u.lil_sh[3], vb), dot(u.lil_sh[4], vb), dot(u.lil_sh[5], vb));
    res = res + u.lil_sh[6].rgb * (n.x * n.x - n.y * n.y);
    return res;
}

fn lil_sh_l1(n: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(u.lil_sh[0].xyz, n), dot(u.lil_sh[1].xyz, n), dot(u.lil_sh[2].xyz, n));
}

fn lil_main_light() -> LilLight {
    var o: LilLight;
    let main_color = u.light_color.rgb;
    let lum = dot(main_color, vec3<f32>(0.0396819152, 0.458021790, 0.00609653955));
    let main_dir = u.light_dir.xyz * lum;
    let sh9 = (u.lil_sh[0].xyz + u.lil_sh[1].xyz + u.lil_sh[2].xyz) * 0.333333;
    let sh9_abs = vec3<f32>(sh9.x, abs(sh9.y), sh9.z);
    let ov = lil.p[P_LIGHT_DIR];
    // オーバーライドの w が 0 でなければ物の空間の向き（モデルは世界の空間なので、そのまま。長さは保つ）
    let custom = ov.xyz;
    o.dir = normalize(sh9_abs + main_dir + custom);
    let res = lil_sh_l0l2(o.dir);
    let sh_max = res + lil_sh_l1(o.dir);
    let sum = u.lil_sh[0].xyz + u.lil_sh[1].xyz + u.lil_sh[2].xyz;
    // SH の 1 次の項が無い（一様な環境光）とき、Unity は normalize(0) で NaN になり、使う所（saturate）で 0 になる。同じ結果にする
    if (dot(sum, sum) > 0.0) {
        o.ind = res + lil_sh_l1(normalize(sum));
    } else {
        o.ind = vec3<f32>(0.0);
    }
    // LIL_CORRECT_LIGHTCOLOR_VS
    let l = lil.p[P_LIGHT];
    var c = clamp(sh_max + main_color, vec3<f32>(l.x), vec3<f32>(l.y));
    c = mix(c, vec3<f32>(lil_gray(c)), l.z);
    c = mix(c, vec3<f32>(1.0), l.w);
    o.color = c;
    return o;
}

// 乗算済みのリニアの色をガンマで書く（半透明は乗算済みのガンマ）。
fn lil_output(col: vec4<f32>, transparent: bool) -> vec4<f32> {
    if (transparent) {
        if (col.a <= 0.0) {
            return vec4<f32>(0.0);
        }
        return vec4<f32>(linear_to_srgb(col.rgb / col.a) * col.a, col.a);
    }
    return vec4<f32>(linear_to_srgb(col.rgb), 1.0);
}

// アルファマスク（lilToon の OVERRIDE_ALPHAMASK。不透明の描画モードでは使わない）。
fn apply_alpha_mask(a: f32, mask_tex: f32, mode: i32) -> f32 {
    let am = lil.p[P_ALPHA_MASK];
    var out = a;
    if (mode != 0 && am.x > 0.5) {
        let mask = saturate(mask_tex * am.y + am.z);
        let m = i32(am.x + 0.5);
        if (m == 1) {
            out = mask;
        } else if (m == 2) {
            out = a * mask;
        } else if (m == 3) {
            out = saturate(a + mask);
        } else if (m == 4) {
            out = saturate(a - mask);
        }
    }
    return out;
}

// ───────── メインカラー 2nd・3rd（lilGetMain2nd・lilGetSubTex・lilCalcDecalUV・lilCalcAtlasAnimation） ─────────

// 重ねる層の色（リニア。A が重ねる強さ）と、描画モードが不透明でないときの透過モードの結果のアルファ。
struct LayerOut {
    color: vec4<f32>,
    alpha: f32,
};

fn decal_uv(uv: vec2<f32>, st: vec4<f32>, angle: f32, d: vec4<f32>, d2: vec4<f32>, right_hand: bool) -> vec2<f32> {
    var o = uv;
    // 複製
    if (d.w > 0.5) {
        o.x = abs(o.x - 0.5) + 0.5;
    }
    o = o * st.xy + st.zw;
    // 反転
    if (d2.y > 0.5 && uv.x < 0.5) {
        o.x = 1.0 - o.x;
    }
    if (d2.x > 0.5 && right_hand) {
        o.x = 1.0 - o.x;
    }
    // 隠す
    if (d.y > 0.5 && right_hand) {
        o.x = -1.0;
    }
    if (d.z > 0.5 && !right_hand) {
        o.x = -1.0;
    }
    // 回す
    o = (o - st.zw) / st.xy;
    o = rotate_uv(o, angle);
    return o * st.xy + st.zw;
}

// lilCalcAtlasAnimation（時刻 0: FPS が 0 なら合計フレーム数の番号のこま、ほかは 0 番）
fn atlas_uv(uv: vec2<f32>, anim: vec4<f32>, sub: vec4<f32>) -> vec2<f32> {
    var t = 0u;
    if (anim.w == 0.0) {
        t = u32(max(anim.z, 0.0));
    }
    let fx = max(u32(max(anim.x, 1.0)), 1u);
    var o = mix(vec2<f32>(uv.x, 1.0 - uv.y), vec2<f32>(0.5), sub.z);
    let ox = f32(t % fx);
    let oy = f32(t / fx);
    o = (o + vec2<f32>(ox, oy)) * sub.xy / max(anim.xy, vec2<f32>(1e-6));
    o.y = 1.0 - o.y;
    return o;
}

fn lil_layer(
    base: i32,
    slot: i32,
    mask_slot: i32,
    uv0: vec2<f32>,
    uv_mat: vec2<f32>,
    uv_main: vec2<f32>,
    gx_main: vec2<f32>,
    gy_main: vec2<f32>,
    nv: f32,
    depth: f32,
    facing: f32,
    right_hand: bool,
    col_a: f32,
    mode: i32,
) -> LayerOut {
    let lp = lil.p[base + L_P];
    let q = lil.p[base + L_Q];
    let st = lil.p[base + L_ST];
    let d = lil.p[base + L_DECAL];
    let d2 = lil.p[base + L_DECAL2];
    var c = lil.p[base + L_COLOR];
    var uv = uv0;
    if (i32(q.y + 0.5) == 4) {
        uv = uv_mat;
    }
    let uv2 = decal_uv(uv, st, q.x, d, d2, right_hand);
    let samp = atlas_uv(uv2, lil.p[base + L_ANIM], lil.p[base + L_SUB]);
    let tex = slot_grad(slot, samp, dpdx(samp), dpdy(samp));
    var t = tex;
    // MSDF（テクスチャの 3 成分の中央値を距離として読む）
    let sd = max(min(tex.r, tex.g), min(max(tex.r, tex.g), tex.b));
    let msdf = saturate((sd - 0.5) / clamp(fwidth(sd), 0.01, 1.0));
    if (q.w > 0.5) {
        t = vec4<f32>(1.0, 1.0, 1.0, msdf);
    }
    // デカール（UV が 0〜1 の外は描かない。縁はアンチエイリアス）
    let fw2 = fwidth(uv2);
    let inside = lil_in01(uv2.x, fw2.x, saturate(nv - 0.05)) * lil_in01(uv2.y, fw2.y, saturate(nv - 0.05));
    if (d.x > 0.5) {
        t.a = t.a * inside;
    }
    c = c * t;
    c.a = c.a * slot_grad(mask_slot, uv_main, gx_main, gy_main).r;
    let fade = lil.p[base + L_FADE];
    c.a = mix(c.a, c.a * saturate((depth - fade.x) / (fade.y - fade.x)), fade.z);
    let cull = i32(q.z + 0.5);
    if ((cull == 1 && facing > 0.0) || (cull == 2 && facing < 0.0)) {
        c.a = 0.0;
    }
    var a = col_a;
    let am = i32(lp.w + 0.5);
    if (mode != 0 && am != 0) {
        if (am == 1) {
            a = c.a;
        } else if (am == 2) {
            a = a * c.a;
        } else if (am == 3) {
            a = saturate(a + c.a);
        } else if (am == 4) {
            a = saturate(a - c.a);
        }
        c.a = 1.0;
    }
    var o: LayerOut;
    o.color = c;
    o.alpha = a;
    return o;
}

// ───────── ラメ（lilVoronoi・lilCalcGlitter・lilHashRGB4） ─────────

// lilHashRGB4 の 1 点（u32 の掛け算は 2^32 で回る。HLSL と同じ）
fn hash_rgb(q: vec2<u32>) -> vec3<f32> {
    let m1 = 1597334677u;
    let m2 = 3812015801u;
    let m3 = 2912667907u;
    let n = (q.x * m1) ^ (q.y * m2);
    return vec3<f32>(vec3<u32>(n * m1, n * m2, n * m3)) * (1.0 / 4294967295.0);
}

// lilVoronoi の、近い点（xyz が乱数、w が距離²）
fn lil_voronoi(pos: vec2<f32>, scale_randomize: f32) -> vec4<f32> {
    let q = vec2<u32>(pos);
    let noise0 = hash_rgb(q);
    let noise1 = hash_rgb(vec2<u32>(q.x + 1u, q.y));
    let noise2 = hash_rgb(vec2<u32>(q.x, q.y + 1u));
    let noise3 = hash_rgb(vec2<u32>(q.x + 1u, q.y + 1u));
    let fp = fract(pos).xyxy + vec4<f32>(0.5, 0.5, -0.5, -0.5);
    var dist4 = vec4<f32>(
        dot(fp.xy - noise0.xy, fp.xy - noise0.xy),
        dot(fp.zy - noise1.xy, fp.zy - noise1.xy),
        dot(fp.xw - noise2.xy, fp.xw - noise2.xy),
        dot(fp.zw - noise3.xy, fp.zw - noise3.xy),
    );
    dist4 = mix(dist4, dist4 / max(vec4<f32>(noise0.z, noise1.z, noise2.z, noise3.z), vec4<f32>(0.001)), scale_randomize);
    var near0 = vec4<f32>(noise1, dist4.y);
    if (dist4.x < dist4.y) {
        near0 = vec4<f32>(noise0, dist4.x);
    }
    var near1 = vec4<f32>(noise3, dist4.w);
    if (dist4.z < dist4.w) {
        near1 = vec4<f32>(noise2, dist4.z);
    }
    return select(near1, near0, near0.w < near1.w);
}

// lilCalcGlitter（形のテクスチャは描かない。時刻 0）
fn lil_glitter(uv: vec2<f32>, n: vec3<f32>, view: vec3<f32>, camera: vec3<f32>, light: vec3<f32>, p1: vec4<f32>, p2: vec4<f32>,
    post_contrast: f32, sensitivity: f32, scale_randomize: f32) -> vec3<f32> {
    var pos = uv * p1.xy;
    let dd = fwidth(pos);
    let factor = fract(sin(dot(floor(pos / floor(dd + 3.0)), vec2<f32>(12.9898, 78.233))) * 46203.4357) + 0.5;
    let factor2 = floor(dd + factor * 0.5);
    pos = pos / max(vec2<f32>(1.0), factor2) + p1.xy * factor2;
    let near = lil_voronoi(pos, scale_randomize);
    let fw_near = fwidth(near.w);
    var g_normal = abs(fract(near.xyz * 14.274) * 2.0 - 1.0);
    g_normal = normalize(g_normal * 2.0 - 1.0);
    var glitter = dot(g_normal, camera);
    glitter = abs(fract(glitter * sensitivity + sensitivity) - 0.5) * 4.0 - 1.0;
    glitter = saturate(1.0 - (glitter * p1.w + p1.w));
    glitter = pow(glitter, post_contrast);
    // 円（アンチエイリアス）
    glitter = glitter * saturate((p1.z - near.w) / fw_near);
    // 角度
    let h = normalize(view + light * p2.z);
    let nh = saturate(dot(n, h));
    glitter = saturate(glitter * saturate(nh * p2.y + 1.0 - p2.y));
    // ランダムな色
    return vec3<f32>(glitter) - glitter * fract(near.xyz * 278.436) * p2.w;
}

// ───────── 面 ─────────

@fragment
fn fs_liltoon(f: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return lil_output(lil_shade(f, front), i32(lil.p[P_MODE].x + 0.5) == 2);
}

// 半透明を sRGB の描き先へ（ハードウェアがリニアで重ねる。Unity と同じ重ね方）: リニアの乗算済みのまま書く。
@fragment
fn fs_liltoon_linear(f: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return lil_shade(f, front);
}

// 面の色（リニア。半透明は乗算済み）。
fn lil_shade(f: VsOut, front: bool) -> vec4<f32> {
    let facing = select(-1.0, 1.0, front);
    let mode = i32(lil.p[P_MODE].x + 0.5);
    let transparent = mode == 2;
    let uv0 = f.uv;
    // メインの UV（lilCalcDoubleSideUV と lilCalcUV。角度は時刻 0 の回転）
    let uvp = lil.p[P_UV];
    var uv_base = uv0;
    if (facing < uvp.y - 1.0) {
        uv_base.x = uv_base.x + 1.0;
    }
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = rotate_uv(uv_base * main_st.xy + main_st.zw, uvp.x);
    // 勾配は条件の外で（導関数の一様性）
    let gx = dpdx(uv_main);
    let gy = dpdy(uv_main);
    let gx0 = dpdx(uv0);
    let gy0 = dpdy(uv0);

    // メインカラー（3D ビューの約束: Color の何も描いていない所は、不透明なら市松の上に見せる）
    var col = slot_grad(SLOT_MAIN, uv_main, gx, gy);
    if (mode == 0 && lil.slot_flags[SLOT_MAIN].y > 0.5 && i32(lil.slot_flags[SLOT_MAIN].z + 0.5) == 0) {
        let p = textureSampleGrad(color_tex, paint_sampler, uv_main, gx, gy);
        let g = checker_gamma(uv0) * (1.0 - p.a) + premultiplied_gamma(p);
        col = vec4<f32>(srgb_to_linear(g), 1.0);
    }
    // 色調補正
    let before = col.rgb;
    let adjust_mask = slot_grad(SLOT_ADJUST_MASK, uv_main, gx, gy).r;
    col = vec4<f32>(mix(before, lil_tone(col.rgb, lil.p[P_HSVG]), adjust_mask), col.a);
    col = col * lil.p[P_COLOR];

    // 法線（ノーマルマップ・ノーマルマップ 2nd を接空間で重ねる）
    let geometric = normalize(f.normal);
    var n = geometric;
    let has_tangents = lil.p[P_FLAGS].x > 0.5;
    var normalmap = vec3<f32>(0.0, 0.0, 1.0);
    var use_normalmap = false;
    let bump = lil.p[P_BUMP];
    if (feat(F_BUMP) && bump.x > 0.5) {
        let bump_st = lil.p[P_BUMP_ST];
        let t = slot_grad(SLOT_BUMP, uv_main * bump_st.xy + bump_st.zw, gx * bump_st.xy, gy * bump_st.xy);
        normalmap = lil_unpack_normal(t, bump.y, slot_received(SLOT_BUMP));
        use_normalmap = true;
    }
    let bump2 = lil.p[P_BUMP2];
    if (feat(F_BUMP2) && bump2.x > 0.5) {
        let st2 = lil.p[P_BUMP2_ST];
        let t2 = slot_grad(SLOT_BUMP2, uv0 * st2.xy + st2.zw, gx0 * st2.xy, gy0 * st2.xy);
        let scale2 = bump2.y * slot_grad(SLOT_BUMP2_MASK, uv_main, gx, gy).r;
        normalmap = lil_blend_normal(normalmap, lil_unpack_normal(t2, scale2, slot_received(SLOT_BUMP2)));
        use_normalmap = true;
    }
    var tangent = f.tangent;
    var bitangent = f.bitangent;
    if (use_normalmap && has_tangents) {
        n = normalize(tangent * normalmap.x + bitangent * normalmap.y + geometric * normalmap.z);
    }
    let flip = lil.p[P_LIGHT2].w;
    if (facing < flip - 1.0) {
        n = -n;
    }
    let orig_n = geometric;
    let view = normalize(u.camera.xyz - f.world);
    let nv = saturate(dot(n, view));
    let nvabs = abs(dot(n, view));
    let depth = distance(u.camera.xyz, f.world);
    // マットキャップの UV の向き（lilToon の fd.uvMat）
    let uv_mat = view_dir_xy(n) * 0.5 + 0.5;
    // 右手の接空間か（デカールのミラー。Unity の接線の w）
    let right_hand = dot(cross(f.normal, f.tangent), f.bitangent) > 0.0;

    // 光
    let light = lil_main_light();
    let to_light = light.dir;
    var light_color = light.color;
    let ind_light = light.ind;
    let inv_lighting = sat3((vec3<f32>(1.0) - light_color) * sqrt(light_color));
    let attenuation = 1.0 - shadow_amount(f.world, geometric, normalize(u.light_dir.xyz), f.clip.xy);
    // 影を受け取る（主な光の影のマップ）
    let calculated = saturate(attenuation + distance(to_light, normalize(u.light_dir.xyz)));

    // メインカラー 2nd・3rd（光の前に重ねる分。光の後の分は下）
    var layer2 = vec4<f32>(0.0);
    var layer3 = vec4<f32>(0.0);
    let l2 = lil.p[P_LAYER2];
    if (feat(F_MAIN2) && l2.x > 0.5) {
        let o = lil_layer(P_LAYER2, SLOT_MAIN2, SLOT_MAIN2_MASK, uv0, uv_mat, uv_main, gx, gy, nv, depth, facing, right_hand, col.a, mode);
        layer2 = o.color;
        col.a = o.alpha;
        col = vec4<f32>(lil_blend(col.rgb, layer2.rgb, vec3<f32>(layer2.a * l2.y), i32(l2.z + 0.5)), col.a);
    }
    let l3 = lil.p[P_LAYER3];
    if (feat(F_MAIN3) && l3.x > 0.5) {
        let o = lil_layer(P_LAYER3, SLOT_MAIN3, SLOT_MAIN3_MASK, uv0, uv_mat, uv_main, gx, gy, nv, depth, facing, right_hand, col.a, mode);
        layer3 = o.color;
        col.a = o.alpha;
        col = vec4<f32>(lil_blend(col.rgb, layer3.rgb, vec3<f32>(layer3.a * l3.y), i32(l3.z + 0.5)), col.a);
    }

    // 異方性反射（接線・光沢・マットキャップの法線）
    var matcap_n = n;
    var matcap2_n = n;
    var reflection_n = n;
    var anisotropy = 0.0;
    var perceptual_roughness = 1.0;
    var aniso_spec = false;
    let an = lil.p[P_ANISO];
    if (feat(F_ANISO) && an.x > 0.5) {
        let ast = lil.p[P_ANISO_ST];
        let tmap = slot_grad(SLOT_ANISO_TANGENT, uv_main * ast.xy + ast.zw, gx * ast.xy, gy * ast.xy);
        let at = lil_unpack_normal(tmap, 1.0, slot_received(SLOT_ANISO_TANGENT));
        tangent = ortho_normalize(normalize(f.tangent * at.x + f.bitangent * at.y + geometric * at.z), n);
        bitangent = cross(n, tangent);
        let mst = lil.p[P_ANISO_MASK_ST];
        anisotropy = an.y * slot_grad(SLOT_ANISO_MASK, uv_main * mst.xy + mst.zw, gx * mst.xy, gy * mst.xy).r;
        // lilGetAnisotropyNormalWS
        var adir = tangent;
        if (anisotropy > 0.0) {
            adir = bitangent;
        }
        adir = ortho_normalize(view, adir);
        let aniso_n = normalize(mix(n, adir, abs(anisotropy)));
        if (an.z > 0.5) {
            reflection_n = aniso_n;
            perceptual_roughness = saturate(1.2 - abs(anisotropy));
            aniso_spec = true;
        }
        if (an.w > 0.5) {
            matcap_n = aniso_n;
        }
        if (lil.p[P_ANISO_Q].x > 0.5) {
            matcap2_n = aniso_n;
        }
    }

    // 影（導関数を使う値は、アルファで捨てる前に取る）
    var shadowmix = 1.0;
    let sp = lil.p[P_SHADOW];
    let lod = lil.p[P_SHADOW_LOD];
    let s1 = lil.p[P_SHADOW1];
    let s2 = lil.p[P_SHADOW2];
    let s3 = lil.p[P_SHADOW3];
    let flat = lil.p[P_SHADOW_FLAT];
    let mask_type = i32(sp.z + 0.5);
    var lns = vec4<f32>(1.0);
    var ssm = vec4<f32>(1.0);
    var shadow1_tex = vec4<f32>(0.0);
    var shadow2_tex_in = vec4<f32>(0.0);
    var shadow3_tex_in = vec4<f32>(0.0);
    if (feat(F_SHADOW)) {
        let ssm_tex = slot_grad(SLOT_SHADOW_STRENGTH, uv_main, max(abs(gx), vec2<f32>(lod.x)), max(abs(gy), vec2<f32>(lod.x)));
        let blur_mask = slot_grad(SLOT_SHADOW_BLUR, uv_main, max(abs(gx), vec2<f32>(lod.z)), max(abs(gy), vec2<f32>(lod.z)));
        let border_tex = slot_grad(SLOT_SHADOW_BORDER, uv_main, max(abs(gx), vec2<f32>(lod.y)), max(abs(gy), vec2<f32>(lod.y)));
        shadow1_tex = slot_grad(SLOT_SHADOW1_TEX, uv_main, gx, gy);
        shadow2_tex_in = slot_grad(SLOT_SHADOW2_TEX, uv_main, gx, gy);
        shadow3_tex_in = slot_grad(SLOT_SHADOW3_TEX, uv_main, gx, gy);
        let n1 = mix(orig_n, n, s1.z);
        let n2 = mix(orig_n, n, s2.z);
        let n3 = mix(orig_n, n, s3.z);
        ssm = ssm_tex;
        lns.x = saturate(dot(to_light, n1) * 0.5 + 0.5);
        lns.y = saturate(dot(to_light, n2) * 0.5 + 0.5);
        lns.z = saturate(dot(to_light, n3) * 0.5 + 0.5);
        var aa = lil.p[P_LIGHT2].y;
        if (mask_type == 2) {
            // SDF（顔の影）。物の向きは世界の向き（モデルは世界の空間）: 右が −X、前が +Z
            let face_r = vec3<f32>(-1.0, 0.0, 0.0);
            let l_dot_r = dot(to_light.xz, face_r.xz);
            let sdf = select(ssm.r, ssm.g, l_dot_r < 0.0);
            var face_f = vec3<f32>(0.0, 0.0, 1.0);
            face_f.y = face_f.y * flat.y;
            face_f = select(normalize(face_f), vec3<f32>(0.0), dot(face_f, face_f) == 0.0);
            var face_l = to_light;
            face_l.y = face_l.y * flat.y;
            face_l = select(normalize(face_l), vec3<f32>(0.0), dot(face_l, face_l) == 0.0);
            let ln_sdf = dot(face_l, face_f);
            lns = mix(vec4<f32>(saturate(ln_sdf * 0.5 + sdf * 0.5 + 0.25)), lns, ssm.b);
            aa = 0.0;
            ssm.r = ssm.a;
        }
        lns.x = lns.x * mix(1.0, calculated, s1.w);
        lns.y = lns.y * mix(1.0, calculated, s2.w);
        lns.z = lns.z * mix(1.0, calculated, s3.w);
        // ぼかしマスク
        let blur1 = s1.y * blur_mask.r;
        let blur2 = s2.y * blur_mask.g;
        let blur3 = s3.y * blur_mask.b;
        // AO
        var bm = border_tex;
        let ao = lil.p[P_AO_SHIFT];
        let ao2 = lil.p[P_AO_SHIFT2];
        bm.r = saturate(bm.r * ao.x + ao.y);
        bm.g = saturate(bm.g * ao.z + ao.w);
        bm.b = saturate(bm.b * ao2.x + ao2.y);
        let post_ao = sp.w > 0.5;
        if (!post_ao) {
            lns = vec4<f32>(lns.xyz * bm.rgb, lns.w);
        }
        let fw_x = fwidth(lns.x);
        let fw_y = fwidth(lns.y);
        let fw_z = fwidth(lns.z);
        lns.w = lns.x;
        lns.x = lil_tooning_ns(aa, lns.x, s1.x, blur1, fw_x);
        lns.y = lil_tooning_ns(aa, lns.y, s2.x, blur2, fw_y);
        lns.w = lil_tooning_ns_range(aa, lns.w, s1.x, blur1, flat.z, fw_x);
        lns.z = lil_tooning_ns(aa, lns.z, s3.x, blur3, fw_z);
        if (post_ao) {
            lns = lns * bm.rgbr;
        }
        lns = clamp(lns, vec4<f32>(0.0), vec4<f32>(1.0));
    }

    let aa_main = lil.p[P_LIGHT2].y;
    // リムシェード（導関数は捨てる前に）
    var rim_shade = 0.0;
    let rs = lil.p[P_RIM_SHADE];
    if (feat(F_RIM_SHADE) && rs.x > 0.5) {
        let rs_n = mix(orig_n, n, rs.y);
        let rs_nv = abs(dot(rs_n, view));
        let r0 = pow(saturate(1.0 - rs_nv), lil.p[P_RIM_SHADE2].x);
        rim_shade = saturate(lil_tooning_ns(aa_main, r0, rs.z, rs.w, fwidth(r0)));
        rim_shade = rim_shade * lil.p[P_RIM_SHADE_COLOR].a * slot_grad(SLOT_RIM_SHADE_MASK, uv_main, gx, gy).r;
    }
    // 逆光ライト（導関数は捨てる前に）
    var backlight = 0.0;
    var backlight_color = vec4<f32>(0.0);
    let bl = lil.p[P_BACKLIGHT];
    if (feat(F_BACKLIGHT) && bl.x > 0.5) {
        let blp = lil.p[P_BACKLIGHT_P];
        let bl_n = mix(orig_n, n, blp.x);
        let bst = lil.p[P_BACKLIGHT_ST];
        backlight_color = lil.p[P_BACKLIGHT_COLOR] * slot_grad(SLOT_BACKLIGHT, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
        let hl = dot(view, to_light);
        let factor = pow(saturate(-hl * 0.5 + 0.5), blp.w);
        var bl_ln = dot(normalize(-view * lil.p[P_BACKLIGHT_Q].x + to_light), bl_n) * 0.5 + 0.5;
        if (bl.z > 0.5) {
            bl_ln = bl_ln * calculated;
        }
        bl_ln = saturate(lil_tooning_ns(aa_main, bl_ln, blp.y, blp.z, fwidth(bl_ln)));
        backlight = saturate(factor * bl_ln);
        if (facing < bl.w - 1.0) {
            backlight = 0.0;
        }
    }
    // 光沢（滑らかさ・金属度・光沢の項。導関数は捨てる前に）
    var smoothness = 1.0;
    var roughness = 1.0;
    var metallic = 0.0;
    var spec_term = 0.0;
    var spec_fresnel_lh = 1.0;
    let rf = lil.p[P_REFL];
    let rfp = lil.p[P_REFL_P];
    let rfq = lil.p[P_REFL_Q];
    if (feat(F_REFLECTION) && rf.x > 0.5) {
        let sst = lil.p[P_SMOOTH_ST];
        smoothness = rf.y * slot_grad(SLOT_SMOOTHNESS, uv_main * sst.xy + sst.zw, gx * sst.xy, gy * sst.xy).r;
        // GSAA
        let dnx = abs(dpdx(n));
        let dny = abs(dpdy(n));
        let dxy = max(dot(dnx, dnx), dot(dny, dny));
        let gsaa = dxy / (dxy * 5.0 + 0.002) * lil.p[P_REFL_R].y;
        smoothness = min(smoothness, saturate(1.0 - gsaa));
        perceptual_roughness = perceptual_roughness - smoothness * perceptual_roughness;
        roughness = perceptual_roughness * perceptual_roughness;
        let mst = lil.p[P_METAL_ST];
        metallic = rf.z * slot_grad(SLOT_METALLIC, uv_main * mst.xy + mst.zw, gx * mst.xy, gy * mst.xy).r;
        // lilCalcSpecular（主な光。トゥーンと GGX、異方性反射）
        let sn = mix(orig_n, n, rfp.z);
        let h = normalize(view + to_light);
        let nh = saturate(dot(sn, h));
        if (rfp.y > 0.5 && !aniso_spec) {
            let v = pow(nh, 1.0 / roughness);
            spec_term = saturate(lil_tooning_ns(aa_main, v, rfp.w, rfq.x, fwidth(v)));
            spec_fresnel_lh = -1.0;
        } else {
            let snv = saturate(dot(sn, view));
            let snl = saturate(dot(sn, to_light));
            let lh = saturate(dot(to_light, h));
            var ggx = 0.0;
            var lambda_v = 0.0;
            var lambda_l = 0.0;
            if (aniso_spec) {
                let rt = max(roughness * (1.0 + anisotropy), 0.002);
                let rb = max(roughness * (1.0 - anisotropy), 0.002);
                let tv = dot(tangent, view);
                let bv = dot(bitangent, view);
                let tl = dot(tangent, to_light);
                let bl2 = dot(bitangent, to_light);
                lambda_v = snl * length(vec3<f32>(rt * tv, rb * bv, snv));
                lambda_l = snv * length(vec3<f32>(rt * tl, rb * bl2, snl));
                let a1 = lil.p[P_ANISO1];
                let a2 = lil.p[P_ANISO2];
                let rt1 = rt * a1.x;
                let rb1 = rb * a1.y;
                let rt2 = rt * a2.x;
                let rb2 = rb * a2.y;
                let nst = lil.p[P_ANISO_NOISE_ST];
                let noise = slot_grad(SLOT_ANISO_NOISE, uv_main * nst.xy + nst.zw, gx * nst.xy, gy * nst.xy).r - 0.5;
                let shift1 = noise * a1.w + a1.z;
                let shift2 = noise * a2.w + a2.z;
                let t1 = normalize(tangent - sn * shift1);
                let b1 = normalize(bitangent - sn * shift1);
                let t2 = normalize(tangent - sn * shift2);
                let b2 = normalize(bitangent - sn * shift2);
                let r1 = rt1 * rb1;
                let r2 = rt2 * rb2;
                let v1 = vec3<f32>(dot(t1, h) * rb1, dot(b1, h) * rt1, nh * r1);
                let v2 = vec3<f32>(dot(t2, h) * rb2, dot(b2, h) * rt2, nh * r2);
                let w1 = r1 / dot(v1, v1);
                let w2 = r2 / dot(v2, v2);
                let s = lil.p[P_ANISO_S];
                ggx = r1 * w1 * w1 * s.x + r2 * w2 * w2 * s.y;
            } else {
                let r2v = max(roughness, 0.002);
                lambda_v = snl * (snv * (1.0 - r2v) + r2v);
                lambda_l = snv * (snl * (1.0 - r2v) + r2v);
                let rr2 = r2v * r2v;
                let d = (nh * rr2 - nh) * nh + 1.0;
                ggx = rr2 / (d * d + 1e-7);
            }
            let sjggx = 0.5 / (lambda_v + lambda_l + 1e-5);
            spec_term = sjggx * ggx * snl;
            if (aniso_spec && rfp.y > 0.5) {
                spec_term = lil_tooning_step(aa_main, spec_term, 0.5, fwidth(spec_term));
                spec_fresnel_lh = -1.0;
            } else {
                spec_fresnel_lh = lh;
            }
        }
    }
    // ラメ（導関数は捨てる前に）
    var glitter = vec3<f32>(0.0);
    let gl = lil.p[P_GLITTER];
    let glq = lil.p[P_GLITTER_Q];
    if (feat(F_GLITTER) && gl.x > 0.5) {
        let gl_n = mix(orig_n, n, glq.w);
        let glr = lil.p[P_GLITTER_R];
        glitter = lil_glitter(uv0, gl_n, view, u.lil_camera_front.xyz, to_light, lil.p[P_GLITTER_P1], lil.p[P_GLITTER_P2], glr.x, glr.y, glr.z);
    }
    // リムライト（ライト方向あり。導関数は捨てる前に）
    let rim = lil.p[P_RIM];
    let rp = lil.p[P_RIM_P];
    let rq = lil.p[P_RIM_Q];
    let rr = lil.p[P_RIM_R];
    let rim_n = mix(orig_n, n, rim.w);
    let rim_nv = abs(dot(rim_n, view));
    let ln_raw = dot(to_light, rim_n) * 0.5 + 0.5;
    let ln_dir = saturate((ln_raw + rr.x) / (1.0 + rr.x));
    let ln_indir = saturate((1.0 - ln_raw + rr.y) / (1.0 + rr.y));
    var rim_f = pow(saturate(1.0 - rim_nv), rp.z);
    if (facing < rq.y - 1.0) {
        rim_f = 0.0;
    }
    let rim_dir_in = mix(rim_f, rim_f * ln_dir, rq.w);
    let rim_ind_in = rim_f * ln_indir * rq.w;
    let fw_rd = fwidth(rim_dir_in);
    let fw_ri = fwidth(rim_ind_in);
    let uv_rim = vec2<f32>(nvabs, nvabs);
    let gx_rim = dpdx(uv_rim);
    let gy_rim = dpdy(uv_rim);
    // アルファマスク（不透明の描画モードでは使わない）
    if (feat(F_ALPHA_MASK)) {
        let ast = lil.p[P_ALPHA_MASK_ST];
        let am_tex = slot_grad(SLOT_ALPHA_MASK, uv_main * ast.xy + ast.zw, gx * ast.xy, gy * ast.xy).r;
        col.a = apply_alpha_mask(col.a, am_tex, mode);
    }
    let fw_alpha = fwidth(col.a);
    let cutoff = lil.p[P_MODE].y;
    if (mode == 0) {
        col.a = 1.0;
    } else if (mode == 1) {
        col.a = saturate((col.a - cutoff) / max(fw_alpha, 0.0001) + 0.5);
        if (col.a == 0.0) {
            discard;
        }
    } else if (col.a - cutoff < 0.0) {
        discard;
    }
    let albedo = col.rgb;

    // 裏面を影に
    let bfshadow = select(1.0, 1.0 - lil.p[P_LIGHT2].z, facing < 0.0);
    lns.x = lns.x * bfshadow;
    lns.y = lns.y * bfshadow;
    lns.w = lns.w * bfshadow;
    lns.z = lns.z * bfshadow;
    if (feat(F_SHADOW) && sp.x > 0.5) {
        shadowmix = lns.x;
        var strength = sp.y;
        if (mask_type == 1) {
            // 平面（物の前 (0, 0.25, 1) の向き）
            let flat_n = normalize(vec3<f32>(0.0, 0.25, 1.0));
            var ln_flat = saturate((dot(flat_n, to_light) + flat.x) / flat.y);
            ln_flat = ln_flat * mix(1.0, calculated, s1.w);
            lns = mix(vec4<f32>(ln_flat), lns, ssm.r);
        } else {
            strength = strength * ssm.r;
        }
        lns.x = mix(1.0, lns.x, strength);
        // 影色（LUT は描かない: 通常の影色テクスチャとして読む）
        var indirect = mix(albedo, shadow1_tex.rgb, shadow1_tex.a) * lil.p[P_SHADOW1_COLOR].rgb;
        let c2 = lil.p[P_SHADOW2_COLOR];
        let shadow2 = mix(albedo, shadow2_tex_in.rgb, shadow2_tex_in.a) * c2.rgb;
        lns.y = c2.a - lns.y * c2.a;
        indirect = mix(indirect, shadow2, lns.y);
        let c3 = lil.p[P_SHADOW3_COLOR];
        let shadow3 = mix(albedo, shadow3_tex_in.rgb, shadow3_tex_in.a) * c3.rgb;
        lns.z = c3.a - lns.z * c3.a;
        indirect = mix(indirect, shadow3, lns.z);
        indirect = mix(indirect, indirect * albedo, flat.w);
        let direct = albedo * light_color;
        indirect = indirect * light_color;
        indirect = mix(indirect, albedo, sat3(ind_light * lil.p[P_LIGHT2].x));
        indirect = min(indirect, direct);
        indirect = mix(indirect, direct, lns.w * lil.p[P_SHADOW_BORDER_COLOR].rgb);
        col = vec4<f32>(mix(indirect, direct, lns.x), col.a);
    } else {
        col = vec4<f32>(col.rgb * light_color, col.a);
    }
    let max_limit = lil.p[P_LIGHT].y;
    light_color = min(light_color, vec3<f32>(max_limit));
    shadowmix = saturate(shadowmix);
    col = vec4<f32>(min(col.rgb, albedo * max_limit), col.a);

    // メインカラー 2nd・3rd の光の後の分（ライトの明るさを反映しない分）
    if (feat(F_MAIN2) && l2.x > 0.5) {
        col = vec4<f32>(lil_blend(col.rgb, layer2.rgb, vec3<f32>(layer2.a - layer2.a * l2.y), i32(l2.z + 0.5)), col.a);
    }
    if (feat(F_MAIN3) && l3.x > 0.5) {
        col = vec4<f32>(lil_blend(col.rgb, layer3.rgb, vec3<f32>(layer3.a - layer3.a * l3.y), i32(l3.z + 0.5)), col.a);
    }

    // リムシェード
    if (feat(F_RIM_SHADE) && rs.x > 0.5) {
        col = vec4<f32>(mix(col.rgb, col.rgb * lil.p[P_RIM_SHADE_COLOR].rgb, rim_shade), col.a);
    }
    // 逆光ライト
    if (feat(F_BACKLIGHT) && bl.x > 0.5) {
        let bc = vec4<f32>(mix(backlight_color.rgb, backlight_color.rgb * albedo, bl.y), backlight_color.a);
        col = vec4<f32>(col.rgb + backlight * bc.a * bc.rgb * light_color, col.a);
    }

    // 乗算済み（半透明）
    if (transparent) {
        col = vec4<f32>(col.rgb * col.a, col.a);
    }

    // 光沢（光沢の項と環境光の反射）
    if (feat(F_REFLECTION) && rf.x > 0.5) {
        let rfr = lil.p[P_REFL_R];
        let blend_mode = i32(rfr.x + 0.5);
        col = vec4<f32>(col.rgb - metallic * col.rgb, col.a);
        let specular = mix(vec3<f32>(rf.w), albedo, metallic);
        let cst = lil.p[P_REFL_COLOR_ST];
        var rcolor = lil.p[P_REFL_COLOR] * slot_grad(SLOT_REFL_COLOR, uv_main * cst.xy + cst.zw, gx * cst.xy, gy * cst.xy);
        if (transparent && rfq.w > 0.5) {
            rcolor.a = rcolor.a * col.a;
        }
        if (rfp.x > 0.5) {
            var refl = vec3<f32>(spec_term);
            if (spec_fresnel_lh >= 0.0) {
                // lilFresnelTerm
                let a = 1.0 - spec_fresnel_lh;
                refl = spec_term * (specular + (vec3<f32>(1.0) - specular) * (a * a * a * a * a));
            }
            col = vec4<f32>(lil_blend(col.rgb, rcolor.rgb * light_color, refl * rcolor.a, blend_mode), col.a);
        }
        if (rfq.y > 0.5) {
            let rn = mix(orig_n, reflection_n, rfq.z);
            // 環境光の反射（3D ビューの環境。Unity の反射プローブと同じ mip の選び方）。環境なしは一様な環境光
            var env = u.ambient.rgb;
            if (u.env.x > 0.5) {
                let mip = perceptual_roughness * (1.7 - 0.7 * perceptual_roughness) * 6.0;
                env = textureSampleLevel(env_cube, env_sampler, to_source(reflect(-view, rn)), mip).rgb * u.env.y;
            }
            let one_minus_reflectivity = (1.0 - DIELECTRIC) - metallic * (1.0 - DIELECTRIC);
            let grazing = saturate(smoothness + (1.0 - one_minus_reflectivity));
            let surface_reduction = 1.0 / (roughness * roughness + 1.0);
            let a = 1.0 - nv;
            let fresnel = mix(specular, vec3<f32>(grazing), a * a * a * a * a);
            let refl = surface_reduction * env * fresnel;
            col = vec4<f32>(lil_blend(col.rgb, rcolor.rgb, refl * rcolor.a, blend_mode), col.a);
        }
    }

    // マットキャップ
    let mc = lil.p[P_MATCAP];
    let mcb = lil.p[P_MC_BUMP];
    if (feat(F_MATCAP) && mc.x > 0.5) {
        let mcp = lil.p[P_MATCAP_P];
        let mcq = lil.p[P_MATCAP_Q];
        var mc_n = mix(orig_n, matcap_n, mcq.x);
        if (mcb.x > 0.5 && has_tangents) {
            let bst = lil.p[P_MC_BUMP_ST];
            let t = slot_grad(SLOT_MATCAP_BUMP, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
            let nm = lil_unpack_normal(t, mcb.y, slot_received(SLOT_MATCAP_BUMP));
            mc_n = normalize(f.tangent * nm.x + f.bitangent * nm.y + geometric * nm.z);
            if (facing < flip - 1.0) {
                mc_n = -mc_n;
            }
        }
        let mc_uv = matcap_uv(normalize(mc_n), view, lil.p[P_MATCAP_ST], mcq.y > 0.5, mcq.z > 0.5);
        let mc_tex = slot_level(SLOT_MATCAP, mc_uv, mcp.w);
        let mc_mask_st = lil.p[P_MATCAP_MASK_ST];
        let mc_mask = slot_grad(SLOT_MATCAP_MASK, uv_main * mc_mask_st.xy + mc_mask_st.zw, gx * mc_mask_st.xy, gy * mc_mask_st.xy).rgb;
        var c = lil.p[P_MATCAP_COLOR] * mc_tex;
        c = vec4<f32>(mix(c.rgb, c.rgb * light_color, mcp.x), mix(c.a, c.a * shadowmix, mcp.y));
        if (transparent && mcq.w > 0.5) {
            c.a = c.a * col.a;
        }
        if (facing < mcp.z - 1.0) {
            c.a = 0.0;
        }
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, mc.w), c.a);
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, mc.y * c.a * mc_mask, i32(mc.z + 0.5)), col.a);
    }
    let mc2 = lil.p[P_MATCAP2];
    if (feat(F_MATCAP2) && mc2.x > 0.5) {
        let mc2p = lil.p[P_MATCAP2_P];
        let mc2q = lil.p[P_MATCAP2_Q];
        // 2nd は lilToon と同じく、混ぜた法線を正規化しないで UV を出す
        var mc2_n = mix(orig_n, matcap2_n, mc2q.x);
        if (mcb.z > 0.5 && has_tangents) {
            let bst = lil.p[P_MC2_BUMP_ST];
            let t = slot_grad(SLOT_MATCAP2_BUMP, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
            let nm = lil_unpack_normal(t, mcb.w, slot_received(SLOT_MATCAP2_BUMP));
            mc2_n = normalize(f.tangent * nm.x + f.bitangent * nm.y + geometric * nm.z);
            if (facing < flip - 1.0) {
                mc2_n = -mc2_n;
            }
        }
        let mc2_uv = matcap_uv(mc2_n, view, lil.p[P_MATCAP2_ST], mc2q.y > 0.5, mc2q.z > 0.5);
        let mc2_tex = slot_level(SLOT_MATCAP2, mc2_uv, mc2p.w);
        let mc2_mask_st = lil.p[P_MATCAP2_MASK_ST];
        let mc2_mask = slot_grad(SLOT_MATCAP2_MASK, uv_main * mc2_mask_st.xy + mc2_mask_st.zw, gx * mc2_mask_st.xy, gy * mc2_mask_st.xy).rgb;
        var c = lil.p[P_MATCAP2_COLOR] * mc2_tex;
        c = vec4<f32>(mix(c.rgb, c.rgb * light_color, mc2p.x), mix(c.a, c.a * shadowmix, mc2p.y));
        if (transparent && mc2q.w > 0.5) {
            c.a = c.a * col.a;
        }
        if (facing < mc2p.z - 1.0) {
            c.a = 0.0;
        }
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, mc2.w), c.a);
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, mc2.y * c.a * mc2_mask, i32(mc2.z + 0.5)), col.a);
    }

    // リムライト（ライト方向あり。値は上で取った）
    if (feat(F_RIM) && rim.x > 0.5) {
        let rim_st = lil.p[P_RIM_ST];
        let rim_tex = slot_grad(SLOT_RIM, uv_main * rim_st.xy + rim_st.zw, gx * rim_st.xy, gy * rim_st.xy);
        var rim_color = lil.p[P_RIM_COLOR] * rim_tex;
        let rim_indir_color = lil.p[P_RIM_INDIR_COLOR] * rim_tex;
        rim_color = vec4<f32>(mix(rim_color.rgb, rim_color.rgb * albedo, rim.z), rim_color.a);
        var rim_dir = saturate(lil_tooning_ns(aa_main, rim_dir_in, rp.x, rp.y, fw_rd));
        var rim_ind = saturate(lil_tooning_ns(aa_main, rim_ind_in, rr.z, rr.w, fw_ri));
        rim_dir = mix(rim_dir, rim_dir * shadowmix, rq.x);
        rim_ind = mix(rim_ind, rim_ind * shadowmix, rq.x);
        if (transparent && rq.z > 0.5) {
            rim_dir = rim_dir * col.a;
            rim_ind = rim_ind * col.a;
        }
        let rim_mul = vec3<f32>(1.0 - rp.w) + light_color * rp.w;
        let blend_mode = i32(rim.y + 0.5);
        col = vec4<f32>(lil_blend(col.rgb, rim_color.rgb * rim_mul, vec3<f32>(rim_dir * rim_color.a), blend_mode), col.a);
        col = vec4<f32>(lil_blend(col.rgb, rim_indir_color.rgb * rim_mul, vec3<f32>(rim_ind * rim_indir_color.a), blend_mode), col.a);
    }

    // ラメ
    if (feat(F_GLITTER) && gl.x > 0.5) {
        let gst = lil.p[P_GLITTER_ST];
        var gc = lil.p[P_GLITTER_COLOR] * slot_grad(SLOT_GLITTER, uv_main * gst.xy + gst.zw, gx * gst.xy, gy * gst.xy);
        gc = vec4<f32>(gc.rgb * glitter, gc.a);
        gc = vec4<f32>(mix(gc.rgb, gc.rgb * albedo, gl.z), gc.a);
        if (transparent && glq.z > 0.5) {
            gc.a = gc.a * col.a;
        }
        if (facing < glq.y - 1.0) {
            gc.a = 0.0;
        }
        gc.a = mix(gc.a, gc.a * shadowmix, glq.x);
        gc = vec4<f32>(mix(gc.rgb, gc.rgb * light_color, gl.w), gc.a);
        col = vec4<f32>(col.rgb + gc.rgb * gc.a, col.a);
    }

    // 発光
    let em = lil.p[P_EMISSION];
    if (feat(F_EMISSION) && em.x > 0.5) {
        let emx = lil.p[P_EMISSION_X];
        let em_st = lil.p[P_EMISSION_ST];
        let em_rim = i32(emx.y + 0.5) == 4;
        let em_uv = rotate_uv(select(uv0, uv_rim, em_rim) * em_st.xy + em_st.zw, emx.z);
        let em_tex = slot_grad(SLOT_EMISSION, em_uv, select(gx0, gx_rim, em_rim) * em_st.xy, select(gy0, gy_rim, em_rim) * em_st.xy);
        let em_mask_st = lil.p[P_EMISSION_MASK_ST];
        let em_mask = slot_grad(SLOT_EMISSION_MASK, rotate_uv(uv0 * em_mask_st.xy + em_mask_st.zw, emx.w), gx0 * em_mask_st.xy, gy0 * em_mask_st.xy);
        var c = lil.p[P_EMISSION_COLOR] * em_tex * em_mask;
        c = vec4<f32>(mix(c.rgb, c.rgb * inv_lighting, emx.x), c.a);
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, em.w), c.a);
        var b = em.y * c.a;
        if (transparent) {
            b = b * col.a;
        }
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, vec3<f32>(b), i32(em.z + 0.5)), col.a);
    }
    let em2 = lil.p[P_EMISSION2];
    if (feat(F_EMISSION2) && em2.x > 0.5) {
        let em2x = lil.p[P_EMISSION2_X];
        let em2_st = lil.p[P_EMISSION2_ST];
        let em2_rim = i32(em2x.y + 0.5) == 4;
        let em2_uv = rotate_uv(select(uv0, uv_rim, em2_rim) * em2_st.xy + em2_st.zw, em2x.z);
        let em2_tex = slot_grad(SLOT_EMISSION2, em2_uv, select(gx0, gx_rim, em2_rim) * em2_st.xy, select(gy0, gy_rim, em2_rim) * em2_st.xy);
        let em2_mask_st = lil.p[P_EMISSION2_MASK_ST];
        let em2_mask = slot_grad(SLOT_EMISSION2_MASK, rotate_uv(uv0 * em2_mask_st.xy + em2_mask_st.zw, em2x.w), gx0 * em2_mask_st.xy, gy0 * em2_mask_st.xy);
        var c = lil.p[P_EMISSION2_COLOR] * em2_tex * em2_mask;
        c = vec4<f32>(mix(c.rgb, c.rgb * inv_lighting, em2x.x), c.a);
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, em2.w), c.a);
        var b = em2.y * c.a;
        if (transparent) {
            b = b * col.a;
        }
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, vec3<f32>(b), i32(em2.z + 0.5)), col.a);
    }

    // 裏面の色
    if (feat(F_BACKFACE)) {
        let back = lil.p[P_BACKFACE];
        if (facing < 0.0) {
            col = vec4<f32>(mix(col.rgb, back.rgb * light_color, back.a), col.a);
        }
    }

    // 距離フェード
    if (feat(F_DISTANCE_FADE)) {
        col = distance_fade(col, depth, orig_n, view, facing, transparent, false);
    }
    if (!transparent) {
        col.a = 1.0;
    }
    return col;
}

// lilDistanceFade（`outline` は輪郭線: 裏面の扱いが無い）。
fn distance_fade(col_in: vec4<f32>, depth: f32, orig_n: vec3<f32>, view: vec3<f32>, facing: f32, transparent: bool, outline: bool) -> vec4<f32> {
    var col = col_in;
    let fade = lil.p[P_DFADE];
    let fp = lil.p[P_DFADE_P];
    var d = depth;
    if (fp.x > 0.5) {
        d = distance(u.camera.xyz, lil.p[P_DFADE_ORIGIN].xyz);
    }
    var dist_fade = saturate((d - fade.x) / (fade.y - fade.x));
    if (outline) {
        dist_fade = dist_fade * fade.z;
    } else if (facing < fade.w - 1.0) {
        dist_fade = fade.z;
    } else {
        dist_fade = dist_fade * fade.z;
    }
    let fc = lil.p[P_DFADE_COLOR];
    var fade_color = fc.rgb;
    let rim_color = lil.p[P_DFADE_RIM_COLOR];
    let nvabs = abs(dot(orig_n, view));
    let fade_rim = pow(saturate(1.0 - nvabs), fp.y);
    fade_color = mix(fade_color, rim_color.rgb * col.rgb, fade_rim * rim_color.a);
    if (transparent) {
        col = vec4<f32>(mix(col.rgb, fade_color * fc.a, dist_fade), mix(col.a, col.a * fc.a, dist_fade));
    } else {
        col = vec4<f32>(mix(col.rgb, fade_color, dist_fade), col.a);
    }
    return col;
}

// ───────── 輪郭線（裏返しの殻） ─────────

@vertex
fn vs_outline(v: VsIn) -> VsOut {
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = rotate_uv(v.uv * main_st.xy + main_st.zw, lil.p[P_UV].x);
    let o = lil.p[P_OUTLINE];
    var width = o.y * 0.01;
    width = width * slot_level(SLOT_OUTLINE_WIDTH, uv_main, 0.0).r;
    width = width * mix(1.0, saturate(length(u.camera.xyz - v.position)), o.z);
    var p = v.position + v.normal * width;
    let to_camera = u.camera.xyz - p;
    p = p - normalize(to_camera) * lil.p[P_OUTLINE_Q].x;
    var out: VsOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    if (lil.p[P_OUTLINE_Q].y > 0.5 && abs(width) < 0.000001) {
        // 太さ 0 の頂点を消す（lilToon は位置を NaN にする。ここでは切り取りの外へ）
        out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    out.world = p;
    out.normal = v.normal;
    out.tangent = v.tangent.xyz;
    out.bitangent = cross(v.normal, v.tangent.xyz) * v.tangent.w;
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_outline(f: VsOut) -> @location(0) vec4<f32> {
    return lil_output(lil_outline_shade(f), i32(lil.p[P_MODE].x + 0.5) == 2);
}

@fragment
fn fs_outline_linear(f: VsOut) -> @location(0) vec4<f32> {
    return lil_outline_shade(f);
}

fn lil_outline_shade(f: VsOut) -> vec4<f32> {
    let mode = i32(lil.p[P_MODE].x + 0.5);
    let transparent = mode == 2;
    let st = lil.p[P_OUTLINE_ST];
    let uv = rotate_uv(f.uv * st.xy + st.zw, lil.p[P_OUTLINE_Q].z);
    let gx = dpdx(uv);
    let gy = dpdy(uv);
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = rotate_uv(f.uv * main_st.xy + main_st.zw, lil.p[P_UV].x);
    let gxm = dpdx(uv_main);
    let gym = dpdy(uv_main);
    var col = slot_grad(SLOT_OUTLINE, uv, gx, gy);
    col = vec4<f32>(lil_tone(col.rgb, lil.p[P_OUTLINE_HSVG]), col.a);
    let n = normalize(f.normal);
    // 輪郭線のパスは光の向きを受け取らない（lilToon の既定 (0, 1, 0)）
    let l = vec3<f32>(0.0, 1.0, 0.0);
    let nv = normalize(view_dir_xy(n));
    let lv = normalize(view_dir_xy(l));
    let ndotl = dot(nv, lv) * 0.5 + 0.5;
    let op = lil.p[P_OUTLINE_P];
    let lit_color = lil.p[P_OUTLINE_LIT_COLOR];
    var lit_rgb = lit_color.rgb;
    if (op.z > 0.5) {
        lit_rgb = col.rgb * lit_color.rgb;
    }
    var lit_factor = saturate(ndotl * op.x + op.y) * lit_color.a;
    if (op.w > 0.5) {
        lit_factor = lit_factor * (1.0 - shadow_amount(f.world, n, normalize(u.light_dir.xyz), f.clip.xy));
    }
    let oc = lil.p[P_OUTLINE_COLOR];
    col = vec4<f32>(mix(col.rgb * oc.rgb, lit_rgb, lit_factor), col.a * oc.a);
    // アルファ
    if (feat(F_ALPHA_MASK)) {
        let ast = lil.p[P_ALPHA_MASK_ST];
        let am_tex = slot_grad(SLOT_ALPHA_MASK, uv_main * ast.xy + ast.zw, gxm * ast.xy, gym * ast.xy).r;
        col.a = apply_alpha_mask(col.a, am_tex, mode);
    }
    let fw_alpha = fwidth(col.a);
    let cutoff = lil.p[P_MODE].y;
    if (mode == 0) {
        col.a = 1.0;
    } else if (mode == 1) {
        col.a = saturate((col.a - cutoff) / max(fw_alpha, 0.0001) + 0.5);
        if (col.a == 0.0) {
            discard;
        }
    } else if (col.a - cutoff < 0.0) {
        discard;
    }
    let light = lil_main_light();
    let o = lil.p[P_OUTLINE];
    col = vec4<f32>(mix(col.rgb, col.rgb * min(light.color, vec3<f32>(lil.p[P_LIGHT].y)), o.w), col.a);
    if (transparent) {
        col = vec4<f32>(col.rgb * col.a, col.a);
    } else {
        col.a = 1.0;
    }
    if (feat(F_DISTANCE_FADE)) {
        let view = normalize(u.camera.xyz - f.world);
        col = distance_fade(col, distance(u.camera.xyz, f.world), n, view, 1.0, transparent, true);
    }
    return col;
}

// 世界の向きを視点の空間の XY へ（lilTransformDirWStoVSCenter の xy）。
fn view_dir_xy(d: vec3<f32>) -> vec2<f32> {
    // Unity の視点の行列の 0 行目（右）。front は手前への向き（−前）なので、右 = 上 × 前 = front × 上
    let right = cross(u.lil_camera_front.xyz, u.lil_camera_up.xyz);
    return vec2<f32>(dot(d, right), dot(d, u.lil_camera_up.xyz));
}
