//! 通信と外への出口の印（[`MARKERS`]）と、ソケットの宛先の確かめ。
use super::lex::*;

// ───────── 印 ─────────

pub(super) enum Pat {
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
pub(super) const MARKERS: &[(&str, Pat)] = &[
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

pub(super) fn marker_matches(pat: &Pat, t: &[Token], i: usize) -> bool {
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
pub(super) struct Hit {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) mark: &'static str,
}

/// `a::bind(…)`・`a::connect(…)` で、`a` が Listener・Stream・Socket を含む型の名前のとき、宛先が 127.0.0.1 と読めるか。
/// 読めなければ（ファイル・行・呼び出し）を返す。呼び出しの引数か、その呼び出しと同じ行に、`LOCALHOST`・`LOOPBACK`・`localhost`・`127.0.0.1`・`::1` があればよい。
pub(super) fn endpoint_failures(file: &str, t: &[Token]) -> Vec<(String, usize, String)> {
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
pub(super) fn scan_tokens(
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
