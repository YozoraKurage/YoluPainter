// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_frag.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

