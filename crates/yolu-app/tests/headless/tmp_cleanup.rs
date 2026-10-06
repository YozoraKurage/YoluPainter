//! 試験が作る一時の物の後始末（`common::tmp`・`common::names`）の確かめ。試験（のスレッド）が終わると、フォルダ・Live Link の名前のファイルが
//! 残らない。ここで作るフォルダ・名前は、試験の本体の代わりに別のスレッドを立てて作る（libtest は試験ごとにスレッドを分けるので、
//! 「スレッドが終わる」が「試験が終わる」）。
use std::path::PathBuf;

use crate::common::{names, tmp};

/// スレッドの中で作ったものを、スレッドが終わったあとで確かめる。
fn made_in_a_thread<T: Send + 'static>(make: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(make).join().expect("スレッドが落ちない")
}

#[test]
fn a_test_directory_is_gone_when_the_test_thread_ends_and_not_before() {
    let (dir, inside_exists) = made_in_a_thread(|| {
        let dir = tmp::test_dir("cleanup");
        std::fs::write(dir.join("file.txt"), b"x").unwrap();
        let inside_exists = dir.join("file.txt").is_file();
        (dir, inside_exists)
    });
    assert!(inside_exists, "試験の間は使える");
    assert!(!dir.exists(), "試験（のスレッド）が終わったら消える: {}", dir.display());
}

#[test]
fn directories_made_by_two_calls_do_not_collide() {
    let (a, b) = made_in_a_thread(|| (tmp::test_dir("same"), tmp::test_dir("same")));
    assert_ne!(a, b, "同じタグでも別のフォルダ");
}

#[test]
fn a_directory_registered_after_creation_is_removed_with_its_contents_and_a_failing_test_is_cleaned_too() {
    let dir: PathBuf = std::env::temp_dir().join(format!("yolu-test-registered-{}", std::process::id()));
    let made = dir.clone();
    // 落ちる（panic する）試験でも、スレッドの後始末は走る
    let outcome = std::thread::spawn(move || {
        std::fs::create_dir_all(made.join("inner")).unwrap();
        std::fs::write(made.join("inner/file.txt"), b"x").unwrap();
        tmp::clean_up_after_test(&made);
        panic!("試験の失敗");
    })
    .join();
    assert!(outcome.is_err());
    assert!(!dir.exists(), "落ちた試験の分も消える");
}

#[test]
fn sweep_removes_what_the_thread_registered_right_away() {
    made_in_a_thread(|| {
        let dir = tmp::test_dir("sweep");
        assert!(dir.is_dir());
        tmp::sweep();
        assert!(!dir.exists(), "常駐のスレッドはジョブごとにこれを呼ぶ");
    });
}

#[test]
fn live_link_names_leave_no_lock_key_or_socket_file_after_the_test() {
    let (name, files, held) = made_in_a_thread(|| {
        let name = names::unique_name("ylclean", "files");
        let server = yolu_protocol::Server::bind(&name, false).expect("待ち受けられる");
        let dir = yolu_protocol::private::link_dir().unwrap();
        let files: Vec<PathBuf> = ["lock", "key", "sock"].iter().map(|e| dir.join(format!("{name}.{e}"))).collect();
        let held = files.iter().filter(|f| f.exists()).count();
        drop(server);
        (name, files, held)
    });
    assert!(held >= 2, "待ち受けている間は、鍵とロックのファイルがある（{name}）");
    for file in files {
        assert!(!file.exists(), "試験が終わったら消える: {}", file.display());
    }
}

#[test]
fn live_link_names_do_not_repeat() {
    let first = names::unique_name("ylclean", "same");
    let second = names::unique_name("ylclean", "same");
    assert_ne!(first, second);
}

#[test]
fn a_backdated_file_reports_the_new_modified_time_and_a_rewrite_changes_it() {
    let (dir, path) = made_in_a_thread(|| {
        let dir = tmp::test_dir("backdate");
        let path = dir.join("a.txt");
        std::fs::write(&path, b"first").unwrap();
        let old = tmp::backdate(&path);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
        assert!(old < std::time::SystemTime::now() - std::time::Duration::from_secs(300), "少し前（時刻の粒度より十分前）");
        // 書き直すと、返った時刻とは食い違う（待たずに見つかる）
        std::fs::write(&path, b"second").unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
        (dir.clone(), path)
    });
    assert!(!dir.exists() && !path.exists());
}
