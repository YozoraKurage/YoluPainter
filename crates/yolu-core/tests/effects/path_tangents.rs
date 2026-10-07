//! パスの点の接線: 滑らかな点だけのパスは今までと同じ画素（`tests/paths.rs` の C# の正解も同じ）、角の点で折れる（角だけのパスは
//! 折れ線）、取っ手の点は 3 次のベジェを通る。2D と 3D（休みの形のモデルの空間の取っ手）。並列の数と SIMD の道によらず同じ画素。
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_canvas, render_surface, CanvasPath, CanvasPoint, Error, Options, PathBrush,
    PathPoint, SurfacePath, Tangent,
};
use yolu_core::{BrushSettings, Channel, Rgba8};

const SIZE: u32 = 96;

fn options() -> Options<'static> {
    Options {
        width: SIZE,
        height: SIZE,
        tile_size: 16,
        ..Options::default()
    }
}

fn brush(radius: f64) -> PathBrush {
    PathBrush(BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        color: Rgba8::new(255, 255, 255, 255),
        pressure_size: false,
        ..BrushSettings::default()
    })
}

fn canvas(points: Vec<CanvasPoint>) -> CanvasPath {
    CanvasPath {
        style: Default::default(),
        id: 1,
        channel: Channel::Color,
        brush: brush(1.5),
        points,
        material: None,
    }
}

fn pt(x: f64, y: f64, tangent: Tangent<DVec2>) -> CanvasPoint {
    CanvasPoint::new(x, y, 1.0).unwrap().with_tangent(tangent)
}

/// 塗られた画素の中心（アルファが 0 でない画素）。
fn painted(path: &CanvasPath) -> Vec<DVec2> {
    let r = render_canvas(path, &options()).unwrap();
    let bytes = r.channels[0].1.to_canvas_bytes();
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if bytes[((y * SIZE + x) * 4 + 3) as usize] > 0 {
                out.push(DVec2::new(x as f64 + 0.5, y as f64 + 0.5));
            }
        }
    }
    out
}

fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn distance_to_polyline(p: DVec2, line: &[DVec2]) -> f64 {
    line.windows(2)
        .map(|w| distance_to_segment(p, w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn corners_make_a_polyline_and_smooth_points_round_it() {
    let v = [
        DVec2::new(15.0, 20.0),
        DVec2::new(48.0, 75.0),
        DVec2::new(80.0, 20.0),
        DVec2::new(85.0, 70.0),
    ];
    let make = |t: Tangent<DVec2>| canvas(v.iter().map(|p| pt(p.x, p.y, t)).collect());
    // 角だけのパスは折れ線（半径 1.5 の線の縁まで）
    let corner = painted(&make(Tangent::Corner));
    let far = corner
        .iter()
        .map(|p| distance_to_polyline(*p, &v))
        .fold(0.0, f64::max);
    assert!(far < 1.5 + 1.0, "折れ線から {far} 画素");
    // 折れ線の線分の真ん中は塗られる
    for w in v.windows(2) {
        let mid = (w[0] + w[1]) * 0.5;
        assert!(corner.iter().any(|p| p.distance(mid) < 1.0), "{mid}");
    }
    // 滑らかなパスは同じ点を丸く通るので、折れ線から外へ出る
    let smooth = painted(&make(Tangent::Smooth));
    let far = smooth
        .iter()
        .map(|p| distance_to_polyline(*p, &v))
        .fold(0.0, f64::max);
    assert!(far > 4.0, "滑らかな曲線は折れ線から {far} 画素");
}

#[test]
fn a_smooth_path_draws_the_same_bytes_with_and_without_the_tangent_field() {
    // CanvasPoint::new は滑らか。角の点を 1 つ戻すと、元と同じ画素
    let v = [(10.0, 10.0), (40.0, 60.0), (70.0, 15.0)];
    let smooth = canvas(v.iter().map(|&(x, y)| pt(x, y, Tangent::Smooth)).collect());
    let mut cornered = smooth.clone();
    cornered.points[1].tangent = Tangent::Corner;
    let a = render_canvas(&smooth, &options()).unwrap();
    let b = render_canvas(&cornered, &options()).unwrap();
    assert_ne!(
        a.channels[0].1.to_canvas_bytes(),
        b.channels[0].1.to_canvas_bytes()
    );
    cornered.points[1].tangent = Tangent::Smooth;
    let c = render_canvas(&cornered, &options()).unwrap();
    assert_eq!(
        a.channels[0].1.to_canvas_bytes(),
        c.channels[0].1.to_canvas_bytes()
    );
    assert_eq!(a.samples, c.samples);
}

#[test]
fn handles_follow_the_cubic_bezier() {
    // (10,48) から (86,48) へ、取っ手で上に膨らむ。ベジェの真ん中は 48 + 0.75 × 36 = 75
    let path = canvas(vec![
        pt(
            10.0,
            48.0,
            Tangent::Handles {
                incoming: DVec2::ZERO,
                outgoing: DVec2::new(0.0, 36.0),
            },
        ),
        pt(
            86.0,
            48.0,
            Tangent::Handles {
                incoming: DVec2::new(0.0, 36.0),
                outgoing: DVec2::ZERO,
            },
        ),
    ]);
    let curve: Vec<DVec2> = (0..=200)
        .map(|k| {
            let t = k as f64 / 200.0;
            let s = 1.0 - t;
            let c = [
                DVec2::new(10.0, 48.0),
                DVec2::new(10.0, 84.0),
                DVec2::new(86.0, 84.0),
                DVec2::new(86.0, 48.0),
            ];
            c[0] * (s * s * s)
                + c[1] * (3.0 * s * s * t)
                + c[2] * (3.0 * s * t * t)
                + c[3] * (t * t * t)
        })
        .collect();
    let px = painted(&path);
    let far = px
        .iter()
        .map(|p| distance_to_polyline(*p, &curve))
        .fold(0.0, f64::max);
    assert!(far < 1.5 + 1.0, "ベジェから {far} 画素");
    assert!(px.iter().any(|p| p.distance(DVec2::new(48.0, 75.0)) < 1.0));
}

#[test]
fn handles_are_checked() {
    let mut p = canvas(vec![pt(10.0, 10.0, Tangent::Smooth)]);
    p.points[0].tangent = Tangent::Handles {
        incoming: DVec2::new(f64::NAN, 0.0),
        outgoing: DVec2::ZERO,
    };
    assert!(matches!(p.validate(), Err(Error::Invalid(_))));
    p.points[0].tangent = Tangent::Handles {
        incoming: DVec2::ZERO,
        outgoing: DVec2::new(2e6, 0.0),
    };
    assert!(matches!(p.validate(), Err(Error::Invalid(_))));
}

fn plane() -> SurfaceGeometry {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap()
}

/// 平面の (x, y)（0〜1）にある点（三角形 0 は x ≥ y、1 は x < y）。
fn on_plane(x: f64, y: f64, tangent: Tangent<Vec3>) -> PathPoint {
    let p = if x >= y {
        // a(0,0) b(1,0) c(1,1): 位置 = u·b + v·c → x = u + v, y = v
        PathPoint::new(0, x - y, y, 1.0).unwrap()
    } else {
        // a(0,0) c(1,1) d(0,1): x = u, y = u + v
        PathPoint::new(1, x, y - x, 1.0).unwrap()
    };
    p.with_tangent(tangent)
}

fn surface(points: Vec<PathPoint>, g: &SurfaceGeometry) -> SurfacePath {
    SurfacePath {
        style: Default::default(),
        id: 2,
        channel: Channel::Color,
        brush: brush(0.02),
        points,
        model_fingerprint: fingerprint(g),
        material: None,
    }
}

fn painted_uv(path: &SurfacePath, g: &SurfaceGeometry) -> Vec<DVec2> {
    let r = render_surface(path, g, &options()).unwrap();
    let bytes = r.channels[0].1.to_canvas_bytes();
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if bytes[((y * SIZE + x) * 4 + 3) as usize] > 0 {
                out.push(DVec2::new(
                    (x as f64 + 0.5) / SIZE as f64,
                    (y as f64 + 0.5) / SIZE as f64,
                ));
            }
        }
    }
    out
}

#[test]
fn surface_corners_and_handles_bend_the_path_on_the_model() {
    let g = plane();
    let v = [(0.15, 0.2), (0.5, 0.8), (0.85, 0.2)];
    let line: Vec<DVec2> = v.iter().map(|&(x, y)| DVec2::new(x, y)).collect();
    let corner = surface(
        v.iter()
            .map(|&(x, y)| on_plane(x, y, Tangent::Corner))
            .collect(),
        &g,
    );
    let px = painted_uv(&corner, &g);
    let far = px
        .iter()
        .map(|p| distance_to_polyline(*p, &line))
        .fold(0.0, f64::max);
    // 半径 0.02 と画素の半分の対角
    assert!(far < 0.02 + 1.0 / SIZE as f64, "折れ線から {far}");
    let smooth = surface(
        v.iter()
            .map(|&(x, y)| on_plane(x, y, Tangent::Smooth))
            .collect(),
        &g,
    );
    let far = painted_uv(&smooth, &g)
        .iter()
        .map(|p| distance_to_polyline(*p, &line))
        .fold(0.0, f64::max);
    assert!(far > 0.04, "滑らかな曲線は折れ線から {far}");
    // 取っ手（モデルの空間の向き）: 2 点の間を上へ膨らむ
    let handles = surface(
        vec![
            on_plane(
                0.1,
                0.3,
                Tangent::Handles {
                    incoming: Vec3::ZERO,
                    outgoing: Vec3::new(0.0, 0.4, 0.0),
                },
            ),
            on_plane(
                0.9,
                0.3,
                Tangent::Handles {
                    incoming: Vec3::new(0.0, 0.4, 0.0),
                    outgoing: Vec3::ZERO,
                },
            ),
        ],
        &g,
    );
    let px = painted_uv(&handles, &g);
    // ベジェの真ん中は y = 0.3 + 0.75 × 0.4 = 0.6
    assert!(
        px.iter()
            .any(|p| p.distance(DVec2::new(0.5, 0.6)) < 1.5 / SIZE as f64),
        "膨らんだ曲線の真ん中"
    );
    assert!(
        !px.iter().any(|p| p.distance(DVec2::new(0.5, 0.3)) < 0.05),
        "まっすぐには通らない"
    );
}

#[test]
fn tangent_paths_draw_the_same_bytes_for_any_worker_count() {
    let path = canvas(vec![
        pt(10.0, 10.0, Tangent::Corner),
        pt(
            50.0,
            80.0,
            Tangent::Handles {
                incoming: DVec2::new(-20.0, 5.0),
                outgoing: DVec2::new(30.0, -2.0),
            },
        ),
        pt(85.0, 15.0, Tangent::Smooth),
        pt(40.0, 30.0, Tangent::Corner),
    ]);
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                render_canvas(&path, &options()).unwrap().channels[0]
                    .1
                    .to_canvas_bytes()
            })
    };
    let one = run(1);
    assert_eq!(run(2), one);
    assert_eq!(run(4), one);
}
