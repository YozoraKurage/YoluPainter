# 互換性の正解データ

すべて人工的な試験用データ。ユーザーの作品やモデルは含まない。

| ファイル | 由来・検証する範囲 |
|---|---|
| `format1.ylp` 〜 `format6.ylp` | Unity版の `Tests/Editor/Persistence/Fixtures~/`。旧形式の移行、画像・スマートリソース・ブラシ、ZIPの全エントリとmanifestのバイト一致 |
| `format5-shared-materials.ylp` | 同じUnity版の試験。3セットと共有するスマートマテリアルの保持 |
| `native-v1.utpaint` 〜 `native-v21.utpaint` | C#のBinaryWriterで各版の基礎配置を組み、Unity版 `DocumentBinary.Read` が受理することを確認。画素・透明RGB・日本語名・版2以降のマスク・版7以降のノーマル設定を含む |
| `native-rich-v21.utpaint` | C#の実際の `DocumentBinary.Write` が出力。全チャンネル、マスク、全フィルター種別、全Generator種別、固定キー・ID色、形状・ランプ、画像・デカール、調整、2D/3Dマテリアルパス、グループ、ロック、Anchor、手動ID色。C#自身の読んで再保存もバイト一致 |
| `m1-mode-00.ylp` 〜 `m1-mode-25.ylp` と同名PNG | Unity 0.2.0のCore（形式7・正本21）の実際の書き手とCPU合成。26合成モード、連続クリッピング、最下層のクリッピング印、非表示層、半透明、透明RGB、日本語名、17×11画素・8タイルの外周。00はmaterial全種類と先頭でない現在セットも含む。 |
| `m1-pattern.ylp` と同名PNG | 65×33画素の縞とグラデーション。圧縮が効くdeflateストリームでもC#出力と全バイト一致することを検証。 |
| `selection-v1.bin` | C#の `SelectionBinary.Write` による選択範囲 |

旧正本の基礎21件は各旧版アプリが実際に出力したものではない。拡張機能は正本21の書き手で確認しており、全版と全属性の組合せを網羅したという意味ではない。

C#で追加したフィクスチャの再生成:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE"
```

`UNITY_SOURCE` はUnity版のソース。読み取りだけ行う。Unity同梱のRoslynとMonoを使い、エディタやテストデーモンは起動しない。ビルド結果はRust側の `target/io-fixtures/`、生成物はこのフォルダ。`DocumentBinary.CurrentVersion` が21でなければ生成を断る。

元の形式1〜6のZIPはこのツールでは作り直さない。新しい試験データを追加するときも、原本のファイルを上書きして互換性の基準を置き換えない。

形式7の合成データの再生成:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --m1
```

保存済みの同じ27件の `.ylp` を、C#の `YlpFormat.Open` → `DocumentBinary.Read` → `Composite(Color)` → `RgbaPng.Encode` とRustの `composite_png` 例でそれぞれ開いて出力し、PNGファイル全バイト（画素だけでなく圧縮・CRCも）を再比較する:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --verify
```

検証の出力は `target/io-fixtures/comparison/`。既存の正解データは変更しない。C#出力が保存済みのPNGから変わった場合も検証を失敗させる。小さい人工画像27件の検証であり、すべての画像サイズ・圧縮ランタイムでのバイト一致を保証するものではない。

PSDの入力・互換判定・書き戻し正解は [psd/README.md](psd/README.md) を参照。
