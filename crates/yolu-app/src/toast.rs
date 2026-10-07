//! 小さな知らせ（トースト）。直前の操作の結果と理由（`AppState::message`）は、状態の帯の左には常に出さず、ここで短く出して消える。
//!
//! - `message` を書く操作（`AppState::apply`・1 フレーム）の終わりに、`message` が書かれていれば、前と同じ文でも新しい知らせとして出す
//!   （「保存しました。」を続けて押せば、そのたびに出る）。操作の外で直接書かれた文は、次のフレームの始めに、前に見た文と違えば出す。
//! - 出し方は知らせの種類（`notice::Kind`。文の中身では決めない）で決める: 済んだ知らせは数秒、断り・注意・失敗はそれより長く。
//!   押せば（どの知らせも）すぐ消える。ポインタを乗せているあいだは消えない（読む間）。最後の 0.4 秒でうすくなる。
//!   知らせを通さずに直に書かれた文（種類が無い）は、済んだ知らせと同じ出し方。
//! - 文の中身は今のまま（`message` は `notice` の `notify` が書く。試験は `message` と `shell::status_text` を読む。操作の外でも最後の文は読める）。
//! - 文を空にして（操作の中は `AppState::clear_message`）から同じ文が来たときも、新しい知らせとして出す。

use egui::{pos2, vec2, Align2, Color32, Order, Sense, Stroke};

use crate::notice::Kind;
use crate::state::AppState;
use crate::ui::theme as t;

/// 済んだ知らせを出している秒数。
pub const INFO_SECONDS: f64 = 4.0;
/// 断り・注意・失敗を出している秒数。
pub const LONG_SECONDS: f64 = 12.0;
/// ポインタを乗せ続けたとき、初めて出てから出し続けてよい長さの上限（普通の出し方の何倍まで。済んだ知らせ 12 秒・ほか 36 秒）。
pub const HOLD_LIMIT: f64 = 3.0;
/// 消える前にうすくなる秒数。
pub const FADE_SECONDS: f64 = 0.4;
/// 知らせの最大の幅（文字の部分）。
const MAX_WIDTH: f32 = 520.0;
const PADDING: egui::Vec2 = vec2(12.0, 7.0);

/// 今出している知らせ。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toast {
    /// 出している文（消えていても、最後の文）。
    text: String,
    /// 最後に見た `message`（操作の外で直接書かれた文が新しいかを見る。空になれば空）。
    observed: String,
    /// 新しく出す文と種類（次の `update` で出し始める）。
    pending: Option<(String, Kind)>,
    /// 出し始めた時刻（egui の時刻。ポインタを乗せているあいだは延びる）。
    since: f64,
    /// この文が初めて出た時刻（乗せ続けても、ここから `HOLD_LIMIT` 倍までしか出さない）。
    first: f64,
    /// 押されて消した。
    dismissed: bool,
    /// 出している文の種類。
    kind: Kind,
    /// `message` を書く操作が入れ子になっている深さ（`begin` から `end` まで）。
    depth: u32,
    /// いちばん外の操作の中で、`message` を明示して空にした（空のまま終わっても、前の文を戻さない）。
    cleared: bool,
    /// いちばん外の操作の中で、すでに知らせとして出した文（内側の操作の終わりで出した文を、外側の終わりで重ねて出さない）。
    noted: Option<String>,
}

impl Toast {
    /// `message` を書く操作の終わりに呼ぶ: 書かれた文を、前と同じ文でも新しい知らせとして出す（次の `update` から）。
    pub fn write(&mut self, message: &str, kind: Kind) {
        self.observed = message.to_owned();
        self.pending = (!message.is_empty()).then(|| (message.to_owned(), kind));
        if message.is_empty() {
            // 文が空になれば、出している知らせも消える（空の message に知らせは無い）
            self.text.clear();
        }
    }

    /// 操作の外で直接書かれた `message` を見る（フレームの始め）。前に見た文と違えば新しい知らせ。空になっていれば、出している知らせも
    /// 出す前の知らせも取り下げる（次に来る文は、前と同じでも新しい知らせ）。
    pub fn observe(&mut self, message: &str, kind: Kind) {
        if self.observed != message {
            self.write(message, kind);
        }
    }

    /// `message` を書く操作（`AppState::apply`・1 フレーム）の入口: 前の文を預かって `message` を空にする。出口（`end`）で、
    /// 操作が文を書いたかを、前と同じ文でも見分けられる。操作の外で直接書かれていた文は、ここで見る。
    pub fn begin(&mut self, message: &mut String, kind: Kind) -> String {
        if self.depth == 0 {
            self.observe(message, kind);
            self.noted = None;
            self.cleared = false;
        }
        self.depth += 1;
        std::mem::take(message)
    }

    /// `begin` の出口。文が書かれていれば新しい知らせとして出し（文はそのまま残す）、書かれていなければ預かった前の文を戻す
    /// （操作の外から最後の文はいつでも読める）。
    pub fn end(&mut self, message: &mut String, prior: String, kind: Kind) {
        self.depth = self.depth.saturating_sub(1);
        if !message.is_empty() {
            self.flush(message, kind);
        } else if !self.cleared {
            *message = prior;
        }
    }

    /// 操作の中で `message` を明示して空にする（出す前の知らせも取り下げる。空のまま操作が終わっても、前の文を戻さない）。
    pub fn clear(&mut self, message: &mut String) {
        message.clear();
        self.cleared = true;
        self.write("", Kind::Info);
    }

    /// 操作の途中で、ここまでに書かれた `message`（`begin` のあとに書かれた分だけ）を知らせにする。同じ操作の中で出した文は重ねて出さない。
    pub fn flush(&mut self, message: &str, kind: Kind) {
        if !message.is_empty() && self.noted.as_deref() != Some(message) {
            self.write(message, kind);
            self.noted = Some(message.to_owned());
        }
    }

    /// 出し始めるのを待っている文があるか。
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// 毎フレーム、時刻 `now` に呼ぶ。新しく出す文があれば、そこから出し始める。
    pub fn update(&mut self, now: f64) {
        if let Some((text, kind)) = self.pending.take() {
            self.kind = kind;
            self.text = text;
            self.since = now;
            self.first = now;
            self.dismissed = false;
        }
    }

    /// 出している文字（消えていても、最後の文）。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 出している文の種類。
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// この文を出している秒数。
    pub fn lifetime(&self) -> f64 {
        if self.kind == Kind::Info {
            INFO_SECONDS
        } else {
            LONG_SECONDS
        }
    }

    /// 時刻 `now` に出ているか。
    pub fn is_visible(&self, now: f64) -> bool {
        !self.dismissed && !self.text.is_empty() && now - self.since < self.lifetime()
    }

    /// 消えるまでの残り（秒。出ていなければ 0）。
    pub fn remaining(&self, now: f64) -> f64 {
        if self.is_visible(now) {
            (self.since + self.lifetime() - now).max(0.0)
        } else {
            0.0
        }
    }

    /// 押されて消す。
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// ポインタが乗っているあいだは、出し始めを今に延ばす（読んでいるあいだに消えない）。ただし、出している長さが初めて出てから
    /// `HOLD_LIMIT` 倍に収まるあいだだけ（ポインタを置きっぱなしでも、いつかは消える）。
    pub fn hold(&mut self, now: f64) {
        if self.is_visible(now) && now - self.first < self.lifetime() * (HOLD_LIMIT - 1.0) {
            self.since = now;
        }
    }

    /// 濃さ（最後の `FADE_SECONDS` でうすくなる）。
    pub fn opacity(&self, now: f64) -> f32 {
        (self.remaining(now) / FADE_SECONDS).clamp(0.0, 1.0) as f32
    }
}

/// 知らせを描く（毎フレーム。ビューの左下、状態の帯のすぐ上）。消える時刻に描き直しを頼む。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    let now = ctx.input(|i| i.time);
    // フレームの中で、ここまでに書かれた文（`apply` の外の書き込みも）をすぐ出す
    app.flush_message();
    app.toast.update(now);
    if !app.toast.is_visible(now) {
        return;
    }
    let text = app.toast.text().to_owned();
    let kind = app.toast.kind();
    let opacity = app.toast.opacity(now);
    let screen = ctx.content_rect();
    let wrap =
        (screen.width() - t::TOOL_STRIP_WIDTH - 48.0 - PADDING.x * 2.0).clamp(120.0, MAX_WIDTH);
    let mut dismissed = false;
    let mut hovered = false;
    egui::Area::new(egui::Id::new("yolu.toast"))
        .order(Order::Foreground)
        .anchor(
            Align2::LEFT_BOTTOM,
            vec2(t::TOOL_STRIP_WIDTH + 12.0, -(t::STATUS_BAR_HEIGHT + 12.0)),
        )
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            let fade = |c: Color32| c.gamma_multiply(opacity);
            let galley = ui
                .painter()
                .layout(text.clone(), t::LABEL.font(), fade(t::TEXT), wrap);
            let size = galley.size() + PADDING * 2.0 + vec2(4.0, 0.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let p = ui.painter();
            p.rect_filled(rect.expand(1.0), 6.0, fade(Color32::from_black_alpha(60)));
            p.rect_filled(rect, 5.0, fade(t::PANEL_HEADER));
            p.rect_stroke(
                rect,
                5.0,
                Stroke::new(1.0, fade(t::SEPARATOR)),
                egui::StrokeKind::Inside,
            );
            // 左の帯: 失敗と断りは赤、注意は黄、済んだ知らせは青
            p.rect_filled(
                egui::Rect::from_min_size(
                    rect.min + vec2(0.0, 3.0),
                    vec2(3.0, rect.height() - 6.0),
                ),
                1.5,
                fade(band_color(kind)),
            );
            p.galley(
                rect.min + pos2(PADDING.x + 4.0, PADDING.y).to_vec2(),
                galley,
                t::TEXT,
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
            dismissed = response.clicked();
            hovered = response.hovered();
        });
    if dismissed {
        app.toast.dismiss();
        return;
    }
    if hovered {
        app.toast.hold(now);
    }
    // 消える時刻（と、うすくなっていく間）に描き直す
    let remaining = app.toast.remaining(now);
    let wait = if remaining > FADE_SECONDS {
        remaining - FADE_SECONDS
    } else {
        1.0 / 30.0
    };
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
}

/// トーストの左の帯の色（種類ごと。色は今ある物だけ）。
pub fn band_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Info => t::ACCENT,
        Kind::Warning => t::WARNING,
        Kind::Refusal | Kind::Error => t::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_decides_how_long_a_toast_stays_not_the_wording() {
        // 言い回しによらず、種類だけで決まる: 済んだ知らせは短く、断り・注意・失敗は長く
        for (kind, seconds) in [
            (Kind::Info, INFO_SECONDS),
            (Kind::Refusal, LONG_SECONDS),
            (Kind::Warning, LONG_SECONDS),
            (Kind::Error, LONG_SECONDS),
        ] {
            for text in [
                "保存できません。",
                "保存しました。",
                "Cannot save.",
                "Saved.",
            ] {
                let mut toast = Toast::default();
                toast.write(text, kind);
                toast.update(0.0);
                assert_eq!(toast.kind(), kind);
                assert_eq!(toast.lifetime(), seconds, "{kind:?} {text}");
            }
        }
        // 色: 失敗と断りは赤、注意は黄、済んだ知らせは青（今ある色だけ）
        assert_eq!(band_color(Kind::Error), t::ERROR);
        assert_eq!(band_color(Kind::Refusal), t::ERROR);
        assert_eq!(band_color(Kind::Warning), t::WARNING);
        assert_eq!(band_color(Kind::Info), t::ACCENT);
    }

    /// `message` を書く操作（`apply`・1 フレーム）の代わり: 入口で預かり、`text` を書いて（None なら書かずに）、出口で出す。
    fn operation(toast: &mut Toast, message: &mut String, text: Option<&str>) {
        operation_with(toast, message, text, Kind::Info);
    }

    /// `operation` の、書く文の種類を決めた形。
    fn operation_with(toast: &mut Toast, message: &mut String, text: Option<&str>, kind: Kind) {
        let prior = toast.begin(message, Kind::Info);
        if let Some(text) = text {
            *message = text.to_owned();
        }
        toast.end(message, prior, kind);
    }

    #[test]
    fn a_new_sentence_shows_for_a_few_seconds_and_an_error_for_longer() {
        let mut toast = Toast::default();
        let mut message = String::new();
        toast.update(0.0);
        assert!(!toast.is_visible(0.0));
        operation(&mut toast, &mut message, Some("保存しました。"));
        toast.update(1.0);
        assert!(toast.is_visible(1.0) && toast.is_visible(1.0 + INFO_SECONDS - 0.01));
        assert!(!toast.is_visible(1.0 + INFO_SECONDS + 0.01));
        operation_with(&mut toast, &mut message, Some("開けません"), Kind::Error);
        toast.update(10.0);
        assert_eq!(toast.kind(), Kind::Error);
        assert!(
            toast.is_visible(10.0 + INFO_SECONDS + 1.0),
            "エラーは普通の知らせより長く出る"
        );
        assert!(
            toast.is_visible(10.0 + LONG_SECONDS - 0.01)
                && !toast.is_visible(10.0 + LONG_SECONDS + 0.01)
        );
        const { assert!(LONG_SECONDS > INFO_SECONDS * 2.0) };
    }

    #[test]
    fn the_same_sentence_written_again_is_a_new_toast_after_it_timed_out_or_was_dismissed() {
        let mut toast = Toast::default();
        let mut message = String::new();
        operation(&mut toast, &mut message, Some("保存しました。"));
        toast.update(0.0);
        assert!(toast.is_visible(1.0));
        // 時間切れのあとに、同じ文がもう一度書かれる（message は書き換わらず残ったまま）
        assert!(!toast.is_visible(INFO_SECONDS + 0.5));
        operation(&mut toast, &mut message, Some("保存しました。"));
        toast.update(10.0);
        assert!(
            toast.is_visible(10.5),
            "時間切れのあとの同じ文も、新しい知らせ"
        );
        // 押して消したあとにも
        toast.dismiss();
        assert!(!toast.is_visible(11.0));
        operation(&mut toast, &mut message, Some("保存しました。"));
        toast.update(12.0);
        assert!(
            toast.is_visible(12.5),
            "押して消したあとの同じ文も、新しい知らせ"
        );
        assert_eq!(message, "保存しました。");
    }

    #[test]
    fn an_operation_that_writes_nothing_keeps_the_last_sentence_and_shows_nothing_new() {
        let mut toast = Toast::default();
        let mut message = String::new();
        operation(&mut toast, &mut message, Some("取り消しました。"));
        toast.update(0.0);
        assert!(!toast.is_pending());
        // 何も書かない操作・フレーム: 最後の文は読めるまま、出し直さない
        operation(&mut toast, &mut message, None);
        assert_eq!(message, "取り消しました。", "最後の文は操作の外から読める");
        toast.update(INFO_SECONDS + 1.0);
        assert!(
            !toast.is_pending() && !toast.is_visible(INFO_SECONDS + 1.0),
            "書かれていなければ出し直さない"
        );
    }

    #[test]
    fn clearing_the_message_in_an_operation_sticks_and_withdraws_a_waiting_toast() {
        let mut toast = Toast::default();
        let mut message = String::from("前の文");
        // 明示して空にした操作は、何も書かずに終わっても、前の文を戻さない（保存の結果が書かれたかを見分ける所が頼る）
        let prior = toast.begin(&mut message, Kind::Info);
        toast.clear(&mut message);
        toast.end(&mut message, prior, Kind::Info);
        assert!(message.is_empty());
        // 書いてから空にすれば、まだ出していない知らせも取り下げる
        let prior = toast.begin(&mut message, Kind::Info);
        message = "書いた文".into();
        toast.flush(&message, Kind::Info);
        assert!(toast.is_pending());
        toast.clear(&mut message);
        toast.end(&mut message, prior, Kind::Info);
        assert!(message.is_empty() && !toast.is_pending());
        // 次の操作は、また前の文を戻す
        message = "後の文".into();
        let prior = toast.begin(&mut message, Kind::Info);
        toast.end(&mut message, prior, Kind::Info);
        assert_eq!(message, "後の文");
    }

    #[test]
    fn nested_operations_notify_once_per_sentence_and_again_for_a_different_one() {
        let mut toast = Toast::default();
        let mut message = String::new();
        // フレームの中の apply（入れ子）が書いた文は、外側の終わりで重ねて出さない（出し始めたあとに、もう一度出し始めて消えた印を戻さない）
        let frame = toast.begin(&mut message, Kind::Info);
        let apply = toast.begin(&mut message, Kind::Info);
        message = "保存しました。".into();
        toast.end(&mut message, apply, Kind::Info);
        toast.update(0.0);
        toast.dismiss();
        toast.end(&mut message, frame, Kind::Info);
        assert!(!toast.is_pending(), "内側で出した文を外側が重ねて出さない");
        toast.update(0.1);
        assert!(
            !toast.is_visible(0.2),
            "押して消した知らせが、同じフレームの終わりで戻らない"
        );
        // 同じフレームの中で、別の文があとから書かれれば、それも出す
        let frame = toast.begin(&mut message, Kind::Info);
        let apply = toast.begin(&mut message, Kind::Info);
        message = "保存しました。".into();
        toast.end(&mut message, apply, Kind::Info);
        message = "取り消しました。".into();
        toast.end(&mut message, frame, Kind::Info);
        assert!(toast.is_pending());
        toast.update(1.0);
        assert_eq!(toast.text(), "取り消しました。");
    }

    #[test]
    fn a_sentence_written_outside_any_operation_is_seen_when_it_differs_from_the_last_one() {
        let mut toast = Toast::default();
        // 起動の知らせ・試験が直接書いた文: 次のフレームの始めに、前に見た文と違えば出す
        let mut message = String::from("起動時の知らせ");
        let prior = toast.begin(&mut message, Kind::Info);
        assert!(message.is_empty(), "操作の中では前の文は見えない");
        toast.end(&mut message, prior, Kind::Info);
        assert_eq!(message, "起動時の知らせ");
        toast.update(0.0);
        assert!(toast.is_visible(1.0));
        // 同じ文のままなら出し直さない。空にしてからなら、同じ文でも新しい知らせ
        let prior = toast.begin(&mut message, Kind::Info);
        toast.end(&mut message, prior, Kind::Info);
        assert!(!toast.is_pending());
        message.clear();
        let prior = toast.begin(&mut message, Kind::Info);
        toast.end(&mut message, prior, Kind::Info);
        assert!(
            !toast.is_visible(1.5),
            "空にされたら、出している知らせも消える"
        );
        message = "起動時の知らせ".into();
        let prior = toast.begin(&mut message, Kind::Info);
        toast.end(&mut message, prior, Kind::Info);
        assert!(toast.is_pending(), "空を挟めば、同じ文でも新しい知らせ");
    }

    #[test]
    fn pressing_dismisses_and_hovering_holds_and_the_end_fades() {
        let mut toast = Toast::default();
        toast.write("取り消しました。", Kind::Info);
        toast.update(0.0);
        assert_eq!(toast.opacity(1.0), 1.0);
        assert!(
            toast.opacity(INFO_SECONDS - FADE_SECONDS / 2.0) < 1.0
                && toast.opacity(INFO_SECONDS - FADE_SECONDS / 2.0) > 0.0
        );
        toast.hold(3.0);
        assert!(
            toast.is_visible(3.0 + INFO_SECONDS - 0.01),
            "乗せているあいだは延びる"
        );
        // 乗せ続けても、初めて出てから HOLD_LIMIT 倍で打ち切る（合計で INFO_SECONDS * HOLD_LIMIT を超えない）
        let limit = INFO_SECONDS * HOLD_LIMIT;
        let mut t = 3.0;
        while t < limit + 1.0 {
            toast.hold(t);
            t += 0.25;
        }
        assert!(
            !toast.is_visible(limit + 0.01),
            "置きっぱなしでも、初めて出てから {limit} 秒で消える"
        );
        assert!(
            toast.is_visible(limit - 0.5),
            "乗せているあいだは、上限まで出ている"
        );
        toast.write("取り消しました。", Kind::Info);
        toast.update(100.0);
        toast.dismiss();
        assert!(!toast.is_visible(101.0));
        assert_eq!(toast.remaining(101.0), 0.0);
    }
}
