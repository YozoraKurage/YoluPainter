//! 面の計算（geometry）を Unity 版の SurfaceGeometry（Editor/Preview の原文を本物の UnityEngine.CoreModule と組んだもの）の出力と
//! ビットで照らし合わせる。台本は tests/golden/surface/cases.txt、正解は同じフォルダの index.txt（tools/csharp-golden/run.sh surface で作る）。
//! 形の作り方・乱数・台本の読み方・出力の書き方は tools/csharp-golden/SurfaceGolden.cs と揃えてある（片方を変えたら両方を変える）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use yolu_core::geometry::{
    cube_sphere, demo_cube, region, DabRefusal, Ray, SurfaceBrushBudget, SurfaceGeometry,
    SurfaceHit, SurfaceRegionKind, SurfaceTriangle, SurfaceVisibilityCache,
};
use yolu_core::glam::{Vec2, Vec3};

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn u01(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
    fn s(&mut self) -> f32 {
        (self.u01() * 2.0 - 1.0) as f32
    }
    fn f(&mut self) -> f32 {
        self.u01() as f32
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(14_695_981_039_346_656_037)
    }
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(1_099_511_628_211);
    }
    fn int(&mut self, v: i32) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }
    fn float(&mut self, f: f32) {
        self.int(f.to_bits() as i32);
    }
    fn v3(&mut self, v: Vec3) {
        self.float(v.x);
        self.float(v.y);
        self.float(v.z);
    }
    fn v2(&mut self, v: Vec2) {
        self.float(v.x);
        self.float(v.y);
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn h(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}
fn h3(v: Vec3) -> String {
    format!("{},{},{}", h(v.x), h(v.y), h(v.z))
}
fn h2(v: Vec2) -> String {
    format!("{},{}", h(v.x), h(v.y))
}

fn add_cube(list: &mut Vec<SurfaceTriangle>, r: i32, s: i32, m: i32) {
    let mesh = demo_cube();
    for t in mesh.submeshes[0].indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        list.push(
            SurfaceTriangle::new(
                mesh.positions[a],
                mesh.positions[b],
                mesh.positions[c],
                mesh.uvs[a],
                mesh.uvs[b],
                mesh.uvs[c],
            )
            .with_slot(r, s, m),
        );
    }
}

fn add_sphere(
    list: &mut Vec<SurfaceTriangle>,
    n: i32,
    radius: f32,
    faces: &str,
    r: i32,
    s: i32,
    m: i32,
) {
    let mesh = cube_sphere(n as u32, radius);
    let per_face = 2 * (n * n) as usize;
    for (k, t) in mesh.submeshes[0].indices.chunks_exact(3).enumerate() {
        let face = (k / per_face) as u8;
        if !faces.as_bytes().contains(&(b'0' + face)) {
            continue;
        }
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        list.push(
            SurfaceTriangle::new(
                mesh.positions[a],
                mesh.positions[b],
                mesh.positions[c],
                mesh.uvs[a],
                mesh.uvs[b],
                mesh.uvs[c],
            )
            .with_slot(r, s, m),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn add_plate(
    list: &mut Vec<SurfaceTriangle>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: f32,
    r: i32,
    s: i32,
    m: i32,
) {
    let (a, b, c, d) = (
        Vec3::new(x0, y0, z),
        Vec3::new(x0, y1, z),
        Vec3::new(x1, y0, z),
        Vec3::new(x1, y1, z),
    );
    list.push(
        SurfaceTriangle::new(
            a,
            b,
            c,
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
        )
        .with_slot(r, s, m),
    );
    list.push(
        SurfaceTriangle::new(
            c,
            b,
            d,
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        )
        .with_slot(r, s, m),
    );
}

fn add_soup(
    list: &mut Vec<SurfaceTriangle>,
    seed: u64,
    count: i32,
    scale: f32,
    r: i32,
    s: i32,
    m: i32,
) {
    let mut g = SplitMix(seed);
    for _ in 0..count {
        let (ax, ay, az) = (g.s(), g.s(), g.s());
        let a = Vec3::new(ax, ay, az) * scale;
        let (bx, by, bz) = (g.f(), g.f(), g.f());
        let b = a + Vec3::new(bx * 0.2, by * 0.2, bz * 0.2) * scale;
        let (cx, cy, cz) = (g.f(), g.f(), g.f());
        let c = a + Vec3::new(cx * 0.2, cy * 0.2, cz * 0.2) * scale;
        let (u0, v0, u1, v1, u2, v2) = (g.f(), g.f(), g.f(), g.f(), g.f(), g.f());
        list.push(
            SurfaceTriangle::new(
                a,
                b,
                c,
                Vec2::new(u0, v0),
                Vec2::new(u1, v1),
                Vec2::new(u2, v2),
            )
            .with_slot(r, s, m),
        );
    }
}

struct Case {
    name: String,
    triangles: Vec<SurfaceTriangle>,
    geometry: Option<SurfaceGeometry>,
    budget: SurfaceBrushBudget,
    cache: Option<SurfaceVisibilityCache>,
    outputs: Vec<String>,
    params: Fnv,
}

impl Case {
    fn fl(&mut self, s: &str) -> f32 {
        let v = if s == "inf" {
            f32::INFINITY
        } else {
            s.parse::<f32>().expect("小数")
        };
        self.params.float(v);
        v
    }
    fn int(&mut self, s: &str) -> i32 {
        let v = s.parse::<i32>().expect("整数");
        self.params.int(v);
        v
    }
    fn g(&self) -> &SurfaceGeometry {
        self.geometry.as_ref().expect("build の後")
    }
}

fn hit_text(h: &SurfaceHit) -> String {
    format!(
        "tri={} r={} s={} m={} dist={} pos={} n={} bary={} uv={}",
        h.triangle,
        h.renderer,
        h.material_slot,
        h.material,
        self::h(h.distance),
        h3(h.position),
        h3(h.normal),
        h3(h.barycentric),
        h2(h.uv)
    )
}

fn hash_hit(f: &mut Fnv, h: &SurfaceHit) {
    f.int(h.triangle as i32);
    f.float(h.distance);
    f.v3(h.position);
    f.v3(h.normal);
    f.v3(h.barycentric);
    f.v2(h.uv);
}

fn why(r: Option<DabRefusal>) -> &'static str {
    match r {
        None => "-",
        Some(DabRefusal::SnapshotChanged) => "snapshot",
        Some(DabRefusal::InvalidArguments) => "invalid",
        Some(DabRefusal::BindingMismatch) => "binding",
        Some(DabRefusal::TriangleBudget) => "triangles",
        Some(DabRefusal::PixelBudget) => "pixels",
        Some(DabRefusal::VisibilityBudget) => "visibility",
        Some(DabRefusal::BvhBudget) => "bvh",
        // 投影の塗りだけの断り（古い道は出さない）
        Some(DabRefusal::MemoryBudget) => "memory",
    }
}

fn structure(g: &SurfaceGeometry) -> String {
    let mut adj = Fnv::new();
    for i in 0..g.triangle_count() {
        let n = g.neighbors(i);
        adj.int(n.len() as i32);
        for &x in n {
            adj.int(x as i32);
        }
    }
    let mut bvh = Fnv::new();
    let mut nodes = 0;
    let mut indices = Vec::new();
    g.visit_bvh(
        |b, l, r, start, count| {
            nodes += 1;
            bvh.v3(b.center);
            bvh.v3(b.extents);
            bvh.int(l as i32);
            bvh.int(r as i32);
            bvh.int(start as i32);
            bvh.int(count as i32);
        },
        |i| indices.push(i),
    );
    for i in indices {
        bvh.int(i as i32);
    }
    let b = g.bounds();
    format!(
        "structure tris={} nonmanifold={} center={} extents={} eps={} adj={} nodes={} bvh={}",
        g.triangle_count(),
        g.non_manifold_edge_count(),
        h3(b.center),
        h3(b.extents),
        h(g.visibility_epsilon()),
        adj.hex(),
        nodes,
        bvh.hex()
    )
}

fn step(c: &mut Case, t: &[&str]) {
    match t[0] {
        "clear" => c.triangles.clear(),
        "cube" => {
            let (r, s, m) = (c.int(t[1]), c.int(t[2]), c.int(t[3]));
            add_cube(&mut c.triangles, r, s, m);
        }
        "sphere" => {
            let n = c.int(t[1]);
            let radius = c.fl(t[2]);
            let faces = t.get(6).copied().unwrap_or("012345");
            let (r, s, m) = (c.int(t[3]), c.int(t[4]), c.int(t[5]));
            add_sphere(&mut c.triangles, n, radius, faces, r, s, m);
        }
        "plate" => {
            let (x0, y0, x1, y1, z) = (c.fl(t[1]), c.fl(t[2]), c.fl(t[3]), c.fl(t[4]), c.fl(t[5]));
            let (r, s, m) = (c.int(t[6]), c.int(t[7]), c.int(t[8]));
            add_plate(&mut c.triangles, x0, y0, x1, y1, z, r, s, m);
        }
        "soup" => {
            let seed: u64 = t[1].parse().unwrap();
            let count = c.int(t[2]);
            let scale = c.fl(t[3]);
            let (r, s, m) = (c.int(t[4]), c.int(t[5]), c.int(t[6]));
            add_soup(&mut c.triangles, seed, count, scale, r, s, m);
        }
        "build" => {
            let revision = c.int(t[1]);
            let weld = if t.len() > 2 { c.fl(t[2]) } else { 0.000001 };
            c.geometry = Some(
                SurfaceGeometry::new(c.triangles.clone(), revision as u32, weld).expect("組める"),
            );
        }
        "budget" => {
            c.budget = SurfaceBrushBudget::default();
            for kv in &t[1..] {
                if *kv == "default" {
                    continue;
                }
                let (k, v) = kv.split_once('=').unwrap();
                let v = c.int(v);
                match k {
                    "triangles" => c.budget.max_triangles = v,
                    "pixels" => c.budget.max_candidate_pixels = v,
                    "rays" => c.budget.max_visibility_rays = v,
                    "tests" => c.budget.max_ray_triangle_tests = v,
                    "visits" => c.budget.max_ray_node_visits = v,
                    _ => panic!("予算の鍵: {k}"),
                }
            }
        }
        "cache" => c.cache = (t[1] == "new").then(SurfaceVisibilityCache::new),
        "out" => {
            let s = structure(c.g());
            c.outputs.push(s);
        }
        "ray" => {
            let o = Vec3::new(c.fl(t[1]), c.fl(t[2]), c.fl(t[3]));
            let to = Vec3::new(c.fl(t[4]), c.fl(t[5]), c.fl(t[6]));
            let cull = c.int(t[7]) != 0;
            let max = c.fl(t[8]);
            let text = match c.g().raycast(Ray::new(o, to - o), cull, max) {
                Some(hit) => format!("ray {}", hit_text(&hit)),
                None => "ray none".into(),
            };
            c.outputs.push(text);
        }
        "rays" => {
            let mut r = SplitMix(t[1].parse().unwrap());
            let count = c.int(t[2]);
            let cull = c.int(t[3]) != 0;
            let g = c.g();
            let center = g.bounds().center;
            let radius = 0.0001f32.max(g.bounds().extents.length());
            let (mut f, mut hits) = (Fnv::new(), 0);
            for k in 0..count {
                let (ox, oy, oz, tx, ty, tz) = (r.s(), r.s(), r.s(), r.s(), r.s(), r.s());
                let origin = center + Vec3::new(ox, oy, oz) * (radius * 3.0);
                let target = center + Vec3::new(tx, ty, tz) * (radius * 0.5);
                if let Some(hit) = g.raycast(Ray::new(origin, target - origin), cull, f32::INFINITY)
                {
                    hits += 1;
                    f.int(k);
                    hash_hit(&mut f, &hit);
                }
            }
            c.outputs
                .push(format!("rays n={count} hits={hits} hash={}", f.hex()));
        }
        "dab" => {
            let cam = Vec3::new(c.fl(t[1]), c.fl(t[2]), c.fl(t[3]));
            let to = Vec3::new(c.fl(t[4]), c.fl(t[5]), c.fl(t[6]));
            let radius = c.fl(t[7]);
            let (w, hh) = (c.int(t[8]), c.int(t[9]));
            let hardness = c.fl(t[10]);
            let ignore = t.get(11) == Some(&"ignore");
            let budget = c.budget;
            let g = c.geometry.as_ref().expect("build の後");
            let Some(hit) = g.raycast(Ray::new(cam, to - cam), true, f32::INFINITY) else {
                c.outputs.push("dab nohit".into());
                return;
            };
            let d = g.build_surface_dabs(
                &hit,
                radius,
                w,
                hh,
                cam,
                hardness,
                &budget,
                c.cache.as_mut(),
                ignore,
            );
            let mut f = Fnv::new();
            let mut minimum = f32::INFINITY;
            for p in &d.pixels {
                if p.coverage < minimum {
                    minimum = p.coverage;
                }
                f.int(p.x);
                f.int(p.y);
                f.float(p.coverage);
                f.int(p.triangle.map_or(-1, |t| t as i32));
                f.v3(p.position);
            }
            let cache = c
                .cache
                .as_ref()
                .map(|k| format!(" cachehits={} cached={}", k.hits, k.len()))
                .unwrap_or_default();
            if std::env::var("SURFACE_DUMP").ok().as_deref() == Some(c.name.as_str()) {
                for p in &d.pixels {
                    println!(
                        "{} {} {} {} {}",
                        p.x,
                        p.y,
                        h(p.coverage),
                        p.triangle.map_or(-1, |t| t as i32),
                        h3(p.position)
                    );
                }
            }
            c.outputs.push(format!(
                "dab tri={} pixels={} hash={} cand={} rays={} tests={} visited={} visits={} mincov={} clipped={} why={}{}",
                hit.triangle,
                d.pixels.len(),
                f.hex(),
                d.candidate_pixels,
                d.visibility_rays,
                d.ray_triangle_tests,
                d.visited_triangles,
                d.ray_node_visits,
                h(minimum),
                d.was_clipped() as i32,
                why(d.refusal),
                cache
            ));
        }
        "closest" => {
            let p = Vec3::new(c.fl(t[1]), c.fl(t[2]), c.fl(t[3]));
            let max = c.fl(t[4]);
            let facing = Vec3::new(c.fl(t[5]), c.fl(t[6]), c.fl(t[7]));
            let (visits, material) = (c.int(t[8]), c.int(t[9]));
            let text = match c.g().find_closest_point(p, max, facing, visits, material) {
                Err(_) => "closest exceeded".into(),
                Ok(None) => "closest none".into(),
                Ok(Some(hit)) => format!("closest {}", hit_text(&hit)),
            };
            c.outputs.push(text);
        }
        "region" => {
            let tri = c.int(t[1]);
            let kind = match t[2] {
                "Triangle" => SurfaceRegionKind::Triangle,
                "UvIsland" => SurfaceRegionKind::UvIsland,
                "MeshPart" => SurfaceRegionKind::MeshPart,
                "Material" => SurfaceRegionKind::Material,
                other => panic!("範囲: {other}"),
            };
            let list = region(c.g(), tri as u32, kind);
            let mut f = Fnv::new();
            for &i in &list {
                f.int(i as i32);
            }
            c.outputs
                .push(format!("region n={} hash={}", list.len(), f.hex()));
        }
        other => panic!("知らない命令: {other}"),
    }
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/surface")
}

#[test]
fn surface_geometry_matches_unity_bit_for_bit() {
    let script = std::fs::read_to_string(golden_dir().join("cases.txt")).unwrap();
    let mut cases: Vec<Case> = Vec::new();
    for raw in script.lines() {
        let line = raw.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        if t[0] == "case" {
            cases.push(Case {
                name: t[1].to_string(),
                triangles: Vec::new(),
                geometry: None,
                budget: SurfaceBrushBudget::default(),
                cache: None,
                outputs: Vec::new(),
                params: Fnv::new(),
            });
            continue;
        }
        let c = cases.last_mut().expect("case の前の命令");
        for tok in &t {
            for b in tok.bytes() {
                c.params.byte(b);
            }
        }
        step(c, &t);
    }
    // 正解: case 名前 params=… と out 番号 中身
    let index = std::fs::read_to_string(golden_dir().join("index.txt")).unwrap();
    let mut expected: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut current = String::new();
    for line in index.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("case ") {
            let (name, params) = rest.split_once(" params=").unwrap();
            current = name.to_string();
            expected.insert(current.clone(), (params.to_string(), Vec::new()));
        } else if let Some(rest) = line.strip_prefix("out ") {
            let (_, body) = rest.split_once(' ').unwrap();
            expected.get_mut(&current).unwrap().1.push(body.to_string());
        }
    }
    let mut failures = Vec::new();
    let mut compared = 0;
    for c in &cases {
        let Some((params, outs)) = expected.get(&c.name) else {
            failures.push(format!(
                "{}: 正解に無い（run.sh surface で作り直す）",
                c.name
            ));
            continue;
        };
        if *params != c.params.hex() {
            failures.push(format!(
                "{}: 台本の読み方が違う（params {} ≠ {}）",
                c.name,
                c.params.hex(),
                params
            ));
            continue;
        }
        if outs.len() != c.outputs.len() {
            failures.push(format!(
                "{}: 出力の数 {} ≠ {}",
                c.name,
                c.outputs.len(),
                outs.len()
            ));
        }
        for (i, (got, want)) in c.outputs.iter().zip(outs).enumerate() {
            compared += 1;
            if got != want {
                failures.push(format!("{} out {i}\n  Rust: {got}\n  C#:   {want}", c.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} 件の違い:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(compared > 100, "照らした出力 {compared}");
}
