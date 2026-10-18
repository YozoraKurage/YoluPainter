//! 筆先の繰り返し方法（`TipSelection` の 6 種）・反転のしかた（`FlipRule`）・ダブの重なりの比較暗（`TipShape::darken_overlap`）・質感の回転
//! （`PaperTexture::angle`）の 2D。筆先は 3 枚（全部・左半分・右半分）で、間隔を広くして重ならないダブの並びを見る。今までの設定（繰り返し方法が
//! 乱数か順、反転が固定、重ねが足す、回転 0）は、新しい乱数の列を引かないので、同じバイトのまま。
#![allow(clippy::chunks_exact_to_as_chunks)]
use std::sync::Arc;

use yolu_core::glam::DVec2;
use yolu_core::{
    Brush, BrushSettings, BrushTip, Channel, Document, FlipRule, LayerId, PaperTexture, Rgba8,
    TipSelection,
};

const W: u32 = 160;
const H: u32 = 32;
/// 直径 8 の筆先が、12 画素おき（間隔 1.5 × 直径）に置かれる。
const STEP: f64 = 12.0;
const X0: f64 = 10.0;
const Y: f64 = 16.0;

fn doc() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(W, H, 32).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    (d, l)
}

fn base(spacing: f64) -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 4.0,
            hardness: 1.0,
            spacing,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        yolu_core::PressureSwitches::NONE,
    )
}

/// 16×16 の筆先。`filled(x, y)` が真の画素が塗れる（行は下から）。
fn tip(name: &str, filled: impl Fn(usize, usize) -> bool) -> Arc<BrushTip> {
    let mut alpha = vec![0u8; 16 * 16];
    for y in 0..16 {
        for x in 0..16 {
            if filled(x, y) {
                alpha[y * 16 + x] = 255;
            }
        }
    }
    Arc::new(BrushTip::new(name, 16, 16, alpha).unwrap())
}
fn whole() -> Arc<BrushTip> {
    tip("whole", |_, _| true)
}
fn left_half() -> Arc<BrushTip> {
    tip("left half", |x, _| x < 8)
}
fn right_half() -> Arc<BrushTip> {
    tip("right half", |x, _| x >= 8)
}
fn lower_half() -> Arc<BrushTip> {
    tip("lower half", |_, y| y < 8)
}
/// 見分けのつく 3 枚（番号 0 = 全部・1 = 左半分・2 = 右半分）。
fn three() -> Vec<Arc<BrushTip>> {
    vec![whole(), left_half(), right_half()]
}

fn stroke(b: &Brush, from: (f64, f64), to: (f64, f64)) -> Document {
    let (mut d, l) = doc();
    let mut s = d.begin_brush_stroke(l, b).unwrap();
    s.add_point(&mut d, from.0, from.1, 1.0, DVec2::ZERO)
        .unwrap();
    s.add_point(&mut d, to.0, to.1, 1.0, DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
    d
}

fn alpha(d: &Document, x: f64, y: f64) -> u8 {
    d.composite_pixel(Channel::Color, x as u32, y as u32)
        .unwrap()
        .a
}

/// k 番目のダブ（x = X0 + STEP × k）の筆先の見分け: 左右の半分の塗れ方（(左, 右)）。何も塗れなければ (false, false)。
fn halves(d: &Document, k: usize) -> (bool, bool) {
    let x = X0 + STEP * k as f64;
    (alpha(d, x - 2.0, Y) > 0, alpha(d, x + 2.0, Y) > 0)
}
/// 番号 0（全部）・1（左半分）・2（右半分）。置かれていなければ None。
fn code(d: &Document, k: usize) -> Option<u8> {
    match halves(d, k) {
        (true, true) => Some(0),
        (true, false) => Some(1),
        (false, true) => Some(2),
        (false, false) => None,
    }
}
fn codes(d: &Document, n: usize) -> Vec<Option<u8>> {
    (0..n).map(|k| code(d, k)).collect()
}

/// 10 個のダブを 12 画素おきに置いた線（最後のダブは X0 + 108）。
fn ten_dabs(b: &Brush) -> Document {
    stroke(b, (X0, Y), (X0 + STEP * 9.0, Y))
}

fn selecting(selection: TipSelection) -> Brush {
    let mut b = base(1.5);
    b.tip.images = three();
    b.tip.selection = selection;
    b
}

fn some(v: &[u8]) -> Vec<Option<u8>> {
    v.iter().map(|&c| Some(c)).collect()
}

#[test]
fn the_six_repeat_methods_pick_three_tips_for_ten_dabs() {
    // 繰り返し: 0 1 2 0 1 2 … ／ 折り返し: 端を重ねずに戻る ／ 繰り返さない: 最後の筆先のまま
    let d = ten_dabs(&selecting(TipSelection::Sequential));
    assert_eq!(codes(&d, 10), some(&[0, 1, 2, 0, 1, 2, 0, 1, 2, 0]));
    let d = ten_dabs(&selecting(TipSelection::PingPong));
    assert_eq!(codes(&d, 10), some(&[0, 1, 2, 1, 0, 1, 2, 1, 0, 1]));
    let d = ten_dabs(&selecting(TipSelection::Hold));
    assert_eq!(codes(&d, 10), some(&[0, 1, 2, 2, 2, 2, 2, 2, 2, 2]));
    // 一巡のみ: 並びを使い切ったあとは何も置かない
    let d = ten_dabs(&selecting(TipSelection::Once));
    let mut expected = some(&[0, 1, 2]);
    expected.extend([None; 7]);
    assert_eq!(codes(&d, 10), expected);
}

#[test]
fn one_tip_in_the_list_stops_after_the_first_dab_for_the_single_pass_methods() {
    // 並び（`images`）が 1 枚でも、一巡のみ・ランダムに一巡は最初のダブで止まり、繰り返す方法は同じ筆先で続く
    for selection in [TipSelection::Once, TipSelection::Shuffle] {
        let mut b = base(1.5);
        (b.tip.images, b.tip.selection) = (vec![whole()], selection);
        let mut expected = some(&[0]);
        expected.extend([None; 9]);
        assert_eq!(codes(&ten_dabs(&b), 10), expected, "{selection:?}");
    }
    for selection in [
        TipSelection::Sequential,
        TipSelection::PingPong,
        TipSelection::Hold,
    ] {
        let mut b = base(1.5);
        (b.tip.images, b.tip.selection) = (vec![whole()], selection);
        assert_eq!(codes(&ten_dabs(&b), 10), some(&[0; 10]), "{selection:?}");
    }
    // 1 枚の筆先（`image`）は、並びを持たないので繰り返し方法を読まず、いつも置く
    let mut b = base(1.5);
    (b.tip.image, b.tip.selection) = (Some(whole()), TipSelection::Once);
    assert_eq!(codes(&ten_dabs(&b), 10), some(&[0; 10]));
}

#[test]
fn a_shuffled_single_pass_uses_every_tip_once_in_an_order_the_seed_decides() {
    let order = |seed: i32| {
        let mut b = selecting(TipSelection::Shuffle);
        b.seed = seed;
        codes(&ten_dabs(&b), 10)
    };
    let mut seen = std::collections::HashSet::new();
    for seed in 0..16 {
        let got = order(seed);
        assert_eq!(got, order(seed), "同じ種は同じ並び（種 {seed}）");
        let mut first: Vec<u8> = got[..3].iter().map(|c| c.expect("置く")).collect();
        assert!(got[3..].iter().all(Option::is_none), "一巡のあとは置かない");
        seen.insert(first.clone());
        first.sort_unstable();
        assert_eq!(first, vec![0, 1, 2], "3 枚を 1 回ずつ（種 {seed}）");
    }
    assert!(seen.len() > 1, "種が変われば並びも変わる: {seen:?}");
}

#[test]
fn a_one_tip_list_is_the_same_in_every_repeat_method_except_the_ones_that_stop() {
    let one = |selection: TipSelection| {
        let mut b = base(1.5);
        b.tip.images = vec![whole()];
        b.tip.selection = selection;
        codes(&ten_dabs(&b), 10)
    };
    let all_whole = some(&[0; 10]);
    for selection in [
        TipSelection::Sequential,
        TipSelection::PingPong,
        TipSelection::Hold,
    ] {
        assert_eq!(one(selection), all_whole, "{selection:?}");
    }
    for selection in [TipSelection::Once, TipSelection::Shuffle] {
        let got = one(selection);
        assert_eq!(got[0], Some(0), "{selection:?}");
        assert!(got[1..].iter().all(Option::is_none), "{selection:?}");
    }
}

/// 全部の画素が同じバイトか（ダブの置き方の乱数の列が動いていないことの確かめに使う）。
fn bytes(d: &Document) -> Vec<u8> {
    d.composite(d.bounds()).unwrap()
}

/// ゆらぎ・散布・数のある、重ならないダブ。左右に対称な筆先だけを使い、反転が見た目を変えないようにする。
fn jittered(selection: TipSelection, rule: FlipRule) -> Brush {
    let mut b = base(1.5);
    b.seed = 7;
    b.tip.images = vec![
        tip("a", |x, _| (4..12).contains(&x)),
        tip("b", |x, y| x != 8 || y < 12),
    ];
    // 左右に対称な筆先は 1 枚目だけ。2 枚目は上下だけ見る
    b.tip.images[1] = tip("b", |x, _| (2..14).contains(&x));
    b.tip.selection = selection;
    b.tip.flip_x = rule;
    b.jitter.size = 0.3;
    b.jitter.angle = 0.0;
    b.jitter.opacity = 0.3;
    b.jitter.scatter = 0.2;
    b.jitter.count = 2;
    b
}

#[test]
fn a_random_flip_draws_from_its_own_stream_and_leaves_the_other_random_draws_alone() {
    // 左右に対称な筆先なら、ランダムな反転の有無で絵は変わらない（ゆらぎ・散布・筆先の乱数選びの列を引く数が同じ）
    for selection in [TipSelection::Random, TipSelection::Sequential] {
        let plain = bytes(&ten_dabs(&jittered(selection, FlipRule::Off)));
        let flipped = bytes(&ten_dabs(&jittered(selection, FlipRule::Random)));
        assert!(plain.chunks_exact(4).any(|p| p[3] > 0));
        assert_eq!(plain, flipped, "{selection:?}");
    }
}

fn left_only(rule: FlipRule, seed: i32) -> Brush {
    let mut b = base(1.5);
    b.seed = seed;
    b.tip.images = vec![left_half(), left_half()];
    b.tip.flip_x = rule;
    b
}

fn flipped_left_to_right(d: &Document, n: usize) -> Vec<bool> {
    (0..n)
        .map(|k| match halves(d, k) {
            (true, false) => false,
            (false, true) => true,
            other => panic!("{k} 番目のダブが半分になっていない: {other:?}"),
        })
        .collect()
}

#[test]
fn flips_are_none_always_or_decided_per_dab_by_the_seed() {
    let n = 10;
    // なし・いつも反転（今までのとおり）
    let d = ten_dabs(&left_only(FlipRule::Off, 0));
    assert_eq!(flipped_left_to_right(&d, n), vec![false; n]);
    let d = ten_dabs(&left_only(FlipRule::On, 0));
    assert_eq!(flipped_left_to_right(&d, n), vec![true; n]);
    // ランダム: 種で決まり、反転するダブとしないダブが混ざる
    let random = |seed: i32| {
        let d = ten_dabs(&left_only(FlipRule::Random, seed));
        flipped_left_to_right(&d, n)
    };
    let mut seen = std::collections::HashSet::new();
    for seed in 0..12 {
        let got = random(seed);
        assert_eq!(got, random(seed), "同じ種は同じ反転（種 {seed}）");
        seen.insert(got);
    }
    assert!(seen.len() > 1, "種が変われば反転も変わる");
    let all: Vec<bool> = seen.iter().flatten().copied().collect();
    assert!(all.contains(&true) && all.contains(&false), "両方出る");
}

#[test]
fn flip_on_fold_flips_only_the_dabs_that_walk_back() {
    // 3 枚の折り返し 0 1 2 1 0 1 2 1 0 1: 戻る向き（前より小さい番号）のダブだけ反転する
    let mut b = base(1.5);
    b.tip.images = vec![left_half(), left_half(), left_half()];
    b.tip.selection = TipSelection::PingPong;
    b.tip.flip_x = FlipRule::OnFold;
    let d = ten_dabs(&b);
    let got = flipped_left_to_right(&d, 10);
    let expected = [
        false, false, false, true, true, false, false, true, true, false,
    ];
    assert_eq!(got, expected);
    // 折り返さない繰り返し方法では、戻る向きが無いので反転しない
    for selection in [TipSelection::Sequential, TipSelection::Hold] {
        b.tip.selection = selection;
        let d = ten_dabs(&b);
        assert_eq!(
            flipped_left_to_right(&d, 10),
            vec![false; 10],
            "{selection:?}"
        );
    }
}

#[test]
fn the_vertical_flip_follows_the_same_rules() {
    // 下半分の筆先: 反転すると上半分だけ塗れる
    let mut b = base(1.5);
    b.tip.images = vec![lower_half(), lower_half()];
    b.tip.flip_y = FlipRule::On;
    let d = ten_dabs(&b);
    for k in 0..10 {
        let x = X0 + STEP * k as f64;
        assert!(
            alpha(&d, x, Y + 2.0) > 0 && alpha(&d, x, Y - 2.0) == 0,
            "{k}"
        );
    }
    b.tip.flip_y = FlipRule::Random;
    b.seed = 5;
    let d = ten_dabs(&b);
    let up: Vec<bool> = (0..10)
        .map(|k| alpha(&d, X0 + STEP * k as f64, Y + 2.0) > 0)
        .collect();
    assert!(up.contains(&true) && up.contains(&false), "{up:?}");
}

/// 半径 8 の硬い丸を、2 つのダブが重なるように置く（間隔 0.5 × 直径 16 = 8 画素。線の長さ 8 で 2 つ）。
fn overlapping(flow: f64, darken: bool, pressures: (f64, f64)) -> Document {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 8.0,
            hardness: 1.0,
            spacing: 0.5,
            flow,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        yolu_core::PressureSwitches {
            size: false,
            opacity: true,
            flow: false,
        },
    );
    b.tip.darken_overlap = darken;
    let (mut d, l) = doc();
    let mut s = d.begin_brush_stroke(l, &b).unwrap();
    s.add_point(&mut d, 40.0, Y, pressures.0, DVec2::ZERO)
        .unwrap();
    s.add_point(&mut d, 48.0, Y, pressures.1, DVec2::ZERO)
        .unwrap();
    d.end_stroke(s).unwrap();
    d
}

#[test]
fn overlapping_dabs_keep_the_darker_value_instead_of_adding_up() {
    // 流量 0.5 の 2 つのダブ。足すと重なる所は 0.5 + 0.5 × 0.5 = 0.75、比較暗なら 0.5 のまま
    let single = alpha(&overlapping(0.5, true, (1.0, 1.0)), 33.0, Y);
    let overlap = |darken: bool| alpha(&overlapping(0.5, darken, (1.0, 1.0)), 44.0, Y);
    assert!(
        (127..=128).contains(&single),
        "1 つのダブだけの所: {single}"
    );
    assert_eq!(overlap(true), single, "比較暗: 重なる所も 1 つ分");
    assert!(overlap(false) > 180, "足す（今まで）: {}", overlap(false));
    // 濃さの違う（筆圧で不透明度が違う）2 つ: 重なる所は濃い方
    let faint_then_strong = overlapping(1.0, true, (0.4, 1.0));
    let strong_then_faint = overlapping(1.0, true, (1.0, 0.4));
    let (strong, faint) = (255u8, 102u8);
    assert_eq!(alpha(&faint_then_strong, 44.0, Y), strong);
    assert_eq!(alpha(&strong_then_faint, 44.0, Y), strong);
    assert!(alpha(&faint_then_strong, 33.0, Y).abs_diff(faint) <= 1);
    assert!(alpha(&strong_then_faint, 55.0, Y).abs_diff(faint) <= 1);
}

#[test]
fn without_darken_the_overlap_still_builds_up_by_the_old_formula() {
    // 流量 0.5 の 2 つのダブの重なり: 0.5 + (1 − 0.5) × 0.5 = 0.75（191）。比較暗が切なら、今までの寄せの式のまま
    let d = overlapping(0.5, false, (1.0, 1.0));
    assert_eq!(alpha(&d, 44.0, Y), 191);
    assert!((127..=128).contains(&alpha(&d, 33.0, Y)));
}

/// 横に 0 → 240 と増えていく 16 × 16 の質感（x の番号 × 16）。
fn ramp() -> Arc<BrushTip> {
    let alpha: Vec<u8> = (0..16 * 16).map(|i| (i % 16) as u8 * 16).collect();
    Arc::new(BrushTip::new("ramp", 16, 16, alpha).unwrap())
}

/// 周期 16 でなめらかに繰り返す質感（継ぎ目で値が飛ばない）。
fn waves() -> Arc<BrushTip> {
    let alpha: Vec<u8> = (0..16 * 16)
        .map(|i| {
            let (x, y) = ((i % 16) as f64, (i / 16) as f64);
            let t = std::f64::consts::TAU / 16.0;
            (128.0 + 100.0 * (x * t).sin() * (y * t).cos()) as u8
        })
        .collect();
    Arc::new(BrushTip::new("waves", 16, 16, alpha).unwrap())
}

fn textured(image: Arc<BrushTip>, angle: f64) -> Document {
    let mut b = base(0.1);
    b.base.radius = 14.0;
    let mut t = PaperTexture::new(image, 1.0);
    t.angle = angle;
    b.texture = Some(t);
    stroke(&b, (30.0, Y), (130.0, Y))
}

#[test]
fn a_turned_texture_runs_its_ramp_along_the_turned_axis() {
    // 横に増える質感を反時計回りに回す: 0° は右へ、90° は上へ（y が大きい側へ）、180° は左へ、-90° は下へ増える。
    // 周期 16 の 1 周の中（継ぎ目をまたがない 2 点）で比べる
    let at = |angle: f64, x: f64, y: f64| alpha(&textured(ramp(), angle), x, y);
    assert!(at(0.0, 68.0, Y) > at(0.0, 65.0, Y), "0°: 右へ増える");
    assert!(at(180.0, 68.0, Y) < at(180.0, 65.0, Y), "180°: 左へ増える");
    // y 方向: x = 64 の列で、y = 18..24 は質感の 2..8 番目（90°）。-90° は逆向き（16 - y の側）
    assert!(
        at(90.0, 64.0, 22.0) > at(90.0, 64.0, 18.0),
        "90°: 上へ増える"
    );
    assert!(
        at(-90.0, 64.0, 22.0) < at(-90.0, 64.0, 18.0),
        "-90°: 下へ増える"
    );
    // 0° の絵は y によらない（縦じまの向きのまま）
    let plain = textured(ramp(), 0.0);
    assert_eq!(alpha(&plain, 64.0, 18.0), alpha(&plain, 64.0, 22.0));
}

#[test]
fn a_turned_texture_has_no_seam_where_the_tiles_meet() {
    // 継ぎ目なく繰り返す質感は、どの角度でも隣の画素との差が滑らかな変化の範囲に収まる（回した画像が並ぶのでなく、読む位置を回す）
    let step = |angle: f64| -> u8 {
        let b = bytes(&textured(waves(), angle));
        let row =
            |y: usize| -> Vec<u8> { (40..120).map(|x| b[(y * W as usize + x) * 4 + 3]).collect() };
        (14..19)
            .flat_map(|y| {
                let r = row(y);
                r.windows(2)
                    .map(|w| w[0].abs_diff(w[1]))
                    .collect::<Vec<_>>()
            })
            .max()
            .unwrap()
    };
    let flat = step(0.0);
    assert!(flat < 60, "{flat}");
    for angle in [11.0, 37.0, 143.0] {
        let turned = step(angle);
        assert!(turned < 60, "{angle}°: {turned}");
    }
}

#[test]
fn a_turned_texture_paints_the_same_bytes_every_time() {
    let a = bytes(&textured(waves(), 143.0));
    assert_eq!(a, bytes(&textured(waves(), 143.0)));
    assert_ne!(a, bytes(&textured(waves(), 0.0)));
}

#[test]
fn a_huge_finite_texture_angle_turns_like_its_remainder_of_a_full_turn() {
    // 度からラジアンへの掛け算で溢れる大きさの角度（有限なので受けられる）も、360 度で割った余りの角度と同じバイトで描く
    let reference = bytes(&textured(waves(), 143.0));
    assert_ne!(reference, bytes(&textured(waves(), 0.0)));
    for angle in [360.0 * 5.0 + 143.0, -360.0 * 3.0 + 143.0] {
        assert_eq!(bytes(&textured(waves(), angle)), reference, "{angle}");
    }
    for angle in [f64::MAX, -f64::MAX, 1e300, f64::MAX / 3.0] {
        let turned = bytes(&textured(waves(), angle));
        assert_eq!(turned, bytes(&textured(waves(), angle % 360.0)), "{angle}");
        assert!(turned.chunks_exact(4).any(|p| p[3] > 0), "{angle}");
    }
}
