//! 2D のキャンバスと 3D ビューの描き始めの表（D1〜D24）: 同じ文書の同じ場面を 2D と 3D で動かし（文書は、多くの D が試しの立方体の 256 角。
//! D1 の後半と D21 のポーズは試しの人形の 128 角、D3 は 2 つのテクスチャセットの立方体 `two_materials`、D8 はモデルの無い文書）、断りの有る無し・
//! 断りの文・出どころ（`Source`）・描いた物・残るストロークを、1 つの D に 1 つのテストで確かめる。
//!
//! 期待は今の振る舞い（2D と 3D で揃っていない所も、今の値をそのまま書く）。2D と 3D を揃えたら、その D のテストの期待を書き換える。
//! 2D と 3D は別々に確かめ、2D と 3D を比べる `assert_eq!` は置かない（片方を揃えたとき、変わるのは片方の行だけ）。
//! 入力はペンを主にし（筆圧・接触・離し）、マウスは押しの持ち主とボタンの違いを見る所だけで使う。
use crate::common;
use crate::common::drive::Drive;
use crate::parity_input::{hold, sample, texel_at, thin_brush, world};
use crate::view3d_brush::{cube_view, press_with, release_with};

use common::*;
use egui::{pos2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::{BrushEffect, Rgba8};
use yolu_app::notice::{Kind, Source};
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Canvas,
    View3d,
}

const VIEWS: [View; 2] = [View::Canvas, View::View3d];

/// 同じ文書（試しの立方体を読んだ 256 角の文書・レイヤー 1 つ・細い硬いブラシ）を、2D のキャンバスか 3D ビューで前に出す。
fn scene(view: View) -> (H, Rect) {
    let (mut h, rect) = cube_view();
    thin_brush(&mut h);
    match view {
        View::View3d => (h, rect),
        View::Canvas => {
            click_tab(&mut h, Tab::Canvas);
            h.run();
            let rect = canvas_rect(&h);
            (h, rect)
        }
    }
}

/// 押す点（`x`・`y` は −1〜1）: 3D は手前の面（−Z）の上、2D は文書の真ん中のまわり（文書の幅の ±30%）。
fn spot(h: &H, view: View, rect: Rect, x: f32, y: f32) -> Pos2 {
    match view {
        View::View3d => world(h, rect, x * 0.4, y * 0.4),
        View::Canvas => {
            let s = h.app();
            let (w, hh) = (s.doc().width(), s.doc().height());
            s.view().view(rect, w, hh).to_screen(
                w as f64 * (0.5 + 0.3 * x as f64),
                hh as f64 * (0.5 + 0.3 * y as f64),
            )
        }
    }
}

/// 横に短い線（6 点）。
fn line(h: &H, view: View, rect: Rect) -> Vec<Pos2> {
    let (a, b) = (
        spot(h, view, rect, -0.5, 0.0),
        spot(h, view, rect, 0.5, 0.0),
    );
    (0..6).map(|i| a + (b - a) * (i as f32 / 5.0)).collect()
}

/// 離したあと、3D の持ち越しを塗り終えて確定するまで回す。
fn settle(h: &mut H) {
    let mut frames = 0;
    while h.app().is_stroking() && frames < 2000 {
        h.step();
        frames += 1;
    }
    h.run();
}

/// ペンで `points` をなぞる（修飾 `m` を押したまま。触れる → 動く → 離す、1 点ごとに 1 フレーム）。
fn pen_stroke(h: &mut H, points: &[Pos2], m: Modifiers) {
    hold(h, m);
    for (i, p) in points.iter().enumerate() {
        h.state().pen().push(sample(*p, true, i as u32 * 10));
        h.step();
    }
    let end = *points.last().unwrap();
    h.state()
        .pen()
        .push(sample(end, false, points.len() as u32 * 10));
    h.step();
    hold(h, Modifiers::NONE);
    settle(h);
}

/// マウスの左ボタンで `points` をなぞる（修飾 `m` を押したまま）。
fn mouse_stroke(h: &mut H, points: &[Pos2], m: Modifiers) {
    hold(h, m);
    press_with(h, points[0], m);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    release_with(h, *points.last().unwrap(), m);
    h.step();
    hold(h, Modifiers::NONE);
    settle(h);
}

/// 知らせを消す（前の準備の知らせと混ぜない）。
fn clear(h: &mut H) {
    *h.last_notice_mut() = None;
    h.message_mut().clear();
}

/// 操作の後の様子: 最後の知らせ（種類・出どころ・文）・取り消しの段の数・描いている最中か・合成の不透明度の合計。
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    notice: Option<(Kind, Source, String)>,
    strokes: usize,
    stroking: bool,
    ink: u64,
}

fn seen(h: &H) -> Seen {
    let doc = h.doc();
    let ink = doc
        .composite(doc.bounds())
        .unwrap()
        .chunks(4)
        .map(|p| p[3] as u64)
        .sum();
    Seen {
        notice: h
            .last_notice()
            .as_ref()
            .map(|n| (n.kind, n.source, n.text.clone())),
        strokes: doc.undo_count(),
        stroking: h.app().is_stroking(),
        ink,
    }
}

/// 断った: 断りの知らせ（出どころと文）、段も画素も増えず、描いていない。
#[track_caller]
fn assert_refused(s: &Seen, before: &Seen, source: Source, text: &str, what: &str) {
    assert_eq!(
        s.notice,
        Some((Kind::Refusal, source, text.to_owned())),
        "{what}"
    );
    assert_eq!(s.strokes, before.strokes, "{what}: 段は増えない");
    assert_eq!(s.ink, before.ink, "{what}: 画素は変わらない");
    assert!(!s.stroking, "{what}: 描いていない");
}

/// 何も言わずに何もしない: 知らせも段も画素も無く、描いていない。
#[track_caller]
fn assert_silent(s: &Seen, before: &Seen, what: &str) {
    assert_eq!(s.notice, None, "{what}: 知らせを出さない");
    assert_eq!(s.strokes, before.strokes, "{what}: 段は増えない");
    assert_eq!(s.ink, before.ink, "{what}: 画素は変わらない");
    assert!(!s.stroking, "{what}: 描いていない");
}

/// 描いた: 知らせ無しで 1 段増え、画素が増えた。
#[track_caller]
fn assert_painted(s: &Seen, before: &Seen, what: &str) {
    assert_eq!(s.notice, None, "{what}: 知らせを出さない");
    assert_eq!(s.strokes, before.strokes + 1, "{what}: 1 段");
    assert!(s.ink > before.ink, "{what}: 画素が増えた");
    assert!(!s.stroking, "{what}: 描き終えた");
}

/// ブラシ・消しゴムの描き始めの断りの出どころ。
fn paint_source(view: View) -> Source {
    match view {
        View::Canvas => Source::Canvas,
        View::View3d => Source::View3d,
    }
}

fn set_tool(h: &mut H, tool: Tool) {
    h.act(Action::SelectTool(tool));
    h.run();
}

fn read_only(h: &mut H, reason: &str) {
    let i = h.sets().current_index();
    h.sets_mut().get_mut(i).unwrap().read_only = Some(reason.into());
}

fn read_only_text(h: &H, reason: &str) -> String {
    yolu_app::lang::refusals::read_only_set(h.lang(), reason)
}

/// 選んでいるレイヤーの全面を、場所で色の違う不透明な模様にする（スポイトがどこから取ったかが分かる）。履歴は消す。
fn pattern(h: &mut H) {
    let layer = h.selected_layer().expect("レイヤー");
    let size = h.doc().width();
    let doc = h.doc_mut();
    for y in 0..size {
        for x in 0..size {
            doc.set_pixel(layer, x, y, Rgba8::new(x as u8, y as u8, 90, 255))
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

/// 描画色（0〜255 の RGB）。
fn main_rgb(h: &H) -> [u8; 3] {
    let c = h.color().main;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

/// 画面の点の下の文書の画素（2D は表示の逆、3D は面の UV）。
fn texel(h: &H, view: View, rect: Rect, at: Pos2) -> (u32, u32) {
    match view {
        View::View3d => texel_at(h, rect, at).expect("面の上"),
        View::Canvas => {
            let s = h.app();
            let (x, y) = s
                .view()
                .view(rect, s.doc().width(), s.doc().height())
                .to_canvas(at);
            (x as u32, y as u32)
        }
    }
}

/// 3D ビューのモデルの外（左下の隅の近く。軸のギズモからも遠い）。
fn off_model(h: &H, rect: Rect) -> Pos2 {
    let p = pos2(rect.left() + 40.0, rect.bottom() - 40.0);
    assert!(texel_at(h, rect, p).is_none(), "モデルの外");
    p
}

// ───────── D1〜D24 ─────────

/// D1 描き始めの確かめの順。
/// - 2D: 読むだけ → クイックマスク → レイヤー無し → `paint_blocker` → ステンシル → クローン
/// - 3D: モデル → テクスチャセット → 読むだけ → 面とほかのセット → クイックマスク → レイヤー無し → `paint_blocker` → 定規 → 効果・対称・ステンシル
#[test]
fn d01_the_order_of_the_checks_before_a_stroke() {
    for view in VIEWS {
        // 読むだけのセットで、レイヤーも選んでいない: どちらも読むだけを先に断る
        let (mut h, rect) = scene(view);
        read_only(&mut h, "テスト");
        h.set_selected_layer(None);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let text = read_only_text(&h, "テスト");
        assert_refused(
            &seen(&h),
            &before,
            paint_source(view),
            &text,
            &format!("{view:?}"),
        );

        // クイックマスクで、レイヤーを選んでいない: どちらもクイックマスクが先（選択範囲を塗る。描くレイヤーは要らない）
        let (mut h, rect) = scene(view);
        h.act(Action::Sel(yolu_app::selection::SelAction::Ui(
            yolu_app::selection::SelUiOp::QuickMask(Some(true)),
        )));
        h.run();
        h.set_selected_layer(None);
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let s = seen(&h);
        let notice = s.notice.clone().expect("選択範囲の知らせ");
        assert_eq!(
            (notice.0, notice.1),
            (Kind::Info, Source::Selection),
            "{view:?}: クイックマスクは断らず、選択範囲を直した知らせ: {notice:?}"
        );
        assert_eq!(s.strokes, before.strokes + 1, "{view:?}: 選択範囲の 1 段");
        assert!(
            h.doc().selection().is_some_and(|m| !m.is_empty()),
            "{view:?}: 選択範囲を塗った"
        );
    }
    // テクスチャセットがモデルに無く、読むだけでもある（試しの人形はセットとマテリアルを結ぶ。今のセットを外す）:
    // 3D はテクスチャセットを先に断り、2D は読むだけを断る
    for view in VIEWS {
        let mut h = app(1470.0, 760.0, 128);
        h.act(Action::Pose(yolu_app::view3d::pose::PoseAction::LoadFigure));
        h.run();
        let i = h.sets().current_index();
        h.sets_mut().get_mut(i).unwrap().bound = None;
        h.app_mut().sync_view3d();
        if view == View::Canvas {
            click_tab(&mut h, Tab::Canvas);
        }
        h.run();
        assert!(h.view3d().material < 0, "今のセットはモデルに付いていない");
        read_only(&mut h, "テスト");
        clear(&mut h);
        let rect = match view {
            View::View3d => h.state().view3d_rect().unwrap(),
            View::Canvas => canvas_rect(&h),
        };
        let before = seen(&h);
        let c = rect.center();
        let points: Vec<Pos2> = (0..6)
            .map(|i| c + egui::vec2(i as f32 * 6.0, 0.0))
            .collect();
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let text = match view {
            View::Canvas => read_only_text(&h, "テスト"),
            View::View3d => h.app().region_missing_reason(),
        };
        assert_refused(
            &seen(&h),
            &before,
            paint_source(view),
            &text,
            &format!("{view:?}"),
        );
    }
}

/// D2 「レイヤーが無い」の文。
/// - 2D: ブラシは `canvas-no-layer-to-paint-on`（`Source::Canvas`）
/// - 3D: ブラシは `view3d-no-layer-to-paint-on`（`Source::View3d`）。図形は 2D・3D とも `paint_gate` の `m2-no-layer-to-paint`（`Source::Ruler`）、
///   グラデーションは 2D・3D とも `gradient-no-layer-to-paint-on`、パスは 2D・3D とも `pathtool-no-layer-to-paint-on`（`Source::Path`）
///
/// キー 5 つ（canvas・view3d・pathtool・gradient・m2）がこの 1 本に全部出る。
#[test]
fn d02_the_words_for_no_layer_to_paint_on() {
    for view in VIEWS {
        for tool in [Tool::Brush, Tool::Shape, Tool::Gradient, Tool::Path] {
            let (mut h, rect) = scene(view);
            set_tool(&mut h, tool);
            h.set_selected_layer(None);
            clear(&mut h);
            let before = seen(&h);
            let points = line(&h, view, rect);
            pen_stroke(&mut h, &points, Modifiers::NONE);
            let lang = h.lang();
            let (source, key) = match (tool, view) {
                (Tool::Brush, View::Canvas) => (Source::Canvas, "canvas-no-layer-to-paint-on"),
                (Tool::Brush, View::View3d) => (Source::View3d, "view3d-no-layer-to-paint-on"),
                (Tool::Shape, _) => (Source::Ruler, "m2-no-layer-to-paint"),
                (Tool::Gradient, _) => (Source::Gradient, "gradient-no-layer-to-paint-on"),
                _ => (Source::Path, "pathtool-no-layer-to-paint-on"),
            };
            assert_refused(
                &seen(&h),
                &before,
                source,
                lang.t(key),
                &format!("{view:?} {tool:?}"),
            );
        }
    }
}

/// D3 「ほかのテクスチャセットの面」の文（3D だけの断り）。
/// - 2D: 面を押さないので、この断りは無い（どこにでも描く）
/// - 3D: 描く・定規は `view3d-surface-of-another-texture-set`（名前はそのまま、`Source::View3d`）。範囲（バケツ）は `region-this-face-belongs-to-another`
///   （`Lang::quote` つき、`Source::Fill`）
#[test]
fn d03_the_words_for_a_face_of_another_texture_set() {
    use crate::view3d_rulers::two_materials;
    use yolu_core::glam::Vec3;
    let on_b =
        |h: &H, rect: Rect| crate::view3d_brush::screen_of(h, rect, Vec3::new(1.1, 0.0, 0.0));
    for tool in [Tool::Brush, Tool::Ruler, Tool::Fill] {
        let (mut h, rect) = two_materials();
        thin_brush(&mut h);
        set_tool(&mut h, tool);
        clear(&mut h);
        let before = seen(&h);
        let at = on_b(&h, rect);
        pen_stroke(&mut h, &[at, at + egui::vec2(20.0, 4.0)], Modifiers::NONE);
        let lang = h.lang();
        let (source, text) = match tool {
            Tool::Fill => (
                Source::Fill,
                lang.f(
                    "region-this-face-belongs-to-another",
                    &[("name", &lang.quote("b"))],
                ),
            ),
            _ => (
                Source::View3d,
                lang.f("view3d-surface-of-another-texture-set", &[("name", &"b")]),
            ),
        };
        assert_refused(&seen(&h), &before, source, &text, &format!("3D {tool:?}"));
    }
    // 2D: 同じ文書のキャンバスでは、どこを押しても今のセットに描く
    let (mut h, _) = two_materials();
    thin_brush(&mut h);
    click_tab(&mut h, Tab::Canvas);
    h.run();
    let rect = canvas_rect(&h);
    clear(&mut h);
    let before = seen(&h);
    let points = line(&h, View::Canvas, rect);
    pen_stroke(&mut h, &points, Modifiers::NONE);
    assert_painted(&seen(&h), &before, "2D");
}

/// D4 Shift の直線で、手ぶれ補正と曲線を切るか。
/// - 2D: `begin_stroke(…, shift || snapped, …)` で切る（Shift で始めたストロークは、続けてなぞった所も補正しない）
/// - 3D: `open_stroke(…, constraint.is_some(), …)`。Shift だけでは切らない（続けてなぞった所に補正が効く）
///
/// 対照として、Shift 無しでは 2D・3D とも補正 0 と 40 で画素が違う（補正がこの道に効いている）ことも見る。
#[test]
fn d04_a_shift_line_turns_off_the_stabilizer_in_2d_but_not_in_3d() {
    for view in VIEWS {
        let shot = |stabilizer: f64| {
            let (mut h, rect) = scene(view);
            // 1 本目（前の終点を作る）は補正なし
            let a = spot(&h, view, rect, -0.6, -0.4);
            pen_stroke(&mut h, &[a, a + egui::vec2(2.0, 0.0)], Modifiers::NONE);
            h.m2_mut().brush.assist.stabilizer = stabilizer;
            // Shift + 押して、押した点のぶれを超えて曲がりながらなぞる
            let b = spot(&h, view, rect, 0.0, 0.0);
            let path: Vec<Pos2> = (0..14)
                .map(|i| {
                    let t = i as f32 / 13.0;
                    b + egui::vec2(t * 60.0, (t * 9.0).sin() * 14.0)
                })
                .collect();
            pen_stroke(&mut h, &path, Modifiers::SHIFT);
            assert_eq!(h.doc().undo_count(), 2, "{view:?}: {}", h.message());
            let doc = h.doc();
            doc.composite(doc.bounds()).unwrap()
        };
        let plain = shot(0.0);
        let stabilized = shot(40.0);
        // 対照: Shift 無しでは、2D も 3D も補正 0 と 40 で画素が違う（補正がこの短い道に効いていること）
        let free = |stabilizer: f64| {
            let (mut h, rect) = scene(view);
            h.m2_mut().brush.assist.stabilizer = stabilizer;
            let b = spot(&h, view, rect, 0.0, 0.0);
            let path: Vec<Pos2> = (0..14)
                .map(|i| {
                    let t = i as f32 / 13.0;
                    b + egui::vec2(t * 60.0, (t * 9.0).sin() * 14.0)
                })
                .collect();
            pen_stroke(&mut h, &path, Modifiers::NONE);
            let doc = h.doc();
            doc.composite(doc.bounds()).unwrap()
        };
        assert!(
            free(0.0) != free(40.0),
            "{view:?}: Shift 無しなら補正が効く"
        );
        match view {
            View::Canvas => assert!(plain == stabilized, "2D: Shift の直線は補正を切る"),
            View::View3d => assert!(plain != stabilized, "3D: Shift だけでは補正を切らない"),
        }
    }
}

/// D5 3D の面に当たらない押し（2D は文書の外を押した場合）。
/// - 2D: ブラシは断らない。範囲（バケツ）・スポイトは下の今の値
/// - 3D: ブラシは当たらなくても始める（面のストロークが受ける。断らない）。定規は無言で作らない。範囲は `region-no-triangle-of-this-texture`、
///   スポイトは `eyedrop-nothing-of-the-model-under`、クローンの元は `view3d-not-a-surface-of-the`
#[test]
fn d05_a_press_that_misses_the_model() {
    // 3D
    for tool in [Tool::Brush, Tool::Ruler, Tool::Fill, Tool::Eyedropper] {
        let (mut h, rect) = scene(View::View3d);
        set_tool(&mut h, tool);
        clear(&mut h);
        let before = seen(&h);
        let p = off_model(&h, rect);
        pen_stroke(&mut h, &[p, p + egui::vec2(8.0, 0.0)], Modifiers::NONE);
        let s = seen(&h);
        let lang = h.lang();
        match tool {
            Tool::Brush => {
                assert_eq!(s.notice, None, "3D ブラシ: 断らない");
                assert_eq!(s.ink, before.ink, "3D ブラシ: 画素は変わらない");
                assert!(!s.stroking);
                assert_eq!(
                    s.strokes, before.strokes,
                    "3D ブラシ: 何も描かない線は段に残さない"
                );
            }
            Tool::Ruler => {
                assert_silent(&s, &before, "3D 定規");
                assert!(h
                    .app()
                    .rulers_shown(yolu_app::rulers::Place::View3d)
                    .is_empty());
            }
            Tool::Fill => assert_refused(
                &s,
                &before,
                Source::Fill,
                lang.t("region-no-triangle-of-this-texture"),
                "3D バケツ",
            ),
            _ => assert_refused(
                &s,
                &before,
                Source::Eyedropper,
                lang.t("eyedrop-nothing-of-the-model-under"),
                "3D スポイト",
            ),
        }
    }
    // 3D: クローンの元（Alt + 左を動かさずに離す）をモデルの外に
    let (mut h, rect) = scene(View::View3d);
    h.m2_mut().brush.effect = BrushEffect::Clone {
        offset: Default::default(),
    };
    clear(&mut h);
    let before = seen(&h);
    let p = off_model(&h, rect);
    hold(&mut h, Modifiers::ALT);
    press_with(&h, p, Modifiers::ALT);
    h.step();
    release_with(&h, p, Modifiers::ALT);
    h.run();
    hold(&mut h, Modifiers::NONE);
    let lang = h.lang();
    assert_refused(
        &seen(&h),
        &before,
        Source::View3d,
        lang.t("view3d-not-a-surface-of-the"),
        "3D クローンの元",
    );
    // 2D: 文書の外（キャンバスの表示域の左下の隅）
    for tool in [Tool::Brush, Tool::Fill, Tool::Eyedropper] {
        let (mut h, rect) = scene(View::Canvas);
        set_tool(&mut h, tool);
        clear(&mut h);
        let before = seen(&h);
        let p = pos2(rect.left() + 10.0, rect.bottom() - 10.0);
        let s = h.app();
        let (x, y) = s
            .view()
            .view(rect, s.doc().width(), s.doc().height())
            .to_canvas(p);
        assert!(x < 0.0 || y > s.doc().height() as f64, "文書の外");
        pen_stroke(&mut h, &[p, p + egui::vec2(8.0, 0.0)], Modifiers::NONE);
        let s = seen(&h);
        let lang = h.lang();
        match tool {
            Tool::Brush => {
                assert_eq!(s.notice, None, "2D ブラシ: 断らない");
                assert_eq!(s.ink, before.ink, "2D ブラシ: 画素は変わらない");
                assert_eq!(
                    s.strokes, before.strokes,
                    "2D ブラシ: 何も描かない線は段に残さない"
                );
            }
            Tool::Fill => assert_refused(
                &s,
                &before,
                Source::Fill,
                lang.t("region-no-triangle-of-this-texture"),
                "2D バケツ",
            ),
            _ => assert_silent(&s, &before, "2D スポイト"),
        }
    }
}

/// D6 押しの前の確かめの置き場（モード）。
/// - 2D: 編集・ポーズのモードは呼び出し側（`handle_input`）が見て、ツールへ渡さない
/// - 3D: モードは `begin` の中（編集のモードの押しは物の選びへ）。どちらも描かず、何も言わない
#[test]
fn d06_the_edit_mode_press_with_the_mouse_paints_nothing_in_both_views() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        assert!(h.app_mut().set_mode(yolu_app::mode::EditorMode::Edit));
        h.run();
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        mouse_stroke(&mut h, &points, Modifiers::NONE);
        assert_silent(&seen(&h), &before, &format!("{view:?}"));
    }
}

/// D7 開始が core に断られたときの文。
/// - 2D: `canvas-cannot-paint` + 理由（`Source::Canvas`）
/// - 3D: `view3d-cannot-paint` + 理由（`Source::View3d`）。文は同じ「描けません」で、出どころだけが違う
#[test]
fn d07_a_start_refused_by_the_core_says_cannot_paint_from_each_view() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        let layer = h.selected_layer().unwrap();
        h.doc_mut()
            .set_layer_locks(layer, yolu_core::LayerLocks::PIXELS)
            .unwrap();
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let s = seen(&h);
        let lang = h.lang();
        let key = match view {
            View::Canvas => "canvas-cannot-paint",
            View::View3d => "view3d-cannot-paint",
        };
        let (kind, source, text) = s.notice.clone().expect("断りの知らせ");
        assert_eq!(kind, Kind::Refusal, "{view:?}");
        assert_eq!(source, paint_source(view), "{view:?}");
        let what = lang.t(key).trim_end_matches(['。', '.']);
        assert!(
            text.starts_with(what) && text.len() > what.len() + 3,
            "{view:?}: 「描けません」+ 理由: {text}"
        );
        assert_eq!(s.strokes, before.strokes);
        assert_eq!(s.ink, before.ink);
    }
}

/// D8 モデルが無いとき。
/// - 2D: ブラシ・図形は描く。範囲（バケツ）は `region-no-model` を断る（`Source::Fill`）
/// - 3D: モデルが無ければ、入力の頭（`handle`）で戻るので、ブラシ・図形・範囲・パスの押しはどれも無言で何もしない
#[test]
fn d08_no_model() {
    for view in VIEWS {
        for tool in [Tool::Brush, Tool::Shape, Tool::Fill, Tool::Path] {
            let mut h = app(1470.0, 760.0, 256);
            thin_brush(&mut h);
            if view == View::View3d {
                click_tab(&mut h, Tab::View3d);
                h.run();
            }
            assert!(h.view3d().model.is_none());
            set_tool(&mut h, tool);
            clear(&mut h);
            let rect = match view {
                View::View3d => h.state().view3d_rect().expect("3D のタブ"),
                View::Canvas => canvas_rect(&h),
            };
            let before = seen(&h);
            let points: Vec<Pos2> = match view {
                View::View3d => {
                    let c = rect.center();
                    (0..6)
                        .map(|i| c + egui::vec2(i as f32 * 8.0, 0.0))
                        .collect()
                }
                View::Canvas => line(&h, view, rect),
            };
            pen_stroke(&mut h, &points, Modifiers::NONE);
            let s = seen(&h);
            let what = format!("{view:?} {tool:?}");
            match (view, tool) {
                (View::View3d, _) => assert_silent(&s, &before, &what),
                (View::Canvas, Tool::Fill) => {
                    let lang = h.lang();
                    assert_refused(&s, &before, Source::Fill, lang.t("region-no-model"), &what)
                }
                (View::Canvas, Tool::Path) => {
                    assert_eq!(s.notice, None, "{what}: モデルが無くても断らない");
                }
                (View::Canvas, _) => assert_painted(&s, &before, &what),
            }
        }
    }
}

/// D9 クローンの元が無いとき。
/// - 2D: 断らない（ブラシの offset のまま写す）
/// - 3D: `view3d-no-clone-source` を断る
#[test]
fn d09_clone_without_a_source() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        pattern(&mut h);
        h.m2_mut().brush.effect = BrushEffect::Clone {
            offset: yolu_core::glam::DVec2::new(40.0, 0.0),
        };
        assert!(h.clone().source.is_none() && h.clone().canvas_source_for(h.doc().id()).is_none());
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let s = seen(&h);
        match view {
            View::Canvas => {
                assert_eq!(s.notice, None, "2D: 断らない");
                assert_eq!(
                    s.strokes,
                    before.strokes + 1,
                    "2D: offset のまま写した 1 段"
                );
            }
            View::View3d => {
                let lang = h.lang();
                assert_refused(
                    &s,
                    &before,
                    Source::View3d,
                    lang.t("view3d-no-clone-source"),
                    "3D",
                );
            }
        }
    }
}

/// D10 対称とぼかし・指先・クローン（同じ 2D の縦の対称定規で）。
/// - 2D: 開始が core に断られ、`canvas-cannot-paint` + 理由
/// - 3D: `open_stroke` が先に `view3d-smudge-and-clone-do-not` を断る
#[test]
fn d10_smudge_with_symmetry() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        h.m2_mut().brush.effect = BrushEffect::Smudge { strength: 1.0 };
        common::rulers::vertical(h.app_mut(), 128.0);
        h.run();
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let s = seen(&h);
        let lang = h.lang();
        match view {
            View::Canvas => {
                let (_, source, text) = s.notice.clone().expect("2D: 断りの知らせ");
                assert_eq!(source, Source::Canvas);
                let what = lang.t("canvas-cannot-paint").trim_end_matches(['。', '.']);
                assert!(text.starts_with(what), "2D: {text}");
                assert_eq!(s.strokes, before.strokes);
                assert_eq!(s.ink, before.ink);
            }
            View::View3d => assert_refused(
                &s,
                &before,
                Source::View3d,
                lang.t("view3d-smudge-and-clone-do-not"),
                "3D",
            ),
        }
    }
}

/// D11 2D だけのツール（移動・変形・テキスト）。
/// - 2D: キャンバスで使える（断らない）
/// - 3D: `Surface::Unsupported` で `view3d-this-tool-works-on-the` を断る
#[test]
fn d11_the_2d_only_tools() {
    for view in VIEWS {
        for tool in [Tool::Move, Tool::Text] {
            let (mut h, rect) = scene(view);
            if tool == Tool::Move {
                pattern(&mut h);
            }
            set_tool(&mut h, tool);
            clear(&mut h);
            let before = seen(&h);
            let points = line(&h, view, rect);
            pen_stroke(&mut h, &points, Modifiers::NONE);
            let s = seen(&h);
            let what = format!("{view:?} {tool:?}");
            match view {
                View::Canvas => {
                    assert!(
                        s.notice.as_ref().is_none_or(|n| n.0 != Kind::Refusal),
                        "{what}: 断らない: {:?}",
                        s.notice
                    );
                    if tool == Tool::Text {
                        assert!(h.text().editing.is_some(), "{what}: 文字を打ち始めた");
                    }
                }
                View::View3d => {
                    let lang = h.lang();
                    assert_refused(
                        &s,
                        &before,
                        Source::View3d,
                        lang.t("view3d-this-tool-works-on-the"),
                        &what,
                    );
                }
            }
        }
    }
}

/// D12 ストロークの終わりの時機。
/// - 2D: 離したフレームで確定する
/// - 3D: 離すと `released`。毎フレーム持ち越しを塗り、塗り終えたら確定する（その間は描いている最中）
#[test]
fn d12_the_stroke_ends_on_the_lift_in_2d_and_after_the_queue_in_3d() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        h.view3d_mut().input.paint_budget = Some(std::time::Duration::ZERO);
        let a = spot(&h, view, rect, -0.8, 0.0);
        let b = spot(&h, view, rect, 0.8, 0.0);
        let points: Vec<Pos2> = (0..4).map(|i| a + (b - a) * (i as f32 / 3.0)).collect();
        for (i, p) in points.iter().enumerate() {
            h.state().pen().push(sample(*p, true, i as u32 * 10));
        }
        h.state().pen().push(sample(b, false, 100));
        h.step();
        match view {
            View::Canvas => {
                assert!(!h.app().is_stroking(), "2D: 離したフレームで確定");
                assert_eq!(h.doc().undo_count(), 1);
            }
            View::View3d => {
                assert!(h.app().is_stroking(), "3D: 離した後も持ち越しを塗っている");
                assert!(h.view3d().input.released);
                assert_eq!(h.doc().undo_count(), 0, "3D: まだ確定していない");
                settle(&mut h);
                assert_eq!(h.doc().undo_count(), 1, "3D: 塗り終えて確定");
            }
        }
    }
}

/// D13 マウスのストロークが残っている間に、ペンが触れた。
/// - 2D: ペンの `press_kind` は描いている間も `Tool` を通し、`begin_any` の前に「始めたストロークが残っていないか」を見ない（下の今の値）
/// - 3D: `press_kind` が `stroke.is_some()` で `Ignored`（ペンは何もしない）
#[test]
fn d13_a_pen_touch_while_a_mouse_stroke_is_still_down() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        clear(&mut h);
        let a = spot(&h, view, rect, -0.6, -0.3);
        press_with(&h, a, Modifiers::NONE);
        h.step();
        move_to(&h, a + egui::vec2(10.0, 0.0));
        h.step();
        assert!(h.app().is_stroking(), "{view:?}: マウスで描いている");
        // ペンが別の所に触れて動いて離れる（マウスは押したまま）
        let b = spot(&h, view, rect, 0.2, 0.4);
        for (i, p) in [b, b + egui::vec2(10.0, 0.0), b + egui::vec2(20.0, 0.0)]
            .into_iter()
            .enumerate()
        {
            h.state().pen().push(sample(p, true, i as u32 * 10));
            h.step();
        }
        h.state()
            .pen()
            .push(sample(b + egui::vec2(20.0, 0.0), false, 40));
        h.step();
        let during = seen(&h);
        release_with(&h, a + egui::vec2(10.0, 0.0), Modifiers::NONE);
        h.step();
        settle(&mut h);
        let s = seen(&h);
        match view {
            View::Canvas => {
                // ペンの押しは `begin_any` へ進み、core が「描いている最中」で断る（今の値）
                let lang = h.lang();
                let what = lang.t("canvas-cannot-paint").trim_end_matches(['。', '.']);
                let n = during.notice.clone().expect("2D: ペンの押しの知らせ");
                assert_eq!((n.0, n.1), (Kind::Refusal, Source::Canvas), "2D: {n:?}");
                assert!(n.2.starts_with(what), "2D: {}", n.2);
                assert_eq!(s.strokes, 1, "2D: マウスの 1 本だけ");
            }
            View::View3d => {
                assert_eq!(during.notice, None, "3D: ペンの押しは何もしない");
                assert_eq!(s.strokes, 1, "3D: マウスの 1 本だけ");
                assert!(!ink_near_texel(&h, view, rect, b), "3D: ペンの所は塗らない");
            }
        }
    }
}

/// 画面の点の下の画素のまわり（半径 2）に、何か塗られているか。
fn ink_near_texel(h: &H, view: View, rect: Rect, at: Pos2) -> bool {
    let (x, y) = texel(h, view, rect, at);
    let doc = h.doc();
    let size = doc.width();
    (y.saturating_sub(2)..=(y + 2).min(size - 1)).any(|yy| {
        (x.saturating_sub(2)..=(x + 2).min(size - 1))
            .any(|xx| yolu_app::engine::composite_pixel(doc, xx, yy)[3] > 0)
    })
}

/// 右ボタンを押して動かして離す（`moved` なら 2 点目で離す）。
fn right_drag(h: &mut H, from: Pos2, to: Option<Pos2>) {
    press(h, from, PointerButton::Secondary);
    h.step();
    let end = match to {
        Some(to) => {
            for i in 1..=6 {
                move_to(h, from + (to - from) * (i as f32 / 6.0));
                h.step();
            }
            to
        }
        None => from,
    };
    release(h, end, PointerButton::Secondary);
    h.run();
}

/// D14 右ボタン（動かさずに離す・動かして離す）。
/// - 2D: 右ボタンはスポイトのドラッグ（`Start::Pick`）。動かして離せば、離した所の色を取る
/// - 3D: 右ボタンは回す。動かさずに離したときだけスポイト（`click_of` の `Pick`）。動かしたら色を取らない
#[test]
fn d14_the_right_button_picks_on_a_click_in_both_and_on_a_drag_only_in_2d() {
    for view in VIEWS {
        // 動かさずに離す: どちらも押した所の色
        let (mut h, rect) = scene(view);
        pattern(&mut h);
        h.color_mut().set_main([0.0, 0.0, 0.0, 1.0]);
        let a = spot(&h, view, rect, -0.5, -0.5);
        right_drag(&mut h, a, None);
        let (x, y) = texel(&h, view, rect, a);
        assert_eq!(
            main_rgb(&h),
            [x as u8, y as u8, 90],
            "{view:?}: クリックで取る"
        );
        // 動かして離す
        let (mut h, rect) = scene(view);
        pattern(&mut h);
        h.color_mut().set_main([0.0, 0.0, 0.0, 1.0]);
        let yaw = h.view3d().camera.yaw;
        let (a, b) = (
            spot(&h, view, rect, -0.5, -0.5),
            spot(&h, view, rect, 0.5, 0.3),
        );
        let (bx, by) = texel(&h, view, rect, b);
        right_drag(&mut h, a, Some(b));
        match view {
            View::Canvas => {
                assert_eq!(main_rgb(&h), [bx as u8, by as u8, 90], "2D: 離した所の色");
            }
            View::View3d => {
                assert_eq!(main_rgb(&h), [0, 0, 0], "3D: 動かしたら取らない");
                assert_ne!(h.view3d().camera.yaw, yaw, "3D: 回した");
            }
        }
        assert_eq!(h.doc().undo_count(), 0, "{view:?}: 描かない");
    }
}

/// ペンのサイドボタンで、`points` を触れてなぞって離す。
fn barrel_stroke(h: &mut H, points: &[Pos2]) {
    let barrel = |at: Pos2, contact: bool, t: u32| yolu_app::pen::PenSample {
        barrel: true,
        ..sample(at, contact, t)
    };
    for (i, p) in points.iter().enumerate() {
        h.state().pen().push(barrel(*p, true, i as u32 * 10));
        h.step();
    }
    let end = *points.last().unwrap();
    h.state().pen().push(barrel(end, false, 1000));
    h.step();
    h.run();
}

/// D15 ペンのサイドボタン。
/// - 2D: `PressKind::Eyedrop`（スポイトのドラッグ。動かしても、離した所の色を取る）
/// - 3D: 右ボタンと同じ `View`（回す。動かさずに離したときだけスポイト）
#[test]
fn d15_the_pen_side_button_picks_on_a_tap_in_both_and_on_a_drag_only_in_2d() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        pattern(&mut h);
        h.color_mut().set_main([0.0, 0.0, 0.0, 1.0]);
        let a = spot(&h, view, rect, -0.5, -0.5);
        barrel_stroke(&mut h, &[a]);
        let (x, y) = texel(&h, view, rect, a);
        assert_eq!(
            main_rgb(&h),
            [x as u8, y as u8, 90],
            "{view:?}: 触れて離せば取る"
        );

        let (mut h, rect) = scene(view);
        pattern(&mut h);
        h.color_mut().set_main([0.0, 0.0, 0.0, 1.0]);
        let yaw = h.view3d().camera.yaw;
        let (a, b) = (
            spot(&h, view, rect, -0.5, -0.5),
            spot(&h, view, rect, 0.5, 0.3),
        );
        let (bx, by) = texel(&h, view, rect, b);
        let path: Vec<Pos2> = (0..=6).map(|i| a + (b - a) * (i as f32 / 6.0)).collect();
        barrel_stroke(&mut h, &path);
        match view {
            View::Canvas => assert_eq!(main_rgb(&h), [bx as u8, by as u8, 90], "2D: 離した所の色"),
            View::View3d => {
                assert_eq!(main_rgb(&h), [0, 0, 0], "3D: なぞったら取らない");
                assert_ne!(h.view3d().camera.yaw, yaw, "3D: 回した");
            }
        }
        assert_eq!(h.doc().undo_count(), 0, "{view:?}: 描かない");
    }
}

/// D16 `press_kind` の判定の順（ビューを動かしている最中のペン先）。
/// - 2D: ① 押せない・一番上でない・ステンシル ② 描いている ③ ナビ・離しの操作 ④ ツール。マウスの中ボタンのパンの途中でも、ペン先は描く
/// - 3D: ① 押せない・一番上でない・ビューを動かしている・描いている・図形・選択・ステンシル で `Ignored`。パンの途中のペン先は何もしない
#[test]
fn d16_a_pen_tip_while_the_mouse_middle_button_pans() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        clear(&mut h);
        let m = spot(&h, view, rect, 0.8, 0.8);
        press(&h, m, PointerButton::Middle);
        h.step();
        match view {
            View::Canvas => assert!(h.canvas().panning, "2D: 中ボタンでパンが始まっている"),
            View::View3d => assert!(
                h.view3d().input.nav.is_some(),
                "3D: 中ボタンでビューを動かしている"
            ),
        }
        let before = seen(&h);
        let b = spot(&h, view, rect, -0.4, 0.0);
        for (i, p) in [b, b + egui::vec2(10.0, 0.0), b + egui::vec2(20.0, 0.0)]
            .into_iter()
            .enumerate()
        {
            h.state().pen().push(sample(p, true, i as u32 * 10));
            h.step();
        }
        h.state()
            .pen()
            .push(sample(b + egui::vec2(20.0, 0.0), false, 40));
        h.step();
        settle(&mut h);
        let s = seen(&h);
        release(&h, m, PointerButton::Middle);
        h.run();
        match view {
            View::Canvas => assert_painted(&s, &before, "2D: パンの途中でもペン先は描く"),
            View::View3d => assert_silent(&s, &before, "3D: パンの途中のペン先は何もしない"),
        }
    }
}

/// D17 ツールの押しのボタン（選択の「追加」の行を、左以外のボタン（Extra1 + Shift）にした）。
/// - 2D: `app.canvas.tool_button` で、左以外のボタンでも選択を始められる
/// - 3D: ツールは左ボタンだけ（左以外のボタンの押しは何もしない）
#[test]
fn d17_a_selection_row_moved_to_another_button() {
    use yolu_app::keyconfig::Combo;
    use yolu_app::keymap::{Operation, GESTURES};
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        let row = GESTURES
            .iter()
            .find(|g| g.operation == Operation::SelectionAdd)
            .unwrap();
        let combo = Combo {
            button: yolu_app::state::key::PointerButton::Extra1,
            alt: false,
            shift: true,
            ctrl: false,
        };
        h.keys_mut().set_combo(row.index, Some(combo));
        assert!(h.keys_mut().combo_conflict(row.index).is_none());
        set_tool(&mut h, Tool::SelectRect);
        clear(&mut h);
        let (a, b) = (
            spot(&h, view, rect, -0.5, -0.5),
            spot(&h, view, rect, 0.5, 0.5),
        );
        hold(&mut h, Modifiers::SHIFT);
        h.event(Event::PointerMoved(a));
        h.event(Event::PointerButton {
            pos: a,
            button: PointerButton::Extra1,
            pressed: true,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
        for i in 1..=4 {
            move_to(&h, a + (b - a) * (i as f32 / 4.0));
            h.step();
        }
        h.event(Event::PointerButton {
            pos: b,
            button: PointerButton::Extra1,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        });
        h.step();
        hold(&mut h, Modifiers::NONE);
        h.run();
        let selected = h.doc().selection().is_some_and(|m| !m.is_empty());
        match view {
            View::Canvas => assert!(selected, "2D: Extra1 + Shift で選択範囲を足した"),
            View::View3d => {
                assert!(!selected, "3D: 左以外のボタンはツールにならない");
                assert_eq!(h.doc().undo_count(), 0);
            }
        }
    }
}

/// D18 3D ビューの設定のパネルを開いている間の押し。
/// - 2D: 見ない（描く）
/// - 3D: `settings_open` で押しを捨てる（無言）
#[test]
fn d18_a_press_while_the_3d_settings_panel_is_open() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        h.view3d_mut().display.settings_open = true;
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        match view {
            View::Canvas => assert_painted(&seen(&h), &before, "2D"),
            View::View3d => assert_silent(&seen(&h), &before, "3D"),
        }
    }
}

/// 点のグラデーションのレイヤーを作り、その点を編集している状態にする（点の空間はモデル）。
fn points_layer(h: &mut H) -> yolu_app::engine::LayerId {
    h.act(Action::M2(yolu_app::m2::Edit::NewFill));
    let layer = h.selected_layer().unwrap();
    h.act(Action::Fill(yolu_app::fillfx::FillOp::AddPoints {
        layer,
        channel: yolu_app::engine::Channel::Color,
    }));
    h.run();
    assert_eq!(
        h.fillfx().edit_points,
        Some((layer, yolu_app::engine::Channel::Color))
    );
    layer
}

fn point_count(h: &H, layer: yolu_app::engine::LayerId) -> usize {
    h.doc()
        .layer(layer)
        .unwrap()
        .fill_points(yolu_app::engine::Channel::Color)
        .map_or(0, |g| g.points.len())
}

/// D19 離しを取りこぼしたとき（ボタンは押していないのに、ドラッグの途中の印だけが残った）の、点のグラデーションの点のドラッグ。
/// - 2D: 取りこぼしは、全ツールが同じ道で終える（点のドラッグは `finish_stroke` から `fillfx::points::release`）
/// - 3D: 取りこぼしは 7 つの個別の道で、点のドラッグ（`fillfx.point_drag`）の分は無い。マウスの点のドラッグの印が残り続ける
#[test]
fn d19_a_missed_release_of_a_point_drag() {
    use yolu_app::fillfx::points::PointDrag;
    for view in VIEWS {
        let (mut h, _) = scene(view);
        let layer = points_layer(&mut h);
        let in_3d = view == View::View3d;
        let id = h.doc().id();
        h.fillfx_mut().point_drag = Some(PointDrag {
            layer,
            channel: yolu_app::engine::Channel::Color,
            index: 0,
            source: yolu_app::fillfx::gizmo::Source::Mouse,
            doc_id: id,
            in_3d,
            added: false,
        });
        if !in_3d {
            // 2D の点のドラッグは、押したときにストロークの札も持つ（`begin_any`）
            h.canvas_mut().stroke = Some(yolu_app::state::StrokeSource::Mouse);
        }
        for _ in 0..4 {
            h.step();
        }
        let left = h.fillfx().point_drag.is_some();
        match view {
            View::Canvas => assert!(!left, "2D: 取りこぼしたら終える"),
            View::View3d => {
                assert!(left, "3D: 点のドラッグの印が残る（取りこぼしの道が無い）");
                assert!(
                    !h.app().is_stroking(),
                    "3D: 残った印は「描いている最中」には数えない"
                );
            }
        }
    }
}

/// D20 終わりの点の覚え方（Shift の直線の始め）。
/// - 2D: 前の終点は文書の座標（`canvas.previous_end`）
/// - 3D: 前の終点は面の点（`view3d.input.previous_end`）。2D と 3D で別々に覚え、片方の終点はもう片方の Shift の直線に使わない
#[test]
fn d20_each_view_keeps_its_own_previous_end() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        assert_eq!(h.doc().undo_count(), 1);
        assert_eq!(
            h.canvas().previous_end.is_some(),
            view == View::Canvas,
            "{view:?}: 2D の終点"
        );
        assert_eq!(
            h.view3d().input.previous_end.is_some(),
            view == View::View3d,
            "{view:?}: 3D の終点"
        );
        // もう片方のビューで Shift + 押し: 前の終点が無いので、押した点だけ（線を引かない）
        let other = match view {
            View::Canvas => View::View3d,
            View::View3d => View::Canvas,
        };
        click_tab(
            &mut h,
            match other {
                View::Canvas => Tab::Canvas,
                View::View3d => Tab::View3d,
            },
        );
        h.run();
        let rect = match other {
            View::View3d => h.state().view3d_rect().unwrap(),
            View::Canvas => canvas_rect(&h),
        };
        let at = spot(&h, other, rect, 0.6, 0.6);
        let ink = seen(&h).ink;
        pen_stroke(&mut h, &[at], Modifiers::SHIFT);
        assert_eq!(h.doc().undo_count(), 2, "{other:?}");
        let added = seen(&h).ink - ink;
        // 押した点だけのダブ（半径 2 の円。線を引いたら何十倍にもなる）
        assert!(
            added > 0 && added < 255 * 40,
            "{other:?}: 押した点だけ: {added}"
        );
    }
}

/// D21 編集・ポーズのモードのペン。
/// - 2D: 編集・ポーズのモードでペンは `Ignored`（描かない）
/// - 3D: ポーズのモードはペンの点を捨てる（`pen = &[]`）。編集のモードのペンは物の選び（描かない）
#[test]
fn d21_the_pen_in_the_edit_and_pose_modes() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        assert!(h.app_mut().set_mode(yolu_app::mode::EditorMode::Edit));
        h.run();
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        assert_silent(&seen(&h), &before, &format!("{view:?} 編集"));
    }
    // ポーズのモード（試しの人形を読む）
    for view in VIEWS {
        let mut h = app(1470.0, 760.0, 128);
        h.act(Action::Pose(yolu_app::view3d::pose::PoseAction::LoadFigure));
        h.run();
        thin_brush(&mut h);
        assert!(
            h.app_mut().set_mode(yolu_app::mode::EditorMode::Pose),
            "{}",
            h.message()
        );
        if view == View::Canvas {
            click_tab(&mut h, Tab::Canvas);
        }
        h.run();
        let rect = match view {
            View::View3d => h.state().view3d_rect().unwrap(),
            View::Canvas => canvas_rect(&h),
        };
        clear(&mut h);
        let before = seen(&h);
        let c = rect.center();
        let points: Vec<Pos2> = (0..6)
            .map(|i| c + egui::vec2(i as f32 * 6.0, 0.0))
            .collect();
        pen_stroke(&mut h, &points, Modifiers::NONE);
        let s = seen(&h);
        assert_eq!(s.strokes, before.strokes, "{view:?} ポーズ: 描かない");
        assert_eq!(s.ink, before.ink, "{view:?} ポーズ");
        if view == View::View3d {
            assert!(
                h.view3d().input.pen_press.is_none(),
                "3D ポーズ: ペンの押しを持たない"
            );
        }
    }
}

/// D22 押した瞬間の `rulers.last_drew` の記録（「スナップする特殊定規の切り替え」が、ポインタがどちらにも乗っていないときに使う）。
/// - 2D: ツールの押し（ペン・マウス）で、描かないナビゲーションを除いて、`begin_any` の前に記録する
/// - 3D: `begin` の中（モード・ベイクの確かめの後）で記録する。`begin` に届かない押し（点のグラデーションの点）は記録しない
#[test]
fn d22_which_presses_remember_the_view_last_drawn_in() {
    use yolu_app::rulers::Place;
    let place = |view: View| match view {
        View::Canvas => Place::Canvas,
        View::View3d => Place::View3d,
    };
    for view in VIEWS {
        // ブラシの押し: どちらも記録する
        let (mut h, rect) = scene(view);
        h.rulers_mut().last_drew = None;
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        assert_eq!(h.rulers().last_drew, Some(place(view)), "{view:?}: ブラシ");
        // 編集のモードの押し: どちらも記録しない
        let (mut h, rect) = scene(view);
        assert!(h.app_mut().set_mode(yolu_app::mode::EditorMode::Edit));
        h.run();
        h.rulers_mut().last_drew = None;
        let points = line(&h, view, rect);
        pen_stroke(&mut h, &points, Modifiers::NONE);
        assert_eq!(h.rulers().last_drew, None, "{view:?}: 編集のモード");
        // 点のグラデーション（モデルの空間の点）を編集している間の押し
        let (mut h, rect) = scene(view);
        let layer = points_layer(&mut h);
        let points_before = point_count(&h, layer);
        h.rulers_mut().last_drew = None;
        let at = spot(&h, view, rect, 0.5, 0.5);
        pen_stroke(&mut h, &[at], Modifiers::NONE);
        match view {
            View::Canvas => {
                // 2D: モデルの空間の点はキャンバスでは足さない。押しはツールの押しとして記録する
                assert_eq!(point_count(&h, layer), points_before, "2D: 点は足さない");
                assert_eq!(h.rulers().last_drew, Some(Place::Canvas), "2D: 記録する");
            }
            View::View3d => {
                // 3D: 面に点を足し（`begin` の前の `fillfx::points::press`）、記録しない
                assert_eq!(point_count(&h, layer), points_before + 1, "3D: 点を足した");
                assert_eq!(h.rulers().last_drew, None, "3D: 点の押しは記録しない");
            }
        }
    }
}

/// D23 グラデーションの離しの後（押した後に、セットが読むだけになった）。
/// - 2D: 離しで `Action::Gradient(Apply)` を当て、`apply` の読むだけの確かめが `Source::Edit` で断る
/// - 3D: 離しで `draft::gradient` が直に確かめ、同じ文を `Source::Gradient` で断る
#[test]
fn d23_a_gradient_released_after_the_set_became_read_only() {
    for view in VIEWS {
        let (mut h, rect) = scene(view);
        set_tool(&mut h, Tool::Gradient);
        clear(&mut h);
        let before = seen(&h);
        let points = line(&h, view, rect);
        for (i, p) in points.iter().enumerate() {
            h.state().pen().push(sample(*p, true, i as u32 * 10));
            h.step();
        }
        read_only(&mut h, "テスト");
        h.state()
            .pen()
            .push(sample(*points.last().unwrap(), false, 100));
        h.step();
        settle(&mut h);
        let source = match view {
            View::Canvas => Source::Edit,
            View::View3d => Source::Gradient,
        };
        let text = read_only_text(&h, "テスト");
        assert_refused(&seen(&h), &before, source, &text, &format!("{view:?}"));
    }
}

/// D24 選んでいるレイヤーが文書に無いとき（古い選び）。
/// - 2D: `begin_stroke` は `selected_layer` が `Some` かだけを見て、`paint_blocker` も素通しする。core が `LayerNotFound` で断り、`canvas-cannot-paint` + 理由
/// - 3D: ブラシは同じ式で `view3d-cannot-paint` + 理由。図形は 2D・3D とも `paint_gate` が `m2-no-layer-to-paint` で断る
#[test]
fn d24_a_selected_layer_that_is_no_longer_in_the_document() {
    for view in VIEWS {
        for tool in [Tool::Brush, Tool::Shape] {
            let (mut h, rect) = scene(view);
            set_tool(&mut h, tool);
            let gone = h.doc_mut().add_layer("消すレイヤー").unwrap();
            h.doc_mut().remove_layer(gone).unwrap();
            h.doc_mut().clear_history().unwrap();
            h.set_selected_layer(Some(gone));
            clear(&mut h);
            let before = seen(&h);
            let points = line(&h, view, rect);
            pen_stroke(&mut h, &points, Modifiers::NONE);
            let s = seen(&h);
            let what = format!("{view:?} {tool:?}");
            let lang = h.lang();
            match tool {
                Tool::Brush => {
                    let key = match view {
                        View::Canvas => "canvas-cannot-paint",
                        View::View3d => "view3d-cannot-paint",
                    };
                    let (kind, source, text) = s.notice.clone().expect("知らせ");
                    assert_eq!(kind, Kind::Error, "{what}: core の LayerNotFound は失敗");
                    assert_eq!(source, paint_source(view), "{what}");
                    let head = lang.t(key).trim_end_matches(['。', '.']);
                    assert!(text.starts_with(head), "{what}: {text}");
                    assert_eq!(s.strokes, before.strokes, "{what}");
                    assert_eq!(s.ink, before.ink, "{what}");
                }
                _ => assert_refused(
                    &s,
                    &before,
                    Source::Ruler,
                    lang.t("m2-no-layer-to-paint"),
                    &what,
                ),
            }
        }
    }
}
