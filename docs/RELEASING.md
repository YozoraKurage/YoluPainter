# 配布の手順

Windows x86_64 MSVC の zip とインストーラー（NSIS の setup.exe）、Linux x86_64 の tar.gz を作ります。macOS、AppImage は対象外です。
実行ファイル・インストーラーのコード署名は、[SignPath Foundation への申し込み](#コード署名signpath-foundation)が通るまで付けません（署名なしで出します）。
ビルドには Rust stable、Python 3.10 以降、各 OS の C/C++ ビルド環境が必要です。Windows のインストーラーには NSIS 3 も要ります。Linux の実行環境は README を参照してください。
Ubuntu 22.04 で作るため、これより古い glibc 環境での動作は保証しません。

Linux の既存依存 `wayland-protocols-plasma` と `wayland-protocols-misc` の protocol XML には LGPL-2.1-or-later の表記があり、
`tools/licenses-reviewed.json` の `blocked` に記録しています。条件を確認するか依存構成を変更するまで、Linux の `bundle` は停止し、
両 OS の成功が必要な Release 作成も進みません。`blocked` を外すのは、生成バインディングへの条件の適用を確認した後だけです。

依存を追加・更新して `licenses-reviewed.json` へ承認を足すときは、クレートの宣言だけでなく、同梱する XML・ソース全体に
GPL/LGPL の表記が無いかを調べます。ヒットしたら、選ばないデュアルライセンスの側（例: `self_cell` の GPL）かを確かめ、そうでなければ `blocked` に理由を書きます。

```sh
grep -rIlE 'SPDX-License-Identifier:.*GPL|GNU (Lesser|Library) General Public' ~/.cargo/registry/src/*/<クレート>-<版>
```

## 版と配布物

`Cargo.toml` の workspace.package.version を更新し、`Cargo.lock` を更新・コミットしてから、そのコミットを配布対象にします。
版は SemVer です。試験版は `0.1.0-rc.1` のようにプレリリース識別子を付けてください。

```sh
cargo xtask build --target x86_64-pc-windows-msvc --release
cargo xtask bundle --target x86_64-pc-windows-msvc
cargo xtask installer --target x86_64-pc-windows-msvc
cargo xtask symbols --target x86_64-pc-windows-msvc   # Windows だけ。PDB の付属物（下の「PDB の付属物」）
# Linux では target を x86_64-unknown-linux-gnu に替える（installer は Windows だけ）
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist
```

`bundle` と `installer` は直前に同じコミットからビルドした release 実行ファイルを使います。古いビルドを使わないでください。
`bundle` と `installer` は `target/dist` に過去の版を残すので、手元で出すときは `target/dist` を空にしてから作ります。
違う版や余分なファイルが残っていると `updater-json` が拒否します（PDB の付属物 `yolupainter-<版>-x86_64-pc-windows-msvc-pdb.zip` の名前だけは例外。CI は毎回まっさらです）。
出力は `target/dist/yolupainter-<版>-<target>.zip`（または `.tar.gz`）と、Windows の `yolupainter-<版>-x86_64-pc-windows-msvc-setup.exe`、`symbols` を走らせたときの PDB の付属物。
実行ファイル、LICENSE、操作・動作環境を含む README（日英）、THIRD_PARTY.md、対象別の DEPENDENCIES.md と許諾全文、使う人向けの `docs/`（`docs/en/` を含む）を
同梱します（インストーラーも同じ物を入れます）。文書は配布物の中でもフォルダつきの `docs/GUIDE.md`・`docs/en/GUIDE.md` の名前で入るので、README からの相対のリンクがそのまま効きます。
開発の手順（`docs/DEVELOPMENT.md`・`docs/RELEASING.md`）は入れません。入れる物の一覧は `crates/xtask/src/main.rs` の `BUNDLED_DOCS` と `LEFT_OUT_DOCS` の 1 か所で、
`docs/` に足したファイルは、そのどちらかへ必ず載せます（載せ忘れると `bundle`・`installer` が止まり、`cargo test -p xtask` も落ちます）。
入れる物は `.md` の文書だけで、画像など `.md` でない物は `LEFT_OUT_DOCS` へ載せて入れません（更新で前の版にだけあった文書を消すインストーラーの掃除が `.md` だけを対象にするため。入れる必要が出たら、`installer/yolupainter.nsi` の `RemoveOldDocs` も直します。試験が断ります）。
入れる文書の相対のリンクが配布物の中で切れないことも試験が確かめるので、入れない物（`docs/DEVELOPMENT.md`・`CHANGELOG.md`・`crates/` の README など）へは GitHub の URL で張ります。
インストーラーのスクリプト `installer/yolupainter.nsi` の `DocFiles` にも同じ一覧があり、試験が突き合わせます（文書を足したら両方に足します）。
`tools/third-party.py` が対象ごとに許諾を照合し、未確認の依存や原文の不一致では束ねません。
更新クレート（`yolu-update`）はアプリに組み込まれているので、その依存の許諾全文も含めます。
`xtask` 自体は配りません。独自の `CARGO_TARGET_DIR` は使わず、出力を `target/` に揃えてください。

`updater-json` の入力は、その版の配布物だけを置いた専用フォルダです。違う版や余分なファイルは拒否します
（PDB の付属物は、名前が完全一致の 1 つだけ許し、更新情報には載せません）。
Windows の zip があるのにインストーラーが無い版も拒否します（インストーラーで入れたアプリは、更新にインストーラーを使うので、並べて出します）。
更新情報（schema 1）には、zip・tar.gz を対象の三つ組み（`x86_64-pc-windows-msvc` など）で、インストーラーを別の鍵 `x86_64-pc-windows-msvc-setup` で載せます。
ファイル名は `updater-v1.json`（schema の番号入り。[更新情報の互換](#更新情報の互換)を参照）。
URL は `https://github.com/YozoraKurage/YoluPainter/releases/download/v<版>/<配布物名>` に固定です。アプリが更新情報を取る場所は
`https://github.com/YozoraKurage/YoluPainter/releases/latest/download/updater-v1.json`（GitHub の「最新の Release」。下書き・プレリリースは含みません）です。
フォークから配る場合は、更新クレートの `RELEASE_BASE`・`UPDATER_URL` も変更してビルドします。
署名なしの JSON は検査専用で、更新クレートは受理しません。

### PDB の付属物

クラッシュの記録は各フレームの番地と実行ファイルの基底の番地（`Image base`）を書くので、配布物の PDB があれば、同じ版の関数名・行へ引けます。
`.github/workflows/release.yml` の Windows の組みは、`cargo xtask build` の前に `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`・`CARGO_PROFILE_RELEASE_STRIP=none` を環境に置き
（命令は変えず、行番号つきの PDB を実行ファイルとは別に作る）、`cargo xtask installer` のあとに `cargo xtask symbols` で `yolupainter.pdb` だけを入れた
`yolupainter-<版>-x86_64-pc-windows-msvc-pdb.zip` を `target/dist` に作ります。Release の付属物としては載りますが、zip・インストーラーには入らず、
更新の対象ではありません（署名つきの更新情報に載せず、アプリは取りに行きません）。`verify` は、通常のファイルで、空でなく、大きさの上限内で、中身が `yolupainter.pdb` だけであることを見ます。
`symbols` は PDB を調べる前に前回の付属物を消すので、組み直しに失敗したあとで古い PDB が残りません。

### Windows のインストーラー

NSIS のスクリプトは `installer/yolupainter.nsi` です。利用者ごとのインストール（`%LOCALAPPDATA%\Programs\YoluPainter`、管理者権限なし、64 ビット）で、
スタートメニューのショートカットとアンインストーラー（「設定 → アプリ」に登録）を置きます。`.ylp` の関連付けは、画面では選べます。

| 引数 | 動き |
|---|---|
| `/S` | 画面を出さない |
| `/ASSOC=1` / `/ASSOC=0` | `.ylp` の関連付けを付ける・付けない（無音のとき。省略は今の状態のまま、初めてなら付けない） |
| `/RUN` | 入れ終わったらアプリを起こす（アプリの更新が使います）。待ちの上限で何も変えずに終わるときも、今入っているアプリを起こし直します |
| `/D=<パス>` | 入れ先（最後に置く。省略は前の入れ先） |
| アンインストーラーの `/S` `/DELETEDATA` | 無音。アプリが作り直せるデータ（設定・窓の配置・復旧・クラッシュの記録・サムネイルのキャッシュ。`%APPDATA%\YoluPainter` と `%LOCALAPPDATA%\YoluPainter` の名指しした物）は、画面では消すかを聞き、無音では `/DELETEDATA` のときだけ消す。個人のライブラリ・ブラシ・サブツール・グラデーション・カラーセット・表示のプリセットなど利用者が作った物と、知らないファイルは、どちらでも残す（表は `docs/INSTALL.md`） |

実行中のアプリは終了させません。実行ファイルが使われている間は待ち（無音は 60 秒まで。超えたら何も変えずに終了コード 5。`/RUN` が付いていれば今入っている実行ファイルを起こし直します）、
アンインストーラーは自分が入れたファイルだけを消します（入れ先に利用者のファイルがあれば、入れ先のフォルダは残ります）。

文書は入れ先の `docs\`・`docs\en\` に入ります。入れた文書の名前は `docs\.installed` に記録し、更新（上書き）のとき、前の版の記録にある文書を先に消してから今の版の文書を入れるので、
前の版にだけあった文書（名前を変えた・外した文書）が残りません。消すのは記録にある `docs\` の下の `.md` だけで、`..` を含む名前は読み飛ばします
（利用者が `docs\` に置いたファイルと、入れ先の外は消えません）。アンインストールは一覧の文書と記録を消し、`docs\en`・`docs\` は空のときだけ消します（`RMDir /r` は使いません）。

NSIS は 3.x が要ります。ワークフローは、windows-latest のイメージに `makensis` が無ければ Chocolatey（`choco install nsis --version=3.11.0`）で入れ、
入っていた物でも入れた物でも `makensis /VERSION` が 3.11 でなければ止めます（配布物の作り方を変えないため）。確かめた `makensis` は
`MAKENSIS` で `cargo xtask installer` へ渡します。手元では Windows は `winget install NSIS.NSIS`、Debian・Ubuntu は
`sudo apt install nsis` で入れます（`MAKENSIS` に `makensis` の場所を指定することもできます）。
`cargo test -p xtask` は、`makensis` があればスクリプトを実際にコンパイルします（CI の Linux は `nsis` を入れて走らせます）。

画面を出さない流れ（新規・更新・`/RUN`・関連付けの保持・アンインストール。文書の入れ方と、更新で前の版にだけあった文書が消えること・利用者のファイルが消えないことを含む）と、待ちの上限（書き込みで開けない実行ファイルが残っている間は待ち、
上限を超えたら何も変えずに終了コード 5。`/RUN` の有無で今の実行ファイルを起こし直すかも確かめます。試験用に上限を 3 秒へ縮めたインストーラー `-DWAIT_STEPS=6` を使います）は、Wine で通せます。

```sh
python3 tools/test-installer.py
```

実際に動いているアプリを待つ動きは、Wine が動いている実行ファイルの上書きを断る版でだけ確かめられます（上書きできる版では、注意を出して通ります）。
ページの並び・チェック・確認の窓などの画面は Wine では確かめません。Windows の実機で、インストール・更新・動いているアプリがある間の更新・アンインストールを 1 回ずつ通してください。

### exe のバージョン情報

`crates/yolu-app/build.rs` が、製品名 `YoluPainter`・版（workspace の版。プレリリース識別子つき）・作者名・アイコン
（`crates/yolu-app/assets/logo/yolupainter.ico`）を実行ファイルへ埋めます。インストーラーも同じ製品名・版を持ちます
（コード署名の条件の「製品名と版のメタデータ」）。

## 更新情報の互換

更新クレートは、署名が通った更新情報のうち知らない target の配布物を読み飛ばし、自分の target の配布物だけを厳密に確かめます。
macOS など対象を足した版の更新情報を、すでに配った版が拒否することはありません。インストーラーも、知らない鍵の配布物として旧いクライアントが読み飛ばします。配布物の数の上限は固定値（`MAX_ASSETS`）です。

次の変更は schema を上げる必要があります。既存のクライアントは schema が違う更新情報を拒否し、そのまま古い版に残ります。

- 本文の項目の追加・削除・意味の変更
- 既知の target の配布物の項目の追加・削除・意味の変更

schema を上げるときは、旧 schema を読むクライアントが取得する更新情報を旧形式のまま残します。
アプリが取る URL（`UPDATER_URL`）はファイル名に schema の番号を含みます。新しい schema は新しい名前（`updater-v2.json`）に置き、
旧い `updater-v1.json` も同じ Release に置き続けてください（`UPDATER_FILE`・`UPDATER_URL`・`UPDATER_SCHEMA` を一緒に変えます）。

## 更新署名の鍵（初回だけ）

配布の管理者が安全な手元の環境で実行してください。リポジトリの外に、アクセスを制限した保管先を用意します。

```sh
cargo xtask keygen --output /secure/location/update-private-key.hex
```

秘密鍵は 32 バイトの seed を hex にしたファイルです。既存ファイルは上書きしません。
Unix では作成権限を 0600 にします。Windows では保管フォルダの ACL を管理者本人に限定してください。
標準出力には公開鍵だけが表示されます。秘密鍵は Git、ログ、配布物に入れず、紛失に備えて安全にバックアップします。
公開鍵は 16 進の 64 文字で、配布のビルドがアプリへ組み込みます。控えを失ったときは秘密鍵ファイルから取り出せます。

```sh
cargo xtask pubkey --key-file /secure/location/update-private-key.hex
```

アプリは、ビルド時の環境変数 `YOLUPAINTER_UPDATE_PUBLIC_KEY` に入れた公開鍵だけを信じ（`yolu_update::embedded_public_key`）、
公開鍵を組み込んでいないビルド（ソースからの手元のビルドなど）は、ヘルプのメニューの更新の項目も初回の問いも出さず、通信もしません。
`cargo xtask build` はこの環境変数を検査し、空の値（GitHub の未設定の変数）は未設定として扱い、入っているのに鍵として使えない値
（長さ・形式が違う、弱い鍵）はビルドを止めます。Draft を作る配布では `--require-update-key` を付けて、未設定も止めます
（更新できないアプリを配らないため）。更新サーバーから公開鍵を受け取る設計にはしないでください。
鍵の変更は既存アプリの信頼する公開鍵も変える必要があり、自動の鍵更新は未対応です。

手元で署名する場合は次のように実行します。秘密鍵を引数そのものに書かないでください。

```sh
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist --sign --key-file /secure/location/update-private-key.hex
```

`--key-file` を省くと環境変数 `YOLUPAINTER_UPDATE_PRIVATE_KEY` を読みます。
署名したら、公開鍵だけで、アプリと同じ検証（署名・版・大きさ・SHA-256）を通るかを確かめます。
別の鍵で署名した更新情報、配布物との食い違い、載っているのに無い配布物はここで失敗します。
zip・tar.gz は中身も開いて、梱包の一覧（上の文書を含む）と照らします。足りないファイルも、一覧に無いファイルも、同じ名前の重複も、名前を並べて断ります
（インストーラーの中は開けないので、一覧どおりの段から作ることと、スクリプトの試験で確かめます）。

```sh
cargo xtask verify --version 0.1.0-rc.1 --assets target/dist --public-key <公開鍵の hex>
```

署名は Ed25519、対象は JSON envelope の `payload` 文字列の UTF-8 バイトです。
`signature` は 64 バイトの署名の hex。payload を解析・再整形してから検証しないでください。
本文には schema=1、version、target/name/url/sha256/size を持つ assets が入ります。

## GitHub の設定と起動

1. リポジトリに environment `release` を作り、承認者と利用可能なブランチ／タグを制限します。
2. **environment の secret** `YOLUPAINTER_UPDATE_PRIVATE_KEY` に秘密鍵ファイルの内容を保存します。リポジトリ全体の secret には置きません。
3. リポジトリの変数（Variables）`YOLUPAINTER_UPDATE_PUBLIC_KEY` に公開鍵の hex を保存します。公開鍵は秘密ではありません。
   ビルドのジョブが、この値をアプリへ組み込みます（dry-run=false では、空だとビルドが止まります）。
   draft ジョブは署名の直後にこの公開鍵で `verify` を実行し、通らなければ Draft を作りません。
4. Actions の「配布物の作成」で配布対象のコミットを含むブランチ／タグを選び、kind を prerelease または stable にします。
5. 最初は **dry-run=true（既定）** で実行します。Windows・Linux をビルドし、未署名 JSON を含む `release-preview` artifact を作ります。secret に触れず、Release は作りません。
6. artifact を取得し、両 OS で展開・起動・同梱文書・許諾全文を確認します。Linux 実行ファイルには実行権限があります。
   Windows はインストーラーで、インストール・起動・更新（前の版のインストーラーで入れた上に入れる）・アンインストールを通します。
7. 同じコミットに対して dry-run=false で実行し、environment の承認を行います。署名付き JSON とアーカイブを **Draft Release** にアップロードします。
8. 署名と各配布物の SHA-256・サイズはワークフローが公開鍵で確認済みです。Draft のタグと対象コミット、版、prerelease の状態を確認します。
   本文は空で作られるので、両 OS の実機確認後、管理者が本文を書き、README の「Code signing policy」の節へのリンクを入れて手で公開します。
   **公開した時点で `releases/latest` が切り替わり、アプリの更新の確認がその版を見つけ始めます**（stable のみ。prerelease は `latest` に出ません）。

同時実行は 1 本です。実行中の処理は自動取消しません。既存の同じタグの Release は上書きしません。
失敗後は Draft とタグの状態を確認してから再実行してください。非公開リポジトリでは認証と Actions の利用枠にも注意してください。
GitHub の prerelease 設定と SemVer のプレリリース識別子は別です。stable はプレリリース識別子を含む版を拒否します。

## コード署名（SignPath Foundation）

署名なしで 0.x を出し、使われ始めたら [SignPath Foundation の無償のコード署名](https://signpath.org/terms.html)に申し込みます。
リポジトリの側でそろえてある条件と、申し込みのときに管理者がすることです。

| 条件 | 状態 |
|---|---|
| OSI の許諾・自作のコードだけ | [MIT](../LICENSE)。依存の許諾は `THIRD_PARTY.md` と許諾の全文で追う |
| 製品名と版のメタデータ | 実行ファイル（`build.rs`）とインストーラー（NSIS）が、製品名 `YoluPainter` と同じ版を持つ |
| 聞かずに通信しない | 更新の確認は初回の問いで「はい」を選んだときと、手で押したときだけ。README の「Code signing policy」にプライバシーの一文 |
| システムの変更の告知・アンインストール | `.ylp` の関連付けは選択肢で、無音では付けない（今の状態のまま）。アンインストーラーは入れたファイルだけを消し、利用者のデータは聞く |
| Code signing policy の表記 | README の節（役割とプライバシー）。定型文は、申し込みが通ってから足す |
| 検証できるビルド・リリースごとの手動の承認 | `配布物の作成` ワークフロー（手で起動・dry-run が既定・Draft を作るには environment の承認） |

管理者がすること:

1. 最初のリリース（署名なし）を出し、使われ始めるのを待ちます（申し込みは「署名する形で、もうリリースされている」ことが条件です）。
2. GitHub と SignPath の全員で多要素認証を有効にし、SignPath Foundation へ申し込みます。
3. 通ったら、README の「Code signing policy」の節の先頭へ次の定型文を足し、Release の本文からもこの節へリンクします。

   ```
   Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/)
   ```

4. SignPath 側で、署名する実行ファイルの製品名を `YoluPainter` に、製品の版をビルド内で同じ値に強制する設定（artifact configuration）を作り、
   ワークフローに署名の段階を足します。署名する物は `yolupainter.exe`（zip とインストーラーに入れる前）とインストーラー本体です。
   署名した実行ファイルを入れた zip・インストーラーを作り直し、更新情報（`updater-json`）の SHA-256 は署名後の配布物から作ります。
   インストーラーが書き出すアンインストーラーの署名は NSIS の `!uninstfinalize` を使う手順になるので、この段階で設計します。
   この署名の段階は、まだワークフローに入っていません。

## 検証と現在の範囲

```sh
cargo test -p yolu-update -p xtask --locked
python3 tools/test-release-tools.py
actionlint .github/workflows/release.yml
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

更新クレートは HTTP の口を差し替える形で、署名確認・版比較・明示承認後のダウンロード・サイズと SHA-256 の確認までを提供します。
アプリ側の HTTP 実装（`crates/yolu-app/src/update/http.rs`）は、Windows は WinHTTP（OS の証明書・プロキシ・TLS）、Linux は `curl` で、
https だけ・時間切れ・読み込み中のサイズ上限・取消を守り、止まらない転送の連なりは失敗にします。転送の回数は、curl は 5 回まで（転送先も https だけ）、
WinHTTP は 5 回に絞る設定を試み、設定できない環境では OS の既定（10 回）まで、で、https から http へは WinHTTP の既定が断ります。
現在の API は同期で、取得データをメモリに保持します。更新情報は 1 MiB、配布物は 2 GiB が上限です。
プレリリースの版を提案するのは、実行中のアプリ自身がプレリリースのときだけです。同じ版・古い版への更新は提案しません。
署名済みの過去の情報の再送による「新しい版を見せない」攻撃への鮮度保証はありません。
ダウンロードは、利用者が「更新」を押したあとだけ始めます。ダウンロードしたインストーラーは、署名つきの更新情報の SHA-256・大きさで確かめたものを
利用者ごとの置き場（Windows は `%LOCALAPPDATA%\YoluPainter\updates`）へ置き、走らせる直前にもう一度確かめます。
置き場のインストーラーと書きかけは、次のダウンロードと、アプリの起動時（今の版以下のものだけ。走っている最中のものは次の起動で）に片付けます。
アプリ自身は実行ファイルを置き換えません。インストールした Windows はインストーラーを無音で走らせて終了し（インストーラーが終了を待って入れ、`/RUN` で起こし直す）、
zip・Linux はその版のリリースのページを開きます。実行ファイルの隣に `uninstall.exe` があるかで、インストーラーで入れた物かを見分けます。

ワークフローの構成は [ALCOM の配布ワークフロー](https://github.com/vrc-get/vrc-get/blob/master/.github/workflows/publish-gui.yml)を参考にしています。
