//! Generator の段の行: 値を作る画素の範囲。

/// 透明な隙間がこれより短ければ、前後の範囲をつないで 1 回の行の評価にする（隙間の画素の値も作るが使わない）。
/// 行の評価の 1 回の呼びの手間（作業領域の用意・格子の覚えの引き当て）と、隙間の画素を作る手間の釣り合い。
const GAP: usize = 16;

/// 行の中で Generator の値を使う画素の範囲 `[始め, 終わり)` を、左から順に `f` へ渡す。マスクは行の全部。色・スカラーは
/// 不透明度が 0 でない画素の連なり（透明な画素は段が読まない。`GAP` より短い透明な隙間はつなぐ）。全部透明な行は 1 度も呼ばない。
pub(super) fn spans(row: &[u8], all: bool, mut f: impl FnMut(usize, usize)) {
    let n = row.len() / 4;
    if all {
        if n > 0 {
            f(0, n);
        }
        return;
    }
    let opaque = |i: usize| row[i * 4 + 3] != 0;
    let mut open: Option<(usize, usize)> = None;
    for i in (0..n).filter(|&i| opaque(i)) {
        open = match open {
            Some((start, end)) if i - end < GAP => Some((start, i + 1)),
            Some((start, end)) => {
                f(start, end);
                Some((i, i + 1))
            }
            None => Some((i, i + 1)),
        };
    }
    if let Some((start, end)) = open {
        f(start, end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(alpha: &[u8], all: bool) -> Vec<(usize, usize)> {
        let row: Vec<u8> = alpha.iter().flat_map(|a| [7, 8, 9, *a]).collect();
        let mut v = Vec::new();
        spans(&row, all, |s, e| v.push((s, e)));
        v
    }

    #[test]
    fn spans_cover_every_opaque_pixel_and_join_short_gaps() {
        assert_eq!(collect(&[], false), vec![]);
        assert_eq!(collect(&[0; 40], false), vec![]);
        assert_eq!(collect(&[0; 40], true), vec![(0, 40)]);
        let mut a = vec![0u8; 100];
        for i in [3, 4, 10, 40, 41, 42, 99] {
            a[i] = 1;
        }
        // 4 → 10 は 5 画素の隙間でつなぐ、10 → 40 は 29 画素で分ける、42 → 99 も分ける
        assert_eq!(collect(&a, false), vec![(3, 11), (40, 43), (99, 100)]);
        assert_eq!(collect(&a, true), vec![(0, 100)]);
        // 隙間がちょうど GAP − 1 ならつなぎ、GAP なら分ける
        let mut b = vec![0u8; 40];
        b[0] = 255;
        b[GAP] = 255;
        b[2 * GAP + 1] = 255;
        assert_eq!(
            collect(&b, false),
            vec![(0, GAP + 1), (2 * GAP + 1, 2 * GAP + 2)]
        );
    }

    // ───────── 行の評価が 1 画素ずつの評価と同じバイト ─────────

    use crate::filter::{
        evaluate, GeneratorBlend, GeneratorInput, Image, Options, Settings, Stage,
    };
    use crate::filter::{Generated, ValueType};
    use crate::generator::{
        self, anchor, BoundGenerator, ColorStop, GrungePreset, Kind, LuminanceCorrection, Map,
        MapKind, MapState, MixMode, NoiseSpace, OpacityStop, ProceduralSpace, Ramp, Shape,
    };
    use crate::math::simd;
    use crate::Rect;

    const KEY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const W: u32 = 45;
    const H: u32 = 23;

    /// 位置・向き・スカラー・ID のマップ（値がなめらかな所と飛ぶ所、被覆 0 の画素がある）と、入力の画像（透明な画素の連なり・短い隙間がある）。
    struct Scene {
        vector: Vec<u16>,
        normal: Vec<u16>,
        scalar: Vec<u16>,
        id: Vec<u16>,
        cover: Vec<u8>,
        input: Vec<u8>,
    }
    impl Scene {
        fn new() -> Self {
            let mut state = 0x2468_ace1u32;
            let mut next = move || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state >> 8
            };
            let mut s = Scene {
                vector: vec![],
                normal: vec![],
                scalar: vec![],
                id: vec![],
                cover: vec![],
                input: vec![],
            };
            for y in 0..H {
                for x in 0..W {
                    let jump = (x * 2 + y) % 11 == 0;
                    s.vector.extend([
                        (if jump {
                            next() % 65536
                        } else {
                            x * 1400 + y * 70
                        }) as u16,
                        (if jump {
                            next() % 65536
                        } else {
                            y * 2800 + x * 30
                        }) as u16,
                        ((x * 13 + y * 7) % 50 * 1300) as u16,
                    ]);
                    s.normal.extend([
                        (28000 + (x * 900) % 12000) as u16,
                        (38000 + (y * 700) % 12000) as u16,
                        (21000 + ((x + y) * 400) % 22000) as u16,
                    ]);
                    s.scalar.push(((x * 1700 + y * 900) % 65536) as u16);
                    // ID: 3 色の帯（どれかが選ぶ色に当たる）
                    let band = [[65535u16, 0, 0], [0, 65535, 0], [4112, 8224, 12336]]
                        [(x / 7 % 3) as usize];
                    s.id.extend(band);
                    s.cover.push(u8::from(!(x * 5 + y * 3).is_multiple_of(17)));
                    // 透明: 長い連なり（行の左の 1/3 の一部）と、短い隙間
                    let alpha = if (y % 4 == 1 && x < W / 3) || (x + y) % 9 == 0 {
                        0
                    } else if (x + y) % 5 == 0 {
                        255
                    } else {
                        (next() % 256) as u8
                    };
                    s.input.extend([
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                        (next() % 256) as u8,
                        alpha,
                    ]);
                }
            }
            // 全部透明な行を 1 つ
            for x in 0..W as usize {
                s.input[(3 * W as usize + x) * 4 + 3] = 0;
            }
            s
        }
        fn map<'a>(&'a self, kind: MapKind, data: &'a [u16]) -> Map<'a> {
            Map {
                kind,
                width: W,
                height: H,
                data,
                coverage: &self.cover,
                bounds_min: [-1., -2., -3.],
                bounds_max: [2., 3., 1.],
                condition_key: KEY,
                state: MapState::Current,
            }
        }
        fn maps(&self) -> Vec<Map<'_>> {
            vec![
                self.map(MapKind::Position, &self.vector),
                self.map(MapKind::WorldNormal, &self.normal),
                self.map(MapKind::BentNormal, &self.normal),
                self.map(MapKind::Curvature, &self.scalar),
                self.map(MapKind::AmbientOcclusion, &self.scalar),
                self.map(MapKind::Thickness, &self.scalar),
                self.map(MapKind::Id, &self.id),
            ]
        }
    }

    /// Anchor の読み先（値の無い画素がある）。
    struct AnchorValues;
    impl anchor::ValueSource for AnchorValues {
        fn dimensions(&self) -> (u32, u32) {
            (W, H)
        }
        fn value(&self, x: u32, y: u32) -> Option<f64> {
            (!(x + 2 * y).is_multiple_of(7)).then(|| f64::from((x * 3 + y * 5) % 31) / 30.)
        }
    }

    fn ramp(mode: MixMode) -> Ramp {
        let stop = |position, rgb: [u8; 3], midpoint| ColorStop {
            position,
            color: crate::Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            midpoint,
        };
        Ramp::new(
            vec![
                stop(0.05, [200, 30, 10], 0.3),
                stop(0.5, [20, 220, 60], 0.7),
                stop(0.95, [10, 40, 250], 0.5),
            ],
            vec![
                OpacityStop {
                    position: 0.,
                    opacity: 0.9,
                    midpoint: 0.4,
                },
                OpacityStop {
                    position: 1.,
                    opacity: 0.2,
                    midpoint: 0.5,
                },
            ],
            None,
        )
        .unwrap()
        .with_mixing(mode, LuminanceCorrection::Medium)
    }

    /// 全部の種類（重ねるノイズ・ランプの有無と混色・形・空間を変えて）。
    fn all_settings() -> Vec<(String, generator::Settings)> {
        use generator::Settings;
        let mut all = Vec::new();
        for (label, amount, space) in [
            ("none", 0., NoiseSpace::Model),
            ("model", 0.6, NoiseSpace::Model),
            ("uv", 0.6, NoiseSpace::Uv),
        ] {
            let mut s = Settings::new(Kind::EdgeWear);
            s.noise_amount = amount;
            s.noise_space = space;
            s.noise_scale = 0.08;
            all.push((format!("edge-{label}"), s));
        }
        let mut dirt = Settings::new(Kind::Dirt);
        dirt.balance = 0.3;
        all.push(("dirt".into(), dirt));
        let mut position = Settings::new(Kind::PositionGradient);
        position.axis = 0;
        position.invert = true;
        all.push(("position".into(), position));
        all.push(("thickness".into(), Settings::new(Kind::Thickness)));
        let mut direction = Settings::new(Kind::Direction);
        direction.use_bent_normal = true;
        direction.direction = [0.3, -0.7, 0.2];
        all.push(("direction".into(), direction));
        for (label, ramp) in [
            ("plain", None),
            ("standard", Some(ramp(MixMode::Standard))),
            ("linear", Some(ramp(MixMode::Linear))),
            ("perceptual", Some(ramp(MixMode::Perceptual))),
        ] {
            let mut s = Settings::new(Kind::ShapeGradient);
            s.ramp = ramp;
            s.volume.shape = Shape::Sphere;
            s.volume.rotation = [10., 20., 30.];
            s.noise_amount = 0.3;
            all.push((format!("shape-{label}"), s));
        }
        let mut id = Settings::new(Kind::IdColor);
        id.id_colors = vec![0xff0000, 0x102030];
        all.push(("id".into(), id));
        all.push(("anchor".into(), Settings::new(Kind::Anchor)));
        let mut noise = Settings::new(Kind::Noise);
        noise.procedural.seed = 3;
        noise.procedural.scale = 0.1;
        all.push(("noise".into(), noise));
        let mut grunge = Settings::grunge(GrungePreset::Cracks);
        grunge.procedural.space = ProceduralSpace::Triplanar;
        all.push(("grunge".into(), grunge));
        all
    }

    fn filter_blend(b: generator::Blend) -> GeneratorBlend {
        use generator::Blend as B;
        match b {
            B::Multiply => GeneratorBlend::Multiply,
            B::Replace => GeneratorBlend::Replace,
            B::Screen => GeneratorBlend::Screen,
            B::Max => GeneratorBlend::Max,
            B::Min => GeneratorBlend::Min,
            B::Add => GeneratorBlend::Add,
            B::Subtract => GeneratorBlend::Subtract,
        }
    }

    /// 段の番号で引く束縛済みの Generator。`rows` が偽なら `sample` だけ（既定の `sample_row` = 1 画素ずつ）。
    struct Bound<'a> {
        bound: Vec<BoundGenerator<'a>>,
        scalar: bool,
        rows: bool,
    }
    impl GeneratorInput for Bound<'_> {
        fn sample(&self, slot: u32, x: u32, y: u32) -> Option<Generated> {
            self.bound[slot as usize].sample(x, y, self.scalar)
        }
        fn sample_row(&self, slot: u32, x0: u32, y: u32, out: &mut [Option<Generated>]) {
            if self.rows {
                self.bound[slot as usize].sample_row(x0, y, self.scalar, out);
            } else {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = self.sample(slot, x0 + i as u32, y);
                }
            }
        }
    }

    /// 前の評価器の Generator の段（画素ごとに `sample` を呼び、透明な画素は読まない）をそのまま書いた参照。点の段だけのスタック用。
    fn per_pixel(
        input: &[u8],
        value_type: ValueType,
        stack: &[Stage],
        generators: &dyn GeneratorInput,
    ) -> Vec<u8> {
        let mask = value_type == ValueType::Mask;
        let mut out = input.to_vec();
        for (i, p) in out.chunks_exact_mut(4).enumerate() {
            let (x, y) = (i as u32 % W, i as u32 / W);
            if mask {
                let hide = p[3];
                p.copy_from_slice(&[hide, hide, hide, 255]);
            }
            for s in stack {
                let Settings::Generator { slot, blend } = s.settings else {
                    unreachable!()
                };
                if p[3] == 0 && !mask {
                    continue;
                }
                let Some(g) = generators.sample(slot, x, y) else {
                    continue;
                };
                super::super::pixels::generate(p, g, blend, s.strength, mask);
            }
            if mask {
                let hide = p[0];
                p.copy_from_slice(&[0, 0, 0, hide]);
            }
        }
        out
    }

    /// フィルターのスタックの Generator の段（行ごとに `sample_row` で読む）が、前の評価器（画素ごとに `sample`）と同じバイト。全部の種類・
    /// ランプの有無と混色・色・スカラー・マスク・強さで、この CPU が持つ道（スカラーを含む全部）で回す。段は 2 つ重ね（2 つ目は別の種類で
    /// 強さ 0.43）、ブロックを小さくして行の途中から始まる評価・全部透明な行・透明な画素の長い連なりと短い隙間も通る。
    #[test]
    fn row_sampling_gives_the_same_bytes_as_pixel_sampling() {
        let scene = Scene::new();
        let maps = scene.maps();
        let image = Image::new(&scene.input, W, H).unwrap();
        let anchor_values = AnchorValues;
        let all = all_settings();
        let mut compared = 0;
        for (i, (name, first)) in all.iter().enumerate() {
            let (_, second) = &all[(i + 5) % all.len()];
            fn bind<'a>(
                s: &'a generator::Settings,
                maps: &'a [Map<'a>],
                anchor: &'a dyn anchor::ValueSource,
            ) -> BoundGenerator<'a> {
                let frame = Some(generator::ModelFrame::default());
                BoundGenerator::bind(s, maps, frame, (W, H), Ok(anchor)).unwrap()
            }
            let stack = [
                Stage::new(Settings::Generator {
                    slot: 0,
                    blend: filter_blend(first.blend),
                }),
                Stage {
                    strength: 0.43,
                    ..Stage::new(Settings::Generator {
                        slot: 1,
                        blend: GeneratorBlend::Screen,
                    })
                },
            ];
            for value_type in [ValueType::Color, ValueType::Scalar, ValueType::Mask] {
                let input = |rows| Bound {
                    bound: vec![
                        bind(first, &maps, &anchor_values),
                        bind(second, &maps, &anchor_values),
                    ],
                    scalar: value_type != ValueType::Color,
                    rows,
                };
                let reference = per_pixel(&scene.input, value_type, &stack, &input(false));
                for rows in [false, true] {
                    let input = input(rows);
                    for level in simd::forced::supported() {
                        let got = simd::forced::with_level(level, || {
                            evaluate(
                                &image,
                                value_type,
                                &stack,
                                Rect::new(0, 0, W, H),
                                &Options {
                                    block_size: 16,
                                    generators: Some(&input),
                                    ..Options::default()
                                },
                            )
                            .unwrap()
                        });
                        assert!(
                            got == reference,
                            "{name} {value_type:?} {level:?} 行 {rows}: 画素ごとの評価と違う"
                        );
                        compared += 1;
                    }
                }
            }
        }
        assert!(compared >= all_settings().len() * 3 * 2, "{compared}");
    }
}
