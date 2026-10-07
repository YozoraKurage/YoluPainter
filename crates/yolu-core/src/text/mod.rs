//! テキストレイヤーの値（[`TextSettings`]）と、その値から層の画素を描く道（並べる [`layout`]・塗る [`render`]）。
//!
//! - 並べ: `harfrust`（HarfBuzz の移植）で段落ごとに字形を選んで並べ、折り返しの幅があれば字間で行を分ける（下の「折り返し」）。
//! - 形: `skrifa` で字の輪郭を、ヒンティングをかけずに大きさ（px）の曲線にする（画面の解像度に合わせた変形をしないので、どの大きさ・
//!   回転でも形が同じ比で変わる）。
//! - 塗り: `vello_cpu` で輪郭を塗り（アンチエイリアス）、覆う量をアルファにして色を置く（straight RGBA8。覆わない画素は透明で RGB も 0）。
//!   **結果は CPU・スレッドの数によらず同じバイト**: `vello_cpu` の SIMD の道を、CPU を調べずに既定の道（x86_64 の既定のターゲットでは
//!   SIMD を使わない道）に固定し、`u8` の道で塗る。キャンバスは文書の原点に揃えた 256 画素の升目ごとに別々に塗るので、升目を並べて塗っても
//!   結果は変わらない。
//!
//! 座標は文書の画素（左下が原点、y は上向き）。文字の位置（[`TextSettings::x`]・`y`）は 1 行目の上端の基準の点で、行は下へ進む。
//! 揃えの基準は、折り返しの幅が 0 なら基準の点（左揃えは点から右へ・中央は点を真ん中に・右揃えは点で終わる）、幅があれば点から右へ
//! 幅の箱。回転は基準の点のまわりで、反時計回りが正（度）。
//!
//! フォントのファイルは文書に入れない（フォントの許諾が絡む）。文書はフォントの名前（同梱）か、利用者が選んだファイル（OS のフォントも）の
//! 道・中身の SHA-256・ファミリー名・PostScript 名・太さ・斜体かを持ち、フォントの中身は呼び手が渡す（[`TextFont`]）。

mod layout;
mod render;
#[cfg(test)]
mod tests;

pub use layout::{layout, TextLayout, TextLine};
pub use render::{default_batch, render, render_batched, render_into};

use crate::error::CoreError;
use crate::types::Rgba8;

/// 文の長さの上限（UTF-8 のバイト数。.ylp の正本の文字列の上限と同じ）。
pub const MAX_TEXT_BYTES: usize = 4096;
/// 大きさ（1 em の画素）の範囲。
pub const MIN_SIZE: f64 = 1.0;
pub const MAX_SIZE: f64 = 4096.0;
/// 行間（大きさに掛ける）の範囲。
pub const MIN_LINE_HEIGHT: f64 = 0.1;
pub const MAX_LINE_HEIGHT: f64 = 10.0;
/// 字間（大きさに掛ける。負は詰める）の範囲。
pub const MIN_LETTER_SPACING: f64 = -1.0;
pub const MAX_LETTER_SPACING: f64 = 10.0;
/// 位置・折り返しの幅の絶対値の上限（画素）。
pub const MAX_COORDINATE: f64 = 1.0e6;
/// 同梱のフォントの名前の長さの上限。
pub const MAX_FONT_NAME: usize = 64;
/// フォントのファイルの道の長さの上限（UTF-8 のバイト数）。
pub const MAX_FONT_PATH_BYTES: usize = 4096;
/// フォントのファミリー名・PostScript 名の長さの上限（UTF-8 のバイト数）。
pub const MAX_FONT_FAMILY_BYTES: usize = 256;

/// フォントのファイルの中の名前と太さ（開き直したときに、OS のフォントから同じフォントを探す手がかり）。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FontNames {
    /// ファミリー名（name の 16 番、無ければ 1 番。英語の名前、無ければ最初の名前。読めなければ空）。
    pub family: String,
    /// PostScript 名（name の 6 番。無ければ空）。
    pub postscript: String,
    /// 太さ（OS/2 の usWeightClass、1〜1000）。
    pub weight: u16,
    /// 斜体か（斜体・斜め）。
    pub italic: bool,
}

impl Default for FontNames {
    fn default() -> Self {
        FontNames {
            family: String::new(),
            postscript: String::new(),
            weight: 400,
            italic: false,
        }
    }
}

/// テキストレイヤーのフォント。中身は文書に入れず、呼び手が名前・道から探して渡す。
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TextFont {
    /// アプリに同梱したフォント（名前は `a-z 0-9 -` の 1〜64 文字）。
    Bundled(String),
    /// 利用者が選んだフォントのファイル（OS のフォントも）。`index` はフォントの束（.ttc）の中の番号（束でなければ 0）。`sha256` は
    /// 選んだときの中身で、開き直したときに同じフォントかを確かめる。`names` は道に無いときに OS のフォントから探す手がかり。
    File {
        path: String,
        index: u32,
        sha256: [u8; 32],
        names: FontNames,
    },
}

impl TextFont {
    /// フォントの束（.ttc）の中の番号（同梱のフォントは 0）。
    pub fn index(&self) -> u32 {
        match self {
            TextFont::Bundled(_) => 0,
            TextFont::File { index, .. } => *index,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        match self {
            TextFont::Bundled(name) => {
                if name.is_empty()
                    || name.len() > MAX_FONT_NAME
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(CoreError::InvalidArgument("同梱のフォントの名前"));
                }
            }
            TextFont::File { path, names, .. } => {
                if path.is_empty()
                    || path.len() > MAX_FONT_PATH_BYTES
                    || path.chars().any(char::is_control)
                {
                    return Err(CoreError::InvalidArgument("フォントのファイルの道"));
                }
                let name_ok =
                    |s: &str| s.len() <= MAX_FONT_FAMILY_BYTES && !s.chars().any(char::is_control);
                if !name_ok(&names.family)
                    || !name_ok(&names.postscript)
                    || !(1..=1000).contains(&names.weight)
                {
                    return Err(CoreError::InvalidArgument("フォントの名前"));
                }
            }
        }
        Ok(())
    }
}

/// 行の揃え。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// テキストレイヤーの値（あとから編集できる文字）。層の画素はこの値とフォントから描いた結果。
#[derive(Clone, PartialEq, Debug)]
pub struct TextSettings {
    /// 文（改行は `\n`。ほかの制御文字はタブだけ置ける）。
    pub text: String,
    pub font: TextFont,
    /// 大きさ（1 em の画素）。
    pub size: f64,
    /// 文字の色（アルファは文字の不透明度）。
    pub color: Rgba8,
    /// 行間（行の送り。大きさに掛ける）。
    pub line_height: f64,
    /// 字間（字ごとに加える送り。大きさに掛ける。負は詰める）。
    pub letter_spacing: f64,
    pub align: TextAlign,
    /// 基準の点（文書の画素、左下が原点）。1 行目の上端。
    pub x: f64,
    pub y: f64,
    /// 基準の点のまわりの回転（度、反時計回りが正）。
    pub rotation: f64,
    /// 折り返しの幅（画素。0 は折り返さない）。
    pub wrap_width: f64,
}

impl TextSettings {
    /// 既定の値で、文とフォントと基準の点だけを決めた値。
    pub fn new(text: &str, font: TextFont, x: f64, y: f64) -> TextSettings {
        TextSettings {
            text: text.to_owned(),
            font,
            size: 48.0,
            color: Rgba8::new(0, 0, 0, 255),
            line_height: 1.2,
            letter_spacing: 0.0,
            align: TextAlign::Left,
            x,
            y,
            rotation: 0.0,
            wrap_width: 0.0,
        }
    }

    /// 値の検査（範囲・有限・文の長さと文字）。
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidArgument("文の長さ"));
        }
        if self
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(CoreError::InvalidArgument("文の制御文字"));
        }
        self.font.validate()?;
        let within = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
        if !within(self.size, MIN_SIZE, MAX_SIZE) {
            return Err(CoreError::InvalidArgument("文字のサイズ"));
        }
        if !within(self.line_height, MIN_LINE_HEIGHT, MAX_LINE_HEIGHT) {
            return Err(CoreError::InvalidArgument("行間"));
        }
        if !within(self.letter_spacing, MIN_LETTER_SPACING, MAX_LETTER_SPACING) {
            return Err(CoreError::InvalidArgument("字間"));
        }
        if !within(self.x, -MAX_COORDINATE, MAX_COORDINATE)
            || !within(self.y, -MAX_COORDINATE, MAX_COORDINATE)
        {
            return Err(CoreError::InvalidArgument("文字の位置"));
        }
        if !within(self.rotation, -360.0, 360.0) {
            return Err(CoreError::InvalidArgument("文字の回転"));
        }
        if !within(self.wrap_width, 0.0, MAX_COORDINATE) {
            return Err(CoreError::InvalidArgument("折り返しの幅"));
        }
        Ok(())
    }

    /// 並べ・形が変わらず、位置と回転だけが違うか（動かす・回すの描き直しの判断に使う）。
    pub fn same_shape(&self, other: &TextSettings) -> bool {
        self.text == other.text
            && self.font == other.font
            && self.size == other.size
            && self.line_height == other.line_height
            && self.letter_spacing == other.letter_spacing
            && self.align == other.align
            && self.wrap_width == other.wrap_width
    }
}

/// 既定の同梱のフォントの名前（アプリに同梱した BIZ UDPGothic。中身はアプリが持つ）。
pub const DEFAULT_FONT: &str = "biz-udpgothic";
/// 同梱のフォントの名前（並びは画面の選び方の順）。
pub const BUNDLED_FONTS: [&str; 2] = [DEFAULT_FONT, "biz-udpgothic-bold"];

/// 利用者が選んだフォントのファイルの値（道・束の番号・中身の SHA-256・名前と太さ）。描けるかは確かめない（[`check_font`]）。
pub fn file_font(path: &str, index: u32, bytes: &[u8]) -> TextFont {
    TextFont::File {
        path: path.to_owned(),
        index,
        sha256: sha256(bytes),
        names: describe_font(bytes, index)
            .map(|d| d.names)
            .unwrap_or_default(),
    }
}

/// 中身の SHA-256。
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// フォントのファイルの中の 1 つのフォントの名前（一覧に並べる・同じフォントを探す）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontDescription {
    pub names: FontNames,
    /// スタイルの名前（name の 17 番、無ければ 2 番。英語の名前、無ければ最初の名前）。
    pub style: String,
    /// 日本語のファミリー名（あれば）。
    pub family_ja: Option<String>,
}

/// フォントの名前と太さを読む（描けないフォント・輪郭の無いフォントは理由）。`index` は束の中の番号。並べ・塗りの準備はしないので
/// [`check_font`] より軽い（OS のフォントの一覧を作るため）。
pub fn describe_font(bytes: &[u8], index: u32) -> Result<FontDescription, CoreError> {
    use skrifa::{string::StringId, MetadataProvider};
    let font = skrifa::FontRef::from_index(bytes, index)
        .map_err(|_| CoreError::InvalidArgument("フォントのファイルを読めない"))?;
    if font.outline_glyphs().format().is_none() {
        return Err(CoreError::InvalidArgument("フォントに輪郭が無い"));
    }
    let clean = |s: String| -> String {
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        let mut s = s.trim().to_owned();
        while s.len() > MAX_FONT_FAMILY_BYTES {
            s.pop();
        }
        s
    };
    let english = |ids: &[StringId]| -> String {
        ids.iter()
            .find_map(|id| font.localized_strings(*id).english_or_first())
            .map(|s| clean(s.to_string()))
            .unwrap_or_default()
    };
    let families = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME];
    let family_ja = families.iter().find_map(|id| {
        font.localized_strings(*id)
            .find(|s| s.language().is_some_and(|l| l.starts_with("ja")))
            .map(|s| clean(s.to_string()))
            .filter(|s| !s.is_empty())
    });
    let attributes = font.attributes();
    let weight = attributes.weight.value().round().clamp(1.0, 1000.0) as u16;
    Ok(FontDescription {
        names: FontNames {
            family: english(&families),
            postscript: english(&[StringId::POSTSCRIPT_NAME]),
            weight,
            italic: attributes.style != skrifa::attribute::Style::Normal,
        },
        style: english(&[
            StringId::TYPOGRAPHIC_SUBFAMILY_NAME,
            StringId::SUBFAMILY_NAME,
        ]),
        family_ja,
    })
}

/// フォントの中身が、文書のフォントと同じものか（ファイルのフォントは選んだときの SHA-256 と比べる。同梱のフォントは呼び手が名前で探したもの）。
pub fn font_matches(font: &TextFont, bytes: &[u8]) -> bool {
    match font {
        TextFont::Bundled(_) => true,
        TextFont::File { sha256: want, .. } => sha256(bytes) == *want,
    }
}

/// フォントのファイルを読めるか（呼び手が選んだファイルを確かめる）。読めなければ理由を返す。`index` は束の中の番号。
pub fn check_font(bytes: &[u8], index: u32) -> Result<(), CoreError> {
    layout::Face::new(bytes, index).map(|_| ())
}

/// フォントの束（.ttc）の中のフォントの数（束でなければ 1、読めなければ 0）。
pub fn font_count(bytes: &[u8]) -> u32 {
    match skrifa::raw::FileRef::new(bytes) {
        Ok(skrifa::raw::FileRef::Font(_)) => 1,
        Ok(skrifa::raw::FileRef::Collection(c)) => c.len(),
        Err(_) => 0,
    }
}
