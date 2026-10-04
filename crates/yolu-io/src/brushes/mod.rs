//! ブラシ形式の取り込み: GIMP の GBR・GIH・VBR、Photoshop の ABR（版 1・2・6〜10）・PAT、CLIP STUDIO の SUT、PNG の筆先を、
//! core のブラシ（[`yolu_core::Brush`]）にする。アプリへの口は [`import`] の 1 本（メモリ上のバイト列からは [`import_bytes`]）。
//!
//! - 元のアプリの設定のうち core で表せるもの（筆先・間隔・角度・真円率・ゆらぎ・散布・筆圧・フェード・傾き・色の変化・
//!   デュアルブラシ・質感）は `Brush` に入れる。表せないもの・近似したもの・読めなかったものは、ブラシごとの
//!   [`Unrepresented`] に型で載せる（黙って捨てない）。プリセットの形が壊れていて取り込めなかったものは [`ImportedSet::skipped`]。
//! - 信頼できないファイル（ダウンロードしたブラシ）を読むので、ファイルの大きさ（[`MAX_FILE_BYTES`]。VBR は [`MAX_VBR_BYTES`]）、展開する画素の合計
//!   （[`MAX_DECODED_BYTES`]）、記述子の深さ・値の数に上限を持ち、宣言された個数で先に確保せず、切れていれば `Truncated` で断る。
//!   何も取り込まない形で断り、途中まで取り込んだ結果は返さない（ABR の設定・模様の節の読み損ねだけは、筆先を取り込んで知らせる）。
//!   ファイル全体にかかわる注記（読めなかった節・使えなかった模様）は [`ImportedSet::notes`] に 1 回だけ持ち、ブラシごとには
//!   複製しない（ブラシの数と注記の数の積で、信頼できないファイルが数 GB を確保できてしまう）。
//! - 失敗と注記は型のまま返す。`Display` は日本語、`english()` は英語（画面が言語ごとに選ぶ）。
//! - 同梱の Krita の筆先は [`bundled`]。

mod abr;
pub mod bundled;
mod descriptor;
mod error;
mod gimp;
mod notes;
mod pattern;
mod png_tip;
mod reader;
mod sut;

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use yolu_core::{Brush, BrushSettings, BrushTip};

pub use error::{
    BrushImportError, Counted, Fault, PackBitsFault, PatternMode, PatternRefusal, SizedItem,
    SkippedPattern, UnsupportedFile,
};
pub use notes::{
    AbrKind, ControlKind, DualNote, PatternNote, Setting, Source, SutInput, SutNote, SutTarget,
    TextureNote, Unrepresented, VbrShape,
};
pub use reader::MAX_DECODED_BYTES;

use reader::Budget;

/// これより大きいファイルは読み込む前に断る。市販の `.abr` の大きいもの（数十 MB）が入る大きさ。
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// `.sut` の上限。公開の調べでは実物は 21 KB〜15 MB（素材の画像を含む）。SQLite はメモリ上へ写して開くので、ファイルの大きさの 2 倍までメモリを使う。
pub const MAX_SUT_BYTES: u64 = 128 * 1024 * 1024;

/// `.vbr` の上限。パラメトリックブラシは 10 行ほどの文字（実物は 200 バイト前後）なので、改行だけの巨大なファイルを
/// 行に割って確保する前に断る。
pub const MAX_VBR_BYTES: u64 = 64 * 1024;

/// 取り込めるファイルの種類（拡張子で決める）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Abr,
    Pat,
    Gbr,
    Gih,
    Vbr,
    Png,
    /// CLIP STUDIO PAINT のサブツール（SQLite）。
    Sut,
}

impl FileKind {
    /// ファイル選択で示す拡張子。
    pub const EXTENSIONS: [&'static str; 7] = ["abr", "pat", "gbr", "gih", "vbr", "png", "sut"];

    /// 拡張子（点なし。大文字小文字を区別しない）から。読まない種類は理由つきで断る。
    pub fn from_extension(extension: &str) -> Result<FileKind, UnsupportedFile> {
        match extension.to_ascii_lowercase().as_str() {
            "abr" => Ok(FileKind::Abr),
            "pat" => Ok(FileKind::Pat),
            "gbr" => Ok(FileKind::Gbr),
            "gih" => Ok(FileKind::Gih),
            "vbr" => Ok(FileKind::Vbr),
            "png" => Ok(FileKind::Png),
            "sut" => Ok(FileKind::Sut),
            "kpp" => Err(UnsupportedFile::KritaPreset),
            other => Err(UnsupportedFile::Extension(printable(other, 16))),
        }
    }

    /// この種類のファイルの大きさの上限（バイト）。
    pub fn max_bytes(self) -> u64 {
        match self {
            FileKind::Vbr => MAX_VBR_BYTES,
            FileKind::Sut => MAX_SUT_BYTES,
            _ => MAX_FILE_BYTES,
        }
    }
}

/// 1 つのファイルから取り込んだブラシ。
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedBrush {
    /// 表示名（空にならない。制御文字を含まない）。
    pub name: String,
    pub source: Source,
    /// core のブラシ。色（描画色）・副色・乱数の種は既定のまま（使う側が決める）。筆先と質感の画像は共有する（`Arc`）ので、
    /// 同じ筆先を使う複数のブラシは同じ画像を指す。
    pub brush: Brush,
    /// このブラシの設定のうち、表せない・近似した・読めなかったもの（ファイル全体の注記は [`ImportedSet::notes`]）。
    pub unrepresented: Vec<Unrepresented>,
}

impl ImportedBrush {
    /// 名前（空なら "Untitled"）と設定を検証して作る。範囲外なら読み手の不具合として断る。
    pub fn new(
        name: &str,
        source: Source,
        brush: Brush,
        unrepresented: Vec<Unrepresented>,
    ) -> Result<ImportedBrush, BrushImportError> {
        brush.validate()?;
        let name = short_text(name, 128);
        Ok(ImportedBrush {
            name: if name.is_empty() {
                "Untitled".into()
            } else {
                name
            },
            source,
            brush,
            unrepresented,
        })
    }
}

/// 取り込めなかったプリセット。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedBrush {
    pub name: String,
    pub reason: SkipReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// プリセットに筆先の形の記述が無い。
    NoTipShape,
    /// プリセットが指す筆先がファイルに無い。
    TipNotInFile,
    /// ブラシの設定の行がファイルに無い（名前だけが残っている）。
    SettingsNotInFile,
}

/// 1 つのファイルの取り込み結果（ファイルの並び。ABR はプリセット、次に使われなかった筆先）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedSet {
    pub brushes: Vec<ImportedBrush>,
    pub skipped: Vec<SkippedBrush>,
    /// ファイル全体にかかわる注記: 読めなかったプリセット・模様の節、読み飛ばした節、使えなかった模様。どのブラシのものでもないので、
    /// ブラシの `unrepresented` には複製しない。
    pub notes: Vec<Unrepresented>,
}

/// ブラシのファイルを拡張子で読み分ける。読めない形式は理由を添えて断る（黙って丸い筆先にしない）。
pub fn import(path: &Path) -> Result<ImportedSet, BrushImportError> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kind = FileKind::from_extension(&extension)?;
    let file = std::fs::File::open(path)?;
    let limit = kind.max_bytes();
    let length = file.metadata()?.len();
    if length > limit {
        return Err(BrushImportError::FileTooLarge { limit });
    }
    // 読んでいる間に伸びたファイルでも上限を超えて読まない
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(BrushImportError::FileTooLarge { limit });
    }
    let name = path.file_name().map(|n| pretty_name(&n.to_string_lossy()));
    import_bytes(kind, &bytes, name.as_deref())
}

/// メモリ上のファイルの中身から。`fallback_name` は名前が無いブラシ・筆先の代わりの名前（普通はファイル名から作った表示名）。
pub fn import_bytes(
    kind: FileKind,
    bytes: &[u8],
    fallback_name: Option<&str>,
) -> Result<ImportedSet, BrushImportError> {
    let limit = kind.max_bytes();
    if bytes.len() as u64 > limit {
        return Err(BrushImportError::FileTooLarge { limit });
    }
    let mut budget = Budget::new(MAX_DECODED_BYTES);
    let one = |brush: ImportedBrush| ImportedSet {
        brushes: vec![brush],
        ..ImportedSet::default()
    };
    match kind {
        FileKind::Abr => abr::read_abr(bytes, fallback_name, &mut budget),
        FileKind::Pat => pattern::read_pat_brushes(bytes, &mut budget),
        FileKind::Sut => sut::read_sut(bytes, fallback_name, &mut budget),
        FileKind::Gbr => gimp::read_gbr(bytes, fallback_name, &mut budget).map(one),
        FileKind::Gih => gimp::read_gih(bytes, fallback_name, &mut budget).map(one),
        FileKind::Vbr => {
            let text = String::from_utf8_lossy(bytes);
            gimp::read_vbr(
                text.trim_start_matches('\u{feff}'),
                fallback_name,
                &mut budget,
            )
            .map(one)
        }
        FileKind::Png => {
            let name = fallback_name.unwrap_or("Untitled");
            let tip = png_tip::read_png_tip(bytes, name)?;
            let brush = Brush {
                base: BrushSettings {
                    radius: tip.width().max(tip.height()) as f64 / 2.0,
                    spacing: 0.1,
                    ..BrushSettings::default()
                },
                tip: yolu_core::TipShape {
                    image: Some(Arc::new(tip)),
                    ..yolu_core::TipShape::default()
                },
                ..Brush::default()
            };
            Ok(one(ImportedBrush::new(
                name,
                Source::PngTip,
                brush,
                Vec::new(),
            )?))
        }
    }
}

/// PNG の筆先の画像（暗いほど塗り、白と透明は塗らない）。行は下から。
pub fn read_png_tip(bytes: &[u8], name: &str) -> Result<BrushTip, BrushImportError> {
    Ok(png_tip::read_png_tip(bytes, name)?)
}

/// ファイル名からの表示名（`chalk_grainy-01.gbr` → `Chalk grainy 01`）。
pub fn pretty_name(file: &str) -> String {
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem.replace(['_', '-'], " ");
    let stem = stem.trim();
    if stem.is_empty() {
        return file.to_string();
    }
    let mut chars = stem.chars();
    let first = chars.next().unwrap_or(' ');
    let mut upper = first.to_uppercase();
    let head = match (upper.next(), upper.next()) {
        (Some(c), None) => c,
        _ => first,
    };
    format!("{head}{}", chars.as_str())
}

/// ファイルから来た文字列を画面に出せる形にする: 制御文字（改行を含む）を除き、前後の空白を除き、`max` 文字までにする。
pub(crate) fn short_text(text: &str, max: usize) -> String {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim().chars().take(max).collect()
}

/// 形式の印（4 文字のキー・型名など）を画面に出せる形にする: 制御文字（改行を含む）は `?` にして（`ascii()` が 0x80 以上を `?` に
/// するのと同じ。字数と位置が残り、空にならない）、`max` 文字までにする。名前は `short_text`（制御文字を除く）。
pub(crate) fn printable(text: &str, max: usize) -> String {
    text.chars()
        .take(max)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// 0〜1 の値を 0〜255 へ四捨五入する（NaN は 0）。core の `to_byte` と同じ式。
pub(crate) fn to_byte(value: f64) -> u8 {
    let v = value * 255.0 + 0.5;
    if v >= 255.0 {
        255
    } else if v > 0.0 {
        v as u8
    } else {
        0
    }
}
