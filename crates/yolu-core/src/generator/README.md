# Generator・ランプ・Anchor の画像評価

`Settings` を `BoundGenerator::bind` で検査し、読み取り専用の入力に束縛する。`evaluate` は `Source` の指定領域から新しい RGBA8 画像を返す。
`Image` は連続した画像用。タイル入力は `Source` を実装してキャンバス座標で画素を返す。領域の切り方・Rayon のプールの並列度で計算結果は変わらない。
入力と設定は借用するため、評価中のスナップショットは変更できない。成功した画像だけを呼び出し側で採用する。

対応する種類は EdgeWear、Dirt、PositionGradient、Thickness、Direction（ワールド／ベント法線）、ShapeGradient、IdColor、Anchor に、Rust 版だけの Noise（64）・Grunge（65）。
ノイズは各種類に重ねる4オクターブの値ノイズで、UV またはモデル空間を使う。ShapeGradient は箱・球・平面、ルートの位置と回転、形の中心・回転・大きさ・減衰を持つ。
`Ramp` は独立した RGB と不透明度の分岐点、中点、PCHIP の値カーブ、5種類のプリセットを持つ。色は保存された sRGB 値のまま補間し、透明な画素の RGB を保つ。
色の分岐点の α は評価に使わず、C# の `GradientStop` と同じく `Ramp::new` が 255 にそろえる（不透明度は独立した分岐点が持つ）。α の違う入力から作ったランプは `==` で等しい。
スカラーとマスクのランプは輝度 0.2126/0.7152/0.0722、Anchor の色読み取りは C# と同じ 0.3/0.59/0.11 を使う。

マップは C# Core と同じ u16 の成分と被覆を渡す。`MapState` は呼び出し側がベイクの由来と現在のモデルを照合した結果を表す。
生成・由来の計算・更新監視は含まない。欠損、古い、未検証、サイズ違い、ピン不一致、ルート不明、境界箱の大きさゼロ、ID 未選択、Anchor 不可は
`Inactive` として返し、段の入力を通す。被覆ゼロの画素も入力を通す。壊れた設定・画像長・由来の形式・非有限値は `Error` として断る。
極端な有限座標の演算が無限大になる場合も断る（C# のモデルフレームで巨大な四元数を正規化できてしまう場合より厳格）。

`BoundGenerator::sample` は `Generated`（ランプなしは `Scalar(f64)`、ランプ付きは評価済み straight RGBA8 の `Mapped([u8; 4])`）を返す。
スカラーは常に有限の 0..1 で、値を持たない画素（被覆ゼロ・入力を通す段）は `None`。フィルターの `GeneratorInput` が受ける `Generated` と同じ形なので、
`sample(slot, x, y)` から束縛済みの `BoundGenerator::sample(x, y, scalar)` の結果をそのまま返せる（列挙の写し替えは要らない）。
フィルターなど別の評価器に渡す場合は、スカラー／マスクの対象で `scalar = true` を指定する。`evaluate` の `Target::Mask` は A を隠す量として読み、出力は RGB=0・A=隠す量。色の完全透明画素は RGB も変更しない。
接空間法線は対象に含めない。`Settings::algorithm_version` はランプなし1、ランプ付き2。

`Settings::anchor`（`anchor::Reference`）が C# の `GeneratorSettings` の AnchorId・AnchorChannel・AnchorRead（正本の版 20）を持つ。
`id` は `Point::id` で 0 が未選択、`channel` は読むチャンネル、`read` は `ReadMode::Value`（値×被覆）か `Coverage`（被覆）。
Anchor の既定は Height、他の種類は Color（C# と同じ）。`Settings::validate` は C# と同じく、Anchor 以外の種類が既定値以外を持つこと、
Anchor が Normal チャンネルを読むことを断る（種類が Normal のユーザーチャンネルは `Plan::new` が断る）。マスクの Anchor は `channel` と `read` を無視する。
読む順は `Reference::resolve` → `Point`（層番号と配置）→ 層なら `Plan::new(layers, point.host, dimensions, チャンネルの種類)` と
`Reference::read_for(チャンネルの種類)` で `LayerSample`、マスクなら `MaskSample` → `BoundGenerator::bind` の `anchor`。

`anchor::resolve` は未選択・欠損・同じ層／上の層を拒否する。すべての参照が小さい層番号を向くため、循環を作れない。
点の一覧を受け取る時点で `validate_points` を呼び、ID と配置の重複を断る。層の並びが変わった場合は再解決する。
`anchor::Plan` は1チャンネルの評価済み層画像を受け、その層までのスタックを合成する。グループ・マスク・クリッピング・調整層に対応する。
通過グループでは下から続け、分離グループでは透明から始める。Anchor より上のクリッピングと、祖先自身の不透明度・マスク・表示は含めない。
層のフィルターは入力画像に評価しておく。グループ自身のフィルターや投影は呼び出し側で画像に評価して渡す。
層の結果を `LayerSample` に渡すと値×被覆・色の輝度×被覆・被覆を読める。`MaskSample` はフィルター評価済みの隠す量に有効状態・反転・濃度を適用する。
文書の編集、保存、履歴、キャッシュ、Anchor の連鎖のスナップショット作成順は呼び出し側が管理する。

作業予算は返却する RGBA8 の `幅×高さ×4` バイト。別の画素バッファは作らない。外部の入力、設定・合成計画などのメタデータ、スレッドスタックは含めない。
C# FilterEngine はブロック用の作業バッファを数えるため、同じ数値の予算での拒否境界は一致しない。
取消は行の境界と返却直前に検査し、失敗時は途中画像を返さない（`evaluate` と `Plan::evaluate` のどちらも、並列度 1 で読んだ画素の数を数えて行ごとの確認と最後の確認を別々に試験している）。C# FilterEngine には取消トークンがなく、Rust の追加契約である。

`bash tools/csharp-golden/run-generator.sh` は Unity に同梱の Roslyn/Mono で正本の Core をそのまま組み、合成した入力を実評価する。
出力は `target/csharp-generator/golden/`。試験は全画素の SHA-256 を `tests/generator-index.txt` と比較する。
`GEN_THREADS=1` と `4` で並列度を指定できる。`run-generator.sh bench` と
`cargo run --release -p yolu-core --example generator_bench` は4096²、ウォームアップ1回・計測1回。入力画像と設定の作成は時間に含めず、束縛と返却画像の確保は含める。

`YOLU_GENERATOR_GOLDEN` に生成先を指定して試験を実行すると、338事例を C# 出力の各バイトとも直接比較する。

## ノイズ・グランジ（Rust 版だけの種類）

`Kind::Noise`（64）と `Kind::Grunge`（65）は C# に対応が無く、マップを読まずに位置・向き・UV から値を作る（設定は `Settings::procedural`）。C# の種類（0〜7）と重ならない 64 から振り、
`.ylp` の保存は正本の版 23（`crates/yolu-io/README.md` の「手続き型の Generator」）。レベル（low・high・softness・invert）がしきい値・コントラストで、blend・強さ・マスクの対象は他の種類と同じ。

- ノイズ: 基底は値・Perlin（勾配）・Worley（セル。F1・F2・F2−F1）、重ね方は fBm・ridged・turbulence、オクターブ 1〜8・ラクナリティ 1〜4・ゲイン 0〜1・大きさ・シード・回転・にじみ（座標のゆがみ）。
- グランジ: プリセットは汚れの斑・錆の斑・傷の筋・ほこり・指紋・布目・ひび・飛沫・塗装の剥げ・木目・革のしぼ（`GrungePreset`）。ノイズの層（`Layer`）としきい値・三角波の組み合わせで、値は 1 が「ある」。
  `GrungePreset::default_scale` が選んだときの模様の大きさ、`Settings::grunge(preset)` が既定の設定。`preview(&settings, w, h)` は UV 空間の見本（灰色の RGBA8。画面のサムネイル用）。
- 空間: 位置（Position のマップで 3D。UV の島の継ぎ目で模様がずれない）・トライプラナー（Position と WorldNormal。塗りつぶしの投影と同じ重み）・UV（x・y の格子を周期で巻き、端で継ぎ目が出ない）。
  2D の模様のプリセット（傷の筋・指紋・布目）は、位置の空間では自動でトライプラナー。位置のマップが使えないときは入力のまま通さず UV に落とし、`BoundGenerator::fallback` が理由を返す
  （`Document::generator_fallback`・`fallback_effect_list`。`inactive` とは別）。`used_maps` は設定だけで決まる（使えるかは見ない）。
- 決定性: 式は + − × ÷ sqrt floor と整数だけ（libm を使わない。回転は多項式の sin・cos）。画素ごとに位置・座標だけから決まるので、スレッド数・領域の切り方・評価ブロックの大きさで結果は変わらない。
  取消は他の種類と同じく行の境界、予算は返す RGBA8 の大きさ。実装を固定するハッシュは `tests/procedural-index.txt`（回帰の固定で、外部の正解ではない）。
