//! 3D ビューで、入り抜きのあるブラシ（組み込みのブラシに入り・抜きを付けた物）で試しの立方体に描く: クリックは入り抜きの無いブラシと同じ
//! 全径の点、Shift の直線は両端まで全径、手で描いた線は両端が細る（core の `surface/taper_start` と同じ決まりを、画面の入力から）。
use crate::common::drive::Drive;
use crate::view3d_brush::{cube_view, screen_of};

use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

fn view(taper: (f64, f64)) -> (H, Rect) {
    let (mut h, rect) = cube_view();
    let s = h.app_mut();
    s.brush_mut().radius = 6.0;
    s.brush_mut().hardness = 1.0;
    s.brush_mut().pressure.size = false;
    s.m2_mut().brush.assist.taper_in = taper.0;
    s.m2_mut().brush.assist.taper_out = taper.1;
    (h, rect)
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

/// 手前の面（z −0.5）の点 points を、押したまま順になぞって離す（1 点ならクリック）。
fn stroke(h: &mut H, rect: Rect, points: &[(f32, f32)], m: Modifiers) {
    let at: Vec<Pos2> = points
        .iter()
        .map(|(x, y)| screen_of(h, rect, Vec3::new(*x, *y, -0.5)))
        .collect();
    stroke_at(h, &at, m);
}

/// 画面の点 at を、押したまま順になぞって離す。
fn stroke_at(h: &mut H, at: &[Pos2], m: Modifiers) {
    mouse(h, at[0], true, m);
    for p in &at[1..] {
        h.input_mut().events.push(Event::PointerMoved(*p));
        h.step();
    }
    mouse(h, *at.last().unwrap(), false, m);
    h.run();
}

fn composite(h: &H) -> Vec<u8> {
    h.doc().composite(h.doc().bounds()).unwrap()
}

fn painted(h: &H) -> usize {
    composite(h)
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0)
        .count()
}

#[test]
fn a_3d_click_with_a_tapered_brush_paints_the_same_full_dot_as_without_a_taper() {
    let draw = |taper| {
        let (mut h, rect) = view(taper);
        stroke(&mut h, rect, &[(0.0, 0.0)], Modifiers::NONE);
        h
    };
    let plain = draw((0.0, 0.0));
    assert!(painted(&plain) > 20);
    for taper in [(20.0, 20.0), (20.0, 0.0), (0.0, 20.0)] {
        let h = draw(taper);
        assert!(h.message().is_empty(), "{}", h.message());
        assert_eq!(composite(&h), composite(&plain), "{taper:?}");
    }
}

/// 塗った画素の太さを、塗った範囲の長い向きに沿って並べたもの（UV の上の線の、始めから終わりまでの列ごとの塗った画素の数）。
fn profile(h: &H) -> Vec<usize> {
    let doc = h.doc();
    let (w, hh) = (doc.width() as usize, doc.height() as usize);
    let c = composite(h);
    let on = |x: usize, y: usize| c[(y * w + x) * 4 + 3] > 0;
    let (mut x0, mut x1, mut y0, mut y1) = (w, 0, hh, 0);
    for y in 0..hh {
        for x in 0..w {
            if on(x, y) {
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
        }
    }
    assert!(x0 <= x1, "何も塗っていない");
    if x1 - x0 >= y1 - y0 {
        (x0..=x1)
            .map(|x| (y0..=y1).filter(|&y| on(x, y)).count())
            .collect()
    } else {
        (y0..=y1)
            .map(|y| (x0..=x1).filter(|&x| on(x, y)).count())
            .collect()
    }
}

/// 太さの並びの、中ほどと、両端から長さの 1/8 の所。
fn middle_and_ends(p: &[usize]) -> (usize, usize, usize) {
    let n = p.len();
    (p[n / 2], p[n / 8], p[n - 1 - n / 8])
}

#[test]
fn a_3d_shift_line_with_a_tapered_brush_is_full_to_both_ends() {
    let draw = |taper, shake: &[(f32, f32)]| {
        let (mut h, rect) = view(taper);
        stroke(&mut h, rect, &[(-0.3, 0.0)], Modifiers::NONE);
        let end = screen_of(&h, rect, Vec3::new(0.3, 0.0, -0.5));
        let mut at = vec![end];
        at.extend(shake.iter().map(|(x, y)| end + egui::vec2(*x, *y)));
        stroke_at(&mut h, &at, Modifiers::SHIFT);
        h
    };
    // 直線を閉じた後、終点のそばで揺れて離しても同じ（Shift の押しのぶれの抑え 8 点を超え、画面の半径（13 点ほど）より近い揺れ）
    for shake in [&[][..], &[(9.0, 0.0), (11.0, 2.0), (10.0, -1.0)]] {
        let plain = draw((0.0, 0.0), shake);
        let p = profile(&plain);
        let (middle, head, tail) = middle_and_ends(&p);
        assert!(p.len() > 30, "直線が描けた: {p:?}");
        assert!(
            middle >= 4 && head >= middle * 4 / 5 && tail >= middle * 4 / 5,
            "{p:?}"
        );
        let h = draw((20.0, 20.0), shake);
        assert_eq!(profile(&h), p, "{shake:?}");
        assert_eq!(composite(&h), composite(&plain), "{shake:?}");
    }
}

#[test]
fn a_3d_hand_drawn_line_with_a_tapered_brush_still_thins_at_both_ends() {
    let draw = |taper| {
        let (mut h, rect) = view(taper);
        let points: Vec<(f32, f32)> = (0..=8).map(|i| (-0.3 + 0.075 * i as f32, 0.0)).collect();
        stroke(&mut h, rect, &points, Modifiers::NONE);
        h
    };
    let plain = profile(&draw((0.0, 0.0)));
    let tapered = profile(&draw((40.0, 40.0)));
    let (middle, head, tail) = middle_and_ends(&tapered);
    assert_eq!(middle, middle_and_ends(&plain).0, "{tapered:?} {plain:?}");
    assert!(
        head < middle && tail < middle,
        "両端がそれぞれ細る: {tapered:?}"
    );
}

#[test]
fn a_3d_shaky_tap_with_a_tapered_brush_is_the_same_dot_as_without_a_taper() {
    // 画面の 1〜2 点ほど揺れるタップと、行き来する揺れ（ダブの間隔より大きく、画面の半径より小さい）
    let shakes: [&[(f32, f32)]; 2] = [
        &[(0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (1.0, 0.0)],
        &[(0.0, 0.0), (2.0, 0.0), (0.0, 0.0), (2.0, 0.0), (1.0, 0.0)],
    ];
    for offsets in shakes {
        let draw = |taper| {
            let (mut h, rect) = view(taper);
            let c = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
            let at: Vec<Pos2> = offsets
                .iter()
                .map(|(x, y)| c + egui::vec2(*x, *y))
                .collect();
            stroke_at(&mut h, &at, Modifiers::NONE);
            h
        };
        let plain = draw((0.0, 0.0));
        assert!(painted(&plain) > 20);
        for taper in [(20.0, 20.0), (20.0, 0.0), (0.0, 20.0)] {
            assert_eq!(
                composite(&draw(taper)),
                composite(&plain),
                "{taper:?} {offsets:?}"
            );
        }
    }
}
