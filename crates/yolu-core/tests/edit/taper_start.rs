//! 入り抜きのあるブラシの、ストロークの形ごとの掛かり方（2D）: クリック（押した点から min(半径, 入りの長さ) より離れずに離したストローク。
//! ペンのタップの揺れを含む）は入り抜きの無いときと同じ点、Shift の直線（最初の 2 つの入力の直後に `end_line` で閉じた線）は線全体に
//! 入り抜きを掛けない（直線の終点のそばで離せば抜きも）、手で描いた線は入りで 0 から育ち、抜きで 0 へ細る。入りのあるブラシは形が分かるまで
//! ダブを待たせ、入り抜きの無いブラシは待たせない。
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, Channel, Document, PressureSwitches, Rgba8, StrokeAssist, TaperTargets,
};

const W: u32 = 128;
const H: u32 = 48;

fn brush(taper_in: f64, taper_out: f64) -> Brush {
    brush_sized(6.0, taper_in, taper_out)
}

fn brush_sized(radius: f64, taper_in: f64, taper_out: f64) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius,
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
        taper_targets: TaperTargets::default(),
        ..StrokeAssist::default()
    };
    b
}

fn alpha(doc: &Document) -> Vec<u8> {
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[3])
        .collect()
}

/// 描き方: 入力の点（y は 24.5）と、何番目の入力の後で直線を閉じるか（Shift の直線は 2）。
struct Input<'a> {
    xs: &'a [f64],
    end_line_after: Option<usize>,
}

/// 入力を順に足し、離した後のアルファ。
fn draw(b: &Brush, input: &Input) -> Vec<u8> {
    let mut doc = Document::with_tile_size(W, H, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for (i, x) in input.xs.iter().enumerate() {
        s.add_point(&mut doc, *x, 24.5, 1.0, DVec2::ZERO).unwrap();
        if input.end_line_after == Some(i + 1) {
            s.end_line(&mut doc).unwrap();
        }
    }
    doc.end_stroke(s).unwrap();
    alpha(&doc)
}

/// 列 x の塗った画素の数（太さ）。
fn thickness(a: &[u8], x: usize) -> usize {
    (0..H as usize)
        .filter(|&y| a[y * W as usize + x] > 0)
        .count()
}

const TAPERS: [(f64, f64); 3] = [(20.0, 20.0), (20.0, 0.0), (0.0, 20.0)];

#[test]
fn a_click_with_a_taper_places_one_dot_of_the_full_diameter() {
    let plain = brush(0.0, 0.0);
    // 1 回のクリックと、動かずに入力が続いたクリック（筆圧だけが来る）
    for xs in [&[60.5][..], &[60.5, 60.5, 60.5]] {
        let input = Input {
            xs,
            end_line_after: None,
        };
        let want = draw(&plain, &input);
        assert!(thickness(&want, 60) >= 11, "全径の点");
        for (i, o) in TAPERS {
            assert_eq!(draw(&brush(i, o), &input), want, "入り {i}・抜き {o}");
        }
    }
}

#[test]
fn a_stroke_shorter_than_one_spacing_is_a_dot_too() {
    // 間隔（直径 12 × 0.1）より短い動き: ダブは押した点の 1 つだけなので、クリックと同じく全径
    let input = Input {
        xs: &[60.5, 61.0],
        end_line_after: None,
    };
    let want = draw(&brush(0.0, 0.0), &input);
    for (i, o) in TAPERS {
        assert_eq!(draw(&brush(i, o), &input), want, "入り {i}・抜き {o}");
    }
}

#[test]
fn a_shift_line_is_not_tapered_at_either_end() {
    let input = Input {
        xs: &[10.5, 110.5],
        end_line_after: Some(2),
    };
    let want = draw(&brush(0.0, 0.0), &input);
    assert!(thickness(&want, 10) >= 11 && thickness(&want, 110) >= 11);
    for (i, o) in TAPERS {
        assert_eq!(draw(&brush(i, o), &input), want, "入り {i}・抜き {o}");
    }
}

#[test]
fn a_shift_line_continued_by_hand_keeps_its_start_and_thins_only_the_end_of_the_drag() {
    let input = Input {
        xs: &[10.5, 40.5, 70.5, 110.5],
        end_line_after: Some(2),
    };
    let a = draw(&brush(20.0, 20.0), &input);
    let plain = draw(&brush(0.0, 0.0), &input);
    for x in [10, 30, 45, 60] {
        assert_eq!(thickness(&a, x), thickness(&plain, x), "x {x}");
    }
    assert!(
        thickness(&a, 108) < thickness(&a, 80),
        "続けて描いた所の抜き"
    );
}

#[test]
fn a_hand_drawn_line_still_grows_from_nothing_and_thins_to_nothing() {
    let input = Input {
        xs: &[10.5, 60.5, 110.5],
        end_line_after: None,
    };
    let a = draw(&brush(20.0, 20.0), &input);
    let full = thickness(&a, 60);
    assert!(full >= 11);
    assert!(thickness(&a, 12) < full / 2, "入りの先頭は細い");
    assert!(thickness(&a, 109) < full / 2, "抜きの終わりは細い");
    // 入りだけ・抜きだけも、それぞれの端だけが細る
    let a = draw(&brush(20.0, 0.0), &input);
    assert!(thickness(&a, 12) < full / 2);
    assert_eq!(thickness(&a, 109), full);
    let a = draw(&brush(0.0, 20.0), &input);
    assert_eq!(thickness(&a, 12), full);
    assert!(thickness(&a, 109) < full / 2);
}

#[test]
fn a_taper_in_waits_for_the_shape_of_the_stroke_and_no_taper_does_not_wait() {
    let painted = |b: &Brush, xs: &[f64]| {
        let mut doc = Document::with_tile_size(W, H, 32).unwrap();
        let layer = doc.add_layer("a").unwrap();
        let mut s = doc.begin_brush_stroke(layer, b).unwrap();
        for x in xs {
            s.add_point(&mut doc, *x, 24.5, 1.0, DVec2::ZERO).unwrap();
        }
        let during = alpha(&doc);
        doc.cancel_stroke(s);
        during
    };
    let xs = [10.5, 60.5, 110.5];
    // 入り抜きが無ければ、押した点からすぐ描く
    assert!(thickness(&painted(&brush(0.0, 0.0), &xs[..1]), 10) > 0);
    // 入りがあると、2 つ目の入力まで（クリックか・Shift の直線か・手で描いた線かが分かるまで）は描かず、3 つ目で手で描いた線と分かって描く
    let b = brush(20.0, 0.0);
    assert!(painted(&b, &xs[..2]).iter().all(|&v| v == 0));
    let a = painted(&b, &xs);
    assert!(thickness(&a, 60) >= 11);
    assert!(thickness(&a, 12) < 6);
}

#[test]
fn a_shaky_pen_tap_is_the_same_dot_as_without_a_taper() {
    // 間隔（直径 × 0.1）より大きく揺れるタップと、行き来する揺れ。押した点からの離れは半径・入りの長さより小さい
    let cases: [(f64, &[f64]); 4] = [
        (6.0, &[60.5, 61.5, 62.5, 61.0]),
        (6.0, &[60.5, 62.5, 60.5, 62.5, 61.5]),
        (3.0, &[60.5, 61.5, 60.5, 61.5, 60.5]),
        (3.0, &[60.5, 62.0, 61.0]),
    ];
    for (radius, xs) in cases {
        let input = Input {
            xs,
            end_line_after: None,
        };
        let want = draw(&brush_sized(radius, 0.0, 0.0), &input);
        assert!(
            thickness(&want, 61) as f64 >= 2.0 * radius - 1.0,
            "全径の点: {radius} {xs:?}"
        );
        for (i, o) in TAPERS {
            assert_eq!(
                draw(&brush_sized(radius, i, o), &input),
                want,
                "半径 {radius}・入り {i}・抜き {o}・{xs:?}"
            );
        }
    }
}

#[test]
fn a_shift_line_released_after_a_small_shake_keeps_its_end() {
    // 直線を閉じた後、終点から半径より小さく揺れて離す: 直線の終わりに抜きを掛けない
    let input = Input {
        xs: &[10.5, 110.5, 111.5, 112.5, 111.0],
        end_line_after: Some(2),
    };
    let want = draw(&brush(0.0, 0.0), &input);
    for (i, o) in TAPERS {
        assert_eq!(draw(&brush(i, o), &input), want, "入り {i}・抜き {o}");
    }
}
