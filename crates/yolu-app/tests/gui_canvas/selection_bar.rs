//! 選択範囲の下のボタンの帯（Photoshop のコンテキストタスクバー・CLIP STUDIO の選択範囲ランチャー）: 外接矩形のすぐ下に出て、表示の回転・拡大に付いていき、
//! 描いている間・選択の形を作っている間は隠れる。ボタンは 1 回の Undo で、文字のラベルは無い。ドラッグでずらせて、メニューで出さないこともできる。
use crate::common;

use common::*;
use egui::{vec2, Event, Key, Modifiers, PointerButton, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::SelectionCombine;
use yolu_app::lang::Lang;
use yolu_app::selection::{ModifyKind, SelAction, SelEdit, SelUiOp};
use yolu_app::state::{Action, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

// 日本語のツールチップ（読み上げの名前）。
const DESELECT: &str = "選択を解除（Ctrl+D / Esc）";
const INVERT: &str = "選択範囲を反転（Ctrl+Shift+I）";
const GROW: &str = "選択範囲を拡張…";
const SHRINK: &str = "選択範囲を縮小…";
const FILL: &str = "描画色で塗りつぶす";
const ERASE: &str = "選択範囲を消去（Delete）";
const COPY: &str = "コピーして新しいレイヤーに（Ctrl+J）";
const MASK: &str = "選択範囲をレイヤーマスクにする";
const REMEMBER: &str = "選択範囲を覚える…";
const ALL: [&str; 9] = [
    DESELECT, INVERT, GROW, SHRINK, FILL, ERASE, COPY, MASK, REMEMBER,
];

fn select_rect(h: &mut H, x0: i64, y0: i64, x1: i64, y1: i64) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
            x0,
            y0,
            x1,
            y1,
            mode: SelectionCombine::Replace,
        })));
    h.run();
}

fn st(h: &H) -> &yolu_app::state::AppState {
    &h.state().state
}

const GRIP: &str = "ボタンの帯を動かす";

/// 帯の部品（キャンバスの中のもの。選択のツールのときはプロパティの欄にも同じ名前のボタンがある）。
fn in_canvas(h: &H, label: &str) -> Option<Rect> {
    let canvas = canvas_rect(h);
    h.query_all_by_label(label)
        .map(|n| n.rect())
        .find(|r| canvas.contains_rect(*r))
}

fn bar_shown(h: &H) -> bool {
    in_canvas(h, GRIP).is_some()
}

/// 帯（持ち手とボタンを囲んだ矩形）。
fn bar_rect(h: &H) -> Rect {
    let mut rects: Vec<Rect> = ALL
        .iter()
        .map(|l| in_canvas(h, l).unwrap_or_else(|| panic!("{l} が帯に無い")))
        .collect();
    rects.push(in_canvas(h, GRIP).expect("持ち手"));
    rects.iter().skip(1).fold(rects[0], |a, r| a.union(*r))
}

/// 選択範囲の外接矩形（画面の点）。
fn screen_bounds(h: &H) -> Rect {
    let s = st(h);
    let view = s.view.view(canvas_rect(h), s.doc.width(), s.doc.height());
    let m = s.doc.selection().expect("選択範囲がある");
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..m.height() {
        for x in 0..m.width() {
            if m.amount(x, y) > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    let corners =
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| view.to_screen(x as f64, y as f64));
    Rect::from_points(&corners)
}

fn pixel(h: &H, x: u32, y: u32) -> [u8; 4] {
    yolu_app::engine::composite_pixel(&st(h).doc, x, y)
}

fn click_label(h: &mut H, label: &str) {
    let at = in_canvas(h, label)
        .unwrap_or_else(|| panic!("{label} が帯に無い"))
        .center();
    click(h, at);
}

#[test]
fn the_bar_floats_below_the_selection_with_icons_only() {
    let mut h = app(1280.0, 800.0, 256);
    assert!(!bar_shown(&h), "選択範囲が無ければ出ない");
    select_rect(&mut h, 80, 80, 140, 120);
    assert!(bar_shown(&h));
    let (bar, bounds, canvas) = (bar_rect(&h), screen_bounds(&h), canvas_rect(&h));
    assert!(
        bar.top() >= bounds.bottom(),
        "外接矩形の下: {bar:?} {bounds:?}"
    );
    assert!(bar.top() - bounds.bottom() < 24.0, "すぐ下");
    assert!((bar.center().x - bounds.center().x).abs() < 2.0, "中央");
    assert!(canvas.contains_rect(bar), "表示域の中");
    // ボタンの並びは左から右（解除・反転・拡張・縮小・塗りつぶし・消去・コピー・マスク・覚える）
    let xs: Vec<f32> = ALL
        .iter()
        .map(|l| in_canvas(&h, l).unwrap().center().x)
        .collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    // 文字のラベルは無い: 帯の中の読み上げの名前はボタンのツールチップだけ（描いた文字は無い）
    h.snapshot("selection_bar");
}

#[test]
fn the_bar_follows_zoom_rotation_flip_and_pan() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 60, 100, 120, 160);
    for (zoom, angle, flip, pan) in [
        (2.0, 0.0, false, vec2(0.0, 0.0)),
        (1.0, 90.0, false, vec2(0.0, 0.0)),
        (1.5, 33.0, false, vec2(40.0, -20.0)),
        (1.0, 20.0, true, vec2(-30.0, 10.0)),
    ] {
        {
            let view = &mut h.state_mut().state.view;
            view.fit();
            view.zoom = zoom;
            view.pan = pan;
            view.set_angle(angle);
            view.flip = flip;
        }
        h.run();
        let (bar, bounds, canvas) = (bar_rect(&h), screen_bounds(&h), canvas_rect(&h));
        assert!(
            canvas.contains_rect(bar),
            "{zoom} {angle} {flip}: 表示域の中 {bar:?} {canvas:?}"
        );
        assert!(
            bar.top() >= bounds.bottom() || bar.bottom() <= bounds.top(),
            "{zoom} {angle} {flip}: 外接矩形に重ならない {bar:?} {bounds:?}"
        );
    }
}

#[test]
fn the_bar_goes_above_or_inside_when_there_is_no_room_below() {
    let mut h = app(1280.0, 800.0, 256);
    // キャンバスの下端に近い選択範囲: 下に入らないので上
    select_rect(&mut h, 100, 0, 160, 8);
    let (bar, bounds) = (bar_rect(&h), screen_bounds(&h));
    // （文書の y は画面の下から数える設定の場合もあるので、どちらかの側にあればよい）
    assert!(
        bar.top() >= bounds.bottom() || bar.bottom() <= bounds.top(),
        "{bar:?} {bounds:?}"
    );
    assert!(canvas_rect(&h).contains_rect(bar));
    // 画面いっぱいの選択範囲: 内側の下
    select_rect(&mut h, 0, 0, 256, 256);
    let bar = bar_rect(&h);
    let canvas = canvas_rect(&h);
    assert!(canvas.contains_rect(bar));
    assert!(
        bar.bottom() > canvas.bottom() - 20.0,
        "表示域の下の内側 {bar:?} {canvas:?}"
    );
    // 表示域の外へ出た選択範囲には、帯を出さない
    h.state_mut().state.view.zoom = 8.0;
    h.state_mut().state.view.pan = vec2(-5000.0, 0.0);
    select_rect(&mut h, 200, 200, 240, 240);
    h.state_mut().state.view.pan = vec2(5000.0, 0.0);
    h.run();
    assert!(!bar_shown(&h));
}

#[test]
fn the_bar_hides_while_painting_making_a_shape_or_moving_the_view() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 80, 80, 140, 120);
    assert!(bar_shown(&h));
    let c = canvas_rect(&h).center();
    // 描いている間
    press(&h, offset(c, 60.0, 60.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 90.0, 60.0));
    h.step();
    assert!(st(&h).is_stroking());
    assert!(!bar_shown(&h), "描いている間は隠れる");
    release(&h, offset(c, 90.0, 60.0), PointerButton::Primary);
    h.run();
    assert!(bar_shown(&h), "離したら戻る");
    // 選択の形を作っている間
    h.state_mut()
        .state
        .apply(Action::SelectTool(Tool::SelectRect));
    press(&h, offset(c, -100.0, -100.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, -60.0, -70.0));
    h.step();
    assert!(st(&h).sel.drag.is_some());
    assert!(!bar_shown(&h), "選択の形を作っている間は隠れる");
    release(&h, offset(c, -60.0, -70.0), PointerButton::Primary);
    h.run();
    assert!(bar_shown(&h));
    // 多角形の点を打っている間
    h.state_mut().state.apply(Action::SelectTool(Tool::Polygon));
    click(&mut h, offset(c, 100.0, -100.0));
    assert!(!st(&h).sel.polygon.is_empty());
    assert!(!bar_shown(&h));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(bar_shown(&h));
    // 表示を動かしている間（Space + ドラッグのパン・Ctrl+Space の拡縮）
    h.event(Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    press(&h, c, PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 20.0, 0.0));
    h.step();
    assert!(!bar_shown(&h), "パンしている間は隠れる");
    release(&h, offset(c, 20.0, 0.0), PointerButton::Primary);
    h.event(Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert!(bar_shown(&h));
}

/// 選択の形のドラッグを、押す・動かす・離すで行う。
fn drag_shape(h: &mut H, from: egui::Pos2, to: egui::Pos2) {
    press(h, from, PointerButton::Primary);
    h.step();
    move_to(h, to);
    h.step();
    release(h, to, PointerButton::Primary);
    h.run();
}

#[test]
fn the_bar_appears_as_soon_as_any_selection_tool_finishes_the_selection() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    let clear = |h: &mut H| {
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
        h.run();
        assert!(!bar_shown(h) && st(h).doc.selection().is_none());
    };
    for tool in [Tool::SelectRect, Tool::SelectEllipse] {
        h.state_mut().state.apply(Action::SelectTool(tool));
        drag_shape(&mut h, offset(c, -80.0, -60.0), offset(c, 80.0, 60.0));
        assert!(st(&h).doc.selection().is_some(), "{tool:?}");
        assert!(bar_shown(&h), "{tool:?}: 離したら帯が出る");
        clear(&mut h);
    }
    // なげなわ: なぞって離す
    h.state_mut().state.apply(Action::SelectTool(Tool::Lasso));
    press(&h, offset(c, -80.0, -60.0), PointerButton::Primary);
    h.step();
    for p in [(80.0, -60.0), (80.0, 60.0), (-80.0, 60.0), (-80.0, -50.0)] {
        move_to(&h, offset(c, p.0, p.1));
        h.step();
    }
    release(&h, offset(c, -80.0, -50.0), PointerButton::Primary);
    h.run();
    assert!(st(&h).doc.selection().is_some());
    assert!(bar_shown(&h), "なげなわ: 離したら帯が出る");
    clear(&mut h);
    // 多角形: Enter で閉じる・ダブルクリックで閉じる
    h.state_mut().state.apply(Action::SelectTool(Tool::Polygon));
    for p in [(-80.0, -60.0), (80.0, -60.0), (80.0, 60.0), (-80.0, 60.0)] {
        click(&mut h, offset(c, p.0, p.1));
    }
    assert!(!bar_shown(&h), "点を打っている間は隠れる");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(st(&h).doc.selection().is_some() && st(&h).sel.polygon.is_empty());
    assert!(bar_shown(&h), "多角形: Enter で閉じたら帯が出る");
    clear(&mut h);
    for p in [(-80.0, -60.0), (80.0, -60.0), (80.0, 60.0)] {
        click(&mut h, offset(c, p.0, p.1));
    }
    click(&mut h, offset(c, 80.0, 60.0));
    h.run();
    assert!(st(&h).doc.selection().is_some(), "ダブルクリックで閉じる");
    assert!(bar_shown(&h), "多角形: ダブルクリックで閉じたら帯が出る");
    clear(&mut h);
    // 自動選択: クリック
    h.state_mut().state.apply(Action::SelectTool(Tool::Wand));
    click(&mut h, offset(c, 0.0, 0.0));
    assert!(st(&h).doc.selection().is_some());
    assert!(bar_shown(&h), "自動選択: クリックしたら帯が出る");
    clear(&mut h);
    // ペン（Windows Ink）: 触れて動かして離す。離したら帯が出る（押しの札が残らない）
    h.state_mut()
        .state
        .apply(Action::SelectTool(Tool::SelectRect));
    for (p, contact) in [
        (offset(c, -80.0, -60.0), true),
        (offset(c, 0.0, 0.0), true),
        (offset(c, 80.0, 60.0), true),
        (offset(c, 80.0, 60.0), false),
    ] {
        h.state().pen().push(yolu_app::pen::PenSample {
            pos: [p.x, p.y],
            pressure: 0.5,
            tilt: yolu_app::engine::Tilt::default(),
            rotation: None,
            contact,
            eraser: false,
            barrel: false,
            pointer_id: 3,
            time_ms: 0,
        });
        h.step();
    }
    h.run();
    assert!(st(&h).doc.selection().is_some());
    assert!(bar_shown(&h), "ペン: 離したら帯が出る");
}

#[test]
fn escape_clears_a_finished_selection_like_ctrl_d_in_one_undo() {
    let mut h = app(1280.0, 800.0, 256);
    // 選択が無いときの Esc は何もしない（取り消しの段も、知らせも）
    let steps = st(&h).doc.undo_count();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(st(&h).message.is_empty(), "{}", st(&h).message);
    // 選択を作り終えた後（点線と帯が出ている）の Esc: 解除。Ctrl+D と同じ 1 回の取り消し、同じ知らせ
    select_rect(&mut h, 80, 80, 140, 120);
    assert!(bar_shown(&h));
    let steps = st(&h).doc.undo_count();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(st(&h).doc.selection().is_none());
    assert!(!bar_shown(&h));
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    let message = st(&h).message.clone();
    assert_eq!(message, "選択を解除しました。");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(st(&h).doc.selection().is_some(), "Undo 1 回で選択が戻る");
    assert_eq!(st(&h).doc.undo_count(), steps);
    // Ctrl+D と同じ結果（選択も、段の数も、知らせも）
    key(&h, Key::D, Modifiers::COMMAND);
    h.run();
    assert!(st(&h).doc.selection().is_none());
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    assert_eq!(st(&h).message, message);
    // 描き終えたあとも、押し続けていない限り効く（描いたあとの Esc）
    select_rect(&mut h, 80, 80, 140, 120);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    let c = canvas_rect(&h).center();
    drag_shape(&mut h, offset(c, 60.0, 60.0), offset(c, 90.0, 60.0));
    assert!(st(&h).doc.selection().is_some());
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(
        st(&h).doc.selection().is_none(),
        "描き終えたあとの Esc で解除"
    );
}

#[test]
fn escape_goes_first_to_what_is_in_progress_text_fields_menus_and_windows() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    select_rect(&mut h, 80, 80, 140, 120);
    let esc = |h: &mut H| {
        key(h, Key::Escape, Modifiers::NONE);
        h.run();
    };
    // 選択の形を作っている途中（ドラッグ）: Esc はそれをやめるだけで、今の選択は残る
    h.state_mut()
        .state
        .apply(Action::SelectTool(Tool::SelectRect));
    press(&h, offset(c, -100.0, -100.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, -60.0, -70.0));
    h.step();
    assert!(st(&h).sel.drag.is_some());
    esc(&mut h);
    assert!(st(&h).sel.drag.is_none(), "ドラッグをやめた");
    assert!(st(&h).doc.selection().is_some(), "選択は残る");
    release(&h, offset(c, -60.0, -70.0), PointerButton::Primary);
    h.run();
    assert!(
        st(&h).doc.selection().is_some(),
        "離した後も選択は残り、新しく作らない"
    );
    // 多角形の点を打っている途中: 1 回目の Esc は途中の形だけ、2 回目で選択を解除
    h.state_mut().state.apply(Action::SelectTool(Tool::Polygon));
    click(&mut h, offset(c, 100.0, -100.0));
    assert!(!st(&h).sel.polygon.is_empty());
    esc(&mut h);
    assert!(st(&h).sel.polygon.is_empty());
    assert!(st(&h).doc.selection().is_some());
    // 描いている途中（押している）: Esc は描きをやめるだけ
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    press(&h, offset(c, 60.0, 60.0), PointerButton::Primary);
    h.step();
    move_to(&h, offset(c, 90.0, 60.0));
    h.step();
    assert!(st(&h).is_stroking());
    esc(&mut h);
    assert!(!st(&h).is_stroking());
    assert!(st(&h).doc.selection().is_some(), "描きをやめただけ");
    release(&h, offset(c, 90.0, 60.0), PointerButton::Primary);
    h.run();
    assert!(st(&h).doc.selection().is_some());
    // 文字の入力欄（チャンネルの名前を変えている）: Esc は入力をやめるだけ
    click_tab(&mut h, yolu_app::Tab::Channels);
    h.get_by_label("チャンネルを追加").click();
    h.run();
    let at = popup_item(&h, "L8").center();
    click(&mut h, at);
    let row = rect_of(&h, "スカラー 1", |r| r.height() < 40.0);
    let name_at = egui::pos2(row.left() + 100.0, row.center().y);
    for _ in 0..2 {
        press(&h, name_at, PointerButton::Primary);
        release(&h, name_at, PointerButton::Primary);
        h.step();
    }
    h.run();
    assert!(
        st(&h).m2.renaming_channel.is_some(),
        "名前を変える入力欄が開いた"
    );
    esc(&mut h);
    assert!(st(&h).m2.renaming_channel.is_none(), "入力をやめた");
    assert!(
        st(&h).doc.selection().is_some(),
        "入力欄の Esc で選択を外さない"
    );
    // メニューを開いている間: Esc はメニューを閉じるだけ
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    assert!(st(&h).popup.is_some());
    esc(&mut h);
    assert!(st(&h).popup.is_none(), "メニューが閉じた");
    assert!(
        st(&h).doc.selection().is_some(),
        "メニューの Esc で選択を外さない"
    );
    // 浮いたウィンドウが開いている間（設定のウィンドウ）: ウィンドウを優先して、選択は残す
    h.state_mut()
        .state
        .apply(Action::Prefs(yolu_app::prefs::PrefsAction::Open));
    h.run();
    esc(&mut h);
    assert!(
        st(&h).doc.selection().is_some(),
        "ウィンドウが開いている間の Esc で選択を外さない"
    );
    h.state_mut()
        .state
        .apply(Action::Prefs(yolu_app::prefs::PrefsAction::Close));
    h.run();
    h.run();
    // 何も使わなくなったら、Esc で解除
    esc(&mut h);
    assert!(
        st(&h).doc.selection().is_none(),
        "Esc を使うものが無いので解除"
    );
}

/// Esc を 1 回押して、選択がまだ残り、取り消しの段が増えていないこと（Esc は先に別の部品が使った）。
fn esc_keeps_the_selection(h: &mut H, what: &str) {
    let steps = st(h).doc.undo_count();
    key(h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(
        st(h).doc.selection().is_some(),
        "{what}: Esc で選択を外さない"
    );
    assert_eq!(
        st(h).doc.undo_count(),
        steps,
        "{what}: 取り消しの段も積まない"
    );
}

/// 何も Esc を使うものが無くなったら、次の 1 回で選択を解除する。
fn esc_clears_the_selection(h: &mut H, what: &str) {
    key(h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(
        st(h).doc.selection().is_none(),
        "{what}: 使うものが無いので Esc で解除"
    );
}

#[test]
fn escape_closes_the_3d_settings_panel_before_the_selection_with_the_canvas_beside_it() {
    use egui_dock::{DockState, NodeIndex};
    use yolu_app::view3d::display::Op;
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.view3d.load_demo();
    // キャンバスと 3D ビューを左右に並べる（既定のドックは同じ組のタブなので、並べたときだけ両方が同時に描かれる）
    let mut dock = DockState::new(vec![yolu_app::Tab::Canvas]);
    dock.main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![yolu_app::Tab::View3d]);
    h.state_mut().dock = dock;
    h.run();
    assert!(h.state().view3d_rect().is_some(), "3D も出ている");
    select_rect(&mut h, 80, 80, 140, 120);
    h.state_mut()
        .state
        .apply(Action::View3d(Op::ToggleSettings));
    h.run();
    assert!(st(&h).view3d.display.settings_open, "設定のパネルが開いた");
    // Esc はパネルを閉じるだけ
    esc_keeps_the_selection(&mut h, "3D の設定のパネル");
    assert!(!st(&h).view3d.display.settings_open, "パネルは閉じた");
    h.run();
    esc_clears_the_selection(&mut h, "パネルを閉じたあと");
}

#[test]
fn escape_closes_the_color_picker_before_the_selection() {
    use yolu_app::m2::{AdjustmentKind, Edit};
    let mut h = app(1280.0, 1500.0, 64);
    h.state_mut()
        .state
        .apply(Action::M2(Edit::NewAdjustment(AdjustmentKind::GradientMap)));
    h.run();
    let id = st(&h).selected_layer.expect("足したレイヤーを選ぶ");
    select_rect(&mut h, 10, 10, 30, 30);
    // 色の見本を押して色の選びを開く（プロパティの欄を、見本が見えるところまで送る）
    let label = "分岐点の色";
    let right = |r: Rect| r.left() > rx();
    let at = rect_of(&h, label, right).top();
    let scroll = st(&h).m2.props_scroll + (at - 900.0);
    h.state_mut().state.m2.props_scroll = scroll.max(0.0);
    h.run();
    let swatch = rect_of(&h, label, right);
    click(&mut h, swatch.center());
    assert!(
        yolu_app::panels::color_window::is_target(
            &h.ctx,
            yolu_app::panels::ramp_rows::stop_target(("adjustment", id.0), 0)
        ),
        "色のウィンドウが開いた"
    );
    assert!(st(&h).doc.selection().is_some(), "開く押しで選択を外さない");
    // Esc は色の選びを（元の色へ戻して）閉じるだけ
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(
        !yolu_app::panels::color_window::is_open(&h.ctx),
        "色のウィンドウが閉じた"
    );
    assert!(
        st(&h).doc.selection().is_some(),
        "色の選びの Esc で選択を外さない"
    );
    h.run();
    esc_clears_the_selection(&mut h, "色の選びを閉じたあと");
}

#[test]
fn escape_answers_a_confirm_window_before_the_selection() {
    use yolu_app::m2::Edit;
    use yolu_core::Rgba8;
    let mut h = app(1280.0, 800.0, 64);
    let paint = |h: &mut H, rect: (u32, u32, u32, u32), color: Rgba8| {
        let s = &mut h.state_mut().state;
        let id = s.selected_layer.unwrap();
        for y in rect.1..rect.3 {
            for x in rect.0..rect.2 {
                s.doc
                    .set_channel_pixel(id, yolu_app::engine::Channel::Color, x, y, color)
                    .unwrap();
            }
        }
        id
    };
    // 下から 3 つのレイヤー。間をはさんだ 2 つを結合すると見た目が変わるので、確認のウィンドウが出る
    let a = paint(&mut h, (2, 2, 12, 12), Rgba8::new(255, 0, 0, 255));
    h.state_mut().state.apply(Action::NewLayer);
    paint(&mut h, (6, 6, 16, 16), Rgba8::new(0, 255, 0, 255));
    h.state_mut().state.apply(Action::NewLayer);
    let c = paint(&mut h, (10, 10, 20, 20), Rgba8::new(0, 0, 255, 255));
    h.run();
    select_rect(&mut h, 30, 30, 50, 50);
    h.state_mut().state.select_layers([a, c], c);
    h.state_mut().state.apply(Action::M2(Edit::MergeDown));
    h.run();
    assert!(
        st(&h).layer_ops.merge_confirm.is_some(),
        "確認のウィンドウが出た"
    );
    let layers = st(&h).doc.layers().len();
    // Esc はウィンドウをやめるだけ
    esc_keeps_the_selection(&mut h, "確認のウィンドウ");
    assert!(
        st(&h).layer_ops.merge_confirm.is_none(),
        "ウィンドウが閉じた"
    );
    assert_eq!(st(&h).doc.layers().len(), layers, "結合していない");
    h.run();
    esc_clears_the_selection(&mut h, "確認のウィンドウを閉じたあと");
}

#[test]
fn escape_closes_a_color_set_rename_before_the_selection() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 80, 80, 140, 120);
    click_tab(&mut h, yolu_app::Tab::ColorSets);
    h.state_mut().state.colorsets.rename = Some("セット".into());
    h.run();
    assert!(st(&h).colorsets.rename.is_some());
    esc_keeps_the_selection(&mut h, "色セットの名前の変更");
    assert!(st(&h).colorsets.rename.is_none(), "名前の変更をやめた");
    h.run();
    esc_clears_the_selection(&mut h, "名前の変更をやめたあと");
}

#[test]
fn escape_cancels_a_color_set_swatch_drag_before_the_selection() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 80, 80, 140, 120);
    click_tab(&mut h, yolu_app::Tab::ColorSets);
    let swatch_at = |h: &H, i: usize| {
        let label = st(h).colorsets.palette().colors[i].label();
        rect_of(h, &label, |r| r.width() < 40.0).center()
    };
    let before = st(&h).colorsets.palette().clone();
    let from = swatch_at(&h, 0);
    let to = swatch_at(&h, 2);
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, to);
    h.step();
    assert!(st(&h).colorsets.dragging.is_some(), "色見本を動かしている");
    // ボタンを押したままの Esc: ドラッグをやめるだけ
    esc_keeps_the_selection(&mut h, "色見本のドラッグ");
    assert!(st(&h).colorsets.dragging.is_none(), "ドラッグをやめた");
    release(&h, to, PointerButton::Primary);
    h.run();
    assert_eq!(st(&h).colorsets.palette(), &before, "落とさない");
    assert!(st(&h).doc.selection().is_some());
    esc_clears_the_selection(&mut h, "ドラッグをやめたあと");
}

#[test]
fn escape_cancels_a_brush_row_drag_before_the_selection() {
    let mut h = app(1600.0, 1200.0, 128);
    select_rect(&mut h, 20, 20, 60, 50);
    for _ in 0..2 {
        h.get_by_label("今の設定を新しいブラシに").click();
        h.run();
    }
    let row = |h: &H, name: &str| {
        rect_of(h, name, |r| {
            r.left() < 340.0 && r.top() > 100.0 && r.top() < 900.0 && r.width() > 200.0
        })
    };
    let order = st(&h).toolset.set.brushes();
    let a = row(&h, "ブラシ");
    let b = row(&h, "ブラシ 2");
    let from = egui::pos2(b.left() + 30.0, b.center().y);
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, offset(from, 0.0, -8.0));
    h.step();
    let over = egui::pos2(a.left() + 30.0, a.top() + 4.0);
    move_to(&h, over);
    h.step();
    assert!(st(&h).toolset.ui.drag.is_some(), "ブラシを動かしている");
    esc_keeps_the_selection(&mut h, "ブラシの並べ替えのドラッグ");
    assert!(st(&h).toolset.ui.drag.is_none(), "ドラッグをやめた");
    release(&h, over, PointerButton::Primary);
    h.run();
    assert_eq!(st(&h).toolset.set.brushes(), order, "落とさない");
    esc_clears_the_selection(&mut h, "ドラッグをやめたあと");
}

#[test]
fn escape_cancels_a_running_fill_before_the_selection() {
    // 65536 画素を超える文書の塗りつぶしは別のスレッドの仕事になる
    let mut h = app(1280.0, 800.0, 512);
    select_rect(&mut h, 80, 80, 140, 120);
    yolu_app::region::bucket::start(&mut h.state_mut().state, vec![(1.0, 1.0)]);
    assert!(st(&h).region.job.is_some(), "塗りつぶしの仕事が走っている");
    let steps = st(&h).doc.undo_count();
    // Esc は仕事を取り消すだけ
    esc_keeps_the_selection(&mut h, "塗りつぶしの仕事");
    assert!(st(&h).region.job.is_none(), "仕事は取り消された");
    assert_eq!(st(&h).doc.undo_count(), steps, "塗っていない");
    h.run();
    esc_clears_the_selection(&mut h, "仕事を取り消したあと");
}

#[test]
fn deselect_invert_grow_and_shrink_each_take_one_undo_step() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 100, 100, 140, 140);
    let steps = st(&h).doc.undo_count();
    // 反転
    click_label(&mut h, INVERT);
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    assert_eq!(
        st(&h).doc.selection().unwrap().amount(120, 120),
        0,
        "中は外れた"
    );
    assert_eq!(st(&h).doc.selection().unwrap().amount(10, 10), 255);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(st(&h).doc.selection().unwrap().amount(120, 120), 255);
    // 拡張: 量を聞くウィンドウが開き、OK で 1 回の Undo
    click_label(&mut h, GROW);
    assert_eq!(st(&h).sel.dialog.map(|d| d.kind), Some(ModifyKind::Grow));
    assert!(!bar_shown(&h), "ウィンドウが開いている間は隠れる");
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::ApplyAmount)));
    h.run();
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "反転を戻したあとの拡張で 1 段"
    );
    let m = st(&h).doc.selection().unwrap();
    assert!(
        m.amount(97, 120) > 0 && m.amount(120, 120) == 255,
        "広がった"
    );
    // 縮小
    click_label(&mut h, SHRINK);
    assert_eq!(st(&h).sel.dialog.map(|d| d.kind), Some(ModifyKind::Shrink));
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::CancelAmount)));
    h.run();
    assert!(bar_shown(&h));
    // 解除: 帯も消える。Undo で選択範囲と帯が戻る
    click_label(&mut h, DESELECT);
    assert!(st(&h).doc.selection().is_none());
    assert!(!bar_shown(&h));
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(st(&h).doc.selection().is_some() && bar_shown(&h));
}

#[test]
fn fill_and_erase_work_inside_the_selection_only_and_undo_in_one_step() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.color.set_main([0.9, 0.1, 0.1, 1.0]);
    select_rect(&mut h, 100, 100, 140, 140);
    let steps = st(&h).doc.undo_count();
    click_label(&mut h, FILL);
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    let inside = pixel(&h, 120, 120);
    assert_eq!(inside[3], 255);
    assert!(inside[0] > 200 && inside[1] < 60, "{inside:?}");
    assert_eq!(pixel(&h, 90, 90)[3], 0, "選択範囲の外は塗らない");
    assert_eq!(pixel(&h, 141, 120)[3], 0);
    click_label(&mut h, ERASE);
    assert_eq!(st(&h).doc.undo_count(), steps + 2);
    assert_eq!(pixel(&h, 120, 120)[3], 0, "消えた");
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(pixel(&h, 120, 120)[3], 255, "消去を戻す");
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(pixel(&h, 120, 120)[3], 0, "塗りつぶしを戻す");
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn fill_goes_into_the_mask_while_the_mask_is_being_painted() {
    let mut h = app(1280.0, 800.0, 256);
    let id = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    select_rect(&mut h, 100, 100, 140, 140);
    // 消去 = 隠す（マスクの消しゴムと同じ）、塗りつぶし = 見せる
    click_label(&mut h, ERASE);
    let mask = st(&h).doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(mask.factor_at(120, 120).unwrap(), 0.0, "隠した");
    assert_eq!(mask.factor_at(10, 10).unwrap(), 1.0, "外はそのまま");
    click_label(&mut h, FILL);
    let mask = st(&h).doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(mask.factor_at(120, 120).unwrap(), 1.0, "見せた");
}

#[test]
fn copy_to_a_new_layer_keeps_the_selection_the_clipboard_and_takes_one_undo() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.color.set_main([0.1, 0.8, 0.2, 1.0]);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]); // 塗る
    select_rect(&mut h, 0, 0, 256, 256);
    let (layers, steps) = (st(&h).doc.layers().len(), st(&h).doc.undo_count());
    let before = st(&h).doc.layers()[0].id();
    assert!(st(&h).clip.pixels.is_none());
    click_label(&mut h, COPY);
    assert_eq!(st(&h).doc.layers().len(), layers + 1, "新しいレイヤー");
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "貼り付けと選択の戻しが 1 回の Undo"
    );
    assert_eq!(
        st(&h).selected_layer,
        st(&h).doc.layers().last().map(|l| l.id()),
        "足したレイヤーを選ぶ"
    );
    assert!(st(&h).doc.selection().is_some(), "選択範囲は残る");
    assert!(st(&h).clip.pixels.is_none(), "クリップボードは変えない");
    assert!(
        st(&h).message.contains("新しいレイヤー"),
        "{}",
        st(&h).message
    );
    // 新しいレイヤーに同じ画素がある
    let new = st(&h).selected_layer.unwrap();
    let top = st(&h)
        .doc
        .layer(new)
        .unwrap()
        .surface(yolu_app::engine::Channel::Color)
        .unwrap()
        .pixel(120, 120)
        .unwrap();
    assert_eq!(top.a, 255);
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(st(&h).doc.layers().len(), layers);
    assert!(
        st(&h).doc.selection().is_some(),
        "Undo のあとも選択範囲は同じ"
    );
    assert_eq!(st(&h).doc.layers()[0].id(), before);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn make_mask_adds_a_mask_hides_the_outside_keeps_the_selection_and_takes_one_undo() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 100, 100, 140, 140);
    let id = st(&h).selected_layer.unwrap();
    assert!(st(&h).doc.layer(id).unwrap().mask().is_none());
    let steps = st(&h).doc.undo_count();
    click_label(&mut h, MASK);
    let mask = st(&h).doc.layer(id).unwrap().mask().expect("マスクが付く");
    assert_eq!(
        mask.factor_at(120, 120).unwrap(),
        1.0,
        "選択範囲の中は見える"
    );
    assert_eq!(mask.factor_at(10, 10).unwrap(), 0.0, "外は隠れる");
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "マスクを足すのと隠すのが 1 回の Undo"
    );
    assert!(st(&h).doc.selection().is_some(), "選択範囲は残る");
    assert!(st(&h).m2.edit_mask, "マスクを描く状態");
    // マスクがあれば、その上に重ねて隠す（中は変えない）
    select_rect(&mut h, 110, 110, 130, 130);
    click_label(&mut h, MASK);
    let mask = st(&h).doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(mask.factor_at(120, 120).unwrap(), 1.0);
    assert_eq!(
        mask.factor_at(105, 105).unwrap(),
        0.0,
        "前は見えていた所も、今の選択範囲の外は隠れる"
    );
    h.state_mut().state.apply(Action::Undo);
    let mask = st(&h).doc.layer(id).unwrap().mask().unwrap();
    assert_eq!(mask.factor_at(105, 105).unwrap(), 1.0, "1 回の Undo で戻る");
}

/// 反転したマスクは保存値が「隠す量」のまま、0 が隠れて 255 が見える。外を隠す書き込みは逆の値へ寄せる（マスクの塗る・消すと同じ決まり）。
#[test]
fn make_mask_on_an_inverted_mask_still_hides_the_outside_and_takes_one_undo() {
    let mut h = app(1280.0, 800.0, 256);
    let id = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::MaskInverted(id, true)));
    let factor = |h: &H, x: u32, y: u32| {
        st(h)
            .doc
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .factor_at(x, y)
            .unwrap()
    };
    assert_eq!(factor(&h, 10, 10), 0.0, "空の反転マスクは全部隠す");
    // 選択範囲の中を見せる（反転したマスクの「塗りつぶし」= 見せる）
    select_rect(&mut h, 100, 100, 140, 140);
    click_label(&mut h, FILL);
    assert_eq!(factor(&h, 120, 120), 1.0, "見せた");
    assert_eq!(factor(&h, 105, 105), 1.0);
    assert_eq!(factor(&h, 10, 10), 0.0);
    // 小さい範囲を選んでマスクにする: 外（前は見えていた所も）が隠れ、中は見えたまま
    select_rect(&mut h, 110, 110, 130, 130);
    let steps = st(&h).doc.undo_count();
    click_label(&mut h, MASK);
    assert_eq!(factor(&h, 120, 120), 1.0, "選択範囲の中は見える");
    assert_eq!(factor(&h, 105, 105), 0.0, "前は見えていた所も、外は隠れる");
    assert_eq!(
        factor(&h, 10, 10),
        0.0,
        "外は隠れたまま（見える側へ書かない）"
    );
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(factor(&h, 105, 105), 1.0, "1 回の Undo で戻る");
    assert_eq!(factor(&h, 10, 10), 0.0);
    assert_eq!(st(&h).doc.undo_count(), steps);
}

/// 新しいレイヤーへのコピーは 1 回の操作の予算（`stroke_budget_bytes`）を上限にとる。超えたら断って何も積まない。
#[test]
fn copy_to_a_new_layer_over_the_budget_is_refused_and_changes_nothing() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        h.state_mut().state.color.set_main([0.1, 0.8, 0.2, 1.0]);
        let c = canvas_rect(&h).center();
        drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
        select_rect(&mut h, 0, 0, 256, 256);
        h.state_mut()
            .state
            .doc
            .set_stroke_budget_bytes(1024)
            .unwrap();
        let (layers, steps, selected) = (
            st(&h).doc.layers().len(),
            st(&h).doc.undo_count(),
            st(&h).selected_layer,
        );
        let revision = st(&h).doc.revision();
        let label = in_canvas(&h, lang.pick(COPY, "Copy to a New Layer (Ctrl+J)"));
        let at = label.expect("コピーのボタン").center();
        click(&mut h, at);
        assert_eq!(
            st(&h).doc.layers().len(),
            layers,
            "{lang:?}: レイヤーは増えない"
        );
        assert_eq!(st(&h).doc.undo_count(), steps, "{lang:?}: 1 段も積まれない");
        assert_eq!(
            st(&h).doc.revision(),
            revision,
            "{lang:?}: 文書は変わらない"
        );
        assert_eq!(st(&h).selected_layer, selected);
        assert!(st(&h).doc.selection().is_some(), "選択範囲はそのまま");
        assert_eq!(
            st(&h).message,
            lang.pick(
                "コピーする範囲が大きすぎる",
                "The area to copy is too large"
            ),
            "{lang:?}: 断りの理由を言う（バイトの数は出さない）"
        );
        assert!(st(&h).clip.pixels.is_none(), "クリップボードは変えない");
    }
}

/// ロックされたレイヤーでは、塗りつぶし・消去（画素のロック以上）とマスク（すべてのロック）を core が断る。理由を言って、文書は 1 段も変わらない。
#[test]
fn fill_erase_and_mask_on_a_locked_layer_are_refused_and_change_nothing() {
    use yolu_core::LayerLocks;
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.color.set_main([0.1, 0.8, 0.2, 1.0]);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    let id = st(&h).selected_layer.unwrap();
    select_rect(&mut h, 0, 0, 256, 256);
    // 画素のロック: 塗りつぶしと消去は断る
    h.state_mut()
        .state
        .doc
        .set_layer_locks(id, LayerLocks::PIXELS)
        .unwrap();
    h.run();
    let before = pixel(&h, 128, 128);
    let (layers, steps) = (st(&h).doc.layers().len(), st(&h).doc.undo_count());
    let revision = st(&h).doc.revision();
    for label in [FILL, ERASE] {
        h.state_mut().state.message.clear();
        click_label(&mut h, label);
        assert!(
            st(&h).message.contains("ロック"),
            "{label}: {}",
            st(&h).message
        );
        assert_eq!(st(&h).doc.revision(), revision, "{label}");
        assert_eq!(st(&h).doc.undo_count(), steps, "{label}");
        assert_eq!(pixel(&h, 128, 128), before, "{label}");
    }
    // すべてのロック: マスクも付けられない（マスクは足されず、選択範囲も残る）
    h.state_mut()
        .state
        .doc
        .set_layer_locks(id, LayerLocks::ALL)
        .unwrap();
    h.run();
    let (revision, steps) = (st(&h).doc.revision(), st(&h).doc.undo_count());
    h.state_mut().state.message.clear();
    click_label(&mut h, MASK);
    assert!(st(&h).message.contains("ロック"), "{}", st(&h).message);
    assert!(
        st(&h).doc.layer(id).unwrap().mask().is_none(),
        "マスクは付かない"
    );
    assert_eq!(st(&h).doc.revision(), revision);
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert_eq!(st(&h).doc.layers().len(), layers);
    assert!(st(&h).doc.selection().is_some());
    // ロックしている間の消去も、塗りつぶしも、画素は同じまま
    for label in [FILL, ERASE] {
        click_label(&mut h, label);
        assert_eq!(pixel(&h, 128, 128), before, "{label}");
    }
    assert_eq!(st(&h).doc.undo_count(), steps);
}

#[test]
fn buttons_that_cannot_work_say_why_and_do_nothing() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 100, 100, 140, 140);
    // グループは描けず、画素もコピーできない（マスクにはできる）
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::GroupSelected));
    h.run();
    let steps = st(&h).doc.undo_count();
    let reason = "このレイヤーには描けません（グループ）。";
    let fill = format!("{FILL}（{reason}）");
    assert!(
        h.get_by_label(&fill).accesskit_node().is_disabled(),
        "塗りつぶしは押せない"
    );
    let erase = format!("選択範囲を消去（Delete）（{reason}）");
    assert!(h.get_by_label(&erase).accesskit_node().is_disabled());
    let copy = "コピーして新しいレイヤーに（Ctrl+J）（画素を持たないレイヤーです）";
    assert!(
        h.get_by_label(copy).accesskit_node().is_disabled(),
        "コピーは押せない"
    );
    assert!(
        !h.get_by_label(MASK).accesskit_node().is_disabled(),
        "マスクは付けられる"
    );
    let at = h.get_by_label(&fill).rect().center();
    click(&mut h, at);
    assert_eq!(st(&h).doc.undo_count(), steps, "押せないボタンは何もしない");
    // 解除・反転・拡張・縮小はレイヤーに依らず使える
    for label in [DESELECT, INVERT, GROW, SHRINK] {
        assert!(
            !h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
    }
}

#[test]
fn the_bar_can_be_dragged_and_comes_home_when_the_selection_is_cleared() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 100, 100, 140, 140);
    let home = bar_rect(&h);
    let grip = in_canvas(&h, GRIP).unwrap().center();
    drag(
        &mut h,
        &[grip, offset(grip, 40.0, 10.0), offset(grip, 120.0, 40.0)],
    );
    h.run();
    let moved = bar_rect(&h);
    assert!(
        (moved.min - home.min - vec2(120.0, 40.0)).length() < 3.0,
        "{home:?} → {moved:?}"
    );
    assert_eq!(
        st(&h).doc.undo_count(),
        1,
        "持ち手のドラッグは描かず、Undo にも入らない"
    );
    // 表示域の外へはみ出す引き方をしても、帯は表示域の中
    let grip = in_canvas(&h, GRIP).unwrap().center();
    drag(
        &mut h,
        &[
            grip,
            offset(grip, 800.0, 800.0),
            offset(grip, 3000.0, 3000.0),
        ],
    );
    h.run();
    assert!(canvas_rect(&h).contains_rect(bar_rect(&h)));
    // 選択範囲を外すと初めの位置へ戻る（次の選択範囲では、その下に出る）
    click_label(&mut h, DESELECT);
    select_rect(&mut h, 100, 100, 140, 140);
    let again = bar_rect(&h);
    assert!((again.min - home.min).length() < 3.0, "{home:?} {again:?}");
}

#[test]
fn a_pen_touch_on_a_button_presses_it_once_and_never_paints() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 100, 100, 140, 140);
    let at = in_canvas(&h, INVERT).unwrap().center();
    let sample = |contact: bool| yolu_app::pen::PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 9,
        time_ms: 0,
    };
    let steps = st(&h).doc.undo_count();
    // 触れる（winit の代わりの入力つき）
    h.state().pen().push(sample(true));
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(
        bar_shown(&h),
        "ペンが触れている間も、押したボタンは消えない"
    );
    h.state().pen().push(sample(false));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "反転が 1 回だけ（描かない）"
    );
    assert_eq!(st(&h).doc.selection().unwrap().amount(120, 120), 0);
}

#[test]
fn the_select_menu_runs_the_same_edits_and_hides_the_bar() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.color.set_main([0.2, 0.2, 0.9, 1.0]);
    select_rect(&mut h, 100, 100, 140, 140);
    // メニューの「描画色で塗りつぶす」
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    let at = popup_item(&h, "描画色で塗りつぶす").center();
    click(&mut h, at);
    assert_eq!(pixel(&h, 120, 120)[3], 255);
    // 「選択範囲のボタンの帯を表示」を外すと、帯は出ない。メニューのチェックも外れる。もう一度で戻る
    assert!(bar_shown(&h));
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    let at = popup_item(&h, "選択範囲のボタンの帯を表示").center();
    click(&mut h, at);
    assert!(!st(&h).prefs.settings.selection_bar);
    assert!(!bar_shown(&h));
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    let at = popup_item(&h, "選択範囲のボタンの帯を表示").center();
    click(&mut h, at);
    assert!(st(&h).prefs.settings.selection_bar && bar_shown(&h));
}

#[test]
fn delete_erases_the_selection_and_ctrl_j_copies_it_to_a_new_layer_while_without_one_it_duplicates_the_layer(
) {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.color.set_main([0.9, 0.5, 0.1, 1.0]);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -60.0, 0.0), offset(c, 60.0, 0.0)]);
    let steps = st(&h).doc.undo_count();
    // 選択範囲が無いとき、Delete は何もしない。Ctrl+J はレイヤーの複製（レイヤーの操作）
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), steps, "Delete は何もしない");
    key(&h, Key::J, Modifiers::COMMAND);
    h.run();
    assert_eq!(
        st(&h).doc.layers().len(),
        2,
        "選択範囲が無ければレイヤーの複製"
    );
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(st(&h).doc.layers().len(), 1);
    // 選択範囲があるとき: Ctrl+J は選択範囲の画素だけを新しいレイヤーへ（レイヤーの複製ではない）
    let at = |dx: f32| canvas_pixel_xy(&h, dx);
    assert_eq!(
        canvas_pixel(&h, offset(c, 40.0, 0.0))[3],
        255,
        "塗った線は中心から 40 点の所にもある"
    );
    let inside_x = at(0.0) as u32;
    // 塗った線（中心から左右に 60 点）の中で、選択範囲（中心の左右に 5 画素）の外になる点（中心から 40 点の所）
    let scale = {
        let s = st(&h);
        s.view
            .view(canvas_rect(&h), s.doc.width(), s.doc.height())
            .pixel_size()
    };
    let outside_x = inside_x + (40.0 / scale).ceil() as u32;
    select_rect(&mut h, inside_x as i64 - 5, 0, inside_x as i64 + 5, 256);
    key(&h, Key::J, Modifiers::COMMAND);
    h.run();
    assert_eq!(st(&h).doc.layers().len(), 2);
    let new = st(&h).selected_layer.unwrap();
    let surface = |h: &H, x: u32| {
        st(h)
            .doc
            .layer(new)
            .unwrap()
            .surface(yolu_app::engine::Channel::Color)
            .and_then(|s| s.pixel(x, c_row(h)).ok())
            .map_or(0, |p| p.a)
    };
    assert_eq!(surface(&h, inside_x), 255, "選択範囲の中は写る");
    assert_eq!(surface(&h, outside_x), 0, "外は写らない（塗ってあっても）");
    assert!(st(&h).doc.selection().is_some(), "選択範囲は残る");
    // Delete: 選んでいるレイヤーの選択範囲の中だけを消す（下のレイヤーはそのまま）
    let below = st(&h).doc.layers()[0].id();
    h.state_mut().state.selected_layer = Some(below);
    h.run();
    let before = st(&h).doc.undo_count();
    key(&h, Key::Delete, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).doc.undo_count(), before + 1);
    let a = |x: u32| {
        st(&h)
            .doc
            .layer(below)
            .unwrap()
            .surface(yolu_app::engine::Channel::Color)
            .and_then(|s| s.pixel(x, c_row(&h)).ok())
            .map_or(0, |p| p.a)
    };
    assert_eq!(a(inside_x), 0, "選択範囲の中は消えた");
    assert_eq!(a(outside_x), 255, "外は残る");
}

/// 画面の中心の下にある、文書の列。
/// 選択範囲があるあいだの Ctrl+J は「コピーして新しいレイヤー」。レイヤーのメニューの「複製」は、そのあいだ Ctrl+J を出さない（表示と動作を合わせる）。
#[test]
fn the_layer_menu_shows_ctrl_j_for_duplicate_only_while_there_is_no_selection() {
    use yolu_app::state::AppState;
    use yolu_app::ui::menu::Entry;
    let shortcut_of = |s: &AppState, name: &str| -> Option<Option<String>> {
        yolu_app::shell::menu_entries(s, 2)
            .into_iter()
            .find_map(|e| match e {
                Entry::Item {
                    label, shortcut, ..
                } if label == name => Some(shortcut),
                _ => None,
            })
    };
    for lang in Lang::ALL {
        let mut s = AppState::new_in(64, 64, lang);
        let duplicate = lang.pick("複製", "Duplicate");
        assert_eq!(
            shortcut_of(&s, duplicate),
            Some(Some("Ctrl+J".to_owned())),
            "{lang:?}: 選択範囲が無ければ複製が Ctrl+J"
        );
        s.apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
            x0: 4,
            y0: 4,
            x1: 20,
            y1: 20,
            mode: SelectionCombine::Replace,
        })));
        assert_eq!(
            shortcut_of(&s, duplicate),
            Some(None),
            "{lang:?}: 選択範囲があるあいだは複製に Ctrl+J を出さない"
        );
        s.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
        assert_eq!(
            shortcut_of(&s, duplicate),
            Some(Some("Ctrl+J".to_owned())),
            "{lang:?}: 外せば戻る"
        );
    }
}

fn canvas_pixel_xy(h: &H, dx: f32) -> i64 {
    let s = st(h);
    let view = s.view.view(canvas_rect(h), s.doc.width(), s.doc.height());
    let (x, _) = view.to_canvas(canvas_rect(h).center() + vec2(dx, 0.0));
    x.floor() as i64
}

/// 画面の中心の下にある、文書の行。
fn c_row(h: &H) -> u32 {
    let s = st(h);
    let view = s.view.view(canvas_rect(h), s.doc.width(), s.doc.height());
    let (_, y) = view.to_canvas(canvas_rect(h).center());
    y.floor() as u32
}

#[test]
fn the_bar_speaks_english_and_has_no_japanese_in_it() {
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.lang = Lang::En;
    select_rect(&mut h, 100, 100, 140, 140);
    for label in [
        "Deselect (Ctrl+D / Esc)",
        "Invert Selection (Ctrl+Shift+I)",
        "Grow Selection…",
        "Shrink Selection…",
        "Fill with the Paint Color",
        "Erase Selection (Delete)",
        "Copy to a New Layer (Ctrl+J)",
        "Make the Selection a Layer Mask",
        "Remember Selection…",
        "Move the Button Bar",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label(DESELECT).is_none());
    h.snapshot("selection_bar_english");
}

#[test]
fn the_remember_button_opens_the_remembered_selections_window_and_a_name_can_be_saved() {
    let mut h = app(1280.0, 800.0, 256);
    select_rect(&mut h, 80, 80, 140, 120);
    assert!(st(&h).sel.saved_window.is_none());
    click_label(&mut h, REMEMBER);
    h.run();
    assert!(
        st(&h).sel.saved_window.is_some(),
        "帯のボタンでウィンドウが開く"
    );
    h.get_by_label("覚える").click();
    h.run();
    assert_eq!(st(&h).saved_selections().len(), 1);
    assert_eq!(st(&h).saved_selections()[0].name, "選択範囲 1");
    // 描いている間は押せない（帯ごと隠れる）。読むだけのセットでは押せない
    h.state_mut().state.sel.saved_window = None;
    let index = st(&h).sets.current_index();
    h.state_mut().state.sets.get_mut(index).unwrap().read_only = Some("試験".into());
    h.run();
    assert!(!bar_shown(&h), "読むだけのセットでは帯を出さない");
}
