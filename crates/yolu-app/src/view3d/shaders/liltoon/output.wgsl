// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_frag.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

