//! 手続き型のノイズ・グランジ（Rust 版だけの Generator の種類 64・65）。C# に対応する実装は無いので、正解ファイルは無く、
//! 性質（決定性・継ぎ目・取消・予算・検査）と、実装を固定するハッシュ（回帰の固定であって外部の正解ではない）を試す。
#![allow(clippy::chunks_exact_to_as_chunks)]
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use yolu_core::{generator::*, Rect};

const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// 位置・向き・被覆の合成のシーン（整数だけで作る）。
struct Scene {
    w: u32,
    h: u32,
    position: Vec<u16>,
    normal: Vec<u16>,
    cover: Vec<u8>,
}
impl Scene {
    fn new(w: u32, h: u32) -> Self {
        let mut position = Vec::new();
        let mut normal = Vec::new();
        let mut cover = Vec::new();
        for y in 0..h {
            for x in 0..w {
                position.extend([
                    (x * 65535 / (w - 1)) as u16,
                    (y * 65535 / (h - 1)) as u16,
                    ((x * 7 + y * 13) % 50 * 1300) as u16,
                ]);
                // 左は +x、右下は +y、右上は +z に向く（境目で重みが混ざる）
                let n: [i32; 3] = if x < w / 3 {
                    [1000, 150, 100]
                } else if y < h / 2 {
                    [100, 1000, 200]
                } else {
                    [150, 100, 1000]
                };
                normal.extend(n.map(|v| (32767 + v * 32) as u16));
                cover.push(1);
            }
        }
        Self {
            w,
            h,
            position,
            normal,
            cover,
        }
    }
    fn maps(&self, normal: bool) -> Vec<Map<'_>> {
        let mut v = vec![Map {
            kind: MapKind::Position,
            width: self.w,
            height: self.h,
            data: &self.position,
            coverage: &self.cover,
            bounds_min: [-1., -2., -3.],
            bounds_max: [2., 3., 1.],
            condition_key: KEY,
            state: MapState::Current,
        }];
        if normal {
            v.push(Map {
                kind: MapKind::WorldNormal,
                data: &self.normal,
                ..v[0].clone()
            });
        }
        v
    }
}

fn eval(
    s: &Settings,
    maps: &[Map<'_>],
    (w, h): (u32, u32),
    region: Rect,
    options: &Options<'_>,
) -> Result<Output, Error> {
    let mut s = s.clone();
    s.blend = Blend::Replace;
    let white = vec![255u8; w as usize * h as usize * 4];
    let source = Image::new(&white, w, h)?;
    let bound = BoundGenerator::bind(&s, maps, None, (w, h), Err(anchor::Issue::NotChosen))?;
    evaluate(&source, &bound, region, Target::Color, 1., options)
}
fn full(s: &Settings, maps: &[Map<'_>], size: (u32, u32)) -> Vec<u8> {
    eval(
        s,
        maps,
        size,
        Rect::new(0, 0, size.0, size.1),
        &Options::default(),
    )
    .unwrap()
    .pixels
}
fn grey(pixels: &[u8]) -> Vec<f64> {
    pixels.chunks_exact(4).map(|p| p[0] as f64 / 255.).collect()
}

fn noise(basis: NoiseBasis, fractal: FractalMode) -> Settings {
    let mut s = Settings::new(Kind::Noise);
    s.procedural.basis = basis;
    s.procedural.fractal = fractal;
    s.procedural.seed = 7;
    s
}
fn all_settings() -> Vec<(String, Settings)> {
    let mut v = Vec::new();
    for basis in [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley] {
        for fractal in [
            FractalMode::Fbm,
            FractalMode::Ridged,
            FractalMode::Turbulence,
        ] {
            v.push((
                format!("noise-{basis:?}-{fractal:?}"),
                noise(basis, fractal),
            ));
        }
    }
    for out in [CellOutput::F2, CellOutput::F2MinusF1] {
        let mut s = noise(NoiseBasis::Worley, FractalMode::Fbm);
        s.procedural.cell_output = out;
        v.push((format!("noise-Worley-{out:?}"), s));
    }
    for preset in GrungePreset::ALL {
        let mut s = Settings::grunge(preset);
        s.procedural.seed = 7;
        v.push((format!("grunge-{}", preset.id()), s));
    }
    v
}
fn in_space(mut s: Settings, space: ProceduralSpace) -> Settings {
    s.procedural.space = space;
    s
}

#[test]
fn kinds_are_numbered_outside_the_csharp_range() {
    assert_eq!(Kind::Noise as u8, 64);
    assert_eq!(Kind::Grunge as u8, 65);
    assert_eq!(Kind::from_index(64), Some(Kind::Noise));
    assert_eq!(Kind::from_index(65), Some(Kind::Grunge));
    for i in (8..64).chain(66..300).chain([-1]) {
        assert_eq!(Kind::from_index(i), None, "{i}");
    }
    for i in 0..8 {
        assert!(!Kind::from_index(i).unwrap().is_procedural());
    }
}

#[test]
fn threads_and_regions_do_not_change_pixels() {
    let scene = Scene::new(53, 37);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let four = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let region = Rect::new(13, 5, 31, 17);
    for (name, base) in all_settings() {
        for space in [
            ProceduralSpace::Position,
            ProceduralSpace::Triplanar,
            ProceduralSpace::Uv,
        ] {
            let mut s = in_space(base.clone(), space);
            s.procedural.bleed = 0.4;
            s.procedural.rotation = [13., -27., 41.];
            let maps = scene.maps(true);
            let a = one.install(|| full(&s, &maps, (scene.w, scene.h)));
            let b = four.install(|| full(&s, &maps, (scene.w, scene.h)));
            assert_eq!(a, b, "{name} {space:?}");
            let part = four.install(|| {
                eval(&s, &maps, (scene.w, scene.h), region, &Options::default())
                    .unwrap()
                    .pixels
            });
            let mut crop = Vec::new();
            for y in region.y..region.y + region.height {
                let start = (y * scene.w + region.x) as usize * 4;
                crop.extend_from_slice(&a[start..start + region.width as usize * 4]);
            }
            assert_eq!(part, crop, "{name} {space:?} 領域の切り方で変わった");
        }
    }
}

#[test]
fn position_space_gives_the_same_value_at_the_same_place_whatever_the_uv() {
    // 2 つの UV の島が同じ位置を指す: 島ごとに別の座標だと継ぎ目に段が出る。位置で評価すれば同じ値になる
    let (w, h) = (40u32, 20u32);
    let mut position = Vec::new();
    for y in 0..h {
        for x in 0..w {
            // 左半分と右半分は同じ位置の並び（右は上下を反転して並べた別の島）
            let (px, py) = if x < w / 2 {
                (x, y)
            } else {
                (x - w / 2, h - 1 - y)
            };
            position.extend([
                (px * 65535 / (w / 2 - 1)) as u16,
                (py * 65535 / (h - 1)) as u16,
                ((px + py) * 900) as u16,
            ]);
        }
    }
    let cover = vec![1u8; (w * h) as usize];
    let map = Map {
        kind: MapKind::Position,
        width: w,
        height: h,
        data: &position,
        coverage: &cover,
        bounds_min: [0.; 3],
        bounds_max: [1., 1., 1.],
        condition_key: KEY,
        state: MapState::Current,
    };
    for (name, base) in all_settings() {
        if name.contains("fingerprints") || name.contains("weave") || name.contains("scratches") {
            continue; // 平面の模様は向きのマップがいる（下のトライプラナーの試験）
        }
        let s = in_space(base, ProceduralSpace::Position);
        let px = full(&s, std::slice::from_ref(&map), (w, h));
        let g = grey(&px);
        for y in 0..h {
            for x in 0..w / 2 {
                let a = g[(y * w + x) as usize];
                let b = g[((h - 1 - y) * w + x + w / 2) as usize];
                assert_eq!(a, b, "{name} ({x},{y})");
            }
        }
        assert!(
            g.iter().any(|v| (*v - g[0]).abs() > 0.05),
            "{name}: 一様な値になっている"
        );
    }
}

#[test]
fn triplanar_uses_the_normal_and_blends_across_the_border() {
    let scene = Scene::new(48, 32);
    let maps = scene.maps(true);
    let s = in_space(
        Settings::grunge(GrungePreset::Weave),
        ProceduralSpace::Triplanar,
    );
    assert_eq!(s.used_maps(), vec![MapKind::Position, MapKind::WorldNormal]);
    let px = full(&s, &maps, (scene.w, scene.h));
    assert!(px.chunks_exact(4).all(|p| p[3] == 255));
    // 向きのマップを変えると、混ぜ方が変わって値も変わる
    let mut flipped = Scene::new(48, 32);
    for c in flipped.normal.chunks_exact_mut(3) {
        c.swap(0, 2);
    }
    let other = full(&s, &flipped.maps(true), (scene.w, scene.h));
    assert_ne!(px, other);
    // 位置の空間でも、2D の模様のプリセットは自動でトライプラナー（向きのマップを読む）
    let auto = Settings::grunge(GrungePreset::Weave);
    assert_eq!(auto.procedural.space, ProceduralSpace::Position);
    assert_eq!(
        auto.used_maps(),
        vec![MapKind::Position, MapKind::WorldNormal]
    );
    assert_eq!(full(&auto, &maps, (scene.w, scene.h)), px);
    // 3D のノイズは位置だけ
    assert_eq!(
        Settings::new(Kind::Noise).used_maps(),
        vec![MapKind::Position]
    );
    assert_eq!(
        in_space(Settings::new(Kind::Noise), ProceduralSpace::Uv).used_maps(),
        vec![]
    );
}

/// 束縛して、入力を通していない（`inactive` が無い）ことを確かめ、UV に落とした理由を返す。
fn fallback_of(s: &Settings, maps: &[Map<'_>], size: (u32, u32)) -> Option<Inactive> {
    let b = BoundGenerator::bind(s, maps, None, size, Err(anchor::Issue::NotChosen)).unwrap();
    assert!(b.inactive().is_none());
    b.fallback().cloned()
}

#[test]
fn unusable_maps_fall_back_to_uv_with_a_reason_instead_of_passing_the_input() {
    let scene = Scene::new(24, 16);
    let size = (scene.w, scene.h);
    let missing = |k| Some(Inactive::MissingMap(k));
    for (name, base) in all_settings() {
        let uv = full(&in_space(base.clone(), ProceduralSpace::Uv), &[], size);
        let want_normal =
            name.contains("fingerprints") || name.contains("weave") || name.contains("scratches");
        let mut s = base.clone();
        // 無い
        assert_eq!(
            fallback_of(&s, &[], size),
            missing(MapKind::Position),
            "{name}"
        );
        assert_eq!(full(&s, &[], size), uv, "{name}: UV の評価と同じ");
        // 使える
        let maps = scene.maps(true);
        assert_eq!(fallback_of(&s, &maps, size), None, "{name}");
        assert_ne!(full(&s, &maps, size), uv, "{name}");
        // 古い・未検証
        for (state, why) in [
            (MapState::Stale, Inactive::StaleMap(MapKind::Position)),
            (
                MapState::Unverified,
                Inactive::UnverifiedMap(MapKind::Position),
            ),
        ] {
            let mut maps = scene.maps(true);
            maps[0].state = state;
            assert_eq!(fallback_of(&s, &maps, size), Some(why), "{name}");
            assert_eq!(full(&s, &maps, size), uv, "{name}");
        }
        // 大きさが違う
        let small = Scene::new(12, 8);
        let maps = small.maps(true);
        assert_eq!(
            fallback_of(&s, &maps, size),
            Some(Inactive::MapSize(MapKind::Position)),
            "{name}"
        );
        // ピンが違う
        let maps = scene.maps(true);
        s.pins.insert(MapKind::Position, "b".repeat(64));
        assert_eq!(
            fallback_of(&s, &maps, size),
            Some(Inactive::PinMismatch(MapKind::Position)),
            "{name}"
        );
        s.pins.insert(MapKind::Position, KEY.into());
        assert_eq!(fallback_of(&s, &maps, size), None, "{name}");
        s.pins.clear();
        // 境界箱が 0
        let mut maps = scene.maps(true);
        maps[0].bounds_max = maps[0].bounds_min;
        assert_eq!(
            fallback_of(&s, &maps, size),
            Some(Inactive::EmptyBounds),
            "{name}"
        );
        // 向きのマップだけ無い（トライプラナーが要る設定だけが落ちる）
        let maps = scene.maps(false);
        assert_eq!(
            fallback_of(&s, &maps, size),
            want_normal.then_some(Inactive::MissingMap(MapKind::WorldNormal)),
            "{name}"
        );
        // UV を選んでいればマップを見ない（理由も無い）
        let uv_settings = in_space(base, ProceduralSpace::Uv);
        assert_eq!(fallback_of(&uv_settings, &[], size), None, "{name}");
    }
}

#[test]
fn uncovered_texels_pass_the_input_through_in_position_space() {
    let mut scene = Scene::new(20, 20);
    for c in scene.cover.iter_mut().step_by(3) {
        *c = 0;
    }
    let s = Settings::new(Kind::Noise);
    let maps = scene.maps(false);
    let bound =
        BoundGenerator::bind(&s, &maps, None, (20, 20), Err(anchor::Issue::NotChosen)).unwrap();
    for i in 0..400u32 {
        let v = bound.value(i % 20, i / 20);
        assert_eq!(v.is_none(), i % 3 == 0, "{i}");
        if let Some(v) = v {
            assert!((0. ..=1.).contains(&v));
        }
    }
}

#[test]
fn rotation_turns_the_pattern_in_position_space_but_not_in_uv() {
    let scene = Scene::new(40, 30);
    let maps = scene.maps(true);
    let size = (40, 30);
    let a = noise(NoiseBasis::Perlin, FractalMode::Fbm);
    let mut b = a.clone();
    b.procedural.rotation = [0., 0., 90.];
    assert_ne!(full(&a, &maps, size), full(&b, &maps, size));
    let (ua, ub) = (
        in_space(a.clone(), ProceduralSpace::Uv),
        in_space(b, ProceduralSpace::Uv),
    );
    assert_eq!(
        full(&ua, &[], size),
        full(&ub, &[], size),
        "UV では回転しない（周期を保つ）"
    );
    // 360 度は 0 度と同じ（90 度で x と y が入れ替わることは、crate 内の `rotation_by_90_degrees_around_z_swaps_x_and_y` が座標で見る）
    let mut c = a.clone();
    c.procedural.rotation = [0., 0., 360.];
    assert_eq!(full(&a, &maps, size), full(&c, &maps, size));
}

#[test]
fn seed_scale_and_every_setting_change_the_pattern() {
    let size = (48, 32);
    let base = Settings::new(Kind::Noise);
    let reference = full(&in_space(base.clone(), ProceduralSpace::Uv), &[], size);
    let mut variants = Vec::new();
    let mut s = base.clone();
    s.procedural.seed = 1;
    variants.push(("seed", s));
    let mut s = base.clone();
    s.procedural.scale = 0.2;
    variants.push(("scale", s));
    let mut s = base.clone();
    s.procedural.octaves = 2;
    variants.push(("octaves", s));
    let mut s = base.clone();
    s.procedural.lacunarity = 3.;
    variants.push(("lacunarity", s));
    let mut s = base.clone();
    s.procedural.gain = 0.8;
    variants.push(("gain", s));
    let mut s = base.clone();
    s.procedural.bleed = 0.7;
    variants.push(("bleed", s));
    let mut s = base.clone();
    s.procedural.fractal = FractalMode::Ridged;
    variants.push(("fractal", s));
    let mut s = base.clone();
    s.procedural.basis = NoiseBasis::Value;
    variants.push(("basis", s));
    let mut s = base.clone();
    s.low = 0.3;
    variants.push(("low", s));
    let mut s = base.clone();
    s.invert = true;
    variants.push(("invert", s));
    for (what, s) in variants {
        let out = full(&in_space(s, ProceduralSpace::Uv), &[], size);
        assert_ne!(out, reference, "{what}");
    }
}

#[test]
fn every_kind_and_preset_makes_a_usable_pattern() {
    // 全部が一様・全面 1・全面 0 でないこと（式を壊したときに気づく）。値の幅と被覆の割合を見る
    for (name, s) in all_settings() {
        let g = grey(&preview(&s, 96, 96).unwrap());
        let mean = g.iter().sum::<f64>() / g.len() as f64;
        let hi = g.iter().filter(|v| **v > 0.5).count() as f64 / g.len() as f64;
        let (lo_v, hi_v) = g
            .iter()
            .fold((1f64, 0f64), |(a, b), v| (a.min(*v), b.max(*v)));
        eprintln!("{name}: mean {mean:.3} >0.5 {hi:.3} range {lo_v:.2}..{hi_v:.2}");
        assert!(hi_v - lo_v > 0.3, "{name}: 値の幅が足りない");
        assert!(
            (0.02..=0.98).contains(&mean),
            "{name}: 平均が偏りすぎている {mean}"
        );
    }
}

#[test]
fn preview_is_grey_opaque_and_refuses_other_kinds() {
    let s = Settings::grunge(GrungePreset::Rust);
    let px = preview(&s, 32, 24).unwrap();
    assert_eq!(px.len(), 32 * 24 * 4);
    assert!(px
        .chunks_exact(4)
        .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
    assert_eq!(px, preview(&s, 32, 24).unwrap());
    assert!(preview(&Settings::new(Kind::Dirt), 8, 8).is_err());
}

/// 見本の出力は `Options::default()` の予算までで、超える大きさは確保の前に断る（入力の白い画像も持たない）。
/// 以前は予算の検査より前に入力の白い画像（幅 × 高さ × 4 バイト）を確保していたので、大きな引数ではここで落ちた。
#[test]
fn preview_over_the_budget_is_refused_before_anything_is_allocated() {
    let s = Settings::grunge(GrungePreset::Rust);
    let budget = Options::default().budget_bytes;
    assert_eq!(budget, 256 * 1024 * 1024);
    // 予算を 1 画素分だけ超える（8192 × 8192 × 4 がちょうど予算）
    let needed = 8192u64 * 8193 * 4;
    assert_eq!(
        preview(&s, 8192, 8193).unwrap_err(),
        Error::Budget { needed, budget }
    );
    assert!(matches!(
        preview(&s, 65536, 65536),
        Err(Error::Budget { .. })
    ));
    // 幅 × 高さ × 4 が u64 に収まらない大きさは、断るだけで落ちない
    assert!(preview(&s, u32::MAX, u32::MAX).is_err());
    assert!(preview(&s, 0, 8).is_err() && preview(&s, 8, 0).is_err());
    // 予算の中は今までどおり
    assert_eq!(preview(&s, 64, 48).unwrap().len(), 64 * 48 * 4);
}

#[test]
fn uv_patterns_tile_at_every_size_and_scale() {
    // 端の画素が反対の端の隣と連続する（周期を巻いている）: 隣り合う画素の差と、端をまたぐ差が同じ程度。画素の差の平均は、細かい模様
    // （ほこり・しぼ・飛沫・布目）では内側の差がもともと大きく、巻き忘れを見逃し得る粗い確かめ。式の周期そのものは、crate 内の
    // `every_recipe_is_periodic_over_the_uv_counts_for_every_basis_and_preset` が全基底・全プリセットで直に見る。
    for (name, base) in all_settings() {
        for (w, h, scale) in [(64u32, 64u32, 0.25), (96, 40, 0.1), (40, 96, 0.5)] {
            let mut s = base.clone();
            s.procedural.scale = scale;
            s.procedural.bleed = 0.5;
            let g = grey(&preview(&s, w, h).unwrap());
            let at = |x: u32, y: u32| g[(y * w + x) as usize];
            let (mut inner_x, mut seam_x, mut inner_y, mut seam_y) = (0., 0., 0., 0.);
            for y in 0..h {
                for x in 0..w - 1 {
                    inner_x += (at(x + 1, y) - at(x, y)).abs();
                }
                seam_x += (at(0, y) - at(w - 1, y)).abs();
            }
            for x in 0..w {
                for y in 0..h - 1 {
                    inner_y += (at(x, y + 1) - at(x, y)).abs();
                }
                seam_y += (at(x, 0) - at(x, h - 1)).abs();
            }
            let (ix, sx) = (inner_x / ((w - 1) * h) as f64, seam_x / h as f64);
            let (iy, sy) = (inner_y / (w * (h - 1)) as f64, seam_y / w as f64);
            // 継ぎ目の段は、内側の隣り合う差の数倍にならない（巻いていなければ画素の差の平均は 0.3 ほどになる）
            assert!(
                sx <= ix * 4. + 0.02,
                "{name} {w}x{h} 横の継ぎ目 {sx} 内側 {ix}"
            );
            assert!(
                sy <= iy * 4. + 0.02,
                "{name} {w}x{h} 縦の継ぎ目 {sy} 内側 {iy}"
            );
        }
    }
}

#[test]
fn budget_and_cancel_are_honoured() {
    let s = Settings::grunge(GrungePreset::Cracks);
    let size = (64, 64);
    let tiny = Options {
        budget_bytes: 64 * 64 * 4 - 1,
        cancel: None,
    };
    assert!(matches!(
        eval(&s, &[], size, Rect::new(0, 0, 64, 64), &tiny),
        Err(Error::Budget { .. })
    ));
    let flag = AtomicBool::new(true);
    let cancelled = Options {
        budget_bytes: u64::MAX,
        cancel: Some(&flag),
    };
    assert_eq!(
        eval(&s, &[], size, Rect::new(0, 0, 64, 64), &cancelled).err(),
        Some(Error::Cancelled)
    );
    flag.store(false, Ordering::Relaxed);
    assert!(eval(&s, &[], size, Rect::new(0, 0, 64, 64), &cancelled).is_ok());
}

#[test]
fn settings_are_validated() {
    let ok = Settings::new(Kind::Noise);
    assert!(ok.validate().is_ok());
    for p in GrungePreset::ALL {
        assert!(Settings::grunge(p).validate().is_ok());
    }
    let mut bad: Vec<(&str, Settings)> = Vec::new();
    let mut tweak = |what, f: &dyn Fn(&mut Settings)| {
        let mut s = ok.clone();
        f(&mut s);
        bad.push((what, s));
    };
    tweak("octaves 0", &|s| s.procedural.octaves = 0);
    tweak("octaves 9", &|s| s.procedural.octaves = 9);
    tweak("lacunarity 0.5", &|s| s.procedural.lacunarity = 0.5);
    tweak("lacunarity 4.5", &|s| s.procedural.lacunarity = 4.5);
    tweak("lacunarity nan", &|s| s.procedural.lacunarity = f64::NAN);
    tweak("gain 1.5", &|s| s.procedural.gain = 1.5);
    tweak("scale 0", &|s| s.procedural.scale = 0.);
    tweak("scale 2", &|s| s.procedural.scale = 2.);
    tweak("scale inf", &|s| s.procedural.scale = f64::INFINITY);
    tweak("rotation 361", &|s| s.procedural.rotation[1] = 361.);
    tweak("rotation nan", &|s| s.procedural.rotation[2] = f64::NAN);
    tweak("bleed 2", &|s| s.procedural.bleed = 2.);
    tweak("blend width -1", &|s| s.procedural.blend_width = -1.);
    tweak("preset on noise", &|s| {
        s.procedural.preset = GrungePreset::Rust
    });
    tweak("cell output on value", &|s| {
        s.procedural.basis = NoiseBasis::Value;
        s.procedural.cell_output = CellOutput::F2;
    });
    tweak("overlay noise", &|s| s.noise_amount = 0.5);
    tweak("overlay seed", &|s| s.noise_seed = 3);
    tweak("overlay space", &|s| s.noise_space = NoiseSpace::Uv);
    tweak("levels", &|s| s.high = s.low);
    for (what, s) in &bad {
        assert!(s.validate().is_err(), "{what}");
        assert!(
            BoundGenerator::bind(s, &[], None, (4, 4), Err(anchor::Issue::NotChosen)).is_err(),
            "{what}"
        );
    }
    // Noise 専用の欄をグランジが持つ・手続き型の欄をほかの種類が持つ
    let mut g = Settings::grunge(GrungePreset::Dust);
    g.procedural.octaves = 3;
    assert!(g.validate().is_err());
    let mut g = Settings::grunge(GrungePreset::Dust);
    g.procedural.basis = NoiseBasis::Worley;
    assert!(g.validate().is_err());
    for kind in [Kind::EdgeWear, Kind::Dirt, Kind::Thickness, Kind::Anchor] {
        let mut s = Settings::new(kind);
        s.procedural.seed = 4;
        assert!(s.validate().is_err(), "{kind:?}");
    }
    // ピンは Position・WorldNormal だけ
    let mut s = ok.clone();
    s.pins.insert(MapKind::Curvature, KEY.into());
    assert!(s.validate().is_err());
    s.pins.clear();
    s.pins.insert(MapKind::WorldNormal, KEY.into());
    assert!(s.validate().is_ok());
}

#[test]
fn procedural_kinds_do_not_disturb_the_existing_kinds() {
    // 既存の種類の設定は手続き型の欄を既定のまま持つ（保存・比較が変わらない）
    for kind in [
        Kind::EdgeWear,
        Kind::Dirt,
        Kind::PositionGradient,
        Kind::Thickness,
        Kind::Direction,
        Kind::ShapeGradient,
        Kind::IdColor,
        Kind::Anchor,
    ] {
        let s = Settings::new(kind);
        assert_eq!(s.procedural, Procedural::default());
        assert_eq!(s.algorithm_version(), 1);
    }
    assert_eq!(Settings::new(Kind::Noise).algorithm_version(), 1);
    assert_eq!(Settings::new(Kind::Grunge).algorithm_version(), 1);
}

#[test]
fn levels_blend_and_strength_work_like_the_other_kinds() {
    let size = (32, 24);
    let mut s = Settings::new(Kind::Noise);
    s.procedural.space = ProceduralSpace::Uv;
    // 既定は 0..1 のそのまま: レベルを狭めると 0 と 1 に寄る
    let plain = grey(&full(&s, &[], size));
    s.low = 0.45;
    s.high = 0.55;
    let hard = grey(&full(&s, &[], size));
    let extreme = |g: &[f64]| g.iter().filter(|v| **v == 0. || **v == 1.).count();
    assert!(extreme(&hard) > extreme(&plain) + 100);
    s.invert = true;
    let inv = grey(&full(&s, &[], size));
    for (a, b) in hard.iter().zip(&inv) {
        assert!((a + b - 1.).abs() < 0.01);
    }
    // 乗算は入力を暗くし、強さ 0 は入力のまま
    let mut m = Settings::new(Kind::Noise);
    m.procedural.space = ProceduralSpace::Uv;
    m.blend = Blend::Multiply;
    let white = vec![255u8; 32 * 24 * 4];
    let source = Image::new(&white, 32, 24).unwrap();
    let bound = BoundGenerator::bind(&m, &[], None, size, Err(anchor::Issue::NotChosen)).unwrap();
    let out = evaluate(
        &source,
        &bound,
        Rect::new(0, 0, 32, 24),
        Target::Color,
        0.,
        &Options::default(),
    )
    .unwrap();
    assert_eq!(out.pixels, white);
    let out = evaluate(
        &source,
        &bound,
        Rect::new(0, 0, 32, 24),
        Target::Scalar,
        1.,
        &Options::default(),
    )
    .unwrap();
    assert!(out.pixels.chunks_exact(4).any(|p| p[0] < 250));
    // マスクの対象では、隠す量が A に入る
    let mut r = m.clone();
    r.blend = Blend::Replace;
    let bound = BoundGenerator::bind(&r, &[], None, size, Err(anchor::Issue::NotChosen)).unwrap();
    let out = evaluate(
        &source,
        &bound,
        Rect::new(0, 0, 32, 24),
        Target::Mask,
        1.,
        &Options::default(),
    )
    .unwrap();
    assert!(out
        .pixels
        .chunks_exact(4)
        .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
    assert!(out.pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
}

/// 実装を固定するハッシュ（回帰の固定。C# に対応する実装は無い）。更新するときは理由を VALIDATION に残す。
#[test]
fn pinned_hashes() {
    let scene = Scene::new(40, 28);
    let maps = scene.maps(true);
    let mut lines = Vec::new();
    for (name, base) in all_settings() {
        for space in [
            ProceduralSpace::Position,
            ProceduralSpace::Triplanar,
            ProceduralSpace::Uv,
        ] {
            let mut s = in_space(base.clone(), space);
            s.procedural.bleed = 0.25;
            let px = full(&s, &maps, (40, 28));
            lines.push(format!("{name}-{space:?} {}", hash(&px)));
        }
    }
    let table = include_str!("procedural-index.txt");
    if std::env::var("YOLU_PROCEDURAL_WRITE").is_ok() {
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/procedural-index.txt"),
            lines.join("\n") + "\n",
        )
        .unwrap();
        return;
    }
    let expected: Vec<&str> = table.lines().collect();
    assert_eq!(expected.len(), lines.len());
    for (e, l) in expected.iter().zip(&lines) {
        assert_eq!(e, l);
    }
}

/// 見た目を確かめるための書き出し（`YOLU_PROCEDURAL_SHEET=<ディレクトリ>`。PGM）。ふだんは何もしない。
#[test]
fn dump_sheet() {
    let Ok(dir) = std::env::var("YOLU_PROCEDURAL_SHEET") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for (name, s) in all_settings() {
        let size = 256u32;
        let px = preview(&s, size, size).unwrap();
        let mut out = format!("P5\n{size} {size}\n255\n").into_bytes();
        out.extend(px.chunks_exact(4).map(|p| p[0]));
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.pgm")), out).unwrap();
    }
}
