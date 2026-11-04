//! 言語のフォント（Noto Sans CJK の TC・SC・KR。SIL OFL 1.1）を取って置く（`cargo xtask fetch-fonts`）。
//!
//! 中国語・韓国語の画面のフォントは、合わせて約 37 MB あるので、リポジトリにも実行ファイルにも入れず、配布物の `fonts/` に別のファイルとして
//! 入れる（アプリはその言語を選んだときだけ読む。`crates/yolu-app/src/ui/fonts.rs`）。ここは、配布元（notofonts/noto-cjk の
//! Sans2.004 のタグ）から取り、表の大きさと SHA-256 に合うことを確かめて、`crates/yolu-app/assets/fonts-cjk/`（git に入れない）へ置く。
//! 合わないファイルは置かない。配布物を作る段（`bundle`・`installer`）は、置いたファイルを同じ表で確かめ直す。

use crate::{root, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub use yolu_update::fonts::{PinnedFont as Font, LANG_FONTS as FONTS};

/// 配布元のタグ（取り直しても同じ物になるよう、ブランチでなくタグで固定する）。表（`yolu_update::fonts`）の `upstream_dir` がこの下のディレクトリ。
const BASE: &str = "https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/Sans/SubsetOTF";

/// 言語のフォントの許諾の全文（リポジトリに入れている。配布物の `fonts/OFL-Noto.txt`）。
pub const LICENSE: &str = "crates/yolu-app/assets/fonts/OFL-Noto.txt";
/// 配布物の中の言語のフォントのフォルダ（Windows・Linux は実行ファイルの隣、Mac は .app の `Contents/Resources/`）。
pub const PAYLOAD_DIR: &str = "fonts";

/// 置き先（`crates/yolu-app/assets/fonts-cjk/`。アプリの開発の実行も、ここから読む）。
pub fn dir(root: &Path) -> PathBuf {
    root.join("crates/yolu-app/assets/fonts-cjk")
}

fn url(font: &Font) -> String {
    format!("{BASE}/{}/{}", font.upstream_dir, font.file)
}

/// 中身が表の大きさと SHA-256 に合うか（表はアプリと同じ `yolu_update::fonts`）。合わなければ理由。
fn check_bytes(font: &Font, bytes: &[u8]) -> std::result::Result<(), String> {
    font.check(bytes)
}

/// 置いてある言語のフォントが、全部あり、表に合うことを確かめる（配布物を作る前）。
pub fn verify(root: &Path) -> Result<()> {
    let dir = dir(root);
    for font in FONTS {
        let path = dir.join(font.file);
        let bytes = fs::read(&path).map_err(|e| {
            format!(
                "言語のフォント {} がありません（cargo xtask fetch-fonts）: {e}",
                path.display()
            )
        })?;
        check_bytes(font, &bytes)?;
    }
    if !root.join(LICENSE).is_file() {
        return Err(format!("言語のフォントの許諾 {LICENSE} がありません").into());
    }
    Ok(())
}

/// `cargo xtask fetch-fonts`: 無い・合わないファイルを取り直し、表に合う物だけを置く。
pub fn fetch(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(extra) = args.next() {
        return Err(format!("fetch-fonts は引数を取りません: {extra}").into());
    }
    let root = root();
    let dir = dir(&root);
    fs::create_dir_all(&dir)?;
    for font in FONTS {
        let path = dir.join(font.file);
        if fs::read(&path).is_ok_and(|bytes| check_bytes(font, &bytes).is_ok()) {
            println!("{}: 置いてあります", font.file);
            continue;
        }
        let partial = dir.join(format!("{}.part", font.file));
        let _ = fs::remove_file(&partial);
        let status = Command::new("curl")
            .args([
                "--proto",
                "=https",
                "--tlsv1.2",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "3",
                "--output",
            ])
            .arg(&partial)
            .arg(url(font))
            .status()
            .map_err(|e| format!("curl を起動できません: {e}"))?;
        if !status.success() {
            let _ = fs::remove_file(&partial);
            return Err(format!("{} を取れません（{}）", font.file, url(font)).into());
        }
        let bytes = fs::read(&partial)?;
        if let Err(why) = check_bytes(font, &bytes) {
            let _ = fs::remove_file(&partial);
            return Err(why.into());
        }
        fs::rename(&partial, &path)?;
        println!("{}: 取りました（{} バイト）", font.file, font.size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_urls_come_from_the_pinned_tag() {
        for font in FONTS {
            assert!(url(font)
                .starts_with("https://raw.githubusercontent.com/notofonts/noto-cjk/Sans2.004/"));
            assert!(url(font).ends_with(font.file));
        }
    }

    #[test]
    fn bytes_that_do_not_match_the_table_are_refused() {
        let font = &FONTS[0];
        assert!(check_bytes(font, b"short").unwrap_err().contains("大きさ"));
        let same_size = vec![0u8; font.size as usize];
        assert!(check_bytes(font, &same_size)
            .unwrap_err()
            .contains("SHA-256"));
    }

    #[test]
    fn the_license_is_in_the_repository() {
        let text = fs::read_to_string(root().join(LICENSE)).unwrap();
        assert!(text.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(text.contains("Adobe"));
    }
}
