// ミップマップを 1 段ずつ作る（1 つ上の段を読んで下の段へ描く。どのチャンネルの形式でも同じ）。
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // 1 つ上の段の 2 × 2 の真ん中を双線形で読む（箱のフィルター）
    let size = vec2<f32>(max(textureDimensions(src) / 2u, vec2<u32>(1u, 1u)));
    return textureSampleLevel(src, samp, p.xy / size, 0.0);
}
