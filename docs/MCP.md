# AI のアシスタントから操作する（MCP）

[English](en/MCP.md)

`yolupainter-cli mcp` は、AI のアシスタント（Claude Desktop・Claude Code・ChatGPT のデスクトップなど）が YoluPainter を操作するための MCP サーバーです。
標準入出力でつなぐ手元のサーバーで、ネットワークには出ません。アシスタントは、`.ylp` のレイヤー・マスク・効果を読み、値を直し、見本の画像を見て、書き出し・保存ができます。
使えるツールは、[コマンドライン](CLI.md)の命令と同じ 25 個（名前の `.` は `_` になります）です。
描く操作（ストローク・塗りつぶし・選択）はまだありません。

相手は 2 通りで、ツールごとの任意の引数 `file` で選びます。

- `file` を付ける: アプリを使わずに、その `.ylp` を開いて操作します。編集はサーバーの中のメモリにあり、`save`（`confirm: true` が要る）を呼ぶまでファイルは変わりません。
  同じ `file` への次の呼び出しは同じ文書を使うので、取り消し（`undo`）もできます。同時に 8 つまで開き、全部が保存していない編集を持つときは、別のファイルを開く前に断ります。
  相対パスの `file` は、サーバーを起こしたフォルダからで、書き出し先などの相対パスは、その `.ylp` のあるフォルダ（最初に開いたときのまま。名前を付けて保存で別のフォルダへ移っても変わりません）からです。
  - 外でファイルが書き換わったとき: 保存していない編集が無ければ、次の呼び出しで開き直して新しい中身を読みます。編集があるときは開き直さず、`save` が `conflict` で断ります。
    その編集を捨てて開き直すには、`doc_open` を `file` と `path` に同じ `.ylp` を指して `confirm: true` で呼びます（取り消しの段も捨てます）。
  - `save_as` のあと: その文書は新しいファイルに移ります。以後は新しい道を `file` に指します。元の道の `file` は、元のファイルを開き直した別の文書です。
    移り先のファイルを別の `file` が保存していない編集つきで開いているときは、書く前に `conflict` で断ります。
- `file` を省く: いま起動している YoluPainter の、開いている文書が相手です。アプリの設定「外からの操作を受ける」を入れておきます。

## つなぎ方

実行ファイル `yolupainter-cli.exe` は、インストーラーで入れたときは `%LOCALAPPDATA%\Programs\YoluPainter\` にあります（zip では `yolupainter.exe` の隣）。以下の `<名前>` は Windows のユーザー名です。

### Claude Desktop（拡張）

[Releases](https://github.com/YozoraKurage/YoluPainter/releases) の `yolupainter-<版>-x86_64-pc-windows-msvc.mcpb` をダウンロードし、ダブルクリックするか、Claude Desktop の設定の拡張機能の画面へドラッグして、
インストールを確かめます。拡張には `yolupainter-cli.exe` が入っているので、YoluPainter 本体を別に入れていなくても、`file` を付けた使い方はできます。アプリを相手にするには、アプリも入れて起動します。
拡張は、アプリの更新では入れ替わりません。新しい版の `.mcpb` を同じ手順で入れ直します。

### Claude Code

```
claude mcp add yolupainter -- C:\Users\<名前>\AppData\Local\Programs\YoluPainter\yolupainter-cli.exe mcp
```

プロジェクトで共有するなら、`.mcp.json` に書きます。

```json
{
  "mcpServers": {
    "yolupainter": {
      "command": "C:\\Users\\<名前>\\AppData\\Local\\Programs\\YoluPainter\\yolupainter-cli.exe",
      "args": ["mcp"]
    }
  }
}
```

### ChatGPT のデスクトップ（Work・Codex）

ローカルの標準入出力のサーバーは、デスクトップの Work と Codex で使えます。`~/.codex/config.toml` に足します（Windows のパスは、`'` で囲むとそのまま書けます）。

```toml
[mcp_servers.yolupainter]
command = 'C:\Users\<名前>\AppData\Local\Programs\YoluPainter\yolupainter-cli.exe'
args = ["mcp"]
```

ChatGPT の Web と claude.ai は、公開した HTTPS のサーバーしかつなげないので、手元のこのサーバーは使えません（いまは対象外です）。
ChatGPT は、MCP の画像の返事を表示できないことがあります。見本は、画像のほかに、リンク（`resources/read` で同じ PNG を読める）でも返します。

## YoluPainter の設定

アプリの設定「外からの操作を受ける」を入れると、アプリが同じ PC の同じユーザーのプログラムからの命令を受けるようになります。既定は切で、入れている間は状態の帯に小さな印が出ます。
切ると、受け付けをやめ、つながっていたものを閉じます。入れていないと、`file` を省いたツールは「つなげない」という誤りを、直し方つきで返します。

アプリは、描いている最中・保存の途中・読むだけのセットなどでは、理由を言って断ります。受けた命令は、アプリの画面の取り消しの 1 段になります。

## ツールと資料

ツールの一覧と、命令ごとの引数・種類は [コマンドラインの文書](CLI.md#命令の一覧)にあります。各ツールには、名前・題・説明・入力と出力の JSON Schema と、
読むだけか・壊すか・同じ引数で繰り返してよいかの注釈が付きます。返事は、structuredContent（出力の JSON Schema どおり）と、同じ内容の text です。
失敗は、`isError` の返事で、`code`・日本語と英語の `message`・`data` を持つ JSON です。

- 見本（`preview`）は、PNG を image として返し、同じ PNG への resource_link（`yolupainter://preview/<番号>.png`。直近の 8 枚まで覚えています）も付けます。
- 資料（resources）: `yolupainter://docs/<名前>` は、入れてある版の文書です（`guide`・`cli`・`mcp`・`install`・`psd`・`brush`・.ylp の形式の仕様 `ylp-format` など。英語があるものは英語が既定で、`<名前>.ja`・`<名前>.en` で言語を選べます）。
  `yolupainter://ops/commands` は命令の一覧と JSON Schema、`yolupainter://ops/effect-kinds` は効果の種類と値の範囲（`effect_list_kinds` と同じ）です。
- プロトコルの版は 2025-11-25 と 2026-07-28 の両方に答えます（`initialize` のある流れと、`server/discover` と要求ごとの `_meta` の流れ）。

## 安全

- 壊す操作（レイヤー・マスク・効果の削除、上書き保存、既にあるファイルの置き換え）は、引数 `confirm: true` が無ければ何も変えずに断り、
  ツールには「壊す」の注釈が付きます。AI は、使う人に確かめてから `confirm: true` を付けます。保存は、前の版を隣の `<ファイル名>-backups~` に残します。
- 任意のコードを実行するツールはありません。扱うファイルは、`file` の `.ylp` と、書き出し・保存のツールが指した道だけです。相対パスで `..` を使って作業のフォルダの外へ出ることはできません。
- アプリへの命令は、同じ PC の同じユーザーのプログラムからだけ受けます。経路は Unix ソケット・Windows の名前付きパイプで、自分だけが読める置き場に鍵のファイルを置き、つなぐたびにその鍵を知っていることを確かめ合います。
  Live Link とは別の名前・別の鍵です。ネットワークの口は開けません。
- MCP のクライアントが、ツールの注釈を信用するかどうかは、クライアント次第です。アシスタントの設定で、壊すツールは呼ぶたびに確かめる設定にしておくと安全です。

## つながらないとき

- 「起動中の YoluPainter につなげません」: アプリが起きていないか、設定「外からの操作を受ける」が切です。入れるか、`file` を付けて画面なしで操作します。
- 「返事がありません」: アプリが描いている最中などで、命令を受けられていません。操作が済んだかは分からないので、`doc_info` や `history_info` で確かめてから、もう一度頼みます。
