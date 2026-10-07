use crate::generator_support;
use crate::golden_update;
use generator_support::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use yolu_core::{generator::*, BlendMode, Channel, ChannelKind, Rect, Rgba8};
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
/// 索引は 338 事例（8 種 × 変種 12 × 対象 3 = 288、ランプ 2、部分スタック 48）。1 行消えても落ちる。
#[test]
fn index_lists_exactly_the_documented_338_cases() {
    let lines: Vec<&str> = include_str!("../generator-index.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
        .collect();
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for k in 0..8 {
        for v in 0..12 {
            for t in 0..3 {
                expected.insert(format!("{k}-{v}-{t}"));
            }
        }
    }
    expected.extend(["ramp-0".to_string(), "ramp-1".to_string()]);
    expected.extend((0..48).map(|v| format!("anchor-{v}")));
    assert_eq!(expected.len(), 338);
    let names: Vec<&str> = lines
        .iter()
        .map(|l| {
            let (name, hash) = l.split_once(' ').unwrap();
            assert!(
                hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
                "{name}"
            );
            name
        })
        .collect();
    assert_eq!(names.len(), 338);
    assert_eq!(
        names.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
        expected
    );
}
#[test]
fn csharp_all_generators_and_ramps_match_every_byte() {
    for line in include_str!("../generator-index.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let (name, expected) = line.split_once(' ').unwrap();
        let bytes = if name.starts_with("ramp-") {
            let scalar = name == "ramp-1";
            (-10..=1010)
                .flat_map(|i| {
                    ramp()
                        .evaluate(i as f64 / 1000., scalar)
                        .unwrap()
                        .to_array()
                })
                .collect()
        } else if let Some(v) = name.strip_prefix("anchor-") {
            anchor_scene(v.parse().unwrap())
        } else {
            let p: Vec<usize> = name.split('-').map(|s| s.parse().unwrap()).collect();
            run(
                KINDS[p[0]],
                p[1],
                [Target::Color, Target::Scalar, Target::Mask][p[2]],
                41,
                29,
            )
        };
        let got = hash(&bytes);
        if got != expected && golden_update::updating() {
            let path = golden_update::tests_dir().join("generator-index.txt");
            golden_update::replace_line(&path, name, &format!("{name} {got}"));
            continue;
        }
        assert_eq!(got, expected, "{name}");
        if let Ok(dir) = std::env::var("YOLU_GENERATOR_GOLDEN") {
            let expected =
                std::fs::read(std::path::Path::new(&dir).join(format!("{name}.rgba"))).unwrap();
            assert_eq!(bytes, expected, "C# 画素の直接比較: {name}");
        }
    }
}
#[test]
fn parallelism_does_not_change_pixels() {
    let a = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let b = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    for v in 0..48 {
        assert_eq!(a.install(|| anchor_scene(v)), b.install(|| anchor_scene(v)));
    }
    for k in KINDS {
        for t in [Target::Color, Target::Scalar, Target::Mask] {
            assert_eq!(
                a.install(|| run(k, 5, t, 67, 35)),
                b.install(|| run(k, 5, t, 67, 35))
            );
        }
    }
}
#[test]
fn ramp_known_answers_and_transparent_rgb() {
    let r = Ramp::default();
    assert_eq!(
        r.evaluate(0.5, false).unwrap(),
        Rgba8::new(128, 128, 128, 255)
    );
    assert_eq!(r.evaluate(-3., false).unwrap(), Rgba8::new(0, 0, 0, 255));
    assert_eq!(
        r.evaluate(7., true).unwrap(),
        Rgba8::new(255, 255, 255, 255)
    );
    let r = ramp();
    assert_eq!(r.sample_stops(0.63, false).unwrap().a, 0);
    assert_ne!(r.sample_stops(0.63, false).unwrap().r, 0);
    assert!(r.evaluate(f64::NAN, false).is_err());
}
#[test]
fn ramp_rejects_bad_points_and_budgets() {
    let r = Ramp::default();
    for n in [0, 1, 33] {
        assert!(Ramp::new(vec![r.colors()[0]; n], r.opacities().to_vec(), None).is_err());
    }
    let mut c = r.colors().to_vec();
    c[0].midpoint = 0.;
    assert!(Ramp::new(c, r.opacities().to_vec(), None).is_err());
    for point in [
        CurvePoint { x: 0.01, y: 0.5 },
        CurvePoint {
            x: 0.5,
            y: f64::NAN,
        },
        CurvePoint { x: 1., y: 0.5 },
    ] {
        assert!(Ramp::new(
            r.colors().to_vec(),
            r.opacities().to_vec(),
            Some(vec![
                CurvePoint { x: 0., y: 0. },
                point,
                CurvePoint { x: 1., y: 1. }
            ])
        )
        .is_err());
    }
}
#[test]
fn curve_is_bounded_and_monotone_on_each_segment() {
    let r = ramp();
    for pair in r.curve().windows(2) {
        let mut prev = r.curve_value(pair[0].x).unwrap();
        for i in 1..=100 {
            let x = pair[0].x + (pair[1].x - pair[0].x) * i as f64 / 100.;
            let y = r.curve_value(x).unwrap();
            assert!((0. ..=1.).contains(&y));
            assert!((y - prev) * (pair[1].y - pair[0].y) >= -1e-15);
            prev = y;
        }
    }
}
#[test]
fn shape_known_answers_and_refusals() {
    let v = Volume::default();
    assert_eq!(v.value_at([0.; 3]).unwrap(), 1.);
    assert_eq!(v.value_at([0.5, 0., 0.]).unwrap(), 0.);
    assert_eq!(v.value_at([0.375, 0., 0.]).unwrap(), 0.5);
    for shape in [Shape::Sphere, Shape::Plane] {
        let v = Volume { shape, ..v };
        assert_eq!(
            v.value_at([0., 0.5, 0.]).unwrap(),
            if shape == Shape::Plane { 1. } else { 0. }
        );
    }
    assert!(Volume { size: [0.; 3], ..v }.validate().is_err());
    assert!(ModelFrame::new([0.; 3], [0.; 4]).is_err());
    assert!(ModelFrame::new([0.; 3], [f64::MAX; 4]).is_err());
}
#[test]
fn settings_reject_nonfinite_unused_fields_and_invalid_pins() {
    let s = Settings::new(Kind::Thickness);
    let mut invalid = vec![];
    let mut g = s.clone();
    g.low = f64::NAN;
    invalid.push(g);
    let mut g = s.clone();
    g.high = 0.0001;
    invalid.push(g);
    let mut g = s.clone();
    g.balance = 0.1;
    invalid.push(g);
    let mut g = s.clone();
    g.ramp = Some(Ramp::default());
    invalid.push(g);
    let mut g = s.clone();
    g.pins.insert(MapKind::WorldNormal, KEY.into());
    invalid.push(g);
    let mut g = s.clone();
    g.pins.insert(MapKind::Thickness, "A".repeat(64));
    invalid.push(g);
    for g in invalid {
        assert!(g.validate().is_err());
    }
    let mut g = Settings::new(Kind::IdColor);
    g.id_colors = vec![1, 1];
    assert!(g.validate().is_err());
    g.id_colors = vec![0xffffff + 1];
    assert!(g.validate().is_err());
}
#[test]
fn map_refusals_keep_input_and_reason() {
    let s = settings(Kind::Thickness, 0);
    let owned = Maps::new(&s, 7, 5);
    let source_data = pixels(7, 5, Target::Color);
    let source = Image::new(&source_data, 7, 5).unwrap();
    for (state, reason) in [
        (MapState::Stale, Inactive::StaleMap(MapKind::Thickness)),
        (
            MapState::Unverified,
            Inactive::UnverifiedMap(MapKind::Thickness),
        ),
    ] {
        let mut maps = owned.maps();
        maps[0].state = state;
        let b = BoundGenerator::bind(
            &s,
            &maps,
            Some(frame()),
            (7, 5),
            Err(anchor::Issue::NotChosen),
        )
        .unwrap();
        let out = evaluate(
            &source,
            &b,
            Rect::new(0, 0, 7, 5),
            Target::Color,
            1.,
            &Options::default(),
        )
        .unwrap();
        assert_eq!(out.inactive, Some(reason));
        assert_eq!(out.pixels, source_data);
    }
    let b = BoundGenerator::bind(&s, &[], None, (7, 5), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(
        b.inactive(),
        Some(&Inactive::MissingMap(MapKind::Thickness))
    );
    let maps = owned.maps();
    let b = BoundGenerator::bind(&s, &maps, None, (8, 5), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::MapSize(MapKind::Thickness)));
    let mut s = s;
    s.pins.insert(MapKind::Thickness, "b".repeat(64));
    let b = BoundGenerator::bind(&s, &maps, None, (7, 5), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(
        b.inactive(),
        Some(&Inactive::PinMismatch(MapKind::Thickness))
    );
}
#[test]
fn budgets_cancellation_and_regions_are_atomic() {
    let s = settings(Kind::Thickness, 0);
    let owned = Maps::new(&s, 7, 5);
    let maps = owned.maps();
    let b = BoundGenerator::bind(&s, &maps, None, (7, 5), Err(anchor::Issue::Missing)).unwrap();
    let data = pixels(7, 5, Target::Color);
    let image = Image::new(&data, 7, 5).unwrap();
    let rect = Rect::new(0, 0, 7, 5);
    assert!(matches!(
        evaluate(
            &image,
            &b,
            rect,
            Target::Color,
            1.,
            &Options {
                budget_bytes: 139,
                cancel: None
            }
        ),
        Err(Error::Budget {
            needed: 140,
            budget: 139
        })
    ));
    assert!(evaluate(
        &image,
        &b,
        rect,
        Target::Color,
        1.,
        &Options {
            budget_bytes: 140,
            cancel: None
        }
    )
    .is_ok());
    assert!(matches!(
        evaluate(
            &image,
            &b,
            rect,
            Target::Color,
            1.,
            &Options {
                budget_bytes: 140,
                cancel: Some(&AtomicBool::new(true))
            }
        ),
        Err(Error::Cancelled)
    ));
    assert!(evaluate(
        &image,
        &b,
        Rect::new(u32::MAX, 0, 7, 5),
        Target::Color,
        1.,
        &Options::default()
    )
    .is_err());
    let full = evaluate(&image, &b, rect, Target::Color, 1., &Options::default())
        .unwrap()
        .pixels;
    let part = evaluate(
        &image,
        &b,
        Rect::new(2, 1, 3, 2),
        Target::Color,
        1.,
        &Options::default(),
    )
    .unwrap()
    .pixels;
    for y in 0..2 {
        assert_eq!(
            &part[y * 12..(y + 1) * 12],
            &full[((y + 1) * 7 + 2) * 4..((y + 1) * 7 + 5) * 4]
        );
    }
    assert_eq!(data, pixels(7, 5, Target::Color));
}
#[test]
fn anchor_strict_order_rejects_self_forward_and_cycles() {
    use anchor::*;
    let points = [
        Point {
            id: 1,
            host: 0,
            placement: Placement::Layer,
        },
        Point {
            id: 2,
            host: 1,
            placement: Placement::Mask,
        },
    ];
    assert_eq!(resolve(&points, 1, 1, 2), Ok(points[0]));
    assert_eq!(resolve(&points, 1, 0, 2), Err(Issue::NotBelow));
    assert_eq!(resolve(&points, 2, 0, 2), Err(Issue::NotBelow));
    assert_eq!(resolve(&points, 0, 1, 2), Err(Issue::NotChosen));
    assert_eq!(resolve(&points, 3, 1, 2), Err(Issue::Missing));
    assert!(validate_points(&[points[0], points[0]], 2).is_err());
}
#[test]
fn anchor_value_uses_coverage_and_mask_density() {
    use anchor::*;
    let data = [200, 50, 10, 128];
    let image = Image::new(&data, 1, 1).unwrap();
    let a = LayerSample {
        source: &image,
        read: Read::Scalar,
    };
    assert_eq!(a.value(0, 0), Some((200. / 255.) * (128. / 255.)));
    let m = MaskSample::new(Mask {
        source: &image,
        enabled: true,
        inverted: true,
        density: 0.4,
    })
    .unwrap();
    assert_eq!(m.value(0, 0), Some(1. - 0.4 * (1. - 128. / 255.)));
    assert_eq!(m.value(1, 0), None);
}
#[test]
fn anchor_omits_above_and_ancestor_opacity() {
    use anchor::*;
    let mut layers = vec![
        Layer::new(Content::Fill(Rgba8::new(40, 60, 80, 255))),
        Layer::new(Content::Group),
        Layer::new(Content::Fill(Rgba8::new(200, 0, 0, 128))),
        Layer::new(Content::Fill(Rgba8::new(0, 255, 0, 255))),
    ];
    layers[1].blend = BlendMode::PassThrough;
    layers[1].opacity = 0.;
    layers[1].visible = false;
    layers[2].parent = Some(1);
    layers[3].parent = Some(1);
    layers[3].clipping = true;
    let p = Plan::new(&layers, 2, (1, 1), ChannelKind::Color).unwrap();
    assert_eq!(
        p.pixel(0, 0),
        yolu_core::blend::blend(
            Rgba8::new(40, 60, 80, 255),
            Rgba8::new(200, 0, 0, 128),
            1.,
            BlendMode::Normal
        )
    );
    layers[1].blend = BlendMode::Normal;
    let p = Plan::new(&layers, 2, (1, 1), ChannelKind::Color).unwrap();
    assert_eq!(p.pixel(0, 0), Rgba8::new(200, 0, 0, 128));
}
#[test]
fn anchor_rejects_parent_cycles_normal_and_budget() {
    use anchor::*;
    let mut layers = vec![Layer::new(Content::Group), Layer::new(Content::Group)];
    layers[0].parent = Some(1);
    layers[1].parent = Some(0);
    assert!(Plan::new(&layers, 0, (1, 1), ChannelKind::Color).is_err());
    let layers = vec![Layer::new(Content::Fill(Rgba8::new(1, 2, 3, 255)))];
    assert!(Plan::new(&layers, 0, (1, 1), ChannelKind::Normal).is_err());
    let p = Plan::new(&layers, 0, (1, 1), ChannelKind::Color).unwrap();
    assert!(matches!(
        p.evaluate(
            Rect::new(0, 0, 1, 1),
            &Options {
                budget_bytes: 3,
                cancel: None
            }
        ),
        Err(Error::Budget { .. })
    ));
}
#[test]
fn mask_uses_only_alpha_even_for_inactive_stages() {
    let data = [231, 71, 99, 42];
    let image = Image::new(&data, 1, 1).unwrap();
    let s = Settings::new(Kind::Thickness);
    let b = BoundGenerator::bind(&s, &[], None, (1, 1), Err(anchor::Issue::Missing)).unwrap();
    for strength in [0., 1.] {
        assert_eq!(
            evaluate(
                &image,
                &b,
                Rect::new(0, 0, 1, 1),
                Target::Mask,
                strength,
                &Options::default()
            )
            .unwrap()
            .pixels,
            [0, 0, 0, 42]
        );
    }
}
#[test]
fn rejects_bad_map_buffers_and_reports_unavailable_inputs() {
    let mut s = settings(Kind::ShapeGradient, 1);
    let owned = Maps::new(&s, 3, 2);
    let mut maps = owned.maps();
    let b = BoundGenerator::bind(&s, &maps, None, (3, 2), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::MissingFrame));
    s.noise_space = NoiseSpace::Model;
    maps[0].bounds_max = maps[0].bounds_min;
    let b = BoundGenerator::bind(
        &s,
        &maps,
        Some(frame()),
        (3, 2),
        Err(anchor::Issue::Missing),
    )
    .unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::EmptyBounds));
    maps[0].data = &[];
    assert!(BoundGenerator::bind(
        &s,
        &maps,
        Some(frame()),
        (3, 2),
        Err(anchor::Issue::Missing)
    )
    .is_err());
    let s = Settings::new(Kind::Anchor);
    let b = BoundGenerator::bind(&s, &[], None, (3, 2), Err(anchor::Issue::NotBelow)).unwrap();
    assert_eq!(
        b.inactive(),
        Some(&Inactive::Anchor(anchor::Issue::NotBelow))
    );
    let s = Settings::new(Kind::IdColor);
    let owned = Maps::new(&s, 3, 2);
    let maps = owned.maps();
    let b = BoundGenerator::bind(&s, &maps, None, (3, 2), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::NoIdColors));
}
#[test]
fn empty_map_texels_and_transparent_rgb_are_preserved() {
    let s = settings(Kind::Thickness, 0);
    let mut owned = Maps::new(&s, 4, 3);
    owned.data[0].2.fill(0);
    let maps = owned.maps();
    let data = pixels(4, 3, Target::Color);
    let src = Image::new(&data, 4, 3).unwrap();
    let b = BoundGenerator::bind(&s, &maps, None, (4, 3), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(
        evaluate(
            &src,
            &b,
            Rect::new(0, 0, 4, 3),
            Target::Color,
            1.,
            &Options::default()
        )
        .unwrap()
        .pixels,
        data
    );
    let result = run(Kind::ShapeGradient, 5, Target::Color, 41, 29);
    let input = pixels(41, 29, Target::Color);
    for (a, b) in result
        .as_chunks::<4>()
        .0
        .iter()
        .zip(input.as_chunks::<4>().0)
    {
        if b[3] == 0 {
            assert_eq!(a, b);
        }
    }
}
#[test]
fn presets_keep_stored_rgb_and_independent_opacity() {
    let fg = Rgba8::new(219, 31, 71, 3);
    let bg = Rgba8::new(7, 19, 127, 0);
    let r = Ramp::preset(Preset::ForegroundTransparent, fg, bg);
    assert_eq!(r.evaluate(1., false).unwrap(), Rgba8::new(219, 31, 71, 0));
    assert_eq!(r.evaluate(0., false).unwrap(), Rgba8::new(219, 31, 71, 255));
    let r = Ramp::preset(Preset::ForegroundBackground, fg, bg);
    assert_eq!(r.evaluate(1., false).unwrap(), Rgba8::new(7, 19, 127, 255));
    let r = Ramp::preset(Preset::WarmCool, fg, bg);
    assert_eq!(
        r.evaluate(0., false).unwrap(),
        Rgba8::new(255, 110, 40, 255)
    );
    assert_eq!(
        Ramp::preset(Preset::WhiteBlack, fg, bg)
            .evaluate(1., false)
            .unwrap(),
        Rgba8::new(0, 0, 0, 255)
    );
}
#[test]
fn model_noise_depends_on_position_and_uv_noise_on_canvas() {
    let mut s = Settings::new(Kind::Thickness);
    s.noise_amount = 1.;
    s.noise_seed = -782;
    s.noise_scale = 0.031;
    let mut owned = Maps::new(&s, 9, 5);
    for (_, data, coverage) in &mut owned.data {
        data.fill(45000);
        coverage.fill(1);
    }
    let maps = owned.maps();
    let b = BoundGenerator::bind(&s, &maps, None, (9, 5), Err(anchor::Issue::Missing)).unwrap();
    assert_eq!(b.value(0, 0), b.value(8, 4));
    s.noise_space = NoiseSpace::Uv;
    let b = BoundGenerator::bind(&s, &maps, None, (9, 5), Err(anchor::Issue::Missing)).unwrap();
    assert_ne!(b.value(0, 0), b.value(8, 4));
}

// ---- 取消 ----
// 並列度 1 のプールでは行が上から順に処理される。読んだ画素の数を数え、行の境界の確認と返却直前の確認を別々に固定する。
const CW: u32 = 16;
const CH: u32 = 32;
struct Probe<'a> {
    reads: AtomicUsize,
    cancel_at: Option<usize>,
    cancel: &'a AtomicBool,
}
impl Source for Probe<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (CW, CH)
    }
    fn pixel(&self, _: u32, _: u32) -> Rgba8 {
        if Some(self.reads.fetch_add(1, Ordering::Relaxed)) == self.cancel_at {
            self.cancel.store(true, Ordering::Relaxed);
        }
        Rgba8::new(1, 2, 3, 255)
    }
}
fn probe_run(
    cancel_at: Option<usize>,
    pre_cancelled: bool,
    run: &(impl Fn(&Probe<'_>, &Options<'_>) -> Result<(), Error> + Sync),
) -> (Result<(), Error>, usize) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let cancel = AtomicBool::new(pre_cancelled);
    let probe = Probe {
        reads: AtomicUsize::new(0),
        cancel_at,
        cancel: &cancel,
    };
    let options = Options {
        budget_bytes: u64::from(CW) * u64::from(CH) * 4,
        cancel: Some(&cancel),
    };
    let result = pool.install(|| run(&probe, &options));
    (result, probe.reads.load(Ordering::Relaxed))
}
fn check_cancellation(run: impl Fn(&Probe<'_>, &Options<'_>) -> Result<(), Error> + Sync) {
    let total = (CW * CH) as usize;
    let row = CW as usize;
    // 取消しなければ全画素を 1 回ずつ読む（数え方の確認）。
    assert_eq!(probe_run(None, false, &run), (Ok(()), total));
    // 取消済みなら 1 画素も読まない。
    assert_eq!(
        probe_run(None, true, &run),
        (Err(Error::Cancelled), 0),
        "取消済み"
    );
    // 行 5 の中で立てても、その行は最後まで読み、次の行は読まない（行の境界の確認）。
    for (label, at) in [
        ("行の先頭", 5 * row),
        ("行の途中", 5 * row + row / 2),
        ("行の最後", 6 * row - 1),
    ] {
        assert_eq!(
            probe_run(Some(at), false, &run),
            (Err(Error::Cancelled), 6 * row),
            "{label}"
        );
    }
    // 最後の行の最後の読み出しで立てても、行の確認では拾えず、返却直前の確認で Cancelled になる。
    assert_eq!(
        probe_run(Some(total - 1), false, &run),
        (Err(Error::Cancelled), total),
        "返却直前"
    );
}
#[test]
fn evaluate_checks_cancellation_at_every_row_and_before_returning() {
    let s = Settings::new(Kind::Thickness);
    let b = BoundGenerator::bind(&s, &[], None, (CW, CH), Err(anchor::Issue::Missing)).unwrap();
    check_cancellation(|source, options| {
        evaluate(
            source,
            &b,
            Rect::new(0, 0, CW, CH),
            Target::Color,
            1.,
            options,
        )
        .map(|_| ())
    });
}
#[test]
fn plan_evaluate_checks_cancellation_at_every_row_and_before_returning() {
    check_cancellation(|source, options| {
        let layers = [anchor::Layer::new(anchor::Content::Pixels(source))];
        anchor::Plan::new(&layers, 0, (CW, CH), ChannelKind::Color)
            .unwrap()
            .evaluate(Rect::new(0, 0, CW, CH), options)
            .map(|_| ())
    });
}

// ---- 拒否と境界 ----
fn invalid<T>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::Invalid(_)))
}
#[test]
fn plan_rejects_each_invalid_layer_setting() {
    use anchor::*;
    let plan = |layers: &[Layer<'_>], host: usize| {
        Plan::new(layers, host, (2, 1), ChannelKind::Color).map(|_| ())
    };
    let fill = || Layer::new(Content::Fill(Rgba8::new(1, 2, 3, 255)));
    let group = || Layer::new(Content::Group);
    assert!(plan(&[fill()], 0).is_ok());
    // レイヤーの番号と画像の大きさ
    assert!(invalid(plan(&[fill()], 1)));
    assert!(invalid(plan(&[], 0)));
    for dims in [(0, 1), (1, 0)] {
        assert!(invalid(
            Plan::new(&[fill()], 0, dims, ChannelKind::Color).map(|_| ())
        ));
    }
    // 親: グループなら通り、グループでないレイヤー・範囲外・自分自身は断る。
    let mut layers = vec![group(), fill()];
    layers[1].parent = Some(0);
    assert!(plan(&layers, 1).is_ok());
    for parent in [
        Content::Fill(Rgba8::new(1, 2, 3, 255)),
        Content::Adjustment(yolu_core::AdjustmentSettings::invert()),
    ] {
        let mut layers = vec![Layer::new(parent), fill()];
        layers[1].parent = Some(0);
        assert!(invalid(plan(&layers, 1)));
    }
    let mut layers = vec![fill()];
    layers[0].parent = Some(1);
    assert!(invalid(plan(&layers, 0)));
    let mut layers = vec![group()];
    layers[0].parent = Some(0);
    assert!(invalid(plan(&layers, 0)));
    // グループ以外の PassThrough
    let data = pixels(2, 1, Target::Color);
    let image = Image::new(&data, 2, 1).unwrap();
    for content in [
        Content::Fill(Rgba8::new(1, 2, 3, 255)),
        Content::Adjustment(yolu_core::AdjustmentSettings::invert()),
        Content::Pixels(&image),
    ] {
        let mut l = Layer::new(content);
        assert!(plan(std::slice::from_ref(&l), 0).is_ok());
        l.blend = BlendMode::PassThrough;
        assert!(invalid(plan(&[l], 0)));
    }
    let mut layers = vec![group(), fill()];
    layers[0].blend = BlendMode::PassThrough;
    layers[1].parent = Some(0);
    assert!(plan(&layers, 1).is_ok());
    // 不透明度は有限の 0..1
    for opacity in [0., 1.] {
        let mut l = fill();
        l.opacity = opacity;
        assert!(plan(&[l], 0).is_ok());
    }
    for opacity in [-0.0001, 1.0001, f64::NAN, f64::INFINITY] {
        let mut l = fill();
        l.opacity = opacity;
        assert!(invalid(plan(&[l], 0)), "{opacity}");
    }
    // Pixels・Mask の大きさと濃度
    let small = pixels(1, 1, Target::Color);
    let wrong = Image::new(&small, 1, 1).unwrap();
    assert!(plan(&[Layer::new(Content::Pixels(&image))], 0).is_ok());
    assert!(invalid(plan(&[Layer::new(Content::Pixels(&wrong))], 0)));
    let masked = |source: &dyn Source, density: f64| {
        let mut l = fill();
        l.mask = Some(Mask {
            source,
            enabled: true,
            inverted: false,
            density,
        });
        plan(&[l], 0)
    };
    assert!(masked(&image, 0.).is_ok());
    assert!(masked(&image, 1.).is_ok());
    assert!(invalid(masked(&wrong, 0.5)));
    for density in [-0.0001, 1.0001, f64::NAN] {
        assert!(invalid(masked(&image, density)), "{density}");
    }
}
#[test]
fn plan_depth_limit_is_256_ancestors() {
    use anchor::*;
    let chain = |groups: usize| {
        let mut layers: Vec<Layer<'static>> = (0..groups)
            .map(|i| {
                let mut l = Layer::new(Content::Group);
                l.parent = i.checked_sub(1);
                l
            })
            .collect();
        let mut leaf = Layer::new(Content::Fill(Rgba8::new(1, 2, 3, 255)));
        leaf.parent = groups.checked_sub(1);
        layers.push(leaf);
        layers
    };
    let layers = chain(256);
    let host = layers.len() - 1;
    let plan = Plan::new(&layers, host, (1, 1), ChannelKind::Color).unwrap();
    assert_eq!(plan.pixel(0, 0), Rgba8::new(1, 2, 3, 255));
    let layers = chain(257);
    let host = layers.len() - 1;
    assert!(invalid(
        Plan::new(&layers, host, (1, 1), ChannelKind::Color).map(|_| ())
    ));
}
#[test]
fn bind_rejects_zero_size_duplicate_maps_bad_maps_and_anchor_size() {
    let s = settings(Kind::Thickness, 0);
    let owned = Maps::new(&s, 7, 5);
    let ok = |maps: &[Map<'_>], dims: (u32, u32)| {
        BoundGenerator::bind(&s, maps, None, dims, Err(anchor::Issue::Missing)).map(|_| ())
    };
    let maps = owned.maps();
    assert!(ok(&maps, (7, 5)).is_ok());
    assert!(invalid(ok(&maps, (0, 5))));
    assert!(invalid(ok(&maps, (7, 0))));
    // 同じ種類のマップが 2 つ
    let mut twice = maps.clone();
    twice.push(maps[0].clone());
    assert!(invalid(ok(&twice, (7, 5))));
    // マップ自体の不正（由来の形式・画像の長さ・境界箱）
    let upper = "A".repeat(64);
    let short = "a".repeat(63);
    let nonhex = "g".repeat(64);
    let mut bad: Vec<(&str, Map<'_>)> = vec![];
    for (label, key) in [
        ("大文字", &upper),
        ("短い", &short),
        ("16進でない", &nonhex),
    ] {
        let mut m = maps[0].clone();
        m.condition_key = key;
        bad.push((label, m));
    }
    let mut m = maps[0].clone();
    m.coverage = &m.coverage[1..];
    bad.push(("被覆の長さ", m));
    let mut m = maps[0].clone();
    m.data = &m.data[1..];
    bad.push(("値の長さ", m));
    let mut m = maps[0].clone();
    m.width = 0;
    bad.push(("幅 0", m));
    let mut m = maps[0].clone();
    m.bounds_min = [3., 0., 0.];
    m.bounds_max = [2., 1., 1.];
    bad.push(("境界箱の逆転", m));
    for v in [f64::NAN, f64::INFINITY] {
        let mut m = maps[0].clone();
        m.bounds_max[1] = v;
        bad.push(("境界箱の非有限", m));
    }
    let mut m = maps[0].clone();
    m.bounds_min = [-f64::MAX; 3];
    m.bounds_max = [f64::MAX; 3];
    bad.push(("境界箱の幅が無限大", m));
    for (label, m) in bad {
        assert!(invalid(ok(&[m], (7, 5))), "{label}");
    }
    // Anchor の画像と Generator の大きさ
    let data = pixels(3, 2, Target::Color);
    let image = Image::new(&data, 3, 2).unwrap();
    let a = anchor::LayerSample {
        source: &image,
        read: anchor::Read::Scalar,
    };
    let s = Settings::new(Kind::Anchor);
    let anchored = |dims: (u32, u32)| BoundGenerator::bind(&s, &[], None, dims, Ok(&a)).map(|_| ());
    assert!(anchored((3, 2)).is_ok());
    assert!(invalid(anchored((4, 2))));
    assert!(invalid(anchored((3, 1))));
}
#[test]
fn settings_validate_each_field_on_both_sides_of_its_limit() {
    let check = |label: &str, kind: Kind, expect: bool, edit: &dyn Fn(&mut Settings)| {
        let mut s = Settings::new(kind);
        edit(&mut s);
        assert_eq!(s.validate().is_ok(), expect, "{label}");
    };
    let t = Kind::Thickness;
    check("既定", t, true, &|_| {});
    // レベル
    check("low<0", t, false, &|s| s.low = -0.001);
    check("high>1", t, false, &|s| s.high = 1.001);
    check("high-low=0.001", t, true, &|s| s.high = 0.001);
    check("high-low<0.001", t, false, &|s| s.high = 0.0009);
    check("low>high", t, false, &|s| {
        s.low = 0.8;
        s.high = 0.2
    });
    // 減衰・ノイズ
    for (label, set) in [
        (
            "softness",
            (|s: &mut Settings, v| s.softness = v) as fn(&mut Settings, f64),
        ),
        ("noise_amount", |s, v| s.noise_amount = v),
    ] {
        check(label, t, true, &|s| set(s, 0.));
        check(label, t, true, &|s| set(s, 1.));
        check(label, t, false, &|s| set(s, -0.001));
        check(label, t, false, &|s| set(s, 1.001));
        check(label, t, false, &|s| set(s, f64::NAN));
    }
    check("scale=0.001", t, true, &|s| s.noise_scale = 0.001);
    check("scale<0.001", t, false, &|s| s.noise_scale = 0.0009);
    check("scale=1", t, true, &|s| s.noise_scale = 1.);
    check("scale>1", t, false, &|s| s.noise_scale = 1.001);
    check("scale NaN", t, false, &|s| s.noise_scale = f64::NAN);
    check("scale inf", t, false, &|s| s.noise_scale = f64::INFINITY);
    // 種類ごとの割合・軸
    let d = Kind::Dirt;
    for v in [0., 1.] {
        check("dirt balance", d, true, &|s| s.balance = v);
    }
    for v in [-0.001, 1.001, f64::NAN] {
        check("dirt balance", d, false, &|s| s.balance = v);
    }
    check("balance は Dirt 専用", t, false, &|s| s.balance = 0.6);
    let p = Kind::PositionGradient;
    check("axis=2", p, true, &|s| s.axis = 2);
    check("axis=3", p, false, &|s| s.axis = 3);
    check("axis は PositionGradient 専用", t, false, &|s| {
        s.axis = 0
    });
    // 方向
    let dir = Kind::Direction;
    check("方向 0", dir, false, &|s| s.direction = [0.; 3]);
    check("方向が小さすぎる", dir, false, &|s| {
        s.direction = [5e-7, 0., 0.]
    });
    check("方向が小さい", dir, true, &|s| {
        s.direction = [2e-6, 0., 0.]
    });
    check("方向 1e6", dir, true, &|s| s.direction = [1e6, 0., 0.]);
    check("方向 >1e6", dir, false, &|s| {
        s.direction = [1.0001e6, 0., 0.]
    });
    check("方向 NaN", dir, false, &|s| {
        s.direction = [f64::NAN, 0., 1.]
    });
    check("方向 inf", dir, false, &|s| {
        s.direction = [0., f64::INFINITY, 1.]
    });
    check("ベント法線", dir, true, &|s| s.use_bent_normal = true);
    check("方向は Direction 専用", t, false, &|s| {
        s.direction = [1., 0., 0.]
    });
    check("方向の逆向きも専用", t, false, &|s| {
        s.direction = [0., -1., 0.]
    });
    check("ベント法線は Direction 専用", t, false, &|s| {
        s.use_bent_normal = true
    });
    // 形・ランプ
    let d = Volume::default();
    for (label, volume) in [
        (
            "shape",
            Volume {
                shape: Shape::Sphere,
                ..d
            },
        ),
        (
            "center",
            Volume {
                center: [0.1, 0., 0.],
                ..d
            },
        ),
        (
            "rotation",
            Volume {
                rotation: [0., 5., 0.],
                ..d
            },
        ),
        (
            "size",
            Volume {
                size: [1., 2., 1.],
                ..d
            },
        ),
        ("falloff", Volume { falloff: 0.25, ..d }),
    ] {
        check(
            &format!("形({label})は ShapeGradient 専用"),
            t,
            false,
            &|s| s.volume = volume,
        );
        check(
            &format!("ShapeGradient の形({label})"),
            Kind::ShapeGradient,
            true,
            &|s| s.volume = volume,
        );
    }
    check("ランプは ShapeGradient 専用", t, false, &|s| {
        s.ramp = Some(Ramp::default())
    });
    check(
        "ShapeGradient のランプ",
        Kind::ShapeGradient,
        true,
        &|s| s.ramp = Some(Ramp::default()),
    );
    // ID 色
    check("ID 色は IdColor 専用", t, false, &|s| {
        s.id_colors = vec![1]
    });
    check("許容差は IdColor 専用", t, false, &|s| {
        s.id_tolerance = 9
    });
    let id = Kind::IdColor;
    check("ID 色 32", id, true, &|s| s.id_colors = (0..32).collect());
    check("ID 色 33", id, false, &|s| s.id_colors = (0..33).collect());
    check("ID 色 0xffffff", id, true, &|s| {
        s.id_colors = vec![0xffffff]
    });
    check("許容差 0", id, true, &|s| s.id_tolerance = 0);
    check("許容差 255", id, true, &|s| s.id_tolerance = 255);
    // ピン
    check("ピン", t, true, &|s| {
        s.pins.insert(MapKind::Thickness, KEY.into());
        s.pins.insert(MapKind::Position, KEY.into());
    });
    check("ピンが大文字", t, false, &|s| {
        s.pins.insert(MapKind::Thickness, "A".repeat(64));
    });
    check("ピンが短い", t, false, &|s| {
        s.pins.insert(MapKind::Thickness, "a".repeat(63));
    });
    check("読まないマップのピン", t, false, &|s| {
        s.pins.insert(MapKind::Curvature, KEY.into());
    });
}
#[test]
fn plan_budget_is_exact_and_regions_match_the_full_evaluation() {
    use anchor::*;
    let layers = vec![Layer::new(Content::Fill(Rgba8::new(1, 2, 3, 255)))];
    let p = Plan::new(&layers, 0, (3, 2), ChannelKind::Color).unwrap();
    let options = |budget_bytes| Options {
        budget_bytes,
        cancel: None,
    };
    let full = Rect::new(0, 0, 3, 2);
    assert_eq!(
        p.evaluate(full, &options(23)),
        Err(Error::Budget {
            needed: 24,
            budget: 23
        })
    );
    assert_eq!(p.evaluate(full, &options(24)).unwrap().len(), 24);
    assert_eq!(
        p.evaluate(Rect::new(2, 1, 1, 1), &options(4))
            .unwrap()
            .len(),
        4
    );
    for r in [
        Rect::new(1, 0, 3, 2),
        Rect::new(0, 1, 3, 2),
        Rect::new(0, 0, 0, 2),
        Rect::new(0, 0, 3, 0),
        Rect::new(u32::MAX, 0, 1, 1),
    ] {
        assert!(invalid(p.evaluate(r, &options(1 << 20))), "{r:?}");
    }
    // 領域を切っても、全面を評価した同じ画素になる（部分スタックの全 48 事例）。
    for v in 0..48 {
        let whole = anchor_scene(v);
        for region in [
            Rect::new(5, 3, 17, 11),
            Rect::new(40, 28, 1, 1),
            Rect::new(0, 0, 1, 29),
            Rect::new(0, 0, 41, 1),
        ] {
            let part = anchor_scene_region(v, region);
            let w = region.width as usize;
            assert_eq!(part.len(), w * region.height as usize * 4);
            for row in 0..region.height as usize {
                let at = ((region.y as usize + row) * 41 + region.x as usize) * 4;
                assert_eq!(
                    &part[row * w * 4..(row + 1) * w * 4],
                    &whole[at..at + w * 4],
                    "anchor-{v} {region:?} 行 {row}"
                );
            }
        }
    }
}

// ---- Anchor の参照・ランプ・フィルターとの契約 ----
#[test]
fn settings_hold_the_anchor_reference_and_check_it_like_csharp() {
    use anchor::*;
    let mut s = Settings::new(Kind::Anchor);
    assert_eq!(
        s.anchor,
        Reference {
            id: 0,
            channel: Channel::Height,
            read: ReadMode::Value
        }
    );
    assert!(s.validate().is_ok());
    // 参照は設定の一部として比較される（保存・Undo の「変わったか」の判定に使える）。
    let user = Channel::from_index(Channel::STANDARD_COUNT).unwrap();
    s.anchor = Reference {
        id: 0x1234_5678_9abc_def0_1234_5678_9abc_def0,
        channel: user,
        read: ReadMode::Coverage,
    };
    assert!(s.validate().is_ok());
    let mut t = s.clone();
    assert_eq!(s, t);
    for edit in [
        (|t: &mut Settings| t.anchor.id += 1) as fn(&mut Settings),
        |t| t.anchor.channel = Channel::Color,
        |t| t.anchor.read = ReadMode::Value,
    ] {
        t = s.clone();
        edit(&mut t);
        assert_ne!(s, t);
        assert!(t.validate().is_ok());
    }
    // Normal チャンネルは Anchor でも読めない（ユーザーチャンネルの種類が Normal の場合は Plan::new が断る）。
    s.anchor.channel = Channel::Normal;
    assert!(invalid(s.validate()));
    // Anchor 以外の種類は既定値のまま。1 つでも違えば断る。
    for kind in KINDS.into_iter().filter(|k| *k != Kind::Anchor) {
        let s = Settings::new(kind);
        assert_eq!(
            s.anchor,
            Reference {
                id: 0,
                channel: Channel::Color,
                read: ReadMode::Value
            }
        );
        assert!(s.validate().is_ok());
        for edit in [
            (|s: &mut Settings| s.anchor.id = 1) as fn(&mut Settings),
            |s| s.anchor.channel = Channel::Height,
            |s| s.anchor.read = ReadMode::Coverage,
        ] {
            let mut s = s.clone();
            edit(&mut s);
            assert!(invalid(s.validate()), "{kind:?}");
        }
    }
    // 読み方はチャンネルの種類で決まる。
    let by = |read, kind| {
        Reference {
            id: 1,
            channel: Channel::Height,
            read,
        }
        .read_for(kind)
    };
    assert_eq!(by(ReadMode::Value, ChannelKind::Color), Ok(Read::Color));
    assert_eq!(by(ReadMode::Value, ChannelKind::Scalar), Ok(Read::Scalar));
    assert_eq!(
        by(ReadMode::Coverage, ChannelKind::Color),
        Ok(Read::Coverage)
    );
    assert_eq!(
        by(ReadMode::Coverage, ChannelKind::Scalar),
        Ok(Read::Coverage)
    );
    assert!(invalid(by(ReadMode::Value, ChannelKind::Normal)));
    assert!(invalid(by(ReadMode::Coverage, ChannelKind::Normal)));
    // 参照先の解決は anchor::resolve と同じ（未選択・欠損・同じレイヤー／上のレイヤー）。
    let points = [Point {
        id: 7,
        host: 0,
        placement: Placement::Layer,
    }];
    let r = |id| Reference {
        id,
        channel: Channel::Height,
        read: ReadMode::Value,
    };
    assert_eq!(r(7).resolve(&points, 1, 2), Ok(points[0]));
    assert_eq!(r(7).resolve(&points, 0, 2), Err(Issue::NotBelow));
    assert_eq!(r(0).resolve(&points, 1, 2), Err(Issue::NotChosen));
    assert_eq!(r(8).resolve(&points, 1, 2), Err(Issue::Missing));
}
#[test]
fn ramp_value_curve_is_the_shared_curve_type_and_evaluates_through_it() {
    use yolu_core::curve::Curve;
    let a = ramp();
    // ランプの値のカーブは共通の Curve。点も評価も Ramp の窓口と同じ
    let c: &Curve = a.value_curve();
    assert_eq!(c.points(), a.curve());
    for i in 0..=2000 {
        let x = i as f64 / 2000.;
        assert_eq!(c.value(x).unwrap(), a.curve_value(x).unwrap(), "{x}");
    }
    // カーブだけを差し替えると、色・不透明度はそのまま、評価はカーブを通る
    let softer = Curve::new(vec![
        CurvePoint { x: 0., y: 0. },
        CurvePoint { x: 0.3, y: 0.7 },
        CurvePoint { x: 1., y: 1. },
    ])
    .unwrap();
    let b = a.with_value_curve(softer.clone());
    assert_eq!((b.colors(), b.opacities()), (a.colors(), a.opacities()));
    assert_eq!(b.value_curve(), &softer);
    assert_eq!(b.curve_value(0.3).unwrap(), softer.value(0.3).unwrap());
    assert_eq!(
        b.evaluate(0.3, false).unwrap(),
        a.sample_stops(softer.value(0.3).unwrap(), false).unwrap()
    );
    // 作り直しても同じ（Ramp::new の検査は Curve::new と同じ）
    let again = Ramp::new(
        b.colors().to_vec(),
        b.opacities().to_vec(),
        Some(b.curve().to_vec()),
    )
    .unwrap();
    assert_eq!(again, b);
    // 履歴の大きさは点の数ぶん（4 点から 3 点で 24 バイト小さい）
    assert_eq!(a.curve().len(), 4);
    assert_eq!(a.byte_size() - b.byte_size(), 24);
}
#[test]
fn ramp_normalizes_color_stop_alpha_like_csharp_gradient_stop() {
    // C# の GradientStop は色の α を 255 にそろえる。そろえた形が == で等しい。
    let a = ramp();
    assert!(a.colors().iter().all(|s| s.color.a == 255));
    let mut stops = a.colors().to_vec();
    for (s, original) in stops.iter_mut().zip(a.colors()) {
        s.color.a = 7;
        assert_eq!(
            (s.color.r, s.color.g, s.color.b),
            (original.color.r, original.color.g, original.color.b)
        );
    }
    let b = Ramp::new(stops, a.opacities().to_vec(), Some(a.curve().to_vec())).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.colors(), b.colors());
    // RGB と評価の結果は変わらない（α は評価に使わない）。
    for i in 0..=1000 {
        for scalar in [false, true] {
            assert_eq!(
                a.evaluate(i as f64 / 1000., scalar).unwrap(),
                b.evaluate(i as f64 / 1000., scalar).unwrap()
            );
        }
    }
    // プリセットも、α の違う描画色から同じランプになる。
    for preset in [
        Preset::BlackWhite,
        Preset::WhiteBlack,
        Preset::ForegroundBackground,
        Preset::ForegroundTransparent,
        Preset::WarmCool,
    ] {
        assert_eq!(
            Ramp::preset(preset, Rgba8::new(1, 2, 3, 7), Rgba8::new(4, 5, 6, 0)),
            Ramp::preset(preset, Rgba8::new(1, 2, 3, 255), Rgba8::new(4, 5, 6, 255)),
            "{preset:?}"
        );
    }
    assert_eq!(Ramp::default().colors()[0].color.a, 255);
}
#[test]
fn generated_scalars_are_finite_unit_values_and_ramps_give_mapped() {
    // rsfilter の GeneratorInput の契約: スカラーは有限の 0..1。ランプがあれば Mapped、なければ Scalar。
    for kind in KINDS {
        for v in 0..12 {
            let has_ramp = settings(kind, v).ramp.is_some();
            let (mut seen, mut skipped) = (0, 0);
            with_bound(kind, v, 41, 29, |b| {
                for y in 0..29 {
                    for x in 0..41 {
                        match b.sample(x, y, false) {
                            Some(Generated::Scalar(s)) => {
                                assert!(!has_ramp);
                                assert!(
                                    s.is_finite() && (0. ..=1.).contains(&s),
                                    "{kind:?} v{v} ({x},{y}) {s}"
                                );
                                seen += 1;
                            }
                            Some(Generated::Mapped(_)) => {
                                assert!(has_ramp);
                                seen += 1;
                            }
                            None => skipped += 1,
                        }
                    }
                }
            });
            assert!(seen > 0, "{kind:?} v{v}: 値を返した画素が無い");
            assert!(skipped + seen == 41 * 29);
        }
    }
}

/// C# の AnchorTests.ChainsOfAnchorsAreFollowed: Anchor が別の Anchor の読むレイヤーを読む連鎖。L1（描いた Height）の Anchor 1 を、
/// L2（Height 10 の塗りつぶし）のフィルターが Add で読み、L2 の Anchor 2 を L3 のフィルターが読み、…と 4 段つなぎ、最後の
/// Anchor をプローブレイヤーのマスクが読む。どの段の値も「そのレイヤーまでの合成」を読んだ値で、L1 に 1 画素描くと、どの段の出力にも
/// プローブのマスクにも、その 1 画素だけが（段ごとに 10 ずつ足されて）現れる。
#[test]
fn chains_of_anchors_are_followed() {
    use anchor::*;
    const AW: u32 = 8;
    const AH: u32 = 6;
    const LINKS: usize = 4;
    let grey = |v: u8| Rgba8::new(v, v, v, 255);
    let solid = |v: u8| -> Vec<u8> { (0..AW * AH).flat_map(|_| grey(v).to_array()).collect() };
    // レイヤーの番号（下から）: 0 下地、1 L1、2.. 連鎖のレイヤー、最後がプローブ。Anchor の ID は置かれたレイヤーの番号 + 1。
    let points: Vec<Point> = (1..=LINKS + 1)
        .map(|host| Point {
            id: host as u128 + 1,
            host,
            placement: Placement::Layer,
        })
        .collect();
    validate_points(&points, LINKS + 3).unwrap();
    let run = |painted: &[(u32, u32)]| -> (Vec<Vec<u8>>, Vec<u8>) {
        let mut l1 = vec![0u8; (AW * AH * 4) as usize];
        for (x, y) in painted {
            l1[((y * AW + x) * 4) as usize..][..4].copy_from_slice(&grey(128).to_array());
        }
        let l1_image = Image::new(&l1, AW, AH).unwrap();
        let fill = solid(10);
        let fill_image = Image::new(&fill, AW, AH).unwrap();
        let reading = |id: u128, blend: Blend| {
            let mut s = Settings::new(Kind::Anchor);
            s.blend = blend;
            s.anchor = Reference {
                id,
                channel: Channel::Height,
                read: ReadMode::Value,
            };
            s
        };
        let mut outputs: Vec<Vec<u8>> = Vec::new();
        // 連鎖のレイヤー k（2 から）は、その下の Anchor（レイヤー k − 1 の Anchor）を Add で読む
        for k in 2..2 + LINKS {
            let next = {
                let images: Vec<Image> = outputs
                    .iter()
                    .map(|o| Image::new(o, AW, AH).unwrap())
                    .collect();
                let mut layers = vec![
                    Layer::new(Content::Fill(grey(40))),
                    Layer::new(Content::Pixels(&l1_image)),
                ];
                layers.extend(images.iter().map(|i| Layer::new(Content::Pixels(i))));
                assert_eq!(layers.len(), k, "下のレイヤーだけで Anchor の面を作る");
                let host = k - 1;
                let plan = Plan::new(&layers, host, (AW, AH), ChannelKind::Scalar).unwrap();
                let sample = LayerSample {
                    source: &plan,
                    read: Read::Scalar,
                };
                let reader = reading(host as u128 + 1, Blend::Add);
                assert_eq!(
                    reader.anchor.resolve(&points, k, LINKS + 3),
                    Ok(points[host - 1])
                );
                let bound =
                    BoundGenerator::bind(&reader, &[], None, (AW, AH), Ok(&sample)).unwrap();
                assert!(bound.inactive().is_none(), "連鎖の段 {k} は有効");
                let out = evaluate(
                    &fill_image,
                    &bound,
                    Rect::new(0, 0, AW, AH),
                    Target::Scalar,
                    1.0,
                    &Options::default(),
                )
                .unwrap();
                out.pixels
            };
            outputs.push(next);
        }
        // プローブ: マスクが最後の連鎖のレイヤーの Anchor を読む（置換）
        let images: Vec<Image> = outputs
            .iter()
            .map(|o| Image::new(o, AW, AH).unwrap())
            .collect();
        let mut layers = vec![
            Layer::new(Content::Fill(grey(40))),
            Layer::new(Content::Pixels(&l1_image)),
        ];
        layers.extend(images.iter().map(|i| Layer::new(Content::Pixels(i))));
        let host = layers.len() - 1;
        let plan = Plan::new(&layers, host, (AW, AH), ChannelKind::Scalar).unwrap();
        let sample = LayerSample {
            source: &plan,
            read: Read::Scalar,
        };
        let probe = reading(host as u128 + 1, Blend::Replace);
        assert_eq!(
            probe.anchor.resolve(&points, LINKS + 2, LINKS + 3),
            Ok(points[host - 1])
        );
        let bound = BoundGenerator::bind(&probe, &[], None, (AW, AH), Ok(&sample)).unwrap();
        let empty = vec![0u8; (AW * AH * 4) as usize];
        let mask = evaluate(
            &Image::new(&empty, AW, AH).unwrap(),
            &bound,
            Rect::new(0, 0, AW, AH),
            Target::Mask,
            1.0,
            &Options::default(),
        )
        .unwrap();
        (outputs, mask.pixels)
    };
    let (plain, plain_mask) = run(&[]);
    let (painted, painted_mask) = run(&[(3, 2)]);
    for (k, (a, b)) in plain.iter().zip(&painted).enumerate() {
        for i in 0..(AW * AH) as usize {
            let (x, y) = (i as u32 % AW, i as u32 / AW);
            // 段 k の出力は、下地 40（描いた画素は 128）に 10 を k + 1 回足した値
            let base = if (x, y) == (3, 2) { 128 } else { 40 };
            assert_eq!(b[i * 4], base + 10 * (k as u8 + 1), "段 {k} ({x},{y})");
            assert_eq!(a[i * 4], 40 + 10 * (k as u8 + 1), "段 {k} ({x},{y}) 描く前");
            assert_eq!(a[i * 4 + 3], 255);
        }
        let changed: Vec<usize> = (0..(AW * AH) as usize)
            .filter(|i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4])
            .collect();
        assert_eq!(
            changed,
            vec![(2 * AW + 3) as usize],
            "段 {k} は 1 画素だけ変わる"
        );
    }
    // プローブのマスクの隠す量は 255 − 最後の段の値。描いた画素は 4 つの Anchor を通って現れる
    for i in 0..(AW * AH) as usize {
        let painted_here = i == (2 * AW + 3) as usize;
        let last = if painted_here { 128 } else { 40 } + 10 * LINKS as u8;
        assert_eq!(painted_mask[i * 4 + 3], 255 - last, "マスク {i}");
        assert_eq!(
            plain_mask[i * 4 + 3],
            255 - (40 + 10 * LINKS as u8),
            "マスク {i} 描く前"
        );
        assert_eq!(
            &painted_mask[i * 4..i * 4 + 3],
            &[0, 0, 0],
            "マスクは RGB を持たない"
        );
    }
    assert_ne!(
        painted_mask[(2 * AW + 3) as usize * 4 + 3],
        painted_mask[3],
        "描いた画素のマスクだけが違う（4 つの Anchor を通して）"
    );
}
