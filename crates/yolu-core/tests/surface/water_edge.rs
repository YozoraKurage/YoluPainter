//! 水彩境界の 3D: 幅は画面の点で、最初の面のダブの中心のテクセルの画面の大きさでテクセルへ直す。カメラに正面を向けた平らな四角
//! （UV が文書の全体。世界の 1 単位が画面の 1 点）で、同じブラシ・同じ点の列を 2D と 3D に描いて比べる（テクスチャの解像度を 2 倍に
//! すると、テクセルの帯の幅も 2 倍）。ドラッグ後に処理の縁と、取り消し。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSettings, ColorMix, MixMode, Rgba8, WaterEdge};

use crate::anti_alias::{bytes, document, front, quad, screen};

fn brush(edge: Option<WaterEdge>) -> Brush {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 6.0,
            hardness: 1.0,
            spacing: 0.0625,
            color: Rgba8::new(20, 40, 160, 255),
            ..BrushSettings::default()
        },
        yolu_core::PressureSwitches::NONE,
    );
    b.water_edge = edge;
    b
}

fn edge(width: f64, after_stroke: bool) -> WaterEdge {
    WaterEdge {
        width,
        opacity: 0.3,
        darkness: 0.8,
        after_stroke,
        blur: 0.0,
    }
}

const LINE: [(f64, f64); 3] = [(20.25, 18.5), (50.0, 30.25), (74.5, 44.0)];

fn draw_2d(b: &Brush, scale: u32) -> Vec<u8> {
    let s = scale as f64;
    let mut b = b.clone();
    b.base.radius *= s;
    let (mut d, l) = document(scale);
    let mut st = d.begin_brush_stroke(l, &b).unwrap();
    for &(x, y) in &LINE {
        st.add_point(&mut d, x * s, y * s, 1.0, DVec2::ZERO)
            .unwrap();
    }
    d.end_stroke(st).unwrap();
    bytes(&d, l)
}

/// 3D に描く。`cancel` なら確定せずに取り消して、その後の画素を返す。
fn draw_3d(b: &Brush, scale: u32, cancel: bool) -> Vec<u8> {
    draw_3d_on(b, scale, cancel, false)
}

/// 3D に描く（`ground` なら、先に 2D で別の色の線を下地に置く）。
fn draw_3d_on(b: &Brush, scale: u32, cancel: bool, ground: bool) -> Vec<u8> {
    let g = quad();
    let (mut d, l) = document(scale);
    if ground {
        let mut under = brush(None);
        under.base.color = Rgba8::new(210, 70, 20, 255);
        under.base.radius = 5.0 * scale as f64;
        let mut st = d.begin_brush_stroke(l, &under).unwrap();
        for (x, y) in [(18.0, 40.0), (76.0, 20.0)] {
            st.add_point(&mut d, x * scale as f64, y * scale as f64, 1.0, DVec2::ZERO)
                .unwrap();
        }
        d.end_stroke(st).unwrap();
    }
    let mut b = b.clone();
    b.base.radius *= d.width() as f64 / g.brush_scale() as f64;
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = LINE[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(&yolu_core::geometry::MaterialSet::one(0)),
        screen(x0, y0),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for &(x, y) in &LINE[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    if cancel {
        d.cancel_stroke(stroke);
    } else {
        d.end_stroke(stroke).unwrap();
    }
    bytes(&d, l)
}

/// 違う画素の数と、成分の差のいちばん大きい値。
fn differ(a: &[u8], b: &[u8]) -> (usize, u8) {
    let mut n = 0;
    let mut max = 0u8;
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = (0..4).map(|c| p[c].abs_diff(q[c])).max().unwrap();
        if d > 0 {
            n += 1;
            max = max.max(d);
        }
    }
    (n, max)
}

#[test]
fn the_3d_edge_matches_the_2d_edge_in_texels_of_the_screen_width() {
    // 1 倍: 画面の点 1 つがテクセル 1 つなので、2D と同じ幅
    let two = draw_2d(&brush(Some(edge(3.0, false))), 1);
    let three = draw_3d(&brush(Some(edge(3.0, false))), 1, false);
    assert_ne!(two, draw_2d(&brush(None), 1), "縁が付く");
    let (n, max) = differ(&two, &three);
    println!("1 倍: 違う画素 {n}・差の最大 {max}");
    assert_eq!((n, max), (0, 0), "1 倍: 2D と同じバイト");
    // 2 倍: テクセルの幅も 2 倍（2D では幅を 2 倍にした線と同じ）
    let two = draw_2d(&brush(Some(edge(6.0, false))), 2);
    let three = draw_3d(&brush(Some(edge(3.0, false))), 2, false);
    let (n, max) = differ(&two, &three);
    println!("2 倍: 違う画素 {n}・差の最大 {max}");
    assert_eq!((n, max), (0, 0), "2 倍: 幅を 2 倍にした 2D と同じバイト");
    assert_ne!(three, draw_3d(&brush(Some(edge(6.0, false))), 2, false));
}

#[test]
fn the_3d_after_stroke_edge_and_cancel() {
    let live = draw_3d(&brush(Some(edge(4.0, false))), 1, false);
    let after = draw_3d(&brush(Some(edge(4.0, true))), 1, false);
    assert_eq!(after, live, "ぼかし幅 0 の後処理は、描いている間の縁と同じ");
    for after_stroke in [false, true] {
        let cancelled = draw_3d(&brush(Some(edge(4.0, after_stroke))), 1, true);
        assert!(cancelled.iter().all(|&v| v == 0), "取り消しは何も残さない");
    }
}

#[test]
fn zero_powers_on_the_surface_paint_the_same_bytes_as_no_edge() {
    // 面のダブの塗り（`apply_dab`）・色の混ぜ（混ぜる・重ねるはダブの画素をまとめて、伸ばすは `apply_mapped_dab`）で、
    // 透明度影響・明度影響が 0 の縁は、水彩境界の無い線と同じバイト（縁の画素をダブの式と同じ合成で計算し直すこと）
    let quiet = |after_stroke| WaterEdge {
        width: 4.0,
        opacity: 0.0,
        darkness: 0.0,
        after_stroke,
        blur: if after_stroke { 2.0 } else { 0.0 },
    };
    let mixing = |mode| {
        let mut b = brush(None);
        b.base.hardness = 0.6;
        b.mix = ColorMix {
            mode,
            paint: 0.5,
            density: 0.7,
            stretch: 0.6,
            ..ColorMix::default()
        };
        b
    };
    for (label, b) in [
        ("硬い丸", brush(None)),
        ("混ぜる", mixing(MixMode::Mix)),
        ("重ねる", mixing(MixMode::Layer)),
        ("伸ばす", mixing(MixMode::Smear)),
    ] {
        let plain = draw_3d_on(&b, 1, false, true);
        for after_stroke in [false, true] {
            let mut e = b.clone();
            e.water_edge = Some(quiet(after_stroke));
            assert_eq!(
                draw_3d_on(&e, 1, false, true),
                plain,
                "{label} 後処理 {after_stroke}"
            );
        }
    }
}
