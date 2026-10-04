# 第三者の許諾

Windows 向け `yolu-app`（スタンドアロン）と `yolu-bridge`（Unity に入れる DLL）の依存一覧。
対象は `x86_64-pc-windows-gnu`、既定の機能、下表に記録した Cargo.lock。
依存を更新したときや別のターゲット・機能で配るときは、一覧と全文束を更新する。
Mac・Linux・Windows MSVC の一覧を兼ねるものではない。

`tools/third-party.py` は Cargo の通常依存とビルド依存を製品別にたどり、試験用依存を除く。
手続きマクロとビルド依存も保守的に全文束へ含める。「実行時」は通常依存の到達範囲であり、
最適化後のバイナリにそのクレートの全コードが残るという意味ではない。
`tools/licenses-reviewed.json` に、版・宣言された許諾・選択する許諾・原文の場所・SHA-256 を固定してある。
クレートに原文が無い場合は、そのクレートの発行時コミットにある上流の原文を取得する。
単に同じ種類の一般的な許諾文で代用せず、著作権表記と NOTICE も保持する。

生成方法は [README](README.md#許諾の一覧と配布用の全文) を参照。
一覧の JSON と Markdown、成功した製品の `THIRD_PARTY_LICENSES.txt` は `target/third-party/<クレート>/` にできる。
生成物は Git に入れない。この文書は確認済みの依存構成の記録で、配布には生成した全文を同梱する。

## 承認済みの条件と配布前に確認する点

- `clipboard-win 5.4.1` と `error-code 3.4.0` は **BSL-1.0（Boost Software License）**。
  **2026-10-04 にユーザーが使用を許可した**。`egui-winit → arboard → clipboard-win → error-code` から入る。
  出典: [clipboard-win の原文](https://github.com/DoumanAsh/clipboard-win/blob/3b27cf2bfd1adcfa6e0264eb51c1025ddaf0f342/LICENSE)、
  [error-code の原文](https://github.com/DoumanAsh/error-code/blob/e4615e514db3ff64f5f1b328b4865b20a3dcdbc3/LICENSE)。
- egui の標準書体に含まれる Hack の原文には **Bitstream Vera** の条件もある。
  **2026-10-04 にユーザーがこの条件も許可した**。OFL-1.1・Ubuntu Font Licence とともに全文を保持する。
- 上記の発行時コミットの原文と SHA-256 を照合し、**app・bridge ともクレート分の配布用全文束を生成できる**。
  未確認の版や原文の変更を自動承認せず、依存を更新したら再確認する。
- `self_cell` の宣言は `Apache-2.0 OR GPL-2.0-only`。選択するのは Apache-2.0 であり、GPL の条件は選択しない。
  `Unlicense OR MIT` も MIT を選択する。AND の条件はすべて残す。
- 自作部分の配布許諾、Rust 標準ライブラリ、実際にリンクする MinGW/GCC ランタイム、追加で同梱する DLL の表記は、
  最終的な配布物と使用ツールチェーンに合わせて別途確認する。このクレート一覧だけで製品全体の配布可否は確定しない。
  リポジトリはバイナリ配布時に公開する。

## フォントとアイコン

`epaint_default_fonts 0.36.2` の原文は
[発行時の fonts ディレクトリ](https://github.com/emilk/egui/tree/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint_default_fonts/fonts) にある。
フォントを加工せず同梱する前提で、次の全文を収集する。

| 書体 | 許諾と原文 |
|---|---|
| Noto Emoji | OFL-1.1、`fonts/OFL.txt` |
| Ubuntu Light | Ubuntu Font Licence 1.0、`fonts/UFL.txt` |
| Hack | MIT、DejaVu のパブリックドメインの注記、Bitstream Vera、`fonts/Hack-Regular.txt` |
| emoji-icon-font | MIT、`fonts/emoji-icon-font-mit-license.txt` |

OFL-1.1 と Ubuntu Font Licence は標準書体に限って含めてよいものとし、各書体の著作権・名称・条件を原文のまま残す。
書体を変更して配る場合は、予約された書体名などの条件を再確認する。
Fluent UI System Icons と Phosphor Icons は MIT。
[既存のアイコンの表記](crates/yolu-app/assets/icons/THIRD-PARTY-NOTICES.md) も app の全文束に含める。
アイコンは下表のクレート件数には含めない。

`libz-sys` の Rust 側は MIT を選び、同梱 zlib の Zlib 許諾も含める。
`unicode-ident` の Unicode-3.0、ビルド用 `regex-syntax` の Unicode データの Unicode-DFS-2016、
`tracing-core` 内の spin の MIT 原文も収集する。未使用の zlib contrib・zlib-ng と Linux 専用 rfd バックエンドは対象外。

## 生成・試験に使う道具

今回の道具は Python 3.10 以降の標準ライブラリだけを使い、cargo-about / cargo-deny / pip の追加パッケージは不要。
以下は開発環境で使う道具の許諾であり、道具本体を製品へ同梱しない。

| 道具 | 許諾・参照元 |
|---|---|
| Python | PSF License と付随する許諾。 [Python の表記](https://docs.python.org/3/license.html) |
| Rust / Cargo | 主に MIT OR Apache-2.0。 [Rust](https://github.com/rust-lang/rust/blob/master/COPYRIGHT)・[Cargo](https://github.com/rust-lang/cargo/blob/master/LICENSE-MIT) の第三者表記も参照 |
| Bash | GPL-3.0-or-later。 [Bash](https://www.gnu.org/software/bash/) |
| Wine | LGPL-2.1-or-later。使用環境の `/usr/share/doc/wine/copyright` と [上流の COPYING.LIB](https://gitlab.winehq.org/wine/wine/-/blob/master/COPYING.LIB) |
| MinGW-w64 の GCC・binutils | ツール本体は GPL 系。ランタイムは別の許諾・例外を持つため [MinGW-w64](https://www.mingw-w64.org/) と各インストールの copyright を確認 |

Wine 用の `bcryptprimitives.dll` はこのリポジトリの小さな接続コードから試験時だけ作る。
Windows 配布物や Unity の Plugins に入れない。Wine や Windows の DLL をコピーして作るものではない。

## yolu-app の依存一覧

対象: `x86_64-pc-windows-gnu`、通常の機能。Cargo.lock SHA-256: `d19d315ee6a40e7de4d20cae9075f4023bd4e28bd024adbdbe8a5ce8f7244f13`。

外部クレート 180 件（同名の別版は別件）。実行時 153 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| 0BSD | 2 |
| Apache-2.0 | 12 |
| Apache-2.0 AND MIT | 1 |
| BSL-1.0 | 2 |
| ISC | 1 |
| MIT | 158 |
| MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 1 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Zlib | 1 |
| Zlib | 1 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| accesskit | 0.24.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_consumer | 0.35.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_windows | 0.32.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| accesskit_winit | 0.32.2 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| adler2 | 2.0.1 | 実行時 | 0BSD OR MIT OR Apache-2.0 | MIT | 確認済み |
| ahash | 0.8.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| allocator-api2 | 0.2.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arboard | 3.6.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| arrayvec | 0.7.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ash | 0.38.0+1.3.281 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| autocfg | 1.5.1 | ビルド・マクロ用 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-set | 0.10.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bit-vec | 0.9.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| bitflags | 2.13.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| block-buffer | 0.10.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| bytemuck | 1.25.2 | 実行時 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| bytemuck_derive | 1.12.1 | ビルド・マクロ用 | Zlib OR Apache-2.0 OR MIT | MIT | 確認済み |
| byteorder-lite | 0.1.0 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| cc | 1.6.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg-if | 1.0.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cfg_aliases | 0.2.2 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| clipboard-win | 5.4.1 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| codespan-reporting | 0.13.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| color | 0.3.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| cpufeatures | 0.2.17 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crc32fast | 1.5.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-deque | 0.8.8 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-epoch | 0.9.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crossbeam-utils | 0.8.23 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| crypto-common | 0.1.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| cursor-icon | 1.2.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| digest | 0.10.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| doctest-file | 1.1.1 | ビルド・マクロ用 | 0BSD | 0BSD | 確認済み |
| document-features | 0.2.12 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| dpi | 0.1.2 | 実行時 | Apache-2.0 AND MIT | Apache-2.0 AND MIT | 確認済み |
| duplicate | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| ecolor | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| eframe | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-wgpu | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui-winit | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| egui_dock | 0.21.1 | 実行時 | MIT | MIT | 確認済み |
| either | 1.18.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| emath | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint | 0.36.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| epaint_default_fonts | 0.36.2 | 実行時 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | MIT AND OFL-1.1 AND Ubuntu-font-1.0 AND Bitstream-Vera | 確認済み |
| equivalent | 1.0.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| error-code | 3.4.0 | 実行時 | BSL-1.0 | BSL-1.0 | 確認済み |
| euclid | 0.22.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fdeflate | 0.3.7 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| fearless_simd | 0.4.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| find-msvc-tools | 0.1.14 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| flate2 | 1.1.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| foldhash | 0.2.0 | 実行時 | Zlib | Zlib | 確認済み |
| font-types | 0.12.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| generic-array | 0.14.7 | 実行時 | MIT | MIT | 確認済み |
| gl_generator | 0.14.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| glam | 0.33.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| glow | 0.17.0 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| glutin_wgl_sys | 0.6.1 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| gpu-allocator | 0.28.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| guillotiere | 0.7.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| half | 2.7.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| harfrust | 0.12.0 | 実行時 | MIT | MIT | 確認済み |
| hashbrown | 0.16.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| hashbrown | 0.17.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| heck | 0.5.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| image | 0.25.10 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| indexmap | 2.14.2 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| interprocess | 2.4.4 | 実行時 | 0BSD OR Apache-2.0 | Apache-2.0 | 確認済み |
| itertools | 0.15.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| itoa | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| khronos-egl | 6.0.0 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| khronos_api | 3.1.0 | ビルド・マクロ用 | Apache-2.0 | Apache-2.0 | 確認済み |
| kurbo | 0.13.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| libc | 0.2.190 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| libloading | 0.8.9 | 実行時 | ISC | ISC | 確認済み |
| libm | 0.2.16 | 実行時 | MIT | MIT | 確認済み |
| libz-sys | 1.1.29 | 実行時 | MIT OR Apache-2.0 | MIT AND Zlib | 確認済み |
| linebender_resource_handle | 0.1.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| litrs | 1.0.0 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| lock_api | 0.4.14 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| log | 0.4.34 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| memchr | 2.8.3 | 実行時 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.8.9 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| miniz_oxide | 0.9.1 | 実行時 | MIT OR Zlib OR Apache-2.0 | MIT | 確認済み |
| moxcms | 0.8.1 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| naga-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| naga | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| nohash-hasher | 0.2.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| num-traits | 0.2.19 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| once_cell | 1.21.4 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| ordered-float | 5.5.0 | 実行時 | MIT | MIT | 確認済み |
| parking_lot | 0.12.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| parking_lot_core | 0.9.12 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| paste | 1.0.15 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| peniko | 0.6.1 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pin-project-lite | 0.2.17 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| pkg-config | 0.3.34 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| png | 0.18.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pollster | 1.0.1 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| polycool | 0.4.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| presser | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2-diagnostics | 0.10.1 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| profiling | 1.0.18 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| pxfm | 0.1.30 | 実行時 | BSD-3-Clause OR Apache-2.0 | Apache-2.0 | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| range-alloc | 0.1.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| raw-window-handle | 0.6.2 | 実行時 | MIT OR Apache-2.0 OR Zlib | MIT | 確認済み |
| rayon-core | 1.13.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rayon | 1.12.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| read-fonts | 0.41.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| recvmsg | 1.0.0 | 実行時 | 0BSD | 0BSD | 確認済み |
| renderdoc-sys | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| rfd | 0.17.2 | 実行時 | MIT | MIT | 確認済み |
| rustc-hash | 1.1.0 | 実行時 | Apache-2.0/MIT | MIT | 確認済み |
| rustc-hash | 2.1.3 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| scopeguard | 1.2.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| self_cell | 1.3.0 | 実行時 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 | 確認済み |
| serde | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_core | 1.0.229 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| serde_json | 1.0.151 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| sha2 | 0.10.9 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| shlex | 2.0.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| simd-adler32 | 0.3.10 | 実行時 | MIT | MIT | 確認済み |
| skrifa | 0.44.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smallvec | 1.16.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| smol_str | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| spirv | 0.4.0+sdk-1.4.341.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| static_assertions | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 3.0.6 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror-impl | 2.0.21 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| thiserror | 2.0.21 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| tracing-core | 0.1.36 | 実行時 | MIT | MIT | 確認済み |
| tracing | 0.1.44 | 実行時 | MIT | MIT | 確認済み |
| type-map | 0.5.1 | 実行時 | MIT/Apache-2.0 | MIT | 確認済み |
| typenum | 1.20.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-general-category | 1.1.0 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| unicode-ident | 1.0.26 | 実行時 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| unicode-segmentation | 1.13.3 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-width | 0.2.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| uuid | 1.27.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vcpkg | 0.2.15 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| vello_common | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| vello_cpu | 0.1.0 | 実行時 | Apache-2.0 OR MIT | MIT | 確認済み |
| version_check | 0.9.5 | ビルド・マクロ用 | MIT/Apache-2.0 | MIT | 確認済み |
| web-time | 1.1.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core-deps-windows-linux-android | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-core | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-hal | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-naga-bridge | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu-types | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| wgpu | 30.0.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| widestring | 1.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-collections | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-core | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-future | 0.3.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-implement | 0.60.2 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-interface | 0.59.3 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-numerics | 0.3.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-result | 0.4.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-strings | 0.5.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.52.0 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.60.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-targets | 0.53.5 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-threading | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows | 0.62.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_gnu | 0.52.6 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows_x86_64_gnu | 0.53.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| winit | 0.30.13 | 実行時 | Apache-2.0 | Apache-2.0 | 確認済み |
| xml-rs | 0.8.29 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| zerocopy-derive | 0.8.59 | ビルド・マクロ用 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zerocopy | 0.8.59 | 実行時 | BSD-2-Clause OR Apache-2.0 OR MIT | MIT | 確認済み |
| zmij | 1.0.23 | 実行時 | MIT | MIT | 確認済み |

## yolu-bridge の依存一覧

対象: `x86_64-pc-windows-gnu`、通常の機能。Cargo.lock SHA-256: `d19d315ee6a40e7de4d20cae9075f4023bd4e28bd024adbdbe8a5ce8f7244f13`。

外部クレート 17 件（同名の別版は別件）。実行時 6 件。

ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。

| 選択した許諾（追加条件を含む） | 件数 |
|---|---:|
| 0BSD | 2 |
| Apache-2.0 | 1 |
| MIT | 12 |
| MIT AND Unicode-3.0 | 1 |
| MIT AND Unicode-DFS-2016 | 1 |

状態: クレートの許諾照合は成功。

| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |
|---|---|---|---|---|---|
| aho-corasick | 1.1.5 | ビルド・マクロ用 | Unlicense OR MIT | MIT | 確認済み |
| csbindgen | 1.9.8 | ビルド・マクロ用 | MIT | MIT | 確認済み |
| doctest-file | 1.1.1 | ビルド・マクロ用 | 0BSD | 0BSD | 確認済み |
| interprocess | 2.4.4 | 実行時 | 0BSD OR Apache-2.0 | Apache-2.0 | 確認済み |
| memchr | 2.8.3 | ビルド・マクロ用 | Unlicense OR MIT | MIT | 確認済み |
| memmap2 | 0.9.11 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| proc-macro2 | 1.0.107 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| quote | 1.0.47 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| recvmsg | 1.0.0 | 実行時 | 0BSD | 0BSD | 確認済み |
| regex-automata | 0.4.18 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| regex-syntax | 0.8.11 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT AND Unicode-DFS-2016 | 確認済み |
| regex | 1.13.1 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| syn | 2.0.119 | ビルド・マクロ用 | MIT OR Apache-2.0 | MIT | 確認済み |
| unicode-ident | 1.0.26 | ビルド・マクロ用 | (MIT OR Apache-2.0) AND Unicode-3.0 | MIT AND Unicode-3.0 | 確認済み |
| widestring | 1.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-link | 0.2.1 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
| windows-sys | 0.61.2 | 実行時 | MIT OR Apache-2.0 | MIT | 確認済み |
