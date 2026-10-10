//! 選択範囲と 2D の対称定規で描く操作（egui_kittest）。どれも「操作 → 文書が変わる → Undo で戻る」。ツールの帯・オプションバー・メニュー・
//! プロパティの欄・量を聞くウィンドウ・キャンバスの入力・対称定規のブラシ・.ylp の保存と読み込み。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{
    BrushEffect, Channel, CoreError, Rgba8, SelectionCombine, SelectionMask, TileCoord,
    DEFAULT_WORKING_BUDGET_BYTES,
};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::PenSample;
use yolu_app::rulers::RulerAction;
use yolu_app::selection::{ModifyKind, SelAction, SelEdit, SelUiOp};
use yolu_app::state::{Action, AppState, PopupKind, Tool};
use yolu_app::YoluApp;
use yolu_core::{Ruler, RulerKind};

type H = Harness<'static, YoluApp>;

// ───────── ツール ─────────

fn st(h: &H) -> &AppState {
    &h.state().state
}

/// 今の対称定規を全部外して、`put` で新しい対称定規を選んでいるレイヤーに置く（置いた定規の分の取り消しの段が 1 つ増える）。
fn symmetry(h: &mut H, put: impl FnOnce(&mut AppState)) {
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        let ids: Vec<_> = common::rulers::of(s, layer).iter().map(|r| r.id).collect();
        if !ids.is_empty() {
            s.apply(Action::Ruler(RulerAction::Delete { owner: layer, ids }));
        }
        put(s);
    }
    h.run();
}

/// 「特殊定規にスナップ」を入れ直す。
fn special_snap(h: &mut H, on: bool) {
    let s = &mut h.state_mut().state;
    if s.rulers.snap_special != on {
        s.apply(Action::Ruler(RulerAction::ToggleSnapSpecial));
    }
    h.run();
}

fn pick_tool(h: &mut H, tool: Tool) {
    h.state_mut().state.apply(Action::SelectTool(tool));
    h.run();
}

fn undo(h: &mut H) {
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
}

fn redo(h: &mut H) {
    key(h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
}

fn steps(h: &H) -> usize {
    st(h).doc.undo_count()
}

/// 画面の点の下の、選択範囲の量（選択なしは 0）。
fn amount_at(h: &H, p: Pos2) -> u8 {
    let s = st(h);
    let r = canvas_rect(h);
    let (x, y) = s.view.view(r, s.doc.width(), s.doc.height()).to_canvas(p);
    s.doc
        .selection()
        .map_or(0, |m| m.amount(x.floor() as u32, y.floor() as u32))
}

fn selected(h: &H, p: Pos2) -> bool {
    amount_at(h, p) == 255
}

/// 修飾キーを押したままのドラッグ（離したときの修飾で組み合わせ方が決まる）。
fn drag_with(h: &mut H, points: &[Pos2], modifiers: Modifiers) {
    h.event(Event::PointerMoved(points[0]));
    h.event(Event::PointerButton {
        pos: points[0],
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    h.event(Event::PointerButton {
        pos: *points.last().unwrap(),
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    h.step();
    h.run();
}

/// キャンバスの中心から (dx, dy) の点。
fn at(h: &H, dx: f32, dy: f32) -> Pos2 {
    offset(canvas_rect(h).center(), dx, dy)
}

/// キャンバスの中心からのずれの点をなぞる（`&mut h` と `at(&h, ..)` を同じ式に書けないので、ずれのまま渡す）。
fn drag_by(h: &mut H, points: &[(f32, f32)]) {
    let pts: Vec<Pos2> = points.iter().map(|&(x, y)| at(h, x, y)).collect();
    drag(h, &pts);
}

fn drag_with_by(h: &mut H, points: &[(f32, f32)], modifiers: Modifiers) {
    let pts: Vec<Pos2> = points.iter().map(|&(x, y)| at(h, x, y)).collect();
    drag_with(h, &pts, modifiers);
}

fn click_by(h: &mut H, dx: f32, dy: f32) {
    let p = at(h, dx, dy);
    click(h, p);
}

/// 矩形のドラッグ（キャンバスの中心からのずれで 2 つの角）。
fn drag_rect(h: &mut H, a: (f32, f32), b: (f32, f32)) {
    let (p, q) = (at(h, a.0, a.1), at(h, b.0, b.1));
    drag(h, &[p, q]);
}

/// 同じ場所に点を重ねないよう、クリックの間を空ける（ダブルクリックにならないように）。
fn wait(h: &mut H) {
    for _ in 0..30 {
        h.step();
    }
}

/// 色の矩形をレイヤーの Color へ入れる（キャンバスの座標。タイルごとに読み込む）。
fn paint_rect(s: &mut AppState, x0: u32, y0: u32, x1: u32, y1: u32, rgba: [u8; 4]) {
    let id = s.selected_layer.unwrap();
    let ts = s.doc.tile_size();
    for ty in 0..s.doc.height().div_ceil(ts) {
        for tx in 0..s.doc.width().div_ceil(ts) {
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            let mut any = false;
            for ly in 0..ts {
                for lx in 0..ts {
                    let (x, y) = (tx * ts + lx, ty * ts + ly);
                    if x >= x0 && x < x1 && y >= y0 && y < y1 {
                        bytes[((ly * ts + lx) * 4) as usize..][..4].copy_from_slice(&rgba);
                        any = true;
                    }
                }
            }
            if any {
                s.doc
                    .import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
}

fn alpha_at(h: &H, p: Pos2) -> u8 {
    canvas_pixel(h, p)[3]
}

// ───────── ツールの帯とキー ─────────

#[test]
fn tool_strip_buttons_and_keys_pick_the_selection_tools() {
    let mut h = app(1000.0, 640.0, 256);
    for (label, tool) in [
        ("長方形選択（M）", Tool::SelectRect),
        ("楕円形選択（Shift+M）", Tool::SelectEllipse),
        ("なげなわ（L）", Tool::Lasso),
        ("多角形選択（Shift+L）", Tool::Polygon),
        ("自動選択（W）", Tool::Wand),
        ("ブラシ（B）", Tool::Brush),
    ] {
        h.get_by_label(label).click();
        h.run();
        assert_eq!(st(&h).tool, tool, "{label}");
    }
    for (k, mods, tool) in [
        (Key::M, Modifiers::NONE, Tool::SelectRect),
        (Key::M, Modifiers::SHIFT, Tool::SelectEllipse),
        (Key::L, Modifiers::NONE, Tool::Lasso),
        (Key::L, Modifiers::SHIFT, Tool::Polygon),
        (Key::W, Modifiers::NONE, Tool::Wand),
        (Key::B, Modifiers::NONE, Tool::Brush),
        (Key::E, Modifiers::NONE, Tool::Eraser),
    ] {
        key(&h, k, mods);
        h.run();
        assert_eq!(st(&h).tool, tool, "{k:?} {mods:?}");
    }
}

#[test]
fn tool_names_and_option_bar_follow_the_language() {
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    for label in [
        "Rectangle Select (M)",
        "Ellipse Select (Shift+M)",
        "Lasso (L)",
        "Polygon Select (Shift+L)",
        "Magic Wand (W)",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("Magic Wand (W)").click();
    h.run();
    // 作成方法は、オプションバー（4 つ。名前とキーのツールチップ）とツールプロパティ（新規・追加・削除に「⋯」）の両方に。
    // 許容値はオプションバーとツールプロパティ、隣接と全レイヤーはツールプロパティ
    for label in [
        "New",
        "Add (Shift)",
        "Subtract (Ctrl)",
        "Intersect (Shift+Ctrl)",
        "Tolerance",
    ] {
        bar_rect(&h, label);
    }
    for label in ["New", "Add", "Subtract", "All modes", "Tolerance"] {
        dock_rect(&h, label);
    }
    for label in ["Contiguous", "Sample All Layers"] {
        dock_rect(&h, label);
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label}");
    }
    h.state_mut().state.sel.animate = false;
    h.snapshot("selection_english");
}

#[test]
fn english_select_menu_and_amount_window_work() {
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    let title = menu_title(&h, "Select").center();
    click(&mut h, title);
    let item = popup_item(&h, "Select All").center();
    click(&mut h, item);
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    let title = menu_title(&h, "Select").center();
    click(&mut h, title);
    let item = popup_item(&h, "Grow…").center();
    click(&mut h, item);
    assert_eq!(
        st(&h).sel.dialog.map(|d| d.kind),
        Some(ModifyKind::Grow),
        "{:?}",
        st(&h).message
    );
    h.get_by_label("Cancel").click();
    h.run();
    assert!(st(&h).sel.dialog.is_none());
    // 同じ操作のステータスバーの文も英語
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    assert_eq!(st(&h).message, "Deselected.");
}

// ───────── 形 ─────────

#[test]
fn rectangle_drag_selects_and_undo_and_redo_follow() {
    let mut h = app(1000.0, 640.0, 512);
    key(&h, Key::M, Modifiers::NONE);
    h.run();
    h.state_mut().state.sel.animate = false;
    let before = steps(&h);
    drag_rect(&mut h, (-100.0, -60.0), (120.0, 80.0));
    assert_eq!(steps(&h), before + 1, "1 回の Undo");
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    assert!(selected(&h, at(&h, -90.0, -50.0)));
    assert_eq!(amount_at(&h, at(&h, -130.0, 0.0)), 0);
    assert_eq!(amount_at(&h, at(&h, 0.0, 120.0)), 0);
    assert!(st(&h).modified);
    h.snapshot("selection_rect_ants");
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none());
    redo(&mut h);
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    assert!(st(&h).canvas.stroke.is_none() && !st(&h).doc.has_active_stroke());
}

#[test]
fn ellipse_and_lasso_drags_select_their_shapes() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectEllipse);
    drag_rect(&mut h, (-100.0, -60.0), (100.0, 60.0));
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    assert_eq!(amount_at(&h, at(&h, 95.0, 55.0)), 0, "矩形の角は楕円の外");
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none());
    pick_tool(&mut h, Tool::Lasso);
    let before = steps(&h);
    let triangle = [
        at(&h, -100.0, -80.0),
        at(&h, 100.0, -80.0),
        at(&h, 0.0, 80.0),
        at(&h, -100.0, -80.0),
    ];
    // 投げ縄は途中の点をなぞる
    let mut path = Vec::new();
    for w in triangle.windows(2) {
        for i in 0..8 {
            path.push(w[0] + (w[1] - w[0]) * (i as f32 / 8.0));
        }
    }
    path.push(triangle[3]);
    drag(&mut h, &path);
    assert_eq!(steps(&h), before + 1);
    assert!(selected(&h, at(&h, 0.0, -40.0)), "三角形の中");
    assert_eq!(amount_at(&h, at(&h, -90.0, 60.0)), 0, "三角形の外");
}

#[test]
fn polygon_clicks_close_by_enter_first_point_or_double_click() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::Polygon);
    h.state_mut().state.sel.animate = false;
    let square = [(-80.0, -60.0), (80.0, -60.0), (80.0, 60.0), (-80.0, 60.0)];
    // Enter で閉じる
    for (dx, dy) in square {
        click_by(&mut h, dx, dy);
        wait(&mut h);
    }
    assert_eq!(st(&h).sel.polygon.len(), 4);
    assert!(st(&h).doc.selection().is_none(), "閉じるまでは選ばない");
    h.snapshot("selection_polygon_draft");
    let before = steps(&h);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(steps(&h), before + 1);
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    assert!(st(&h).sel.polygon.is_empty());
    undo(&mut h);
    // 始めの点を押して閉じる
    for (dx, dy) in square {
        click_by(&mut h, dx, dy);
        wait(&mut h);
    }
    click_by(&mut h, square[0].0 + 2.0, square[0].1 + 2.0);
    assert!(selected(&h, at(&h, 0.0, 0.0)), "始めの点で閉じる");
    assert_eq!(st(&h).sel.polygon.len(), 0);
    undo(&mut h);
    // ダブルクリックで閉じる（最後の点を 2 回）
    for (dx, dy) in &square[..3] {
        click_by(&mut h, *dx, *dy);
        wait(&mut h);
    }
    let last = at(&h, square[3].0, square[3].1);
    click(&mut h, last);
    click(&mut h, last);
    assert!(selected(&h, at(&h, 0.0, 0.0)), "ダブルクリックで閉じる");
}

#[test]
fn polygon_backspace_escape_tool_switch_and_focus_loss_drop_the_draft() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::Polygon);
    for (dx, dy) in [(-80.0, -60.0), (80.0, -60.0), (0.0, 60.0)] {
        click_by(&mut h, dx, dy);
        wait(&mut h);
    }
    assert_eq!(st(&h).sel.polygon.len(), 3);
    key(&h, Key::Backspace, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).sel.polygon.len(), 2);
    // 取り消しのキーは、多角形の途中なら文書でなく最後の点に当たる
    let steps_before = steps(&h);
    undo(&mut h);
    assert_eq!(st(&h).sel.polygon.len(), 1);
    assert_eq!(steps(&h), steps_before);
    // メニューの「取り消し」の有効・無効も、キーが当たる先に合わせる（文書に戻す段が無くても、点がある間は押せる）
    assert!(!st(&h).doc.can_undo() && st(&h).can_undo());
    undo(&mut h);
    assert!(st(&h).sel.polygon.is_empty());
    assert!(!st(&h).can_undo(), "点も段も無ければ押せない");
    click_by(&mut h, -80.0, -60.0);
    wait(&mut h);
    assert!(
        st(&h).can_redo() == st(&h).doc.can_redo(),
        "やり直しは点に対応しない"
    );
    click_by(&mut h, 80.0, -60.0);
    wait(&mut h);
    assert_eq!(st(&h).sel.polygon.len(), 2);
    undo(&mut h);
    assert_eq!(st(&h).sel.polygon.len(), 1);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(st(&h).sel.polygon.is_empty() && st(&h).doc.selection().is_none());
    // ツールを替えたら捨てる
    click_by(&mut h, 10.0, 10.0);
    assert_eq!(st(&h).sel.polygon.len(), 1);
    pick_tool(&mut h, Tool::SelectRect);
    assert!(st(&h).sel.polygon.is_empty());
    // フォーカスを失ったら、ドラッグも多角形も捨てる
    pick_tool(&mut h, Tool::Polygon);
    click_by(&mut h, 10.0, 10.0);
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(st(&h).sel.polygon.is_empty());
    pick_tool(&mut h, Tool::SelectRect);
    press(&h, at(&h, 0.0, 0.0), PointerButton::Primary);
    move_to(&h, at(&h, 50.0, 40.0));
    h.step();
    assert!(st(&h).sel.drag.is_some());
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(st(&h).sel.drag.is_none());
    release(&h, at(&h, 50.0, 40.0), PointerButton::Primary);
    h.run();
    assert!(st(&h).doc.selection().is_none(), "取り消した形は選ばない");
}

#[test]
fn escape_cancels_a_drag_and_a_click_without_moving_deselects() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    press(&h, at(&h, -50.0, -50.0), PointerButton::Primary);
    move_to(&h, at(&h, 60.0, 40.0));
    h.step();
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    release(&h, at(&h, 60.0, 40.0), PointerButton::Primary);
    h.run();
    assert!(st(&h).doc.selection().is_none(), "Esc で形を捨てる");
    assert_eq!(steps(&h), 0);
    drag_rect(&mut h, (-50.0, -50.0), (60.0, 40.0));
    let before = steps(&h);
    click_by(&mut h, 200.0, 100.0);
    assert!(st(&h).doc.selection().is_none(), "クリックだけで解除");
    assert_eq!(steps(&h), before + 1);
    undo(&mut h);
    assert!(selected(&h, at(&h, 0.0, 0.0)));
}

#[test]
fn wand_selects_contiguous_or_everywhere_inside_tolerance() {
    let mut h = app(1000.0, 640.0, 256);
    // 左下と右上のタイル（128）が赤、ほかは透明
    paint_rect(&mut h.state_mut().state, 0, 0, 128, 128, [255, 0, 0, 255]);
    paint_rect(
        &mut h.state_mut().state,
        128,
        128,
        256,
        256,
        [255, 0, 0, 255],
    );
    h.run();
    pick_tool(&mut h, Tool::Wand);
    let r = canvas_rect(&h);
    let view = st(&h).view.view(r, 256, 256);
    let s = |x: f64, y: f64| view.to_screen(x, y);
    let before = steps(&h);
    click(&mut h, s(60.0, 60.0));
    assert_eq!(steps(&h), before + 1);
    assert!(selected(&h, s(60.0, 60.0)));
    assert_eq!(
        amount_at(&h, s(200.0, 200.0)),
        0,
        "隣接: つながらない赤は選ばない"
    );
    undo(&mut h);
    // 隣接を切ると、同じ色の画素を全部
    h.state_mut().state.sel.contiguous = false;
    click(&mut h, s(60.0, 60.0));
    assert!(selected(&h, s(60.0, 60.0)) && selected(&h, s(200.0, 200.0)));
    assert_eq!(amount_at(&h, s(200.0, 60.0)), 0);
    undo(&mut h);
    // 許容値が大きければ透明の画素も入る（RGBA の差が 255 以下）
    h.state_mut().state.sel.tolerance = 255;
    click(&mut h, s(60.0, 60.0));
    assert!(selected(&h, s(200.0, 60.0)));
}

// ───────── 組み合わせ ─────────

#[test]
fn option_bar_modes_and_modifier_keys_combine_shapes() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    drag_rect(&mut h, (-120.0, -60.0), (-10.0, 60.0)); // 左
    let (left, right) = (at(&h, -90.0, 0.0), at(&h, 60.0, 0.0));
    assert!(selected(&h, left) && amount_at(&h, right) == 0);
    // オプションバーの「足す」
    let button = bar_rect(&h, "追加（Shift）").center();
    click(&mut h, button);
    assert_eq!(st(&h).sel.combine, SelectionCombine::Add);
    drag_rect(&mut h, (10.0, -60.0), (120.0, 60.0));
    assert!(selected(&h, left) && selected(&h, right), "足す");
    // 「引く」: 左の外側を引く
    let button = bar_rect(&h, "削除（Ctrl）").center();
    click(&mut h, button);
    drag_rect(&mut h, (-130.0, -80.0), (-60.0, 80.0));
    assert_eq!(amount_at(&h, left), 0, "引いた所は外れる");
    assert!(selected(&h, at(&h, -30.0, 0.0)), "引いていない所は残る");
    assert!(selected(&h, right));
    // 「重ねる」: 右の半分と重なる所だけ
    let button = bar_rect(&h, "共通（Shift+Ctrl）").center();
    click(&mut h, button);
    drag_rect(&mut h, (0.0, -80.0), (130.0, 80.0));
    assert!(selected(&h, right));
    assert_eq!(amount_at(&h, at(&h, -30.0, 0.0)), 0);
    // 置き換え + Shift で足す・Ctrl で引く・Shift+Ctrl で重ねる
    let button = bar_rect(&h, "新規").center();
    click(&mut h, button);
    drag_rect(&mut h, (-120.0, -60.0), (0.0, 60.0));
    drag_with_by(&mut h, &[(10.0, -60.0), (120.0, 60.0)], Modifiers::SHIFT);
    assert!(selected(&h, left) && selected(&h, right), "Shift で足す");
    drag_with_by(
        &mut h,
        &[(-130.0, -80.0), (-60.0, 80.0)],
        Modifiers::COMMAND,
    );
    assert_eq!(amount_at(&h, left), 0, "Ctrl で引く");
    assert!(selected(&h, right));
    drag_with_by(
        &mut h,
        &[(40.0, -80.0), (130.0, 80.0)],
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert!(selected(&h, at(&h, 80.0, 0.0)), "Shift+Ctrl で重ねる");
    assert_eq!(amount_at(&h, at(&h, 20.0, 0.0)), 0);
    assert_eq!(
        st(&h).sel.combine,
        SelectionCombine::Replace,
        "修飾はオプションバーの値を変えない"
    );
}

#[test]
fn select_all_deselect_and_invert_are_in_the_select_menu_not_the_tool_properties() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    // ツールプロパティ・オプションバーには、操作のボタンを置かない
    for label in [
        "すべてを選択（Ctrl+A）",
        "選択を解除（Ctrl+D）",
        "選択範囲を反転（Ctrl+Shift+I）",
    ] {
        assert!(h.query_all_by_label(label).next().is_none(), "{label}");
    }
    let run = |h: &mut H, label: &str| {
        let title = menu_title(h, "選択範囲").center();
        click(h, title);
        let item = popup_item(h, label).center();
        click(h, item);
    };
    run(&mut h, "すべてを選択");
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    run(&mut h, "選択を解除");
    assert!(st(&h).doc.selection().is_none());
    drag_rect(&mut h, (-50.0, -50.0), (50.0, 50.0));
    run(&mut h, "選択範囲を反転");
    assert_eq!(amount_at(&h, at(&h, 0.0, 0.0)), 0);
    assert!(selected(&h, at(&h, 100.0, 0.0)));
    undo(&mut h);
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none());
}

// ───────── メニュー・キー・量を聞くウィンドウ ─────────

#[test]
fn select_menu_items_and_keys_run_their_edits() {
    let mut h = app(1000.0, 640.0, 256);
    // 選択範囲が無いと、解除・反転・拡張などは押せない
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    assert_eq!(
        st(&h).popup.as_ref().map(|p| p.kind),
        Some(PopupKind::MenuBar(3))
    );
    let item = popup_item(&h, "選択を解除").center();
    click(&mut h, item);
    assert!(st(&h).popup.is_some(), "無効の項目は閉じない");
    let item = popup_item(&h, "すべてを選択").center();
    click(&mut h, item);
    assert!(st(&h).popup.is_none());
    assert!(selected(&h, at(&h, 0.0, 0.0)) && selected(&h, at(&h, 120.0, 90.0)));
    // キー
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    assert!(st(&h).doc.selection().is_none());
    key(&h, Key::A, Modifiers::COMMAND);
    h.run();
    assert!(st(&h).doc.selection().is_some());
    key(&h, Key::I, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert!(
        st(&h).doc.selection().is_none(),
        "全部を反転すると何も残らない"
    );
    undo(&mut h);
    assert!(st(&h).doc.selection().is_some());
    // ツールの項目
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    let item = popup_item(&h, "多角形選択").center();
    click(&mut h, item);
    assert_eq!(st(&h).tool, Tool::Polygon);
    h.state_mut().state.sel.animate = false;
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    h.snapshot("selection_select_menu");
}

#[test]
fn amount_dialog_applies_with_ok_or_enter_and_cancels_with_escape_or_the_button() {
    let mut h = app(1280.0, 800.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    drag_rect(&mut h, (-60.0, -40.0), (60.0, 40.0));
    let original = st(&h).doc.selection().unwrap().clone();
    let open = |h: &mut H, label: &str| {
        let title = menu_title(h, "選択範囲").center();
        click(h, title);
        let item = popup_item(h, label).center();
        click(h, item);
    };
    open(&mut h, "拡張…");
    assert!(st(&h).sel.dialog.is_some());
    // ウィンドウの外のキーは効かない（モーダル）
    key(&h, Key::W, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).tool, Tool::SelectRect);
    h.state_mut().state.sel.dialog.as_mut().unwrap().radius = 12;
    h.run();
    h.state_mut().state.sel.animate = false;
    h.snapshot("selection_amount_dialog");
    let before = steps(&h);
    h.get_by_label("OK").click();
    h.run();
    assert!(st(&h).sel.dialog.is_none());
    assert_eq!(steps(&h), before + 1, "1 回の Undo");
    let grown = original.grow(12, DEFAULT_WORKING_BUDGET_BYTES).unwrap();
    assert_eq!(st(&h).doc.selection(), Some(&grown));
    assert_eq!(st(&h).sel.radius, 12, "次に開くときは同じ半径");
    undo(&mut h);
    assert_eq!(st(&h).doc.selection(), Some(&original));
    // Esc で取り消し
    open(&mut h, "境界をぼかす…");
    assert_eq!(st(&h).sel.dialog.map(|d| d.radius), Some(12));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(st(&h).sel.dialog.is_none());
    assert_eq!(steps(&h), before, "取り消しは何も変えない");
    // キャンセルのボタン
    open(&mut h, "縮小…");
    h.get_by_label("キャンセル").click();
    h.run();
    assert!(st(&h).sel.dialog.is_none());
    assert_eq!(st(&h).doc.selection(), Some(&original));
    // Enter で適用（縮小は端を固定の設定も渡す）
    open(&mut h, "縮小…");
    h.state_mut().state.sel.dialog.as_mut().unwrap().radius = 5;
    h.state_mut().state.sel.dialog.as_mut().unwrap().edge_lock = true;
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    let shrunk = original
        .shrink(5, true, DEFAULT_WORKING_BUDGET_BYTES)
        .unwrap();
    assert_eq!(st(&h).doc.selection(), Some(&shrunk));
    assert!(st(&h).sel.edge_lock);
}

#[test]
fn select_menu_modifies_the_selection_with_the_radius_and_edge_lock() {
    let mut h = app(1280.0, 800.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    drag_rect(&mut h, (-40.0, -30.0), (40.0, 30.0));
    let original = st(&h).doc.selection().unwrap().clone();
    let budget = DEFAULT_WORKING_BUDGET_BYTES;
    let open = |h: &mut H, label: &str| {
        let title = menu_title(h, "選択範囲").center();
        click(h, title);
        let item = popup_item(h, label).center();
        click(h, item);
    };
    // 半径を使う 4 つは、量のウィンドウを開いて半径を決めて適用する。境界をくっきりは直に適用する
    for (label, expect) in [
        ("拡張…", original.grow(6, budget).unwrap()),
        ("縮小…", original.shrink(6, false, budget).unwrap()),
        ("境界線…", original.border(6, false, budget).unwrap()),
        (
            "境界をぼかす…",
            original.feather(6.0, false, budget).unwrap(),
        ),
        ("境界をくっきり", original.sharpen()),
    ] {
        let before = steps(&h);
        open(&mut h, label);
        if label.ends_with('…') {
            h.state_mut().state.sel.dialog.as_mut().unwrap().radius = 6;
            key(&h, Key::Enter, Modifiers::NONE);
            h.run();
        }
        if expect == original {
            assert_eq!(steps(&h), before, "{label}: 変わらないなら段を積まない");
        } else {
            assert_eq!(steps(&h), before + 1, "{label}");
            assert_eq!(st(&h).doc.selection(), Some(&expect), "{label}");
            undo(&mut h);
        }
        assert_eq!(
            st(&h).doc.selection(),
            Some(&original),
            "{label} の後で戻る"
        );
    }
    // 選択範囲が無ければ、項目は押せない
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    let before = steps(&h);
    open(&mut h, "拡張…");
    assert!(st(&h).sel.dialog.is_none());
    assert_eq!(steps(&h), before);
}

// ───────── 選択の内側だけに効く ─────────

#[test]
fn brush_and_eraser_only_change_pixels_inside_the_selection() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    drag_rect(&mut h, (-60.0, -80.0), (60.0, 80.0));
    pick_tool(&mut h, Tool::Brush);
    let (inside, outside) = (at(&h, 0.0, 0.0), at(&h, -150.0, 0.0));
    drag_by(
        &mut h,
        &[
            (-200.0, 0.0),
            (-100.0, 0.0),
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 0.0),
        ],
    );
    assert_eq!(alpha_at(&h, inside), 255, "選択の内側は塗る");
    assert_eq!(alpha_at(&h, outside), 0, "外は塗らない");
    // 消しゴム: 選択を外して全体に塗ってから、選択の内側だけ消す
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    drag_by(
        &mut h,
        &[(-200.0, 0.0), (-100.0, 0.0), (100.0, 0.0), (200.0, 0.0)],
    );
    assert_eq!(alpha_at(&h, outside), 255);
    pick_tool(&mut h, Tool::SelectRect);
    drag_rect(&mut h, (-60.0, -80.0), (60.0, 80.0));
    pick_tool(&mut h, Tool::Eraser);
    drag_by(&mut h, &[(-200.0, 0.0), (0.0, 0.0), (200.0, 0.0)]);
    assert_eq!(alpha_at(&h, inside), 0, "内側は消える");
    assert_eq!(alpha_at(&h, outside), 255, "外は残る");
}

#[test]
fn selection_tools_do_not_paint_on_the_canvas_or_the_cube() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::Lasso);
    let c = at(&h, 0.0, 0.0);
    drag(
        &mut h,
        &[
            c,
            offset(c, 40.0, 0.0),
            offset(c, 40.0, 40.0),
            offset(c, 0.0, 40.0),
        ],
    );
    assert!(st(&h).doc.selection().is_some());
    assert_eq!(
        alpha_at(&h, offset(c, 20.0, 20.0)),
        0,
        "選択のツールは描かない"
    );
    // 3D のビューでも画素は描かない（面に描くのはブラシ・消しゴムだけ。引いた形は選択範囲になるので、取り消し 1 回）
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    let before = steps(&h);
    let a = rect.center();
    drag(
        &mut h,
        &[
            a,
            offset(a, 30.0, 10.0),
            offset(a, 60.0, 20.0),
            offset(a, 20.0, 50.0),
        ],
    );
    assert_eq!(
        steps(&h),
        before + 1,
        "3D の選択のツールは選択範囲を作る（取り消し 1 回）: {}",
        st(&h).message
    );
    assert!(!st(&h).doc.has_active_stroke());
    let doc = &st(&h).doc;
    assert!(
        doc.composite(doc.bounds())
            .unwrap()
            .chunks(4)
            .all(|p| p[3] == 0),
        "3D では選択のツールで描かない"
    );
    assert!(!st(&h).message.is_empty());
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    pick_tool(&mut h, Tool::Brush);
    drag(&mut h, &[a, offset(a, 30.0, 10.0), offset(a, 60.0, 20.0)]);
    assert!(
        steps(&h) > before,
        "ブラシなら同じ操作で描ける: {}",
        st(&h).message
    );
}

fn pen_at(pos: Pos2, contact: bool) -> PenSample {
    PenSample {
        pos: [pos.x, pos.y],
        pressure: 0.5,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 3,
        time_ms: 0,
    }
}

#[test]
fn a_selection_tool_pen_presses_drags_and_releases_like_the_mouse() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    for (p, contact) in [
        (at(&h, -60.0, -40.0), true),
        (at(&h, 0.0, 0.0), true),
        (at(&h, 60.0, 40.0), true),
        (at(&h, 60.0, 40.0), false),
    ] {
        h.state().pen().push(pen_at(p, contact));
        h.step();
    }
    h.run();
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    assert_eq!(amount_at(&h, at(&h, 100.0, 0.0)), 0);
    assert!(st(&h).sel.pen_down.is_none());
    assert_eq!(steps(&h), 1);
}

#[test]
fn holding_y_for_the_stencil_keeps_the_selection_tools_from_starting_a_shape() {
    use yolu_app::stencil::StencilOp;
    let dir = temp_dir("stencil-t");
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut()
        .state
        .apply(Action::Stencil(StencilOp::Load(half_open_png(&dir))));
    h.run();
    assert!(st(&h).stencil.image.is_some(), "{}", st(&h).message);
    pick_tool(&mut h, Tool::SelectRect);
    h.event(Event::Key {
        key: Key::Y,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    // マウス: Y を押したままのドラッグはステンシルを動かし、選択の形は始まらない
    let path = [at(&h, -60.0, -40.0), at(&h, 0.0, 0.0), at(&h, 60.0, 40.0)];
    drag(&mut h, &path);
    assert!(
        st(&h).doc.selection().is_none(),
        "Y を押したままのドラッグで選択ができた"
    );
    assert!(st(&h).sel.drag.is_none());
    // ペン: 触れても選択の形は始まらない
    for (p, contact) in [
        (at(&h, -60.0, -40.0), true),
        (at(&h, 60.0, 40.0), true),
        (at(&h, 60.0, 40.0), false),
    ] {
        h.state().pen().push(pen_at(p, contact));
        h.step();
    }
    h.run();
    assert!(
        st(&h).doc.selection().is_none(),
        "Y を押したままのペンで選択ができた"
    );
    assert!(st(&h).sel.pen_down.is_none() && st(&h).sel.drag.is_none());
    assert_eq!(steps(&h), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn switching_to_a_selection_tool_during_a_pen_stroke_still_ends_the_stroke_when_the_pen_lifts() {
    use yolu_app::state::StrokeSource;
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::Brush);
    for p in [at(&h, -60.0, 0.0), at(&h, -20.0, 10.0)] {
        h.state().pen().push(pen_at(p, true));
        h.step();
    }
    assert_eq!(st(&h).canvas.stroke, Some(StrokeSource::Pen(3)));
    // ペンを付けたまま M（長方形選択）。描いている間もツールは替えられる
    key(&h, Key::M, Modifiers::NONE);
    h.step();
    assert_eq!(st(&h).tool, Tool::SelectRect);
    // 動かしている間はストロークの続き（選択の形は始まらない）
    h.state().pen().push(pen_at(at(&h, 20.0, 10.0), true));
    h.step();
    assert_eq!(st(&h).canvas.stroke, Some(StrokeSource::Pen(3)));
    assert!(st(&h).sel.drag.is_none() && st(&h).sel.pen_down.is_none());
    // 離すとストロークが確定する（取り残さない）。マウスも使え、編集・Undo も断られない
    h.state().pen().push(pen_at(at(&h, 20.0, 10.0), false));
    h.step();
    h.run();
    let s = st(&h);
    assert!(s.canvas.stroke.is_none(), "ペンのストロークが残った");
    assert!(!s.is_stroking());
    assert_eq!(steps(&h), 1, "描いた分は 1 回の Undo");
    assert!(s.doc.selection().is_none());
    assert!(s.sel.drag.is_none() && s.sel.pen_down.is_none());
    // 続けて、選択のツールでペンの選択ができる
    for (p, contact) in [
        (at(&h, -40.0, -40.0), true),
        (at(&h, 40.0, 40.0), true),
        (at(&h, 40.0, 40.0), false),
    ] {
        h.state().pen().push(pen_at(p, contact));
        h.step();
    }
    h.run();
    assert!(selected(&h, at(&h, 0.0, 0.0)));
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none(), "選択の Undo");
    undo(&mut h);
    assert_eq!(steps(&h), 0, "描いた分の Undo");
    // マウスのストロークは、最中にツールを替えても従来どおり離して終わる
    pick_tool(&mut h, Tool::Brush);
    let a = at(&h, -50.0, -50.0);
    h.event(Event::PointerMoved(a));
    h.event(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    let mid = at(&h, 0.0, 0.0);
    move_to(&h, mid);
    h.step();
    key(&h, Key::L, Modifiers::NONE);
    h.step();
    let end = at(&h, 50.0, 0.0);
    move_to(&h, end);
    h.step();
    h.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.run();
    assert!(!st(&h).is_stroking());
    assert_eq!(st(&h).tool, Tool::Lasso);
    assert_eq!(steps(&h), 1);
}

// ───────── 対称 ─────────

#[test]
fn the_special_snap_switch_turns_the_symmetry_ruler_off_and_on() {
    let mut h = app(1000.0, 640.0, 512);
    let r = canvas_rect(&h);
    let view = st(&h).view.view(r, 512, 512);
    let s = |x: f64, y: f64| view.to_screen(x, y);
    symmetry(&mut h, |s| {
        common::rulers::vertical(s, 256.0);
    });
    assert!(st(&h).rulers.snap_special, "特殊定規を作ると入る");
    let dab = |h: &mut H, p: Pos2| drag(h, &[p, offset(p, 0.5, 0.0)]);
    dab(&mut h, s(100.0, 400.0));
    assert!(alpha_at(&h, s(412.0, 400.0)) > 0, "入っていれば映る");
    undo(&mut h);
    special_snap(&mut h, false);
    dab(&mut h, s(100.0, 400.0));
    assert!(alpha_at(&h, s(100.0, 400.0)) > 0);
    assert_eq!(alpha_at(&h, s(412.0, 400.0)), 0, "切ると映さない");
    undo(&mut h);
    // 「表示」のメニューの項目でも切り替わる（チェックが付く）
    let entries = yolu_app::shell::menu_entries(st(&h), 5);
    let item = entries
        .iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, check, .. }
                if label == "特殊定規にスナップ" =>
            {
                Some(*check)
            }
            _ => None,
        })
        .expect("表示のメニューの特殊定規にスナップ");
    assert_eq!(
        item,
        yolu_app::ui::menu::Check::None,
        "切ったのでチェックは外れる"
    );
    special_snap(&mut h, true);
    dab(&mut h, s(100.0, 400.0));
    assert!(alpha_at(&h, s(412.0, 400.0)) > 0);
}

#[test]
fn symmetric_strokes_mirror_across_the_axes_and_the_radial_copies() {
    let mut h = app(1000.0, 640.0, 512);
    let r = canvas_rect(&h);
    let view = st(&h).view.view(r, 512, 512);
    let s = |x: f64, y: f64| view.to_screen(x, y);
    symmetry(&mut h, |s| {
        common::rulers::vertical(s, 256.0);
    });
    let dab = |h: &mut H, p: Pos2| drag(h, &[p, offset(p, 0.5, 0.0)]);
    dab(&mut h, s(100.0, 400.0));
    assert!(alpha_at(&h, s(100.0, 400.0)) > 0);
    assert!(
        alpha_at(&h, s(412.0, 400.0)) > 0,
        "縦の軸（x = 256）で左右に映る"
    );
    assert_eq!(alpha_at(&h, s(100.0, 112.0)), 0, "横には映らない");
    undo(&mut h);
    symmetry(&mut h, |s| {
        common::rulers::both(s, (256.0, 256.0));
    });
    dab(&mut h, s(100.0, 400.0));
    for (x, y) in [
        (100.0, 400.0),
        (412.0, 400.0),
        (100.0, 112.0),
        (412.0, 112.0),
    ] {
        assert!(alpha_at(&h, s(x, y)) > 0, "{x},{y}");
    }
    undo(&mut h);
    // 中心をずらす: x = 128 で映る
    symmetry(&mut h, |s| {
        common::rulers::vertical(s, 128.0);
    });
    dab(&mut h, s(60.0, 300.0));
    assert!(alpha_at(&h, s(196.0, 300.0)) > 0);
    undo(&mut h);
    // 回転対称 4 つ: 中心のまわりに 90° ずつ
    symmetry(&mut h, |s| {
        common::rulers::radial(s, (256.0, 256.0), 4);
    });
    dab(&mut h, s(356.0, 256.0)); // 中心から右へ 100
    let copies = [
        (356.0, 256.0),
        (256.0, 356.0),
        (156.0, 256.0),
        (256.0, 156.0),
    ];
    for (x, y) in copies {
        assert!(alpha_at(&h, s(x, y)) > 0, "{x},{y}");
    }
    // 1 回の Undo で映した分も戻る
    undo(&mut h);
    for (x, y) in copies {
        assert_eq!(alpha_at(&h, s(x, y)), 0);
    }
    // 線対称 6 本: 鏡 3 枚（0°・60°・120° の軸）と回転 3 つ（120° ごと）で 6 つに映る。60° だけ回した点には映らない
    symmetry(&mut h, |s| {
        common::rulers::symmetry_2d(s, (256.0, 256.0), (1.0, 0.0), 6, true);
    });
    dab(&mut h, s(356.0, 266.0));
    let (dx, dy) = (100.0, 10.0);
    let image = |turn: f64, flip: bool| {
        let y = if flip { -dy } else { dy };
        let (sn, c) = turn.sin_cos();
        s(256.0 + dx * c - y * sn, 256.0 + dx * sn + y * c)
    };
    for k in 0..3 {
        for flip in [false, true] {
            let turn = std::f64::consts::TAU * f64::from(k) / 3.0;
            assert!(alpha_at(&h, image(turn, flip)) > 0, "{k} {flip}");
        }
    }
    assert_eq!(
        alpha_at(&h, image(std::f64::consts::TAU / 6.0, false)),
        0,
        "60° だけ回した点には映らない"
    );
    undo(&mut h);
    // 対称定規を隠せば映さない
    let layer = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::Ruler(RulerAction::SetAllVisible {
            owner: layer,
            visible: false,
        }));
    dab(&mut h, s(356.0, 256.0));
    assert_eq!(alpha_at(&h, s(156.0, 256.0)), 0);
}

#[test]
fn smudge_and_clone_cannot_be_combined_with_symmetry() {
    let mut h = app(1000.0, 640.0, 256);
    symmetry(&mut h, |s| {
        common::rulers::vertical(s, 128.0);
    });
    h.state_mut().state.m2.brush.effect = BrushEffect::Smudge { strength: 0.5 };
    let p = at(&h, 0.0, 0.0);
    drag(&mut h, &[p, offset(p, 20.0, 0.0)]);
    assert_eq!(
        steps(&h),
        1,
        "断ったので何も描かない（定規を置いた 1 段だけ）"
    );
    assert!(
        st(&h).message.contains("対称"),
        "理由が出る: {}",
        st(&h).message
    );
    assert!(st(&h).canvas.stroke.is_none() && !st(&h).doc.has_active_stroke());
    // 効果をペイントに戻せば、対称のまま描ける
    h.state_mut().state.m2.brush.effect = BrushEffect::Paint;
    drag(&mut h, &[p, offset(p, 20.0, 0.0)]);
    assert_eq!(steps(&h), 2);
}

#[test]
fn the_axes_of_a_radial_symmetry_ruler_are_drawn_and_a_hidden_ruler_draws_and_copies_nothing() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    symmetry(&mut h, |s| {
        common::rulers::radial(s, (0.4 * 512.0, 0.55 * 512.0), 6);
    });
    h.snapshot("rulers_symmetry_radial_axes");
    // 隠すと線も消え、写しも効かない
    let layer = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::Ruler(RulerAction::SetAllVisible {
            owner: layer,
            visible: false,
        }));
    h.run();
    assert!(st(&h)
        .rulers_shown(yolu_app::rulers::Place::Canvas)
        .is_empty());
    assert!(!st(&h).canvas_symmetry().enabled());
}

/// 左下 `limit` 画素四方に、1 画素おきの孤立した点（選ばれた画素ごとに縁が 4 本。つながらない）。
fn dotted_selection(doc: &yolu_app::engine::Document, limit: u32) -> SelectionMask {
    let (size, ts) = (doc.width(), doc.tile_size());
    let mut tiles = Vec::new();
    for ty in 0..size.div_ceil(ts) {
        for tx in 0..size.div_ceil(ts) {
            let mut a = vec![0u8; (ts * ts) as usize];
            for ly in 0..ts {
                for lx in 0..ts {
                    let (x, y) = (tx * ts + lx, ty * ts + ly);
                    if x.is_multiple_of(2) && y.is_multiple_of(2) && x < limit && y < limit {
                        a[(ly * ts + lx) as usize] = 255;
                    }
                }
            }
            tiles.push((TileCoord::new(tx, ty), a));
        }
    }
    SelectionMask::from_amount_tiles(size, size, ts, tiles).unwrap()
}

#[test]
fn a_selection_edge_cut_at_the_segment_limit_is_flagged_on_the_canvas() {
    // 768 × 768 の孤立した点: 縁が 59 万本で、つなげたあとの上限（40 万本）で打ち切る
    let mut h = app(1000.0, 640.0, 768);
    h.state_mut().state.sel.animate = false;
    assert!(!st(&h).sel.edge_partial);
    let mask = dotted_selection(&st(&h).doc, 768);
    h.state_mut().state.doc.set_selection(Some(mask)).unwrap();
    h.run();
    assert!(st(&h).sel.edge_partial, "縁を一部しか描かないのに印が無い");
    // 単純な選択範囲（縁が 4 本）では出ない。解除しても出ない
    run(&mut h.state_mut().state, SelEdit::All);
    h.run();
    assert!(!st(&h).sel.edge_partial);
    let mask = dotted_selection(&st(&h).doc, 768);
    h.state_mut().state.doc.set_selection(Some(mask)).unwrap();
    h.run();
    assert!(st(&h).sel.edge_partial);
    run(&mut h.state_mut().state, SelEdit::Clear);
    h.run();
    assert!(!st(&h).sel.edge_partial);
}

#[test]
fn a_selection_edge_past_the_drawn_limit_is_flagged_even_when_the_edge_itself_is_complete() {
    // 256 × 256 の孤立した点 6.5 万本: 縁は上限以内だが、1 画面に描く上限（4 万本）を超える
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut().state.sel.animate = false;
    let mask = dotted_selection(&st(&h).doc, 256);
    h.state_mut().state.doc.set_selection(Some(mask)).unwrap();
    h.run();
    assert!(st(&h).sel.edge_partial);
    // 拡大して画面にかかる線分が上限以内になれば、印は消える
    let rect = canvas_rect(&h);
    h.state_mut()
        .state
        .view
        .zoom_to(16.0, Some(rect.center()), rect);
    h.run();
    assert!(
        !st(&h).sel.edge_partial,
        "見える線分が少なくなったら印を消す"
    );
}

// ───────── Undo・描いている間・拒否（画面を描かない） ─────────

fn doc_state(size: u32) -> AppState {
    AppState::new(size, size)
}

fn run(s: &mut AppState, e: SelEdit) {
    s.apply(Action::Sel(SelAction::Edit(e)));
}

fn rect(mode: SelectionCombine, x0: i64, y0: i64, x1: i64, y1: i64) -> SelEdit {
    SelEdit::Rect {
        x0,
        y0,
        x1,
        y1,
        mode,
    }
}

#[test]
fn headless_each_selection_edit_is_one_undo_and_a_no_change_adds_none() {
    let mut s = doc_state(64);
    let all = [
        rect(SelectionCombine::Replace, 8, 8, 40, 40),
        SelEdit::Ellipse {
            cx: 32.0,
            cy: 32.0,
            rx: 20.0,
            ry: 12.0,
            mode: SelectionCombine::Add,
        },
        SelEdit::Polygon {
            points: vec![(5.0, 5.0), (60.0, 8.0), (30.0, 58.0)],
            mode: SelectionCombine::Intersect,
        },
        SelEdit::Invert,
        SelEdit::Modify {
            kind: ModifyKind::Grow,
            radius: 3,
            edge_lock: false,
        },
        SelEdit::Modify {
            kind: ModifyKind::Shrink,
            radius: 2,
            edge_lock: true,
        },
        SelEdit::Modify {
            kind: ModifyKind::Border,
            radius: 2,
            edge_lock: false,
        },
        SelEdit::Modify {
            kind: ModifyKind::Feather,
            radius: 4,
            edge_lock: false,
        },
        SelEdit::Modify {
            kind: ModifyKind::Sharpen,
            radius: 0,
            edge_lock: false,
        },
    ];
    let mut expected = s.doc.undo_count();
    for e in &all {
        let before = s.doc.selection().cloned();
        run(&mut s, e.clone());
        expected += 1;
        assert_eq!(s.doc.undo_count(), expected, "{e:?}: {}", s.message);
        assert_ne!(s.doc.selection(), before.as_ref(), "{e:?}");
        assert!(s.modified);
    }
    // 全部を取り消すと選択なしに戻る
    for _ in 0..all.len() {
        s.apply(Action::Undo);
    }
    assert!(s.doc.selection().is_none());
    // 変わらない操作は段を積まない
    let top = s.doc.undo_count();
    run(&mut s, SelEdit::Clear);
    run(&mut s, SelEdit::Invert);
    assert_eq!(s.doc.undo_count(), top, "選択なしの解除・反転は何もしない");
    run(&mut s, SelEdit::All);
    run(&mut s, SelEdit::All);
    assert_eq!(s.doc.undo_count(), top + 1, "すでに全部なら積まない");
    run(&mut s, rect(SelectionCombine::Add, 3, 3, 9, 9));
    assert_eq!(s.doc.undo_count(), top + 1, "全部に足しても変わらない");
    run(&mut s, rect(SelectionCombine::Subtract, 0, 0, 64, 64));
    assert!(s.doc.selection().is_none(), "全部を引くと何も残らない");
    assert_eq!(s.doc.undo_count(), top + 2);
    run(&mut s, rect(SelectionCombine::Subtract, 0, 0, 64, 64));
    assert_eq!(s.doc.undo_count(), top + 2, "選択なしから引いても積まない");
}

#[test]
fn headless_selection_edits_are_refused_while_stroking_and_on_read_only_sets() {
    let mut s = doc_state(64);
    let id = s.selected_layer.unwrap();
    let stroke = s.begin_canvas_stroke(id, false, None).unwrap();
    let before = s.doc.undo_count();
    for e in [
        SelEdit::All,
        rect(SelectionCombine::Replace, 0, 0, 10, 10),
        SelEdit::Clear,
    ] {
        run(&mut s, e);
        assert!(s.doc.selection().is_none() && s.doc.undo_count() == before);
        assert!(s.message.contains("描いている間"), "{}", s.message);
    }
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::OpenAmount(
        ModifyKind::Grow,
    ))));
    assert!(s.sel.dialog.is_none());
    // 描いている間は定規を置けない（ストロークに固めた対称と食い違わせない）
    let ruler = Ruler::canvas(
        yolu_core::RulerId(1),
        RulerKind::Symmetry,
        yolu_core::glam::DVec2::new(5.0, 5.0),
        yolu_core::glam::DVec2::new(5.0, 9.0),
    );
    s.apply(Action::Ruler(RulerAction::Create(ruler)));
    assert_eq!(common::rulers::total(&s), 0, "描いている間は定規を置かない");
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.doc.cancel_stroke(stroke);
    // 読むだけのセット
    s.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    run(&mut s, SelEdit::All);
    assert!(s.doc.selection().is_none());
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::OpenAmount(
        ModifyKind::Grow,
    ))));
    assert!(s.sel.dialog.is_none());
}

#[test]
fn headless_selection_tools_do_not_start_while_a_stroke_is_running_or_the_set_is_read_only() {
    use yolu_app::selection::canvas::{press, release};
    use yolu_app::state::StrokeSource;
    let mut s = doc_state(64);
    let view = s.view.view(
        egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(256.0, 256.0)),
        64,
        64,
    );
    let id = s.selected_layer.unwrap();
    let stroke = s.begin_canvas_stroke(id, false, None).unwrap();
    s.stroke = Some(stroke);
    s.canvas.stroke = Some(StrokeSource::Mouse);
    for tool in [Tool::SelectRect, Tool::Lasso, Tool::Polygon, Tool::Wand] {
        s.tool = tool;
        press(
            &mut s,
            &view,
            Pos2::new(100.0, 100.0),
            StrokeSource::Mouse,
            Modifiers::NONE,
            0.0,
        );
        assert!(s.sel.drag.is_none() && s.sel.polygon.is_empty(), "{tool:?}");
        assert!(s.doc.selection().is_none());
    }
    // 終えたら始められる。離した点が離れていれば形になる
    s.canvas.stroke = None;
    if let Some(stroke) = s.stroke.take() {
        s.doc.cancel_stroke(stroke);
    }
    s.tool = Tool::SelectRect;
    press(
        &mut s,
        &view,
        Pos2::new(40.0, 40.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
        0.0,
    );
    assert!(s.sel.drag.is_some());
    release(
        &mut s,
        &view,
        Pos2::new(160.0, 160.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
    );
    assert!(s.doc.selection().is_some(), "{}", s.message);
    // 読むだけのセットでは始めない
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    s.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    press(
        &mut s,
        &view,
        Pos2::new(40.0, 40.0),
        StrokeSource::Mouse,
        Modifiers::NONE,
        0.0,
    );
    assert!(s.sel.drag.is_none());
    assert!(s.message.contains("読むだけ"), "{}", s.message);
}

#[test]
fn headless_refused_edits_change_nothing_and_say_why() {
    // 作業の予算を超えるぼかし: 8192² の全面の選択範囲を半径 200 でぼかす（1 GiB の予算を超える）
    let mut s = doc_state(8192);
    run(&mut s, SelEdit::All);
    let selection = s.doc.selection().cloned();
    let steps = s.doc.undo_count();
    run(
        &mut s,
        SelEdit::Modify {
            kind: ModifyKind::Feather,
            radius: 200,
            edge_lock: false,
        },
    );
    assert_eq!(s.doc.selection(), selection.as_ref());
    assert_eq!(s.doc.undo_count(), steps);
    let budget = s.lang.core_error(&CoreError::WorkingBudgetExceeded);
    assert!(s.message.contains(&budget), "{}", s.message);
    // 型の拒否: 点が多すぎる多角形・有限でない楕円・キャンバスの外の種
    let mut s = doc_state(64);
    for e in [
        SelEdit::Polygon {
            points: vec![(1.0, 1.0); 100_001],
            mode: SelectionCombine::Replace,
        },
        SelEdit::Ellipse {
            cx: f64::NAN,
            cy: 0.0,
            rx: 1.0,
            ry: 1.0,
            mode: SelectionCombine::Replace,
        },
        SelEdit::Wand {
            seeds: vec![(64, 0)],
            mode: SelectionCombine::Replace,
        },
    ] {
        s.message.clear();
        run(&mut s, e.clone());
        assert!(
            s.doc.selection().is_none() && s.doc.undo_count() == 0,
            "{e:?}"
        );
        assert!(!s.message.is_empty(), "{e:?}: 理由が出る");
    }
    // 半径は上限（200）に丸める
    run(&mut s, rect(SelectionCombine::Replace, 20, 20, 30, 30));
    run(
        &mut s,
        SelEdit::Modify {
            kind: ModifyKind::Grow,
            radius: 100_000,
            edge_lock: false,
        },
    );
    assert!(
        s.doc.selection().unwrap().amount(0, 0) == 255,
        "{}",
        s.message
    );
}

/// 拒否の文は画面の言語で出る（英語の画面に日本語の文を出さない）。文書・選択範囲・Undo の段は変わらない。
#[test]
fn headless_refusals_are_told_in_the_language_and_change_nothing() {
    // 型の拒否: 点が多すぎる多角形・有限でない楕円・キャンバスの外の種（既存の選択範囲があっても残る）
    let refusals = [
        (
            SelEdit::Polygon {
                points: vec![(1.0, 1.0); 100_001],
                mode: SelectionCombine::Replace,
            },
            "多角形の点が多すぎる",
            "Too many polygon points",
        ),
        (
            SelEdit::Ellipse {
                cx: f64::NAN,
                cy: 0.0,
                rx: 1.0,
                ry: 1.0,
                mode: SelectionCombine::Replace,
            },
            "値が範囲外",
            "Invalid value",
        ),
        (
            SelEdit::Wand {
                seeds: vec![(64, 0)],
                mode: SelectionCombine::Replace,
            },
            "種がキャンバスの外",
            "Seed outside canvas",
        ),
    ];
    for lang in [Lang::Ja, Lang::En] {
        let mut s = doc_state(64);
        s.lang = lang;
        run(&mut s, rect(SelectionCombine::Replace, 10, 10, 20, 20));
        let selection = s.doc.selection().cloned();
        assert!(selection.is_some());
        let steps = s.doc.undo_count();
        for (edit, ja, en) in &refusals {
            s.message.clear();
            run(&mut s, edit.clone());
            let m = s.message.clone();
            assert_eq!(s.doc.selection(), selection.as_ref(), "{lang:?} {m}");
            assert_eq!(s.doc.undo_count(), steps, "{lang:?} {m}");
            match lang {
                Lang::En => {
                    assert!(m.is_ascii(), "英語の画面に日本語が混じる: {m:?}");
                    assert!(m.contains(en), "{m:?}");
                }
                Lang::Ja => assert!(m.contains(ja), "{m:?}"),
            }
        }
    }
    // 作業の予算を超えるぼかし（8192² の全面を半径 200）
    let mut s = doc_state(8192);
    run(&mut s, SelEdit::All);
    let selection = s.doc.selection().cloned();
    let steps = s.doc.undo_count();
    for lang in [Lang::Ja, Lang::En] {
        s.lang = lang;
        s.message.clear();
        run(
            &mut s,
            SelEdit::Modify {
                kind: ModifyKind::Feather,
                radius: 200,
                edge_lock: false,
            },
        );
        assert_eq!(s.doc.selection(), selection.as_ref());
        assert_eq!(s.doc.undo_count(), steps);
        let expected = lang.core_error(&CoreError::WorkingBudgetExceeded);
        assert!(s.message.contains(&expected), "{lang:?} {}", s.message);
        assert_eq!(s.message.is_ascii(), lang == Lang::En, "{}", s.message);
    }
    assert_eq!(
        Lang::En.core_error(&CoreError::WorkingBudgetExceeded),
        "Working memory budget exceeded"
    );
}

/// .ylp の選択範囲の受け渡しの失敗も画面の言語の文で返る。大きさの違う選択範囲は、文書を選択なしのまま、書き込みも断る。
#[test]
fn headless_selection_file_failures_are_told_in_the_language_and_touch_nothing() {
    use yolu_app::selection::io::{restore_into, write_into};
    use yolu_io::{SaveTarget, Selection};
    let dir = temp_dir("io-lang");
    let path = dir.join("one.ylp");
    let mut saved = doc_state(64);
    saved.apply(Action::SaveProjectAs(path.clone()));
    assert!(
        saved.message.starts_with("保存しました"),
        "{}",
        saved.message
    );
    // 32² の文書の選択範囲（64² の文書とは大きさが違う）
    let (mut small, _) = yolu_app::state::blank_document(32, 32);
    small
        .set_selection(Some(SelectionMask::rectangle(&small, 4, 4, 20, 20)))
        .unwrap();
    let small_mask = small.selection().cloned().unwrap();
    let stored = Selection::from_core(&small_mask).unwrap();
    for lang in [Lang::Ja, Lang::En] {
        let (ja, en) = (
            ("選択範囲を戻せません", "選択範囲の大きさがキャンバスと違う"),
            (
                "Cannot restore the selection",
                "Selection size does not match canvas",
            ),
        );
        // 読み込み: 大きさが違えば選択なしのまま理由を返す。同じ大きさなら戻る（Undo の段は増えない）
        let (mut doc, _) = yolu_app::state::blank_document(64, 64);
        let err = restore_into(&mut doc, Some(&stored), lang).unwrap_err();
        assert!(doc.selection().is_none() && !doc.can_undo(), "{lang:?}");
        match lang {
            Lang::En => {
                assert!(err.is_ascii(), "英語の画面に日本語が混じる: {err:?}");
                assert!(err.contains(en.0) && err.contains(en.1), "{err:?}");
            }
            Lang::Ja => assert!(err.contains(ja.0) && err.contains(ja.1), "{err:?}"),
        }
        let (mut same, _) = yolu_app::state::blank_document(32, 32);
        restore_into(&mut same, Some(&stored), lang).unwrap();
        assert_eq!(same.selection(), Some(&small_mask));
        assert!(!same.can_undo());
        restore_into(&mut same, None, lang).unwrap();

        // 書き込み: 文書と大きさの違う選択範囲は、プロジェクトの検証が断る。理由は言語の文で、元のプロジェクトは変わらない
        let (project, _target) = SaveTarget::open(&path).unwrap();
        let id = project.sets()[0].id.clone();
        let err = write_into(project.clone(), &[(id.as_str(), Some(&small_mask))], lang)
            .map(|_| ())
            .unwrap_err();
        match lang {
            Lang::En => {
                assert!(err.is_ascii(), "英語の画面に日本語が混じる: {err:?}");
                assert!(err.starts_with("Cannot write the selection"), "{err:?}");
            }
            Lang::Ja => assert!(err.starts_with("選択範囲を書けません"), "{err:?}"),
        }
        assert!(project.sets()[0].selection.is_none());
        // 同じ大きさなら書ける
        let (mut doc64, _) = yolu_app::state::blank_document(64, 64);
        doc64
            .set_selection(Some(SelectionMask::rectangle(&doc64, 4, 4, 20, 20)))
            .unwrap();
        let mask64 = doc64.selection().cloned().unwrap();
        let written = write_into(project, &[(id.as_str(), Some(&mask64))], lang).unwrap();
        assert_eq!(
            written.sets()[0].selection,
            Some(Selection::from_core(&mask64).unwrap())
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_edge_lock_keeps_the_selection_at_the_canvas_edge_when_shrinking() {
    let mut s = doc_state(64);
    run(&mut s, SelEdit::All);
    let modify = |edge_lock| SelEdit::Modify {
        kind: ModifyKind::Shrink,
        radius: 5,
        edge_lock,
    };
    run(&mut s, modify(false));
    assert_eq!(
        s.doc.selection().unwrap().amount(0, 32),
        0,
        "縮むとキャンバスの端が外れる"
    );
    assert_eq!(s.doc.selection().unwrap().amount(32, 32), 255);
    s.apply(Action::Undo);
    run(&mut s, modify(true));
    assert_eq!(
        s.doc.selection().unwrap().amount(0, 32),
        255,
        "端を固定するとキャンバスの端は縮まない"
    );
    assert_eq!(s.doc.undo_count(), 1, "全面のままなので段を積まない");
}

#[test]
fn headless_wand_reads_the_selected_layer_or_the_composite() {
    let mut s = doc_state(256);
    paint_rect(&mut s, 0, 0, 128, 128, [255, 0, 0, 255]);
    s.apply(Action::NewLayer);
    paint_rect(&mut s, 128, 0, 256, 128, [0, 0, 255, 255]);
    s.sel.tolerance = 0;
    let wand = |x| SelEdit::Wand {
        seeds: vec![(x, 10)],
        mode: SelectionCombine::Replace,
    };
    // 選んでいる上のレイヤー（右下だけ青）: 左下は透明の画素なので、透明が選ばれる（青はつながらない別の色）
    run(&mut s, wand(10));
    assert_eq!(s.doc.selection().unwrap().amount(10, 10), 255);
    assert_eq!(s.doc.selection().unwrap().amount(200, 10), 0);
    // 全レイヤー: 合成（左下が赤・右下が青）の赤だけ
    s.sel.all_layers = true;
    run(&mut s, wand(10));
    assert_eq!(s.doc.selection().unwrap().amount(10, 10), 255);
    assert_eq!(s.doc.selection().unwrap().amount(130, 10), 0);
    assert_eq!(
        s.doc.selection().unwrap().amount(10, 200),
        0,
        "透明は赤と違う"
    );
}

#[test]
fn headless_fill_stays_inside_the_selection() {
    let mut s = doc_state(64);
    let id = s.selected_layer.unwrap();
    run(&mut s, rect(SelectionCombine::Replace, 10, 10, 30, 30));
    let changed = s
        .doc
        .fill(
            id,
            Channel::Color,
            Rgba8::new(255, 0, 0, 255),
            1.0,
            None,
            false,
        )
        .unwrap();
    assert!(changed);
    let px = |s: &AppState, x, y| s.doc.composite_pixel(Channel::Color, x, y).unwrap();
    assert_eq!(px(&s, 20, 20), Rgba8::new(255, 0, 0, 255));
    assert_eq!(px(&s, 5, 5).a, 0, "外は塗らない");
    s.apply(Action::Undo); // 塗りつぶしを戻す
    assert_eq!(px(&s, 20, 20).a, 0);
    assert!(s.doc.selection().is_some(), "選択範囲は残る");
}

// ───────── 対称のブラシ（画面を描かない） ─────────

#[test]
fn headless_symmetry_reaches_the_brush_and_follows_the_document_size() {
    let mut s = doc_state(64);
    common::rulers::vertical(&mut s, 16.0);
    let id = s.selected_layer.unwrap();
    let mut stroke = s.begin_canvas_stroke(id, false, None).unwrap();
    let sample =
        yolu_app::engine::BrushSample::new(6.5, 20.5, 1.0, 0.0, yolu_app::engine::DVec2::ZERO)
            .unwrap();
    stroke.add_sample(&mut s.doc, sample).unwrap();
    s.doc.end_stroke(stroke).unwrap();
    let a = |x, y| s.doc.composite_pixel(Channel::Color, x, y).unwrap().a;
    assert!(a(6, 20) > 0);
    assert!(a(25, 20) > 0, "x = 16 の軸で映る（64 × 0.25）");
    assert_eq!(a(57, 20), 0);
    // 基本のブラシ（3D の面のストロークが使う）は対称を持たない
    assert!(!s.stroke_brush(false).symmetry.enabled());
    // 対称定規の中心は文書の画素で持つ。画像のサイズを変えると、定規も一緒に写る（取り消しで戻る）
    let c = s.canvas_symmetry();
    assert_eq!((c.center.x, c.center.y), (16.0, 0.0));
    s.doc
        .resize_image(512, 512, yolu_app::engine::CanvasResampling::Nearest)
        .unwrap();
    let c = s.canvas_symmetry();
    assert_eq!((c.center.x, c.center.y), (128.0, 0.0));
    assert!(c.enabled(), "サイズを変えたあとも効く");
    s.doc.undo().unwrap();
    let c = s.canvas_symmetry();
    assert_eq!((c.center.x, c.center.y), (16.0, 0.0));
}

#[test]
fn headless_symmetry_is_refused_for_smudge_and_clone_with_a_reason_and_no_stroke() {
    let mut s = doc_state(64);
    common::rulers::both(&mut s, (32.0, 32.0));
    let id = s.selected_layer.unwrap();
    for effect in [
        BrushEffect::Smudge { strength: 0.5 },
        BrushEffect::Clone {
            offset: yolu_app::engine::DVec2::new(10.0, 0.0),
        },
    ] {
        s.m2.brush.effect = effect;
        let Err(err) = s.begin_canvas_stroke(id, false, None) else {
            panic!("断るはず");
        };
        assert!(err.to_string().contains("対称"), "{err}");
        assert!(!s.doc.has_active_stroke());
        assert!(s.sel.stroke_symmetry.is_none());
    }
}

/// 左半分が白、右半分が黒の画像（ステンシルは量として読み、左半分だけが通る）。
fn half_open_png(dir: &std::path::Path) -> std::path::PathBuf {
    let img = image::RgbaImage::from_fn(64, 64, |x, _| {
        let v = if x < 32 { 255 } else { 0 };
        image::Rgba([v, v, v, 255])
    });
    let path = dir.join("half.png");
    img.save(&path).unwrap();
    path
}

/// 選択範囲とステンシルが別々の所で塞がり、対称の映しがその画素の選択範囲とステンシルを読むことを確かめる。
/// 選択範囲は x ∈ [16, 56)、ステンシルは x < 32 だけが開く。x < 16 は選択だけが塞ぎ、32 ≤ x < 56 はステンシルだけが塞ぐ。
/// 対称は縦の軸 x = 32 で、映した先は 64 - x（主の側が塞がれていても映した側だけが通る・その逆）。
#[test]
fn headless_a_canvas_stroke_obeys_the_selection_the_stencil_and_the_symmetry_together() {
    use yolu_app::engine::composite_pixel;
    use yolu_app::stencil::StencilOp;
    let dir = temp_dir("stencil-seam");
    let mut s = doc_state(64);
    s.brush.radius = 3.0;
    s.brush.hardness = 1.0;
    // 縁の画素の 0・255 を確かめるので、縁の帯の無い丸で描く
    s.brush.anti_alias = yolu_app::engine::AntiAlias::None;
    s.m2.random_seed = false;
    s.stencil.size = 1.0;
    s.apply(Action::Stencil(StencilOp::Load(half_open_png(&dir))));
    assert!(s.stencil.image.is_some(), "{}", s.message);
    run(&mut s, rect(SelectionCombine::Replace, 16, 0, 56, 64));
    let layer = s.selected_layer.unwrap();
    let canvas = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(64.0, 64.0));
    assert!(s.canvas_stencil(canvas).unwrap().is_some());
    // x0 から x1 まで y の高さを 2 画素おきに 1 本のストロークで描く（今の対称・選択範囲・ステンシルのまま）
    let line = |s: &mut AppState, y: f64, x0: f64, x1: f64| {
        let stencil = s.canvas_stencil(canvas).unwrap();
        let mut stroke = s.begin_canvas_stroke(layer, false, stencil).unwrap();
        let mut x = x0;
        while x <= x1 {
            stroke
                .add_point(&mut s.doc, x, y, 1.0, Default::default())
                .unwrap();
            x += 2.0;
        }
        s.doc.end_stroke(stroke).unwrap();
    };
    let alpha = |s: &AppState, x, y| composite_pixel(&s.doc, x, y)[3];

    // 対称なし: 選択だけが塞ぐ所・両方が通す所・ステンシルだけが塞ぐ所・両方が塞ぐ所を 1 本で通る
    line(&mut s, 20.0, 4.0, 60.0);
    assert_eq!(
        alpha(&s, 8, 20),
        0,
        "ステンシルが開いていても、選択の外（x < 16）は塗れない"
    );
    assert_eq!(alpha(&s, 15, 20), 0, "選択の縁の外の 1 画素");
    assert_eq!(alpha(&s, 16, 20), 255, "選択の縁の内の 1 画素");
    assert_eq!(alpha(&s, 24, 20), 255, "選択もステンシルも開いた所は塗れる");
    assert_eq!(
        alpha(&s, 40, 20),
        0,
        "選択の内側でも、ステンシルの閉じた所は塗れない"
    );
    assert_eq!(alpha(&s, 60, 20), 0, "選択もステンシルも閉じた所");
    s.apply(Action::Undo);
    assert_eq!(alpha(&s, 24, 20), 0);

    // 対称（縦の軸 x = 32）: 3 本の別々のストローク。主・映しの片方だけが通る
    common::rulers::vertical(&mut s, 32.0);
    // 主（x = 40〜44）はステンシルが塞ぎ、映し（x = 24〜20）は選択もステンシルも開いて塗れる
    line(&mut s, 12.0, 40.0, 44.0);
    assert_eq!(alpha(&s, 42, 12), 0, "主の側はステンシルが塞ぐ");
    assert_eq!(
        alpha(&s, 22, 12),
        255,
        "主が塞がれても、映した先で選択もステンシルも開いていれば塗れる"
    );
    // 主（x = 50〜54）は選択の内側でステンシルが塞ぎ、映し（x = 14〜10）はステンシルが開いても選択の外
    line(&mut s, 28.0, 50.0, 54.0);
    assert_eq!(alpha(&s, 52, 28), 0, "主の側はステンシルが塞ぐ");
    assert_eq!(
        alpha(&s, 16, 28),
        255,
        "映した先の選択の縁の内の 1 画素は塗れる"
    );
    assert_eq!(
        alpha(&s, 15, 28),
        0,
        "映した先の選択の縁の外の 1 画素は、主の側の選択が開いていても塗れない"
    );
    assert_eq!(
        alpha(&s, 12, 28),
        0,
        "映した先の選択の外は、ステンシルが開いていても塗れない"
    );
    // 主（x = 24〜28）は両方が開いて塗れ、映し（x = 40〜36）は選択の内側でステンシルだけが塞ぐ
    line(&mut s, 44.0, 24.0, 28.0);
    assert_eq!(alpha(&s, 26, 44), 255, "主の側は塗れる");
    assert_eq!(
        alpha(&s, 38, 44),
        0,
        "映した先は選択の内側でも、そこのステンシルが閉じていれば塗れない"
    );
    // ストロークは 1 本ずつ 1 回の Undo。選択範囲は消えない
    s.apply(Action::Undo);
    assert_eq!(alpha(&s, 26, 44), 0);
    assert_eq!(alpha(&s, 22, 12), 255, "前のストロークは残る");
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(alpha(&s, 22, 12), 0);
    assert_eq!(alpha(&s, 16, 28), 0);
    assert!(
        s.doc.selection().is_some(),
        "ストロークの Undo は選択範囲を消さない"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── .ylp の保存と読み込み ─────────

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-sel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn headless_selection_survives_save_and_reopen_and_clearing_removes_it() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("sel.ylp");
    let mut s = doc_state(64);
    paint_rect(&mut s, 0, 0, 64, 64, [10, 20, 30, 255]);
    run(&mut s, rect(SelectionCombine::Replace, 8, 8, 40, 40));
    run(
        &mut s,
        SelEdit::Modify {
            kind: ModifyKind::Feather,
            radius: 6,
            edge_lock: false,
        },
    );
    let saved = s.doc.selection().cloned().unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = doc_state(64);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.doc.selection(), Some(&saved), "{}", again.message);
    assert!(!again.doc.can_undo(), "開いた直後は戻せる段が無い");
    assert!(!again.modified);
    // 選択範囲だけを変えて上書き保存 → 読み直すと新しい選択範囲
    run(&mut again, rect(SelectionCombine::Replace, 20, 20, 50, 30));
    let moved = again.doc.selection().cloned().unwrap();
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    let mut third = doc_state(64);
    third.apply(Action::OpenProject(path.clone()));
    assert_eq!(third.doc.selection(), Some(&moved));
    // 解除して保存すると selection.bin が消える
    run(&mut third, SelEdit::Clear);
    third.apply(Action::SaveProject);
    let mut fourth = doc_state(64);
    fourth.apply(Action::OpenProject(path.clone()));
    assert!(fourth.doc.selection().is_none(), "{}", fourth.message);
    // 描いた内容は残る
    assert_eq!(
        fourth.doc.composite_pixel(Channel::Color, 30, 30).unwrap(),
        Rgba8::new(10, 20, 30, 255)
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 選択範囲を持つ 2 つのセットを .ylp に保存して開き直した状態（A が今のセット）。
fn two_sets_with_selections(dir: &std::path::Path) -> (AppState, SelectionMask, SelectionMask) {
    use yolu_app::sets::{guid_string, MaterialRef, TextureSets};
    let path = dir.join("sets.ylp");
    let make = |x0, x1| {
        let (mut doc, _) = yolu_app::state::blank_document(64, 64);
        doc.set_selection(Some(SelectionMask::rectangle(&doc, x0, 4, x1, 20)))
            .unwrap();
        doc
    };
    let (a, b) = (make(4, 20), make(30, 50));
    let (sel_a, sel_b) = (
        a.selection().cloned().unwrap(),
        b.selection().cloned().unwrap(),
    );
    let parts = vec![
        (
            guid_string(a.id()),
            "A".to_string(),
            MaterialRef::PendingSlot(0),
            None,
            a,
        ),
        (
            guid_string(b.id()),
            "B".to_string(),
            MaterialRef::PendingSlot(1),
            None,
            b,
        ),
    ];
    let (sets, doc) = TextureSets::from_parts(parts, 0);
    let mut s = doc_state(64);
    s.replace_sets(sets, doc);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = doc_state(64);
    again.apply(Action::OpenProject(path));
    assert_eq!(again.sets.len(), 2, "{}", again.message);
    (again, sel_a, sel_b)
}

fn open_drafts(s: &mut AppState) {
    s.sel.polygon.push((1.0, 1.0));
    s.sel.polygon_hover = Some((2.0, 2.0));
    s.sel.dialog = Some(yolu_app::selection::AmountDialog {
        kind: ModifyKind::Grow,
        radius: 3,
        edge_lock: false,
    });
}

#[test]
fn headless_each_set_keeps_its_own_selection_and_its_own_undo_history() {
    let dir = temp_dir("sets");
    let (mut again, sel_a, sel_b) = two_sets_with_selections(&dir);
    assert_eq!(again.doc.selection(), Some(&sel_a));
    again.switch_set(1).unwrap();
    assert_eq!(
        again.doc.selection(),
        Some(&sel_b),
        "セットごとに別の選択範囲"
    );
    // B で選択範囲を替える（B の履歴に 1 段）。A へ戻ると A の選択範囲と履歴はそのまま
    run(&mut again, rect(SelectionCombine::Replace, 0, 0, 10, 10));
    let edited_b = again.doc.selection().cloned().unwrap();
    assert_ne!(edited_b, sel_b);
    assert_eq!(again.doc.undo_count(), 1);
    again.switch_set(0).unwrap();
    assert_eq!(again.doc.selection(), Some(&sel_a));
    assert!(!again.doc.can_undo(), "B の履歴は A に持ち込まない");
    again.apply(Action::Undo);
    assert_eq!(again.doc.selection(), Some(&sel_a), "A には戻す段が無い");
    // B へ戻ると編集した選択範囲のまま。Undo で B の前の選択範囲へ戻り、A は動かない
    again.switch_set(1).unwrap();
    assert_eq!(again.doc.selection(), Some(&edited_b));
    again.apply(Action::Undo);
    assert_eq!(again.doc.selection(), Some(&sel_b), "B の前の選択範囲へ");
    again.apply(Action::Redo);
    assert_eq!(again.doc.selection(), Some(&edited_b));
    again.switch_set(0).unwrap();
    assert_eq!(again.doc.selection(), Some(&sel_a));
    // セットを替えると、途中の形も量を聞くウィンドウも捨てる
    open_drafts(&mut again);
    again.switch_set(1).unwrap();
    assert!(again.sel.polygon.is_empty() && again.sel.polygon_hover.is_none());
    assert!(again.sel.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_opening_a_project_drops_the_drafts_of_the_previous_document() {
    let dir = temp_dir("sets-open");
    let (mut again, _, _) = two_sets_with_selections(&dir);
    let path = dir.join("sets.ylp");
    open_drafts(&mut again);
    again.sel.drag = Some(yolu_app::selection::ShapeDrag {
        tool: Tool::SelectRect,
        source: yolu_app::state::StrokeSource::Mouse,
        start_screen: Pos2::ZERO,
        start: (1.0, 1.0),
        current: (5.0, 5.0),
        lasso: Vec::new(),
        moved: 10.0,
    });
    again.apply(Action::OpenProject(path));
    assert_eq!(again.sets.len(), 2, "{}", again.message);
    assert!(again.sel.polygon.is_empty() && again.sel.dialog.is_none());
    assert!(again.sel.drag.is_none(), "開き直しで形の途中も捨てる");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_importing_a_psd_into_the_current_set_drops_the_drafts_of_the_old_document() {
    use yolu_app::psd::{PsdAction, PsdTarget};
    let dir = temp_dir("psd-drafts");
    let path = dir.join("in.psd");
    let mut source = doc_state(64);
    source.apply(Action::Psd(PsdAction::Export(path.clone())));
    source.wait_psd();
    assert!(path.exists(), "{}", source.message);
    let mut s = doc_state(32);
    open_drafts(&mut s);
    s.apply(Action::Psd(PsdAction::Import {
        path: path.clone(),
        target: PsdTarget::CurrentSet,
    }));
    // 読んでいる間（別のスレッド）に、新しい点を打つ・ウィンドウを開くのは続けられる。読み終わるときに、前の文書の途中は捨てる
    s.wait_psd();
    assert_eq!(s.doc.width(), 64, "{}", s.message);
    assert!(s.sel.polygon.is_empty() && s.sel.polygon_hover.is_none());
    assert!(s.sel.dialog.is_none(), "前の文書の座標のウィンドウが残った");
    assert!(s.sel.drag.is_none() && s.sel.pen_down.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 言語・名前 ─────────

#[test]
fn headless_every_edits_status_message_follows_the_language() {
    let edits = || -> Vec<SelEdit> {
        let mut v = vec![
            SelEdit::Clear,
            SelEdit::Invert,
            SelEdit::All,
            SelEdit::All,
            rect(SelectionCombine::Replace, 8, 8, 40, 40),
            rect(SelectionCombine::Replace, 8, 8, 40, 40),
            rect(SelectionCombine::Add, 20, 20, 50, 50),
            rect(SelectionCombine::Subtract, 0, 0, 64, 64),
            rect(SelectionCombine::Subtract, 0, 0, 64, 64),
            SelEdit::Ellipse {
                cx: 32.0,
                cy: 32.0,
                rx: 20.0,
                ry: 12.0,
                mode: SelectionCombine::Replace,
            },
            SelEdit::Polygon {
                points: vec![(5.0, 5.0), (60.0, 8.0), (30.0, 58.0)],
                mode: SelectionCombine::Intersect,
            },
            SelEdit::Wand {
                seeds: vec![(1, 1)],
                mode: SelectionCombine::Replace,
            },
            SelEdit::Invert,
            SelEdit::Invert,
        ];
        // ぼかした選択範囲をくっきりさせる（硬い選択範囲では変わらないので、完了の文を通すには柔らかくしておく）
        v.push(rect(SelectionCombine::Replace, 10, 10, 50, 50));
        for (kind, radius) in [(ModifyKind::Feather, 6), (ModifyKind::Sharpen, 0)] {
            v.push(SelEdit::Modify {
                kind,
                radius,
                edge_lock: false,
            });
        }
        for kind in ModifyKind::ALL {
            // 半径をとる 4 種は「n px」、くっきりは名前だけ。変わらない・何も残らない場合の文も通す
            for radius in [4, 4, 200] {
                v.push(rect(SelectionCombine::Replace, 10, 10, 50, 50));
                v.push(SelEdit::Modify {
                    kind,
                    radius,
                    edge_lock: false,
                });
            }
            v.push(SelEdit::Modify {
                kind,
                radius: 4,
                edge_lock: false,
            });
        }
        v
    };
    for lang in [Lang::Ja, Lang::En] {
        let mut s = doc_state(64);
        s.lang = lang;
        let mut sharpen_seen = false;
        for e in edits() {
            s.message.clear();
            run(&mut s, e.clone());
            let m = s.message.clone();
            assert!(!m.is_empty(), "{lang:?} {e:?}");
            match lang {
                Lang::En => assert!(m.is_ascii(), "英語の画面に日本語が混じる: {m:?} ({e:?})"),
                Lang::Ja => assert!(!m.is_ascii(), "日本語の画面の文: {m:?} ({e:?})"),
            }
            if matches!(
                e,
                SelEdit::Modify {
                    kind: ModifyKind::Sharpen,
                    ..
                }
            ) && s.doc.selection().is_some()
                && m.starts_with(ModifyKind::Sharpen.name(lang))
                && !m.contains(':')
            {
                // 半径をとらないくっきりの完了の文: 言語の句点で終わる
                sharpen_seen = true;
                assert!(
                    m.ends_with(lang.pick("。", ".")) && !m.ends_with(lang.pick(".", "。")),
                    "{lang:?}: {m:?}"
                );
            }
        }
        assert!(
            sharpen_seen,
            "{lang:?}: くっきりの完了の文を確かめられなかった"
        );
    }
}

#[test]
fn headless_every_selection_label_exists_in_both_languages() {
    let labels = |lang: Lang| -> Vec<String> {
        let mut s = doc_state(32);
        s.lang = lang;
        run(&mut s, SelEdit::All);
        let mut v = Vec::new();
        for entries in [yolu_app::selection::menu::select_menu(&s)] {
            for e in entries {
                if let yolu_app::ui::menu::Entry::Item { label, .. } = e {
                    assert!(!label.trim().is_empty());
                    v.push(label);
                }
            }
        }
        for tool in Tool::ALL {
            assert!(!tool.name_in(lang).is_empty());
            v.push(tool.name_in(lang).to_owned());
        }
        for kind in ModifyKind::ALL {
            v.push(kind.name(lang).to_owned());
            v.push(kind.tooltip(lang).to_owned());
        }
        v
    };
    let (ja, en) = (labels(Lang::Ja), labels(Lang::En));
    assert_eq!(ja.len(), en.len());
    assert!(ja.iter().zip(&en).all(|(a, b)| a != b), "{ja:?} {en:?}");
    // ツールの ID・キーは重ならない
    let mut ids: Vec<&str> = Tool::ALL.iter().map(|t| t.id()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), Tool::ALL.len());
    let mut keys: Vec<&str> = Tool::ALL.iter().map(|t| t.key()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), Tool::ALL.len());
}
