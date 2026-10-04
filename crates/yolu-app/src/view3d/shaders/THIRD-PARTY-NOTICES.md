# 3D ビューの lilToon の再現の第三者表記

`liltoon.wgsl` と、`crates/yolu-app/src/look/liltoon.rs` のプロパティの既定値・名前は、lilToon 2.3.4 の
シェーダー（`Shader/Includes`）とインスペクターから式と値を WGSL・Rust に移したもの。lilToon のファイル・テクスチャ・
画像は同梱しない。配布元: [lilToon](https://github.com/lilxyzw/lilToon)（版 2.3.4、
`https://github.com/lilxyzw/lilToon/releases/download/2.3.4/jp.lilxyzw.liltoon-2.3.4.zip`）。

ビルトインのレンダーパイプラインの光の式は、lilToon に同梱の OpenLit Library 1.0.2（`openlit_core.hlsl`）から移した。
OpenLit の許諾は [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/)（表記の義務は無い。出どころとして記す）。

lilToon の許諾の原文（パッケージの `LICENSE`、SHA-256 `7ca2e241979e77241d90ce5c122dc2f5128881ed5af3538eb9aa1e61f35d2660`）:

```
MIT License

Copyright (c) 2020-present lilxyzw

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
