[English](CONTRIBUTING.en.md)

# 貢献の手引き

## 訳を足す

画面の文は [Fluent](https://projectfluent.org/) の `.ftl` で、`crates/yolu-app/locales/<タグ>/<モジュール>.ftl` に言語ごとに置きます。
タグは BCP 47 の形（`ja`・`en`・`zh-TW`・`zh-CN`・`ko` など）です。訳の無い文は英語で出ます。

### 1. 文を訳す

1. `crates/yolu-app/locales/en/` をフォルダごと `crates/yolu-app/locales/<タグ>/` へコピーします。
2. `=` の右の値だけを訳します。キー、`{ $name }` のような引数、`{ " " }` のような空白の書き方は変えません。
3. 英語の複数形の選び（`{ $count -> [one] … *[other] … }`）は、その言語で要らなければ 1 つの文にしてかまいません。
4. 訳していないキーは消してかまいません（英語の文で出ます）。英語に無いキーは足せません。

### 2. 言語の表に 1 行を足す

| ファイル | 足す物 |
|---|---|
| `crates/yolu-ops/src/text.rs` | `Lang` の値と、`IDS` の行（タグ・ロケールの名前の頭・Windows の言語 ID） |
| `crates/yolu-app/src/lang.rs` | `Lang` の値と、`TABLE` の行（その言語での名前・引用の印・括弧・句点・フォント） |

`zh-CN`（简体中文）と `ko`（한국어）は表に行があるので、フォルダを置くだけで選べるようになります。
表に行の無いフォルダは、ビルドのときに断られます。

初めて起動したときの言語は、OS の言語で決まります（Windows は表示言語、Mac は優先する言語の一覧、Linux は `LC_ALL`・`LC_MESSAGES`・`LANG`）。
環境変数 `YOLUPAINTER_LANG=<タグ>` を付けて起動すると、その言語で始まります（設定に言語が無いときだけ）。

### 3. フォント

同梱のフォント（BIZ UDPGothic）に無い字を使う言語や、同じ漢字でも字形が違う言語は、言語のフォントを使います。
中国語と韓国語は Noto Sans CJK（TC・SC・KR）で、その言語を選んだときだけ読みます。
フォントのファイルはリポジトリに入れず、次の命令で取ります（大きさと SHA-256 を確かめて、`crates/yolu-app/assets/fonts-cjk/` に置きます）。

```sh
cargo xtask fetch-fonts
```

新しいフォントを足すときは、`crates/yolu-app/src/ui/fonts.rs`（`LangFont`）、`crates/xtask/src/fonts.rs`（取得の表）、
`tools/licenses-reviewed.json`、`THIRD_PARTY.md`、`installer/yolupainter.nsi`（`FontFiles`）を揃えます。許諾は SIL Open Font License などの、
同梱してよいものに限ります。

### 4. テスト

```sh
cargo test -p yolu-app --test headless ftl_keys::   # 読めるか・キーと引数が英語にあるか（訳の無いキーの数も出ます）
cargo test -p yolu-app --lib ui::fonts             # 訳の全部の字が、その言語のフォントにあるか（cargo xtask fetch-fonts の後）
cargo test -p yolu-app --lib lang::                # 言語の表・書式・OS の言語の見分け
```

画面で確かめるときは、言語を指定して起動するか、パネルを撮ります。

```sh
YOLUPAINTER_LANG=zh-TW cargo run -p yolu-app
cargo run -p yolu-mock -- --panel layers --lang zh-TW --png layers.png
```

### 5. 用語を揃える

- 同じ英語の語は、同じ訳にします。既にある訳を検索してから訳します（例: `grep -rn "Texture Set" crates/yolu-app/locales/en/` で英語のキーを探し、
  同じキーの訳を見ます）。
- 合成モードなど、ほかのアプリと同じ名前の物は、その言語で広く使われている呼び名に合わせます。
- ツールチップは、名前とショートカットのキーだけにします（使い方の説明の文を足しません）。
