//! テキストツール（テキストレイヤー）: 押して打つ・打ち直す・打った分が 1 回の取り消し・値の欄・移動・変形で値が動く・フォントが無いときの断り・日英。
//! `headless_` で始まる試験は画面を描かない。画面の試験はウィンドウで打つ。`YOLU_TEXT_SHOTS=<フォルダ>` を付けると、打っている所と欄の絵（日英）を
//! そのフォルダへ書く（確かめ用。試験の結果には使わない）。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, Rect};
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::textlayer::{ColorSource, Field, TextAction};
use yolu_app::YoluApp;
use yolu_core::text::{TextAlign, TextFont, TextSettings};
use yolu_core::{Affine2D, LayerId, LayerLocks, Rgba8};

fn state() -> AppState {
    let mut s = AppState::new(256, 256);
    s.apply(Action::SelectTool(Tool::Text));
    s
}

fn text_of(s: &AppState, id: LayerId) -> Option<TextSettings> {
    s.doc.layer(id).and_then(|l| l.text()).cloned()
}

fn editing_layer(s: &AppState) -> Option<LayerId> {
    s.text.editing.as_ref().and_then(|e| e.layer)
}

#[test]
fn headless_typing_a_new_text_is_one_undo_and_an_empty_text_leaves_nothing() {
    let mut s = state();
    let layers = s.doc.layers().len();
    let undo = s.doc.undo_count();
    s.apply(Action::Text(TextAction::Begin { x: 20.0, y: 200.0 }));
    assert!(s.text.editing.is_some());
    // 何も打たないうちはレイヤーを作らない
    assert_eq!(s.doc.layers().len(), layers);
    for text in ["H", "He", "Hello", "Hello\n文字"] {
        s.text_typed(text.into());
    }
    let id = editing_layer(&s).expect("最初の 1 文字でレイヤーを作る");
    assert_eq!(s.selected_layer, Some(id));
    s.apply(Action::Text(TextAction::Commit));
    assert!(s.text.editing.is_none());
    let t = text_of(&s, id).unwrap();
    assert_eq!(t.text, "Hello\n文字");
    assert_eq!((t.x, t.y), (20.0, 200.0));
    assert_eq!(s.doc.undo_count(), undo + 1, "打った分は 1 回の取り消し");
    s.apply(Action::Undo);
    assert!(s.doc.layer(id).is_none());
    s.apply(Action::Redo);
    assert_eq!(text_of(&s, id).unwrap().text, "Hello\n文字");
    // 打って全部消してから終えると、レイヤーは残らない
    let undo = s.doc.undo_count();
    let layers = s.doc.layers().len();
    s.apply(Action::Text(TextAction::Begin { x: 5.0, y: 50.0 }));
    s.text_typed("x".into());
    s.text_typed(String::new());
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(s.doc.layers().len(), layers);
    assert_eq!(s.doc.undo_count(), undo);
}

#[test]
fn headless_retyping_a_text_layer_and_changing_values_redraws_it() {
    let mut s = state();
    s.apply(Action::Text(TextAction::Begin { x: 10.0, y: 120.0 }));
    s.text_typed("abc".into());
    let id = editing_layer(&s).unwrap();
    s.apply(Action::Text(TextAction::Commit));
    let undo = s.doc.undo_count();
    s.apply(Action::Text(TextAction::Edit(id)));
    s.text_typed("abcd".into());
    s.text_typed("abcde".into());
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(text_of(&s, id).unwrap().text, "abcde");
    assert_eq!(s.doc.undo_count(), undo + 1);
    // 選んでいるテキストレイヤーの値（1 回の取り消しで描き直す）
    s.apply(Action::Text(TextAction::Set(Field::Size(30.0))));
    s.apply(Action::Text(TextAction::Set(Field::Align(
        TextAlign::Center,
    ))));
    let t = text_of(&s, id).unwrap();
    assert_eq!((t.size, t.align), (30.0, TextAlign::Center));
    assert_eq!(s.doc.undo_count(), undo + 3);
    s.apply(Action::Undo);
    assert_eq!(text_of(&s, id).unwrap().align, TextAlign::Left);
    // テキストレイヤーでないときは次の文字の既定
    s.selected_layer = None;
    s.apply(Action::Text(TextAction::Set(Field::Size(64.0))));
    assert_eq!(s.text.defaults.size, 64.0);
    // ラスタライズで値を外す（画素は残る）
    s.selected_layer = Some(id);
    s.apply(Action::Text(TextAction::Rasterize(id)));
    assert!(text_of(&s, id).is_none());
}

// ───────── テキストの色（描画色・ツールの色） ─────────

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

fn byte(color: [f32; 4]) -> Rgba8 {
    yolu_app::matpaint::single_value(color)
}

fn color_of(s: &AppState, id: LayerId) -> Rgba8 {
    text_of(s, id).expect("テキストの値").color
}

/// 描画色を `color` にして、1 フレームぶんの追従を回す（ボタンが押されていない間）。
fn paint(s: &mut AppState, color: [f32; 4]) {
    s.color.set_main(color);
    s.text_follow_paint_color(false);
}

/// 描画色で文字を打ち、打ち終えて、そのテキストレイヤーを選んだままにする。
fn typed_layer(s: &mut AppState, color: [f32; 4]) -> LayerId {
    s.color.set_main(color);
    s.text_follow_paint_color(false);
    s.apply(Action::Text(TextAction::Begin { x: 10.0, y: 120.0 }));
    s.text_typed("abc".into());
    let id = editing_layer(s).expect("テキストレイヤー");
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(s.selected_layer, Some(id));
    id
}

#[test]
fn headless_a_new_text_is_made_in_the_paint_color_and_a_selected_text_layer_follows_it() {
    let mut s = state();
    assert_eq!(s.text.color_source, ColorSource::PaintColor, "既定は描画色");
    let id = typed_layer(&mut s, RED);
    assert_eq!(color_of(&s, id), byte(RED), "新しい文字は描画色");
    // 選んでいるテキストレイヤーは、描画色が変わると同じ色になる（1 回の変更が 1 回の取り消し）
    let undo = s.doc.undo_count();
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(BLUE));
    assert_eq!(s.doc.undo_count(), undo + 1);
    // 描画色が変わらないフレームでは、何も起こさない
    s.text_follow_paint_color(false);
    s.text_follow_paint_color(false);
    assert_eq!(s.doc.undo_count(), undo + 1);
    s.apply(Action::Undo);
    assert_eq!(color_of(&s, id), byte(RED));
    s.apply(Action::Redo);
    assert_eq!(color_of(&s, id), byte(BLUE));
    // 色の円のドラッグ（押している間に何度も変わる）は 1 回の取り消し
    let undo = s.doc.undo_count();
    for step in 0..6 {
        s.color.set_main([step as f32 / 6.0, 1.0, 0.0, 1.0]);
        s.text_follow_paint_color(true);
    }
    assert_eq!(color_of(&s, id), byte([5.0 / 6.0, 1.0, 0.0, 1.0]));
    s.text_follow_paint_color(false); // 離した
    assert_eq!(s.doc.undo_count(), undo + 1, "ドラッグ 1 回は取り消し 1 回");
    s.apply(Action::Undo);
    assert_eq!(color_of(&s, id), byte(BLUE), "ドラッグの前の色に戻る");
    s.apply(Action::Redo);
    assert_eq!(color_of(&s, id), byte([5.0 / 6.0, 1.0, 0.0, 1.0]));
    // 離したあとの変更は、別の取り消し
    let undo = s.doc.undo_count();
    paint(&mut s, GREEN);
    paint(&mut s, RED);
    assert_eq!(s.doc.undo_count(), undo + 2);
    // テキストレイヤーを選んでいなければ、描画色が変わってもそのレイヤーは変わらない。次の文字は、作るときの描画色で作る
    s.selected_layer = None;
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(RED));
    s.apply(Action::Text(TextAction::Begin { x: 30.0, y: 50.0 }));
    s.text_typed("n".into());
    let second = editing_layer(&s).unwrap();
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(color_of(&s, second), byte(BLUE));
    assert_eq!(color_of(&s, id), byte(RED), "ほかのテキストは変わらない");
}

#[test]
fn headless_the_first_frame_only_records_the_paint_color_and_leaves_the_selected_text_alone() {
    let mut s = state();
    s.apply(Action::Text(TextAction::Begin { x: 10.0, y: 120.0 }));
    s.text_typed("abc".into());
    let id = editing_layer(&s).unwrap();
    s.apply(Action::Text(TextAction::Commit));
    let made = color_of(&s, id);
    // まだ 1 度も追従を回していない（開いた直後・起動の直後）。描画色がテキストの色と違っても、最初の 1 回では変えない
    s.color.set_main(RED);
    let undo = s.doc.undo_count();
    s.text_follow_paint_color(false);
    assert_eq!(color_of(&s, id), made);
    assert_eq!(s.doc.undo_count(), undo);
    // 次から、描画色が変わったら追従する
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(BLUE));
}

#[test]
fn headless_the_paint_color_also_changes_the_text_being_typed_in_one_undo() {
    let mut s = state();
    s.color.set_main(RED);
    s.text_follow_paint_color(false);
    let before = s.doc.undo_count();
    s.apply(Action::Text(TextAction::Begin { x: 10.0, y: 120.0 }));
    // まだ 1 文字も打っていないあいだに替えても、その文字の色になる
    paint(&mut s, BLUE);
    s.text_typed("a".into());
    let id = editing_layer(&s).unwrap();
    assert_eq!(color_of(&s, id), byte(BLUE));
    // 打っている間に替える
    paint(&mut s, GREEN);
    assert_eq!(color_of(&s, id), byte(GREEN));
    s.text_typed("ab".into());
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(color_of(&s, id), byte(GREEN));
    assert_eq!(
        s.doc.undo_count(),
        before + 1,
        "打った分と色は、打ち終わりまでで 1 回の取り消し"
    );
    s.apply(Action::Undo);
    assert!(s.doc.layer(id).is_none());
}

#[test]
fn headless_the_tool_color_makes_new_text_in_its_own_color_and_the_paint_color_stops_following() {
    let mut s = state();
    let id = typed_layer(&mut s, RED);
    s.apply(Action::Text(TextAction::ColorSource(
        ColorSource::ToolColor,
    )));
    assert_eq!(s.text.color_source, ColorSource::ToolColor);
    // ツールの色のあいだは、描画色が変わってもテキストレイヤーは変わらない
    let undo = s.doc.undo_count();
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(RED));
    assert_eq!(s.doc.undo_count(), undo);
    // ツールの色を変えると、選んでいるテキストレイヤーも同じ色になり（1 回の取り消し）、次の文字もその色
    s.apply(Action::Text(TextAction::ToolColor {
        color: byte(GREEN),
        dragging: false,
    }));
    assert_eq!(color_of(&s, id), byte(GREEN));
    assert_eq!(s.doc.undo_count(), undo + 1);
    assert_eq!(s.text.defaults.color, byte(GREEN));
    s.apply(Action::Undo);
    assert_eq!(color_of(&s, id), byte(RED));
    s.selected_layer = None;
    s.apply(Action::Text(TextAction::Begin { x: 40.0, y: 60.0 }));
    s.text_typed("t".into());
    let tool = editing_layer(&s).unwrap();
    s.apply(Action::Text(TextAction::Commit));
    assert_eq!(color_of(&s, tool), byte(GREEN), "ツールの色で作る");
    // ドラッグ（色のウィンドウ）は 1 回の取り消し
    let undo = s.doc.undo_count();
    for step in 1..=4 {
        s.apply(Action::Text(TextAction::ToolColor {
            color: Rgba8::new(step * 50, 0, 0, 255),
            dragging: true,
        }));
    }
    s.m2_end_drag();
    assert_eq!(color_of(&s, tool), Rgba8::new(200, 0, 0, 255));
    assert_eq!(s.doc.undo_count(), undo + 1, "ドラッグ 1 回は取り消し 1 回");
    s.apply(Action::Undo);
    assert_eq!(color_of(&s, tool), byte(GREEN));
    // 描画色に戻すと、新しい文字は描画色になる。戻しただけでは、選んでいるテキストの色を変えない
    s.apply(Action::Text(TextAction::ColorSource(
        ColorSource::PaintColor,
    )));
    s.selected_layer = Some(id);
    let kept = color_of(&s, id);
    s.text_follow_paint_color(false);
    assert_eq!(color_of(&s, id), kept, "戻しただけでは変えない");
    paint(&mut s, [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(color_of(&s, id), byte([0.5, 0.5, 0.5, 1.0]));
}

#[test]
fn headless_the_paint_color_leaves_text_alone_when_it_cannot_be_changed() {
    // 描いている間・読むだけのセットでは、黙って変えない
    let mut s = state();
    let id = typed_layer(&mut s, RED);
    s.sets.get_mut(0).unwrap().read_only = Some("試験".into());
    s.message.clear();
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(RED));
    assert!(s.message.is_empty(), "{}", s.message);
    s.sets.get_mut(0).unwrap().read_only = None;
    let settings = s.stroke_settings(false);
    let other = s.doc.add_layer("下").unwrap();
    let stroke = s.doc.begin_stroke(other, &settings).unwrap();
    paint(&mut s, GREEN);
    assert_eq!(color_of(&s, id), byte(RED));
    assert!(s.message.is_empty(), "{}", s.message);
    s.doc.cancel_stroke(stroke);
}

#[test]
fn headless_a_locked_text_layer_is_left_alone_without_a_message_every_frame() {
    for (name, locks) in [
        ("すべて", LayerLocks::ALL),
        ("画素", LayerLocks::PIXELS),
        ("透明部分", LayerLocks::TRANSPARENCY),
    ] {
        let mut s = state();
        let id = typed_layer(&mut s, RED);
        s.doc.set_layer_locks(id, locks).unwrap();
        s.message.clear();
        let undo = s.doc.undo_count();
        // 何フレームも描画色を変えても、色は変わらず、断りも出ない
        for color in [BLUE, GREEN, [0.5, 0.5, 0.5, 1.0]] {
            paint(&mut s, color);
            assert_eq!(color_of(&s, id), byte(RED), "{name}");
            assert!(s.message.is_empty(), "{name}: {}", s.message);
            assert!(s.last_notice.is_none(), "{name}");
        }
        assert_eq!(s.doc.undo_count(), undo, "{name}");
        // ロックを外すと、次に変えたときから追従する
        s.doc.set_layer_locks(id, LayerLocks::NONE).unwrap();
        paint(&mut s, BLUE);
        assert_eq!(color_of(&s, id), byte(BLUE), "{name}");
    }
}

#[test]
fn headless_the_paint_color_follows_only_while_the_text_tool_is_in_use() {
    let mut s = state();
    let id = typed_layer(&mut s, RED);
    // ブラシに替えて次の色を選んでも、選んだままのテキストの色は変わらない
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.selected_layer, Some(id));
    let undo = s.doc.undo_count();
    paint(&mut s, BLUE);
    paint(&mut s, GREEN);
    assert_eq!(color_of(&s, id), byte(RED));
    assert_eq!(s.doc.undo_count(), undo);
    // テキストのツールへ戻しただけでは、色は変わらない（描画色の基準は追い続けている）
    s.apply(Action::SelectTool(Tool::Text));
    s.text_follow_paint_color(false);
    assert_eq!(color_of(&s, id), byte(RED));
    // テキストのツールのあいだに描画色を変えたら追従する
    paint(&mut s, BLUE);
    assert_eq!(color_of(&s, id), byte(BLUE));
    // ドラッグの途中でほかのツールへ替わったら、まとめを切って、そこから先は追従しない
    s.color.set_main([0.2, 0.2, 0.2, 1.0]);
    s.text_follow_paint_color(true);
    assert_eq!(color_of(&s, id), byte([0.2, 0.2, 0.2, 1.0]));
    s.apply(Action::SelectTool(Tool::Move));
    s.color.set_main([0.8, 0.8, 0.8, 1.0]);
    s.text_follow_paint_color(true);
    assert_eq!(color_of(&s, id), byte([0.2, 0.2, 0.2, 1.0]));
    // 打っている間は、ツールによらず（打つのはテキストのツール）追従する
    s.apply(Action::SelectTool(Tool::Text));
    s.apply(Action::Text(TextAction::Edit(id)));
    paint(&mut s, GREEN);
    assert_eq!(color_of(&s, id), byte(GREEN));
    s.apply(Action::Text(TextAction::Commit));
}

#[test]
fn headless_a_text_layer_color_survives_saving_and_opening() {
    let dir = std::env::temp_dir().join(format!("yolu-textcolor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = state();
    let id = typed_layer(&mut s, RED);
    paint(&mut s, BLUE);
    let file = dir.join("text-color.ylp");
    s.apply(Action::SaveProjectAs(file.clone()));
    assert!(file.exists(), "{}", s.message);
    let mut t = AppState::new(32, 32);
    t.apply(Action::OpenProject(file));
    assert_eq!(color_of(&t, id), byte(BLUE), "{}", t.message);
    // 開いたあと、そのレイヤーを選んでテキストのツールで描画色を変えると、また同じ色になる
    t.apply(Action::SelectTool(Tool::Text));
    t.selected_layer = Some(id);
    t.text_follow_paint_color(false);
    paint(&mut t, GREEN);
    assert_eq!(color_of(&t, id), byte(GREEN));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_moving_a_text_layer_moves_its_values_and_flipping_is_refused() {
    let mut s = state();
    s.apply(Action::Text(TextAction::Begin { x: 10.0, y: 120.0 }));
    s.text_typed("move".into());
    let id = editing_layer(&s).unwrap();
    s.apply(Action::Text(TextAction::Commit));
    s.apply(Action::SelectTool(Tool::Move));
    let undo = s.doc.undo_count();
    s.apply(Action::M2(yolu_app::m2::Edit::Transform(
        yolu_app::layerops::Xform::Move { dx: 7, dy: -3 },
    )));
    let t = text_of(&s, id).unwrap();
    assert_eq!((t.x, t.y), (17.0, 117.0));
    assert_eq!(s.doc.undo_count(), undo + 1);
    s.apply(Action::M2(yolu_app::m2::Edit::Transform(
        yolu_app::layerops::Xform::Affine(
            Affine2D::from_parts((0.0, 0.0), (0.0, 0.0), 30.0, (1.0, 1.0)).unwrap(),
        ),
    )));
    assert!((text_of(&s, id).unwrap().rotation - 30.0).abs() < 1e-9);
    let before = s.doc.undo_count();
    s.apply(Action::M2(yolu_app::m2::Edit::Transform(
        yolu_app::layerops::Xform::Flip { horizontal: true },
    )));
    assert_eq!(s.doc.undo_count(), before, "反転は断る");
    assert!(s.message.contains("反転"), "{}", s.message);
    // 縦横の倍率が違う・傾く変形は値で表せないので断る（黙って一様な倍率にしない）。一様な拡大は値のサイズになる
    let value = text_of(&s, id).unwrap();
    for (what, transform) in [
        (
            "縦横の倍率",
            Affine2D::from_parts((0.0, 0.0), (0.0, 0.0), 0.0, (2.0, 1.0)).unwrap(),
        ),
        (
            "回して縦横の倍率",
            Affine2D::from_parts((0.0, 0.0), (0.0, 0.0), 25.0, (1.0, 1.5)).unwrap(),
        ),
        (
            "傾き",
            Affine2D {
                b: 0.3,
                ..Affine2D::IDENTITY
            },
        ),
    ] {
        for lang in Lang::ALL {
            s.lang = lang;
            s.message.clear();
            let before = s.doc.undo_count();
            s.apply(Action::M2(yolu_app::m2::Edit::Transform(
                yolu_app::layerops::Xform::Affine(transform),
            )));
            assert_eq!(s.doc.undo_count(), before, "{what}は断る");
            assert_eq!(text_of(&s, id).unwrap(), value, "{what}で値を変えない");
            assert!(
                s.message
                    .contains(lang.pick("縦横の比を変えられません", "aspect ratio")),
                "{what}: {}",
                s.message
            );
        }
    }
    s.lang = Lang::Ja;
    let size = value.size;
    s.apply(Action::M2(yolu_app::m2::Edit::Transform(
        yolu_app::layerops::Xform::Affine(
            Affine2D::from_parts((0.0, 0.0), (0.0, 0.0), 0.0, (2.0, 2.0)).unwrap(),
        ),
    )));
    assert_eq!(text_of(&s, id).unwrap().size, size * 2.0);
    // 数値の変形の縦横の倍率も同じ
    let before = s.doc.undo_count();
    s.apply(Action::M2(yolu_app::m2::Edit::Transform(
        yolu_app::layerops::Xform::Numeric {
            dx: 0.0,
            dy: 0.0,
            degrees: 0.0,
            sx: 1.0,
            sy: 0.5,
        },
    )));
    assert_eq!(s.doc.undo_count(), before);
    assert!(s.message.contains("縦横の比"), "{}", s.message);
}

#[test]
fn headless_a_text_layer_whose_font_is_missing_shows_its_pixels_and_refuses_edits() {
    let mut s = state();
    let id = s.doc.add_layer("missing").unwrap();
    s.doc
        .set_pixel(id, 3, 3, yolu_core::Rgba8::new(1, 2, 3, 255))
        .unwrap();
    let font = TextFont::File {
        path: "/nonexistent/font.ttf".into(),
        index: 0,
        sha256: [7; 32],
        names: Default::default(),
    };
    s.doc
        .set_text_for_load(id, TextSettings::new("x", font, 0.0, 10.0))
        .unwrap();
    s.selected_layer = Some(id);
    // OS のフォントは空の一覧（この PC に入っているフォントで結果が変わらないように）
    s.text.fonts.list = Some(Default::default());
    for lang in Lang::ALL {
        s.lang = lang;
        assert!(s.text_font_problem(id).is_some());
        s.apply(Action::Text(TextAction::Edit(id)));
        assert!(s.text.editing.is_none());
        assert!(
            s.message
                .contains(lang.pick("フォントが見つかりません", "Font not found")),
            "{}",
            s.message
        );
    }
}

/// 決めたフォルダのフォントの一覧と、その中のフォントのファイルの道。
fn font_folder(
    name: &str,
) -> (
    std::path::PathBuf,
    std::sync::Arc<yolu_io::fonts::SystemFonts>,
) {
    let dir = std::env::temp_dir().join(format!("yolu-text-fonts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    std::fs::copy(source.join("BIZUDPGothic-Bold.ttf"), dir.join("bold.ttf")).unwrap();
    let list = std::sync::Arc::new(yolu_io::fonts::SystemFonts::from_dirs(&[&dir]));
    (dir, list)
}

#[test]
fn headless_a_font_found_by_name_with_other_contents_is_reported_and_redraws_when_edited() {
    let (dir, list) = font_folder("different");
    let mut s = state();
    s.lang = Lang::Ja;
    s.text.fonts.list = Some(list);
    let bytes = std::fs::read(dir.join("bold.ttf")).unwrap();
    // 別の PC で選んだ同じ名前のフォント（道は無く、中身の印が違う）
    let TextFont::File { names, .. } = yolu_core::text::file_font("x", 0, &bytes) else {
        panic!()
    };
    let font = TextFont::File {
        path: "/elsewhere/BIZUDPGothic-Bold.ttf".into(),
        index: 0,
        sha256: [3; 32],
        names,
    };
    let id = s.doc.add_layer("other").unwrap();
    s.doc
        .set_pixel(id, 3, 3, yolu_core::Rgba8::new(1, 2, 3, 255))
        .unwrap();
    s.doc
        .set_text_for_load(id, TextSettings::new("x", font.clone(), 0.0, 10.0))
        .unwrap();
    s.doc.clear_history().unwrap();
    s.selected_layer = Some(id);
    // 開いたときの確かめ: 1 度だけ知らせ、画素はそのまま
    s.text.check_fonts = true;
    s.text_poll();
    assert!(!s.text.check_fonts);
    assert!(s.message.contains("フォントが違います"), "{}", s.message);
    assert!(s.message.contains("other"), "{}", s.message);
    assert_eq!(
        s.text_font_status(id),
        yolu_app::textlayer::FontStatus::Different
    );
    assert!(
        s.text_font_problem(id).is_none(),
        "違うフォントは編集できる"
    );
    assert_eq!(
        text_of(&s, id).unwrap().font,
        font,
        "開いただけでは値を変えない"
    );
    // 値を変えると、見つけたフォントで描き直す（値の道と印もそのフォントのもの）
    s.apply(Action::Text(TextAction::Set(Field::Size(20.0))));
    let t = text_of(&s, id).unwrap();
    assert_eq!(t.size, 20.0);
    let TextFont::File { path, sha256, .. } = &t.font else {
        panic!()
    };
    assert!(path.ends_with("bold.ttf"), "{path}");
    assert_eq!(*sha256, yolu_core::text::sha256(&bytes));
    assert_eq!(
        s.text_font_status(id),
        yolu_app::textlayer::FontStatus::Ready
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_installed_fonts_are_picked_from_the_list() {
    let (dir, list) = font_folder("pick");
    let mut s = state();
    s.lang = Lang::Ja;
    s.text.fonts.list = Some(list.clone());
    let face = &list.faces()[0];
    s.apply(Action::Text(TextAction::SystemFont {
        path: face.path.clone(),
        index: face.index,
    }));
    let TextFont::File { names, .. } = &s.text.defaults.font else {
        panic!("{:?}", s.text.defaults.font)
    };
    assert_eq!(names.postscript, "BIZUDPGothic-Bold");
    assert_eq!(names.weight, 700);
    // メニューに「インストール済みのフォント」の入れ子があり、選んだフォントに印
    let entries = yolu_app::textlayer::props::font_entries(&s);
    let installed = entries
        .iter()
        .find(|e| e.label() == Some("インストール済みのフォント"))
        .expect("入れ子");
    let yolu_app::ui::menu::Entry::Submenu {
        entries, enabled, ..
    } = installed
    else {
        panic!()
    };
    assert!(*enabled);
    assert!(matches!(
        &entries[0],
        yolu_app::ui::menu::Entry::Item { label, check: yolu_app::ui::menu::Check::Radio, .. }
            if label == "BIZ UDPゴシック"
    ));
    // 一覧をなめている間は押せない（理由はツールチップ）
    s.text.fonts.list = None;
    let entries = yolu_app::textlayer::props::font_entries(&s);
    assert!(entries.iter().any(|e| matches!(
        e,
        yolu_app::ui::menu::Entry::Submenu {
            enabled: false,
            tooltip: Some(_),
            ..
        }
    )));
    let _ = std::fs::remove_dir_all(&dir);
}

/// ウィンドウで打つ: テキストツールでキャンバスを押して離し、打って Esc で終える。
#[test]
fn typing_on_the_canvas_makes_a_text_layer_in_one_undo_step() {
    gpu_thread::run(typing_on_the_canvas_gpu);
}

fn typing_on_the_canvas_gpu() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::SelectTool(Tool::Text));
    h.run();
    let r = canvas_rect(&h);
    let undo = h.state().state.doc.undo_count();
    click(&mut h, center(r));
    assert!(h.state().state.text.editing.is_some());
    h.event(Event::Text("Ab".into()));
    h.run();
    h.event(Event::Text("文字".into()));
    h.run();
    let id = editing_layer(&h.state().state).expect("打ったテキストレイヤー");
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    let s = &h.state().state;
    assert!(s.text.editing.is_none(), "Esc で打ち終わる");
    assert_eq!(text_of(s, id).unwrap().text, "Ab文字");
    assert_eq!(s.doc.undo_count(), undo + 1);
}

/// 移動・変形ツールでテキストレイヤーをダブルクリックすると、テキストツールに替わってその箱で打ち直す。キー T はテキストツール。
#[test]
fn double_clicking_a_text_layer_with_the_move_tool_edits_it() {
    gpu_thread::run(double_click_gpu);
}

fn double_click_gpu() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.lang = Lang::Ja;
    h.run();
    key(&h, Key::T, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Text, "T はテキストツール");
    let at = center(canvas_rect(&h));
    click(&mut h, at);
    h.event(Event::Text("Move".into()));
    h.run();
    let id = editing_layer(&h.state().state).expect("打ったテキストレイヤー");
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    h.state_mut().state.apply(Action::SelectTool(Tool::Move));
    h.run();
    let undo = h.state().state.doc.undo_count();
    let value = text_of(&h.state().state, id).unwrap();
    // 箱の中（基準の点から右下）を 2 回押す
    let inside = offset(at, 6.0, 6.0);
    click(&mut h, inside);
    click(&mut h, inside);
    let s = &h.state().state;
    assert_eq!(s.tool, Tool::Text, "テキストツールに替わる");
    assert_eq!(editing_layer(s), Some(id), "そのレイヤーを打ち直す");
    assert_eq!(s.doc.undo_count(), undo, "動かしていない");
    assert_eq!(text_of(s, id).unwrap(), value);
    // そのまま打てる（入力欄がフォーカスを受ける）
    h.event(Event::Text("X".into()));
    h.run();
    let typed = text_of(&h.state().state, id).unwrap().text;
    assert!(typed.contains('X') && typed.contains("Move"), "{typed}");
    // 外を 2 回押しても替わらない
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    h.state_mut().state.apply(Action::SelectTool(Tool::Move));
    h.run();
    let outside = offset(at, -60.0, 80.0);
    click(&mut h, outside);
    click(&mut h, outside);
    assert_eq!(h.state().state.tool, Tool::Move);
}

/// テキストツールでテキストレイヤーを選んでいるとき、サイズはオプションバー・左のツールプロパティ・右のレイヤーのプロパティの 3 か所に出る。
/// どれをドラッグして（クリックして）離しても、ドラッグした値が 1 回の取り消しで当たる（動かしていないほかのスライダーが、
/// ドラッグ中の値を消さない）。
#[test]
fn dragging_a_size_slider_applies_the_value_even_when_the_same_slider_is_shown_three_times() {
    gpu_thread::run(shared_slider_gpu);
}

/// スライダーの置き場所の名前と、その矩形かを見る条件。
type Place = (&'static str, fn(Rect) -> bool);

fn shared_slider_gpu() {
    let mut h = app(1600.0, 1300.0, 256);
    h.state_mut().state.lang = Lang::Ja;
    h.state_mut().state.apply(Action::SelectTool(Tool::Text));
    h.run();
    let at = center(canvas_rect(&h));
    click(&mut h, at);
    h.event(Event::Text("Size".into()));
    h.run();
    let id = editing_layer(&h.state().state).expect("打ったテキストレイヤー");
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    h.state_mut().state.ui.property_tab = 1;
    h.run();
    assert_eq!(h.state().state.selected_layer, Some(id));
    // 3 か所のサイズ（バー・左のドック・右のレイヤーのプロパティ）
    let places: [Place; 3] = [
        ("バー", |r| r.top() > 24.0 && r.bottom() < 62.0),
        ("ツールプロパティ", |r| {
            r.left() < 390.0 && r.top() > 62.0
        }),
        ("レイヤーのプロパティ", |r| {
            r.left() > rx() && r.top() > 62.0
        }),
    ];
    let undo = h.state().state.doc.undo_count();
    let mut steps = 0;
    for (place, pick) in &places {
        // 小さい値から始めて、ドラッグで最大・最小（欄の外まで動かして離す）、クリックで大きい値・最小（どれも値が変わる）
        for (gesture, from, to) in [
            ("ドラッグ", 0.5, 1.2),
            ("ドラッグ", 0.5, -0.2),
            ("クリック", 0.95, 0.95),
            ("クリック", 0.0, 0.0),
        ] {
            let slider = rect_of(&h, "サイズ", pick);
            let at = |f: f32| pos2(slider.left() + slider.width() * f + 1.0, slider.center().y);
            if gesture == "ドラッグ" {
                drag(&mut h, &[at(from), at((from + to) / 2.0), at(to)]);
            } else {
                click(&mut h, at(from));
            }
            steps += 1;
            let s = &h.state().state;
            let size = text_of(s, id).unwrap().size;
            let high = from > 0.4 && to > 0.4;
            assert!(
                if high { size > 300.0 } else { size == 1.0 },
                "{place}の{gesture}: {size}"
            );
            assert_eq!(
                s.doc.undo_count(),
                undo + steps,
                "{place}の{gesture}は 1 回の取り消し"
            );
        }
    }
    // Esc でドラッグを止めると、押し始めの値のまま（取り消しの段も増えない）。小数の値（行間）も、f32 の丸めで値を変えない
    let steps_done = h.state().state.doc.undo_count();
    h.state_mut()
        .state
        .apply(Action::Text(TextAction::Set(Field::LineHeight(1.37))));
    h.run();
    let steps_done = steps_done + 1;
    for (place, pick) in &places[1..] {
        let slider = rect_of(&h, "行間", pick);
        let at = |f: f32| pos2(slider.left() + slider.width() * f, slider.center().y);
        press(&h, at(0.5), egui::PointerButton::Primary);
        h.step();
        move_to(&h, at(0.9));
        h.step();
        key(&h, Key::Escape, Modifiers::NONE);
        h.step();
        release(&h, at(0.9), egui::PointerButton::Primary);
        h.step();
        h.run();
        let s = &h.state().state;
        assert_eq!(text_of(s, id).unwrap().line_height, 1.37, "{place}の Esc");
        assert_eq!(
            s.doc.undo_count(),
            steps_done,
            "{place}の Esc は段を増やさない"
        );
        assert!(s.text.pending.is_none(), "{place}の Esc");
    }
}

/// ツールプロパティの「テキストの色」: 見出しと 2 つの選び（描画色・ツールの色）。選ぶと色の元が替わり、描画色のあいだは、フレームごとに、
/// 選んでいるテキストレイヤーが描画色に追従する（円のドラッグ中は 1 回の取り消し）。
#[test]
fn the_text_color_choice_in_the_tool_properties_switches_the_source_and_the_layer_follows_the_paint_color(
) {
    gpu_thread::run(text_color_gpu);
}

fn text_color_gpu() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 900.0, 256);
        h.state_mut().state.lang = lang;
        h.state_mut().state.apply(Action::SelectTool(Tool::Text));
        h.run();
        let paint_name = lang.pick("描画色", "Paint Color");
        let tool_name = lang.pick("ツールの色", "Tool Color");
        let paint = dock_rect(&h, paint_name);
        let tool = dock_rect(&h, tool_name);
        assert!(h.state().state.text.color_source == ColorSource::PaintColor);
        click(&mut h, tool.center());
        assert_eq!(h.state().state.text.color_source, ColorSource::ToolColor);
        click(&mut h, paint.center());
        assert_eq!(h.state().state.text.color_source, ColorSource::PaintColor);
        // 画面の文字に日本語が混ざらない（英語）
        if lang == Lang::En {
            assert!(!has_japanese(paint_name) && !has_japanese(tool_name));
        }
        // 文字を打って、そのレイヤーを選んだまま、描画色を変える（フレームを回すだけで追従する）
        let at = center(canvas_rect(&h));
        click(&mut h, at);
        h.event(Event::Text("Hi".into()));
        h.run();
        let id = editing_layer(&h.state().state).expect("打ったテキストレイヤー");
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        assert_eq!(h.state().state.selected_layer, Some(id));
        h.state_mut().state.color.set_main(BLUE);
        h.run();
        assert_eq!(color_of(&h.state().state, id), byte(BLUE), "{lang:?}");
        // 押している間に何度か変えるドラッグ（どこかを押したまま）は、離すまで 1 回の取り消し
        let undo = h.state().state.doc.undo_count();
        let hold = pos2(640.0, 880.0);
        press(&h, hold, egui::PointerButton::Primary);
        h.step();
        for step in 1..=4 {
            h.state_mut()
                .state
                .color
                .set_main([step as f32 / 4.0, 0.5, 0.0, 1.0]);
            h.step();
        }
        release(&h, hold, egui::PointerButton::Primary);
        h.run();
        assert_eq!(
            color_of(&h.state().state, id),
            byte([1.0, 0.5, 0.0, 1.0]),
            "{lang:?}"
        );
        assert_eq!(h.state().state.doc.undo_count(), undo + 1, "{lang:?}");
        if std::env::var_os("YOLU_TEXT_SHOTS").is_some() {
            save(&mut h, &format!("text-color-paint-{lang:?}"));
        }
        // ツールの色に替えると、描画色を変えても追従しない
        let tool = dock_rect(&h, tool_name);
        click(&mut h, tool.center());
        h.state_mut().state.color.set_main(GREEN);
        h.run();
        assert_eq!(
            color_of(&h.state().state, id),
            byte([1.0, 0.5, 0.0, 1.0]),
            "{lang:?}: ツールの色のあいだは追従しない"
        );
        if std::env::var_os("YOLU_TEXT_SHOTS").is_some() {
            save(&mut h, &format!("text-color-tool-{lang:?}"));
        }
    }
}

/// 確かめ用の絵（`YOLU_TEXT_SHOTS` のときだけ書く）: 打っている所・テキストレイヤーを選んだプロパティの欄・フォントの選び、日英。
#[test]
fn shots_of_the_text_tool() {
    if std::env::var_os("YOLU_TEXT_SHOTS").is_none() {
        return;
    }
    gpu_thread::run(shots_gpu);
}

fn save(h: &mut egui_kittest::Harness<'_, YoluApp>, name: &str) {
    let dir = std::path::PathBuf::from(std::env::var_os("YOLU_TEXT_SHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let image = h.render().expect("描画");
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

/// 描いた文字（`Shape::Text`）のうち、`text` と同じ文で `pick` に合う矩形（画面の点）。
fn painted(
    h: &egui_kittest::Harness<'_, YoluApp>,
    text: &str,
    pick: impl Fn(Rect) -> bool,
) -> Option<Rect> {
    fn find(shape: &egui::epaint::Shape, text: &str, pick: &dyn Fn(Rect) -> bool) -> Option<Rect> {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text, pick)),
            egui::epaint::Shape::Text(t) if t.galley.job.text == text => {
                Some(t.galley.rect.translate(t.pos.to_vec2())).filter(|r| pick(*r))
            }
            _ => None,
        }
    }
    h.output()
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text, &pick))
}

fn shots_gpu() {
    for lang in Lang::ALL {
        let tag = match lang {
            Lang::Ja => "ja",
            Lang::En => "en",
        };
        let mut h = app(1280.0, 1100.0, 512);
        h.state_mut().state.lang = lang;
        // OS のフォントの一覧（この PC に入っているフォント）と、スタイルが 2 つ以上あるファミリーの名前
        let list = std::sync::Arc::new(yolu_io::fonts::SystemFonts::scan());
        let family_name = list
            .families()
            .iter()
            .find(|f| f.faces.len() > 1)
            .map(|f| match (lang, &f.name_ja) {
                (Lang::Ja, Some(ja)) => ja.clone(),
                _ => f.name.clone(),
            })
            .expect("スタイルが 2 つ以上あるフォント");
        h.state_mut().state.text.fonts.list = Some(list);
        h.state_mut().state.apply(Action::SelectTool(Tool::Text));
        h.run();
        let r = canvas_rect(&h);
        // 打っている所（2 行。カーソルと枠）
        click(
            &mut h,
            pos2(r.left() + r.width() * 0.3, r.top() + r.height() * 0.3),
        );
        h.event(Event::Text(
            lang.pick("テキストレイヤー", "Text layer").into(),
        ));
        h.run();
        key(&h, Key::Enter, Modifiers::NONE);
        h.event(Event::Text("Hello 文字".into()));
        h.run();
        save(&mut h, &format!("text-typing-{tag}"));
        key(&h, Key::Escape, Modifiers::NONE);
        h.run();
        // テキストレイヤーを選んだプロパティの欄（レイヤーのタブ）
        h.state_mut().state.ui.property_tab = 1;
        h.run();
        save(&mut h, &format!("text-layer-props-{tag}"));
        // フォントの選び
        let label = lang.pick("BIZ UDPゴシック", "BIZ UDPGothic");
        let font = painted(&h, label, |r: Rect| r.left() < 300.0).expect("フォントの欄");
        click(&mut h, font.center());
        save(&mut h, &format!("text-font-menu-{tag}"));
        // インストール済みのフォントの入れ子と、スタイルの入れ子
        let installed = lang.pick("インストール済みのフォント", "Installed Fonts");
        let row = painted(&h, installed, |_| true).expect("インストール済みのフォント");
        h.event(Event::PointerMoved(row.center()));
        h.run();
        let family =
            painted(&h, &family_name, |r: Rect| r.left() > row.right()).expect("ファミリー");
        // 入れ子の行へ横に動かしてから下りる（斜めに動かすと、途中の行の入れ子に替わる）
        h.event(Event::PointerMoved(pos2(family.center().x, row.center().y)));
        h.run();
        h.event(Event::PointerMoved(family.center()));
        h.run();
        save(&mut h, &format!("text-font-installed-{tag}"));
        // Esc 1 回で入れ子を 1 段ずつ閉じる（スタイル・インストール済みのフォント・メニュー）
        for _ in 0..3 {
            key(&h, Key::Escape, Modifiers::NONE);
            h.run();
        }
        // フォントが見つからないテキストレイヤーを開いたとき（描いた画素のまま、値は変えられない）
        let id = h.state().state.selected_layer.expect("テキストレイヤー");
        let mut value = text_of(&h.state().state, id).expect("テキストの値");
        value.font = TextFont::File {
            path: "/nonexistent/font.ttf".into(),
            index: 0,
            sha256: [7; 32],
            names: Default::default(),
        };
        // 開いた文書と同じ形（画素はそのまま、値だけ別のフォント）にする
        let doc = &mut h.state_mut().state.doc;
        doc.rasterize(id).unwrap();
        doc.set_text_for_load(id, value).unwrap();
        doc.clear_history().unwrap();
        h.state_mut().state.text.check_fonts = true;
        h.run();
        save(&mut h, &format!("text-missing-font-{tag}"));
    }
}
