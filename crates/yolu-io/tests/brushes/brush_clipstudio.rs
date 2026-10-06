//! CLIP STUDIO のサブツールのフォルダを探して一覧にする部分（`brushes::clipstudio`）: フォルダの見つけ方（試験用の一時フォルダの構成で）・
//! 見つからない理由・上限・読むだけであること（ファイルの更新時刻・中身・フォルダの項目が変わらない）・書き込み中のファイル（SQLite の鍵・
//! 書き込みの記録 WAL）・名前と筆先の見本。試験の `.sut` は試験の中で組む（実物は持ち込まない）。
#![allow(clippy::cloned_ref_to_slice_refs)]

use crate::brush_files;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use brush_files::sut::*;
use yolu_io::brushes::clipstudio::{
    find, peek, preview_of, read_stable, scan, Limits, Missing, Places, PREVIEW_SIDE,
};
use yolu_io::brushes::{import, BrushImportError};

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/clipstudio-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn simple_sut(name: &str) -> Vec<u8> {
    SutBuilder::new()
        .brush(name, 1, &[("BrushSize", real(30.0))])
        .build()
}

fn put(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// フォルダの下の全部の項目（相対パス・種類・大きさ・更新時刻）。「読んだだけ」を確かめるために、読む前後で比べる。
/// 値は項目のパスから読み直す。Windows の `DirEntry::metadata` はフォルダの一覧に残った値で、ほかの手が開いて書いている
/// ファイルの大きさや、中の項目が増えたフォルダの更新時刻が遅れて変わるため、読む前後の比べが「読んだだけ」と無関係に揺れる。
fn listing(dir: &Path) -> Vec<(String, bool, u64, Option<SystemTime>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, bool, u64, Option<SystemTime>)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let meta = std::fs::symlink_metadata(entry.path()).unwrap();
            let rel = entry
                .path()
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            out.push((rel, meta.is_dir(), meta.len(), meta.modified().ok()));
            if meta.is_dir() {
                walk(base, &entry.path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = listing(dir)
        .into_iter()
        .filter(|(_, is_dir, ..)| !is_dir)
        .map(|(rel, ..)| {
            let bytes = std::fs::read(dir.join(&rel)).unwrap();
            (rel, bytes)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn the_default_celsys_folders_follow_the_published_locations() {
    let places = Places {
        appdata: Some(PathBuf::from("/u/AppData/Roaming")),
        profile: Some(PathBuf::from("/u")),
        onedrive: Some(PathBuf::from("/u/OneDrive")),
    };
    assert_eq!(
        places.celsys_folders(),
        [
            PathBuf::from("/u/AppData/Roaming/CELSYSUserData/CELSYS"),
            PathBuf::from("/u/Documents/CELSYS"),
            PathBuf::from("/u/OneDrive/Documents/CELSYS"),
        ],
        "新しい版の場所（V1.10.13 以降）から、古い版の書類の下、OneDrive の書類の下の順"
    );
    assert_eq!(Places::default().celsys_folders(), Vec::<PathBuf>::new());
    // 同じ場所は 1 度だけ
    let same = Places {
        appdata: None,
        profile: Some(PathBuf::from("/u")),
        onedrive: Some(PathBuf::from("/u")),
    };
    assert_eq!(
        same.celsys_folders(),
        [PathBuf::from("/u/Documents/CELSYS")]
    );
}

#[test]
fn sut_files_are_found_under_the_celsys_folders_whatever_the_subfolder_names() {
    let home = temp_dir("find");
    let roaming = home.join("AppData/Roaming");
    let new_root = roaming.join("CELSYSUserData/CELSYS");
    let old_root = home.join("Documents/CELSYS");
    put(
        &new_root.join("CLIPStudioModule/SubTool/Pen/a.sut"),
        &simple_sut("A"),
    );
    put(
        &new_root.join("CLIPStudioModule/SubTool/Brush/Deep/er/B.SUT"),
        &simple_sut("B"),
    );
    put(
        &new_root.join("CLIPStudioPaintVer3_0_0/SubTool/c.sut"),
        &simple_sut("C"),
    );
    put(
        &old_root.join("CLIPStudioModule/SubTool/Pen/d.sut"),
        &simple_sut("D"),
    );
    // .sut でないもの・拡張子だけ似たものは数えない
    put(
        &new_root.join("CLIPStudioModule/SubTool/Pen/readme.txt"),
        b"x",
    );
    put(&new_root.join("CLIPStudioModule/SubTool/Pen/e.sutx"), b"x");
    put(&new_root.join("CLIPStudioModule/SubTool/Pen/sut"), b"x");
    let places = Places {
        appdata: Some(roaming),
        profile: Some(home.clone()),
        onedrive: None,
    };
    let found = find(&places);
    assert_eq!(found.missing, None);
    assert!(!found.truncated);
    assert_eq!(found.searched, [new_root.clone(), old_root.clone()]);
    let names: Vec<String> = found
        .files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    // パスの小文字の順（新しい版の設定のフォルダが先）
    assert_eq!(names, ["B.SUT", "a.sut", "c.sut", "d.sut"]);
    assert_eq!(found.files.len(), 4);
    assert!(found.files.iter().all(|f| f.size > 0));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_missing_empty_or_unreadable_folder_gives_a_reason_and_no_files() {
    let home = temp_dir("missing");
    let none = Places {
        appdata: Some(home.join("AppData/Roaming")),
        profile: Some(home.clone()),
        onedrive: None,
    };
    let result = find(&none);
    assert!(result.files.is_empty() && result.searched.is_empty());
    assert_eq!(result.missing, Some(Missing::NoFolder));
    assert_eq!(find(&Places::default()).missing, Some(Missing::NoFolder));
    // フォルダはあるが .sut が無い
    let root = home.join("AppData/Roaming/CELSYSUserData/CELSYS");
    put(&root.join("CLIPStudioModule/readme.txt"), b"x");
    let result = find(&none);
    assert_eq!(result.missing, Some(Missing::NoFiles));
    assert_eq!(result.searched, [root.clone()]);
    // 開けない（権限）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = home.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // root の権限で動いていて開けてしまう環境では確かめない
        if std::fs::read_dir(&locked).is_err() {
            let result = scan(&[locked.clone()], Limits::default());
            assert_eq!(result.missing, Some(Missing::Unreadable));
            assert!(result.files.is_empty());
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // 手で選んだフォルダが無い
    let picked = scan(&[home.join("nowhere")], Limits::default());
    assert_eq!(picked.missing, Some(Missing::NoFolder));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn the_search_stops_at_its_limits_and_says_so() {
    let dir = temp_dir("limits");
    for i in 0..6 {
        put(&dir.join(format!("f{i}.sut")), &simple_sut("X"));
    }
    // 集める数の上限
    let capped = scan(
        &[dir.clone()],
        Limits {
            files: 4,
            ..Limits::default()
        },
    );
    assert_eq!(capped.files.len(), 4);
    assert!(capped.truncated);
    // 見る項目の数の上限
    let few = scan(
        &[dir.clone()],
        Limits {
            entries: 3,
            ..Limits::default()
        },
    );
    assert!(few.files.len() <= 3 && few.truncated);
    // 深さの上限: 深すぎる所の .sut は集めず、入らなかったことを知らせる
    put(&dir.join("a/b/c/deep.sut"), &simple_sut("Deep"));
    let shallow = scan(
        &[dir.clone()],
        Limits {
            depth: 1,
            ..Limits::default()
        },
    );
    assert_eq!(shallow.files.len(), 6);
    assert!(shallow.truncated);
    let all = scan(&[dir.clone()], Limits::default());
    assert_eq!(all.files.len(), 7);
    assert!(!all.truncated);
    // 範囲の中に収まっていれば truncated は立たない（ちょうど上限の数でも、次が無ければ）
    let exact = scan(
        &[dir.clone()],
        Limits {
            files: 7,
            ..Limits::default()
        },
    );
    assert_eq!(exact.files.len(), 7);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_not_followed_so_a_loop_cannot_hold_the_search() {
    let dir = temp_dir("links");
    put(&dir.join("real/a.sut"), &simple_sut("A"));
    std::os::unix::fs::symlink(&dir, dir.join("real/loop")).unwrap();
    std::os::unix::fs::symlink(dir.join("real/a.sut"), dir.join("alias.sut")).unwrap();
    let found = scan(&[dir.clone()], Limits::default());
    assert_eq!(
        found.files.len(),
        1,
        "実体だけ。リンクの別名・輪はたどらない"
    );
    assert!(!found.truncated);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn looking_listing_peeking_and_importing_change_nothing_in_the_folder() {
    let root = temp_dir("readonly");
    let sub = root.join("CLIPStudioModule/SubTool/Pen");
    put(&sub.join("a.sut"), &simple_sut("Alpha"));
    put(&sub.join("b.sut"), &simple_sut("Beta"));
    // ファイルを読み取り専用にしても読める（読むのに書き込みの権限を使わない）
    for name in ["a.sut", "b.sut"] {
        let mut perm = std::fs::metadata(sub.join(name)).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(sub.join(name), perm).unwrap();
    }
    let before = (listing(&root), contents(&root));
    let found = scan(&[root.clone()], Limits::default());
    assert_eq!(found.files.len(), 2);
    for f in &found.files {
        let p = peek(&f.path).unwrap();
        assert_eq!(p.brushes, 1);
        assert!(["Alpha", "Beta"].contains(&p.names[0].as_str()));
        let set = import(&f.path).unwrap();
        assert_eq!(set.brushes.len(), 1);
        assert_eq!(read_stable(&f.path, 1 << 30).unwrap().len() as u64, f.size);
    }
    let after = (listing(&root), contents(&root));
    assert_eq!(
        before, after,
        "更新時刻・大きさ・中身・項目の数が変わらない（作業用のファイルも作らない）"
    );
    for name in ["a.sut", "b.sut"] {
        let mut perm = std::fs::metadata(sub.join(name)).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(sub.join(name), perm).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

fn is_busy<T>(r: Result<T, BrushImportError>) -> bool {
    matches!(r, Err(BrushImportError::Io(ref e)) if e.kind() == std::io::ErrorKind::ResourceBusy)
}

#[test]
fn a_file_that_another_program_holds_locked_is_still_read_and_left_alone() {
    let dir = temp_dir("locked-db");
    let path = dir.join("tool.sut");
    put(&path, &simple_sut("Held"));
    // CLIP STUDIO のように、同じファイルを開いて排他の書き込みの途中でいる（ジャーナルのファイルもできる）
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute_batch("BEGIN EXCLUSIVE; UPDATE Node SET NodeName = 'Changing';")
        .unwrap();
    let before = listing(&dir);
    let journal = std::fs::read(dir.join("tool.sut-journal")).unwrap();
    // 退避のファイルの先頭は、書き手が中身を同期して初めて有効になる（それまでは 0）。本体のページを上書きするのはそれより後
    assert_eq!(journal[0], 0, "まだ同期していない退避のファイル");
    let main_bytes = std::fs::read(&path).unwrap();
    let p = peek(&path).expect("書き込みの途中でも、コミット済みの中身が読める");
    assert_eq!(p.names, ["Held"], "コミットされていない変更は見えない");
    assert_eq!(import(&path).unwrap().brushes.len(), 1);
    assert_eq!(listing(&dir), before, "こちらは何も足さず・変えない");
    assert_eq!(std::fs::read(&path).unwrap(), main_bytes);
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

/// 書き手がキャッシュを溢れさせて、まだ確定していないページを本体へ書き出した状態（元のページは `-journal` に退避される）を作る。
/// 返す接続は、コミットも取り消しもしないで開いたまま。
fn spilled_uncommitted_writer(path: &Path) -> (rusqlite::Connection, Vec<u8>) {
    {
        let setup = rusqlite::Connection::open(path).unwrap();
        setup
            .execute_batch(
                "PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY;
                 CREATE TABLE Filler (x BLOB);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 300)
                 INSERT INTO Filler SELECT zeroblob(2000) FROM n;",
            )
            .unwrap();
    }
    let committed = std::fs::read(path).unwrap();
    let writer = rusqlite::Connection::open(path).unwrap();
    writer
        .execute_batch(
            "PRAGMA cache_size = 5; BEGIN; UPDATE Filler SET x = randomblob(2000);
             UPDATE Node SET NodeName = 'Uncommitted';",
        )
        .unwrap();
    let main = std::fs::read(path).unwrap();
    let mut journal = path.as_os_str().to_owned();
    journal.push("-journal");
    let head = std::fs::read(&journal).unwrap();
    assert!(
        head.first().is_some_and(|b| *b != 0),
        "同期した、退避したページを持つジャーナルがある"
    );
    assert_ne!(
        main, committed,
        "確定前のページが本体に書き出された（この試験の前提）"
    );
    (writer, committed)
}

#[test]
fn a_file_with_pages_written_before_the_commit_is_refused_until_the_writer_finishes() {
    let dir = temp_dir("spilled");
    let path = dir.join("tool.sut");
    put(&path, &simple_sut("Held"));
    let (writer, committed) = spilled_uncommitted_writer(&path);
    let before = listing(&dir);
    // 本体の大きさも更新時刻も読んでいる間は変わらない。それでも確定前の中身を渡さない
    assert!(is_busy(read_stable(&path, 1 << 30)));
    assert!(is_busy(peek(&path)));
    assert!(is_busy(import(&path)));
    assert_eq!(listing(&dir), before, "こちらは何も足さず・変えない");
    // 取り消されて本体が元に戻り、ジャーナルが消えれば読める
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    assert_eq!(read_stable(&path, 1 << 30).unwrap(), committed);
    assert_eq!(peek(&path).unwrap().names, ["Held"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_hot_journal_left_by_a_crash_is_refused_and_a_finished_one_is_not() {
    let dir = temp_dir("journal-left");
    let path = dir.join("tool.sut");
    put(&path, &simple_sut("Held"));
    let mut journal = path.as_os_str().to_owned();
    journal.push("-journal");
    let journal = PathBuf::from(journal);
    // 書き込みの途中で落ちて残った（先頭が 0 でない）ジャーナル: 本体に確定前のページがありうるので断る
    put(
        &journal,
        &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7, 1, 2, 3],
    );
    assert!(is_busy(read_stable(&path, 1 << 30)));
    // 空・先頭を 0 にして無効にした（TRUNCATE・PERSIST のモードで終えた）ジャーナルは、確定済み
    put(&journal, b"");
    assert!(read_stable(&path, 1 << 30).is_ok());
    put(&journal, &[0u8; 512]);
    assert!(read_stable(&path, 1 << 30).is_ok());
    // ジャーナルが無ければ確定済み
    std::fs::remove_file(&journal).unwrap();
    assert!(read_stable(&path, 1 << 30).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_file_in_write_ahead_log_mode_is_read_as_its_last_checkpointed_state() {
    let dir = temp_dir("wal");
    let path = dir.join("tool.sut");
    put(&path, &simple_sut("Checkpointed"));
    let writer = rusqlite::Connection::open(&path).unwrap();
    // 書き込みの記録の形式へ（ファイルの先頭の印が変わる）。自動の書き戻しは止め、新しい変更は WAL にだけ残す
    writer
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
        .unwrap();
    writer
        .execute_batch("UPDATE Node SET NodeName = 'InTheLog';")
        .unwrap();
    let head = std::fs::read(&path).unwrap();
    assert_eq!((head[18], head[19]), (2, 2), "WAL の印");
    let before = listing(&dir);
    assert!(
        before.iter().any(|(rel, ..)| rel.ends_with("-wal")),
        "WAL がある: {before:?}"
    );
    let main_before = std::fs::read(&path).unwrap();
    let p = peek(&path).expect("WAL の形式の印があっても、本体のファイルをふつうの形式として読む");
    assert_eq!(
        p.names,
        ["Checkpointed"],
        "WAL に残った最新の変更は読まない"
    );
    assert_eq!(
        listing(&dir),
        before,
        "WAL・共有メモリを作り直さず、変えない"
    );
    assert_eq!(std::fs::read(&path).unwrap(), main_before);
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_file_that_is_not_a_database_or_too_large_is_refused_with_a_reason() {
    let dir = temp_dir("refuse");
    put(&dir.join("junk.sut"), b"not a database at all");
    put(&dir.join("empty.sut"), b"");
    for name in ["junk.sut", "empty.sut"] {
        assert!(
            matches!(peek(&dir.join(name)), Err(BrushImportError::Fault(_))),
            "{name}"
        );
    }
    assert!(matches!(
        read_stable(&dir.join("junk.sut"), 4),
        Err(BrushImportError::FileTooLarge { limit: 4 })
    ));
    assert!(matches!(
        peek(&dir.join("absent.sut")),
        Err(BrushImportError::Io(_))
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_preview_shows_the_tip_image_fitted_to_the_frame_and_a_round_tip_as_a_disc() {
    // 画像の筆先: 2×2（暗い画素が塗る）。行は下から。見本は枠いっぱいに、行は上から
    let file = SutBuilder::new()
        .material(
            Some("tip_a"),
            material_with_thumbnail(&png_gray(2, 2, &[0, 255, 255, 0])),
        )
        .brush(
            "Stamp",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["C:\\mats\\tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
            ],
        )
        .build();
    let dir = temp_dir("preview");
    put(&dir.join("stamp.sut"), &file);
    let p = peek(&dir.join("stamp.sut")).unwrap();
    let side = PREVIEW_SIDE as usize;
    assert_eq!(
        (p.preview.side as usize, p.preview.alpha.len()),
        (side, side * side)
    );
    let at = |x: usize, y: usize| p.preview.alpha[y * side + x];
    // PNG の左上と右下が暗い（塗る）。見本も同じ向き（PNG の上が見本の上）
    assert_eq!(
        (at(side / 4, side / 4), at(3 * side / 4, side / 4)),
        (255, 0)
    );
    assert_eq!(
        (at(side / 4, 3 * side / 4), at(3 * side / 4, 3 * side / 4)),
        (0, 255)
    );
    std::fs::remove_dir_all(dir).unwrap();

    // 丸い筆先: 真ん中は塗り、隅は塗らない。硬さが低いと縁がぼける
    let hard = preview_of(&yolu_core::Brush::default());
    let center = (side / 2) * side + side / 2;
    assert_eq!(hard.alpha[center], 255);
    assert_eq!(hard.alpha[0], 0);
    let mut soft_brush = yolu_core::Brush::default();
    soft_brush.base.hardness = 0.0;
    let soft = preview_of(&soft_brush);
    let mid = (side / 2) * side + side / 2 + side / 4;
    assert!(
        soft.alpha[mid] > 0 && soft.alpha[mid] < 255,
        "{}",
        soft.alpha[mid]
    );
    // 真円率が低いと縦に潰れる
    let mut flat_brush = yolu_core::Brush::default();
    flat_brush.tip.roundness = 0.25;
    let flat = preview_of(&flat_brush);
    let above = (side / 2 - side / 4) * side + side / 2;
    assert_eq!(flat.alpha[above], 0);
    assert_eq!(hard.alpha[above], 255);
}
