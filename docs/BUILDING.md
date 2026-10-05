# ソースからのビルド

[English](en/BUILDING.md)

Rust の stable ツールチェーンと C/C++ のビルド環境が必要です。導入方法は [Rust のインストール手順](https://doc.rust-lang.org/book/ch01-01-installation.html)を参照してください。ソースを取得・展開し、`Cargo.toml` のあるフォルダで次のコマンドを実行します。初回のビルドには依存パッケージを取得するネット接続が必要です。

表示には GPU と対応ドライバーが必要です。描画基盤は [wgpu](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html) で、Windows は Direct3D 12 または Vulkan、Mac は Metal、Linux は Vulkan などを使います。アプリ単体の起動に Unity は必要ありません。

## Windows

64 ビットの Windows と、Visual Studio Build Tools の「C++ によるデスクトップ開発」（Windows SDK を含む）、Rust の MSVC ツールチェーンを用意します。PowerShell で実行します。

```powershell
cargo build --release -p yolu-app --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\yolupainter.exe
```

ペンタブレットはドライバー側でも Windows Ink を有効にしてください。ブラシのツールプロパティのペンのボタンで、どの項目を筆圧で変えるか選べます。ペンの筆圧が強すぎる・弱すぎるときは「表示 → 筆圧の調整…」で直せます。

## Mac（試用）

Xcode Command Line Tools と Rust を用意し、Metal が使える環境で実行します。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

Windows Ink に相当する専用のペン入力処理はありません。マウス操作を基本とし、筆圧の取得は OS と入力機器に依存します。現在の Unity 用ブリッジの作成ツールは Mac 用ライブラリを生成しません。

## Linux（試用）

C/C++ コンパイラー、`pkg-config`、X11 または Wayland のデスクトップ環境、GPU ドライバーを用意します。ファイル選択には D-Bus セッションと `xdg-desktop-portal`、デスクトップに合うポータルのバックエンドが必要です。画面の書体（BIZ UDPGothic）は実行ファイルに含まれるので、システムの日本語フォントは要りません。

Debian・Ubuntu 系でのパッケージ名の例は `build-essential`、`pkg-config`、`libxkbcommon-dev`、`libwayland-dev`、`libvulkan1`、`xdg-desktop-portal`、`xdg-desktop-portal-gtk` です。GPU ドライバーは機器に合うものを使用してください。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

マウスで描画できます。筆圧の取得はデスクトップ環境と入力機器に依存します。
