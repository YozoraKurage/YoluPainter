//! ブラシの縁のアンチエイリアスの 3D: 帯はテクセルの幅で、投影の画素ごとのテクセルの画面の大きさで画面の距離へ直す。カメラに正面を向けた
//! 平らな四角（UV が文書の全体）で、同じブラシ・同じ点の列を 2D と 3D に描いて比べる（テクスチャの解像度を 2 倍にしても、帯はテクセルの
//! 幅のまま）。柔らかい筆先・なし は今のバイトのまま。見えない面にも塗る対称の写し（面のダブ）にも帯が付く。
#![allow(clippy::chunks_exact_to_as_chunks)]
use std::sync::Arc;

use yolu_core::geometry::{
    pick, CameraView, MirrorPlane, OrbitCamera, SurfaceBrushBudget, SurfaceGeometry, SurfaceStroke,
    SurfaceStrokeOptions, SurfaceSymmetrySetup, SurfaceTriangle, SymmetryAxis,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{DVec2, Quat, Vec2, Vec3};
use yolu_core::{AntiAlias, Brush, BrushSettings, Channel, Document, LayerId, Rgba8};

const W: u32 = 96;
const H: u32 = 64;
const MARGIN: f32 = 16.0;

/// 世界の x 0..W・y 0..H の四角（UV が文書の全体）。
fn quad() -> Arc<SurfaceGeometry> {
    let (w, h) = (W as f32, H as f32);
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let uv = |x: f32, y: f32| Vec2::new(x / w, y / h);
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
        .with_slot(0, 0, 0)
    };
    Arc::new(
        SurfaceGeometry::new(
            vec![
                tri((0.0, 0.0), (0.0, h), (w, 0.0)),
                tri((w, 0.0), (0.0, h), (w, h)),
            ],
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    )
}

/// 四角を正面に見るカメラ（世界の 1 単位が画面の 1 点）。
fn front() -> CameraView {
    let (sw, sh) = (W as f32 + 2.0 * MARGIN, H as f32 + 2.0 * MARGIN);
    let distance = sh / (2.0 * (15.0f32).to_radians().tan());
    OrbitCamera {
        target: Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: W as f32,
        ..Default::default()
    }
    .view(sw, sh)
}

/// 世界の点 (x, y) の画面の点。
fn screen(x: f64, y: f64) -> Vec2 {
    Vec2::new(
        (x + MARGIN as f64) as f32,
        (H as f64 + MARGIN as f64 - y) as f32,
    )
}

fn document(scale: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W * scale, H * scale, 32).unwrap();
    let l = d.add_layer("paint").unwrap();
    (d, l)
}

fn bytes(d: &Document, l: LayerId) -> Vec<u8> {
    let (w, h) = (d.width(), d.height());
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or_else(|| vec![0; (w * h * 4) as usize], |s| s.to_canvas_bytes())
}

fn brush(level: AntiAlias, radius: f64, hardness: f64) -> Brush {
    Brush::from(BrushSettings {
        radius,
        hardness,
        spacing: 0.0625,
        color: Rgba8::new(20, 40, 160, 255),
        pressure_size: false,
        pressure_opacity: false,
        anti_alias: level,
        ..BrushSettings::default()
    })
}

/// 2D: 世界の点の列（文書の画素では × scale）、半径も世界の単位（× scale）。
fn draw_2d(b: &Brush, points: &[(f64, f64)], scale: u32) -> Vec<u8> {
    let s = scale as f64;
    let mut b = b.clone();
    b.base.radius *= s;
    let (mut d, l) = document(scale);
    let mut st = d.begin_brush_stroke(l, &b).unwrap();
    for &(x, y) in points {
        st.add_point(&mut d, x * s, y * s, 1.0, DVec2::ZERO)
            .unwrap();
    }
    d.end_stroke(st).unwrap();
    bytes(&d, l)
}

/// 3D: 同じ点の列を四角に描く（画面の半径を世界の単位の半径にそろえる）。
fn draw_3d(
    b: &Brush,
    points: &[(f64, f64)],
    scale: u32,
    symmetry: Option<SurfaceSymmetrySetup>,
) -> Vec<u8> {
    let g = quad();
    let (mut d, l) = document(scale);
    let mut b = b.clone();
    b.base.radius *= d.width() as f64 / g.brush_scale() as f64;
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let (x0, y0) = points[0];
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        front(),
        &b.base,
        Some(0),
        screen(x0, y0),
        1.0,
        SurfaceStrokeOptions {
            symmetry,
            ..SurfaceStrokeOptions::default()
        },
    )
    .unwrap();
    for &(x, y) in &points[1..] {
        s.add(&mut d, &mut stroke, screen(x, y), 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
}

/// 塗った画素の数・アルファが違う画素の数・アルファの差のいちばん大きい値。
fn compare(a: &[u8], b: &[u8]) -> (usize, usize, u8) {
    let (mut painted, mut differ, mut max) = (0, 0, 0u8);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if p[3] > 0 || q[3] > 0 {
            painted += 1;
        }
        let d = p[3].abs_diff(q[3]);
        if d > 0 {
            differ += 1;
            max = max.max(d);
        }
    }
    (painted, differ, max)
}

fn partial(a: &[u8]) -> usize {
    a.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 255).count()
}

const LEVELS: [AntiAlias; 3] = [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong];

#[test]
fn the_3d_edge_matches_the_2d_edge_on_a_facing_plane_at_any_texture_size() {
    let line = [(20.25, 18.5), (50.0, 30.25), (74.5, 44.0)];
    for scale in [1u32, 2] {
        // ダブの間隔はどれも 0.75（3D の間隔の下限 0.5 点より広く、2D と同じ数のダブが同じ所に並ぶ）
        for (radius, spacing, label) in [
            (6.0, 0.0625, "硬い丸"),
            (0.6, 0.625, "細い線"),
            (0.3, 1.25, "1 画素より小さい"),
        ] {
            for level in LEVELS {
                let mut b = brush(level, radius, 1.0);
                b.base.spacing = spacing;
                let two = draw_2d(&b, &line, scale);
                let three = draw_3d(&b, &line, scale, None);
                let (painted, differ, max) = compare(&two, &three);
                println!("{label} 倍率 {scale} {level:?}: 塗った {painted}・違う {differ}・差の最大 {max}・帯 2D {}・3D {}", partial(&two), partial(&three));
                assert!(painted > 0);
                assert!(
                    partial(&three) > 0,
                    "{label} {scale} {level:?}: 3D に帯が無い"
                );
                // 2D は f32 の行の核、3D は倍精度の式だが、この並びでは同じバイトになる
                assert_eq!(
                    (differ, max),
                    (0, 0),
                    "{label} {scale} {level:?}: 違う画素 {differ} / {painted}"
                );
            }
        }
    }
}

#[test]
fn soft_and_none_brushes_keep_todays_3d_bytes() {
    let line = [(20.25, 18.5), (50.0, 30.25), (74.5, 44.0)];
    for (radius, hardness) in [(9.0, 0.4), (4.0, 1.0)] {
        let none = draw_3d(&brush(AntiAlias::None, radius, hardness), &line, 1, None);
        if hardness == 1.0 {
            // なし の硬い丸は 2 値
            assert_eq!(partial(&none), 0);
            continue;
        }
        for level in LEVELS {
            assert_eq!(
                draw_3d(&brush(level, radius, hardness), &line, 1, None),
                none,
                "{level:?}: 柔らかい筆先は今のバイト"
            );
        }
    }
}

#[test]
fn surface_copies_that_ignore_visibility_get_the_band_too() {
    let center = Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0);
    let mirror = MirrorPlane::from_model(center, Quat::IDENTITY, SymmetryAxis::X, 0.0);
    let line = [(20.25, 22.5), (30.0, 40.25)];
    for level in [AntiAlias::None, AntiAlias::Medium] {
        let a = draw_3d(
            &brush(level, 5.0, 1.0),
            &line,
            1,
            Some(SurfaceSymmetrySetup {
                mirror: Some(mirror),
                radial: None,
                ignore_visibility: true,
            }),
        );
        // 写しの側（x が中心より右）の帯の画素
        let copy = a
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| (i % W as usize) >= W as usize / 2 && p[3] > 0 && p[3] < 255)
            .count();
        let painted = a
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| (i % W as usize) >= W as usize / 2 && p[3] > 0)
            .count();
        assert!(painted > 0, "{level:?}: 写しを塗った");
        if level == AntiAlias::None {
            assert_eq!(copy, 0);
        } else {
            assert!(copy > 0, "写しの面のダブにも帯");
        }
    }
}

/// 四角を横（ヨー）と縦（ピッチ）に回した向きから見るカメラ（テクスチャの横の画素が画面で縮み、奥ほど小さく写る）。
fn turned(yaw: f32, pitch: f32) -> CameraView {
    let (sw, sh) = (W as f32 + 2.0 * MARGIN, H as f32 + 2.0 * MARGIN);
    let distance = sh / (2.0 * (15.0f32).to_radians().tan()) * 0.8;
    OrbitCamera {
        target: Vec3::new(W as f32 / 2.0, H as f32 / 2.0, 0.0),
        yaw,
        pitch,
        distance,
        model_radius: W as f32,
        ..Default::default()
    }
    .view(sw, sh)
}

/// 画面の点 at に 1 つのダブを置いた、テクスチャのアルファと、中心のテクセルの位置。
fn one_dab(view: CameraView, level: AntiAlias, radius: f64, at: Vec2) -> (Vec<u8>, (f64, f64)) {
    let g = quad();
    let (mut d, l) = document(1);
    let mut b = brush(level, radius, 1.0);
    b.base.radius *= d.width() as f64 / g.brush_scale() as f64;
    let hit = pick(&g, &view, at).expect("四角の上");
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g.clone(),
        view,
        &b.base,
        Some(0),
        at,
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    let alpha = bytes(&d, l).chunks_exact(4).map(|p| p[3]).collect();
    (
        alpha,
        (hit.uv.x as f64 * W as f64, hit.uv.y as f64 * H as f64),
    )
}

/// 中心を通る行（横）と列（縦）の、中心から外へ向かう 4 つの向きの、縁の帯のテクセル（覆い 0.02〜0.98）の数。
fn band_texels(alpha: &[u8], (cx, cy): (f64, f64)) -> [usize; 4] {
    let at = |x: i64, y: i64| -> f64 {
        if x < 0 || y < 0 || x >= W as i64 || y >= H as i64 {
            0.0
        } else {
            alpha[(y * W as i64 + x) as usize] as f64 / 255.0
        }
    };
    let (x0, y0) = (cx.floor() as i64, cy.floor() as i64);
    let count = |dx: i64, dy: i64| {
        (1..64)
            .map(|k| at(x0 + dx * k, y0 + dy * k))
            .filter(|&c| c > 0.02 && c < 0.98)
            .count()
    };
    [count(-1, 0), count(1, 0), count(0, -1), count(0, 1)]
}

#[test]
fn the_band_stays_one_texel_wide_on_turned_and_receding_faces() {
    // 横に 60° 回した面（横のテクセルは画面で半分ほどに縮み、左右で奥行きが違う）と、さらに縦にも回した面。帯は投影の画素ごとの
    // テクセルの画面の大きさで直すので、テクスチャの上では、どの向きにもテクセルの段の幅（強 2・中 1）になる
    for (yaw, pitch) in [(60.0, 0.0), (55.0, 35.0)] {
        let view = turned(yaw, pitch);
        let at = Vec2::new(view.width / 2.0, view.height / 2.0);
        let (none, c) = one_dab(view, AntiAlias::None, 9.0, at);
        assert_eq!(band_texels(&none, c), [0; 4], "なし は 2 値");
        for (level, lo, hi) in [(AntiAlias::Medium, 0, 2), (AntiAlias::Strong, 1, 3)] {
            let (alpha, c) = one_dab(view, level, 9.0, at);
            let counts = band_texels(&alpha, c);
            println!("ヨー {yaw} ピッチ {pitch} {level:?}: {counts:?}");
            for n in counts {
                assert!(
                    (lo..=hi).contains(&n),
                    "ヨー {yaw} ピッチ {pitch} {level:?}: {counts:?}"
                );
            }
            assert!(counts.iter().sum::<usize>() > 4 * lo, "{counts:?}");
        }
    }
}

#[test]
fn surface_dabs_of_soft_tips_keep_todays_coverage_bits() {
    // 面のダブ（モデルの空間の球）: 帯がぼかしの幅以下の画素は、なし と同じ式・同じ距離（元の半径で割る）で、覆いの f32 のビットまで同じ
    let g = quad();
    let view = front();
    let hit = pick(&g, &view, screen(48.25, 30.5)).unwrap();
    let budget = SurfaceBrushBudget::default();
    for (radius, hardness) in [(8.0f32, 0.3f32), (8.0, 0.6), (3.0, 0.0)] {
        let none = g.build_surface_dabs(
            &hit,
            radius,
            W as i32,
            H as i32,
            view.position,
            hardness,
            &budget,
            None,
            false,
        );
        assert!(!none.pixels.is_empty());
        for level in LEVELS {
            let aa = g.build_surface_dabs_anti_aliased(
                &hit,
                radius,
                W as i32,
                H as i32,
                view.position,
                hardness,
                level,
                &budget,
                None,
                false,
            );
            assert_eq!(
                aa.pixels.len(),
                none.pixels.len(),
                "{radius} {hardness} {level:?}"
            );
            for (a, b) in aa.pixels.iter().zip(&none.pixels) {
                assert_eq!(
                    (a.x, a.y, a.coverage.to_bits()),
                    (b.x, b.y, b.coverage.to_bits()),
                    "{radius} {hardness} {level:?}"
                );
            }
        }
    }
    // 硬い丸には帯が付く
    let none = g.build_surface_dabs(
        &hit,
        8.0,
        W as i32,
        H as i32,
        view.position,
        1.0,
        &budget,
        None,
        false,
    );
    let aa = g.build_surface_dabs_anti_aliased(
        &hit,
        8.0,
        W as i32,
        H as i32,
        view.position,
        1.0,
        AntiAlias::Medium,
        &budget,
        None,
        false,
    );
    assert!(none.pixels.iter().all(|p| p.coverage == 1.0));
    assert!(aa
        .pixels
        .iter()
        .any(|p| p.coverage > 0.0 && p.coverage < 1.0));
}
