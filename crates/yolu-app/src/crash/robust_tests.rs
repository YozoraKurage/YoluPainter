//! 頑丈さの試験: 受け止めた panic の扱い（サムネイルの仕事場・文書を開く変換）、割り当ての失敗と abort の記録。
//! 実際の hook・シグナルは、この試験の実行ファイルを子として起こして確かめる（`child_robust`）。
use super::*;

/// この試験の実行ファイル（と、子として起こすもの）も、確保の失敗を記録するアロケーターで動かす（製品は `main.rs`）。
#[global_allocator]
static ALLOCATOR: RecordingAlloc = RecordingAlloc;

/// 確保できない大きさ（128 TiB。64 ビットの利用者の空間より大きい）。
const IMPOSSIBLE: usize = 1 << 47;

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("yolu-robust-test-{}", stamp()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 子プロセスの本体。環境変数で渡された記録先へ `install_at` し、`YOLU_ROBUST_MODE` の動きをする。
#[test]
fn child_robust() {
    let Ok(dir) = std::env::var("YOLU_ROBUST_DIR") else {
        return;
    };
    install_at(PathBuf::from(dir));
    match std::env::var("YOLU_ROBUST_MODE")
        .unwrap_or_default()
        .as_str()
    {
        "service" => {
            // サムネイルの仕事場: 仕事が panic しても、スレッドは続き、結果は None（落ちた記録にはならない）
            let service = crate::library::service::Service::<u8>::new(1);
            service.request("thumb", || {
                Box::new(|_| -> u8 { panic!("thumbnail worker panic") })
            });
            service.wait_done();
            let done = service.poll(8);
            assert!(
                matches!(done.as_slice(), [(key, None)] if key == "thumb"),
                "{}",
                done.len()
            );
            // 続けて、次の仕事も走る
            service.request("next", || Box::new(|_| 7u8));
            service.wait_done();
            assert!(matches!(service.poll(8).as_slice(), [(_, Some(7))]));
            panic!("final panic");
        }
        "convert" => {
            // 文書を開く変換の途中の panic は、理由の文の失敗になる（ほかのセットは開く）
            let failed: Result<u8, String> =
                crate::project::caught_as_text(crate::lang::Lang::En, || {
                    panic!("conversion panic")
                });
            assert_eq!(failed.unwrap_err(), "Reading the project stopped");
            assert_eq!(
                crate::project::caught_as_text(crate::lang::Lang::En, || Ok(3u8)),
                Ok(3)
            );
            panic!("final panic");
        }
        "alloc_fatal" => {
            // 確保できない大きさを求め、標準の確保の失敗の道（`handle_alloc_error` → abort）を通る
            let layout = std::alloc::Layout::from_size_align(IMPOSSIBLE, 1).unwrap();
            // SAFETY: 返った番地は使わない（null のはず）。
            let ptr = unsafe { std::alloc::alloc(layout) };
            assert!(ptr.is_null());
            std::alloc::handle_alloc_error(layout);
        }
        "alloc_handled" => {
            // 受け止めて動き続ける失敗（`try_reserve` の断り）。正常な終わり（`main` と同じ後始末）まで進む
            let mut buffer = Vec::<u8>::new();
            assert!(buffer.try_reserve_exact(IMPOSSIBLE).is_err());
            cleanup();
        }
        "alloc_handled_then_abort" => {
            let mut buffer = Vec::<u8>::new();
            assert!(buffer.try_reserve_exact(IMPOSSIBLE).is_err());
            // 受け止めた失敗のあとで、ほかの理由で止まる
            std::process::abort();
        }
        "double_panic" => {
            struct Bomb;
            impl Drop for Bomb {
                fn drop(&mut self) {
                    panic!("second panic in a destructor");
                }
            }
            let _bomb = Bomb;
            panic!("first panic");
        }
        "event" => {
            // panic ではない出来事（GPU の装置を失った、など）は、落ちた記録の枠と普段のログに書ける
            let path = event(
                "GPU device lost",
                "Adapter: Sample (Vulkan)\nReason: Unknown\nSaved for recovery: Yes",
            );
            assert!(path.is_some_and(|p| p.is_file()));
            note("GPU error: Validation Error: sample\nsecond line");
            note("GPU error: private image.png");
            cleanup();
        }
        other => panic!("知らない動き: {other}"),
    }
}

fn run_child(mode: &str) -> (Dir, std::process::Output) {
    let dir = Dir::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash::robust_tests::child_robust",
            "--nocapture",
        ])
        .env("YOLU_ROBUST_DIR", &dir.0)
        .env("YOLU_ROBUST_MODE", mode)
        .output()
        .unwrap();
    (dir, output)
}

fn crash_texts(dir: &Path) -> Vec<String> {
    files(dir, "crash-")
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect()
}

fn session_text(dir: &Path) -> String {
    files(dir, "session-")
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect()
}

/// 受け止めて続けた panic は落ちた記録（次の起動の「落ちました」）にせず、最後の本物の panic だけが記録になる。
fn only_the_final_panic_is_a_crash(mode: &str, handled_text: &str) {
    let (dir, output) = run_child(mode);
    assert!(!output.status.success(), "最後の panic で終わる");
    let texts = crash_texts(&dir.0);
    let panics: Vec<_> = texts
        .iter()
        .filter(|t| t.contains("Kind: Rust panic"))
        .collect();
    assert_eq!(
        panics.len(),
        1,
        "{texts:?}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(panics[0].contains("final panic"));
    assert!(!panics[0].contains(handled_text));
    let session = session_text(&dir.0);
    assert!(
        session.contains("Handled panic") && session.contains(handled_text),
        "{session}"
    );
    assert!(window::Report::load(dir.0.clone())
        .text
        .contains("final panic"));
    assert!(!window::Report::load(dir.0.clone())
        .text
        .contains(handled_text));
}

#[test]
fn a_panic_the_thumbnail_service_survives_is_not_a_crash_record() {
    only_the_final_panic_is_a_crash("service", "thumbnail worker panic");
}

#[test]
fn a_panic_while_converting_a_document_to_open_it_is_not_a_crash_record() {
    only_the_final_panic_is_a_crash("convert", "conversion panic");
}

/// 確保の失敗が書かれた記録（この起動のネイティブ記録先）を探す。
fn allocation_record(dir: &Path) -> Option<String> {
    crash_texts(dir)
        .into_iter()
        .find(|t| t.contains("Allocation failed"))
}

/// 確保が失敗して止まると、失敗した大きさと呼び出しの番地が、次の起動に出る記録に残る。ヘッダーは 1 つだけ。
/// Linux では、続けて来る SIGABRT の行が欄の後ろに付く。Windows の abort（`__fastfail`）はフィルターに来ないので、この欄が唯一の手がかり。
#[test]
fn an_allocation_failure_that_ends_the_process_leaves_its_size_and_frames_in_the_record() {
    let (dir, output) = run_child("alloc_fatal");
    assert!(!output.status.success(), "確保の失敗で止まる");
    let record = allocation_record(&dir.0).unwrap_or_else(|| {
        panic!(
            "{:?}\n{}",
            crash_texts(&dir.0),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        record.contains(&format!("Allocation failed: {IMPOSSIBLE} bytes (align 1)")),
        "{record}"
    );
    assert!(record.contains("Kind: Native crash"), "{record}");
    assert_eq!(
        record.matches("YoluPainter ").count(),
        1,
        "ヘッダーは 1 つ: {record}"
    );
    assert!(record.contains("Image base: 0x"), "{record}");
    #[cfg(any(windows, all(target_os = "linux", target_env = "gnu")))]
    assert!(
        record.lines().any(|l| l.starts_with("0x")),
        "呼び出しの番地: {record}"
    );
    #[cfg(target_os = "linux")]
    {
        let at = record.find("Signal: SIGABRT").expect(&record);
        assert!(
            at > record.find("Allocation failed").unwrap(),
            "シグナルの行は欄の後ろ"
        );
    }
    // 次の起動が読む記録になる
    assert!(window::Report::load(dir.0.clone())
        .text
        .contains("Allocation failed"));
}

/// 受け止めて動き続けた確保の失敗（`try_reserve` の断り）は、正常に終われば落ちた記録に残らない。
#[test]
fn a_handled_allocation_failure_is_not_a_crash_record_after_a_clean_exit() {
    let (dir, output) = run_child("alloc_handled");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        crash_texts(&dir.0).iter().all(|t| t.is_empty()),
        "{:?}",
        crash_texts(&dir.0)
    );
    assert!(
        files(&dir.0, "crash-").is_empty(),
        "空の記録先は残さない: {:?}",
        files(&dir.0, "crash-")
    );
    assert!(!window::Report::load(dir.0.clone()).unread);
}

/// 受け止めた失敗のあとに別の理由で止まったら、その記録に失敗の欄も入る（直前のメモリの逼迫が分かる）。
#[cfg(target_os = "linux")]
#[test]
fn an_earlier_handled_failure_stays_in_the_record_of_a_later_abort() {
    let (dir, output) = run_child("alloc_handled_then_abort");
    assert!(!output.status.success());
    let record = allocation_record(&dir.0).expect("失敗の欄");
    assert!(record.contains("Signal: SIGABRT"), "{record}");
    assert_eq!(record.matches("YoluPainter ").count(), 1, "{record}");
}

/// 二重の panic（破棄の中の panic）で止まるときも、2 つとも落ちた記録に残る（abort の前に hook が走る）。
#[test]
fn a_panic_inside_a_destructor_during_a_panic_leaves_both_panics_in_the_record() {
    let (dir, output) = run_child("double_panic");
    assert!(!output.status.success());
    let texts = crash_texts(&dir.0);
    let panics: Vec<_> = texts
        .iter()
        .filter(|t| t.contains("Kind: Rust panic"))
        .collect();
    assert!(
        panics.iter().any(|t| t.contains("first panic")),
        "{texts:?}"
    );
    assert!(
        panics
            .iter()
            .any(|t| t.contains("second panic in a destructor")),
        "{texts:?}"
    );
}

/// panic ではない出来事は、落ちた記録の枠に書かれ、次の起動に報告の印が出る。診断の 1 行は普段のログへ（名前のある行は伏せる）。
#[test]
fn an_event_is_a_crash_record_and_a_note_is_one_line_of_the_ordinary_log() {
    let (dir, output) = run_child("event");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let texts = crash_texts(&dir.0);
    let events: Vec<_> = texts
        .iter()
        .filter(|t| t.contains("Kind: GPU device lost"))
        .collect();
    assert_eq!(events.len(), 1, "{texts:?}");
    assert!(
        events[0].contains("Reason: Unknown") && events[0].contains("Saved for recovery: Yes"),
        "{}",
        events[0]
    );
    assert!(
        window::Report::load(dir.0.clone()).unread,
        "次の起動に報告の印が出る"
    );
    let session = session_text(&dir.0);
    assert!(
        session.contains("GPU error: Validation Error: sample | second line"),
        "{session}"
    );
    assert!(
        !session.contains("private"),
        "名前のある行は伏せる: {session}"
    );
}
