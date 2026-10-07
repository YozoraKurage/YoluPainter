// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_common_macro.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
// 世界の向きを視点の空間の XY へ（lilTransformDirWStoVSCenter の xy）。
fn view_dir_xy(d: vec3<f32>) -> vec2<f32> {
    // Unity の視点の行列の 0 行目（右）。front は手前への向き（−前）なので、右 = 上 × 前 = front × 上
    let right = cross(u.lil_camera_front.xyz, u.lil_camera_up.xyz);
    return vec2<f32>(dot(d, right), dot(d, u.lil_camera_up.xyz));
}
