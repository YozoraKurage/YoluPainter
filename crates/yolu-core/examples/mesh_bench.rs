//! C# MeshGolden の自作モデルと同じ69312面・2048²の計測。各行は 種類,秒,レイ数,中身の SHA-256
//! （テクセルの由来の並び + 16 bit の値のリトルエンディアン。C# の bench も同じ列を出す）。
use sha2::{Digest, Sha256};
use yolu_core::mesh_maps::*;
fn main() {
    let divisions = 76;
    let triangles = 12 * divisions * divisions;
    let mut corners = vec![0.; triangles * 9];
    let mut uvs = vec![0.; triangles * 6];
    let mut slots = vec![0; triangles];
    let order = [0, 1, 2, 0, 2, 3];
    let mut at = 0;
    for face in 0..6 {
        for y in 0..divisions {
            for x in 0..divisions {
                let axis = face / 2;
                let sign = if face % 2 == 0 { 1. } else { -1. };
                for &v in &order {
                    let u = (x + usize::from(v != 0 && v != 3)) as f32 / divisions as f32;
                    let w = (y + usize::from(v >= 2)) as f32 / divisions as f32;
                    let k = at * 3;
                    corners[k + axis] = sign;
                    corners[k + (axis + 1) % 3] = u * 2. - 1.;
                    corners[k + (axis + 2) % 3] = (w * 2. - 1.) * sign;
                    uvs[at * 2] = (face as f32 % 3. + u) / 3.;
                    uvs[at * 2 + 1] = ((face / 3) as f32 + w) / 2.;
                    slots[at / 3] = face as i32;
                    at += 1;
                }
            }
        }
    }
    let input = MeshBakeInput::new(corners, uvs, slots, MeshBakeAttributes::default()).unwrap();
    println!(
        "triangles={},size=2048x2048,samples=8,threads=4,padding=0,antialiasing=1",
        input.triangle_count()
    );
    for kind in MeshMapKind::ALL {
        let settings = MeshBakeSettings {
            width: 2048,
            height: 2048,
            target_slot: -1,
            padding: 0,
            maps: vec![kind],
            ao_samples: 8,
            thickness_samples: 8,
            ..Default::default()
        };
        let budget = MeshBakeBudget {
            max_degree_of_parallelism: 4,
            ..Default::default()
        };
        let result = bake(&input, &settings, &budget, None, None, |_, _| true).unwrap();
        let map = &result.maps[0];
        let mut digest = Sha256::new();
        digest.update(map.coverage());
        for v in map.data() {
            digest.update(v.to_le_bytes());
        }
        println!(
            "{},{:.6},{},{:x}",
            kind.name(),
            result.report.total_seconds,
            result.report.rays,
            digest.finalize()
        );
    }
}
