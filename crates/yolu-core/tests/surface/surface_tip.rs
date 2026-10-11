//! 3D のストロークの筆先（画像の筆先・ゆらぎ・紙の質感・デュアルブラシ・色の変化・入り抜き・フェード）とダブの置き方が 2D のストロークと
//! 同じになること。カメラに正面を向けた平らな四角（UV が文書の全体、文書の 1 画素が画面の 1 点）に、同じブラシで同じ点の列を 2D と 3D に
//! 描いて、テクセルの値を比べる。
//!
//! 比べ方の前提: 3D のブラシの半径はモデルの単位から画面へ直すので（箱の対角線 × 半径 / 文書の幅）、3D のブラシの半径を「箱の対角線 / 幅」で
//! 割って、画面の半径を 2D の半径にそろえる。ダブは 2D も 3D も線の長さで間隔ごとに置く（前の区間の余りを持ち越す）ので、入力の点の間隔に
//! よらず同じ位置に同じ数のダブが並ぶ。残る差は、テクセルの中心と入力の点を画面へ写す f32 の丸め（1e-5 点ほど）と、丸い筆先の式の違い
//! （3D の丸は今までの式）による、覆いの縁の 1〜数段。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;

use yolu_core::geometry::{
    pick, CameraView, MirrorPlane, OrbitCamera, RadialSymmetry, SurfaceCloneSource, SurfaceEffect,
    SurfaceGeometry, SurfaceInput, SurfaceStroke, SurfaceStrokeOptions, SurfaceSymmetrySetup,
    SurfaceTriangle, SymmetryAxis, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{DVec2, Quat, Vec2, Vec3};
use yolu_core::{
    builtin_presets, builtin_tip, Brush, BrushEffect, BrushSample, BrushSettings, Channel,
    ColorDynamics, Document, DualBrush, DualBrushMode, Jitter, LayerId, PaperTexture, Rgba8,
    TextureMode, TipShape,
};

const W: u32 = 192;
const H: u32 = 128;
/// 四角のまわりの画面の余白（点）。
const MARGIN: f32 = 16.0;

/// 文書の全体を UV に持つ四角（世界の x 0..W・y 0..H、z = 0。法線は −Z で、−Z の側のカメラを向く）。世界の 1 単位が文書の 1 画素。
fn quad() -> Arc<SurfaceGeometry> {
    let (w, h) = (W as f32, H as f32);
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(x / w, y / h);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
        .with_slot(0, 0, 0)
    };
    Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((0.0, 0.0), (0.0, h), (w, 0.0)),
                tri((w, 0.0), (0.0, h), (w, h)),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

/// 四角を −Z の側から正面に見るカメラ。世界の 1 単位が画面の 1 点になる距離（縦の画角 30°）。文書の点 (x, y) は画面の
/// (x + 余白, 高さ + 余白 − y)。
fn front() -> CameraView {
    let (sw, sh) = (W as f32 + 2.0 * MARGIN, H as f32 + 2.0 * MARGIN);
    let distance = sh / (2.0 * (15.0f32).to_radians().tan());
    OrbitCamera {
        target: Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: W as f32,
        ..Default::default()
    }
    .view(sw, sh)
}

fn screen(x: f64, y: f64) -> Vec2 {
    Vec2::new(
        (x + MARGIN as f64) as f32,
        (H as f64 + MARGIN as f64 - y) as f32,
    )
}

fn document() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let l = d.add_layer("paint").unwrap();
    (d, l)
}

fn bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
}

/// 2D の点の列を、そのまま 2D に描く。
fn draw_2d(brush: &Brush, points: &[(f64, f64)]) -> Vec<u8> {
    let (mut d, l) = document();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for &(x, y) in points {
        s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    bytes(&d, l)
}

/// 3D のブラシ: 画面の半径を 2D の半径にそろえる。
fn brush_3d(g: &SurfaceGeometry, brush: &Brush) -> Brush {
    let k = W as f64 / g.brush_scale() as f64;
    let mut b = brush.clone();
    b.base.radius *= k;
    if let Some(d) = b.dual.as_mut() {
        d.radius *= k;
    }
    b
}

/// 同じ点の列を 3D の四角に描く。
fn draw_3d(brush: &Brush, points: &[(f64, f64)]) -> Vec<u8> {
    let g = quad();
    let b = brush_3d(&g, brush);
    let (mut d, l) = document();
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(0),
        screen(x0, y0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &(x, y) in &points[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// 直線の上の点の列: 始まり (x, y)、向き (ux, uy)（長さ 1）、区間の長さは間隔 gap の何倍か。
fn line(start: (f64, f64), (ux, uy): (f64, f64), gap: f64, steps: &[u32]) -> Vec<(f64, f64)> {
    let mut points = vec![start];
    let mut along = 0.0;
    for &n in steps {
        along += n as f64 * gap;
        points.push((start.0 + ux * along, start.1 + uy * along));
    }
    points
}

/// 差の数: 塗った画素（どちらかでアルファが 0 でない）、値が違う画素、チャンネルの差のいちばん大きい値。色はアルファを掛けた値
/// （プリマルチプライド、0〜255 に丸める）で比べる（覆いの縁のほぼ透明な画素の、見えない色の差を数えない）。
fn compare(a: &[u8], b: &[u8]) -> (usize, usize, u8) {
    let weighted = |p: &[u8]| -> [u8; 4] {
        let k = |c: u8| ((c as u32 * p[3] as u32 + 127) / 255) as u8;
        [k(p[0]), k(p[1]), k(p[2]), p[3]]
    };
    let (mut painted, mut differ, mut max) = (0, 0, 0u8);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if p[3] > 0 || q[3] > 0 {
            painted += 1;
        }
        let (p, q) = (weighted(p), weighted(q));
        let d = p.iter().zip(&q).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        if d > 0 {
            differ += 1;
            max = max.max(d);
        }
    }
    (painted, differ, max)
}

fn tip(name: &str) -> Arc<yolu_core::BrushTip> {
    builtin_tip(name).unwrap()
}

/// 試験のブラシ: 同梱の「かすれ筆」（画像の筆先・線の向き・不透明度のゆらぎ）と、ゆらぎ・散布・数・紙の質感・入り抜き・筆先の並びのブラシ、
/// 潰して回した丸とデュアルブラシとフェードのブラシ、色の変化（描画色/背景色・色相、描点ごと）のブラシ。半径と間隔は、間隔が 2 の冪の
/// 分数になるように選ぶ（2D の線の長さの足し算に丸めを入れない）。
fn brushes() -> Vec<(&'static str, Brush, f64)> {
    let mut out = Vec::new();
    let mut dry = builtin_presets()
        .into_iter()
        .find(|p| p.id == "dry-brush")
        .unwrap()
        .brush;
    dry.base.radius = 16.0;
    dry.base.spacing = 0.03125; // 間隔 1 画素
    dry.base.color = Rgba8::new(90, 50, 20, 255);
    out.push(("かすれ筆", dry, 1.0));

    let mut jitter = Brush::from(BrushSettings {
        radius: 12.0,
        spacing: 0.25, // 間隔 6
        flow: 0.7,
        color: Rgba8::new(30, 60, 160, 255),
        ..BrushSettings::default()
    });
    jitter.tip = TipShape {
        images: vec![tip("charcoal"), tip("noisy-disc"), tip("dots")],
        angle: 20.0,
        roundness: 0.8,
        ..TipShape::default()
    };
    jitter.jitter = Jitter {
        size: 0.4,
        angle: 0.5,
        roundness: 0.5,
        opacity: 0.3,
        flow: 0.3,
        scatter: 0.5,
        count: 2,
    };
    jitter.texture = Some(PaperTexture {
        scale: 2.0,
        ..PaperTexture::new(tip("grain"), 0.7)
    });
    jitter.assist.taper_in = 30.0;
    jitter.assist.taper_out = 24.0;
    jitter.seed = 11;
    out.push(("ゆらぎ・散布・質感・入り抜き", jitter, 6.0));

    let mut dual = Brush::from(BrushSettings {
        radius: 10.0,
        hardness: 0.5,
        spacing: 0.25, // 間隔 5
        color: Rgba8::new(200, 40, 90, 255),
        ..BrushSettings::default()
    });
    dual.tip.roundness = 0.45;
    dual.tip.angle = 35.0;
    dual.dual = Some(DualBrush {
        tip: Some(tip("dots")),
        radius: 6.0,
        spacing: 5.0 / 12.0, // 間隔 5（6 × 2 × 5/12）
        scatter: 0.3,
        count: 2,
        mode: DualBrushMode::Multiply,
        ..DualBrush::default()
    });
    dual.controls.fade_size = 60;
    dual.controls.fade_opacity = 80;
    dual.texture = Some(PaperTexture {
        mode: TextureMode::Overlay,
        ..PaperTexture::new(tip("grain"), 0.5)
    });
    dual.seed = 5;
    out.push(("潰した丸・デュアル・フェード", dual, 5.0));

    let mut color = Brush::from(BrushSettings {
        radius: 9.0,
        hardness: 0.7,
        spacing: 0.25, // 間隔 4.5
        color: Rgba8::new(240, 200, 30, 255),
        ..BrushSettings::default()
    });
    color.tip.image = Some(tip("rounded-square"));
    color.tip.follow_direction = true;
    color.color = ColorDynamics {
        secondary: Rgba8::new(20, 30, 220, 255),
        foreground_background: 0.6,
        hue: 0.3,
        brightness: 0.2,
        per_tip: true,
        ..ColorDynamics::default()
    };
    color.seed = 3;
    out.push(("色の変化", color, 4.5));
    out
}

/// 入力の点の列の 3 通り: 区間の長さが間隔の倍数の直線、間隔より短い区間（間隔の 0.6 倍）の直線、長さ（間隔の 0.2〜3.7 倍）と向き
/// （±50°）がばらばらな折れ線。どれも文書の (40, 30) から 100 画素ほど。
fn inputs(gap: f64) -> Vec<(&'static str, Vec<(f64, f64)>)> {
    let multiples = line(
        (40.0, 30.0),
        (0.8, 0.6),
        gap,
        &[12, 7, 20, 9].map(|n| (n as f64 * 2.0 / gap).round().max(1.0) as u32),
    );
    let mut short = vec![(40.0, 30.0)];
    while short.len() < 400 && (short.len() as f64) * 0.6 * gap < 96.0 {
        let n = short.len() as f64;
        short.push((40.0 + 0.8 * 0.6 * gap * n, 30.0 + 0.6 * 0.6 * gap * n));
    }
    // 決まった種の乱数（線形合同法）で、区間の長さと向きを散らす
    let mut seed = 0x2545_f491u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut ragged = vec![(40.0, 30.0)];
    let mut along = 0.0;
    while along < 96.0 {
        let length = gap * (0.2 + 3.5 * next());
        let angle = 0.6435 + (next() * 2.0 - 1.0) * 0.87;
        let (x, y) = *ragged.last().unwrap();
        ragged.push((x + length * angle.cos(), y + length * angle.sin()));
        along += length;
    }
    vec![
        ("倍数の区間", multiples),
        ("短い区間", short),
        ("ばらばらの区間", ragged),
    ]
}

/// 同じブラシ・同じ点の列なら、3D の四角に描いたテクセルは 2D の画素とほぼ同じ（差はテクセルの中心を画面へ写す丸めと丸い筆先の式の
/// 違いによる縁の数段まで）。入力の点の間隔によらない（3D も線の長さで間隔ごとにダブを置く）。直す前の 3D（筆先の画像・ゆらぎ・質感・
/// デュアルを使わない丸い筆先）では大きく違う。
#[test]
fn a_flat_quad_facing_the_camera_paints_the_same_tip_as_the_2d_canvas() {
    for (name, brush, gap) in brushes() {
        for (shape, points) in inputs(gap) {
            let a = draw_2d(&brush, &points);
            let b = draw_3d(&brush, &points);
            let (painted, differ, max) = compare(&a, &b);
            println!("{name}・{shape}: 塗った画素 {painted}・違う画素 {differ}・最大の差 {max}");
            assert!(
                painted > 300,
                "{name}・{shape}: 試験の前提: 線を描いた ({painted})"
            );
            assert!(max <= 4, "{name}・{shape}: 差の最大 {max}");
            assert!(
                differ * 100 <= painted,
                "{name}・{shape}: 違う画素は塗った画素の 1 % まで ({differ}/{painted})"
            );
        }
    }
}

/// フェード（描点の数で薄く・小さくなる）が、入力の点の間隔によらず 2D と同じ長さで消える（区間ごとにダブを足していた前の 3D では、
/// 入力の間が間隔より短いと、ずっと短く消えた）。
#[test]
fn fade_reaches_the_same_length_as_2d_whatever_the_input_spacing() {
    let mut b = Brush::from(BrushSettings {
        radius: 6.0,
        hardness: 1.0,
        spacing: 0.25, // 間隔 3
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.controls.fade_size = 30; // 30 描点 = 90 画素で消える
                               // 右へ 1 画素ずつの入力（間隔より短い）
    let points: Vec<(f64, f64)> = (0..=140).map(|i| (20.0 + i as f64, 64.0)).collect();
    let reach = |px: &[u8]| {
        (0..W as usize)
            .filter(|&x| (0..H as usize).any(|y| px[(y * W as usize + x) * 4 + 3] > 0))
            .max()
            .unwrap()
    };
    let (flat, surface) = (draw_2d(&b, &points), draw_3d(&b, &points));
    let (a, c) = (reach(&flat), reach(&surface));
    println!("フェードで消える所: 2D {a}・3D {c}");
    assert!(
        (90..=115).contains(&a),
        "試験の前提: 90 画素ほどで消える ({a})"
    );
    assert!(a.abs_diff(c) <= 1, "2D {a}・3D {c}");
    let (painted, differ, max) = compare(&flat, &surface);
    assert!(
        max <= 4 && differ * 100 <= painted,
        "{differ}/{painted}・{max}"
    );
}

/// 3D の入力の点を、傾き・回転・時刻つきで描く（入力ごとに paint_queued を呼ぶ・呼ばないなどの塗り方を how で選ぶ）。
fn draw_3d_inputs(
    brush: &Brush,
    inputs: &[SurfaceInput],
    how: impl Fn(&mut Document, &mut SurfaceStroke, &mut yolu_core::Stroke, usize),
) -> Vec<u8> {
    let g = quad();
    let (mut d, l) = document();
    let mut stroke = d.begin_brush_stroke(l, brush).unwrap();
    let mut s = SurfaceStroke::begin_input(
        &mut d,
        &mut stroke,
        g,
        front(),
        &brush.base,
        Some(0),
        inputs[0],
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for (i, input) in inputs.iter().enumerate().skip(1) {
        s.add_input(&mut d, &mut stroke, *input).unwrap();
        how(&mut d, &mut s, &mut stroke, i);
    }
    s.finish(&mut d, &mut stroke).unwrap();
    assert_eq!(s.queued(), 0);
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// ペンの傾き（大きさと角度）・軸の回転（角度）・筆の速さ（不透明度）も、2D と同じ式で効く（3D の傾きは画面の右・上の軸、回転は画面で
/// 反時計回り、速さは画面の点 / 秒）。
#[test]
fn pen_tilt_rotation_and_speed_shape_3d_dabs_like_2d() {
    let mut b = Brush::from(BrushSettings {
        radius: 12.0,
        spacing: 0.125, // 間隔 3
        color: Rgba8::new(60, 160, 60, 255),
        ..BrushSettings::default()
    });
    b.tip.image = Some(tip("bristles"));
    b.controls.tilt_size = true;
    b.controls.tilt_angle = true;
    b.controls.rotation_angle = true;
    b.controls.speed_opacity = true;
    b.controls.speed_max = 2000.0;
    // 区間は間隔の倍数（9・6・12・15 個）。傾き・回転・時刻を点ごとに変える
    let points = line((50.0, 40.0), (0.6, 0.8), 3.0, &[9, 6, 12, 15]);
    let pens: Vec<(DVec2, f64, f64)> = (0..points.len())
        .map(|i| {
            let t = i as f64;
            (
                DVec2::new(0.1 * t - 0.15, 0.3 - 0.08 * t),
                0.4 * t,
                0.05 * t + 0.003 * t * t,
            )
        })
        .collect();
    let a = {
        let (mut d, l) = document();
        let mut s = d.begin_brush_stroke(l, &b).unwrap();
        for (&(x, y), &(tilt, rotation, time)) in points.iter().zip(&pens) {
            let sample = BrushSample::new(x, y, 1.0, time, tilt)
                .unwrap()
                .with_rotation(rotation)
                .unwrap();
            s.add_sample(&mut d, sample).unwrap();
        }
        d.end_stroke(s).unwrap();
        bytes(&d, l)
    };
    let g = quad();
    let b3 = brush_3d(&g, &b);
    let inputs: Vec<SurfaceInput> = points
        .iter()
        .zip(&pens)
        .map(|(&(x, y), &(tilt, rotation, time))| SurfaceInput {
            tilt,
            rotation,
            time,
            ..SurfaceInput::new(screen(x, y), 1.0)
        })
        .collect();
    let c = draw_3d_inputs(&b3, &inputs, |_, _, _, _| {});
    let (painted, differ, max) = compare(&a, &c);
    println!("傾き・回転・速さ: 塗った画素 {painted}・違う画素 {differ}・最大の差 {max}");
    assert!(painted > 300);
    assert!(
        max <= 4 && differ * 100 <= painted,
        "{differ}/{painted}・{max}"
    );
    // 傾き・回転・速さを渡さない 3D とは違う（効いている）
    let plain: Vec<SurfaceInput> = points
        .iter()
        .map(|&(x, y)| SurfaceInput::new(screen(x, y), 1.0))
        .collect();
    let (_, differ, _) = compare(&c, &draw_3d_inputs(&b3, &plain, |_, _, _, _| {}));
    assert!(
        differ * 4 > painted,
        "傾き・回転・速さが効く ({differ}/{painted})"
    );
}

/// デュアルブラシの 8 つの合わせ方が、どれも 2D と同じ。
#[test]
fn every_dual_brush_mode_matches_2d() {
    for mode in DualBrushMode::ALL {
        let mut b = Brush::from(BrushSettings {
            radius: 11.0,
            hardness: 0.6,
            spacing: 0.25, // 間隔 5.5
            color: Rgba8::new(120, 30, 200, 255),
            ..BrushSettings::default()
        });
        b.dual = Some(DualBrush {
            tip: Some(tip("dots")),
            radius: 11.0,
            spacing: 0.25,
            hardness: 0.5,
            mode,
            ..DualBrush::default()
        });
        let points = line((40.0, 40.0), (0.8, 0.6), 5.5, &[6, 4, 10]);
        let (painted, differ, max) = compare(&draw_2d(&b, &points), &draw_3d(&b, &points));
        println!("{mode:?}: 塗った画素 {painted}・違う画素 {differ}・最大の差 {max}");
        assert!(painted > 100, "{mode:?}: 試験の前提 ({painted})");
        assert!(
            max <= 4 && differ * 100 <= painted,
            "{mode:?}: {differ}/{painted}・{max}"
        );
    }
}

/// ゆらぎ・フェード・傾き・抜きのダブの列は、入力のまとまり方・フレームの区切り（待ち行列をいつ塗るか）によらず同じ。
#[test]
fn jitter_fade_and_tilt_do_not_depend_on_when_the_queue_is_painted() {
    queue_independence(true);
}

/// 曲線を切にしたブラシ（入力の点を直線で結び、点が来るたびに区間を並べる）でも、フレームの区切りによらず同じ。
#[test]
fn straight_segments_do_not_depend_on_when_the_queue_is_painted() {
    queue_independence(false);
}

/// ゆらぎ・フェード・傾き・抜き・色の変化のブラシを、入力ごとにすぐ全部塗る・5 つずつ塗る・離したときにまとめて塗るの 3 通りで描き、
/// 同じ画素になること（curve はブラシの「曲線」）。
fn queue_independence(curve: bool) {
    let mut b = Brush::from(BrushSettings {
        radius: 8.0,
        spacing: 0.05,
        color: Rgba8::new(200, 120, 20, 255),
        ..BrushSettings::default()
    });
    b.tip.image = Some(tip("charcoal"));
    b.tip.follow_direction = true;
    b.jitter = Jitter {
        size: 0.3,
        angle: 0.3,
        scatter: 0.6,
        opacity: 0.4,
        count: 3,
        ..Jitter::default()
    };
    b.controls.fade_size = 300;
    b.controls.tilt_opacity = true;
    b.assist.taper_out = 40.0;
    b.assist.curve = curve;
    b.color = ColorDynamics {
        hue: 0.5,
        ..ColorDynamics::default()
    };
    let g = quad();
    let b3 = brush_3d(&g, &b);
    let inputs: Vec<SurfaceInput> = (0..7)
        .map(|i| {
            let t = i as f64;
            SurfaceInput {
                tilt: DVec2::new(0.1 * t, -0.05 * t),
                time: 0.02 * t,
                ..SurfaceInput::new(screen(30.0 + 22.0 * t, 64.0 + 25.0 * (t * 0.9).sin()), 1.0)
            }
        })
        .collect();
    let at_once = draw_3d_inputs(&b3, &inputs, |d, s, stroke, _| {
        s.paint_queued(d, stroke, usize::MAX).unwrap();
    });
    let in_frames = draw_3d_inputs(&b3, &inputs, |d, s, stroke, _| {
        s.paint_queued(d, stroke, 5).unwrap();
    });
    let on_release = draw_3d_inputs(&b3, &inputs, |_, _, _, _| {});
    assert!(at_once.chunks_exact(4).filter(|p| p[3] > 0).count() > 500);
    assert!(in_frames == at_once, "曲線 {curve}: フレームに分けても同じ");
    assert!(
        on_release == at_once,
        "曲線 {curve}: 離したときにまとめて塗っても同じ"
    );
}

/// 紙の質感は、塗るテクセルの文書の画素の座標で読む: カメラを動かして描いても、2D で描いても、硬い丸で覆い切った所の値は同じ。
#[test]
fn paper_texture_follows_document_pixels_whatever_the_camera() {
    let mut b = Brush::from(BrushSettings {
        radius: 14.0,
        hardness: 1.0,
        spacing: 0.0625,
        opacity: 1.0,
        flow: 1.0,
        pressure_opacity: false,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.texture = Some(PaperTexture::new(tip("grain"), 1.0));
    let points = [(70.0, 60.0), (100.0, 64.0), (120.0, 66.0)];
    let flat = draw_2d(&b, &points);
    let g = quad();
    let b3 = brush_3d(&g, &b);
    let mut views = vec![front()];
    let mut moved = OrbitCamera {
        target: Vec3::new(W as f32 / 2.0 + 7.3, H as f32 / 2.0 - 4.1, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance: front()
            .position
            .distance(Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0))
            * 0.83,
        model_radius: W as f32,
        ..Default::default()
    };
    views.push(moved.view(W as f32 + 32.0, H as f32 + 32.0));
    moved.yaw = 12.0;
    views.push(moved.view(W as f32 + 32.0, H as f32 + 32.0));
    // 線の中ほどの範囲は、どの描き方でも覆い切っている
    let window = |px: &[u8]| -> Vec<u8> {
        let mut v = Vec::new();
        for y in 58..68 {
            for x in 85..105 {
                v.push(px[(y * W as usize + x) * 4 + 3]);
            }
        }
        v
    };
    let expected = window(&flat);
    assert!(
        expected.iter().max().unwrap() - expected.iter().min().unwrap() > 40,
        "試験の前提: 質感が見える"
    );
    for view in views {
        let (mut d, l) = document();
        let mut stroke = d.begin_brush_stroke(l, &b3).unwrap();
        let at = |x: f64, y: f64| view.to_screen(Vec3::new(x as f32, y as f32, 0.0)).unwrap();
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut stroke,
            g.clone(),
            view,
            &b3.base,
            Some(0),
            at(points[0].0, points[0].1),
            1.0,
            SurfaceStrokeOptions::default(),
        )
        .unwrap();
        for &(x, y) in &points[1..] {
            s.add(&mut d, &mut stroke, at(x, y), 1.0).unwrap();
        }
        s.finish(&mut d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
        assert_eq!(window(&bytes(&d, l)), expected, "{view:?}");
    }
}

/// 3D でも手ぶれ補正が効く: 糸より小さい上下の揺れは筆に届かず、離したときに最後の入力の点まで描く（長さは画面の点）。
#[test]
fn the_stabilizer_smooths_3d_input() {
    let zigzag: Vec<(f64, f64)> = (0..13)
        .map(|i| {
            (
                30.0 + 10.0 * i as f64,
                64.0 + if i % 2 == 0 { 9.0 } else { -9.0 },
            )
        })
        .collect();
    let rows_out = |stabilizer: f64| -> (usize, bool) {
        let mut b = Brush::from(BrushSettings {
            radius: 4.0,
            hardness: 1.0,
            spacing: 0.1,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        });
        b.assist.stabilizer = stabilizer;
        let px = draw_3d(&b, &zigzag);
        // 始まり（筆は最初の入力の点から）と終わり（離すと最後の入力の点まで描く）を除いた、線の中ほど
        let far = (0..H as usize)
            .filter(|&y| (y as f64 + 0.5 - 64.0).abs() > 9.0)
            .flat_map(|y| (70..120usize).map(move |x| (x, y)))
            .filter(|&(x, y)| px[(y * W as usize + x) * 4 + 3] > 0)
            .count();
        let (lx, ly) = zigzag[zigzag.len() - 1];
        let end = px[(ly as usize * W as usize + lx as usize) * 4 + 3] > 0;
        (far, end)
    };
    let (loose, _) = rows_out(0.0);
    let (steady, end) = rows_out(30.0);
    assert!(loose > 20, "補正なしは揺れの山まで描く ({loose})");
    assert_eq!(steady, 0, "糸より小さい揺れは描かない");
    assert!(end, "離したときに最後の入力の点まで描く");
}

/// 3D でも入り抜きが効く: 線の始めと終わりは細く、抜きの長さの内のダブは、線が伸びるか離すまで待つ（長さは画面の点）。
#[test]
fn taper_in_and_out_thin_the_3d_stroke_ends() {
    let mut b = Brush::from(BrushSettings {
        radius: 9.0,
        hardness: 1.0,
        spacing: 0.1,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.assist.taper_in = 40.0;
    b.assist.taper_out = 40.0;
    let g = quad();
    let b3 = brush_3d(&g, &b);
    let (mut d, l) = document();
    let mut stroke = d.begin_brush_stroke(l, &b3).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b3.base,
        Some(0),
        screen(20.0, 64.0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for i in 1..=8 {
        s.add(
            &mut d,
            &mut stroke,
            screen(20.0 + 20.0 * i as f64, 64.0),
            1.0,
        )
        .unwrap();
        s.paint_queued(&mut d, &mut stroke, usize::MAX).unwrap();
    }
    assert_eq!(s.queued(), 0, "抜きを待つダブは、塗れるダブに数えない");
    let height = |px: &[u8], x: usize| {
        (0..H as usize)
            .filter(|&y| px[(y * W as usize + x) * 4 + 3] > 0)
            .count()
    };
    let before = bytes(&d, l);
    assert_eq!(
        height(&before, 165),
        0,
        "抜きの長さの内は、離すまで描かない"
    );
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    let after = bytes(&d, l);
    let (start, middle, end) = (height(&after, 26), height(&after, 100), height(&after, 172));
    assert!(middle >= 17, "{middle}");
    assert!(start > 0 && start * 2 < middle, "入り {start} / {middle}");
    assert!(end > 0 && end * 2 < middle, "抜き {end} / {middle}");
}

/// 「曲線」の切り替えに従う: 切なら入力の点を直線で結び（2D の折れ線と同じ）、入なら 2D の曲線と同じく角を丸めて通る。
#[test]
fn the_3d_stroke_follows_the_curve_switch() {
    let mut b = Brush::from(BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        spacing: 0.25, // 間隔 1.5
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    // 鋭い角のある折れ線（区間の長さは間隔の倍数）
    let points = vec![(30.0, 30.0), (120.0, 30.0), (66.0, 102.0)];
    let set = |px: &[u8]| -> Vec<bool> { px.chunks_exact(4).map(|p| p[3] > 0).collect() };
    let overlap = |a: &[bool], c: &[bool]| -> f64 {
        let both = a.iter().zip(c).filter(|(x, y)| **x && **y).count();
        let either = a.iter().zip(c).filter(|(x, y)| **x || **y).count();
        both as f64 / either as f64
    };
    let straight = draw_3d(&b, &points);
    let (painted, differ, max) = compare(&draw_2d(&b, &points), &straight);
    assert!(
        max <= 4 && differ * 100 <= painted,
        "切: 2D の折れ線と同じ ({differ}/{painted}・{max})"
    );
    b.assist.curve = true;
    let curved = draw_3d(&b, &points);
    let flat_curve = draw_2d(&b, &points);
    let (same, apart) = (
        overlap(&set(&curved), &set(&flat_curve)),
        overlap(&set(&curved), &set(&straight)),
    );
    println!("曲線: 2D の曲線と重なる {same:.3}・折れ線と重なる {apart:.3}");
    assert!(same > 0.85, "入: 2D の曲線とほぼ同じ道 ({same})");
    assert!(apart < same - 0.1, "入: 折れ線とは違う道 ({apart})");
}

/// 鏡映の写しは筆先を左右に反転し、放射状の写しは回す（写したカメラから同じ画面の形で読むので、写しの側のダブは元のダブの鏡像・回した像）。
/// 見えない面にも塗る写し（カメラによらない足跡）も同じ。
#[test]
fn symmetric_copies_flip_and_turn_the_tip() {
    let mut b = Brush::from(BrushSettings {
        radius: 14.0,
        color: Rgba8::new(0, 0, 0, 255),
        ..BrushSettings::default()
    });
    b.tip.image = Some(tip("charcoal"));
    b.tip.angle = 30.0;
    let g = quad();
    let b3 = brush_3d(&g, &b);
    let center = Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0);
    let mirror = MirrorPlane::from_model(center, Quat::IDENTITY, SymmetryAxis::X, 0.0);
    let radial = RadialSymmetry::new(center, Vec3::Z, 2).unwrap();
    let dab = |setup: SurfaceSymmetrySetup| -> Vec<u8> {
        let (mut d, l) = document();
        let mut stroke = d.begin_brush_stroke(l, &b3).unwrap();
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut stroke,
            g.clone(),
            front(),
            &b3.base,
            Some(0),
            screen(60.0, 50.0),
            1.0,
            SurfaceStrokeOptions {
                symmetry: Some(setup),
                ..SurfaceStrokeOptions::default()
            },
        )
        .unwrap();
        s.finish(&mut d, &mut stroke).unwrap();
        assert_eq!(s.stats.copies, 1);
        d.end_stroke(stroke).unwrap();
        bytes(&d, l)
    };
    let alpha = |px: &[u8], x: usize, y: usize| px[(y * W as usize + x) * 4 + 3] as i32;
    for ignore_visibility in [false, true] {
        let mirrored = dab(SurfaceSymmetrySetup {
            mirror: Some(mirror),
            radial: None,
            ignore_visibility,
        });
        let turned = dab(SurfaceSymmetrySetup {
            mirror: None,
            radial: Some(radial),
            ignore_visibility,
        });
        let (mut flip, mut shift, mut turn, mut painted) = (0, 0, 0, 0);
        for y in 30..70usize {
            for x in 40..80usize {
                let a = alpha(&mirrored, x, y);
                if a > 0 {
                    painted += 1;
                }
                flip = flip.max((a - alpha(&mirrored, W as usize - 1 - x, y)).abs());
                shift = shift.max((a - alpha(&mirrored, x + (W as usize - 120), y)).abs());
                let r = alpha(&turned, x, y);
                turn = turn.max((r - alpha(&turned, W as usize - 1 - x, H as usize - 1 - y)).abs());
            }
        }
        println!("見えない面にも {ignore_visibility}: 鏡像の差 {flip}・ずらしただけの差 {shift}・回した差 {turn}");
        assert!(painted > 200);
        assert!(flip <= 6, "鏡映の写しは左右に反転した筆先 ({flip})");
        assert!(
            shift > 60,
            "試験の前提: ずらしただけの筆先とは違う ({shift})"
        );
        assert!(turn <= 6, "放射状の写しは回した筆先 ({turn})");
    }
}

/// ぼかし・指先・クローンも、覆いを筆先の形から取る（細長い筆先なら、変わる所も細長い）。2D の同じブラシと変わる所がほぼ重なる。
#[test]
fn blur_smudge_and_clone_use_the_tip_shape() {
    let checker = |d: &mut Document, l: LayerId| {
        for y in 0..H {
            for x in 0..W {
                let v = if (x / 3 + y / 3) % 2 == 0 { 230 } else { 20 };
                d.set_pixel(l, x, y, Rgba8::new(v, 255 - v, 90, 255))
                    .unwrap();
            }
        }
        d.clear_history().unwrap();
    };
    let g = quad();
    for effect in ["ぼかし", "指先", "クローン"] {
        let mut b = Brush::from(BrushSettings {
            radius: 20.0,
            spacing: 0.05,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        });
        b.tip.image = Some(tip("bristles"));
        let (start, end) = ((90.0, 64.0), (96.0, 64.0));
        let source = (61.0, 40.0); // 格子の周期（6）の倍数でないずれ
        b.effect = match effect {
            "ぼかし" => BrushEffect::Blur { radius: 2 },
            "指先" => BrushEffect::Smudge { strength: 1.0 },
            _ => BrushEffect::Clone {
                offset: DVec2::new(source.0 - start.0, source.1 - start.1),
            },
        };
        // 2D
        let (mut d, l) = document();
        checker(&mut d, l);
        let before = bytes(&d, l);
        let mut s = d.begin_brush_stroke(l, &b).unwrap();
        for (x, y) in [start, end] {
            s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
        }
        d.end_stroke(s).unwrap();
        let flat = bytes(&d, l);
        // 3D
        let b3 = brush_3d(&g, &b);
        let (mut d, l) = document();
        checker(&mut d, l);
        let view = front();
        let surface_effect = match effect {
            "ぼかし" => SurfaceEffect::Blur,
            "指先" => SurfaceEffect::Smudge,
            _ => SurfaceEffect::Clone(SurfaceCloneSource {
                source: pick(&g, &view, screen(source.0, source.1)).unwrap(),
                destination: None,
            }),
        };
        let mut stroke = d.begin_brush_stroke(l, &b3).unwrap();
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut stroke,
            g.clone(),
            view,
            &b3.base,
            Some(0),
            screen(start.0, start.1),
            1.0,
            SurfaceStrokeOptions {
                effect: surface_effect,
                ..SurfaceStrokeOptions::default()
            },
        )
        .unwrap();
        s.add(&mut d, &mut stroke, screen(end.0, end.1), 1.0)
            .unwrap();
        s.finish(&mut d, &mut stroke).unwrap();
        d.end_stroke(stroke).unwrap();
        let surface = bytes(&d, l);
        let changed = |px: &[u8]| -> Vec<bool> {
            px.chunks_exact(4)
                .zip(before.chunks_exact(4))
                .map(|(a, b)| a != b)
                .collect()
        };
        let (a, c) = (changed(&flat), changed(&surface));
        let both = a.iter().zip(&c).filter(|(x, y)| **x && **y).count();
        let either = a.iter().zip(&c).filter(|(x, y)| **x || **y).count();
        // 変わった所の外接の箱の縦横（細長い筆先: 横は直径の 1/4 ほど）
        let (mut x0, mut x1, mut y0, mut y1) = (W as usize, 0, H as usize, 0);
        for (i, _) in c.iter().enumerate().filter(|(_, v)| **v) {
            let (x, y) = (i % W as usize, i / W as usize);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        let (w, h) = (x1.saturating_sub(x0) + 1, y1.saturating_sub(y0) + 1);
        println!("{effect}: 変わった所が重なる {both}/{either}・3D の箱 {w}×{h}");
        assert!(either > 100, "{effect}: 試験の前提 ({either})");
        assert!(
            both * 10 >= either * 8,
            "{effect}: 2D と変わる所が重なる ({both}/{either})"
        );
        assert!(w * 2 < h, "{effect}: 細長い筆先の形 ({w}×{h})");
    }
}
