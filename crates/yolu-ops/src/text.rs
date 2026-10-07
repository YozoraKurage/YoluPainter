//! 日本語と英語の文。誤りの理由・知らせは両方の文を持ち、CLI は `--lang`、アプリは自分の言語で選ぶ。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 文の言語。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Ja,
    En,
}

impl Lang {
    /// `ja` / `en`（大文字小文字・`ja-JP` のような後ろの印は問わない）から。
    pub fn parse(text: &str) -> Option<Lang> {
        let head = text.split(['-', '_']).next()?.to_ascii_lowercase();
        match head.as_str() {
            "ja" => Some(Lang::Ja),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
    pub fn pick<T>(self, ja: T, en: T) -> T {
        match self {
            Lang::Ja => ja,
            Lang::En => en,
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

/// 日本語と英語の同じ意味の文。
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
    pub fn pick(&self, lang: Lang) -> &str {
        match lang {
            Lang::Ja => &self.ja,
            Lang::En => &self.en,
        }
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
}
