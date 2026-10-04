// BVH を compute でたどる交差（core の MeshRayBvh::trace と同じ手順。節の箱は子 2 つを見て近い方から、遠い方は入る距離と一緒に積む）。
// 箱・三角形の値は f32 に丸めた core の BVH そのもの。平行の許容幅は f32 の丸めで潰れる 1e-9 ではなく 1e-6 にする。
const STACK: u32 = 96u;
// 三角形の辺の判定の余裕（重心座標）。core は f64 で余裕なしだが、f32 では 2 つの三角形が共有する辺の上のレイが両方の判定の
// 丸めで漏れる（すき間）ことがあるので、辺の外へ 1e-5 だけ広げる。広げた分で別の三角形に当たっても、手前の方が採られる。
const EDGE: f32 = 1e-5;
fn box_enter(nb: u32, inv: vec3<f32>, oi: vec3<f32>, limit: f32) -> f32 {
    let mn = vec3<f32>(bf(bvh_nodes[nb]), bf(bvh_nodes[nb + 1u]), bf(bvh_nodes[nb + 2u]));
    let mx = vec3<f32>(bf(bvh_nodes[nb + 4u]), bf(bvh_nodes[nb + 5u]), bf(bvh_nodes[nb + 6u]));
    let positive = inv >= vec3<f32>(0.0);
    let a = select(mx, mn, positive) * inv - oi;
    let c = select(mn, mx, positive) * inv - oi;
    let lo = max(max(max(a.x, a.y), a.z), 0.0);
    let hi = min(min(min(c.x, c.y), c.z), limit);
    if (lo <= hi) { return lo; }
    return -1.0;
}
fn trace_scene(b: u32, o: vec3<f32>, d_in: vec3<f32>, limit: f32, ignore: u32, any_hit: bool, back: bool) -> Hit {
    var h = Hit(false, limit, 0u, 0.0, 0.0);
    let table = P.bvh_table_off + b * 4u;
    let node_base = idx[table];
    let tri_base = idx[table + 1u];
    if (idx[table + 2u] == 0u) { return h; }
    var d = d_in;
    if (d.x == 0.0) { d.x = 1e-30; }
    if (d.y == 0.0) { d.y = 1e-30; }
    if (d.z == 0.0) { d.z = 1e-30; }
    let inv = vec3<f32>(1.0) / d;
    let oi = o * inv;
    var best = limit;
    if (box_enter(node_base * 8u, inv, oi, best) < 0.0) { return h; }
    var stack_node: array<u32, 96>;
    var stack_t: array<f32, 96>;
    var sp = 0u;
    var at = 0u;
    loop {
        let nb = (node_base + at) * 8u;
        let count = bvh_nodes[nb + 7u];
        let first = bvh_nodes[nb + 3u];
        if (count > 0u) {
            for (var k = first; k < first + count; k++) {
                let tb = (tri_base + k) * 12u;
                let e1 = vec3<f32>(bf(bvh_tris[tb + 4u]), bf(bvh_tris[tb + 5u]), bf(bvh_tris[tb + 6u]));
                let e2 = vec3<f32>(bf(bvh_tris[tb + 8u]), bf(bvh_tris[tb + 9u]), bf(bvh_tris[tb + 10u]));
                let eps = bf(bvh_tris[tb + 7u]) * 1000.0;
                let q0 = cross(d, e2);
                let det = dot(e1, q0);
                if ((det > -eps && det < eps) || (back && det < 0.0)) { continue; }
                let rcp = 1.0 / det;
                let s = o - vec3<f32>(bf(bvh_tris[tb]), bf(bvh_tris[tb + 1u]), bf(bvh_tris[tb + 2u]));
                let u = dot(s, q0) * rcp;
                if (u < -EDGE || u > 1.0 + EDGE) { continue; }
                let q = cross(s, e1);
                let v = dot(d, q) * rcp;
                if (v < -EDGE || u + v > 1.0 + EDGE) { continue; }
                let dist = dot(e2, q) * rcp;
                let original = bvh_tris[tb + 3u];
                if (dist <= 0.0 || dist >= best || original == ignore) { continue; }
                best = dist;
                h = Hit(true, dist, original, clamp(u, 0.0, 1.0), clamp(v, 0.0, 1.0));
                if (any_hit) { return h; }
            }
        } else {
            let l = first;
            let ta = box_enter((node_base + l) * 8u, inv, oi, best);
            let tr = box_enter((node_base + l + 1u) * 8u, inv, oi, best);
            if (ta >= 0.0 && tr >= 0.0) {
                if (sp < STACK) {
                    if (ta <= tr) { stack_node[sp] = l + 1u; stack_t[sp] = tr; } else { stack_node[sp] = l; stack_t[sp] = ta; }
                    sp += 1u;
                }
                if (ta <= tr) { at = l; } else { at = l + 1u; }
                continue;
            }
            if (ta >= 0.0) { at = l; continue; }
            if (tr >= 0.0) { at = l + 1u; continue; }
        }
        loop {
            if (sp == 0u) { return h; }
            sp -= 1u;
            if (stack_t[sp] < best) { at = stack_node[sp]; break; }
        }
    }
    return h;
}
