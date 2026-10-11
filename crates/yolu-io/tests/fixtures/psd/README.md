# PSDの人工フィクスチャ

`csharp.bin.gz` はUnity版の純C# `PsdCodec` と `Tests/Editor/Psd` の人工データから生成した正解集。ユーザーのファイルやPhotoshop/CSPで作った実データは含まない。出典コミットとソース指紋は `source.txt`。

## 再生成と試験

`UNITY_SOURCE` に Unity 版のソースの場所（Unity ブリッジのタグ `0.4.0` を取り出した場所。0.5.0 で `Runtime/Core` と `Tests/` が外れた）を設定し、リポジトリのルートで実行する。

```sh
python3 tools/psd-fixtures/generate.py --source "$UNITY_SOURCE"
cargo test -p yolu-io --test psd_io psd::
```

生成にはUnity同梱の.NET/Roslyn・Monoと、既存のNUnit DLLを使う。場所は `--unity-data`・`--nunit` で指定可能。Unityのエディタやテストデーモンは起動しない。Unity側は読み取りだけで、書き込みはRust側の `target/psd-fixtures/` とこのフォルダだけ。

再生成は Unity 版の PSD 試験を実行し、同じ入力と予算の組を SHA-256 で重複排除する。試験が失敗した場合は生成も失敗する。生成用のコードは `target/psd-fixtures/` に置き、入力側のソースは変更しない。

## データ形式

解凍後の形式はレコードの連続で、整数はリトルエンディアン。各レコードは次の順序:

1. C#試験名（i32長＋UTF-8）
2. 原本（i32長＋バイト）
3. 互換モード（i32: 編集0・原本保持1・拒否2）
4. 予算10項目（各i64: 原本、出力、寸法、キャンバス画素、レイヤー数、復号、メタデータ、名前、診断、グループ深さ）
5. 原本保持の有無（1バイト）
6. 診断コードのカンマ区切り（i32長＋UTF-8）
7. C#書き戻し（i32長＋バイト。長さ−1は無し）

## 比較対象

Rust の試験では互換モード・原本保持、拒否以外の診断コード集合、編集可能な入力の書き戻しバイト列を比較する。診断の文章は言語が異なるため比較しない。拒否結果の詳細コードは Rust 側では `MalformedOrLimit` にまとめる。保持専用・拒否の入力については、編集用データの取得と編集書き出しが拒否されることを確認する。

これらの人工データによる比較は、任意の PSD・全機能の組合せ・Photoshop/CSP の描画一致を保証するものではない。

## PSD の写し（core ⇔ PSD）の正解

`../../golden/psd/` は Unity 版の `PsdBridge.Export`・`Import` と `PsdCodec` に人工データを通した正解（マスク・グループ・塗りつぶし・調整・ロック・チャンネルごとの合成）。`<事例>.psd` は C# の書き出しのバイト列（取り込み事例は C# が作った PSD）、`<事例>.snap` はそれを C# で取り込んだ文書の中身、`<事例>.refused` は C# が書き出しを断る事例の理由。Photoshop/CSP の実データは含まない。台本は `tools/csharp-golden/PsdBridgeGolden.cs` と `tests/psd_io/psd_golden.rs` が同じものを持つ。

```sh
tools/csharp-golden/run.sh psd          # Unity 版のソースの既定の場所は /workspace。タグ 0.4.0 の場所を $YOLUPAINTER_UNITY_SOURCE か --source で渡す
cargo test -p yolu-io --test psd_io psd_golden::
```

Rust の試験は、同じ台本の文書を core で作って `from_core` → `write` した結果が C# の書き出しと全バイト一致することと、その PSD を `to_core` にした中身が C# の取り込みと同じこと、C# が断る書き出しを Rust も断ることを確かめる。往復・断り・ロック・ID の試験は `tests/psd_io/psd_m2.rs`。
