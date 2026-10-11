//! Unity 版 .ylp の検証、損失のない正本の読み書き、メモリ上の旧形式移行。
mod archive;
pub mod atomic;
mod bigdoc;
pub use bigdoc::{DocumentSource, SetDocument};
pub mod brushes;
mod composite_png;
mod core_bridge;
pub use core_bridge::{MAX_DOCUMENT_EDGE, MAX_DOCUMENT_LAYERS};
mod distribution;
pub mod export;
pub mod fonts;
mod generation;
pub mod library;
pub mod livelink;
pub mod look;
mod native;
mod package;
pub mod pose;
mod project;
pub mod saved_selections;
mod selection;
pub mod shelf;
pub mod smart;
mod store;
pub use archive::{Archive, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES};
pub use distribution::{Found, Inventory, Removal, IMPORTED_ORIGINAL};
pub use generation::{
    generation_time_ms, utc_stamp, CommitOptions, Committed, Fault, Files, Footprint, Generation,
    GenerationFootprint, GenerationInfo, GenerationStore, LowSpace, RecoveryInfo, SpaceGuard,
    StoreError, INFO_LIMIT, INFO_NAME,
};
pub use native::{
    NativeDocument, NativeField, NativeValue, ADJUST_VERSION, ANTI_ALIAS_VERSION,
    BAKE_PRIORITY_VERSION, EFFECTS_VERSION, MAX_NATIVE_VERSION, MIXING_VERSION, PATHS_VERSION,
    POINT_GRADIENT_VERSION, PROCEDURAL_VERSION, RULERS_VERSION, SEAMS_VERSION, SPLIT_VERSION,
    TEXT_VERSION, UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION,
};
pub use package::{
    Blob, Keep, Limits, Package, Thresholds, MAX_ENTRIES, MAX_ONE_ENTRY, MAX_PART_BYTES,
    OVER_LAYER_PIXELS_DOCUMENT, OVER_LAYER_PIXELS_TOTAL, SOURCE_RELEASED,
};
pub use project::{
    entry_form, FormatInfo, MaterialAsset, MaterialRef, Note, Project, Resource, SetSpec,
    TextureSet, WriterInfo, MAX_FORMAT, MAX_PROJECT_SETS, MODEL_PATH_MAX, RESOURCE_ENTRIES,
    ROOT_ENTRIES, SAVED_SELECTIONS_FORMAT, SET_ENTRIES,
};
pub use selection::{Selection, SelectionTile};
use std::fmt;
pub use store::{
    backup_folder, backup_origin, backups, BackupKeep, FileStamp, PruneFailure, SaveReport,
    SaveStage, SaveTarget, MAX_BACKUPS_TO_KEEP,
};

/// 画面は種類から短い理由を作る。Display は内部の詳細診断を保つ。
#[derive(Debug)]
pub enum Error {
    InvalidData(String),
    /// 予算・上限を超えた（アーカイブ・展開・JSON・画素・レイヤー数・名前の長さなど）。壊れたファイルとは別に言い分けるための種類。
    Budget(String),
    /// まだ正本に書けない中身。黙って落とさず、書けるまで保存を断る。
    Unwritable(Unwritable),
    Io(std::io::Error),
    Json(serde_json::Error),
    Core(yolu_core::CoreError),
    SaveConflict(String),
    /// 新しい版の .ylp（版と、保存したアプリの名前・版。画面は種類から短い理由を作り、書き手は診断として残す）。
    UnsupportedFormat {
        format: i32,
        app: String,
        version: String,
    },
}
/// まだ .ylp の正本に書けない中身（書けるようになるまで、保存を断る）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unwritable {
    /// 塗りつぶしのグラデーションのランプの混色・混合率曲線（Unity 版と共有の並びに形が無い）。
    GeneratorRampMixing,
}
impl fmt::Display for Unwritable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::GeneratorRampMixing => {
                "塗りつぶしのグラデーションのランプの混色は .ylp に書けません"
            }
        })
    }
}
impl Error {
    /// 変換の途中の失敗に、どの項目かを添えて包み直す。予算超過（`Budget`、core の予算）は種類を保ち、画面が壊れたファイルと言い分けられるようにする。
    pub(crate) fn in_context(self, context: impl fmt::Display) -> Self {
        match self {
            Self::Budget(why) => Self::Budget(format!("{context}: {why}")),
            Self::Core(
                e @ (yolu_core::CoreError::SourceBudgetExceeded
                | yolu_core::CoreError::StrokeBudgetExceeded
                | yolu_core::CoreError::WorkingBudgetExceeded),
            ) => Self::Budget(format!("{context}: {e}")),
            Self::Core(e) => Self::InvalidData(format!("{context}: {e}")),
            other => Self::InvalidData(format!("{context}: {other}")),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(text) | Self::Budget(text) | Self::SaveConflict(text) => {
                f.write_str(text)
            }
            Self::Unwritable(what) => what.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::Json(e) => write!(f, "JSONが不正です: {e}"),
            Self::Core(e) => write!(f, "core: {e}"),
            Self::UnsupportedFormat { format, app, version } => write!(
                f,
                ".ylp形式{format}（{app} {version}で保存）は未対応です。対応上限は形式{MAX_FORMAT}です"
            ),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Core(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn check(ok: bool, why: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidData(why.into()))
    }
}
/// `check` の、予算・上限の超過。壊れたファイル（`InvalidData`）とは別の種類にする。
pub(crate) fn check_budget(ok: bool, why: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Budget(why.into()))
    }
}
pub(crate) fn hash(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}
pub(crate) fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn valid_id(s: &str) -> bool {
    s.len() == 36
        && s != "00000000-0000-0000-0000-000000000000"
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
pub(crate) fn guid(b: &[u8]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",b[3],b[2],b[1],b[0],b[5],b[4],b[7],b[6],b[8],b[9],b[10],b[11],b[12],b[13],b[14],b[15])
}

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeExamples;

pub use composite_png::{composite_png, composite_pngs};
impl From<yolu_core::CoreError> for Error {
    fn from(e: yolu_core::CoreError) -> Self {
        Self::Core(e)
    }
}

pub mod psd;

pub mod mesh_map;
