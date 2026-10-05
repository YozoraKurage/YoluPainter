// 3D ビューのブルーム: HDR の描き先から明るい所を取り出して縮めながらぼかし（13 点の縮小。最初の段はしきい値と、1 点だけ飛び抜けた
// 明るさが全体を光らせないための Karis の平均つき）、拡大しながら 3×3 のテントでぼかして足し戻す。トーンマッピングの前（HDR）で、
// 足すのは tonemap.wgsl の側。値は最初の段だけ「画面にそのまま出す値」（ガンマ）で入り、リニアに直して露出を掛けてから、あとの段はずっとリニア。
struct Params {
    // x: しきい値（露出を掛けたあとのリニアの明るさ）、y: ひざ（しきい値の下でなめらかに立ち上げる幅）、
    // z: 拡大で足すときの重み、w: 露出の倍率（2^EV）
    p: vec4<f32>,
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: VsOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn tap(uv: vec2<f32>, ts: vec2<f32>, dx: f32, dy: f32) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv + vec2<f32>(dx, dy) * ts, 0.0).rgb;
}

// ガンマの HDR の値 → リニア × 露出 → しきい値の上（ひざでなめらかに）だけを残す。f16 に収まるよう上を切る。
fn bright(c: vec3<f32>) -> vec3<f32> {
    let lin = min(srgb_to_linear(max(c, vec3<f32>(0.0))) * params.p.w, vec3<f32>(1000.0));
    let br = max(lin.r, max(lin.g, lin.b));
    let knee = params.p.y;
    var soft = clamp(br - params.p.x + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee + 1e-5);
    let gain = max(soft, br - params.p.x) / max(br, 1e-5);
    return lin * gain;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// 13 点の縮小の 5 つの組（中央の 4 点が半分、四隅の 4 組が 1/8 ずつ）の重みつきの平均に、Karis の重み 1 / (1 + 明るさ) を掛ける。
@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    let ts = 1.0 / vec2<f32>(textureDimensions(src));
    let a = bright(tap(in.uv, ts, -2.0, 2.0));
    let b = bright(tap(in.uv, ts, 0.0, 2.0));
    let c = bright(tap(in.uv, ts, 2.0, 2.0));
    let d = bright(tap(in.uv, ts, -2.0, 0.0));
    let e = bright(tap(in.uv, ts, 0.0, 0.0));
    let f = bright(tap(in.uv, ts, 2.0, 0.0));
    let g = bright(tap(in.uv, ts, -2.0, -2.0));
    let h = bright(tap(in.uv, ts, 0.0, -2.0));
    let i = bright(tap(in.uv, ts, 2.0, -2.0));
    let j = bright(tap(in.uv, ts, -1.0, 1.0));
    let k = bright(tap(in.uv, ts, 1.0, 1.0));
    let l = bright(tap(in.uv, ts, -1.0, -1.0));
    let m = bright(tap(in.uv, ts, 1.0, -1.0));
    let g0 = (j + k + l + m) * 0.25;
    let g1 = (a + b + d + e) * 0.25;
    let g2 = (b + c + e + f) * 0.25;
    let g3 = (d + e + g + h) * 0.25;
    let g4 = (e + f + h + i) * 0.25;
    let w0 = 0.5 / (1.0 + luma(g0));
    let w1 = 0.125 / (1.0 + luma(g1));
    let w2 = 0.125 / (1.0 + luma(g2));
    let w3 = 0.125 / (1.0 + luma(g3));
    let w4 = 0.125 / (1.0 + luma(g4));
    let sum = g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4;
    return vec4<f32>(sum / (w0 + w1 + w2 + w3 + w4), 1.0);
}

// 2 段目以降の縮小（リニアのまま。重みは合計 1）。
@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    let ts = 1.0 / vec2<f32>(textureDimensions(src));
    let a = tap(in.uv, ts, -2.0, 2.0);
    let b = tap(in.uv, ts, 0.0, 2.0);
    let c = tap(in.uv, ts, 2.0, 2.0);
    let d = tap(in.uv, ts, -2.0, 0.0);
    let e = tap(in.uv, ts, 0.0, 0.0);
    let f = tap(in.uv, ts, 2.0, 0.0);
    let g = tap(in.uv, ts, -2.0, -2.0);
    let h = tap(in.uv, ts, 0.0, -2.0);
    let i = tap(in.uv, ts, 2.0, -2.0);
    let j = tap(in.uv, ts, -1.0, 1.0);
    let k = tap(in.uv, ts, 1.0, 1.0);
    let l = tap(in.uv, ts, -1.0, -1.0);
    let m = tap(in.uv, ts, 1.0, -1.0);
    let sum = (j + k + l + m) * 0.125 + (a + b + d + e) * 0.03125 + (b + c + e + f) * 0.03125
        + (d + e + g + h) * 0.03125 + (e + f + h + i) * 0.03125;
    return vec4<f32>(sum, 1.0);
}

// 拡大: 1 つ小さい段を 3×3 のテント（合計 1）で広げ、散らしの重みを掛けて、1 つ大きい段へ加算で重ねる（パイプラインの加算の混合）。
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let ts = 1.0 / vec2<f32>(textureDimensions(src));
    let sum = tap(in.uv, ts, 0.0, 0.0) * 4.0
        + (tap(in.uv, ts, -1.0, 0.0) + tap(in.uv, ts, 1.0, 0.0) + tap(in.uv, ts, 0.0, -1.0) + tap(in.uv, ts, 0.0, 1.0)) * 2.0
        + (tap(in.uv, ts, -1.0, -1.0) + tap(in.uv, ts, 1.0, -1.0) + tap(in.uv, ts, -1.0, 1.0) + tap(in.uv, ts, 1.0, 1.0));
    return vec4<f32>(sum * (params.p.z / 16.0), 1.0);
}
