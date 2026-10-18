//! 水彩境界の 2D のストローク: 透明度影響・明度影響が 0 なら水彩境界の無い線と同じバイト（ダブの式と同じ合成で計算し直すこと）、
//! 幅・透明度影響・明度影響で縁の画素が決まった向きに動くこと、ドラッグ後に処理の入切（描いている間と終えた後）とぼかし幅、
//! 入力の区切り方によらないこと、2D の対称、取り消し・Undo・Redo、選択範囲・透明部分のロック、ストロークの予算、範囲の断り、
//! スレッドの数によらないこと。距離の計算と帯の形・道（SIMD）によらないことは core の中のテスト（`brush::water_edge`）が見る。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;

use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, BrushStencil, CanvasSymmetry, ColorMix, CoreError, Document,
    ImageColorSpace, LayerId, LayerLocks, MixMode, PressureSwitches, Reason, Rgba8, SelectionMask,
    StencilImage, StencilMapping, StencilMode, StencilTiling, SymmetryMode, WaterEdge,
};

const W: u32 = 160;
const H: u32 = 96;
const Y: f64 = 48.0;

fn base(hardness: f64, opacity: f64) -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 12.0,
            hardness,
            spacing: 0.1,
            opacity,
            flow: 1.0,
            color: Rgba8::new(48, 112, 192, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    )
}

fn with_edge(mut b: Brush, e: WaterEdge) -> Brush {
    b.water_edge = Some(e);
    b
}

fn edge(width: f64, opacity: f64, darkness: f64) -> WaterEdge {
    WaterEdge {
        width,
        opacity,
        darkness,
        after_stroke: false,
        blur: 0.0,
    }
}

fn sample(x: f64, y: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, 0.0, DVec2::ZERO).unwrap()
}

/// 横の線（x 24〜136）を、点 `n` 個に分けた入力で。
fn line(n: usize) -> Vec<BrushSample> {
    (0..n)
        .map(|i| sample(24.0 + 112.0 * i as f64 / (n - 1) as f64, Y))
        .collect()
}

fn doc() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn stroke(d: &mut Document, l: LayerId, b: &Brush, samples: &[BrushSample]) {
    let mut s = d.begin_brush_stroke(l, b).unwrap();
    for &p in samples {
        s.add_sample(d, p).unwrap();
    }
    d.end_stroke(s).unwrap();
}

fn draw(b: &Brush, samples: &[BrushSample]) -> Vec<u8> {
    let (mut d, l) = doc();
    stroke(&mut d, l, b, samples);
    d.composite(d.bounds()).unwrap()
}

/// 画素 (x, y)（左下原点）。
fn at(p: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [p[i], p[i + 1], p[i + 2], p[i + 3]]
}

/// 列 x の、色のある画素のうち、一番上の画素の y。
fn top(p: &[u8], x: u32) -> u32 {
    (0..H)
        .rev()
        .find(|&y| at(p, x, y)[3] > 0)
        .expect("塗った列")
}

/// 下地に別の色のストロークを置く（色の混ぜ・透明部分のロックの下地）。
fn ground(d: &mut Document, l: LayerId) {
    let mut g = base(1.0, 1.0);
    g.base.color = Rgba8::new(200, 60, 30, 255);
    g.base.radius = 9.0;
    stroke(d, l, &g, &[sample(40.0, 40.0), sample(120.0, 58.0)]);
    d.clear_history().unwrap();
}

#[test]
fn zero_powers_paint_the_same_bytes_as_no_edge() {
    let quiet = edge(3.0, 0.0, 0.0);
    let mut tipped = base(0.6, 0.8);
    tipped.color.hue = 0.3;
    tipped.color.per_tip = true;
    tipped.seed = 5;
    let mut stenciled = base(0.6, 0.9);
    stenciled.stencil = Some(colour_stencil());
    for (label, b, ground_first, select, lock) in [
        ("硬い丸", base(0.9, 1.0), false, false, false),
        ("柔らかい丸・不透明度", base(0.2, 0.6), false, false, false),
        ("ダブごとの色", tipped, false, false, false),
        ("混ぜる", mixing(MixMode::Mix), true, false, false),
        ("伸ばす", mixing(MixMode::Smear), true, false, false),
        ("重ねる", mixing(MixMode::Layer), true, false, false),
        ("色のステンシル", stenciled, false, false, false),
        ("ぼかした選択範囲", base(0.5, 1.0), false, true, false),
        ("透明部分のロック", base(0.5, 0.7), true, false, true),
    ] {
        for after_stroke in [false, true] {
            let run = |b: &Brush| {
                let (mut d, l) = doc();
                if ground_first {
                    ground(&mut d, l);
                }
                if select {
                    let m = SelectionMask::ellipse(&d, 70.0, 50.0, 40.0, 20.0)
                        .unwrap()
                        .feather(6.0, false, u64::MAX)
                        .unwrap();
                    d.set_selection(Some(m)).unwrap();
                }
                if lock {
                    d.set_layer_locks(l, LayerLocks::TRANSPARENCY).unwrap();
                }
                stroke(&mut d, l, b, &line(12));
                d.composite(d.bounds()).unwrap()
            };
            let plain = run(&b);
            let edged = run(&with_edge(
                b.clone(),
                WaterEdge {
                    after_stroke,
                    blur: if after_stroke { 2.5 } else { 0.0 },
                    ..quiet
                },
            ));
            assert!(plain.chunks_exact(4).any(|p| p[3] > 0), "{label}");
            assert_eq!(edged, plain, "{label} 後処理 {after_stroke}");
        }
    }
    // 画素ごとに塗る入口（`apply_pixel`。メッシュのダブ）も同じ
    let pixels = |b: &Brush| {
        let (mut d, l) = doc();
        let mut s = d.begin_brush_stroke(l, b).unwrap();
        for y in 30..66i64 {
            for x in 20..140i64 {
                let c = 1.0 - ((y - 48) as f64 / 18.0).abs();
                s.apply_pixel(&mut d, x, y, c.clamp(0.0, 1.0) * 0.9, 1.0)
                    .unwrap();
            }
        }
        d.end_stroke(s).unwrap();
        d.composite(d.bounds()).unwrap()
    };
    let b = base(0.5, 0.8);
    for after_stroke in [false, true] {
        let e = WaterEdge {
            after_stroke,
            ..quiet
        };
        assert_eq!(
            pixels(&with_edge(b.clone(), e)),
            pixels(&b),
            "画素ごと 後処理 {after_stroke}"
        );
    }
}

/// 色の混ぜのブラシ（`mode` の方式。下地の色を拾う値）。
fn mixing(mode: MixMode) -> Brush {
    let mut b = base(0.7, 1.0);
    b.mix = ColorMix {
        mode,
        paint: 0.5,
        density: 0.7,
        stretch: 0.6,
        ..ColorMix::default()
    };
    b
}

/// 色のモードのステンシル（キャンバスに重なる、場所で色の変わる画像）。
fn colour_stencil() -> Arc<BrushStencil> {
    let mut bytes = vec![0u8; (W * H * 4) as usize];
    for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % W as usize, i / W as usize);
        p.copy_from_slice(&[(x * 3 % 256) as u8, (y * 5 % 256) as u8, 90, 255]);
    }
    let img = StencilImage::new(
        W as usize,
        H as usize,
        bytes,
        ImageColorSpace::Srgb,
        StencilImage::DEFAULT_MIP_BUDGET_BYTES,
    )
    .unwrap();
    Arc::new(BrushStencil::new(
        Arc::new(img),
        StencilMode::Color,
        StencilTiling::None,
        false,
        Some(StencilMapping::translation(0.0, 0.0)),
        &[],
    ))
}

#[test]
fn mixes_that_read_the_current_surface_do_not_depend_on_the_split() {
    // 伸ばす・重ねるは下地を今の面から読む。描いている間に付けた縁は下地に入れないので、入力の区切り方によらず同じ線で、
    // ぼかし幅 0 のドラッグ後に処理とも同じバイト
    for mode in [MixMode::Smear, MixMode::Layer] {
        let live = with_edge(mixing(mode), edge(4.0, 0.4, 0.8));
        let after = with_edge(
            mixing(mode),
            WaterEdge {
                after_stroke: true,
                ..edge(4.0, 0.4, 0.8)
            },
        );
        let run = |b: &Brush, n: usize| {
            let (mut d, l) = doc();
            ground(&mut d, l);
            stroke(&mut d, l, b, &line(n));
            d.composite(d.bounds()).unwrap()
        };
        let whole = run(&live, 2);
        assert_ne!(whole, run(&mixing(mode), 2), "{mode:?}: 縁が付く");
        for n in [3, 8, 29] {
            assert_eq!(run(&live, n), whole, "{mode:?}: {n} 点");
        }
        assert_eq!(run(&after, 29), whole, "{mode:?}: ドラッグ後に処理");
    }
}

#[test]
fn opacity_darkness_and_width_move_the_rim_the_set_way() {
    let soft = base(0.3, 0.6);
    let plain = draw(&soft, &line(12));
    let x = 80;
    let rim = top(&plain, x);
    let inside = (Y as u32) - 1; // 線の芯（縁から 10 画素ほど内）
                                 // 透明度影響: 縁の画素のアルファが上がっていく。芯は変わらない
    let mut last = at(&plain, x, rim)[3];
    for opacity in [0.02, 0.05, 1.0] {
        let p = draw(&with_edge(soft.clone(), edge(3.0, opacity, 0.0)), &line(12));
        let a = at(&p, x, rim)[3];
        assert!(a > last, "透明度影響 {opacity}: {a} > {last}");
        last = a;
        assert_eq!(at(&p, x, inside), at(&plain, x, inside), "芯は幅の外");
        assert_eq!(top(&p, x), rim, "縁の外へは塗らない");
    }
    assert_eq!(last, 255, "透明度影響 1 の一番外は不透明");
    // 明度影響: 縁の色が暗くなっていく（アルファは透明度影響だけで決まる）
    let lit = draw(&with_edge(soft.clone(), edge(3.0, 0.0, 0.0)), &line(12));
    let mut last = at(&lit, x, rim);
    for darkness in [0.3, 0.6, 1.0] {
        let p = draw(
            &with_edge(soft.clone(), edge(3.0, 0.0, darkness)),
            &line(12),
        );
        let c = at(&p, x, rim);
        assert!(
            c[0] < last[0] && c[1] < last[1] && c[2] < last[2],
            "明度影響 {darkness}"
        );
        assert_eq!(c[3], at(&plain, x, rim)[3]);
        last = c;
    }
    // 幅: 暗くなる画素の帯が内へ広がる
    let band = |width: f64| {
        let p = draw(&with_edge(soft.clone(), edge(width, 0.0, 1.0)), &line(12));
        (0..H)
            .filter(|&y| at(&p, x, y)[3] > 0 && at(&p, x, y) != at(&plain, x, y))
            .count()
    };
    let (thin, mid, wide) = (band(1.0), band(4.0), band(9.0));
    assert!(thin < mid && mid < wide, "{thin} {mid} {wide}");
    assert_eq!(thin, 2, "幅 1 は上下の一番外の画素だけ");
}

#[test]
fn after_stroke_paints_no_edge_while_drawing_and_the_same_edge_at_the_end() {
    let soft = base(0.4, 0.8);
    let live = with_edge(soft.clone(), edge(4.0, 0.4, 0.8));
    let after = with_edge(
        soft.clone(),
        WaterEdge {
            after_stroke: true,
            ..edge(4.0, 0.4, 0.8)
        },
    );
    let samples = line(12);
    // 描いている間: 切は縁が見え、入は縁なし（水彩境界の無いブラシと同じ）
    let midway = |b: &Brush| {
        let (mut d, l) = doc();
        let mut s = d.begin_brush_stroke(l, b).unwrap();
        for &p in &samples {
            s.add_sample(&mut d, p).unwrap();
        }
        let during = d.composite(d.bounds()).unwrap();
        d.end_stroke(s).unwrap();
        (during, d.composite(d.bounds()).unwrap())
    };
    let (plain_during, plain_end) = midway(&soft);
    let (live_during, live_end) = midway(&live);
    let (after_during, after_end) = midway(&after);
    assert_ne!(live_during, plain_during, "切は描いている間に縁");
    assert_eq!(after_during, plain_during, "入は描いている間は縁なし");
    assert_ne!(after_end, plain_end);
    // ぼかし幅 0 なら、終えた後は描いている間に付けた縁と同じバイト
    assert_eq!(after_end, live_end);
    // ぼかし幅: 帯が内へなだらかに広がり、一番外の画素も縁の無い線より濃い
    let blurred = draw(
        &with_edge(
            soft.clone(),
            WaterEdge {
                after_stroke: true,
                blur: 4.0,
                ..edge(4.0, 0.4, 0.8)
            },
        ),
        &samples,
    );
    let x = 80;
    let changed = |p: &[u8]| {
        (0..H)
            .filter(|&y| at(p, x, y)[3] > 0 && at(p, x, y) != at(&plain_end, x, y))
            .count()
    };
    assert!(changed(&blurred) > changed(&after_end));
    let rim = top(&plain_end, x);
    assert!(at(&blurred, x, rim)[3] > at(&plain_end, x, rim)[3]);
}

#[test]
fn the_result_does_not_depend_on_how_the_input_is_split() {
    let b = with_edge(base(0.5, 0.9), edge(5.0, 0.3, 0.7));
    let whole = draw(&b, &line(2));
    for n in [3, 8, 29] {
        assert_eq!(draw(&b, &line(n)), whole, "{n} 点");
    }
}

#[test]
fn canvas_symmetry_puts_the_edge_on_every_copy() {
    let mut b = with_edge(base(0.5, 0.9), edge(4.0, 0.3, 0.9));
    b.symmetry = CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(80.0, 48.0), 2).unwrap();
    let p = draw(&b, &[sample(20.0, 40.0), sample(60.0, 60.0)]);
    let mut painted = 0;
    for y in 0..H {
        for x in 0..W {
            assert_eq!(at(&p, x, y), at(&p, W - 1 - x, y), "({x}, {y})");
            painted += usize::from(at(&p, x, y)[3] > 0);
        }
    }
    assert!(painted > 0);
    // 写しの側の縁も暗い
    let plain = {
        let mut q = b.clone();
        q.water_edge = None;
        draw(&q, &[sample(20.0, 40.0), sample(60.0, 60.0)])
    };
    let right_dark = (0..H)
        .flat_map(|y| (W / 2..W).map(move |x| (x, y)))
        .filter(|&(x, y)| at(&p, x, y)[3] > 0 && at(&p, x, y)[0] < at(&plain, x, y)[0])
        .count();
    assert!(right_dark > 0);
}

#[test]
fn cancel_undo_and_redo_leave_one_step() {
    let b = with_edge(base(0.5, 0.9), edge(4.0, 0.5, 0.8));
    let (mut d, l) = doc();
    ground(&mut d, l);
    let before = d.composite(d.bounds()).unwrap();
    // 取り消し（Escape・フォーカスを失った）: 描いている間に付けた縁も残さない
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for &p in &line(6) {
        s.add_sample(&mut d, p).unwrap();
    }
    assert_ne!(d.composite(d.bounds()).unwrap(), before);
    assert!(d.cancel_active_stroke());
    assert_eq!(d.composite(d.bounds()).unwrap(), before);
    assert_eq!(d.undo_count(), 0);
    // ドラッグ後に処理の縁も、ストロークと一緒の 1 段
    for after_stroke in [false, true] {
        let b = with_edge(
            b.clone(),
            WaterEdge {
                after_stroke,
                ..edge(4.0, 0.5, 0.8)
            },
        );
        stroke(&mut d, l, &b, &line(6));
        let done = d.composite(d.bounds()).unwrap();
        assert_eq!(d.undo_count(), 1);
        assert!(d.undo().unwrap());
        assert_eq!(d.composite(d.bounds()).unwrap(), before);
        assert!(d.redo().unwrap());
        assert_eq!(d.composite(d.bounds()).unwrap(), done);
        assert!(d.undo().unwrap());
        d.clear_history().unwrap();
    }
}

#[test]
fn the_selection_and_the_transparency_lock_hold() {
    let b = with_edge(base(0.5, 0.9), edge(4.0, 1.0, 1.0));
    // 選択範囲の外は変わらない（縁は選択範囲の内側にできる）
    let (mut d, l) = doc();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 80, 96)))
        .unwrap();
    stroke(&mut d, l, &b, &line(8));
    let p = d.composite(d.bounds()).unwrap();
    for y in 0..H {
        for x in 80..W {
            assert_eq!(at(&p, x, y)[3], 0, "({x}, {y})");
        }
    }
    let x = 79;
    let column: Vec<[u8; 4]> = (0..H).map(|y| at(&p, x, y)).collect();
    assert!(column.iter().any(|c| c[3] > 0));
    // 透明部分のロック: アルファも透明の画素も変わらない
    let (mut d, l) = doc();
    ground(&mut d, l);
    let before = d.composite(d.bounds()).unwrap();
    d.set_layer_locks(l, LayerLocks::TRANSPARENCY).unwrap();
    stroke(&mut d, l, &b, &line(8));
    let after = d.composite(d.bounds()).unwrap();
    assert_ne!(after, before);
    for (q, r) in after.chunks_exact(4).zip(before.chunks_exact(4)) {
        assert_eq!(q[3], r[3]);
        if r[3] == 0 {
            assert_eq!(q, r);
        }
    }
}

#[test]
fn the_stroke_budget_also_counts_the_edge_pass() {
    let plain = base(0.5, 0.9);
    let b = with_edge(plain.clone(), edge(20.0, 0.5, 0.5));
    let fits = |brush: &Brush, budget: u64| -> Result<(), CoreError> {
        let (mut d, l) = doc();
        d.set_stroke_budget_bytes(budget).unwrap();
        let mut s = d.begin_brush_stroke(l, brush)?;
        for &p in &line(4) {
            s.add_sample(&mut d, p)?;
        }
        d.end_stroke(s).map(|_| ())
    };
    // 水彩境界の無い線が収まる一番小さい予算（2 分探し）
    let (mut lo, mut hi) = (1u64, 64 << 20);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if fits(&plain, mid).is_ok() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    assert!(fits(&plain, lo).is_ok());
    // 縁の計算の一時の領域も数える: 同じ予算では断り、何も残さない
    let (mut d, l) = doc();
    d.set_stroke_budget_bytes(lo).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    let mut refused = None;
    for &p in &line(4) {
        if let Err(e) = s.add_sample(&mut d, p) {
            refused = Some(e);
            break;
        }
    }
    let refused = match refused {
        Some(e) => e,
        None => d.end_stroke(s).unwrap_err(),
    };
    assert_eq!(refused, CoreError::StrokeBudgetExceeded);
    assert!(d.composite(d.bounds()).unwrap().iter().all(|&v| v == 0));
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn out_of_range_settings_are_refused_before_the_stroke() {
    let (mut d, l) = doc();
    for (bad, why) in [
        (edge(0.0, 0.2, 0.0), Reason::WaterEdgeRange),
        (edge(3.0, 2.0, 0.0), Reason::WaterEdgeRange),
        (edge(3.0, 0.2, f64::NAN), Reason::WaterEdge),
    ] {
        assert_eq!(
            d.begin_brush_stroke(l, &with_edge(base(0.5, 1.0), bad))
                .unwrap_err(),
            CoreError::InvalidArgument(why)
        );
    }
}

#[test]
fn eraser_and_effect_brushes_ignore_the_edge() {
    let mut eraser = base(0.5, 1.0);
    eraser.base.erase = true;
    let run = |b: &Brush| {
        let (mut d, l) = doc();
        ground(&mut d, l);
        stroke(&mut d, l, b, &line(6));
        d.composite(d.bounds()).unwrap()
    };
    assert_eq!(
        run(&with_edge(eraser.clone(), edge(4.0, 1.0, 1.0))),
        run(&eraser)
    );
    let mut blur = base(0.5, 1.0);
    blur.effect = yolu_core::BrushEffect::Blur { radius: 3 };
    assert_eq!(
        run(&with_edge(blur.clone(), edge(4.0, 1.0, 1.0))),
        run(&blur)
    );
}

#[test]
fn threads_do_not_change_the_bytes() {
    let wide = |after_stroke| {
        with_edge(
            base(0.4, 0.9),
            WaterEdge {
                after_stroke,
                blur: if after_stroke { 3.0 } else { 0.0 },
                ..edge(8.0, 0.4, 0.8)
            },
        )
    };
    let shape = [
        sample(10.0, 10.0),
        sample(150.0, 30.0),
        sample(20.0, 80.0),
        sample(140.0, 86.0),
    ];
    for after_stroke in [false, true] {
        let b = wide(after_stroke);
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| draw(&b, &shape))
        };
        let one = run(1);
        assert_eq!(run(4), one, "後処理 {after_stroke}");
        assert_eq!(run(3), one, "後処理 {after_stroke}");
    }
}
