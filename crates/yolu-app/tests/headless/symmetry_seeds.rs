//! バケツの近い色と自動選択に、2D の対称定規が効く試験。押した点を写しの全部の点にして、それぞれから範囲を求めた和を 1 回で塗る・選ぶ。
//! 画面を描かない。文書には、写しの位置に同じ色の小さな四角（8 画素四方）だけを置き、その四角が塗られる・選ばれる数で数える。
use crate::common;

use egui::{pos2, vec2, Modifiers, Rect};
use yolu_app::canvas::view::CanvasView;
use yolu_app::engine::{composite_pixel, Rgba8};
use yolu_app::region::tools::{bucket, Where};
use yolu_app::selection::canvas::press;
use yolu_app::state::{Action, AppState, StrokeSource, Tool};

const SIZE: u32 = 256;
const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const GREEN: [u8; 4] = [0, 255, 0, 255];
const HALF: i64 = 4;

type Point = (f64, f64);

fn polar(center: Point, radius: f64, degrees: f64) -> Point {
    let a = degrees.to_radians();
    (center.0 + radius * a.cos(), center.1 + radius * a.sin())
}

fn inside(p: Point) -> bool {
    (0.0..SIZE as f64).contains(&p.0) && (0.0..SIZE as f64).contains(&p.1)
}

/// 画面を使わない 256 四方の文書と、キャンバスの表示。
fn canvas() -> (AppState, CanvasView) {
    let s = AppState::new(SIZE, SIZE);
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 512.0));
    let view = s.view.view(rect, SIZE, SIZE);
    (s, view)
}

/// 選んでいるレイヤーの `at` を中心にした 8 画素四方を赤くする。
fn block(s: &mut AppState, at: Point) {
    let layer = s.selected_layer.unwrap();
    let (cx, cy) = (at.0.floor() as i64, at.1.floor() as i64);
    for y in cy - HALF..cy + HALF {
        for x in cx - HALF..cx + HALF {
            s.doc.set_pixel(layer, x as u32, y as u32, RED).unwrap();
        }
    }
}

/// 四角の中の画素が全部 `want` か（合成の色）。
fn block_is(s: &AppState, at: Point, want: [u8; 4]) -> bool {
    let (cx, cy) = (at.0.floor() as i64, at.1.floor() as i64);
    (cy - HALF..cy + HALF).all(|y| {
        (cx - HALF..cx + HALF).all(|x| composite_pixel(&s.doc, x as u32, y as u32) == want)
    })
}

fn bucket_state() -> (AppState, CanvasView) {
    let (mut s, view) = canvas();
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s.tool = Tool::Fill;
    s.region.by_color = true;
    s.region.tolerance = 0;
    (s, view)
}

fn wand_state() -> (AppState, CanvasView) {
    let (mut s, view) = canvas();
    s.apply(Action::SelectTool(Tool::Wand));
    s.sel.tolerance = 0;
    (s, view)
}

fn click_wand(s: &mut AppState, view: &CanvasView, at: Point) {
    press(
        s,
        view,
        view.to_screen(at.0, at.1),
        StrokeSource::Mouse,
        Modifiers::NONE,
        0.0,
    );
}

fn click_bucket(s: &mut AppState, view: &CanvasView, at: Point) {
    bucket(s, Where::Canvas(view), view.to_screen(at.0, at.1));
}

/// 選択範囲に入っている画素の数。
fn selected_pixels(s: &AppState) -> usize {
    s.doc
        .selection()
        .map(|m| m.to_canvas_bytes().iter().filter(|&&a| a > 0).count())
        .unwrap_or(0)
}

const CENTER: Point = (128.0, 128.0);
/// 押す点の中心からの向き（度）と距離。どの写しとも重ならない向き 50 度の四角を、塗られてはいけない物として置く。
const FIRST: f64 = 20.0;
const RADIUS: f64 = 60.0;
const DECOY: f64 = 50.0;

/// 対称定規と、押した点 `P` の写しとして期待する点（`P` が最初）。
struct Case {
    name: &'static str,
    put: fn(&mut AppState),
    images: Vec<Point>,
}

fn cases() -> Vec<Case> {
    let p = polar(CENTER, RADIUS, FIRST);
    vec![
        Case {
            name: "線対称 2 本（縦の軸）",
            put: |s| {
                common::rulers::vertical(s, 128.0);
            },
            images: vec![p, (256.0 - p.0, p.1)],
        },
        Case {
            name: "線対称 4 本（縦と横の軸）",
            put: |s| {
                common::rulers::both(s, (128.0, 128.0));
            },
            images: vec![
                p,
                (256.0 - p.0, p.1),
                (p.0, 256.0 - p.1),
                (256.0 - p.0, 256.0 - p.1),
            ],
        },
        Case {
            name: "線対称 6 本（最初の線は 0 度）",
            put: |s| {
                common::rulers::symmetry_2d(s, (128.0, 128.0), (1.0, 0.0), 6, true);
            },
            // 3 つの回転（0・120・240 度）と、0・60・120 度の軸の 3 枚の鏡（向き θ は 2φ − θ に写る）
            images: [20.0, 140.0, 260.0, -20.0, 100.0, 220.0]
                .map(|d| polar(CENTER, RADIUS, d))
                .to_vec(),
        },
        Case {
            name: "回転対称 6 本",
            put: |s| {
                common::rulers::radial(s, (128.0, 128.0), 6);
            },
            images: (0..6)
                .map(|j| polar(CENTER, RADIUS, FIRST + 60.0 * j as f64))
                .collect(),
        },
    ]
}

/// 写しの位置の四角と、写しでない位置の四角を置く。
fn lay_out(s: &mut AppState, case: &Case) {
    for &p in &case.images {
        block(s, p);
    }
    block(s, polar(CENTER, RADIUS, DECOY));
    (case.put)(s);
}

#[test]
fn the_bucket_fills_every_copy_of_the_pressed_point_in_one_undo() {
    for case in cases() {
        let (mut s, view) = bucket_state();
        lay_out(&mut s, &case);
        let before = s.doc.undo_count();
        click_bucket(&mut s, &view, case.images[0]);
        assert_eq!(
            s.doc.undo_count(),
            before + 1,
            "{}: 1 回の取り消し",
            case.name
        );
        for &p in &case.images {
            assert!(
                block_is(&s, p, GREEN),
                "{}: 写し {p:?} が塗られる",
                case.name
            );
        }
        assert!(
            block_is(&s, polar(CENTER, RADIUS, DECOY), [255, 0, 0, 255]),
            "{}: 写しでない四角は塗らない",
            case.name
        );
        s.apply(Action::Undo);
        for &p in &case.images {
            assert!(
                block_is(&s, p, [255, 0, 0, 255]),
                "{}: 取り消しで全部戻る",
                case.name
            );
        }
    }
}

#[test]
fn the_bucket_with_the_snap_off_fills_only_the_pressed_place() {
    for case in cases() {
        let (mut s, view) = bucket_state();
        lay_out(&mut s, &case);
        s.region.snap_symmetry = false;
        click_bucket(&mut s, &view, case.images[0]);
        assert!(block_is(&s, case.images[0], GREEN), "{}", case.name);
        for &p in &case.images[1..] {
            assert!(
                block_is(&s, p, [255, 0, 0, 255]),
                "{}: 切なら写しは塗らない {p:?}",
                case.name
            );
        }
    }
}

#[test]
fn the_wand_selects_every_copy_of_the_pressed_point_in_one_undo() {
    for case in cases() {
        let (mut s, view) = wand_state();
        lay_out(&mut s, &case);
        let before = s.doc.undo_count();
        click_wand(&mut s, &view, case.images[0]);
        assert_eq!(
            s.doc.undo_count(),
            before + 1,
            "{}: 1 回の取り消し",
            case.name
        );
        assert_eq!(
            selected_pixels(&s),
            case.images.len() * 64,
            "{}: 写しの数だけ四角が選ばれる",
            case.name
        );
        let m = s.doc.selection().unwrap();
        for &p in &case.images {
            assert_eq!(
                m.amount(p.0.floor() as u32, p.1.floor() as u32),
                255,
                "{}: {p:?}",
                case.name
            );
        }
        s.apply(Action::Undo);
        assert!(s.doc.selection().is_none(), "{}: 取り消しで戻る", case.name);
    }
}

#[test]
fn the_wand_with_the_snap_off_selects_only_the_pressed_place() {
    for case in cases() {
        let (mut s, view) = wand_state();
        lay_out(&mut s, &case);
        s.sel.snap_symmetry = false;
        click_wand(&mut s, &view, case.images[0]);
        assert_eq!(selected_pixels(&s), 64, "{}", case.name);
    }
}

#[test]
fn the_wand_combines_the_sum_with_the_creation_mode_once() {
    let case = &cases()[0];
    let (mut s, view) = wand_state();
    lay_out(&mut s, case);
    // 追加: 先に選んでいた別の四角（写しでない）に、写しの和を足す
    let decoy = polar(CENTER, RADIUS, DECOY);
    s.sel.snap_symmetry = false;
    click_wand(&mut s, &view, decoy);
    assert_eq!(selected_pixels(&s), 64);
    s.sel.snap_symmetry = true;
    s.sel.combine = yolu_app::engine::SelectionCombine::Add;
    let before = s.doc.undo_count();
    click_wand(&mut s, &view, case.images[0]);
    assert_eq!(s.doc.undo_count(), before + 1, "追加も 1 回の取り消し");
    assert_eq!(selected_pixels(&s), 3 * 64);
    // 削除: 写しの和を引く
    s.sel.combine = yolu_app::engine::SelectionCombine::Subtract;
    click_wand(&mut s, &view, case.images[0]);
    assert_eq!(
        selected_pixels(&s),
        64,
        "写しの 2 つとも引かれ、先に選んだ四角が残る"
    );
}

#[test]
fn copies_outside_the_canvas_are_dropped() {
    // 回転対称 6 本を隅の近くに置く: 6 つの写しのうちキャンバスの内にあるものだけが種になる
    let center = (30.0, 30.0);
    let images: Vec<Point> = (0..6)
        .map(|j| polar(center, RADIUS, FIRST + 60.0 * j as f64))
        .collect();
    let inner: Vec<Point> = images.iter().copied().filter(|&p| inside(p)).collect();
    assert!(
        inner.len() == 2,
        "試験の前提: 内の写しは 2 つ（{}）",
        inner.len()
    );
    {
        let (mut s, view) = bucket_state();
        for &p in &inner {
            block(&mut s, p);
        }
        common::rulers::radial(&mut s, center, 6);
        let before = s.doc.undo_count();
        click_bucket(&mut s, &view, images[0]);
        assert_eq!(s.doc.undo_count(), before + 1);
        for &p in &inner {
            assert!(block_is(&s, p, GREEN), "{p:?}");
        }
        assert_eq!(s.message, "塗りました。", "外の写しは断らずに捨てる");
    }
    {
        let (mut s, view) = wand_state();
        for &p in &inner {
            block(&mut s, p);
        }
        common::rulers::radial(&mut s, center, 6);
        click_wand(&mut s, &view, images[0]);
        assert_eq!(selected_pixels(&s), inner.len() * 64);
    }
    // 線対称 2 本の軸の近く: 写しがキャンバスの外へ出る押し方は、押した所だけ
    {
        let (mut s, view) = bucket_state();
        let p = (60.0, 100.0);
        block(&mut s, p);
        common::rulers::vertical(&mut s, 20.0);
        click_bucket(&mut s, &view, p);
        assert!(block_is(&s, p, GREEN));
        assert_eq!(s.message, "塗りました。");
    }
}

#[test]
fn seeds_that_land_on_the_same_pixel_are_used_once() {
    let (mut s, view) = bucket_state();
    common::rulers::vertical(&mut s, 128.0);
    // 軸の上: 写しは同じ画素に重なる
    assert_eq!(s.symmetry_seeds((128.0, 100.5), true).len(), 1);
    // 軸の両側の隣り合う画素は別の種で、同じ四角の中なので、和は四角 1 つ
    assert_eq!(s.symmetry_seeds((128.3, 100.5), true).len(), 2);
    block(&mut s, (128.0, 100.5));
    let before = s.doc.undo_count();
    click_bucket(&mut s, &view, (128.3, 100.5));
    assert_eq!(s.doc.undo_count(), before + 1);
    assert!(block_is(&s, (128.0, 100.5), GREEN));
    // 切なら、定規があっても押した点だけ
    assert_eq!(
        s.symmetry_seeds((128.3, 100.5), false),
        vec![(128.3, 100.5)]
    );
    // 外の点は種にならない
    assert!(s.symmetry_seeds((-1.0, 5.0), false).is_empty());
}

#[test]
fn without_a_symmetry_ruler_the_pressed_point_is_the_only_seed() {
    let (mut s, view) = bucket_state();
    block(&mut s, (60.5, 100.5));
    block(&mut s, (195.5, 100.5));
    click_bucket(&mut s, &view, (60.5, 100.5));
    assert!(block_is(&s, (60.5, 100.5), GREEN));
    assert!(block_is(&s, (195.5, 100.5), [255, 0, 0, 255]));
    // 直線定規だけでも同じ
    common::rulers::line(&mut s, (0.0, 10.0), (256.0, 10.0));
    assert_eq!(s.symmetry_seeds((60.5, 100.5), true), vec![(60.5, 100.5)]);
}

#[test]
fn with_snap_to_special_ruler_off_the_symmetry_does_not_move_the_seeds() {
    let (mut s, _) = bucket_state();
    common::rulers::vertical(&mut s, 128.0);
    assert_eq!(s.symmetry_seeds((60.5, 100.5), true).len(), 2);
    s.rulers.snap_special = false;
    assert_eq!(s.symmetry_seeds((60.5, 100.5), true).len(), 1);
}

#[test]
fn a_non_contiguous_bucket_unions_the_matching_pixels_of_every_seed() {
    // 隣接を切る: 種ごとの「近い色の全部」の和。色の違う 2 つの種からは、それぞれの色の画素が塗られる
    let (mut s, view) = bucket_state();
    s.region.contiguous = false;
    let layer = s.selected_layer.unwrap();
    block(&mut s, (60.5, 100.5));
    // 軸の反対側の四角は青
    let blue = Rgba8::new(0, 0, 255, 255);
    for y in 96..104 {
        for x in 191..199 {
            s.doc.set_pixel(layer, x, y, blue).unwrap();
        }
    }
    common::rulers::vertical(&mut s, 128.0);
    click_bucket(&mut s, &view, (60.5, 100.5));
    assert!(block_is(&s, (60.5, 100.5), GREEN));
    assert!(
        block_is(&s, (195.5, 100.5), GREEN),
        "鏡の側の青い四角も、その色に近い画素として塗られる"
    );
    // 赤でも青でもない画素（透明）は塗らない
    assert_eq!(composite_pixel(&s.doc, 5, 5), [0, 0, 0, 0]);
}

/// 横に色が少しずつ変わる（赤の成分が x）レイヤーを敷く。
fn lay_gradient(s: &mut AppState) {
    let layer = s.selected_layer.unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            s.doc
                .set_pixel(layer, x, y, Rgba8::new(x as u8, 0, 0, 255))
                .unwrap();
        }
    }
}

fn filled_columns(s: &AppState, y: u32) -> Vec<bool> {
    (0..SIZE)
        .map(|x| {
            let p = composite_pixel(&s.doc, x, y);
            p[1] > p[0]
        })
        .collect()
}

fn selected_columns(s: &AppState, y: u32) -> Vec<bool> {
    let m = s.doc.selection().expect("選択範囲");
    (0..SIZE).map(|x| m.amount(x, y) > 0).collect()
}

#[test]
fn the_bucket_finds_the_range_of_each_seed_by_its_own_color_and_matches_the_wand() {
    // 縦の軸 x = 63 の鏡: 画素 60 の写しは 65。許容 10 の範囲は 60 が [50, 70]、65 が [55, 75]。65 は 60 の範囲の中にあるが、
    // 自分の色の範囲（71〜75 も入る）で求める
    let (mut s, view) = bucket_state();
    lay_gradient(&mut s);
    s.region.tolerance = 10;
    common::rulers::vertical(&mut s, 63.0);
    click_bucket(&mut s, &view, (60.5, 100.5));
    let filled = filled_columns(&s, 100);
    let want: Vec<bool> = (0..SIZE).map(|x| (50..=75).contains(&x)).collect();
    assert_eq!(filled, want, "和は 50〜75 の列");
    // 同じ押し方の自動選択と同じ範囲
    let (mut w, wview) = wand_state();
    lay_gradient(&mut w);
    w.sel.tolerance = 10;
    common::rulers::vertical(&mut w, 63.0);
    click_wand(&mut w, &wview, (60.5, 100.5));
    assert_eq!(selected_columns(&w, 100), filled, "自動選択と同じ");
    // 写しの側（65）を押しても同じ範囲（種の順によらない）
    let (mut b, view2) = bucket_state();
    lay_gradient(&mut b);
    b.region.tolerance = 10;
    common::rulers::vertical(&mut b, 63.0);
    click_bucket(&mut b, &view2, (65.5, 100.5));
    assert_eq!(filled_columns(&b, 100), filled, "種の順によらない");
}

#[test]
fn pressing_outside_the_canvas_the_wand_uses_the_edge_pixel_and_its_copy_and_the_bucket_does_nothing(
) {
    // 縦の軸 x = 128 の鏡。左の外 (-20, 100.5) を押すと、左の端の画素 0 の中心 (0.5, 100.5) と、その写し (255.5, 100.5)
    let (mut s, view) = wand_state();
    block(&mut s, (4.0, 100.5));
    block(&mut s, (252.0, 100.5));
    common::rulers::vertical(&mut s, 128.0);
    let before = s.doc.undo_count();
    click_wand(&mut s, &view, (-20.0, 100.5));
    assert_eq!(s.doc.undo_count(), before + 1, "{}", s.message);
    let m = s.doc.selection().expect("選択範囲");
    assert_eq!(m.amount(2, 100), 255, "寄せた端の画素の四角");
    assert_eq!(m.amount(253, 100), 255, "その写しの四角");
    assert_eq!(selected_pixels(&s), 2 * 64);
    // バケツは、押した点が外なら、対称定規があっても何も塗らない
    let (mut b, bview) = bucket_state();
    block(&mut b, (4.0, 100.5));
    block(&mut b, (252.0, 100.5));
    common::rulers::vertical(&mut b, 128.0);
    let steps = b.doc.undo_count();
    click_bucket(&mut b, &bview, (-20.0, 100.5));
    assert_eq!(b.doc.undo_count(), steps);
    assert!(block_is(&b, (4.0, 100.5), [255, 0, 0, 255]));
    assert!(block_is(&b, (252.0, 100.5), [255, 0, 0, 255]));
    click_bucket(&mut b, &bview, (276.0, 100.5));
    assert_eq!(b.doc.undo_count(), steps, "右の外も同じ");
}

#[test]
fn the_wand_intersects_the_sum_of_the_copies_once() {
    let case = &cases()[0];
    let (mut s, view) = wand_state();
    lay_out(&mut s, case);
    // 写しの 2 つと写しでない四角の 3 つを選んでおく
    let decoy = polar(CENTER, RADIUS, DECOY);
    s.sel.snap_symmetry = false;
    click_wand(&mut s, &view, decoy);
    s.sel.snap_symmetry = true;
    s.sel.combine = yolu_app::engine::SelectionCombine::Add;
    click_wand(&mut s, &view, case.images[0]);
    assert_eq!(selected_pixels(&s), 3 * 64);
    // 共通: 写しの和との重なり（写しの 2 つ）だけが残る
    s.sel.combine = yolu_app::engine::SelectionCombine::Intersect;
    let before = s.doc.undo_count();
    click_wand(&mut s, &view, case.images[0]);
    assert_eq!(s.doc.undo_count(), before + 1, "1 回の取り消し");
    assert_eq!(selected_pixels(&s), 2 * 64, "写しの 2 つだけ");
    let m = s.doc.selection().unwrap();
    for &p in &case.images {
        assert_eq!(m.amount(p.0.floor() as u32, p.1.floor() as u32), 255);
    }
    assert_eq!(m.amount(decoy.0.floor() as u32, decoy.1.floor() as u32), 0);
}

#[test]
fn copies_over_the_budget_are_refused_and_nothing_changes() {
    // バケツ: 作業の予算が足りない（文書のレイヤーの画素の予算をぎりぎりにする）
    let case = &cases()[0];
    let (mut s, view) = bucket_state();
    lay_out(&mut s, case);
    let allocated = s.doc.allocated_bytes();
    s.doc.set_source_budget_bytes(allocated).unwrap();
    let before = s.doc.undo_count();
    click_bucket(&mut s, &view, case.images[0]);
    assert_ne!(s.message, "塗りました。", "断りの文が出る");
    assert!(!s.message.is_empty());
    assert_eq!(s.doc.undo_count(), before);
    for &p in &case.images {
        assert!(block_is(&s, p, [255, 0, 0, 255]), "画素は元のまま {p:?}");
    }
    assert!(!s.modified || s.doc.undo_count() == before);
    // 自動選択: 作業の予算（1 GiB）を超える大きさ（16384² の全面）。最初の種で断り、選択範囲は変わらない
    let mut w = AppState::new(16384, 16384);
    w.apply(Action::SelectTool(Tool::Wand));
    common::rulers::vertical(&mut w, 8192.0);
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 512.0));
    let view = w.view.view(rect, 16384, 16384);
    let steps = w.doc.undo_count();
    click_wand(&mut w, &view, (4000.5, 8000.5));
    let budget = w
        .lang
        .core_error(&yolu_app::engine::CoreError::WorkingBudgetExceeded);
    assert!(w.message.contains(&budget), "{}", w.message);
    assert!(w.doc.selection().is_none());
    assert_eq!(w.doc.undo_count(), steps);
}
