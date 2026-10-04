# 塗りつぶし投影の C# 正解

`tools/csharp-golden/fill.sh golden` で再生成します。すべて人工データです。

`<mode>-<variant>.rgba` は37×29、straight RGBA8、左下からの行優先です。mode は0 UV、1トライプラナー、2平面、3球面、4円柱、5デカール。variant 0〜7 の入力は `tools/csharp-golden/FillGolden.cs` と Rust の `tests/support/fill_cases.rs` にあります。評価式は C# の原文を呼び出し、ここで再実装していません。

`decal-values-<variant>.rgba` はデカール（mode 5）の `ApplyDecalToValue`。同じ variant 0〜7 の設定に、座標だけで決まる人工の値（`tests/support/fill_cases.rs` の `decal_values`）を渡した結果で、形式は上と同じです。

`.rgba` は改行を変換してはいけないバイト列なので、リポジトリの `.gitattributes` で `binary`、このフォルダ全体を `-text` にしています。

`source.txt` は参照元コミットと評価式のSHA-256、`contracts.txt` は拒否設定と予算境界の実行結果です。生成器はC#の並列度1と4の全バイト一致を確認してから保存します。
