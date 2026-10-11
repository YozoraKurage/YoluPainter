# Generator・ランプ・Anchor の画像評価

`Settings` を `BoundGenerator::bind` で検査し、読み取り専用の入力に束縛する。`evaluate` は `Source` の指定領域から新しい RGBA8 画像を返す。
`Image` は連続した画像用。タイル入力は `Source` を実装してキャンバス座標で画素を返す。領域の切り方・Rayon のプールの並列度で計算結果は変わらない。
入力と設定は借用するため、評価中のスナップショットは変更できない。成功した画像だけを呼び出し側で採用する。

対応する種類は EdgeWear、Dirt、PositionGradient、Thickness、Direction（ワールド／ベント法線）、ShapeGradient、IdColor、Anchor に、Rust 版だけの Noise（64）・Grunge（65）・Image（70）と、下の 0.5.0 の種類（66〜69）。
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
`sample(slot, x, y)` から束縛済みの `BoundGenerator::sample(x, y, scalar)` の結果をそのまま返せる（`filter::Generated` はこの型の再公開）。
行の口 `sample_row(slot, x0, y, out)` は `BoundGenerator::sample_row` へそのまま渡せ、フィルターの段は行ごとにこちらを呼ぶ。
フィルターなど別の評価器に渡す場合は、スカラー／マスクの対象で `scalar = true` を指定する。`evaluate` の `Target::Mask` は A を隠す量として読み、出力は RGB=0・A=隠す量。色の完全透明画素は RGB も変更しない。
接空間法線は対象に含めない。`Settings::algorithm_version` はランプなし1、ランプ付き2。

`Settings::anchor`（`anchor::Reference`）が C# の `GeneratorSettings` の AnchorId・AnchorChannel・AnchorRead（正本の版 20）を持つ。
`id` は `Point::id` で 0 が未選択、`channel` は読むチャンネル、`read` は `ReadMode::Value`（値×被覆）か `Coverage`（被覆）。
Anchor の既定は Height、他の種類は Color（C# と同じ）。`Settings::validate` は C# と同じく、Anchor 以外の種類が既定値以外を持つこと、
Anchor が Normal チャンネルを読むことを断る（種類が Normal のユーザーチャンネルは `Plan::new` が断る）。マスクの Anchor は `channel` と `read` を無視する。
読む順は `Reference::resolve` → `Point`（レイヤー番号と配置）→ レイヤーなら `Plan::new(layers, point.host, dimensions, チャンネルの種類)` と
`Reference::read_for(チャンネルの種類)` で `LayerSample`、マスクなら `MaskSample` → `BoundGenerator::bind` の `anchor`。

`anchor::resolve` は未選択・欠損・同じレイヤー／上のレイヤーを拒否する。すべての参照が小さいレイヤー番号を向くため、循環を作れない。
点の一覧を受け取る時点で `validate_points` を呼び、ID と配置の重複を断る。レイヤーの並びが変わった場合は再解決する。
`anchor::Plan` は1チャンネルの評価済みレイヤー画像を受け、そのレイヤーまでのスタックを合成する。グループ・マスク・クリッピング・調整レイヤーに対応する。
通過グループでは下から続け、分離グループでは透明から始める。Anchor より上のクリッピングと、祖先自身の不透明度・マスク・表示は含めない。
レイヤーのフィルターは入力画像に評価しておく。グループ自身のフィルターや投影は呼び出し側で画像に評価して渡す。
レイヤーの結果を `LayerSample` に渡すと値×被覆・色の輝度×被覆・被覆を読める。`MaskSample` はフィルター評価済みの隠す量に有効状態・反転・濃度を適用する。
文書の編集、保存、履歴、キャッシュ、Anchor の連鎖のスナップショット作成順は呼び出し側が管理する。

作業予算は返却する RGBA8 の `幅×高さ×4` バイト。別の画素バッファは作らない（行の作業領域 = 幅 × 8 バイトの値の行と格子の覚えをスレッドごとに持つが、画素のバッファではないので予算に含めない）。外部の入力、設定・合成計画などのメタデータ、スレッドスタックも含めない。
C# FilterEngine はブロック用の作業バッファを数えるため、同じ数値の予算での拒否境界は一致しない。
取消は行の境界と返却直前に検査し、失敗時は途中画像を返さない（`evaluate` と `Plan::evaluate` のどちらも、並列度 1 で読んだ画素の数を数えて行ごとの確認と最後の確認を別々に試験している）。C# FilterEngine には取消トークンがなく、Rust の追加契約である。

## 行ごとの評価と SIMD

`evaluate` は行を単位に評価する（`evaluate/rows.rs`）。入力の行を `Source::read_row` で読み、種類ごとの基底の値の行 → レベル・減衰・反転 → 重ねるノイズ → ランプ・ブレンドの合成の順に、
画素ごとに変わらないもの（種類・ブレンド・ランプの有無・形の種類）をループの外で決めて処理する。値を持たない画素は `Option::None` と同じ意味の印で運び、`BoundGenerator::sample_row` が
1 画素ずつの `sample` と同じ結果を行でまとめて返す（`Generated` は f64 までビットで一致）。`ValueSource::value_row`・`Source::read_row` の既定は 1 画素ずつの呼びで、行をまとめて読める実装は置き換えられる。

ノイズ・グランジの乱数は、レイヤー・オクターブ・セルの枠ごとに束縛のときに 1 回だけ出す（`procedural::Plan`）。格子の角の値（Value・Perlin の勾配の向き・Worley の特徴点・線分）は hash だけで決まるので、
直前の格子を覚えて、同じ格子に入る隣の画素では引き直さない（違う格子に入れば引き直す）。覚えはスレッドごとの作業領域が持ち、画素を 1 つずつ呼ぶ経路（`sample`。フィルターの粗い評価）でも計画ごとに使い回す。

SIMD の道は `math::simd` の土台（x86_64 は実行時に AVX2・SSE4.1・スカラー、aarch64 は NEON・スカラー。`YOLU_SIMD` で下げられる）。重ねるノイズ・レベル・ノイズ・グランジ 11 種・にじみ・トライプラナー・ランプの色・全ブレンドの合成を、N 画素（AVX2 は 4、
SSE4.1・NEON は 2）を 1 組にして計算する。N 画素が同じ格子に入る組は角の値を全部のレーンで共有し、格子をまたぐ組・被覆の無い画素を含む組・向きが定まらない画素を含む組は 1 画素ずつの式で計算する。
レーンの式は 1 画素の式と同じ IEEE の演算を同じ順に並べたもの（FMA の積和も使わない）なので、**結果のバイトは全部の道で同じ**。ランプの混色（通常・リニア・知覚的）もレーンで計算する（リニア・知覚的の重い式は、道ごとの `inline(never)` の入口 `MixLanes::mix_curved` に 1 つずつ置き、ramp・合成の入口へは畳み込まない）。
リニア・知覚的の式は + − × ÷ と sqrt だけで書いてある（`mixing.rs`: 分岐点の線形の光は 256 値の表、立方根は [1/8, 1] へ寄せてからのニュートン法 4 回、
`c^(1/2.4)` は `r·√√r`。OS の `powf`・`cbrt`・`hypot` を使わない）ので、レーンと 1 画素の式が同じ演算になり、OS の数学の関数の違いにも左右されない。
分岐点ごとの線形の光・Oklab・彩度は `Ramp` が作るときに 1 度だけ求める。前の式（OS の数学の関数）との差は、色の組 3,081 × 重み 513 × 混色 2 × 補正 3 の
約 2,845 万の値で 0（glibc。`mixing.rs` の試験）。混合率曲線のある区間は、N 画素が同じ区間ならその曲線のレーンの式、またがる組は 1 画素ずつの曲線の式で重みを出す。

`cargo run --release -p yolu-core --example generator_perf` は種類 × 大きさ（256²・1024²・4096²）× スレッド数（1・8）の表（環境変数 `GEN_SIZES`・`GEN_THREADS`・`GEN_ROWS`・`GEN_RUNS` で絞る）で、
出力は全画素の SHA-256 で毎回同じ。`GEN_BREAKDOWN=1` は 4096²・スレッド 1 の段ごとの内訳（入力の取り込み・値・ノイズ・行・全体）。`GEN_FUZZ=件数` は計測の代わりに、固定のシードの乱数で作った設定・マップ・入力・領域の組を評価して出力のハッシュを 1 件ずつ出す。前後のコミットでビルドした例の出力（`YOLU_SIMD` を変えても）を `diff` すれば、設定をばらまいた入力でバイトが変わっていないことを確かめられる。
入力は C# との一致試験と同じ合成入力（`rand`。マップの値が画素ごとに飛ぶので格子の覚えが
当たらない）と、メッシュのマップに近いなめらかな入力（`smooth`）の 2 通り。

`bash tools/csharp-golden/run-generator.sh` は Unity に同梱の Roslyn/Mono で正本の Core（Unity ブリッジのタグ `0.4.0` の `Runtime/Core`。場所は環境変数 `YOLUPAINTER_UNITY_SOURCE`）をそのままビルドし、合成した入力を実評価する。
出力は `target/csharp-generator/golden/`。試験は全画素の SHA-256 を `tests/generator-index.txt` と比較する。
`GEN_THREADS=1` と `4` で並列度を指定できる。`run-generator.sh bench` と
`cargo run --release -p yolu-core --example generator_bench` は4096²、ウォームアップ1回・計測1回。入力画像と設定の作成は時間に含めず、束縛と返却画像の確保は含める。

`YOLU_GENERATOR_GOLDEN` に生成先を指定して試験を実行すると、338事例を C# 出力の各バイトとも直接比較する。

## ノイズ・グランジ（Rust 版だけの種類）

`Kind::Noise`（64）と `Kind::Grunge`（65）は C# に対応が無く、マップを読まずに位置・向き・UV から値を作る（設定は `Settings::procedural`）。C# の種類（0〜7）と重ならない 64 から振り、
`.ylp` の保存は正本の版 23（`docs/YLP_FORMAT.md` の「Generator」）。レベル（low・high・softness・invert）がしきい値・コントラストで、blend・強さ・マスクの対象は他の種類と同じ。

- ノイズ: 基底は値・Perlin（勾配）・Worley（セル。F1・F2・F2−F1）、重ね方は fBm・ridged・turbulence、オクターブ 1〜8・ラクナリティ 1〜4・ゲイン 0〜1・大きさ・シード・回転・にじみ（座標のゆがみ）。
- グランジ: プリセットは汚れの斑・錆の斑・傷の筋・ほこり・指紋・布目・ひび・飛沫・塗装の剥げ・木目・革のしぼ（`GrungePreset`）。ノイズのレイヤー（`Layer`）としきい値・三角波の組み合わせで、値は 1 が「ある」。
  `GrungePreset::default_scale` が選んだときの模様の大きさ、`Settings::grunge(preset)` が既定の設定。`preview(&settings, w, h)` は UV 空間の見本（灰色の RGBA8。画面のサムネイル用）。
- 空間: 位置（Position のマップで 3D。UV アイランドの継ぎ目で模様がずれない）・トライプラナー（Position と WorldNormal。塗りつぶしの投影と同じ重み）・UV（x・y の格子を周期で巻き、端で継ぎ目が出ない）。
  2D の模様のプリセット（傷の筋・指紋・布目）は、位置の空間では自動でトライプラナー。位置のマップが使えないときは入力のまま通さず UV に落とし、`BoundGenerator::fallback` が理由を返す
  （`Document::generator_fallback`・`fallback_effect_list`。`inactive` とは別）。`used_maps` は設定だけで決まる（使えるかは見ない）。
- 決定性: 式は + − × ÷ sqrt floor と整数だけ（libm を使わない。回転は多項式の sin・cos）。画素ごとに位置・座標だけから決まるので、スレッド数・領域の切り方・評価ブロックの大きさで結果は変わらない。
  取消は他の種類と同じく行の境界、予算は返す RGBA8 の大きさ。実装を固定するハッシュは `tests/procedural-index.txt`（回帰の固定で、外部の正解ではない）。

## 画像（Rust 版だけの種類）

`Kind::Image`（70）はアセットの画像を、塗りつぶしレイヤーの画像と同じ投影（UV・トライプラナー・平面・球・円柱。デカールは除く）で読む（設定は `Settings::image`:
画像の ID・`fill_image::Projection`・成分）。`.ylp` の保存は正本の版 28。

- 画像そのものは文書の外の入力（`EffectInputs` の画像）。`BoundGenerator::bind` の後に `with_image(&FillSampler)` で、投影・文書の大きさ・位置と向きのマップ・
  モデルのルートを束縛したサンプラーを渡す（文書の評価は `document/eval.rs` が色の対象には色として、マスク・スカラーには値のままのミップマップを作って渡す）。
  渡すまで・画像を選んでいない・投影が読むマップが使えないときは `inactive`（`NoImage`・`MissingImage`・`MissingMap` ほか）で入力のまま通す。
- 値: 色のチャンネルでは画素の色（`invert` は RGB を反転、アルファは見える度合いに掛ける）を `blend` で合わせ、マスク・スカラーでは選んだ成分（R・G・B・A・輝度）に
  レベル（low・high・softness・invert）を通す。画像の値の無い画素（外側が透明の画像の外・位置のマップに面の無い画素）は入力のまま。
- 読み方は `FillSampler::projected`（塗りつぶしレイヤーと同じミップマップと投影の式）なので、色のチャンネルでは同じ画像・同じ投影の塗りつぶしレイヤーと同じ画素になる
  （`tests/effects/image_stage.rs`）。マスク・スカラーの輝度は、補間した後の RGB から丸めずに求める。塗りつぶしレイヤーがスカラーのチャンネルで読む輝度は元の画素ごとに
  8 bit へ丸めてから補間するので、補間がかかる投影では値が少し違う（R・G・B・A の成分は塗りつぶしレイヤーに読み方が無い）。
  行の評価（`sample_row`）も画素ごとに `projected` を呼ぶ（レーンの式にはしていない）。

## 模様・ライト・マスクの組み立て（0.5.0。Rust 版だけの種類）

`Kind::Pattern`（66）・`Kind::UvIslandVariation`（67）・`Kind::Light`（68）・`Kind::MaskBuilder`（69）。設定は `Settings::pattern`・`island`・`light`・`mask_builder`
（その種類以外は既定のまま）。重ねるノイズは持たず、模様・ライトは共通の `softness` を 0 にして自分の `softness` を使う（マスクの組み立ては共通の減衰を使う）。
式（`kinds050.rs`）は + − × ÷ sqrt floor と多項式の sin・cos だけで、1 画素ずつ（行の評価も同じ式を呼ぶ。SIMD にしない）。

- 模様: UV（画素の中心）を回転して scale 回繰り返し、縞・市松・水玉・格子は繰り返しの中の、縁は UV の正方形の縁からの距離で内外を決め、softness の幅で smoothstep。マップを読まない。
- ライト: ワールドの法線 n と光の向き d（azimuth は +Z から +X へ、elevation は水平から +Y へ）で lit = clamp((n·d + softness) / (1 + softness))、値は ambient + (1 − ambient) × lit。
  向きの定まらない法線の画素は値を持たない。
- マスクの組み立て: 曲率・AO・位置の高さ（Y）・厚みを、ヒストグラムスキャンと同じ式（level・contrast）で 0〜1 にし、invert、重みを付けて掛ける（1 − w + w·t）・最大（w·t）・足す（1 まで）。
  重みが 0 のマップは読まない（無くても断らない）。読むマップの被覆の無い画素は値を持たない。
- アイランドごとのばらつき: テクセルの UV アイランドの番号 i（`geometry::IslandMap`。モデルが同じなら解像度によらず同じ番号）とシードから、
  u = `cell_hash(seeds(seed)[0], i, 0, 0)` × 2^-32（[0, 1)。u32 → f64 と 2 の冪の掛け算に丸めは無い）、値は min + (max − min) × u（min ≤ max。等しければアイランドによらず同じ値）。
  共通のレベル・減衰・反転を通す。アイランドの図は束縛の後に `BoundGenerator::with_islands` で渡し、渡すまでは `Inactive::NoModel`、作業予算で作れなければ
  `islands_refused` で `Inactive::IslandMap`（入力のまま通す）。アイランドの外（0）の画素は値を持たない。重なったテクセルは番号の小さい三角形のアイランド（アイランドの図の決まり）。
  行の評価はアイランドの図の行の連なりを歩き、連なりごとに 1 回だけ値を求める。
