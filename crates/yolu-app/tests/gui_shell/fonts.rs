//! 画面の書体（同梱の BIZ UDPGothic）: 文字が欄の縦の真ん中に来る。Windows の游ゴシックは縦の寸法の癖で文字が上に寄り、かなが
//! 広がって見えたので、全 OS で同じ書体を同梱した。描いた画素の字面の縦の中心を測って、行の矩形の真ん中との差を確かめる
//! （日本語・英字の大文字・数字、普通と太字、いくつかの大きさ、画面の拡大 1.0・1.5・2.0）。
use crate::common;

use egui::{pos2, vec2, Color32, Rect};
use yolu_app::ui::theme::{self as t, TextStyle};
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::YoluApp;

struct Sample {
    ready: bool,
    rows: Vec<(Rect, String)>,
    style: TextStyle,
}

/// 行の高さ（メニューバーの行と同じ 24 点）と、行どうしの間。
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;

/// 文字を 1 つずつ別の行の矩形の中に描き、行ごとに、字面（背景と違う画素）の縦の中心と、行の矩形の真ん中との差（点）を返す。
/// 1 つの窓に全部の行を描くので、組ごとに窓を作らない。
fn center_offsets(texts: &[&str], style: TextStyle, ppp: f32) -> Vec<f32> {
    let rows: Vec<(Rect, String)> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let top = 10.0 + i as f32 * (ROW + GAP);
            (
                Rect::from_min_size(pos2(20.0, top), vec2(180.0, ROW)),
                (*text).to_owned(),
            )
        })
        .collect();
    let height = 20.0 + texts.len() as f32 * (ROW + GAP);
    let sample = Sample {
        ready: false,
        rows: rows.clone(),
        style,
    };
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(220.0, height))
        .with_pixels_per_point(ppp)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            |ui, s: &mut Sample| {
                if !s.ready {
                    // 書体は次のフレームから効く
                    YoluApp::setup(ui.ctx());
                    s.ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                let area = ui.max_rect();
                ui.allocate_rect(area, egui::Sense::hover());
                let p = ui.painter().clone();
                p.rect_filled(area, 0.0, t::PANEL_BG);
                for (rect, text) in &s.rows {
                    w::text(&p, *rect, text, s.style, Align::Left);
                }
            },
            sample,
        );
    h.run();
    let image = h.render().expect("描画");
    let background = t::PANEL_BG;
    let differs = |x: u32, y: u32| {
        let p = image.get_pixel(x, y).0;
        let d = (p[0] as i32 - background.r() as i32).abs()
            + (p[1] as i32 - background.g() as i32).abs()
            + (p[2] as i32 - background.b() as i32).abs();
        d > 90
    };
    rows.iter()
        .map(|(rect, text)| {
            let (mut top, mut bottom) = (u32::MAX, 0u32);
            let r = (
                (rect.left() * ppp) as u32,
                (rect.top() * ppp) as u32,
                (rect.right() * ppp) as u32,
                (rect.bottom() * ppp) as u32,
            );
            for y in r.1..r.3.min(image.height()) {
                for x in r.0..r.2.min(image.width()) {
                    if differs(x, y) {
                        top = top.min(y);
                        bottom = bottom.max(y);
                    }
                }
            }
            assert!(top != u32::MAX, "字面が無い: {text}");
            // 点の単位の上端・下端（下端は画素の下の縁）
            let (top, bottom) = (top as f32 / ppp, (bottom + 1) as f32 / ppp);
            (top + bottom) * 0.5 - rect.center().y
        })
        .collect()
}

fn regular(size: f32) -> TextStyle {
    TextStyle {
        size,
        bold: false,
        color: Color32::WHITE,
    }
}

fn bold(size: f32) -> TextStyle {
    TextStyle {
        size,
        bold: true,
        color: Color32::WHITE,
    }
}

/// 日本語（漢字とかな。字面の上下は書体の中の位置で少し違うので、複数の字で）。
const JAPANESE: [&str; 4] = ["漢字", "ブラシ", "あいう", "選択範囲"];
/// 英字の大文字と数字。
const CAPITALS_AND_DIGITS: [&str; 4] = ["HEX", "ABC", "100", "0123"];

fn assert_centered(texts: &[&str]) {
    for ppp in [1.0, 1.5, 2.0] {
        for size in [10.0, 11.0, 12.0, 14.0] {
            for (what, style) in [("普通", regular(size)), ("太字", bold(size))] {
                let offsets = center_offsets(texts, style, ppp);
                for (text, offset) in texts.iter().zip(offsets) {
                    assert!(
                        offset.abs() <= 1.0,
                        "{text}（{what} {size} pt・拡大 {ppp}）の字面の縦の中心が行の真ん中から {offset} 点ずれている"
                    );
                }
            }
        }
    }
}

#[test]
fn japanese_text_sits_in_the_vertical_middle_of_the_row() {
    assert_centered(&JAPANESE);
}

#[test]
fn english_capitals_and_digits_sit_in_the_vertical_middle_of_the_row() {
    assert_centered(&CAPITALS_AND_DIGITS);
}

#[test]
fn lowercase_english_sits_a_little_below_the_middle_like_any_typeface_but_not_far() {
    // 小文字だけの語は、x の高さの分だけ字面が下に寄る（どの書体も同じ）。それでも 2 点以内
    for ppp in [1.0, 2.0] {
        for size in [11.0, 12.0] {
            let offset = center_offsets(&["xaecnomu"], regular(size), ppp)[0];
            assert!(offset > -0.5 && offset <= 2.0, "小文字（{size} pt・拡大 {ppp}）: {offset} 点");
        }
    }
}

#[test]
fn japanese_and_english_text_share_one_line_height() {
    // 英字だけの行と日本語だけの行の高さが同じ（同じ書体なので、混ぜても行がばらつかない）
    #[derive(Default)]
    struct Heights {
        ready: bool,
        ja: f32,
        en: f32,
        mixed: f32,
    }
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(100.0, 40.0))
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            |ui, s: &mut Heights| {
                if !s.ready {
                    YoluApp::setup(ui.ctx());
                    s.ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                let height = |text: &str| {
                    ui.painter()
                        .layout_no_wrap(text.to_owned(), t::LABEL.font(), Color32::WHITE)
                        .size()
                        .y
                };
                (s.ja, s.en, s.mixed) = (height("漢字かな"), height("HEX abc"), height("漢字 HEX"));
            },
            Heights::default(),
        );
    h.run();
    let s = h.state();
    assert!(s.ja > 0.0);
    assert!((s.ja - s.en).abs() < 0.01 && (s.ja - s.mixed).abs() < 0.01, "{} {} {}", s.ja, s.en, s.mixed);
}
