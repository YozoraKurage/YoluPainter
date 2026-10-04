# 開発用の手順

以下はリポジトリのルートで実行します。通常のビルドと操作は [README](../README.md) を参照してください。

## 試験

```sh
cargo test --workspace --locked
```

GPU・画面の試験には動作する描画バックエンドが必要です。GPU 試験にはアダプターがないと処理を省くものがあるため、結果の passed だけで描画確認済みとは判断せず、標準エラーの理由も確認してください。詳しくは [yolu-gpu](../crates/yolu-gpu/README.md#検証と計測) を参照してください。

Unity 版 C# との照合には、リポジトリに収録された人工データを使います。core の正解の再生成ツールは `tools/csharp-golden/run.sh` です（出力先は `--out` で指定できます）。編集できるパスの正解は `tools/csharp-golden/run-paths.sh <出力先>` で作り、試験が読む `crates/yolu-core/tests/golden/paths` へ出力します。効果と層のロック・層の操作のつなぎ目の正解（事例ごとの SHA-256）は `tools/csharp-golden/run-seam.sh` で `crates/yolu-core/tests/golden/seam.txt` へ作ります。食い違ったときは、試験を `SEAM_DUMP_DIR=<フォルダ>` で回して Rust の生のバイト列を書き出し、`run-seam.sh dump <事例名> <出力>` の C# の側と `cmp` で比べます。どれも Unity 版のソースと Unity 同梱の .NET・Mono が必要です。PSD の写し（core ⇔ PSD）の正解は `tools/csharp-golden/run.sh psd` で作ります。I/O と PSD のデータ形式・再生成方法は [I/O のフィクスチャ](../crates/yolu-io/tests/fixtures/README.md)と [PSD のフィクスチャ](../crates/yolu-io/tests/fixtures/psd/README.md)を参照してください。ブラシ形式の取り込みの正解は `tools/csharp-golden/brushes.sh` で作り（Rust の試験が入力を書き、C# の読み手に通して `crates/yolu-io/tests/fixtures/brushes/` へ出力）、違いの調査は `BRUSH_GOLDEN_SHOW=<記録の番号>` でその入力の完全な指紋を出します。

## CI

`.github/workflows/ci.yml` は `main` への push・`pull_request`・`workflow_dispatch` で起動します（同じブランチの古い実行は取り消します）。外の Actions はコミットの SHA で固定し、版の名前をコメントに書いています。上げるときは、その版のタグが指すコミットを確かめてから SHA を書き換えます。

- Linux（`ubuntu-latest`）: `cargo test --workspace --locked` と `cargo clippy --workspace --all-targets --locked -- -D warnings`。Xvfb、Mesa とビルド用のパッケージを導入し（画面の書体はアプリに同梱しているので、OS の書体は入れません）、`WGPU_BACKEND=gl`、`LIBGL_ALWAYS_SOFTWARE=1`、`GALLIUM_DRIVER=llvmpipe` でソフトウェア描画を選びます。試験は同時の描画負荷を抑えるため直列に実行し、`--nocapture` で GPU 試験が省かれた理由もログに残します。
- Windows（`windows-latest`、MSVC）: `cargo build -p yolu-app --locked`、core・io・protocol・bridge・link-demo の試験、app の `--lib` と `--test livelink headless_`・`--test brush_list headless_`・`--test update headless_`・`--test recovery headless_`（復旧の OS のロックと置換）。GPU・画面の統合試験は対象外です。
- 両 OS で [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache) を使い、同じブランチの古い CI は後続の実行で取り消します。

`cargo fmt --check` は既存の `crates/yolu-core/src/geometry/query.rs` に整形差分があるため、まだ必須検査にしていません。コードの整形を別途済ませてから追加してください。

Unity 版 C# を実行する正解の再生成・照合は、Unity 版のソースと Unity 同梱の .NET・Mono が必要なため、この CI では回しません。収録済みの人工データを使う Rust の照合試験は通常の `cargo test` に含みます。

CI の定義は `actionlint .github/workflows/ci.yml` で実行せずに検査できます。初回の実行では、clippy の既存警告、Ubuntu の Mesa の版による画面の正解との差、GPU 試験が省かれていないかを確認してください。ソフトウェア描画での結果は、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。

## Unity 用ブリッジ

Linux 上で Bash、MinGW-w64 と Rust の `x86_64-pc-windows-gnu` ターゲットを用意します。`tools/build-bridge.sh` を引数なしで実行すると、Linux と Windows のブリッジと C# 宣言を `target/bridge-out/` に作ります。Mac 用は生成しません。

Unity 版へ組み込む場合は、対応する Unity パッケージのルートをスクリプトの引数に指定します。`Plugins/LiveLink/` と `Editor/LiveLink/Native/` の既存の配置先へコピーするので、変更先を確認してから実行してください。読み込み済みのネイティブライブラリを更新した後は Unity を再起動します。ブリッジの ABI を変えるときは Rust の `ABI_VERSION` と Unity の `LiveLinkBridge.ExpectedAbi` を合わせます。

## Windows 向けの画面なし試験（Wine）

Linux 上で Python 3.10 以降、Wine、MinGW-w64 と Rust の `x86_64-pc-windows-gnu` ターゲットを用意し、`tools/wine-tests.sh` を実行します。古い Wine で `bcryptprimitives.dll` が不足する場合だけ `--compat-bcrypt` を付けます。時間制限は `--timeout 180` のように秒で指定できます。

core・io・protocol・bridge と app の画面なし試験が対象です。`--package yolu-protocol --package yolu-bridge` のようにクレートを絞れます（全部を組むと wgpu・egui まで Windows 向けに組むため、Live Link の通信だけを確かめたいとき用）。ログと結果は `target/wine-tests/summary.json` と同じフォルダに残ります。GPU・画面の試験は対象外で、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。互換 DLL は試験専用で、製品に同梱しません。

Windows のインストーラー（NSIS）を画面なしで通す試験は `python3 tools/test-installer.py`（Wine・MinGW-w64・`makensis` が要る。内容は [RELEASING](RELEASING.md#windows-のインストーラー)）です。`tools/wine-tests.sh` の app の試験に含まれる通信の試験（`update::http`）は、同じ機械の `http://127.0.0.1` に立てた小さなサーバーへ、Windows では WinHTTP の本物で接続します。

Wine は DACL（Live Link の鍵・名前付きパイプ・共有メモリのファイルを自分だけにする設定）をファイルやフォルダに保存しないので、`yolu-protocol` の DACL の中身を調べる試験（`private::windows_tests`）は Wine では呼び出しが通ることまでを見て、中身は本物の Windows の `cargo test -p yolu-protocol` で確かめます。別のユーザーとして開けないことを確かめる試験（`another_user_can_neither_connect_nor_read_the_files`）は、Linux で `sudo -n -u nobody` が使えるときだけ走ります。

## 配布用の許諾全文

Python 3.10 以降と Cargo を使います。

```sh
python3 tools/third-party.py --bundle --offline
python3 tools/third-party.py --package yolu-bridge --bundle --offline
```

初回など依存や原文が取得済みでない場合は `--offline` を外して実行します。照合に成功すると `target/third-party/<クレート>/THIRD_PARTY_LICENSES.txt` ができるので、該当する配布物に同梱します。追加の Python パッケージは不要です。

`--target x86_64-pc-windows-msvc` または `--target x86_64-unknown-linux-gnu` を指定すると、対象別に照合して `target/third-party/<target>/<クレート>/` へ出力します。省略時は従来の Windows GNU が対象です。`--package yolu-update` で更新クレートも確認できます。配布の詳細は [RELEASING](RELEASING.md) を参照してください。`tools/licenses-reviewed.json` に原文と SHA-256 を記録し、原文の欠落・変更や未確認の版・許諾では生成を失敗させます。製品ごとの範囲とクレート以外の表記は [THIRD_PARTY.md](../THIRD_PARTY.md) にあります。
