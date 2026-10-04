//! 選択範囲（形・組み合わせ・変更・自動選択）と、文書の選択範囲の Undo・塗りつぶし・ブラシの切り取り・2D の対称。
//! 画素そのものを C# と照らすのは golden.rs（sel_*・sym_*）。ここは既知の答え・取消・予算・壊れた入力を確かめる。

// 画素の格子を (x, y) の添字で見比べる試験なので、添字の範囲の繰り返しの方が読みやすい。
#![allow(clippy::needless_range_loop)]

use yolu_core::glam::DVec2;
use yolu_core::selection::{
    DEFAULT_WORKING_BUDGET_BYTES as BUDGET, MAX_MODIFY_RADIUS, MAX_POLYGON_POINTS,
};
use yolu_core::{
    Brush, BrushSettings, CanvasSymmetry, Channel, CoreError, Document, LayerId, Rgba8,
    SelectionCombine, SelectionMask, SymmetryMode, TileCoord,
};

fn doc(w: u32, h: u32, tile: u32) -> Document {
    Document::with_tile_size(w, h, tile).unwrap()
}
fn amounts(m: &SelectionMask) -> Vec<u8> {
    m.to_canvas_bytes()
}
fn count(m: &SelectionMask, f: impl Fn(u8) -> bool) -> usize {
    amounts(m).into_iter().filter(|a| f(*a)).count()
}
fn px(d: &Document, layer: LayerId, x: u32, y: u32) -> Rgba8 {
    d.layer(layer).unwrap().pixel(Channel::Color, x, y).unwrap()
}
fn red() -> Rgba8 {
    Rgba8::new(255, 0, 0, 255)
}
/// 画素に何も描かれていない（透明の黒）か。
fn blank(d: &Document, layer: LayerId, x: u32, y: u32) -> bool {
    px(d, layer, x, y) == Rgba8::TRANSPARENT
}
/// 太い硬い筆（選択範囲の切り取りを見るため、押し込む量が 1 の丸）。
fn thick(radius: f64, erase: bool) -> BrushSettings {
    BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        color: red(),
        pressure_size: false,
        pressure_opacity: false,
        erase,
        ..BrushSettings::default()
    }
}
fn line(d: &mut Document, layer: LayerId, brush: &Brush, from: (f64, f64), to: (f64, f64)) {
    let mut s = d.begin_brush_stroke(layer, brush).unwrap();
    s.add_point(d, from.0, from.1, 1.0, DVec2::ZERO).unwrap();
    s.add_point(d, to.0, to.1, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
}

// ───────── 形 ─────────

#[test]
fn a_rectangle_selects_the_pixels_whose_centres_are_inside_and_clips_to_the_canvas() {
    let d = doc(40, 30, 16);
    let m = SelectionMask::rectangle(&d, 5, 4, 15, 9);
    assert_eq!(count(&m, |a| a == 255), 10 * 5);
    assert_eq!(count(&m, |a| a != 0 && a != 255), 0);
    assert_eq!(
        (
            m.amount(5, 4),
            m.amount(14, 8),
            m.amount(15, 8),
            m.amount(14, 9)
        ),
        (255, 255, 0, 0)
    );
    // 逆向きの角は入れ替える。画布の外へはみ出した分は無い
    assert_eq!(SelectionMask::rectangle(&d, 15, 9, 5, 4), m);
    let wide = SelectionMask::rectangle(&d, -10, -10, 100, 100);
    assert_eq!(count(&wide, |a| a == 255), 40 * 30);
    assert!(SelectionMask::rectangle(&d, 7, 7, 7, 20).is_empty(), "幅 0");
    assert!(
        SelectionMask::rectangle(&d, 50, 0, 60, 10).is_empty(),
        "画布の外"
    );
    assert_eq!(SelectionMask::all(&d), wide);
    assert!(SelectionMask::none(&d).is_empty());
}

#[test]
fn an_ellipse_is_smooth_symmetric_and_has_about_the_right_area() {
    let d = doc(120, 80, 32);
    let m = SelectionMask::ellipse(&d, 60.0, 40.0, 30.0, 20.0).unwrap();
    assert_eq!(m.amount(60, 40), 255);
    assert_eq!(m.amount(60, 5), 0);
    assert_eq!(m.amount(5, 40), 0);
    // 量の合計 / 255 が楕円の面積 πab に近い（4×4 の点で数えるので 1% 以内）
    let area: f64 = amounts(&m).iter().map(|&a| a as f64 / 255.0).sum();
    let exact = std::f64::consts::PI * 30.0 * 20.0;
    assert!((area - exact).abs() / exact < 0.01, "{area} / {exact}");
    // 中心 (60, 40) は画素の角なので、上下左右の鏡で量が同じ
    for (x, y) in [(31, 40), (45, 22), (58, 59)] {
        assert_eq!(m.amount(x, y), m.amount(119 - x, y), "左右 {x},{y}");
        assert_eq!(m.amount(x, y), m.amount(x, 79 - y), "上下 {x},{y}");
    }
    assert!(count(&m, |a| a != 0 && a != 255) > 40, "縁は滑らか");
    // 半径は絶対値、0 は何も選ばない
    assert_eq!(
        SelectionMask::ellipse(&d, 60.0, 40.0, -30.0, -20.0).unwrap(),
        m
    );
    assert!(SelectionMask::ellipse(&d, 60.0, 40.0, 0.0, 20.0)
        .unwrap()
        .is_empty());
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(SelectionMask::ellipse(&d, bad, 40.0, 3.0, 3.0).is_err());
        assert!(SelectionMask::ellipse(&d, 1.0, 40.0, bad, 3.0).is_err());
    }
    // 画布から遠く離れた巨大な楕円でも落ちない
    assert!(SelectionMask::ellipse(&d, 1e15, 1e15, 1e14, 1e14).is_ok());
}

#[test]
fn a_polygon_follows_the_even_odd_rule_and_refuses_bad_points() {
    let d = doc(64, 48, 16);
    let p = |x, y| DVec2::new(x, y);
    // 画素の角に合う正方形は矩形と同じ
    let square = SelectionMask::polygon(
        &d,
        &[p(4.0, 4.0), p(12.0, 4.0), p(12.0, 12.0), p(4.0, 12.0)],
    )
    .unwrap();
    assert_eq!(square, SelectionMask::rectangle(&d, 4, 4, 12, 12));
    // 入れ子の 2 つの正方形（同じ向き）は偶奇の規則で内側が抜ける
    let ring = SelectionMask::polygon(
        &d,
        &[
            p(2.0, 2.0),
            p(30.0, 2.0),
            p(30.0, 30.0),
            p(2.0, 30.0),
            p(2.0, 2.0),
            p(10.0, 10.0),
            p(22.0, 10.0),
            p(22.0, 22.0),
            p(10.0, 22.0),
            p(10.0, 10.0),
        ],
    )
    .unwrap();
    assert_eq!(ring.amount(5, 5), 255);
    assert_eq!(ring.amount(15, 15), 0, "穴");
    // 三角形: 斜めの辺の画素は部分的に選ばれる
    let tri = SelectionMask::polygon(&d, &[p(0.0, 0.0), p(40.0, 0.0), p(0.0, 40.0)]).unwrap();
    assert_eq!(tri.amount(2, 2), 255);
    assert_eq!(tri.amount(38, 38), 0);
    assert!(count(&tri, |a| a != 0 && a != 255) >= 30);
    // 3 点未満は何も選ばない。点が多すぎる・有限でないのは断る
    assert!(SelectionMask::polygon(&d, &[p(1.0, 1.0), p(9.0, 9.0)])
        .unwrap()
        .is_empty());
    assert!(SelectionMask::polygon(&d, &[]).unwrap().is_empty());
    let many = vec![p(1.0, 1.0); MAX_POLYGON_POINTS + 1];
    assert!(matches!(
        SelectionMask::polygon(&d, &many),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(SelectionMask::polygon(&d, &[p(0.0, 0.0), p(f64::NAN, 3.0), p(5.0, 5.0)]).is_err());
    assert!(
        SelectionMask::polygon(&d, &[p(0.0, 0.0), p(f64::INFINITY, 3.0), p(5.0, 5.0)]).is_err()
    );
    // 上限ちょうどの点の数は通る（同じ点の繰り返しなので何も選ばれない）
    let limit = vec![p(1.0, 1.0); MAX_POLYGON_POINTS];
    assert!(SelectionMask::polygon(&d, &limit).unwrap().is_empty());
}

// ───────── 組み合わせ・反転・鋭く ─────────

#[test]
fn combining_follows_the_formulas_on_soft_amounts() {
    let d = doc(32, 32, 16);
    let budget = BUDGET;
    // 画素ごとに量の違う選択範囲を、ぼかしで作る
    let a = SelectionMask::rectangle(&d, 4, 4, 20, 20)
        .feather(3.0, false, budget)
        .unwrap();
    let b = SelectionMask::rectangle(&d, 12, 12, 28, 28)
        .feather(3.0, false, budget)
        .unwrap();
    let (va, vb) = (amounts(&a), amounts(&b));
    let check = |mode, f: &dyn Fn(u8, u8) -> u8| {
        let got = amounts(&a.combine(&b, mode).unwrap());
        for i in 0..va.len() {
            assert_eq!(got[i], f(va[i], vb[i]), "{mode:?} {i}");
        }
    };
    check(SelectionCombine::Add, &|x, y| x.max(y));
    check(SelectionCombine::Intersect, &|x, y| x.min(y));
    check(SelectionCombine::Subtract, &|x, y| {
        ((x as u32 * (255 - y as u32) + 127) / 255) as u8
    });
    assert_eq!(a.combine(&b, SelectionCombine::Replace).unwrap(), b);
    assert!(count(&a, |v| v != 0 && v != 255) > 0);
    // 反転は 255 − 量（全タイルを作る）、2 回で戻る
    let inv = a.invert();
    for (i, v) in amounts(&inv).iter().enumerate() {
        assert_eq!(*v, 255 - va[i]);
    }
    assert_eq!(inv.invert(), a);
    assert_eq!(SelectionMask::none(&d).invert(), SelectionMask::all(&d));
    // 鋭く: 128 以上は 255、ほかは 0
    for (i, v) in amounts(&a.sharpen()).iter().enumerate() {
        assert_eq!(*v, if va[i] >= 128 { 255 } else { 0 });
    }
    // 大きさの違う選択範囲は組み合わせない
    let other = doc(33, 32, 16);
    assert!(a
        .combine(&SelectionMask::all(&other), SelectionCombine::Add)
        .is_err());
    let tile8 = doc(32, 32, 8);
    assert!(a
        .combine(&SelectionMask::all(&tile8), SelectionCombine::Add)
        .is_err());
}

#[test]
fn an_unchanged_result_is_the_same_selection_object() {
    let d = doc(32, 32, 16);
    let m = SelectionMask::rectangle(&d, 4, 4, 20, 20);
    assert!(m.grow(0, BUDGET).unwrap().same_as(&m));
    assert!(m.shrink(0, false, BUDGET).unwrap().same_as(&m));
    assert!(m.feather(0.0, false, BUDGET).unwrap().same_as(&m));
    assert!(
        m.feather(0.1, false, BUDGET).unwrap().same_as(&m),
        "標準偏差が 0.05 未満"
    );
    assert!(!m.same_as(&m.sharpen()), "鋭くは新しい札（同じ中身でも）");
    let empty = SelectionMask::none(&d);
    assert!(empty.grow(5, BUDGET).unwrap().same_as(&empty));
    assert!(empty.feather(9.0, false, BUDGET).unwrap().same_as(&empty));
}

// ───────── 拡張・縮小・境界・ぼかし ─────────

#[test]
fn grow_shrink_and_border_have_known_shapes() {
    let d = doc(64, 64, 16);
    let one = SelectionMask::rectangle(&d, 30, 30, 31, 31);
    // GIMP の円: 半径 1 は 3×3 の四角、半径 3 は角が 2 の丸
    let g1 = one.grow(1, BUDGET).unwrap();
    assert_eq!(count(&g1, |a| a == 255), 9);
    assert_eq!(
        (g1.amount(29, 29), g1.amount(31, 31), g1.amount(28, 30)),
        (255, 255, 0)
    );
    let g3 = one.grow(3, BUDGET).unwrap();
    assert_eq!(
        count(&g3, |a| a == 255),
        2 * 5 + 5 * 7,
        "半径 3 の円（横のずれごとの高さ 5・7・7・7・7・7・5）の画素の数"
    );
    assert_eq!(g3.amount(33, 30), 255);
    assert_eq!(g3.amount(33, 33), 0, "角");
    // 縮小は拡張の逆: 3×3 は 1 画素へ、1 画素は消える
    assert_eq!(g1.shrink(1, false, BUDGET).unwrap(), one);
    assert!(one.shrink(1, false, BUDGET).unwrap().is_empty());
    // 柔らかい縁は柔らかいまま（濃淡のモルフォロジー）: 拡張は元の量以上、縮小は元の量以下
    let soft = SelectionMask::ellipse(&d, 32.0, 32.0, 12.3, 9.7).unwrap();
    let (s, up, down) = (
        amounts(&soft),
        amounts(&soft.grow(3, BUDGET).unwrap()),
        amounts(&soft.shrink(3, false, BUDGET).unwrap()),
    );
    for i in 0..s.len() {
        assert!(up[i] >= s[i] && down[i] <= s[i], "{i}");
    }
    // 境界の帯: 拡張(r) − 縮小(r+1) で、中は空・外も空
    let sq = SelectionMask::rectangle(&d, 20, 20, 44, 44);
    let band = sq.border(4, false, BUDGET).unwrap();
    assert_eq!(band.amount(32, 32), 0, "中");
    assert_eq!(band.amount(20, 32), 255, "縁");
    assert_eq!(band.amount(10, 10), 0, "外");
    assert!(sq.border(0, false, BUDGET).unwrap().is_empty());
    // 半径 1 の境界は、元から縮小(1) を引いた縁の 1 周
    let ring = sq.border(1, false, BUDGET).unwrap();
    assert_eq!(count(&ring, |a| a == 255), 24 * 24 - 22 * 22);
}

#[test]
fn edge_lock_keeps_the_selection_going_past_the_canvas_edge() {
    let d = doc(32, 32, 16);
    let all = SelectionMask::all(&d);
    assert!(!all.shrink(4, false, BUDGET).unwrap().is_empty());
    assert_eq!(
        all.shrink(4, false, BUDGET).unwrap().amount(0, 0),
        0,
        "画布の縁から縮む"
    );
    assert_eq!(
        all.shrink(4, true, BUDGET).unwrap(),
        all,
        "縁を固定すれば縮まない"
    );
    let left = SelectionMask::rectangle(&d, 0, 0, 16, 32);
    let locked = left.shrink(3, true, BUDGET).unwrap();
    assert_eq!(locked.amount(0, 5), 255, "画布の縁に接する辺は縮まない");
    assert_eq!(locked.amount(14, 5), 0, "内側の辺は縮む");
    // ぼかしも、縁を固定すれば画布の縁の画素が外へ続くと数える
    let f = left.feather(6.0, false, BUDGET).unwrap();
    let g = left.feather(6.0, true, BUDGET).unwrap();
    assert!(f.amount(0, 16) < 255);
    assert_eq!(g.amount(0, 16), 255);
}

#[test]
fn feather_blurs_the_amounts_and_keeps_their_total() {
    let d = doc(80, 80, 16);
    let sq = SelectionMask::rectangle(&d, 30, 30, 50, 50);
    for r in [3.0, 10.0, 40.0] {
        let f = sq.feather(r, false, BUDGET).unwrap();
        assert!(f.amount(40, 40) > 0);
        assert!(f.amount(29, 40) > 0, "外へ広がる r={r}");
        assert!(f.amount(31, 40) < 255 || r < 3.5, "縁が薄まる r={r}");
        let total = |m: &SelectionMask| amounts(m).iter().map(|&a| a as f64).sum::<f64>();
        // 画布の中に収まるなら量の合計はほぼ保たれる（丸めの誤差だけ）
        if r <= 10.0 {
            assert!((total(&f) - total(&sq)).abs() / total(&sq) < 0.01, "r={r}");
        }
        // 左右・上下対称（正方形は画布の真ん中）
        for (x, y) in [(25, 40), (30, 30), (28, 45)] {
            assert_eq!(f.amount(x, y), f.amount(79 - x, y), "r={r} {x},{y}");
            assert_eq!(f.amount(x, y), f.amount(x, 79 - y), "r={r} {x},{y}");
        }
    }
    // 半径は 0〜200（有限）
    assert!(sq
        .feather(MAX_MODIFY_RADIUS as f64 + 0.5, false, BUDGET)
        .is_err());
    assert!(sq.feather(-1.0, false, BUDGET).is_err());
    assert!(sq.feather(f64::NAN, false, BUDGET).is_err());
    assert!(sq.grow(MAX_MODIFY_RADIUS + 1, BUDGET).is_err());
    assert!(sq.shrink(MAX_MODIFY_RADIUS + 1, false, BUDGET).is_err());
    assert!(sq.border(MAX_MODIFY_RADIUS + 1, false, BUDGET).is_err());
    assert!(sq.feather(MAX_MODIFY_RADIUS as f64, false, BUDGET).is_ok());
}

#[test]
fn the_result_does_not_depend_on_the_thread_count() {
    let d = doc(200, 160, 32);
    let m = SelectionMask::ellipse(&d, 100.0, 80.0, 70.0, 50.0).unwrap();
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            [
                m.grow(9, BUDGET).unwrap(),
                m.shrink(9, false, BUDGET).unwrap(),
                m.border(7, false, BUDGET).unwrap(),
                m.feather(5.0, false, BUDGET).unwrap(),
                m.feather(40.0, true, BUDGET).unwrap(),
            ]
            .map(|s| amounts(&s))
        })
    };
    assert_eq!(run(1), run(4));
}

// ───────── 自動選択 ─────────

fn paint(d: &mut Document, id: LayerId, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgba8) {
    for y in y0..y1 {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}
fn wand(
    d: &Document,
    layer: Option<LayerId>,
    seed: (u32, u32),
    tol: u8,
    contiguous: bool,
) -> SelectionMask {
    SelectionMask::magic_wand(
        d,
        layer,
        Channel::Color,
        seed.0,
        seed.1,
        tol,
        contiguous,
        BUDGET,
    )
    .unwrap()
}

#[test]
fn the_wand_follows_four_neighbours_or_takes_every_match() {
    let mut d = doc(64, 48, 16);
    let a = d.add_layer("a").unwrap();
    let (r, g, b) = (
        red(),
        Rgba8::new(0, 255, 0, 255),
        Rgba8::new(0, 0, 255, 255),
    );
    paint(&mut d, a, 0, 0, 32, 48, r);
    paint(&mut d, a, 32, 0, 64, 48, b);
    paint(&mut d, a, 10, 10, 14, 14, g); // 赤の中の緑
    paint(&mut d, a, 40, 20, 44, 24, g); // 青の中の緑
                                         // 赤の領域から、緑の穴を除く
    let red_area = wand(&d, Some(a), (5, 5), 0, true);
    assert_eq!(count(&red_area, |v| v == 255), 32 * 48 - 16);
    assert_eq!(
        count(&red_area, |v| v != 0 && v != 255),
        0,
        "縁は硬い（0 か 255）"
    );
    assert_eq!(red_area.amount(11, 11), 0);
    // 緑は、つながる範囲なら 1 つ・全体なら 2 つ
    assert_eq!(
        count(&wand(&d, Some(a), (11, 11), 0, true), |v| v == 255),
        16
    );
    assert_eq!(
        count(&wand(&d, Some(a), (11, 11), 0, false), |v| v == 255),
        32
    );
    // 全体は、つながらない場所も拾う
    let blues = wand(&d, Some(a), (60, 2), 0, false);
    assert_eq!(count(&blues, |v| v == 255), 32 * 48 - 16);
}

#[test]
fn the_wand_tolerance_is_the_largest_component_difference_and_includes_alpha() {
    let mut d = doc(32, 8, 8);
    let a = d.add_layer("a").unwrap();
    for x in 0..32u32 {
        // 赤の成分だけが x に比例（2 ずつ）、不透明
        for y in 0..8 {
            d.set_pixel(a, x, y, Rgba8::new((x * 2) as u8, 50, 50, 255))
                .unwrap();
        }
    }
    // 許し幅 5 は、赤の差が 5 以下 = 左右に 2 列ずつ
    let m = wand(&d, Some(a), (10, 3), 5, true);
    assert_eq!(count(&m, |v| v == 255), 5 * 8);
    assert_eq!(
        (
            m.amount(8, 0),
            m.amount(12, 7),
            m.amount(7, 0),
            m.amount(13, 0)
        ),
        (255, 255, 0, 0)
    );
    assert_eq!(count(&wand(&d, Some(a), (10, 3), 0, true), |v| v == 255), 8);
    assert_eq!(
        count(&wand(&d, Some(a), (10, 3), 255, true), |v| v == 255),
        32 * 8
    );
    // アルファも成分: 透明と不透明は差が 255 なので、254 までは別
    d.set_pixel(a, 20, 3, Rgba8::new(20, 50, 50, 0)).unwrap();
    assert!(wand(&d, Some(a), (10, 3), 254, false).amount(20, 3) == 0);
    assert_eq!(wand(&d, Some(a), (10, 3), 255, false).amount(20, 3), 255);
    // 斜めにしか触れない画素はつながらない（4 近傍）
    let c = d.add_layer("c").unwrap();
    d.set_pixel(c, 3, 3, red()).unwrap();
    d.set_pixel(c, 4, 4, red()).unwrap();
    assert_eq!(count(&wand(&d, Some(c), (3, 3), 0, true), |v| v == 255), 1);
    assert_eq!(count(&wand(&d, Some(c), (3, 3), 0, false), |v| v == 255), 2);
}

#[test]
fn the_wand_can_use_the_composite_or_other_layer_kinds_as_the_reference() {
    let mut d = doc(32, 32, 16);
    let bottom = d.add_layer("bottom").unwrap();
    paint(&mut d, bottom, 0, 0, 32, 32, Rgba8::new(10, 10, 10, 255));
    let top = d.add_layer("top").unwrap();
    paint(&mut d, top, 8, 8, 24, 24, red());
    // 合成が基準: 上の層の赤い四角が 1 つの領域
    assert_eq!(
        count(&wand(&d, None, (12, 12), 0, true), |v| v == 255),
        16 * 16
    );
    // 層そのものが基準: 上の層の透明な所（赤の外）
    assert_eq!(
        count(&wand(&d, Some(top), (2, 2), 0, true), |v| v == 255),
        32 * 32 - 16 * 16
    );
    // 下の層は一面同じ色
    assert_eq!(
        count(&wand(&d, Some(bottom), (2, 2), 0, true), |v| v == 255),
        32 * 32
    );
    // 合成が基準でつながりを見ない（帯ごとの経路）も同じ集まり
    assert_eq!(
        wand(&d, None, (12, 12), 0, false),
        wand(&d, None, (12, 12), 0, true)
    );
    // 塗りつぶしの層は全面が同じ値
    let fill = d
        .add_fill_layer("fill", &[(Channel::Color, Rgba8::new(0, 90, 0, 255))], None)
        .unwrap();
    assert_eq!(
        count(&wand(&d, Some(fill), (0, 0), 0, true), |v| v == 255),
        32 * 32
    );
}

#[test]
fn the_wand_refuses_bad_seeds_unknown_layers_and_small_budgets() {
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 64, 64, red());
    let magic = |layer, x, y, contiguous, budget| {
        SelectionMask::magic_wand(&d, layer, Channel::Color, x, y, 0, contiguous, budget)
    };
    assert!(matches!(
        magic(Some(a), 64, 0, true, BUDGET),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        magic(Some(a), 0, 64, false, BUDGET),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        magic(Some(LayerId(7)), 0, 0, true, BUDGET),
        Err(CoreError::LayerNotFound)
    ));
    assert!(magic(Some(a), 0, 0, true, BUDGET).is_ok());
    // 作業のメモリの見積もりが予算を超えるなら、確保の前に断る
    for contiguous in [true, false] {
        for layer in [Some(a), None] {
            assert!(
                matches!(
                    magic(layer, 0, 0, contiguous, 100),
                    Err(CoreError::WorkingBudgetExceeded)
                ),
                "{contiguous} {layer:?}"
            );
        }
    }
    // 予算の拒否は何も変えない
    assert!(d.selection().is_none());
    assert_eq!(px(&d, a, 5, 5), red());
}

// ───────── 文書の選択範囲と Undo ─────────

#[test]
fn setting_the_selection_is_one_undo_step_and_does_not_touch_pixels() {
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 64, 64, red());
    d.clear_history().unwrap();
    let serial = d.change_serial();
    let m1 = SelectionMask::rectangle(&d, 5, 5, 30, 30);
    let m2 = SelectionMask::ellipse(&d, 32.0, 32.0, 20.0, 10.0).unwrap();

    d.set_selection(Some(m1.clone())).unwrap();
    assert_eq!((d.undo_count(), d.redo_count()), (1, 0));
    assert!(d.history_bytes() >= 64 + m1.history_bytes());
    assert!(d.selection().unwrap().same_as(&m1));
    assert!(d.revision() > 0);
    assert_eq!(
        d.change_serial(),
        serial,
        "画素は変えない（表示の再合成が要らない）"
    );
    // 同じ札を置いても段は積まない
    d.set_selection(Some(m1.clone())).unwrap();
    assert_eq!(d.undo_count(), 1);
    d.set_selection(Some(m2.clone())).unwrap();
    assert_eq!(d.undo_count(), 2);
    // Undo は 1 つ前の札に戻る（同じ札）。Redo で進む
    assert!(d.undo().unwrap());
    assert!(d.selection().unwrap().same_as(&m1));
    assert!(d.undo().unwrap());
    assert!(d.selection().is_none());
    assert!(d.redo().unwrap());
    assert!(d.redo().unwrap());
    assert!(d.selection().unwrap().same_as(&m2));
    // 選択を外すのも 1 段。選択が無いときは何もしない
    d.clear_selection().unwrap();
    assert_eq!(d.undo_count(), 3);
    d.clear_selection().unwrap();
    assert_eq!(d.undo_count(), 3);
    assert!(d.undo().unwrap());
    assert!(d.selection().unwrap().same_as(&m2));
    // 新しい編集で Redo は消える
    d.set_selection(None).unwrap();
    assert!(!d.can_redo());
    assert_eq!(px(&d, a, 0, 0), red());
}

#[test]
fn an_empty_selection_means_no_selection_and_other_edits_interleave_with_undo() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    d.set_selection(Some(SelectionMask::none(&d))).unwrap();
    assert!(d.selection().is_none());
    assert_eq!(d.undo_count(), 0, "選択なしに空を置いても段は積まない");
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 8, 8)))
        .unwrap();
    d.set_selection(Some(SelectionMask::none(&d))).unwrap();
    assert!(d.selection().is_none(), "空は選択を外す");
    assert_eq!(d.undo_count(), 2);
    // 選択 → 画素 → 取り消しの順に戻る
    d.undo().unwrap();
    d.undo().unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 16, 16)))
        .unwrap();
    let fill_changed = d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap();
    assert!(fill_changed);
    assert_eq!(
        (px(&d, a, 5, 5), px(&d, a, 20, 20)),
        (red(), Rgba8::TRANSPARENT)
    );
    d.undo().unwrap();
    assert_eq!(px(&d, a, 5, 5), Rgba8::TRANSPARENT);
    assert!(d.selection().is_some(), "塗りの Undo は選択範囲を外さない");
    d.undo().unwrap();
    assert!(d.selection().is_none());
}

#[test]
fn the_selection_is_refused_while_a_stroke_runs_or_for_another_canvas() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    let m = SelectionMask::rectangle(&d, 0, 0, 8, 8);
    let s = d.begin_stroke(a, &thick(2.0, false)).unwrap();
    assert!(matches!(
        d.set_selection(Some(m.clone())),
        Err(CoreError::StrokeActive)
    ));
    assert!(matches!(d.clear_selection(), Err(CoreError::StrokeActive)));
    assert!(matches!(
        d.combine_selection(&m, SelectionCombine::Replace),
        Err(CoreError::StrokeActive)
    ));
    d.cancel_stroke(s);
    assert!(d.selection().is_none());
    // 大きさ・タイルの大きさが違う選択範囲は置けない
    for other in [doc(33, 32, 16), doc(32, 33, 16), doc(32, 32, 8)] {
        let wrong = SelectionMask::all(&other);
        assert!(matches!(
            d.set_selection(Some(wrong.clone())),
            Err(CoreError::InvalidArgument(_))
        ));
        assert!(matches!(
            d.restore_selection(Some(wrong)),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    assert!(d.selection().is_none());
}

#[test]
fn combining_into_the_document_follows_the_tool_modifiers() {
    let mut d = doc(64, 64, 16);
    d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    let left = SelectionMask::rectangle(&d, 0, 0, 40, 64);
    let right = SelectionMask::rectangle(&d, 24, 0, 64, 64);
    // 選択が無いとき: 置き換え・足す・重ねるは形そのもの、引くのは何も選ばない
    d.combine_selection(&left, SelectionCombine::Subtract)
        .unwrap();
    assert!(d.selection().is_none());
    d.combine_selection(&left, SelectionCombine::Intersect)
        .unwrap();
    assert_eq!(d.selection().unwrap(), &left);
    d.combine_selection(&right, SelectionCombine::Add).unwrap();
    assert_eq!(count(d.selection().unwrap(), |v| v == 255), 64 * 64);
    d.combine_selection(&left, SelectionCombine::Subtract)
        .unwrap();
    assert_eq!(count(d.selection().unwrap(), |v| v == 255), 24 * 64);
    d.combine_selection(&right, SelectionCombine::Replace)
        .unwrap();
    assert_eq!(d.selection().unwrap(), &right);
    // 全部を引くと選択は外れる
    d.combine_selection(&SelectionMask::all(&d), SelectionCombine::Subtract)
        .unwrap();
    assert!(d.selection().is_none());
    assert!(d.can_undo());
}

#[test]
fn a_loaded_selection_is_restored_only_before_any_history() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    let m = SelectionMask::rectangle(&d, 4, 4, 20, 20);
    d.restore_selection(Some(m.clone())).unwrap();
    assert!(d.selection().unwrap().same_as(&m));
    assert_eq!((d.undo_count(), d.redo_count()), (0, 0), "履歴は増えない");
    d.restore_selection(Some(SelectionMask::none(&d))).unwrap();
    assert!(d.selection().is_none(), "空は選択なし");
    d.restore_selection(Some(m)).unwrap();
    // 履歴ができたら断る
    d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap();
    assert!(matches!(
        d.restore_selection(None),
        Err(CoreError::Unsupported(_))
    ));
    assert!(d.selection().is_some());
}

// ───────── ブラシ・消しゴムを選択範囲で切る ─────────

#[test]
fn a_stroke_only_changes_the_selected_pixels_in_proportion_to_their_amount() {
    let mut d = doc(64, 32, 16);
    let a = d.add_layer("a").unwrap();
    let brush = Brush::from(thick(6.0, false));
    // 幅 20〜40 だけを選ぶ。縁の列 20 は半分の量（128）
    let hard = SelectionMask::rectangle(&d, 21, 0, 40, 32);
    let half_column = SelectionMask::from_amount_tiles(
        64,
        32,
        16,
        (0..2u32).flat_map(|ty| {
            [TileCoord::new(1, ty)].into_iter().map(move |c| {
                let mut t = vec![0u8; 16 * 16];
                for y in 0..16 {
                    t[y * 16 + 4] = 128; // x = 20
                }
                (c, t)
            })
        }),
    )
    .unwrap();
    let selection = hard.combine(&half_column, SelectionCombine::Add).unwrap();
    d.set_selection(Some(selection)).unwrap();
    line(&mut d, a, &brush, (2.0, 16.0), (62.0, 16.0));
    for x in 0..64u32 {
        let p = px(&d, a, x, 16);
        match x {
            21..=39 => assert_eq!(p, red(), "{x}"),
            20 => assert!((p.a as i32 - 128).abs() <= 1 && p.r == 255, "{x}: {p:?}"),
            _ => assert_eq!(p, Rgba8::TRANSPARENT, "{x}"),
        }
    }
    // 選択範囲の外の画素は RGB も含めて何も書かれない
    assert!(blank(&d, a, 5, 16) && blank(&d, a, 50, 16));
    // 1 回の Undo で戻る
    d.undo().unwrap();
    assert!(blank(&d, a, 25, 16) && blank(&d, a, 20, 16));
    assert!(d.selection().is_some());
}

#[test]
fn a_half_selected_pixel_moves_halfway_from_what_was_there() {
    let mut d = doc(16, 16, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 16, 16, Rgba8::new(0, 0, 255, 255));
    let mut half = vec![0u8; 256];
    half.fill(128);
    let selection =
        SelectionMask::from_amount_tiles(16, 16, 16, [(TileCoord::new(0, 0), half)]).unwrap();
    d.set_selection(Some(selection)).unwrap();
    d.clear_history().unwrap();
    line(
        &mut d,
        a,
        &Brush::from(thick(20.0, false)),
        (8.0, 8.0),
        (8.5, 8.0),
    );
    let expected = yolu_core::blend::fade(Rgba8::new(0, 0, 255, 255), red(), 128.0 / 255.0);
    assert_eq!(px(&d, a, 8, 8), expected);
    assert!(expected.r > 100 && expected.b > 100, "赤と青の中間");
}

#[test]
fn the_eraser_only_erases_inside_the_selection() {
    let mut d = doc(64, 32, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 64, 32, Rgba8::new(10, 200, 30, 255));
    d.set_selection(Some(SelectionMask::rectangle(&d, 20, 0, 44, 32)))
        .unwrap();
    line(
        &mut d,
        a,
        &Brush::from(thick(8.0, true)),
        (2.0, 16.0),
        (62.0, 16.0),
    );
    assert_eq!(px(&d, a, 5, 16).a, 255);
    assert_eq!(px(&d, a, 60, 16).a, 255);
    assert_eq!(px(&d, a, 30, 16).a, 0);
    assert_eq!(
        px(&d, a, 19, 16),
        Rgba8::new(10, 200, 30, 255),
        "外は元のまま"
    );
    assert_eq!(px(&d, a, 44, 16), Rgba8::new(10, 200, 30, 255));
}

#[test]
fn a_stroke_outside_the_selection_changes_nothing_and_leaves_no_history() {
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 40, 40, 60, 60)))
        .unwrap();
    d.clear_history().unwrap();
    let mut s = d
        .begin_brush_stroke(a, &Brush::from(thick(5.0, false)))
        .unwrap();
    s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut d, 20.0, 12.0, 1.0, DVec2::ZERO).unwrap();
    let r = d.end_stroke(s).unwrap();
    assert!(!r.changed);
    assert_eq!(d.undo_count(), 0);
    assert_eq!(
        d.layer(a)
            .unwrap()
            .surface(Channel::Color)
            .map_or(0, |s| s.tile_count()),
        0,
        "タイルも作らない"
    );
}

#[test]
fn a_mask_stroke_is_cut_by_the_selection_too() {
    let mut d = doc(64, 32, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 64, 32, red());
    d.add_layer_mask(a).unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 24, 0, 40, 32)))
        .unwrap();
    let mut s = d.begin_mask_stroke(a, &thick(5.0, false)).unwrap();
    s.add_point(&mut d, 2.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut d, 62.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
    let hide = |x| {
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(x, 16)
            .unwrap()
            .a
    };
    assert_eq!((hide(10), hide(50)), (0, 0));
    assert_eq!((hide(30), hide(24), hide(39)), (255, 255, 255));
}

#[test]
fn a_stroke_keeps_the_selection_it_started_with() {
    let mut d = doc(64, 32, 16);
    let a = d.add_layer("a").unwrap();
    let left = SelectionMask::rectangle(&d, 0, 0, 32, 32);
    d.set_selection(Some(left)).unwrap();
    let mut s = d
        .begin_brush_stroke(a, &Brush::from(thick(4.0, false)))
        .unwrap();
    s.add_point(&mut d, 2.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    // ストロークの途中で選択範囲は変えられない（変わらないので、右へ描いても切られたまま）
    assert!(d.set_selection(None).is_err());
    s.add_point(&mut d, 62.0, 16.0, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(px(&d, a, 10, 16), red());
    assert!(blank(&d, a, 50, 16));
    // 取り消したストロークの後も選択範囲はそのまま
    let s = d
        .begin_brush_stroke(a, &Brush::from(thick(4.0, false)))
        .unwrap();
    d.cancel_stroke(s);
    assert!(d.selection().is_some());
}

// ───────── 塗りつぶし ─────────

#[test]
fn fill_uses_the_selection_the_region_or_both() {
    let mut d = doc(64, 32, 16);
    let a = d.add_layer("a").unwrap();
    let covered = |d: &Document| (0..64).filter(|&x| px(d, a, x, 8) == red()).count();
    // 選択も範囲も無ければ全体
    assert!(d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap());
    assert_eq!(covered(&d), 64);
    d.undo().unwrap();
    // 選択範囲だけ
    d.set_selection(Some(SelectionMask::rectangle(&d, 10, 0, 30, 32)))
        .unwrap();
    assert!(d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap());
    assert_eq!(covered(&d), 20);
    d.undo().unwrap();
    // 範囲と選択範囲の重なり
    let region = SelectionMask::rectangle(&d, 20, 0, 50, 32);
    assert!(d
        .fill(a, Channel::Color, red(), 1.0, Some(&region), false)
        .unwrap());
    assert_eq!(covered(&d), 10);
    d.undo().unwrap();
    // 選択が無いときは範囲だけ
    d.clear_selection().unwrap();
    assert!(d
        .fill(a, Channel::Color, red(), 1.0, Some(&region), false)
        .unwrap());
    assert_eq!(covered(&d), 30);
}

#[test]
fn fill_blends_by_opacity_and_amount_and_erases() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 32, 32, Rgba8::new(0, 0, 255, 255));
    d.clear_history().unwrap();
    d.fill(a, Channel::Color, red(), 0.5, None, false).unwrap();
    let half = px(&d, a, 3, 3);
    assert_eq!(
        half,
        yolu_core::blend::blend(
            Rgba8::new(0, 0, 255, 255),
            red(),
            0.5,
            yolu_core::BlendMode::Normal
        )
    );
    d.undo().unwrap();
    // 消去: 色のアルファ × 不透明度 × 量だけアルファを減らす
    d.fill(
        a,
        Channel::Color,
        Rgba8::new(0, 0, 0, 255),
        0.25,
        None,
        true,
    )
    .unwrap();
    let eroded = px(&d, a, 3, 3);
    assert!((eroded.a as i32 - 191).abs() <= 1, "{eroded:?}");
    assert_eq!((eroded.r, eroded.g, eroded.b), (0, 0, 255), "RGB は残す");
    d.undo().unwrap();
    // すでに同じ色なら変わらない: false で、履歴に積まない
    d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap();
    let before = d.undo_count();
    assert!(!d.fill(a, Channel::Color, red(), 1.0, None, false).unwrap());
    assert_eq!(d.undo_count(), before);
    // 選択範囲が全部 0 の所だけを指す範囲は何もしない
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 4, 4)))
        .unwrap();
    let region = SelectionMask::rectangle(&d, 20, 20, 30, 30);
    let before = d.undo_count();
    assert!(!d
        .fill(
            a,
            Channel::Color,
            Rgba8::new(1, 2, 3, 255),
            1.0,
            Some(&region),
            false
        )
        .unwrap());
    assert_eq!(d.undo_count(), before);
}

#[test]
fn fill_refuses_wrong_arguments_without_changing_anything() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    let group = d.group_layers(&[a], "g").unwrap();
    let fill_layer = d
        .add_fill_layer("f", &[(Channel::Color, red())], None)
        .unwrap();
    d.clear_history().unwrap();
    let all = SelectionMask::all(&d);
    assert!(d.fill(a, Channel::Color, red(), 1.5, None, false).is_err());
    assert!(d
        .fill(a, Channel::Color, red(), f64::NAN, None, false)
        .is_err());
    assert!(matches!(
        d.fill(LayerId(99), Channel::Color, red(), 1.0, None, false),
        Err(CoreError::LayerNotFound)
    ));
    assert!(matches!(
        d.fill(group, Channel::Color, red(), 1.0, None, false),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        d.fill(fill_layer, Channel::Color, red(), 1.0, None, false),
        Err(CoreError::Unsupported(_))
    ));
    let other = doc(33, 32, 16);
    let wrong = SelectionMask::all(&other);
    assert!(matches!(
        d.fill(a, Channel::Color, red(), 1.0, Some(&wrong), false),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(
        d.fill_mask(a, 1.0, Some(&all), false).is_err(),
        "マスクが無い"
    );
    assert_eq!(d.undo_count(), 0);
    assert_eq!(
        d.layer(a)
            .unwrap()
            .surface(Channel::Color)
            .map_or(0, |s| s.tile_count()),
        0,
        "面も作らない"
    );
}

#[test]
fn fill_over_the_budget_is_refused_and_undone() {
    let mut d = doc(128, 128, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 128, 128, Rgba8::new(0, 0, 255, 255));
    d.clear_history().unwrap();
    let before = d.composite(d.bounds()).unwrap();
    // 巻き戻しの写しの予算が足りない
    d.set_stroke_budget_bytes(4096).unwrap();
    assert_eq!(
        d.fill(a, Channel::Color, red(), 1.0, None, false),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert_eq!(
        d.composite(d.bounds()).unwrap(),
        before,
        "書いたタイルは全部戻す"
    );
    assert_eq!(d.undo_count(), 0);
    // 画素の予算が足りない（空の層への塗りは新しいタイルを作る。縁が柔らかい選択範囲の縁のタイルは一様でなく、1 KiB ずつ要る）
    d.set_stroke_budget_bytes(1 << 30).unwrap();
    let b = d.add_layer("b").unwrap();
    d.set_selection(Some(
        SelectionMask::ellipse(&d, 64.0, 64.0, 60.0, 60.0).unwrap(),
    ))
    .unwrap();
    d.clear_history().unwrap();
    let used = d.allocated_bytes();
    d.set_source_budget_bytes(used + 2048).unwrap();
    assert_eq!(
        d.fill(b, Channel::Color, red(), 1.0, None, false),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(d.allocated_bytes(), used);
    assert_eq!(
        d.layer(b)
            .unwrap()
            .surface(Channel::Color)
            .map_or(0, |s| s.tile_count()),
        0
    );
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn fill_mask_hides_or_reveals_inside_the_range() {
    let mut d = doc(32, 32, 16);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, 0, 0, 32, 32, red());
    d.add_layer_mask(a).unwrap();
    d.clear_history().unwrap();
    let hide = |d: &Document, x, y| {
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(x, y)
            .unwrap()
            .a
    };
    let region = SelectionMask::rectangle(&d, 8, 8, 16, 16);
    assert!(d.fill_mask(a, 1.0, Some(&region), false).unwrap());
    assert_eq!((hide(&d, 10, 10), hide(&d, 20, 20)), (255, 0));
    // 半分の量で見せる: 隠す量が半分へ寄る（四捨五入）
    assert!(d.fill_mask(a, 0.5, Some(&region), true).unwrap());
    assert_eq!(hide(&d, 10, 10), 128);
    assert!(d.fill_mask(a, 1.0, None, true).unwrap());
    assert_eq!(hide(&d, 10, 10), 0);
    assert!(
        !d.fill_mask(a, 1.0, None, true).unwrap(),
        "もう何も隠していない"
    );
    assert!(d.fill_mask(a, 2.0, None, true).is_err());
    d.undo().unwrap();
    assert_eq!(hide(&d, 10, 10), 128);
}

// ───────── 2D の対称 ─────────

fn symmetric(mode: SymmetryMode, center: (f64, f64), count: u32) -> Brush {
    let mut b = Brush::from(thick(5.0, false));
    b.symmetry = CanvasSymmetry::new(mode, DVec2::new(center.0, center.1), count).unwrap();
    b
}
fn dot(d: &mut Document, layer: LayerId, brush: &Brush, at: (f64, f64)) {
    let mut s = d.begin_brush_stroke(layer, brush).unwrap();
    s.add_point(d, at.0, at.1, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
}
fn alpha_map(d: &Document, layer: LayerId) -> Vec<Vec<u8>> {
    (0..d.height())
        .map(|y| (0..d.width()).map(|x| px(d, layer, x, y).a).collect())
        .collect()
}

#[test]
fn mirror_and_rotation_symmetry_put_the_dab_in_every_copy() {
    let mut d = doc(64, 64, 16);
    let v = d.add_layer("vertical").unwrap();
    dot(
        &mut d,
        v,
        &symmetric(SymmetryMode::Vertical, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    let m = alpha_map(&d, v);
    assert_eq!(m[20][10], 255);
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(m[y][x], m[y][63 - x], "縦の軸 {x},{y}");
        }
    }
    let h = d.add_layer("horizontal").unwrap();
    dot(
        &mut d,
        h,
        &symmetric(SymmetryMode::Horizontal, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    let m = alpha_map(&d, h);
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(m[y][x], m[63 - y][x], "横の軸 {x},{y}");
        }
    }
    let both = d.add_layer("both").unwrap();
    dot(
        &mut d,
        both,
        &symmetric(SymmetryMode::Both, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    let m = alpha_map(&d, both);
    assert_eq!(m.iter().flatten().filter(|&&a| a == 255).count() % 4, 0);
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(m[y][x], m[63 - y][63 - x]);
            assert_eq!(m[y][x], m[y][63 - x]);
        }
    }
    // 放射状 4: 90° の回転で同じ（画素 (x, y) は (63 − y, x) へ）
    let r = d.add_layer("radial").unwrap();
    dot(
        &mut d,
        r,
        &symmetric(SymmetryMode::Radial, (32.0, 32.0), 4),
        (10.5, 20.5),
    );
    let m = alpha_map(&d, r);
    assert_eq!(m[20][10], 255);
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(m[y][x], m[x][63 - y], "回転 {x},{y}");
        }
    }
    // 対称なしは 1 つだけ
    let n = d.add_layer("none").unwrap();
    dot(
        &mut d,
        n,
        &symmetric(SymmetryMode::None, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    let m = alpha_map(&d, n);
    assert_eq!(m[20][53], 0);
}

#[test]
fn symmetry_composes_with_the_selection_and_undo() {
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 32, 64)))
        .unwrap();
    d.clear_history().unwrap();
    dot(
        &mut d,
        a,
        &symmetric(SymmetryMode::Vertical, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    assert_eq!(px(&d, a, 10, 20), red());
    assert!(blank(&d, a, 53, 20), "写しは選択範囲の外なので描かない");
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(blank(&d, a, 10, 20));
    d.clear_selection().unwrap();
    dot(
        &mut d,
        a,
        &symmetric(SymmetryMode::Vertical, (32.0, 32.0), 2),
        (10.5, 20.5),
    );
    assert_eq!(px(&d, a, 53, 20), red());
    d.undo().unwrap();
    assert!(
        blank(&d, a, 53, 20) && blank(&d, a, 10, 20),
        "写しも 1 回の Undo で戻る"
    );
}

#[test]
fn a_mirror_that_lands_off_canvas_still_paints_the_part_that_is_on_canvas() {
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    // 元のダブは画布の外（x = -3.5）、鏡の中心 x = 10 の写しは x = 23.5 で画布の中
    dot(
        &mut d,
        a,
        &symmetric(SymmetryMode::Vertical, (10.0, 32.0), 2),
        (-3.5, 20.5),
    );
    assert_eq!(px(&d, a, 23, 20), red());
    assert!(blank(&d, a, 2, 20));
}

#[test]
fn symmetry_refuses_bad_settings_smudge_and_too_much_work() {
    let c = DVec2::new(32.0, 32.0);
    assert!(CanvasSymmetry::new(SymmetryMode::Radial, c, 1).is_err());
    assert!(CanvasSymmetry::new(SymmetryMode::Radial, c, 17).is_err());
    assert!(CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(f64::NAN, 0.0), 2).is_err());
    // 指先・クローンは対称と組めない
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    let mut smudge = symmetric(SymmetryMode::Both, (32.0, 32.0), 2);
    smudge.effect = yolu_core::BrushEffect::Smudge { strength: 0.5 };
    assert!(matches!(
        d.begin_brush_stroke(a, &smudge),
        Err(CoreError::Unsupported(_))
    ));
    // 範囲外の中心の設定は、ブラシの検証で断る（struct を直接組んだ場合）
    let mut bad = symmetric(SymmetryMode::Vertical, (32.0, 32.0), 2);
    bad.symmetry.center = DVec2::new(2e7, 0.0);
    assert!(d.begin_brush_stroke(a, &bad).is_err());
    // 1 ダブで調べる画素の数が上限（4 Mi 画素）を超えたら、ストロークごと取り消す
    let mut big = doc(1500, 1500, 128);
    let l = big.add_layer("a").unwrap();
    big.clear_history().unwrap();
    let mut brush = symmetric(SymmetryMode::Both, (750.0, 750.0), 2);
    brush.base.radius = 800.0;
    let mut s = big.begin_brush_stroke(l, &brush).unwrap();
    let r = s.add_point(&mut big, 700.0, 700.0, 1.0, DVec2::ZERO);
    assert_eq!(r, Err(CoreError::WorkingBudgetExceeded));
    assert!(!big.has_active_stroke(), "取り消した");
    assert_eq!(big.undo_count(), 0);
}

// ───────── 予算・壊れた入力 ─────────

#[test]
fn selection_changes_over_the_working_budget_are_refused_before_allocating() {
    let d = doc(256, 256, 64);
    let m = SelectionMask::all(&d);
    let tiny = 1000;
    assert_eq!(m.grow(5, tiny), Err(CoreError::WorkingBudgetExceeded));
    assert_eq!(
        m.shrink(5, false, tiny),
        Err(CoreError::WorkingBudgetExceeded)
    );
    assert_eq!(
        m.border(5, false, tiny),
        Err(CoreError::WorkingBudgetExceeded)
    );
    assert_eq!(
        m.feather(5.0, false, tiny),
        Err(CoreError::WorkingBudgetExceeded)
    );
    // 半径 0 や空の選択範囲は作業が要らないので、予算が 0 でも通る
    assert!(m.grow(0, 0).is_ok());
    assert!(SelectionMask::none(&d).feather(50.0, false, 0).is_ok());
    // 元の選択範囲は変わらない（不変の札）
    assert_eq!(m, SelectionMask::all(&d));
    // 十分な予算なら通る
    assert!(m.feather(5.0, false, BUDGET).is_ok());
}

#[test]
fn stored_tiles_that_could_not_have_been_written_are_refused() {
    let tile = |fill: u8| vec![fill; 16 * 16];
    let ok = |tiles: Vec<(TileCoord, Vec<u8>)>| SelectionMask::from_amount_tiles(20, 20, 16, tiles);
    // 正しいタイル（20×20 の右上のタイルは 4×4 だけが画布）
    let mut corner = vec![0u8; 256];
    for y in 0..4 {
        for x in 0..4 {
            corner[y * 16 + x] = 200;
        }
    }
    let good = ok(vec![
        (TileCoord::new(0, 0), tile(255)),
        (TileCoord::new(1, 1), corner.clone()),
    ])
    .unwrap();
    assert_eq!(good.amount(3, 3), 255);
    assert_eq!(good.amount(19, 19), 200);
    assert_eq!(good.amount(16, 0), 0);
    // 画布の外のタイル、長さの違うタイル、全部 0 のタイル、重なり、画布の外の余白の量
    assert!(ok(vec![(TileCoord::new(2, 0), tile(1))]).is_err());
    assert!(ok(vec![(TileCoord::new(0, 2), tile(1))]).is_err());
    assert!(ok(vec![(TileCoord::new(0, 0), vec![1; 255])]).is_err());
    assert!(ok(vec![(TileCoord::new(0, 0), vec![1; 257])]).is_err());
    assert!(ok(vec![(TileCoord::new(0, 0), tile(0))]).is_err());
    assert!(ok(vec![
        (TileCoord::new(0, 0), tile(1)),
        (TileCoord::new(0, 0), tile(2))
    ])
    .is_err());
    assert!(
        ok(vec![(TileCoord::new(1, 1), tile(9))]).is_err(),
        "右上のタイルの余白に量がある"
    );
    // 大きさ・タイルの大きさの範囲
    assert!(SelectionMask::empty(0, 10, 16).is_err());
    assert!(SelectionMask::empty(10, 0, 16).is_err());
    assert!(SelectionMask::empty(32769, 10, 16).is_err());
    assert!(SelectionMask::empty(10, 10, 0).is_err());
    assert!(SelectionMask::empty(10, 10, 1025).is_err());
    assert!(SelectionMask::empty(32768, 32768, 1024).is_ok());
    // タイルが 0 枚なら空の選択範囲
    assert!(ok(vec![]).unwrap().is_empty());
}

#[test]
fn reading_a_selection_reports_its_size_and_tiles() {
    let d = doc(40, 40, 16);
    let one = SelectionMask::rectangle(&d, 2, 2, 6, 6);
    assert_eq!(one.tile_coords(), vec![TileCoord::new(0, 0)]);
    assert_eq!(one.tile_bounds(), Some((0, 0, 16, 16)));
    assert_eq!(SelectionMask::none(&d).tile_bounds(), None);
    let edge = SelectionMask::rectangle(&d, 35, 35, 40, 40);
    assert_eq!(edge.tile_bounds(), Some((32, 32, 40, 40)), "画布で切る");
    // 一様なタイルは 1 バイト、履歴の重さは C# の RGBA のタイルでの大きさ
    let all = SelectionMask::all(&d);
    assert_eq!(all.tile_coords().len(), 9);
    assert!(all.allocated_bytes() < 9 * 16 * 16);
    assert_eq!(one.history_bytes(), 16 * 16 * 4);
    // タイルの量の読み出し
    let mut out = vec![0u8; 256];
    assert!(one.copy_tile(TileCoord::new(0, 0), &mut out).unwrap());
    assert_eq!(out[2 * 16 + 2], 255);
    assert!(!one.copy_tile(TileCoord::new(1, 0), &mut out).unwrap());
    assert!(out.iter().all(|&a| a == 0), "無いタイルは 0 で埋める");
    assert!(
        one.copy_tile(TileCoord::new(3, 0), &mut out).is_err(),
        "画布の外のタイル"
    );
    assert!(
        one.copy_tile(TileCoord::new(0, 0), &mut out[..10]).is_err(),
        "長さが違う"
    );
    assert_eq!(one.amount(1000, 1000), 0, "画布の外は 0");
}

#[test]
fn per_pixel_painting_for_the_3d_view_does_not_mirror_in_uv_space() {
    // 3D の面のストロークは apply_pixel で画素を塗る（写しは面の側で作る）ので、2D の対称の設定は見ない
    let mut d = doc(64, 64, 16);
    let a = d.add_layer("a").unwrap();
    let brush = symmetric(SymmetryMode::Both, (32.0, 32.0), 2);
    let mut s = d.begin_brush_stroke(a, &brush).unwrap();
    s.apply_pixel(&mut d, 10, 20, 1.0, 1.0).unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(px(&d, a, 10, 20), red());
    assert!(blank(&d, a, 53, 20) && blank(&d, a, 10, 43) && blank(&d, a, 53, 43));
    // 選択範囲は効く
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 32, 64)))
        .unwrap();
    let mut s = d
        .begin_brush_stroke(a, &Brush::from(thick(1.0, false)))
        .unwrap();
    s.apply_pixel(&mut d, 5, 5, 1.0, 1.0).unwrap();
    s.apply_pixel(&mut d, 50, 5, 1.0, 1.0).unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(px(&d, a, 5, 5), red());
    assert!(blank(&d, a, 50, 5));
}
