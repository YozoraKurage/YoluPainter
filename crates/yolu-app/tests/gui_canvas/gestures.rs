//! ビュー（2D のキャンバス・3D ビュー）への押しの振り分け: ドックのタブをつかんだまま入っても描かない、ペンのサイドボタン・Alt・
//! Space・Ctrl+Space・R・消しゴムの端の組み合わせ、Ctrl+Space の拡縮（マウスもペンも）、ペンの押しが egui のポインタの代わりの入力と二重に
//! 効かないこと、サイドボタンの接触を egui の部品に右ボタンとして届けること。実機のペンは無いので、`PenSample` を差し込む
//! （Windows Ink のウィンドウが詰める点の代わり）。egui の側の代わりの入力（winit の Touch とポインタ）は、`Emulate::Yes` で同じ形に作る。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::pen::PenSample;
use yolu_app::{Tab, YoluApp};
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

/// ペンの点に添えて、winit が作る egui の代わりの入力（Touch とポインタ）も届けるか。
#[derive(Clone, Copy, PartialEq)]
enum Emulate {
    No,
    Yes,
}

fn sample(at: Pos2, contact: bool, barrel: bool, eraser: bool) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure: 1.0,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser,
        barrel,
        pointer_id: 5,
        time_ms: 0,
    }
}

/// ペンの点 1 つ（1 フレーム）。`first` は触れた最初の点、`last` は離した点。
struct Pen {
    barrel: bool,
    eraser: bool,
    emulate: Emulate,
}

impl Pen {
    fn tip() -> Pen {
        Pen {
            barrel: false,
            eraser: false,
            emulate: Emulate::No,
        }
    }
    fn barrel() -> Pen {
        Pen {
            barrel: true,
            ..Pen::tip()
        }
    }
    fn eraser() -> Pen {
        Pen {
            eraser: true,
            ..Pen::tip()
        }
    }
    fn emulated(self) -> Pen {
        Pen {
            emulate: Emulate::Yes,
            ..self
        }
    }

    /// ペンの点 1 つと、winit が同じウィンドウのメッセージから作る egui の入力を、1 つのフレームへ入れて走らせる（kittest の `event` は
    /// 1 つにつき 1 フレームなので、同じフレームに入れるには入力へ直に足す。実際のウィンドウでは、1 つの WM_POINTER の点と Touch・ポインタは同じフレーム）。
    fn frame(&self, h: &mut H, point: PenSample, events: Vec<Event>) {
        h.state().pen().push(point);
        if self.emulate == Emulate::Yes {
            h.input_mut().events.extend(events);
        }
        h.step();
    }

    fn down(&self, h: &mut H, at: Pos2) {
        if self.emulate == Emulate::Yes {
            // 触れる前に、ペンは浮いて近づく（浮いている点と、winit が作るポインタの移動）。egui はその位置の部品を知っている
            self.frame(
                h,
                sample(at, false, self.barrel, self.eraser),
                vec![Event::PointerMoved(at)],
            );
        }
        let modifiers = current_modifiers(h);
        self.frame(
            h,
            sample(at, true, self.barrel, self.eraser),
            vec![
                Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(5),
                    phase: egui::TouchPhase::Start,
                    pos: at,
                    force: Some(1.0),
                },
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
        );
    }

    fn to(&self, h: &mut H, at: Pos2) {
        self.frame(
            h,
            sample(at, true, self.barrel, self.eraser),
            vec![
                Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(5),
                    phase: egui::TouchPhase::Move,
                    pos: at,
                    force: Some(1.0),
                },
                Event::PointerMoved(at),
            ],
        );
    }

    fn up(&self, h: &mut H, at: Pos2) {
        let modifiers = current_modifiers(h);
        self.frame(
            h,
            sample(at, false, self.barrel, self.eraser),
            vec![
                Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(5),
                    phase: egui::TouchPhase::End,
                    pos: at,
                    force: None,
                },
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers,
                },
                Event::PointerGone,
            ],
        );
        h.run();
    }

    /// 触れて、points をなぞって、離す。
    fn drag(&self, h: &mut H, points: &[Pos2]) {
        self.down(h, points[0]);
        for p in &points[1..] {
            self.to(h, *p);
        }
        self.up(h, *points.last().unwrap());
    }
}

thread_local! {
    /// いま押している修飾（`hold` で足したもの。試験ごとのスレッドに 1 つ）。
    static HELD: std::cell::Cell<Modifiers> = const { std::cell::Cell::new(Modifiers::NONE) };
}

fn current_modifiers(_h: &H) -> Modifiers {
    HELD.with(|m| m.get())
}

/// 修飾を押したままにする（`release_mods` で離す）。
fn hold(h: &mut H, m: Modifiers) {
    HELD.with(|held| held.set(m));
    h.event(Event::ModifiersChanged(m));
    h.step();
}

fn release_mods(h: &mut H) {
    hold(h, Modifiers::NONE);
}

fn key_event(h: &mut H, key: Key, pressed: bool) {
    let modifiers = current_modifiers(h);
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    });
    h.step();
}

fn hold_key(h: &mut H, key: Key) {
    key_event(h, key, true);
}

fn release_key(h: &mut H, key: Key) {
    key_event(h, key, false);
}

/// マウスの押す・離す・動く（押している修飾を、イベントにも添える。実際のウィンドウのイベントもそうなっている）。
fn m_press(h: &H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: current_modifiers(h),
    });
}

fn m_release(h: &H, at: Pos2) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: current_modifiers(h),
    });
}

fn m_drag(h: &mut H, points: &[Pos2]) {
    m_press(h, points[0]);
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    m_release(h, *points.last().unwrap());
    h.step();
    h.run();
}

fn m_click(h: &mut H, at: Pos2) {
    m_press(h, at);
    h.step();
    m_release(h, at);
    h.run();
}

fn strokes(h: &H) -> usize {
    h.state().state.doc.undo_count()
}

fn nothing_started(h: &H) {
    let s = &h.state().state;
    assert!(!s.doc.can_undo(), "何も描かれていない");
    assert!(!s.doc.has_active_stroke() && s.canvas.stroke.is_none());
}

fn view_of(h: &H) -> yolu_app::canvas::view::ViewState {
    h.state().state.view
}

fn cursor(h: &H) -> egui::CursorIcon {
    h.output().platform_output.cursor_icon
}

// ───────── 1. ドッキング中の誤描画 ─────────

/// ドックのタブの見出し（3D ビュー）をつかんで、キャンバスの上へ動かして離す。マウスの押しは見出しで始まる。
fn drag_tab_over_the_canvas_with_the_mouse(h: &mut H) {
    let tab = h.state().tab_rects[&Tab::View3d].center();
    let c = canvas_rect(h).center();
    press(h, tab, PointerButton::Primary);
    h.step();
    for i in 1..=8 {
        move_to(h, tab + (c - tab) * (i as f32 / 8.0));
        h.step();
    }
    move_to(h, offset(c, 30.0, 10.0));
    h.step();
    release(h, offset(c, 30.0, 10.0), PointerButton::Primary);
    h.run();
}

#[test]
fn dragging_a_dock_tab_over_the_canvas_with_the_mouse_paints_nothing() {
    let mut h = app(1280.0, 800.0, 256);
    drag_tab_over_the_canvas_with_the_mouse(&mut h);
    nothing_started(&h);
}

#[test]
fn dragging_a_dock_tab_over_the_canvas_with_the_pen_paints_nothing() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let mut h = app(1280.0, 800.0, 256);
        let tab = h.state().tab_rects[&Tab::View3d].center();
        let c = canvas_rect(&h).center();
        let pen = Pen {
            emulate,
            ..Pen::tip()
        };
        // 見出しに触れ、キャンバスの上へ動かして離す（ペンは触れたまま、押した所は見出し）
        let mut path: Vec<Pos2> = (0..=8)
            .map(|i| tab + (c - tab) * (i as f32 / 8.0))
            .collect();
        path.push(offset(c, 30.0, 10.0));
        pen.drag(&mut h, &path);
        nothing_started(&h);
        assert!(
            h.state().state.canvas.pen_press.is_none(),
            "離したら押しは終わる"
        );
    }
}

#[test]
fn a_pen_press_that_began_outside_the_canvas_never_paints_when_it_moves_in() {
    // ドックの見出しでなくても（レイヤーのパネルの上で触れてキャンバスへ動かしても）、押し始めがビューの外ならその押しは描かない
    let mut h = app(1280.0, 800.0, 256);
    let layers = h.state().tab_rects[&Tab::Layers].center();
    let c = canvas_rect(&h).center();
    Pen::tip().drag(
        &mut h,
        &[layers, offset(layers, -80.0, 0.0), c, offset(c, 40.0, 0.0)],
    );
    nothing_started(&h);
    // 同じ押しの途中でキャンバスに入っても、次に触れ直したら描ける
    Pen::tip().drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
}

#[test]
fn a_pen_press_that_began_on_the_canvas_keeps_painting_when_it_leaves_and_returns() {
    let mut h = app(1280.0, 800.0, 256);
    let r = canvas_rect(&h);
    let c = r.center();
    let pen = Pen::tip();
    pen.down(&mut h, c);
    pen.to(&mut h, offset(c, 20.0, 0.0));
    pen.to(&mut h, pos2(r.left() - 30.0, c.y)); // 外へ
    pen.to(&mut h, offset(c, -20.0, 10.0));
    pen.up(&mut h, offset(c, -20.0, 10.0));
    assert_eq!(strokes(&h), 1, "1 回の触れ = 1 つのストローク");
}

#[test]
fn a_press_on_the_dock_separator_that_reaches_into_the_canvas_does_not_paint() {
    // キャンバスと右の列の分け目は、キャンバスの縁へ少し食い込んで掴める。その食い込んだ所（キャンバスの矩形の中）を押しても、
    // キャンバスの押しではない（分け目を動かすつもりの押しで描かない）
    // （ペンは、winit が作るポインタの代わりの入力が分け目の押しを egui に伝える場合だけ。ペンの点だけでは分け目の押しと分からない）
    for emulate in [None, Some(Emulate::Yes)] {
        let mut h = app(1280.0, 800.0, 256);
        let r = canvas_rect(&h);
        // 分け目の掴める幅（食い込み）は、端から 1 点に満たない（分け目の位置の端数で、1 点では外れることがある）
        let edge = pos2(r.right() - 0.5, r.center().y);
        match emulate {
            None => {
                press(&h, edge, PointerButton::Primary);
                h.step();
                move_to(&h, offset(edge, -30.0, 0.0));
                h.step();
                release(&h, offset(edge, -30.0, 0.0), PointerButton::Primary);
                h.run();
            }
            Some(emulate) => {
                let pen = Pen {
                    emulate,
                    ..Pen::tip()
                };
                pen.drag(
                    &mut h,
                    &[edge, offset(edge, -30.0, 0.0), offset(edge, -60.0, 0.0)],
                );
            }
        }
        nothing_started(&h);
    }
}

#[test]
fn dragging_a_dock_tab_over_the_3d_view_paints_nothing() {
    let (mut h, rect) = cube_view();
    // キャンバスのタブの見出しをつかんで 3D ビューの上へ
    let tab = h.state().tab_rects[&Tab::Canvas].center();
    let target = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let pen = Pen::tip().emulated();
    let path: Vec<Pos2> = (0..=8)
        .map(|i| tab + (target - tab) * (i as f32 / 8.0))
        .collect();
    pen.drag(&mut h, &path);
    nothing_started(&h);
    assert_eq!(painted(&h), 0);
    // マウスでも
    press(&h, tab, PointerButton::Primary);
    h.step();
    for p in &path[1..] {
        move_to(&h, *p);
        h.step();
    }
    release(&h, target, PointerButton::Primary);
    h.run();
    nothing_started(&h);
    assert_eq!(painted(&h), 0);
}

#[test]
fn the_grab_of_a_dock_tab_blocks_a_stroke_until_the_frame_after_it_ends() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    // つかんでいる印（前のフレームの結果）が立っている間は、触れても描かない
    h.state_mut().state.ui.dock_grab = [true, false];
    Pen::tip().drag(&mut h, &[c, offset(c, 20.0, 0.0)]);
    assert!(!h.state().state.doc.can_undo(), "つかんでいる間は描かない");
    h.run();
    h.run();
    assert!(
        !h.state().state.dock_grabbed(),
        "つかんでいなければ自然に下りる"
    );
    Pen::tip().drag(&mut h, &[c, offset(c, 20.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
}

// ───────── 2. ペンの 2D ─────────

#[test]
fn the_pen_tip_and_the_eraser_end_paint_on_the_canvas() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    Pen::tip().drag(&mut h, &[c, offset(c, 30.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
    assert_eq!(canvas_pixel(&h, c)[3], 255);
    Pen::eraser().drag(&mut h, &[c, offset(c, 30.0, 0.0)]);
    assert_eq!(strokes(&h), 2);
    assert_eq!(canvas_pixel(&h, c)[3], 0, "消しゴムの端で消える");
}

#[test]
fn the_pen_side_button_alt_and_ctrl_never_paint_on_the_canvas() {
    let c0 = |h: &H| canvas_rect(h).center();
    let cases: [(&str, Pen, Modifiers); 6] = [
        ("サイドボタン", Pen::barrel(), Modifiers::NONE),
        (
            "サイドボタン + 消しゴムの端",
            Pen {
                barrel: true,
                eraser: true,
                emulate: Emulate::No,
            },
            Modifiers::NONE,
        ),
        ("Alt", Pen::tip(), Modifiers::ALT),
        ("Ctrl", Pen::tip(), Modifiers::CTRL),
        ("Alt + 消しゴムの端", Pen::eraser(), Modifiers::ALT),
        (
            "サイドボタン + Alt",
            Pen {
                barrel: true,
                ..Pen::tip()
            },
            Modifiers::ALT,
        ),
    ];
    for (name, pen, m) in cases {
        for emulate in [Emulate::No, Emulate::Yes] {
            let mut h = common::app(1280.0, 800.0, 256);
            let c = c0(&h);
            hold(&mut h, m);
            let pen = Pen {
                emulate,
                ..pen_copy(&pen)
            };
            pen.drag(&mut h, &[c, offset(c, 30.0, 0.0), offset(c, 30.0, 30.0)]);
            release_mods(&mut h);
            assert!(
                !h.state().state.doc.can_undo(),
                "{name} {}: 描かない",
                emulate == Emulate::Yes
            );
            assert!(h.state().state.canvas.stroke.is_none());
        }
    }
}

fn pen_copy(p: &Pen) -> Pen {
    Pen {
        barrel: p.barrel,
        eraser: p.eraser,
        emulate: p.emulate,
    }
}

#[test]
fn a_modifier_pressed_after_the_stroke_began_does_not_stop_it_and_one_released_does_not_start_one()
{
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    let pen = Pen::tip();
    // 描き始めてから Alt・Space・Shift を押しても、そのストロークは描き続ける（押す前に決める）
    pen.down(&mut h, c);
    hold(&mut h, Modifiers::ALT);
    hold_key(&mut h, Key::Space);
    pen.to(&mut h, offset(c, 30.0, 0.0));
    pen.to(&mut h, offset(c, 60.0, 0.0));
    pen.up(&mut h, offset(c, 60.0, 0.0));
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
    assert_eq!(strokes(&h), 1);
    assert_eq!(
        canvas_pixel(&h, offset(c, 60.0, 0.0))[3],
        255,
        "最後まで描いた"
    );
    assert_eq!(view_of(&h).pan, vec2(0.0, 0.0), "パンにはならない");
    // 修飾を押して触れ、途中で離しても、その押しは描き始めない
    hold(&mut h, Modifiers::ALT);
    pen.down(&mut h, offset(c, 0.0, 40.0));
    release_mods(&mut h);
    pen.to(&mut h, offset(c, 40.0, 40.0));
    pen.up(&mut h, offset(c, 40.0, 40.0));
    assert_eq!(strokes(&h), 1, "押す前に決める");
}

#[test]
fn the_pen_pans_with_space_and_rotates_with_alt_in_steps_like_the_mouse() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let mut h = app(1280.0, 800.0, 256);
        let c = canvas_rect(&h).center();
        let pen = Pen {
            emulate,
            ..Pen::tip()
        };
        hold_key(&mut h, Key::Space);
        pen.drag(&mut h, &[c, offset(c, 30.0, 0.0), offset(c, 60.0, 20.0)]);
        release_key(&mut h, Key::Space);
        // 二重に動かない（egui のポインタの代わりの入力が来ても、ペンの点が動かした分だけ）
        assert_eq!(
            view_of(&h).pan,
            vec2(60.0, 20.0),
            "{}",
            emulate == Emulate::Yes
        );
        nothing_started(&h);
        // 押しの始めに Alt を持っていれば表示の回転（描かない）。15° 刻みが既定
        h.state_mut().state.view.fit();
        let before = view_of(&h).angle;
        hold(&mut h, Modifiers::ALT);
        let center = canvas_rect(&h).center();
        let start = offset(center, 100.0, 0.0);
        let path = [
            start,
            offset(center, 90.0, 40.0),
            offset(center, 70.0, 70.0),
            offset(center, 20.0, 100.0),
        ];
        pen.drag(&mut h, &path);
        let stepped = view_of(&h).angle;
        assert_ne!(stepped, before, "回った {}", emulate == Emulate::Yes);
        assert!(
            (stepped / 15.0 - (stepped / 15.0).round()).abs() < 1e-3,
            "15° の倍数: {stepped}"
        );
        nothing_started(&h);
        // Shift も押していれば自由
        h.state_mut().state.view.fit();
        hold(&mut h, Modifiers::ALT | Modifiers::SHIFT);
        pen.drag(&mut h, &path);
        let free = view_of(&h).angle;
        assert!(
            (free / 15.0 - (free / 15.0).round()).abs() > 1e-2,
            "自由な角度: {free}"
        );
        release_mods(&mut h);
        nothing_started(&h);
        // R を押しながらは、もう回さない（描く）
        h.state_mut().state.view.fit();
        let before = view_of(&h).angle;
        hold_key(&mut h, Key::R);
        pen.drag(&mut h, &path);
        release_key(&mut h, Key::R);
        assert_eq!(view_of(&h).angle, before, "R では回らない");
        assert_eq!(strokes(&h), 1, "R を押していても描く");
    }
}

#[test]
fn the_pen_with_ctrl_and_space_zooms_by_dragging_and_by_clicking() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let mut h = app(1280.0, 800.0, 256);
        let c = canvas_rect(&h).center();
        let pen = Pen {
            emulate,
            ..Pen::tip()
        };
        hold(&mut h, Modifiers::CTRL);
        hold_key(&mut h, Key::Space);
        // 右へ動かすと拡大（押した点が中心: そこの画素は動かない）
        let anchor = offset(c, 100.0, 40.0);
        let (px, py) = h
            .state()
            .state
            .view
            .view(canvas_rect(&h), 256, 256)
            .to_canvas(anchor);
        pen.drag(
            &mut h,
            &[
                anchor,
                offset(anchor, 30.0, 0.0),
                offset(anchor, 60.0, 0.0),
                offset(anchor, 100.0, 0.0),
            ],
        );
        let zoom = view_of(&h).zoom;
        assert!(zoom > 1.8 && zoom < 3.0, "100 点で約 2.2 倍: {zoom}");
        let (qx, qy) = h
            .state()
            .state
            .view
            .view(canvas_rect(&h), 256, 256)
            .to_canvas(anchor);
        assert!(
            (px - qx).abs() < 0.01 && (py - qy).abs() < 0.01,
            "押した点の画素は動かない"
        );
        // 左へ動かすと縮小
        pen.drag(
            &mut h,
            &[
                anchor,
                offset(anchor, -50.0, 0.0),
                offset(anchor, -100.0, 0.0),
            ],
        );
        assert!(view_of(&h).zoom < zoom, "縮んだ");
        // 動かさずに離すとクリック: 拡大
        h.state_mut().state.view.fit();
        pen.drag(&mut h, &[anchor, offset(anchor, 1.0, 0.0)]);
        assert_eq!(view_of(&h).zoom, 2.0, "クリックで 2 倍");
        // Alt も押していれば縮小
        hold(&mut h, Modifiers::CTRL | Modifiers::ALT);
        pen.drag(&mut h, &[anchor, offset(anchor, 1.0, 0.0)]);
        assert_eq!(view_of(&h).zoom, 1.0, "Ctrl+Alt+Space のクリックで半分");
        release_key(&mut h, Key::Space);
        release_mods(&mut h);
        nothing_started(&h);
        assert!(
            h.state().state.canvas.zooming.is_none() && h.state().state.canvas.pen_press.is_none()
        );
    }
}

/// Ctrl+Space のペンの拡縮のクリック（動かさずに離す）は、離しの操作。OS に押しを奪われて補った離しでは出さない（マウスの取りこぼしと同じ）。本物の離しは拡大する。
#[test]
fn a_taken_pen_press_does_not_zoom_like_a_click_but_a_real_release_does() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    let pen = Pen::tip().emulated();
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    let anchor = offset(c, 100.0, 40.0);
    let near = offset(anchor, 1.0, 0.0);
    pen.down(&mut h, anchor);
    pen.to(&mut h, near);
    h.state().pen().push_lost(sample(near, false, false, false));
    let modifiers = current_modifiers(&h);
    h.input_mut().events.push(Event::PointerButton {
        pos: near,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    h.step();
    assert_eq!(
        view_of(&h).zoom,
        1.0,
        "補った離しではクリックの拡縮をしない"
    );
    assert!(
        h.state().state.canvas.zooming.is_none() && h.state().state.canvas.pen_press.is_none(),
        "押しの途中は残らない"
    );
    // 本物の離しは、同じ操作で 2 倍にする
    pen.drag(&mut h, &[anchor, near]);
    assert_eq!(view_of(&h).zoom, 2.0, "クリックで 2 倍");
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
}

#[test]
fn the_mouse_zooms_with_ctrl_and_space_and_pans_with_space_alone() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    let anchor = offset(c, -60.0, 30.0);
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    m_drag(
        &mut h,
        &[anchor, offset(anchor, 40.0, 0.0), offset(anchor, 80.0, 0.0)],
    );
    let zoomed = view_of(&h).zoom;
    assert!(zoomed > 1.5, "右へのドラッグで拡大: {zoomed}");
    assert!(
        view_of(&h).pan.length() > 0.0,
        "押した点が中心なのでパンも動く"
    );
    nothing_started(&h);
    // クリックで拡大、Ctrl+Alt+Space のクリックで縮小
    h.state_mut().state.view.fit();
    m_click(&mut h, anchor);
    assert_eq!(view_of(&h).zoom, 2.0);
    hold(&mut h, Modifiers::CTRL | Modifiers::ALT);
    m_click(&mut h, anchor);
    assert_eq!(view_of(&h).zoom, 1.0);
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
    // Space だけならパン（今のまま）
    h.state_mut().state.view.fit();
    hold_key(&mut h, Key::Space);
    m_drag(&mut h, &[c, offset(c, 25.0, 5.0)]);
    release_key(&mut h, Key::Space);
    assert_eq!(view_of(&h).zoom, 1.0);
    assert_eq!(view_of(&h).pan, vec2(25.0, 5.0));
    nothing_started(&h);
}

#[test]
fn the_pointer_is_a_magnifier_while_ctrl_and_space_are_held() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    move_to(&h, c);
    h.step();
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    move_to(&h, offset(c, 5.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomIn);
    hold(&mut h, Modifiers::CTRL | Modifiers::ALT);
    move_to(&h, offset(c, 9.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomOut, "Alt を足すと縮小");
    hold(&mut h, Modifiers::NONE);
    move_to(&h, offset(c, 12.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::Grab, "Space だけなら掴む手");
    release_key(&mut h, Key::Space);
    // ドラッグ中は、離すまで虫めがね
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    m_press(&h, c);
    h.step();
    move_to(&h, offset(c, 20.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomIn);
    m_release(&h, offset(c, 20.0, 0.0));
    h.run();
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
}

#[test]
fn a_pen_press_is_not_acted_on_twice_when_the_pointer_copy_arrives_too() {
    // 描く: 1 つのストローク（ポインタの代わりの押しが 2 つ目のストロークを始めない）
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    Pen::tip()
        .emulated()
        .drag(&mut h, &[c, offset(c, 20.0, 0.0), offset(c, 40.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
    // 選択のツール（矩形）: 1 つの選択範囲を 1 回の Undo で
    h.state_mut()
        .state
        .apply(yolu_app::state::Action::SelectTool(
            yolu_app::state::Tool::SelectRect,
        ));
    Pen::tip()
        .emulated()
        .drag(&mut h, &[offset(c, -60.0, -60.0), offset(c, 60.0, 60.0)]);
    assert_eq!(strokes(&h), 2, "選択は 1 回の Undo");
    assert!(h.state().state.doc.selection().is_some());
}

#[test]
fn mouse_clicks_still_work_while_the_pen_only_hovers() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    // ペンが浮いているだけ（触れていない点）のフレームがあっても、マウスは描ける
    h.state()
        .pen()
        .push(sample(offset(c, 100.0, 100.0), false, false, false));
    h.step();
    m_drag(&mut h, &[c, offset(c, 30.0, 0.0)]);
    assert_eq!(strokes(&h), 1);
}

#[test]
fn a_pen_press_is_dropped_when_the_window_loses_focus() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    hold_key(&mut h, Key::Space);
    Pen::tip().down(&mut h, c);
    assert!(h.state().state.canvas.pen_press.is_some());
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(h.state().state.canvas.pen_press.is_none());
    assert!(!h.state().state.canvas.panning);
    release_key(&mut h, Key::Space);
}

#[test]
fn alt_does_not_start_a_stroke_for_the_mouse_either_and_selection_keeps_its_modifiers() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    // Alt を押したブラシは描き始めない（Alt はスポイトに予約）
    hold(&mut h, Modifiers::ALT);
    h.event(Event::PointerMoved(c));
    h.event(Event::PointerButton {
        pos: c,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: c,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    h.run();
    release_mods(&mut h);
    nothing_started(&h);
    // 選択のツールでは Shift・Ctrl は組み合わせの修飾で、ペンで触れても効く（足す）
    h.state_mut()
        .state
        .apply(yolu_app::state::Action::SelectTool(
            yolu_app::state::Tool::SelectRect,
        ));
    Pen::tip().drag(&mut h, &[offset(c, -80.0, -40.0), offset(c, -20.0, 20.0)]);
    let first = h.state().state.doc.selection().cloned().unwrap();
    hold(&mut h, Modifiers::SHIFT);
    Pen::tip().drag(&mut h, &[offset(c, 20.0, -40.0), offset(c, 80.0, 20.0)]);
    release_mods(&mut h);
    let both = h.state().state.doc.selection().cloned().unwrap();
    assert!(!both.same_as(&first), "Shift で足した");
}

// ───────── 3. ペンの 3D ─────────

fn cube_view() -> (H, Rect) {
    cube_view_from(app(1100.0, 760.0, 256))
}

fn cube_view_from(mut h: H) -> (H, Rect) {
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

fn screen_of(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn painted(h: &H) -> usize {
    let doc = &h.state().state.doc;
    let mut n = 0;
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if yolu_app::engine::composite_pixel(doc, x, y)[3] > 0 {
                n += 1;
            }
        }
    }
    n
}

fn camera(h: &H) -> yolu_core::geometry::OrbitCamera {
    h.state().state.view3d.camera
}

#[test]
fn the_pen_tip_paints_the_surface_and_the_side_button_orbits_it() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let (mut h, rect) = cube_view();
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        Pen {
            emulate,
            ..Pen::tip()
        }
        .drag(&mut h, &[at, offset(at, 6.0, 0.0)]);
        assert!(painted(&h) > 0, "ペン先は描く");
        assert_eq!(strokes(&h), 1);
        // サイドボタン: 右ドラッグと同じ回す。描かない。二重に回らない
        let before = camera(&h);
        Pen {
            emulate,
            ..Pen::barrel()
        }
        .drag(&mut h, &[at, offset(at, 40.0, 0.0), offset(at, 80.0, 20.0)]);
        let after = camera(&h);
        assert_eq!(strokes(&h), 1, "サイドボタンでは描かない");
        let d = vec2(80.0, 20.0);
        assert!(
            (after.yaw - (before.yaw + d.x * 0.35)).abs() < 1e-3,
            "{} {} {emulate:?}",
            before.yaw,
            after.yaw,
            emulate = emulate == Emulate::Yes
        );
        assert!((after.pitch - (before.pitch + d.y * 0.35)).abs() < 1e-3);
        assert!(
            h.state().state.view3d.input.nav.is_none()
                && h.state().state.view3d.input.pen_press.is_none()
        );
    }
}

#[test]
fn the_pen_with_the_side_button_and_shift_orbits_and_no_longer_pans() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let before = camera(&h);
    hold(&mut h, Modifiers::SHIFT);
    Pen::barrel().drag(&mut h, &[at, offset(at, 30.0, 0.0), offset(at, 60.0, 10.0)]);
    release_mods(&mut h);
    let after = camera(&h);
    assert_eq!(after.target, before.target, "Shift でもパンしない");
    assert_ne!(after.yaw, before.yaw, "回す");
    assert_eq!(strokes(&h), 0);
}

#[test]
fn the_pen_with_alt_snap_orbits_and_alt_shift_no_longer_pans_like_the_mouse() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let (mut h, rect) = cube_view();
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        let before = camera(&h);
        hold(&mut h, Modifiers::ALT);
        Pen {
            emulate,
            ..Pen::tip()
        }
        .drag(&mut h, &[at, offset(at, 50.0, 0.0)]);
        release_mods(&mut h);
        let after = camera(&h);
        assert!(
            (after.yaw - (before.yaw + 50.0 * 0.35)).abs() < 1e-3,
            "Alt + ペンで回す（二重に回らない。軸から離れていれば吸い付かない）{} {}",
            before.yaw,
            after.yaw
        );
        assert_eq!(strokes(&h), 0, "描かない");
        // Alt + Shift: パンしない（回す）
        let before = camera(&h);
        hold(&mut h, Modifiers::ALT | Modifiers::SHIFT);
        Pen {
            emulate,
            ..Pen::tip()
        }
        .drag(&mut h, &[at, offset(at, 30.0, 0.0)]);
        release_mods(&mut h);
        assert_eq!(camera(&h).target, before.target);
        assert_ne!(camera(&h).yaw, before.yaw);
        assert_eq!(strokes(&h), 0);
        // マウスの Alt + 左ドラッグも同じ量（ペンとマウスは同じ決まり）
        let before = camera(&h);
        h.event(Event::ModifiersChanged(Modifiers::ALT));
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::ALT,
        });
        h.step();
        h.event(Event::PointerMoved(offset(at, 50.0, 0.0)));
        h.step();
        h.event(Event::PointerButton {
            pos: offset(at, 50.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::ALT,
        });
        h.run();
        release_mods(&mut h);
        assert!((camera(&h).yaw - (before.yaw + 50.0 * 0.35)).abs() < 1e-3);
    }
}

#[test]
fn the_pen_pans_the_3d_view_with_space_and_never_paints() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let before = camera(&h);
    hold_key(&mut h, Key::Space);
    Pen::tip().drag(&mut h, &[at, offset(at, 30.0, 0.0)]);
    release_key(&mut h, Key::Space);
    assert_ne!(camera(&h).target, before.target);
    assert_eq!(strokes(&h), 0);
    // マウスの Space + 左ドラッグも同じパン
    let before = camera(&h);
    hold_key(&mut h, Key::Space);
    m_drag(&mut h, &[at, offset(at, 30.0, 0.0)]);
    release_key(&mut h, Key::Space);
    assert_eq!(camera(&h).target, {
        let mut c = before;
        c.pan(30.0, 0.0, rect.height());
        c.target
    });
    assert_eq!(strokes(&h), 0);
}

#[test]
fn ctrl_and_space_dolly_the_3d_view_for_the_pen_and_the_mouse() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    // 右へ動かすと寄る（距離が縮む）、左は引く
    let d0 = camera(&h).distance;
    Pen::tip().drag(&mut h, &[at, offset(at, 40.0, 0.0), offset(at, 80.0, 0.0)]);
    let d1 = camera(&h).distance;
    assert!(d1 < d0, "寄った: {d0} → {d1}");
    Pen::tip().drag(
        &mut h,
        &[at, offset(at, -40.0, 0.0), offset(at, -80.0, 0.0)],
    );
    assert!(camera(&h).distance > d1, "引いた");
    // クリックで寄る・Alt を足して引く
    let d2 = camera(&h).distance;
    Pen::tip().drag(&mut h, &[at, offset(at, 1.0, 0.0)]);
    let d3 = camera(&h).distance;
    assert!(d3 < d2, "クリックで寄る");
    hold(&mut h, Modifiers::CTRL | Modifiers::ALT);
    Pen::tip().drag(&mut h, &[at, offset(at, 1.0, 0.0)]);
    assert!(
        (camera(&h).distance - d2).abs() < 1e-3,
        "Ctrl+Alt+Space のクリックで戻る"
    );
    hold(&mut h, Modifiers::CTRL);
    // マウスも同じ
    m_drag(&mut h, &[at, offset(at, 40.0, 0.0), offset(at, 80.0, 0.0)]);
    assert!(camera(&h).distance < d2, "マウスも寄る");
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
    assert_eq!(strokes(&h), 0);
    assert!(
        h.state().state.view3d.input.nav.is_none() && h.state().state.view3d.input.zoom.is_none()
    );
}

#[test]
fn the_pen_never_paints_the_surface_with_ctrl_or_the_side_button() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    for (m, pen) in [
        (Modifiers::CTRL, Pen::tip()),
        (Modifiers::CTRL, Pen::eraser()),
        (Modifiers::NONE, Pen::barrel()),
    ] {
        hold(&mut h, m);
        pen.drag(&mut h, &[at, offset(at, 3.0, 0.0)]);
        release_mods(&mut h);
        assert_eq!(strokes(&h), 0);
        assert_eq!(painted(&h), 0);
    }
}

#[test]
fn a_pen_press_that_began_outside_the_3d_view_never_paints_when_it_moves_in() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let outside = h.state().tab_rects[&Tab::Layers].center();
    Pen::tip().drag(&mut h, &[outside, at, offset(at, 10.0, 0.0)]);
    assert_eq!(strokes(&h), 0);
    assert_eq!(painted(&h), 0);
}

#[test]
fn the_3d_pen_press_goes_when_the_window_loses_focus_and_a_started_orbit_ends() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    Pen::barrel().down(&mut h, at);
    Pen::barrel().to(&mut h, offset(at, 20.0, 0.0));
    assert!(h.state().state.view3d.input.nav.is_some());
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(h.state().state.view3d.input.nav.is_none());
    assert!(h.state().state.view3d.input.pen_press.is_none());
}

// ───────── 4. サイドボタンを egui の部品に右ボタンとして届ける ─────────

#[test]
fn the_side_button_press_reaches_the_ui_as_a_right_click_and_opens_the_layer_menu() {
    let mut h = app(1280.0, 800.0, 256);
    let row = rect_of(&h, "レイヤー 1", |_| true);
    let at = pos2(row.left() + 90.0, row.center().y);
    // winit がペンの Touch から作る入力（左ボタン）を、Windows Ink の点のサイドボタンで右ボタンに直す
    let mut events = vec![
        Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(5),
            phase: egui::TouchPhase::Start,
            pos: at,
            force: Some(0.5),
        },
        Event::PointerMoved(at),
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ];
    h.state().pen().push(sample(at, true, true, false));
    h.state_mut().remap_pen_buttons(&mut events);
    assert!(matches!(
        events[2],
        Event::PointerButton {
            button: PointerButton::Secondary,
            pressed: true,
            ..
        }
    ));
    for e in events {
        h.event(e);
    }
    h.step();
    let mut release_events = vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }];
    h.state().pen().push(sample(at, false, false, false)); // 離すときにはサイドボタンが外れていても、押した右ボタンを離す
    h.state_mut().remap_pen_buttons(&mut release_events);
    assert!(matches!(
        release_events[0],
        Event::PointerButton {
            button: PointerButton::Secondary,
            pressed: false,
            ..
        }
    ));
    for e in release_events {
        h.event(e);
    }
    h.run();
    assert!(
        matches!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(yolu_app::state::PopupKind::LayerContext(_))
        ),
        "右クリックのメニューが開く"
    );
}

#[test]
fn only_a_pen_touch_with_the_side_button_is_turned_into_a_right_button() {
    use yolu_app::pen::ButtonMap;
    let press_ev = |button| Event::PointerButton {
        pos: pos2(1.0, 1.0),
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    let release_ev = |button| Event::PointerButton {
        pos: pos2(1.0, 1.0),
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    };
    let button_of = |e: &Event| match e {
        Event::PointerButton { button, .. } => *button,
        _ => panic!(),
    };
    let at = pos2(1.0, 1.0);
    let mut map = ButtonMap::default();
    // ペンの点が無いフレームのポインタ（マウス・指）は直さない
    let mut mouse = [
        press_ev(PointerButton::Primary),
        release_ev(PointerButton::Primary),
    ];
    map.remap(&[], &mut mouse);
    assert!(mouse.iter().all(|e| button_of(e) == PointerButton::Primary));
    // サイドボタン無しのペンの接触も直さない
    let mut tip = [
        press_ev(PointerButton::Primary),
        release_ev(PointerButton::Primary),
    ];
    map.remap(&[sample(at, true, false, false)], &mut tip);
    assert!(tip.iter().all(|e| button_of(e) == PointerButton::Primary));
    // サイドボタンを押した接触は右ボタン。押しの途中でサイドボタンを離しても、離すのも右ボタン
    let mut barrel = [press_ev(PointerButton::Primary)];
    map.remap(&[sample(at, true, true, false)], &mut barrel);
    assert_eq!(button_of(&barrel[0]), PointerButton::Secondary);
    assert!(map.is_secondary());
    let mut up = [release_ev(PointerButton::Primary)];
    map.remap(&[sample(at, false, false, false)], &mut up);
    assert_eq!(button_of(&up[0]), PointerButton::Secondary);
    assert!(!map.is_secondary(), "離したら終わり");
    // 本物の右ボタンと中ボタンはそのまま
    let mut others = [
        press_ev(PointerButton::Secondary),
        press_ev(PointerButton::Middle),
    ];
    map.remap(&[sample(at, true, true, false)], &mut others);
    assert_eq!(button_of(&others[0]), PointerButton::Secondary);
    assert_eq!(button_of(&others[1]), PointerButton::Middle);
    // 次の押しは、その押しの始めのサイドボタンで決め直す
    let mut next = [
        press_ev(PointerButton::Primary),
        release_ev(PointerButton::Primary),
    ];
    map.remap(&[sample(at, true, false, false)], &mut next);
    assert!(next.iter().all(|e| button_of(e) == PointerButton::Primary));
}

#[test]
fn the_pointer_is_a_magnifier_over_the_3d_view_while_ctrl_and_space_are_held() {
    let (mut h, rect) = cube_view();
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    move_to(&h, at);
    h.step();
    hold(&mut h, Modifiers::CTRL);
    hold_key(&mut h, Key::Space);
    move_to(&h, offset(at, 4.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomIn);
    hold(&mut h, Modifiers::CTRL | Modifiers::ALT);
    move_to(&h, offset(at, 8.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomOut, "Alt を足すと縮小");
    // ドラッグ中は、離すまで虫めがね
    hold(&mut h, Modifiers::CTRL);
    m_press(&h, at);
    h.step();
    move_to(&h, offset(at, 30.0, 0.0));
    h.step();
    assert_eq!(cursor(&h), egui::CursorIcon::ZoomIn);
    m_release(&h, offset(at, 30.0, 0.0));
    h.run();
    release_key(&mut h, Key::Space);
    release_mods(&mut h);
    assert_eq!(strokes(&h), 0);
}

// ───────── 4. 隠れたビューは押しの印を持ち越さない ─────────

#[test]
fn a_pen_press_does_not_outlive_a_canvas_that_was_hidden_and_the_next_touch_paints() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let mut h = app(1280.0, 800.0, 256);
        let c = canvas_rect(&h).center();
        let pen = Pen {
            emulate,
            ..Pen::tip()
        };
        // Space を押してペンで触れる（ビューを動かす押し）。触れたまま 3D のタブへ切り替える（キャンバスは隠れ、ペンの離れを受け取れない）
        hold_key(&mut h, Key::Space);
        pen.down(&mut h, c);
        assert!(h.state().state.canvas.pen_press.is_some());
        assert!(h.state().state.canvas.panning, "パンの途中");
        click_tab(&mut h, Tab::View3d);
        h.run();
        assert!(!h.state().state.ui.canvas_visible, "キャンバスは隠れた");
        assert!(
            h.state().state.canvas.pen_press.is_none(),
            "押しの印を捨てた"
        );
        assert!(!h.state().state.canvas.panning, "パンの途中も捨てた");
        // 隠れているあいだに離す（キャンバスは見ない）。戻す
        pen.up(&mut h, c);
        release_key(&mut h, Key::Space);
        click_tab(&mut h, Tab::Canvas);
        h.run();
        assert!(h.state().state.ui.canvas_visible);
        assert!(h.state().state.canvas.pen_press.is_none());
        nothing_started(&h);
        // 次に触れた押しは、前の押しの続きではなく、新しい押しとして描ける
        let c = canvas_rect(&h).center();
        pen.drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
        assert_eq!(strokes(&h), 1, "描けた {}", emulate == Emulate::Yes);
        assert!(h.state().state.canvas.pen_press.is_none());
    }
}

#[test]
fn a_pen_press_does_not_outlive_a_3d_view_that_was_hidden_and_never_picks_a_clone_source() {
    for emulate in [Emulate::No, Emulate::Yes] {
        let (mut h, rect) = cube_view_from(common::app(1100.0, 760.0, 256));
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        let pen = Pen {
            emulate,
            ..Pen::tip()
        };
        // クローンのブラシで、Alt を押してペンで触れる（回す押し。動かさずに離せば元を決める印も付く）
        let normal = h.state().state.m2.brush.effect;
        h.state_mut().state.m2.brush.effect = yolu_app::engine::BrushEffect::Clone {
            offset: Default::default(),
        };
        hold(&mut h, Modifiers::ALT);
        pen.down(&mut h, at);
        {
            let input = &h.state().state.view3d.input;
            assert!(
                input.pen_press.is_some() && input.nav.is_some(),
                "回している"
            );
            assert!(input.clone_press.is_some(), "元を決める印");
        }
        // 触れたまま、キャンバスのタブへ切り替える（3D は隠れ、ペンの離れを受け取れない）
        click_tab(&mut h, Tab::Canvas);
        h.run();
        assert!(!h.state().state.view3d.visible, "3D は隠れた");
        {
            let input = &h.state().state.view3d.input;
            assert!(input.pen_press.is_none(), "押しの印を捨てた");
            assert!(
                input.nav.is_none() && input.zoom.is_none(),
                "回しの途中も捨てた"
            );
            assert!(input.clone_press.is_none(), "元を決める印も捨てた");
        }
        pen.up(&mut h, at);
        release_mods(&mut h);
        // 戻したあと、離れた近くで別の押しを離しても、前の印で元が決まらない
        click_tab(&mut h, Tab::View3d);
        h.run();
        assert!(h.state().state.view3d.visible);
        h.state_mut().state.m2.brush.effect = normal;
        assert!(h.state().state.clone.source.is_none(), "元は決まっていない");
        assert!(h.state().state.view3d.input.clone_press.is_none());
        // 次に触れた押しは、新しい押しとして描ける
        pen.drag(&mut h, &[at, offset(at, 6.0, 0.0)]);
        assert!(painted(&h) > 0, "描けた {}", emulate == Emulate::Yes);
        assert_eq!(strokes(&h), 1);
        assert!(h.state().state.view3d.input.pen_press.is_none());
    }
}

// ───────── 5. ステンシルを動かしているあいだ（Y）は、ペンでもビューを動かさない ─────────

/// 2 × 2 の画像を貼り、置き場を初めに戻す（Y で動かすステンシルの試験の用意）。
fn stencil_ready(h: &mut H) {
    let s = &mut h.state_mut().state.stencil;
    s.set_image_rgba("a", 2, 2, &[255; 16]).unwrap();
    s.reset_placement();
}

/// ステンシルの置き場（動いたかを見る）。
fn placement(h: &H) -> (f32, f32, [f32; 2]) {
    let s = &h.state().state.stencil;
    (s.size, s.angle, s.center)
}

#[test]
fn while_y_moves_the_stencil_the_pen_with_alt_or_space_moves_only_the_stencil_in_2d() {
    // (押している修飾, Space を押すか)。Alt + 左はステンシルの拡縮、Space だけでは回す（ステンシルの決まり）
    for (name, mods, space) in [
        ("alt", Modifiers::ALT, false),
        ("space", Modifiers::NONE, true),
        ("alt+shift", Modifiers::ALT | Modifiers::SHIFT, false),
    ] {
        let mut h = app(1280.0, 800.0, 256);
        stencil_ready(&mut h);
        let c = canvas_rect(&h).center();
        let path = [
            offset(c, 60.0, 0.0),
            offset(c, 90.0, 40.0),
            offset(c, 60.0, 80.0),
        ];
        let (view, base) = (view_of(&h), placement(&h));
        let begin = |h: &mut H| {
            hold_key(h, Key::Y);
            if space {
                hold_key(h, Key::Space);
            }
            if mods != Modifiers::NONE {
                hold(h, mods);
            }
        };
        let end = |h: &mut H| {
            release_mods(h);
            if space {
                release_key(h, Key::Space);
            }
            release_key(h, Key::Y);
        };
        begin(&mut h);
        Pen::tip().emulated().drag(&mut h, &path);
        end(&mut h);
        assert_eq!(view_of(&h), view, "{name}: 表示は動かない");
        assert_eq!(strokes(&h), 0, "{name}: 描かない");
        let by_pen = placement(&h);
        assert_ne!(by_pen, base, "{name}: ステンシルが動いた");
        // マウスで同じ操作をしたときと同じだけ（二重に効いていない）
        h.state_mut().state.stencil.reset_placement();
        begin(&mut h);
        m_drag(&mut h, &path);
        end(&mut h);
        assert_eq!(placement(&h), by_pen, "{name}: ペンとマウスで同じ");
        assert_eq!(view_of(&h), view);
        nothing_started(&h);
    }
}

#[test]
fn while_y_moves_the_stencil_the_pen_never_orbits_or_pans_the_3d_view() {
    // サイドボタン（右ボタンの回す）・Alt（回す）・Alt + Shift（パン）・Space（パン）
    let cases: [(&str, bool, Modifiers, bool); 4] = [
        ("barrel", true, Modifiers::NONE, false),
        ("alt", false, Modifiers::ALT, false),
        ("alt+shift", false, Modifiers::ALT | Modifiers::SHIFT, false),
        ("space", false, Modifiers::NONE, true),
    ];
    for (name, barrel, mods, space) in cases {
        let (mut h, rect) = cube_view_from(common::app(1100.0, 760.0, 256));
        stencil_ready(&mut h);
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        let before = camera(&h);
        hold_key(&mut h, Key::Y);
        if space {
            hold_key(&mut h, Key::Space);
        }
        if mods != Modifiers::NONE {
            hold(&mut h, mods);
        }
        let pen = Pen {
            barrel,
            ..Pen::tip().emulated()
        };
        pen.drag(&mut h, &[at, offset(at, 40.0, 0.0), offset(at, 80.0, 20.0)]);
        release_mods(&mut h);
        if space {
            release_key(&mut h, Key::Space);
        }
        release_key(&mut h, Key::Y);
        let after = camera(&h);
        assert_eq!(after.yaw, before.yaw, "{name}: 回さない");
        assert_eq!(after.pitch, before.pitch, "{name}: 回さない");
        assert_eq!(after.target, before.target, "{name}: パンしない");
        assert_eq!(strokes(&h), 0, "{name}: 描かない");
        let input = &h.state().state.view3d.input;
        assert!(input.nav.is_none() && input.pen_press.is_none(), "{name}");
    }
}
