# タイル合成とブラシの GPU プレビュー

`GpuPainter` は wgpu 30 の compute で、`yolu-core` の平坦なレイヤー文書をタイルごとに合成します。26 合成モード、層の不透明度、表示、チャンネル、クリッピングを扱います。RGB の色空間を変換せず、straight RGBA8 を層ごとに半段切り上げで丸めます。元の CPU タイルは読み取るだけです。保存・Undo の正本には常に `yolu-core` を使ってください。

```rust
use yolu_core::{Channel, Document, TileCoord};
use yolu_gpu::{Compositor, Options};
let doc = Document::new(128, 128).unwrap();
let mut compositor = Compositor::new(Options::default());
let result = compositor.composite_tiles(&doc, Channel::Color, &[TileCoord::new(0, 0)]).unwrap();
assert!(result.is_current(&doc));
// GPU が使えなければ CPU の結果と fallback_reason() の理由が得られる。
```

`Document::changed_tiles(channel, serial)` の一覧を `composite_tiles` に渡すと、そのタイルだけを合成します。空になったタイルも透明な結果を返します。端の部分タイルは `TileResult::rect` の幅と高さに切り詰めます。行は左下から上へ並びます。キャッシュは呼び手が所有し、文書・チャンネルの切り替えでは破棄してください。同じ ID を持つ文書を読み直したときも破棄します。`is_current` は同一の文書を編集し続けている間の ID・revision 照合です。

`brush_dabs` はストローク開始前の矩形と補間済みの `Dab` 一覧を受け取り、丸い筆先の smoothstep、硬さ、筆圧、流量、不透明度の天井、消しゴムを計算します。座標は渡した矩形の左下からの画素座標です。同じストロークのダブをまとめて渡してください。複数呼び出しをまたぐ覆いの蓄積や入力点の補間は行いません。取消では結果を捨て、正本の描画・Undo は core のストロークで行います。透明画素の RGB は入力コピーと無描画で保持し、消しゴムでアルファが 0 になったときは core と同じく RGBA を 0 にします。

## 常駐キャッシュと表示テクスチャ

`ResidentCompositor` は入力の層タイルを GPU に常駐させ、合成結果を表示用の `Rgba8Unorm` テクスチャに保持します。`update` は `Document::changed_tiles` の世代を追い、影響を受ける座標だけを再合成します。その座標の各層を CPU の照合用コピーと完全比較して、実際に変わった層のタイルだけを転送します。不透明度・合成モードなど画素を変えない変更では、入力の再転送が不要です。ハッシュ衝突に依存しない代わりに、入力と同量の CPU コピーを持ちます。

```rust
use yolu_core::{Channel, Document};
use yolu_gpu::{ResidentCompositor, ResidentOptions};
let doc = Document::new(256, 256).unwrap();
let mut gpu = ResidentCompositor::new(ResidentOptions::default())?;
let stats = gpu.update(&doc, Channel::Color)?;
let display = gpu.display()?;
// display.texture / display.view を表示に使う。行は左下から。保存は core の正本から行う。
assert_eq!(display.revision, doc.revision());
// 必要なときだけ任意の矩形を読み戻す。
let request = gpu.request_readback(doc.bounds())?;
let pixels = gpu.finish_readback(request)?;
# Ok::<(), yolu_gpu::GpuError>(())
```

表示テクスチャは借用で返します。次の更新で内容が変わるため、古い世代のスナップショットとして保持しないでください。文書 ID・寸法・チャンネルの変更で常駐コピーを破棄します。同じ ID の文書を再読み込みするときは `reset()` を明示的に呼んでください。層の追加・削除・順序変更、取消・Undo・Redo も変更記録から反映します。`GpuPainter` の既存の読み戻し API とブラシは引き続き利用できます。app への組み込みは含みません。

### 常駐予算と読み戻しの寿命

既定の常駐予算は 768 MiB。表示テクスチャ、入力の GPU コピーと同量の CPU コピー、作業バッファ、束ごとの転送用余裕を数えます。上限に達すると、最後に使用した時刻が古い層タイルから追い出します。表示と最小 1 束すら入らない予算は、理由付きで拒否します。`UpdateStats` で更新数・転送数/バイト・ヒット数・追い出し数・保持予約量を確認できます。CPU のコンテナ管理領域、シェーダー、ドライバーの内部プール、呼び手が持つ CPU の読み戻し結果は対象外です。

更新は最大 16 タイル（設定可能、1〜64）ずつ処理します。常駐入力から作業域への GPU 内コピーを行い、入力とは別の表示テクスチャに書き込みます。画素の読み戻しはありませんが、転送用メモリの寿命を区切るため束ごとに GPU の処理完了を待ちます。フレームをまたいで表示更新を非同期に重ねる実装ではありません。

読み戻しには別の既定 64 MiB の予算があります。256 バイトに揃えた行幅で、未完了または未取得の要求のバッファを合計します。`request_readback` はコピーとマップを開始し、`finish_readback` は完了を待って密な RGBA8 を返します。要求はバッファとデバイスを所有し、表示インスタンスを破棄しても `Readback::wait_ready` で完了を処理できます。要求を途中で捨てても、GPU のコピー完了までは予算の予約を保持します。更新のないフレームでも `poll` を呼んで完了通知を処理できます。

表示の世代が変わった要求、別の表示インスタンスの要求は採用しません。`reset` は旧テクスチャを参照するコピーの完了を待ってから資源を手放し、旧資源の分を次世代の予算として二重に使うことを防ぎます。GPU の確保・実行失敗では古い表示を無効にし、理由を返します。呼び手は既存の CPU 合成経路に戻してください。これらは Unity 版の派生表示/CPU 正本、完了前の再利用禁止、所有するコピー量の予算という契約に合わせています。

### 常駐後の計測

Windows でも同じ example を使用できます。出力されるアダプター名・バックエンドと併せて記録してください。

```
cargo run --release -p yolu-gpu --example resident_measure -- 4096 3
```

引数は画布の一辺（128 の倍数）と測定回数です。4 層と 16 層を測り、初回常駐と各ケース 1 回のウォームアップを除外します。局所変更は 1 層の 1 タイル、全面変更は全層の全タイルを実際に変更します。GPU 時間は入力の照合、必要な転送、合成と完了待ちを含みます。読み戻し有りでは更新範囲（局所 128²、全面 4096²）を取得します。CPU は同範囲の合成だけで、表示への転送は含めません。入力を変更する処理と画素の一致確認は計測外です。

以下は release、4096²、3 回の中央値（ms）。llvmpipe/lavapipe、LLVM 15.0.7、Mesa 23.2.1、Vulkan の **ソフトウェア GPU** の数値です。このコンテナの実 GPU 経路は GLX のみで EGL がなく、wgpu から利用できません。Windows・実 GPU の結果ではありません。

| 層数 | 変更 | 読み戻し | CPU | GPU |
|---|---|---|---:|---:|
| 4 | 1 タイル | 無 | 1.207 | 1.252 |
| 4 | 1 タイル | 有 | 1.135 | 1.444 |
| 4 | 全層・全面 | 無 | 32.736 | 405.527 |
| 4 | 全層・全面 | 有 | 32.639 | 480.850 |
| 16 | 1 タイル | 無 | 1.206 | 2.378 |
| 16 | 1 タイル | 有 | 1.088 | 2.364 |
| 16 | 全層・全面 | 無 | 147.456 | 1421.094 |
| 16 | 全層・全面 | 有 | 143.332 | 1590.049 |

転送量は局所 64 KiB、4 層全面 256 MiB、16 層全面 1 GiB。常駐予算 3 GiB で追い出しは 0、保持予約量はそれぞれ 612,368,800 / 2,248,147,744 バイト（CPU 照合コピーを含む）。計測画素の最大差は 1、16 層の局所は 0 でした。別々の試行の中央値なので、読み戻し有無の差をそのまま転送時間とは解釈できません。今回の環境で CPU より速いとは言えません。

常駐経路の試験では、変わった層だけの転送、画素転送なしのプロパティ変更、LRU、予算拒否、部分タイル、クリッピング、文書/チャンネル切り替え、読み戻し取消・世代・所有元破棄を検証します。追加の通常/クリッピングの画像では 26 モードすべて最大差 0。既存のより広い入力の試験は下記の許容差を維持しています。

## GpuPainter の同期 API の予算と失敗

読み取り入力と出力は別バッファです。1 回の読み戻しを完了してから次の束を投入します。既定 64 MiB の GPU 作業予算には入力、メタデータ、出力、読み戻し、およびそれらと同量の転送用余裕を計上します。ドライバーの内部領域、シェーダー、呼び手が保持する CPU の結果はこの予算に含めません。1 束は最大 64 タイルで、デバイスのバッファ・ディスパッチ上限も適用します。ブラシは 1 回百万ダブ以内かつ予算以内です。

読み戻しは最大 30 秒待ちます。失敗したインスタンスは次の投入を拒否し、再作成が必要です。`Compositor` は GPU の初期化または実行失敗を理由付きで CPU に切り替えます。`GpuPainter` の直接利用やブラシは `Result` のエラーを受けて core へ戻してください。Normal チャンネルは現行 core と同じく未対応として拒否します。

## 検証と計測

```
export XDG_RUNTIME_DIR=/tmp/xdg-$USER
mkdir -p "$XDG_RUNTIME_DIR"
cargo test -p yolu-gpu -- --nocapture
cargo run -p yolu-gpu --example measure
```

アダプターまたはデバイスを取得できない場合、GPU 試験は理由を標準エラーへ出して終了します（Rust の集計では passed と表示されるため `--nocapture` の出力を確認してください）。シェーダーの不具合はスキップせず失敗します。`WGPU_BACKEND` など wgpu の環境変数でバックエンドを選択できます。

許容差は Unity 版 `Tests/Editor/Gpu/GpuTests.cs` の `MaxByteError = 1` と同じです。通常とクリッピングは、各モード 67×35 画素、アルファ 0・1・90・120・254・255、不透明度 0・0.37・0.85・1 で CPU と照合します。非線形モードの多段合成では前段の丸め差が増幅するため、任意の層数について最大差 1 を保証しません。ブラシは Unity の古い線形覆いの試験シェーダーではなく、現行 core の smoothstep と蓄積式を正とします。

コンテナの llvmpipe（LLVM 15.0.7、Mesa 23.2.1、Vulkan）で、SoftLight は最大差 0、他の 25 モードは最大差 1。ブラシの塗り/消しゴム×硬さ 3 種×筆圧連動 2 種の 12 条件は最大差 0 でした。不透明・不透明度 1 の Normal、Darken、Lighten、LinearDodge、LinearBurn、LinearLight、PinLight、HardMix、Difference、Subtract、DarkerColor、LighterColor は、各 65536 画素で全バイト一致を要求しています。

従来の同期 API 用の `measure` は 4096²・4 層（Normal/Multiply/Screen/Normal）でウォームアップ後 3 回測り、転送と読み戻し込みで CPU と比較します。開発プロファイルでの中央値は全面 1024 タイルが CPU 40.09 ms／GPU 394.50 ms、変更 1 タイルが CPU 1.10 ms／GPU 1.37 ms、計測画像の最大差は 0 でした。これはソフトウェア Vulkan での計測で、実 GPU の高速化や Windows の D3D12/Vulkan を確認したものではありません。

依存する wgpu、pollster、bytemuck は MIT または Apache-2.0 を選択できるライセンスです。
