//! 記録した点の列を流し直す: ペンの `PenSample` の列（筆圧・傾き・回転・時刻・接触・消しゴムの端・サイドボタン）とマウスのイベントの列を、
//! 2D のキャンバスと 3D ビューに 1 点ずつ（1 フレームに 1 点）流し、描いたレイヤーの画素の SHA-256・取り消しの段の数・未保存の印が
//! 決まった値になることを確かめる。点の時刻・筆圧の調整・離しの取りこぼし・修飾キーの扱いが入力の道で変われば、値が変わって落ちる。
//!
//! 期待のハッシュは、Linux（glibc 2.35 のコンテナ）で今のコードを回して得た値。ほかの glibc・OS で同じになるかは確かめていない
//! （筆圧・傾き・カメラ・筆先の三角関数と pow は libm を通る）。そのため決まった値との比べは `cfg!(target_os = "linux")` のときだけ回し、
//! 段の数・未保存の印・値を消した列との比べ・筆圧の調整の見分けはどの OS でも回す。文書の画素は CPU で塗るので GPU には依らない。
//! 描き方を意図して変えたときは、落ちた場面の値を、失敗の文に出る行で書き換える。場面はペンが主（40 のうち 36）。筆圧・傾き・回転・速さの列は、
//! その値を消した同じ列と画素が違うことも確かめる（値が描き方に届いていない場面を、決まった値だけで通さない）。
//! OS のペンの読み（Windows Ink・WinTab・mac のタブレット）は通らない（点の列は受け口へ直に入れる）。
use crate::common;
use crate::common::drive::Drive;
use crate::parity_input::{hold, world};
use crate::view3d_brush::{cube_view, press_with, release_with};

use common::*;
use egui::{Event, Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::{Channel, Rgba8, Tilt};
use yolu_app::pen::adjust::PressureAdjust;
use yolu_app::pen::PenSample;
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Canvas,
    View3d,
}

/// 流した結果: 描いたレイヤー（選択ペンは選択範囲）の SHA-256・取り消しの段の数・未保存の印。
#[derive(Clone, Debug, PartialEq)]
struct Replayed {
    hash: String,
    strokes: usize,
    modified: bool,
}

/// 期待の値（今のコードで回して得た値）。
struct Expect {
    hash: &'static str,
    strokes: usize,
    modified: bool,
}

#[track_caller]
fn check(name: &str, got: Replayed, want: Expect) {
    // 段の数と未保存の印は、どの OS でも同じ。ハッシュの定数は libm に依るので Linux（取った環境）だけで比べる
    assert!(
        got.strokes == want.strokes && got.modified == want.modified,
        "{name} の段の数・未保存の印が変わった: 段 {}（期待 {}）・未保存 {}（期待 {}）",
        got.strokes,
        want.strokes,
        got.modified,
        want.modified
    );
    if cfg!(target_os = "linux") {
        assert!(
            got.hash == want.hash,
            "{name} の画素のハッシュが変わった。意図した変更なら書き換える:\n    \"{}\", {}, {}",
            got.hash,
            got.strokes,
            got.modified
        );
    }
}

/// 同じ文書（試しの立方体を読んだ 256 角の文書・レイヤー 1 つ）を、2D のキャンバスか 3D ビューで前に出す。ブラシは乱数を使わず、
/// 筆圧で大きさと不透明度を変える。
fn open(view: View) -> (H, Rect) {
    let (mut h, rect) = cube_view();
    {
        let s = h.app_mut();
        s.m2_mut().random_seed = false;
        s.brush_mut().radius = 6.0;
        s.brush_mut().hardness = 0.7;
        s.brush_mut().spacing = 0.1;
        s.brush_mut().pressure.size = true;
        s.brush_mut().pressure.opacity = true;
        s.brush_mut().pressure.flow = false;
        s.color_mut().set_main([0.1, 0.3, 0.8, 1.0]);
    }
    match view {
        View::View3d => (h, rect),
        View::Canvas => {
            click_tab(&mut h, Tab::Canvas);
            h.run();
            let rect = canvas_rect(&h);
            (h, rect)
        }
    }
}

/// 点（`x`・`y` は −1〜1）: 3D は手前の面（−Z）の上、2D は文書の真ん中のまわり（文書の幅の ±35%）。
fn spot(h: &H, view: View, rect: Rect, x: f32, y: f32) -> Pos2 {
    match view {
        View::View3d => world(h, rect, x * 0.4, y * 0.4),
        View::Canvas => {
            let s = h.app();
            let (w, hh) = (s.doc().width(), s.doc().height());
            s.view().view(rect, w, hh).to_screen(
                w as f64 * (0.5 + 0.35 * x as f64),
                hh as f64 * (0.5 + 0.35 * y as f64),
            )
        }
    }
}

/// 左から右への波打つ道の `t`（0〜1）の点。
fn along(h: &H, view: View, rect: Rect, t: f32) -> Pos2 {
    spot(
        h,
        view,
        rect,
        -0.8 + 1.6 * t,
        -0.2 + 0.4 * t + 0.25 * (t * std::f32::consts::TAU * 1.5).sin(),
    )
}

/// ペンの点（接触・筆圧・時刻。傾き・回転・消しゴムの端・サイドボタンは無し）。
fn pen(at: Pos2, pressure: f32, time_ms: u32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure,
        tilt: Tilt::default(),
        rotation: None,
        contact: true,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    }
}

/// 離す点（最後の点の位置・時刻のあと）。
fn lift(last: &PenSample) -> PenSample {
    PenSample {
        contact: false,
        pressure: 0.0,
        time_ms: last.time_ms + 8,
        ..*last
    }
}

/// 点の列を 1 フレームに 1 点ずつ流す（最後に離す）。
fn feed(h: &mut H, samples: &[PenSample]) {
    for s in samples {
        h.state().pen().push(*s);
        h.step();
    }
    let end = lift(samples.last().unwrap());
    h.state().pen().push(end);
    h.step();
}

/// 離したあと、3D の持ち越しを塗り終えて確定するまで回す。
fn settle(h: &mut H) {
    let mut frames = 0;
    while h.app().is_stroking() && frames < 3000 {
        h.step();
        frames += 1;
    }
    assert!(!h.app().is_stroking(), "確定しない");
    h.run();
}

/// 描いたレイヤー（選んでいるレイヤー）の色のチャンネルの SHA-256（幅・高さと、行ごとの straight RGBA8）。
fn layer_hash(h: &H) -> String {
    use sha2::{Digest, Sha256};
    let doc = h.doc();
    let (w, hh) = (doc.width(), doc.height());
    let mut d = Sha256::new();
    d.update(w.to_le_bytes());
    d.update(hh.to_le_bytes());
    let layer = doc.layer(h.selected_layer().expect("レイヤー")).unwrap();
    let surface = layer.surface(Channel::Color).expect("色のチャンネル");
    let mut row = vec![0u8; w as usize * 4];
    for y in 0..hh {
        surface.read_row(0, y, &mut row).unwrap();
        d.update(&row);
    }
    hex::encode(d.finalize())
}

/// 選択範囲の SHA-256（無ければ「無し」の印の SHA-256）。
fn selection_hash(h: &H) -> String {
    use sha2::{Digest, Sha256};
    let mut d = Sha256::new();
    match h.doc().selection() {
        Some(m) => {
            d.update(m.width().to_le_bytes());
            d.update(m.height().to_le_bytes());
            d.update(m.to_canvas_bytes());
        }
        None => d.update(b"no selection"),
    }
    hex::encode(d.finalize())
}

fn replayed(h: &H) -> Replayed {
    Replayed {
        hash: layer_hash(h),
        strokes: h.doc().undo_count(),
        modified: h.modified(),
    }
}

/// 選んでいるレイヤーの全面を不透明な色で塗る（消しゴムの場面の下地）。履歴は消す。
fn solid(h: &mut H) {
    let layer = h.selected_layer().unwrap();
    let size = h.doc().width();
    let doc = h.doc_mut();
    for y in 0..size {
        for x in 0..size {
            doc.set_pixel(layer, x, y, Rgba8::new(200, 60, 40, 255))
                .unwrap();
        }
    }
    doc.clear_history().unwrap();
}

// ───────── ブラシの点の列 ─────────

/// ペンの点の列の種類（ブラシで流す）。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Curve {
    /// 筆圧が波打つ（0.15〜1）。
    Pressure,
    /// 傾きが変わる（傾きで大きさが変わり、倒れた向きを筆先の角度に足す、潰した筆先）。
    Tilt,
    /// ペンの軸の回転が変わる（回転を筆先の角度に足す、潰した筆先）。
    Rotation,
    /// 点の間の時刻が 4 ms から 34 ms へ伸びる（速さで大きさが変わるブラシ）。
    Speed,
    /// 入り抜き: 筆圧が 0 から上がって 0 へ下がる。
    Taper,
    /// とても速い: 6 点を 2 ms 刻みで（速さのブラシ）。
    Fast,
    /// とても遅い: 40 点を 60 ms 刻みで（速さのブラシ）。
    Slow,
    /// 点の間が飛ぶ: 前の 3 分の 1 と後ろの 3 分の 1 だけ（間の点が無い）。
    Gap,
}

/// その種類のブラシの設定。
fn brush_for(h: &mut H, curve: Curve) {
    let b = &mut h.m2_mut().brush;
    match curve {
        Curve::Tilt => {
            b.influences.size.tilt.on = true;
            b.controls.tilt_angle = true;
            b.tip.roundness = 0.3;
        }
        Curve::Rotation => {
            b.controls.rotation_angle = true;
            b.tip.roundness = 0.3;
        }
        Curve::Speed | Curve::Fast | Curve::Slow => {
            b.influences.size.speed.on = true;
            b.influences.speed_max = 900.0;
        }
        _ => {}
    }
}

/// その種類の点の列。`neutral` なら、その種類の値を消した列（筆圧は一定、傾きは 0、回転は無し、時刻は等しい刻み）。
fn curve_samples(h: &H, view: View, rect: Rect, curve: Curve, neutral: bool) -> Vec<PenSample> {
    let n = match curve {
        Curve::Fast => 6,
        Curve::Slow => 40,
        _ => 24,
    };
    let mut time = 0u32;
    let mut out = Vec::new();
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        if curve == Curve::Gap && (0.34..0.66).contains(&t) {
            continue;
        }
        let dt = match curve {
            Curve::Speed if !neutral => 4 + (30.0 * t) as u32,
            Curve::Speed => 19,
            Curve::Fast => 2,
            Curve::Slow => 60,
            _ => 8,
        };
        let pressure = match curve {
            Curve::Pressure if neutral => 0.6,
            Curve::Pressure => 0.15 + 0.85 * (0.5 + 0.5 * (t * std::f32::consts::TAU * 2.0).sin()),
            Curve::Taper => (t * std::f32::consts::PI).sin().max(0.0),
            _ => 0.8,
        };
        let mut s = pen(along(h, view, rect, t), pressure, time);
        if curve == Curve::Tilt && !neutral {
            s.tilt = Tilt {
                x: -50.0 + 100.0 * t,
                y: 35.0 * (t * std::f32::consts::TAU).sin(),
            };
        }
        if curve == Curve::Rotation && !neutral {
            s.rotation = Some(540.0 * t);
        }
        out.push(s);
        time += dt;
    }
    if curve == Curve::Taper {
        let last = out.len() - 1;
        out[0].pressure = 0.0;
        out[last].pressure = 0.0;
    }
    out
}

/// ブラシで、その種類の点の列を流す。
fn brush_run(view: View, curve: Curve, neutral: bool) -> Replayed {
    let (mut h, rect) = open(view);
    brush_for(&mut h, curve);
    let samples = curve_samples(&h, view, rect, curve, neutral);
    feed(&mut h, &samples);
    settle(&mut h);
    assert!(
        h.message().is_empty(),
        "{view:?} {curve:?}: {}",
        h.message()
    );
    replayed(&h)
}

/// ブラシで流し、筆圧・傾き・回転・速さの列は、その値を消した列と画素が違うことも確かめる。
fn brush(view: View, curve: Curve) -> Replayed {
    let got = brush_run(view, curve, false);
    if matches!(
        curve,
        Curve::Pressure | Curve::Tilt | Curve::Rotation | Curve::Speed
    ) {
        let plain = brush_run(view, curve, true);
        assert_ne!(
            got.hash, plain.hash,
            "{view:?} {curve:?}: この値が描き方に届いている"
        );
    }
    got
}

// ───────── 消しゴム・ペンの端・ボタン ─────────

/// 消しゴムのツールで、筆圧の波打つ列（下地を塗った上で）。
fn eraser_tool(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    solid(&mut h);
    h.act(Action::SelectTool(Tool::Eraser));
    h.run();
    let samples = curve_samples(&h, view, rect, Curve::Pressure, false);
    feed(&mut h, &samples);
    settle(&mut h);
    replayed(&h)
}

/// ブラシのツールのまま、ペンの消しゴムの端で（下地を塗った上で）。
fn eraser_end(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    solid(&mut h);
    let samples: Vec<PenSample> = curve_samples(&h, view, rect, Curve::Pressure, false)
        .into_iter()
        .map(|s| PenSample { eraser: true, ..s })
        .collect();
    feed(&mut h, &samples);
    settle(&mut h);
    replayed(&h)
}

/// サイドボタンを押したままなぞる（2D はスポイト、3D は回す。どちらも描かない）。
fn barrel(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    let samples: Vec<PenSample> = curve_samples(&h, view, rect, Curve::Pressure, true)
        .into_iter()
        .map(|s| PenSample { barrel: true, ..s })
        .collect();
    feed(&mut h, &samples);
    settle(&mut h);
    replayed(&h)
}

/// 浮かせたまま（接触なし）なぞってから、触れてすぐ離す（短いタップ）。
fn hover_then_tap(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    for i in 0..10 {
        let t = i as f32 / 9.0;
        let s = PenSample {
            contact: false,
            pressure: 0.0,
            ..pen(along(&h, view, rect, t), 0.0, i * 8)
        };
        h.state().pen().push(s);
        h.step();
    }
    let tap = pen(along(&h, view, rect, 1.0), 0.7, 100);
    feed(&mut h, &[tap]);
    settle(&mut h);
    replayed(&h)
}

/// 短い線を描いてから、Shift を押したまま別の所に触れて離す（前の終点からの直線）。
fn pen_shift_line(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    let first: Vec<PenSample> = (0..4)
        .map(|i| pen(along(&h, view, rect, i as f32 * 0.02), 0.8, i * 8))
        .collect();
    feed(&mut h, &first);
    settle(&mut h);
    hold(&mut h, Modifiers::SHIFT);
    let end = pen(along(&h, view, rect, 1.0), 0.8, 200);
    feed(&mut h, &[end]);
    hold(&mut h, Modifiers::NONE);
    settle(&mut h);
    replayed(&h)
}

/// Ctrl を押したままペンでなぞる（描くツールの Ctrl は、ペンでは描かない）。
fn pen_ctrl(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    hold(&mut h, Modifiers::CTRL);
    let samples = curve_samples(&h, view, rect, Curve::Pressure, true);
    feed(&mut h, &samples);
    hold(&mut h, Modifiers::NONE);
    settle(&mut h);
    replayed(&h)
}

/// なぞっている途中でウィンドウのフォーカスを失い（離しを受け取れない）、そのまま触れた点が続いてから離す。
fn focus_lost(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    let samples = curve_samples(&h, view, rect, Curve::Pressure, false);
    let (before, after) = samples.split_at(12);
    for s in before {
        h.state().pen().push(*s);
        h.step();
    }
    h.event(Event::WindowFocused(false));
    h.step();
    h.event(Event::WindowFocused(true));
    h.step();
    feed(&mut h, after);
    settle(&mut h);
    replayed(&h)
}

/// 3D: 1 フレームに 1 ダブの枠で描いて離し、持ち越しが残っている間に次の線を触れ始める（先に前の線を確定してから、次の線を受ける）。
fn settle_then_next_press() -> Replayed {
    let (mut h, rect) = open(View::View3d);
    h.view3d_mut().input.paint_budget = Some(std::time::Duration::ZERO);
    let first: Vec<PenSample> = (0..10)
        .map(|i| pen(along(&h, View::View3d, rect, i as f32 / 20.0), 0.9, i * 8))
        .collect();
    feed(&mut h, &first);
    assert!(h.view3d().input.released, "前提: 離した後も持ち越しが残る");
    let second: Vec<PenSample> = (10..20)
        .map(|i| {
            pen(
                along(&h, View::View3d, rect, i as f32 / 20.0),
                0.5,
                300 + i * 8,
            )
        })
        .collect();
    feed(&mut h, &second);
    settle(&mut h);
    replayed(&h)
}

/// 全体の筆圧の調整（下限 0.25・上限 0.75）を入れて、筆圧の波打つ列を流す。調整が 1 度だけ当たることを、2 の冪の値で確かめる:
/// 調整した 0.375 は、調整しない 0.25 と同じ線で、調整しない 0.375 とは違う線。
fn adjusted(view: View) -> Replayed {
    let adjust = || PressureAdjust::new(0.25, 0.75, vec![]).unwrap();
    let flat = |adjust: PressureAdjust, pressure: f32| {
        let (mut h, rect) = open(view);
        h.prefs_mut().settings.pressure = adjust;
        let samples: Vec<PenSample> = curve_samples(&h, view, rect, Curve::Pressure, true)
            .into_iter()
            .map(|s| PenSample { pressure, ..s })
            .collect();
        feed(&mut h, &samples);
        settle(&mut h);
        layer_hash(&h)
    };
    let plain = flat(PressureAdjust::default(), 0.25);
    assert_eq!(
        flat(adjust(), 0.375),
        plain,
        "{view:?}: 調整は 1 度だけ当たる"
    );
    assert_ne!(flat(PressureAdjust::default(), 0.375), plain, "{view:?}");
    let (mut h, rect) = open(view);
    h.prefs_mut().settings.pressure = adjust();
    let samples = curve_samples(&h, view, rect, Curve::Pressure, false);
    feed(&mut h, &samples);
    settle(&mut h);
    replayed(&h)
}

// ───────── マウス ─────────

/// マウスの左ボタンで波打つ道をなぞる（修飾 `m`）。
fn mouse_line(h: &mut H, view: View, rect: Rect, from: f32, to: f32, points: usize, m: Modifiers) {
    hold(h, m);
    let at = |h: &H, i: usize| {
        along(
            h,
            view,
            rect,
            from + (to - from) * i as f32 / (points - 1).max(1) as f32,
        )
    };
    let start = at(h, 0);
    press_with(h, start, m);
    h.step();
    for i in 1..points {
        let p = at(h, i);
        move_to(h, p);
        h.step();
    }
    let end = at(h, points - 1);
    release_with(h, end, m);
    h.step();
    hold(h, Modifiers::NONE);
    settle(h);
}

fn mouse_brush(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    mouse_line(&mut h, view, rect, 0.0, 1.0, 24, Modifiers::NONE);
    replayed(&h)
}

fn mouse_shift_line(view: View) -> Replayed {
    let (mut h, rect) = open(view);
    mouse_line(&mut h, view, rect, 0.0, 0.06, 4, Modifiers::NONE);
    mouse_line(&mut h, view, rect, 1.0, 1.0, 1, Modifiers::SHIFT);
    replayed(&h)
}

// ───────── 図形・バケツ・選択ペン（2D） ─────────

fn tool_run(tool: Tool, samples: impl Fn(&H, Rect) -> Vec<PenSample>) -> H {
    let (mut h, rect) = open(View::Canvas);
    h.act(Action::SelectTool(tool));
    h.run();
    let samples = samples(&h, rect);
    feed(&mut h, &samples);
    settle(&mut h);
    h
}

fn pen_shape() -> Replayed {
    let h = tool_run(Tool::Shape, |h, rect| {
        (0..8)
            .map(|i| {
                pen(
                    spot(
                        h,
                        View::Canvas,
                        rect,
                        -0.5 + i as f32 * 0.14,
                        -0.4 + i as f32 * 0.1,
                    ),
                    0.8,
                    i * 8,
                )
            })
            .collect()
    });
    replayed(&h)
}

fn pen_bucket() -> Replayed {
    let h = tool_run(Tool::Fill, |h, rect| {
        vec![pen(spot(h, View::Canvas, rect, -0.6, -0.6), 0.8, 0)]
    });
    replayed(&h)
}

fn pen_selection() -> Replayed {
    let h = tool_run(Tool::SelectPen, |h, rect| {
        curve_samples(h, View::Canvas, rect, Curve::Pressure, false)
    });
    Replayed {
        hash: selection_hash(&h),
        strokes: h.doc().undo_count(),
        modified: h.modified(),
    }
}

// ───────── 場面 ─────────

macro_rules! replay {
    ($name:ident, $run:expr, $hash:literal, $strokes:literal, $modified:literal) => {
        #[test]
        fn $name() {
            check(
                stringify!($name),
                $run,
                Expect {
                    hash: $hash,
                    strokes: $strokes,
                    modified: $modified,
                },
            );
        }
    };
}

use Curve::*;
use View::{Canvas as V2, View3d as V3};

// ペン × ブラシ（筆圧・傾き・回転・速さ・入り抜き・とても速い・とても遅い・点の間が飛ぶ）× 2D・3D
replay!(
    pen_brush_pressure_2d,
    brush(V2, Pressure),
    "8df0989ee747bc69a93aea8c91e6b2bc05f5e2db6b3fb9561ba34b74e104bf6d",
    1,
    true
);
replay!(
    pen_brush_pressure_3d,
    brush(V3, Pressure),
    "8228a304a165db37b81561858ef3de4ca3970061e9b43de818f8ecd7bb6a5f93",
    1,
    true
);
replay!(
    pen_brush_tilt_2d,
    brush(V2, Tilt),
    "158fd1dc98c3e63fb960d8dfe5eee5e5769c70a4caf6dc52e89a4fb910e35d3a",
    1,
    true
);
replay!(
    pen_brush_tilt_3d,
    brush(V3, Tilt),
    "1c8296901f1a4959a67f7b034c96ca5a720363ff247a4923cbfdcd3fe4560e1d",
    1,
    true
);
replay!(
    pen_brush_rotation_2d,
    brush(V2, Rotation),
    "12acca0f219e18154bd53ed17b4759fd68c046fa39e8a29409b132b1edd9dbd6",
    1,
    true
);
replay!(
    pen_brush_rotation_3d,
    brush(V3, Rotation),
    "cfed3b399f9bd148a2471375b8f58603ac35aa4759f9cc8848ebb335eff80f0c",
    1,
    true
);
replay!(
    pen_brush_speed_2d,
    brush(V2, Speed),
    "03376cac48008219df176c6d464e3a39e7153e514bf39689ab475c4e09938903",
    1,
    true
);
replay!(
    pen_brush_speed_3d,
    brush(V3, Speed),
    "9585ee71073058d83807507b7d82a2de0ecd4d14dbf6523b5a60dd09dae2a847",
    1,
    true
);
replay!(
    pen_brush_taper_2d,
    brush(V2, Taper),
    "936e14789adb1d24d9ceea524d02a8b4b14cc1677c6dc48672727592012ece94",
    1,
    true
);
replay!(
    pen_brush_taper_3d,
    brush(V3, Taper),
    "36451b20263e4c8b91e532211a707f8494347a25682cba0c4db641728e07e026",
    1,
    true
);
replay!(
    pen_brush_fast_2d,
    brush(V2, Fast),
    "a20739592e3e8eb25f93de8473227c2566964484ce3e6e75236c31bc112005f3",
    1,
    true
);
replay!(
    pen_brush_fast_3d,
    brush(V3, Fast),
    "6ed24a983304f4512d644097587688ba4254ed76f2862b434fde5b001beac898",
    1,
    true
);
replay!(
    pen_brush_slow_2d,
    brush(V2, Slow),
    "5a5a6604065c2a6192ecab7c1723fe30b59804ae2990cae9853fc7e75eea31fc",
    1,
    true
);
replay!(
    pen_brush_slow_3d,
    brush(V3, Slow),
    "2fa88cd80a8b480f5f67a868d4d2928e3edb924728de9cc574114da793ca879f",
    1,
    true
);
replay!(
    pen_brush_gap_2d,
    brush(V2, Gap),
    "4e6f70f42f841cb72e7e7bd6502875b4b0515656eedbfaaf1032cd7d4863366f",
    1,
    true
);
replay!(
    pen_brush_gap_3d,
    brush(V3, Gap),
    "ffceeb3ea1576f69d5f28f438240d679d70bbe8104b082cfcc5ed961f32df9dd",
    1,
    true
);
// ペン × 消しゴム（ツールと、ペンの消しゴムの端）
replay!(
    pen_eraser_tool_2d,
    eraser_tool(V2),
    "fc715d6c97e4fadbcb6a2c54aa6c6ddcccc91d3ba39e5fa3242720970443ed7d",
    1,
    true
);
replay!(
    pen_eraser_tool_3d,
    eraser_tool(V3),
    "1ed44d4c6eaff1d5d804815f7c3a5d7cd411c78b2a6b8270f23f1eeb643d797c",
    1,
    true
);
replay!(
    pen_eraser_end_2d,
    eraser_end(V2),
    "5d71b76d299f9d1fd331701eee31293587c4cb94f10bfa46b242f13aca83b8a0",
    1,
    true
);
replay!(
    pen_eraser_end_3d,
    eraser_end(V3),
    "6bd2e2385252316f71f3e98dd0b951de3abe93461b95f9cd9a79ad913effb247",
    1,
    true
);
// ペンのサイドボタン・浮かせたまま動かしてタップ
replay!(
    pen_barrel_2d,
    barrel(V2),
    "6749f2fa0aa62269770cbff154c541ea8cc4207716afea9cd661eb3867096eed",
    0,
    false
);
replay!(
    pen_barrel_3d,
    barrel(V3),
    "6749f2fa0aa62269770cbff154c541ea8cc4207716afea9cd661eb3867096eed",
    0,
    false
);
replay!(
    pen_hover_then_tap_2d,
    hover_then_tap(V2),
    "db282e7a99dec67fbe3dfa3c31367f61684b565c089d2748444f68873cf6956e",
    1,
    true
);
replay!(
    pen_hover_then_tap_3d,
    hover_then_tap(V3),
    "1259face97c5ca8d37809bd0fc55923807f611d7972792736c1da40e3e42d1de",
    1,
    true
);
// ペンの修飾キー（Shift の直線・Ctrl を押したまま）
replay!(
    pen_shift_line_2d,
    pen_shift_line(V2),
    "e53098f2c51b5cfa7b1d0301a53fc12ae11a4c155853ca43277924c7f65325f5",
    2,
    true
);
replay!(
    pen_shift_line_3d,
    pen_shift_line(V3),
    "b57c9a2cef53c9c36a919df40d87ba085d92f2219f5f5b51c61252fcb702f604",
    2,
    true
);
replay!(
    pen_ctrl_2d,
    pen_ctrl(V2),
    "6749f2fa0aa62269770cbff154c541ea8cc4207716afea9cd661eb3867096eed",
    0,
    false
);
replay!(
    pen_ctrl_3d,
    pen_ctrl(V3),
    "6749f2fa0aa62269770cbff154c541ea8cc4207716afea9cd661eb3867096eed",
    0,
    false
);
// 途中でフォーカスを失う（取りこぼしの離し）・3D の確定待ちの間の次の押し
replay!(
    pen_focus_lost_2d,
    focus_lost(V2),
    "f162ccf4891aa003aad07862f7b0395f97ab74da030fd68c30ff689945cfa4bc",
    2,
    true
);
replay!(
    pen_focus_lost_3d,
    focus_lost(V3),
    "438aefbd39ed086e86d9118d6acae10389fc6578710717a571e0b8ac5726e8c4",
    2,
    true
);
replay!(
    pen_settle_then_next_press_3d,
    settle_then_next_press(),
    "ca47ce1c5c3bd9380581045da93799e2fce24ac3cc91b7364990e7a8a056e5f3",
    2,
    true
);
// 全体の筆圧の調整（既定でない）
replay!(
    pen_pressure_adjusted_2d,
    adjusted(V2),
    "c95fad31e73b436e04e36b3e5fbca30563731f12ada2b8ca9c8b37ffa2deaf5e",
    1,
    true
);
replay!(
    pen_pressure_adjusted_3d,
    adjusted(V3),
    "da9f1e90352bf12ee518aa20f2843cddf7525c58394c5b0ca8c9520ec627b8c0",
    1,
    true
);
// 図形・バケツ・選択ペン（2D のペン）
replay!(
    pen_shape_2d,
    pen_shape(),
    "faefc6c48e6c3da11416849b3be90e9eed0936753951f58b37bf2fb86ed19f05",
    1,
    true
);
replay!(
    pen_bucket_2d,
    pen_bucket(),
    "9ac97445be1f3e8e227b59fff59c414b726a1d31ad8ed85cd30f1683dff0e3cb",
    1,
    true
);
replay!(
    pen_selection_pen_2d,
    pen_selection(),
    "d35287aa66c4731080126ff99731494c6305e305475bd77a58c45f014203c3fc",
    1,
    true
);
// マウス: ブラシと Shift の直線
replay!(
    mouse_brush_2d,
    mouse_brush(V2),
    "d20507d0b03ca3f828e5a3a5170752c50d62afb116799558288416c1c23c59d8",
    1,
    true
);
replay!(
    mouse_brush_3d,
    mouse_brush(V3),
    "59b442656f806549490ca5c05b329553f6b78ffd683ca00e2ded6790d6f7d00c",
    1,
    true
);
replay!(
    mouse_shift_line_2d,
    mouse_shift_line(V2),
    "0bcabf0a9ea0aebbbf352ba54de44668ff224163728c2c03feb53d785482cf44",
    2,
    true
);
replay!(
    mouse_shift_line_3d,
    mouse_shift_line(V3),
    "fbdeb4904d0841fa007f3138b068fdd86329f651979dd611bd7e0a25802b6d32",
    2,
    true
);
