//! 書き出しのテンプレート（`yolu_core::export`）とパディング（`yolu_core::padding`）の既知の答え・断り方・並列の度合いに依らないこと。
//! C# の Core との全バイトの照合は export_golden.rs。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::sync::atomic::AtomicBool;

use yolu_core::export::{
    build, channel_image, channel_working_bytes, default_value, occlusion_byte, should_write,
    working_bytes, ExportError, ExportImage, ExportImageKind, ExportScalar, ExportTemplate,
};
use yolu_core::glam::DVec2;
use yolu_core::padding::{
    self, coverage, coverage_tuned, dilate, dilate_cancellable, dilate_tuned, Reach, Rings, Tuning,
    MAX_RING_REACH,
};
use yolu_core::{Channel, Document, HeightEdgeMode, NormalSettings, NormalYDirection, Rect, Rgba8};

const W: u32 = 8;
const H: u32 = 4;

type Px = [u8; 4];
/// レイヤーに塗る画素（x, y から）。
type Painter<'a> = &'a dyn Fn(u32, u32) -> Px;

/// 8 × 4 の文書。レイヤーごとに 1 つのチャンネルを塗る（Color 以外なら、そのレイヤーの Color は使わない）。
fn document(layers: &[(Channel, Painter<'_>)]) -> Document {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    for (channel, at) in layers {
        let id = doc.add_layer(&format!("{channel:?}")).unwrap();
        for y in 0..H {
            for x in 0..W {
                let p = at(x, y);
                doc.set_channel_pixel(id, *channel, x, y, Rgba8::new(p[0], p[1], p[2], p[3]))
                    .unwrap();
            }
        }
        if *channel != Channel::Color {
            doc.set_channel_enabled(id, Channel::Color, false).unwrap();
        }
    }
    doc.clear_history().unwrap();
    doc
}

fn px(image: &[u8], x: u32, y: u32) -> Px {
    let o = ((y * W + x) * 4) as usize;
    [image[o], image[o + 1], image[o + 2], image[o + 3]]
}

fn image<'a>(template: &'a ExportTemplate, suffix: &str) -> &'a ExportImage {
    template.image(suffix).unwrap()
}

const ALL: u64 = u64::MAX;

#[test]
fn metallic_and_smoothness_are_packed_from_value_times_alpha() {
    // Metallic: 左半分だけ 200（不透明）。Roughness: 全面 100、アルファ 128 → r × a = 50 → Smoothness 205
    let doc = document(&[
        (Channel::Metallic, &|x, _| {
            if x < 4 {
                [200, 200, 200, 255]
            } else {
                [0, 0, 0, 0]
            }
        }),
        (Channel::Roughness, &|_, _| [100, 100, 100, 128]),
    ]);
    let standard = ExportTemplate::unity_standard();
    let packed = build(&doc, image(&standard, "MetallicSmoothness"), None, ALL).unwrap();
    assert_eq!(px(&packed, 1, 1), [200, 200, 200, 205]);
    assert_eq!(
        px(&packed, 6, 1),
        [0, 0, 0, 205],
        "塗っていない Metallic は 0"
    );
    // HDRP の MaskMap: R Metallic・G AO（無ければ 1）・B 1・A Smoothness
    let hdrp = ExportTemplate::unity_hdrp();
    let mask = build(&doc, image(&hdrp, "MaskMap"), None, ALL).unwrap();
    assert_eq!(px(&mask, 1, 1), [200, 255, 255, 205]);
    let occlusion: Vec<u8> = (0..W * H).map(|i| (i * 7) as u8).collect();
    let mask = build(&doc, image(&hdrp, "MaskMap"), Some(&occlusion), ALL).unwrap();
    assert_eq!(
        px(&mask, 3, 2)[1],
        occlusion[(2 * W + 3) as usize],
        "焼いた AO は G へ"
    );
    assert_eq!(
        px(&mask, 3, 2)[0],
        px(&packed, 3, 2)[0],
        "AO があっても Metallic は同じ"
    );
}

#[test]
fn unused_channels_take_the_defaults_and_unread_images_are_not_written() {
    let doc = document(&[(Channel::Metallic, &|_, _| [90, 90, 90, 255])]);
    let standard = ExportTemplate::unity_standard();
    let packed = build(&doc, image(&standard, "MetallicSmoothness"), None, ALL).unwrap();
    assert_eq!(
        px(&packed, 0, 0),
        [90, 90, 90, 128],
        "Roughness を塗っていない: Unity の既定の Smoothness 0.5"
    );
    assert!(
        !should_write(&doc, image(&standard, "Height"), false),
        "Height のレイヤーが無い"
    );
    assert!(
        !should_write(&doc, image(&standard, "Albedo"), false),
        "Color のレイヤーが無い"
    );
    let ao = image(&standard, "Occlusion");
    assert!(!should_write(&doc, ao, false), "AO を焼いていない");
    assert!(should_write(&doc, ao, true));
    assert!(should_write(
        &doc,
        image(&standard, "MetallicSmoothness"),
        false
    ));
    let lil = ExportTemplate::lil_toon();
    assert!(
        !should_write(&doc, image(&lil, "Smoothness"), false),
        "lilToon の平滑度は Roughness だけを読む"
    );
    // 読むものが無い画像も、作ろうと思えば既定の値で作れる
    let height = build(&doc, image(&standard, "Height"), None, ALL).unwrap();
    assert_eq!(px(&height, 0, 0), [0, 0, 0, 255]);
    let occlusion = build(&doc, ao, None, ALL).unwrap();
    assert_eq!(px(&occlusion, 5, 3), [255, 255, 255, 255]);
    assert_eq!(default_value(ExportScalar::Smoothness), 128);
    assert_eq!(default_value(ExportScalar::Roughness), 0);
}

#[test]
fn emission_is_premultiplied_and_opaque_and_colour_is_the_composite() {
    let doc = document(&[
        (Channel::Color, &|x, _| [10, 20, 30, (x * 30) as u8]),
        (Channel::Emission, &|_, _| [255, 100, 0, 128]),
    ]);
    let standard = ExportTemplate::unity_standard();
    let emission = build(&doc, image(&standard, "Emission"), None, ALL).unwrap();
    assert_eq!(px(&emission, 2, 2), [128, 50, 0, 255]);
    let hdrp = ExportTemplate::unity_hdrp();
    assert_eq!(
        build(&doc, image(&hdrp, "BaseColor"), None, ALL).unwrap(),
        doc.composite(doc.bounds()).unwrap()
    );
    let lil = ExportTemplate::lil_toon();
    assert_eq!(
        build(&doc, image(&lil, "Normal"), None, ALL).unwrap(),
        doc.normal_output(ALL).unwrap()
    );
    assert!(image(&standard, "Emission").srgb());
    assert!(!image(&hdrp, "MaskMap").srgb());
    assert!(image(&standard, "Normal").normal_map() && !image(&standard, "Normal").srgb());
    assert!(image(&standard, "Albedo").alpha_is_transparency());
    assert!(!image(&standard, "Emission").alpha_is_transparency());
}

#[test]
fn a_height_only_document_still_has_a_derived_normal() {
    let mut doc = document(&[(Channel::Height, &|x, _| [(x * 30) as u8, 0, 0, 255])]);
    let standard = ExportTemplate::unity_standard();
    let normal = image(&standard, "Normal");
    doc.set_normal_settings(
        NormalSettings::new(false, 4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap(),
        false,
    )
    .unwrap();
    assert!(
        !should_write(&doc, normal, false),
        "Normal のレイヤーも、Height → Normal も無い"
    );
    doc.set_normal_settings(
        NormalSettings::new(true, 4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap(),
        false,
    )
    .unwrap();
    assert!(
        should_write(&doc, normal, false),
        "Height から作った法線を書く"
    );
    let built = build(&doc, normal, None, ALL).unwrap();
    assert_eq!(built, doc.normal_output(ALL).unwrap());
    assert_ne!(
        px(&built, 3, 2),
        [128, 128, 255, 255],
        "傾きがあるので平らではない"
    );
}

#[test]
fn templates_match_unity_ids_suffixes_and_are_distinct() {
    let built_in = ExportTemplate::built_in();
    let listing: Vec<(&str, &str, Vec<&str>)> = built_in
        .iter()
        .map(|t| {
            (
                t.id.as_str(),
                t.name.as_str(),
                t.images.iter().map(|i| i.suffix()).collect(),
            )
        })
        .collect();
    assert_eq!(
        listing,
        [
            (
                "unity-standard",
                "Unity Standard / URP Lit",
                vec![
                    "Albedo",
                    "MetallicSmoothness",
                    "Normal",
                    "Height",
                    "Occlusion",
                    "Emission"
                ]
            ),
            (
                "unity-hdrp",
                "HDRP Lit",
                vec!["BaseColor", "MaskMap", "Normal", "Height", "Emission"]
            ),
            (
                "liltoon",
                "lilToon",
                vec!["Main", "Normal", "Smoothness", "Metallic", "Emission"]
            ),
        ]
    );
    for t in &built_in {
        let mut suffixes: Vec<&str> = t.images.iter().map(|i| i.suffix()).collect();
        suffixes.sort_unstable();
        suffixes.dedup();
        assert_eq!(suffixes.len(), t.images.len(), "{}", t.name);
        assert_eq!(ExportTemplate::built_in_by_id(&t.id).as_ref(), Some(t));
    }
    assert!(ExportTemplate::built_in_by_id("nope").is_none());
    // 詰め方（C# の ExportTemplate の定義と同じ）
    let hdrp = ExportTemplate::unity_hdrp();
    assert_eq!(
        image(&hdrp, "MaskMap").scalars(),
        [
            ExportScalar::Metallic,
            ExportScalar::Occlusion,
            ExportScalar::One,
            ExportScalar::Smoothness
        ]
    );
    assert_eq!(image(&hdrp, "Normal").kind(), ExportImageKind::Normal);
    assert!(image(&hdrp, "Normal").scalars().is_empty());
}

#[test]
fn bad_inputs_and_the_working_budget_are_refused_before_allocating() {
    let doc = document(&[
        (Channel::Metallic, &|_, _| [90, 90, 90, 255]),
        (Channel::Color, &|_, _| [1, 2, 3, 255]),
    ]);
    let hdrp = ExportTemplate::unity_hdrp();
    let mask = image(&hdrp, "MaskMap");
    assert_eq!(
        build(&doc, mask, Some(&[0u8; 3]), ALL),
        Err(ExportError::InvalidArgument(
            "AO は 1 テクセルに 1 値（幅 × 高さ）"
        ))
    );
    let n = (W * H) as u64;
    assert_eq!(working_bytes(&doc, image(&hdrp, "BaseColor")), 4 * n);
    assert_eq!(working_bytes(&doc, image(&hdrp, "Emission")), 4 * n);
    assert_eq!(working_bytes(&doc, mask), 8 * n, "チャンネルの合成と出力");
    assert_eq!(
        working_bytes(&doc, image(&hdrp, "Normal")),
        doc.normal_working_bytes()
    );
    let ao_only = ExportTemplate::unity_standard();
    assert_eq!(
        working_bytes(&doc, image(&ao_only, "Occlusion")),
        4 * n,
        "AO だけの画像は合成を持たない"
    );
    for (suffix, image) in hdrp.images.iter().map(|i| (i.suffix(), i)) {
        let needed = working_bytes(&doc, image);
        assert_eq!(
            build(&doc, image, None, needed - 1),
            Err(ExportError::WorkingBudgetExceeded {
                needed,
                allowed: needed - 1
            }),
            "{suffix}"
        );
        assert!(
            build(&doc, image, None, needed).is_ok(),
            "{suffix}: ちょうどなら通る"
        );
    }
}

#[test]
fn occlusion_bytes_round_to_even_and_clamp() {
    assert_eq!(occlusion_byte(0.0), 0);
    assert_eq!(occlusion_byte(1.0), 255);
    assert_eq!(occlusion_byte(0.5), 128, "127.5 は偶数へ");
    assert_eq!(occlusion_byte(1.5 / 255.0), 2, "1.5 は偶数へ");
    assert_eq!(occlusion_byte(2.5 / 255.0), 2, "2.5 は偶数へ");
    assert_eq!(occlusion_byte(-0.3), 0);
    assert_eq!(occlusion_byte(7.0), 255);
    assert_eq!(occlusion_byte(f32::NAN), 255, "NaN は遮蔽なし");
    assert_eq!(occlusion_byte(f32::INFINITY), 255);
    assert_eq!(occlusion_byte(f32::NEG_INFINITY), 0);
}

// ───────── パディング ─────────

fn tri(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> [DVec2; 3] {
    [
        DVec2::new(a.0, a.1),
        DVec2::new(b.0, b.1),
        DVec2::new(c.0, c.1),
    ]
}

fn covered(w: u32, h: u32, triangles: &[[DVec2; 3]]) -> Vec<usize> {
    let c = coverage(w, h, triangles.iter().copied()).unwrap();
    (0..c.len()).filter(|&i| c[i]).collect()
}

#[test]
fn coverage_marks_every_texel_a_triangle_touches() {
    // テクセル (2, 2) の中だけの小さな三角形 → そのテクセルだけ
    assert_eq!(
        covered(8, 8, &[tri((2.2, 2.2), (2.8, 2.3), (2.5, 2.9))]),
        [2 * 8 + 2]
    );
    // 4 つのテクセルの境目の点を覆う三角形 → 4 つとも（角に触れる分も保守的に入る）
    assert_eq!(
        covered(8, 8, &[tri((3.9, 3.9), (4.1, 3.9), (4.0, 4.1))]),
        [3 * 8 + 3, 3 * 8 + 4, 4 * 8 + 3, 4 * 8 + 4]
    );
    // 斜めの細い三角形: 外接矩形の隅のテクセルは入らない（分離軸で外れる）
    let c = coverage(8, 8, [tri((0.5, 0.5), (7.5, 7.5), (7.5, 7.0))]).unwrap();
    assert!(c[0] && c[7 * 8 + 7]);
    assert!(!c[7 * 8] && !c[7], "外接矩形の遠い隅");
    // 線分につぶれた三角形も、通るテクセルを覆う。画像の外は切る。NaN は飛ばす
    assert_eq!(
        covered(
            8,
            8,
            &[
                tri((0.5, 4.5), (6.5, 4.5), (6.5, 4.5)),
                tri((-5.0, -5.0), (-1.0, -5.0), (-3.0, -1.0)),
                tri((f64::NAN, 0.0), (1.0, 1.0), (0.0, 1.0)),
            ]
        ),
        (0..7).map(|x| 4 * 8 + x).collect::<Vec<_>>()
    );
}

#[test]
fn a_triangle_beyond_the_int_range_is_clipped_to_the_image() {
    // C# は (int) の桁あふれでこの三角形を偶然飛ばす。Rust は画像で切って、重なる所を覆う
    let c = covered(8, 8, &[tri((-3.0e9, 6.0), (-3.0e9, 8.0), (4.0, 7.0))]);
    assert!(c.contains(&(6 * 8)) && c.contains(&(7 * 8 + 3)), "{c:?}");
    assert_eq!(
        covered(
            4,
            4,
            &[tri(
                (-1.0e300, -1.0e300),
                (1.0e300, -1.0e300),
                (0.0, 1.0e300)
            )]
        )
        .len(),
        16
    );
}

fn image_of(w: usize, h: usize, at: impl Fn(usize, usize) -> Px) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            out[(y * w + x) * 4..][..4].copy_from_slice(&at(x, y));
        }
    }
    out
}
fn at(image: &[u8], w: usize, x: usize, y: usize) -> Px {
    let o = (y * w + x) * 4;
    [image[o], image[o + 1], image[o + 2], image[o + 3]]
}

#[test]
fn dilation_keeps_the_covered_texels_and_grows_one_ring_per_step() {
    const N: usize = 9;
    let mut keep = vec![false; N * N];
    keep[4 * N + 4] = true;
    let source = image_of(N, N, |x, y| {
        if x == 4 && y == 4 {
            [200, 100, 50, 255]
        } else {
            [1, 2, 3, 0]
        }
    });
    let copy = source.clone();
    let two = dilate(&source, 9, 9, &keep, Reach::Texels(2), ALL).unwrap();
    assert_eq!(source, copy, "入力は変えない");
    for y in 0..N {
        for x in 0..N {
            let ring = (x as i32 - 4).abs().max((y as i32 - 4).abs());
            let expected = if ring <= 2 {
                [200, 100, 50, 255]
            } else {
                [1, 2, 3, 0]
            };
            assert_eq!(at(&two, N, x, y), expected, "texel {x},{y} (ring {ring})");
        }
    }
    let all = dilate(&source, 9, 9, &keep, Reach::Fill, ALL).unwrap();
    assert!(
        all.chunks_exact(4).all(|p| p[0] == 200 && p[3] == 255),
        "Fill は全部に届く"
    );
    assert_eq!(
        dilate(&source, 9, 9, &keep, Reach::Texels(0), ALL).unwrap(),
        source
    );
    assert_eq!(
        dilate(&source, 9, 9, &[false; N * N], Reach::Fill, ALL).unwrap(),
        source,
        "覆いが無ければ広げる元が無い"
    );
}

#[test]
fn dilation_weighs_colours_by_alpha() {
    // 不透明な赤と、透明な黒の隣では、赤のまま半分のアルファ（透明な色が黒くにじまない）。全部が透明なら色の平均
    let keep = [true, false, true];
    let image = image_of(3, 1, |x, _| match x {
        0 => [255, 0, 0, 255],
        2 => [0, 0, 0, 0],
        _ => [9, 9, 9, 9],
    });
    assert_eq!(
        at(
            &dilate(&image, 3, 1, &keep, Reach::Texels(1), ALL).unwrap(),
            3,
            1,
            0
        ),
        [255, 0, 0, 128]
    );
    let clear = image_of(3, 1, |x, _| {
        if x == 0 {
            [10, 20, 30, 0]
        } else {
            [30, 40, 50, 0]
        }
    });
    assert_eq!(
        at(
            &dilate(&clear, 3, 1, &keep, Reach::Texels(1), ALL).unwrap(),
            3,
            1,
            0
        ),
        [20, 30, 40, 0]
    );
}

#[test]
fn dilation_does_not_depend_on_the_scan_order() {
    // 同じ段の中では前の段の結果だけを読むので、左右対称の入力からは左右対称の結果になる
    const W2: usize = 16;
    const H2: usize = 5;
    let mut keep = vec![false; W2 * H2];
    keep[2 * W2] = true;
    keep[2 * W2 + 15] = true;
    let image = image_of(W2, H2, |x, _| match x {
        0 => [255, 0, 0, 255],
        15 => [0, 0, 255, 255],
        _ => [0, 0, 0, 0],
    });
    let out = dilate(&image, 16, 5, &keep, Reach::Fill, ALL).unwrap();
    for y in 0..H2 {
        for x in 0..W2 {
            let [r, g, b, a] = at(&out, W2, x, y);
            let [r2, g2, b2, a2] = at(&out, W2, W2 - 1 - x, y);
            assert_eq!([r, g, b, a], [b2, g2, r2, a2], "{x},{y}");
        }
    }
}

#[test]
fn padding_bad_inputs_and_budget_are_refused() {
    assert_eq!(
        dilate(&[0; 12], 2, 2, &[false; 4], Reach::Texels(1), ALL),
        Err(ExportError::InvalidArgument(
            "画像と覆いは幅 × 高さ（画像は × 4 バイト）"
        ))
    );
    assert!(dilate(&[0; 16], 2, 2, &[false; 3], Reach::Texels(1), ALL).is_err());
    assert!(dilate(&[], 0, 2, &[], Reach::Texels(1), ALL).is_err());
    assert!(Reach::from_setting(-2).is_err());
    assert_eq!(Reach::from_setting(-1), Ok(Reach::Fill));
    assert_eq!(Reach::from_setting(16), Ok(Reach::Texels(16)));
    assert_eq!(Reach::Fill.to_setting(), -1);
    assert_eq!(Reach::Texels(64).to_setting(), 64);
    assert_eq!(padding::working_bytes(4, 4), 12 * 16);
    let err = dilate(&[0; 64], 4, 4, &[false; 16], Reach::Texels(1), 191).unwrap_err();
    assert_eq!(
        err,
        ExportError::WorkingBudgetExceeded {
            needed: 192,
            allowed: 191
        }
    );
    assert!(dilate(&[0; 64], 4, 4, &[false; 16], Reach::Texels(1), 192).is_ok());
    assert!(coverage(0, 4, std::iter::empty()).is_err());
    assert!(coverage(4, 0, std::iter::empty()).is_err());
}

#[test]
fn a_raised_flag_cancels_the_dilation() {
    let image = vec![0u8; 8 * 8 * 4];
    let mut keep = vec![false; 64];
    keep[27] = true;
    let flag = AtomicBool::new(true);
    assert_eq!(
        dilate_cancellable(&image, 8, 8, &keep, Reach::Fill, ALL, Some(&flag)),
        Err(ExportError::Cancelled)
    );
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(dilate_cancellable(&image, 8, 8, &keep, Reach::Fill, ALL, Some(&flag)).is_ok());
}

// ───────── 並列の度合いに依らない ─────────

/// C# の `TexturePadding.Dilate` を、そのまま 1 本のループで写した参照（並列にも最適化にもしない）。
fn reference_dilate(rgba: &[u8], w: usize, h: usize, keep: &[bool], texels: i32) -> Vec<u8> {
    let n = w * h;
    let mut output = rgba.to_vec();
    if texels == 0 {
        return output;
    }
    let neighbors = |i: usize| -> Vec<usize> {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let mut v = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (x + dx, y + dy);
                if (dx == 0 && dy == 0) || nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                v.push((ny as usize) * w + nx as usize);
            }
        }
        v
    };
    let mut step = vec![0i32; n];
    for i in 0..n {
        if keep[i] {
            step[i] = 1;
        }
    }
    let mut frontier: Vec<usize> = (0..n)
        .filter(|&i| keep[i] && neighbors(i).iter().any(|&j| step[j] == 0))
        .collect();
    let mut k = 1;
    while !frontier.is_empty() && (texels < 0 || k <= texels) {
        let mark = -(k + 1);
        let mut candidates = Vec::new();
        for &f in &frontier {
            for j in neighbors(f) {
                if step[j] == 0 {
                    step[j] = mark;
                    candidates.push(j);
                }
            }
        }
        candidates.sort_unstable();
        let colors: Vec<[u8; 4]> = candidates
            .iter()
            .map(|&c| {
                let (mut r, mut g, mut b, mut a, mut rw, mut gw, mut bw, mut count) =
                    (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
                for j in neighbors(c) {
                    if step[j] <= 0 {
                        continue;
                    }
                    let p = &output[j * 4..j * 4 + 4];
                    let al = p[3] as u64;
                    r += p[0] as u64;
                    g += p[1] as u64;
                    b += p[2] as u64;
                    a += al;
                    count += 1;
                    rw += p[0] as u64 * al;
                    gw += p[1] as u64 * al;
                    bw += p[2] as u64 * al;
                }
                let round = |s: u64, c: u64| ((2 * s + c) / (2 * c)) as u8;
                if a > 0 {
                    [round(rw, a), round(gw, a), round(bw, a), round(a, count)]
                } else {
                    [
                        round(r, count),
                        round(g, count),
                        round(b, count),
                        round(a, count),
                    ]
                }
            })
            .collect();
        for (&c, v) in candidates.iter().zip(colors) {
            output[c * 4..c * 4 + 4].copy_from_slice(&v);
            step[c] = k + 1;
        }
        frontier = candidates;
        k += 1;
    }
    output
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn u01(&mut self) -> f64 {
        (self.next() >> 11) as f64 / 9007199254740992.0
    }
}

fn random_image(w: usize, h: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..w * h * 4).map(|_| (rng.next() >> 8) as u8).collect()
}

fn random_keep(w: usize, h: usize, seed: u64, percent: f64) -> Vec<bool> {
    let mut rng = Rng(seed);
    (0..w * h).map(|_| rng.u01() * 100.0 < percent).collect()
}

fn in_pool<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(f)
}

#[test]
fn dilation_matches_the_plain_loop_for_any_number_of_threads() {
    // 1000 × 700 で覆い 10%: 段 1 の候補が約 36 万で、既定の塊（262144）を超えて 2 つに分かれ、どちらも並列の下限（16384）を超える
    // （既定の分け方のままで、塊の切れ目と並列の枝を通る。小さい塊・下限での切れ目は every_way_of_splitting が受け持つ）
    let (w, h) = (1000usize, 700usize);
    let image = random_image(w, h, 1);
    let keep = random_keep(w, h, 2, 10.0);
    let first_ring = (0..w * h)
        .filter(|&i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            !keep[i]
                && (-1..=1).any(|dy| {
                    (-1..=1).any(|dx| {
                        let (nx, ny) = (x + dx, y + dy);
                        (dx != 0 || dy != 0)
                            && (0..w as i64).contains(&nx)
                            && (0..h as i64).contains(&ny)
                            && keep[ny as usize * w + nx as usize]
                    })
                })
        })
        .count();
    assert!(
        first_ring > Tuning::DEFAULT.color_chunk
            && first_ring - Tuning::DEFAULT.color_chunk >= Tuning::DEFAULT.parallel_min,
        "段 1 の候補 {first_ring} 個が、既定の塊 {} で 2 つに分かれ、どちらも並列の下限 {} 以上になる大きさ",
        Tuning::DEFAULT.color_chunk,
        Tuning::DEFAULT.parallel_min
    );
    for texels in [1, 3, -1] {
        let expected = reference_dilate(&image, w, h, &keep, texels);
        let reach = Reach::from_setting(texels).unwrap();
        for threads in [1, 2, 7] {
            let got = in_pool(threads, || {
                dilate(&image, w as u32, h as u32, &keep, reach, ALL).unwrap()
            });
            assert!(got == expected, "texels {texels}・{threads} スレッド");
        }
    }
    // 細い・端の形
    for (w, h, percent) in [
        (1usize, 40usize, 20.0),
        (40, 1, 20.0),
        (3, 3, 30.0),
        (97, 61, 0.5),
    ] {
        let image = random_image(w, h, 5);
        let keep = random_keep(w, h, 6, percent);
        assert!(
            dilate(&image, w as u32, h as u32, &keep, Reach::Fill, ALL).unwrap()
                == reference_dilate(&image, w, h, &keep, -1),
            "{w}x{h}"
        );
    }
}

#[test]
fn coverage_is_the_same_for_any_number_of_threads_and_batch_boundaries() {
    // 7 万個（1 回に並列へ回す束の 65536 を超える）
    let (w, h) = (1200u32, 800u32);
    let mut rng = Rng(3);
    let triangles: Vec<[DVec2; 3]> = (0..70_000)
        .map(|_| {
            let x0 = (rng.u01() * 1.2 - 0.1) * w as f64;
            let y0 = (rng.u01() * 1.2 - 0.1) * h as f64;
            let mut p = |c: f64| c + (rng.u01() * 2.0 - 1.0) * 3.0;
            let (x1, y1, x2, y2) = (p(x0), p(y0), p(x0), p(y0));
            tri((x0, y0), (x1, y1), (x2, y2))
        })
        .collect();
    let base = in_pool(1, || coverage(w, h, triangles.iter().copied()).unwrap());
    assert!(base.iter().any(|&c| c) && base.iter().any(|&c| !c));
    for threads in [2, 5, 11] {
        let got = in_pool(threads, || {
            coverage(w, h, triangles.iter().copied()).unwrap()
        });
        assert!(got == base, "{threads} スレッド");
    }
    // 束（既定 65536 個）の境目に依らない: 束 1 つに収まる前半（ちょうど 65536 個）と後半の覆いの OR は、境目をまたぐ全体の覆いと同じ
    // （束の小さい値での切れ目は every_way_of_splitting が受け持つ）
    let batch = Tuning::DEFAULT.triangle_batch;
    assert!(triangles.len() > batch);
    let mut or = coverage(w, h, triangles[..batch].iter().copied()).unwrap();
    for (o, c) in or
        .iter_mut()
        .zip(coverage(w, h, triangles[batch..].iter().copied()).unwrap())
    {
        *o |= c;
    }
    assert!(base == or, "束の境目をまたぐ全体と、束ごとの OR が違う");
}

#[test]
fn every_way_of_splitting_the_work_gives_the_same_result() {
    // 塊・並列の下限・帯・束を小さくして、境目（塊の切れ目・並列の枝・帯をまたぐ三角形・束の切れ目）を全部通す
    let (w, h) = (97usize, 61usize);
    let image = random_image(w, h, 11);
    let keep = random_keep(w, h, 12, 8.0);
    let expected = reference_dilate(&image, w, h, &keep, -1);
    for (color_chunk, parallel_min) in [(1, 1), (7, 1), (7, usize::MAX), (1000, 1), (1 << 18, 1)] {
        let tuning = Tuning {
            color_chunk,
            parallel_min,
            ..Tuning::DEFAULT
        };
        let got = dilate_tuned(
            &image,
            w as u32,
            h as u32,
            &keep,
            Reach::Fill,
            ALL,
            None,
            tuning,
        )
        .unwrap();
        assert!(
            got == expected,
            "chunk {color_chunk}・parallel_min {parallel_min}"
        );
    }
    let mut rng = Rng(21);
    let triangles: Vec<[DVec2; 3]> = (0..400)
        .map(|_| {
            let x0 = (rng.u01() * 1.2 - 0.1) * w as f64;
            let y0 = (rng.u01() * 1.2 - 0.1) * h as f64;
            let mut p = |c: f64| c + (rng.u01() * 2.0 - 1.0) * 14.0;
            let (x1, y1, x2, y2) = (p(x0), p(y0), p(x0), p(y0));
            tri((x0, y0), (x1, y1), (x2, y2))
        })
        .collect();
    let base = coverage(w as u32, h as u32, triangles.iter().copied()).unwrap();
    assert!(base.iter().any(|&c| c) && base.iter().any(|&c| !c));
    for (triangle_batch, band_rows) in [(1, 1), (3, 1), (3, 5), (64, 7), (1 << 16, 1000), (0, 0)] {
        let tuning = Tuning {
            triangle_batch,
            band_rows,
            ..Tuning::DEFAULT
        };
        let got = coverage_tuned(w as u32, h as u32, triangles.iter().copied(), tuning).unwrap();
        assert!(got == base, "batch {triangle_batch}・band {band_rows}");
    }
}

// ───────── チャンネル 1 つのファイルの画像 ─────────

#[test]
fn a_channel_image_is_the_composite_and_a_normal_follows_the_file_direction() {
    let mut doc = document(&[
        (Channel::Color, &|x, _| [x as u8 * 30, 10, 20, 255]),
        (Channel::Emission, &|_, _| [200, 100, 50, 128]),
        (Channel::Roughness, &|_, y| [y as u8 * 60, 0, 0, 200]),
        (Channel::Normal, &|x, _| [x as u8 * 20, 100, 220, 255]),
    ]);
    // Color・Emission・スカラーは、合成のバイトそのまま（詰めない・アルファを掛けない・不透明にしない）
    for channel in [Channel::Color, Channel::Emission, Channel::Roughness] {
        assert_eq!(
            channel_image(&doc, channel, ALL).unwrap(),
            doc.composite_channel(channel, doc.bounds()).unwrap(),
            "{channel:?}"
        );
    }
    let emission = channel_image(&doc, Channel::Emission, ALL).unwrap();
    assert_eq!(
        px(&emission, 3, 1),
        [200, 100, 50, 128],
        "テンプレートの Emission と違い、アルファのまま"
    );
    // Color は、テンプレートの BaseColor と同じバイト
    assert_eq!(
        channel_image(&doc, Channel::Color, ALL).unwrap(),
        build(
            &doc,
            image(&ExportTemplate::unity_hdrp(), "BaseColor"),
            None,
            ALL
        )
        .unwrap()
    );
    // Normal: OpenGL はテンプレートの Normal と同じバイト、DirectX は緑だけが 255 − G
    let opengl = channel_image(&doc, Channel::Normal, ALL).unwrap();
    assert_eq!(
        opengl,
        build(
            &doc,
            image(&ExportTemplate::unity_hdrp(), "Normal"),
            None,
            ALL
        )
        .unwrap()
    );
    doc.set_normal_settings(
        doc.normal_settings()
            .with_file_direction(NormalYDirection::DirectX),
        false,
    )
    .unwrap();
    let directx = channel_image(&doc, Channel::Normal, ALL).unwrap();
    assert_ne!(directx, opengl);
    for (a, b) in opengl.chunks(4).zip(directx.chunks(4)) {
        assert_eq!([a[0], 255 - a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]]);
    }
    assert_eq!(
        build(
            &doc,
            image(&ExportTemplate::unity_hdrp(), "Normal"),
            None,
            ALL
        )
        .unwrap(),
        opengl,
        "テンプレートは Unity 向けなので、ファイルの向きに依らず OpenGL"
    );
}

#[test]
fn a_channel_image_includes_the_normal_derived_from_height() {
    let mut doc = document(&[(Channel::Height, &|x, _| [x as u8 * 30, 0, 0, 255])]);
    doc.set_normal_settings(
        NormalSettings::new(true, 8.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap(),
        false,
    )
    .unwrap();
    let derived = channel_image(&doc, Channel::Normal, ALL).unwrap();
    assert_eq!(derived, doc.normal_output(ALL).unwrap());
    assert_ne!(
        px(&derived, 3, 1),
        [128, 128, 255, 255],
        "傾きのある所は平らでない"
    );
}

#[test]
fn a_channel_image_refuses_before_allocating_and_for_a_channel_the_document_lacks() {
    let doc = document(&[(Channel::Color, &|_, _| [1, 2, 3, 255])]);
    let n = (W * H) as u64;
    assert_eq!(channel_working_bytes(&doc, Channel::Color), 4 * n);
    assert_eq!(
        channel_working_bytes(&doc, Channel::Normal),
        doc.normal_working_bytes()
    );
    for channel in [Channel::Color, Channel::Normal] {
        let needed = channel_working_bytes(&doc, channel);
        assert_eq!(
            channel_image(&doc, channel, needed - 1),
            Err(ExportError::WorkingBudgetExceeded {
                needed,
                allowed: needed - 1
            }),
            "{channel:?}"
        );
        assert!(
            channel_image(&doc, channel, needed).is_ok(),
            "{channel:?}: ちょうどなら通る"
        );
    }
    let missing = Channel::from_index(40).unwrap();
    assert!(doc.channel_info(missing).is_none());
    assert!(matches!(
        channel_image(&doc, missing, ALL),
        Err(ExportError::Core(_))
    ));
}

// ───────── 段の地図と、矩形だけの塗り広げ直し ─────────

/// `pixels` から矩形 `r` を切り出す（行は下から上のまま）。
fn cut(pixels: &[u8], w: usize, r: Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity((r.width * r.height * 4) as usize);
    for y in r.y..r.y + r.height {
        let start = (y as usize * w + r.x as usize) * 4;
        out.extend_from_slice(&pixels[start..start + r.width as usize * 4]);
    }
    out
}

/// `inner` を `by` だけ広げて画像の中に切った矩形。
fn grow(inner: Rect, by: u32, w: u32, h: u32) -> Rect {
    let x0 = inner.x.saturating_sub(by);
    let y0 = inner.y.saturating_sub(by);
    let x1 = (inner.x + inner.width + by).min(w);
    let y1 = (inner.y + inner.height + by).min(h);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

#[test]
fn rings_count_the_same_steps_as_the_whole_dilation() {
    let (w, h) = (61usize, 37usize);
    let keep = random_keep(w, h, 31, 3.0);
    let reach = 6;
    let rings = Rings::new(w as u32, h as u32, &keep, reach).unwrap();
    // 段 k のテクセルは、覆いからの 8 近傍の距離（チェビシェフ距離）がちょうど k
    for y in 0..h {
        for x in 0..w {
            let d = (0..h)
                .flat_map(|yy| (0..w).map(move |xx| (xx, yy)))
                .filter(|&(xx, yy)| keep[yy * w + xx])
                .map(|(xx, yy)| xx.abs_diff(x).max(yy.abs_diff(y)) as u32)
                .min();
            let expected = d.filter(|&d| d <= reach);
            assert_eq!(rings.ring(x as u32, y as u32), expected, "({x}, {y})");
        }
    }
    assert_eq!(rings.bytes(), w * h);
    assert_eq!(rings.reach(), reach);
    // 矩形の中の種類（覆いの中・塗り広げる）
    for (x, y, rw, rh) in [(0, 0, w, h), (3, 4, 9, 5), (20, 10, 1, 1), (50, 30, 40, 40)] {
        let r = Rect::new(x as u32, y as u32, rw as u32, rh as u32);
        let (mut k, mut f) = (false, false);
        for yy in y..(y + rh).min(h) {
            for xx in x..(x + rw).min(w) {
                match rings.ring(xx as u32, yy as u32) {
                    Some(0) => k = true,
                    Some(_) => f = true,
                    None => {}
                }
            }
        }
        assert_eq!(rings.kinds_in(r), (k, f), "{r:?}");
    }
    // 覆いが無ければどこにも届かない
    let none = Rings::new(5, 4, &[false; 20], 3).unwrap();
    assert!((0..4).all(|y| (0..5).all(|x| none.ring(x, y).is_none())));
}

#[test]
fn redilating_a_region_gives_the_whole_dilation_inside_it() {
    let (w, h) = (83u32, 59u32);
    let image = random_image(w as usize, h as usize, 41);
    let keep = random_keep(w as usize, h as usize, 42, 2.0);
    let mut rng = Rng(43);
    for reach in [1u32, 2, 5, 12] {
        let whole = dilate(&image, w, h, &keep, Reach::Texels(reach), ALL).unwrap();
        let rings = Rings::new(w, h, &keep, reach).unwrap();
        for case in 0..40 {
            // 端に付く矩形・1 テクセルの矩形・画像全体も通す
            let inner = match case {
                0 => Rect::new(0, 0, w, h),
                1 => Rect::new(0, 0, 1, 1),
                2 => Rect::new(w - 3, h - 2, 3, 2),
                _ => {
                    let x = (rng.u01() * w as f64) as u32 % w;
                    let y = (rng.u01() * h as f64) as u32 % h;
                    let rw = 1 + (rng.u01() * 20.0) as u32;
                    let rh = 1 + (rng.u01() * 20.0) as u32;
                    Rect::new(x, y, rw.min(w - x), rh.min(h - y))
                }
            };
            // 入力は、内の矩形を段数だけ広げた範囲（それより広くても同じ）
            for extra in [0, 3] {
                let outer = grow(inner, reach + extra, w, h);
                let mut pixels = cut(&image, w as usize, outer);
                rings.dilate_region(&mut pixels, outer, inner).unwrap();
                let local = Rect::new(
                    inner.x - outer.x,
                    inner.y - outer.y,
                    inner.width,
                    inner.height,
                );
                assert!(
                    cut(&pixels, outer.width as usize, local) == cut(&whole, w as usize, inner),
                    "段数 {reach}・{inner:?}・外 {outer:?}"
                );
            }
        }
    }
}

#[test]
fn redilating_a_region_is_the_same_for_any_number_of_threads() {
    // 段 1 の候補が並列の下限（16384）を超える大きさで、並列の枝を通す
    let (w, h) = (700u32, 500u32);
    let image = random_image(w as usize, h as usize, 51);
    let keep = random_keep(w as usize, h as usize, 52, 5.0);
    let reach = 4;
    let whole = dilate(&image, w, h, &keep, Reach::Texels(reach), ALL).unwrap();
    let rings = Rings::new(w, h, &keep, reach).unwrap();
    let inner = Rect::new(0, 0, w, h);
    for threads in [1, 3, 8] {
        let mut pixels = image.clone();
        in_pool(threads, || rings.dilate_region(&mut pixels, inner, inner)).unwrap();
        assert!(pixels == whole, "{threads} スレッド");
        let again = in_pool(threads, || Rings::new(w, h, &keep, reach)).unwrap();
        assert_eq!(again, rings, "{threads} スレッドの段の地図");
    }
}

#[test]
fn rings_and_region_redilation_refuse_bad_inputs() {
    assert!(Rings::new(0, 4, &[], 1).is_err());
    assert!(Rings::new(4, 4, &[false; 15], 1).is_err());
    assert!(Rings::new(4, 4, &[false; 16], MAX_RING_REACH + 1).is_err());
    assert!(Rings::new(4, 4, &[false; 16], MAX_RING_REACH).is_ok());
    let mut keep = vec![false; 64];
    keep[27] = true;
    let rings = Rings::new(8, 8, &keep, 2).unwrap();
    let inner = Rect::new(3, 3, 2, 2);
    let outer = grow(inner, 2, 8, 8);
    let mut pixels = vec![0u8; (outer.width * outer.height * 4) as usize];
    // 外の矩形が画像からはみ出す・画素の数が違う・段数ぶんの入力が足りない
    assert!(rings
        .dilate_region(&mut pixels, Rect::new(4, 4, 6, 6), inner)
        .is_err());
    assert!(rings.dilate_region(&mut pixels[4..], outer, inner).is_err());
    let narrow = grow(inner, 1, 8, 8);
    let mut small = vec![0u8; (narrow.width * narrow.height * 4) as usize];
    assert!(rings.dilate_region(&mut small, narrow, inner).is_err());
    assert!(rings.dilate_region(&mut pixels, outer, inner).is_ok());
}
