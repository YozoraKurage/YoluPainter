//! 主の wgpu のデバイス（eframe が 1 つだけ作る。ウィンドウの描画・キャンバスの GPU の表示・3D ビューが使う）の見張り。
//!
//! wgpu の既定は 2 つとも黙って困る形: GPU を失っても何も知らせず（以後の描画が黙って失敗し、ウィンドウが描き換わらなくなる）、受け手の無い
//! 誤り（検証・メモリ不足）は panic でアプリごと落とす。ここで両方の受け口を付け、どちらも画面のスレッドが次のフレームで読む。
//!
//! - GPU を失ったとき: `YoluApp` が、描いていた絵（core の文書）には触らずに復旧の書き置きを急いで取り、GPU の道（キャンバス・3D ビュー）を
//!   手放して、理由を出し、失った GPU で 1 度も描かずにプロセスを終える（`app::gpu_lost`）。eframe はデバイスを作り直せない（デバイスはウィンドウを
//!   初めて作るときに 1 度だけ作られ、`RenderState` に固定される。作り直せるのは面だけ）うえ、失ったデバイスでの描画は egui-wgpu の中で panic になる
//!   ので、続けられない。復旧用に保存したあと、次の起動へ任せる。
//! - 受け手の無い誤り: 落とさず、普段のログへ 1 行（同じ文は 1 度）。
//!
//! 保証の射程: 失ったことをすぐ知ることは保証しない。受け口が呼ばれるのは、wgpu が失ったと気づいたあと（次の提出・poll）で、その間の描画は
//! 黙って失敗する。画面のスレッドは、フレームの初めのほか、フレームの中の描画の区切り（別ウィンドウを描く前・別ウィンドウ 1 つの終わり・
//! 別ウィンドウ 1 つを描いた直後・`ui` の終わり。`app::FramePoint`）で読む。区切りの間で失った分（読んだあと、eframe の描画の中で失ったと
//! 分かる）は防げない。
//!
//! 受け口は別のスレッド（wgpu の内部）から呼ばれる。ここでは書き込みと描き直しの頼みだけをして、重い処理は画面のスレッドに任せる。
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::wgpu;

use crate::lang::Lang;

/// 覚えておく誤りの文の数の上限（同じ文は 1 つ。画面のスレッドが読むまでの分）。
const ERROR_KEEP: usize = 16;
/// 1 回の実行で、別々の文として記録する誤りの数の上限（毎フレーム違う文の誤りで、ログが埋まらないように）。
const ERROR_KINDS: usize = 64;

/// 失った GPU の様子。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lost {
    /// wgpu が言う理由（`Unknown`・`Destroyed`）。
    pub reason: String,
    /// ドライバーの文（記録だけに使う。画面には出さない）。
    pub message: String,
}

/// 画面のスレッドが読む知らせ。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Events {
    /// GPU を失った（1 度だけ返す）。
    pub lost: Option<Lost>,
    /// 受け手の無い誤りの文（新しいものだけ）。
    pub errors: Vec<String>,
}

#[derive(Default)]
struct Inner {
    adapter: String,
    lost: Option<Lost>,
    announced: bool,
    errors: VecDeque<String>,
    seen: HashSet<u64>,
}

/// デバイスの見張り。複製は同じ状態を指す（wgpu の受け口へ渡す側と、読む側）。
#[derive(Clone, Default)]
pub struct GpuWatch(Arc<Mutex<Inner>>);

impl GpuWatch {
    /// `device` に受け口を付ける。何かあれば `ctx` へ描き直しを頼む（ウィンドウが隠れていても、次の `logic` で読む）。
    pub fn attach(device: &wgpu::Device, ctx: &egui::Context) -> GpuWatch {
        let watch = GpuWatch::default();
        let (on_lost, lost_ctx) = (watch.clone(), ctx.clone());
        device.set_device_lost_callback(move |reason, message| {
            on_lost.record_loss(format!("{reason:?}"), message);
            lost_ctx.request_repaint();
        });
        let (on_error, error_ctx) = (watch.clone(), ctx.clone());
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            on_error.record_error(error.to_string());
            error_ctx.request_repaint();
        }));
        watch
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record_loss(&self, reason: String, message: String) {
        let mut inner = self.inner();
        // 最初の失った理由だけを覚える（失ったあとの誤りは、その結果）
        inner.lost.get_or_insert(Lost { reason, message });
    }

    fn record_error(&self, text: String) {
        let mut inner = self.inner();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let key = hasher.finish();
        if inner.seen.len() >= ERROR_KINDS || !inner.seen.insert(key) {
            return;
        }
        if inner.errors.len() >= ERROR_KEEP {
            inner.errors.pop_front();
        }
        inner.errors.push_back(text);
    }

    /// 見張っている GPU のアダプターの名前（記録だけに使う）。
    pub fn set_adapter(&self, name: String) {
        self.inner().adapter = name;
    }

    pub fn adapter(&self) -> String {
        self.inner().adapter.clone()
    }

    /// 溜まった知らせを取り出す（失った知らせは 1 度だけ）。
    pub fn take(&self) -> Events {
        let mut inner = self.inner();
        let lost = if inner.announced {
            None
        } else {
            inner.lost.clone()
        };
        inner.announced |= lost.is_some();
        Events {
            lost,
            errors: inner.errors.drain(..).collect(),
        }
    }

    /// GPU を失ったか（知らせを取り出したあとも true）。
    pub fn is_lost(&self) -> bool {
        self.inner().lost.is_some()
    }

    /// 試験用: GPU を失った知らせを入れる（本物の GPU の喪失を起こさずに、受ける側の流れを通す）。
    #[doc(hidden)]
    pub fn inject_loss(&self, reason: &str, message: &str) {
        self.record_loss(reason.to_owned(), message.to_owned());
    }

    /// 試験用: 受け手の無い誤りの知らせを入れる。
    #[doc(hidden)]
    pub fn inject_error(&self, text: &str) {
        self.record_error(text.to_owned());
    }
}

/// 復旧用に書けたか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    /// 変更があり、復旧の書き置きに入った。
    Yes,
    /// 保存していない変更が無い（書き置きは要らない）。
    NothingToSave,
    /// 書けなかった（復旧が動いていない・期限に間に合わなかった・書き込みの失敗）。
    No,
}

/// GPU でエラーが起きて続けられないときの理由の文（画面と、終わる前のウィンドウに出す。名前・状態・短い理由だけ）。
pub fn lost_text(lang: Lang, saved: Saved) -> String {
    let head = lang.pick(
        "GPU でエラーが起きたため、続けられません。",
        "A GPU error occurred, so YoluPainter cannot continue.",
    );
    let tail = match saved {
        Saved::Yes => lang.pick(
            "描いていた絵は復旧用に保存しました。",
            " Your work was saved for recovery.",
        ),
        Saved::NothingToSave => "",
        Saved::No => lang.pick(
            "復旧用に保存できませんでした。",
            " Could not save for recovery.",
        ),
    };
    format!("{head}{tail}")
}

/// ウィンドウに出す文（`lost_text` に、このあと終わることを足す。ボタンの文言を替えられない OS でも、OK が何をするかが分かる）。
pub fn lost_dialog_text(lang: Lang, saved: Saved) -> String {
    format!(
        "{}\n{}",
        lost_text(lang, saved),
        lang.pick("このあと終了します。", "YoluPainter will now close.")
    )
}

/// 落ちた記録の枠へ書く中身（開発向け。ドライバーの文を含む）。
pub fn lost_detail(adapter: &str, lost: &Lost, saved: Saved) -> String {
    format!(
        "Adapter: {adapter}\nReason: {}\nMessage: {}\nSaved for recovery: {saved:?}",
        lost.reason, lost.message
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loss_is_announced_once_and_the_first_reason_stays() {
        let watch = GpuWatch::default();
        assert_eq!(watch.take(), Events::default());
        assert!(!watch.is_lost());
        watch.inject_loss("Unknown", "driver reset");
        watch.inject_loss("Destroyed", "after the first");
        assert!(watch.is_lost());
        let first = watch.take();
        assert_eq!(
            first.lost,
            Some(Lost {
                reason: "Unknown".into(),
                message: "driver reset".into()
            })
        );
        assert_eq!(watch.take().lost, None, "知らせは 1 度だけ");
        assert!(watch.is_lost(), "失ったことは残る");
    }

    #[test]
    fn identical_errors_are_one_and_the_kinds_are_capped() {
        let watch = GpuWatch::default();
        for _ in 0..1000 {
            watch.inject_error("validation: same");
        }
        assert_eq!(watch.take().errors, vec!["validation: same".to_owned()]);
        assert!(
            watch.take().errors.is_empty(),
            "同じ文は、取り出したあとも数えない"
        );
        for i in 0..1000 {
            watch.inject_error(&format!("error {i}"));
        }
        let taken = watch.take().errors;
        assert!(taken.len() <= ERROR_KEEP);
        for i in 1000..2000 {
            watch.inject_error(&format!("error {i}"));
        }
        let total_kinds = watch.inner().seen.len();
        assert!(total_kinds <= ERROR_KINDS, "{total_kinds}");
    }

    #[test]
    fn the_reason_names_the_state_and_never_tells_what_to_do() {
        for lang in [Lang::Ja, Lang::En] {
            for saved in [Saved::Yes, Saved::NothingToSave, Saved::No] {
                let text = lost_text(lang, saved);
                assert!(
                    text.starts_with(lang.pick(
                        "GPU でエラーが起きたため、続けられません。",
                        "A GPU error occurred, so YoluPainter cannot continue."
                    )),
                    "{text}"
                );
                let dialog = lost_dialog_text(lang, saved);
                assert!(
                    dialog.starts_with(&text) && dialog.lines().count() == 2,
                    "{dialog}"
                );
                if lang == Lang::En {
                    assert!(text.is_ascii(), "{text}");
                }
                // 画面・ウィンドウの文に、利用者に分かりにくい言葉（装置・機械・device）を使わない
                for word in ["装置", "機械", "device", "Device"] {
                    assert!(!dialog.contains(word), "{word}: {dialog}");
                }
            }
        }
        assert!(lost_text(Lang::Ja, Saved::Yes).contains("復旧用に保存しました"));
        assert!(lost_text(Lang::En, Saved::Yes).contains("saved for recovery"));
        assert!(lost_text(Lang::Ja, Saved::No).contains("保存できませんでした"));
        assert!(lost_text(Lang::En, Saved::No).contains("Could not save"));
        assert!(!lost_text(Lang::En, Saved::NothingToSave).contains("recovery"));
    }

    /// 画面・ウィンドウに出る文は、日英とも次のとおり（書き置きの有無の 3 通りずつ）。
    #[test]
    fn the_texts_are_exactly_these() {
        let ja = |saved| lost_text(Lang::Ja, saved);
        let en = |saved| lost_text(Lang::En, saved);
        assert_eq!(
            ja(Saved::Yes),
            "GPU でエラーが起きたため、続けられません。描いていた絵は復旧用に保存しました。"
        );
        assert_eq!(
            ja(Saved::NothingToSave),
            "GPU でエラーが起きたため、続けられません。"
        );
        assert_eq!(
            ja(Saved::No),
            "GPU でエラーが起きたため、続けられません。復旧用に保存できませんでした。"
        );
        assert_eq!(
            en(Saved::Yes),
            "A GPU error occurred, so YoluPainter cannot continue. Your work was saved for recovery."
        );
        assert_eq!(
            en(Saved::NothingToSave),
            "A GPU error occurred, so YoluPainter cannot continue."
        );
        assert_eq!(
            en(Saved::No),
            "A GPU error occurred, so YoluPainter cannot continue. Could not save for recovery."
        );
        assert_eq!(
            lost_dialog_text(Lang::Ja, Saved::NothingToSave),
            "GPU でエラーが起きたため、続けられません。\nこのあと終了します。"
        );
        assert_eq!(
            lost_dialog_text(Lang::En, Saved::NothingToSave),
            "A GPU error occurred, so YoluPainter cannot continue.\nYoluPainter will now close."
        );
    }

    #[test]
    fn the_crash_record_names_the_adapter_reason_and_whether_the_work_was_saved() {
        let lost = Lost {
            reason: "Unknown".into(),
            message: "driver reset".into(),
        };
        for (saved, word) in [
            (Saved::Yes, "Yes"),
            (Saved::NothingToSave, "NothingToSave"),
            (Saved::No, "No"),
        ] {
            assert_eq!(
                lost_detail("Sample (Vulkan)", &lost, saved),
                format!(
                    "Adapter: Sample (Vulkan)\nReason: Unknown\nMessage: driver reset\nSaved for recovery: {word}"
                )
            );
        }
    }
}
