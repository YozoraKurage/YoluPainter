//! `MeshBakePlan`（別の実行場所へ渡す入口）の試験。GPU が無くても回る: 実行場所が読む平らな配列が、CPU の BVH・曲率と同じ値・並びで
//! あること、CPU の行ごとの計算を通した結果が `bake` と全バイト一致すること、`finish` が合わない画素を断ることを確かめる。
use super::*;
use crate::mesh_maps::{
    bake, bake::cpu_rows, bvh::MeshRayBvh, curvature::Curvature, MeshBakeAttributes,
};

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}
fn unit(seed: u32) -> f32 {
    (hash(seed) >> 8) as f32 / 16_777_216.0
}

/// 6 面の分割立方体（頂点法線つき）。UV は 3×2 の格子に面ごとの島で、テクセルの中心が対角線に乗らないようにずらす。
/// `bulge` は球に近づける割合、`noise` は頂点ごとの凹凸（同じ位置の頂点は同じ凹凸）。
fn cube(divisions: usize, bulge: f32, noise: f32, seed: u32) -> MeshBakeInput {
    let triangles = 12 * divisions * divisions;
    let (mut corners, mut uvs, mut slots) = (
        vec![0f32; triangles * 9],
        vec![0f32; triangles * 6],
        vec![0; triangles],
    );
    let mut at = 0;
    for face in 0..6 {
        for y in 0..divisions {
            for x in 0..divisions {
                let axis = face / 2;
                let sign = if face % 2 == 0 { 1. } else { -1. };
                for v in [0, 1, 2, 0, 2, 3] {
                    let u = (x + usize::from(v != 0 && v != 3)) as f32 / divisions as f32;
                    let w = (y + usize::from(v >= 2)) as f32 / divisions as f32;
                    let mut p = [0f32; 3];
                    p[axis] = sign;
                    p[(axis + 1) % 3] = u * 2. - 1.;
                    p[(axis + 2) % 3] = (w * 2. - 1.) * sign;
                    let key = ((p[0] * 1024.) as i32 as u32)
                        ^ ((p[1] * 1024.) as i32 as u32).wrapping_mul(7919)
                        ^ ((p[2] * 1024.) as i32 as u32).wrapping_mul(104_729)
                        ^ seed;
                    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    let r = 1. - bulge + bulge / len + noise * (unit(key) - 0.5);
                    corners[at * 3..at * 3 + 3].copy_from_slice(&[p[0] * r, p[1] * r, p[2] * r]);
                    let (su, sv) = ((face as f32 % 3. + u) / 3., ((face / 3) as f32 + w) / 2.);
                    uvs[at * 2] = su * 0.9731 + 0.0127;
                    uvs[at * 2 + 1] = sv * 0.9731 + 0.0127;
                    slots[at / 3] = face as i32;
                    at += 1;
                }
            }
        }
    }
    let mut normals = vec![0f32; triangles * 9];
    for i in 0..triangles * 3 {
        let p = &corners[i * 3..i * 3 + 3];
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
        normals[i * 3..i * 3 + 3].copy_from_slice(&[p[0] / l, p[1] / l, p[2] / l]);
    }
    MeshBakeInput::new(
        corners,
        uvs,
        slots,
        MeshBakeAttributes {
            normals: Some(normals),
            ..Default::default()
        },
    )
    .unwrap()
}

fn settings(maps: &[MeshMapKind]) -> MeshBakeSettings {
    MeshBakeSettings {
        width: 40,
        height: 36,
        target_slot: -1,
        padding: 4,
        antialiasing: 2,
        maps: maps.to_vec(),
        ao_samples: 8,
        thickness_samples: 8,
        curvature_radius: 0.15,
        reference_frontal: 0.1,
        reference_rear: 0.1,
        ..Default::default()
    }
}

fn ready<'a>(
    input: &'a MeshBakeInput,
    s: &'a MeshBakeSettings,
    reference: Option<&'a MeshBakeInput>,
) -> MeshBakePlan<'a> {
    match MeshBakePlan::prepare(
        input,
        s,
        &MeshBakeBudget::default(),
        None,
        reference,
        &mut |_, _| true,
    )
    .unwrap()
    {
        MeshBakePlanOutcome::Ready(plan) => plan,
        MeshBakePlanOutcome::Stopped(_) => panic!("止まらないはず"),
    }
}
/// 準備のあと、CPU の行ごとの計算（`bake` が使うもの）で焼いた画素。
fn cpu_raw(plan: &mut MeshBakePlan<'_>) -> MeshBakeRaw {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(plan.prep.threads)
        .build()
        .unwrap();
    match cpu_rows(&mut plan.prep, &plan.control, &pool, &mut |_, _| true) {
        Ok(raw) => raw,
        Err(_) => panic!("止まらないはず"),
    }
}

fn same_result(a: &MeshBakeResult, b: &MeshBakeResult) {
    assert_eq!(a.status, b.status);
    assert_eq!(a.maps.len(), b.maps.len());
    for (x, y) in a.maps.iter().zip(&b.maps) {
        assert_eq!(x.provenance(), y.provenance());
        assert!(x.data() == y.data(), "{:?} の値", x.kind());
        assert!(x.coverage() == y.coverage(), "{:?} の覆い", x.kind());
    }
    let (r, q) = (&a.report, &b.report);
    assert_eq!(
        (r.rays, r.projected_samples, r.missed_samples),
        (q.rays, q.projected_samples, q.missed_samples)
    );
    assert_eq!(
        (
            r.covered_texels,
            r.overlap_texels,
            r.padded_texels,
            r.empty_texels
        ),
        (
            q.covered_texels,
            q.overlap_texels,
            q.padded_texels,
            q.empty_texels
        )
    );
}

#[test]
fn a_plan_finished_from_the_cpu_rows_is_the_same_as_bake_byte_for_byte() {
    let low = cube(4, 0.6, 0.04, 21);
    let high = cube(10, 0.6, 0.05, 23);
    for (reference, label) in [(None, "自己ベイク"), (Some(&high), "高ポリからの投影")]
    {
        let s = settings(&MeshMapKind::ALL);
        let expect = bake(
            &low,
            &s,
            &MeshBakeBudget::default(),
            None,
            reference,
            |_, _| true,
        )
        .unwrap();
        let mut plan = ready(&low, &s, reference);
        let raw = cpu_raw(&mut plan);
        assert_eq!(raw.outputs.len(), s.maps.len());
        let got = plan.finish(raw, &mut |_, _| true).unwrap();
        assert_eq!(got.status, MeshBakeStatus::Completed, "{label}");
        same_result(&expect, &got);
    }
}

#[test]
fn finish_refuses_pixels_that_do_not_match_the_settings() {
    let low = cube(3, 0.6, 0.04, 5);
    let s = settings(&[MeshMapKind::WorldNormal, MeshMapKind::Opacity]);
    let texels = 40 * 36;
    let refusal = "焼いた画素の長さまたは由来が設定と合いません";
    type Spoil = fn(&mut MeshBakeRaw);
    let broken: [(&str, Spoil); 6] = [
        ("覆いが短い", |r| {
            r.coverage.pop();
        }),
        ("覆いが長い", |r| r.coverage.push(0)),
        ("覆いが 3（0〜2 の外）", |r| r.coverage[7] = 3),
        ("マップが足りない", |r| {
            r.outputs.pop();
        }),
        ("マップが多い", |r| r.outputs.push(vec![0; 40 * 36])),
        ("チャンネルが足りない", |r| {
            r.outputs[0].pop();
        }),
    ];
    for (what, spoil) in broken {
        let mut plan = ready(&low, &s, None);
        let mut raw = cpu_raw(&mut plan);
        spoil(&mut raw);
        let error = plan
            .finish(raw, &mut |_, _| true)
            .err()
            .unwrap_or_else(|| panic!("{what}: 断るはず"));
        assert_eq!(error.to_string(), refusal, "{what}");
    }
    // 合っていれば通る（上の拒否が、ほかの理由でないことの対）
    let mut plan = ready(&low, &s, None);
    let raw = cpu_raw(&mut plan);
    assert_eq!(raw.coverage.len(), texels);
    assert!(plan.finish(raw, &mut |_, _| true).is_ok());
}

/// 平らな BVH が CPU の BVH（`visit`）と同じ節・面の並びと値であること。
fn assert_flat_bvh_matches(flat: &FlatBvh, cpu: &MeshRayBvh) {
    let mut nodes = 0;
    cpu.visit(
        |bounds, first, count| {
            assert_eq!(
                &flat.bounds[nodes * 6..nodes * 6 + 6],
                &bounds,
                "節 {nodes}"
            );
            assert_eq!(
                (flat.first[nodes] as usize, flat.count[nodes] as usize),
                (first, count)
            );
            nodes += 1;
        },
        |_, _| {},
    );
    let mut triangles = 0;
    cpu.visit(
        |_, _, _| {},
        |original, values| {
            assert_eq!(
                &flat.tris[triangles * 10..triangles * 10 + 10],
                &values,
                "面 {triangles}"
            );
            assert_eq!(flat.original[triangles] as usize, original);
            triangles += 1;
        },
    );
    assert_eq!(
        (flat.node_count(), flat.triangle_count()),
        (nodes, triangles)
    );
    assert!(nodes > 1 && triangles > 10, "自明でない BVH");
    assert_eq!(flat.bounds.len(), nodes * 6);
    assert_eq!(flat.tris.len(), triangles * 10);
}

/// 平らな曲率の評価（`Curvature::evaluate` と同じ式を、平らな配列の上で。セルは昇順の表を二分探索で引く）。
fn evaluate_flat(f: &FlatCurvature, p: [f64; 3], component: u32) -> f64 {
    if f.cells.is_empty() {
        return 0.;
    }
    let reach = 1.5 * f.radius;
    let r2 = f.radius * f.radius;
    let inv = 1. / r2;
    let lo: [i64; 3] =
        std::array::from_fn(|a| (((p[a] - reach - f.min[a]) / f.cell).floor() as i64).max(0));
    let hi: [i64; 3] = std::array::from_fn(|a| ((p[a] + reach - f.min[a]) / f.cell).floor() as i64);
    let mut sum = 0.;
    for x in lo[0]..=hi[0] {
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                let key = [x as u32, y as u32, z as u32];
                let Ok(at) = f.cells.binary_search_by_key(&key, |c| [c[0], c[1], c[2]]) else {
                    continue;
                };
                let (start, n) = (f.cells[at][3] as usize, f.cells[at][4] as usize);
                for s in start..start + n {
                    if f.segment_component[s] != component {
                        continue;
                    }
                    let g = &f.segments[s * 7..s * 7 + 7];
                    let a = [f64::from(g[0]), f64::from(g[1]), f64::from(g[2])];
                    let b = [f64::from(g[3]), f64::from(g[4]), f64::from(g[5])];
                    let phi = f64::from(g[6]);
                    let m = [a[0] - p[0], a[1] - p[1], a[2] - p[2]];
                    let e = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
                    let (ee, me, mm) = (dot(e, e), dot(m, e), dot(m, m));
                    if ee <= 0. {
                        continue;
                    }
                    let disc = me * me - ee * (mm - r2);
                    if disc <= 0. {
                        continue;
                    }
                    let root = disc.sqrt();
                    let t0 = ((-me - root) / ee).max(0.);
                    let t1 = ((-me + root) / ee).min(1.);
                    if t1 <= t0 {
                        continue;
                    }
                    let (qa, qb, qc) = (1. - mm * inv, -2. * me * inv, -ee * inv);
                    let anti = |t: f64| {
                        let (t2, t3) = (t * t, t * t * t);
                        qa * qa * t
                            + qa * qb * t2
                            + (qb * qb + 2. * qa * qc) * t3 / 3.
                            + qb * qc * t2 * t2 / 2.
                            + qc * qc * t3 * t2 / 5.
                    };
                    sum += phi * (anti(t1) - anti(t0)) * ee.sqrt();
                }
            }
        }
    }
    sum / f.norm
}

fn assert_flat_curvature_matches(
    flat: &FlatCurvature,
    cpu: &Curvature,
    min: [f64; 3],
    max: [f64; 3],
) {
    assert!(flat.segments.len() == cpu.segment_count() * 7 && !flat.segments.is_empty());
    assert_eq!(flat.segment_component.len(), cpu.segment_count());
    assert_eq!(flat.components.len(), cpu.components.len());
    assert!(
        flat.cells.windows(2).all(|w| w[0][..3] < w[1][..3]),
        "セルは (x, y, z) の昇順で重複がない"
    );
    // 全部のセルの本数の合計 = 線分の数（どの線分もどれかのセルに入っている）
    assert_eq!(
        flat.cells.iter().map(|c| c[4] as usize).sum::<usize>(),
        cpu.segment_count()
    );
    // 評価: 面の上・外・最小の角の手前（lo が 0 に切り上がる）の点で、CPU の評価と同じ
    let span = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let mut worst = 0f64;
    let mut nonzero = 0;
    for i in 0..400u32 {
        let r = |k: u32| f64::from(unit(i * 3 + k));
        let p = [
            min[0] - 0.3 * span[0] + 1.6 * span[0] * r(0),
            min[1] - 0.3 * span[1] + 1.6 * span[1] * r(1),
            min[2] - 0.3 * span[2] + 1.6 * span[2] * r(2),
        ];
        for component in [
            flat.components[0],
            flat.components[flat.components.len() / 2],
            987_654,
        ] {
            let a = cpu.evaluate(p, component as usize);
            let b = evaluate_flat(flat, p, component);
            worst = worst.max((a - b).abs());
            nonzero += usize::from(a.abs() > 1e-3);
        }
    }
    assert!(worst < 1e-4, "CPU の評価との最大差 {worst}");
    assert!(nonzero > 50, "評価が 0 ばかりでない（{nonzero}）");
}

#[test]
fn the_scene_arrays_are_one_entry_per_receiver_and_the_flat_structures_match_the_cpu() {
    let low = cube(4, 0.6, 0.04, 21);
    let high = cube(9, 0.6, 0.05, 23);
    let s = settings(&MeshMapKind::ALL);
    let plan = ready(&low, &s, Some(&high));
    let scene = plan.scene();
    let n = scene.receivers.len();
    assert!(n > 50);
    assert!(
        scene.receivers.windows(2).all(|w| w[0] < w[1]) && scene.receivers[n - 1] < 192,
        "受け手は元の三角形の番号の昇順"
    );
    assert_eq!((scene.width, scene.height, scene.antialiasing), (40, 36, 2));
    assert_eq!(scene.maps, s.maps);
    assert_eq!(scene.corners.len(), n * 9);
    assert_eq!(scene.normals.as_ref().map(Vec::len), Some(n * 9));
    assert_eq!(scene.face.len(), n * 3);
    assert_eq!(scene.raster_affine.len(), n * 6);
    assert_eq!(scene.raster_bounds.len(), n * 4);
    assert_eq!(scene.frames.as_ref().map(Vec::len), Some(n * 27));
    assert_eq!(scene.low_component.len(), n);
    // 受け手の角・面の法線は、元の三角形の番号で引いた値
    for (r, &t) in scene.receivers.iter().enumerate() {
        let t = t as usize;
        assert_eq!(
            scene.corners[r * 9..r * 9 + 9],
            low.corners[t * 9..t * 9 + 9]
        );
        assert_eq!(
            scene.normals.as_ref().unwrap()[r * 9..r * 9 + 9],
            low.normals.as_ref().unwrap()[t * 9..t * 9 + 9]
        );
    }
    assert_eq!((scene.rays.ao.len(), scene.rays.thickness.len()), (8, 8));
    assert!(scene.rays.want_ao && scene.rays.want_thickness);
    let projection = scene.projection.as_ref().expect("高ポリを渡した");
    assert_eq!(projection.cage.len(), n * 9);
    assert_eq!(projection.target.len(), n);
    assert!(projection.groups.is_empty() && !projection.by_name);
    assert_eq!(projection.high_face.len(), 3 * high.triangle_count());
    assert_eq!(projection.high_corners.len(), 9 * high.triangle_count());
    let ids = scene.ids.as_ref().expect("ID のマップがある");
    assert_eq!((ids.low.len(), ids.manual.len()), (n, n));
    assert_eq!(ids.high.len(), high.triangle_count());
    assert!(ids.manual.iter().all(|m| *m == SCENE_NONE));

    // 平らな BVH・曲率は CPU のものと同じ値・並び
    assert_flat_bvh_matches(
        scene.low_bvh.as_ref().expect("レイのマップがある"),
        &plan.prep.low_bvh,
    );
    assert_flat_bvh_matches(&projection.bvh, plan.prep.high_bvh.as_ref().unwrap());
    let curvature = scene.curvature.as_ref().expect("曲率のマップがある");
    assert_flat_curvature_matches(
        curvature,
        plan.prep.curvature.as_ref().unwrap(),
        low.min,
        low.max,
    );
    assert_flat_curvature_matches(
        projection.curvature.as_ref().expect("高ポリの曲率"),
        plan.prep.high_curvature.as_ref().unwrap(),
        high.min,
        high.max,
    );
    // 受け手ごとの連結成分は、三角形ごとの成分を受け手の並びで引いたもの
    for (r, &t) in scene.receivers.iter().enumerate() {
        assert_eq!(scene.low_component[r], curvature.components[t as usize]);
    }
}

#[test]
fn the_scene_leaves_out_what_the_maps_do_not_need() {
    let low = cube(3, 0.6, 0.04, 5);
    let s = settings(&[MeshMapKind::WorldNormal, MeshMapKind::Position]);
    let plan = ready(&low, &s, None);
    let scene = plan.scene();
    assert!(scene.low_bvh.is_none() && scene.curvature.is_none());
    assert!(scene.frames.is_none() && scene.ids.is_none() && scene.projection.is_none());
    assert!(scene.low_component.is_empty());
}

#[test]
fn flattening_drops_cells_with_a_negative_index_that_no_evaluation_can_reach() {
    let low = cube(4, 0.6, 0.04, 21);
    let s = settings(&[MeshMapKind::Curvature]);
    let mut plan = ready(&low, &s, None);
    let cpu = plan.prep.curvature.as_mut().unwrap();
    let before = cpu.flatten().cells.len();
    // 最小の角の手前（番号 -1）に余計なセルを足す。評価は 0 以上の番号しか引かないので、落としても結果は変わらない
    cpu.add_cell_for_test([-1, 0, 0]);
    let flat = cpu.flatten();
    assert_eq!(flat.cells.len(), before, "負の番号のセルは落ちる");
    assert!(flat.cells.windows(2).all(|w| w[0][..3] < w[1][..3]));
    let (min, max) = (low.min, low.max);
    assert_flat_curvature_matches(&flat, plan.prep.curvature.as_ref().unwrap(), min, max);
}
