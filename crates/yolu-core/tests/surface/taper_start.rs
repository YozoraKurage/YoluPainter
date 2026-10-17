//! 入り抜きのあるブラシの、ストロークの形ごとの掛かり方（3D の面）: クリック（タップの揺れを含む。境は画面の点の半径と入りの長さの小さい方）は
//! 入り抜きの無いときと同じ点、Shift の直線（押した点と次の 1 つの入力の直後に `end_line` で閉じた線）は線全体に入り抜きを掛けない、手で描いた
//! 線は入りで 0 から育ち抜きで 0 へ細る（2D と同じ決まり）。
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::{Brush, BrushSettings, PressureSwitches, Rgba8, StrokeAssist};

use crate::anti_alias::{bytes, document, front, quad, screen, H, W};

fn brush(taper_in: f64, taper_out: f64) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 5.0,
            hardness: 1.0,
            spacing: 0.1,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        PressureSwitches {
            size: false,
            opacity: false,
            flow: false,
        },
    );
    b.assist = StrokeAssist {
        taper_in,
        taper_out,
        ..StrokeAssist::default()
    };
    b
}

/// 押した点 xs[0] から、続きの入力を足して（end_line_after 番目の入力の後で直線を閉じる）離した後のアルファ（世界の y 30 の横の線）。
fn draw(b: &Brush, xs: &[f64], end_line_after: Option<usize>) -> Vec<u8> {
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(xs[0], 30.0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for (i, x) in xs.iter().enumerate().skip(1) {
        s.add(&mut d, &mut stroke, screen(*x, 30.0), 1.0).unwrap();
        if end_line_after == Some(i + 1) {
            s.end_line().unwrap();
        }
        s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    let out = bytes(&d, l);
    d.end_stroke(stroke).unwrap();
    out.as_chunks::<4>().0.iter().map(|p| p[3]).collect()
}

/// 列 x の塗った画素の数（太さ）。
fn thickness(a: &[u8], x: usize) -> usize {
    let w = W as usize;
    (0..H as usize).filter(|&y| a[y * w + x] > 0).count()
}

const TAPERS: [(f64, f64); 3] = [(20.0, 20.0), (20.0, 0.0), (0.0, 20.0)];

#[test]
fn a_3d_click_with_a_taper_places_one_dot_of_the_full_diameter() {
    for xs in [&[48.0][..], &[48.0, 48.0, 48.0]] {
        let want = draw(&brush(0.0, 0.0), xs, None);
        assert!(thickness(&want, 48) >= 9, "全径の点");
        for (i, o) in TAPERS {
            assert_eq!(draw(&brush(i, o), xs, None), want, "入り {i}・抜き {o}");
        }
    }
}

#[test]
fn a_3d_shift_line_is_not_tapered_at_either_end() {
    let xs = [10.0, 86.0];
    let want = draw(&brush(0.0, 0.0), &xs, Some(2));
    assert!(thickness(&want, 10) >= 9 && thickness(&want, 86) >= 9);
    for (i, o) in TAPERS {
        assert_eq!(draw(&brush(i, o), &xs, Some(2)), want, "入り {i}・抜き {o}");
    }
}

#[test]
fn a_3d_hand_drawn_line_still_grows_from_nothing_and_thins_to_nothing() {
    let xs = [10.0, 30.0, 50.0, 70.0, 86.0];
    let a = draw(&brush(20.0, 20.0), &xs, None);
    let full = thickness(&a, 48);
    assert!(full >= 9);
    assert!(thickness(&a, 11) < full / 2, "入りの先頭は細い");
    assert!(thickness(&a, 85) < full / 2, "抜きの終わりは細い");
}

#[test]
fn a_3d_shaky_tap_is_the_same_dot_as_without_a_taper() {
    // 間隔より大きく揺れるタップと、行き来する揺れ（画面の点。押した点からの離れは、画面の半径・入りの長さより小さい）
    for xs in [
        &[48.0, 49.5, 50.5, 49.0][..],
        &[48.0, 50.0, 48.0, 50.0, 49.0],
    ] {
        let want = draw(&brush(0.0, 0.0), xs, None);
        assert!(thickness(&want, 49) >= 9, "全径の点");
        for (i, o) in TAPERS {
            assert_eq!(
                draw(&brush(i, o), xs, None),
                want,
                "入り {i}・抜き {o}・{xs:?}"
            );
        }
    }
}

#[test]
fn a_3d_shift_line_released_after_a_small_shake_keeps_its_end() {
    let xs = [10.0, 80.0, 81.5, 82.5, 81.0];
    let want = draw(&brush(0.0, 0.0), &xs, Some(2));
    for (i, o) in TAPERS {
        assert_eq!(draw(&brush(i, o), &xs, Some(2)), want, "入り {i}・抜き {o}");
    }
}
