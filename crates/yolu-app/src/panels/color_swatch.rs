//! メインの色（描画色）とサブの色（背景色）の 2 枚。左のツールの帯の一番下の端（最後のツールのすぐ下ではなく、帯の下に付ける。
//! Photoshop のツールの帯の色と同じ位置）に置き、描画色が左上・背景色が右下に重なる。右上に入れ替え（X）、左下に初期設定（D）の小さなボタン。
//! 背景色の四角を押しても入れ替わる。帯の幅（アイコンの幅）に収め、ツールチップと X・D のキーは変えない。

use egui::{pos2, vec2, Rect, Ui};

use crate::state::{to_hex, Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 2 枚の組の幅（帯の幅からアイコンの左右の余白を引いた幅）と高さ。
pub const BLOCK_WIDTH: f32 = 34.0;
pub const BLOCK_HEIGHT: f32 = 42.0;
/// 帯の下の端から組までの余白。
const BOTTOM_GAP: f32 = 6.0;
/// 組の上と最後のツールの間に空ける高さ。
const TOP_GAP: f32 = 4.0;

/// 2 枚の組が帯の下の端に付くとき、ツールのボタンの並びから先に取っておく高さ。
pub fn reserved_height() -> f32 {
    BLOCK_HEIGHT + BOTTOM_GAP + TOP_GAP
}

/// ツールの帯 `strip` の下の端に付く、2 枚の組の場所（帯の中で左右の中央）。
pub fn area(strip: Rect) -> Rect {
    let width = BLOCK_WIDTH.min((strip.width() - 10.0).max(0.0));
    Rect::from_min_size(
        pos2(
            strip.center().x - width * 0.5 - 0.5,
            strip.bottom() - BOTTOM_GAP - BLOCK_HEIGHT,
        ),
        vec2(width, BLOCK_HEIGHT),
    )
}

/// 組の中の 4 つの部品の場所（描画色・背景色・入れ替え・初期設定）。
#[derive(Clone, Copy, Debug)]
pub struct Parts {
    pub main: Rect,
    pub sub: Rect,
    pub swap: Rect,
    pub default: Rect,
}

pub fn parts(area: Rect) -> Parts {
    let at =
        |x: f32, y: f32, size: f32| Rect::from_min_size(area.min + vec2(x, y), vec2(size, size));
    Parts {
        main: at(0.0, 4.0, 20.0),
        sub: at(14.0, 17.0, 20.0),
        swap: at(21.0, 0.0, 13.0),
        default: at(0.0, 28.0, 13.0),
    }
}

/// 2 枚の色と、入れ替え・初期設定のボタンを `area`（`area()` が返す場所）に描く。
pub fn draw(ui: &mut Ui, app: &mut AppState, area: Rect) {
    let lang = app.lang;
    let parts = parts(area);
    if w::color_swatch(
        ui,
        parts.sub,
        "color.sub",
        app.color.sub,
        &format!(
            "{} #{}",
            lang.pick(
                "サブの色（背景色）。押すとメインの色と入れ替えます",
                "Background color. Click to swap with the foreground color."
            ),
            to_hex(app.color.sub)
        ),
        true,
    )
    .clicked()
    {
        app.apply(Action::SwapColors);
    }
    w::fill(ui.painter(), parts.main.expand(1.0), t::PANEL_BG);
    let _ = w::color_swatch(
        ui,
        parts.main,
        "color.main",
        app.color.main,
        &format!(
            "{} #{}",
            lang.pick(
                "メインの色（描画色。ブラシで塗る色）",
                "Foreground color (the color the brush paints)"
            ),
            to_hex(app.color.main)
        ),
        true,
    );
    if w::icon_button(
        ui,
        parts.swap,
        "color.swap",
        "swap_horiz",
        &swap_tip(lang, app.mode),
        false,
        true,
        12.0,
    )
    .clicked()
    {
        app.apply(Action::SwapColors);
    }
    if w::icon_button(
        ui,
        parts.default,
        "color.default",
        "restart_alt",
        &default_tip(lang, app.mode),
        false,
        true,
        11.0,
    )
    .clicked()
    {
        app.apply(Action::DefaultColors);
    }
}

/// 入れ替えのボタンのツールチップ（キーは今の割り当て）。
pub fn swap_tip(lang: crate::lang::Lang, mode: crate::mode::EditorMode) -> String {
    crate::shortcuts::named_with_keys(
        lang,
        lang.pick(
            "メインとサブの色を入れ替え",
            "Swap foreground and background colors",
        ),
        &[crate::shortcuts::key_in("color.swap", mode)],
    )
}

/// 初期設定の色のボタンのツールチップ（キーは今の割り当て）。
pub fn default_tip(lang: crate::lang::Lang, mode: crate::mode::EditorMode) -> String {
    crate::shortcuts::named_with_keys(
        lang,
        lang.pick("初期設定の色", "Default colors"),
        &[crate::shortcuts::key_in("color.default", mode)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_sits_at_the_bottom_of_the_strip_and_the_parts_stay_inside_it() {
        let strip = Rect::from_min_size(
            pos2(0.0, 60.0),
            vec2(crate::ui::theme::TOOL_STRIP_WIDTH, 500.0),
        );
        let block = area(strip);
        assert!(strip.contains_rect(block), "{block:?}");
        assert_eq!(block.bottom(), strip.bottom() - BOTTOM_GAP);
        assert!(block.width() <= strip.width() - 10.0);
        let p = parts(block);
        for part in [p.main, p.sub, p.swap, p.default] {
            assert!(block.expand(0.01).contains_rect(part), "{part:?} {block:?}");
        }
        // 入れ替えと初期設定は、2 枚の色に重ならない（メインとサブだけが重なる）
        let overlap = |a: Rect, b: Rect| {
            let i = a.intersect(b);
            i.width() > 0.0 && i.height() > 0.0
        };
        assert!(overlap(p.main, p.sub));
        for button in [p.swap, p.default] {
            assert!(!overlap(button, p.main) && !overlap(button, p.sub));
        }
        assert!(!overlap(p.swap, p.default));
        // ツールのボタンの並びから先に取る高さは、組と下の余白と上の間の合計
        assert_eq!(reserved_height(), BLOCK_HEIGHT + BOTTOM_GAP + TOP_GAP);
    }
}
