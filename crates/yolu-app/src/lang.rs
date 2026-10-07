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

impl From<yolu_ops::Lang> for Lang {
    fn from(lang: yolu_ops::Lang) -> Self {
        match lang {
            yolu_ops::Lang::Ja => Lang::Ja,
            yolu_ops::Lang::En => Lang::En,
        }
    }
}

/// 初めて起動したときの言語（設定のファイルに言語がまだ無いときだけ使う）。OS の言語が日本語なら日本語、それ以外と分からないときは英語。
pub fn system_lang() -> Lang {
    system_lang_from(|name| std::env::var(name).ok(), os_ui_language())
}

/// `system_lang` の中身。順に見る: 環境変数 `YOLUPAINTER_LANG`（明示の上書き）→ OS の表示言語（`os_ui_language`。Windows だけ）→
/// ない OS は `LC_ALL`・`LC_MESSAGES`・`LANG`（`ja` で始まれば日本語）。どれでも分からなければ英語。
/// Windows は、表示言語が日本語以外ならロケールの環境変数（Git Bash などが持ち込む）を見ずに英語にする。
pub fn system_lang_from(get: impl Fn(&str) -> Option<String>, os_ui: Option<u16>) -> Lang {
    let found = yolu_ops::Lang::from_override(&get).or_else(|| match os_ui {
        Some(id) => Some(os_ui_lang(id)),
        None => yolu_ops::Lang::from_locale_vars(&get),
    });
    found.map_or(Lang::En, Lang::from)
}

/// Windows の言語 ID（`LANGID`）の主言語（下位 10 ビット）が日本語か。
fn os_ui_lang(id: u16) -> yolu_ops::Lang {
    const LANG_JAPANESE: u16 = 0x11;
    if id & 0x3ff == LANG_JAPANESE {
        yolu_ops::Lang::Ja
    } else {
        yolu_ops::Lang::En
    }
}

/// OS の表示言語の言語 ID（Windows の `GetUserDefaultUILanguage`）。Windows 以外は None。
#[cfg(windows)]
fn os_ui_language() -> Option<u16> {
    // SAFETY: 引数なしで値を返すだけの関数
    Some(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() })
}

#[cfg(not(windows))]
fn os_ui_language() -> Option<u16> {
    None
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

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    /// 初めての起動の言語（Windows 以外）: 上書きが先、`LC_ALL` が `LANG` より先、`ja` で始まれば日本語。
    /// `C`・空・知らない言語・手がかりが無いときは英語。
    #[test]
    fn the_first_language_follows_the_environment() {
        let lang = |pairs| system_lang_from(env(pairs), None);
        assert_eq!(lang(&[("LANG", "ja_JP.UTF-8")]), Lang::Ja);
        assert_eq!(lang(&[("LANG", "ja")]), Lang::Ja);
        assert_eq!(lang(&[("LANG", "en_US.UTF-8")]), Lang::En);
        assert_eq!(
            lang(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "ja_JP.UTF-8")]),
            Lang::En
        );
        assert_eq!(
            lang(&[("LC_ALL", "ja_JP.UTF-8"), ("LANG", "en_US.UTF-8")]),
            Lang::Ja
        );
        assert_eq!(
            lang(&[("LC_MESSAGES", "ja_JP.UTF-8"), ("LANG", "en_US")]),
            Lang::Ja
        );
        assert_eq!(
            lang(&[("YOLUPAINTER_LANG", "en"), ("LC_ALL", "ja_JP.UTF-8")]),
            Lang::En
        );
        assert_eq!(
            lang(&[("YOLUPAINTER_LANG", "ja"), ("LANG", "en_US.UTF-8")]),
            Lang::Ja
        );
        for unknown in ["C", "C.UTF-8", "POSIX", "", "zh_CN.UTF-8", "fr_FR"] {
            let value = unknown.to_owned();
            let get = move |name: &str| (name == "LANG").then(|| value.clone());
            assert_eq!(system_lang_from(get, None), Lang::En, "{unknown:?}");
        }
        assert_eq!(lang(&[]), Lang::En, "手がかりが無ければ英語");
        // 上書きが読めない値のときは、その先を見る
        assert_eq!(
            lang(&[("YOLUPAINTER_LANG", "xx"), ("LANG", "ja_JP.UTF-8")]),
            Lang::Ja
        );
    }

    /// Windows は、OS の表示言語の主言語が日本語なら日本語、それ以外（英語・中国語・0）は英語。ロケールの環境変数は見ない。
    /// 明示の上書きだけが OS の言語より先。
    #[test]
    fn on_windows_the_display_language_decides() {
        let lang = |pairs, id| system_lang_from(env(pairs), Some(id));
        assert_eq!(lang(&[], 0x0411), Lang::Ja, "ja-JP");
        assert_eq!(lang(&[], 0x0409), Lang::En, "en-US");
        assert_eq!(lang(&[], 0x0809), Lang::En, "en-GB");
        assert_eq!(lang(&[], 0x0804), Lang::En, "zh-CN");
        assert_eq!(lang(&[], 0x0404), Lang::En, "zh-TW");
        assert_eq!(lang(&[], 0x0000), Lang::En, "中立");
        // 主言語は下位 10 ビット（上位は副言語）
        assert_eq!(lang(&[], 0x0011), Lang::Ja);
        assert_eq!(lang(&[], 0x0009), Lang::En);
        // ロケールの環境変数は見ない
        assert_eq!(lang(&[("LANG", "ja_JP.UTF-8")], 0x0409), Lang::En);
        assert_eq!(lang(&[("LC_ALL", "en_US")], 0x0411), Lang::Ja);
        // 上書きは OS の言語より先（読めない値は飛ばす）
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "en")], 0x0411), Lang::En);
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "ja")], 0x0409), Lang::Ja);
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "xx")], 0x0411), Lang::Ja);
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
