//! 筆圧の調整の窓（表示 → 筆圧の調整…）: 枠の中で普段の強さで何本か描くと、描いた線の筆圧の分布から下限・上限と曲線を自動で決める
//! （式は [`super::adjust`]）。下限・上限は 2 本のスライダーで直し、曲線は共通の編集の部品（`ui::curve::curve_editor`）で手で直す。
//! 曲線の枠には、描いた線の筆圧の分布を薄い棒で重ねる。「元に戻す」は窓を開いたときの調整へ、「既定」は直線へ戻す。
//! 描いた線は枠の中だけに持ち（文書には入らない）、窓を閉じると捨てる。決めた調整は設定（端末ごと）へ入り、ペンの筆圧をブラシへ渡す前に
//! 直す（`AppState::adjust_pressure`）。マウスの筆圧は 1 のまま、調整を通らない。
//!
//! 枠の中の筆圧は、Windows Ink（`PenSample`）か、ペンが egui の Touch の力として来る環境のどちらか（Windows Ink が使える間は
//! 同じ押しが Touch にも来るので、Touch は読まない）。どちらも調整を通す前の値を集めるので、調整を変えても集めた分布は変わらない。

use egui::{pos2, vec2, Color32, Id, Pos2, Rect, Stroke, Vec2};
use yolu_core::curve::Curve;

use super::adjust::{fit, FitError, PressureAdjust};
use super::PenSample;
use crate::state::{Action, AppState};
use crate::ui::curve;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, SliderSpec};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 380.0;
const GAP: f32 = 6.0;
/// 曲線の枠の高さ。
const CURVE_HEIGHT: f32 = 190.0;
/// 描く枠の高さ。
const DRAW_HEIGHT: f32 = 96.0;
/// 描いた線の本数・点の数の上限（古いものから捨てる。窓を開いたままの長い試しでメモリを増やさない）。点の数は、線をまたいで古い線から捨て、
/// 1 本が上限を超えて続くときはその線の古い点から捨てる。
pub const MAX_STROKES: usize = 64;
pub const MAX_DOTS: usize = 20_000;
/// 分布の棒の数。
const BINS: usize = 32;

fn window_id() -> Id {
    Id::new("yolu.pressure")
}

/// 最後に描いた窓の矩形（画面の点。開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// 枠の中で描いた点 1 つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    /// 枠の左上からの位置（画面の点）。
    pub pos: Vec2,
    /// ペンの筆圧（調整を通す前）。
    pub pressure: f32,
}

/// 窓の操作（`Action::Pressure`）。
#[derive(Clone, Debug, PartialEq)]
pub enum PressureAction {
    Open,
    Close,
    /// 下限・上限（動かしたほうが、相手から `MIN_SPAN` の所で止まる。相手は動かさない）。
    SetRange {
        low: f32,
        high: f32,
        moved_low: bool,
    },
    /// 曲線（編集の部品が返した、検査済みのもの。点を足す・動かす・消すが決まったときに出る）。
    SetCurve(Curve),
    /// 描いた線から決める。
    Fit,
    /// 描いた線を消す。
    Clear,
    /// 窓を開いたときの調整へ。
    Revert,
    /// 下限 0・上限 1・直線へ。
    Reset,
}

/// 窓の状態（描いた線・開いたときの調整・自動で決められなかった理由）。
#[derive(Debug, Default)]
pub struct PressureWindow {
    pub open: bool,
    offset: Vec2,
    opened_with: PressureAdjust,
    /// 描いた線（古い順）。
    pub strokes: Vec<Vec<Dot>>,
    /// 最後に描いた枠（画面の点）。ペンの点はこの中だけ集める（窓が毎フレーム決める。試験が決めてもよい）。
    pub frame: Option<Rect>,
    /// 最後に描いた曲線の枠（画面の点。窓が毎フレーム決める。試験が曲線を操作する位置を知るために読む）。
    pub curve_frame: Option<Rect>,
    /// 今描いている線のペン。
    drawing: Option<u32>,
    /// 自動で決められなかった理由（次に決められる・線を消すと消える）。
    pub note: Option<FitError>,
    /// 下限・上限のスライダーをドラッグしている最中。設定のファイルへは、離す（か窓を閉じる）まで調整を書かない（ドラッグの間じゅう
    /// フレームごとに同期付きの書き込みをしない。退避の数の `PrefsState::dragging` と同じ扱い）。
    pub dragging: bool,
}

impl PressureWindow {
    fn dots(&self) -> usize {
        self.strokes.iter().map(Vec::len).sum()
    }

    /// 調整を通す前の筆圧（0 を除く）の全部。
    pub fn samples(&self) -> Vec<f32> {
        self.strokes
            .iter()
            .flatten()
            .map(|d| d.pressure)
            .filter(|p| *p > 0.0)
            .collect()
    }

    fn record(&mut self, id: u32, pos: Pos2, pressure: f32) {
        let Some(frame) = self.frame else { return };
        let inside = frame.contains(pos);
        if !inside {
            self.drawing = None;
            return;
        }
        if self.drawing != Some(id) || self.strokes.is_empty() {
            self.drawing = Some(id);
            self.strokes.push(Vec::new());
        }
        if let Some(stroke) = self.strokes.last_mut() {
            stroke.push(Dot {
                pos: pos - frame.min,
                pressure,
            });
        }
        self.trim();
    }

    /// 本数と点の数の上限を守る。古い線から捨て、残る線が 1 本で点の数だけが超えるときは、その線の古い点から捨てる
    /// （今引いている線は最後の線で、まるごとは捨てない）。
    fn trim(&mut self) {
        if self.strokes.len() > MAX_STROKES {
            let extra = self.strokes.len() - MAX_STROKES;
            self.strokes.drain(..extra);
        }
        let mut excess = self.dots().saturating_sub(MAX_DOTS);
        while excess > 0 && !self.strokes.is_empty() {
            let oldest = self.strokes[0].len();
            if oldest <= excess && self.strokes.len() > 1 {
                excess -= oldest;
                self.strokes.remove(0);
            } else {
                self.strokes[0].drain(..excess.min(oldest));
                excess = 0;
            }
        }
    }

    /// ペンを離した・窓を閉じた: 次の接触は新しい線にする。
    fn lift(&mut self) {
        self.drawing = None;
    }
}

impl AppState {
    /// ペンの筆圧をブラシへ渡す値にする（設定の調整。既定なら筆圧そのもの）。マウスの筆圧（1）は通さない。
    pub fn adjust_pressure(&self, pressure: f32) -> f32 {
        self.prefs.settings.pressure.apply(pressure)
    }

    /// このフレームのペンの点（調整を通す前）を、窓が開いていれば枠の中の線として集める。
    pub fn pressure_observe(&mut self, pixels_per_point: f32, samples: &[PenSample]) {
        let window = &mut self.pressure;
        if !window.open {
            return;
        }
        for s in samples {
            if !s.contact {
                window.lift();
                continue;
            }
            window.record(s.pointer_id, s.pos_points(pixels_per_point), s.pressure);
        }
    }

    /// Windows Ink が使えない環境で、ペンが egui の Touch の力として来る点を集める（`pressure_observe` と同じ枠の線へ）。
    pub fn pressure_observe_touch(&mut self, events: &[egui::Event]) {
        let window = &mut self.pressure;
        if !window.open {
            return;
        }
        for event in events {
            if let egui::Event::Touch {
                id, phase, pos, force, ..
            } = event
            {
                match (phase, force) {
                    (egui::TouchPhase::Start | egui::TouchPhase::Move, Some(force)) => {
                        window.record(id.0 as u32, *pos, force.clamp(0.0, 1.0));
                    }
                    (egui::TouchPhase::End | egui::TouchPhase::Cancel, _) => window.lift(),
                    _ => {}
                }
            }
        }
    }

    pub fn pressure_apply(&mut self, action: PressureAction) {
        match action {
            PressureAction::Open => {
                self.pressure.opened_with = self.prefs.settings.pressure.clone();
                self.pressure.strokes.clear();
                self.pressure.note = None;
                self.pressure.drawing = None;
                self.pressure.open = true;
            }
            PressureAction::Close => {
                self.pressure.open = false;
                self.pressure.dragging = false;
                self.pressure.strokes.clear();
                self.pressure.frame = None;
                self.pressure.curve_frame = None;
                self.pressure.drawing = None;
                self.pressure.note = None;
            }
            PressureAction::SetRange {
                low,
                high,
                moved_low,
            } => {
                let next = self.prefs.settings.pressure.with_range(low, high, moved_low);
                self.prefs.settings.pressure = next;
            }
            PressureAction::SetCurve(curve) => {
                if let Ok(next) = self.prefs.settings.pressure.with_curve_shape(curve) {
                    self.prefs.settings.pressure = next;
                }
            }
            PressureAction::Fit => match fit(&self.pressure.samples()) {
                Ok(adjust) => {
                    self.prefs.settings.pressure = adjust;
                    self.pressure.note = None;
                }
                Err(why) => self.pressure.note = Some(why),
            },
            PressureAction::Clear => {
                self.pressure.strokes.clear();
                self.pressure.drawing = None;
                self.pressure.note = None;
            }
            PressureAction::Revert => {
                self.prefs.settings.pressure = self.pressure.opened_with.clone();
            }
            PressureAction::Reset => self.prefs.settings.pressure = PressureAdjust::default(),
        }
    }
}

fn why_text(lang: crate::lang::Lang, why: FitError) -> &'static str {
    match why {
        FitError::TooFew => lang.pick("線が足りません", "Not enough strokes"),
        FitError::TooNarrow => lang.pick("筆圧がほぼ一定です", "The pressure barely varies"),
    }
}

/// 窓の高さ。
fn window_height() -> f32 {
    window::HEADER_HEIGHT
        + 8.0
        + CURVE_HEIGHT
        + GAP
        + 2.0 * (t::SLIDER_ROW_HEIGHT + GAP)
        + DRAW_HEIGHT
        + GAP
        + t::ROW_HEIGHT
        + GAP
        + t::ROW_HEIGHT // 知らせの行
        + 8.0
}

/// 曲線の枠（`ui::curve::curve_editor`）の上に、調整を通す前の筆圧の分布（薄い棒）を重ねる。分布は、下限・上限で 0〜1 にしたあとの
/// 横軸に重ねる（曲線が効く位置と同じ）。数は出さない。枠は背景を塗るので、編集の部品を描いたあとに呼ぶ。
fn distribution_overlay(ui: &egui::Ui, rect: Rect, adjust: &PressureAdjust, samples: &[f32]) {
    if samples.is_empty() {
        return;
    }
    let p = ui.painter().clone();
    let inner = rect.shrink(6.0);
    let at = |x: f32, y: f32| pos2(inner.left() + x * inner.width(), inner.bottom() - y * inner.height());
    let mut bins = [0u32; BINS];
    let span = adjust.high() - adjust.low();
    for s in samples {
        let x = ((s - adjust.low()) / span).clamp(0.0, 1.0);
        bins[((x * BINS as f32) as usize).min(BINS - 1)] += 1;
    }
    let tallest = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
    let fill = Color32::from_rgba_unmultiplied(0x3D, 0x8E, 0xF0, 70);
    for (i, n) in bins.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        let height = *n as f32 / tallest * 0.6;
        let (x0, x1) = (i as f32 / BINS as f32, (i + 1) as f32 / BINS as f32);
        p.rect_filled(Rect::from_two_pos(at(x0, 0.0), at(x1, height)).shrink2(vec2(0.5, 0.0)), 0.0, fill);
    }
}

/// 描く枠: 描いた線を、調整を通した筆圧で太さを変えて出す。
fn draw_frame(ui: &mut egui::Ui, rect: Rect, window: &PressureWindow, adjust: &PressureAdjust) {
    let p = ui.painter().clone();
    w::rounded(&p, rect, t::CANVAS_BG, 3.0);
    w::outline(&p, rect, t::BORDER, 1.0, 3.0);
    let clip = p.with_clip_rect(rect);
    for stroke in &window.strokes {
        let mut previous: Option<&Dot> = None;
        for dot in stroke {
            if let Some(a) = previous {
                let pressure = adjust.apply((a.pressure + dot.pressure) * 0.5);
                clip.line_segment(
                    [rect.min + a.pos, rect.min + dot.pos],
                    Stroke::new(1.0 + 7.0 * pressure, t::TEXT),
                );
            }
            previous = Some(dot);
        }
        if let ([only], true) = (stroke.as_slice(), stroke.len() == 1) {
            clip.circle_filled(rect.min + only.pos, 0.5 + 3.5 * adjust.apply(only.pressure), t::TEXT);
        }
    }
}

/// 開いていれば窓を描き、選んだ値を `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.pressure.open {
        app.pressure.dragging = false;
        return;
    }
    let lang = app.lang;
    let spec = Spec {
        title: lang.pick("筆圧の調整", "Pen Pressure"),
        icon: Some("stylus"),
        size: vec2(WIDTH, window_height()),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let id = window_id();
    let mut offset = app.pressure.offset;
    let mut actions: Vec<PressureAction> = Vec::new();
    let adjust = app.prefs.settings.pressure.clone();
    let samples = app.pressure.samples();
    let note = app.pressure.note;
    let can_fit = !samples.is_empty();
    let has_strokes = !app.pressure.strokes.is_empty();
    let reverted = adjust == app.pressure.opened_with;
    let mut frame_rect = None;
    let mut curve_rect = None;
    let mut dragging = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        let mut rows = w::Rows::new(frame.body, 8.0);
        let curve = rows.row(CURVE_HEIGHT, GAP);
        curve_rect = Some(curve);
        if let Some(next) = curve::curve_editor(
            ui,
            curve,
            id.with("curve"),
            &adjust.curve_shape(),
            lang.pick(
                "ペンの筆圧（左から右）が、ブラシへ渡す筆圧（下から上）になる。薄い棒は、描いた線の筆圧の分布。何も無い所を押すと点を足し、ドラッグで動かし、右クリックで消す。Esc でドラッグをやめる",
                "The pen pressure (across) becomes the pressure the brush gets (up). The faint bars show the pressure of the strokes you drew. Click to add a point, drag to move, right-click to remove. Escape cancels a drag",
            ),
            true,
        ) {
            actions.push(PressureAction::SetCurve(next));
        }
        distribution_overlay(ui, curve, &adjust, &samples);
        let sliders = [
            (
                "low",
                lang.pick("下限", "Low"),
                adjust.low(),
                lang.pick(
                    "この筆圧以下を 0 にする（軽く触れただけの揺れを無くす）",
                    "Pen pressure at or below this counts as zero (ignores the light touches)",
                ),
                true,
            ),
            (
                "high",
                lang.pick("上限", "High"),
                adjust.high(),
                lang.pick(
                    "この筆圧以上を 1 にする（強く押さなくても最大にする）",
                    "Pen pressure at or above this counts as full (reach the maximum without pressing hard)",
                ),
                false,
            ),
        ];
        for (key, label, value, tip, is_low) in sliders {
            let out = w::slider(
                ui,
                rows.row(t::SLIDER_ROW_HEIGHT, GAP),
                ("pressure", key),
                value * 100.0,
                &SliderSpec::new(label, 0.0, 100.0, NumberFormat::int("%")).tooltip(tip),
            );
            dragging |= out.active;
            if out.changed {
                let v = (out.value / 100.0).clamp(0.0, 1.0);
                let (low, high) = if is_low { (v, adjust.high()) } else { (adjust.low(), v) };
                actions.push(PressureAction::SetRange {
                    low,
                    high,
                    moved_low: is_low,
                });
            }
        }
        let draw = rows.row(DRAW_HEIGHT, GAP);
        draw_frame(ui, draw, &app.pressure, &adjust);
        frame_rect = Some(draw);
        ui.interact(draw, id.with("draw"), egui::Sense::hover()).on_hover_text(lang.pick(
            "普段の強さで何本か描く。描いた線は文書に入らない",
            "Draw a few strokes at your usual strength. The strokes are not part of the document",
        ));
        let row = rows.row(t::ROW_HEIGHT, GAP);
        let buttons = w::Rows::split(row, 4, GAP);
        let items = [
            (
                "fit",
                lang.pick("自動調整", "Auto"),
                can_fit,
                lang.pick(
                    "描いた線の筆圧から、下限・上限・曲線を決める",
                    "Low, high and curve from the drawn strokes",
                ),
                PressureAction::Fit,
                true,
            ),
            (
                "clear",
                lang.pick("消す", "Clear"),
                has_strokes,
                lang.pick("描いた線を消す", "Removes the drawn strokes"),
                PressureAction::Clear,
                false,
            ),
            (
                "revert",
                lang.pick("元に戻す", "Revert"),
                !reverted,
                lang.pick(
                    "窓を開いたときの調整に戻す",
                    "Goes back to the adjustment from when the window opened",
                ),
                PressureAction::Revert,
                false,
            ),
            (
                "reset",
                lang.pick("既定", "Default"),
                !adjust.is_default(),
                lang.pick(
                    "下限 0%・上限 100%・直線に戻す",
                    "Back to low 0%, high 100% and a straight line",
                ),
                PressureAction::Reset,
                false,
            ),
        ];
        for ((key, label, enabled, tip, action, primary), rect) in items.into_iter().zip(buttons) {
            if w::button(ui, rect, ("pressure", key), label, primary, enabled, Some(tip), None).clicked() {
                actions.push(action);
            }
        }
        if let Some(why) = note {
            let row = rows.row(t::ROW_HEIGHT, 0.0);
            w::text(
                ui.painter(),
                row,
                why_text(lang, why),
                t::LABEL_DIM.with_color(t::WARNING),
                w::Align::Left,
            );
        }
    });
    app.pressure.offset = offset;
    app.pressure.frame = frame_rect;
    app.pressure.curve_frame = curve_rect;
    app.pressure.dragging = dragging;
    for action in actions {
        app.apply(Action::Pressure(action));
    }
    if closed {
        app.apply(Action::Pressure(PressureAction::Close));
    }
}
