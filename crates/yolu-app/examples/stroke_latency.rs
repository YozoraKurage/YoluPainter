//! ペンの入力 1 つが画面に出るまでの遅れの内訳を測る台（CPU の表示の道。GPU は使わない）。
//!
//!   cargo run -p yolu-app --release --example stroke_latency -- [--kinds hard,soft,tip,smudge] [--sizes 8,64,256,1000]
//!       [--view fit|z100] [--spacing 0.15] [--doc 4096] [--repeat 3]
//!
//! 文書は `--doc` の正方形（既定 4096²）: 下に塗りつぶし、その上に大きな柔らかい筆で塗ったレイヤー、その上に描くレイヤー。入力は決まった点の列
//! （約 12 画素ごと）で、点を足すたびに、アプリの表示と同じ手順（`CanvasDisplay` の CPU の頁へ、変わったタイルを合成して上げる）を、
//! 見えている所が全部示されるまで回す。1 つの入力ごとに、次の区間の時間（ミリ秒）の中央値・p95・最長を TSV で出す:
//!   点の追加（入力 → ダブ → 文書。`Stroke::add_sample`）、最初の同期（合成 → 上げる）、見える所が済むまでの同期の合計、
//!   全体（点の追加 + 見える所が済むまで）、見える所が済むまでのフレーム数。
//! 測れないもの: ウィンドウの枠・egui の描画・GPU への転送と画面の更新の待ち（描画器の側）、OS のペン入力のイベントが来るまでの時間。
//! 出力は `ブラシ<TAB>大きさ<TAB>表示<TAB>測るもの<TAB>p50<TAB>p95<TAB>最長`。

use std::time::Instant;

use yolu_app::canvas::cpu::Viewport;
use yolu_app::canvas::display::CanvasDisplay;
use yolu_app::canvas::gpu::CanvasBackend;
use yolu_app::engine::{Channel, Document, LayerId, Rgba8};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushEffect, BrushSettings, ColorMix, DualBrush, MixMode, PaperTexture,
    Rect,
};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[(((s.len() - 1) as f64) * p).round() as usize]
}

/// 文書: 塗りつぶし → 大きな柔らかい筆で塗ったレイヤー（合成が空でないように）→ 描くレイヤー。
fn document(size: u32) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(size, size, 128).unwrap();
    doc.set_stroke_budget_bytes(512 << 20).unwrap();
    doc.add_fill_layer(
        "背景",
        &[(Channel::Color, Rgba8::new(235, 232, 225, 255))],
        None,
    )
    .unwrap();
    let paint = doc.add_layer("下絵").unwrap();
    let brush = BrushSettings {
        radius: size as f64 / 10.0,
        hardness: 0.3,
        spacing: 0.2,
        opacity: 0.7,
        color: Rgba8::new(90, 140, 190, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(paint, &brush).unwrap();
    for i in 0..40 {
        let t = i as f64 / 39.0;
        s.add_point(
            &mut doc,
            size as f64 * (0.1 + 0.8 * t),
            size as f64 * (0.3 + 0.4 * (t * 9.0).sin().abs()),
            1.0,
            DVec2::ZERO,
        )
        .unwrap();
    }
    doc.end_stroke(s).unwrap();
    // 効果のブラシが読む絵があるレイヤー
    let target = doc.add_layer("描くレイヤー").unwrap();
    let texture = BrushSettings {
        radius: size as f64 / 20.0,
        hardness: 0.6,
        spacing: 0.3,
        color: Rgba8::new(200, 90, 60, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke(target, &texture).unwrap();
    for i in 0..60 {
        let t = i as f64 / 59.0;
        s.add_point(
            &mut doc,
            size as f64 * (0.05 + 0.9 * t),
            size as f64 * (0.45 + 0.1 * (t * 14.0).sin()),
            1.0,
            DVec2::ZERO,
        )
        .unwrap();
    }
    doc.end_stroke(s).unwrap();
    doc.clear_history().unwrap();
    (doc, target)
}

fn brush_for(kind: &str, size: f64, spacing: f64) -> Option<Brush> {
    let radius = size / 2.0;
    let mut b = Brush::from(BrushSettings {
        radius,
        hardness: 1.0,
        spacing,
        color: Rgba8::new(30, 30, 60, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    });
    b.seed = 7;
    match kind {
        "hard" => {}
        "soft" => b.base.hardness = 0.0,
        "tip" => {
            b.base.hardness = 0.8;
            b.tip.image = builtin_tip("charcoal");
            b.tip.follow_direction = true;
        }
        "texture" => {
            b.base.hardness = 0.8;
            b.base.flow = 0.5;
            b.texture = Some(PaperTexture {
                scale: 2.0,
                ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.6)
            });
        }
        "dual" => {
            b.base.hardness = 0.8;
            b.dual = Some(DualBrush {
                radius: radius / 4.0,
                spacing: 0.25,
                scatter: 0.5,
                count: 2,
                hardness: 0.5,
                ..DualBrush::default()
            });
        }
        "mix" => {
            b.base.hardness = 0.8;
            b.mix = ColorMix {
                mode: MixMode::Mix,
                ..ColorMix::default()
            };
        }
        "smudge" => {
            b.base.hardness = 0.8;
            b.effect = BrushEffect::SMUDGE;
        }
        "blur" => {
            b.base.hardness = 0.8;
            b.effect = BrushEffect::BLUR;
        }
        _ => return None,
    }
    Some(b)
}

/// 入力の点の列（文書の中央のウィンドウに収まる。約 12 画素ごと）。
fn path(doc: u32) -> Vec<(f64, f64)> {
    let c = doc as f64 / 2.0;
    let n = 108;
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                c - 650.0 + 1300.0 * t,
                c + 300.0 * (t * std::f64::consts::TAU * 1.5).sin(),
            )
        })
        .collect()
}

fn viewport(view: &str, doc: &Document) -> Viewport {
    const WINDOW: (f64, f64) = (1280.0, 800.0);
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    match view {
        "z100" => {
            let (vw, vh) = (WINDOW.0 as u32, WINDOW.1 as u32);
            Viewport {
                visible: Rect::new((doc.width() - vw) / 2, (doc.height() - vh) / 2, vw, vh),
                pixel_size: 1.0,
            }
        }
        _ => Viewport {
            visible: Rect::new(0, 0, doc.width(), doc.height()),
            pixel_size: (WINDOW.0 / w).min(WINDOW.1 / h) as f32,
        },
    }
}

/// 1 回の測定の入力ごとの時間。
#[derive(Default)]
struct Samples {
    add: Vec<f64>,
    first: Vec<f64>,
    visible: Vec<f64>,
    total: Vec<f64>,
    frames: Vec<f64>,
}

fn run(doc_size: u32, brush: &Brush, points: &[(f64, f64)], view: &str, s: &mut Samples) {
    let (mut doc, target) = document(doc_size);
    let ctx = egui::Context::default();
    let mut display = CanvasDisplay::new();
    display.set_backend(CanvasBackend::Cpu);
    // 最初の全部を上げておく（描き始めの前の状態）
    display.set_viewport(None);
    display.sync_channel(&ctx, &doc, Channel::Color);
    ctx.tex_manager().write().take_delta().clear();
    display.set_viewport(Some(viewport(view, &doc)));
    loop {
        display.sync_channel(&ctx, &doc, Channel::Color);
        ctx.tex_manager().write().take_delta().clear();
        if display.pending_tiles() == 0 {
            break;
        }
    }
    let mut stroke = doc.begin_brush_stroke(target, brush).unwrap();
    for &(x, y) in points {
        let t = Instant::now();
        stroke.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).unwrap();
        let add = ms(t);
        // 見えている所が全部示されるまでフレームを回す（描いている途中の 1 回。離したあとの仕上げは含めない）
        let (mut first, mut visible, mut frames) = (0.0, 0.0, 0usize);
        loop {
            let t = Instant::now();
            display.sync_channel(&ctx, &doc, Channel::Color);
            let dt = ms(t);
            ctx.tex_manager().write().take_delta().clear();
            frames += 1;
            if frames == 1 {
                first = dt;
            }
            visible += dt;
            if display.unshown_visible_tiles() == 0 || frames > 100_000 {
                break;
            }
        }
        s.add.push(add);
        s.first.push(first);
        s.visible.push(visible);
        s.total.push(add + visible);
        s.frames.push(frames as f64);
    }
    doc.end_stroke(stroke).unwrap();
}

fn list(args: &[String], key: &str) -> Option<Vec<String>> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.to_string()).collect())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kinds = list(&args, "--kinds").unwrap_or_else(|| {
        ["hard", "soft", "tip", "mix", "smudge"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    });
    let sizes: Vec<f64> = list(&args, "--sizes").map_or(vec![8.0, 64.0, 256.0, 1000.0], |v| {
        v.iter().map(|s| s.parse().unwrap()).collect()
    });
    let views = list(&args, "--view").unwrap_or_else(|| vec!["fit".into(), "z100".into()]);
    let spacing: f64 = list(&args, "--spacing").map_or(0.15, |v| v[0].parse().unwrap());
    let doc_size: u32 = list(&args, "--doc").map_or(4096, |v| v[0].parse().unwrap());
    let repeat: usize = list(&args, "--repeat").map_or(3, |v| v[0].parse().unwrap());
    println!(
        "# 文書 {doc_size}²・間隔 {spacing}・スレッド {}",
        rayon::current_num_threads()
    );
    println!("ブラシ\t大きさ\t表示\t測るもの\tp50\tp95\t最長");
    let points = path(doc_size);
    for kind in &kinds {
        for &size in &sizes {
            let Some(brush) = brush_for(kind, size, spacing) else {
                eprintln!("知らない種類: {kind}");
                continue;
            };
            for view in &views {
                let mut s = Samples::default();
                for _ in 0..repeat {
                    run(doc_size, &brush, &points, view, &mut s);
                }
                for (name, v) in [
                    ("点の追加(ms)", &s.add),
                    ("最初の同期(ms)", &s.first),
                    ("見える所が済むまで(ms)", &s.visible),
                    ("全体: 点の追加 + 見える所(ms)", &s.total),
                    ("見える所が済むまでのフレーム", &s.frames),
                ] {
                    println!(
                        "{kind}\t{size}\t{view}\t{name}\t{:.3}\t{:.3}\t{:.3}",
                        percentile(v, 0.5),
                        percentile(v, 0.95),
                        v.iter().cloned().fold(0.0, f64::max)
                    );
                }
            }
        }
    }
}
