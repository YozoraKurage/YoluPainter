//! GPU ベイクを CPU（`yolu_core::mesh_maps::bake`）と照らす。全バイト一致は求めない（f32 と f64、UV の覆いの境の判定、レイの当たり外れ）。
//! アダプターが無いときは理由を標準エラーへ出して終わる（Rust の集計では passed なので `--nocapture` の出力を確かめる）。
//! 環境変数 `YOLUPAINTER_REQUIRE_GPU` を設定すると、スキップせず失敗にする（アダプターのある環境で試験が実行されたことの確認用）。
#[path = "support/gpu_lease.rs"]
mod gpu_lease;
#[path = "support/meshes.rs"]
mod meshes;
use meshes::*;
use std::sync::atomic::AtomicBool;
use yolu_core::mesh_maps::{
    bake, id_part_binding, BakedMeshMap, IdColorAssignments, MeshBakeBudget, MeshBakeInput,
    MeshBakeSettings, MeshBakeStatus, MeshIdSource, MeshMapKind, MeshOccluders,
};
use yolu_gpu::{bake_mesh_maps, BakeBackend, BakeGpu, GpuBakeError, GpuBakeOptions, GpuBakeSlot};

/// アダプターが無くて試験を飛ばす。`YOLUPAINTER_REQUIRE_GPU` があれば、飛ばさず失敗にする。
fn skipped(why: &str) {
    eprintln!("GPU ベイク試験をスキップ: {why}");
    assert!(
        std::env::var_os("YOLUPAINTER_REQUIRE_GPU").is_none(),
        "YOLUPAINTER_REQUIRE_GPU があるのに GPU を使えない: {why}"
    );
}

fn gpu() -> Option<BakeGpu> {
    gpu_lease::lease();
    match BakeGpu::new(GpuBakeOptions {
        allow_software: true,
        ray_query: false,
        ..Default::default()
    }) {
        Ok(g) => {
            eprintln!("GPU ベイク試験: {}", g.adapter());
            Some(g)
        }
        Err(GpuBakeError::Unavailable(e)) => {
            assert!(e.starts_with("GPU 利用不可:"), "{e}");
            skipped(&e);
            None
        }
        Err(e) => panic!("GPU の初期化に失敗: {e}"),
    }
}

/// ray query の道を試すアダプター。ray query の機能つきでデバイスを作れたときだけ返す（ソフトウェアの描画は対応しないので、
/// 使えなければ理由を出して飛ばす）。`YOLUPAINTER_REQUIRE_RAY_QUERY` があれば、飛ばさず失敗にする（ray query のあるアダプターで
/// 試験が実行されたことの確認用）。
fn gpu_ray_query() -> Option<BakeGpu> {
    gpu_lease::lease();
    let why = match BakeGpu::new(GpuBakeOptions {
        allow_software: true,
        ray_query: true,
        ..Default::default()
    }) {
        Ok(g) if g.adapter().ray_query => {
            eprintln!("ray query の試験: {}", g.adapter());
            return Some(g);
        }
        Ok(g) => format!("{} は ray query を使えない", g.adapter()),
        Err(e) => e.to_string(),
    };
    eprintln!("ray query の試験をスキップ: {why}");
    assert!(
        std::env::var_os("YOLUPAINTER_REQUIRE_RAY_QUERY").is_none(),
        "YOLUPAINTER_REQUIRE_RAY_QUERY があるのに ray query を使えない: {why}"
    );
    None
}

/// compute の道と、使えるなら ray query の道（同じ許容で CPU と照らす）。
struct Gpus {
    compute: BakeGpu,
    ray_query: Option<BakeGpu>,
}
fn gpus() -> Option<Gpus> {
    let compute = gpu()?;
    Some(Gpus {
        compute,
        ray_query: gpu_ray_query(),
    })
}
fn run_all(
    g: &mut Gpus,
    input: &MeshBakeInput,
    reference: Option<&MeshBakeInput>,
    settings: &MeshBakeSettings,
) -> Vec<(MeshMapKind, Diff)> {
    let out = run(&mut g.compute, input, reference, settings, false);
    if let Some(rq) = &mut g.ray_query {
        eprintln!("  -- ray query の道 --");
        let _ = run(rq, input, reference, settings, true);
    }
    out
}

/// マップ 1 枚の CPU との差。値は 16 bit（0〜65535）。
#[derive(Debug, Default)]
pub struct Diff {
    pub max: u32,
    pub mean: f64,
    /// 差が 0.5%（328）を超えるテクセルの割合（覆うテクセルのうち）。
    pub over_half_percent: f64,
    /// 差が 5%（3277）を超えるテクセルの割合。
    pub over_five_percent: f64,
    pub covered: usize,
}
fn diff(cpu: &BakedMeshMap, gpu: &BakedMeshMap) -> Diff {
    let ch = cpu.channels();
    let mut d = Diff::default();
    let mut sum = 0f64;
    let (mut a, mut b) = (0usize, 0usize);
    for i in 0..cpu.coverage().len() {
        if cpu.coverage()[i] == 0 || gpu.coverage()[i] == 0 {
            continue;
        }
        d.covered += 1;
        let mut worst = 0u32;
        for c in 0..ch {
            let x = cpu.data()[i * ch + c];
            let y = gpu.data()[i * ch + c];
            let e = u32::from(x.abs_diff(y));
            sum += f64::from(e);
            worst = worst.max(e);
        }
        d.max = d.max.max(worst);
        a += usize::from(worst > 328);
        b += usize::from(worst > 3277);
    }
    d.mean = sum / (d.covered * ch).max(1) as f64;
    d.over_half_percent = a as f64 / d.covered.max(1) as f64;
    d.over_five_percent = b as f64 / d.covered.max(1) as f64;
    d
}
/// 許し幅（16 bit の値で数える）。根拠は README の「CPU との差」。
/// - 高ポリへの投影なし: 光線を飛ばさないマップは f32 の丸めだけなので最大 8（計測は 1）、光線のマップは f32 と f64 でレイの当たり外れが
///   変わるサンプルがあるので、0.5% を超えるテクセルは 0.2% 以下、5% を超えるテクセルは 0.05% 以下、最大は 6%（計測は最大 2.8%・0.012%・0）。
///   ID・不透明度は値が合う。
/// - 高ポリへの投影あり: 投影の光線が当たるか外れるかが f32 と f64 で分かれるテクセルは別の面（または外れ）の値になり、差は 0 と 1 や別の
///   部品の色まで大きくなる（69312 面の高ポリ・2048² で 5 / 約 400 万テクセル）。差の大きさは抑えず、0.5% を超えるテクセルの割合を 0.2% 以下、
///   平均を 2 以下に抑える。
fn check_tolerance(kind: MeshMapKind, d: &Diff, projected: bool) {
    use MeshMapKind::*;
    if projected {
        assert!(
            d.over_half_percent <= 0.002,
            "{kind:?} の 0.5% 超 {}",
            d.over_half_percent
        );
        assert!(d.mean <= 2.0, "{kind:?} の平均差 {}", d.mean);
        return;
    }
    match kind {
        AmbientOcclusion | Thickness | BentNormal => {
            assert!(d.max <= 3932, "{kind:?} の最大差 {} が 6% を超える", d.max);
            assert!(
                d.over_half_percent <= 0.002,
                "{kind:?} の 0.5% 超 {}",
                d.over_half_percent
            );
            assert!(
                d.over_five_percent <= 0.0005,
                "{kind:?} の 5% 超 {}",
                d.over_five_percent
            );
        }
        Id | Opacity => assert_eq!(d.max, 0, "{kind:?} は値が合う"),
        _ => assert!(d.max <= 8, "{kind:?} の最大差 {} が 8 を超える", d.max),
    }
}
fn coverage_mismatch(cpu: &BakedMeshMap, gpu: &BakedMeshMap) -> (usize, usize) {
    let mut wrong = 0;
    let mut covered = 0;
    for (a, b) in cpu.coverage().iter().zip(gpu.coverage()) {
        covered += usize::from(*a != 0);
        wrong += usize::from(a != b);
    }
    (wrong, covered)
}

fn run(
    g: &mut BakeGpu,
    input: &MeshBakeInput,
    reference: Option<&MeshBakeInput>,
    settings: &MeshBakeSettings,
    expect_ray_query: bool,
) -> Vec<(MeshMapKind, Diff)> {
    let budget = MeshBakeBudget::default();
    let cpu = bake(input, settings, &budget, None, reference, |_, _| true).unwrap();
    let baked = g
        .bake(input, settings, &budget, None, reference, |_, _| true)
        .unwrap_or_else(|e| panic!("GPU ベイクに失敗: {e}"));
    if expect_ray_query {
        // 自己照合に通らず compute に戻ったのでは、ray query の道を試したことにならない
        assert_eq!(
            baked.stats.method,
            yolu_gpu::GpuBakeMethod::RayQuery,
            "ray query の道を使えなかった: {:?}",
            baked.stats.ray_query_note
        );
        eprintln!("  {}", baked.stats.ray_query_note.as_deref().unwrap_or(""));
    }
    let gpu = baked.result;
    assert_eq!(gpu.status, MeshBakeStatus::Completed);
    assert_eq!(gpu.maps.len(), cpu.maps.len());
    let (wrong, covered) = coverage_mismatch(&cpu.maps[0], &gpu.maps[0]);
    eprintln!(
        "  覆い: 食い違い {wrong} / {covered}（{:.4}%）、dispatch {} 回、最長 {:.1} ms、帯 {}、入力 {} KiB",
        wrong as f64 / covered.max(1) as f64 * 100.0,
        baked.stats.dispatches,
        baked.stats.max_dispatch_ms,
        baked.stats.bands,
        baked.stats.input_bytes >> 10,
    );
    assert!(
        wrong as f64 <= covered as f64 * 0.001,
        "覆いの食い違いが多い: {wrong} / {covered}"
    );
    cpu.maps
        .iter()
        .zip(&gpu.maps)
        .map(|(c, g)| {
            assert_eq!(c.provenance(), g.provenance());
            let d = diff(c, g);
            check_tolerance(c.kind(), &d, reference.is_some());
            eprintln!(
                "  {:>16}: 最大差 {:>5}（{:.3}%）、平均 {:.2}、0.5% 超 {:.4}%、5% 超 {:.4}%",
                c.kind().name(),
                d.max,
                d.max as f64 / 655.35,
                d.mean,
                d.over_half_percent * 100.0,
                d.over_five_percent * 100.0
            );
            (c.kind(), d)
        })
        .collect()
}

#[test]
fn self_bake_all_maps_on_a_cube() {
    let Some(mut g) = gpus() else { return };
    let c = skewed(cube(6, 0.0, 0.0, 1));
    let input = input_plain(&c);
    let settings = MeshBakeSettings {
        width: 96,
        height: 64,
        target_slot: -1,
        padding: 0,
        maps: MeshMapKind::ALL.to_vec(),
        ao_samples: 16,
        thickness_samples: 16,
        ..Default::default()
    };
    eprintln!("立方体 {} 面・自己ベイク", input.triangle_count());
    let _ = run_all(&mut g, &input, None, &settings);
}

fn all_maps(width: i32, height: i32) -> MeshBakeSettings {
    MeshBakeSettings {
        width,
        height,
        target_slot: -1,
        padding: 4,
        antialiasing: 2,
        maps: MeshMapKind::ALL.to_vec(),
        ao_samples: 32,
        thickness_samples: 32,
        curvature_radius: 0.05,
        ..Default::default()
    }
}

#[test]
fn noisy_bulged_cube_with_vertex_normals_and_antialiasing() {
    let Some(mut g) = gpus() else { return };
    let c = skewed(cube(10, 0.6, 0.08, 7));
    let input = input_with_normals(&c);
    eprintln!(
        "凹凸のある立方体 {} 面・頂点法線・AA 2・余白 4",
        input.triangle_count()
    );
    let _ = run_all(&mut g, &input, None, &all_maps(128, 128));
}

#[test]
fn projection_from_a_displaced_high_poly() {
    let Some(mut g) = gpus() else { return };
    let low = input_with_normals(&skewed(cube(4, 0.8, 0.0, 3)));
    let high = input_with_normals(&skewed(cube(16, 0.8, 0.06, 5)));
    let mut s = all_maps(128, 128);
    s.antialiasing = 1;
    s.reference_frontal = 0.1;
    s.reference_rear = 0.1;
    eprintln!(
        "低ポリ {} 面 ← 高ポリ {} 面",
        low.triangle_count(),
        high.triangle_count()
    );
    let _ = run_all(&mut g, &low, Some(&high), &s);
}

fn names(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(|s| s.to_string()).collect())
}

#[test]
fn projection_matched_by_name_with_an_unmatched_part() {
    let Some(mut g) = gpu() else { return };
    let low = input_with(
        &skewed(cube(4, 0.8, 0.0, 3)),
        &Extras {
            normals: true,
            renderers: Some([0, 0, 1, 1, 2, 2]),
            renderer_names: names(&["Part_A_low", "Part_B_low", "Part_C_low"]),
            ..Default::default()
        },
    );
    let high = input_with(
        &skewed(cube(12, 0.8, 0.05, 5)),
        &Extras {
            normals: true,
            renderers: Some([0, 0, 1, 1, 0, 0]),
            renderer_names: names(&["part_a_HIGH", "Part_B_high"]),
            ..Default::default()
        },
    );
    let mut s = all_maps(96, 64);
    s.antialiasing = 1;
    s.reference_match_by_name = true;
    s.reference_frontal = 0.1;
    s.reference_rear = 0.1;
    eprintln!("名前の対応（C は高ポリに無い）");
    let _ = run(&mut g, &low, Some(&high), &s, false);
}

#[test]
fn id_sources_manual_colors_and_vertex_colors() {
    let Some(mut g) = gpu() else { return };
    let c = skewed(cube(5, 0.5, 0.04, 9));
    let extras = Extras {
        normals: true,
        renderers: Some([0, 0, 1, 1, 2, 2]),
        renderer_names: names(&["A_low", "B_low", "C_low"]),
        colors: true,
        material_keys: Some(["m0", "m0", "m1", "m2", "m2", "m2"]),
    };
    let low = input_with(&c, &extras);
    let high = input_with(
        &skewed(cube(10, 0.5, 0.05, 11)),
        &Extras {
            normals: true,
            renderers: Some([0, 1, 2, 0, 1, 2]),
            renderer_names: names(&["A_high", "B_high", "C_high"]),
            colors: true,
            material_keys: Some(["m1", "m1", "m0", "m2", "m3", "m3"]),
        },
    );
    for source in [
        MeshIdSource::MaterialSlot,
        MeshIdSource::Mesh,
        MeshIdSource::UvIsland,
        MeshIdSource::MeshPart,
        MeshIdSource::VertexColor,
        MeshIdSource::MaterialAsset,
    ] {
        for reference in [None, Some(&high)] {
            let mut s = all_maps(64, 64);
            s.maps = vec![MeshMapKind::Id, MeshMapKind::Opacity];
            s.id_source = source;
            s.antialiasing = 3;
            s.reference_frontal = 0.15;
            s.reference_rear = 0.15;
            eprintln!("ID {source:?} 高ポリ {}", reference.is_some());
            let diffs = run(&mut g, &low, reference, &s, false);
            if reference.is_none() {
                assert_eq!(diffs[0].1.max, 0, "ID は値が合う");
            }
        }
    }
    // 手動の ID 色
    let (parts, binding) = id_part_binding(&low);
    assert!(parts.iter().max().unwrap() >= &3);
    let manual = IdColorAssignments::new(
        binding,
        [(0usize, 0xff0000u32), (2, 0x00ff00)].into_iter().collect(),
    )
    .unwrap();
    let mut s = all_maps(64, 64);
    s.maps = vec![MeshMapKind::Id];
    s.id_source = MeshIdSource::MeshPart;
    s.manual_id_colors = manual;
    eprintln!("手動の ID 色");
    let diffs = run(&mut g, &low, None, &s, false);
    assert_eq!(diffs[0].1.max, 0);
}

#[test]
fn several_slots_occluders_and_odd_sizes() {
    let Some(mut g) = gpus() else { return };
    let low = input_with(
        &skewed(cube(6, 0.4, 0.05, 13)),
        &Extras {
            normals: true,
            ..Default::default()
        },
    );
    for (slots, occluders, size) in [
        (vec![0, 2, 4], MeshOccluders::TargetSlotOnly, (100, 37)),
        (vec![1], MeshOccluders::WholeModel, (33, 71)),
        (vec![3, 5], MeshOccluders::TargetSlotOnly, (9, 130)),
    ] {
        let mut s = all_maps(size.0, size.1);
        s.target_slot = slots[0];
        s.target_slots = slots.clone();
        s.occluders = occluders;
        s.antialiasing = 4;
        s.padding = 8;
        s.ao_samples = 24;
        s.thickness_samples = 24;
        eprintln!(
            "スロット {slots:?} {occluders:?} {}x{} AA 4",
            size.0, size.1
        );
        let _ = run_all(&mut g, &low, None, &s);
    }
}

/// 全マップの出力が一様でないこと（比べる相手も自分も一様な試験にならないように）。
fn distinct(map: &BakedMeshMap) -> usize {
    let mut v: Vec<u16> = map
        .data()
        .iter()
        .enumerate()
        .filter(|(i, _)| map.coverage()[i / map.channels()] != 0)
        .map(|(_, v)| *v)
        .collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

#[test]
fn maps_are_not_uniform() {
    let Some(mut g) = gpu() else { return };
    let low = input_with_normals(&skewed(cube(4, 0.8, 0.0, 3)));
    let high = input_with_normals(&skewed(cube(16, 0.8, 0.06, 5)));
    let mut s = all_maps(128, 128);
    s.antialiasing = 1;
    s.reference_frontal = 0.1;
    s.reference_rear = 0.1;
    let baked = g
        .bake(
            &low,
            &s,
            &MeshBakeBudget::default(),
            None,
            Some(&high),
            |_, _| true,
        )
        .unwrap();
    for m in &baked.result.maps {
        let n = distinct(m);
        eprintln!("{:?}: 値の種類 {n}", m.kind());
        match m.kind() {
            MeshMapKind::Opacity => {}
            // 6 つのスロットの色（3 チャンネルの値が重なって 3〜18 種類）
            MeshMapKind::Id => assert!(n >= 3, "ID が一様（{n} 種類）"),
            kind => assert!(n > 20, "{kind:?} が一様に近い（{n} 種類）"),
        }
    }
    assert!(baked.result.report.rays > 0 && baked.result.report.projected_samples > 0);
}

fn gpu_with(options: GpuBakeOptions) -> Option<BakeGpu> {
    gpu_lease::lease();
    match BakeGpu::new(GpuBakeOptions {
        allow_software: true,
        ray_query: false,
        ..options
    }) {
        Ok(g) => Some(g),
        Err(e) => {
            skipped(&e.to_string());
            None
        }
    }
}
fn same(a: &yolu_core::mesh_maps::MeshBakeResult, b: &yolu_core::mesh_maps::MeshBakeResult) {
    assert_eq!(a.maps.len(), b.maps.len());
    for (x, y) in a.maps.iter().zip(&b.maps) {
        assert_eq!(x.data(), y.data(), "{:?} の値", x.kind());
        assert_eq!(x.coverage(), y.coverage(), "{:?} の覆い", x.kind());
    }
    assert_eq!(a.report.rays, b.report.rays);
    assert_eq!(a.report.projected_samples, b.report.projected_samples);
    assert_eq!(a.report.missed_samples, b.report.missed_samples);
}

#[test]
fn bands_and_dispatch_sizes_do_not_change_the_result() {
    // 1 帯の側は dispatch を時間で絞らせない（負荷で dispatch の数が揺れて、下の比べが崩れないように）
    let Some(mut whole) = gpu_with(GpuBakeOptions {
        target_dispatch_ms: 1e9,
        ..Default::default()
    }) else {
        return;
    };
    let low = input_with_normals(&skewed(cube(5, 0.6, 0.04, 21)));
    let high = input_with_normals(&skewed(cube(12, 0.6, 0.05, 23)));
    let mut s = all_maps(100, 75);
    s.reference_frontal = 0.1;
    s.reference_rear = 0.1;
    let budget = MeshBakeBudget::default();
    let one = whole
        .bake(&low, &s, &budget, None, Some(&high), |_, _| true)
        .unwrap();
    assert_eq!(one.stats.bands, 1);
    // 出力 + 読み戻しが 3 行しか入らない予算
    let out_stride: u64 = s.maps.iter().map(|k| k.channels() as u64).sum();
    let row = (out_stride + 1) * 4 * 2 * 100;
    let Some(mut banded) = gpu_with(GpuBakeOptions {
        budget_bytes: one.stats.input_bytes + row * 3 + 100,
        // 1 回の dispatch も最小まで小さくさせる
        target_dispatch_ms: 0.000_01,
        ..Default::default()
    }) else {
        return;
    };
    let many = banded
        .bake(&low, &s, &budget, None, Some(&high), |_, _| true)
        .unwrap();
    eprintln!(
        "1 帯 {} 回 / 帯 {}・dispatch {} 回（最長 {:.1} ms）",
        one.stats.dispatches, many.stats.bands, many.stats.dispatches, many.stats.max_dispatch_ms
    );
    assert_eq!(many.stats.bands, 25, "75 行を 3 行ずつ");
    assert!(many.stats.dispatches > one.stats.dispatches * 5);
    same(&one.result, &many.result);
}

#[test]
fn cancel_time_limit_and_refusals_leave_nothing() {
    let Some(mut g) = gpu() else { return };
    let low = input_with_normals(&skewed(cube(5, 0.6, 0.04, 21)));
    let s = all_maps(128, 128);
    let budget = MeshBakeBudget::default();
    // 取消: 2 回目の "Baking" の通知で止める（CPU の bake と同じく、空の結果と Canceled）
    let mut calls = 0;
    let baked = g
        .bake(&low, &s, &budget, None, None, |_, phase| {
            if phase == "Baking" {
                calls += 1;
            }
            calls < 3
        })
        .unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::Canceled);
    assert!(baked.result.maps.is_empty());
    // 旗での取消（旗が立ったまま始めるので、準備の最初の確認で止まり、GPU は動かない。GPU が動いている間の取消は下の別の試験）
    let flag = AtomicBool::new(true);
    let baked = g
        .bake(&low, &s, &budget, Some(&flag), None, |_, _| true)
        .unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::Canceled);
    assert!(baked.result.maps.is_empty());
    // 時間切れ（始めた時点で過ぎているので、準備の最初の確認で止まり、GPU は動かない。動いている間の時間切れは下の別の試験）
    let limited = MeshBakeBudget {
        max_seconds: 1e-9,
        ..Default::default()
    };
    let baked = g.bake(&low, &s, &limited, None, None, |_, _| true).unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::TimedOut);
    assert!(baked.result.maps.is_empty());
    // 断る入力は CPU と同じ理由（Refused）。GPU は失敗扱いにならず、続けて使える
    let mut outside = skewed(cube(2, 0.0, 0.0, 1));
    outside.uvs[0] = 2.0;
    let bad = input_plain(&outside);
    let cpu_error = bake(&bad, &s, &budget, None, None, |_, _| true)
        .err()
        .unwrap()
        .to_string();
    match g.bake(&bad, &s, &budget, None, None, |_, _| true) {
        Err(GpuBakeError::Refused(e)) => assert_eq!(e.to_string(), cpu_error),
        other => panic!("断られるはず: {:?}", other.map(|b| b.stats)),
    }
    assert!(g.failure().is_none());
    assert!(g.bake(&low, &s, &budget, None, None, |_, _| true).is_ok());
}

#[test]
fn budget_too_small_is_reported_with_the_reason() {
    let low = input_with_normals(&skewed(cube(5, 0.6, 0.04, 21)));
    let s = all_maps(128, 128);
    let Some(mut tiny) = gpu_with(GpuBakeOptions {
        budget_bytes: 4096,
        ..Default::default()
    }) else {
        return;
    };
    match tiny.bake(&low, &s, &MeshBakeBudget::default(), None, None, |_, _| {
        true
    }) {
        Err(GpuBakeError::Budget(why)) => eprintln!("予算の拒否: {why}"),
        other => panic!("予算で断るはず: {:?}", other.map(|b| b.stats)),
    }
    assert!(tiny.failure().is_none(), "予算の拒否は GPU の故障ではない");
}

#[test]
fn uv_grid_on_exact_ties_differs_no_more_than_the_cpu_does_from_itself() {
    // テクセルの中心が四角形の対角線（2 つの三角形の共有する辺）の上に乗る UV。辺の上のサンプルは、どちらの三角形の面の
    // 法線を使うかで AO のレイが 1〜2 本変わる。CPU 同士でも UV を数 ulp ずらすだけで同じ程度の食い違いが出ることを基準にする。
    let Some(mut g) = gpu() else { return };
    let ex = Extras {
        normals: true,
        ..Default::default()
    };
    let a = input_with(&cube(4, 0.8, 0.0, 3), &ex);
    let mut shifted = cube(4, 0.8, 0.0, 3);
    for v in shifted.uvs.iter_mut() {
        *v += 1.0e-7 * (*v + 0.1);
    }
    let b = input_with(&shifted, &ex);
    let mut s = all_maps(96, 64);
    s.antialiasing = 1;
    s.padding = 0;
    s.maps = vec![MeshMapKind::AmbientOcclusion];
    let budget = MeshBakeBudget::default();
    let count = |x: &BakedMeshMap, y: &BakedMeshMap| {
        x.data()
            .iter()
            .zip(y.data())
            .filter(|(p, q)| p.abs_diff(**q) > 1000)
            .count()
    };
    let cpu = bake(&a, &s, &budget, None, None, |_, _| true).unwrap();
    let cpu_shifted = bake(&b, &s, &budget, None, None, |_, _| true).unwrap();
    let gpu = g
        .bake(&a, &s, &budget, None, None, |_, _| true)
        .unwrap()
        .result;
    let baseline = count(&cpu.maps[0], &cpu_shifted.maps[0]);
    let measured = count(&cpu.maps[0], &gpu.maps[0]);
    eprintln!("対角線上の AO（1.5% 超の差）: CPU 同士（UV を数 ulp ずらす）{baseline} / GPU と CPU {measured}（6144 テクセル）");
    assert!(baseline > 0, "この入力は辺の上のサンプルで揺れるはず");
    assert!(
        measured <= baseline * 2 + 8,
        "GPU の食い違い {measured} が CPU 自身の揺れ {baseline} の範囲を超える"
    );
}

#[test]
fn auto_falls_back_to_cpu_with_a_reason_and_the_gpu_mode_uses_the_software_adapter() {
    let low = input_with_normals(&skewed(cube(4, 0.8, 0.0, 3)));
    let mut s = all_maps(64, 64);
    s.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion];
    s.ao_samples = 8;
    let budget = MeshBakeBudget::default();
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        ray_query: false,
        ..Default::default()
    });
    let cpu = bake(&low, &s, &budget, None, None, |_, _| true).unwrap();
    // CPU を選ぶ
    let (r, run) = bake_mesh_maps(
        BakeBackend::Cpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(!run.used_gpu() && run.fallback_reason.is_none());
    same(&cpu, &r);
    // このコンテナにはソフトウェアのアダプターしか無い。自動では使わず、理由つきで CPU（結果は CPU の bake と全バイト同じ）
    let probe = slot.probe(false);
    let (r, run) = bake_mesh_maps(
        BakeBackend::Auto,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    match probe {
        Err(why) => {
            assert!(!run.used_gpu());
            let reason = run.fallback_reason.clone().expect("理由が要る");
            assert_eq!(reason, why);
            eprintln!("自動 → CPU の理由: {reason}");
            same(&cpu, &r);
        }
        Ok(a) => eprintln!("この環境にはハードウェアの GPU がある: {a}"),
    }
    // 「GPU」はソフトウェアの描画も許す
    let adapter = match slot.probe(true) {
        Ok(a) => a,
        Err(why) => {
            skipped(&why);
            return;
        }
    };
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(
        run.used_gpu() && run.fallback_reason.is_none(),
        "{:?}",
        run.fallback_reason
    );
    assert_eq!(run.gpu.as_ref().unwrap().0, adapter);
    assert_eq!(r.maps.len(), 2);
    // GPU が壊れたら理由つきで CPU に戻り、次は作り直す
    slot.fail_for_test("試験の故障");
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(!run.used_gpu());
    assert!(
        run.fallback_reason
            .as_deref()
            .unwrap()
            .contains("試験の故障"),
        "{:?}",
        run.fallback_reason
    );
    same(&cpu, &r);
    let (_, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(
        run.used_gpu(),
        "作り直して GPU に戻る: {:?}",
        run.fallback_reason
    );
    // CPU も断る入力はそのまま Err（CPU へ戻して同じ断りを繰り返さない）
    let mut outside = skewed(cube(2, 0.0, 0.0, 1));
    outside.uvs[0] = 2.0;
    let bad = input_plain(&outside);
    assert!(bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &bad,
        &s,
        &budget,
        None,
        None,
        |_, _| true
    )
    .is_err());
}

#[test]
fn a_gpu_budget_overflow_falls_back_to_the_cpu_with_the_reason() {
    let low = input_with_normals(&skewed(cube(4, 0.8, 0.0, 3)));
    let mut s = all_maps(64, 64);
    s.maps = vec![MeshMapKind::WorldNormal];
    let budget = MeshBakeBudget::default();
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        budget_bytes: 4096,
        ray_query: false,
        ..Default::default()
    });
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    if let Err(why) = slot.probe(true) {
        skipped(&why);
        return;
    }
    assert!(!run.used_gpu());
    let reason = run.fallback_reason.unwrap();
    assert!(reason.contains("予算"), "{reason}");
    same(
        &bake(&low, &s, &budget, None, None, |_, _| true).unwrap(),
        &r,
    );
}

#[test]
fn a_dispatch_never_exceeds_the_workgroup_limit_even_when_the_chunk_has_grown_to_the_most() {
    // 光線の要らないマップは 1 テクセル 1 回で軽いので、dispatch の大きさが上限まで育つ。4096² は 1 つの帯が 2^22 テクセルを超え、
    // 育った dispatch が 1 次元のワークグループ数の上限（65535）を超える大きさに届く。目標の時間を大きくして確実に育てる。
    let Some(mut g) = gpu_with(GpuBakeOptions {
        target_dispatch_ms: 1e9,
        ..Default::default()
    }) else {
        return;
    };
    let limit = g.max_workgroups_per_dimension();
    let low = input_plain(&skewed(cube(4, 0.8, 0.0, 3)));
    let mut s = all_maps(4096, 4096);
    s.maps = vec![MeshMapKind::Opacity];
    s.antialiasing = 1;
    s.padding = 0;
    let budget = MeshBakeBudget::default();
    let baked = g
        .bake(&low, &s, &budget, None, None, |_, _| true)
        .unwrap_or_else(|e| panic!("GPU ベイクに失敗: {e}"));
    let texels = 4096u64 * 4096;
    eprintln!(
        "4096² 光線なし: 帯 {}、dispatch {} 回、最大 {} テクセル（ワークグループ {}、上限 {limit}）、最長 {:.1} ms",
        baked.stats.bands,
        baked.stats.dispatches,
        baked.stats.max_dispatch_texels,
        baked.stats.max_dispatch_texels.div_ceil(64),
        baked.stats.max_dispatch_ms
    );
    assert_eq!(baked.stats.bands, 1);
    assert!(
        u64::from(baked.stats.max_dispatch_texels) * 2 > 1 << 22 && texels > 1 << 22,
        "育った dispatch が 2^21 を超えるはず（この試験の前提）: {}",
        baked.stats.max_dispatch_texels
    );
    assert!(
        baked.stats.max_dispatch_texels.div_ceil(64) <= limit,
        "dispatch が上限を超えた"
    );
    // 値も CPU と合う（不透明度は値が合い、覆いの食い違いは境の判定だけ）
    let cpu = bake(&low, &s, &budget, None, None, |_, _| true).unwrap();
    let (wrong, covered) = coverage_mismatch(&cpu.maps[0], &baked.result.maps[0]);
    assert!(
        wrong as f64 <= covered as f64 * 0.001,
        "{wrong} / {covered}"
    );
    assert_eq!(diff(&cpu.maps[0], &baked.result.maps[0]).max, 0);
}

/// CPU（f64）は焼けるが、GPU（f32）では位置の正規化 `(p - min) * scale` が桁あふれする大きな座標（f32 の最大は約 3.4e38）。
fn huge_cube() -> MeshBakeInput {
    let mut c = skewed(cube(3, 0.3, 0.0, 5));
    for v in c.corners.iter_mut() {
        *v *= 3.0e38;
    }
    input_plain(&c)
}

#[test]
fn a_nan_or_infinite_value_fails_the_gpu_run_instead_of_being_stored_as_zero() {
    let Some(mut g) = gpu() else { return };
    let input = huge_cube();
    let mut s = all_maps(32, 32);
    s.maps = vec![MeshMapKind::Position];
    s.antialiasing = 1;
    s.padding = 0;
    let budget = MeshBakeBudget::default();
    // CPU は f64 なので焼ける（値は 0〜1 に収まる）
    let cpu = bake(&input, &s, &budget, None, None, |_, _| true).unwrap();
    assert_eq!(cpu.status, MeshBakeStatus::Completed);
    // 座標が大きすぎるモデルは、GPU が NaN・無限大を作るかどうか（GPU ごとに違う）に頼らず、焼く前に断る
    match g.bake(&input, &s, &budget, None, None, |_, _| true) {
        Err(GpuBakeError::Failed(why)) => assert!(why.contains("NaN"), "{why}"),
        other => panic!("座標が大きすぎるので断るはず: {:?}", other.map(|b| b.stats)),
    }
    // 使う側（自動）は理由つきで CPU に戻り、結果は CPU の bake と同じ
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        ray_query: false,
        ..Default::default()
    });
    if let Err(why) = slot.probe(true) {
        skipped(&why);
        return;
    }
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &input,
        &s,
        &budget,
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    assert!(!run.used_gpu());
    assert_eq!(run.fallback_kind, Some(yolu_gpu::FallbackKind::Failed));
    assert!(run.fallback_reason.unwrap().contains("NaN"));
    same(&cpu, &r);
}

#[test]
fn the_self_check_answers_of_the_compute_path_are_well_formed() {
    // ray query の道が合わせるべき答え。この環境では ray query を動かせないので、答えの並び（8 ワード）・値の範囲・両方の性質のレイに
    // 当たりがあることを compute の道で確かめる（並びが狂えば、実機の自己照合は毎回落ちるか、見るべき値を見なくなる）。
    let Some(mut g) = gpu() else { return };
    g.probe_selfcheck_for_test();
    let low = input_with_normals(&skewed(cube(6, 0.6, 0.04, 21)));
    let triangles = low.triangle_count() as u32;
    let mut s = all_maps(32, 32);
    s.maps = vec![MeshMapKind::AmbientOcclusion];
    g.bake(&low, &s, &MeshBakeBudget::default(), None, None, |_, _| {
        true
    })
    .unwrap();
    let words = g.selfcheck_for_test().expect("飛ばしたはず");
    assert_eq!(words.len(), 4096 * 8);
    let (mut plain, mut culled, mut any_hit, mut uv_differs) = (0, 0, 0, 0);
    for (i, w) in words.as_chunks::<8>().0.iter().enumerate() {
        assert_eq!(w[5..], [0, 0, 0], "ray {i}: 使わないワードは 0");
        if w[0] == 0 {
            assert!(w[1..5].iter().all(|v| *v == 0), "ray {i}: 外れは 0");
            continue;
        }
        let f = |k: usize| f32::from_bits(w[k]);
        assert!(f(1) > 0.0 && f(1).is_finite(), "ray {i}: t {}", f(1));
        assert!(w[2] < triangles, "ray {i}: 三角形 {}", w[2]);
        assert!(
            (0.0..=1.0).contains(&f(3)) && (0.0..=1.0).contains(&f(4)) && f(3) + f(4) <= 1.0 + 1e-3,
            "ray {i}: u {} v {}",
            f(3),
            f(4)
        );
        if i & 4 != 0 {
            culled += 1;
        } else {
            plain += 1;
        }
        any_hit += usize::from(i & 8 != 0);
        uv_differs += usize::from((f(3) - f(4)).abs() > 1e-3);
    }
    eprintln!("自己照合: 当たり {plain} + 裏面を飛ばす {culled}（最初の当たりで止める {any_hit}）、u と v が違う {uv_differs}");
    // どの性質のレイにも当たりがある。始点の三角形に戻るレイは重心（u = v = 1/3）に当たるので、u と v が違う当たり（入れ替えを
    // 見分けられる当たり）はほかの三角形に当たったレイだけ
    assert!(plain > 100 && culled > 100 && any_hit > 100);
    assert!(uv_differs > 300, "{uv_differs}");
}

/// dispatch を最小まで小さくさせて、帯の中の確認の回数を増やす（"Baking" の通知は、焼き始め + dispatch ごと）。
fn many_small_dispatches() -> GpuBakeOptions {
    GpuBakeOptions {
        target_dispatch_ms: 1e-9,
        ..Default::default()
    }
}

#[test]
fn a_cancel_or_time_limit_while_the_gpu_is_running_returns_an_empty_result_and_leaves_the_device_usable(
) {
    let Some(mut g) = gpu_with(many_small_dispatches()) else {
        return;
    };
    let low = input_with_normals(&skewed(cube(5, 0.6, 0.04, 21)));
    let s = all_maps(128, 128);
    let budget = MeshBakeBudget::default();
    // 旗: 3 回目の "Baking" の通知（2 回目の dispatch のあと）で立てる。次の確認（3 回目の dispatch のあと）で止まる
    let flag = AtomicBool::new(false);
    let mut baking = 0;
    let baked = g
        .bake(&low, &s, &budget, Some(&flag), None, |_, phase| {
            if phase == "Baking" {
                baking += 1;
                if baking == 3 {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            true
        })
        .unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::Canceled);
    assert!(baked.result.maps.is_empty());
    assert_eq!(baked.stats.dispatches, 3, "GPU が 3 回動いてから止まる");
    assert!(g.failure().is_none());
    // 時間切れ: 同じ所で、制限を超えるまで待つ（準備は制限の内に終わる大きさ）
    let limited = MeshBakeBudget {
        max_seconds: 2.0,
        ..Default::default()
    };
    let mut baking = 0;
    let baked = g
        .bake(&low, &s, &limited, None, None, |_, phase| {
            if phase == "Baking" {
                baking += 1;
                if baking == 3 {
                    std::thread::sleep(std::time::Duration::from_millis(2100));
                }
            }
            true
        })
        .unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::TimedOut);
    assert!(baked.result.maps.is_empty());
    assert_eq!(baked.stats.dispatches, 3, "GPU が 3 回動いてから止まる");
    assert!(g.failure().is_none());
    // 進捗コールバックの false も同じ所で止まる
    let mut baking = 0;
    let baked = g
        .bake(&low, &s, &budget, None, None, |_, phase| {
            if phase == "Baking" {
                baking += 1;
            }
            baking < 4
        })
        .unwrap();
    assert_eq!(baked.result.status, MeshBakeStatus::Canceled);
    assert_eq!(baked.stats.dispatches, 3);
    // 止めたあとも、同じデバイスで最後まで焼ける
    let done = g.bake(&low, &s, &budget, None, None, |_, _| true).unwrap();
    assert_eq!(done.result.status, MeshBakeStatus::Completed);
    assert_eq!(done.result.maps.len(), s.maps.len());
}

#[test]
fn a_stopped_bake_names_the_gpu_only_when_a_dispatch_ran() {
    let low = input_with_normals(&skewed(cube(5, 0.6, 0.04, 21)));
    let s = all_maps(128, 128);
    let budget = MeshBakeBudget::default();
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        ray_query: false,
        ..many_small_dispatches()
    });
    if let Err(why) = slot.probe(true) {
        skipped(&why);
        return;
    }
    // 準備の途中で止めた: GPU は一度も動いていないので、どこで焼いたとも言わず、CPU に戻った理由も無い
    let flag = AtomicBool::new(true);
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        Some(&flag),
        None,
        |_, _| true,
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Canceled);
    assert_eq!(run.requested, BakeBackend::Gpu);
    assert!(!run.used_gpu(), "{:?}", run.gpu.map(|g| g.1));
    assert!(run.fallback_kind.is_none() && run.fallback_reason.is_none());
    // GPU が動いている間に止めた: GPU で焼いていた（途中まで）と言う
    let mut baking = 0;
    let (r, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &budget,
        None,
        None,
        |_, phase| {
            if phase == "Baking" {
                baking += 1;
            }
            baking < 3
        },
    )
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Canceled);
    let (_, stats) = run.gpu.expect("dispatch が動いたので GPU");
    assert_eq!(stats.dispatches, 2);
    assert!(run.fallback_kind.is_none());
}

/// 重なった UV の持ち主の決め方（`MeshBakeSettings::overlap`）は、受け手の並びとして GPU にも渡る: 同じ UV に重ねた −X の小さな四角
/// （三角形 0・1）と +X の大きな四角（2・3）で、決め方ごとに CPU と同じ側が焼かれる（違う側なら位置のマップの差が許す幅を超える）。
#[test]
fn overlapped_uvs_take_the_same_owner_as_the_cpu() {
    use yolu_core::mesh_maps::{MeshBakeAttributes, MeshOverlapPriority, MeshOverlapRule};
    let Some(mut g) = gpu() else { return };
    let quad = |x0: f32, x1: f32, y1: f32| {
        vec![
            x0, 0., 0., x1, 0., 0., x0, y1, 0., x0, y1, 0., x1, 0., 0., x1, y1, 0.,
        ]
    };
    let mut corners = quad(-2., -1., 1.);
    corners.extend(quad(1., 3., 2.));
    let uv = [
        0.25f32, 0.25, 0.75, 0.25, 0.25, 0.75, 0.25, 0.75, 0.75, 0.25, 0.75, 0.75,
    ];
    let uvs = [uv, uv].concat();
    let input =
        MeshBakeInput::new(corners, uvs, vec![0; 4], MeshBakeAttributes::default()).unwrap();
    for rule in MeshOverlapRule::ALL {
        let mut overlap = MeshOverlapPriority::default();
        overlap.rule = rule;
        let settings = MeshBakeSettings {
            width: 32,
            height: 32,
            padding: 0,
            maps: vec![MeshMapKind::Position],
            overlap,
            ..Default::default()
        };
        eprintln!("重なった UV・{}", rule.name());
        let _ = run(&mut g, &input, None, &settings, false);
    }
}

/// スロットは、ray query の入切が変わると次の確認でデバイスを作り直す（デバイスの作り方が変わるため）。切にすると、対応するアダプターでも
/// ray query つきのデバイスにならず、ベイクは compute の道で、`ray_query_note` は無い。入に戻すと、対応するアダプターでは ray query つきに戻る。
#[test]
fn the_slot_remakes_the_device_when_ray_query_is_switched() {
    gpu_lease::lease();
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        ray_query: true,
        ..Default::default()
    });
    let on = match slot.probe(true) {
        Ok(a) => a,
        Err(e) => return skipped(&e),
    };
    slot.set_ray_query(false);
    let off = slot.probe(true).expect("切でも作れる");
    assert!(!off.ray_query, "切なら ray query つきのデバイスにしない");
    assert_eq!(off.name, on.name, "同じアダプター");
    let low = input_with_normals(&skewed(cube(4, 0.3, 0.0, 3)));
    let mut s = all_maps(32, 32);
    s.maps = vec![MeshMapKind::AmbientOcclusion];
    let (_, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &MeshBakeBudget::default(),
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    let (_, stats) = run.gpu.expect("GPU で焼いた");
    assert_eq!(stats.method, yolu_gpu::GpuBakeMethod::Compute);
    assert_eq!(
        stats.ray_query_why,
        Some(yolu_gpu::RayQueryWhy::Disabled),
        "切のときは、切という理由の種類が残る"
    );
    slot.set_ray_query(true);
    let again = slot.probe(true).unwrap();
    assert_eq!(again.ray_query, on.ray_query, "入に戻すと元の通り");
    // 入のとき、RT コアの無いアダプターは理由を残して compute、あるアダプターは ray query か（通らなければ）理由つきの compute
    let (_, run) = bake_mesh_maps(
        BakeBackend::Gpu,
        &slot,
        &low,
        &s,
        &MeshBakeBudget::default(),
        None,
        None,
        |_, _| true,
    )
    .unwrap();
    let (_, stats) = run.gpu.expect("GPU で焼いた");
    let note = stats
        .ray_query_note
        .expect("入なら使った説明か使わなかった理由が残る");
    eprintln!("入のときの道: {:?} / {note}", stats.method);
    match stats.method {
        yolu_gpu::GpuBakeMethod::RayQuery => assert!(note.starts_with("ray query"), "{note}"),
        yolu_gpu::GpuBakeMethod::Compute => {
            assert!(note.contains("ray query を使わない理由"), "{note}")
        }
    }
}

fn tiny_ao() -> (MeshBakeInput, MeshBakeSettings) {
    let low = input_with_normals(&skewed(cube(4, 0.3, 0.0, 3)));
    let mut s = all_maps(32, 32);
    s.maps = vec![MeshMapKind::AmbientOcclusion];
    (low, s)
}

/// ray query の道で焼いている途中の（固まりではない）失敗は、ray query を止めて compute だけで 1 回やり直す。CPU には落とさない。
/// 固まり・応答なしの失敗（完了待ちの時間切れ）と、compute の道の失敗は、やり直さず CPU に戻る。止めた ray query は、入切が変わるまで戻さない。
#[test]
fn a_failure_midway_on_ray_query_is_redone_on_compute_but_a_hang_or_a_compute_failure_goes_to_the_cpu(
) {
    use yolu_gpu::MidFailure;
    gpu_lease::lease();
    let slot = GpuBakeSlot::new(GpuBakeOptions {
        ray_query: true,
        ..Default::default()
    });
    if let Err(e) = slot.probe(true) {
        return skipped(&e);
    }
    let (low, s) = tiny_ao();
    let run = |slot: &GpuBakeSlot| {
        bake_mesh_maps(
            BakeBackend::Gpu,
            slot,
            &low,
            &s,
            &MeshBakeBudget::default(),
            None,
            None,
            |_, _| true,
        )
        .unwrap()
        .1
    };
    // ray query の途中の失敗 → compute でやり直して GPU で焼ける
    slot.fail_midway_for_test("試験の失敗", MidFailure::RayQuery);
    let redone = run(&slot);
    let (_, stats) = redone.gpu.expect("compute でやり直して GPU で焼いた");
    assert!(redone.fallback_kind.is_none());
    assert_eq!(stats.method, yolu_gpu::GpuBakeMethod::Compute);
    assert_eq!(stats.ray_query_why, Some(yolu_gpu::RayQueryWhy::RunFailed));
    assert!(slot.ray_query_block().unwrap().contains("試験の失敗"));
    // 止めたあとは、毎回 ray query で失敗して CPU に落ちない（同じ設定のあいだ compute のまま）
    let next = run(&slot);
    let (_, stats) = next.gpu.expect("GPU で焼いた");
    assert_eq!(stats.ray_query_why, Some(yolu_gpu::RayQueryWhy::RunFailed));
    // 入切が変わると、ray query を試し直す
    slot.set_ray_query(false);
    slot.set_ray_query(true);
    assert!(slot.ray_query_block().is_none());
    slot.probe(true).unwrap(); // 作り直す（失敗の差し込みは、作り直したデバイスに入れる）
                               // 固まりの疑い（完了待ちの時間切れ）は、同じ GPU でやり直さず CPU に戻る
    slot.fail_midway_for_test("試験の固まり", MidFailure::RayQueryHung);
    let hung = run(&slot);
    assert!(hung.gpu.is_none());
    assert_eq!(hung.fallback_kind, Some(yolu_gpu::FallbackKind::Failed));
    assert!(
        slot.ray_query_block().is_none(),
        "固まりでは止めの印を付けない"
    );
    // compute の道の失敗も CPU に戻る
    slot.probe(true).unwrap();
    slot.fail_midway_for_test("試験の失敗（compute）", MidFailure::Compute);
    let plain = run(&slot);
    assert!(plain.gpu.is_none());
    assert_eq!(plain.fallback_kind, Some(yolu_gpu::FallbackKind::Failed));
}

/// ray query の準備（加速構造の構築・自己照合）の間も、取消・時間切れ・進捗の中止が効く。ray query の使えるアダプターだけで試す
/// （使えなければ理由を出して飛ばす）。進捗の通知の数を compute の道で数えておき、ray query の道だけが足す通知で中止する。
#[test]
fn a_cancel_during_the_ray_query_preparation_stops_the_bake_before_any_dispatch() {
    let Some(mut rq) = gpu_ray_query() else {
        return;
    };
    let Some(mut compute) = gpu() else { return };
    let (low, s) = tiny_ao();
    let mut before = 0usize;
    compute
        .bake(
            &low,
            &s,
            &MeshBakeBudget::default(),
            None,
            None,
            |_, phase| {
                if phase != "Baking" {
                    before += 1;
                }
                true
            },
        )
        .unwrap();
    let mut calls = 0usize;
    let baked = rq
        .bake(
            &low,
            &s,
            &MeshBakeBudget::default(),
            None,
            None,
            |_, phase| {
                if phase == "Baking" {
                    return true;
                }
                calls += 1;
                calls <= before
            },
        )
        .unwrap();
    assert_ne!(
        baked.result.status,
        MeshBakeStatus::Completed,
        "準備の間の中止で止まる"
    );
    assert_eq!(baked.stats.dispatches, 0, "dispatch の前に止まる");
}
