//! 通信と外への出口の印を持ってよいファイルの一覧（[`ALLOWED`]）。

// ───────── 許す一覧 ─────────

/// 印の数の決め方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Count {
    /// ちょうどこの数。増えたら（同じファイルに 2 つ目の出口が入ったら）落ちる。
    Exactly(usize),
    /// 1 つ以上（同じ種類の識別子が幾つも並ぶもの）。
    AtLeastOne,
}
pub(super) use Count::{AtLeastOne, Exactly};

pub(super) struct Allow {
    pub(super) file: &'static str,
    pub(super) mark: &'static str,
    pub(super) count: Count,
    pub(super) why: &'static str,
}

/// 通信と外への出口の印を持ってよいファイル。印ごとに理由と数を持つ。ここに無い印は落ちる。
pub(super) const ALLOWED: &[Allow] = &[
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
        why: "更新の確認: Linux・macOS は curl を起動して https の GET だけをする",
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
        file: "crates/yolu-app/src/update/http.rs",
        mark: "Networking",
        count: Exactly(1),
        why: "更新の確認: Windows の API の WinHTTP を使う宣言（`windows::Win32::Networking::WinHttp`）",
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
