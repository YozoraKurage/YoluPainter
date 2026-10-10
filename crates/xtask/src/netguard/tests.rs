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
    (
        "tokio::process",
        "fn f() { let c = tokio::process::Command::from(s); }",
    ),
    (
        "async_process",
        "fn f() { let c = async_process::Command::from(s); }",
    ),
    (
        "rustix::net",
        "fn f() { let s = rustix::net::socket(d, t, p); }",
    ),
    (
        "nix::sys::socket",
        "use nix::sys::socket::{socket, AddressFamily};",
    ),
    ("send_to", "fn f() { sock.send_to(b, a); }"),
    (
        "WinInet",
        "use windows::Win32::Networking::WinInet::HINTERNET;",
    ),
    ("InternetOpen", "fn f() { InternetOpenW(a, 0, p, p, 0); }"),
    (
        "InternetConnect",
        "fn f() { InternetConnectW(h, s, 80, u, p, 3, 0, 0); }",
    ),
    (
        "Urlmon",
        "use windows::Win32::System::Com::Urlmon::URLOpenStreamW;",
    ),
    (
        "URLDownloadToFile",
        "fn f() { URLDownloadToFileW(c, u, f, 0, cb); }",
    ),
    ("HttpClient", "fn f() { let c = HttpClient::new(); }"),
    ("Networking", "use windows::Win32::Networking::Foo;"),
    (
        "NetworkManagement",
        "use windows::Win32::NetworkManagement::IpHelper::X;",
    ),
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
    let scan =
        scan_one("fn f() {}\n\nfn g() { let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, p)); }\n");
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
        "fn f() { TcpStream::connect(format!(\"127.0.0.1:{port}\")); }",
        "fn f() { UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)); }",
        "fn f() { TcpListener::bind(\n    SocketAddr::new(\n        IpAddr::V4(Ipv4Addr::LOOPBACK),\n        port,\n    ),\n); }",
        "fn f() { TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))); }",
        "fn f() { TcpStream::connect(std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port)); }",
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
        // LOCALHOST の文字があっても、宛先が決まらない形
        "fn f() { TcpStream::connect((host.unwrap_or(Ipv4Addr::LOCALHOST), port)); }",
        "fn f() { TcpListener::bind((if lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST }, port)); }",
        "fn f() { TcpListener::bind(if lan { UNSPECIFIED } else { LOCALHOST }); }",
        "fn f() { TcpListener::bind(match mode { Lan => ADDR, _ => LOCALHOST }); }",
        "fn f() { let a = Ipv4Addr::LOCALHOST; let _ = TcpStream::connect((a, port)); }",
        "fn f() { TcpListener::bind((Ipv4Addr::LOCALHOST, port).max(any)); }",
        "fn f() { TcpStream::connect((Ipv4Addr::new(10, 0, 0, 1), port)); }",
        "fn f() { TcpListener::bind(SocketAddr::from((addr_from_settings(), port))); }",
        "fn f() { TcpListener::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)); }",
        "fn f() { TcpListener::bind(\"0.0.0.0:80\") /* \"127.0.0.1\" */; }",
        "fn f() { TcpStream::connect(format!(\"{host}:127\")); }",
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
fn an_aliased_socket_type_is_still_a_socket() {
    for bad in [
        "use std::net::TcpListener as L;\nfn f() { L::bind((\"0.0.0.0\", 80)); }",
        "use std::net::{TcpStream as S, Ipv4Addr};\nfn f() { S::connect((host, 80)); }",
        "use tokio::net::UdpSocket as Sock;\nfn f() { Sock::bind(addr); }",
        "type Conn = std::net::TcpStream;\nfn f() { Conn::connect(addr); }",
    ] {
        assert_eq!(endpoints(bad).len(), 1, "{bad}");
    }
    assert_eq!(
        endpoints(
            "use std::net::TcpListener as L;\nfn f() { L::bind((Ipv4Addr::LOCALHOST, 80)); }"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn process_commands_are_found_under_any_name() {
    for source in [
        "use std::process::Command as Cmd;\nfn f() { Cmd::new(\"x\"); }",
        "use std::process::{Stdio, Command as C2};\nfn f() { C2::new(\"x\"); }",
        "use std::process::Command;\nfn f() { Command::new(\"x\"); }",
        "use std::process::*;\nfn f() { Command::new(\"x\"); }",
        "use std::process as p;\nfn f() { p::Command::new(\"x\"); }",
        "use std::process;\nfn f() { process::Command::new(\"x\"); }",
        "use tokio::process::Command;\nfn f() { Command::new(\"x\"); }",
        "use tokio::process::Command as Tc;\nfn f() { Tc::new(\"x\"); }",
        "type Spawner = std::process::Command;\nfn f() { Spawner::new(\"x\"); }",
        "fn f() { std::process::Command::new(\"x\"); }",
        "fn f() { let c = Command::new(\"x\"); }",
    ] {
        assert!(marks(source).contains(&"Command::new"), "{source}");
    }
    assert!(marks("fn f() { tokio::process::Command::from(c); }").contains(&"tokio::process"));
}

#[test]
fn a_command_type_of_the_program_itself_is_not_a_spawn() {
    for source in [
        "struct Command { name: String }\nimpl Command { fn new(n: &str) -> Command { Command { name: n.into() } } }\nfn f() { Command::new(\"x\"); }",
        "use crate::commands::Command;\nfn f() { Command::new(\"x\"); }",
        "enum Command { A }\nfn f() { Command::new(); }",
        "fn f() { crate::commands::Command::new(\"x\"); }",
        "use egui::Context as Command;\nfn f() { Command::new(); }",
    ] {
        assert!(!marks(source).contains(&"Command::new"), "{source}");
    }
}

fn scan_files(files: &[(&str, &str)]) -> Vec<(String, &'static str)> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(f, t)| (f.to_string(), t.to_string()))
        .collect();
    let mut found: Vec<(String, &'static str)> = scan_sources(&owned)
        .hits
        .into_iter()
        .map(|h| (h.file, h.mark))
        .collect();
    found.sort();
    found.dedup();
    found
}

#[test]
fn test_module_files_named_by_a_path_attribute_are_not_scanned() {
    let hit = "fn f() { std::process::Command::new(\"x\"); }";
    let found = scan_files(&[
        (
            "crates/demo/src/lib.rs",
            "#[cfg(test)]\n#[path = \"helpers/check.rs\"]\nmod check;\n#[path = \"real.rs\"]\nmod other_name;\n#[cfg(test)]\n#[path = \"../tests_support/up.rs\"]\nmod up;\n",
        ),
        ("crates/demo/src/helpers/check.rs", hit),
        ("crates/demo/src/real.rs", hit),
        ("crates/demo/src/check.rs", hit),
        ("crates/demo/tests_support/up.rs", hit),
    ]);
    // path で指された試験のファイルは除く。名前が同じだけの別のファイル（src/check.rs）と、道の指していないファイルは読む
    assert_eq!(
        found,
        vec![
            ("crates/demo/src/check.rs".to_owned(), "Command::new"),
            ("crates/demo/src/real.rs".to_owned(), "Command::new"),
        ]
    );
}

#[test]
fn an_inner_cfg_test_removes_only_its_own_module() {
    let source = "
        mod a {
            #![cfg(test)]
            fn t() { std::process::Command::new(\"a\"); }
        }
        #[allow(dead_code)]
        pub mod b {
            #![cfg(test)]
            fn t() { ShellExecuteW(); }
        }
        mod c {
            fn real() { WinHttpOpen(); }
        }
        fn prod() { UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)); }
    ";
    assert_eq!(marks(source), vec!["UdpSocket", "WinHttp"]);
    // ファイルの先頭の `#![cfg(test)]` は丸ごと
    assert_eq!(
        marks("#![allow(unused)]\n#![cfg(test)]\nfn t() { UdpSocket::bind(a); }"),
        Vec::<&str>::new()
    );
}

#[test]
fn code_after_a_test_only_arm_or_labelled_loop_is_still_read() {
    let arms = "
        fn f(x: u8) {
            match x {
                #[cfg(test)]
                0 => { std::process::Command::new(\"t\"); }
                1 => { ShellExecuteW(); }
                #[cfg(test)]
                2 => std::process::Command::new(\"u\"),
                _ => { WinHttpOpen(); }
            }
        }
    ";
    assert_eq!(marks(arms), vec!["ShellExecute", "WinHttp"]);
    let labelled = "
        fn g() {
            #[cfg(test)]
            'outer: loop { std::process::Command::new(\"x\"); break 'outer; }
            ShellExecuteW();
        }
    ";
    assert_eq!(marks(labelled), vec!["ShellExecute"]);
}

#[test]
fn windows_network_features_are_found_by_name() {
    for feature in [
        "Win32_Networking",
        "Win32_Networking_WinSock",
        "Win32_NetworkManagement_IpHelper",
        "Networking",
        "Networking_Sockets",
        "Web",
        "Web_Http",
        "Win32_Web_InternetExplorer",
        "Win32_System_Com_Urlmon",
        "Win32_Data_Xml_MsXml",
    ] {
        assert!(is_windows_net_feature(feature), "{feature}");
    }
    for feature in [
        "Win32_UI_Shell",
        "Win32_System_Com",
        "Win32_Foundation",
        "Win32_Graphics_Dxgi",
        "Webbing",
        "Win32_Data_Xml",
    ] {
        assert!(!is_windows_net_feature(feature), "{feature}");
    }
}

#[test]
fn resolved_windows_features_are_checked_not_only_the_declared_ones() {
    let with = |extra: &[&str]| {
        let mut m = meta(
            &good_direct(),
            &[
                ("yolu-app", "windows", None),
                ("yolu-mcp", "windows-sys", None),
            ],
        );
        let nodes = m["resolve"]["nodes"].as_array_mut().unwrap();
        for node in nodes.iter_mut() {
            match node["id"].as_str().unwrap() {
                "windows" => {
                    node["features"] = json!(["Win32_Networking", "Win32_Networking_WinHttp"])
                }
                "windows-sys" => {
                    let mut f = vec![
                        "Win32_Networking",
                        "Win32_Networking_WinSock",
                        "Win32_NetworkManagement",
                        "Win32_NetworkManagement_IpHelper",
                    ];
                    f.extend(extra);
                    node["features"] = json!(f);
                }
                _ => {}
            }
        }
        check_metadata(&m)
    };
    assert_eq!(with(&[]), Vec::<String>::new());
    let found = with(&["Web_Http", "Win32_System_Com_Urlmon"]);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|v| v.contains("windows-sys")));
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

fn check(direct: &[(&str, &str, &[&str])], edges: &[(&str, &str, Option<&str>)]) -> Vec<String> {
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

/// 依存のソースの確かめ用の作り物の `cargo metadata`。`deps` は（名前・yolu-app からの辺の種類。null は本番）。
fn dep_meta(deps: &[(&str, Option<&str>)]) -> serde_json::Value {
    let mut packages = vec![
        json!({ "id": "yolu-app", "name": "yolu-app", "version": "0.0.0", "manifest_path": "/ws/crates/yolu-app/Cargo.toml", "dependencies": [] }),
        json!({ "id": "yolu-cli", "name": "yolu-cli", "version": "0.0.0", "manifest_path": "/ws/crates/yolu-cli/Cargo.toml", "dependencies": [] }),
    ];
    let mut edges = Vec::new();
    for (name, kind) in deps {
        packages.push(json!({ "id": name, "name": name, "version": "1.2.3", "manifest_path": format!("/registry/{name}/Cargo.toml"), "dependencies": [] }));
        edges.push(json!({ "pkg": name, "dep_kinds": [{ "kind": kind }] }));
    }
    let mut nodes = vec![
        json!({ "id": "yolu-app", "deps": edges }),
        json!({ "id": "yolu-cli", "deps": [] }),
    ];
    nodes.extend(
        deps.iter()
            .map(|(name, _)| json!({ "id": name, "deps": [] })),
    );
    json!({
        "workspace_members": ["yolu-app", "yolu-cli"],
        "packages": packages,
        "resolve": { "nodes": nodes },
    })
}

fn fake_sources(
    sources: &'static [(&'static str, &'static str)],
) -> impl Fn(&Path) -> Result<Vec<(String, String)>> + Sync {
    move |dir: &Path| {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        sources
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| vec![("src/lib.rs".to_owned(), (*text).to_owned())])
            .ok_or_else(|| format!("{name} のソースが無い").into())
    }
}

#[test]
fn a_dependency_that_starts_using_sockets_must_be_listed() {
    let meta = dep_meta(&[("tokio", None), ("newcomer", None), ("quiet", None)]);
    let scan = scan_dependency_sources(
        &meta,
        fake_sources(&[
            ("tokio", "use std::net::TcpStream;"),
            ("newcomer", "fn f() { let s = UdpSocket::bind(a); }"),
            ("quiet", "fn f() {}"),
        ]),
    );
    assert_eq!(scan.crates, 3);
    assert_eq!(scan.violations.len(), 1, "{:?}", scan.violations);
    assert!(
        scan.violations[0].contains("newcomer") && scan.violations[0].contains("SOCKET_CRATES")
    );
    let names: Vec<&str> = scan.table.iter().map(|t| t.0.as_str()).collect();
    assert_eq!(names, vec!["newcomer", "tokio"]);
}

#[test]
fn dependency_sources_ignore_tests_strings_and_process_calls() {
    let meta = dep_meta(&[("noisy", None)]);
    let scan = scan_dependency_sources(
        &meta,
        fake_sources(&[(
            "noisy",
            "/// see https://example.com\nconst U: &str = \"https://example.com\";\n\
             fn run() { std::process::Command::new(\"git\"); ShellExecuteW(); }\n\
             #[cfg(test)]\nmod tests { use std::net::TcpStream; }",
        )]),
    );
    assert!(scan.violations.is_empty(), "{:?}", scan.violations);
    assert!(scan.table.is_empty());
}

#[test]
fn only_the_shipped_dependencies_are_read() {
    // 試験だけの依存（dev）は読まない。ビルドスクリプトの依存（build）は読む。windows の宣言の束は機能の側で見る
    let meta = dep_meta(&[
        ("only-for-tests", Some("dev")),
        ("build-helper", Some("build")),
        ("windows-sys", None),
    ]);
    let scan = scan_dependency_sources(
        &meta,
        fake_sources(&[
            ("only-for-tests", "use std::net::TcpStream;"),
            ("build-helper", "use std::net::TcpStream;"),
            ("windows-sys", "use std::net::TcpStream;"),
        ]),
    );
    assert_eq!(scan.crates, 1);
    assert_eq!(scan.violations.len(), 1, "{:?}", scan.violations);
    assert!(scan.violations[0].contains("build-helper"));
}

#[test]
fn an_unreadable_dependency_is_reported_not_skipped() {
    let meta = dep_meta(&[("missing", None)]);
    let scan = scan_dependency_sources(&meta, fake_sources(&[]));
    assert_eq!(scan.crates, 0);
    assert_eq!(scan.violations.len(), 1);
    assert!(scan.violations[0].contains("missing") && scan.violations[0].contains("読めません"));
}

#[test]
fn the_socket_crate_list_is_well_formed() {
    let mut seen = BTreeSet::new();
    for (name, why) in SOCKET_CRATES {
        assert!(seen.insert(*name), "{name} が二重");
        assert!(why.chars().count() >= 8, "{name} に理由が要る");
    }
}
