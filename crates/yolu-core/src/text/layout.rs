//! 文字の並べ: 段落（`\n` で区切る）ごとに `harfrust` で字形を選んで並べ、折り返しの幅があれば行を分け、揃えで行の位置を決める。
//!
//! 並べはフォントの単位（整数）で行い、大きさ ÷ フォントの 1 em の単位で画素に直す（f64）。並べの向きは左から右に固定する（右から左の文字の
//! 並べの向きと、向きの混ざった段落の並べ替えは扱わない）。
//!
//! 折り返し: 字の境目（`harfrust` の字のまとまり）のうち、次で分けてよい所だけで行を分ける（Unicode の行分けの規則の一部）。
//! - 空白（半角・全角・タブ）の後ろで分けてよい。空白の前では分けない（行末の空白は行の幅に数えない）。
//! - 漢字・かな・ハングルなど CJK の文字の前後は分けてよい。ただし行頭に来ない文字（句読点・閉じ括弧・小書きのかな・長音など）の前と、
//!   行末に来ない文字（開き括弧）の後ろでは分けない。
//! - 1 語が幅を超えて分けられる所が無いときは、その語の中の字の境目で分ける（1 字が幅を超えるときはその字だけの行にする）。

use harfrust::{Direction, ShaperData, UnicodeBuffer};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::MetadataProvider;

use super::{TextAlign, TextSettings};
use crate::error::CoreError;

/// 読んだフォント（並べの表と輪郭）。
pub(crate) struct Face<'a> {
    pub(crate) font: skrifa::FontRef<'a>,
    shaper: ShaperData,
    units_per_em: f64,
}

impl<'a> Face<'a> {
    pub(crate) fn new(bytes: &'a [u8], index: u32) -> Result<Face<'a>, CoreError> {
        let font = skrifa::FontRef::from_index(bytes, index)
            .map_err(|_| CoreError::InvalidArgument("フォントのファイルを読めない"))?;
        let units = font
            .head()
            .map_err(|_| CoreError::InvalidArgument("フォントのファイルを読めない"))?
            .units_per_em();
        if !(16..=16384).contains(&units) {
            return Err(CoreError::InvalidArgument("フォントのファイルを読めない"));
        }
        // 輪郭の表が無いフォント（色の絵文字だけのフォントなど）は描けない
        if font.outline_glyphs().format().is_none() {
            return Err(CoreError::InvalidArgument("フォントに輪郭が無い"));
        }
        let shaper = ShaperData::new(&font);
        Ok(Face {
            font,
            shaper,
            units_per_em: units as f64,
        })
    }
}

/// 並べた字形 1 つ（基準の点からの位置。y は上向き、回転の前）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct PlacedGlyph {
    pub(crate) id: u32,
    pub(crate) x: f64,
    pub(crate) y: f64,
}

/// 並べた行 1 つ。位置は基準の点から（y は上向き、回転の前）。
#[derive(Clone, PartialEq, Debug)]
pub struct TextLine {
    /// 文の中のバイトの範囲（改行を含まない。折り返しで分けた行は、分けた所まで）。
    pub start: usize,
    pub end: usize,
    /// 行の左端。
    pub x: f64,
    /// 行の幅（行末の空白と、最後の字の後ろの字間を除く）。
    pub width: f64,
    /// 行の上端（基準の点の y を 0 として、下へ負）。
    pub top: f64,
    /// ベースライン。
    pub baseline: f64,
    /// 字の境目の位置: （文の中のバイトの位置, x）。行の始めから終わりまで昇順。カーソルを置ける所。
    pub carets: Vec<(usize, f64)>,
}

/// 並べの結果。
#[derive(Clone, PartialEq, Debug)]
pub struct TextLayout {
    pub lines: Vec<TextLine>,
    pub(crate) glyphs: Vec<PlacedGlyph>,
    /// 行の送り（画素）。
    pub line_advance: f64,
    /// フォントの上の高さ・下の深さ（画素、どちらも正）。
    pub ascent: f64,
    pub descent: f64,
}

impl TextLayout {
    /// 回転の前の、全部の行の箱（左, 下, 右, 上。基準の点から）。行が無ければ点。
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for l in &self.lines {
            b[0] = b[0].min(l.x);
            b[2] = b[2].max(l.x + l.width);
            b[3] = b[3].max(l.top);
            b[1] = b[1].min(l.top - self.line_advance);
        }
        if b[0] > b[2] {
            [0.0; 4]
        } else {
            b
        }
    }
}

/// 字のまとまり 1 つ（`harfrust` の cluster）。
struct Cluster {
    /// 段落の中のバイトの位置。
    start: usize,
    end: usize,
    /// 字形（段落の字形の並びの範囲）。
    glyphs: std::ops::Range<usize>,
    /// 送り（字間を含む。画素）。
    advance: f64,
    /// 空白だけのまとまり。
    space: bool,
}

/// 段落の字形（フォントの単位を画素に直したもの）。
struct ShapedGlyph {
    id: u32,
    advance: f64,
    dx: f64,
    dy: f64,
}

/// 文を並べる。フォントは `bytes`（束なら `index` 番）。
pub fn layout(settings: &TextSettings, bytes: &[u8], index: u32) -> Result<TextLayout, CoreError> {
    settings.validate()?;
    let face = Face::new(bytes, index)?;
    Ok(layout_face(settings, &face))
}

pub(crate) fn layout_face(settings: &TextSettings, face: &Face<'_>) -> TextLayout {
    let scale = settings.size / face.units_per_em;
    let metrics = face
        .font
        .metrics(Size::new(settings.size as f32), LocationRef::default());
    let ascent = metrics.ascent as f64;
    let descent = (-metrics.descent as f64).max(0.0);
    let line_advance = settings.line_height * settings.size;
    let spacing = settings.letter_spacing * settings.size;
    let shaper = face.shaper.shaper(&face.font).build();

    let mut lines = Vec::new();
    let mut glyphs = Vec::new();
    let mut offset = 0usize;
    for paragraph in settings.text.split('\n') {
        let base = offset;
        offset += paragraph.len() + 1;
        let (shaped, clusters) = shape(&shaper, paragraph, scale, spacing);
        for range in break_lines(paragraph, &clusters, settings.wrap_width) {
            let top = -(lines.len() as f64) * line_advance;
            let baseline = top - ascent;
            let used = &clusters[range.clone()];
            // 行末の空白と最後の字間は幅に数えない
            let visible = used.iter().rposition(|c| !c.space).map_or(0, |i| i + 1);
            let width = if visible == 0 {
                0.0
            } else {
                used[..visible].iter().map(|c| c.advance).sum::<f64>() - spacing
            };
            let x = match (settings.align, settings.wrap_width > 0.0) {
                (TextAlign::Left, _) => 0.0,
                (TextAlign::Center, false) => -width * 0.5,
                (TextAlign::Right, false) => -width,
                (TextAlign::Center, true) => (settings.wrap_width - width) * 0.5,
                (TextAlign::Right, true) => settings.wrap_width - width,
            };
            let (start, end) = match (used.first(), used.last()) {
                (Some(f), Some(l)) => (f.start, l.end),
                _ => {
                    // 空の段落・空の行: 行の始めは段落の始め（または分けた所）
                    let at = clusters
                        .get(range.start)
                        .map_or(paragraph.len(), |c| c.start);
                    (at, at)
                }
            };
            let mut carets = Vec::with_capacity(used.len() + 1);
            let mut pen = x;
            carets.push((base + start, pen));
            for c in used {
                for g in &shaped[c.glyphs.clone()] {
                    glyphs.push(PlacedGlyph {
                        id: g.id,
                        x: pen + g.dx,
                        y: baseline + g.dy,
                    });
                    pen += g.advance;
                }
                pen += spacing;
                carets.push((base + c.end, pen - spacing));
            }
            lines.push(TextLine {
                start: base + start,
                end: base + end,
                x,
                width,
                top,
                baseline,
                carets,
            });
        }
    }
    TextLayout {
        lines,
        glyphs,
        line_advance,
        ascent,
        descent,
    }
}

/// 段落を並べ、字のまとまりに分ける。
fn shape(
    shaper: &harfrust::Shaper<'_>,
    paragraph: &str,
    scale: f64,
    spacing: f64,
) -> (Vec<ShapedGlyph>, Vec<Cluster>) {
    if paragraph.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(paragraph);
    buffer.guess_segment_properties();
    buffer.set_direction(Direction::LeftToRight);
    let out = shaper.shape(buffer, harfrust::ShapeOptions::new());
    let shaped: Vec<ShapedGlyph> = out
        .glyph_infos()
        .iter()
        .zip(out.glyph_positions())
        .map(|(info, pos)| ShapedGlyph {
            id: info.glyph_id,
            advance: pos.x_advance as f64 * scale,
            dx: pos.x_offset as f64 * scale,
            dy: pos.y_offset as f64 * scale,
        })
        .collect();
    let infos = out.glyph_infos();
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut i = 0;
    while i < infos.len() {
        let start = infos[i].cluster as usize;
        let mut j = i + 1;
        while j < infos.len() && infos[j].cluster as usize == start {
            j += 1;
        }
        let advance = shaped[i..j].iter().map(|g| g.advance).sum::<f64>() + spacing;
        clusters.push(Cluster {
            start,
            end: start, // 下で次のまとまりの始めに
            glyphs: i..j,
            advance,
            space: false,
        });
        i = j;
    }
    // 左から右に並べたので、まとまりの始めは昇順（並べ替えの無いフォントでも念のため整える）
    clusters.sort_by_key(|c| c.start);
    let n = clusters.len();
    for k in 0..n {
        let end = if k + 1 < n {
            clusters[k + 1].start
        } else {
            paragraph.len()
        };
        clusters[k].end = end.max(clusters[k].start);
        clusters[k].space = paragraph
            .get(clusters[k].start..clusters[k].end)
            .is_some_and(|s| !s.is_empty() && s.chars().all(is_space));
    }
    (shaped, clusters)
}

/// 段落の行の分け（字のまとまりの範囲の並び）。空の段落も 1 行。
fn break_lines(paragraph: &str, clusters: &[Cluster], wrap: f64) -> Vec<std::ops::Range<usize>> {
    if clusters.is_empty() {
        return std::iter::once(0..0).collect();
    }
    if wrap <= 0.0 {
        return std::iter::once(0..clusters.len()).collect();
    }
    let first_char = |c: &Cluster| paragraph[c.start..].chars().next().unwrap_or(' ');
    let last_char = |c: &Cluster| paragraph[..c.end].chars().next_back().unwrap_or(' ');
    let mut lines = Vec::new();
    let mut start = 0;
    while start < clusters.len() {
        let mut width = 0.0; // 空白でない最後の字までの送りの和
        let mut pending = 0.0; // まだ数えていない空白の送り
        let mut last_break: Option<usize> = None; // この位置の前で分けてよい
        let mut end = clusters.len();
        let mut k = start;
        while k < clusters.len() {
            let c = &clusters[k];
            if k > start && allows_break(last_char(&clusters[k - 1]), first_char(c)) {
                last_break = Some(k);
            }
            if c.space {
                pending += c.advance;
                k += 1;
                continue;
            }
            let next = width + pending + c.advance;
            if next > wrap + 1e-9 && k > start {
                end = match last_break {
                    Some(b) if b > start => b,
                    _ => k,
                };
                break;
            }
            width = next;
            pending = 0.0;
            k += 1;
        }
        lines.push(start..end);
        start = end;
    }
    lines
}

/// 空白（行末に残しても幅に数えない）。
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{3000}')
}

/// 行頭に置かない文字（句読点・閉じ括弧・小書きのかな・長音・繰り返し記号など）。
fn no_break_before(c: char) -> bool {
    matches!(
        c,
        '、' | '。'
            | '，'
            | '．'
            | '・'
            | '：'
            | '；'
            | '？'
            | '！'
            | 'ー'
            | '〜'
            | '～'
            | '」'
            | '』'
            | '）'
            | '］'
            | '｝'
            | '〉'
            | '》'
            | '】'
            | '〕'
            | '〗'
            | '〙'
            | '〛'
            | '’'
            | '”'
            | 'ゝ'
            | 'ゞ'
            | 'ヽ'
            | 'ヾ'
            | '々'
            | '〻'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ゕ'
            | 'ゖ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
            | '…'
            | '‥'
            | ')'
            | ']'
            | '}'
            | ','
            | '.'
            | ':'
            | ';'
            | '!'
            | '?'
            | '%'
    ) || ('\u{31F0}'..='\u{31FF}').contains(&c)
}

/// 行末に置かない文字（開き括弧）。
fn no_break_after(c: char) -> bool {
    matches!(
        c,
        '「' | '『'
            | '（'
            | '［'
            | '｛'
            | '〈'
            | '《'
            | '【'
            | '〔'
            | '〖'
            | '〘'
            | '〚'
            | '‘'
            | '“'
            | '('
            | '['
            | '{'
    )
}

/// 漢字・かな・ハングル・全角の形など、字間で分けてよい文字。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF
        | 0x2E80..=0x2FFF
        | 0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3100..=0x31FF
        | 0x3200..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFF)
}

/// `prev` と `next` の間で行を分けてよいか。
fn allows_break(prev: char, next: char) -> bool {
    if is_space(next) {
        return false;
    }
    if is_space(prev) {
        return true;
    }
    if no_break_before(next) || no_break_after(prev) {
        return false;
    }
    is_cjk(prev) || is_cjk(next)
}

#[cfg(test)]
mod rules {
    use super::*;

    #[test]
    fn breaks_follow_spaces_and_cjk_but_not_kinsoku() {
        assert!(allows_break(' ', 'a'));
        assert!(!allows_break('a', ' '));
        assert!(!allows_break('a', 'b'));
        assert!(allows_break('あ', 'い'));
        assert!(allows_break('a', '漢'));
        assert!(!allows_break('あ', '。'));
        assert!(!allows_break('「', 'あ'));
        assert!(!allows_break('か', 'ッ'));
        assert!(allows_break('。', 'あ'));
    }
}
