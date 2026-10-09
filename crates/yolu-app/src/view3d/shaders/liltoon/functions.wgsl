// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_functions.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
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

// lilIsPerspective()（Unity の unity_OrthoParams.w == 0。u.camera.w は正投影なら 1）
fn lil_is_perspective() -> bool {
    return u.camera.w < 0.5;
}

// lilCalcMatCapUV: 透視でマテリアルの「透視」が入なら視線、ほかはカメラの手前への向き
fn matcap_uv(n: vec3<f32>, v: vec3<f32>, st: vec4<f32>, zrot_cancel: bool, perspective: bool) -> vec2<f32> {
    var nvd = v;
    if (!(lil_is_perspective() && perspective)) {
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

