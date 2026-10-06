//! 行ごとの評価（`BoundGenerator::sample_row`）が、1 画素ずつの `sample` と同じ値を返すこと。種類・変種・対象・行の切り方
//! （行の途中から・右端を越える・下端を越える）のすべてで、`Generated` がそのまま（f64 はビットまで）等しい。
mod generator_support;
use generator_support::*;
use yolu_core::generator::*;

/// `(x0, y, 長さ)`: 行の全体・途中から端まで・右端を越える・下端を越える・上端の外・左端の外。
fn windows(w: u32, h: u32) -> Vec<(u32, u32, usize)> {
    vec![
        (0, 0, w as usize),
        (3, 5, (w - 3) as usize),
        (7, h - 1, 11),
        (w - 2, 2, 5),
        (0, h, 4),
        (w, 0, 3),
        (1, h / 2, 1),
        (0, 3, 0),
    ]
}

fn same(a: Option<Generated>, b: Option<Generated>) -> bool {
    match (a, b) {
        (Some(Generated::Scalar(x)), Some(Generated::Scalar(y))) => x.to_bits() == y.to_bits(),
        (a, b) => a == b,
    }
}

#[test]
fn rows_equal_single_pixels_for_every_kind_and_variant() {
    let (w, h) = (53, 37);
    let mut checked = 0;
    for kind in KINDS {
        for v in 0..12 {
            with_bound(kind, v, w, h, |b| {
                for scalar in [false, true] {
                    for (x0, y, len) in windows(w, h) {
                        let mut got = vec![Some(Generated::Scalar(-1.)); len];
                        b.sample_row(x0, y, scalar, &mut got);
                        for (i, g) in got.into_iter().enumerate() {
                            let want = b.sample(x0 + i as u32, y, scalar);
                            assert!(
                                same(g, want),
                                "{kind:?} v{v} scalar {scalar} ({}, {y}): {g:?} != {want:?}",
                                x0 + i as u32
                            );
                            checked += 1;
                        }
                    }
                }
            });
        }
    }
    assert!(checked > 20_000, "{checked}");
}

/// ノイズ・グランジも行で評価でき、画素ごとの結果と同じ（位置の空間・UV・トライプラナー、にじみ・回転つき）。
#[test]
fn procedural_rows_equal_single_pixels() {
    let (w, h) = (40u32, 28u32);
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
            normal.extend(if x < w / 3 {
                [60000u16, 33000, 31000]
            } else if y < h / 2 {
                [33000, 60000, 34000]
            } else {
                [34000, 31000, 60000]
            });
            cover.push(u8::from((x + 2 * y) % 11 != 0));
        }
    }
    let entries = [
        (MapKind::Position, &position),
        (MapKind::WorldNormal, &normal),
    ];
    let maps = entries
        .iter()
        .map(|(kind, data)| Map {
            kind: *kind,
            width: w,
            height: h,
            data,
            coverage: &cover,
            bounds_min: [-1., -2., -3.],
            bounds_max: [2., 3., 1.],
            condition_key: KEY,
            state: MapState::Current,
        })
        .collect::<Vec<_>>();
    let mut settings = Vec::new();
    for basis in [NoiseBasis::Value, NoiseBasis::Perlin, NoiseBasis::Worley] {
        let mut s = Settings::new(Kind::Noise);
        s.procedural.basis = basis;
        s.procedural.seed = 7;
        settings.push(s);
    }
    for preset in GrungePreset::ALL {
        settings.push(Settings::grunge(preset));
    }
    let mut checked = 0;
    for base in settings {
        for space in [
            ProceduralSpace::Position,
            ProceduralSpace::Triplanar,
            ProceduralSpace::Uv,
        ] {
            for bleed in [0., 0.5] {
                let mut s = base.clone();
                s.procedural.space = space;
                s.procedural.bleed = bleed;
                s.procedural.rotation = [13., -27., 41.];
                s.ramp = None;
                let b =
                    BoundGenerator::bind(&s, &maps, None, (w, h), Err(anchor::Issue::NotChosen))
                        .unwrap();
                for (x0, y, len) in windows(w, h) {
                    let mut got = vec![None; len];
                    b.sample_row(x0, y, false, &mut got);
                    for (i, g) in got.into_iter().enumerate() {
                        let want = b.sample(x0 + i as u32, y, false);
                        assert!(
                            same(g, want),
                            "{:?} {space:?} bleed {bleed} ({}, {y}): {g:?} != {want:?}",
                            s.procedural.preset,
                            x0 + i as u32
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 8_000, "{checked}");
}
