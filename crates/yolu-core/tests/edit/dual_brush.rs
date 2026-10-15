//! デュアルブラシの足した項目（合成モード 5 種・サイズの連動・濃度・影響元・向き・2 つ目のアンチエイリアス）の 2D のストローク:
//! モードの式の端の値、筆圧で主が細ると連動した 2 つ目も細ること、濃度と影響元と向きが 2 つ目のダブに効くこと、決まった種で同じ
//! バイト、Undo・Redo、範囲の外と NaN の断り。今のデュアルの設定のバイトは、ゴールデン（`golden.rs`・reference・3D の指紋）が見る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::glam::DVec2;
use yolu_core::{
    AntiAlias, Brush, BrushSample, BrushSettings, CoreError, Document, DualBrush, DualBrushMode,
    InputResponse, InputUse, PressureSwitches, Reason, Rgba8,
};

const SIDE: u32 = 128;
const CENTER: f64 = 64.0;

/// 半径 20 の硬い丸に、半径 6 の硬い丸の 2 つ目（乗算）を入れたもの。
fn dualed(dual: DualBrush) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 20.0,
            hardness: 1.0,
            spacing: 0.25,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(30, 60, 200, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.dual = Some(DualBrush {
        radius: 6.0,
        hardness: 1.0,
        ..dual
    });
    b.seed = 9;
    b
}

fn sample(x: f64, y: f64, pressure: f64, tilt: DVec2) -> BrushSample {
    BrushSample::new(x, y, pressure, 0.0, tilt).unwrap()
}

/// 点の列を描いた画素と、文書（Undo のため）。
fn draw(brush: &Brush, samples: &[BrushSample]) -> (Document, Vec<u8>) {
    let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for &p in samples {
        s.add_sample(&mut d, p).unwrap();
    }
    d.end_stroke(s).unwrap();
    let pixels = d.composite(d.bounds()).unwrap();
    (d, pixels)
}

/// 真ん中に 1 点だけ置いた画素。
fn dot(brush: &Brush, pressure: f64, tilt: DVec2) -> Vec<u8> {
    draw(brush, &[sample(CENTER, CENTER, pressure, tilt)]).1
}

/// 横の線（x = 16〜112）を筆圧 pressure で描いた画素。
fn stroke(brush: &Brush, pressure: f64) -> Vec<u8> {
    let points: Vec<BrushSample> = (0..9)
        .map(|i| sample(16.0 + 12.0 * i as f64, CENTER, pressure, DVec2::ZERO))
        .collect();
    draw(brush, &points).1
}

fn painted(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
}

fn max_alpha(pixels: &[u8]) -> u8 {
    pixels.chunks_exact(4).map(|p| p[3]).max().unwrap_or(0)
}

/// 塗った画素の外接の箱の幅と高さ。
fn extent(pixels: &[u8]) -> (u32, u32) {
    let (mut x0, mut x1, mut y0, mut y1) = (SIDE, 0, SIDE, 0);
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[3] != 0 {
            let (x, y) = (i as u32 % SIDE, i as u32 / SIDE);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
    }
    (x1 + 1 - x0, y1 + 1 - y0)
}

fn pressure_use() -> InputUse {
    InputUse {
        on: true,
        response: InputResponse::new(0.0, vec![]).unwrap(),
    }
}

#[test]
fn the_added_modes_follow_their_formulas_at_the_edges() {
    use DualBrushMode::*;
    for m in DualBrushMode::EXTENDED {
        assert!(m.is_extended() && !DualBrushMode::ALL.contains(&m), "{m:?}");
        assert_eq!(m.combine(0.0, 1.0), 0.0, "主が 0 なら 0: {m:?}");
        assert_eq!(m.combine(1.0, 1.0), 1.0, "{m:?}");
        assert_eq!(m.combine(1.0, 0.0), 1.0, "{m:?}");
    }
    assert_eq!(
        DualBrushMode::EVERY[..8],
        DualBrushMode::ALL,
        "今の 8 つの値の順のまま"
    );
    assert_eq!(DualBrushMode::EVERY[8..], DualBrushMode::EXTENDED);
    // 2 つ目が 0 のとき: 通常・加算・比較(明)・スクリーン・高さ(リニア) は主のまま
    for m in DualBrushMode::EXTENDED {
        assert_eq!(m.combine(0.5, 0.0), 0.5, "{m:?}");
    }
    assert_eq!(Normal.combine(0.5, 1.0), 0.5, "通常は 2 つ目を使わない");
    assert_eq!(Add.combine(0.5, 0.25), 0.75);
    assert_eq!(Add.combine(0.75, 0.5), 1.0, "加算は 1 で止まる");
    assert_eq!(Lighten.combine(0.25, 0.5), 0.5);
    assert_eq!(Lighten.combine(0.75, 0.5), 0.75);
    assert_eq!(Screen.combine(0.5, 0.5), 0.75);
    assert_eq!(LinearHeight.combine(0.5, 0.5), 0.25);
    assert_eq!(LinearHeight.combine(0.5, 1.0), 0.0);
    // 高さ(リニア) は減算より濃い（同じ値のとき、主が 0 でも 1 でもない所で）
    assert!(LinearHeight.combine(0.5, 0.25) > Subtract.combine(0.5, 0.25));
}

#[test]
fn the_added_modes_paint_like_their_formulas() {
    // 硬い主（覆い 1）: 通常は主のまま、加算・比較(明)・スクリーン・高さ(リニア) も主が 1 なので 1
    for mode in DualBrushMode::EXTENDED {
        let b = dualed(DualBrush {
            mode,
            ..DualBrush::default()
        });
        let pixels = dot(&b, 1.0, DVec2::ZERO);
        assert_eq!(max_alpha(&pixels), 255, "{mode:?}");
        // 主が 1 なので、主の円全部が塗れる（2 つ目で欠けない）
        let plain = dot(&dualed(DualBrush::default()), 1.0, DVec2::ZERO);
        assert!(
            painted(&pixels) > painted(&plain),
            "{mode:?}: 乗算は 2 つ目の所だけ"
        );
    }
}

#[test]
fn a_linked_secondary_shrinks_with_the_main_brush_and_an_unlinked_one_does_not() {
    let mut linked = dualed(DualBrush {
        link_size: true,
        ..DualBrush::default()
    });
    linked.influences.size.pressure = pressure_use();
    let mut unlinked = linked.clone();
    unlinked.dual.as_mut().unwrap().link_size = false;
    // 筆圧 1: 主は設定のまま、連動の比は 1 で同じ
    assert_eq!(
        dot(&linked, 1.0, DVec2::ZERO),
        dot(&unlinked, 1.0, DVec2::ZERO)
    );
    // 筆圧 0.5: 主は半径 10。連動しない 2 つ目は半径 6 のまま、連動した 2 つ目は半径 3
    let full = painted(&dot(&unlinked, 0.5, DVec2::ZERO));
    let small = painted(&dot(&linked, 0.5, DVec2::ZERO));
    let r6 = std::f64::consts::PI * 36.0;
    let r3 = std::f64::consts::PI * 9.0;
    assert!((full as f64 - r6).abs() < r6 * 0.15, "{full}");
    assert!((small as f64 - r3).abs() < r3 * 0.3, "{small}");
}

#[test]
fn the_density_scales_what_the_secondary_lays_down() {
    let half = dualed(DualBrush {
        flow: 0.5,
        ..DualBrush::default()
    });
    let a = max_alpha(&dot(&half, 1.0, DVec2::ZERO));
    assert!((126..=130).contains(&a), "乗算で 0.5: {a}");
    // 濃度 1 は今のまま
    let full = dualed(DualBrush {
        flow: 1.0,
        ..DualBrush::default()
    });
    assert_eq!(max_alpha(&dot(&full, 1.0, DVec2::ZERO)), 255);
    // 濃度の影響元（筆圧）: 筆圧 0.5 で半分
    let mut by_pressure = dualed(DualBrush::default());
    by_pressure.influences.dual_flow.pressure = pressure_use();
    let a = max_alpha(&dot(&by_pressure, 0.5, DVec2::ZERO));
    assert!((126..=130).contains(&a), "{a}");
}

#[test]
fn the_size_and_thickness_sources_change_the_secondary_dab() {
    let plain = dualed(DualBrush::default());
    let mut sized = plain.clone();
    sized.influences.dual_size.pressure = pressure_use();
    // 主は筆圧で変わらない（主の大きさの影響元を使わない）
    let base = painted(&dot(&plain, 0.5, DVec2::ZERO));
    let small = painted(&dot(&sized, 0.5, DVec2::ZERO));
    assert!(small * 3 < base, "半径 3 と 6: {small}・{base}");
    let mut thin = plain.clone();
    thin.influences.dual_thickness.pressure = pressure_use();
    let (w, h) = extent(&dot(&thin, 0.3, DVec2::ZERO));
    assert!(w > h * 2, "厚さ 0.3 の 2 つ目は横長: {w}×{h}");
}

#[test]
fn the_spacing_source_places_the_secondary_dabs_closer() {
    let wide = dualed(DualBrush {
        spacing: 4.0,
        ..DualBrush::default()
    });
    let mut closer = wide.clone();
    closer.influences.dual_spacing.pressure = pressure_use();
    // 筆圧 1 は同じ間隔
    assert_eq!(stroke(&wide, 1.0), stroke(&closer, 1.0));
    // 筆圧 0.125 で間隔は 8 分の 1（48 から 6）になり、2 つ目のダブが増えて塗れる所が増える
    let sparse = painted(&stroke(&wide, 0.125));
    let dense = painted(&stroke(&closer, 0.125));
    assert!(dense > sparse * 2, "{dense}・{sparse}");
}

#[test]
fn the_random_sources_draw_from_their_own_stream_with_a_fixed_seed() {
    let mut b = dualed(DualBrush {
        spacing: 0.8,
        angle_random: 1.0,
        roundness: 0.4,
        ..DualBrush::default()
    });
    b.influences.dual_size.random = InputUse::STRAIGHT;
    b.influences.dual_flow.random = InputUse::STRAIGHT;
    let a = stroke(&b, 1.0);
    assert_eq!(a, stroke(&b, 1.0), "同じ種なら同じバイト");
    let mut other = b.clone();
    other.seed = 10;
    assert_ne!(a, stroke(&other, 1.0), "種が違えば違う");
    // 主の乱数の列（ゆらぎ）は引かない: 主のゆらぎの無いブラシで、2 つ目の乱数の有無は主の形を変えない（通常は 2 つ目を使わない）
    let mut normal = b.clone();
    normal.dual.as_mut().unwrap().mode = DualBrushMode::Normal;
    let mut no_random = dualed(DualBrush {
        mode: DualBrushMode::Normal,
        ..DualBrush::default()
    });
    no_random.seed = b.seed;
    assert_eq!(stroke(&normal, 1.0), stroke(&no_random, 1.0));
}

#[test]
fn the_angle_adds_the_pen_tilt_direction() {
    let flat = dualed(DualBrush {
        roundness: 0.3,
        ..DualBrush::default()
    });
    let tilted = DVec2::new(0.0, 0.6);
    let (w, h) = extent(&dot(&flat, 1.0, tilted));
    assert!(w > h, "向きを足さなければ横長: {w}×{h}");
    let mut turned = flat.clone();
    turned.dual.as_mut().unwrap().angle_tilt = true;
    let (w, h) = extent(&dot(&turned, 1.0, tilted));
    assert!(h > w, "倒した向き（90°）を足すと縦長: {w}×{h}");
    // 線の方向: 縦の線では 90° 回る
    let mut line = flat.clone();
    line.dual.as_mut().unwrap().angle_line = true;
    let points: Vec<BrushSample> = (0..3)
        .map(|i| sample(CENTER, 20.0 + 44.0 * i as f64, 1.0, DVec2::ZERO))
        .collect();
    let plain = flat.clone();
    assert_ne!(draw(&line, &points).1, draw(&plain, &points).1);
}

#[test]
fn the_secondary_anti_aliasing_is_its_own_setting() {
    let soft = dualed(DualBrush {
        anti_alias: Some(AntiAlias::Strong),
        ..DualBrush::default()
    });
    let same = dualed(DualBrush {
        anti_alias: Some(AntiAlias::None),
        ..DualBrush::default()
    });
    let plain = dualed(DualBrush::default());
    let base = dot(&plain, 1.0, DVec2::ZERO);
    assert_eq!(dot(&same, 1.0, DVec2::ZERO), base, "主と同じ段は今のまま");
    assert_ne!(dot(&soft, 1.0, DVec2::ZERO), base);
}

#[test]
fn undo_and_redo_of_a_stroke_with_the_added_items_restore_the_exact_bytes() {
    let mut b = dualed(DualBrush {
        mode: DualBrushMode::Screen,
        link_size: true,
        flow: 0.7,
        angle_line: true,
        angle_random: 0.3,
        ..DualBrush::default()
    });
    b.influences.dual_spacing.random = InputUse::STRAIGHT;
    let points: Vec<BrushSample> = (0..8)
        .map(|i| sample(24.0 + 10.0 * i as f64, CENTER, 0.8, DVec2::ZERO))
        .collect();
    let (mut d, drawn) = draw(&b, &points);
    assert!(painted(&drawn) > 0, "テストの前提: 描けている");
    assert!(d.undo().unwrap());
    assert_eq!(
        painted(&d.composite(d.bounds()).unwrap()),
        0,
        "Undo で描く前"
    );
    assert!(d.redo().unwrap());
    assert_eq!(d.composite(d.bounds()).unwrap(), drawn, "Redo で描いた後");
}

#[test]
fn a_stroke_with_out_of_range_added_items_is_refused() {
    for (bad, reason) in [
        (
            DualBrush {
                flow: 1.5,
                ..DualBrush::default()
            },
            Reason::DualBrushFlow,
        ),
        (
            DualBrush {
                flow: -0.1,
                ..DualBrush::default()
            },
            Reason::DualBrushFlow,
        ),
        (
            DualBrush {
                flow: f64::NAN,
                ..DualBrush::default()
            },
            Reason::DualBrush,
        ),
        (
            DualBrush {
                angle_random: 2.0,
                ..DualBrush::default()
            },
            Reason::DualBrushAngleRandom,
        ),
        (
            DualBrush {
                angle_random: f64::INFINITY,
                ..DualBrush::default()
            },
            Reason::DualBrush,
        ),
    ] {
        let b = dualed(bad.clone());
        let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
        let l = d.add_layer("L").unwrap();
        let Err(err) = d.begin_brush_stroke(l, &b) else {
            panic!("断る: {bad:?}");
        };
        assert!(
            matches!(&err, CoreError::InvalidArgument(r) if *r == reason),
            "{err:?}"
        );
    }
    // 影響元の範囲の外（最小値）は、作るときに断る
    assert!(InputResponse::new(-0.5, vec![]).is_err());
}

/// 筆圧 0 で間隔の係数が下限（0.01）まで縮み、区間の長さは設定の間隔の上限の半分（前の確かめを通る）の線。
fn shrinking_spacing_stroke() -> (Brush, Vec<BrushSample>) {
    // 2 つ目の間隔は max(0.01, 直径 1 × 0.01) = 0.01（設定で取れる最小）。区間は 5000（= 上限 100 万 × 0.01 の半分）で、設定の間隔の
    // ままなら 50 万個
    let mut b = dualed(DualBrush {
        spacing: 0.01,
        ..DualBrush::default()
    });
    b.dual.as_mut().unwrap().radius = 0.5;
    b.influences.dual_spacing.pressure = pressure_use();
    let points = vec![
        sample(0.0, CENTER, 0.0, DVec2::ZERO),
        sample(5000.0, CENTER, 0.0, DVec2::ZERO),
    ];
    (b, points)
}

#[test]
fn a_spacing_source_that_shrinks_the_step_past_the_dab_limit_is_refused_while_placing() {
    let (b, points) = shrinking_spacing_stroke();
    let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let before = d.composite(d.bounds()).unwrap();
    let undo = d.undo_count();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    let mut refused = None;
    for &p in &points {
        if let Err(e) = s.add_sample(&mut d, p) {
            refused = Some(e);
            break;
        }
    }
    assert!(
        matches!(
            refused,
            Some(CoreError::InvalidArgument(
                Reason::TooManyDualBrushDabsPerSegment
            ))
        ),
        "{refused:?}"
    );
    // 断られたストロークは取り消され、途中の主・2 つ目のダブは残らない
    assert_eq!(d.composite(d.bounds()).unwrap(), before);
    assert_eq!(d.undo_count(), undo);
    assert!(s.add_sample(&mut d, points[0]).is_err(), "取り消し済み");
    // 次のストロークは普通に描け、新しい文書に描いたのと同じ
    let plain = dualed(DualBrush::default());
    let mut s = d.begin_brush_stroke(l, &plain).unwrap();
    for i in 0..9 {
        s.add_sample(
            &mut d,
            sample(16.0 + 12.0 * i as f64, CENTER, 1.0, DVec2::ZERO),
        )
        .unwrap();
    }
    d.end_stroke(s).unwrap();
    assert_eq!(
        d.composite(d.bounds()).unwrap(),
        stroke(&plain, 1.0),
        "断りの後の次のストローク"
    );
}

#[test]
fn the_same_long_segment_with_a_spacing_that_stays_put_is_accepted() {
    // 断りが並べながらの確かめによることの対照: 筆圧 1 なら間隔の係数は 1 のままで、同じ区間は 50 万個で上限に収まる
    let (b, _) = shrinking_spacing_stroke();
    let points = [
        sample(0.0, CENTER, 1.0, DVec2::ZERO),
        sample(5000.0, CENTER, 1.0, DVec2::ZERO),
    ];
    let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for &p in &points {
        s.add_sample(&mut d, p).unwrap();
    }
    d.end_stroke(s).unwrap();
}

/// 筆圧 0・「最低 1 画素」・サイズの連動の線（真ん中の画素の中心に 1 点）。主は 1 画素の半径 0.5 が残る。
fn floored_linked(mode: DualBrushMode, link: bool) -> Brush {
    let mut b = dualed(DualBrush {
        mode,
        link_size: link,
        ..DualBrush::default()
    });
    b.influences.size.pressure = pressure_use();
    b.controls.min_one_pixel = true;
    b
}

#[test]
fn a_floored_main_dab_keeps_its_one_pixel_line_under_a_linked_multiply() {
    let (x, y) = (CENTER + 0.5, CENTER + 0.5);
    let at = |b: &Brush, pressure: f64| draw(b, &[sample(x, y, pressure, DVec2::ZERO)]).1;
    let centre = |px: &[u8]| px[((CENTER as usize) * SIDE as usize + CENTER as usize) * 4 + 3];
    for mode in [DualBrushMode::Multiply, DualBrushMode::LinearHeight] {
        // 連動した 2 つ目は半径 6 × (0.5 / 20) = 0.15 まで細るが、0 にはならない: 主の 1 画素が残る
        let linked = at(&floored_linked(mode, true), 0.0);
        assert_eq!(painted(&linked), 1, "{mode:?}");
        // 連動しなければ 2 つ目は半径 6 のまま、主の 1 画素も同じように残る
        assert_eq!(
            painted(&at(&floored_linked(mode, false), 0.0)),
            1,
            "{mode:?}"
        );
        assert!(centre(&linked) > 0, "{mode:?}");
    }
    // 下限が効かない太さ（筆圧 0.5 では比 0.5 > 0.5 / 20）と、最低 1 画素なしは、持ち上げる前と同じ式のまま
    let on = floored_linked(DualBrushMode::Multiply, true);
    let mut off = on.clone();
    off.controls.min_one_pixel = false;
    assert_eq!(at(&on, 0.5), at(&off, 0.5));
    assert_eq!(
        painted(&at(&off, 0.0)),
        0,
        "最低 1 画素なしは筆圧 0 で何も描かない"
    );
}

#[test]
fn a_secondary_with_a_vanishing_radius_neither_panics_nor_poisons_the_pixels() {
    // 2 つ目のサイズの影響元（筆圧）で、2 つ目の半径が 0 より大きく 0.5 未満（6e-7・6e-9）になる。合わせは加算なので、2 つ目の覆いが
    // NaN になれば主の画素まで消える。筆圧 0（2 つ目の半径 0 で置かない）と同じバイトなら、何も混ざっていない
    for tip in [None, yolu_core::builtin_tip("dots")] {
        for anti_alias in [None, Some(AntiAlias::Weak), Some(AntiAlias::Strong)] {
            // 主は筆圧に応えない形（2 つ目だけが小さい）と、主が最低 1 画素の下限で残り連動で 2 つ目も縮む形
            for floored in [false, true] {
                let mut b = dualed(DualBrush {
                    mode: DualBrushMode::Add,
                    tip: tip.clone(),
                    anti_alias,
                    link_size: floored,
                    scatter: 0.5,
                    roundness: 0.3,
                    angle: 40.0,
                    count: 2,
                    ..DualBrush::default()
                });
                b.influences.dual_size.pressure = pressure_use();
                if floored {
                    b.influences.size.pressure = pressure_use();
                    b.controls.min_one_pixel = true;
                }
                let line = |pressure: f64| -> Vec<u8> {
                    let points: Vec<BrushSample> = (0..5)
                        .map(|i| {
                            sample(30.5 + 15.0 * i as f64, CENTER + 0.5, pressure, DVec2::ZERO)
                        })
                        .collect();
                    draw(&b, &points).1
                };
                let none = line(0.0);
                assert!(painted(&none) > 0, "テストの前提: 主は描ける");
                for pressure in [1e-7, 1e-9] {
                    assert_eq!(
                        line(pressure),
                        none,
                        "{tip:?}・{anti_alias:?}・{floored}・{pressure}"
                    );
                }
            }
        }
    }
}
