//! Unity 版 .ylp の検証、損失のない正本の読み書き、メモリ上の旧形式移行。
mod archive;
mod native;
mod project;
mod selection;
mod store;
pub use archive::{Archive, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES};
pub use native::{NativeDocument, NativeField, NativeValue};
pub use project::{FormatInfo, Project, Resource, TextureSet, WriterInfo};
pub use selection::{Selection, SelectionTile};
use std::fmt;
pub use store::{FileStamp, SaveTarget};

#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self(format!("JSONが不正です: {e}"))
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn check(ok: bool, why: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(why.into()))
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
