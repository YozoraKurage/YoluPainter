# YoluPainter-rs

YoluPainter のスタンドアロン版です。2D キャンバスと 3D モデルにテクスチャを描き、レイヤーを含む作業を `.ylp` に保存できます。Unity 版 YoluPainter と Live Link でつなぐと、描いた色を Unity のシーンで実際のマテリアルに重ねて確認できます。

Windows を主な対象としています。Mac・Linux は試用向けです。

## できること

- Color（色）のラスター層に、丸いブラシと消しゴムで描画。直径・硬さ・間隔・不透明度・流量を調整でき、Windows Ink の筆圧を直径・不透明度・流量に割り当てられます。
- レイヤーの追加・削除・名前変更・並べ替え、表示・不透明度・26 種類の合成モードの変更、取り消しとやり直し。
- 2D 表示の拡大縮小・移動・回転・左右反転。表示だけを変え、保存する画像は回転・反転しません。
- 3D ビューの試しの立方体、または Live Link で受け取ったモデルに描画。マテリアルごとにテクスチャセットを切り替えられます。
- `.ylp` の新規作成・読み込み・保存。Unity 版との互換性は[下の表](#unity-版との-ylp-の受け渡し)を参照してください。

現在、画面から扱える描画チャンネルは Color です。PSD・PNG の取り込み／書き出し、モデルファイルの直接読み込み、グループ・マスク・調整層の編集は画面から利用できません。アセット欄は準備中です。

## 動く環境とソースからの起動

Rust の stable ツールチェーンと C/C++ のビルド環境が必要です。導入方法は [Rust のインストール手順](https://doc.rust-lang.org/book/ch01-01-installation.html)を参照してください。ソースを取得・展開し、`Cargo.toml` のあるフォルダで次のコマンドを実行します。初回のビルドには依存パッケージを取得するネット接続が必要です。

表示には GPU と対応ドライバーが必要です。描画基盤は [wgpu](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html) で、Windows は Direct3D 12 または Vulkan、Mac は Metal、Linux は Vulkan などを使います。アプリ単体の起動に Unity は必要ありません。

### Windows

64 ビットの Windows と、Visual Studio Build Tools の「C++ によるデスクトップ開発」（Windows SDK を含む）、Rust の MSVC ツールチェーンを用意します。PowerShell で実行します。

```powershell
cargo build --release -p yolu-app --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\yolupainter.exe
```

ペンタブレットはドライバー側でも Windows Ink を有効にしてください。ブラシ設定のペンのボタンで、どの項目を筆圧で変えるか選べます。

### Mac（試用）

Xcode Command Line Tools と Rust を用意し、Metal が使える環境で実行します。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

Windows Ink に相当する専用のペン入力処理はありません。マウス操作を基本とし、筆圧の取得は OS と入力機器に依存します。現在の Unity 用ブリッジの作成ツールは Mac 用ライブラリを生成しません。

### Linux（試用）

C/C++ コンパイラー、`pkg-config`、X11 または Wayland のデスクトップ環境、GPU ドライバーを用意します。ファイル選択には D-Bus セッションと `xdg-desktop-portal`、デスクトップに合うポータルのバックエンドが必要です。日本語表示には Noto Sans CJK を使用します。

Debian・Ubuntu 系でのパッケージ名の例は `build-essential`、`pkg-config`、`libxkbcommon-dev`、`libwayland-dev`、`libvulkan1`、`xdg-desktop-portal`、`xdg-desktop-portal-gtk`、`fonts-noto-cjk` です。GPU ドライバーは機器に合うものを使用してください。

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

マウスで描画できます。筆圧の取得はデスクトップ環境と入力機器に依存します。

## 最初の操作

起動すると、2048 × 2048 のキャンバスと試しの立方体が表示されます。カラー欄で色を選び、2D または 3D ビューを左ドラッグすると描けます。レイヤー欄で描く層を選び、必要に応じて「レイヤー」メニューから新しい層を追加してください。

| 操作 | 入力 |
|---|---|
| ブラシ／消しゴム | `B` / `E` |
| ブラシを小さく／大きく | `[` / `]` |
| 取り消し／やり直し | `Ctrl+Z` / `Ctrl+Shift+Z` または `Ctrl+Y` |
| 描いているストロークの取消 | `Esc` |
| 2D の拡大縮小 | ホイール |
| 2D の移動 | 中ボタンドラッグ、または `Space`＋左ドラッグ |
| 2D の回転 | `R`＋左ドラッグ、または `Shift`＋中ボタンドラッグ |
| 2D の回転を戻す／左右反転 | `Shift+R` / `H` |
| 2D を画面に合わせる | `Ctrl+0` |
| 3D の回転 | 右ドラッグ、または `Alt`＋左ドラッグ |
| 3D の移動／拡大縮小 | 中ボタンドラッグ／ホイール |
| 新規／開く／保存／別名で保存 | `Ctrl+N` / `Ctrl+O` / `Ctrl+S` / `Ctrl+Shift+S` |

Mac では表の `Ctrl` を `Command` に読み替えてください。メニューの表示は `Ctrl` です。`.ylp` はウィンドウへドラッグ＆ドロップしても開けます。モデル全体を見失ったら「表示 → 3D ビューでモデル全体を見る」、パネルを戻すには「表示 → パネルの並びを戻す」を使います。

## Unity との Live Link

同じ PC で動く Unity エディターと接続します。Unity 2022.3 のプロジェクトに、**Live Link 対応の Unity 版 YoluPainter（VPM パッケージ `net.yozolab.yolupainter`）と OS に合うネイティブブリッジ**が必要です。`YozoLab → YoluPainter → Live Link` がない版では、この接続は利用できません。Windows 用は `yolu_bridge.dll`、Linux 用は `libyolu_bridge.so` です。

1. スタンドアロンを起動し、「ファイル → Live Link」を有効にします。下の状態欄に Unity を待っていることが表示されます。
2. Unity で `YozoLab → YoluPainter → Live Link` を開きます。接続名（`Link name`）を `yolupainter-livelink` にして `Connect` を押します。
3. シーンのモデルのゲームオブジェクトを指定します。選択中のものなら `Use selection` を押し、`Send model` で送ります。
4. スタンドアロンのテクスチャセット欄でマテリアルを選び、2D または 3D ビューで描きます。表示中で、Unity 側に Color の反映先があるセットの合成結果が Unity に送られます。
5. 作業を `.ylp` に保存します。接続を終えるには Unity の `Disconnect`、またはスタンドアロンの「ファイル → Live Link」を使います。

Unity では lilToon・Standard などのマテリアルに描いた色を一時表示します。元のテクスチャやマテリアルのアセットへ書き込む操作ではありません。切断すると一時表示を外します。Live Link は作業ファイルの保存を代行しないため、スタンドアロン側で保存してください。

接続できる Unity は一度に 1 つです。版の不一致や待ち受け失敗は状態欄で確認できます。ブリッジを更新したときは Unity を再起動してください。接続名を変える場合は、スタンドアロン起動前に環境変数 `YOLUPAINTER_LINK_NAME` を設定し、Unity 側にも同じ名前を指定します。

## Unity 版との .ylp の受け渡し

| 内容 | スタンドアロンでの扱い |
|---|---|
| `.ylp` 形式 1〜7、内部の文書形式 1〜21 | 読み込み。保存時は `.ylp` 形式 7 に更新 |
| Color のラスター層、名前・表示・不透明度・26 合成モード・クリッピング | 編集と保存。文書・レイヤーの ID、透明画素の RGB を保持 |
| マスク、グループ、塗りつぶし、調整、ロック、別チャンネル、パス、フィルター、Generator、Anchor など | 含むセットを読み取り専用にし、理由を表示。元の文書を保持 |
| 読み取り専用セットの見た目 | 保存済みの Color 合成画像を表示。画像がない・読めない場合は空の表示と理由を提示 |
| 編集していないセット、埋め込みリソース、未知の追加エントリ | 元のデータを保持して保存 |
| より新しい形式や未知の値、壊れたファイル | 読み込みを拒否して理由を表示 |

Unity へ戻すときは、形式 7 と文書形式 21 を読める Unity 版で開いてください。古い Unity 版へ戻すための形式への変換はありません。読み取り専用セットは合成画像に置き換えて保存するのではなく、元の編集情報を残します。ただし、保存済み画像は最新の効果を再計算したものとは限りません。

編集したセットは文書形式 21 と Color の合成画像で保存します。Undo の履歴は保存しません。`.ylp` 全体の ZIP バイト列や、ウィンドウ配置・モデルの接続状態まで両アプリで同じになることは保証しません。

上書き時の直前の版は、同じフォルダの `<ファイル名>-backups~/` に残ります。自動削除はしません。読み込み後に別のアプリがファイルを書き換えた場合は、上書きを断ります。同じファイルを両アプリで同時編集せず、保存してからもう一方で開き直してください。

## 許諾の一覧と配布用の全文

使用しているライブラリ・フォント・アイコンの許諾は [THIRD_PARTY.md](THIRD_PARTY.md) を参照してください。現在の一覧は Windows GNU 向けで、Mac・Linux・Windows MSVC の一覧を兼ねません。配布用の許諾全文の生成方法は[開発用の手順](docs/DEVELOPMENT.md#配布用の許諾全文)にあります。

試験や Unity ブリッジのビルドについては [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) を参照してください。
