//! デュアルブラシの 2 つ目の複数の筆先・反転・紙の質感・散布効果の 3D: カメラに正面を向けた平らな四角に、2D と同じブラシ・同じ点の列で
//! 描くと、2D と同じ覆いになること（筆先の選び・反転・粒子は 2D と同じ関数・同じ乱数の列、紙の質感は 2D と同じくテクセルの文書の画素で
//! 読む）。3D は画面から面への写しで画素の縁が少し動くので、違う画素が少ないことを見る（今のデュアルの比べ `dual_brush.rs` と同じ基準）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, Brush, BrushSettings, DualBrush, FlipRule, InputResponse, InputUse, PaperTexture,
    PressureSwitches, Rgba8, Spray, SprayDirection, TextureMode, TipSelection,
};

use crate::anti_alias::{bytes, document, front, quad, screen, W};

/// 半径 9 の柔らかい丸に、2 つ目を入れたもの（`dual_brush.rs` と同じ主）。
fn dualed(dual: DualBrush) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 9.0,
            hardness: 0.6,
            spacing: 0.25,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(120, 30, 200, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.dual = Some(DualBrush {
        radius: 8.0,
        spacing: 0.25,
        hardness: 0.5,
        ..dual
    });
    b.seed = 4;
    b
}

fn points() -> Vec<(f64, f64)> {
    (0..5)
        .map(|i| (14.0 + 17.0 * i as f64, 24.0 + 4.0 * i as f64))
        .collect()
}

fn draw_2d_at(brush: &Brush, pressure: f64) -> Vec<u8> {
    let (mut d, l) = document(1);
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for (x, y) in points() {
        s.add_point(&mut d, x, y, pressure, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    bytes(&d, l)
}

fn draw_3d_at(brush: &Brush, pressure: f64) -> Vec<u8> {
    let g = quad();
    let mut b = brush.clone();
    // ブラシ・2 つ目の半径と 2 つ目の粒子サイズ（どれも文書の画素）を、四角の上で 2D と同じ大きさになるように直す。紙の質感は
    // 文書の画素で読むので直さない
    let scale = W as f64 / g.brush_scale() as f64;
    b.base.radius *= scale;
    if let Some(d) = b.dual.as_mut() {
        d.radius *= scale;
        if let Some(s) = d.spray.as_mut() {
            s.size *= scale;
        }
    }
    let (mut d, l) = document(1);
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points()[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(0),
        screen(x0, y0),
        pressure as f32,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &(x, y) in &points()[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), pressure as f32)
            .unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// 塗った画素の数・アルファの差が 8 を超える画素の数。
fn compare(a: &[u8], b: &[u8]) -> (usize, usize) {
    let mut painted = 0;
    let mut differ = 0;
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if p[3] > 0 || q[3] > 0 {
            painted += 1;
        }
        if p[3].abs_diff(q[3]) > 8 {
            differ += 1;
        }
    }
    (painted, differ)
}

/// 筆圧 PRESSURE の線で、2D と 3D の違う画素が、今のデュアルの比べと同じ基準（塗った画素の 2%）に収まる。違いの数を返す。
fn assert_close(brush: &Brush, what: &str) -> usize {
    let (painted, differ) = compare(&draw_2d_at(brush, PRESSURE), &draw_3d_at(brush, PRESSURE));
    println!("{what}: 塗った画素 {painted}・違う画素 {differ}");
    assert!(painted > 100, "{what}: テストの前提 ({painted})");
    assert!(differ * 50 <= painted, "{what}: {differ}/{painted}");
    differ
}

/// 別の種の 2D とは、2D と 3D の違いよりずっと多く違う（乱数の列が効いている）。
fn assert_seeded(brush: &Brush, same: usize, what: &str) {
    let mut other = brush.clone();
    other.seed = 5;
    let (_, unlike) = compare(&draw_2d_at(brush, PRESSURE), &draw_2d_at(&other, PRESSURE));
    assert!(
        same * 3 < unlike,
        "{what}: 2D と 3D の違い {same}・別の種の違い {unlike}"
    );
}

const PRESSURE: f64 = 0.8;

/// 今のデュアルの比べ（`dual_brush.rs` の連動・濃度・影響元・向き）と同じ 2 つ目と影響元に、足した項目 extra を入れたもの。
fn with_new(extra: DualBrush) -> Brush {
    let mut b = dualed(DualBrush {
        link_size: true,
        flow: 0.7,
        roundness: 0.5,
        angle_line: true,
        angle_random: 0.4,
        ..extra
    });
    b.influences.size.pressure = InputUse::STRAIGHT;
    b.influences.dual_size.pressure = InputUse {
        on: true,
        response: InputResponse::new(0.5, vec![]).unwrap(),
    };
    b.influences.dual_spacing.random = InputUse {
        on: true,
        response: InputResponse::new(0.5, vec![]).unwrap(),
    };
    b.influences.dual_thickness.random = InputUse::STRAIGHT;
    b
}

fn two_tips() -> Vec<std::sync::Arc<yolu_core::BrushTip>> {
    vec![
        builtin_tip("dots").unwrap(),
        builtin_tip("rounded-square").unwrap(),
    ]
}

fn grain() -> PaperTexture {
    PaperTexture::new(builtin_tip("grain").unwrap(), 0.8)
}

#[test]
fn several_secondary_tips_and_their_flips_match_2d() {
    for (selection, x_rule, y_rule) in [
        (TipSelection::Sequential, FlipRule::Fixed, FlipRule::Fixed),
        (TipSelection::PingPong, FlipRule::OnFold, FlipRule::Random),
        (TipSelection::Random, FlipRule::Random, FlipRule::Fixed),
        (TipSelection::Hold, FlipRule::Fixed, FlipRule::Random),
    ] {
        let b = with_new(DualBrush {
            tips: two_tips(),
            selection,
            flip_x_rule: x_rule,
            flip_y_rule: y_rule,
            ..DualBrush::default()
        });
        let what = format!("複数の筆先 {selection:?} {x_rule:?}/{y_rule:?}");
        let same = assert_close(&b, &what);
        if selection == TipSelection::Random {
            assert_seeded(&b, same, &what);
        }
    }
}

#[test]
fn the_secondary_texture_matches_2d() {
    for (mode, angle) in [
        (TextureMode::Multiply, 0.0),
        (TextureMode::Overlay, 20.0),
        (TextureMode::Subtract, 0.0),
    ] {
        let b = with_new(DualBrush {
            texture: Some(PaperTexture {
                mode,
                angle,
                scale: 0.8,
                ..grain()
            }),
            ..DualBrush::default()
        });
        assert_close(&b, &format!("紙の質感 {mode:?}"));
    }
}

#[test]
fn the_secondary_spray_matches_2d() {
    let b = with_new(DualBrush {
        spray: Some(Spray {
            size: 6.0,
            link_size: true,
            density: 6.0,
            bias: 0.3,
            direction: SprayDirection::Line,
            random_angle: Some(0.5),
            ..Spray::default()
        }),
        ..DualBrush::default()
    });
    let same = assert_close(&b, "散布効果");
    assert_seeded(&b, same, "散布効果");
}

#[test]
fn every_new_item_together_matches_2d() {
    let b = with_new(DualBrush {
        tips: two_tips(),
        selection: TipSelection::PingPong,
        flip_x_rule: FlipRule::OnFold,
        darken_overlap: true,
        texture: Some(grain()),
        spray: Some(Spray {
            size: 6.0,
            density: 3.0,
            ..Spray::default()
        }),
        ..DualBrush::default()
    });
    assert_close(&b, "全部");
}
