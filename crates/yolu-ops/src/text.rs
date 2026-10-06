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
}

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
