//! Windows の上の帯（OS のタイトルバーを外し、メニューの帯の右端に小さな最小化・最大化・閉じる）と、枠なしのウィンドウの縁。
//! ウィンドウの枠を外すのは Windows だけ（`main.rs`）だが、帯と縁の中身は OS に依らないので、Linux でも `set_custom_frame(true)` で
//! Windows の帯を描いて確かめる。ウィンドウへ送る頼み（`ViewportCommand`）は、フレームごとの出力から読む。
use crate::common;

use common::*;
use egui::{
    pos2, vec2, CursorIcon, Event, PointerButton, Pos2, Rect, ResizeDirection, ViewportCommand,
};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::lang::Lang;
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, PopupKind};
use yolu_app::titlebar::{self, Button};
use yolu_app::ui::theme as t;
use yolu_app::{shell, Tab, YoluApp};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 700.0;

/// Windows の帯のウィンドウ（自前の枠）。
fn windows_app(width: f32, height: f32) -> Harness<'static, YoluApp> {
    let mut h = app(width, height, 64);
    h.state_mut().set_custom_frame(true);
    h.run();
    h
}

fn set_maximized(h: &mut Harness<'_, YoluApp>, on: bool) {
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("root")
        .maximized = Some(on);
    h.step();
}

/// 直前のフレームがウィンドウへ送った頼み。
fn commands(h: &Harness<'_, YoluApp>) -> Vec<ViewportCommand> {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

/// 入力を 1 つずつ 1 フレームで渡し、そのフレームごとにウィンドウへ送った頼みを集める。
fn play(h: &mut Harness<'_, YoluApp>, events: Vec<Event>) -> Vec<ViewportCommand> {
    let mut sent = Vec::new();
    for event in events {
        h.event(event);
        h.step();
        sent.extend(commands(h));
    }
    sent
}

/// 入力をグループごとに 1 フレームで渡し、そのフレームごとにウィンドウへ送った頼みを集める（winit は、ペン・指の接触の始まりの Touch と、その押しの代わりの入力を、
/// 同じ入力のまとまりに入れる）。
fn play_groups(h: &mut Harness<'_, YoluApp>, groups: Vec<Vec<Event>>) -> Vec<ViewportCommand> {
    let mut sent = Vec::new();
    for group in groups {
        // `Harness::event` は 1 つごとに 1 フレーム進めるので、まとめて入れる
        h.input_mut().events.extend(group);
        h.step();
        sent.extend(commands(h));
    }
    sent
}

/// ペン・指の押しと引き（離しまで）。
fn touch_drag(at: Pos2, by: egui::Vec2) -> Vec<Vec<Event>> {
    vec![
        vec![touch_start(at), pointer(at), button(at, true)],
        vec![pointer(at + by * 0.5)],
        vec![pointer(at + by)],
        vec![button(at + by, false)],
    ]
}

fn pointer(at: Pos2) -> Event {
    Event::PointerMoved(at)
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Alt を押しながらの左ボタン（3D ビューでは回す操作。モデルに当たらなくても始まるので、押しがビューに渡ったかを確かめられる）。
fn alt_button(at: Pos2, pressed: bool) -> Event {
    let modifiers = egui::Modifiers {
        alt: true,
        ..egui::Modifiers::NONE
    };
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    }
}

/// ペンの 1 点（位置は物理の画素。試験のウィンドウは 1 点 = 1 画素）。
fn pen_point(at: Pos2, contact: bool, pressure: f32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    }
}

/// `at` を押して、`by` だけ引いて、離す。
fn press_and_drag(h: &mut Harness<'_, YoluApp>, at: Pos2, by: egui::Vec2) -> Vec<ViewportCommand> {
    play(
        h,
        vec![
            pointer(at),
            button(at, true),
            pointer(at + by * 0.5),
            pointer(at + by),
            button(at + by, false),
        ],
    )
}

/// `at` をダブルクリックする。
fn double_click(h: &mut Harness<'_, YoluApp>, at: Pos2) -> Vec<ViewportCommand> {
    play(
        h,
        vec![
            pointer(at),
            button(at, true),
            button(at, false),
            button(at, true),
            button(at, false),
        ],
    )
}

fn single_click(h: &mut Harness<'_, YoluApp>, at: Pos2) -> Vec<ViewportCommand> {
    play(h, vec![pointer(at), button(at, true), button(at, false)])
}

fn count(commands: &[ViewportCommand], is: impl Fn(&ViewportCommand) -> bool) -> usize {
    commands.iter().filter(|c| is(c)).count()
}

fn starts_drag(c: &ViewportCommand) -> bool {
    matches!(c, ViewportCommand::StartDrag)
}

fn toggles_maximized(c: &ViewportCommand) -> bool {
    matches!(c, ViewportCommand::Maximized(_))
}

fn begins_resize(c: &ViewportCommand) -> Option<ResizeDirection> {
    match c {
        ViewportCommand::BeginResize(direction) => Some(*direction),
        _ => None,
    }
}

fn resizes(commands: &[ViewportCommand]) -> Vec<ResizeDirection> {
    commands.iter().filter_map(begins_resize).collect()
}

/// 入口の印の矩形（名前はツールチップの文）。
fn link_rect(h: &Harness<'_, YoluApp>) -> Rect {
    let tip = h.state().state.link.tooltip(h.state().state.lang);
    h.get_by_label(&tip).rect()
}

fn last_menu_title(lang: Lang) -> &'static str {
    shell::menu_titles(lang)[shell::menu_titles(lang).len() - 1]
}

/// 帯の何も無い所（メニューの見出しの右、クラッシュ・Live Link の印の左）の点。
fn empty_bar_point(h: &Harness<'_, YoluApp>) -> Pos2 {
    let lang = h.state().state.lang;
    let from = menu_title(h, last_menu_title(lang)).right();
    let to = link_rect(h).left() - 30.0;
    assert!(to - from > 60.0, "何も無い所が足りない: {from}..{to}");
    pos2(from + 40.0, t::MENU_BAR_HEIGHT / 2.0)
}

fn top_shot(h: &mut Harness<'_, YoluApp>, name: &str, results: &mut SnapshotResults) {
    // 直前に押した所のポインタが絵に残らないように（乗せた絵は、乗せてから撮る）
    let image = h.render().expect("描画");
    let cropped =
        image::imageops::crop_imm(&image, 0, 0, image.width(), t::MENU_BAR_HEIGHT as u32 + 2)
            .to_image();
    results.add(egui_kittest::try_image_snapshot(&cropped, name));
}

// ───────── 帯の絵と寸法 ─────────

/// 絵: 帯の右端のボタン（日英・最大化中・閉じるに乗せた）。名前・Live Link の印・保存の印はボタンの左に収まる。
#[test]
fn the_windows_bar_looks_right() {
    let mut results = SnapshotResults::new();
    for (lang, suffix) in [(Lang::Ja, ""), (Lang::En, "_english")] {
        let mut h = windows_app(WIDTH, HEIGHT);
        h.state_mut().state.lang = lang;
        h.state_mut().state.project_name = lang.pick("名称未設定", "Untitled").into();
        h.state_mut().state.modified = true;
        h.run();
        h.event(Event::PointerGone);
        h.step();
        top_shot(&mut h, &format!("titlebar_windows{suffix}"), &mut results);
        if lang == Lang::Ja {
            // 最大化中（元に戻すの印）
            set_maximized(&mut h, true);
            top_shot(&mut h, "titlebar_windows_maximized", &mut results);
            set_maximized(&mut h, false);
            // 閉じるに乗せる（赤）、最小化に乗せる
            let [min, _, close] = titlebar::button_rects(Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(WIDTH, t::MENU_BAR_HEIGHT),
            ));
            play(&mut h, vec![pointer(close.center())]);
            top_shot(&mut h, "titlebar_windows_close_hover", &mut results);
            play(&mut h, vec![pointer(min.center())]);
            top_shot(&mut h, "titlebar_windows_minimize_hover", &mut results);
        }
    }
}

/// 枠を外していないウィンドウ（Mac・Linux）は、今の帯のまま: ボタンも縁も無い。
#[test]
fn without_the_custom_frame_the_bar_has_no_buttons_and_the_edges_do_nothing() {
    let mut h = app(WIDTH, HEIGHT, 64);
    for lang in Lang::ALL {
        h.state_mut().state.lang = lang;
        h.run();
        for button in Button::ALL {
            for maximized in [false, true] {
                assert!(
                    h.query_by_label(button.name(lang, maximized)).is_none(),
                    "{lang:?} {button:?}: ボタンがある"
                );
            }
        }
    }
    let at = pos2(WIDTH - 1.0, HEIGHT / 2.0);
    let sent = play(
        &mut h,
        vec![pointer(at), button(at, true), button(at, false)],
    );
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    assert_ne!(
        h.output().platform_output.cursor_icon,
        CursorIcon::ResizeHorizontal
    );
    // 帯を引いても動かさず、ダブルクリックでも最大化しない
    let empty = pos2(600.0, t::MENU_BAR_HEIGHT / 2.0);
    let sent = press_and_drag(&mut h, empty, vec2(30.0, 0.0));
    assert_eq!(count(&sent, starts_drag), 0, "{sent:?}");
    let sent = double_click(&mut h, empty);
    assert_eq!(count(&sent, toggles_maximized), 0, "{sent:?}");
}

/// 主のウィンドウの帯も同じ: ペン・指の押しの引きは OS の移動の輪（`StartDrag`）に渡さず、アプリの側で動かす。マウスの引きは今までどおり `StartDrag`（見張りに伝える）。
#[test]
fn a_pen_drag_on_the_empty_bar_is_moved_by_the_app_but_a_mouse_drag_still_starts_the_os_move() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let mover = TestMover::new(true);
    h.state_mut().set_pen_mover(Some(mover.clone()));
    let empty = empty_bar_point(&h);
    // ペン: 触れた入力と押しを同じ入力のまとまりに入れて、引く
    let by = vec2(60.0, 20.0);
    let sent = play_groups(&mut h, touch_drag(empty, by));
    assert_eq!(mover.calls(), 1, "引き始めでアプリの手を 1 度呼ぶ");
    assert_eq!(count(&sent, starts_drag), 0, "{sent:?}");
    assert_eq!(mover.handed(), 0);
    // 動かせなくても（点が途絶えた・指）、ペン・指の押しは輪に渡さない
    mover.answer(false);
    let sent = play_groups(&mut h, touch_drag(empty, by));
    assert_eq!(mover.calls(), 2);
    assert_eq!(count(&sent, starts_drag), 0, "{sent:?}");
    // マウス: 手を呼ばず、StartDrag を出して見張りに伝える
    let sent = press_and_drag(&mut h, empty, by);
    assert_eq!(mover.calls(), 2);
    assert_eq!(count(&sent, starts_drag), 1, "{sent:?}");
    assert_eq!(mover.handed(), 1);
    // 手の無い受け口（Windows 以外）は、ペンの接触でも今までどおり StartDrag
    h.state_mut().set_pen_mover(None);
    let sent = play_groups(&mut h, touch_drag(empty, by));
    assert_eq!(count(&sent, starts_drag), 1, "{sent:?}");
}

/// ボタンは帯の右端に 3 つ並び、帯の下の線の上までの高さで、名前・印は左に収まる（細いウィンドウでも）。
#[test]
fn the_buttons_sit_at_the_right_end_and_the_name_and_marks_stay_left_of_them() {
    for lang in Lang::ALL {
        for width in [960.0, 1280.0] {
            let mut h = windows_app(width, HEIGHT);
            h.state_mut().state.lang = lang;
            h.state_mut().state.project_name = "とても長いプロジェクトの名前を付けた作品".repeat(3);
            h.state_mut().state.modified = true;
            h.run();
            let rects = Button::ALL.map(|b| h.get_by_label(b.name(lang, false)).rect());
            for (i, r) in rects.iter().enumerate() {
                assert_eq!(r.width(), titlebar::BUTTON_WIDTH, "{lang:?} {width}: {i}");
                assert!(
                    r.top() <= 0.5 && r.bottom() <= t::MENU_BAR_HEIGHT,
                    "{lang:?} {width}: {r:?}"
                );
            }
            assert_eq!(
                rects[2].right(),
                width,
                "{lang:?} {width}: 閉じるはウィンドウの右端"
            );
            assert!(rects[0].left() < rects[1].left() && rects[1].left() < rects[2].left());
            let icon = link_rect(&h);
            assert!(
                icon.right() < rects[0].left(),
                "{lang:?} {width}: Live Link の印 {icon:?} がボタンに重なる"
            );
            let last = menu_title(&h, last_menu_title(lang));
            assert!(
                icon.left() >= last.right(),
                "{lang:?} {width}: 印がメニューの見出しに重なる"
            );
            // 名前の字（明るい画素）が、ボタンの列に入り込まない
            let image = h.render().expect("描画");
            for y in 2..(t::MENU_BAR_HEIGHT as u32 - 2) {
                let edge = rects[0].left() as u32;
                for x in edge - 6..edge {
                    let p = image.get_pixel(x, y).0;
                    assert!(
                        p[0] as u32 + p[1] as u32 + p[2] as u32 <= 330,
                        "{lang:?} {width}: 名前の字がボタンの左の隙間 ({x},{y}) まで来る"
                    );
                }
            }
        }
    }
}

/// 最大化中は最大化のボタンが「元に戻す」になる（名前・アイコン）。ボタンの名前は日英で出る。
#[test]
fn the_maximize_button_becomes_restore_while_maximized() {
    for lang in Lang::ALL {
        let mut h = windows_app(WIDTH, HEIGHT);
        h.state_mut().state.lang = lang;
        h.run();
        h.get_by_label(lang.pick("最小化", "Minimize"));
        h.get_by_label(lang.pick("最大化", "Maximize"));
        h.get_by_label(lang.pick("閉じる", "Close"));
        assert!(h.query_by_label(lang.pick("元に戻す", "Restore")).is_none());
        set_maximized(&mut h, true);
        h.get_by_label(lang.pick("元に戻す", "Restore"));
        assert!(h.query_by_label(lang.pick("最大化", "Maximize")).is_none());
    }
}

/// ツールチップは名前だけ（説明は付けない）。
#[test]
fn the_button_tooltips_are_only_the_names() {
    let mut h = windows_app(WIDTH, HEIGHT);
    for button in Button::ALL {
        let rect = h.get_by_label(button.name(Lang::Ja, false)).rect();
        hover_and_wait(&mut h, rect.center());
        // 名前の部品のほかに、同じ名前のツールチップが 1 つだけ出る（長い文は出ない）
        let found = h.query_all_by_label(button.name(Lang::Ja, false)).count();
        assert!(found >= 2, "{button:?}: ツールチップが出ない（{found}）");
        move_to(&h, pos2(WIDTH / 2.0, 300.0));
        h.run();
    }
}

// ───────── ボタンがウィンドウへ頼むこと ─────────

#[test]
fn minimize_and_maximize_and_restore_send_their_commands() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = h.get_by_label("最小化").rect().center();
    let sent = single_click(&mut h, at);
    assert!(
        sent.iter()
            .any(|c| matches!(c, ViewportCommand::Minimized(true))),
        "{sent:?}"
    );
    assert_eq!(count(&sent, toggles_maximized), 0);

    let at = h.get_by_label("最大化").rect().center();
    let sent = single_click(&mut h, at);
    assert!(
        sent.iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(true))),
        "{sent:?}"
    );

    // 最大化中は、同じボタンが元に戻す
    set_maximized(&mut h, true);
    let at = h.get_by_label("元に戻す").rect().center();
    let sent = single_click(&mut h, at);
    assert!(
        sent.iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(false))),
        "{sent:?}"
    );
}

/// 閉じるは、メニューの「終了」と同じ道（`Action::Quit`。保存していない変更の確かめは、終了の処理がウィンドウを開く実際のアプリで行う）。
#[test]
fn close_goes_the_same_way_as_the_quit_menu_item() {
    for modified in [false, true] {
        // メニューの「終了」を選んだとき
        let mut via_menu = windows_app(WIDTH, HEIGHT);
        via_menu.state_mut().state.modified = modified;
        via_menu.state_mut().state.apply(Action::Quit);
        via_menu.step();
        let menu = (
            via_menu.state().state.quit,
            commands(&via_menu)
                .iter()
                .any(|c| matches!(c, ViewportCommand::Close)),
        );
        // 閉じるのボタン
        let mut h = windows_app(WIDTH, HEIGHT);
        h.state_mut().state.modified = modified;
        h.run();
        let at = h.get_by_label("閉じる").rect().center();
        let sent = single_click(&mut h, at);
        let button = (
            h.state().state.quit,
            sent.iter().any(|c| matches!(c, ViewportCommand::Close)),
        );
        assert_eq!(button, menu, "modified={modified}");
        assert!(button.0 && button.1, "modified={modified}: 終了の頼みになり、ウィンドウを閉じる頼みが出る（試験のウィンドウは確かめを開かない）");
    }
}

// ───────── 動かす・最大化 ─────────

#[test]
fn dragging_the_empty_bar_starts_a_window_drag() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = empty_bar_point(&h);
    let sent = press_and_drag(&mut h, at, vec2(40.0, 6.0));
    assert_eq!(count(&sent, starts_drag), 1, "{sent:?}");
    assert_eq!(count(&sent, toggles_maximized), 0);
    // 押して離しただけ（動かさない）なら、何もしない
    let sent = single_click(&mut h, at);
    assert!(sent.is_empty(), "{sent:?}");
}

#[test]
fn double_clicking_the_empty_bar_toggles_maximized() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = empty_bar_point(&h);
    let sent = double_click(&mut h, at);
    assert!(
        sent.iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(true))),
        "{sent:?}"
    );
    assert_eq!(count(&sent, starts_drag), 0, "ダブルクリックは動かさない");
    // 最大化中は元に戻す（前のダブルクリックから離す。続けると 3 回目のクリックになる）
    set_maximized(&mut h, true);
    for _ in 0..40 {
        h.step();
    }
    let sent = double_click(&mut h, at);
    assert!(
        sent.iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(false))),
        "{sent:?}"
    );
}

/// 名前の上（右の端）の何も無い所も動かす。
#[test]
fn dragging_over_the_project_name_also_moves_the_window() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = pos2(
        titlebar::content_rect(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, t::MENU_BAR_HEIGHT)),
            true,
        )
        .right()
            - 12.0,
        12.0,
    );
    let sent = press_and_drag(&mut h, at, vec2(-30.0, 8.0));
    assert_eq!(count(&sent, starts_drag), 1, "{sent:?}");
}

/// メニューの見出し・Live Link の印を押したときは、動かさず最大化もしない（メニュー・ウィンドウはいつもどおり開く）。
#[test]
fn pressing_the_menus_and_the_link_mark_never_moves_the_window() {
    for lang in Lang::ALL {
        let mut h = windows_app(WIDTH, HEIGHT);
        h.state_mut().state.lang = lang;
        h.run();
        // メニューの見出し（引いても、ダブルクリックしても）
        for title in shell::menu_titles(lang) {
            let at = menu_title(&h, title).center();
            let sent = press_and_drag(&mut h, at, vec2(24.0, 0.0));
            assert_eq!(count(&sent, starts_drag), 0, "{lang:?} {title}: {sent:?}");
            h.state_mut().state.popup = None;
            h.run();
            let sent = double_click(&mut h, at);
            assert_eq!(
                count(&sent, toggles_maximized),
                0,
                "{lang:?} {title}: {sent:?}"
            );
            h.state_mut().state.popup = None;
            h.run();
        }
        // Live Link の印（押すとウィンドウが開く）
        let at = link_rect(&h).center();
        let sent = press_and_drag(&mut h, at, vec2(24.0, 0.0));
        assert_eq!(count(&sent, starts_drag), 0, "{lang:?} 印: {sent:?}");
        h.state_mut().state.popup = None;
        h.run();
        let sent = double_click(&mut h, at);
        assert_eq!(count(&sent, toggles_maximized), 0, "{lang:?} 印: {sent:?}");
    }
    // 押したら、メニューは開く（動かさないだけで、操作は奪わない）
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = menu_title(&h, "ファイル").center();
    single_click(&mut h, at);
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::MenuBar(0))
    ));
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = link_rect(&h).center();
    single_click(&mut h, at);
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::LiveLink)
    ));
}

/// ボタンを押したときも、動かさず最大化の切り替えを重ねない（ダブルクリックで最大化を 2 回送らない）。
#[test]
fn pressing_a_button_does_not_also_move_or_double_toggle() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = h.get_by_label("最大化").rect().center();
    let sent = double_click(&mut h, at);
    assert_eq!(count(&sent, starts_drag), 0, "{sent:?}");
    // ダブルクリック = 押して離すを 2 回 = ボタンの 2 回の押し。帯の最大化の切り替えは足さない
    assert_eq!(count(&sent, toggles_maximized), 2, "{sent:?}");
}

/// 開いているメニューの上（受け皿が覆っている間）に重なる帯の何も無い所は、動かさない。
#[test]
fn an_open_menu_covers_the_bar_so_it_does_not_start_a_drag() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = menu_title(&h, "ファイル").center();
    single_click(&mut h, at);
    assert!(h.state().state.popup.is_some());
    let empty = empty_bar_point(&h);
    let sent = press_and_drag(&mut h, empty, vec2(40.0, 0.0));
    assert_eq!(count(&sent, starts_drag), 0, "{sent:?}");
}

// ───────── ウィンドウの縁 ─────────

fn edge_cases() -> [(Pos2, ResizeDirection, CursorIcon); 8] {
    [
        (
            pos2(WIDTH - 1.0, HEIGHT / 2.0),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            pos2(1.0, HEIGHT / 2.0),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            pos2(WIDTH / 2.0, HEIGHT - 1.0),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            pos2(WIDTH / 2.0, 1.0),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            pos2(1.0, 1.0),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            pos2(WIDTH - 1.0, 1.0),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            pos2(1.0, HEIGHT - 1.0),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            pos2(WIDTH - 1.0, HEIGHT - 1.0),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ]
}

/// 縁を押すと向きに合う `BeginResize` を 1 度だけ送り、乗せるとポインタの形が変わる。
#[test]
fn pressing_a_window_edge_begins_a_resize_in_that_direction() {
    for (at, direction, cursor) in edge_cases() {
        let mut h = windows_app(WIDTH, HEIGHT);
        // 乗せただけ: 形が変わるが、頼みは出ない
        let sent = play(&mut h, vec![pointer(at)]);
        assert!(sent.is_empty(), "{direction:?}: {sent:?}");
        assert_eq!(
            h.output().platform_output.cursor_icon,
            cursor,
            "{direction:?}: 乗せたときの形"
        );
        let sent = play(&mut h, vec![button(at, true)]);
        assert_eq!(resizes(&sent), vec![direction], "{direction:?}: {sent:?}");
        // 離したあと、次の押しは新しく受ける
        play(&mut h, vec![button(at, false)]);
        let sent = play(&mut h, vec![button(at, true)]);
        assert_eq!(resizes(&sent), vec![direction], "{direction:?}: 2 度目");
    }
}

#[test]
fn inside_the_edge_nothing_resizes_and_the_cursor_stays() {
    let mut h = windows_app(WIDTH, HEIGHT);
    for at in [
        // （右の列の組の境目を避けた高さ）
        pos2(WIDTH - titlebar::EDGE - 2.0, 250.0),
        pos2(titlebar::EDGE + 2.0, HEIGHT / 2.0),
        pos2(WIDTH / 2.0, HEIGHT - titlebar::EDGE - 2.0),
        pos2(WIDTH / 2.0, 300.0),
    ] {
        let sent = play(
            &mut h,
            vec![pointer(at), button(at, true), button(at, false)],
        );
        assert!(resizes(&sent).is_empty(), "{at:?}: {sent:?}");
        let cursor = h.output().platform_output.cursor_icon;
        assert!(
            !matches!(
                cursor,
                CursorIcon::ResizeHorizontal
                    | CursorIcon::ResizeVertical
                    | CursorIcon::ResizeNwSe
                    | CursorIcon::ResizeNeSw
            ),
            "{at:?}: {cursor:?}"
        );
    }
}

/// 最大化中は縁が無い（最大化したウィンドウは、縁で大きさを変えない）。
#[test]
fn while_maximized_the_edges_do_nothing() {
    let mut h = windows_app(WIDTH, HEIGHT);
    set_maximized(&mut h, true);
    for (at, direction, cursor) in edge_cases() {
        let sent = play(&mut h, vec![pointer(at)]);
        assert_ne!(
            h.output().platform_output.cursor_icon,
            cursor,
            "{direction:?}"
        );
        let sent2 = play(&mut h, vec![button(at, true), button(at, false)]);
        assert!(
            resizes(&sent).is_empty() && resizes(&sent2).is_empty(),
            "{direction:?}: {sent:?} {sent2:?}"
        );
    }
    // 全画面も同じ
    set_maximized(&mut h, false);
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .fullscreen = Some(true);
    h.step();
    let at = pos2(WIDTH - 1.0, HEIGHT / 2.0);
    let sent = play(
        &mut h,
        vec![pointer(at), button(at, true), button(at, false)],
    );
    assert!(resizes(&sent).is_empty(), "{sent:?}");
}

/// 開いているメニュー・ウィンドウが縁を覆っているときは、縁を押しても大きさを変えない（押しはそのメニューのもの）。
#[test]
fn a_menu_over_the_edge_keeps_the_press() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let at = menu_title(&h, "ファイル").center();
    single_click(&mut h, at);
    assert!(h.state().state.popup.is_some());
    let edge = pos2(1.0, HEIGHT / 2.0);
    let sent = play(&mut h, vec![pointer(edge)]);
    assert_ne!(
        h.output().platform_output.cursor_icon,
        CursorIcon::ResizeHorizontal
    );
    let sent2 = play(&mut h, vec![button(edge, true), button(edge, false)]);
    assert!(
        resizes(&sent).is_empty() && resizes(&sent2).is_empty(),
        "{sent:?} {sent2:?}"
    );
}

/// ペン・タッチの押し（winit は Touch と、同じ押しのポインタを同じフレームで届ける）は、縁の押しとして受けない。
#[test]
fn a_touch_press_does_not_begin_a_resize() {
    let at = pos2(WIDTH - 1.0, HEIGHT / 2.0);
    let mut h = windows_app(WIDTH, HEIGHT);
    play(&mut h, vec![pointer(at)]);
    h.input_mut().events.push(Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: at,
        force: Some(0.5),
    });
    h.input_mut().events.push(button(at, true));
    h.step();
    let sent = commands(&h);
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    // 同じ場所をマウスで押せば受ける（対照）
    play(&mut h, vec![button(at, false)]);
    let sent = play(&mut h, vec![button(at, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::East], "{sent:?}");
}

/// 縁がキャンバスの端に重なっていても、縁を押した押しではキャンバスに点を描かない。1 つ内側を押せば描く。
#[test]
fn an_edge_press_over_the_canvas_does_not_paint() {
    let mut h = windows_app(WIDTH, HEIGHT);
    h.state_mut().dock = egui_dock::DockState::new(vec![Tab::Canvas]);
    h.run();
    let canvas = canvas_rect(&h);
    assert!(
        canvas.right() >= WIDTH - 1.0,
        "キャンバスがウィンドウの右端まで届く: {canvas:?}"
    );
    let y = canvas.center().y;
    // 縁（右端）: 大きさを変える頼みが出て、ストロークは始まらない
    let edge = pos2(WIDTH - 1.0, y);
    let sent = play(&mut h, vec![pointer(edge), button(edge, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::East], "{sent:?}");
    play(&mut h, vec![pointer(edge - vec2(0.0, 8.0))]);
    assert!(
        !h.state().state.is_stroking(),
        "縁の押しでストロークが始まった"
    );
    play(&mut h, vec![button(edge - vec2(0.0, 8.0), false)]);
    // 縁より内側: 描き始める（対照）
    let inner = pos2(WIDTH - titlebar::EDGE - 20.0, y);
    let sent = play(
        &mut h,
        vec![
            pointer(inner),
            button(inner, true),
            pointer(inner - vec2(0.0, 8.0)),
        ],
    );
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    assert!(
        h.state().state.is_stroking(),
        "縁の内側ではストロークが始まる"
    );
    play(&mut h, vec![button(inner - vec2(0.0, 8.0), false)]);
}

/// 描いている最中に縁で別の押しが来ても、大きさを変えない（描いている最中は縁の押しを受けない）。
#[test]
fn a_press_at_the_edge_during_a_stroke_does_not_begin_a_resize() {
    let mut h = windows_app(WIDTH, HEIGHT);
    h.state_mut().dock = egui_dock::DockState::new(vec![Tab::Canvas]);
    h.run();
    let y = canvas_rect(&h).center().y;
    let inner = pos2(WIDTH - titlebar::EDGE - 20.0, y);
    play(
        &mut h,
        vec![
            pointer(inner),
            button(inner, true),
            pointer(inner - vec2(0.0, 8.0)),
        ],
    );
    assert!(h.state().state.is_stroking(), "描いている最中");
    let edge = pos2(WIDTH - 1.0, y);
    let sent = play(&mut h, vec![pointer(edge), button(edge, true)]);
    assert!(
        resizes(&sent).is_empty(),
        "描いている最中の縁の押しが大きさを変えた: {sent:?}"
    );
    play(&mut h, vec![button(edge, false)]);
    assert!(!h.state().state.is_stroking());
    // 描き終えたあとは、同じ縁の押しを受ける（対照）
    let sent = play(&mut h, vec![button(edge, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::East], "{sent:?}");
}

/// ペンが縁の押しを止めるのは「紙に触れている」（`contact`）だけ。筆圧は触れていなくても 1 のペン・触れた直後は 0 のペンがあり、決め手にならない。
#[test]
fn the_pen_blocks_an_edge_press_by_contact_not_by_pressure() {
    let at = pos2(WIDTH - 1.0, HEIGHT / 2.0);
    for (contact, pressure, resizes_expected) in [
        (true, 0.0, false),
        (true, 1.0, false),
        (false, 1.0, true),
        (false, 0.0, true),
    ] {
        let mut h = windows_app(WIDTH, HEIGHT);
        play(&mut h, vec![pointer(at)]);
        // ペンの点と、マウスの押し（Touch は無い）を同じフレームに
        h.state().pen().push(pen_point(at, contact, pressure));
        h.input_mut().events.push(button(at, true));
        h.step();
        let sent = commands(&h);
        let got = resizes(&sent);
        if resizes_expected {
            assert_eq!(
                got,
                vec![ResizeDirection::East],
                "contact={contact} pressure={pressure}: {sent:?}"
            );
        } else {
            assert!(
                got.is_empty(),
                "contact={contact} pressure={pressure}: {sent:?}"
            );
        }
    }
}

/// 縁をまたいで描き続けても大きさを変えない（押しが縁の外で始まったとき）。
#[test]
fn a_stroke_that_runs_into_the_edge_keeps_painting_and_does_not_resize() {
    let mut h = windows_app(WIDTH, HEIGHT);
    h.state_mut().dock = egui_dock::DockState::new(vec![Tab::Canvas]);
    h.run();
    let y = canvas_rect(&h).center().y;
    let start = pos2(WIDTH - 60.0, y);
    let sent = play(
        &mut h,
        vec![
            pointer(start),
            button(start, true),
            pointer(start + vec2(20.0, 0.0)),
            pointer(pos2(WIDTH - 1.0, y)),
            pointer(pos2(WIDTH - 1.0, y + 6.0)),
        ],
    );
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    assert!(h.state().state.is_stroking());
    play(&mut h, vec![button(pos2(WIDTH - 1.0, y + 6.0), false)]);
}

/// 上の縁の上のメニューの見出しは、メニューを開く（大きさを変えない）。何も無い所は上へ大きさを変える。
#[test]
fn the_top_edge_yields_to_the_menu_titles_but_resizes_over_the_empty_bar() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let title = menu_title(&h, "ファイル");
    let at = pos2(title.center().x, 3.0);
    assert!(
        title.top() <= 2.0 && at.y < titlebar::EDGE,
        "見出しは上の縁に届く: {title:?}"
    );
    let sent = play(
        &mut h,
        vec![pointer(at), button(at, true), button(at, false)],
    );
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    assert!(
        matches!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::MenuBar(0))
        ),
        "メニューが開く"
    );
    h.state_mut().state.popup = None;
    h.run();
    let empty = pos2(empty_bar_point(&h).x, 1.0);
    let sent = play(&mut h, vec![pointer(empty), button(empty, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::North], "{sent:?}");
}

/// 上の縁にかかる Live Link の印（egui の押しを持たず、生の押しでウィンドウを開く）も、メニューの見出しと同じく縁より先に押しを受ける。
#[test]
fn the_top_edge_yields_to_the_link_mark() {
    let mut h = windows_app(WIDTH, HEIGHT);
    let mark = link_rect(&h);
    let at = pos2(mark.center().x, 2.0);
    assert!(
        mark.top() <= 2.0 && at.y < titlebar::EDGE,
        "印は上の縁に届く: {mark:?}"
    );
    let sent = play(
        &mut h,
        vec![pointer(at), button(at, true), button(at, false)],
    );
    assert!(
        resizes(&sent).is_empty(),
        "印の上の押しが縁になった: {sent:?}"
    );
    assert!(
        matches!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::LiveLink)
        ),
        "Live Link のウィンドウが開く"
    );
    // 印の外の同じ高さ（何も無い所）は、上へ大きさを変える（対照）
    h.state_mut().state.popup = None;
    h.run();
    let empty = pos2(empty_bar_point(&h).x, 2.0);
    let sent = play(&mut h, vec![pointer(empty), button(empty, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::North], "{sent:?}");
}

/// 3D ビューでも、縁の押しは回す操作の始まりにならない（ビューの入力は別の経路）。縁の 1 点内側では始まる。3D ビューがウィンドウの縁に届くのは
/// 右だけ（左はツールの列、下は状態の帯）。
#[test]
fn an_edge_press_over_the_3d_view_does_not_start_a_navigation() {
    let mut h = windows_app(WIDTH, HEIGHT);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().dock = egui_dock::DockState::new(vec![Tab::View3d]);
    h.run();
    let y = HEIGHT / 2.0;
    let edge = pos2(WIDTH - 1.0, y);
    let sent = play(&mut h, vec![pointer(edge), alt_button(edge, true)]);
    assert_eq!(resizes(&sent), vec![ResizeDirection::East], "{sent:?}");
    assert!(
        h.state().state.view3d.input.nav.is_none(),
        "縁の押しで回す操作が始まった"
    );
    // 縁の押しを持ったまま動かしても、回さない
    let moved = edge + vec2(-30.0, 12.0);
    play(&mut h, vec![pointer(moved)]);
    assert!(
        h.state().state.view3d.input.nav.is_none(),
        "縁の押しのまま動かして回した"
    );
    play(&mut h, vec![alt_button(moved, false)]);
    // 縁の 1 点内側（縁の幅の外）では、同じ押しが回す操作を始める（対照）
    let inner = pos2(WIDTH - titlebar::EDGE - 1.0, y);
    let sent = play(&mut h, vec![pointer(inner), alt_button(inner, true)]);
    assert!(resizes(&sent).is_empty(), "{sent:?}");
    assert!(
        h.state().state.view3d.input.nav.is_some(),
        "縁の内側で回す操作が始まらない"
    );
    play(&mut h, vec![alt_button(inner, false)]);
    assert!(h.state().state.view3d.input.nav.is_none());
}

/// 右の縁にあるスクロールのつまみは、縁より先に押しを受ける（つまみを掴んで動かせる）。
#[test]
fn a_scroll_thumb_at_the_right_edge_keeps_its_press() {
    // 背の低いウィンドウで、右のパネルの欄があふれてつまみが出る
    let mut h = windows_app(WIDTH, 400.0);
    h.run();
    let thumbs: Vec<Rect> = h
        .get_all_by_role(egui::accesskit::Role::ScrollBar)
        .map(|n| n.rect())
        .filter(|r| r.right() >= WIDTH - titlebar::EDGE - 0.5 && r.height() >= 20.0)
        .collect();
    assert!(!thumbs.is_empty(), "右の縁に届くつまみの溝が出ている");
    for thumb in thumbs {
        let at = pos2(WIDTH - 1.0, thumb.center().y);
        let sent = play(
            &mut h,
            vec![
                pointer(at),
                button(at, true),
                pointer(at + vec2(0.0, 12.0)),
                button(at + vec2(0.0, 12.0), false),
            ],
        );
        assert!(
            resizes(&sent).is_empty(),
            "つまみの上の押しが縁になった: {sent:?} {thumb:?}"
        );
    }
}
