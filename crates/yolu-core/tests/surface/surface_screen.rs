//! 3D ビューの画面の形（グラデーション・図形の塗り）を見える面へ写す `cover_screen` と、テクセルの点でグラデーションを決める文書の塗りの
//! 不変条件: 見える表の面だけ（隠れた面・裏の面は 0）・テクセルの画面の位置・長方形と楕円の中と外・アンチエイリアスの帯（テクセルの幅）・
//! 正投影・並列の度合いによらない結果・予算で断る・2D のグラデーションのバイトは変わらない・画面のグラデーションの色と選択範囲。
//! 期待は、テクセルを UV の重心座標で 3D の点へ戻し、カメラで画面へ写した素朴な計算から出す。

use std::collections::BTreeMap;
use std::sync::Arc;

use yolu_core::blend::blend;
use yolu_core::geometry::{
    cover_screen, CameraView, DabRefusal, OrbitCamera, Projection, ProjectionSettings,
    ScreenCoverSettings, ScreenCoverage, ScreenShape, SurfaceGeometry, SurfaceStrokeError,
    SurfaceTriangle, DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::material::{ChannelPaint, GradientSettings, GradientShape};
use yolu_core::{BlendMode, Channel, Document, LayerId, Rgba8, SelectionMask};

const SIZE: u32 = 64;

fn geometry(triangles: Vec<SurfaceTriangle>) -> Arc<SurfaceGeometry> {
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// z の高さの四角（x0..x1 × y0..y1）。UV は uv0..uv1 の長方形。toward が負なら法線は −Z（−Z の側のカメラを向く）。
fn quad(
    out: &mut Vec<SurfaceTriangle>,
    z: f32,
    (x0, x1): (f32, f32),
    (y0, y1): (f32, f32),
    (uv0, uv1): (Vec2, Vec2),
    toward: f32,
) {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    let uv = |x: f32, y: f32| {
        Vec2::new(
            uv0.x + (uv1.x - uv0.x) * (x - x0) / (x1 - x0),
            uv0.y + (uv1.y - uv0.y) * (y - y0) / (y1 - y0),
        )
    };
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        let (b, c) = if toward < 0.0 { (b, c) } else { (c, b) };
        SurfaceTriangle::new(
            p(a.0, a.1),
            p(b.0, b.1),
            p(c.0, c.1),
            uv(a.0, a.1),
            uv(b.0, b.1),
            uv(c.0, c.1),
        )
    };
    out.push(tri((x0, y0), (x0, y1), (x1, y0)));
    out.push(tri((x1, y0), (x0, y1), (x1, y1)));
}

/// 手前の表の面（UV の左下の 4 分の 1）・その後ろに隠れた面（右下）・横の裏の面（左上）。
fn scene() -> Arc<SurfaceGeometry> {
    let mut t = Vec::new();
    quad(
        &mut t,
        0.0,
        (-1.0, 1.0),
        (-1.0, 1.0),
        (Vec2::new(0.0, 0.0), Vec2::new(0.5, 0.5)),
        -1.0,
    );
    quad(
        &mut t,
        1.0,
        (-0.5, 0.5),
        (-0.5, 0.5),
        (Vec2::new(0.5, 0.0), Vec2::new(1.0, 0.5)),
        -1.0,
    );
    quad(
        &mut t,
        0.0,
        (1.3, 1.9),
        (-0.5, 0.5),
        (Vec2::new(0.0, 0.5), Vec2::new(0.5, 1.0)),
        1.0,
    );
    geometry(t)
}

fn camera(projection: Projection) -> CameraView {
    OrbitCamera {
        target: Vec3::new(0.4, 0.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        distance: 6.0,
        model_radius: 1.0,
        projection,
    }
    .view(220.0, 200.0)
}

fn perspective() -> CameraView {
    camera(Projection::default())
}

/// 投影の切り替え: 面の向きの弱めとにじみを切る（覆いを形の覆いだけにする）。
fn plain() -> ProjectionSettings {
    ProjectionSettings {
        angle_falloff: false,
        seam_bleed: 0,
        ..ProjectionSettings::default()
    }
}

fn settings(shape: ScreenShape, positions: bool) -> ScreenCoverSettings {
    ScreenCoverSettings {
        material: Some(0),
        projection: plain(),
        shape,
        positions,
        room: 512 << 20,
    }
}

fn document() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(SIZE, SIZE, 16).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

/// UV の四角 uv0..uv1 に入るテクセルの中心の、四角（x0..x1 × y0..y1、高さ z）の上の 3D の点。
fn texels(
    (uv0, uv1): (Vec2, Vec2),
    (x0, x1): (f32, f32),
    (y0, y1): (f32, f32),
    z: f32,
) -> BTreeMap<(u32, u32), Vec3> {
    let mut out = BTreeMap::new();
    let s = SIZE as f32;
    for ty in (uv0.y * s) as u32..(uv1.y * s) as u32 {
        for tx in (uv0.x * s) as u32..(uv1.x * s) as u32 {
            let u = ((tx as f32 + 0.5) / s - uv0.x) / (uv1.x - uv0.x);
            let v = ((ty as f32 + 0.5) / s - uv0.y) / (uv1.y - uv0.y);
            out.insert(
                (tx, ty),
                Vec3::new(x0 + (x1 - x0) * u, y0 + (y1 - y0) * v, z),
            );
        }
    }
    out
}

fn front() -> BTreeMap<(u32, u32), Vec3> {
    texels(
        (Vec2::new(0.0, 0.0), Vec2::new(0.5, 0.5)),
        (-1.0, 1.0),
        (-1.0, 1.0),
        0.0,
    )
}

fn amount(c: &ScreenCoverage, x: u32, y: u32) -> u8 {
    c.mask().amount(x, y)
}

fn screen(view: &CameraView, p: Vec3) -> DVec2 {
    view.to_screen(p).expect("カメラの前").as_dvec2()
}

#[test]
fn the_whole_view_reaches_every_front_texel_with_its_screen_point_and_no_hidden_or_back_texel() {
    let (doc, _) = document();
    let view = perspective();
    let c = cover_screen(&doc, scene(), &view, &settings(ScreenShape::View, true)).unwrap();
    let front = front();
    for (&(x, y), &p) in &front {
        assert_eq!(amount(&c, x, y), 255, "表の面 ({x}, {y})");
        let s = c.screen(x, y).expect("位置");
        assert!(s.distance(screen(&view, p)) < 0.01, "({x}, {y}) {s}");
    }
    let mut lit = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if amount(&c, x, y) > 0 {
                lit += 1;
                assert!(front.contains_key(&(x, y)), "隠れた面・裏の面 ({x}, {y})");
            }
        }
    }
    assert_eq!(lit, front.len());
}

/// 手前の面のテクセル (x, y) の中の点（中心からテクセルの単位で o だけずれた所）の世界の点（UV の左下の 4 分の 1 → x・y が −1〜1）。
fn front_point(x: u32, y: u32, o: DVec2) -> Vec3 {
    let s = SIZE as f64;
    let u = (x as f64 + 0.5 + o.x) / s;
    let v = (y as f64 + 0.5 + o.y) / s;
    Vec3::new((-1.0 + 4.0 * u) as f32, (-1.0 + 4.0 * v) as f32, 0.0)
}

const OFFSETS: [f64; 4] = [-0.375, -0.125, 0.125, 0.375];

/// テクセルを 4 × 4 の点で見た、形の中（distance が 0 以下）の点の数からの量（2D の選択範囲の形と同じ丸め）と、縁に近い点があるか。
fn sampled(view: &CameraView, x: u32, y: u32, distance: impl Fn(DVec2) -> f64) -> (u8, bool) {
    let mut count = 0u32;
    let mut near = false;
    for oy in OFFSETS {
        for ox in OFFSETS {
            let d = distance(screen(view, front_point(x, y, DVec2::new(ox, oy))));
            near |= d.abs() < 0.05;
            if d <= 0.0 {
                count += 1;
            }
        }
    }
    (((count * 255 + 8) / 16) as u8, near)
}

#[test]
fn rectangles_and_ellipses_count_four_by_four_points_of_each_texel_on_screen() {
    let (doc, _) = document();
    let (a, b) = (DVec2::new(70.3, 60.1), DVec2::new(150.7, 125.4));
    let center = (a + b) * 0.5;
    let half = (b - a) * 0.5;
    // 正投影ではテクセルの写しが線形なので点の数がそのまま一致し、透視では写しをテクセルの中心の一次で近似するので、縁で 1 点まで違いうる
    for (view, slack) in [
        (camera(Projection::Orthographic { height: 3.0 }), 0),
        (perspective(), 16),
    ] {
        for shape in [
            ScreenShape::Rectangle { a, b, corner: 0.0 },
            ScreenShape::Rectangle {
                a: b,
                b: a,
                corner: 0.0,
            },
            ScreenShape::Ellipse { a, b },
        ] {
            let c = cover_screen(&doc, scene(), &view, &settings(shape, false)).unwrap();
            let distance = |s: DVec2| match shape {
                // 符号は楕円の式、大きさはおおよその画面の距離（縁に近い点を見分けるだけ）
                ScreenShape::Ellipse { .. } => {
                    (((s - center) / half).length() - 1.0) * half.min_element()
                }
                _ => {
                    let q = (s - center).abs() - half;
                    q.x.max(q.y)
                }
            };
            let (mut full, mut partial) = (0, 0);
            for &(x, y) in front().keys() {
                let (want, near) = sampled(&view, x, y, distance);
                let got = amount(&c, x, y);
                if got == 255 {
                    full += 1;
                } else if got > 0 {
                    partial += 1;
                }
                if near && slack == 0 {
                    continue;
                }
                assert!(
                    (got as i32 - want as i32).abs() <= slack,
                    "{shape:?} ({x}, {y}) {got} {want}"
                );
            }
            assert!(full > 50 && partial > 10, "{shape:?} {full} {partial}");
        }
    }
}

#[test]
fn a_fill_seen_straight_on_counts_the_same_amounts_as_the_2d_shapes() {
    // 正投影で真正面から、テクセル 1 つが画面の 2 点の正方形に写るカメラ: 画面の形が、文書の画素の座標の同じ形になる
    let (doc, _) = document();
    let view = camera(Projection::Orthographic {
        height: 200.0 / 32.0,
    });
    let o = screen(&view, Vec3::ZERO);
    let ex = screen(&view, Vec3::X) - o;
    let ey = screen(&view, Vec3::Y) - o;
    assert!(
        (ex.x - 32.0).abs() < 1e-3 && (ey.y + 32.0).abs() < 1e-3,
        "{ex} {ey}"
    );
    // 画面の点 → 文書の画素の座標（手前の面は世界の −1〜1 が文書の 0〜32）
    let canvas = |s: DVec2| {
        DVec2::new(
            16.0 * ((s.x - o.x) / 32.0 + 1.0),
            16.0 * ((o.y - s.y) / 32.0 + 1.0),
        )
    };
    let (a, b) = (DVec2::new(73.1, 58.3), DVec2::new(122.9, 101.7));
    let (ca, cb) = (canvas(a), canvas(b));
    let polygon = SelectionMask::polygon(
        &doc,
        &[
            DVec2::new(ca.x, ca.y),
            DVec2::new(cb.x, ca.y),
            DVec2::new(cb.x, cb.y),
            DVec2::new(ca.x, cb.y),
        ],
    )
    .unwrap();
    let mid = (ca + cb) * 0.5;
    let r = (cb - ca).abs() * 0.5;
    let ellipse = SelectionMask::ellipse(&doc, mid.x, mid.y, r.x, r.y).unwrap();
    for (shape, two_d) in [
        (ScreenShape::Rectangle { a, b, corner: 0.0 }, &polygon),
        (ScreenShape::Ellipse { a, b }, &ellipse),
    ] {
        let c = cover_screen(&doc, scene(), &view, &settings(shape, false)).unwrap();
        let mut partial = 0;
        for &(x, y) in front().keys() {
            assert_eq!(amount(&c, x, y), two_d.amount(x, y), "{shape:?} ({x}, {y})");
            let v = two_d.amount(x, y);
            if v > 0 && v < 255 {
                partial += 1;
            }
        }
        assert!(partial > 10, "{shape:?} {partial}");
    }
}

/// 画面の点が多角形（偶奇の規則）の中か。
fn in_polygon(points: &[DVec2], p: DVec2) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 点から多角形のどれかの辺までの距離（縁に近い点を見分けるだけ）。
fn edge_distance(points: &[DVec2], p: DVec2) -> f64 {
    let mut best = f64::MAX;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        best = best.min((a + ab * t).distance(p));
    }
    best
}

#[test]
fn a_polygon_seen_straight_on_counts_the_same_amounts_as_the_2d_polygon() {
    // 正投影で真正面から、テクセル 1 つが画面の 2 点の正方形に写るカメラ（長方形・楕円の同じ試験と同じ）
    let (doc, _) = document();
    let view = camera(Projection::Orthographic {
        height: 200.0 / 32.0,
    });
    let o = screen(&view, Vec3::ZERO);
    let canvas = |s: DVec2| {
        DVec2::new(
            16.0 * ((s.x - o.x) / 32.0 + 1.0),
            16.0 * ((o.y - s.y) / 32.0 + 1.0),
        )
    };
    let v = DVec2::new;
    // 凸・凹（へこみのある形）・自分と交わる形（蝶ネクタイ。偶奇の規則で真ん中の三角は外）・細長い形
    let shapes: [Vec<DVec2>; 4] = [
        vec![
            v(73.1, 58.3),
            v(140.4, 70.2),
            v(150.9, 120.7),
            v(90.6, 140.1),
            v(66.2, 100.5),
        ],
        vec![
            v(60.3, 50.9),
            v(160.2, 55.1),
            v(158.8, 150.4),
            v(120.7, 98.3),
            v(62.5, 152.6),
        ],
        vec![
            v(63.7, 61.2),
            v(158.9, 140.3),
            v(61.4, 141.7),
            v(160.6, 62.9),
        ],
        vec![
            v(50.2, 100.1),
            v(170.9, 103.7),
            v(171.3, 109.2),
            v(51.8, 106.4),
        ],
    ];
    for (n, points) in shapes.iter().enumerate() {
        let two_d =
            SelectionMask::polygon(&doc, &points.iter().map(|&p| canvas(p)).collect::<Vec<_>>())
                .unwrap();
        let c = cover_screen(
            &doc,
            scene(),
            &view,
            &settings(ScreenShape::Polygon { points }, false),
        )
        .unwrap();
        let (mut partial, mut full) = (0, 0);
        for &(x, y) in front().keys() {
            assert_eq!(amount(&c, x, y), two_d.amount(x, y), "形 {n} ({x}, {y})");
            match two_d.amount(x, y) {
                0 => {}
                255 => full += 1,
                _ => partial += 1,
            }
        }
        assert!(partial > 10 && full > 5, "形 {n}: {partial} {full}");
    }
}

#[test]
fn a_polygon_counts_four_by_four_points_of_each_texel_on_screen_with_the_even_odd_rule() {
    let (doc, _) = document();
    let v = DVec2::new;
    // 蝶ネクタイと、へこみのある形
    let shapes: [Vec<DVec2>; 2] = [
        vec![
            v(63.7, 61.2),
            v(158.9, 140.3),
            v(61.4, 141.7),
            v(160.6, 62.9),
        ],
        vec![
            v(60.3, 50.9),
            v(160.2, 55.1),
            v(158.8, 150.4),
            v(120.7, 98.3),
            v(62.5, 152.6),
        ],
    ];
    // 正投影ではテクセルの写しが線形なので点の数がそのまま一致し、透視では写しをテクセルの中心の一次で近似するので、縁で 1 点まで違いうる
    for (view, slack) in [
        (camera(Projection::Orthographic { height: 3.0 }), 0),
        (perspective(), 16),
    ] {
        for points in &shapes {
            let c = cover_screen(
                &doc,
                scene(),
                &view,
                &settings(ScreenShape::Polygon { points }, false),
            )
            .unwrap();
            let (mut full, mut partial) = (0, 0);
            for &(x, y) in front().keys() {
                // 期待: テクセルを 4 × 4 の点で見て、画面の多角形の中の点の数（2D の選択範囲の形と同じ丸め）
                let (mut count, mut near) = (0u32, false);
                for oy in OFFSETS {
                    for ox in OFFSETS {
                        let s = screen(&view, front_point(x, y, DVec2::new(ox, oy)));
                        near |= edge_distance(points, s) < 0.05;
                        if in_polygon(points, s) {
                            count += 1;
                        }
                    }
                }
                let want = ((count * 255 + 8) / 16) as i32;
                let got = amount(&c, x, y);
                if got == 255 {
                    full += 1;
                } else if got > 0 {
                    partial += 1;
                }
                if near && slack == 0 {
                    continue;
                }
                assert!(
                    (got as i32 - want).abs() <= slack,
                    "{points:?} ({x}, {y}) {got} {want}"
                );
            }
            assert!(full > 20 && partial > 10, "{points:?} {full} {partial}");
        }
    }
}

#[test]
fn a_polygon_of_the_four_corners_is_the_rectangle_and_one_far_off_screen_is_the_same_shape() {
    let (doc, _) = document();
    let view = camera(Projection::Orthographic {
        height: 200.0 / 32.0,
    });
    let (a, b) = (DVec2::new(73.1, 58.3), DVec2::new(122.9, 101.7));
    let corners = [a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y)];
    let rect = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Rectangle { a, b, corner: 0.0 }, false),
    )
    .unwrap();
    let poly = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &corners }, false),
    )
    .unwrap();
    assert!(!rect.is_empty() && rect.mask() == poly.mask());
    // 表示域の外まで大きく広がる形（帯の表を作る範囲の外の辺を持つ）でも、画面の中は同じ形のまま
    let wide = [
        DVec2::new(a.x, -9000.0),
        DVec2::new(a.x, 9000.0),
        DVec2::new(b.x, 9000.0),
        DVec2::new(b.x, -9000.0),
    ];
    let tall = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &wide }, false),
    )
    .unwrap();
    let column = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(
            ScreenShape::Rectangle {
                a: DVec2::new(a.x, -9000.0),
                b: DVec2::new(b.x, 9000.0),
                corner: 0.0,
            },
            false,
        ),
    )
    .unwrap();
    assert!(!tall.is_empty() && tall.mask() == column.mask());
}

#[test]
fn a_polygon_covers_only_the_visible_front_texels() {
    // 表示域の全体を覆う多角形でも、隠れた面・裏の面は覆わない
    let (doc, _) = document();
    let view = perspective();
    let all = [
        DVec2::new(-10.0, -10.0),
        DVec2::new(240.0, -10.0),
        DVec2::new(240.0, 210.0),
        DVec2::new(-10.0, 210.0),
    ];
    let c = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &all }, false),
    )
    .unwrap();
    let front = front();
    let mut lit = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if amount(&c, x, y) > 0 {
                lit += 1;
                assert!(front.contains_key(&(x, y)), "隠れた面・裏の面 ({x}, {y})");
            }
        }
    }
    assert_eq!(lit, front.len());
}

#[test]
fn a_polygon_with_too_few_points_covers_nothing_and_bad_or_huge_input_or_a_small_budget_is_refused()
{
    let (doc, _) = document();
    let view = perspective();
    let two = [DVec2::new(40.0, 40.0), DVec2::new(160.0, 150.0)];
    let c = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &two }, false),
    )
    .unwrap();
    assert!(c.is_empty());
    // 一直線（面積が無い）
    let line = [
        DVec2::new(40.0, 40.0),
        DVec2::new(100.0, 40.0),
        DVec2::new(160.0, 40.0),
    ];
    let c = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &line }, false),
    )
    .unwrap();
    assert!(c.is_empty());
    let bad = [
        DVec2::new(40.0, 40.0),
        DVec2::new(f64::NAN, 100.0),
        DVec2::new(160.0, 150.0),
    ];
    assert!(matches!(
        cover_screen(
            &doc,
            scene(),
            &view,
            &settings(ScreenShape::Polygon { points: &bad }, false)
        ),
        Err(SurfaceStrokeError::Dab(DabRefusal::InvalidArguments))
    ));
    let many = vec![DVec2::new(1.0, 1.0); yolu_core::selection::MAX_POLYGON_POINTS + 1];
    assert!(matches!(
        cover_screen(
            &doc,
            scene(),
            &view,
            &settings(ScreenShape::Polygon { points: &many }, false)
        ),
        Err(SurfaceStrokeError::Dab(DabRefusal::InvalidArguments))
    ));
    // 縦に長い辺で細かく往復する線（辺 2 万本が約 150 の帯をまたぐ）: 帯ごとの辺の表が大きくなるので、予算が小さければ断る（結果を作りかけて落とさない）
    let zigzag: Vec<DVec2> = (0..20_000)
        .map(|i| DVec2::new(30.0 + i as f64 * 0.007, 20.0 + (i % 2) as f64 * 150.0))
        .collect();
    let small = ScreenCoverSettings {
        room: 1 << 20,
        ..settings(ScreenShape::Polygon { points: &zigzag }, false)
    };
    assert!(matches!(
        cover_screen(&doc, scene(), &view, &small),
        Err(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
    ));
    // 予算が十分なら、同じ形を覆える
    let c = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Polygon { points: &zigzag }, false),
    )
    .unwrap();
    assert!(!c.is_empty());
}

#[test]
fn the_polygon_table_is_counted_in_the_same_budget_as_the_buckets_and_candidates() {
    // 縦に長い辺で往復する多角形（辺 4000 本が約 150 の帯をまたぐ = 約 2.5 MB の帯の表）。表を引いた残りだけを区画・候補・溜めに渡すので、
    // 表の分に少し足しただけの予算では、表は作れても区画が作れずに断る。表の分と十分な残りがあれば通る
    let (doc, _) = document();
    let view = perspective();
    let zigzag: Vec<DVec2> = (0..4_000)
        .map(|i| DVec2::new(30.0 + i as f64 * 0.03, 20.0 + (i % 2) as f64 * 150.0))
        .collect();
    let shape = ScreenShape::Polygon { points: &zigzag };
    let table = cover_screen(&doc, scene(), &view, &settings(shape, false))
        .unwrap()
        .table_bytes;
    assert!(table > 2_000_000, "{table}");
    let with = |extra: u64| {
        let s = ScreenCoverSettings {
            room: table + extra,
            ..settings(shape, false)
        };
        cover_screen(&doc, scene(), &view, &s)
    };
    assert!(
        matches!(
            with(8192),
            Err(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
        ),
        "表のあとに 8 KiB しか残らないなら、区画を作れずに断る"
    );
    assert!(with(1 << 20).is_ok(), "表のあとに 1 MiB 残れば通る");
}

#[test]
fn rounded_corners_leave_the_corner_texels_out() {
    let (doc, _) = document();
    let view = perspective();
    let (a, b) = (DVec2::new(60.0, 50.0), DVec2::new(160.0, 150.0));
    let square = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Rectangle { a, b, corner: 0.0 }, false),
    )
    .unwrap();
    let round = cover_screen(
        &doc,
        scene(),
        &view,
        &settings(ScreenShape::Rectangle { a, b, corner: 30.0 }, false),
    )
    .unwrap();
    let corner = DVec2::new(60.0 + 30.0, 50.0 + 30.0);
    // テクセルの中の点が中心から離れる長さ（テクセル 1 つの画面の長さ k の 0.75 倍より短い）より、縁から離れたテクセルだけを見る
    let k = (screen(&view, Vec3::new(2.0 / 32.0, 0.0, 0.0)) - screen(&view, Vec3::ZERO)).length();
    let m = 0.75 * k + 0.1;
    let mut cut = 0;
    for (&(x, y), &p) in &front() {
        let s = screen(&view, p);
        if s.x < corner.x
            && s.y < corner.y
            && s.distance(corner) > 30.0 + m
            && s.x > 60.0 + m
            && s.y > 50.0 + m
        {
            assert_eq!(amount(&square, x, y), 255);
            assert_eq!(amount(&round, x, y), 0, "丸めた角の外 ({x}, {y})");
            cut += 1;
        }
    }
    assert!(cut > 0);
}

#[test]
fn the_result_does_not_depend_on_the_number_of_threads() {
    let (doc, _) = document();
    let view = perspective();
    let lasso = [
        DVec2::new(40.0, 40.0),
        DVec2::new(170.0, 60.0),
        DVec2::new(120.0, 110.0),
        DVec2::new(180.0, 165.0),
        DVec2::new(60.0, 150.0),
    ];
    let run = |threads: usize, shape: ScreenShape| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let s = ScreenCoverSettings {
                    projection: ProjectionSettings::default(),
                    ..settings(shape, true)
                };
                cover_screen(&doc, scene(), &view, &s).unwrap()
            })
    };
    for shape in [
        ScreenShape::View,
        ScreenShape::Ellipse {
            a: DVec2::new(40.0, 30.0),
            b: DVec2::new(170.0, 160.0),
        },
        ScreenShape::Polygon { points: &lasso },
    ] {
        let one = run(1, shape);
        let many = run(6, shape);
        assert!(one.mask() == many.mask(), "{shape:?}");
        assert!(!one.is_empty());
        for y in 0..SIZE {
            for x in 0..SIZE {
                if amount(&one, x, y) > 0 {
                    assert_eq!(one.screen(x, y), many.screen(x, y), "({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn the_whole_view_is_refused_over_the_budget_and_an_empty_box_covers_nothing() {
    let (doc, _) = document();
    let view = perspective();
    let s = ScreenCoverSettings {
        room: 4096,
        ..settings(ScreenShape::View, true)
    };
    assert!(matches!(
        cover_screen(&doc, scene(), &view, &s),
        Err(SurfaceStrokeError::Dab(DabRefusal::MemoryBudget))
    ));
    let flat = ScreenShape::Rectangle {
        a: DVec2::new(50.0, 50.0),
        b: DVec2::new(150.0, 50.0),
        corner: 0.0,
    };
    let c = cover_screen(&doc, scene(), &view, &settings(flat, false)).unwrap();
    assert!(c.is_empty());
    let bad = ScreenShape::Ellipse {
        a: DVec2::new(f64::NAN, 0.0),
        b: DVec2::new(10.0, 10.0),
    };
    assert!(matches!(
        cover_screen(&doc, scene(), &view, &settings(bad, false)),
        Err(SurfaceStrokeError::Dab(DabRefusal::InvalidArguments))
    ));
}

fn color_bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l).unwrap().surface(Channel::Color).map_or_else(
        || vec![0; (SIZE * SIZE * 4) as usize],
        |s| s.to_canvas_bytes(),
    )
}

fn rgba(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
}

#[test]
fn the_canvas_gradient_and_the_placed_form_paint_the_normal_blend_of_the_color_at_their_points() {
    // 期待は、塗る道を通らずに作る: 透明の画素へ、その点のグラデーションの色を不透明度で普通に重ねた値（選択範囲なし・量 1）。
    // 2D のグラデーションが C# と同じバイトであることは、reference の material_golden（csharp_all_bytes_at_both_parallel_degrees・
    // csharp_mask_all_bytes_at_both_parallel_degrees）が見る。
    let g = GradientSettings {
        shape: GradientShape::Radial,
        start: DVec2::new(20.0, 30.0),
        end: DVec2::new(50.0, 10.0),
        from: Rgba8::new(200, 40, 10, 255),
        to: Rgba8::new(0, 90, 255, 128),
        opacity: 0.8,
    };
    let expected = |place: &dyn Fn(u32, u32) -> DVec2| -> Vec<u8> {
        let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let p = place(x, y);
                let c = blend(
                    Rgba8::TRANSPARENT,
                    g.color_at(p.x, p.y),
                    g.opacity,
                    BlendMode::Normal,
                );
                out.extend_from_slice(&c.to_array());
            }
        }
        out
    };
    let paint = [ChannelPaint::new(Channel::Color, g.from)];
    let to = [ChannelPaint::new(Channel::Color, g.to)];
    // 2D: 画素の中心の点
    let (mut a, la) = document();
    a.gradient_material(la, &paint, Some(&to), &g, None, false)
        .unwrap();
    assert_eq!(
        color_bytes(&a, la),
        expected(&|x, y| DVec2::new(x as f64 + 0.5, y as f64 + 0.5))
    );
    // 点を渡す形: 渡した点の色（中心でない点で、点が効いていることを見る）
    let shift = |x: u32, y: u32| DVec2::new(y as f64 * 0.7 + 3.0, x as f64 * 1.3 - 2.0);
    let (mut b, lb) = document();
    b.gradient_material_at(lb, &paint, Some(&to), &g, None, false, &|x, y| {
        Some(shift(x, y))
    })
    .unwrap();
    assert_eq!(color_bytes(&b, lb), expected(&shift));
    // 写しの無い画素は変えない
    let (mut c, lc) = document();
    assert!(!c
        .gradient_material_at(lc, &paint, Some(&to), &g, None, false, &|_, _| None)
        .unwrap());
    assert!(!c.can_undo());
}

#[test]
fn a_screen_gradient_colors_each_visible_texel_by_where_it_lands_on_screen() {
    let view = perspective();
    let (mut doc, layer) = document();
    // 選択範囲: 文書の左半分（x < 24）だけ
    let selection = SelectionMask::rectangle(&doc, 0, 0, 24, SIZE as i64);
    doc.set_selection(Some(selection)).unwrap();
    doc.clear_history().unwrap();
    let c = cover_screen(&doc, scene(), &view, &settings(ScreenShape::View, true)).unwrap();
    let (start, end) = (DVec2::new(70.0, 100.0), DVec2::new(150.0, 100.0));
    let g = GradientSettings {
        shape: GradientShape::Linear,
        start,
        end,
        from: Rgba8::new(255, 0, 0, 255),
        to: Rgba8::new(0, 0, 255, 255),
        opacity: 1.0,
    };
    let paint = [ChannelPaint::new(Channel::Color, g.from)];
    let to = [ChannelPaint::new(Channel::Color, g.to)];
    assert!(doc
        .gradient_material_at(
            layer,
            &paint,
            Some(&to),
            &g,
            Some(c.mask()),
            false,
            &|x, y| c.screen(x, y)
        )
        .unwrap());
    let bytes = color_bytes(&doc, layer);
    for (&(x, y), &p) in &front() {
        let s = screen(&view, p);
        let got = rgba(&bytes, x, y);
        if x >= 24 {
            assert_eq!(got, [0; 4], "選択範囲の外 ({x}, {y})");
        } else {
            assert_eq!(got, g.color_at(s.x, s.y).to_array(), "({x}, {y}) {s}");
        }
    }
    // 隠れた面・裏の面の画素は変わらない
    for (x, y) in [(40, 10), (10, 40)] {
        assert_eq!(rgba(&bytes, x, y), [0; 4]);
    }
    assert_eq!(doc.undo_count(), 1);
    doc.undo().unwrap();
    assert!(color_bytes(&doc, layer).iter().all(|&v| v == 0));
}
