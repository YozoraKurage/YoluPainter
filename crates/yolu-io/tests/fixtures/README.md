# 互換性の正解データ

すべて人工的な試験用データ。ユーザーの作品やモデルは含まない。

C# の書き手・読み手の正解は、Unity ブリッジのタグ `0.4.0` 以前の `Runtime/Core` で作った物（0.5.0 の Unity ブリッジは .ylp を扱わず、`Runtime/Core` と `Tests/` を外した）。再生成するときは、そのタグを取り出して、下の `UNITY_SOURCE` に渡す。

| ファイル | 内容・比較対象 |
|---|---|
| `format1.ylp` 〜 `format6.ylp` | Unity版（タグ `0.4.0` まで）の `Tests/Editor/Persistence/Fixtures~/`。旧形式の移行、画像・スマートリソース・ブラシ、ZIPの全エントリとmanifestのバイト一致 |
| `format5-shared-materials.ylp` | 同じUnity版の試験。3セットと共有するスマートマテリアルの保持 |
| `native-v1.utpaint` 〜 `native-v21.utpaint` | 各版の基礎配置と Unity 版 `DocumentBinary.Read` との互換性。画素・透明RGB・日本語名・版2以降のマスク・版7以降のノーマル設定を含む |
| `native-rich-v21.utpaint` | C# の `DocumentBinary.Write` による正本。全チャンネル、マスク、全フィルター種別、全Generator種別、固定キー・ID色、形状・ランプ、画像・デカール、調整、2D/3Dマテリアルパス、グループ、ロック、Anchor、手動ID色。読んで再保存したバイト列を比較 |
| `m1-mode-00.ylp` 〜 `m1-mode-25.ylp` と同名PNG | 形式7・正本21の書き手と CPU 合成。26合成モード、連続クリッピング、最下レイヤーのクリッピング印、非表示レイヤー、半透明、透明RGB、日本語名、17×11画素・8タイルの外周。00はmaterial全種類と先頭でない現在セットも含む。 |
| `m1-pattern.ylp` と同名PNG | 65×33画素の縞とグラデーション。圧縮が効くdeflateストリームでもC#出力と全バイト一致することを検証。 |
| `selection-v1.bin` | C#の `SelectionBinary.Write` による選択範囲 |
| `effects-filters`・`effects-generators`・`effects-anchors`・`effects-fills`・`effects-paths` の `.utpaint`・`.composite`・`.layers` と `effects-inputs.bin` | C# の実際の書き手（`tools/io-fixtures/EffectFixture.cs`、正本21）が作った、効果（フィルターのスタック・Generator・Anchor・塗りつぶしの画像と投影・グラデーション・パス）の入った正本。`.composite` は全チャンネルの合成（合成の式が f32 になってから core で撮り直した）、`.layers` はレイヤー・マスクごとの評価した出力、`effects-inputs.bin` は評価に渡す人工の入力（メッシュマップ・モデルのルート・画像）。`tests/ylp/effects_bridge.rs` が使う |
| `effects-legacy-v9` 〜 `effects-legacy-v20` の `.utpaint`・`.v21`・`.composite` | 効果の入った古い版（9〜20）の並びの正本を C# の読み手で読み、`DocumentBinary.Write` で書き直した版21（`.v21`）と合成。版ごとの読みと移行を確かめる |
| `rust-written-effects-v21.utpaint`・`.unity.txt`・`.layers`・`.composite` | 編集 API で作った効果入りの版21を Rust の `from_core` が書いたもの（固定の ID。試験が同じバイト列を作ることを確かめる）と、Unity 0.2.0 の読み手に読ませた記録・評価した出力・合成 |
| `procedural-v23.utpaint`・`.unity.txt` | Rust の `from_core` が書いた版23（ノイズ・グランジ）と、Unity 0.2.0 の読み手に読ませた記録（「Unsupported archive version」で断る）。`tests/ylp/procedural_bridge.rs` |
| `smart/` | `.ylsmart` と索引の人工データ（[smart/README.md](smart/README.md)） |
| `m2-groups.utpaint`・`m2-masks.utpaint`・`m2-channels.utpaint`・`m2-clipping.utpaint`・`m2-tiny.utpaint` と同名 `.composite` | Unity 0.2.0のCore（正本21）の実際の書き手（`tools/io-fixtures/M2Fixture.cs`）。グループ（通過・分離・入れ子・空・非表示・クリップ）、ラスターマスク（有効・反転・濃度・画素なし・グループのマスク）、チャンネルごとの有効と合成、塗りつぶし・調整（反転・レベル補正・色相/彩度/明度）、Normalの設定、クリッピング、1画素。`.composite` はC#の全チャンネルの合成（番号の順、下の行から）に続けてNormalのファイル出力。ただし m2-groups・m2-masks・m2-channels の `.composite` は、合成の式が f32 になってから core の出力で撮り直した物（`YOLU_GOLDEN_UPDATE=1 cargo test -p yolu-io --test ylp m2_bridge`。C# の出力とは 4〜23 バイトが違い、差は最大 1〜2 段）。m2-clipping・m2-tiny は C# の出力のまま |
| `locks-v21.utpaint` と同名 `.composite` | Unity 0.2.0のCore（正本21）の実際の書き手（`tools/io-fixtures/M2Fixture.cs` の `--locks`）。レイヤーのロック（版12の属性の印のビット1と直後のint）: 個別4種・重ね・すべて・グループ・塗りつぶし・調整・クリッピングとチャンネルごとの合成との同居・ロックの無いレイヤー。ロックは合成を変えない（`.composite` はロックを外しても同じ） |
| `rust-written-locks-v21.utpaint`・`.unity.txt`・`.composite` | `locks-v21` を編集してロックを付け外し・複製した版21をRustの `from_core` が書いたもの（固定のIDで、試験が同じバイト列を作ることを確かめる）と、それをUnity 0.2.0の `DocumentBinary` に読ませた記録（読めて、書き直すと同じバイト列、レイヤーごとの自分のロックと効くロック）、C#の全チャンネルの合成 |
| `user-channels-v22.utpaint` | Rustの `from_core` が書いた版22（ユーザーチャンネル3つ、番号6・8・9）。C#に書き手が無いので、この書き手のバイト列を正解として固定する（試験が同じバイト列を作ることを確かめる） |
| `user-channels-v22.unity.txt` | 上の版22を、Unity 0.2.0の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open`（`Runtime/Core` をそのままコンパイル）に読ませた結果。`TexturePaintWindow.ReadTextureSets` の行は、ウィンドウが正本の読みの失敗に付ける文を、読み手の例外から同じ形に作ったもので、行の名前もそう記す |
| `adjust-v24.utpaint` | Rustの `from_core` が書いた版24（色調補正の6種: グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2値化・ポスタリゼーションを、調整レイヤー6つと塗りつぶしレイヤーのフィルターの段6つ・マスクの段1つに）。C#に書き手が無いので、この書き手のバイト列を正解として固定する（試験が同じバイト列を作ることを確かめる）。作り直しは `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp adjust_bridge::version_24` |
| `adjust-v24.unity.txt` | 上の版24を、Unity 0.2.0（タグ `0.2.0` の `Runtime/Core` をそのままコンパイル）の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open` に読ませた結果（`generate.py --adjust`）。版22・23と同じく「Unsupported archive version」で断る記録 |
| `rust-written-v21.utpaint` | Rustの `from_core` が書いた版21（`m2-groups` を開いて、塗りつぶし・調整・マスク・複製したグループ・チャンネルごとの合成を編集したもの）。ユーザーチャンネルが無い文書をUnity 0.2.0が読めることの正解 |
| `rust-written-v21.unity.txt`・`rust-written-v21.composite` | 上の版21をUnity 0.2.0の `DocumentBinary.ReadId` / `Read` に読ませ、`Write` で書き直したバイト列が元と同じか、レイヤーの数を記録したものと、C#の全チャンネルの合成（`m2-*.composite` と同じ並び） |
| `unity-generation/` | Unity 0.2.0 の `GenerationStore.Commit`（`Runtime/Core` をそのままコンパイル）が書いた復旧用の置き場。確定 2 回（正本・選択範囲・resources・`recovery.json`。変わらない中身は共有）。Rust の `GenerationStore` が読めること・一覧に出せること・続けて確定できることの正解 |
| `rust-generation.unity.txt` | 開いた `format6.ylp` のエントリをそのまま世代にした置き場（`as-opened`）と、`sets/<ID>/` の下の入れ子の名前を除いたもの（`flat-names`）を、Unity 0.2.0 の `GenerationStore.Load` に読ませた結果。前者は「Unsafe generation filename」で断られ、後者は読める（Rust の世代の名前の範囲が Unity 版より広い記録） |
| `selection/selection-*.bin` と `.amounts` | 70×50画素・タイル16の文書で作った選択範囲（矩形・楕円・多角形の組み合わせ、ぼかし、全選択、反転、何も選ばない）のC#の `SelectionBinary.Write` の出力と、キャンバスの量の生の並び（下の行から）。Rustで同じ選択範囲を作り、書いたバイト列が全バイト一致することと、読んで同じ量に戻ることを確かめる |
| `brushes/cases.txt`・`brushes/fuzz.txt`・`brushes/source.txt` | ブラシ形式の取り込み（`tests/brushes/brush_golden.rs`）。Rust の試験が作った入力（手で作った 88 事例と、同梱の Krita の GIMP 形式の実ファイル 37 個（`.gbr`・`.gih`）と、手で作った事例を壊した約 1.1 万の入力。`tests/brush_files/corpus.rs`）を、Unity 版の `GimpBrushReader`・`PhotoshopBrushReader`・`PhotoshopPatternReader`（`Runtime/Core` を原文のままビルドする）に通した結果。`cases.txt` は事例ごとの取り込めたか・断ったかと設定の指紋、`fuzz.txt` は壊した入力ごとの結果と指紋の短縮、`source.txt` は C# の原文の指紋。`tools/csharp-golden/brushes.sh` で作り直す |
| `brushes/png-tips.txt` | 同梱の Krita の PNG の筆先 39 個の被覆率（暗いほど塗り、白と透明は塗らない）の指紋。別の復号器（Pillow）で求める（`tools/brush-fixtures/png_tips.py`）。`tests/brushes/brush_bundled.rs` が Rust の読み手と比べる |

旧正本のデータは各版の基礎配置を表す人工データであり、全版と全属性の組合せを網羅するものではない。

## 再生成

リポジトリのルートで実行する。

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE"
```

`UNITY_SOURCE` はUnity版のソース（Unity ブリッジのタグ `0.4.0` を取り出した場所）。読み取りだけ行う。Unity同梱のRoslynとMonoを使い、エディタやテストデーモンは起動しない。ビルド結果はRust側の `target/io-fixtures/`、生成物はこのフォルダ。`DocumentBinary.CurrentVersion` が21でなければ生成を断る。

元の形式1〜6のZIPはこのツールでは作り直さない。新しい試験データを追加するときも、原本のファイルを上書きして互換性の基準を置き換えない。

復旧用の世代（`unity-generation/` と `rust-generation.unity.txt`。書くたびに世代の名前の時刻が変わる）の再生成:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --generation
```

レイヤーの種類・チャンネルの正本5件と全チャンネルの合成の再生成（出力は同じバイト列になる）:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --m2
```

Rustが書く正解の正本（版22と版21）と、Unity 0.2.0の読み手の記録の取り直し。正本を作り直したときだけ:

```sh
YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp -- m2_bridge::version_22_fixture m2_bridge::version_21
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --user-channels --rust-written
```

レイヤーのロックの正本（C#の書き手）と、Rustが書いたロックつきの版21をUnity版の読み手に読ませる記録の取り直し（Rustが書く正本は `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp layer_locks::rust_written` で作り直してから）:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --locks
YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp layer_locks::rust_written
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --rust-written-locks
```

形式7の合成データの再生成:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --m1
```

保存済みの `.ylp` を、C#の `YlpFormat.Open` → `DocumentBinary.Read` → `Composite(Color)` → `RgbaPng.Encode` とRustの `composite_png` 例でそれぞれ開いて出力し、PNGファイル全バイト（画素だけでなく圧縮・CRCも）を再比較する:

```sh
python3 tools/io-fixtures/generate.py --source "$UNITY_SOURCE" --verify
```

検証の出力は `target/io-fixtures/comparison/`。既存の正解データは変更しない。C#出力が保存済みのPNGから変わった場合も検証を失敗させる。人工画像を使った比較であり、すべての画像サイズ・圧縮ランタイムでのバイト一致を保証するものではない。

選択範囲の正解（`selection/`）の再生成（Unity同梱のRoslynとMonoを使う）:

```sh
tools/csharp-golden/run.sh selbin
```

効果入りの正解（`effects-*`）・Rust が書く効果入りの版21・手続き型の版23・色調補正の版24の読み手の記録は、同じ `generate.py` の `--effects`・`--rust-written-effects`・`--procedural`・`--adjust` で取り直す
（Rust が書く正本は、それぞれ `YOLU_UPDATE_FIXTURES=1 cargo test -p yolu-io --test ylp effects_bridge::rust_written_effects_fixture_is`・`procedural_bridge::version_23_fixture`・`adjust_bridge::version_24` で作り直してから）。

PSDの入力・互換判定・書き戻し正解は [psd/README.md](psd/README.md) を参照。
