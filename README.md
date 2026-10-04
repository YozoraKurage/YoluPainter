# YoluPainter-rs

YoluPainter のスタンドアロン版（Rust）。描くのはこのソフトの窓で、Unity は塗った絵を lilToon などの本物のマテリアルで見せる
（Unity のパッケージ YoluPainter とつなぐ）。まだ試作の前の骨組み。

## 構成（Cargo の作業場）

| クレート | 役目 |
|---|---|
| `yolu-core` | 文書（タイル・レイヤー・チャンネル）と CPU の合成・ブラシ・フィルター、3D の面の計算（当たり・BVH・面の上のダブ）。GPU にも OS にも頼らない |
| `yolu-gpu` | wgpu での合成・ブラシ・ベイク（CPU の経路と照らし合わせる） |
| `yolu-io` | .ylp（Unity 版と同じ形式）・PSD・画像の読み書き |
| `yolu-protocol` | スタンドアロンと Unity のブリッジのあいだの通信の形 |
| `yolu-bridge` | Unity のエディタ拡張が読む DLL（薄い。スタンドアロンにつなぐだけ） |
| `yolu-app` | スタンドアロンの描画ソフト（egui） |
| `yolu-link-demo` | Live Link の試しのスタンドアロン（yolu-app が載るまで。マテリアルごとに試しの模様を描いて返す） |

## 組む・試す

```
cargo build
cargo test
```

## Unity 版へ入れるブリッジ

`tools/build-bridge.sh <Unity 版のパッケージの根>` が yolu-bridge を Linux（.so）と Windows（x86_64-pc-windows-gnu の .dll、mingw-w64 が要る）に
組み、csbindgen の C# の宣言（`crates/yolu-bridge/generated/LiveLinkNative.g.cs`）と一緒に `Plugins/LiveLink` と `Editor/LiveLink/Native` へ写す。
ブリッジの C の関数の意味を変えたら `ABI_VERSION`（`crates/yolu-bridge/src/ffi.rs`）と Unity 版の `LiveLinkBridge.ExpectedAbi` を一緒に上げる。

## C# 版との照合（正解のファイル）

`yolu-core` の合成（グループ・マスク・塗りつぶし・調整・クリッピング・チャンネルごとの合成・Normal のベクトルの合成と出力を含む）・
ブラシ・Undo は、Unity 版の C# の Core とバイト一致を確かめている。正解のファイル（`crates/yolu-core/tests/golden/`）は
`tools/csharp-golden/run.sh` で作り直す（Unity 版のリポジトリと、Unity に同梱の .NET・Mono が要る。事例は両方が読む `cases.txt`、
命令の書き方はその頭）。`run.sh bench` は C# の速さを、`cargo run --release -p yolu-core --example bench` は同じ中身の Rust の速さを測る。
3D の面の計算（`yolu_core::geometry`）は、Unity 版の SurfaceGeometry とビットで一致を確かめている。正解（`crates/yolu-core/tests/golden/surface/`）は
`tools/csharp-golden/run.sh surface` で作り直す（Unity 版の Editor/Preview の原文を、Unity に同梱の UnityEngine.CoreModule.dll と組む）。

## 許諾の一覧と配布用の全文

[THIRD_PARTY.md](THIRD_PARTY.md) に Windows GNU 向けの製品別一覧、フォント・アイコン、開発用の道具の許諾を記載している。
Python 3.10 以降と Cargo が必要（追加の Python パッケージや cargo-about の導入は不要）。

```sh
python3 tools/third-party.py --bundle
# ブリッジだけを作る場合
python3 tools/third-party.py --package yolu-bridge --bundle
```

`target/third-party/<クレート>/` に一覧と、照合に成功した場合だけ `THIRD_PARTY_LICENSES.txt` ができる。
全文を該当する配布物と一緒に入れる。初回はクレートと、同梱されていない原文の取得にネット接続が必要。
取得後は `--offline` で再照合できる。原文は発行時コミットと SHA-256 で固定され、版の変更・原文の欠落・未承認の許諾では終了 1 になる。
2026-10-04 にユーザーが BSL-1.0（clipboard-win・error-code）と Hack 書体の Bitstream Vera 条件を許可した。
発行時の原文と SHA-256 を照合し、app・bridge とも全文束を生成できる。
依存を更新したときは `tools/licenses-reviewed.json` の原文・条件を確認し、一覧も更新する。

## Windows 向けの画面なし試験（Wine）

Linux 上で Python 3.10 以降・Wine・MinGW-w64 を用意し、Rust の対象を追加する。

```sh
rustup target add x86_64-pc-windows-gnu
tools/wine-tests.sh
# 古い Wine で bcryptprimitives.dll が不足する場合だけ
tools/wine-tests.sh --compat-bcrypt
```

core・io・protocol・bridge の単体／結合試験と、app の単体試験・名前が `headless_` で始まる結合試験を実行する。
ビルド結果から対象を選ぶので、以前の試験 exe を混ぜない。ログ・成功／失敗／無視の件数・除外した試験の名前は
`target/wine-tests/summary.json` と同じディレクトリのログに残る。ビルド失敗・実行失敗・時間切れは終了 1。
試験ごとの時間制限は `--timeout 180`（秒）で変更できる。専用 Wine 環境も `target/` 内に作る。

wgpu／egui_kittest の画面の試験と `yolu-gpu` は Wine の描画バックエンドでの動作を検証できないため対象外。
描画の検証はネイティブ環境の `cargo test` で別に行う。Wine の結果は Windows 実機、ペンタブ、Unity との接続確認の代わりにはならない。
`--compat-bcrypt` の DLL は試験専用で、配布しない。
