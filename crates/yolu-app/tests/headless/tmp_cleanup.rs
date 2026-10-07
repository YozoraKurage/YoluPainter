//! 試験が作る一時の物の後始末（`common::tmp`）の確かめ。試験（のスレッド）が終わると、控えたフォルダ（Live Link の受け渡しのフォルダを含む）が
//! 残らず、控えていない物には触らない。ここで作るフォルダは、試験の本体の代わりに別のスレッドを立てて作る（libtest は試験ごとにスレッドを
//! 分けるので、「スレッドが終わる」が「試験が終わる」）。
use std::path::PathBuf;

use crate::common::{livelink::Exchange, tmp};

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
    assert!(
        !dir.exists(),
        "試験（のスレッド）が終わったら消える: {}",
        dir.display()
    );
}

#[test]
fn directories_made_by_two_calls_do_not_collide() {
    let (a, b) = made_in_a_thread(|| (tmp::test_dir("same"), tmp::test_dir("same")));
    assert_ne!(a, b, "同じタグでも別のフォルダ");
}

#[test]
fn a_directory_registered_after_creation_is_removed_with_its_contents_and_a_failing_test_is_cleaned_too(
) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("yolu-test-registered-{}", std::process::id()));
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
fn a_live_link_exchange_folder_is_gone_with_what_was_put_in_it_and_a_neighbour_is_left_alone() {
    let neighbour =
        std::env::temp_dir().join(format!("yolu-test-neighbour-{}", std::process::id()));
    std::fs::create_dir_all(&neighbour).unwrap();
    let (root, put) = made_in_a_thread(|| {
        let exchange = Exchange::new("cleanup");
        exchange.put_bytes("request.json", b"{}");
        let put = exchange.folder().inbox().join("request.json");
        assert!(put.is_file(), "試験の間は置いた頼みがある");
        (exchange.root.clone(), put)
    });
    assert!(
        !put.exists() && !root.exists(),
        "試験が終わったら、受け渡しのフォルダごと消える: {}",
        root.display()
    );
    assert!(neighbour.is_dir(), "控えていないフォルダには触らない");
    std::fs::remove_dir_all(&neighbour).unwrap();
}

#[test]
fn a_backdated_file_reports_the_new_modified_time_and_a_rewrite_changes_it() {
    let (dir, path) = made_in_a_thread(|| {
        let dir = tmp::test_dir("backdate");
        let path = dir.join("a.txt");
        std::fs::write(&path, b"first").unwrap();
        let old = tmp::backdate(&path);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
        assert!(
            old < std::time::SystemTime::now() - std::time::Duration::from_secs(300),
            "少し前（時刻の粒度より十分前）"
        );
        // 書き直すと、返った時刻とは食い違う（待たずに見つかる）
        std::fs::write(&path, b"second").unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
        (dir.clone(), path)
    });
    assert!(!dir.exists() && !path.exists());
}
