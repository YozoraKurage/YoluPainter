//! 0.5.0 の Generator の種類（模様 66・光 68・マスクの組み立て 69）: 式どおりの値、行の評価と 1 画素ずつの値が同じ、マップが無いときの
//! 断り（使うマップだけ）、種類ごとの欄の検査、領域の切り方で同じバイト。
use yolu_core::generator::*;
use yolu_core::Rect;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// 決まった値のマップ（1 成分は 0〜65535 の斜めの並び、3 成分は法線らしい向きのまわり）。被覆は 13 画素に 1 つ 0。
struct Maps {
    data: Vec<(MapKind, Vec<u16>, Vec<u8>)>,
    w: u32,
    h: u32,
}
impl Maps {
    fn new(kinds: &[MapKind], w: u32, h: u32) -> Self {
        let data = kinds
            .iter()
            .map(|&k| {
                let mut data = Vec::new();
                let mut cover = Vec::new();
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..k.channels() {
                            let v = (u64::from(x) * 2311
                                + u64::from(y) * 4129
                                + c as u64 * 9001
                                + k as u64 * 613)
                                % 65536;
                            data.push(v as u16);
                        }
                        cover.push(u8::from(!(x + 2 * y).is_multiple_of(13)));
                    }
                }
                (k, data, cover)
            })
            .collect();
        Self { data, w, h }
    }
    fn maps(&self) -> Vec<Map<'_>> {
        self.data
            .iter()
            .map(|(k, d, c)| Map {
                kind: *k,
                width: self.w,
                height: self.h,
                data: d,
                coverage: c,
                bounds_min: [-1., -2., -3.],
                bounds_max: [2., 3., 1.],
                condition_key: KEY,
                state: MapState::Current,
            })
            .collect()
    }
    fn at(&self, k: MapKind, x: u32, y: u32) -> Option<Vec<f64>> {
        let (_, d, c) = self.data.iter().find(|(m, _, _)| *m == k)?;
        let i = (y * self.w + x) as usize;
        (c[i] != 0).then(|| {
            (0..k.channels())
                .map(|ch| f64::from(d[i * k.channels() + ch]))
                .collect()
        })
    }
}

fn no_anchor() -> Result<&'static dyn anchor::ValueSource, anchor::Issue> {
    Err(anchor::Issue::NotChosen)
}

/// 種類ごとの設定の変種。
fn variants(kind: Kind) -> Vec<Settings> {
    let mut out = Vec::new();
    for v in 0..6usize {
        let mut s = Settings::new(kind);
        s.low = [0., 0.1, 0.25][v % 3];
        s.high = [1., 0.9, 0.8][v % 3];
        s.invert = v % 2 == 1;
        s.blend = [Blend::Replace, Blend::Multiply, Blend::Screen][v % 3];
        match kind {
            Kind::Pattern => {
                s.pattern = Pattern {
                    shape: PatternShape::ALL[v % 5],
                    scale: [8., 3.5, 17.][v % 3],
                    angle: [0., 33., 270.][v % 3],
                    width: [0.5, 0.2, 0.8][v % 3],
                    softness: [0., 0.4, 1.][v % 3],
                    offset: [[0., 0.], [0.25, 0.7], [1., 0.5]][v % 3],
                };
            }
            Kind::Light => {
                s.light = Light {
                    azimuth: [45., 0., 300.][v % 3],
                    elevation: [45., 90., 5.][v % 3],
                    softness: [0.2, 0., 1.][v % 3],
                    ambient: [0., 0.3, 1.][v % 3],
                };
            }
            _ => {
                s.softness = [0., 0.5, 1.][v % 3];
                s.mask_builder.combine = MaskCombine::ALL[v % 3];
                for (k, input) in s.mask_builder.inputs.iter_mut().enumerate() {
                    *input = MaskInput {
                        weight: [1., 0.6, 0., 0.3][(k + v) % 4],
                        level: [0.5, 0.2, 0.8][(k + v) % 3],
                        contrast: [0., 0.5, 1.][(k * 2 + v) % 3],
                        invert: (k + v) % 2 == 1,
                    };
                }
            }
        }
        s.validate().unwrap();
        out.push(s);
    }
    out
}

/// 基底の値からレベル・減衰・反転を当てた値（`BoundGenerator::value` の後半と同じ式）。
fn levelled(s: &Settings, base: f64) -> f64 {
    let mut t = ((base - s.low) / (s.high - s.low)).clamp(0., 1.);
    if s.softness > 0. {
        t += s.softness * (t * t * (3. - 2. * t) - t);
    }
    if s.invert {
        1. - t
    } else {
        t
    }
}

const NEW_KINDS: [Kind; 3] = [Kind::Pattern, Kind::Light, Kind::MaskBuilder];

#[test]
fn each_kind_follows_its_formula() {
    let (w, h) = (31u32, 23u32);
    for kind in NEW_KINDS {
        for s in variants(kind) {
            let owned = Maps::new(&s.candidate_maps(), w, h);
            let maps = owned.maps();
            let b = BoundGenerator::bind(&s, &maps, None, (w, h), no_anchor()).unwrap();
            assert!(b.inactive().is_none(), "{kind:?}");
            for y in 0..h {
                for x in 0..w {
                    let base = match kind {
                        Kind::Pattern => Some(s.pattern.value(
                            (f64::from(x) + 0.5) / f64::from(w),
                            (f64::from(y) + 0.5) / f64::from(h),
                        )),
                        Kind::Light => owned
                            .at(MapKind::WorldNormal, x, y)
                            .and_then(|n| s.light.value([n[0], n[1], n[2]], s.light.direction())),
                        _ => s.mask_builder.value(|k| {
                            let v = owned.at(k, x, y)?;
                            Some(if k == MapKind::Position {
                                v[1] / 65535.
                            } else {
                                v[0] / 65535.
                            })
                        }),
                    };
                    let want = base.map(|b| levelled(&s, b));
                    let got = b.value(x, y);
                    assert_eq!(
                        got.map(f64::to_bits),
                        want.map(f64::to_bits),
                        "{kind:?} ({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn rows_equal_single_pixels_and_regions_equal_the_whole() {
    let (w, h) = (37u32, 29u32);
    let mut source = Vec::new();
    for i in 0..w * h {
        source.extend_from_slice(&[(i * 7) as u8, (i * 13) as u8, (i * 3) as u8, (i * 11) as u8]);
    }
    let image = Image::new(&source, w, h).unwrap();
    for kind in NEW_KINDS {
        for s in variants(kind) {
            let owned = Maps::new(&s.candidate_maps(), w, h);
            let maps = owned.maps();
            let b = BoundGenerator::bind(&s, &maps, None, (w, h), no_anchor()).unwrap();
            for scalar in [false, true] {
                for y in [0, 5, h - 1] {
                    let mut row = vec![None; w as usize];
                    b.sample_row(0, y, scalar, &mut row);
                    for (x, got) in row.into_iter().enumerate() {
                        assert_eq!(got, b.sample(x as u32, y, scalar), "{kind:?} ({x}, {y})");
                    }
                }
            }
            for target in [Target::Color, Target::Scalar, Target::Mask] {
                let all = evaluate(
                    &image,
                    &b,
                    Rect::new(0, 0, w, h),
                    target,
                    0.8,
                    &Options::default(),
                )
                .unwrap()
                .pixels;
                let r = Rect::new(5, 7, 19, 11);
                let part = evaluate(&image, &b, r, target, 0.8, &Options::default())
                    .unwrap()
                    .pixels;
                for y in 0..r.height {
                    let a = (((r.y + y) * w + r.x) * 4) as usize;
                    let p = (y * r.width * 4) as usize;
                    assert_eq!(
                        &all[a..a + (r.width * 4) as usize],
                        &part[p..p + (r.width * 4) as usize],
                        "{kind:?} {target:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn only_the_maps_in_use_are_required() {
    let (w, h) = (8u32, 8u32);
    // 模様はマップを読まない
    let pattern = Settings::new(Kind::Pattern);
    assert!(pattern.used_maps().is_empty() && pattern.candidate_maps().is_empty());
    let b = BoundGenerator::bind(&pattern, &[], None, (w, h), no_anchor()).unwrap();
    assert!(b.inactive().is_none());
    // 光はワールドの法線が無ければ入力のまま
    let light = Settings::new(Kind::Light);
    let b = BoundGenerator::bind(&light, &[], None, (w, h), no_anchor()).unwrap();
    assert_eq!(
        b.inactive(),
        Some(&Inactive::MissingMap(MapKind::WorldNormal))
    );
    // マスクの組み立ては、重みが 0 でないマップだけ
    let mut m = Settings::new(Kind::MaskBuilder);
    let owned = Maps::new(&[MapKind::Curvature], w, h);
    let maps = owned.maps();
    let b = BoundGenerator::bind(&m, &maps, None, (w, h), no_anchor()).unwrap();
    assert!(
        b.inactive().is_none(),
        "曲率だけ（既定）なら AO が無くても効く"
    );
    m.mask_builder.inputs[1].weight = 0.25;
    let b = BoundGenerator::bind(&m, &maps, None, (w, h), no_anchor()).unwrap();
    assert_eq!(
        b.inactive(),
        Some(&Inactive::MissingMap(MapKind::AmbientOcclusion))
    );
    assert_eq!(
        m.used_maps(),
        vec![MapKind::Curvature, MapKind::AmbientOcclusion]
    );
}

#[test]
fn kind_settings_stay_with_their_kind_and_ranges_are_checked() {
    // ほかの種類が模様・光・マスクの組み立ての欄を持つのは断る
    let mut s = Settings::new(Kind::EdgeWear);
    s.pattern.width = 0.3;
    assert!(s.validate().is_err());
    let mut s = Settings::new(Kind::Pattern);
    s.light.ambient = 0.5;
    assert!(s.validate().is_err());
    let mut s = Settings::new(Kind::Light);
    s.mask_builder.combine = MaskCombine::Max;
    assert!(s.validate().is_err());
    // 重ねるノイズ・共通の減衰（模様・光）は持たない
    let mut s = Settings::new(Kind::Pattern);
    s.noise_amount = 0.2;
    assert!(s.validate().is_err());
    let mut s = Settings::new(Kind::Light);
    s.softness = 0.5;
    assert!(s.validate().is_err());
    let mut s = Settings::new(Kind::MaskBuilder);
    s.softness = 0.5;
    assert!(s.validate().is_ok(), "マスクの組み立ては共通の減衰を使う");
    // 範囲
    let pattern = |f: &dyn Fn(&mut Pattern)| {
        let mut s = Settings::new(Kind::Pattern);
        f(&mut s.pattern);
        s.validate()
    };
    assert!(pattern(&|p| p.scale = 1.).is_ok());
    assert!(pattern(&|p| p.scale = 512.).is_ok());
    assert!(pattern(&|p| p.scale = 0.99).is_err());
    assert!(pattern(&|p| p.scale = 512.5).is_err());
    assert!(pattern(&|p| p.angle = 360.5).is_err());
    assert!(pattern(&|p| p.width = 1.01).is_err());
    assert!(pattern(&|p| p.offset = [0., f64::NAN]).is_err());
    let light = |f: &dyn Fn(&mut Light)| {
        let mut s = Settings::new(Kind::Light);
        f(&mut s.light);
        s.validate()
    };
    assert!(light(&|l| l.elevation = 90.).is_ok());
    assert!(light(&|l| l.elevation = 90.5).is_err());
    assert!(light(&|l| l.azimuth = -1.).is_err());
    assert!(light(&|l| l.ambient = 1.5).is_err());
    let mut s = Settings::new(Kind::MaskBuilder);
    s.mask_builder.inputs[3].contrast = 1.2;
    assert!(s.validate().is_err());
    // 番号
    for (kind, index) in [
        (Kind::Pattern, 66),
        (Kind::Light, 68),
        (Kind::MaskBuilder, 69),
    ] {
        assert_eq!(kind as u8, index);
        assert_eq!(Kind::from_index(i64::from(index)), Some(kind));
        assert!(kind.is_rust_only() && kind.is_050() && !kind.is_procedural());
    }
    assert_eq!(
        Kind::from_index(67),
        None,
        "67（UV の島ごとの値）はまだ無い"
    );
}
