//! デュアルブラシの 2 つ目の複数の筆先・繰り返し方法・反転・比較暗・紙の質感・散布効果の 2D のストローク: 2 つ目に効くこと、
//! 決まった種で同じバイト、紙の質感が溜まりに効くこと（式が主の天井と同じことは core の中のテスト）、散布の間は旧形の散布・数を使わないこと、Undo・Redo、範囲の外と NaN と
//! 枚数の上限の断り。筆先の選び・反転・粒子の並びが主と同じ関数を通ることは、core の中のテスト（`brush::dual`）が見る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use std::sync::Arc;

use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushSample, BrushSettings, BrushTip, CanvasSymmetry, CoreError, Document,
    DualBrush, DualBrushMode, FlipRule, PaperTexture, PressureSwitches, Reason, Rgba8, Spray,
    SprayDirection, SymmetryMode, TextureMode, TipSelection,
};

const SIDE: u32 = 128;
const CENTER: f64 = 64.0;

/// 半径 20 の硬い丸に、半径 6 の硬い丸の 2 つ目（乗算: 塗れるのは 2 つ目の溜まりの所だけ）を入れたもの。
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

fn sample(x: f64, y: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, 0.0, DVec2::ZERO).unwrap()
}

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

/// 横の線（x = 16〜112）。
fn line() -> Vec<BrushSample> {
    (0..9)
        .map(|i| sample(16.0 + 12.0 * i as f64, CENTER))
        .collect()
}

fn stroke(brush: &Brush) -> Vec<u8> {
    draw(brush, &line()).1
}

fn dot(brush: &Brush) -> Vec<u8> {
    draw(brush, &[sample(CENTER, CENTER)]).1
}

fn painted(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
}

fn alpha(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIDE + x) * 4 + 3) as usize]
}

/// 左の半分だけが塗れる 8 × 8 の筆先（左右の反転が見える）。
fn left_half() -> Arc<BrushTip> {
    let alpha = (0..64).map(|i| if i % 8 < 4 { 255 } else { 0 }).collect();
    Arc::new(BrushTip::new("left", 8, 8, alpha).unwrap())
}

/// 塗った画素の x の平均（左右の寄り）。
fn mean_x(pixels: &[u8]) -> f64 {
    let (mut sum, mut n) = (0.0, 0.0);
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[3] != 0 {
            sum += (i as u32 % SIDE) as f64;
            n += 1.0;
        }
    }
    sum / n
}

fn grain() -> PaperTexture {
    PaperTexture::new(builtin_tip("grain").unwrap(), 1.0)
}

fn spray() -> Spray {
    Spray {
        size: 4.0,
        density: 5.0,
        bias: 0.2,
        direction: SprayDirection::Line,
        random_angle: Some(0.4),
        ..Spray::default()
    }
}

#[test]
fn several_secondary_tips_change_the_stroke_and_repeat_with_the_same_seed() {
    let tips = vec![builtin_tip("dots").unwrap(), builtin_tip("rim").unwrap()];
    let one = stroke(&dualed(DualBrush {
        tip: Some(tips[0].clone()),
        ..DualBrush::default()
    }));
    for selection in [TipSelection::Sequential, TipSelection::Random] {
        let b = dualed(DualBrush {
            tips: tips.clone(),
            selection,
            ..DualBrush::default()
        });
        let a = stroke(&b);
        assert!(painted(&a) > 0, "{selection:?}");
        assert_ne!(a, one, "{selection:?}: 2 枚目の筆先も使う");
        assert_eq!(stroke(&b), a, "{selection:?}: 同じ種で同じバイト");
    }
    // 1 枚の並びを順に使うのは、その 1 枚を筆先に置いたのと同じ画素
    let listed = stroke(&dualed(DualBrush {
        tips: vec![tips[0].clone()],
        selection: TipSelection::Sequential,
        ..DualBrush::default()
    }));
    assert_eq!(listed, one);
}

#[test]
fn once_lays_down_the_secondary_only_until_the_list_runs_out() {
    // 一巡のみ・1 枚: 最初の 2 つ目のダブ（x = 16 の所、半径 6）の後は溜まりが増えないので、乗算の主はその外を塗らない
    let tip = builtin_tip("rounded-square").unwrap();
    let once = stroke(&dualed(DualBrush {
        tips: vec![tip.clone()],
        selection: TipSelection::Once,
        ..DualBrush::default()
    }));
    let all = stroke(&dualed(DualBrush {
        tips: vec![tip],
        selection: TipSelection::Sequential,
        ..DualBrush::default()
    }));
    assert!(painted(&once) > 0);
    for x in 40..SIDE {
        assert_eq!(alpha(&once, x, CENTER as u32), 0, "x = {x}");
    }
    assert!(alpha(&all, 80, CENTER as u32) > 0, "順に使えば線の全体");
}

#[test]
fn the_secondary_flips_mirror_its_tip() {
    let plain = dot(&dualed(DualBrush {
        tip: Some(left_half()),
        radius: 10.0,
        ..DualBrush::default()
    }));
    let flipped = dot(&dualed(DualBrush {
        tip: Some(left_half()),
        radius: 10.0,
        flip_x: true,
        ..DualBrush::default()
    }));
    assert!(painted(&plain) > 0 && painted(&flipped) > 0);
    // 点の中心は画素 63 と 64 の間。反転すると、塗れる半分が中心の反対の側へ移る（左右の寄りが同じ大きさで逆）
    let (a, b) = (mean_x(&plain) - 63.5, mean_x(&flipped) - 63.5);
    assert!(a.abs() > 2.0 && (a + b).abs() < 0.5, "{a} {b}");
    // ランダムな反転は、決まった種で同じバイト、反転しない線とは違う
    let random = dualed(DualBrush {
        tip: Some(left_half()),
        flip_x_rule: FlipRule::Random,
        ..DualBrush::default()
    });
    let a = stroke(&random);
    assert_eq!(stroke(&random), a);
    let fixed = stroke(&dualed(DualBrush {
        tip: Some(left_half()),
        ..DualBrush::default()
    }));
    assert_ne!(a, fixed);
}

#[test]
fn the_darken_flag_keeps_the_pixels_because_the_pool_keeps_the_larger_coverage() {
    let base = DualBrush {
        tip: builtin_tip("dots"),
        spacing: 0.1,
        ..DualBrush::default()
    };
    let on = stroke(&dualed(DualBrush {
        darken_overlap: true,
        ..base.clone()
    }));
    assert_eq!(on, stroke(&dualed(base)));
}

#[test]
fn the_secondary_texture_masks_the_stroke_and_no_depth_is_no_texture() {
    let base = DualBrush {
        radius: 30.0,
        ..DualBrush::default()
    };
    let plain = stroke(&dualed(base.clone()));
    let textured = stroke(&dualed(DualBrush {
        texture: Some(grain()),
        ..base.clone()
    }));
    assert_ne!(plain, textured);
    assert!(painted(&textured) > 0);
    let none = stroke(&dualed(DualBrush {
        texture: Some(PaperTexture {
            depth: 0.0,
            ..grain()
        }),
        ..base
    }));
    assert_eq!(none, plain, "深さ 0 は質感なしと同じ");
}

#[test]
fn the_secondary_texture_also_reaches_the_symmetric_copies() {
    let mut b = dualed(DualBrush {
        radius: 30.0,
        texture: Some(grain()),
        ..DualBrush::default()
    });
    b.symmetry =
        CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(CENTER, CENTER), 2).unwrap();
    let points = [sample(30.0, 40.0), sample(40.0, 50.0)];
    let textured = draw(&b, &points).1;
    b.dual.as_mut().unwrap().texture = None;
    let plain = draw(&b, &points).1;
    // 写しの側（右半分）も質感で削れる
    let right = |p: &[u8]| -> u32 {
        p.chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| *i as u32 % SIDE >= CENTER as u32)
            .map(|(_, p)| u32::from(p[3]))
            .sum()
    };
    assert!(right(&plain) > 0);
    assert!(right(&textured) < right(&plain));
}

#[test]
fn the_secondary_spray_replaces_the_old_scatter_and_count() {
    let sprayed = DualBrush {
        tip: builtin_tip("dots"),
        spray: Some(spray()),
        ..DualBrush::default()
    };
    let a = stroke(&dualed(sprayed.clone()));
    assert!(painted(&a) > 0);
    assert_eq!(stroke(&dualed(sprayed.clone())), a, "同じ種で同じバイト");
    // 散布の間は、旧形の散布・数は効かない
    let old = stroke(&dualed(DualBrush {
        scatter: 3.0,
        count: 4,
        ..sprayed.clone()
    }));
    assert_eq!(old, a);
    // 散布の無い 2 つ目とは違う
    let plain = stroke(&dualed(DualBrush {
        spray: None,
        ..sprayed
    }));
    assert_ne!(plain, a);
}

#[test]
fn undo_and_redo_of_a_stroke_with_every_new_item_restore_the_exact_bytes() {
    let b = dualed(DualBrush {
        tips: vec![builtin_tip("dots").unwrap(), left_half()],
        selection: TipSelection::PingPong,
        flip_x_rule: FlipRule::OnFold,
        flip_y_rule: FlipRule::Random,
        darken_overlap: true,
        texture: Some(PaperTexture {
            mode: TextureMode::Overlay,
            ..grain()
        }),
        spray: Some(spray()),
        mode: DualBrushMode::Screen,
        ..DualBrush::default()
    });
    let (mut d, drawn) = draw(&b, &line());
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
fn a_stroke_with_out_of_range_new_items_is_refused() {
    let tip = builtin_tip("dots").unwrap();
    for (bad, reason) in [
        (
            DualBrush {
                tips: vec![tip.clone(); 257],
                ..DualBrush::default()
            },
            Reason::DualBrushTips,
        ),
        (
            DualBrush {
                texture: Some(PaperTexture {
                    depth: f64::NAN,
                    ..grain()
                }),
                ..DualBrush::default()
            },
            Reason::TextureDepth,
        ),
        (
            DualBrush {
                texture: Some(PaperTexture {
                    depth: 1.5,
                    ..grain()
                }),
                ..DualBrush::default()
            },
            Reason::TextureDepthRange,
        ),
        (
            DualBrush {
                texture: Some(PaperTexture {
                    scale: 100.0,
                    ..grain()
                }),
                ..DualBrush::default()
            },
            Reason::TextureScaleRange,
        ),
        (
            DualBrush {
                spray: Some(Spray {
                    density: 0.0,
                    ..spray()
                }),
                ..DualBrush::default()
            },
            Reason::SprayRange,
        ),
        (
            DualBrush {
                spray: Some(Spray {
                    size: f64::NAN,
                    ..spray()
                }),
                ..DualBrush::default()
            },
            Reason::Spray,
        ),
    ] {
        let b = dualed(bad.clone());
        let mut d = Document::with_tile_size(SIDE, SIDE, 32).unwrap();
        let l = d.add_layer("L").unwrap();
        let Err(err) = d.begin_brush_stroke(l, &b) else {
            panic!("断る: {reason:?}");
        };
        assert!(
            matches!(&err, CoreError::InvalidArgument(r) if *r == reason),
            "{reason:?}: {err:?}"
        );
    }
    // 256 枚は受ける
    let b = dualed(DualBrush {
        tips: vec![tip; 256],
        ..DualBrush::default()
    });
    b.validate().unwrap();
}
