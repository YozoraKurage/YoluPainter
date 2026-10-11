# AI のアシスタントから操作する（MCP）

[English](en/MCP.md)

YoluPainter は、設定「外からの操作を受ける」を入れている間、MCP のサーバーとして `http://127.0.0.1:17347/mcp` で待ちます。AI のアシスタント（Claude Code・Codex・
Claude Desktop など）はここへつなぎ、アプリでいま開いている文書のレイヤー・マスク・効果を読み、値を直し、見本の画像を見て、書き出し・保存ができます。
ツールはアプリが持つので、アプリを更新すれば新しいツールが使えます。この PC の中だけの通信で、ネットワークには出ません。
使えるツールは、[コマンドライン](CLI.md)の命令と同じ 26 個（名前の `.` は `_` になります）です。描く操作（ストローク・塗りつぶし・選択）・ベイク・別の文書を開くことはできません。

## YoluPainter の設定

1. アプリの「編集 → 設定…」の「Live Link と外からの操作」で「外からの操作を受ける」を入れます（既定は切）。入れている間だけ待ち、状態の帯の右端に小さな丸が出ます。丸のツールチップに、つなぐ先の URL が出ます。
2. 番号は、入れている間その下に出る「ポート番号」で変えられます（既定は 17347。1024〜65535）。変えたときは、つなぐ側の設定の番号も同じにします。
3. 待てないとき（番号をほかのプログラムが使っている など）は、丸が「受けられない」の色になり、ツールチップに理由が出ます。番号を変えるか、そのプログラムを閉じてから、設定を切って入れ直します。

受けた命令は、アプリの画面の取り消しの 1 段になり、画面にそのまま映ります。描いている最中・保存の途中・読むだけのセットなどでは、理由を言って断ります（`busy`・`read_only`）。
切ると、待つのをやめます。返事を待っていた要求には「受け付けをやめた」の誤りを返します（始まっていた保存は最後まで続きます）。

## つなぎ方

### Claude Code（プラグイン）

YoluPainter のプラグインには、アプリへつなぐ MCP の設定と、ツールの使い方（スキル）が入っています。

```
/plugin marketplace add YozoraKurage/YoluPainter
/plugin install yolupainter@yolupainter
```

新しい版のプラグインを自動で受けるには、`/plugin` の Marketplaces で `yolupainter` を選び、自動更新を入れます（ほかの人のマーケットプレイスは、既定では自動更新が切です）。
シェルからなら `claude plugin marketplace add YozoraKurage/YoluPainter`・`claude plugin install yolupainter@yolupainter` です。

プラグインを使わずに、MCP のサーバーだけを追加することもできます。

```
claude mcp add --transport http yolupainter http://127.0.0.1:17347/mcp
```

プラグインの設定は既定の番号を指しています。アプリで番号を変えたときは、この `claude mcp add` で自分の番号のサーバーを追加し、`/mcp` でプラグインの `yolupainter` を切ります。

### Codex（プラグイン）

```
codex plugin marketplace add YozoraKurage/YoluPainter
codex plugin add yolupainter@yolupainter
```

Codex は起動のたびに、追加したマーケットプレイスを新しくし、そこから入れたプラグインも入れ直します（手で新しくするなら `codex plugin marketplace upgrade`）。

プラグインを使わないなら、`~/.codex/config.toml` に追加します（`codex mcp add yolupainter --url http://127.0.0.1:17347/mcp` でも同じです）。

```toml
[mcp_servers.yolupainter]
url = "http://127.0.0.1:17347/mcp"
```

### Claude Desktop（拡張）

Claude Desktop のチャットは、標準入出力の拡張（`.mcpb`）でつなぎます。[Releases](https://github.com/YozoraKurage/YoluPainter/releases) の
`yolupainter-<版>-x86_64-pc-windows-msvc.mcpb` をダウンロードし、ダブルクリックするか、Claude Desktop の設定の拡張機能の画面へドラッグして、インストールを確かめます。
拡張の中身は、標準入出力をアプリの受け口へつなぐ中継（`yolupainter-cli mcp`）だけで、ツールはアプリが答えます。アプリで番号を変えたときは、拡張の設定の「Port」も同じにします。
拡張は、Claude Desktop が始めるときにアプリへ問い合わせます。そのときアプリが受けていなければつながりに失敗するので、アプリを起動して設定を入れてから、拡張を切って入れ直します。

### そのほかのクライアント

Streamable HTTP の MCP を話すクライアントなら、`http://127.0.0.1:17347/mcp` を指せばつなげます。標準入出力しか使えないクライアントは、
`yolupainter-cli mcp`（インストーラーで入れたときは `%LOCALAPPDATA%\Programs\YoluPainter\yolupainter-cli.exe`、Mac の試作の zip では `YoluPainter.app/Contents/MacOS/yolupainter-cli`。番号を変えたなら `--port 番号`）を起動するように設定します。

## ツールと資料

ツールの一覧と、命令ごとの引数・種類は [コマンドラインの文書](CLI.md#命令の一覧)にあります。各ツールには、名前・題・説明・入力と出力の JSON Schema と、
読むだけか・壊すか・同じ引数で繰り返してよいかの注釈が付きます。返事は、structuredContent（出力の JSON Schema どおり）と、同じ内容の text です。
失敗は、`isError` の返事で、`code`・日本語と英語の `message`・`data` を持つ JSON です。

- 見本（`preview`）は、PNG を image として返し、同じ PNG への resource_link（`yolupainter://preview/<番号>.png`。直近の 8 枚まで覚えています）も付けます。
- 資料（resources）: `yolupainter://docs/<名前>` は、入れてある版の文書です（使い方の入り口 `guide` と、その頁 `guide-start`・`guide-paint`・`guide-select`・`guide-layers`・`guide-fill`・`guide-paths`・`guide-3d`・`guide-files`・`guide-settings`・`guide-keys`、`cli`・`mcp`・`install`・`psd`・`brush`・.ylp の形式の仕様 `ylp-format` など。英語があるものは英語が既定で、`<名前>.ja`・`<名前>.en` で言語を選べます）。
  `yolupainter://ops/commands` は命令の一覧と JSON Schema、`yolupainter://ops/effect-kinds` は効果の種類と値の範囲（`effect_list_kinds` と同じ）です。
- `doc_open` は、アプリが開いているファイルを指したときだけ、その文書を返します。別のファイルは `unsupported` で断ります（アプリは文書を開き替えません）。
- レイヤーの欄（`layer`・`above`・`parent`）に `$selected` と書くと、アプリの今のテクスチャセットで選んでいるレイヤーを指します。`$created:<n>` は、まとめて当てる実行
  （コマンドラインの batch・アクション）の中だけで使え、ツールを 1 つ呼ぶときは断ります（[相対の指し方](CLI.md#相対の指し方)）。
- `action_run` は、レイヤー・マスク・効果を変える命令の列（`{"commands": [{"command": "layer.add", "args": {...}}, ...]}`）を、アプリの取り消しの 1 段で当てます。
  途中の命令が断れば全部を戻し、何番目か（`data.index`、0 から）を返します。列の中では `$created:<n>` も使えます。
- プロトコルの版は 2025-11-25 と 2026-07-28 の両方に答えます（`initialize` のある流れと、`server/discover` と要求ごとの `_meta` の流れ）。受け口は状態を持たない形で、
  `Mcp-Session-Id` は使いません。

## 安全

- 壊す操作（レイヤー・マスク・効果の削除、上書き保存、既にあるファイルの置き換え）は、引数 `confirm: true` が無ければ何も変えずに断り、
  ツールには「壊す」の注釈が付きます（`action_run` は、列の中の壊す命令がそれぞれ `confirm: true` を要り、ツールには「壊す」の注釈が付きます）。AI は、使う人に確かめてから `confirm: true` を付けます。保存は、前の版を隣の `<ファイル名>-backups~` に残します。
- 任意のコードを実行するツールはありません。扱うファイルは、書き出し・保存のツールが指した道だけです。相対パスは開いているプロジェクトのフォルダからで、`..` で外へ出ることはできません。
- 待つのは `127.0.0.1`（この PC の中）だけです。要求の `Host` が `127.0.0.1:<番号>`・`localhost:<番号>` でないもの、`Origin` があってそれが同じ所でないもの
  （ほかの名前で 127.0.0.1 を指すウェブページや、ほかのサイトのページからの要求）は断ります。同時につなげるのは 8 つまでです。
- 合言葉はありません。設定を入れている間は、この PC のほかのアカウントのプログラムもつなげます。使わないときは切っておきます。
- MCP のクライアントが、ツールの注釈を信用するかどうかは、クライアント次第です。アシスタントの設定で、壊すツールは呼ぶたびに確かめる設定にしておくと安全です。

## つながらないとき

- 「起動中の YoluPainter につなげません」: アプリが起きていないか、設定「外からの操作を受ける」が切か、番号が違います。アプリの状態の帯の丸のツールチップで、待っている URL を確かめます。
- 丸が「受けられない」の色: ツールチップの理由を見ます。「ほかのプログラムが使っています」なら、アプリの番号を変え、つなぐ側の番号も同じにします。
- 「つながりの数が上限です」（`busy`）: 同時につなげる 8 つが、ほかのクライアントなどで埋まっています。少し待ってから、もう一度頼みます。
- 「返事がありません」: アプリが描いている最中などで、命令を受けられていません。操作が済んだかは分からないので、`doc_info` や `history_info` で確かめてから、もう一度頼みます。
