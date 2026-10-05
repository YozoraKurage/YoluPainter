//! 取り込みの失敗。文はここでは確定させず、種類と値を型のまま運ぶ（画面が言語ごとに文を作る。`Display` は日本語、
//! `english()` は英語）。壊れたファイルのどの場所で止まったかはオフセットなどの値として型に持つ（ログ・試験は `Debug` で読む）が、
//! 画面の文には出さない。診断の本文（デコーダーの英語など）は運ばない。

use std::fmt;

use yolu_core::CoreError;

/// ファイルを取り込めなかった理由。何も取り込まない（途中まで取り込んだ結果は返さない）。
#[derive(Debug)]
pub enum BrushImportError {
    /// ファイルを開けない・読めない。
    Io(std::io::Error),
    /// ファイルが上限（[`MAX_FILE_BYTES`](super::MAX_FILE_BYTES)）より大きい。読み込む前に断る。
    FileTooLarge { limit: u64 },
    /// 読まない種類のファイル。
    Unsupported(UnsupportedFile),
    /// 形式の中身が壊れている・対応しない。
    Fault(Fault),
    /// `.pat` の模様が 1 つも使えない（理由つき）。
    NoUsablePattern(Vec<SkippedPattern>),
    /// 取り込んだ設定が core の範囲に入らない（読み手の不具合。通常は起きない）。
    Core(CoreError),
}

/// 読まないファイルの種類。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedFile {
    /// Krita のブラシプリセット `.kpp`（Krita の筆のエンジンの設定。筆先の画像 `.gbr`・`.gih`・`.png` は別に読める）。
    KritaPreset,
    /// 知らない拡張子（拡張子が無ければ空）。
    Extension(String),
}

/// 使えなかった模様と理由（`.pat` と ABR の模様の節）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedPattern {
    pub name: String,
    pub reason: PatternRefusal,
}

/// 読めたが筆先の質感に使えない模様の理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternRefusal {
    /// チャンネルどうしで大きさが違う。
    ChannelsDiffer,
    /// 1 辺が 1〜2048 の外。
    TooLarge { width: i64, height: i64 },
    /// 8・16 bit 以外。
    Depth(i16),
    /// ZIP 圧縮の画素。
    Zip,
    /// 対応しない色のモード。
    Mode(PatternMode),
    /// 色のチャンネルが足りない。
    ChannelsMissing,
}

/// 模様の色のモード（対応するのはグレー・インデックス・RGB だけ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternMode {
    Bitmap,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
    Other(i32),
}

/// 筆先・模様の対象。大きさの断りで何の大きさかを言う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizedItem {
    Brush,
    Tip,
    Pattern,
    Image,
}

/// 数や長さを検査した場所（宣言が残りのバイト数に入らない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counted {
    NameLength,
    PixelData,
    SectionLength,
    SampleLength,
    StringLength,
    KeyLength,
    DataLength,
    PatternLength,
    PatternDataLength,
    PatternChannelLength,
}

/// PackBits の 1 行が壊れている理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackBitsFault {
    EndedEarly,
    Overflow,
}

/// 形式の中身の壊れ方・対応外。どれも読んだ位置か値を持つ。
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// 読む途中でファイルが尽きた。
    Truncated {
        offset: usize,
    },
    /// 宣言された個数・長さが、残りのバイト数に入らない（壊れている、または切れている）。
    BadCount {
        what: Counted,
        value: u64,
        offset: usize,
    },
    /// 展開した筆先・模様の合計が上限を超える（小さいファイルが巨大な画像へ展開される形を断つ）。
    Budget,
    /// 1 辺が 1〜2048 の外。
    SizeOutOfRange {
        what: SizedItem,
        width: i64,
        height: i64,
    },

    // GIMP .gbr / .gih
    GbrHeaderSize {
        version: u32,
        size: u32,
    },
    GbrSignature,
    GbrNameTooLong,
    GbrHeaderMismatch,
    /// 16 bit 浮動小数点（CinePaint）の筆。
    GbrCinePaint,
    GbrPixelSize(u32),
    GbrVersion(u32),
    GihHeader,
    GihCellCount,

    // GIMP .vbr
    NotVbr,
    VbrEndsEarly {
        line: usize,
    },
    VbrNumber {
        line: usize,
        min: f64,
        max: f64,
    },
    VbrShape(String),
    VbrVersion(String),

    // Photoshop .abr
    AbrVersion(i16),
    AbrSubversion(i16),
    AbrBrushCount(i16),
    AbrBrushTruncated {
        index: usize,
    },
    AbrBrushType {
        index: usize,
        kind: i16,
    },
    /// 版 6 以降の筆先（`samp`）の個数が上限（`abr::MAX_TIPS`）を超える。値は上限。
    AbrTipCount(usize),
    AbrSection {
        offset: usize,
    },
    TipDepth(i16),
    Tip16BitCompressed,
    TipCompression(u8),
    PackBits(PackBitsFault),
    AbrNoBrushes,

    // ActionDescriptor（ABR の設定）
    DescriptorDepth,
    DescriptorItems,
    DescriptorType {
        kind: String,
        offset: usize,
    },
    DescriptorReference(String),
    DescriptorNotFinite,

    // Photoshop パターン（.pat と ABR の模様の節）
    NotPat,
    PatVersion(u16),
    PatCount(u32),
    PatternVersion(i32),
    PatternDataVersion(i32),
    PatternChannelCount(i32),
    PatternChannelLengthTooShort(u64),
    PatternEmptyRecord,
    /// 模様が 1 つも無い（`.pat`）。
    NoPatterns,

    // PNG の筆先
    NotPng,
    PngLimits,

    // CLIP STUDIO の .sut（SQLite）
    /// SQLite のデータベースとして読めない（署名が無い・壊れている・暗号化されている）。
    SutNotDatabase,
    /// ブラシの表（`Node`）が無い・読めない。
    SutNoNodeTable,
    /// 読めるブラシが 1 つも無い。
    SutNoBrushes,
    /// データベースの読み取りが上限（命令の数・展開する画素）を超えた。
    SutLimits,
}

impl From<std::io::Error> for BrushImportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<UnsupportedFile> for BrushImportError {
    fn from(f: UnsupportedFile) -> Self {
        Self::Unsupported(f)
    }
}
impl From<Fault> for BrushImportError {
    fn from(f: Fault) -> Self {
        Self::Fault(f)
    }
}
impl From<CoreError> for BrushImportError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

pub(crate) type Result<T> = std::result::Result<T, Fault>;

impl PatternMode {
    pub(crate) fn from_code(mode: i32) -> PatternMode {
        match mode {
            0 => PatternMode::Bitmap,
            4 => PatternMode::Cmyk,
            7 => PatternMode::Multichannel,
            8 => PatternMode::Duotone,
            9 => PatternMode::Lab,
            other => PatternMode::Other(other),
        }
    }
    fn texts(self) -> (String, String) {
        match self {
            PatternMode::Bitmap => ("ビットマップ".into(), "bitmap".into()),
            PatternMode::Cmyk => ("CMYK".into(), "CMYK".into()),
            PatternMode::Multichannel => ("マルチチャンネル".into(), "multichannel".into()),
            PatternMode::Duotone => ("ダブルトーン".into(), "duotone".into()),
            PatternMode::Lab => ("Lab".into(), "Lab".into()),
            PatternMode::Other(n) => (format!("モード {n}"), format!("mode {n}")),
        }
    }
}

impl PatternRefusal {
    pub(crate) fn texts(&self) -> (String, String) {
        match self {
            Self::ChannelsDiffer => (
                "チャンネルの大きさが違う".into(),
                "its channels differ in size".into(),
            ),
            Self::TooLarge { width, height } => (
                format!("{width}×{height} 画素で、1 辺は 1〜2048 に収まらない"),
                format!("it is {width}x{height} pixels; textures must be 1..2048 per side"),
            ),
            Self::Depth(d) => (
                format!("{d} bit の模様は未対応"),
                format!("{d}-bit patterns are not supported"),
            ),
            Self::Zip => (
                "ZIP 圧縮の画素は未対応".into(),
                "ZIP-compressed pattern data is not supported".into(),
            ),
            Self::Mode(m) => {
                let (ja, en) = m.texts();
                (
                    format!("{ja} モードの模様は未対応"),
                    format!("patterns in {en} mode are not supported"),
                )
            }
            Self::ChannelsMissing => (
                "色のチャンネルが足りない".into(),
                "its colour channels are missing".into(),
            ),
        }
    }
}

impl SizedItem {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Brush => ("ブラシ", "Brush"),
            Self::Tip => ("筆先", "Tip"),
            Self::Pattern => ("模様", "Pattern"),
            Self::Image => ("画像", "Image"),
        }
    }
}

impl Counted {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::NameLength => ("名前の長さ", "name length"),
            Self::PixelData => ("画素データ", "pixel data"),
            Self::SectionLength => ("セクションの長さ", "section length"),
            Self::SampleLength => ("サンプルの長さ", "sample length"),
            Self::StringLength => ("文字列の長さ", "string length"),
            Self::KeyLength => ("キーの長さ", "key length"),
            Self::DataLength => ("データの長さ", "data length"),
            Self::PatternLength => ("模様の長さ", "pattern length"),
            Self::PatternDataLength => ("模様のデータの長さ", "pattern data length"),
            Self::PatternChannelLength => ("模様のチャンネルの長さ", "pattern channel length"),
        }
    }
}

impl Fault {
    /// 日本語と英語の文。
    pub(crate) fn texts(&self) -> (String, String) {
        use Fault::*;
        let side = yolu_core::BrushTip::MAX_SIZE;
        match self {
            Truncated { .. } => (
                "ファイルが途中で終わっています".into(),
                "The file is truncated.".into(),
            ),
            BadCount { what, .. } => {
                let (ja, en) = what.texts();
                (format!("{ja}が不正です"), format!("Invalid {en}."))
            }
            Budget => (
                "筆先・模様が多すぎる、または大きすぎます（メモリの上限）".into(),
                "The tips or patterns are too many or too large (memory limit).".into(),
            ),
            SizeOutOfRange {
                what,
                width,
                height,
            } => {
                let (ja, en) = what.texts();
                (
                    format!("{ja}の大きさ {width}×{height} は 1〜{side} の外です"),
                    format!("{en} size {width}x{height} is outside 1..{side}."),
                )
            }
            GbrHeaderSize { version, size } => (
                format!("版 {version} の GBR のヘッダーの大きさ {size} が不正です"),
                format!("Invalid header size {size} for GBR version {version}."),
            ),
            GbrSignature => (
                "GIMP のブラシの署名がありません".into(),
                "Missing the GIMP brush signature.".into(),
            ),
            GbrNameTooLong => (
                "ブラシの名前が 256 バイトを超えています".into(),
                "The brush name is longer than 256 bytes.".into(),
            ),
            GbrHeaderMismatch => (
                "ヘッダーの大きさが各項目と合いません".into(),
                "Header size does not match its fields.".into(),
            ),
            GbrCinePaint => (
                "16 bit 浮動小数点（CinePaint）のブラシは未対応です".into(),
                "16-bit float (CinePaint) brushes are not supported.".into(),
            ),
            GbrPixelSize(n) => (
                format!("画素のバイト数 {n} は未対応です"),
                format!("Unsupported pixel size {n}."),
            ),
            GbrVersion(n) => (
                format!("GIMP ブラシの版 {n} は未対応です"),
                format!("Unsupported GIMP brush version {n}."),
            ),
            GihHeader => (
                "画像ホースのヘッダーが無い、または長すぎます".into(),
                "The image hose header is missing or too long.".into(),
            ),
            GihCellCount => (
                "画像ホースのヘッダーは 1〜256 のセル数で始まる必要があります".into(),
                "The image hose header must start with a cell count of 1..256.".into(),
            ),
            NotVbr => (
                "GIMP のパラメトリックブラシではありません（GIMP-VBR が無い）".into(),
                "Not a GIMP parametric brush (missing GIMP-VBR).".into(),
            ),
            VbrEndsEarly { line } => (
                format!("パラメトリックブラシが途中で終わっています（{line} 行目）"),
                format!("The parametric brush ends early (line {line})."),
            ),
            VbrNumber { line, min, max } => (
                format!("{line} 行目は {min}〜{max} の数でなければなりません"),
                format!("Line {line} must be a number in {min}..{max}."),
            ),
            VbrShape(shape) => (
                format!("知らない形 '{shape}'"),
                format!("Unknown shape '{shape}'."),
            ),
            VbrVersion(v) => (
                format!("パラメトリックブラシの版 '{v}' は未対応です"),
                format!("Unsupported parametric brush version '{v}'."),
            ),
            AbrVersion(v) => (
                format!("ABR の版 {v} は未対応です"),
                format!("Unsupported ABR version {v}."),
            ),
            AbrSubversion(v) => (
                format!("ABR の副版 {v} は未対応です"),
                format!("Unsupported ABR subversion {v}."),
            ),
            AbrBrushCount(n) => (
                format!("ブラシの数 {n} が不正です"),
                format!("Invalid brush count {n}."),
            ),
            AbrBrushTruncated { index } => (
                format!("{index} 番目のブラシが途中で切れています"),
                format!("Brush {index} is truncated."),
            ),
            AbrBrushType { index, kind } => (
                format!("{index} 番目のブラシの種類 {kind} は未対応です"),
                format!("Unknown brush type {kind} in brush {index}."),
            ),
            AbrTipCount(_) => ("筆先が多すぎます".into(), "Too many tips.".into()),
            AbrSection { .. } => (
                "8BIM のセクションがありません".into(),
                "An 8BIM section is missing.".into(),
            ),
            TipDepth(d) => (
                format!("筆先の {d} bit は未対応です"),
                format!("Unsupported tip depth {d}."),
            ),
            Tip16BitCompressed => (
                "16 bit で圧縮された筆先は未対応です".into(),
                "16-bit compressed tips are not supported.".into(),
            ),
            TipCompression(c) => (
                format!("筆先の圧縮方式 {c} は未対応です"),
                format!("Unsupported tip compression {c}."),
            ),
            PackBits(PackBitsFault::EndedEarly) => (
                "圧縮された 1 行が途中で終わっています".into(),
                "A compressed row ended early.".into(),
            ),
            PackBits(PackBitsFault::Overflow) => (
                "圧縮された 1 行が幅を超えています".into(),
                "A compressed row overflows its width.".into(),
            ),
            AbrNoBrushes => (
                "このファイルに読めるブラシがありません".into(),
                "The file contains no brushes this tool can read.".into(),
            ),
            DescriptorDepth => (
                "設定の入れ子が深すぎます".into(),
                "Descriptor nesting is too deep.".into(),
            ),
            DescriptorItems => (
                "設定の項目が多すぎます".into(),
                "Too many descriptor items.".into(),
            ),
            DescriptorType { kind, .. } => (
                format!("設定の値の型 '{kind}' は未対応です"),
                format!("Unsupported descriptor value type '{kind}'."),
            ),
            DescriptorReference(form) => (
                format!("設定の参照の形 '{form}' は未対応です"),
                format!("Unsupported reference form '{form}'."),
            ),
            DescriptorNotFinite => (
                "設定に有限でない数があります".into(),
                "A descriptor number is not finite.".into(),
            ),
            NotPat => (
                "Photoshop のパターンファイルではありません（8BPT の署名が無い）".into(),
                "Not a Photoshop pattern file (no 8BPT signature).".into(),
            ),
            PatVersion(v) => (
                format!("パターンファイルの版 {v} は未対応です"),
                format!("Unsupported pattern file version {v}."),
            ),
            PatCount(n) => (
                format!("模様の数 {n} が不正です"),
                format!("Invalid pattern count {n}."),
            ),
            PatternVersion(v) => (
                format!("模様の版 {v} は未対応です"),
                format!("Unsupported pattern version {v}."),
            ),
            PatternDataVersion(v) => (
                format!("模様のデータの版 {v} は未対応です"),
                format!("Unsupported pattern data version {v}."),
            ),
            PatternChannelCount(n) => (
                format!("模様のチャンネル数 {n} が不正です"),
                format!("Invalid pattern channel count {n}."),
            ),
            PatternChannelLengthTooShort(n) => (
                format!("模様のチャンネルの長さ {n} が不正です"),
                format!("Invalid pattern channel length {n}."),
            ),
            PatternEmptyRecord => (
                "空の模様の記録があります".into(),
                "An empty pattern record.".into(),
            ),
            NoPatterns => (
                "このファイルに模様がありません".into(),
                "The file contains no patterns.".into(),
            ),
            NotPng => (
                "PNG として読めません".into(),
                "Not a readable PNG image.".into(),
            ),
            PngLimits => (
                "画像が大きすぎます（メモリの上限）".into(),
                "Image too large (memory limit).".into(),
            ),
            SutNotDatabase => (
                "CLIP STUDIO のブラシ（SQLite のデータベース）として読めません".into(),
                "Not a readable CLIP STUDIO brush (SQLite database).".into(),
            ),
            SutNoNodeTable => (
                "ブラシの表（Node）がありません。CLIP STUDIO のブラシではない可能性があります"
                    .into(),
                "The brush table (Node) is missing; this may not be a CLIP STUDIO brush.".into(),
            ),
            SutNoBrushes => (
                "このファイルに読めるブラシがありません".into(),
                "The file contains no brushes this tool can read.".into(),
            ),
            SutLimits => (
                "データベースが大きすぎる、または複雑すぎます（読み取りの上限）".into(),
                "The database is too large or too complex (read limit).".into(),
            ),
        }
    }
}

impl BrushImportError {
    fn texts(&self) -> (String, String) {
        match self {
            Self::Io(e) => (format!("ファイルを読めません（{e}）"), "The file cannot be read.".into()),
            Self::FileTooLarge { .. } => ("ファイルが大きすぎます".into(), "The file is too large.".into()),
            Self::Unsupported(UnsupportedFile::KritaPreset) => (
                "Krita のブラシプリセット（.kpp）は未対応です".into(),
                "Krita brush presets (.kpp) are not supported.".into(),
            ),
            Self::Unsupported(UnsupportedFile::Extension(ext)) => (
                format!("ブラシファイルの種類 '{ext}' は未対応です（対応: abr, pat, gbr, gih, vbr, png, sut）"),
                format!("Unsupported brush file type '{ext}'. Supported: abr, pat, gbr, gih, vbr, png, sut."),
            ),
            Self::Fault(f) => f.texts(),
            Self::NoUsablePattern(skipped) => {
                if skipped.is_empty() {
                    return Fault::NoPatterns.texts();
                }
                let ja: Vec<String> = skipped.iter().map(|s| format!("'{}' は{}", s.name, s.reason.texts().0)).collect();
                let en: Vec<String> = skipped.iter().map(|s| format!("Pattern '{}' was skipped: {}.", s.name, s.reason.texts().1)).collect();
                (format!("使える模様がありません。{}", ja.join("、")), format!("No pattern in the file can be used. {}", en.join(" ")))
            }
            Self::Core(e) => (format!("ブラシの設定が範囲外です（{e}）"), "The brush settings are out of range.".into()),
        }
    }

    /// 英語の文（画面が英語のとき）。OS の診断の本文は含めない。
    pub fn english(&self) -> String {
        self.texts().1
    }
}

impl fmt::Display for BrushImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.texts().0)
    }
}
impl std::error::Error for BrushImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Core(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.texts().0)
    }
}
impl Fault {
    /// 英語の文。
    pub fn english(&self) -> String {
        self.texts().1
    }
}
