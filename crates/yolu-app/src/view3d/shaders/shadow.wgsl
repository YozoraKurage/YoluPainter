// 3D ビューの影のマップ（光から見た深さだけを描く）。Unity 版の PreviewShadow.shader のパス 0（CASTER）と同じ向きの約束:
// `matrix` は世界 → (u, v, 深さ)。u・v は 0〜1（v は下向き）、深さは 0〜1 で 0 が光の側（外接球の直径が 1）。描き先はその深さをそのまま
// 深度バッファに書く（scene.wgsl が比較サンプラーで読む）。

struct Shadow {
    matrix: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> shadow: Shadow;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    let s = shadow.matrix * vec4<f32>(position, 1.0);
    return vec4<f32>(s.x * 2.0 - 1.0, 1.0 - s.y * 2.0, s.z, 1.0);
}
