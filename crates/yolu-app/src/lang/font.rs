//! 言語のフォントのファイルの名前（言語の表が指す。読むのは `ui::fonts`）。

/// 言語のフォント（配布物の `fonts/` の別のファイル。言語の表 `lang::Info::font` が指す）。
#[derive(Debug, PartialEq, Eq)]
pub struct LangFont {
    /// `FontDefinitions::font_data` の鍵の頭（`-regular`・`-bold` を付ける）。
    pub key: &'static str,
    /// 普通の太さのファイルの名前。
    pub regular: &'static str,
    /// 太字のファイルの名前。
    pub bold: &'static str,
}

/// 繁体字中国語（Noto Sans CJK TC）。
pub const NOTO_SANS_TC: LangFont = LangFont {
    key: "noto-sans-tc",
    regular: "NotoSansTC-Regular.otf",
    bold: "NotoSansTC-Bold.otf",
};
/// 簡体字中国語（Noto Sans CJK SC）。
pub const NOTO_SANS_SC: LangFont = LangFont {
    key: "noto-sans-sc",
    regular: "NotoSansSC-Regular.otf",
    bold: "NotoSansSC-Bold.otf",
};
/// 韓国語（Noto Sans CJK KR）。
pub const NOTO_SANS_KR: LangFont = LangFont {
    key: "noto-sans-kr",
    regular: "NotoSansKR-Regular.otf",
    bold: "NotoSansKR-Bold.otf",
};
