# yolu-cli

`yolupainter-cli`: 命令（[yolu-ops](../yolu-ops/README.md)）を、コマンドラインから `.ylp`（画面なし）か起動中のアプリへ当てます。`yolupainter-cli mcp` は、
標準入出力の MCP のクライアントを、起動中のアプリの MCP の受け口（[yolu-mcp](../yolu-mcp/README.md)）へつなぐ中継です。
使う人向けの文書は [docs/CLI.md](../../docs/CLI.md) と [docs/MCP.md](../../docs/MCP.md) です。ここは作りの説明です。

- 実行ファイルは `yolupainter-cli`（Windows ではコンソールの `yolupainter-cli.exe`）。アプリと同じ配布物・同じ版で配り、Claude Desktop 用の `.mcpb` にも入ります（`cargo xtask mcpb`）。
- `args`: 引数の読み。命令の JSON Schema（yolu-ops の型から作った物）に合わせて、`--名前 値` の型を決めます（文字列の欄の `123` は文字列、数の欄は数、真偽は `--flag`、`.` で入れ子）。
  `--file`・`--live`・`--save`・`--pretty`・`--lang`・`--timeout`・`--out`・`--port`・`--cwd` は CLI の予約名で、命令の欄の名前と重ならないことを試験が確かめます。
- `cli`: 実行と出力。成功は標準出力の JSON、失敗は標準出力の `{"error": ...}` と標準エラーの 1 行、終了コードは 0 成功・1 命令が断った・2 引数の誤り・3 アプリにつなげない・4 確認が要る。
  `--file` は `FileHost` で開いて当て（`--save` で成功したときだけ保存）、`batch` は 1 つの `FileHost` に順に当てて、途中で失敗したら保存しません。
- `live`: 起動中のアプリへ。アプリの受け口（`http://127.0.0.1:<番号>/mcp`。番号は `--port`、既定は 17347）へ MCP の `tools/call` を 1 回送り、結果を命令の返事に戻します
  （`yolu_mcp::client::reply_of`。見本は画像の content を PNG に戻す）。受け口は状態を持たないので `initialize` はしません。呼び出しごとにつなぎ直します。
  つなげない理由は `data.live == "unreachable"` の誤りにします（終了コード 3）。
- `relay`（`mcp`）: 1 行 1 メッセージの JSON-RPC を読み、そのまま `POST /mcp` で送り、返事を 1 行ずつ書きます。ツールの一覧・中身・資料・版の取り決めはアプリが答えるので、
  中継は持ちません（アプリの更新だけで新しくなる）。`initialize` で決まった版を以後の要求の頭（`MCP-Protocol-Version`）に付け、2026-07-28 の流れには `Mcp-Method`・`Mcp-Name` も付けます。
  要求は 4 つまで並べて送ります。アプリにつなげないときは、`tools/call` にはツールの失敗（アプリの誤りと同じ形）、ほかの要求には JSON-RPC の誤りを、直し方の文つきで返します。
- 標準入力は `lock()` を持ち続けない（`LazyStdin`）。tokio の標準入力の読みが同じ錠を使うので、持ち続けると中継の読みが止まります。

## 試験

```sh
cargo test -p yolu-cli
cargo clippy -p yolu-cli --all-targets -- -D warnings
```

- `tests/cli.rs`: 各命令を `.ylp` に当てる・保存・確認・まとめて当てる・見本の画像・`schema` が型と一致すること・終了コードと日英の文・本物の実行ファイル。
- `tests/live.rs`: 起動中のアプリの代わりに、本物の MCP の受け口（`yolu_mcp::http`）を立てて本物の文書に当てる待ち受け（`tests/common` の `FakeApp`）に対する往復・
  断り・見本の画像・HTTP の段で壊れた相手（閉じる・MCP でない・上限・返事が無い）。
- `tests/mcp.rs`: 本物の `yolupainter-cli mcp` を標準入出力で話し、2025-11-25 の `initialize` の流れと 2026-07-28 の `server/discover` の流れがアプリまで届いて戻ること、
  アプリにつなげないときの返事、あとからアプリが待ち始めればつながること、入力が閉じたら終わること。
