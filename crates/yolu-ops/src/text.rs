//! 言語の識別（タグ・OS の言語の見分け）と、日本語と英語の文。誤りの理由・知らせは日本語と英語の文を持ち、CLI は `--lang`、アプリは
//! 自分の言語で選ぶ。日本語と英語のほかの言語（`zh-TW` など）の文はアプリの画面の .ftl だけにあり、ここの文はそれらの言語では英語になる。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 言語。並びはアプリの言語の表（`yolu-app` の `lang.rs`）と同じ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Lang {
    #[default]
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ko")]
    Ko,
}

/// 言語を見分ける手がかり（表の 1 行）。
struct Id {
    lang: Lang,
    /// 設定・`--lang`・`YOLUPAINTER_LANG` に書くタグ（BCP 47）。
    tag: &'static str,
    /// ロケールの名前の頭（小文字・`_` を `-` に替えた形。`zh-hant` は `zh-Hant-TW` のような Mac の書き方）。
    /// いちばん長く合った物を選ぶ（`zh-tw` が `zh` より先）。
    prefixes: &'static [&'static str],
    /// Windows の言語 ID（`LANGID`）で、ぴったり合う物。
    windows_ids: &'static [u16],
    /// Windows の言語 ID の主言語（下位 10 ビット）。`windows_ids` に合わなかったときに見る。
    windows_primary: Option<u16>,
}

const IDS: [Id; 5] = [
    Id {
        lang: Lang::Ja,
        tag: "ja",
        prefixes: &["ja"],
        windows_ids: &[],
        windows_primary: Some(0x11),
    },
    Id {
        lang: Lang::En,
        tag: "en",
        prefixes: &["en"],
        windows_ids: &[],
        windows_primary: Some(0x09),
    },
    Id {
        lang: Lang::ZhTw,
        tag: "zh-TW",
        prefixes: &["zh-tw", "zh-hk", "zh-mo", "zh-hant"],
        // 台湾・香港・マカオ・繁体字（中立）
        windows_ids: &[0x0404, 0x0C04, 0x1404, 0x7C04],
        windows_primary: None,
    },
    Id {
        lang: Lang::ZhCn,
        tag: "zh-CN",
        prefixes: &["zh-cn", "zh-sg", "zh-hans", "zh"],
        // 中国・シンガポール・簡体字（中立）
        windows_ids: &[0x0804, 0x1004, 0x0004],
        // ほかの中国語（副言語の分からない物）は簡体字
        windows_primary: Some(0x04),
    },
    Id {
        lang: Lang::Ko,
        tag: "ko",
        prefixes: &["ko"],
        windows_ids: &[],
        windows_primary: Some(0x12),
    },
];

impl Lang {
    /// 表の並びの全部の言語（文の有無を問わない。アプリで選べる言語はアプリの `Lang::ALL`）。
    pub const ALL: [Lang; 5] = [Lang::Ja, Lang::En, Lang::ZhTw, Lang::ZhCn, Lang::Ko];

    const fn id(self) -> &'static Id {
        &IDS[self as usize]
    }

    /// 言語のタグ（`ja`・`en`・`zh-TW`・`zh-CN`・`ko`）。
    pub const fn tag(self) -> &'static str {
        self.id().tag
    }

    /// タグ・ロケールの名前から（大文字小文字・`_` と `-`・`.UTF-8` のような符号化の印・`@` の後ろは問わない。`ja-JP`・`zh_TW.UTF-8`・
    /// `zh-Hant-HK`・`ko-KR` など）。いちばん長く合った頭の言語。分からなければ None。
    pub fn parse(text: &str) -> Option<Lang> {
        let name = text
            .split(['.', '@'])
            .next()?
            .trim()
            .replace('_', "-")
            .to_ascii_lowercase();
        IDS.iter()
            .flat_map(|id| id.prefixes.iter().map(move |p| (id.lang, *p)))
            .filter(|(_, prefix)| {
                name == *prefix
                    || name
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('-'))
            })
            .max_by_key(|(_, prefix)| prefix.len())
            .map(|(lang, _)| lang)
    }

    /// Windows の言語 ID（`GetUserDefaultUILanguage`）から。ぴったり合う ID が先、次に主言語。分からなければ None。
    pub fn from_windows_id(id: u16) -> Option<Lang> {
        IDS.iter()
            .find(|row| row.windows_ids.contains(&id))
            .or_else(|| {
                IDS.iter()
                    .find(|row| row.windows_primary == Some(id & 0x3ff))
            })
            .map(|row| row.lang)
    }

    /// 優先する言語の一覧（Mac の `NSLocale.preferredLanguages`。先が先）の、読める先頭の言語。
    pub fn from_preferred(list: &[impl AsRef<str>]) -> Option<Lang> {
        list.iter().find_map(|name| Lang::parse(name.as_ref()))
    }

    /// 日本語か、それ以外（英語）か。日本語と英語の 2 つの文を持つ物（[`Text`] など）を選ぶ。
    pub fn pick<T>(self, ja: T, en: T) -> T {
        match self {
            Lang::Ja => ja,
            _ => en,
        }
    }

    /// 環境変数の言語（`YOLUPAINTER_LANG`・`LC_ALL`・`LC_MESSAGES`・`LANG` の順。`C`・空・知らない言語は手がかりにせず、次の変数を見る）。
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Option<Lang> {
        Self::from_names(&get, &[OVERRIDE_VAR]).or_else(|| Self::from_locale_vars(get))
    }

    /// 明示の上書き（`YOLUPAINTER_LANG`）だけ。
    pub fn from_override(get: impl Fn(&str) -> Option<String>) -> Option<Lang> {
        Self::from_names(&get, &[OVERRIDE_VAR])
    }

    /// OS のロケールの環境変数（`LC_ALL`・`LC_MESSAGES`・`LANG` の順）だけ。
    pub fn from_locale_vars(get: impl Fn(&str) -> Option<String>) -> Option<Lang> {
        Self::from_names(&get, &LOCALE_VARS)
    }

    fn from_names(get: &impl Fn(&str) -> Option<String>, names: &[&str]) -> Option<Lang> {
        names
            .iter()
            .filter_map(|name| get(name).filter(|v| !v.is_empty()))
            .find_map(|v| Lang::parse(&v))
    }
}

/// 言語を決める明示の上書きの環境変数。
const OVERRIDE_VAR: &str = "YOLUPAINTER_LANG";
/// OS のロケールの環境変数（先の物が先）。
const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// 日本語と英語の同じ意味の文。ほかの言語では英語（[`Text::pick`]）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Text {
    pub ja: String,
    pub en: String,
}

impl Text {
    pub fn new(ja: impl Into<String>, en: impl Into<String>) -> Self {
        Text {
            ja: ja.into(),
            en: en.into(),
        }
    }
    /// その言語の文。日本語のほかは英語。
    pub fn pick(&self, lang: Lang) -> &str {
        lang.pick(&self.ja, &self.en)
    }
}

impl std::fmt::Display for Text {
    /// 英語（ログ・診断用。言語を選ぶ所は `pick`）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.en)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    /// 環境変数の言語は、上書き → `LC_ALL` → `LC_MESSAGES` → `LANG` の順で、読めない値は飛ばす。
    #[test]
    fn variables_are_read_in_order_and_unreadable_values_are_skipped() {
        assert_eq!(Lang::from_vars(vars(&[])), None);
        assert_eq!(
            Lang::from_vars(vars(&[("LANG", "ja_JP.UTF-8")])),
            Some(Lang::Ja)
        );
        assert_eq!(
            Lang::from_vars(vars(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "ja_JP.UTF-8")])),
            Some(Lang::En)
        );
        assert_eq!(
            Lang::from_vars(vars(&[("LC_MESSAGES", "ja"), ("LANG", "en")])),
            Some(Lang::Ja)
        );
        assert_eq!(
            Lang::from_vars(vars(&[("YOLUPAINTER_LANG", "en"), ("LC_ALL", "ja_JP")])),
            Some(Lang::En)
        );
        // `C`・空・知らない言語は手がかりにならず、次の変数へ進む
        for unknown in ["C", "C.UTF-8", "POSIX", "", "fr_FR.UTF-8"] {
            let get = |name: &str| match name {
                "LC_ALL" => Some(unknown.to_owned()),
                "LANG" => Some("ja_JP".to_owned()),
                _ => None,
            };
            assert_eq!(Lang::from_vars(get), Some(Lang::Ja), "{unknown:?}");
        }
        assert_eq!(Lang::from_vars(vars(&[("LANG", "C.UTF-8")])), None);
    }

    /// 上書きだけ・ロケールだけを分けて読める（Windows は上書きのあと、OS の表示言語を見て、ロケールの変数は見ない）。
    #[test]
    fn the_override_and_the_locale_variables_can_be_read_apart() {
        let both = &[("YOLUPAINTER_LANG", "en"), ("LANG", "ja_JP.UTF-8")];
        assert_eq!(Lang::from_override(vars(both)), Some(Lang::En));
        assert_eq!(Lang::from_locale_vars(vars(both)), Some(Lang::Ja));
        assert_eq!(Lang::from_override(vars(&[("LANG", "ja_JP")])), None);
        assert_eq!(
            Lang::from_locale_vars(vars(&[("YOLUPAINTER_LANG", "ja")])),
            None
        );
    }

    /// タグ・ロケールの名前の見分け: 繁体字（台湾・香港・マカオ・Hant）と簡体字（中国・シンガポール・Hans・地域なし）、韓国語。
    #[test]
    fn locale_names_tell_the_chinese_scripts_and_korean_apart() {
        for (name, lang) in [
            ("ja", Lang::Ja),
            ("ja_JP.UTF-8", Lang::Ja),
            ("en-US", Lang::En),
            ("zh-TW", Lang::ZhTw),
            ("zh_TW.UTF-8", Lang::ZhTw),
            ("zh_HK.Big5", Lang::ZhTw),
            ("zh-MO", Lang::ZhTw),
            ("zh-Hant-HK", Lang::ZhTw),
            ("zh-Hant", Lang::ZhTw),
            ("zh-CN", Lang::ZhCn),
            ("zh_CN.UTF-8", Lang::ZhCn),
            ("zh_SG", Lang::ZhCn),
            ("zh-Hans-CN", Lang::ZhCn),
            ("zh", Lang::ZhCn),
            ("ko", Lang::Ko),
            ("ko_KR.UTF-8", Lang::Ko),
            ("ko-KR", Lang::Ko),
        ] {
            assert_eq!(Lang::parse(name), Some(lang), "{name}");
        }
        for name in ["", "C", "fr_FR", "zhx", "jap", "kor"] {
            assert_eq!(Lang::parse(name), None, "{name}");
        }
        for lang in Lang::ALL {
            assert_eq!(Lang::parse(lang.tag()), Some(lang));
        }
    }

    /// Windows の言語 ID: 中国語は副言語まで見る。日本語・韓国語・英語は主言語。ほかは分からない。
    #[test]
    fn windows_language_ids_are_told_apart() {
        for (id, lang) in [
            (0x0411, Some(Lang::Ja)),
            (0x0011, Some(Lang::Ja)),
            (0x0409, Some(Lang::En)),
            (0x0809, Some(Lang::En)),
            (0x0404, Some(Lang::ZhTw)),
            (0x0C04, Some(Lang::ZhTw)),
            (0x1404, Some(Lang::ZhTw)),
            (0x0804, Some(Lang::ZhCn)),
            (0x1004, Some(Lang::ZhCn)),
            (0x0412, Some(Lang::Ko)),
            (0x040C, None),
            (0x0000, None),
        ] {
            assert_eq!(Lang::from_windows_id(id), lang, "{id:#06x}");
        }
    }

    /// 優先する言語の一覧は、読める先頭の言語。
    #[test]
    fn the_first_readable_preferred_language_wins() {
        assert_eq!(
            Lang::from_preferred(&["fr-FR", "zh-Hant-TW", "en-US"]),
            Some(Lang::ZhTw)
        );
        assert_eq!(Lang::from_preferred(&["ko-KR", "ja-JP"]), Some(Lang::Ko));
        assert_eq!(Lang::from_preferred(&["de"]), None);
        assert_eq!(Lang::from_preferred(&[] as &[&str]), None);
    }

    /// 日本語と英語の文は、ほかの言語で英語。
    #[test]
    fn other_languages_read_the_english_text() {
        let text = Text::new("あ", "a");
        assert_eq!(text.pick(Lang::Ja), "あ");
        for lang in [Lang::En, Lang::ZhTw, Lang::ZhCn, Lang::Ko] {
            assert_eq!(text.pick(lang), "a");
        }
    }
}
