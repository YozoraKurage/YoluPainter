//! C# の MeshBaker（Unity 同梱 Mono）が焼いた、不規則な座標・数千面・高ポリ・名前・接線・頂点法線・ID の各元の結果との全バイト一致。
//! 入力は tests/support が作る（C# の MeshGoldenRough.cs と同じ手順）。正解は SHA-256 だけを置く（中身は再生成できる）。
use crate::support;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use yolu_core::mesh_maps::*;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn payload_sha(map: &BakedMeshMap) -> String {
    let mut bytes = map.coverage().to_vec();
    for v in map.data() {
        bytes.extend(v.to_le_bytes());
    }
    sha(&bytes)
}
fn golden(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/golden/mesh/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
struct Expected {
    checks: Vec<(String, String, String, String, String)>,
    maps: HashMap<(String, String), (String, String)>,
    keys: HashMap<(String, String), String>,
    reports: HashMap<String, HashMap<String, String>>,
}
fn expected() -> Expected {
    let mut e = Expected {
        checks: vec![],
        maps: HashMap::new(),
        keys: HashMap::new(),
        reports: HashMap::new(),
    };
    for line in golden("rough.txt").lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "map" => {
                e.maps
                    .insert((f[1].into(), f[2].into()), (f[3].into(), f[4].into()));
            }
            "key" => {
                e.keys.insert((f[1].into(), f[2].into()), f[3].into());
            }
            "report" => {
                e.reports.insert(
                    f[1].into(),
                    f[2..]
                        .iter()
                        .map(|p| {
                            let (k, v) = p.split_once('=').unwrap();
                            (k.to_string(), v.to_string())
                        })
                        .collect(),
                );
            }
            "check" => e.checks.push((
                f[1].into(),
                f[2].into(),
                f[3].into(),
                f[4].into(),
                f[5].into(),
            )),
            "probe" => assert_eq!(
                f[1], "float_sum=0",
                "Mono が float を float のまま計算していません"
            ),
            _ => panic!("{line}"),
        }
    }
    e
}
fn bake_with(case: &support::Case, threads: usize) -> MeshBakeResult {
    let budget = MeshBakeBudget {
        max_degree_of_parallelism: threads,
        ..Default::default()
    };
    let r = bake(
        &case.input,
        &case.settings,
        &budget,
        None,
        case.reference.as_ref(),
        |_, _| true,
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    r
}

#[test]
fn csharp_rough_cases_all_bytes_report_and_threads() {
    let expected = expected();
    let mut failures = vec![];
    for case in support::cases() {
        let mut previous: Option<Vec<BakedMeshMap>> = None;
        for threads in [1, 4] {
            let result = bake_with(&case, threads);
            if let Some(previous) = &previous {
                assert_eq!(&result.maps, previous, "{}: 並列度 {threads}", case.name);
            }
            for map in &result.maps {
                let key = (case.name.to_string(), format!("{:?}", map.kind()));
                let condition = condition_key(
                    &case.input,
                    &case.settings,
                    map.kind(),
                    case.reference.as_ref(),
                )
                .unwrap();
                if condition != expected.keys[&key] || condition != map.provenance().condition_key()
                {
                    failures.push(format!("{} {:?} 条件の鍵", case.name, map.kind()));
                }
                let (bin, payload) = &expected.maps[&key];
                let got_bin = sha(&yolu_io::mesh_map::write(map).unwrap());
                if &got_bin != bin || &payload_sha(map) != payload {
                    failures.push(format!("{} {:?} (並列度 {threads})", case.name, map.kind()));
                }
            }
            previous = Some(result.maps);
        }
        let r = bake_with(&case, 1).report;
        let want = &expected.reports[case.name];
        let got: Vec<(&str, u64)> = vec![
            ("estimated", r.estimated_bytes),
            ("receiving", r.receiving_triangles as u64),
            ("zero_uv", r.zero_uv_area_triangles as u64),
            ("degenerate", r.degenerate_triangles as u64),
            ("occluders", r.occluder_triangles as u64),
            ("reference", r.reference_triangles as u64),
            ("id_parts", r.id_parts as u64),
            ("rays", r.rays),
            ("projected", r.projected_samples),
            ("missed", r.missed_samples),
            ("covered", r.covered_texels),
            ("overlap", r.overlap_texels),
            ("padded", r.padded_texels),
            ("empty", r.empty_texels),
            ("boundary", r.boundary_edges as u64),
            ("non_manifold", r.non_manifold_edges as u64),
            ("inconsistent", r.inconsistent_winding_edges as u64),
            ("segments", r.curvature_segments as u64),
        ];
        let notes: Vec<String> = r.notes.iter().map(MeshBakeNote::code).collect();
        if want["notes"] != notes.join(",") {
            failures.push(format!(
                "{} notes: C# {} / Rust {}",
                case.name,
                want["notes"],
                notes.join(",")
            ));
        }
        assert_eq!(r.diagnostics.len(), r.notes.len());
        for (name, value) in got {
            if want[name] != value.to_string() {
                failures.push(format!(
                    "{} report {name}: C# {} / Rust {value}",
                    case.name, want[name]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn csharp_id_palette_levels_separation_and_colors() {
    for line in golden("id-palette.txt").lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "palette" => {
                let n: usize = f[1].parse().unwrap();
                let colors = id_palette(n).unwrap();
                assert_eq!(id_palette_levels(n).unwrap().to_string(), f[2], "{n} 段");
                assert_eq!(
                    id_palette_separation(n).unwrap().to_string(),
                    f[3],
                    "{n} 離れ"
                );
                let bytes: Vec<u8> = colors.iter().flat_map(|c| c.to_le_bytes()).collect();
                assert_eq!(sha(&bytes), f[4], "{n} 色");
            }
            "colors" => {
                let n: usize = f[1].parse().unwrap();
                let text: Vec<String> = id_palette(n)
                    .unwrap()
                    .iter()
                    .map(|c| format!("{c:06x}"))
                    .collect();
                assert_eq!(text.join(","), f[2]);
            }
            _ => panic!("{line}"),
        }
    }
    assert!(id_palette_levels(256 * 256 * 256 - 255).is_err());
    assert!(id_palette_levels(256 * 256 * 256 - 256).is_ok());
}

#[test]
fn parallelism_is_bounded_and_results_do_not_depend_on_it() {
    // 96×70・余白 6。並列度 4 までは 16 行ずつ（最後は 6 行）、64 以上は 256 行が 1 まとまりで、多くのスレッドが遊ぶ。
    let case = support::cases()
        .into_iter()
        .find(|c| c.name == "rough-large")
        .unwrap();
    let reference = bake_with(&case, 1);
    for threads in [
        2,
        3,
        4,
        7,
        64,
        MAX_PARALLELISM,
        MAX_PARALLELISM + 1,
        100_000,
        usize::MAX,
    ] {
        assert_eq!(
            bake_with(&case, threads).maps,
            reference.maps,
            "並列度 {threads}"
        );
    }
    // 見積もりも丸めた値で数える（桁あふれして予算を素通りしない）
    let budget = |n| MeshBakeBudget {
        max_degree_of_parallelism: n,
        ..Default::default()
    };
    let at = |n| estimate_bytes(&case.input, &case.settings, &budget(n), None).unwrap();
    assert_eq!(at(MAX_PARALLELISM + 1), at(MAX_PARALLELISM));
    assert_eq!(at(usize::MAX), at(MAX_PARALLELISM));
    assert!(at(MAX_PARALLELISM) > at(1));
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    assert_eq!(at(0), at(cores.min(MAX_PARALLELISM)));
    // 極端な値でも、予算が足りなければ準備の前に断る（準備を終えてから生成の失敗を返さない）
    let mut calls = 0;
    let refused = bake(
        &case.input,
        &case.settings,
        &MeshBakeBudget {
            max_bytes: at(1),
            max_degree_of_parallelism: usize::MAX,
            ..Default::default()
        },
        None,
        None,
        |_, _| {
            calls += 1;
            true
        },
    );
    assert_eq!(
        refused.err().unwrap().to_string(),
        "ベイクの見積もりがメモリ予算を超えます"
    );
    assert_eq!(calls, 0);
}
#[test]
fn heights_around_the_row_batches_match_across_parallelism() {
    let low = support::rough(6, 0.12, 31, false);
    let input = support::input(&low, support::Extra::default());
    for height in [1, 2, 15, 16, 17, 31, 32, 33, 63, 64, 65, 130, 257] {
        let mut s = support::settings(40, height);
        s.padding = 2;
        s.ao_samples = 4;
        s.thickness_samples = 4;
        let case = support::Case {
            name: "heights",
            input: input.clone(),
            reference: None,
            settings: s,
        };
        let reference = bake_with(&case, 1);
        for threads in [2, 4, 5, 64, 1000] {
            assert_eq!(
                bake_with(&case, threads).maps,
                reference.maps,
                "高さ {height} 並列度 {threads}"
            );
        }
    }
}

/// 古さの判定が C# の `MeshMapProvenance.Check` と同じ状態・同じ理由の並びになる。
#[test]
fn csharp_freshness_check_states_and_reasons() {
    use support::{input, rough, Extra};
    let expected = expected();
    let moved = input(&rough(10, 0.06, 2, false), Extra::default());
    let other = input(&rough(10, 0.06, 2, true), Extra::default());
    type Change<'a> = Box<dyn Fn(&mut MeshMapExpectation) + 'a>;
    let variants: Vec<(&str, Change)> = vec![
        ("exact", Box::new(|_| {})),
        (
            "no-model",
            Box::new(|e| {
                e.mesh_hash = None;
                e.topology_hash = None;
            }),
        ),
        ("width", Box::new(|e| e.width += 1)),
        ("height", Box::new(|e| e.height *= 2)),
        ("slot", Box::new(|e| e.target_slot = 3)),
        ("outside-model", Box::new(|e| e.target_slot = -2)),
        ("uv1", Box::new(|e| e.uv_channel = 1)),
        (
            "padding",
            Box::new(|e| e.settings.as_mut().unwrap().padding += 1),
        ),
        (
            "antialiasing",
            Box::new(|e| {
                let s = e.settings.as_mut().unwrap();
                s.antialiasing = if s.antialiasing == 1 { 2 } else { 1 };
            }),
        ),
        (
            "ao-samples",
            Box::new(|e| e.settings.as_mut().unwrap().ao_samples = 32),
        ),
        (
            "curvature-radius",
            Box::new(|e| e.settings.as_mut().unwrap().curvature_radius = 0.04),
        ),
        (
            "id-source",
            Box::new(|e| e.settings.as_mut().unwrap().id_source = MeshIdSource::MeshPart),
        ),
        (
            "moved-vertex",
            Box::new(|e| {
                e.mesh_hash = Some(moved.hash().into());
                e.topology_hash = Some(moved.topology_hash().into());
            }),
        ),
        (
            "new-uv",
            Box::new(|e| {
                e.mesh_hash = Some(other.hash().into());
                e.topology_hash = Some(other.topology_hash().into());
            }),
        ),
        ("reference-removed", Box::new(|e| e.reference_hash = None)),
        (
            "reference-changed",
            Box::new(|e| e.reference_hash = Some("1234".into())),
        ),
        (
            "reference-settings",
            Box::new(|e| e.settings.as_mut().unwrap().reference_frontal = 0.02),
        ),
        (
            "reference-no-settings",
            Box::new(|e| {
                e.settings = None;
                e.reference_hash = Some("1234".into());
            }),
        ),
        ("no-settings", Box::new(|e| e.settings = None)),
        (
            "slot-list",
            Box::new(|e| {
                e.target_slot = 0;
                e.target_slots = vec![0, 1];
            }),
        ),
    ];
    let mut checked = 0;
    for case in support::cases() {
        if case.name != "rough-ref-tangents-vertex-cage" && case.name != "rough-self" {
            continue;
        }
        let result = bake_with(&case, 1);
        for map in &result.maps {
            if !matches!(
                map.kind(),
                MeshMapKind::WorldNormal | MeshMapKind::AmbientOcclusion | MeshMapKind::Id
            ) {
                continue;
            }
            let mut run =
                |variant: &str, provenance: &MeshMapProvenance, e: &MeshMapExpectation| {
                    let check = provenance.check(e);
                    let codes: Vec<&str> = check.reasons.iter().map(|r| r.code()).collect();
                    let got = format!(
                        "{:?} {}",
                        check.state,
                        if matches!(check.state, MeshMapState::Stale) {
                            codes.join(",")
                        } else {
                            "-".into()
                        }
                    );
                    let want = expected
                        .checks
                        .iter()
                        .find(|c| {
                            c.0 == case.name && c.1 == format!("{:?}", map.kind()) && c.2 == variant
                        })
                        .unwrap_or_else(|| panic!("{} {:?} {variant}", case.name, map.kind()));
                    assert_eq!(
                        got,
                        format!("{} {}", want.3, want.4),
                        "{} {:?} {variant}",
                        case.name,
                        map.kind()
                    );
                    // 理由は全部、文になる
                    for r in &check.reasons {
                        assert!(!r.to_string().is_empty());
                    }
                    checked += 1;
                };
            for (name, change) in &variants {
                let mut e = MeshMapExpectation::for_bake(
                    &case.input,
                    &case.settings,
                    case.reference.as_ref(),
                );
                change(&mut e);
                run(name, map.provenance(), &e);
            }
            let e =
                MeshMapExpectation::for_bake(&case.input, &case.settings, case.reference.as_ref());
            let mut p = map.provenance().clone();
            p.engine_version = 1;
            run("engine", &p, &e);
            let mut p = map.provenance().clone();
            p.space = "Other".into();
            run("space", &p, &e);
            let mut p = map.provenance().clone();
            p.pose = "Animated".into();
            run("pose", &p, &e);
        }
    }
    assert_eq!(checked, expected.checks.len(), "C# の事例を全部通ったこと");
}
