//! 主の GPU（wgpu のデバイス）を失ったときの流れ（受け口は `gpu_watch`）。
//!
//! 失ったら、描いていた絵（core の文書）には触らずに、次の順で進め、**プロセスをここで終える**（`EXIT_CODE`）:
//! 描いている最中のストロークの確定 → 保存の途中なら終わるまで待つ（`SAVE_WAIT`）→ 復旧の書き置き → GPU の道（キャンバスの GPU の表示・
//! 3D ビュー・メモリの計測）を手放す → 走っている仕事に取り消しを頼む → 理由を落ちた記録の枠と普段のログへ → OS のウィンドウで知らせる
//! （実際のウィンドウだけ。閉じるまで待つ）→ 仕事が止まるのを待つ → `on_exit`（書き置きの書き込み中の分を待つ）→ Live Link の印と
//! クリップボードの口を片づける → 普段のログ → 終了。
//! フレームには戻らない: eframe は `ui` が返ったあとに、失ったデバイスでそのフレームを描く（egui-wgpu の中で、頂点・インデックスの
//! 書き込み先を取れずに panic する）ので、閉じる頼み（`ViewportCommand::Close`）を出して戻る作りでは、閉じる前の 1 フレームで落ちる。
//! 別ウィンドウ（`show_viewport_immediate`）はメインウィンドウの `ui` の途中でその場で描かれるので、見張りを読む場所は `FramePoint` の
//! とおり、別ウィンドウの前後にもある。
//! 終わるときは、書き置きの「保存していない作業」の印を消さない（落ちたときと同じ）ので、次の起動の復旧のウィンドウから続きを開ける。
use std::time::{Duration, Instant};

use eframe::egui_wgpu::RenderState;

use super::YoluApp;
use crate::gpu_watch::{self, GpuWatch, Lost, Saved};
use crate::notice::Source;

/// 書き置きを急いで取るときと、終わるときに書き込み中の分を待つとき、待つ長さ（遅いディスクで、終わる前に固まらないように）。
pub(super) const RECOVERY_WAIT: Duration = Duration::from_secs(10);
/// 保存の途中で失ったとき、保存が終わるのを待つ長さの上限。保存は利用者がはっきり頼んだもので GPU を使わないので、書き置きより長く取る
/// （待つ間、ウィンドウは描き換わらない）。間に合わなければ、保存を待たずに終える（置換は最後の 1 回なので、前の版が残る）。
pub(super) const SAVE_WAIT: Duration = Duration::from_secs(60);
/// 終わる前に、走っている仕事が止まるのを待つ長さ（ふつうに閉じるときと同じ）。
const JOBS_WAIT: Duration = Duration::from_secs(3);
/// GPU でエラーが起きて終わるときのプロセスの終了コード（BSD の sysexits の `EX_SOFTWARE` と同じ番号。0 でも、落ちたときの OS の
/// 例外の番号でもない、決まった値にして、起動した側が見分けられるようにする）。
pub const EXIT_CODE: i32 = 70;

/// プロセスの終え方（終了コードを受けて、プロセスを終える。戻らない。試験が、戻らない代わりに巻き戻しで抜ける物へ替える）。
pub(super) type Exit = Box<dyn FnMut(i32)>;

/// フレームの中で、見張りを読む場所。試験が `set_frame_probe` で、失った知らせを「ここで」入れて、その直後の読みで戻らずに終わることを確かめる。
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePoint {
    /// 別ウィンドウを描き始める前（メインウィンドウのドックを描いたあと）。
    BeforeDetached,
    /// 別ウィンドウ 1 つ（持ち主の番号）の `ui` の終わり。eframe がそのウィンドウを描く前。
    DetachedPassEnd(u64),
    /// 別ウィンドウ 1 つ（持ち主の番号）を描き終えたあと。次のウィンドウを描く前。
    AfterDetached(u64),
    /// メインウィンドウの `ui` の終わり。eframe がそのフレームを描く前。
    UiEnd,
}

impl YoluApp {
    /// 主の GPU の見張りを付ける（実際のウィンドウと、GPU を失ったときの流れを確かめる試験）。付けると、受け手の無い誤りも panic にせず記録する。
    pub fn watch_gpu(&mut self, rs: &RenderState, ctx: &egui::Context) {
        let info = rs.adapter.get_info();
        let watch = GpuWatch::attach(&rs.device, ctx);
        watch.set_adapter(format!("{} ({:?})", info.name, info.backend));
        self.gpu_watch = Some(watch);
    }

    /// 試験用: 本物の GPU を使わずに、見張りを付ける（`GpuWatch::inject_loss` で失った知らせを入れて、受ける側の流れを通す）。
    #[doc(hidden)]
    pub fn set_gpu_watch(&mut self, watch: GpuWatch) {
        self.gpu_watch = Some(watch);
    }

    /// 主の GPU の見張り（試験が、失った知らせを入れる）。
    #[doc(hidden)]
    pub fn gpu_watch(&self) -> Option<&GpuWatch> {
        self.gpu_watch.as_ref()
    }

    /// 試験用: GPU を失ったとき、書き置きの書き込みを待つ長さの上限（既定は 10 秒）。止めた書き手で、期限が働くことを短い時間で確かめる。
    #[doc(hidden)]
    pub fn set_gpu_lost_wait(&mut self, wait: Duration) {
        self.gpu_lost_wait = wait;
    }

    /// 試験用: GPU を失ったとき、保存の途中の分を待つ長さの上限（既定は 60 秒）。
    #[doc(hidden)]
    pub fn set_gpu_lost_save_wait(&mut self, wait: Duration) {
        self.gpu_lost_save_wait = wait;
    }

    /// 試験用: 失って終わるときのプロセスの終え方を替える（既定は `std::process::exit`）。本物は戻らないので、替える物も戻らない（巻き戻しで抜ける）
    /// 形にすると、「終えたあとの処理（フレームの続き・描画）は走らない」ことまで確かめられる。受け取るのは終了コード。
    #[doc(hidden)]
    pub fn set_gpu_lost_exit(&mut self, exit: impl FnMut(i32) + 'static) {
        self.gpu_lost_exit = Some(Box::new(exit));
    }

    /// 試験用: フレームの中の、見張りを読む場所（`FramePoint`）に着いたときに呼ばれる物を付ける（読む直前に呼ぶ）。
    #[doc(hidden)]
    pub fn set_frame_probe(&mut self, probe: impl FnMut(FramePoint) + 'static) {
        self.frame_probe = Some(Box::new(probe));
    }

    /// GPU を失って終わろうとしているとき、その様子（終えたあとは戻らないので、読めるのは試験だけ）。
    pub fn gpu_lost(&self) -> Option<&Lost> {
        self.gpu_lost.as_ref()
    }

    /// フレームの中の `point` で見張りを読む（失った知らせがあれば、戻らずにプロセスを終える）。
    pub(super) fn watch_point(&mut self, ctx: &egui::Context, point: FramePoint) {
        if let Some(probe) = self.frame_probe.as_mut() {
            probe(point);
        }
        self.poll_gpu_watch(ctx);
    }

    /// 見張りの知らせを受ける（フレームの初め。ウィンドウが隠れている間も）。失った知らせがあれば、戻らずにプロセスを終える。
    pub(super) fn poll_gpu_watch(&mut self, _ctx: &egui::Context) {
        let Some(watch) = &self.gpu_watch else {
            return;
        };
        let events = watch.take();
        for error in &events.errors {
            // 受け手の無い誤り（検証・メモリ不足）は、落とさず普段のログへ
            crate::crash::note(&format!("GPU error: {error}"));
        }
        if let Some(lost) = events.lost {
            self.handle_gpu_lost(lost);
        }
    }

    fn handle_gpu_lost(&mut self, lost: Lost) {
        // 描いている最中のストロークは、描いた所までで確定する（書き置きに入り、取り残さない）
        if self.state.is_stroking() {
            crate::canvas::finish_stroke(&mut self.state, false);
        }
        // 保存の途中なら、先に終わるまで待つ。保存の頼みは「変更あり」の印を下ろし、保存の間は書き置きの見張りも止まる（保存は開いた .ylp を
        // 置き換えるので、書き置きの読みと重ねない）ので、待たずに書き置きの要不要を決めると、保存の間に描いた分・失敗して戻った変更を取りこぼす
        let save_late = self.wait_saving_within(self.gpu_lost_save_wait);
        // 変更があれば、復旧の書き置きを今すぐ取る（文書そのものは触らない）。保存が期限に間に合わなかったときは、書き置きを取れない
        let saved = if save_late {
            Saved::No
        } else if !self.state.modified {
            Saved::NothingToSave
        } else if self.state.recovery.is_enabled()
            && self.state.recovery_flush_within(self.gpu_lost_wait)
        {
            Saved::Yes
        } else {
            Saved::No
        };
        // 走っている仕事は、利用者が OS のウィンドウを閉じるのを待たずに、取り消しを頼んでおく（止まるのを待つのは閉じたあと）
        crate::jobs::request_stop(&mut self.state);
        // GPU の道を手放す（失ったデバイスの資源の後始末はドライバー任せ。キャンバスは CPU の表示へ落ちる）
        self.display.attach_render_state(None);
        self.renderer3d = None;
        self.gpu_device = None;
        let lang = self.state.lang;
        // 画面の帯と普段のログへ（`fail` が知らせとして書く）。続けて落ちた記録の枠へ、ドライバーの文つきで書く
        // （次の起動で報告の印が出る。ウィンドウや終了の途中で止められても、先に書いておく）
        self.state
            .fail(Source::Display, gpu_watch::lost_text(lang, saved));
        let adapter = self
            .gpu_watch
            .as_ref()
            .map(GpuWatch::adapter)
            .unwrap_or_default();
        let mut detail = gpu_watch::lost_detail(&adapter, &lost, saved);
        if save_late {
            detail.push_str("\nSave: still running at the deadline");
            crate::crash::note("GPU lost: a save was still running at the deadline");
        }
        crate::crash::event("GPU device lost", &detail);
        self.gpu_lost = Some(lost);
        if self.dialogs {
            // ウィンドウの描画は続けられないので、理由は OS のウィンドウで出す。閉じるまでここで待つ
            crate::dialog::message()
                .set_title("YoluPainter")
                .set_description(gpu_watch::lost_dialog_text(lang, saved))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        self.exit_after_gpu_lost(saved);
    }

    /// 失った GPU では 1 度も描かずに、後始末をしてプロセスを終える。戻るのは、終え方を替えた試験だけ。
    ///
    /// ふつうに閉じるときの後始末のうち、GPU に触らないものを、期限つきでここで行う: 走っている仕事が止まるのを待つ・書き置きの書き込み中の
    /// 分を待つ（印は消さない。保存が間に合わなかったときも）・Live Link の起きている印を消す・クリップボードの口を手放す
    /// （X11 は、書いた画像がクリップボードの管理に渡るのを少し待つ）。並び（`persist_layout` が書いた物）は、別ウィンドウを描いている
    /// 途中で終えることがあり、いまの状態から書き直すと別ウィンドウが抜けるので、`on_exit` でも書き直さない。
    fn exit_after_gpu_lost(&mut self, saved: Saved) {
        crate::jobs::wait_stopped(&mut self.state, JOBS_WAIT);
        eframe::App::on_exit(self);
        self.link.shutdown();
        self.state.clip.release_os();
        crate::crash::note(&format!(
            "GPU lost: exiting with code {EXIT_CODE} (saved for recovery: {saved:?})"
        ));
        crate::crash::cleanup();
        match self.gpu_lost_exit.as_mut() {
            Some(exit) => exit(EXIT_CODE),
            None => std::process::exit(EXIT_CODE),
        }
    }

    /// 保存の途中なら、終わるまで `wait` を上限に待って結果を受ける（保存は途中で止めない）。間に合わなければ true（まだ保存中）。
    fn wait_saving_within(&mut self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        self.poll_saving();
        while self.state.is_saving() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            self.poll_saving();
        }
        self.state.is_saving()
    }
}

#[cfg(test)]
mod tests {
    //! 失った知らせから、プロセスが終わるまで（本物の `std::process::exit`）を、この試験の実行ファイルを子として起こして確かめる
    //! （`child_lost`）。ウィンドウも GPU も使わない: 見張りは `GpuWatch::inject_loss` で動かす。親は、終了コード・落ちた記録の枠・普段のログ・
    //! 書き置きが、終わったあとに残っていることを見る。
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::engine::DVec2;
    use crate::lang::Lang;
    use crate::pen::PenInput;
    use crate::recovery::{DiskSpace, RecoverySettings, SpaceProbe};
    use crate::state::AppState;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let p = std::env::temp_dir().join(format!(
                "yolu-gpulost-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn space_probe() -> SpaceProbe {
        Arc::new(|_| {
            Some(DiskSpace {
                total: 1 << 40,
                available: 1 << 39,
            })
        })
    }

    /// 子プロセスの本体。`YOLU_GPULOST_DIR` の下へ記録先（`logs`）と書き置き（`recovery`）を置き、`YOLU_GPULOST_MODE`（`yes`・`nothing`・`no`・`slowsave`）の
    /// 状態で GPU を失った知らせを受ける。本物の終え方なので、戻らずにプロセスが終わる（戻ったら 99）。
    #[test]
    fn child_lost() {
        let (Ok(dir), Ok(mode)) = (
            std::env::var("YOLU_GPULOST_DIR"),
            std::env::var("YOLU_GPULOST_MODE"),
        ) else {
            return;
        };
        let dir = PathBuf::from(dir);
        crate::crash::install_at(dir.join("logs"));
        let mut state = AppState::new_in(64, 64, Lang::En);
        if mode != "no" {
            state.recovery.set_space_probe(Some(space_probe()));
            state
                .recovery
                .enable(
                    dir.join("recovery"),
                    RecoverySettings {
                        interval_seconds: 3600,
                        ..RecoverySettings::default()
                    },
                )
                .unwrap();
        }
        if mode != "nothing" {
            let layer = state.selected_layer.unwrap();
            let brush = state.stroke_settings(false);
            let mut stroke = state.doc.begin_stroke(layer, &brush).unwrap();
            stroke
                .add_point(&mut state.doc, 20.0, 20.0, 1.0, DVec2::ZERO)
                .unwrap();
            state.doc.end_stroke(stroke).unwrap();
            state.modified = true;
        }
        let ctx = egui::Context::default();
        let mut app = YoluApp::for_context(&ctx, state, PenInput::detached());
        if mode == "slowsave" {
            // 前の書き置きがあり、そのあと始めた保存が（止めてあって）終わらない
            assert!(app.state.recovery_flush());
            app.state.save.background = true;
            std::mem::forget(app.state.save.hold_next());
            app.state
                .apply(crate::state::Action::SaveProjectAs(dir.join("slow.ylp")));
            assert!(app.state.is_saving());
            app.set_gpu_lost_save_wait(Duration::from_millis(200));
        }
        // Live Link を受け付けておく（起きている印が書かれる）
        let exchange = dir.join("LiveLink");
        app.link.set_folder(exchange.clone()).unwrap();
        app.state.prefs.settings.livelink_on_startup = true;
        app.start_live_link(&ctx, [std::ffi::OsString::from("yolupainter")].into_iter());
        assert!(exchange.join("presence.json").is_file());
        let watch = GpuWatch::default();
        watch.set_adapter("Sample GPU (Vulkan)".into());
        watch.inject_loss("Unknown", "driver reset");
        app.set_gpu_watch(watch);
        app.poll_gpu_watch(&ctx);
        std::process::exit(99);
    }

    /// 子が終わるのを待つ上限（過ぎたら殺して、その時点の出力つきで試験を落とす）。
    const CHILD_TIMEOUT: Duration = Duration::from_secs(60);

    struct Ended {
        code: Option<i32>,
        /// 子の標準エラー（子が落ちたときの手がかり。試験の失敗の文に添える）。
        stderr: String,
        /// 落ちた記録の枠のうち、中身のあるもの。
        crash: Vec<String>,
        /// 空のまま残った落ちた記録の枠の数（正しく終えたあとに、空の記録先を残さない）。
        empty_crash: usize,
        /// 普段のログの全部。
        session: String,
    }

    fn texts(dir: &Path, prefix: &str) -> Vec<String> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".log"))
            })
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .collect()
    }

    fn run_child(dir: &Path, mode: &str) -> Ended {
        use std::io::Read;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "app::gpu_lost::tests::child_lost", "--nocapture"])
            .env("YOLU_GPULOST_DIR", dir)
            .env("YOLU_GPULOST_MODE", mode)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // 出力は別のスレッドで読み続ける（読まないと、子がパイプの詰まりで止まる）
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        };
        let stdout = drain(Box::new(child.stdout.take().unwrap()));
        let stderr = drain(Box::new(child.stderr.take().unwrap()));
        let deadline = Instant::now() + CHILD_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
        assert!(
            status.is_some(),
            "子が {CHILD_TIMEOUT:?} 経っても終わらない（殺した）\nstdout: {stdout}\nstderr: {stderr}"
        );
        let logs = dir.join("logs");
        let (crash, empty): (Vec<_>, Vec<_>) = texts(&logs, "crash-")
            .into_iter()
            .partition(|text| !text.is_empty());
        Ended {
            code: status.and_then(|status| status.code()),
            stderr,
            crash,
            empty_crash: empty.len(),
            session: texts(&logs, "session-").concat(),
        }
    }

    /// 次の起動が復旧のウィンドウに並べる行（落ちた体の書き置き）の数。
    fn offered_rows(root: &Path) -> usize {
        let mut next = AppState::new_in(64, 64, Lang::En);
        next.recovery.set_space_probe(Some(space_probe()));
        next.recovery
            .enable(root.to_path_buf(), RecoverySettings::default())
            .unwrap();
        next.recovery.window.as_ref().map_or(0, |w| w.rows.len())
    }

    #[test]
    fn the_process_ends_with_the_code_after_the_checkpoint_and_both_logs_are_written() {
        let dir = Dir::new("yes");
        let ended = run_child(&dir.0, "yes");
        assert_eq!(
            ended.code,
            Some(EXIT_CODE),
            "戻らずに、決まった終了コードで終わる"
        );
        assert_eq!(ended.crash.len(), 1, "{:?}", ended.crash);
        let record = &ended.crash[0];
        assert!(
            record.contains("Kind: GPU device lost")
                && record.contains("Adapter: Sample GPU (Vulkan)")
                && record.contains("Reason: Unknown")
                && record.contains("Message: driver reset")
                && record.contains("Saved for recovery: Yes"),
            "{record}"
        );
        assert_eq!(ended.empty_crash, 0, "空の記録先は残さない");
        assert!(
            !dir.0.join("LiveLink").join("presence.json").exists(),
            "Live Link の起きている印は、デストラクターが走らなくても消してから終わる"
        );
        assert!(
            ended.session.contains("Error display")
                && ended
                    .session
                    .contains("GPU lost: exiting with code 70 (saved for recovery: Yes)"),
            "{}",
            ended.session
        );
        assert_eq!(
            offered_rows(&dir.0.join("recovery")),
            1,
            "書き置きが残り、次の起動の復旧のウィンドウに並ぶ"
        );
    }

    #[test]
    fn with_nothing_to_save_the_process_ends_cleanly_without_a_checkpoint() {
        let dir = Dir::new("nothing");
        let ended = run_child(&dir.0, "nothing");
        assert_eq!(ended.code, Some(EXIT_CODE), "{}", ended.stderr);
        assert_eq!(ended.crash.len(), 1);
        assert!(
            ended.crash[0].contains("Saved for recovery: NothingToSave"),
            "{}",
            ended.crash[0]
        );
        assert!(
            ended
                .session
                .contains("GPU lost: exiting with code 70 (saved for recovery: NothingToSave)"),
            "{}",
            ended.session
        );
        assert_eq!(
            offered_rows(&dir.0.join("recovery")),
            0,
            "変更が無いので、正しく閉じて、復旧の行は出ない"
        );
    }

    #[test]
    fn without_a_recovery_session_the_records_say_it_was_not_saved() {
        let dir = Dir::new("no");
        let ended = run_child(&dir.0, "no");
        assert_eq!(ended.code, Some(EXIT_CODE), "{}", ended.stderr);
        assert_eq!(ended.crash.len(), 1);
        assert!(
            ended.crash[0].contains("Saved for recovery: No"),
            "{}",
            ended.crash[0]
        );
        assert!(
            ended
                .session
                .contains("GPU lost: exiting with code 70 (saved for recovery: No)"),
            "{}",
            ended.session
        );
    }

    #[test]
    fn a_save_still_running_at_the_deadline_is_recorded_and_the_recovery_mark_stays() {
        let dir = Dir::new("slowsave");
        let ended = run_child(&dir.0, "slowsave");
        assert_eq!(ended.code, Some(EXIT_CODE), "{}", ended.stderr);
        assert_eq!(ended.crash.len(), 1);
        let record = &ended.crash[0];
        assert!(
            record.contains("Saved for recovery: No")
                && record.contains("Save: still running at the deadline"),
            "{record}"
        );
        assert!(
            ended
                .session
                .contains("GPU lost: a save was still running at the deadline")
                && ended
                    .session
                    .contains("GPU lost: exiting with code 70 (saved for recovery: No)"),
            "{}",
            ended.session
        );
        assert!(!dir.0.join("slow.ylp").exists(), "保存は確定していない");
        assert_eq!(
            offered_rows(&dir.0.join("recovery")),
            1,
            "保存の途中で終えたので、印は消えず、前の書き置きが並ぶ"
        );
    }
}
