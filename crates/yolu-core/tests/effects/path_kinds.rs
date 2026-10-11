//! パスの種類: 消しゴム（ブラシの `erase` と同じ画素・一覧の前のパスを消す）、指先（前のパスの色を引きずる）、塗り（閉じた曲線の
//! 内側。3D は 1 つの UV アイランドだけ、アイランドをまたげば断る）、リボン（画像を向きに回したダブ。並べる・伸ばす。画像が無ければ断る）。
//! ワーカーの数によらず同じ画素。
use std::collections::HashMap;

use yolu_core::effects::ImageInput;
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_canvas, render_list, render_surface, CanvasPath, CanvasPoint, Error,
    LayerPathEntry, Options, PathBrush, PathKind, PathPoint, PathStyle, Ribbon, RibbonMode,
    SurfacePath,
};
use yolu_core::{BrushSettings, Channel, ImageColorSpace, ImageId, LayerPath, Rgba8};

const SIZE: u32 = 64;
const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);

fn options() -> Options<'static> {
    Options {
        width: SIZE,
        height: SIZE,
        tile_size: 16,
        ..Options::default()
    }
}

fn brush(radius: f64, color: Rgba8) -> PathBrush {
    PathBrush(BrushSettings {
        radius,
        hardness: 1.0,
        spacing: 0.1,
        color,
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    })
}

fn canvas(id: u128, points: &[(f64, f64)], kind: PathKind, radius: f64) -> CanvasPath {
    CanvasPath {
        id,
        channel: Channel::Color,
        brush: brush(radius, RED),
        points: points
            .iter()
            .map(|&(x, y)| CanvasPoint::new(x, y, 1.0).unwrap())
            .collect(),
        material: None,
        style: PathStyle {
            kind,
            ..Default::default()
        },
    }
}

fn bytes(o: &Options<'_>, entries: &[LayerPathEntry], g: Option<&SurfaceGeometry>) -> Vec<u8> {
    render_list(entries, g, o).unwrap().channels[0]
        .1
        .to_canvas_bytes()
}

fn px(b: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [b[i], b[i + 1], b[i + 2], b[i + 3]]
}

fn entry(p: CanvasPath) -> LayerPathEntry {
    LayerPathEntry::new(LayerPath::Canvas(p))
}

#[test]
fn an_erase_path_is_the_erasing_brush_and_clears_the_paths_below() {
    let line = [(8.0, 32.0), (56.0, 32.0)];
    let mut erase = canvas(2, &[(32.0, 8.0), (32.0, 56.0)], PathKind::Erase, 4.0);
    let mut flagged = erase.clone();
    flagged.style = PathStyle::default();
    flagged.brush.0.erase = true;
    let below = entry(canvas(1, &line, PathKind::Stroke, 4.0));
    let a = bytes(&options(), &[below.clone(), entry(erase.clone())], None);
    let b = bytes(&options(), &[below.clone(), entry(flagged)], None);
    assert_eq!(a, b, "種類の消しゴムは、ブラシの erase と同じ画素");
    assert_eq!(px(&a, 32, 32)[3], 0, "交わる所は消える");
    assert!(px(&a, 12, 32)[3] > 0, "離れた所は残る");
    // 消しゴムだけの一覧は何も描かない
    erase.id = 3;
    assert!(bytes(&options(), &[entry(erase)], None)
        .chunks(4)
        .all(|p| p[3] == 0));
}

#[test]
fn a_smudge_path_drags_the_color_of_the_paths_below() {
    // 縦の赤い線の上を、左から右へ指先で引きずる: 線の右の透明な所に赤が付く
    let below = entry(canvas(
        1,
        &[(20.0, 8.0), (20.0, 56.0)],
        PathKind::Stroke,
        3.0,
    ));
    let smudge = entry(canvas(
        2,
        &[(14.0, 32.0), (40.0, 32.0)],
        PathKind::SMUDGE,
        5.0,
    ));
    let only = bytes(&options(), std::slice::from_ref(&below), None);
    let both = bytes(&options(), &[below, smudge.clone()], None);
    assert_eq!(px(&only, 30, 32)[3], 0);
    assert!(px(&both, 30, 32)[3] > 0, "{:?}", px(&both, 30, 32));
    assert_eq!(px(&both, 30, 32)[0], RED.r, "引きずった色は赤");
    // 下に何も無ければ、指先は何も描かない
    assert!(bytes(&options(), &[smudge], None)
        .chunks(4)
        .all(|p| p[3] == 0));
}

#[test]
fn a_fill_path_paints_the_inside_and_closes_an_open_path() {
    let square = [(10.0, 10.0), (50.0, 10.0), (50.0, 40.0), (10.0, 40.0)];
    let mut p = canvas(1, &square, PathKind::Fill, 2.0);
    for q in &mut p.points {
        q.tangent = yolu_core::paths::Tangent::Corner;
    }
    p.brush.0.opacity = 0.5;
    let b = bytes(&options(), &[entry(p.clone())], None);
    assert_eq!(
        px(&b, 30, 25),
        [RED.r, RED.g, RED.b, 128],
        "内側は不透明度まで"
    );
    assert_eq!(px(&b, 5, 25)[3], 0, "外は塗らない");
    assert_eq!(px(&b, 30, 45)[3], 0);
    // 縁の画素（10.0 の線の上）は全部、外側（9）は塗らない
    assert_eq!(px(&b, 10, 25)[3], 128);
    assert_eq!(px(&b, 9, 25)[3], 0);
    // 2 点のパスは内側が無い
    let line = canvas(2, &square[..2], PathKind::Fill, 2.0);
    assert!(bytes(&options(), &[entry(line)], None)
        .chunks(4)
        .all(|p| p[3] == 0));
}

fn image(colors: &[[u8; 4]], w: u32, h: u32) -> ImageInput {
    ImageInput::new(w, h, colors.concat(), ImageColorSpace::Srgb).unwrap()
}

fn with_image(img: ImageInput) -> Options<'static> {
    let mut images = HashMap::new();
    images.insert(ImageId(7), img);
    Options {
        images,
        ..options()
    }
}

fn ribbon(mode: RibbonMode) -> PathKind {
    PathKind::Ribbon(Ribbon {
        image: ImageId(7),
        mode,
        spacing: 1.0,
    })
}

#[test]
fn a_stretched_ribbon_lays_one_image_along_the_whole_path() {
    // 左半分が赤・右半分が青の画像を、横の線いっぱいに伸ばす
    let o = with_image(image(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1));
    let p = canvas(
        1,
        &[(4.0, 32.0), (60.0, 32.0)],
        ribbon(RibbonMode::Stretch),
        6.0,
    );
    let b = bytes(&o, &[entry(p)], None);
    let (left, right) = (px(&b, 10, 32), px(&b, 54, 32));
    assert_eq!(left, [255, 0, 0, 255], "始めの方は画像の左");
    assert_eq!(right, [0, 0, 255, 255], "終わりの方は画像の右");
    assert_eq!(px(&b, 30, 32 + 8)[3], 0, "幅（直径 12）の外は描かない");
    assert!(px(&b, 30, 32 + 5)[3] > 0);
}

#[test]
fn a_tiled_ribbon_repeats_the_image_and_turns_with_the_path() {
    // 1 × 1 の不透明な緑を、縦の線に沿って並べる: 縦の帯になる
    let o = with_image(image(&[[0, 200, 0, 255]], 1, 1));
    let p = canvas(
        1,
        &[(32.0, 6.0), (32.0, 58.0)],
        ribbon(RibbonMode::Tile),
        4.0,
    );
    let b = bytes(&o, &[entry(p.clone())], None);
    for y in [8, 20, 31, 45, 56] {
        assert_eq!(px(&b, 32, y), [0, 200, 0, 255], "y = {y}");
    }
    assert_eq!(px(&b, 32 + 6, 30)[3], 0, "幅の外");
    // 画像が無ければ断る
    let err = render_list(&[entry(p)], None, &options()).unwrap_err();
    assert_eq!(err, Error::MissingImage);
}

#[test]
fn a_ribbon_paints_its_image_colors_only_on_color_channels() {
    let o = with_image(image(&[[0, 0, 255, 255]], 1, 1));
    let mut p = canvas(
        1,
        &[(4.0, 32.0), (60.0, 32.0)],
        ribbon(RibbonMode::Tile),
        6.0,
    );
    p.material = Some(vec![
        yolu_core::paths::ChannelPaint {
            channel: Channel::Color,
            color: RED,
        },
        yolu_core::paths::ChannelPaint {
            channel: Channel::Roughness,
            color: Rgba8::new(90, 90, 90, 255),
        },
    ]);
    let r = render_canvas(&p, &o).unwrap();
    let color = r.channels.iter().find(|c| c.0 == Channel::Color).unwrap();
    let rough = r
        .channels
        .iter()
        .find(|c| c.0 == Channel::Roughness)
        .unwrap();
    assert_eq!(color.1.pixel(30, 32).unwrap(), Rgba8::new(0, 0, 255, 255));
    assert_eq!(rough.1.pixel(30, 32).unwrap(), Rgba8::new(90, 90, 90, 255));
}

/// 2 つの UV アイランドの板: 左（x 0〜1）は UV の左半分、右（x 1〜2）は UV の右半分で、間に UV の継ぎ目。
fn two_islands() -> SurfaceGeometry {
    let quad = |x0: f32, u0: f32| {
        let (a, b, c, d) = (
            Vec3::new(x0, 0.0, 0.0),
            Vec3::new(x0 + 1.0, 0.0, 0.0),
            Vec3::new(x0 + 1.0, 1.0, 0.0),
            Vec3::new(x0, 1.0, 0.0),
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
    let mut t = quad(0.0, 0.0).to_vec();
    t.extend(quad(1.0, 0.55));
    SurfaceGeometry::new(t, 1, DEFAULT_WELD_TOLERANCE).unwrap()
}

/// 板の (x, y) の点（左のアイランドは三角形 0・1、右のアイランドは 2・3）。
fn on(x: f64, y: f64) -> PathPoint {
    let (base, lx) = if x < 1.0 { (0, x) } else { (2, x - 1.0) };
    if lx >= y {
        PathPoint::new(base, lx - y, y, 1.0).unwrap()
    } else {
        PathPoint::new(base + 1, lx, y - lx, 1.0).unwrap()
    }
}

fn surface(g: &SurfaceGeometry, points: Vec<PathPoint>, kind: PathKind) -> SurfacePath {
    SurfacePath {
        id: 9,
        channel: Channel::Color,
        brush: brush(0.05, RED),
        points,
        model_fingerprint: fingerprint(g),
        material: None,
        style: PathStyle {
            kind,
            ..Default::default()
        },
    }
}

#[test]
fn a_3d_fill_paints_one_uv_island_and_refuses_to_cross_islands() {
    let g = two_islands();
    let tri = vec![on(0.2, 0.2), on(0.8, 0.2), on(0.8, 0.8), on(0.2, 0.8)];
    let p = surface(&g, tri, PathKind::Fill);
    let r = render_surface(&p, &g, &options()).unwrap();
    let b = r.channels[0].1.to_canvas_bytes();
    // 左のアイランドの UV の内側（u = 0.45 × 0.5、v = 0.5）
    let x = (0.45 * 0.5 * SIZE as f64) as u32;
    assert_eq!(px(&b, x, SIZE / 2), [RED.r, RED.g, RED.b, 255]);
    assert_eq!(px(&b, x, 4)[3], 0, "曲線の外");
    // 右のアイランドには何も描かない
    let right = (0.8 * SIZE as f64) as u32;
    assert_eq!(px(&b, right, SIZE / 2)[3], 0);
    // アイランドをまたぐ点
    let crossing = surface(
        &g,
        vec![on(0.5, 0.2), on(1.5, 0.2), on(1.5, 0.8), on(0.5, 0.8)],
        PathKind::Fill,
    );
    assert_eq!(
        render_surface(&crossing, &g, &options()).unwrap_err(),
        Error::FillIslands
    );
}

#[test]
fn a_3d_ribbon_stretches_its_image_along_the_surface() {
    let g = two_islands();
    let o = with_image(image(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1));
    let p = surface(
        &g,
        vec![on(0.1, 0.5), on(0.9, 0.5)],
        ribbon(RibbonMode::Stretch),
    );
    let r = render_surface(&p, &g, &o).unwrap();
    assert!(r.dabs > 4 && r.gaps == 0, "{} {}", r.dabs, r.gaps);
    let b = r.channels[0].1.to_canvas_bytes();
    let u = |x: f64| (x * 0.45 * SIZE as f64) as u32;
    assert_eq!(px(&b, u(0.2), SIZE / 2), [255, 0, 0, 255]);
    assert_eq!(px(&b, u(0.8), SIZE / 2), [0, 0, 255, 255]);
    assert_eq!(px(&b, u(0.5), (0.8 * SIZE as f64) as u32)[3], 0, "幅の外");
}

#[test]
fn every_kind_draws_the_same_bytes_for_any_worker_count() {
    let o = with_image(image(&[[255, 0, 0, 255], [0, 0, 255, 128]], 2, 1));
    let entries = vec![
        entry(canvas(
            1,
            &[(8.0, 8.0), (56.0, 20.0), (40.0, 56.0)],
            PathKind::Fill,
            3.0,
        )),
        entry(canvas(
            2,
            &[(4.0, 30.0), (60.0, 34.0)],
            ribbon(RibbonMode::Tile),
            5.0,
        )),
        entry(canvas(
            3,
            &[(30.0, 4.0), (34.0, 60.0)],
            PathKind::SMUDGE,
            6.0,
        )),
        entry(canvas(
            4,
            &[(10.0, 50.0), (50.0, 10.0)],
            PathKind::Erase,
            2.0,
        )),
    ];
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| bytes(&o, &entries, None))
    };
    let one = run(1);
    assert!(one.chunks(4).any(|p| p[3] > 0));
    assert_eq!(run(2), one);
    assert_eq!(run(4), one);
}

/// 描いた画素の指紋（FNV-1a 64）。SIMD の道（環境変数 `YOLU_SIMD` の scalar・sse41・avx2・neon）ごとに同じ値になることを、
/// 決めた値との照合で確かめる（道を替えて試験を回す）。
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn every_kind_draws_the_same_bytes_on_every_simd_path() {
    let o = with_image(image(&[[255, 0, 0, 255], [0, 0, 255, 128]], 2, 1));
    let mut corner = canvas(
        5,
        &[(6.0, 50.0), (30.0, 60.0), (58.0, 44.0)],
        PathKind::Stroke,
        2.5,
    );
    corner.points[1].tangent = yolu_core::paths::Tangent::Corner;
    corner.style.tip = yolu_core::builtin_tip("bristles");
    corner.style.follow = true;
    let entries = vec![
        entry(canvas(
            1,
            &[(8.0, 8.0), (56.0, 20.0), (40.0, 56.0)],
            PathKind::Fill,
            3.0,
        )),
        entry(canvas(
            2,
            &[(4.0, 30.0), (60.0, 34.0)],
            ribbon(RibbonMode::Stretch),
            5.0,
        )),
        entry(canvas(
            3,
            &[(30.0, 4.0), (34.0, 60.0)],
            PathKind::SMUDGE,
            6.0,
        )),
        entry(canvas(
            4,
            &[(10.0, 50.0), (50.0, 10.0)],
            PathKind::Erase,
            2.0,
        )),
        entry(corner),
    ];
    let b = bytes(&o, &entries, None);
    assert_eq!(
        fnv(&b),
        EXPECTED,
        "道: {:?}",
        std::env::var("YOLU_SIMD").ok()
    );
}

const EXPECTED: u64 = 1_608_905_333_514_068_755;

// ───────── 予算と取消 ─────────

fn big_options(stroke_budget_bytes: u64) -> Options<'static> {
    Options {
        width: 512,
        height: 512,
        tile_size: 16,
        stroke_budget_bytes,
        ..Options::default()
    }
}

/// 縦に長い細い歯がたくさんある櫛の形（画素は少なく、副標本の行ごとの交わりと多角形の点は多い）。
fn comb(kind: PathKind) -> CanvasPath {
    let mut points = Vec::new();
    for i in 0..100 {
        let x = 2.0 + i as f64 * 4.0;
        points.extend([(x, 2.0), (x, 500.0), (x + 1.0, 500.0), (x + 1.0, 2.0)]);
    }
    let mut p = canvas(1, &points, kind, 1.0);
    for q in &mut p.points {
        q.tangent = yolu_core::paths::Tangent::Corner;
    }
    p
}

fn painted(entries: &[LayerPathEntry], o: &Options<'_>) -> usize {
    bytes(o, entries, None)
        .chunks(4)
        .filter(|p| p[3] > 0)
        .count()
}

#[test]
fn a_fill_counts_its_work_memory_against_the_stroke_budget() {
    let fill = comb(PathKind::Fill);
    // 十分な予算なら塗れる
    assert!(painted(&[entry(fill.clone())], &big_options(64 << 20)) > 10_000);
    // 同じ形を辿るストロークは、同じ予算（4 MiB）で通る: 作業面の巻き戻しは予算に収まっている
    let stroke = comb(PathKind::Stroke);
    assert!(painted(&[entry(stroke)], &big_options(4 << 20)) > 10_000);
    // 塗りの覚え（多角形の点と、副標本の行ごとの交わり）は同じ予算を超えるので、何も返さずに断る
    assert_eq!(
        render_list(&[entry(fill)], None, &big_options(4 << 20)).unwrap_err(),
        Error::Core(yolu_core::CoreError::StrokeBudgetExceeded)
    );
}

#[test]
fn a_3d_fill_counts_its_work_memory_against_the_stroke_budget() {
    let g = two_islands();
    let tri = vec![on(0.2, 0.2), on(0.8, 0.2), on(0.8, 0.8), on(0.2, 0.8)];
    let p = surface(&g, tri, PathKind::Fill);
    assert!(render_surface(&p, &g, &options()).is_ok());
    let tight = Options {
        stroke_budget_bytes: 256,
        ..options()
    };
    assert_eq!(
        render_surface(&p, &g, &tight).unwrap_err(),
        Error::Core(yolu_core::CoreError::StrokeBudgetExceeded)
    );
}

#[test]
fn a_ribbon_that_gathers_more_than_the_stroke_budget_is_refused() {
    let line = [(4.0, 30.0), (60.0, 34.0)];
    let p = canvas(2, &line, ribbon(RibbonMode::Stretch), 8.0);
    let img = image(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1);
    assert!(render_list(&[entry(p.clone())], None, &with_image(img.clone())).is_ok());
    let tight = Options {
        stroke_budget_bytes: 256,
        ..with_image(img)
    };
    assert_eq!(
        render_list(&[entry(p)], None, &tight).unwrap_err(),
        Error::Core(yolu_core::CoreError::StrokeBudgetExceeded)
    );
}

#[test]
fn a_fill_and_a_ribbon_that_do_not_fit_the_work_surface_budget_are_refused() {
    let img = image(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1);
    let tight = |o: Options<'static>| Options {
        source_budget_bytes: 1,
        ..o
    };
    let fill = canvas(
        1,
        &[(10.0, 10.0), (50.0, 10.0), (50.0, 40.0), (10.0, 40.0)],
        PathKind::Fill,
        2.0,
    );
    let ribbon = canvas(
        2,
        &[(4.0, 30.0), (60.0, 34.0)],
        self::ribbon(RibbonMode::Tile),
        6.0,
    );
    let source = Error::Core(yolu_core::CoreError::SourceBudgetExceeded);
    assert_eq!(
        render_list(&[entry(fill)], None, &tight(options())).unwrap_err(),
        source
    );
    assert_eq!(
        render_list(&[entry(ribbon)], None, &tight(with_image(img.clone()))).unwrap_err(),
        source
    );
    let g = two_islands();
    let p = surface(
        &g,
        vec![on(0.2, 0.2), on(0.8, 0.2), on(0.8, 0.8), on(0.2, 0.8)],
        PathKind::Fill,
    );
    assert_eq!(
        render_surface(&p, &g, &tight(options())).unwrap_err(),
        source
    );
    let p = surface(
        &g,
        vec![on(0.1, 0.5), on(0.9, 0.5)],
        self::ribbon(RibbonMode::Stretch),
    );
    assert_eq!(
        render_surface(&p, &g, &tight(with_image(img))).unwrap_err(),
        source
    );
}

#[test]
fn a_canceled_list_of_every_kind_returns_nothing_and_leaves_the_layer_alone() {
    use std::sync::atomic::AtomicBool;
    let img = image(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1);
    let entries = vec![
        entry(canvas(
            1,
            &[(8.0, 8.0), (56.0, 20.0)],
            PathKind::Stroke,
            3.0,
        )),
        entry(canvas(
            2,
            &[(4.0, 30.0), (60.0, 34.0)],
            ribbon(RibbonMode::Stretch),
            5.0,
        )),
        entry(canvas(
            3,
            &[(10.0, 40.0), (50.0, 40.0), (30.0, 60.0)],
            PathKind::Fill,
            3.0,
        )),
        entry(canvas(
            4,
            &[(30.0, 4.0), (34.0, 60.0)],
            PathKind::SMUDGE,
            6.0,
        )),
        entry(canvas(
            5,
            &[(10.0, 50.0), (50.0, 10.0)],
            PathKind::Erase,
            2.0,
        )),
    ];
    let flag = AtomicBool::new(true);
    let o = Options {
        cancel: Some(&flag),
        ..with_image(img.clone())
    };
    assert_eq!(
        render_list(&entries, None, &o).unwrap_err(),
        Error::Canceled
    );
    for e in &entries {
        assert_eq!(
            render_list(std::slice::from_ref(e), None, &o).unwrap_err(),
            Error::Canceled
        );
    }
    // 文書のレイヤーは、取り消された描き直しで何も変わらない（パスも画素も Undo の段も）
    let mut doc = yolu_core::Document::with_tile_size(SIZE, SIZE, 16).unwrap();
    doc.set_effect_inputs(yolu_core::EffectInputs::new().with_image(ImageId(7), img))
        .unwrap();
    let layer = doc.add_layer("パス").unwrap();
    doc.set_canvas_paths(layer, vec![entries[0].clone()])
        .unwrap();
    let (before, steps) = (
        doc.layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        doc.undo_count(),
    );
    assert!(doc
        .set_canvas_paths_cancellable(layer, entries.clone(), Some(&flag))
        .is_err());
    assert_eq!(doc.undo_count(), steps);
    assert_eq!(doc.layer(layer).unwrap().paths().len(), 1);
    assert_eq!(
        doc.layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        before
    );
    // 取消の旗が立っていなければ、同じ一覧が描ける
    let clear = AtomicBool::new(false);
    doc.set_canvas_paths_cancellable(layer, entries, Some(&clear))
        .unwrap();
    assert_eq!(doc.layer(layer).unwrap().paths().len(), 5);
}
