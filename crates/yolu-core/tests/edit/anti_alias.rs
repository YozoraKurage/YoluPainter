//! ブラシの縁のアンチエイリアス（`BrushSettings::anti_alias`）の 2D: なし と柔らかい筆先は今のバイトのまま、硬い丸は段の幅の帯で
//! 縁がなめらかになり太さは変わらない、左右・上下・点の対称、外へ向けて単調、1 画素より小さいダブは面積に見合う濃さ（画素の間を
//! 通っても消えない）、画像の筆先の小さなダブ、2D の対称（写し）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, AntiAlias, Brush, BrushSettings, CanvasSymmetry, Channel, Document, Rgba8,
    SymmetryMode,
};

const SIZE: u32 = 64;

fn brush(level: AntiAlias, radius: f64, hardness: f64) -> Brush {
    Brush::from(BrushSettings {
        radius,
        hardness,
        spacing: 0.1,
        opacity: 1.0,
        flow: 1.0,
        color: Rgba8::new(0, 0, 0, 255),
        pressure_size: false,
        pressure_opacity: false,
        pressure_flow: false,
        erase: false,
        anti_alias: level,
    })
}

/// 点の列を 1 本のストロークで描いた、レイヤーの全バイト。
fn draw(b: &Brush, points: &[(f64, f64)]) -> Vec<u8> {
    let mut doc = Document::with_tile_size(SIZE, SIZE, 32).unwrap();
    let layer = doc.add_layer("a").unwrap();
    let mut s = doc.begin_brush_stroke(layer, b).unwrap();
    for &(x, y) in points {
        s.add_point(&mut doc, x, y, 1.0, DVec2::ZERO).unwrap();
    }
    doc.end_stroke(s).unwrap();
    doc.layers()[0].surface(Channel::Color).map_or_else(
        || vec![0; (SIZE * SIZE * 4) as usize],
        |s| s.to_canvas_bytes(),
    )
}

/// 1 つのダブのアルファ（下の行から、画素の番号 y × SIZE + x）。
fn dab(b: &Brush, x: f64, y: f64) -> Vec<u8> {
    draw(b, &[(x, y)]).chunks_exact(4).map(|p| p[3]).collect()
}

fn at(a: &[u8], x: i64, y: i64) -> u8 {
    a[(y * SIZE as i64 + x) as usize]
}

fn ink(a: &[u8]) -> f64 {
    a.iter().map(|&v| v as f64 / 255.0).sum()
}

const LEVELS: [AntiAlias; 3] = [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong];

#[test]
fn soft_brushes_paint_the_same_bytes_at_every_level() {
    // 帯（2 画素まで）がぼかしの幅（半径 ×（1 − 硬さ））以下の筆先は、どの段でも今の式のまま
    let line = [(8.0, 9.5), (30.25, 40.0), (55.0, 20.75)];
    let mut ellipse = brush(AntiAlias::None, 12.0, 0.3);
    ellipse.tip.roundness = 0.5;
    ellipse.tip.angle = 25.0;
    for base in [
        brush(AntiAlias::None, 12.0, 0.4),
        brush(AntiAlias::None, 5.0, 0.2),
        ellipse,
    ] {
        let none = draw(&base, &line);
        assert!(none.iter().any(|&v| v != 0));
        for level in LEVELS {
            let mut b = base.clone();
            b.base.anti_alias = level;
            assert_eq!(draw(&b, &line), none, "{level:?} {:?}", base.base);
        }
    }
}

#[test]
fn a_hard_round_tip_gets_a_band_of_the_levels_width_and_keeps_its_thickness() {
    let r = 8.0;
    // 中心を画素の中のいろいろな所に置き、中心を通る行の、縁の帯の画素（0.02〜0.98）の数と、行の覆いの合計（太さ）を測る
    let mut widths = Vec::new();
    for level in [
        AntiAlias::None,
        AntiAlias::Weak,
        AntiAlias::Medium,
        AntiAlias::Strong,
    ] {
        let b = brush(level, r, 1.0);
        let (mut partial, mut count) = (0usize, 0usize);
        for k in 0..8 {
            let x = 32.0 + k as f64 / 8.0;
            let a = dab(&b, x, 32.5);
            let row: Vec<f64> = (0..SIZE as i64)
                .map(|i| at(&a, i, 32) as f64 / 255.0)
                .collect();
            partial += row.iter().filter(|&&c| c > 0.02 && c < 0.98).count();
            count += 2;
            // なし は画素の中心が半径の上に乗ると 2 値で 1 画素太る
            let thickness: f64 = row.iter().sum();
            let allowed = if level == AntiAlias::None { 1.0 } else { 0.5 };
            assert!(
                (thickness - 2.0 * r).abs() <= allowed,
                "{level:?} x {x}: 太さ {thickness}"
            );
        }
        widths.push(partial as f64 / count as f64);
    }
    // 片側の帯の画素の数の平均: なし 0、弱 < 中 < 強（smoothstep の 0.02〜0.98 は帯の 0.84）
    assert_eq!(widths[0], 0.0, "なし は 2 値");
    assert!(widths[1] > 0.2 && widths[1] < 0.75, "{widths:?}");
    assert!(widths[2] > 0.6 && widths[2] < 1.2, "{widths:?}");
    assert!(widths[3] > 1.3 && widths[3] < 2.1, "{widths:?}");
}

#[test]
fn the_band_is_symmetric_and_falls_monotonically() {
    for level in LEVELS {
        let a = dab(&brush(level, 6.0, 1.0), 32.5, 32.5);
        for k in 0..12 {
            assert_eq!(at(&a, 32 - k, 32), at(&a, 32 + k, 32), "{level:?} 左右 {k}");
            assert_eq!(at(&a, 32, 32 - k), at(&a, 32, 32 + k), "{level:?} 上下 {k}");
            assert!(
                at(&a, 32 + k, 32) >= at(&a, 33 + k, 32),
                "{level:?} 単調 {k}"
            );
            assert!(
                at(&a, 32 + k, 32 + k) >= at(&a, 33 + k, 33 + k),
                "{level:?} 斜め {k}"
            );
        }
        // 回して潰した丸は点の対称
        let mut b = brush(level, 10.0, 1.0);
        b.tip.roundness = 0.35;
        b.tip.angle = 30.0;
        let a = dab(&b, 32.5, 32.5);
        let partial = a.iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(partial > 0, "{level:?}: 潰した丸にも帯");
        for j in -12..=12 {
            for i in -12..=12 {
                assert_eq!(
                    at(&a, 32 + i, 32 + j),
                    at(&a, 32 - i, 32 - j),
                    "{level:?} ({i}, {j})"
                );
            }
        }
    }
}

#[test]
fn thin_rotated_ellipses_stay_within_the_radius_plus_band_and_keep_their_area() {
    // 短い軸が帯の半分より細い楕円: 短い軸を帯の半分まで広げて濃さで面積を保ち、帯の式の距離を外接の箱の外の距離で押さえるので、
    // 帯が長い軸の先の外へ伸びない
    for (r, q) in [(6.0, 0.05), (3.0, 0.1), (12.0, 0.04)] {
        let area = std::f64::consts::PI * r * r * q;
        for level in LEVELS {
            // 弱は細い線で帯を 1 画素まで広げる
            let band = level.band().max(1.0);
            for angle in [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0, 110.0, 135.0, 160.0] {
                for (x, y) in [(32.5, 32.5), (32.2, 32.9)] {
                    let mut b = brush(level, r, 1.0);
                    b.tip.roundness = q;
                    b.tip.angle = angle;
                    let a = dab(&b, x, y);
                    let mut farthest = 0.0f64;
                    for (i, &v) in a.iter().enumerate() {
                        if v > 0 {
                            let (px, py) = (
                                (i % SIZE as usize) as f64 + 0.5,
                                (i / SIZE as usize) as f64 + 0.5,
                            );
                            farthest = farthest.max(((px - x).powi(2) + (py - y).powi(2)).sqrt());
                        }
                    }
                    assert!(
                        farthest <= r + band,
                        "{level:?} r {r} q {q} 角度 {angle}: {farthest} まで塗った"
                    );
                    let got = ink(&a);
                    assert!(
                        got > 0.0 && got <= 1.3 * area,
                        "{level:?} r {r} q {q} 角度 {angle}: インク {got} / 面積 {area}"
                    );
                }
            }
        }
    }
}

#[test]
fn dabs_smaller_than_a_pixel_get_a_density_for_their_area() {
    let offsets = [(0.0, 0.0), (0.25, 0.0), (0.5, 0.0), (0.5, 0.5), (0.25, 0.4)];
    for r in [0.3, 0.7, 1.5] {
        let area = std::f64::consts::PI * r * r;
        // なし: 画素の中心が当たらない所では消え（0.3・0.7）、当たれば 1 画素分（0.3）
        let none: Vec<f64> = offsets
            .iter()
            .map(|(dx, dy)| ink(&dab(&brush(AntiAlias::None, r, 1.0), 32.5 + dx, 32.5 + dy)))
            .collect();
        if r < 1.0 {
            assert!(none.contains(&0.0), "{r}: {none:?}");
        }
        if r == 0.3 {
            assert_eq!(none[0], 1.0, "1 画素分のスタンプ");
        }
        for level in LEVELS {
            let inks: Vec<f64> = offsets
                .iter()
                .map(|(dx, dy)| ink(&dab(&brush(level, r, 1.0), 32.5 + dx, 32.5 + dy)))
                .collect();
            let (lo, hi) = inks
                .iter()
                .fold((f64::MAX, 0.0f64), |(l, h), &v| (l.min(v), h.max(v)));
            assert!(
                lo > 0.5 * area && hi < 1.7 * area,
                "{level:?} r {r}: {inks:?} 面積 {area}"
            );
            // 置き場所による揺れ: 弱は帯が狭いぶん大きい
            let spread = if level == AntiAlias::Weak { 1.75 } else { 1.45 };
            assert!(
                hi / lo < spread,
                "{level:?} r {r}: 置き場所で濃さが揺れる {inks:?}"
            );
            if r == 0.3 {
                let a = dab(&brush(level, r, 1.0), 32.5, 32.5);
                assert!(at(&a, 32, 32) < 128, "{level:?}: 1 画素分の濃さにしない");
            }
        }
    }
}

#[test]
fn a_tiny_image_tip_is_spread_with_a_density_for_its_size() {
    let tip = |level| {
        let mut b = brush(level, 0.3, 1.0);
        b.tip.image = builtin_tip("rounded-square");
        b
    };
    // 大きな画像の筆先は、どの段でも今のバイト
    let mut big = tip(AntiAlias::None);
    big.base.radius = 9.0;
    let line = [(10.0, 10.0), (50.0, 40.0)];
    let none = draw(&big, &line);
    for level in LEVELS {
        let mut b = big.clone();
        b.base.anti_alias = level;
        assert_eq!(draw(&b, &line), none, "{level:?}");
    }
    // 小さなダブ: なし は画素の中心にかからないと消え、かかれば 1 画素分。段があれば帯の半分まで広げて薄くする
    // （画像の補間は今のままなので、縁の透明な画像では、画素の中心が広げた縁に乗る置き場所ではまだ消えうる）
    assert_eq!(ink(&dab(&tip(AntiAlias::None), 32.9, 32.7)), 0.0);
    let centred = dab(&tip(AntiAlias::None), 32.5, 32.5);
    assert_eq!(at(&centred, 32, 32), 255);
    for level in LEVELS {
        let between = ink(&dab(&tip(level), 32.9, 32.7));
        let a = dab(&tip(level), 32.5, 32.5);
        assert!(between > 0.0, "{level:?}");
        assert!(at(&a, 32, 32) < 255, "{level:?}");
    }
}

#[test]
fn symmetric_copies_use_the_same_edge() {
    for level in LEVELS {
        let mut b = brush(level, 4.5, 1.0);
        b.symmetry =
            CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(32.0, 32.0), 2).unwrap();
        let a: Vec<u8> = draw(&b, &[(20.3, 30.7), (24.1, 41.2)])
            .chunks_exact(4)
            .map(|p| p[3])
            .collect();
        let partial = a.iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(partial > 0, "{level:?}: 写しにも帯");
        for y in 0..SIZE as i64 {
            for x in 0..32 {
                assert_eq!(at(&a, x, y), at(&a, 63 - x, y), "{level:?} ({x}, {y})");
            }
        }
        // 写しの無いストロークの元の側とほぼ同じ（対称のダブは倍精度の式）
        let mut plain = b.clone();
        plain.symmetry = CanvasSymmetry::default();
        let p: Vec<u8> = draw(&plain, &[(20.3, 30.7), (24.1, 41.2)])
            .chunks_exact(4)
            .map(|p| p[3])
            .collect();
        for y in 0..SIZE as i64 {
            for x in 0..32 {
                assert!(
                    (at(&a, x, y) as i32 - at(&p, x, y) as i32).abs() <= 1,
                    "{level:?} ({x}, {y})"
                );
            }
        }
    }
}
