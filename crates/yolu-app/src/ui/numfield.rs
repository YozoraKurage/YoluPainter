//! 数値の欄（ドラッグで増減・クリックで打つ）: 範囲が決まらない値（形の中心・回転・大きさ・繰り返し・オフセット）の行に使う。
//! スライダーと同じく、ドラッグは押し始めの値から数える（Esc で押し始めの値に戻し、離すまで残りのドラッグは受けない）。クリックだけなら
//! 打つ欄になり、Enter で決め（範囲の外は端へ）、Esc か外を押すとやめる。ドラッグ中の変更を 1 回の Undo にまとめるのは、欄を並べる側
//! （`changed` のたびに `coalesce` で渡し、離したらまとめを終える）。軸の色の下線は X・Y・Z の見分け。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo};

use super::theme as t;
use super::widgets::{fit, outline, rounded, text, Align};

/// 数値の欄の決まり。
#[derive(Clone, Copy, Debug)]
pub struct NumSpec {
    pub min: f64,
    pub max: f64,
    /// ドラッグの 1 画素あたりの増減。
    pub step: f64,
    pub decimals: u8,
}

impl NumSpec {
    pub fn new(min: f64, max: f64, step: f64, decimals: u8) -> NumSpec {
        NumSpec {
            min,
            max,
            step,
            decimals,
        }
    }

    /// 表示する文字（小数の後ろの 0 は落とす）。
    pub fn format(&self, v: f64) -> String {
        let scale = 10f64.powi(i32::from(self.decimals));
        let rounded = (v * scale).round() / scale;
        let mut s = format!("{:.*}", usize::from(self.decimals), rounded);
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        if s == "-0" {
            s = "0".into();
        }
        s
    }
}

/// 数値の欄の結果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumOutcome {
    pub value: f64,
    pub changed: bool,
}

/// ドラッグとクリックを分ける距離（点）。
const CLICK_SLOP: f32 = 3.0;

/// 数値の欄。`label` は左の短い名前（X・Y・Z・U・V など。空でもよい）、`underline` は軸の色。
#[allow(clippy::too_many_arguments)]
pub fn number_field(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl egui::AsIdSalt,
    label: &str,
    value: f64,
    spec: &NumSpec,
    tooltip: Option<&str>,
    underline: Option<Color32>,
    enabled: bool,
) -> NumOutcome {
    let id = ui.make_persistent_id(id_salt);
    let enabled = enabled && ui.is_enabled();
    // 色・枠の見た目（描いている間は描き始める前のまま。押せるかは本当の `enabled`）
    let shown_look = super::widgets::look(ui.ctx(), id, enabled);
    let mut out = NumOutcome {
        value,
        changed: false,
    };
    let edit_id = id.with("edit");
    let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
    if let Some(mut buffer) = editing {
        draw_box(ui, r, label, "", underline, true, false, true);
        let te_id = id.with("field");
        let label_w = label_width(ui, label);
        let inner = Rect::from_min_max(
            pos2(r.left() + label_w + 2.0, r.top() + 1.0),
            pos2(r.right() - 3.0, r.bottom() - 1.0),
        );
        let response = ui.put(
            inner,
            egui::TextEdit::singleline(&mut buffer)
                .id(te_id)
                .frame(egui::Frame::NONE)
                .font(t::VALUE.font())
                .text_color(t::TEXT)
                .margin(egui::Margin::ZERO)
                .horizontal_align(egui::Align::RIGHT)
                .vertical_align(egui::Align::Center),
        );
        let first: bool = ui.data(|d| d.get_temp(edit_id.with("focus")).unwrap_or(false));
        if first {
            response.request_focus();
            ui.data_mut(|d| d.insert_temp(edit_id.with("focus"), false));
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if response.lost_focus() || (!first && !response.has_focus()) {
            if enter && !escape {
                if let Ok(typed) = buffer.trim().parse::<f64>() {
                    if typed.is_finite() {
                        out.value = typed.clamp(spec.min, spec.max);
                        out.changed = out.value != value;
                    }
                }
            }
            ui.data_mut(|d| d.remove::<String>(edit_id));
        } else {
            ui.data_mut(|d| d.insert_temp(edit_id, buffer));
        }
        return out;
    }

    let response = ui.interact(
        r,
        id,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let (start_id, cancel_id, from_id, moved_id) = (
        id.with("start"),
        id.with("cancelled"),
        id.with("from"),
        id.with("moved"),
    );
    let pointer_x = response.interact_pointer_pos().map(|p| p.x);
    let mut dragging = false;
    if enabled && response.is_pointer_button_down_on() {
        let cancelled = ui.data(|d| d.get_temp::<bool>(cancel_id)).unwrap_or(false);
        if !cancelled {
            if ui.data(|d| d.get_temp::<f64>(start_id)).is_none() {
                ui.data_mut(|d| {
                    d.insert_temp(start_id, value);
                    d.insert_temp(from_id, pointer_x.unwrap_or(r.center().x));
                });
            }
            let start: f64 = ui.data(|d| d.get_temp(start_id)).unwrap_or(value);
            let from: f32 = ui.data(|d| d.get_temp(from_id)).unwrap_or(0.0);
            if let Some(x) = pointer_x {
                // 一度動かしたら、押し始めの近くへ戻しても動かし続ける（戻した所の値が押し始めの値になる）
                let past = ui.data(|d| d.get_temp::<bool>(moved_id)).unwrap_or(false);
                if past || (x - from).abs() > CLICK_SLOP {
                    dragging = true;
                    ui.data_mut(|d| d.insert_temp(moved_id, true));
                    let fine = ui.input(|i| i.modifiers.shift);
                    let step = if fine { spec.step * 0.1 } else { spec.step };
                    let next = (start + f64::from(x - from) * step).clamp(spec.min, spec.max);
                    // 値は表示の桁で丸める（桁の外のごみを文書へ入れない）
                    let scale = 10f64.powi(i32::from(spec.decimals) + i32::from(fine));
                    let next = (next * scale).round() / scale;
                    if next != value {
                        out.value = next;
                        out.changed = true;
                    }
                }
            }
        }
    }
    // Esc でドラッグを止めた: 押し始めの値へ戻し、離すまで残りのドラッグは受けない
    let mut escaped = false;
    if enabled && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if let Some(start) = ui.data(|d| d.get_temp::<f64>(start_id)) {
            escaped = true;
            ui.data_mut(|d| {
                d.remove::<f64>(start_id);
                d.insert_temp(cancel_id, true);
            });
            dragging = false;
            if start != out.value {
                out.value = start;
                out.changed = true;
            }
        }
    }
    // 動かした（または Esc で止めた）押下の離しは、クリックとして打つ欄を開かない
    let moved = ui.data(|d| {
        d.get_temp::<bool>(moved_id).unwrap_or(false)
            || d.get_temp::<bool>(cancel_id).unwrap_or(false)
    });
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| {
            d.remove::<f64>(start_id);
            d.remove::<f32>(from_id);
            d.remove::<bool>(cancel_id);
            d.remove::<bool>(moved_id);
        });
    }
    if enabled && !escaped && !moved && response.clicked() {
        ui.data_mut(|d| {
            d.insert_temp(edit_id, spec.format(value));
            d.insert_temp(edit_id.with("focus"), true);
        });
        ui.ctx().request_repaint();
    }
    if ui.is_rect_visible(r) {
        let shown = spec.format(out.value);
        draw_box(
            ui,
            r,
            label,
            &shown,
            underline,
            false,
            shown_look.live && (response.hovered() || dragging),
            shown_look.enabled,
        );
    }
    let name = if label.is_empty() {
        tooltip.unwrap_or("").to_owned()
    } else {
        label.to_owned()
    };
    let current = out.value;
    response.widget_info(|| WidgetInfo::slider(enabled, current, &name));
    match tooltip {
        Some(tip) if !tip.is_empty() => {
            let _ = response.on_hover_text(tip);
        }
        _ => {}
    }
    out
}

fn label_width(ui: &Ui, label: &str) -> f32 {
    if label.is_empty() {
        0.0
    } else {
        super::widgets::text_width(ui.painter(), label, t::LABEL_DIM) + 8.0
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_box(
    ui: &Ui,
    r: Rect,
    label: &str,
    value: &str,
    underline: Option<Color32>,
    editing: bool,
    hot: bool,
    enabled: bool,
) {
    let p = ui.painter();
    rounded(p, r, t::CONTROL_BG, 3.0);
    outline(
        p,
        r,
        if editing {
            t::ACCENT
        } else if hot {
            t::ACCENT_DIM
        } else {
            t::BORDER
        },
        1.0,
        3.0,
    );
    let color = if enabled { t::TEXT } else { t::TEXT_DISABLED };
    let lw = label_width(ui, label);
    if !label.is_empty() {
        text(
            p,
            Rect::from_min_size(pos2(r.left() + 5.0, r.top()), vec2(lw, r.height())),
            label,
            t::LABEL_DIM.with_color(if enabled {
                t::TEXT_DIM
            } else {
                t::TEXT_DISABLED
            }),
            Align::Left,
        );
    }
    if !editing {
        let area = Rect::from_min_max(
            pos2(r.left() + lw, r.top()),
            pos2(r.right() - 4.0, r.bottom()),
        );
        let shown = fit(p, value, area.width(), t::VALUE);
        text(p, area, &shown, t::VALUE.with_color(color), Align::Right);
    }
    if let Some(c) = underline {
        p.rect_filled(
            Rect::from_min_size(
                pos2(r.left() + 4.0, r.bottom() - 2.5),
                vec2((r.width() - 8.0).max(0.0), 1.5),
            ),
            0.0,
            if enabled { c } else { c.gamma_multiply(0.4) },
        );
    }
}
