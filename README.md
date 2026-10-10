[English](README.en.md)

# YoluPainter

[![CI](https://github.com/YozoraKurage/YoluPainter/actions/workflows/main-tested.yml/badge.svg?branch=main)](https://github.com/YozoraKurage/YoluPainter/actions/workflows/main-tested.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

YoluPainter は、2D のキャンバスと 3D のモデルにテクスチャを描くペイントアプリです。
Unity とつなぐと、シーンのモデルとマテリアルの値を受け取って開き、描いた絵を Unity のマテリアルに当てられます。

Windows を主な対象にしています。Mac と Linux は試用向けです。

## 主な機能

- 2D と 3D のペイント。筆圧と傾きに応えるブラシ、色の混ぜ、ぼかし・指先・クローン、対称、定規、ステンシル
- ペイント・編集・ポーズのモード。3D ビューで投影の箱・デカール・点・3D のパス・ポーズのボーンを選んで動かせます
- キー・マウスの組み合わせとパイメニューの割り当てを、設定のウィンドウの「ショートカット」の区分で変えられます
- レイヤー、グループ、マスク、クリッピング、26 の合成モード、調整レイヤー、テキストレイヤー、ロック
- Color・Roughness・Metallic・Height・Normal・Emission とユーザーチャンネルを 1 回のストロークでまとめて描くマテリアルのペイント
- フィルター、焼いたメッシュマップ（AO（アンビエントオクルージョン）・曲率・厚み・ID など）を読む Generator、ノイズとグランジ、スマートマテリアル
- 3D ビューでの lilToon の見た目
- Unity との Live Link。シーンのモデルを 1 回の操作で開き、書き出した絵を Unity へ返します（マテリアルに当てるのは、Unity で確かめたときだけ）
- コマンドライン（`yolupainter-cli`）と、MCP でつなぐ AI のアシスタントからの、レイヤー・マスク・効果の操作
- PSD のレイヤー・グループ・マスク・調整の読み書き、ABR のブラシと CLIP STUDIO のブラシ（.sut）の取り込み
- チャンネルごとの PNG と、Unity Standard・URP・HDRP・lilToon 向けのテンプレートの書き出し
- 落ちたときの自動の復旧

## ダウンロード

Windows（64 ビット）のインストーラーと zip は [Releases](https://github.com/YozoraKurage/YoluPainter/releases) にあります。
インストールの選択肢と更新は [docs/INSTALL.md](docs/INSTALL.md) を見てください。

Mac には、試作の zip（`yolupainter-<版>-macos-universal-experimental.zip`）も付きます。Apple Silicon 向けと Intel 向けの両方のコードを持つ（universal の）`YoluPainter.app` が入っています。
Apple の開発者の署名と公証はなく（署名は ad-hoc だけ）、アプリは自分では更新しません。新しい版は、ヘルプのメニューにリリースのページを開く項目が出て知らせます。
Apple Silicon の Mac（macOS 27）で確かめたのは、起動、設定とログの書き込み、署名、universal の形までです。ウィンドウの表示と描画は確かめていません。Intel の Mac では動かしていません。

署名がないので、zip を展開した `YoluPainter.app` を初めて開くと macOS が止めます。次の順で開きます（[Apple の手引き](https://support.apple.com/ja-jp/guide/mac-help/mh40616/mac)）。

1. `YoluPainter.app` をダブルクリックします。止められるので、表示を閉じます。
2. アップルメニュー →「システム設定」を開き、サイドバーの「プライバシーとセキュリティ」を押します。
3. 「セキュリティ」の項目に移動して、「開く」を押します。
4. 「このまま開く」を押します（アプリを開こうとしたあと、約 1 時間だけ使えます）。
5. ログインパスワードを入力して、「OK」を押します。以後は、ダブルクリックで開きます。

## ソースからのビルド

Rust の stable と C/C++ のビルド環境が要ります。

```sh
cargo build --release -p yolu-app --locked
```

OS ごとの準備は [docs/BUILDING.md](docs/BUILDING.md)、試験は [docs/DEVELOPMENT.md](https://github.com/YozoraKurage/YoluPainter/blob/main/docs/DEVELOPMENT.md) にあります。

## 文書

- [使い方](docs/GUIDE.md)
- [Unity との連携](docs/UNITY.md)（Live Link）
- [コマンドラインと AI からの操作](docs/CLI.md)（`yolupainter-cli`・[MCP でつなぐ](docs/MCP.md)）
- [PSD](docs/PSD.md)、[ブラシ](docs/BRUSH.md)、[ブラシの取り込み](docs/BRUSH_IMPORT.md)、[サブツール](docs/SUBTOOLS.md)、[グラデーションマップ](docs/GRADIENT_MAP.md)、[3D ビュー](docs/PREVIEW.md)、[復旧](docs/RECOVERY.md)、[配布用に保存](docs/SAVE_FOR_DISTRIBUTION.md)、[.ylp の形式](docs/YLP_FORMAT.md)
- [変更の記録](https://github.com/YozoraKurage/YoluPainter/blob/main/CHANGELOG.md)

## お問い合わせ

お問い合わせ、開発に関する質問、機能の要望は [Discord](https://discord.gg/c8NNfhJ94J) へどうぞ。

## プライバシー

更新の確認を選んだときに GitHub へ最新の版を問い合わせるほかは、ネットワークへ情報を送りません。Live Link は同じ PC のフォルダでファイルを受け渡します。外からの操作は、設定で入れている間だけ `127.0.0.1`（この PC の中）で待ちます。

## 許諾

[MIT License](LICENSE)。使っているライブラリ・フォント・アイコンの許諾は [THIRD_PARTY.md](THIRD_PARTY.md) にあり、配布物には許諾の全文を同梱しています。
