// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）のテクスチャの読み方（スロットの割り当ては 3D ビューのもの）。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

