// 3D ビューの選択範囲の重ね。面の UV から選択範囲の量（R8。ミップつき）を読み、面の上に描く。
// - 縁（style 1）: 量 0.5 の等値線を、画面の 1 画素あたりの量の変わり方（微分）で画素の距離にして、1〜2 点の太さの線にする。
//   線は白と黒の点線で、点線の位相は画面の座標（拡大・縮小しても太さと間隔は変わらない）。
// - 赤い重ね（style 2。クイックマスク）: 量 × 0.5 の濃さの赤。
// - 被覆（選択ペンの途中）: 量 × 0.45 の濃さの青（追加）か橙（削除）を、下に重ねる。
// 描き先は面と同じ（深さは面の描きのまま読むだけ）で、出力は「画面にそのまま出す値」（ガンマ）の通常の混ぜ（乗算なしの α）。

struct Globals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: Globals;

struct Params {
    // x: 表示（0 なし・1 縁・2 赤い重ね）、y: 点線の位相（画素）、z: 線の太さの半分（画素）、w: 点線の 1 周期（画素）
    a: vec4<f32>,
    // x: 縁の白の値（トーンマッピングを通すときは、通したあとに白く見える値）、y: 被覆（0 なし・1 追加・2 削除）
    b: vec4<f32>,
};
@group(1) @binding(0) var mask_tex: texture_2d<f32>;
@group(1) @binding(1) var cover_tex: texture_2d<f32>;
@group(1) @binding(2) var samp: sampler;
@group(1) @binding(3) var<uniform> p: Params;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(v.position, 1.0);
    o.uv = v.uv;
    return o;
}

@fragment
fn fs_main(f: VsOut) -> @location(0) vec4<f32> {
    // 微分は一様な流れの中で取る
    let m = textureSample(mask_tex, samp, f.uv).r;
    let cover = textureSample(cover_tex, samp, f.uv).r;
    let grad = vec2<f32>(dpdx(m), dpdy(m));
    let grad_len = length(grad);

    // 下のレイヤー: 被覆の色（追加: 青、削除: 橙）。乗算済みで持つ
    var pm = vec3<f32>(0.0);
    var alpha = 0.0;
    if (p.b.y > 0.5 && cover > 0.0) {
        let tint = select(vec3<f32>(70.0, 150.0, 255.0), vec3<f32>(255.0, 160.0, 40.0), p.b.y > 1.5) / 255.0;
        alpha = 0.45 * cover;
        pm = tint * alpha;
    }
    // 上のレイヤー
    var top_pm = vec3<f32>(0.0);
    var top_a = 0.0;
    if (p.a.x > 1.5) {
        // 赤い重ね
        top_a = 0.5 * m;
        top_pm = vec3<f32>(255.0, 48.0, 48.0) / 255.0 * top_a;
    } else if (p.a.x > 0.5 && grad_len > 1e-5) {
        // 縁: 量が 0.5 の線までの画素の距離
        let d = abs(m - 0.5) / grad_len;
        let line = clamp(p.a.z + 0.5 - d, 0.0, 1.0);
        if (line > 0.0) {
            // 点線: 縁が横向きなら x、縦向きなら y の画面の座標に沿って、白と黒を半周期ずつ
            let tangent = vec2<f32>(-grad.y, grad.x);
            let along = select(f.clip.y, f.clip.x, abs(tangent.x) > abs(tangent.y));
            let black = fract((along + p.a.y) / p.a.w) < 0.5;
            let color = select(vec3<f32>(p.b.x), vec3<f32>(0.0), black);
            top_a = line;
            top_pm = color * line;
        }
    }
    // 上のレイヤーを下のレイヤーに重ねる（乗算済みの over）
    let out_a = top_a + alpha * (1.0 - top_a);
    let out_pm = top_pm + pm * (1.0 - top_a);
    if (out_a <= 0.002) {
        discard;
    }
    return vec4<f32>(out_pm / out_a, out_a);
}
