# コマンドライン（yolupainter-cli）

[English](en/CLI.md)

`yolupainter-cli` は、YoluPainter の `.ylp` を、画面を出さずに、またはいま起動しているアプリに対して、コマンドから操作する小さなコンソールのプログラムです。
スクリプトや AI から、レイヤー・マスク・効果の読み書き、見本の画像、書き出し、保存を呼べます。AI のアシスタントは、起動中のアプリへ MCP で直接つなぎます
（[MCP の文書](MCP.md)）。標準入出力の MCP しか使えないクライアントのための中継も、同じプログラム（`yolupainter-cli mcp`）にあります。

インストーラーで入れたときは、アプリと同じフォルダ（既定は `%LOCALAPPDATA%\Programs\YoluPainter\yolupainter-cli.exe`）に入ります。zip と tar.gz でも、アプリの隣にあります。Mac の試作の zip では、`YoluPainter.app` の中の `Contents/MacOS/yolupainter-cli` です（[ダウンロードと更新の「Mac（試作）」](INSTALL.md#mac試作)）。
インストーラーは PATH を変えないので、コマンドプロンプトや PowerShell からは、そのフォルダへ移るか、フルパスで呼びます。
アプリと同じ版が入り、更新もいっしょに入れ替わります。

## 使い方

```
yolupainter-cli <命令> [--名前 値 ...] [--file project.ylp [--save]] [--pretty]
yolupainter-cli batch [ファイル|-] --file project.ylp [--save]
yolupainter-cli run-action <アクション.json> [--file project.ylp [--save]]
yolupainter-cli commands            命令の一覧
yolupainter-cli schema [命令]       命令の JSON Schema（--tools は MCP のツールの定義）
yolupainter-cli mcp [--port 番号]   標準入出力の MCP を起動中のアプリへ中継する
```

### 相手を選ぶ

- `--file project.ylp`: アプリを使わずに、その `.ylp` を開いて命令を 1 つ当てます。保存しなければファイルは変わりません。`--save` を付けると、命令が成功したあとにその `.ylp` へ上書き保存します
  （前の版は隣の `<ファイル名>-backups~` フォルダに残ります）。
- `--file` を付けないと、いま起動している YoluPainter が相手です。アプリの設定「外からの操作を受ける」を入れておきます（既定は切です）。
  命令はアプリの中で実行され、1 つの命令が画面の取り消しの 1 段になります。アプリが受けていなければ、直し方を言う誤りを返します（終了コード 3）。
  - アプリとは、この PC の中だけの HTTP（`http://127.0.0.1:17347/mcp`）でつなぎます。アプリの設定で番号を変えたときは、`--port 番号` で同じ番号を指します。
  - 返事を待つのは 60 秒までです（`--timeout 秒` で変えられます）。間に合わなければ、操作が済んだかは分からないので、`doc.info` や `history.info` で確かめてから頼み直します。

### 引数の渡し方

命令の名前は `layer.set` の形です（MCP のツールの名前の `layer_set` でも通ります）。引数は `--名前 値` で渡し、値の型は命令の JSON Schema に合わせて読みます
（文字列の欄に `123` と書いても文字列、数の欄は数）。

```
yolupainter-cli layer.set --file work.ylp --layer Base --opacity 0.5 --blend-mode Multiply --save
yolupainter-cli effect.add --file work.ylp --layer Base --kind blur --values.radius 4
yolupainter-cli layer.add --file work.ylp --kind fill --name Wash --fill.Color "#336699"
yolupainter-cli layer.delete --file work.ylp --layer Tint --confirm
yolupainter-cli export.channels --file work.ylp --dir out --channels Color,Normal
```

- 真偽の欄は `--confirm`（真）・`--visible false`・`--no-confirm`。配列の欄は、繰り返す（`--channels Color --channels Normal`）か、コンマで区切るか、JSON の配列で渡します。
- 入れ子の欄は `.` でたどる（`--values.radius 4`）か、JSON で渡す（`--values '{"radius":4}'`）。
- まるごと JSON で渡すなら、命令の名前のあとに `'{"layer":"Base","opacity":0.5}'`・`@args.json`・`-`（標準入力）を 1 つ置きます。あとに書いた `--名前` が上書きします。
- `--file` の相対パスと、書き出す先（`--dir`・`--path`）の相対パスは、今のフォルダ（`--cwd` で替えられます）からです。`..` で作業のフォルダの外へ出る相対パスは断られます
  （そこへ書きたいときは絶対パスで指します）。

### まとめて当てる（batch）

`.ylp` を開いて閉じるのを繰り返さずに、複数の命令を 1 回の起動で当てます。入力は JSON の配列か、1 行 1 命令（空行と `#` の行は読み飛ばす）です。

```
yolupainter-cli batch commands.jsonl --file work.ylp --save
```

```json
{"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699"}}}
{"command": "layer.set", "args": {"layer": "Wash", "opacity": 0.5}}
{"command": "preview", "args": {"max_edge": 512}}
```

途中で失敗したら、そこで止まり、何も保存しません（誤りの `data.index` が何番目か、`data.completed` がいくつ済んだかを教えます）。
返事は `{"replies": [...], "saved": {...}}` です（`saved` は `--save` のとき）。
batch の中では、前の命令で作ったレイヤー・効果を `$created:<n>` で指せます（[相対の指し方](#相対の指し方)）。

### アクションを当てる（run-action）

アプリの「アクション」のパネルで記録したアクションは、設定のフォルダの `actions/<名前>.json` に置かれます（Windows は
`%APPDATA%\YoluPainter\actions`）。そのファイルを、画面なしの `.ylp` にも、起動中のアプリにも（`--file` が無いとき）当てられます。

```
yolupainter-cli run-action "アクション 1.json" --file work.ylp --save
```

```json
{
  "format": 1,
  "name": "アクション 1",
  "commands": [
    {"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699ff"}, "above": "$selected"}},
    {"command": "effect.add", "args": {"layer": "$created:1", "kind": "blur", "values": {"radius": 4.0}}}
  ]
}
```

- 命令の列を、1 つのテクスチャセットへ**取り消しの 1 段**で当てます。途中の命令が断ったら、そこまでに当てた分も戻して止まり、何も保存しません
  （誤りの `data.index`・`data.command`・`data.completed` は batch と同じ）。
- 入れられる命令は、レイヤー・マスク・効果を変える命令だけです（`layer.add`・`layer.delete`・`layer.move`・`layer.set`・`mask.add`・`mask.delete`・`mask.set`・
  `effect.add`・`effect.set`・`effect.delete`）。読む・見本・書き出し・保存・取り消しの命令が入っていれば、当てる前に断ります。`set` は省くか、全部同じにします。
- 上限: 1 つのアクションの命令は 1,000 個、ファイルは 4 MiB、名前は 100 文字まで。`format` が 1 でないファイルは読みません。
- 書き出し・保存はアクションに入らず、アプリの記録にも残りません（アクションはレイヤー・マスク・効果の編集だけです）。
- 返事は `{"action": "<名前>", "reply": "action", "set": "<セットの ID>", "steps": [{"layer": "..."}, ...], "undo_count": 1, "can_undo": true, "saved": {...}}` です。
  `steps` は命令ごとに、作った・変えたレイヤー（`layer`）と効果（`effect`）、何も変えなかったか（`unchanged`）です。
- 起動中のアプリへは命令 `action.run` の 1 回で送り、アプリの取り消し 1 回で全部戻ります。`$selected` はアプリの今のテクスチャセットで選んでいるレイヤーです。
  `.ylp` には選んでいたレイヤーが入っていないので、`$selected` を使うアクションは画面なしでは断ります。
- 同じ実行は命令 `action.run`（`{"commands": [...]}`。MCP のツール `action_run`）でも呼べます。

### 相対の指し方

記録したアクションを別の文書でも使えるように、レイヤー・効果の欄（`layer`・`above`・`parent`・`effect`）には ID と名前のほかに次の指し方を書けます。

| 書き方 | 指すもの | 使える所 |
|---|---|---|
| `$selected` | 選んでいるレイヤー（レイヤーの欄だけ） | 起動中のアプリ（今のテクスチャセットで選んでいるレイヤー）。アクションと batch では始めた時の 1 つ。画面なしの `.ylp` では断ります |
| `$created:<n>` | 同じ実行の中で n 番目（1 から）に作ったレイヤーか効果（`layer.add`・`effect.add` の順の通し番号） | アクションと batch の中だけ。1 つだけの命令では断ります |

- レイヤーの欄に効果を、効果の欄にレイヤーを指すと断ります。番号の外も断ります（作った数を `data.created` に返します）。
- `$` で始まるほかの文字列は名前として探します（`$created:` で始まるのに番号が読めないものは断ります）。

### 返事と終了コード

返事は標準出力の JSON です（`--pretty` で整形）。見本（`preview`）の PNG は base64 で入ります。`--out 見本.png` を付けると、PNG をそのファイルへ書き、JSON には `png_file` の道を出します。

失敗のときは、標準出力に `{"error": {"code": "...", "message": {"ja": "...", "en": "..."}, "data": {...}}}`、標準エラーに 1 行の文を出します。
文の言語は `--lang ja|en`（環境変数 `YOLUPAINTER_LANG`・`LANG` でも）で選び、決まらなければ日本語と英語を並べます。

| 終了コード | 意味 |
|---:|---|
| 0 | 成功 |
| 1 | 命令が断った（見つからない・値が範囲の外・読むだけのセット・ファイルの失敗など。`error.code` で区別） |
| 2 | 引数の誤り（知らない命令・欄、型の違い、JSON が読めない） |
| 3 | 起動中のアプリにつなげない（アプリが起きていない・設定が切・番号が違う） |
| 4 | 壊す操作に確認（`--confirm`）が無い |

誤りの `code` の一覧は、[命令の仕様](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-ops/README.md)にあります。

## 命令の一覧

「読む」は何も変えません。「編集」は文書を変え、取り消しの 1 段になります。「壊す」は `--confirm` が要ります。「置き換え」は、既にあるファイルを置き換えるときだけ `--confirm` が要ります。
`set` はテクスチャセットの ID か名前で、省くと今のセットです。レイヤーは 32 桁の 16 進の ID か名前で指します（同じ名前が複数あれば、候補の ID を返して断ります）。
[相対の指し方](#相対の指し方)（`$selected`・`$created:<n>`）も使えます。

| 命令 | 引数 | 種類 |
|---|---|---|
| `doc.info` | | 読む |
| `doc.open` | `path`・`confirm` | 置き換え（開いている文書の保存していない変更を捨てるとき） |
| `set.info` | `set` | 読む |
| `layer.get` | `layer` | 読む |
| `layer.add` | `kind`（paint・fill・group・adjustment・text）・`name`・`above`・`fill`・`adjustment`・`channels`・`text` | 編集 |
| `layer.delete` | `layer`・`confirm` | 壊す |
| `layer.move` | `layer`・`parent`・`to_root`・`index` | 編集 |
| `layer.set` | `layer`・`name`・`visible`・`opacity`・`blend_mode`・`clipping`・`locks`・`channels`・`fill`・`adjustment`・`points`（塗りつぶしの点のグラデーション。チャンネル → `space`・`spread`・`points`、`null` で外す）・`text` | 編集 |
| `mask.add` | `layer` | 編集 |
| `mask.delete` | `layer`・`confirm` | 壊す |
| `mask.set` | `layer`・`enabled`・`inverted`・`density` | 編集 |
| `effect.get` | `layer`・`effect` | 読む |
| `effect.add` | `layer`・`target`・`kind`・`values`・`channels`・`strength`・`enabled`・`index` | 編集 |
| `effect.set` | `layer`・`effect`・`kind`・`values`・`channels`・`strength`・`enabled`・`index` | 編集 |
| `effect.delete` | `layer`・`effect`・`confirm` | 壊す |
| `effect.list_kinds` | | 読む |
| `history.info` | `set` | 読む |
| `undo`・`redo` | `set`・`steps` | 編集 |
| `preview` | `set`・`channel`・`max_edge` | 読む |
| `export.channels` | `set`・`channels`・`dir`・`name`・`confirm` | 置き換え |
| `export.textures` | `set`・`template`・`dir`・`name`・`confirm` | 置き換え |
| `export.psd` | `set`・`path`・`channel`・`mode`・`confirm` | 置き換え |
| `save` | `confirm` | 壊す（開いている `.ylp` を上書き） |
| `save_as` | `path`・`confirm` | 置き換え |
| `action.run` | `commands`（[アクション](#アクションを当てるrun-action)の命令の列） | 編集（全部で 1 段。中の壊す命令は、それぞれ `confirm`） |

欄の型・範囲・説明は `yolupainter-cli schema <命令>` で出せます。効果の種類と値の範囲は `effect.list_kinds` が返します。
描く操作（ストローク・塗りつぶし・選択）はまだ命令にありません。

## 安全のこと

- 壊す操作（削除・上書き保存・既にあるファイルの置き換え）は、`--confirm` が無ければ何も変えずに断ります。`--save` は、付けた時点で上書きを頼んだものとして扱います。
- 保存は、検証した一時ファイルからの 1 回の置き換えで確定します。開いたあとに外でファイルが書き換えられていれば上書きせず（`conflict`）、前の版は隣の退避のフォルダに残します。
- 任意のコードを実行する命令はありません。扱うファイルは、`--file` の `.ylp` と、書き出し・保存の命令が指した道だけです。
- アプリへの命令は、この PC の中（`127.0.0.1`）からだけ受けます。合言葉は無いので、設定を入れている間は、この PC のほかのアカウントのプログラムもつなげます（[MCP の文書](MCP.md)の「安全」）。
- 画面なしでは、焼いたメッシュマップやモデルを読む Generator は効きません（設定は残り、見本・書き出し・保存した合成の PNG には入りません。返事の `notes`・`inactive_effects` に出ます）。
  スタンドアロンだけの機能（ノイズ・グランジ・グラデーションマップなど）を使ったセットは、新しい版で保存され、0.4.x までの Unity 版は開けません（[.ylp の形式](YLP_FORMAT.md)の「読み手ごとの範囲」）。保存の返事の `notes` がそのセットを知らせます。
