// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_frag.hlsl・lil_common_functions.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

