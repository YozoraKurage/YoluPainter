//! ほかのテクスチャセットを指すポインターレイヤーの、向こうに描いた後のこちらの更新の時間を測る台（CPU。GPU は使わない）。
//!
//!   cargo run -p yolu-app --release --example pointer_feed_bench -- [--size 4096] [--points 100] [--runs 5] [--all-channels]
//!
//! 文書は 2 つのテクスチャセット（`--size` の正方形、既定 4096²）。A のポインターレイヤーが B の最初のレイヤーセットを指す（既定は Color だけを
//! 読む。`--all-channels` は文書の全部のチャンネル）。B の描くレイヤーに `--points` 点のストロークを描いた後、次を測る:
//!   届ける（効果の入力の同期: B の合成の変わったタイルを作り直し、A へ渡す）、A の合成（A の変わったタイルを合成する。3D ビューが読む所）。
//! 最初の 1 回（B の全体を合成して渡す）も出す。出力は `測るもの<TAB>p50<TAB>最長`（ミリ秒）。

use std::time::Instant;

use yolu_app::engine::{Channel, LayerKind, Rgba8};
use yolu_app::fx::pointers::set_key;
use yolu_app::fx::pointers::PointerOp;
use yolu_app::fx::FxOp;
use yolu_app::state::{Action, AppState};
use yolu_core::glam::DVec2;
use yolu_core::{BrushSettings, ExternalSet, PointerTarget, RowOrder};

fn arg(name: &str, default: u32) -> u32 {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn stats(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[v.len() - 1])
}

/// A の、`since` の後に変わったタイルを合成する（3D ビューが読む所）。合成したタイルの数を返す。
fn composite_changed(app: &AppState, since: u64) -> usize {
    let doc = &app.session.doc;
    let tiles = doc
        .changed_tiles(Channel::Color, since)
        .unwrap_or_else(|| doc.canvas_tiles().collect());
    let ts = doc.tile_size() as usize;
    let mut buf = vec![0u8; ts * ts * 4];
    for t in &tiles {
        let r = doc.tile_rect(*t).unwrap();
        let out = &mut buf[..r.width as usize * r.height as usize * 4];
        doc.composite_into(Channel::Color, r, out, RowOrder::BottomUp)
            .unwrap();
    }
    tiles.len()
}

fn main() {
    let size = arg("--size", 4096);
    let points = arg("--points", 100);
    let runs = arg("--runs", 5).max(1) as usize;
    let all = std::env::args().any(|a| a == "--all-channels");
    let mut app = AppState::new(size, size);
    app.add_texture_set().unwrap();
    let b = app.session.sets.current_index();
    let key = ExternalSet {
        texture_set: set_key(&app.session.sets.current().id).unwrap(),
        layer_set: app.session.doc.layer_sets()[0].id.0,
    };
    let raster = app
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == LayerKind::Raster)
        .unwrap()
        .id();
    app.switch_set(0).unwrap();
    app.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let id = app.session.selected_layer.unwrap();
    app.apply(Action::Fx(FxOp::Pointer(PointerOp::Target {
        layer: id,
        target: PointerTarget::External(key),
    })));
    if !all {
        for c in app.session.doc.channels() {
            if c != Channel::Color {
                app.session.doc.set_channel_enabled(id, c, false).unwrap();
            }
        }
    }
    let t = Instant::now();
    app.sync_effect_inputs();
    let first_feed = ms(t);
    let since = 0;
    let t = Instant::now();
    composite_changed(&app, since);
    let first_composite = ms(t);
    println!("first-feed\t{first_feed:.1}\t{first_feed:.1}");
    println!("first-composite\t{first_composite:.1}\t{first_composite:.1}");
    let (mut feed, mut comp) = (Vec::new(), Vec::new());
    let mut tiles = 0;
    for run in 0..runs {
        // B に描く（今のセットを B にして、ストロークを終える）
        app.switch_set(b).unwrap();
        let brush = BrushSettings {
            radius: 24.0,
            hardness: 0.8,
            spacing: 0.1,
            color: Rgba8::new(200, 60, 30, 255),
            ..BrushSettings::default()
        };
        let doc = &mut app.session.doc;
        let mut s = doc.begin_stroke(raster, &brush).unwrap();
        for i in 0..points {
            let t = i as f64 / points.max(2) as f64;
            let x = size as f64 * (0.1 + 0.3 * t);
            let y = size as f64 * (0.2 + 0.1 * run as f64 + 0.05 * (t * 6.0).sin());
            s.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
        }
        doc.end_stroke(s).unwrap();
        app.switch_set(0).unwrap();
        let since = app.session.doc.change_serial();
        let t = Instant::now();
        app.sync_effect_inputs();
        feed.push(ms(t));
        let t = Instant::now();
        tiles = composite_changed(&app, since);
        comp.push(ms(t));
    }
    let (fp, fm) = stats(&mut feed);
    let (cp, cm) = stats(&mut comp);
    println!("feed\t{fp:.1}\t{fm:.1}");
    println!("composite\t{cp:.1}\t{cm:.1}\t({tiles} tiles)");
}
