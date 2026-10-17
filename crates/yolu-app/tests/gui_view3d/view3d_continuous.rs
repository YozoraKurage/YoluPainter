//! 3D ビューの連続吹き付けとリボン（egui_kittest。試しの立方体の上で）: 連続吹き付けのブラシでマウスを止めて押している間、フレームごとに
//! 点が足されて押した所が濃くなる。リボンのブラシの曲がった線。
use crate::common::drive::{Drive, DriveApp};
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};
use crate::view3d_select::pen_sample;

use egui::{Event, Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

fn brush(h: &mut H, continuous: bool) {
    let s = h.app_mut();
    s.brush_mut().radius = 6.0;
    s.brush_mut().hardness = 0.5;
    s.brush_mut().flow = 0.1;
    s.brush_mut().pressure.size = false;
    s.m2_mut().brush.tip.continuous = continuous;
}

/// 離してから、描いている最中でなくなるまでフレームを進める。
fn drain(h: &mut H) {
    let mut frames = 0;
    while h.app().is_stroking() {
        h.step();
        frames += 1;
        assert!(frames < 5000, "確定しない");
    }
}

fn alpha_sum(h: &H) -> u64 {
    let doc = h.doc();
    let px = doc.composite(doc.bounds()).unwrap();
    px.as_chunks::<4>().0.iter().map(|p| u64::from(p[3])).sum()
}

/// 手前の面の真ん中で押し、`frames` フレーム動かさずに押してから離す。受けた点の数と、全部のアルファの和。
fn hold(continuous: bool, frames: usize) -> (usize, u64) {
    let (mut h, rect) = cube_view();
    brush(&mut h, continuous);
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    press_with(&h, at, Modifiers::NONE);
    h.step();
    for _ in 0..frames {
        h.step();
    }
    let points = h.view3d().view.input.stroke_points;
    release_with(&h, at, Modifiers::NONE);
    h.step();
    drain(&mut h);
    (points, alpha_sum(&h))
}

#[test]
fn holding_the_pointer_still_on_the_surface_adds_points_and_darkens_the_spot() {
    let (points, dark) = hold(true, 30);
    let (plain_points, plain) = hold(false, 30);
    assert_eq!(plain_points, 1, "切のブラシは点を足さない");
    assert!(points >= 25, "{points}");
    assert!(dark > plain, "{dark} > {plain}");
}

/// ペンを `at` に置く（接触か離しか）。OS の時刻は 1 フレームの 17 ms ごと。
fn pen_at(h: &mut H, at: Pos2, contact: bool, time_ms: u32) {
    h.push_pen(pen_sample(at, contact, time_ms));
    h.step();
}

#[test]
fn holding_the_pen_still_on_the_surface_adds_points_until_it_moves_again_and_stops_after_the_release(
) {
    let mut darkness = Vec::new();
    for continuous in [true, false] {
        let (mut h, rect) = cube_view();
        brush(&mut h, continuous);
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        // OS の時刻（1 秒から）はフレームの時刻（egui）と違う時計
        pen_at(&mut h, at, true, 1000);
        for _ in 0..30 {
            h.step();
        }
        let held = h.view3d().view.input.stroke_points;
        if continuous {
            assert!(held >= 25, "止めている間も点が足される: {held}");
        } else {
            assert_eq!(held, 1, "切のブラシは足さない");
        }
        // 動かし直すと、その点が入る。止めた後の続きの点はまた足される
        let moved = screen_of(&h, rect, Vec3::new(0.2, 0.0, -0.5));
        pen_at(&mut h, moved, true, 1000 + 17 * 31);
        let after_move = h.view3d().view.input.stroke_points;
        assert!(after_move > held, "{continuous}: {after_move} > {held}");
        for _ in 0..30 {
            h.step();
        }
        let again = h.view3d().view.input.stroke_points;
        if continuous {
            assert!(again >= after_move + 25, "{again} >= {after_move} + 25");
        } else {
            assert_eq!(again, after_move);
        }
        // 離した後は増えない
        pen_at(&mut h, moved, false, 1000 + 17 * 62);
        drain(&mut h);
        h.run();
        assert!(h.view3d().view.input.held.is_none());
        let points = h.view3d().view.input.stroke_points;
        let alpha = alpha_sum(&h);
        for _ in 0..30 {
            h.step();
        }
        assert_eq!(h.view3d().view.input.stroke_points, points);
        assert_eq!(alpha_sum(&h), alpha);
        darkness.push(alpha);
    }
    // 渡した点の時刻が OS の時計で進むので、止めた間のダブが時刻の間隔ごとに置かれて濃くなる
    assert!(darkness[0] > darkness[1], "{darkness:?}");
}

#[test]
fn escape_focus_loss_and_release_stop_the_held_points_and_the_frame_requests() {
    for how in ["escape", "focus lost", "release"] {
        let (mut h, rect) = cube_view();
        brush(&mut h, true);
        let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
        press_with(&h, at, Modifiers::NONE);
        h.step();
        for _ in 0..10 {
            h.step();
        }
        assert!(
            h.view3d().view.input.stroke_points >= 8,
            "{how}: 試験の前提: 点が足されている"
        );
        match how {
            "escape" => crate::common::key(&h, egui::Key::Escape, Modifiers::NONE),
            "focus lost" => h.event(Event::WindowFocused(false)),
            _ => release_with(&h, at, Modifiers::NONE),
        }
        h.step();
        drain(&mut h);
        // 次のフレームを頼み続けていれば、ここで終わらない（`run` は頼みが止まるまで進める）
        h.run();
        assert!(
            h.view3d().view.input.held.is_none(),
            "{how}: 最後の点を手放す"
        );
        let points = h.view3d().view.input.stroke_points;
        for _ in 0..20 {
            h.step();
        }
        assert_eq!(h.view3d().view.input.stroke_points, points, "{how}");
        assert!(!h.app().is_stroking(), "{how}");
    }
}

fn view_shot(h: &mut H, rect: Rect, name: &str) {
    crate::common::shot::clean_region(h, name, rect);
}

#[test]
fn snapshot_brush_continuous_3d() {
    let (mut h, rect) = cube_view();
    brush(&mut h, true);
    let a = Vec3::new(-0.4, 0.0, -0.5);
    let b = Vec3::new(0.4, 0.0, -0.5);
    let point = |h: &H, t: f32| screen_of(h, rect, a + (b - a) * t);
    let first = point(&h, 0.0);
    press_with(&h, first, Modifiers::NONE);
    h.step();
    for i in 1..=20 {
        let p = point(&h, i as f32 / 40.0);
        h.input_mut().events.push(Event::PointerMoved(p));
        h.step();
    }
    // 線の中ほどで 1 秒（60 フレーム）止める
    for _ in 0..60 {
        h.step();
    }
    let mut last = Pos2::ZERO;
    for i in 21..=40 {
        last = point(&h, i as f32 / 40.0);
        h.input_mut().events.push(Event::PointerMoved(last));
        h.step();
    }
    release_with(&h, last, Modifiers::NONE);
    h.step();
    drain(&mut h);
    h.run();
    view_shot(&mut h, rect, "brush_continuous_3d");
}

#[test]
fn snapshot_brush_ribbon_3d() {
    let (mut h, rect) = cube_view();
    let s = h.app_mut();
    s.brush_mut().radius = 8.0;
    s.brush_mut().spacing = 0.1;
    s.brush_mut().pressure.size = false;
    let b = &mut s.m2_mut().brush;
    b.tip.images = vec![stripes()];
    b.tip.ribbon = true;
    let points: Vec<Pos2> = (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0;
            screen_of(
                &h,
                rect,
                Vec3::new(-0.4 + 0.8 * t, 0.25 * (t * 6.0).sin(), -0.5),
            )
        })
        .collect();
    press_with(&h, points[0], Modifiers::NONE);
    h.step();
    for p in &points[1..] {
        h.input_mut().events.push(Event::PointerMoved(*p));
        h.step();
    }
    release_with(&h, *points.last().unwrap(), Modifiers::NONE);
    h.step();
    drain(&mut h);
    h.run();
    view_shot(&mut h, rect, "brush_ribbon_3d");
}

/// 斜めの縞の筆先（96 × 24。2D のテストと同じ形）。
fn stripes() -> std::sync::Arc<yolu_core::BrushTip> {
    let (w, h) = (96u32, 24u32);
    let alpha = (0..h)
        .flat_map(|y| (0..w).map(move |x| if ((x + y) / 6) % 2 == 0 { 255 } else { 50 }))
        .collect();
    std::sync::Arc::new(yolu_core::BrushTip::new("stripes", w, h, alpha).unwrap())
}
