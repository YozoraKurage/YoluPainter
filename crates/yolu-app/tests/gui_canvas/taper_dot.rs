//! 入り抜きのあるブラシ（組み込みのブラシに入り・抜きを付けた物。既定のブラシに入り抜きを持つ物は無い）でキャンバスに描く: マウスでもペンでも、
//! クリック（少し揺れるタップも）は入り抜きの無いブラシと同じ全径の点、Shift の直線は両端まで全径（離す前に終点のそばで揺れても）、
//! 手で描いた線は両端が細る（core の `edit/taper_start` と同じ決まりを、画面の入力から）。
use crate::common;
use crate::common::drive::Drive;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use yolu_app::engine::composite_pixel;
use yolu_app::pen::PenSample;
use yolu_app::state::AppState;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

const SIZE: u32 = 64;

fn app(taper: (f64, f64)) -> H {
    let mut h = common::app(1280.0, 800.0, SIZE);
    let s = h.app_mut();
    s.brush_mut().radius = 3.0;
    s.brush_mut().hardness = 1.0;
    s.brush_mut().pressure.size = false;
    s.m2_mut().brush.assist.taper_in = taper.0;
    s.m2_mut().brush.assist.taper_out = taper.1;
    h
}

fn point(h: &H, x: f64, y: f64) -> Pos2 {
    let s = h.app();
    s.view()
        .view(
            s.ui().canvas_rect.unwrap(),
            s.doc().width(),
            s.doc().height(),
        )
        .to_screen(x, y)
}

fn mouse(h: &mut H, p: Pos2, down: bool, m: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(m));
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: m,
    });
    h.step();
}

fn pen(h: &mut H, p: Pos2, contact: bool, m: Modifiers, time_ms: u32) {
    h.input_mut().events.push(Event::ModifiersChanged(m));
    h.state().pen().push(PenSample {
        pos: [p.x, p.y],
        pressure: if contact { 1.0 } else { 0.0 },
        tilt: Default::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    });
    // ペンと同時に来る代わりのマウスの入力（二重に描かない）
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: contact,
        modifiers: m,
    });
    h.step();
}

/// 文書の点 points を、押したまま順になぞって離す（1 点ならクリック）。
fn stroke(h: &mut H, is_pen: bool, points: &[(f64, f64)], m: Modifiers) {
    let at: Vec<Pos2> = points.iter().map(|(x, y)| point(h, *x, *y)).collect();
    if is_pen {
        for (i, p) in at.iter().enumerate() {
            pen(h, *p, true, m, 100 + 20 * i as u32);
        }
        pen(h, *at.last().unwrap(), false, m, 100 + 20 * at.len() as u32);
    } else {
        mouse(h, at[0], true, m);
        for p in &at[1..] {
            h.input_mut().events.push(Event::PointerMoved(*p));
            h.step();
        }
        mouse(h, *at.last().unwrap(), false, m);
    }
    h.run();
}

fn composite(s: &AppState) -> Vec<u8> {
    s.doc().composite(s.doc().bounds()).unwrap()
}

/// 列 x の塗った画素の数（太さ）。
fn thickness(s: &AppState, x: u32) -> usize {
    (0..SIZE)
        .filter(|y| composite_pixel(s.doc(), x, *y)[3] > 0)
        .count()
}

#[test]
fn a_click_with_a_tapered_brush_paints_the_same_full_dot_as_without_a_taper() {
    for is_pen in [false, true] {
        let draw = |taper| {
            let mut h = app(taper);
            stroke(&mut h, is_pen, &[(32.0, 32.0)], Modifiers::NONE);
            h
        };
        let plain = draw((0.0, 0.0));
        assert!(
            composite_pixel(plain.app().doc(), 32, 32)[3] > 0,
            "{is_pen}"
        );
        for taper in [(12.0, 12.0), (12.0, 0.0), (0.0, 12.0)] {
            let h = draw(taper);
            assert_eq!(
                composite(h.app()),
                composite(plain.app()),
                "{is_pen}: {taper:?}"
            );
        }
    }
}

#[test]
fn a_shift_line_with_a_tapered_brush_is_full_to_both_ends() {
    // 直線を閉じた後、終点のそばで 1〜2 画素揺れて離しても同じ
    let shakes: [&[(f64, f64)]; 2] = [
        &[(56.0, 32.0)],
        &[(56.0, 32.0), (57.0, 33.0), (58.0, 32.0), (57.0, 31.0)],
    ];
    for is_pen in [false, true] {
        for shake in shakes {
            let draw = |taper| {
                let mut h = app(taper);
                stroke(&mut h, is_pen, &[(8.0, 32.0)], Modifiers::NONE);
                stroke(&mut h, is_pen, shake, Modifiers::SHIFT);
                h
            };
            let plain = draw((0.0, 0.0));
            let h = draw((12.0, 12.0));
            assert_eq!(
                composite(h.app()),
                composite(plain.app()),
                "{is_pen} {shake:?}"
            );
            let s = h.app();
            let middle = thickness(s, 32);
            assert!(middle >= 5, "直線が描けた");
            assert!(
                thickness(s, 10) >= middle && thickness(s, 54) >= middle,
                "両端まで全径"
            );
        }
    }
}

#[test]
fn a_shaky_tap_with_a_tapered_brush_paints_the_same_dot_as_without_a_taper() {
    // 1〜2 画素揺れるタップと、行き来する揺れ（ダブの間隔より大きく、半径より小さい）
    let shakes: [&[(f64, f64)]; 2] = [
        &[(32.0, 32.0), (33.0, 32.0), (34.0, 33.0), (33.0, 32.0)],
        &[
            (32.0, 32.0),
            (34.0, 32.0),
            (32.0, 32.0),
            (34.0, 32.0),
            (33.0, 32.0),
        ],
    ];
    for is_pen in [false, true] {
        for points in shakes {
            let draw = |taper| {
                let mut h = app(taper);
                stroke(&mut h, is_pen, points, Modifiers::NONE);
                h
            };
            let plain = draw((0.0, 0.0));
            assert!(thickness(plain.app(), 33) >= 5, "{is_pen}");
            for taper in [(12.0, 12.0), (12.0, 0.0), (0.0, 12.0)] {
                assert_eq!(
                    composite(draw(taper).app()),
                    composite(plain.app()),
                    "{is_pen}: {taper:?} {points:?}"
                );
            }
        }
    }
}

#[test]
fn a_hand_drawn_line_with_a_tapered_brush_still_thins_at_both_ends() {
    for is_pen in [false, true] {
        let mut h = app((12.0, 12.0));
        let points: Vec<(f64, f64)> = (0..=8).map(|i| (8.0 + 6.0 * i as f64, 32.0)).collect();
        stroke(&mut h, is_pen, &points, Modifiers::NONE);
        let s = h.app();
        let middle = thickness(s, 32);
        assert!(middle >= 5, "{is_pen}: {middle}");
        assert!(thickness(s, 9) < middle, "{is_pen}: 入り");
        assert!(thickness(s, 55) < middle, "{is_pen}: 抜き");
    }
}
