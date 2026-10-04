// メッシュマップのベイク（yolu-core の mesh_maps::bake の行ごとの計算と同じ式を f32 で）。
// 1 スレッドが 1 テクセルの全サンプルを受け持つ: UV の所有者（ラスタ）→ 高ポリへの投影 → 各マップの値 → AO・厚み・ベントノーマルのレイ →
// サンプルの平均と 16 bit への量子化。BVH・曲率の線分・ID・接空間は core が CPU で作った準備済みの値を読むだけ（式を別に作らない）。
// 余白と記録は core の後始末（CPU）。出力は 1 チャンネル 1 ワード（下位 16 bit）で、書くのは自分のテクセルだけ。

struct Params {
    map_offsets: array<vec4<u32>, 3>,
    cvf: array<vec4<f32>, 4>,
    cvu: array<vec4<u32>, 2>,
    width: u32, height: u32, aa: u32, band_first: u32,
    band_texels: u32, local_start: u32, count: u32, out_stride: u32,
    flags: u32, tiles_x: u32, tile_off_off: u32, tile_tri_off: u32,
    bvh_table_off: u32, low_bvh: u32, high_bvh: u32, ao_count: u32,
    th_count: u32, ao_dirs_off: u32, th_dirs_off: u32, lo_f_stride: u32,
    lo_cage_off: u32, lo_frame_off: u32, pad0: u32, pad1: u32,
    ao_cos2: f32, th_cos2: f32, ray_offset: f32, ao_max: f32,
    th_max: f32, frontal: f32, rear: f32, range: f32,
    min_x: f32, min_y: f32, min_z: f32, pad2: f32,
    scale_x: f32, scale_y: f32, scale_z: f32, pad3: f32,
}
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> lo_f: array<f32>;
@group(0) @binding(2) var<storage, read> lo_u: array<u32>;
@group(0) @binding(3) var<storage, read> hi_f: array<f32>;
@group(0) @binding(4) var<storage, read> hi_u: array<u32>;
@group(0) @binding(5) var<storage, read> idx: array<u32>;
@group(0) @binding(6) var<storage, read> bvh_nodes: array<u32>;
@group(0) @binding(7) var<storage, read> bvh_tris: array<u32>;
@group(0) @binding(8) var<storage, read> misc: array<u32>;
@group(0) @binding(9) var<storage, read_write> out: array<u32>;
@group(0) @binding(10) var<storage, read_write> cov: array<u32>;
@group(0) @binding(11) var<storage, read_write> counters: array<atomic<u32>, 4>;
//@EXTRA_BINDINGS@

const NONE: u32 = 0xffffffffu;
const LU: u32 = 16u;
const HF: u32 = 21u;
const F_NORMALS: u32 = 1u;
const F_PROJ: u32 = 2u;
const F_BYNAME: u32 = 4u;
const F_FRAMES: u32 = 8u;
const F_IDS: u32 = 16u;
const F_UVISLAND: u32 = 32u;
const F_CURV: u32 = 64u;
const F_RAYS: u32 = 128u;
const F_AO: u32 = 256u;
const F_TH: u32 = 512u;
const F_ANYHIT: u32 = 1024u;
const F_BACK: u32 = 2048u;
const F_HI_NORMALS: u32 = 4096u;
const F_RQ_FLIP: u32 = 8192u;
// UV の覆いの判定。core は f64 で -1e-9 / 1e-6 だが、ここは f32 の丸め（重心座標でおよそ 2e-7）より広く取る。
const EPS_COVER: f32 = 1e-6;
const EPS_INTERIOR: f32 = 5e-6;
const PI: f32 = 3.14159265358979;

struct Hit { found: bool, t: f32, tri: u32, u: f32, v: f32 }
fn has(flag: u32) -> bool { return (P.flags & flag) != 0u; }
fn lo3(i: u32) -> vec3<f32> { return vec3<f32>(lo_f[i], lo_f[i + 1u], lo_f[i + 2u]); }
fn hi3(i: u32) -> vec3<f32> { return vec3<f32>(hi_f[i], hi_f[i + 1u], hi_f[i + 2u]); }
// 整数と浮動小数が混ざる配列は u32 で持ち、浮動小数は bitcast で読む（小さい整数の ビット列を f32 として読むと非正規数で、
// 実装によっては 0 に潰れるため、整数を f32 の配列に入れない）。
fn bf(a: u32) -> f32 { return bitcast<f32>(a); }
fn mi3(i: u32) -> vec3<f32> { return vec3<f32>(bf(misc[i]), bf(misc[i + 1u]), bf(misc[i + 2u])); }
fn interp3(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, u: f32, v: f32) -> vec3<f32> {
    let w = 1.0 - u - v;
    return a * w + b * u + c * v;
}
fn hash32(x_in: u32) -> u32 {
    var x = x_in;
    x ^= x >> 16u; x = x * 0x7feb352du;
    x ^= x >> 15u; x = x * 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

// ───────── 面の上の点（core の Surface::point） ─────────
struct Surf { p: vec3<f32>, g: vec3<f32>, n: vec3<f32> }
fn low_point(r: u32, u: f32, v: f32) -> Surf {
    let o = r * P.lo_f_stride;
    let p = interp3(lo3(o), lo3(o + 3u), lo3(o + 6u), u, v);
    let g = lo3(o + 18u);
    var n = g;
    if (has(F_NORMALS)) {
        let s = interp3(lo3(o + 9u), lo3(o + 12u), lo3(o + 15u), u, v);
        let l = length(s);
        if (l > 1e-6 && dot(s, g) > 0.0) { n = s / l; }
    }
    return Surf(p, g, n);
}
fn high_point(t: u32, u: f32, v: f32) -> Surf {
    let o = t * HF;
    let p = interp3(hi3(o), hi3(o + 3u), hi3(o + 6u), u, v);
    let g = hi3(o + 18u);
    var n = g;
    if (has(F_HI_NORMALS)) {
        let s = interp3(hi3(o + 9u), hi3(o + 12u), hi3(o + 15u), u, v);
        let l = length(s);
        if (l > 1e-6 && dot(s, g) > 0.0) { n = s / l; }
    }
    return Surf(p, g, n);
}

// ───────── UV のラスタ（core の Raster::row と同じ: 番号の小さい三角形が所有し、両方が内側なら重なり） ─────────
struct Owner { r: i32, u: f32, v: f32, overlap: bool }
fn raster_sample(x: u32, y: u32, i: u32, j: u32) -> Owner {
    let n = f32(P.aa);
    let sx = (f32(i) + 0.5) / n;
    let sy = (f32(j) + 0.5) / n;
    let tile = (y >> 3u) * P.tiles_x + (x >> 3u);
    let a = idx[P.tile_off_off + tile];
    let b = idx[P.tile_off_off + tile + 1u];
    let xi = i32(x);
    let yi = i32(y);
    var res = Owner(-1, 0.0, 0.0, false);
    var owner_interior = false;
    for (var k = a; k < b; k++) {
        let r = idx[P.tile_tri_off + k];
        let ri = r * LU + 8u;
        let x0 = bitcast<i32>(lo_u[ri + 2u]);
        let x1 = bitcast<i32>(lo_u[ri + 3u]);
        let y0 = bitcast<i32>(lo_u[ri + 4u]);
        let y1 = bitcast<i32>(lo_u[ri + 5u]);
        if (yi < y0 || yi > y1 || xi < x0 || xi > x1) { continue; }
        let ax = bitcast<i32>(lo_u[ri]);
        let ay = bitcast<i32>(lo_u[ri + 1u]);
        let rf = r * P.lo_f_stride + 21u;
        let dx = f32(xi - ax) + (sx - lo_f[rf]);
        let dy = f32(yi - ay) + (sy - lo_f[rf + 1u]);
        let u = lo_f[rf + 2u] * dx + lo_f[rf + 3u] * dy;
        let v = lo_f[rf + 4u] * dx + lo_f[rf + 5u] * dy;
        let w = 1.0 - u - v;
        let m = min(w, min(u, v));
        if (m < -EPS_COVER) { continue; }
        let interior = m > EPS_INTERIOR;
        if (res.r < 0) {
            res = Owner(i32(r), u, v, false);
            owner_interior = interior;
        } else if (interior && owner_interior) {
            res.overlap = true;
        }
    }
    return res;
}

// ───────── 曲率（core の Curvature::evaluate。セルの引きは開番地法の表） ─────────
fn cell_lookup(ci: u32, x: u32, y: u32, z: u32) -> vec2<u32> {
    let off = P.cvu[ci].y;
    let mask = P.cvu[ci].z;
    var slot = hash32((x * 73856093u) ^ (y * 19349663u) ^ (z * 83492791u)) & mask;
    for (var probe = 0u; probe <= mask; probe++) {
        let e = off + slot * 5u;
        let ex = idx[e];
        if (ex == NONE) { return vec2<u32>(0u, 0u); }
        if (ex == x && idx[e + 1u] == y && idx[e + 2u] == z) { return vec2<u32>(idx[e + 3u], idx[e + 4u]); }
        slot = (slot + 1u) & mask;
    }
    return vec2<u32>(0u, 0u);
}
fn antiderivative(a: f32, b: f32, c: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    return a * a * t + a * b * t2 + (b * b + 2.0 * a * c) * t3 / 3.0 + b * c * t2 * t2 / 2.0 + c * c * t3 * t2 / 5.0;
}
fn curvature_at(ci: u32, p: vec3<f32>, comp: u32) -> f32 {
    if (P.cvu[ci].w == 0u) { return 0.0; }
    let radius = P.cvf[ci * 2u].x;
    let cell = P.cvf[ci * 2u].y;
    let norm = P.cvf[ci * 2u].z;
    let mn = P.cvf[ci * 2u + 1u].xyz;
    let seg_base = P.cvu[ci].x;
    let reach = 1.5 * radius;
    let r2 = radius * radius;
    let inv = 1.0 / r2;
    let lo = max(vec3<i32>(floor((p - vec3<f32>(reach) - mn) / cell)), vec3<i32>(0));
    let hi = min(vec3<i32>(floor((p + vec3<f32>(reach) - mn) / cell)), lo + vec3<i32>(3));
    var sum = 0.0;
    for (var cx = lo.x; cx <= hi.x; cx++) {
        for (var cy = lo.y; cy <= hi.y; cy++) {
            for (var cz = lo.z; cz <= hi.z; cz++) {
                let range = cell_lookup(ci, u32(cx), u32(cy), u32(cz));
                for (var s = range.x; s < range.x + range.y; s++) {
                    let sb = (seg_base + s) * 8u;
                    if (misc[sb + 7u] != comp) { continue; }
                    let m = mi3(sb) - p;
                    let e = mi3(sb + 3u) - mi3(sb);
                    let ee = dot(e, e);
                    let me = dot(m, e);
                    let mm = dot(m, m);
                    if (ee <= 0.0) { continue; }
                    let disc = me * me - ee * (mm - r2);
                    if (disc <= 0.0) { continue; }
                    let root = sqrt(disc);
                    let t0 = max((-me - root) / ee, 0.0);
                    let t1 = min((-me + root) / ee, 1.0);
                    if (t1 <= t0) { continue; }
                    let a = 1.0 - mm * inv;
                    let b = -2.0 * me * inv;
                    let c = -ee * inv;
                    let integral = antiderivative(a, b, c, t1) - antiderivative(a, b, c, t0);
                    sum += bf(misc[sb + 6u]) * integral * sqrt(ee);
                }
            }
        }
    }
    return sum / norm;
}

// ───────── 接空間（core の Frames::tangent_space） ─────────
fn tangent_space(r: u32, u: f32, v: f32, m: vec3<f32>) -> vec3<f32> {
    let o = r * P.lo_f_stride + P.lo_frame_off;
    let tv = interp3(lo3(o), lo3(o + 3u), lo3(o + 6u), u, v);
    let b = interp3(lo3(o + 9u), lo3(o + 12u), lo3(o + 15u), u, v);
    let n = interp3(lo3(o + 18u), lo3(o + 21u), lo3(o + 24u), u, v);
    let bn = cross(b, n);
    let det = dot(tv, bn);
    var value: vec3<f32>;
    if (abs(det) > 1e-12) {
        value = vec3<f32>(dot(m, bn) / det, dot(tv, cross(m, n)) / det, dot(tv, cross(b, m)) / det);
    } else {
        value = vec3<f32>(dot(m, tv), dot(m, b), dot(m, n));
    }
    let l = length(value);
    if (l > 1e-12) { return value / l; }
    return vec3<f32>(0.0, 0.0, 1.0);
}

// ───────── レイ（core の Rays::trace。乱数の列は core と同じ整数のハッシュ） ─────────
// 2π × turns の (cos, sin)。GPU の sin/cos は精度が実装まかせ（Vulkan は ±2^-11 まで許す）なので、象限を畳んで多項式で求める。
fn cos_sin_turns(turns: f32) -> vec2<f32> {
    let u = turns * 4.0;
    let q = floor(u + 0.5);
    let f = (u - q) * 1.5707963267948966;
    let f2 = f * f;
    let s = f + f * f2 * (-1.6666654611e-1 + f2 * (8.3321608736e-3 + f2 * -1.9515295891e-4));
    let c = 1.0 - 0.5 * f2 + f2 * f2 * (4.166664568298827e-2 + f2 * (-1.388731625493765e-3 + f2 * 2.443315711809948e-5));
    let k = u32(q) & 3u;
    if (k == 0u) { return vec2<f32>(c, s); }
    if (k == 1u) { return vec2<f32>(-s, c); }
    if (k == 2u) { return vec2<f32>(-c, -s); }
    return vec2<f32>(s, -c);
}
struct RayOut { ao: f32, th: f32, bent: vec3<f32>, rays: u32 }
struct Basis { t: vec3<f32>, q: vec3<f32>, n: vec3<f32>, shift: f32, rc: f32, rs: f32 }
fn ray_direction(b: Basis, smp: vec3<f32>, cos2: f32, inward: bool) -> vec3<f32> {
    var u = smp.x + b.shift;
    if (u >= 1.0) { u -= 1.0; }
    let c2 = 1.0 - u * (1.0 - cos2);
    let ct = sqrt(c2);
    var st = 0.0;
    if (c2 < 1.0) { st = sqrt(1.0 - c2); }
    let cp = smp.y * b.rc - smp.z * b.rs;
    let sp = smp.z * b.rc + smp.y * b.rs;
    let lx = st * cp;
    let ly = st * sp;
    if (inward) { return b.t * lx + b.q * ly - b.n * ct; }
    return b.t * lx + b.q * ly + b.n * ct;
}
fn ray_trace(bvh: u32, p: vec3<f32>, n: vec3<f32>, g: vec3<f32>, tri: u32, x: u32, y: u32, sub: u32) -> RayOut {
    let h = hash32((x * 0x9e3779b1u) ^ hash32(y + 0x68e31da4u) ^ (sub * 0x85ebca6bu));
    let shift = f32(h >> 8u) * (1.0 / 16777216.0);
    let h2 = hash32(h ^ 0xb5297a4du);
    let cs = cos_sin_turns(f32(h2 >> 8u) * (1.0 / 16777216.0));
    var sgn = -1.0;
    if (n.z >= 0.0) { sgn = 1.0; }
    let a = -1.0 / (sgn + n.z);
    let bb = n.x * n.y * a;
    let basis = Basis(
        vec3<f32>(1.0 + sgn * n.x * n.x * a, sgn * bb, -sgn * n.x),
        vec3<f32>(bb, sgn + n.y * n.y * a, -n.y),
        n, shift, cs.x, cs.y);
    var res = RayOut(0.0, 0.0, n, 0u);
    if (has(F_AO)) {
        let origin = p + g * P.ray_offset;
        var occlusion = 0.0;
        var sum = vec3<f32>(0.0);
        var valid = 0u;
        for (var k = 0u; k < P.ao_count; k++) {
            let d = ray_direction(basis, mi3(P.ao_dirs_off + k * 4u), P.ao_cos2, false);
            if (dot(d, g) <= 1e-4) { continue; }
            valid += 1u;
            let hit = trace_scene(bvh, origin, d, P.ao_max, tri, has(F_ANYHIT), has(F_BACK));
            if (hit.found) {
                if (has(F_ANYHIT)) { occlusion += 1.0; } else { occlusion += 1.0 - hit.t / P.ao_max; }
            } else {
                sum += d;
            }
        }
        res.rays += valid;
        if (valid > 0u) { res.ao = 1.0 - occlusion / f32(valid); } else { res.ao = 1.0; }
        let l = length(sum);
        if (l > 1e-12) { res.bent = sum / l; }
    }
    if (has(F_TH)) {
        let origin = p - g * P.ray_offset;
        var dist_sum = 0.0;
        var valid = 0u;
        for (var k = 0u; k < P.th_count; k++) {
            let d = ray_direction(basis, mi3(P.th_dirs_off + k * 4u), P.th_cos2, true);
            if (-dot(d, g) <= 1e-4) { continue; }
            valid += 1u;
            let hit = trace_scene(bvh, origin, d, P.th_max, tri, false, false);
            if (hit.found) { dist_sum += min(hit.t, P.th_max); } else { dist_sum += P.th_max; }
        }
        res.rays += valid;
        if (valid > 0u) { res.th = dist_sum / f32(valid) / P.th_max; } else { res.th = 1.0; }
    }
    return res;
}

// NaN・無限大（指数部がすべて 1）なら 1。比較は実装が NaN を無いものとして最適化しうるので、ビットで見る。量子化の前の値に使い、
// 数を counters[3] に足す（NaN は量子化で黙って 0 になるため、そのままでは CPU の結果との差が見えない）。
fn nonfinite(v: f32) -> u32 {
    return select(0u, 1u, (bitcast<u32>(v) & 0x7f800000u) == 0x7f800000u);
}
fn quantize(v: f32) -> u32 {
    if (!(v > 0.0)) { return 0u; }
    if (v >= 1.0) { return 65535u; }
    return u32(v * 65535.0 + 0.5);
}
fn map_offset(kind: u32) -> u32 { return P.map_offsets[kind >> 2u][kind & 3u]; }
fn channels_of(kind: u32) -> u32 {
    if (kind == 0u || kind == 1u || kind == 5u || kind == 7u || kind == 8u) { return 3u; }
    return 1u;
}

@compute @workgroup_size(64)
fn bake(@builtin(global_invocation_id) gid: vec3<u32>) {
    let local = P.local_start + gid.x;
    if (gid.x >= P.count || local >= P.band_texels) { return; }
    let g = P.band_first + local;
    let x = g % P.width;
    let y = g / P.width;
    let aa = P.aa;
    var sums: array<vec3<f32>, 10>;
    for (var k = 0u; k < 10u; k++) { sums[k] = vec3<f32>(0.0); }
    var id_samples: array<u32, 16>;
    var count = 0u;
    var overlap = false;
    var n_rays = 0u;
    var n_projected = 0u;
    var n_missed = 0u;
    var n_nonfinite = 0u;
    let mn = vec3<f32>(P.min_x, P.min_y, P.min_z);
    let scale = vec3<f32>(P.scale_x, P.scale_y, P.scale_z);
    for (var j = 0u; j < aa; j++) {
        for (var i = 0u; i < aa; i++) {
            let ow = raster_sample(x, y, i, j);
            if (ow.r < 0) { continue; }
            let r = u32(ow.r);
            let t = lo_u[r * LU];
            let low = low_point(r, ow.u, ow.v);
            var point = low.p;
            var normal = low.n;
            var face = low.g;
            var st = t;
            var hit = false;
            var rise = 0.0;
            if (has(F_PROJ)) {
                n_projected += 1u;
                let co = r * P.lo_f_stride + P.lo_cage_off;
                var cage = interp3(lo3(co), lo3(co + 3u), lo3(co + 6u), ow.u, ow.v);
                let cl = length(cage);
                if (cl > 1e-9) { cage = cage / cl; } else { cage = low.n; }
                var bvh = P.high_bvh;
                if (has(F_BYNAME)) {
                    bvh = NONE;
                    let group = lo_u[r * LU + 3u];
                    if (group != NONE) { bvh = P.high_bvh + 1u + group; }
                }
                var found = false;
                var h: Hit;
                if (bvh != NONE) {
                    h = trace_scene(bvh, point + cage * P.frontal, -cage, P.frontal + P.rear, NONE, false, false);
                    found = h.found;
                }
                if (found) {
                    hit = true;
                    st = h.tri;
                    rise = P.frontal - h.t;
                    let hs = high_point(st, h.u, h.v);
                    point = hs.p;
                    face = hs.g;
                    normal = hs.n;
                } else {
                    n_missed += 1u;
                }
            }
            sums[0] += normal;
            sums[1] += vec3<f32>(
                select(0.5, (point.x - mn.x) * scale.x, scale.x > 0.0),
                select(0.5, (point.y - mn.y) * scale.y, scale.y > 0.0),
                select(0.5, (point.z - mn.z) * scale.z, scale.z > 0.0));
            if (has(F_CURV)) {
                if (hit) { sums[3].x += 0.5 + 0.5 * curvature_at(1u, point, hi_u[st * 2u + 1u]); }
                else { sums[3].x += 0.5 + 0.5 * curvature_at(0u, point, lo_u[r * LU + 4u]); }
            }
            if (has(F_FRAMES)) {
                if (hit) { sums[5] += tangent_space(r, ow.u, ow.v, normal); }
                else { sums[5] += vec3<f32>(0.0, 0.0, 1.0); }
            }
            sums[6].x += 0.5 + 0.5 * clamp(rise / P.range, -1.0, 1.0);
            if (!has(F_PROJ) || hit) { sums[9].x += 1.0; }
            if (has(F_RAYS)) {
                var bvh = P.low_bvh;
                if (hit) { bvh = P.high_bvh; }
                let ro = ray_trace(bvh, point, normal, face, st, x, y, j * aa + i);
                n_rays += ro.rays;
                sums[2].x += ro.ao;
                sums[4].x += ro.th;
                sums[8] += ro.bent;
            }
            if (has(F_IDS)) {
                var id = lo_u[r * LU + 1u];
                let manual = lo_u[r * LU + 2u];
                if (manual != NONE) { id = manual; }
                else if (!has(F_UVISLAND) && hit) { id = hi_u[st * 2u]; }
                id_samples[count] = id;
            }
            count += 1u;
            overlap = overlap || ow.overlap;
        }
    }
    let base = local * P.out_stride;
    if (count == 0u) {
        for (var k = 0u; k < 10u; k++) {
            let off = map_offset(k);
            if (off == NONE) { continue; }
            for (var a = 0u; a < channels_of(k); a++) { out[base + off + a] = 0u; }
        }
        cov[local] = 0u;
    } else {
        cov[local] = select(1u, 2u, overlap);
        let inv = 1.0 / f32(count);
        for (var k = 0u; k < 10u; k++) {
            let off = map_offset(k);
            if (off == NONE) { continue; }
            if (k == 0u || k == 5u || k == 8u) {
                let l = length(sums[k]);
                n_nonfinite += nonfinite(l);
                var nv = vec3<f32>(0.0, 0.0, 1.0);
                if (l > 1e-12) { nv = sums[k] / l; }
                out[base + off] = quantize(nv.x * 0.5 + 0.5);
                out[base + off + 1u] = quantize(nv.y * 0.5 + 0.5);
                out[base + off + 2u] = quantize(nv.z * 0.5 + 0.5);
            } else if (k == 7u) {
                var best = id_samples[0];
                var best_count = 0u;
                for (var s = 0u; s < count; s++) {
                    var c = 0u;
                    for (var q = 0u; q < count; q++) {
                        if (id_samples[q] == id_samples[s]) { c += 1u; }
                    }
                    if (c > best_count) { best_count = c; best = id_samples[s]; }
                }
                out[base + off] = ((best >> 16u) & 255u) * 257u;
                out[base + off + 1u] = ((best >> 8u) & 255u) * 257u;
                out[base + off + 2u] = (best & 255u) * 257u;
            } else {
                let s = sums[k] * inv;
                n_nonfinite += nonfinite(s.x);
                out[base + off] = quantize(s.x);
                if (channels_of(k) == 3u) {
                    n_nonfinite += nonfinite(s.y) + nonfinite(s.z);
                    out[base + off + 1u] = quantize(s.y);
                    out[base + off + 2u] = quantize(s.z);
                }
            }
        }
    }
    if (n_rays > 0u) { atomicAdd(&counters[0], n_rays); }
    if (n_projected > 0u) { atomicAdd(&counters[1], n_projected); }
    if (n_missed > 0u) { atomicAdd(&counters[2], n_missed); }
    if (n_nonfinite > 0u) { atomicAdd(&counters[3], n_nonfinite); }
}

// ───────── 自己照合（compute と ray query の交差が同じ答えを返すかを、ここで決めた同じレイで確かめる） ─────────
// 三角形の重心の近くから、ランダムな向きに飛ばす（面の上・下、裏面を飛ばす・飛ばさない、最初の当たりで止める・止めない）。
// 結果は out に 8 ワードずつ: [当たったか, t, 三角形, u, v, 0, 0, 0]（`rayquery::CHECK_STRIDE`）。
@compute @workgroup_size(64)
fn selfcheck(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= P.count) { return; }
    var b = P.low_bvh;
    if (b == NONE) { b = P.high_bvh; }
    let base = i * 8u;
    for (var k = 0u; k < 8u; k++) { out[base + k] = 0u; }
    if (b == NONE) { return; }
    let table = P.bvh_table_off + b * 4u;
    let tri_base = idx[table + 1u];
    let tri_count = idx[table + 3u];
    if (tri_count == 0u) { return; }
    let h0 = hash32(i * 0x9e3779b1u + 0x1234567u);
    let tb = (tri_base + h0 % tri_count) * 12u;
    let a = vec3<f32>(bf(bvh_tris[tb]), bf(bvh_tris[tb + 1u]), bf(bvh_tris[tb + 2u]));
    let e1 = vec3<f32>(bf(bvh_tris[tb + 4u]), bf(bvh_tris[tb + 5u]), bf(bvh_tris[tb + 6u]));
    let e2 = vec3<f32>(bf(bvh_tris[tb + 8u]), bf(bvh_tris[tb + 9u]), bf(bvh_tris[tb + 10u]));
    var n = cross(e1, e2);
    let nl = length(n);
    if (nl > 0.0) { n = n / nl; } else { n = vec3<f32>(0.0, 0.0, 1.0); }
    let h1 = hash32(h0 ^ 0x68e31da4u);
    let h2 = hash32(h1 ^ 0xb5297a4du);
    let z = f32(h1 >> 8u) * (2.0 / 16777216.0) - 1.0;
    let cs = cos_sin_turns(f32(h2 >> 8u) * (1.0 / 16777216.0));
    let r = sqrt(max(1.0 - z * z, 0.0));
    let d = vec3<f32>(r * cs.x, r * cs.y, z);
    var origin = a + (e1 + e2) / 3.0 + n * (P.ray_offset * 3.0);
    if ((i & 2u) != 0u) { origin = a + (e1 + e2) / 3.0 - n * (P.ray_offset * 3.0); }
    let limit = max(max(P.ao_max, P.th_max), P.range);
    let hit = trace_scene(b, origin, d, limit, NONE, (i & 8u) != 0u, (i & 4u) != 0u);
    if (hit.found) {
        out[base] = 1u;
        out[base + 1u] = bitcast<u32>(hit.t);
        out[base + 2u] = hit.tri;
        out[base + 3u] = bitcast<u32>(hit.u);
        out[base + 4u] = bitcast<u32>(hit.v);
    }
}
