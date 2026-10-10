//! 依存の見張り（`Cargo.lock` の名前・`cargo metadata`）。
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
/// Windows の API の機能のうち通信に当たる物の頭。WinHTTP（更新の確認）だけ、yolu-app に許す。
pub(super) const WINDOWS_NET_FEATURE_PREFIXES: &[&str] =
    &["Win32_Networking_", "Win32_NetworkManagement_"];
pub(super) const WINDOWS_NET_ALLOWED: &[(&str, &str)] = &[("yolu-app", "Win32_Networking_WinHttp")];

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
