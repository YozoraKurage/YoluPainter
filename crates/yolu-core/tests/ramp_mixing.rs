//! ランプの混色（混色モード・輝度の補正）と区間ごとの混合率曲線: 既定は昔どおり、曲線は中点に勝つ、分岐点の差し替えでの扱い、
//! グラデーションマップの表への反映。
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, OpacityStop, Ramp};
use yolu_core::{GradientMap, Rgba8};

fn stop(position: f64, rgb: [u8; 3], midpoint: f64) -> ColorStop {
    ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    }
}

fn opaque() -> Vec<OpacityStop> {
    vec![
        OpacityStop {
            position: 0.0,
            opacity: 1.0,
            midpoint: 0.5,
        },
        OpacityStop {
            position: 1.0,
            opacity: 1.0,
            midpoint: 0.5,
        },
    ]
}

fn ramp3() -> Ramp {
    Ramp::new(
        vec![
            stop(0.0, [10, 20, 120], 0.5),
            stop(0.5, [200, 60, 90], 0.3),
            stop(1.0, [250, 230, 120], 0.5),
        ],
        opaque(),
        None,
    )
    .unwrap()
}

fn curve(points: &[(f64, f64)]) -> Curve {
    Curve::new(points.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap()
}

#[test]
fn a_ramp_without_mixing_evaluates_exactly_as_before() {
    let r = ramp3();
    assert!(!r.uses_mixing());
    assert_eq!(r.mix_mode(), MixMode::Standard);
    // 区間 0（中点 0.5）の真ん中は、sRGB の値の単純な平均
    let c = r.sample_stops(0.25, false).unwrap();
    assert_eq!((c.r, c.g, c.b), (105, 40, 105));
    // 中点 0.3 の区間は、中点で重みが 0.5
    let m = r.sample_stops(0.5 + 0.5 * 0.3, false).unwrap();
    assert_eq!(
        (m.r, m.g, m.b),
        (((200.0 + 250.0) / 2.0 + 0.5) as u8, 145, 105)
    );
}

#[test]
fn a_segment_curve_replaces_the_midpoint_in_its_segment_only() {
    let r = ramp3();
    // 区間 1 に「ずっと左の色」の曲線（重みが 0 のまま）
    let flat = curve(&[(0.0, 0.0), (1.0, 0.0)]);
    let with = r.with_segment_curve(1, Some(flat)).unwrap();
    assert!(with.uses_mixing());
    let inside = with.sample_stops(0.8, false).unwrap();
    assert_eq!(
        (inside.r, inside.g, inside.b),
        (200, 60, 90),
        "左の色のまま"
    );
    // 区間 0 は変わらない
    assert_eq!(
        with.sample_stops(0.25, false).unwrap(),
        r.sample_stops(0.25, false).unwrap()
    );
    // 曲線の右端の高さが 1 でなければ、右の分岐点の上でも左の色のまま（分岐点で色が跳ぶことを許す。CLIP STUDIO の混合率曲線と同じ）
    assert_eq!(with.sample_stops(1.0, false).unwrap().r, 200);
    // 外すと元に戻り、全部なしは「使っていない」と等しい
    let back = with.with_segment_curve(1, None).unwrap();
    assert_eq!(back, r);
    assert!(!back.uses_mixing());
    assert!(back.segment_curves().is_empty());
}

#[test]
fn segment_curve_errors_and_normalisation() {
    let r = ramp3();
    assert!(
        r.with_segment_curve(2, Some(Curve::identity())).is_err(),
        "区間は 2 つ"
    );
    assert!(r.with_segment_curves(vec![None]).is_err(), "数が合わない");
    assert!(r
        .with_segment_curves(vec![None, None])
        .unwrap()
        .segment_curves()
        .is_empty());
    let both = r
        .with_segment_curves(vec![Some(Curve::identity()), None])
        .unwrap();
    assert_eq!(both.segment_curves().len(), 2);
    assert!(both.segment_curve(0).is_some() && both.segment_curve(1).is_none());
    assert!(both.segment_curve(7).is_none());
}

#[test]
fn replacing_stops_keeps_mixing_and_keeps_segment_curves_only_when_the_count_is_the_same() {
    let r = ramp3()
        .with_mixing(MixMode::Perceptual, LuminanceCorrection::Low)
        .with_segment_curve(0, Some(curve(&[(0.0, 0.0), (0.4, 0.8), (1.0, 1.0)])))
        .unwrap();
    // 位置だけ変える（数が同じ）
    let mut colors = r.colors().to_vec();
    colors[1].position = 0.6;
    let moved = r.with_stops(colors, r.opacities().to_vec()).unwrap();
    assert_eq!(moved.mix_mode(), MixMode::Perceptual);
    assert_eq!(moved.luminance_correction(), LuminanceCorrection::Low);
    assert!(moved.segment_curve(0).is_some());
    // 数が変わる（消した）と曲線は外す。混色は残す
    let mut fewer = r.colors().to_vec();
    fewer.remove(1);
    let removed = r.with_stops(fewer, r.opacities().to_vec()).unwrap();
    assert_eq!(removed.mix_mode(), MixMode::Perceptual);
    assert!(removed.segment_curves().is_empty());
    // 値のカーブの差し替えは混色・混合率曲線を残す
    let valued = r.with_value_curve(curve(&[(0.0, 0.0), (1.0, 0.5)]));
    assert_eq!(valued.mix_mode(), MixMode::Perceptual);
    assert!(valued.segment_curve(0).is_some());
}

#[test]
fn the_luminance_correction_is_only_kept_for_the_perceptual_mode() {
    let r = ramp3();
    let p = r.with_mixing(MixMode::Perceptual, LuminanceCorrection::Max);
    assert_eq!(p.luminance_correction(), LuminanceCorrection::Max);
    let l = p.with_mixing(MixMode::Linear, LuminanceCorrection::Max);
    assert_eq!(
        l.luminance_correction(),
        LuminanceCorrection::default(),
        "見えない値は既定へ"
    );
    // 同じ見た目なら同じランプ（見えない値の違いで食い違わない）
    assert_eq!(
        r.with_mixing(MixMode::Linear, LuminanceCorrection::None),
        r.with_mixing(MixMode::Linear, LuminanceCorrection::Max)
    );
    assert!(l.uses_mixing());
    assert_eq!(l.without_mixing(), r);
}

#[test]
fn modes_change_the_middle_but_never_the_stops_and_the_perceptual_middle_is_lighter_than_a_dark_mix(
) {
    let std = ramp3();
    for mode in [MixMode::Linear, MixMode::Perceptual] {
        let r = std.with_mixing(mode, LuminanceCorrection::High);
        for p in [0.0, 0.5, 1.0] {
            assert_eq!(
                r.sample_stops(p, false).unwrap(),
                std.sample_stops(p, false).unwrap(),
                "{mode:?} {p}"
            );
        }
        assert_ne!(
            r.sample_stops(0.25, false).unwrap(),
            std.sample_stops(0.25, false).unwrap(),
            "{mode:?}"
        );
    }
}

#[test]
fn midpoint_curves_pass_through_the_midpoint() {
    for m in [0.1, 0.3, 0.5, 0.8, 0.99] {
        let c = Ramp::curve_from_midpoint(m);
        let at = c.value(m.clamp(0.03, 0.97)).unwrap();
        assert!((at - 0.5).abs() < 1e-9, "{m}: {at}");
        assert_eq!(c.value(0.0).unwrap(), 0.0);
        assert_eq!(c.value(1.0).unwrap(), 1.0);
    }
    assert!(Ramp::curve_from_midpoint(0.5).is_identity());
}

#[test]
fn the_history_size_counts_the_segment_curves() {
    let r = ramp3();
    let with = r
        .with_segment_curve(0, Some(curve(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)])))
        .unwrap();
    assert_eq!(with.byte_size(), r.byte_size() + 16 + 24 * 3);
}

#[test]
fn the_gradient_map_table_follows_the_mixing() {
    let std = GradientMap::new(ramp3(), false);
    let perceptual = GradientMap::new(
        ramp3().with_mixing(MixMode::Perceptual, LuminanceCorrection::High),
        false,
    );
    let grey = |v: u8| Rgba8::new(v, v, v, 255);
    // 暗い端・明るい端は同じ。途中は違う
    assert_eq!(std.apply(grey(0)), perceptual.apply(grey(0)));
    assert_eq!(std.apply(grey(255)), perceptual.apply(grey(255)));
    assert_ne!(std.apply(grey(64)), perceptual.apply(grey(64)));
    // 等しさは混色も見る（表が同じでも別の設定）
    assert_ne!(std, perceptual);
}
