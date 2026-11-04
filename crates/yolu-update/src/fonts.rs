//! 言語のフォント（Noto Sans CJK の TC・SC・KR。SIL OFL 1.1）の表。`cargo xtask fetch-fonts`（取る・配布物を作る前に確かめる）と、
//! アプリ（言語のフォントを読むとき）が同じ表を使い、ファイルの大きさと SHA-256 が合う物だけを使う。
//! 取る場所（URL）は `cargo xtask fetch-fonts` だけが持つ（アプリは取らない）。

/// 表の 1 つのファイル。
pub struct PinnedFont {
    /// ファイルの名前（配布物の `fonts/`・開発の置き場も同じ名前）。
    pub file: &'static str,
    /// 配布元のディレクトリ（`Sans/SubsetOTF/<dir>/`）。
    pub upstream_dir: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// 言語のフォントの表。
pub const LANG_FONTS: &[PinnedFont] = &[
    PinnedFont {
        file: "NotoSansTC-Regular.otf",
        upstream_dir: "TC",
        size: 5_683_368,
        sha256: "5bab0cb3c1cf89dde07c4a95a4054b195afbcfe784d69d75c340780712237537",
    },
    PinnedFont {
        file: "NotoSansTC-Bold.otf",
        upstream_dir: "TC",
        size: 5_839_972,
        sha256: "55420b259eb119bf5f2a0aadba10cf9d736c12d64ab93e78546d69ef5f43558b",
    },
    PinnedFont {
        file: "NotoSansSC-Regular.otf",
        upstream_dir: "SC",
        size: 8_331_336,
        sha256: "faa6c9df652116dde789d351359f3d7e5d2285a2b2a1f04a2d7244df706d5ea9",
    },
    PinnedFont {
        file: "NotoSansSC-Bold.otf",
        upstream_dir: "SC",
        size: 8_543_168,
        sha256: "c6cb5a93abaa9edc8ee7463b7ebb7f42d618d40e6ed2f7a5371c97b0b64767c0",
    },
    PinnedFont {
        file: "NotoSansKR-Regular.otf",
        upstream_dir: "KR",
        size: 4_644_748,
        sha256: "69975a0ac8472717870aefeab0a4d52739308d90856b9955313b2ad5e0148d68",
    },
    PinnedFont {
        file: "NotoSansKR-Bold.otf",
        upstream_dir: "KR",
        size: 4_816_044,
        sha256: "5a6ceb287ed2fc6cfc6213144ebea68cbd94b20fc9eb873d8486493bf02d9bda",
    },
];

/// ファイルの名前から表の行。
pub fn find(file: &str) -> Option<&'static PinnedFont> {
    LANG_FONTS.iter().find(|font| font.file == file)
}

impl PinnedFont {
    /// 中身が表の大きさと SHA-256 に合うか。合わなければ理由（日本語）。
    pub fn check(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() as u64 != self.size {
            return Err(format!(
                "{} の大きさが {} バイトです（表は {}）",
                self.file,
                bytes.len(),
                self.size
            ));
        }
        let digest = crate::sha256(bytes);
        if digest != self.sha256 {
            return Err(format!(
                "{} の SHA-256 が {digest} です（表は {}）",
                self.file, self.sha256
            ));
        }
        Ok(())
    }
}

/// 表にある名前のファイルの中身が、表に合うか。表に無い名前は断る。
pub fn check(file: &str, bytes: &[u8]) -> Result<(), String> {
    find(file)
        .ok_or_else(|| format!("{file} は言語のフォントの表にありません"))?
        .check(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_unique_files_from_the_pinned_tag() {
        let mut names = std::collections::BTreeSet::new();
        for font in LANG_FONTS {
            assert!(names.insert(font.file), "{}", font.file);
            assert_eq!(font.sha256.len(), 64);
            assert!(font.file.ends_with(".otf"));
            assert!(font.file.contains(font.upstream_dir));
        }
        assert_eq!(LANG_FONTS.len(), 6, "TC・SC・KR の普通と太字");
    }

    #[test]
    fn bytes_that_do_not_match_the_table_are_refused() {
        let font = &LANG_FONTS[0];
        assert!(font.check(b"short").unwrap_err().contains("大きさ"));
        let same_size = vec![0u8; font.size as usize];
        assert!(font.check(&same_size).unwrap_err().contains("SHA-256"));
        assert!(check("Unknown.otf", b"").is_err());
        assert!(check(font.file, b"short").is_err());
    }
}
