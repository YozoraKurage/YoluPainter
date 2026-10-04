// lilToon の再現（基本の範囲）。scene.wgsl の後ろにつないで 1 つのモジュールにする（一様バッファ・束ね・色の変換・影は scene.wgsl のもの）。
//
// 式は lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の Shader/Includes から移した: lil_pass_forward_normal.hlsl の frag の順、
// lil_common_frag.hlsl の lilGetShading・lilGetMatCap・lilGetMatCap2nd・lilGetRim・lilEmission・lilEmission2nd・アルファマスク・輪郭線の色、
// lil_common_functions.hlsl の lilTooning*・lilToneCorrection・lilBlendColor・lilCalcMatCapUV・lilUnpackNormalScale・lilGetOutlineWidth・
// lilCalcOutlinePosition、ビルトインのレンダーパイプラインの光（openlit_core.hlsl の ComputeLights。OpenLit は CC0）と
// lil_common_macro.hlsl の LIL_CORRECT_LIGHTCOLOR。機能の入切は、エディタの既定（全部の機能を組み込んだ版）と同じ。
//
// 再現しないもの（値は持つが描かない）: メインカラー 2nd・3rd、デカール、グラデーションマップ、ディゾルブ、ディザー、ノーマルマップ 2nd、
// 異方性、逆光、リムシェード、光沢（反射）、ラメ、視差、距離フェード、AudioLink、ID マスク、ファー・宝石・屈折、時間で動く値
// （スクロール・点滅・グラデーション）、影色の LUT、追加のライト（頂点ライト・ForwardAdd）、ライトマップ、霧。
//
// 色の約束: lilToon はリニアで解き、最後に scene.wgsl と同じくガンマにして書く。半透明は、描き先を sRGB の見え方にして（`*_linear` の
// 入り口）リニアの乗算済みで重ねる（Unity と同じ重ね方）。トーンマッピングのとき（描き先が HDR）は、ガンマの値のまま重ねる。

// ───────── 束ね（group 1 の 7〜10） ─────────

const NP: i32 = 60;
const NS: i32 = 21;
const NU: i32 = 16;

struct Lil {
    p: array<vec4<f32>, 60>,
    // スロットごとの成分の元（-1 既定・-2 は 0・-3 は 1・0 以上は 元 × 4 + 成分。元は 0〜5 標準のチャンネル（Slot の番号）・
    // 6〜21 ユーザーチャンネルの配列の層・32〜33 画像）
    slot_src: array<vec4<i32>, 21>,
    // スロットの既定の値（割り当てていない成分）
    slot_def: array<vec4<f32>, 21>,
    // x: RGB を sRGB からリニアへ（1）、y: 1 つの元の RGBA をそのまま（1）、z: その元の番号
    slot_flags: array<vec4<f32>, 21>,
    // ユーザーチャンネルの配列の層ごとの、何も描いていない所の値（ガンマのまま。スカラーは R）
    user_default: array<vec4<f32>, 16>,
};

@group(1) @binding(7) var<uniform> lil: Lil;
@group(1) @binding(8) var user_tex: texture_2d_array<f32>;
@group(1) @binding(9) var image1_tex: texture_2d<f32>;
@group(1) @binding(10) var image2_tex: texture_2d<f32>;

// 値の並び（render.rs の `liltoon::params` と同じ）
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
const P_EMISSION_X: i32 = 25;     // x 蛍光、y UV
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
const P_OUTLINE_Q: i32 = 58;      // x Z バイアス、y 太さ 0 を消す
const P_FLAGS: i32 = 59;          // x 法線マップを読める（接線がある）

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
    }
    return vec4<f32>(0.0);
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
    if (flags.y > 0.5) {
        return finish_slot(i, fetch_grad(i32(flags.z + 0.5), uv, gx, gy));
    }
    let src = lil.slot_src[i];
    var out = lil.slot_def[i];
    for (var k = 0; k < 4; k = k + 1) {
        let code = src[k];
        if (code >= 0) {
            out[k] = component(fetch_grad(code / 4, uv, gx, gy), code % 4);
        } else if (code == -2) {
            out[k] = 0.0;
        } else if (code == -3) {
            out[k] = 1.0;
        }
    }
    return finish_slot(i, out);
}

fn slot_level(i: i32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let flags = lil.slot_flags[i];
    if (flags.y > 0.5) {
        return finish_slot(i, fetch_level(i32(flags.z + 0.5), uv, lod));
    }
    let src = lil.slot_src[i];
    var out = lil.slot_def[i];
    for (var k = 0; k < 4; k = k + 1) {
        let code = src[k];
        if (code >= 0) {
            out[k] = component(fetch_level(code / 4, uv, lod), code % 4);
        } else if (code == -2) {
            out[k] = 0.0;
        } else if (code == -3) {
            out[k] = 1.0;
        }
    }
    return finish_slot(i, out);
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

fn lil_unpack_normal(t: vec4<f32>, scale: f32) -> vec3<f32> {
    // Unity のノーマルマップの取り込み（DXT5nm）と同じく、XY から Z を作り直す
    let xy = (t.xy * 2.0 - vec2<f32>(1.0)) * scale;
    return vec3<f32>(xy, sqrt(1.0 - saturate(dot(xy, xy))));
}

fn ortho_normalize(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return normalize(t - n * dot(n, t));
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
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = uv0 * main_st.xy + main_st.zw;
    // 勾配は条件の外で（導関数の一様性）
    let gx = dpdx(uv_main);
    let gy = dpdy(uv_main);
    let gx0 = dpdx(uv0);
    let gy0 = dpdy(uv0);

    // メインカラー（3D ビューの約束: Color の何も描いていない所は、不透明なら市松の上に見せる）
    var col = slot_grad(SLOT_MAIN, uv_main, gx, gy);
    if (mode == 0 && lil.slot_flags[SLOT_MAIN].y > 0.5 && i32(lil.slot_flags[SLOT_MAIN].z + 0.5) == 0) {
        let p = textureSampleGrad(color_tex, paint_sampler, uv_main, gx, gy);
        let g = checker_gamma(uv0) * (1.0 - p.a) + p.rgb;
        col = vec4<f32>(srgb_to_linear(g), 1.0);
    }
    // 色調補正
    let before = col.rgb;
    let adjust_mask = slot_grad(SLOT_ADJUST_MASK, uv_main, gx, gy).r;
    col = vec4<f32>(mix(before, lil_tone(col.rgb, lil.p[P_HSVG]), adjust_mask), col.a);
    col = col * lil.p[P_COLOR];

    // 法線
    let geometric = normalize(f.normal);
    var n = geometric;
    let bump = lil.p[P_BUMP];
    let bump_st = lil.p[P_BUMP_ST];
    let bump_tex = slot_grad(SLOT_BUMP, uv_main * bump_st.xy + bump_st.zw, gx * bump_st.xy, gy * bump_st.xy);
    if (bump.x > 0.5 && lil.p[P_FLAGS].x > 0.5) {
        let t = lil_unpack_normal(bump_tex, bump.y);
        n = normalize(f.tangent * t.x + f.bitangent * t.y + geometric * t.z);
    }
    let flip = lil.p[P_LIGHT2].w;
    if (facing < flip - 1.0) {
        n = -n;
    }
    let orig_n = geometric;
    let view = normalize(u.camera.xyz - f.world);
    let nvabs = abs(dot(n, view));

    // 光
    let light = lil_main_light();
    let to_light = light.dir;
    var light_color = light.color;
    let ind_light = light.ind;
    let inv_lighting = sat3((vec3<f32>(1.0) - light_color) * sqrt(light_color));
    let attenuation = 1.0 - shadow_amount(f.world, geometric, normalize(u.light_dir.xyz), f.clip.xy);

    // 影（導関数を使う値は、アルファで捨てる前に取る）
    var shadowmix = 1.0;
    let sp = lil.p[P_SHADOW];
    let lod = lil.p[P_SHADOW_LOD];
    let ssm_tex = slot_grad(SLOT_SHADOW_STRENGTH, uv_main, max(abs(gx), vec2<f32>(lod.x)), max(abs(gy), vec2<f32>(lod.x)));
    let blur_mask = slot_grad(SLOT_SHADOW_BLUR, uv_main, max(abs(gx), vec2<f32>(lod.z)), max(abs(gy), vec2<f32>(lod.z)));
    let border_tex = slot_grad(SLOT_SHADOW_BORDER, uv_main, max(abs(gx), vec2<f32>(lod.y)), max(abs(gy), vec2<f32>(lod.y)));
    let shadow1_tex = slot_grad(SLOT_SHADOW1_TEX, uv_main, gx, gy);
    let shadow2_tex_in = slot_grad(SLOT_SHADOW2_TEX, uv_main, gx, gy);
    let shadow3_tex_in = slot_grad(SLOT_SHADOW3_TEX, uv_main, gx, gy);
    let s1 = lil.p[P_SHADOW1];
    let s2 = lil.p[P_SHADOW2];
    let s3 = lil.p[P_SHADOW3];
    let n1 = mix(orig_n, n, s1.z);
    let n2 = mix(orig_n, n, s2.z);
    let n3 = mix(orig_n, n, s3.z);
    var ssm = ssm_tex;
    var lns = vec4<f32>(1.0);
    lns.x = saturate(dot(to_light, n1) * 0.5 + 0.5);
    lns.y = saturate(dot(to_light, n2) * 0.5 + 0.5);
    lns.z = saturate(dot(to_light, n3) * 0.5 + 0.5);
    let flat = lil.p[P_SHADOW_FLAT];
    let mask_type = i32(sp.z + 0.5);
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
    // 影を受け取る（主な光の影のマップ。影を切っていれば受けない）
    let calculated = saturate(attenuation + distance(to_light, normalize(u.light_dir.xyz)));
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
    // 導関数は条件の外で取る（値の微分なので、マスクの前後どちらの値かは lilToon と同じ）
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
    // リムライト（ライト方向あり）
    let rim = lil.p[P_RIM];
    let rp = lil.p[P_RIM_P];
    let rq = lil.p[P_RIM_Q];
    let rr = lil.p[P_RIM_R];
    let rim_st = lil.p[P_RIM_ST];
    let rim_tex = slot_grad(SLOT_RIM, uv_main * rim_st.xy + rim_st.zw, gx * rim_st.xy, gy * rim_st.xy);
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
    let am = lil.p[P_ALPHA_MASK];
    let am_tex = slot_grad(SLOT_ALPHA_MASK, uv_main, gx, gy).r;
    if (mode != 0 && am.x > 0.5) {
        let mask = saturate(am_tex * am.y + am.z);
        let m = i32(am.x + 0.5);
        if (m == 1) {
            col.a = mask;
        } else if (m == 2) {
            col.a = col.a * mask;
        } else if (m == 3) {
            col.a = saturate(col.a + mask);
        } else if (m == 4) {
            col.a = saturate(col.a - mask);
        }
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
    if (sp.x > 0.5) {
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

    // 乗算済み（半透明）
    if (transparent) {
        col = vec4<f32>(col.rgb * col.a, col.a);
    }

    // マットキャップ
    let mc = lil.p[P_MATCAP];
    let mcp = lil.p[P_MATCAP_P];
    let mcq = lil.p[P_MATCAP_Q];
    let mc_n = normalize(mix(orig_n, n, mcq.x));
    let mc_uv = matcap_uv(mc_n, view, lil.p[P_MATCAP_ST], mcq.y > 0.5, mcq.z > 0.5);
    let mc_tex = slot_level(SLOT_MATCAP, mc_uv, mcp.w);
    let mc_mask_st = lil.p[P_MATCAP_MASK_ST];
    let mc_mask = slot_grad(SLOT_MATCAP_MASK, uv_main * mc_mask_st.xy + mc_mask_st.zw, gx * mc_mask_st.xy, gy * mc_mask_st.xy).rgb;
    if (mc.x > 0.5) {
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
    let mc2p = lil.p[P_MATCAP2_P];
    let mc2q = lil.p[P_MATCAP2_Q];
    // 2nd は lilToon と同じく、混ぜた法線を正規化しないで UV を出す
    let mc2_n = mix(orig_n, n, mc2q.x);
    let mc2_uv = matcap_uv(mc2_n, view, lil.p[P_MATCAP2_ST], mc2q.y > 0.5, mc2q.z > 0.5);
    let mc2_tex = slot_level(SLOT_MATCAP2, mc2_uv, mc2p.w);
    let mc2_mask_st = lil.p[P_MATCAP2_MASK_ST];
    let mc2_mask = slot_grad(SLOT_MATCAP2_MASK, uv_main * mc2_mask_st.xy + mc2_mask_st.zw, gx * mc2_mask_st.xy, gy * mc2_mask_st.xy).rgb;
    if (mc2.x > 0.5) {
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
    let aa_main = lil.p[P_LIGHT2].y;
    if (rim.x > 0.5) {
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

    // 発光
    let em = lil.p[P_EMISSION];
    let emx = lil.p[P_EMISSION_X];
    let em_st = lil.p[P_EMISSION_ST];
    let em_rim = i32(emx.y + 0.5) == 4;
    let em_uv = select(uv0, uv_rim, em_rim);
    let em_tex = slot_grad(SLOT_EMISSION, em_uv * em_st.xy + em_st.zw, select(gx0, gx_rim, em_rim) * em_st.xy, select(gy0, gy_rim, em_rim) * em_st.xy);
    let em_mask_st = lil.p[P_EMISSION_MASK_ST];
    let em_mask = slot_grad(SLOT_EMISSION_MASK, uv0 * em_mask_st.xy + em_mask_st.zw, gx0 * em_mask_st.xy, gy0 * em_mask_st.xy);
    if (em.x > 0.5) {
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
    let em2x = lil.p[P_EMISSION2_X];
    let em2_st = lil.p[P_EMISSION2_ST];
    let em2_rim = i32(em2x.y + 0.5) == 4;
    let em2_uv = select(uv0, uv_rim, em2_rim);
    let em2_tex = slot_grad(SLOT_EMISSION2, em2_uv * em2_st.xy + em2_st.zw, select(gx0, gx_rim, em2_rim) * em2_st.xy, select(gy0, gy_rim, em2_rim) * em2_st.xy);
    let em2_mask_st = lil.p[P_EMISSION2_MASK_ST];
    let em2_mask = slot_grad(SLOT_EMISSION2_MASK, uv0 * em2_mask_st.xy + em2_mask_st.zw, gx0 * em2_mask_st.xy, gy0 * em2_mask_st.xy);
    if (em2.x > 0.5) {
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
    let back = lil.p[P_BACKFACE];
    if (facing < 0.0) {
        col = vec4<f32>(mix(col.rgb, back.rgb * light_color, back.a), col.a);
    }
    if (!transparent) {
        col.a = 1.0;
    }
    return col;
}

// ───────── 輪郭線（裏返しの殻） ─────────

@vertex
fn vs_outline(v: VsIn) -> VsOut {
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = v.uv * main_st.xy + main_st.zw;
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
    let uv = f.uv * st.xy + st.zw;
    let gx = dpdx(uv);
    let gy = dpdy(uv);
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
    let am = lil.p[P_ALPHA_MASK];
    let main_st = lil.p[P_MAIN_ST];
    let am_tex = slot_grad(SLOT_ALPHA_MASK, f.uv * main_st.xy + main_st.zw, gx, gy).r;
    if (mode != 0 && am.x > 0.5) {
        let mask = saturate(am_tex * am.y + am.z);
        let m = i32(am.x + 0.5);
        if (m == 1) {
            col.a = mask;
        } else if (m == 2) {
            col.a = col.a * mask;
        } else if (m == 3) {
            col.a = saturate(col.a + mask);
        } else if (m == 4) {
            col.a = saturate(col.a - mask);
        }
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
    return col;
}

// 世界の向きを視点の空間の XY へ（lilTransformDirWStoVSCenter の xy）。
fn view_dir_xy(d: vec3<f32>) -> vec2<f32> {
    // Unity の視点の行列の 0 行目（右）。front は手前への向き（−前）なので、右 = 上 × 前 = front × 上
    let right = cross(u.lil_camera_front.xyz, u.lil_camera_up.xyz);
    return vec2<f32>(dot(d, right), dot(d, u.lil_camera_up.xyz));
}
