//! FBX のテイク（中のアニメ）から、テイクとフレームを選んでポーズにする（画面なし）: 一覧・選び・当てる（取り消しの 1 段・手で直せる）・
//! BlendShape・描いている間・読み直したファイルが変わっていたとき・`pose.json` に覚える選び。FBX はコードで組んだ ASCII の腕
//! （`fbx_ascii::arm_takes_scene`。実のデータは使わない）。まとめた Rig（Live Link）は `livelink_files.rs`、欄の画面は `gui_view3d/pose_takes.rs`。
use crate::common::livelink::fbx_ascii;

use std::path::{Path, PathBuf};
use std::time::Instant;

use yolu_app::state::{Action, AppState};
use yolu_app::view3d::pose::{self, stored, takes, PoseAction};
use yolu_core::glam::{Quat, Vec3};
use yolu_io::{Limits, Project};
use yolu_model::ModelLimits;

fn temp(tag: &str) -> PathBuf {
    crate::common::tmp::test_dir(&format!("pose-takes-{tag}"))
}

/// FBX を書いて裏で読み、3D ビューに入れる（プロジェクトのモデルのファイルとして）。
fn load(s: &mut AppState, path: &Path, scene: &fbx_ascii::Scene) {
    std::fs::write(path, scene.to_ascii()).unwrap();
    let job = pose::prepare_fbx(&mut s.view3d, path, ModelLimits::default());
    let start = Instant::now();
    let prepared = loop {
        if let Some(r) = job.poll() {
            break r.expect("読める");
        }
        assert!(start.elapsed().as_secs() < 120, "読み込みが終わらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    pose::install_prepared(&mut s.view3d, prepared);
    s.np.model_file = Some(path.to_path_buf());
}

fn session(s: &AppState) -> &pose::PoseSession {
    s.view3d.pose.session.as_ref().unwrap()
}

fn bone(s: &AppState, name: &str) -> usize {
    session(s)
        .rig
        .bones()
        .iter()
        .position(|b| b.name == name)
        .unwrap()
}

/// 選んで当て、終わるまで待つ。
fn apply(s: &mut AppState, take: usize, frame: i64) {
    s.apply(Action::Pose(PoseAction::ChooseTake(0, take)));
    takes::set_frame(s, frame);
    s.apply(Action::Pose(PoseAction::ApplyTake));
    takes::wait(s);
}

#[test]
fn headless_the_takes_of_an_fbx_are_listed_and_a_model_without_takes_has_none() {
    let dir = temp("list");
    let mut s = AppState::new(64, 64);
    load(&mut s, &dir.join("腕.fbx"), &fbx_ascii::arm_takes_scene());
    let t = &session(&s).takes;
    assert!(!t.is_empty());
    let names: Vec<String> = t.all().into_iter().map(|(a, b)| t.label(a, b)).collect();
    assert_eq!(names, ["Wave", "Raise"], "動かす値の無いテイクは入れない");
    assert_eq!(t.chosen(), Some((0, 0)), "初めは最初のテイク");
    assert_eq!(t.frame(), 0, "その始まり");
    load(&mut s, &dir.join("なし.fbx"), &fbx_ascii::arm_scene());
    assert!(session(&s).takes.is_empty());
    s.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(session(&s).takes.is_empty(), "試しの人形にもテイクは無い");
}

#[test]
fn headless_a_take_frame_becomes_the_pose_in_one_undo_step_and_can_be_edited_by_hand() {
    let dir = temp("apply");
    let mut s = AppState::new(64, 64);
    load(&mut s, &dir.join("腕.fbx"), &fbx_ascii::arm_takes_scene());
    let rest = session(&s).pose().clone();
    let lower = bone(&s, "Lower");
    let (arm_mesh, thick, bend) = (1, 0, 1);
    // Wave のフレーム 15（0.5 秒）: Lower の z は 45 度（Unity の座標で逆向き）、Bend は 50。Thick（テイクが動かさない）は今のまま
    apply(&mut s, 0, 15);
    assert!(s.message.contains("「Wave」のフレーム 15"), "{}", s.message);
    let p = session(&s).pose().clone();
    let want = Quat::from_rotation_z(-45f32.to_radians());
    assert!(p.locals[lower].rotation.dot(want).abs() > 0.99999);
    assert!((p.blend_weights[arm_mesh][bend] - 50.0).abs() < 1e-3);
    assert_eq!(
        p.blend_weights[arm_mesh][thick],
        rest.blend_weights[arm_mesh][thick]
    );
    assert_eq!(session(&s).undo_len(), 1, "取り消しの 1 段");
    // 当てたポーズを手で直せる（ほかの変更と同じ取り消しの段）
    let mut hand = p.clone();
    hand.locals[lower].rotation = Quat::from_rotation_x(0.3);
    pose::set_pose(&mut s.view3d, hand.clone()).unwrap();
    assert_eq!(session(&s).undo_len(), 2);
    // 別のテイク: テイクが動かさない骨は休みの値へ戻る（Raise は Lower の大きさと Upper の移動だけ）
    apply(&mut s, 1, 50);
    let p = session(&s).pose().clone();
    let upper = bone(&s, "Upper");
    assert!((p.locals[upper].translation - Vec3::new(0.0, 1.2, 0.0)).length() < 1e-5);
    assert_eq!(p.locals[lower].rotation, rest.locals[lower].rotation);
    let t = (50.0 / 30.0 - 0.5) / 1.5;
    assert!((p.locals[lower].scale.x - (1.0 + 0.5 * t)).abs() < 1e-4);
    assert_eq!(
        p.blend_weights[arm_mesh][bend], 50.0,
        "Raise は BlendShape を動かさない（前の値のまま）"
    );
    // 取り消すと手で直したポーズ、もう 1 度でテイクのポーズ、もう 1 度で休み
    s.apply(Action::Pose(PoseAction::Undo));
    assert_eq!(session(&s).pose(), &hand);
    s.apply(Action::Pose(PoseAction::Undo));
    s.apply(Action::Pose(PoseAction::Undo));
    assert_eq!(session(&s).pose(), &rest);
}

#[test]
fn headless_the_frame_stays_within_the_take_and_the_choice_marks_the_document_modified() {
    let dir = temp("range");
    let mut s = AppState::new(64, 64);
    load(&mut s, &dir.join("腕.fbx"), &fbx_ascii::arm_takes_scene());
    s.modified = false;
    takes::set_frame(&mut s, 99);
    assert_eq!(session(&s).takes.frame(), 30, "Wave は 0〜30");
    s.apply(Action::Pose(PoseAction::ChooseTake(0, 1)));
    assert_eq!(
        session(&s).takes.frame(),
        30,
        "Raise（15〜60）の中ならそのまま"
    );
    takes::set_frame(&mut s, -5);
    assert_eq!(session(&s).takes.frame(), 15);
    // 範囲の外の番号は選ばない
    s.apply(Action::Pose(PoseAction::ChooseTake(0, 9)));
    assert_eq!(session(&s).takes.chosen(), Some((0, 1)));
    pose::sync_modified(&mut s);
    assert!(s.modified, "選びは pose.json に残るので変更あり");
}

#[test]
fn headless_applying_waits_for_the_stroke_and_is_refused_while_stroking() {
    let dir = temp("stroke");
    let mut s = AppState::new(64, 64);
    load(&mut s, &dir.join("腕.fbx"), &fbx_ascii::arm_takes_scene());
    let rest = session(&s).pose().clone();
    let layer = s.selected_layer.unwrap();
    let settings = s.stroke_settings(false);
    // 描いている間は始めない
    let stroke = s.doc.begin_stroke(layer, &settings).unwrap();
    assert!(s.is_stroking());
    s.apply(Action::Pose(PoseAction::ApplyTake));
    assert!(!session(&s).takes.is_running());
    s.doc.cancel_stroke(stroke);
    // 始めてから描き始めたら、描き終わるまで当てない
    takes::set_frame(&mut s, 30);
    s.apply(Action::Pose(PoseAction::ApplyTake));
    assert!(session(&s).takes.is_running());
    let stroke = s.doc.begin_stroke(layer, &settings).unwrap();
    let start = Instant::now();
    while start.elapsed().as_millis() < 300 {
        takes::poll(&mut s);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(session(&s).pose(), &rest, "描いている間は当てない");
    assert!(session(&s).takes.is_running());
    s.doc.cancel_stroke(stroke);
    takes::wait(&mut s);
    assert_ne!(session(&s).pose(), &rest, "描き終えたら当てる");
}

#[test]
fn headless_a_file_changed_since_loading_is_refused_with_a_reason_and_the_pose_stays() {
    let dir = temp("changed");
    let path = dir.join("腕.fbx");
    let mut s = AppState::new(64, 64);
    load(&mut s, &path, &fbx_ascii::arm_takes_scene());
    let rest = session(&s).pose().clone();
    let mut renamed = fbx_ascii::arm_takes_scene();
    renamed.nodes[2].name = "Forearm".into();
    std::fs::write(&path, renamed.to_ascii()).unwrap();
    apply(&mut s, 0, 10);
    assert_eq!(
        s.message,
        "「Wave」のフレーム 10 をポーズにできません（ファイルが読み込んだときと変わっています）。"
    );
    assert_eq!(session(&s).pose(), &rest);
    assert_eq!(session(&s).undo_len(), 0);
    // ファイルが無い
    std::fs::remove_file(&path).unwrap();
    apply(&mut s, 0, 10);
    assert!(
        s.message
            .starts_with("「Wave」のフレーム 10 をポーズにできません（"),
        "{}",
        s.message
    );
    // 英語
    s.lang = yolu_app::lang::Lang::En;
    std::fs::write(&path, renamed.to_ascii()).unwrap();
    apply(&mut s, 0, 10);
    assert_eq!(
        s.message,
        "Cannot pose from frame 10 of \"Wave\" (The file has changed since it was loaded)."
    );
}

fn save_as(s: &mut AppState, path: &Path) {
    s.apply(Action::SaveProjectAs(path.to_path_buf()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
}

#[test]
fn headless_the_chosen_take_and_frame_are_kept_in_pose_json_and_come_back() {
    let dir = temp("saved");
    let fbx = dir.join("腕.fbx");
    let ylp = dir.join("a.ylp");
    let mut s = AppState::new(64, 64);
    load(&mut s, &fbx, &fbx_ascii::arm_takes_scene());
    // 最初のテイクの始まりのままなら書かない
    save_as(&mut s, &ylp);
    let project = Project::open(&ylp, &Limits::unbounded()).unwrap();
    assert!(project.pose().unwrap().is_none());
    // 選んで当てる → 保存
    apply(&mut s, 1, 40);
    let posed = session(&s).pose().clone();
    save_as(&mut s, &ylp);
    let project = Project::open(&ylp, &Limits::unbounded()).unwrap();
    let stored = project.pose().unwrap().unwrap();
    assert_eq!(
        stored.take,
        Some(yolu_io::pose::StoredTake {
            name: "Raise".into(),
            frame: 40
        })
    );
    // 開き直す: ポーズも選びも戻る（開いた直後は変更なし・取り消しなし）
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(ylp.clone()));
    assert!(again.project.is_some(), "{}", again.message);
    load(&mut again, &fbx, &fbx_ascii::arm_takes_scene());
    let note = stored::restore_from_project(&mut again);
    assert_eq!(note.as_deref(), Some("ポーズを戻しました。"));
    let t = &session(&again).takes;
    assert_eq!(t.chosen(), Some((0, 1)));
    assert_eq!(t.frame(), 40);
    let p = session(&again).pose();
    for (a, b) in p.locals.iter().zip(&posed.locals) {
        assert!((a.translation - b.translation).length() < 1e-5);
        assert!(a.rotation.dot(b.rotation).abs() > 0.99999);
    }
    assert_eq!(session(&again).undo_len(), 0);
    pose::sync_modified(&mut again);
    assert!(!again.modified);
    // テイクがモデルに無い: 選びは最初のテイクのまま、理由を残す
    let mut other = fbx_ascii::arm_takes_scene();
    other.takes[1].name = "Lift".into();
    let mut third = AppState::new(64, 64);
    third.apply(Action::OpenProject(ylp.clone()));
    load(&mut third, &fbx, &other);
    let note = stored::restore_from_project(&mut third);
    assert_eq!(
        note.as_deref(),
        Some("ポーズを戻しました（合わない項目 1 件）。")
    );
    let s3 = session(&third);
    assert_eq!(s3.takes.chosen(), Some((0, 0)));
    assert!(s3.preset_notes[0]
        .describe(yolu_app::lang::Lang::Ja)
        .contains("Raise (テイクがありません)"));
}
