use super::*;
use crate::{lang::Lang, state::Action};
use egui_kittest::{kittest::Queryable, Harness};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("yolu-crash-test-{}", stamp()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn redacts_case_slashes_unicode_and_filenames_with_spaces() {
    let r = Redactor {
        homes: vec!["C:\\Users\\SampleUser".into(), "/home/架空利用者".into()],
        users: vec!["SampleUser".into(), "架空利用者".into()],
    };
    let text = r.redact("C:\\USERS\\SAMPLEUSER\\code.rs\nC:/Users/SampleUser/code.rs\n/home/架空利用者/code.rs\nSampleUser\nfailed: My private painting.PSD\n模型 ファイル.fbx\n\"secret.ylp\"\nC:\\\\Users\\\\SampleUser\\\\code.rs");
    for private in [
        "SampleUser",
        "SAMPLEUSER",
        "架空利用者",
        "private",
        "模型",
        "secret",
    ] {
        assert!(!text.contains(private), "{text}");
    }
    assert!(text.contains("~/code.rs"));
    assert!(text.contains("[user]"));
}

/// 利用者名が a〜f と数字だけの短い名前でも、番地（0x…）は壊さない。名前は番地の外でだけ伏せる。
#[test]
fn a_hex_like_user_name_does_not_break_addresses() {
    let r = Redactor {
        homes: Vec::new(),
        users: vec!["ed".into()],
    };
    let text = "Image base: 0x7ff6ed12ab34\n  3:     0x00007ff6a1ed34ed - ed::run\n 12: 0XED - x\n/home/ED/code.rs\ned0x1f\n0xed";
    let redacted = r.redact(text);
    let lines: Vec<&str> = redacted.lines().collect();
    assert_eq!(lines[0], "Image base: 0x7ff6ed12ab34");
    assert_eq!(lines[1], "  3:     0x00007ff6a1ed34ed - [user]::run");
    assert_eq!(lines[2], " 12: 0XED - x");
    assert_eq!(lines[3], "/home/[user]/code.rs");
    // 語の途中の 0x は番地ではない（名前は伏せる）。番地そのものは 1 桁でも残す
    assert_eq!(lines[4], "[user]0x1f");
    assert_eq!(lines[5], "0xed");
    // 名前と番地が重なって並ぶとき、番地の外の名前は伏せ、重なった一致を見落とさない
    assert_eq!(
        r.redact("eded 0xeded eded"),
        "[user][user] 0xeded [user][user]"
    );
    // 番地とみなすのは、語の頭の 0x と 16 進の数字が 1 桁以上続くものだけ
    assert_eq!(hex_words("a0x1 0x 0xg 0x1g _0x2 (0x3)"), [12..15, 23..26]);
}

#[test]
fn crash_rotation_size_and_actions_only() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    for _ in 0..35 {
        r.action(Action::SaveProjectAs(PathBuf::from("private folder/secret.ylp")).kind_name());
        r.action(Action::Undo.kind_name());
    }
    for _ in 0..23 {
        r.record("test", "safe detail").unwrap();
    }
    let paths = files(&dir.0, "crash-");
    assert_eq!(paths.len(), 20);
    let text = read_report(paths.last().unwrap()).unwrap();
    // 30 件は、種類名が替わるたびの 1 件（15 組）
    assert_eq!(text.matches("SaveProjectAs").count(), 15);
    assert_eq!(text.matches("Undo").count(), 15);
    assert!(!text.contains("secret"));
    assert!(!text.contains("private folder"));
    let path = r.record("test", &"あ".repeat(MAX_BYTES)).unwrap();
    assert!(fs::metadata(&path).unwrap().len() <= MAX_BYTES as u64);
    assert!(fs::read_to_string(path).unwrap().ends_with("[truncated]\n"));
}

#[test]
fn consecutive_identical_actions_are_one_entry_with_a_count() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    for _ in 0..500 {
        r.action(Action::ZoomIn.kind_name());
    }
    r.action(Action::NewLayer.kind_name());
    let text = read_report(&r.record("test", "x").unwrap()).unwrap();
    assert!(text.contains("Actions: ZoomIn x500, NewLayer\n"), "{text}");
}

#[test]
fn ordinary_log_rotation_and_unwritable_directory_are_nonfatal() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    for i in 0..8 {
        let text = format!("{i} {}", "x".repeat(MAX_BYTES));
        r.note_problem(&text);
        r.message(&text);
    }
    assert_eq!(files(&dir.0, "session-").len(), 5);
    for path in files(&dir.0, "session-") {
        assert!(fs::metadata(path).unwrap().len() <= MAX_BYTES as u64);
    }
    let invalid = dir.0.join("file");
    fs::write(&invalid, "occupied").unwrap();
    let broken = Recorder::new(invalid);
    assert!(broken.record("test", "failed").is_none());
    broken.note_problem("failed");
    broken.message("failed");
}

#[test]
fn startup_failure_can_be_recorded_without_a_window() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    let path = record_startup_failure(&r, "GPU adapter unavailable").unwrap();
    let text = read_report(&path).unwrap();
    assert!(text.contains("Startup failure"));
    assert!(text.contains("GPU adapter unavailable"));
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    assert!(text.contains(std::env::consts::OS));
    let url = window::issue_url();
    assert!(url.contains("/issues/new?title="));
    assert!(!url.contains("GPU"));
    assert!(!url.contains("Backtrace"));
}

#[test]
fn child_crash() {
    let Ok(dir) = std::env::var("YOLU_TEST_CRASH_DIR") else {
        return;
    };
    install_at(PathBuf::from(dir));
    action(Action::SaveProjectAs("hidden/secret.ylp".into()).kind_name());
    let mode = std::env::var("YOLU_TEST_CRASH_MODE").unwrap_or_default();
    if mode == "native" {
        #[cfg(windows)]
        // SAFETY: 試験だけの子プロセスで、継続不能の例外を明示的に発生させる。
        unsafe {
            windows::Win32::System::Diagnostics::Debug::RaiseException(0xe0424242, 1, None);
        }
        std::process::abort();
    }
    if mode == "rotation" {
        for _ in 0..25 {
            LOGGER.get().unwrap().record("caught", "handled panic");
        }
        std::process::abort();
    }
    if mode == "guarded" {
        guard(|| panic!("guarded panic"), || {});
    }
    if mode == "handled" {
        // 受け止めて動き続ける panic（復旧の書き手・見本の描画と同じ形）を、落ちた記録の枠より多く起こす。
        for i in 0..25 {
            let result = handled(|| {
                if i == 0 {
                    panic!("handled panic private image.psd");
                }
                panic!("handled panic {i}");
            });
            assert!(result.is_err());
        }
        panic!("final panic");
    }
    if mode == "repeat" {
        for _ in 0..5 {
            let _ = std::panic::catch_unwind(|| panic!("same panic"));
        }
        panic!("final panic");
    }
    if mode == "writer" {
        // 復旧の書き手の panic は、書き手が受け止めて動き続ける（落ちではない）。実際の書き手で起こす。
        let mut state = crate::state::AppState::new_in(64, 64, Lang::En);
        state
            .recovery
            .set_fault(Some(std::sync::Arc::new(|stage: &str| {
                if stage == "snapshot" {
                    panic!("writer panic");
                }
                Ok(())
            })));
        let root = std::env::temp_dir().join(format!("yolu-crash-writer-{}", stamp()));
        state
            .recovery
            .enable(root.clone(), crate::recovery::RecoverySettings::default())
            .unwrap();
        let layer = state.selected_layer.unwrap();
        let brush = state.stroke_settings(false);
        let mut stroke = state.doc.begin_stroke(layer, &brush).unwrap();
        stroke
            .add_point(&mut state.doc, 20.0, 20.0, 1.0, crate::engine::DVec2::ZERO)
            .unwrap();
        state.doc.end_stroke(stroke).unwrap();
        state.modified = true;
        state.recovery_flush();
        let _ = fs::remove_dir_all(root);
        panic!("final panic");
    }
    if mode == "app" {
        // AppState::apply の配線。レイヤーの名前は記録に入らず、操作の種類名と失敗の文（名前を除いた理由）だけが入る。
        let mut state = crate::state::AppState::new_in(32, 32, Lang::En);
        let id = state.selected_layer.unwrap();
        state.doc.set_layer_name(id, "SecretLayerName").unwrap();
        for _ in 0..40 {
            state.apply(Action::ZoomIn);
        }
        state.apply(Action::StartRename(id));
        state.apply(Action::NewLayer);
        state.apply(Action::Undo);
        state.message = "Saved.".into();
        message(&state.message);
        let why = Lang::En.core_error(&yolu_core::CoreError::LayerNotFound);
        state.message = format!("SecretLayerName: {why}");
        message(&state.message);
        panic!("app panic");
    }
    if mode == "notices" {
        // 知らせの口の配線。注意と失敗は種類と出どころ（と覚えた理由）、断りは印を付けた文だけを書き、済んだ知らせと名前は書かない。
        use crate::notice::Source;
        let mut state = crate::state::AppState::new_in(32, 32, Lang::En);
        state.refuse(Source::Edit, crate::lang::refusals::during_stroke(Lang::En));
        state.info(Source::Save, "Saved SecretProjectName.");
        state.refuse(
            Source::Layer,
            "SecretLayerName is not a layer you can edit.",
        );
        state.warn(Source::Bake, "SecretSetName: baked with notes.");
        state.fail(
            Source::Save,
            format!(
                "SecretProjectName: {}",
                Lang::En.core_error(&yolu_core::CoreError::LayerNotFound)
            ),
        );
        panic!("notices panic");
    }
    #[cfg(target_os = "linux")]
    if mode == "overflow" {
        #[allow(unconditional_recursion)]
        fn recurse(n: u64) -> u64 {
            let page = [n; 512];
            std::hint::black_box(&page);
            recurse(n + 1) + page[0]
        }
        std::process::exit(recurse(0) as i32);
    }
    panic!("test panic\nprivate image.psd");
}

fn run_child_output(mode: &str) -> (Temp, std::process::Output) {
    run_child_output_with(mode, &[])
}
fn run_child_output_with(mode: &str, envs: &[(&str, &str)]) -> (Temp, std::process::Output) {
    let dir = Temp::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash::tests::child_crash", "--nocapture"])
        .env("YOLU_TEST_CRASH_DIR", &dir.0)
        .env("YOLU_TEST_CRASH_MODE", mode)
        .envs(envs.iter().copied())
        .output()
        .unwrap();
    assert!(!output.status.success());
    (dir, output)
}
fn run_child(mode: &str) -> Temp {
    run_child_output(mode).0
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

/// 「  12: 0x7ff6a1b2c3d4 - 名前」の形の行から、命令の番地を取る。
fn frame_address(line: &str) -> Option<usize> {
    let (index, rest) = line.trim_start().split_once(": ")?;
    index.parse::<u32>().ok()?;
    // 番地は桁をそろえるために左に空白が付く
    let hex = rest.trim_start().strip_prefix("0x")?.split(" - ").next()?;
    usize::from_str_radix(hex, 16).ok()
}

/// 基底の番地は、実行ファイルの中の関数の番地より前にある（相対の番地が正しく取れる）。
#[test]
fn the_image_base_precedes_the_code_of_this_executable() {
    let here = the_image_base_precedes_the_code_of_this_executable as fn() as usize;
    let base = super::image_base();
    if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
        let base = base.expect("基底が分かる OS");
        assert!(
            here > base && here - base < (1 << 31),
            "{here:#x} {base:#x}"
        );
    } else {
        assert!(base.is_none());
    }
}

#[test]
fn real_panic_hook_records_backtrace_and_redacts_payload() {
    let dir = run_child("panic");
    let report = window::Report::load(dir.0.clone());
    assert!(report.unread);
    assert!(report.text.contains("test panic"));
    assert!(report.text.contains("Backtrace:"));
    // 記号が解決できなくても、各フレームの番地が残る（番地 - 基底 を PDB で引く）。基底は頭の行に書く
    let frames: Vec<&str> = report
        .text
        .lines()
        .skip_while(|line| *line != "Backtrace:")
        .skip(1)
        .filter(|line| frame_address(line).is_some())
        .collect();
    assert!(
        frames.len() >= 3,
        "番地つきのフレームが無い: {}",
        report.text
    );
    if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
        let base = report
            .text
            .lines()
            .find_map(|line| line.strip_prefix("Image base: 0x"))
            .and_then(|hex| usize::from_str_radix(hex, 16).ok())
            .unwrap_or_else(|| panic!("基底の行が無い: {}", report.text));
        // 少なくとも 1 つのフレームは自分の実行ファイルの中（基底より後ろ）にある
        assert!(
            frames
                .iter()
                .filter_map(|line| frame_address(line))
                .any(|ip| ip > base && ip - base < (1 << 31)),
            "{}",
            report.text
        );
    }
    assert!(report.text.contains("SaveProjectAs"));
    assert!(!report.text.contains("private image"));
    assert!(!report.text.contains("secret.ylp"));
}
/// 利用者名が 16 進の文字だけの環境（USER・USERNAME が `d`）でも、記録の番地は 1 つも壊れず、基底の行も読める。
#[test]
fn a_hex_like_user_name_keeps_every_frame_address_in_a_real_panic_report() {
    let (dir, _) = run_child_output_with("panic", &[("USER", "d"), ("USERNAME", "d")]);
    let report = window::Report::load(dir.0.clone());
    let frames: Vec<&str> = report
        .text
        .lines()
        .skip_while(|line| *line != "Backtrace:")
        .skip(1)
        .filter(|line| {
            line.trim_start()
                .split_once(": ")
                .is_some_and(|(index, _)| index.parse::<u32>().is_ok())
        })
        .collect();
    assert!(frames.len() >= 3, "{}", report.text);
    for line in frames {
        assert!(
            frame_address(line).is_some(),
            "番地が壊れた: {line}\n{}",
            report.text
        );
    }
    if cfg!(any(windows, target_os = "linux", target_os = "macos")) {
        assert!(
            report
                .text
                .lines()
                .find_map(|line| line.strip_prefix("Image base: 0x"))
                .is_some_and(|hex| usize::from_str_radix(hex, 16).is_ok()),
            "{}",
            report.text
        );
    }
}
#[test]
#[cfg(target_os = "linux")]
fn linux_abort_writes_native_record() {
    let dir = run_child("native");
    let report = window::Report::load(dir.0.clone());
    assert!(report.unread);
    assert!(report.text.contains("SIGABRT"));
}

#[test]
fn report_ui_indicator_copy_folder_github_close_in_both_languages() {
    for lang in [Lang::Ja, Lang::En] {
        let dir = Temp::new();
        let r = Recorder::new(dir.0.clone());
        r.record("test", "sample report").unwrap();
        let report = window::Report::load(dir.0.clone());
        let expected = report.text.clone();
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(
                move |ui, state: &mut window::Report| {
                    state.indicator(ui, lang);
                    state.show(ui.ctx(), lang);
                },
                report,
            );
        h.run();
        h.get_by_label("!").click();
        h.run();
        assert!(h.state().open);
        h.get_by_label(lang.pick("コピー", "Copy")).click();
        h.step();
        assert!(h.output().platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if text == &expected)
        ));
        h.get_by_label(lang.pick("フォルダを開く", "Open Folder"))
            .click();
        h.run();
        assert_eq!(h.state_mut().request.take(), Some(window::Request::Folder));
        h.get_by_label(lang.pick("GitHub に報告", "Report on GitHub"))
            .click();
        h.run();
        assert_eq!(h.state_mut().request.take(), Some(window::Request::GitHub));
        h.get_by_label(lang.pick("閉じる", "Close")).click();
        h.run();
        assert!(!h.state().unread);
        assert!(!h.state().open);
        assert!(!window::Report::load(dir.0.clone()).unread);
        assert!(read_report(files(&dir.0, "crash-").last().unwrap()).is_some());
        r.record("test", "another crash").unwrap();
        assert!(window::Report::load(dir.0.clone()).unread);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn native_output_survives_rotation_of_caught_panics() {
    let dir = run_child("rotation");
    let paths = files(&dir.0, "crash-");
    assert_eq!(paths.len(), 20);
    assert!(window::Report::load(dir.0.clone()).text.contains("SIGABRT"));
}

#[test]
fn app_menu_indicator_and_recovery_open_the_same_report() {
    for lang in [Lang::Ja, Lang::En] {
        let dir = Temp::new();
        Recorder::new(dir.0.clone())
            .record("test", "sample report")
            .unwrap();
        let mut state = crate::state::AppState::new_in(32, 32, lang);
        state.crash = window::Report::load(dir.0.clone());
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(move |cc| {
                crate::YoluApp::for_context(&cc.egui_ctx, state, crate::pen::PenInput::detached())
            });
        h.run();
        h.get_by_label("!").click();
        h.run();
        assert!(h.state().state.crash.open);
        h.get_by_label(lang.pick("閉じる", "Close")).click();
        h.run();
        assert!(!h.state().state.crash.unread);
        h.get_by_label(lang.pick("ヘルプ", "Help")).click();
        h.run();
        h.get_by_label(lang.pick("ログのフォルダを開く", "Open Log Folder"))
            .click();
        h.run();
        assert_eq!(
            h.state_mut().state.crash.request.take(),
            Some(window::Request::Folder)
        );
        h.state_mut().state.recovery.window = Some(Default::default());
        h.run();
        h.get_by_label(lang.pick("クラッシュの報告", "Crash Report"))
            .click();
        h.run();
        assert!(h.state().state.crash.open);
    }
}

#[test]
#[cfg(windows)]
fn windows_exception_writes_native_record() {
    let dir = run_child("native");
    let report = window::Report::load(dir.0.clone());
    assert!(report.unread);
    assert!(report.text.contains("Exception: 0xe0424242"));
    assert!(report.text.contains("Address:"));
    assert!(report.text.contains("Module:"));
    assert!(!report.text.contains("secret.ylp"));
}

#[test]
fn a_panic_that_reaches_main_leaves_no_empty_native_file() {
    let dir = run_child("guarded");
    let paths = files(&dir.0, "crash-");
    assert!(!paths.is_empty());
    assert!(
        paths.iter().all(|p| !is_empty(p)),
        "空の記録先が残っている: {paths:?}"
    );
    let report = window::Report::load(dir.0.clone());
    assert!(report.unread);
    assert!(report.text.contains("guarded panic"));
}

#[test]
fn guard_passes_a_result_through_without_the_panic_notice() {
    let mut told = false;
    assert_eq!(guard(|| 5, || told = true), 5);
    assert!(!told);
}

#[test]
fn caught_panics_are_not_crash_records_and_do_not_push_out_real_ones() {
    let dir = run_child("handled");
    let texts = crash_texts(&dir.0);
    let panics: Vec<_> = texts
        .iter()
        .filter(|t| t.contains("Kind: Rust panic"))
        .collect();
    assert_eq!(panics.len(), 1, "{texts:?}");
    assert!(panics[0].contains("final panic"));
    assert!(!panics[0].contains("handled panic"));
    // 受け止めた panic は普段のログに 1 行ずつ。名前のある行は伏せる
    let session = session_text(&dir.0);
    assert!(session.contains("Handled panic"), "{session}");
    assert!(!session.contains("private"), "{session}");
    // 次の起動に出すのは本物の落ちだけ
    assert!(window::Report::load(dir.0.clone())
        .text
        .contains("final panic"));
}

#[test]
fn identical_panics_in_a_short_time_are_one_record() {
    let dir = run_child("repeat");
    let texts = crash_texts(&dir.0);
    let same = texts.iter().filter(|t| t.contains("same panic")).count();
    assert_eq!(same, 1, "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("final panic")));
}

#[test]
fn handled_restores_the_depth_after_a_panic_and_nests() {
    let depth = || HANDLED.with(Cell::get);
    assert_eq!(depth(), 0);
    let result = handled(|| {
        assert_eq!(depth(), 1);
        assert!(handled(|| panic!("inner")).is_err());
        assert_eq!(depth(), 1);
        panic!("outer");
    });
    assert!(result.is_err());
    assert_eq!(depth(), 0);
    assert_eq!(handled(|| 3).unwrap(), 3);
}

#[test]
fn apply_records_only_action_names_and_failure_reasons_without_names() {
    let dir = run_child("app");
    let report = window::Report::load(dir.0.clone());
    assert!(report.text.contains("app panic"), "{}", report.text);
    assert!(
        report
            .text
            .contains("ZoomIn x40, StartRename, NewLayer, Undo"),
        "{}",
        report.text
    );
    assert!(!report.text.contains("SecretLayerName"), "{}", report.text);
    // 普段のログは、失敗の文の理由だけ（成功の知らせと、前に付いたレイヤーの名前は書かない）
    let session = session_text(&dir.0);
    assert!(session.contains("Layer not found"), "{session}");
    assert!(!session.contains("Saved."), "{session}");
    assert!(!session.contains("SecretLayerName"), "{session}");
}

/// 知らせの口を通った文の診断の記録: 注意と失敗は種類と出どころ、断りは失敗・断りの文として印を付けた文だけ。
/// 済んだ知らせと、印の無い文（名前や理由の分からない文）は書かない。
#[test]
fn notify_records_refusals_marked_as_problems_and_never_names() {
    let dir = run_child("notices");
    let session = session_text(&dir.0);
    let lines: Vec<&str> = session.lines().collect();
    assert_eq!(lines.len(), 3, "{session}");
    assert!(lines[0].ends_with(" Not while drawing."), "{session}");
    assert!(lines[1].ends_with(" Warning bake"), "{session}");
    assert!(
        lines[2].ends_with(" Error save: Layer not found"),
        "{session}"
    );
    for private in ["Secret", "Saved", "baked", "not a layer"] {
        assert!(!session.contains(private), "{private}: {session}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn stack_overflow_is_recorded_and_still_reported_by_the_runtime() {
    let (dir, output) = run_child_output("overflow");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("overflowed its stack"), "{stderr}");
    let report = window::Report::load(dir.0.clone());
    assert!(report.text.contains("SIGSEGV"), "{}", report.text);
}

#[test]
fn ordinary_log_keeps_failures_only_and_never_names_or_paths() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    r.note_problem("Access denied");
    r.note_problem("Cannot read C:\\Users\\SomeoneElse\\private.psd: Access denied");
    r.message("Saved.");
    r.message("Picked #ff00aa");
    r.message("My Private Painting: Access denied");
    r.message("My Private Painting: Access denied");
    r.message("Cannot read C:\\Users\\SomeoneElse\\private.psd: Access denied");
    let text = session_text(&dir.0);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("Access denied"));
    for private in ["Saved", "Picked", "Private", "SomeoneElse", "private.psd"] {
        assert!(!text.contains(private), "{private}: {text}");
    }
}

/// 注意と失敗の知らせは、種類と出どころの名前（言語によらない）と、失敗の文として覚えた部分（名前の付かない理由）だけを書く。
/// 覚えた部分が無い文は、種類と出どころだけ（制作物の名前・パスを書かない決まりのまま）。同じ知らせが続いたら 1 行。
#[test]
fn notices_write_the_kind_the_source_and_only_the_known_reason() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    r.note_problem("Access denied");
    r.notice("Error", "save", "My Private Painting: Access denied");
    r.notice("Error", "save", "My Private Painting: Access denied");
    r.notice("Warning", "bake", "Secret Set: baked 3 maps");
    let text = session_text(&dir.0);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].ends_with(" Error save: Access denied"), "{text}");
    assert!(lines[1].ends_with(" Warning bake"), "{text}");
    for private in ["Private", "Secret", "baked"] {
        assert!(!text.contains(private), "{private}: {text}");
    }
}

#[test]
fn ordinary_log_compares_the_raw_message_first_and_forgets_old_problems() {
    let dir = Temp::new();
    let r = Recorder::new(dir.0.clone());
    r.note_problem("first problem");
    for i in 0..PROBLEM_KEEP {
        r.note_problem(&format!("problem {i}"));
    }
    r.message("first problem");
    assert!(session_text(&dir.0).is_empty());
    r.message("problem 3");
    r.message("problem 3");
    assert_eq!(session_text(&dir.0).lines().count(), 1);
    // 同じ文を知らせ直しても、直前と同じ間は 1 回だけ
    r.note_problem("problem 3");
    r.message("problem 3");
    assert_eq!(session_text(&dir.0).lines().count(), 1);
}

#[test]
fn empty_files_do_not_use_up_the_crash_quota() {
    let dir = Temp::new();
    for i in 0..30 {
        fs::write(
            dir.0.join(format!("crash-record{i:02}.log")),
            "Kind: test\n",
        )
        .unwrap();
    }
    for i in 0..10 {
        fs::write(dir.0.join(format!("crash-empty{i:02}.log")), "").unwrap();
    }
    prune(&dir.0, "crash-", CRASH_KEEP);
    let paths = files(&dir.0, "crash-");
    assert_eq!(paths.iter().filter(|p| !is_empty(p)).count(), CRASH_KEEP);
    // 空のファイルは、動いている別の起動のものかもしれないので prune では消さない
    assert_eq!(paths.iter().filter(|p| is_empty(p)).count(), 10);
}

#[test]
fn stale_empty_native_files_are_swept_but_live_ones_and_records_are_kept() {
    let dir = Temp::new();
    let stale = dir.0.join("crash-00000000000000000001-0000000001.log");
    let live = dir.0.join("crash-00000000000000000002-0000000002.log");
    let record = dir.0.join("crash-00000000000000000003-0000000003.log");
    let own = dir.0.join("crash-00000000000000000004-0000000004.log");
    fs::write(&stale, "").unwrap();
    fs::write(&record, "Kind: test\n").unwrap();
    fs::write(&own, "").unwrap();
    // 動いている別の起動は、記録先のロックを持っている
    let holder = File::create(&live).unwrap();
    holder.try_lock().unwrap();
    native::sweep(&dir.0, &own);
    assert!(!stale.exists());
    assert!(live.exists());
    assert!(record.exists());
    assert!(own.exists());
    // その起動が終われば（ロックが空けば）次の起動が片付ける
    drop(holder);
    native::sweep(&dir.0, &own);
    assert!(!live.exists());
    assert!(record.exists());
}

#[test]
fn the_folder_opens_for_ok_and_for_the_custom_label_only() {
    use rfd::MessageDialogResult as R;
    let folder = "ログのフォルダを開く";
    // Windows の MessageBoxW は文言を捨て、OK かキャンセルで返す。文言を出せる OS はその文言で返す。
    assert!(opens_folder(&R::Ok, folder));
    assert!(opens_folder(&R::Custom(folder.into()), folder));
    assert!(!opens_folder(&R::Custom("閉じる".into()), folder));
    assert!(!opens_folder(&R::Cancel, folder));
    assert!(!opens_folder(&R::Yes, folder));
    assert!(!opens_folder(&R::No, folder));
}

#[test]
fn the_failure_dialog_says_what_ok_does_only_where_labels_are_dropped() {
    for lang in [Lang::Ja, Lang::En] {
        let plain = dialog_text(lang, Some("GPU adapter unavailable"), true);
        assert_eq!(plain, lang.pick("GPU を使えません", "GPU unavailable"));
        let asked = dialog_text(lang, Some("GPU adapter unavailable"), false);
        assert!(asked.starts_with(&plain));
        assert!(asked.ends_with(lang.pick("ログのフォルダを開きますか？", "Open the log folder?")));
        assert_eq!(asked.lines().count(), 2);
    }
}

#[test]
fn the_short_reason_tells_the_gpu_from_the_window_and_the_unknown() {
    for lang in [Lang::Ja, Lang::En] {
        for gpu in [
            "GPU adapter unavailable",
            "No suitable graphics ADAPTER found",
            "failed to create the Gpu device",
        ] {
            assert_eq!(
                short_reason(lang, Some(gpu)),
                lang.pick("GPU を使えません", "GPU unavailable"),
                "{gpu}"
            );
        }
        assert_eq!(
            short_reason(lang, Some("Failed to create the window")),
            lang.pick("ウィンドウを開けません", "Cannot open the window")
        );
        assert_eq!(
            short_reason(lang, None),
            lang.pick("予期しないエラー", "Unexpected error")
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn helper_processes_are_reaped_and_leave_no_zombie() {
    let id = spawn_reaped(&mut std::process::Command::new("true")).unwrap();
    let path = format!("/proc/{id}");
    for _ in 0..300 {
        // 終わりを受けるまでは /proc/<id> が残る（ゾンビ）
        if !Path::new(&path).exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("終わったプロセスが残っている: {path}");
}

#[test]
fn a_panic_the_recovery_writer_survives_is_not_a_crash_record() {
    let dir = run_child("writer");
    let texts = crash_texts(&dir.0);
    let panics: Vec<_> = texts
        .iter()
        .filter(|t| t.contains("Kind: Rust panic"))
        .collect();
    assert_eq!(panics.len(), 1, "{texts:?}");
    assert!(panics[0].contains("final panic"));
    assert!(!panics[0].contains("writer panic"));
    assert!(session_text(&dir.0).contains("writer panic"));
}
