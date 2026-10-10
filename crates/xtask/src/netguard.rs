//! 外へ出る通信の見張り（段 1: コードと依存）。`cargo xtask netguard` で表を出し、同じ確かめを試験（`cargo test -p xtask`）が毎回回す。
//!
//! アプリが外へ通信するのは、利用者が選んだ更新の確認だけ（README・INSTALL の約束）。待ち受けは、設定で入れている間の 127.0.0.1 だけ。
//! この約束が、新しいコードや依存で静かに破れないよう、2 つを読む。
//!
//! - ソース: `crates/*/src/**/*.rs` と `crates/*/build.rs`（この xtask 自身は配らないので対象外）から、通信と外への出口の印
//!   （[`MARKERS`]。ソケットの型・HTTP の部品・プロセスの起動・「開く」の呼び出し・`http(s)://` の文字列など）を探し、[`ALLOWED`]（ファイル・印・理由・数）に無いものを落とす。
//!   Rust の字句（コメント・文字列・文字・ライフタイム）を読み分けるので、コメントや文字列の中の識別子には反応せず、行番号が合う。
//!   `#[cfg(test)]` を付けた項目と、`#[cfg(test)] mod x;` のファイルは、配る物に入らないので除く。
//!   ソケットの `bind`・`connect` は、宛先に 127.0.0.1（`LOCALHOST`・`LOOPBACK`・`localhost`・`::1`）が読めなければ、一覧とは別に落とす。
//! - 依存: `Cargo.lock` に HTTP の客・TLS・DNS・テレメトリなどの名前が無いこと、`cargo metadata` で、待ち受けの部品（hyper・hyper-util・socket2・mio、
//!   tokio の `net`）を直接の依存に持つのが yolu-mcp だけであること、配るアプリ（yolu-app・yolu-cli）が「ブラウザーを開く」クレートに依存しないこと、
//!   低水準の通信の部品に依存するクレートが決めた範囲から増えていないこと、Windows の API の機能に WinHTTP 以外の通信が入っていないことを確かめる。
//!
//! 一覧に足すときは、README・INSTALL の約束（更新の確認のほかに通信しない・外からの操作は 127.0.0.1 だけ）を破らないか確かめ、足す行に理由を書く
//! （`docs/DEVELOPMENT.md` の「通信の見張り」）。
use super::{cargo, root, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

// ───────── 字句 ─────────

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    /// 文字列の中身（エスケープは読まずそのまま）。
    Str(String),
    /// `::`
    Sep,
    Punct(char),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    line: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `start`（開きの `"` の次）から閉じの `"` まで。戻りは（中身・閉じの次の位置・読み飛ばした改行の数）。
fn quoted(c: &[char], start: usize) -> (String, usize, usize) {
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
fn raw_quoted(c: &[char], start: usize, hashes: usize) -> (String, usize, usize) {
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

/// `'` から始まる文字リテラルかライフタイム（ラベル）を読み飛ばした次の位置。
fn skip_char_or_lifetime(c: &[char], i: usize) -> usize {
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

fn lex(text: &str) -> Vec<Token> {
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
            i = skip_char_or_lifetime(&c, i);
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

fn is_punct(t: &[Token], i: usize, p: char) -> bool {
    matches!(t.get(i), Some(Token { tok: Tok::Punct(c), .. }) if *c == p)
}

fn ident_at(t: &[Token], i: usize) -> Option<&str> {
    match t.get(i) {
        Some(Token {
            tok: Tok::Ident(s), ..
        }) => Some(s),
        _ => None,
    }
}

/// `open` の位置の括弧に対応する閉じの位置（無ければ末尾）。
fn matching(t: &[Token], open: usize) -> usize {
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
fn cfg_test_attribute(t: &[Token], i: usize) -> Option<(usize, bool)> {
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

/// `#[cfg(test)]` の次の項目（後ろの属性も含む）の終わりの次の位置と、`mod 名前;` ならその名前。
fn skip_item(t: &[Token], from: usize) -> (usize, Option<String>) {
    let mut j = from;
    while is_punct(t, j, '#') && is_punct(t, j + 1, '[') {
        j = matching(t, j + 1) + 1;
    }
    // 修飾子を飛ばして、項目の種類の語へ
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
        .then(|| ident_at(t, k + 1).map(str::to_owned))
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
            // 構造体の欄・match の腕・enum の変種に付いた属性
            Tok::Punct(',') if depth == 0 && !to_semicolon && !to_brace => return (m + 1, None),
            _ => {}
        }
    }
    (t.len(), None)
}

/// 試験だけの項目を除いた字句と、`#[cfg(test)] mod 名前;` の名前。`#![cfg(test)]` のファイルは丸ごと除く。
fn strip_test_items(tokens: Vec<Token>) -> (Vec<Token>, Vec<String>) {
    let (mut out, mut modules, mut i) = (Vec::with_capacity(tokens.len()), Vec::new(), 0);
    while i < tokens.len() {
        if let Some((after, inner)) = cfg_test_attribute(&tokens, i) {
            if inner {
                return (Vec::new(), Vec::new());
            }
            let (end, module) = skip_item(&tokens, after);
            modules.extend(module);
            i = end.max(after);
            continue;
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    (out, modules)
}

// ───────── 印 ─────────

enum Pat {
    /// その識別子そのもの。
    Ident(&'static str),
    /// その語で始まる識別子（`WinHttpOpen` など）。
    Prefix(&'static str),
    /// 識別子の並び `a::b`。
    Path(&'static [&'static str]),
    /// 後ろに `::` が続く識別子（クレートの名前）。
    Crate(&'static str),
    /// 文字列がその文を含む。
    StrHas(&'static str),
    /// 文字列がその文そのもの。
    StrIs(&'static str),
}
use Pat::{Crate, Ident, Path as PathPat, Prefix, StrHas, StrIs};

/// 通信と外への出口の印（名前は [`ALLOWED`] が使う）。足りない印は足してよい（足すと、その印を持つファイルが一覧に載るまで落ちる）。
const MARKERS: &[(&str, Pat)] = &[
    // ソケット・名前の引き
    ("TcpStream", Ident("TcpStream")),
    ("TcpListener", Ident("TcpListener")),
    ("TcpSocket", Ident("TcpSocket")),
    ("UdpSocket", Ident("UdpSocket")),
    ("ToSocketAddrs", Ident("ToSocketAddrs")),
    ("to_socket_addrs", Ident("to_socket_addrs")),
    ("std::net", PathPat(&["std", "net"])),
    ("tokio::net", PathPat(&["tokio", "net"])),
    ("socket2", Crate("socket2")),
    ("mio", Crate("mio")),
    ("libc::socket", PathPat(&["libc", "socket"])),
    ("libc::connect", PathPat(&["libc", "connect"])),
    ("libc::bind", PathPat(&["libc", "bind"])),
    ("libc::sendto", PathPat(&["libc", "sendto"])),
    ("libc::getaddrinfo", PathPat(&["libc", "getaddrinfo"])),
    ("getaddrinfo", Ident("getaddrinfo")),
    ("gethostbyname", Ident("gethostbyname")),
    ("AF_INET", Prefix("AF_INET")),
    ("SOCK_STREAM", Ident("SOCK_STREAM")),
    ("SOCK_DGRAM", Ident("SOCK_DGRAM")),
    ("WSA", Prefix("WSA")),
    ("WinSock", Prefix("WinSock")),
    // HTTP・通信の部品
    ("hyper", Crate("hyper")),
    ("hyper_util", Crate("hyper_util")),
    ("WinHttp", Prefix("WinHttp")),
    ("reqwest", Crate("reqwest")),
    ("ureq", Crate("ureq")),
    ("isahc", Crate("isahc")),
    ("surf", Crate("surf")),
    ("attohttpc", Crate("attohttpc")),
    ("minreq", Crate("minreq")),
    ("curl", Crate("curl")),
    ("tungstenite", Crate("tungstenite")),
    ("quinn", Crate("quinn")),
    ("h2", Crate("h2")),
    ("rustls", Crate("rustls")),
    ("native_tls", Crate("native_tls")),
    ("openssl", Crate("openssl")),
    ("\"curl\"", StrIs("curl")),
    ("\"http://\"", StrHas("http://")),
    ("\"https://\"", StrHas("https://")),
    // プロセスの起動と「開く」
    ("Command::new", PathPat(&["Command", "new"])),
    ("libc::system", PathPat(&["libc", "system"])),
    ("ShellExecute", Prefix("ShellExecute")),
    ("xdg-open", StrHas("xdg-open")),
    ("open::that", PathPat(&["open", "that"])),
    ("open::with", PathPat(&["open", "with"])),
    ("opener", Crate("opener")),
    ("webbrowser", Crate("webbrowser")),
    ("open_url", Ident("open_url")),
    ("OpenUrl", Ident("OpenUrl")),
    ("hyperlink", Ident("hyperlink")),
    ("hyperlink_to", Ident("hyperlink_to")),
];

fn marker_matches(pat: &Pat, t: &[Token], i: usize) -> bool {
    match (pat, &t[i].tok) {
        (Ident(name), Tok::Ident(s)) => s == name,
        (Prefix(prefix), Tok::Ident(s)) => s.starts_with(prefix),
        (Crate(name), Tok::Ident(s)) => {
            s == name && matches!(t.get(i + 1), Some(Token { tok: Tok::Sep, .. }))
        }
        (PathPat(parts), Tok::Ident(first)) => {
            first == parts[0]
                && parts.iter().enumerate().skip(1).all(|(n, part)| {
                    matches!(t.get(i + 2 * n - 1), Some(Token { tok: Tok::Sep, .. }))
                        && ident_at(t, i + 2 * n) == Some(*part)
                })
        }
        (StrHas(text), Tok::Str(s)) => s.contains(text),
        (StrIs(text), Tok::Str(s)) => s == text,
        _ => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Hit {
    file: String,
    line: usize,
    mark: &'static str,
}

/// `a::bind(…)`・`a::connect(…)` で、`a` が Listener・Stream・Socket を含む型の名前のとき、宛先が 127.0.0.1 と読めるか。
/// 読めなければ（ファイル・行・呼び出し）を返す。呼び出しの引数か、その呼び出しと同じ行に、`LOCALHOST`・`LOOPBACK`・`localhost`・`127.0.0.1`・`::1` があればよい。
fn endpoint_failures(file: &str, t: &[Token]) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for i in 0..t.len() {
        let Some(receiver) = ident_at(t, i) else {
            continue;
        };
        if !matches!(t.get(i + 1), Some(Token { tok: Tok::Sep, .. })) {
            continue;
        }
        let Some(function) = ident_at(t, i + 2) else {
            continue;
        };
        if !matches!(function, "bind" | "connect" | "connect_timeout")
            || !["Listener", "Stream", "Socket"]
                .iter()
                .any(|kind| receiver.contains(kind))
            || !is_punct(t, i + 3, '(')
        {
            continue;
        }
        let end = matching(t, i + 3);
        let line = t[i].line;
        let loopback = t.iter().enumerate().any(|(k, token)| {
            let near = (i + 3..=end).contains(&k) || token.line == line;
            near && match &token.tok {
                Tok::Ident(s) => matches!(s.as_str(), "LOCALHOST" | "LOOPBACK" | "localhost"),
                Tok::Str(s) => {
                    s.contains("127.0.0.1") || s.contains("::1") || s.contains("localhost")
                }
                _ => false,
            }
        });
        if !loopback {
            out.push((file.to_owned(), line, format!("{receiver}::{function}(…)")));
        }
    }
    out
}

/// 1 つのファイルの字句（試験の項目は除いてある）から、印と、宛先の読めない bind・connect を集める。
fn scan_tokens(
    file: &str,
    tokens: &[Token],
    hits: &mut Vec<Hit>,
    endpoints: &mut Vec<(String, usize, String)>,
) {
    for i in 0..tokens.len() {
        for (mark, pat) in MARKERS {
            if marker_matches(pat, tokens, i) {
                hits.push(Hit {
                    file: file.to_owned(),
                    line: tokens[i].line,
                    mark,
                });
            }
        }
    }
    endpoints.extend(endpoint_failures(file, tokens));
}

// ───────── 許す一覧 ─────────

/// 印の数の決め方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Count {
    /// ちょうどこの数。増えたら（同じファイルに 2 つ目の出口が入ったら）落ちる。
    Exactly(usize),
    /// 1 つ以上（同じ種類の識別子が幾つも並ぶもの）。
    AtLeastOne,
}
use Count::{AtLeastOne, Exactly};

struct Allow {
    file: &'static str,
    mark: &'static str,
    count: Count,
    why: &'static str,
}

/// 通信と外への出口の印を持ってよいファイル。印ごとに理由と数を持つ。ここに無い印は落ちる。
const ALLOWED: &[Allow] = &[
    // ビルドの手元（配る物には入らない）
    Allow {
        file: "crates/yolu-app/build.rs",
        mark: "Command::new",
        count: Exactly(1),
        why: "ビルドのときに git でリビジョンを読む（手元のプロセスだけ。アプリには入らない）",
    },
    // 更新の確認（利用者が選んだときだけ。聞くまで通信しない）
    Allow {
        file: "crates/yolu-app/src/update/http.rs",
        mark: "Command::new",
        count: Exactly(1),
        why: "更新の確認: Linux は curl を起動して https の GET だけをする",
    },
    Allow {
        file: "crates/yolu-app/src/update/http.rs",
        mark: "\"curl\"",
        count: Exactly(1),
        why: "更新の確認: 起動する外部コマンドの名前（Command::new の引数）",
    },
    Allow {
        file: "crates/yolu-app/src/update/http.rs",
        mark: "\"https://\"",
        count: Exactly(1),
        why: "更新の確認: 取り先の URL は https だけを通す",
    },
    Allow {
        file: "crates/yolu-app/src/update/http.rs",
        mark: "\"http://\"",
        count: Exactly(1),
        why: "更新の確認: http は試験だけ、同じ機械（127.0.0.1・localhost）に限って通す口（for_loopback_test）",
    },
    Allow {
        file: "crates/yolu-app/src/update/http.rs",
        mark: "WinHttp",
        count: AtLeastOne,
        why: "更新の確認: Windows は OS の WinHTTP で https の GET だけをする",
    },
    Allow {
        file: "crates/yolu-update/src/lib.rs",
        mark: "\"https://\"",
        count: Exactly(4),
        why: "更新の確認: 更新情報と配布物の取り先（GitHub の Releases）の URL と、https だけを通す確かめ（通信そのものは update/http.rs）",
    },
    // 押したときに、OS のブラウザー・ファイル閲覧で開くだけ（アプリは通信しない）
    Allow {
        file: "crates/yolu-app/src/update/launch.rs",
        mark: "Command::new",
        count: Exactly(2),
        why: "確かめ済みのインストーラーの起動（Windows）と、リリースのページを OS のブラウザーで開く呼び出し（Windows 以外）",
    },
    Allow {
        file: "crates/yolu-app/src/update/launch.rs",
        mark: "ShellExecute",
        count: Exactly(2),
        why: "リリースのページを OS のブラウザーで開く（Windows。押したときだけ。開いたあとの通信はブラウザー）",
    },
    Allow {
        file: "crates/yolu-app/src/update/launch.rs",
        mark: "xdg-open",
        count: Exactly(1),
        why: "リリースのページを OS のブラウザーで開く（Windows 以外。押したときだけ）",
    },
    Allow {
        file: "crates/yolu-app/src/update/launch.rs",
        mark: "\"https://\"",
        count: Exactly(1),
        why: "開く URL は https の見える ASCII だけを通す確かめ（checked_url）",
    },
    Allow {
        file: "crates/yolu-app/src/crash/window.rs",
        mark: "\"https://\"",
        count: Exactly(1),
        why: "落ちたあとの「報告」で、GitHub の Issue の新規作成のページを OS のブラウザーで開く URL（押したときだけ。アプリは送らない）",
    },
    Allow {
        file: "crates/yolu-app/src/crash/mod.rs",
        mark: "Command::new",
        count: Exactly(3),
        why: "落ちた記録のフォルダを OS のファイル閲覧で開く（Windows の explorer.exe・macOS の open・それ以外の xdg-open。ローカルの道）",
    },
    Allow {
        file: "crates/yolu-app/src/crash/mod.rs",
        mark: "xdg-open",
        count: Exactly(1),
        why: "落ちた記録のフォルダを OS のファイル閲覧で開く（ローカルの道）",
    },
    Allow {
        file: "crates/yolu-app/src/library/ops.rs",
        mark: "Command::new",
        count: Exactly(1),
        why: "ライブラリのフォルダを OS のファイル閲覧で開く（Windows 以外。ローカルの道）",
    },
    Allow {
        file: "crates/yolu-app/src/library/ops.rs",
        mark: "ShellExecute",
        count: Exactly(2),
        why: "ライブラリのフォルダを OS のファイル閲覧で開く（Windows。ローカルの道）",
    },
    Allow {
        file: "crates/yolu-app/src/library/ops.rs",
        mark: "xdg-open",
        count: Exactly(1),
        why: "ライブラリのフォルダを OS のファイル閲覧で開く（Windows 以外。ローカルの道）",
    },
    Allow {
        file: "crates/yolu-app/src/settings.rs",
        mark: "Command::new",
        count: Exactly(1),
        why: "macOS の実メモリの量を sysctl で読む（手元のプロセス。通信しない）",
    },
    // 127.0.0.1 の受け口と客（設定で入れている間だけ待つ）
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "TcpListener",
        count: Exactly(2),
        why: "外からの操作の受け口: 127.0.0.1 だけで待つ（bind は LOCALHOST を直接渡す。確かめは endpoint_failures）",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "TcpStream",
        count: Exactly(1),
        why: "外からの操作の受け口: つながりが多すぎるときに断る返事の型（受けた接続）",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "std::net",
        count: Exactly(1),
        why: "外からの操作の受け口: アドレスの型（Ipv4Addr・SocketAddr）と待ち受けのソケット",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "tokio::net",
        count: Exactly(2),
        why: "外からの操作の受け口: 待ち受けと受けた接続の tokio の型",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "hyper",
        count: AtLeastOne,
        why: "外からの操作の受け口: HTTP/1.1 のサーバー（Host・Origin を確かめる）",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "hyper_util",
        count: AtLeastOne,
        why: "外からの操作の受け口: hyper を tokio の流れに載せる",
    },
    Allow {
        file: "crates/yolu-mcp/src/http.rs",
        mark: "\"http://\"",
        count: Exactly(3),
        why: "外からの操作の受け口: Origin が自分（http://127.0.0.1・http://localhost）かの確かめ",
    },
    Allow {
        file: "crates/yolu-mcp/src/client.rs",
        mark: "TcpStream",
        count: Exactly(1),
        why: "起動中のアプリへの客（yolupainter-cli）: 127.0.0.1 へ接続する（LOCALHOST を直接渡す）",
    },
    Allow {
        file: "crates/yolu-mcp/src/client.rs",
        mark: "std::net",
        count: Exactly(1),
        why: "起動中のアプリへの客: Ipv4Addr::LOCALHOST を使うための型",
    },
    Allow {
        file: "crates/yolu-mcp/src/client.rs",
        mark: "tokio::net",
        count: Exactly(1),
        why: "起動中のアプリへの客: 127.0.0.1 への接続の tokio の型",
    },
    Allow {
        file: "crates/yolu-mcp/src/client.rs",
        mark: "hyper",
        count: AtLeastOne,
        why: "起動中のアプリへの客: HTTP/1.1 のクライアント（127.0.0.1 の受け口にだけ話す）",
    },
    Allow {
        file: "crates/yolu-mcp/src/client.rs",
        mark: "hyper_util",
        count: AtLeastOne,
        why: "起動中のアプリへの客: hyper を tokio の流れに載せる",
    },
    Allow {
        file: "crates/yolu-mcp/src/lib.rs",
        mark: "\"http://\"",
        count: Exactly(1),
        why: "つなぐ側に渡す受け口の URL の文（http://127.0.0.1:<番号>/mcp）",
    },
    Allow {
        file: "crates/yolu-cli/src/cli.rs",
        mark: "\"http://\"",
        count: Exactly(2),
        why: "ヘルプの文（日英）に書く受け口の URL（http://127.0.0.1:<番号>/mcp）",
    },
];

// ───────── 走査 ─────────

#[derive(Debug, Default)]
struct Scan {
    hits: Vec<Hit>,
    /// 宛先が 127.0.0.1 と読めない bind・connect（ファイル・行・呼び出し）。
    endpoints: Vec<(String, usize, String)>,
    /// 走査したファイルの数（空振りを見抜く）。
    files: usize,
}

/// `crates/*/src/**/*.rs` と `crates/*/build.rs`（xtask を除く）の（`/` 区切りの相対の道・中身）。
fn source_files(root: &Path) -> Result<Vec<(String, String)>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut crates: Vec<_> = fs::read_dir(root.join("crates"))?.collect::<std::io::Result<_>>()?;
    crates.sort_by_key(|e| e.path());
    let mut paths = Vec::new();
    for entry in crates {
        let dir = entry.path();
        if !dir.is_dir() || entry.file_name() == "xtask" {
            continue;
        }
        if dir.join("build.rs").is_file() {
            paths.push(dir.join("build.rs"));
        }
        if dir.join("src").is_dir() {
            walk(&dir.join("src"), &mut paths)?;
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            Ok((relative, fs::read_to_string(&path)?))
        })
        .collect()
}

/// `file` に書いた `mod name;` が指すファイルの道（`name.rs` か `name/mod.rs`。`mod.rs`・`lib.rs`・`main.rs` は同じ階、ほかは `<ファイル名>/` の階）。
fn module_files(file: &str, name: &str) -> [String; 2] {
    let (dir, base) = file.rsplit_once('/').unwrap_or(("", file));
    let stem = base.trim_end_matches(".rs");
    let folder = if matches!(base, "mod.rs" | "lib.rs" | "main.rs") {
        dir.to_owned()
    } else {
        format!("{dir}/{stem}")
    };
    [
        format!("{folder}/{name}.rs"),
        format!("{folder}/{name}/mod.rs"),
    ]
}

/// ファイルの集まり（相対の道・中身）を走査する。試験だけのファイル（`#[cfg(test)] mod x;` の先と、その下のモジュール）は除く。
fn scan_sources(files: &[(String, String)]) -> Scan {
    let lexed: Vec<(&str, Vec<Token>, Vec<String>)> = files
        .iter()
        .map(|(file, text)| {
            let (tokens, modules) = strip_test_items(lex(text));
            (file.as_str(), tokens, modules)
        })
        .collect();
    // 試験だけのモジュールのファイルと、その下のフォルダ
    let mut test_files = BTreeSet::new();
    let mut test_folders = Vec::new();
    for (file, _, modules) in &lexed {
        for name in modules {
            let [plain, nested] = module_files(file, name);
            test_files.insert(plain.clone());
            test_files.insert(nested);
            test_folders.push(format!("{}/", plain.trim_end_matches(".rs")));
        }
    }
    let mut scan = Scan::default();
    for (file, tokens, _) in &lexed {
        if test_files.contains(*file) || test_folders.iter().any(|d| file.starts_with(d.as_str())) {
            continue;
        }
        scan.files += 1;
        scan_tokens(file, tokens, &mut scan.hits, &mut scan.endpoints);
    }
    scan
}

// ───────── 突き合わせ ─────────

#[derive(Debug, Default)]
struct Report {
    violations: Vec<String>,
    /// （ファイル・印・見つけた数・理由）。一覧のうち使われた行。
    table: Vec<(String, String, usize, String)>,
}

fn guidance() -> &'static str {
    "通信を足す前に README・INSTALL の約束（更新の確認のほかに通信しない。外からの操作は設定で入れている間だけ 127.0.0.1）を破らないか確かめ、\
     足してよいときは crates/xtask/src/netguard.rs の ALLOWED に、ファイル・印・数と理由を足します。"
}

fn evaluate(scan: &Scan, allowed: &[Allow]) -> Report {
    let mut report = Report::default();
    let mut found: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    for hit in &scan.hits {
        found
            .entry((&hit.file, hit.mark))
            .or_default()
            .push(hit.line);
    }
    let listed: HashMap<(&str, &str), &Allow> =
        allowed.iter().map(|a| ((a.file, a.mark), a)).collect();
    for ((file, mark), lines) in &found {
        let lines_text = lines
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("・");
        match listed.get(&(*file, *mark)) {
            None => report.violations.push(format!(
                "{file}:{} の {mark} が、許す一覧にありません（{} 行目）。{}",
                lines[0],
                lines_text,
                guidance()
            )),
            Some(allow) => {
                if let Exactly(n) = allow.count {
                    if n != lines.len() {
                        report.violations.push(format!(
                            "{file} の {mark} が一覧では {n} 件なのに {} 件になりました（{} 行目）。\
                             増えた出口が理由に当てはまるか確かめ、当てはまるときだけ一覧の数を直します。{}",
                            lines.len(),
                            lines_text,
                            guidance()
                        ));
                    }
                }
                report.table.push((
                    file.to_string(),
                    mark.to_string(),
                    lines.len(),
                    allow.why.to_string(),
                ));
            }
        }
    }
    for allow in allowed {
        if !found.contains_key(&(allow.file, allow.mark)) {
            report.violations.push(format!(
                "許す一覧の {} の {} が、ソースの中に見つかりません。使われなくなった行は一覧から消します（理由: {}）。",
                allow.file, allow.mark, allow.why
            ));
        }
    }
    for (file, line, call) in &scan.endpoints {
        report.violations.push(format!(
            "{file}:{line} の {call} の宛先に 127.0.0.1（LOCALHOST・LOOPBACK・localhost・::1）が読めません。\
             外へ向かうソケットは作りません。宛先は呼び出しの引数に直接書きます。"
        ));
    }
    report
}

// ───────── 依存 ─────────

/// どこにあっても落とす名前（`Cargo.lock`。開発用の依存も含む）。
const BANNED_ANYWHERE: &[(&str, &[&str])] = &[
    (
        "HTTP の客・TLS",
        &[
            "reqwest",
            "ureq",
            "isahc",
            "surf",
            "curl",
            "curl-sys",
            "attohttpc",
            "minreq",
            "ehttp",
            "hyper-tls",
            "hyper-rustls",
            "hyper-proxy",
            "native-tls",
            "openssl",
            "openssl-sys",
            "rustls",
            "boring",
        ],
    ),
    (
        "WebSocket・QUIC・HTTP/2",
        &[
            "tungstenite",
            "tokio-tungstenite",
            "async-tungstenite",
            "tokio-websockets",
            "fastwebsockets",
            "quinn",
            "quinn-proto",
            "quinn-udp",
            "h2",
            "h3",
        ],
    ),
    (
        "名前の引き",
        &[
            "hickory-resolver",
            "hickory-client",
            "hickory-proto",
            "trust-dns-resolver",
            "trust-dns-proto",
            "dns-lookup",
            "c-ares",
        ],
    ),
    (
        "テレメトリ",
        &[
            "sentry",
            "sentry-core",
            "opentelemetry",
            "opentelemetry-otlp",
            "posthog-rs",
        ],
    ),
    (
        "待ち受けの枠組み・TCP の部品",
        &[
            "axum",
            "warp",
            "actix-web",
            "rocket",
            "tide",
            "tiny_http",
            "rouille",
            "poem",
            "salvo",
            "async-net",
            "net2",
            "lettre",
        ],
    ),
];

/// 配るアプリ（yolu-app・yolu-cli）の依存に入ってはならない名前（試験だけの依存には出てよい）。
const BANNED_SHIPPED: &[&str] = &["open", "opener", "webbrowser"];
const SHIPPED_ROOTS: &[&str] = &["yolu-app", "yolu-cli"];

/// 低水準の通信の部品と、それに直接依存してよい（配るアプリの依存の中の）クレート。
const NET_PARTS: &[(&str, &[&str])] = &[
    ("socket2", &["tokio"]),
    ("mio", &["tokio"]),
    ("hyper", &["hyper-util", "yolu-mcp"]),
    ("hyper-util", &["yolu-mcp"]),
];

/// 直接の依存に持ってよいのが yolu-mcp だけのクレート（127.0.0.1 の受け口と客はここに閉じる）。
const NET_ONLY_IN: (&str, &[&str]) = ("yolu-mcp", &["hyper", "hyper-util", "socket2", "mio"]);
/// 通信・プロセスの起動に当たる tokio の機能（yolu-mcp だけが `net` を使う）。
const TOKIO_NET_FEATURES: &[&str] = &["net", "full", "process"];
/// Windows の API の機能のうち通信に当たる物の頭。WinHTTP（更新の確認）だけ、yolu-app に許す。
const WINDOWS_NET_FEATURE_PREFIXES: &[&str] = &["Win32_Networking_", "Win32_NetworkManagement_"];
const WINDOWS_NET_ALLOWED: &[(&str, &str)] = &[("yolu-app", "Win32_Networking_WinHttp")];

/// `Cargo.lock` の [[package]] の名前。
fn lock_names(lock: &str) -> BTreeSet<String> {
    lock.lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

fn check_lock(lock: &str) -> Vec<String> {
    let names = lock_names(lock);
    let mut out = Vec::new();
    for (kind, banned) in BANNED_ANYWHERE {
        for name in *banned {
            if names.contains(*name) {
                out.push(format!(
                    "Cargo.lock に {name}（{kind}）が入りました。アプリが更新の確認のほかに外へ通信する部品は入れません。\
                     入れる必要が出たら、README・INSTALL の約束を先に見直します。"
                ));
            }
        }
    }
    out
}

fn string_list(value: &serde_json::Value) -> Vec<&str> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

/// `cargo metadata`（依存も含む）の確かめ。
fn check_metadata(meta: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let empty = Vec::new();
    let packages = meta["packages"].as_array().unwrap_or(&empty);
    let members: BTreeSet<&str> = string_list(&meta["workspace_members"])
        .into_iter()
        .collect();
    // 作業場所のクレートの直接の依存
    for package in packages {
        let (Some(id), Some(name)) = (package["id"].as_str(), package["name"].as_str()) else {
            continue;
        };
        if !members.contains(id) {
            continue;
        }
        for dependency in package["dependencies"].as_array().unwrap_or(&empty) {
            let Some(dep) = dependency["name"].as_str() else {
                continue;
            };
            let features = string_list(&dependency["features"]);
            if name != NET_ONLY_IN.0 && NET_ONLY_IN.1.contains(&dep) {
                out.push(format!(
                    "{name} が {dep} を直接の依存に持ちます。待ち受け・通信の部品は {} だけが持ちます（アプリは yolu-mcp の関数を通して使います）。",
                    NET_ONLY_IN.0
                ));
            }
            if dep == "tokio" && name != NET_ONLY_IN.0 {
                for feature in features.iter().filter(|f| TOKIO_NET_FEATURES.contains(f)) {
                    out.push(format!(
                        "{name} が tokio の機能 {feature} を使います。通信・プロセスの起動に当たる機能は {} だけが使います。",
                        NET_ONLY_IN.0
                    ));
                }
            }
            if dep == "windows" || dep == "windows-sys" {
                for feature in features.iter().filter(|f| {
                    WINDOWS_NET_FEATURE_PREFIXES
                        .iter()
                        .any(|p| f.starts_with(p))
                }) {
                    if !WINDOWS_NET_ALLOWED.contains(&(name, feature)) {
                        out.push(format!(
                            "{name} が {dep} の機能 {feature}（通信）を使います。許すのは WinHTTP（更新の確認）だけです。"
                        ));
                    }
                }
            }
        }
    }
    // 配るアプリの依存の閉包（試験・ビルドだけの依存は含めない）
    let names: HashMap<&str, &str> = packages
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let nodes: HashMap<&str, &serde_json::Value> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut shipped: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = packages
        .iter()
        .filter_map(|p| {
            p["id"]
                .as_str()
                .filter(|_| SHIPPED_ROOTS.contains(&p["name"].as_str().unwrap_or("")))
        })
        .collect();
    let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    while let Some(id) = stack.pop() {
        if !shipped.insert(id) {
            continue;
        }
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().unwrap_or(&empty) {
            let normal = dep["dep_kinds"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .any(|k| k["kind"].is_null());
            let (Some(dep_id), true) = (dep["pkg"].as_str(), normal) else {
                continue;
            };
            if let (Some(from), Some(to)) = (names.get(id), names.get(dep_id)) {
                dependents.entry(to).or_default().insert(from);
            }
            stack.push(dep_id);
        }
    }
    let shipped_names: BTreeSet<&str> = shipped
        .iter()
        .filter_map(|id| names.get(id).copied())
        .collect();
    for root in SHIPPED_ROOTS {
        if !shipped_names.contains(root) {
            out.push(format!(
                "cargo metadata に {root} が見つかりません（配るアプリの依存を読めず、見張りが空振りになります）。"
            ));
        }
    }
    for banned in BANNED_SHIPPED {
        if shipped_names.contains(banned) {
            out.push(format!(
                "配るアプリ（{}）の依存に {banned}（ブラウザーや外のプログラムを開く部品）が入りました。「開く」は update/launch.rs・library/ops.rs・crash/mod.rs の既存の呼び出しだけです。",
                SHIPPED_ROOTS.join("・")
            ));
        }
    }
    for (part, allowed) in NET_PARTS {
        for dependent in dependents.get(part).into_iter().flatten() {
            if !allowed.contains(dependent) {
                out.push(format!(
                    "配るアプリの依存で、{dependent} が低水準の通信の部品 {part} に依存しています（許すのは {}）。\
                     通信の部品を新しく使う依存が入ったので、外へ通信しないか確かめ、よければ NET_PARTS に足します。",
                    allowed.join("・")
                ));
            }
        }
    }
    out
}

/// `cargo metadata --locked`。まずオフラインで読み、取得済みの依存が足りない（ほかの OS だけの依存を取っていない CI など）ときだけ、取得を許して読み直す。
fn read_metadata(root: &Path) -> Result<serde_json::Value> {
    let mut stderr = String::new();
    for offline in [true, false] {
        let mut command = cargo();
        command
            .current_dir(root)
            .args(["metadata", "--locked", "--format-version", "1"]);
        if offline {
            command.arg("--offline");
        }
        let output = command.output()?;
        if output.status.success() {
            return Ok(serde_json::from_slice(&output.stdout)?);
        }
        stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    Err(format!("cargo metadata が失敗しました: {stderr}").into())
}

// ───────── 入り口 ─────────

/// リポジトリの全部を確かめた結果。
fn check_repository(root: &Path) -> Result<(Report, usize)> {
    let sources = source_files(root)?;
    let scan = scan_sources(&sources);
    let mut report = evaluate(&scan, ALLOWED);
    report
        .violations
        .extend(check_lock(&fs::read_to_string(root.join("Cargo.lock"))?));
    report
        .violations
        .extend(check_metadata(&read_metadata(root)?));
    Ok((report, scan.files))
}

/// `cargo xtask netguard`: 許す一覧の使われ方の表を出し、落ちるものがあれば終了コード 1。
pub(crate) fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(arg) = args.next() {
        return Err(format!("未対応の引数: {arg}").into());
    }
    let (report, files) = check_repository(&root())?;
    println!("通信の見張り: ソース {files} ファイルを読みました。許す一覧の使われ方:");
    for (file, mark, count, why) in &report.table {
        println!("  {file}  {mark} ×{count}  {why}");
    }
    if report.violations.is_empty() {
        println!("通信の見張り: 一覧の外の出口も、依存の問題もありません。");
        return Ok(());
    }
    for violation in &report.violations {
        eprintln!("- {violation}");
    }
    Err(format!(
        "通信の見張りに {} 件、引っかかりました",
        report.violations.len()
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DEMO: &str = "crates/demo/src/lib.rs";

    fn scan_one(text: &str) -> Scan {
        scan_sources(&[(DEMO.to_owned(), text.to_owned())])
    }

    fn marks(text: &str) -> Vec<&'static str> {
        let mut found: Vec<&'static str> = scan_one(text).hits.iter().map(|h| h.mark).collect();
        found.sort();
        found.dedup();
        found
    }

    /// 印ごとの、その印だけ（か、その印を含む）の作り物のソース。`MARKERS` の全部にあること（足した印に試験を付け忘れない）。
    const SNIPPETS: &[(&str, &str)] = &[
        ("TcpStream", "fn f() { let _ = TcpStream::connect(a); }"),
        ("TcpListener", "fn f() { let _ = TcpListener::bind(a); }"),
        ("TcpSocket", "fn f() { let s = TcpSocket::new_v4(); }"),
        ("UdpSocket", "fn f() { let s = UdpSocket::bind(a); }"),
        ("ToSocketAddrs", "fn f(a: impl ToSocketAddrs) {}"),
        (
            "to_socket_addrs",
            "fn f() { let a = host.to_socket_addrs(); }",
        ),
        ("std::net", "use std::net::Ipv4Addr;"),
        (
            "tokio::net",
            "fn f() { let l = tokio::net::Foo::from_std(s); }",
        ),
        (
            "socket2",
            "fn f() { let s = socket2::Socket::new(d, t, None); }",
        ),
        ("mio", "fn f() { let p = mio::Poll::new(); }"),
        (
            "libc::socket",
            "fn f() { unsafe { libc::socket(2, 1, 0) }; }",
        ),
        (
            "libc::connect",
            "fn f() { unsafe { libc::connect(s, a, n) }; }",
        ),
        ("libc::bind", "fn f() { unsafe { libc::bind(s, a, n) }; }"),
        (
            "libc::sendto",
            "fn f() { unsafe { libc::sendto(s, b, n, 0, a, l) }; }",
        ),
        (
            "libc::getaddrinfo",
            "fn f() { unsafe { libc::getaddrinfo(h, s, a, r) }; }",
        ),
        (
            "getaddrinfo",
            "extern \"C\" { fn getaddrinfo(h: *const u8); }",
        ),
        (
            "gethostbyname",
            "extern \"C\" { fn gethostbyname(h: *const u8); }",
        ),
        ("AF_INET", "const D: i32 = AF_INET6;"),
        ("SOCK_STREAM", "const T: i32 = SOCK_STREAM;"),
        ("SOCK_DGRAM", "const T: i32 = SOCK_DGRAM;"),
        ("WSA", "fn f() { WSAStartup(0x202, &mut data); }"),
        (
            "WinSock",
            "use windows::Win32::Networking::WinSock::SOCKET;",
        ),
        ("hyper", "use hyper::Request;"),
        ("hyper_util", "use hyper_util::rt::TokioIo;"),
        ("WinHttp", "fn f() { WinHttpOpen(a, b, c, d, 0); }"),
        ("reqwest", "fn f() { let c = reqwest::Client::new(); }"),
        ("ureq", "fn f() { ureq::get(u); }"),
        ("isahc", "fn f() { isahc::get(u); }"),
        ("surf", "fn f() { surf::get(u); }"),
        ("attohttpc", "fn f() { attohttpc::get(u); }"),
        ("minreq", "fn f() { minreq::get(u); }"),
        ("curl", "fn f() { let e = curl::easy::Easy::new(); }"),
        ("tungstenite", "fn f() { tungstenite::connect(u); }"),
        ("quinn", "fn f() { quinn::Endpoint::client(a); }"),
        ("h2", "fn f() { h2::client::handshake(io); }"),
        ("rustls", "fn f() { rustls::ClientConfig::builder(); }"),
        ("native_tls", "fn f() { native_tls::TlsConnector::new(); }"),
        (
            "openssl",
            "fn f() { openssl::ssl::SslConnector::builder(m); }",
        ),
        ("\"curl\"", "const C: &str = \"curl\";"),
        ("\"http://\"", "const U: &str = \"http://example.com/x\";"),
        (
            "\"https://\"",
            "const U: &str = \"see https://example.com/x\";",
        ),
        (
            "Command::new",
            "fn f() { let c = std::process::Command::new(\"sh\"); }",
        ),
        ("libc::system", "fn f() { unsafe { libc::system(c) }; }"),
        (
            "ShellExecute",
            "fn f() { ShellExecuteW(h, v, u, p, d, 1); }",
        ),
        ("xdg-open", "const O: &str = \"xdg-open\";"),
        ("open::that", "fn f() { open::that(url); }"),
        ("open::with", "fn f() { open::with(url, app); }"),
        ("opener", "fn f() { opener::open(u); }"),
        ("webbrowser", "fn f() { webbrowser::open(u); }"),
        ("open_url", "fn f() { ctx.open_url(u); }"),
        ("OpenUrl", "fn f() { let o = OpenUrl::new_tab(u); }"),
        ("hyperlink", "fn f() { ui.hyperlink(u); }"),
        ("hyperlink_to", "fn f() { ui.hyperlink_to(t, u); }"),
    ];

    #[test]
    fn every_marker_is_found_in_made_up_source() {
        let named: BTreeSet<&str> = SNIPPETS.iter().map(|(name, _)| *name).collect();
        let known: BTreeSet<&str> = MARKERS.iter().map(|(name, _)| *name).collect();
        assert_eq!(named, known, "MARKERS と SNIPPETS の印が食い違っています");
        assert_eq!(
            SNIPPETS.len(),
            MARKERS.len(),
            "SNIPPETS に同じ印が 2 回あります"
        );
        for (name, text) in SNIPPETS {
            assert!(
                marks(text).contains(name),
                "{name} を含む作り物のソースが見つかりません: {text}"
            );
        }
    }

    #[test]
    fn comments_and_idents_in_strings_are_not_code() {
        let source = r##"
            // TcpStream は使わない
            /// http://127.0.0.1:1/mcp
            //! Command::new
            /* UdpSocket /* nested Command::new */ still comment */
            fn f<'a>(x: &'a str) -> &'a str {
                let _ = ('{', '}', '"', '\'', '\\', b'x', "TcpStream", r#"UdpSocket"#);
                x
            }
        "##;
        assert_eq!(marks(source), Vec::<&str>::new());
    }

    #[test]
    fn strings_of_every_kind_are_read() {
        let source = r####"
            const A: &str = "https://a.example";
            const B: &str = r#"https://b.example"#;
            const C: &[u8] = b"http://c.example";
            const D: &str = r###"quote " and "# inside, then https://d.example"###;
            const E: &str = "multi
line https://e.example";
            fn after() { let _ = std::process::Command::new("x"); }
        "####;
        let scan = scan_one(source);
        let https: Vec<usize> = scan
            .hits
            .iter()
            .filter(|h| h.mark == "\"https://\"")
            .map(|h| h.line)
            .collect();
        assert_eq!(https, vec![2, 3, 5, 6], "行番号は元のファイルの行");
        assert!(scan
            .hits
            .iter()
            .any(|h| h.mark == "\"http://\"" && h.line == 4));
        let command = scan.hits.iter().find(|h| h.mark == "Command::new").unwrap();
        assert_eq!(command.line, 8, "複数行の文字列のあとの行番号");
    }

    #[test]
    fn raw_identifiers_and_lifetimes_do_not_hide_code() {
        let source =
            "fn f<'a>(r#type: &'a str) { let l: &'static str = x; let _ = TcpStream::connect(a); }";
        assert_eq!(marks(source), vec!["TcpStream"]);
    }

    #[test]
    fn test_only_items_are_not_shipped_but_the_code_after_them_is() {
        let source = r#"
            #[cfg(test)]
            mod tests {
                use std::net::TcpStream;
                const BRACE: &str = "}";
                fn t() { let _ = TcpStream::connect(a); }
            }
            #[cfg(test)]
            fn helper() -> [u8; 4] { let _ = Command::new("x"); [0; 4] }
            #[cfg(test)]
            use std::net::UdpSocket;
            #[cfg(test)]
            const TABLE: [u8; 2] = [1, 2];
            #[cfg(test)]
            thread_local! { static N: u8 = const { 0 }; }
            #[cfg(all(test, windows))]
            fn windows_helper() { WinHttpOpen(); }
            struct S {
                #[cfg(test)]
                builds: Foo<TcpListener, u8>,
                kept: u8,
            }
            fn mixed() {
                #[cfg(test)]
                if x { Command::new("a"); } else { Command::new("b"); }
                #[cfg(test)]
                Command::new("c");
                let last = ShellExecuteW(a);
            }
        "#;
        // 試験の項目の中身は印にならず、後ろの本番のコード（ShellExecute だけ）が残る
        assert_eq!(marks(source), vec!["ShellExecute"]);
    }

    #[test]
    fn cfg_that_can_ship_is_still_scanned() {
        assert_eq!(
            marks("#[cfg(any(test, windows))] fn f() { WinHttpOpen(); }"),
            vec!["WinHttp"]
        );
        assert_eq!(
            marks("#[cfg(not(test))] fn f() { Command::new(\"x\"); }"),
            vec!["Command::new"]
        );
        assert_eq!(
            marks("#![cfg(test)]\nfn f() { Command::new(\"x\"); }"),
            Vec::<&str>::new(),
            "ファイル全体が試験"
        );
    }

    #[test]
    fn files_of_test_modules_are_not_scanned() {
        let hit = "fn f() { let _ = Command::new(\"x\"); }";
        let files = [
            (
                "crates/demo/src/lib.rs",
                "#[cfg(test)]\nmod tests;\nmod shipped;\n#[cfg(test)]\nmod helpers;\n",
            ),
            ("crates/demo/src/tests.rs", hit),
            (
                "crates/demo/src/shipped.rs",
                "#[cfg(test)]\nmod t;\nfn g() {}\n",
            ),
            ("crates/demo/src/shipped/t.rs", hit),
            ("crates/demo/src/helpers/mod.rs", hit),
            ("crates/demo/src/helpers/deeper.rs", hit),
            ("crates/demo/src/other.rs", hit),
        ]
        .map(|(file, text)| (file.to_owned(), text.to_owned()));
        let scan = scan_sources(&files);
        let hit_files: Vec<&str> = scan.hits.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(hit_files, vec!["crates/demo/src/other.rs"]);
    }

    fn allow(file: &'static str, mark: &'static str, count: Count) -> Allow {
        Allow {
            file,
            mark,
            count,
            why: "試験の理由",
        }
    }

    #[test]
    fn an_unlisted_mark_names_the_file_the_line_and_how_to_allow_it() {
        let scan = scan_one(
            "fn f() {}\n\nfn g() { let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, p)); }\n",
        );
        let report = evaluate(&scan, &[]);
        assert_eq!(report.violations.len(), 1);
        let text = &report.violations[0];
        assert!(text.contains("crates/demo/src/lib.rs:3"), "{text}");
        assert!(text.contains("TcpStream"), "{text}");
        assert!(text.contains("ALLOWED"), "{text}");
        assert!(text.contains("理由") || text.contains("README"), "{text}");
    }

    #[test]
    fn a_listed_mark_passes_and_the_count_is_pinned() {
        let one = "fn f() { let _ = Command::new(\"a\"); }";
        let two = "fn f() { let _ = Command::new(\"a\"); let _ = Command::new(\"b\"); }";
        let pinned = [allow(DEMO, "Command::new", Exactly(1))];
        let free = [allow(DEMO, "Command::new", AtLeastOne)];
        let ok = evaluate(&scan_one(one), &pinned);
        assert!(ok.violations.is_empty(), "{:?}", ok.violations);
        assert_eq!(ok.table.len(), 1);
        let more = evaluate(&scan_one(two), &pinned);
        assert_eq!(more.violations.len(), 1, "2 つ目の出口は落ちる");
        assert!(
            more.violations[0].contains("2 件"),
            "{}",
            more.violations[0]
        );
        assert!(evaluate(&scan_one(two), &free).violations.is_empty());
        // 一覧は印ごと: 別の印は通らない
        let other = evaluate(&scan_one("fn f() { ShellExecuteW(a); }"), &pinned);
        assert!(other.violations.iter().any(|v| v.contains("ShellExecute")));
    }

    #[test]
    fn a_row_that_matches_nothing_is_reported() {
        let report = evaluate(
            &scan_one("fn f() {}"),
            &[allow(DEMO, "TcpStream", AtLeastOne)],
        );
        assert_eq!(report.violations.len(), 1);
        assert!(
            report.violations[0].contains("見つかりません"),
            "{}",
            report.violations[0]
        );
    }

    fn endpoints(text: &str) -> Vec<String> {
        scan_one(text)
            .endpoints
            .into_iter()
            .map(|(_, _, call)| call)
            .collect()
    }

    #[test]
    fn bind_and_connect_must_name_the_loopback() {
        for ok in [
            "fn f() { TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))); }",
            "fn f() { TcpStream::connect((Ipv4Addr::LOCALHOST, port)); }",
            "fn f() { TcpStream::connect((\"127.0.0.1\", port)); }",
            "fn f() { StdListener::bind(\"[::1]:0\"); }",
            "fn f() { UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)); }",
            "fn f() { TcpListener::bind(\n    SocketAddr::new(\n        IpAddr::V4(Ipv4Addr::LOOPBACK),\n        port,\n    ),\n); }",
            "fn f() { let a = Ipv4Addr::LOCALHOST; let _ = TcpStream::connect((a, port)); }",
            "fn f() { Foo::bind(port); Bar::connect(host); }",
        ] {
            assert_eq!(endpoints(ok), Vec::<String>::new(), "{ok}");
        }
        for bad in [
            "fn f() { TcpListener::bind(\"0.0.0.0:80\"); }",
            "fn f() { TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)); }",
            "fn f() { TcpStream::connect((host, 80)); }",
            "fn f() { UdpSocket::bind((\"0.0.0.0\", 0)); }",
            "fn f() { TcpStream::connect_timeout(&addr, timeout); }",
            "fn f() {\n    let addr = Ipv4Addr::LOCALHOST;\n    TcpListener::bind(addr);\n}",
            "fn f() { TcpListener::bind(addr) /* LOCALHOST */; }",
        ] {
            assert_eq!(endpoints(bad).len(), 1, "{bad}");
        }
        let report = evaluate(&scan_one("fn f() { TcpListener::bind(a); }"), &[]);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.contains("lib.rs:1") && v.contains("127.0.0.1")),
            "{:?}",
            report.violations
        );
    }

    #[test]
    fn the_allowed_list_is_well_formed() {
        let known: BTreeSet<&str> = MARKERS.iter().map(|(name, _)| *name).collect();
        let mut seen = BTreeSet::new();
        for row in ALLOWED {
            assert!(
                known.contains(row.mark),
                "{} の印 {} は MARKERS に無い",
                row.file,
                row.mark
            );
            assert!(
                row.why.trim().chars().count() >= 8,
                "{} の {} に理由が要る",
                row.file,
                row.mark
            );
            assert!(
                seen.insert((row.file, row.mark)),
                "{} の {} が二重",
                row.file,
                row.mark
            );
            assert!(
                row.file.starts_with("crates/") && !row.file.starts_with("crates/xtask/"),
                "{}",
                row.file
            );
            if let Exactly(n) = row.count {
                assert!(n > 0, "{} の {}", row.file, row.mark);
            }
        }
    }

    #[test]
    fn the_repository_keeps_to_the_allowed_network_surface() {
        let (report, files) = check_repository(&root()).unwrap();
        assert!(files > 300, "ソースを読めていません（{files} ファイル）");
        assert!(
            !report.table.is_empty(),
            "一覧の行が 1 つも使われていません"
        );
        assert!(
            report.violations.is_empty(),
            "通信の見張りに引っかかりました:\n- {}",
            report.violations.join("\n- ")
        );
    }

    #[test]
    fn the_scan_skips_xtask_and_reads_build_scripts() {
        let files = source_files(&root()).unwrap();
        assert!(files
            .iter()
            .all(|(file, _)| !file.starts_with("crates/xtask/")));
        assert!(files
            .iter()
            .any(|(file, _)| file == "crates/yolu-app/build.rs"));
        assert!(files
            .iter()
            .any(|(file, _)| file == "crates/yolu-mcp/src/http.rs"));
    }

    // ───────── 依存 ─────────

    #[test]
    fn the_lock_file_refuses_http_clients_and_the_like() {
        let lock = |names: &[&str]| {
            names
                .iter()
                .map(|n| format!("[[package]]\nname = \"{n}\"\nversion = \"1.0.0\"\n"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(check_lock(&lock(&["hyper", "tokio", "mio", "socket2", "open"])).is_empty());
        for (name, kind) in [
            ("reqwest", "HTTP"),
            ("ureq", "HTTP"),
            ("curl-sys", "HTTP"),
            ("tungstenite", "WebSocket"),
            ("quinn", "QUIC"),
            ("h2", "HTTP/2"),
            ("rustls", "TLS"),
            ("hickory-resolver", "名前の引き"),
            ("sentry", "テレメトリ"),
            ("axum", "待ち受け"),
        ] {
            let found = check_lock(&lock(&["hyper", name]));
            assert_eq!(found.len(), 1, "{name}");
            assert!(
                found[0].contains(name) && found[0].contains(kind),
                "{}",
                found[0]
            );
        }
        assert_eq!(check_lock("name = \"x\"\n"), Vec::<String>::new());
    }

    /// 作り物の `cargo metadata`。`direct` は（作業場所のクレート・依存の名前・機能）、`edges` は（元・先・種類。null は本番）。
    fn meta(
        direct: &[(&str, &str, &[&str])],
        edges: &[(&str, &str, Option<&str>)],
    ) -> serde_json::Value {
        let mut names: BTreeSet<&str> = ["yolu-app", "yolu-cli", "yolu-mcp"].into_iter().collect();
        for (from, to, _) in edges {
            names.extend([*from, *to]);
        }
        for (member, dep, _) in direct {
            names.extend([*member, *dep]);
        }
        let packages: Vec<_> = names
            .iter()
            .map(|name| {
                let dependencies: Vec<_> = direct
                    .iter()
                    .filter(|(member, _, _)| member == name)
                    .map(|(_, dep, features)| json!({ "name": dep, "features": features }))
                    .collect();
                json!({ "id": name, "name": name, "dependencies": dependencies })
            })
            .collect();
        let nodes: Vec<_> = names
            .iter()
            .map(|name| {
                let deps: Vec<_> = edges
                    .iter()
                    .filter(|(from, _, _)| from == name)
                    .map(|(_, to, kind)| json!({ "pkg": to, "dep_kinds": [{ "kind": kind }] }))
                    .collect();
                json!({ "id": name, "deps": deps })
            })
            .collect();
        json!({
            "workspace_members": ["yolu-app", "yolu-cli", "yolu-mcp"],
            "packages": packages,
            "resolve": { "nodes": nodes },
        })
    }

    /// 今のリポジトリと同じ形（受け口の部品は yolu-mcp の下だけ）。
    fn good_direct() -> Vec<(&'static str, &'static str, &'static [&'static str])> {
        vec![
            ("yolu-mcp", "hyper", &["server", "client", "http1"]),
            ("yolu-mcp", "hyper-util", &["tokio"]),
            ("yolu-mcp", "tokio", &["rt", "net", "time"]),
            ("yolu-app", "tokio", &["sync"]),
            ("yolu-cli", "tokio", &["rt", "io-std", "sync"]),
            (
                "yolu-app",
                "windows",
                &["Win32_Networking_WinHttp", "Win32_UI_Shell"],
            ),
        ]
    }

    fn good_edges() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
        vec![
            ("yolu-app", "yolu-mcp", None),
            ("yolu-cli", "yolu-mcp", None),
            ("yolu-app", "tokio", None),
            ("yolu-mcp", "hyper", None),
            ("yolu-mcp", "hyper-util", None),
            ("hyper-util", "hyper", None),
            ("yolu-mcp", "tokio", None),
            ("tokio", "mio", None),
            ("tokio", "socket2", None),
            ("hyper", "tokio", None),
            ("yolu-app", "open", Some("dev")),
        ]
    }

    fn check(
        direct: &[(&str, &str, &[&str])],
        edges: &[(&str, &str, Option<&str>)],
    ) -> Vec<String> {
        check_metadata(&meta(direct, edges))
    }

    #[test]
    fn the_present_dependency_shape_passes() {
        assert_eq!(check(&good_direct(), &good_edges()), Vec::<String>::new());
    }

    #[test]
    fn only_the_mcp_crate_may_hold_the_listening_parts() {
        for dep in ["hyper", "hyper-util", "socket2", "mio"] {
            for member in ["yolu-app", "yolu-cli"] {
                let mut direct = good_direct();
                direct.push((member, dep, &[]));
                let found = check(&direct, &good_edges());
                assert_eq!(found.len(), 1, "{member} → {dep}: {found:?}");
                assert!(
                    found[0].contains(member) && found[0].contains(dep),
                    "{}",
                    found[0]
                );
            }
        }
    }

    #[test]
    fn tokio_net_process_and_full_are_for_the_mcp_crate_only() {
        for feature in ["net", "process", "full"] {
            let features = ["rt", feature];
            let mut direct = good_direct();
            direct.push(("yolu-cli", "tokio", &features));
            let found = check(&direct, &good_edges());
            assert_eq!(found.len(), 1, "{feature}: {found:?}");
            assert!(found[0].contains(feature), "{}", found[0]);
        }
        let mut direct = good_direct();
        direct.push((
            "yolu-app",
            "tokio",
            &["sync", "time", "rt", "macros", "io-util"],
        ));
        assert!(
            check(&direct, &good_edges()).is_empty(),
            "net 以外の機能は通る"
        );
    }

    #[test]
    fn windows_networking_is_winhttp_in_the_app_only() {
        for feature in [
            "Win32_Networking_WinSock",
            "Win32_NetworkManagement_IpHelper",
        ] {
            let features = [feature];
            let mut direct = good_direct();
            direct.push(("yolu-app", "windows", &features));
            let found = check(&direct, &good_edges());
            assert_eq!(found.len(), 1, "{feature}: {found:?}");
        }
        let mut direct = good_direct();
        direct.push(("yolu-cli", "windows-sys", &["Win32_Networking_WinHttp"]));
        assert_eq!(
            check(&direct, &good_edges()).len(),
            1,
            "WinHTTP も yolu-app だけ"
        );
    }

    #[test]
    fn the_shipped_app_may_not_depend_on_an_opener_crate() {
        for opener in ["open", "opener", "webbrowser"] {
            let mut edges = good_edges();
            edges.push(("yolu-app", opener, None));
            let found = check(&good_direct(), &edges);
            assert!(
                found.iter().any(|v| v.contains(opener)),
                "{opener}: {found:?}"
            );
            // 試験・ビルドだけの依存（今の egui_kittest 経由の open と同じ）は配る物に入らない
            let mut edges = good_edges();
            edges.push(("yolu-app", opener, Some("dev")));
            edges.push(("yolu-app", opener, Some("build")));
            assert!(check(&good_direct(), &edges).is_empty(), "{opener}");
        }
        // 間接の依存でも見つける
        let mut edges = good_edges();
        edges.push(("yolu-cli", "helper", None));
        edges.push(("helper", "webbrowser", None));
        assert!(!check(&good_direct(), &edges).is_empty());
    }

    #[test]
    fn a_new_user_of_the_low_level_parts_is_reported() {
        for part in ["socket2", "mio", "hyper", "hyper-util"] {
            let mut edges = good_edges();
            edges.push(("yolu-app", "newcomer", None));
            edges.push(("newcomer", part, None));
            let found = check(&good_direct(), &edges);
            assert_eq!(found.len(), 1, "{part}: {found:?}");
            assert!(
                found[0].contains("newcomer") && found[0].contains(part),
                "{}",
                found[0]
            );
        }
        // 試験だけの依存なら配る物に入らない
        let mut edges = good_edges();
        edges.push(("yolu-app", "newcomer", Some("dev")));
        edges.push(("newcomer", "socket2", None));
        assert!(check(&good_direct(), &edges).is_empty());
    }

    #[test]
    fn an_empty_metadata_does_not_pass_silently() {
        let found = check_metadata(
            &json!({ "workspace_members": [], "packages": [], "resolve": { "nodes": [] } }),
        );
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|v| v.contains("見つかりません")));
    }

    #[test]
    fn the_command_takes_no_arguments() {
        assert!(run(["--list".to_owned()].into_iter()).is_err());
    }
}
