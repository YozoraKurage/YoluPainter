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
}

mod errors;
pub(crate) use errors::budget_text;
