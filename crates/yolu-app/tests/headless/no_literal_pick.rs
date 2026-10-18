//! 画面の文を `lang.pick("日本語", "English")` のように日英の対としてコードに書かない（画面の文は `locales/<言語>/*.ftl` に置き、
//! `lang.t("キー")`・`lang.f("キー", …)` で引く）。本番のソース（テストを除く）の `.pick(` で、引数が 2 つあり、どちらかが
//! 文字列の直書き（`"…"`・`"…".into()`・`"…".to_owned()`・`String::from("…")`・`format!(…)`、前に `&` があってもよい）の呼び出しを落とす。
//! 変数・表の欄・関数の結果を渡す `pick`（数・配列・定数の型・保存したデータの対）は見ない。
//!
//! `ALLOWED` は、まだ移していない所の許しの一覧（ファイルと、空白を詰めた呼び出しの文字。行の番号は使わない。移し終えた後は表を
//! 空にする）。表の行がソースに無くなったら落とす（移し終えた所の許しを残さない）。表が空でないときは件数を出す。

use std::collections::BTreeSet;

/// まだ画面の文を直書きの対で持っている呼び出し（ファイル, 空白を詰めた呼び出し）。
const ALLOWED: &[(&str, &str)] = &[
    // 日本語が core の `FillError` の文そのもので、英語だけを直書きしている（core の文を .ftl に写し、食い違いを見張る仕組みがまだ無い）
    (
        "fillfx/mod.rs",
        r##"pick(other.to_string(),"Theprojectioncannotbeused".into())"##,
    ),
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str,
    Punct(char),
}

/// 字句に分ける（コメントは飛ばす。文字列・文字は 1 つの字句。生の文字列にも対応）。
fn lex(text: &str) -> Vec<(Tok, usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if (c == 'r' || (c == 'b' && at(i + 1) == 'r'))
            && matches!(at(i + 1 + usize::from(c == 'b')), '#' | '"')
        {
            let start = i;
            i += 1 + usize::from(c == 'b');
            let mut hashes = 0;
            while at(i) == '#' {
                hashes += 1;
                i += 1;
            }
            if at(i) != '"' {
                out.push((Tok::Ident("r".into()), start, i));
                continue;
            }
            i += 1;
            loop {
                if i >= chars.len() {
                    break;
                }
                if chars[i] == '"' && (0..hashes).all(|k| at(i + 1 + k) == '#') {
                    i += 1 + hashes;
                    break;
                }
                i += 1;
            }
            out.push((Tok::Str, start, i));
        } else if c == '"' || (c == 'b' && at(i + 1) == '"') {
            let start = i;
            i += 1 + usize::from(c == 'b');
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push((Tok::Str, start, i.min(chars.len())));
        } else if c == '\'' {
            // 文字（'a'・'\n'）か、ライフタイム（'a）
            let start = i;
            if at(i + 1) == '\\' {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
                out.push((Tok::Str, start, i.min(chars.len())));
            } else if at(i + 2) == '\'' {
                i += 3;
                out.push((Tok::Str, start, i));
            } else {
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push((Tok::Ident(chars[start..i].iter().collect()), start, i));
        } else {
            out.push((Tok::Punct(c), i, i + 1));
            i += 1;
        }
    }
    out
}

/// 対の閉じ括弧の番号（`toks[open]` は開き括弧）。
fn matching(toks: &[(Tok, usize, usize)], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (j, (tok, _, _)) in toks.iter().enumerate().skip(open) {
        match tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

/// 引数を最上位のカンマで分けた字句の範囲。
fn split_args(toks: &[(Tok, usize, usize)], from: usize, to: usize) -> Vec<(usize, usize)> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, from);
    for (j, (tok, _, _)) in toks.iter().enumerate().take(to).skip(from) {
        match tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => depth -= 1,
            Tok::Punct(',') if depth == 0 => {
                out.push((start, j));
                start = j + 1;
            }
            _ => {}
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

/// 引数が文字列の直書き（`"…"`・`format!(…)`・`String::from("…")`。前の `&` と、後ろの `.into()`・`.to_owned()`・`.to_string()` を許す）か。
fn is_direct_text(toks: &[(Tok, usize, usize)], from: usize, to: usize) -> bool {
    let mut i = from;
    while matches!(toks.get(i), Some((Tok::Punct('&'), _, _))) && i < to {
        i += 1;
    }
    let ident =
        |k: usize, name: &str| matches!(toks.get(k), Some((Tok::Ident(n), _, _)) if n == name);
    let punct = |k: usize, c: char| matches!(toks.get(k), Some((Tok::Punct(p), _, _)) if *p == c);
    let end = if i < to && matches!(toks[i].0, Tok::Str) {
        i + 1
    } else if ident(i, "format") && punct(i + 1, '!') && punct(i + 2, '(') {
        match matching(toks, i + 2) {
            Some(close) => close + 1,
            None => return false,
        }
    } else if ident(i, "String")
        && punct(i + 1, ':')
        && punct(i + 2, ':')
        && ident(i + 3, "from")
        && punct(i + 4, '(')
    {
        match matching(toks, i + 4) {
            Some(close) => close + 1,
            None => return false,
        }
    } else {
        return false;
    };
    let mut k = end;
    while k < to {
        let converts = punct(k, '.')
            && (ident(k + 1, "into") || ident(k + 1, "to_owned") || ident(k + 1, "to_string"))
            && punct(k + 2, '(')
            && punct(k + 3, ')');
        if !converts {
            return false;
        }
        k += 4;
    }
    true
}

/// 見つけた呼び出し: (呼び出しの空白を詰めた文字)。
fn found_in(text: &str) -> Vec<String> {
    let toks = lex(text);
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for k in 1..toks.len().saturating_sub(2) {
        let is_pick = matches!(&toks[k].0, Tok::Ident(n) if n == "pick")
            && toks[k - 1].0 == Tok::Punct('.')
            && toks[k + 1].0 == Tok::Punct('(');
        if !is_pick {
            continue;
        }
        let Some(close) = matching(&toks, k + 1) else {
            continue;
        };
        let args = split_args(&toks, k + 2, close);
        if args.len() == 2 && args.iter().any(|(a, b)| is_direct_text(&toks, *a, *b)) {
            let call: String = chars[toks[k].1..toks[close].2]
                .iter()
                .filter(|c| !c.is_whitespace())
                .collect();
            out.push(call);
        }
    }
    out
}

#[test]
fn screen_texts_are_not_written_as_pairs_in_the_source() {
    let sources = crate::no_instruction_text::production_sources();
    assert!(sources.len() > 100, "読むソースが少ない: {}", sources.len());
    let allowed: BTreeSet<(&str, &str)> = ALLOWED.iter().copied().collect();
    let mut found = Vec::new();
    let mut still_in_source = BTreeSet::new();
    for (file, text) in &sources {
        for call in found_in(text) {
            if allowed.contains(&(file.as_str(), call.as_str())) {
                still_in_source.insert((file.clone(), call));
            } else {
                found.push(format!("{file}: {call}"));
            }
        }
    }
    if !ALLOWED.is_empty() {
        eprintln!(
            "pick の直書きの許し: 表 {} 件、そのうちソースに残っている物 {} 件（移し終えたら表を空にする）",
            ALLOWED.len(),
            still_in_source.len()
        );
    }
    let stale: Vec<String> = ALLOWED
        .iter()
        .filter(|(f, c)| !still_in_source.contains(&(f.to_string(), c.to_string())))
        .map(|(f, c)| format!("{f}: {c}"))
        .collect();
    assert!(
        stale.is_empty(),
        "許しの表に、ソースに無くなった行がある（移し終えたので表から消す）:\n{}",
        stale.join("\n")
    );
    assert!(
        found.is_empty(),
        "画面の文は .ftl に置き、`lang.t(\"キー\")`・`lang.f(\"キー\", …)` で引く（日英の対を `pick` に直書きしない）:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_reader_finds_literal_pairs_and_passes_the_rest() {
    let found = |src: &str| found_in(src).len();
    assert_eq!(found(r#"let a = lang.pick("あ", "a");"#), 1);
    assert_eq!(
        found("lang.pick(\n    format!(\"{n} 件\"),\n    format!(\"{n} items\"),\n)"),
        1
    );
    assert_eq!(found(r#"lang.pick(text.clone(), "Fallback".into())"#), 1);
    assert_eq!(found(r#"lang.pick("あ".to_owned(), x)"#), 1);
    assert_eq!(found(r#"lang.pick(&String::from("あ"), &y)"#), 1);
    assert_eq!(found(r##"lang.pick(r#"あ"#, other)"##), 1);
    // 変数・表の欄・関数の結果・引数が 1 つ・コメント・文字列の中は見ない
    assert_eq!(found("lang.pick(ja, en)"), 0);
    assert_eq!(found("lang.pick(self.ja, self.en)"), 0);
    assert_eq!(found("lang.pick(o.0, o.1)"), 0);
    assert_eq!(
        found("lang.pick(budget_text(t).unwrap_or(\"x\").into(), y)"),
        0
    );
    assert_eq!(found("name.pick(lang)"), 0);
    assert_eq!(found("// lang.pick(\"あ\", \"a\")"), 0);
    assert_eq!(found(r#"let s = "lang.pick(\"あ\", \"a\")";"#), 0);
    assert_eq!(found("let pick = |a, b| a; pick(\"x\", \"y\")"), 0);
}
