//! ブラシの詳細の行: 目のボタン（ツールプロパティに出す行をブラシごとに覚える）・影響元のボタンとウィンドウ（ツールプロパティの行からも同じ）・消すモード（コマンドと、
//! 実際の入力で描く）・画面上のサイズで指定（表示倍率を変えて描く）。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{rows, BrushAction, BrushKey, Category, Row};
use yolu_app::engine::{Channel, TipColorChange};
use yolu_app::state::{Action, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn open_detail(h: &mut H, category: Category) {
    let ui = &mut h.brushes_mut().ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn detail_rect(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

/// 詳細の欄の部品（左の区分と同じ名前のものと区別して、ウィンドウの右側から探す）。
fn in_pane(h: &H, label: &str) -> Rect {
    let window = detail_rect(h);
    rect_of(h, label, |r| {
        r.left() > window.left() + 168.0 && window.contains(r.center())
    })
}

/// 左のパネル（ツールプロパティ）にその名前の行があるか。
fn tool_has(h: &H, label: &str) -> bool {
    h.query_all_by_label(label)
        .any(|n| in_panel(n.rect()) && n.rect().top() > 200.0)
}

fn in_panel(r: Rect) -> bool {
    r.left() < 390.0 && r.top() > 80.0 && r.top() < 700.0
}

/// ツールプロパティの行の上からの位置（行の名前の部品の上端）。
fn tool_top(h: &H, label: &str) -> f32 {
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| in_panel(*r) && r.top() > 200.0)
        .unwrap_or_else(|| panic!("ツールプロパティに {label} が無い"))
        .top()
}

/// 詳細の欄の行の左端の目のボタン（行と同じ高さにある「ツールプロパティに表示」）。
fn eye_of(h: &H, row_label: &str) -> Rect {
    let window = detail_rect(h);
    let row = in_pane(h, row_label);
    h.query_all_by_label("ツールプロパティに表示")
        .map(|n| n.rect())
        .find(|r| window.contains(r.center()) && (r.center().y - row.center().y).abs() < 8.0)
        .unwrap_or_else(|| panic!("{row_label} の目のボタン"))
}

/// 影響元のボタンの「押し込んだ」印（どれかの入力を使っている）。
fn lit(node: &egui_kittest::Node<'_>) -> bool {
    let n = node.accesskit_node();
    n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn the_eye_button_puts_a_row_in_the_tool_properties_and_each_brush_remembers_it() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Size);
    // 前のツールプロパティが出していた行に目が入っている。それ以外は出ない
    assert!(tool_has(&h, "直径") && tool_has(&h, "手ぶれ補正"));
    assert!(!tool_has(&h, "最低 1 ピクセル"));
    for row in Row::ALL {
        assert_eq!(
            rows::is_shown(&h.m2().brush, row),
            row.default_shown(),
            "{row:?}"
        );
    }
    assert!(!h.app().brush_is_modified(b("standard")));
    // 最低 1 ピクセルの目を入れると、ツールプロパティにも出る。区分の順（直径の下・不透明度の上）に並ぶ
    let eye = eye_of(&h, "最低 1 ピクセル");
    click(&mut h, eye.center());
    assert!(rows::is_shown(&h.m2().brush, Row::MinOnePixel));
    assert!(tool_has(&h, "最低 1 ピクセル"));
    assert!(tool_top(&h, "直径") < tool_top(&h, "最低 1 ピクセル"));
    assert!(tool_top(&h, "最低 1 ピクセル") < tool_top(&h, "不透明度"));
    // 目の印はブラシの設定（変更ありの印になる）
    assert!(h.app().brush_is_modified(b("standard")));
    // ツールプロパティの行の値も、詳細の行と同じ設定を動かす
    let at = rect_of(&h, "最低 1 ピクセル", |r| {
        in_panel(r) && r.top() > 200.0
    })
    .center();
    click(&mut h, at);
    assert!(h.m2().brush.controls.min_one_pixel);
    // 直径の目を切ると、ツールプロパティから直径が消える（詳細の行は残る）
    let eye = eye_of(&h, "直径");
    click(&mut h, eye.center());
    assert!(!rows::is_shown(&h.m2().brush, Row::Size));
    assert!(!tool_has(&h, "直径"));
    in_pane(&h, "直径");
    // 区分の「既定に戻す」は値だけを戻し、目の印は変えない
    h.get_by_label("ブラシサイズを既定に戻す").click();
    h.run();
    assert!(!h.m2().brush.controls.min_one_pixel);
    assert!(
        rows::is_shown(&h.m2().brush, Row::MinOnePixel)
            && !rows::is_shown(&h.m2().brush, Row::Size)
    );
    // 別のブラシは、そのブラシの目の印（前の形なので既定の行）。戻ると、変えた目の印も残っている
    h.act(Action::Brush(BrushAction::Select(b("pencil"))));
    h.run();
    assert!(tool_has(&h, "直径") && !tool_has(&h, "最低 1 ピクセル"));
    h.act(Action::Brush(BrushAction::Select(b("standard"))));
    h.run();
    assert!(!tool_has(&h, "直径") && tool_has(&h, "最低 1 ピクセル"));
    // 元に戻す: 初めの形（印を持たない）へ
    h.act(Action::Brush(BrushAction::Revert(b("standard"))));
    h.run();
    assert_eq!(h.m2().brush.shown_rows, None);
    assert!(tool_has(&h, "直径") && !tool_has(&h, "最低 1 ピクセル"));
    assert!(!h.app().brush_is_modified(b("standard")));
    // どの操作も Undo の段を作らない
    assert!(!h.doc().can_undo());
}

#[test]
fn a_toggle_row_with_an_eye_is_shown_in_the_tool_properties_and_works_there() {
    let mut h = app(1600.0, 1000.0, 128);
    // 消去の行（消すモード）の目を入れる
    open_detail(&mut h, Category::Erase);
    let eye = eye_of(&h, "消去");
    click(&mut h, eye.center());
    assert!(tool_has(&h, "消去"));
    let at = rect_of(&h, "消去", |r| in_panel(r) && r.top() > 200.0).center();
    click(&mut h, at);
    assert!(h.m2().erase_mode, "ツールプロパティの行からも切り替わる");
    // 詳細の同じ行からも切り替わる
    let at = in_pane(&h, "消去").center();
    click(&mut h, at);
    assert!(!h.m2().erase_mode);
}

/// 色の変化の色相の影響元: ツールプロパティの行のボタンから同じウィンドウが開き、ランダムを入れると両方のボタンが押し込んだ印になる。
#[test]
fn the_hue_dynamics_window_opens_from_the_tool_row_and_lights_both_buttons() {
    let mut h = app(1600.0, 1000.0, 128);
    h.m2_mut().brush.color.tip = Some(TipColorChange {
        hue: 90.0,
        ..TipColorChange::default()
    });
    rows::set_shown(&mut h.m2_mut().brush, Row::ColorHue, true);
    open_detail(&mut h, Category::ColorChange);
    // ツールプロパティの影響元のボタンは、上から 直径・不透明度・流量・色相・硬さ の 5 つ
    let buttons = |h: &H| -> Vec<(Rect, bool)> {
        let mut v: Vec<(Rect, bool)> = h
            .query_all_by_label("影響元")
            .filter(|n| in_panel(n.rect()))
            .map(|n| (n.rect(), lit(&n)))
            .collect();
        v.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        v
    };
    let tool = buttons(&h);
    assert_eq!(tool.len(), 5, "{tool:?}");
    assert!(!tool[3].1, "色相は何も使っていない");
    click(&mut h, tool[3].0.center());
    assert_eq!(h.brushes().ui.detail.dynamics, Some(Row::ColorHue));
    // ウィンドウ（見出しは「色相影響元設定」）と、入力の行
    let window =
        yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_rows::dynamics_id())
            .expect("影響元設定のウィンドウ");
    let random = h
        .query_all_by_label("ランダム")
        .map(|n| n.rect())
        .find(|r| window.contains(r.center()))
        .expect("ランダムの欄");
    click(&mut h, random.center());
    let hue = &h.m2().brush.influences.color_hue;
    assert!(hue.random.on, "ランダムが入る");
    assert_eq!(hue.random.response.min(), -1.0, "最小値 −100%（±値）");
    assert!(h.app().brush_is_modified(b("standard")));
    // ツールプロパティの行のボタンも、詳細の行のボタンも、押し込んだ印になる
    assert!(buttons(&h)[3].1, "ツールプロパティの色相のボタン");
    let detail_buttons: Vec<bool> = h
        .query_all_by_label("影響元")
        .filter(|n| detail_rect(&h).contains(n.rect().center()))
        .map(|n| lit(&n))
        .collect();
    assert!(
        detail_buttons.iter().filter(|on| **on).count() >= 1,
        "{detail_buttons:?}"
    );
}

/// 色相のランダムを入れた線は色相がばらつき、入れない線は一定（同じ種なら同じ線）。実際の入力の道（キャンバスのドラッグ）で描く。
#[test]
fn a_hue_dynamics_random_makes_the_hue_of_a_line_vary() {
    let draw = |random: bool| -> std::collections::BTreeSet<[u8; 3]> {
        let mut h = app(1600.0, 900.0, 128);
        h.m2_mut().brush.color.tip = Some(TipColorChange {
            hue: 180.0,
            ..TipColorChange::default()
        });
        h.m2_mut().brush.color.per_tip = true;
        if random {
            let hue = &mut h.m2_mut().brush.influences.color_hue;
            hue.random.on = true;
            hue.random.response =
                yolu_app::engine::InputResponse::new_signed(-1.0, Vec::new()).unwrap();
        }
        h.color_mut().main = [0.9, 0.2, 0.2, 1.0];
        let b = h.brush_mut();
        b.radius = 6.0;
        b.hardness = 1.0;
        b.spacing = 0.1;
        b.pressure.size = false;
        b.pressure.opacity = false;
        h.run();
        let c = canvas_rect(&h).center();
        drag(
            &mut h,
            &[
                offset(c, -60.0, 0.0),
                offset(c, 0.0, 0.0),
                offset(c, 60.0, 0.0),
            ],
        );
        let layer = h.selected_layer().unwrap();
        let bytes = h
            .doc()
            .layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .map(|s| s.to_canvas_bytes())
            .expect("描いた");
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    };
    let steady = draw(false);
    let varied = draw(true);
    assert_eq!(
        steady.len(),
        1,
        "影響元を使わなければ、1 つの色: {steady:?}"
    );
    assert!(
        varied.len() > 3,
        "ランダムを入れると色相がばらつく: {}",
        varied.len()
    );
}

/// 消すモード: コマンドで入り切りし、入っている間は、ブラシのツールでも今のブラシの形のまま消す（実際の入力で描く）。ブラシには保存せず、ツールを替えても残る。
#[test]
fn the_erase_mode_command_erases_with_the_current_brush_and_stays_across_tools() {
    let mut h = app(1600.0, 900.0, 128);
    {
        let b = h.brush_mut();
        b.radius = 10.0;
        b.hardness = 1.0;
        b.pressure.size = false;
        b.pressure.opacity = false;
    }
    h.run();
    let c = canvas_rect(&h).center();
    let line = [
        offset(c, -40.0, 0.0),
        offset(c, 0.0, 0.0),
        offset(c, 40.0, 0.0),
    ];
    let alpha_sum = |h: &H| -> u64 {
        let layer = h.selected_layer().unwrap();
        h.doc()
            .layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .map_or(0, |s| {
                s.to_canvas_bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| u64::from(p[3]))
                    .sum()
            })
    };
    drag(&mut h, &line);
    let painted = alpha_sum(&h);
    assert!(painted > 0);
    assert!(canvas_pixel(&h, c)[3] > 200, "描けた");
    // コマンド（既定のキーは無い）で入れる。ブラシは変わらず、保存する設定でもない
    let command = yolu_app::commands::find("brush.erase_mode").expect("コマンド");
    assert!(command.rebindable());
    assert!(!h.m2().erase_mode);
    let before = h.app().brush_is_modified(b("standard"));
    h.act(command.runnable().expect("実行できる"));
    assert!(h.m2().erase_mode);
    assert_eq!(
        h.app().brush_is_modified(b("standard")),
        before,
        "ブラシの設定ではない"
    );
    // 同じ線を重ねると消える（ブラシのまま消す）。消した線も、描いた線と同じ 1 回の Undo
    let steps = h.doc().undo_count();
    drag(&mut h, &line);
    let centre = |h: &H| canvas_pixel(h, c)[3];
    assert_eq!(centre(&h), 0, "同じブラシの形で線の芯は消える");
    assert!(
        alpha_sum(&h) * 5 < painted,
        "縁の半透明の分だけが残る: {}",
        alpha_sum(&h)
    );
    assert_eq!(h.doc().undo_count(), steps + 1);
    // ツールを替えても残る（ブラシのツールへ戻しても入ったまま）
    h.act(Action::SelectTool(Tool::Fill));
    h.act(Action::SelectTool(Tool::Brush));
    assert!(h.m2().erase_mode);
    // 切ると、また描ける
    h.act(command.runnable().unwrap());
    assert!(!h.m2().erase_mode);
    drag(&mut h, &line);
    assert!(alpha_sum(&h) > painted * 3 / 4, "消えずに描き足される");
    // 詳細の「消去」の行は同じ設定
    open_detail(&mut h, Category::Erase);
    let at = in_pane(&h, "消去").center();
    click(&mut h, at);
    assert!(h.m2().erase_mode);
    h.get_by_label("消去を既定に戻す").click();
    h.run();
    assert!(!h.m2().erase_mode);
}

/// 描いたレイヤーの、列 `x` の覆いの合計（文書の画素。半透明の縁も覆いの分だけ数える）。
fn column_coverage(h: &H, x: usize) -> f32 {
    let layer = h.selected_layer().unwrap();
    let bytes = h
        .doc()
        .layer(layer)
        .unwrap()
        .surface(Channel::Color)
        .map(|s| s.to_canvas_bytes())
        .expect("描いた");
    let w = h.doc().width() as usize;
    (0..h.doc().height() as usize)
        .map(|y| f32::from(bytes[(y * w + x) * 4 + 3]) / 255.0)
        .sum()
}

fn on_screen_brush(h: &mut H, radius: f32) {
    h.m2_mut().brush.controls.size_by_screen = true;
    let b = h.brush_mut();
    b.radius = radius;
    b.hardness = 1.0;
    b.spacing = 0.05;
    b.pressure.size = false;
    b.pressure.opacity = false;
    h.run();
}

/// 画面上のサイズで指定: 表示倍率を変えても、線の画面上の太さは同じ（文書の画素での太さは倍率に反比例）。切ると文書の画素の大きさのまま。
#[test]
fn a_brush_sized_on_screen_keeps_its_width_on_screen_at_every_zoom() {
    let mut h = app(1600.0, 900.0, 256);
    on_screen_brush(&mut h, 40.0);
    let canvas = canvas_rect(&h);
    let line = |h: &mut H| {
        let c = canvas.center();
        drag(
            h,
            &[
                offset(c, -80.0, 0.0),
                offset(c, 0.0, 0.0),
                offset(c, 80.0, 0.0),
            ],
        );
    };
    let mut docs = Vec::new();
    for zoom in [1.0f32, 2.0, 4.0] {
        h.view_mut().zoom = zoom;
        h.run();
        let p = h
            .view()
            .view(canvas_rect(&h), h.doc().width(), h.doc().height())
            .pixel_size();
        line(&mut h);
        let doc_width = column_coverage(&h, h.doc().width() as usize / 2);
        // 直径 80 点の線が、どの倍率でも画面で 80 点（文書の画素の丸めの分は 1 割まで）
        let on_screen = doc_width * p;
        assert!(
            (on_screen - 80.0).abs() <= 8.0,
            "倍率 {zoom}: 画面の太さ {on_screen}"
        );
        docs.push(doc_width);
        h.act(Action::Undo);
        h.run();
    }
    // 倍率が 2 倍になると、文書の画素での太さは半分
    assert!((docs[0] / docs[1] - 2.0).abs() < 0.3, "{docs:?}");
    assert!((docs[1] / docs[2] - 2.0).abs() < 0.3, "{docs:?}");
    // 切ると、文書の画素で一定（倍率が上がると画面で太くなる）
    h.m2_mut().brush.controls.size_by_screen = false;
    h.brush_mut().radius = 8.0;
    let mut fixed = Vec::new();
    for zoom in [1.0f32, 4.0] {
        h.view_mut().zoom = zoom;
        h.run();
        line(&mut h);
        fixed.push(column_coverage(&h, h.doc().width() as usize / 2));
        h.act(Action::Undo);
        h.run();
    }
    assert!(
        (fixed[0] - fixed[1]).abs() < 1.0,
        "切のときは文書の画素の大きさのまま: {fixed:?}"
    );
    // 設定の値は倍率で書き換えない
    assert_eq!(h.brush().radius, 8.0);
}

/// 描き始めたストロークは、途中で表示を拡大・縮小しても変わらない（始めの半径で固める）。
#[test]
fn the_screen_size_is_fixed_when_the_stroke_starts() {
    let mut h = app(1600.0, 900.0, 256);
    on_screen_brush(&mut h, 40.0);
    let canvas = canvas_rect(&h);
    let c = canvas.center();
    // 倍率 2 で始めて、途中で 1 へ替えても、そのストロークの半径は始めの倍率のまま
    h.view_mut().zoom = 2.0;
    h.run();
    let p = h
        .view()
        .view(canvas_rect(&h), h.doc().width(), h.doc().height())
        .pixel_size();
    press(&h, offset(c, -60.0, 0.0), egui::PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, -20.0, 0.0));
    h.step();
    h.view_mut().zoom = 1.0;
    h.run();
    move_to(&h, offset(c, 20.0, 0.0));
    h.step();
    release(&h, offset(c, 20.0, 0.0), egui::PointerButton::Primary);
    h.step();
    h.run();
    // 描いた線のどの列でも、太さは始めの倍率の太さ（直径 80 点 ÷ p）を超えない。始めの倍率が 1 に替わっていれば 2 倍になる
    let expected = 80.0 / p;
    let mut widest = 0.0f32;
    for x in 0..h.doc().width() as usize {
        widest = widest.max(column_coverage(&h, x));
    }
    assert!(
        (widest - expected).abs() <= expected * 0.15,
        "最も太い列 {widest} / 始めの倍率の太さ {expected}"
    );
}

/// 2-紙質の区分: 2-ブラシ形状が切のうちは触れず、使うと画像・濃さ・回転・合わせ方が 2 つ目の質感だけを変える（主の紙質は変わらない）。
/// 区分の「既定に戻す」で外れ、2-ブラシ形状が切のとき戻した値は、使い直しても残らない。
#[test]
fn the_second_paper_page_edits_only_the_dual_texture() {
    use yolu_app::m2::{BrushOp, UiOp};
    use yolu_core::TextureMode;
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::DualPaper);
    // 左の一覧は 2-ブラシ形状の子に並ぶ。切のうちは画像の箱も薄い
    assert_eq!(Category::DualPaper.parent(), Some(Category::Dual));
    let image = "2-紙質の画像: なし";
    assert!(h.get_by_label(image).accesskit_node().is_disabled());
    h.act(Action::M2Ui(UiOp::Brush(BrushOp::DualEnabled(true))));
    h.run();
    assert!(!h.get_by_label(image).accesskit_node().is_disabled());
    // 質感を付けるまでは、濃さ・回転・合わせ方は触れない
    assert!(h
        .get_by_label("2-質感のモード: 乗算")
        .accesskit_node()
        .is_disabled());
    let at = in_pane(&h, image).center();
    click(&mut h, at);
    let at = popup_item(&h, "粒子").center();
    click(&mut h, at);
    let dual = h
        .m2()
        .brush
        .dual
        .as_ref()
        .expect("2-ブラシ形状は入っている");
    assert!(dual.texture.is_some(), "2 つ目の質感が付く");
    assert!(h.m2().brush.texture.is_none(), "主の紙質は変わらない");
    // 濃さ・回転・合わせ方
    let depth = in_pane(&h, "2-質感の濃さ");
    click(&mut h, pos2(depth.right() - 1.0, depth.center().y));
    let rotation = in_pane(&h, "2-紙質の回転");
    click(&mut h, pos2(rotation.right() - 1.0, rotation.center().y));
    let at = in_pane(&h, "2-質感のモード: 乗算").center();
    click(&mut h, at);
    let at = popup_item(&h, "オーバーレイ").center();
    click(&mut h, at);
    let texture = h.m2().brush.dual.as_ref().unwrap().texture.clone().unwrap();
    assert!(texture.depth > 0.95, "{}", texture.depth);
    assert!(texture.angle > 170.0, "{}", texture.angle);
    assert_eq!(texture.mode, TextureMode::Overlay);
    assert!(h.m2().brush.texture.is_none());
    // 画像を替えても、濃さ・回転・合わせ方は残る
    let at = in_pane(&h, "2-紙質の画像: 粒子").center();
    click(&mut h, at);
    let at = popup_item(&h, "ドット").center();
    click(&mut h, at);
    let again = h.m2().brush.dual.as_ref().unwrap().texture.clone().unwrap();
    assert_ne!(again.image, texture.image);
    assert_eq!(
        (again.depth, again.angle, again.mode),
        (texture.depth, texture.angle, texture.mode)
    );
    // 目の印は初めは無い（ツールプロパティに出さない）。区分の「既定に戻す」で外れる
    assert!(!rows::is_shown(&h.m2().brush, Row::DualPaperImage));
    h.get_by_label("2-紙質を既定に戻す").click();
    h.run();
    assert!(h.m2().brush.dual.as_ref().unwrap().texture.is_none());
    // どの設定も Undo の段を作らない
    assert!(!h.doc().can_undo());
}
