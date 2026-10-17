//! 補正の拡張（2D）: 手ぶれ補正の速度による調整の向き（補正タイプ 2 つ）・はらい（0・途中・100%）・後補正（強さ・速度による調整・
//! 表示倍率による調整、離したときの描き直しが 1 回の Undo、描いている途中の取り消し、選択範囲・透明部分のロック・予算・対称・Shift の直線）。
//! 新しい項目が既定のブラシは、今までと同じバイト。
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, CanvasSymmetry, Channel, CoreError, Document, LayerId,
    LayerLocks, PostCorrection, PressureSwitches, Rgba8, SelectionMask, StabilizerSpeed,
    StrokeAssist, SymmetryMode,
};

const W: u32 = 160;
const H: u32 = 96;

fn brush(radius: f64, pressure_size: bool) -> Brush {
    Brush::simple(
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
            size: pressure_size,
            opacity: false,
            flow: false,
        },
    )
}

fn post(strength: f64) -> PostCorrection {
    PostCorrection {
        on: true,
        strength,
        by_speed: false,
        by_view_scale: false,
        ..PostCorrection::default()
    }
}

fn sample(x: f64, y: f64, pressure: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, pressure, time, DVec2::ZERO).unwrap()
}

fn new_doc() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    (doc, layer)
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

/// 入力を順に足して離す（`corner_after` 番目の入力の後で直線を閉じる）。
fn draw_in(doc: &mut Document, layer: LayerId, b: &Brush, points: &[BrushSample]) {
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for p in points {
        s.add_sample(doc, *p).unwrap();
    }
    doc.end_stroke(s).unwrap();
}

fn draw(b: &Brush, points: &[BrushSample]) -> Vec<u8> {
    let (mut doc, layer) = new_doc();
    draw_in(&mut doc, layer, b, points);
    alpha(&doc)
}

/// y = 48 の横の線に、`amp` の高さの揺れ（2 画素ごとに上下）を足した入力（1 画素おき、`speed` 画素 / 秒）。
fn wobbly(amp: f64, speed: f64) -> Vec<BrushSample> {
    (0..=120)
        .map(|i| {
            let x = 20.0 + i as f64;
            let y = 48.0 + if (i / 2) % 2 == 0 { amp } else { -amp };
            sample(x, y, 1.0, i as f64 / speed)
        })
        .collect()
}

/// 列ごとの塗った画素の重心の y の、列の間の揺れ（となりの列との差の絶対値の平均）。まっすぐなら 0 に近い。
fn wobble(a: &[u8], from: usize, to: usize) -> f64 {
    let w = W as usize;
    let centre = |x: usize| {
        let (mut s, mut n) = (0.0, 0.0);
        for y in 0..H as usize {
            let v = f64::from(a[y * w + x]);
            s += v * y as f64;
            n += v;
        }
        s / n
    };
    let c: Vec<f64> = (from..to).map(centre).collect();
    c.windows(2).map(|p| (p[1] - p[0]).abs()).sum::<f64>() / (c.len() - 1) as f64
}

/// 列 x の塗った画素の数（太さ）。
fn thickness(a: &[u8], x: usize) -> usize {
    (0..H as usize)
        .filter(|&y| a[y * W as usize + x] > 0)
        .count()
}

#[test]
fn brushes_without_the_new_settings_draw_the_same_bytes() {
    let base = {
        let mut b = brush(4.0, true);
        b.assist.stabilizer = 6.0;
        b
    };
    let points: Vec<BrushSample> = (0..=40)
        .map(|i| {
            let t = i as f64 / 40.0;
            sample(20.0 + 120.0 * t, 30.0 + 30.0 * t * t, 1.0 - t, t)
        })
        .collect();
    let want = draw(&base, &points);
    // 後補正が切なら、強さ・速度・表示倍率の値も、表示倍率の受け渡しも線を変えない
    let mut off = base.clone();
    off.assist.post = PostCorrection {
        strength: 80.0,
        by_speed: false,
        ..PostCorrection::default()
    };
    assert_eq!(draw(&off.at_view_scale(3.0), &points), want);
    // 後補正が入りでも強さ 0 は描き直さない
    let mut zero = base.clone();
    zero.assist.post = post(0.0);
    assert_eq!(draw(&zero, &points), want);
    // 速度による調整・はらいが無ければ同じ
    assert_eq!(draw(&base.at_view_scale(0.5), &points), want);
}

/// 角 (60, 20) を回る折れ線の入力（1 画素おき、`speed` 画素 / 秒）。
fn corner(speed: f64) -> Vec<BrushSample> {
    let mut out = Vec::new();
    for i in 0..=40 {
        out.push((20.0 + i as f64, 20.0));
    }
    for i in 1..=50 {
        out.push((60.0, 20.0 + i as f64));
    }
    out.iter()
        .enumerate()
        .map(|(i, p)| sample(p.0, p.1, 1.0, i as f64 / speed))
        .collect()
}

/// 角の点 (60, 20) から、いちばん近い塗った画素（アルファ 128 超え）までの距離。手ぶれ補正の糸が長いほど角を内側へ切る。
fn corner_cut(a: &[u8]) -> f64 {
    let mut best = f64::INFINITY;
    for y in 0..H as usize {
        for x in 0..W as usize {
            if a[y * W as usize + x] > 128 {
                let d = (x as f64 + 0.5 - 60.0).hypot(y as f64 + 0.5 - 20.0);
                best = best.min(d);
            }
        }
    }
    best
}

#[test]
fn the_stabilizer_speed_type_sets_which_speed_is_corrected() {
    let mut b = brush(2.0, false);
    b.assist.stabilizer = 16.0;
    let (slow, fast) = (corner(30.0), corner(30000.0));
    let plain = corner_cut(&draw(&b, &slow));
    assert!(plain > 3.0, "糸が角を切る: {plain}");
    // 補正タイプ「すばやく描いたときに弱く補正」: 速いと角の切れが小さく、ゆっくりでは調整の無いときと同じ
    b.assist.stabilizer_speed = Some(StabilizerSpeed::WeakenWhenFast);
    let (weak_slow, weak_fast) = (corner_cut(&draw(&b, &slow)), corner_cut(&draw(&b, &fast)));
    assert!(weak_fast < weak_slow, "{weak_fast} < {weak_slow}");
    // 「ゆっくり描いたときに補正」: 速さの上限の 2 割を超えて速いと補正しない（角を切らない）
    b.assist.stabilizer_speed = Some(StabilizerSpeed::SlowOnly);
    let (only_slow, only_fast) = (corner_cut(&draw(&b, &slow)), corner_cut(&draw(&b, &fast)));
    assert!(only_fast < 1.0, "速いと補正しない: {only_fast}");
    assert!(only_slow > only_fast + 2.0, "{only_slow} > {only_fast}");
}

/// 筆圧 1 で 60 画素、そのあと 60 画素で筆圧 0 へ抜く横の線（0.5 秒）。
fn release() -> Vec<BrushSample> {
    (0..=120)
        .map(|i| {
            let p = if i <= 60 {
                1.0
            } else {
                1.0 - (i - 60) as f64 / 60.0
            };
            sample(20.0 + i as f64, 48.0, p, i as f64 / 240.0)
        })
        .collect()
}

#[test]
fn the_sweep_keeps_the_line_thick_while_pressure_is_released() {
    let mut b = brush(6.0, true);
    let points = release();
    let base = draw(&b, &points);
    let mut tails = Vec::new();
    for sweep in [0.0, 0.25, 0.5, 1.0] {
        b.assist.sweep = sweep;
        let a = draw(&b, &points);
        // 筆圧 1 の所（抜く前）は変わらない
        assert_eq!(thickness(&a, 60), thickness(&base, 60), "{sweep}");
        tails.push(thickness(&a, 128));
    }
    assert_eq!(
        draw(&brush(6.0, true), &points),
        base,
        "はらい 0 は今までと同じ"
    );
    assert!(
        tails.windows(2).all(|w| w[0] <= w[1]) && tails[0] < tails[1] && tails[1] < tails[3],
        "はらいが大きいほど抜きの終わりが太い: {tails:?}"
    );
    // はらい 100% で 48 画素抜いても、太さは 8 割以上残る（筆圧の下がり方の上限は長さ 1 あたり約 0.002）
    assert!(tails[3] * 10 >= thickness(&base, 60) * 8, "{tails:?}");
    // 筆圧が上がるときは、そのまま追う
    b.assist.sweep = 1.0;
    let rising: Vec<BrushSample> = (0..=100)
        .map(|i| sample(20.0 + i as f64, 48.0, i as f64 / 100.0, i as f64 / 200.0))
        .collect();
    assert_eq!(draw(&b, &rising), draw(&brush(6.0, true), &rising));
}

#[test]
fn post_correction_redraws_the_line_straighter_when_released() {
    let mut b = brush(2.0, false);
    let points = wobbly(0.6, 200.0);
    let raw = draw(&b, &points);
    b.assist.post = post(10.0); // 許し 1 画素 > 揺れ 0.6
    let (mut doc, layer) = new_doc();
    let mut s = doc.begin_brush_stroke(layer, &b).unwrap();
    for p in &points {
        s.add_sample(&mut doc, *p).unwrap();
    }
    // 描いている間は、補正の無い線を見せる
    assert_eq!(alpha(&doc), raw);
    doc.end_stroke(s).unwrap();
    let fixed = alpha(&doc);
    assert_ne!(fixed, raw);
    assert!(
        wobble(&fixed, 30, 130) < wobble(&raw, 30, 130) / 3.0,
        "{} < {}",
        wobble(&fixed, 30, 130),
        wobble(&raw, 30, 130)
    );
    // 描き直しは 1 回の Undo で、描く前へ戻る（レイヤーの追加の後に 1 段だけ）
    assert_eq!(doc.undo_count(), 2);
    assert!(doc.undo().unwrap());
    assert!(alpha(&doc).iter().all(|a| *a == 0));
    assert!(doc.redo().unwrap());
    assert_eq!(alpha(&doc), fixed);
    // 許しより大きな揺れは残す（強さ 2 は許し 0.2 画素）
    b.assist.post = post(2.0);
    assert_eq!(draw(&b, &points), raw);
}

#[test]
fn cancelling_a_post_corrected_stroke_leaves_nothing() {
    let mut b = brush(2.0, false);
    b.assist.post = post(10.0);
    let (mut doc, layer) = new_doc();
    let mut s = doc.begin_brush_stroke(layer, &b).unwrap();
    for p in wobbly(0.6, 200.0) {
        s.add_sample(&mut doc, p).unwrap();
    }
    assert!(alpha(&doc).iter().any(|a| *a > 0));
    assert!(doc.cancel_active_stroke());
    assert!(alpha(&doc).iter().all(|a| *a == 0));
    assert_eq!(
        doc.undo_count(),
        1,
        "取り消しは段を積まない（レイヤーの追加だけ）"
    );
}

#[test]
fn post_correction_follows_the_view_scale_and_the_speed() {
    let mut b = brush(2.0, false);
    let points = wobbly(0.6, 200.0);
    let raw = draw(&b, &points);
    // 表示倍率による調整: 許しは画面の点（強さ 10 で 1 点）。4 倍に拡大していれば文書の 0.25 画素で揺れが残り、縮小していれば消える
    b.assist.post = PostCorrection {
        by_view_scale: true,
        ..post(10.0)
    };
    assert_eq!(draw(&b.at_view_scale(4.0), &points), raw);
    let zoomed_out = draw(&b.at_view_scale(0.5), &points);
    assert!(wobble(&zoomed_out, 30, 130) < wobble(&raw, 30, 130) / 3.0);
    // 切なら表示倍率によらない
    b.assist.post = post(10.0);
    assert_eq!(
        draw(&b.at_view_scale(4.0), &points),
        draw(&b.at_view_scale(0.5), &points)
    );
    // 速度による調整: 速さの上限で許しが半分になり、ゆっくり描いた線より揺れが残る
    b.assist.post = PostCorrection {
        by_speed: true,
        ..post(10.0)
    };
    let fast = draw(&b, &wobbly(0.6, 1.0e6));
    let slow = draw(&b, &points);
    assert!(wobble(&slow, 30, 130) < wobble(&raw, 30, 130) / 3.0);
    assert!(
        wobble(&fast, 30, 130) > wobble(&slow, 30, 130) * 2.0,
        "{} > {}",
        wobble(&fast, 30, 130),
        wobble(&slow, 30, 130)
    );
}

#[test]
fn post_correction_stays_inside_the_selection_and_the_transparency_lock() {
    let mut b = brush(3.0, false);
    b.assist.post = post(10.0);
    let points = wobbly(0.6, 200.0);
    // 選択範囲の外は描き直しでも塗らない
    let (mut doc, layer) = new_doc();
    let mask = SelectionMask::rectangle(&doc, 0, 0, 80, H as i64);
    doc.set_selection(Some(mask)).unwrap();
    draw_in(&mut doc, layer, &b, &points);
    let a = alpha(&doc);
    assert!(
        (0..H as usize).all(|y| a[y * W as usize + 90] == 0),
        "選択範囲の外"
    );
    assert!(thickness(&a, 60) > 0);
    // 透明部分のロック: 透明な画素は透明のまま、塗ってある画素の色だけ変わる
    let (mut doc, layer) = new_doc();
    let mut fill = brush(10.0, false);
    fill.base.color = Rgba8::new(255, 0, 0, 255);
    draw_in(
        &mut doc,
        layer,
        &fill,
        &[sample(50.0, 48.0, 1.0, 0.0), sample(51.0, 48.0, 1.0, 0.01)],
    );
    let before = alpha(&doc);
    doc.set_layer_locks(layer, LayerLocks::TRANSPARENCY)
        .unwrap();
    draw_in(&mut doc, layer, &b, &points);
    assert_eq!(alpha(&doc), before, "アルファは変わらない");
}

/// 予算 `budget` バイトで `b` の線を最後まで描けるか（断られたら、ストロークは取り消されて何も残らない）。
fn completes(b: &Brush, budget: u64, points: &[BrushSample]) -> bool {
    let (mut doc, layer) = new_doc();
    doc.set_stroke_budget_bytes(budget).unwrap();
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for p in points {
        match s.add_sample(&mut doc, *p) {
            Ok(()) => {}
            Err(CoreError::StrokeBudgetExceeded) => {
                assert!(alpha(&doc).iter().all(|a| *a == 0), "何も残さない");
                assert!(doc.end_stroke(s).is_err());
                return false;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    doc.end_stroke(s).is_ok()
}

#[test]
fn a_post_corrected_stroke_over_the_budget_is_cancelled() {
    // 後補正が覚える入力の分だけが予算を超える大きさ: 後補正の無い同じ線が通る最小の予算に 2000 バイトを足した予算で、
    // 後補正の線は断られる（覚えた入力を数えなければ、タイルの写しだけで通ってしまう）
    let plain = brush(2.0, false);
    let mut corrected = plain.clone();
    corrected.assist.post = post(10.0);
    let points = wobbly(0.6, 200.0);
    let (mut low, mut high) = (1u64, 1 << 22);
    assert!(completes(&plain, high, &points));
    while low + 1 < high {
        let mid = (low + high) / 2;
        if completes(&plain, mid, &points) {
            high = mid;
        } else {
            low = mid;
        }
    }
    let plain_least = high;
    assert!(!completes(&plain, plain_least - 1, &points));
    // 覚える入力は 121 点 × 1 点 64 バイト以上で、2000 バイトを超える
    let budget = plain_least + 2000;
    assert!(completes(&plain, budget, &points), "後補正が無ければ通る");
    assert!(
        !completes(&corrected, budget, &points),
        "覚えた入力を数えると超える"
    );
    // 十分な予算なら通る
    assert!(completes(&corrected, 1 << 22, &points));
}

#[test]
fn post_correction_redraws_the_mirrored_copy_too() {
    let mut b = brush(2.0, false);
    b.assist.post = post(10.0);
    b.symmetry = CanvasSymmetry {
        mode: SymmetryMode::Vertical,
        center: DVec2::new(f64::from(W) / 2.0, f64::from(H) / 2.0),
        ..CanvasSymmetry::default()
    };
    let points: Vec<BrushSample> = wobbly(0.6, 200.0)
        .into_iter()
        .map(|p| sample(p.x * 0.5, p.y + 20.0, 1.0, p.time))
        .collect();
    let a = draw(&b, &points);
    let w = W as usize;
    for y in 0..H as usize {
        for x in 0..w {
            assert_eq!(
                a[y * w + x],
                a[y * w + (w - 1 - x)],
                "({x}, {y}) と鏡の画素"
            );
        }
    }
    assert!(a.iter().any(|v| *v > 0));
}

#[test]
fn post_correction_keeps_shift_line_corners() {
    let mut b = brush(2.0, false);
    b.assist.post = post(10.0);
    let (mut doc, layer) = new_doc();
    let mut s = doc.begin_brush_stroke(layer, &b).unwrap();
    // 揺れのある線の後に、Shift の直線（角）で下へ
    for p in wobbly(0.6, 200.0).iter().take(61) {
        s.add_sample(&mut doc, *p).unwrap();
    }
    s.end_line(&mut doc).unwrap();
    s.add_sample(&mut doc, sample(80.0, 90.0, 1.0, 1.0))
        .unwrap();
    s.end_line(&mut doc).unwrap();
    doc.end_stroke(s).unwrap();
    let a = alpha(&doc);
    // 角の点のそばを塗り、直線は (80, 48.6)→(80, 90) のまま縦
    assert!(a[48 * W as usize + 80] > 0);
    for y in 55..88 {
        assert!(a[y * W as usize + 80] > 128, "y {y}");
        assert_eq!(a[y * W as usize + 84], 0, "y {y}");
    }
    assert!(wobble(&a, 30, 70) < 0.1);
}

#[test]
fn stroke_assist_default_is_unchanged() {
    let a = StrokeAssist::default();
    assert!(!a.uses_correction());
    assert_eq!(a.sweep, 0.0);
    assert!(a.stabilizer_speed.is_none() && !a.post.on);
}

#[test]
fn post_correction_redraws_every_channel_of_a_material_stroke() {
    use yolu_core::material::ChannelPaint;
    let mut b = brush(2.0, false);
    let points = wobbly(0.6, 200.0);
    let material = [
        ChannelPaint::new(Channel::Color, Rgba8::new(200, 20, 10, 255)),
        ChannelPaint::new(Channel::Roughness, Rgba8::new(90, 90, 90, 255)),
    ];
    let paint = |b: &Brush| {
        let (mut doc, layer) = new_doc();
        doc.clear_history().unwrap();
        let mut s = doc
            .begin_material_brush_stroke(layer, &material, b)
            .unwrap();
        for p in &points {
            s.add_sample(&mut doc, *p).unwrap();
        }
        doc.end_stroke(s).unwrap();
        let channel = |c: Channel| -> Vec<u8> {
            doc.layers()[0]
                .surface(c)
                .unwrap()
                .to_canvas_bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[3])
                .collect()
        };
        (channel(Channel::Color), channel(Channel::Roughness), doc)
    };
    let (raw_color, _, _) = paint(&b);
    b.assist.post = post(10.0);
    let (color, rough, mut doc) = paint(&b);
    assert!(wobble(&color, 30, 130) < wobble(&raw_color, 30, 130) / 3.0);
    assert_eq!(color, rough, "全部のチャンネルを同じ線で描き直す");
    assert_eq!(doc.undo_count(), 1, "描き直しは 1 回の Undo");
    assert!(doc.undo().unwrap());
    assert!(doc.layers()[0]
        .surface(Channel::Color)
        .is_none_or(|s| s.to_canvas_bytes().iter().all(|v| *v == 0)));
}
