//! ステンシル（Unity 版の C# の Core の StencilTests のうち、1 つの面へ描く範囲を移したもの。値は C# の試験の期待値そのもの）。
//! マスク・マテリアルで塗る部分は、まだ core に無いので移していない（選択範囲は selection.rs、透明部分のロックは docops.rs の試験）。

use std::sync::Arc;

use yolu_core::brush::{linear_to_srgb, luminance};
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushPixel, BrushSample, BrushSettings, BrushStencil, Channel, CoreError, Document,
    ImageColorSpace, LayerId, Rgba8, RowOrder, StencilImage, StencilMapping, StencilMode,
    StencilPoint, StencilTiling,
};

const W: u32 = 64;
const H: u32 = 48;
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const BLACK: Rgba8 = Rgba8::new(0, 0, 0, 255);

fn document() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 16).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.clear_history().unwrap();
    (d, l)
}
fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> Rgba8) -> Vec<u8> {
    let mut v = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            v[(y * w + x) * 4..][..4].copy_from_slice(&f(x, y).to_array());
        }
    }
    v
}
fn stencil_image(
    w: usize,
    h: usize,
    f: impl Fn(usize, usize) -> Rgba8,
    space: ImageColorSpace,
) -> Arc<StencilImage> {
    Arc::new(
        StencilImage::new(
            w,
            h,
            image(w, h, f),
            space,
            StencilImage::DEFAULT_MIP_BUDGET_BYTES,
        )
        .unwrap(),
    )
}
fn stencil(
    img: Arc<StencilImage>,
    mode: StencilMode,
    tiling: StencilTiling,
    invert: bool,
    mapping: Option<StencilMapping>,
    channels: &[Channel],
) -> Option<Arc<BrushStencil>> {
    Some(Arc::new(BrushStencil::new(
        img, mode, tiling, invert, mapping, channels,
    )))
}
fn identity() -> Option<StencilMapping> {
    Some(StencilMapping::translation(0.0, 0.0))
}
/// 画布全体を 1 つのダブで覆う硬いブラシ（覆いは全部 1、天井は不透明度）。
fn covering(opacity: f64, flow: f64, color: Rgba8) -> Brush {
    Brush {
        seed: 1,
        ..Brush::from(BrushSettings {
            radius: 200.0,
            hardness: 1.0,
            opacity,
            flow,
            pressure_opacity: false,
            pressure_flow: false,
            pressure_size: false,
            color,
            ..BrushSettings::default()
        })
    }
}
fn dab(d: &mut Document, l: LayerId, b: &Brush) {
    dab_in(d, l, Channel::Color, b);
}
fn dab_in(d: &mut Document, l: LayerId, ch: Channel, b: &Brush) {
    let mut s = d.begin_brush_stroke_in(l, ch, b).unwrap();
    s.add_sample(
        d,
        BrushSample::new(32.0, 24.0, 1.0, 0.0, DVec2::ZERO).unwrap(),
    )
    .unwrap();
    assert!(d.end_stroke(s).unwrap().changed);
}
fn pixel(d: &Document, l: LayerId, x: u32, y: u32) -> Rgba8 {
    d.layer(l).unwrap().pixel(Channel::Color, x, y).unwrap()
}
fn pixel_in(d: &Document, l: LayerId, ch: Channel, x: u32, y: u32) -> Rgba8 {
    d.layer(l).unwrap().pixel(ch, x, y).unwrap()
}
fn layer_bytes(d: &Document, l: LayerId) -> Vec<u8> {
    d.layer(l)
        .unwrap()
        .surface(Channel::Color)
        .map_or(vec![0; (W * H * 4) as usize], |s| s.to_canvas_bytes())
}
fn varied(x: usize, y: usize) -> Rgba8 {
    Rgba8::new(
        (x * 13 % 256) as u8,
        (y * 29 % 256) as u8,
        ((x * y + 7) % 256) as u8,
        255,
    )
}

// ───────── 量（マスク） ─────────

#[test]
fn a_mask_lets_the_paint_through_where_it_is_white_and_holds_it_back_where_it_is_black() {
    let img = stencil_image(
        64,
        48,
        |x, _| if x < 32 { WHITE } else { BLACK },
        ImageColorSpace::Srgb,
    );
    let (mut plain, pl) = document();
    dab(
        &mut plain,
        pl,
        &covering(0.8, 1.0, Rgba8::new(30, 200, 90, 255)),
    );
    let (mut d, l) = document();
    let mut b = covering(0.8, 1.0, Rgba8::new(30, 200, 90, 255));
    b.stencil = stencil(
        img,
        StencilMode::Mask,
        StencilTiling::None,
        false,
        identity(),
        &[],
    );
    dab(&mut d, l, &b);
    for y in 0..H {
        for x in 0..W {
            let expected = if x < 32 {
                pixel(&plain, pl, x, y)
            } else {
                Rgba8::TRANSPARENT
            };
            assert_eq!(pixel(&d, l, x, y), expected, "{x},{y}");
        }
    }
}

#[test]
fn a_grey_caps_the_stroke_at_its_value_however_many_dabs_overlap() {
    // 天井に掛けるので、流量の小さいダブを何度重ねても 128/255 を越えない。不透明度 128/255 のストロークとバイトまで同じ
    let grey = stencil_image(
        64,
        48,
        |_, _| Rgba8::new(128, 128, 128, 255),
        ImageColorSpace::Srgb,
    );
    let line: Vec<BrushSample> = (0..30)
        .map(|i| {
            BrushSample::new(
                10.0 + i as f64 * 0.5,
                20.0 + (i % 3) as f64,
                1.0,
                i as f64 * 0.01,
                DVec2::ZERO,
            )
            .unwrap()
        })
        .collect();
    let brush = |opacity: f64| {
        let mut b = covering(opacity, 0.25, Rgba8::new(30, 200, 90, 255));
        b.base.radius = 12.0;
        b.base.hardness = 0.5;
        b.base.spacing = 0.05;
        b
    };
    let draw = |b: &Brush| {
        let (mut d, l) = document();
        let mut s = d.begin_brush_stroke(l, b).unwrap();
        for p in &line {
            s.add_sample(&mut d, *p).unwrap();
        }
        d.end_stroke(s).unwrap();
        (layer_bytes(&d, l), pixel(&d, l, 14, 21))
    };
    let (plain, _) = draw(&brush(128.0 / 255.0));
    let mut b = brush(1.0);
    b.stencil = stencil(
        grey,
        StencilMode::Mask,
        StencilTiling::None,
        false,
        identity(),
        &[],
    );
    let (through, p) = draw(&b);
    assert_eq!(
        through, plain,
        "50 % の灰色のステンシルは不透明度 128/255 とバイトまで同じ"
    );
    assert_eq!(p.a, 128, "ステンシルの量まで溜まり、越えない");
}

#[test]
fn invert_and_alpha_shape_the_mask() {
    // 透明な所は塗らない。反転は (1 − 輝度) × α: 透明の上の黒い形（ロゴ）を反転で塗れる
    let logo = stencil_image(
        64,
        48,
        |x, _| {
            if x < 20 {
                BLACK
            } else if x < 40 {
                Rgba8::new(255, 255, 255, 0)
            } else {
                WHITE
            }
        },
        ImageColorSpace::Srgb,
    );
    let (mut d, l) = document();
    let mut b = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    b.stencil = stencil(
        logo.clone(),
        StencilMode::Mask,
        StencilTiling::None,
        true,
        identity(),
        &[],
    );
    dab(&mut d, l, &b);
    assert_eq!(pixel(&d, l, 5, 10).a, 255, "黒は反転で塗る");
    assert_eq!(pixel(&d, l, 30, 10), Rgba8::TRANSPARENT, "透明は塗らない");
    assert_eq!(pixel(&d, l, 50, 10), Rgba8::TRANSPARENT, "白は反転で止める");
    let (mut e, el) = document();
    b.stencil = stencil(
        logo,
        StencilMode::Mask,
        StencilTiling::None,
        false,
        identity(),
        &[],
    );
    dab(&mut e, el, &b);
    assert_eq!(pixel(&e, el, 5, 10), Rgba8::TRANSPARENT);
    assert_eq!(pixel(&e, el, 30, 10), Rgba8::TRANSPARENT);
    assert_eq!(pixel(&e, el, 50, 10).a, 255);
}

// ───────── 色 ─────────

#[test]
fn a_colour_stencil_paints_its_own_colours_texel_for_texel() {
    // 画像の画素がキャンバスの画素にちょうど重なる写し（8, 4 ずらす）: 塗った画素はその画素の色そのもの。画像の外は塗らない
    let img = stencil_image(40, 30, varied, ImageColorSpace::Srgb);
    let (mut d, l) = document();
    let mut b = covering(1.0, 1.0, Rgba8::new(1, 2, 3, 255));
    b.stencil = stencil(
        img,
        StencilMode::Color,
        StencilTiling::None,
        false,
        Some(StencilMapping::translation(-8.0, -4.0)),
        &[Channel::Color],
    );
    dab(&mut d, l, &b);
    for y in 0..H as usize {
        for x in 0..W as usize {
            let inside = (8..48).contains(&x) && (4..34).contains(&y);
            let expected = if inside {
                varied(x - 8, y - 4)
            } else {
                Rgba8::TRANSPARENT
            };
            assert_eq!(pixel(&d, l, x as u32, y as u32), expected, "{x},{y}");
        }
    }
}

#[test]
fn a_colour_stencils_alpha_is_the_amount_and_the_brush_alpha_stays() {
    let img = stencil_image(
        64,
        48,
        |x, _| {
            Rgba8::new(
                200,
                10,
                60,
                if x < 20 {
                    0
                } else if x < 40 {
                    128
                } else {
                    255
                },
            )
        },
        ImageColorSpace::Srgb,
    );
    let (mut d, l) = document();
    let mut b = covering(1.0, 1.0, Rgba8::new(0, 0, 0, 255));
    b.stencil = stencil(
        img.clone(),
        StencilMode::Color,
        StencilTiling::None,
        false,
        identity(),
        &[Channel::Color],
    );
    dab(&mut d, l, &b);
    assert_eq!(pixel(&d, l, 10, 10), Rgba8::TRANSPARENT);
    assert_eq!(pixel(&d, l, 30, 10), Rgba8::new(200, 10, 60, 128));
    assert_eq!(pixel(&d, l, 50, 10), Rgba8::new(200, 10, 60, 255));
    // 描画色のアルファ（半透明の筆）はそのまま効く
    let (mut e, el) = document();
    b.base.color = Rgba8::new(0, 0, 0, 102);
    dab(&mut e, el, &b);
    assert_eq!(pixel(&e, el, 50, 10), Rgba8::new(200, 10, 60, 102));
}

#[test]
fn colour_channels_encode_a_linear_image_and_scalar_channels_take_the_luminance() {
    // 塗りつぶしの画像と同じ読み方: 色のチャンネルではデータ（リニア）の画像を sRGB に、スカラーのチャンネルは輝度
    let srgb = |v| linear_to_srgb(v);
    let l = luminance(50, 120, 230);
    for (ch, space, expected) in [
        (
            Channel::Color,
            ImageColorSpace::Srgb,
            Rgba8::new(50, 120, 230, 255),
        ),
        (
            Channel::Color,
            ImageColorSpace::Linear,
            Rgba8::new(srgb(50), srgb(120), srgb(230), 255),
        ),
        (
            Channel::Emission,
            ImageColorSpace::Linear,
            Rgba8::new(srgb(50), srgb(120), srgb(230), 255),
        ),
        (
            Channel::Roughness,
            ImageColorSpace::Srgb,
            Rgba8::new(l, l, l, 255),
        ),
    ] {
        let (mut d, lid) = document();
        let mut b = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
        b.stencil = stencil(
            stencil_image(64, 48, |_, _| Rgba8::new(50, 120, 230, 255), space),
            StencilMode::Color,
            StencilTiling::None,
            false,
            identity(),
            &[ch],
        );
        dab_in(&mut d, lid, ch, &b);
        assert_eq!(pixel_in(&d, lid, ch, 20, 20), expected, "{ch:?} {space:?}");
    }
    assert_eq!(linear_to_srgb(128), 188);
}

// ───────── 繰り返し ─────────

#[test]
fn tiling_repeats_the_image_only_along_its_axes() {
    let img = stencil_image(8, 8, |_, _| WHITE, ImageColorSpace::Srgb);
    for tiling in [
        StencilTiling::None,
        StencilTiling::Horizontal,
        StencilTiling::Vertical,
        StencilTiling::Both,
    ] {
        let (mut d, l) = document();
        let mut b = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
        b.stencil = stencil(
            img.clone(),
            StencilMode::Mask,
            tiling,
            false,
            identity(),
            &[],
        );
        dab(&mut d, l, &b);
        let h = matches!(tiling, StencilTiling::Horizontal | StencilTiling::Both);
        let v = matches!(tiling, StencilTiling::Vertical | StencilTiling::Both);
        for y in (0..H).step_by(3) {
            for x in (0..W).step_by(3) {
                let painted = (x < 8 || h) && (y < 8 || v);
                assert_eq!(
                    pixel(&d, l, x, y).a,
                    if painted { 255 } else { 0 },
                    "{tiling:?} {x},{y}"
                );
            }
        }
    }
}

// ───────── 3D のダブ ─────────

#[test]
fn mesh_dabs_read_the_stencil_at_their_own_points_and_match_the_canvas_mapping() {
    let img = stencil_image(64, 48, varied, ImageColorSpace::Srgb);
    let mapping = StencilMapping::new(0.9, 0.1, 2.5, -0.15, 1.05, 1.25).unwrap();
    // 2D の写しを使うストローク（apply_pixel は点が無ければ写しで読む）と、同じ点を渡すストロークは同じ
    let (mut a, al) = document();
    let (mut c, cl) = document();
    let mut ba = covering(0.9, 1.0, Rgba8::new(30, 200, 90, 255));
    ba.stencil = stencil(
        img.clone(),
        StencilMode::Color,
        StencilTiling::None,
        false,
        Some(mapping),
        &[Channel::Color],
    );
    let mut bc = covering(0.9, 1.0, Rgba8::new(30, 200, 90, 255));
    bc.stencil = stencil(
        img,
        StencilMode::Color,
        StencilTiling::None,
        false,
        None,
        &[Channel::Color],
    );
    let mut sa = a.begin_brush_stroke(al, &ba).unwrap();
    let mut sc = c.begin_brush_stroke(cl, &bc).unwrap();
    for y in 4..40i64 {
        for x in 3..50i64 {
            let cov = ((x * 5 + y * 3) % 10) as f64 / 9.0;
            let pressure = 0.5 + (x % 3) as f64 * 0.2;
            sa.apply_pixel(&mut a, x, y, cov, pressure).unwrap();
            let (ix, iy) = mapping.map(x, y);
            let at = StencilPoint::new(ix, iy, mapping.footprint()).unwrap();
            sc.apply_pixel_at(&mut c, x, y, cov, pressure, at).unwrap();
        }
    }
    a.end_stroke(sa).unwrap();
    c.end_stroke(sc).unwrap();
    let (ca, cc) = (
        a.composite(a.bounds()).unwrap(),
        c.composite(c.bounds()).unwrap(),
    );
    assert_eq!(ca, cc);
    assert!(ca.iter().any(|&v| v != 0));

    // 面のダブ（apply_dab_at）も点ごとに読む: 量 0 の点は塗らない
    let (mut e, el) = document();
    let mut be = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    be.stencil = stencil(
        stencil_image(4, 4, |_, _| WHITE, ImageColorSpace::Srgb),
        StencilMode::Mask,
        StencilTiling::None,
        false,
        None,
        &[],
    );
    let mut s = e.begin_brush_stroke(el, &be).unwrap();
    let pixels = [
        BrushPixel {
            x: 5,
            y: 5,
            coverage: 1.0,
        },
        BrushPixel {
            x: 6,
            y: 5,
            coverage: 1.0,
        },
    ];
    let points = [
        StencilPoint::new(2.0, 2.0, 1.0).unwrap(),
        StencilPoint::new(10.0, 2.0, 1.0).unwrap(),
    ];
    s.apply_dab_at(&mut e, &pixels, DVec2::new(5.0, 5.0), 1.0, &points)
        .unwrap();
    e.end_stroke(s).unwrap();
    assert_eq!(pixel(&e, el, 5, 5).a, 255);
    assert_eq!(pixel(&e, el, 6, 5), Rgba8::TRANSPARENT, "ステンシルの外");

    // 繰り返しでも、FAR_AWAY より遠い点（カメラの後ろなど）は読まない
    let (mut f, fl) = document();
    let mut bf = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    bf.stencil = stencil(
        stencil_image(4, 4, |_, _| WHITE, ImageColorSpace::Srgb),
        StencilMode::Mask,
        StencilTiling::Both,
        false,
        None,
        &[],
    );
    let mut s = f.begin_brush_stroke(fl, &bf).unwrap();
    let far = StencilPoint::new(-StencilImage::FAR_AWAY, -StencilImage::FAR_AWAY, 1.0).unwrap();
    assert!(!s.apply_pixel_at(&mut f, 5, 5, 1.0, 1.0, far).unwrap());
    let near = StencilPoint::new(1e7 + 0.5, 2.5, 1.0).unwrap();
    assert!(
        s.apply_pixel_at(&mut f, 6, 5, 1.0, 1.0, near).unwrap(),
        "遠いが遠すぎない繰り返しのステンシル"
    );
    f.end_stroke(s).unwrap();
}

// ───────── Undo・取消・消しゴム・並列 ─────────

#[test]
fn undo_redo_and_cancel_restore_exactly() {
    let img = stencil_image(64, 48, varied, ImageColorSpace::Srgb);
    let (mut d, l) = document();
    let before = layer_bytes(&d, l);
    let mut b = covering(0.7, 0.6, Rgba8::new(30, 200, 90, 255));
    b.base.radius = 9.0;
    b.stencil = stencil(
        img,
        StencilMode::Color,
        StencilTiling::None,
        false,
        Some(StencilMapping::translation(1.0, 2.0)),
        &[Channel::Color],
    );
    let line = |d: &mut Document, s: &mut yolu_core::Stroke| {
        s.add_sample(
            d,
            BrushSample::new(10.0, 10.0, 1.0, 0.0, DVec2::ZERO).unwrap(),
        )
        .unwrap();
        s.add_sample(
            d,
            BrushSample::new(50.0, 30.0, 1.0, 0.1, DVec2::ZERO).unwrap(),
        )
        .unwrap();
    };
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    line(&mut d, &mut s);
    d.cancel_stroke(s);
    assert_eq!(layer_bytes(&d, l), before, "取消");
    assert_eq!(d.undo_count(), 0);
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    line(&mut d, &mut s);
    d.end_stroke(s).unwrap();
    let painted = layer_bytes(&d, l);
    assert_ne!(painted, before);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(layer_bytes(&d, l), before, "Undo");
    d.redo().unwrap();
    assert_eq!(layer_bytes(&d, l), painted, "Redo");
}

#[test]
fn erasing_through_a_colour_stencil_takes_the_amount_only() {
    let (mut e, el) = document();
    for y in 0..H {
        for x in 0..W {
            e.set_pixel(el, x, y, Rgba8::new(1, 2, 3, 255)).unwrap();
        }
    }
    e.clear_history().unwrap();
    let mut b = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    b.base.erase = true;
    b.stencil = stencil(
        stencil_image(
            64,
            48,
            |x, _| Rgba8::new(9, 9, 200, if x < 32 { 255 } else { 0 }),
            ImageColorSpace::Srgb,
        ),
        StencilMode::Color,
        StencilTiling::None,
        false,
        identity(),
        &[Channel::Color],
    );
    dab(&mut e, el, &b);
    assert_eq!(pixel(&e, el, 8, 8), Rgba8::TRANSPARENT);
    assert_eq!(pixel(&e, el, 50, 8), Rgba8::new(1, 2, 3, 255));
}

#[test]
fn large_dabs_on_worker_threads_give_the_same_bytes() {
    let img = stencil_image(97, 61, varied, ImageColorSpace::Srgb);
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let mut d = Document::with_tile_size(384, 256, 32).unwrap();
            let l = d.add_layer("paint").unwrap();
            let mut b = Brush::from(BrushSettings {
                radius: 75.0,
                hardness: 0.5,
                spacing: 0.1,
                opacity: 0.8,
                flow: 0.3,
                ..BrushSettings::default()
            });
            b.seed = 5;
            b.stencil = stencil(
                img.clone(),
                StencilMode::Color,
                StencilTiling::Both,
                false,
                Some(StencilMapping::new(0.4, 0.1, 3.0, -0.05, 0.45, 7.0).unwrap()),
                &[Channel::Color],
            );
            let mut s = d.begin_brush_stroke(l, &b).unwrap();
            for (x, y, p, t) in [
                (60.0, 60.0, 0.5, 0.0),
                (200.0, 110.0, 1.0, 0.1),
                (330.0, 190.0, 0.7, 0.2),
            ] {
                s.add_sample(&mut d, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())
                    .unwrap();
            }
            let parallel = d.active_stroke_stats().unwrap().parallel_dabs;
            d.end_stroke(s).unwrap();
            let mut out = vec![0u8; 384 * 256 * 4];
            d.composite_into(Channel::Color, d.bounds(), &mut out, RowOrder::BottomUp)
                .unwrap();
            (out, parallel)
        })
    };
    let (one, p1) = run(1);
    assert!(one.iter().any(|&v| v != 0));
    assert_eq!(p1, 0);
    let (three, p3) = run(3);
    assert!(p3 > 0, "ワーカーの経路を通る");
    assert!(three == one, "スレッドの数によらず同じバイト");
}

// ───────── 拒否 ─────────

#[test]
fn bad_input_and_budgets_are_refused_with_nothing_changed() {
    let img = stencil_image(64, 64, varied, ImageColorSpace::Srgb);
    // 2D の写しの無いステンシルで画布に描くと断り、ストロークを取り消す（何も残さない）
    let (mut d, l) = document();
    let before = layer_bytes(&d, l);
    let mut b = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    b.stencil = stencil(
        img.clone(),
        StencilMode::Mask,
        StencilTiling::None,
        false,
        None,
        &[],
    );
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    assert!(matches!(
        s.add_sample(
            &mut d,
            BrushSample::new(32.0, 24.0, 1.0, 0.0, DVec2::ZERO).unwrap()
        ),
        Err(CoreError::Unsupported(_))
    ));
    assert!(!d.has_active_stroke(), "ストロークは取り消した");
    assert_eq!(layer_bytes(&d, l), before);
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    assert!(s.apply_pixel(&mut d, 3, 3, 1.0, 1.0).is_err());
    assert!(!d.has_active_stroke());
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    let px = [BrushPixel {
        x: 1,
        y: 1,
        coverage: 1.0,
    }];
    assert!(matches!(
        s.apply_dab_at(&mut d, &px, DVec2::new(1.0, 1.0), 1.0, &[]),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(!d.has_active_stroke());
    assert_eq!(d.undo_count(), 0);
    assert_eq!(layer_bytes(&d, l), before);

    // 画素ごとに読んだステンシルの値（タイルの画素 × 12 バイト）も 1 回のストロークの予算に入る
    // （16² のタイル 12 枚: 無し 12 × (64 + 16² × 4) = 13,056 バイト、有り 12 × (64 + 16² × 16) = 49,920 バイト）
    let (mut plain, pl) = document();
    plain.set_stroke_budget_bytes(20000).unwrap();
    dab(
        &mut plain,
        pl,
        &covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255)),
    );
    let (mut tight, tl) = document();
    tight.set_stroke_budget_bytes(20000).unwrap();
    let mut bt = covering(1.0, 1.0, Rgba8::new(30, 200, 90, 255));
    bt.stencil = stencil(
        stencil_image(64, 48, |_, _| WHITE, ImageColorSpace::Srgb),
        StencilMode::Mask,
        StencilTiling::Both,
        false,
        identity(),
        &[],
    );
    let mut s = tight.begin_brush_stroke(tl, &bt).unwrap();
    assert_eq!(
        s.add_sample(
            &mut tight,
            BrushSample::new(32.0, 24.0, 1.0, 0.0, DVec2::ZERO).unwrap()
        ),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert!(!tight.has_active_stroke());
    assert_eq!(tight.undo_count(), 0);
    assert!(layer_bytes(&tight, tl).iter().all(|&v| v == 0));
}
