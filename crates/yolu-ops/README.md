# yolu-ops

外から文書を操作する命令の型と、画面なしで `.ylp` を操作するホストです。

- 命令（`Command`）・返事（`Reply`）・誤り（`OpError`）は serde の JSON で、命令の版を持ち、JSON Schema を型から作れます。
- 文書の命令は `yolu_core::Document` の上の関数（`doc_ops`）1 か所にあり、**1 つの命令が取り消しの 1 段**です。
- プロジェクトの命令（セットの一覧・文書の読み書き・書き出し・保存・見本）は `OpHost` に向けて書いてあります。ホストは、画面なしの
  `FileHost`（この crate）と、起動中のアプリの中のホストです。
- 壊す操作（削除・上書き保存・置き換え）は `confirm: true` が無ければ断ります。任意のコードを実行する命令はありません。

```rust,no_run
use yolu_ops::{execute, parse_command_str, FileHost, OpHost, PathPolicy};

fn main() -> Result<(), yolu_ops::OpError> {
    let mut host = FileHost::new(PathPolicy::current_dir().expect("今のフォルダ"));
    host.open("work/character.ylp".as_ref(), true)?;
    let command = parse_command_str(
        r#"{"command": "layer.set", "args": {"layer": "Base", "opacity": 0.5}}"#,
    )?;
    let reply = execute(&mut host, &command)?; // 取り消しの 1 段になる
    println!("{}", serde_json::to_string(&reply).unwrap());
    execute(&mut host, &parse_command_str(r#"{"command": "save", "args": {"confirm": true}}"#)?)?;
    Ok(())
}
```

## 命令の JSON

命令は `{"v": 1, "command": "<名前>", "args": {...}}` です。`v`（命令の版）と `args` は省けます（今の版・空）。引数の知らない欄は断ります。
返事は `{"reply": "<種類>", ...}`、誤りは `{"code": "<種類>", "message": {"ja": "...", "en": "..."}, "data": {...}}` です。

対象の指し方:

- **セット**（`set`）: ID か名前。省くと今のセット。
- **レイヤー**（`layer`、`above`、`parent`）: 32 桁の 16 進の ID か、名前（同じ名前が複数あれば `ambiguous` で、候補の ID を返します）。
- **チャンネル**: `Color`・`Roughness`・`Metallic`・`Height`・`Normal`・`Emission`、またはユーザーチャンネルの名前（大文字小文字は問いません）。番号でも指せます。
- **効果**: 効果の ID（`effect.add` の返事と `effect.get` に出ます）。
- **色**: `#rrggbb` か `#rrggbbaa`。
- **相対の指し方**（`refs`）: レイヤーの欄の `$selected` はホストが選んでいるレイヤー（`OpHost::selected_layer`。画面なしの `FileHost` は .ylp に選んでいたレイヤーが
  無いので断る）。`$created:<n>` は同じ実行の中で n 番目に作ったレイヤーか効果（`layer.add`・`effect.add` の通し番号）で、まとめて当てる実行（`execute_in` に
  `Created` を渡す・`action::run`・`action.run`）の中だけ。1 つだけの命令（`execute`）では断ります。起動中のアプリへ 1 つずつ送る呼び手は、送る前に `substitute_created` で替えます。

## 命令の一覧

「読む」は何も変えません。「編集」は文書を変え、取り消しの 1 段になります。「壊す」は `confirm: true` が要ります。「置き換え」は、置き換えるときだけ
`confirm: true` が要ります（新しいファイルには要りません）。

| 命令 | 引数（`set` は共通で省略可） | 返事 | 種類 |
|---|---|---|---|
| `doc.info` | なし | `doc` | 読む |
| `doc.open` | `path`、`confirm` | `doc` | 置き換え（開いていた文書の保存していない変更を捨てるとき） |
| `set.info` | `set` | `set` | 読む |
| `layer.get` | `layer` | `layer` | 読む |
| `layer.add` | `kind`（paint・fill・group・adjustment・text）、`name`、`above`、`fill`、`adjustment`、`channels`、`text` | `edited` | 編集 |
| `layer.delete` | `layer`、`confirm` | `edited` | 壊す |
| `layer.move` | `layer`、`parent`、`to_root`、`index` | `edited` | 編集 |
| `layer.set` | `layer`、`name`、`visible`、`opacity`、`blend_mode`、`clipping`、`locks`、`channels`、`fill`、`adjustment`、`points`、`text` | `edited` | 編集（全部で 1 段） |
| `mask.add` | `layer` | `edited` | 編集 |
| `mask.delete` | `layer`、`confirm` | `edited` | 壊す |
| `mask.set` | `layer`、`enabled`、`inverted`、`density` | `edited` | 編集 |
| `effect.get` | `layer`、`effect` | `effects` | 読む |
| `effect.add` | `layer`、`target`（content・mask）、`kind`、`values`、`channels`、`strength`、`enabled`、`index` | `edited` | 編集 |
| `effect.set` | `layer`、`effect`、`kind`、`values`、`channels`、`strength`、`enabled`、`index` | `edited` | 編集（全部で 1 段） |
| `effect.delete` | `layer`、`effect`、`confirm` | `edited` | 壊す |
| `effect.list_kinds` | なし | `kinds` | 読む |
| `history.info` | `set` | `history` | 読む |
| `undo` / `redo` | `set`、`steps`（1〜100） | `undone` | 編集の取り消し・やり直し |
| `preview` | `set`、`channel`、`max_edge`（1〜2048） | `preview` | 読む |
| `export.channels` | `set`、`channels`、`dir`、`name`、`confirm` | `exported` | 置き換え |
| `export.textures` | `set`、`template`、`dir`、`name`、`confirm` | `exported` | 置き換え |
| `export.psd` | `set`、`path`、`channel`、`mode`（bake・flat）、`confirm` | `exported` | 置き換え |
| `save` | `confirm` | `saved` | 壊す（開いている `.ylp` を上書き） |
| `save_as` | `path`、`confirm` | `saved` | 置き換え |
| `action.run` | `commands`（命令の列。[アクション](#アクション)） | `action` | 編集（全部で 1 段。中の壊す命令は、それぞれ `confirm`） |

ツールの名前は、命令の名前の `.` を `_` にしたもの（`layer.set` → `layer_set`。`CommandSpec::tool_name`）です。
欄の型・範囲・説明は `yolu_ops::commands()`（命令の一覧。名前・日英の説明・読むだけか壊すか・引数と返事の JSON Schema）と、`command_schema()`・
`reply_schema()`・`error_schema()` で取れます。壊す印は `Danger`（`Always`・`WhenReplacing`・`PerCommand`）で、確認の欄（`confirm`）を持つ命令
（`Always`・`WhenReplacing`）と持たない命令が、この印と一致することを試験が確かめます。`PerCommand` は `action.run` で、列の中の壊す命令がそれぞれ `confirm` を持ちます。

## アクション

`action` は、記録した命令の列のファイル（`{"format": 1, "name": "...", "commands": [...]}`。命令は上の JSON）と、その列を 1 つのテクスチャセットへ
**取り消しの 1 段**で当てる `action::run` です。命令 `action.run`（`{"commands": [...]}`。MCP のツール `action_run`）も同じ実行で、起動中のアプリにも画面なしの
ホストにも同じに当たります。返事は `action`（`set`・命令ごとの `steps`（作った・変えた `layer`・`effect`、`unchanged`）・`unchanged`・`undo_count`・`can_undo`）。

- 入れられるのは、レイヤー・マスク・効果を変える命令だけ（`ACTION_COMMANDS`・`check_allowed`）。読む・見本・書き出し・保存・取り消し・`action.run` は、当てる前に断ります。壊す命令の `confirm` も先に見ます。
- 全部を `Document::batch` の 1 回で当てます（`doc_ops` の編集は、外がまとめの中ならそのまとめに積む）。途中の命令が断ったら、そこまでの分も戻して、誤りの
  `data` に `index`（0 から）・`command`・`completed` を添えます。
- 上限: 命令 1,000 個（`MAX_COMMANDS`）・ファイル 4 MiB（`MAX_FILE_BYTES`）・名前 100 文字（`MAX_NAME_CHARS`）。`format` が 1 でないファイルは `unsupported_version`。

## 取り消し

編集の命令は `Document::batch` の 1 回で、断られたら積んだ段を戻して文書を元のままにします。取り消しはセッションの中だけです（ファイルには残りません）。
同じ値を渡した編集は何も変えず（`edited.unchanged`）、取り消しの段を増やしません。

## 効果の種類と値

効果は `kind`（種類の名前）と `values`（欄の名前 → 数・真偽・選択肢の文字列）で追加し、変えます。種類・欄の型・範囲・既定は `effect.list_kinds` が返し、
範囲の検査は文書（core）と同じ定義です（範囲の外は切り詰めず断ります）。渡さない欄は、追加するときは既定、変えるときは今の値です。

- フィルター: `blur`・`sharpen`・`noise`・`levels`・`invert`・`normalize`・`color_balance`・`brightness_contrast`・`threshold`・`posterize`・
  `histogram_scan`・`histogram_range`・`slope_blur`・`directional_blur`・`warp`・`morphology`・`edge_detect`・`high_pass`・`median`・`glow`
  （調整レイヤーには `levels`・`invert`・`hue_saturation` などを `layer.add`・`layer.set` の `adjustment` で）。
- Generator: `edge_wear`・`dirt`・`position_gradient`・`thickness`・`direction`・`procedural_noise`・`grunge`・`pattern`・`light`・`mask_builder`・`uv_island_variation`。
- リスト・曲線・参照を持つ種類（`gradient_map`・`tone_curve`・`shape_gradient`・`id_color`・`anchor`・`image`）は、値だけでは追加できません（`addable: false`）。
  すでにある段は `effect.get` で読め、強さ・有効・チャンネルは変えられ、値を渡して変えるのは断ります（その部分を黙って作り直しません）。
- スタンドアロンだけの種類（`rust_only: true`: ノイズ・グランジ・画像（`image`）、グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション、
  0.5.0 のフィルターとジェネレーター（上の一覧の `histogram_scan` から `glow` までと、`pattern`・`light`・`mask_builder`・`uv_island_variation`））を使ったセットは、新しい文書の版で保存され、
  Unity 版（0.4.x まで）は開けません（理由を言って断り、中身は消えません。0.5.0 以降の Unity ブリッジは .ylp を開きません）。保存の返事の `notes` がそのセットを知らせます。
- 効果の種類を変えるとき、適用中のチャンネルに使えない設定は、どのチャンネルかを言って断ります（`channels` で選び直します）。

**画面なしでは効かない効果**: 焼いたメッシュマップ・モデルを読む Generator（`needs_baked_maps: true`。モデルの UV アイランドを読む `uv_island_variation` も）は、画面なしのホストにマップもモデルも無いので、入力のまま通します。
設定は文書に残りますが、見本・書き出し・保存した合成の PNG には入りません（返事の `notes`・`inactive_effects` に出ます）。手続き型のノイズ・グランジは、位置のマップが
無ければ UV で評価するので効きます。塗りつぶしの画像はプロジェクトの画像を渡します。

## 見本の画像

`preview` は、セット・チャンネル・最大の辺から PNG（上の行が先）を返します。画素は文書の正本の合成（書き出しのチャンネルの画像と同じ）で、縮めるときは箱の平均
（不透明度で重み付けた色の平均なので、透明の縁が黒くにじみません）です。拡大はせず、縮めない大きさでは合成の画素そのままです。

## 書き出し

書くのは検証済みの一時ファイルから 1 枚ずつの置き換えで、途中で失敗してももうあるファイルは変わりません。もうあるファイルは `confirm: true` が無ければ置き換えません
（書く前に、置き換えるファイルの一覧つきで断ります）。

- `export.channels`: チャンネルごとの PNG。名前は `<名前>_<チャンネル>.png`（セットが複数のとき `<名前>_<セット>_<チャンネル>.png`）。既定の名前は `.ylp` の名前。
- `export.textures`: テンプレート（`unity-standard`・`unity-hdrp`・`liltoon`）の画像のうち、読むものがある画像だけ。塗り広げ（UV の外）と焼いた AO は、モデルから作るもので、
  画面なしでは渡せないので使いません。
- `export.psd`: 1 つのチャンネルを PSD に（`bake` はレイヤーを残して PSD に形の無いものを焼き、焼いた・丸めた・落としたものを `notes` に出す。`flat` は合成を 1 枚）。
  書いたあと読み戻して確かめます。

## 保存

`FileHost` の保存は yolu-io の安全な保存です: 検証した一時ファイルから 1 回の置き換えで確定し、開いたあとに外で書き換えられたファイルは上書きせず（`conflict`）、
前の版は隣の `<名前>-backups~/` に残します（消しません）。

- 編集したセットだけ文書から作り直し（合成の PNG も）、編集していないセット・読むだけのセット・選択範囲・見た目などの別のエントリは、開いたファイルのバイト列のまま写します。
- 何も編集していなければ書きません（`saved.written: false`）。
- 古い形式のファイルは形式 7 へ上げて書きます（`saved.upgraded_from`）。
- `save_as` は、新しいファイルには確認が要らず、もうあるファイルは有効な `.ylp` に限って `confirm: true` で置き換えます（前の版は退避へ）。名前は `.ylp` で終わる必要があります。
- core で扱えない中身があるセットは**読むだけ**です（`read_only`。`set.info` は大きさと理由だけ）。保存しても、そのセットのバイト列は変わりません。

## ファイルの道

絶対パスはそのまま使います。相対パスは作業のフォルダ（`PathPolicy`。既定は今のフォルダ）からで、`..` で作業のフォルダの外へ出るものは理由つきで断ります
（`path_refused`）。字面だけの検査で、シンボリックリンクは追いません。空・NUL を含む・ドライブだけの相対（`C:foo`）も断ります。

## 誤り

| `code` | 意味 |
|---|---|
| `invalid_request` | JSON が読めない・引数が足りない・知らない欄・型が違う |
| `unknown_command` | 知らない命令の名前（`data.commands` に一覧） |
| `unsupported_version` | 命令の版が合わない（`data.supported`） |
| `no_document` | 開いている文書が無い |
| `not_found` | セット・レイヤー・効果・チャンネル・効果の種類・テンプレートが無い |
| `ambiguous` | 名前が複数に当たる（`data.candidates`。レイヤーは `{id, kind}`・セットは `{id, name}`・チャンネルは `{index, name}` の並び） |
| `invalid_value` | 値が範囲の外・選択肢に無い・組み合わせが断られた |
| `read_only` | 読むだけのセット |
| `unsupported` | そのレイヤー・チャンネル・種類にはできない、この版では扱わない |
| `confirm_required` | 壊す操作に `confirm: true` が無い（`data.files` などに対象） |
| `path_refused` | 道が使えない（作業のフォルダの外・名前の形・通常のファイルでない） |
| `budget` | 予算・上限を超える |
| `refused` | 文書が断った操作（ロック・結合の条件など） |
| `busy` | 今はできない（ストロークの途中など） |
| `conflict` | 保存先が外で変わっている・別の保存が進んでいる |
| `invalid_project` | `.ylp` が壊れている・読めない形 |
| `io` | ファイルの読み書きに失敗した |
| `cancelled` | 取り消した |
| `internal` | 想定していない失敗 |

どの誤りも日本語と英語の両方の文を持ちます（`message.ja`・`message.en`）。断った命令は何も変えません。

## 版

命令の版は `COMMAND_VERSION`（今は 1）です。引数の欄を追加するだけなら上げず、古い命令が読めなくなる変え方をするときに上げます。版の違う命令は `unsupported_version` で断ります。
`.ylp` の形式は変えません（保存は今の形式 7 か、名前を付けて残した選択範囲を使うファイルの 8 のまま）。

## 起動中のアプリへの通信

起動中のアプリへの命令は、アプリの MCP の受け口（`http://127.0.0.1:<番号>/mcp`。yolu-mcp）のツール（`tools/call`）で運びます。この crate は通信を持ちません。
アプリのホストは、受けた命令を画面のスレッドで 1 つずつ実行します（保存は裏で動き、返事は保存の終わりまで遅れます）。相対パスはアプリが開いているプロジェクトの
フォルダからなので、呼び手は絶対パスにして渡してください。

## ホストを作る側へ

`OpHost` を実装すると、同じ命令が動きます。必要なのは、文書を開く（`open`）・セットの文書を読む（`read_set`）・書き換える（`write_set`）・保存する（`save`）です。書き出し（`export`）と
見本（`preview`）は、既定では `read_set` から `export::run`・`preview::render` を通します（起動中のアプリのホストは、アプリの書き出しの設定や合成の道へ向けて置き換えられます）。
`execute` は、壊す操作の確認 → 振り分けを行います。

## 試験

```sh
cargo test -p yolu-ops
cargo clippy -p yolu-ops --all-targets -- -D warnings
```

## 命令にしていないこと

描く（ストローク・塗りつぶし・選択）・ベイク・Live Link・モデルの読み込み・複数の文書を同時に開くこと。画面なしのホストでは、焼いたメッシュマップ・モデルを読む Generator の評価もしません。
