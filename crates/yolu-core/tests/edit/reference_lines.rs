//! はみ出し防止（`Brush::reference_lines`、参照レイヤーの線をはみ出さない）: ダブごとに中心からたどった範囲だけを塗ること（線の上の画素を
//! 塗らない・線をまたいだ両側を塗る・線のそばのダブは線の向こうを塗らない）、色の誤差、領域拡縮（丸く・四角く・最も濃いピクセルまで・縮める）、
//! 参照の印が無いとき、始めの凍結、2D の対称、3D の面のダブ、取り消し・Undo・選択範囲・透明部分のロック・予算、はみ出し防止が切のときの同じバイト。
#![allow(clippy::chunks_exact_to_as_chunks)]
use std::collections::HashSet;

use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushPixel, BrushSample, BrushSettings, CanvasSymmetry, Channel, CoreError, Document,
    EffectSettings, ExpandMode, FilterSpec, FilterTarget, LayerId, LayerLocks, PostCorrection,
    PressureSwitches, Rect, ReferenceLines, Rgba8, SelectionMask, SymmetryMode,
};

const W: u32 = 240;
const H: u32 = 80;
/// 縦の線の列（灰・黒・黒・灰）。
const LINE: [u32; 4] = [150, 151, 152, 153];
const ROW: f64 = 40.5;

const BLACK: Rgba8 = Rgba8::new(0, 0, 0, 255);
const GRAY: Rgba8 = Rgba8::new(0, 0, 0, 128);

/// 参照のレイヤー（縦の線 1 本）と、その上の塗るレイヤー。
fn scene() -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let r = d.add_layer("参照").unwrap();
    for y in 0..H {
        for (i, &x) in LINE.iter().enumerate() {
            let c = if i == 0 || i == 3 { GRAY } else { BLACK };
            d.set_pixel(r, x, y, c).unwrap();
        }
    }
    let p = d.add_layer("塗る").unwrap();
    d.clear_history().unwrap();
    (d, r, p)
}

fn brush(lines: ReferenceLines) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 10.0,
            hardness: 0.8,
            spacing: 0.1,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(224, 32, 32, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.reference_lines = lines;
    b
}

fn on() -> ReferenceLines {
    ReferenceLines {
        enabled: true,
        ..ReferenceLines::default()
    }
}

fn expanded(amount: f64, mode: ExpandMode) -> ReferenceLines {
    ReferenceLines {
        enabled: true,
        expand: true,
        amount,
        mode,
        ..ReferenceLines::default()
    }
}

/// `marked` を参照の印にして、点の列を 1 本のストロークで描く。use_reference_layers の答えも返す。
fn draw(
    d: &mut Document,
    layer: LayerId,
    b: &Brush,
    marked: &[LayerId],
    points: &[(f64, f64)],
) -> bool {
    let marked: HashSet<LayerId> = marked.iter().copied().collect();
    let mut s = d.begin_brush_stroke(layer, b).unwrap();
    let used = s.use_reference_layers(d, &marked, Channel::Color).unwrap();
    for &(x, y) in points {
        s.add_point(d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    used
}

fn alpha(d: &Document, layer: LayerId, x: u32, y: u32) -> u8 {
    d.layer(layer)
        .unwrap()
        .pixel(Channel::Color, x, y)
        .unwrap()
        .a
}

fn layer_bytes(d: &Document, layer: LayerId) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..d.height() {
        for x in 0..d.width() {
            out.extend_from_slice(
                &d.layer(layer)
                    .unwrap()
                    .pixel(Channel::Color, x, y)
                    .unwrap()
                    .to_array(),
            );
        }
    }
    out
}

const ACROSS: [(f64, f64); 2] = [(60.0, ROW), (220.0, ROW)];

#[test]
fn the_line_pixels_stay_unpainted_and_both_sides_are_painted() {
    let (mut d, r, p) = scene();
    assert!(draw(&mut d, p, &brush(on()), &[r], &ACROSS));
    let y = ROW as u32;
    for x in LINE {
        assert_eq!(alpha(&d, p, x, y), 0, "線の上の画素 x {x} は塗らない");
    }
    assert_eq!(alpha(&d, p, 149, y), 255, "線の手前");
    assert_eq!(alpha(&d, p, 154, y), 255, "線の向こう");
    assert_eq!(alpha(&d, p, 100, y), 255);
    assert_eq!(alpha(&d, p, 200, y), 255);
}

#[test]
fn dabs_near_the_line_do_not_paint_across_it_so_the_edge_thins_there() {
    // ダブごとに中心からたどる: 線のそばの縁（ストロークの上の端の行）は、線の向こうのダブの分が届かず薄くなる
    let (mut d, r, p) = scene();
    draw(&mut d, p, &brush(on()), &[r], &ACROSS);
    let (mut off, _, q) = scene();
    draw(&mut off, q, &brush(ReferenceLines::default()), &[], &ACROSS);
    let edge = (ROW + 8.5) as u32;
    let far = alpha(&d, p, 100, edge);
    assert_eq!(far, alpha(&off, q, 100, edge), "線から離れた所は切と同じ");
    let near = alpha(&d, p, 148, edge);
    assert!(near < far, "線のそばの縁は薄い: {near} / {far}");
    assert!(alpha(&d, p, 155, edge) < far);
}

#[test]
fn a_brush_without_reference_lines_paints_the_same_bytes() {
    let (mut a, r, p) = scene();
    let off = brush(ReferenceLines::default());
    assert!(
        !draw(&mut a, p, &off, &[r], &ACROSS),
        "切のブラシは何もしない"
    );
    let (mut b, _, q) = scene();
    let plain = brush(ReferenceLines::default());
    let mut s = b.begin_brush_stroke(q, &plain).unwrap();
    for &(x, y) in &ACROSS {
        s.add_point(&mut b, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    b.end_stroke(s).unwrap();
    assert_eq!(layer_bytes(&a, p), layer_bytes(&b, q));
}

#[test]
fn without_reference_marks_the_brush_paints_as_if_off() {
    let (mut a, _, p) = scene();
    assert!(
        !draw(&mut a, p, &brush(on()), &[], &ACROSS),
        "印が無ければ効かない"
    );
    let (mut b, _, q) = scene();
    draw(&mut b, q, &brush(ReferenceLines::default()), &[], &ACROSS);
    assert_eq!(layer_bytes(&a, p), layer_bytes(&b, q));
    // 消したレイヤーの印だけでも効かない
    let (mut c, r, s) = scene();
    let gone = c.add_layer("消す").unwrap();
    c.remove_layer(gone).unwrap();
    assert!(!draw(&mut c, s, &brush(on()), &[gone], &ACROSS));
    let _ = r;
}

#[test]
fn a_stroke_away_from_the_line_paints_the_same_bytes_as_off() {
    let (mut a, r, p) = scene();
    let short = [(40.0, ROW), (120.0, ROW)];
    draw(&mut a, p, &brush(on()), &[r], &short);
    let (mut b, _, q) = scene();
    draw(&mut b, q, &brush(ReferenceLines::default()), &[], &short);
    assert_eq!(layer_bytes(&a, p), layer_bytes(&b, q));
}

#[test]
fn the_color_margin_decides_what_counts_as_a_line() {
    // アルファ 30 の薄い線: 色の誤差 10 %（許し幅 26）では線、20 %（51）では線でない
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let r = d.add_layer("参照").unwrap();
    for y in 0..H {
        d.set_pixel(r, 150, y, Rgba8::new(0, 0, 0, 30)).unwrap();
    }
    let p = d.add_layer("塗る").unwrap();
    let mut e = d.capture_snapshot().unwrap();
    let narrow = ReferenceLines {
        margin: 10.0,
        ..on()
    };
    draw(&mut d, p, &brush(narrow), &[r], &ACROSS);
    assert_eq!(alpha(&d, p, 150, ROW as u32), 0);
    let wide = ReferenceLines {
        margin: 20.0,
        ..on()
    };
    draw(&mut e, p, &brush(wide), &[r], &ACROSS);
    assert_eq!(alpha(&e, p, 150, ROW as u32), 255);
}

#[test]
fn area_scaling_round_and_square_paint_over_the_line() {
    for mode in [ExpandMode::Round, ExpandMode::Square, ExpandMode::Darkest] {
        let (mut d, r, p) = scene();
        draw(&mut d, p, &brush(expanded(10.0, mode)), &[r], &ACROSS);
        for x in LINE {
            assert_eq!(
                alpha(&d, p, x, ROW as u32),
                255,
                "{mode:?}: 線の上も塗る x {x}"
            );
        }
    }
}

/// 縦と横の線で囲んだ角（左下）の中に 1 つだけダブを置く。
fn corner(lines: ReferenceLines) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(128, 128, 32).unwrap();
    let r = d.add_layer("参照").unwrap();
    for i in 0..128 {
        for k in [50, 51] {
            d.set_pixel(r, k, i, Rgba8::new(0, 0, 0, 255)).unwrap();
            d.set_pixel(r, i, k, Rgba8::new(0, 0, 0, 255)).unwrap();
        }
    }
    let p = d.add_layer("塗る").unwrap();
    let mut b = brush(lines);
    b.base.radius = 30.0;
    b.base.hardness = 1.0;
    draw(&mut d, p, &b, &[r], &[(40.5, 40.5)]);
    (d, p)
}

#[test]
fn round_square_and_darkest_scaling_differ_where_they_should() {
    let (round, p) = corner(expanded(10.0, ExpandMode::Round));
    let (square, q) = corner(expanded(10.0, ExpandMode::Square));
    let (dark, s) = corner(expanded(10.0, ExpandMode::Darkest));
    let (plain, t) = corner(on());
    // 線を越えた所（開いているがつながっていない）: 丸く・四角くは届き、最も濃いピクセルまでは線で止まる
    assert_eq!(alpha(&round, p, 53, 30), 255);
    assert_eq!(alpha(&square, q, 53, 30), 255);
    assert_eq!(alpha(&dark, s, 53, 30), 0);
    assert_eq!(alpha(&dark, s, 51, 30), 255, "線は塗る");
    assert_eq!(alpha(&plain, t, 50, 30), 0);
    // 角の斜め: 四角くは届き、丸くは届かない
    assert_eq!(alpha(&square, q, 58, 58), 255);
    assert_eq!(alpha(&round, p, 58, 58), 0);
}

#[test]
fn negative_area_scaling_shrinks_away_from_the_line() {
    let (mut d, r, p) = scene();
    draw(
        &mut d,
        p,
        &brush(expanded(-3.0, ExpandMode::Round)),
        &[r],
        &ACROSS,
    );
    let y = ROW as u32;
    for x in 147..=149 {
        assert_eq!(alpha(&d, p, x, y), 0, "線から 3 画素は塗らない x {x}");
    }
    assert_eq!(alpha(&d, p, 146, y), 255);
    assert_eq!(alpha(&d, p, 157, y), 255);
    assert_eq!(alpha(&d, p, 156, y), 0);
}

#[test]
fn painting_on_the_reference_layer_itself_keeps_the_frozen_reference() {
    // 参照のレイヤーそのものに描く: 塗った赤で範囲が変わらない（始めの凍結のまま）
    let (mut d, r, _) = scene();
    draw(&mut d, r, &brush(on()), &[r], &ACROSS);
    let y = ROW as u32;
    for x in [60, 100, 149, 154, 200, 220] {
        assert_eq!(
            d.layer(r).unwrap().pixel(Channel::Color, x, y).unwrap(),
            Rgba8::new(224, 32, 32, 255),
            "x {x}"
        );
    }
    assert_eq!(
        d.layer(r).unwrap().pixel(Channel::Color, 151, y).unwrap(),
        BLACK
    );
}

#[test]
fn the_reference_is_set_only_before_the_first_dab() {
    let (mut d, r, p) = scene();
    let marked: HashSet<LayerId> = [r].into_iter().collect();
    let mut s = d.begin_brush_stroke(p, &brush(on())).unwrap();
    s.add_point(&mut d, 60.0, ROW, 1.0, DVec2::ZERO).unwrap();
    let e = s
        .use_reference_layers(&mut d, &marked, Channel::Color)
        .unwrap_err();
    assert!(matches!(e, CoreError::Unsupported(_)), "{e:?}");
    assert!(!d.has_active_stroke(), "ストロークごと取り消す");
    assert_eq!(alpha(&d, p, 60, ROW as u32), 0);
}

#[test]
fn symmetric_copies_find_their_own_range() {
    // 縦の軸 x = 120 の対称: 元（右）は線をまたぐ、写し（左。線が無い）は全部塗る
    let (mut d, r, p) = scene();
    let mut b = brush(on());
    b.symmetry = CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(120.0, 40.0), 2).unwrap();
    draw(&mut d, p, &b, &[r], &[(130.0, ROW), (170.0, ROW)]);
    let y = ROW as u32;
    assert_eq!(alpha(&d, p, 151, y), 0, "元の線の上");
    assert_eq!(alpha(&d, p, 160, y), 255);
    assert_eq!(
        alpha(&d, p, 240 - 1 - 151, y),
        255,
        "写しの同じ所は線が無い"
    );
}

#[test]
fn surface_dabs_follow_the_uv_pixels_of_the_reference() {
    // 3D の面のダブ: 線をまたぐ 1 つの塊は中心の側だけ、離れた塊（継ぎ目の向こう）は覆いの一番大きい画素から
    let (mut d, r, p) = scene();
    let marked: HashSet<LayerId> = [r].into_iter().collect();
    let mut s = d.begin_brush_stroke(p, &brush(on())).unwrap();
    assert!(s
        .use_reference_layers(&mut d, &marked, Channel::Color)
        .unwrap());
    let mut pixels = Vec::new();
    for y in 30..50 {
        for x in 140..165 {
            pixels.push(BrushPixel {
                x,
                y,
                coverage: 1.0,
            });
        }
        for x in 10..20 {
            pixels.push(BrushPixel {
                x,
                y,
                coverage: 0.5,
            });
        }
    }
    assert!(s
        .apply_dab(&mut d, &pixels, DVec2::new(145.5, 40.5), 1.0)
        .unwrap());
    d.end_stroke(s).unwrap();
    assert_eq!(alpha(&d, p, 145, 40), 255);
    assert_eq!(alpha(&d, p, 151, 40), 0, "線");
    assert_eq!(alpha(&d, p, 160, 40), 0, "線の向こう");
    assert!(alpha(&d, p, 15, 40) > 0, "離れた塊");
}

#[test]
fn undo_cancel_selection_lock_and_budget_behave_like_any_stroke() {
    // Undo は 1 段
    let (mut d, r, p) = scene();
    let before = layer_bytes(&d, p);
    draw(&mut d, p, &brush(on()), &[r], &ACROSS);
    d.undo().unwrap();
    assert_eq!(layer_bytes(&d, p), before);
    assert!(!d.can_undo());
    // 取り消し
    let marked: HashSet<LayerId> = [r].into_iter().collect();
    let mut s = d.begin_brush_stroke(p, &brush(on())).unwrap();
    s.use_reference_layers(&mut d, &marked, Channel::Color)
        .unwrap();
    s.add_point(&mut d, 60.0, ROW, 1.0, DVec2::ZERO).unwrap();
    s.add_point(&mut d, 220.0, ROW, 1.0, DVec2::ZERO).unwrap();
    d.cancel_stroke(s);
    assert_eq!(layer_bytes(&d, p), before);
    // 選択範囲: 範囲と選択の両方の内側だけ
    let sel = SelectionMask::rectangle(&d, 0, 0, 200, H as i64);
    d.set_selection(Some(sel)).unwrap();
    draw(&mut d, p, &brush(on()), &[r], &ACROSS);
    assert_eq!(alpha(&d, p, 180, ROW as u32), 255);
    assert_eq!(alpha(&d, p, 210, ROW as u32), 0, "選択の外");
    assert_eq!(alpha(&d, p, 151, ROW as u32), 0, "線");
    d.set_selection(None).unwrap();
    // 透明部分のロック: 透明な画素は塗らない
    let (mut e, r2, q) = scene();
    for x in 100..=200 {
        e.set_pixel(q, x, ROW as u32, Rgba8::new(0, 0, 255, 255))
            .unwrap();
    }
    e.set_layer_locks(q, LayerLocks::TRANSPARENCY).unwrap();
    draw(&mut e, q, &brush(on()), &[r2], &ACROSS);
    assert_eq!(alpha(&e, q, 90, ROW as u32), 0, "透明のまま");
    assert_eq!(alpha(&e, q, 120, ROW as u32), 255);
    assert_eq!(
        e.layer(q)
            .unwrap()
            .pixel(Channel::Color, 151, ROW as u32)
            .unwrap(),
        Rgba8::new(0, 0, 255, 255),
        "線の上は元の色のまま"
    );
    // 予算: 参照の合成を覚える分も数え、超えたらストロークごと取り消す
    let (mut f, r3, t) = scene();
    f.set_stroke_budget_bytes(4096).unwrap();
    let marked: HashSet<LayerId> = [r3].into_iter().collect();
    let mut s = f.begin_brush_stroke(t, &brush(on())).unwrap();
    s.use_reference_layers(&mut f, &marked, Channel::Color)
        .unwrap();
    let e = s
        .add_point(&mut f, 60.0, ROW, 1.0, DVec2::ZERO)
        .unwrap_err();
    assert_eq!(e, CoreError::StrokeBudgetExceeded);
    assert!(!f.has_active_stroke());
    assert_eq!(alpha(&f, t, 60, ROW as u32), 0);
}

#[test]
fn out_of_range_settings_are_refused() {
    let (mut d, _, p) = scene();
    for bad in [
        ReferenceLines {
            margin: -1.0,
            ..on()
        },
        ReferenceLines {
            margin: 101.0,
            ..on()
        },
        ReferenceLines {
            amount: 100.5,
            ..on()
        },
        ReferenceLines {
            amount: f64::NAN,
            ..on()
        },
    ] {
        assert!(matches!(
            d.begin_brush_stroke(p, &brush(bad)),
            Err(CoreError::InvalidArgument(_))
        ));
    }
}

/// 参照のレイヤーのぼかし（内容のフィルター）が元の面のタイルの外へ広げた画素も、線として読む。参照の重なりの合成を画素で書いた文書と、
/// 同じブラシ・同じ線で同じ画素になる（バケツが全面で合成して読む画素と同じ）。
#[test]
fn spread_pixels_of_a_filtered_reference_layer_are_read_as_lines() {
    for lines in [
        on(),
        expanded(6.0, ExpandMode::Darkest),
        expanded(-3.0, ExpandMode::Round),
    ] {
        let (mut a, ra, pa) = scene();
        // 太い線（x 140〜159 はタイルの右端まで）にぼかしを掛ける
        for y in 0..H {
            for x in 140..160 {
                a.set_pixel(ra, x, y, BLACK).unwrap();
            }
        }
        a.add_filter(
            ra,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(12)).channels(&[Channel::Color]),
        )
        .unwrap();
        // ぼかしは、線のあるタイル（x 128〜159）の外の x 160〜 へ広がる
        assert_eq!(
            a.layer(ra)
                .unwrap()
                .pixel(Channel::Color, 160, 40)
                .unwrap()
                .a,
            0,
            "元の面は x 160 に画素を持たない"
        );
        let marked: HashSet<LayerId> = [ra].into();
        let set = a.layer(ra).unwrap().layer_set();
        let flat = a
            .reference_view(&marked)
            .unwrap()
            .composite_set_channel(set, Channel::Color, Rect::new(0, 0, W, H))
            .unwrap();
        let at = |x: u32, y: u32| {
            let i = (((H - 1 - y) * W + x) * 4) as usize;
            Rgba8::from_slice(&flat[i..i + 4])
        };
        assert!(at(160, 40).a > 100, "合成では x 160 に濃く広がっている");

        let mut b = Document::with_tile_size(W, H, 32).unwrap();
        let rb = b.add_layer("参照").unwrap();
        for y in 0..H {
            for x in 0..W {
                if at(x, y).a > 0 {
                    b.set_pixel(rb, x, y, at(x, y)).unwrap();
                }
            }
        }
        let pb = b.add_layer("塗る").unwrap();
        b.clear_history().unwrap();

        let brush = brush(lines);
        draw(&mut a, pa, &brush, &[ra], &ACROSS);
        draw(&mut b, pb, &brush, &[rb], &ACROSS);
        let (bytes_a, bytes_b) = (layer_bytes(&a, pa), layer_bytes(&b, pb));
        let differing = bytes_a
            .chunks(4)
            .zip(bytes_b.chunks(4))
            .filter(|(x, y)| x != y)
            .count();
        assert_eq!(differing, 0, "違う画素の数 {lines:?}");
        // 広がった先の線の上は塗らない（元の面だけを見ると、x 160〜 は線に見えず塗られる）
        assert_eq!(alpha(&a, pa, 160, 40), alpha(&b, pb, 160, 40));
    }
}

#[test]
fn the_default_color_margin_matches_the_bucket_tolerance() {
    // 値を変えないときに、バケツの「近い色」の初めの許し幅（32）と同じ許し幅で近い色を見る
    assert_eq!(ReferenceLines::default().tolerance(), 32);
}

/// 線を横切る、y が 2 画素ごとに上下へ 0.6 揺れる入力（1 画素おき、後補正の許し 1 画素より小さな揺れ）。
fn wobbly_across() -> Vec<BrushSample> {
    (0..=160)
        .map(|i| {
            let y = ROW + if (i / 2) % 2 == 0 { 0.6 } else { -0.6 };
            BrushSample::new(
                60.0 + f64::from(i),
                y,
                1.0,
                f64::from(i) / 200.0,
                DVec2::ZERO,
            )
            .unwrap()
        })
        .collect()
}

fn post_corrected(lines: ReferenceLines) -> Brush {
    let mut b = brush(lines);
    b.assist.post = PostCorrection {
        on: true,
        strength: 10.0, // 許し 1 画素 > 揺れ 0.6
        by_speed: false,
        by_view_scale: false,
        ..PostCorrection::default()
    };
    b
}

/// `marked` を参照の印にして、サンプル列を 1 本のストロークで描く。
fn draw_samples(
    d: &mut Document,
    layer: LayerId,
    b: &Brush,
    marked: &[LayerId],
    at: &[BrushSample],
) {
    let marked: HashSet<LayerId> = marked.iter().copied().collect();
    let mut s = d.begin_brush_stroke(layer, b).unwrap();
    assert!(s.use_reference_layers(d, &marked, Channel::Color).unwrap());
    for p in at {
        s.add_sample(d, *p).unwrap();
    }
    d.end_stroke(s).unwrap();
}

#[test]
fn a_post_corrected_stroke_is_redrawn_with_the_frozen_reference() {
    // 後補正は離したときに整えた入力で描き直す。描き直しも、始めに凍結した参照を持ち、線の向こうを塗らない
    let (mut d, r, p) = scene();
    draw_samples(&mut d, p, &post_corrected(on()), &[r], &wobbly_across());
    // 描き直したこと（後補正が無い同じブラシの線とは別の画素）
    let (mut raw, r2, q) = scene();
    draw_samples(&mut raw, q, &brush(on()), &[r2], &wobbly_across());
    assert_ne!(layer_bytes(&d, p), layer_bytes(&raw, q), "描き直した");
    for y in 34..48 {
        for x in LINE {
            assert_eq!(alpha(&d, p, x, y), 0, "線の上の画素 ({x}, {y}) は塗らない");
        }
    }
    assert_eq!(alpha(&d, p, 149, ROW as u32), 255, "線の手前");
    assert_eq!(alpha(&d, p, 154, ROW as u32), 255, "線の向こう");
    // 参照のレイヤーそのものに描いても、始めに凍結した参照のまま（描き直しの前に塗った赤で、範囲は変わらない）
    let (mut e, r3, _) = scene();
    draw_samples(&mut e, r3, &post_corrected(on()), &[r3], &wobbly_across());
    let y = ROW as u32;
    assert_eq!(
        e.layer(r3).unwrap().pixel(Channel::Color, 151, y).unwrap(),
        BLACK
    );
    assert_eq!(
        e.layer(r3).unwrap().pixel(Channel::Color, 100, y).unwrap(),
        Rgba8::new(224, 32, 32, 255)
    );
}

/// 参照のある線（`wobbly_across`）を離す前まで描いたときの、ストロークの持つバイト（巻き戻しの写しと、凍結した参照・覚えた入力など）。
fn held_while_drawing(b: &Brush) -> u64 {
    let (mut d, r, p) = scene();
    let marked: HashSet<LayerId> = [r].into_iter().collect();
    let mut s = d.begin_brush_stroke(p, b).unwrap();
    s.use_reference_layers(&mut d, &marked, Channel::Color)
        .unwrap();
    for q in wobbly_across() {
        s.add_sample(&mut d, q).unwrap();
    }
    let held = d.active_stroke_stats().unwrap().rollback_bytes;
    d.end_stroke(s).unwrap();
    held
}

#[test]
fn the_stroke_counts_both_the_remembered_inputs_and_the_frozen_reference() {
    // 後補正の覚えた入力と、はみ出し防止の凍結した参照の両方を持つストロークが数えるバイトは、巻き戻しの写し（どれも同じ線なので同じ）に、
    // 凍結した参照の分と覚えた入力の分を足した大きさ。どちらかが数えられていなければ合わない
    // （はみ出し防止が切のブラシでは use_reference_layers は何も凍結しない）
    let neither = held_while_drawing(&brush(ReferenceLines::default()));
    let reference = held_while_drawing(&brush(on()));
    let remembered = held_while_drawing(&post_corrected(ReferenceLines::default()));
    let both = held_while_drawing(&post_corrected(on()));
    assert!(reference > neither && remembered > neither);
    assert_eq!(both, reference + remembered - neither);
}
