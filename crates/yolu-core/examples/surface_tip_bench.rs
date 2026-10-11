//! 3D のストロークの 1 ダブの時間（丸い筆先と画像の筆先）。
//!   cargo run --release -p yolu-core --example surface_tip_bench [回数]
//! 立方体を 76 × 76 に分けて膨らませた球（69,312 三角形）を斜めから見て、文書 2048² と 4096² に、画面の上の同じ道筋を
//! 描く（ダブの数は間隔で決まる）。ブラシは同梱の「丸」（硬さ 0.8）と「かすれ筆」（画像の筆先・線の向き・不透明度のゆらぎ）と、
//! 丸と同じ設定で筆先の画像だけ「角の丸い四角」にしたもの。ストロークの始め（投影の準備と
//! 最初のダブ）を除いた、2 つ目からのダブの時間の平均（道筋が進んで作る区画も含む）を、回数の中央値で出す。

use std::sync::Arc;
use std::time::Instant;

use yolu_core::geometry::{
    cube_sphere, model_triangles, OrbitCamera, SurfaceGeometry, SurfaceStroke,
    SurfaceStrokeOptions, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::Vec2;
use yolu_core::{builtin_presets, builtin_tip, Brush, BrushSettings, Document, Rgba8};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let repeat: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let g = Arc::new(
        SurfaceGeometry::new(
            model_triangles(&[cube_sphere(76, 0.5)]).unwrap(),
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    );
    let view = OrbitCamera::framing(&g.bounds()).view(1200.0, 900.0);
    let round = Brush::from(BrushSettings {
        hardness: 0.8,
        color: Rgba8::new(30, 90, 200, 255),
        ..BrushSettings::default()
    });
    let dry = builtin_presets()
        .into_iter()
        .find(|p| p.id == "dry-brush")
        .expect("かすれ筆")
        .brush;
    // 画像の筆先で、丸とほぼ同じ所を覆うもの（画素ごとの式の重さを丸と比べる）
    let mut square = round.clone();
    square.tip.image = builtin_tip("rounded-square");
    for size in [2048u32, 4096] {
        for (name, brush) in [
            ("丸", &round),
            ("かすれ筆", &dry),
            ("角の丸い四角", &square),
        ] {
            // 画面の上の直径をそろえる（文書の大きさに比例させる）
            let mut brush = brush.clone();
            brush.base.radius = 24.0 * size as f64 / 2048.0;
            let mut per = Vec::new();
            let mut dabs = 0;
            for _ in 0..repeat {
                let mut d = Document::new(size, size).unwrap();
                let l = d.add_layer("p").unwrap();
                let mut stroke = d.begin_brush_stroke(l, &brush).unwrap();
                let start = Vec2::new(450.0, 380.0);
                let mut s = SurfaceStroke::begin_with_options(
                    &mut d,
                    &mut stroke,
                    g.clone(),
                    view,
                    &brush.base,
                    Some(0),
                    start,
                    1.0,
                    SurfaceStrokeOptions::default(),
                )
                .unwrap();
                let first = s.stats.dabs;
                let clock = Instant::now();
                for i in 1..=24 {
                    let t = i as f32 / 24.0;
                    let p = start + Vec2::new(300.0 * t, 140.0 * (t * 3.0).sin());
                    s.add(&mut d, &mut stroke, p, 1.0).unwrap();
                }
                s.finish(&mut d, &mut stroke).unwrap();
                let elapsed = clock.elapsed().as_secs_f64() * 1000.0;
                dabs = s.stats.dabs - first;
                per.push(elapsed / dabs.max(1) as f64);
                d.end_stroke(stroke).unwrap();
            }
            println!(
                "{size}² {name}: 1 ダブ {:.3} ms（{dabs} ダブ、{repeat} 回の中央値）",
                median(per)
            );
        }
    }
}
