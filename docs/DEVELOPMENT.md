# 開発用の手順

以下はリポジトリのルートで実行します。通常のビルドと操作は [README](../README.md) を参照してください。

## 試験

```sh
cargo test --workspace --locked
```

コマンドラインと MCP サーバーは `cargo test -p yolu-cli`（`cargo clippy -p yolu-cli --all-targets -- -D warnings`）で、作りと試験の分け方は [crates/yolu-cli/README.md](../crates/yolu-cli/README.md) にあります。

GPU・画面の試験には動作する描画バックエンドが必要です。GPU 試験にはアダプターがないと処理を省くものがあるため、結果の passed だけで描画確認済みとは判断せず、標準エラーの理由も確認してください。環境変数 `YOLUPAINTER_REQUIRE_GPU=1` を付けると、省かずに落とします（CI の画面の試験は付けています）。詳しくは [yolu-gpu](../crates/yolu-gpu/README.md#検証と計測) を参照してください。

レイヤーの合成の式は Rust の f32 の式が正本で、合成を通る正解は Rust で撮り直しています。正解を撮り直すときは `YOLU_GOLDEN_UPDATE=1 cargo test -p yolu-core -p yolu-io` で、違った正解だけを今の出力で書き直し、差分を見て意図した変化だけかを確かめます。Normal のチャンネルの合成とブラシの画素も f32 の式で、Rust で撮り直しています。フィルター・Generator の値など f64 のままの式は、今も Unity 版 C# の正解と照らしています。

Unity 版 C# との照合には、リポジトリに収録された人工データを使います。core の正解の再生成ツールは `tools/csharp-golden/run.sh` です（出力先は `--out` で指定できます）。編集できるパスの正解は `tools/csharp-golden/run-paths.sh <出力先>` で作り、試験が読む `crates/yolu-core/tests/golden/paths` へ出力します。効果とレイヤーのロック・レイヤーの操作のつなぎ目の正解（事例ごとの SHA-256）は `tools/csharp-golden/run-seam.sh` で `crates/yolu-core/tests/golden/seam.txt` へ作ります。食い違ったときは、試験を `SEAM_DUMP_DIR=<フォルダ>` で回して Rust の生のバイト列を書き出し、`run-seam.sh dump <事例名> <出力>` の C# の側と `cmp` で比べます。どれも Unity 版のソースと Unity 同梱の .NET・Mono が必要です。Unity 版の `Runtime/Core` などは Unity ブリッジの 0.5.0 で外れたので、Unity ブリッジのリポジトリのタグ `0.4.0` を取り出し、環境変数 `YOLUPAINTER_UNITY_SOURCE` にその場所を渡します（`tools/csharp-golden/` のツールと `tools/bench-all.sh` の既定の場所は `/workspace`）。PSD の写し（core ⇔ PSD）の正解は `tools/csharp-golden/run.sh psd` で作ります。I/O と PSD のデータ形式・再生成方法は [I/O のフィクスチャ](../crates/yolu-io/tests/fixtures/README.md)と [PSD のフィクスチャ](../crates/yolu-io/tests/fixtures/psd/README.md)を参照してください。ブラシ形式の取り込みの正解は `tools/csharp-golden/brushes.sh` で作り（Rust の試験が入力を書き、C# の読み手に通して `crates/yolu-io/tests/fixtures/brushes/` へ出力）、違いの調査は `BRUSH_GOLDEN_SHOW=<記録の番号>` でその入力の完全な指紋を出します。

### aarch64（NEON）の試験を x86_64 の Linux で回す

aarch64 の道（NEON）は、Mac の CI のほかに、x86_64 の Linux でも QEMU のユーザーモードで試験できます。Debian・Ubuntu では `qemu-user-static`・`gcc-aarch64-linux-gnu`・`libc6-dev-arm64-cross` を入れ、Rust のターゲット `aarch64-unknown-linux-gnu` を追加します（`rustup target add aarch64-unknown-linux-gnu`）。target は普段の物と分けて回します。

```sh
CARGO_TARGET_DIR=target-arm CARGO_INCREMENTAL=0 \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64-static -L /usr/aarch64-linux-gnu" \
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
cargo test -p yolu-core --lib --target aarch64-unknown-linux-gnu --locked
```

QEMU は浮動小数点の演算を IEEE どおりに再現するので、NEON の結果のバイトがスカラーと一致するかを確かめられます（`YOLU_SIMD=scalar`・`YOLU_SIMD=neon` を付けて道を固定しても回せます）。速さは QEMU では測れないので、Mac の実機で測ります（下の「画素の計算の SIMD」）。型だけを見るなら、`cargo clippy -p yolu-core --all-targets --target aarch64-unknown-linux-gnu -- -D warnings` が QEMU なしで通ります。

### yolu-app の結合試験の置き方

`crates/yolu-app/tests/` の試験は、性質ごとに数本の実行ファイル（「束」）にまとめています。1 ファイルを 1 本の実行ファイルにすると、試験の数だけアプリ全体のリンクと共通部品のビルドし直しが増え、`target/` が試験の実行ファイルだけで 10 GB を超えるためです。

| 束（`cargo test -p yolu-app --test <束>`） | 中身 |
| --- | --- |
| `headless` | ウィンドウも GPU のデバイスも作らない試験（文書・保存（バックグラウンドの保存・選択範囲・ポーズを含む）・取り込み・Live Link のフォルダの受け渡しと頼み・外からの操作（MCP の受け口）・ディスクキャッシュ・ツールの並び・ソースの文言の検査）。同時に走る |
| `gui_canvas` | キャンバス・ツール・ブラシ・選択・色・効果の画面と、パネルの既定の並び（`egui_kittest`） |
| `gui_shell` | ウィンドウの全体・メニュー・設定のウィンドウとショートカットの編集・パネルの配置と別ウィンドウ・文書の出し入れ（PSD のドロップを含む）・閉じる流れと保存の途中の終了・GPU のデバイスの喪失・復旧・更新・Live Link の画面と受け取りの上限・言語・アセット・ライブラリ |
| `gui_view3d` | 3D ビュー（アンチエイリアス・ブルーム・視点の操作・3D の上の選択と描画を含む）・モード（ペイント／編集／ポーズ）・マテリアルの見た目（lilToon を含む）・ポーズ・テクスチャセット・出力 |
| `threads`・`window_lease`・`windowpos`（直下の 1 ファイル 1 本） | プロセス全体の状態を持つ試験: rayon の全体のプール・ウィンドウの貸し出しの数え・覚えたウィンドウの置き場所 |

束の中のファイルは `tests/<束>/<名前>.rs`、束の入口は `tests/<束>/main.rs` の `mod` の並びです。試験の名前は `<ファイル名>::<試験名>` になるので、ファイルや試験名で絞れます。

```sh
cargo test -p yolu-app --test gui_shell                      # 1 つの束
cargo test -p yolu-app --test gui_shell i18n::               # 束の中の 1 ファイル（以前の `--test i18n`）
cargo test -p yolu-app --test headless no_instruction_text:: # 以前の `--test no_instruction_text`
cargo test -p yolu-app --test gui_canvas layerops::undo      # ファイル名と試験名の一部
cargo test -p yolu-app -- --list | grep layerops             # どの束にあるか（`Running tests/<束>/main.rs` の下）
```

- **新しい試験を追加する**: 画面を作らないなら `headless/`、作る（`common::app`・`common::gpu_thread::builder`）なら内容に近い `gui_*/` にファイルを置き、その束の `main.rs` に `mod 名前;` を 1 行追加します。追加し忘れは `headless/bundle_layout.rs` が落ちて知らせます（置いただけではビルドされず、走らないのに通るため）。ファイルの先頭で `use crate::common;` と書くと、共通部品（`tests/common/`）を `common::…` で使えます。
- 直下（`tests/<名前>.rs`）に置いた 1 ファイル 1 本の試験を束へ移すには、`git mv tests/<名前>.rs tests/<束>/<名前>.rs`、ファイル先頭の `mod common;` を `use crate::common;` に替え、束の `main.rs` に `mod <名前>;` を追加し、コメントや文書の `--test <名前>` を `--test <束> <名前>::` に直します（試験の名前は `<名前>::<元の名前>` になります）。
- **直下に 1 ファイル 1 本で置く**のは、プロセス全体の状態（rayon の全体のプール・環境変数・ウィンドウの貸し出しの数え・覚えたウィンドウの置き場所）を変える・数える試験だけです。束の中の試験どうしは同じプロセスで走るので、そのような試験を混ぜると順序で結果が変わります。
- **ウィンドウ（harness）は必ず `common::gpu_thread::builder()` から作る**（`Harness::builder` などを直接使うと `window_lease` が落ちます）。ウィンドウを持つ試験は貸し出しで 1 つずつ走ります（lavapipe の中で同時にデバイスを作ると落ちることがあったため）。描画の設定は `common::app` か `.renderer(common::shared_gpu::renderer())`（`.wgpu()` はウィンドウごとにデバイスと 3D のパイプラインをビルドし直すので使いません。例外は、デバイスを破棄する・誤りの受け口を付けて共用のデバイスを壊す `gpu_lost` と、製品と同じデバイスの設定が要る `view3d_fx` で、自前のデバイスを貸し出しの中で作ります）。GPU の接続はプロセスで 1 つを共有し、`Renderer`・テクスチャ・3D の絵はウィンドウごとに作り直します。ウィンドウを作らなくても GPU のデバイスを作る試験（製品のスレッドで GPU の確認・ベイクをする試験、`common::canvas_device::begin`）は、先頭で `common::gpu_thread::lease()` を取ります（`canvas_device::begin` は中で取ります）。
- `crates/yolu-gpu/tests/` の GPU 試験は、デバイス（`GpuPainter::new` など）を作る前に `support::gpu_lease::lease()` を呼びます。同じ実行ファイルの別の試験のスレッドとデバイスを同時に作って使うと、lavapipe の中でプロセスごと落ちることがあったためです（1 つの試験がデバイスを何個作っても 1 回の貸し出しで足ります）。
- **一時のフォルダ**は `common::tmp::test_dir(タグ)`（試験が終わると消えます）か、自分で作った所で `common::tmp::clean_up_after_test(&dir)` を呼びます。Live Link の受け渡しのフォルダは `common::livelink::Exchange::new(タグ)` が `common::tmp::test_dir` の下に作るので、置いた頼み・返事ごと試験の終わりに消えます。調べるために残したいときは `YOLUPAINTER_KEEP_TEST_FILES=1` を付けます。
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
| yolu-core | `edit` | レイヤー・レイヤーの操作・選択範囲・コピーとペースト・履歴・チャンネル・色調補正・書き出し・2D の合成 |
| | `effects` | フィルター・Generator・Anchor・パス・スマートマテリアルと、レイヤーのロック・レイヤーの操作とのつなぎ目 |
| | `reference` | 実 C# Core の正解・収録したハッシュとの全バイトの照合 |
| | `surface` | 3D の面への投影・面のストローク・対称・メッシュのマップ・ブラシの参照元・ステンシル・チャンネルの塗り |
| | `brush`・`document`・`golden`・`mix`・`parallelism`・`paths`・`pressure`（直下の 1 ファイル 1 本） | ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を変える、またはその値に頼ってダブの経路を確かめる試験 |
| | `seam_memory`（直下の 1 ファイル 1 本） | 確保を数える `#[global_allocator]` を置くので、1 本の実行ファイルにする。継ぎ目をまたぐ評価の実際の確保が見積りに収まることを確かめる |
| yolu-io | `brushes` | ブラシの取り込み（同梱の筆先・GIMP・Photoshop・CLIP STUDIO の形式・信頼できないファイル） |
| | `psd_io` | PSD の書き出し・取り込み・焼き込み・調整レイヤー・C# の正解との照合 |
| | `ylp` | .ylp の形式の読み書き・前の版との互換・断り方・C# の書き手との一致・形式の仕様の文書 |
| | `projects` | 大きな .ylp・復旧の世代・ライブラリ・アセット・配布用の写し・保存の並列・新しいプロジェクト |
| | `mesh` | メッシュのマップのベイク・レイの探索（BVH） |

```sh
cargo test -p yolu-core --test reference                 # 1 つの束
cargo test -p yolu-core --test reference seam_golden::   # 束の中の 1 ファイル（以前の `--test seam_golden`）
cargo test -p yolu-io --test ylp format_doc::            # 以前の `--test format_doc`
```

- 追加の仕方は yolu-app と同じです（内容に近い束のフォルダにファイルを置き、その `main.rs` に `mod 名前;` を追加する）。追加し忘れと、直下に理由の無い 1 ファイル 1 本が増えたことは、`yolu-core/tests/edit/bundle_layout.rs`・`yolu-io/tests/ylp/bundle_layout.rs` が落ちて知らせます。
- 共通の部品（`attach_support`・`golden_update`・`brush_files` など）は、束の `main.rs` で `#[path]` を付けて 1 度だけ宣言し、ファイルでは `use crate::<部品>;` と書きます。`include_str!`・`include_bytes!` の道はファイルの場所から数えます（束のフォルダの 1 つ上が `tests/`）。
- 使用メモリ（VmHWM）を測る手で回す試験（`#[ignore]` の `bigdoc::measure`・`psd_stream::measure`）は、名前で絞って回します（束のほかの試験と同じプロセスで同時に走ると、測りが乱れます）。

## CI

`.github/workflows/ci.yml` は `pull_request`・`workflow_dispatch` で起動します（同じブランチの古い実行は取り消します）。`main` への push では動かしません（main は CI を通した PR からしか変わらず、push の CI は PR の最後の CI と同じ中身をもう一度ビルドするだけになるため）。代わりに `.github/workflows/main-tested.yml` が、main の中身（ファイルの木）が PR の最後の CI を通した中身と同じで、その CI が成功しているかだけを確かめます（README の CI の印はこの結果です）。違うとき（PR の枝が main より古いまま入った）は失敗にするので、`ci.yml` を手で動かします。main 向けの PR では、試験のジョブと並べて、配る物のビルド（`dist-plan` → `dist`。`.github/workflows/dist-build.yml`）も走ります。配布はその成果物を受け取ります（[RELEASING.md](RELEASING.md#配る物をビルドする場所と受け取る道)）。外の Actions はコミットの SHA で固定し、版の名前をコメントに書いています。上げるときは、その版のタグが指すコミットを確かめてから SHA を書き換えます。

- Linux の試験と静的検査（`ubuntu-24.04`）: `cargo test --workspace --exclude yolu-app --exclude yolu-gpu --locked --no-fail-fast`（描画しないクレート。試験は既定の並列）、配る物のツールの試験、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`。
- Linux の画面の試験（`ubuntu-24.04`）: Xvfb と Mesa の lavapipe（`WGPU_BACKEND=vulkan`）のソフトウェア描画で、yolu-gpu・yolu-app の試験を `tools/render-tests.py` が回します。試験の実行ファイルごとの別のプロセスを 3 列に並べ（列の中は順に）、プロセスの中は `--test-threads=1` です（ウィンドウ・GPU のデバイスを同じプロセスで同時に作ると lavapipe の中で落ちることがあったため。プロセスどうしは別のデバイス）。`YOLUPAINTER_REQUIRE_GPU=1` を付けるので、アダプターを取れないと GPU の試験は飛ばずに落ちます。回す間は全体で 20 分（`--time-limit`）で区切り、超えた試験はプロセスのグループごと殺して失敗にし、止まった試験の名前が分かるようにログの終わりをすぐに出します（その列の残りは「回さず」として落ちた物に並びます）。単体試験は `--lib`・`--bins`、ドキュメントの試験は `--doc` で回し、どれでも回らない試験の target（example・bench の `test = true`）があると、並べる前に止まります。手元で同じ形に回すときは `xvfb-run -a tools/render-tests.py --log-dir <フォルダ>`（`--lanes 1` で 1 本ずつ）。runner の Ubuntu の版は、収録済みの正解が glibc と Mesa の版に結びつくので固定しています。
- Linux の通信の見張り（`ubuntu-24.04`。画面の試験とは別のジョブ）: 本物のアプリを `strace` の下で動かし、外への通信の試みが無いことを `tools/network-watch.py` が確かめます（[通信の見張り](#通信の見張り)の段 2）。別のジョブにしているのは、更新の確認を動かすために更新用の公開鍵（使い捨て）を組み込んで作る必要があり、その環境変数を変えると yolu-update と yolu-app を作り直すため（同じジョブに置くと、画面の試験の作った物のキャッシュが、次の回の画面の試験のビルドのやり直しになります）。キャッシュは画面の試験のジョブと共有キー（`shared-key: linux-render`）で読むだけにし（`save-if: false`）、鍵つきの物で上書きしません。xdotool が効かず場面を飛ばしたときは、終わりに `::warning::` を出します。ツールの試験（`tools/test-network-watch.py`）は「Linux の試験と静的検査」でも回ります。
- cargo-deny（`ubuntu-24.04`。`.github/workflows/cargo-deny.yml`）: 週 1 回（月曜 03:23 UTC）と手動だけで、毎回の CI には入れません。`deny.toml` の決まりで advisories（既知の脆弱性・取り下げられた版）・bans・licenses・sources を確かめます
  （手元では `cargo deny --locked check advisories bans licenses sources`）。cargo-deny は版を固定して `cargo install` します（`deny.toml` の書式は版に結びつくので、上げるときは一緒に確かめます）。
  `paste`（RUSTSEC-2024-0436、保守の終わり）は egui_dock がビルドの時だけ使うマクロで、脆弱性ではないので理由つきで無視しています。見つけた脆弱性は、更新で直せるか、使われ方に当たらないかを確かめ、後者だけを理由つきで `ignore` に足します。
- Windows（`windows-latest`、MSVC）: `cargo build -p yolu-app -p yolu-cli --locked`、core・io・brush-sets・protocol・ops・cli の試験、app の `--lib` と、束の中の `headless_` の試験（`--test gui_shell -- update::headless_`・`--test headless -- brush_list::headless_ recovery::headless_ livelink_files::headless_ saved_selections::headless_ pose_saved::headless_`。復旧の OS のロックと置換、Live Link の受け渡しのフォルダとファイルの置換、.ylp の置換を含む）。GPU・画面の統合試験は対象外です。
- macOS（`macos-14`、Apple Silicon）: Windows のジョブの前半と同じ手順で、`cargo build -p yolu-app -p yolu-cli --locked`、core・io・brush-sets・protocol・ops・cli の試験、app の `--lib`。続けて、束の中の `headless_` の試験のうち `--test headless -- recovery::headless_ livelink_files::headless_`（復旧のロックと、Live Link の受け渡しのフォルダとファイルの置換）だけを回します（更新・ブラシの一覧・選択範囲・ポーズの保存の `headless_` は、このジョブでは回しません。Linux の画面の試験が Unix で回しています）。macOS のタブレットの入力（`pen::mac_tablet`）がビルドできることと、値の直し（`pen::tablet`。OS に依らないので Linux でも回る）に加えて NSEvent の定数との一致の試験が通ることを見ます。GPU・画面の統合試験は対象外です。配る物（.app と ad-hoc の署名）はこのジョブでは作らず、`dist` のジョブ（`dist-build.yml`）が macOS の runner で作ります（[配布の手順](RELEASING.md#macos-の試作)）。
- どの OS でも [Swatinem/rust-cache](https://github.com/Swatinem/rust-cache) を使い、同じブランチの古い CI は後続の実行で取り消します。

Unity 版 C# を実行する正解の再生成・照合は、Unity 版のソースと Unity 同梱の .NET・Mono が必要なため、この CI では回しません。収録済みの人工データを使う Rust の照合試験は通常の `cargo test` に含みます。

CI の定義は `actionlint .github/workflows/*.yml` で実行せずに検査できます。runner の版を上げたときは、Ubuntu の Mesa の版による画面の正解との差を確認してください。ソフトウェア描画での結果は、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。

## 通信の見張り

アプリが外へ通信するのは、利用者が選んだ更新の確認だけです（[README](../README.md)・[INSTALL](INSTALL.md) の約束）。外からの操作は、設定で入れている間だけ `127.0.0.1` で待ちます。
この約束が、新しいコードや依存で静かに破れないよう、コードと依存を読む見張り（段 1）が毎回の CI で回ります。

通信の部品の見張り（`cargo xtask netguard`。毎回の CI）と cargo-deny（週 1 回）は役が違います。netguard は「アプリが外へ通信する部品が入っていないか」を、ソースと依存のソースの字句から見ます
（ライセンスも脆弱性も見ません）。cargo-deny は「入っている依存に既知の脆弱性・許していないライセンス・知らない出どころ（registry 以外・git）が無いか」を見ます（通信するかどうかは見ません）。
どちらかが通っても、もう一方の代わりにはなりません。

### 段 1: コードと依存を読む

```sh
cargo xtask netguard         # 許す一覧の使われ方の表を出す。一覧の外の出口や依存の問題があれば終了コード 1
cargo test -p xtask netguard # 同じ確かめ（CI では `cargo test --workspace --exclude yolu-app --exclude yolu-gpu` の中で回る）と、見張りが効くことの試験
```

実装は `crates/xtask/src/netguard/` です（`mod.rs`: 全体の流れと結果の表、`lex.rs`: 字句、`markers.rs`: 印と宛先、`allowed.rs`: 許す一覧、`deps.rs`: 依存、`uses.rs`: `use` の読み、`tests.rs`: 見張りが効くことの試験）。

- ソース: `crates/*/src/**/*.rs` と `crates/*/build.rs`（配らない xtask は除く）から、通信と外への出口の印（`MARKERS`）を探します。ソケットの型（`TcpStream`・`TcpListener`・`UdpSocket`・`std::net`・`tokio::net`・`rustix::net` など）、
  HTTP・TLS・WebSocket の部品（`hyper`・`reqwest`・`WinHttp*`・`WinInet`・`Urlmon` など）、プロセスの起動と「開く」の呼び出し（`Command::new`・`tokio::process`・`ShellExecute*`・`xdg-open`・`open::that`・egui の `open_url`/`hyperlink`）、
  `http://`・`https://` を含む文字列です。`Command::new` は `use std::process::Command as Cmd;` の別名や `tokio::process` でも見つけ、自前の `Command` 型は印にしません。
  コメントと文字列の中の識別子には反応せず、`#[cfg(test)]` を付けた項目、`#[cfg(test)] mod x;`（`#[path = "…"]` つきなら道の指すファイル）、モジュールの中の `#![cfg(test)]` は、配る物に入らないので除きます。
  `TcpListener::bind`・`TcpStream::connect` など、型の名前に `Listener`・`Stream`・`Socket` を含む型と、その別名（`use … as L;`・`type S = …;`）の `bind`・`connect` は、宛先が 127.0.0.1 と決まる形でなければ落ちます（一覧に足しても通りません）。
  許す形は、文字列（`"127.0.0.1:0"`・`"[::1]:0"`・`"localhost:80"`）、`(host, port)` の組、`SocketAddr::from/new/V4` などの包みで、ホストは文字列か `Ipv4Addr::LOCALHOST`・`Ipv6Addr::LOOPBACK` などの定数（`IpAddr::V4(…)` の包みも可）です。
  変数・`unwrap_or`・`if`・`match`・`UNSPECIFIED`・`"0.0.0.0"` は決まらないので、宛先を呼び出しの引数に直接書きます。
- 依存: `Cargo.lock` に HTTP の客・TLS・WebSocket/QUIC・名前の引き・テレメトリ・待ち受けの枠組みの名前（`reqwest`・`ureq`・`rustls`・`quinn`・`hickory-resolver`・`sentry` など）が入ると落ちます。
  `cargo metadata`（`--locked`。取得済みの依存が足りなければ取得を許して読み直す）で、`hyper`・`hyper-util`・`socket2`・`mio` を直接の依存に持つのと、tokio の機能 `net`・`full`・`process` を使うのが yolu-mcp だけであること、
  配るアプリ（yolu-app・yolu-cli）の依存に `open`・`opener`・`webbrowser` が入らないこと（今の `open` は試験用の依存だけ）、`socket2`・`mio`・`hyper`・`hyper-util` に依存するクレートが決めた範囲（`NET_PARTS`）から増えないこと、
  Windows の API の機能（`windows`・`windows-sys`）の通信（`Win32_Networking*`・`Win32_NetworkManagement*`・`Networking*`・`Web*`・`Win32_Web*`・`Win32_System_Com_Urlmon`・`Win32_Data_Xml_MsXml`）が、宣言でも Cargo が実際に有効にした機能
  （tokio・mio・socket2 が足す WinSock・IpHelper を含む）でも、決めた範囲（`WINDOWS_RESOLVED_NET`）から増えないことを確かめます。
  配るアプリの依存（本番とビルドスクリプトの閉包。試験だけの依存は除く）のソースも、同じ字句の読み手で読みます（今は約 440 クレート）。ソケット・名前の引き・HTTP の部品の印を使うクレートは、理由つきの一覧
  （`SOCKET_CRATES`。今は 25 名前）に載せ、載っていないクレートが使い始めたら落ちます。windows の宣言の束と、文字列・プロセスの起動の印は読みません。

落ちたときの文は、`<ファイル>:<行> の <印> が、許す一覧にありません` の形です。

許す一覧（`ALLOWED`）には、ファイルと印ごとに数（`Exactly`: 増えたら落ちる。`AtLeastOne`: 同じ種類の識別子が幾つも並ぶものだけ）と理由を書きます。数は印の出た数で、`use` の行の識別子も 1 件と数えます
（`use std::net::TcpStream;` の 1 行で `std::net` と `TcpStream` に 1 件ずつ）。足すときの決まり:

- 先に README・INSTALL の約束（更新の確認のほかに通信しない。外からの操作は設定で入れている間だけ `127.0.0.1`）を破らないか確かめます。破るなら、一覧に足さず、約束の側を決め直します。
- 理由には、何のための出口か（更新の確認・押したときだけブラウザーやファイル閲覧で開く・`127.0.0.1` の受け口・ビルドの手元など）を書きます。
- ソースの側の出口が無くなったら、その行を一覧から消します（使われない行も落ちます。`SOCKET_CRATES` は依存の更新で変わるので、使われなくなっても落ちません）。
- 足りない印は `MARKERS` に足し、`SNIPPETS`（試験の作り物のソース）にも 1 つ足します（両方の印の名前が揃わないと試験が落ちます）。

見張りが読むのはソースと依存のソース・宣言だけです。実際にどこへ接続するかは見ません（段 2 が見ます）。マクロが作るコードは追わず、機能（feature）で無効なコードも数えます。

### 段 2: 実際の通信を見る（Linux だけ）

```sh
# 更新の確認は、更新用の公開鍵を組み込んだビルドでだけ動く。使い捨ての公開鍵を渡して作る（鍵を変えると yolu-update と yolu-app を作り直す）
YOLUPAINTER_UPDATE_PUBLIC_KEY=6acc8faaa5f8beb6897bc7936ab6b930853dc4e563e4dc5f49a76f156ab16201 cargo build -p yolu-app -p yolu-cli --locked
xvfb-run -a -s "-screen 0 1920x1080x24" python3 tools/network-watch.py --log-dir /tmp/network-logs
python3 tools/test-network-watch.py   # ツール自身の試験（アプリは起動しない）
```

`strace`・`xdotool`・`xvfb`・Mesa の lavapipe（画面の試験と同じ）が要ります。`--only main,reopen,idle,update` で起動を絞れます。

`tools/network-watch.py` は、アプリ（`yolupainter`）を `strace -f` の下で xvfb の中に起動し、CLI（`yolupainter-cli`、これも `strace` の下）で操作して、`connect`・`bind`・`listen`・`sendto`・`sendmsg`・`sendmmsg`・
`execve` を読みます。届いたか拒まれたかは見ず、試みだけで判定するので、CI から GitHub に届かなくても同じ結果になります。設定のフォルダ・書き出し先・Live Link のフォルダは、起動ごとの一時のフォルダです。

- 起動は 4 回: `main`（受け口を入れて、起動・更新の問いを Esc で「いいえ」・レイヤーと効果の編集・`xdotool` の筆・保存・PNG とテンプレートと PSD の書き出し・Live Link で 3D のモデルを開く・保存・Ctrl+Q で終了）、
  `reopen`（保存した 3D のモデル入りの文書を起動の引数で開き直す・見本）、`idle`（受け口を入れない既定の設定。更新の問いは出たまま放っておく）、`update`（更新の確認を「する」にした起動）。
  描く操作は CLI・MCP の命令にないので、筆だけは `xdotool` で動かします（`xdotool` が無い・筆が届かないときは、その場面を飛ばして表に書きます）。
- 落とす条件: PC の中（AF_UNIX の画面や D-Bus・AF_NETLINK）と 127.0.0.1・::1 のほかへの接続・送信（`sendto`・`sendmsg`・`sendmmsg` の宛先は、スレッドが多いと結果の行にだけ出るので、そこからも読む）・待ち受け。
  名前の引き（宛先のポートが 53・853・5353。127.0.0.53 のようなループバックの中継も、systemd-resolved と avahi の UNIX ソケットも。glibc が passwd などにも開く nscd のソケットは数えない）。
  受け口を入れていない起動で待ち受けること、入れた起動で 127.0.0.1 のその番号以外で待つこと、待つ呼び出しが 1 つも見えないこと。`curl` を更新の確認の場面以外で起動すること、アプリが落ちたときのダイアログ（zenity など）の起動。
- 許す通信: 更新の確認の場面（`update` の起動）で、`curl`（子のスレッドも）の外への試み。ただし curl の引数は `update/http.rs` の `curl_args`（単体試験 `curl_arguments_are_exactly_these` が全体を固定）と同じ組だけ
  （`--data`・`-H`・`-b`・`-T`・`-K`・`--url` などが足されると落ちる）、取りに行く URL は `--` の後の 1 つで、`https://github.com/YozoraKurage/YoluPainter/` の下の `?`・`#`・`@` の無い形だけです。
  curl の接続先のアドレス（GitHub の配信先の IP や名前の引き）は固定しません。ほかのプロセスの外への試みは落とします。
- 始めに、外への connect・sendto・sendmmsg、本物の名前の引き（`getaddrinfo`）、ループバックを試みる小さなプログラムを `strace` で読み、見張りが捉えることを確かめます（`strace` が使えない環境や読みの崩れで、アプリの結果が空振りにならないように）。
- 終わりに、場面ごとの接続の数と宛先の表を出します（落ちたときは、どの起動の・どの場面の・どのプロセスの・どの呼び出しかを挙げます。ログの全文は `--log-dir`）。

保証の射程: Linux の `strace` で見える試み（システムコール）だけで、通した場面だけです。Windows は段 1 だけ（ソースと依存の見張り）で、段 2 に当たる試験はありません。画面を閉じる操作は Ctrl+Q のキーで通し、
外からの操作の受け口を実行中に入れて切る操作は通していません（入れた起動と入れない起動を別々に見ています。実行中に入れて切る動きは、`crates/yolu-app/tests/headless/mcp_server.rs` の試験が 127.0.0.1 の上で確かめます）。

## Windows 向けの画面なし試験（Wine）

Linux 上で Python 3.10 以降、Wine、MinGW-w64 と Rust の `x86_64-pc-windows-gnu` ターゲットを用意し、`tools/wine-tests.sh` を実行します。古い Wine で `bcryptprimitives.dll` が不足する場合だけ `--compat-bcrypt` を付けます。時間制限は `--timeout 180` のように秒で指定できます。

core・io・protocol・ops・cli と app の画面なし試験が対象です（`yolu-cli` は、本物の `yolupainter-cli.exe` を標準入出力で動かす MCP の試験と、アプリの代わりの MCP の受け口（`yolu_mcp::http`）へ 127.0.0.1 の HTTP でつなぐ試験を含みます）。`--package yolu-protocol --package yolu-ops` のようにクレートを絞れます（全部をビルドすると wgpu・egui まで Windows 向けにビルドするため、画面の無いクレートだけを確かめたいとき用）。ログと結果は `target/wine-tests/summary.json` と同じフォルダに残ります。GPU・画面の試験は対象外で、Windows 実機の描画・ペンタブ・Unity 接続の確認を兼ねません。互換 DLL は試験専用で、製品に同梱しません。

Windows のインストーラー（NSIS）を画面なしで通す試験は `python3 tools/test-installer.py`（Wine・MinGW-w64・`makensis` が要る。内容は [RELEASING](RELEASING.md#windows-のインストーラー)）です。`tools/wine-tests.sh` の app の試験に含まれる通信の試験（`update::http`）は、同じ PC の `http://127.0.0.1` に立てた小さなサーバーへ、Windows では WinHTTP の本物で接続します。

Wine は DACL（Live Link の受け渡しのフォルダとファイルを自分だけにする設定）をファイルやフォルダに保存しないので、`yolu-protocol` の DACL の中身を調べる試験（`private::windows_tests`）は Wine では呼び出しが通ることまでを見て、中身は本物の Windows の `cargo test -p yolu-protocol` で確かめます。

## 配布用の許諾全文

Python 3.10 以降と Cargo を使います。

```sh
python3 tools/third-party.py --bundle --offline
```

初回など依存や原文が取得済みでない場合は `--offline` を外して実行します。照合に成功すると `target/third-party/<クレート>/THIRD_PARTY_LICENSES.txt` ができるので、該当する配布物に同梱します。追加の Python パッケージは不要です。

`--target x86_64-pc-windows-msvc`・`--target x86_64-unknown-linux-gnu`・`--target universal-apple-darwin`（macOS の配布物。`aarch64-apple-darwin` と `x86_64-apple-darwin` の依存の和。この 2 つ単独も指せます）のどれかを指定すると、対象別に照合して `target/third-party/<target>/<クレート>/` へ出力します。省略時は従来の Windows GNU が対象です。`--package yolu-update` で更新クレートも確認できます。配布の詳細は [RELEASING](RELEASING.md) を参照してください。`tools/licenses-reviewed.json` に原文と SHA-256 を記録し、原文の欠落・変更や未確認の版・許諾では生成を失敗させます。製品ごとの範囲とクレート以外の表記は [THIRD_PARTY.md](../THIRD_PARTY.md) にあります。

## CPU の速さをまとめて測る

Linux で `tools/bench-all.sh --runs 5 --threads 4` を実行すると、既存の Rust と C# の合成・ブラシ・面のベンチを同じ回数・並列上限・CPU 割当で順に測り、
`target/bench-all/summary.md` に比較表、同じ場所にログと実行条件を保存する。Python 3.10 以降・taskset・上記の C# 用の Unity 同梱ツールが必要。
`--source DIR`（または `YOLUPAINTER_UNITY_SOURCE`）で Unity 版の場所（0.4.x までの `Runtime/Core` を持つ checkout。上の注意と同じ）、`--only blur` などで M2 ブラシの種類を絞れる（合成・通常ブラシ・面は常に測る）。
合成・ブラシは予熱 2 回を除き、面は予熱なしの中央値。フィルターはレベル補正・ブラシのぼかし／指先を含む。GPU と独立フィルター全種は対象外。

## 画素の計算の SIMD

x86_64 では、合成（Normal チャンネルを含む）・調整レイヤー・フィルターの画素の計算に AVX2（と FMA）・SSE4.1 を使い、実行時に CPU が持つ一番広い道を選ぶ
（Windows の配布物も同じ）。aarch64（Apple Silicon の Mac など）では NEON（f64 が 2 本・f32 が 4 本）を使う。NEON は aarch64 のどの CPU にもあるので、実行時の判定はしない。
それ以外の CPU は、今までの画素ごとの計算を使う。結果のバイトはどの道でも同じで、試験が道ごとに画素ごとの式と比べる。
環境変数 `YOLU_SIMD`（x86_64 は `scalar`・`sse41`・`avx2`、aarch64 は `scalar`・`neon`）で狭い道へ下げられる（CPU が持たない広い道には上げない。別の CPU の道の名前は知らない値として無視する）。
2D の合成（矩形・タイルの束・歩幅つきの粗い合成・グループの出力）はこの行の核で重ねる。参照の `composite_pixel`（画素ごとの式）とバイトが同じで、試験が全モード・マスク・クリッピング・グループ・調整レイヤー・Normal チャンネルを道ごとに比べる。
表示に寄与するレイヤー・複数のレイヤー・グループの結合も、タイルの合成で焼く（下のレイヤーへ結合する `merge_down` は、下のレイヤーの画素を下地にする方法があるので画素ごとの式のまま）。

`cargo run --release -p yolu-core --example simd_bench [blend|adjust|filter|kernel|all] [回数]` が、合成モード・調整の種類・フィルターごとの時間
（1 タイルと 4096²。`kernel` は行の核だけの ns/画素）を測る。スレッドは `SIMD_THREADS`（既定 1）、名前の絞り込みは `SIMD_FILTER`（カンマ区切り）。
Mac（Apple Silicon）の NEON の効きは、同じコマンドを `YOLU_SIMD=scalar` と既定（NEON）で回して比べる。出力の先頭の道の名前で、どの道で測ったかが分かる。
歩幅つきの粗い合成（ドラッグの間の仮の絵）は `cargo run --release -p yolu-core --example coarse_bench [回数]` で、辺 × レイヤー数 × 歩幅 × スレッド数ごとの 1 回の時間を測る
（組は `COARSE_SCENE`・`COARSE_SIZE`・`COARSE_LAYERS`・`COARSE_STRIDE`、束の枚数は `COARSE_CHUNK`）。

2D のブラシのダブの画素（丸・筆先の画像・紙の質感・デュアル・指先・ぼかし・クローン・色の混ぜ・ダブごとの色）も同じ道で、行ごとにレーンで描く。
参照は画素ごとの式（`YOLU_SIMD=scalar`）で、試験が乱数で振ったブラシ・レイヤー・タイルの大きさ・点の列を道ごとに描いて、レイヤーの全バイトとダブの数を比べる。
選択範囲・透明部分のロックは、色を塗る・消すだけのブラシなら行の核、画素ごとの色・効果のブラシでは画素ごとの式。ステンシル・乗算でない紙の質感・3D の面のダブも画素ごとの式のまま。ダブをタイルごとにワーカーで描くかは、
外接の箱の大きさに画素ごとの時間の見積もり（ブラシの種類で決まる）を掛けて決める。速いブラシの大きなダブは、ワーカーを起こす費用が勝つので直列で描く。

`cargo run --release -p yolu-core --example stroke_bench -- --threads 1` が、ブラシの種類 × 大きさ × 間隔ごとに、4096² の文書への 1 ストロークの時間を測る
（`--threads 1` は CPU 時間、2 以上は壁時計。`--features stroke-profile` を付けると段ごとの時間も出る）。ペンの入力が画面に出るまでの遅れは
`cargo run --release -p yolu-app --example stroke_latency`、取り込んだブラシは `cargo run --release -p yolu-io --example stroke_bench_imported -- --bundled 6` で測る。

3D ビューのダブは、`cargo run --release -p yolu-core --features stroke-profile --example surface_dab_bench -- --threads 8` が、合成の球（3,888 と 202,800 三角形）・文書 2048² と 4096²・ブラシの大きさ 64・128・256 ごとに、速いストロークの
ダブ 1 つの時間（中央・最大）と内訳（投影の塗り・区画の作り直し・覆いの集め・文書へ塗る）、同じ文書・大きさの 2D のダブとの比べ、1 フレームに塗る時間の枠（`--budget`、ミリ秒）で塗ったときのフレームの時間と、離した後の残りを塗り切るまでを測る。

## 機能ごとの試験と実装の置き場

使う人向けの文書から、試験のコマンド・確かめた範囲・実装の置き場をここへ集めています。

### 復旧（docs/RECOVERY.md）の試験

```text
cargo test -p yolu-io --test projects generation::   世代の置き場（量の数え方・書く前の空きの確かめ）
cargo test -p yolu-app --lib recovery::        上限の消し方・空きの守り・設定
cargo test -p yolu-app --test headless recovery::   書き置きと上限・空きの守り・ウィンドウの数え直し
cargo test -p yolu-app --test gui_shell recovery_ui::   ウィンドウ（使う量・詳しく・使用中）
```

空きの守りは、試験では空きを偽って確かめます。実際のディスクを満杯にした試験はしていません。Windows の `GetDiskFreeSpaceExW` の呼び出しは、`x86_64-pc-windows-gnu` 向けにコンパイルが通ることまでで、実機では確かめていません。

### ブラシの取り込みの確かめた範囲（docs/BRUSH_IMPORT.md）

- 合成の `.sut` で確かめたこと: 設定の写し、画像（プレビュー・平らな PNG・埋め込まれた PNG）の取り出しと被覆率、複数の筆先の順、名前で決まらないときの並び、
  質感、影響元（筆圧の最小値と曲線、傾き・速さ・ランダム、読めない・大きすぎる BLOB）、旗の注記、表の欠け、`VariantID` の列の欠け、ノード・素材の数の上限、
  筆先の一部が欠けたときの知らせ、並びで当てる条件（素材と参照の数がちょうど同じ）と推定の知らせ、筆先 256 枚の打ち切り、命令の数と時間の上限
  （上限を小さくして踏む試験と、実際の上限で断られる走査の多いファイル）、SQL からの `load_extension()` の拒否、画素の予算、
  ビュー・仮想表の拒否、読まない生成列の不評価、WAL の印、約 32 万通りの壊し方（全バイトの書き換え・切り詰め・極端な値）と、壊し方では届かない
  形の正しい極端なファイル（参照が上限いっぱい・素材が上限など）。
  Windows 向け（GNU）のビルドを Wine で動かした試験も通っています（実機の Windows では確かめていません）。
- 合成の `.sut` で確かめたこと（入り抜き・傾き・筆先の向き・手ぶれ補正。`tests/brushes/brush_import_sut.rs`）: 入り抜きの長さが画素で線の補助へ写ること・片側だけ・長さ 0・上限への丸め・
  影響先（種類・影響先の旗・BLOB の形）や単位や長さが合わないときは写さずに知らせること、傾きが大きさ・不透明度・流量の切り替えへ写ること（直線は注記なし・曲線の形は
  近似の注記・上がる曲線と太さと速さの併用は写さない・曲線の枠が無い／長さが合わないヘッダーは写さない）、筆圧の曲線を枠で選ぶこと（傾きの曲線を筆圧に取り違えない）、
  筆先の角度の巻き方・角度のランダムの旗と強さ・向きの筆圧/傾き/速さの注記、手ぶれ補正の段階の写しと範囲、整数の列が常に百分率であること（硬さの 1 は 1%）、写せた項目の並びと重ならないこと。
- 「CLIP STUDIO から」のウィンドウの試験（`yolu-app/tests/headless/brush_import.rs`・`yolu-app/tests/gui_canvas/brush_import_ui.rs`・`yolu-io/tests/brushes/brush_clipstudio.rs`）: 一覧（名前・筆先の見本・読めなかった行は選べない）、選んだものだけの取り込み、
  見つからない理由（日英）、手で選んだフォルダと探し直し、探している間の取り消し、取り込み中は取り込まないこと、CLIP STUDIO のフォルダの「更新時刻・大きさ・中身・項目の数が変わらないこと」、
  入り抜き・手ぶれ補正を持つブラシの重ね方・戻し方・複製・追加・保存と読み戻し（版 4）。
- 合成の C2F で確かめたこと（`tests/brushes/brush_import_sut_layer.rs`）: 原寸の筆先・質感（被覆率・向き・端の欠けたタイル・画素の無いタイル・長いオーバーフロー・UTF-8 と UTF-16LE・`Parameter`
  の終わりの語の数の違い）、元の画像と描画用の画像の優先、プレビューより C2F、塊の壊れ（印・CRC・切れ・知らない塊・塊の数）、読めない側のページ数のずれ、内部ページの輪・存在しない子、
  フォルダでないレイヤーの数、色の面・大きすぎる画像・空の面、zlib の壊れ、塊の CRC を計算し直したうえでの約 3400 通りの壊し方と切り詰め（ファイル全体は必ず取り込めること）、
  上限の内側と外側（B 木の深さ 8 の葉は読めて 9 は断る・1 行 60 MiB は読めて 64 MiB を名乗る行は断る〔小さなファイルが大きな行を名乗る形〕・行の合計が 256 MiB を超える・
  256 素材を並べても取り込み全体の仕事の上限で打ち切られる・2048×2048 の素材を並べても画素の合計の上限で打ち切られる）、同じ名前で根の違う表（同じ根の写しは読める）、
  UTF-8 と UTF-16LE の混在、タイルの塊の壊れ（番号の重複・数の不足と過剰・範囲の外・大きさの欄・長さの欄・zlib の展開の長さ・終わりの印・塊の全長）と、それぞれの断りの理由、
  素材の種類ごとの当て方（デュアルブラシ込みの数が合うとき・合わないとき・種類の分からない素材が混ざるとき）。壊れた C2F では丸い筆先に戻り、プレビューがあればそれを使う。
- 本物の `.sut` 7 つでの確かめ（試験の外。ファイルは配布の許諾が分からないのでリポジトリに入れていない。環境変数 `YOLU_REAL_SUT_DIR` のフォルダを読む `#[ignore]` の試験
  `tests/brushes/brush_import_sut_real.rs`。ブラシの名前・ファイル名は出さず、写した値と注記の種類だけを出す）: 列のスキーマ（整数の列が百分率）、影響元の BLOB 176 件の形（ヘッダー 44 バイトと
  曲線の枠）、写した設定（入り抜き 5 つ〔長さ 20 と 20 が 4 つ、1130 と 14 が 1 つ〕・傾き（大きさ）4 つ・手ぶれ補正 1 つ〔段階 6〕・筆先の角度 2 つ〔180°・220°〕・角度のランダム 3 つ〔強さ 30・80・100〕・硬さ 1% 1 つ）、フォルダとして一覧にして覗いても更新時刻・大きさが変わらないこと（7 個を 100 ms ほどで一覧にして覗いた）。
- 本物の `.sut` 1 つでの確かめ（試験の外。ファイルは配布の許諾が分からないのでリポジトリに入れていない。環境変数 `YOLU_REAL_SUT` のパスを読む `#[ignore]` の試験）:
  素材 4 つの画像が原寸で読めること（筆先 2・質感 2。1 辺は 500〜1000 画素）、ブラシ 1 つの筆先と質感が（種類ごとの並びの推定で）当たり、「独自の形式で読めない」と言われないこと。
  画素の意味は、同じ環境変数の `#[ignore]` の試験 `the_pixels_are_the_black_ink_opacity_the_material_thumbnail_shows`（`src/brushes/sut/c2f.rs`）が、素材の中の CLIP STUDIO の見本の不透明度との
  相関・向き（上下・左右）・値の向き・平均・インクの色で確かめます（見本のある素材 2 つ。数値は上の「画素の意味」）。
- 確かめていないこと: **実機の Windows** で、CLIP STUDIO の入った PC のフォルダが見つかるか・CLIP STUDIO が開いているときに読めるか（上の「サブツールのフォルダから選ぶ」）。
  入り抜きの長さの単位が画素であること・影響先の既定が大きさであること・割合の意味、傾きの曲線の軸と向き、手ぶれ補正の段階の単位、筆先の角度の向きの正負と角度のランダムの強さの意味、
  向きの旗の下の 2 ビット、速さ・ランダムの影響の強さの置き場（実物 7 つはどれも既定に近い値で、ほかの値を見ていない）。
- 確かめていないこと: 本物の `.sut`（CLIP STUDIO の版ごと）での列の名前・単位・曲線の意味、`.sut` を CLIP STUDIO で再び読み込めること（書き出しはしません）。C2F は別の版・
  色の素材（5 面）・表の根が読めない側にある素材では確かめていない（読めなければ断る）。素材の種類ごとの当て方は 1 つのファイルの 4 素材だけに基づく推定。筆先の画素の最大の
  濃さ（見た筆先では 95 / 255）が CLIP STUDIO の描画でそのまま使われるかは確かめていない。質感の「白が塗れる」向き（質感の画素を輝度 255 − v にして乗算する向き）は、
  見本が示すのは画素の意味（黒の不透明度）までで、CLIP STUDIO が質感を塗りにどう使うかは確かめていない（画面の注記は「質感の模様」の推定の知らせだけで、この向きの未確認は知らせない）。見本の無い素材（`CanvasPreview` を持つもの）の画素の意味も確かめていない。

### ツールの登録とキー（docs/SUBTOOLS.md）

ツールの名前・キー・サブツールの種類・ツールプロパティ・オプションバー・キャンバスと 3D ビューの入力は、`crates/yolu-app/src/tools/` のツールの表（`TOOLS`）が持ちます。
キーボードの割り当て・マウスと修飾キーの組み合わせ・ビューの中のキーは `crates/yolu-app/src/keymap.rs` の表が持ち、キーの処理も設定のウィンドウの「ショートカット」の区分も同じ表を読みます。
利用者が設定のウィンドウで変えた所だけは `crates/yolu-app/src/keyconfig.rs` が設定のフォルダの `keymap.json` に持ち、`keymap::install` で表に重ねます。
