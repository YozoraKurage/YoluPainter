use yolu_core::mesh_maps::*;
fn cube_case(scale: f32, scenario: usize) -> MeshBakeInput {
    let mut corners = vec![0.; 12 * 9];
    let mut uvs = vec![0.; 12 * 6];
    let mut slots = vec![0; 12];
    let order = [0, 1, 2, 0, 2, 3];
    for face in 0..6 {
        let axis = face / 2;
        let sign = if face % 2 == 0 { 1. } else { -1. };
        for (c, &v) in order.iter().enumerate() {
            let u = if v == 0 || v == 3 { 0. } else { 1. };
            let w = if v < 2 { 0. } else { 1. };
            let k = (face * 6 + c) * 3;
            corners[k + axis] = sign * scale;
            corners[k + (axis + 1) % 3] = (u * 2. - 1.) * scale;
            corners[k + (axis + 2) % 3] = (w * 2. - 1.) * sign * scale;
            let q = (face * 6 + c) * 2;
            uvs[q] = (face as f32 % 3. + u) / 3.;
            uvs[q + 1] = ((face / 3) as f32 + w) / 2.;
        }
        slots[face * 2] = face as i32;
        slots[face * 2 + 1] = face as i32;
    }
    if scenario == 11 {
        for t in 0..12 {
            uvs[t * 6..t * 6 + 6].copy_from_slice(&[0., 0., 1., 0., 0., 1.]);
        }
    }
    if scenario == 14 {
        for t in 0..12 {
            for a in 0..3 {
                corners.swap(t * 9 + 3 + a, t * 9 + 6 + a);
            }
        }
    }
    let attrs = MeshBakeAttributes {
        normals: if scenario == 10 {
            Some(reconstruct_normals(&corners, 180.).unwrap())
        } else {
            None
        },
        colors: if scenario == 5 {
            Some((0..144).map(|j| (j * 17 % 31) as f32 / 30.).collect())
        } else {
            None
        },
        renderer_names: if scenario == 9 {
            Some(vec![if scale == 1. {
                "shell_low"
            } else {
                "other_high"
            }
            .into()])
        } else {
            None
        },
        material_keys: if scenario == 7 {
            Some(
                (0..12)
                    .map(|t| match t % 3 {
                        0 => Some("合成A".into()),
                        1 => Some("合成B".into()),
                        _ => None,
                    })
                    .collect(),
            )
        } else {
            None
        },
        ..Default::default()
    };
    MeshBakeInput::new(corners, uvs, slots, attrs).unwrap()
}
fn cube(scale: f32) -> MeshBakeInput {
    cube_case(scale, 0)
}
fn settings(scenario: usize) -> MeshBakeSettings {
    MeshBakeSettings {
        width: 24,
        height: 16,
        target_slot: if scenario == 3 { 0 } else { -1 },
        padding: if scenario == 3 { 2 } else { 0 },
        antialiasing: if scenario == 1 { 2 } else { 1 },
        maps: MeshMapKind::ALL.to_vec(),
        ao_samples: 8,
        thickness_samples: 8,
        curvature_radius: 0.2,
        thickness_max_distance: 1.,
        ao_max_distance: 1.,
        id_source: if scenario == 3 {
            MeshIdSource::UvIsland
        } else {
            MeshIdSource::MaterialSlot
        },
        ..Default::default()
    }
}
#[test]
fn csharp_maps_and_parallelism() {
    let mut differences = vec![];
    for scenario in 0..16 {
        let input = cube_case(1., scenario);
        let high = cube_case(1.01, scenario);
        let mut settings = settings(scenario);
        match scenario {
            4 => settings.id_source = MeshIdSource::Mesh,
            5 => settings.id_source = MeshIdSource::VertexColor,
            6 => settings.id_source = MeshIdSource::MeshPart,
            7 => settings.id_source = MeshIdSource::MaterialAsset,
            8 => {
                let (_, binding) = id_part_binding(&input);
                settings.manual_id_colors = IdColorAssignments::new(
                    binding,
                    [(0, 0x123456), (2, 0xfedcba)].into_iter().collect(),
                )
                .unwrap();
            }
            9 => settings.reference_match_by_name = true,
            12 => {
                settings.target_slot = 0;
                settings.target_slots = vec![0, 2];
                settings.antialiasing = 4;
                settings.padding = 3;
                settings.occluders = MeshOccluders::TargetSlotOnly;
            }
            15 => {
                settings.ao_max_distance = 1e-5;
                settings.thickness_max_distance = 0.10000000000000002;
            }
            13 => {
                settings.ao_ignore_backfaces = true;
                settings.ao_falloff = MeshOcclusionFalloff::None;
            }
            _ => {}
        }
        let mut previous = None;
        for threads in [1, 4] {
            let budget = MeshBakeBudget {
                max_degree_of_parallelism: threads,
                ..Default::default()
            };
            let result = bake(
                &input,
                &settings,
                &budget,
                None,
                if scenario == 2 || scenario == 9 {
                    Some(&high)
                } else {
                    None
                },
                |_, _| true,
            )
            .unwrap();
            assert_eq!(result.status, MeshBakeStatus::Completed);
            if let Some(previous) = &previous {
                assert_eq!(&result.maps, previous, "並列度 {threads}");
            }
            for map in &result.maps {
                let bytes = std::fs::read(format!(
                    "{}/tests/golden/mesh/bake-{scenario}-{}.bin",
                    env!("CARGO_MANIFEST_DIR"),
                    map.kind().name()
                ))
                .unwrap();
                let expected = yolu_io::mesh_map::read(&bytes).unwrap();
                let diff = map
                    .data()
                    .iter()
                    .zip(expected.data())
                    .filter(|(a, b)| a != b)
                    .count();
                if diff > 0 {
                    differences.push(format!(
                        "{scenario}/{:?}: {diff} 成分, 最大差 {}",
                        map.kind(),
                        map.data()
                            .iter()
                            .zip(expected.data())
                            .map(|(a, b)| a.abs_diff(*b))
                            .max()
                            .unwrap()
                    ));
                }
                assert_eq!(
                    map.coverage(),
                    expected.coverage(),
                    "{scenario}/{:?} coverage",
                    map.kind()
                );
                assert_eq!(
                    map.provenance(),
                    expected.provenance(),
                    "{scenario}/{:?} provenance",
                    map.kind()
                );
                if diff == 0 {
                    assert_eq!(yolu_io::mesh_map::write(map).unwrap(), bytes);
                }
            }
            previous = Some(result.maps);
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
#[test]
fn refuses_budget_and_stops_without_partial_maps() {
    let input = cube(1.);
    let settings = settings(0);
    let budget = MeshBakeBudget {
        max_degree_of_parallelism: 1,
        ..Default::default()
    };
    let estimated = estimate_bytes(&input, &settings, &budget, None).unwrap();
    assert!(bake(
        &input,
        &settings,
        &MeshBakeBudget {
            max_bytes: estimated - 1,
            ..budget.clone()
        },
        None,
        None,
        |_, _| true
    )
    .is_err());
    let canceled = std::sync::atomic::AtomicBool::new(true);
    let r = bake(&input, &settings, &budget, Some(&canceled), None, |_, _| {
        true
    })
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Canceled);
    assert!(r.maps.is_empty());
    let r = bake(&input, &settings, &budget, None, None, |fraction, _| {
        fraction < 0.05
    })
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Canceled);
    assert!(r.maps.is_empty());
    let r = bake(
        &input,
        &settings,
        &MeshBakeBudget {
            max_seconds: f64::MIN_POSITIVE,
            ..budget
        },
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::TimedOut);
    assert!(r.maps.is_empty());
}

fn try_bake(
    input: &MeshBakeInput,
    settings: &MeshBakeSettings,
    max_bytes: u64,
) -> (Result<MeshBakeResult>, Vec<String>) {
    let budget = MeshBakeBudget {
        max_bytes,
        max_degree_of_parallelism: 1,
        ..Default::default()
    };
    let mut phases = vec![];
    let result = bake(input, settings, &budget, None, None, |_, phase| {
        phases.push(phase.to_string());
        true
    });
    (result, phases)
}
#[test]
fn budget_boundary_and_the_second_stage_refusals() {
    let input = cube(1.);
    // 1 段目: 見積もりちょうどで焼け、1 バイト足りなければ何も始めずに断る
    let lean = MeshBakeSettings {
        maps: vec![MeshMapKind::WorldNormal, MeshMapKind::Position],
        ..settings(0)
    };
    let estimate = estimate_bytes(
        &input,
        &lean,
        &MeshBakeBudget {
            max_degree_of_parallelism: 1,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let (result, phases) = try_bake(&input, &lean, estimate);
    let result = result.unwrap();
    assert_eq!(result.status, MeshBakeStatus::Completed);
    assert_eq!(result.report.estimated_bytes, estimate);
    assert_eq!(phases.last().map(String::as_str), Some("Done"));
    let (result, phases) = try_bake(&input, &lean, estimate - 1);
    assert_eq!(
        result.err().unwrap().to_string(),
        "ベイクの見積もりがメモリ予算を超えます"
    );
    assert!(phases.is_empty(), "断る前に進捗を呼んでいます: {phases:?}");
    assert!(try_bake(&input, &lean, 0).0.is_err());

    // 2 段目（曲率の線分）: 見積もりに入らない分は残りの予算で断る。準備に入ってから分かるので "Preparing" まで
    let curved = MeshBakeSettings {
        maps: vec![MeshMapKind::Curvature],
        ..settings(0)
    };
    let estimate = estimate_bytes(
        &input,
        &curved,
        &MeshBakeBudget {
            max_degree_of_parallelism: 1,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let segments = try_bake(&input, &curved, u64::MAX / 4)
        .0
        .unwrap()
        .report
        .curvature_segments as u64;
    assert!(segments > 1);
    let (result, phases) = try_bake(&input, &curved, estimate);
    assert_eq!(
        result.err().unwrap().to_string(),
        "曲率の線分が残りのベイク予算を超えます"
    );
    assert!(
        !phases.is_empty() && phases.iter().all(|p| p == "Preparing"),
        "{phases:?}"
    );
    let (result, _) = try_bake(&input, &curved, estimate + 76 * segments - 1);
    assert_eq!(
        result.err().unwrap().to_string(),
        "曲率の線分が残りのベイク予算を超えます"
    );
    let (result, _) = try_bake(&input, &curved, estimate + 76 * segments);
    assert_eq!(result.unwrap().status, MeshBakeStatus::Completed);

    // 2 段目（UV の行帯参照）: 帯をまたぐ三角形が多いほど要る。大きな UV で境目を探す
    let tall = MeshBakeSettings {
        maps: vec![MeshMapKind::WorldNormal],
        width: 24,
        height: 64,
        ..settings(0)
    };
    let estimate = estimate_bytes(
        &input,
        &tall,
        &MeshBakeBudget {
            max_degree_of_parallelism: 1,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let (result, phases) = try_bake(&input, &tall, estimate);
    assert_eq!(
        result.err().unwrap().to_string(),
        "UVの行帯参照がベイク予算を超えます"
    );
    assert!(
        !phases.is_empty() && phases.iter().all(|p| p == "Preparing"),
        "{phases:?}"
    );
    let needed = (1..=1024u64)
        .map(|n| n * 4)
        .find(|extra| try_bake(&input, &tall, estimate + extra).0.is_ok())
        .expect("数 KiB 足せば焼ける");
    assert!(try_bake(&input, &tall, estimate + needed - 4).0.is_err());
    assert_eq!(
        try_bake(&input, &tall, estimate + needed).0.unwrap().status,
        MeshBakeStatus::Completed
    );
}
