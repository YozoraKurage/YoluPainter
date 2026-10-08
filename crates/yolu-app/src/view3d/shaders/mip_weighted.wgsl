// 表示の写しのミップマップを、UV の上の画素だけで作る（押し引き）。重み（R8）は、そのテクセルが覆い・塗り広げの中の画素をどれだけ含むか
// （0 はまったく含まない）。どのチャンネルの形式でも同じ式（sRGB の形式は読むときリニア・書くときに sRGB）。
//
// 押し（fs_push）: 1 つ上の段の箱（偶数の辺なら 2 × 2、奇数の辺は 2〜3 の幅）の重みつきの平均。重みが全部 0 の箱は仮に平均を書く（引きが書き換える）。
// 引き（fs_pull）: 重みが 0 のテクセルだけ、1 つ粗い段を真ん中で双線形に読んだ値で書き換える。読むとき、粗い段の重みが 0 のテクセルは数えない
// （4 つとも 0 のときだけ、全部を数える）。重みが 0 でないテクセルは捨てる（押した値のまま）。
@group(0) @binding(0) var src: texture_2d<f32>;
// 押し: src の段の重み。引き: src（1 つ粗い段）の重み。
@group(0) @binding(1) var wsrc: texture_2d<f32>;
// 描き先の段の重み。
@group(0) @binding(2) var wdst: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// 描き先のテクセル o（全体 m）が受け持つ、1 つ上の段（n）のテクセルの範囲 [lo, hi)。上の段の画素 y は floor((2y + 1) * m / (2n)) の箱に入る
// （箱の真ん中を、上の段の座標へ比例して写した位置に置く）。引きの双線形の読み位置と同じ写しなので、重みが 0 でないテクセルのとなりの
// テクセルが読む 4 つの中に、必ず重みが 0 でないテクセルが入る。
fn first_of(o: u32, n: u32, m: u32) -> u32 {
    return (2u * o * n + m - 1u) / (2u * m);
}

@fragment
fn fs_push(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let n = textureDimensions(src);
    let m = max(n / 2u, vec2<u32>(1u, 1u));
    let o = vec2<u32>(p.xy);
    // 1 つ上の段の 1 つの箱が 2 × 2 のとき（両方の辺が偶数。ミップの段の大きさのほとんど）は、ループなしで同じ順に足す
    if (n.x == 2u * m.x && n.y == 2u * m.y) {
        let a = vec2<u32>(o.x * 2u, o.y * 2u);
        let c0 = textureLoad(src, a, 0);
        let c1 = textureLoad(src, a + vec2<u32>(1u, 0u), 0);
        let c2 = textureLoad(src, a + vec2<u32>(0u, 1u), 0);
        let c3 = textureLoad(src, a + vec2<u32>(1u, 1u), 0);
        let w0 = textureLoad(wsrc, a, 0).r;
        let w1 = textureLoad(wsrc, a + vec2<u32>(1u, 0u), 0).r;
        let w2 = textureLoad(wsrc, a + vec2<u32>(0u, 1u), 0).r;
        let w3 = textureLoad(wsrc, a + vec2<u32>(1u, 1u), 0).r;
        let total = w0 + w1 + w2 + w3;
        if (total > 0.0) {
            return (c0 * w0 + c1 * w1 + c2 * w2 + c3 * w3) / total;
        }
        return (c0 + c1 + c2 + c3) / 4.0;
    }
    let x0 = first_of(o.x, n.x, m.x);
    let x1 = first_of(o.x + 1u, n.x, m.x);
    let y0 = first_of(o.y, n.y, m.y);
    let y1 = first_of(o.y + 1u, n.y, m.y);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    var plain = vec4<f32>(0.0);
    var count = 0.0;
    for (var y = y0; y < y1; y = y + 1u) {
        for (var x = x0; x < x1; x = x + 1u) {
            let c = textureLoad(src, vec2<u32>(x, y), 0);
            let w = textureLoad(wsrc, vec2<u32>(x, y), 0).r;
            sum = sum + c * w;
            weight = weight + w;
            plain = plain + c;
            count = count + 1.0;
        }
    }
    if (weight > 0.0) {
        return sum / weight;
    }
    return plain / count;
}

@fragment
fn fs_pull(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let o = vec2<u32>(p.xy);
    if (textureLoad(wdst, o, 0).r > 0.0) {
        discard;
    }
    let m = vec2<i32>(textureDimensions(src));
    let n = vec2<f32>(textureDimensions(wdst));
    // 双線形の読み位置（1 つ粗い段のテクセルの座標。テクセルの真ん中が 0.5）
    let f = (vec2<f32>(o) + vec2<f32>(0.5)) * vec2<f32>(m) / n - vec2<f32>(0.5);
    let f0 = floor(f);
    let t = f - f0;
    let hi = m - vec2<i32>(1);
    let a = clamp(vec2<i32>(f0), vec2<i32>(0), hi);
    let b = clamp(vec2<i32>(f0) + vec2<i32>(1), vec2<i32>(0), hi);
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    var plain = vec4<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let right = (k & 1) == 1;
        let up = (k & 2) == 2;
        let at = vec2<i32>(select(a.x, b.x, right), select(a.y, b.y, up));
        let bx = select(1.0 - t.x, t.x, right);
        let by = select(1.0 - t.y, t.y, up);
        let bilinear = bx * by;
        let c = textureLoad(src, at, 0);
        plain = plain + c * bilinear;
        if (textureLoad(wsrc, at, 0).r > 0.0) {
            sum = sum + c * bilinear;
            weight = weight + bilinear;
        }
    }
    if (weight > 0.0) {
        return sum / weight;
    }
    return plain;
}
