// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）に同梱の OpenLit Library 1.0.2（CC0）の ComputeLights と lil_common_macro.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

