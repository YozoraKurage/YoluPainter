//! 浮いた色の選び: 色相の円と中の四角（カラーの欄と同じ見た目と操作）と 16 進の欄を、押した所のそばに出す。ランプの色の分岐点のダブルクリック
//! （と色の見本）から開く。色を決める 1 つの小さな窓で、描画色（`ColorState`）は動かさない。
//!
//! - 円か四角をドラッグするか 16 進を入れると、その場で色が変わる（呼び手が毎フレーム受け取って文書へ当てる）。
//! - Esc で開いたときの色へ戻して閉じる。窓の外を押すか、開いた分岐点が無くなっても閉じる（そのときの色のまま）。
//! - 色相は選びの途中で覚え、彩度や明度が 0 になっても失わない。外から色が変わった（Undo・スポイト）ときは選びの表示を合わせる。

use std::sync::{Arc, Mutex};

use egui::{pos2, vec2, Color32, Context, Id, Order, Pos2, Rect, Sense};

use super::color::{hue_at, in_ring, wheel_square, ColorTextures, RING_THICKNESS};
use crate::lang::Lang;
use crate::state::{hsv_to_rgb, parse_hex, rgb_to_hsv, to_hex};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 窓の大きさ（円の大きさ + 16 進の欄）。
pub const SIZE: egui::Vec2 = vec2(188.0, 220.0);
const PAD: f32 = 8.0;
const WHEEL: f32 = SIZE.x - 2.0 * PAD;
const HEX_HEIGHT: f32 = 22.0;

/// 1 回の呼びの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 変わらない（開いていない場合も）。
    Idle,
    /// 色が変わった（ドラッグ・16 進）。
    Changed([u8; 3]),
    /// Esc で開いたときの色へ戻して閉じた。
    Reverted([u8; 3]),
    /// 窓の外を押して閉じた。
    Closed,
}

#[derive(Clone)]
struct State {
    /// 開いた分岐点の番号（別の分岐点を選んだら閉じる）。
    target: usize,
    anchor: Rect,
    /// 欄の列（幅は欄の幅）。窓は、入るならこの列の左に置いて、欄の操作を隠さない。
    column: Rect,
    original: [u8; 3],
    /// 今出している色。
    shown: [u8; 3],
    hue: f32,
    sat: f32,
    val: f32,
    /// 開いたフレーム（そのフレームの押しで閉じない）。
    opened: u64,
    /// 最後に呼ばれたフレーム（欄が描かれなくなった（別の層を選んだ）あとに、開いたまま残らないように）。
    seen: u64,
}

fn state_id(id: Id) -> Id {
    id.with("color-popup")
}

fn textures(ctx: &Context) -> Arc<Mutex<ColorTextures>> {
    let key = Id::new("yolu.ramp.color-popup.textures");
    if let Some(found) = ctx.data(|d| d.get_temp::<Arc<Mutex<ColorTextures>>>(key)) {
        return found;
    }
    let made = Arc::new(Mutex::new(ColorTextures::default()));
    ctx.data_mut(|d| d.insert_temp(key, made.clone()));
    made
}

fn floats(c: [u8; 3]) -> (f32, f32, f32) {
    (
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
    )
}

fn bytes(rgb: (f32, f32, f32)) -> [u8; 3] {
    [w::to_byte(rgb.0), w::to_byte(rgb.1), w::to_byte(rgb.2)]
}

/// `anchor`（分岐点の印のあたり）のそばに開く。`column` は欄の列（窓は、入るならその左に置く）。`current` は今の色、`target` は開いた分岐点の番号。
/// 開いている間に呼び直すと、開き直す（元の色は今の色になる）。
pub fn open(ctx: &Context, id: Id, anchor: Rect, column: Rect, current: [u8; 3], target: usize) {
    let (r, g, b) = floats(current);
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    // データの書き込みの最中に文脈の読み取りをすると詰まるので、先に取る
    let opened = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            state_id(id),
            State {
                target,
                anchor,
                column,
                original: current,
                shown: current,
                hue,
                sat,
                val,
                opened,
                seen: opened,
            },
        )
    });
    ctx.request_repaint();
}

pub fn is_open(ctx: &Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<State>(state_id(id))).is_some()
}

pub fn close(ctx: &Context, id: Id) {
    ctx.data_mut(|d| d.remove::<State>(state_id(id)));
}

/// 窓の場所（開いていれば。試験が円の位置を知るために読む）。
pub fn rect(ctx: &Context, id: Id) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Rect>(state_id(id).with("rect")))
}

/// 窓を置く場所。欄の列 `column` の左（欄の操作と、色の変わるのを見せる棒を隠さない）に入れば、そこで `anchor` の高さのあたりへ。入らなければ（欄が画面の
/// 左に寄っているなど）`anchor` の下（入らなければ上）へ。どちらも画面の中に収める。
pub fn place(anchor: Rect, column: Rect, screen: Rect) -> Rect {
    let gap = 8.0;
    let left = column.left() - SIZE.x - gap;
    if left >= screen.left() + 4.0 {
        let top = (anchor.center().y - SIZE.y * 0.5).clamp(
            screen.top() + 4.0,
            (screen.bottom() - SIZE.y - 4.0).max(screen.top()),
        );
        return Rect::from_min_size(pos2(left, top), SIZE);
    }
    crate::ui::menu::place(anchor.translate(vec2(0.0, 6.0)), SIZE, screen)
}

/// 円の外接の正方形（窓の場所から）。
pub fn wheel_of(window: Rect) -> Rect {
    Rect::from_min_size(window.min + vec2(PAD, PAD), vec2(WHEEL, WHEEL))
}

/// 16 進の欄の場所（窓の場所から）。
pub fn hex_of(window: Rect) -> Rect {
    Rect::from_min_size(
        pos2(window.left() + PAD, window.top() + PAD + WHEEL + 6.0),
        vec2(WHEEL, HEX_HEIGHT),
    )
}

fn marker(p: &egui::Painter, at: Pos2, radius: f32) {
    p.circle_stroke(at, radius, egui::Stroke::new(2.0, Color32::BLACK));
    p.circle_stroke(at, radius - 1.0, egui::Stroke::new(1.5, Color32::WHITE));
}

/// 毎フレーム呼ぶ（開いていなければ何もしない）。`current` は文書の今の色、`target` は今選んでいる分岐点の番号。`present` は開いた分岐点がまだ
/// あるか。無ければ、別の分岐点を選んだら閉じる。
pub fn show(
    ctx: &Context,
    id: Id,
    current: [u8; 3],
    target: usize,
    present: bool,
    lang: Lang,
) -> Outcome {
    let Some(mut state) = ctx.data(|d| d.get_temp::<State>(state_id(id))) else {
        return Outcome::Idle;
    };
    // 欄が何フレームも描かれなかった（別の層・段を選んでいた）なら、前の続きとして開き直さない
    let frame = ctx.cumulative_frame_nr();
    if frame > state.seen + 3 {
        close(ctx, id);
        return Outcome::Idle;
    }
    state.seen = frame;
    if !present || state.target != target {
        close(ctx, id);
        return Outcome::Closed;
    }
    // 外から色が変わった（Undo・スポイト・別の操作）。選びの表示を合わせる（灰色では色相を残す）
    if current != state.shown {
        let (r, g, b) = floats(current);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if s > 1e-4 && v > 1e-4 {
            state.hue = h;
        }
        if v > 1e-4 {
            state.sat = s;
        }
        state.val = v;
        state.shown = current;
    }
    let screen = ctx.content_rect();
    let window = place(state.anchor, state.column, screen);
    ctx.data_mut(|d| d.insert_temp(state_id(id).with("rect"), window));
    let tex = textures(ctx);
    let mut outcome = Outcome::Idle;
    let area = egui::Area::new(state_id(id).with("area"))
        .order(Order::Foreground)
        .fixed_pos(window.min)
        .constrain(false)
        .show(ctx, |ui| {
            // 窓の上の押下は下へ通さない
            ui.allocate_exact_size(window.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            for i in (1..=5).rev() {
                let f = i as f32;
                p.rect_filled(
                    window.expand(f).translate(vec2(0.0, 2.0)),
                    6.0 + f,
                    Color32::from_black_alpha(14),
                );
            }
            w::rounded(&p, window, t::PANEL_BG, 6.0);
            w::outline(&p, window, t::SEPARATOR, 1.0, 6.0);
            let wheel = wheel_of(window);
            let sq = wheel_square(wheel);
            let wheel_id = id.with("color-popup-wheel");
            let response = ui.interact(wheel, wheel_id, Sense::click_and_drag());
            // 押した所が輪なら色相、中の四角なら彩度と明度（ドラッグの間は押したほうのまま）
            let mode_id = wheel_id.with("mode");
            if response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()) {
                let origin = ui.input(|i| i.pointer.press_origin());
                let mode = origin
                    .map(|o| {
                        if in_ring(wheel, o) {
                            1u8
                        } else if sq.contains(o) {
                            2
                        } else {
                            0
                        }
                    })
                    .unwrap_or(0);
                ui.data_mut(|d| d.insert_temp(mode_id, mode));
            }
            let mut picked = false;
            if response.is_pointer_button_down_on() {
                let mode: u8 = ui.data(|d| d.get_temp(mode_id).unwrap_or(0));
                if let Some(at) = response.interact_pointer_pos() {
                    match mode {
                        1 => {
                            state.hue = hue_at(wheel, at);
                            picked = true;
                        }
                        2 => {
                            state.sat = ((at.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
                            state.val = (1.0 - (at.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
                            picked = true;
                        }
                        _ => {}
                    }
                }
            }
            if picked {
                let next = bytes(hsv_to_rgb(state.hue, state.sat, state.val));
                if next != state.shown {
                    state.shown = next;
                    outcome = Outcome::Changed(next);
                }
            }
            let (ring, square) = {
                let mut tex = tex.lock().unwrap_or_else(|e| e.into_inner());
                (tex.ring(ui.ctx()), tex.sv(ui.ctx(), state.hue))
            };
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            p.image(ring, wheel, uv, Color32::WHITE);
            let radius = wheel.width() * 0.5 * (1.0 - RING_THICKNESS * 0.5);
            let a = state.hue * std::f32::consts::TAU;
            marker(
                &p,
                pos2(
                    wheel.center().x + a.sin() * radius,
                    wheel.center().y - a.cos() * radius,
                ),
                wheel.width() * RING_THICKNESS * 0.42,
            );
            p.image(square, sq, uv, Color32::WHITE);
            w::outline(&p, sq, t::BORDER, 1.0, 0.0);
            marker(
                &p,
                pos2(
                    sq.left() + state.sat * sq.width(),
                    sq.top() + (1.0 - state.val) * sq.height(),
                ),
                6.0,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Other,
                    true,
                    lang.pick("色相の円", "Hue wheel"),
                )
            });
            // 16 進
            let hex = format!(
                "#{}",
                to_hex([
                    f32::from(state.shown[0]) / 255.0,
                    f32::from(state.shown[1]) / 255.0,
                    f32::from(state.shown[2]) / 255.0,
                    1.0,
                ])
            );
            let typed = w::text_field(
                ui,
                hex_of(window),
                wheel_id.with("hex"),
                &hex,
                Some(lang.pick("16 進の色（#RRGGBB）", "Hex color (#RRGGBB)")),
                false,
            );
            if let Some(text) = typed.committed {
                if let Some(rgb) = parse_hex(&text) {
                    let next = bytes((rgb[0], rgb[1], rgb[2]));
                    let (h, s, v) = rgb_to_hsv(rgb[0], rgb[1], rgb[2]);
                    if s > 1e-4 && v > 1e-4 {
                        state.hue = h;
                    }
                    if v > 1e-4 {
                        state.sat = s;
                    }
                    state.val = v;
                    if next != state.shown {
                        state.shown = next;
                        outcome = Outcome::Changed(next);
                    }
                }
            }
        });
    let _ = area;
    // Esc: 開いたときの色へ戻して閉じる（16 進の欄を打っている途中の Esc は、その欄のやめるだけにして、窓は閉じない）
    let typing = ctx.memory(|m| m.focused().is_some());
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !typing {
        close(ctx, id);
        return Outcome::Reverted(state.original);
    }
    // 窓の外を押したら閉じる（開いたフレームの押しと、開いた分岐点そのものへの押しは数えない）
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !window.contains(at) && !state.anchor.expand(8.0).contains(at))
    });
    if pressed_outside && ctx.cumulative_frame_nr() > state.opened {
        close(ctx, id);
        return if outcome == Outcome::Idle {
            Outcome::Closed
        } else {
            outcome
        };
    }
    ctx.data_mut(|d| d.insert_temp(state_id(id), state));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_goes_left_of_the_column_when_it_fits_and_below_the_stop_when_it_does_not() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 1000.0));
        let anchor = Rect::from_center_size(pos2(1160.0, 860.0), vec2(10.0, 10.0));
        let column = Rect::from_min_size(pos2(1070.0, 800.0), vec2(200.0, 0.0));
        let w = place(anchor, column, screen);
        assert!(w.right() <= column.left(), "欄の左: {w:?}");
        assert!(screen.contains_rect(w));
        assert!(
            (w.center().y - anchor.center().y).abs() < 60.0 || w.bottom() <= screen.bottom() - 3.0
        );
        // 画面の上下の端では、収まるように寄せる
        let high = Rect::from_center_size(pos2(1160.0, 20.0), vec2(10.0, 10.0));
        assert!(place(high, column, screen).top() >= 0.0);
        let low = Rect::from_center_size(pos2(1160.0, 990.0), vec2(10.0, 10.0));
        assert!(place(low, column, screen).bottom() <= 1000.0);
        // 欄が画面の左に寄っていて、左に入らないときは、分岐点の下
        let narrow_column = Rect::from_min_size(pos2(8.0, 100.0), vec2(280.0, 0.0));
        let near = Rect::from_center_size(pos2(100.0, 200.0), vec2(10.0, 10.0));
        let w = place(near, narrow_column, screen);
        assert!(
            w.top() >= near.bottom() - 10.0 && screen.contains_rect(w),
            "{w:?}"
        );
    }
}
