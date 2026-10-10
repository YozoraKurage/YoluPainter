//! 依存の見張り（`Cargo.lock` の名前・`cargo metadata`）。
use super::{crate_files, markers::MARKERS, scan_sources};
use crate::{cargo, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

// ───────── 依存 ─────────

/// どこにあっても落とす名前（`Cargo.lock`。開発用の依存も含む）。
pub(super) const BANNED_ANYWHERE: &[(&str, &[&str])] = &[
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
pub(super) const BANNED_SHIPPED: &[&str] = &["open", "opener", "webbrowser"];
pub(super) const SHIPPED_ROOTS: &[&str] = &["yolu-app", "yolu-cli"];

/// 低水準の通信の部品と、それに直接依存してよい（配るアプリの依存の中の）クレート。
pub(super) const NET_PARTS: &[(&str, &[&str])] = &[
    ("socket2", &["tokio"]),
    ("mio", &["tokio"]),
    ("hyper", &["hyper-util", "yolu-mcp"]),
    ("hyper-util", &["yolu-mcp"]),
];

/// 直接の依存に持ってよいのが yolu-mcp だけのクレート（127.0.0.1 の受け口と客はここに閉じる）。
pub(super) const NET_ONLY_IN: (&str, &[&str]) =
    ("yolu-mcp", &["hyper", "hyper-util", "socket2", "mio"]);
/// 通信・プロセスの起動に当たる tokio の機能（yolu-mcp だけが `net` を使う）。
pub(super) const TOKIO_NET_FEATURES: &[&str] = &["net", "full", "process"];
/// Windows の API の機能のうち通信に当たる物（WinSock・WinHTTP・WinInet・IpHelper などの `Win32_Networking*`・`Win32_NetworkManagement*`、WinRT の `Networking*`・`Web*`、
/// `Win32_Web*`、URL を取る `Win32_System_Com_Urlmon`、MSXML の `Win32_Data_Xml_MsXml`）。WinHTTP（更新の確認）だけ、yolu-app に許す。
pub(super) fn is_windows_net_feature(feature: &str) -> bool {
    const PARTS: &[&str] = &[
        "Win32_Networking",
        "Win32_NetworkManagement",
        "Networking",
        "Win32_Web",
        "Web",
        "Win32_System_Com_Urlmon",
        "Win32_Data_Xml_MsXml",
    ];
    PARTS.iter().any(|part| {
        feature == *part
            || feature
                .strip_prefix(part)
                .is_some_and(|rest| rest.starts_with('_'))
    })
}
pub(super) const WINDOWS_NET_ALLOWED: &[(&str, &str)] = &[("yolu-app", "Win32_Networking_WinHttp")];
/// 解決した（Cargo が実際に有効にした）windows・windows-sys の通信の機能。tokio・mio・socket2 が足す WinSock・IpHelper は、
/// 127.0.0.1 の受け口と客（yolu-mcp）の分で、直接の宣言では見えない。ここに無い通信の機能が足されたら、確かめを求める。
pub(super) const WINDOWS_RESOLVED_NET: &[(&str, &[&str])] = &[
    ("windows", &["Win32_Networking", "Win32_Networking_WinHttp"]),
    (
        "windows-sys",
        &[
            "Win32_NetworkManagement",
            "Win32_NetworkManagement_IpHelper",
            "Win32_Networking",
            "Win32_Networking_WinSock",
        ],
    ),
];

/// `Cargo.lock` の [[package]] の名前。
pub(super) fn lock_names(lock: &str) -> BTreeSet<String> {
    lock.lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

pub(super) fn check_lock(lock: &str) -> Vec<String> {
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

pub(super) fn string_list(value: &serde_json::Value) -> Vec<&str> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

/// `cargo metadata`（依存も含む）の確かめ。
pub(super) fn check_metadata(meta: &serde_json::Value) -> Vec<String> {
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
                for feature in features.iter().filter(|f| is_windows_net_feature(f)) {
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
    // 解決した windows・windows-sys の通信の機能（tokio・mio が足した物は、宣言には出ない）
    for (id, node) in &nodes {
        let Some(name) = names.get(id) else { continue };
        let Some((_, expected)) = WINDOWS_RESOLVED_NET.iter().find(|(n, _)| n == name) else {
            continue;
        };
        for feature in string_list(&node["features"]) {
            if is_windows_net_feature(feature) && !expected.contains(&feature) {
                out.push(format!(
                    "{name} の機能 {feature}（通信）が有効になりました。Windows の通信は WinHTTP（更新の確認）と、tokio・mio・socket2 の WinSock・IpHelper（127.0.0.1 の受け口）だけです。\
                     新しい依存が足した機能なら、外へ通信しないか確かめ、よければ WINDOWS_RESOLVED_NET に足します。"
                ));
            }
        }
    }
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

/// 配るアプリの依存のうち、ソケット・通信の部品を使うクレート（名前。版は問わない）と、その理由。ここに無いクレートが使い始めたら落ちる。
/// 使わなくなった行は落とさない（依存の更新で変わるため。表には出る）。
pub(super) const SOCKET_CRATES: &[(&str, &str)] = &[
    ("async-io", "zbus の下の非同期の I/O（TCP・UDP・UNIX ソケットの型を持つ。アプリは D-Bus を UNIX ソケットで使う）"),
    ("hermit-abi", "Hermit OS 専用の宣言（Linux・Windows・macOS ではビルドされない）"),
    ("hyper", "127.0.0.1 の受け口と客の HTTP/1.1（yolu-mcp）。HTTP/2（h2）の機能は無効"),
    ("hyper-util", "hyper を tokio の TCP に載せる部品（yolu-mcp の 127.0.0.1 の受け口と客）"),
    ("libc", "OS の C の宣言（ソケットの定数・関数の宣言を含む。呼び出しはその使い手のソースで見る）"),
    ("linux-raw-sys", "Linux のシステムコールの宣言（rustix の下。ソケットの定数を含む）"),
    ("log", "IpAddr などをログの値にする変換だけ（通信しない）"),
    ("mio", "tokio の下のイベントの待ち（TCP・UDP の型。受け口と客の分）"),
    ("ndk-sys", "Android 専用の宣言（Linux・Windows・macOS ではビルドされない）"),
    ("polling", "async-io の下の I/O の待ち（Windows の WinSock の宣言を含む。zbus の D-Bus 用）"),
    ("rmcp", "MCP の SDK。streamable-http のサーバーの部品を使う（待ち受けは yolu-mcp が 127.0.0.1 だけで開く）。reqwest などのクライアントの機能は無効"),
    ("rustix", "システムコールの安全な包み（ソケットの API を含む。zbus・x11rb・wayland-backend・async-io の下）"),
    ("schemars", "IpAddr などの型の JSON Schema だけ（通信しない）"),
    ("serde", "IpAddr などの型の変換だけ（通信しない）"),
    ("serde_core", "IpAddr などの型の変換だけ（通信しない）"),
    ("socket2", "tokio・mio の下のソケットの包み（受け口と客の分）"),
    ("tokio", "非同期の実行。net の機能は yolu-mcp だけが使う（127.0.0.1 の受け口と客）"),
    ("tokio-stream", "tokio・rmcp の下（TCP の流れの型を持つ）"),
    ("tokio-util", "tokio・rmcp・yolu-mcp の下（UDP・TCP の枠組みの型を持つ）"),
    ("uds_windows", "Windows の UNIX ソケット（zbus の D-Bus 用。WinSock の宣言を含む）"),
    ("wasip2", "WASI 専用の宣言（Linux・Windows・macOS ではビルドされない）"),
    ("wayland-backend", "Wayland の接続は UNIX ソケット（rustix::net）。TCP は使わない"),
    ("x11rb", "X11 の接続。DISPLAY が `ホスト名:番号` のときは TCP でも X サーバーへつなぐ（利用者の画面の設定。既定は UNIX ソケット）"),
    ("zbus", "D-Bus（rfd の xdg-portal・accesskit）。既定のセッションバスは UNIX ソケット。TCP の転送の機能は持つが使わない"),
    ("zvariant", "IpAddr などの型の D-Bus の値への変換だけ（通信しない）"),
];

/// 依存のソースで読まない印（文字列は説明に混ざる。プロセスの起動と「開く」は、依存の中では普通にある）。
const NOT_SOCKET_MARKS: &[&str] = &[
    "\"curl\"",
    "\"http://\"",
    "\"https://\"",
    "Command::new",
    "tokio::process",
    "async_process",
    "libc::system",
    "ShellExecute",
    "xdg-open",
    "open::that",
    "open::with",
    "opener",
    "webbrowser",
    "open_url",
    "OpenUrl",
    "hyperlink",
    "hyperlink_to",
];

/// ソースを読まない依存（API の宣言だけを生成した束で、機能の側（`WINDOWS_RESOLVED_NET`）で見る）。
fn is_binding_crate(name: &str) -> bool {
    name.starts_with("windows")
        || name.starts_with("winapi")
        || matches!(name, "web-sys" | "js-sys")
}

#[derive(Debug, Default)]
pub(super) struct DepScan {
    pub(super) violations: Vec<String>,
    /// （クレートの名前・版・見つけた印の数・印の名前）。許す一覧に載っている物も含む。
    pub(super) table: Vec<(String, String, usize, String)>,
    /// ソースを読んだクレートの数（空振りを見抜く）。
    pub(super) crates: usize,
}

/// 配るアプリの依存（本番とビルドスクリプトの閉包。試験だけの依存は含めない）のソースを同じ字句の読み手で読み、ソケットを使うクレートを集める。
/// `read` は（クレートのフォルダ）→（相対の道・中身）。
pub(super) fn scan_dependency_sources(
    meta: &serde_json::Value,
    read: impl Fn(&Path) -> Result<Vec<(String, String)>> + Sync,
) -> DepScan {
    let mut out = DepScan::default();
    let empty = Vec::new();
    let packages = meta["packages"].as_array().unwrap_or(&empty);
    let members: BTreeSet<&str> = string_list(&meta["workspace_members"])
        .into_iter()
        .collect();
    let info: HashMap<&str, (&str, &str, &str)> = packages
        .iter()
        .filter_map(|p| {
            Some((
                p["id"].as_str()?,
                (
                    p["name"].as_str()?,
                    p["version"].as_str().unwrap_or("?"),
                    p["manifest_path"].as_str()?,
                ),
            ))
        })
        .collect();
    let nodes: HashMap<&str, &serde_json::Value> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut closure: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = info
        .iter()
        .filter(|(_, (name, _, _))| SHIPPED_ROOTS.contains(name))
        .map(|(id, _)| *id)
        .collect();
    while let Some(id) = stack.pop() {
        if !closure.insert(id) {
            continue;
        }
        for dep in nodes
            .get(id)
            .map(|n| n["deps"].as_array().unwrap_or(&empty))
            .into_iter()
            .flatten()
        {
            let wanted = dep["dep_kinds"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .any(|k| k["kind"].is_null() || k["kind"] == "build");
            if let (Some(dep_id), true) = (dep["pkg"].as_str(), wanted) {
                stack.push(dep_id);
            }
        }
    }
    let socket_marks: BTreeSet<&'static str> = MARKERS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !NOT_SOCKET_MARKS.contains(name))
        .collect();
    // クレートごとに独立なので、スレッドに分けて読む（約 440 クレート。1 本だと十数秒かかる）
    let targets: Vec<(&str, &str, &Path)> = closure
        .into_iter()
        .filter_map(|id| {
            let &(name, version, manifest) = info.get(id)?;
            if members.contains(id) || is_binding_crate(name) {
                return None;
            }
            Some((name, version, Path::new(manifest).parent()?))
        })
        .collect();
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(16);
    let chunk = targets.len().div_ceil(threads).max(1);
    type Found<'a> = (
        &'a str,
        &'a str,
        std::result::Result<(usize, BTreeSet<&'static str>), String>,
    );
    let found: Vec<Found> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .chunks(chunk)
            .map(|part| {
                let (read, socket_marks) = (&read, &socket_marks);
                scope.spawn(move || {
                    part.iter()
                        .map(|&(name, version, dir)| {
                            let result = match read(dir) {
                                Ok(files) => {
                                    let scan = scan_sources(&files);
                                    let mut marks = BTreeSet::new();
                                    let mut count = 0;
                                    for hit in
                                        scan.hits.iter().filter(|h| socket_marks.contains(h.mark))
                                    {
                                        marks.insert(hit.mark);
                                        count += 1;
                                    }
                                    Ok((count, marks))
                                }
                                Err(error) => Err(format!("{}: {error}", dir.display())),
                            };
                            (name, version, result)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut by_name: BTreeMap<&str, (BTreeSet<&str>, usize, BTreeSet<&str>)> = BTreeMap::new();
    for (name, version, result) in found {
        match result {
            Err(error) => out.violations.push(format!(
                "依存 {name} {version} のソースを読めません（{error}）。取得済みでないと、通信の部品を使うか確かめられません。"
            )),
            Ok((count, marks)) => {
                out.crates += 1;
                if count > 0 {
                    let entry = by_name.entry(name).or_default();
                    entry.0.insert(version);
                    entry.1 += count;
                    entry.2.extend(marks);
                }
            }
        }
    }
    for (name, (versions, count, marks)) in by_name {
        if count == 0 {
            continue;
        }
        out.table.push((
            name.to_owned(),
            versions.into_iter().collect::<Vec<_>>().join("・"),
            count,
            marks.into_iter().collect::<Vec<_>>().join("・"),
        ));
        if !SOCKET_CRATES.iter().any(|(n, _)| *n == name) {
            out.violations.push(format!(
                "配るアプリの依存 {name}（{}）が、ソケット・通信の部品（{}）を使っています。外へ通信する部品でないか確かめ、よければ \
                 crates/xtask/src/netguard/deps.rs の SOCKET_CRATES に、名前と理由を足します。",
                out.table.last().map(|t| t.1.as_str()).unwrap_or(""),
                out.table.last().map(|t| t.3.as_str()).unwrap_or("")
            ));
        }
    }
    out
}

/// 依存のフォルダを読む（本番の `scan_dependency_sources` の引数）。
pub(super) fn read_crate_dir(dir: &Path) -> Result<Vec<(String, String)>> {
    crate_files(dir, dir)
}

/// `cargo metadata --locked`。まずオフラインで読み、取得済みの依存が足りない（ほかの OS だけの依存を取っていない CI など）ときだけ、取得を許して読み直す。
pub(super) fn read_metadata(root: &Path) -> Result<serde_json::Value> {
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
