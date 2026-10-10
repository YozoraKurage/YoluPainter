# ダウンロードと更新

[English](en/INSTALL.md)

[Releases](https://github.com/YozoraKurage/YoluPainter/releases) から入手します。Windows（64 ビット）には、インストーラーと zip の 2 つの形があります。Claude Desktop 用の拡張（`.mcpb`）も付きます。Mac には試作の zip があります（[下](#mac試作)）。

- `yolupainter-<版>-x86_64-pc-windows-msvc-setup.exe`（インストーラー）: 利用者ごとにインストールします。管理者権限は要りません。入れ先は `%LOCALAPPDATA%\Programs\YoluPainter`（変えられます）で、スタートメニューに登録し、`.ylp` をこのアプリで開く関連付けは選べます。
  - アンインストールは「設定 → アプリ」から行い、設定と復旧のデータも消すかを聞かれます（聞かれなければ残ります。自分で作った物はどちらでも消えません。[下の表](#アンインストールとデータ)）。
  - 画面を出さずに入れるときは `/S`、関連付けは `/ASSOC=1`（付けない `/ASSOC=0`）、入れ終わったあとにアプリを起動するときは `/RUN` を付けます。
  - 画面を出さずに消すときは `/S`、設定と復旧のデータも消すときは `/DELETEDATA` を付けます。
- `yolupainter-<版>-x86_64-pc-windows-msvc.zip`: 展開して `yolupainter.exe` を実行します（インストール不要）。
- `yolupainter-<版>-x86_64-pc-windows-msvc.mcpb`: Claude Desktop に入れる拡張です（[AI のアシスタントから操作する](MCP.md)）。ダブルクリックで入れます。中身は起動中のアプリへの中継だけで、ツールはアプリが答えます（アプリの更新で新しくなります）。Claude Code と Codex はプラグインで入れます。

どちらにも、コマンドラインのプログラム `yolupainter-cli.exe`（[使い方](CLI.md)）と、`README.md` と、この文書を含む `docs` フォルダ（英語は `docs/en`）が入っているので、ネットワークが無くても読めます。

## Mac（試作）

- `yolupainter-<版>-macos-universal-experimental.zip`: 展開してできるフォルダに、`YoluPainter.app`（Apple Silicon 向けと Intel 向けの両方のコードを持つ universal）、`README.md`、許諾の全文、この文書を含む `docs` フォルダ（英語は `docs/en`）が入っています。`YoluPainter.app` を「アプリケーション」などへ移して使います。コマンドラインのプログラム（[使い方](CLI.md)）は `YoluPainter.app/Contents/MacOS/yolupainter-cli` です。

試作のため、Apple の開発者の署名と公証はありません（署名は ad-hoc だけです）。実行ファイルが要求する最低の macOS は 11 です。Apple Silicon の Mac（macOS 27）で確かめたのは、起動、設定とログの書き込み、署名、universal の形までです。ウィンドウの表示と描画は確かめていません。Intel の Mac では動かしていません。署名がないので、zip を展開した `YoluPainter.app` を初めて開くと、macOS が止めます。次の順で開きます（[Apple の手引き](https://support.apple.com/ja-jp/guide/mac-help/mh40616/mac)）。

1. `YoluPainter.app` をダブルクリックします。止められるので、表示を閉じます。
2. アップルメニュー →「システム設定」を開き、サイドバーの「プライバシーとセキュリティ」を押します。
3. 「セキュリティ」の項目に移動して、「開く」を押します。
4. 「このまま開く」を押します（アプリを開こうとしたあと、約 1 時間だけ使えます）。
5. ログインパスワードを入力して、「OK」を押します。以後は、ダブルクリックで開きます。

Mac のアプリは、自分では更新しません（ダウンロードも入れ替えもしません）。初回の問いで「はい」を選ぶと、起動のたびに（「ヘルプ → 更新を確かめる…」では手で）GitHub へ最新の版を問い合わせ、新しい版があれば「ヘルプ」の見出しに印が付いて、「YoluPainter x.y.z のリリースを開く」でその版のリリースのページを開きます。新しい zip を展開し、`YoluPainter.app` を置き換えてください。置き換えても、設定やプロジェクトは残ります。

アンインストールは `YoluPainter.app` をゴミ箱へ入れます。そのほかのデータは自動では消えないので、要らなければ自分で消します。

| 場所 | 中身 |
| --- | --- |
| `~/Library/Application Support/YoluPainter/` | 設定・復旧用の世代（`recovery/`）・クラッシュの記録（`logs/`）・Live Link の受け渡しのフォルダ（`LiveLink/`）、ライブラリ・ブラシなど自分で作った物 |
| `~/Library/Caches/YoluPainter/thumbnails/` | サムネイルのキャッシュ |

## 更新

初めて起動したとき「起動時に更新を確かめますか？」と聞きます。「はい」を選んだときだけ、起動のたびに GitHub へ最新の版を問い合わせます。この選択は「編集 → 設定…」の「更新」の区分の「起動時に更新を確かめる」でいつでも替えられ、「ヘルプ → 更新を確かめる…」で手でも確かめられます。

「編集 → 設定…」の「更新」の区分の「試験版を使う」を入れると、正式版より前の試験版（版が `0.4.0-rc.1` のように `alpha`・`beta`・`rc` の識別子を持つもの）も更新の候補にし、正式版と試験版の新しい方を勧めます。既定は切で、切のときは正式版だけを見ます。試験版を使っている間は、状態の帯の版の左に「試験版」と出ます。版を下げる更新はしないので、試験版を入れていた人が設定を切っても、次の正式版が今の版より新しくなるまでは何も勧めません。

新しい版があると「ヘルプ」の見出しに印が付き、インストーラーで入れた Windows では「YoluPainter x.y.z に更新」で、ダウンロード・検証・更新までを進めます（保存していない変更があるときは、先に保存するか聞きます）。ダウンロードは、署名つきの更新情報の署名・大きさ・SHA-256 を確かめたものだけを使います。zip で展開した Windows と Linux、Mac では、その版のリリースのページを開きます。ソースからビルドしたアプリには、更新の項目（ヘルプの「更新を確かめる…」と設定の「更新」）はありません（配布物のビルドだけが、更新用の公開鍵を組み込みます）。

インストールした YoluPainter のウィンドウがほかにも開いているときは、「ほかの YoluPainter が開いています」と出して更新を始めません（インストーラーは、使われている実行ファイルを書き換えられないため）。落としたインストーラーは残るので、ほかのウィンドウを閉じてから、もう一度「更新して再起動」を押すとすぐ入ります。インストーラーは実行ファイルが使われている間、無音のときは 60 秒まで待ち、それでも使われていれば何も変えずに終わります。アプリの更新は `/RUN` を付けて走らせるので、このときは今入っているアプリを起こし直します（`/RUN` が無い無音の実行では起こしません）。

## アンインストールとデータ

アンインストールは、入れたファイル（アプリ本体・文書・ショートカット・関連付け）と、更新のために落としたインストーラー（`%LOCALAPPDATA%\YoluPainter\updates`）を消します。そのほかのデータは、「設定と復旧のデータも削除しますか？」に「はい」と答えたとき（無音では `/DELETEDATA`）に、次の表の「削除で消える」物だけを消します。自分で作った物は、どの答えでも消えません。

| 場所 | 中身 | 「削除」で |
| --- | --- | --- |
| `%APPDATA%\YoluPainter\settings.conf` | 設定 | 消える |
| `%APPDATA%\YoluPainter\recovery.conf`・`update.conf`・`layout.json` | 復旧の設定・更新の確かめと試験版の選択・パネルとウィンドウの配置 | 消える |
| `%APPDATA%\YoluPainter\places.conf` | ファイルを選ぶウィンドウの始まりの場所（種類ごとに前に選んだフォルダ） | 消える |
| `%APPDATA%\YoluPainter\recovery\` | 復旧用の世代 | 消える |
| `%APPDATA%\YoluPainter\logs\` | クラッシュの記録 | 消える |
| `%LOCALAPPDATA%\YoluPainter\thumbnails\` | サムネイルのキャッシュ | 消える |
| `%LOCALAPPDATA%\YoluPainter\LiveLink\` | Live Link の受け渡しのフォルダ（Unity の頼みと返事・起きている印） | 消える |
| `%APPDATA%\YoluPainter\Library\`（ライブラリの場所の既定） | 個人のライブラリ | 残る |
| `%APPDATA%\YoluPainter\brushes\` | 自分のブラシ・消しゴム | 残る |
| `%APPDATA%\YoluPainter\subtools\` | 自分のサブツール | 残る |
| `%APPDATA%\YoluPainter\gradients\` | グラデーションのセット | 残る |
| `%APPDATA%\YoluPainter\colorsets\` | カラーセット | 残る |
| `%APPDATA%\YoluPainter\hide_presets\` | 面の隠し方のプリセット | 残る |
| `%APPDATA%\YoluPainter\pose_presets\` | ポーズのプリセット | 残る |
| `%APPDATA%\YoluPainter\actions\` | アクション | 残る |
| `%APPDATA%\YoluPainter\keymap.json` | ショートカット（キー・マウスの組み合わせ・パイメニュー）の割り当て（読めなかったファイルは、次に書くとき `keymap.broken.json` に退避。同じ名前があれば `keymap.broken-2.json` のように番号を付けます） | 残る |
| `%APPDATA%\YoluPainter\tools.json` | ツールバーとブラシのグループの並び（読めなかったファイルは `tools.broken.json` に退避） | 残る |

残る物と、このアプリが作ったのではないファイルがあるときは、`%APPDATA%\YoluPainter` のフォルダごと残ります。設定でライブラリの場所や復旧の置き場を別のフォルダにしているときは、そのフォルダには触りません。`.ylp` などの文書は、どちらの場合も消えません。

## 起動時の警告（SmartScreen）

現在の配布物にはコード署名がありません。Windows の SmartScreen が「Windows によって PC が保護されました」と出したときは、「詳細情報」→「実行」で起動できます。

## プライバシー

更新の確認を選んだとき（「起動時に更新を確かめる」を「はい」にしたとき、または「更新を確かめる…」を押したとき）に GitHub へ最新の版を問い合わせるほかは、ネットワークへ情報を送りません。問い合わせは、更新情報のファイルを取る通常の HTTPS の要求だけです（「試験版を使う」を入れていると、試験版の更新情報のファイルも取ります）。アプリの名前と版（User-Agent）のほかに、利用者を識別する情報は付けません。Live Link は同じ PC のフォルダでのファイルの受け渡しだけで、外からの操作は、設定で入れている間だけ `127.0.0.1`（この PC の中）で待ちます。
