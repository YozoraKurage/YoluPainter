use std::sync::atomic::{AtomicBool, Ordering};
use yolu_core::mesh_maps::*;
fn plane(uvs: Vec<f32>) -> MeshBakeInput {
    MeshBakeInput::new(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        uvs,
        vec![0],
        MeshBakeAttributes::default(),
    )
    .unwrap()
}
fn settings() -> MeshBakeSettings {
    MeshBakeSettings {
        width: 4,
        height: 4,
        padding: 0,
        maps: MeshMapKind::ALL.to_vec(),
        ao_samples: 8,
        thickness_samples: 8,
        ..Default::default()
    }
}
#[test]
fn plane_known_values() {
    let input = plane(vec![0., 0., 1., 0., 0., 1.]);
    let s = settings();
    let r = bake(
        &input,
        &s,
        &MeshBakeBudget::default(),
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    assert_eq!(r.report.covered_texels, 10);
    assert_eq!(r.report.empty_texels, 6);
    for map in &r.maps {
        let at = |c| map.raw_value(0, 0, c).unwrap();
        match map.kind() {
            MeshMapKind::WorldNormal | MeshMapKind::TangentNormal => {
                assert_eq!([at(0), at(1), at(2)], [32768, 32768, 65535])
            }
            MeshMapKind::Position => assert_eq!([at(0), at(1), at(2)], [8192, 8192, 32768]),
            MeshMapKind::AmbientOcclusion | MeshMapKind::Thickness | MeshMapKind::Opacity => {
                assert_eq!(at(0), 65535)
            }
            MeshMapKind::Curvature | MeshMapKind::Height => assert_eq!(at(0), 32768),
            MeshMapKind::Id => assert_eq!([at(0), at(1), at(2)], [0, 0, 65535]),
            MeshMapKind::BentNormal => assert!(at(2) > 32768),
        }
        assert_eq!(map.raw_value(3, 3, 0).unwrap(), 0);
        assert_eq!(map.to_rgba8(false)[15 * 4 + 3], 0);
    }
}
#[test]
fn rejects_uv_settings_and_manual_binding() {
    let budget = MeshBakeBudget::default();
    let input = plane(vec![0., 0., 1., 0., 0., 1.]);
    let no_faces = "対象スロットに焼き込めるUVのある三角形がありません";
    // 断る前に進捗を呼ばないこと。準備の途中で分かる断り（手動 ID 色の対応）は "Preparing" までで、焼き始めない。
    let bake_refusal = |input: &MeshBakeInput, s: &MeshBakeSettings| {
        let mut phases = vec![];
        let error = bake(input, s, &budget, None, None, |_, phase| {
            phases.push(phase.to_string());
            true
        })
        .err()
        .expect("断られるはずです")
        .to_string();
        assert!(
            phases.iter().all(|p| p == "Preparing"),
            "{error}: {phases:?}"
        );
        (error, phases.len())
    };
    let bake_error = |input: &MeshBakeInput, s: &MeshBakeSettings| {
        let (error, calls) = bake_refusal(input, s);
        assert_eq!(calls, 0, "{error}: 断る前に進捗を呼んでいます");
        error
    };
    // UV: 面積 0 は焼く面が無い、0〜1 の外は繰り返し・UDIM として断る
    assert_eq!(bake_error(&plane(vec![0.; 6]), &settings()), no_faces);
    assert!(
        bake_error(&plane(vec![0., 0., 2., 0., 0., 1.]), &settings())
            .starts_with("UVが0〜1の外です")
    );
    // 設定の検証（面が無いのとは別の理由）
    let with = |change: &dyn Fn(&mut MeshBakeSettings)| {
        let mut s = settings();
        change(&mut s);
        s
    };
    for (s, why) in [
        (with(&|s| s.width = 0), "ベイクの大きさが範囲外です"),
        (with(&|s| s.height = 8193), "ベイクの大きさが範囲外です"),
        (
            with(&|s| s.maps.push(MeshMapKind::Id)),
            "マップの種類が空または重複しています",
        ),
        (
            with(&|s| s.maps.clear()),
            "マップの種類が空または重複しています",
        ),
        (
            with(&|s| s.ao_max_distance = f64::NAN),
            "距離は対角線比で0より大きく4以下が必要です",
        ),
        (
            with(&|s| s.reference_rear = 4.5),
            "距離は対角線比で0より大きく4以下が必要です",
        ),
        (with(&|s| s.ao_samples = 0), "レイのサンプル数が範囲外です"),
        (
            with(&|s| s.thickness_spread_degrees = 0.5),
            "レイの広がりは1〜180度が必要です",
        ),
        (
            with(&|s| s.curvature_radius = 0.0001),
            "曲率半径が範囲外です",
        ),
        (
            with(&|s| s.padding = 65),
            "余白またはアンチエイリアスが範囲外です",
        ),
        (
            with(&|s| s.antialiasing = 5),
            "余白またはアンチエイリアスが範囲外です",
        ),
        (
            with(&|s| s.target_slots = vec![1, 0]),
            "対象スロットが不正です",
        ),
        (with(&|s| s.target_slot = -2), "対象スロットが不正です"),
        (
            with(&|s| {
                s.target_slot = 0;
                s.target_slots = vec![1];
            }),
            "対象スロットが不正です",
        ),
    ] {
        assert_eq!(bake_error(&input, &s), why);
    }
    // 設定は正しいが、そのスロットを使う三角形が無い
    assert_eq!(bake_error(&input, &with(&|s| s.target_slot = 1)), no_faces);
    assert_eq!(
        bake_error(
            &input,
            &with(&|s| {
                s.target_slot = 1;
                s.target_slots = vec![1, 2];
            })
        ),
        no_faces
    );
    // 手動 ID 色は別のモデルの指紋では断る
    let mut s = settings();
    s.manual_id_colors =
        IdColorAssignments::new("0".repeat(64), [(0, 0)].into_iter().collect()).unwrap();
    let (error, calls) = bake_refusal(&input, &s);
    assert_eq!(error, "手動ID色が別のモデルまたは分割に属しています");
    assert!(calls > 0, "準備に入ってから分かる断り");
    // 手動 ID 色の型
    for (binding, colors) in [
        ("0".repeat(64), vec![(0usize, 0x1000000u32)]),
        ("0".repeat(63), vec![(0, 0)]),
        ("G".repeat(64), vec![(0, 0)]),
        ("0".repeat(64), vec![(4_000_000, 0)]),
    ] {
        assert!(IdColorAssignments::new(binding, colors.into_iter().collect()).is_err());
    }
}
#[test]
fn cancellation_during_rows_and_padding_returns_nothing() {
    let input = plane(vec![0., 0., 1., 0., 0., 1.]);
    let cancel = AtomicBool::new(false);
    let mut s = settings();
    s.height = 64;
    s.padding = 3;
    for phase in ["Baking", "Padding"] {
        cancel.store(false, Ordering::Relaxed);
        let r = bake(
            &input,
            &s,
            &MeshBakeBudget::default(),
            Some(&cancel),
            None,
            |fraction, current| {
                if current == phase && fraction > 0.05 {
                    cancel.store(true, Ordering::Relaxed);
                    return false;
                }
                true
            },
        )
        .unwrap();
        assert_eq!(r.status, MeshBakeStatus::Canceled);
        assert!(r.maps.is_empty());
    }
}
#[test]
fn palette_is_unique_and_never_gray() {
    for n in [0, 1, 6, 24, 60, 120, 4096] {
        let colors = id_palette(n).unwrap();
        let set: std::collections::HashSet<_> = colors.iter().collect();
        assert_eq!(set.len(), n);
        assert!(colors.iter().all(|c| {
            let r = c >> 16;
            let g = c >> 8 & 255;
            let b = c & 255;
            r != g || g != b
        }));
    }
}

fn mesh(
    corners: Vec<f32>,
    uvs: Vec<f32>,
    slots: Vec<i32>,
    attrs: MeshBakeAttributes,
) -> MeshBakeInput {
    MeshBakeInput::new(corners, uvs, slots, attrs).unwrap()
}
fn baked(
    input: &MeshBakeInput,
    s: &MeshBakeSettings,
    reference: Option<&MeshBakeInput>,
) -> MeshBakeReport {
    let r = bake(
        input,
        s,
        &MeshBakeBudget::default(),
        None,
        reference,
        |_, _| true,
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    assert_eq!(r.report.diagnostics.len(), r.report.notes.len());
    for (text, note) in r.report.diagnostics.iter().zip(&r.report.notes) {
        assert_eq!(text, &note.to_string());
        assert!(!text.is_empty() && !text.contains("クリック"), "{text}");
    }
    r.report
}
fn only(kinds: &[MeshMapKind]) -> MeshBakeSettings {
    MeshBakeSettings {
        maps: kinds.to_vec(),
        ..settings()
    }
}
const TRIANGLE_UV: [f32; 6] = [0., 0., 1., 0., 0., 1.];

#[test]
fn notes_report_flat_white_and_missing_results() {
    use MeshBakeNote::*;
    // 高ポリも頂点法線も接線も無い: 平ら・面の法線・UV から作った接線
    let flat = plane(TRIANGLE_UV.to_vec());
    let r = baked(&flat, &settings(), None);
    assert_eq!(
        r.notes,
        vec![
            FaceNormals,
            TangentFallback(1),
            IdParts {
                parts: 1,
                source: MeshIdSource::MaterialSlot,
                separation: 255
            },
            NoReference
        ]
    );
    assert_eq!(r.fallback_triangles, 1);
    assert_eq!(r.id_parts, 1);
    // 接空間法線も高さも ID も焼かないなら、その理由は出ない
    let r = baked(
        &flat,
        &only(&[MeshMapKind::WorldNormal, MeshMapKind::Position]),
        None,
    );
    assert_eq!(r.notes, vec![FaceNormals]);
    assert_eq!(
        r.fallback_triangles, 0,
        "接空間法線を焼かないときは数えない"
    );
    // 高ポリがあれば NoReference は出ない（当たらなくても別の理由で出る）
    let reference = plane(TRIANGLE_UV.to_vec());
    let r = baked(
        &flat,
        &only(&[MeshMapKind::TangentNormal, MeshMapKind::Height]),
        Some(&reference),
    );
    assert_eq!(r.notes, vec![FaceNormals, TangentFallback(1)]);
    // 頂点法線は由来で言葉が変わる（authored は何も出さない）
    let normals = vec![0., 0., 1., 0., 0., 1., 0., 0., 1.];
    for (source, expected) in [
        (None, vec![]),
        (Some("authored"), vec![]),
        (
            Some("reconstructed-crease-60"),
            vec![ReconstructedNormals("reconstructed-crease-60".into())],
        ),
    ] {
        let input = mesh(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            TRIANGLE_UV.to_vec(),
            vec![0],
            MeshBakeAttributes {
                normals: Some(normals.clone()),
                normal_source: source.map(String::from),
                ..Default::default()
            },
        );
        assert_eq!(
            baked(&input, &only(&[MeshMapKind::WorldNormal]), None).notes,
            expected,
            "{source:?}"
        );
    }
}
#[test]
fn notes_count_triangles_without_tangents_and_zero_uv_area_and_overlap() {
    use MeshBakeNote::*;
    // 2 つの三角形。1 つ目にだけ接線がある
    let corners = vec![
        0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 0., 1., 1.,
    ];
    let uvs = vec![0., 0., 0.5, 0., 0., 0.5, 0.5, 0.5, 1., 0.5, 0.5, 1.];
    let mut tangents = vec![0.; 24];
    for c in 0..3 {
        tangents[c * 4] = 1.;
        tangents[c * 4 + 3] = 1.;
    }
    let with_tangents = mesh(
        corners.clone(),
        uvs.clone(),
        vec![0, 0],
        MeshBakeAttributes {
            normals: Some(vec![
                0., 0., 1., 0., 0., 1., 0., 0., 1., 0., -1., 0., 0., -1., 0., 0., -1., 0.,
            ]),
            tangents: Some(tangents.clone()),
            ..Default::default()
        },
    );
    let r = baked(&with_tangents, &only(&[MeshMapKind::TangentNormal]), None);
    assert_eq!(r.notes, vec![TangentFallback(1), NoReference]);
    assert_eq!(r.fallback_triangles, 1);
    // 全部に接線があれば数えない
    tangents[12..16].copy_from_slice(&[0., 1., 0., -1.]);
    let all = mesh(
        corners.clone(),
        uvs,
        vec![0, 0],
        MeshBakeAttributes {
            tangents: Some(tangents),
            ..Default::default()
        },
    );
    let r = baked(&all, &only(&[MeshMapKind::TangentNormal]), None);
    assert_eq!(r.fallback_triangles, 0);
    assert_eq!(r.notes, vec![FaceNormals, NoReference]);
    // UV 面積が 0 の三角形は焼かず、遮るだけ（数を知らせ、ほかの三角形は焼く）
    let degenerate = mesh(
        corners.clone(),
        vec![0., 0., 1., 0., 0., 1., 0.2, 0.2, 0.2, 0.2, 0.2, 0.2],
        vec![0, 0],
        MeshBakeAttributes::default(),
    );
    let r = baked(&degenerate, &only(&[MeshMapKind::WorldNormal]), None);
    assert_eq!((r.zero_uv_area_triangles, r.receiving_triangles), (1, 1));
    assert_eq!(r.notes, vec![ZeroUvAreaTriangles(1), FaceNormals]);
    // 別のスロットの面積 0 は数えない
    let other_slot = mesh(
        corners.clone(),
        vec![0., 0., 1., 0., 0., 1., 0.2, 0.2, 0.2, 0.2, 0.2, 0.2],
        vec![0, 1],
        MeshBakeAttributes::default(),
    );
    let mut s = only(&[MeshMapKind::WorldNormal]);
    s.target_slot = 0;
    assert_eq!(baked(&other_slot, &s, None).notes, vec![FaceNormals]);
    // 重なる UV は先頭に出る
    let overlap = mesh(
        corners,
        vec![0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.],
        vec![0, 0],
        MeshBakeAttributes::default(),
    );
    let r = baked(&overlap, &only(&[MeshMapKind::WorldNormal]), None);
    assert!(r.overlap_texels > 0);
    assert_eq!(
        r.notes,
        vec![OverlappingTexels(r.overlap_texels), FaceNormals]
    );
}
#[test]
fn notes_report_missed_samples_ignored_edges_and_id_notes() {
    use MeshBakeNote::*;
    let low = plane(TRIANGLE_UV.to_vec());
    let far = mesh(
        vec![0., 0., 5., 1., 0., 5., 0., 1., 5.],
        TRIANGLE_UV.to_vec(),
        vec![0],
        MeshBakeAttributes::default(),
    );
    for by_name in [false, true] {
        let mut s = only(&[MeshMapKind::Opacity, MeshMapKind::Height]);
        s.reference_match_by_name = by_name;
        s.reference_frontal = 0.01;
        s.reference_rear = 0.01;
        let r = baked(&low, &s, Some(&far));
        assert!(r.projected_samples > 0);
        assert_eq!(
            r.missed_samples, r.projected_samples,
            "遠い高ポリには全部当たらない"
        );
        assert_eq!(
            r.notes,
            vec![
                FaceNormals,
                MissedSamples {
                    missed: r.missed_samples,
                    projected: r.projected_samples,
                    by_name
                }
            ]
        );
    }
    // 辺: 3 面が共有（非多様体）、同じ向きで共有（巻きが合わない）
    let fan = |third: [f32; 9]| {
        let mut corners = vec![
            0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        corners.extend(third);
        mesh(
            corners.clone(),
            [TRIANGLE_UV, TRIANGLE_UV, TRIANGLE_UV][..corners.len() / 9].concat(),
            vec![0; corners.len() / 9],
            MeshBakeAttributes::default(),
        )
    };
    let s = only(&[MeshMapKind::Curvature]);
    let r = baked(&fan([0., 0., 0., 1., 0., 0., 0., -1., 0.]), &s, None);
    assert_eq!((r.non_manifold_edges, r.inconsistent_winding_edges), (1, 0));
    assert!(r.notes.contains(&IgnoredEdges(1)), "{:?}", r.notes);
    let s2 = only(&[MeshMapKind::Curvature]);
    let flipped = mesh(
        vec![
            0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        [TRIANGLE_UV, TRIANGLE_UV].concat(),
        vec![0, 0],
        MeshBakeAttributes::default(),
    );
    let r = baked(&flipped, &s2, None);
    assert_eq!((r.non_manifold_edges, r.inconsistent_winding_edges), (0, 1));
    assert!(r.notes.contains(&IgnoredEdges(1)));
    // ID: 部品の数と離れ（手動色・頂点カラーでは別の記録）
    let mut ids = only(&[MeshMapKind::Id]);
    for (source, parts, separation) in [
        (MeshIdSource::MaterialSlot, 1, 255),
        (MeshIdSource::MeshPart, 2, 255),
        (MeshIdSource::UvIsland, 2, 255),
        (MeshIdSource::Mesh, 1, 255),
        (MeshIdSource::MaterialAsset, 1, 255),
    ] {
        ids.id_source = source;
        let input = mesh(
            vec![
                0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 0., 1., 1.,
            ],
            vec![0., 0., 0.4, 0., 0., 0.4, 0.6, 0.6, 1., 0.6, 0.6, 1.],
            vec![0, 0],
            MeshBakeAttributes {
                normals: Some(vec![0.; 18]),
                ..Default::default()
            },
        );
        let r = baked(&input, &ids, None);
        let found = r.notes.iter().find_map(|n| match n {
            IdParts {
                parts,
                separation,
                source: s,
            } if *s == source => Some((*parts, *separation)),
            _ => None,
        });
        assert_eq!(found, Some((parts, separation)), "{source:?}");
        assert_eq!(r.id_parts, parts);
    }
    ids.id_source = MeshIdSource::VertexColor;
    let plain = plane(TRIANGLE_UV.to_vec());
    assert_eq!(
        baked(&plain, &ids, None).notes,
        vec![FaceNormals, NoVertexColors]
    );
    let colored = mesh(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        TRIANGLE_UV.to_vec(),
        vec![0],
        MeshBakeAttributes {
            colors: Some(vec![1., 0., 0., 1., 1., 0., 0., 1., 1., 0., 0., 1.]),
            ..Default::default()
        },
    );
    assert_eq!(baked(&colored, &ids, None).notes, vec![FaceNormals]);
    let (_, binding) = id_part_binding(&plain);
    ids.id_source = MeshIdSource::MaterialSlot;
    ids.manual_id_colors =
        IdColorAssignments::new(binding, [(0, 0x112233)].into_iter().collect()).unwrap();
    assert_eq!(
        baked(&plain, &ids, None).notes,
        vec![FaceNormals, ManualIdColors(1)]
    );
}
#[test]
fn progress_ends_with_done_and_ignores_its_answer() {
    let input = plane(TRIANGLE_UV.to_vec());
    let mut steps: Vec<(f64, String)> = vec![];
    let r = bake(
        &input,
        &settings(),
        &MeshBakeBudget::default(),
        None,
        None,
        |f, p| {
            steps.push((f, p.to_string()));
            true
        },
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    assert_eq!(steps.last(), Some(&(1., "Done".to_string())));
    assert!(steps.windows(2).all(|w| w[0].0 <= w[1].0), "{steps:?}");
    assert_eq!(steps[0], (0., "Preparing".to_string()));
    // 完了の通知に false を返しても、もう結果は決まっている
    let r = bake(
        &input,
        &settings(),
        &MeshBakeBudget::default(),
        None,
        None,
        |_, p| p != "Done",
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    assert_eq!(r.maps.len(), MeshMapKind::ALL.len());
    // 取り消しでは Done を呼ばない
    let mut done = false;
    let r = bake(
        &input,
        &settings(),
        &MeshBakeBudget::default(),
        None,
        None,
        |f, p| {
            done |= p == "Done";
            f < 0.05
        },
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Canceled);
    assert!(!done);
}
