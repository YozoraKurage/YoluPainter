# 開発用の手順

以下はリポジトリのルートで実行します。通常のビルドと操作は [README](../README.md) を参照してください。

## 試験

```sh
cargo test --workspace --locked
```

コマンドラインと MCP サーバーは `cargo test -p yolu-cli`（`cargo clippy -p yolu-cli --all-targets -- -D warnings`）で、作りと試験の分け方は [crates/yolu-cli/README.md](../crates/yolu-cli/README.md) にあります。

GPU・画面の試験には動作する描画バックエンドが必要です。GPU 試験にはアダプターがないと処理を省くものがあるため、結果の passed だけで描画確認済みとは判断せず、標準エラーの理由も確認してください。環境変数 `YOLUPAINTER_REQUIRE_GPU=1` を付けると、省かずに落とします（CI の画面の試験は付けています）。詳しくは [yolu-gpu](../crates/yolu-gpu/README.md#検証と計測) を参照してください。

層の合成の式は Rust の f32 の式が正本で、合成を通る正解は Rust で撮り直しています。正解を撮り直すときは `YOLU_GOLDEN_UPDATE=1 cargo test -p yolu-core -p yolu-io` で、違った正解だけを今の出力で書き直し、差分を見て意図した変化だけかを確かめます。ブラシの画素・Normal のチャンネル・フィルター・Generator の値など f64 のままの式は、今も Unity 版 C# の正解と照らしています。

Unity 版 C# との照合には、リポジトリに収録された人工データを使います。core の正解の再生成ツールは `tools/csharp-golden/run.sh` です（出力先は `--out` で指定できます）。編集できるパスの正解は `tools/csharp-golden/run-paths.sh <出力先>` で作り、試験が読む `crates/yolu-core/tests/golden/paths` へ出力します。効果と層のロック・層の操作のつなぎ目の正解（事例ごとの SHA-256）は `tools/csharp-golden/run-seam.sh` で `crates/yolu-core/tests/golden/seam.txt` へ作ります。食い違ったときは、試験を `SEAM_DUMP_DIR=<フォルダ>` で回して Rust の生のバイト列を書き出し、`run-seam.sh dump <事例名> <出力>` の C# の側と `cmp` で比べます。どれも Unity 版のソースと Unity 同梱の .NET・Mono が必要です。PSD の写し（core ⇔ PSD）の正解は `tools/csharp-golden/run.sh psd` で作ります。I/O と PSD のデータ形式・再生成方法は [I/O のフィクスチャ](../crates/yolu-io/tests/fixtures/README.md)と [PSD のフィクスチャ](../crates/yolu-io/tests/fixtures/psd/README.md)を参照してください。ブラシ形式の取り込みの正解は `tools/csharp-golden/brushes.sh` で作り（Rust の試験が入力を書き、C# の読み手に通して `crates/yolu-io/tests/fixtures/brushes/` へ出力）、違いの調査は `BRUSH_GOLDEN_SHOW=<記録の番号>` でその入力の完全な指紋を出します。

### yolu-app の結合試験の置き方

`crates/yolu-app/tests/` の試験は、性質ごとに数本の実行ファイル（「束」）にまとめています。1 ファイルを 1 本の実行ファイルにすると、試験の数だけアプリ全体のリンクと共通部品のビルドし直しが増え、`target/` が試験の実行ファイルだけで 10 GB を超えるためです。

| 束（`cargo test -p yolu-app --test <束>`） | 中身 |
| --- | --- |
| `headless` | 窓・GPU の装置を作らない試験（文書・保存（裏の保存・選択範囲・ポーズを含む）・取り込み・Live Link の通信と頼み・ソースの文言の検査）。同時に走る |
| `gui_canvas` | キャンバス・ツール・ブラシ・選択・色・効果の画面（`egui_kittest`） |
| `gui_shell` | 窓の全体・メニュー・設定・文書の出し入れ（PSD のドロップを含む）・閉じる流れと保存の途中の終了・GPU の装置の喪失・復旧・更新・Live Link の画面と受け取りの上限・言語・棚・ライブラリ |
| `gui_view3d` | 3D ビュー（アンチエイリアス・ブルームを含む）・マテリアルの見た目（lilToon を含む）・ポーズ・テクスチャセット・出力 |
| `threads`・`window_lease`・`windowpos`（直下の 1 ファイル 1 本） | プロセス全体の状態を持つ試験: rayon の全体のプール・窓の貸し出しの数え・覚えた窓の置き場所 |

束の中のファイルは `tests/<束>/<名前>.rs`、束の入口は `tests/<束>/main.rs` の `mod` の並びです。試験の名前は `<ファイル名>::<試験名>` になるので、ファイルや試験名で絞れます。

```sh
cargo test -p yolu-app --test gui_shell                      # 1 つの束
cargo test -p yolu-app --test gui_shell i18n::               # 束の中の 1 ファイル（以前の `--test i18n`）
cargo test -p yolu-app --test headless no_instruction_text:: # 以前の `--test no_instruction_text`
cargo test -p yolu-app --test gui_canvas layerops::undo      # ファイル名と試験名の一部
cargo test -p yolu-app -- --list | grep layerops             # どの束にあるか（`Running tests/<束>/main.rs` の下）
```

- **新しい試験を足す**: 画面を作らないなら `headless/`、作る（`common::app`・`common::gpu_thread::builder`）なら内容に近い `gui_*/` にファイルを置き、その束の `main.rs` に `mod 名前;` を 1 行足します。足し忘れは `headless/bundle_layout.rs` が落ちて知らせます（置いただけではビルドされず、走らないのに通るため）。ファイルの先頭で `use crate::common;` と書くと、共通部品（`tests/common/`）を `common::…` で使えます。
- 直下（`tests/<名前>.rs`）に置いた 1 ファイル 1 本の試験を束へ移すには、`git mv tests/<名前>.rs tests/<束>/<名前>.rs`、ファイル先頭の `mod common;` を `use crate::common;` に替え、束の `main.rs` に `mod <名前>;` を足し、コメントや文書の `--test <名前>` を `--test <束> <名前>::` に直します（試験の名前は `<名前>::<元の名前>` になります）。
- **直下に 1 ファイル 1 本で置く**のは、プロセス全体の状態（rayon の全体のプール・環境変数・窓の貸し出しの数え・覚えた窓の置き場所）を変える・数える試験だけです。束の中の試験どうしは同じプロセスで走るので、そのような試験を混ぜると順序で結果が変わります。
- **窓（harness）は必ず `common::gpu_thread::builder()` から作る**（`Harness::builder` などを直接使うと `window_lease` が落ちます）。窓を持つ試験は貸し出しで 1 つずつ走ります（lavapipe の中で同時に装置を作ると落ちることがあったため）。描画の設定は `common::app` か `.renderer(common::shared_gpu::renderer())`（`.wgpu()` は窓ごとに装置と 3D のパイプラインをビルドし直すので使いません。例外は、装置を破棄する・誤りの受け口を付けて共用の装置を壊す `gpu_lost` と、製品と同じ装置の設定が要る `view3d_fx` で、自前の装置を貸し出しの中で作ります）。GPU の接続はプロセスで 1 つを共有し、`Renderer`・テクスチャ・3D の絵は窓ごとに作り直します。窓を作らなくても GPU の装置を作る試験（製品のスレッドで GPU の確認・ベイクをする試験、`common::canvas_device::begin`）は、先頭で `common::gpu_thread::lease()` を取ります（`canvas_device::begin` は中で取ります）。
- `crates/yolu-gpu/tests/` の GPU 試験は、装置（`GpuPainter::new` など）を作る前に `support::gpu_lease::lease()` を呼びます。同じ実行ファイルの別の試験のスレッドと装置を同時に作って使うと、lavapipe の中でプロセスごと落ちることがあったためです（1 つの試験が装置を何個作っても 1 回の貸し出しで足ります）。
- **一時のフォルダ**は `common::tmp::test_dir(タグ)`（試験が終わると消えます）か、自分で作った所で `common::tmp::clean_up_after_test(&dir)` を呼びます。**Live Link の名前**は `common::names::unique_name(接頭辞, タグ)` で作ると、鍵・ソケット・ロックのファイルも試験の終わりに消えます（ロックのファイルは製品が消さないため）。調べるために残したいときは `YOLUPAINTER_KEEP_TEST_FILES=1` を付けます。
- 「書き直さない」を更新時刻で確かめるときは、`common::tmp::backdate(&path)` で更新時刻を少し前にしてから比べます（時刻の粒度より早い書き直しを見逃さず、`sleep` を待たない）。
- 試験が自分の実行ファイルを子として起こすとき（`--exact` に試験名を渡す形）は、束の中では名前にモジュールの道筋（`livelink::child_unity`）が付きます。`module_path!()` から作ってください。

同じ束・同じ試験を続けて回して揺れを調べるには `tools/repeat-tests.py` を使います（`--shuffle` で回ごとに試験の順を混ぜ、順序への依存も調べます）。

```sh
tools/repeat-tests.py --rounds 20 --out /tmp/repeat --test gui_canvas --test gui_shell --test gui_view3d
tools/repeat-tests.py --rounds 20 --out /tmp/repeat --shuffle --test headless
```

ビルドし直しの時間は、ほとんどが yolu-app のライブラリ 1 つの rustc です（葉の 1 ファイルを変えたあと: `cargo check` 約 6 秒、`cargo build` 約 20 秒、`cargo test -p yolu-app --test headless --no-run` 約 25 秒。リンクは 1 本 1 秒前後）。環境変数 `CARGO_INCREMENTAL=0` を付けていなければ、dev のビルドは増分で、同じ変更の後のビルドし直しは 3〜5 秒になります（キャッシュは `target/debug/incremental` に約 1.4 GB。ディスクが厳しいときだけ 0 にします）。

### yolu-core・yolu-io の結合試験の置き方

yolu-app と同じく、`crates/yolu-core/tests/`・`crates/yolu-io/tests/` の試験も性質ごとの束（`tests/<束>/main.rs`）にまとめています。試験の名前は `<ファイル名>::<試験名>` です。

| クレート | 束（`--test <束>`） | 中身 |
| --- | --- | --- |
| yolu-core | `edit` | 層・層の操作・選択範囲・コピーとペースト・履歴・チャンネル・色調補正・書き出し・2D の合成 |
| | `effects` | フィルター・Generator・Anchor・パス・スマートマテリアルと、層のロック・層の操作とのつなぎ目 |
| | `reference` | 実 C# Core の正解・収録したハッシュとの全バイトの照合 |
| | `surface` | 3D の面への投影・面のストローク・対称・メッシュのマップ・ブラシの参照元・ステンシル・チャンネルの塗り |
| | `brush`・`document`・`golden`・`mix`・`parallelism`・`paths`・`pressure`（直下の 1 ファイル 1 本） | ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を変える、またはその値に頼ってダブの経路を確かめる試験 |
| yolu-io | `brushes` | ブラシの取り込み（同梱の筆先・GIMP・Photoshop・CLIP STUDIO の形式・信頼できないファイル） |
| | `psd_io` | PSD の書き出し・取り込み・焼き込み・調整レイヤー・C# の正解との照合 |
| | `ylp` | .ylp の形式の読み書き・前の版との互換・断り方・C# の書き手との一致・形式の仕様の文書 |
| | `projects` | 大きな .ylp・復旧の世代・ライブラリ・棚・配布用の写し・保存の並列・新しいプロジェクト |
| | `mesh` | メッシュのマップのベイク・レイの探索（BVH） |

```sh
cargo test -p yolu-core --test reference                 # 1 つの束
cargo test -p yolu-core --test reference seam_golden::   # 束の中の 1 ファイル（以前の `--test seam_golden`）
cargo test -p yolu-io --test ylp format_doc::            # 以前の `--test format_doc`
```

- 足し方は yolu-app と同じです（内容に近い束のフォルダにファイルを置き、その `main.rs` に `mod 名前;` を足す）。足し忘れと、直下に理由の無い 1 ファイル 1 本が増えたことは、`yolu-core/tests/edit/bundle_layout.rs`・`yolu-io/tests/ylp/bundle_layout.rs` が落ちて知らせます。
- 共通の部品（`attach_support`・`golden_update`・`brush_files` など）は、束の `main.rs` で `#[path]` を付けて 1 度だけ宣言し、ファイルでは `use crate::<部品>;` と書きます。`include_str!`・`include_bytes!` の道はファイルの場所から数えます（束のフォルダの 1 つ上が `tests/`）。
- 使用メモリ（VmHWM）を測る手で回す試験（`#[ignore]` の `bigdoc::measure`・`psd_stream::measure`）は、名前で絞って回します（束のほかの試験と同じプロセスで同時に走ると、測りが乱れます）。

## CI

`.github/workflows/ci.yml` は `pull_request`・`workflow_dispatch` で起動します（同じブランチの古い実行は取り消します）。`main` への push では動かしません（main は CI を通した PR からしか変わらず、push の CI は PR の最後の CI と同じ中身をもう一度ビルドするだけになるため）。main 向けの PR では、試験のジョブと並べて、配る物のビルド（`dist-plan` → `dist`。`.github/workflows/dist-build.yml`）も走ります。配布はその成果物を受け取ります（[RELEASING.md](RELEASING.md#配る物をビルドする場所と受け取る道)）。外の Actions はコミットの SHA で固定し、版の名前をコメントに書いています。上げるときは、その版のタグが指すコミットを確かめてから SHA を書き換えます。

- Linux の試験と静的検査（`ubuntu-24.04`）: `cargo test --workspace --exclude yolu-app --exclude yolu-gpu --locked --no-fail-fast`（描画しないクレート。試験は既定の並列）、配る物の道具の試験、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`。
- Linux の画面の試験（`ubuntu-24.04`）: Xvfb と Mesa の lavapipe（`WGPU_BACKEND=vulkan`）のソフトウェア描画で、yolu-gpu・yolu-app の試験を `tools/render-tests.py` が回します。試験の実行ファイルごとの別のプロセスを 3 列に並べ（列の中は順に）、プロセスの中は `--test-threads=1` です（窓・GPU の装置を同じプロセスで同時に作ると lavapipe の中で落ちることがあったため。プロセスどうしは別の装置）。`YOLUPAINTER_REQUIRE_GPU=1` を付けるので、アダプターを取れないと GPU の試験は飛ばずに落ちます。回す間は全体で 20 分（`--time-limit`）で区切り、超えた試験はプロセスのグループごと殺して失敗にし、止まった試験の名前が分かるようにログの終わりをすぐに出します（その列の残りは「回さず」として落ちた物に並びます）。単体試験は `--lib`・`--bins`、ドキュメントの試験は `--doc` で回し、どれでも回らない試験の target（example・bench の `test = true`）があると、並べる前に止まります。手元で同じ形に回すときは `xvfb-run -a tools/render-tests.py --log-dir <フォルダ>`（`--lanes 1` で 1 本ずつ）。runner の Ubuntu の版は、収録済みの正解が glibc と Mesa の版に結びつくので固定しています。
- Windows（`windows-latest`、MSVC）: `cargo build -p yolu-app -p yolu-cli --locked`、core・io・protocol・ops・cli の試験、app の `--lib` と、束の中の `headless_` の試験（`--test gui_shell -- update::headless_`・`--test headless -- brush_list::headless_ recovery::headless_ livelink_files::headless_ saved_selections::headless_ pose_saved::headless_`。復旧の OS のロックと置換、Live Link の受け渡しのフォルダとファイルの置換、.ylp の置換を含む）。GPU・画面の統合試験は対象外です。
- 両 OS で [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache) を使い、同じブランチの古い CI は後続の実行で取り消します。

Unity 版 C# を実行する正解の再生成・照合は、Unity 版のソースと Unity 同梱の .NET・Mono が必要なため、この CI では回しません。収録済みの人工データを使う Rust の照合試験は通常の `cargo test` に含みます。

CI の定義は `actionlint .github/workflows/ci.yml` で実行せずに検査できます。runner の版を上げたときは、Ubuntu の Mesa の版による画面の正解との差を確認してください。ソフトウェア描画での結果は、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。

## Windows 向けの画面なし試験（Wine）

Linux 上で Python 3.10 以降、Wine、MinGW-w64 と Rust の `x86_64-pc-windows-gnu` ターゲットを用意し、`tools/wine-tests.sh` を実行します。古い Wine で `bcryptprimitives.dll` が不足する場合だけ `--compat-bcrypt` を付けます。時間制限は `--timeout 180` のように秒で指定できます。

core・io・protocol・ops・cli と app の画面なし試験が対象です（`yolu-cli` は、本物の `yolupainter-cli.exe` を標準入出力で動かす MCP の試験と、名前付きパイプでアプリの代わりの待ち受けにつなぐ試験を含みます）。`--package yolu-protocol --package yolu-ops` のようにクレートを絞れます（全部をビルドすると wgpu・egui まで Windows 向けにビルドするため、通信だけを確かめたいとき用）。ログと結果は `target/wine-tests/summary.json` と同じフォルダに残ります。GPU・画面の試験は対象外で、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。互換 DLL は試験専用で、製品に同梱しません。

Windows のインストーラー（NSIS）を画面なしで通す試験は `python3 tools/test-installer.py`（Wine・MinGW-w64・`makensis` が要る。内容は [RELEASING](RELEASING.md#windows-のインストーラー)）です。`tools/wine-tests.sh` の app の試験に含まれる通信の試験（`update::http`）は、同じ機械の `http://127.0.0.1` に立てた小さなサーバーへ、Windows では WinHTTP の本物で接続します。

Wine は DACL（Live Link の鍵・名前付きパイプ・共有メモリのファイルを自分だけにする設定）をファイルやフォルダに保存しないので、`yolu-protocol` の DACL の中身を調べる試験（`private::windows_tests`）は Wine では呼び出しが通ることまでを見て、中身は本物の Windows の `cargo test -p yolu-protocol` で確かめます。別のユーザーとして開けないことを確かめる試験（`another_user_can_neither_connect_nor_read_the_files`）は、Linux で `sudo -n -u nobody` が使えるときだけ走ります。同じく、つないだ先がなりすませない（パイプを匿名の段で開く）ことを調べる試験（`windows_pipe::the_server_cannot_impersonate_the_bridge_after_reading`）も、Wine はなりすましの段を保存しないので呼び出しが通るまでで、中身は本物の Windows で確かめます。

## 配布用の許諾全文

Python 3.10 以降と Cargo を使います。

```sh
python3 tools/third-party.py --bundle --offline
```

初回など依存や原文が取得済みでない場合は `--offline` を外して実行します。照合に成功すると `target/third-party/<クレート>/THIRD_PARTY_LICENSES.txt` ができるので、該当する配布物に同梱します。追加の Python パッケージは不要です。

`--target x86_64-pc-windows-msvc` または `--target x86_64-unknown-linux-gnu` を指定すると、対象別に照合して `target/third-party/<target>/<クレート>/` へ出力します。省略時は従来の Windows GNU が対象です。`--package yolu-update` で更新クレートも確認できます。配布の詳細は [RELEASING](RELEASING.md) を参照してください。`tools/licenses-reviewed.json` に原文と SHA-256 を記録し、原文の欠落・変更や未確認の版・許諾では生成を失敗させます。製品ごとの範囲とクレート以外の表記は [THIRD_PARTY.md](../THIRD_PARTY.md) にあります。

## CPU の速さをまとめて測る

Linux で `tools/bench-all.sh --runs 5 --threads 4` を実行すると、既存の Rust と C# の合成・ブラシ・面のベンチを同じ回数・並列上限・CPU 割当で順に測り、
`target/bench-all/summary.md` に比較表、同じ場所にログと実行条件を保存する。Python 3.10 以降・taskset・上記の C# 用の Unity 同梱ツールが必要。
`--source DIR` で Unity 版の場所、`--only blur` などで M2 ブラシの種類を絞れる（合成・通常ブラシ・面は常に測る）。
合成・ブラシは予熱 2 回を除き、面は予熱なしの中央値。フィルターはレベル補正・ブラシのぼかし／指先を含む。GPU と独立フィルター全種は対象外。

## 画素の計算の SIMD

x86_64 では、合成（Normal チャンネルを含む）・調整の層・フィルターの画素の計算に AVX2（と FMA）・SSE4.1 を使い、実行時に CPU が持つ一番広い道を選ぶ
（Windows の配布物も同じ）。それ以外の CPU（aarch64 など）は、今までの画素ごとの計算を使う。結果のバイトはどの道でも同じで、試験が道ごとに画素ごとの式と比べる。
環境変数 `YOLU_SIMD`（`scalar`・`sse41`・`avx2`）で狭い道へ下げられる（CPU が持たない広い道には上げない）。
2D の合成（矩形・タイルの束・歩幅つきの粗い合成・グループの出力）はこの行の核で重ねる。参照の `composite_pixel`（画素ごとの式）とバイトが同じで、試験が全モード・マスク・クリッピング・グループ・調整の層・Normal チャンネルを道ごとに比べる。
表示に寄与する層・複数の層・グループの結合も、タイルの合成で焼く（下の層へ結合する `merge_down` は、下の層の画素を下地にする方法があるので画素ごとの式のまま）。

`cargo run --release -p yolu-core --example simd_bench [blend|adjust|filter|kernel|all] [回数]` が、合成モード・調整の種類・フィルターごとの時間
（1 タイルと 4096²。`kernel` は行の核だけの ns/画素）を測る。スレッドは `SIMD_THREADS`（既定 1）、名前の絞り込みは `SIMD_FILTER`（カンマ区切り）。

2D のブラシのダブの画素（丸・筆先の画像・紙の質感・デュアル・指先・ぼかし・クローン・色の混ぜ・ダブごとの色）も同じ道で、行ごとにレーンで描く。
参照は画素ごとの式（`YOLU_SIMD=scalar`）で、試験が乱数で振ったブラシ・層・タイルの大きさ・点の列を道ごとに描いて、層の全バイトとダブの数を比べる。
選択範囲・透明部分のロックは、色を塗る・消すだけのブラシなら行の核、画素ごとの色・効果のブラシでは画素ごとの式。ステンシル・乗算でない紙の質感・3D の面のダブも画素ごとの式のまま。ダブをタイルごとにワーカーで描くかは、
外接の箱の大きさに画素ごとの時間の見積もり（ブラシの種類で決まる）を掛けて決める。速いブラシの大きなダブは、ワーカーを起こす費用が勝つので直列で描く。

`cargo run --release -p yolu-core --example stroke_bench -- --threads 1` が、ブラシの種類 × 大きさ × 間隔ごとに、4096² の文書への 1 ストロークの時間を測る
（`--threads 1` は CPU 時間、2 以上は壁時計。`--features stroke-profile` を付けると段ごとの時間も出る）。ペンの入力が画面に出るまでの遅れは
`cargo run --release -p yolu-app --example stroke_latency`、取り込んだブラシは `cargo run --release -p yolu-io --example stroke_bench_imported -- --bundled 6` で測る。
