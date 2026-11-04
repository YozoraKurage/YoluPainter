[日本語](CONTRIBUTING.md)

# Contributing

## Adding a translation

The UI text is [Fluent](https://projectfluent.org/) `.ftl` files, one folder per language under `crates/yolu-app/locales/<tag>/<module>.ftl`.
The tag is a BCP 47 language tag (`ja`, `en`, `zh-TW`, `zh-CN`, `ko`, ...). Any text without a translation is shown in English.

### 1. Translate the text

1. Copy the folder `crates/yolu-app/locales/en/` to `crates/yolu-app/locales/<tag>/`.
2. Translate only the values on the right of `=`. Keep the keys, the arguments such as `{ $name }`, and spacing written as `{ " " }`.
3. If your language does not need the English plural choice (`{ $count -> [one] … *[other] … }`), a single sentence is fine.
4. You may delete keys you have not translated (they are shown in English). Keys that are not in English cannot be added.

### 2. Add a row to the language table

| File | What to add |
|---|---|
| `crates/yolu-ops/src/text.rs` | A `Lang` value and a row in `IDS` (tag, locale name prefixes, Windows language IDs) |
| `crates/yolu-app/src/lang.rs` | A `Lang` value and a row in `TABLE` (the name in the language itself, quotation marks, parentheses, full stop, font) |

`zh-CN` (简体中文) and `ko` (한국어) already have rows, so adding their folder is enough to make them selectable.
A folder without a row in the table is refused at build time.

On the first start, the language follows the OS (the display language on Windows, the preferred languages on Mac, `LC_ALL`, `LC_MESSAGES` and `LANG` on Linux).
Start with the environment variable `YOLUPAINTER_LANG=<tag>` to begin in that language (only while the settings have no language yet).

### 3. Fonts

Languages that use characters missing from the bundled font (BIZ UDPGothic), or that draw the same Han characters with different shapes, use a language font.
Chinese and Korean use Noto Sans CJK (TC, SC, KR), loaded only when that language is selected.
The font files are not in the repository. Fetch them with the command below (it checks the size and SHA-256 and places them in `crates/yolu-app/assets/fonts-cjk/`).

```sh
cargo xtask fetch-fonts
```

To add a new font, update `crates/yolu-app/src/ui/fonts.rs` (`LangFont`), `crates/xtask/src/fonts.rs` (the download table),
`tools/licenses-reviewed.json`, `THIRD_PARTY.md` and `installer/yolupainter.nsi` (`FontFiles`) together. Only fonts whose license allows bundling,
such as the SIL Open Font License, can be added.

### 4. Tests

```sh
cargo test -p yolu-app --test headless ftl_keys::   # the files read, and every key and argument exists in English (also prints the number of untranslated keys)
cargo test -p yolu-app --lib ui::fonts             # every character of a translation is in its fonts (after cargo xtask fetch-fonts)
cargo test -p yolu-app --lib lang::                # the language table, formats and OS language detection
```

To check the screen, start the app in that language, or capture a panel.

```sh
YOLUPAINTER_LANG=zh-TW cargo run -p yolu-app
cargo run -p yolu-mock -- --panel layers --lang zh-TW --png layers.png
```

### 5. Keeping terms consistent

- Translate the same English word the same way. Search the existing translation first (for example, find the English key with
  `grep -rn "Texture Set" crates/yolu-app/locales/en/` and look at the translation of the same key).
- For names shared with other applications, such as blend modes, use the names commonly used in your language.
- Tooltips hold only the name and the shortcut key (no sentences explaining how to use it).
