//! 画面の言葉。文は `locales/<タグ>/<モジュール>.ftl` に置き、呼ぶ場所はキーで引く（`lang.t("キー")`・引数のある文は
//! `lang.f("キー", &[("名前", &値)])`）。名前・状態・短い理由だけ。説明はツールチップ（キーの終わりが `-tip`）。
//! 言語ごとの名前・書式・フォントは言語の表（`TABLE`）にあり、訳の無いキーは英語の文になる。
//! 状態は `AppState::lang`。試験は同じ画面で言語を替えて文言を確かめる。

/// 画面の言語。並びは言語の表（`TABLE`）と `yolu_ops::Lang` と同じ。言語を足すときは、ここと表と `yolu_ops::Lang` に 1 行ずつ足し、
/// `locales/<タグ>/` に .ftl を置く（フォルダの無い言語は選べない。[`Lang::ALL`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
    #[default]
    Ja,
    En,
    ZhTw,
    ZhCn,
    Ko,
}

/// 言語の表の 1 行: 名前・書式・フォント。タグと OS の言語の見分けは `yolu_ops::Lang`（CLI と同じ表）。
pub struct Info {
    /// 自分の言葉での名前（言語の選択肢に出す）。
    pub name: &'static str,
    /// 文の中に置く名前の引用の印（開き・閉じ）。
    quote: [&'static str; 2],
    /// 断り・失敗の理由を添える括弧（開き・閉じ。前の空白も含む）。
    paren: [&'static str; 2],
    /// 文の終わりの句点。
    stop: &'static str,
    /// 文を別の文の途中に続けるとき、頭の大文字を小文字にするか（英語）。
    lower_first: bool,
    /// その言語のフォント（読むのは `ui::fonts`。同梱の BIZ UDPGothic より先に置く。None は同梱のフォントだけ）。
    pub font: Option<&'static font::LangFont>,
}

/// 言語の表（[`Lang`] の並び）。
const TABLE: [Info; 5] = [
    Info {
        name: "日本語",
        quote: ["「", "」"],
        paren: ["（", "）"],
        stop: "。",
        lower_first: false,
        font: None,
    },
    Info {
        name: "English",
        quote: ["\"", "\""],
        paren: [" (", ")"],
        stop: ".",
        lower_first: true,
        font: None,
    },
    Info {
        name: "繁體中文",
        quote: ["「", "」"],
        paren: ["（", "）"],
        stop: "。",
        lower_first: false,
        font: Some(&font::NOTO_SANS_TC),
    },
    Info {
        name: "简体中文",
        quote: ["“", "”"],
        paren: ["（", "）"],
        stop: "。",
        lower_first: false,
        font: Some(&font::NOTO_SANS_SC),
    },
    Info {
        name: "한국어",
        // 訳が入るときに、訳の使い方に合わせて決め直す
        quote: ["\"", "\""],
        paren: [" (", ")"],
        stop: ".",
        lower_first: false,
        font: Some(&font::NOTO_SANS_KR),
    },
];

/// 文字列が同じか（const の中で使う）。
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `locales/` に .ftl のフォルダがある言語（表の並び）。表に行の無いフォルダはコンパイルの時に断る。
const fn available() -> [Lang; catalog::LOCALE_COUNT] {
    let mut out = [Lang::En; catalog::LOCALE_COUNT];
    let mut found = 0;
    let mut i = 0;
    while i < Lang::EVERY.len() {
        let lang = Lang::EVERY[i];
        if catalog::has_locale(lang.ops().tag()) {
            out[found] = lang;
            found += 1;
        }
        i += 1;
    }
    assert!(
        found == catalog::LOCALE_COUNT,
        "locales/ に言語の表（lang.rs）に無いフォルダがある"
    );
    out
}

impl Lang {
    /// 選べる言語（`locales/<タグ>/` のある言語。表の並び）。
    pub const ALL: [Lang; catalog::LOCALE_COUNT] = available();
    /// 表の全部の言語（文のフォルダの有無を問わない）。
    const EVERY: [Lang; 5] = [Lang::Ja, Lang::En, Lang::ZhTw, Lang::ZhCn, Lang::Ko];
    /// 日本語と英語（テストが日英の文を [`Lang::pick`] で比べるときの並び）。
    pub const JA_EN: [Lang; 2] = [Lang::Ja, Lang::En];

    /// 言語の表の行。
    pub fn info(self) -> &'static Info {
        &TABLE[self as usize]
    }

    /// 同じ並びの `yolu_ops::Lang`（タグ・CLI と MCP の文）。
    pub const fn ops(self) -> yolu_ops::Lang {
        match self {
            Lang::Ja => yolu_ops::Lang::Ja,
            Lang::En => yolu_ops::Lang::En,
            Lang::ZhTw => yolu_ops::Lang::ZhTw,
            Lang::ZhCn => yolu_ops::Lang::ZhCn,
            Lang::Ko => yolu_ops::Lang::Ko,
        }
    }

    /// 言語のタグ（`ja`・`en`・`zh-TW`・`zh-CN`・`ko`。設定のファイルに書く値）。
    pub fn tag(self) -> &'static str {
        self.ops().tag()
    }

    /// タグから、選べる言語（[`Lang::ALL`]）だけ。前の設定の `ja`・`en` もそのまま読める。
    pub fn from_tag(tag: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|lang| lang.tag() == tag)
    }

    /// 選べる言語か（`locales/<タグ>/` がある）。
    pub fn is_available(self) -> bool {
        Lang::ALL.contains(&self)
    }

    /// 日本語の値か、それ以外（英語）の値。テストと下書き（`yolu-mock`）が日英の文を直に比べる・書くための物で、本番の画面の文は
    /// .ftl のキー（`t`・`f`）で引く（ほかの言語で英語になるため）。
    pub fn pick<T>(self, ja: T, en: T) -> T {
        self.ops().pick(ja, en)
    }

    /// 日本語の画面は core・yolu-io の日本語の文（診断の文。.ftl に写していない）、ほかの言語は英語の文。
    pub fn core_text<T>(self, japanese: T, english: T) -> T {
        if self == Lang::Ja {
            japanese
        } else {
            english
        }
    }

    /// 画面の文（`locales/<言語>/<モジュール>.ftl` のキー）。引数の無い文。その言語に無いキーは英語の文。
    pub fn t(self, key: &'static str) -> &'static str {
        catalog::text(self, key)
    }

    /// 引数のある文。引数は Rust で書式を済ませた値（`Display`）で渡し、英語の複数形を選ぶ数だけは [`Count`] で渡す。
    /// その言語に無いキーは英語の文。
    pub fn f(self, key: &'static str, args: &[(&'static str, &dyn Arg)]) -> String {
        catalog::format_with(self, key, args)
    }

    /// キーを実行の時に組み立てる文（表の識別子からキーを作る所）。無いキーは None（英語にも無いとき）。
    pub fn lookup(self, key: &str) -> Option<&'static str> {
        catalog::lookup_or_english(self, key)
    }

    /// core の断りの理由の文（キーは [`reason_key`]）。
    pub fn reason(self, reason: yolu_core::Reason) -> &'static str {
        catalog::reason(self, reason)
    }

    /// 言語の自分の言葉での名前（言語の選択肢に出す）。
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// 断り・失敗の文: 「何が」の文に理由を括弧で添えた 1 つの文（「アセットに保存できません（アセットがいっぱいです）。」・
    /// "Cannot save to the project's assets (assets are full)."）。「できません: 理由」のように、文と文をコロンでつながない。
    /// 理由の終わりの句点は外す（英語の理由の頭の大文字は残す。"Krita …" のような名前を小文字にしないため）。理由が空なら「何が」だけの文。
    /// 括弧と句点は言語の表の物。
    pub fn with_reason(self, what: impl AsRef<str>, reason: impl AsRef<str>) -> String {
        let info = self.info();
        let what = what.as_ref().trim_end_matches(['。', '.']);
        let reason = reason.as_ref().trim().trim_end_matches(['。', '.']);
        if reason.is_empty() {
            format!("{what}{}", info.stop)
        } else {
            let [open, close] = info.paren;
            format!("{what}{open}{reason}{close}{}", info.stop)
        }
    }

    /// テクスチャセットの中で起きたことの文（「テクスチャセット「A」で、選択範囲を読めません（…）。」・
    /// `In texture set "A", cannot read the selection (…).`）。続く文の頭は、英語では小文字にする。
    pub fn in_set(self, set: &str, sentence: &str) -> String {
        let sentence = if self.info().lower_first {
            lower_first(sentence)
        } else {
            std::borrow::Cow::Borrowed(sentence)
        };
        self.f(
            "lang-in-texture-set",
            &[("set", &self.quote(set)), ("sentence", &sentence)],
        )
    }

    /// 読めなかったがファイルには残っていることを添えた文（「〜を読めません（理由）。ファイルには残っています。」）。
    pub fn kept_in_file(self, sentence: String) -> String {
        sentence + self.t("lang-it-is-kept-in-the")
    }

    /// 文の途中に置く道具・項目の名前（英語は全部を小文字にする: "Brush" → "brush"。ほかの言語はそのまま）。
    pub fn name_in_sentence(self, name: &str) -> String {
        if self.info().lower_first {
            name.to_lowercase()
        } else {
            name.to_owned()
        }
    }

    /// 文の中に置く名前（「名前」・"name"。印は言語の表の物で、英語は ASCII の引用符）。
    pub fn quote(self, name: &str) -> String {
        let [open, close] = self.info().quote;
        format!("{open}{name}{close}")
    }
}

/// 選べる言語ごとの文（裏のスレッドで作り、後で画面の言語で選ぶ物）。無い言語は英語。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texts(Vec<(Lang, String)>);

impl Texts {
    /// 選べる言語の全部で `text` を呼んで作る。
    pub fn by_lang(text: impl Fn(Lang) -> String) -> Texts {
        Texts(Lang::ALL.into_iter().map(|l| (l, text(l))).collect())
    }

    /// その言語の文（無ければ英語）。
    pub fn get(&self, lang: Lang) -> &str {
        let find = |l: Lang| self.0.iter().find(|(t, _)| *t == l);
        find(lang)
            .or_else(|| find(Lang::En))
            .or(self.0.first())
            .map_or("", |(_, text)| text)
    }
}

impl From<yolu_ops::Lang> for Lang {
    fn from(lang: yolu_ops::Lang) -> Self {
        Lang::EVERY[lang as usize]
    }
}

/// 初めて起動したときの言語（設定のファイルに言語がまだ無いときだけ使う）。OS の言語が選べる言語ならそれ、それ以外と分からないときは英語。
pub fn system_lang() -> Lang {
    system_lang_from(|name| std::env::var(name).ok(), os_language())
}

/// OS が教える言語（環境変数のほか）。
#[derive(Clone, Debug)]
pub enum OsLanguage {
    /// Windows の表示言語の ID（`GetUserDefaultUILanguage`）。
    Windows(u16),
    /// Mac の優先する言語の一覧（`NSLocale.preferredLanguages`。先が先）。
    Preferred(Vec<String>),
}

/// `system_lang` の中身。順に見る: 環境変数 `YOLUPAINTER_LANG`（明示の上書き）→ OS の言語（Windows は表示言語の ID、Mac は優先する言語の
/// 一覧を上から見て選べる最初の言語）→ どちらも無い OS（Linux）は `LC_ALL`・`LC_MESSAGES`・`LANG`。見分けた言語が選べない（`locales/` に無い）か、どれでも分からなければ英語。
/// Windows と Mac は、OS の言語が分からなくてもロケールの環境変数（Git Bash・端末が持ち込む）を見ずに英語にする。
pub fn system_lang_from(get: impl Fn(&str) -> Option<String>, os: Option<OsLanguage>) -> Lang {
    let available = |lang: yolu_ops::Lang| Some(Lang::from(lang)).filter(|l| l.is_available());
    if let Some(lang) = yolu_ops::Lang::from_override(&get).and_then(available) {
        return lang;
    }
    match os {
        Some(OsLanguage::Windows(id)) => yolu_ops::Lang::from_windows_id(id)
            .and_then(available)
            .unwrap_or(Lang::En),
        // 一覧を上から見て、YoluPainter が持つ最初の言語（持たない言語は飛ばす）
        Some(OsLanguage::Preferred(list)) => list
            .iter()
            .find_map(|name| yolu_ops::Lang::parse(name).and_then(available))
            .unwrap_or(Lang::En),
        None => yolu_ops::Lang::from_locale_vars(&get)
            .and_then(available)
            .unwrap_or(Lang::En),
    }
}

/// OS の表示言語の言語 ID（Windows の `GetUserDefaultUILanguage`）。
#[cfg(windows)]
fn os_language() -> Option<OsLanguage> {
    // SAFETY: 引数なしで値を返すだけの関数
    Some(OsLanguage::Windows(unsafe {
        windows::Win32::Globalization::GetUserDefaultUILanguage()
    }))
}

/// Mac の優先する言語の一覧（GUI のアプリは `LANG` を持たないことがあるので、環境変数でなく OS に聞く）。
#[cfg(target_os = "macos")]
fn os_language() -> Option<OsLanguage> {
    // 自動解放される物（一覧・文字列）は、このプールの中で Rust の文字列へ写して手放す
    objc2::rc::autoreleasepool(|_| {
        let list = objc2_foundation::NSLocale::preferredLanguages();
        Some(OsLanguage::Preferred(
            list.to_vec().iter().map(|name| name.to_string()).collect(),
        ))
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_language() -> Option<OsLanguage> {
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

    /// `Lang::reason` が、本物の `Reason` から `.ftl` のキーを引いて、日英の文を返す。
    #[test]
    fn reason_comes_from_the_ftl() {
        use yolu_core::Reason;
        assert_eq!(
            Lang::En.reason(Reason::ColorBalanceRange),
            "Color balance (−100–100)"
        );
        assert_eq!(
            Lang::Ja.reason(Reason::ColorBalanceRange),
            "カラーバランス（−100〜100）"
        );
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
        assert_eq!(lang(&[("LANG", "zh_TW.UTF-8")]), Lang::ZhTw);
        assert_eq!(lang(&[("LANG", "zh_HK.UTF-8")]), Lang::ZhTw);
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "zh-TW")]), Lang::ZhTw);
        assert_eq!(lang(&[("LANG", "zh_CN.UTF-8")]), Lang::ZhCn);
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "zh-CN")]), Lang::ZhCn);
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
        for unknown in ["C", "C.UTF-8", "POSIX", "", "fr_FR"] {
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
        let lang = |pairs, id| system_lang_from(env(pairs), Some(OsLanguage::Windows(id)));
        assert_eq!(lang(&[], 0x0411), Lang::Ja, "ja-JP");
        assert_eq!(lang(&[], 0x0409), Lang::En, "en-US");
        assert_eq!(lang(&[], 0x0809), Lang::En, "en-GB");
        assert_eq!(lang(&[], 0x0404), Lang::ZhTw, "zh-TW");
        assert_eq!(lang(&[], 0x0C04), Lang::ZhTw, "zh-HK");
        assert_eq!(lang(&[], 0x1404), Lang::ZhTw, "zh-MO");
        // 訳のフォルダの無い言語は英語（フォルダを足すと、その言語になる）
        for (id, ops) in [
            (0x0804, yolu_ops::Lang::ZhCn),
            (0x1004, yolu_ops::Lang::ZhCn),
            (0x0412, yolu_ops::Lang::Ko),
        ] {
            let expected = Some(Lang::from(ops))
                .filter(|l| l.is_available())
                .unwrap_or(Lang::En);
            assert_eq!(lang(&[], id), expected, "{id:#06x}");
        }
        assert_eq!(lang(&[], 0x040C), Lang::En, "fr-FR");
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

    /// 繁體中文の書式: 「」の引用・全角の括弧・。の句点。続く文の頭は小文字にしない。
    #[test]
    fn traditional_chinese_formats_like_the_table() {
        let zh = Lang::ZhTw;
        assert_eq!(zh.quote("A"), "「A」");
        assert_eq!(
            zh.with_reason("無法儲存。", "資源已滿。"),
            "無法儲存（資源已滿）。"
        );
        assert_eq!(zh.with_reason("無法儲存", ""), "無法儲存。");
        assert_eq!(
            zh.in_set("A", "Krita 的檔案無法讀取。"),
            "在紋理集「A」中，Krita 的檔案無法讀取。"
        );
        assert_eq!(zh.name(), "繁體中文");
        assert_eq!(zh.tag(), "zh-TW");
    }

    /// Mac の優先する言語の一覧: 読める先頭の言語。選べない言語・知らない言語は英語。環境変数は見ない。
    #[test]
    fn on_mac_the_preferred_languages_decide() {
        let lang = |pairs, list: &[&str]| {
            system_lang_from(
                env(pairs),
                Some(OsLanguage::Preferred(
                    list.iter().map(|s| s.to_string()).collect(),
                )),
            )
        };
        assert_eq!(lang(&[], &["ja-JP", "en-US"]), Lang::Ja);
        assert_eq!(lang(&[], &["fr-FR", "zh-Hant-TW"]), Lang::ZhTw);
        assert_eq!(lang(&[], &["en-GB", "ja-JP"]), Lang::En);
        assert_eq!(lang(&[], &["de-DE"]), Lang::En);
        assert_eq!(lang(&[], &["de"]), Lang::En);
        // 一覧の上から、選べる最初の言語（先頭が選べない言語でも、次を見る）
        assert_eq!(lang(&[], &["zh-Hans-CN", "ja-JP"]), Lang::ZhCn);
        assert_eq!(lang(&[], &["ko-KR", "ja-JP"]), Lang::Ja);
        assert_eq!(lang(&[], &["fr-FR", "ja-JP"]), Lang::Ja);
        assert_eq!(lang(&[], &["zh-Hant-TW", "ja-JP"]), Lang::ZhTw);
        assert_eq!(lang(&[], &["fr-FR", "zh-Hans-CN"]), Lang::ZhCn);
        assert_eq!(lang(&[("LANG", "ja_JP.UTF-8")], &[]), Lang::En);
        assert_eq!(lang(&[("YOLUPAINTER_LANG", "ja")], &["en-US"]), Lang::Ja);
    }

    /// 表の行は `Lang` の並びのとおり（行が入れ替わると、名前・書式・フォントが別の言語の物になる）。
    #[test]
    fn each_table_row_belongs_to_its_language() {
        assert_eq!(Lang::EVERY.len(), TABLE.len());
        for (lang, tag, name, quote) in [
            (Lang::Ja, "ja", "日本語", "「"),
            (Lang::En, "en", "English", "\""),
            (Lang::ZhTw, "zh-TW", "繁體中文", "「"),
            (Lang::ZhCn, "zh-CN", "简体中文", "“"),
            (Lang::Ko, "ko", "한국어", "\""),
        ] {
            assert_eq!(Lang::EVERY[lang as usize], lang);
            assert_eq!(lang.tag(), tag);
            assert_eq!(lang.name(), name);
            assert_eq!(lang.quote("x").chars().next().unwrap().to_string(), quote);
            assert_eq!(Lang::from(lang.ops()), lang);
        }
        // フォントは言語の表の行にだけある（日本語・英語は同梱のフォント）
        assert_eq!(Lang::Ja.info().font, None);
        assert_eq!(Lang::En.info().font, None);
        assert_eq!(Lang::ZhTw.info().font, Some(&font::NOTO_SANS_TC));
        assert_eq!(Lang::ZhCn.info().font, Some(&font::NOTO_SANS_SC));
        assert_eq!(Lang::Ko.info().font, Some(&font::NOTO_SANS_KR));
    }

    /// 表: 選べる言語は `locales/` のフォルダの言語で、日本語・英語・繁體中文・简体中文がある。タグで引ける。
    #[test]
    fn the_table_lists_the_languages_with_texts() {
        for lang in [Lang::Ja, Lang::En, Lang::ZhTw, Lang::ZhCn] {
            assert!(Lang::ALL.contains(&lang), "{lang:?}");
            assert_eq!(Lang::from_tag(lang.tag()), Some(lang));
        }
        assert_eq!(Lang::from_tag("xx"), None);
        for lang in Lang::EVERY {
            assert_eq!(Lang::from(lang.ops()), lang);
        }
    }
}

mod catalog;
mod errors;
pub mod font;
pub mod refusals;
pub use catalog::{reason_key, Arg, Count};
pub use errors::{
    brush_import_error, library_io_error, library_known_error, path_error, psd_copy_refusal,
    psd_copy_refusal_tooltip, rebind_error, shelf_io_error,
};
pub(crate) use errors::{
    hide_preset_save_error, inactive_effect_reason, pose_preset_save_error, update_failure,
    update_start_failure,
};
