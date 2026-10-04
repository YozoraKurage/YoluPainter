// 3D ビューの絵だけに当てる露出とトーンマッピング（Unity 版の PreviewToneMap.shader と同じ式。brdf.rs の tone_map が CPU の参照）。
// 入力は HDR の描き先で、値は「画面にそのまま出す値」（ガンマ）。ガンマの値としてリニアに直し → 露出の倍率 → 曲線 → 0〜1 で切る → ガンマに戻す。
struct Params {
    // x: 曲線（0 なし・1 Neutral・2 ACES）、y: 露出の倍率（2^EV）
    p: vec4<f32>,
};
@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let v = max(c, vec3<f32>(0.0));
    let lo = v * 12.92;
    let hi = 1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, v <= vec3<f32>(0.0031308));
}

fn neutral_curve(x: vec3<f32>, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> vec3<f32> {
    return ((x * (a * x + vec3<f32>(c * b)) + vec3<f32>(d * e)) / (x * (a * x + vec3<f32>(b)) + vec3<f32>(d * f))) - vec3<f32>(e / f);
}

fn neutral(x: vec3<f32>) -> vec3<f32> {
    let a = 0.2; let b = 0.29; let c = 0.24; let d = 0.272; let e = 0.02; let f = 0.3; let white = 5.3;
    let white_scale = 1.0 / neutral_curve(vec3<f32>(white), a, b, c, d, e, f).x;
    return neutral_curve(x * white_scale, a, b, c, d, e, f) * white_scale;
}

fn aces(color: vec3<f32>) -> vec3<f32> {
    let v = vec3<f32>(
        0.59719 * color.x + 0.35458 * color.y + 0.04823 * color.z,
        0.07600 * color.x + 0.90834 * color.y + 0.01566 * color.z,
        0.02840 * color.x + 0.13383 * color.y + 0.83777 * color.z,
    );
    let a = v * (v + vec3<f32>(0.0245786)) - vec3<f32>(0.000090537);
    let b = v * (0.983729 * v + vec3<f32>(0.4329510)) + vec3<f32>(0.238081);
    let c = a / b;
    return vec3<f32>(
        1.60475 * c.x - 0.53108 * c.y - 0.07367 * c.z,
        -0.10208 * c.x + 1.10813 * c.y - 0.00605 * c.z,
        -0.00327 * c.x - 0.07276 * c.y + 1.07602 * c.z,
    );
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(hdr, vec2<i32>(p.xy), 0);
    var x = srgb_to_linear(max(c.rgb, vec3<f32>(0.0)));
    x = x * params.p.y;
    if (params.p.x > 1.5) {
        x = aces(x);
    } else if (params.p.x > 0.5) {
        x = neutral(x);
    }
    x = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(linear_to_srgb(x), 1.0);
}
