//! 画面のフォント。BIZ UDPGothic（Regular と Bold。SIL OFL 1.1。全文は `assets/fonts/OFL.txt`）を実行ファイルに同梱し、全 OS で同じフォントを使う。
//! OS のフォントを読む方式は、Windows の游ゴシックが縦の寸法の癖で文字を上に寄せ、かなを広げて見せたのでやめた。
//!
//! 英数字も同じフォントで揃える（egui の既定の Ubuntu Light と組まない）: 同じフォントなら、かなと英字の高さと太さが揃い、行の中で字の
//! 大きさがばらつかない。egui の既定のフォントは、このフォントに無い記号（絵文字など）を補う後ろ盾として末尾に残す。
//! 縦の位置はフォントの寸法（ascent 1802・descent 246・1 em = 2048）のとおりで、行の高さの真ん中が漢字の字面の真ん中に合うので、
//! 補正（`FontTweak`）は入れない（`tests/gui_shell/fonts.rs` が字面の真ん中を測って確かめる）。
//!
//! 下線（_）・ダッシュ（– — ―）・マイナス（−）・上線（‾ ¯）などは、原本のヒンティングの命令が、細い棒（0.3〜0.6 画素）の上下の縁を別々に
//! 画素の格子へ丸め、両方が同じ線に落ちて厚み 0 になる大きさ（12 pt の `_` など）があり、egui は箱の高さが 0 の字を描かない。
//! `FontTweak` の `hinting_target` はこのフォント（命令を持つ）には効かず、`hinting` を切ると全部の字がぼやけるので、消えるグリフだけを
//! 命令なしで抜き出した小さなフォント（`*-Lines.ttf`。作り方は `tools/make-ui-font-lines.py`）を、普通・太字のそれぞれの先頭に置く。
//! 抜粋は縦の寸法が原本と同じで、egui が先頭のフォントの寸法で決める行の高さは変わらない。
//!
//! 中国語・韓国語の画面は、その言語のフォント（Noto Sans CJK の TC・SC・KR。SIL OFL 1.1）を BIZ UDPGothic より先に置く（同じ漢字が
//! 日本の字形で出ない。BIZ UDPGothic に無い字も出る）。このフォントは実行ファイルに埋めず、配布物の `fonts/`（Mac は .app の
//! `Contents/Resources/fonts/`）に別のファイルとして置き、その言語を選んだときだけ読む（[`LangFonts`]）。細い字の抜粋は先頭のまま
//! なので、行の高さは日本語・英語の画面と同じ（その言語のフォントの字は、egui が行の中で縦の真ん中に寄せる）。

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use egui::{FontData, FontDefinitions, FontFamily};

use super::theme::BOLD;
use crate::lang::Lang;

/// 普通の太さ。
pub(crate) const REGULAR: &[u8] = include_bytes!("../../assets/fonts/BIZUDPGothic-Regular.ttf");
/// 太字。
pub(crate) const BOLD_FACE: &[u8] = include_bytes!("../../assets/fonts/BIZUDPGothic-Bold.ttf");
/// 普通の太さの、細い字だけの抜粋（命令なし）。
pub(crate) const REGULAR_LINES: &[u8] =
    include_bytes!("../../assets/fonts/BIZUDPGothic-Regular-Lines.ttf");
/// 太字の、細い字だけの抜粋（命令なし）。
pub(crate) const BOLD_LINES: &[u8] =
    include_bytes!("../../assets/fonts/BIZUDPGothic-Bold-Lines.ttf");

/// 同梱したフォントの名前（`FontDefinitions::font_data` の鍵）。
pub const REGULAR_NAME: &str = "biz-udpgothic-regular";
pub const BOLD_NAME: &str = "biz-udpgothic-bold";
pub const REGULAR_LINES_NAME: &str = "biz-udpgothic-regular-lines";
pub const BOLD_LINES_NAME: &str = "biz-udpgothic-bold-lines";

pub use crate::lang::font::{LangFont, NOTO_SANS_KR, NOTO_SANS_SC, NOTO_SANS_TC};

/// 読んだ言語のフォント（普通と太字）。
#[derive(Clone)]
pub struct LoadedFont {
    pub font: &'static LangFont,
    /// 読んだ中身。フォントの定義へはこの `Arc` をそのまま渡す（言語を替えるたびにバイト列を写さない）。
    pub regular: Arc<FontData>,
    pub bold: Arc<FontData>,
}

/// 言語のフォントのファイルを探すフォルダ（先が先）: 実行ファイルの隣の `fonts/`、Mac の .app の `Contents/Resources/fonts/`、
/// 開発の実行（`cargo run`・テスト）は `crates/yolu-app/assets/fonts-cjk/`（`cargo xtask fetch-fonts` が置く。git に入れない）。
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir.join("fonts"));
        if cfg!(target_os = "macos") {
            dirs.push(exe_dir.join("../Resources/fonts"));
        }
    }
    if cfg!(debug_assertions) {
        dirs.push(dev_font_dir());
    }
    dirs
}

/// 開発のときの言語のフォントの置き場（`cargo xtask fetch-fonts` の置き先）。
pub fn dev_font_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts-cjk")
}

/// 言語のフォントを読む（`dirs` の先のフォルダから。普通の太さのファイルがあるフォルダの物を使う）。太字が無ければ普通の太さで代える。
/// 中身は `yolu_update::fonts` の表（`cargo xtask fetch-fonts` と同じ）の大きさと SHA-256 に合う物だけ使う: egui は壊れたフォントを
/// 読むと落ちるので、1 バイトでも違うファイルは読めない物として断る（太字だけ合わないときは普通の太さで代える）。
pub fn load(font: &'static LangFont, dirs: &[PathBuf]) -> std::io::Result<LoadedFont> {
    let dir = dirs
        .iter()
        .find(|dir| dir.join(font.regular).is_file())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let read = |name: &str| -> std::io::Result<Arc<FontData>> {
        let bytes = std::fs::read(dir.join(name))?;
        yolu_update::fonts::check(name, &bytes)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, why))?;
        Ok(Arc::new(FontData::from_owned(bytes)))
    };
    let regular = read(font.regular)?;
    let bold = read(font.bold).unwrap_or_else(|_| regular.clone());
    Ok(LoadedFont {
        font,
        regular,
        bold,
    })
}

/// フォントの定義（細い字の抜粋・普通・太字を先頭に置き、egui の既定のフォントを後ろに残す）。日本語・英語の画面の並び。
pub fn definitions() -> FontDefinitions {
    definitions_with(None)
}

/// 言語のフォントを入れたフォントの定義: 細い字の抜粋 → 言語のフォント → BIZ UDPGothic → egui の既定（太字も同じ形）。
/// `lang` が None なら [`definitions`] と同じ。
pub fn definitions_with(lang: Option<&LoadedFont>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert(REGULAR_NAME.into(), FontData::from_static(REGULAR).into());
    fonts
        .font_data
        .insert(BOLD_NAME.into(), FontData::from_static(BOLD_FACE).into());
    fonts.font_data.insert(
        REGULAR_LINES_NAME.into(),
        FontData::from_static(REGULAR_LINES).into(),
    );
    fonts.font_data.insert(
        BOLD_LINES_NAME.into(),
        FontData::from_static(BOLD_LINES).into(),
    );
    // 言語のフォント（あれば、BIZ UDPGothic の前）
    let lang_names = lang.map(|loaded| {
        let regular = format!("{}-regular", loaded.font.key);
        let bold = format!("{}-bold", loaded.font.key);
        fonts
            .font_data
            .insert(regular.clone(), loaded.regular.clone());
        fonts.font_data.insert(bold.clone(), loaded.bold.clone());
        (regular, bold)
    });
    let (lang_regular, lang_bold): (Vec<String>, Vec<String>) = lang_names.into_iter().unzip();
    // 抜粋が先頭: 同じ字を持つフォントのうち先のものが使われる
    let mut proportional = vec![REGULAR_LINES_NAME.to_owned()];
    proportional.extend(lang_regular.iter().cloned());
    proportional.push(REGULAR_NAME.to_owned());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .splice(0..0, proportional);
    // 均等幅は既定のフォント（Hack）のまま、日本語（と言語の字）だけ同梱のフォントで補う
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    mono.extend(lang_regular.iter().cloned());
    mono.push(REGULAR_NAME.into());
    // 太字: 太字の抜粋 → （言語の太字）→ 太字 → 普通の並び → 既定（記号の補い）
    let mut bold = vec![BOLD_LINES_NAME.to_owned()];
    bold.extend(lang_bold);
    bold.push(BOLD_NAME.to_owned());
    bold.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts.families.insert(FontFamily::Name(BOLD.into()), bold);
    fonts
}

/// フォントを入れる（日本語・英語の並び。言語のフォントは [`LangFonts`] が後から入れる）。
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// 画面の言語に合わせて、言語のフォントを読み・入れ替える（アプリが毎フレーム [`LangFonts::sync`] を呼ぶ）。ファイルは別のスレッドで
/// 読み、読めたら入れて描き直す（言語を替えた直後の数フレームは、前の並びのまま描く）。読めた物は覚えておき、言語を戻すときは読み直さない。
#[derive(Default)]
pub struct LangFonts {
    /// 今入れている言語のフォント（None は日本語・英語の並び）。
    installed: Option<&'static LangFont>,
    loaded: Vec<LoadedFont>,
    /// 読んでいる途中のフォントと、結果の受け口。
    loading: Option<(
        &'static LangFont,
        mpsc::Receiver<std::io::Result<LoadedFont>>,
    )>,
    /// 読めなかったフォント（知らせは 1 回。同じ実行の中では読み直さない）。
    failed: Vec<&'static LangFont>,
    /// 探すフォルダ（None は [`font_dirs`]。テストが替える）。
    dirs: Option<Vec<PathBuf>>,
}

impl LangFonts {
    /// `dirs` のフォルダから探す（テスト）。
    pub fn with_dirs(dirs: Vec<PathBuf>) -> LangFonts {
        LangFonts {
            dirs: Some(dirs),
            ..Default::default()
        }
    }

    /// `lang` のフォントを入れる。読めなかったときは、知らせの文を返す（1 つのフォントにつき 1 回）。
    pub fn sync(&mut self, ctx: &egui::Context, lang: Lang) -> Option<String> {
        let want = lang.info().font;
        let mut problem = None;
        if let Some((font, receiver)) = &self.loading {
            let font = *font;
            match receiver.try_recv() {
                Ok(Ok(loaded)) => {
                    self.loaded.push(loaded);
                    self.loading = None;
                }
                Ok(Err(error)) => {
                    self.loading = None;
                    self.failed.push(font);
                    problem = Some(lang.with_reason(
                        lang.f("fonts-cannot-load-the-font", &[("name", &font.regular)]),
                        lang.file_error(&error),
                    ));
                }
                // 読み終わるまで描き直しを続ける（裏のスレッドの描き直しの頼みを待たずに、読めたフレームで入れる）
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint(),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.failed.push(font);
                }
            }
        }
        if want == self.installed {
            return problem;
        }
        let Some(font) = want else {
            ctx.set_fonts(definitions());
            self.installed = None;
            return problem;
        };
        if let Some(loaded) = self.loaded.iter().find(|l| l.font == font) {
            ctx.set_fonts(definitions_with(Some(loaded)));
            self.installed = Some(font);
            ctx.request_repaint();
        } else if self.failed.contains(&font) {
            // 読めなかった言語では日本語・英語の並びのまま
            if self.installed.is_some() {
                ctx.set_fonts(definitions());
                self.installed = None;
            }
        } else if self.loading.as_ref().is_none_or(|(f, _)| *f != font) {
            let (sender, receiver) = mpsc::channel();
            let waker = ctx.clone();
            let dirs = self.dirs.clone();
            let spawned = std::thread::Builder::new()
                .name("lang-font".into())
                .spawn(move || {
                    let _ = sender.send(load(font, &dirs.unwrap_or_else(font_dirs)));
                    waker.request_repaint();
                });
            if spawned.is_ok() {
                self.loading = Some((font, receiver));
                ctx.request_repaint();
            } else {
                self.failed.push(font);
            }
        }
        problem
    }

    /// 読んでいる途中か（テスト）。
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// 今入れている言語のフォント（テスト）。
    pub fn installed(&self) -> Option<&'static LangFont> {
        self.installed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sfnt の表（タグで探す）。
    fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        (0..count).find_map(|i| {
            let entry = &font[12 + 16 * i..28 + 16 * i];
            (entry[..4] == tag[..]).then(|| {
                let offset = u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize;
                let length = u32::from_be_bytes(entry[12..16].try_into().unwrap()) as usize;
                &font[offset..offset + length]
            })
        })
    }

    #[test]
    fn the_bundled_faces_come_first_and_the_defaults_stay_as_a_fallback() {
        let fonts = definitions();
        let proportional = &fonts.families[&FontFamily::Proportional];
        // 細い字の抜粋が先頭（同じ字を持つフォントのうち先のものが使われる）。次が本体
        assert_eq!(proportional[0], REGULAR_LINES_NAME);
        assert_eq!(proportional[1], REGULAR_NAME);
        assert!(proportional.len() > 2, "既定のフォントを後ろ盾に残す");
        let bold = &fonts.families[&FontFamily::Name(BOLD.into())];
        assert_eq!(bold[0], BOLD_LINES_NAME);
        assert_eq!(bold[1], BOLD_NAME);
        assert_eq!(bold[2], REGULAR_LINES_NAME);
        assert_eq!(bold[3], REGULAR_NAME);
        // 均等幅は既定のフォントが先で、日本語だけ同梱のフォントで補う
        let mono = &fonts.families[&FontFamily::Monospace];
        assert_ne!(mono[0], REGULAR_NAME);
        assert_eq!(mono.last().map(String::as_str), Some(REGULAR_NAME));
    }

    #[test]
    fn the_faces_are_the_published_files() {
        // 同梱ファイルは配布元の原本のまま（加工しない。SHA-256 は tools/licenses-reviewed.json の bundled にも置く）
        assert_eq!(REGULAR.len(), 4_669_688);
        assert_eq!(BOLD_FACE.len(), 4_640_592);
        for face in [REGULAR, BOLD_FACE] {
            assert_eq!(&face[..4], &[0, 1, 0, 0], "TrueType");
        }
    }

    #[test]
    fn the_line_faces_have_no_hinting_instructions_and_the_published_faces_do() {
        // 命令を持つフォントは egui の読み出し（skrifa）が命令の道を使い、細い棒の厚みが 0 に潰れる。
        // 抜粋に命令が残ると（作り直しの設定の誤りなど）、消える不具合が戻る
        for face in [REGULAR, BOLD_FACE] {
            for tag in [b"fpgm", b"prep", b"cvt "] {
                assert!(table(face, tag).is_some(), "原本の {tag:?}");
            }
        }
        for face in [REGULAR_LINES, BOLD_LINES] {
            assert_eq!(&face[..4], &[0, 1, 0, 0], "TrueType");
            for tag in [b"fpgm", b"prep", b"cvt ", b"gasp"] {
                assert!(table(face, tag).is_none(), "抜粋に {tag:?} がある");
            }
            for tag in [
                b"glyf", b"loca", b"cmap", b"hmtx", b"head", b"hhea", b"maxp",
            ] {
                assert!(table(face, tag).is_some(), "抜粋に {tag:?} が無い");
            }
            // maxp の maxSizeOfInstructions（26）が 0
            let maxp = table(face, b"maxp").unwrap();
            assert_eq!(u16::from_be_bytes([maxp[26], maxp[27]]), 0);
        }
    }

    #[test]
    fn the_line_faces_keep_the_vertical_metrics_of_the_published_faces() {
        // egui は先頭のフォントの寸法で行の高さを決める。抜粋が先頭に来ても、行の高さと文字の縦の位置が変わらない
        for (lines, face) in [(REGULAR_LINES, REGULAR), (BOLD_LINES, BOLD_FACE)] {
            let words = |font: &[u8], tag: &[u8; 4], offsets: &[usize]| -> Vec<u16> {
                let data = table(font, tag).unwrap();
                offsets
                    .iter()
                    .map(|&o| u16::from_be_bytes([data[o], data[o + 1]]))
                    .collect()
            };
            // head: unitsPerEm（18）、hhea: ascender・descender・lineGap（4・6・8）、
            // OS/2: sTypoAscender・sTypoDescender・sTypoLineGap・usWinAscent・usWinDescent（68・70・72・74・76）
            for (tag, offsets) in [
                (b"head", &[18usize][..]),
                (b"hhea", &[4, 6, 8][..]),
                (b"OS/2", &[68, 70, 72, 74, 76][..]),
            ] {
                assert_eq!(
                    words(lines, tag, offsets),
                    words(face, tag, offsets),
                    "{tag:?}"
                );
            }
        }
    }

    /// フォントの定義 `fonts` で、字の並びの幅（点）。
    fn width_of(fonts: FontDefinitions, text: &str, family: FontFamily, size: f32) -> f32 {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts);
        let screen = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 200.0),
            )),
            ..Default::default()
        };
        // フォントは最初のパスの初めに効く。念のため 2 回回して、2 回目に測る
        let mut width = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(screen.clone(), |_| {
                width = ctx.fonts_mut(|f| {
                    f.layout_no_wrap(
                        text.to_owned(),
                        egui::FontId::new(size, family.clone()),
                        egui::Color32::WHITE,
                    )
                    .size()
                    .x
                });
            });
            output.textures_delta.clear();
        }
        width
    }

    #[test]
    fn the_line_faces_do_not_change_the_advance_widths() {
        // 抜粋の字の送り幅は原本と同じ（字の並びの幅が変わると、画面の字の位置が全部ずれる）
        let without_lines = || {
            let mut fonts = definitions();
            for family in fonts.families.values_mut() {
                family.retain(|name| name != REGULAR_LINES_NAME && name != BOLD_LINES_NAME);
            }
            fonts
        };
        let text = "a_b \u{2014} \u{2013} \u{2015} \u{2212}X \u{203E} \u{00AF}z";
        for family in [FontFamily::Proportional, FontFamily::Name(BOLD.into())] {
            for size in [10.0, 12.0, 14.0] {
                let with = width_of(definitions(), text, family.clone(), size);
                let without = width_of(without_lines(), text, family.clone(), size);
                assert!(with > 0.0);
                assert!(
                    (with - without).abs() < 0.01,
                    "{family:?} {size} pt: 抜粋あり {with}・なし {without}"
                );
            }
        }
    }

    #[test]
    fn a_language_font_goes_before_the_bundled_faces_and_after_the_line_faces() {
        let loaded = LoadedFont {
            font: &NOTO_SANS_TC,
            regular: Arc::new(FontData::from_static(REGULAR)),
            bold: Arc::new(FontData::from_static(BOLD_FACE)),
        };
        let fonts = definitions_with(Some(&loaded));
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(
            proportional[..3],
            [REGULAR_LINES_NAME, "noto-sans-tc-regular", REGULAR_NAME]
        );
        let bold = &fonts.families[&FontFamily::Name(BOLD.into())];
        assert_eq!(
            bold[..5],
            [
                BOLD_LINES_NAME,
                "noto-sans-tc-bold",
                BOLD_NAME,
                REGULAR_LINES_NAME,
                "noto-sans-tc-regular"
            ]
        );
        let mono = &fonts.families[&FontFamily::Monospace];
        assert_eq!(
            mono[mono.len() - 2..],
            ["noto-sans-tc-regular".to_owned(), REGULAR_NAME.to_owned()]
        );
        // 言語のフォントが無いときは日本語・英語の並びのまま
        assert_eq!(definitions_with(None).families, definitions().families);
    }

    /// 開発の置き場（`cargo xtask fetch-fonts` が置く）にある言語のフォントのファイルの中身。無ければ理由を出して None。
    fn real_font(name: &str) -> Option<Vec<u8>> {
        let path = dev_font_dir().join(name);
        match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                eprintln!(
                    "飛ばす: 言語のフォント {} がありません（cargo xtask fetch-fonts）: {e}",
                    path.display()
                );
                None
            }
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// アプリの言語のフォントのファイル名は、`cargo xtask fetch-fonts` と同じ表（`yolu_update::fonts`）にある。
    #[test]
    fn the_language_fonts_are_in_the_shared_table() {
        for font in [&NOTO_SANS_TC, &NOTO_SANS_SC, &NOTO_SANS_KR] {
            for file in [font.regular, font.bold] {
                assert!(yolu_update::fonts::find(file).is_some(), "{file}");
            }
        }
        assert_eq!(yolu_update::fonts::LANG_FONTS.len(), 6);
    }

    /// 言語のフォントを読む: 無いときは見つからない、フォントでないファイルと 1 バイト違うファイルは断る（落ちない）、
    /// 太字が無い・合わないときは普通の太さで代える。
    #[test]
    fn loading_a_language_font_refuses_missing_and_broken_files() {
        let dir = temp_dir("yolu-lang-font");
        let dirs = vec![dir.join("none"), dir.clone()];
        let missing = load(&NOTO_SANS_TC, &dirs).err().unwrap();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
        std::fs::write(dir.join(NOTO_SANS_TC.regular), b"not a font at all").unwrap();
        let broken = load(&NOTO_SANS_TC, &dirs).err().unwrap();
        assert_eq!(broken.kind(), std::io::ErrorKind::InvalidData);
        if let (Some(regular), Some(bold)) = (
            real_font(NOTO_SANS_TC.regular),
            real_font(NOTO_SANS_TC.bold),
        ) {
            // 1 バイトだけ違う（大きさは同じ。頭は sfnt のまま）
            let mut flipped = regular.clone();
            *flipped.last_mut().unwrap() ^= 1;
            std::fs::write(dir.join(NOTO_SANS_TC.regular), &flipped).unwrap();
            let changed = load(&NOTO_SANS_TC, &dirs).err().unwrap();
            assert_eq!(changed.kind(), std::io::ErrorKind::InvalidData);
            // 切れたファイル
            std::fs::write(
                dir.join(NOTO_SANS_TC.regular),
                &regular[..regular.len() - 1],
            )
            .unwrap();
            assert!(load(&NOTO_SANS_TC, &dirs).is_err());
            // 合うファイル。太字が無ければ普通の太さ、太字が合わなければ同じく普通の太さ
            std::fs::write(dir.join(NOTO_SANS_TC.regular), &regular).unwrap();
            let loaded = load(&NOTO_SANS_TC, &dirs).unwrap();
            assert_eq!(loaded.regular.font.len(), regular.len());
            assert!(
                Arc::ptr_eq(&loaded.regular, &loaded.bold),
                "太字が無ければ普通の太さ"
            );
            let mut flipped_bold = bold.clone();
            flipped_bold[100] ^= 1;
            std::fs::write(dir.join(NOTO_SANS_TC.bold), &flipped_bold).unwrap();
            let loaded = load(&NOTO_SANS_TC, &dirs).unwrap();
            assert!(
                Arc::ptr_eq(&loaded.regular, &loaded.bold),
                "太字が合わなければ普通の太さ"
            );
            std::fs::write(dir.join(NOTO_SANS_TC.bold), &bold).unwrap();
            let loaded = load(&NOTO_SANS_TC, &dirs).unwrap();
            assert!(!Arc::ptr_eq(&loaded.regular, &loaded.bold));
            // 定義へは同じ `Arc` を渡す（バイト列を写さない）
            let fonts = definitions_with(Some(&loaded));
            assert!(Arc::ptr_eq(
                &fonts.font_data["noto-sans-tc-regular"],
                &loaded.regular
            ));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// フォントの定義 `fonts` の普通と太字の並びに無い字。
    fn missing_glyphs(
        fonts: FontDefinitions,
        chars: &std::collections::BTreeSet<char>,
    ) -> Vec<(FontFamily, char)> {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts);
        let mut missing = Vec::new();
        // フォントは次のパスの初めに効く。2 回目に見る
        for pass in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                if pass == 0 {
                    return;
                }
                ctx.fonts_mut(|f| {
                    for family in [FontFamily::Proportional, FontFamily::Name(BOLD.into())] {
                        let id = egui::FontId::new(14.0, family.clone());
                        for c in chars {
                            if !f.has_glyph(&id, *c) {
                                missing.push((family.clone(), *c));
                            }
                        }
                    }
                });
            });
            output.textures_delta.clear();
        }
        missing
    }

    /// 言語の .ftl の全部の字が、その言語のフォントの並び（普通と太字）のどれかにある。言語のフォントのファイルが無い環境
    /// （`cargo xtask fetch-fonts` の前）は、理由を出して飛ばす。
    #[test]
    fn every_character_of_a_language_has_a_glyph_in_its_fonts() {
        for lang in Lang::ALL {
            let Some(font) = lang.info().font else {
                continue;
            };
            let loaded = match load(font, &[dev_font_dir()]) {
                Ok(loaded) => loaded,
                Err(e) => {
                    eprintln!(
                        "飛ばす: {} の言語のフォント {} が {} にありません（cargo xtask fetch-fonts）: {e}",
                        lang.tag(),
                        font.regular,
                        dev_font_dir().display()
                    );
                    continue;
                }
            };
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("locales")
                .join(lang.tag());
            let mut chars = std::collections::BTreeSet::new();
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let text = std::fs::read_to_string(entry.path()).unwrap();
                for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
                    chars.extend(
                        line.chars()
                            .filter(|c| !c.is_whitespace() && !c.is_control()),
                    );
                }
            }
            assert!(chars.len() > 500, "{} の字 {}", lang.tag(), chars.len());
            let missing = missing_glyphs(definitions_with(Some(&loaded)), &chars);
            // 測り方の確かめ: 同梱のフォントだけでは足りない字がある（言語のフォントが効いている）
            assert!(
                !missing_glyphs(definitions(), &chars).is_empty(),
                "{}: 同梱のフォントだけで全部の字がある",
                lang.tag()
            );
            assert!(
                missing.is_empty(),
                "{} で字の無い物: {missing:?}",
                lang.tag()
            );
        }
    }

    /// 言語を替えると、その言語のフォントを裏で読んで入れる。日本語へ戻すと同梱の並びへ戻し、もう一度選ぶと読み直さずに入れる。
    /// 読めないときは知らせの文を 1 回だけ返し、並びは日本語・英語のまま。
    #[test]
    fn language_fonts_are_loaded_in_the_background_and_a_failure_is_told_once() {
        let settle = |fonts: &mut LangFonts, ctx: &egui::Context, lang: Lang| {
            let mut told = Vec::new();
            for _ in 0..500 {
                told.extend(fonts.sync(ctx, lang));
                if !fonts.is_loading() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            told.extend(fonts.sync(ctx, lang));
            told
        };
        let ctx = egui::Context::default();
        let dir = temp_dir("yolu-lang-fonts");
        // 無いとき: 知らせは 1 回、並びは同梱のまま
        let mut fonts = LangFonts::with_dirs(vec![dir.clone()]);
        let told = settle(&mut fonts, &ctx, Lang::ZhTw);
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains(NOTO_SANS_TC.regular), "{told:?}");
        assert_eq!(fonts.installed(), None);
        assert!(
            settle(&mut fonts, &ctx, Lang::ZhTw).is_empty(),
            "知らせは 1 回"
        );
        // 壊れているとき（1 バイト違う）: 落ちずに知らせ 1 回、並びは同梱のまま
        let Some(real) = real_font(NOTO_SANS_TC.regular) else {
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        };
        let mut flipped = real.clone();
        flipped[1000] ^= 1;
        std::fs::write(dir.join(NOTO_SANS_TC.regular), &flipped).unwrap();
        let mut fonts = LangFonts::with_dirs(vec![dir.clone()]);
        let told = settle(&mut fonts, &ctx, Lang::ZhTw);
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!(fonts.installed(), None);
        // あるとき: 入る。日本語で外れ、戻すと読み直さずに入る
        std::fs::write(dir.join(NOTO_SANS_TC.regular), &real).unwrap();
        let mut fonts = LangFonts::with_dirs(vec![dir.clone()]);
        assert!(settle(&mut fonts, &ctx, Lang::ZhTw).is_empty());
        assert_eq!(fonts.installed(), Some(&NOTO_SANS_TC));
        assert!(settle(&mut fonts, &ctx, Lang::Ja).is_empty());
        assert_eq!(fonts.installed(), None);
        assert!(fonts.sync(&ctx, Lang::ZhTw).is_none());
        assert!(!fonts.is_loading(), "読んだ物を覚えている");
        assert_eq!(fonts.installed(), Some(&NOTO_SANS_TC));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
