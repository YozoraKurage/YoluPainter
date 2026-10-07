//! 面の上の参照の写像（Unity 版の SurfaceBrushSamplingTests。値は C# の試験の期待値そのもの）: 展開の図が UV アイランドをまたいで画素を
//! 読み、クローン・指先が離れたアイランド・鏡映したアイランド・折れた面の間でも同じ模様を運ぶ。余白・別のスロット・つながらない辺・非多様体の辺は
//! 越えず、図の予算は部分の図を返さずに断る。
#![allow(clippy::chunks_exact_to_as_chunks)]

use yolu_core::geometry::{
    Ray, SamplingError, SurfaceBrushBudget, SurfaceGeometry, SurfaceHit, SurfacePixel,
    SurfaceTriangle,
};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::{
    Brush, BrushEffect, BrushMappedPixel, BrushPixel, BrushSettings, Channel, Document, LayerId,
    Rgba8,
};

const W: i32 = 32;
const H: i32 = 16;

fn p(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

/// 左右に 1 枚ずつの板（世界の x が 0..1 と 1..2 で、辺 x = 1 を共有する）。UV は左が x 0〜12 画素、右が 20〜32 画素の離れたアイランド。
/// mirrored は右のアイランドの UV の向きを反転し、mirrored_v は上下を反転する。folded は右の板を辺のところで手前へ折る。
fn faces(
    mirrored: bool,
    folded: bool,
    right_slot: i32,
    gap: f32,
    mirrored_v: bool,
) -> Vec<SurfaceTriangle> {
    let r = |x: f32, y: f32| {
        if folded {
            Vec3::new(1.0, y, x - 1.0)
        } else {
            p(x + gap, y)
        }
    };
    let l = |x: f32, y: f32| Vec2::new(0.375 * x, 0.125 + 0.75 * y);
    let u = |x: f32, y: f32| {
        Vec2::new(
            if mirrored {
                1.0 - 0.375 * (x - 1.0)
            } else {
                0.625 + 0.375 * (x - 1.0)
            },
            0.125 + 0.75 * if mirrored_v { 1.0 - y } else { y },
        )
    };
    vec![
        SurfaceTriangle::new(
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            l(0.0, 0.0),
            l(1.0, 0.0),
            l(1.0, 1.0),
        ),
        SurfaceTriangle::new(
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
            l(0.0, 0.0),
            l(1.0, 1.0),
            l(0.0, 1.0),
        ),
        SurfaceTriangle::new(
            r(1.0, 0.0),
            r(2.0, 0.0),
            r(2.0, 1.0),
            u(1.0, 0.0),
            u(2.0, 0.0),
            u(2.0, 1.0),
        )
        .with_slot(0, right_slot, -1),
        SurfaceTriangle::new(
            r(1.0, 0.0),
            r(2.0, 1.0),
            r(1.0, 1.0),
            u(1.0, 0.0),
            u(2.0, 1.0),
            u(1.0, 1.0),
        )
        .with_slot(0, right_slot, -1),
    ]
}
fn flat() -> Vec<SurfaceTriangle> {
    faces(false, false, 0, 0.0, false)
}
fn geometry(t: Vec<SurfaceTriangle>) -> SurfaceGeometry {
    SurfaceGeometry::new(t, 1, 0.000_001).unwrap()
}
fn hit(g: &SurfaceGeometry, at: Vec3) -> SurfaceHit {
    hit_along(g, at, Vec3::Z)
}
fn hit_along(g: &SurfaceGeometry, at: Vec3, normal: Vec3) -> SurfaceHit {
    g.raycast(Ray::new(at + normal * 3.0, -normal), true, f32::INFINITY)
        .expect("当たる")
}
fn chart_of<'g>(
    g: &'g SurfaceGeometry,
    anchor: &SurfaceHit,
    radius: f32,
) -> yolu_core::geometry::SamplingChart<'g> {
    g.build_sampling_chart(anchor, radius, Vec3::ZERO, 2048, i64::MAX)
        .unwrap()
}
fn dab(g: &SurfaceGeometry, at: &SurfaceHit, radius: f32) -> Vec<SurfacePixel> {
    let result = g.build_surface_dabs(
        at,
        radius,
        W,
        H,
        Vec3::new(1.0, 0.5, 3.0),
        1.0,
        &SurfaceBrushBudget::default(),
        None,
        false,
    );
    assert_eq!(result.refusal, None);
    result.pixels
}
fn plan(
    dest: &yolu_core::geometry::SamplingChart<'_>,
    source: &mut yolu_core::geometry::SamplingChart<'_>,
    pixels: &[SurfacePixel],
    offset: Vec2,
) -> Vec<BrushMappedPixel> {
    let mut result = Vec::new();
    for px in pixels {
        let point = dest.pixel_coordinates(px).expect("図の上にある画素");
        let pixel = BrushPixel {
            x: px.x as i64,
            y: px.y as i64,
            coverage: px.coverage as f64,
        };
        if let Some(mapped) = source.try_sample(point + offset, pixel, W, H).unwrap() {
            result.push(mapped);
        }
    }
    result
}
fn brush(effect: BrushEffect) -> Brush {
    Brush {
        effect,
        ..Brush::from(BrushSettings {
            flow: 1.0,
            opacity: 1.0,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        })
    }
}
fn document(tile: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W as u32, H as u32, tile).unwrap();
    let l = d.add_layer("paint").unwrap();
    (d, l)
}
fn snapshot(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .to_canvas_bytes()
}
fn color(d: &Document, l: LayerId, x: i32, y: i32) -> Rgba8 {
    d.layer(l)
        .unwrap()
        .pixel(Channel::Color, x as u32, y as u32)
        .unwrap()
}

#[test]
fn clone_copies_the_surface_pattern_across_separated_and_mirrored_islands() {
    for (mirrored, mirrored_v) in [(false, false), (true, false), (false, true), (true, true)] {
        let g = geometry(faces(mirrored, false, 0, 0.0, mirrored_v));
        let source = hit(&g, p(0.25, 0.5));
        let dest = hit(&g, p(1.25, 0.5));
        let (mut d, layer) = document(8);
        for y in 0..H {
            for x in 0..12 {
                d.set_pixel(
                    layer,
                    x as u32,
                    y as u32,
                    Rgba8::new((x * 17) as u8, (y * 13) as u8, 90, 255),
                )
                .unwrap();
            }
        }
        d.clear_history().unwrap();
        let before = snapshot(&d, layer);
        let a = chart_of(&g, &dest, 0.6);
        let mut b = chart_of(&g, &source, 0.6);
        let pixels = dab(&g, &dest, 0.3);
        let mapped = plan(&a, &mut b, &pixels, Vec2::ZERO);
        assert!(mapped.len() > 10);
        let mut s = d
            .begin_brush_stroke(
                layer,
                &brush(BrushEffect::Clone {
                    offset: DVec2::ZERO,
                }),
            )
            .unwrap();
        s.apply_mapped_dab(&mut d, &mapped, 1.0, 0, None).unwrap();
        d.end_stroke(s).unwrap();
        let mut checked = 0;
        for px in &pixels {
            if px.triangle.is_none_or(|t| t < 2) {
                continue;
            }
            // 世界 x の写し元は x − 1。UV の鏡映を独立に逆算して画素の式と比べる。
            let wx = if mirrored {
                1.0 + (1.0 - (px.x as f32 + 0.5) / W as f32) / 0.375
            } else {
                1.0 + ((px.x as f32 + 0.5) / W as f32 - 0.625) / 0.375
            };
            let sx = ((wx - 1.0) * 0.375 * W as f32).floor() as i32;
            if (0..12).contains(&sx) {
                let sy = if mirrored_v { H - 1 - px.y } else { px.y };
                assert_eq!(
                    color(&d, layer, px.x, px.y),
                    Rgba8::new((sx * 17) as u8, (sy * 13) as u8, 90, 255),
                    "UV の向きと画素 {},{} mirrored={mirrored} v={mirrored_v}",
                    px.x,
                    px.y
                );
                checked += 1;
            }
        }
        assert!(checked > 10);
        let after = snapshot(&d, layer);
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(snapshot(&d, layer), before);
        d.redo().unwrap();
        assert_eq!(snapshot(&d, layer), after);
    }
}

#[test]
fn smudge_pulls_across_a_seam_without_sampling_the_atlas_gap() {
    for mirrored in [false, true] {
        let g = geometry(faces(mirrored, false, 0, 0.0, false));
        let previous = hit(&g, p(0.875, 0.5));
        let current = hit(&g, p(1.125, 0.5));
        let (mut d, layer) = document(4);
        for y in 0..H {
            for x in 0..W {
                let c = if x < 12 {
                    Rgba8::new(220, 30, 70, 255)
                } else if x < 20 {
                    Rgba8::new(0, 255, 0, 255)
                } else {
                    Rgba8::new(0, 0, 0, 255)
                };
                d.set_pixel(layer, x as u32, y as u32, c).unwrap();
            }
        }
        let mut chart = g
            .build_sampling_chart(&current, 0.8, Vec3::ZERO, 2048, i64::MAX)
            .unwrap();
        let offset = chart.coordinates(&previous).expect("同じ図の上");
        assert!((offset.x - -0.25).abs() < 1e-5, "{offset}");
        let pixels = dab(&g, &current, 0.2);
        let mapped = {
            let mut m = Vec::new();
            for px in &pixels {
                let point = chart.pixel_coordinates(px).unwrap();
                let pixel = BrushPixel {
                    x: px.x as i64,
                    y: px.y as i64,
                    coverage: px.coverage as f64,
                };
                if let Some(v) = chart.try_sample(point + offset, pixel, W, H).unwrap() {
                    m.push(v);
                }
            }
            m
        };
        let mut s = d
            .begin_brush_stroke(layer, &brush(BrushEffect::Smudge { strength: 1.0 }))
            .unwrap();
        s.apply_mapped_dab(&mut d, &mapped, 1.0, 0, None).unwrap();
        d.end_stroke(s).unwrap();
        let mut changed = 0;
        for px in pixels
            .iter()
            .filter(|px| px.triangle.is_some_and(|t| t >= 2))
        {
            let c = color(&d, layer, px.x, px.y);
            assert!(
                c.g <= 30,
                "離れたアイランドの間の緑を読まない: {c:?} at {},{}",
                px.x,
                px.y
            );
            if c.r > 0 {
                changed += 1;
            }
        }
        assert!(changed > 4, "{changed}");
    }
}

#[test]
fn bilinear_taps_cross_the_geometric_edge_instead_of_reading_the_gap() {
    let g = geometry(flat());
    let at = hit(&g, p(0.99, 0.5));
    let mut chart = chart_of(&g, &at, 0.3);
    let sample = chart
        .try_sample(
            Vec2::ZERO,
            BrushPixel {
                x: 0,
                y: 0,
                coverage: 1.0,
            },
            W,
            H,
        )
        .unwrap()
        .expect("読める");
    let taps: Vec<_> = [sample.a, sample.b, sample.c, sample.d]
        .into_iter()
        .filter(|t| t.weight > 0.0)
        .collect();
    assert!(taps.iter().any(|t| t.x < 12));
    assert!(taps.iter().any(|t| t.x >= 20));
    assert!(taps.iter().all(|t| t.x < 12 || t.x >= 20), "{taps:?}");
    let sum: f64 = taps.iter().map(|t| t.weight).sum();
    assert!((sum - 1.0).abs() < 1e-6, "{sum}");
}

#[test]
fn folded_triangles_unfold_to_their_surface_distance_and_stale_hits_refuse() {
    let g = geometry(faces(false, true, 0, 0.0, false));
    let a = hit(&g, p(0.8, 0.5));
    let b = hit_along(&g, Vec3::new(1.0, 0.5, 0.2), Vec3::NEG_X);
    let chart = chart_of(&g, &a, 1.0);
    let q = chart.coordinates(&b).expect("折れた面も図に入る");
    assert!((q.x - 0.4).abs() < 1e-5 && q.y.abs() < 1e-5, "{q}");
    let newer = SurfaceGeometry::new(flat(), 2, 0.000_001).unwrap();
    assert_eq!(
        newer
            .build_sampling_chart(&a, 1.0, Vec3::ZERO, 2048, i64::MAX)
            .err(),
        Some(SamplingError::SnapshotChanged)
    );
    // 世代の違う当たりの座標は引かない
    assert_eq!(chart.coordinates(&SurfaceHit { revision: 9, ..b }), None);
}

#[test]
fn sampling_never_crosses_an_unconnected_or_ambiguous_edge() {
    for kind in ["slot", "gap", "nonmanifold"] {
        let mut t = faces(
            false,
            false,
            if kind == "slot" { 1 } else { 0 },
            if kind == "gap" { 0.01 } else { 0.0 },
            false,
        );
        if kind == "nonmanifold" {
            t.push(SurfaceTriangle::new(
                p(1.0, 0.0),
                p(1.0, 1.0),
                Vec3::new(1.0, 0.5, 1.0),
                Vec2::ZERO,
                Vec2::Y,
                Vec2::ONE,
            ));
        }
        let g = geometry(t);
        let anchor = hit(&g, p(0.8, 0.5));
        let mut chart = chart_of(&g, &anchor, 3.0);
        assert_eq!(chart.triangle_count(), 2, "{kind}");
        assert_eq!(
            chart
                .try_sample(
                    Vec2::new(0.4, 0.0),
                    BrushPixel {
                        x: 0,
                        y: 0,
                        coverage: 1.0
                    },
                    W,
                    H
                )
                .unwrap(),
            None,
            "{kind}"
        );
    }
}

#[test]
fn chart_budgets_return_no_partial_plan_and_pixels_keep_their_provenance() {
    let g = geometry(flat());
    let at = hit(&g, p(0.9, 0.5));
    assert_eq!(
        g.build_sampling_chart(&at, 1.0, Vec3::ZERO, 1, i64::MAX)
            .err(),
        Some(SamplingError::ChartBudget)
    );
    assert_eq!(
        g.build_sampling_chart(&at, 1.0, Vec3::ZERO, 2048, 127)
            .err(),
        Some(SamplingError::ChartBudget)
    );
    // 1 三角形 128 バイト: 4 枚ぶんちょうどなら通る
    let chart = g
        .build_sampling_chart(&at, 1.0, Vec3::ZERO, 2048, 512)
        .unwrap();
    assert_eq!(chart.nominal_bytes(), chart.triangle_count() as i64 * 128);
    let pixels = dab(&g, &at, 0.3);
    assert!(pixels.iter().any(|px| px.triangle.is_some_and(|t| t >= 2)));
    assert!(pixels.iter().all(|px| px.triangle.is_some()));
    for px in &pixels {
        assert!(px.position.z.abs() < 1e-6);
    }
}

#[test]
fn an_invalid_anchor_or_radius_is_refused() {
    let g = geometry(flat());
    let at = hit(&g, p(0.5, 0.5));
    for radius in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            g.build_sampling_chart(&at, radius, Vec3::ZERO, 2048, i64::MAX)
                .err(),
            Some(SamplingError::InvalidArguments)
        );
    }
    assert_eq!(
        g.build_sampling_chart(&at, 1.0, Vec3::new(f32::NAN, 0.0, 0.0), 2048, i64::MAX)
            .err(),
        Some(SamplingError::InvalidArguments)
    );
    let wrong = SurfaceHit { renderer: 5, ..at };
    assert_eq!(
        g.build_sampling_chart(&wrong, 1.0, Vec3::ZERO, 2048, i64::MAX)
            .err(),
        Some(SamplingError::BindingMismatch)
    );
    let stale = SurfaceHit { triangle: 99, ..at };
    assert_eq!(
        g.build_sampling_chart(&stale, 1.0, Vec3::ZERO, 2048, i64::MAX)
            .err(),
        Some(SamplingError::SnapshotChanged)
    );
}
