//! デュアルブラシの足した項目の 3D: カメラに正面を向けた平らな四角に、2D と同じブラシ・同じ点の列で描くと、足した合成モード 5 種・
//! サイズの連動・濃度・影響元（筆圧・ランダム）・向きの足しが 2D と同じ覆いになること（合わせは 2D と同じ `combine32`、2 つ目のダブの
//! 見え方は 2D と同じ関数・同じ乱数の列）。3D は画面から面への写しで画素の縁が少し動くので、違う画素が少ないことを見る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeError, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, AntiAlias, Brush, BrushSettings, Document, DualBrush, DualBrushMode,
    InputResponse, InputUse, LayerId, PressureSwitches, Rgba8,
};

use crate::anti_alias::{bytes, document, front, quad, screen, W};

/// 半径 9 の柔らかい丸に、画像の 2 つ目を入れたもの。
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
        tip: builtin_tip("dots"),
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

fn draw_2d(brush: &Brush, pressure: f64) -> Vec<u8> {
    let (mut d, l) = document(1);
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for (x, y) in points() {
        s.add_point(&mut d, x, y, pressure, DVec2::ZERO).unwrap();
    }
    d.end_stroke(s).unwrap();
    bytes(&d, l)
}

fn draw_3d(brush: &Brush, pressure: f64) -> Vec<u8> {
    let g = quad();
    let mut b = brush.clone();
    // ブラシと 2 つ目の半径（文書の画素）を、四角の上で 2D と同じ大きさになるように直す
    let scale = W as f64 / g.brush_scale() as f64;
    b.base.radius *= scale;
    if let Some(d) = b.dual.as_mut() {
        d.radius *= scale;
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

fn assert_close(brush: &Brush, pressure: f64, what: &str) {
    let (painted, differ) = compare(&draw_2d(brush, pressure), &draw_3d(brush, pressure));
    println!("{what}: 塗った画素 {painted}・違う画素 {differ}");
    assert!(painted > 100, "{what}: テストの前提 ({painted})");
    assert!(differ * 50 <= painted, "{what}: {differ}/{painted}");
}

#[test]
fn every_added_dual_brush_mode_matches_2d() {
    for mode in DualBrushMode::EXTENDED {
        let b = dualed(DualBrush {
            mode,
            ..DualBrush::default()
        });
        assert_close(&b, 1.0, &format!("{mode:?}"));
    }
}

#[test]
fn the_link_density_sources_and_angle_match_2d() {
    let mut b = dualed(DualBrush {
        link_size: true,
        flow: 0.7,
        roundness: 0.5,
        angle_line: true,
        angle_random: 0.4,
        ..DualBrush::default()
    });
    // 2 つ目は潰した丸（向きと厚さが形に出る）
    b.dual.as_mut().unwrap().tip = None;
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
    assert_close(&b, 0.8, "連動・濃度・影響元・向き");
    // 別の種の 2D（別の乱数の列）は、ずっと多く違う
    let mut other = b.clone();
    other.seed = 5;
    let (_, unlike) = compare(&draw_2d(&b, 0.8), &draw_2d(&other, 0.8));
    let (_, same) = compare(&draw_2d(&b, 0.8), &draw_3d(&b, 0.8));
    assert!(
        same * 3 < unlike,
        "2D と 3D の違い {same}・別の種の違い {unlike}"
    );
}

fn pressure_use() -> InputUse {
    InputUse {
        on: true,
        response: InputResponse::new(0.0, vec![]).unwrap(),
    }
}

/// 3D の 1 本のストローク（`draw_3d` と同じ設定・半径の直し）を、渡した文書へ描く。失敗したらストロークを取り消して、そのエラーを返す。
fn try_3d(
    d: &mut Document,
    l: LayerId,
    brush: &Brush,
    points: &[(f64, f64)],
    pressure: f64,
) -> Result<(), SurfaceStrokeError> {
    let g = quad();
    let mut b = brush.clone();
    let scale = W as f64 / g.brush_scale() as f64;
    b.base.radius *= scale;
    if let Some(dual) = b.dual.as_mut() {
        dual.radius *= scale;
    }
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points[0];
    let result = (|| {
        let mut s = SurfaceStroke::begin_with_options(
            d,
            &mut stroke,
            g.clone(),
            front(),
            &b.base,
            Some(0),
            screen(x0, y0),
            pressure as f32,
            SurfaceStrokeOptions::default(),
        )?;
        for &(x, y) in &points[1..] {
            s.add(d, &mut stroke, screen(x, y), pressure as f32)?;
        }
        s.finish(d, &mut stroke)
    })();
    match result {
        Ok(()) => {
            d.end_stroke(stroke).unwrap();
            Ok(())
        }
        Err(e) => {
            d.cancel_stroke(stroke);
            Err(e)
        }
    }
}

#[test]
fn a_spacing_source_that_shrinks_the_step_past_the_dab_limit_is_refused_while_placing() {
    // 2 つ目の間隔は画面で 0.5（下限。直径 × 0.01 が 0.5 に届かない）。区間は 16000 で、そのままなら 32000 個（上限 65536 の内）。筆圧 0 の間隔の係数は下限 0.01 で、
    // 3200000 個になる
    let mut b = dualed(DualBrush {
        spacing: 0.01,
        ..DualBrush::default()
    });
    b.influences.dual_spacing.pressure = pressure_use();
    let far = [(14.0, 24.0), (16014.0, 24.0)];

    let (mut d, l) = document(1);
    d.clear_history().unwrap();
    let before = bytes(&d, l);
    let undo = d.undo_count();
    let refused = try_3d(&mut d, l, &b, &far, 0.0);
    assert!(
        matches!(refused, Err(SurfaceStrokeError::TooManyDabs)),
        "{refused:?}"
    );
    // 取り消した後の文書は描く前のまま（途中の主・2 つ目のダブは残らない）。次のストロークは普通に描ける
    assert_eq!(bytes(&d, l), before);
    assert_eq!(d.undo_count(), undo);
    let plain = dualed(DualBrush::default());
    try_3d(&mut d, l, &plain, &points(), 1.0).unwrap();
    assert_eq!(
        bytes(&d, l),
        draw_3d(&plain, 1.0),
        "断りの後の次のストローク"
    );

    // 対照: 筆圧 1 は間隔の係数が 1 のままで、同じ区間は上限に収まる
    let (mut d, l) = document(1);
    try_3d(&mut d, l, &b, &far, 1.0).unwrap();
}

#[test]
fn a_floored_main_dab_keeps_its_one_pixel_line_under_a_linked_multiply() {
    for mode in [DualBrushMode::Multiply, DualBrushMode::LinearHeight] {
        let make = |link: bool| {
            let mut b = dualed(DualBrush {
                mode,
                link_size: link,
                hardness: 1.0,
                ..DualBrush::default()
            });
            b.dual.as_mut().unwrap().tip = None;
            b.influences.size.pressure = pressure_use();
            b.controls.min_one_pixel = true;
            b
        };
        let one = |b: &Brush| {
            let (mut d, l) = document(1);
            // 画素の中心に 1 点（筆圧 0）
            try_3d(&mut d, l, b, &[(48.5, 32.5), (48.5, 32.5)], 0.0).unwrap();
            bytes(&d, l).chunks_exact(4).filter(|p| p[3] != 0).count()
        };
        // 連動した 2 つ目は半径 6 × (0.5 / 9) まで細るが、0 にはならない: 主の下限の線が残る
        let linked = one(&make(true));
        let unlinked = one(&make(false));
        assert!(linked > 0, "{mode:?}: {linked}");
        assert!(unlinked > 0, "{mode:?}: {unlinked}");
    }
}

#[test]
fn a_secondary_with_a_vanishing_radius_neither_panics_nor_poisons_the_pixels() {
    // 2 つ目のサイズの影響元（筆圧）で、2 つ目の半径が 0 より大きく 0.5 未満になる。合わせは加算なので、2 つ目の覆いが NaN になれば
    // 主の画素まで消える。筆圧 0（2 つ目の半径 0 で置かない）と同じバイトなら、何も混ざっていない
    for tip in [None, builtin_tip("dots")] {
        for anti_alias in [None, Some(AntiAlias::Weak), Some(AntiAlias::Strong)] {
            for link in [false, true] {
                let mut b = dualed(DualBrush {
                    mode: DualBrushMode::Add,
                    tip: tip.clone(),
                    anti_alias,
                    link_size: link,
                    scatter: 0.5,
                    roundness: 0.3,
                    angle: 40.0,
                    count: 2,
                    ..DualBrush::default()
                });
                b.influences.dual_size.pressure = pressure_use();
                let none = draw_3d(&b, 0.0);
                assert!(
                    none.chunks_exact(4).any(|p| p[3] != 0),
                    "テストの前提: 主は描ける"
                );
                for pressure in [1e-7, 1e-9] {
                    assert_eq!(
                        draw_3d(&b, pressure),
                        none,
                        "{tip:?}・{anti_alias:?}・{link}・{pressure}"
                    );
                }
            }
        }
    }
}
