//! 保存の途中でウィンドウを閉じる・終了するときは、閉じるのを待たせて、保存が終わってから（成功でも失敗でも結果を受けてから）閉じる。
//! 保存していない変更の確認は、保存の後の状態で聞く。画面のスレッドは回し続ける（待つあいだも 1 フレームずつ返る）。
//! 保存の仕事そのものは `headless/save_background.rs`。
use crate::common;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::*;
use egui::{vec2, ViewportCommand, ViewportEvent, ViewportId};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::DVec2;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-saveclose-{tag}-{}-{}",
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

type H = Harness<'static, YoluApp>;

/// 裏のスレッドで保存するウィンドウ（描いて、保存していない印を付けてある）。
fn window() -> H {
    window_in(Lang::Ja)
}
fn window_in(lang: Lang) -> H {
    let mut h = gpu_thread::builder()
        .with_size(vec2(900.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new_in(128, 128, lang),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(900.0);
    h.run();
    let s = &mut h.state_mut().state;
    s.save.background = true;
    paint(s, 20.0);
    h
}
fn paint(s: &mut AppState, x: f64) {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}
/// 直前のフレームがウィンドウの外へ出した「閉じる」の頼み（`Close`・`CancelClose`）。
fn close_commands(h: &H) -> Vec<&'static str> {
    h.output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| {
            v.commands
                .iter()
                .filter_map(|c| match c {
                    ViewportCommand::Close => Some("Close"),
                    ViewportCommand::CancelClose => Some("CancelClose"),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}
/// ウィンドウの ×（OS が出す閉じる頼み）を、次のフレームへ入れる。
fn press_window_close(h: &mut H) {
    h.input_mut()
        .viewports
        .get_mut(&ViewportId::ROOT)
        .expect("根のウィンドウ")
        .events
        .push(ViewportEvent::Close);
}
/// 保存の仕事が終わって、その結果を受けたフレームまで 1 フレームずつ進める（返すのはその間の「閉じる」の頼み。画面のスレッドは
/// 待つあいだも 1 フレームずつ返る）。
fn step_until_saved(h: &mut H) -> Vec<&'static str> {
    let start = Instant::now();
    let mut all = Vec::new();
    while h.state().state.is_saving() {
        h.step();
        all.extend(close_commands(h));
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "保存が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    // 終わったフレームのあとの 1 フレームで、結果の後の状態から閉じる流れが進む
    h.step();
    all.extend(close_commands(h));
    all
}
fn counter() -> (Rc<Cell<u32>>, Rc<Cell<bool>>) {
    (Rc::new(Cell::new(0)), Rc::new(Cell::new(true)))
}

#[test]
fn quitting_during_a_save_waits_for_the_save_and_then_closes() {
    let dir = TempDir::new("quit");
    let path = dir.file("作品.ylp");
    let mut h = window();
    let (asked, _) = counter();
    let seen = asked.clone();
    h.state_mut().answer_close_question(move |_| {
        seen.set(seen.get() + 1);
        true
    });
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(h.state().state.is_saving());
    // 終了を頼む。保存の間は、閉じない・聞かない・小さなウィンドウを出す。フレームは返り続ける
    h.state_mut().state.apply(Action::Quit);
    for _ in 0..5 {
        h.step();
        assert!(
            close_commands(&h).is_empty(),
            "保存の途中で閉じる頼みを出さない: {:?}",
            close_commands(&h)
        );
    }
    assert!(!h.state().is_closing());
    assert_eq!(asked.get(), 0, "保存の途中には聞かない");
    assert!(h.state().state.quit, "終了の頼みは覚えている");
    assert!(
        yolu_app::windows::saving_window_rect(&h.ctx).is_some(),
        "「保存しています」の小さなウィンドウ"
    );
    hold.release();
    let commands = step_until_saved(&mut h);
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    assert!(path.exists(), "保存は捨てずに終わらせてから閉じる");
    assert!(h.state().is_closing());
    assert!(
        commands.contains(&"Close"),
        "保存が終わってから閉じる: {commands:?}"
    );
    assert_eq!(asked.get(), 0, "保存が済んで変更が無ければ聞かない");
}

#[test]
fn the_window_close_button_during_a_save_is_held_back_and_closes_after_the_save() {
    let dir = TempDir::new("x");
    let path = dir.file("作品.ylp");
    let mut h = window();
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    press_window_close(&mut h);
    h.step();
    assert_eq!(
        close_commands(&h),
        vec!["CancelClose"],
        "ウィンドウの閉じるは止める"
    );
    assert!(
        h.state().state.quit && !h.state().is_closing(),
        "止めた頼みは覚えている"
    );
    for _ in 0..3 {
        h.step();
        assert!(close_commands(&h).is_empty());
    }
    hold.release();
    let commands = step_until_saved(&mut h);
    assert!(path.exists());
    assert!(h.state().is_closing());
    assert!(commands.contains(&"Close"), "{commands:?}");
}

/// ウィンドウが隠れている間（最小化など）は、eframe は egui のパスを回さず `logic` だけを呼ぶ。そこでも保存の途中の閉じる頼みは止めて待ち、
/// 保存が終わって終了の頼みが残っていれば閉じる。
#[test]
fn a_hidden_window_also_holds_back_the_close_during_a_save() {
    let dir = TempDir::new("hidden");
    let path = dir.file("作品.ylp");
    let mut h = window();
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    h.input_mut()
        .viewports
        .get_mut(&ViewportId::ROOT)
        .expect("根のウィンドウ")
        .minimized = Some(true);
    press_window_close(&mut h);
    h.step();
    assert!(
        !close_commands(&h).is_empty() && close_commands(&h).iter().all(|c| *c == "CancelClose"),
        "{:?}",
        close_commands(&h)
    );
    assert!(h.state().state.quit && !h.state().is_closing());
    hold.release();
    let commands = step_until_saved(&mut h);
    assert!(path.exists());
    assert!(h.state().is_closing());
    assert!(commands.contains(&"Close"), "{commands:?}");
}

/// 隠れたウィンドウの 1 回（実際のウィンドウが最小化されたときに eframe が `ui` の代わりに呼ぶ `logic` の中身）だけを回す。kittest は `logic` の後に必ず
/// `ui` も回すので、`ui` が回らない隠れたウィンドウの道は、これで通す。
fn hidden_tick(h: &mut H) {
    let ctx = h.ctx.clone();
    h.state_mut().tick_hidden(&ctx);
}

/// OS の終了を待たせる印（Windows の「保存しています」の理由）は、保存の有無に合う。ウィンドウが隠れていて `ui` が回らなくても、保存を頼んだ後に
/// 最小化すれば印が付き、最小化したまま保存が終われば印が消える（付いたままだと、ウィンドウを戻すまで OS の終了が止まる。付かないと、保存中でも
/// OS の終了を待たせない）。
#[test]
fn the_shutdown_mark_follows_a_save_that_starts_while_the_window_gets_hidden() {
    let dir = TempDir::new("markstart");
    let mut h = window();
    assert!(!h.state().saving_marked());
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(dir.file("作品.ylp")));
    // 次の `ui` のフレームが来る前に最小化した: 隠れたウィンドウの 1 回だけが回る
    hidden_tick(&mut h);
    assert!(h.state().state.is_saving());
    assert!(
        h.state().saving_marked(),
        "隠れたウィンドウでも、保存の間は OS の終了を待たせる印を付ける"
    );
    hold.release();
    let start = Instant::now();
    while h.state().state.is_saving() {
        hidden_tick(&mut h);
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "保存が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!h.state().saving_marked());
}

#[test]
fn the_shutdown_mark_goes_when_a_save_ends_while_the_window_is_hidden() {
    let dir = TempDir::new("markend");
    let mut h = window();
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(dir.file("作品.ylp")));
    h.step();
    assert!(h.state().saving_marked(), "見えている間の `ui` が付ける");
    hidden_tick(&mut h);
    assert!(h.state().saving_marked() && h.state().state.is_saving());
    hold.release();
    let start = Instant::now();
    while h.state().state.is_saving() {
        hidden_tick(&mut h);
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "保存が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    assert!(
        !h.state().saving_marked(),
        "隠れたまま保存が終わったら、印も消える"
    );
}

#[test]
fn what_is_drawn_during_the_save_is_asked_about_after_it() {
    let dir = TempDir::new("drawn");
    let mut h = window();
    let (asked, answer) = counter();
    let (seen, reply) = (asked.clone(), answer.clone());
    h.state_mut().answer_close_question(move |s: &AppState| {
        assert!(s.modified, "問われるのは、保存の後も変更が残っているとき");
        seen.set(seen.get() + 1);
        reply.get()
    });
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(dir.file("作品.ylp")));
    h.state_mut().state.apply(Action::Quit);
    h.step();
    // 保存の間に描く（保存に入らない分は、保存の後も「変更あり」）
    paint(&mut h.state_mut().state, 60.0);
    hold.release();
    answer.set(false);
    let commands = step_until_saved(&mut h);
    assert_eq!(asked.get(), 1, "保存が終わってから、1 度だけ聞く");
    assert!(
        !h.state().is_closing() && !h.state().state.quit,
        "続けると答えたので、終わらない"
    );
    assert!(!commands.contains(&"Close"), "{commands:?}");
    assert!(h.state().state.modified);
    // もう 1 度終了を頼んで、捨てて終わると答える
    answer.set(true);
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(asked.get(), 2);
    assert!(h.state().is_closing());
    assert!(close_commands(&h).contains(&"Close"));
}

/// 2 つ目のセットを足して描き、全タイルをディスクへ逃がして読めなくし、読めないセットの印を付ける（まだ 1 度も保存していないセット）。
fn add_never_saved_unreadable_set(h: &mut H, cache: &std::path::Path) {
    use yolu_app::engine::Channel;
    use yolu_app::newproject::NpAction;
    use yolu_core::tile_cache::{self, CacheSettings};
    let s = &mut h.state_mut().state;
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 2, "{}", s.message);
    s.switch_set(1).unwrap();
    s.ensure_selection();
    paint(s, 40.0);
    s.switch_set(0).unwrap();
    tile_cache::configure(&CacheSettings {
        enabled: false,
        folder: Some(cache.to_path_buf()),
        memory_limit: u64::MAX,
        disk_limit: 1 << 40,
    });
    tile_cache::evict_now(0);
    for l in s.set_doc(1).layers() {
        if let Some(surface) = l.surface(Channel::Color) {
            surface.fail_tile_reads_for_test();
        }
    }
    assert!(s.set_doc(1).composite(s.set_doc(1).bounds()).is_err());
    s.check_tile_cache();
    assert!(s.sets.get(1).unwrap().read_only.is_some(), "{}", s.message);
}

/// 読めないため保存に入れなかったセットは、画面にあってファイルに無い。保存が済んだあとも「保存していない変更」のままで、閉じるときに聞く
/// （聞かずに閉じると、そのセットは何の確認もなく消える）。
#[test]
fn a_set_left_out_of_the_save_is_still_asked_about_when_closing() {
    let dir = TempDir::new("left-out");
    let cache = TempDir::new("left-out-cache");
    let mut h = window();
    add_never_saved_unreadable_set(&mut h, &cache.0);
    let (asked, answer) = counter();
    let (seen, reply) = (asked.clone(), answer.clone());
    h.state_mut().answer_close_question(move |s: &AppState| {
        assert!(s.modified, "入れなかったセットがあるので、変更は残る");
        seen.set(seen.get() + 1);
        reply.get()
    });
    let path = dir.file("作品.ylp");
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    h.state_mut().state.apply(Action::Quit);
    h.step();
    hold.release();
    answer.set(false);
    let commands = step_until_saved(&mut h);
    assert!(path.exists());
    assert_eq!(asked.get(), 1, "保存が終わってから、1 度だけ聞く");
    assert!(
        !h.state().is_closing() && !h.state().state.quit,
        "続けると答えたので、終わらない"
    );
    assert!(!commands.contains(&"Close"), "{commands:?}");
    assert!(
        h.state().state.shows_modified(),
        "開き直す・捨てるときの問い（confirm_discard）も、この印で聞く"
    );
    // もう 1 度保存しても、また入れずに印は残る。捨てて終わると答えれば閉じる
    h.state_mut().state.apply(Action::SaveProject);
    h.state_mut().state.wait_save();
    assert!(h.state().state.modified);
    answer.set(true);
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(asked.get(), 2);
    assert!(h.state().is_closing());
    assert!(close_commands(&h).contains(&"Close"));
}

#[test]
fn a_failed_save_gives_its_reason_and_goes_back_to_the_question() {
    let dir = TempDir::new("failed");
    // フォルダーを作れない場所（通常のファイルの下）へ保存する
    std::fs::write(dir.file("ファイル"), b"x").unwrap();
    let target = dir.file("ファイル").join("下").join("作品.ylp");
    let mut h = window();
    let (asked, answer) = counter();
    let (seen, reply) = (asked.clone(), answer.clone());
    h.state_mut().answer_close_question(move |s: &AppState| {
        assert!(s.modified, "失敗した保存は、保存の前の「変更あり」を戻す");
        if seen.get() == 0 {
            assert!(
                s.message.contains("保存できません"),
                "理由を出してから聞く: {}",
                s.message
            );
        }
        seen.set(seen.get() + 1);
        reply.get()
    });
    let hold = h.state_mut().state.save.hold_next();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(target.clone()));
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert!(!h.state().is_closing());
    hold.release();
    answer.set(false);
    let commands = step_until_saved(&mut h);
    assert_eq!(asked.get(), 1);
    assert!(!h.state().is_closing() && !h.state().state.quit);
    assert!(!commands.contains(&"Close"), "{commands:?}");
    assert!(!target.exists());
    // 捨てて終わると答えれば閉じる
    answer.set(true);
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert!(h.state().is_closing());
}

#[test]
fn closing_without_a_save_is_as_before() {
    let mut h = window();
    let (asked, answer) = counter();
    let (seen, reply) = (asked.clone(), answer.clone());
    h.state_mut().answer_close_question(move |_| {
        seen.set(seen.get() + 1);
        reply.get()
    });
    h.state_mut().state.apply(Action::Quit);
    h.step();
    assert_eq!(
        asked.get(),
        1,
        "保存していない変更があれば、今までどおり聞く"
    );
    assert!(h.state().is_closing());
    assert!(close_commands(&h).contains(&"Close"));
    assert!(
        yolu_app::windows::saving_window_rect(&h.ctx).is_none(),
        "保存していないので、待つウィンドウは出さない"
    );
}

/// 終わる前の後始末（走っている仕事を止めて、少し待つ）は、保存を取り消さず、待たない（保存は閉じる流れが待つ）。
#[test]
fn stopping_jobs_before_quitting_neither_cancels_nor_waits_for_a_save() {
    let dir = TempDir::new("stop");
    let path = dir.file("作品.ylp");
    let mut state = AppState::new(128, 128);
    state.save.background = true;
    paint(&mut state, 20.0);
    let hold = state.save.hold_next();
    state.apply(Action::SaveProjectAs(path.clone()));
    assert!(state.is_saving());
    let t = Instant::now();
    yolu_app::windows::stop_jobs(&mut state, Duration::from_secs(3));
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "保存を待たない: {:?}",
        t.elapsed()
    );
    assert!(state.is_saving(), "保存は取り消さない");
    hold.release();
    state.wait_save();
    assert!(
        state.message.starts_with("保存しました"),
        "{}",
        state.message
    );
    assert!(path.exists());
}

// ───────── 画面（進み具合の札・待つウィンドウ） ─────────

/// 保存の間のウィンドウの全体（右下の札に「保存しています」と進み。取消のボタンは無い。メニューバーの右の名前に、保存の結果が出るまで「•」）。
#[test]
fn the_job_card_shows_saving_with_the_name_and_no_cancel_button() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in [Lang::Ja, Lang::En] {
        let dir = TempDir::new("card");
        let mut h = window_in(lang);
        let hold = h.state_mut().state.save.hold_next();
        h.state_mut()
            .state
            .apply(Action::SaveProjectAs(dir.file("work.ylp")));
        h.event(egui::Event::PointerGone);
        h.step();
        h.step();
        assert!(
            h.state().state.shows_modified(),
            "保存の結果が出るまで、保存していない印を見せる"
        );
        let cancels: Vec<_> = h
            .query_all_by_label_contains(lang.pick("取消", "Cancel"))
            .collect();
        assert!(
            cancels.is_empty(),
            "保存は取り消せない（取消のボタンが無い）"
        );
        h.snapshot(format!("save_card_{}", lang.pick("ja", "en")));
        snapshots.extend_harness(&mut h);
        hold.release();
        h.state_mut().state.wait_save();
        h.step();
        assert!(h.state().state.save_progress().is_none());
    }
}

/// 終わる頼みを待たせている小さなウィンドウ（取り消しのボタンは無い）。
#[test]
fn the_waiting_window_before_closing_has_no_cancel_button() {
    for lang in [Lang::Ja, Lang::En] {
        let dir = TempDir::new("wait");
        let mut h = window_in(lang);
        let hold = h.state_mut().state.save.hold_next();
        h.state_mut()
            .state
            .apply(Action::SaveProjectAs(dir.file("work.ylp")));
        h.state_mut().state.apply(Action::Quit);
        h.event(egui::Event::PointerGone);
        h.step();
        h.step();
        let rect = yolu_app::windows::saving_window_rect(&h.ctx).expect("待つウィンドウを描いた");
        assert!(
            rect.width() < 400.0 && rect.height() < 120.0,
            "小さなウィンドウ: {rect:?}"
        );
        let cancels: Vec<_> = h
            .query_all_by_label_contains(lang.pick("取消", "Cancel"))
            .collect();
        assert!(cancels.is_empty());
        let image = h.render().expect("描画");
        let cropped = image::imageops::crop_imm(
            &image,
            rect.left().floor() as u32,
            rect.top().floor() as u32,
            rect.width().ceil() as u32,
            rect.height().ceil() as u32,
        )
        .to_image();
        egui_kittest::image_snapshot(&cropped, format!("save_closing_{}", lang.pick("ja", "en")));
        hold.release();
        h.state_mut().state.wait_save();
        h.step();
        h.step();
        assert!(h.state().is_closing());
    }
}
