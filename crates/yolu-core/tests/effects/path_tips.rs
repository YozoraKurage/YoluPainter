//! パスの筆先: 画像の筆先（正方形の角まで塗る）、2D の向き（パスに沿う・決めた角度）、3D の面の上の筆先と向き、3D の投影の深さ
//! （浅いと面から離れた曲線は描かない、自動なら届く）。
use std::sync::Arc;

use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_canvas, render_surface, CanvasPath, CanvasPoint, Options, PathBrush,
    PathPoint, PathStyle, SurfacePath, Tangent,
};
use yolu_core::{BrushSettings, BrushTip, Channel, Rgba8};

const SIZE: u32 = 64;

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
        pressure_opacity: false,
        ..BrushSettings::default()
    })
}

/// 全部が覆い 255 の w × h の筆先。
fn tip(w: u32, h: u32) -> Arc<BrushTip> {
    Arc::new(BrushTip::new("試し", w, h, vec![255; (w * h) as usize]).unwrap())
}

fn canvas(points: &[(f64, f64)], radius: f64, style: PathStyle) -> CanvasPath {
    CanvasPath {
        id: 1,
        channel: Channel::Color,
        brush: brush(radius),
        points: points
            .iter()
            .map(|&(x, y)| CanvasPoint::new(x, y, 1.0).unwrap())
            .collect(),
        material: None,
        style,
    }
}

fn alpha(b: &[u8], x: u32, y: u32) -> u8 {
    b[((y * SIZE + x) * 4 + 3) as usize]
}

fn bytes2(p: &CanvasPath) -> Vec<u8> {
    render_canvas(p, &options()).unwrap().channels[0]
        .1
        .to_canvas_bytes()
}

/// 行 y の塗られた画素の数。
fn row_width(b: &[u8], y: u32) -> usize {
    (0..SIZE).filter(|&x| alpha(b, x, y) > 0).count()
}

#[test]
fn a_square_tip_paints_the_corners_a_round_tip_leaves() {
    let round = bytes2(&canvas(&[(32.0, 32.0)], 10.0, PathStyle::default()));
    let square = bytes2(&canvas(
        &[(32.0, 32.0)],
        10.0,
        PathStyle {
            tip: Some(tip(4, 4)),
            ..Default::default()
        },
    ));
    // 中心から (8, 8) の所は、正方形の中・円の外
    assert_eq!(alpha(&round, 40, 40), 0);
    assert!(alpha(&square, 40, 40) > 0);
    assert!(alpha(&square, 32, 32) > 0 && alpha(&round, 32, 32) > 0);
}

#[test]
fn a_2d_tip_turns_with_the_path_or_keeps_its_angle() {
    // 横に長い筆先（8 × 2）を縦の線に: 決めた角度 0 なら横に広い帯、沿わせれば細い帯、角度 90 も細い帯
    let line = [(32.0, 8.0), (32.0, 56.0)];
    let style = |follow: bool, angle: f64| PathStyle {
        tip: Some(tip(8, 2)),
        follow,
        angle,
        ..Default::default()
    };
    let fixed = bytes2(&canvas(&line, 10.0, style(false, 0.0)));
    let follow = bytes2(&canvas(&line, 10.0, style(true, 0.0)));
    let turned = bytes2(&canvas(&line, 10.0, style(false, 90.0)));
    let (a, b, c) = (
        row_width(&fixed, 32),
        row_width(&follow, 32),
        row_width(&turned, 32),
    );
    assert!(a >= 18, "決めた角度 0 は横に直径ほど: {a}");
    assert!(b <= 7, "沿わせると細い: {b}");
    assert!(c <= 7, "90 度回すと細い: {c}");
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

fn on(x: f64, y: f64) -> PathPoint {
    if x >= y {
        PathPoint::new(0, x - y, y, 1.0).unwrap()
    } else {
        PathPoint::new(1, x, y - x, 1.0).unwrap()
    }
}

fn surface(
    g: &SurfaceGeometry,
    points: Vec<PathPoint>,
    radius: f64,
    style: PathStyle,
) -> SurfacePath {
    SurfacePath {
        id: 2,
        channel: Channel::Color,
        brush: brush(radius),
        points,
        model_fingerprint: fingerprint(g),
        material: None,
        style,
    }
}

fn bytes3(p: &SurfacePath, g: &SurfaceGeometry) -> (Vec<u8>, usize) {
    let r = render_surface(p, g, &options()).unwrap();
    (r.channels[0].1.to_canvas_bytes(), r.gaps)
}

#[test]
fn a_3d_tip_lies_on_the_surface_and_its_angle_starts_from_model_up() {
    let g = plane();
    let (round, _) = bytes3(
        &surface(&g, vec![on(0.5, 0.5)], 0.15, PathStyle::default()),
        &g,
    );
    let (square, _) = bytes3(
        &surface(
            &g,
            vec![on(0.5, 0.5)],
            0.15,
            PathStyle {
                tip: Some(tip(4, 4)),
                ..Default::default()
            },
        ),
        &g,
    );
    // UV は位置と同じ（0〜1）。中心から (0.12, 0.12) は正方形の中・円の外
    let px = |v: f64| (v * SIZE as f64) as u32;
    assert_eq!(alpha(&round, px(0.62), px(0.62)), 0);
    assert!(alpha(&square, px(0.62), px(0.62)) > 0);
    // 横に長い筆先（8 × 2）: 角度 0 は筆先の横の軸がモデルの +Y（縦に長い）、90 度で横に長い
    let long = |angle: f64| {
        bytes3(
            &surface(
                &g,
                vec![on(0.5, 0.5)],
                0.2,
                PathStyle {
                    tip: Some(tip(8, 2)),
                    angle,
                    ..Default::default()
                },
            ),
            &g,
        )
        .0
    };
    let column = |b: &[u8], x: u32| (0..SIZE).filter(|&y| alpha(b, x, y) > 0).count();
    let (up, side) = (long(0.0), long(90.0));
    assert!(
        column(&up, px(0.5)) > 2 * row_width(&up, px(0.5)),
        "縦に長い"
    );
    assert!(
        row_width(&side, px(0.5)) > 2 * column(&side, px(0.5)),
        "横に長い"
    );
}

#[test]
fn a_shallow_depth_skips_the_curve_away_from_the_surface() {
    let g = plane();
    // 取っ手で面から最大 0.15 浮いた曲線（z の向き。自動の深さは制御多角形の長さの 1/4 ≈ 0.19）
    let lifted = vec![
        on(0.2, 0.5).with_tangent(Tangent::Handles {
            incoming: Vec3::ZERO,
            outgoing: Vec3::new(0.2, 0.0, 0.2),
        }),
        on(0.8, 0.5).with_tangent(Tangent::Handles {
            incoming: Vec3::new(-0.2, 0.0, 0.2),
            outgoing: Vec3::ZERO,
        }),
    ];
    let (_, auto) = bytes3(&surface(&g, lifted.clone(), 0.03, PathStyle::default()), &g);
    let (_, shallow) = bytes3(
        &surface(
            &g,
            lifted,
            0.03,
            PathStyle {
                depth: Some(1.0),
                ..Default::default()
            },
        ),
        &g,
    );
    assert_eq!(auto, 0, "自動の深さは区間の長さで届く");
    assert!(shallow > 0, "半径ほどの深さでは、浮いた所を描かない");
}

// ───────── 対称・向きの反転 ─────────

#[test]
fn a_2d_symmetric_path_also_draws_its_mirrored_copy() {
    use yolu_core::paths::PathSymmetry;
    use yolu_core::{CanvasSymmetry, SymmetryMode};
    let line = [(8.0, 20.0), (24.0, 44.0)];
    let plain = bytes2(&canvas(&line, 2.0, PathStyle::default()));
    let mirrored = bytes2(&canvas(
        &line,
        2.0,
        PathStyle {
            symmetry: PathSymmetry::Canvas(
                CanvasSymmetry::new(
                    SymmetryMode::Vertical,
                    yolu_core::glam::DVec2::new(32.0, 32.0),
                    2,
                )
                .unwrap(),
            ),
            ..Default::default()
        },
    ));
    // 縦の軸 x = 32 で左右に映る（画素 x は 63 − x）
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = alpha(&plain, x, y);
            if a > 0 {
                assert_eq!(alpha(&mirrored, 63 - x, y), a, "({x}, {y})");
                assert_eq!(alpha(&mirrored, x, y), a);
            }
        }
    }
    assert_eq!(alpha(&plain, 48, 32), 0);
    assert!(alpha(&mirrored, 47, 32) > 0 || alpha(&mirrored, 48, 32) > 0);
}

#[test]
fn a_2d_path_with_a_diagonal_lines_symmetry_also_draws_the_transposed_copy() {
    use yolu_core::paths::PathSymmetry;
    use yolu_core::CanvasSymmetry;
    let line = [(8.0, 20.0), (24.0, 44.0)];
    let mirrored = bytes2(&canvas(
        &line,
        2.0,
        PathStyle {
            symmetry: PathSymmetry::Canvas(
                CanvasSymmetry::lines(yolu_core::glam::DVec2::new(32.0, 32.0), 2, 45.0).unwrap(),
            ),
            ..Default::default()
        },
    ));
    // 中心 (32, 32) を通る 45 度の鏡: 画素 (x, y) は (y, x) へ写る
    let mut drawn = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let a = alpha(&mirrored, x, y);
            drawn += u32::from(a > 0);
            assert_eq!(alpha(&mirrored, y, x), a, "({x}, {y})");
        }
    }
    assert!(drawn > 40);
    assert!(
        alpha(&mirrored, 20, 8) > 0 || alpha(&mirrored, 20, 9) > 0,
        "写し側の端"
    );
}

#[test]
fn a_3d_mirrored_path_lands_on_the_reflected_surface_and_reversing_keeps_the_pixels() {
    use yolu_core::paths::PathSymmetry;
    let g = plane();
    let points = vec![on(0.1, 0.2), on(0.3, 0.8)];
    let (plain, _) = bytes3(&surface(&g, points.clone(), 0.04, PathStyle::default()), &g);
    let mirror = PathStyle {
        symmetry: PathSymmetry::Mirror {
            point: Vec3::new(0.5, 0.0, 0.0),
            normal: Vec3::X,
        },
        ..Default::default()
    };
    let (both, gaps) = bytes3(&surface(&g, points.clone(), 0.04, mirror.clone()), &g);
    assert_eq!(gaps, 0);
    let px = |v: f64| (v * SIZE as f64) as u32;
    assert_eq!(alpha(&plain, px(0.8), px(0.5)), 0);
    assert!(alpha(&both, px(0.8), px(0.5)) > 0, "x = 0.5 で映った側");
    assert!(alpha(&both, px(0.2), px(0.5)) > 0, "元の側");
    // 映す面の外（許す距離より遠い）へ映る点は描かず、点の数を欠落に数える
    let far = PathStyle {
        symmetry: PathSymmetry::Mirror {
            point: Vec3::new(5.0, 0.0, 0.0),
            normal: Vec3::X,
        },
        ..Default::default()
    };
    let (_, gaps) = bytes3(&surface(&g, points.clone(), 0.04, far), &g);
    assert_eq!(gaps, 2);
    // 向きを逆にしても、丸いブラシの画素はほぼ同じ（曲線は同じで、ダブの置き場が逆の端から並ぶ）
    let path = surface(&g, points, 0.04, PathStyle::default());
    let (reversed, _) = bytes3(&path.reversed(), &g);
    let (a, b) = (
        plain.chunks(4).filter(|p| p[3] > 0).count(),
        reversed.chunks(4).filter(|p| p[3] > 0).count(),
    );
    assert!((a as i64 - b as i64).abs() <= a as i64 / 20, "{a} {b}");
}
