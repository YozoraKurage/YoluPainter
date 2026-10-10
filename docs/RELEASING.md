# 配布の手順

Windows x86_64 MSVC の zip とインストーラー（NSIS の setup.exe）を出します。Linux x86_64 の tar.gz も作れますが、下の許諾の判断が済むまでは配りません
（`release.yml` の入力 `linux` は切が既定です）。macOS は試作として、署名なし（ad-hoc の署名だけ）の `YoluPainter.app` の zip を出します
（[macOS の試作](#macos-の試作)。入力 `macos` は入が既定で、切にすると載せません。試作なので、ビルドが落ちても配布は止まらず、Windows だけで出します。[macOS（試作）が落ちたとき](#macos試作が落ちたとき)）。AppImage は対象外です。
実行ファイル・インストーラーのコード署名は、[SignPath Foundation への申し込み](#コード署名signpath-foundation)が通るまで付けません（署名なしで出します）。
ビルドには Rust stable、Python 3.10 以降、各 OS の C/C++ ビルド環境が必要です。Windows のインストーラーには NSIS 3 も要ります。macOS の .app は macOS の上でだけ作れます（Xcode Command Line Tools の `lipo`・`codesign` など）。Linux の実行環境は [BUILDING.md の「Linux（試用）」](BUILDING.md#linux試用)を参照してください。
Ubuntu 22.04 で作るため、これより古い glibc 環境での動作は保証しません。

Linux の既存依存 `wayland-protocols-plasma` と `wayland-protocols-misc` の protocol XML には LGPL-2.1-or-later の表記があり、
`tools/licenses-reviewed.json` の `blocked` に記録しています。条件を確認するか依存構成を変更するまで、Linux の `bundle` は停止し、
入力 `linux` を入にした配布も進みません。`blocked` を外すのは、生成バインディングへの条件の適用を確認した後だけです。

依存を追加・更新して `licenses-reviewed.json` へ承認を追加するときは、クレートの宣言だけでなく、同梱する XML・ソース全体に
GPL/LGPL の表記が無いかを調べます。ヒットしたら、選ばないデュアルライセンスの側（例: `self_cell` の GPL）かを確かめ、そうでなければ `blocked` に理由を書きます。

```sh
grep -rIlE 'SPDX-License-Identifier:.*GPL|GNU (Lesser|Library) General Public' ~/.cargo/registry/src/*/<クレート>-<版>
```

許諾の原文は、クレートに同梱の物（`path`）か、そのクレートの発行時コミットの上流の物（`url`。コミット固定）を SHA-256 で固定します。クレートにも上流にも許諾の本文が無いときは、
上流の記載（`Cargo.toml` の `license` と `authors`）から組み立てた文を `tools/license-texts/` に置いて固定します（`repo`。この下のファイルだけ）。文の頭に、上流の原文ではなく、
どの記載から組み立てたかを書きます。年は、上流の記載で分かる物だけ書きます（今の文は、どれも年を書いていません）。上流の LICENSE.md など、本文ではないが許諾について書いてある物は、文のあとに並べて固定します。

## 出す流れ

1. 版を上げるリリース（`x.y.0`）では、[出す前の文書の確かめ](#出す前の文書の確かめ)を済ませ、直した文書を作業の枝に入れます。
2. 版を上げ（[版と配布物](#版と配布物)）、main 向けの PR を作ります。PR の CI が試験を回し、配る物もビルドします（[配る物をビルドする場所と、受け取る道](#配る物をビルドする場所と受け取る道)）。
3. Actions の「配布物の作成」を、作業の枝から dry-run=true（リリースの下書きを作らない試し運転）で動かし、配る物を確かめます。
4. PR の CI が全部成功したら、PR を main にマージします。
5. main から「配布物の作成」を dry-run=false で動かします。environment の承認のあとに、リリースの下書きができます。
6. 下書きを確かめ、本文を書いて公開します。

各段の細かい手順は[GitHub の設定と起動](#github-の設定と起動)にあります。

## 出す前の文書の確かめ

版を上げるリリース（`0.6.0` のように末尾が 0 の版）では、main 向けの PR を作る前（配布のワークフローを試し運転で回す前）に、
公開している文書を全部、その版の変更の記録・画面の文字・コードと照らし、古い手順と抜けを直して、同じ PR に入れます。
使う人向けの文書は配る物（zip・インストーラー）にもそのまま入る（[版と配布物](#版と配布物)）ので、出した版の配布物の中の文書は、あとから直せません。

確かめる文書:

- スタンドアロン（このリポジトリ）: `README.md`・`README.en.md`、`docs/` の日本語と `docs/en/`、`CHANGELOG.md`、コードの中の README（`crates/` と `tools/` の下の `README.md`・`smart.README.md`）、
  `plugin/` の `SKILL.md`、許諾の表記（`THIRD_PARTY.md`・`crates/` の下の `THIRD-PARTY-NOTICES.md`）
- Unity ブリッジ（YoluPainter-UnityBridge）: `README.md`・`Editor/LiveLink/README.md`・`licence.md`・`Documentation~/THIRD_PARTY.md`。スタンドアロンと同じ版で出すので、一緒に確かめます

確かめ方（1 つの文書ごとに、頭から）:

1. 確かめられる主張に印を付けます: ファイル・型・モジュールの名前と置き場、コマンドと引数、ワークフローの入力とジョブの名前、数・既定値・版、画面の名前と操作の順。
2. 1 つずつ実物と照らします: 画面の文字（`crates/yolu-app/src` の `lang.pick(日, 英)`・メニュー・ツールチップ）、コードと試験、`.github/workflows/`・`crates/xtask`・`tools/`、
   その版の `CHANGELOG.md`、コミットの本文（`git log <前の版のタグ>..HEAD -- <道>`）。
3. 古い所・誤り・抜け（その版で追加した機能の説明が無い所）を直します。実物で確かめられなかった事は書きません。
4. 言葉を画面にそろえます（レイヤー・ツール・ウィンドウ・アイランド・フォント・キャンバス。ビルドのことは「ビルド」と書きます）。
5. 試験で見張れる所は回します: `cargo test -p xtask`（配る文書の一覧と、入れる文書の相対リンクが配布物の中で切れないこと）、
   `cargo test -p yolu-io --test ylp format_doc`（`docs/YLP_FORMAT.md` の版とエントリが実装と合うこと）、許諾を変えたら `cargo xtask preflight --only licenses`。

## 版と配布物

`Cargo.toml` の workspace.package.version を更新し、`Cargo.lock` を更新・コミットしてから、そのコミットを配布対象にします。
`Cargo.lock` が変わると（版だけの更新でも）ハッシュが替わるので、`python3 tools/third-party.py` で製品別の一覧を作り直し、`THIRD_PARTY.md` の「Cargo.lock SHA-256」
（製品別の一覧ごとと、Cargo.lock 全体の節）を出力に合わせます。クレートの件数が替わったときは、件数の表も直します。
AI のアシスタント向けのプラグイン（`plugin/.claude-plugin/plugin.json`・`plugin/.codex-plugin/plugin.json`）の `version` も同じ版にします（`yolu-mcp` の試験 `tests/plugin.rs` が食い違いを断ります）。
プラグインは Release に載せず、`main` の `plugin/` と根の `.claude-plugin/marketplace.json` から配ります（Claude Code はこの `version` が変わったときに新しい版を入れます）。
版は SemVer です。試験版（kind=prerelease）は `0.1.0-rc.1` のように、プレリリース識別子を `alpha.N`・`beta.N`・`rc.N`（N は整数）の 1 つにします。
この形にすると、`rc.2` < `rc.10` < 正式版の順に並び（`rc10` のようにつなげると辞書順で `rc10` < `rc2` になるので認めません）、アプリの「試験版を使う」の設定が見つけられます
（[試験版の置き場](#試験版の置き場)）。stable は識別子を含む版を使えません。`cargo xtask preflight --kind prerelease` が版の形を確かめます。

```sh
cargo xtask build --target x86_64-pc-windows-msvc --release
cargo xtask bundle --target x86_64-pc-windows-msvc
cargo xtask installer --target x86_64-pc-windows-msvc
cargo xtask symbols --target x86_64-pc-windows-msvc   # Windows だけ。PDB の付属物（下の「PDB の付属物」）
cargo xtask mcpb --target x86_64-pc-windows-msvc      # Windows だけ。Claude Desktop に入れる拡張（下の「Claude Desktop の拡張（.mcpb）」）
# Linux では target を x86_64-unknown-linux-gnu に替える（installer は Windows だけ）
# macOS（macOS の上で）は target を universal-apple-darwin にする: build が Apple Silicon と Intel の 2 つをビルドして lipo で 1 つにし、bundle が .app を作って zip にする
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist
```

`bundle` と `installer` は直前に同じコミットからビルドした release 実行ファイルを使います。古いビルドを使わないでください。
`bundle` と `installer` は `target/dist` に過去の版を残すので、手元で出すときは `target/dist` を空にしてから作ります。
違う版や余分なファイルが残っていると `updater-json` が拒否します（PDB の付属物 `yolupainter-<版>-x86_64-pc-windows-msvc-pdb.zip` の名前だけは例外。CI は毎回まっさらです）。
出力は `target/dist/yolupainter-<版>-<target>.zip`（または `.tar.gz`。macOS は `yolupainter-<版>-macos-universal-experimental.zip`）と、Windows の `yolupainter-<版>-x86_64-pc-windows-msvc-setup.exe`、`symbols` を走らせたときの PDB の付属物、`mcpb` を走らせたときの `.mcpb`。
実行ファイル（アプリの `yolupainter` と、コマンドラインと MCP サーバーの `yolupainter-cli`。`build` が同じ命令でビルドします）、LICENSE、操作・動作環境を含む README（日英）、THIRD_PARTY.md、対象別の DEPENDENCIES.md と許諾全文、使う人向けの `docs/`（`docs/en/` を含む）を
同梱します（インストーラーも同じ物を入れます）。文書は配布物の中でもフォルダつきの `docs/GUIDE.md`・`docs/en/GUIDE.md` の名前で入るので、README からの相対のリンクがそのまま効きます。
開発の手順（`docs/DEVELOPMENT.md`・`docs/RELEASING.md`）は入れません。入れる物の一覧は `crates/xtask/src/main.rs` の `BUNDLED_DOCS` と `LEFT_OUT_DOCS` の 1 か所で、
`docs/` に追加したファイルは、そのどちらかへ必ず載せます（載せ忘れると `bundle`・`installer` が止まり、`cargo test -p xtask` も落ちます）。
入れる物は `.md` の文書だけで、画像など `.md` でない物は `LEFT_OUT_DOCS` へ載せて入れません（更新で前の版にだけあった文書を消すインストーラーの掃除が `.md` だけを対象にするため。入れる必要が出たら、`installer/yolupainter.nsi` の `RemoveOldDocs` も直します。試験が断ります）。
入れる文書の相対のリンクが配布物の中で切れないことも試験が確かめるので、入れない物（`docs/DEVELOPMENT.md`・`CHANGELOG.md`・`crates/` の README など）へは GitHub の URL で張ります。
インストーラーのスクリプト `installer/yolupainter.nsi` の `DocFiles` にも同じ一覧があり、試験が突き合わせます（文書を追加したら両方に追加します）。
`tools/third-party.py` が対象ごとに許諾を照合し、未確認の依存や原文の不一致では束ねません。
更新クレート（`yolu-update`）はアプリに組み込まれ、コマンドライン（`yolu-cli`）は同じ配布物に入るので、その依存の許諾全文も含めます（`--include-update --include-cli`）。
`xtask` 自体は配りません。独自の `CARGO_TARGET_DIR` は使わず、出力を `target/` に揃えてください。

`updater-json` の入力は、その版の配布物だけを置いた専用フォルダです。違う版や余分なファイルは拒否します
（PDB の付属物は、名前が完全一致の 1 つだけ許し、更新情報には載せません）。
Windows の zip があるのにインストーラーが無い版も拒否します（インストーラーで入れたアプリは、更新にインストーラーを使うので、並べて出します）。
更新情報（schema 1）には、zip・tar.gz を対象の三つ組（`x86_64-pc-windows-msvc` など）で、インストーラーを別の鍵 `x86_64-pc-windows-msvc-setup` で、macOS の zip を鍵 `universal-apple-darwin` で載せます。
ファイル名は `updater-v1.json`（schema の番号入り。[更新情報の互換](#更新情報の互換)を参照）。
URL は `https://github.com/YozoraKurage/YoluPainter/releases/download/v<版>/<配布物名>` に固定です。アプリが更新情報を取る場所は
`https://github.com/YozoraKurage/YoluPainter/releases/latest/download/updater-v1.json`（GitHub の「最新の Release」。下書き・プレリリースは含みません）です。
設定「試験版を使う」を入れたアプリは、これに加えて固定のタグの置き場（`releases/download/updater-beta/updater-v1.json`）も見ます。
フォークから配る場合は、更新クレートの `RELEASE_BASE`・`UPDATER_URL` も変更してビルドします。
署名なしの JSON は検査専用で、更新クレートは受理しません。

### macOS の試作

`cargo xtask build --target universal-apple-darwin --release` が、最初に前回の出力（2 つのターゲットと universal の実行ファイル）を消してから、`aarch64-apple-darwin` と `x86_64-apple-darwin` で
同じ命令（`-p yolu-app -p yolu-cli`）をビルドし、`lipo` で 1 つにします（`target/universal-apple-darwin/release/`。途中で失敗しても前回の物が残らず、`bundle` が古い実行ファイルを新しい版の zip にしません。
両方のスライスの最低の macOS は `MACOSX_DEPLOYMENT_TARGET=11.0` に揃えます）。`cargo xtask bundle --target universal-apple-darwin` が、
`YoluPainter.app/Contents/{Info.plist,MacOS,Resources}` を作り、`codesign --force --deep --sign - YoluPainter.app`（ad-hoc）で署名し、`ditto -c -k --keepParent --norsrc --noextattr --noqtn` で zip にします
（`crates/xtask/src/macos.rs`。リソースフォーク・拡張属性・検疫の印は zip に入れません）。`--identifier` は付けません: 束の識別子は `Info.plist` の `net.yozolab.yolupainter`、
束の中のコマンドラインにはファイル名に中身のハッシュを足した `yolupainter-cli-…` が付きます（`--identifier` を付けると、`--deep` で両方が同じ識別子になります）。

- 配布物は `yolupainter-<版>-macos-universal-experimental.zip`。最上位のフォルダ `yolupainter-<版>-macos-universal-experimental/` に、`YoluPainter.app`、Windows・Linux の配布物と同じ文書と許諾の表記
  （`LICENSE`・README・`THIRD_PARTY.md`・`DEPENDENCIES.md`・`THIRD_PARTY_LICENSES.txt`・`docs/`。一覧は `BUNDLED_DOCS`・`ROOT_FILES` の 1 か所）が入ります。
  コマンドラインの `yolupainter-cli` は `.app` の `Contents/MacOS/` の中です（`--deep` の署名が及ぶ）。
- `Info.plist` の版はワークスペースの版。表示の版 `CFBundleShortVersionString` は `x.y.z` の整数だけ（試験版の識別子は入れません）。ビルド番号 `CFBundleVersion` は、正式版は `x.y.z`、
  試験版は識別子の番号を足した `x.y.z.N`（`0.6.1-rc.2` → `0.6.1.2`）。試験版どうし・正式版と区別するための形で、新旧の並びは表しません（版の前後は更新情報の SemVer が決めます）。束の識別子は `net.yozolab.yolupainter`。
  アイコンは今あるロゴ（`crates/yolu-app/assets/logo/yolupainter-1024.png`）から `sips`・`iconutil` で作ります（新しい絵は作りません）。`.ylp` の関連付け（`CFBundleDocumentTypes`）は載せません
  （macOS が開く要求を渡すのは Apple Event で、アプリは引数だけを読むため）。
- 許諾は `tools/third-party.py --target universal-apple-darwin`（2 つの Rust のターゲットの木の和）。`cargo xtask preflight --target aarch64-apple-darwin --only licenses,attributes` のように、
  どちらのターゲット単独でも照合できます。
- Apple Developer の署名・公証・.dmg・Homebrew は、まだありません。ダウンロードした zip の `.app` は Gatekeeper に止められるので、開き方を README と Release の本文に書きます。
  アプリは更新を落とさず入れ替えず、新しい版を知らせてリリースのページを開くだけです（[検証と現在の範囲](#検証と現在の範囲)）。
- 確かめるのは、zip を別の場所へ展開して `codesign -dv`・`codesign --verify --deep --strict`・`lipo -info`・実行ファイルの起動（数秒動き続け、設定やログのファイルを書くこと）と、検疫の印（`com.apple.quarantine`）が付いた状態で起きること。
  画面は SSH では見えないので、Mac の実機で開いて、ウィンドウが出て描けることを版ごとに 1 回確かめてください（確かめるまで、README・`docs/INSTALL.md`・CHANGELOG・リリースの本文は「起動・書き込み・署名・universal の形まで。ウィンドウの表示と描画は未確認」の範囲で書きます）。
- 試作なので、macOS のビルドが落ちても配布は止まりません（[macOS（試作）が落ちたとき](#macos試作が落ちたとき)）。

### macOS（試作）が落ちたとき

macOS の対象は `tools/dist-targets.json` の `experimental: true` です。落ちても 0.6.0 の配布を止めません。ほかの対象（Windows）は今までどおり必須で、1 つでも欠ければ止まります
（`cargo xtask preflight` の `targets` が、macOS は試作・Windows は試作でないことを見ます）。

- PR の CI: `dist-build.yml` の build ジョブが `continue-on-error: ${{ matrix.experimental || false }}` なので、macOS のジョブが落ちても CI の結論は失敗になりません（そのジョブは赤く見えます）。
  時間の上限は対象の `timeout_minutes`（macOS は 90 分。ほかは 60 分）。
- `main-tested.yml`: `python3 tools/dist.py ci-ok` が、試作の対象のジョブ（名前に `（universal-apple-darwin）` を含む）だけが落ちた CI を成功とみなします。ほかのジョブが 1 つでも落ちていれば、今までどおり失敗です。
- 配布の受け取り: `judge_run` は試作の対象のジョブの失敗を数えず、`find` は macOS の成果物が無くても（落ちた・期限切れ・目録が合わない）Windows の成果物を受け取ります。macOS は載りません。
- 配布の中のビルド: macOS だけが落ちても `metadata` は進みます。`install` が、試作でない対象の成果物が欠けていれば止め、試作の対象の成果物が無ければ載せずに、実行の Summary に
  「universal-apple-darwin（試作）は載らなかった」と理由を出します。下書きは Windows だけで作られます。
- 載らなかった版は、更新情報に macOS の項目が無く（macOS のアプリは「この環境向けの配布物がありません」と答えます）、リリースの本文にも macOS の節を貼りません。
- macOS を外すとき: `tools/dist-targets.json` の macOS の対象の `default` を外すコミット（PR の CI でビルドしなくなります）。その配布だけ外すなら、入力 `macos` を切にします。

確かめること（GitHub の設定で、リポジトリの外なのでここでは確かめられません）: main の ruleset の必須のステータスチェックに、macOS のジョブ（名前に `universal-apple-darwin` を含む「配る物のビルド」）が入っていないこと。
入っていると、macOS が落ちた PR はマージできません。入れないのが決めです。

### PDB の付属物

クラッシュの記録は各フレームの番地と実行ファイルの基底の番地（`Image base`）を書くので、配布物の PDB があれば、同じ版の関数名・行へ引けます。
`.github/workflows/dist-build.yml` の Windows のビルドは、`cargo xtask build` の前に `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`・`CARGO_PROFILE_RELEASE_STRIP=none` を環境に置き
（命令は変えず、行番号つきの PDB を実行ファイルとは別に作る）、`cargo xtask installer` のあとに `cargo xtask symbols` で `yolupainter.pdb` だけを入れた
`yolupainter-<版>-x86_64-pc-windows-msvc-pdb.zip` を `target/dist` に作ります。Release の付属物としては載りますが、zip・インストーラーには入らず、
更新の対象ではありません（署名つきの更新情報に載せず、アプリは取りに行きません）。`verify` は、通常のファイルで、空でなく、大きさの上限内で、中身が `yolupainter.pdb` だけであることを見ます。
`symbols` は PDB を調べる前に前回の付属物を消すので、ビルドし直しに失敗したあとで古い PDB が残りません。

### Claude Desktop の拡張（.mcpb）

`cargo xtask mcpb --target x86_64-pc-windows-msvc` が、`target/dist/yolupainter-<版>-x86_64-pc-windows-msvc.mcpb` を作ります（`build` のあとに。`yolupainter-cli.exe` を使います）。
中身は zip で、`manifest.json`（mcpb の manifest_version 0.3。`server.type` は `binary`、実行ファイルは `server/yolupainter-cli.exe`、引数は `mcp --port ${user_config.port}`（拡張の設定「Port」。既定は 17347）、
対象は `win32` だけ。実行ファイルは標準入出力を起動中のアプリの受け口へつなぐ中継で、ツールの一覧は書かず `tools_generated` にして、アプリが `tools/list` で返す物を正本にします）・アイコン（アプリのロゴの PNG）・`LICENSE`・コマンドラインの許諾の全文（`DEPENDENCIES.md`・`THIRD_PARTY_LICENSES.txt`。
`tools/third-party.py --package yolu-cli --built-with yolu-app --bundle`。実行ファイルはアプリと 1 回のビルドで作り機能が合わさるので、そのビルドでの `yolu-cli` の部分木で数えます）・実行ファイル 1 つです。ロゴを替えるときは `crates/xtask/src/main.rs` の `MCPB_LOGO` を替えます。

PDB の付属物と同じく、Release に載せるだけで、更新の対象ではありません。署名つきの更新情報には載せず（アプリは取りに行かず、インストールした拡張は、新しい `.mcpb` を入れ直して替えます）、
`updater-json` はこの名前（版と対象が今の版と一致する 1 つ）だけを知って読み飛ばし、`verify` は中身の形（一覧のファイルだけで、manifest が今の版・実行ファイルを指す）を見ます。
`dist-build.yml` の Windows のビルドが `xtask mcpb` を走らせ、`target/dist/*` ごと成果物・Release の付属物になります。
manifest は公式の検証（`npx @anthropic-ai/mcpb validate manifest.json`）が通ることを確かめています。Windows の実機では、アプリの設定「外からの操作を受ける」を入れてから Claude Desktop に入れて、ツールが並び、アプリの文書を操作できることを、版ごとに 1 回確かめてください。

### Windows のインストーラー

NSIS のスクリプトは `installer/yolupainter.nsi` です。利用者ごとのインストール（`%LOCALAPPDATA%\Programs\YoluPainter`、管理者権限なし、64 ビット）で、
スタートメニューのショートカットとアンインストーラー（「設定 → アプリ」に登録）を置きます。`.ylp` の関連付けは、画面では選べます。

| 引数 | 動き |
|---|---|
| `/S` | 画面を出さない |
| `/ASSOC=1` / `/ASSOC=0` | `.ylp` の関連付けを付ける・付けない（無音のとき。省略は今の状態のまま、初めてなら付けない） |
| `/RUN` | 入れ終わったらアプリを起こす（アプリの更新が使います）。待ちの上限で何も変えずに終わるときも、今入っているアプリを起こし直します |
| `/D=<パス>` | 入れ先（最後に置く。省略は前の入れ先） |
| アンインストーラーの `/S` `/DELETEDATA` | 無音。アプリが作り直せるデータ（設定・ウィンドウの配置・復旧・クラッシュの記録・サムネイルのキャッシュ。`%APPDATA%\YoluPainter` と `%LOCALAPPDATA%\YoluPainter` の名指しした物）は、画面では消すかを聞き、無音では `/DELETEDATA` のときだけ消す。個人のライブラリ・ブラシ・サブツール・グラデーション・カラーセット・表示のプリセットなど利用者が作った物と、知らないファイルは、どちらでも残す（表は `docs/INSTALL.md`） |

実行中のアプリは終了させません。実行ファイル（アプリと `yolupainter-cli.exe`。MCP のクライアントが動かし続けていることがあります）が使われている間は待ち（無音は 60 秒まで。超えたら何も変えずに終了コード 5。`/RUN` が付いていれば今入っているアプリの実行ファイルを起こし直します）、
アンインストーラーは自分が入れたファイルだけを消します（入れ先に利用者のファイルがあれば、入れ先のフォルダは残ります）。

文書は入れ先の `docs\`・`docs\en\` に入ります。入れた文書の名前は `docs\.installed` に記録し、更新（上書き）のとき、前の版の記録にある文書を先に消してから今の版の文書を入れるので、
前の版にだけあった文書（名前を変えた・外した文書）が残りません。消すのは記録にある `docs\` の下の `.md` だけで、`..` を含む名前は読み飛ばします
（利用者が `docs\` に置いたファイルと、入れ先の外は消えません）。アンインストールは一覧の文書と記録を消し、`docs\en`・`docs\` は空のときだけ消します（`RMDir /r` は使いません）。

NSIS は 3.x が要ります。ワークフローは、windows-latest のイメージに `makensis` が無ければ Chocolatey（`choco install nsis --version=3.11.0`）で入れ、
入っていた物でも入れた物でも `makensis /VERSION` が 3.11 でなければ止めます（配布物の作り方を変えないため）。確かめた `makensis` は
`MAKENSIS` で `cargo xtask installer` へ渡します。手元では Windows は `winget install NSIS.NSIS`、Debian・Ubuntu は
`sudo apt install nsis` で入れます（`MAKENSIS` に `makensis` の場所を指定することもできます）。
`cargo test -p xtask` は、`makensis` があればスクリプトを実際にコンパイルします（CI の Linux は `nsis` を入れて走らせます）。

画面を出さない流れ（新規・更新・`/RUN`・関連付けの保持・アンインストール。文書の入れ方と、更新で前の版にだけあった文書が消えること・利用者のファイルが消えないことを含む）と、待ちの上限（書き込みで開けない実行ファイルが残っている間は待ち、
上限を超えたら何も変えずに終了コード 5。`/RUN` の有無で今の実行ファイルを起こし直すかも確かめます。試験用に上限を 3 秒へ縮めたインストーラー `-DWAIT_STEPS=6` を使います）は、Wine で通せます。

```sh
python3 tools/test-installer.py
```

実際に動いているアプリを待つ動きは、Wine が動いている実行ファイルの上書きを断る版でだけ確かめられます（上書きできる版では、注意を出して通ります）。
ページの並び・チェック・確認のウィンドウなどの画面は Wine では確かめません。Windows の実機で、インストール・更新・動いているアプリがある間の更新・アンインストールを 1 回ずつ通してください。

### exe のバージョン情報

`crates/yolu-app/build.rs` が、製品名 `YoluPainter`・版（workspace の版。プレリリース識別子つき）・作者名・アイコン
（`crates/yolu-app/assets/logo/yolupainter.ico`）を実行ファイルへ埋めます。インストーラーも同じ製品名・版を持ちます
（コード署名の条件の「製品名と版のメタデータ」）。

### 配るビルドの profile

配るビルドは既定の release（`cargo build --release`）です。`lto = "fat"`・`codegen-units = 1`（パニックは unwind のまま）の profile も測りましたが、採りませんでした。
再測は `python3 tools/bench-profiles.py --app --cold`（その profile の定義はツールが `--config` で与えます。Cargo.toml には入れていません）。

測った範囲（2026-10-05。AMD Ryzen 9 7950X3D の Linux のコンテナ、rustc 1.99.0、cargo の並列 8・sccache なし・インクリメンタルなし、負荷平均 2〜7。
操作の時間は 1 つの CPU に固定・rayon 1 スレッドで、5 回の中央値を 3 回（profile を交互に）まわした中央値）:

| | release | lto fat・codegen-units 1 |
|---|---:|---:|
| yolu-app を冷えた状態からビルドする | 106 s | 244 s（2.30 倍） |
| exe の大きさ（Linux） | 68.9 MB | 56.1 MB（0.81 倍） |
| 重い操作の速さの比（release ÷ 右。1 より大きいほど右が速い） | | 合成 0.993・ブラシ 0.997・書き出し 0.999・保存と開く 1.019・PSD 1.022（51 行の幾何平均 0.999。行ごとには 0.93〜1.08） |

重い操作は 5% も速くならず、ビルドする時間は 2.3 倍でした。GitHub の Windows の runner では、2026-10-05 の 0.3.0 の配布で「ビルドと配布物の作成」の段が 9 分 25 秒だったので、
2.3 倍なら 20 分を超える見込みで、配るビルドの時間が長くなりすぎます。得は exe が約 19% 小さくなることだけでした。
測ったのは Linux・1 CPU・yolu-core の合成とブラシと書き出し（`bench`・`export_bench`）と、保存・PSD（`tools/profile-bench/io_bench.rs`）で、GPU・画面・複数スレッド・Windows の速さは測っていません。

## 事前確認

配る物を作る段（許諾の照合・梱包・インストーラー）が、本番で初めて走って止まる種類の失敗を、ビルドせずに並べます。手元でも CI のビルドの最初でも同じ命令です。

```sh
cargo xtask preflight                       # 既定の対象（tools/dist-targets.json の、入力が要らない対象）
cargo xtask preflight --kind stable         # 出す種類も確かめる（stable にプレリリースの版は使えない）
cargo xtask preflight --kind prerelease     # 試験版は alpha.N・beta.N・rc.N の版だけ
cargo xtask preflight --target x86_64-unknown-linux-gnu --offline
cargo xtask preflight --target aarch64-apple-darwin --only licenses,attributes   # macOS の Rust のターゲット単独の許諾の照合（universal-apple-darwin はその和）
cargo xtask preflight --installer           # 加えて、Wine でインストーラーを通す（tools/test-installer.py。wine32 が要る）
cargo xtask preflight --only version --kind prerelease
```

| 確かめ | 見ること |
|---|---|
| `licenses` | 対象ごとの許諾の照合（`tools/third-party.py --target T --bundle` と同じ。未確認の依存・原文の不一致・`blocked` で落ちる。macOS の対象は objc2 系など macOS だけの依存も数える） |
| `attributes` | SHA-256 で照合する表記ファイル（`tools/licenses-reviewed.json` の `bundled` と、リポジトリに置いた許諾の文 `tools/license-texts/`）が、`.gitattributes` で `eol=lf` か `-text` か（Windows の checkout で CRLF になると照合が落ちる） |
| `nsis` | `installer/yolupainter.nsi` の `Target` が、公式の Windows 版 NSIS が持つ stub（`x86-unicode`・`x86-ansi`）か（amd64 の stub は無い） |
| `mcpb` | Windows の対象で、`.mcpb` の manifest が作れて形が合うか、拡張に入れるロゴの PNG があるか、コマンドラインの許諾の束（`tools/third-party.py --package yolu-cli --built-with yolu-app --bundle`）が作れるか |
| `macos` | macOS の .app を作る対象（`universal-apple-darwin`）で、`Info.plist` が組めて版が合うか、アイコンの元のロゴが 1024 x 1024 の PNG か、同梱する文書が一覧に載っているか。macOS の上では、加えて `lipo`・`codesign`・`iconutil`・`sips`・`ditto`・`plutil` と、2 つの Rust のターゲットが入っているか |
| `version` | yolu-app の版のタグ `v<版>` が origin にまだ無いか（`git ls-remote --tags origin`。問い合わせられなければ「飛ばした」と表示する）、`--kind stable` ならプレリリースの版でないか、`--kind prerelease` なら `alpha.N`・`beta.N`・`rc.N` の版か |
| `targets` | `tools/dist-targets.json` の対象が、xtask の配れる対象で、Windows にだけインストーラーがあるか。macOS の対象は macOS の runner で、`rust_targets` が xtask の 2 つのターゲットと同じか |
| `workflows` | ワークフローの YAML が読めるか、release.yml の入力（既定の入・切を含む）と対象の並びが食い違わないか、外の Action が SHA で固定されているか、配る物の手順がキャッシュを使っていないか（`tools/check-workflows.py`。PyYAML が無ければ「飛ばした」） |
| `installer` | Wine でインストーラーを通す（`--installer` か `--only installer` のときだけ） |

失敗は 1 行ずつ理由つきで並べ、全部を回してから終了コード 1 を返します（最初の 1 つで止めません）。「飛ばした」は失敗にしません。

## 配る物をビルドする場所と、受け取る道

配る物（zip・インストーラー）は、**main 向けの PR の CI** がビルドします。配布（`release.yml`）は、同じ木の物があれば**ビルドし直さずに受け取ります**。
同じ内容を PR の CI と配布で 2 度ビルドするのは無駄で、試験を通した物と配る物が別のビルドになるためです。

- `ci.yml` は main への push では動かしません（main は CI を通した PR からしか変わりません）。代わりに `main-tested.yml` が、main の木がマージした PR の先頭の木と同じで、
  その PR の CI が成功しているかだけを確かめます（ビルドし直しません。README の CI の印はこの結果です）。
  PR の CI（`ci.yml`）は、main 向けの、同じリポジトリの枝からの PR のときだけ、試験のジョブと並べて「配る物のビルド」
  （`dist-plan` → `dist`）を走らせます（待ち時間は延びません）。フォークの PR ではビルドしません。
- ビルドの手順は `.github/workflows/dist-build.yml` の 1 つで、`ci.yml` と `release.yml` の両方が呼びます。対象の並びは `tools/dist-targets.json` の 1 か所です
  （Windows は常に。Linux は `release.yml` の入力 `linux` が入のときだけで、CI ではビルドしません。macOS の `universal-apple-darwin` は、入力 `macos` が既定で入なので、PR の CI でもビルドします
  （配布で切にできるのは入力だけ。切にした配布は、macOS を除いた対象で、CI の成果物を受け取るか、ビルドします）。対象を追加したら、この一覧と、必要なら入力を直します。`cargo xtask preflight` の `targets`・`workflows` が食い違いを断ります）。
  macOS の対象は macOS の runner（`macos-14`）でビルドし、一覧の `rust_targets`（Apple Silicon と Intel の 2 つ）を runner へ入れます。手順は 診断用の ID（`tools/dist.py revision`）→ `cargo xtask preflight --target <対象>` → `cargo xtask build --release`（PR の CI と dry-run=false の配布は `--require-update-key` も付け、公開鍵が空なら止めます）
  → `bundle`（macOS は .app の組み立て・署名・zip）→ `installer`（Windows。先に NSIS 3.11 を確かめます）→ `mcpb`・`symbols`（Windows）→ 目録（`tools/dist.py catalog`）→ 成果物のアップロードです。
- 成果物の名前は `dist-<木>-<対象>`（保存 14 日）。木は `git rev-parse HEAD^{tree}` で、PR の CI は merge の commit の木です。中に `catalog.json`
  （木・commit・アプリに埋めた ID の元の commit・版・対象・組み込んだ更新用の公開鍵・各ファイルの SHA-256 と大きさ・rustc・runner）が入ります。
- アプリの版の表示（「0.4.0 · a1b2c3d」）とクラッシュ報告の `Git:` の ID は、PR の CI では**PR の先頭の commit**です。PR の CI がビルドするのは merge の commit
  （`refs/pull/N/merge`）で、main にもタグにも入らず、PR を閉じたあとに参照できる保証が無いためです。昇格の条件で、PR の先頭の commit の木は成果物の木（＝配る参照の木）と同じなので、
  ID は配る物と同じ木の commit を指します。目録の `commit` はビルドした merge の commit、`revision` が ID の元の commit です。配布がビルドする場合は dispatch した commit です。
- `release.yml` は、最初に `plan`（対象と、dispatch した参照の木）と `find`（探す）を走らせます。`find` は、その木の成果物を、次の**全部**を満たす実行の中から探します。
  - `.github/workflows/ci.yml` の `pull_request` の実行で、完了して成功（全部のジョブが `success`。`skipped` も受け取らない。ただし試作の対象（macOS）のジョブの失敗は数えない）、同じリポジトリの枝からの実行
  - その実行の `head_sha`（PR の先頭の commit）の木が、GitHub の API で確かめて、成果物の名前の木と同じ
  - 成果物が期限切れでなく、GitHub が記録した digest があって落とした zip と同じ（digest の記録が無い成果物は確かめようが無いので受け取らない）で、目録の木・対象・組み込んだ公開鍵（いまのリポジトリ変数と同じ）・各ファイルの SHA-256 が合う
  - 全部の対象が同じ 1 つの実行にそろっている（試作の対象（macOS）は、その実行に成果物があって目録が合うときだけ載せ、無ければ載せずに受け取る）
- そろえば `build`（ビルド）を飛ばし、`metadata` が目録の SHA-256 をもう一度確かめて `target/dist` に集めます（取り込む成果物は、`plan` の出力 `pattern` で、今回の対象の一覧の物だけに絞ります。入力 `macos` を切にした配布は、CI に macOS の成果物があっても取り込みません）。そこから先（未署名の更新情報の確認・リリースの下書きの署名と作成）はビルドした場合と同じです。
  そろわなければ（木が違う・期限切れ・成功でない・Linux を追加した・鍵が変わった・API が失敗した）、いつもどおり同じ手順でビルドします。理由は実行の Summary に 1 行ずつ出ます。
  入力 `rebuild` を入にすると、探さずに必ずビルドします。
- 配布のワークフロー（`release.yml`・`dist-build.yml`）はキャッシュを使いません（Actions のキャッシュ・rust-cache・sccache）。汚染されたキャッシュが配る物に入る道を作らないためで、
  `cargo xtask preflight` の `workflows` が確かめます。PR の CI の試験のジョブは今までどおりキャッシュを使います。
- `metadata` は、受け取る道でもビルドする道でも、出す今のタグがまだ無いことと種類・版が合うことを `cargo xtask preflight --only version` で確かめ直します（PR の時点の確認とは別に）。

### 安全の筋と、保証しないこと

筋: 成果物の名前の木と、その実行の PR の先頭の木と、いま配ろうとしている参照の木が同じなら、ワークフローの台本（`.github/workflows/` の中身）もソースも同じです。
つまり成果物は、main に入る台本と同じ台本・同じソースを GitHub の runner が実行してビルドした物です。PR の枝がワークフローを書き換えて、名前だけ main の木を名乗る成果物を作っても、
その実行の先頭の木は書き換えた物の木なので、受け取りません。PR の先頭の木と merge の木がずれる（PR の枝が main に追いついていない）ときは、名前の木と先頭の木が合わず、ビルドし直します
（受け取れる機会を捨てて、確かめを守る側に倒しています）。

保証しないこと:
- 昇格した物に埋まった ID は、Release の対象の commit そのものではありません。PR の先頭の commit で、木は同じですが、main が PR を squash で取り込むと main の履歴には入りません
  （GitHub の `refs/pull/N/head` から引けますが、その保存は GitHub 任せです）。木が同じことは、成果物の名前と目録で確かめます。
- 依存のクレートの取得は `Cargo.lock` の固定（チェックサム）に頼ります。PR の時点と配布の時点で別の取得をしても、同じ中身であることまでは確かめません。
- Rust のツールチェーンは `stable` で、runner のイメージも更新されます。PR の時点と今で版が違い得ます（目録の `rustc`・runner に記録します）。同じ木から同じバイト列になることは保証しません（再現可能なビルドではありません）。
- 更新用の公開鍵はアプリに組み込まれます。PR の後にリポジトリ変数を変えると、目録の鍵と合わず、ビルドし直します。
- PR の CI の試験が通ったことは、PR の時点の runner・ツールチェーンでの結果です。Windows の実機での確認の代わりにはなりません。
- 保存は 14 日です。PR を出してから 14 日以上たって配るときは、ビルドし直します。

手元で受け取りの条件を確かめるには、`python3 tools/test-dist.py`（GitHub の API を偽の窓口に置き換えた試験）を回します。

## 試験版の置き場

試験版（kind=prerelease の Release）は、`releases/latest` に出ません。設定「試験版を使う」を入れたアプリが見つけられるように、
公開した試験版の署名つきの更新情報を、**固定のタグ `updater-beta` の Release** に原本のまま上書きで置きます。

- アプリ（`yolu-update` の `UpdateClient::check_channels`）は、設定が入のとき、stable の置き場（`releases/latest/download/updater-v1.json`）と
  試験版の置き場（`releases/download/updater-beta/updater-v1.json`）を両方取り、**今の版より新しい方**を勧めます。どちらも同じ鍵・同じ検証です
  （署名・形式・版・対象・大きさ・SHA-256）。同じ版が両方にあれば stable を勧め、版を下げる更新は勧めません。
- 切のときは stable の置き場だけを見ます。stable の更新情報に試験版の版が載っていても受けません。試験版を入れていた人が設定を切っても、
  次の stable が今の版より新しくなるまでは何も勧めません（試験版から正式版へ戻るのは、そのとき）。
- stable が出れば `releases/latest` が切り替わります。置き場は何もしなくてよく、試験版を使う人にも、試験版より新しい stable が見えます。
- 確かめの途中で設定を切ると、その確かめは試験版の結果だけを捨て、stable の結果で答えます（今の版より新しい stable があれば勧めます。「最新」と言うのは stable も新しくないときだけ）。
  ダウンロード中の試験版は取り消し、転送が済んだあとの確かめ・書き込みの間に切った場合も受けません。落とし済みの試験版は、置き場のインストーラーも消します。
- 失敗の理由（通信できない・検証を通らない）は stable の取得で決めます。試験版の置き場が引けないこと（まだ 1 つも出していない間はいつもそうです）は、理由に混ぜません。
- 試験版の置き場が引けない（まだ 1 つも出していない間も含む）・署名が合わない・対象の配布物が無いときは、stable の結果だけで答えます。
  逆に stable が引けないときは、新しい試験版が見つかればそれを勧め、見つからなければ「最新」とは答えずに失敗を伝えます。

### 置き場の決め方の比べ

| 候補 | 認証なしで引けるか | 回数の上限 | キャッシュ | 壊れにくさ | 署名との関係 | 公開の順・運用 |
|---|---|---|---|---|---|---|
| **固定のタグの Release の資産**（採用） | 引ける（stable の道と同じ種類の URL） | 無い（Web の配布の道で、API の回数制限の外） | 最初の応答は `no-cache`（stable の道で確かめた）。置き換えは資産の削除と再登録なので、古い写しが返る見込みは低い（置き換えの反映の遅れは測っていない） | アプリが決めた更新情報の形をそのまま返す。GitHub の API の JSON の形に依存しない | 原本の署名つきの更新情報。stable と同じ鍵・同じ形で、アプリは追加の仕組み無しに検証できる | 公開（published）のときに 1 回だけ上書き。リリースの下書きのうちは置き場が動かない。置き場の Release を 1 つ持つ |
| GitHub の API で最新の prerelease を引く（`releases`） | 引ける（認証なし） | 認証なしは 1 時間 60 回・IP ごと。共有の回線の利用者がまとめて止まる | API の応答に依存 | Release の本文などを含む大きな JSON の形・ページ送り・並び（作成日順で stable と混ざる）に依存する | API の応答は署名の外。結局、更新情報を取る 2 回目の通信が要る | 下書きは認証が無いと見えない。置き場の更新の手間は要らない |
| `releases.atom`（フィード）を読む | 引ける | 無い | フィードのキャッシュに依存 | XML の解析と依存の追加。項目から prerelease かを確実に見分けられる保証が無い | 署名の外の指し示しで、2 回目の通信が要る | 置き場の更新の手間は要らない |
| リポジトリのファイル（`main` の raw）に置く | 引ける | 無い | 数分のキャッシュ | ファイル名は固定 | 署名つきの更新情報を置ける | 保護された `main` へワークフローが push することになり、PR 必須の運用と合わない |

採った理由は、認証と API の回数に頼らず、署名つきの更新情報を原本のまま（再署名なし・秘密鍵に触れずに）置けて、アプリの検証が stable と 1 つで済むことです。
失うものは、置き場の Release が 1 つ増えること（Releases の一覧に prerelease として出ます）と、置き換えの間の数秒に取得が失敗し得ることです
（そのときの確かめは stable の結果で答えるので、更新が止まることはありません）。

### 試験版を出す手順

1. 作業ブランチで `Cargo.toml` の workspace.package.version を `0.4.0-rc.1` のように `alpha.N`・`beta.N`・`rc.N` の版にし、`Cargo.lock` を更新してコミットし、main 向けの PR を出します
   （CI が配る物をビルドします。`cargo xtask preflight --kind prerelease` で版の形も確かめられます）。
2. Actions の「配布物の作成」を kind=prerelease で、まず作業の枝から dry-run=true の試し運転をします。PR を main にマージしたあと、main から dry-run=false（environment の承認。`main` からだけ動かせます）で動かすと、
   リリースの下書きが prerelease の印つきで出ます。
3. 実機の確認のあとに下書きを公開します。**公開すると `beta-channel.yml` が動き**、公開した Release の配布物をすべて取り、公開鍵だけで署名・版・大きさ・SHA-256・梱包の中身を確かめ直して
   （`cargo xtask beta-channel`）、`updater-beta` の Release の `updater-v1.json` を上書きします（初めてなら Release を作ります）。
   置き場の Release は毎回 prerelease・「最新の Release」にしない設定へ付け直すので、stable の `releases/latest` は動きません。
4. 「試験版を使う」を入れたアプリが、次の更新の確かめで新しい試験版を見つけます。入れていない人には何も変わりません。

`xtask beta-channel` の規則:
- 版は `alpha.N`・`beta.N`・`rc.N` の形だけ（正式版や別の識別子の版は置き場に載せません）。
- 置き場の今の更新情報の版が今の版以上なら、置き換えません（古い試験版を後から公開しても、置き場が戻りません）。今の置き場が読めなければ、直すつもりで置きます。
  この比べは戻し防止の目安で、署名の確かめではありません（アプリが署名を確かめます）。
- ワークフローは、置き場の Release がまだ無いとき（`release not found`）だけ新しく作ります。取得の失敗など、ほかの失敗は止めます（置き場の版を見ずに上書きしないため）。
- 失敗したときは、Actions の「Re-run」で同じ公開に対してやり直せます。置き場の Release を手で消したり、prerelease の印を外したりしないでください
  （外すと `releases/latest` が置き場を指します。次の試験版の公開で付け直されます）。

保証しないこと: 置き場の更新情報は署名つきの過去の物の再送を防げません（stable と同じ）。試験版の置き場への反映は、公開のあと、ワークフローが終わるまで（数分）かかります。
反映の遅れ（資産の置き換えから、取得する側に新しい中身が返るまで）は測っていません。

### 以前の版との違いと互換

- 失った能力: 以前は、試験版の版で動いているアプリが、stable の更新情報に載った試験版の版も自動で受けていました（今の版がプレリリースなら受ける、という判定）。
  今は設定「試験版を使う」が入のときだけ受けます。stable の更新情報は preflight が試験版の版を断るので、公式の手順で出した物では起こらない場面ですが、
  手元で作って stable の置き場に載せた試験版は、設定が切のアプリには届かなくなりました。
- 設定ファイル: 試験版を入にすると `update.conf` に `use_beta=on` の行が増えます（切のときは旧い版と同じ 1 行だけで、ファイルが無かった人には作りません）。
  0.3.x はこのファイルを「1 行だけの形」で読むので、行が増えたファイルは読めない選択として扱い、初回の問い（起動時に確かめるか）をもう一度出します。
  試験版の設定そのものは 0.3.x に無いので、戻したアプリは stable だけを見ます。切にしてから戻せば、旧い 1 行のままです。
- 配布のワークフロー: kind の既定は prerelease のままですが、prerelease は試験版の形の版しか受け付けなくなりました。正式版の形の版を出すときは kind=stable を選びます。

## 更新情報の互換

更新クレートは、署名が通った更新情報のうち知らない target の配布物を読み飛ばし、自分の target の配布物だけを厳密に確かめます。
macOS など対象を追加した版の更新情報を、すでに配った版が拒否することはありません。インストーラーも、知らない鍵の配布物として旧いクライアントが読み飛ばします。配布物の数の上限は固定値（`MAX_ASSETS`）です。

次の変更は schema を上げる必要があります。既存のクライアントは schema が違う更新情報を拒否し、そのまま古い版に残ります。

- 本文の項目の追加・削除・意味の変更
- 既知の target の配布物の項目の追加・削除・意味の変更

schema を上げるときは、旧 schema を読むクライアントが取得する更新情報を旧形式のまま残します。
アプリが取る URL（`UPDATER_URL`）はファイル名に schema の番号を含みます。新しい schema は新しい名前（`updater-v2.json`）に置き、
旧い `updater-v1.json` も同じ Release に置き続けてください（`UPDATER_FILE`・`UPDATER_URL`・`UPDATER_SCHEMA` を一緒に変えます）。

## 更新署名の鍵（初回だけ）

配布の管理者が安全な手元の環境で実行してください。リポジトリの外に、アクセスを制限した保管先を用意します。

```sh
cargo xtask keygen --output /secure/location/update-private-key.hex
```

秘密鍵は 32 バイトの seed を hex にしたファイルです。既存ファイルは上書きしません。
Unix では作成権限を 0600 にします。Windows では保管フォルダの ACL を管理者本人に限定してください。
標準出力には公開鍵だけが表示されます。秘密鍵は Git、ログ、配布物に入れず、紛失に備えて安全にバックアップします。
公開鍵は 16 進の 64 文字で、配布のビルドがアプリへ組み込みます。控えを失ったときは秘密鍵ファイルから取り出せます。

```sh
cargo xtask pubkey --key-file /secure/location/update-private-key.hex
```

アプリは、ビルド時の環境変数 `YOLUPAINTER_UPDATE_PUBLIC_KEY` に入れた公開鍵だけを信じ（`yolu_update::embedded_public_key`）、
公開鍵を組み込んでいないビルド（ソースからの手元のビルドなど）は、ヘルプのメニューの更新の項目も初回の問いも出さず、通信もしません。
`cargo xtask build` はこの環境変数を検査し、空の値（GitHub の未設定の変数）は未設定として扱い、入っているのに鍵として使えない値
（長さ・形式が違う、弱い鍵）はビルドを止めます。リリースの下書きを作る配布では `--require-update-key` を付けて、未設定も止めます
（更新できないアプリを配らないため）。更新サーバーから公開鍵を受け取る設計にはしないでください。
鍵の変更は既存アプリの信頼する公開鍵も変える必要があり、自動の鍵更新は未対応です。

手元で署名する場合は次のように実行します。秘密鍵を引数そのものに書かないでください。

```sh
cargo xtask updater-json --version 0.1.0-rc.1 --assets target/dist --sign --key-file /secure/location/update-private-key.hex
```

`--key-file` を省くと環境変数 `YOLUPAINTER_UPDATE_PRIVATE_KEY` を読みます。
署名したら、公開鍵だけで、アプリと同じ検証（署名・版・大きさ・SHA-256）を通るかを確かめます。
別の鍵で署名した更新情報、配布物との食い違い、載っているのに無い配布物はここで失敗します。
zip・tar.gz は中身も開いて、梱包の一覧（上の文書を含む）と照らします。足りないファイルも、一覧に無いファイルも、同じ名前の重複も、名前を並べて断ります
（インストーラーの中は開けないので、一覧どおりの段から作ることと、スクリプトの試験で確かめます）。

```sh
cargo xtask verify --version 0.1.0-rc.1 --assets target/dist --public-key <公開鍵の hex>
```

署名は Ed25519、対象は JSON envelope の `payload` 文字列の UTF-8 バイトです。
`signature` は 64 バイトの署名の hex。payload を解析・再整形してから検証しないでください。
本文には schema=1、version、target/name/url/sha256/size を持つ assets が入ります。

## GitHub の設定と起動

1. リポジトリに environment `release` を作り、承認者を決め、利用できるブランチを `main`、タグを `v*` に制限します。
2. **environment の secret** `YOLUPAINTER_UPDATE_PRIVATE_KEY` に秘密鍵ファイルの内容を保存します。リポジトリ全体の secret には置きません。
3. リポジトリの変数（Variables）`YOLUPAINTER_UPDATE_PUBLIC_KEY` に公開鍵の hex を保存します。公開鍵は秘密ではありません。
   配る物のビルドが、この値をアプリへ組み込みます（main 向けの PR の CI のビルドと、dry-run=false の配布のビルドでは、空だと止まります。成果物の目録にも載り、変数を変えると、PR の CI の成果物は受け取らずビルドし直します）。
   draft ジョブは署名の直後にこの公開鍵で `verify` を実行し、通らなければリリースの下書きを作りません。
4. Actions の「配布物の作成」を開き、kind を prerelease または stable にします。
   版が `0.4.0-rc.1` のような試験版なら prerelease、`0.4.0` のようにプレリリース識別子のない正式版の形なら **stable** を選びます。
   kind の既定は prerelease なので、正式版の形の版を既定のまま動かすと preflight の version で止まります（止まるのはリリースの下書きを作る前です）。
   動かす枝は、dry-run=true の試し運転なら配布対象の枝を選べます。**dry-run=false は `main` からだけ**です（リリースの下書きのジョブが environment `release` を使い、1. の制限に合わない枝からは environment に断られます）。
   なので本番は、PR を main にマージして CI の確かめが済んでから動かします。
5. 最初は **dry-run=true（既定。リリースの下書きを作らない試し運転）** で実行します。同じ木の物が main 向けの PR の CI にあれば受け取り（[配る物をビルドする場所と、受け取る道](#配る物をビルドする場所と受け取る道)）、無ければ Windows と macOS（`linux` が入なら Linux も。`macos` を切にすれば macOS は除く）をビルドし、未署名 JSON を含む `release-preview` artifact を作ります。secret に触れず、Release は作りません。
6. artifact を取得し、ビルドした OS（Windows と macOS。入力 `linux` を入にしたときは Linux も）で展開・起動・同梱文書・許諾全文を確認します。Linux 実行ファイルには実行権限があります。
   Windows はインストーラーで、インストール・起動・更新（前の版のインストーラーで入れた上に入れる）・アンインストールを通します。macOS は zip を展開して `codesign -dv`・`lipo -info`・起動を確かめ、README の手順（「このまま開く」）で開けること、ウィンドウが出て描けることを Mac の実機で 1 回確かめます。
7. PR を main にマージしたあと、main から kind を合わせて dry-run=false で実行し、environment の承認を行います。署名付き JSON とアーカイブを **リリースの下書き（Draft Release）** にアップロードします。
   マージのあとの main の木（ファイルの中身）が PR の先頭の木と同じなら（`main-tested.yml` が確かめます）、PR の CI の成果物を受け取れます。違えば、いつもどおりビルドし直します（[配る物をビルドする場所と、受け取る道](#配る物をビルドする場所と受け取る道)）。
8. 署名と各配布物の SHA-256・サイズはワークフローが公開鍵で確認済みです。下書きのタグと対象コミット、版、prerelease の状態を確認します。
   本文は空で作られるので、実機確認のあと、管理者が本文を書いて手で公開します。macOS の試作が載る版は、署名がないこと・開き方・動作を確かめた環境・自動更新がないことも本文に書きます
   （コード署名を付けるようになったら、README の「Code signing policy」の節へのリンクも本文に入れます。節の準備は[コード署名](#コード署名signpath-foundation)）。
   **公開した時点で `releases/latest` が切り替わり、アプリの更新の確認がその版を見つけ始めます**（stable のみ。prerelease は `latest` に出ません）。
   prerelease を公開すると `.github/workflows/beta-channel.yml` が動き、試験版の置き場を更新します（[試験版の置き場](#試験版の置き場)）。

同時実行は 1 本です。実行中の処理は自動取消しません。既存の同じタグの Release は上書きしません。
失敗後はリリースの下書きとタグの状態を確認してから再実行してください。非公開リポジトリでは認証と Actions の利用枠にも注意してください。
版の形と kind は組になっています。kind=prerelease は `alpha.N`・`beta.N`・`rc.N` の版だけ、kind=stable はプレリリース識別子のない版だけを受け付けます
（`cargo xtask preflight` の version が確かめ、合わなければリリースの下書きを作る前に止まります）。

## コード署名（SignPath Foundation）

署名なしで 0.x を出し、使われ始めたら [SignPath Foundation の無償のコード署名](https://signpath.org/terms.html)に申し込みます。
リポジトリの側でそろえてある条件と、申し込みのときに管理者がすることです。

| 条件 | 状態 |
|---|---|
| OSI の許諾・自作のコードだけ | [MIT](../LICENSE)。依存の許諾は `THIRD_PARTY.md` と許諾の全文で追う |
| 製品名と版のメタデータ | 実行ファイル（`build.rs`）とインストーラー（NSIS）が、製品名 `YoluPainter` と同じ版を持つ |
| 聞かずに通信しない | 更新の確認は初回の問いで「はい」を選んだときと、手で押したときだけ。プライバシーは README の「プライバシー」の節に一文、全文は [INSTALL.md](INSTALL.md) |
| システムの変更の告知・アンインストール | `.ylp` の関連付けは選択肢で、無音では付けない（今の状態のまま）。アンインストーラーは入れたファイルだけを消し、利用者のデータは聞く |
| Code signing policy の表記 | README にこの節は今ありません。申し込みの前に、署名の役割（作者・レビュー・署名の承認）とプライバシーを書いた節を追加します。定型文は、申し込みが通ってから追加します |
| 検証できるビルド・リリースごとの手動の承認 | `配布物の作成` ワークフロー（手で起動・dry-run が既定・リリースの下書きを作るには environment の承認） |

管理者がすること:

1. 最初のリリース（署名なし）を出し、使われ始めるのを待ちます（申し込みは「署名する形で、もうリリースされている」ことが条件です）。
2. README に「Code signing policy」の節（署名の役割とプライバシー）を追加します。
3. GitHub と SignPath の全員で多要素認証を有効にし、SignPath Foundation へ申し込みます。
4. 通ったら、README の「Code signing policy」の節の先頭へ次の定型文を追加し、Release の本文からもこの節へリンクします。

   ```
   Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/)
   ```

5. SignPath 側で、署名する実行ファイルの製品名を `YoluPainter` に、製品の版をビルド内で同じ値に強制する設定（artifact configuration）を作り、
   ワークフローに署名の段階を追加します。署名する物は `yolupainter.exe`（zip とインストーラーに入れる前）とインストーラー本体です。
   署名した実行ファイルを入れた zip・インストーラーを作り直し、更新情報（`updater-json`）の SHA-256 は署名後の配布物から作ります。
   インストーラーが書き出すアンインストーラーの署名は NSIS の `!uninstfinalize` を使う手順になるので、この段階で設計します。
   この署名の段階は、まだワークフローに入っていません。

## 検証と現在の範囲

```sh
cargo test -p yolu-update -p xtask --locked
python3 tools/test-release-tools.py
python3 tools/test-dist.py
cargo xtask preflight
actionlint .github/workflows/*.yml
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

更新クレートは HTTP の口を差し替える形で、署名確認・版比較・明示承認後のダウンロード・サイズと SHA-256 の確認までを提供します。
アプリ側の HTTP 実装（`crates/yolu-app/src/update/http.rs`）は、Windows は WinHTTP（OS の証明書・プロキシ・TLS）、Linux と macOS は `curl` で、
https だけ・時間切れ・読み込み中のサイズ上限・取消を守り、止まらない転送の連なりは失敗にします。転送の回数は、curl は 5 回まで（転送先も https だけ）、
WinHTTP は 5 回に絞る設定を試み、設定できない環境では OS の既定（10 回）まで、で、https から http へは WinHTTP の既定が断ります。
現在の API は同期で、取得データをメモリに保持します。更新情報は 1 MiB、配布物は 2 GiB が上限です。
試験版を提案するのは、設定「試験版を使う」が入っているときだけです（stable の更新情報に試験版の版が載っていても受けません）。同じ版・古い版への更新は提案しません。
署名済みの過去の情報の再送による「新しい版を見せない」攻撃への鮮度保証はありません。
ダウンロードは、利用者が「更新」を押したあとだけ始めます。ダウンロードしたインストーラーは、署名つきの更新情報の SHA-256・大きさで確かめたものを
利用者ごとの置き場（Windows は `%LOCALAPPDATA%\YoluPainter\updates`）へ置き、走らせる直前にもう一度確かめます。
置き場のインストーラーと書きかけは、次のダウンロードと、アプリの起動時（今の版以下のものだけ。走っている最中のものは次の起動で）に片付けます。
アプリ自身は実行ファイルを置き換えません。インストールした Windows はインストーラーを無音で走らせて終了し（インストーラーが終了を待って入れ、`/RUN` で起こし直す）、
zip・Linux・macOS はその版のリリースのページを開きます。実行ファイルの隣に `uninstall.exe` があるかで、インストーラーで入れた物かを見分けます。
macOS は Intel でも Apple Silicon でも同じ鍵 `universal-apple-darwin` の配布物を探し、落とさず、置き場（`updates`）も持たず、入れ替えもしません（`.app` の署名は ad-hoc だけで、自分を差し替えるのは安全でないため）。
更新情報にこの配布物が載っていない版（`macos` を切にして出した版）では、署名は正しいまま「この環境向けの配布物がありません」と答えます。
