//! モデルの今のポーズを .ylp に残す（根の `pose.json`。形式は上げない状態のエントリ）: 保存・戻す・合わない骨と BlendShape・休みの形・
//! Live Link など、ポーズのセッションが無いときの保存・読めないエントリ・復旧の書き置き・「変更あり」の印。画面を描かないので Wine でも回る。
//! ポーズの欄の画面は `gui_view3d/pose_ui.rs`、ポーズの操作は `gui_view3d/pose.rs`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yolu_app::lang::Lang;
use yolu_app::recovery::{DiskSpace, RecoveryAction, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::pose::{self, stored, PoseAction};
use yolu_core::glam::{EulerRot, Quat, Vec3};
use yolu_io::pose::{StoredBone, StoredPose, StoredShape};
use yolu_io::{Blob, Limits, Project};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-pose-saved-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 試しの人形を読み、プロジェクトのモデル（ファイル）として扱う（ポーズは、プロジェクトのモデルのものだけ保存される）。
fn figure(dir: &TempDir) -> AppState {
    let mut s = AppState::new(64, 64);
    s.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(s.view3d.pose.session.is_some(), "{}", s.message);
    // 読んだモデルはファイルのもの（試しの人形は、ふつうはファイルを持たない）
    s.np.model_file = Some(dir.file("figure.fbx"));
    s
}

fn bone(s: &AppState, name: &str) -> usize {
    s.view3d
        .pose
        .session
        .as_ref()
        .unwrap()
        .rig
        .bones()
        .iter()
        .position(|b| b.name == name)
        .unwrap()
}

/// 右上腕を回して動かして大きさを変え、頭を回し、メッシュ 0 の最初の BlendShape の重みを 70 にする（ポーズの取り消しの段は 1 つ）。
fn pose_it(s: &mut AppState) {
    let session = s.view3d.pose.session.as_ref().unwrap();
    let mut p = session.pose().clone();
    let arm = bone(s, "右上腕");
    p.locals[arm].rotation = Quat::from_euler(EulerRot::XYZ, 0.3, -0.5, 1.1);
    p.locals[arm].translation += Vec3::new(0.01, -0.02, 0.03);
    p.locals[arm].scale = Vec3::new(1.0, 1.25, 0.8);
    let head = bone(s, "頭");
    p.locals[head].rotation = Quat::from_rotation_y(0.4);
    p.blend_weights[0][0] = 70.0;
    pose::set_pose(&mut s.view3d, p).unwrap();
}

fn session_pose(s: &AppState) -> yolu_core::skin::Pose {
    s.view3d.pose.session.as_ref().unwrap().pose().clone()
}

fn close(a: &yolu_core::skin::Pose, b: &yolu_core::skin::Pose) -> bool {
    a.locals.len() == b.locals.len()
        && a.locals.iter().zip(&b.locals).all(|(x, y)| {
            (x.translation - y.translation).length() < 1.0e-5
                && x.rotation.dot(y.rotation).abs() > 0.999_999
                && (x.scale - y.scale).abs().max_element() < 1.0e-5
        })
        && a.blend_weights == b.blend_weights
}

fn save_as(s: &mut AppState, path: &Path) {
    s.apply(Action::SaveProjectAs(path.to_path_buf()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
}

fn on_disk(path: &Path) -> Project {
    Project::open(path, &Limits::unbounded()).unwrap()
}

/// ファイルを開く（モデルのファイルは無いので読めない。そのあと試しの人形をファイルのモデルとして入れて、ポーズを戻す）。
fn reopen_with_figure(dir: &TempDir, path: &Path) -> (AppState, Option<String>) {
    let mut s = AppState::new(64, 64);
    s.apply(Action::OpenProject(path.to_path_buf()));
    assert!(s.project.is_some(), "{}", s.message);
    s.apply(Action::Pose(PoseAction::LoadFigure));
    s.np.model_file = Some(dir.file("figure.fbx"));
    let note = stored::restore_from_project(&mut s);
    (s, note)
}

#[test]
fn headless_the_pose_of_the_project_model_is_saved_and_comes_back_on_the_same_model() {
    let dir = TempDir::new("roundtrip");
    let path = dir.file("a.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    let posed = session_pose(&s);
    save_as(&mut s, &path);
    let project = on_disk(&path);
    assert_eq!(project.info().format, 7, "ポーズは形式を上げない");
    let stored_pose = project.pose().unwrap().expect("pose.json がある");
    assert_eq!(
        stored_pose.bones.len(),
        2,
        "動かした骨だけ（休みの形からの差）"
    );
    assert_eq!(stored_pose.shapes.len(), 1);
    assert_eq!(stored_pose.shapes[0].weight, 70.0);
    assert!(stored_pose
        .bones
        .iter()
        .any(|b| b.path.last().unwrap() == "右上腕"));
    // 同じモデルを開くと戻る。取り消しの段にも「変更あり」にもならない
    let (mut again, note) = reopen_with_figure(&dir, &path);
    assert!(close(&session_pose(&again), &posed), "{note:?}");
    let session = again.view3d.pose.session.as_ref().unwrap();
    assert!(!session.can_undo() && !session.can_redo());
    assert_eq!(session.edits, 0);
    assert_eq!(note.as_deref(), Some("ポーズを戻しました。"));
    assert!(session.preset_notes.is_empty());
    pose::sync_modified(&mut again);
    assert!(!again.modified, "戻しただけでは変更ではない");
    // 戻したあとに変えれば変更あり・取り消しは戻した状態へ
    let mut p = session_pose(&again);
    p.locals[bone(&again, "左足")].rotation = Quat::from_rotation_x(0.5);
    pose::set_pose(&mut again.view3d, p).unwrap();
    pose::sync_modified(&mut again);
    assert!(again.modified);
    assert!(pose::undo(&mut again.view3d).unwrap());
    assert!(close(&session_pose(&again), &posed));
}

#[test]
fn headless_a_pose_change_marks_the_project_modified_but_the_demo_figure_without_a_file_does_not() {
    let dir = TempDir::new("modified");
    let mut s = figure(&dir);
    pose::sync_modified(&mut s);
    assert!(!s.modified, "モデルを入れただけでは変更ではない");
    pose_it(&mut s);
    pose::sync_modified(&mut s);
    assert!(s.modified, "ポーズを変えると保存が要る");
    s.modified = false;
    pose::sync_modified(&mut s);
    assert!(!s.modified, "同じポーズでは印を付け直さない");
    // 取り消しも変更
    assert!(pose::undo(&mut s.view3d).unwrap());
    pose::sync_modified(&mut s);
    assert!(s.modified);
    // ファイルのモデルが無い（試しの人形そのまま）ときは、残らないので数えない
    let mut demo = AppState::new(64, 64);
    demo.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(demo.np.model_file.is_none());
    pose_it(&mut demo);
    pose::sync_modified(&mut demo);
    assert!(!demo.modified);
}

#[test]
fn headless_a_rest_pose_removes_the_entry_and_a_model_without_a_file_does_not_write_one() {
    let dir = TempDir::new("rest");
    let path = dir.file("rest.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    save_as(&mut s, &path);
    assert!(on_disk(&path).pose().unwrap().is_some());
    // 休みの形へ戻して保存すると、エントリが消える
    pose::reset(&mut s.view3d).unwrap();
    assert!(!s.view3d.pose.session.as_ref().unwrap().is_posed());
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(on_disk(&path).pose().unwrap().is_none());
    assert!(!on_disk(&path)
        .original_archive()
        .entries()
        .contains_key("pose.json"));
    // ファイルのモデルが無いプロジェクトでは、ポーズを動かしても書かない（モデルの参照も無い）
    let mut demo = AppState::new(64, 64);
    demo.apply(Action::Pose(PoseAction::LoadFigure));
    pose_it(&mut demo);
    let other = dir.file("demo.ylp");
    save_as(&mut demo, &other);
    assert!(on_disk(&other).pose().unwrap().is_none());
}

#[test]
fn headless_a_model_that_does_not_fit_skips_with_reasons_and_leaves_the_pose_alone_when_nothing_fits(
) {
    let dir = TempDir::new("mismatch");
    let path = dir.file("m.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    save_as(&mut s, &path);
    // 骨が合わない・BlendShape が合わない項目を足したファイル（別のモデルのポーズ）
    let project = on_disk(&path);
    let mut pose = project.pose().unwrap().unwrap();
    pose.bones.push(StoredBone {
        path: vec!["腰".into(), "尻尾".into()],
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    });
    pose.shapes.push(StoredShape {
        mesh: "無いメッシュ".into(),
        name: "x".into(),
        weight: 10.0,
    });
    std::fs::write(
        &path,
        project.with_pose(Some(&pose)).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    let (again, note) = reopen_with_figure(&dir, &path);
    assert_eq!(
        note.as_deref(),
        Some("ポーズを戻しました（合わない項目 2 件）。"),
        "{note:?}"
    );
    let skipped = &again.view3d.pose.session.as_ref().unwrap().preset_notes;
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    let texts: Vec<String> = skipped.iter().map(|k| k.describe(Lang::Ja)).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("腰/尻尾") && t.contains("ボーンがありません")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("無いメッシュ/x") && t.contains("BlendShape がありません")),
        "{texts:?}"
    );
    let en: Vec<String> = skipped.iter().map(|k| k.describe(Lang::En)).collect();
    assert!(
        en.iter().any(|t| t.contains("Bone not found"))
            && en.iter().any(|t| t.contains("BlendShape not found")),
        "{en:?}"
    );
    assert!(
        close(&session_pose(&again), &session_pose(&s)),
        "合う項目は戻る"
    );
    // 1 つも合わなければ、ポーズは休みの形のまま
    let nothing = StoredPose {
        bones: vec![StoredBone {
            path: vec!["無い".into()],
            translation: [0.1; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }],
        shapes: Vec::new(),
    };
    let project = on_disk(&path);
    std::fs::write(
        &path,
        project
            .with_pose(Some(&nothing))
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let (none_fit, note) = reopen_with_figure(&dir, &path);
    assert!(!none_fit.view3d.pose.session.as_ref().unwrap().is_posed());
    assert_eq!(
        note.as_deref(),
        Some("ファイルのポーズに合うボーンがありません。")
    );
    // 英語の窓の文
    let mut en = AppState::new_in(64, 64, Lang::En);
    en.apply(Action::OpenProject(path.clone()));
    en.apply(Action::Pose(PoseAction::LoadFigure));
    en.np.model_file = Some(dir.file("figure.fbx"));
    assert_eq!(
        stored::restore_from_project(&mut en).as_deref(),
        Some("No bone fits the pose in the file.")
    );
}

#[test]
fn headless_saving_without_a_pose_session_keeps_the_pose_in_the_file() {
    // モデルを読んでいる最中・見つからない・Live Link のモデルが出ている間は、ポーズのセッションが無い。ファイルのポーズに触らない
    let dir = TempDir::new("keep");
    let path = dir.file("k.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    save_as(&mut s, &path);
    let before = on_disk(&path).original_archive().entries()["pose.json"]
        .bytes()
        .unwrap();
    let mut open = AppState::new(64, 64);
    open.apply(Action::OpenProject(path.clone()));
    // モデルのファイルは無いので、セッションは無いまま（参照だけが残る）
    assert!(open.view3d.pose.session.is_none());
    assert!(open.np.model_file.is_some(), "参照は残る");
    open.modified = true;
    open.apply(Action::SaveProject);
    assert!(open.message.starts_with("保存しました"), "{}", open.message);
    let after = on_disk(&path);
    assert_eq!(
        after.original_archive().entries()["pose.json"]
            .bytes()
            .unwrap(),
        before,
        "ポーズはそのまま"
    );
    assert!(after.view_model().unwrap().is_some(), "モデルの参照も残る");
    // 保存をもう一度しても同じ
    open.modified = true;
    open.apply(Action::SaveProject);
    assert_eq!(
        on_disk(&path).original_archive().entries()["pose.json"]
            .bytes()
            .unwrap(),
        before
    );
    // モデルを外した（試しの立方体などに替えた）プロジェクトは、ポーズも外す（どのモデルのポーズでもなくなる）
    open.apply(Action::LoadDemoModel);
    assert!(open.np.model_file.is_none());
    open.apply(Action::SaveProject);
    assert!(open.message.starts_with("保存しました"), "{}", open.message);
    assert!(on_disk(&path).pose().unwrap().is_none());
}

fn tamper_pose(path: &Path, bytes: &[u8]) {
    let project = Project::read(&std::fs::read(path).unwrap()).unwrap();
    let mut files = project.original_archive().entries().clone();
    files.insert("pose.json".into(), Blob::from(bytes.to_vec()));
    std::fs::write(
        path,
        Project::from_entries(files).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
}

#[test]
fn headless_an_unreadable_pose_is_told_kept_in_the_file_and_replaced_only_by_a_new_pose() {
    let dir = TempDir::new("unreadable");
    let path = dir.file("u.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    save_as(&mut s, &path);
    tamper_pose(&path, b"{ not json");
    for lang in Lang::ALL {
        let mut open = AppState::new_in(64, 64, lang);
        open.apply(Action::OpenProject(path.clone()));
        open.apply(Action::Pose(PoseAction::LoadFigure));
        open.np.model_file = Some(dir.file("figure.fbx"));
        let note = stored::restore_from_project(&mut open).unwrap();
        assert!(
            note.contains(lang.pick("ポーズを読めません", "Cannot read the pose")),
            "{note}"
        );
        assert!(!open.view3d.pose.session.as_ref().unwrap().is_posed());
    }
    // 休みの形のまま保存しても、読めなかったエントリは消さない
    let mut open = AppState::new(64, 64);
    open.apply(Action::OpenProject(path.clone()));
    open.apply(Action::Pose(PoseAction::LoadFigure));
    open.np.model_file = Some(dir.file("figure.fbx"));
    open.modified = true;
    open.apply(Action::SaveProject);
    assert!(open.message.starts_with("保存しました"), "{}", open.message);
    assert!(!open.message.contains("置き換え"), "{}", open.message);
    assert_eq!(
        on_disk(&path).original_archive().entries()["pose.json"]
            .bytes()
            .unwrap()
            .as_ref(),
        b"{ not json"
    );
    // 新しいポーズを付けて保存すると置き換わり、そのことを言う
    pose_it(&mut open);
    open.apply(Action::SaveProject);
    assert!(
        open.message.contains("読めなかったポーズ"),
        "{}",
        open.message
    );
    assert!(on_disk(&path).pose().unwrap().is_some());
}

// ───────── Live Link ─────────

fn link_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};
    Model {
        generation: 1,
        name: "Unity".into(),
        materials: vec![MaterialInfo {
            key: MaterialKey::Material {
                name: "Skin".into(),
                asset: None,
            },
            shader: "Standard".into(),
            textures: Vec::new(),
            routes: Vec::new(),
        }],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 1, 2],
            }],
        }],
    }
}

#[test]
fn headless_a_live_link_model_neither_restores_nor_removes_the_saved_pose_and_its_own_pose_is_not_saved(
) {
    let dir = TempDir::new("link");
    let path = dir.file("l.ylp");
    let mut s = figure(&dir);
    pose_it(&mut s);
    save_as(&mut s, &path);
    let before = on_disk(&path).original_archive().entries()["pose.json"]
        .bytes()
        .unwrap();
    // 開いて、プロジェクトのモデルのポーズを戻す
    let (mut open, note) = reopen_with_figure(&dir, &path);
    assert_eq!(note.as_deref(), Some("ポーズを戻しました。"));
    // Unity のモデルが来る: ポーズのセッションは終わる（Live Link のモデルは骨を持たない）。受けたポーズは頂点の位置
    let model = link_model();
    let (_, loaded) = open.receive_link_model(&model);
    loaded.unwrap();
    pose::poll(&mut open.view3d);
    open.sync_rig_model();
    assert!(
        open.view3d.pose.session.is_none(),
        "Live Link のモデルはポーズのセッションを持たない"
    );
    assert!(open.model.as_ref().is_some_and(|m| m.is_link()));
    let received = yolu_protocol::Pose {
        generation: 1,
        meshes: vec![yolu_protocol::MeshPose {
            mesh: 0,
            positions: vec![[0.0, 0.5, 0.0], [1.0, 0.5, 0.0], [0.0, 1.5, 0.0]],
            normals: vec![],
        }],
    };
    let _ = open.receive_link_pose(&received);
    // 保存: ファイルのポーズはそのまま（消えず、受けたポーズで上書きもされない）。モデルの参照も残る
    open.modified = true;
    open.apply(Action::SaveProject);
    assert!(open.message.starts_with("保存しました"), "{}", open.message);
    let after = on_disk(&path);
    assert_eq!(
        after.original_archive().entries()["pose.json"]
            .bytes()
            .unwrap(),
        before
    );
    assert!(after.view_model().unwrap().is_some());
    // もう一度開けば、保存済みのポーズが戻る
    let (again, note) = reopen_with_figure(&dir, &path);
    assert_eq!(note.as_deref(), Some("ポーズを戻しました。"));
    assert!(close(&session_pose(&again), &session_pose(&s)));
}

// ───────── 復旧 ─────────

fn plenty() -> SpaceProbe {
    Arc::new(|_| {
        Some(DiskSpace {
            total: 1000 << 30,
            available: 900 << 30,
        })
    })
}
fn session(root: &Path) -> AppState {
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    s.recovery
        .enable(
            root.to_path_buf(),
            RecoverySettings {
                interval_seconds: 15,
                strokes_between: 0,
                generations_to_keep: 3,
                directory: None,
                ..RecoverySettings::default()
            },
        )
        .unwrap();
    s
}
fn write_after(s: &mut AppState, from: Instant) {
    s.recovery_tick_at(from);
    s.recovery_tick_at(
        from + Duration::from_secs(s.recovery.settings().interval_seconds as u64 + 1),
    );
    s.recovery_wait();
}

#[test]
fn headless_a_pose_only_change_is_written_to_the_recovery_checkpoint_and_comes_back() {
    let dir = TempDir::new("recovery");
    let root = dir.file("recovery");
    let mut s = session(&root);
    s.apply(Action::Pose(PoseAction::LoadFigure));
    s.np.model_file = Some(dir.file("figure.fbx"));
    // 絵を描かずに、ポーズだけを変える（変更ありの印が付き、書き置きの鍵が変わる）
    s.modified = true;
    write_after(&mut s, Instant::now());
    let first = s.recovery.checkpoints();
    pose_it(&mut s);
    pose::sync_modified(&mut s);
    write_after(&mut s, Instant::now() + Duration::from_secs(100));
    assert_eq!(
        s.recovery.checkpoints(),
        first + 1,
        "ポーズだけの変更も書き置きになる"
    );
    assert!(s.recovery.is_idle());
    drop(s);
    let mut s2 = session(&root);
    s2.recovery_apply(RecoveryAction::Open);
    let recovered = s2
        .project
        .as_ref()
        .unwrap()
        .project()
        .pose()
        .unwrap()
        .expect("書き置きにポーズがある");
    assert_eq!(recovered.bones.len(), 2);
    assert_eq!(recovered.shapes[0].weight, 70.0);
}

// ───────── 保存の知らせ ─────────

#[test]
fn headless_the_pose_note_names_what_could_not_be_saved() {
    let lang = Lang::Ja;
    assert_eq!(stored::unsaved_note(lang, &[]), "");
    assert_eq!(
        stored::unsaved_note(lang, &["右腕".into(), "Face/Smile".into()]),
        " ポーズに保存できない項目: 右腕・Face/Smile。"
    );
    assert_eq!(
        stored::unsaved_note(Lang::En, &["a".into(), "b".into()]),
        " Not saved in the pose: a, b."
    );
}
