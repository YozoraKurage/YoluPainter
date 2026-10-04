// 頂点の並びを入れ替えた加速構造では、ハードウェアの重心座標の x・y が compute の道の v・u に当たる。
fn barycentrics(b: vec2<f32>) -> vec2<f32> {
    if (has(F_RQ_FLIP)) { return vec2<f32>(b.y, b.x); }
    return b;
}
// ハードウェアの ray query（wgpu の実験機能）で BVH の代わりに加速構造をたどる。`trace_scene` の約束は compute の道と同じだが、
// 次の 2 点は同じにできない:
//  - `ignore`（自分の三角形を飛ばす）: 候補の交差を選べないので、ここでは見ない。レイの始点を面の外へ ray_offset だけ持ち上げて
//    いるので、自分の面には当たらない（compute の道の `ignore` も、実際にはこの余裕が効いている）。
//  - 裏面を飛ばす: ハードウェアの `RAY_FLAG_CULL_BACK_FACING`。表裏の規約（Vulkan の既定は時計回りが表）が compute の `det < 0`
//    を飛ばす向きと同じかは、裏面を飛ばす設定のときだけ自己照合で確かめる。逆なら加速構造の頂点の並び（辺 1 と辺 2）を入れ替えて
//    作り直し、そのとき重心座標の u と v が入れ替わるので `F_RQ_FLIP` で戻す。
// 三角形は不透明（OPAQUE）の幾何なので、候補の列挙はなく、最も近い当たりが決まる。
fn trace_scene(b: u32, o: vec3<f32>, d: vec3<f32>, limit: f32, ignore: u32, any_hit: bool, back: bool) -> Hit {
    var h = Hit(false, limit, 0u, 0.0, 0.0);
    let table = P.bvh_table_off + b * 4u;
    if (idx[table + 2u] == 0u) { return h; }
    let tri_base = idx[table + 1u];
    var flags: u32 = RAY_FLAG_NONE;
    if (any_hit) { flags = flags | RAY_FLAG_TERMINATE_ON_FIRST_HIT; }
    if (back) { flags = flags | RAY_FLAG_CULL_BACK_FACING; }
    if (b == P.low_bvh) {
        var rq: ray_query;
        rayQueryInitialize(&rq, tlas_low, RayDesc(flags, 0xFFu, 0.0, limit, o, d));
        while (rayQueryProceed(&rq)) {}
        let hit = rayQueryGetCommittedIntersection(&rq);
        if (hit.kind != RAY_QUERY_INTERSECTION_NONE) {
            let bc = barycentrics(hit.barycentrics);
            h = Hit(true, hit.t, bvh_tris[(tri_base + hit.primitive_index) * 12u + 3u], bc.x, bc.y);
        }
    } else {
        var rq: ray_query;
        rayQueryInitialize(&rq, tlas_high, RayDesc(flags, 0xFFu, 0.0, limit, o, d));
        while (rayQueryProceed(&rq)) {}
        let hit = rayQueryGetCommittedIntersection(&rq);
        if (hit.kind != RAY_QUERY_INTERSECTION_NONE) {
            let bc = barycentrics(hit.barycentrics);
            h = Hit(true, hit.t, bvh_tris[(tri_base + hit.primitive_index) * 12u + 3u], bc.x, bc.y);
        }
    }
    return h;
}
