//! 名前を付けて残した選択範囲の保存と読み込み（.ylp の形式 8。使う文書だけ）・復旧の書き置き・読めない項目の扱い。
//! 画面を描かないので Wine でも回る。残す・名前を変える・消す・呼び戻すの操作と窓は `gui_canvas/selection_build.rs`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::Digest;
use yolu_app::engine::{CanvasResampling, SelectionCombine, SelectionMask};
use yolu_app::lang::Lang;
use yolu_app::recovery::{DiskSpace, RecoveryAction, RecoverySettings, SpaceProbe};
use yolu_app::selection::saved::SavedOp;
use yolu_app::selection::{SelAction, SelEdit};
use yolu_app::state::{Action, AppState};
use yolu_io::saved_selections::INDEX;
use yolu_io::{Blob, Limits, Project};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-saved-sel-{tag}-{}-{}",
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

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> SelEdit {
    SelEdit::Rect {
        x0,
        y0,
        x1,
        y1,
        mode: SelectionCombine::Replace,
    }
}
fn run(s: &mut AppState, e: SelEdit) {
    s.apply(Action::Sel(SelAction::Edit(e)));
}
fn save(s: &mut AppState, name: &str) {
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Save(name.into()))));
}
fn names(s: &AppState) -> Vec<String> {
    s.saved_selections()
        .iter()
        .map(|x| x.name.clone())
        .collect()
}
fn open(path: &Path) -> AppState {
    let mut s = AppState::new(64, 64);
    s.apply(Action::OpenProject(path.to_path_buf()));
    s
}
fn on_disk(path: &Path) -> Project {
    Project::open(path, &Limits::unbounded()).unwrap()
}
fn saved_ok(s: &AppState) {
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
}

/// 髪・服を残した 1 セットの .ylp。
fn project_with_two(dir: &TempDir) -> (PathBuf, AppState) {
    let path = dir.file("a.ylp");
    let mut s = AppState::new(64, 64);
    run(&mut s, rect(2, 2, 20, 20));
    save(&mut s, "髪");
    run(&mut s, rect(30, 10, 60, 50));
    save(&mut s, "服");
    run(&mut s, SelEdit::Clear);
    s.apply(Action::SaveProjectAs(path.clone()));
    saved_ok(&s);
    (path, s)
}

#[test]
fn headless_saved_selections_survive_save_and_reopen_in_order_and_use_format_8_only_then() {
    let dir = TempDir::new("roundtrip");
    let (path, s) = project_with_two(&dir);
    let project = on_disk(&path);
    assert_eq!(project.info().format, 8, "使う文書だけ形式 8");
    assert_eq!(
        project.original_archive().manifest_version(),
        3,
        "外側の版は変わらない"
    );
    let id = project.sets()[0].id.clone();
    let stored = project.saved_selections(&id).unwrap();
    assert!(stored.skipped.is_empty());
    assert_eq!(
        stored
            .items
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>(),
        ["髪", "服"]
    );
    // 開き直すと、名前・並び・中身が同じで、戻せる段は無く、保存済み
    let again = open(&path);
    assert_eq!(names(&again), ["髪", "服"], "{}", again.message);
    for (a, b) in again.saved_selections().iter().zip(s.saved_selections()) {
        assert_eq!(a.mask, b.mask, "{}", a.name);
    }
    assert!(!again.doc.can_undo(), "開いた直後は戻せる段が無い");
    assert!(!again.modified);
    assert_eq!(again.project.as_ref().unwrap().format(), 8);
    // 呼び戻しもできる（大きさが合う）
    let mut again = again;
    again.apply(Action::Sel(SelAction::Edit(SelEdit::Recall {
        index: 1,
        mode: SelectionCombine::Replace,
    })));
    assert_eq!(
        again.doc.selection(),
        Some(&SelectionMask::rectangle(&again.doc, 30, 10, 60, 50))
    );
}

#[test]
fn headless_a_name_change_and_a_delete_are_saved_and_the_last_delete_goes_back_to_format_7() {
    let dir = TempDir::new("edits");
    let (path, _) = project_with_two(&dir);
    let mut s = open(&path);
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
        index: 0,
        name: "前髪".into(),
    })));
    assert!(s.modified, "名前の変更は保存が要る変更");
    s.apply(Action::SaveProject);
    saved_ok(&s);
    let s = open(&path);
    assert_eq!(names(&s), ["前髪", "服"]);
    // 1 つ消す: 残りは残る。全部消す: エントリが無くなり、形式は 7 に戻る
    let mut s = s;
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert_eq!(on_disk(&path).info().format, 8);
    assert_eq!(names(&open(&path)), ["服"]);
    let mut s = open(&path);
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.apply(Action::SaveProject);
    saved_ok(&s);
    let project = on_disk(&path);
    assert_eq!(project.info().format, 7);
    let id = project.sets()[0].id.clone();
    assert!(!project
        .original_archive()
        .entries()
        .keys()
        .any(|n| n.starts_with(&format!("sets/{id}/selection-")) || n.ends_with(INDEX)));
    assert!(open(&path).saved_selections().is_empty());
}

#[test]
fn headless_a_document_that_does_not_use_them_stays_format_7_and_an_old_format_is_not_given_new_entries(
) {
    let dir = TempDir::new("plain");
    let path = dir.file("plain.ylp");
    let mut s = AppState::new(64, 64);
    run(&mut s, rect(2, 2, 20, 20));
    s.apply(Action::SaveProjectAs(path.clone()));
    saved_ok(&s);
    assert_eq!(on_disk(&path).info().format, 7);
    assert!(!on_disk(&path)
        .original_archive()
        .entries()
        .keys()
        .any(|n| n.contains("selections.json") || n.contains("/selection-") || n == "pose.json"));
    // Unity 版が書いた古い形式を開いて保存し直しても、新しいエントリを足さず、形式は 7（8 にならない）
    for n in [3, 6] {
        let old = dir.file(&format!("format{n}.ylp"));
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../yolu-io/tests/fixtures/format{n}.ylp")),
            &old,
        )
        .unwrap();
        let mut s = open(&old);
        assert!(s.saved_selections().is_empty(), "形式 {n}: {}", s.message);
        // 何か変えて保存（文書を書き直す）
        run(&mut s, SelEdit::All);
        s.modified = true;
        s.apply(Action::SaveProject);
        saved_ok(&s);
        let project = on_disk(&old);
        assert_eq!(project.info().format, 7, "形式 {n}");
        assert!(
            !project
                .original_archive()
                .entries()
                .keys()
                .any(|e| e.contains("selections.json")
                    || e.contains("/selection-")
                    || e == "pose.json"),
            "形式 {n}"
        );
    }
}

#[test]
fn headless_each_set_keeps_its_own_saved_selections_across_save_and_reopen() {
    let dir = TempDir::new("sets");
    let path = dir.file("two.ylp");
    let mut s = AppState::new(64, 64);
    run(&mut s, rect(0, 0, 10, 10));
    save(&mut s, "A1");
    s.add_texture_set().unwrap();
    run(&mut s, rect(20, 20, 40, 40));
    save(&mut s, "B1");
    save(&mut s, "B2");
    s.apply(Action::SaveProjectAs(path.clone()));
    saved_ok(&s);
    let mut again = open(&path);
    assert_eq!(again.sets.len(), 2, "{}", again.message);
    again.switch_set(0).unwrap();
    assert_eq!(names(&again), ["A1"]);
    again.switch_set(1).unwrap();
    assert_eq!(names(&again), ["B1", "B2"]);
    // 片方のセットだけ変えて保存しても、もう片方は変わらない
    again.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    again.apply(Action::SaveProject);
    saved_ok(&again);
    let mut third = open(&path);
    third.switch_set(0).unwrap();
    assert_eq!(names(&third), ["A1"]);
    third.switch_set(1).unwrap();
    assert_eq!(names(&third), ["B2"]);
}

#[test]
fn headless_saving_after_the_document_was_resized_writes_the_resampled_saved_selections() {
    let dir = TempDir::new("resize");
    let (path, _) = project_with_two(&dir);
    let mut s = open(&path);
    // 今の選択範囲も持たせる（selection.bin も大きさが変わる）
    run(&mut s, rect(1, 1, 8, 8));
    s.apply(Action::SaveProject);
    saved_ok(&s);
    let mut s = open(&path);
    s.doc
        .resize_image(32, 32, CanvasResampling::Nearest)
        .unwrap();
    s.modified = true;
    s.apply(Action::SaveProject);
    saved_ok(&s);
    // 古い大きさの残した選択範囲は壊れたエントリではないので、「読めなかった項目を置き換えた」とは言わない
    assert!(!s.message.contains("置き換え"), "{}", s.message);
    let again = open(&path);
    assert!(
        again.doc.selection().is_some(),
        "今の選択範囲も新しい大きさで残る: {}",
        again.message
    );
    assert_eq!(
        (again.doc.width(), again.doc.height()),
        (32, 32),
        "{}",
        again.message
    );
    assert_eq!(names(&again), ["髪", "服"], "{}", again.message);
    assert!(again
        .saved_selections()
        .iter()
        .all(|s| s.mask.width() == 32));
    assert!(
        again.message.matches("読めない").count() == 0,
        "{}",
        again.message
    );
}

/// 保存した .ylp にある、残した選択範囲のエントリの名前（索引と中身）。
fn saved_entries(path: &Path) -> Vec<String> {
    let project = on_disk(path);
    let id = project.sets()[0].id.clone();
    project
        .original_archive()
        .entries()
        .keys()
        .filter(|n| n.starts_with(&format!("sets/{id}/selection-")) || n.ends_with(INDEX))
        .cloned()
        .collect()
}

#[test]
fn headless_deleting_every_saved_selection_then_resizing_leaves_no_stale_entry_and_goes_back_to_format_7(
) {
    let dir = TempDir::new("resize-empty");
    let (path, _) = project_with_two(&dir);
    assert!(!saved_entries(&path).is_empty());
    let mut s = open(&path);
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    assert!(s.saved_selections().is_empty());
    s.doc
        .resize_image(32, 32, CanvasResampling::Nearest)
        .unwrap();
    s.modified = true;
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert_eq!(
        saved_entries(&path),
        Vec::<String>::new(),
        "古い大きさのエントリが残らない"
    );
    assert_eq!(
        on_disk(&path).info().format,
        7,
        "使わなくなれば形式 7 に戻る"
    );
    let again = open(&path);
    assert_eq!(
        (again.doc.width(), again.doc.height()),
        (32, 32),
        "{}",
        again.message
    );
    assert!(again.saved_selections().is_empty());
    assert!(
        !again.message.contains("読めない"),
        "開き直しに、読めない覚えた選択範囲が出ない: {}",
        again.message
    );
}

#[test]
fn headless_a_shrink_that_removes_every_saved_selection_drops_the_stale_entries_too() {
    let dir = TempDir::new("resize-shrunk");
    let path = dir.file("tiny.ylp");
    let mut s = AppState::new(64, 64);
    // 1 画素幅の線の選択は、近傍で縮めると消える
    run(&mut s, rect(10, 10, 11, 60));
    save(&mut s, "線");
    s.apply(Action::SaveProjectAs(path.clone()));
    saved_ok(&s);
    let mut s = open(&path);
    assert_eq!(names(&s), ["線"], "{}", s.message);
    s.doc.resize_image(8, 8, CanvasResampling::Nearest).unwrap();
    s.modified = true;
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert!(
        s.saved_selections().is_empty(),
        "縮小で 1 件とも外れる: {}",
        s.message
    );
    assert_eq!(saved_entries(&path), Vec::<String>::new());
    assert_eq!(on_disk(&path).info().format, 7);
    let again = open(&path);
    assert!(again.saved_selections().is_empty(), "{}", again.message);
    assert!(!again.message.contains("読めない"), "{}", again.message);
}

#[test]
fn headless_a_saved_selection_the_document_does_not_fit_stays_in_the_file_when_nothing_was_resized()
{
    let dir = TempDir::new("wrong-size-kept");
    let (path, _) = project_with_two(&dir);
    // ファイルの文書の大きさを、残した選択範囲のものと食い違うように変える（壊れた・手で直したファイル）。文書は変えないので、
    // 開いたときに理由つきで飛ばし、保存してもエントリはファイルに残る
    tamper(&path, |files, id| {
        let name = format!("sets/{id}/{INDEX}");
        let index: serde_json::Value =
            serde_json::from_slice(&files[&name].bytes().unwrap()).unwrap();
        let content = index["selections"][0]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        let leaf = format!("sets/{id}/selection-{content}.bin");
        // 頭の幅（8..12。0..4 は印、4..8 は版）だけ書き換えると中身の印が合わなくなるので、印も付け直す
        let mut bytes = files[&leaf].bytes().unwrap().to_vec();
        bytes[8..12].copy_from_slice(&128i32.to_le_bytes());
        let new_content = format!("{:x}", sha2::Sha256::digest(&bytes))[..32].to_owned();
        files.remove(&leaf);
        files.insert(
            format!("sets/{id}/selection-{new_content}.bin"),
            Blob::from(bytes),
        );
        let mut index = index;
        index["selections"][0]["content"] = serde_json::Value::String(new_content);
        files.insert(name, Blob::from(serde_json::to_vec(&index).unwrap()));
    });
    let mut s = open(&path);
    assert_eq!(names(&s), ["服"], "{}", s.message);
    assert!(
        s.message.contains("大きさが違います") && s.message.contains("ファイルには残っています"),
        "{}",
        s.message
    );
    let before = saved_entries(&path);
    s.modified = true;
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert_eq!(
        saved_entries(&path),
        before,
        "文書を変えていないので、飛ばした項目のエントリも残る"
    );
}

/// 保存した .ylp の中の、残した選択範囲のエントリを書き換えて書き戻す。
fn tamper(path: &Path, edit: impl FnOnce(&mut yolu_io::Files, &str)) {
    let project = Project::read(&std::fs::read(path).unwrap()).unwrap();
    let id = project.sets()[0].id.clone();
    let mut files = project.original_archive().entries().clone();
    edit(&mut files, &id);
    std::fs::write(
        path,
        Project::from_entries(files).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
}

#[test]
fn headless_an_unreadable_item_is_told_kept_in_the_file_and_replaced_only_when_the_list_changes() {
    let dir = TempDir::new("broken");
    let (path, _) = project_with_two(&dir);
    // 「服」の中身を消す（索引には残る）
    tamper(&path, |files, id| {
        let index: serde_json::Value =
            serde_json::from_slice(&files[&format!("sets/{id}/{INDEX}")].bytes().unwrap()).unwrap();
        let content = index["selections"][1]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        files.remove(&format!("sets/{id}/selection-{content}.bin"));
    });
    let mut s = open(&path);
    assert_eq!(names(&s), ["髪"], "読める項目は読む");
    assert!(
        s.message.contains("服")
            && s.message.contains("中身がありません")
            && s.message.contains("ファイルには残っています"),
        "{}",
        s.message
    );
    // 何も変えずに保存しても、読めなかった項目はファイルに残る
    s.modified = true;
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert!(!s.message.contains("置き換え"), "{}", s.message);
    let project = on_disk(&path);
    let id = project.sets()[0].id.clone();
    assert_eq!(
        project.saved_selections(&id).unwrap().skipped.len(),
        1,
        "読めなかった項目はそのまま"
    );
    // 残した選択範囲を変えて保存すると置き換わり、そのことを言う
    let mut s = open(&path);
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
        index: 0,
        name: "前髪".into(),
    })));
    s.apply(Action::SaveProject);
    saved_ok(&s);
    assert!(s.message.contains("置き換えました"), "{}", s.message);
    let project = on_disk(&path);
    assert!(project.saved_selections(&id).unwrap().skipped.is_empty());
    assert_eq!(names(&open(&path)), ["前髪"]);
}

#[test]
fn headless_a_broken_index_opens_the_document_with_a_reason_in_both_languages() {
    let dir = TempDir::new("index");
    let (path, _) = project_with_two(&dir);
    tamper(&path, |files, id| {
        files.insert(
            format!("sets/{id}/{INDEX}"),
            Blob::from(b"{ broken".to_vec()),
        );
    });
    for lang in Lang::ALL {
        let mut s = AppState::new_in(64, 64, lang);
        s.apply(Action::OpenProject(path.clone()));
        assert!(s.saved_selections().is_empty(), "{}", s.message);
        assert_eq!(s.doc.width(), 64, "文書は開く: {}", s.message);
        let want = lang.pick("索引を読めません", "the index cannot be read");
        assert!(s.message.contains(want), "{}", s.message);
        // 英語の窓に、日本語の理由は出ない（セットの名前は文書が持つ名前のまま）
        assert_eq!(
            s.message.contains("索引"),
            lang == Lang::Ja,
            "{}",
            s.message
        );
    }
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
fn headless_saved_selections_come_back_after_a_crash_and_the_saved_file_has_them() {
    let dir = TempDir::new("recovery");
    let root = dir.file("recovery");
    let mut s = session(&root);
    run(&mut s, rect(2, 2, 20, 20));
    save(&mut s, "髪");
    run(&mut s, rect(30, 10, 60, 50));
    save(&mut s, "服");
    assert!(s.modified);
    write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1);
    // 名前の変更だけでも書き置きになる（絵も選択範囲も変えていない）
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
        index: 0,
        name: "前髪".into(),
    })));
    write_after(&mut s, Instant::now() + Duration::from_secs(100));
    assert_eq!(
        s.recovery.checkpoints(),
        2,
        "残した選択範囲の変更も書き置きに入る"
    );
    assert!(s.recovery.is_idle());
    drop(s);

    let mut s2 = session(&root);
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(names(&s2), ["前髪", "服"], "{}", s2.message);
    assert!(!s2.doc.can_undo(), "復旧は正本を返す。途中の履歴は返さない");
    // 復旧した文書は、別の場所へ保存した .ylp にも残した選択範囲を書く（形式 8）
    let saved = dir.file("recovered.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    saved_ok(&s2);
    assert_eq!(on_disk(&saved).info().format, 8);
    assert_eq!(names(&open(&saved)), ["前髪", "服"]);
}

#[test]
fn headless_a_recovery_of_a_resized_document_with_no_saved_selection_left_has_no_stale_entry() {
    let dir = TempDir::new("recovery-resize");
    let (path, _) = project_with_two(&dir);
    let root = dir.file("recovery");
    let mut s = session(&root);
    s.apply(Action::OpenProject(path.clone()));
    assert_eq!(names(&s), ["髪", "服"], "{}", s.message);
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.doc
        .resize_image(32, 32, CanvasResampling::Nearest)
        .unwrap();
    s.modified = true;
    write_after(&mut s, Instant::now());
    assert_eq!(s.recovery.checkpoints(), 1);
    drop(s);

    let mut s2 = session(&root);
    s2.recovery_apply(RecoveryAction::Open);
    assert_eq!(
        (s2.doc.width(), s2.doc.height()),
        (32, 32),
        "{}",
        s2.message
    );
    assert!(s2.saved_selections().is_empty(), "{}", s2.message);
    assert!(
        !s2.message.contains("読めない"),
        "復旧した文書に、古い大きさの項目が出ない: {}",
        s2.message
    );
    // 復旧した文書を別の場所へ保存しても、残した選択範囲のエントリは無く、形式 7
    let saved = dir.file("recovered.ylp");
    s2.apply(Action::SaveProjectAs(saved.clone()));
    saved_ok(&s2);
    assert_eq!(saved_entries(&saved), Vec::<String>::new());
    assert_eq!(on_disk(&saved).info().format, 7);
}
