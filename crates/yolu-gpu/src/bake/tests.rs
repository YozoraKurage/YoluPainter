//! WGSL の `Params` と Rust の `Params` の並び（オフセットと大きさ）が一致することを、naga の構造体の配置から確かめる。
use super::{pack::Params, COMMON, TRACE_COMPUTE};
use std::mem::{offset_of, size_of};

macro_rules! offsets {
    ($($field:ident),* $(,)?) => {
        vec![$((stringify!($field), offset_of!(Params, $field))),*]
    };
}

#[test]
fn wgsl_params_layout_matches_rust() {
    let source = format!("{COMMON}\n{TRACE_COMPUTE}");
    let module = naga::front::wgsl::parse_str(&source).expect("WGSL を読めない");
    let (_, params) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("Params"))
        .expect("Params が無い");
    let naga::TypeInner::Struct { members, span } = &params.inner else {
        panic!("Params が構造体でない");
    };
    let rust = offsets![
        map_offsets,
        cvf,
        cvu,
        width,
        height,
        aa,
        band_first,
        band_texels,
        local_start,
        count,
        out_stride,
        flags,
        tiles_x,
        tile_off_off,
        tile_tri_off,
        bvh_table_off,
        low_bvh,
        high_bvh,
        ao_count,
        th_count,
        ao_dirs_off,
        th_dirs_off,
        lo_f_stride,
        lo_cage_off,
        lo_frame_off,
        pad0,
        pad1,
        ao_cos2,
        th_cos2,
        ray_offset,
        ao_max,
        th_max,
        frontal,
        rear,
        range,
        min_x,
        min_y,
        min_z,
        pad2,
        scale_x,
        scale_y,
        scale_z,
        pad3,
    ];
    assert_eq!(members.len(), rust.len(), "メンバーの数");
    for (m, (name, offset)) in members.iter().zip(&rust) {
        assert_eq!(m.name.as_deref(), Some(*name), "メンバーの並び");
        assert_eq!(m.offset as usize, *offset, "{name} のオフセット");
    }
    assert_eq!(*span as usize, size_of::<Params>(), "構造体の大きさ");
}

#[test]
fn wgsl_validates_with_the_compute_trace() {
    let source = format!("{COMMON}\n{TRACE_COMPUTE}");
    let module = naga::front::wgsl::parse_str(&source).expect("WGSL を読めない");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("WGSL の検証に失敗");
}

#[test]
fn wgsl_validates_with_the_ray_query_trace() {
    // ray query のシェーダーは、この環境のアダプターでは動かせない（llvmpipe に無い）ので、naga で読めて検証に通ることだけを確かめる。
    let source = super::rayquery::source(COMMON);
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("WGSL を読めない: {}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::RAY_QUERY,
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("WGSL の検証に失敗: {}", e.emit_to_string(&source)));
}

#[test]
fn ray_query_shader_translates_to_spirv() {
    // Vulkan へ渡す SPIR-V に変換できること（変換の失敗は実機でパイプラインを作れない原因になる）。ray query の機能つきで。
    let source = super::rayquery::source(COMMON);
    let module = naga::front::wgsl::parse_str(&source).expect("WGSL を読めない");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::RAY_QUERY,
    )
    .validate(&module)
    .expect("WGSL の検証に失敗");
    let options = naga::back::spv::Options {
        capabilities: Some(
            [
                naga::back::spv::Capability::Shader,
                naga::back::spv::Capability::RayQueryKHR,
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    };
    for entry in ["bake", "selfcheck"] {
        let pipeline = naga::back::spv::PipelineOptions {
            shader_stage: naga::ShaderStage::Compute,
            entry_point: entry.into(),
        };
        let words = naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline))
            .unwrap_or_else(|e| panic!("{entry} を SPIR-V にできない: {e}"));
        assert!(words.len() > 1000, "{entry}: SPIR-V が小さすぎる");
    }
}

#[test]
fn a_dispatch_never_holds_more_workgroups_than_the_device_allows() {
    use super::{dispatch_limit, max_chunk, GROUP, MAX_DISPATCH_TEXELS};
    // WebGPU が保証する上限（65535）: 65536 グループ（= 1 << 22 テクセル）には届かない
    assert_eq!(dispatch_limit(65535), 65535 * GROUP);
    assert!(MAX_DISPATCH_TEXELS.div_ceil(GROUP) <= 65535);
    // 上限が大きいデバイスでも、保証されている 65535 を超えない
    assert_eq!(dispatch_limit(u32::MAX), MAX_DISPATCH_TEXELS);
    // 小さい上限・0 でも 1 グループは焼ける
    assert_eq!(dispatch_limit(256), 256 * GROUP);
    assert_eq!(dispatch_limit(0), GROUP);
    for limit in [1u32, 256, 65535, 1 << 20] {
        let cap = dispatch_limit(limit);
        // 光線の要らないマップ（1 テクセル 1 回）が一番大きく育つ
        for rays in [1u64, 2, 64, 1 << 20, u64::MAX / 4] {
            let chunk = max_chunk(rays, cap);
            assert!(chunk >= GROUP, "{limit} {rays}");
            assert!(
                chunk.div_ceil(GROUP) <= limit.max(1),
                "{limit} {rays}: {chunk}"
            );
        }
    }
}

#[test]
fn ray_query_is_on_unless_the_environment_turns_it_off() {
    use super::flag_enabled;
    for on in [
        None,
        Some(""),
        Some(" "),
        Some("1"),
        Some("true"),
        Some("on"),
        Some("yes"),
        Some("ray"),
    ] {
        assert!(flag_enabled(on), "{on:?}");
    }
    for off in ["0", "false", "FALSE", "off", "Off", " no ", " 0 "] {
        assert!(!flag_enabled(Some(off)), "{off:?}");
    }
}

#[test]
fn the_reason_ray_query_is_not_used_is_kept_with_its_kind() {
    use super::{ray_query_unavailable, RayQueryWhy};
    assert_eq!(ray_query_unavailable(true, true, None), None);
    let (why, note) = ray_query_unavailable(false, false, None).unwrap();
    assert_eq!(why, RayQueryWhy::NotSupported);
    assert!(note.contains("対応していない"), "{note}");
    let (why, note) = ray_query_unavailable(
        true,
        false,
        Some("Feature EXPERIMENTAL_RAY_QUERY is not supported"),
    )
    .unwrap();
    assert_eq!(why, RayQueryWhy::Device);
    assert!(
        note.contains("デバイスを作れなかった") && note.contains("EXPERIMENTAL_RAY_QUERY"),
        "{note}"
    );
    let (_, note) = ray_query_unavailable(true, false, None).unwrap();
    assert!(note.contains("理由は不明"));
}

#[test]
fn a_ray_query_device_that_fails_after_it_was_made_falls_back_to_the_plain_device_with_the_reason()
{
    use super::{with_ray_query_fallback, GpuBakeError};
    type R = Result<(&'static str, Option<String>), GpuBakeError>;
    let plain = |e: Option<String>| async move { R::Ok(("plain", e)) };
    // 試さない（対応していない・切）: 理由なしで plain
    let none = pollster::block_on(with_ray_query_fallback(
        None::<std::future::Ready<Result<(&'static str, Option<String>), String>>>,
        plain,
    ))
    .unwrap();
    assert_eq!(none, ("plain", None));
    // 作れたあと、準備（compute のシェーダー・最初の確認）で失敗した: 理由を持って plain で作り直す（Err にして CPU へ落とさない）
    let failed = pollster::block_on(with_ray_query_fallback(
        Some(std::future::ready(
            Err::<(&'static str, Option<String>), _>(
                "GPU 利用不可: ベイクのシェーダーを作れません: x".to_string(),
            ),
        )),
        plain,
    ))
    .unwrap();
    assert_eq!(failed.0, "plain");
    assert!(failed.1.unwrap().contains("ベイクのシェーダーを作れません"));
    // 成功したらそのまま（plain は呼ばない）
    let ok = pollster::block_on(with_ray_query_fallback(
        Some(std::future::ready(Ok(("rq", None)))),
        |_| async { R::Err(GpuBakeError::Failed("呼ばれない".into())) },
    ))
    .unwrap();
    assert_eq!(ok.0, "rq");
}

/// 自己照合の結果の並び（`CHECK_STRIDE` ワード）で、もっともらしい答えを n 本ぶん作る。
fn selfcheck_answers(n: usize) -> Vec<u32> {
    use super::rayquery::CHECK_STRIDE;
    let mut out = vec![0u32; n * CHECK_STRIDE];
    for i in 0..n {
        if i % 3 == 0 {
            continue; // 外れ
        }
        let w = &mut out[i * CHECK_STRIDE..(i + 1) * CHECK_STRIDE];
        w[0] = 1;
        w[1] = (0.5 + i as f32 * 1e-3).to_bits();
        w[2] = (i % 7) as u32;
        w[3] = (0.05 + (i % 17) as f32 / 40.0).to_bits();
        w[4] = (0.02 + (i % 13) as f32 / 50.0).to_bits();
    }
    out
}

#[test]
fn the_self_check_compares_barycentrics_not_only_the_triangle_and_distance() {
    use super::rayquery::{compare, Verdict, CHECK_RAYS, CHECK_STRIDE};
    let n = CHECK_RAYS as usize;
    let compute = selfcheck_answers(n);
    assert!(matches!(
        compare(&compute, &compute, 1.0, true),
        Verdict::Pass(_)
    ));
    // u と v が入れ替わる規約の食い違い: 当たったか・距離・三角形は全部合うので、重心座標を見なければ通ってしまう
    let mut swapped = compute.clone();
    for w in swapped.as_chunks_mut::<CHECK_STRIDE>().0.iter_mut() {
        w.swap(3, 4);
    }
    match compare(&compute, &swapped, 1.0, false) {
        Verdict::Fail(why) => assert!(why.contains("重心座標"), "{why}"),
        other => panic!("通らないはず: {other:?}"),
    }
    // 重心座標が少しだけ（許容幅の内で）揺れるのは通る
    let mut close = compute.clone();
    for w in close.as_chunks_mut::<CHECK_STRIDE>().0.iter_mut() {
        w[3] = (f32::from_bits(w[3]) + 1e-5).to_bits();
    }
    assert!(matches!(
        compare(&compute, &close, 1.0, false),
        Verdict::Pass(_)
    ));
    // 最初の当たりで止めるレイ（8 ビット目）は、どの当たりかが違ってよいので当たったかだけを見る
    let mut any_hit_differs = compute.clone();
    for (i, w) in any_hit_differs
        .as_chunks_mut::<CHECK_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        if i & 8 != 0 && w[0] == 1 {
            w[1] = 9.0f32.to_bits();
            w[2] = 99;
            w[3] = 0.9f32.to_bits();
        }
    }
    assert!(matches!(
        compare(&compute, &any_hit_differs, 1.0, true),
        Verdict::Pass(_)
    ));
    // 距離の食い違いは通らない
    let mut far = compute.clone();
    for (i, w) in far.as_chunks_mut::<CHECK_STRIDE>().0.iter_mut().enumerate() {
        if i & 8 == 0 && w[0] == 1 {
            w[1] = (f32::from_bits(w[1]) + 0.1).to_bits();
        }
    }
    assert!(matches!(
        compare(&compute, &far, 1.0, false),
        Verdict::Fail(_)
    ));
}

#[test]
fn back_face_rays_are_only_required_when_the_bake_culls_back_faces() {
    use super::rayquery::{compare, Verdict, CHECK_RAYS, CHECK_STRIDE};
    let n = CHECK_RAYS as usize;
    let compute = selfcheck_answers(n);
    // 表裏の規約が逆なら、裏面を飛ばすレイ（3 ビット目）の当たりだけが食い違う（飛ばすはずの当たりが消える・残る）
    let mut reversed = compute.clone();
    for (i, w) in reversed
        .as_chunks_mut::<CHECK_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        if i & 4 != 0 {
            w[0] ^= 1;
        }
    }
    // 裏面を飛ばさない焼きでは、ray query の道は裏面の旗を使わないので課さない
    match compare(&compute, &reversed, 1.0, false) {
        Verdict::Pass(note) => assert!(!note.contains("裏面を飛ばすレイ"), "{note}"),
        other => panic!("通るはず: {other:?}"),
    }
    // 裏面を飛ばす焼きでは、ほかのレイは合うので「規約が逆」と分かる（頂点の並びを入れ替えてやり直す手がかり）
    match compare(&compute, &reversed, 1.0, true) {
        Verdict::CullReversed(why) => assert!(why.contains("裏面を飛ばすレイ"), "{why}"),
        other => panic!("規約の逆と判る形で通らないはず: {other:?}"),
    }
    // 裏面を飛ばさないレイが合わないのは、規約ではなく道そのものの食い違い
    let mut broken = compute.clone();
    for (i, w) in broken
        .as_chunks_mut::<CHECK_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        if i & 4 == 0 {
            w[0] ^= 1;
        }
    }
    assert!(matches!(
        compare(&compute, &broken, 1.0, true),
        Verdict::Fail(_)
    ));
    assert!(matches!(
        compare(&compute, &broken, 1.0, false),
        Verdict::Fail(_)
    ));
}
