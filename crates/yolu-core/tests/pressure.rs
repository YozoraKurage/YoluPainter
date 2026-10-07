//! 筆圧の応え（項目ごとの最小値と曲線）: 既定では応えを足す前と画素までバイト単位で同じ（応えを足す前のコードが描いた画素のハッシュを固定して
//! 筆圧が変わる線で確かめる）、最小値と曲線が大きさ・不透明度・流量・硬さに
//! 効く、切っている項目は応えを使わない、範囲と形の検査、同じ入力は同じ画素、Undo で戻る。
//! 1 つのダブは「1 点だけのストローク」で置き、筆圧の応えを通した値は、同じ値を固定の設定に入れたブラシと画素を比べて確かめる
//! （応えの式そのものは `brush/pressure.rs` の単体試験）。
//! 束に入れず直下の 1 本: ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を最大と 0 に切り替えて、直列の経路だけ・ワーカーの経路を通ることを確かめる。同じプロセスのほかの試験が閾値を変えると外れる。
#![allow(clippy::chunks_exact_to_as_chunks)]

use yolu_core::generator::CurvePoint;
use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSample, BrushSettings, Channel, Document, LayerId, PressureResponse,
    PressureResponses, Rgba8,
};

const INK: Rgba8 = Rgba8::new(20, 30, 40, 255);
const SIZE: u32 = 64;

fn canvas() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(SIZE, SIZE, 16).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn pt(x: f64, y: f64) -> CurvePoint {
    CurvePoint { x, y }
}

fn response(min: f64, curve: &[CurvePoint]) -> PressureResponse {
    PressureResponse::new(min, curve.to_vec()).unwrap()
}

/// 筆圧のフラグだけが違う丸いブラシ（半径 10・硬さ 1・不透明度 1・流量 1）。
fn round(size: bool, opacity: bool, flow: bool) -> Brush {
    Brush::from(BrushSettings {
        radius: 10.0,
        hardness: 1.0,
        spacing: 0.1,
        opacity: 1.0,
        flow: 1.0,
        color: INK,
        pressure_size: size,
        pressure_opacity: opacity,
        pressure_flow: flow,
        ..BrushSettings::default()
    })
}

fn sample(x: f64, y: f64, pressure: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, pressure, time, DVec2::ZERO).unwrap()
}

/// ブラシで点列を描いて確定し、合成した画素（RGBA）を返す。
fn draw(brush: &Brush, points: &[(f64, f64, f64)]) -> Vec<u8> {
    draw_counting(brush, points).0
}

/// `draw` に加えて、確定の前に取った「ワーカーで並列に描いたダブの数」を返す（並列の経路が実際に通ったかを確かめる）。
fn draw_counting(brush: &Brush, points: &[(f64, f64, f64)]) -> (Vec<u8>, u64) {
    let (mut d, l) = canvas();
    let mut s = d.begin_brush_stroke(l, brush).unwrap();
    for (i, p) in points.iter().enumerate() {
        s.add_sample(&mut d, sample(p.0, p.1, p.2, i as f64 * 0.01))
            .unwrap();
    }
    let parallel = d.active_stroke_stats().unwrap().parallel_dabs;
    d.end_stroke(s).unwrap();
    (d.composite(d.bounds()).unwrap(), parallel)
}

/// 画素の数（アルファが 0 でないもの）。
fn painted(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
}

fn alpha(pixels: &[u8], x: usize, y: usize) -> u8 {
    pixels[(y * SIZE as usize + x) * 4 + 3]
}

/// 筆圧が行ったり来たりする線（応えの違いが画素に出やすい）。
fn wavy() -> Vec<(f64, f64, f64)> {
    (0..=40)
        .map(|i| {
            let t = i as f64 / 40.0;
            (
                8.0 + 48.0 * t,
                32.0 + 8.0 * (t * 9.0).sin(),
                0.05 + 0.95 * ((t * 7.0).sin() * 0.5 + 0.5),
            )
        })
        .collect()
}

fn all_straight() -> PressureResponses {
    let straight = PressureResponse::new(0.0, vec![pt(0.0, 0.0), pt(1.0, 1.0)]).unwrap();
    PressureResponses {
        size: straight.clone(),
        opacity: straight.clone(),
        flow: straight.clone(),
        hardness: straight,
    }
}

/// FNV-1a（64 ビット）。画素の列を 1 つの値にして固定する。
fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// 応えを足す前のコード（8ac0a3d の親）が、同じ線・同じブラシで描いた画素のハッシュ。同じ試験の補助関数（`canvas`・`round`・`wavy`・`draw`）を
/// そのコードに写して測った。筆圧が行ったり来たりする線なので、ダブごとに大きさ・不透明度・流量が筆圧そのものに従う道が効く。
#[test]
fn the_default_response_paints_the_bytes_the_code_painted_before_responses_existed() {
    let before_responses = [
        ((true, true, false), 0x2677_9ba8_2060_e4fe_u64),
        ((true, false, true), 0xd0c8_3347_5d34_1bbe),
        ((false, true, true), 0xdac0_0180_0df8_52cb),
        ((true, true, true), 0x5df9_32a3_6c54_2453),
    ];
    for ((size, opacity, flow), hash) in before_responses {
        let mut brush = round(size, opacity, flow);
        brush.base.hardness = 0.6;
        brush.base.flow = 0.5;
        brush.base.opacity = 0.8;
        assert_eq!(brush.pressure, all_straight());
        let pixels = draw(&brush, &wavy());
        assert!(painted(&pixels) > 800, "{size} {opacity} {flow}");
        assert_eq!(fnv(&pixels), hash, "{size} {opacity} {flow}");
    }
}

#[test]
fn an_explicit_straight_response_equals_the_default_response() {
    assert_eq!(Brush::default().pressure, all_straight());
    for (size, opacity, flow) in [
        (true, true, false),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let mut a = round(size, opacity, flow);
        a.base.hardness = 0.6;
        a.base.flow = 0.5;
        a.base.opacity = 0.8;
        let mut b = a.clone();
        b.pressure = all_straight();
        assert_eq!(a, b);
        assert_eq!(
            draw(&a, &wavy()),
            draw(&b, &wavy()),
            "{size} {opacity} {flow}"
        );
    }
}

#[test]
fn the_default_pressure_flags_still_follow_the_pressure_itself() {
    // 応えを足す前の式（半径 × 筆圧・不透明度 × 筆圧）と同じ画素: 筆圧を固定の設定へ入れたブラシと比べる
    let p = 0.37;
    let with_pressure = draw(&round(true, false, false), &[(32.0, 32.0, p)]);
    let mut fixed = round(false, false, false);
    fixed.base.radius = 10.0 * p;
    assert_eq!(with_pressure, draw(&fixed, &[(32.0, 32.0, 1.0)]));
    let with_pressure = draw(&round(false, true, false), &[(32.0, 32.0, p)]);
    let mut fixed = round(false, false, false);
    fixed.base.opacity = p;
    assert_eq!(with_pressure, draw(&fixed, &[(32.0, 32.0, 1.0)]));
}

#[test]
fn a_size_minimum_keeps_a_dab_at_pressure_zero() {
    let none = draw(&round(true, false, false), &[(32.0, 32.0, 0.0)]);
    assert_eq!(painted(&none), 0, "最小値 0 の筆圧 0 は点を置かない");
    let mut b = round(true, false, false);
    b.pressure.size = response(0.4, &[]);
    let lifted = draw(&b, &[(32.0, 32.0, 0.0)]);
    let mut fixed = round(false, false, false);
    fixed.base.radius = 10.0 * 0.4;
    assert_eq!(lifted, draw(&fixed, &[(32.0, 32.0, 1.0)]));
    assert!(painted(&lifted) > 40);
    // 筆圧 1 は元の大きさのまま
    assert_eq!(
        draw(&b, &[(32.0, 32.0, 1.0)]),
        draw(&round(false, false, false), &[(32.0, 32.0, 1.0)])
    );
}

#[test]
fn a_size_curve_sets_the_radius_from_the_shaped_pressure() {
    let soft = response(0.1, &[pt(0.0, 0.0), pt(0.5, 0.85), pt(1.0, 1.0)]);
    let mut b = round(true, false, false);
    b.pressure.size = soft.clone();
    for p in [0.0, 0.25, 0.5, 0.8, 1.0] {
        let mut fixed = round(false, false, false);
        fixed.base.radius = 10.0 * soft.apply(p);
        assert_eq!(
            draw(&b, &[(32.0, 32.0, p)]),
            draw(&fixed, &[(32.0, 32.0, 1.0)]),
            "p = {p}"
        );
    }
    // 曲線は筆圧が低い所を持ち上げる: 同じ筆圧 0.25 で、直線より大きい
    let straight = draw(&round(true, false, false), &[(32.0, 32.0, 0.25)]);
    assert!(painted(&draw(&b, &[(32.0, 32.0, 0.25)])) > painted(&straight));
}

#[test]
fn an_opacity_minimum_and_curve_set_the_ceiling() {
    let mut b = round(false, true, false);
    b.pressure.opacity = response(0.25, &[]);
    let at_zero = draw(&b, &[(32.0, 32.0, 0.0)]);
    let mut fixed = round(false, false, false);
    fixed.base.opacity = 0.25;
    assert_eq!(at_zero, draw(&fixed, &[(32.0, 32.0, 1.0)]));
    assert!((i32::from(alpha(&at_zero, 32, 32)) - 64).abs() <= 1);
    // 曲線: 筆圧 0.5 の係数をそのまま天井にする
    let curve = response(0.0, &[pt(0.0, 0.0), pt(0.5, 0.9), pt(1.0, 1.0)]);
    b.pressure.opacity = curve.clone();
    let mid = draw(&b, &[(32.0, 32.0, 0.5)]);
    let mut fixed = round(false, false, false);
    fixed.base.opacity = curve.apply(0.5);
    assert_eq!(mid, draw(&fixed, &[(32.0, 32.0, 1.0)]));
    assert!(alpha(&mid, 32, 32) > 200);
}

#[test]
fn a_flow_minimum_sets_how_fast_dabs_build_up() {
    // 流量が低いと、同じ場所に重ねたダブが天井へ寄るのが遅い。筆圧 0 でも最小値の流量で積もる
    let (mut brush, mut fixed) = (round(false, false, true), round(false, false, false));
    brush.base.flow = 0.5;
    brush.pressure.flow = response(0.4, &[]);
    fixed.base.flow = 0.5 * 0.4;
    let points = [(32.0, 32.0, 0.0), (32.0, 32.0, 0.0), (32.0, 32.0, 0.0)];
    let fixed_points = [(32.0, 32.0, 1.0), (32.0, 32.0, 1.0), (32.0, 32.0, 1.0)];
    let a = draw(&brush, &points);
    let b = draw(&fixed, &fixed_points);
    for (x, y) in [(32, 32), (28, 32), (38, 30)] {
        assert!(
            (i32::from(alpha(&a, x, y)) - i32::from(alpha(&b, x, y))).abs() <= 1,
            "{x},{y}: {} {}",
            alpha(&a, x, y),
            alpha(&b, x, y)
        );
    }
    assert!(alpha(&a, 32, 32) > 0);
    // 最小値 0 の筆圧 0 は積もらない
    brush.pressure.flow = response(0.0, &[]);
    assert_eq!(painted(&draw(&brush, &points)), 0);
}

#[test]
fn a_hardness_response_softens_the_edge_at_low_pressure() {
    let mut brush = round(false, false, false);
    brush.base.radius = 20.0;
    brush.base.hardness = 0.9;
    brush.controls.pressure_hardness = true;
    brush.pressure.hardness = response(0.0, &[]);
    for p in [0.0, 0.3, 1.0] {
        let mut fixed = brush.clone();
        fixed.controls.pressure_hardness = false;
        fixed.base.hardness = 0.9 * brush.pressure.hardness.apply(p);
        assert_eq!(
            draw(&brush, &[(32.0, 32.0, p)]),
            draw(&fixed, &[(32.0, 32.0, 1.0)]),
            "p = {p}"
        );
    }
    // 硬さは筆圧が低いほど柔らかい: 縁の近くの濃さが筆圧 1 より薄い
    let soft = draw(&brush, &[(32.0, 32.0, 0.1)]);
    let hard = draw(&brush, &[(32.0, 32.0, 1.0)]);
    assert!(alpha(&soft, 32 + 15, 32) < alpha(&hard, 32 + 15, 32));
}

#[test]
fn items_that_are_switched_off_ignore_their_response() {
    let mut off = round(false, false, false);
    off.pressure = PressureResponses {
        size: response(0.9, &[pt(0.0, 0.0), pt(0.5, 0.2), pt(1.0, 1.0)]),
        opacity: response(0.9, &[]),
        flow: response(0.9, &[]),
        hardness: response(0.9, &[]),
    };
    let plain = round(false, false, false);
    assert_eq!(draw(&off, &wavy()), draw(&plain, &wavy()));
    // 1 項目だけ入れても、ほかの項目の応えは使わない
    let mut size_only = round(true, false, false);
    size_only.pressure.opacity = response(0.9, &[]);
    assert_eq!(
        draw(&size_only, &wavy()),
        draw(&round(true, false, false), &wavy())
    );
}

#[test]
fn the_same_input_paints_the_same_pixels_and_one_undo_restores_the_canvas() {
    let mut b = round(true, true, true);
    b.base.flow = 0.6;
    b.controls.pressure_hardness = true;
    b.base.hardness = 0.7;
    b.pressure = PressureResponses {
        size: response(0.15, &[pt(0.0, 0.0), pt(0.4, 0.7), pt(1.0, 1.0)]),
        opacity: response(0.3, &[pt(0.0, 0.0), pt(0.6, 0.3), pt(1.0, 1.0)]),
        flow: response(0.2, &[]),
        hardness: response(0.1, &[pt(0.0, 0.0), pt(0.5, 0.2), pt(1.0, 1.0)]),
    };
    let first = draw(&b, &wavy());
    assert_eq!(first, draw(&b, &wavy()));
    assert!(painted(&first) > 0);
    // 筆圧の応えを入れたブラシは、入れないブラシと違う画素を描く
    let mut plain = b.clone();
    plain.pressure = PressureResponses::default();
    assert_ne!(first, draw(&plain, &wavy()));

    let (mut d, l) = canvas();
    let before = d.composite(d.bounds()).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for (i, p) in wavy().iter().enumerate() {
        s.add_sample(&mut d, sample(p.0, p.1, p.2, i as f64 * 0.01))
            .unwrap();
    }
    assert!(d.end_stroke(s).unwrap().changed);
    assert_ne!(d.composite(d.bounds()).unwrap(), before);
    d.undo().unwrap();
    assert_eq!(d.composite(d.bounds()).unwrap(), before);
}

#[test]
fn a_stroke_begun_in_a_named_channel_uses_the_same_response() {
    // チャンネルを指して始めるストロークも、同じ筆圧の応えで大きさが決まる
    let mut b = round(true, false, false);
    b.pressure.size = response(0.5, &[]);
    let (mut d, l) = canvas();
    let mut s = d.begin_brush_stroke_in(l, Channel::Color, &b).unwrap();
    s.add_sample(&mut d, sample(32.0, 32.0, 0.0, 0.0)).unwrap();
    d.end_stroke(s).unwrap();
    let pixels = d.composite(d.bounds()).unwrap();
    let mut fixed = round(false, false, false);
    fixed.base.radius = 5.0;
    assert_eq!(pixels, draw(&fixed, &[(32.0, 32.0, 1.0)]));
}

#[test]
fn responses_are_validated_when_they_are_built() {
    assert!(PressureResponse::new(1.5, vec![]).is_err());
    assert!(PressureResponse::new(f64::NAN, vec![]).is_err());
    assert!(PressureResponse::new(0.0, vec![pt(0.0, 0.0), pt(0.5, 2.0), pt(1.0, 1.0)]).is_err());
    assert!(PressureResponse::new(0.0, vec![pt(0.0, 0.0)]).is_err());
    // 作れた応えは、どのブラシの検査も通る
    let mut b = round(true, true, true);
    b.pressure.size = response(1.0, &[]);
    b.pressure.opacity = response(0.0, &[pt(0.0, 1.0), pt(1.0, 0.0)]);
    b.validate().unwrap();
}

#[test]
fn a_decreasing_curve_inverts_the_pressure() {
    // 強く押すほど小さい（点の値は 0〜1 で、向きは自由）
    let mut b = round(true, false, false);
    b.pressure.size = response(0.0, &[pt(0.0, 1.0), pt(1.0, 0.2)]);
    let light = painted(&draw(&b, &[(32.0, 32.0, 0.1)]));
    let hard = painted(&draw(&b, &[(32.0, 32.0, 1.0)]));
    assert!(light > hard, "{light} {hard}");
}

#[test]
fn a_cancelled_stroke_with_a_response_leaves_the_canvas_untouched() {
    let mut b = round(true, true, true);
    b.pressure.size = response(0.3, &[]);
    b.pressure.opacity = response(0.2, &[pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)]);
    let (mut d, l) = canvas();
    let before = d.composite(d.bounds()).unwrap();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    for (i, p) in wavy().iter().enumerate() {
        s.add_sample(&mut d, sample(p.0, p.1, p.2, i as f64 * 0.01))
            .unwrap();
    }
    assert_ne!(d.composite(d.bounds()).unwrap(), before, "途中は描けている");
    d.cancel_stroke(s);
    assert_eq!(d.composite(d.bounds()).unwrap(), before);
    assert!(!d.can_undo(), "取り消したストロークは履歴に残らない");
}

/// ワーカーで描く経路（ダブごとに並列）を通しても、応えを通した画素は同じ（スレッド数・経路で変わらない）。
#[test]
fn the_response_gives_the_same_pixels_on_the_serial_and_the_parallel_paths() {
    struct Restore(i64);
    impl Drop for Restore {
        fn drop(&mut self) {
            yolu_core::brush::set_parallel_dab_pixels(self.0);
        }
    }
    let mut b = round(true, true, true);
    b.controls.pressure_hardness = true;
    b.base.hardness = 0.6;
    b.base.flow = 0.7;
    b.pressure = PressureResponses {
        size: response(0.2, &[pt(0.0, 0.0), pt(0.4, 0.7), pt(1.0, 1.0)]),
        opacity: response(0.3, &[]),
        flow: response(0.1, &[pt(0.0, 0.0), pt(0.6, 0.3), pt(1.0, 1.0)]),
        hardness: response(0.0, &[]),
    };
    // スレッドの数は環境（RAYON_NUM_THREADS・CPU の数）に頼らず、3 本のワーカーで固定する。
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(3)
        .build()
        .unwrap();
    let _restore = Restore(yolu_core::brush::set_parallel_dab_pixels(i64::MAX));
    let (serial, serial_dabs) = pool.install(|| draw_counting(&b, &wavy()));
    yolu_core::brush::set_parallel_dab_pixels(0);
    let (parallel, parallel_dabs) = pool.install(|| draw_counting(&b, &wavy()));
    assert!(painted(&serial) > 0);
    assert_eq!(serial_dabs, 0, "しきい値が大きいときは直列の経路");
    assert!(parallel_dabs > 0, "ワーカーで並列に描く経路を通る");
    assert!(serial == parallel, "経路によらず同じ画素");
}
