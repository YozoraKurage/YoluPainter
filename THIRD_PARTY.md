# 第三者の許諾

Windows MSVC・Windows GNU・Linux GNU・macOS（universal）向けの `yolu-app`（スタンドアロン、更新依存とコマンドライン `yolu-cli` の依存を含む）と
`yolu-cli`（Claude Desktop 用の `.mcpb`。アプリと同じビルドでの部分木）、開発用 `xtask` の依存一覧。既定の機能、下表に記録した Cargo.lock が対象。
依存を更新したときや別のターゲット・機能で配るときは、一覧と全文束を更新する。
各対象の依存と、Linux の配布を止めている条件を分けて記載する。

`tools/third-party.py` は Cargo の通常依存とビルド依存を製品別にたどり、試験用依存を除く。
手続きマクロとビルド依存も保守的に全文束へ含める。「実行時」は通常依存の到達範囲であり、
最適化後のバイナリにそのクレートの全コードが残るという意味ではない。
`tools/licenses-reviewed.json` に、版・宣言された許諾・選択する許諾・原文の場所・SHA-256 を固定してある。
クレートに原文が無い場合は、そのクレートの発行時コミットにある上流の原文を取得する。
単に同じ種類の一般的な許諾文で代用せず、著作権表記と NOTICE も保持する。
クレートにも上流にも許諾の本文が無い場合だけ、上流の記載（`Cargo.toml` の `license` と `authors`）から組み立てた MIT の文を `tools/license-texts/` に置いて固定する。
その文の頭に、上流の原文ではないことと、使った記載を書く。年は、上流の記載に無いので書かない。

生成方法は [開発用の手順](https://github.com/YozoraKurage/YoluPainter/blob/main/docs/DEVELOPMENT.md#配布用の許諾全文) を参照。
一覧の JSON と Markdown、成功した製品の `THIRD_PARTY_LICENSES.txt` は `target/third-party/<target>/<クレート>/` にできる（`--target` を省くと Windows GNU 用を `target/third-party/<クレート>/` に出力）。
生成物は Git に入れない。この文書は確認済みの依存構成の記録で、配布には生成した全文を同梱する。

## 適用する許諾と配布時の表記

- `clipboard-win 5.4.1` と `error-code 3.4.0` は **BSL-1.0（Boost Software License）**。
  `egui-winit → arboard → clipboard-win → error-code` から入る。
  出典: [clipboard-win の原文](https://github.com/DoumanAsh/clipboard-win/blob/3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342/LICENSE)、
  [error-code の原文](https://github.com/DoumanAsh/error-code/blob/e4615e514db3ff64f5f1b328b4865b20a3dcdbc3/LICENSE)。
- `yolu-app` は OS のクリップボードの画像のために `arboard 3.6.1` を `image-data` と `wayland-data-control`（Wayland の data-control を先に試し、
  使えなければ X11）で直接使う。`arboard` 自体は `egui-winit` 経由で入っていたもので、増えるのは Linux だけの `wl-clipboard-rs 0.9.4`
  （MIT OR Apache-2.0 から MIT）と、その依存の `os_pipe`・`tree_magic_mini`・`nom`（いずれも MIT）、`petgraph`・`fixedbitset`・
  `hashbrown 0.15.5`（MIT OR Apache-2.0 から MIT）、`foldhash 0.1.5`（Zlib）。Windows の依存は変わらない。`tree_magic_mini` の GPL のデータ
  （別クレート `tree_magic_db`、`with-gpl-data` 機能）は有効にしていない。有効にすると GPL が入るので、機能を有効にしないこと。
- `yolu-app` は画面の並び（ドックのタブの組・分け方・ウィンドウの大きさ）の保存のために、`egui_dock` の `serde` 機能を有効にしている。
  `egui`・`epaint`・`emath`・`ecolor`・`accesskit` などが既に使っている `serde` を使うだけで、増えるクレートは `accesskit` の `serde` 機能が引く
  手続きマクロの `enumn 0.1.14`（MIT OR Apache-2.0 から MIT。原文は `LICENSE-MIT`）の 1 件だけ。
- egui の標準フォントに含まれる Hack の原文には **Bitstream Vera** の条件もある。
  OFL-1.1・Ubuntu Font Licence とともに全文を保持する。
- `yolu-io` は OS に入っているフォントの一覧（テキストレイヤーのフォントの選び）のために `fontdb 0.24.0`（MIT）を `std`・`fs`・`memmap` の機能で使う（`default-features` は切）。
  OS のフォントのファイルを読むだけで、OS のフォントは同梱しない。同梱するフォントは下の BIZ UDPGothic と、egui の標準フォント（上の表）。
- `yolu-app` は Live Link の元の絵（JPG）を読むために `image` の `jpeg` 機能を有効にしている。増えるのは `zune-jpeg 0.5.15` と `zune-core 0.5.3`
  （どちらも MIT OR Apache-2.0 OR Zlib から MIT。原文はクレートの `LICENSE-MIT`）の 2 件で、どちらも Cargo.lock には前から（試験の依存の `tiff` 経由で）あった。
  2 件の全ソースから GPL・LGPL・AGPL の表記を検索して、該当は無かった。TGA の読み（`tga` 機能）は `image` の中だけで、クレートは増えない。
- 上記の発行時コミットの原文と SHA-256 を照合し、**Windows MSVC・Windows GNU・macOS（universal）は app のクレート分の配布用全文束を生成できる**。
  未確認の版や原文の変更を自動承認せず、依存を更新したら再確認する。
- macOS（universal）の配布物にだけ入る依存 32 件（`objc2` 系・`block2`・`dispatch2`・`dispatch`・`core-foundation` 系・`core-graphics` 系・`foreign-types` 系・`accesskit_macos`・
  `accesskit_consumer 0.38.0`・`raw-window-metal`・`wgpu-core-deps-apple`・`tiff`・`weezl`・`fax`・`quick-error 2.0.1`・`bitflags 1.3.2`）を `tools/licenses-reviewed.json` に登録した。
  許諾は MIT（`OR` の組は MIT を選ぶ）。原文は、クレートに同梱の物（15 件）か、そのクレートの発行時コミットの上流の物（17 件）を SHA-256 で固定している（うち 11 件は、下の組み立てた文を先頭に足している）。
  `objc2` の 0.6 系の 10 件（`objc2`・`block2`・`dispatch2`・`objc2-app-kit`・`objc2-core-foundation`・`objc2-core-graphics`・`objc2-encode`・`objc2-foundation`・`objc2-metal`・`objc2-quartz-core`）と
  `dispatch 0.2.0` の計 11 件は、クレートにも上流のリポジトリにも許諾の本文のファイルが無い（上流の `LICENSE.md` は許諾の種類と、Apple の SDK から作ったバインディングであることの説明で、
  著作権の行も MIT の本文も無い。`dispatch` は上流のどこにも許諾の本文が無い）。そこで、上流の記載から組み立てた MIT の文（本文は MIT の標準の文、著作権の行は `Cargo.toml` の `authors`）を
  `tools/license-texts/` に置き、全文束の先頭に載せる。作者名は `objc2`・`block2`・`objc2-encode` が Mads Marquart、`dispatch2` が Mads Marquart と Mary、`dispatch` が Steven Sheldon。
  `objc2-foundation`・`objc2-app-kit`・`objc2-core-foundation`・`objc2-core-graphics`・`objc2-metal`・`objc2-quartz-core` は `Cargo.toml` に `authors` が無いので、同じリポジトリの `objc2` の `authors`（Mads Marquart）を使う。
  上流の `LICENSE.md`（`dispatch` は `Cargo.toml`）は、組み立てた文のあとに並べて固定している。
  `objc2` の 0.5 系（`objc2`・`block2`・`objc-sys`・`objc2-app-kit`・`objc2-foundation` の古い版）は、上流の `LICENSE.txt`（MIT の本文）を固定している。
- `self_cell` の宣言は `Apache-2.0 OR GPL-2.0-only`。選択するのは Apache-2.0 であり、GPL の条件は選択しない。
  `Unlicense OR MIT` も MIT を選択する。AND の条件はすべて残す。
- `bevy_mikktspace 1.0.0`（3D ビューの法線マップの接線）は **Zlib AND (MIT OR Apache-2.0)**。MikkTSpace の参照実装（Morten S. Mikkelsen）を
  Rust に書き直したもので、Zlib の注意書きは独立した許諾ファイルが無くクレートの `src/lib.rs` の冒頭にある。MIT の `LICENSE-MIT` とともに
  その原文（`lib.rs`）を全文束へ含める。選ぶのは MIT で、Zlib の条件（出所を偽らない・改変を明示する・注意書きを消さない）は残す。
- `yolu-ops`（外から文書を操作する命令の型と、画面なしのホスト。`yolu-cli` とアプリが使う）は、命令・返事・誤りの JSON Schema を型から作るために
  `schemars 1.2.2`（MIT。原文はクレートの `LICENSE`）を使う。増えるクレートは `schemars_derive 1.2.2`（手続きマクロ。MIT）と、`serde_derive_internals 0.30.0`・
  `dyn-clone 1.0.20`・`ref-cast 1.0.27`・`ref-cast-impl 1.0.27`（いずれも MIT OR Apache-2.0 から MIT。原文は `LICENSE-MIT`）の計 6 件で、どれも各クレートに同梱の原文を
  SHA-256 で照合する。アプリは外からの操作を受けるために `yolu-ops` を直接使い、同じ配布物に入る `yolu-cli` も使うので、この 6 件は `yolu-app` の一覧に入る。
- `yolu-cli`（コマンドライン `yolupainter-cli` と、標準入出力の MCP を起動中のアプリへつなぐ中継。アプリと同じ配布物・同じ版で配り、Claude Desktop 用の `.mcpb` にも入る）と
  `yolu-mcp` は、MCP のために公式の Rust SDK `rmcp 3.5.1` と `tokio`（MIT。標準入出力・実行基盤・127.0.0.1 の通信の機能だけ）を使う。`rmcp` の宣言は **Apache-2.0** で、クレートの配布物には許諾の全文が無い。MCP の project は MIT から
  Apache-2.0 へ移行する途中で、再許諾の同意を得ていない寄稿は MIT のままなので、上流の `LICENSE`（Apache-2.0 の全文・MIT・文書だけに適用する CC-BY-4.0 の注記）を、
  発行時の commit（`79437f291b2c44053d00dcd5db969fd0cca7c887`）の固定 URL から取り、SHA-256 で照合して Apache-2.0 と MIT の両方を選ぶ（CC-BY-4.0 は文書だけで、コードには適用しない）。
  増えるクレートは `rmcp` と、`rmcp` が引く `pastey 0.2.3`（MIT OR Apache-2.0 から MIT）、`tokio 1.53.2`・`tokio-macros 2.7.2`・`tokio-util 0.7.19`・`bytes 1.12.1`（いずれも MIT）、
  `futures`・`futures-channel`・`futures-executor`・`futures-sink`（0.3.34。MIT OR Apache-2.0 から MIT）、`chrono 0.4.45`（MIT OR Apache-2.0 から MIT。原文は `LICENSE.txt`）の 11 件。
  `getrandom 0.4.3`（MIT OR Apache-2.0 から MIT）は、試験専用から通常の依存に移る（`rmcp` が引く `uuid` が使う。登録済み）。いずれも各クレートに同梱の原文を SHA-256 で照合し、
  クレートの全ソースから GPL・LGPL・AGPL の表記を検索して、該当は無かった。`rmcp` の `macros`・`client`・OAuth の機能は使わないので、`reqwest` などの通信の部品は入らない。
- `yolu-mcp`（MCP の中身と、127.0.0.1 の HTTP の受け口・客。アプリが受け口を動かし、`yolu-cli` が中継と起動中のアプリへの命令に使う）は、`rmcp` の
  Streamable HTTP の受け口の機能（`transport-streamable-http-server`）と、それを載せる `hyper 1.12.0`（`server`・`client`・`http1` の機能だけ。HTTP/2 は入れない）・
  `hyper-util 0.1.21`（`tokio` の機能だけ）・`http-body-util 0.1.5`（いずれも MIT）を使う。増えるクレートは、`hyper`・`hyper-util`・`http-body-util`・`http-body 1.1.0`・
  `tokio-stream 0.1.19`・`tower-service 0.3.3`・`try-lock 0.2.5`・`want 0.3.2`・`mio 1.2.4`（いずれも MIT。原文は `LICENSE`）と、`http 1.5.0`・`httparse 1.10.1`・
  `httpdate 1.0.3`・`socket2 0.6.5`・`sse-stream 0.2.6`・`base64 0.23.1`・`rand 0.10.3`・`rand_core 0.10.1`・`chacha20 0.10.2`・`cpufeatures 0.3.1`（いずれも MIT OR Apache-2.0
  から MIT。原文は `LICENSE-MIT`、`rand`・`rand_core` は `COPYRIGHT` も）の 19 件。`rand` 以下の 4 件と `sse-stream`・`base64 0.23.1` は `rmcp` の HTTP の受け口の機能が、
  `mio`・`socket2` は `tokio` の `net` の機能が引く。いずれも各クレートに同梱の原文を SHA-256 で照合し、クレートの全ソースから GPL・LGPL・AGPL の表記を検索して、該当は無かった。
  TLS の部品は入らない（待つのも客も 127.0.0.1 の平文の HTTP だけ）。
- exe のバージョン情報（アイコン・製品名・版）を埋めるビルド用の `winresource`（MIT）は、実行ファイルには入らず、ビルド依存として全文束に含める。
- 更新の通信は OS の部品を呼ぶだけで、通信の部品は同梱しない。Windows は OS 付属の WinHTTP、Linux は利用者の環境の `curl` を呼ぶ。
- Windows のインストーラーは NSIS 3（zlib/libpng 許諾）で作る。作ったインストーラーには NSIS の実行時の部品（stub）が入り、
  圧縮方式ごとの部品（LZMA・bzip2・zlib）にはそれぞれの許諾と NSIS の例外が付く。上流の表記は
  [NSIS の許諾](https://nsis.sourceforge.io/NSIS_License) を参照。インストーラーに入れるファイルは zip と同じで、全文束も同じ物を入れる。
- 自作部分の配布許諾、Rust 標準ライブラリ、実際にリンクする MinGW/GCC ランタイム、追加で同梱する DLL の表記は、
  最終的な配布物と使用ツールチェーンに合わせて別途確認する。このクレート一覧だけで製品全体の配布可否は確定しない。

## フォントとアイコン

`epaint_default_fonts 0.36.2` の原文は
[発行時の fonts ディレクトリ](https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint_default_fonts/fonts) にある。
フォントを加工せず同梱する前提で、次の全文を収集する。

| フォント | 許諾と原文 |
|---|---|
| Noto Emoji | OFL-1.1、`fonts/OFL.txt` |
| Ubuntu Light | Ubuntu Font Licence 1.0、`fonts/UFL.txt` |
| Hack | MIT、DejaVu のパブリックドメインの注記、Bitstream Vera、`fonts/Hack-Regular.txt` |
| emoji-icon-font | MIT、`fonts/emoji-icon-font-mit-license.txt` |

標準フォントの OFL-1.1 と Ubuntu Font Licence は、各フォントの著作権・名称・条件を原文のまま全文束に含める。
フォントを変更して配る場合は、予約されたフォント名などの条件を再確認する。
Fluent UI System Icons と Phosphor Icons は MIT。
[既存のアイコンの表記](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-app/assets/icons/THIRD-PARTY-NOTICES.md) も app の全文束に含める。
アイコン 118 個は下表のクレート件数には含めない。各 PNG と Fluent・Phosphor の元の名前・太さの対応は上の表記に記載する。
Phosphor はブラシ・グラデーション・長方形選択・楕円選択・多角形選択・選択ペン・ID 選択・アンカーと、スナップのアイコンの磁石に使っている。

### 同梱の画面のフォント（BIZ UDPGothic）

`yolu-app` は画面のフォントとして **BIZ UDPGothic**（Regular と Bold。モリサワの BIZ UD ゴシックを Google Fonts が公開したもの）を実行ファイルに
埋め込み、全 OS で同じフォントを使う。許諾は **SIL Open Font License 1.1**。
配布元は [googlefonts/morisawa-biz-ud-gothic](https://github.com/googlefonts/morisawa-biz-ud-gothic) のリリース v1.051
（コミット `18934af56b9c003ca58c54bffbf226848cb11032`）の TTF で、Regular と Bold は加工せずに `crates/yolu-app/assets/fonts/` へ置く。
許諾の全文は同じ場所の `OFL.txt`（配布元の同じコミットの原文）と
[フォントの第三者表記](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-app/assets/fonts/THIRD-PARTY-NOTICES.md) に保持し、app の全文束にも含める。フォントはクレートではないので、上の件数には含めない
（アイコンと同じ扱い）。

| 同梱ファイル | SHA-256 |
|---|---|
| `BIZUDPGothic-Regular.ttf` | `595dfefa96dcb281c40f5d560841f3fc623b0d9d12064481a7daf78044462673` |
| `BIZUDPGothic-Bold.ttf` | `3a1adc5c062064d4e6fe20df1368ffd52d6bf969d5e0ac90f4083da8cd5597b8` |
| `BIZUDPGothic-Regular-Lines.ttf` | `75f2740ab18211e3e79c3373c11e3e3b187389a9942d78f0fecb8cc08eb509a8` |
| `BIZUDPGothic-Bold-Lines.ttf` | `8287377180647c950182bb0d078e71753bf5a80fb06bdce9fa767d17499c1170` |
| `OFL.txt` | `e753d7155d53c747d037a445e584c8ecfca6dd79846db610417e282a736b28bc` |

`*-Lines.ttf` は、Regular と Bold から下線（_）・ダッシュ（– — ―）・マイナス（−）・上線（‾ ¯）など 9 字の輪郭だけを、ヒンティングの命令を外して抜き出した
派生のフォント（各約 2.4 KB）で、`tools/make-ui-font-lines.py`（fontTools 4.65.0）で作り直せる。原本の命令は、細い棒の上下の縁を別々に画素の格子へ丸め、
12 pt の `_` などを厚み 0 にして描かれなくするので、画面のフォントの先頭に置いて、この字だけを命令なしで描く。
`OFL.txt` の著作権表記に予約されたフォント名の宣言は無い。原本の `BIZ UDPGothic` は Morisawa Inc. の商標なので、派生の名前は `YoluPainter UI Lines` に変え、
著作権表記・許諾の文・商標の注記は name 表に残して、同じ SIL Open Font License 1.1 のまま同梱する。

フォント・アイコン・筆先の各ファイルと表記の SHA-256、許諾の種類は `tools/licenses-reviewed.json` の `bundled` に固定し、`tools/third-party.py` が app の照合のたびに確かめる
（確認済みファイルの変更・欠落、対象フォルダーへの未登録ファイルの追加では全文束を作らない）。フォントを加工して配る場合は、予約されたフォント名などの条件を再確認する。

`libz-sys` の Rust 側は MIT を選び、同梱 zlib の Zlib 許諾も含める。
`unicode-ident` の Unicode-3.0、
`tracing-core` 内の spin の MIT 原文も収集する。未使用の zlib contrib・zlib-ng は対象外。
Linux 専用の `rfd` バックエンドにある window_identifier の MIT 原文も保持する（共通設定のため Windows の束にも保守的に含める）。

## lilToon の再現（式の移植）

3D ビューの lilToon の見た目（`crates/yolu-app/src/view3d/shaders/liltoon/` の部品）と、見た目の欄のプロパティの名前・既定値
（`crates/yolu-app/src/look/liltoon.rs`）は、[lilToon](https://github.com/lilxyzw/lilToon) 2.3.4（**MIT**、Copyright (c) 2020-present lilxyzw）の
シェーダーとインスペクターから式と値を移したもの。lilToon のファイル・テクスチャは同梱しない。光の式は lilToon に同梱の
OpenLit Library 1.0.2（**CC0 1.0**）から移した。ラメの乱数は lilToon の `lilHashRGB4` からで、その元は Shadertoy の
「Simplest Fastest 2D Hash」（MdcfDj、**Unlicense**。原文の先頭の表記で確認）。MIT の原文と出どころは [再現の第三者表記](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-app/src/view3d/shaders/THIRD-PARTY-NOTICES.md)
にあり、`tools/licenses-reviewed.json` の `bundled` で照合して app の全文束に含める。クレートではないので、上の件数には含めない。

## グラデーションの混色（Oklab）

グラデーションマップの「知覚的」な混色（`crates/yolu-core/src/generator/mixing.rs`）は、色空間 Oklab（Björn Ottosson、2020）の sRGB との変換の式
（<https://bottosson.github.io/posts/oklab/>。著者は式と参照コードを **MIT**（Copyright (c) 2020 Björn Ottosson）で公開している）の係数を使う。
変換の行列の係数だけを使い、参照コードは移していない。クレートではないので、上の件数には含めない。輝度の補正の式（彩度の落ちた量に比例して明るさを持ち上げる）はこのアプリのもの。

## 生成・試験に使うツール

許諾全文の生成ツールは Python 3.10 以降の標準ライブラリだけを使い、cargo-about / cargo-deny / pip の追加パッケージは不要。
以下は開発環境で使うツールの許諾であり、ツール本体を製品へ同梱しない。

| ツール | 許諾・参照元 |
|---|---|
| Python | PSF License と付随する許諾。 [Python の表記](https://docs.python.org/3/license.html) |
| Rust / Cargo | 主に MIT OR Apache-2.0。 [Rust](https://github.com/rust-lang/rust/blob/master/COPYRIGHT)・[Cargo](https://github.com/rust-lang/cargo/blob/master/LICENSE-MIT) の第三者表記も参照 |
| Bash | GPL-3.0-or-later。 [Bash](https://www.gnu.org/software/bash/) |
| Wine | LGPL-2.1-or-later。使用環境の `/usr/share/doc/wine/copyright` と [上流の COPYING.LIB](https://gitlab.winehq.org/wine/wine/-/blob/master/COPYING.LIB) |
| MinGW-w64 の GCC・binutils | ツール本体は GPL 系。ランタイムは別の許諾・例外を持つため [MinGW-w64](https://www.mingw-w64.org/) と各インストールの copyright を確認 |

Wine 用の `bcryptprimitives.dll` はこのリポジトリの小さな接続コードから試験時だけ作る。
Windows 配布物に入れない。Wine や Windows の DLL をコピーして作るものではない。

## 同梱の筆先とブラシ形式

`crates/yolu-brush-sets/data/krita4/brushes/` は、Krita の `Krita_4_Default_Resources.bundle` の筆先 76 個（`.png`・`.gih`・`.gbr`）を
1 バイトも変えずに持つ。束の原本の `meta.xml`（許諾 **CC0 1.0** の宣言）と、取り込み時に作成した
`SHA256SUMS`・出どころと束の SHA-256 を書いた `README.md` は `crates/yolu-brush-sets/data/krita4/` にある。
作者の表記（CC0 では不要）: David Revoy（Deevad）と、Ramon Miranda・Razvanc・Radian・Wolthera・Storm・Scottyp ほかの派生、Krita プロジェクト。
Krita のプリセット（`.kpp`）・SVG の筆先・パターンは入れていない。このアプリでの設定（半径・間隔など）は独自のもの。
[筆先の第三者表記](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-app/assets/brushes/THIRD-PARTY-NOTICES.md) に出典・作者・元の許諾宣言と CC0 の全文をまとめ、app の全文束にも含める。
`tools/licenses-reviewed.json` で筆先 76 個・元の宣言・取り込み記録・表記のハッシュを照合する。
ロゴを除くアプリの同梱画像はこの筆先と前述のアイコンであり、試験の fixtures・snapshots は配布へ含めない。

ブラシ形式の読み手は公開された形式の説明から書いた。他のプロジェクトのコードは使っていない:
GIMP の `gbr.txt`・`gih.txt`・`vbr.txt`（devel-docs）、Photoshop File Formats Specification（版 1・2 のブラシ、記述子、パターン）、
版 6 以降の `samp`・`desc` の並びの公開された解説。

CLIP STUDIO PAINT の `.sut`（SQLite）の読み手も、公開された解析から表・列・BLOB の形という事実だけを取って書いた（コードは移していない）:
Kenneth Evans の VS-CSPBrushInfo（MIT。`Node`・`Variant`・影響元と素材の参照の BLOB）、Leon Schoenbrunn の CSP2PC（MIT。列の名前）、
Mari Paint の取り込みの注釈つきソースと調査ノート（Apache-2.0。影響元のヘッダー・素材の PNG の場所・列の別名）、Brushfactory の公開の説明
（`MaterialFile.FileData` が無圧縮 tar であること）。AGPL-3.0 の CSPBrushExtract、GPL-3.0 の変換器（Krita 向け）、許諾の無い実装は読んでいない。
分かった形と、本物のファイルで確かめていない所は [ブラシの取り込み](docs/BRUSH_IMPORT.md) に書く。

SQLite の読み取りには `rusqlite 0.40.2`（MIT）を `bundled`・`serialize`・`limits`・`hooks` で使う。`libsqlite3-sys 0.38.2`（MIT）が
SQLite 3.53.2 の amalgamation（`sqlite3.c`・`sqlite3.h`）を同梱してコンパイルする。**SQLite はパブリックドメイン**（著作権の放棄と祝福の文が
`sqlite3.c` の冒頭にあり、許諾の全文は無い）。このクレートには SQLCipher のソース（`sqlcipher/`）も入っているが、機能
`bundled-sqlcipher` は有効にしておらず、組み込まない。SQLite の拡張の読み込み（`load_extension`）も有効にしていない。実行時の依存は
`fallible-iterator 0.3.0`・`fallible-streaming-iterator 0.1.9`（どちらも MIT/Apache-2.0 から MIT）が増える。
SQLite の C のコンパイルでクリーンビルドが重くなる: この環境（gcc、最適化 3、1 コア）で約 40 秒。2 回目以降は再利用される。

| 確認したソース（`libsqlite3-sys 0.38.2` 同梱） | SHA-256 |
|---|---|
| `sqlite3/sqlite3.c`（SQLite 3.53.2） | `0a409f1633283fa31a9126b11fbfd64a1991c5d30defad07e5745d4667f5e23d` |
| `sqlite3/sqlite3.h` | `9e69a1353a4288450b0d5239ede11fc7f1f4c8e5eb07491fc8317eacb5b7de7e` |
| `rusqlite`・`libsqlite3-sys` の `LICENSE`（同一の MIT） | `c10c1f27337546471e5f7e4e97fdd398b35b9d4e126115dcd22de8d8e65abf6f` |

## FBX の読み込み（ufbx）

`yolu-model` が使う `ufbx 0.11.5` は `MIT OR Unlicense` から **MIT** を選ぶ。
クレートに LICENSE は含まれないため、`.cargo_vcs_info.json` が示す発行時コミット
`856c76840977cd504f053bf17ab4948b675b981a` の
[ufbx-rust の原文](https://github.com/ufbx/ufbx-rust/blob/856c76840977cd504f053bf17ab4948b675b981a/LICENSE) を使う。

同梱 C の ufbx 0.23.1 も MIT を選ぶ。上記コミットの
[sfs-deps.json.lock](https://github.com/ufbx/ufbx-rust/blob/856c76840977cd504f053bf17ab4948b675b981a/sfs-deps.json.lock) が指す
`48d2bb4114905c51347a91b8b6a276563daaa3f1` の
[C 本体の原文](https://github.com/ufbx/ufbx/blob/48d2bb4114905c51347a91b8b6a276563daaa3f1/LICENSE) を別の出典として収集する。
このコミットの `ufbx.c`・`ufbx.h` は、クレート同梱分および ufbx-rust の発行時コミットのファイルとバイト一致を確認した。

| 確認した原文・ソース | SHA-256 |
|---|---|
| ufbx-rust と C 本体の LICENSE（両者同一） | `0dd48ebadf52273c736256325c8f078c03c8bb4facee22a4122de0ad3f615391` |
| 同梱 `ufbx/ufbx.c` | `4a956c26a708e40d82ecb0aeaee54da709d5ba7c144c9642174970059bda7e1d` |
| 同梱 `ufbx/ufbx.h` | `f787be529af577efc04f3e3e735844a08556718dbb613d1211c2b313d72895dc` |

原文は代替の Unlicense も含む形のまま保持するが、選択する条件は MIT のみ。
Rust と C の著作権表記（2020 Samuli Raivio）を全文束に含め、C 本体を別クレートとして件数に重ねない。

## 配布対象別の照合

`cargo xtask bundle` と同じく app は `--include-update --include-cli` を付けた集合を記載する。
`yolu-update` はアプリの自動更新として組み込み済みで、その依存も app の一覧に含まれる。
`yolu-cli`（`yolupainter-cli`）は app と同じ配布物（zip・tar.gz・インストーラー）に入るので、その依存も app の一覧と全文束に含め、
`.mcpb` には `yolu-cli` の一覧と全文束（`--package yolu-cli --built-with yolu-app`）を入れる。実行ファイルは `-p yolu-app -p yolu-cli` の 1 回のビルドで作り、2 つのクレートの機能が合わさって、`yolu-cli` 単独の木に無い依存（`libm`・`foldhash`）が入るので、数えるのはそのビルドでの `yolu-cli` の部分木。
開発用 `xtask` の依存は後段に分ける。
同名でも別版のクレートは別件として数える。製品間・対象間の件数は重複する。

```sh
python3 tools/third-party.py --target x86_64-pc-windows-msvc --package yolu-app --include-update --include-cli --bundle
python3 tools/third-party.py --target x86_64-pc-windows-gnu --package yolu-app --include-update --include-cli --bundle
python3 tools/third-party.py --target x86_64-pc-windows-msvc --package yolu-cli --built-with yolu-app --bundle
# Linux の app は下記の条件が未承認のため終了 1。全文束を作らない。
python3 tools/third-party.py --target x86_64-unknown-linux-gnu --package yolu-app --include-update --include-cli --bundle
# macOS の universal は Apple Silicon と Intel の 2 つのターゲットの木の和（どちらか単独の aarch64-apple-darwin・x86_64-apple-darwin も指せる）。.mcpb は Windows だけなので cli の一覧は無い
python3 tools/third-party.py --target universal-apple-darwin --package yolu-app --include-update --include-cli --bundle
```

原文取得後は `--offline` でも同じ照合ができる。
配布物には対象ごとの `THIRD_PARTY.md` を `DEPENDENCIES.md`、全文を `THIRD_PARTY_LICENSES.txt` として同梱する。
`LICENSE`・この `THIRD_PARTY.md`・`README.md`・`README.en.md` と、使う人向けの `docs/`（日英。開発とリリースの手順は入れない）も同梱する。
zip・tar.gz とインストーラーは `xtask` の共通の梱包一覧を使い、NSIS でも同じ文書を入れる。[配布の手順](https://github.com/YozoraKurage/YoluPainter/blob/main/docs/RELEASING.md) と一致する。
別の対象の一覧を流用しない。結果は `target/third-party/<target>/<クレート>/` に出力する。

| 対象 | app（更新・コマンドライン込み） | cli（app と同じビルド） | update 単独 | xtask | app の全文束 |
|---|---:|---:|---:|---:|---|
| Windows MSVC | 251 | 132 | 31 | 49 | 生成成功 |
| Windows GNU | 251 | 132 | 31 | 49 | 生成成功 |
| Linux GNU | 319 | 131 | 31 | 54 | 判断待ち |
| macOS universal | 246 | — | — | — | 生成成功 |

署名検証に使う `ed25519-dalek`・`curve25519-dalek`・`subtle` は BSD-3-Clause。
更新・梱包用のクレートも含めて原文を照合し、未確認のクレートが無いことを確認した。

Linux の `wayland-protocols-plasma 0.3.12` と `wayland-protocols-misc 0.3.12` はクレートの宣言が MIT でも、
同梱 protocol XML に **LGPL-2.1-or-later** の条件がある。生成バインディングの配布条件について判断待ちであり、
`blocked` を維持する。以下の Linux 集計の MIT 件数にはこの 2 件も含まれるが、許可済みという意味ではない。
Linux app の全文束は生成せず、依存の変更・削除も行わない。update・xtask の照合は成功する。

## Cargo.lock 全体と試験専用の依存

`python3 tools/third-party.py --audit-lock --offline` は Windows MSVC・Windows GNU・Linux GNU・macOS（Apple Silicon と Intel）の workspace 全体を調べる。
通常・ビルド依存に加えて試験依存の原文も照合し、他 OS・無効な機能の依存を含む lock 全件を
`target/third-party/lock-inventory.json` に分類する。古い登録・未確認の版・未承認条件があれば終了 1。
配布用の照合とは別であり、対象外のクレートを承認済みとは扱わない。

照合した Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。外部クレート 496 件の内訳は次のとおり。

| 範囲（5 ターゲットの和集合、同名の別版は別件） | 件数 | 結果 |
|---|---:|---|
| 通常・ビルド依存 | 387 | 原文を照合。Linux の protocol XML 2 件は判断待ち |
| 試験専用 | 12 | MIT 11 件は原文を照合。MPL-2.0 1 件は未承認 |
| 対象外（他 OS・現在無効な機能） | 97 | 宣言と対象外であることを記録。配布用の原文照合・承認は行わない |
| Cargo.lock に無い古い登録 | 0 | 削除なし |

試験専用の一覧（配布用全文束には含めない）:

| クレート | 版 | 宣言 | 選択・状態 |
|---|---|---|---|
| anyhow | 1.0.104 | MIT OR Apache-2.0 | MIT |
| colored | 2.2.0 | MPL-2.0 | 未承認 |
| dify | 0.8.0 | MIT | MIT |
| egui_kittest | 0.36.2 | MIT OR Apache-2.0 | MIT |
| getopts | 0.2.24 | MIT OR Apache-2.0 | MIT |
| is-docker | 0.2.0 | MIT | MIT |
| is-wsl | 0.4.0 | MIT | MIT |
| kittest | 0.4.0 | MIT OR Apache-2.0 | MIT |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | MIT |
| open | 5.4.4 | MIT | MIT |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | MIT |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | MIT |

`egui_kittest → dify → colored 2.2.0` は MPL-2.0。許容一覧へ自動追加せず、原文と未承認理由だけを記録する。
現在の製品の依存には含まれない。上記の lock 監査はこの 1 件と Linux の 2 件で終了 1 となる。

全件のクレート同梱ファイルから GPL・LGPL・AGPL の表記も検索した（今の Cargo.lock の 496 件で、`gpl` の部分一致と `General Public License` を大文字小文字を区別せずに検索し、語としてのヒットは下に挙げた物だけだった。部分一致だけのヒットは `eggplant`・`SIGPLAN94`・`Placement`・`WindowsFilteringPlatform` など、別の語や識別子の一部）（`rusqlite` 系の 4 件と `schemars` 系の 6 件にはなく、`libsqlite3-sys` の `sqlite3.c` のヒットは単語 Eggplant）。Linux の既知の 2 件に加え、
対象外の `orbclient 0.3.55` の `res/unifont-license.txt` に GPL-2.0-or-later とフォント埋め込み例外がある。
現在の Windows・Linux の依存には含まれないが、このクレートを使う対象・機能を追加するときは判断が必要。
`self_cell` の GPL は Apache-2.0 と選択できる条件、対象外の `r-efi 5.3.0`・`6.0.0` の LGPL は MIT・Apache-2.0 と選択できる条件。
`colored` 原文の GPL の言及は MPL の Secondary License の定義であり、GPL を選択した記録ではない。
`x11rb 0.13.2` の `examples/record.rs` は GPLv3 のサンプルを元にした旨の表記があるが、製品で使うライブラリにこの examples は含めない。
ほかのヒットは LLVM 例外の GPLv2 への言及、`slotmap` の互換性の説明、`vcpkg` の試験用ファイル名一覧、
前述の `tree_magic_mini` の無効な `with-gpl-data` 機能と変更履歴である。
AGPL の表記は検出しなかった。文字列検索だけで許諾の不存在を保証するものではない。
依存構成と各クレートの blocked は解除していない。

## Windows MSVC の製品別一覧

### yolu-app（yolu-update と yolu-cli の依存を含む） の依存一覧

対象: `x86_64-pc-windows-msvc`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 251 件（同名の別版は別件）。実行時 207 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 | 11 |
| Apache-2.0 AND MIT | 2 |
| BSD-3-Clause | 3 |
| BSL-1.0 | 2 |
| ISC | 1 |
| MIT | 226 |
| MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 1 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 2 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| accesskit | 0.24.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_consumer | 0.35.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_windows | 0.32.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_winit | 0.32.2 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| ahash | 0.8.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| allocator-api2 | 0.2.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arboard | 3.6.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ash | 0.38.0+1.3.281 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bevy_mikktspace | 1.0.0 | 実行時 | Zlib AND (MIT OR Apache-2.0) | MIT AND Zlib | 確認済み |
| bit-set | 0.10.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-vec | 0.9.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| byteorder-lite | 0.1.0 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg_aliases | 0.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| clipboard-win | 5.4.1 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| codespan-reporting | 0.13.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cursor-icon | 1.2.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| curve25519-dalek-derive | 0.1.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| curve25519-dalek | 4.1.3 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| document-features | 0.2.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dpi | 0.1.2 | 実行時 | Apache-2.0 AND MIT | Apache-2.0 AND MIT | 確認済み |
| duplicate | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ecolor | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ed25519-dalek | 2.2.0 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| ed25519 | 2.2.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| eframe | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-wgpu | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-winit | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui_dock | 0.21.1 | 実行時 | MIT | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| emath | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| enumn | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint_default_fonts | 0.36.2 | 実行時 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| error-code | 3.4.0 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| gl_generator | 0.14.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glow | 0.17.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| glutin_wgl_sys | 0.6.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| gpu-allocator | 0.28.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| half | 2.7.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.16.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| heck | 0.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| hex | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| image | 0.25.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itertools | 0.15.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| khronos-egl | 6.0.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| khronos_api | 3.1.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libloading | 0.8.9 | 実行時 | ISC | ISC | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| litrs | 1.0.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| lock_api | 0.4.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| moxcms | 0.8.1 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| naga-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| naga | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| nohash-hasher | 0.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ordered-float | 5.5.0 | 実行時 | MIT | MIT | 確認済み |
| parking_lot | 0.12.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot_core | 0.9.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| paste | 1.0.15 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pollster | 1.0.1 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| presser | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2-diagnostics | 0.10.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| profiling | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pxfm | 0.1.30 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| range-alloc | 0.1.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| raw-window-handle | 0.6.2 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| renderdoc-sys | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rfd | 0.17.2 | 実行時 | MIT | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| rustc-hash | 1.1.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| rustc-hash | 2.1.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| rustc_version | 0.4.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| scopeguard | 1.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| self_cell | 1.3.0 | 実行時 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 | 確認済み |
| semver | 1.0.28 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_spanned | 1.1.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| signature | 2.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smol_str | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| spirv | 0.4.0+sdk-1.4.341.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| static_assertions | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| subtle | 2.6.1 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| toml | 1.1.6+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_datetime | 1.1.1+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_parser | 1.1.3+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| type-map | 0.5.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ufbx | 0.11.5 | 実行時 | MIT OR Unlicense | MIT | 確認済み |
| unicode-general-category | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| unicode-ident | 1.0.26 | 実行時 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| unicode-segmentation | 1.13.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-width | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| web-time | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core-deps-windows-linux-android | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-hal | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-naga-bridge | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-collections | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-core | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-future | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-implement | 0.60.2 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-interface | 0.59.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-numerics | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-result | 0.4.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-strings | 0.5.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.52.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.60.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.53.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-threading | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_msvc | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_msvc | 0.53.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| winit | 0.30.13 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| winnow | 1.0.4 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| winresource | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| xml-rs | 0.8.29 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zerocopy-derive | 0.8.59 | ビルド・マクロ用 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zerocopy | 0.8.59 | 実行時 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zeroize | 1.9.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |
| zune-core | 0.5.3 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zune-jpeg | 0.5.15 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |

### yolu-cli の依存一覧

対象: `x86_64-pc-windows-msvc`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 132 件（同名の別版は別件）。実行時 109 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

`yolu-app` と同じ cargo のビルド（`-p yolu-app -p yolu-cli`）で機能が合わさった、`yolu-cli` の部分木。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 AND MIT | 1 |
| MIT | 127 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 1 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-ident | 1.0.26 | ビルド・マクロ用 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |

## Windows GNU の製品別一覧

### yolu-app（yolu-update と yolu-cli の依存を含む） の依存一覧

対象: `x86_64-pc-windows-gnu`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 251 件（同名の別版は別件）。実行時 207 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 | 11 |
| Apache-2.0 AND MIT | 2 |
| BSD-3-Clause | 3 |
| BSL-1.0 | 2 |
| ISC | 1 |
| MIT | 226 |
| MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 1 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 2 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| accesskit | 0.24.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_consumer | 0.35.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_windows | 0.32.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_winit | 0.32.2 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| ahash | 0.8.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| allocator-api2 | 0.2.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arboard | 3.6.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ash | 0.38.0+1.3.281 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bevy_mikktspace | 1.0.0 | 実行時 | Zlib AND (MIT OR Apache-2.0) | MIT AND Zlib | 確認済み |
| bit-set | 0.10.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-vec | 0.9.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| byteorder-lite | 0.1.0 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg_aliases | 0.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| clipboard-win | 5.4.1 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| codespan-reporting | 0.13.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cursor-icon | 1.2.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| curve25519-dalek-derive | 0.1.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| curve25519-dalek | 4.1.3 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| document-features | 0.2.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dpi | 0.1.2 | 実行時 | Apache-2.0 AND MIT | Apache-2.0 AND MIT | 確認済み |
| duplicate | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ecolor | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ed25519-dalek | 2.2.0 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| ed25519 | 2.2.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| eframe | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-wgpu | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-winit | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui_dock | 0.21.1 | 実行時 | MIT | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| emath | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| enumn | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint_default_fonts | 0.36.2 | 実行時 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| error-code | 3.4.0 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| gl_generator | 0.14.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glow | 0.17.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| glutin_wgl_sys | 0.6.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| gpu-allocator | 0.28.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| half | 2.7.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.16.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| heck | 0.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| hex | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| image | 0.25.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itertools | 0.15.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| khronos-egl | 6.0.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| khronos_api | 3.1.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libloading | 0.8.9 | 実行時 | ISC | ISC | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| litrs | 1.0.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| lock_api | 0.4.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| moxcms | 0.8.1 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| naga-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| naga | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| nohash-hasher | 0.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ordered-float | 5.5.0 | 実行時 | MIT | MIT | 確認済み |
| parking_lot | 0.12.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot_core | 0.9.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| paste | 1.0.15 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pollster | 1.0.1 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| presser | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2-diagnostics | 0.10.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| profiling | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pxfm | 0.1.30 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| range-alloc | 0.1.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| raw-window-handle | 0.6.2 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| renderdoc-sys | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rfd | 0.17.2 | 実行時 | MIT | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| rustc-hash | 1.1.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| rustc-hash | 2.1.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| rustc_version | 0.4.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| scopeguard | 1.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| self_cell | 1.3.0 | 実行時 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 | 確認済み |
| semver | 1.0.28 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_spanned | 1.1.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| signature | 2.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smol_str | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| spirv | 0.4.0+sdk-1.4.341.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| static_assertions | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| subtle | 2.6.1 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| toml | 1.1.6+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_datetime | 1.1.1+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_parser | 1.1.3+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| type-map | 0.5.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ufbx | 0.11.5 | 実行時 | MIT OR Unlicense | MIT | 確認済み |
| unicode-general-category | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| unicode-ident | 1.0.26 | 実行時 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| unicode-segmentation | 1.13.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-width | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| web-time | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core-deps-windows-linux-android | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-hal | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-naga-bridge | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-collections | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-core | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-future | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-implement | 0.60.2 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-interface | 0.59.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-numerics | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-result | 0.4.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-strings | 0.5.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.52.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.60.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.53.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-threading | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_gnu | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_gnu | 0.53.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| winit | 0.30.13 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| winnow | 1.0.4 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| winresource | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| xml-rs | 0.8.29 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zerocopy-derive | 0.8.59 | ビルド・マクロ用 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zerocopy | 0.8.59 | 実行時 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zeroize | 1.9.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |
| zune-core | 0.5.3 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zune-jpeg | 0.5.15 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |

### yolu-cli の依存一覧

対象: `x86_64-pc-windows-gnu`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 132 件（同名の別版は別件）。実行時 109 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

`yolu-app` と同じ cargo のビルド（`-p yolu-app -p yolu-cli`）で機能が合わさった、`yolu-cli` の部分木。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 AND MIT | 1 |
| MIT | 127 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 1 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-ident | 1.0.26 | ビルド・マクロ用 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |

## Linux GNU の製品別一覧

### yolu-app（yolu-update と yolu-cli の依存を含む） の依存一覧

対象: `x86_64-unknown-linux-gnu`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 319 件（同名の別版は別件）。実行時 271 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 | 9 |
| Apache-2.0 AND MIT | 2 |
| BSD-3-Clause | 3 |
| ISC | 1 |
| MIT | 297 |
| MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 1 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 2 |
| Zlib | 3 |

状態: 要確認。配布用全文束は生成しない。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| accesskit | 0.24.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_atspi_common | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_consumer | 0.36.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_unix | 0.21.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_winit | 0.32.2 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| ahash | 0.8.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| allocator-api2 | 0.2.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arboard | 3.6.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| as-raw-xcb-connection | 1.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ash | 0.38.0+1.3.281 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-broadcast | 0.7.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-channel | 2.5.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-executor | 1.14.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-io | 2.6.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-lock | 3.4.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-process | 2.5.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-recursion | 1.2.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-signal | 0.2.14 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-task | 4.7.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| atspi-common | 0.13.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| atspi-proxies | 0.13.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| atspi | 0.29.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bevy_mikktspace | 1.0.0 | 実行時 | Zlib AND (MIT OR Apache-2.0) | MIT AND Zlib | 確認済み |
| bit-set | 0.10.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-vec | 0.9.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| blocking | 1.7.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| byteorder-lite | 0.1.0 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| calloop-wayland-source | 0.3.0 | 実行時 | MIT | MIT | 確認済み |
| calloop-wayland-source | 0.4.1 | 実行時 | MIT | MIT | 確認済み |
| calloop | 0.13.0 | 実行時 | MIT | MIT | 確認済み |
| calloop | 0.14.5 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg_aliases | 0.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| codespan-reporting | 0.13.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| concurrent-queue | 2.5.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cursor-icon | 1.2.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| curve25519-dalek-derive | 0.1.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| curve25519-dalek | 4.1.3 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| dlib | 0.5.3 | 実行時 | MIT | MIT | 確認済み |
| document-features | 0.2.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| downcast-rs | 1.2.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| dpi | 0.1.2 | 実行時 | Apache-2.0 AND MIT | Apache-2.0 AND MIT | 確認済み |
| duplicate | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ecolor | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ed25519-dalek | 2.2.0 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| ed25519 | 2.2.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| eframe | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-wgpu | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-winit | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui_dock | 0.21.1 | 実行時 | MIT | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| emath | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| endi | 1.1.1 | 実行時 | MIT | MIT | 確認済み |
| enumflags2 | 0.7.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| enumflags2_derive | 0.7.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| enumn | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint_default_fonts | 0.36.2 | 実行時 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| errno | 0.3.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| event-listener-strategy | 0.5.4 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| event-listener | 5.4.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fastrand | 2.5.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| fixedbitset | 0.5.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.1.5 | 実行時 | Zlib | Zlib | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-lite | 2.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| gethostname | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| getrandom | 0.3.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glow | 0.17.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| gpu-allocator | 0.28.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| half | 2.7.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.15.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.16.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| heck | 0.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| hex | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| image | 0.25.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itertools | 0.15.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| khronos-egl | 6.0.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libloading | 0.8.9 | 実行時 | ISC | ISC | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| linux-raw-sys | 0.12.1 | 実行時 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | MIT | 確認済み |
| linux-raw-sys | 0.4.15 | 実行時 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | MIT | 確認済み |
| litrs | 1.0.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| lock_api | 0.4.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| moxcms | 0.8.1 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| naga-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| naga | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| nohash-hasher | 0.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| nom | 8.0.0 | 実行時 | MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ordered-float | 5.5.0 | 実行時 | MIT | MIT | 確認済み |
| ordered-stream | 0.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| os_pipe | 1.2.3 | 実行時 | MIT | MIT | 確認済み |
| parking | 2.2.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| parking_lot | 0.12.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot_core | 0.9.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| paste | 1.0.15 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| percent-encoding | 2.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| petgraph | 0.8.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| phf | 0.13.1 | 実行時 | MIT | MIT | 確認済み |
| phf_generator | 0.13.1 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| phf_macros | 0.13.1 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| phf_shared | 0.13.1 | 実行時 | MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| piper | 0.2.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| polling | 3.11.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pollster | 0.4.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| pollster | 1.0.1 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| presser | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro-crate | 3.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2-diagnostics | 0.10.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| profiling | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pxfm | 0.1.30 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| quick-xml | 0.41.0 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| quote | 1.0.47 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| raw-window-handle | 0.6.2 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| renderdoc-sys | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rfd | 0.17.2 | 実行時 | MIT | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| rustc-hash | 1.1.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| rustc-hash | 2.1.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| rustc_version | 0.4.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rustix | 0.38.44 | 実行時 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | MIT | 確認済み |
| rustix | 1.1.5 | 実行時 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| scoped-tls | 1.0.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| scopeguard | 1.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| self_cell | 1.3.0 | 実行時 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 | 確認済み |
| semver | 1.0.28 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_repr | 0.1.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_spanned | 1.1.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| signal-hook-registry | 1.4.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| signature | 2.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| siphasher | 1.0.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smithay-client-toolkit | 0.19.2 | 実行時 | MIT | MIT | 確認済み |
| smithay-client-toolkit | 0.20.0 | 実行時 | MIT | MIT | 確認済み |
| smithay-clipboard | 0.7.3 | 実行時 | MIT | MIT | 確認済み |
| smol_str | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| spirv | 0.4.0+sdk-1.4.341.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| static_assertions | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| subtle | 2.6.1 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 1.0.69 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 1.0.69 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| toml | 1.1.6+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_datetime | 1.1.1+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_edit | 0.25.15+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_parser | 1.1.3+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| tree_magic_mini | 3.2.2 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| type-map | 0.5.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ufbx | 0.11.5 | 実行時 | MIT OR Unlicense | MIT | 確認済み |
| unicode-general-category | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| unicode-ident | 1.0.26 | 実行時 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| unicode-segmentation | 1.13.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-width | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| wayland-backend | 0.3.17 | 実行時 | MIT | MIT | 確認済み |
| wayland-client | 0.31.15 | 実行時 | MIT | MIT | 確認済み |
| wayland-csd-frame | 0.3.0 | 実行時 | MIT | MIT | 確認済み |
| wayland-cursor | 0.31.14 | 実行時 | MIT | MIT | 確認済み |
| wayland-protocols-experimental | 20250721.0.1 | 実行時 | MIT | MIT | 確認済み |
| wayland-protocols-misc | 0.3.12 | 実行時 | MIT | MIT | 同梱する protocol XML（server-decoration.xml）に LGPL-2.1-or-later の表記あり。生成バインディングの配布条件を確認するまで Linux 配布を停止する。 |
| wayland-protocols-plasma | 0.3.12 | 実行時 | MIT | MIT | 同梱する protocol XML に LGPL-2.1-or-later の表記あり。生成バインディングの配布条件を確認するまで Linux 配布を停止する。 |
| wayland-protocols-wlr | 0.3.12 | 実行時 | MIT | MIT | 確認済み |
| wayland-protocols | 0.32.13 | 実行時 | MIT | MIT | 確認済み |
| wayland-scanner | 0.31.11 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| wayland-sys | 0.31.11 | 実行時 | MIT | MIT | 確認済み |
| web-time | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core-deps-windows-linux-android | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-hal | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-naga-bridge | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| winit | 0.30.13 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| winnow | 1.0.4 | 実行時 | MIT | MIT | 確認済み |
| winresource | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| wl-clipboard-rs | 0.9.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| x11-dl | 2.21.0 | 実行時 | MIT | MIT | 確認済み |
| x11rb-protocol | 0.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| x11rb | 0.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| xcursor | 0.3.11 | 実行時 | MIT | MIT | 確認済み |
| xkbcommon-dl | 0.4.2 | 実行時 | MIT | MIT | 確認済み |
| xkeysym | 0.2.1 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zbus-lockstep-macros | 0.5.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zbus-lockstep | 0.5.2 | 実行時 | MIT | MIT | 確認済み |
| zbus | 5.19.0 | 実行時 | MIT | MIT | 確認済み |
| zbus_macros | 5.19.0 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zbus_names | 4.3.4 | 実行時 | MIT | MIT | 確認済み |
| zbus_xml | 5.2.1 | 実行時 | MIT | MIT | 確認済み |
| zcheapstr | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| zerocopy-derive | 0.8.59 | ビルド・マクロ用 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zerocopy | 0.8.59 | 実行時 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zeroize | 1.9.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |
| zune-core | 0.5.3 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zune-jpeg | 0.5.15 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zvariant | 5.15.0 | 実行時 | MIT | MIT | 確認済み |
| zvariant_derive | 5.15.0 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zvariant_utils | 4.2.0 | 実行時 | MIT | MIT | 確認済み |

### yolu-cli の依存一覧

対象: `x86_64-unknown-linux-gnu`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 131 件（同名の別版は別件）。実行時 108 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

`yolu-app` と同じ cargo のビルド（`-p yolu-app -p yolu-cli`）で機能が合わさった、`yolu-cli` の部分木。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 AND MIT | 1 |
| MIT | 126 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 1 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-ident | 1.0.26 | ビルド・マクロ用 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |

## macOS（universal）の製品別一覧

### yolu-app（yolu-update と yolu-cli の依存を含む） の依存一覧

対象: `universal-apple-darwin`、通常の機能。Cargo.lock SHA-256: `626bc8ffd69f876e625a4c8a6feec75468d17e2ecf01d4b19a9bc3c757d69588`。

外部クレート 246 件（同名の別版は別件）。実行時 206 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| Apache-2.0 | 7 |
| Apache-2.0 AND MIT | 2 |
| BSD-3-Clause | 3 |
| ISC | 1 |
| MIT | 227 |
| MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 1 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 2 |
| Zlib | 2 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| accesskit | 0.24.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_consumer | 0.38.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_macos | 0.26.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_winit | 0.32.2 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| ahash | 0.8.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arboard | 3.6.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| async-trait | 0.1.92 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| atomic-waker | 1.1.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| base64 | 0.23.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bevy_mikktspace | 1.0.0 | 実行時 | Zlib AND (MIT OR Apache-2.0) | MIT AND Zlib | 確認済み |
| bit-set | 0.10.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-vec | 0.9.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bitflags | 1.3.2 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block2 | 0.5.1 | 実行時 | MIT | MIT | 確認済み |
| block2 | 0.6.2 | 実行時 | MIT | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| byteorder-lite | 0.1.0 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| bytes | 1.12.1 | 実行時 | MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg_aliases | 0.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| chacha20 | 0.10.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| chrono | 0.4.45 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| codespan-reporting | 0.13.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| core-foundation-sys | 0.8.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| core-foundation | 0.9.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| core-graphics-types | 0.1.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| core-graphics | 0.23.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cpufeatures | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cursor-icon | 1.2.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| curve25519-dalek-derive | 0.1.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| curve25519-dalek | 4.1.3 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| dispatch2 | 0.3.1 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| dispatch | 0.2.0 | 実行時 | MIT | MIT | 確認済み |
| document-features | 0.2.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dpi | 0.1.2 | 実行時 | Apache-2.0 AND MIT | Apache-2.0 AND MIT | 確認済み |
| duplicate | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dyn-clone | 1.0.20 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ecolor | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ed25519-dalek | 2.2.0 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| ed25519 | 2.2.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| eframe | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-wgpu | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-winit | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui_dock | 0.21.1 | 実行時 | MIT | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| emath | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| enumn | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint_default_fonts | 0.36.2 | 実行時 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fallible-iterator | 0.3.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fallible-streaming-iterator | 0.1.9 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| fax | 0.2.7 | 実行時 | MIT | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fontdb | 0.24.0 | 実行時 | MIT | MIT | 確認済み |
| foreign-types-macros | 0.2.4 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| foreign-types-shared | 0.3.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| foreign-types | 0.5.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| futures-channel | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-core | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-executor | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-io | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-macro | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-sink | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-task | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures-util | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| futures | 0.3.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| getrandom | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| half | 2.7.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.16.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| heck | 0.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| hex | 0.4.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| http-body-util | 0.1.5 | 実行時 | MIT | MIT | 確認済み |
| http-body | 1.1.0 | 実行時 | MIT | MIT | 確認済み |
| http | 1.5.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httparse | 1.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| httpdate | 1.0.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hyper-util | 0.1.21 | 実行時 | MIT | MIT | 確認済み |
| hyper | 1.12.0 | 実行時 | MIT | MIT | 確認済み |
| image | 0.25.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| itertools | 0.15.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libloading | 0.8.9 | 実行時 | ISC | ISC | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libsqlite3-sys | 0.38.2 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| litrs | 1.0.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| lock_api | 0.4.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| mio | 1.2.4 | 実行時 | MIT | MIT | 確認済み |
| moxcms | 0.8.1 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| naga-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| naga | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| nohash-hasher | 0.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| objc-sys | 0.3.5 | 実行時 | MIT | MIT | 確認済み |
| objc2-app-kit | 0.2.2 | 実行時 | MIT | MIT | 確認済み |
| objc2-app-kit | 0.3.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| objc2-core-foundation | 0.3.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| objc2-core-graphics | 0.3.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| objc2-encode | 4.1.0 | 実行時 | MIT | MIT | 確認済み |
| objc2-foundation | 0.2.2 | 実行時 | MIT | MIT | 確認済み |
| objc2-foundation | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| objc2-metal | 0.3.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| objc2-quartz-core | 0.3.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| objc2 | 0.5.2 | 実行時 | MIT | MIT | 確認済み |
| objc2 | 0.6.4 | 実行時 | MIT | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot | 0.12.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot_core | 0.9.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| paste | 1.0.15 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| pastey | 0.2.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pollster | 1.0.1 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2-diagnostics | 0.10.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| profiling | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pxfm | 0.1.30 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| quick-error | 2.0.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand | 0.10.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rand_core | 0.10.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| raw-window-handle | 0.6.2 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| raw-window-metal | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast-impl | 1.0.27 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ref-cast | 1.0.27 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rfd | 0.17.2 | 実行時 | MIT | MIT | 確認済み |
| rmcp | 3.5.1 | 実行時 | Apache-2.0 | Apache-2.0 AND MIT | 確認済み |
| rusqlite | 0.40.2 | 実行時 | MIT | MIT | 確認済み |
| rustc-hash | 1.1.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| rustc-hash | 2.1.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| rustc_version | 0.4.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| schemars | 1.2.2 | 実行時 | MIT | MIT | 確認済み |
| schemars_derive | 1.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| scopeguard | 1.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| self_cell | 1.3.0 | 実行時 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 | 確認済み |
| semver | 1.0.28 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive | 1.0.229 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_derive_internals | 0.30.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_spanned | 1.1.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| signature | 2.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| slab | 0.4.12 | 実行時 | MIT | MIT | 確認済み |
| slotmap | 1.1.1 | 実行時 | Zlib | Zlib | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smol_str | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| socket2 | 0.6.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sse-stream | 0.2.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| static_assertions | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| subtle | 2.6.1 | 実行時 | BSD-3-Clause | BSD-3-Clause | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tiff | 0.11.3 | 実行時 | MIT | MIT | 確認済み |
| tinyvec | 1.13.3 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| tokio-macros | 2.7.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tokio-stream | 0.1.19 | 実行時 | MIT | MIT | 確認済み |
| tokio-util | 0.7.19 | 実行時 | MIT | MIT | 確認済み |
| tokio | 1.53.2 | 実行時 | MIT | MIT | 確認済み |
| toml | 1.1.6+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_datetime | 1.1.1+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| toml_parser | 1.1.3+spec-1.1.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| tower-service | 0.3.3 | 実行時 | MIT | MIT | 確認済み |
| tracing-attributes | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| try-lock | 0.2.5 | 実行時 | MIT | MIT | 確認済み |
| type-map | 0.5.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ufbx | 0.11.5 | 実行時 | MIT OR Unlicense | MIT | 確認済み |
| unicode-general-category | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| unicode-ident | 1.0.26 | 実行時 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| unicode-segmentation | 1.13.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-width | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| want | 0.3.2 | 実行時 | MIT | MIT | 確認済み |
| web-time | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| weezl | 0.1.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core-deps-apple | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-hal | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-naga-bridge | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| winit | 0.30.13 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| winnow | 1.0.4 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| winresource | 0.1.31 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zerocopy-derive | 0.8.59 | ビルド・マクロ用 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zerocopy | 0.8.59 | 実行時 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zeroize | 1.9.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |
| zune-core | 0.5.3 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| zune-jpeg | 0.5.15 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |

## 開発用 xtask の依存一覧

`cargo xtask` 自体は製品に同梱しない。通常依存とビルド依存を含み、試験用依存を除いた集合。
各対象で `python3 tools/third-party.py --target <target> --package xtask` を実行して照合した。
`yolu-update` 単独も同じ指定の `--package yolu-update` で各 31 件を照合済み（その全件は上の app の集合に含まれる）。
「○」は対象に含まれ原文の照合が成功したもの、「—」は対象外。

| クレート | 版 | 選択する許諾 | Windows MSVC | Windows GNU | Linux GNU |
|---|---|---|---|---|---|
| adler2 | 2.0.1 | MIT | ○ | ○ | ○ |
| bitflags | 2.13.2 | MIT | — | — | ○ |
| block-buffer | 0.10.4 | MIT | ○ | ○ | ○ |
| bumpalo | 3.20.3 | MIT | ○ | ○ | ○ |
| cfg-if | 1.0.5 | MIT | ○ | ○ | ○ |
| cpufeatures | 0.2.17 | MIT | ○ | ○ | ○ |
| crc32fast | 1.5.2 | MIT | ○ | ○ | ○ |
| crypto-common | 0.1.7 | MIT | ○ | ○ | ○ |
| curve25519-dalek-derive | 0.1.1 | MIT | ○ | ○ | ○ |
| curve25519-dalek | 4.1.3 | BSD-3-Clause | ○ | ○ | ○ |
| digest | 0.10.7 | MIT | ○ | ○ | ○ |
| displaydoc | 0.2.7 | MIT | ○ | ○ | ○ |
| ed25519-dalek | 2.2.0 | BSD-3-Clause | ○ | ○ | ○ |
| ed25519 | 2.2.3 | MIT | ○ | ○ | ○ |
| equivalent | 1.0.2 | MIT | ○ | ○ | ○ |
| filetime | 0.2.29 | MIT | ○ | ○ | ○ |
| flate2 | 1.1.10 | MIT | ○ | ○ | ○ |
| generic-array | 0.14.7 | MIT | ○ | ○ | ○ |
| getrandom | 0.3.4 | MIT | ○ | ○ | ○ |
| hashbrown | 0.17.1 | MIT | ○ | ○ | ○ |
| hex | 0.4.3 | MIT | ○ | ○ | ○ |
| indexmap | 2.14.2 | MIT | ○ | ○ | ○ |
| itoa | 1.0.18 | MIT | ○ | ○ | ○ |
| libc | 0.2.190 | MIT | — | — | ○ |
| linux-raw-sys | 0.12.1 | MIT | — | — | ○ |
| log | 0.4.34 | MIT | ○ | ○ | ○ |
| memchr | 2.8.3 | MIT | ○ | ○ | ○ |
| miniz_oxide | 0.9.1 | MIT | ○ | ○ | ○ |
| proc-macro2 | 1.0.107 | MIT | ○ | ○ | ○ |
| quote | 1.0.47 | MIT | ○ | ○ | ○ |
| rustc_version | 0.4.1 | MIT | ○ | ○ | ○ |
| rustix | 1.1.5 | MIT | — | — | ○ |
| semver | 1.0.28 | MIT | ○ | ○ | ○ |
| serde | 1.0.229 | MIT | ○ | ○ | ○ |
| serde_core | 1.0.229 | MIT | ○ | ○ | ○ |
| serde_derive | 1.0.229 | MIT | ○ | ○ | ○ |
| serde_json | 1.0.151 | MIT | ○ | ○ | ○ |
| sha2 | 0.10.9 | MIT | ○ | ○ | ○ |
| signature | 2.2.0 | MIT | ○ | ○ | ○ |
| simd-adler32 | 0.3.10 | MIT | ○ | ○ | ○ |
| subtle | 2.6.1 | BSD-3-Clause | ○ | ○ | ○ |
| syn | 2.0.119 | MIT | ○ | ○ | ○ |
| syn | 3.0.6 | MIT | ○ | ○ | ○ |
| tar | 0.4.46 | MIT | ○ | ○ | ○ |
| thiserror-impl | 2.0.21 | MIT | ○ | ○ | ○ |
| thiserror | 2.0.21 | MIT | ○ | ○ | ○ |
| typenum | 1.20.1 | MIT | ○ | ○ | ○ |
| unicode-ident | 1.0.26 | MIT AND Unicode-3.0 | ○ | ○ | ○ |
| version_check | 0.9.5 | MIT | ○ | ○ | ○ |
| xattr | 1.6.1 | MIT | — | — | ○ |
| zeroize | 1.9.0 | MIT | ○ | ○ | ○ |
| zip | 2.4.2 | MIT | ○ | ○ | ○ |
| zmij | 1.0.23 | MIT | ○ | ○ | ○ |
| zopfli | 0.8.3 | Apache-2.0 | ○ | ○ | ○ |
