# ソースからのビルド

[English](en/BUILDING.md)

Rust の stable ツールチェーンと C/C++ のビルド環境が必要です。導入方法は [Rust のインストール手順](https://doc.rust-lang.org/book/ch01-01-installation.html)を参照してください。ソースを取得・展開し、`Cargo.toml` のあるフォルダで次のコマンドを実行します。初回のビルドには依存パッケージを取得するネット接続が必要です。

表示には GPU と対応ドライバーが必要です。描画基盤は [wgpu](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html) で、Windows は Direct3D 12 または Vulkan、Mac は Metal、Linux は Vulkan などを使います。アプリ単体の起動に Unity は必要ありません。

コマンドラインと MCP サーバー（`yolupainter-cli`。[使い方](CLI.md)）も使うときは、同じ命令の `-p yolu-app` を `-p yolu-cli` にしてビルドします（画面のライブラリを使わないので、ビルドは短く済みます）。

## Windows

64 ビットの Windows と、Visual Studio Build Tools の「C++ によるデスクトップ開発」（Windows SDK を含む）、Rust の MSVC ツールチェーンを用意します。PowerShell で実行します。

```powershell
cargo build --release -p yolu-app --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\yolupainter.exe
```

ペンタブレットは、既定では Windows Ink で読むので、ドライバー側でも Windows Ink を有効にしてください（WinTab で読むときは、「編集 → 設定…」の「ペン」の「ペンの入力」で選びます）。ブラシのツールプロパティのペンのボタンで、どの項目を筆圧で変えるか選べます。ペンの筆圧が強すぎる・弱すぎるときは「編集 → 設定…」の「ペン」の「筆圧の調整」で直せます。

## Mac（試用）

Xcode Command Line Tools と Rust を用意し、Metal が使える環境で実行します。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

配る形（Apple Silicon 向けと Intel 向けの両方のコードを持つ universal の `YoluPainter.app` を入れた zip）は、Xcode Command Line Tools（`lipo`・`codesign`・`iconutil`・`sips`・`ditto`・`plutil`）、Python 3.10 以降、2 つの Rust のターゲットを用意して、リポジトリの根で作ります。

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo xtask build --target universal-apple-darwin --release
cargo xtask bundle --target universal-apple-darwin
```

`target/dist/yolupainter-<版>-macos-universal-experimental.zip` ができます。2 つのターゲットでビルドして `lipo` で 1 つにし（最低の macOS は 11.0 に揃えます）、今あるロゴからアイコンを作り、ad-hoc の署名（`codesign -s -`）をして `ditto` で zip にします。Apple の開発者の署名と公証はしません。署名なしの `.app` の開き方は [ダウンロードと更新の「Mac（試作）」](INSTALL.md#mac試作) にあります。

ペンタブレット（Wacom・XP-Pen など）は、ドライバーが macOS の標準のイベントで送る筆圧・傾き・消しゴムの端・サイドボタンを読みます（試し）。メーカーごとの SDK は使いません。おかしいときは「編集 → 設定…」の「ペン」の「タブレットの筆圧（試し）」を切ると、ペンはマウスと同じに描きます。ペンの筆圧が強すぎる・弱すぎるときは「編集 → 設定…」の「ペン」の「筆圧の調整」で直せます。

## Linux（試用）

C/C++ コンパイラー、`pkg-config`、X11 のデスクトップ環境、GPU ドライバーを用意します。ウィンドウは X11 で開くので、Wayland のデスクトップでは XWayland の上で動きます（XWayland の無い Wayland だけの環境では起動できません）。ファイル選択には D-Bus セッションと `xdg-desktop-portal`、デスクトップに合うポータルのバックエンドが必要です。確認のウィンドウ（はい・いいえ。保存していない変更を捨てるかなど）には `zenity` が要ります（無いと、その確かめが要る操作は取りやめになります）。画面のフォント（BIZ UDPGothic）は実行ファイルに含まれるので、システムの日本語フォントは要りません。

Debian・Ubuntu 系でのパッケージ名の例は `build-essential`、`pkg-config`、`libxkbcommon-dev`、`libwayland-dev`（Wayland の部品もビルドに含まれるので、ビルドには要ります）、`libvulkan1`、`xdg-desktop-portal`、`xdg-desktop-portal-gtk`、`zenity` です。GPU ドライバーは機器に合うものを使用してください。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

マウスで描画できます。筆圧の取得はデスクトップ環境と入力機器に依存します。
