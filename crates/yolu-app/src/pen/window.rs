//! 筆圧の調整（設定のウィンドウの「ペン」の区分。編集 → 設定… の「ペン」で開く）: 枠の中で普段の強さで何本か描くと、描いた線の筆圧の分布から下限・上限と曲線を
//! 自動で決める（式は [`super::adjust`]）。下限・上限は 2 本のスライダーで直し、曲線は共通の編集の部品（`ui::curve::curve_editor`）で手で直す。
//! 曲線の枠には、描いた線の筆圧の分布を薄い棒で重ねる。「元に戻す」は区分を開いたときの調整へ、「既定」は直線へ戻す。
//! 描いた線は枠の中だけに持ち（文書には入らない）、区分を離れる・設定のウィンドウを閉じると捨てる。決めた調整は設定（端末ごと）へ入り、ペンの筆圧をブラシへ渡す前に
//! 直す（`AppState::adjust_pressure`）。マウスの筆圧は 1 のまま、調整を通らない。
//!
//! 枠の中の筆圧は、Windows Ink（`PenSample`）か、ペンが egui の Touch の力として来る環境のどちらか（Windows Ink が使える間は
//! 同じ押しが Touch にも来るので、Touch は読まない）。どちらも調整を通す前の値を集めるので、調整を変えても集めた分布は変わらない。
//! 描く枠が画面に出ている間だけ集める（`PressureWindow::open`）。

use egui::{pos2, vec2, Color32, Id, Pos2, Rect, Stroke, Vec2};
use yolu_core::curve::Curve;

use super::adjust::{fit, FitError, PressureAdjust};
use super::PenSample;
use crate::prefs::{Category, PrefsAction};
use crate::state::{Action, AppState};
use crate::ui::curve;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, SliderSpec};

const GAP: f32 = 6.0;
/// 曲線の枠の高さ。
const CURVE_HEIGHT: f32 = 190.0;
/// 描く枠の高さ。
const DRAW_HEIGHT: f32 = 96.0;
/// 描いた線の本数・点の数の上限（古いものから捨てる。ウィンドウを開いたままの長い試しでメモリを増やさない）。点の数は、線をまたいで古い線から捨て、
/// 1 本が上限を超えて続くときはその線の古い点から捨てる。
pub const MAX_STROKES: usize = 64;
pub const MAX_DOTS: usize = 20_000;
/// 分布の棒の数。
const BINS: usize = 32;

fn block_id() -> Id {
    Id::new("yolu.pressure")
}

/// 枠の中で描いた点 1 つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    /// 枠の左上からの位置（画面の点）。
    pub pos: Vec2,
    /// ペンの筆圧（調整を通す前）。
    pub pressure: f32,
}

/// 筆圧の調整の操作（`Action::Pressure`）。
#[derive(Clone, Debug, PartialEq)]
pub enum PressureAction {
    /// 設定のウィンドウを「ペン」の区分で開く。
    Open,
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
    /// 「ペン」の区分を開いたときの調整へ。
    Revert,
    /// 下限 0・上限 1・直線へ。
    Reset,
}

/// 筆圧の調整の状態（描いた線・開いたときの調整・自動で決められなかった理由）。
#[derive(Debug, Default)]
pub struct PressureWindow {
    /// 筆圧の調整を出している間（`pressure_enter` から `pressure_leave` まで）。
    entered: bool,
    opened_with: PressureAdjust,
    /// 描いた線（古い順）。
    pub strokes: Vec<Vec<Dot>>,
    /// 最後に描いた枠（画面の点）。ペンの点はこの中だけ集める（設定のウィンドウが毎フレーム決める。試験が決めてもよい）。
    /// 枠が出ていない間は None で、何も集めない。
    pub frame: Option<Rect>,
    /// 枠のうち画面に出ている範囲（スクロールで隠れた所には、ペンの点を集めない。None は全部）。
    pub visible: Option<Rect>,
    /// 最後に描いた曲線の枠（画面の点。設定のウィンドウが毎フレーム決める。試験が曲線を操作する位置を知るために読む）。
    pub curve_frame: Option<Rect>,
    /// 今描いている線のペン。
    drawing: Option<u32>,
    /// 自動で決められなかった理由（次に決められる・線を消すと消える）。
    pub note: Option<FitError>,
    /// 下限・上限のスライダーをドラッグしている最中。設定のファイルへは、離す（か区分を離れる）まで調整を書かない（ドラッグの間じゅう
    /// フレームごとに同期付きの書き込みをしない。退避の数の `PrefsState::dragging` と同じ扱い）。
    pub dragging: bool,
}

impl PressureWindow {
    /// 描く枠が画面に出ているか（出ている間だけ、ペンの筆圧を集める）。
    pub fn open(&self) -> bool {
        self.frame.is_some()
    }

    /// 筆圧の調整を、設定のウィンドウの右の欄に出しているか（出し始めから、消えるまで）。
    pub(crate) fn entered(&self) -> bool {
        self.entered
    }

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
        let inside = frame.contains(pos) && self.visible.is_none_or(|v| v.contains(pos));
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

    /// ペンを離した・ウィンドウを閉じた: 次の接触は新しい線にする。
    fn lift(&mut self) {
        self.drawing = None;
    }
}

impl AppState {
    /// ペンの筆圧をブラシへ渡す値にする（設定の調整。既定なら筆圧そのもの）。マウスの筆圧（1）は通さない。
    pub fn adjust_pressure(&self, pressure: f32) -> f32 {
        self.prefs.settings.pressure.apply(pressure)
    }

    /// egui の Touch（指・Windows Ink が無い環境のペン）の力を覚える。ペンの点と同じ全体の調整を通し、マウスの道の筆圧に使う
    /// （2D のキャンバスも 3D ビューも。力が無い・終わったら 1）。
    pub fn note_touch(&mut self, force: Option<f32>, phase: egui::TouchPhase) {
        if let Some(f) = force {
            self.canvas.touch_pressure = Some(self.adjust_pressure(f.clamp(0.0, 1.0)));
        }
        if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
            self.forget_touch();
        }
    }

    /// Touch の力を忘れる（終わった・フォーカスを失って終わりを受け取れない。次のマウスの押しの筆圧を 1 に戻す）。
    pub fn forget_touch(&mut self) {
        self.canvas.touch_pressure = None;
    }

    /// このフレームのペンの点（調整を通す前）を、ウィンドウが開いていれば枠の中の線として集める。
    pub fn pressure_observe(&mut self, pixels_per_point: f32, samples: &[PenSample]) {
        let window = &mut self.pressure;
        if !window.open() {
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
        if !window.open() {
            return;
        }
        for event in events {
            if let egui::Event::Touch {
                id,
                phase,
                pos,
                force,
                ..
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
            PressureAction::Open => self.prefs_apply(PrefsAction::OpenAt(Category::Pen)),
            PressureAction::SetRange {
                low,
                high,
                moved_low,
            } => {
                let next = self
                    .prefs
                    .settings
                    .pressure
                    .with_range(low, high, moved_low);
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

impl AppState {
    /// 描く枠が画面に出始めた（「ペン」の区分を開いた）: 今の調整を「元に戻す」の行き先に覚え、前の線を捨てる。
    pub(crate) fn pressure_enter(&mut self) {
        self.pressure.entered = true;
        self.pressure.opened_with = self.prefs.settings.pressure.clone();
        self.pressure.strokes.clear();
        self.pressure.note = None;
        self.pressure.drawing = None;
    }

    /// 描く枠が画面から消えた（区分を離れた・設定のウィンドウを閉じた）: 描いた線を捨てる（文書には入っていない）。
    pub(crate) fn pressure_leave(&mut self) {
        self.pressure.entered = false;
        self.pressure.dragging = false;
        self.pressure.strokes.clear();
        self.pressure.frame = None;
        self.pressure.visible = None;
        self.pressure.curve_frame = None;
        self.pressure.drawing = None;
        self.pressure.note = None;
    }
}

fn why_text(lang: crate::lang::Lang, why: FitError) -> &'static str {
    match why {
        FitError::TooFew => lang.pick("線が足りません", "Not enough strokes"),
        FitError::TooNarrow => lang.pick("筆圧がほぼ一定です", "The pressure barely varies"),
    }
}

/// 曲線の枠（`ui::curve::curve_editor`）の上に、調整を通す前の筆圧の分布（薄い棒）を重ねる。分布は、下限・上限で 0〜1 にしたあとの
/// 横軸に重ねる（曲線が効く位置と同じ）。数は出さない。枠は背景を塗るので、編集の部品を描いたあとに呼ぶ。
fn distribution_overlay(ui: &egui::Ui, rect: Rect, adjust: &PressureAdjust, samples: &[f32]) {
    if samples.is_empty() {
        return;
    }
    let p = ui.painter().clone();
    let inner = rect.shrink(6.0);
    let at = |x: f32, y: f32| {
        pos2(
            inner.left() + x * inner.width(),
            inner.bottom() - y * inner.height(),
        )
    };
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
        p.rect_filled(
            Rect::from_two_pos(at(x0, 0.0), at(x1, height)).shrink2(vec2(0.5, 0.0)),
            0.0,
            fill,
        );
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
            clip.circle_filled(
                rect.min + only.pos,
                0.5 + 3.5 * adjust.apply(only.pressure),
                t::TEXT,
            );
        }
    }
}

/// 筆圧の調整の中身（曲線・下限と上限・描く枠・ボタン）を `rows` に並べ、選んだ値を `Action` として当てる。設定のウィンドウの「ペン」の区分が呼ぶ。
/// `visible` は、右の欄のうち画面に出ている範囲（スクロールで隠れた所の枠へは、ペンの点を集めない）。
pub(crate) fn rows(ui: &mut egui::Ui, rows: &mut w::Rows, app: &mut AppState, visible: Rect) {
    let lang = app.lang;
    let id = block_id();
    let mut actions: Vec<PressureAction> = Vec::new();
    let adjust = app.prefs.settings.pressure.clone();
    let samples = app.pressure.samples();
    let note = app.pressure.note;
    let can_fit = !samples.is_empty();
    let has_strokes = !app.pressure.strokes.is_empty();
    let reverted = adjust == app.pressure.opened_with;
    let mut dragging = false;
    let curve_rect = rows.row(CURVE_HEIGHT, GAP);
    if let Some(next) = curve::curve_editor(
        ui,
        curve_rect,
        id.with("curve"),
        &adjust.curve_shape(),
        crate::settings::setting_name(lang, "pressure_curve"),
        true,
    ) {
        actions.push(PressureAction::SetCurve(next));
    }
    distribution_overlay(ui, curve_rect, &adjust, &samples);
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
            let (low, high) = if is_low {
                (v, adjust.high())
            } else {
                (adjust.low(), v)
            };
            actions.push(PressureAction::SetRange {
                low,
                high,
                moved_low: is_low,
            });
        }
    }
    let draw = rows.row(DRAW_HEIGHT, GAP);
    draw_frame(ui, draw, &app.pressure, &adjust);
    ui.interact(draw, id.with("draw"), egui::Sense::hover())
        .on_hover_text(lang.pick(
            "普段の強さで何本か描く。描いた線はプロジェクトに入らない",
            "Draw a few strokes at your usual strength. The strokes are not part of the project",
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
                "「ペン」を開いたときの調整に戻す",
                "Goes back to the adjustment from when Pen was opened",
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
        if w::button(
            ui,
            rect,
            ("pressure", key),
            label,
            primary,
            enabled,
            Some(tip),
            None,
        )
        .clicked()
        {
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
    // 描く枠が画面に出ている間だけ、ペンの点を集める。点の位置は枠全体からの距離で持ち、スクロールで隠れた部分では集めない
    let shown = draw.intersect(visible);
    let on_screen = shown.width() > 0.0 && shown.height() > 0.0;
    app.pressure.frame = on_screen.then_some(draw);
    app.pressure.visible = on_screen.then_some(shown);
    app.pressure.curve_frame = Some(curve_rect);
    app.pressure.dragging = dragging;
    for action in actions {
        app.apply(Action::Pressure(action));
    }
}
