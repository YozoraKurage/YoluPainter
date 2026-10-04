//! C# の MeshGoldenRough.cs と同じ入力を作る。整数のハッシュと 1 文 1 演算（f32 に入れる）だけで、C# とビット列が一致する。
#![allow(dead_code)]
use yolu_core::mesh_maps::*;

pub fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0f32 / 16777216.0f32)
}
fn signed(h: u32) -> f32 {
    let mut d = unit(h);
    d *= 2.0;
    d -= 1.0;
    d
}
pub struct Gen {
    pub corners: Vec<f32>,
    pub uvs: Vec<f32>,
    pub slots: Vec<i32>,
}
fn lattice(x: i32, y: i32, z: i32, n: i32, amp: f32, seed: u32, p: &mut [f32]) {
    let key = ((x * 257 + y) * 257 + z) as u32;
    let mut s = amp * signed(mix(key.wrapping_mul(0x9e3779b1) ^ seed));
    s += 1.0;
    let axis = |a: i32| {
        let mut c = a as f32 * 2.0;
        c /= n as f32;
        c -= 1.0;
        c
    };
    p[0] = axis(x) * s;
    p[1] = axis(y) * s;
    p[2] = axis(z) * s;
}
fn shrink(q: &mut [f32], dim: usize) {
    for a in 0..dim {
        let mut s = q[a] + q[dim + a];
        let s2 = q[2 * dim + a] + q[3 * dim + a];
        s += s2;
        s *= 0.25;
        for v in 0..4 {
            let mut d = q[v * dim + a] - s;
            d *= 0.9;
            q[v * dim + a] = s + d;
        }
    }
}
pub fn rough(n: i32, amp: f32, seed: u32, shatter: bool) -> Gen {
    let tris = 12 * (n * n) as usize;
    let mut g = Gen {
        corners: vec![0.; tris * 9],
        uvs: vec![0.; tris * 6],
        slots: vec![0; tris],
    };
    let order = [0usize, 1, 2, 0, 2, 3];
    let mut t = 0;
    let mut qp = [0f32; 12];
    let mut qu = [0f32; 8];
    for face in 0..6i32 {
        let axis = (face / 2) as usize;
        let sign = if face % 2 == 0 { 1 } else { -1 };
        for j in 0..n {
            for i in 0..n {
                for v in 0..4usize {
                    let u = i32::from(!(v == 0 || v == 3));
                    let w = i32::from(v >= 2);
                    let iu = i + u;
                    let jw = j + w;
                    let mut c = [0i32; 3];
                    c[axis] = if sign > 0 { n } else { 0 };
                    c[(axis + 1) % 3] = iu;
                    c[(axis + 2) % 3] = if sign > 0 { jw } else { n - jw };
                    lattice(c[0], c[1], c[2], n, amp, seed, &mut qp[v * 3..v * 3 + 3]);
                    let mut fu = iu as f32 / n as f32;
                    fu += (face % 3) as f32;
                    fu /= 3.0;
                    let mut fv = jw as f32 / n as f32;
                    fv += (face / 3) as f32;
                    fv /= 2.0;
                    qu[v * 2] = fu;
                    qu[v * 2 + 1] = fv;
                }
                if shatter {
                    shrink(&mut qp, 3);
                    shrink(&mut qu, 2);
                }
                for k in 0..2 {
                    for cc in 0..3 {
                        let v = order[k * 3 + cc];
                        g.corners[t * 9 + cc * 3..t * 9 + cc * 3 + 3]
                            .copy_from_slice(&qp[v * 3..v * 3 + 3]);
                        g.uvs[t * 6 + cc * 2..t * 6 + cc * 2 + 2]
                            .copy_from_slice(&qu[v * 2..v * 2 + 2]);
                    }
                    g.slots[t] = face;
                    t += 1;
                }
            }
        }
    }
    g
}
pub fn nested(a: &Gen, b: &Gen, scale: f32, slot_offset: i32) -> Gen {
    let (na, nb) = (a.slots.len(), b.slots.len());
    let mut g = Gen {
        corners: a.corners.clone(),
        uvs: a.uvs.clone(),
        slots: a.slots.clone(),
    };
    assert!(na > 0 && nb > 0);
    for v in &b.corners {
        let scaled = v * scale;
        g.corners.push(scaled);
    }
    g.uvs.extend(&b.uvs);
    g.slots.extend(b.slots.iter().map(|s| s + slot_offset));
    g
}
pub fn rough_tangents(corners: &[f32]) -> Vec<f32> {
    let n = corners.len() / 9;
    let mut tg = vec![0.; n * 12];
    for t in 0..n {
        if t % 4 == 0 {
            continue;
        }
        for c in 0..3 {
            let a = t * 9 + c * 3;
            let b = t * 9 + (c + 1) % 3 * 3;
            let o = t * 12 + c * 4;
            for k in 0..3 {
                tg[o + k] = corners[b + k] - corners[a + k];
            }
            tg[o + 3] = if (t + c) % 3 == 0 { -1. } else { 1. };
        }
    }
    tg
}
pub fn rough_colors(n: usize, seed: u32) -> Vec<f32> {
    let mut palette = [0f32; 16];
    for c in 0..4usize {
        for k in 0..3usize {
            palette[c * 4 + k] = match c {
                0 => 0.,
                1 => 1.,
                _ => unit(mix(((c * 7 + k) as u32) ^ seed)),
            };
        }
        palette[c * 4 + 3] = 1.;
    }
    let mut colors = vec![0.; n * 12];
    for t in 0..n {
        for c in 0..3 {
            let pick = (mix(((t * 3 + c) as u32 + 17) ^ seed) % 4) as usize;
            colors[t * 12 + c * 4..t * 12 + c * 4 + 4]
                .copy_from_slice(&palette[pick * 4..pick * 4 + 4]);
        }
    }
    colors
}
#[derive(Default)]
pub struct Extra {
    pub normals: Option<Vec<f32>>,
    pub normal_source: Option<String>,
    pub tangents: Option<Vec<f32>>,
    pub colors: Option<Vec<f32>>,
    pub renderers: Option<Vec<i32>>,
    pub names: Option<Vec<&'static str>>,
    pub keys: Option<Vec<Option<String>>>,
}
pub fn input(g: &Gen, e: Extra) -> MeshBakeInput {
    MeshBakeInput::new(
        g.corners.clone(),
        g.uvs.clone(),
        g.slots.clone(),
        MeshBakeAttributes {
            normals: e.normals,
            tangents: e.tangents,
            colors: e.colors,
            renderers: e.renderers,
            renderer_names: e.names.map(|n| n.into_iter().map(String::from).collect()),
            material_keys: e.keys,
            normal_source: e.normal_source,
            ..Default::default()
        },
    )
    .unwrap()
}
pub fn settings(width: i32, height: i32) -> MeshBakeSettings {
    MeshBakeSettings {
        width,
        height,
        target_slot: -1,
        padding: 0,
        antialiasing: 1,
        maps: MeshMapKind::ALL.to_vec(),
        ao_samples: 16,
        thickness_samples: 16,
        curvature_radius: 0.05,
        ao_max_distance: 0.1,
        thickness_max_distance: 0.3,
        ..Default::default()
    }
}
pub fn benchmark_model() -> MeshBakeInput {
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
    MeshBakeInput::new(corners, uvs, slots, MeshBakeAttributes::default()).unwrap()
}
pub struct Case {
    pub name: &'static str,
    pub input: MeshBakeInput,
    pub reference: Option<MeshBakeInput>,
    pub settings: MeshBakeSettings,
}
pub fn cases() -> Vec<Case> {
    let mut out = vec![];
    let a = rough(12, 0.2, 1, false);
    let self_input = input(&a, Extra::default());
    let mut s1 = settings(64, 64);
    s1.padding = 4;
    out.push(Case {
        name: "rough-self",
        input: self_input.clone(),
        reference: None,
        settings: s1,
    });
    let mut s2 = settings(48, 40);
    s2.ao_ignore_backfaces = true;
    s2.ao_falloff = MeshOcclusionFalloff::None;
    s2.antialiasing = 2;
    out.push(Case {
        name: "rough-backfaces-none",
        input: self_input,
        reference: None,
        settings: s2,
    });

    let nested_input = input(
        &nested(&rough(8, 0.1, 21, false), &rough(8, 0.1, 22, false), 1.2, 6),
        Extra::default(),
    );
    let mut s2a = settings(64, 64);
    s2a.ao_ignore_backfaces = true;
    s2a.ao_max_distance = 0.12;
    out.push(Case {
        name: "nested-ignore-backfaces",
        input: nested_input.clone(),
        reference: None,
        settings: s2a,
    });
    let mut s2b = settings(48, 48);
    s2b.ao_falloff = MeshOcclusionFalloff::None;
    s2b.antialiasing = 2;
    s2b.ao_max_distance = 0.12;
    s2b.padding = 2;
    out.push(Case {
        name: "nested-occlude-none",
        input: nested_input,
        reference: None,
        settings: s2b,
    });

    let low = rough(10, 0.06, 2, false);
    let high = rough(16, 0.06, 3, false);
    let high_input = input(&high, Extra::default());
    let low_tangents = input(
        &low,
        Extra {
            normals: Some(reconstruct_normals(&low.corners, 60.).unwrap()),
            normal_source: Some("reconstructed-crease-60".into()),
            tangents: Some(rough_tangents(&low.corners)),
            ..Default::default()
        },
    );
    let mut s3 = settings(64, 64);
    s3.reference_average_normals = false;
    s3.reference_frontal = 0.03;
    s3.reference_rear = 0.03;
    s3.padding = 3;
    out.push(Case {
        name: "rough-ref-tangents-vertex-cage",
        input: low_tangents,
        reference: Some(high_input.clone()),
        settings: s3,
    });

    let low_plain = input(&low, Extra::default());
    let mut s4 = settings(56, 48);
    s4.id_source = MeshIdSource::MeshPart;
    s4.reference_frontal = 0.05;
    s4.reference_rear = 0.02;
    s4.antialiasing = 3;
    out.push(Case {
        name: "rough-ref-fallback-tangents",
        input: low_plain,
        reference: Some(high_input),
        settings: s4,
    });

    let low_renderers: Vec<i32> = low.slots.iter().map(|s| s % 4).collect();
    let high_renderers: Vec<i32> = high.slots.iter().map(|s| s % 4).collect();
    let low_named = input(
        &low,
        Extra {
            renderers: Some(low_renderers),
            names: Some(vec!["arm_low", "LEG_Low", "tail_low", "Head"]),
            ..Default::default()
        },
    );
    let high_named = input(
        &high,
        Extra {
            renderers: Some(high_renderers),
            names: Some(vec!["ARM_HIGH", "leg", "Tail_high", "body_high"]),
            ..Default::default()
        },
    );
    let mut s5 = settings(64, 48);
    s5.reference_match_by_name = true;
    s5.id_source = MeshIdSource::Mesh;
    s5.reference_frontal = 0.04;
    s5.reference_rear = 0.04;
    out.push(Case {
        name: "rough-ref-names",
        input: low_named,
        reference: Some(high_named),
        settings: s5,
    });

    let shattered = rough(6, 0.05, 4, true);
    let smooth_high = rough(10, 0.05, 5, false);
    let low_keys: Vec<Option<String>> = (0..shattered.slots.len())
        .map(|t| {
            if (t / 2) % 5 == 0 {
                None
            } else {
                Some(format!("m{}", (t / 2) % 40))
            }
        })
        .collect();
    let high_keys: Vec<Option<String>> = (0..smooth_high.slots.len())
        .map(|t| {
            if smooth_high.slots[t] < 3 {
                None
            } else {
                Some(format!("m{}", t % 7))
            }
        })
        .collect();
    let low_shattered = input(
        &shattered,
        Extra {
            keys: Some(low_keys),
            ..Default::default()
        },
    );
    let high_keyed = input(
        &smooth_high,
        Extra {
            keys: Some(high_keys),
            ..Default::default()
        },
    );
    let mut s6 = settings(64, 64);
    s6.id_source = MeshIdSource::MaterialAsset;
    s6.padding = 2;
    s6.reference_frontal = 0.08;
    s6.reference_rear = 0.08;
    out.push(Case {
        name: "shattered-material-asset-ref",
        input: low_shattered,
        reference: Some(high_keyed),
        settings: s6,
    });

    let low_s = input(&shattered, Extra::default());
    let high_s = input(&smooth_high, Extra::default());
    let mut s7 = settings(64, 64);
    s7.id_source = MeshIdSource::MeshPart;
    s7.reference_frontal = 0.08;
    s7.reference_rear = 0.08;
    out.push(Case {
        name: "shattered-mesh-part-ref",
        input: low_s.clone(),
        reference: Some(high_s.clone()),
        settings: s7,
    });
    let mut s8 = settings(64, 64);
    s8.id_source = MeshIdSource::UvIsland;
    s8.padding = 3;
    s8.antialiasing = 2;
    s8.reference_frontal = 0.08;
    s8.reference_rear = 0.08;
    out.push(Case {
        name: "shattered-uv-island-ref",
        input: low_s.clone(),
        reference: Some(high_s),
        settings: s8,
    });

    let low_colored = input(
        &shattered,
        Extra {
            colors: Some(rough_colors(shattered.slots.len(), 11)),
            ..Default::default()
        },
    );
    let high_colored = input(
        &smooth_high,
        Extra {
            colors: Some(rough_colors(smooth_high.slots.len(), 12)),
            ..Default::default()
        },
    );
    let mut s9 = settings(64, 64);
    s9.id_source = MeshIdSource::VertexColor;
    s9.reference_frontal = 0.08;
    s9.reference_rear = 0.08;
    out.push(Case {
        name: "shattered-vertex-color-ref",
        input: low_colored,
        reference: Some(high_colored),
        settings: s9,
    });

    let (_, binding) = id_part_binding(&low_s);
    let mut s10 = settings(64, 64);
    s10.id_source = MeshIdSource::MeshPart;
    s10.target_slot = 1;
    s10.target_slots = vec![1, 4];
    s10.occluders = MeshOccluders::TargetSlotOnly;
    s10.manual_id_colors = IdColorAssignments::new(
        binding,
        [(40, 0x00ff80), (150, 0x102030)].into_iter().collect(),
    )
    .unwrap();
    out.push(Case {
        name: "shattered-manual-colors",
        input: low_s,
        reference: None,
        settings: s10,
    });

    let big = input(&rough(24, 0.1, 6, false), Extra::default());
    let mut s11 = settings(96, 70);
    s11.padding = 6;
    out.push(Case {
        name: "rough-large",
        input: big,
        reference: None,
        settings: s11,
    });

    out.push(Case {
        name: "bench-256",
        input: benchmark_model(),
        reference: None,
        settings: MeshBakeSettings {
            width: 256,
            height: 256,
            target_slot: -1,
            padding: 0,
            maps: MeshMapKind::ALL.to_vec(),
            ao_samples: 8,
            thickness_samples: 8,
            ..Default::default()
        },
    });
    out
}
