//! 主の GPU の装置を失ったときの流れ（装置の受け口は `gpu_watch`）。
//!
//! 失ったら、描いていた絵（core の文書）には触らずに、復旧の書き置きを急いで取り、GPU の道（キャンバスの GPU の表示・3D ビュー・
//! メモリの計測）を手放して CPU の表示へ落とし、理由を出して、終わる（保存の途中なら、保存が終わるまで待つ）。eframe は装置を作り直せない
//! （`gpu_watch` の説明）ので、窓の描画そのものが止まる。窓が描けないため、理由は OS の窓にも出す（実際の窓だけ）。
//! 終わるときは、書き置きの「保存していない作業」の印を消さない（落ちたときと同じ）ので、次の起動の復旧の窓から続きを開ける。
use std::time::Duration;

use eframe::egui_wgpu::RenderState;

use super::YoluApp;
use crate::gpu_watch::{self, GpuWatch, Lost, Saved};

/// 書き置きを急いで取るときと、終わるときに書き込み中の分を待つとき、待つ長さ（遅いディスクで、終わる前に固まらないように）。
pub(super) const RECOVERY_WAIT: Duration = Duration::from_secs(10);

impl YoluApp {
    /// 主の装置の見張りを付ける（実際の窓と、装置を失ったときの流れを確かめる試験）。付けると、受け手の無い誤りも panic にせず記録する。
    pub fn watch_gpu(&mut self, rs: &RenderState, ctx: &egui::Context) {
        let info = rs.adapter.get_info();
        let watch = GpuWatch::attach(&rs.device, ctx);
        watch.set_adapter(format!("{} ({:?})", info.name, info.backend));
        self.gpu_watch = Some(watch);
    }

    /// 主の装置の見張り（試験が、失った知らせを入れる）。
    #[doc(hidden)]
    pub fn gpu_watch(&self) -> Option<&GpuWatch> {
        self.gpu_watch.as_ref()
    }

    /// 試験用: 装置を失ったとき、書き置きの書き込みを待つ長さの上限（既定は 10 秒）。止めた書き手で、期限が働くことを短い時間で確かめる。
    #[doc(hidden)]
    pub fn set_gpu_lost_wait(&mut self, wait: Duration) {
        self.gpu_lost_wait = wait;
    }

    /// 装置を失って終わろうとしているとき、その様子。
    pub fn gpu_lost(&self) -> Option<&Lost> {
        self.gpu_lost.as_ref()
    }

    /// 見張りの知らせを受ける（フレームの初め。窓が隠れている間も）。
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
        // 変更があれば、復旧の書き置きを今すぐ取る（文書そのものは触らない）
        let saved = if !self.state.modified {
            Saved::NothingToSave
        } else if self.state.recovery.is_enabled()
            && self.state.recovery_flush_within(self.gpu_lost_wait)
        {
            Saved::Yes
        } else {
            Saved::No
        };
        // GPU の道を手放す（失った装置の資源の後始末はドライバー任せ。キャンバスは CPU の表示へ落ちる）
        self.display.attach_render_state(None);
        self.renderer3d = None;
        self.gpu_device = None;
        let lang = self.state.lang;
        self.state.message = gpu_watch::lost_text(lang, saved);
        let adapter = self
            .gpu_watch
            .as_ref()
            .map(GpuWatch::adapter)
            .unwrap_or_default();
        crate::crash::event(
            "GPU device lost",
            &gpu_watch::lost_detail(&adapter, &lost, saved),
        );
        self.gpu_lost = Some(lost);
        if self.dialogs {
            // 窓の描画が止まっているので、理由は OS の窓で出す。閉じるまでここで待つ
            crate::dialog::message()
                .set_title("YoluPainter")
                .set_description(gpu_watch::lost_dialog_text(lang, saved))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
        // 終える。保存の途中なら、保存が終わるまで閉じる流れが待つ。保存していない変更の確かめは、書き置きがあるので聞かない
        self.state.quit = true;
    }
}
