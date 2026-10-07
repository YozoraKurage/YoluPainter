// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_functions.hlsl・lil_common_functions_thirdparty.hlsl（lilHashRGB4 の元は The Unlicense）。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

