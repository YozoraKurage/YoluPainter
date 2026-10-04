# 配布の手順

Windows x86_64 MSVC の zip と Linux x86_64 の tar.gz を作ります。macOS、インストーラー、AppImage、実行ファイルのコード署名は対象外です。
ビルドには Rust stable、Python 3.10 以降、各 OS の C/C++ ビルド環境が必要です。Linux の実行環境は README を参照してください。
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
# Linux では target を x86_64-unknown-linux-gnu に替える
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist
```

`bundle` は直前に同じコミットからビルドした release 実行ファイルを束ねます。古いビルドを使わないでください。
`bundle` は `target/dist` に過去の版を残すので、手元で出すときは `target/dist` を空にしてから `bundle` します。
違う版や余分なファイルが残っていると `updater-json` が拒否します（CI は毎回まっさらです）。
出力は `target/dist/yolupainter-<版>-<target>.zip` または `.tar.gz`。
実行ファイル、LICENSE、操作・動作環境を含む README、THIRD_PARTY.md、対象別の DEPENDENCIES.md と許諾全文を同梱します。
`tools/third-party.py` が対象ごとに許諾を照合し、未確認の依存や原文の不一致では束ねません。
更新クレートはまだアプリに接続していませんが、将来の組み込みに備えた許諾全文も保守的に含めます。
`xtask` 自体は配りません。独自の `CARGO_TARGET_DIR` は使わず、出力を `target/` に揃えてください。

`updater-json` の入力は、その版の配布アーカイブだけを置いた専用フォルダです。違う版や余分なファイルは拒否します。
URL は `https://github.com/YozoraKurage/YoluPainter-rs/releases/download/v<版>/<配布物名>` に固定です。
フォークから配る場合は、更新クレートの RELEASE_BASE も変更してビルドします。
署名なしの JSON は検査専用で、更新クレートは受理しません。

## 更新情報の互換

更新クレートは、署名が通った更新情報のうち知らない target の配布物を読み飛ばし、自分の target の配布物だけを厳密に確かめます。
macOS など対象を足した版の `updater.json` を、すでに配った版が拒否することはありません。配布物の数の上限は固定値（`MAX_ASSETS`）です。

次の変更は schema を上げる必要があります。既存のクライアントは schema が違う更新情報を拒否し、そのまま古い版に残ります。

- 本文の項目の追加・削除・意味の変更
- 既知の target の配布物の項目の追加・削除・意味の変更

schema を上げるときは、旧 schema を読むクライアントが取得する更新情報を旧形式のまま残します。
アプリを更新情報へ接続するときに、取得先の URL へ schema の番号を含め、新しい schema は新しい URL に置いてください。

## 更新署名の鍵（初回だけ）

配布の管理者が安全な手元の環境で実行してください。リポジトリの外に、アクセスを制限した保管先を用意します。

```sh
cargo xtask keygen --output /secure/location/update-private-key.hex
```

秘密鍵は 32 バイトの seed を hex にしたファイルです。既存ファイルは上書きしません。
Unix では作成権限を 0600 にします。Windows では保管フォルダの ACL を管理者本人に限定してください。
標準出力には公開鍵だけが表示されます。秘密鍵は Git、ログ、配布物に入れず、紛失に備えて安全にバックアップします。
公開鍵は 16 進の 64 文字で、後でアプリへ組み込みます。控えを失ったときは秘密鍵ファイルから取り出せます。

```sh
cargo xtask pubkey --key-file /secure/location/update-private-key.hex
```

アプリへ更新処理を接続するときは、公開鍵をビルド時の `YOLUPAINTER_UPDATE_PUBLIC_KEY` に設定し、
`UpdateClient::embedded` を利用します。現時点ではアプリにこの呼び出しはありません。
公開鍵未設定はエラーになります。更新サーバーから公開鍵を受け取る設計にはしないでください。
鍵の変更は既存アプリの信頼する公開鍵も変える必要があり、自動の鍵更新は未対応です。

手元で署名する場合は次のように実行します。秘密鍵を引数そのものに書かないでください。

```sh
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist --sign --key-file /secure/location/update-private-key.hex
```

`--key-file` を省くと環境変数 `YOLUPAINTER_UPDATE_PRIVATE_KEY` を読みます。
署名したら、公開鍵だけで、アプリと同じ検証（署名・版・大きさ・SHA-256）を通るかを確かめます。
別の鍵で署名した更新情報、配布物との食い違い、載っているのに無い配布物はここで失敗します。

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
   draft ジョブは署名の直後にこの公開鍵で `verify` を実行し、通らなければ Draft を作りません。
4. Actions の「配布物の作成」で配布対象のコミットを含むブランチ／タグを選び、kind を prerelease または stable にします。
5. 最初は **dry-run=true（既定）** で実行します。Windows・Linux をビルドし、未署名 JSON を含む `release-preview` artifact を作ります。secret に触れず、Release は作りません。
6. artifact を取得し、両 OS で展開・起動・同梱文書・許諾全文を確認します。Linux 実行ファイルには実行権限があります。
7. 同じコミットに対して dry-run=false で実行し、environment の承認を行います。署名付き JSON とアーカイブを **Draft Release** にアップロードします。
8. 署名と各配布物の SHA-256・サイズはワークフローが公開鍵で確認済みです。Draft のタグと対象コミット、版、prerelease の状態を確認します。
   本文は空で作られるので、両 OS の実機確認後、管理者が本文を書いて手で公開します。

同時実行は 1 本です。実行中の処理は自動取消しません。既存の同じタグの Release は上書きしません。
失敗後は Draft とタグの状態を確認してから再実行してください。非公開リポジトリでは認証と Actions の利用枠にも注意してください。
GitHub の prerelease 設定と SemVer のプレリリース識別子は別です。stable はプレリリース識別子を含む版を拒否します。

## 検証と現在の範囲

```sh
cargo test -p yolu-update -p xtask --locked
python3 tools/test-release-tools.py
actionlint .github/workflows/release.yml
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

更新クレートは HTTP の口を差し替える形で、署名確認・版比較・明示承認後のダウンロード・サイズと SHA-256 の確認までを提供します。
HTTP 実装は TLS 証明書、時間制限、読み込み中のサイズ上限、リダイレクト時の認証情報の扱いを守る必要があります。
現在の API は同期で、取得データをメモリに保持します。更新情報は 1 MiB、配布物は 2 GiB が上限です。
安定版だけを受け取る設定では SemVer のプレリリース版を提案しません。同じ版・古い版への更新は提案しません。
署名済みの過去の情報の再送による「新しい版を見せない」攻撃への鮮度保証はありません。
ダウンロードの承認はインストールの承認を兼ねません。自動展開・自分自身の置き換え・再起動は未実装です。

ワークフローの構成は [ALCOM の配布ワークフロー](https://github.com/vrc-get/vrc-get/blob/master/.github/workflows/publish-gui.yml)を参考にしています。
