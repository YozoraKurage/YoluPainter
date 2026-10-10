//! 命令の失敗。種類（`code`）は決まった名前で、理由の文は日本語と英語の両方を持つ（`message`）。
//! 断った命令は何も変えない（文書の命令は `Document::batch` の巻き戻し、ファイルの命令は検証済みの一時ファイルからの置き換え）。

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use yolu_core::effects::catalog::ParamError;
use yolu_core::CoreError;

use crate::text::Text;

/// 失敗の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 命令の JSON が読めない・引数が足りない・知らない欄がある。
    InvalidRequest,
    /// 知らない命令の名前。
    UnknownCommand,
    /// 命令の版が合わない。
    UnsupportedVersion,
    /// 開いている文書が無い。
    NoDocument,
    /// セット・レイヤー・効果・チャンネル・効果の種類が無い。
    NotFound,
    /// 名前が複数に当たる（IDで指定する）。
    Ambiguous,
    /// 値が範囲の外・型が違う・選択肢にない。
    InvalidValue,
    /// 読むだけのセット（編集できない中身がある）。
    ReadOnly,
    /// そのレイヤー・チャンネル・種類にはできない操作、この版では扱わないもの。
    Unsupported,
    /// 壊す操作に `confirm: true` が無い。
    ConfirmRequired,
    /// ファイルの道が使えない（作業のフォルダの外・名前の形）。
    PathRefused,
    /// 予算・上限を超える。
    Budget,
    /// 文書が断った操作（ロック・結合の条件など）。
    Refused,
    /// 今はできない（ストロークの途中・まとめた編集の途中）。
    Busy,
    /// 保存先が外で変わっている・別の保存が進んでいる。
    Conflict,
    /// .ylp・ファイルが壊れている・読めない形。
    InvalidProject,
    /// ファイルの読み書きに失敗した。
    Io,
    /// 取り消した。
    Cancelled,
    /// 想定していない失敗。
    Internal,
}

/// 命令の失敗。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OpError {
    pub code: ErrorCode,
    pub message: Text,
    /// 失敗の詳しい中身（候補・ファイルの名前・診断など。種類による）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl fmt::Display for OpError {
    /// 英語の文（言語を選ぶ所は `message.pick`）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message.en)
    }
}
impl std::error::Error for OpError {}

impl OpError {
    pub fn new(code: ErrorCode, ja: impl Into<String>, en: impl Into<String>) -> Self {
        OpError {
            code,
            message: Text::new(ja, en),
            data: None,
        }
    }
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
    pub fn invalid_request(ja: impl Into<String>, en: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, ja, en)
    }
    pub fn invalid_value(ja: impl Into<String>, en: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidValue, ja, en)
    }
    pub fn not_found(what: Noun, name: &str) -> Self {
        let (ja, en) = what.words();
        Self::new(
            ErrorCode::NotFound,
            format!("{ja}「{name}」が見つかりません"),
            format!("No {en} named \"{name}\""),
        )
        .with_data(json!({"name": name}))
    }
    pub fn no_document() -> Self {
        Self::new(
            ErrorCode::NoDocument,
            "開いているプロジェクトがありません（doc.open で開く）",
            "No document is open (open one with doc.open)",
        )
    }
    pub fn confirm_required(
        ja: impl Into<String>,
        en: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        let e = Self::new(ErrorCode::ConfirmRequired, ja, en);
        match data {
            Some(d) => e.with_data(d),
            None => e,
        }
    }
    /// もうあるファイルの置き換えに `confirm: true` が無い（保存。`files` は置き換えるファイルの道）。
    pub fn replace_confirm_required(files: &[String]) -> Self {
        Self::confirm_required(
            "もうあるファイルを置き換えます。confirm: true を付けてください",
            "The existing file would be replaced; pass confirm: true",
            Some(json!({"files": files})),
        )
    }
    /// 保存先の名前が .ylp で終わらない。
    pub fn ylp_name_required(path: &str) -> Self {
        Self::new(
            ErrorCode::PathRefused,
            "保存先の名前は .ylp で終わります",
            "The file name must end with .ylp",
        )
        .with_data(json!({"path": path}))
    }
    /// まだファイルになっていない文書の上書き保存（保存先を `save_as` で言う）。
    pub fn no_file_to_save() -> Self {
        Self::invalid_value(
            "まだファイルになっていないプロジェクトです。save_as で保存先を指定してください",
            "The document is not a file yet; give a destination with save_as",
        )
        .with_data(json!({"reason": "no_file"}))
    }
    /// core の失敗。理由の文は日本語が core の診断、英語は種類ごとの文（細かい理由は `data.detail` に日本語で残す）。
    pub fn from_core(e: &CoreError) -> Self {
        let ja = e.to_string();
        let detail = json!({"detail": ja});
        let (code, en): (ErrorCode, String) = match e {
            CoreError::MergeRefused(_) | CoreError::MergeAppearance(_) => {
                (ErrorCode::Refused, "Cannot merge these layers".into())
            }
            CoreError::Cancelled => (ErrorCode::Cancelled, "Cancelled".into()),
            CoreError::LayerLocked { .. } => (
                ErrorCode::Refused,
                "The layer or a parent group is locked".into(),
            ),
            CoreError::InactiveEffect { .. } => (
                ErrorCode::Refused,
                "An inactive effect cannot be baked".into(),
            ),
            CoreError::InvalidArgument(what) => (
                ErrorCode::InvalidValue,
                if what.is_ascii() {
                    format!("Invalid value: {what}")
                } else {
                    "A value is out of range or not allowed".into()
                },
            ),
            CoreError::LayerNotFound => (ErrorCode::NotFound, "Layer not found".into()),
            CoreError::ChannelNotFound => (ErrorCode::NotFound, "Channel not found".into()),
            CoreError::StrokeActive | CoreError::NoActiveStroke => {
                (ErrorCode::Busy, "A stroke is in progress".into())
            }
            CoreError::BatchActive => (
                ErrorCode::Busy,
                "Not allowed inside a batch of edits".into(),
            ),
            CoreError::TileUnreadable => (
                ErrorCode::Io,
                "A tile could not be read back from the disk cache".into(),
            ),
            CoreError::Unsupported(what) => (
                ErrorCode::Unsupported,
                if what.is_ascii() {
                    format!("Unsupported: {what}")
                } else {
                    "This operation is not supported here".into()
                },
            ),
            CoreError::SourceBudgetExceeded => (
                ErrorCode::Budget,
                "The pixel budget would be exceeded".into(),
            ),
            CoreError::StrokeBudgetExceeded => (
                ErrorCode::Budget,
                "The stroke budget would be exceeded".into(),
            ),
            CoreError::WorkingBudgetExceeded => (
                ErrorCode::Budget,
                "The working memory limit would be exceeded".into(),
            ),
            CoreError::Clipboard(_) => {
                (ErrorCode::Refused, "The pixel operation was refused".into())
            }
        };
        OpError {
            code,
            message: Text::new(ja, en),
            data: Some(detail),
        }
    }
    /// 効果の値の欄の失敗（効果の種類の表）。
    pub fn from_param(e: &ParamError) -> Self {
        let ja = e.to_string();
        match e {
            ParamError::UnknownKind(kind) => Self::new(
                ErrorCode::NotFound,
                format!("知らない効果の種類: {kind}（effect.list_kinds で一覧）"),
                format!("Unknown effect kind: {kind} (see effect.list_kinds)"),
            ),
            ParamError::WrongTarget { kind, adjustment } => Self::new(
                ErrorCode::Unsupported,
                ja,
                if *adjustment {
                    format!("{kind} cannot be used as an adjustment layer setting")
                } else {
                    format!("{kind} cannot be used as a filter stack effect")
                },
            ),
            ParamError::NotEditable { kind } => Self::new(
                ErrorCode::Unsupported,
                ja,
                format!("{kind} has parts (lists, curves, references) that values cannot create or change"),
            ),
            ParamError::UnknownParam { kind, name } => Self::new(
                ErrorCode::InvalidValue,
                ja,
                format!("{kind} has no parameter named {name}"),
            ),
            ParamError::WrongType { name, expected } => Self::new(
                ErrorCode::InvalidValue,
                ja,
                format!(
                    "{name} must be {}",
                    match *expected {
                        "整数（数）" => "an integer",
                        "数" => "a number",
                        "真偽" => "a boolean",
                        _ => "one of the listed choices (a string)",
                    }
                ),
            ),
            ParamError::OutOfRange { name, min, max } => Self::new(
                ErrorCode::InvalidValue,
                ja,
                format!("{name} must be between {min} and {max}"),
            )
            .with_data(json!({"param": name, "min": min, "max": max})),
            ParamError::NotInteger { name } => {
                Self::new(ErrorCode::InvalidValue, ja, format!("{name} must be an integer"))
            }
            ParamError::UnknownOption { name, options } => Self::new(
                ErrorCode::InvalidValue,
                ja,
                format!("{name} must be one of: {}", options.join(", ")),
            )
            .with_data(json!({"param": name, "options": options})),
            ParamError::Refused(core) => Self::from_core(core),
        }
    }
    /// .ylp・ファイルの失敗。日本語は yolu-io の診断、英語は種類ごとの文。
    pub fn from_io(e: &yolu_io::Error) -> Self {
        use yolu_io::Error as E;
        let ja = e.to_string();
        match e {
            E::Core(core) => Self::from_core(core),
            E::Io(io) => Self::new(ErrorCode::Io, ja.clone(), format!("File operation failed: {io}")),
            E::Json(_) => Self::new(ErrorCode::InvalidProject, ja, "Invalid JSON in the project"),
            E::InvalidData(_) => {
                Self::new(ErrorCode::InvalidProject, ja, "Invalid or unsupported project data")
            }
            E::Budget(_) => {
                Self::new(ErrorCode::Budget, ja, "A size, count or memory limit would be exceeded")
            }
            E::Unwritable(_) => Self::new(
                ErrorCode::Unsupported,
                ja,
                "Some content cannot be written to .ylp yet, so the save was refused",
            ),
            E::SaveConflict(_) => {
                Self::new(ErrorCode::Conflict, ja, "The save target changed or is in use")
            }
            E::UnsupportedFormat { format, app, version } => Self::new(
                ErrorCode::Unsupported,
                ja,
                format!(
                    "Unsupported .ylp format {format} (saved by {app} {version}; supported up to {})",
                    yolu_io::MAX_FORMAT
                ),
            ),
        }
    }
    /// 書き出し（PNG）の失敗。
    pub fn from_export(e: &yolu_io::export::ExportError) -> Self {
        use yolu_io::export::ExportError as E;
        let ja = e.to_string();
        match e {
            E::InvalidName(name) => {
                Self::new(ErrorCode::InvalidValue, ja, format!("Not a usable file name: {name}"))
            }
            E::NameClash(names) => Self::new(
                ErrorCode::InvalidValue,
                ja,
                "Several images would be written to the same file name",
            )
            .with_data(json!({"names": names})),
            E::WouldReplace(paths) => Self::confirm_required(
                ja,
                "Existing files would be replaced; pass confirm: true",
                Some(json!({"files": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()})),
            ),
            E::NotAFile(path) => Self::new(
                ErrorCode::PathRefused,
                ja,
                "The destination is not a regular file",
            )
            .with_data(json!({"path": path.display().to_string()})),
            E::Cancelled => Self::new(ErrorCode::Cancelled, ja, "Cancelled; nothing was replaced"),
            E::InvalidImage(_) => Self::new(ErrorCode::InvalidValue, ja, "The image cannot be written as a PNG"),
            E::Calc(calc) => Self::from_calc(calc),
            E::Io(_) => Self::new(ErrorCode::Io, ja, "A file operation failed"),
            E::Verify(_) => Self::new(
                ErrorCode::Io,
                ja,
                "The written PNG did not read back identically; nothing was replaced",
            ),
            E::Partial { written, cause: _ } => Self::new(
                ErrorCode::Io,
                ja,
                format!("The export failed while replacing files; {} file(s) were already replaced", written.len()),
            )
            .with_data(json!({"written": written.iter().map(|w| w.path.display().to_string()).collect::<Vec<_>>()})),
        }
    }
    /// 書き出しの計算（core）の失敗。
    pub fn from_calc(e: &yolu_core::export::ExportError) -> Self {
        use yolu_core::export::ExportError as E;
        let ja = e.to_string();
        match e {
            E::Core(core) => Self::from_core(core),
            E::Cancelled => Self::new(ErrorCode::Cancelled, ja, "Cancelled"),
            E::WorkingBudgetExceeded { needed, allowed } => Self::new(
                ErrorCode::Budget,
                ja,
                format!(
                    "The export needs {needed} bytes of working memory; the limit is {allowed}"
                ),
            ),
            E::InvalidArgument(_) => Self::new(
                ErrorCode::InvalidValue,
                ja,
                "An export value is out of range",
            ),
        }
    }
}

impl From<CoreError> for OpError {
    fn from(e: CoreError) -> Self {
        OpError::from_core(&e)
    }
}
impl From<ParamError> for OpError {
    fn from(e: ParamError) -> Self {
        OpError::from_param(&e)
    }
}
impl From<yolu_io::Error> for OpError {
    fn from(e: yolu_io::Error) -> Self {
        OpError::from_io(&e)
    }
}

/// 「無い」を言うときの対象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noun {
    Set,
    Layer,
    Effect,
    Channel,
    Template,
}

impl Noun {
    fn words(self) -> (&'static str, &'static str) {
        match self {
            Noun::Set => ("テクスチャセット", "texture set"),
            Noun::Layer => ("レイヤー", "layer"),
            Noun::Effect => ("効果", "effect"),
            Noun::Channel => ("チャンネル", "channel"),
            Noun::Template => ("書き出しのテンプレート", "export template"),
        }
    }
}
