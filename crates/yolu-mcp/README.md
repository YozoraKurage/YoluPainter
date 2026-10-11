# yolu-mcp

YoluPainter の MCP サーバーの中身と、手元の HTTP の受け口・客です。起動中のアプリ（yolu-app の `mcp_server`）が受け口を動かし、
コマンドライン（yolu-cli）が客と中継に使います。使う人向けの文書は [docs/MCP.md](../../docs/MCP.md) です。

- `tools`: ツールの定義を yolu-ops の命令の一覧から作ります（名前は命令の `.` を `_` に。題は英語と日本語、説明は英語、inputSchema は命令の引数、
  outputSchema は返事の中身（見本の PNG の欄は外す）、注釈は読むだけ・壊す・繰り返してよい・外の世界に触れない）。手で書いた表は持ちません。
- `docs`: `docs/` の使う人向けの文書（開発の手順を除く）を実行ファイルに埋め込み、資料 `yolupainter://docs/<名前>`（`.ja`・`.en` で言語）として返します。
  文書を追加したら、ここへ追加します（試験が追加し忘れを断ります）。
- `server`: rmcp の `ServerHandler`。命令は `Backend`（`fn run(&self, Command) -> 非同期の Result<Reply, OpError>`）へ渡します。返事は structuredContent と同じ JSON の text、
  見本は image と resource_link（直近 8 枚・64 MiB を覚え、`resources/read` で返す）、失敗は `isError` の誤りの JSON。資料は文書・命令の一覧・効果の種類。
  版は rmcp が 2025-11-25 以前の `initialize` と 2026-07-28 の `server/discover` の両方を受けます。
- `http`: 127.0.0.1 だけで待つ受け口（hyper の HTTP/1.1。tokio は `Running` のスレッドの中だけ）。rmcp に渡す前に `Host`（`127.0.0.1:<番号>`・`localhost:<番号>`）と
  `Origin`（あれば同じ 2 つの `http://`）を確かめて 403 で断ります。道は `/mcp` だけ。rmcp は状態を持たない形（`Mcp-Session-Id` を出さない・返事は JSON・GET と DELETE は 405）です。
  同時のつながりは 8 つまで（超えると 503）、本文は 64 MiB まで（413）、要求の頭は 10 秒まで待ちます。止めると新しいつながりを受けず、走っている要求の返事を書き終えてから
  （0.5 秒まで）口を閉じます（止めたあと、同じ番号をすぐに開き直せます）。
- `client`: 受け口への客（`post`・同期の `post_blocking`）、Streamable HTTP の客が付ける頭（`mcp_headers`）、返事の本文（JSON・SSE）からメッセージを取り出す `messages`、
  `tools/call` の結果を命令の返事に戻す `reply_of`。
- `reach`: 起動中のアプリにつなげない・返事が来ない・受け付けをやめたことを言う誤り（`data.live == "unreachable"`。コマンドラインの終了コード 3）。

既定の番号は 17347 です（IANA の登録で割り当てが無く、Linux と Windows の一時の番号の範囲の外）。

## 試験

```sh
cargo test -p yolu-mcp
cargo clippy -p yolu-mcp --all-targets -- -D warnings
```

- `tests/mcp.rs`: 本物の受け口を画面なしのホスト（起動中のアプリの代わり）を相手に立て、本物の客で 2 つの版の流れ・ツールの一覧と schema・呼び出し・誤りの形・見本の画像・資料・
  structuredContent が outputSchema に合うことを確かめます。
- `tests/http.rs`: Host・Origin の断り・道・127.0.0.1 だけ・同時の上限・本文の上限・頭の時間切れ・止めると閉じること。
- `tests/plugin.rs`: 配るプラグイン（`plugin/` と根の `.claude-plugin/marketplace.json`）が、この受け口と合っていること（目録の版がアプリと同じ・MCP の設定が既定の番号を指す・
  Claude Code と Codex の目録が同じスキルと設定を指す・スキルが書くツールの名前・誤りの種類・引数・資料が本当にある）。
