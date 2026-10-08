# .ylp ファイルの形式

YoluPainter の作業ファイル `.ylp` の仕様です。外のツール（変換・検査・ビルドの途中で .ylp を読むもの）を作る人と、形式を変える人のための文書で、
この文書が形式の**正本**です。読み書きのコードは `crates/yolu-io`（外側は `package.rs`・`archive.rs`、中身は `project.rs`、正本は `native.rs`・
`bigdoc.rs`、各エントリはそれぞれの読み手）にあります。コードとこの文書が食い違ったら、それは不具合です。形式を変えるコミットは、同じコミットでこの
文書を直します（`crates/yolu-io/tests/ylp/format_doc.rs` が、書き手のエントリ・版の定数・正本の欄の名前がこの文書に載っているかを確かめます）。

.ylp は作業ファイルで、テクスチャではありません。マテリアルには書き出した PNG を割り当てます（.ylp を配った先に YoluPainter があるとは限らないため）。

## 版の早見

| レイヤー | 版 | 今の書き手が書くもの |
|---|---|---|
| 外側（zip・mimetype・manifest・名前の決まり） | manifest の 1 行目 `YOLUPAINTER-YLP-1`〜`4` | `YLP-3`。今の上限に収まらないファイルだけ `YLP-4` |
| 中身の形式（エントリの並び・`ylp.json`・`project.json`・`resources.json`） | `ylp.json` の `format`、1〜8 | 7。名前を付けて残した選択範囲を使うファイルだけ 8 |
| テクスチャセットの正本 `document.utpaint` | `DOTPAINT` の後の版、1〜30・32・33（31 は欠番） | 使う機能で決まる 21〜25・27〜30・32・33。512 MiB を超える正本だけ 26（中の版は 21〜25・27〜30・32・33） |
| `meshmap-<種類>.bin` | `YLPMMAP` の後の版 1〜3、エンジンの版 2 | 3 |
| `selection.bin`・`selection-<印>.bin` | `YLSL` の後の版 1 | 1 |
| `look.json`・`pose.json`・`selections.json`・`livelink.json` | 各 JSON の `format` 1 | 1 |
| `.ylsmart`（アセットのスマートマテリアル・スマートマスク） | manifest `YOLUPAINTER-SMART-1`、`smart.json` の `format` 1 | 1 |
| `.ylbrush`（アセットの携帯ブラシ） | manifest `YOLUPAINTER-BRUSH-1`、`state.json` の `schema` 1〜3 | 書かない（読んでバイト列のまま残す） |

コードの定数: `project::MAX_FORMAT`（8）・`SAVED_SELECTIONS_FORMAT`（8）、`native::UNITY_NATIVE_VERSION`（21）・`USER_CHANNELS_VERSION`（22）・
`PROCEDURAL_VERSION`（23）・`ADJUST_VERSION`（24）・`MIXING_VERSION`（25）・`SPLIT_VERSION`（26）・`PATHS_VERSION`（27）・`EFFECTS_VERSION`（28）・`POINT_GRADIENT_VERSION`（29）・`TEXT_VERSION`（30）・`SEAMS_VERSION`（32）・`BAKE_PRIORITY_VERSION`（33）・`MAX_NATIVE_VERSION`（33）、`mesh_map::FORMAT_VERSION`（3）、
`look::FORMAT`・`pose::FORMAT`・`saved_selections::FORMAT`・`livelink::FORMAT`（どれも 1）。

### 読み手ごとの範囲

表と、この文書の以下で単に「Unity 版」と書くのは、Unity のパッケージの 0.4.x までです。Unity ブリッジの 0.5.0 以降は Live Link とマテリアルへの適用だけで、.ylp を開かず、書きません
（Assets に置いた .ylp は、ファイルの中身も GUID も変えないまま、Unity では普通のファイル（DefaultAsset）になります）。

| 読み手 | 外側 | 中身の形式 | 正本の版 | 知らないエントリ |
|---|---|---|---|---|
| このアプリ（0.5.0〜） | `YLP-1`〜`4` | 1〜8 | 1〜30・32・33 | 知らせて、バイト列のまま残す |
| スタンドアロン 0.4.x | `YLP-1`〜`4` | 1〜8 | 1〜26（27〜30・32・33 は「`.version の値 27 は未対応または範囲外です (1..25)`」のように版の値だけが違う文で、分けた正本の中の版 27〜30・32・33 は「`分けた正本の中の版 27 は未対応です`」のように断る） | 知らせて、バイト列のまま残す |
| スタンドアロン 0.3.0〜0.3.2 | `YLP-1`〜`4` | 1〜7（8 は書いたアプリを添えて断る） | 1〜26 | 知らせて、バイト列のまま残す（`pose.json` もこの扱い） |
| Unity 版 0.4.x まで（Unity のパッケージの EditorWindow とインポーター。0.2.0 から新しい形式を追加していない） | `YLP-1`〜`3`（`YLP-4` は断る） | 1〜7（8 は断る） | 1〜21（22 以上は `Unsupported archive version; source retained unchanged.`） | 知らせて、保存で落とす |

どの読み手も、読めないものは**ファイルに触れずに**断ります（一部だけを読んで保存し直し、黙って何かを失うことはありません）。1 つのセットでも正本の版が
22 以上なら、その .ylp は Unity 版（0.4.x まで）では開けません（Unity 版で開くには、その機能を消して保存し直し、版 21 に戻します）。

手動の ID の色の塊（版 19 から）は、Unity 版（書き手でもあります）と、このアプリの 0.3.0 以降が読めます。0.4.x までのアプリは、塊のあるセットを読むだけで
開き（理由を出し、元のバイト列のまま保存する）、色は編集できません。0.5.0 からは、色を文書へ戻して編集でき、保存でも残ります。

## 外側

- zip。先頭は無圧縮の `mimetype`（中身は `application/x-yolupainter`）、次に `manifest.sha256`、続いてエントリを名前の順に置きます。PNG は無圧縮、
  ほかは Deflate で入れます。zip の日時と圧縮の結果は形式の一部ではありません（同じ中身でもバイト列が同じとは限りません）。
- `manifest.sha256` は UTF-8 のテキスト。1 行目が外側の版、続く各行が `<SHA-256 の小文字の 16 進 64 桁> <バイト数> <名前>`（名前の順）。中身の正しさは
  SHA-256 で決め、zip の CRC-32 も確かめます。manifest に無いエントリ・manifest にあってファイルに無いエントリ・長さや SHA-256 の違いは、どれも読まずに
  断ります。
- エントリの名前: 英数字と `. - _`、96 バイトまで。`.` で始まる名前・`..` を含む名前・空の名前は断ります。置けるフォルダは外側の版で決まります。
  - `YOLUPAINTER-YLP-1`（中身の形式 1・2）: 根と `composite/` の下だけ。根に `document.utpaint` が要ります。
  - `YOLUPAINTER-YLP-2`（中身の形式 3）: 加えて、名前の前に `sets/<ID>/` を 1 つ置けます（ID は小文字のハイフン付きの GUID 36 文字。大文字・ハイフン
    無し・空の GUID は断る）。`sets/<ID>/composite/` も置けます。`document.utpaint` は根かどれかの `sets/<ID>/` に要ります。
  - `YOLUPAINTER-YLP-3`（中身の形式 4 から）: 加えて、根の `resources/` の下に 1 段の名前を置けます（その下にフォルダは置けない。`sets/<ID>/resources/` も
    置けない）。
  - `YOLUPAINTER-YLP-4`: 名前の決まりは `YLP-3` と同じで、上限と zip64 だけが違います（下）。
  - 名前の決まりを広げるときは外側の版を上げます。古い読み手は、名前で断る代わりに「新しい YoluPainter で書かれた」と断ります。
- `YLP-3` までの上限: 1 エントリ 512 MiB、合計 768 MiB、1000 エントリ、manifest 1 MiB、zip64 なし。宣言した長さを超えて展開しません。
- `YOLUPAINTER-YLP-4`（`package.rs`）: `YLP-3` の上限に収まらないファイルだけ。収まるプロジェクトは `YLP-3` で書き、バイト列も `YLP-4` を追加する前と同じです。
  - 1 エントリは、正本の部分（`document.utpaint.<n>`）が 256 MiB、ほかが 512 MiB まで。エントリは 65000 まで、manifest は 16 MiB まで。
  - セットごとの正本（`document.utpaint` と部分の長さの合計）は、読み手の設定「レイヤーのメモリ」の予算（256 MiB を下回らない）の 4 倍まで。全体は
    「正本のあるセットの数 × それ ＋ 768 MiB」まで。この 2 つは読み手の設定で、形式の制約ではありません。書き手は予算を超えても書き、今の予算で開き直せない
    ことを保存のときに知らせます。読み手は、どの予算かだけを言って展開の前に断ります。
  - zip64 は要るときだけ使います。4 GiB を超える位置のローカルヘッダーは中央ディレクトリの zip64 の拡張で指し（その記録の版は 45）、65535 を超える
    エントリか 4 GiB を超える中央ディレクトリは zip64 の終端の記録と目印を書きます。`YLP-3` までのファイルに zip64 の構造があれば断ります。データ記述子は
    書きませんが、読み手は受けます。

## エントリ

種類の意味:

- **正本**: 失うと作業を失うもの。読めなければ開くのを断ります（Unity 版は `selection.bin` だけ、理由を知らせて無しで開く）。
- **状態**: 開いた後の画面の状態。読めなければ既定に戻して知らせ、エントリはバイト列のまま残します（書き換えるまで）。
- **派生**: 正本から作り直せるもの。読めなければ使わずに作り直します。正本の代わりにはしません。

根（コードの一覧は `project::ROOT_ENTRIES`）:

| エントリ | 種類 | 形式 | 中身 | このアプリ |
|---|---|---|---|---|
| `ylp.json` | 記録 | 2 から | 中身の形式と書いたアプリ（下） | 書く |
| `project.json` | 正本 | 3 から | テクスチャセットの並びと今のセット（下） | 書く |
| `resources.json` | 正本 | 4 から | アセット（プロジェクトのリソース）の並び。アセットが空なら書かない（下） | 書く |
| `resources/<content>.png`・`.ylsmart`・`.ylbrush` | 正本 | 4・5・6 から | アセットの中身（下） | 書く |
| `view.json` | 状態 | | 画面の状態（下） | `standaloneModel` だけを書き、ほかのキーは残す |
| `pose.json` | 状態 | 7 から（形式は上げない） | モデルの今のポーズ（下） | 書く |
| `brush.json` | 状態 | | Unity 版のブラシの設定（`schema` 1〜3） | 書かない（残す） |
| `thumbnail.png` | 派生 | | Unity 版のインポーターが見る見本（長辺 256 px 以下） | 書かない（残す） |
| `model.json` | 状態 | | 古い書き手の状態。今の書き手は書かない | 書かない（残す） |
| `livelink.json` | 状態 | 7 から（形式は上げない） | Live Link の相手の文書（FBX の並び・レンダラーとマテリアルの結び・ポーズ・相手。下） | 書く |

テクスチャセットごと（`sets/<ID>/` の下。形式 2 までは同じ名前で根にあった。コードの一覧は `project::SET_ENTRIES`）:

| エントリ | 種類 | 形式 | 中身 | このアプリ |
|---|---|---|---|---|
| `document.utpaint` | 正本 | 1 から | セットの正本（下の「正本」） | 書く |
| `document.utpaint.<n>` | 正本 | 7 から（形式は上げない） | 正本の版 26 の部分（`<n>` は 1 から続く 10 進） | 512 MiB を超える正本だけ書く |
| `selection.bin` | 正本 | 1 から | 今の選択範囲（下） | 書く |
| `selections.json` | 正本 | 8 | 名前を付けて残した選択範囲の索引（下） | 書く |
| `selection-<印>.bin` | 正本 | 8 | 残した選択範囲の中身（下） | 書く |
| `look.json` | 状態 | 7 から（形式は上げない） | 3D ビューの見た目の設定（下） | 書く |
| `composite/<チャンネル>.png` | 派生 | 1 から | 標準のチャンネルの合成（下） | 書く |
| `meshmap-<種類>.bin` | 派生 | 1 から | 焼いたメッシュマップ（下） | 書く |
| `imported-original.psd` | 正本 | 2 から | 取り込んだ PSD の原本のバイト列（変えない） | 書かない（残す） |

知らないエントリ（今の形式のどの読み手も知らない名前、並びに無いセットの `sets/<ID>/`）は、開くときに一覧で知らせます。このアプリはバイト列のまま
残し、Unity 版は保存で落とします。

## 中身の形式の版

| 形式 | 変えたこと | 移行（古い形式を開くとき。メモリの上だけで、ファイルは書き換えない） |
|---|---|---|
| 1 | （`ylp.json` の無いファイル） | — |
| 2 | `ylp.json` を追加した。並びは 1 と同じ | なにもしない |
| 3 | テクスチャセット。根に `project.json`、正本・選択範囲・取り込んだ PSD・合成・メッシュマップはセットごとに `sets/<ID>/` の下。manifest は `YLP-2` | 根の `document.utpaint`・`selection.bin`・`imported-original.psd`・`composite/*`・`meshmap-*.bin` を `sets/<文書の ID>/` へ動かし（ID は正本の頭の文書の ID）、`view.json` の `materialSlot`（無い・読めなければ 0）から 1 つのセットの `project.json` を作る。名前は仮の `Texture Set 1`。ほかのエントリは根に残す |
| 4 | アセット（`resources.json` と `resources/<content>.png`）。manifest は `YLP-3` | なにもしない |
| 5 | アセットの種類にスマートマテリアル・スマートマスク（`resources/<SHA-256>.ylsmart`） | なにもしない |
| 6 | アセットの種類にブラシ（`.ylbrush`）・マテリアル（`.ylsmart`）、Unity のアセットの `localFileID`、置き場の下位の階層 | なにもしない |
| 7 | テクスチャセットはマテリアルごと。`project.json` の各セットの `materialSlot` をやめ、`material`（マテリアルの鍵）にした。メッシュマップの版 3 | 各セットの `materialSlot` を `"material": { "slot": <番号> }` にする |
| 8 | 名前を付けて残した選択範囲（`sets/<ID>/selections.json` と `selection-<印>.bin`）。名前の決まりは変わらないので manifest は `YLP-3` のまま。**使うファイルだけ**が 8 で、全部なくなれば 7 に戻る | なにもしない |

- 書くときは今の形式で書きます。古い形式のファイルを開いて保存すると形式 7（残した選択範囲があれば 8）になり、古い読み手では開けなくなります。
  直前の版は退避に残ります（下の「保存」）。
- 新しすぎる形式は、どのエントリにも触れずに、形式の番号と書いたアプリを添えて断ります。

## ylp.json（形式 2 から）

```json
{
  "format": 7,
  "savedBy": { "app": "YoluPainter", "version": "0.4.0", "unity": "standalone" },
  "createdBy": { "app": "YoluPainter", "version": "0.4.0", "unity": "standalone" }
}
```

- `format`（整数、2 以上、必須）: 中身の形式。
- `savedBy`（必須）: 最後に保存したアプリ。`app`・`version`・`unity`（どれも 1〜256 文字）。Unity 版は `unity` に Unity の版を、このアプリは `standalone` を書きます。
- `createdBy`（省略可）: 最初に作ったアプリ。形式 1 のファイルから作ったものは、分からないので書きません。
- 知らないキーは読み飛ばします。

正本・記録の JSON（`ylp.json`・`project.json`・`resources.json`、`.ylsmart` の `smart.json`）に共通の決まり: UTF-8 の JSON のオブジェクト。キーの重複・深さ 16 を
超える入れ子・1024 文字（UTF-16）を超える文字列とキーは断ります。`ylp.json`・`project.json` は 64 KiB、`resources.json` は 1 MiB まで。

## project.json（形式 3 から。この形は形式 7）

```json
{
  "sets": [
    { "id": "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0", "name": "Body", "material": { "name": "Skin", "guid": "0123456789abcdef0123456789abcdef", "fileId": 2100000 } },
    { "id": "11111111-2222-3333-4444-555555555555", "name": "Hair", "material": { "name": "Hair" } },
    { "id": "22222222-3333-4444-5555-666666666666", "name": "Unassigned", "material": { "unassigned": true } },
    { "id": "33333333-4444-5555-6666-777777777777", "name": "Texture Set 1", "material": { "slot": 0 } }
  ],
  "current": "11111111-2222-3333-4444-555555555555"
}
```

- `sets`（必須、1〜64 個、並びの順がパネルの順）:
  - `id`（必須）: 小文字のハイフン付きの GUID。エントリの置き場 `sets/<id>/` の名前。
  - `name`（必須）: 1〜256 文字、空白だけでない、制御文字なし。大文字小文字を区別せずにほかのセットと重ならない（書き出すファイルの名前に使う）。
  - `material`（必須、形式 7 から）: 次のどれか 1 つの形（2 つ以上・どれも無いものは断る）。
    - `name`（0〜256 文字、制御文字なし）と、Unity のアセットなら `guid`（小文字の 16 進 32 文字）と `fileId`（符号付き 64 bit の整数）。`guid` と `fileId` は揃えて書く。
    - `"unassigned": true`: モデルのマテリアルの無いスロットの全部。
    - `slot`（0〜65535）: まだマテリアルに結び付けていないスロットの番号（形式 6 までのファイル）。
    - 同じ `guid`・`fileId` の 2 つ、2 つの `unassigned`、同じ `slot` の 2 つは断る。名前だけの鍵は重なってよい。
    - モデルのマテリアルへの照合は、識別子 → `unassigned` → 名前（大文字小文字まで同じ、次に区別せず）→ `slot` の順で、1 つのマテリアルは 1 つのセットだけが
      持つ。合わないセットも開けます（3D に見えないだけ）。
  - `materialSlot`: 形式 6 まで。形式 7 では読みません（移行が `material` の `slot` にする）。
- `current`（必須）: 今のセットの `id`（並びにあること）。
- 知らないキー（根・各セット）は読み飛ばし、書き直すときも残します。決まりに合わないものは開くのを断ります（既定に戻して開かない）。
- 並びにあるセットに `sets/<id>/document.utpaint` が無ければ断ります。

## アセット（resources.json と resources/。形式 4 から）

```json
{
  "resources": [
    { "id": "aaaaaaaa-0000-4000-8000-000000000001", "kind": "image", "name": "Scratches", "content": "<64 桁の小文字 16 進>", "width": 1024, "height": 1024, "colorSpace": "srgb",
      "origin": { "type": "unityAsset", "guid": "0123456789abcdef0123456789abcdef", "path": "Assets/Textures/Scratches.png", "stamp": "<Unity の依存ハッシュ>", "localFileID": 2800000 } },
    { "id": "bbbbbbbb-0000-4000-8000-000000000002", "kind": "smartMaterial", "name": "Rusty", "content": "<ファイルの SHA-256>", "length": 5519,
      "origin": { "type": "library", "file": "Rusty.ylsmart", "sha256": "<同じ>", "length": 5519 } }
  ]
}
```

- `resources`（必須、0〜256 個、並びの順がパネルの順）。アセットが空なら `resources.json` を書きません（無いファイルはアセットが空）。
  - `id`（必須）: 小文字のハイフン付きの GUID。正本のレイヤー（塗りつぶしの画像の `resource_id`）と `look.json` の `image` が指す名前。重ならない。
  - `kind`（必須）: `image`、形式 5 から `smartMaterial`・`smartMask`、形式 6 から `brush`・`material`。知らない種類は断る（落として保存しない）。
  - `name`（必須）: 1〜256 文字、空白だけでない、制御文字なし。重なってよい。
  - `content`（必須、64 桁の小文字 16 進）:
    - `image`: ASCII の `YLPRGBA8`、幅と高さ（32 bit の little-endian）、straight RGBA8 の画素（下の行から）を続けた SHA-256。画素は `resources/<content>.png`。
    - ほか: ファイルのバイト列の SHA-256。ファイルは `resources/<content>.ylsmart`（`smartMaterial`・`smartMask`・`material`）か `resources/<content>.ylbrush`（`brush`）。
  - `image` だけ: `width`・`height`（必須、1〜8192。PNG と同じでなければ断る）、`colorSpace`（省略可。`srgb`・`linear`・`unspecified`、既定 `unspecified`）。
  - `image` 以外: `length`（必須、1〜512 MiB。ファイルの長さと同じ）。
  - `origin`（省略可、既定は `{ "type": "none" }`）: 出どころ。写しは出どころが消えても使えます。
    - `none`: 写しだけ。
    - `unityAsset`: `guid`（32 桁の小文字 16 進）、`path`（1〜1024 文字。見せるだけ）、`stamp`（省略可、0〜128 文字。Unity の依存ハッシュ）、`localFileID`
      （省略可、符号付き 64 bit。無い・0 は主アセット）、`readThroughGpu`（省略可）。
    - `file`: `path`（絶対パス、1〜1024 文字）、`sha256`、`length`。
    - `library`: `file`（置き場の中の `/` 区切りの相対パス。`\`・`:`・空の区間・`.`・`..` は断る）、`sha256`、`length`。
    - `builtIn`: `key`（1〜64 文字の `a-z 0-9 -`）、`version`（1 以上）。
  - 2 つのリソースが同じ中身を持ってよく、そのときエントリは 1 つです。
- `resources/<content>.png`: 色の型 6（RGBA）・8 bit・インターレース無し・補助のチャンクなし（ICC・gAMA を付けない）。透明画素の RGB も保ちます。開くときに全部を
  復号し、画素のハッシュが名前と、大きさが並びと同じかを確かめます。保存は読んだバイト列をそのまま書きます。
- `resources/<content>.ylsmart`・`.ylbrush`: 下の「.ylsmart」「.ylbrush」のファイルを、置き場のファイルと同じバイト列で入れます。開くときに長さ・SHA-256・中身の種類を
  確かめます。
- 並びにあるのにエントリが無い・壊れているものは、どれかを添えて開くのを断ります。並びに無い `resources/` のエントリは知らないエントリとして知らせます。
- メモリの予算: 復号した画像の画素・スマートマテリアルのファイルと画素・ブラシのファイルと筆先を合わせて 768 MiB まで。

## view.json（状態）

画面の状態。256 KiB まで、オブジェクトでなければ読みません。知らないキーは残します。

| キー | 書き手 | 中身 |
|---|---|---|
| `modelAssetGuid` | Unity 版 | モデルのアセットの GUID（このアプリが新しく作るときは空の文字列） |
| `selectedChannel` | Unity 版 | 選んだチャンネル（このアプリが新しく作るときは 0） |
| `visibility` | Unity 版 | `hiddenSets`（非表示のセットの ID）と `hiddenRenderers`（モデルの中の兄弟番号の道とレンダラーの番号。例 `000001/000000:0`）。上限はセット 64・レンダラー 4096・鍵 2048 文字 |
| `standaloneModel` | このアプリ | `{ "path": <モデルのファイル（FBX など）の .ylp からの相対か絶対の道、1〜1024 文字（UTF-16）、制御文字なし> }` |
| `materialSlot` | 形式 2 まで | 形式 3 への移行が読む。今は書かない |

## pose.json（状態。根）

モデルの今のポーズ。8 MiB まで。休みの形からの差だけを持ち、骨は名前の道、BlendShape はメッシュの名前と BlendShape の名前の組で持つので、名前の組が
同じモデルへ戻せます（合わないものは戻すときに理由つきで飛ばす）。ポーズのプリセット（`.ylpose`）と同じ表し方です。

```json
{
  "format": 1,
  "bones": [ { "path": ["Hips", "Spine"], "translation": [0.0, 0.01, 0.0], "rotation": [0.0, 0.0, 0.1, 0.99], "scale": [1.0, 1.0, 1.0] } ],
  "shapes": [ { "mesh": "Face", "name": "Smile", "weight": 40.0 } ],
  "take": { "name": "Take 001", "frame": 12 }
}
```

- `format`: 1。ほかは読まずに断ります。
- `bones`（4096 個まで）: `path`（根から骨までの名前の並び、1〜256 段、名前は 1〜256 文字）、`translation`（親の空間での足し算の差、絶対値 1e6 まで）、
  `rotation`（骨のローカルの差、単位クォータニオン x・y・z・w。長さの許す幅 0.01、読んだ後で正規化。今の回転 = 休みの回転 × 差）、`scale`（休みの大きさとの比、
  絶対値 1e6 まで）。
- `shapes`（4096 個まで）: `mesh`・`name`（1〜256 文字）、`weight`（Unity と同じ 0〜100 の目盛り、絶対値 1e4 まで）。
- `take`（無くてよい）: ポーズの欄で選んでいるモデルのテイク（FBX の中のアニメのスタック）の `name`（1〜256 文字）と `frame`（整数、絶対値 1e9 まで）。
  開いたときに欄の選びを戻すだけで、ポーズの値は `bones`・`shapes` が持ちます（テイクがモデルに無ければ、欄は最初のテイクのまま）。0.5.0 から書き、
  0.4.x のスタンドアロンは知らないキーとして読み飛ばします（保存し直すと落ちる。失うのは欄の選びだけ）。
- 範囲の外・数でない値・単位でない回転・骨や BlendShape の重なり・数の上限を超えたものは、エントリごと断ります（一部だけを読まない）。断ったエントリは
  バイト列のまま残ります。
- 形式も正本の版も上げない状態のエントリです。Unity 版は知らないエントリとして知らせて保存で落とし（失うのはポーズだけ）、スタンドアロン 0.3.x はバイト列のまま
  残します。

## look.json（状態。セットの下）

テクスチャセットの 3D ビューの描き方（`standard` の PBR か `lilToon` の再現か）と、lilToon のときのマテリアルの値。1 MiB まで。標準の見た目で何も設定していない
セットはエントリを書きません。

```json
{
  "format": 1,
  "kind": "lilToon",
  "shader": "Hidden/lilToonCutoutOutline",
  "properties": {
    "_ShadowBorder": { "float": 0.5 },
    "_UseShadow": { "int": 1 },
    "_ShadowColor": { "color": [0.82, 0.76, 0.85, 1.0] },
    "_MainTex_ST": { "vector": [1.0, 1.0, 0.0, 0.0] }
  },
  "textures": {
    "_MainTex": { "channel": 0 },
    "_ShadowStrengthMask": { "packed": [ { "channel": 7, "component": 0 }, { "channel": 7, "component": 0 }, "zero", "one" ] },
    "_MatCapTex": { "image": "<アセットの画像の id からハイフンを除いた 32 桁の 16 進>" }
  },
  "keywords": [],
  "kindChosen": true,
  "received": {
    "kind": "lilToon",
    "shader": "Hidden/lilToonTransparent",
    "source": "lilToon 2.3.4 · Standard/Transparent",
    "properties": {},
    "textures": { "_MainTex": { "channel": 0 } },
    "keywords": [],
    "missing": { "_MatCapTex": "pending", "_ShadowColorTex": "overBudget" }
  }
}
```

- `format`: 1。2 以上・知らない `kind` は読まずに断り、エントリはそのまま残ります（標準の見た目で開き、見た目を変えずに保存すれば残す。見た目を変えて保存すると
  今の設定で上書きし、保存の知らせで言う）。
- `kind`: `standard` か `lilToon`。`shader` は lilToon のシェーダーの名前（描画モードと輪郭線はここから読む）。
- `properties`: lilToon のプロパティの名前と型（`float`・`int`・`color`・`vector`）。色はガンマの空間（Unity のマテリアルの値と同じ）。知らないプロパティも残す。
- `shaderGuid`・`shaderVersion`（任意。Unity のシェーダーのアセットの GUID と版。1〜128 文字）と `renderQueue`（任意。Unity のマテリアルの描画の順、
  符号付き 32 bit の整数）: 持つだけ（無ければ不明・シェーダーの既定）。`received` にも同じ欄を書く。
- `textures`: スロット（テクスチャのプロパティの名前）ごとの元。`channel`（標準 0〜5・ユーザーチャンネル 6〜63）、`packed`（R・G・B・A の 4 つを `zero`・`one`・
  `{ "channel", "component" }` から）、`image`（アセットの画像）。文書に無いチャンネルは割り当てなしとして扱う。
- `kindChosen`（真偽、無ければ偽）: 利用者が描き方を選んだか。選んでいなければ、Unity から受けた値があるとき受けた描き方で描く。
- `received`（任意）: Live Link で Unity のマテリアルから受けた値。本体と同じ形の `kind`・`shader`・`shaderGuid`・`shaderVersion`・`renderQueue`・`properties`・
  `textures`・`keywords` と、出どころの文 `source`、絵の無いスロットの理由 `missing`（`overBudget`: 受けた絵の予算を超えた・`unreadable`: 絵のファイルを読めない・
  `notAFile`: Unity の中にしかない絵）。受けた絵の画素は書かず、絵のあったスロットは `missing` にも書かない（開き直すと `livelink.json` の絵のファイルから読み直す）。
  0.4 までの書き手の `pending`（届いていない）は `unreadable` として読む。描くときは受けた値の上に本体を重ねる。
- 上限: プロパティ 2048・スロット 256・キーワード 256、名前は 1〜128 文字（UTF-16。シェーダーの名前は 256）で制御文字なし、値は有限の数だけ。
- 書き直すとき、前のエントリが同じ `format` なら知らないキーを残します。
- 形式も正本の版も上げない状態のエントリです。Unity 版は知らないエントリとして知らせて保存で落とします（失うのは見た目の設定だけ）。

## livelink.json（状態。根）

Live Link の相手の文書（Unity のシーンのオブジェクトを開いた文書）。16 MiB まで。Unity なしで開き直したとき、同じモデル（FBX の並び・レンダラーとマテリアルの
結び）とポーズになるように残す。中身は Live Link の頼み（`docs/LIVELINK.md` の「頼み」）と同じ形で、`format` 1・`kind` `"open"`。

- `target`（相手の身元 `key`・名前・書き出しの置き場）、`models`（FBX の絶対の道・GUID・取り込みの設定）、`renderers`（レンダラーの道・FBX の中のメッシュの道・
  入切・サブメッシュごとのマテリアルの番号・**今の** BlendShape の重み）、`bones`（**今の**ポーズで休みと違う骨の、FBX の親の骨に対するローカル）、`materials`
  （鍵・名前・シェーダーの身元・テクスチャの道。**値は書かない**: 値は各セットの `look.json` の `received`。lilToon の印の値 `_lilToonVersion` だけは残し、
  開き直したとき lilToon のスロットの絵をファイルから読むかを決める）、`refused`（Unity が送れなかったレンダラー）。
- 骨の道は Unity の根から名前で下る。道で指せない骨（畳んだ FBX の根の骨のような Unity の根の外の骨・同じ名前の兄弟）の値は書けないので、保存の知らせに骨の名前を出す
  （開き直すと休みに戻る）。
- 読み手は大きさ・JSON のオブジェクト・`format` 1 を確かめ（`livelink::validate`）、中身は Live Link の頼みと同じ決まりで読む（数の上限・有限の数・番号の範囲）。
  読めないものは、モデルを開き直さずにエントリをバイト列のまま残す。FBX・絵の道がネットワークの道（`\\host\share`）なら、開いたときに自動では読まない。
- 開き直すとき、テクスチャセットは鍵で結ぶだけで増やさない。元の絵は入れ直さない（レイヤーに入っている）。スロットの絵は道から読み直す。
- モデルを FBX・形ごと渡されたメッシュに替えて保存すると、エントリを外す。モデルがまだ無い（開き直している最中・開き直せなかった）間の保存は、
  エントリに触れない。
- 形式も正本の版も上げない状態のエントリです。スタンドアロン 0.4.x は知らないエントリとして知らせ、保存し直してもバイト列のまま残します（モデルは開かない。
  テクスチャセットは開ける）。Unity 版は知らせて保存で落とします（失うのは相手の記録とポーズだけ）。

## 選択範囲

### selection.bin（今の選択範囲）

little-endian。

| 欄 | 中身 |
|---|---|
| 4 byte | `YLSL` |
| int | 版（1） |
| int 幅・高さ・タイルの大きさ | 正本と同じでなければ断る |
| int タイルの数 | 0〜列 × 行 |
| タイルごと | int x・int y（左下のタイルが (0, 0)。`y × 列 + x` の昇順）、タイルの大きさ² byte の量（下の行から。0 でない量が 1 つ以上、文書の外の余白は 0） |

量のあるタイルだけを書くので、同じ選択範囲はいつも同じバイト列です。長さが合わない・並びが違う・正本と大きさが違うものは、開くのを断ります（Unity 版は理由を知らせて選択なしで開く）。

### selections.json と selection-<印>.bin（形式 8）

名前を付けて残した選択範囲。

```json
{ "format": 1, "selections": [ { "name": "前髪", "content": "<中身の印>" } ] }
```

- 索引 `sets/<ID>/selections.json`（256 KiB まで）: `format`（1。ほかは読まない）、`selections`（並びの順、1 セットに 32 個まで）。`name` は前後の空白がなく、1〜256 文字、
  制御文字なし、並びの中で重ならない。知らないキーは読み飛ばします。
- 中身 `sets/<ID>/selection-<印>.bin`: `selection.bin` と同じ `YLSL` の版 1。印は中身のバイト列の SHA-256 の先頭 128 bit（小文字の 16 進 32 桁。エントリの名前が
  96 バイトまでのため）。同じ中身は 1 つのエントリを共有します。
- 読み手は、壊れた項目だけを飛ばし（理由つき）、読める項目は読みます。飛ばしたエントリはバイト列のまま残ります（残した選択範囲を書き換えるまで）。文書と大きさが
  違うものは飛ばします。
- 書き手は、書き直すときに前の索引と `selection-*.bin` をセットごと全部置き換えます（使われなくなった中身を残さない）。1 つでもエントリがあれば `ylp.json` の `format` を 8 に、
  全部なくなれば 7 に戻します（外側の `YLP-3` は変えない）。

## composite/<チャンネル>.png（派生）

`<チャンネル>` は `Color`・`Roughness`・`Metallic`・`Height`・`Normal`・`Emission`。どれかのレイヤーが有効にしている標準のチャンネルごとに、正本の合成を
straight RGBA8 の PNG（色の型 6、補助のチャンクなし。PNG なので上の行から）で書きます。`Color` は使うレイヤーが無くても書き、Height から Normal を作る設定で
Height を使っていれば `Normal` も書きます（Normal は OpenGL の向き）。ユーザーチャンネルの合成は書きません。正本を替えたセットの `composite/` は消してから書き直します。
Unity 版のインポーターは、この並びからセットのチャンネルを出します。

## meshmap-<種類>.bin（派生）

`<種類>` は `WorldNormal`・`Position`・`AmbientOcclusion`・`Curvature`・`Thickness`・`TangentNormal`・`Height`・`Id`・`BentNormal`・`Opacity`（番号 0〜9）。
little-endian。

| 欄 | 中身 |
|---|---|
| 8 byte | `YLPMMAP\0` |
| int 版・int 種類・int エンジンの版 | 版 1〜3（書くのは 3）、エンジンの版 2 |
| 文字列 メッシュのハッシュ・トポロジーのハッシュ | 文字列は int の長さ（0〜4096）と UTF-8 |
| int UV のチャンネル・幅・高さ・スロット・パディング | |
| int アンチエイリアスの段数 | 版 2 から（版 1 は 1） |
| int チャンネル数 | 種類で決まる（3 か 1） |
| int スロットの数と、その数の int のスロットの番号（昇順） | 版 3 から（0〜65536 個）。版 1・2 はスロットの欄の 1 つ（負なら無し） |
| 文字列 設定の鍵・空間・ポーズ・元 | ID マップの元は `source=<元>;algorithm=2` に、必要ならマテリアル識別（`materials=<SHA-256>`）・手動の色（`manual=<SHA-256>`）の鍵。どの種類の鍵にも、重なった UV の優先（版 33 の頭の欄 `bake_priority`）が既定でないときだけ、末尾に `;owner=<決め方の名前。LowestIndex 以外>`・`;outside=skip`（0〜1 の外のアイランドを焼かない）・`;islands=<SHA-256>`（手で選んだアイランド）のうち変えた物が付き、既定は何も付かない |
| double ×6 | 由来の境界箱の最小 x・y・z、最大 x・y・z |
| int 長さ・Deflate の中身 | 展開すると、覆いの byte（テクセルの数）と、チャンネルごとに 16 bit の値の上位 byte の面と下位 byte の面（行ごとに左の値との差） |

どの条件で焼いたか（由来）が今のモデル・設定と違えば古いとして使いません（焼き直しを促す）。Generator と投影はメッシュマップを読み、無い・古いときは
値をそのまま出して理由を知らせます。

## 正本（document.utpaint）

テクスチャセットの唯一の正本。読み手は `native.rs`（`NativeDocument::read`）、core の文書との変換は `core_bridge.rs`。

### 値の型

little-endian。

| 型 | バイト |
|---|---|
| int | 4（符号付き） |
| byte | 1 |
| bool | 1（0 か 1。ほかは断る） |
| double | 8（IEEE 754。範囲の外・NaN・無限大は断る） |
| GUID | 16（C# の `Guid.ToByteArray()` の順: 最初の 3 つの区切りは little-endian。空は全部 0） |
| 文字列 | int の長さ（0〜4096 バイト）と UTF-8 |
| バイト列 | 決まった長さ（色の RGBA8 は 4、RGB は 3、タイルは タイルの大きさ² × 4） |

座標は左下が原点です。タイルの x・y は左下のタイルが (0, 0)、タイルの中の画素は下の行から、行の中は左から並べます。画素は straight RGBA8 で、透明画素の RGB も
保ちます。

以下の表の「欄」は、読み手が項目に付ける名前（`NativeDocument::fields()` の道、例 `layers[0].opacity`）です。「版」の欄は、その欄がある正本の版です。

### 頭

| 欄 | 型 | 版 | 中身 |
|---|---|---|---|
| `magic` | 8 byte | | `DOTPAINT` |
| `version` | int | | 1〜25・27〜30・32・33。26 は分けた正本（下）で、続けて int の中の版（21〜25・27〜30・32・33）と int の部分の数（1 以上）が来る |
| `id` | GUID | | 文書の ID（空でない） |
| `width`・`height` | int | | 1〜8192 |
| `tile_size` | int | | 8〜512 の 2 の累乗 |
| `normal` | 塊 | 7 | `algorithm` int（1）、`derive_from_height` bool、`strength` double（−256〜256）、`edges` int（0 端で止める・1 巻く）、`file_direction` int（0 OpenGL・1 DirectX） |
| `user_channel_count` | int | 22 | ユーザーチャンネルの数（版 22 は 1〜58、版 23 からは 0〜58） |
| `user_channels[i]` | 塊 | 22 | `channel` int（6〜63、昇順、歯抜けでよい）、`name` 文字列（1〜128 文字、制御文字なし、標準の 6 つの名前とほかと重ならない）、`kind` int（0 色・1 スカラー・2 法線）、`color_space` int（0 sRGB・1 線形）、`default` RGBA8 |
| `filter_seams` | bool | 32 | レイヤーのフィルターの近傍の段（ぼかしなど）が UV の継ぎ目をまたいで読むか（文書の設定）。版 32 の正本はいつも書く。この欄の無い版の正本は入（既定）として読む |
| `bake_priority` | 塊 | 33 | 重なった UV のテクセルの持ち主の決め方（ベイクの優先）。版 33 の正本はいつも書く。`rule` int（0 番号の小さい三角形・1 3D の面積の大きいアイランド・2 モデルの空間の +X の側のアイランド・3 −X の側のアイランド）、`skip_outside` bool（UV の外接矩形が 0〜1 の正方形と重ならないアイランドを焼かない。偽なら 0〜1 の外の UV があるベイクは断る）、`binding` 文字列（手で選んだアイランドを結び付けたモデルの指紋。下の 2 つの一覧が空なら空、どちらかにあれば小文字の SHA-256 64 桁）、`skip_count` int（0〜4096）と `skip[i]` int（焼かないアイランドの三角形の番号の 1 つ。0〜3999999、狭義の昇順）、`prefer_count` int（0〜4096）と `prefer[i]` int（優先するアイランド。同じ決まりで、`skip` と同じ番号を置かない）。アイランドは UV と 3D の位置の両方で辺を共有してつながる三角形（同じスロットの中）で、番号は三角形の通し番号（メッシュの順 × サブメッシュの順）。この欄の無い版の正本は既定（番号の小さい三角形・外さない・手で選んだアイランドなし）として読む |
| `layer_count` | int | | 0〜2048 |
| `layers[i]` | レイヤー | | レイヤーの数だけ（下）。`layers[0]` が一番下 |
| `manual_id_colors` | 塊 | 19 | 任意。レイヤーの後にデータが残っていれば: `tag` 4 byte（`YLID`）、`count` int（1〜4096）、`binding` 文字列（小文字の SHA-256 64 桁。三角形の UV・スロット・レンダラーのトポロジーと、全三角形のメッシュの塊の番号の並びから作る指紋）、`colors[i]`: `part` int（0〜3999999、狭義の昇順）・`rgb` int（0xRRGGBB） |

この後にデータがあれば「新しい読み手が必要」として断ります。

手動の ID の色（`manual_id_colors`）の書き手は、色が 1 つでもあるときだけ塊を書きます（空なら塊を書かず、色を持たない文書と同じバイト列）。塊は版 19 から
置けて、書き手の版（21 以上）はどれも 19 以上なので、色のために版は上がりません（色だけを持つ文書は、Unity 版が読める 21 のまま）。`binding` は色を決めた
ときのモデルの指紋で、読み手は今のモデルと合うかを見ずに、書いてあったとおりに文書へ戻します。今のモデルと合わない色は、別のモデルのものとして扱い
（色の編集と ID マップのベイクを断ります。全部戻せば外せます）、捨てません。

チャンネルの番号: 0 Color・1 Roughness・2 Metallic・3 Height・4 Normal・5 Emission（標準）、6〜63 はユーザーチャンネル（版 22 から、頭の一覧にある番号だけ）。
以下で「チャンネル」と書いた欄のうち、チャンネルごとの合成・塗りつぶしの値・調整の対象・ラスターのチャンネルはユーザーチャンネルも置けます（個数の上限は
6 ＋ 一覧の数）。塗りつぶしの画像・グラデーション、フィルター、パスのマテリアルは標準の 0〜5 だけです。同じ並びの中で同じチャンネルは 2 回置けません。

### レイヤー

| 欄 | 型 | 版 | 中身 |
|---|---|---|---|
| `id` | GUID | | 空でない。文書の中で重ならない |
| `name` | 文字列 | | |
| `visible` | bool | | |
| `opacity` | double | | 0〜1 |
| `blend` | int | | 合成モード（下の表、0〜26）。26（通過）はグループだけ |
| `attributes` | byte | 12 | 属性の印: ビット 0 クリッピング、1 `locks` が続く、2 チャンネルごとの合成が続く（版 14）、3 塗りつぶしの画像と投影が続く（版 16）、4 Anchor が続く（版 20）、5 塗りつぶしのグラデーションが続く（版 21）、6 パスの一覧が続く（版 27）、7 `attributes_ext` が続く（版 29）。その版に無いビットは断る |
| `locks` | int | 12 | ビット 1 のとき。1〜15: ビット 0 透明部分・1 画素・2 位置・3 すべて |
| `attributes_ext` | int | 29 | 属性のビット 7 のとき。続きの属性の印（1 以上、その版に無いビットは断る）: ビット 0 塗りつぶしの点のグラデーションが続く、ビット 1 テキストの値が続く（版 30）。ビット 2 から後は空けてある |
| `channel_blend_count` | byte | 14 | ビット 2 のとき。1〜チャンネルの上限 |
| `channel_blends[i]` | 塊 | 14 | `channel` int、`parts` byte（1〜3: ビット 0 合成モード・1 不透明度）、ビット 0 なら `mode` int（0〜26）、ビット 1 なら `opacity` double（0〜1）。無い部分はレイヤーの値に従う |
| `clipping` | bool | 5〜11 | 版 12 からは属性の印のビット 0 |
| `kind` | int | 3 | 0 ラスター・1 塗りつぶし（版 3）・2 調整（版 4）・3 グループ（版 6）。版 2 までは 0 |
| `parent` | GUID | 6 | 親のグループの ID（空は一番上） |
| `fill_count` | int | 3 | 塗りつぶしでなければ 0 |
| `fills[i]` | 塊 | 3 | `channel` int、`enabled` bool、`rgba` RGBA8 |
| `image_count` | int | 16 | ビット 3 のとき（塗りつぶしだけ）。0〜6 |
| `images[i]` | 塊 | 16 | `channel` int（標準、塗りつぶしの値があるチャンネル）、`resource_id` GUID（アセットの画像の `id`。空でない）、版 29 からは続けて `anisotropic` bool（画像を異方性のフィルターで読むか。この欄の無い版の画像は、このアプリ（0.5.0〜）では読む。0.4.x までは等方の三線形で読んでいた） |
| `projection` | 塊 | 16 | ビット 3 のとき。下の「投影」。画像が 0 枚なら投影を既定から変えていること |
| `gradient_count` | int | 21 | ビット 5 のとき（塗りつぶしだけ）。1〜6 |
| `gradients[i]` | 塊 | 21 | `channel` int（標準、Normal でない、塗りつぶしの値があり画像の無いチャンネル）と、Generator の欄（下。種類 5・アルゴリズムの版 2・合成 Replace であること） |
| `point_gradient_count` | int | 29 | `attributes_ext` のビット 0 のとき（塗りつぶしだけ）。1〜6 |
| `point_gradients[i]` | 塊 | 29 | `channel` int（標準、Normal でない、塗りつぶしの値があり、画像もグラデーションも無いチャンネル）、`algorithm` int（1）、`space` int（0 モデルの空間・1 UV の空間）、`spread` double（広がり、0〜1）、`point_count` int（1〜64）、`points[k]`: `x`・`y`・`z` double（±1e6。モデルの空間はモデルのルートの空間のシーンの単位、UV の空間は u・v で `z` は 0）、`rgba` RGBA8（点の色と不透明度） |
| `adjustment` | 塊 | 4 | 調整レイヤーだけ。下の「調整」 |
| `channel_count` | int | | ラスターでなければ 0。ただし塗りつぶしレイヤーで属性のビット 6 のとき（版 27）は、パスの一覧の画素の面の数 |
| `channels[i]` | 塊 | | `channel` int、`enabled` bool、タイル（下） |
| `has_mask` | bool | 2 | |
| `mask` | 塊 | 2 | `enabled` bool、`inverted` bool、`density` double（0〜1）、タイル（量はアルファ。RGB は 0 でなければ断る） |
| `has_surface_path` | bool | 8 | ラスターだけ |
| `surface_path` | 塊 | 8 | 3D のパス（下の「パス」） |
| `has_filters` | bool | 9 | |
| `filters` | 塊 | 9 | 中身のフィルター（下）。マスクがあれば続けて `mask.filters`（チャンネルの欄の無いフィルター）。調整・グループのレイヤーの中身のフィルターは 0 個 |
| `has_canvas_path` | bool | 10 | ラスターだけ。3D のパスと両方は持てない |
| `canvas_path` | 塊 | 10 | 2D のパス（下） |
| `anchor_flags` | byte | 20 | ビット 4 のとき。1〜3: ビット 0 レイヤーの Anchor、ビット 1 マスクの Anchor（マスクが要る） |
| `anchor`・`mask.anchor` | 塊 | 20 | `id` GUID（空でない、文書の中で重ならない）、`name` 文字列（空白だけでない、128 文字（UTF-16）まで）。Anchor が持つ値は書かない（下のレイヤーから作り直す） |
| `paths` | 塊 | 27 | ビット 6 のとき（ラスターか塗りつぶしレイヤー。`has_surface_path`・`has_canvas_path` は偽）。下の「パスの一覧」 |
| `text` | 塊 | 30 | 続きの印のビット 1 のとき。テキストレイヤーの値（下の「テキスト」）。パスの無いラスターのレイヤー（`has_surface_path`・`has_canvas_path`・属性のビット 6 は偽）で、有効な Color のチャンネルがあること |

合成モード: 0 Normal・1 Multiply・2 Screen・3 Overlay・4 Darken・5 Lighten・6 ColorDodge・7 ColorBurn・8 LinearDodge・9 LinearBurn・10 HardLight・
11 SoftLight・12 VividLight・13 LinearLight・14 PinLight・15 HardMix・16 Difference・17 Exclusion・18 Subtract・19 Divide・20 Hue・21 Saturation・22 Color・
23 Luminosity・24 DarkerColor・25 LighterColor・26 PassThrough。

タイル: `tile_count` int（0〜列 × 行）、`tiles[i]`: `x` int・`y` int（重ならない）、`length` int（タイルの大きさ² × 4 と同じ）、`rgba` バイト列。画素の無いタイルは書きません。

### 投影（projection）

| 欄 | 型 | 中身 |
|---|---|---|
| `algorithm` | int | 1 |
| `mode` | int | 0 UV・1 トライプラナー・2 平面・3 球・4 円柱・5 デカール（版 17） |
| `wrap` | int | 0 繰り返す・1 端で止める・2 画像の外を透明に（版 17） |
| `tile_u`・`tile_v` | double | 0.001〜10000 |
| `offset_u`・`offset_v` | double | −10000〜10000 |
| `rotation` | double | −360〜360（度） |
| `blend_width` | double | 0〜1（トライプラナーの混ぜる幅） |
| `placement` | 塊 | `center_x`・`center_y`・`center_z`（±1e6）、`rotation_x`・`rotation_y`・`rotation_z`（±360 度、Unity の Z → X → Y の順のオイラー角）、`size_x`・`size_y`・`size_z`（1e-6〜1e6）。モデルのルートの位置と向きを基準にしたシーンの単位（ルートの大きさは掛けない） |
| `depth_hardness`・`backface_angle`・`backface_hardness` | double | デカールだけ。0〜1・0〜180・0〜1 |

既定は UV・繰り返す・タイル 1・オフセット 0・回転 0・混ぜる幅 0.3・置き場は中心 0・回転 0・大きさ 1。

### Generator

フィルターの種類 6 の段（`filters.items[i].generator`）と、塗りつぶしのグラデーション（`gradients[i]`）が持つ欄。

| 欄 | 型 | 中身 |
|---|---|---|
| `type` | int | 0 EdgeWear・1 Dirt・2 PositionGradient・3 Thickness・4 Direction・5 ShapeGradient（版 13）・6 IdColor（版 15）・7 Anchor（版 20）・64 ノイズ・65 グランジ（版 23）・66 模様・67 アイランドごとのばらつき・68 ライト・69 マスクの組み立て・70 画像（版 28）。8〜63 は Unity 版の将来のために空けてあり、断る |
| `algorithm` | int | 1。ShapeGradient は版 21 から 2（勾配つき）も |
| `low`・`high` | double | 0〜1（`high − low` は 0.001 以上） |
| `softness` | double | 0〜1 |
| `invert` | bool | |
| `noise_amount` | double | 0〜1（重ねるノイズ） |
| `noise_scale` | double | 0.001〜1 |
| `noise_seed` | int | |
| `noise_space` | int | 0 モデル・1 UV |
| `blend` | int | 0 Multiply・1 Replace・2 Screen・3 Max・4 Min・5 Add・6 Subtract |
| `balance` | double | 0〜1（Dirt だけ。ほかは 0.5） |
| `axis` | int | 0〜2（PositionGradient だけ。ほかは 1） |
| `direction_x`・`direction_y`・`direction_z` | double | ±1e6（Direction だけ。長さ 0 でない。ほかは (0, 1, 0)） |
| `bent_normal` | bool | Direction だけ |
| `pin_count` | int | 0〜8 |
| `pins[i]` | 塊 | `kind` int（メッシュマップの種類 0〜9。種類ごとに使えるものだけ、重ならない）、`key` 文字列（そのベイクの条件の鍵。小文字の 16 進 64 桁） |
| `volume` | 塊 | ShapeGradient だけ: `shape` int（0 ボックス・1 球・2 平面）と投影の `placement` と同じ 9 つ、`falloff` double（0〜1） |
| `ramp` | 塊 | ShapeGradient のアルゴリズムの版 2 だけ。下の「ランプ」 |
| `tolerance`・`color_count`・`colors[i]` | int | IdColor だけ: 許容の幅 0〜255、色の数 0〜32、色 0xRRGGBB（重ならない） |
| `anchor_id`・`anchor_channel`・`anchor_read` | GUID・int・int | Anchor だけ: 読む Anchor の ID（まだ選んでいなければ空）、チャンネル（0〜5、Normal でない）、読み方（0 値・1 覆い） |
| `procedural` | 塊 | ノイズ・グランジだけ（下） |
| `effect` | 塊 | 模様・ライト・マスクの組み立て・画像・アイランドごとのばらつきだけ（版 28。下） |

ピンに使えるメッシュマップ: EdgeWear は Curvature・Position、Dirt は AmbientOcclusion・Curvature・Position、PositionGradient・ShapeGradient・Anchor は Position、
Thickness は Thickness・Position、IdColor は Id・Position、Direction は WorldNormal・BentNormal・Position、ノイズ・グランジは Position・WorldNormal、
模様は無し、ライトは WorldNormal、マスクの組み立ては Curvature・AmbientOcclusion・Position・Thickness、画像は無し（読むマップは投影の種類で決まり、塗りつぶしレイヤーの投影と
同じく最新のベイクを読む）、アイランドごとのばらつきは無し（焼いたマップを読まず、モデルの UV アイランドを読む）。

ノイズ・グランジ（`procedural`。重ねるノイズ・`balance`・`axis`・向き・`bent_normal` は既定のまま）:

| 欄 | 型 | 中身 |
|---|---|---|
| `space` | int | 0 位置（3D）・1 トライプラナー・2 UV（周期で巻く） |
| `scale` | double | 0.001〜1（境界箱の対角線・UV の長い辺に対する 1 セルの割合） |
| `seed` | int | |
| `rotation_x`・`rotation_y`・`rotation_z` | double | ±360 度（UV では効かない） |
| `bleed`・`blend_width` | double | 0〜1（にじみ・トライプラナーの境目の幅） |
| `basis`・`cell_output`・`fractal`・`octaves`・`lacunarity`・`gain` | int・int・int・int・double・double | ノイズだけ: 基底（0 値・1 Perlin・2 Worley）、セルの出力（0 F1・1 F2・2 F2−F1。Worley 以外は 0）、重ね方（0 fBm・1 ridged・2 turbulence）、オクターブ 1〜8、ラクナリティ 1〜4、ゲイン 0〜1 |
| `preset` | int | グランジだけ: 0 汚れの斑・1 錆の斑・2 傷の筋・3 ほこり・4 指紋・5 布目・6 ひび・7 飛沫・8 塗装の剥げ・9 木目・10 革のしぼ |

ノイズ・グランジの式は + − × ÷ sqrt floor と整数だけで書き、同じ設定・シード・マップなら、スレッドの数・領域に依らず同じバイトになります。式を変えるときは
アルゴリズムの版を上げて古い式を残します。

模様・ライト・マスクの組み立て・アイランドごとのばらつき（`effect`。版 28。重ねるノイズ・`balance`・`axis`・向き・`bent_normal` は既定のまま。模様・ライトは共通の `softness` が 0 で、
ぼかし・回り込みは自分の `softness` で持つ）:

| 種類 | 欄 |
|---|---|
| 66 模様 | `shape` int（0 縞・1 市松・2 水玉・3 縁・4 格子）、`scale` double（1〜512。UV の 0〜1 に繰り返す回数）、`angle` double（0〜360 度）、`width`・`softness`・`offset_u`・`offset_v` double（0〜1） |
| 68 ライト | `azimuth` double（0〜360 度。0 が +Z、90 が +X）、`elevation` double（0〜90 度）、`softness`・`ambient` double（0〜1） |
| 69 マスクの組み立て | `curvature`・`ambient_occlusion`・`position`・`thickness` の塊（それぞれ `weight`・`level`・`contrast` double（0〜1）と `invert` bool）、`combine` int（0 乗算・1 最大・2 加算） |
| 67 アイランドごとのばらつき | `seed` int（i32 の全域）、`min`・`max` double（0〜1。`min` ≤ `max`、等しいのはアイランドによらず同じ値）。共通の `softness` は効く。ピンは持たない |

アイランドごとのばらつきは、テクセルのモデルの UV アイランド（UV で辺を共有してつながる三角形。番号は 1 から、アイランドのいちばん小さい三角形の番号の順で、モデルが同じなら
解像度によらず同じ）の番号 i と `seed` から u = `cell_hash(seeds(seed)[0], i, 0, 0)` × 2^-32（[0, 1)。ノイズと同じ整数の hash）を出し、
`min + (max − min) × u` を `low`・`high`・`softness`・`invert` に通した値を `blend` で合わせます。アイランドの外のテクセルは入力のまま、UV が重なったテクセルは番号の小さい三角形のアイランドの値です。
モデルが無い・アイランドの図が作業メモリの予算（文書のアイランドの図・帯の写しの予算）に収まらないときは、段は入力を通して理由を出します。アイランドの図は保存しません（モデルから作り直す）。

画像（`effect`。版 28。重ねるノイズ・`balance`・`axis`・向き・`bent_normal` は既定のまま、ピンは持たない）:

| 欄 | 型 | 中身 |
|---|---|---|
| `resource_id` | GUID | 読むアセットの画像の `id`（塗りつぶしの画像の `resource_id` と同じ）。まだ選んでいなければ空 |
| `projection` | 塊 | 上の「投影」と同じ欄。`mode` は 0〜4（デカールの 5 は断る） |
| `component` | int | マスク・スカラーのチャンネルで値にする成分: 0 R・1 G・2 B・3 A・4 輝度（0.2126・0.7152・0.0722） |

画像の段は、塗りつぶしレイヤーの画像と同じ投影・同じミップマップの読み方で画像を読み、色のチャンネルではその画素の色（`invert` は RGB を反転、アルファは
見える度合いに掛ける）、マスク・スカラーのチャンネルでは選んだ成分を `low`・`high`・`softness`・`invert` に通した値を `blend` で合わせます。画像の値の無い画素
（外側が透明の画像の外・位置のマップに面の無い画素）は入力のままです。画像を選んでいない・アセットに無い・読めない・投影が読むマップが使えないときは、段は入力を
通して理由を出します。

### ランプとカーブ

ランプ（`ramp`）: `colors_count` int（2〜32）、`colors[i]`: `position` double（0〜1、昇順、間隔 0.0001 以上）・`rgb` 3 byte・`midpoint` double（0.01〜0.99）、
`opacities_count` int（2〜32）、`opacities[i]`: `position`・`opacity` double（0〜1）・`midpoint`、続けて値のカーブ `curve`。

カーブ（`<名前>_count` と `<名前>[i]`）: 点の数 2〜16、各点の `x`・`y` double（0〜1）。`x` は 0 から始まり 1 で終わる昇順で、間隔は 0.02 − 1e-6 以上。

### 調整（adjustment）

| 欄 | 型 | 中身 |
|---|---|---|
| `type` | int | 0 反転・1 レベル補正・2 色相/彩度/明度、版 24 から 64〜69（下）。3〜63 は断る |
| `algorithm` | int | 1 |
| `input_black`・`input_white`・`gamma`・`output_black`・`output_white`・`hue`・`saturation`・`lightness` | double | レベル補正は `input_black` ≥ 0・`input_white` ≤ 1・幅 1/255 以上・`gamma` 0.1〜9.99・出力 0〜1。色相/彩度/明度は −180〜180・−1〜1・−1〜1。64 からの種類は既定（0, 1, 1, 0, 1, 0, 0, 0）のまま |
| `detail` | 塊 | 64 からの種類だけ。下の「色調補正の欄」 |
| `channel_count`・`channels[i].channel` | int | 対象のチャンネル。色相/彩度・グラデーションマップ・カラーバランスは色のチャンネル（Color・Emission・色のユーザーチャンネル）だけ、トーンカーブ・明るさ/コントラスト・2 値化・ポスタリゼーションは法線に当てない |

### 色調補正の欄（版 24）

調整レイヤーは `detail`、フィルターの段は `adjust` の塊の中。

| 種類 | 欄 |
|---|---|
| 64 グラデーションマップ | `reverse` bool、`ramp`。版 25 からは続けて混色: `mix` int（0 通常・1 知覚的・2 リニア）、`luminance` int（0〜4。知覚的でなければ 3）、`segment_count` int（色の分岐点の数 − 1）、`segments[i]`: `enabled` bool と、真なら混合率曲線 `curve` |
| 65 トーンカーブ | カーブ `composite`・`red`・`green`・`blue` |
| 66 カラーバランス | `shadows_cyan_red`・`shadows_magenta_green`・`shadows_yellow_blue`、`midtones_…`、`highlights_…` の 9 つの double（−100〜100）、`preserve_luminosity` bool |
| 67 明るさ/コントラスト | `brightness` double（−150〜150）、`contrast` double（−50〜100） |
| 68 2 値化 | `level` int（1〜255） |
| 69 ポスタリゼーション | `levels` int（2〜255） |

### フィルター（filters）

`count` int（0〜32）、`items[i]`:

| 欄 | 型 | 中身 |
|---|---|---|
| `id` | GUID | 空でない。文書の中で重ならない |
| `type` | int | 0 ぼかし・1 シャープ・2 ノイズ・3 レベル補正・4 反転・5 正規化・6 Generator（版 11）、版 24 から 64〜69（色調補正）、版 28 から 70〜79（下）。7〜63 は断る |
| `algorithm` | int | 1 |
| `enabled` | bool | |
| `strength` | double | 0〜1 |
| `channel_count`・`channels[i].channel` | int | 中身のフィルターだけ（マスクのフィルターには無い）: 1〜6 個、標準のチャンネル |
| `radius` | int | ぼかし 1〜256、シャープ 1〜64、ほかは 0 |
| `amount` | double | 0〜5（シャープ。ノイズは 0〜1。ほかは 0） |
| `threshold` | int | 0〜255（シャープだけ。ほかは 0） |
| `seed`・`monochrome` | int・bool | ノイズだけ（ほかは 0・偽）。色のノイズはスカラーのチャンネルに置けない |
| `input_black`・`input_white`・`gamma`・`output_black`・`output_white` | double | レベル補正だけ（範囲は調整と同じ）。ほかは既定 |
| `generator` | 塊 | 種類 6 だけ。Generator の欄 |
| `adjust` | 塊 | 64〜69 の種類だけ。色調補正の欄（グラデーションマップ・カラーバランスは色のチャンネルだけで、マスクには置けない） |
| `effect` | 塊 | 70〜79 の種類だけ（版 28）。下の「フィルターの欄（版 28）」 |

Normal に置けるのはぼかしだけ。有効で強さが 0 より大きい段の到達半径（ぼかし・シャープは `radius`、70〜79 は下の表）の合計は、チャンネルごとに 512 まで。

### フィルターの欄（版 28）

フィルターの段の `effect` の塊の中。共通の `radius`・`amount`・`threshold`・`seed`・`monochrome`・レベル補正の欄は既定（0・0・0・0・偽・0, 1, 1, 0, 1）のまま。
長さ・半径は文書の画素。到達半径は実数の長さの切り上げ。

| 種類 | 欄 | 到達半径 | 置ける所 |
|---|---|---|---|
| 70 ヒストグラムスキャン | `position`・`contrast` double（0〜1） | 0 | スカラーのチャンネル（Roughness・Metallic・Height）とマスク |
| 71 ヒストグラムレンジ | `range`・`position` double（0〜1） | 0 | スカラーのチャンネルとマスク |
| 72 スロープぼかし | `intensity` double（0〜64）、`samples` int（1〜32）、`mode` int（0 平均・1 最小・2 最大）、`scale` double（1〜256）、`seed` int | `intensity` | 色・スカラーのチャンネルとマスク |
| 73 方向のぼかし | `angle` double（0〜360 度）、`distance` double（0〜256） | `distance` | 色・スカラーのチャンネルとマスク |
| 74 ゆがみ | `intensity` double（0〜128）、`scale` double（1〜256）、`seed` int | `intensity` | 色・スカラーのチャンネルとマスク |
| 75 モルフォロジー | `mode` int（0 太らせる・1 細らせる）、`radius` int（1〜64） | `radius` | スカラーのチャンネルとマスク |
| 76 エッジ検出 | `width` int（1〜16）、`threshold` double（0〜1） | `width` + 1 | スカラーのチャンネルとマスク |
| 77 ハイパス | `radius` int（1〜256） | `radius` | 色・スカラーのチャンネルとマスク |
| 78 メディアン | `radius` int（1〜16） | `radius` | 色・スカラーのチャンネルとマスク |
| 79 グロー | `threshold` double（0〜1）、`radius` int（1〜256）、`intensity` double（0〜4） | `radius` | 色のチャンネル（Color・Emission）だけ |

### パス（surface_path・canvas_path）

| 欄 | 型 | 中身 |
|---|---|---|
| `algorithm` | int | 1 |
| `id` | GUID | 空でもよい |
| `channel` | int | 0〜5 |
| `model_fingerprint` | 文字列 | 3D のパスだけ。1〜128 文字（UTF-16） |
| `brush` | 塊 | `radius` double（正。3D は 1e6、2D は 4096 まで）、`hardness`・`opacity`・`flow` double（0〜1）、`spacing` double（0.01〜4）、`rgba` RGBA8、`erase`・`pressure_size`・`pressure_opacity`・`pressure_flow` bool |
| `point_count` | int | 0〜4096 |
| `points[i]` | 塊 | 3D: `triangle` int（0 以上）と `u`・`v` double（三角形の重心座標、`u + v` ≤ 1）。2D: `x`・`y` double（±1e6、画素の座標）。どちらも `pressure` double（0〜1） |
| `material_count` | byte | 版 18。0〜6。0 なら `channel` がレイヤーの有効なチャンネルであること |
| `material[i]` | 塊 | 版 18: `channel` int（標準、レイヤーにあるチャンネル、重ならない）、`rgba` RGBA8 |

### パスの一覧（paths。版 27）

1 つのレイヤーの、下から順に描くパスの並び。レイヤーの対象チャンネルの画素は、見せるパスを順に同じ作業面へ描いた結果です（後のパスが前のパスの上に重なり、
消しゴムのパスは前のパスの画素を消す）。

塗りつぶしレイヤーのパス: 画素（`channels`）はパスを描いた結果だけで、合成では塗りつぶし → そのレイヤーの効果のスタック → パスの画素の順に重ねます（パスの
画素を「通常」で、`src · a + dst · da · (1 − a)`）。塗りつぶしレイヤーのパスは、1 本でもこの一覧の形で書きます（1 本の欄はラスターレイヤーだけ）。

| 欄 | 型 | 中身 |
|---|---|---|
| `count` | int | 1〜256 |
| `items[i].name` | 文字列 | 0〜128 文字（UTF-16）、制御文字なし。空なら画面が並びの番号から名前を作る |
| `items[i].visible` | bool | 偽なら描かない（点と設定は残る） |
| `items[i].surface` | bool | 真なら 3D のパス（`model_fingerprint` を持つ並び）、偽なら 2D のパス |
| `items[i].path` | 塊 | 上の「パス」と同じ並び |
| `items[i].extra` | 塊 | 1 本のパスの並びに無い設定（下の「パスの拡張」） |

一覧のパスは、どれも同じ側（2D か 3D か）・同じ `channel`・（3D は）同じ `model_fingerprint` で、`id` が重なりません（読み手は断る）。
書き手は、塗りつぶしレイヤーのパス・2 本以上・名前を付けた・隠したパスレイヤーと、拡張の設定（ストロークと消しゴム以外の種類・筆先・深さ・対称・滑らかでない点）を使うパスレイヤーだけを
この形で書き、名前の無い見せる 1 本のパス（ストロークか消しゴムで、丸い筆先・角度 0・自動の深さ・対称なし・滑らかな点だけ）は今までどおり
`surface_path`・`canvas_path` に書きます。

#### パスの拡張（extra。版 27）

| 欄 | 型 | 中身 |
|---|---|---|
| `kind` | byte | 種類: 0 ストローク、1 リボン、2 塗り、3 指先、4 消しゴム |
| `ribbon` | 塊 | 種類 1 のとき。`image` GUID（アセットの画像の `id`。空でない）、`mode` byte（0 並べる・1 伸ばす）、`spacing` double（0.1〜4。ダブの長さに対する中心の間隔。並べるときだけ効く） |
| `strength` | double | 種類 3 のとき。指先の強さ 0〜1 |
| `has_tip` | bool | 筆先の画像を持つか（ストローク・消しゴム・指先に効く。偽なら丸） |
| `tip` | 塊 | `has_tip` のとき。`name` 文字列、`width`・`height` int（1〜2048）、`alpha` byte 列（幅 × 高さ。覆い、行は下から） |
| `angle` | double | 筆先の角度（度、−360〜360、反時計回り） |
| `follow` | bool | 筆先をパスの進む向きに回す（角度はその向きから） |
| `has_depth` | bool | 3D の投影の深さを決めるか（偽なら自動） |
| `depth` | double | `has_depth` のとき。ブラシの半径の倍数（0.05〜64）。曲線から法線の向きにどこまで面を探すか |
| `symmetry` | byte | 対称: 0 なし、1 キャンバスの対称（2D のパスだけ）、2 鏡の面（3D のパスだけ） |
| `canvas_symmetry` | 塊 | `symmetry` 1 のとき。`mode` int（1 縦・2 横・3 両方・4 放射状）、`center_x`・`center_y` double（±1e7、画素）、`count` int（2〜16。放射状の写しの数） |
| `mirror` | 塊 | `symmetry` 2 のとき。`point_x`・`point_y`・`point_z`（面の上の点）、`normal_x`・`normal_y`・`normal_z`（法線。0 でない）。どれも double（±1000000）、休みの形のモデルの空間 |
| `tangent_count` | int | 0〜点の数。滑らかでない点の数（書かない点は滑らか） |
| `tangents[i].index` | int | 0〜点の数 − 1。前の項目より大きい |
| `tangents[i].kind` | byte | 1 角、2 取っ手 |
| `tangents[i].incoming`・`outgoing` | 塊 | 種類 2 のとき。2D は `x`・`y`、3D は `x`・`y`・`z`（double、±1000000）。点からの向き（2D は画素、3D は休みの形のモデルの空間） |

種類の意味（描き方の詳しい決まりは `crates/yolu-core/src/paths/README.md`）:

- ストローク: 丸いブラシのストローク（今までのパス）。消しゴム: 同じストロークで、一覧の前のパスの画素を消す。1 本の欄で書くときは、消しゴムの
  種類をブラシの `erase` の印で表す（Unity 版と同じ並び）。読み手は 1 本の欄の `erase` の印を消しゴムの種類として読む。
- 指先: 一覧の前のパスの画素を、パスに沿って引きずる。
- 塗り: パスの曲線（開いたパスは終わりから始めへ閉じる）の内側（巻き数が 0 でない所）を、ブラシの色・組の値と不透明度で塗る。3D は点が全部 1 つの
  UV アイランドにあるときだけで、曲線を面へ投影した UV の多角形の内側のうち、そのアイランドの UV に入る所。
- リボン: アセットの画像を、パスの向きに回した長方形のダブとして並べる（幅はブラシの直径）。色の種類のチャンネル（Color・Emission）は画像の色、
  ほかのチャンネルは組の値（無ければブラシの色）を、画像のアルファを覆いにして重ねる。アセットに無い画像のリボンは描き直せない（保存した画素は残る）。

対称: 映した側は持たず、描くときに作って元のパスのすぐ後に描く。2D はキャンバスの対称の写し（最初の恒等の写しは元のパス）で点と取っ手を写し、
3D は鏡の面で点の位置を映して、映した位置のいちばん近い面（同じマテリアル・向きの合う面・許す距離 = ブラシの半径とモデルの境界箱の対角線の
1% の大きい方の内側）へ置く。置けない点があれば、その写しは描かない。

筆先: 2D は通常のブラシの筆先と同じ（角度はキャンバスの向き、`follow` なら線の向きから）。3D は面のダブの中心の接平面に筆先を置き、横の軸は
`follow` ならパスの進む向き、そうでなければモデルの +Y を接平面に落とした向き（平行なら +X・+Z）から角度だけ回す。自動の深さは、ダブの
間隔の 4 倍・区間の長さの 1/4・半径の 4 倍の大きい方（今までの式）。

接線の意味: 区間（点 s → s + 1）の両端が滑らかなら、曲線は今までどおり点を順に通る centripetal Catmull–Rom です。どちらかの端が角か取っ手なら
3 次のベジェで、滑らかな端の制御点は Catmull–Rom のその端での微分 m から `点 ± m · (t2 − t1) / 3`（t は centripetal の節）、角の端は点そのもの、
取っ手の端は点 + 取っ手（出る側は `outgoing`、入る側は `incoming`）。

### テキスト（text）

テキストレイヤー（版 30）は、ラスターのレイヤーがテキストの値を持ったものです。レイヤーの Color のチャンネルの画素は、この値とフォントから描いた結果で、ほかのレイヤーと
同じタイルで保存します（読み手は画素をそのまま見せ、開くときに描き直さない）。フォントのファイルは入れません。

| 欄 | 型 | 中身 |
|---|---|---|
| `algorithm` | int | 1（並べと塗りの版。下） |
| `content` | 文字列 | 文（UTF-8 で 4096 バイトまで。改行は LF、ほかの制御文字はタブだけ） |
| `font_kind` | int | 0 アプリに同梱のフォント・1 フォントのファイル（OS に入っているフォントも、利用者が選んだファイルも） |
| `font_name` | 文字列 | `font_kind` が 0 のとき。同梱のフォントの名前（`a-z`・`0-9`・`-` の 1〜64 文字。今の名前は `biz-udpgothic`・`biz-udpgothic-bold`） |
| `font_path`・`font_index`・`font_sha256` | 文字列・int・文字列 | `font_kind` が 1 のとき。フォントのファイルの道（1〜4096 バイト、制御文字なし）、束（.ttc）の中の番号（0 以上。束でなければ 0）、選んだときのファイルの中身の SHA-256（小文字の 64 桁） |
| `font_family`・`font_postscript` | 文字列・文字列 | `font_kind` が 1 のとき。フォントのファミリー名（name の 16 番、無ければ 1 番。英語の名前、無ければ最初の名前）と PostScript 名（name の 6 番）。どちらも 0〜256 バイト、制御文字なし。読めなかった名前は空 |
| `font_weight`・`font_italic` | int・bool | `font_kind` が 1 のとき。太さ（OS/2 の usWeightClass、1〜1000）と、斜体か（OS/2 の fsSelection の斜体・斜め） |
| `size` | double | サイズ（1 em の画素、1〜4096） |
| `rgba` | RGBA8 | 文字の色（アルファは文字の不透明度。レイヤーの不透明度とは別） |
| `line_height` | double | 行間（行の送り。サイズに掛ける、0.1〜10） |
| `letter_spacing` | double | 字間（字ごとに足す送り。サイズに掛ける、−1〜10） |
| `align` | int | 0 左・1 中央・2 右 |
| `x`・`y` | double | 基準の点（文書の画素、左下が原点、±1000000）。1 行目の上端で、行は下へ進む |
| `rotation` | double | 基準の点のまわりの回転（度、反時計回りが正、−360〜360） |
| `wrap_width` | double | 折り返しの幅（画素、0〜1000000。0 は折り返さない）。0 なら揃えの基準は基準の点、0 でなければ基準の点から右へこの幅の箱 |

並べと塗り（`algorithm` 1）: 段落ごとに左から右へ並べ（右から左の文字・縦書きは扱わない）、折り返しの幅があれば空白の後と CJK の字の間で分け、
ヒンティングをかけずに輪郭を塗ります（アンチエイリアスの覆う量をアルファにし、覆わない画素は透明で RGB も 0）。同じ値・同じフォントなら、CPU・
スレッドの数によらず同じバイトです。式を変えて同じ値の画素が変わるときは `algorithm` を上げます（保存した画素は変わらず、文を直したときの
描き直しだけが新しい式になる）。可変フォントは既定のインスタンスで描きます。

フォントを探す（このアプリの決まり。読み手の検査ではない）: 同梱のフォントは名前で探します。ファイルのフォントは、`font_path` のファイルの中身が
`font_sha256` と同じならそれを使い、違う・無いときは OS に入っているフォントを `font_postscript` → `font_family` と `font_weight`（と `font_italic`）で
探し、最後に `font_path` のファイルです。見つけた中身の SHA-256 が違えば「フォントが違います」と知らせ、保存した画素はそのまま、テキストを直したときに
見つけたフォントで描き直します（値の道・SHA-256・名前もそのフォントのものになる）。どこにも無ければ、開くのは断らず保存した画素をそのまま見せ、
テキストの値の編集だけを理由を添えて断ります（別のフォントで黙って描き直さない）。

### 文書をまたぐ決まり

- 親（`parent`）は文書にあるグループで、子より上（大きい番号）にあり、グループの子は親のすぐ下に続けて並びます。グループの入れ子はこのアプリでは
  64 段まで（`yolu_core::MAX_GROUP_DEPTH`。超えるファイルは上限として断る。Unity 版の読み手には上限が無い）。
- Anchor の ID とフィルターの ID は文書の中で重ならない。Anchor の段が自分のレイヤーの Anchor を指すものは断ります。消えた Anchor・読むレイヤーより上にある Anchor を指す
  参照は開くのを断らずにそのまま読み、その段は入力を通して理由を出します（保存しても参照は残る）。
- アセットに無い `resource_id` は開くのを断らずに ID を残し、塗りつぶしの値を見せて知らせます（画像の Generator の段は入力を通して知らせます）。
- 塗りつぶしのグラデーションはアルゴリズムの版 2・合成 Replace であること。
- 点のグラデーション（アルゴリズムの版 1）の値: 点 i の重み `1 / (d² + s²)`（d は点までの距離、s は広がり × 長さ。長さはモデルの空間なら位置のマップの
  箱の対角線、UV の空間なら 1）。色は不透明度で重みを付けて混ぜ、不透明度は重みで混ぜる。s が 0 で点の真上はその点の色。モデルの空間は、位置のマップが
  覆わない画素・位置のマップかモデルのルートが使えない文書では、塗りつぶしの値を見せて理由を知らせます。

### 正本の版

各版は、それより前の版の並びに欄を追加したものです。読み手は 1〜25・27〜30・32（と 26）を読み、その版に無い欄・値を持つものは断ります。間の 31 は意味を決めておらず、
読み手は断ります（分けた正本の中の版でも同じ）。新しい機能を使わない文書は、前の版と同じ並びで版の数だけが違います。

| 版 | 追加したもの |
|---|---|
| 1 | レイヤー（ID・名前・表示・不透明度・合成モード）とチャンネルのタイル |
| 2 | レイヤーのラスターマスク |
| 3 | レイヤーの種類（ラスター・塗りつぶし）と塗りつぶしの値 |
| 4 | 調整レイヤー |
| 5 | クリッピング |
| 6 | グループ（親の ID、通過） |
| 7 | Normal の出力の設定 |
| 8 | 編集できる 3D のパス |
| 9 | フィルターのスタック |
| 10 | 2D のパス |
| 11 | フィルターの種類 6（Generator） |
| 12 | レイヤーの属性の印とロック |
| 13 | Generator の種類 5（ShapeGradient）と形の欄 |
| 14 | チャンネルごとの合成モードと不透明度 |
| 15 | Generator の種類 6（IdColor） |
| 16 | 塗りつぶしの画像と投影 |
| 17 | デカール（投影の種類 5）と外側 2（画像の外を透明に） |
| 18 | パスのマテリアルの組 |
| 19 | 手動の ID の色（末尾の塊 `YLID`。色があるときだけ書く） |
| 20 | Anchor（属性のビット 4）と Generator の種類 7 |
| 21 | ShapeGradient の勾配（アルゴリズムの版 2）、塗りつぶしの直接のグラデーション（属性のビット 5）。Unity 版が書く最後の版 |
| 22 | ユーザーチャンネル（頭の一覧） |
| 23 | Generator の種類 64・65（ノイズ・グランジ）。ユーザーチャンネルの一覧は 0 個も書く |
| 24 | 調整レイヤー・フィルターの段の種類 64〜69（色調補正） |
| 25 | グラデーションマップの混色と区間ごとの混合率曲線 |
| 26 | 分けた正本（下） |
| 27 | パスの一覧（属性のビット 6 と `paths`）と、パスの拡張（種類・筆先・投影の深さ・対称・点の接線）、塗りつぶしレイヤーのパスとその画素 |
| 28 | フィルターの段の種類 70〜79（`effect` の塊）と、Generator の種類 66〜70（`effect` の塊） |
| 29 | 続きの属性の印（属性のビット 7 と `attributes_ext`）、塗りつぶしの点のグラデーション（`attributes_ext` のビット 0）と、塗りつぶしの画像ごとの異方性のフィルターの入・切（`images[i].anisotropic`） |
| 30 | テキストレイヤー（続きの属性の印のビット 1 と `text` の塊） |
| 32 | レイヤーのフィルターが UV の継ぎ目をまたぐかの設定（頭の `filter_seams`） |
| 33 | 重なった UV のテクセルの持ち主の決め方（頭の `bake_priority`） |

このアプリの書き手（`NativeDocument::from_core`）は、使う機能が要る一番小さな版で書きます: ベイクの優先を既定から変えていれば 33、継ぎ目の設定を切っていれば 32、テキストレイヤーがあれば 30、点のグラデーションか異方性を切った塗りつぶしの画像があれば 29、フィルターの段の種類 70〜79 か Generator の種類 66〜70 があれば 28、パスの一覧があれば 27、グラデーションマップの混色があれば 25、色調補正があれば 24、ノイズ・
グランジがあれば 23、ユーザーチャンネルがあれば 22、どれも無ければ 21。機能を消して保存し直すと版も下がります。開いたまま変えていない正本は、元のバイト列の
まま書きます。

版 22 から 25 と 27〜30・32・33 は、Unity 版に無い機能を使う文書だけを新しい版にするための版です。Unity 版（0.4.x まで）の読み手は版の数だけで、ファイルに触れずに断ります（版 22・23・24 は
Unity 版の `Runtime/Core` の読み手に読ませた記録がある: `crates/yolu-io/tests/fixtures/*.unity.txt`）。.ylp の形式（7）は上げません（上げると、その機能を使わない
セットも含めてファイル全体を Unity 版が開けなくなる）。Unity ブリッジは 0.5.0 から .ylp を扱いませんが、版の数の意味は変えません（同じ版の数が別の
意味になると、古い読み手も新しい読み手も相手の文書を読み違える）。種類の番号の 8〜63（Generator）・3〜63（調整）・7〜63（フィルター）は Unity 版の並びのために空けたままにし、
このアプリだけの種類は 64 から振ります。`.ylsmart`（形式 1）にはユーザーチャンネル・64 からの種類を入れません（書き手が理由を添えて断る）。

読み手が読む版は 1〜25・26・27〜30・32・33 だけです。間の 31 は意味を決めていないので、範囲に入れず版の数で断ります（版 25 の並びとして読むと、意味の無い版の文書を
黙って読んでしまう）。分けた正本の中の版も同じで、21〜25・27〜30・32・33 のほかと、26 自身は断ります。

31 は欠番で、これからも割り振りません。版は積み重ね（上の版は下の版の機能を全部持てる）なので、版 32・33 の読み手は、31 に後から意味を付けた中身を
知りません。31 を使うと、版 32・33 の文書を前の読み手が読み違えます。新しい機能の版は、配った一番上の版より上の番号にします。

### 分けた正本（版 26）

中身の正本（中の版 21〜25・27〜30・32・33 の並び）が 512 MiB を超える文書だけを分けて書きます。

| エントリ | 中身 |
|---|---|
| `document.utpaint`（ヘッダー） | `DOTPAINT`、int 26、int 中の版、int 部分の数（1 以上）、続けて中の版の並び（`id` から最後まで）からバイト列の値（色・画素。`magic` のほか全部）を抜いたもの |
| `document.utpaint.1`・`.2`… | バイト列の値を並びの順に。番号は 1 から続く（0 始まり・0 埋め・飛びは断る） |

- 書き手は、2 番目からのレイヤーの始まりと、256 MiB を超える手前で区切ります（値 1 つは分けない）。今の部分も次のレイヤーの値も 16 MiB に満たなければレイヤーの始まりで
  区切りません。変わらないレイヤーの部分は前と同じ中身になり、復旧の世代で共有されます。レイヤーより後の値（手動の ID の色の `tag`）は最後のレイヤーの値として数えます。
- 手動の ID の色の塊は、`tag`（`YLID`）が `Bytes` の値なので最後の部分に入り、`count`・`binding`・色の並びは平の値なのでヘッダーの末尾に入ります。
- 読み手は区切りの位置を決め打ちせず、値が部分の境目をまたがない・空の部分が無い・部分の数（ヘッダーとエントリ）が合う・全部の部分を余りなく使う、を
  確かめます。読んだ項目は中の版の正本を読んだのと同じです。
- 部分の名前は今の名前の決まりに収まるので、`YLP-3` に収まる大きさのファイルにも版 26 の正本が入りえます。
- 古い読み手: スタンドアロンの版 25 までの読み手は「`.version の値 26 は未対応`」、Unity 版は `Unsupported archive version; source retained unchanged.` で断ります。
  `YLP-4` のファイルは、Unity 版は 832 MiB を超えれば `The file exceeds the read budget.`、それ以下なら 1002 を超えるエントリで `Too many entries.`、1 MiB を
  超える manifest で `… is larger than allowed.`、ほかは manifest の版で `The file was written by a newer YoluPainter (YOLUPAINTER-YLP-4).` と断ります。

## 開くとき

1. 外側を読み、manifest と全エントリの長さ・CRC・SHA-256 を確かめる（ファイルから開くときは、大きなエントリは位置だけを覚え、要るときに確かめ直しながら読む）。
2. `ylp.json` を読む（無ければ形式 1）。今より新しい形式なら、どのエントリにも触れずに断る。
3. 古い形式なら移行の段を順に通す（メモリの上だけ）。
4. `project.json` を読み、並びの全部のセットの正本と選択範囲、アセットの並びと中身を読む。1 つでも読めなければ、開いているプロジェクトは何も変えない。
5. 知らないエントリを一覧にして知らせる。core の文書にできない正本（調整の種類が使わない値が既定でないものなど）は、セットと理由を知らせて読むだけで開く（元のバイト列のまま保存する）。手動の ID の色は文書へ戻して開く。

## 保存

- 書いたものを読み直して確かめ、同じフォルダの一時ファイル（`.<名前>.<プロセス番号>-<通し番号>.pending~`。名前が 200 バイトを超えるときは、先頭を切って指紋を付けた短い形）に書いてフラッシュし、読み直して確かめ、開いた・保存した時点の
  ファイルの印（SHA-256・長さ・更新時刻）と今のファイルが同じことを確かめてから、最後の 1 回で置き換えます。違えば上書きを断ります（外の書き換え）。途中で
  失敗しても元のファイルはそのままです。
- 直前の版は `<ファイル名>-backups~/` に退避します。残す数は設定で決め（既定はすべて）、超えた古いものだけを消します。
- いつも今の形式で書き、`ylp.json` に保存したアプリと（分かれば）作ったアプリを記録します。

## 復旧の世代

保存していない作業の書き置き（`generation.rs` の `GenerationStore`。.ylp ではない）。置き場の中に `current`・`previous`（世代の名前）、
`generations/<世代>/manifest.sha256`、`contents/<SHA-256>.bin` を置き、世代を確かめてから `current` を最後に置き換えて確定します。manifest の 1 行目は
`DOTPAINT-MANIFEST-1`（中身を世代の中に持つ）か `DOTPAINT-MANIFEST-2`（中身は `contents/` にあり、同じ中身は世代の間で 1 つを共有する）、続く行は
`<SHA-256> <長さ> <名前>` で、名前は .ylp のエントリと同じ範囲（`ylp.json`・`project.json`・`resources.json`・`resources/*`・`sets/<ID>/*` など）です。世代に添える
一覧用の情報は `recovery.json`（.ylp には入らない）。Unity 版が書いた世代はこのアプリが読めます。逆は、`sets/<ID>/composite/Color.png` のような入れ子の名前を
含む世代を Unity 版が「Unsafe generation filename」で断ります。量の数え方と置き場は [RECOVERY.md](RECOVERY.md)。

## .ylsmart（スマートマテリアル・スマートマスク）

置き場に置く 1 つのファイルで、.ylp のアセットにも同じバイト列で入ります（`smart.rs`）。

- 外側は .ylp と同じ zip のレイヤー。`mimetype` は `application/x-yolupainter-smart`、manifest の 1 行目は `YOLUPAINTER-SMART-1`。名前は根と `resources/` の下の 1 段。
- `smart.json`（正本）: `format`（1）、`kind`（`smartMaterial`・`smartMask`）、`name`、`width`・`height`・`layers`・`channels`（読むときにレイヤーと比べ、違えば断る）、
  `repin`（ベイクへのピンがあった Generator の段の ID。置いた先で付け直す）、`savedBy`（必須）。任意の `thumbnailShape: "sphere"`（見本が球）。知らないキーは読み飛ばす。
- `layers.utpaint`（正本）: 選んだレイヤーだけを持つ正本（版 1〜21）。保存したテクスチャセットの大きさ。スマートマスクは値の無い塗りつぶしレイヤー 1 つがマスクを持つ。
  モデルの上のパスは持たない。Generator はベイクへのピンを持たない。
- `resources.json`・`resources/<content>.png`（レイヤーが参照する画像があるとき）: .ylp と同じ形（画像だけ）。
- `thumbnail.png`（派生）: パネルの見本。
- `.ylmaterial`（置き場のマテリアル）は、塗りつぶしレイヤーを持つ `.ylsmart` と同じ形です。

## .ylbrush（アセットの携帯ブラシ）

.ylp のアセットに入る Unity 版のブラシの形（このアプリは読んで、バイト列のまま残す）。zip のレイヤーで、`mimetype` は `application/x-yolupainter-brush`、manifest の 1 行目は
`YOLUPAINTER-BRUSH-1`。`state.json`（`schema` 1〜3。外の筆先の ID `tipId`・`textureId`・`dualTipId` は空であること）、`tip-0.png` から続く筆先（256 枚まで）、
任意の `texture.png`・`dual.png`。PNG は RGBA8・1〜2048 px。このアプリの利用者のブラシのファイル（設定のフォルダの `brushes/`）は同じ拡張子のテキスト形式で、
[BRUSH.md](BRUSH.md) にあります。

## 形式を変えるとき

1. **正本・状態のエントリを追加する・名前や置き場を変える・意味を変える**ときは中身の形式を上げ（`project::MAX_FORMAT`）、前の形式からの移行を追加します。上げないと、
   古い読み手が知らないエントリを落として保存し、作業を失います。使う文書だけを新しい形式にできるなら、そうします（形式 8 のように）。
2. 派生のエントリだけを追加するときは上げなくてよい（古い読み手は知らないエントリとして知らせ、作り直す）。名前の決まり（置けるフォルダ）を広げるなら外側の版も
   上げます。状態のエントリは、古い読み手が落としても失うのがその状態だけなら、形式を上げずに追加できます（`look.json`・`pose.json`）。
3. エントリの中身の版だけを変えるときは、そのエントリの版を上げ、古い版も読めるようにします。正本の版を上げるときは、新しい機能を使う文書だけを新しい版で
   書きます（版 22〜25 のように）。
4. **同じコミットでこの文書を直します**（版の早見・読み手ごとの範囲・エントリの表・その欄）。`format_doc.rs` が載り忘れを落とします。
5. 前の形式・版のファイルを試験のフィクスチャにして、開ける・中身が同じに読み書きできることを確かめます（`crates/yolu-io/tests/ylp/compatibility.rs` など）。

## 決めたことの理由

形を選んだ理由・採らなかった案・保証の射程。Unity 版の読み手に実際に読ませた記録は `crates/yolu-io/tests/fixtures/*.unity.txt`。

### Live Link の相手を別のエントリにした（`livelink.json`）

- **状態のエントリ（採用）。** 相手の記録は画素の意味を変えないので、形式を上げると Live Link を使っただけで古い読み手がファイル全体を開けなくなる。
  別のエントリなら、0.4.x は知らないエントリとしてバイト列のまま残し（`Note::UnknownEntryKept`）、テクスチャセットは鍵（Unity のマテリアルの GUID と
  localFileId）のまま開ける。失うのは、0.4.x で開いている間のモデルの表示だけ。
- **ポーズを `pose.json` に書く案（採らない）。** `pose.json` はプロジェクトのモデルのファイル（`view.json` の `standaloneModel`）のポーズで、0.4.x は
  モデルのファイルの無い文書を保存するとき `pose.json` を外す。相手のポーズを `livelink.json` の中に持てば、0.4.x で保存し直しても失わない。
- **値（lilToon のマテリアルの値）は書かない。** 値は `look.json` の `received` にあり、保存するかは利用者の設定で決まる。同じ値を 2 か所に持つと、
  設定で外したのに残る・食い違う。

### 見た目の設定を別のエントリにした（`look.json`）

選んだ理由と、採らなかった案:

- **別のエントリ（採用）。** 見た目の設定は画素の意味を変えないので、正本の版を上げると、lilToon の見た目にしただけで Unity 0.2.0 がファイル全体を開けなくなる。
  別のエントリなら Unity 0.2.0 は開けて、失うのは見た目の設定だけで済む。
- **正本の版を上げて正本の中に持つ。** 上の互換の損失が大きい。
- **view.json に入れる。** Unity 版 0.2.0 は view.json を自分の項目（モデル・選んだチャンネル・可視性）だけで書き直す（`JsonUtility`）ので、Unity 版で保存すると落ちるのは同じ。画面の状態と、保存する文書の見た目の値を同じ場所に混ぜない。

### 版 22（ユーザーチャンネル）

Unity 0.2.0 は版22を「Unsupported archive version; source retained unchanged.」で断る。C#の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open`（`Runtime/Core` をそのままコンパイル）にRustが書いた版22を読ませた記録が [`user-channels-v22.unity.txt`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-io/tests/fixtures/user-channels-v22.unity.txt) で、外側の .ylp と `project.json` は読め、開く手順のセットの正本の読みで断る（全部のセットを読めてから入れ替えるので、Unityの状態は変わらず、ファイルも書き換えない）。記録の最後の行は、ウィンドウ（`TexturePaintWindow`）が正本の読みの失敗に付ける文を、読み手の例外から同じ形に組み立てたもの。

選んだ理由と、採らなかった案:

- **版を上げる（採用）。** Unity版が新しい機能を追加するたびに使ってきた決まりで、古い読み手は版の数だけで、ファイルに触れずに断る。ユーザーチャンネルが無い文書は版21のままなので、既存の文書、Rust版で保存した大多数の文書をUnity 0.2.0が開ける。
- **全部の文書を版22（0個の一覧）で書く。** Unity 0.2.0がRust版で保存した文書を1つも開けなくなる。ユーザーチャンネルを使わない人まで巻き込むので採らない。失うもの: チャンネルを追加すると版が22に、全部消すと21に変わる（保存のたびに今の中身で決まる）。
- **.ylp の形式を8にする。** Unity 0.2.0は `ylp.json` の時点でファイル全体を断る。ユーザーチャンネルを持たないセットも巻き添えになる。形式を上げる決まり（エントリの追加・意味の変更）に当たらない。
- **ユーザーチャンネルの画素を別のエントリに置き、`document.utpaint` は版21のままにする。** Unity 0.2.0は知らないエントリとして一覧に出すだけで、そのまま開き、保存するとそのエントリを落とす。ユーザーチャンネルを黙って失う保存ができてしまうので、「未対応の情報を黙って捨てない」に反する。
- **版21のまま番号6〜63を許す。** Unity 0.2.0は `Invalid or duplicate channel` の汎用の文で断り、新しい版が要るとは言えず、破損と区別できない。名前・種類・色空間・既定値の置き場も無い。
- **`ylp.json` に機能の旗を追加する。** Unity 0.2.0は知らないキーを読み飛ばすので、効かない。

移行: 版1〜21はそのまま読め（ユーザーチャンネルの一覧は空）、coreを通して保存すると版21（ユーザーチャンネルがあれば版22）になる。版22を読むRust版は、版21までを読む規則を変えない。版22から21へ戻すには、ユーザーチャンネルを消して保存し直す。

保証の射程と失うもの:

- 1つのセットでも版22なら、その .ylp 全体をUnity 0.2.0は開けない（セットごとには開けない）。Unityで開く必要があるときは、ユーザーチャンネルを消したファイルを保存する。版22から21へ直す移行はUnity側には無い。
- 版22の意味は上の「正本」の節（頭の `user_channels`）に固定する。Unity版が版22以降を別の意味で使うなら、同じ形式を仕様にして揃える（版の数が食い違うと、どちらの書き手も相手の文書を読み違える）。
- ユーザーチャンネルはC#に書き手が無い。合成は（標準のチャンネルも）coreのf32の合成の式が正本で、Unity 0.2.0のC#の合成と画素の一致は保証しない（保証は読み手の結果と往復のバイト一致の範囲）。合成のPNG（`composite/<チャンネル>.png`）は標準のチャンネルだけで、ユーザーチャンネルの分は書かない。
- 保証の範囲は `Runtime/Core` の読み手（`DocumentBinary`・`YlpFormat`）の結果まで。Unityのウィンドウ（EditorWindow）の操作の結果は範囲に含めない。

### 版 23（ノイズ・グランジ）

評価の約束: 位置（Position）のマップが使える間は、位置から 3D で評価する（UV アイランドの継ぎ目で模様がずれない）。2D の模様のプリセット（傷の筋・指紋・布目）は、位置の空間では自動でトライプラナー（向きのマップが要る。塗りつぶしの投影と同じ重み）。位置のマップが使えない（無い・古い・大きさが違う・ピンと違う・境界箱が 0）ときは、入力のまま通さず UV の空間に落とし、理由を `BoundGenerator::fallback` と `Document::fallback_effect_list` で返す（値は出ている。UV では x・y の格子を周期で巻くので端で継ぎ目が出ず、回転は効かない）。式は + − × ÷ sqrt floor と整数だけで書き（sin・cos・pow・exp を使わない）、同じ設定・シード・マップなら、スレッド数・領域・ブロックの大きさに依らず同じバイトになる。

Unity 0.2.0 は版23 を、版22 と同じく「Unsupported archive version; source retained unchanged.」で断る。C#の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open`（`Runtime/Core` をそのままコンパイル）にRustが書いた版23を読ませた記録が [`procedural-v23.unity.txt`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-io/tests/fixtures/procedural-v23.unity.txt)（正本は [`procedural-v23.utpaint`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-io/tests/fixtures/procedural-v23.utpaint)）。

選んだ理由と、採らなかった案:

- **版を上げる（採用）。** ユーザーチャンネルと同じ流儀で、古い読み手は版の数だけで、ファイルに触れずに断る。この種類を使わない文書は版21（ユーザーチャンネルがあれば22）のままなので、Unity 0.2.0が開ける範囲は変わらない。
- **版21のまま種類 64・65 を許す。** 版の数が食い違うと、どちらの書き手も相手の文書を読み違える（版22 の節と同じ理由）。Unity 版が将来 64 以降を別の意味で使うと、同じ版21 のファイルの意味が割れる。
- **既存の種類（EdgeWear・Dirt）の「重ねるノイズ」を拡張する。** 同じ種類の画素が Unity 版と変わり、C# の正解との全バイト一致（338事例）を壊す。
- **.ylsmart（形式1）に入れる。** 形式1 は Unity 版と共有で、版23 の断片を入れた .ylp を Unity 版がアセットごと開けなくなる。ユーザーチャンネルと同じく、この種類を含む素材は `SmartFile::from_core` が理由を添えて断る（`REFUSAL_RUST_GENERATORS`）。失うもの: ノイズ・グランジを使ったレイヤーを「レイヤーから保存」でアセットへ入れられない（同梱の素材はコードで作った core の素材をアセットの「組み込み」に並べるので影響しない）。形式2 を決めれば入れられる。

移行: 版1〜22はそのまま読める。この種類の段を消して保存し直すと、版21（ユーザーチャンネルがあれば22）に戻る。

保証の射程と失うもの:

- 1つのセットでも版23 なら、その .ylp 全体をUnity 0.2.0は開けない。Unityで開く必要があるときは、ノイズ・グランジの段を消した（または焼き込んだ）ファイルを保存する。
- 画素は Rust の式で、C#に対応する実装が無い。外部の正解は無く、`tests/procedural-index.txt`（実装を固定するハッシュ。回帰の固定）と性質の試験（決定性・継ぎ目・取消・予算・検査）で守る。式を変えると既存の文書の見た目が変わるので、変えるときはアルゴリズムの版を上げて古い式を残す。
- 数値の一致は Linux x86_64 で確かめた範囲。Windows での確認は未測定。
- 保証の範囲は `Runtime/Core` の読み手（`DocumentBinary`・`YlpFormat`）の結果まで。Unityのウィンドウ（EditorWindow）の操作の結果は範囲に含めない。

### 版 24（色調補正）

Unity 0.2.0 は版24 を、版22・23 と同じく「Unsupported archive version; source retained unchanged.」で断る。C#の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open`（`Runtime/Core` をそのままコンパイル）にRustが書いた版24を読ませた記録が [`adjust-v24.unity.txt`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-io/tests/fixtures/adjust-v24.unity.txt)（正本は [`adjust-v24.utpaint`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-io/tests/fixtures/adjust-v24.utpaint)）。

選んだ理由と、採らなかった案:

- **使う文書だけ版を上げる（採用）。** ユーザーチャンネル・手続き型と同じ流儀で、古い読み手は版の数だけで、ファイルに触れずに断る。色調補正を使わない文書は版21・22・23 のままなので、Unity 0.2.0が開ける範囲は変わらない。
- **使わない文書も版24 に上げる。** Unity 0.2.0 が開けなくなる互換の損失が大きい。
- **版21のまま種類 64〜69 を許す。** 版の数が食い違うと、どちらの書き手も相手の文書を読み違える。Unity 版が将来 64 以降を別の意味で使うと、同じ版21 のファイルの意味が割れる。
- **.ylsmart（形式1）に入れる。** 形式1 は Unity 版と共有で、版24 の断片を入れた .ylp を Unity 版がアセットごと開けなくなる。この種類を含む素材は `SmartFile::from_core` が理由を添えて断る（`REFUSAL_RUST_ADJUSTMENTS`）。失うもの: 色調補正を使ったレイヤーを「レイヤーから保存」でアセットへ入れられない。形式2 を決めれば入れられる。

移行: 版1〜23はそのまま読める。色調補正のレイヤーと段を消して保存し直すと、版21（ユーザーチャンネルがあれば22、ノイズ・グランジがあれば23）に戻る。

保証の射程と失うもの:

- 1つのセットでも版24 なら、その .ylp 全体をUnity 0.2.0は開けない。Unityで開く必要があるときは、色調補正のレイヤーと段を消した（または焼き込んだ）ファイルを保存する。
- 式（輝度の重み・明るさ/コントラストの式など）はRustの式で、PhotoshopやCLIP STUDIOの同名の調整と一致するとは言わない。C#に対応する実装は無い。式を変えると既存の文書の見た目が変わる。
- PSDの調整レイヤーとしての読み書きは、`crates/yolu-io/README.md` の「PSD・PSB」の表のとおり。
- 保証の範囲は `Runtime/Core` の読み手（`DocumentBinary`・`YlpFormat`）の結果まで。Unityのウィンドウ（EditorWindow）の操作の結果は範囲に含めない。

### 版 25（グラデーションマップの混色）

Unity 0.2.0 は版25 を、版22〜24 と同じく版の数だけで断る（Unity 版の `DocumentBinary.IsReadable` は 1〜21 だけを読み、21 を超える版を「Unsupported archive version; source retained unchanged.」で断る。版22〜24 は実際に読ませた記録がある。版25 の文書そのものを読ませた記録は、この環境に .NET が無く取れていない）。

選んだ理由と、採らなかった案:

- **使う文書だけ版を上げる（採用）。** 色調補正を使わない文書は 21〜23、使うだけなら 24 のまま。古い読み手が開ける範囲は広がらない。
- **塗りつぶしのグラデーションのランプにも混色を持たせる。** そのランプの並びは Unity 版と共有（版 21）で、意味を変えられない。混色を持つランプが塗りつぶしのグラデーションに入っていれば、黙って落とさず保存を断る（`Unwritable::GeneratorRampMixing`。画面は塗りつぶしのグラデーションに混色を出さない）。
- **版24 のまま混色の欄を追加する。** 版24 のグラデーションマップを書く既存の書き手と、版の数が同じで並びが違うファイルができ、どちらも相手を読み違える。

移行: 版1〜24はそのまま読める。混色をやめて保存し直すと、版24（ほかの機能が無ければ 21〜23）に戻る。

保証の射程と失うもの:

- 1つのグラデーションマップでも混色を使えば、その .ylp 全体を Unity 0.2.0 は開けない。
- 混色の式はこのアプリの式で、CLIP STUDIO・Photoshop の同名のモードと画素まで一致するとは言わない。式を変えると既存の文書の見た目が変わるので、変えるときは版を追加して古い式を残す。
- .ylsmart（形式1）には入れない（版24 の色調補正と同じ断り）。

### 版 27（パスの一覧）

1 つのレイヤーに何本ものパスを置く（パスごとに名前・表示）。パスの種類（ストローク・リボン・塗り・指先・消しゴム）、筆先の画像・角度・向き、投影の深さ、対称と、点ごとの接線（角・取っ手）を持つ。
塗りつぶしレイヤーにもパスを置ける（塗りつぶしと効果のスタックの上にパスの画素を重ねる）。Unity 0.2.0 は版 22〜25 と同じく版の数だけで断る（`DocumentBinary.IsReadable` は 1〜21 だけを
読む。版 27 の文書そのものを読ませた記録は取れていない）。スタンドアロン 0.4.x は「`.version の値 27 は未対応または範囲外です (1..25)`」で断る（読み手の
範囲は 1〜26。試験 `crates/yolu-io/tests/ylp/path_lists_bridge.rs`）。

選んだ理由と、採らなかった案:

- **使うレイヤーだけ一覧の形で書く（採用）。** 名前の無い見せる 1 本のパスは、今の `surface_path`・`canvas_path` のまま書くので、一覧を使わない文書は前と同じ
  版・同じバイト列。
- **レイヤーの属性のビットで一覧を追加する（採用）。** 一覧を持たないレイヤーは 1 バイトも増えない（レイヤーごとの bool を追加すると、版 27 以降の全部のレイヤーが 1 バイトずつ増える）。
- **1 本のパスの欄を並べて繰り返す。** 古い欄はレイヤーに 1 つの決まりで、2 つ目を置く場所が無い。
- **一覧の 1 本目を古い欄に書き、2 本目からを一覧に書く。** 古い読み手が一部だけを読んで保存し直すと、2 本目からを黙って失う。版で丸ごと断る方がよい。
- **新しいパスの設定は一覧の項目の `extra` に置く（採用）。** 1 本のパスの並び（版 8・10・18）は Unity 版と同じ並びのまま残し、その並びで表せない
  設定を使うパスだけを一覧の形で書く。接線は滑らかでない点だけを書く（多くの点は滑らかなので、点ごとに 1 バイトを追加するより小さい）。
- **滑らかな区間の式を Catmull–Rom のまま残す（採用）。** 角・取っ手の無いパスは前と同じバイトで描く。全部の区間をベジェに直す案は、同じ曲線でも
  浮動小数の丸めが変わり、今の文書の画素が変わるので採らない。

移行: 版 1〜26 はそのまま読める（古い 1 本のパスは名前の無い、見せる一覧の 1 本として読む）。パスを 1 本に戻し名前を消し、拡張の設定を外して
（点を滑らかに戻して）保存し直すと、ほかの機能が無ければ前の版に戻る。

保証の射程と失うもの:

- 一覧を使うレイヤーが 1 つでもあれば、その .ylp 全体を Unity 0.2.0 とスタンドアロン 0.4.x は開けない。
- .ylsmart（形式 1）には入れない（書き手が理由を添えて断る。1 本のパスは今までどおり入る）。

### 版 28（0.5.0 の効果）

フィルターの段の種類 70〜79（ヒストグラムスキャン・ヒストグラムレンジ・スロープぼかし・方向のぼかし・ゆがみ・モルフォロジー・エッジ検出・ハイパス・メディアン・グロー）と、
Generator の種類 66 模様・67 アイランドごとのばらつき・68 ライト・69 マスクの組み立て・70 画像を追加した版。種類ごとの欄は段・Generator の `effect` の塊に置き、共通の欄（`radius` など）は既定のまま書く。
アイランドごとのばらつき（67）は、モデルの UV アイランドの番号とシードからアイランドごとに 1 つの値を出す。画像（70）はアセットの画像を、塗りつぶしレイヤーの画像と同じ投影（UV・トライプラナー・平面・球・円柱）で読み、
フィルターのスタックの段の値にする。投影は塗りつぶしレイヤーの `projection` と同じ並びをそのまま入れる。

選んだ理由と、採らなかった案:

- **使う文書だけ版を上げる（採用）。** 版 22〜25 と同じ流儀で、この種類を使わない文書は前の版のまま。版 25 までの読み手（スタンドアロン 0.4.x）は
  「`.version の値 28 は未対応または範囲外です`」（分けた正本なら「分けた正本の中の版 28 は未対応です」）、Unity 0.2.0 は版の数だけで
  「Unsupported archive version; source retained unchanged.」と、ファイルに触れずに断る。
- **共通の欄（`radius`・`amount`・`threshold`・`seed`）に種類ごとの値を詰める。** 欄の意味が種類で変わり、範囲の検査（`radius` は種類ごとに上限が違う・`amount` は 0〜5）を
  種類ごとに書き分けることになる。実数の長さ（スロープぼかし・ゆがみ・方向のぼかし）は int の `radius` に入らない。
- **色調補正の `adjust` の塊に入れる。** `adjust` は調整レイヤーの `detail` と同じ並び（調整レイヤーと共有の種類 64〜69）で、フィルターにしか無い種類を混ぜると調整レイヤーの
  種類の番号と食い違う。
- **投影の欄を画像の段のために作り直す。** 塗りつぶしレイヤーの `projection` と同じ並びなら、読み手の検査（範囲・置き場）と書き手を 1 つにでき、塗りつぶしレイヤーと
  画像の段で同じ設定が同じ位置を読む。違う並びにすると、片方だけ直したときに食い違う。
- **デカールを許す。** デカールは箱の外に値が無く、縁のやわらかさと裏向きの消え方を見える度合いに掛けるので、段の値（0〜1 の値か画素の色）と意味が合わない。
  デカールは塗りつぶしレイヤーで置く。
- **画像の画素を正本に入れる。** 画像はアセット（`resources.json`）に 1 度だけ入り、塗りつぶしの画像と同じく ID で指す。同じ画像を読む段が増えても .ylp は大きくならない。

移行: 版 1〜27 はそのまま読める（版 28 の正本は版 27 の中身（パスの一覧）も読み書きできる）。これらの段を消して保存し直すと、使う機能に応じて 21〜25・27 に戻る。継ぎ目の設定（版 32）を切った文書は、これらの段を持てる版 32 のまま
（版 32 の正本は版 28 の中身も読み書きできる。書き手は継ぎ目の設定を切っていれば 32、切っていなくてこれらの段があれば 28 で書く）。

保証の射程と失うもの:

- 1 つの段でも種類 70〜79・Generator の 66〜70 を使えば、その .ylp 全体をスタンドアロン 0.4.x と Unity 0.2.0 は開けない。
- アイランドごとのばらつき（67）の値はアイランドの番号で決まるので、同じ文書でもモデル（三角形の並び・UV）を替えるとアイランドの番号が変わり、値も変わる。アイランドの図は保存しないので、
  開き直したときにモデルが無ければ入力のまま通す（設定は残る）。
- 式は Rust の式（`yolu_core::filter`。スロープぼかし・ゆがみが読む内蔵の値ノイズは Generator のノイズと同じ hash）で、Substance Painter などの同じ名前のフィルターと
  画素まで一致するとは言わない。式を変えると既存の文書の見た目が変わるので、変えるときは版を追加して古い式を残す。
- 読み方は塗りつぶしレイヤーの画像の読み方（ミップマップ・投影の式）で、色のチャンネルでは同じ画像・同じ投影の塗りつぶしレイヤーと同じ画素になる
  （`crates/yolu-core/tests/effects/image_stage.rs`）。マスク・スカラーの輝度は補間した後の RGB から丸めずに求めるので、補間がかかる投影では、塗りつぶしレイヤーが
  スカラーのチャンネルで読む輝度（元の画素ごとに 8 bit へ丸めてから補間）と少し違う。読み方の式を変えると両方の見た目が変わる。
- .ylsmart（形式 1）には入れない（`REFUSAL_NEW_FILTERS`・`REFUSAL_IMAGE_GENERATORS`。版 24 の色調補正と同じ断り）。

### 版 29（点のグラデーション・画像の読み方・続きの属性の印）

塗りつぶしレイヤーのチャンネルごとの点のグラデーション（モデルの空間か UV の空間の点の色を、逆距離の 2 乗の重みで混ぜる）と、塗りつぶしの画像ごとの異方性の
フィルターの入・切（既定は入。切った画像だけ `false`）を追加した版。レイヤーの属性の印（1 バイト）はビット 6 まで使い切ったので、ビット 7 を「続きの属性の印
`attributes_ext`（int）が続く」にし、点のグラデーションはその続きの印のビット 0 に置く。続きの印はロックの直後に書き、0 なら書かない。

選んだ理由と、採らなかった案:

- **続きの属性の印を int で追加する（採用）。** 属性の印の空きはビット 7 だけで、点のグラデーションに使うと次の機能が置けない。int の続きの印なら
  31 個のビットが残り、使わないレイヤーは今までどおり 1 バイトも増えない。
- **レイヤーの種類ごとに同じビットの意味を変える。** 点のグラデーションは塗りつぶしレイヤーだけなので、ほかのレイヤーの機能とビット 6 を分け合える。ただ、パスの一覧は
  ラスターと塗りつぶしの両方のレイヤーで使うので分け合えず、読み手の検査も種類ごとに分かれて食い違いやすい。
- **レイヤーごとに bool を追加する。** 使わないレイヤーまで 1 バイトずつ増える。

移行: 版 1〜28 はそのまま読める（版 29 の正本は版 27・28 の中身も読み書きできる）。点のグラデーションを消し、画像の異方性を入に戻して保存し直すと、
ほかの機能で決まる版に戻る。版 32 の正本は版 29 の中身も読み書きできる。

保証の射程と失うもの:

- 1 つのセットでも点のグラデーションか異方性を切った画像を使えば、その .ylp 全体をスタンドアロン 0.4.x と Unity 0.2.0 は開けない（版の数だけで断る）。
- .ylsmart（形式 1）には入れない（`REFUSAL_POINT_GRADIENTS`）。
- 版 29 の正本は、画像ごとに `anisotropic` の bool を書く（入のままの画像も書く。版 28 までは書かない）。

### 版 30（テキストレイヤー）

テキストレイヤーを追加した版。ラスターのレイヤーにテキストの値（文・フォント・サイズ・色・行間・字間・揃え・位置・回転・折り返しの幅）を持たせ、レイヤーの Color の
画素はその値から描いた結果としてほかのレイヤーと同じタイルで書く。値はレイヤーの末尾（パスの一覧の後）の `text` の塊に置き、続きの属性の印（`attributes_ext`）の
ビット 1 で続くことを示す。

選んだ理由と、採らなかった案:

- **使う文書だけ版を上げる（採用）。** 版 22〜25 と同じ流儀で、テキストレイヤーの無い文書は前の版のまま。版 25 までの読み手（スタンドアロン 0.4.x）は
  「`.version の値 30 は未対応または範囲外です`」（分けた正本なら「分けた正本の中の版 30 は未対応です」）、Unity 0.2.0 は版の数だけで
  「Unsupported archive version; source retained unchanged.」と、ファイルに触れずに断る。
- **レイヤーの種類はラスター（`kind` 0）のまま、テキストの値を追加する（採用）。** パスのレイヤー（版 8・10）と同じ形で、画素はいつもタイルにあるので、テキストを知らない
  処理（合成・書き出し・PSD）も今のレイヤーと同じに扱える。種類の値（4 など）を追加すると、Unity 版のレイヤーの種類と同じ番号の意味が分かれるおそれがあり、
  番号を空けておける。失うもの: 種類の値だけではテキストレイヤーを見分けられない（続きの属性の印のビット 1 を見る）。
- **フォントのファイルを入れない（採用）。** フォントの許諾は、文書に埋めて配ることを許すとは限らない。文書は名前（同梱のフォント）か、道・中身の SHA-256・
  ファミリー名・PostScript 名・太さ・斜体か（ファイルのフォント）だけを持つ。失うもの: フォントの無い所で開くと、テキストの値は編集できない（画素はそのまま見え、
  書き出しも変わらない）。
- **名前と太さも持つ（採用）。** 道と SHA-256 だけでは、別の PC（フォントのフォルダが違う）で同じフォントを見つけられない。PostScript 名は 1 つのスタイルを
  指し、無いフォントや重なる名前のためにファミリー名と太さ・斜体かも持つ。スタイルの名前（name の 17・2 番）は言語で変わり、探す手がかりにしにくいので持たない。
- **画素も保存する（採用）。** 開くときに描き直すと、フォントの無い所で何も見えなくなり、並べ・塗りの式を変えたときに古い文書の見た目が変わる。
  失うもの: テキストレイヤーの分だけファイルが大きい（文字の覆う所のタイルだけ）。
- **文の上限を 4096 バイトにする。** 正本の文字列の上限（4096 バイト）と同じにして、文字列の欄の読み書きを今のままにした。

移行: 版 1〜29 はそのまま読める（版 30 の正本は版 27〜29 の中身も読み書きできる）。テキストレイヤーをラスタライズ（値を外す）か消して保存し直すと、
ほかの機能で決まる版に戻る。版 32 の正本は版 30 の中身も読み書きできる（継ぎ目の設定を切った文書のテキストレイヤーは版 32 で書く）。

保証の射程と失うもの:

- 1 つのセットでも版 30 なら、その .ylp 全体をスタンドアロン 0.4.x と Unity 0.2.0 は開けない。開く必要があるときは、テキストレイヤーをラスタライズして保存する。
- 描いた画素はこのアプリの並べ・塗りの式で、ほかのアプリの同じフォント・同じサイズの文字と画素まで一致するとは言わない。
- .ylsmart（形式 1）には入れない。スマートマテリアルにしたテキストレイヤーは画素だけになる（知らせる）。
- PSD の書き出しではテキストレイヤーを画素のレイヤーとして書く（PSD のテキストレイヤーは書かない）。PSD の取り込みのテキストレイヤーは今までどおり画素で読む。

### 版 32（継ぎ目の設定）

文書の設定「フィルターが UV の継ぎ目をまたぐ」（`filter_seams`）。入のとき、モデルの UV の位相がある文書では、レイヤーのフィルターの近傍の段（ぼかし・シャープなど）が、
アイランドの外の帯を継ぎ目の相手のアイランドの画素で埋めてから評価し、アイランドの外は段の入力のまま戻す。合成の画素が変わるので、正本の頭の欄にした。既定は入で、入の文書は
版を上げない（版 21〜25・27・28 のまま）。切った文書だけが版 32 になる。

選んだ理由と、採らなかった案:

- **切った文書だけ版を上げる（採用）。** 既定の入は欄を書かずに表せるので、使わない文書の版は変わらない。古い読み手（0.4.x・Unity 0.2.0）はもともと
  継ぎ目をまたがないので、入の文書を開いても読み違えは無い（見た目は 0.4.x の評価になる）。
- **状態のエントリ（`look.json` のような別のエントリ）にする。** 古い読み手が開けるようにはなるが、合成の画素を変える設定を、Unity 版が保存で落とす所に
  置くことになる。落ちると切った文書が黙って入に戻り、見た目が変わる。
- **レイヤーごと（フィルターの段の欄）に持つ。** モデルの継ぎ目は文書に 1 つなので、レイヤーごとに違えると、同じ縁でレイヤーによって継ぎ目が出る。段の欄を増やすと、
  使わない文書のフィルターの並びも変わる。

移行: 版1〜28 はそのまま読み、継ぎ目の設定は入。設定を入に戻して保存し直すと、ほかの機能で決まる版（21〜25・27・28）に戻る。版 32 の中身は版 28（0.5.0 の効果）の
並びを含むので、版 28 の段のある文書は継ぎ目の設定を切るだけで 28 から 32 へ上がる。

保証の射程と失うもの:

- 1つのセットでも設定を切っていれば、その .ylp 全体を 0.4.x のスタンドアロンと Unity 0.2.0 は開けない（版の数だけで、ファイルに触れずに断る）。
- 欄は設定だけで、帯の写し（モデル・解像度ごとに作り直せる）は書かない。モデルが無い文書では、入でも評価は 2D のまま。

### 版 33（ベイクの優先）

重なった UV（同じテクセルを 2 つ以上の三角形が覆う）で、どの三角形がテクセルを焼くかの決め方（`bake_priority`）。テクスチャセットごとの文書の設定で、
決め方で焼く値が変わり、手で選んだアイランド（三角形の番号）はモデルの形に結び付くので、正本の頭の欄にした。既定（番号の小さい三角形・外さない・手で選んだアイランドなし）は
欄を書かずに表せ、既定の文書は版を上げない（版 21〜25・27〜30・32 のまま）。変えた文書だけが版 33 になる。

選んだ理由と、採らなかった案:

- **変えた文書だけ版を上げる（採用）。** 0.4.x と Unity 0.2.0 の読み手はもともと番号の小さい三角形で焼くので、既定の文書は古い読み手で開いても意味が同じ。
- **状態のエントリ（`look.json` のような別のエントリ）にする。** 古い読み手が開けるようにはなるが、焼く値を変える設定を Unity 版が保存で落とす所に置くことになり、
  落ちると決め方が黙って既定に戻る（焼き直すと値が変わる）。
- **焼いたメッシュマップの由来の鍵だけで表す。** 鍵には決め方の SHA-256 だけが入り（`;owner=…;outside=skip;islands=<SHA-256>`、既定は何も追加しない）、
  手で選んだアイランドの番号は戻せない。マップを消すと設定も消える。
- **アイランドを UV の座標で覚える。** UV をぴったり重ねたミラーの両側は UV の座標で区別できない。三角形の番号とモデルの指紋（`MeshBakeInput` のトポロジーの
  SHA-256）で覚え、指紋の違うモデルではベイクの前に断る（黙って別のアイランドを外さない）。

移行: 版 1〜32 はそのまま読み、ベイクの優先は既定。既定に戻して保存し直すと、ほかの機能で決まる版（21〜25・27〜30・32）に戻る。版 33 の正本は版 27〜32 の中身も読み書きできる。

保証の射程と失うもの:

- 1つのセットでも決め方を変えていれば、その .ylp 全体を 0.4.x のスタンドアロンと Unity 0.2.0 は開けない（版の数だけで、ファイルに触れずに断る）。
- 手で選んだアイランドは、三角形の数・UV・スロット・レンダラーが同じモデルでだけ使える。モデルを作り直したら、一覧を外して選び直す。
- 焼いたマップの値の計算（法線・AO など）は変えない。変わるのは、重なったテクセルをどの三角形の値にするかと、焼かないアイランドのテクセルが空になることだけ。

### 手動の ID の色（版 19）を書く

塊の形は、Unity 版の `DocumentBinary`（`WriteIdColors`・`ReadIdColors`）が決めたもので、このアプリの書き手は C# の書き手が作った見本（`native-rich-v21.utpaint`）を読んで書き戻すと同じバイト列になる。

選んだ理由と、採らなかった案:

- **色があるときだけ塊を書き、版は上げない（採用）。** 塊は版 19 から置けて、書き手の版（21 以上）はどれも 19 以上なので、色のために版を追加する必要が無い。色だけを持つ文書は版 21 のままで、Unity 版が読める。全部外すと塊も消え、色を持たなかったときと同じバイト列に戻る。
- **新しい版の番号を使う。** 色を使うだけで、Unity 版が文書ごと開けなくなる。塊の形と版 19 は仕様にあり、Unity 版は読み書きできるので、追加する必要が無い。
- **指紋が今のモデルと合わない色を、読むときに捨てる・読むのを断る。** モデルがまだ読み込まれていない間は照合できず、別のモデルを開いて保存し直しただけで色が消える。読み手は書いてあったとおり文書に戻し、使う側が照合する（合わなければ、色の編集と ID マップのベイクを断る）。
- **色を正本の外のエントリに置く。** 色は文書の一部（Undo の対象で、ID マップを焼く条件）で、`document.utpaint` の版 19 に決まった置き場がある。別のエントリにすると、Unity 版が保存で落とす。

保証の射程と失うもの:

- 色は塊の番号（メッシュの塊の連番）で結ばれる。モデルの塊の数や並びが変わると、指紋が変わって別のモデルのものとして扱う（番号を使って色を別の塊へ当てはめない）。
- 色は 4096 個まで、塊の番号は 0〜3999999、色は 0xRRGGBB。範囲の外の値は、書き手に届く前に文書が断り、読み手も断る。
