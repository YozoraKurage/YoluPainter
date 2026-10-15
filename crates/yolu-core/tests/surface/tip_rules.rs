//! 筆先の繰り返し方法・反転のしかた・ダブの重なりの比較暗・質感の回転の 3D: カメラに正面を向けた平らな四角（UV が文書の全体）で、2D と同じ
//! 設定・同じ点の列を描く。筆先の選びと反転は 2D と同じ関数が決める（同じ種なら同じ並び）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use std::sync::Arc;

use yolu_core::geometry::{SurfaceStroke, SurfaceStrokeOptions};
use yolu_core::glam::Vec2;
use yolu_core::{
    Brush, BrushSettings, BrushTip, FlipRule, PaperTexture, PressureSwitches, Rgba8, TipSelection,
};

use crate::anti_alias::{bytes, document, front, quad, screen, W};

const STEP: f64 = 12.0;
const X0: f64 = 10.0;
const Y: f64 = 16.0;

fn base(spacing: f64) -> Brush {
    Brush::simple(
        BrushSettings {
            radius: 4.0,
            hardness: 1.0,
            spacing,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    )
}

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

/// 四角に点の列を描いた、アルファ（行は `bytes` の順、1 行 `W` 画素）。半径は 2D と同じ大きさ（文書の画素）で渡す。
fn draw(b: &Brush, points: &[(f64, f64)]) -> Vec<u8> {
    let g = quad();
    let (mut d, l) = document(1);
    let mut b = b.clone();
    b.base.radius *= d.width() as f64 / g.brush_scale() as f64;
    let mut stroke = d.begin_brush_stroke(l, &b).unwrap();
    let at = |p: &(f64, f64)| -> Vec2 { screen(p.0, p.1) };
    let mut s = SurfaceStroke::begin_with_options(
        &mut d,
        &mut stroke,
        g,
        front(),
        &b.base,
        Some(0),
        at(&points[0]),
        1.0,
        SurfaceStrokeOptions::default(),
    )
    .unwrap();
    for p in &points[1..] {
        s.add(&mut d, &mut stroke, at(p), 1.0).unwrap();
    }
    s.finish(&mut d, &mut stroke).unwrap();
    d.end_stroke(stroke).unwrap();
    bytes(&d, l)
        .chunks_exact(4)
        .map(|p| p[3])
        .collect::<Vec<u8>>()
}

fn line(b: &Brush, dabs: usize) -> Vec<u8> {
    draw(b, &[(X0, Y), (X0 + STEP * (dabs - 1) as f64, Y)])
}

/// いちばん多く塗れている行。
fn busiest_row(a: &[u8]) -> usize {
    let w = W as usize;
    (0..a.len() / w)
        .max_by_key(|&y| a[y * w..(y + 1) * w].iter().filter(|&&v| v > 0).count())
        .unwrap()
}

/// その行で、塗れた画素の連なりの数。
fn runs(a: &[u8], row: usize) -> usize {
    let w = W as usize;
    let r = &a[row * w..(row + 1) * w];
    (0..w)
        .filter(|&x| r[x] > 0 && (x == 0 || r[x - 1] == 0))
        .count()
}

/// k 番目のダブの中心（x = X0 + STEP × k）の左右の半分の塗れ方。
fn halves(a: &[u8], row: usize, k: usize) -> (bool, bool) {
    let w = W as usize;
    let x = (X0 + STEP * k as f64) as usize;
    (a[row * w + x - 2] > 0, a[row * w + x + 2] > 0)
}

fn with(selection: TipSelection) -> Brush {
    let mut b = base(1.5);
    b.tip.images = vec![
        tip("a", |_, _| true),
        tip("b", |_, _| true),
        tip("c", |_, _| true),
    ];
    b.tip.selection = selection;
    b
}

#[test]
fn the_repeat_methods_that_stop_leave_only_the_tips_they_use_on_the_surface() {
    // 7 つのダブ。全部の筆先を埋めた四角にして、置かれたダブの数を連なりの数で数える
    let count = |selection: TipSelection| {
        let a = line(&with(selection), 7);
        runs(&a, busiest_row(&a))
    };
    for selection in [
        TipSelection::Random,
        TipSelection::Sequential,
        TipSelection::PingPong,
        TipSelection::Hold,
    ] {
        assert_eq!(count(selection), 7, "{selection:?}");
    }
    for selection in [TipSelection::Once, TipSelection::Shuffle] {
        assert_eq!(count(selection), 3, "{selection:?}: 一巡で止まる");
    }
}

/// 左半分の筆先 2 枚。反転すると、ダブの中心の右側が塗れる。
fn left_halves(rule: FlipRule, fixed: bool, selection: TipSelection, seed: i32) -> Brush {
    let mut b = base(1.5);
    b.seed = seed;
    let left = tip("left", |x, _| x < 8);
    b.tip.images = vec![left.clone(), left.clone(), left];
    b.tip.selection = selection;
    b.tip.flip_x = fixed;
    b.tip.flip_x_rule = rule;
    b
}

#[test]
fn flip_rules_choose_the_side_per_dab_on_the_surface() {
    let n = 7; // 四角の幅（96）に収まる数
    let side = |b: &Brush| -> Vec<(bool, bool)> {
        let a = line(b, n);
        let row = busiest_row(&a);
        (0..n).map(|k| halves(&a, row, k)).collect()
    };
    let none = side(&left_halves(
        FlipRule::Fixed,
        false,
        TipSelection::Sequential,
        0,
    ));
    let always = side(&left_halves(
        FlipRule::Fixed,
        true,
        TipSelection::Sequential,
        0,
    ));
    assert!(none.iter().all(|&h| h == none[0]));
    assert!(none[0] != (true, true) && none[0] != (false, false));
    assert_eq!(
        always,
        none.iter().map(|&(l, r)| (r, l)).collect::<Vec<_>>()
    );
    // ランダム: 種で決まり、両方の向きが出る。2D と同じ並び（同じ関数と同じ列で決める）
    let random = |seed: i32| {
        side(&left_halves(
            FlipRule::Random,
            false,
            TipSelection::Sequential,
            seed,
        ))
    };
    let mut seen = std::collections::HashSet::new();
    for seed in 0..8 {
        let got = random(seed);
        assert_eq!(got, random(seed), "種 {seed}");
        seen.insert(got);
    }
    assert!(seen.len() > 1);
    let flipped: Vec<bool> = seen.iter().flatten().map(|&h| h != none[0]).collect();
    assert!(flipped.contains(&true) && flipped.contains(&false));
    // 折り返しの戻り: 0 1 2 1 0 1 2 のうち、戻る向き（3・4 番目）だけ反転
    let fold = side(&left_halves(
        FlipRule::OnFold,
        false,
        TipSelection::PingPong,
        0,
    ));
    for (k, h) in fold.iter().enumerate() {
        let back = matches!(k, 3 | 4);
        assert_eq!(*h != none[0], back, "{k} 番目");
    }
}

fn overlapping(darken: bool) -> Vec<u8> {
    let mut b = Brush::simple(
        BrushSettings {
            radius: 8.0,
            hardness: 1.0,
            spacing: 0.0625,
            flow: 0.3,
            color: Rgba8::new(0, 0, 0, 255),
            ..BrushSettings::default()
        },
        PressureSwitches::NONE,
    );
    b.tip.darken_overlap = darken;
    draw(&b, &[(20.0, 30.0), (70.0, 30.0)])
}

#[test]
fn overlapping_dabs_do_not_add_up_when_they_keep_the_darker_value() {
    // 流量 0.3 の細かい間隔: 足せば、線の真ん中は天井（255）へ近づく。比較暗なら 0.3 × 255 ≒ 77 で頭打ち
    let darker = overlapping(true);
    let added = overlapping(false);
    let max = |a: &[u8]| *a.iter().max().unwrap();
    assert!(max(&added) > 200, "{}", max(&added));
    assert!(max(&darker) <= 78 && max(&darker) >= 76, "{}", max(&darker));
}

#[test]
fn a_turned_texture_runs_its_ramp_along_the_turned_axis_on_the_surface() {
    let ramp = || {
        let alpha: Vec<u8> = (0..16 * 16).map(|i| (i % 16) as u8 * 16).collect();
        Arc::new(BrushTip::new("ramp", 16, 16, alpha).unwrap())
    };
    let paint = |angle: f64| {
        let mut b = base(0.1);
        b.base.radius = 14.0;
        let mut t = PaperTexture::new(ramp(), 1.0);
        t.angle = angle;
        b.texture = Some(t);
        draw(&b, &[(20.0, 32.0), (76.0, 32.0)])
    };
    let w = W as usize;
    // 面の画素の並びは行・列の向きが上下逆でも、ランプの向きは行（列）の増える向きに対して決まる: 0° は x へ、90° は行の向きへ
    let at = |a: &[u8], x: usize, y: usize| a[y * w + x];
    let flat = paint(0.0);
    let row = busiest_row(&flat);
    assert!(
        at(&flat, 54, row) > at(&flat, 51, row),
        "0°: x の増える向きへ増える"
    );
    let turned = paint(90.0);
    let (up, down) = (row - 4, row + 4);
    // 90°: 質感は y（世界の y。面の画素の行とは逆向きかもしれない）に沿って増える。0° の x に沿った増え方とは違う
    assert_eq!(
        at(&turned, 50, row),
        at(&turned, 54, row),
        "90°: x に沿っては変わらない"
    );
    assert_ne!(at(&turned, 50, up), at(&turned, 50, down));
}
