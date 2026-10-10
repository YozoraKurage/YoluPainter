//! ステンシル（画面に重ねた画像を通して塗る）の画面の操作（egui_kittest）: プロパティのタブ（画像を読む・読み方・繰り返し・反転・不透明度・
//! 大きさ・角度）、2D のキャンバスと 3D のビューの上の重ね表示、Y を押したままのドラッグ（回す・動かす・大きさ）、N（効かない）、
//! 通して塗った結果。どれも「操作 → 文書が変わる → Undo で戻る」。ステンシルは文書ではなくアプリの状態で、保存しない。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, Channel};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::mode::{EditorMode, ModeAction};
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use yolu_app::stencil::{StencilOp, DEFAULT_SIZE, MAX_SIZE, MIN_SIZE};
use yolu_app::ui::menu::PopupState;
use yolu_app::{Tab, YoluApp};
use yolu_core::{StencilMode, StencilTiling};
use yolu_io::Archive;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-stencil-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(dir: &Path, name: &str, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> PathBuf {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}

/// 左半分が白、右半分が黒の灰色の画像（量のモードで、左半分だけが通る）。
fn half_png(dir: &Path, name: &str) -> PathBuf {
    write_png(dir, name, 64, 64, |x, _| {
        if x < 32 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    })
}

/// 見た目の確かめ用（輪と斜めのグラデーションの灰色。縦横比 3 : 2）。
fn ring_png(dir: &Path, name: &str) -> PathBuf {
    write_png(dir, name, 96, 64, |x, y| {
        let (dx, dy) = (x as f32 - 48.0, y as f32 - 32.0);
        let d = (dx * dx + dy * dy).sqrt();
        let v = if (14.0..22.0).contains(&d) {
            255
        } else {
            ((x + y) * 255 / 158) as u8 / 2
        };
        [v, v, v, 255]
    })
}

/// 色の画像（赤から青のグラデーション。右下は透明）。
fn color_png(dir: &Path, name: &str) -> PathBuf {
    write_png(dir, name, 96, 64, |x, y| {
        let a = if x > 64 && y > 40 { 0 } else { 255 };
        [(255 - x * 2) as u8, (y * 3) as u8, (x * 2) as u8, a]
    })
}

fn load(h: &mut Harness<'_, YoluApp>, path: &Path) {
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Load(path.to_path_buf())));
    h.run();
}

fn key_down(h: &mut Harness<'_, YoluApp>, key: Key) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn key_up(h: &mut Harness<'_, YoluApp>, key: Key) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn press_with(h: &mut Harness<'_, YoluApp>, at: Pos2, button: PointerButton, m: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed: true,
        modifiers: m,
    });
    h.step();
}

fn release_at(h: &mut Harness<'_, YoluApp>, at: Pos2, button: PointerButton) {
    release(h, at, button);
    h.step();
}

/// 回した重ね表示のテクスチャの補間は、GPU のドライバのレイヤーで数画素ゆれる（回したキャンバスの試験と同じ）。それ以外の差は落とす。
fn rotated_texture() -> egui_kittest::SnapshotOptions {
    egui_kittest::SnapshotOptions::new().max_failed_pixels(64)
}

/// ウィンドウに落としたファイル（パスだけ）。
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn popup_kind(h: &Harness<'_, YoluApp>) -> Option<PopupKind> {
    h.state().state.popup.as_ref().map(|p| p.kind)
}

fn st<'a>(h: &'a Harness<'_, YoluApp>) -> &'a yolu_app::stencil::StencilState {
    &h.state().state.stencil
}

/// 画面の左右へ横切る線（中心の高さ）。
fn line_across(h: &mut Harness<'_, YoluApp>, from_dx: f32, to_dx: f32) {
    let c = canvas_rect(h).center();
    let n = 12;
    let points: Vec<Pos2> = (0..=n)
        .map(|i| pos2(c.x + from_dx + (to_dx - from_dx) * i as f32 / n as f32, c.y))
        .collect();
    drag(h, &points);
}

fn small_brush(h: &mut Harness<'_, YoluApp>) {
    let s = &mut h.state_mut().state;
    s.brush.radius = 6.0;
    s.brush.hardness = 1.0;
    s.m2.random_seed = false;
}

// ───────── 画面なしの試験（Wine でも回る） ─────────

#[test]
fn headless_loading_a_png_sets_the_stencil_and_bad_files_say_why() {
    let dir = temp_dir("load");
    let path = write_png(&dir, "half.png", 64, 32, |x, _| {
        if x < 32 {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        }
    });
    let mut s = AppState::new(64, 64);
    assert!(!s.stencil.applies() && !s.stencil.shown());
    s.apply(Action::Stencil(StencilOp::Load(path.clone())));
    let img = s.stencil.image.as_ref().expect("読んだ");
    assert_eq!(
        (img.name.as_str(), img.width, img.height),
        ("half.png", 64, 32)
    );
    assert!(s.message.contains("half.png"), "{}", s.message);
    assert_eq!(s.stencil.recent.len(), 1);
    assert!(s.stencil.applies() && s.stencil.shown());
    assert!(!s.doc.can_undo() && !s.modified, "文書は変わらない");
    // 灰色の画像は、自動で量
    assert_eq!(s.stencil.resolved_mode(), Some(StencilMode::Mask));

    // PNG でないファイル・無いファイル・大きすぎる画像は、今のステンシルのまま理由を出す
    let bad = dir.join("bad.png");
    std::fs::write(&bad, b"not a png at all").unwrap();
    for target in [bad.clone(), dir.join("missing.png")] {
        s.message.clear();
        s.apply(Action::Stencil(StencilOp::Load(target.clone())));
        let name = target.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            s.message.contains(&name) && s.message.contains("読めません"),
            "{}",
            s.message
        );
        assert_eq!(s.stencil.image.as_ref().unwrap().name, "half.png");
    }
    let wide = write_png(&dir, "wide.png", 8193, 1, |_, _| [255; 4]);
    s.message.clear();
    s.apply(Action::Stencil(StencilOp::Load(wide)));
    assert!(
        s.message.contains("wide.png") && s.message.contains("8193"),
        "{}",
        s.message
    );
    assert_eq!(s.stencil.image.as_ref().unwrap().name, "half.png");
    assert_eq!(s.stencil.recent.len(), 1, "読めなかった画像は覚えない");
    // 英語
    s.lang = Lang::En;
    s.apply(Action::Stencil(StencilOp::Load(bad)));
    assert!(
        s.message.contains("Cannot load the stencil"),
        "{}",
        s.message
    );
    // 読んだ画像の一覧から読み直す。新しい順で、同じ画像は 1 つ
    let other = ring_png(&dir, "ring.png");
    s.apply(Action::Stencil(StencilOp::Load(other)));
    s.apply(Action::Stencil(StencilOp::Load(path)));
    let names: Vec<&str> = s.stencil.recent.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["half.png", "ring.png"]);
    s.apply(Action::Stencil(StencilOp::Recent(1)));
    assert_eq!(s.stencil.image.as_ref().unwrap().name, "ring.png");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_a_too_big_mip_chain_is_refused_and_the_old_stencil_stays() {
    let dir = temp_dir("budget");
    let path = half_png(&dir, "half.png");
    let mut s = AppState::new(64, 64);
    s.apply(Action::Stencil(StencilOp::Load(path.clone())));
    s.stencil.mip_budget = 10; // ミップマップの予算を下げる
    let big = ring_png(&dir, "ring.png");
    s.apply(Action::Stencil(StencilOp::Load(big)));
    assert!(
        s.message.contains("ring.png") && s.message.contains("ミップマップ"),
        "{}",
        s.message
    );
    assert_eq!(s.stencil.image.as_ref().unwrap().name, "half.png");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_load_failures_are_told_in_the_selected_language_with_no_japanese_in_english() {
    let dir = temp_dir("errlang");
    let mut s = AppState::new(64, 64);
    s.apply(Action::Stencil(StencilOp::Load(half_png(&dir, "half.png"))));
    s.lang = Lang::En;
    let bad = dir.join("bad.png");
    std::fs::write(&bad, b"not a png at all").unwrap();
    let wide = write_png(&dir, "wide.png", 8193, 1, |_, _| [255; 4]);
    let ring = ring_png(&dir, "ring.png");
    // ミップマップが予算を超える画像（core の InvalidArgument）は、core の英訳の表を通る
    s.stencil.mip_budget = 10;
    let cases = [
        (ring.clone(), "Stencil mipmap budget exceeded"),
        (dir.join("missing.png"), "File or folder not found"),
        (bad, "Not a readable PNG"),
        (wide, "Image too large (8193 × 1; maximum side 8192)"),
    ];
    for (path, reason) in cases {
        s.message.clear();
        s.apply(Action::Stencil(StencilOp::Load(path.clone())));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            s.message.starts_with("Cannot load the stencil")
                && s.message.contains(&name)
                && s.message.contains(reason),
            "{}",
            s.message
        );
        // 「×」だけは英語の文にも出る。かなと漢字は混ざらない
        assert!(
            s.message.chars().all(|c| c.is_ascii() || c == '×'),
            "{}",
            s.message
        );
        assert_eq!(s.stencil.image.as_ref().unwrap().name, "half.png");
    }
    // 同じ失敗を日本語で
    s.lang = Lang::Ja;
    s.message.clear();
    s.apply(Action::Stencil(StencilOp::Load(ring)));
    assert!(
        s.message.contains("ステンシルのミップマップが予算を超える"),
        "{}",
        s.message
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_ops_clamp_their_values_and_are_refused_while_stroking() {
    let mut s = AppState::new(64, 64);
    s.stencil.set_image_rgba("a", 2, 2, &[255; 16]).unwrap();
    s.apply(Action::Stencil(StencilOp::Size(1e9)));
    assert_eq!(s.stencil.size, MAX_SIZE);
    s.apply(Action::Stencil(StencilOp::Size(0.0)));
    assert_eq!(s.stencil.size, MIN_SIZE);
    s.apply(Action::Stencil(StencilOp::Size(f32::NAN)));
    assert_eq!(s.stencil.size, DEFAULT_SIZE);
    s.apply(Action::Stencil(StencilOp::Opacity(7.0)));
    assert_eq!(s.stencil.opacity, 1.0);
    s.apply(Action::Stencil(StencilOp::Opacity(f32::INFINITY)));
    assert_eq!(s.stencil.opacity, 0.5, "有限でない値は初めの値");
    s.apply(Action::Stencil(StencilOp::Angle(190.0)));
    assert_eq!(s.stencil.angle, -170.0);
    s.apply(Action::Stencil(StencilOp::Mode(StencilMode::Color)));
    s.apply(Action::Stencil(StencilOp::Tiling(StencilTiling::Both)));
    s.apply(Action::Stencil(StencilOp::Invert(true)));
    assert_eq!(
        (s.stencil.mode, s.stencil.tiling, s.stencil.invert),
        (StencilMode::Color, StencilTiling::Both, true)
    );
    s.apply(Action::Stencil(StencilOp::ResetPlacement));
    assert_eq!(
        (s.stencil.center, s.stencil.size, s.stencil.angle),
        ([0.5, 0.5], DEFAULT_SIZE, 0.0)
    );
    assert_eq!(s.stencil.tiling, StencilTiling::Both, "置き場だけ戻す");

    // 描いている最中は、何も変えない
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.stroke = Some(stroke);
    for op in [
        StencilOp::Size(1.0),
        StencilOp::Clear,
        StencilOp::Mode(StencilMode::Mask),
        StencilOp::Pick,
    ] {
        s.message.clear();
        s.apply(Action::Stencil(op));
        assert_eq!(s.message, "描いている間はできません。");
    }
    assert!(s.stencil.image.is_some() && s.stencil.size == DEFAULT_SIZE);
    assert_eq!(s.dialog_request, None);
    // ステンシルを外すと、置き場も読み方も残る
    let stroke = s.stroke.take().unwrap();
    s.doc.cancel_stroke(stroke);
    s.apply(Action::Stencil(StencilOp::Clear));
    assert!(s.stencil.image.is_none());
    assert_eq!(s.stencil.mode, StencilMode::Color);
    assert_eq!(s.message, "ステンシルを外しました。");
    s.apply(Action::Stencil(StencilOp::Pick));
    assert_eq!(s.dialog_request, Some(DialogRequest::OpenStencil));
}

/// 64 × 64 のキャンバスの矩形（拡大 100% でキャンバスにぴったり重なる）。
fn exact_rect() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 64.0))
}

/// キャンバスの横線を、ステンシルを通して描く（ステンシルはキャンバスと 1 対 1 で重ねる）。
fn stencil_line(s: &mut AppState, y: f32) {
    let layer = s.selected_layer.unwrap();
    let stencil = s.canvas_stencil(exact_rect()).unwrap();
    let mut stroke = s.begin_paint_stroke_with(layer, false, stencil).unwrap();
    for i in 0..=28 {
        stroke
            .add_point(
                &mut s.doc,
                4.0 + 2.0 * i as f64,
                y as f64,
                1.0,
                Default::default(),
            )
            .unwrap();
    }
    s.doc.end_stroke(stroke).unwrap();
}

fn half_state(dir: &Path) -> AppState {
    let mut s = AppState::new(64, 64);
    s.stencil.size = 1.0; // キャンバスにぴったり（画像は正方形）
    s.brush.radius = 3.0;
    s.brush.hardness = 1.0;
    s.m2.random_seed = false;
    s.apply(Action::Stencil(StencilOp::Load(half_png(dir, "half.png"))));
    assert!(s.stencil.image.is_some(), "{}", s.message);
    s
}

#[test]
fn headless_a_canvas_stroke_through_the_stencil_paints_only_where_it_is_open_and_undoes_in_one_step(
) {
    let dir = temp_dir("canvas");
    let mut s = half_state(&dir);
    stencil_line(&mut s, 32.0);
    assert_eq!(
        composite_pixel(&s.doc, 10, 32)[3],
        255,
        "白い所（左）は通る"
    );
    assert_eq!(
        composite_pixel(&s.doc, 54, 32)[3],
        0,
        "黒い所（右）は止める"
    );
    assert!(s.doc.can_undo());
    s.apply(Action::Undo);
    assert_eq!(composite_pixel(&s.doc, 10, 32)[3], 0);
    assert!(!s.doc.can_undo(), "ストロークは 1 回の Undo");

    // 反転: 黒が通る
    s.apply(Action::Stencil(StencilOp::Invert(true)));
    stencil_line(&mut s, 32.0);
    assert_eq!(composite_pixel(&s.doc, 10, 32)[3], 0);
    assert_eq!(composite_pixel(&s.doc, 54, 32)[3], 255);
    s.apply(Action::Undo);

    // N を押しているあいだは、ステンシルを使わない（置き場・画像はそのまま）
    s.stencil.ignore_held = true;
    assert!(s.canvas_stencil(exact_rect()).unwrap().is_none());
    s.stencil.ignore_held = false;
    assert!(s.canvas_stencil(exact_rect()).unwrap().is_some());

    // ステンシルを外すと、そのまま全部に塗れる
    s.apply(Action::Stencil(StencilOp::Clear));
    stencil_line(&mut s, 32.0);
    assert_eq!(composite_pixel(&s.doc, 54, 32)[3], 255);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_the_stencil_stays_on_the_screen_when_the_canvas_view_changes() {
    let dir = temp_dir("screen");
    let mut s = half_state(&dir);
    // 画面は 64 × 64 の矩形そのまま。画面を 90° 回すと、キャンバスの左は画面の上へ回る。ステンシルは画面に貼り付いているので、
    // 白い左半分は画面の左のまま（キャンバスの上の位置が変わる）
    s.view.set_angle(90.0);
    let rect = exact_rect();
    let stencil = s.canvas_stencil(rect).unwrap().unwrap();
    let view = s.view.view(rect, 64, 64);
    for (screen, open) in [
        (pos2(10.0, 32.0), true),
        (pos2(54.0, 32.0), false),
        (pos2(8.0, 8.0), true),
        (pos2(56.0, 56.0), false),
    ] {
        let (x, y) = view.to_canvas(screen);
        let sample = stencil
            .sample_canvas(x.floor() as i64, y.floor() as i64)
            .unwrap();
        assert_eq!(sample.amount > 0.5, open, "{screen:?} → ({x}, {y})");
    }
    // 拡大・反転・パンでも同じ
    s.view.flip_horizontally();
    s.view.zoom = 0.5;
    s.view.pan = vec2(5.0, -3.0);
    let stencil = s.canvas_stencil(rect).unwrap().unwrap();
    let view = s.view.view(rect, 64, 64);
    for (screen, open) in [(pos2(10.0, 32.0), true), (pos2(54.0, 32.0), false)] {
        let (x, y) = view.to_canvas(screen);
        let sample = stencil
            .sample_canvas(x.floor() as i64, y.floor() as i64)
            .unwrap();
        assert_eq!(sample.amount > 0.5, open, "{screen:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_the_placement_moves_scales_and_turns_the_image() {
    let dir = temp_dir("placement");
    let mut s = half_state(&dir);
    let rect = exact_rect();
    let open_at = |s: &AppState, x: f32, y: f32| {
        let stencil = s.canvas_stencil(rect).unwrap().unwrap();
        // 画面の点（y は下向き）→ キャンバスの画素（拡大 100% で、y が上向き）
        stencil
            .sample_canvas(x.floor() as i64, (63.0 - y).floor() as i64)
            .unwrap()
            .amount
            > 0.5
    };
    assert!(open_at(&s, 10.0, 32.0) && !open_at(&s, 54.0, 32.0));
    // 時計回りに 90°: 画像の左（白）が画面の上へ
    s.apply(Action::Stencil(StencilOp::Angle(90.0)));
    assert!(open_at(&s, 32.0, 10.0) && !open_at(&s, 32.0, 54.0));
    assert!(!open_at(&s, 10.0, 32.0));
    s.apply(Action::Stencil(StencilOp::Angle(0.0)));
    // 小さくすると、画像の外は塗らない（繰り返さない）
    s.apply(Action::Stencil(StencilOp::Size(0.25)));
    assert!(
        open_at(&s, 28.0, 32.0) && !open_at(&s, 10.0, 32.0),
        "画像の外"
    );
    // 動かす: 中心を左へ
    s.stencil.set_center(0.25, 0.5);
    assert!(open_at(&s, 12.0, 32.0) && !open_at(&s, 20.0, 32.0) && !open_at(&s, 40.0, 32.0));
    // 繰り返す（横）: 外にも画像が並ぶ（幅 16 点ごとに白黒）
    s.apply(Action::Stencil(StencilOp::Tiling(
        StencilTiling::Horizontal,
    )));
    s.stencil.set_center(0.5, 0.5);
    s.apply(Action::Stencil(StencilOp::Size(0.25)));
    let tiled: Vec<bool> = [4.0, 12.0, 20.0, 28.0, 36.0, 44.0, 52.0, 60.0]
        .iter()
        .map(|x| open_at(&s, *x, 32.0))
        .collect();
    assert_eq!(
        tiled,
        [false, true, false, true, false, true, false, true],
        "16 点の画像が横に並ぶ"
    );
    assert!(!open_at(&s, 20.0, 5.0), "縦には繰り返さない");
    let _ = std::fs::remove_dir_all(dir);
}

/// キャンバスの点（x, y）をなぞる 1 本のストロークを、今のステンシルの設定で描く（試験のツール）。
fn run_stroke(s: &mut AppState, eraser: bool, points: &[(f64, f64)]) {
    let layer = s.selected_layer.unwrap();
    let stencil = s.canvas_stencil(exact_rect()).unwrap();
    let mut stroke = s.begin_paint_stroke_with(layer, eraser, stencil).unwrap();
    for &(x, y) in points {
        stroke
            .add_point(&mut s.doc, x, y, 1.0, Default::default())
            .unwrap();
    }
    s.doc.end_stroke(stroke).unwrap();
}

/// キャンバスの 1 行の覆い（左から）。
fn alpha_row(s: &AppState, y: u32) -> Vec<u8> {
    (0..s.doc.width())
        .map(|x| composite_pixel(&s.doc, x, y)[3])
        .collect()
}

/// 消しゴムも、ステンシルを通す: 開いている所だけ消える。
#[test]
fn headless_the_eraser_goes_through_the_stencil_too() {
    let dir = temp_dir("eraser");
    let mut s = half_state(&dir);
    // まず、ステンシル無しで全体に描く
    s.apply(Action::Stencil(StencilOp::Clear));
    stencil_line(&mut s, 32.0);
    assert_eq!(composite_pixel(&s.doc, 54, 32)[3], 255);
    s.apply(Action::Stencil(StencilOp::Load(dir.join("half.png"))));
    // 消しゴム: 左（白い所）だけ消える
    let line: Vec<(f64, f64)> = (0..=28).map(|i| (4.0 + 2.0 * i as f64, 32.0)).collect();
    run_stroke(&mut s, true, &line);
    assert_eq!(composite_pixel(&s.doc, 10, 32)[3], 0, "白い所は消える");
    assert_eq!(composite_pixel(&s.doc, 54, 32)[3], 255, "黒い所は残る");
    s.apply(Action::Undo);
    assert_eq!(composite_pixel(&s.doc, 10, 32)[3], 255);
    let _ = std::fs::remove_dir_all(dir);
}

/// 効果のブラシ（ぼかし）も、ステンシルを通す: 止める側（黒い右半分）の画素は 1 つも変わらず、通す側（白い左半分）の画素は変わる。
/// 縦の縞（ぼかしが効く図）を先に描き、ステンシルを外した同じ操作を対照にして、この試験がステンシルの無視を見分けられることを確かめる。
#[test]
fn headless_the_blur_brush_blurs_only_where_the_stencil_is_open() {
    use yolu_app::m2::{BrushOp, EffectKind};
    let dir = temp_dir("blur");
    let mut s = half_state(&dir);
    // 幅 4・間隔 4 の縦縞を、ステンシル無しで全体に描く
    s.apply(Action::Stencil(StencilOp::Clear));
    s.brush.radius = 2.0;
    for k in 0..8 {
        let x = 2.0 + 8.0 * k as f64;
        let stripe: Vec<(f64, f64)> = (0..=30).map(|i| (x, 2.0 + 2.0 * i as f64)).collect();
        run_stroke(&mut s, false, &stripe);
    }
    s.apply(Action::Stencil(StencilOp::Load(dir.join("half.png"))));
    let before = alpha_row(&s, 32);
    assert!(
        before[0..32].contains(&0) && before[0..32].contains(&255),
        "縞: {before:?}"
    );
    // ぼかしで横切る（画面の左右とも、行 32 を中心に半径 6 の帯）
    s.brush.radius = 6.0;
    s.m2_ui(UiOp::Brush(BrushOp::Effect(EffectKind::Blur)));
    s.message.clear();
    let across: Vec<(f64, f64)> = (0..=28).map(|i| (4.0 + 2.0 * i as f64, 32.0)).collect();
    run_stroke(&mut s, false, &across);
    assert!(s.message.is_empty(), "{}", s.message);
    let through = alpha_row(&s, 32);
    assert_eq!(
        &through[40..60],
        &before[40..60],
        "止める側（黒い右半分）は 1 画素も変わらない"
    );
    assert_ne!(
        &through[4..24],
        &before[4..24],
        "通す側（白い左半分）はぼける"
    );
    s.apply(Action::Undo);
    assert_eq!(alpha_row(&s, 32), before, "1 回の Undo で戻る");
    // 対照: ステンシルを外すと、右半分もぼける（ステンシルを無視する実装だと、上の確かめが落ちる）
    s.apply(Action::Stencil(StencilOp::Clear));
    run_stroke(&mut s, false, &across);
    let plain = alpha_row(&s, 32);
    assert_ne!(
        &plain[40..60],
        &before[40..60],
        "ステンシル無しなら右もぼける"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// PNG の上の行が、画面の上に来る（core の画像は下の行から持つので、読むときに上下を返す）。
#[test]
fn headless_the_top_of_the_image_is_the_top_of_the_screen() {
    let dir = temp_dir("vertical");
    let mut s = AppState::new(64, 64);
    s.stencil.size = 1.0;
    // 上の半分が白（PNG の行 0〜31）、下の半分が黒
    let path = write_png(&dir, "top.png", 64, 64, |_, y| {
        if y < 32 {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        }
    });
    s.apply(Action::Stencil(StencilOp::Load(path)));
    let stencil = s.canvas_stencil(exact_rect()).unwrap().unwrap();
    // 画面の点（y は下向き）→ キャンバスの画素（y は上向き）
    let open =
        |x: i64, screen_y: i64| stencil.sample_canvas(x, 63 - screen_y).unwrap().amount > 0.5;
    assert!(open(32, 10) && !open(32, 54), "画面の上が白");
    // 3D の道（画面の点 → 画像）も同じ向き
    let (_, surface) = s.surface_stencil(exact_rect()).unwrap().unwrap();
    let to_image = surface.screen_to_image();
    let (_, top) = to_image.map_point(32.0, 10.0);
    let (_, bottom) = to_image.map_point(32.0, 54.0);
    assert!(
        top > 32.0 && bottom < 32.0,
        "{top} {bottom}: 画像の y は上向き"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_a_color_stencil_paints_its_colors_and_masks_take_only_the_amount() {
    let dir = temp_dir("color");
    let mut s = AppState::new(64, 64);
    s.stencil.size = 1.0;
    s.brush.radius = 3.0;
    s.brush.hardness = 1.0;
    s.m2.random_seed = false;
    // 左が赤、右が緑（灰色でないので、自動は色）
    let path = write_png(&dir, "rg.png", 64, 64, |x, _| {
        if x < 32 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        }
    });
    s.apply(Action::Stencil(StencilOp::Load(path)));
    assert_eq!(s.stencil.resolved_mode(), Some(StencilMode::Color));
    stencil_line(&mut s, 32.0);
    assert_eq!(composite_pixel(&s.doc, 10, 32), [255, 0, 0, 255]);
    assert_eq!(composite_pixel(&s.doc, 54, 32), [0, 255, 0, 255]);
    s.apply(Action::Undo);
    // スカラーのチャンネル（ラフネス）は、輝度を塗る
    s.m2_ui(UiOp::PaintChannel(Channel::Roughness));
    stencil_line(&mut s, 32.0);
    let red = s.doc.composite_pixel(Channel::Roughness, 10, 32).unwrap();
    assert_eq!((red.r, red.a), (54, 255), "赤の輝度（Rec. 709）");
    s.apply(Action::Undo);
    // 量のモードにすると、色は塗らず量だけ（描画色で塗る）
    s.m2_ui(UiOp::PaintChannel(Channel::Color));
    s.apply(Action::Stencil(StencilOp::Mode(StencilMode::Mask)));
    stencil_line(&mut s, 32.0);
    let (a, b) = (
        composite_pixel(&s.doc, 10, 32),
        composite_pixel(&s.doc, 54, 32),
    );
    assert_eq!(&a[0..3], &[0, 0, 0], "描画色");
    assert!(a[3] > 0 && b[3] > 0, "赤も緑も輝度があるので通る");
    s.apply(Action::Undo);
    // マスクへは、色のモードでも量だけ
    s.apply(Action::Stencil(StencilOp::Mode(StencilMode::Color)));
    let id = s.selected_layer.unwrap();
    s.apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    assert!(s.m2.edit_mask);
    assert!(s.canvas_stencil(exact_rect()).unwrap().is_some());
    stencil_line(&mut s, 32.0);
    let mask = s.doc.layer(id).unwrap().mask().unwrap().surface();
    let px = mask.pixel(10, 32).unwrap();
    assert_eq!((px.r, px.g, px.b), (0, 0, 0), "マスクの色は黒のまま");
    assert!(px.a > 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_the_stencil_is_app_state_and_is_not_saved_in_the_project() {
    let dir = temp_dir("saved");
    let mut s = half_state(&dir);
    s.apply(Action::Stencil(StencilOp::Angle(33.0)));
    stencil_line(&mut s, 32.0);
    let path = dir.join("with.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // .ylp は zip（中の JSON は圧縮される）なので、生のバイト列ではなく、展開した中身で確かめる: 同じ文書をステンシル無しで保存した
    // .ylp と、エントリの名前も中身も（project.json・ylp.json・文書の正本を含めて全部）一致し、画像の名前はどのエントリにも出てこない
    let with = Archive::read(&std::fs::read(&path).unwrap()).unwrap();
    s.apply(Action::Stencil(StencilOp::Clear));
    let bare_path = dir.join("without.ylp");
    s.apply(Action::SaveProjectAs(bare_path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let bare = Archive::read(&std::fs::read(&bare_path).unwrap()).unwrap();
    let names = |a: &Archive| a.entries().keys().cloned().collect::<Vec<_>>();
    assert_eq!(names(&with), names(&bare), "エントリの名前の並び");
    for (name, data) in with.entries() {
        assert_eq!(
            Some(data),
            bare.entries().get(name),
            "{name} の中身はステンシルの有無で変わらない"
        );
        assert!(
            !data.windows(8).any(|w| w == b"half.png"),
            "{name}: ステンシルの画像の名前は .ylp に入らない"
        );
    }
    // 読み込み直したステンシルの置き場へ戻す（上で外したので、もう一度読んで、角度を戻す）
    s.apply(Action::Stencil(StencilOp::Load(dir.join("half.png"))));
    s.apply(Action::Stencil(StencilOp::Angle(33.0)));
    // 別のプロジェクトを開いても、新しいプロジェクトにしても、ステンシルはそのまま（ウィンドウの状態）
    s.apply(Action::NewProject);
    assert!(s.stencil.image.is_some() && s.stencil.angle == 33.0);
    s.apply(Action::OpenProject(path.clone()));
    assert!(s.stencil.image.is_some() && s.stencil.angle == 33.0);
    // 新しく立ち上げたアプリは、ステンシルを持たない
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert!(again.stencil.image.is_none() && again.stencil.angle == 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── プロパティのタブ ─────────

fn open_stencil_tab(h: &mut Harness<'_, YoluApp>) {
    h.state_mut().state.ui.property_tab = 0;
    h.run();
}

#[test]
fn the_stencil_tab_loads_picks_and_edits_without_touching_the_document() {
    let dir = temp_dir("tab");
    let mut h = app(1280.0, 1000.0, 128);
    open_stencil_tab(&mut h);
    assert!(h.query_by_label("画像: なし").is_some());
    // 画像の箱 → 「画像を読む…」: ファイルのウィンドウの頼みが出る（試験ではウィンドウは開かない）
    h.get_by_label("画像: なし").click();
    h.run();
    assert_eq!(
        popup_kind(&h),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::StencilImage))
    );
    let at = popup_item(&h, "画像を読む…").center();
    click(&mut h, at);
    assert_eq!(popup_kind(&h), None);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::OpenStencil)
    );
    // 読んだ後: 箱に名前、設定の行が出る
    load(&mut h, &half_png(&dir, "half.png"));
    assert!(h.query_by_label("画像: half.png").is_some());
    let mode = h.get_by_label("読み方: 自動 (量)");
    mode.click();
    h.run();
    let at = popup_item(&h, "色").center();
    click(&mut h, at);
    assert_eq!(st(&h).mode, StencilMode::Color);
    assert!(h.query_by_label("読み方: 色").is_some());
    // 反転は量のときだけ効く
    h.get_by_label("反転").click();
    h.run();
    assert!(!st(&h).invert, "色のモードでは効かない");
    h.get_by_label("読み方: 色").click();
    h.run();
    let at = popup_item(&h, "量（マスク）").center();
    click(&mut h, at);
    h.get_by_label("反転").click();
    h.run();
    assert!(st(&h).invert);
    // 繰り返し
    h.get_by_label("繰り返さない").click();
    h.run();
    let at = popup_item(&h, "縦横に繰り返す").center();
    click(&mut h, at);
    assert_eq!(st(&h).tiling, StencilTiling::Both);
    // 大きさのスライダー（2〜400%）
    let slider = h.get_by_label("大きさ").rect();
    drag(
        &mut h,
        &[
            pos2(slider.left() + 2.0, slider.center().y),
            pos2(slider.left() + slider.width() * 0.5, slider.center().y),
        ],
    );
    let size = st(&h).size;
    assert!((1.5..2.5).contains(&size), "{size}");
    h.get_by_label("置き場を戻す").click();
    h.run();
    assert_eq!(st(&h).size, DEFAULT_SIZE);
    // 外す
    h.get_by_label("ステンシルを外す").click();
    h.run();
    assert!(st(&h).image.is_none());
    assert!(h.query_by_label("画像: なし").is_some());
    assert!(!h.state().state.doc.can_undo(), "どの操作も文書を変えない");
    assert!(!h.state().state.modified);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_stencil_tab_lists_recent_images_and_a_dropped_png_loads() {
    let dir = temp_dir("recent");
    let mut h = app(1280.0, 1000.0, 128);
    open_stencil_tab(&mut h);
    let (a, b) = (half_png(&dir, "a.png"), ring_png(&dir, "b.png"));
    load(&mut h, &a);
    load(&mut h, &b);
    h.get_by_label("画像: b.png").click();
    h.run();
    let at = popup_item(&h, "a.png").center();
    click(&mut h, at);
    assert_eq!(st(&h).image.as_ref().unwrap().name, "a.png");
    // 外す（ポップアップの「なし」）
    h.get_by_label("画像: a.png").click();
    h.run();
    let at = popup_item(&h, "なし（ステンシルを使わない）").center();
    click(&mut h, at);
    assert!(st(&h).image.is_none());
    // PNG を箱の上へ落とす
    let boxed = h.get_by_label("画像: なし").rect();
    h.event(Event::PointerMoved(boxed.center()));
    h.step();
    h.input_mut().hovered_files.push(egui::HoveredFile {
        path: Some(b.clone()),
        mime: "image/png".into(),
    });
    h.step();
    h.input_mut().hovered_files.clear();
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(b)));
    h.run();
    assert_eq!(
        st(&h).image.as_ref().map(|i| i.name.as_str()),
        Some("b.png")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_stencil_tab_speaks_both_languages() {
    let dir = temp_dir("lang");
    let mut h = app(1280.0, 1000.0, 128);
    open_stencil_tab(&mut h);
    load(&mut h, &color_png(&dir, "color.png"));
    assert!(h.query_by_label("読み方: 自動 (色)").is_some());
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert!(h.query_by_label("Image: color.png").is_some());
    assert!(h.query_by_label("Reads as: Auto (Color)").is_some());
    assert!(h.query_by_label("No tiling").is_some());
    assert!(h.query_by_label("Overlay opacity").is_some());
    assert!(h.query_by_label("Reset placement").is_some());
    assert!(h.query_by_label("Stop using the stencil").is_some());
    h.get_by_label("Reads as: Auto (Color)").click();
    h.run();
    let at = popup_item(&h, "Amount (mask)").center();
    click(&mut h, at);
    assert_eq!(st(&h).mode, StencilMode::Mask);
    h.get_by_label("Image: color.png").click();
    h.run();
    assert!(popup_item(&h, "Load an Image…").width() > 0.0);
    assert!(popup_item(&h, "None (no stencil)").width() > 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn snapshot_the_stencil_tab() {
    let dir = temp_dir("snap-tab");
    let mut h = app(1280.0, 1000.0, 128);
    open_stencil_tab(&mut h);
    h.snapshot("stencil_tab_empty");
    load(&mut h, &color_png(&dir, "gradient.png"));
    h.state_mut().state.stencil.angle = 30.0;
    h.run();
    h.snapshot_options("stencil_tab_color", &rotated_texture());
    load(&mut h, &ring_png(&dir, "ring.png"));
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    h.snapshot_options("stencil_tab_mask_english", &rotated_texture());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── キャンバスの上の重ね表示と Y のドラッグ ─────────

#[test]
fn snapshot_the_overlay_on_the_canvas_with_y_held_and_tiled() {
    let dir = temp_dir("snap-canvas");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &ring_png(&dir, "ring.png"));
    h.state_mut().state.stencil.opacity = 0.6;
    small_brush(&mut h);
    line_across(&mut h, -120.0, 120.0); // 通して塗った後
    key_down(&mut h, Key::Y);
    h.run();
    h.snapshot("stencil_canvas_overlay_held");
    key_up(&mut h, Key::Y);
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Tiling(StencilTiling::Both)));
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Size(0.3)));
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Angle(20.0)));
    h.run();
    h.snapshot_options("stencil_canvas_overlay_tiled", &rotated_texture());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn n_held_hides_the_overlay_and_paints_without_the_stencil() {
    let dir = temp_dir("n");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    small_brush(&mut h);
    h.state_mut().state.stencil.size = 0.5;
    let c = canvas_rect(&h).center();
    // 画像の外（右の遠く）は、ステンシルがあると塗れない
    let far = pos2(c.x + 150.0, c.y);
    line_across(&mut h, 120.0, 180.0);
    assert_eq!(canvas_pixel(&h, far)[3], 0, "画像の外は塗らない");
    assert!(
        !h.state().state.doc.can_undo(),
        "何も塗れなければ Undo の段も無い"
    );
    key_down(&mut h, Key::N);
    assert!(st(&h).ignore_held && !st(&h).shown() && !st(&h).applies());
    line_across(&mut h, 120.0, 180.0);
    assert_eq!(
        canvas_pixel(&h, far),
        [0, 0, 0, 255],
        "N を押しているあいだは使わない"
    );
    key_up(&mut h, Key::N);
    assert!(!st(&h).ignore_held && st(&h).shown());
    // Ctrl+N（新しいプロジェクト）では N を押した扱いにならない
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.event(Event::Key {
        key: Key::N,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.step();
    assert!(!st(&h).ignore_held);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_canvas_stroke_through_the_stencil_follows_the_screen_not_the_canvas_view() {
    let dir = temp_dir("screen-ui");
    let mut h = app(1600.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    small_brush(&mut h);
    {
        let s = &mut h.state_mut().state;
        s.view.zoom = 1.5;
        s.view.set_angle(90.0);
    }
    h.run();
    let r = canvas_rect(&h);
    let (c, hh) = (r.center(), r.height());
    // 画面のステンシルは画像（正方形）の幅が 0.6 × 表示域の高さ。白い左半分は画面の左へ 0.3 × 高さ
    line_across(&mut h, -0.4 * hh, 0.4 * hh);
    let inside = |h: &Harness<'_, YoluApp>, dx: f32| canvas_pixel(h, pos2(c.x + dx * hh, c.y))[3];
    assert_eq!(inside(&h, -0.15), 255, "画面の左の白い所");
    assert_eq!(inside(&h, 0.15), 0, "画面の右の黒い所");
    assert_eq!(inside(&h, -0.38), 0, "画像の外");
    assert!(h.state().state.doc.can_undo());
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(inside(&h, -0.15), 0, "1 回の Undo で戻る");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn t_drag_turns_moves_and_resizes_the_stencil_and_never_paints() {
    let dir = temp_dir("tdrag");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    let r = canvas_rect(&h);
    let c = r.center();
    key_down(&mut h, Key::Y);
    assert!(st(&h).key_held);
    // 左ドラッグ: 回す（中心の右から真下へ = 時計回りに 90°）
    press_with(
        &mut h,
        pos2(c.x + 100.0, c.y),
        PointerButton::Primary,
        Modifiers::NONE,
    );
    assert!(st(&h).drag.is_some());
    for p in [pos2(c.x + 70.7, c.y + 70.7), pos2(c.x, c.y + 100.0)] {
        move_to(&h, p);
        h.step();
    }
    release_at(&mut h, pos2(c.x, c.y + 100.0), PointerButton::Primary);
    assert!((st(&h).angle - 90.0).abs() < 1.0, "{}", st(&h).angle);
    assert!(st(&h).drag.is_none());
    // 中ボタン: 動かす
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    move_to(&h, pos2(c.x + 100.0, c.y - 50.0));
    h.step();
    release_at(&mut h, pos2(c.x + 100.0, c.y - 50.0), PointerButton::Middle);
    let center = st(&h).center;
    assert!((center[0] - (0.5 + 100.0 / r.width())).abs() < 1e-3);
    assert!((center[1] - (0.5 - 50.0 / r.height())).abs() < 1e-3);
    // Ctrl+左も動かす、右ボタンと Alt+左は大きさ
    press_with(&mut h, c, PointerButton::Primary, Modifiers::COMMAND);
    assert_eq!(
        st(&h).drag.map(|d| d.kind),
        Some(yolu_app::stencil::DragKind::Move)
    );
    release_at(&mut h, c, PointerButton::Primary);
    press_with(&mut h, c, PointerButton::Secondary, Modifiers::NONE);
    move_to(&h, pos2(c.x + 100.0, c.y));
    h.step();
    release_at(&mut h, pos2(c.x + 100.0, c.y), PointerButton::Secondary);
    let grown = st(&h).size;
    assert!(grown > DEFAULT_SIZE * 1.5, "{grown}");
    press_with(&mut h, c, PointerButton::Primary, Modifiers::ALT);
    assert_eq!(
        st(&h).drag.map(|d| d.kind),
        Some(yolu_app::stencil::DragKind::Scale)
    );
    move_to(&h, pos2(c.x - 100.0, c.y));
    h.step();
    release_at(&mut h, pos2(c.x - 100.0, c.y), PointerButton::Primary);
    assert!(st(&h).size < grown);
    // ドラッグの最中の Esc は、始めの置き場に戻す
    let before = (st(&h).center, st(&h).size, st(&h).angle);
    press_with(
        &mut h,
        pos2(c.x + 80.0, c.y),
        PointerButton::Middle,
        Modifiers::NONE,
    );
    move_to(&h, pos2(c.x + 200.0, c.y + 100.0));
    h.step();
    assert_ne!(st(&h).center, before.0);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert!(st(&h).drag.is_none());
    assert_eq!((st(&h).center, st(&h).size, st(&h).angle), before);
    release_at(
        &mut h,
        pos2(c.x + 200.0, c.y + 100.0),
        PointerButton::Middle,
    );
    // Y を離してもドラッグはボタンを離すまで続く
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    key_up(&mut h, Key::Y);
    assert!(!st(&h).key_held && st(&h).drag.is_some());
    move_to(&h, pos2(c.x + 30.0, c.y));
    h.step();
    release_at(&mut h, pos2(c.x + 30.0, c.y), PointerButton::Middle);
    assert!(st(&h).drag.is_none());
    // ここまで、描いていない・回していない・パンしていない
    let s = &h.state().state;
    assert!(!s.doc.can_undo() && !s.modified);
    assert_eq!((s.view.angle, s.view.pan), (0.0, vec2(0.0, 0.0)));
    // Y を離したら、ふつうに描ける（ステンシルを通して）
    small_brush(&mut h);
    h.state_mut().state.stencil.reset_placement();
    h.run();
    line_across(&mut h, -100.0, 100.0);
    assert!(h.state().state.doc.can_undo());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_stencil_drag_ends_on_focus_loss_and_a_stroke_is_never_interrupted() {
    let dir = temp_dir("focus");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    let c = canvas_rect(&h).center();
    key_down(&mut h, Key::Y);
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    move_to(&h, pos2(c.x + 40.0, c.y));
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(st(&h).drag.is_none() && !st(&h).key_held);
    assert!(st(&h).center[0] > 0.5, "今の置き場のまま終わる");
    release_at(&mut h, pos2(c.x + 40.0, c.y), PointerButton::Middle);
    // 描いている最中に Y を押しても、ドラッグは始まらない
    h.event(Event::WindowFocused(true));
    h.step();
    small_brush(&mut h);
    h.state_mut().state.stencil.reset_placement();
    press(&h, pos2(c.x - 40.0, c.y), PointerButton::Primary);
    h.step();
    assert!(h.state().state.is_stroking());
    key_down(&mut h, Key::Y);
    press_with(
        &mut h,
        pos2(c.x, c.y),
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert!(st(&h).drag.is_none(), "ストロークの最中は始めない");
    release_at(&mut h, pos2(c.x, c.y), PointerButton::Secondary);
    release_at(&mut h, pos2(c.x - 40.0, c.y), PointerButton::Primary);
    key_up(&mut h, Key::Y);
    h.run();
    assert!(!h.state().state.is_stroking());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn t_drag_without_an_image_says_so_and_does_not_paint() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    key_down(&mut h, Key::Y);
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_none());
    assert_eq!(h.state().state.message, "ステンシルの画像がありません。");
    release_at(&mut h, c, PointerButton::Primary);
    assert!(!h.state().state.doc.can_undo());
}

// ───────── 3D のビュー ─────────

fn cube_view(doc: u32) -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app(1100.0, 760.0, doc);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

/// モデルの空間の点の、ウィンドウの画面の点。
fn cube_screen(h: &Harness<'_, YoluApp>, rect: Rect, p: yolu_core::glam::Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

/// モデルの空間の 2 点の間を、画面の上で等間隔に結ぶ点（画面の点）。
fn cube_segment(
    h: &Harness<'_, YoluApp>,
    rect: Rect,
    a: yolu_core::glam::Vec3,
    b: yolu_core::glam::Vec3,
) -> Vec<Pos2> {
    let (from, to) = (cube_screen(h, rect, a), cube_screen(h, rect, b));
    (0..=16)
        .map(|i| from + (to - from) * (i as f32 / 16.0))
        .collect()
}

/// 手前の面を、画面の中心をまたいで横に描く点（画面の点）。
fn cube_points(h: &Harness<'_, YoluApp>, rect: Rect) -> Vec<Pos2> {
    use yolu_core::glam::Vec3;
    cube_segment(
        h,
        rect,
        Vec3::new(-0.45, 0.1, -0.5),
        Vec3::new(0.45, 0.1, -0.5),
    )
}

/// 手前の面を、画面の中心をまたいで横に描く。
fn cube_line(h: &mut Harness<'_, YoluApp>, rect: Rect) {
    let points = cube_points(h, rect);
    drag(h, &points);
}

fn painted(h: &Harness<'_, YoluApp>) -> BTreeSet<(u32, u32)> {
    let doc = &h.state().state.doc;
    let mut out = BTreeSet::new();
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if composite_pixel(doc, x, y)[3] > 0 {
                out.insert((x, y));
            }
        }
    }
    out
}

#[test]
fn a_3d_stroke_goes_through_the_stencil_laid_over_the_view_and_undoes_in_one_step() {
    let dir = temp_dir("cube");
    let path = half_png(&dir, "half.png");
    let plain = {
        let (mut h, rect) = cube_view(256);
        h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
        cube_line(&mut h, rect);
        painted(&h)
    };
    assert!(plain.len() > 300, "{}", plain.len());
    let (mut h, rect) = cube_view(256);
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    load(&mut h, &path);
    // 画像は表示域の高さの 0.9 倍の正方形。白い左半分と黒い右半分の境が、線の真ん中を通るように貼る
    let mid = {
        use yolu_core::glam::Vec3;
        let view = h
            .state()
            .state
            .view3d
            .camera
            .view(rect.width(), rect.height());
        view.to_screen(Vec3::new(0.0, 0.1, -0.5))
            .expect("カメラの前")
    };
    h.state_mut().state.stencil.size = 0.9;
    h.state_mut()
        .state
        .stencil
        .set_center(mid.x / rect.width(), mid.y / rect.height());
    h.state_mut().state.message.clear();
    h.run();
    cube_line(&mut h, rect);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    let through = painted(&h);
    let n = plain.len();
    assert!(
        through.len() > n / 4 && through.len() < n * 3 / 4,
        "{} / {n}",
        through.len()
    );
    assert!(
        through.is_subset(&plain),
        "通した画素は、通さないときに塗られる画素の一部"
    );
    assert!(h.state().state.doc.can_undo());
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted(&h).is_empty(), "1 回の Undo で戻る");
    // 反転すると、残りの側が通る（境の滲みのほかは重ならず、合わせて全部）
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Invert(true)));
    h.run();
    cube_line(&mut h, rect);
    let inverted = painted(&h);
    assert!(
        inverted.len() > n / 4 && inverted.len() < n * 3 / 4,
        "{}",
        inverted.len()
    );
    assert!(
        inverted.intersection(&through).count() < n / 6,
        "境のあたりだけ重なる"
    );
    assert!(
        inverted.union(&through).count() * 100 >= n * 95,
        "合わせて全部"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// ステンシルを通した結果が、画面の白い側の画素だけであること（黒い側の画素は 1 つも無く、白い側の画素は 9 割以上ある）。
/// 白い側・黒い側の画素は、ステンシル無しで画面の境から離れた線を描いて測る。
fn assert_only_the_white_side(
    through: &BTreeSet<(u32, u32)>,
    white: &BTreeSet<(u32, u32)>,
    black: &BTreeSet<(u32, u32)>,
    what: &str,
) {
    assert!(
        white.len() > 40 && black.len() > 40,
        "{what}: 測った画素が少ない {} / {}",
        white.len(),
        black.len()
    );
    assert!(
        white.is_disjoint(black),
        "{what}: 測った線が境をまたいでいる"
    );
    assert!(!through.is_empty(), "{what}: 何も塗られない");
    assert_eq!(
        through.intersection(black).count(),
        0,
        "{what}: 黒い側が塗られた"
    );
    let hit = through.intersection(white).count();
    assert!(
        hit * 10 >= white.len() * 9,
        "{what}: 白い側が塗られない {hit} / {}",
        white.len()
    );
}

/// 描いて、塗られた画素を取り、1 回の Undo で戻す。
fn stroke_and_undo(h: &mut Harness<'_, YoluApp>, points: &[Pos2]) -> BTreeSet<(u32, u32)> {
    drag(h, points);
    let out = painted(h);
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted(h).is_empty(), "1 回の Undo で戻る");
    out
}

#[test]
fn a_3d_stencil_opens_the_side_of_the_screen_it_covers_left_right_and_top_bottom() {
    use yolu_core::glam::Vec3;
    let dir = temp_dir("cube-orient");
    let left_white = half_png(&dir, "left.png");
    let top_white = write_png(&dir, "top.png", 64, 64, |_, y| {
        if y < 32 {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        }
    });
    let (mut h, rect) = cube_view(256);
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    small_brush(&mut h);
    let v = |x: f32, y: f32| Vec3::new(x, y, -0.5);
    // 画像の真ん中（左右・上下の境）を、手前の面の中心に置く。画像は表示域の高さの 0.9 倍の正方形
    let center = cube_screen(&h, rect, v(0.0, 0.0));
    // 前提: 線の左・右・上・下の端は、境（中心）からそれぞれ離れた側にある
    let reach = |dx: f32, dy: f32| cube_screen(&h, rect, v(dx, dy)) - center;
    assert!(reach(-0.2, 0.0).x < -20.0 && reach(0.2, 0.0).x > 20.0);
    assert!(reach(0.0, 0.2).y < -20.0 && reach(0.0, -0.2).y > 20.0);
    // ステンシル無しで、境から離れた 4 本の線が塗る画素を測る
    let seg = |h: &Harness<'_, YoluApp>, a: Vec3, b: Vec3| cube_segment(h, rect, a, b);
    let (p, q) = (
        seg(&h, v(-0.45, 0.0), v(-0.2, 0.0)),
        seg(&h, v(0.2, 0.0), v(0.45, 0.0)),
    );
    let (left, right) = (stroke_and_undo(&mut h, &p), stroke_and_undo(&mut h, &q));
    let (p, q) = (
        seg(&h, v(0.0, 0.4), v(0.0, 0.2)),
        seg(&h, v(0.0, -0.2), v(0.0, -0.4)),
    );
    let (top, bottom) = (stroke_and_undo(&mut h, &p), stroke_and_undo(&mut h, &q));
    // 左右: 白い左半分を通すと左の画素だけ、反転すると右の画素だけ
    load(&mut h, &left_white);
    h.state_mut().state.stencil.size = 0.9;
    h.state_mut().state.stencil.set_center(
        (center.x - rect.left()) / rect.width(),
        (center.y - rect.top()) / rect.height(),
    );
    h.run();
    let across = seg(&h, v(-0.45, 0.0), v(0.45, 0.0));
    let through = stroke_and_undo(&mut h, &across);
    assert_only_the_white_side(&through, &left, &right, "白が左");
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Invert(true)));
    let through = stroke_and_undo(&mut h, &across);
    assert_only_the_white_side(&through, &right, &left, "白が左を反転");
    // 上下: 白い上半分を通すと上の画素だけ、反転すると下の画素だけ
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Invert(false)));
    load(&mut h, &top_white);
    h.run();
    let down = seg(&h, v(0.0, 0.4), v(0.0, -0.4));
    let through = stroke_and_undo(&mut h, &down);
    assert_only_the_white_side(&through, &top, &bottom, "白が上");
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Invert(true)));
    let through = stroke_and_undo(&mut h, &down);
    assert_only_the_white_side(&through, &bottom, &top, "白が上を反転");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn t_drag_works_over_the_3d_view_too_and_the_overlay_follows_the_screen() {
    let dir = temp_dir("cube-drag");
    let (mut h, rect) = cube_view(256);
    load(&mut h, &ring_png(&dir, "ring.png"));
    let c = rect.center();
    key_down(&mut h, Key::Y);
    press_with(
        &mut h,
        pos2(c.x + 100.0, c.y),
        PointerButton::Primary,
        Modifiers::NONE,
    );
    move_to(&h, pos2(c.x, c.y + 100.0));
    h.step();
    release_at(&mut h, pos2(c.x, c.y + 100.0), PointerButton::Primary);
    assert!((st(&h).angle - 90.0).abs() < 1.0, "{}", st(&h).angle);
    // カメラを回しても（右ドラッグ）、置き場は変わらない
    key_up(&mut h, Key::Y);
    let placement = (st(&h).center, st(&h).size, st(&h).angle);
    press_with(&mut h, c, PointerButton::Secondary, Modifiers::NONE);
    move_to(&h, pos2(c.x + 60.0, c.y + 20.0));
    h.step();
    release_at(
        &mut h,
        pos2(c.x + 60.0, c.y + 20.0),
        PointerButton::Secondary,
    );
    assert_ne!(h.state().state.view3d.camera.yaw, -40.0);
    assert_eq!((st(&h).center, st(&h).size, st(&h).angle), placement);
    assert!(!h.state().state.doc.can_undo(), "ドラッグでは描かない");
    assert!(st(&h).has_overlay_texture());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn snapshot_the_overlay_over_the_3d_view() {
    let dir = temp_dir("snap-3d");
    let (mut h, rect) = cube_view(256);
    load(&mut h, &ring_png(&dir, "ring.png"));
    h.state_mut().state.stencil.opacity = 0.55;
    h.state_mut().state.stencil.size = 0.7;
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    h.run();
    cube_line(&mut h, rect);
    key_down(&mut h, Key::Y);
    h.run();
    h.snapshot("stencil_view3d_overlay");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_overlay_texture_is_made_only_when_an_image_is_shown() {
    let dir = temp_dir("overlay-texture");
    let mut h = app(1280.0, 800.0, 256);
    h.run();
    assert!(!st(&h).has_overlay_texture(), "画像が無ければ絵を作らない");
    load(&mut h, &half_png(&dir, "half.png"));
    assert!(st(&h).has_overlay_texture());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── ドラッグの最中・ペン・ポップアップ・ポーズ・ポインタの形 ─────────

fn pen_sample(at: Pos2, contact: bool) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 7,
        time_ms: 0,
    }
}

/// ペンで points をなぞる（ペンの点を全部渡し、最後に浮かせる。1 フレームで届く）。
fn pen_line(h: &mut Harness<'_, YoluApp>, points: &[Pos2]) {
    for p in points {
        h.state().pen().push(pen_sample(*p, true));
    }
    h.state()
        .pen()
        .push(pen_sample(*points.last().unwrap(), false));
    h.run();
}

/// ドラッグの最中にほかのボタンを押しても、ストロークも、キャンバスのパンも始まらない。ほかのボタンの離しは、キャンバスへ通る
/// （ドラッグの前から押していたボタンの状態を、キャンバスが戻せるように）。
#[test]
fn a_second_button_during_a_stencil_drag_starts_nothing() {
    let dir = temp_dir("second-button");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    small_brush(&mut h);
    let c = canvas_rect(&h).center();
    let moved = pos2(c.x + 40.0, c.y);
    key_down(&mut h, Key::Y);
    // 中ボタンで動かしている最中に左を押す: ストロークは始まらず、ドラッグは続く
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(!h.state().state.is_stroking(), "ストロークが始まった");
    move_to(&h, moved);
    h.step();
    assert!(st(&h).center[0] > 0.5, "ドラッグは続く");
    release_at(&mut h, moved, PointerButton::Primary);
    assert!(st(&h).drag.is_some(), "ドラッグのボタンの離しで終わる");
    release_at(&mut h, moved, PointerButton::Middle);
    assert!(st(&h).drag.is_none());
    {
        let s = &h.state().state;
        assert!(!s.doc.can_undo() && !s.modified && s.canvas.stroke.is_none());
    }
    // 左で回している最中に中を押す: パンは立たず、中を離したあと、ボタンを押さずに動かしてもキャンバスは動かない
    let view_before = (h.state().state.view.angle, h.state().state.view.pan);
    press_with(
        &mut h,
        pos2(c.x + 100.0, c.y),
        PointerButton::Primary,
        Modifiers::NONE,
    );
    assert_eq!(
        st(&h).drag.map(|d| d.kind),
        Some(yolu_app::stencil::DragKind::Rotate)
    );
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    assert!(!h.state().state.canvas.panning, "パンが始まった");
    release_at(&mut h, c, PointerButton::Middle);
    assert!(!h.state().state.canvas.panning);
    assert!(st(&h).drag.is_some(), "回すドラッグは続く");
    release_at(&mut h, c, PointerButton::Primary);
    assert!(st(&h).drag.is_none());
    for p in [pos2(c.x + 30.0, c.y + 30.0), pos2(c.x - 20.0, c.y + 50.0)] {
        move_to(&h, p);
        h.step();
    }
    let s = &h.state().state;
    assert_eq!(
        (s.view.angle, s.view.pan),
        view_before,
        "キャンバスは動かない"
    );
    // ドラッグの前から押していた中ボタン（パン）は、ドラッグの最中に離しても戻る
    key_up(&mut h, Key::Y);
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    assert!(h.state().state.canvas.panning);
    key_down(&mut h, Key::Y);
    press_with(
        &mut h,
        pos2(c.x + 100.0, c.y),
        PointerButton::Primary,
        Modifiers::NONE,
    );
    assert!(st(&h).drag.is_some());
    release_at(&mut h, c, PointerButton::Middle);
    assert!(!h.state().state.canvas.panning, "離しはキャンバスへ届く");
    release_at(&mut h, c, PointerButton::Primary);
    assert!(st(&h).drag.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_second_button_during_a_stencil_drag_starts_nothing_over_the_3d_view_too() {
    let dir = temp_dir("second-button-3d");
    let (mut h, rect) = cube_view(256);
    load(&mut h, &half_png(&dir, "half.png"));
    let c = rect.center();
    key_down(&mut h, Key::Y);
    // 中で動かしている最中に左を押す: ストロークは始まらない
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(!h.state().state.is_stroking(), "ストロークが始まった");
    assert!(h.state().state.view3d.input.stroke.is_none());
    release_at(&mut h, c, PointerButton::Primary);
    release_at(&mut h, c, PointerButton::Middle);
    assert!(st(&h).drag.is_none());
    // 左で回している最中に中を押す: カメラは動き出さない
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_some());
    press_with(&mut h, c, PointerButton::Middle, Modifiers::NONE);
    assert!(
        h.state().state.view3d.input.nav.is_none(),
        "カメラが動き出した"
    );
    let yaw = h.state().state.view3d.camera.yaw;
    move_to(&h, pos2(c.x + 30.0, c.y + 10.0));
    h.step();
    assert_eq!(h.state().state.view3d.camera.yaw, yaw);
    release_at(&mut h, c, PointerButton::Middle);
    release_at(&mut h, c, PointerButton::Primary);
    assert!(st(&h).drag.is_none());
    assert!(!h.state().state.doc.can_undo() && !h.state().state.modified);
    let _ = std::fs::remove_dir_all(dir);
}

/// ペン: Y を押しているあいだは、ペンを置いて動かすとステンシルの置き場が動いて、描かない。
#[test]
fn a_pen_with_y_held_moves_the_stencil_and_does_not_paint() {
    let dir = temp_dir("pen-t");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    small_brush(&mut h);
    let c = canvas_rect(&h).center();
    key_down(&mut h, Key::Y);
    // ペンを置く（Windows ではペンはマウスのボタンの知らせも出す）→ 回すドラッグが始まる
    let down = pos2(c.x + 100.0, c.y);
    h.state().pen().push(pen_sample(down, true));
    press_with(&mut h, down, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_some(), "ドラッグが始まらない");
    assert!(!h.state().state.is_stroking(), "ペンが描き始めた");
    let to = pos2(c.x, c.y + 100.0);
    h.state().pen().push(pen_sample(to, true));
    move_to(&h, to);
    h.step();
    assert!((st(&h).angle - 90.0).abs() < 1.0, "{}", st(&h).angle);
    h.state().pen().push(pen_sample(to, false));
    release_at(&mut h, to, PointerButton::Primary);
    assert!(st(&h).drag.is_none());
    let s = &h.state().state;
    assert!(!s.doc.can_undo() && !s.modified, "文書は変わらない");
    let _ = std::fs::remove_dir_all(dir);
}

/// ペン: Y を押していなければ、ステンシルを通した画素だけが塗れて、1 回の Undo で戻る（2D）。
#[test]
fn a_pen_stroke_goes_through_the_stencil_on_the_canvas_and_undoes_in_one_step() {
    let dir = temp_dir("pen-canvas");
    let mut h = app(1600.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    small_brush(&mut h);
    let r = canvas_rect(&h);
    let (c, hh) = (r.center(), r.height());
    // 画面のステンシルは画像（正方形）の幅が 0.6 × 表示域の高さ。白い左半分は画面の左へ 0.3 × 高さ
    let n = 12;
    let (from, to) = (-0.4 * hh, 0.4 * hh);
    let points: Vec<Pos2> = (0..=n)
        .map(|i| pos2(c.x + from + (to - from) * i as f32 / n as f32, c.y))
        .collect();
    pen_line(&mut h, &points);
    let inside = |h: &Harness<'_, YoluApp>, dx: f32| canvas_pixel(h, pos2(c.x + dx * hh, c.y))[3];
    assert_eq!(inside(&h, -0.15), 255, "画面の左の白い所");
    assert_eq!(inside(&h, 0.15), 0, "画面の右の黒い所");
    assert_eq!(inside(&h, -0.38), 0, "画像の外");
    assert!(h.state().state.doc.can_undo());
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(inside(&h, -0.15), 0, "1 回の Undo で戻る");
    assert!(!h.state().state.doc.can_undo());
    let _ = std::fs::remove_dir_all(dir);
}

/// ペン（3D）: Y を押しているあいだは置き場が動いて描かず、押していなければ、ステンシルを通した画素だけが塗れて、1 回の Undo で戻る。
#[test]
fn a_pen_over_the_3d_view_moves_the_stencil_with_y_and_paints_through_it_without() {
    let dir = temp_dir("pen-cube");
    let path = half_png(&dir, "half.png");
    let plain = {
        let (mut h, rect) = cube_view(256);
        h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
        let points = cube_points(&h, rect);
        pen_line(&mut h, &points);
        painted(&h)
    };
    assert!(plain.len() > 300, "{}", plain.len());
    let (mut h, rect) = cube_view(256);
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    load(&mut h, &path);
    // 白い左半分と黒い右半分の境が、線の真ん中を通るように貼る（マウスの試験と同じ）
    let mid = {
        use yolu_core::glam::Vec3;
        let view = h
            .state()
            .state
            .view3d
            .camera
            .view(rect.width(), rect.height());
        view.to_screen(Vec3::new(0.0, 0.1, -0.5))
            .expect("カメラの前")
    };
    h.state_mut().state.stencil.size = 0.9;
    h.state_mut()
        .state
        .stencil
        .set_center(mid.x / rect.width(), mid.y / rect.height());
    h.run();
    let points = cube_points(&h, rect);
    // Y を押しているあいだ: 置き場が動き、描かない
    key_down(&mut h, Key::Y);
    let before = st(&h).center;
    h.state().pen().push(pen_sample(points[2], true));
    press_with(&mut h, points[2], PointerButton::Middle, Modifiers::NONE);
    assert!(st(&h).drag.is_some(), "ドラッグが始まらない");
    assert!(!h.state().state.is_stroking(), "ペンが描き始めた");
    h.state().pen().push(pen_sample(points[8], true));
    move_to(&h, points[8]);
    h.step();
    assert_ne!(st(&h).center, before, "置き場が動く");
    h.state().pen().push(pen_sample(points[8], false));
    release_at(&mut h, points[8], PointerButton::Middle);
    assert!(painted(&h).is_empty() && !h.state().state.doc.can_undo());
    // 置き場を戻して、Y を離し、ペンで描く
    key_up(&mut h, Key::Y);
    h.state_mut()
        .state
        .stencil
        .set_center(mid.x / rect.width(), mid.y / rect.height());
    h.run();
    pen_line(&mut h, &points);
    let through = painted(&h);
    let n = plain.len();
    assert!(
        through.len() > n / 4 && through.len() < n * 3 / 4,
        "{} / {n}",
        through.len()
    );
    assert!(
        through.is_subset(&plain),
        "通した画素は、通さないときに塗られる画素の一部"
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted(&h).is_empty(), "1 回の Undo で戻る");
    let _ = std::fs::remove_dir_all(dir);
}

/// ポップアップが開いているあいだは、Y を押してキャンバスを押しても、ステンシルのドラッグは始まらない。
#[test]
fn a_stencil_drag_does_not_start_while_a_popup_is_open() {
    use egui_kittest::kittest::Queryable;
    let dir = temp_dir("popup");
    let mut h = app(1280.0, 800.0, 256);
    load(&mut h, &half_png(&dir, "half.png"));
    let c = canvas_rect(&h).center();
    key_down(&mut h, Key::Y);
    assert!(st(&h).key_held);
    // レイヤーの合成モードのドロップダウンを開く（Y を押したまま）
    h.get_by_label("通常").click();
    h.run();
    assert!(popup_kind(&h).is_some(), "ポップアップが開いた");
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(
        st(&h).drag.is_none(),
        "ポップアップの上のクリックでドラッグが始まった"
    );
    release_at(&mut h, c, PointerButton::Primary);
    h.run();
    assert!(popup_kind(&h).is_none(), "外のクリックで閉じる");
    // 閉じたあとは、始まる（上で始まらなかったのがポップアップのせいだと確かめる）
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_some());
    release_at(&mut h, c, PointerButton::Primary);
    let _ = std::fs::remove_dir_all(dir);
}

/// ドラッグを始める条件の 1 つずつ（ポップアップ・直前のフレームのポップアップ・ストローク・表示域の外）は、ほかが揃っていても始めさせない。
#[test]
fn headless_each_blocker_alone_stops_a_stencil_drag_from_starting() {
    let dir = temp_dir("blockers");
    let mut s = half_state(&dir);
    s.stencil.key_held = true;
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
    let press = Event::PointerButton {
        pos: pos2(200.0, 150.0),
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    let ctx = egui::Context::default();
    let popup = || OpenPopup {
        kind: PopupKind::SetContext(0),
        state: PopupState::new(&ctx, Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0))),
    };
    let try_start = |s: &mut AppState, over: bool| {
        let taken = yolu_app::stencil::handle_event(s, &press, rect, over, &Modifiers::NONE);
        (taken, s.stencil.drag.take().is_some())
    };
    // 何も邪魔が無ければ始まる（対照）
    assert_eq!(try_start(&mut s, true), (true, true));
    // 表示域の外
    assert_eq!(try_start(&mut s, false), (false, false), "表示域の外");
    // 開いているポップアップ
    s.popup = Some(popup());
    assert_eq!(try_start(&mut s, true), (false, false), "ポップアップ");
    s.popup = None;
    // 直前のフレームで開いていたポップアップ（このフレームの外のクリックで閉じたもの）
    s.ui.popup_was_open = true;
    assert_eq!(
        try_start(&mut s, true),
        (false, false),
        "直前のポップアップ"
    );
    s.ui.popup_was_open = false;
    // ストロークの最中
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke_with(layer, false, None).unwrap();
    s.stroke = Some(stroke);
    assert_eq!(try_start(&mut s, true), (false, false), "ストローク");
    s.stroke = None;
    s.doc.cancel_active_stroke();
    assert_eq!(try_start(&mut s, true), (true, true), "全部外すと始まる");
    let _ = std::fs::remove_dir_all(dir);
}

/// 編集のモードの 3D ビューでは、Y を押しながらドラッグしても置き場は動かず、重ね表示も出ない（ギズモと物を選ぶ操作が優先）。
/// 試しの立方体はボーンが無いので、編集のモードで確かめる（ポーズのモードも同じ `EditorMode::paints` を見る。2D は次の試験）。
#[test]
fn the_stencil_is_not_moved_or_shown_in_edit_mode_in_3d() {
    let dir = temp_dir("pose");
    let (mut h, rect) = cube_view(256);
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Edit)));
    h.run();
    load(&mut h, &ring_png(&dir, "ring.png"));
    assert!(
        !st(&h).has_overlay_texture(),
        "編集のモードでは重ね表示を描かない"
    );
    let placement = (st(&h).center, st(&h).size, st(&h).angle);
    let c = rect.center();
    key_down(&mut h, Key::Y);
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_none(), "編集のモードでドラッグが始まった");
    move_to(&h, pos2(c.x + 60.0, c.y + 30.0));
    h.step();
    release_at(&mut h, c, PointerButton::Primary);
    assert_eq!((st(&h).center, st(&h).size, st(&h).angle), placement);
    assert!(!st(&h).has_overlay_texture());
    // ペイントのモードへ戻ると、同じ操作でドラッグが始まり、重ね表示が出る
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
    h.run();
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_some());
    assert!(st(&h).has_overlay_texture());
    release_at(&mut h, c, PointerButton::Primary);
    let _ = std::fs::remove_dir_all(dir);
}

/// 編集・ポーズのモードの 2D のキャンバスでも、Y を押しながらドラッグしても置き場は動かず、重ね表示も出ない（3D と同じ）。
#[test]
fn the_stencil_is_not_moved_or_shown_on_the_canvas_in_edit_and_pose_modes() {
    let dir = temp_dir("modes-2d");
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut()
        .apply(Action::Pose(yolu_app::view3d::pose::PoseAction::LoadFigure));
    h.run();
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let c = canvas_rect(&h).center();
    for mode in [EditorMode::Edit, EditorMode::Pose] {
        h.state_mut().apply(Action::Mode(ModeAction::Set(mode)));
        h.run();
        assert_eq!(h.state().state.mode, mode, "{}", h.state().state.message);
        if st(&h).image.is_none() {
            load(&mut h, &ring_png(&dir, "ring.png"));
        }
        assert!(
            !st(&h).has_overlay_texture(),
            "{mode:?}: 重ね表示を描かない"
        );
        let placement = (st(&h).center, st(&h).size, st(&h).angle);
        key_down(&mut h, Key::Y);
        assert!(!st(&h).key_held, "{mode:?}: Y が効かない");
        press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
        assert!(st(&h).drag.is_none(), "{mode:?}: ドラッグが始まった");
        move_to(&h, pos2(c.x + 60.0, c.y + 30.0));
        h.step();
        release_at(&mut h, pos2(c.x + 60.0, c.y + 30.0), PointerButton::Primary);
        key_up(&mut h, Key::Y);
        assert_eq!((st(&h).center, st(&h).size, st(&h).angle), placement);
        assert!(!st(&h).has_overlay_texture());
        assert!(!h.state().state.doc.can_undo(), "{mode:?}: 描かない");
    }
    // ペイントのモードへ戻ると、同じ操作でドラッグが始まり、重ね表示が出る
    h.state_mut()
        .apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
    h.run();
    assert!(st(&h).has_overlay_texture());
    key_down(&mut h, Key::Y);
    press_with(&mut h, c, PointerButton::Primary, Modifiers::NONE);
    assert!(st(&h).drag.is_some());
    release_at(&mut h, c, PointerButton::Primary);
    key_up(&mut h, Key::Y);
    let _ = std::fs::remove_dir_all(dir);
}

/// ステンシルの画像が無いときは、Y を押してもブラシのカーソルのまま（ポインタを動かす形に替えるのは、画像があるときだけ。Unity 版と同じ）。
#[test]
fn the_pointer_changes_with_y_only_when_there_is_an_image() {
    let dir = temp_dir("cursor");
    let icon = |h: &Harness<'_, YoluApp>| h.output().platform_output.cursor_icon;
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    move_to(&h, c);
    h.step();
    assert_eq!(
        icon(&h),
        egui::CursorIcon::None,
        "ブラシの円（システムのポインタは隠す）"
    );
    key_down(&mut h, Key::Y);
    assert!(st(&h).key_held);
    assert_eq!(
        icon(&h),
        egui::CursorIcon::None,
        "画像が無ければ、Y を押してもブラシのまま"
    );
    load(&mut h, &half_png(&dir, "half.png"));
    move_to(&h, c);
    h.step();
    assert_eq!(icon(&h), egui::CursorIcon::Move, "画像があれば、動かす形");
    key_up(&mut h, Key::Y);
    assert_eq!(icon(&h), egui::CursorIcon::None, "Y を離すとブラシ");
    // 3D のビューも同じ
    let (mut h, rect) = cube_view(256);
    let c = rect.center();
    move_to(&h, c);
    h.step();
    let brush = icon(&h);
    assert_ne!(brush, egui::CursorIcon::Move);
    key_down(&mut h, Key::Y);
    assert_eq!(icon(&h), brush, "画像が無ければ、Y を押してもブラシのまま");
    load(&mut h, &half_png(&dir, "half.png"));
    move_to(&h, c);
    h.step();
    assert_eq!(icon(&h), egui::CursorIcon::Move);
    let _ = std::fs::remove_dir_all(dir);
}
