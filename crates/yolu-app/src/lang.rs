//! 画面の言葉（日本語と英語）。文言は呼ぶ場所に `lang.pick("日本語", "English")` と並べて置く（名前・状態・短い理由だけ。説明はツールチップ）。
//! 状態は `AppState::lang`。試験は同じ画面で言語を替えて文言を確かめる。

/// 画面の言語。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
    #[default]
    Ja,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ja, Lang::En];

    /// 言語ごとの値から今の言語のものを選ぶ。
    pub fn pick<T>(self, ja: T, en: T) -> T {
        match self {
            Lang::Ja => ja,
            Lang::En => en,
        }
    }

    /// 言語の自分の言葉での名前（言語の選択肢に出す）。
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ja => "日本語",
            Lang::En => "English",
        }
    }

    /// 断り・失敗の文: 「何が」の文に理由を括弧で添えた 1 つの文（「アセットに保存できません（アセットがいっぱいです）。」・
    /// "Cannot save to the project's assets (assets are full)."）。「できません: 理由」のように、文と文をコロンでつながない。
    /// 理由の終わりの句点は外す（英語の理由の頭の大文字は残す。"Krita …" のような名前を小文字にしないため）。理由が空なら「何が」だけの文。
    pub fn with_reason(self, what: impl AsRef<str>, reason: impl AsRef<str>) -> String {
        let what = what.as_ref().trim_end_matches(['。', '.']);
        let reason = reason.as_ref().trim().trim_end_matches(['。', '.']);
        match (self, reason.is_empty()) {
            (Lang::Ja, true) => format!("{what}。"),
            (Lang::En, true) => format!("{what}."),
            (Lang::Ja, false) => format!("{what}（{reason}）。"),
            (Lang::En, false) => format!("{what} ({reason})."),
        }
    }

    /// テクスチャセットの中で起きたことの文（「テクスチャセット「A」で、選択範囲を読めません（…）。」・
    /// `In texture set "A", cannot read the selection (…).`）。
    pub fn in_set(self, set: &str, sentence: &str) -> String {
        let set = self.quote(set);
        match self {
            Lang::Ja => format!("テクスチャセット{set}で、{sentence}"),
            Lang::En => format!("In texture set {set}, {}", lower_first(sentence)),
        }
    }

    /// 読めなかったがファイルには残っていることを添えた文（「〜を読めません（理由）。ファイルには残っています。」）。
    pub fn kept_in_file(self, sentence: String) -> String {
        sentence + self.pick("ファイルには残っています。", " It is kept in the file.")
    }

    /// 文の中に置く名前（「名前」・"name"。英語の文は ASCII の引用符で、日本語が混じらない限り ASCII のまま）。
    pub fn quote(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("「{name}」"),
            Lang::En => format!("\"{name}\""),
        }
    }
}

/// 英語の文の頭の大文字を小文字にする（文の途中に続けるとき。続く字が小文字のときだけ: "Cannot read …" → "cannot read …"、"PNG …" はそのまま）。
fn lower_first(text: &str) -> std::borrow::Cow<'_, str> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second))
            if first.is_ascii_uppercase() && second.is_ascii_lowercase() =>
        {
            std::borrow::Cow::Owned(format!("{}{}", first.to_ascii_lowercase(), &text[1..]))
        }
        _ => std::borrow::Cow::Borrowed(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_language() {
        assert_eq!(Lang::Ja.pick("あ", "a"), "あ");
        assert_eq!(Lang::En.pick("あ".to_string(), "a".to_string()), "a");
        assert_eq!(Lang::default(), Lang::Ja);
    }

    /// 断り・失敗の文は「何が（なぜ）」の 1 つの文。理由の句点は重ねない。
    #[test]
    fn a_reason_joins_what_failed_in_one_sentence() {
        assert_eq!(
            Lang::Ja.with_reason("アセットに保存できません", "アセットがいっぱいです。"),
            "アセットに保存できません（アセットがいっぱいです）。"
        );
        assert_eq!(
            Lang::En.with_reason("Cannot save to the project's assets.", "Assets are full."),
            "Cannot save to the project's assets (Assets are full)."
        );
        assert_eq!(
            Lang::En.in_set("A", "Cannot read the selection (Invalid data)."),
            "In texture set \"A\", cannot read the selection (Invalid data)."
        );
        assert_eq!(
            Lang::Ja.in_set("A", "選択範囲を読めません。"),
            "テクスチャセット「A」で、選択範囲を読めません。"
        );
        assert_eq!(
            Lang::Ja.kept_in_file("ポーズを読めません（壊れています）。".into()),
            "ポーズを読めません（壊れています）。ファイルには残っています。"
        );
        assert_eq!(Lang::Ja.with_reason("開けません", " "), "開けません。");
        assert_eq!(Lang::Ja.quote("a"), "「a」");
        assert_eq!(Lang::En.quote("a"), "\"a\"");
    }
}

mod errors;
pub mod refusals;
pub(crate) use errors::budget_text;
pub use errors::{
    brush_import_error, library_io_error, library_known_error, path_error, psd_copy_refusal,
    psd_copy_refusal_tooltip, rebind_error, shelf_io_error,
};
pub(crate) use errors::{
    hide_preset_save_error, inactive_effect_reason, pose_preset_save_error, update_failure,
    update_start_failure,
};
