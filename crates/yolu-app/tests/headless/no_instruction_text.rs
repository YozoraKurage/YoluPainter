//! 画面に操作の説明文を置かない（Unity 版の `NoInstructionTextTests` と同じ決まり。ユーザーの強い方針）。
//! 「〜をクリックして点を追加」「ブラシはその上から塗ります」のような使い方の文、説明の段落、空の状態の案内を、画面の文字に置かない。
//! 画面の文字は名前・状態・短い理由だけで、説明はツールチップに置く。さらに、開発用の数（メモリの MiB・上げたタイル・三角形の数・
//! 合成の方式）を、状態の帯とビューの隅に出さない。
//!
//! 見る範囲: `src/` の（テストを除く）ソースの文字列リテラルすべてと、ソースがキーで引く .ftl の文（`lang.t("キー")`・
//! `lang.f("キー", …)` の所に、そのキーの日英の文があるものとして見る。core の断りの理由の英語の文 `reason-*` も見る）。
//! ツールチップに渡す文字（`tooltip`・`on_hover_text` などの引数、`tooltip:` の欄、`let tip = …`、名前に tooltip・tip を
//! 含む関数の中）は、説明を置く場所なので見ない。ソースの直書きの文は、ツールチップの引数の場所を、ソースの関数の定義（引数の名前が
//! tooltip・tip・hint）から読む。.ftl の文は、キーの終わりが `-tip` ならツールチップ。
//! 見ない範囲: ツールチップ、組み立て方が上の書き方に当たらない文（変数に入れてから別の場所で出す文）。ボタンやメニューや選択肢の名前も文字列としては
//! 見る（指示の言葉を含まなければ通り、含めば引っかかる）。名前そのものが指示の言葉（「押す」「Tap」など）を含む選択肢だけは、`ALLOWED_NAMES` に
//! 原文で並べて許す（断りの文の `ALLOWED` とは別。文は入れない）。
//! 文として判定するのは「. か 。で終わる」か「. のあとに続きがある」文字列だけ（名前や状態の短い語は、指示の言葉があるときだけ見る）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ───────── 規則 ─────────

/// 操作の指示の言葉（英語は単語、日本語は語）。
const EN_WORDS: [&str; 9] = [
    "click",
    "drag",
    "press",
    "tap",
    "double-click",
    "right-click",
    "please",
    "you",
    "hold down",
];
const JA_WORDS: [&str; 8] = [
    "クリック",
    "ドラッグ",
    "押して",
    "押す",
    "押し",
    "ください",
    "選ぶと",
    "しましょう",
];
/// 命令形の動詞（文の頭・「; 」「, or 」のあと）。
const VERBS: [&str; 46] = [
    "Read",
    "Place",
    "Choose",
    "Select",
    "Wait",
    "Make",
    "Turn",
    "Check",
    "Unlock",
    "Bake",
    "Pick",
    "Enter",
    "Type",
    "Switch",
    "Load",
    "Enable",
    "Disable",
    "Set",
    "Add",
    "Use",
    "Move",
    "Merge",
    "Finish",
    "Show",
    "Hide",
    "Rasterize",
    "Redraw",
    "Reset",
    "Plan",
    "Try",
    "Reduce",
    "Remove",
    "Open",
    "Close",
    "Apply",
    "Copy",
    "Edit",
    "Paint",
    "Restore",
    "Delete",
    "Create",
    "Reload",
    "Export",
    "Rename",
    "Fix",
    "Drag",
];

fn ascii_word_at(lower: &str, word: &str) -> bool {
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(at) = lower[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        let before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        from = start + 1;
    }
    false
}

fn first_word(part: &str) -> &str {
    let part = part.trim_start();
    let end = part
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
        .unwrap_or(part.len());
    &part[..end]
}

/// 命令形の動詞で始まる文か（「Export canceled」のような、動詞の名詞形＋過去分詞の状態は除く）。
fn starts_with_imperative(part: &str) -> bool {
    let first = first_word(part);
    if !VERBS.contains(&first) {
        return false;
    }
    let rest = part.trim_start()[first.len()..].trim_start();
    let second = first_word(rest);
    !rest.is_empty() && !second.ends_with("ed")
}

/// 文に分ける（. ; : ! ? のあとの空白で切る）。
fn sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, (at, c)) in chars.iter().enumerate() {
        if matches!(c, '.' | ';' | ':' | '!' | '?')
            && chars.get(i + 1).is_some_and(|(_, n)| n.is_whitespace())
        {
            parts.push(&text[start..at + c.len_utf8()]);
            start = at + c.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

/// 画面の文字として使ってよいか（操作の指示でないか）の逆: 操作の指示の言い回しを含むなら true。`sentence` なら、命令形で始まる文・
/// 「…を先に」「もう一度…」の案内も見る。
fn looks_like_instruction(text: &str, sentence: bool) -> bool {
    if text.is_empty() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if EN_WORDS.iter().any(|w| ascii_word_at(&lower, w)) {
        return true;
    }
    // 「hold Alt」「hold the …」のような保持の指示（「must hold 9 floats」のような取り決めは指示ではない）
    if let Some(at) = lower.find("hold ") {
        let rest = &text[at + 5..];
        let next = first_word(rest);
        if matches!(next, "Alt" | "Ctrl" | "Shift" | "the")
            || (next.len() == 1 && next.chars().all(|c| c.is_ascii_uppercase()))
        {
            return true;
        }
    }
    if JA_WORDS.iter().any(|w| text.contains(w)) {
        return true;
    }
    // 「〜すると〜します。」「点を置くと、〜ます。」の使い方の説明（日本語の文）
    if ["すると", "れば", "たら", "と、"]
        .iter()
        .any(|w| text.contains(w))
        && (text.contains("ます。") || text.ends_with("ます"))
        && !text.contains("ません")
    {
        return true;
    }
    if !sentence {
        return false;
    }
    let trimmed = text.trim().trim_start_matches(['.', ';', ':', ' ']);
    if !(trimmed.ends_with('.') || trimmed.contains(". ")) {
        return false; // 名前や状態の短い語は文として見ない
    }
    // 文の途中の「…; bake after it is applied」「…, edit it」のような、動詞で続く案内
    if !trimmed.ends_with('?') {
        for marker in ["; ", ", "] {
            for (at, _) in trimmed.match_indices(marker) {
                let w = first_word(&trimmed[at + marker.len()..]);
                if VERBS.iter().any(|v| v.to_ascii_lowercase() == w) {
                    return true;
                }
            }
        }
    }
    for part in sentences(trimmed) {
        let part = part.trim();
        if part.ends_with('?') {
            continue; // 確かめの疑問文（ダイアログ）
        }
        if starts_with_imperative(part) {
            return true;
        }
        // 「…, or Merge …」のあとの動詞
        let mut rest = part;
        while let Some(at) = rest.find(", or ") {
            rest = &rest[at + 5..];
            if starts_with_imperative(rest) {
                return true;
            }
        }
        let lower = part.to_ascii_lowercase();
        if lower.trim_end_matches(['.', ',', ';']).ends_with("first") || lower.contains("first.") {
            return true;
        }
        if let Some(again) = lower.find("again") {
            let before = &lower[..again];
            if [
                "bake", "plan", "set", "redraw", "reset", "try", "choose", "select",
            ]
            .iter()
            .any(|v| ascii_word_at(before, v))
            {
                return true;
            }
        }
    }
    false
}

// ───────── ソースから文字列リテラルを集める ─────────

#[derive(Clone, Debug)]
struct Literal {
    file: String,
    line: usize,
    text: String,
    /// ツールチップに渡す文字（説明を置いてよい場所）。
    tooltip: bool,
    /// この文字を含む関数の名前。
    function: Option<String>,
}

struct Frame {
    kind: char,
    call: String,
    /// `x.call(` の形（メソッドの呼び出し）
    method: bool,
    arg: usize,
    field: Option<String>,
}

/// .ftl のキーで文を引く口（`lang.t("キー")`・`lang.f("キー", …)`。メソッドの呼び出しだけ）。
const KEY_CALLS: [&str; 2] = ["t", "f"];

/// ツールチップの名前か（`tooltip`・`tip`・`hint`・`help` を、`_` で分けた語に含む。`own_tip`・`mode_tooltip`・`RANGE_TIP_JA` など）。
fn tooltip_name(name: &str) -> bool {
    name.to_ascii_lowercase()
        .split('_')
        .any(|part| matches!(part, "tooltip" | "tip" | "hint" | "help"))
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `#[cfg(test)]` を付けた項目（試験のモジュール・試験用の関数・`use`）を除く。項目は、属性の次の行から、`;` で終わる行（`mod x;`・`use`）か、
/// 行頭の `}`（rustfmt の整形で、最上位の項目の終わり）までで、モジュールの試験が途中にあっても後ろの本番のコードは残る。
/// 行数は変えない（除いた行は空行にする）ので、見つけた文字列の行番号はファイルの行と合う。
fn strip_test_items(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = vec![true; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() != "#[cfg(test)]" {
            i += 1;
            continue;
        }
        // 属性の行（`#[…]`）を飛ばして、項目の最初の行へ
        let mut head = i + 1;
        while head < lines.len() && lines[head].trim_start().starts_with("#[") {
            head += 1;
        }
        let mut end = head;
        if head < lines.len() && !lines[head].trim_end().ends_with(';') {
            while end + 1 < lines.len() && lines[end] != "}" {
                end += 1;
            }
        }
        for flag in keep.iter_mut().take(end.min(lines.len() - 1) + 1).skip(i) {
            *flag = false;
        }
        i = end + 1;
    }
    let mut out = String::with_capacity(text.len());
    for (line, kept) in lines.iter().zip(&keep) {
        if *kept {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// 試験を除いたソース（ファイル名が tests.rs で終わるファイルと、`#[cfg(test)]` を付けた項目を除く）。
pub(crate) fn production_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    let mut out = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("tests.rs") {
            continue;
        }
        let text = strip_test_items(&std::fs::read_to_string(&path).unwrap());
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        out.push((relative, text));
    }
    out
}

/// ツールチップの引数の場所（関数の定義から: 引数の名前が tooltip・tip・hint なものの、`self` を除いた番号）。
fn tooltip_parameters(sources: &[(String, String)]) -> BTreeMap<String, BTreeSet<usize>> {
    let mut table: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    // egui の `on_hover_text` は最初の引数
    table.entry("on_hover_text".into()).or_default().insert(0);
    for (_, text) in sources {
        let mut rest = text.as_str();
        while let Some(at) = rest.find("fn ") {
            let before_ok = at == 0 || !rest.as_bytes()[at - 1].is_ascii_alphanumeric();
            rest = &rest[at + 3..];
            if !before_ok {
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let after = &rest[name.len()..];
            // ジェネリクス `<…>` を飛ばして `(` へ
            let Some(open) = after.find('(') else {
                continue;
            };
            if after[..open].contains(['{', ';']) {
                continue;
            }
            let params = &after[open + 1..];
            let (mut depth, mut end) = (0i32, params.len());
            for (i, c) in params.char_indices() {
                match c {
                    '(' | '[' | '<' | '{' => depth += 1,
                    ')' | ']' | '>' | '}' => {
                        if depth == 0 {
                            end = i;
                            break;
                        }
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            // 引数を最上位のカンマで分ける（`->` の `>` が深さを狂わせないよう、引数の中だけを見る）
            let list = &params[..end];
            let (mut depth, mut start, mut index) = (0i32, 0usize, 0usize);
            let mut pieces = Vec::new();
            let prev: Vec<char> = list.chars().collect();
            let mut byte = 0usize;
            for (ci, c) in prev.iter().enumerate() {
                match c {
                    '(' | '[' | '<' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    '>' if ci == 0 || prev[ci - 1] != '-' => depth -= 1,
                    ',' if depth == 0 => {
                        pieces.push(&list[start..byte]);
                        start = byte + 1;
                    }
                    _ => {}
                }
                byte += c.len_utf8();
            }
            pieces.push(&list[start..]);
            for piece in pieces {
                let piece = piece.trim();
                if piece.is_empty() {
                    continue;
                }
                let param = piece.split(':').next().unwrap().trim();
                let param = param
                    .trim_start_matches("mut ")
                    .trim_start_matches('&')
                    .trim();
                if param == "self" || param.ends_with("self") && !param.contains(' ') {
                    continue;
                }
                if tooltip_name(param) {
                    table.entry(name.clone()).or_default().insert(index);
                }
                index += 1;
            }
        }
    }
    table
}

/// 画面に出ない記録の口（`YOLU_PEN_LOG` のファイルにだけ書く行の形）。記録の行の文字列は、ここにまとめて置く。
const LOG_ONLY: [&str; 1] = ["pen/log.rs"];

fn collect_literals() -> Vec<Literal> {
    let sources = production_sources();
    let table = tooltip_parameters(&sources);
    let keys = crate::ftl_keys::texts_by_key();
    let mut out = Vec::new();
    for (file, text) in &sources {
        if LOG_ONLY.contains(&file.as_str()) {
            continue;
        }
        lex(file, text, &table, &keys, &mut out);
    }
    // core の断りの理由（`lang.reason(…)` が理由の種類から引く。キーはソースに無い）の日英の文。日本語の `Reason::ja` の写しのうち、
    // 「ください」で終わる 2 キーだけは外す（core の文なので、core の文を直すときに外す）
    const CORE_POLITE: [&str; 2] = [
        "reason-anchor-reads-height-not-normal",
        "reason-path-ribbon-invalid",
    ];
    for message in crate::ftl_keys::messages().into_iter().filter(|m| {
        m.key.starts_with("reason-") && !(m.lang == "ja" && CORE_POLITE.contains(&m.key.as_str()))
    }) {
        let key = message.key;
        for text in message.texts {
            out.push(Literal {
                file: format!("locales/{}/reason.ftl ({key})", message.lang),
                line: 0,
                text: crate::ftl_keys::rust_placeholders(&text),
                tooltip: false,
                function: None,
            });
        }
    }
    out
}

fn lex(
    file: &str,
    src: &str,
    table: &BTreeMap<String, BTreeSet<usize>>,
    keys: &BTreeMap<String, Vec<String>>,
    out: &mut Vec<Literal>,
) {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let (mut i, mut line) = (0usize, 1usize);
    let mut frames: Vec<Frame> = Vec::new();
    let mut depth = 0usize;
    let mut fns: Vec<(String, usize)> = Vec::new();
    let mut pending_fn: Option<String> = None;
    let mut last_ident: Option<String> = None;
    // 直前の字句が `.` か・最後の名前が `.` の後ろか（`x.t(` を `t(` と見分ける）
    let mut previous_was_dot = false;
    let mut last_ident_method = false;
    let mut previous_was_fn = false;
    let mut let_name: Option<String> = None;
    let mut after_let = false;
    let starts = |i: usize, s: &str| {
        s.chars()
            .enumerate()
            .all(|(k, c)| chars.get(i + k) == Some(&c))
    };
    while i < n {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if starts(i, "//") {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts(i, "/*") {
            let mut nest = 0;
            while i < n {
                if starts(i, "/*") {
                    nest += 1;
                    i += 2;
                } else if starts(i, "*/") {
                    nest -= 1;
                    i += 2;
                    if nest == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        // 文字列（生・バイト列を含む）
        let raw_prefix = if c == 'r' || (c == 'b' && chars.get(i + 1) == Some(&'r')) {
            let mut k = i + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while chars.get(k) == Some(&'#') {
                hashes += 1;
                k += 1;
            }
            (chars.get(k) == Some(&'"')).then_some((k + 1, hashes))
        } else {
            None
        };
        let string_start = if let Some((k, _)) = raw_prefix {
            Some(k)
        } else if c == '"' {
            Some(i + 1)
        } else if c == 'b' && chars.get(i + 1) == Some(&'"') {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut j) = string_start {
            let start_line = line;
            let mut buf = String::new();
            if let Some((_, hashes)) = raw_prefix {
                let closing: String = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                while j < n && !starts(j, &closing) {
                    if chars[j] == '\n' {
                        line += 1;
                    }
                    buf.push(chars[j]);
                    j += 1;
                }
                j += closing.chars().count();
            } else {
                while j < n && chars[j] != '"' {
                    if chars[j] == '\\' {
                        match chars.get(j + 1) {
                            Some('\n') => {
                                j += 2;
                                line += 1;
                                while j < n && chars[j].is_whitespace() {
                                    if chars[j] == '\n' {
                                        line += 1;
                                    }
                                    j += 1;
                                }
                                continue;
                            }
                            Some('n') => buf.push('\n'),
                            Some('t') => buf.push('\t'),
                            Some('"') => buf.push('"'),
                            Some('\\') => buf.push('\\'),
                            Some('\'') => buf.push('\''),
                            Some('u') => {
                                while j < n && chars[j] != '}' {
                                    j += 1;
                                }
                                buf.push('?');
                                j += 1;
                                continue;
                            }
                            Some(other) => {
                                buf.push('\\');
                                buf.push(*other);
                            }
                            None => {}
                        }
                        j += 2;
                        continue;
                    }
                    if chars[j] == '\n' {
                        line += 1;
                    }
                    buf.push(chars[j]);
                    j += 1;
                }
                j += 1;
            }
            // ツールチップか: 祖先の呼び出しの引数の場所・欄の名前・`let tip`・関数の名前
            let mut tooltip = frames.iter().any(|f| {
                (f.kind == '('
                    && table
                        .get(&f.call)
                        .is_some_and(|indexes| indexes.contains(&f.arg)))
                    || f.field.as_deref().is_some_and(tooltip_name)
            });
            tooltip |= let_name.as_deref().is_some_and(tooltip_name);
            let function = fns.last().map(|(name, _)| name.clone());
            tooltip |= function.as_deref().is_some_and(tooltip_name);
            let key_call = frames.last().is_some_and(|f| {
                f.kind == '(' && f.method && f.arg == 0 && KEY_CALLS.contains(&f.call.as_str())
            });
            if key_call {
                // .ftl のキー: そのキーの日英の文を、この場所の文として見る（ツールチップかはキーの `-tip`）
                for text in keys.get(&buf).into_iter().flatten() {
                    out.push(Literal {
                        file: file.to_owned(),
                        line: start_line,
                        text: crate::ftl_keys::rust_placeholders(text),
                        tooltip: buf.ends_with("-tip"),
                        function: function.clone(),
                    });
                }
            } else {
                out.push(Literal {
                    file: file.to_owned(),
                    line: start_line,
                    text: buf,
                    tooltip,
                    function,
                });
            }
            i = j;
            last_ident = None;
            previous_was_dot = false;
            continue;
        }
        if c == '\'' {
            // 文字か、ライフタイム
            let is_char = (chars.get(i + 2) == Some(&'\'') && chars.get(i + 1) != Some(&'\\'))
                || (chars.get(i + 1) == Some(&'\\'));
            if is_char {
                let mut j = i + 1;
                if chars[j] == '\\' {
                    j += 2;
                    while j < n && chars[j] != '\'' {
                        j += 1;
                    }
                } else {
                    j += 1;
                }
                i = j + 1;
            } else {
                i += 1;
                while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
            }
            last_ident = None;
            continue;
        }
        if is_ident_start(c) {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            if previous_was_fn {
                pending_fn = Some(word.clone());
            }
            if after_let && word != "mut" {
                let_name = Some(word.clone());
                after_let = false;
            }
            if matches!(word.as_str(), "let" | "const" | "static") {
                after_let = true;
            }
            previous_was_fn = word == "fn";
            last_ident_method = previous_was_dot;
            previous_was_dot = false;
            last_ident = Some(word);
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            last_ident = None;
            continue;
        }
        match c {
            '(' | '[' | '{' => {
                frames.push(Frame {
                    kind: c,
                    call: if c == '(' {
                        last_ident.clone().unwrap_or_default()
                    } else {
                        String::new()
                    },
                    method: c == '(' && last_ident.is_some() && last_ident_method,
                    arg: 0,
                    field: None,
                });
                if c == '{' {
                    depth += 1;
                    if let Some(name) = pending_fn.take() {
                        fns.push((name, depth));
                    }
                }
            }
            ')' | ']' | '}' => {
                frames.pop();
                if c == '}' {
                    if fns.last().is_some_and(|(_, d)| *d == depth) {
                        fns.pop();
                    }
                    depth = depth.saturating_sub(1);
                }
            }
            ',' => {
                if let Some(frame) = frames.last_mut() {
                    frame.arg += 1;
                    frame.field = None;
                }
            }
            ';' => {
                let_name = None;
                after_let = false;
                pending_fn = None;
            }
            ':' => {
                let double = chars.get(i + 1) == Some(&':') || (i > 0 && chars[i - 1] == ':');
                if !double {
                    if let (Some(frame), Some(name)) = (frames.last_mut(), last_ident.clone()) {
                        frame.field = Some(name);
                    }
                }
            }
            _ => {}
        }
        if !c.is_whitespace() && c != ':' {
            last_ident = None;
        }
        if !c.is_whitespace() {
            previous_was_fn = false;
            previous_was_dot = c == '.';
        }
        i += 1;
    }
}

/// 画面の文字として見るもの（日本語を含む、または英語の語句。コード・シェーダー・識別子は除く）。
fn is_screen_text(text: &str) -> bool {
    if text.chars().count() > 600
        || text.contains("@group")
        || text.contains("fn ") && text.contains(';')
    {
        return false;
    }
    let cjk = text.chars().any(|c| {
        ('\u{3040}'..='\u{30ff}').contains(&c)
            || ('\u{4e00}'..='\u{9fff}').contains(&c)
            || ('\u{ff00}'..='\u{ffef}').contains(&c)
    });
    if cjk {
        return true;
    }
    // 英語: 空白で区切った 2 語以上の語句
    let words = text
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_ascii_alphabetic()))
        .count();
    text.chars().count() >= 6 && words >= 2
}

/// 指示の言い回しを含むが、断った理由や起きたことを言っているだけの文（原文）。新しい文が引っかかったら、使い方の説明でないかを
/// 確かめ、説明なら状態か理由に書き直す（ここに足すのは、断った理由・起きたこと・確かめの文だけ）。
const ALLOWED: [&str; 2] = [
    // 回転・反転を断った理由（描いている・ドラッグしている間）
    "描いている間・ドラッグの間は回せません。",
    "Cannot rotate during a stroke or drag.",
];

/// 指示の言葉（「押す」「Tap」など）を名前の中に含むが、選択肢の名前そのもので、使い方の文ではない物（原文）。選択肢の名前だけを入れる。文は入れない。
const ALLOWED_NAMES: [&str; 7] = [
    // ショートカットの設定の、ツールのキーの動き方の選択肢
    "押すと切り替え",
    "Switch on Press",
    "押している間だけ",
    "短く押すと切り替え・長押しで押している間だけ",
    "Tap to Switch, Hold for Temporary",
    "短押し／長押し",
    "Tap / Hold",
];

fn offenders(filter: impl Fn(&Literal) -> bool, rule: impl Fn(&str) -> bool) -> Vec<String> {
    collect_literals()
        .into_iter()
        .filter(|l| {
            !l.tooltip
                && filter(l)
                && is_screen_text(&l.text)
                && !ALLOWED.contains(&l.text.as_str())
                && !ALLOWED_NAMES.contains(&l.text.as_str())
        })
        .filter(|l| rule(&l.text))
        .map(|l| format!("{}:{}: {}", l.file, l.line, l.text.replace('\n', "\\n")))
        .collect()
}

// ───────── 試験 ─────────

/// 画面の文字（ツールチップを除く）が、操作の指示の言い回しを含まない。
#[test]
fn texts_on_screen_are_names_states_and_reasons_not_instructions() {
    let literals = collect_literals();
    // 走査が空振りしていない: ソースの文字列を数百件以上見つけ、ツールチップの場所と画面の文字の場所を見分けている
    assert!(literals.len() > 2000, "リテラル {}", literals.len());
    let screen = literals
        .iter()
        .filter(|l| !l.tooltip && is_screen_text(&l.text))
        .count();
    let tooltips = literals
        .iter()
        .filter(|l| l.tooltip && is_screen_text(&l.text))
        .count();
    assert!(
        screen > 800 && tooltips > 100,
        "画面 {screen}・ツールチップ {tooltips}"
    );
    // 見分けの例: 色の欄の「押すと…」はツールチップ、描くレイヤーが無いという理由は画面の文字
    let find = |needle: &str| {
        literals
            .iter()
            .find(|l| l.text.contains(needle))
            .unwrap_or_else(|| panic!("{needle}"))
    };
    assert!(find("押すとメインの色と入れ替えます").tooltip);
    assert!(find("Click to swap with the foreground color").tooltip);
    assert!(!find("描くレイヤーがありません。").tooltip);

    let found = offenders(|_| true, |text| looks_like_instruction(text, true));
    assert!(
        found.is_empty(),
        "画面の文字が操作の説明に読める（使い方はツールチップへ。画面には状態か短い理由だけ）:\n{}",
        found.join("\n")
    );
}

/// 開発用の数（タイル・三角形・合成の方式・文書のメモリ）を、状態の帯とビューの隅に出さない。ユーザーの頼みの例外は、状態の帯の右端の
/// 版・ビルドと、このプロセスの使っているメモリ・GPU の量だけ（`usage.rs`。`chrome.rs` が画面で確かめる）。
#[test]
fn the_status_bar_and_the_view_corners_show_no_developer_numbers() {
    const DEVELOPER_WORDS: [&str; 14] = [
        "MiB",
        "KiB",
        "GiB",
        "タイル",
        "tile",
        "三角形",
        "triangle",
        "頂点",
        "vertices",
        "CPU",
        "GPU",
        "compositing",
        "合成の方式",
        "メモリ",
    ];
    // 状態の帯・ビューの隅を描く関数の中の文字には、開発用の言葉が無い
    const CHROME: [(&str, &str); 4] = [
        ("shell.rs", "status_bar"),
        ("shell.rs", "status_text"),
        ("canvas/mod.rs", "draw_corner"),
        ("panels/view3d.rs", "corner"),
    ];
    let literals = collect_literals();
    let sources = production_sources();
    for (file, function) in CHROME {
        // 表の関数はソースにある（名前を変えたら表も直す。見つからないまま通すと、黙って確かめが外れる）
        let source = sources
            .iter()
            .find(|(f, _)| f == file)
            .unwrap_or_else(|| panic!("{file} が無い（CHROME の表を直す）"));
        assert!(
            [format!("fn {function}("), format!("fn {function}<")]
                .iter()
                .any(|head| source.1.contains(head.as_str())),
            "{file} に関数 {function} が無い（CHROME の表を直す）"
        );
        let inside: Vec<&Literal> = literals
            .iter()
            .filter(|l| l.file == file && l.function.as_deref() == Some(function))
            .collect();
        // 3D の隅と状態の帯は固定の文字を持たないことがある（関数はあるが文字が無いのは許す）。見つかったものは開発用の言葉を含まない
        for l in inside {
            for word in DEVELOPER_WORDS {
                assert!(
                    !l.text.to_lowercase().contains(&word.to_lowercase()),
                    "{}:{} {function} に開発用の言葉「{word}」: {}",
                    l.file,
                    l.line,
                    l.text
                );
            }
        }
    }
    // 状態の帯の左は何も出さない（直前の操作の結果と理由は、小さな知らせ `toast` が短く出す）。右端の版・ビルドと使っているメモリは、
    // ユーザーの頼みの例外で `usage` が決める（数の文字は `usage.rs` が作り、ここの関数は文書・履歴・タイル・描画の数を読まない）
    let shell = production_sources()
        .into_iter()
        .find(|(file, _)| file == "shell.rs")
        .unwrap()
        .1;
    let start = shell.find("pub fn status_text").expect("status_text");
    let end = shell[start..]
        .find("/// Live Link の入口の印の色。")
        .expect("次の関数")
        + start;
    let body = &shell[start..end];
    for forbidden in [
        "allocated_bytes",
        "history_bytes",
        "total_tiles",
        "stats",
        "uploaded",
        "doc.",
        "MiB",
    ] {
        assert!(
            !body.contains(forbidden),
            "状態の帯が {forbidden} を読んでいる"
        );
    }
    // 画面の文字（ツールチップ以外）のうち、メモリの量・タイルの数・合成の方式は、理由として断る文にだけある（原文を確かめて足す）
    const REASONS: [&str; 6] = [
        "ファイルが {} MiB を超えています",
        // FBX を読まない理由（ファイルの大きさと上限。yolu-model の日本語の文の英語）
        "File too large ({:.1} MiB, maximum {:.0} MiB)",
        "タイルの大きさ {} は共有メモリで使えません（16〜1024 の 2 の冪）",
        "Invalid shared tile size {} (power of two, 16–1024)",
        "3D を描けません（GPU なし）",
        "Cannot draw 3D (no GPU)",
    ];
    // 設定のウィンドウのメモリの予算の値（MiB は値の単位で、説明でも内部の数でもない。状態の帯・ビューの隅には出さない）
    const SETTING_VALUES: [&str; 3] = ["自動（{} MiB）", "Auto ({} MiB)", "{n} MiB"];
    let leaks: Vec<String> = literals
        .iter()
        .filter(|l| !l.tooltip && is_screen_text(&l.text))
        .filter(|l| {
            [
                "MiB",
                "CPU で合成",
                "CPU compositing",
                "上げたタイル",
                "Uploaded tiles",
                "Layers {",
                "History {",
            ]
            .iter()
            .any(|w| l.text.contains(w))
        })
        .filter(|l| !REASONS.contains(&l.text.as_str()))
        .filter(|l| !(l.file == "prefs/mod.rs" && SETTING_VALUES.contains(&l.text.as_str())))
        .map(|l| format!("{}:{}: {}", l.file, l.line, l.text))
        .collect();
    assert!(
        leaks.is_empty(),
        "開発用の数が画面の文字にある:\n{}",
        leaks.join("\n")
    );
}

/// 名指しされた文言（`{}` は引数の置き場）。.ftl の文は `{}` に寄せてから比べる（`ftl_keys::rust_placeholders`）。
const NAMED: [&str; 9] = [
    "上げたタイル",
    "Uploaded tiles",
    "まだマテリアルに付いていない",
    "Unassigned (slot",
    "メイン  #",
    "サブ  #",
    "Main  #",
    "Sub  #",
    "{} 三角形",
];

fn named_texts(literals: &[Literal]) -> Vec<String> {
    literals
        .iter()
        .filter(|l| !l.tooltip && NAMED.iter().any(|n| l.text.contains(n)))
        .map(|l| format!("{}:{}: {}", l.file, l.line, l.text))
        .collect()
}

/// ユーザーが名指しで「余計な文章やめろ」と言った文言は、ソースに画面の文字として無い（ツールチップは除く。`chrome.rs` が画面でも確かめる）。
#[test]
fn the_texts_the_user_named_are_not_screen_text_in_the_source() {
    let found = named_texts(&collect_literals());
    assert!(
        found.is_empty(),
        "名指しされた文言が画面の文字にある:\n{}",
        found.join("\n")
    );
}

/// 止めた形の文（`{} 三角形`）を .ftl に入れると、`{ $名前 }`・選びの候補の中の置き場でも、名指しの照らし合わせが見つける。
#[test]
fn a_named_text_in_an_ftl_is_found_even_with_placeholders() {
    let dir = std::env::temp_dir().join(format!("yolu-named-ftl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.ftl");
    std::fs::write(
        &path,
        "sample-triangles = { $count } 三角形\n\
         sample-triangles-plural = { $count ->\n    [one] { $count } triangle\n   *[other] { $count } 三角形\n}\n\
         sample-fine = { $count } 個\n",
    )
    .unwrap();
    let (messages, errors) = crate::ftl_keys::parse("ja", &path);
    std::fs::remove_dir_all(&dir).ok();
    assert!(errors.is_empty(), "{errors:?}");
    let keys: BTreeMap<String, Vec<String>> =
        messages.into_iter().map(|m| (m.key, m.texts)).collect();
    let source = r#"
        fn show(ui: &mut Ui, lang: Lang, n: usize) {
            ui.label(lang.f("sample-triangles", &[("count", &n)]));
            ui.label(lang.f("sample-triangles-plural", &[("count", &n)]));
            ui.label(lang.f("sample-fine", &[("count", &n)]));
        }
    "#;
    let mut literals = Vec::new();
    lex("sample.rs", source, &BTreeMap::new(), &keys, &mut literals);
    let found = named_texts(&literals);
    // 行 3（直の文）と行 4（選びの `other` の候補）だけが当たり、当たらない文（行 5・選びの `one` の候補）は当たらない
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].starts_with("sample.rs:3:") && found[1].starts_with("sample.rs:4:"));
    assert!(found.iter().all(|f| f.ends_with("{} 三角形")), "{found:?}");
}

/// 規則が、止めたい文（ユーザーが挙げた例など）を捉え、状態と理由を通すこと。
#[test]
fn the_rule_catches_instructions_and_passes_states_and_reasons() {
    for bad in [
        "Click the 2D canvas or the model to add points; drag a point to move it; Delete removes the last one.",
        "2D キャンバスかモデルをクリックして点を追加。点はドラッグで移動、Delete で最後の点を削除。",
        "ブラシはその上から塗ります。T を押したまま…",
        "Drag to move, corners to scale, outside to rotate. Arrow keys nudge.",
        "ドラッグか矢印キー（Shift で 10 px）で、レイヤーを動かします。",
        "Press T to place the stencil.",
        "Right-click for more.",
        "Please bake the maps.",
        "ベイクしてください。",
        "テクスチャセットを選ぶと、そのセットのレイヤーが出ます。",
        "Select a layer to see its properties.",
        "Hold Shift to constrain the angle.",
        "点を置くと、そこから線が伸びます。",
    ] {
        assert!(looks_like_instruction(bad, true), "{bad}");
    }
    for bad in [
        "Choose a model in Texture Set, or 3D ▸ Demo Cube. The original prefab is never instantiated.",
        "Make a selection first.",
        "Bake the ID map first.",
        "Wait for the model preparation to finish, or cancel it.",
        "Enable the channel before cutting from it.",
        "Choose at least one texture set.",
        "The material changed since the plan was made; nothing was changed. Plan again.",
        "The pose is being changed; bake after it is applied (when the slider is released).",
        "The layer below is a group. Merge the group first, or move the layer into it.",
        "The baked maps were not used because {0} changed during the bake. Bake again.",
        "先に選択範囲を作ってください。",
    ] {
        assert!(looks_like_instruction(bad, true), "{bad}");
    }
    for good in [
        "No model",
        "Layer mask",
        "A stroke is in progress.",
        "No clone source",
        "写し元なし",
        "Pen pressure",
        "ペンの筆圧",
        "モデルなし",
        "待機中",
        "接続中",
        "Waiting",
        "Version mismatch",
        "Live Link: Unity を待っています（yolupainter-livelink）",
        "Live Link: Waiting for Unity (yolupainter-livelink)",
        "No effect in 3D",
        "3D では効きません",
        "選択範囲なし",
        "描くレイヤーがありません。",
        "No layer to paint on.",
        "ストロークを取り消しました。",
        "Stroke cancelled.",
        "読むだけのテクスチャセットには描けません",
        "Cannot paint on a read-only texture set",
        "保存していない変更があります。変更を捨てて終わりますか？",
        "There are unsaved changes. Discard them and quit?",
        "The bake was discarded because {0} changed; the previous maps are unchanged.",
        "The texture set would hold more than {0} layers. Nothing was placed.",
        "Tips must hold 1..256 non-null tips.",
        "ファイルが {} MiB を超えています",
        "Loaded model.fbx (2 notices)",
        "model.fbx を読み込みました（知らせ 2 件）",
    ] {
        assert!(!looks_like_instruction(good, true), "{good}");
    }
}

/// 開発用の数の規則: 状態の帯に出さない言葉を、規則の外の画面の文字から見分ける（ここでは見分ける関数の入り口だけを確かめる）。
#[test]
fn the_scan_tells_tooltips_from_screen_text() {
    let sources = vec![(
        "sample.rs".to_owned(),
        r#"
        fn icon(ui: &mut Ui, label: &str, tooltip: &str, on: bool) {}
        fn show() {
            icon(ui, "名前", "クリックして切り替え", true);
            status("クリックして点を追加");
            let tip = lang.pick("押すと戻します", "Click to restore");
            Item { tooltip: lang.pick("ドラッグして動かす", "Drag to move"), label: "名前" };
            x.on_hover_text("押すとやめます");
        }
        fn mode_tooltip(lang: Lang) -> &'static str { "ドラッグで回す" }
        fn keys() {
            ui.label(lang.t("sample-name"));
            ui.label(lang.f("sample-loaded", &[("name", &name)]));
            icon(ui, lang.t("sample-name"), lang.t("sample-swap-tip"), true);
            t("_Invisible", 0.0, "非表示", "Invisible");
        }
        "#
        .to_owned(),
    )];
    let table = tooltip_parameters(&sources);
    assert_eq!(table.get("icon"), Some(&BTreeSet::from([2])));
    let keys = BTreeMap::from([
        (
            "sample-name".to_owned(),
            vec!["キーの名前".to_owned(), "Key name".to_owned()],
        ),
        (
            "sample-loaded".to_owned(),
            vec!["{ $name } を読み込みました".to_owned()],
        ),
        (
            "sample-swap-tip".to_owned(),
            vec!["押すと入れ替えます".to_owned()],
        ),
        ("_Invisible".to_owned(), vec!["キーではない".to_owned()]),
    ]);
    let mut out = Vec::new();
    lex("sample.rs", &sources[0].1, &table, &keys, &mut out);
    let tooltip_of = |text: &str| {
        out.iter()
            .find(|l| l.text == text)
            .unwrap_or_else(|| panic!("{text}"))
            .tooltip
    };
    assert!(!tooltip_of("名前"));
    assert!(tooltip_of("クリックして切り替え"));
    assert!(!tooltip_of("クリックして点を追加"));
    assert!(tooltip_of("押すと戻します") && tooltip_of("Click to restore"));
    assert!(tooltip_of("ドラッグして動かす"));
    assert!(tooltip_of("押すとやめます"));
    assert!(tooltip_of("ドラッグで回す"));
    // .ftl のキー: キーの文を、引いた所の文として見る（ツールチップかはキーの `-tip`）。キーの文字そのものは見ない
    assert!(!tooltip_of("キーの名前") && !tooltip_of("Key name"));
    assert!(!tooltip_of("{} を読み込みました"));
    assert!(tooltip_of("押すと入れ替えます"));
    assert_eq!(out.iter().filter(|l| l.text == "キーの名前").count(), 2);
    assert!(out.iter().all(|l| !l.text.starts_with("sample-")));
    assert_eq!(
        out.iter()
            .find(|l| l.text == "Key name")
            .unwrap()
            .function
            .as_deref(),
        Some("keys")
    );
    // メソッドでない `t(…)`（効果の表の関数など）はキーとして読まない
    assert!(out.iter().any(|l| l.text == "_Invisible"));
    assert!(out.iter().all(|l| l.text != "キーではない"));
}

/// 試験を除くのは `#[cfg(test)]` を付けた項目だけで、その後ろの本番のコードは見張りの対象に残る（`view3d/pose.rs` は、試験用の関数 1 つの
/// 後ろに、読み込みの知らせを作る本番のコードが続く）。
#[test]
fn only_the_test_items_are_left_out_of_the_scan() {
    let sample = "\
fn before() { let a = \"本番の前\"; }

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn wait() -> u32 {
    let b = \"試験の関数\";
    1
}

fn middle() { let c = \"試験の関数の後ろ\"; }

#[cfg(test)]
use std::time::Duration;

fn after_use() { let d = \"use の後ろ\"; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t() { let e = \"試験のモジュール\"; }
}
";
    let stripped = strip_test_items(sample);
    assert_eq!(
        stripped.lines().count(),
        sample.lines().count(),
        "行番号が合う"
    );
    for kept in ["本番の前", "試験の関数の後ろ", "use の後ろ"] {
        assert!(stripped.contains(kept), "{kept} が残る");
    }
    for dropped in ["試験の関数\"", "試験のモジュール", "Duration"] {
        assert!(!stripped.contains(dropped), "{dropped} が消える");
    }

    // 実物: pose.rs の試験用の関数の後ろの本番のコード（読み込みの知らせ）が、走査に入っている
    let literals = collect_literals();
    let pose: Vec<&Literal> = literals
        .iter()
        .filter(|l| l.file == "view3d/pose.rs")
        .collect();
    assert!(
        pose.iter()
            .any(|l| l.text.contains("を読み込みました") && !l.tooltip),
        "pose.rs の本番の知らせが走査に入っていない（{} 件）",
        pose.len()
    );
    assert!(
        pose.iter().any(|l| l.text.starts_with("Loaded ")),
        "pose.rs の英語の知らせが走査に入っていない"
    );
    // 試験のモジュールの中の文字は入っていない
    assert!(
        production_sources()
            .iter()
            .all(|(_, text)| !text.contains("mod tests {")),
        "試験のモジュールが残っている"
    );
}

/// 見直し用: 画面の文字（ツールチップを除く）の全部を出す（`cargo test -p yolu-app --test headless no_instruction_text:: -- --ignored --nocapture`）。
#[test]
#[ignore = "見直し用の一覧"]
fn list_the_screen_texts() {
    for l in collect_literals()
        .into_iter()
        .filter(|l| !l.tooltip && is_screen_text(&l.text))
    {
        println!("{}:{}: {}", l.file, l.line, l.text.replace('\n', "\\n"));
    }
}
