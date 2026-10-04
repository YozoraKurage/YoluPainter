//! 画面の外枠の決まり（ユーザーの方針）: 状態の帯には直前の操作の結果と理由だけを出す（文書の大きさ・メモリ・上げたタイル・合成の方式・
//! Live Link の様子は出さない）。2D と 3D のビューに見出しの帯は置かず、その高さをビューに返す。帯にあった操作は、ビューの右上の隅に
//! 重ねた小さなアイコン（文字なし。名前はツールチップ）へ。開発用の数は、画面に出さずに状態（AppState）から試験で読む。
mod common;

use common::*;
use egui::epaint::Shape;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::ui::theme as t;
use yolu_app::{shell, Tab, YoluApp};

/// 描いた文字を全部集める。
fn texts(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

fn screen_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        texts(&shape.shape, &mut out);
    }
    out
}

/// 開発用の数・言葉（メモリの量・上げたタイル・合成の方式・三角形の数・ビューの見出しの帯）。
const DEVELOPER: [&str; 13] = [
    "MiB",
    "KiB",
    "上げたタイル",
    "Uploaded tiles",
    "CPU で合成",
    "CPU compositing",
    "三角形",
    "triangles",
    "試しの立方体",
    "Test Cube",
    "2D ·",
    "3D ·",
    "テクスチャセット 1 ·",
];

fn assert_no_developer_text(h: &Harness<'_, YoluApp>, what: &str) {
    let texts = screen_texts(h);
    for text in &texts {
        for word in DEVELOPER {
            assert!(
                !text.contains(word),
                "{what}: 画面に開発用の言葉「{word}」: {text}\n全部: {texts:#?}"
            );
        }
    }
}

#[test]
fn the_status_bar_shows_only_the_last_message_and_no_developer_numbers() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 512);
        h.state_mut().state.lang = lang;
        h.state_mut().apply(Action::LoadDemoModel);
        h.state_mut().state.message = String::new();
        h.run();
        // 描く・Live Link を待つ・3D を出す、のあとでも、状態の帯の文字は message だけ
        let c = canvas_rect(&h);
        drag(&mut h, &[offset(c.center(), -40.0, 0.0), offset(c.center(), 40.0, 10.0)]);
        h.state_mut().state.message = String::new();
        h.run();
        assert_eq!(shell::status_text(&h.state().state), "");
        assert_no_developer_text(&h, &format!("{lang:?}"));

        // 帯の画素は、message が空なら、背景と上の縁の線だけ（何も描かない）
        let image = h.render().expect("描画");
        let (w, hh) = (image.width(), image.height());
        let bar_top = hh - t::STATUS_BAR_HEIGHT as u32;
        let background = *image.get_pixel(w / 2, hh - 4);
        for y in bar_top + 2..hh {
            for x in 0..w {
                assert_eq!(
                    *image.get_pixel(x, y),
                    background,
                    "{lang:?}: 状態の帯の ({x}, {y}) に何かが描かれている"
                );
            }
        }

        // message があれば、それだけが出る
        h.state_mut().state.message = "試験の知らせ".into();
        h.run();
        assert_eq!(shell::status_text(&h.state().state), "試験の知らせ");
        assert!(screen_texts(&h).iter().any(|t| t == "試験の知らせ"));

        // 画面に出さなくなった数は、状態から読める
        let s = h.state();
        assert!(s.display().stats.total_tiles > 0);
        assert!(s.state.doc.allocated_bytes() > 0);
        assert_eq!(s.state.doc.width(), 512);
    }
}

#[test]
fn the_views_have_no_header_band_and_get_its_height_back() {
    let mut h = app(1280.0, 800.0, 512);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.message = String::new(); // 読んだという知らせは状態の帯に出る（見出しの帯の名前とは別）
    h.run();
    // キャンバス: 表示域はタブの帯のすぐ下から始まる（見出しの帯の分だけ下がらない）
    let strip = h.state().tab_rects[&Tab::Canvas];
    let canvas = canvas_rect(&h);
    assert!(
        canvas.top() - strip.bottom() <= 2.0,
        "キャンバスの上 {} とタブの帯の下 {}",
        canvas.top(),
        strip.bottom()
    );
    assert_no_developer_text(&h, "キャンバス");
    // 3D ビュー
    click_tab(&mut h, Tab::View3d);
    let strip = h.state().tab_rects[&Tab::View3d];
    let view = h.state().view3d_rect().expect("3D ビューが前");
    assert!(
        view.top() - strip.bottom() <= 2.0,
        "3D の上 {} とタブの帯の下 {}",
        view.top(),
        strip.bottom()
    );
    assert_no_developer_text(&h, "3D ビュー");
}

#[test]
fn the_3d_corner_icon_sits_at_the_top_right_and_is_not_a_paint_surface() {
    let mut h = app(1280.0, 800.0, 512);
    h.state_mut().apply(Action::LoadDemoModel);
    click_tab(&mut h, Tab::View3d);
    h.run();
    let view = h.state().view3d_rect().expect("3D ビュー");
    let icon = h.get_by_label("モデル全体が見える位置へ戻す").rect();
    assert!(
        view.expand(1.0).contains_rect(icon) && view.right() - icon.right() < 24.0 && icon.top() - view.top() < 24.0,
        "{icon:?} は {view:?} の右上の隅"
    );
    // アイコンを押しても描き始めない（下のビューの入力へ通さない）
    let before = h.state().state.doc.can_undo();
    let at = icon.center();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    assert!(!h.state().state.is_stroking(), "隅のアイコンの上で描き始めた");
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.can_undo(), before, "文書は変わらない");
}

#[test]
fn the_2d_corner_icons_appear_only_when_the_state_needs_them() {
    let mut h = app(1280.0, 800.0, 512);
    // 何も変えていなければ、隅にアイコンは無い
    assert!(h.query_by_label("表示を回しています（-15°）。押すと回転を戻します（Shift+R）").is_none());
    key(&h, Key::Minus, Modifiers::NONE);
    h.run();
    let canvas = canvas_rect(&h);
    let label = "表示を回しています（-15°）。押すと回転を戻します（Shift+R）";
    let icon = h.get_by_label(label).rect();
    assert!(
        canvas.expand(1.0).contains_rect(icon) && canvas.right() - icon.right() < 24.0 && icon.top() - canvas.top() < 24.0,
        "{icon:?} は {canvas:?} の右上の隅"
    );
    // 押しても描き始めず、回転が戻る
    let at = icon.center();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    assert!(!h.state().state.is_stroking());
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.view.angle, 0.0);
    assert!(!h.state().state.doc.can_undo(), "アイコンの上では描かない");
    assert!(h.query_by_label(label).is_none(), "戻したら隅のアイコンも消える");
}

/// ユーザーが名指しで「余計な文章やめろ」と言った文言（2026-10-04）。どの状態・言語・タブでも、画面に出ない。
/// 上げたタイル（状態の帯）、3D・試験・三角形 / 2D・テクスチャセット・カラー 100%（見出しの帯）、まだマテリアルに付いていない（テクスチャセットの
/// 下の帯）、メイン #000000 / サブ #FFFFFF（カラーの欄の左下の文字）。
const NAMED: [&str; 14] = [
    "上げたタイル",
    "Uploaded tiles",
    "三角形",
    "triangles",
    "2D ·",
    "3D ·",
    "付いていない",
    "未割り当て",
    "Unassigned",
    "メイン  #",
    "サブ  #",
    "Main  #",
    "Sub  #",
    "カラー 100%",
];

#[test]
fn the_texts_the_user_named_never_appear_on_screen() {
    for lang in Lang::ALL {
        for model in [false, true] {
            for tab in [Tab::Canvas, Tab::View3d] {
                let mut h = app(1280.0, 800.0, 512);
                h.state_mut().state.lang = lang;
                if model {
                    h.state_mut().apply(Action::LoadDemoModel);
                }
                click_tab(&mut h, tab);
                // 描いたあとと、知らせを消したあと
                if tab == Tab::Canvas {
                    let c = canvas_rect(&h);
                    drag(&mut h, &[offset(c.center(), -30.0, 0.0), offset(c.center(), 30.0, 5.0)]);
                }
                h.state_mut().state.message = String::new();
                h.run();
                let texts = screen_texts(&h);
                for text in &texts {
                    for named in NAMED {
                        assert!(
                            !text.contains(named),
                            "{lang:?} モデル={model} {tab:?}: 画面に名指しされた文言「{named}」: {text}\n全部: {texts:#?}"
                        );
                    }
                }
                // 色の欄の左下は色の四角だけ（メインとサブの値は、四角のツールチップと 16 進の欄）。16 進の欄の「#000000」は 1 つだけ
                assert_eq!(texts.iter().filter(|t| t.starts_with('#')).count(), 1, "{texts:?}");
                // テクスチャセットの行は、名前・（状態のアイコン）・解像度だけ。下に帯が無い（行の下の余白は空）
                assert!(texts.iter().any(|t| t == "512"), "解像度は行に出る");
            }
        }
    }
}
