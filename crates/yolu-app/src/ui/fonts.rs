//! 画面の書体。BIZ UDPGothic（Regular と Bold。SIL OFL 1.1。全文は `assets/fonts/OFL.txt`）を実行ファイルに同梱し、全 OS で同じ書体を使う。
//! OS の書体を読む方式は、Windows の游ゴシックが縦の寸法の癖で文字を上に寄せ、かなを広げて見せたのでやめた。
//!
//! 英数字も同じ書体で揃える（egui の既定の Ubuntu Light と組まない）: 同じ書体なら、かなと英字の高さと太さが揃い、行の中で字の
//! 大きさがばらつかない。egui の既定の書体は、この書体に無い記号（絵文字など）を補う後ろ盾として末尾に残す。
//! 縦の位置は書体の寸法（ascent 1802・descent 246・1 em = 2048）のとおりで、行の高さの真ん中が漢字の字面の真ん中に合うので、
//! 補正（`FontTweak`）は入れない（`tests/gui_shell/fonts.rs` が字面の真ん中を測って確かめる）。
//!
//! 下線（_）・ダッシュ（– — ―）・マイナス（−）・上線（‾ ¯）などは、原本のヒンティングの命令が、細い棒（0.3〜0.6 画素）の上下の縁を別々に
//! 画素の格子へ丸め、両方が同じ線に落ちて厚み 0 になる大きさ（12 pt の `_` など）があり、egui は箱の高さが 0 の字を描かない。
//! `FontTweak` の `hinting_target` はこのフォント（命令を持つ）には効かず、`hinting` を切ると全部の字がぼやけるので、消えるグリフだけを
//! 命令なしで抜き出した小さなフォント（`*-Lines.ttf`。作り方は `tools/make-ui-font-lines.py`）を、普通・太字のそれぞれの先頭に置く。
//! 抜粋は縦の寸法が原本と同じで、egui が先頭のフォントの寸法で決める行の高さは変わらない。

use egui::{FontData, FontDefinitions, FontFamily};

use super::theme::BOLD;

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

/// 同梱した書体の名前（`FontDefinitions::font_data` の鍵）。
pub const REGULAR_NAME: &str = "biz-udpgothic-regular";
pub const BOLD_NAME: &str = "biz-udpgothic-bold";
pub const REGULAR_LINES_NAME: &str = "biz-udpgothic-regular-lines";
pub const BOLD_LINES_NAME: &str = "biz-udpgothic-bold-lines";

/// フォントの定義（細い字の抜粋・普通・太字を先頭に置き、egui の既定のフォントを後ろに残す）。
pub fn definitions() -> FontDefinitions {
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
    // 抜粋が先頭: 同じ字を持つフォントのうち先のものが使われる
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .splice(
            0..0,
            [REGULAR_LINES_NAME.to_owned(), REGULAR_NAME.to_owned()],
        );
    // 均等幅は既定の書体（Hack）のまま、日本語だけ同梱の書体で補う
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push(REGULAR_NAME.into());
    // 太字: 太字の抜粋 → 太字 → 普通の抜粋 → 普通 → 既定（記号の補い）
    let mut bold = vec![BOLD_LINES_NAME.to_owned(), BOLD_NAME.to_owned()];
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

/// 書体を入れる。
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
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
        // 均等幅は既定の書体が先で、日本語だけ同梱の書体で補う
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
}
