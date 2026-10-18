//! リボン（2D）: 筆先の画像（横に 4 つの濃さの帯）を持つブラシで、まっすぐな線が模様を線の長さのとおりに続けて写す（帯の継ぎ目に
//! 抜けが無い）。複数の筆先は繰り返し方法の順につながる。丸い筆先・散布効果では効かない。取り消しで戻る。
use std::sync::Arc;

use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, BrushTip, Channel, CoreError, Document, LayerId, Rgba8, Spray,
    TipSelection,
};

const W: u32 = 256;
const H: u32 = 64;

/// 幅 w・高さ h の筆先。列 x の覆いは value(x)。
fn tip(w: u32, h: u32, value: impl Fn(u32) -> u8) -> Arc<BrushTip> {
    let alpha = (0..h).flat_map(|_| (0..w).map(&value)).collect();
    Arc::new(BrushTip::new("band", w, h, alpha).unwrap())
}

/// 横に 4 つの濃さ（60・120・180・240）の帯の筆先（64 × 16）。
fn bands() -> Arc<BrushTip> {
    tip(64, 16, |x| [60, 120, 180, 240][(x / 16) as usize])
}

/// 半径 8（筆先の高さ 16 が直径に当たり、画像の 1 画素がキャンバスの 1 画素）・間隔 4 画素・流量と不透明度 1 のブラシ。
fn brush(tips: Vec<Arc<BrushTip>>, ribbon: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 8.0,
        hardness: 1.0,
        spacing: 0.25,
        opacity: 1.0,
        flow: 1.0,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.tip.images = tips;
    b.tip.selection = TipSelection::Sequential;
    b.tip.ribbon = ribbon;
    b
}

fn new_doc() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    doc.clear_history().unwrap();
    (doc, layer)
}

fn bytes(doc: &Document) -> Vec<u8> {
    doc.layers()[0]
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
}

fn alpha(doc: &Document, x: u32, y: u32) -> u8 {
    bytes(doc)[((y * W + x) * 4 + 3) as usize]
}

fn draw(doc: &mut Document, layer: LayerId, b: &Brush, points: &[(f64, f64)]) {
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for &(x, y) in points {
        s.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    doc.end_stroke(s).unwrap();
}

/// x 20 から 220 までの、y 32 の横の線（入力は 7 画素ごと: 入力の区切りは間隔とそろわない）。
fn straight() -> Vec<(f64, f64)> {
    (0..=28).map(|i| (20.0 + 7.0 * i as f64, 32.0)).collect()
}

#[test]
fn a_straight_line_copies_the_pattern_by_arc_length_without_seams() {
    let (mut doc, l) = new_doc();
    draw(&mut doc, l, &brush(vec![bands()], true), &straight());
    // 線の始まりから、帯の濃さが 16 画素ごとに上がり、64 画素ごとに繰り返す（端の画素は延ばして読むので、帯の中は一定）
    for x in 21..218u32 {
        let along = x - 20;
        let want = [60u8, 120, 180, 240][((along % 64) / 16) as usize];
        let got = alpha(&doc, x, 32);
        let edge = along % 16 == 0 || along % 16 == 15;
        if edge {
            assert!(got > 0, "x {x}: 帯の継ぎ目に抜けが無い");
        } else {
            assert!(got.abs_diff(want) <= 1, "x {x}: {got} ≠ {want}");
        }
    }
    // 線の幅は画像の高さ（直径 16）
    assert!(alpha(&doc, 100, 32 + 7) > 0 && alpha(&doc, 100, 32 - 8) > 0);
    assert_eq!(alpha(&doc, 100, 32 + 9), 0);
    assert_eq!(alpha(&doc, 100, 32 - 10), 0);
}

#[test]
fn several_tips_join_in_the_repeat_order() {
    // 16 × 16 の 2 枚（覆い 100 と 200）を順に: 線の長さ 16 ごとに入れ替わる
    let tips = vec![tip(16, 16, |_| 100), tip(16, 16, |_| 200)];
    let (mut doc, l) = new_doc();
    draw(&mut doc, l, &brush(tips, true), &straight());
    for x in 21..218u32 {
        let along = x - 20;
        if along % 16 == 0 || along % 16 == 15 {
            continue;
        }
        let want = if (along / 16) % 2 == 0 { 100 } else { 200 };
        assert!(alpha(&doc, x, 32).abs_diff(want) <= 1, "x {x}");
    }
}

#[test]
fn round_tips_and_spray_ignore_the_ribbon() {
    for (with_ribbon, without) in [(brush(vec![], true), brush(vec![], false)), {
        let mut a = brush(vec![bands()], true);
        a.spray = Some(Spray::default());
        let mut b = brush(vec![bands()], false);
        b.spray = Some(Spray::default());
        (a, b)
    }] {
        assert!(!with_ribbon.ribbon_active());
        let (mut a, la) = new_doc();
        draw(&mut a, la, &with_ribbon, &straight());
        let (mut b, lb) = new_doc();
        draw(&mut b, lb, &without, &straight());
        assert_eq!(bytes(&a), bytes(&b));
    }
}

#[test]
fn a_curved_ribbon_is_deterministic_and_undoes() {
    let curve: Vec<(f64, f64)> = (0..=40)
        .map(|i| {
            let t = i as f64 / 40.0;
            (30.0 + 190.0 * t, 32.0 + 18.0 * (t * 6.0).sin())
        })
        .collect();
    let (mut a, la) = new_doc();
    let before = bytes(&a);
    let b = brush(vec![bands(), tip(8, 16, |x| (x * 30) as u8)], true);
    draw(&mut a, la, &b, &curve);
    let (mut c, lc) = new_doc();
    draw(&mut c, lc, &b, &curve);
    assert_eq!(bytes(&a), bytes(&c));
    assert_ne!(bytes(&a), before);
    a.undo().unwrap();
    assert_eq!(bytes(&a), before);
}

/// 幅 1・高さ h の細長い筆先のリボン（半径 8・間隔 4 画素）。筆先 1 枚は 16 ÷ h 画素なので、描点 1 つが新しく写す帯（間隔 4 画素）は
/// 0.25 × h 枚（h = 240 で 60 枚、h = 300 で 75 枚、h = 1024 で 256 枚）。1 つの描点が写す枚数の上限は 64。
fn thin(h: u32) -> Brush {
    brush(vec![tip(1, h, |_| 200)], true)
}

#[test]
fn a_tall_tip_at_or_over_the_piece_limit_leaves_no_gap_in_a_straight_line() {
    // 60 枚: 上限の内。75 枚: 1 つの描点では写し切れず、残りを次の描点が続けて写す（抜けを作らない。遅れは線の終わりに残る）
    for h in [240, 300] {
        let (mut doc, l) = new_doc();
        draw(&mut doc, l, &thin(h), &straight());
        for x in 21..170u32 {
            assert!(alpha(&doc, x, 32) > 0, "h {h}: x {x} に抜けが無い");
        }
    }
}

#[test]
fn a_ribbon_that_cannot_keep_up_with_its_stamps_is_refused_and_leaves_nothing() {
    // 256 枚: 描点ごとに写し残しが溜まり続けるので、遅れが間隔の 16 個分を超えたらストロークを断って取り消す
    let (mut doc, l) = new_doc();
    let before = bytes(&doc);
    let mut s = doc.begin_brush_stroke(l, &thin(1024)).unwrap();
    let refused = straight()
        .into_iter()
        .find_map(|(x, y)| s.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).err());
    assert_eq!(refused, Some(CoreError::StrokeBudgetExceeded));
    assert!(!doc.has_active_stroke(), "ストロークは自分で取り消した");
    assert_eq!(bytes(&doc), before);
    assert_eq!(doc.undo_count(), 0);
}
