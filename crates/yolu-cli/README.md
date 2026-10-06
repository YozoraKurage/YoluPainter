# yolu-cli

`yolupainter-cli`: 命令（[yolu-ops](../yolu-ops/README.md)）を、コマンドラインと MCP サーバー（stdio）から、`.ylp`（画面なし）か起動中のアプリへ当てます。
使う人向けの文書は [docs/CLI.md](../../docs/CLI.md) と [docs/MCP.md](../../docs/MCP.md) です。ここは作りの説明です。

- 実行ファイルは `yolupainter-cli`（Windows ではコンソールの `yolupainter-cli.exe`）。アプリと同じ配布物・同じ版で配り、Claude Desktop 用の `.mcpb` にも入ります（`cargo xtask mcpb`）。
- `args`: 引数の読み。命令の JSON Schema（yolu-ops の型から作った物）に合わせて、`--名前 値` の型を決めます（文字列の欄の `123` は文字列、数の欄は数、真偽は `--flag`、`.` で入れ子）。
  `--file`・`--live`・`--save`・`--pretty`・`--lang`・`--timeout`・`--out`・`--link-name`・`--cwd` は CLI の予約名で、命令の欄の名前と重ならないことを試験が確かめます。
- `cli`: 実行と出力。成功は標準出力の JSON、失敗は標準出力の `{"error": ...}` と標準エラーの 1 行、終了コードは 0 成功・1 命令が断った・2 引数の誤り・3 アプリにつなげない・4 確認が要る。
  `--file` は `FileHost` で開いて当て（`--save` で成功したときだけ保存）、`batch` は 1 つの `FileHost` に順に当てて、途中で失敗したら保存しません。
- `live`: 起動中のアプリへ。経路は yolu-protocol の手元の経路を `yolupainter-ops`（`yolu_ops::link::LINK_NAME`。環境変数 `YOLUPAINTER_OPS_NAME`（`yolu_ops::link::LINK_NAME_ENV`）・`--link-name` で替えられる）で使います。
  つないだ直後の挨拶（鍵のファイルの HMAC の確かめ合い・版の取り決め）は Live Link と同じ `connect_and_greet_within` で、そのあとに yolu-ops の枠（要求 `0x4f50`・返事 `0x4f51`）を流します
  （返事は `ConnectionReader::raw` と `yolu_ops::link::read_frame` で枠のまま受ける。受け口を切ると届く `Bye` は「受け付けをやめた」として返す）。呼び出しごとにつなぎ直し、返事の待ちは別のスレッドで見張ります（Windows の名前付きパイプには読みの時間切れが無いため）。
  つなげない理由は `data.live == "unreachable"` の誤りにします（終了コード 3）。
- `mcp`: rmcp（公式の Rust SDK）の `ServerHandler`。ツールは `tools` が命令の一覧から作り、どのツールにも任意の `file` を足します。
  `file` を付けるとサーバーの中で `session`（`FileHost` の組。上限 8・外で書き換わっていれば編集が無いときだけ開き直す・`save_as` と `doc_open` で別のファイルへ移れば組の道も付け替える）に、省くと `live` へ当てます。
  返事は structuredContent（出力の schema どおり）と text。見本は image と resource_link（直近 8 枚・64 MiB までを覚え、`resources/read` で返す）。資料は `docs`（`docs/` の文書を埋め込み、
  `yolupainter://docs/<名前>` で返す）と、命令の一覧・効果の種類。版は rmcp が 2025-11-25 以前の `initialize` と 2026-07-28 の `server/discover` の両方を受けます。
- 標準入力は `lock()` を持ち続けない（`LazyStdin`）。tokio の標準入力の読みが同じ錠を使うので、持ち続けると MCP サーバーの読みが止まります。

## 試験

```sh
cargo test -p yolu-cli
cargo clippy -p yolu-cli --all-targets -- -D warnings
```

- `tests/cli.rs`: 各命令を `.ylp` に当てる・保存・確認・まとめて当てる・見本の画像・`schema` が型と一致すること・終了コードと日英の文・本物の実行ファイル。
- `tests/live.rs`: 起動中のアプリの代わりに、本物の挨拶と枠で待ち受けて本物の文書に当てる待ち受け（`tests/common` の `FakeApp`）に対する往復・閉じられる・誤った枠・断られる・時間切れ。
- `tests/mcp.rs`: 本物の `yolupainter-cli mcp` を標準入出力で話す。2025-11-25 の `initialize` の流れと 2026-07-28 の `server/discover` の流れ、ツールの一覧・呼び出し・image の返事・資料、
  `file` の組、起動中のアプリへの道、structuredContent が出力の schema に合うこと。
