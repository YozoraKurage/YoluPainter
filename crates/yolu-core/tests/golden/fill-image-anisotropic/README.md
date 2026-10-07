# 塗りつぶし投影の異方性の正解（Rust）

`../fill-image/` と同じ入力（`tests/support/fill_cases.rs` の 6 つの投影 × 8 つの入力と、デカールの値）を、画像とデカールの形の両方を異方性で読んで（`FillInput::anisotropic`・`shape_anisotropic` を立てて）描いた結果です。名前と形式（37×29、straight RGBA8、左下からの行優先）は `../fill-image/` と同じです。

C# の評価式には異方性の読みが無いので、Rust の出力を正解にしています。撮り直しは `YOLU_GOLDEN_UPDATE=1 cargo test -p yolu-core --test reference rust_anisotropic` で、スレッド 1 の出力で書き直し、スレッド 2・4 とタイルに分けた描画が同じバイトかはそのまま確かめます。撮り直したら差分を見て、意図した変化だけかを確かめてください。

足跡が丸い画素（UV の素の写し・拡大）は異方性を入れても切った道と同じバイトなので、`0-0` など一部のファイルは `../fill-image/` と同じ中身です。
