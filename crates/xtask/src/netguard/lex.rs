//! Rust の字句の読み手と、`#[cfg(test)]` の項目を除く読み。

// ───────── 字句 ─────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Tok {
    Ident(String),
    /// 文字列の中身（エスケープは読まずそのまま）。
    Str(String),
    /// `::`
    Sep,
    /// 数のリテラル（中身は読まない）。
    Num,
    /// ライフタイムとラベル（`'a`・`'outer:` の `'outer`）。文字リテラルは字句に出さない。
    Lifetime(String),
    Punct(char),
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub(super) tok: Tok,
    pub(super) line: usize,
}

pub(super) fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub(super) fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `start`（開きの `"` の次）から閉じの `"` まで。戻りは（中身・閉じの次の位置・読み飛ばした改行の数）。
pub(super) fn quoted(c: &[char], start: usize) -> (String, usize, usize) {
    let (mut j, mut lines, mut content) = (start, 0, String::new());
    while j < c.len() {
        match c[j] {
            '\\' => {
                content.push('\\');
                if let Some(&next) = c.get(j + 1) {
                    if next == '\n' {
                        lines += 1;
                    }
                    content.push(next);
                }
                j += 2;
            }
            '"' => return (content, j + 1, lines),
            ch => {
                if ch == '\n' {
                    lines += 1;
                }
                content.push(ch);
                j += 1;
            }
        }
    }
    (content, c.len(), lines)
}

/// `hashes` 個の `#` のついた `r"…"` の中身。`start` は開きの `"` の次。
pub(super) fn raw_quoted(c: &[char], start: usize, hashes: usize) -> (String, usize, usize) {
    let (mut j, mut lines) = (start, 0);
    while j < c.len() {
        if c[j] == '"' && (1..=hashes).all(|h| c.get(j + h) == Some(&'#')) {
            return (c[start..j].iter().collect(), j + 1 + hashes, lines);
        }
        if c[j] == '\n' {
            lines += 1;
        }
        j += 1;
    }
    (c[start..].iter().collect(), c.len(), lines)
}

/// `'` から始まるのが文字リテラル（`'x'`・`'\n'`）か（そうでなければライフタイムかラベル）。
pub(super) fn is_char_literal(c: &[char], i: usize) -> bool {
    c.get(i + 1) == Some(&'\\') || c.get(i + 2) == Some(&'\'')
}

/// `'` から始まる文字リテラルかライフタイム（ラベル）を読み飛ばした次の位置。
pub(super) fn skip_char_or_lifetime(c: &[char], i: usize) -> usize {
    if c.get(i + 1) == Some(&'\\') {
        // 逃がした文字（`'\n'`・`'\''`・`'\u{41}'`）。逃がした 1 文字のあと、閉じの `'` まで
        let mut j = i + 3;
        while j < c.len() && c[j] != '\'' && c[j] != '\n' {
            j += 1;
        }
        (j + 1).min(c.len())
    } else if c.get(i + 2) == Some(&'\'') {
        i + 3
    } else {
        let mut j = i + 1;
        while j < c.len() && is_ident_continue(c[j]) {
            j += 1;
        }
        j
    }
}

pub(super) fn lex(text: &str) -> Vec<Token> {
    let c: Vec<char> = text.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && at(i + 1) == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && at(i + 1) == Some('*') {
            let mut depth = 1;
            i += 2;
            while i < c.len() && depth > 0 {
                if c[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if c[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if ch == '"' {
            let (text, next, lines) = quoted(&c, i + 1);
            out.push(Token {
                tok: Tok::Str(text),
                line,
            });
            line += lines;
            i = next;
        } else if ch == '\'' {
            let next = skip_char_or_lifetime(&c, i);
            if !is_char_literal(&c, i) {
                out.push(Token {
                    tok: Tok::Lifetime(c[i + 1..next].iter().collect()),
                    line,
                });
            }
            i = next;
        } else if ch == ':' && at(i + 1) == Some(':') {
            out.push(Token {
                tok: Tok::Sep,
                line,
            });
            i += 2;
        } else if is_ident_start(ch) {
            let start = i;
            while i < c.len() && is_ident_continue(c[i]) {
                i += 1;
            }
            let word: String = c[start..i].iter().collect();
            match (word.as_str(), at(i)) {
                // b"…"・c"…"
                ("b" | "c", Some('"')) => {
                    let (text, next, lines) = quoted(&c, i + 1);
                    out.push(Token {
                        tok: Tok::Str(text),
                        line,
                    });
                    line += lines;
                    i = next;
                    continue;
                }
                // b'x'
                ("b", Some('\'')) => {
                    i = skip_char_or_lifetime(&c, i);
                    continue;
                }
                // r"…"・r#"…"#・br#"…"#・cr"…" と、生の識別子 r#name
                ("r" | "br" | "cr", Some('"' | '#')) => {
                    let mut j = i;
                    while at(j) == Some('#') {
                        j += 1;
                    }
                    let hashes = j - i;
                    if at(j) == Some('"') {
                        let (text, next, lines) = raw_quoted(&c, j + 1, hashes);
                        out.push(Token {
                            tok: Tok::Str(text),
                            line,
                        });
                        line += lines;
                        i = next;
                        continue;
                    }
                    if word == "r" && hashes == 1 && at(j).is_some_and(is_ident_start) {
                        let from = j;
                        while j < c.len() && is_ident_continue(c[j]) {
                            j += 1;
                        }
                        out.push(Token {
                            tok: Tok::Ident(c[from..j].iter().collect()),
                            line,
                        });
                        i = j;
                        continue;
                    }
                }
                _ => {}
            }
            out.push(Token {
                tok: Tok::Ident(word),
                line,
            });
        } else if ch.is_ascii_digit() {
            while i < c.len() && is_ident_continue(c[i]) {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Num,
                line,
            });
        } else {
            out.push(Token {
                tok: Tok::Punct(ch),
                line,
            });
            i += 1;
        }
    }
    out
}

// ───────── 試験の項目を除く ─────────

pub(super) fn is_punct(t: &[Token], i: usize, p: char) -> bool {
    matches!(t.get(i), Some(Token { tok: Tok::Punct(c), .. }) if *c == p)
}

pub(super) fn ident_at(t: &[Token], i: usize) -> Option<&str> {
    match t.get(i) {
        Some(Token {
            tok: Tok::Ident(s), ..
        }) => Some(s),
        _ => None,
    }
}

/// `open` の位置の括弧に対応する閉じの位置（無ければ末尾）。
pub(super) fn matching(t: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (k, token) in t.iter().enumerate().skip(open) {
        match token.tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    t.len()
}

/// `#[cfg(test)]`・`#[cfg(all(test, …))]`・`#![cfg(test)]` の位置なら（属性の次の位置・内側の属性か）。
pub(super) fn cfg_test_attribute(t: &[Token], i: usize) -> Option<(usize, bool)> {
    if !is_punct(t, i, '#') {
        return None;
    }
    let mut j = i + 1;
    let inner = is_punct(t, j, '!');
    if inner {
        j += 1;
    }
    if !is_punct(t, j, '[') || ident_at(t, j + 1) != Some("cfg") || !is_punct(t, j + 2, '(') {
        return None;
    }
    let only_test = (ident_at(t, j + 3) == Some("test") && is_punct(t, j + 4, ')'))
        || (ident_at(t, j + 3) == Some("all")
            && is_punct(t, j + 4, '(')
            && ident_at(t, j + 5) == Some("test"));
    only_test.then(|| (matching(t, j) + 1, inner))
}

/// `mod 名前;` の宣言（`#[path = "…"]` があればその道）。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ModDecl {
    pub(super) name: String,
    pub(super) path: Option<String>,
}

/// `closer` の位置の閉じ括弧に対応する開きの位置（無ければ 0）。
fn matching_open(t: &[Token], closer: usize) -> usize {
    let mut depth = 0usize;
    for k in (0..=closer).rev() {
        match t[k].tok {
            Tok::Punct(')' | ']' | '}') => depth += 1,
            Tok::Punct('(' | '[' | '{') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    0
}

/// `i` の位置を囲む `{` の位置。
fn enclosing_open(t: &[Token], i: usize) -> Option<usize> {
    let mut bal = 0usize;
    for k in (0..i).rev() {
        match t[k].tok {
            Tok::Punct('}') => bal += 1,
            Tok::Punct('{') if bal == 0 => return Some(k),
            Tok::Punct('{') => bal -= 1,
            _ => {}
        }
    }
    None
}

/// `#[cfg(test)]` の次の項目（後ろの属性も含む）の終わりの次の位置と、`mod 名前;` ならその宣言。
pub(super) fn skip_item(t: &[Token], from: usize) -> (usize, Option<ModDecl>) {
    let mut j = from;
    let mut path = None;
    while is_punct(t, j, '#') && is_punct(t, j + 1, '[') {
        // `#[path = "…"]`
        if ident_at(t, j + 2) == Some("path") && is_punct(t, j + 3, '=') {
            if let Some(Token {
                tok: Tok::Str(text),
                ..
            }) = t.get(j + 4)
            {
                path = Some(text.clone());
            }
        }
        j = matching(t, j + 1) + 1;
    }
    // 修飾子とラベル（`'outer: loop`）を飛ばして、項目の種類の語へ
    let mut k = j;
    loop {
        match ident_at(t, k) {
            Some("pub") => {
                k += 1;
                if is_punct(t, k, '(') {
                    k = matching(t, k) + 1;
                }
            }
            Some("unsafe" | "async" | "default") => k += 1,
            Some("extern") => {
                k += 1;
                if matches!(
                    t.get(k),
                    Some(Token {
                        tok: Tok::Str(_),
                        ..
                    })
                ) {
                    k += 1;
                }
            }
            Some("const")
                if matches!(
                    ident_at(t, k + 1),
                    Some("fn" | "unsafe" | "async" | "extern")
                ) =>
            {
                k += 1
            }
            _ if matches!(
                t.get(k),
                Some(Token {
                    tok: Tok::Lifetime(_),
                    ..
                })
            ) && is_punct(t, k + 1, ':') =>
            {
                k += 2
            }
            _ => break,
        }
    }
    let keyword = ident_at(t, k);
    // `名前! { … }`・`名前!(…);` のマクロの呼び出し
    let opens_group = matches!(
        t.get(k + 2),
        Some(Token {
            tok: Tok::Punct('(' | '[' | '{'),
            ..
        })
    );
    if keyword.is_some_and(|word| {
        !matches!(
            word,
            "macro_rules" | "if" | "while" | "match" | "return" | "break" | "let" | "in"
        )
    }) && is_punct(t, k + 1, '!')
        && opens_group
    {
        let mut end = matching(t, k + 2) + 1;
        if is_punct(t, end, ';') {
            end += 1;
        }
        return (end.min(t.len()), None);
    }
    // `;` までが項目（本体の波括弧は式の一部）
    let to_semicolon = matches!(keyword, Some("use" | "type" | "let" | "const" | "static"));
    // 最初の波括弧の本体で終わる項目（式の `if`・`match`・ループ・`{ … }` も）
    let to_brace = matches!(
        keyword,
        Some(
            "mod"
                | "fn"
                | "impl"
                | "trait"
                | "struct"
                | "enum"
                | "union"
                | "macro_rules"
                | "extern"
                | "if"
                | "match"
                | "for"
                | "while"
                | "loop"
        )
    ) || is_punct(t, k, '{');
    let module = (keyword == Some("mod") && is_punct(t, k + 2, ';'))
        .then(|| {
            ident_at(t, k + 1).map(|name| ModDecl {
                name: name.to_owned(),
                path: path.clone(),
            })
        })
        .flatten();
    let mut depth = 0usize;
    for (m, token) in t.iter().enumerate().skip(j) {
        match token.tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']') if depth == 0 => return (m, None),
            Tok::Punct('}') if depth == 0 => return (m, None),
            Tok::Punct(')' | ']') => depth -= 1,
            Tok::Punct('}') => {
                depth -= 1;
                if depth == 0 && to_brace && !to_semicolon && ident_at(t, m + 1) != Some("else") {
                    return (m + 1, module);
                }
            }
            Tok::Punct(';') if depth == 0 => return (m + 1, module),
            // match の腕 `パターン => { … }`（腕の本体が波括弧なら、コンマが無くてもそこで終わる）
            Tok::Punct('>')
                if depth == 0
                    && !to_semicolon
                    && !to_brace
                    && m > 0
                    && is_punct(t, m - 1, '=')
                    && is_punct(t, m + 1, '{') =>
            {
                let mut end = matching(t, m + 1) + 1;
                if is_punct(t, end, ',') {
                    end += 1;
                }
                return (end.min(t.len()), None);
            }
            // 構造体の欄・match の腕・enum の変種に付いた属性
            Tok::Punct(',') if depth == 0 && !to_semicolon && !to_brace => return (m + 1, None),
            _ => {}
        }
    }
    (t.len(), None)
}

/// 試験だけの項目を除いた字句と、`#[cfg(test)] mod 名前;` の宣言。
/// ファイルの先頭の `#![cfg(test)]` はファイルを丸ごと、モジュールの中の `#![cfg(test)]` はそのモジュールだけ（見出しから閉じまで）除く。
pub(super) fn strip_test_items(tokens: Vec<Token>) -> (Vec<Token>, Vec<ModDecl>) {
    let mut removed = vec![false; tokens.len()];
    let mut modules = Vec::new();
    let (mut i, mut depth) = (0, 0usize);
    while i < tokens.len() {
        if let Some((after, inner)) = cfg_test_attribute(&tokens, i) {
            if inner {
                if depth == 0 {
                    return (Vec::new(), Vec::new());
                }
                // 入れ子のモジュールの中: 囲んでいる `{ … }` を、見出し（`#[…] pub mod 名前`）から除く
                let open = enclosing_open(&tokens, i);
                if let Some(open) = open {
                    let mut start = open;
                    while start > 0 {
                        match tokens[start - 1].tok {
                            Tok::Punct(';' | '{' | '}') => break,
                            Tok::Punct(')' | ']') => start = matching_open(&tokens, start - 1),
                            _ => start -= 1,
                        }
                    }
                    let close = matching(&tokens, open).min(tokens.len() - 1);
                    for flag in &mut removed[start..=close] {
                        *flag = true;
                    }
                }
                i = after;
                continue;
            }
            let (end, module) = skip_item(&tokens, after);
            modules.extend(module);
            let end = end.max(after).min(tokens.len());
            for flag in &mut removed[i..end] {
                *flag = true;
            }
            i = end;
            continue;
        }
        match tokens[i].tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    let kept = tokens
        .into_iter()
        .zip(removed)
        .filter_map(|(token, gone)| (!gone).then_some(token))
        .collect();
    (kept, modules)
}
