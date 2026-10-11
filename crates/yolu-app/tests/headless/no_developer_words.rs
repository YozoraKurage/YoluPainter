//! 画面の文に開発の言葉を置かない（試作の名残・内部の名前・実装の用語）。`no_instruction_text.rs` が操作の説明文を見るのと同じく、
//! `src/` の（試験を除く）ソースの文字列リテラルを見る。ユーザーが読む文は、名前・状態・短い理由だけにする。
//!
//! 見ない範囲: コメント、`#[cfg(test)]` から後ろ・`tests.rs`、`expect(…)`・`panic!(…)` などの開発者向けの文、`=>` の左
//! （`lang/errors.rs` の、core のエラー文を英語にする表のキー。core の文そのものを直すときに、表と一緒に変える）。

use std::path::{Path, PathBuf};

/// 開発の言葉。（言葉, 使ってはいけない理由）
const BANNED: [(&str, &str); 22] = [
    ("M2 の試作", "試作の名残"),
    ("M2 prototype", "試作の名残"),
    ("Rust 版", "実装の言語は利用者に関係ない"),
    ("(Rust)", "実装の言語は利用者に関係ない"),
    ("core の", "内部の階層の名前"),
    ("core で", "内部の階層の名前"),
    ("core document", "内部の階層の名前"),
    ("スナップショット", "内部の仕組みの名前"),
    ("Model snapshot", "内部の仕組みの名前"),
    ("BVH", "内部の仕組みの名前"),
    ("ダブが", "ブラシの内部の単位"),
    ("書き直した正本", "保存の内部の数"),
    ("正本", "保存の内部の言葉（利用者には「プロジェクト」）"),
    (
        "文書",
        "作品は「プロジェクト」、大きさのことは「キャンバス」と書く",
    ),
    ("updated documents", "保存の内部の数"),
    ("（形式 ", "形式の番号は利用者に関係ない"),
    ("(format ", "形式の番号は利用者に関係ない"),
    ("YoluPainter-rs", "古い内部の名前"),
    ("遮蔽のレイ", "内部の仕組みの名前"),
    ("ray budget", "内部の仕組みの名前"),
    ("予算を超えました（取り消した）", "どの予算かが分からない文"),
    ("Not supported on this device", "GPU のことは「GPU」と書く"),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// 1 行の文字列リテラルの中身（`with_keys` が偽なら、`=>` の左にあるものは除く。エスケープは `\"` だけ読む）。
fn literals(line: &str, with_keys: bool) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => break,
            '"' => {
                let start = i;
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        text.push(chars[i + 1]);
                        i += 2;
                    } else {
                        text.push(chars[i]);
                        i += 1;
                    }
                }
                let after: String = chars[(i + 1).min(chars.len())..].iter().collect();
                let before: String = chars[..start].iter().collect();
                let before = before.trim_end();
                // `=>` の左（表のキー）と、`expect("…")`・`panic!("…")` などの開発者向けの文は見ない
                let developer = [
                    "expect(",
                    "panic!(",
                    "unreachable!(",
                    "assert!(",
                    "debug_assert!(",
                ]
                .iter()
                .any(|m| before.ends_with(m));
                if (with_keys || !after.trim_start().starts_with("=>")) && !developer {
                    out.push(text);
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn scan(root: &Path) -> Vec<String> {
    scan_words(root, &BANNED, false)
}

fn scan_words(root: &Path, banned: &[(&str, &str)], with_keys: bool) -> Vec<String> {
    scan_literals(root, with_keys, |literal| banned_in(literal, banned))
}

/// 断る言葉を含む語（断る言葉の一部だが、別の意味で普通に使う語）。この語は見ずに、残りで調べる。
const ALLOWED: [&str; 1] = ["機械的"];

/// `literal` の中の、断る言葉（と理由）。`ALLOWED` の語は数えない。
fn banned_in(literal: &str, banned: &[(&str, &str)]) -> Option<(String, String)> {
    let text = ALLOWED.iter().fold(literal.to_owned(), |text, allowed| {
        text.replace(allowed, "")
    });
    banned
        .iter()
        .find(|(word, _)| text.contains(word))
        .map(|(word, why)| (word.to_string(), why.to_string()))
}

/// `hit` が文字列リテラルごとに（言葉, 理由）を返したものを「ファイル:行」つきで集める。
fn scan_literals(
    root: &Path,
    with_keys: bool,
    hit: impl Fn(&str) -> Option<(String, String)>,
) -> Vec<String> {
    let mut files = Vec::new();
    rust_files(root, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        // 試験の部品（`tests.rs`・`tests/`）は見ない
        if file.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let body = text.split("#[cfg(test)]").next().unwrap();
        for (n, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for literal in literals(line, with_keys) {
                if let Some((word, why)) = hit(&literal) {
                    found.push(format!(
                        "{}:{}: 「{word}」（{why}）: {literal}",
                        file.strip_prefix(root).unwrap().display(),
                        n + 1
                    ));
                }
            }
        }
    }
    found
}

#[test]
fn no_screen_text_in_the_app_uses_a_developer_word() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = scan(&root);
    assert!(
        found.is_empty(),
        "画面の文に開発の言葉:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_dab_refusal_texts_of_the_core_are_plain_words_too() {
    // yolu-core の `DabRefusal` の Display（日本語）。アプリの文（lang/errors.rs）と同じく、利用者の読む文
    use yolu_core::geometry::DabRefusal::*;
    for refusal in [
        SnapshotChanged,
        InvalidArguments,
        BindingMismatch,
        TriangleBudget,
        PixelBudget,
        VisibilityBudget,
        BvhBudget,
        MemoryBudget,
    ] {
        let text = refusal.to_string();
        for word in ["スナップショット", "ダブ", "レイ", "BVH", "予算"] {
            assert!(!text.contains(word), "{refusal:?}: 「{word}」: {text}");
        }
    }
}

/// どの crate の文でも使わない書き方（core の文はそのまま画面に出るものがあり、`lang/errors.rs` の表のキーにもなる）。
/// 英語の直訳の言い回し（layer・island・window・tool・canvas・font）は、使う人の言葉で書く。
const EVERYWHERE: [(&str, &str); 9] = [
    ("画布", "「キャンバス」と書く"),
    ("層", "「レイヤー」と書く"),
    ("島", "「アイランド」と書く"),
    ("窓", "「ウィンドウ」と書く"),
    ("道具", "「ツール」と書く"),
    ("字体", "「フォント」と書く"),
    ("装置", "GPU は「GPU」、PC は「PC」と書く"),
    (
        "機械",
        "PC は「PC」と書く（「機械的」は普通の語なので断らない）",
    ),
    ("機材", "GPU は「GPU」、PC は「PC」と書く"),
];

#[test]
fn no_message_in_any_crate_uses_a_literal_translation_word() {
    // core・io・gpu・ops の文と、`lang/errors.rs` の表のキー（`=>` の左）も見る。core の文を直したら、表のキーも同じ文に直す
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for name in [
        "yolu-core",
        "yolu-io",
        "yolu-gpu",
        "yolu-app",
        "yolu-ops",
        "yolu-mcp",
        "yolu-cli",
    ] {
        let root = crates.join(name).join("src");
        found.extend(
            scan_words(&root, &EVERYWHERE, true)
                .into_iter()
                .map(|f| format!("{name}/{f}")),
        );
    }
    assert!(
        found.is_empty(),
        "文に英語の直訳の言い回し（画布・層・島・窓・道具・字体）と分かりにくい言葉（装置・機械・機材）:\n{}",
        found.join("\n")
    );
}

#[test]
fn no_message_in_any_crate_calls_the_project_or_the_canvas_a_document() {
    // core・io・gpu・ops の文（画面にそのまま出る）と、`lang/errors.rs` の表のキーも見る。作品は「プロジェクト」、大きさは「キャンバス」
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let banned = [(
        "文書",
        "作品は「プロジェクト」、大きさのことは「キャンバス」と書く",
    )];
    let mut found = Vec::new();
    for name in [
        "yolu-core",
        "yolu-io",
        "yolu-gpu",
        "yolu-app",
        "yolu-ops",
        "yolu-mcp",
        "yolu-cli",
    ] {
        let root = crates.join(name).join("src");
        found.extend(
            scan_words(&root, &banned, true)
                .into_iter()
                .map(|f| format!("{name}/{f}")),
        );
    }
    assert!(found.is_empty(), "文に「文書」:\n{}", found.join("\n"));
}

/// 画面の言葉に「棚」（英語は shelf）を使わない。プロジェクトの品（.ylp に保存される画像・スマート素材・ブラシ）は「アセット」
/// （"assets"・"the project's assets"）、自分のフォルダは「ライブラリ」（"library"）。ウィジェットの id・キャッシュのファイルの名前などの
/// 識別子（空白を含まない文字列）は、コードの名前のまま（保存の形と試験の口を変えないため）なので、英語は空白を含む文だけを見る。
fn shelf_word(literal: &str) -> Option<(String, String)> {
    let why = "画面の言葉は「アセット」（プロジェクトの品）か「ライブラリ」（自分のフォルダ）";
    if literal.contains('棚') {
        return Some(("棚".to_owned(), why.to_owned()));
    }
    if literal.to_ascii_lowercase().contains("shelf") && literal.contains(char::is_whitespace) {
        return Some(("shelf".to_owned(), why.to_owned()));
    }
    None
}

#[test]
fn no_screen_text_in_any_crate_calls_the_projects_assets_a_shelf() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    for name in ["yolu-core", "yolu-io", "yolu-gpu", "yolu-app"] {
        let root = crates.join(name).join("src");
        found.extend(
            scan_literals(&root, true, shelf_word)
                .into_iter()
                .map(|f| format!("{name}/{f}")),
        );
    }
    assert!(found.is_empty(), "文に「棚」・shelf:\n{}", found.join("\n"));
}

/// 公開の文書（docs と docs/en・README・crate の README・プラグインの文書）の地の文にも「棚」・shelf を使わない。コードの書式（`…` と
/// コードのブロック）の中は、コードの名前（`yolu_io::shelf::Shelf` など）なので見ない。CHANGELOG は出荷した版の記録なので対象外。
fn prose_with_shelf(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut fenced = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // 1 行の中の `…` を除く（奇数番目の区切りの間がコード）
        let prose: String = line
            .split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            .join(" ");
        if prose.contains('棚') || prose.to_ascii_lowercase().contains("shelf") {
            found.push(format!("{}: {}", n + 1, line.trim()));
        }
    }
    found
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            // 試験の入力（fixtures・golden）と生成物は文書ではない
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !matches!(name.as_str(), "tests" | "target" | "node_modules" | "data") {
                markdown_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

#[test]
fn no_public_document_calls_the_projects_assets_a_shelf() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![root.join("README.md"), root.join("README.en.md")];
    for dir in ["docs", "crates", "plugin"] {
        markdown_files(&root.join(dir), &mut files);
    }
    files.sort();
    let mut checked = 0;
    let mut found = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        checked += 1;
        for hit in prose_with_shelf(&text) {
            found.push(format!(
                "{}:{hit}",
                file.strip_prefix(&root).unwrap().display()
            ));
        }
    }
    assert!(checked > 20, "文書を見つけられない（{checked} 件）");
    assert!(
        found.is_empty(),
        "公開の文書に「棚」・shelf:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_document_check_skips_code_and_finds_prose() {
    assert!(prose_with_shelf("棚に入れる").len() == 1);
    assert!(prose_with_shelf("Added to the shelf").len() == 1);
    assert!(prose_with_shelf("`yolu_io::shelf::Shelf` で扱います").is_empty());
    assert!(prose_with_shelf("```\nshelf.json\n```").is_empty());
    assert!(prose_with_shelf("`a` shelf `b`").len() == 1);
    assert!(prose_with_shelf("アセットに入れる").is_empty());
}

#[test]
fn the_shelf_word_check_skips_identifiers_but_not_sentences() {
    assert!(shelf_word("棚に入れました").is_some());
    assert!(shelf_word("Added to the shelf").is_some());
    assert!(shelf_word("Shelf is full").is_some());
    assert!(shelf_word("shelf.library.add").is_none());
    assert!(shelf_word("shelf:abc").is_none());
    assert!(shelf_word("yolu-shelf-cache-a-1").is_none());
    assert!(shelf_word("アセットに入れました").is_none());
    assert!(shelf_word("Added to the project's assets").is_none());
}

#[test]
fn the_hardware_words_are_refused_but_a_mechanical_manner_is_not() {
    for refused in [
        "GPU の装置がありません",
        "ドライバーが無い機械",
        "この機材は対応していません",
    ] {
        assert!(banned_in(refused, &EVERYWHERE).is_some(), "{refused}");
    }
    // 「機械的」は別の意味の普通の語
    assert!(banned_in("機械的に並べる", &EVERYWHERE).is_none());
    // 「機械的」のあとに断る言葉が続けば、そちらは断る
    assert_eq!(
        banned_in("機械的な機械", &EVERYWHERE).map(|(word, _)| word),
        Some("機械".to_owned())
    );
    assert!(banned_in("この GPU は対応していません", &EVERYWHERE).is_none());
}

#[test]
fn the_scanner_finds_a_developer_word_and_skips_table_keys_and_comments() {
    assert_eq!(
        literals(r#"let a = "core の文書"; // "BVH""#, false),
        ["core の文書"]
    );
    assert_eq!(literals(r#""キー" => "Value""#, false), ["Value"]);
    assert_eq!(literals(r#""キー" => "Value""#, true), ["キー", "Value"]);
}
