//! 連続吹き付け（2D）: 同じ位置で時刻だけ進む入力の列で、ダブが時刻の間隔ごとに増える。時刻が進まない入力では増えない。切なら今と
//! 同じバイト。後補正と同時なら切として扱う。遅い線は速い線より濃い。取り消しで戻る。
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, Channel, CoreError, Document, LayerId, PostCorrection,
    Rgba8, CONTINUOUS_INTERVAL, MAX_CONTINUOUS_PER_INPUT,
};

const W: u32 = 128;
const H: u32 = 64;

fn brush(continuous: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 6.0,
        hardness: 0.5,
        spacing: 0.25,
        opacity: 1.0,
        flow: 0.08,
        color: Rgba8::new(20, 40, 160, 255),
        ..BrushSettings::default()
    });
    b.tip.continuous = continuous;
    b
}

fn sample(x: f64, y: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, time, DVec2::ZERO).unwrap()
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

fn alpha_at(doc: &Document, x: u32, y: u32) -> u8 {
    bytes(doc)[((y * W + x) * 4 + 3) as usize]
}

/// 点の列を 1 つのストロークで描いて、置いたダブの数。
fn draw(doc: &mut Document, layer: LayerId, b: &Brush, samples: &[BrushSample]) -> u64 {
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for p in samples {
        s.add_sample(doc, *p).unwrap();
    }
    doc.end_stroke(s).unwrap().stamps
}

/// 押した点で n 回、時刻だけ 1/60 秒ずつ進める入力の列。
fn held(n: usize) -> Vec<BrushSample> {
    (0..=n)
        .map(|i| sample(40.5, 30.5, i as f64 * CONTINUOUS_INTERVAL + 1e-9 * i as f64))
        .collect()
}

#[test]
fn holding_the_pen_adds_a_dab_per_interval() {
    for n in [0usize, 1, 5, 30] {
        let (mut doc, l) = new_doc();
        assert_eq!(
            draw(&mut doc, l, &brush(true), &held(n)),
            1 + n as u64,
            "{n}"
        );
        // 切なら押した点の 1 つだけ
        let (mut doc, l) = new_doc();
        assert_eq!(draw(&mut doc, l, &brush(false), &held(n)), 1, "{n}");
    }
}

#[test]
fn inputs_that_do_not_advance_time_add_nothing() {
    let (mut doc, l) = new_doc();
    let same: Vec<BrushSample> = (0..20).map(|_| sample(40.5, 30.5, 0.5)).collect();
    assert_eq!(draw(&mut doc, l, &brush(true), &same), 1);
    // 同じ入力を 2 回ずつ渡しても、時刻のダブは時刻の分だけ
    let (mut doc, l) = new_doc();
    let doubled: Vec<BrushSample> = held(10).into_iter().flat_map(|p| [p, p]).collect();
    assert_eq!(draw(&mut doc, l, &brush(true), &doubled), 11);
}

#[test]
fn a_long_pause_is_capped_per_input() {
    let (mut doc, l) = new_doc();
    let samples = [sample(40.5, 30.5, 0.0), sample(40.5, 30.5, 10.0)];
    assert_eq!(
        draw(&mut doc, l, &brush(true), &samples),
        1 + u64::from(MAX_CONTINUOUS_PER_INPUT)
    );
}

#[test]
fn holding_darkens_the_spot_and_the_same_inputs_give_the_same_bytes() {
    let (mut a, la) = new_doc();
    draw(&mut a, la, &brush(true), &held(40));
    let (mut b, lb) = new_doc();
    draw(&mut b, lb, &brush(false), &held(40));
    assert!(alpha_at(&a, 40, 30) > alpha_at(&b, 40, 30));
    let (mut c, lc) = new_doc();
    draw(&mut c, lc, &brush(true), &held(40));
    assert_eq!(bytes(&a), bytes(&c), "同じ入力の列なら同じバイト");
}

#[test]
fn turned_off_or_with_post_correction_it_paints_the_same_bytes_as_before() {
    // 動く線を、動いていない間を挟んで描く
    let mut line = Vec::new();
    for i in 0..=40 {
        let t = i as f64 * 0.01;
        let x = 10.0
            + if i < 20 {
                i as f64 * 2.0
            } else {
                40.0 + (i - 20) as f64 * 0.2
            };
        line.push(sample(x, 30.0, t));
    }
    let (mut off, lo) = new_doc();
    draw(&mut off, lo, &brush(false), &line);
    // 後補正が入っていれば連続吹き付けは切と同じ
    let mut with_post = brush(true);
    with_post.assist.post = PostCorrection {
        on: true,
        ..PostCorrection::default()
    };
    let mut post_only = brush(false);
    post_only.assist.post = with_post.assist.post;
    let (mut p1, l1) = new_doc();
    draw(&mut p1, l1, &with_post, &line);
    let (mut p2, l2) = new_doc();
    draw(&mut p2, l2, &post_only, &line);
    assert_eq!(bytes(&p1), bytes(&p2));
    assert!(!with_post.continuous_active());
    // 連続吹き付けが入りなら、遅い所に時刻のダブが足される
    let (mut on, ln) = new_doc();
    let stamps_on = draw(&mut on, ln, &brush(true), &line);
    let (mut off2, lo2) = new_doc();
    let stamps_off = draw(&mut off2, lo2, &brush(false), &line);
    assert!(stamps_on > stamps_off, "{stamps_on} {stamps_off}");
    assert_eq!(bytes(&off), bytes(&off2));
}

#[test]
fn a_slow_stroke_is_darker_than_a_fast_one() {
    // 同じ道（x 20〜100）を、速く（入力ごとに 4 画素・1/60 秒）と遅く（入力ごとに 0.5 画素・1/60 秒）描く
    let stroke = |step: f64| -> Vec<BrushSample> {
        let n = (80.0 / step) as usize;
        (0..=n)
            .map(|i| {
                sample(
                    20.0 + i as f64 * step,
                    30.5,
                    i as f64 * CONTINUOUS_INTERVAL * 1.01,
                )
            })
            .collect()
    };
    let (mut fast, lf) = new_doc();
    draw(&mut fast, lf, &brush(true), &stroke(4.0));
    let (mut slow, ls) = new_doc();
    draw(&mut slow, ls, &brush(true), &stroke(0.5));
    let sum = |d: &Document| -> u64 { (20..100).map(|x| u64::from(alpha_at(d, x, 30))).sum() };
    assert!(sum(&slow) > sum(&fast), "{} {}", sum(&slow), sum(&fast));
}

#[test]
fn cancelling_and_undoing_restore_the_layer() {
    let (mut doc, l) = new_doc();
    let before = bytes(&doc);
    let mut s = doc.begin_brush_stroke(l, &brush(true)).unwrap();
    for p in held(20) {
        s.add_sample(&mut doc, p).unwrap();
    }
    doc.cancel_stroke(s);
    assert_eq!(bytes(&doc), before);
    draw(&mut doc, l, &brush(true), &held(20));
    assert_ne!(bytes(&doc), before);
    doc.undo().unwrap();
    assert_eq!(bytes(&doc), before);
}

/// 抜きのあるブラシ: 終わりから抜きの長さの内の描点は、線が伸びるか離すまで待たせる。
fn tapered() -> Brush {
    let mut b = brush(true);
    b.assist.taper_out = 24.0;
    b
}

/// 押したまま止めて n 入力、そのあと右へ動かす入力の列。
fn held_then_dragged(n: usize) -> Vec<BrushSample> {
    let mut v = held(n);
    let t = n as f64 * CONTINUOUS_INTERVAL;
    v.extend((1..=10).map(|i| sample(40.5 + 6.0 * i as f64, 30.5, t + i as f64 * 0.01)));
    v
}

#[test]
fn dabs_held_for_the_taper_count_against_the_stroke_budget() {
    // 予算の内: 止めて待たせたダブも、動かした後に置かれる。Undo で戻る
    let (mut doc, l) = new_doc();
    let before = bytes(&doc);
    let stamps = draw(&mut doc, l, &tapered(), &held_then_dragged(30));
    assert!(stamps > 30, "待たせたダブも置く: {stamps}");
    assert_ne!(bytes(&doc), before);
    doc.undo().unwrap();
    assert_eq!(bytes(&doc), before);

    // 予算の外: 止めている間に待たせるダブが増え続けるので、予算を超えたところでストロークごと取り消す
    let (mut doc, l) = new_doc();
    doc.set_stroke_budget_bytes(2000).unwrap();
    let mut s = doc.begin_brush_stroke(l, &tapered()).unwrap();
    let refused = held(200)
        .into_iter()
        .find_map(|p| s.add_sample(&mut doc, p).err());
    assert_eq!(refused, Some(CoreError::StrokeBudgetExceeded));
    assert!(!doc.has_active_stroke(), "ストロークは自分で取り消した");
    assert_eq!(bytes(&doc), before);
    assert_eq!(doc.undo_count(), 0);
}
