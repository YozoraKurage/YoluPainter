//! メニューの入れ子（サブメニュー）とツールチップの試験: 乗せると右に開き、押しても開き、外へ出ると閉じる。画面の端では左へ開く。
//! 押せない項目はラベルに理由を続けず、乗せるとツールチップに出る。キーは奥の段が受け、Esc と ← は 1 段ずつ閉じる。
mod common;

use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::ui::menu::{self, Entry, PopupOutcome, PopupState, Side};
use yolu_app::YoluApp;

#[derive(Default)]
struct Bench {
    ready: bool,
    /// 開く（元の矩形）。次のフレームで開く。
    open_at: Option<Rect>,
    popup: Option<PopupState>,
    chosen: Vec<&'static str>,
    steps: Vec<i32>,
    closed: usize,
}

fn entries() -> Vec<Entry<&'static str>> {
    vec![
        Entry::item("Alpha", "alpha"),
        Entry::submenu(
            "Sub",
            vec![
                Entry::item("Sub One", "s1"),
                Entry::item("Sub Two", "s2")
                    .enabled(false)
                    .tooltip("Not now"),
                Entry::Separator,
                Entry::submenu("Deep", vec![Entry::item("Deep One", "d1")]),
            ],
        ),
        Entry::item("Beta", "beta")
            .enabled(false)
            .tooltip("Reason for Beta"),
        Entry::item("Gamma", "gamma"),
        Entry::submenu("Closed Sub", vec![Entry::item("Never", "never")])
            .enabled(false)
            .tooltip("Reason for Closed Sub"),
    ]
}

fn draw(ui: &mut egui::Ui, b: &mut Bench) {
    if !b.ready {
        YoluApp::setup(ui.ctx());
        b.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    if let Some(anchor) = b.open_at.take() {
        b.popup = Some(PopupState::new(ui.ctx(), anchor));
    }
    if let Some(state) = &mut b.popup {
        match menu::show(
            ui.ctx(),
            egui::Id::new("bench.popup"),
            state,
            &entries(),
            &[],
        ) {
            PopupOutcome::Open => {}
            PopupOutcome::Chosen(a) => {
                b.chosen.push(a);
                b.popup = None;
            }
            PopupOutcome::Close => {
                b.closed += 1;
                b.popup = None;
            }
            PopupOutcome::Step(d) => {
                b.steps.push(d);
                b.popup = None;
            }
        }
    }
}

fn bench(anchor: Rect) -> Harness<'static, Bench> {
    let mut h = Harness::builder()
        .with_size(vec2(640.0, 520.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(common::render_options())
        .wgpu()
        .build_ui_state(draw, Bench::default());
    h.run();
    // ポインタの最初の動きは、前の位置が無いので動きの量が 0。先に 1 度、メニューの外へ置いておく
    h.event(Event::PointerMoved(pos2(630.0, 510.0)));
    h.step();
    h.state_mut().open_at = Some(anchor);
    h.run();
    h
}

fn left_bench() -> Harness<'static, Bench> {
    bench(Rect::from_min_size(pos2(20.0, 40.0), vec2(0.0, 0.0)))
}

fn hover(h: &mut Harness<'_, Bench>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.step();
}

fn row(h: &Harness<'_, Bench>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

fn depth(h: &Harness<'_, Bench>) -> usize {
    h.state().popup.as_ref().map_or(0, |p| p.open_depth())
}

fn click(h: &mut Harness<'_, Bench>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
}

fn key(h: &mut Harness<'_, Bench>, key: Key) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.step();
}

#[test]
fn a_submenu_opens_to_the_right_on_hover_and_closes_when_the_pointer_moves_to_another_row() {
    let mut h = left_bench();
    assert_eq!(depth(&h), 0);
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    assert_eq!(depth(&h), 1, "乗せると開く");
    let popup = h.state().popup.as_ref().unwrap();
    assert_eq!(popup.open_rows(), vec![1]);
    let body = popup.sub_rects()[0];
    assert!(
        body.left() > sub.right(),
        "親の行の右に開く: {body:?} {sub:?}"
    );
    assert!(
        (body.top() - sub.top()).abs() < 12.0,
        "先頭の項目が開いた行の高さにそろう: {body:?} {sub:?}"
    );
    // 開いた入れ子の項目が見える
    assert!(h.query_by_label("Sub One").is_some());
    // 別の行（入れ子でない項目）へ移ると閉じる
    let gamma = row(&h, "Gamma");
    hover(&mut h, gamma.center());
    assert_eq!(depth(&h), 0, "別の行へ移ると閉じる");
    assert!(h.query_by_label("Sub One").is_none());
    assert!(h.state().popup.is_some(), "メニューは開いたまま");
}

#[test]
fn leaving_the_menus_closes_the_submenus_and_a_press_outside_closes_the_popup() {
    let mut h = left_bench();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    assert_eq!(depth(&h), 1);
    // 親の行から入れ子の中へ横切っても閉じない
    let one = row(&h, "Sub One");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    assert_eq!(depth(&h), 1);
    hover(&mut h, one.center());
    assert_eq!(depth(&h), 1, "入れ子の中にいるあいだは開いたまま");
    // どのメニューの上でもない所へ出ると、入れ子は閉じる（メニュー自体は残る）
    hover(&mut h, pos2(630.0, 500.0));
    assert_eq!(depth(&h), 0, "外へ出ると閉じる");
    assert!(h.state().popup.is_some());
    // 外を押せばメニュー全体が閉じる
    click(&mut h, pos2(630.0, 500.0));
    assert!(h.state().popup.is_none());
    assert_eq!(h.state().closed, 1);
    assert!(h.state().chosen.is_empty());
}

#[test]
fn pressing_a_submenu_row_opens_it_and_neither_chooses_nor_closes() {
    let mut h = left_bench();
    // 押すと開く（乗せたときと同じ。選ぶ項目ではないので、選ばれず、閉じもしない）
    let sub = row(&h, "Sub");
    click(&mut h, sub.center());
    assert_eq!(depth(&h), 1);
    assert!(h.state().popup.is_some());
    assert!(h.state().chosen.is_empty(), "入れ子の行は選ぶ項目ではない");
    // もう一度押しても開いたまま
    click(&mut h, sub.center());
    assert_eq!(depth(&h), 1);
}

#[test]
fn a_second_level_opens_inside_the_first_and_its_item_is_chosen() {
    let mut h = left_bench();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let deep = row(&h, "Deep");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    hover(&mut h, deep.center());
    assert_eq!(depth(&h), 2, "2 段目も開く");
    let rects = h.state().popup.as_ref().unwrap().sub_rects();
    assert!(
        rects[1].left() > rects[0].center().x,
        "2 段目は 1 段目の右: {rects:?}"
    );
    let one = row(&h, "Deep One");
    click(&mut h, one.center());
    assert_eq!(h.state().chosen, vec!["d1"]);
    assert!(h.state().popup.is_none(), "選んだらメニュー全体が閉じる");
    // 1 段目の項目も選べる
    h.state_mut().open_at = Some(Rect::from_min_size(pos2(20.0, 40.0), vec2(0.0, 0.0)));
    h.run();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let one = row(&h, "Sub One");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    click(&mut h, one.center());
    assert_eq!(h.state().chosen, vec!["d1", "s1"]);
}

#[test]
fn a_submenu_near_the_right_edge_opens_to_the_left() {
    let mut h = bench(Rect::from_min_size(pos2(620.0, 40.0), vec2(0.0, 0.0)));
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    assert_eq!(depth(&h), 1);
    let body = h.state().popup.as_ref().unwrap().sub_rects()[0];
    assert!(
        body.right() <= sub.left() + 1.0,
        "画面の端では親の左に開く: {body:?} {sub:?}"
    );
    assert!(body.left() >= 0.0, "画面の中: {body:?}");
    // 左に開いても、横切って入れ子の項目へ届く
    let one = row(&h, "Sub One");
    hover(&mut h, pos2(sub.left() + 2.0, sub.center().y));
    hover(&mut h, one.center());
    assert_eq!(depth(&h), 1);
    click(&mut h, one.center());
    assert_eq!(h.state().chosen, vec!["s1"]);
}

#[test]
fn a_submenu_near_the_bottom_edge_stays_inside_the_screen() {
    let mut h = bench(Rect::from_min_size(pos2(20.0, 330.0), vec2(0.0, 0.0)));
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let body = h.state().popup.as_ref().unwrap().sub_rects()[0];
    assert!(body.bottom() <= 520.0 && body.top() >= 0.0, "{body:?}");
}

#[test]
fn a_disabled_item_keeps_its_plain_label_and_says_why_in_a_tooltip() {
    let mut h = left_bench();
    // ラベルは名前だけ（理由を続けない）
    assert!(h.query_by_label("Beta").is_some(), "ラベルは名前だけ");
    assert!(h.query_by_label("Reason for Beta").is_none());
    let beta = row(&h, "Beta");
    h.event(Event::PointerMoved(beta.center()));
    for _ in 0..60 {
        h.step();
    }
    assert!(
        h.query_by_label("Reason for Beta").is_some(),
        "押せない項目に乗せると理由が出る"
    );
    assert!(h.state().chosen.is_empty());
    // 押せない項目を押しても選ばれない・閉じない
    click(&mut h, beta.center());
    assert!(h.state().chosen.is_empty());
    assert!(h.state().popup.is_some());
    // 押せない入れ子の行も、開かずに理由を出す
    let closed_sub = row(&h, "Closed Sub");
    h.event(Event::PointerMoved(closed_sub.center()));
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(depth(&h), 0, "押せない入れ子は開かない");
    assert!(h.query_by_label("Reason for Closed Sub").is_some());
    click(&mut h, closed_sub.center());
    assert_eq!(depth(&h), 0);
    assert!(h.state().popup.is_some());
}

#[test]
fn a_disabled_item_inside_a_submenu_shows_its_reason_too() {
    let mut h = left_bench();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let two = row(&h, "Sub Two");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    h.event(Event::PointerMoved(two.center()));
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(depth(&h), 1);
    assert!(h.query_by_label("Not now").is_some());
}

#[test]
fn the_keys_open_walk_and_close_the_submenus_one_level_at_a_time() {
    let mut h = left_bench();
    // ↓ で最初の選べる行（Alpha）、もう一度で入れ子の行（Sub）。→ で開いて先頭の項目を選ぶ
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowDown);
    assert_eq!(depth(&h), 0, "行を選んだだけでは開かない");
    key(&mut h, Key::ArrowRight);
    assert_eq!(depth(&h), 1, "→ で入れ子が開く");
    assert!(h.state().popup.is_some());
    assert!(
        h.state().steps.is_empty(),
        "入れ子を開く → はメニューバーの隣へ進まない"
    );
    // 入れ子の中を ↓（押せない項目 Sub Two は飛ばし、区切りも飛ばして Deep）
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowRight);
    assert_eq!(depth(&h), 2, "2 段目も → で開く");
    // Enter で 2 段目の先頭を選ぶ
    key(&mut h, Key::Enter);
    assert_eq!(h.state().chosen, vec!["d1"]);
    assert!(h.state().popup.is_none());

    // Esc は 1 段ずつ閉じる（最後の Esc でメニューを閉じる）
    h.state_mut().open_at = Some(Rect::from_min_size(pos2(20.0, 40.0), vec2(0.0, 0.0)));
    h.run();
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowRight);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowRight);
    assert_eq!(depth(&h), 2);
    key(&mut h, Key::Escape);
    assert_eq!(depth(&h), 1, "Esc は奥の段から閉じる");
    assert!(h.state().popup.is_some());
    key(&mut h, Key::Escape);
    assert_eq!(depth(&h), 0);
    assert!(h.state().popup.is_some());
    key(&mut h, Key::Escape);
    assert!(h.state().popup.is_none(), "最後の Esc でメニューが閉じる");
    assert_eq!(h.state().closed, 1);
    assert_eq!(h.state().chosen, vec!["d1"]);

    // ← も 1 段閉じる。入れ子が無いときの ← → は、メニューバーの隣へ進む（呼ぶ側へ知らせる）
    h.state_mut().open_at = Some(Rect::from_min_size(pos2(20.0, 40.0), vec2(0.0, 0.0)));
    h.run();
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowRight);
    assert_eq!(depth(&h), 1);
    key(&mut h, Key::ArrowLeft);
    assert_eq!(depth(&h), 0, "← で入れ子を閉じる");
    assert!(h.state().popup.is_some() && h.state().steps.is_empty());
    key(&mut h, Key::ArrowLeft);
    assert_eq!(h.state().steps, vec![-1]);
}

#[test]
fn the_pointer_and_the_keys_share_the_selected_row() {
    let mut h = left_bench();
    let gamma = row(&h, "Gamma");
    hover(&mut h, gamma.center());
    // ポインタで選んだ行から、キーで次へ（下の入れ子は押せないので、Alpha へ回る）
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::Enter);
    assert_eq!(h.state().chosen, vec!["alpha"]);
}

#[test]
fn a_submenu_is_placed_beside_the_parent_row_and_flips_when_it_does_not_fit() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let parent = Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 300.0));
    let row = Rect::from_min_size(pos2(104.0, 150.0), vec2(192.0, 28.0));
    let size = vec2(216.0, 200.0);
    // 右に置く: 本体の左の端は親の右の端と重なる程度（隙間が無い）、先頭の項目は開いた行の高さ
    let (at, side) = menu::place_sub(parent, row, size, screen, Side::Right);
    assert_eq!(side, Side::Right);
    let body = at.shrink(menu::MARGIN);
    assert!(
        body.left() <= parent.right() && body.left() >= parent.right() - 4.0,
        "{body:?}"
    );
    assert!(
        (body.top() + menu::PADDING - row.top()).abs() < 0.5,
        "{body:?}"
    );
    // 右に入らないなら左へ
    let near_right = Rect::from_min_size(pos2(500.0, 100.0), vec2(200.0, 300.0));
    let (at, side) = menu::place_sub(near_right, row, size, screen, Side::Right);
    assert_eq!(side, Side::Left, "右に入らなければ左");
    let body = at.shrink(menu::MARGIN);
    assert!(
        body.right() >= near_right.left() && body.right() <= near_right.left() + 4.0,
        "{body:?}"
    );
    assert!(body.left() >= 0.0);
    // 下にはみ出すなら画面の中へ上げる
    let low_row = Rect::from_min_size(pos2(104.0, 560.0), vec2(192.0, 28.0));
    let (at, _) = menu::place_sub(parent, low_row, size, screen, Side::Right);
    assert!(at.bottom() <= 600.0 && at.top() >= 0.0, "{at:?}");
}

#[test]
fn a_child_of_a_folded_submenu_tries_the_same_side_first() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let row = Rect::from_min_size(pos2(404.0, 150.0), vec2(192.0, 28.0));
    let size = vec2(216.0, 200.0);
    // 親が左へ折り返した（本体が画面の中ほど）。右にも入るが、子はまず左へ（右へ戻ると祖父の上に重なる）
    let parent = Rect::from_min_size(pos2(400.0, 100.0), vec2(200.0, 300.0));
    let (at, side) = menu::place_sub(parent, row, size, screen, Side::Left);
    let body = at.shrink(menu::MARGIN);
    assert_eq!(side, Side::Left);
    assert!(
        body.right() >= parent.left() && body.right() <= parent.left() + 4.0,
        "{body:?}"
    );
    // 左に入らなければ反対側へ
    let tight = Rect::from_min_size(pos2(120.0, 100.0), vec2(200.0, 300.0));
    let (at, side) = menu::place_sub(tight, row, size, screen, Side::Left);
    let body = at.shrink(menu::MARGIN);
    assert_eq!(side, Side::Right, "左に入らず右に入るなら右");
    assert!(body.left() <= tight.right() && body.left() >= tight.right() - 4.0);
    // どちらにも入らない幅の画面では、左を優先した向きのまま画面の左の端へ寄せる
    let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 600.0));
    let middle = Rect::from_min_size(pos2(40.0, 100.0), vec2(200.0, 300.0));
    let (at, side) = menu::place_sub(middle, row, size, narrow, Side::Left);
    assert_eq!(side, Side::Left);
    assert!(at.shrink(menu::MARGIN).left() >= 0.0, "{at:?}");
}

#[test]
fn two_levels_opened_from_a_menu_at_the_right_edge_fold_left_without_covering_each_other() {
    let mut h = bench(Rect::from_min_size(pos2(620.0, 40.0), vec2(0.0, 0.0)));
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    assert_eq!(depth(&h), 1);
    let deep = row(&h, "Deep");
    hover(&mut h, pos2(sub.left() + 2.0, sub.center().y));
    hover(&mut h, deep.center());
    assert_eq!(depth(&h), 2, "2 段目も開く");
    let rects = h.state().popup.as_ref().unwrap().sub_rects();
    assert!(
        rects[0].right() <= sub.left() + 4.0,
        "1 段目は根の左: {rects:?} {sub:?}"
    );
    assert!(
        rects[1].right() <= rects[0].left() + 4.0,
        "2 段目も 1 段目の左へ折り返し、根を隠さない: {rects:?}"
    );
    assert!(rects[1].left() >= 0.0, "画面の中: {rects:?}");
    // 根の行（Alpha）は、2 段とも重ならずに見えている
    let alpha = row(&h, "Alpha");
    assert!(
        !rects[1].intersects(alpha) && !rects[0].shrink(4.0).intersects(alpha),
        "{rects:?} {alpha:?}"
    );
    // 左へ折り返した 2 段目の項目へ届いて選べる
    hover(&mut h, pos2(deep.left() + 2.0, deep.center().y));
    let one = row(&h, "Deep One");
    hover(&mut h, one.center());
    assert_eq!(depth(&h), 2);
    click(&mut h, one.center());
    assert_eq!(h.state().chosen, vec!["d1"]);
}

#[test]
fn leaves_lists_the_items_inside_submenus_but_not_the_submenu_rows() {
    let list = entries();
    let labels: Vec<&str> = menu::leaves(&list)
        .into_iter()
        .filter_map(|e| e.label())
        .collect();
    assert_eq!(
        labels,
        ["Alpha", "Sub One", "Sub Two", "Deep One", "Beta", "Gamma", "Never"]
    );
}

/// 開いているメニュー全体（親と開いている入れ子）を切り抜いて撮る。
fn snapshot_popup(h: &mut Harness<'_, Bench>, name: &str) {
    let rect = h
        .state()
        .popup
        .as_ref()
        .expect("開いている")
        .rect
        .expand(2.0);
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().max(0.0).floor() as u32,
        rect.top().max(0.0).floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_a_submenu_with_a_disabled_item_and_its_reason() {
    let mut h = left_bench();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let two = row(&h, "Sub Two");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    h.event(Event::PointerMoved(two.center()));
    for _ in 0..60 {
        h.step();
    }
    assert!(h.query_by_label("Not now").is_some());
    snapshot_popup(&mut h, "menus_submenu_reason");
}

#[test]
fn snapshot_two_levels_of_submenus() {
    let mut h = left_bench();
    let sub = row(&h, "Sub");
    hover(&mut h, sub.center());
    let deep = row(&h, "Deep");
    hover(&mut h, pos2(sub.right() - 2.0, sub.center().y));
    hover(&mut h, deep.center());
    assert_eq!(depth(&h), 2);
    snapshot_popup(&mut h, "menus_submenu_two_levels");
}
