//! パスの試験（C# の正解との照合・指紋・断り方）。
//! 束に入れず直下の 1 本: ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を試験の間だけ 0 にして戻す。束のほかの試験のダブの経路を変え、戻すときに、同じ時に走るほかの試験が決めた値も消す。
use std::sync::atomic::AtomicBool;
use yolu_core::{
    geometry::*,
    glam::{Vec2, Vec3},
    paths::*,
    BrushSettings, Channel, CoreError, Rgba8,
};
fn options() -> Options<'static> {
    Options {
        width: 64,
        height: 64,
        tile_size: 16,
        ..Default::default()
    }
}
fn brush(i: usize, surface: bool) -> PathBrush {
    PathBrush(BrushSettings {
        radius: if surface { 0.08 } else { 3.25 },
        hardness: if i.is_multiple_of(3) { 1.0 } else { 0.7 },
        spacing: 0.17,
        opacity: 0.83,
        flow: 0.42,
        color: Rgba8::new(201, 37, 89, 219),
        pressure_size: i.is_multiple_of(2),
        pressure_opacity: i % 3 == 1,
        pressure_flow: i % 3 == 2,
        erase: i == 7,
        anti_alias: yolu_core::AntiAlias::None,
    })
}
fn canvas(i: usize) -> CanvasPath {
    let p = vec![
        CanvasPoint::new(4.5, 7.25, 0.3).unwrap(),
        CanvasPoint::new(29.5, 51.5, 0.9).unwrap(),
        CanvasPoint::new(57.25, 9.75, 0.55).unwrap(),
    ];
    let points = match i {
        0 => vec![],
        1 => p[..1].to_vec(),
        2 => p[..2].to_vec(),
        4 => vec![p[0], p[0], p[1], p[2]],
        5 => vec![
            CanvasPoint::new(-12.0, 2.0, 1.0).unwrap(),
            p[1],
            CanvasPoint::new(80.0, 64.0, 0.0).unwrap(),
        ],
        _ => p,
    };
    CanvasPath {
        style: Default::default(),
        id: 0,
        channel: Channel::Color,
        brush: brush(i, false),
        points,
        material: material(i),
    }
}
fn material(i: usize) -> Option<Vec<ChannelPaint>> {
    (i == 9).then(|| {
        vec![
            ChannelPaint {
                channel: Channel::Color,
                color: Rgba8::new(31, 87, 231, 255),
            },
            ChannelPaint {
                channel: Channel::Roughness,
                color: Rgba8::new(140, 140, 140, 255),
            },
        ]
    })
}
fn plane() -> SurfaceGeometry {
    let a = Vec3::ZERO;
    let b = Vec3::X;
    let c = Vec3::new(1.0, 1.0, 0.0);
    let d = Vec3::Y;
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
fn quad(
    (x0, y0, z, x1, y1): (f32, f32, f32, f32, f32),
    (u0, v0, u1, v1): (f32, f32, f32, f32),
    (renderer, slot): (i32, i32),
) -> [SurfaceTriangle; 2] {
    let (a, b, c, d) = (
        Vec3::new(x0, y0, z),
        Vec3::new(x1, y0, z),
        Vec3::new(x1, y1, z),
        Vec3::new(x0, y1, z),
    );
    let (ua, ub, uc, ud) = (
        Vec2::new(u0, v0),
        Vec2::new(u1, v0),
        Vec2::new(u1, v1),
        Vec2::new(u0, v1),
    );
    [
        SurfaceTriangle::new(a, b, c, ua, ub, uc).with_slot(renderer, slot, -1),
        SurfaceTriangle::new(a, c, d, ua, uc, ud).with_slot(renderer, slot, -1),
    ]
}
/// C# の PathGolden.Shape と同じ形。指紋の入力と、3D だけの入力 10..14 の面。
const SHAPE_NAMES: [&str; 4] = ["plane", "gap", "steep", "slots"];
fn shape(name: &str) -> SurfaceGeometry {
    let mut t = Vec::new();
    match name {
        "plane" => return plane(),
        "gap" => {
            t.extend(quad(
                (0.0, 0.0, 0.0, 1.0, 1.0),
                (0.0, 0.0, 1.0, 0.5),
                (0, 0),
            ));
            t.extend(quad(
                (2.0, 0.0, 0.0, 3.0, 1.0),
                (0.0, 0.5, 1.0, 1.0),
                (0, 0),
            ));
        }
        "stacked" => {
            t.extend(quad(
                (0.0, 0.0, 0.0, 1.0, 1.0),
                (0.0, 0.0, 1.0, 0.5),
                (0, 0),
            ));
            t.extend(quad(
                (1.2, 0.0, 0.15, 2.2, 1.0),
                (0.0, 0.5, 1.0, 1.0),
                (0, 0),
            ));
        }
        "steep" => {
            t.extend(quad(
                (0.0, 0.0, 0.0, 1.0, 1.0),
                (0.0, 0.0, 1.0, 1.0),
                (0, 0),
            ));
            t.push(
                SurfaceTriangle::new(
                    Vec3::new(0.25, 0.75, 0.02),
                    Vec3::new(0.25, 0.05, 0.02),
                    Vec3::new(0.45, 0.4, 1.1),
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.5, 1.0),
                )
                .with_slot(1, 2, -1),
            );
        }
        "slots" => {
            let at = |k: f32| Vec3::new(k * 2.0, 0.0, 0.0);
            let tri = |k: f32, uv: [Vec2; 3], renderer, slot| {
                SurfaceTriangle::new(at(k), at(k) + Vec3::X, at(k) + Vec3::Y, uv[0], uv[1], uv[2])
                    .with_slot(renderer, slot, -1)
            };
            let unit = [Vec2::ZERO, Vec2::X, Vec2::Y];
            t.push(tri(
                0.0,
                [
                    Vec2::new(0.1, 0.333_333_3),
                    Vec2::new(0.723_456_7, -0.25),
                    Vec2::new(1.75, 0.5),
                ],
                1,
                2,
            ));
            t.push(tri(
                1.0,
                [
                    Vec2::new(-0.0, 0.9),
                    Vec2::new(0.125, 2.0),
                    Vec2::new(1e-7, 0.999_999_94),
                ],
                3,
                0,
            ));
            t.push(tri(2.0, unit, 0, 5));
            t.push(tri(3.0, unit, -1, -1));
        }
        _ => panic!("未知の形 {name}"),
    }
    SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap()
}
/// 3D だけの入力 10..14（面の名前・ブラシの番号）。10 は離れた平面、11 は高さの違う平面、12 は急な面、13・14 は筆圧 0 の点。
const SCENES: [(&str, usize); 5] = [
    ("gap", 10),
    ("stacked", 11),
    ("steep", 12),
    ("plane", 12),
    ("plane", 13),
];
fn scene(k: usize) -> (SurfaceGeometry, SurfacePath) {
    let (name, brush_index) = SCENES[k - 10];
    let g = shape(name);
    let pt = |t, u, v, p| PathPoint::new(t, u, v, p).unwrap();
    let points = match k {
        10 | 11 => vec![
            pt(1, 0.2, 0.3, 0.3),
            pt(0, 0.5, 0.3, 0.9),
            pt(2, 0.5, 0.3, 0.55),
            pt(3, 0.5, 0.3, 1.0),
        ],
        12 => vec![
            pt(1, 0.2, 0.3, 0.6),
            pt(0, 0.5, 0.3, 0.9),
            pt(1, 0.5, 0.3, 0.8),
        ],
        _ => vec![
            pt(1, 0.2, 0.3, 0.0),
            pt(0, 0.5, 0.3, 0.9),
            pt(1, 0.5, 0.3, 0.0),
            pt(0, 0.1, 0.1, 0.6),
        ],
    };
    let mut brush = brush(brush_index, true);
    if k == 12 {
        brush.0.radius = 0.15;
    }
    let path = SurfacePath {
        style: Default::default(),
        id: 0,
        channel: Channel::Color,
        brush,
        points,
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    (g, path)
}
fn surface(i: usize, g: &SurfaceGeometry) -> SurfacePath {
    let p = vec![
        PathPoint::new(1, 0.2, 0.3, 0.3).unwrap(),
        PathPoint::new(0, 0.5, 0.3, 0.9).unwrap(),
        PathPoint::new(1, 0.5, 0.3, 0.55).unwrap(),
    ];
    let points = match i {
        0 => vec![],
        1 => p[..1].to_vec(),
        2 => p[..2].to_vec(),
        4 => vec![p[0], p[0], p[1], p[2]],
        _ => p,
    };
    SurfacePath {
        style: Default::default(),
        id: 0,
        channel: Channel::Color,
        brush: brush(i, true),
        points,
        model_fingerprint: fingerprint(g),
        material: material(i),
    }
}
fn check_golden(kind: &str, i: usize, r: Rendered, index: &str) {
    for (ch, s) in r.channels {
        let name = format!("{kind}-{i}-{}", ch.index());
        let expected = std::fs::read(format!(
            "{}/tests/golden/paths/{name}.rgba",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let actual = s.to_canvas_bytes();
        assert!(
            actual == expected,
            "{name}: {} バイト違い",
            actual.iter().zip(&expected).filter(|(a, b)| a != b).count()
        );
        if kind == "surface" {
            let line = index
                .lines()
                .find(|l| l.starts_with(&format!("{name} ")))
                .unwrap();
            let numbers: Vec<_> = line
                .split_whitespace()
                .skip(1)
                .map(|n| n.parse::<usize>().unwrap())
                .collect();
            assert_eq!((r.dabs, r.gaps), (numbers[0], numbers[1]), "{name}");
        }
    }
}
#[test]
fn csharp_fingerprints_cover_renderer_slot_and_fractional_uv() {
    let lines: Vec<_> = include_str!("golden/paths/fingerprint.txt")
        .lines()
        .map(|l| l.split_once(' ').unwrap())
        .collect();
    assert_eq!(
        lines.iter().map(|l| l.0).collect::<Vec<_>>(),
        SHAPE_NAMES,
        "C# の形の並びと違う"
    );
    for (name, expected) in lines {
        assert_eq!(fingerprint(&shape(name)), expected, "{name}");
    }
}
#[test]
fn csharp_all_bytes_and_parallelism() {
    let start = std::time::Instant::now();
    let g = plane();
    let o = options();
    let index = include_str!("golden/paths/index.txt");
    struct Restore(i64);
    impl Drop for Restore {
        fn drop(&mut self) {
            yolu_core::brush::set_parallel_dab_pixels(self.0);
        }
    }
    let _restore = Restore(yolu_core::brush::set_parallel_dab_pixels(0));
    for workers in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            for i in 0..10 {
                check_golden("canvas", i, render_canvas(&canvas(i), &o).unwrap(), index);
                check_golden(
                    "surface",
                    i,
                    render_surface(&surface(i, &g), &g, &o).unwrap(),
                    index,
                );
            }
            for k in 10..15 {
                let (g, p) = scene(k);
                check_golden("surface", k, render_surface(&p, &g, &o).unwrap(), index);
            }
        });
    }
    eprintln!("パス C# 照合 27 面 × 3 ワーカー数: {:?}", start.elapsed());
}
#[test]
fn known_canvas_curve_and_empty() {
    let o = options();
    let r = render_canvas(&canvas(0), &o).unwrap();
    assert_eq!(r.channels[0].1.allocated_bytes(), 0);
    let mut p = canvas(3);
    p.brush = PathBrush(BrushSettings {
        radius: 3.0,
        hardness: 1.0,
        pressure_size: false,
        pressure_opacity: false,
        ..Default::default()
    });
    let r = render_canvas(&p, &o).unwrap();
    let s = &r.channels[0].1;
    for q in p.points {
        assert_eq!(s.pixel(q.x as u32, q.y as u32).unwrap().a, 255);
    }
    assert_eq!(s.pixel(30, 8).unwrap().a, 0);
}
#[test]
fn malformed_points_brushes_and_materials() {
    for x in [f64::NAN, f64::INFINITY, 1e6 + 1.0] {
        assert!(CanvasPoint::new(x, 0.0, 1.0).is_err());
    }
    assert!(CanvasPoint::new(0.0, 0.0, f64::NAN).is_err());
    for (u, v) in [(-0.01, 0.0), (0.8, 0.8), (f64::NAN, 0.0)] {
        assert!(PathPoint::new(0, u, v, 1.0).is_err());
    }
    assert_eq!(PathPoint::new(0, -1e-10, 0.5, 1.0).unwrap().u, 0.0);
    let mut p = canvas(1);
    p.points = vec![p.points[0]; MAX_POINTS + 1];
    assert!(p.validate().is_err());
    p = canvas(1);
    p.brush.0.radius = 4097.0;
    assert!(p.validate().is_err());
    p = canvas(1);
    p.brush.0.spacing = f64::NAN;
    assert!(p.validate().is_err());
    p = canvas(1);
    p.material = Some(vec![]);
    assert!(p.validate().is_err());
    p.material = Some(vec![
        ChannelPaint {
            channel: p.channel,
            color: p.brush.0.color
        };
        2
    ]);
    assert!(p.validate().is_err());
    p = canvas(1);
    p.channel = Channel::from_index(6).unwrap();
    assert!(p.validate().is_err());
}
#[test]
fn source_stroke_sample_and_dab_budgets() {
    let g = plane();
    for workers in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut o = options();
                o.source_budget_bytes = 0;
                assert_eq!(
                    render_canvas(&canvas(3), &o).unwrap_err(),
                    Error::Core(CoreError::SourceBudgetExceeded)
                );
                o = options();
                o.stroke_budget_bytes = 0;
                assert_eq!(
                    render_canvas(&canvas(3), &o).unwrap_err(),
                    Error::Core(CoreError::StrokeBudgetExceeded)
                );
                let mut p = canvas(2);
                p.points[1].x = 1e6;
                p.brush.0.radius = 0.001;
                assert_eq!(
                    render_canvas(&p, &options()).unwrap_err(),
                    Error::TooManySamples
                );
                let mut s = surface(2, &g);
                s.brush.0.radius = 1e-8;
                assert_eq!(
                    render_surface(&s, &g, &options()).unwrap_err(),
                    Error::TooManySamples
                );
                o = options();
                o.surface_budget.max_candidate_pixels = 0;
                assert!(matches!(
                    render_surface(&surface(3, &g), &g, &o),
                    Err(Error::Dab(_))
                ));
            });
    }
}
#[test]
fn cancellation_returns_no_partial_result() {
    let flag = AtomicBool::new(true);
    let o = Options {
        cancel: Some(&flag),
        ..options()
    };
    let g = plane();
    assert_eq!(render_canvas(&canvas(3), &o).unwrap_err(), Error::Canceled);
    assert_eq!(
        render_surface(&surface(3, &g), &g, &o).unwrap_err(),
        Error::Canceled
    );
}
#[test]
fn fingerprint_ignores_pose_but_rejects_uv_slot_and_order() {
    let g = plane();
    let old = fingerprint(&g);
    let mut triangles = g.triangles().to_vec();
    for t in &mut triangles {
        t.a.z += 1.0;
        t.b.z += 1.0;
        t.c.z += 1.0;
    }
    let posed = SurfaceGeometry::new(triangles, 9, DEFAULT_WELD_TOLERANCE).unwrap();
    assert_eq!(fingerprint(&posed), old);
    let a = render_surface(&surface(3, &g), &g, &options()).unwrap();
    let b = render_surface(&surface(3, &g), &posed, &options()).unwrap();
    assert!(a.channels[0].1.allocated_bytes() > 0);
    assert!(b.channels[0].1.allocated_bytes() > 0);
    for mode in 0..3 {
        let mut ts = g.triangles().to_vec();
        match mode {
            0 => ts[0].uv_a.x = 0.1,
            1 => ts[0].material_slot = 1,
            _ => ts.swap(0, 1),
        }
        let other = SurfaceGeometry::new(ts, 1, DEFAULT_WELD_TOLERANCE).unwrap();
        assert_eq!(
            render_surface(&surface(3, &g), &other, &options()).unwrap_err(),
            Error::ModelMismatch
        );
    }
    let mut p = surface(1, &g);
    p.points[0].triangle = 2;
    assert_eq!(
        render_surface(&p, &g, &options()).unwrap_err(),
        Error::MissingTriangle
    );
}
#[test]
fn combined_material_budget_is_not_per_channel() {
    let mut p = canvas(9);
    p.points.truncate(1);
    let ample = render_canvas(&p, &options()).unwrap();
    let bytes: u64 = ample
        .channels
        .iter()
        .map(|(_, s)| s.allocated_bytes())
        .sum();
    let o = Options {
        source_budget_bytes: bytes - 1,
        ..options()
    };
    assert_eq!(
        render_canvas(&p, &o).unwrap_err(),
        Error::Core(CoreError::SourceBudgetExceeded)
    );
}

#[test]
fn csharp_refusal_reasons() {
    let kind = |r: Result<(), Error>| match r {
        Ok(()) => "accepted",
        Err(Error::Core(CoreError::SourceBudgetExceeded)) => "source",
        Err(Error::Core(CoreError::StrokeBudgetExceeded)) => "stroke",
        Err(Error::TooManySamples) => "samples",
        Err(Error::ModelMismatch) => "fingerprint",
        Err(Error::Invalid(_)) => "invalid",
        Err(_) => "unexpected",
    };
    let wide = |n| "\u{1F600}".repeat(n);
    let mut seen = 0;
    for line in include_str!("golden/paths/refusals.txt").lines() {
        let (name, expected) = line.split_once(' ').unwrap();
        let mut o = options();
        let mut p = canvas(3);
        let g = plane();
        let mut sp = surface(3, &g);
        let outcome = match name {
            "source" => {
                o.source_budget_bytes = 0;
                render_canvas(&p, &o).map(|_| ())
            }
            "stroke" => {
                o.stroke_budget_bytes = 0;
                render_canvas(&p, &o).map(|_| ())
            }
            "samples" => {
                p = canvas(2);
                p.brush.0.radius = 0.001;
                p.points[1].x = 1e6;
                render_canvas(&p, &o).map(|_| ())
            }
            "fingerprint" => {
                sp.model_fingerprint = "other".into();
                render_surface(&sp, &g, &o).map(|_| ())
            }
            "canvas-point" => CanvasPoint::new(f64::NAN, 0.0, 1.0).map(|_| ()),
            "surface-point" => PathPoint::new(0, 0.8, 0.8, 1.0).map(|_| ()),
            "brush" => {
                p.brush.0.radius = 4097.0;
                p.validate()
            }
            "material" => {
                p.material = Some(vec![]);
                p.validate()
            }
            "surface-fingerprint-empty" => {
                sp.model_fingerprint.clear();
                sp.validate()
            }
            "surface-fingerprint-128" => {
                sp.model_fingerprint = "a".repeat(128);
                sp.validate()
            }
            "surface-fingerprint-129" => {
                sp.model_fingerprint = "a".repeat(129);
                sp.validate()
            }
            "surface-fingerprint-wide-64" => {
                sp.model_fingerprint = wide(64);
                sp.validate()
            }
            "surface-fingerprint-wide-65" => {
                sp.model_fingerprint = wide(65);
                sp.validate()
            }
            "surface-points-4096" => {
                sp.points = vec![sp.points[0]; 4096];
                sp.validate()
            }
            "surface-points-4097" => {
                sp.points = vec![sp.points[0]; 4097];
                sp.validate()
            }
            "surface-radius-1e6" => {
                sp.brush.0.radius = 1e6;
                sp.validate()
            }
            "surface-radius-over" => {
                sp.brush.0.radius = 1_000_001.0;
                sp.validate()
            }
            "surface-channel" => {
                sp.channel = Channel::from_index(6).unwrap();
                sp.validate()
            }
            "surface-material-duplicate" => {
                sp.material = Some(vec![
                    ChannelPaint {
                        channel: Channel::Color,
                        color: Rgba8::new(1, 2, 3, 255),
                    },
                    ChannelPaint {
                        channel: Channel::Color,
                        color: Rgba8::new(4, 5, 6, 255),
                    },
                ]);
                sp.validate()
            }
            _ => panic!("未知の事例 {name}"),
        };
        assert_eq!(kind(outcome), expected, "{name}");
        seen += 1;
    }
    assert_eq!(seen, 19, "C# の事例の数が変わった");
}
#[test]
fn projection_gaps_and_corner_are_deterministic() {
    // 離れた平面の間は穴。法線が変わる角も、ワーカーの数に依らず同じ結果。
    for corner in [false, true] {
        let base = plane();
        let mut ts = base.triangles().to_vec();
        let mut other = ts.clone();
        for t in &mut other {
            for v in [&mut t.a, &mut t.b, &mut t.c] {
                *v = if corner {
                    Vec3::new(1.0, v.y, v.x)
                } else {
                    *v + Vec3::X * 2.0
                };
            }
            t.uv_a.y = t.uv_a.y * 0.5 + 0.5;
            t.uv_b.y = t.uv_b.y * 0.5 + 0.5;
            t.uv_c.y = t.uv_c.y * 0.5 + 0.5;
        }
        for t in &mut ts {
            t.uv_a.y *= 0.5;
            t.uv_b.y *= 0.5;
            t.uv_c.y *= 0.5;
        }
        ts.extend(other);
        let g = SurfaceGeometry::new(ts, 1, DEFAULT_WELD_TOLERANCE).unwrap();
        let mut p = surface(2, &g);
        p.points[1].triangle = 2;
        p.brush.0.hardness = 1.0;
        let a = render_surface(&p, &g, &options()).unwrap();
        let b = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| render_surface(&p, &g, &options()).unwrap());
        assert_eq!((a.dabs, a.gaps), (b.dabs, b.gaps));
        assert!(a.channels[0].1.to_canvas_bytes() == b.channels[0].1.to_canvas_bytes());
        if !corner {
            assert!(a.gaps > 0);
        }
        assert!(a.dabs > 0);
    }
}

fn big_options() -> Options<'static> {
    Options {
        width: 256,
        height: 256,
        tile_size: 64,
        ..Default::default()
    }
}
fn big_surface_path(g: &SurfaceGeometry) -> SurfacePath {
    let mut p = surface(2, g);
    p.points = vec![
        PathPoint::new(1, 0.2, 0.3, 0.9).unwrap(),
        PathPoint::new(0, 0.5, 0.3, 0.9).unwrap(),
        PathPoint::new(1, 0.5, 0.3, 0.9).unwrap(),
    ];
    p.brush = PathBrush(BrushSettings {
        radius: 0.2,
        hardness: 0.7,
        spacing: 0.17,
        color: Rgba8::new(201, 37, 89, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..Default::default()
    });
    p
}
fn painted(r: &Rendered) -> usize {
    r.channels[0]
        .1
        .to_canvas_bytes()
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|&&a| a > 0)
        .count()
}
#[test]
fn surface_dabs_are_identical_for_one_two_and_four_workers_when_rays_are_sharded() {
    // 遮蔽のレイは 1 ダブの候補が 1024 本以上（MIN_RAYS_PER_TASK = 512 の 2 倍）になって初めて複数のワーカーへ分かれる。
    // 64² のキャンバスの小さな筆は 1 本の逐次の経路しか通らないので、256² のキャンバスと大きな筆で、並ぶ経路を通す。
    // 板の上の一部を覆う板を置き、レイの結果が画素の有無に効く（並びの取り違えが見える）面にする。
    let free = plane();
    let mut ts = free.triangles().to_vec();
    ts.extend(quad(
        (0.45, 0.0, 0.04, 1.5, 1.0),
        (0.0, 0.0, 1.0, 1.0),
        (0, 1),
    ));
    let covered = SurfaceGeometry::new(ts, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    let hit = covered
        .raycast(Ray::new(Vec3::new(0.3, 0.5, 1.0), -Vec3::Z), true, 2.0)
        .unwrap();
    let radius = 0.2;
    let dab = covered.build_surface_dabs(
        &hit,
        radius,
        256,
        256,
        hit.position + hit.normal * radius * 4.0,
        0.7,
        &Default::default(),
        None,
        false,
    );
    assert!(dab.refusal.is_none());
    eprintln!(
        "3D 並列の前提: 1 ダブの遮蔽のレイ {} 本",
        dab.visibility_rays
    );
    assert!(
        dab.visibility_rays >= 1024,
        "並ぶ経路に入る大きさではない: {} 本",
        dab.visibility_rays
    );
    let run = |workers, g: &SurfaceGeometry| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| render_surface(&big_surface_path(g), g, &big_options()).unwrap())
    };
    let base = run(1, &covered);
    assert!(base.dabs > 10 && painted(&base) > 1000);
    for workers in [2, 4] {
        let other = run(workers, &covered);
        assert_eq!(
            (other.dabs, other.gaps),
            (base.dabs, base.gaps),
            "{workers}"
        );
        assert!(
            other.channels[0].1.to_canvas_bytes() == base.channels[0].1.to_canvas_bytes(),
            "{workers} ワーカーで画素が違う"
        );
    }
    assert!(
        painted(&base) < painted(&run(1, &free)),
        "覆いの板が画素に効いていない"
    );
}

fn invalid(r: Result<(), Error>, what: &str) -> String {
    match r {
        Err(Error::Invalid(s)) => s.to_string(),
        other => panic!("{what}: Invalid ではない {other:?}"),
    }
}
#[test]
fn surface_path_rejects_malformed_values_one_by_one() {
    let g = plane();
    let base = surface(3, &g);
    assert_eq!(base.validate(), Ok(()));
    let mut texts = Vec::new();
    let mut reject = |what: &str, edit: &dyn Fn(&mut SurfacePath)| {
        let mut p = base.clone();
        edit(&mut p);
        // 検証だけでなく、評価の入口でも ModelMismatch より先に Invalid で断る
        texts.push(invalid(p.validate(), what));
        assert!(
            matches!(render_surface(&p, &g, &options()), Err(Error::Invalid(_))),
            "{what}: render_surface"
        );
    };
    reject("指紋が空", &|p| p.model_fingerprint.clear());
    reject("指紋が 129 文字", &|p| {
        p.model_fingerprint = "a".repeat(129)
    });
    reject("指紋が UTF-16 で 130", &|p| {
        p.model_fingerprint = "\u{1F600}".repeat(65)
    });
    reject("点が 4097", &|p| {
        p.points = vec![p.points[0]; MAX_POINTS + 1]
    });
    reject("半径が上限超え", &|p| p.brush.0.radius = 1e6 + 1.0);
    reject("半径が 0", &|p| p.brush.0.radius = 0.0);
    reject("半径が負", &|p| p.brush.0.radius = -1.0);
    reject("半径が NaN", &|p| p.brush.0.radius = f64::NAN);
    reject("未正規化の負の重心座標", &|p| {
        p.points[0].u = -1e-10
    });
    reject("負の重心座標", &|p| p.points[0].v = -0.01);
    reject("重心座標の和が 1 超え", &|p| {
        p.points[0].u = 0.8;
        p.points[0].v = 0.8
    });
    reject("重心座標が NaN", &|p| p.points[1].u = f64::NAN);
    reject("筆圧が範囲外", &|p| p.points[2].pressure = 1.5);
    reject("三角形が i32 の外", &|p| {
        p.points[0].triangle = i32::MAX as u32 + 1
    });
    reject("基準が標準外のチャンネル", &|p| {
        p.channel = Channel::from_index(6).unwrap()
    });
    reject("組が空", &|p| p.material = Some(vec![]));
    reject("組に標準外のチャンネル", &|p| {
        p.material = Some(vec![ChannelPaint {
            channel: Channel::from_index(6).unwrap(),
            color: Rgba8::new(1, 2, 3, 255),
        }])
    });
    reject("組に重複", &|p| {
        p.material = Some(vec![
            ChannelPaint {
                channel: Channel::Roughness,
                color: Rgba8::new(1, 2, 3, 255),
            };
            2
        ])
    });
    reject("組が 7 チャンネル", &|p| {
        p.material = Some(
            (0..7)
                .map(|i| ChannelPaint {
                    channel: Channel::from_index(i).unwrap(),
                    color: Rgba8::new(1, 2, 3, 255),
                })
                .collect(),
        )
    });
    // 境界の内側は通る（i32::MAX の三角形番号の存在は評価で MissingTriangle にする）
    for edit in [
        &(|p: &mut SurfacePath| p.model_fingerprint = "a".repeat(128)) as &dyn Fn(&mut SurfacePath),
        &|p| p.model_fingerprint = "\u{1F600}".repeat(64),
        &|p| p.points = vec![p.points[0]; MAX_POINTS],
        &|p| p.brush.0.radius = 1e6,
        &|p| p.points[0].triangle = i32::MAX as u32,
    ] {
        let mut p = base.clone();
        edit(&mut p);
        assert_eq!(p.validate(), Ok(()));
    }
    for t in &texts {
        assert!(!t.contains("ください") && !t.contains("::"), "{t}");
    }
    // ブラシの共通の欄（間隔など）は既存のブラシの検証が断る
    let mut p = base.clone();
    p.brush.0.spacing = f64::NAN;
    assert!(matches!(
        p.validate(),
        Err(Error::Core(CoreError::InvalidArgument(_)))
    ));
}
#[test]
fn error_texts_state_what_and_why_only() {
    let g = plane();
    let mut other = g.triangles().to_vec();
    other.swap(0, 1);
    let other = SurfaceGeometry::new(other, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    let mut errors = vec![
        render_surface(&surface(3, &g), &other, &options()).unwrap_err(),
        Error::TooManySamples,
        Error::Canceled,
    ];
    let mut missing = surface(1, &g);
    missing.points[0].triangle = 9;
    errors.push(render_surface(&missing, &g, &options()).unwrap_err());
    assert_eq!(errors[0], Error::ModelMismatch);
    assert_eq!(errors[3], Error::MissingTriangle);
    for e in errors {
        let text = e.to_string();
        assert!(
            !text.is_empty() && !text.contains("ください") && !text.contains("::"),
            "{text}"
        );
    }
}
