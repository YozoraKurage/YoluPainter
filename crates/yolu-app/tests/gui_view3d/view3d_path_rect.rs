//! 3D ビューのパスのツールで、点を矩形で選ぶドラッグ（Shift+ドラッグ）を取り残さない: フォーカスを失ったら捨て、離したのを取りこぼしたら
//! 最後の位置で確定する（2D のツールと同じ）。egui_kittest で、試しの立方体の上。
use crate::common;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};

use common::*;
use egui::{Event, Modifiers, Rect};
use egui_kittest::Harness;
use yolu_app::state::{Action, Tool};
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

/// 立方体の手前の面（z = −0.5）の点。
fn front(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, -0.5)
}

/// パスのツールで、手前の面に 3 点のパスを置いた 3D ビュー。
fn path_view() -> (Harness<'static, YoluApp>, Rect) {
    let (mut h, rect) = cube_view();
    h.state_mut().state.apply(Action::SelectTool(Tool::Path));
    h.run();
    for (x, y) in [(-0.35, -0.3), (0.0, 0.3), (0.35, -0.3)] {
        let at = screen_of(&h, rect, front(x, y));
        click(&mut h, at);
    }
    assert_eq!(h.state().state.path_layer().map(|_| ()), Some(()));
    (h, rect)
}

/// Shift を押して、矩形の選びを始め、動かす（離さない）。
fn start_rect(h: &mut Harness<'static, YoluApp>, rect: Rect) -> egui::Pos2 {
    let (a, b) = (
        screen_of(h, rect, front(-0.45, -0.45)),
        screen_of(h, rect, front(0.45, 0.45)),
    );
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    press_with(h, a, Modifiers::SHIFT);
    h.step();
    move_to(h, b);
    h.step();
    assert!(
        h.state().state.path.rect.is_some_and(|r| r.surface),
        "Shift+押して矩形の選びが始まる"
    );
    b
}

#[test]
fn losing_focus_drops_a_3d_rectangle_selection_and_a_later_release_selects_nothing() {
    let (mut h, rect) = path_view();
    let b = start_rect(&mut h, rect);
    h.event(Event::WindowFocused(false));
    h.run();
    assert!(
        h.state().state.path.rect.is_none(),
        "フォーカスを失ったら矩形を取り残さない"
    );
    // 戻ってから離しても、古い始点からの矩形で点を選ばない
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    release_with(&h, b, Modifiers::NONE);
    h.run();
    assert!(h.state().state.path.marked.is_none());
    assert!(h.state().state.path.rect.is_none());
}

#[test]
fn a_missed_release_finishes_the_3d_rectangle_selection_at_the_last_position() {
    let (mut h, rect) = path_view();
    let (a, b) = (
        screen_of(&h, rect, front(-0.45, -0.45)),
        screen_of(&h, rect, front(0.45, 0.45)),
    );
    // 最後の位置はビューが覚えている（ポインタが動いた）
    move_to(&h, b);
    h.step();
    // ウィンドウの外で離して、離した印を受け取れなかった状態: 矩形の選びは続いているのに、ボタンは押されていない
    h.state_mut().state.path.rect = Some(yolu_app::pathtool::RectDrag {
        source: yolu_app::state::StrokeSource::Mouse,
        surface: true,
        start: a,
        now: a,
    });
    h.run();
    assert!(
        h.state().state.path.rect.is_none(),
        "取りこぼしても矩形を取り残さない"
    );
    let marked = h.state().state.path.marked.clone();
    assert!(
        marked.is_some_and(|(_, points)| !points.is_empty()),
        "最後の位置までの矩形に入る点を選ぶ"
    );
}
