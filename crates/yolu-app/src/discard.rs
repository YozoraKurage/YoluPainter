//! 保存していない変更の問い（アプリの中のモーダルのウィンドウ）。ボタンは「変更を確認」（初めから選ばれていて Enter で押される。閉じずに・進まずに
//! アプリへ戻る）・「変更を破棄」（捨てて、閉じる・進む）・「キャンセル」（Esc。閉じない・進まない）の 3 つ。OS のメッセージのウィンドウ（rfd）は、
//! Enter で押される既定のボタンを OS ごとに保証できない（Windows は `common-controls-v6` と manifest が要り、Linux の zenity は rfd が既定を決めない）ので使わない。
//!
//! 問いは、聞く側（`YoluApp`）が `Ask` を持ち、毎フレームここの `show` で描いて、選びを読む。捨てて進む先は `AfterDiscard`
//! （聞く前の処理を、答えが出てから続ける）。走っている仕事だけを取り消す問い（変更は無い）は「終わる」・「キャンセル」の 2 つ（Enter はキャンセル）。

use egui::{pos2, vec2, Id, Key, Modifiers, Rect};

use crate::lang::Lang;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w};
use crate::ui::window;

/// 問いの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 保存していない変更がある（閉じる・ほかを開く・新しく作る前）。
    Changes,
    /// 変更は無く、走っている仕事が取り消される（閉じる前）。
    JobsOnly,
}

/// 開いている問い。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub kind: Kind,
    pub text: String,
    /// 答えが「変更を破棄」のときに続ける処理。
    pub then: AfterDiscard,
}

/// 捨てて進む先（聞く前の処理の続き）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterDiscard {
    /// ウィンドウを閉じる・終了する（もう一度閉じる流れを回すと、問わずに閉じる）。
    Close,
    /// Live Link の違う相手の頼みに「捨ててよい」と答える（そうでなければ「捨てない」）。
    LinkDiscard,
    /// 復旧の世代を開く。
    Recovery(crate::recovery::OpenRequest),
    /// 新しいプロジェクトのウィンドウを開く。
    New,
    /// 開くファイルを選んで開く。
    Open,
    /// このプロジェクトを開く。
    OpenPath(std::path::PathBuf),
    /// PSD を選んで、今のセットの文書に替えて読む。
    PsdCurrent,
}

/// 選び。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// 変更を確認（閉じずに・進まずに戻る）。
    Review,
    /// 変更を破棄（捨てて進む）。
    Discard,
    /// キャンセル。
    Cancel,
}

/// ウィンドウの id（テストが矩形を読む）。
pub fn window_id() -> Id {
    Id::new("yolu.window.discard")
}

/// ボタンの名前（変更を確認・変更を破棄・キャンセル）。
pub fn labels(lang: Lang) -> [&'static str; 3] {
    [
        lang.t("discard-review-changes"),
        lang.t("discard-discard-changes"),
        lang.t("discard-cancel"),
    ]
}

/// 走っている仕事だけの問いの、終わるボタンの名前。
pub fn quit_label(lang: Lang) -> &'static str {
    lang.t("discard-quit")
}

const SIZE: egui::Vec2 = vec2(440.0, 150.0);
const BUTTON: egui::Vec2 = vec2(116.0, 28.0);

/// 問いを描き、選びがあれば返す。Enter は初めから選んでいるボタン（変更の問いは「変更を確認」・仕事だけの問いは「キャンセル」）、Esc とウィンドウの
/// 閉じるボタンは「キャンセル」。
pub fn show(ctx: &egui::Context, lang: Lang, ask: &Ask) -> Option<Choice> {
    let id = window_id();
    let [review, discard, cancel] = labels(lang);
    let spec = window::Spec {
        title: "YoluPainter",
        icon: None,
        size: SIZE,
        modal: true,
        close_label: lang.t("discard-close"),
    };
    let esc = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let mut offset = ctx
        .data(|d| d.get_temp::<egui::Vec2>(id.with("offset")))
        .unwrap_or_default();
    let mut chosen = None;
    let closed = window::show(ctx, id, &spec, &mut offset, esc, |ui, frame| {
        let inner = frame.body.shrink2(vec2(16.0, 12.0));
        let text = Rect::from_min_max(
            inner.min,
            pos2(inner.right(), inner.bottom() - BUTTON.y - 12.0),
        );
        w::wrapped_text(ui.painter(), text, &ask.text, t::LABEL);
        let row_y = inner.bottom() - BUTTON.y;
        let mut right = inner.right();
        let mut place = |name: &str, primary: bool| {
            let r = Rect::from_min_size(pos2(right - BUTTON.x, row_y), BUTTON);
            right = r.left() - 8.0;
            (r, name.to_owned(), primary)
        };
        let buttons: Vec<(Choice, Rect, String, bool)> = match ask.kind {
            Kind::Changes => {
                // 右から並べる: キャンセル・変更を破棄・変更を確認（左端が初めの選び）
                let c = place(cancel, false);
                let d = place(discard, false);
                let r = place(review, true);
                vec![
                    (Choice::Cancel, c.0, c.1, c.2),
                    (Choice::Discard, d.0, d.1, d.2),
                    (Choice::Review, r.0, r.1, r.2),
                ]
            }
            Kind::JobsOnly => {
                let c = place(cancel, true);
                let q = place(quit_label(lang), false);
                vec![
                    (Choice::Cancel, c.0, c.1, c.2),
                    (Choice::Discard, q.0, q.1, q.2),
                ]
            }
        };
        for (choice, r, name, primary) in buttons {
            if w::button(
                ui,
                r,
                ("yolu.discard", name.as_str()),
                &name,
                primary,
                true,
                None,
                None,
            )
            .clicked()
            {
                chosen = Some(choice);
            }
        }
    });
    ctx.data_mut(|d| d.insert_temp(id.with("offset"), offset));
    if chosen.is_none() && enter {
        chosen = Some(match ask.kind {
            Kind::Changes => Choice::Review,
            Kind::JobsOnly => Choice::Cancel,
        });
    }
    if chosen.is_none() && closed {
        chosen = Some(Choice::Cancel);
    }
    chosen
}

/// ウィンドウの最後に描いた矩形（テストが読む）。
pub fn window_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}
