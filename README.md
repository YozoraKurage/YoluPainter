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
