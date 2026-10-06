//! ファイルの道の決まり。絶対パスはそのまま使い、相対パスは作業のフォルダからで、`..` で作業のフォルダの外へ出るものは断る
//! （AI が組んだ相対パスで、意図しない場所へ書かない）。
//!
//! 字面だけの検査で、作業のフォルダの中のシンボリックリンクが外を指していても追わない。絶対パスは利用者・呼び手が決めた場所として許す。
//! 使えない形（空・NUL・ドライブだけの相対・ルートだけの指定）は断る。

use std::path::{Component, Path, PathBuf};

use serde_json::json;

use crate::error::{ErrorCode, OpError};

/// 道を決める作業のフォルダ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathPolicy {
    base: PathBuf,
}

impl PathPolicy {
    /// `base`（相対パスの起点）。相対で渡されたら、今のフォルダからの絶対パスにする。
    pub fn new(base: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(PathPolicy { base: std::path::absolute(base.as_ref())? })
    }
    /// プロセスの今のフォルダを作業のフォルダにする。
    pub fn current_dir() -> std::io::Result<Self> {
        Self::new(std::env::current_dir()?)
    }
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// 命令の道（文字列）を、使うパスにする。
    pub fn resolve(&self, text: &str) -> Result<PathBuf, OpError> {
        let refuse = |ja: String, en: String| {
            OpError::new(ErrorCode::PathRefused, ja, en).with_data(json!({"path": text}))
        };
        if text.is_empty() || text.contains('\0') {
            return Err(refuse(
                "パスが空か、使えない文字を含みます".to_owned(),
                "The path is empty or contains an unusable character".to_owned(),
            ));
        }
        let path = Path::new(text);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        let mut kept: Vec<&std::ffi::OsStr> = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => kept.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    if kept.pop().is_none() {
                        return Err(refuse(
                            format!("「{text}」は作業のフォルダの外へ出ます（.. で上へ出られません）。絶対パスで指してください"),
                            format!("\"{text}\" leaves the working folder (\"..\" may not climb out of it); use an absolute path"),
                        ));
                    }
                }
                // ドライブだけ・ルートだけの相対（`C:foo`・`\foo`）は、どこを指すか曖昧なので断る
                Component::Prefix(_) | Component::RootDir => {
                    return Err(refuse(
                        format!("「{text}」はドライブ・ルートの指定が不完全です。絶対パスで指してください"),
                        format!("\"{text}\" names a drive or root without being absolute; use a full absolute path"),
                    ));
                }
            }
        }
        if kept.is_empty() {
            return Err(refuse(
                format!("「{text}」はファイルを指していません"),
                format!("\"{text}\" does not name a file"),
            ));
        }
        let mut out = self.base.clone();
        out.extend(kept);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PathPolicy {
        PathPolicy::new(std::env::temp_dir().join("yolu-ops-policy")).unwrap()
    }

    #[test]
    fn relative_paths_start_at_the_working_folder() {
        let p = policy();
        assert_eq!(p.resolve("a/b.ylp").unwrap(), p.base().join("a").join("b.ylp"));
        assert_eq!(p.resolve("./a.ylp").unwrap(), p.base().join("a.ylp"));
        // 中で .. を使って戻っても、外へ出なければよい
        assert_eq!(p.resolve("a/../b.ylp").unwrap(), p.base().join("b.ylp"));
    }

    #[test]
    fn climbing_out_of_the_working_folder_is_refused_with_a_reason() {
        let p = policy();
        for bad in ["../x.ylp", "a/../../x.ylp", "..", "a/../.."] {
            let e = p.resolve(bad).unwrap_err();
            assert_eq!(e.code, ErrorCode::PathRefused, "{bad}");
            assert!(e.message.ja.contains("外へ出ます") && e.message.en.contains("leaves"), "{bad}");
        }
    }

    #[test]
    fn absolute_paths_are_allowed_as_given_and_odd_forms_are_refused() {
        let p = policy();
        let abs = std::env::temp_dir().join("elsewhere").join("x.ylp");
        assert_eq!(p.resolve(abs.to_str().unwrap()).unwrap(), abs);
        for bad in ["", ".", "./", "a\0b"] {
            assert_eq!(p.resolve(bad).unwrap_err().code, ErrorCode::PathRefused, "{bad:?}");
        }
    }
}
