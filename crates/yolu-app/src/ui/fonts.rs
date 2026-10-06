//! 画面の書体。BIZ UDPGothic（Regular と Bold。SIL OFL 1.1。全文は `assets/fonts/OFL.txt`）を実行ファイルに同梱し、全 OS で同じ書体を使う。
//! OS の書体を読む方式は、Windows の游ゴシックが縦の寸法の癖で文字を上に寄せ、かなを広げて見せたのでやめた。
//!
//! 英数字も同じ書体で揃える（egui の既定の Ubuntu Light と組まない）: 同じ書体なら、かなと英字の高さと太さが揃い、行の中で字の
//! 大きさがばらつかない。egui の既定の書体は、この書体に無い記号（絵文字など）を補う後ろ盾として末尾に残す。
//! 縦の位置は書体の寸法（ascent 1802・descent 246・1 em = 2048）のとおりで、行の高さの真ん中が漢字の字面の真ん中に合うので、
//! 補正（`FontTweak`）は入れない（`tests/gui_shell/fonts.rs` が字面の真ん中を測って確かめる）。

use egui::{FontData, FontDefinitions, FontFamily};

use super::theme::BOLD;

/// 普通の太さ。
const REGULAR: &[u8] = include_bytes!("../../assets/fonts/BIZUDPGothic-Regular.ttf");
/// 太字。
const BOLD_FACE: &[u8] = include_bytes!("../../assets/fonts/BIZUDPGothic-Bold.ttf");

/// 同梱した書体の名前（`FontDefinitions::font_data` の鍵）。
pub const REGULAR_NAME: &str = "biz-udpgothic-regular";
pub const BOLD_NAME: &str = "biz-udpgothic-bold";

/// 書体の定義（普通・太字を先頭に置き、egui の既定の書体を後ろに残す）。
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert(REGULAR_NAME.into(), FontData::from_static(REGULAR).into());
    fonts
        .font_data
        .insert(BOLD_NAME.into(), FontData::from_static(BOLD_FACE).into());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, REGULAR_NAME.into());
    // 均等幅は既定の書体（Hack）のまま、日本語だけ同梱の書体で補う
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push(REGULAR_NAME.into());
    // 太字: 太字 → 普通 → 既定（記号の補い）
    let mut bold = vec![BOLD_NAME.to_owned()];
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

    #[test]
    fn the_bundled_faces_come_first_and_the_defaults_stay_as_a_fallback() {
        let fonts = definitions();
        let proportional = &fonts.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], REGULAR_NAME);
        assert!(proportional.len() > 1, "既定の書体を後ろ盾に残す");
        let bold = &fonts.families[&FontFamily::Name(BOLD.into())];
        assert_eq!(bold[0], BOLD_NAME);
        assert_eq!(bold[1], REGULAR_NAME);
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
}
