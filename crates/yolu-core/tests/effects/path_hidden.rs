//! 3D のパスを、描かない面（`Options::hidden`）を除いて描く: 上に重なる面（服）を除くと、パスの線は下の面（肌）に入り、除いた面の UV には
//! 何も描かない。除く面が無い（None・全部 false）ときは、除く面を持たない描き方と同じバイト。
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_list, render_surface, Error, LayerPathEntry, Options, PathBrush, PathKind,
    PathPoint, PathStyle, SurfacePath,
};
use yolu_core::{BrushSettings, Channel, LayerPath, Rgba8};

const SIZE: u32 = 64;
const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);

fn options(hidden: Option<&[bool]>) -> Options<'_> {
    Options {
        width: SIZE,
        height: SIZE,
        tile_size: 16,
        hidden,
        ..Options::default()
    }
}

/// 2 枚の板: 下（z = 0、三角形 0・1）は UV の左（u 0〜0.45）、上（z = 0.02、三角形 2・3）は UV の右（u 0.55〜1）。上の板は x へ
/// `shift` ずらす（0 なら真上に重なる）。
fn skin_and_cloth_at(shift: f32) -> SurfaceGeometry {
    let quad = |x: f32, z: f32, u0: f32| {
        let (a, b, c, d) = (
            Vec3::new(x, 0.0, z),
            Vec3::new(x + 1.0, 0.0, z),
            Vec3::new(x + 1.0, 1.0, z),
            Vec3::new(x, 1.0, z),
        );
        let (ua, ub, uc, ud) = (
            Vec2::new(u0, 0.0),
            Vec2::new(u0 + 0.45, 0.0),
            Vec2::new(u0 + 0.45, 1.0),
            Vec2::new(u0, 1.0),
        );
        [
            SurfaceTriangle::new(a, b, c, ua, ub, uc),
            SurfaceTriangle::new(a, c, d, ua, uc, ud),
        ]
    };
    let mut t = quad(0.0, 0.0, 0.0).to_vec();
    t.extend(quad(shift, 0.02, 0.55));
    SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap()
}

fn skin_and_cloth() -> SurfaceGeometry {
    skin_and_cloth_at(0.0)
}

/// 下の板の (x, y) の点。
fn on_skin(x: f64, y: f64) -> PathPoint {
    if x >= y {
        PathPoint::new(0, x - y, y, 1.0).unwrap()
    } else {
        PathPoint::new(1, x, y - x, 1.0).unwrap()
    }
}

fn path(g: &SurfaceGeometry, points: Vec<PathPoint>, kind: PathKind) -> SurfacePath {
    SurfacePath {
        id: 7,
        channel: Channel::Color,
        brush: PathBrush(
            BrushSettings {
                radius: 0.05,
                hardness: 1.0,
                spacing: 0.1,
                color: RED,
                ..BrushSettings::default()
            },
            yolu_core::PressureSwitches::NONE,
        ),
        points,
        model_fingerprint: fingerprint(g),
        material: None,
        style: PathStyle {
            kind,
            ..Default::default()
        },
    }
}

fn bytes(p: &SurfacePath, g: &SurfaceGeometry, hidden: Option<&[bool]>) -> Vec<u8> {
    render_surface(p, g, &options(hidden)).unwrap().channels[0]
        .1
        .to_canvas_bytes()
}

/// UV の範囲 [u0, u1) に描いた画素の数。
fn painted(b: &[u8], u0: f64, u1: f64) -> usize {
    let (x0, x1) = ((u0 * SIZE as f64) as usize, (u1 * SIZE as f64) as usize);
    (0..SIZE as usize)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| b[(y * SIZE as usize + x) * 4 + 3] > 0)
        .count()
}

#[test]
fn hiding_the_upper_faces_draws_the_line_on_the_faces_below() {
    let g = skin_and_cloth();
    let line = path(
        &g,
        vec![on_skin(0.1, 0.5), on_skin(0.9, 0.5)],
        PathKind::Stroke,
    );
    // 何も除かないと、上の板（服）の UV に描く
    let plain = bytes(&line, &g, None);
    assert!(painted(&plain, 0.55, 1.0) > 20, "上の板に当たる");
    assert_eq!(painted(&plain, 0.0, 0.5), 0);
    // 上の板を除くと、下の板（肌）の UV にだけ描く
    let cut = bytes(&line, &g, Some(&[false, false, true, true]));
    assert!(painted(&cut, 0.0, 0.5) > 20, "下の板に当たる");
    assert_eq!(painted(&cut, 0.5, 1.0), 0, "除いた面の UV には描かない");
    // 全部の面を除くと、何も描かない（点は全部欠落）
    let r = render_surface(&line, &g, &options(Some(&[true; 4]))).unwrap();
    assert!(r.channels[0]
        .1
        .to_canvas_bytes()
        .chunks(4)
        .all(|p| p[3] == 0));
    assert!(r.gaps > 0 && r.dabs == 0, "{} {}", r.gaps, r.dabs);
}

#[test]
fn nothing_hidden_draws_the_same_bytes_as_before() {
    for (kind, g) in [
        (PathKind::Stroke, skin_and_cloth()),
        (PathKind::Fill, skin_and_cloth_at(2.0)),
    ] {
        let p = path(
            &g,
            vec![
                on_skin(0.2, 0.2),
                on_skin(0.8, 0.2),
                on_skin(0.8, 0.8),
                on_skin(0.2, 0.8),
            ],
            kind,
        );
        let before = bytes(&p, &g, None);
        assert!(before.chunks(4).any(|p| p[3] > 0), "{kind:?}");
        assert_eq!(bytes(&p, &g, Some(&[false; 4])), before, "{kind:?}");
        // 一覧で描いても同じ
        let entries = [LayerPathEntry::new(LayerPath::Surface(p.clone()))];
        let listed = render_list(&entries, Some(&g), &options(Some(&[false; 4])))
            .unwrap()
            .channels[0]
            .1
            .to_canvas_bytes();
        assert_eq!(listed, before, "{kind:?}");
    }
}

#[test]
fn a_fill_skips_the_hidden_faces_of_its_island_and_a_wrong_count_is_refused() {
    let g = skin_and_cloth_at(2.0);
    let fill = path(
        &g,
        vec![
            on_skin(0.2, 0.2),
            on_skin(0.8, 0.2),
            on_skin(0.8, 0.8),
            on_skin(0.2, 0.8),
        ],
        PathKind::Fill,
    );
    // 下の板の三角形 1 を除く: 塗りは三角形 0 の UV にだけ入る（三角形 1 の UV の内側は塗らない）
    let b = bytes(&fill, &g, Some(&[false, true, false, false]));
    let at = |u: f64, v: f64| {
        let (x, y) = ((u * SIZE as f64) as usize, (v * SIZE as f64) as usize);
        b[(y * SIZE as usize + x) * 4 + 3]
    };
    // 三角形 0 は x ≥ y の側（UV の右下）、三角形 1 は x < y の側
    assert!(at(0.45 * 0.7, 0.3) > 0, "残した面は塗る");
    assert_eq!(at(0.45 * 0.3, 0.7), 0, "除いた面は塗らない");
    // 数が形の三角形の数と違う印は断る
    assert_eq!(
        render_surface(&fill, &g, &options(Some(&[true, false]))).unwrap_err(),
        Error::ModelMismatch
    );
}
