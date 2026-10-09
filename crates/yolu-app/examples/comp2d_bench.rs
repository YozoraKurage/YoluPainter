//! 2D の合成の速さを測る台（CPU の表示の道。GPU は使わない）。
//!
//! 実データの PSD（`--psd <パス> --label <A〜D>`）か、乱数で組んだ文書（`--synthetic [辺]`）を取り込んだ文書で、アプリの表示と同じ手順
//! （`CanvasDisplay` の CPU の頁へ、変わったタイルを合成して上げる）を回し、場面ごとの時間（ミリ秒）を TSV で出す。毎回同じ手順・同じ乱数で、
//! 繰り返しの中央値を出す。出力は `場面<TAB>測るもの<TAB>値`。
//!
//! 使い方: `cargo run -p yolu-app --release --example comp2d_bench -- --synthetic 2048` /
//! `-- --psd file.psd --label A`。`--only 名前` で場面を絞る（open・stroke・visible・opacity・effect・zoom・hash・merge・call）。

use std::time::Instant;

use yolu_app::canvas::cpu::Viewport;
use yolu_app::canvas::display::CanvasDisplay;
use yolu_app::canvas::gpu::CanvasBackend;
use yolu_app::engine::{Channel, Document, LayerId, LayerKind, Rgba8};
use yolu_core::effects::{EffectSettings, FilterSpec};
use yolu_core::glam::DVec2;
use yolu_core::{AdjustmentSettings, BlendMode, BrushSettings, FilterTarget, TileCoord};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}
fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[(((v.len() - 1) as f64) * p).round() as usize]
}

fn report(scene: &str, metric: &str, value: f64) {
    println!("{scene}\t{metric}\t{value:.2}");
}

/// 乱数の文書: 下地の塗りつぶし・ラスターレイヤー（合成モードと不透明度を替える）・通過のグループ（クリッピングと調整つき）・
/// 分離のグループ（マスクつき）・上のレイヤーと調整。返す 2 つは描くレイヤー（入れ子の中と、一番上の近く）。
fn synthetic(size: u32) -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(size, size, 128).unwrap();
    let mut rng = Rng(0xC0FFEE);
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Normal,
    ];
    let blob = |d: &mut Document, rng: &mut Rng, id: LayerId, count: usize| {
        for _ in 0..count {
            let r = rng.next();
            let brush = BrushSettings {
                radius: (size as f64 / 16.0) + rng.below(size as u64 / 8) as f64,
                hardness: 0.9,
                spacing: 0.3,
                opacity: 0.4 + (r % 60) as f64 / 100.0,
                flow: 1.0,
                color: Rgba8::new(r as u8, (r >> 8) as u8, (r >> 16) as u8, 255),
                pressure_size: false,
                pressure_opacity: false,
                pressure_flow: false,
                erase: false,
                anti_alias: yolu_core::AntiAlias::None,
            };
            let mut s = d.begin_stroke(id, &brush).unwrap();
            let (x, y) = (rng.below(size as u64) as f64, rng.below(size as u64) as f64);
            s.add_point(d, x, y, 1.0, DVec2::ZERO).unwrap();
            s.add_point(d, x + 30.0, y + 10.0, 1.0, DVec2::ZERO)
                .unwrap();
            d.end_stroke(s).unwrap();
        }
    };
    let bg = d.add_layer("bg").unwrap();
    blob(&mut d, &mut rng, bg, 40);
    let mut ids = Vec::new();
    for i in 0..6 {
        let id = d.add_layer(&format!("L{i}")).unwrap();
        blob(&mut d, &mut rng, id, 12);
        d.set_layer_blend_mode(id, modes[i % modes.len()]).unwrap();
        d.set_layer_opacity(id, 0.6 + 0.06 * i as f64, false)
            .unwrap();
        ids.push(id);
    }
    // 通過のグループ（入れ子の描くレイヤー・クリッピング・調整）
    let g1a = d.add_layer("g1a").unwrap();
    blob(&mut d, &mut rng, g1a, 10);
    let g1b = d.add_layer("g1b").unwrap();
    blob(&mut d, &mut rng, g1b, 10);
    let g1c = d.add_layer("g1c").unwrap();
    blob(&mut d, &mut rng, g1c, 8);
    d.set_layer_clipping(g1c, true).unwrap();
    let g1 = d.group_layers(&[g1a, g1b, g1c], "G1").unwrap();
    d.add_adjustment_layer(
        "lv",
        AdjustmentSettings::levels(0.05, 0.95, 1.1, 0.0, 1.0).unwrap(),
        None,
        Some(g1c),
    )
    .unwrap();
    let _ = g1;
    // 分離のグループ（マスクつき）
    let mut inner = Vec::new();
    for i in 0..4 {
        let id = d.add_layer(&format!("g2_{i}")).unwrap();
        blob(&mut d, &mut rng, id, 8);
        inner.push(id);
    }
    let g2 = d.group_layers(&inner, "G2").unwrap();
    d.set_layer_blend_mode(g2, BlendMode::Multiply).unwrap();
    d.set_layer_opacity(g2, 0.8, false).unwrap();
    d.add_layer_mask(inner[1]).unwrap();
    for i in 0..400u32 {
        d.set_mask_pixel(
            inner[1],
            (i * 37) % size,
            (i * 91) % size,
            (i * 53 % 256) as u8,
        )
        .unwrap();
    }
    // 上のレイヤー
    let mut top = Vec::new();
    for i in 0..4 {
        let id = d.add_layer(&format!("U{i}")).unwrap();
        blob(&mut d, &mut rng, id, 6);
        d.set_layer_opacity(id, 0.5 + 0.1 * i as f64, false)
            .unwrap();
        top.push(id);
    }
    d.add_adjustment_layer(
        "lv-top",
        AdjustmentSettings::levels(0.0, 1.0, 0.9, 0.0, 1.0).unwrap(),
        None,
        None,
    )
    .unwrap();
    d.clear_history().unwrap();
    (d, g1b, top[1])
}

/// 詰まった文書: 全面を半透明の乱数で埋めたレイヤーを `layers` 枚（合成モードを替える）。どのタイルも全部のレイヤーに画素がある（覚えが一番効く形）。
fn dense(size: u32, layers: usize) -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(size, size, 128).unwrap();
    d.set_source_budget_bytes(2 << 30).unwrap();
    let mut rng = Rng(0xD3E5E);
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::Normal,
    ];
    let mut ids = Vec::new();
    for i in 0..layers {
        let id = d.add_layer(&format!("D{i}")).unwrap();
        // 全面を 64 画素のマスごとに別の半透明の色で埋める（1 画素ずつ置くより速い）
        let brush = BrushSettings {
            radius: 48.0,
            hardness: 1.0,
            spacing: 0.2,
            opacity: 0.35,
            flow: 1.0,
            color: Rgba8::new(rng.next() as u8, rng.next() as u8, rng.next() as u8, 255),
            pressure_size: false,
            pressure_opacity: false,
            pressure_flow: false,
            erase: false,
            anti_alias: yolu_core::AntiAlias::None,
        };
        let mut s = d.begin_stroke(id, &brush).unwrap();
        let step = 40.0;
        let mut y = 0.0;
        while y < size as f64 + 40.0 {
            let mut x = 0.0;
            while x < size as f64 + 40.0 {
                s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
                x += step;
            }
            y += step;
        }
        d.end_stroke(s).unwrap();
        d.set_layer_blend_mode(id, modes[i % modes.len()]).unwrap();
        d.set_layer_opacity(id, 0.5 + 0.02 * (i % 10) as f64, false)
            .unwrap();
        ids.push(id);
    }
    d.clear_history().unwrap();
    (d, ids[layers / 2], ids[layers - 3])
}

fn load_psd(path: &str) -> Document {
    let file = std::fs::File::open(path).expect("PSD を開けない");
    let mut reader = std::io::BufReader::new(file);
    let options = yolu_io::psd::CopyOptions {
        source_budget: 4 * 1024 * 1024 * 1024,
        cancel: None,
    };
    match yolu_io::psd::import_copy(&mut reader, &options).expect("取り込みに失敗") {
        yolu_io::psd::CopyOutcome::Imported(i) => i.document,
        yolu_io::psd::CopyOutcome::Refused(r) => panic!("取り込めない: {r:?}"),
    }
}

/// 合成に出る（自分も親のグループも、表示していて不透明度が 0 でない）か。
fn shown(doc: &Document, l: &yolu_app::engine::Layer) -> bool {
    let mut cur = Some(l);
    while let Some(layer) = cur {
        if !layer.visible() || layer.opacity() <= 0.0 {
            return false;
        }
        cur = layer.parent().and_then(|p| doc.layer(p));
    }
    true
}

/// 色の面のタイルが多い順の、描けるレイヤー（合成に出るラスター）の ID。
fn rasters_by_tiles(doc: &Document) -> Vec<(LayerId, usize)> {
    let mut v: Vec<(LayerId, usize)> = doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Raster && shown(doc, l))
        .map(|l| {
            (
                l.id(),
                l.surface(Channel::Color).map_or(0, |s| s.tile_count()),
            )
        })
        .collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    v
}

/// 描くレイヤー: 紙の中ほどの順で、タイルを十分持つ（なければ一番多い）レイヤー。
fn middle_raster(doc: &Document) -> LayerId {
    let list = rasters_by_tiles(doc);
    let max = list.first().map_or(0, |(_, n)| *n);
    let order: Vec<LayerId> = doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Raster && shown(doc, l))
        .map(|l| l.id())
        .collect();
    let enough: Vec<LayerId> = order
        .iter()
        .copied()
        .filter(|id| {
            list.iter()
                .find(|(i, _)| i == id)
                .is_some_and(|(_, n)| *n >= (max / 3).max(4))
        })
        .collect();
    if enough.is_empty() {
        list.first()
            .map(|(id, _)| *id)
            .expect("ラスターレイヤーが無い")
    } else {
        enough[enough.len() / 2]
    }
}

/// 1 回の変更のあとの落ち着くまで（フレームごとの同期の時間。画面の更新の待ちは含めない）。
#[derive(Clone, Copy, Debug, Default)]
struct Settle {
    /// 最初のフレーム（変更を受けた同期）。
    first: f64,
    /// 見えている所が全部正確になるまでの同期の合計と、そのフレーム数。
    visible: f64,
    visible_frames: usize,
    /// 全部が正確になるまでの同期の合計。
    total: f64,
    /// 1 フレームの同期の最長（画面が止まる最長の時間）。
    worst: f64,
    frames: usize,
}

/// 見る範囲: ウィンドウに収めた全体（fit）と、等倍で中央（z100）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    /// 枠なしで全部を上げてから返る（前の同期と同じ形。比較用）。
    Blocking,
    Fit,
    Z100,
}

impl View {
    const ALL: [View; 3] = [View::Blocking, View::Fit, View::Z100];
    fn name(self) -> &'static str {
        match self {
            View::Blocking => "枠なし",
            View::Fit => "全体を表示",
            View::Z100 => "等倍",
        }
    }
    fn viewport(self, doc: &Document) -> Option<Viewport> {
        const WINDOW: (f64, f64) = (1280.0, 800.0);
        let (w, h) = (doc.width() as f64, doc.height() as f64);
        match self {
            View::Blocking => None,
            View::Fit => Some(Viewport {
                visible: yolu_core::Rect::new(0, 0, doc.width(), doc.height()),
                pixel_size: (WINDOW.0 / w).min(WINDOW.1 / h) as f32,
            }),
            View::Z100 => {
                let (vw, vh) = (
                    (WINDOW.0 as u32).min(doc.width()),
                    (WINDOW.1 as u32).min(doc.height()),
                );
                Some(Viewport {
                    visible: yolu_core::Rect::new(
                        (doc.width() - vw) / 2,
                        (doc.height() - vh) / 2,
                        vw,
                        vh,
                    ),
                    pixel_size: 1.0,
                })
            }
        }
    }
}

struct Bench {
    ctx: egui::Context,
    display: CanvasDisplay,
    repeat: usize,
}

/// 描画器が差分を受け取った代わり（毎フレームの終わりに egui が行う）。受け取らないと、上げた画像が積もり続ける。
fn drain(ctx: &egui::Context) {
    ctx.tex_manager().write().take_delta().clear();
}

impl Bench {
    fn new(repeat: usize) -> Bench {
        let mut display = CanvasDisplay::new();
        display.set_backend(CanvasBackend::Cpu);
        Bench {
            ctx: egui::Context::default(),
            display,
            repeat,
        }
    }
    /// 全部を上げるまで返らない同期の時間。
    fn sync_ms(&mut self, doc: &Document) -> (f64, usize) {
        self.display.set_viewport(None);
        let t = Instant::now();
        let n = self.display.sync_channel(&self.ctx, doc, Channel::Color);
        let dt = ms(t);
        drain(&self.ctx);
        (dt, n)
    }
    /// 全部が正確になるまでフレームを回す。
    fn settle(&mut self, doc: &Document, view: View) -> Settle {
        self.display.set_viewport(view.viewport(doc));
        let mut s = Settle::default();
        let mut visible_done = false;
        loop {
            let t = Instant::now();
            self.display.sync_channel(&self.ctx, doc, Channel::Color);
            let dt = ms(t);
            drain(&self.ctx);
            s.frames += 1;
            s.total += dt;
            s.worst = s.worst.max(dt);
            if s.frames == 1 {
                s.first = dt;
            }
            if !visible_done {
                s.visible += dt;
                s.visible_frames += 1;
                visible_done = self.display.pending_visible_tiles() == 0;
            }
            if self.display.pending_tiles() == 0 || s.frames > 100_000 {
                break;
            }
        }
        s
    }
}

impl Bench {
    /// ドラッグの途中の 1 回: 変更を受けたあと、見えている所が示されるまでフレームを回す（離したあとの仕上げは含めない）。
    fn settle_dragging(&mut self, doc: &Document, view: View) -> Settle {
        self.display.set_viewport(view.viewport(doc));
        let mut s = Settle::default();
        loop {
            let t = Instant::now();
            self.display.sync_channel(&self.ctx, doc, Channel::Color);
            let dt = ms(t);
            drain(&self.ctx);
            s.frames += 1;
            s.total += dt;
            s.visible += dt;
            s.visible_frames += 1;
            s.worst = s.worst.max(dt);
            if s.frames == 1 {
                s.first = dt;
            }
            if self.display.unshown_visible_tiles() == 0 || s.frames > 100_000 {
                break;
            }
        }
        s
    }
}

fn report_settle(scene: &str, view: View, s: &Settle) {
    let v = view.name();
    report(scene, &format!("[{v}] 最初のフレーム(ms)"), s.first);
    if view != View::Blocking {
        report(scene, &format!("[{v}] 見える所が済むまで(ms)"), s.visible);
        report(
            scene,
            &format!("[{v}] 見える所が済むまでのフレーム"),
            s.visible_frames as f64,
        );
        report(scene, &format!("[{v}] 1 フレームの最長(ms)"), s.worst);
    }
    report(scene, &format!("[{v}] 全部が済むまで(ms)"), s.total);
}

/// 変更を当てては落ち着かせる（見る範囲ごとに、同じ変更を同じ状態から）。変更は `apply(doc, true)` で当て、`apply(doc, false)` で戻す。
fn scene_change(
    b: &mut Bench,
    doc: &mut Document,
    scene: &str,
    mut apply: impl FnMut(&mut Document, bool),
) {
    for view in View::ALL {
        let mut firsts = Vec::new();
        let mut visibles = Vec::new();
        let mut totals = Vec::new();
        let mut worsts = Vec::new();
        let mut frames = Vec::new();
        let mut shown = Settle::default();
        for _ in 0..b.repeat {
            apply(doc, true);
            let s = b.settle(doc, view);
            firsts.push(s.first);
            visibles.push(s.visible);
            totals.push(s.total);
            worsts.push(s.worst);
            frames.push(s.visible_frames as f64);
            apply(doc, false);
            b.settle(doc, View::Blocking);
        }
        shown.first = median(&mut firsts);
        shown.visible = median(&mut visibles);
        shown.total = median(&mut totals);
        shown.worst = median(&mut worsts);
        shown.visible_frames = median(&mut frames) as usize;
        report_settle(scene, view, &shown);
    }
}

fn scene_open(b: &mut Bench, doc: &Document) {
    for view in View::ALL {
        let mut firsts = Vec::new();
        let mut visibles = Vec::new();
        let mut totals = Vec::new();
        let mut worsts = Vec::new();
        let mut frames = Vec::new();
        for _ in 0..b.repeat {
            b.display.invalidate();
            let s = b.settle(doc, view);
            firsts.push(s.first);
            visibles.push(s.visible);
            totals.push(s.total);
            worsts.push(s.worst);
            frames.push(s.visible_frames as f64);
        }
        let s = Settle {
            first: median(&mut firsts),
            visible: median(&mut visibles),
            total: median(&mut totals),
            worst: median(&mut worsts),
            visible_frames: median(&mut frames) as usize,
            frames: 0,
        };
        report_settle("open", view, &s);
    }
    b.display.invalidate();
    b.sync_ms(doc);
}

fn scene_stroke(b: &mut Bench, doc: &mut Document, layer: LayerId, label: &str) {
    for memo in [false, true] {
        doc.set_composite_memo(memo);
        let name = format!(
            "{label}{}",
            if memo {
                "(覚えあり)"
            } else {
                "(覚えなし)"
            }
        );
        stroke_once(b, doc, layer, &name);
    }
    doc.set_composite_memo(true);
}

fn stroke_once(b: &mut Bench, doc: &mut Document, layer: LayerId, label: &str) {
    let (w, h) = (doc.width() as f64, doc.height() as f64);
    let brush = BrushSettings {
        radius: (w.min(h) / 80.0).max(6.0),
        hardness: 0.8,
        spacing: 0.15,
        opacity: 1.0,
        flow: 1.0,
        color: Rgba8::new(240, 30, 120, 255),
        pressure_size: false,
        pressure_opacity: false,
        pressure_flow: false,
        erase: false,
        anti_alias: yolu_core::AntiAlias::None,
    };
    let mut frames = Vec::new();
    let mut stroke_ms = Vec::new();
    let mut tiles = 0usize;
    let mut first = 0.0;
    let mut s = doc.begin_stroke(layer, &brush).expect("描けるレイヤー");
    let total = Instant::now();
    for i in 0..120 {
        let t = i as f64 / 119.0;
        let x = w * (0.3 + 0.4 * t);
        let y = h * (0.5 + 0.18 * (t * std::f64::consts::TAU * 1.5).sin());
        let ts = Instant::now();
        s.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
        stroke_ms.push(ms(ts));
        let (t, n) = b.sync_ms(doc);
        if i == 0 {
            first = t;
        }
        tiles += n;
        frames.push(t);
    }
    let wall = ms(total);
    let t_end = Instant::now();
    doc.end_stroke(s).unwrap();
    let end_ms = ms(t_end);
    let (settle, _) = b.sync_ms(doc);
    let mut all = frames.clone();
    let mut tail: Vec<f64> = frames[20..].to_vec();
    report(label, "1 フレームの合成 中央値(ms)", median(&mut all));
    report(
        label,
        "1 フレームの合成 p95(ms)",
        percentile(&mut all, 0.95),
    );
    report(label, "最初のフレーム(ms)", first);
    report(label, "2 秒目以降の中央値(ms)", median(&mut tail));
    report(label, "点の追加 中央値(ms)", median(&mut stroke_ms));
    report(label, "120 点の全体(ms)", wall);
    report(label, "上げたタイル(合計)", tiles as f64);
    report(label, "確定(ms)", end_ms);
    report(label, "確定後の表示(ms)", settle);
    b.sync_ms(doc);
}

fn scene_toggle(b: &mut Bench, doc: &mut Document, layer: LayerId) {
    scene_change(b, doc, "visible", |d, on| {
        d.set_layer_visible(layer, !on).unwrap();
    });
    // 戻しの同期は上の中で済んでいる
    b.sync_ms(doc);
}

fn scene_opacity(b: &mut Bench, doc: &mut Document, layer: LayerId) {
    scene_change(b, doc, "opacity", |d, on| {
        d.set_layer_opacity(layer, if on { 0.4 } else { 1.0 }, false)
            .unwrap();
    });
    b.sync_ms(doc);
}

fn scene_effect(b: &mut Bench, doc: &mut Document, layer: LayerId) {
    let id = doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(8)),
        )
        .unwrap();
    b.settle(doc, View::Blocking);
    // 半径をドラッグで変える（まとめて 1 回の Undo。離すまで続く）: 1 回ごとの表示
    for view in View::ALL {
        let mut steps = Vec::new();
        let mut visibles = Vec::new();
        let mut worsts = Vec::new();
        for r in [12u32, 18, 26, 36, 48, 60, 40, 24] {
            doc.set_filter_settings(layer, id, EffectSettings::blur(r), true)
                .unwrap();
            let s = b.settle_dragging(doc, view);
            steps.push(s.first);
            visibles.push(s.visible);
            worsts.push(s.worst);
        }
        let v = view.name();
        report(
            "effect",
            &format!("[{v}] 半径を変えた 1 回の最初のフレーム 中央値(ms)"),
            median(&mut steps),
        );
        if view != View::Blocking {
            report(
                "effect",
                &format!("[{v}] 半径を変えた 1 回の見える所 中央値(ms)"),
                median(&mut visibles),
            );
            report(
                "effect",
                &format!("[{v}] 半径を変えた 1 回の最長フレーム 中央値(ms)"),
                median(&mut worsts),
            );
        }
        // 離したあと
        doc.end_coalescing();
        let s = b.settle(doc, view);
        report_settle("effect", view, &s);
    }
    doc.remove_filter(layer, id).unwrap();
    b.sync_ms(doc);
}

fn scene_zoom(b: &mut Bench, doc: &Document) {
    // 拡大が補間の境（画素 1 つが 2 点）を越える: 頁を作り直して見えている所から
    for view in [View::Blocking, View::Z100] {
        let mut firsts = Vec::new();
        let mut visibles = Vec::new();
        let mut totals = Vec::new();
        for _ in 0..b.repeat.max(2) {
            b.display.set_nearest(false);
            b.settle(doc, View::Blocking);
            b.display.set_nearest(true);
            let s = b.settle(doc, view);
            firsts.push(s.first);
            visibles.push(s.visible);
            totals.push(s.total);
        }
        let v = view.name();
        report(
            "zoom",
            &format!("[{v}] 補間の境を越えた最初のフレーム(ms)"),
            median(&mut firsts),
        );
        if view != View::Blocking {
            report(
                "zoom",
                &format!("[{v}] 補間の境を越えて見える所が済むまで(ms)"),
                median(&mut visibles),
            );
        }
        report(
            "zoom",
            &format!("[{v}] 補間の境を越えて全部が済むまで(ms)"),
            median(&mut totals),
        );
    }
    b.display.set_nearest(false);
    b.sync_ms(doc);
}

/// 結合（表示に寄与するレイヤー・上から最初に結合できるレイヤーを下へ）の時間と、結合したあとの合成の指紋（前後のコードで同じ値になること）。結合は Undo で戻す。
fn scene_merge(doc: &mut Document) {
    let whole = |d: &Document| fnv(&d.composite_channel(Channel::Color, d.bounds()).unwrap());
    // 結合の 1 段は外したレイヤーと結果のレイヤーの分だけ大きく、既定の履歴の予算では Undo の段が残らない
    doc.set_undo_budget_bytes(4 << 30).unwrap();
    let before = whole(doc);
    let mut run = |label: &str, f: &mut dyn FnMut(&mut Document) -> bool| {
        let t = Instant::now();
        if f(doc) {
            report("merge", &format!("{label}(ms)"), ms(t));
            println!("merge\t{label} 後の合成の指紋\t{:016x}", whole(doc));
            assert!(doc.undo().unwrap(), "結合の Undo の段が残っている");
            assert_eq!(whole(doc), before, "Undo で結合の前に戻る");
        } else {
            println!("merge\t{label} は断られた\t-");
        }
    };
    run("表示に寄与するレイヤーを結合", &mut |d| {
        d.merge_visible("merged", 255).is_ok()
    });
    // 下のレイヤーへ: 上から順に、断られない最初のレイヤー（断られたレイヤーは文書を変えない）
    run(
        "上から最初に結合できるレイヤーを下へ結合",
        &mut |d| {
            let ids: Vec<LayerId> = d.layers().iter().rev().map(|l| l.id()).collect();
            ids.iter().any(|id| d.merge_down(*id, 255).is_ok())
        },
    );
}

/// 合成の呼び出し 1 回の固定費: 中央の 8×8 タイルを、タイルごと（スレッド 1 / 既定）・1 回の矩形で合成する時間。
fn scene_call(doc: &Document) {
    let ts = doc.tile_size();
    let (cx, cy) = (doc.width() / ts / 2, doc.height() / ts / 2);
    let tiles: Vec<yolu_core::Rect> = (0..8)
        .flat_map(|j| (0..8).map(move |i| (i, j)))
        .filter_map(|(i, j)| doc.tile_rect(TileCoord::new(cx - 4 + i, cy - 4 + j)))
        .collect();
    let mut buf = vec![0u8; (ts * ts * 4) as usize];
    let mut per_tile = |label: &str, pool: Option<&rayon::ThreadPool>| {
        let mut v = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let mut run = || {
                for r in &tiles {
                    doc.composite_into(
                        Channel::Color,
                        *r,
                        &mut buf[..(r.width * r.height * 4) as usize],
                        yolu_core::RowOrder::BottomUp,
                    )
                    .unwrap();
                }
            };
            match pool {
                Some(p) => p.install(run),
                None => run(),
            }
            v.push(ms(t) / tiles.len() as f64);
        }
        report("call", label, median(&mut v));
    };
    per_tile("タイルごとの呼び出し(1 タイル ms)", None);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    per_tile("タイルごと・スレッド 1(1 タイル ms)", Some(&one));
    let coords: Vec<TileCoord> = (0..8)
        .flat_map(|j| (0..8).map(move |i| TileCoord::new(cx - 4 + i, cy - 4 + j)))
        .collect();
    let mut v = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _ = doc.composite_tiles(Channel::Color, &coords).unwrap();
        v.push(ms(t) / 64.0);
    }
    report("call", "8×8 を 1 回のタイル束(1 タイル ms)", median(&mut v));
    let mut v = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        for c in &coords[..3] {
            let _ = doc.composite_tiles(Channel::Color, &[*c]).unwrap();
        }
        v.push(ms(t) / 3.0);
    }
    report("call", "タイル束を 1 枚ずつ(1 タイル ms)", median(&mut v));
    // 全タイルを束（64 枚）ずつ: 合成だけ・乗算済みへの変換まで・egui の頁へ上げるまで
    let all: Vec<TileCoord> = doc.canvas_tiles().collect();
    let mut v = Vec::new();
    let mut v_conv = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let mut conv = 0.0;
        for chunk in all.chunks(64) {
            let tiles = doc.composite_tiles(Channel::Color, chunk).unwrap();
            let t2 = Instant::now();
            for tile in &tiles {
                let img: Vec<egui::Color32> = tile
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
                    .collect();
                std::hint::black_box(&img);
            }
            conv += ms(t2);
        }
        v.push(ms(t) - conv);
        v_conv.push(conv);
    }
    report("call", "全タイルの合成だけ(ms)", median(&mut v));
    // 歩幅つきの粗い合成（効果の操作中の仮の絵）: 全タイルを束（64 枚）ずつ。歩幅はタイルの一辺の約数だけ
    for stride in [2u32, 4, 8] {
        if !ts.is_multiple_of(stride) {
            continue;
        }
        let mut v = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            for chunk in all.chunks(64) {
                std::hint::black_box(
                    doc.composite_coarse_tiles(Channel::Color, chunk, stride)
                        .unwrap(),
                );
            }
            v.push(ms(t));
        }
        report(
            "call",
            &format!("全タイルの粗い合成 歩幅 {stride}(ms)"),
            median(&mut v),
        );
    }
    report(
        "call",
        "全タイルの乗算済みへの変換だけ(ms)",
        median(&mut v_conv),
    );
    let r = yolu_core::Rect::new((cx - 4) * ts, (cy - 4) * ts, 8 * ts, 8 * ts);
    let mut big = vec![0u8; (r.width * r.height * 4) as usize];
    let mut v = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        doc.composite_into(Channel::Color, r, &mut big, yolu_core::RowOrder::BottomUp)
            .unwrap();
        v.push(ms(t) / 64.0);
    }
    report("call", "8×8 を 1 回の矩形(1 タイル ms)", median(&mut v));
    let mut v = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        one.install(|| {
            doc.composite_into(Channel::Color, r, &mut big, yolu_core::RowOrder::BottomUp)
                .unwrap()
        });
        v.push(ms(t) / 64.0);
    }
    report(
        "call",
        "8×8 を 1 回の矩形・スレッド 1(1 タイル ms)",
        median(&mut v),
    );
    // キャンバス全体を 1 回の矩形で（スレッド 1 / 既定）: 空のタイルが多い文書の、1 コアあたりの全体の費用
    let whole = doc.bounds();
    let mut all_px = vec![0u8; (whole.width as usize) * (whole.height as usize) * 4];
    for (label, pool) in [("スレッド 1", Some(&one)), ("既定", None)] {
        let mut v = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            let mut run = || {
                doc.composite_into(
                    Channel::Color,
                    whole,
                    &mut all_px,
                    yolu_core::RowOrder::BottomUp,
                )
                .unwrap()
            };
            match pool {
                Some(p) => p.install(run),
                None => run(),
            }
            v.push(ms(t));
        }
        report(
            "call",
            &format!("キャンバス全体を 1 回の矩形・{label}(ms)"),
            median(&mut v),
        );
    }
}

/// 束と 1 枚ずつの比: 中央の 8×8 タイルを、1 枚ずつ `composite_into` で（今までの表示の呼び方）・1 回の `composite_tiles`（束）で合成する時間を、
/// 繰り返しごとに続けて測って比を出す（負荷の揺れが両方にかかる）。比 = 1 枚ずつの時間 ÷ 束の時間。スレッドは両方とも既定（1 枚ずつの方は、
/// 1 枚の仕事が小さければ呼んだスレッドだけで計算する）。値は繰り返しの中央値と最小・最大。
fn scene_bundle_ratio(doc: &Document) {
    let ts = doc.tile_size();
    let (cx, cy) = (doc.width() / ts / 2, doc.height() / ts / 2);
    let coords: Vec<TileCoord> = (0..8)
        .flat_map(|j| (0..8).map(move |i| TileCoord::new(cx - 4 + i, cy - 4 + j)))
        .collect();
    let rects: Vec<yolu_core::Rect> = coords.iter().filter_map(|c| doc.tile_rect(*c)).collect();
    let mut buf = vec![0u8; (ts * ts * 4) as usize];
    let (mut singles, mut bundles, mut ratios) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..31 {
        let t = Instant::now();
        for r in &rects {
            doc.composite_into(
                Channel::Color,
                *r,
                &mut buf[..(r.width * r.height * 4) as usize],
                yolu_core::RowOrder::BottomUp,
            )
            .unwrap();
        }
        let single = ms(t) / rects.len() as f64;
        let t = Instant::now();
        let _ = doc.composite_tiles(Channel::Color, &coords).unwrap();
        let bundle = ms(t) / rects.len() as f64;
        singles.push(single);
        bundles.push(bundle);
        ratios.push(single / bundle);
    }
    report(
        "call",
        "1 枚ずつ composite_into(1 タイル ms)",
        median(&mut singles),
    );
    report(
        "call",
        "1 回の composite_tiles(1 タイル ms)",
        median(&mut bundles),
    );
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    report("call", "1 枚ずつ ÷ 束(倍) 最小", ratios[0]);
    report(
        "call",
        "1 枚ずつ ÷ 束(倍) 中央値",
        median(&mut ratios.clone()),
    );
    report("call", "1 枚ずつ ÷ 束(倍) 最大", ratios[ratios.len() - 1]);
}

/// 全体の合成の指紋（FNV-1a 64）。前後のコードで同じ文書から同じ値になること（バイトが変わらないこと）の確かめ。
fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 文書の合成の指紋: 開いた直後・描いている途中（20 点ごと）・描いたあと・不透明度を変えたあと。覚えを使った道と使わない道で同じ値になることも確かめる。
fn scene_hash(doc: &mut Document, layer: LayerId) {
    let whole = |d: &Document| fnv(&d.composite_channel(Channel::Color, d.bounds()).unwrap());
    let run = |doc: &mut Document| -> Vec<(String, u64)> {
        let mut out = vec![("開いた直後".to_string(), whole(doc))];
        let (w, h) = (doc.width() as f64, doc.height() as f64);
        let brush = BrushSettings {
            radius: (w.min(h) / 80.0).max(6.0),
            hardness: 0.8,
            spacing: 0.15,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(240, 30, 120, 255),
            pressure_size: false,
            pressure_opacity: false,
            pressure_flow: false,
            erase: false,
            anti_alias: yolu_core::AntiAlias::None,
        };
        let mut s = doc.begin_stroke(layer, &brush).expect("描けるレイヤー");
        for i in 0..60 {
            let t = i as f64 / 59.0;
            let x = w * (0.3 + 0.4 * t);
            let y = h * (0.5 + 0.18 * (t * std::f64::consts::TAU * 1.5).sin());
            s.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
            if i % 20 == 19 {
                out.push((format!("描いている途中 {}", i + 1), whole(doc)));
            }
        }
        doc.end_stroke(s).unwrap();
        out.push(("描いたあと".to_string(), whole(doc)));
        doc.set_layer_opacity(layer, 0.5, false).unwrap();
        out.push(("不透明度 0.5".to_string(), whole(doc)));
        doc.undo().unwrap();
        doc.undo().unwrap();
        out
    };
    doc.set_composite_memo(true);
    let with = run(doc);
    let hits = doc.composite_memo_stats().hits;
    doc.set_composite_memo(false);
    let without = run(doc);
    doc.set_composite_memo(true);
    assert_eq!(with, without, "覚えを使った合成と使わない合成が違う");
    for (name, h) in &with {
        println!("hash\t{name}\t{h:016x}");
    }
    println!("hash\t覚えから続けたタイル\t{hits}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut synth: Option<u32> = None;
    let mut dense_doc: Option<(u32, usize)> = None;
    let mut psd: Option<String> = None;
    let mut label = String::from("S");
    let mut only: Option<String> = None;
    let mut repeat = 5usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--synthetic" => {
                synth = Some(args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(2048));
                if args.get(i + 1).is_some_and(|s| s.parse::<u32>().is_ok()) {
                    i += 1;
                }
            }
            "--dense" => {
                // --dense 辺 レイヤーの数
                dense_doc = Some((
                    args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(1024),
                    args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(24),
                ));
                i += 2;
            }
            "--psd" => {
                psd = args.get(i + 1).cloned();
                i += 1;
            }
            "--label" => {
                label = args.get(i + 1).cloned().unwrap_or_default();
                i += 1;
            }
            "--only" => {
                only = args.get(i + 1).cloned();
                i += 1;
            }
            "--repeat" => {
                repeat = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let (mut doc, nested, top) = match (&psd, synth) {
        _ if dense_doc.is_some() => {
            let (size, layers) = dense_doc.unwrap();
            dense(size, layers)
        }
        (Some(path), _) => {
            let t = Instant::now();
            let d = load_psd(path);
            eprintln!("取り込み: {:.0} ms", ms(t));
            let mid = middle_raster(&d);
            let top = rasters_by_tiles(&d).first().map(|(id, _)| *id).unwrap();
            (d, mid, top)
        }
        (None, size) => synthetic(size.unwrap_or(2048)),
    };
    eprintln!(
        "{label}: {}×{} タイル{} レイヤー{} 描くレイヤーの番号={:?}",
        doc.width(),
        doc.height(),
        doc.tile_size(),
        doc.layers().len(),
        doc.layer_index(nested)
    );
    let mut b = Bench::new(repeat);
    let want = |name: &str| only.as_deref().is_none_or(|o| o == name);
    println!(
        "# {label}\t{}×{}\tレイヤー {}",
        doc.width(),
        doc.height(),
        doc.layers().len()
    );
    if want("open") {
        scene_open(&mut b, &doc);
    } else {
        b.sync_ms(&doc);
    }
    let big = rasters_by_tiles(&doc).first().map(|(id, _)| *id).unwrap();
    if want("stroke") {
        scene_stroke(&mut b, &mut doc, nested, "stroke");
        if nested != top {
            scene_stroke(&mut b, &mut doc, top, "stroke-top");
        }
    }
    if want("visible") {
        scene_toggle(&mut b, &mut doc, big);
    }
    if want("opacity") {
        scene_opacity(&mut b, &mut doc, big);
    }
    if want("effect") {
        scene_effect(&mut b, &mut doc, big);
    }
    if want("hash") {
        scene_hash(&mut doc, nested);
    }
    if want("merge") {
        scene_merge(&mut doc);
    }
    if want("call") {
        scene_call(&doc);
        scene_bundle_ratio(&doc);
    }
    if want("zoom") {
        scene_zoom(&mut b, &doc);
    }
    let _ = TileCoord::new(0, 0);
}
