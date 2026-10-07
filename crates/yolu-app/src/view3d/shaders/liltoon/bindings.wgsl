// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の値を 3D ビューの束ねへ渡す形（この部品の束ね方は 3D ビューのもの）。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
// ───────── 束ね（group 1 の 7〜10） ─────────

const NP: i32 = 122;
const NS: i32 = 38;
const NU: i32 = 16;

struct Lil {
    p: array<vec4<f32>, 122>,
    // スロットごとの成分の元（-1 既定・-2 は 0・-3 は 1・0 以上は 元 × 4 + 成分。元は 0〜5 標準のチャンネル（Slot の番号）・
    // 6〜21 ユーザーチャンネルの配列のレイヤー・32〜33 画像・40〜55 Unity から受けた絵の配列のレイヤー）
    slot_src: array<vec4<i32>, 38>,
    // スロットの既定の値（割り当てていない成分）
    slot_def: array<vec4<f32>, 38>,
    // x: RGB を sRGB からリニアへ（1）、y: 1 つの元の RGBA をそのまま（1）、z: その元の番号
    slot_flags: array<vec4<f32>, 38>,
    // ユーザーチャンネルの配列のレイヤーごとの、何も描いていない所の値（ガンマのまま。スカラーは R）
    user_default: array<vec4<f32>, 16>,
};

@group(1) @binding(7) var<uniform> lil: Lil;
@group(1) @binding(8) var user_tex: texture_2d_array<f32>;
@group(1) @binding(9) var image1_tex: texture_2d<f32>;
@group(1) @binding(10) var image2_tex: texture_2d<f32>;
// Live Link で Unity から受けた、描いていないスロットの絵（レイヤー 1 つがスロット 1 つ。元の番号 40〜55。Unity が読むのと同じ straight で、
// 割り戻さない。A は不透明度とは限らない: DXT5nm のノーマルマップは A に X を持つ）
@group(1) @binding(11) var received_tex: texture_2d_array<f32>;
const NR: i32 = 16;

