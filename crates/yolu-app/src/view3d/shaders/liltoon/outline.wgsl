// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_functions.hlsl・lil_common_frag.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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
    // lilCalcOutlinePosition の V（Z バイアスはこの逆へ押す）: 透視はカメラの位置への向き、正投影はカメラの手前への向き（LIL_MATRIX_V の 3 行目）
    var to_camera = u.camera.xyz - p;
    if (!lil_is_perspective()) {
        to_camera = u.lil_camera_front.xyz;
    }
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

