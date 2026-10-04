// ユーザーチャンネルの配列のミップマップを 1 段ずつ作る（mip.wgsl と同じ箱のフィルター）。読む元は配列の見え方のまま、層を値で選ぶ
// （GL は配列の 1 層だけの 2D の見え方を読めないので、どの機材でもこの形にする）。
@group(0) @binding(0) var src: texture_2d_array<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> layer: vec4<u32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // 1 つ上の段の 2 × 2 の真ん中を双線形で読む（見え方は 1 つ上の段だけを持つので、その段の大きさの半分が描き先の大きさ）
    let size = vec2<f32>(max(textureDimensions(src) / 2u, vec2<u32>(1u, 1u)));
    return textureSampleLevel(src, samp, p.xy / size, layer.x, 0.0);
}
