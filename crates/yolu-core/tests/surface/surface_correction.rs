//! 補正の拡張（3D の面）: 後補正は離した後の次の塗りで、文書のストロークの画素を戻して整えた画面の点の列で描き直す（1 回の Undo。描いて
//! いる途中の取り消しは何も残さない）。はらいと手ぶれ補正の速度による調整は 2D と同じ式（長さと速さは画面の点）。
use yolu_core::geometry::{SurfaceInput, SurfaceStroke, SurfaceStrokeError, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, Channel, CoreError, Document, LayerId, PostCorrection, PressureSwitches,
    ReferenceLines, Rgba8, StabilizerSpeed,
};

use crate::anti_alias::{bytes, document, front, quad, screen, H, W};

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

fn input(x: f64, y: f64, pressure: f32, time: f64) -> SurfaceInput {
    SurfaceInput {
        at: screen(x, y),
        pressure,
        tilt: DVec2::ZERO,
        rotation: 0.0,
        time,
    }
}

/// 世界の y 32 の横の線に、`amp` の高さの揺れ（2 点ごとに上下）を足した入力（1 点おき、200 点 / 秒）。
fn wobbly(amp: f64) -> Vec<SurfaceInput> {
    (0..=70)
        .map(|i| {
            let y = 32.0 + if (i / 2) % 2 == 0 { amp } else { -amp };
            input(12.0 + i as f64, y, 1.0, i as f64 / 200.0)
        })
        .collect()
}

/// 入力を足して、離す前の画素（アルファ）・離して塗り終えた画素・確定した文書を返す。
fn draw(b: &Brush, inputs: &[SurfaceInput]) -> (Vec<u8>, Vec<u8>, Document, LayerId) {
    let (mut d, l) = document(1);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, b).unwrap();
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(0),
        inputs[0],
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for i in &inputs[1..] {
        s.add_input(&mut d, &mut stroke, *i).unwrap();
        s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    }
    let live = alpha(&d, l);
    s.end_input().unwrap();
    // 描き直しを待つ間は、塗るものが残っている
    if b.assist.post.active() {
        assert!(s.queued() > 0);
    }
    while s.queued() > 0 {
        s.paint_queued(&mut d, &mut stroke, 7).unwrap();
    }
    let done = alpha(&d, l);
    d.end_stroke(stroke).unwrap();
    (live, done, d, l)
}

fn alpha(d: &Document, l: LayerId) -> Vec<u8> {
    bytes(d, l)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[3])
        .collect()
}

/// 列ごとの塗った画素の重心の、となりの列との差の絶対値の平均。
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

fn thickness(a: &[u8], x: usize) -> usize {
    (0..H as usize)
        .filter(|&y| a[y * W as usize + x] > 0)
        .count()
}

#[test]
fn a_3d_post_corrected_stroke_is_redrawn_straighter_after_release() {
    // 揺れの上下の幅 0.8 点は、許し 1 点（強さ 10・表示倍率による調整）より小さい
    let inputs = wobbly(0.4);
    let (_, raw, _, _) = draw(&brush(2.0, false), &inputs);
    let mut b = brush(2.0, false);
    b.assist.post = PostCorrection {
        on: true,
        strength: 10.0,
        by_speed: false,
        by_view_scale: true,
        ..PostCorrection::default()
    };
    let (live, done, mut d, l) = draw(&b, &inputs);
    assert_eq!(live, raw, "描いている間は補正の無い線");
    // 端と端を結ぶ揺れの無い入力の線と同じほどまっすぐ（面の塗りは画素の段が出るので、0 にはならない）
    let (first, last) = (inputs[0], inputs[inputs.len() - 1]);
    let straight: Vec<SurfaceInput> = (0..=70)
        .map(|i| {
            let t = i as f32 / 70.0;
            SurfaceInput {
                at: first.at + (last.at - first.at) * t,
                ..inputs[i]
            }
        })
        .collect();
    let (_, line, _, _) = draw(&brush(2.0, false), &straight);
    let (w_done, w_raw, w_line) = (
        wobble(&done, 18, 76),
        wobble(&raw, 18, 76),
        wobble(&line, 18, 76),
    );
    assert!(
        w_done <= w_line + 0.05 && w_raw > w_line + 0.1,
        "{w_done} {w_raw} {w_line}"
    );
    assert_eq!(d.undo_count(), 1, "描き直しは 1 回の Undo");
    assert!(d.undo().unwrap());
    assert!(alpha(&d, l).iter().all(|a| *a == 0));
    // 同じ入力なら同じ画素
    assert_eq!(draw(&b, &inputs).1, done);
}

#[test]
fn a_3d_post_corrected_stroke_cancelled_before_the_redraw_leaves_nothing() {
    let mut b = brush(2.0, false);
    b.assist.post = PostCorrection {
        on: true,
        ..PostCorrection::default()
    };
    let inputs = wobbly(0.6);
    let (mut d, l) = document(1);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(0),
        inputs[0],
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for i in &inputs[1..] {
        s.add_input(&mut d, &mut stroke, *i).unwrap();
    }
    s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    s.end_input().unwrap();
    // 離した後、描き直しの途中で取り消す（Escape・フォーカス喪失）
    s.paint_queued(&mut d, &mut stroke, 3).unwrap();
    d.cancel_stroke(stroke);
    assert!(alpha(&d, l).iter().all(|a| *a == 0));
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn a_3d_post_corrected_stroke_counts_the_remembered_inputs_in_the_budget() {
    let mut b = brush(2.0, false);
    b.assist.post = PostCorrection {
        on: true,
        ..PostCorrection::default()
    };
    let inputs = wobbly(0.6);
    let (mut d, l) = document(1);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(0),
        inputs[0],
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    // 覚えた入力だけで、投影の塗りに使ってよいバイトを超える大きさ（入力 1 つは 40 バイト前後）
    s.set_projection_memory(Some(400));
    let refused = inputs[1..]
        .iter()
        .find_map(|i| s.add_input(&mut d, &mut stroke, *i).err());
    assert_eq!(
        refused,
        Some(SurfaceStrokeError::Core(CoreError::StrokeBudgetExceeded))
    );
    // 断られたら、呼ぶ側がストロークを取り消す: 何も残らない
    d.cancel_stroke(stroke);
    assert!(alpha(&d, l).iter().all(|a| *a == 0));
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn a_3d_sweep_keeps_the_line_thick_while_pressure_is_released() {
    let release: Vec<SurfaceInput> = (0..=70)
        .map(|i| {
            let p = if i <= 30 {
                1.0
            } else {
                1.0 - (i - 30) as f32 / 40.0
            };
            input(12.0 + i as f64, 32.0, p, i as f64 / 240.0)
        })
        .collect();
    let mut b = brush(6.0, true);
    let (_, base, _, _) = draw(&b, &release);
    b.assist.sweep = 1.0;
    let (_, swept, _, _) = draw(&b, &release);
    assert_eq!(thickness(&swept, 30), thickness(&base, 30));
    assert!(
        thickness(&swept, 76) > thickness(&base, 76) + 3,
        "{} > {}",
        thickness(&swept, 76),
        thickness(&base, 76)
    );
}

#[test]
fn a_3d_stabilizer_that_corrects_only_slow_strokes_follows_a_fast_corner() {
    let corner = |speed: f64| -> Vec<SurfaceInput> {
        let mut points: Vec<(f64, f64)> = (0..=30).map(|i| (16.0 + i as f64, 16.0)).collect();
        points.extend((1..=36).map(|i| (46.0, 16.0 + i as f64)));
        points
            .iter()
            .enumerate()
            .map(|(i, p)| input(p.0, p.1, 1.0, i as f64 / speed))
            .collect()
    };
    let cut = |a: &[u8]| {
        let mut best = f64::INFINITY;
        for y in 0..H as usize {
            for x in 0..W as usize {
                if a[y * W as usize + x] > 128 {
                    best = best.min((x as f64 + 0.5 - 46.0).hypot(y as f64 + 0.5 - 16.0));
                }
            }
        }
        best
    };
    let mut b = brush(2.0, false);
    b.assist.stabilizer = 12.0;
    b.assist.stabilizer_speed = Some(StabilizerSpeed::SlowOnly);
    let (_, slow, _, _) = draw(&b, &corner(30.0));
    let (_, fast, _, _) = draw(&b, &corner(30000.0));
    assert!(cut(&fast) < 1.5, "速いと補正しない: {}", cut(&fast));
    assert!(
        cut(&slow) > cut(&fast) + 2.0,
        "{} > {}",
        cut(&slow),
        cut(&fast)
    );
}

#[test]
fn a_3d_post_corrected_redraw_keeps_the_frozen_reference() {
    // 縦の線（列 48〜51）のある参照のレイヤーを印にして、線を横切る揺れた線を描く。離した後の描き直しも、線の上を塗らない
    let mut b = brush(2.0, false);
    b.reference_lines = ReferenceLines {
        enabled: true,
        ..ReferenceLines::default()
    };
    b.assist.post = PostCorrection {
        on: true,
        strength: 10.0,
        by_speed: false,
        by_view_scale: true,
        ..PostCorrection::default()
    };
    let inputs = wobbly(0.4);
    let run = |reference: bool| {
        let (mut d, l) = document(1);
        let r = d.add_layer("参照").unwrap();
        for y in 0..H {
            for x in 48..52 {
                d.set_pixel(r, x, y, Rgba8::new(0, 0, 0, 255)).unwrap();
            }
        }
        d.clear_history().unwrap();
        let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
        if reference {
            let marked = std::collections::HashSet::from([r]);
            assert!(stroke
                .use_reference_layers(&mut d, &marked, Channel::Color)
                .unwrap());
        }
        let mut s = SurfaceStroke::begin_input(
            &mut d,
            &mut stroke,
            quad(),
            front(),
            &b.base,
            Some(0),
            inputs[0],
            SurfaceStrokeOptions::default(),
        )
        .unwrap();
        for i in &inputs[1..] {
            s.add_input(&mut d, &mut stroke, *i).unwrap();
            s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
        }
        let live = alpha(&d, l);
        s.end_input().unwrap();
        while s.queued() > 0 {
            s.paint_queued(&mut d, &mut stroke, 7).unwrap();
        }
        let done = alpha(&d, l);
        d.end_stroke(stroke).unwrap();
        (live, done)
    };
    let (_, without) = run(false);
    let (live, done) = run(true);
    let on_line = |a: &[u8]| {
        (0..H as usize)
            .flat_map(|y| (48..52).map(move |x| (x, y)))
            .filter(|&(x, y)| a[y * W as usize + x] > 0)
            .count()
    };
    assert!(on_line(&without) > 0, "参照が無ければ線の上も塗る");
    assert_eq!(on_line(&live), 0, "描いている間");
    assert_ne!(live, done, "描き直した");
    assert_eq!(on_line(&done), 0, "描き直しの後も線の上を塗らない");
    assert!(
        thickness(&done, 40) > 0 && thickness(&done, 60) > 0,
        "線の両側は塗る"
    );
}

#[test]
fn a_3d_redraw_refused_after_the_restart_leaves_nothing() {
    // 離した後、描き直しで文書のストロークの画素を戻した後に、塗りが断られる（投影の塗りに使ってよいバイトが足りない）。
    // 呼ぶ側がストロークを取り消すと、元の線の画素も描き直しの途中の画素も残らず、Undo の段も積まれない
    let mut b = brush(2.0, false);
    b.assist.post = PostCorrection {
        on: true,
        strength: 10.0,
        by_speed: false,
        by_view_scale: true,
        ..PostCorrection::default()
    };
    let inputs = wobbly(0.4);
    let (mut d, l) = document(1);
    d.clear_history().unwrap();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        quad(),
        front(),
        &b.base,
        Some(0),
        inputs[0],
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for i in &inputs[1..] {
        s.add_input(&mut d, &mut stroke, *i).unwrap();
        s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    }
    let live = alpha(&d, l);
    assert!(live.iter().any(|a| *a > 0), "描いている間は線がある");
    s.end_input().unwrap();
    s.set_projection_memory(Some(1));
    let refused = s.paint_queued(&mut d, &mut stroke, usize::MAX);
    assert!(refused.is_err(), "{refused:?}");
    assert_ne!(alpha(&d, l), live, "断られたのは、画素を戻した後");
    d.cancel_stroke(stroke);
    assert!(alpha(&d, l).iter().all(|a| *a == 0));
    assert_eq!(d.undo_count(), 0);
}
