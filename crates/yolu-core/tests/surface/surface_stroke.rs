//! 3D の面のストローク（`SurfaceStroke`）に通す効果ブラシと対称（画面なしで、カメラと面だけ）: ぼかし・指先・クローンが UV アイランドの
//! 継ぎ目をまたいで読み元を運ぶ、ミラー・放射状が写しの側へも塗る、どれも 1 回の Undo で戻り、予算・対称との組み合わせ・古い元は
//! ストロークごと断る。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;

use yolu_core::geometry::{
    pick, CameraView, DabRefusal, MirrorOutcome, MirrorPlane, OrbitCamera, RadialSymmetry,
    SamplingError, SurfaceCloneSource, SurfaceEffect, SurfaceGeometry, SurfaceHit, SurfaceStencil,
    SurfaceStroke, SurfaceStrokeError, SurfaceStrokeOptions, SurfaceSymmetrySetup, SurfaceTriangle,
    SymmetryAxis,
};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::{
    Brush, BrushEffect, BrushSettings, BrushStencil, Channel, Document, ImageColorSpace, LayerId,
    Rgba8, StencilImage, StencilMapping, StencilMode, StencilTiling, Stroke,
};

const W: u32 = 32;
const H: u32 = 16;

fn p(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

/// 左右に 1 枚ずつの板（世界の x が 0..1 と 1..2、辺 x = 1 を共有）。UV は左が x 0〜12 画素、右が 20〜32 画素の離れたアイランド。表は +Z。
fn plane() -> Arc<SurfaceGeometry> {
    let l = |x: f32, y: f32| Vec2::new(0.375 * x, 0.125 + 0.75 * y);
    let u = |x: f32, y: f32| Vec2::new(0.625 + 0.375 * (x - 1.0), 0.125 + 0.75 * y);
    let t = |a, b, c, ua, ub, uc| SurfaceTriangle::new(a, b, c, ua, ub, uc);
    Arc::new(
        SurfaceGeometry::new(
            vec![
                t(
                    p(0.0, 0.0),
                    p(1.0, 0.0),
                    p(1.0, 1.0),
                    l(0.0, 0.0),
                    l(1.0, 0.0),
                    l(1.0, 1.0),
                ),
                t(
                    p(0.0, 0.0),
                    p(1.0, 1.0),
                    p(0.0, 1.0),
                    l(0.0, 0.0),
                    l(1.0, 1.0),
                    l(0.0, 1.0),
                ),
                t(
                    p(1.0, 0.0),
                    p(2.0, 0.0),
                    p(2.0, 1.0),
                    u(1.0, 0.0),
                    u(2.0, 0.0),
                    u(2.0, 1.0),
                ),
                t(
                    p(1.0, 0.0),
                    p(2.0, 1.0),
                    p(1.0, 1.0),
                    u(1.0, 0.0),
                    u(2.0, 1.0),
                    u(1.0, 1.0),
                ),
            ],
            1,
            0.000_001,
        )
        .unwrap(),
    )
}

/// 板を正面（+Z の側）から見たカメラ。
fn front(g: &SurfaceGeometry) -> yolu_core::geometry::CameraView {
    let mut cam = OrbitCamera::framing(&g.bounds());
    cam.yaw = 180.0;
    cam.pitch = 0.0;
    cam.view(400.0, 200.0)
}

fn document() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    (d, l)
}
fn bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(Vec::new, |s| s.to_canvas_bytes())
}
fn color(d: &Document, l: LayerId, x: u32, y: u32) -> Rgba8 {
    d.layer(l).unwrap().pixel(Channel::Color, x, y).unwrap()
}
fn base() -> BrushSettings {
    BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        pressure_size: false,
        pressure_opacity: false,
        color: Rgba8::new(10, 200, 30, 255),
        ..BrushSettings::default()
    }
}
fn brush(effect: BrushEffect) -> Brush {
    Brush {
        effect,
        ..Brush::from(base())
    }
}
fn screen(view: &yolu_core::geometry::CameraView, world: Vec3) -> Vec2 {
    view.to_screen(world).unwrap()
}
fn hit_at(g: &SurfaceGeometry, view: &yolu_core::geometry::CameraView, world: Vec3) -> SurfaceHit {
    pick(g, view, screen(view, world)).expect("当たる")
}
/// 世界の 2 点の間を 8 区間で引く。
fn drag(
    d: &mut Document,
    s: &mut SurfaceStroke,
    stroke: &mut Stroke,
    view: &yolu_core::geometry::CameraView,
    from: Vec3,
    to: Vec3,
) -> Result<(), SurfaceStrokeError> {
    let (a, b) = (screen(view, from), screen(view, to));
    for i in 1..=8 {
        s.add(d, stroke, a + (b - a) * (i as f32 / 8.0), 1.0)?;
    }
    s.finish(d, stroke)
}
fn begin(
    d: &mut Document,
    layer: LayerId,
    g: &Arc<SurfaceGeometry>,
    view: yolu_core::geometry::CameraView,
    brush_value: &Brush,
    at: Vec2,
    options: SurfaceStrokeOptions,
) -> (Stroke, Result<SurfaceStroke, SurfaceStrokeError>) {
    let mut stroke = d.begin_brush_stroke(layer, brush_value).unwrap();
    let s = SurfaceStroke::begin_with_options(
        d,
        &mut stroke,
        g.clone(),
        view,
        &brush_value.base,
        Some(0),
        at,
        1.0,
        options,
    );
    (stroke, s)
}

#[test]
fn blur_in_a_surface_stroke_smooths_the_texture_and_undoes_in_one_step() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    for y in 0..H {
        for x in 0..12 {
            let c = if (x + y) % 2 == 0 {
                Rgba8::new(255, 255, 255, 255)
            } else {
                Rgba8::new(0, 0, 0, 255)
            };
            d.set_pixel(layer, x, y, c).unwrap();
        }
    }
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let from = Vec3::new(0.2, 0.5, 0.0);
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Blur { radius: 2 }),
        screen(&view, from),
        SurfaceStrokeOptions {
            effect: SurfaceEffect::Blur,
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    drag(
        &mut d,
        &mut s,
        &mut stroke,
        &view,
        from,
        Vec3::new(0.7, 0.5, 0.0),
    )
    .unwrap();
    assert!(d.end_stroke(stroke).unwrap().changed);
    let mut smoothed = 0;
    for y in 4..12 {
        for x in 2..10 {
            let c = color(&d, layer, x, y);
            if c.r != 0 && c.r != 255 {
                smoothed += 1;
            }
        }
    }
    assert!(smoothed > 10, "市松模様が平均へ寄る: {smoothed}");
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(bytes(&d, layer), before);
}

#[test]
fn clone_in_a_surface_stroke_copies_the_pattern_across_islands() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    for y in 0..H {
        for x in 0..12 {
            d.set_pixel(
                layer,
                x,
                y,
                Rgba8::new((x * 17) as u8, (y * 13) as u8, 90, 255),
            )
            .unwrap();
        }
    }
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let source = hit_at(&g, &view, Vec3::new(0.25, 0.5, 0.0));
    let dest = Vec3::new(1.25, 0.5, 0.0);
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Clone {
            offset: Default::default(),
        }),
        screen(&view, dest),
        SurfaceStrokeOptions {
            effect: SurfaceEffect::Clone(SurfaceCloneSource {
                source,
                destination: None,
            }),
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    drag(
        &mut d,
        &mut s,
        &mut stroke,
        &view,
        dest,
        Vec3::new(1.6, 0.5, 0.0),
    )
    .unwrap();
    let destination = s.clone_destination().expect("最初のダブの面の点");
    assert!(destination.position.distance(dest) < 1e-3);
    assert!(d.end_stroke(stroke).unwrap().changed);
    let mut checked = 0;
    for y in 0..H {
        for x in 20..W {
            let c = color(&d, layer, x, y);
            if c.a == 0 {
                continue;
            }
            // 世界の x が 1 ずれた先のアイランドの画素: UV の x は (x − 20) 画素
            assert_eq!(
                c,
                Rgba8::new(((x - 20) * 17) as u8, (y * 13) as u8, 90, 255),
                "{x},{y}"
            );
            checked += 1;
        }
    }
    assert!(checked > 10, "{checked}");
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(bytes(&d, layer), before);
}

#[test]
fn smudge_in_a_surface_stroke_drags_across_the_seam_without_reading_the_gap() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    for y in 0..H {
        for x in 0..W {
            let c = if x < 12 {
                Rgba8::new(220, 30, 70, 255)
            } else if x < 20 {
                Rgba8::new(0, 255, 0, 255)
            } else {
                Rgba8::new(0, 0, 0, 255)
            };
            d.set_pixel(layer, x, y, c).unwrap();
        }
    }
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let from = Vec3::new(0.7, 0.5, 0.0);
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Smudge { strength: 1.0 }),
        screen(&view, from),
        SurfaceStrokeOptions {
            effect: SurfaceEffect::Smudge,
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    drag(
        &mut d,
        &mut s,
        &mut stroke,
        &view,
        from,
        Vec3::new(1.5, 0.5, 0.0),
    )
    .unwrap();
    assert_eq!(s.stats.lost, 0);
    assert!(d.end_stroke(stroke).unwrap().changed);
    let mut dragged = 0;
    for y in 0..H {
        for x in 20..W {
            let c = color(&d, layer, x, y);
            assert!(
                c.g <= 30,
                "離れたアイランドの間の緑を読まない: {c:?} at {x},{y}"
            );
            if c.r > 0 {
                dragged += 1;
            }
        }
    }
    assert!(dragged > 4, "継ぎ目の向こうへ色が引きずられる: {dragged}");
    d.undo().unwrap();
    assert_eq!(bytes(&d, layer), before);
}

#[test]
fn mirror_in_a_surface_stroke_paints_the_other_island_too() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    d.clear_history().unwrap();
    let plane_x1 = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::X, 1.0);
    let at = Vec3::new(0.3, 0.5, 0.0);
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Paint),
        screen(&view, at),
        SurfaceStrokeOptions {
            symmetry: Some(SurfaceSymmetrySetup {
                mirror: Some(plane_x1),
                radial: None,
                ignore_visibility: false,
            }),
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    assert!(s.stats.copies >= 1, "{:?}", s.stats);
    assert_eq!(s.symmetry_note(), None);
    assert!(d.end_stroke(stroke).unwrap().changed);
    let painted = |d: &Document, lo: u32, hi: u32| {
        (0..H).any(|y| (lo..hi).any(|x| color(d, layer, x, y).a > 0))
    };
    assert!(painted(&d, 0, 12), "元の側");
    assert!(painted(&d, 20, W), "ミラーした側");
    assert!(!painted(&d, 12, 20), "アイランドの間は塗らない");
    d.undo().unwrap();
    assert_eq!(d.undo_count(), 0);
    assert!(!painted(&d, 0, W));
}

#[test]
fn radial_in_a_surface_stroke_rotates_around_the_axis() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    d.clear_history().unwrap();
    let radial = RadialSymmetry::new(Vec3::new(1.0, 0.5, 0.0), Vec3::Z, 2).unwrap();
    let at = Vec3::new(0.3, 0.3, 0.0);
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Paint),
        screen(&view, at),
        SurfaceStrokeOptions {
            symmetry: Some(SurfaceSymmetrySetup {
                mirror: None,
                radial: Some(radial),
                ignore_visibility: false,
            }),
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    assert_eq!(s.stats.copies, 1);
    d.end_stroke(stroke).unwrap();
    // 中心 (1, 0.5) のまわりに 180° 回した先 (1.7, 0.7) は右のアイランドの上半分
    let upper_right = (10..H).any(|y| (20..W).any(|x| color(&d, layer, x, y).a > 0));
    let lower_right = (0..6).any(|y| (20..W).any(|x| color(&d, layer, x, y).a > 0));
    assert!(upper_right && !lower_right);
}

#[test]
fn a_mirror_with_no_surface_nearby_is_skipped_with_a_reason() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    let far = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::X, 5.0);
    let at = Vec3::new(0.5, 0.5, 0.0);
    let (stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Paint),
        screen(&view, at),
        SurfaceStrokeOptions {
            symmetry: Some(SurfaceSymmetrySetup {
                mirror: Some(far),
                radial: None,
                ignore_visibility: false,
            }),
            ..Default::default()
        },
    );
    let s = s.unwrap();
    assert_eq!(s.symmetry_note(), Some(MirrorOutcome::NoSurface));
    assert_eq!(s.stats.copies, 0);
    assert!(d.end_stroke(stroke).unwrap().changed, "元の側は塗る");
}

#[test]
fn smudge_and_clone_refuse_symmetry_before_painting() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    d.set_pixel(layer, 1, 1, Rgba8::new(1, 2, 3, 255)).unwrap();
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let sym = Some(SurfaceSymmetrySetup {
        mirror: Some(MirrorPlane::from_model(
            Vec3::ZERO,
            Quat::IDENTITY,
            SymmetryAxis::X,
            1.0,
        )),
        radial: None,
        ignore_visibility: false,
    });
    let source = hit_at(&g, &view, Vec3::new(0.25, 0.5, 0.0));
    for effect in [
        SurfaceEffect::Smudge,
        SurfaceEffect::Clone(SurfaceCloneSource {
            source,
            destination: None,
        }),
    ] {
        let (stroke, s) = begin(
            &mut d,
            layer,
            &g,
            view,
            &brush(BrushEffect::Smudge { strength: 1.0 }),
            screen(&view, Vec3::new(0.5, 0.5, 0.0)),
            SurfaceStrokeOptions {
                symmetry: sym,
                effect,
                ..Default::default()
            },
        );
        assert!(matches!(s, Err(SurfaceStrokeError::EffectWithSymmetry)));
        d.cancel_stroke(stroke);
        assert_eq!(bytes(&d, layer), before);
        assert_eq!(d.undo_count(), 0);
    }
}

#[test]
fn a_stale_clone_source_is_refused_and_a_tight_budget_cancels_the_stroke() {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    for x in 0..12 {
        d.set_pixel(layer, x, 8, Rgba8::new(200, 50, 50, 255))
            .unwrap();
    }
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let source = hit_at(&g, &view, Vec3::new(0.25, 0.5, 0.0));
    // 世代の違う元
    let stale = SurfaceHit {
        revision: 9,
        ..source
    };
    let (stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Clone {
            offset: Default::default(),
        }),
        screen(&view, Vec3::new(1.25, 0.5, 0.0)),
        SurfaceStrokeOptions {
            effect: SurfaceEffect::Clone(SurfaceCloneSource {
                source: stale,
                destination: None,
            }),
            ..Default::default()
        },
    );
    assert!(matches!(s, Err(SurfaceStrokeError::CloneSource)));
    d.cancel_stroke(stroke);
    // 予算: 投影の画素は入るが、読み元の図が 1 回の操作の予算に入らない。ストロークごと取り消す
    d.set_stroke_budget_bytes(1000).unwrap();
    let clone = |source: SurfaceHit, projection_memory: Option<u64>| SurfaceStrokeOptions {
        effect: SurfaceEffect::Clone(SurfaceCloneSource {
            source,
            destination: None,
        }),
        projection_memory,
        ..Default::default()
    };
    let (stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Clone {
            offset: Default::default(),
        }),
        screen(&view, Vec3::new(1.25, 0.5, 0.0)),
        clone(source, Some(64 << 20)),
    );
    assert_eq!(
        s.err(),
        Some(SurfaceStrokeError::Sampling(SamplingError::ChartBudget))
    );
    d.cancel_stroke(stroke);
    assert!(!d.has_active_stroke());
    assert_eq!(bytes(&d, layer), before);
    assert_eq!(d.undo_count(), 0);
    // 投影の画素も入らない: 2D と同じくストロークごと取り消す（呼び手が取り消す。何も残らない）
    let (stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Clone {
            offset: Default::default(),
        }),
        screen(&view, Vec3::new(1.25, 0.5, 0.0)),
        clone(source, None),
    );
    assert_eq!(
        s.err(),
        Some(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
    );
    d.cancel_stroke(stroke);
    assert!(!d.has_active_stroke());
    assert_eq!(bytes(&d, layer), before);
    assert_eq!(d.undo_count(), 0);
    // 同じ元でも、予算が足りれば通る
    d.set_stroke_budget_bytes(64 << 20).unwrap();
    let (stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &brush(BrushEffect::Clone {
            offset: Default::default(),
        }),
        screen(&view, Vec3::new(1.25, 0.5, 0.0)),
        clone(source, None),
    );
    assert!(s.is_ok());
    d.cancel_stroke(stroke);
}

#[test]
fn clone_in_a_surface_stroke_can_read_the_visible_composite_of_other_layers() {
    let g = plane();
    let view = front(&g);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let below = d.add_layer("below").unwrap();
    for y in 0..H {
        for x in 0..12 {
            d.set_pixel(
                below,
                x,
                y,
                Rgba8::new((x * 17) as u8, (y * 13) as u8, 90, 255),
            )
            .unwrap();
        }
    }
    let target = d.add_layer("target").unwrap();
    d.clear_history().unwrap();
    let source = hit_at(&g, &view, Vec3::new(0.25, 0.5, 0.0));
    let dest = Vec3::new(1.25, 0.5, 0.0);
    let brush_value = brush(BrushEffect::Clone {
        offset: Default::default(),
    });
    // 描くレイヤー（空）だけを読むと、何も写らない。見えているレイヤーの重なりを読むと、下のレイヤーの模様が写る
    for composite in [false, true] {
        let mut stroke = d.begin_brush_stroke(target, &brush_value).unwrap();
        if composite {
            stroke.use_composite_clone_source(&mut d).unwrap();
        }
        let mut s = SurfaceStroke::begin_with_options(
            &mut d,
            &mut stroke,
            g.clone(),
            view,
            &brush_value.base,
            Some(0),
            screen(&view, dest),
            1.0,
            SurfaceStrokeOptions {
                effect: SurfaceEffect::Clone(SurfaceCloneSource {
                    source,
                    destination: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        drag(
            &mut d,
            &mut s,
            &mut stroke,
            &view,
            dest,
            Vec3::new(1.5, 0.5, 0.0),
        )
        .unwrap();
        d.end_stroke(stroke).unwrap();
        let mut painted = 0;
        for y in 0..H {
            for x in 20..W {
                let c = color(&d, target, x, y);
                if c.a > 0 {
                    assert_eq!(
                        c,
                        Rgba8::new(((x - 20) * 17) as u8, (y * 13) as u8, 90, 255)
                    );
                    painted += 1;
                }
            }
        }
        assert_eq!(
            painted > 10,
            composite,
            "composite={composite} painted={painted}"
        );
        if d.undo_count() > 0 {
            d.undo().unwrap();
        }
    }
}

// ───────── ステンシルを通した効果ブラシと対称 ─────────
//
// ステンシルは画面に貼り付いた画像で、ダブの画素ごとにその画素の画面の位置の量を読む。効果ブラシ（ぼかし・クローン・指先）と
// 対称の写しの画素も、元の足跡で、それぞれの画素の位置から点を求める。量は画像の位置に応じて変わり、1 回の Undo で戻る。

/// 64 × 8 の灰色の画像（量のモード）。列 i の量は level(i) / 255。
fn grey_image(level: impl Fn(usize) -> u8) -> Arc<StencilImage> {
    let mut rgba = Vec::new();
    for _ in 0..8 {
        for x in 0..64 {
            let v = level(x);
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Arc::new(
        StencilImage::new(
            64,
            8,
            rgba,
            ImageColorSpace::Srgb,
            StencilImage::DEFAULT_MIP_BUDGET_BYTES,
        )
        .unwrap(),
    )
}
/// 左から右へ量が 0 から増える画像。
fn ramp() -> Arc<StencilImage> {
    grey_image(|x| (x * 4) as u8)
}
fn white() -> Arc<StencilImage> {
    grey_image(|_| 255)
}
fn black() -> Arc<StencilImage> {
    grey_image(|_| 0)
}

/// 画像の幅が、世界の x = lo〜hi の画面の幅に重なる置き場。
type Place = (f32, f32);

fn placement(view: &CameraView, (lo, hi): Place) -> SurfaceStencil {
    let (a, b) = (
        screen(view, Vec3::new(lo, 0.5, 0.0)).x as f64,
        screen(view, Vec3::new(hi, 0.5, 0.0)).x as f64,
    );
    let scale = 64.0 / (b - a);
    SurfaceStencil::new(
        StencilMapping::new(scale, 0.0, -a * scale, 0.0, 0.0, 4.0).unwrap(),
        scale.abs(),
    )
    .unwrap()
}

fn stencil_brush(effect: BrushEffect, image: &Arc<StencilImage>) -> Brush {
    let mut b = brush(effect);
    b.stencil = Some(Arc::new(BrushStencil::new(
        image.clone(),
        StencilMode::Mask,
        StencilTiling::None,
        false,
        None,
        &[],
    )));
    b
}

/// 文書の画素 (x, y) の中心が乗る、板の上の点（左のアイランドは x 0〜12 が世界の 0〜1、右のアイランドは 20〜32 が 1〜2）。
fn world_of(x: u32, y: u32) -> Vec3 {
    let u = (x as f32 + 0.5) / W as f32;
    let v = (y as f32 + 0.5) / H as f32;
    let world_y = (v - 0.125) / 0.75;
    let world_x = if x < W / 2 {
        u / 0.375
    } else {
        1.0 + (u - 0.625) / 0.375
    };
    Vec3::new(world_x, world_y, 0.0)
}

/// 画素 (x, y) の位置で、画像が読ませる量（0〜1）。
fn amount_at(image: &StencilImage, view: &CameraView, place: Place, x: u32, y: u32) -> f64 {
    let screen_point = view.to_screen(world_of(x, y)).unwrap();
    let placement = placement(view, place);
    let (ix, iy) = placement
        .screen_to_image()
        .map_point(screen_point.x as f64, screen_point.y as f64);
    image.read(ix, iy, 1.0, StencilTiling::None).luma_alpha
}

/// 引いたあとの文書とレイヤー（文書ごとにレイヤーの札が違う）と、引く前の画素。
struct Ran {
    d: Document,
    layer: LayerId,
    before: Vec<u8>,
    place: Place,
}
impl Ran {
    fn color(&self, x: u32, y: u32) -> Rgba8 {
        color(&self.d, self.layer, x, y)
    }
    /// 1 回の Undo で、引く前の画素に戻る。
    fn undoes_in_one_step(mut self) {
        assert_eq!(self.d.undo_count(), 1);
        self.d.undo().unwrap();
        assert_eq!(bytes(&self.d, self.layer), self.before);
    }
}

/// 1 つのストロークを引く（`to` が無ければ最初の 1 点だけ）。画像があれば、ブラシにも置き場にもステンシルを付ける。
#[allow(clippy::too_many_arguments)]
fn run_stroke(
    prepare: impl Fn(&mut Document, LayerId),
    g: &Arc<SurfaceGeometry>,
    view: CameraView,
    place: Place,
    effect: BrushEffect,
    options: SurfaceStrokeOptions,
    image: Option<&Arc<StencilImage>>,
    from: Vec3,
    to: Option<Vec3>,
) -> Ran {
    let (mut d, layer) = document();
    prepare(&mut d, layer);
    d.clear_history().unwrap();
    let before = bytes(&d, layer);
    let brush_value = match image {
        Some(i) => stencil_brush(effect, i),
        None => brush(effect),
    };
    let options = SurfaceStrokeOptions {
        stencil: image.map(|_| placement(&view, place)),
        ..options
    };
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        g,
        view,
        &brush_value,
        screen(&view, from),
        options,
    );
    let mut s = s.unwrap();
    match to {
        Some(to) => drag(&mut d, &mut s, &mut stroke, &view, from, to).unwrap(),
        None => s.finish(&mut d, &mut stroke).unwrap(),
    }
    d.end_stroke(stroke).unwrap();
    Ran {
        d,
        layer,
        before,
        place,
    }
}

fn checker(d: &mut Document, layer: LayerId, lo: u32, hi: u32) {
    for y in 0..H {
        for x in lo..hi {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            d.set_pixel(layer, x, y, Rgba8::new(v, v, v, 255)).unwrap();
        }
    }
}

/// ステンシルありの塗った量（アルファ）が、全部白のステンシルの塗った量に、その画素の位置の画像の量を掛けたものと合うか。
/// 確かめた画素の数、そのときの量の最小・最大、いちばん大きいずれ（0〜1）を返す。
fn alpha_matches_the_amount(
    ramped: &Ran,
    full: &Ran,
    view: &CameraView,
    image: &StencilImage,
    columns: std::ops::Range<u32>,
) -> Matched {
    let mut m = Matched::new();
    for y in 0..H {
        for x in columns.clone() {
            let full_alpha = full.color(x, y).a;
            if full_alpha < 100 {
                continue;
            }
            let ratio = ramped.color(x, y).a as f64 / full_alpha as f64;
            m.add(ratio, amount_at(image, view, ramped.place, x, y));
        }
    }
    m
}

/// ぼかしの、ステンシルありの動きが、全部白のときの動き（元の値からぼけた値まで）にその画素の位置の画像の量を掛けたものと合うか。
fn blur_matches_the_amount(
    ramped: &Ran,
    full: &Ran,
    view: &CameraView,
    image: &StencilImage,
    columns: std::ops::Range<u32>,
) -> Matched {
    let mut m = Matched::new();
    for y in 0..H {
        for x in columns.clone() {
            let original = ramped.before[((y * W + x) * 4) as usize] as f64;
            let blurred = full.color(x, y).r as f64;
            if (blurred - original).abs() < 80.0 {
                continue;
            }
            let through = (ramped.color(x, y).r as f64 - original) / (blurred - original);
            m.add(through, amount_at(image, view, ramped.place, x, y));
        }
    }
    m
}

/// 画素ごとの「見た量」と「画像の量」の突き合わせの集計。
struct Matched {
    checked: usize,
    lo: f64,
    hi: f64,
    worst: f64,
    sum: f64,
}
impl Matched {
    fn new() -> Self {
        Matched {
            checked: 0,
            lo: f64::MAX,
            hi: f64::MIN,
            worst: 0.0,
            sum: 0.0,
        }
    }
    fn add(&mut self, seen: f64, expected: f64) {
        self.checked += 1;
        self.lo = self.lo.min(expected);
        self.hi = self.hi.max(expected);
        self.worst = self.worst.max((seen - expected).abs());
        self.sum += expected;
    }
    fn mean(&self) -> f64 {
        self.sum / self.checked.max(1) as f64
    }
}

#[test]
fn blur_through_a_stencil_changes_each_pixel_by_the_amount_at_its_position() {
    let g = plane();
    let view = front(&g);
    let image = ramp();
    let prepare = |d: &mut Document, l: LayerId| checker(d, l, 0, 12);
    let at = Vec3::new(0.45, 0.5, 0.0);
    let options = SurfaceStrokeOptions {
        effect: SurfaceEffect::Blur,
        ..Default::default()
    };
    let blur = BrushEffect::Blur { radius: 2 };
    let run = |image: &Arc<StencilImage>| {
        run_stroke(
            prepare,
            &g,
            view,
            (0.0, 1.0),
            blur,
            options,
            Some(image),
            at,
            None,
        )
    };
    let open = run(&white());
    let ramped = run(&image);
    let m = blur_matches_the_amount(&ramped, &open, &view, &image, 0..12);
    assert!(m.checked > 12, "確かめた画素: {}", m.checked);
    assert!(m.hi - m.lo > 0.2, "量が画素ごとに違う: {}〜{}", m.lo, m.hi);
    assert!(m.worst < 0.06, "ずれ: {}", m.worst);
    ramped.undoes_in_one_step();
}

#[test]
fn clone_through_a_stencil_keeps_each_pixels_amount_when_unreadable_pixels_are_dropped() {
    let g = plane();
    let view = front(&g);
    let image = ramp();
    // 左のアイランドの縁（x = 0.1）を元にして、右のアイランドへ写す。元の円の左の欠けた側（板の外）は読めないので、その画素は落ちる
    let prepare = |d: &mut Document, l: LayerId| {
        for y in 0..H {
            for x in 0..12 {
                d.set_pixel(l, x, y, Rgba8::new((x * 17) as u8, (y * 13) as u8, 90, 255))
                    .unwrap();
            }
        }
    };
    let source = hit_at(&g, &view, Vec3::new(0.1, 0.5, 0.0));
    let options = SurfaceStrokeOptions {
        effect: SurfaceEffect::Clone(SurfaceCloneSource {
            source,
            destination: None,
        }),
        ..Default::default()
    };
    let at = Vec3::new(1.5, 0.5, 0.0);
    let clone = BrushEffect::Clone {
        offset: Default::default(),
    };
    let run = |image: &Arc<StencilImage>| {
        run_stroke(
            prepare,
            &g,
            view,
            (1.0, 2.0),
            clone,
            options,
            Some(image),
            at,
            None,
        )
    };
    let open = run(&white());
    let ramped = run(&image);
    // 読めない画素が落ちている（同じ大きさの通常のダブより少ない）
    let plain = run_stroke(
        prepare,
        &g,
        view,
        (1.0, 2.0),
        BrushEffect::Paint,
        SurfaceStrokeOptions::default(),
        None,
        at,
        None,
    );
    let count = |r: &Ran| {
        (0..H)
            .flat_map(|y| (20..W).map(move |x| (x, y)))
            .filter(|&(x, y)| r.color(x, y).a > 0)
            .count()
    };
    assert!(
        count(&open) > 10 && count(&open) < count(&plain),
        "読めない画素を落とした: {} < {}",
        count(&open),
        count(&plain)
    );
    // 残った画素のどれもが、自分の位置の量で塗られている（落ちた画素の分だけ量がずれていない）
    let m = alpha_matches_the_amount(&ramped, &open, &view, &image, 20..W);
    assert!(m.checked > 10, "確かめた画素: {}", m.checked);
    assert!(m.hi - m.lo > 0.15, "量が画素ごとに違う: {}〜{}", m.lo, m.hi);
    assert!(m.worst < 0.06, "ずれ: {}", m.worst);
    ramped.undoes_in_one_step();
}

fn x_mirror() -> Option<SurfaceSymmetrySetup> {
    Some(SurfaceSymmetrySetup {
        mirror: Some(MirrorPlane::from_model(
            Vec3::ZERO,
            Quat::IDENTITY,
            SymmetryAxis::X,
            1.0,
        )),
        radial: None,
        ignore_visibility: false,
    })
}

#[test]
fn a_mirrored_copy_reads_the_stencil_at_its_own_position() {
    let g = plane();
    let view = front(&g);
    let image = ramp();
    let options = SurfaceStrokeOptions {
        symmetry: x_mirror(),
        ..Default::default()
    };
    let at = Vec3::new(0.3, 0.5, 0.0);
    let run = |image: &Arc<StencilImage>| {
        run_stroke(
            |_, _| {},
            &g,
            view,
            (0.0, 2.0),
            BrushEffect::Paint,
            options,
            Some(image),
            at,
            None,
        )
    };
    let open = run(&white());
    let ramped = run(&image);
    let own = alpha_matches_the_amount(&ramped, &open, &view, &image, 0..12);
    let copy = alpha_matches_the_amount(&ramped, &open, &view, &image, 20..W);
    assert!(
        own.checked > 10 && copy.checked > 10,
        "{} {}",
        own.checked,
        copy.checked
    );
    assert!(
        own.worst < 0.06 && copy.worst < 0.06,
        "{} {}",
        own.worst,
        copy.worst
    );
    // 元と写しは画像の反対側に乗るので、量が大きく違う（写しの量を元の位置で読んでいない）
    assert!(
        (own.mean() - copy.mean()).abs() > 0.4,
        "元 {} 写し {}",
        own.mean(),
        copy.mean()
    );
    ramped.undoes_in_one_step();
}

#[test]
fn blur_with_a_mirror_and_a_stencil_blurs_both_sides_by_their_own_amounts() {
    let g = plane();
    let view = front(&g);
    let image = ramp();
    let prepare = |d: &mut Document, l: LayerId| {
        checker(d, l, 0, 12);
        checker(d, l, 20, W);
    };
    let options = SurfaceStrokeOptions {
        effect: SurfaceEffect::Blur,
        symmetry: x_mirror(),
        ..Default::default()
    };
    let at = Vec3::new(0.3, 0.5, 0.0);
    let blur = BrushEffect::Blur { radius: 2 };
    let run = |image: &Arc<StencilImage>| {
        run_stroke(
            prepare,
            &g,
            view,
            (0.0, 2.0),
            blur,
            options,
            Some(image),
            at,
            None,
        )
    };
    let open = run(&white());
    let ramped = run(&image);
    let own = blur_matches_the_amount(&ramped, &open, &view, &image, 0..12);
    let copy = blur_matches_the_amount(&ramped, &open, &view, &image, 20..W);
    assert!(
        own.checked > 8 && copy.checked > 8,
        "{} {}",
        own.checked,
        copy.checked
    );
    assert!(
        own.worst < 0.06 && copy.worst < 0.06,
        "{} {}",
        own.worst,
        copy.worst
    );
    assert!(
        (own.mean() - copy.mean()).abs() > 0.4,
        "元 {} 写し {}",
        own.mean(),
        copy.mean()
    );
    ramped.undoes_in_one_step();
}

#[test]
fn a_closed_stencil_leaves_blur_smudge_and_clone_untouched() {
    let g = plane();
    let view = front(&g);
    let shut = black();
    let prepare = |d: &mut Document, l: LayerId| {
        for y in 0..H {
            for x in 0..12 {
                d.set_pixel(l, x, y, Rgba8::new((x * 17) as u8, (y * 13) as u8, 90, 255))
                    .unwrap();
            }
        }
    };
    let source = hit_at(&g, &view, Vec3::new(0.25, 0.5, 0.0));
    let cases = [
        (
            BrushEffect::Blur { radius: 2 },
            SurfaceEffect::Blur,
            Vec3::new(0.2, 0.5, 0.0),
        ),
        (
            BrushEffect::Smudge { strength: 1.0 },
            SurfaceEffect::Smudge,
            Vec3::new(0.2, 0.5, 0.0),
        ),
        (
            BrushEffect::Clone {
                offset: Default::default(),
            },
            SurfaceEffect::Clone(SurfaceCloneSource {
                source,
                destination: None,
            }),
            Vec3::new(1.25, 0.5, 0.0),
        ),
    ];
    for (brush_effect, effect, from) in cases {
        let r = run_stroke(
            prepare,
            &g,
            view,
            (0.0, 2.0),
            brush_effect,
            SurfaceStrokeOptions {
                effect,
                ..Default::default()
            },
            Some(&shut),
            from,
            Some(from + Vec3::new(0.4, 0.0, 0.0)),
        );
        assert_eq!(bytes(&r.d, r.layer), r.before, "{effect:?}");
        // 全部開いていれば、同じストロークは変える（閉じたステンシルの試験が空振りでないこと）
        let r = run_stroke(
            prepare,
            &g,
            view,
            (0.0, 2.0),
            brush_effect,
            SurfaceStrokeOptions {
                effect,
                ..Default::default()
            },
            Some(&white()),
            from,
            Some(from + Vec3::new(0.4, 0.0, 0.0)),
        );
        assert_ne!(bytes(&r.d, r.layer), r.before, "{effect:?}");
    }
}

// ───────── 筆圧の応え（大きさ・硬さ・不透明度）─────────

/// 筆圧 pressure で 1 つだけ面にダブを置いたレイヤーの画素（板の左の中ほど）。
fn one_dab(brush_value: &Brush, pressure: f32) -> (Vec<u8>, usize) {
    let g = plane();
    let view = front(&g);
    let (mut d, layer) = document();
    let at = screen(&view, Vec3::new(0.5, 0.5, 0.0));
    let mut stroke = d.begin_brush_stroke(layer, brush_value).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &brush_value.base,
        Some(0),
        at,
        pressure,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    let pixels = bytes(&d, layer);
    let painted = pixels.chunks_exact(4).filter(|p| p[3] != 0).count();
    (pixels, painted)
}

#[test]
fn the_surface_brush_takes_size_hardness_and_opacity_from_the_shaped_pressure() {
    use yolu_core::PressureResponse;
    let mut b = brush(BrushEffect::Paint);
    b.base.radius = 4.0;
    b.base.pressure_size = true;
    // 既定の応え: 筆圧 0 は点を置かない（今までと同じ）
    assert_eq!(one_dab(&b, 0.0).1, 0);
    // 最小値: 筆圧 0 でも、最小値の大きさの点が置かれる
    b.pressure.size = PressureResponse::new(0.5, vec![]).unwrap();
    let lifted = one_dab(&b, 0.0);
    let full = one_dab(&b, 1.0);
    assert!(lifted.1 > 0 && lifted.1 < full.1, "{} {}", lifted.1, full.1);
    // 応えは文書のストロークのブラシのもの: 切ると 3D の大きさも筆圧そのものに戻る
    b.base.pressure_size = false;
    assert_eq!(one_dab(&b, 0.0).0, full.0);

    // 硬さ: 筆圧が低いほど縁が柔らかい（縁の画素のアルファが減る）
    let mut soft = brush(BrushEffect::Paint);
    soft.base.radius = 5.0;
    soft.controls.pressure_hardness = true;
    soft.pressure.hardness = PressureResponse::new(0.0, vec![]).unwrap();
    let light = one_dab(&soft, 0.2).0;
    let firm = one_dab(&soft, 1.0).0;
    let sum = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| u32::from(p[3])).sum::<u32>();
    assert!(sum(&light) < sum(&firm), "{} {}", sum(&light), sum(&firm));

    // 不透明度: 画素ごとの塗りも、ストロークと同じ応えを通す
    let mut o = brush(BrushEffect::Paint);
    o.base.pressure_opacity = true;
    o.pressure.opacity = PressureResponse::new(0.5, vec![]).unwrap();
    let faint = one_dab(&o, 0.0).0;
    let strong = one_dab(&o, 1.0).0;
    let max_alpha = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| p[3]).max().unwrap();
    assert!(
        (i32::from(max_alpha(&faint)) - 128).abs() <= 1,
        "{}",
        max_alpha(&faint)
    );
    assert_eq!(max_alpha(&strong), 255);
}

/// 速い入力の線: 左のアイランドの左端から右のアイランドの右端まで、1 回の入力で動かす（間隔を細かくして、区間のダブを 1 回の入力で塗る数より
/// ずっと多くする）。押した点・遠い点・その先の点の 3 つで、2 つ目の入力が長い区間を描けるようにする（曲線で結ぶブラシ: 最新の区間は
/// その先の点が来るまで待つので、長い区間のダブは 2 つ目の入力でまとめて並ぶ）。
fn fast_line() -> (Arc<SurfaceGeometry>, CameraView, Brush, [Vec2; 3]) {
    let g = plane();
    let view = front(&g);
    let mut b = brush(BrushEffect::Paint);
    b.base.spacing = 0.01;
    b.assist.curve = true;
    let points = [
        screen(&view, p(0.05, 0.5)),
        screen(&view, p(1.95, 0.5)),
        screen(&view, p(1.95, 0.45)),
    ];
    (g, view, b, points)
}

/// how の決まりで速い入力の線を引き、確定したレイヤーの画素と、最初の入力で塗らずに持ち越したダブの数を返す。
fn fast_stroke(
    effect: SurfaceEffect,
    how: impl Fn(&mut Document, &mut SurfaceStroke, &mut Stroke),
) -> (Vec<u8>, usize, usize) {
    let (g, view, mut b, [from, far, next]) = fast_line();
    let (mut d, layer) = document();
    let undo = d.undo_count();
    if effect == SurfaceEffect::Smudge {
        b.effect = BrushEffect::Smudge { strength: 1.0 };
        for y in 0..H {
            for x in 0..W {
                let v = if (x / 2 + y / 2) % 2 == 0 { 255 } else { 0 };
                d.set_pixel(layer, x, y, Rgba8::new(v, 40, 255 - v, 255))
                    .unwrap();
            }
        }
        d.clear_history().unwrap();
    }
    let undo = if effect == SurfaceEffect::Smudge {
        0
    } else {
        undo
    };
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &b,
        from,
        SurfaceStrokeOptions {
            effect,
            ..Default::default()
        },
    );
    let mut s = s.unwrap();
    s.add(&mut d, &mut stroke, far, 1.0).unwrap();
    s.add(&mut d, &mut stroke, next, 1.0).unwrap();
    let held = s.queued();
    how(&mut d, &mut s, &mut stroke);
    s.finish(&mut d, &mut stroke).unwrap();
    assert_eq!(s.queued(), 0, "離したら残りを全部塗る");
    let dabs = s.stats.dabs;
    assert!(d.end_stroke(stroke).unwrap().changed);
    assert_eq!(d.undo_count(), undo + 1, "1 本のストローク");
    (bytes(&d, layer), held, dabs)
}

/// 3D ビューで速く動かして 1 回の入力の区間が長くなっても、ストロークは消えない: 区間のダブを全部並べ、入力では塗らずに持ち越す
/// （塗るのは呼ぶ側がフレームごとに、時間の枠まで）。持ち越した分は、1 フレームに 1 ダブずつ・決まった数ずつ・本物の時間の枠で塗っても、
/// 離したときにまとめて塗っても、全部すぐ塗ったときと同じ画素（並べるときに当たりと大きさを決めるので、塗る時・区切りによらない）。
/// 指先（直前のダブの面の点を読む）も同じ。
#[test]
fn a_long_single_input_keeps_the_stroke_and_paints_the_same_pixels_whenever_the_rest_is_painted() {
    for effect in [SurfaceEffect::Paint, SurfaceEffect::Smudge] {
        // すぐ全部塗る（持ち越し無し）
        let (at_once, held, dabs) = fast_stroke(effect, |d, s, stroke| {
            s.paint_queued(d, stroke, usize::MAX).unwrap();
        });
        assert!(dabs > 256, "{effect:?}: 試験の前提: 長い区間 ({dabs})");
        assert!(
            held > 256,
            "{effect:?}: 入力は塗らずに、区間のダブを全部持ち越す ({held})"
        );
        // 1 フレームに 1 ダブずつ（時間の枠が最小）
        let (one_by_one, _, one_dabs) = fast_stroke(effect, |d, s, stroke| {
            let mut frames = 0;
            while s.queued() > 0 {
                let painted = s.paint_queued_while(d, stroke, |_| false).unwrap();
                assert_eq!(painted, 1, "時間の枠が空でも 1 つは塗る");
                frames += 1;
            }
            assert!(frames > 256, "{effect:?}: 1 ダブずつ {frames} フレーム");
        });
        // 決まった数ずつ（ふつうの枠の見立て）
        let (by_count, _, count_dabs) = fast_stroke(effect, |d, s, stroke| {
            let mut frames = 0;
            while s.queued() > 0 {
                let painted = s.paint_queued_while(d, stroke, |n| n < 37).unwrap();
                assert!(painted <= 37);
                frames += 1;
            }
            assert!(frames > 1, "{effect:?}: 何フレームかに分けて塗った");
        });
        // 本物の時計の枠（切れ目はそのときの速さで変わるが、結果は同じ）
        let (by_clock, _, clock_dabs) = fast_stroke(effect, |d, s, stroke| {
            while s.queued() > 0 {
                let deadline = std::time::Instant::now() + std::time::Duration::from_micros(300);
                assert!(s.paint_queued_until(d, stroke, deadline).unwrap() >= 1);
            }
        });
        // 離したときにまとめて
        let (on_release, _, release_dabs) = fast_stroke(effect, |_, _, _| {});
        assert_eq!(
            (one_dabs, count_dabs, clock_dabs, release_dabs),
            (dabs, dabs, dabs, dabs),
            "{effect:?}"
        );
        assert!(one_by_one == at_once, "{effect:?}: 1 ダブずつでも同じ画素");
        assert!(
            by_count == at_once,
            "{effect:?}: 決まった数ずつでも同じ画素"
        );
        assert!(by_clock == at_once, "{effect:?}: 時計の枠でも同じ画素");
        assert!(
            on_release == at_once,
            "{effect:?}: 離したときに塗っても同じ画素"
        );
        // 線は右のアイランドの右端まで届いている
        let painted = |x: usize, y: usize| at_once[(y * W as usize + x) * 4 + 3] > 0;
        if effect == SurfaceEffect::Paint {
            assert!(
                (0..H as usize).any(|y| painted(2, y)),
                "左のアイランドの左端"
            );
            assert!(
                (0..H as usize).any(|y| painted(30, y)),
                "右のアイランドの右端"
            );
        }
    }
}

/// 時間の枠: 締め切りがもう過ぎていても最初の 1 つは塗り（進まないことが無い）、遠い締め切りなら全部塗る。空の待ち行列には何もしない。
#[test]
fn a_deadline_always_lets_one_dab_through_and_a_far_one_lets_them_all() {
    let (g, view, b, [from, far, next]) = fast_line();
    let (mut d, layer) = document();
    let (mut stroke, s) = begin(
        &mut d,
        layer,
        &g,
        view,
        &b,
        from,
        SurfaceStrokeOptions::default(),
    );
    let mut s = s.unwrap();
    s.add(&mut d, &mut stroke, far, 1.0).unwrap();
    s.add(&mut d, &mut stroke, next, 1.0).unwrap();
    let held = s.queued();
    assert!(held > 256);
    let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
    assert_eq!(s.paint_queued_until(&mut d, &mut stroke, past).unwrap(), 1);
    assert_eq!(s.queued(), held - 1);
    let later = std::time::Instant::now() + std::time::Duration::from_secs(600);
    assert_eq!(
        s.paint_queued_until(&mut d, &mut stroke, later).unwrap(),
        held - 1,
        "遠い締め切りなら、塗れるものを全部塗る"
    );
    assert_eq!(s.queued(), 0);
    assert_eq!(s.paint_queued_until(&mut d, &mut stroke, past).unwrap(), 0);
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
}

/// 離す（`end_input`）は残りを塗らずに並べるだけで、その後の時間の枠の塗りと、確定は、離したときに全部塗る `finish` と同じ画素・同じ
/// 1 本の Undo。`end_input` は 2 回呼んでも何も足さない。
#[test]
fn ending_the_input_leaves_the_rest_to_be_painted_in_time_boxes_and_commits_once() {
    let run = |boxed: bool| {
        let (g, view, b, [from, far, next]) = fast_line();
        let (mut d, layer) = document();
        let undo = d.undo_count();
        let (mut stroke, s) = begin(
            &mut d,
            layer,
            &g,
            view,
            &b,
            from,
            SurfaceStrokeOptions::default(),
        );
        let mut s = s.unwrap();
        s.add(&mut d, &mut stroke, far, 1.0).unwrap();
        s.add(&mut d, &mut stroke, next, 1.0).unwrap();
        assert!(!s.input_ended());
        if boxed {
            s.end_input().unwrap();
            assert!(s.input_ended());
            let queued = s.queued();
            s.end_input().unwrap();
            assert_eq!(s.queued(), queued, "2 回目は何も足さない");
            assert!(queued > 256, "離しても塗らずに持ち越す ({queued})");
            let mut frames = 0;
            while s.queued() > 0 {
                s.paint_queued_while(&mut d, &mut stroke, |n| n < 50)
                    .unwrap();
                frames += 1;
                assert!(d.has_active_stroke(), "残りがある間は描いている最中のまま");
            }
            assert!(frames > 2);
        } else {
            s.finish(&mut d, &mut stroke).unwrap();
        }
        assert_eq!(s.queued(), 0);
        assert!(d.end_stroke(stroke).unwrap().changed);
        assert_eq!(d.undo_count(), undo + 1, "1 本のストローク");
        bytes(&d, layer)
    };
    assert!(run(true) == run(false));
}

/// 持ち越しても、1 回の操作のメモリの上限は今のまま: 持ち越したダブで予算を超えたら、ストロークを取り消して断る（途中まで塗った
/// 画素も戻る）。予算は、最初の入力で塗る左のアイランドの分は入り、持ち越した右のアイランドの分で超える値を探す（投影の塗りの覚えは別に与えて、
/// 予算は巻き戻しの写しだけに効かせる）。
#[test]
fn a_held_over_dab_past_the_memory_budget_still_cancels_the_stroke() {
    let (g, view, b, [from, far, next]) = fast_line();
    let mut found = false;
    for budget in (256u64..=64 << 10).step_by(64) {
        let (mut d, layer) = document();
        let (before, undo) = (bytes(&d, layer), d.undo_count());
        d.set_stroke_budget_bytes(budget).unwrap();
        let (mut stroke, s) = begin(
            &mut d,
            layer,
            &g,
            view,
            &b,
            from,
            SurfaceStrokeOptions {
                projection_memory: Some(64 << 20),
                ..Default::default()
            },
        );
        let Ok(mut s) = s else {
            d.cancel_stroke(stroke);
            continue;
        };
        if s.add(&mut d, &mut stroke, far, 1.0).is_err()
            || s.add(&mut d, &mut stroke, next, 1.0).is_err()
        {
            d.cancel_active_stroke();
            continue;
        }
        assert!(s.queued() > 0);
        match s.finish(&mut d, &mut stroke) {
            Ok(()) => {
                d.end_stroke(stroke).unwrap();
                break;
            }
            Err(e) => {
                assert_eq!(
                    e,
                    SurfaceStrokeError::Core(yolu_core::CoreError::StrokeBudgetExceeded)
                );
                assert!(!d.has_active_stroke(), "core が取り消した");
                assert_eq!(bytes(&d, layer), before, "途中まで塗った画素も戻る");
                assert_eq!(d.undo_count(), undo);
                found = true;
                break;
            }
        }
    }
    assert!(found, "最初の入力は入り、持ち越した分で超える予算がある");
}
