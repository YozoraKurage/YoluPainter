use yolu_core::mesh_maps::{MeshBakeAttributes, MeshBakeInput, MeshRayBvh};
fn input(tied: bool) -> MeshBakeInput {
    let mut corners = vec![];
    let mut uvs = vec![];
    for t in 0..97 {
        let x = if tied { 0. } else { (t * 37 % 101) as f32 / 8. };
        let y = if tied { 0. } else { (t * 19 % 71) as f32 / 8. };
        let z = if tied { 0. } else { (t * 7 % 29) as f32 / 16. };
        corners.extend([x, y, z, x + 0.5, y, z, x, y + 0.75, z]);
        uvs.extend([0., 0., 1., 0., 0., 1.]);
    }
    MeshBakeInput::new(corners, uvs, vec![0; 97], MeshBakeAttributes::default()).unwrap()
}
#[test]
fn csharp_bvh_order_rays_and_input_hash() {
    for tied in [false, true] {
        let input = input(tied);
        let bvh = MeshRayBvh::new(&input, &(0..97).collect::<Vec<_>>()).unwrap();
        let name = if tied { "tied" } else { "spread" };
        let bytes = std::fs::read(format!(
            "{}/tests/golden/mesh/bvh-{name}.bin",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let mut expected = vec![];
        for s in [input.hash(), input.topology_hash()] {
            expected.push(s.len() as u8);
            expected.extend(s.as_bytes());
        }
        expected.extend((bvh.node_count() as i32).to_le_bytes());
        let mut triangles = vec![];
        bvh.visit(
            |b, f, c| {
                for v in b {
                    expected.extend(v.to_le_bytes());
                }
                expected.extend((f as i32).to_le_bytes());
                expected.extend((c as i32).to_le_bytes());
            },
            |t, v| {
                triangles.extend((t as i32).to_le_bytes());
                for f in v {
                    triangles.extend(f.to_le_bytes());
                }
            },
        );
        expected.extend(triangles);
        for t in 0..97 {
            let x = if tied { 0. } else { (t * 37 % 101) as f64 / 8. };
            let y = if tied { 0. } else { (t * 19 % 71) as f64 / 8. };
            let hit = bvh.trace(
                [x + 0.1, y + 0.1, 5.],
                [0., 0., -1.],
                10.,
                None,
                false,
                false,
            );
            let (d, t, u, v) = hit.map_or((f64::INFINITY, -1, 0., 0.), |h| {
                (h.distance, h.triangle as i32, h.u, h.v)
            });
            expected.extend(d.to_le_bytes());
            expected.extend(t.to_le_bytes());
            expected.extend(u.to_le_bytes());
            expected.extend(v.to_le_bytes());
        }
        assert_eq!(expected.len(), bytes.len());
        let first = expected.iter().zip(&bytes).position(|(a, b)| a != b);
        assert_eq!(first, None, "{name}: 最初の差");
    }
}
#[test]
fn rejects_invalid_input() {
    for (corners, uvs, slots) in [
        (vec![], vec![], vec![]),
        (vec![0.; 9], vec![0.; 6], vec![0]),
        (vec![f32::NAN; 9], vec![0.; 6], vec![0]),
        (vec![0.; 8], vec![0.; 6], vec![0]),
    ] {
        assert!(MeshBakeInput::new(corners, uvs, slots, MeshBakeAttributes::default()).is_err());
    }
    assert!(MeshRayBvh::new(&input(false), &[97]).is_err());
}

mod support;
/// C# BvhDump と同じ並びの期待値: 入力の指紋・節・三角形・レイごと 7 つの探索の設定の結果。
fn expected_dump(input: &MeshBakeInput, rays: &[[f64; 6]]) -> Vec<u8> {
    let n = input.triangle_count();
    let bvh = MeshRayBvh::new(input, &(0..n).collect::<Vec<_>>()).unwrap();
    let mut out = vec![];
    for s in [input.hash(), input.topology_hash()] {
        out.push(s.len() as u8);
        out.extend(s.as_bytes());
    }
    out.extend((bvh.node_count() as i32).to_le_bytes());
    let mut triangles = vec![];
    bvh.visit(
        |b, f, c| {
            for v in b {
                out.extend(v.to_le_bytes());
            }
            out.extend((f as i32).to_le_bytes());
            out.extend((c as i32).to_le_bytes());
        },
        |t, v| {
            triangles.extend((t as i32).to_le_bytes());
            for f in v {
                triangles.extend(f.to_le_bytes());
            }
        },
    );
    out.extend(triangles);
    // (打ち切り, 裏面を飛ばす, 自分の番号を無視する)
    let modes = [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (false, false, true),
        (false, true, true),
        (true, false, true),
    ];
    for (index, r) in rays.iter().enumerate() {
        for (any, back, skip) in modes {
            let hit = bvh.trace(
                [r[0], r[1], r[2]],
                [r[3], r[4], r[5]],
                12.,
                skip.then_some(index % n),
                any,
                back,
            );
            let (d, t, u, v) = hit.map_or((f64::INFINITY, -1, 0., 0.), |h| {
                (h.distance, h.triangle as i32, h.u, h.v)
            });
            out.extend(d.to_le_bytes());
            out.extend(t.to_le_bytes());
            out.extend(u.to_le_bytes());
            out.extend(v.to_le_bytes());
        }
    }
    out
}
fn compare(name: &str, expected: Vec<u8>) {
    let bytes = std::fs::read(format!(
        "{}/tests/golden/mesh/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert_eq!(expected.len(), bytes.len(), "{name}: 長さ");
    assert_eq!(
        expected.iter().zip(&bytes).position(|(a, b)| a != b),
        None,
        "{name}: 最初の差"
    );
}
#[test]
fn csharp_bvh_modes_any_hit_backfaces_and_ignored_triangle() {
    let n = 97;
    let mut corners = vec![0.; n * 9];
    let mut uvs = vec![0.; n * 6];
    for t in 0..n {
        let x = (t * 37 % 101) as f32 / 8.;
        let y = (t * 19 % 71) as f32 / 8.;
        let z = (t * 7 % 29) as f32 / 16.;
        let flipped = t % 3 == 0;
        corners[t * 9..t * 9 + 9].copy_from_slice(&[
            x,
            y,
            z,
            if flipped { x } else { x + 0.5 },
            if flipped { y + 0.75 } else { y },
            z,
            if flipped { x + 0.5 } else { x },
            if flipped { y } else { y + 0.75 },
            z,
        ]);
        uvs[t * 6 + 2] = 1.;
        uvs[t * 6 + 5] = 1.;
    }
    let input = MeshBakeInput::new(
        corners.clone(),
        uvs,
        vec![0; n],
        MeshBakeAttributes::default(),
    )
    .unwrap();
    let mut rays = vec![];
    for t in 0..n {
        let ox = corners[t * 9] as f64 + 0.1;
        let oy = corners[t * 9 + 1] as f64 + 0.1;
        rays.push([ox, oy, 5., 0., 0., -1.]);
        rays.push([ox, oy, -5., 0., 0., 1.]);
    }
    compare("bvh-modes.bin", expected_dump(&input, &rays));
}
#[test]
fn csharp_bvh_irregular_coordinates_and_modes() {
    let g = support::rough(12, 0.2, 1, false);
    let input = support::input(&g, support::Extra::default());
    let mut rays = vec![];
    for t in (0..g.slots.len()).step_by(7) {
        let k = t * 9;
        let c = |a: usize| {
            (g.corners[k + a] as f64 + g.corners[k + 3 + a] as f64 + g.corners[k + 6 + a] as f64)
                / 3.
        };
        rays.push([c(0), c(1), 5., 0., 0., -1.]);
        rays.push([c(0), c(1), -5., 0., 0., 1.]);
        rays.push([-5., c(1), c(2), 1., 0., 0.]);
    }
    compare("bvh-rough.bin", expected_dump(&input, &rays));
}
