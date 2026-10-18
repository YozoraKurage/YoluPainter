//! 散布効果（`Brush::spray` と、`Influences` の粒子サイズ・粒子密度）の 2D のストローク: 粒子がブラシの届く円の中にだけ置かれること、
//! 散布の間は散布（`Jitter::scatter`）と数（`Jitter::count`）を使わないこと、粒子の数が 0 なら置かないこと、Undo・Redo、範囲の外の断り。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, CoreError, Document, InputResponse, InputUse,
    PressureSwitches, Rgba8, Spray,
};

const SIDE: u32 = 128;
const CENTER: f64 = 64.0;
const RADIUS: f64 = 20.0;

/// 半径 20 の硬い丸に、粒子サイズ 2 の散布を入れたもの。
fn sprayed(density: f64) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: RADIUS,
            hardness: 1.0,
            spacing: 0.25,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(30, 60, 200, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.spray = Some(Spray {
        size: 2.0,
        density,
        ..Spray::default()
    });
    b.seed = 5;
    b
}

/// 真ん中に 1 点だけ置いたストロークの画素と、文書（Undo のため）。
fn dot(brush: &Brush, pressure: f64) -> (Document, Vec<u8>) {
    let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    s.add_sample(
        &mut d,
        BrushSample::new(CENTER, CENTER, pressure, 0.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    d.end_stroke(s).unwrap();
    let pixels = d.composite(d.bounds()).unwrap();
    (d, pixels)
}

/// 塗った画素の、真ん中からの距離（画素の中心）。
fn painted_distances(pixels: &[u8]) -> Vec<f64> {
    pixels
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[3] != 0)
        .map(|(i, _)| {
            let (x, y) = (
                (i as u32 % SIDE) as f64 + 0.5,
                (i as u32 / SIDE) as f64 + 0.5,
            );
            (x - CENTER).hypot(y - CENTER)
        })
        .collect()
}

#[test]
fn particles_land_only_inside_the_brush_circle_and_replace_scatter_and_count() {
    let (_, plain) = dot(&sprayed(40.0), 1.0);
    let far = painted_distances(&plain);
    assert!(far.len() > 40, "テストの前提: 粒子が塗れる {}", far.len());
    // 粒子の中心は半径 20 の円の中、粒子の半径は 1（画素の中心で測るので 1 の余白）
    assert!(far.iter().all(|&r| r <= RADIUS + 2.0), "{far:?}");
    // 1 つのダブで塗るより、ずっと少ない（散らした粒子）
    assert!(far.len() < 900, "{}", far.len());
    // 散布の間は、散布と数のゆらぎを使わない（同じバイト）
    let mut wide = sprayed(40.0);
    wide.jitter.scatter = 10.0;
    wide.jitter.count = 16;
    assert_eq!(dot(&wide, 1.0).1, plain);
    // 種が違えば違う所に置く
    let mut other = sprayed(40.0);
    other.seed = 6;
    assert_ne!(dot(&other, 1.0).1, plain);
}

#[test]
fn no_particles_are_placed_when_the_count_rounds_to_zero() {
    let mut b = sprayed(10.0);
    b.influences.spray_density.pressure = InputUse {
        on: true,
        response: InputResponse::new(0.0, vec![]).unwrap(),
    };
    let (_, pixels) = dot(&b, 0.01);
    assert!(pixels.chunks_exact(4).all(|p| p[3] == 0), "0.1 個は 0 個");
    let (_, pixels) = dot(&b, 1.0);
    assert!(pixels.chunks_exact(4).any(|p| p[3] != 0), "筆圧 1 なら置く");
}

#[test]
fn undo_and_redo_of_a_sprayed_stroke_restore_the_exact_bytes() {
    let mut b = sprayed(20.0);
    b.influences.spray_size.random = InputUse {
        on: true,
        response: InputResponse::new(0.3, vec![]).unwrap(),
    };
    let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    let before = d.composite(d.bounds()).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for i in 0..8 {
        let x = 24.0 + 10.0 * i as f64;
        s.add_sample(
            &mut d,
            BrushSample::new(x, CENTER, 1.0, 0.0, DVec2::ZERO).unwrap(),
        )
        .unwrap();
    }
    d.end_stroke(s).unwrap();
    let drawn = d.composite(d.bounds()).unwrap();
    assert_ne!(drawn, before, "テストの前提: 描けている");
    assert!(d.undo().unwrap());
    assert_eq!(d.composite(d.bounds()).unwrap(), before, "Undo で描く前");
    assert!(d.redo().unwrap());
    assert_eq!(d.composite(d.bounds()).unwrap(), drawn, "Redo で描いた後");
}

#[test]
fn a_stroke_with_an_out_of_range_spray_is_refused() {
    for bad in [
        Spray {
            density: 0.0,
            ..Spray::default()
        },
        Spray {
            size: f64::NAN,
            ..Spray::default()
        },
        Spray {
            bias: 2.0,
            ..Spray::default()
        },
    ] {
        let mut b = sprayed(5.0);
        b.spray = Some(bad);
        let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
        let l = d.add_layer("L").unwrap();
        let Err(err) = d.begin_brush_stroke(l, &b) else {
            panic!("断る: {bad:?}");
        };
        assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    }
}
