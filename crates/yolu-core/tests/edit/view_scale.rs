//! 画面上のサイズで指定（`Controls::size_by_screen`）: ストロークの始めに、半径 = 画面上の大きさ ÷ 表示倍率（`Brush::at_view_scale`）の文書の
//! 画素へ直して固定する。表示倍率 50%・100%・200% で、始めの半径が画面の大きさに合い、途中で倍率が変わっても、始めたストロークは変わらない。
//! 切（既定）は表示倍率によらず今までと同じ。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSample, BrushSettings, Document, LayerId, PressureSwitches, Rgba8};

const SIZE: u32 = 128;

fn canvas() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(SIZE, SIZE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

/// 半径 radius（画面上なら画面の点、そうでなければ文書の画素）の硬い丸いブラシ。
fn round(radius: f64, by_screen: bool) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius,
            hardness: 1.0,
            spacing: 0.1,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.controls.size_by_screen = by_screen;
    b
}

fn sample(x: f64) -> BrushSample {
    BrushSample::new(x, 64.0, 1.0, 0.0, DVec2::ZERO).unwrap()
}

/// 横の線（x 24〜104、y 64）を引いた、不透明な画素の行 y の数（線の太さ）。
fn thickness(brush: &Brush) -> usize {
    let (mut d, l) = canvas();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for x in [24.0, 64.0, 104.0] {
        s.add_sample(&mut d, sample(x)).unwrap();
    }
    d.end_stroke(s).unwrap();
    let bytes = d.composite(d.bounds()).unwrap();
    (0..SIZE as usize)
        .filter(|y| bytes[(y * SIZE as usize + 64) * 4 + 3] > 0)
        .count()
}

#[test]
fn the_radius_is_the_size_on_screen_divided_by_the_view_scale() {
    // 画面上 10 点の半径: 表示 50%（文書の画素 1 が 0.5 点）で 20 画素、100% で 10、200% で 5
    let b = round(10.0, true);
    for (scale, radius) in [(0.5, 20.0), (1.0, 10.0), (2.0, 5.0)] {
        let at = b.at_view_scale(scale);
        assert_eq!(at.base.radius, radius, "{scale}");
        at.validate().unwrap();
        // 描いた線の太さも、その半径の固定のブラシと同じ（直径 = 画面の大きさ ÷ 倍率）
        assert_eq!(
            thickness(&at),
            thickness(&round(radius, false)),
            "倍率 {scale}"
        );
    }
    // 倍率が小さいほど太い（文書の画素で見て、画面の大きさ一定の線は拡大するほど細い）
    assert!(thickness(&b.at_view_scale(0.5)) > thickness(&b.at_view_scale(1.0)));
    assert!(thickness(&b.at_view_scale(1.0)) > thickness(&b.at_view_scale(2.0)));
}

#[test]
fn the_radius_is_fixed_when_the_stroke_starts_even_if_the_scale_changes_later() {
    let b = round(10.0, true);
    let (mut d, l) = canvas();
    // 倍率 200% で始める。途中で表示を 50% へ縮小しても（そのときのブラシを作っても）、始めたストロークは変わらない
    let at_start = b.at_view_scale(2.0);
    let mut s = d.begin_brush_stroke(l, &at_start).unwrap();
    s.add_sample(&mut d, sample(24.0)).unwrap();
    let _zoomed_out = b.at_view_scale(0.5);
    s.add_sample(&mut d, sample(64.0)).unwrap();
    s.add_sample(&mut d, sample(104.0)).unwrap();
    d.end_stroke(s).unwrap();
    let bytes = d.composite(d.bounds()).unwrap();
    let (mut fixed_doc, fl) = canvas();
    let fixed = round(5.0, false);
    let mut fs = fixed_doc.begin_brush_stroke(fl, &fixed).unwrap();
    for x in [24.0, 64.0, 104.0] {
        fs.add_sample(&mut fixed_doc, sample(x)).unwrap();
    }
    fixed_doc.end_stroke(fs).unwrap();
    assert_eq!(bytes, fixed_doc.composite(fixed_doc.bounds()).unwrap());
    // 元のブラシの設定の大きさは、始めのたびに画面の値のまま変わらない
    assert_eq!(b.base.radius, 10.0);
}

#[test]
fn without_the_setting_the_view_scale_changes_nothing() {
    let b = round(10.0, false);
    for scale in [0.01, 0.5, 1.0, 2.0, 64.0] {
        assert_eq!(b.at_view_scale(scale), b, "{scale}");
    }
    // 倍率が使えない値（0・負・有限でない）は 1 とみなす
    let on = round(10.0, true);
    for scale in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        assert_eq!(on.at_view_scale(scale).base.radius, 10.0, "{scale}");
    }
    // 極端な倍率でも、半径は検査を通る範囲に収まる
    for scale in [1e-9, 1e9] {
        let at = on.at_view_scale(scale);
        assert!(at.base.radius > 0.0 && at.base.radius <= 65536.0, "{scale}");
        at.validate().unwrap();
    }
}
