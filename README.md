# YoluPainter-rs

YoluPainter のスタンドアロン版（Rust）。描くのはこのソフトの窓で、Unity は塗った絵を lilToon などの本物のマテリアルで見せる
（Unity のパッケージ YoluPainter とつなぐ）。まだ試作の前の骨組み。

## 構成（Cargo の作業場）

| クレート | 役目 |
|---|---|
| `yolu-core` | 文書（タイル・レイヤー・チャンネル）と CPU の合成・ブラシ・フィルター。GPU にも OS にも頼らない |
| `yolu-gpu` | wgpu での合成・ブラシ・ベイク（CPU の経路と照らし合わせる） |
| `yolu-io` | .ylp（Unity 版と同じ形式）・PSD・画像の読み書き |
| `yolu-protocol` | スタンドアロンと Unity のブリッジのあいだの通信の形 |
| `yolu-bridge` | Unity のエディタ拡張が読む DLL（薄い。スタンドアロンにつなぐだけ） |
| `yolu-app` | スタンドアロンの描画ソフト（egui） |

## 組む・試す

```
cargo build
cargo test
```

## C# 版との照合（正解のファイル）

`yolu-core` の合成・ブラシ・Undo は、Unity 版の C# の Core とバイト一致を確かめている。正解のファイル（`crates/yolu-core/tests/golden/`）は
`tools/csharp-golden/run.sh` で作り直す（Unity 版のリポジトリと、Unity に同梱の .NET・Mono が要る。事例は両方が読む `cases.txt`）。
