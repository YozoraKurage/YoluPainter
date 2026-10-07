//! 描画試験のウィンドウの貸し出し（`common/gpu_thread.rs`）の試験と、ウィンドウを作る口が貸し出しを通っているかの確かめ。
mod common;
use common::{canvas_device, gpu_thread};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

/// この確かめどうしが、貸し出しの状態（`is_free`）を混ぜないように 1 つずつ走らせる。
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn callers_use_the_same_thread() {
    let _serial = serial();
    static THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let caller = std::thread::current().id();
                gpu_thread::run(|| {
                    let current = std::thread::current().id();
                    assert_eq!(*THREAD.get_or_init(|| current), current);
                });
                assert_ne!(*THREAD.get().unwrap(), caller);
            });
        }
    });
}

#[test]
fn a_failure_returns_to_the_caller_and_the_next_test_runs() {
    let _serial = serial();
    let failure = std::panic::catch_unwind(|| gpu_thread::run(|| panic!("試験の失敗")));
    let payload = failure.expect_err("失敗を成功として扱わない");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"試験の失敗"));
    gpu_thread::run(|| {});
}

#[test]
fn a_failure_carries_the_place_it_panicked_at() {
    let _serial = serial();
    // 次の行で落とす（整形で折れない長さに保つ。折れると行がずれる）
    let line = line!() + 1;
    let failure = gpu_thread::run_checked(|| panic!("場所を見る"));
    let failure = failure.expect_err("失敗を成功として扱わない");
    let at = failure.location.as_deref().expect("落ちた場所が残る");
    assert!(at.contains("window_lease.rs"), "ファイル名: {at}");
    assert!(at.contains(&format!(":{line}:")), "行 {line}: {at}");
    let described = failure.describe();
    assert!(
        described.contains("場所を見る") && described.contains(at),
        "{described}"
    );
    // 次の試験の場所に前の失敗が混ざらない。
    assert!(gpu_thread::run_checked(|| {}).is_ok());
    let line = line!() + 1;
    let second = gpu_thread::run_checked(|| panic!("二度目")).expect_err("失敗");
    assert!(second.location.unwrap().contains(&format!(":{line}:")));
}

/// ウィンドウを持つ試験どうしは重ならない: 持っている間に、同時に持っているスレッドは 1 つだけ。
#[test]
fn two_threads_never_hold_windows_at_once() {
    let _serial = serial();
    static HOLDING: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                gpu_thread::lease();
                let now = HOLDING.fetch_add(1, Ordering::SeqCst) + 1;
                MOST.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(15));
                HOLDING.fetch_sub(1, Ordering::SeqCst);
                // 放すのはスレッドが終わるとき
            });
        }
    });
    assert_eq!(MOST.load(Ordering::SeqCst), 1);
    assert_eq!(HOLDING.load(Ordering::SeqCst), 0);
}

/// 同じスレッドの 2 回目の貸し出しは待たない（試験がウィンドウを 2 つ作っても止まらない）。持っている間は、ほかのスレッドから見て空いていない。
#[test]
fn a_thread_can_lease_again_and_others_wait_until_it_ends() {
    let _serial = serial();
    let (held, held_rx) = std::sync::mpsc::channel();
    let (finish, finish_rx) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::spawn(move || {
        gpu_thread::lease();
        gpu_thread::lease();
        assert!(!gpu_thread::is_free(), "自分が持っている間は空いていない");
        held.send(()).unwrap();
        finish_rx.recv().unwrap();
    });
    held_rx.recv().unwrap();
    let other = std::thread::spawn(|| {
        assert!(
            !gpu_thread::is_free(),
            "ほかのスレッドが持っている間は空いていない"
        );
    });
    other.join().unwrap();
    finish.send(()).unwrap();
    owner.join().unwrap();
    // スレッドが終わると放される（join はスレッドの後始末まで待つ）
    assert!(
        gpu_thread::is_free(),
        "終わったスレッドの貸し出しが残っている"
    );
}

/// 常駐のスレッドへ送る試験は、呼び手の貸し出しを先に放し（さもないと常駐のスレッドが待ち、呼び手が結果を待つ）、
/// ジョブが終わるたびに常駐のスレッドの分も放す。
#[test]
fn run_after_a_lease_does_not_deadlock_and_leaves_nothing_held() {
    let _serial = serial();
    let thread = std::thread::spawn(|| {
        gpu_thread::lease();
        gpu_thread::run(|| {
            gpu_thread::lease();
            assert!(!gpu_thread::is_free(), "ジョブの間はウィンドウを持っている");
        });
        // 呼び手は放した。常駐のスレッドもジョブごとに放した
        assert!(gpu_thread::is_free());
    });
    thread.join().unwrap();
    assert!(gpu_thread::is_free());
}

/// ウィンドウを作る試験のスレッドが落ちても（panic）、貸し出しは放される。
#[test]
fn a_panicking_test_thread_releases_its_lease() {
    let _serial = serial();
    let thread = std::thread::spawn(|| {
        gpu_thread::lease();
        panic!("ウィンドウを持ったまま落ちる");
    });
    assert!(thread.join().is_err());
    assert!(gpu_thread::is_free());
    // 次の試験もウィンドウを作れる（貸し出しが壊れていない）
    std::thread::spawn(gpu_thread::lease).join().unwrap();
    assert!(gpu_thread::is_free());
}

/// GPU 合成の試験が自前で作る装置（`canvas_device::begin`）も、ウィンドウと同じ貸し出しを通る。通らないと、ウィンドウを持つ試験と同時に別々の装置を作る
/// （装置が無い環境でも、貸し出しは装置を探す前に取る）。
#[test]
fn the_canvas_device_is_taken_through_the_same_lease() {
    let _serial = serial();
    let (held, held_rx) = std::sync::mpsc::channel();
    let (finish, finish_rx) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::spawn(move || {
        let device = canvas_device::begin("window_lease");
        assert!(
            !gpu_thread::is_free(),
            "装置を作る間はウィンドウと同じ貸し出しを持つ"
        );
        held.send(()).unwrap();
        finish_rx.recv().unwrap();
        drop(device);
    });
    held_rx.recv().unwrap();
    std::thread::spawn(|| assert!(!gpu_thread::is_free(), "ほかのスレッドから見て空いていない"))
        .join()
        .unwrap();
    finish.send(()).unwrap();
    owner.join().unwrap();
    assert!(
        gpu_thread::is_free(),
        "終わったスレッドの貸し出しが残っている"
    );
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// kittest のウィンドウは `common::gpu_thread::builder()` から作る。ほかの口（`Harness::builder`・`Harness::new*`・`HarnessBuilder`・`Harness::default`）で
/// 作ると貸し出しを通らず、ウィンドウを同時に作って落ちる試験が戻る。新しい試験がそうしていないかを、ソースで確かめる。
#[test]
fn every_window_is_built_through_the_lease() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    rust_files(&tests, &mut files);
    assert!(
        files.len() > 50,
        "試験のファイルが見つからない: {}",
        files.len()
    );
    let forbidden = [
        "Harness::builder",
        "Harness::new",
        "HarnessBuilder::",
        "Harness::default",
        "Harness::<",
    ];
    let mut bad = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(&tests)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // この確かめ自身と、貸し出しの口（builder の中）は除く
        if name == "window_lease.rs" || name == "common/gpu_thread.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if forbidden.iter().any(|f| code.contains(f)) {
                bad.push(format!("{name}:{}: {}", number + 1, code.trim_end()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "ウィンドウを貸し出しを通さずに作っている:\n{}",
        bad.join("\n")
    );
}

/// ウィンドウ（harness）を作らなくても、焼く場所に GPU（`Gpu`・`Auto`）を選んで確かめる試験は、製品のスレッドで GPU の装置を作る。
/// その試験は貸し出し（`gpu_thread::lease`・`builder`・`run`・`canvas_device::begin` のどれか）を取る。呼び忘れを、関数ごとにソースで確かめる
/// （画面の試験の `fix_gpu_probe` と押すだけの確かめは、`Backend(` を直接 apply しないので対象にならない）。
#[test]
fn every_gpu_bake_test_takes_the_lease() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut files = Vec::new();
    rust_files(&tests, &mut files);
    let mut bad = Vec::new();
    let mut checked = 0;
    for path in files {
        let name = path
            .strip_prefix(&tests)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if name == "window_lease.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // 行頭の `fn` ごとに区切る（試験の関数は、どのファイルでも行頭から始まる）
        let mut functions: Vec<(usize, Vec<&str>)> = Vec::new();
        for (number, line) in text.lines().enumerate() {
            if line.starts_with("fn ") || line.starts_with("pub fn ") {
                functions.push((number + 1, Vec::new()));
            }
            if let Some((_, body)) = functions.last_mut() {
                body.push(line);
            }
        }
        for (start, body) in functions {
            let picks_gpu = body.iter().any(|line| {
                let code = line.trim_start();
                !code.starts_with("//")
                    && (code.contains("BakeBackend::Gpu") || code.contains("BakeBackend::Auto"))
                    && (code.contains("Backend(BakeBackend") || code.contains("backend = "))
            });
            if !picks_gpu {
                continue;
            }
            checked += 1;
            let takes_lease = body.iter().any(|line| {
                let code = line.trim_start();
                !code.starts_with("//")
                    && (code.contains("gpu_thread::") || code.contains("canvas_device::begin("))
            });
            if !takes_lease {
                bad.push(format!("{name}:{start}: {}", body[0].trim_end()));
            }
        }
    }
    assert!(
        checked >= 1,
        "GPU で焼く試験が 1 つも見つからない（見つけ方が古くなった）"
    );
    assert!(
        bad.is_empty(),
        "GPU を選んで焼く試験が貸し出しを取っていない（先頭で `common::gpu_thread::lease()` を呼ぶ）:\n{}",
        bad.join("\n")
    );
}
