//! 3D ビューのパスのツールで、点を矩形で選ぶドラッグ（Shift+ドラッグ）を取り残さない: フォーカスを失ったら捨て、離したのを取りこぼしたら
//! 最後の位置で確定する（2D のツールと同じ）。egui_kittest で、試しの立方体の上。
use crate::common;
use crate::common::drive::Drive;
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
    h.act(Action::SelectTool(Tool::Path));
    h.run();
    for (x, y) in [(-0.35, -0.3), (0.0, 0.3), (0.35, -0.3)] {
        let at = screen_of(&h, rect, front(x, y));
        click(&mut h, at);
    }
    assert_eq!(h.app().path_layer().map(|_| ()), Some(()));
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
        h.path().rect.is_some_and(|r| r.surface),
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
        h.path().rect.is_none(),
        "フォーカスを失ったら矩形を取り残さない"
    );
    // 戻ってから離しても、古い始点からの矩形で点を選ばない
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    release_with(&h, b, Modifiers::NONE);
    h.run();
    assert!(h.path().marked.is_none());
    assert!(h.path().rect.is_none());
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
    h.path_mut().rect = Some(yolu_app::pathtool::RectDrag {
        source: yolu_app::state::StrokeSource::Mouse,
        surface: true,
        start: a,
        now: a,
    });
    h.run();
    assert!(h.path().rect.is_none(), "取りこぼしても矩形を取り残さない");
    let marked = h.path().marked.clone();
    assert!(
        marked.is_some_and(|(_, points)| !points.is_empty()),
        "最後の位置までの矩形に入る点を選ぶ"
    );
}

#[test]
fn a_missed_release_finishes_a_3d_path_point_drag_where_it_was() {
    // 同じ押し・同じ動きを、取りこぼした離し(lost)と本物の離し(real)で終え、パスの点を比べる
    let mut ended = Vec::new();
    for lost in [true, false] {
        let (mut h, rect) = path_view();
        let undo = h.doc().undo_count();
        let before = surface_points(&h);
        let from = screen_of(&h, rect, front(0.0, 0.3));
        let to = screen_of(&h, rect, front(0.1, 0.0));
        press_with(&h, from, Modifiers::NONE);
        h.step();
        move_to(&h, to);
        h.step();
        assert!(
            h.path().drag.is_some_and(|d| d.surface),
            "面の上で点を掴んで動かしている"
        );
        if lost {
            // ウィンドウの外で離して、離した印を受け取れなかった状態
            lose_the_release_3d(&mut h);
            h.step();
        } else {
            release_with(&h, to, Modifiers::NONE);
            h.step();
        }
        assert!(
            h.path().drag.is_none(),
            "取りこぼしても点のドラッグを取り残さない"
        );
        assert_eq!(h.doc().undo_count(), undo + 1, "動かした点は確定する");
        let after = surface_points(&h);
        assert_ne!(after, before, "押す前の位置に戻っていない");
        ended.push(after);
    }
    assert_eq!(ended[0], ended[1], "最後の位置で確定する(本物の離しと同じ)");
}

/// パス(面の点のパス)の点の並び(比べるための文字)。
fn surface_points(h: &Harness<'static, YoluApp>) -> String {
    match h.app().path_layer().expect("パス").1 {
        yolu_core::LayerPath::Surface(p) => format!("{:?}", p.points),
        other => panic!("面のパスでない: {other:?}"),
    }
}
