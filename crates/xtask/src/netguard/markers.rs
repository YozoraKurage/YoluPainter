//! 通信と外への出口の印（[`MARKERS`]）と、ソケットの宛先の確かめ。
use super::lex::*;
use super::uses::use_bindings;
use std::collections::BTreeSet;

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
    /// 外のプログラムを起こす `Command::new`（`std::process`・`tokio::process`）。別名（`as Cmd`）と、自前の `Command` は、ファイルの `use` と定義から見分ける。
    Spawn,
}
use Pat::{Crate, Ident, Path as PathPat, Prefix, Spawn, StrHas, StrIs};

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
    ("rustix::net", PathPat(&["rustix", "net"])),
    ("nix::sys::socket", PathPat(&["nix", "sys", "socket"])),
    ("send_to", Ident("send_to")),
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
    ("WinInet", Prefix("WinInet")),
    ("InternetOpen", Prefix("InternetOpen")),
    ("InternetConnect", Prefix("InternetConnect")),
    ("Urlmon", Prefix("Urlmon")),
    ("URLDownloadToFile", Prefix("URLDownloadToFile")),
    ("HttpClient", Ident("HttpClient")),
    ("Networking", Ident("Networking")),
    ("NetworkManagement", Ident("NetworkManagement")),
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
    ("Command::new", Spawn),
    ("tokio::process", PathPat(&["tokio", "process"])),
    ("async_process", Crate("async_process")),
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

/// ファイルごとの文脈（`use` と定義から。`Command::new` と bind・connect の受け手を見分ける）。
#[derive(Debug, Default)]
pub(super) struct Ctx {
    /// `std::process::Command`・`tokio::process::Command` が入っている名前（別名を含む）。
    process_commands: BTreeSet<String>,
    /// `process` の module が入っている名前（`use std::process as p;` の `p`）。
    process_modules: BTreeSet<String>,
    /// `Command` という名前の、process と関係のない型を使っているか（`use` か定義）。
    foreign_command: bool,
    /// ソケットの型が入っている名前（`use … TcpListener as L;`・`type S = TcpStream;`）。
    socket_types: BTreeSet<String>,
}

fn is_socket_type(name: &str) -> bool {
    ["Listener", "Stream", "Socket"]
        .iter()
        .any(|kind| name.contains(kind))
}

impl Ctx {
    pub(super) fn of(t: &[Token]) -> Ctx {
        let mut ctx = Ctx::default();
        for (name, path) in use_bindings(t) {
            let original = path.last().map(String::as_str).unwrap_or("");
            let through_process = path.iter().any(|p| p == "process");
            if name == "*" && through_process {
                ctx.process_commands.insert("Command".into());
            } else if original == "Command" && through_process {
                ctx.process_commands.insert(name.clone());
            } else if name == "Command" && !through_process {
                ctx.foreign_command = true;
            }
            if original == "process" {
                ctx.process_modules.insert(name.clone());
            }
            if is_socket_type(original) {
                ctx.socket_types.insert(name);
            }
        }
        // `struct Command`・`type Sock = TcpStream;`
        for i in 0..t.len() {
            match ident_at(t, i) {
                Some("struct" | "enum" | "union" | "trait")
                    if ident_at(t, i + 1) == Some("Command") =>
                {
                    ctx.foreign_command = true;
                }
                Some("type") => {
                    let Some(name) = ident_at(t, i + 1) else {
                        continue;
                    };
                    let end = (i..t.len())
                        .find(|&k| is_punct(t, k, ';'))
                        .unwrap_or(t.len());
                    let rhs = &t[i + 2..end.max(i + 2)];
                    let idents: Vec<&str> =
                        (0..rhs.len()).filter_map(|k| ident_at(rhs, k)).collect();
                    if idents
                        .windows(2)
                        .any(|w| w[0] == "process" && w[1] == "Command")
                    {
                        ctx.process_commands.insert(name.to_owned());
                    } else if name == "Command" {
                        ctx.foreign_command = true;
                    }
                    if idents.iter().any(|w| is_socket_type(w)) {
                        ctx.socket_types.insert(name.to_owned());
                    }
                }
                _ => {}
            }
        }
        ctx
    }

    /// `X::new` の `X` が、外のプログラムを起こす `Command` か。
    fn spawns(&self, t: &[Token], i: usize) -> bool {
        let Some(name) = ident_at(t, i) else {
            return false;
        };
        if !(matches!(t.get(i + 1), Some(Token { tok: Tok::Sep, .. }))
            && ident_at(t, i + 2) == Some("new"))
        {
            return false;
        }
        // `process::Command::new`・`p::Command::new`
        let qualified = i >= 2 && matches!(t.get(i - 1), Some(Token { tok: Tok::Sep, .. }));
        if name == "Command" && qualified {
            return ident_at(t, i - 2)
                .is_some_and(|m| m == "process" || self.process_modules.contains(m));
        }
        if qualified {
            return false;
        }
        self.process_commands.contains(name) || (name == "Command" && !self.foreign_command)
    }
}

pub(super) fn marker_matches(pat: &Pat, t: &[Token], i: usize, ctx: &Ctx) -> bool {
    match (pat, &t[i].tok) {
        (Spawn, Tok::Ident(_)) => ctx.spawns(t, i),
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

fn is_sep_at(t: &[Token], i: usize) -> bool {
    matches!(t.get(i), Some(Token { tok: Tok::Sep, .. }))
}

/// 文字列が 127.0.0.1・::1・localhost の宛先か（`"127.0.0.1:0"`・`"[::1]:0"`・`"localhost:80"`）。
fn loopback_str(s: &str) -> bool {
    s.starts_with("127.0.0.1")
        || s.starts_with("[::1]")
        || s.starts_with("::1")
        || s.starts_with("localhost")
}

/// コンマで区切った（括弧の外の）最初の要素と、コンマがあったか。
fn first_element(t: &[Token]) -> (&[Token], bool) {
    let mut depth = 0usize;
    for (k, token) in t.iter().enumerate() {
        match token.tok {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => depth = depth.saturating_sub(1),
            Tok::Punct(',') if depth == 0 => return (&t[..k], true),
            _ => {}
        }
    }
    (t, false)
}

/// 宛先の「ホストの式」が、ループバックと決まる形か。許す形は次の 4 つだけ:
/// 文字列（`"127.0.0.1"`）、`Ipv4Addr::LOCALHOST`・`Ipv6Addr::LOOPBACK` などの定数、それを包む `IpAddr::V4(…)`・`IpAddr::V6(…)`。
/// 変数・`unwrap_or`・`if`・`match`・`UNSPECIFIED`・`"0.0.0.0"` は、ループバックと決められないので通さない。
fn loopback_host(t: &[Token]) -> bool {
    if let [Token {
        tok: Tok::Str(s), ..
    }] = t
    {
        return loopback_str(s);
    }
    // 道 `a::b::C`
    let names: Vec<&str> = t
        .iter()
        .enumerate()
        .filter_map(|(k, token)| match &token.tok {
            Tok::Ident(s) if k % 2 == 0 => Some(s.as_str()),
            _ => None,
        })
        .collect();
    let is_path = !t.is_empty()
        && t.len() % 2 == 1
        && t.iter().enumerate().all(|(k, token)| match token.tok {
            Tok::Ident(_) => k % 2 == 0,
            Tok::Sep => k % 2 == 1,
            _ => false,
        });
    if is_path {
        let n = names.len();
        return n >= 2
            && matches!(names[n - 1], "LOCALHOST" | "LOOPBACK")
            && names[n - 2].ends_with("Addr");
    }
    // `IpAddr::V4(host)`・`IpAddr::V6(host)`
    if t.len() >= 5 && is_punct(t, t.len() - 1, ')') {
        if let Some(open) = (0..t.len()).find(|&k| is_punct(t, k, '(')) {
            let callee = &t[..open];
            let wrapper = open >= 3
                && is_sep_at(callee, open - 2)
                && matches!(ident_at(callee, open - 1), Some("V4" | "V6"))
                && ident_at(callee, open - 3) == Some("IpAddr")
                && (open == 3 || is_sep_at(callee, open - 4))
                && matching(t, open) == t.len() - 1;
            if wrapper {
                return loopback_host(&t[open + 1..t.len() - 1]);
            }
        }
    }
    false
}

/// 宛先の引数全体が、ループバックと決まる形か。許す形: 文字列、`format!("127.0.0.1:{port}")`、`(host, port)` の組、
/// `SocketAddr::from(…)`・`SocketAddr::new(host, port)`・`SocketAddr::V4(…)`・`SocketAddrV4::new(host, port)` などの包み。
fn loopback_endpoint(t: &[Token]) -> bool {
    // 末尾のコンマ（`bind(addr,)`・`(host, port,)`）は読まない
    let t = if is_punct(t, t.len().wrapping_sub(1), ',') {
        &t[..t.len() - 1]
    } else {
        t
    };
    if let [Token {
        tok: Tok::Str(s), ..
    }] = t
    {
        return loopback_str(s);
    }
    // `format!("…", …)`
    if ident_at(t, 0) == Some("format") && is_punct(t, 1, '!') && is_punct(t, 2, '(') {
        return matches!(t.get(3), Some(Token { tok: Tok::Str(s), .. }) if loopback_str(s));
    }
    // 括弧 1 つ: 中を読む
    if is_punct(t, 0, '(') && matching(t, 0) == t.len() - 1 {
        return loopback_endpoint(&t[1..t.len() - 1]);
    }
    // `host, port`
    let (first, comma) = first_element(t);
    if comma {
        return loopback_host(first);
    }
    // `SocketAddr::from(…)` などの包み
    if let Some(open) = (0..t.len()).find(|&k| is_punct(t, k, '(')) {
        let wrapper = open >= 3
            && is_sep_at(t, open - 2)
            && matches!(ident_at(t, open - 1), Some("from" | "new" | "V4" | "V6"))
            && ident_at(t, open - 3).is_some_and(|r| r.starts_with("SocketAddr"))
            && matching(t, open) == t.len() - 1;
        if wrapper {
            return loopback_endpoint(&t[open + 1..t.len() - 1]);
        }
    }
    false
}

/// `a::bind(…)`・`a::connect(…)` で、`a` がソケットの型（名前に Listener・Stream・Socket を含む型と、`use … as 別名`・`type 別名 = …` の別名）のとき、
/// 宛先が 127.0.0.1 と決まる形（[`loopback_endpoint`]）か。違えば（ファイル・行・呼び出し）を返す。
pub(super) fn endpoint_failures(
    file: &str,
    t: &[Token],
    ctx: &Ctx,
) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for i in 0..t.len() {
        let Some(receiver) = ident_at(t, i) else {
            continue;
        };
        if !is_sep_at(t, i + 1) {
            continue;
        }
        let Some(function) = ident_at(t, i + 2) else {
            continue;
        };
        if !matches!(function, "bind" | "connect" | "connect_timeout")
            || !(is_socket_type(receiver) || ctx.socket_types.contains(receiver))
            || !is_punct(t, i + 3, '(')
        {
            continue;
        }
        let end = matching(t, i + 3).min(t.len());
        if !loopback_endpoint(&t[i + 4..end]) {
            out.push((
                file.to_owned(),
                t[i].line,
                format!("{receiver}::{function}(…)"),
            ));
        }
    }
    out
}

/// 1 つのファイルの字句（試験の項目は除いてある）から、印と、宛先が 127.0.0.1 と決まらない bind・connect を集める。
pub(super) fn scan_tokens(
    file: &str,
    tokens: &[Token],
    hits: &mut Vec<Hit>,
    endpoints: &mut Vec<(String, usize, String)>,
) {
    let ctx = Ctx::of(tokens);
    for i in 0..tokens.len() {
        for (mark, pat) in MARKERS {
            if marker_matches(pat, tokens, i, &ctx) {
                hits.push(Hit {
                    file: file.to_owned(),
                    line: tokens[i].line,
                    mark,
                });
            }
        }
    }
    endpoints.extend(endpoint_failures(file, tokens, &ctx));
}
