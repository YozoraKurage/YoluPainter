//! ベイクの試験（画面なし）。試しの立方体と、Live Link と同じ形のモデルを別のスレッドで焼き、結果・取消・捨てる条件・古さの判定を確かめる。

use std::time::Instant;

use yolu_core::mesh_maps::{MeshMapKind, MeshMapStaleReason, MeshMapState};
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, MeshPose, Model, Pose, Submesh};

use super::*;
use crate::state::Action;

fn bake(a: BakeAction) -> Action {
    Action::Bake(a)
}

/// 試しの立方体を読んだ 64 × 64 の状態。速く焼けるように設定を小さくする。
fn cube() -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    quick(&mut s);
    s
}

fn quick(s: &mut AppState) {
    s.bake.settings.maps = vec![
        MeshMapKind::WorldNormal,
        MeshMapKind::Position,
        MeshMapKind::AmbientOcclusion,
    ];
    s.bake.settings.ao_samples = 8;
    s.bake.settings.padding = 4;
}

fn info(name: &str) -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: String::new(),
        textures: vec![],
        routes: vec![],
    }
}

/// 2 枚の板（左は 1 つ目、右は 2 つ目のマテリアル。UV は 0〜1 の中で左右に分ける）。
fn two_quads(generation: u32, lift: f32) -> Model {
    let quad = |x: f32, u0: f32| MeshData {
        key: format!("{x}"),
        name: format!("板{x}"),
        skinned: false,
        positions: vec![
            [x, 0.0, 0.0],
            [x + 1.0, 0.0, 0.0],
            [x, 1.0, lift],
            [x + 1.0, 1.0, lift],
        ],
        normals: vec![],
        uv0: vec![[u0, 0.0], [u0 + 0.5, 0.0], [u0, 1.0], [u0 + 0.5, 1.0]],
        submeshes: vec![Submesh {
            material: if u0 == 0.0 { 0 } else { 1 },
            indices: vec![0, 1, 2, 2, 1, 3],
        }],
    };
    Model {
        generation,
        name: "二枚".into(),
        materials: vec![info("Skin"), info("Hair")],
        meshes: vec![quad(0.0, 0.0), quad(2.0, 0.5)],
    }
}

fn kinds(s: &AppState, index: usize) -> Vec<MeshMapKind> {
    s.sets
        .get(index)
        .unwrap()
        .mesh_maps
        .iter()
        .map(|m| m.kind())
        .collect()
}

#[test]
fn refusals_come_in_the_order_the_window_shows_them() {
    let lang_ja = |s: &mut AppState| s.bake_refusal().unwrap();
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    assert_eq!(lang_ja(&mut s), "モデルがありません");
    s.apply(Action::LoadDemoModel);
    assert_eq!(s.bake_refusal(), None);
    // マップを 1 つも選んでいない
    let maps = std::mem::take(&mut s.bake.settings.maps);
    assert_eq!(lang_ja(&mut s), "チェックしたマップがありません");
    s.bake.settings.maps = maps;
    // 描いている間
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    assert_eq!(lang_ja(&mut s), "描いている間はできません。");
    s.apply(bake(BakeAction::Start));
    assert!(!s.bake.is_baking(), "描いている間は始めない");
    assert_eq!(s.message, "描いている間はできません。");
    s.doc.end_stroke(stroke).unwrap();
    assert_eq!(s.bake_refusal(), None);
    // 英語
    s.lang = Lang::En;
    s.bake.settings.maps.clear();
    assert_eq!(s.bake_refusal().unwrap(), "No map is checked");
}

#[test]
fn bakes_the_cube_in_another_thread_and_keeps_the_maps_in_the_set() {
    let mut s = cube();
    assert!(!s.modified);
    s.apply(bake(BakeAction::Start));
    assert!(s.bake.is_baking(), "別のスレッドで走っている");
    assert_eq!(
        s.bake_refusal().as_deref(),
        Some("ベイク中です"),
        "走っている間は 2 つ目を始めない"
    );
    let progress = s.bake.progress().unwrap();
    assert_eq!((progress.index, progress.total), (1, 1));
    s.wait_bake();
    assert!(!s.bake.is_baking());
    assert_eq!(
        kinds(&s, 0),
        [
            MeshMapKind::WorldNormal,
            MeshMapKind::Position,
            MeshMapKind::AmbientOcclusion
        ]
    );
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .unwrap();
    let p = map.provenance();
    assert_eq!((p.width, p.height), (64, 64), "大きさは文書");
    assert_eq!(p.target_slots, [0]);
    assert_eq!((p.padding, p.antialiasing), (4, 1));
    assert!(map.coverage().contains(&1), "立方体の面が UV を覆う");
    assert!(s.modified, "保存していない変更になる");
    assert_eq!(
        s.bake.view,
        MeshMapView::Coverage,
        "初めて焼いたら UV の範囲を見せる"
    );
    assert!(s.message.contains("焼きました"), "{}", s.message);
    assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| *ok));
    assert!(s.sets.current().mesh_maps.report().is_some());
    // 今の条件で焼いたものなので Current
    for kind in kinds(&s, 0) {
        let check = s.mesh_map_check(0, kind).unwrap();
        assert_eq!(
            check.state,
            MeshMapState::Current,
            "{kind:?}: {:?}",
            check.reasons
        );
    }
    // 文書は変わらない（描くレイヤーではない）
    assert!(!s.doc.can_undo());
}

#[test]
fn rebaking_replaces_the_same_kind_and_leaves_the_others() {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let before = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .unwrap()
        .clone();
    s.bake.settings.maps = vec![MeshMapKind::Position];
    s.bake.settings.padding = 8;
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(kinds(&s, 0).len(), 3, "焼き直した種類だけ置き換える");
    let after = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&before, after));
    assert_eq!(after.provenance().padding, 8);
    // 余白を変えたので、焼き直していない種類は古い
    let check = s.mesh_map_check(0, MeshMapKind::WorldNormal).unwrap();
    assert_eq!(check.state, MeshMapState::Stale);
    assert!(check
        .reasons
        .contains(&MeshMapStaleReason::Padding { baked: 4, now: 8 }));
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Current
    );
}

#[test]
fn a_result_is_discarded_when_the_model_changed_while_baking() {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    // 焼いている間に別のモデルへ替える（形が変わるので、結果の意味が変わる）
    s.apply(Action::LoadDemoModel);
    s.wait_bake();
    assert!(s.sets.current().mesh_maps.is_empty(), "捨てる");
    assert!(
        s.sets.current().mesh_maps.report().is_none() && s.sets.current().mesh_maps.run().is_none(),
        "捨てた結果の記録と場所は残さない"
    );
    assert!(s.message.contains("捨てました"), "{}", s.message);
    assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| !*ok));
    assert!(!s.modified);
    // 同じ形でもう一度焼けば入る
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(kinds(&s, 0).len(), 3);
}

#[test]
fn a_result_is_discarded_when_the_document_was_replaced() {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    let (doc, _) = crate::state::blank_document(64, 64);
    s.doc = doc; // 同じ大きさでも別の文書
    s.wait_bake();
    assert!(s.sets.current().mesh_maps.is_empty());
    assert!(s.message.contains("キャンバス"), "{}", s.message);
}

#[test]
fn cancel_keeps_the_previous_maps_and_the_rest_of_the_queue() {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let previous: Vec<_> = s.sets.current().mesh_maps.iter().cloned().collect();
    // 取消が来るまで始めない仕事にして取り消す（取消が効いたことを、焼き終わる速さに頼らず確かめる）
    s.bake.settings.maps = vec![MeshMapKind::AmbientOcclusion];
    s.bake.park_next = true;
    s.apply(bake(BakeAction::Start));
    assert!(s.bake.is_baking());
    s.apply(bake(BakeAction::Cancel));
    assert!(s.bake.progress().unwrap().canceling);
    assert!(s.message.contains("取り消しています"), "{}", s.message);
    s.wait_bake();
    assert!(s.message.contains("取り消しました"), "{}", s.message);
    let now: Vec<_> = s.sets.current().mesh_maps.iter().cloned().collect();
    assert_eq!(now.len(), previous.len());
    assert!(
        now.iter()
            .zip(&previous)
            .all(|(a, b)| std::sync::Arc::ptr_eq(a, b)),
        "取り消したら前のマップのまま"
    );
    assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| !*ok));
}

#[test]
fn checks_keep_one_map_and_one_set() {
    let mut s = cube();
    for kind in s.bake.settings.maps.clone().into_iter().skip(1) {
        s.apply(bake(BakeAction::Map(kind, false)));
    }
    assert_eq!(s.bake.settings.maps.len(), 1);
    s.apply(bake(BakeAction::Map(s.bake.settings.maps[0], false)));
    assert_eq!(s.bake.settings.maps.len(), 1, "最後の 1 つは外せない");
    assert!(s.message.contains("1 つは残します"), "{}", s.message);
    // 足すと種類の並びの順に入る
    s.apply(bake(BakeAction::Map(MeshMapKind::WorldNormal, true)));
    s.apply(bake(BakeAction::Map(MeshMapKind::Curvature, true)));
    assert_eq!(
        s.bake.settings.maps,
        [
            MeshMapKind::WorldNormal,
            MeshMapKind::Curvature,
            MeshMapKind::Position
        ]
        .into_iter()
        .filter(|k| s.bake.settings.maps.contains(k))
        .collect::<Vec<_>>()
    );
    // セット（1 つだけなら外せない）
    let uid = s.sets.current().uid;
    s.apply(bake(BakeAction::Set(uid, false)));
    assert!(s.bake.skipped.is_empty());
}

#[test]
fn bakes_each_checked_set_with_its_own_slots_and_skips_a_set_outside_the_model() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let (report, shape) = s.receive_link_model(&two_quads(1, 0.0));
    assert_eq!(shape, Ok(()));
    assert_eq!(report.created, ["Hair"]);
    quick(&mut s);
    // モデルに無いマテリアルのセット
    let (doc, _) = crate::state::blank_document(32, 32);
    s.sets.push(
        crate::sets::guid_string(doc.id()),
        "Gone".into(),
        false,
        MaterialRef::Material {
            name: "Gone".into(),
            asset: None,
        },
        None,
        doc,
    );
    s.bind_model();
    assert_eq!(s.bakeable_sets(), [0, 1], "モデルに無いセットは焼かない");
    assert_eq!(s.set_slots(0), Some(vec![0]));
    assert_eq!(s.set_slots(1), Some(vec![1]));
    assert_eq!(s.set_slots(2), None);
    s.apply(bake(BakeAction::Start));
    let first = s.bake.progress().unwrap();
    assert_eq!((first.index, first.total), (1, 2));
    s.wait_bake();
    assert_eq!(kinds(&s, 0).len(), 3);
    assert_eq!(kinds(&s, 1).len(), 3, "2 つ目のセットも焼く");
    assert!(kinds(&s, 2).is_empty());
    let slot_of = |i: usize| {
        s.sets
            .get(i)
            .unwrap()
            .mesh_maps
            .get(MeshMapKind::Position)
            .unwrap()
            .provenance()
            .target_slots
            .clone()
    };
    assert_eq!(slot_of(0), [0]);
    assert_eq!(slot_of(1), [1]);
    assert!(
        s.message.contains("2 個のテクスチャセット"),
        "{}",
        s.message
    );
    // モデルに無いセットの状態を聞くと、マップは無い
    assert!(s.mesh_map_check(2, MeshMapKind::Position).is_none());
    // 外したセットは焼かない
    let hair = s.sets.get(1).unwrap().uid;
    s.apply(bake(BakeAction::Set(hair, false)));
    assert_eq!(s.bakeable_sets(), [0]);
}

#[test]
fn a_moved_vertex_or_a_new_model_makes_the_maps_stale_for_their_reason() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    quick(&mut s);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Current
    );
    // ポーズ（頂点が動く。三角形・UV は同じ）
    let moved: Vec<[f32; 3]> = two_quads(1, 0.5).meshes[0].positions.clone();
    s.receive_link_pose(&Pose {
        generation: 1,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: moved,
            normals: vec![],
        }],
    })
    .unwrap();
    let check = s.mesh_map_check(0, MeshMapKind::Position).unwrap();
    assert_eq!(check.state, MeshMapState::Stale);
    assert_eq!(check.reasons, [MeshMapStaleReason::ShapeChanged]);
    // 書き出しの AO には使わない
    assert_eq!(
        s.occlusion_for_export(0),
        Occlusion::Stale(stale_reasons(&check))
    );
    // モデルが無くなると照合できない
    s.close_link_model(1);
    let check = s.mesh_map_check(0, MeshMapKind::Position).unwrap();
    assert_eq!(check.state, MeshMapState::Unverified);
}

/// 床と壁（同じマテリアル。2 枚の板が辺で接して凹む角を作る）。
fn corner_model() -> Model {
    let plate = |positions: [[f32; 3]; 4], u0: f32| MeshData {
        key: format!("{u0}"),
        name: format!("板{u0}"),
        skinned: false,
        positions: positions.to_vec(),
        normals: vec![],
        uv0: vec![[u0, 0.0], [u0 + 0.5, 0.0], [u0, 1.0], [u0 + 0.5, 1.0]],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 1, 2, 2, 1, 3],
        }],
    };
    Model {
        generation: 1,
        name: "角".into(),
        materials: vec![info("Skin")],
        meshes: vec![
            plate(
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                ],
                0.0,
            ),
            plate(
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                ],
                0.5,
            ),
        ],
    }
}

#[test]
fn the_ao_for_export_is_one_byte_per_texel_and_white_outside_the_uvs() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&corner_model());
    quick(&mut s);
    s.bake.settings.ao_samples = 32;
    assert_eq!(s.occlusion_for_export(0), Occlusion::None);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let Occlusion::Bytes(bytes) = s.occlusion_for_export(0) else {
        panic!("焼いた AO が使える");
    };
    assert_eq!(bytes.len(), 64 * 64);
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::AmbientOcclusion)
        .unwrap();
    for (i, c) in map.coverage().iter().enumerate() {
        if *c == 0 {
            assert_eq!(bytes[i], 255, "UV の外は遮蔽なし");
        } else {
            let v = map.value((i % 64) as i32, (i / 64) as i32, 0).unwrap();
            assert_eq!(bytes[i], yolu_core::export::occlusion_byte(v));
        }
    }
    assert!(bytes.iter().any(|b| *b < 255), "凹む角に遮蔽が出る");
    // 文書の大きさが変わったら使わない
    let (doc, _) = crate::state::blank_document(32, 32);
    s.doc = doc;
    assert!(matches!(s.occlusion_for_export(0), Occlusion::Stale(_)));
}

#[test]
fn viewing_a_map_the_set_does_not_have_falls_back_to_none() {
    let mut s = cube();
    s.apply(bake(BakeAction::View(MeshMapView::Kind(
        MeshMapKind::Position,
    ))));
    assert_eq!(
        s.bake.view,
        MeshMapView::None,
        "焼いていないマップは見られない"
    );
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    s.apply(bake(BakeAction::View(MeshMapView::Kind(
        MeshMapKind::Position,
    ))));
    assert_eq!(s.bake.view, MeshMapView::Kind(MeshMapKind::Position));
    s.apply(bake(BakeAction::View(MeshMapView::Kind(
        MeshMapKind::Height,
    ))));
    assert_eq!(s.bake.view, MeshMapView::None);
}

#[test]
fn names_cover_every_kind_in_both_languages() {
    for kind in MeshMapKind::ALL {
        for lang in Lang::ALL {
            assert!(!kind_label(lang, kind).is_empty());
            assert!(!kind_short(lang, kind).is_empty());
            assert!(!window::kind_help(lang, kind).is_empty());
        }
    }
    assert_eq!(slot_list(&[0, 2]), "0, 2");
    assert_eq!(slot_list(&[]), "—");
}

use crate::sets::MaterialRef;

#[test]
fn a_figure_bakes_each_set_and_a_new_pose_makes_the_maps_stale() {
    use crate::view3d::pose::{set_pose, PoseAction};
    use yolu_core::glam::Quat;
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(s.view3d.pose.session.is_some(), "{}", s.message);
    quick(&mut s);
    assert_eq!(s.bake_refusal(), None);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(s.sets.len(), 2, "肌と顔");
    assert!(
        s.message.contains("2 個のテクスチャセット"),
        "{}",
        s.message
    );
    for i in 0..2 {
        assert_eq!(kinds(&s, i).len(), 3, "セット {i}");
        assert_eq!(
            s.mesh_map_check(i, MeshMapKind::Position).unwrap().state,
            MeshMapState::Current
        );
    }
    // 骨を曲げる（三角形・UV は同じで、頂点が動く）と、焼いたマップは古い
    let mut pose = s.view3d.pose.session.as_ref().unwrap().pose().clone();
    let arm = s
        .view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "右上腕")
        .expect("試しの人形の腕");
    pose.locals[arm].rotation = Quat::from_rotation_z(-1.0);
    set_pose(&mut s.view3d, pose).unwrap();
    for i in 0..2 {
        let check = s.mesh_map_check(i, MeshMapKind::Position).unwrap();
        assert_eq!(check.state, MeshMapState::Stale, "セット {i}");
        assert!(check.reasons.contains(&MeshMapStaleReason::ShapeChanged));
    }
    // 今の形で焼き直せば、また最新
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Current
    );
}

#[test]
fn a_bake_the_budget_cannot_hold_or_with_uvs_outside_0_1_is_refused_with_the_reason() {
    // 8192 × 8192 に 10 種類は、メモリ予算（512 MiB）を超える
    let mut s = AppState::new(8192, 8192);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = MeshMapKind::ALL.to_vec();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert!(s.message.contains("予算"), "{}", s.message);
    assert!(s.sets.current().mesh_maps.is_empty());
    assert!(!s.modified);
    assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| !*ok));
    // UV が 0〜1 の外（繰り返し・UDIM）はベイクできない
    let mut model = two_quads(1, 0.0);
    model.meshes[0].uv0[3] = [1.5, 1.0];
    let mut t = AppState::new(64, 64);
    t.bake.backend = BakeBackend::Cpu;
    t.receive_link_model(&model).1.unwrap();
    quick(&mut t);
    t.apply(bake(BakeAction::Start));
    t.wait_bake();
    assert!(t.message.contains("UV"), "{}", t.message);
    assert!(t.sets.iter().all(|x| x.mesh_maps.is_empty()));
    // 英語
    t.lang = Lang::En;
    t.apply(bake(BakeAction::Start));
    t.wait_bake();
    assert!(
        t.message.starts_with("Cannot bake the mesh maps"),
        "{}",
        t.message
    );
}

#[test]
fn a_refusal_is_the_same_whichever_place_is_chosen_and_the_cpu_is_not_retried() {
    for backend in [BakeBackend::Gpu, BakeBackend::Auto] {
        // メモリ予算を超える（準備で断る。GPU に渡す前）
        let mut s = AppState::new(8192, 8192);
        s.bake.backend = backend;
        s.apply(Action::LoadDemoModel);
        s.bake.settings.maps = MeshMapKind::ALL.to_vec();
        s.apply(bake(BakeAction::Start));
        s.wait_bake();
        assert!(s.message.contains("予算"), "{backend:?}: {}", s.message);
        assert!(s.sets.current().mesh_maps.is_empty());
        assert!(
            s.sets.current().mesh_maps.run().is_none(),
            "断ったので記録は残さない"
        );
        assert!(!s.modified);
        assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| !*ok));
        // UV が 0〜1 の外
        let mut model = two_quads(1, 0.0);
        model.meshes[0].uv0[3] = [1.5, 1.0];
        let mut t = AppState::new(64, 64);
        t.bake.backend = backend;
        t.receive_link_model(&model).1.unwrap();
        quick(&mut t);
        t.apply(bake(BakeAction::Start));
        t.wait_bake();
        assert!(t.message.contains("UV"), "{backend:?}: {}", t.message);
        assert!(t.sets.iter().all(|x| x.mesh_maps.is_empty()));
    }
}

#[test]
fn the_built_input_is_released_when_nothing_needs_it() {
    let mut s = cube();
    assert!(s.bake_input().is_ok());
    assert!(s.bake.input.is_some());
    s.release_idle_bake_input();
    assert!(
        s.bake.input.is_none(),
        "ウィンドウも焼いたマップも無ければ手放す"
    );
    // ウィンドウを開いている間・マップがあるあいだは持つ
    s.apply(bake(BakeAction::OpenWindow));
    assert!(s.bake_input().is_ok());
    s.release_idle_bake_input();
    assert!(s.bake.input.is_some());
    s.apply(bake(BakeAction::CloseWindow));
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert!(s.bake_input().is_ok());
    s.release_idle_bake_input();
    assert!(s.bake.input.is_some(), "焼いたマップの古さの判定に要る");
}

#[test]
fn closing_the_window_frees_the_input_the_overlap_caches_point_at() {
    // ウィンドウを閉じて、焼いたマップも走っているベイクも無ければ、アイランドの索引・見取り図も入力と形を握り続けない
    let mut s = cube();
    let geometry_refs = |s: &AppState| Arc::strong_count(&s.view3d.full_model().unwrap().geometry);
    let baseline = geometry_refs(&s);
    s.apply(bake(BakeAction::OpenWindow));
    let input = s.bake_input().unwrap();
    let weak = Arc::downgrade(&input);
    drop(input);
    assert!(matches!(s.overlap_islands(true), Some(Ok(_))));
    assert!(s.overlap_map().is_some(), "見取り図の中身ができる");
    assert!(s.bake.islands.is_some() && s.bake.map.is_some());
    assert!(geometry_refs(&s) > baseline, "見取り図が形を握っている");
    // ウィンドウを開いている間は、入力も索引も見取り図も持つ
    s.release_idle_bake_input();
    assert!(s.bake.input.is_some() && s.bake.islands.is_some() && s.bake.map.is_some());
    s.apply(bake(BakeAction::CloseWindow));
    assert!(s.bake.map.is_none(), "見取り図はウィンドウの物");
    assert_eq!(
        geometry_refs(&s),
        baseline,
        "ウィンドウを閉じたら形を握らない"
    );
    s.release_idle_bake_input();
    assert!(s.bake.input.is_none());
    assert!(s.bake.islands.is_none(), "手放した入力を索引が握っている");
    assert!(weak.upgrade().is_none(), "入力がどこかに残っている");
    assert_eq!(geometry_refs(&s), baseline);
}

#[test]
fn the_island_index_of_the_polygon_fill_menu_does_not_outlive_the_input() {
    // ウィンドウを開かずに、ポリゴン塗りつぶしの右クリックだけでアイランドの索引を作った場合
    let mut s = cube();
    let islands = s.overlap_islands(true).unwrap().unwrap();
    let weak = Arc::downgrade(&s.bake_input().unwrap());
    drop(islands);
    s.release_idle_bake_input();
    assert!(s.bake.input.is_none());
    assert!(s.bake.islands.is_none());
    assert!(weak.upgrade().is_none(), "入力がどこかに残っている");
}

#[test]
fn an_island_index_of_a_replaced_input_is_dropped_while_the_window_is_open() {
    // ウィンドウを開いている間にモデルの形が替わって入力が作り直されたら、古い入力に結び付いた索引は残さない
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    s.apply(bake(BakeAction::OpenWindow));
    assert!(matches!(s.overlap_islands(true), Some(Ok(_))));
    let old = Arc::downgrade(&s.bake_input().unwrap());
    lift(&mut s, 0.3);
    let now = s.bake_input().unwrap();
    assert!(old.upgrade().is_some(), "索引がまだ古い入力を握っている");
    s.release_idle_bake_input();
    assert!(
        s.bake.input.is_some(),
        "ウィンドウを開いている間は今の入力を持つ"
    );
    assert!(s.bake.islands.is_none());
    assert!(old.upgrade().is_none(), "古い入力が残っている");
    assert!(matches!(s.overlap_islands(true), Some(Ok(i)) if i.is_on(&now)));
}

// ───────── モデルの同一性・入力を作る場所 ─────────

/// Live Link のポーズで、1 枚目の板の上の頂点を `z` に動かす（形が変わり、モデルは作り直される）。
fn lift(s: &mut AppState, z: f32) {
    let mut positions = two_quads(1, 0.0).meshes[0].positions.clone();
    positions[2][2] = z;
    positions[3][2] = z;
    s.receive_link_pose(&Pose {
        generation: 1,
        meshes: vec![MeshPose {
            mesh: 0,
            positions,
            normals: vec![],
        }],
    })
    .unwrap();
}

/// ウィンドウの表示と同じ作り方（別のスレッド）で入力ができるまで待つ。
fn wait_input(s: &mut AppState) -> Result<Arc<MeshBakeInput>, String> {
    let start = Instant::now();
    loop {
        if let Some(result) = s.bake_input_nowait() {
            return result;
        }
        assert!(start.elapsed().as_secs() < 120, "入力ができない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn fresh_hash(s: &AppState) -> String {
    input::build_input(s.view3d.full_model().unwrap())
        .unwrap()
        .hash()
        .to_owned()
}

#[test]
fn the_cached_input_follows_the_model_after_it_was_replaced_twice_unnoticed() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    for round in 0..60 {
        let z = 0.001 * (3 * round + 1) as f32;
        lift(&mut s, z);
        let first = s.bake_input().unwrap();
        assert_eq!(first.hash(), fresh_hash(&s));
        // 誰も入力を求めないうちに、モデルが 2 回替わる（旧モデルの割り当ては次のモデルに使われやすい）
        lift(&mut s, z + 0.0005);
        lift(&mut s, z + 0.001);
        let now = s.bake_input().unwrap();
        assert_eq!(
            now.hash(),
            fresh_hash(&s),
            "{round} 回目: 古い形の入力を返した"
        );
        assert_ne!(now.hash(), first.hash());
    }
}

#[test]
fn the_maps_are_stale_when_the_model_was_replaced_twice_without_anyone_asking() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    quick(&mut s);
    for round in 0..12 {
        s.apply(bake(BakeAction::Start));
        s.wait_bake();
        assert_eq!(
            s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
            MeshMapState::Current,
            "{round} 回目"
        );
        // ウィンドウは閉じていて、マップがあるので入力は残っている。そのまま 2 回替わる
        let z = 0.01 * (2 * round + 1) as f32;
        lift(&mut s, z);
        lift(&mut s, z + 0.01);
        let check = s.mesh_map_check(0, MeshMapKind::Position).unwrap();
        assert_eq!(check.state, MeshMapState::Stale, "{round} 回目");
        assert_eq!(check.reasons, [MeshMapStaleReason::ShapeChanged]);
        assert!(
            matches!(s.occlusion_for_export(0), Occlusion::Stale(_)),
            "{round} 回目: 古い AO を書き出しに使う"
        );
    }
}

#[test]
fn a_result_is_discarded_when_the_model_was_replaced_twice_while_baking() {
    for round in 0..20 {
        let mut s = AppState::new(64, 64);
        s.bake.backend = BakeBackend::Cpu;
        let _ = s.receive_link_model(&two_quads(1, 0.0));
        quick(&mut s);
        s.apply(bake(BakeAction::Start));
        assert!(s.bake.is_baking());
        // 焼いている間に 2 回替わる（偶数回で元のアドレスに戻りやすい）
        lift(&mut s, 0.1);
        lift(&mut s, 0.2);
        s.wait_bake();
        assert!(
            s.sets.iter().all(|x| x.mesh_maps.is_empty()),
            "{round} 回目: 古い形の結果を入れた"
        );
        assert!(s.message.contains("捨てました"), "{}", s.message);
        assert!(!s.modified);
    }
}

#[test]
fn the_window_builds_the_input_in_another_thread_and_waits_for_the_latest_model() {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    quick(&mut s);
    // 作っている最中は None（ウィンドウは「確認中」）。入力の理由で断らない
    assert!(s.bake_input_nowait().is_none());
    assert!(s.bake.is_checking());
    assert_eq!(s.bake_refusal_nowait(), None);
    assert!(s.bake.input.is_none(), "このスレッドでは作っていない");
    let first = wait_input(&mut s).unwrap();
    assert!(!s.bake.is_checking());
    assert_eq!(first.hash(), fresh_hash(&s));
    assert!(
        s.bake_input_nowait().is_some(),
        "同じモデルなら作り直さない"
    );

    // モデルが替わると、また別のスレッドで作り直す（その間もウィンドウは止まらない）
    lift(&mut s, 0.3);
    assert!(s.bake_input_nowait().is_none());
    assert!(s.bake.is_checking());
    // 作っている間にまた替わっても、古い形は入れず、最新のモデルで作り直す
    lift(&mut s, 0.4);
    let latest = wait_input(&mut s).unwrap();
    assert_eq!(latest.hash(), fresh_hash(&s));
    assert_ne!(latest.hash(), first.hash());

    // 始める・書き出すときの入力は、作っている最中なら終わりを待つ（二重に作らない）
    lift(&mut s, 0.5);
    assert!(s.bake_input_nowait().is_none());
    let waited = s.bake_input().unwrap();
    assert_eq!(waited.hash(), fresh_hash(&s));
    assert!(!s.bake.is_checking());

    // ウィンドウを閉じていれば、作っている最中のものは手放す
    lift(&mut s, 0.6);
    assert!(s.bake_input_nowait().is_none());
    s.release_idle_bake_input();
    assert!(!s.bake.is_checking());
}

/// 焼いたマップのある 2 枚の板の状態（ウィンドウは閉じている）から、.ylp を開き直してモデルを読み終えた状態にする: モデルは作り直され（入力は前の形のまま）、
/// 照合し直しの印が付く。
fn reopened_with_maps() -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    quick(&mut s);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert!(!s.sets.current().mesh_maps.is_empty());
    s.release_idle_bake_input();
    assert!(s.bake.input.is_some(), "焼いたマップがあるので入力は残る");
    lift(&mut s, 0.3);
    s.expect_reopen_check();
    s
}

/// 画面の毎フレーム（入力を求める → 使わない入力を手放す）。
fn one_frame(s: &mut AppState) {
    let _ = s.bake_input_nowait();
    s.release_idle_bake_input();
}

#[test]
fn the_reopen_check_keeps_the_building_input_until_it_arrives_and_then_lets_go() {
    let mut s = reopened_with_maps();
    assert!(s.bake.window.is_none());
    // ウィンドウが閉じていても、読み終えたモデルの入力ができるまで作りかけを手放さない
    let start = Instant::now();
    while s.bake.reopen_check.is_some() {
        one_frame(&mut s);
        assert!(start.elapsed().as_secs() < 120, "入力ができない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let model = s.view3d.full_model().unwrap().clone();
    assert!(
        s.bake.input.as_ref().is_some_and(|c| c.model.is(&model)),
        "読み終えたモデルの入力ができている"
    );
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Stale,
        "焼いたあとに形を替えたので、本当に古い"
    );
    // 済んだら今までどおり: ポーズ（モデルの作り直し）は追わず、作りかけは手放す
    lift(&mut s, 0.6);
    one_frame(&mut s);
    assert!(!s.bake.is_checking(), "作りかけを持ち続けない");
    assert!(
        s.bake.input.as_ref().is_some_and(|c| c.model.is(&model)),
        "前の入力のまま"
    );
}

#[test]
fn the_reopen_check_does_not_follow_a_model_that_changes_before_the_input_arrives() {
    let mut s = reopened_with_maps();
    assert!(s.bake_input_nowait().is_none());
    assert!(s.bake.is_checking());
    // 照合するモデルから替わった（ポーズを付けた・差し替えた）: 追わずに手放す
    lift(&mut s, 0.6);
    s.release_idle_bake_input();
    assert!(s.bake.reopen_check.is_none());
    assert!(!s.bake.is_checking());
}

#[test]
fn the_reopen_check_is_for_a_project_with_baked_maps_and_a_loaded_model_only() {
    // 試しの立方体は読み終えたモデルではない
    let mut s = cube();
    s.expect_reopen_check();
    assert!(s.bake.reopen_check.is_none());
    // 焼いたマップが無ければ、印は最初のフレームで外れる
    let mut t = AppState::new(64, 64);
    t.bake.backend = BakeBackend::Cpu;
    let _ = t.receive_link_model(&two_quads(1, 0.0));
    t.expect_reopen_check();
    assert!(t.bake.reopen_check.is_some());
    assert!(t.bake_input_nowait().is_none());
    t.release_idle_bake_input();
    assert!(t.bake.reopen_check.is_none());
    assert!(!t.bake.is_checking(), "マップが無ければ作りかけも手放す");
    // 別のプロジェクトになったら無効
    let mut u = reopened_with_maps();
    assert!(u.bake_input_nowait().is_none());
    u.np.generation += 1;
    u.release_idle_bake_input();
    assert!(u.bake.reopen_check.is_none());
    assert!(!u.bake.is_checking());
}

#[test]
fn a_failed_input_is_remembered_and_shown_as_the_reason() {
    use crate::view3d::model::ViewModel;
    // UV が有限でない（3D ビューは受けるが、ベイクの入力は作れない）
    let valid = ViewModel::demo(1);
    let mut meshes = valid.meshes.clone();
    meshes[0].uvs[0] = yolu_core::glam::Vec2::new(f32::NAN, 0.0);
    let broken = ViewModel::with_geometry(
        &valid.name,
        meshes,
        valid.materials.clone(),
        valid.geometry.clone(),
    );
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.view3d.set_model(broken);
    quick(&mut s);
    let failed = wait_input(&mut s);
    assert!(failed.is_err());
    let again = s.bake_input_nowait();
    assert!(
        matches!(again, Some(Err(_))),
        "失敗も覚える（作り直さない）"
    );
    assert!(!s.bake.is_checking());
    let reason = s.bake_refusal_nowait().expect("入力の理由で断る");
    assert!(reason.starts_with("ベイクできません"), "{reason}");
    s.apply(bake(BakeAction::Start));
    assert!(!s.bake.is_baking());
    assert!(s.message.starts_with("ベイクできません"), "{}", s.message);
}

#[test]
fn the_id_page_says_one_color_only_when_the_last_bake_had_one_part() {
    use yolu_core::mesh_maps::MeshIdSource;
    let status =
        |s: &AppState, kind| window::id_status(s.lang, kind, s.sets.current().mesh_maps.report());
    // 立方体は 1 つのスロット: スロットごとなら 1 色
    let mut s = cube();
    s.bake.settings.maps = vec![MeshMapKind::Id];
    s.bake.settings.id_source = MeshIdSource::MaterialSlot;
    assert_eq!(status(&s, MeshMapKind::Id), None, "焼く前は何も言わない");
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(s.sets.current().mesh_maps.report().unwrap().id_parts, 1);
    let text = status(&s, MeshMapKind::Id).expect("部品が 1 つ");
    assert!(text.contains("1 色"), "{text}");
    assert_eq!(status(&s, MeshMapKind::Position), None, "ID のページだけ");
    s.lang = Lang::En;
    assert_eq!(
        status(&s, MeshMapKind::Id).as_deref(),
        Some("The last bake had one part, so the ID is one color")
    );
    // 2 枚の板（同じマテリアルでも別のスロット）: 高ポリが無くても板ごとに別の色。1 色とは言わない
    let mut t = AppState::new(64, 64);
    t.bake.backend = BakeBackend::Cpu;
    let _ = t.receive_link_model(&corner_model());
    quick(&mut t);
    t.bake.settings.maps = vec![MeshMapKind::Id];
    for source in [MeshIdSource::MaterialSlot, MeshIdSource::UvIsland] {
        t.bake.settings.id_source = source;
        t.apply(bake(BakeAction::Start));
        t.wait_bake();
        assert_eq!(
            t.sets.current().mesh_maps.report().unwrap().id_parts,
            2,
            "{source:?}"
        );
        assert_eq!(status(&t, MeshMapKind::Id), None, "{source:?}");
    }
}

// ───────── 焼く場所（自動・GPU・CPU） ─────────

/// GPU の確認（別のスレッド）が終わるまで待つ。
fn wait_probe(s: &AppState) -> Result<BakeAdapter, String> {
    let start = Instant::now();
    loop {
        if let GpuProbe::Done { result, .. } = s.bake.gpu_probe() {
            return result;
        }
        assert!(
            start.elapsed().as_secs() < 120,
            "GPU の確認が終わらない（ハング検出上限）"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn has_japanese(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xff00..=0xffef)
    })
}

fn adapter(software: bool, ray_query: bool) -> BakeAdapter {
    BakeAdapter {
        name: "試験のアダプター".into(),
        backend: "Vulkan".into(),
        device_type: "DiscreteGpu".into(),
        software,
        ray_query,
    }
}

fn stats(method: GpuBakeMethod) -> yolu_gpu::GpuBakeStats {
    yolu_gpu::GpuBakeStats {
        method,
        dispatches: 3,
        max_dispatch_ms: 1.0,
        max_dispatch_texels: 256,
        bands: 1,
        input_bytes: 0,
        band_bytes: 0,
        ray_query_note: None,
        ray_query_why: None,
    }
}

#[test]
fn the_default_is_auto_and_choosing_cpu_never_probes_the_gpu() {
    let mut s = AppState::new(64, 64);
    assert_eq!(s.bake.backend, BakeBackend::Auto, "既定は自動");
    s.apply(bake(BakeAction::Backend(BakeBackend::Cpu)));
    assert_eq!(s.bake.backend, BakeBackend::Cpu);
    assert!(matches!(s.bake.gpu_probe(), GpuProbe::Unknown));
    assert!(!s.bake.is_probing_gpu());
    assert!(probe_line(Lang::Ja, BakeBackend::Cpu, &s.bake.gpu_probe()).is_none());
}

#[test]
fn choosing_a_backend_probes_the_gpu_in_another_thread_and_a_different_choice_probes_again() {
    let mut s = AppState::new(64, 64);
    s.apply(bake(BakeAction::Backend(BakeBackend::Gpu)));
    let first = wait_probe(&s);
    match s.bake.gpu_probe() {
        GpuProbe::Done { allow_software, .. } => assert!(
            allow_software,
            "「GPU」はソフトウェアの描画も許して確かめる"
        ),
        other => panic!("{other:?}"),
    }
    if let Ok(a) = &first {
        assert!(!a.name.is_empty());
    }
    // 自動へ変えると、ソフトウェアを許さない条件でもう一度確かめる
    s.apply(bake(BakeAction::Backend(BakeBackend::Auto)));
    let second = wait_probe(&s);
    match s.bake.gpu_probe() {
        GpuProbe::Done { allow_software, .. } => assert!(!allow_software),
        other => panic!("{other:?}"),
    }
    if let Ok(a) = &second {
        assert!(!a.software, "自動はソフトウェアの描画を使わない: {a:?}");
    }
    // ウィンドウの一行は確かめた結果を言う
    let line = probe_line(Lang::Ja, BakeBackend::Auto, &s.bake.gpu_probe()).unwrap();
    assert_eq!(line.warn, second.is_err());
    assert_eq!(line.detail.is_some(), second.is_err());
    assert!(
        line.text.starts_with(if second.is_ok() {
            "GPU:"
        } else {
            "CPU で焼く"
        }),
        "{}",
        line.text
    );
}

#[test]
fn a_cpu_bake_says_so_and_a_gpu_bake_covers_the_same_texels_with_close_values_and_says_where_it_ran(
) {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let cpu_run = s
        .sets
        .current()
        .mesh_maps
        .run()
        .cloned()
        .expect("記録がある");
    assert!(
        !cpu_run.used_gpu()
            && cpu_run.fallback_kind.is_none()
            && cpu_run.requested == BakeBackend::Cpu
    );
    assert!(s.message.contains("CPU）"), "{}", s.message);
    let cpu_cover: Vec<_> = s
        .sets
        .current()
        .mesh_maps
        .iter()
        .map(|m| (m.kind(), m.coverage().to_vec()))
        .collect();
    let cpu_values: Vec<_> = s
        .sets
        .current()
        .mesh_maps
        .iter()
        .map(|m| (m.kind(), m.data().to_vec()))
        .collect();
    let cpu_line = run_line(Lang::Ja, &cpu_run);
    assert_eq!(
        (cpu_line.text.as_str(), cpu_line.warn, cpu_line.detail),
        ("CPU", false, None),
        "CPU を選んだ CPU は注意にしない"
    );

    s.apply(bake(BakeAction::Backend(BakeBackend::Gpu)));
    let probe = wait_probe(&s);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let run = s
        .sets
        .current()
        .mesh_maps
        .run()
        .cloned()
        .expect("記録がある");
    assert_eq!(run.requested, BakeBackend::Gpu);
    let line = run_line(Lang::Ja, &run);
    match probe {
        Ok(a) => {
            let (used, _) = run.gpu.as_ref().expect("確かめて使えたので GPU で焼く");
            assert_eq!(used.name, a.name);
            assert!(run.fallback_kind.is_none(), "{:?}", run.fallback_reason);
            assert!(s.message.contains("GPU "), "{}", s.message);
            assert!(line.text.starts_with("GPU ") && !line.warn, "{}", line.text);
            // どこで焼いても、テクセルの由来（UV の覆い）は同じ
            for (kind, cover) in &cpu_cover {
                let map = s.sets.current().mesh_maps.get(*kind).unwrap();
                assert_eq!(
                    map.coverage(),
                    cover.as_slice(),
                    "{kind:?}: 覆いは CPU と同じ"
                );
            }
            // 値は全バイト一致ではなく許し幅の内（根拠は yolu-gpu の README の「CPU との差」）: 光線を飛ばさないマップは 16 bit の
            // 値で最大 8、AO は光線 8 本なので 1 本の当たり外れで 12.5% 動くため、5% を超えるテクセルの割合と平均で見る
            for (kind, cpu) in &cpu_values {
                let map = s.sets.current().mesh_maps.get(*kind).unwrap();
                let diffs: Vec<u32> = cpu
                    .iter()
                    .zip(map.data())
                    .map(|(a, b)| u32::from(a.abs_diff(*b)))
                    .collect();
                let max = diffs.iter().copied().max().unwrap_or(0);
                let mean = diffs.iter().map(|d| f64::from(*d)).sum::<f64>() / diffs.len() as f64;
                let over = diffs.iter().filter(|d| **d > 3277).count() as f64 / diffs.len() as f64;
                if *kind == MeshMapKind::AmbientOcclusion {
                    assert!(mean < 655.0 && over < 0.01, "AO: 平均 {mean}、5% 超 {over}");
                } else {
                    assert!(max <= 8, "{kind:?}: 最大差 {max}");
                }
                // 試しの立方体は凸なので AO はどこも 1。そのほかは値が一様でない（比べる相手も自分も一様な試験にしない）
                if *kind != MeshMapKind::AmbientOcclusion {
                    assert!(
                        map.data().iter().any(|v| *v != map.data()[0]),
                        "{kind:?}: 値が一様でない"
                    );
                }
            }
        }
        Err(reason) => {
            assert!(!run.used_gpu());
            assert_eq!(run.fallback_kind, Some(FallbackKind::Unavailable));
            assert!(
                run.fallback_reason
                    .as_deref()
                    .is_some_and(|r| !r.is_empty()),
                "{reason}"
            );
            assert!(s.message.contains("GPU を使えません"), "{}", s.message);
            assert!(line.warn && line.detail.is_some());
        }
    }
    // どちらで焼いても、今の条件で焼いたものなので Current
    for kind in kinds(&s, 0) {
        let check = s.mesh_map_check(0, kind).unwrap();
        assert_eq!(
            check.state,
            MeshMapState::Current,
            "{kind:?}: {:?}",
            check.reasons
        );
    }
    assert!(!s.doc.can_undo(), "文書は変わらない");
}

#[test]
fn a_broken_gpu_falls_back_to_the_cpu_with_the_reason_and_still_bakes() {
    let mut s = cube();
    s.apply(bake(BakeAction::Backend(BakeBackend::Gpu)));
    if wait_probe(&s).is_err() {
        eprintln!("GPU を作れない環境なので、壊れた GPU の試験は省く");
        return;
    }
    s.bake.gpu.fail_for_test("試験の故障");
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let run = s.sets.current().mesh_maps.run().cloned().unwrap();
    assert!(!run.used_gpu());
    assert_eq!(run.fallback_kind, Some(FallbackKind::Failed));
    assert!(run
        .fallback_reason
        .as_deref()
        .unwrap()
        .contains("試験の故障"));
    assert_eq!(kinds(&s, 0).len(), 3, "CPU で焼き直して、マップは入る");
    assert!(
        s.message.contains("GPU の処理に失敗しました"),
        "{}",
        s.message
    );
    assert!(s.bake.outcome.as_ref().is_some_and(|(_, ok)| *ok));
    // 次のベイクは壊れた GPU を作り直して GPU に戻る
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let run = s.sets.current().mesh_maps.run().cloned().unwrap();
    assert!(
        run.used_gpu(),
        "作り直して GPU に戻る: {:?}",
        run.fallback_reason
    );
}

/// 今のマップとその記録・場所（取消のあとも前のままであることを比べる）。
fn snapshot(
    s: &AppState,
) -> (
    Vec<std::sync::Arc<yolu_core::mesh_maps::BakedMeshMap>>,
    String,
    String,
) {
    let set = &s.sets.current().mesh_maps;
    (
        set.iter().cloned().collect(),
        format!("{:?}", set.report()),
        format!("{:?}", set.run()),
    )
}
fn assert_unchanged(
    before: &(
        Vec<std::sync::Arc<yolu_core::mesh_maps::BakedMeshMap>>,
        String,
        String,
    ),
    now: &AppState,
) {
    let after = snapshot(now);
    assert!(
        after.0.len() == before.0.len()
            && after
                .0
                .iter()
                .zip(&before.0)
                .all(|(a, b)| std::sync::Arc::ptr_eq(a, b)),
        "取り消したら前のマップのまま"
    );
    assert_eq!(after.1, before.1, "記録は前のマップのまま");
    assert_eq!(after.2, before.2, "場所も前のマップのまま");
}

/// 準備を終えて、焼き始めた（"Baking"）ところまで待つ。
fn wait_until_baking(s: &AppState) {
    let start = Instant::now();
    while s.bake.progress().is_some_and(|p| p.phase != "Baking") {
        assert!(start.elapsed().as_secs() < 60, "準備が終わらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// CPU で焼いたマップのあとに、GPU を選んだベイクを始め（`park` で止めてから）取り消す。
fn cancel_a_gpu_bake_after_a_cpu_bake(park_mid_bake: bool) {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let before = snapshot(&s);
    let run = s.sets.current().mesh_maps.run().expect("前の場所");
    assert!(!run.used_gpu() && run.requested == BakeBackend::Cpu);
    s.apply(bake(BakeAction::Backend(BakeBackend::Gpu)));
    if wait_probe(&s).is_err() {
        eprintln!("GPU を作れない環境なので、GPU の取消の試験は省く");
        return;
    }
    s.bake.settings.maps = vec![MeshMapKind::AmbientOcclusion];
    // 準備の途中（GPU はまだ動かない）か、GPU が最初の dispatch を終えたあと（焼いている途中）で止めておいて取り消す
    if park_mid_bake {
        s.bake.park_mid_bake = true;
    } else {
        s.bake.park_next = true;
    }
    s.apply(bake(BakeAction::Start));
    assert!(s.bake.is_baking());
    if park_mid_bake {
        wait_until_baking(&s);
    }
    s.apply(bake(BakeAction::Cancel));
    s.wait_bake();
    assert!(s.message.contains("取り消しました"), "{}", s.message);
    // 取り消した試み（準備の途中でも、GPU が動いている途中でも）は、前のマップ・記録・場所を変えない
    assert_unchanged(&before, &s);
    assert!(!s.sets.current().mesh_maps.run().unwrap().used_gpu());
}

#[test]
fn canceling_a_gpu_bake_while_preparing_keeps_the_previous_maps_and_where_they_were_baked() {
    cancel_a_gpu_bake_after_a_cpu_bake(false);
}

#[test]
fn canceling_a_gpu_bake_while_the_gpu_runs_keeps_the_previous_maps_and_where_they_were_baked() {
    cancel_a_gpu_bake_after_a_cpu_bake(true);
}

#[test]
fn canceling_a_cpu_bake_while_it_runs_leaves_the_record_of_the_maps_that_stay() {
    let mut s = cube();
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    let before = snapshot(&s);
    s.bake.settings.maps = vec![MeshMapKind::AmbientOcclusion];
    s.bake.park_mid_bake = true;
    s.apply(bake(BakeAction::Start));
    wait_until_baking(&s);
    s.apply(bake(BakeAction::Cancel));
    s.wait_bake();
    assert!(s.message.contains("取り消しました"), "{}", s.message);
    assert_unchanged(&before, &s);
}

#[test]
fn the_ray_query_setting_reaches_the_device_and_a_change_forgets_the_gpu_check() {
    let s = AppState::new(64, 64);
    // 既定は入（環境変数が切にしているときは、設定が入でも切のまま）
    let allowed = yolu_gpu::GpuBakeOptions::default().ray_query;
    assert!(s.prefs.settings.bake_ray_query);
    s.bake.follow_ray_query(s.prefs.settings.bake_ray_query);
    assert_eq!(s.bake.ray_query_enabled(), allowed);
    s.bake.fix_gpu_probe(false, Err("確かめ済み".into()));
    // 同じ値なら確認の結果を捨てない
    s.bake.follow_ray_query(allowed);
    assert!(matches!(s.bake.gpu_probe(), GpuProbe::Done { .. }));
    // 切ると、デバイスの設定が切になり、確認の結果を捨てる（RT コアの有無が変わるので確かめ直す）
    s.bake.follow_ray_query(false);
    assert!(!s.bake.ray_query_enabled());
    if allowed {
        assert!(matches!(s.bake.gpu_probe(), GpuProbe::Unknown));
    }
    // 入れ直すと、環境変数が許すときだけ入になる
    s.bake.follow_ray_query(true);
    assert_eq!(s.bake.ray_query_enabled(), allowed);
}

#[test]
fn the_reason_the_rt_cores_were_not_used_is_a_short_sentence_in_the_language_without_numbers() {
    use yolu_gpu::RayQueryWhy::*;
    let all = [
        Disabled,
        NotSupported,
        NotApplicable,
        Device,
        Shader,
        Accel,
        CheckRun,
        CheckFailed,
        RunFailed,
    ];
    for why in all {
        let mut st = stats(GpuBakeMethod::Compute);
        st.ray_query_why = Some(why);
        // 詳しい文（数を含む）は、あっても画面に出さない
        st.ray_query_note = Some("compute（ray query を使わない理由: レイ 2048 本、食い違い 3、最大差 2.47e-6、許す数 10）".into());
        let run = BakeRun {
            requested: BakeBackend::Auto,
            gpu: Some((adapter(false, true), st)),
            fallback_kind: None,
            fallback_reason: None,
        };
        let (ja, en) = (run_line(Lang::Ja, &run), run_line(Lang::En, &run));
        for line in [&ja, &en] {
            assert!(!line.warn, "RT コアを使わなかっただけでは注意にしない");
            assert!(
                line.text.contains("compute") && !line.text.contains("ray query"),
                "{}",
                line.text
            );
        }
        let (dj, de) = (ja.detail.unwrap(), en.detail.unwrap());
        assert!(
            has_japanese(&dj) && !has_japanese(&de),
            "{why:?}: {dj} / {de}"
        );
        for d in [&dj, &de] {
            assert!(
                !d.chars().any(|c| c.is_ascii_digit()),
                "{why:?}: 開発用の数を出さない: {d}"
            );
            assert!(!d.contains("2048") && !d.contains("e-6"), "{d}");
        }
    }
    // 設定で切ると環境変数で切るは、別の文
    let off = |env| ray_query_why_text(Lang::En, Disabled, env);
    assert!(off(true).contains("environment variable") && off(false).contains("settings"));
    // 使えたときは何も出さない。RT コアが関わらない（理由なし）ときも出さない
    let used = BakeRun {
        requested: BakeBackend::Auto,
        gpu: Some((adapter(false, true), stats(GpuBakeMethod::RayQuery))),
        fallback_kind: None,
        fallback_reason: None,
    };
    let line = run_line(Lang::Ja, &used);
    assert!(line.text.contains("ray query") && line.detail.is_none());
    let none = BakeRun {
        gpu: Some((adapter(false, false), stats(GpuBakeMethod::Compute))),
        ..used
    };
    assert_eq!(run_line(Lang::Ja, &none).detail, None);
}

#[test]
fn a_stale_gpu_check_is_not_written_back_after_the_setting_changed() {
    let s = AppState::new(64, 64);
    // 確認の途中の世代が古くなったら（設定が変わったら）、その結果を捨てる: 世代は、値が変わるたびに進む
    let before = s.bake.probe_generation_for_test();
    s.bake.follow_ray_query(true);
    assert_eq!(
        s.bake.probe_generation_for_test(),
        before,
        "同じ値では進めない"
    );
    let allowed = yolu_gpu::ray_query_env_allows();
    s.bake.follow_ray_query(false);
    if allowed {
        assert_eq!(s.bake.probe_generation_for_test(), before + 1);
    }
}

#[test]
fn where_it_baked_reads_in_both_languages_without_mixing_them() {
    let hw = adapter(false, true);
    let rq = BakeRun {
        requested: BakeBackend::Auto,
        gpu: Some((hw.clone(), stats(GpuBakeMethod::RayQuery))),
        fallback_kind: None,
        fallback_reason: None,
    };
    let compute = BakeRun {
        gpu: Some((adapter(true, false), stats(GpuBakeMethod::Compute))),
        ..rq.clone()
    };
    assert!(run_line(Lang::Ja, &rq).text.contains("ray query"));
    assert!(run_line(Lang::Ja, &compute).text.contains("ソフトウェア"));
    assert!(run_line(Lang::En, &compute).text.contains("software"));
    assert!(run_line(Lang::En, &compute).text.contains("compute"));
    for kind in [
        FallbackKind::Unavailable,
        FallbackKind::Budget,
        FallbackKind::Failed,
    ] {
        let run = BakeRun {
            requested: BakeBackend::Gpu,
            gpu: None,
            fallback_kind: Some(kind),
            fallback_reason: Some("詳細の文".into()),
        };
        let ja = run_line(Lang::Ja, &run);
        let en = run_line(Lang::En, &run);
        assert!(ja.warn && en.warn);
        assert!(
            ja.text.starts_with("CPU（") && en.text.starts_with("CPU ("),
            "{} / {}",
            ja.text,
            en.text
        );
        assert!(
            !has_japanese(&en.text),
            "英語の画面に日本語を混ぜない: {}",
            en.text
        );
        assert_eq!(
            en.detail.as_deref(),
            Some("詳細の文"),
            "詳細はツールチップへ"
        );
        assert_ne!(fallback_text(Lang::Ja, kind), fallback_text(Lang::En, kind));
    }
    // 確かめの一行
    let none: Result<BakeAdapter, String> = Err("アダプターがありません".into());
    let done = |allow_software, result| GpuProbe::Done {
        allow_software,
        result,
    };
    let en = probe_line(Lang::En, BakeBackend::Auto, &done(false, none.clone())).unwrap();
    assert!(en.warn && !has_japanese(&en.text), "{}", en.text);
    let ok = probe_line(Lang::En, BakeBackend::Gpu, &done(true, Ok(hw))).unwrap();
    assert!(!ok.warn && ok.text.contains("ray query"), "{}", ok.text);
    // 選びを変えた直後（違う条件の結果しかない）は確認中
    let waiting = probe_line(Lang::Ja, BakeBackend::Gpu, &done(false, none)).unwrap();
    assert_eq!(waiting.text, "GPU を確認中");
    for lang in Lang::ALL {
        for backend in [BakeBackend::Auto, BakeBackend::Gpu, BakeBackend::Cpu] {
            assert!(
                !backend_label(lang, backend).is_empty() && !backend_help(lang, backend).is_empty()
            );
        }
    }
}

/// 焼けたが気をつけること（UV の面積が 0 の三角形・縮退・UV の重なり）は 1 つの文で、数を書かない。無ければ None。
#[test]
fn the_uv_warning_has_no_numbers_and_is_absent_for_a_clean_bake() {
    use yolu_core::mesh_maps::MeshBakeReport;
    let clean = MeshBakeReport::default();
    for lang in Lang::ALL {
        assert_eq!(super::uv_warning(lang, &clean), None);
        let report = MeshBakeReport {
            zero_uv_area_triangles: 12,
            degenerate_triangles: 3,
            overlap_texels: 4567,
            ..MeshBakeReport::default()
        };
        let text = super::uv_warning(lang, &report).expect("注意");
        for count in ["12", "3", "4567"] {
            assert!(!text.contains(count), "数を書かない: {text}");
        }
        assert_eq!(
            text.matches(lang.pick("。", ".")).count(),
            3,
            "3 つの文: {text}"
        );
    }
    let only_overlap = MeshBakeReport {
        overlap_texels: 1,
        ..MeshBakeReport::default()
    };
    assert_eq!(
        super::uv_warning(Lang::Ja, &only_overlap).unwrap(),
        "UV が重なる所があり、重なったテクセルにはどちらか一方の面だけが焼かれます。"
    );
}

/// 2 枚の板のうち `flat` 番目に、UV の面積が 0 で形のある三角形を足したモデル。
fn two_quads_with_flat_uv(flat: &[usize]) -> Model {
    let mut model = two_quads(1, 0.0);
    for &mesh in flat {
        let mesh = &mut model.meshes[mesh];
        let first = mesh.positions.len() as u32;
        let u = mesh.uv0[0][0] + 0.1;
        mesh.positions
            .extend([[0.1, 0.1, 0.5], [0.3, 0.1, 0.5], [0.1, 0.3, 0.5]]);
        mesh.uv0.extend([[u, 0.5]; 3]);
        mesh.submeshes[0]
            .indices
            .extend([first, first + 1, first + 2]);
    }
    model
}

/// 2 つのセットを焼いたあとの状態（`flat` の板に UV の面積が 0 の三角形がある）。
fn baked_two_sets(flat: &[usize]) -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let (_, shape) = s.receive_link_model(&two_quads_with_flat_uv(flat));
    assert_eq!(shape, Ok(()));
    quick(&mut s);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(kinds(&s, 0).len(), 3);
    assert_eq!(kinds(&s, 1).len(), 3);
    s
}

/// 複数のセットのまとめの知らせは、どれかのセットに UV の注意があれば注意のまま、その文も添える（最後のセットが無事でも、
/// 最初のセットの注意をまとめの済んだ知らせで上書きして消さない。トーストとウィンドウの下の帯の両方）。同じ注意は 1 回だけ。
#[test]
fn the_summary_of_several_sets_keeps_the_uv_warning_of_any_set() {
    use crate::notice::Kind;
    let warning = "UV の面積が 0 の三角形があり、その面は焼けません。";
    for (flat, count) in [(vec![0], 1), (vec![1], 1), (vec![0, 1], 1), (vec![], 0)] {
        let s = baked_two_sets(&flat);
        assert!(
            s.message
                .starts_with("2 個のテクスチャセットのメッシュマップを焼きました。"),
            "{flat:?}: {}",
            s.message
        );
        assert_eq!(
            s.message.matches(warning).count(),
            count,
            "{flat:?}: {}",
            s.message
        );
        let expected = if flat.is_empty() {
            Kind::Info
        } else {
            Kind::Warning
        };
        assert_eq!(s.message_kind(), expected, "{flat:?}: {}", s.message);
        let (outcome, ok) = s.bake.outcome.clone().expect("ウィンドウの下の結果");
        assert_eq!((outcome.as_str(), ok), (s.message.as_str(), true));
        // ログのウィンドウには、セットごとの注意とまとめの注意（種類は注意）が入る
        let logged = s
            .notice_log
            .entries()
            .filter(|e| e.notice.text.contains(warning))
            .count();
        assert_eq!(
            logged,
            flat.len() + usize::from(!flat.is_empty()),
            "{flat:?}"
        );
    }
}

/// 次に焼くときは、前の並びの注意を引き継がない。
#[test]
fn the_uv_warning_of_the_previous_bake_is_not_carried_to_the_next() {
    let warning = "UV の面積が 0 の三角形があり、その面は焼けません。";
    let mut s = baked_two_sets(&[0]);
    assert!(s.message.contains(warning), "{}", s.message);
    let (_, shape) = s.receive_link_model(&two_quads_with_flat_uv(&[]));
    assert_eq!(shape, Ok(()));
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert!(
        s.message.starts_with("2 個のテクスチャセット"),
        "{}",
        s.message
    );
    assert!(!s.message.contains(warning), "{}", s.message);
}

#[test]
fn the_export_window_follows_the_input_for_a_baked_ao_and_builds_it_once() {
    use crate::export::{ExportAction, ExportForm};
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    let _ = s.receive_link_model(&two_quads(1, 0.0));
    quick(&mut s);
    s.apply(bake(BakeAction::Start));
    s.wait_bake();
    assert!(!s.sets.current().mesh_maps.is_empty());
    s.release_idle_bake_input();
    // 形が変わって（ポーズ）、持っている入力は前の形のもの。ベイクのウィンドウは閉じている
    lift(&mut s, 0.3);
    assert!(s.bake.window.is_none());
    // 書き出しのウィンドウを開いて出力テンプレートにするだけでは、焼いた AO を照合するための入力を追う（作りかけを毎フレーム手放さない）
    s.apply(Action::Export(ExportAction::OpenWindow));
    s.apply(Action::Export(ExportAction::SetForm(
        ExportForm::UnityStandard,
    )));
    assert!(s.export_window_follows_input());
    let before = s.bake.input_builds_started();
    let model = s.view3d.full_model().unwrap().clone();
    for _ in 0..500 {
        // 画面の毎フレーム（一覧を求める → 使わない入力を手放す）
        let _ = s.export_preview();
        s.release_idle_bake_input();
        if s.bake.input.as_ref().is_some_and(|c| c.model.is(&model)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        s.bake.input.as_ref().is_some_and(|c| c.model.is(&model)),
        "今のモデルの入力ができた"
    );
    assert!(!s.bake.is_checking());
    assert_eq!(
        s.bake.input_builds_started() - before,
        1,
        "作りかけを立てては捨てず、1 回で作り終える"
    );
    // 追わない場合: 出力テンプレートが今のチャンネルの PNG なら、AO は要らないので追わない
    s.apply(Action::Export(ExportAction::SetForm(
        ExportForm::ChannelPng,
    )));
    assert!(!s.export_window_follows_input());
    // ウィンドウを閉じても追わない
    s.apply(Action::Export(ExportAction::SetForm(ExportForm::LilToon)));
    assert!(s.export_window_follows_input());
    s.apply(Action::Export(ExportAction::CloseWindow));
    assert!(!s.export_window_follows_input());
    // チェックしたセットに焼いた AO が無ければ追わない
    s.apply(Action::Export(ExportAction::OpenWindow));
    for i in 0..s.sets.len() {
        let uid = s.sets.get(i).unwrap().uid;
        s.apply(Action::Export(ExportAction::SetChecked { uid, on: false }));
    }
    assert!(!s.export_window_follows_input());
}
