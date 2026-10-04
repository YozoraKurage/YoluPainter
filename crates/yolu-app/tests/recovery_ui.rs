//! 復旧の窓の画面（egui_kittest）: 落ちた次の起動で出る・世代を選んで開く・捨てる（確かめてから）・設定を選ぶ・メニューから開く・
//! 日本語と英語。書き置きの頃合い・失敗・落ちた体の起動の細かい振る舞いは `recovery.rs`（アプリの状態だけ）。
mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::*;
use egui::{epaint::Shape, vec2, Event, Key, Modifiers, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::DVec2;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::recovery::{RecoveryAction, RecoverySettings, Row};
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-recovery-ui-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> PathBuf {
        self.0.join("recovery")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn settings() -> RecoverySettings {
    RecoverySettings {
        interval_seconds: 15,
        strokes_between: 0,
        generations_to_keep: 3,
        directory: None,
    }
}
fn paint(s: &mut AppState, x: f64) {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke.add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO).unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}
fn write_after(s: &mut AppState, from: Instant) -> Instant {
    s.recovery_tick_at(from);
    let due = from + Duration::from_secs(16);
    s.recovery_tick_at(due);
    s.recovery_wait();
    due
}

/// 落ちた実行を作る（世代が `count` 個の置き場を残して、終わらずに捨てる）。
fn crashed_root(dir: &TempDir, count: usize, saved_as: Option<&Path>) {
    let mut s = AppState::new(64, 64);
    s.recovery.enable(dir.root(), settings()).unwrap();
    let mut t = Instant::now();
    for i in 0..count {
        paint(&mut s, 10.0 + 10.0 * i as f64);
        if i == 0 {
            if let Some(path) = saved_as {
                s.apply(Action::SaveProjectAs(path.to_path_buf()));
                paint(&mut s, 5.0);
            }
        }
        t = write_after(&mut s, t + Duration::from_secs(100));
    }
    assert_eq!(s.recovery.checkpoints(), count as u64);
    drop(s);
}

fn app_with(state: AppState) -> Harness<'static, YoluApp> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            YoluApp::for_context(&cc.egui_ctx, state, PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.run();
    h
}
fn started(dir: &TempDir, lang: Lang) -> Harness<'static, YoluApp> {
    started_with(dir, lang, settings())
}
fn started_with(dir: &TempDir, lang: Lang, settings: RecoverySettings) -> Harness<'static, YoluApp> {
    let mut state = AppState::new_in(64, 64, lang);
    state.recovery.enable(dir.root(), settings).unwrap();
    app_with(state)
}

fn window(h: &Harness<'_, YoluApp>) -> Rect {
    yolu_app::windows::window_rect(&h.ctx, "recovery").expect("復旧の窓を描いた")
}
fn rows(h: &Harness<'_, YoluApp>) -> usize {
    h.state().state.recovery.window.as_ref().map_or(0, |w| w.rows.len())
}
fn button_in(h: &Harness<'_, YoluApp>, label: &str, area: Rect) -> Rect {
    rect_of(h, label, |r| area.contains(r.center()))
}
fn is_disabled(h: &Harness<'_, YoluApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// 窓の中に描いた文字（アイコンも含む）。
fn window_texts(h: &Harness<'_, YoluApp>, area: Rect) -> Vec<String> {
    fn collect(shape: &Shape, area: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, area, out)),
            Shape::Text(t) if area.contains(t.pos) => out.push(t.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, area, &mut out);
    }
    out
}
fn has_japanese(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

/// 窓の中だけを撮って、正解の絵と比べる。
fn shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    let rect = window(h);
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn a_crash_opens_the_window_at_the_next_start_with_one_row_per_generation() {
    let dir = TempDir::new("start");
    crashed_root(&dir, 2, None);
    let h = started(&dir, Lang::Ja);
    let area = window(&h);
    assert_eq!(rows(&h), 2);
    assert_eq!(h.get_all_by_label("名称未設定").count(), 2, "名前の列");
    let texts = window_texts(&h, area);
    assert!(texts.iter().any(|t| t == "復旧"), "{texts:?}");
    assert_eq!(texts.iter().filter(|t| t.as_str() == "1 セット").count(), 2, "{texts:?}");
    assert!(texts.iter().filter(|t| t.ends_with("秒前") || t.as_str() == "たった今").count() >= 2, "{texts:?}");
    assert!(!is_disabled(&h, "開く"));
    assert!(!is_disabled(&h, "捨てる"));
    // 新しい行（上）が選ばれている
    assert_eq!(h.state().state.recovery.window.as_ref().unwrap().selected, Some(0));
    // 説明の文は画面に置かない（名前・状態・短い理由だけ）
    assert!(texts.iter().all(|t| t.chars().count() <= 24), "{texts:?}");
}

#[test]
fn opening_the_selected_generation_from_the_window_recovers_it_as_untitled() {
    let dir = TempDir::new("open");
    let original = dir.0.join("作品.ylp");
    crashed_root(&dir, 2, Some(&original));
    let before = std::fs::read(&original).unwrap();
    let mut h = started(&dir, Lang::Ja);
    let area = window(&h);
    // 名前は元の .ylp の名前
    assert_eq!(h.get_all_by_label("作品.ylp").count(), 2);
    // 古い方の行を選んで開く
    let second = h
        .get_all_by_label("作品.ylp")
        .map(|n| n.rect())
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    click(&mut h, second.center());
    assert_eq!(h.state().state.recovery.window.as_ref().unwrap().selected, Some(1));
    let open = button_in(&h, "開く", area).center();
    click(&mut h, open);
    let s = &h.state().state;
    assert_eq!(s.project_name, "名称未設定（復旧）");
    assert!(s.modified);
    assert!(s.recovery.window.is_none(), "開いたら窓を閉じる");
    assert!(s.message.starts_with("復旧しました"), "{}", s.message);
    assert_eq!(std::fs::read(&original).unwrap(), before, "元の .ylp は変えない");
}

#[test]
fn a_double_click_on_a_row_opens_it() {
    let dir = TempDir::new("double");
    crashed_root(&dir, 1, None);
    let mut h = started(&dir, Lang::Ja);
    let at = rect_of(&h, "名称未設定", |r| window(&h).contains(r.center())).center();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    release(&h, at, egui::PointerButton::Primary);
    h.step();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.project_name, "名称未設定（復旧）");
}

#[test]
fn discarding_asks_first_and_the_confirm_is_a_separate_modal_window() {
    let dir = TempDir::new("discard");
    crashed_root(&dir, 2, None);
    let mut h = started(&dir, Lang::Ja);
    let area = window(&h);
    let discard = button_in(&h, "捨てる", area).center();
    click(&mut h, discard);
    assert!(h.state().state.recovery.window.as_ref().unwrap().confirm.is_some());
    let modal = yolu_app::windows::window_rect(&h.ctx, "recovery-discard").expect("確かめの窓");
    assert_eq!(rows(&h), 2, "確かめるまで消さない");
    // やめる
    let cancel = button_in(&h, "やめる", modal).center();
    click(&mut h, cancel);
    assert!(h.state().state.recovery.window.as_ref().unwrap().confirm.is_none());
    assert_eq!(rows(&h), 2);
    // 捨てる
    click(&mut h, discard);
    let modal = yolu_app::windows::window_rect(&h.ctx, "recovery-discard").unwrap();
    let confirm = button_in(&h, "捨てる", modal).center();
    click(&mut h, confirm);
    assert_eq!(rows(&h), 1);
    assert!(h.state().state.recovery.window.as_ref().unwrap().confirm.is_none());
    // 最後の 1 つを捨てると、一覧は空（何も書かない。空の欄の文言を置かない）で、開く・捨てるは押せない
    click(&mut h, discard);
    let modal = yolu_app::windows::window_rect(&h.ctx, "recovery-discard").unwrap();
    let confirm = button_in(&h, "捨てる", modal).center();
    click(&mut h, confirm);
    assert_eq!(rows(&h), 0);
    let texts = window_texts(&h, window(&h));
    assert!(texts.iter().all(|t| !t.contains("世代なし") && !t.contains("なし")), "{texts:?}");
    assert!(is_disabled(&h, "開く") && is_disabled(&h, "捨てる"));
}

#[test]
fn an_empty_window_writes_nothing_where_the_list_would_be_in_either_language() {
    for lang in Lang::ALL {
        let dir = TempDir::new("empty");
        let mut h = started(&dir, lang);
        // 世代が 1 つも無くても、メニューから開ける（間隔と世代の数を選べる）。一覧の場所は空くだけで、文言は出さない
        h.state_mut().state.recovery_apply(RecoveryAction::OpenWindow);
        h.run();
        assert_eq!(rows(&h), 0);
        let texts = window_texts(&h, window(&h));
        let known: Vec<&str> = match lang {
            Lang::Ja => vec!["復旧", "restart_alt", "書き置きの間隔", "残す世代", "開く", "捨てる", "閉じる"],
            Lang::En => vec!["Recovery", "restart_alt", "Checkpoint interval", "Generations kept", "Open", "Discard", "Close"],
        };
        for text in &texts {
            let is_value = text.ends_with('秒') || text.ends_with(" s") || text.ends_with('分') || text.ends_with(" min") || text.parse::<u32>().is_ok();
            assert!(known.contains(&text.as_str()) || is_value, "名前・値のほかに文字が出ている: {text:?} in {texts:?}");
        }
    }
}

#[test]
fn the_window_closes_with_the_button_the_close_icon_and_escape() {
    let dir = TempDir::new("close");
    crashed_root(&dir, 1, None);
    let mut h = started(&dir, Lang::Ja);
    let close = button_in(&h, "閉じる", window(&h)).center();
    click(&mut h, close);
    assert!(h.state().state.recovery.window.is_none());
    h.state_mut().state.recovery_apply(RecoveryAction::OpenWindow);
    h.run();
    assert_eq!(rows(&h), 1, "閉じても世代は残り、開き直せる");
    let x = h.get_by_label("ウィンドウを閉じる").rect().center();
    click(&mut h, x);
    assert!(h.state().state.recovery.window.is_none());
    h.state_mut().state.recovery_apply(RecoveryAction::OpenWindow);
    h.run();
    move_to(&h, window(&h).center());
    h.step();
    h.event(Event::Key {
        key: Key::Escape,
        physical_key: Some(Key::Escape),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert!(h.state().state.recovery.window.is_none());
}

#[test]
fn the_interval_and_the_kept_generations_are_chosen_in_the_window() {
    let dir = TempDir::new("settings");
    crashed_root(&dir, 1, None);
    let mut h = started(&dir, Lang::Ja);
    let file = dir.0.join("recovery.conf");
    h.state_mut().state.recovery.set_settings_path(Some(file.clone()));
    let area = window(&h);
    let at = button_in(&h, "30 秒", area).center();
    click(&mut h, at);
    assert_eq!(h.state().state.recovery.settings().interval_seconds, 30);
    let at = button_in(&h, "10", area).center();
    click(&mut h, at);
    assert_eq!(h.state().state.recovery.settings().generations_to_keep, 10);
    let (saved, _) = RecoverySettings::load(&file).unwrap();
    assert_eq!((saved.interval_seconds, saved.generations_to_keep), (30, 10));
    h.state_mut().state.recovery_apply(RecoveryAction::SetInterval(45));
    h.run();
    assert!(h.query_by_label("45 秒").is_some(), "選択肢に無い値も今の値として見せる");
}

#[test]
fn the_window_opens_from_the_file_menu_and_the_item_needs_a_running_recovery() {
    let dir = TempDir::new("menu");
    let mut h = started(&dir, Lang::Ja);
    assert!(h.state().state.recovery.window.is_none(), "落ちていなければ自動では出ない");
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let item = popup_item(&h, "復旧…").center();
    click(&mut h, item);
    assert!(h.state().state.recovery.window.is_some());
    h.run();
    window(&h);
    // 復旧を動かしていないアプリでは、押せない
    let mut plain = app(1280.0, 800.0, 64);
    let at = menu_title(&plain, "ファイル").center();
    click(&mut plain, at);
    let item = plain.get_by_label("復旧…");
    assert!(item.accesskit_node().is_disabled());
}

#[test]
fn the_window_is_in_english_without_japanese_text() {
    let dir = TempDir::new("english");
    crashed_root(&dir, 1, None);
    let mut h = started(&dir, Lang::En);
    let area = window(&h);
    let texts = window_texts(&h, area);
    assert!(
        texts.iter().all(|t| !has_japanese(t)),
        "英語の画面に日本語が残っている: {texts:?}"
    );
    for expected in ["Recovery", "Untitled", "1 set", "Open", "Discard", "Close", "30 s", "Checkpoint interval", "Generations kept"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert!(h.query_by_label("Untitled").is_some());
    // 開く: 英語の名前・知らせ
    let open = button_in(&h, "Open", area).center();
    click(&mut h, open);
    let s = &h.state().state;
    assert_eq!(s.project_name, "Untitled (Recovered)");
    assert!(s.message.starts_with("Recovered"), "{}", s.message);
    // 確かめの窓も英語
    h.state_mut().state.recovery_apply(RecoveryAction::OpenWindow);
    h.run();
    let area = window(&h);
    let discard = button_in(&h, "Discard", area).center();
    click(&mut h, discard);
    let modal = yolu_app::windows::window_rect(&h.ctx, "recovery-discard").unwrap();
    let texts = window_texts(&h, modal);
    assert!(texts.iter().all(|t| !has_japanese(t)), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Discard Generation"), "{texts:?}");
    let confirm = button_in(&h, "Discard", modal).center();
    click(&mut h, confirm);
    let texts = window_texts(&h, window(&h));
    assert!(texts.iter().all(|t| !t.contains("No generations")), "{texts:?}");
    assert!(texts.iter().all(|t| !has_japanese(t)), "{texts:?}");
}

/// ウィンドウのフォーカスを渡す（eframe では egui-winit が、この値と `Event::WindowFocused` を一緒に渡す）。
fn set_focus(h: &mut Harness<'_, YoluApp>, focused: bool) {
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("ルートのビューポート")
        .focused = Some(focused);
    h.event(Event::WindowFocused(focused));
}

/// フレームを回して、書き込みが終わるまで待つ（時間では書かない設定の試験で、書かれなかったことを確かめる前にも使う）。
fn run_frames(h: &mut Harness<'_, YoluApp>, frames: usize) {
    for _ in 0..frames {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    h.state_mut().state.recovery_wait();
}

/// 書き置きが `count` 個になるまでフレームを回す（期限は 120 秒）。
fn run_until_checkpoints(h: &mut Harness<'_, YoluApp>, count: u64) {
    let began = Instant::now();
    while h.state().state.recovery.checkpoints() < count && began.elapsed() < Duration::from_secs(120) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn slow_settings() -> RecoverySettings {
    RecoverySettings {
        interval_seconds: 600,
        ..settings()
    }
}

#[test]
fn losing_the_window_focus_writes_a_checkpoint_from_the_frame_loop() {
    let dir = TempDir::new("focus-lost");
    let mut h = started_with(&dir, Lang::Ja, slow_settings());
    paint(&mut h.state_mut().state, 10.0);
    // フォーカスを得ても書かない（間隔は 600 秒）
    set_focus(&mut h, true);
    run_frames(&mut h, 20);
    assert_eq!(h.state().state.recovery.checkpoints(), 0, "フォーカスを得た側では書かない");
    // フォーカスを失った次のフレームで、時間を待たずに書く
    set_focus(&mut h, false);
    run_until_checkpoints(&mut h, 1);
    let st = &h.state().state;
    assert_eq!(st.recovery.checkpoints(), 1, "{}", st.message);
    assert!(st.recovery.is_marked_dirty());
    // 戻って描き、また外れる: もう 1 つ書く
    set_focus(&mut h, true);
    run_frames(&mut h, 5);
    paint(&mut h.state_mut().state, 40.0);
    run_frames(&mut h, 20);
    assert_eq!(h.state().state.recovery.checkpoints(), 1, "戻って描いても、外れるまでは書かない");
    set_focus(&mut h, false);
    run_until_checkpoints(&mut h, 2);
    assert_eq!(h.state().state.recovery.checkpoints(), 2);
}

#[test]
fn a_window_that_never_had_the_focus_or_only_gained_it_does_not_write() {
    let dir = TempDir::new("focus-never");
    let mut h = started_with(&dir, Lang::Ja, slow_settings());
    paint(&mut h.state_mut().state, 10.0);
    // 最初から外れている（得たことが無い）: 失った瞬間ではない
    set_focus(&mut h, false);
    run_frames(&mut h, 30);
    assert_eq!(h.state().state.recovery.checkpoints(), 0);
    // 外れたまま → 得る: 書かない
    set_focus(&mut h, true);
    run_frames(&mut h, 30);
    assert_eq!(h.state().state.recovery.checkpoints(), 0);
    assert!(h.state().state.recovery.is_idle());
}

#[test]
fn failed_writes_are_shown_in_the_status_band_and_painting_goes_on() {
    let dir = TempDir::new("frames");
    let mut h = started(&dir, Lang::Ja);
    paint(&mut h.state_mut().state, 10.0);
    h.state_mut().state.recovery_request_flush();
    run_until_checkpoints(&mut h, 1);
    let st = &h.state().state;
    assert_eq!(st.recovery.checkpoints(), 1, "{} enabled={} idle={} modified={} stroking={}", st.message, st.recovery.is_enabled(), st.recovery.is_idle(), st.modified, st.is_stroking());
    // 失敗は状態の帯に、短い理由で出る（描くのは止まらない）
    h.state_mut().state.recovery.set_fault(Some(std::sync::Arc::new(|stage: &str| {
        if stage == "before-pointer" {
            return Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
        }
        Ok(())
    })));
    paint(&mut h.state_mut().state, 40.0);
    h.state_mut().state.recovery_request_flush();
    let began = Instant::now();
    while !h.state().state.message.starts_with("復旧用の書き置きに失敗") && began.elapsed() < Duration::from_secs(120) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        h.state().state.message,
        "復旧用の書き置きに失敗: ディスクの空きがありません"
    );
}

#[test]
fn an_unreadable_recovery_setting_is_reported_in_the_status_band_when_the_app_starts() {
    for (lang, expected) in [
        (Lang::Ja, "復旧の設定を読めません: ファイルのデータが不正です。世代は整理しません"),
        (Lang::En, "Cannot read the recovery settings: Invalid file data. Generations are not trimmed"),
    ] {
        let dir = TempDir::new("conf-band");
        let conf = dir.0.join("recovery.conf");
        std::fs::write(&conf, vec![b'a'; 5000]).unwrap();
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_pixels_per_point(1.0)
            .with_render_options(render_options())
            .wgpu()
            .build_eframe(move |cc| {
                let mut app = YoluApp::for_context(&cc.egui_ctx, AppState::new_in(64, 64, lang), PenInput::detached())
                    .with_render_state(cc.wgpu_render_state.as_ref());
                app.start_recovery_with(Some(conf));
                app
            });
        h.run();
        let st = &h.state().state;
        assert_eq!(st.message, expected);
        assert!(st.recovery.is_enabled(), "読めない設定でも、既定で復旧は動く");
        assert_eq!(std::fs::read(dir.0.join("recovery.conf")).unwrap(), vec![b'a'; 5000], "設定のファイルは書き換えない");
    }
}

#[test]
fn closing_the_app_cleanly_writes_the_last_changes_and_removes_the_mark() {
    use eframe::App;
    let dir = TempDir::new("exit");
    let mut h = started(&dir, Lang::Ja);
    paint(&mut h.state_mut().state, 10.0);
    h.state_mut().on_exit();
    // 次の起動は落ちたとは見ず、世代は残っている
    let again = started(&dir, Lang::Ja);
    assert!(again.state().state.recovery.window.is_none());
    let mut again = again;
    again.state_mut().state.recovery_apply(RecoveryAction::OpenWindow);
    again.run();
    assert_eq!(rows(&again), 1);
}

fn row(name: &str, ago_ms: u64, documents: usize, problem: Option<&str>) -> Row {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    Row {
        pool: PathBuf::from("/recovery/pool"),
        id: format!("{ago_ms}"),
        time_ms: Some(now - ago_ms),
        documents,
        name: name.into(),
        crashed: true,
        own: false,
        problem: problem.map(str::to_owned),
    }
}

#[test]
fn the_window_looks_the_same_in_both_languages() {
    for lang in Lang::ALL {
        let mut state = AppState::new_in(64, 64, lang);
        let mut window = yolu_app::recovery::window::WindowState::default();
        window.set_rows(vec![
            row("作品.ylp", 150_000, 2, None),
            row("", 4_000_000, 1, None),
            row("壊れた作品.ylp", 90_000_000, 3, Some("世代の長さが合いません")),
        ]);
        state.recovery.window = Some(window);
        let mut h = app_with(state);
        shot(&mut h, &format!("recovery_window_{}", lang.pick("ja", "en")));
    }
}
