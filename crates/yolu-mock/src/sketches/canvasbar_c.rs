//! キャンバスの上の UI（案 C）: キャンバスの右上に、同じ形の小さな半透明のボタンが 2 つ並ぶ。
//! 左は「目・UV・▽」: 目と UV の文字を押すと UV の線の表示・非表示がすぐ切り替わり、▽（細い仕切りの右）を押すと UV の線の色と不透明度のメニューが開く。
//! 右は、描くチャンネルのアイコンと名前・▽: 押すと下に 3 つの列（テクスチャセット・そのレイヤーセット・チャンネル）の欄が開く。
//! チャンネルの行の右の目で見せるチャンネルを選ぶ。

use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

use super::canvasbar_common::{self as c, Name};
use crate::page::SketchCtx;

const MARGIN: f32 = 8.0;
const GAP: f32 = 4.0;
const BUTTON_HEIGHT: f32 = 26.0;
const ROW: f32 = 26.0;
const HEAD: f32 = 28.0;
const PAD: f32 = 6.0;
const COLUMNS: [f32; 3] = [142.0, 150.0, 196.0];
/// UV の線の色の候補（色の四角を押すと順に替わる。既定は本物の既定の色）。
const UV_COLORS: [[u8; 3]; 4] = [
    [89, 217, 255],
    [255, 255, 255],
    [255, 200, 60],
    [255, 100, 130],
];
/// UV の線の不透明度の既定（本物の既定の色の 153 / 255）。
const UV_OPACITY: f32 = 60.0;

/// 撮る間に、どの形にするか。
#[derive(Clone, Copy, PartialEq)]
enum Pose {
    /// 閉じた形（UV は表示）。
    Closed,
    /// UV を非表示にした形。
    UvOff,
    /// UV のメニューを開いた形。
    UvMenu,
    /// 3 列の欄を開いた形。
    Channels,
}

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, Pose::Closed);
}
pub fn draw_uv_off(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, Pose::UvOff);
}
pub fn draw_uv_menu(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, Pose::UvMenu);
}
pub fn draw_open(ui: &mut Ui, ctx: &mut SketchCtx) {
    show(ui, ctx, Pose::Channels);
}

/// 右上の 2 つのボタンの地（閉じているとき。キャンバスが透ける）。非表示の UV のボタンは、押せることを残したまま暗くする。
fn button_bg(open: bool, hovered: bool, pressed: bool, dimmed: bool) -> Color32 {
    if open {
        t::ACCENT_DIM
    } else if pressed {
        Color32::from_black_alpha(220)
    } else if hovered {
        Color32::from_black_alpha(190)
    } else if dimmed {
        Color32::from_black_alpha(200)
    } else {
        Color32::from_black_alpha(120)
    }
}

fn show(ui: &mut Ui, ctx: &mut SketchCtx, pose: Pose) {
    let view = ui.max_rect();
    ui.advance_cursor_after_rect(view);
    let st = c::State::read(ctx);
    let still = ctx.still;

    // 状態（撮る間は形で決める。触るときは値で決める）
    let uv_on = match pose {
        Pose::UvOff if still => false,
        _ if still => true,
        _ => ctx.values.bool("cb.uv.on", true),
    };
    let uv_menu = if still {
        pose == Pose::UvMenu
    } else {
        ctx.values.bool("cb.uv.menu", false)
    };
    let ch_open = if still {
        pose == Pose::Channels
    } else {
        ctx.values.bool("cb.c.open", false)
    };
    let opacity = ctx.values.float("cb.uv.opacity", UV_OPACITY);
    let color_i = ctx.values.index("cb.uv.color", 0) % UV_COLORS.len();
    let [r, g, b] = UV_COLORS[color_i];
    let uv_color = Color32::from_rgba_unmultiplied(r, g, b, (opacity * 2.55).round() as u8);

    c::canvas(ui, view, uv_on.then_some(uv_color));

    // 右上の 2 つのボタン（右端がチャンネル、その左が UV）
    let (icon, name) = {
        let (n, icon) = c::CHANNELS[st.paint];
        (icon, ctx.pick(n.0, n.1))
    };
    let chip_w =
        (w::text_width(ui.painter(), name, t::LABEL) + 6.0 + 20.0 + 4.0 + 18.0 + 6.0).max(84.0);
    let chip = Rect::from_min_size(
        pos2(view.right() - MARGIN - chip_w, view.top() + MARGIN),
        vec2(chip_w, BUTTON_HEIGHT),
    );
    let uv_label = "UV";
    let uv_text_w = w::text_width(ui.painter(), uv_label, t::LABEL);
    let toggle_w = 6.0 + 18.0 + 3.0 + uv_text_w + 8.0;
    let arrow_w = 22.0;
    let uv = Rect::from_min_size(
        pos2(chip.left() - GAP - toggle_w - arrow_w, view.top() + MARGIN),
        vec2(toggle_w + arrow_w, BUTTON_HEIGHT),
    );
    let toggle = Rect::from_min_size(uv.min, vec2(toggle_w, BUTTON_HEIGHT));
    let arrow = Rect::from_min_max(pos2(toggle.right(), uv.top()), uv.max);

    // UV のボタン: 地は 1 つ、押せる範囲は 2 つ（左が表示・非表示、右の ▽ がメニュー）
    let toggle_r = ui.interact(toggle, Id::new("canvasbar.c.uv.toggle"), Sense::click());
    let arrow_r = ui.interact(arrow, Id::new("canvasbar.c.uv.arrow"), Sense::click());
    let p = ui.painter().clone();
    let dimmed = !uv_on;
    w::rounded(&p, uv, button_bg(false, false, false, dimmed), 6.0);
    // 乗せた・押した範囲だけ明るくする
    for (part, resp, left) in [(toggle, &toggle_r, true), (arrow, &arrow_r, false)] {
        let open = !left && uv_menu;
        let bg = if open {
            Some(t::ACCENT_DIM)
        } else if resp.is_pointer_button_down_on() {
            Some(Color32::from_black_alpha(220))
        } else if resp.hovered() {
            Some(Color32::from_black_alpha(120))
        } else {
            None
        };
        if let Some(bg) = bg {
            // 角は外側だけ丸く（内側の仕切りの側は角にしない）
            let radius = egui::CornerRadius {
                nw: if left { 6 } else { 0 },
                sw: if left { 6 } else { 0 },
                ne: if left { 0 } else { 6 },
                se: if left { 0 } else { 6 },
            };
            p.rect_filled(part, radius, bg);
        }
    }
    let fg = if uv_on { t::TEXT } else { t::TEXT_DIM };
    let fg_hot = if toggle_r.hovered() {
        Color32::WHITE
    } else {
        fg
    };
    w::icon(
        &p,
        Rect::from_min_size(toggle.min + vec2(6.0, 0.0), vec2(18.0, BUTTON_HEIGHT)),
        if uv_on {
            "visibility"
        } else {
            "visibility_off"
        },
        fg_hot,
        16.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(toggle.left() + 27.0, toggle.top()), toggle.max),
        uv_label,
        t::LABEL.with_color(fg_hot),
        Align::Left,
    );
    // 仕切りの線と ▽
    w::vline(
        &p,
        arrow.left(),
        uv.top() + 5.0,
        uv.bottom() - 5.0,
        Color32::from_white_alpha(50),
    );
    w::icon(
        &p,
        arrow,
        "arrow_drop_down",
        if uv_menu || arrow_r.hovered() {
            Color32::WHITE
        } else {
            fg
        },
        18.0,
    );
    let toggle_r = toggle_r.on_hover_text(ctx.pick("UV ワイヤーフレーム", "UV Wireframe"));
    let arrow_r = arrow_r.on_hover_text(ctx.pick(
        "UV ワイヤーフレームの色と不透明度",
        "UV wireframe color and opacity",
    ));

    // チャンネルのボタン（UV のボタンと同じ高さ・同じ地）
    let chip_r = ui.interact(chip, Id::new("canvasbar.c.chip"), Sense::click());
    w::rounded(
        &p,
        chip,
        button_bg(
            ch_open,
            chip_r.hovered(),
            chip_r.is_pointer_button_down_on(),
            false,
        ),
        6.0,
    );
    let chip_fg = if ch_open || chip_r.hovered() {
        Color32::WHITE
    } else {
        t::TEXT
    };
    w::icon(
        &p,
        Rect::from_min_size(chip.min + vec2(6.0, 0.0), vec2(20.0, BUTTON_HEIGHT)),
        icon,
        chip_fg,
        16.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(chip.left() + 30.0, chip.top()), chip.max),
        name,
        t::LABEL.with_color(chip_fg),
        Align::Left,
    );
    w::icon(
        &p,
        Rect::from_min_size(
            pos2(chip.right() - 22.0, chip.top()),
            vec2(18.0, BUTTON_HEIGHT),
        ),
        "arrow_drop_down",
        chip_fg,
        18.0,
    );
    let chip_r = chip_r.on_hover_text(ctx.pick(c::TIP_PAINT.0, c::TIP_PAINT.1));

    // 押したときの動き
    let mut next_uv_on = uv_on;
    let mut next_uv_menu = uv_menu;
    let mut next_ch_open = ch_open;
    if toggle_r.clicked() {
        next_uv_on = !uv_on;
    }
    if arrow_r.clicked() {
        next_uv_menu = !uv_menu;
        next_ch_open = false;
    }
    if chip_r.clicked() {
        next_ch_open = !ch_open;
        next_uv_menu = false;
    }

    // 開いている欄
    let mut open_rect = Rect::NOTHING;
    if uv_menu {
        open_rect = uv_popover(ui, ctx, uv, opacity, color_i);
    }
    if ch_open {
        open_rect = channels_popover(ui, ctx, chip, st);
    }
    // 欄の外を押したら閉じる（欄と 2 つのボタンの上は除く）
    let outside = ui.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|q| !open_rect.contains(q) && !uv.contains(q) && !chip.contains(q))
    });
    if outside {
        next_uv_menu = false;
        next_ch_open = false;
    }
    if !still {
        ctx.values.set_bool("cb.uv.on", next_uv_on);
        ctx.values.set_bool("cb.uv.menu", next_uv_menu);
        ctx.values.set_bool("cb.c.open", next_ch_open);
    }
}

/// UV のボタンの下に開く小さな欄: 線の色（色の四角）と不透明度（スライダー）。矩形を返す。
fn uv_popover(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    anchor: Rect,
    opacity: f32,
    color_i: usize,
) -> Rect {
    let width = 224.0;
    let height = PAD * 2.0 + 26.0 + 6.0 + 24.0;
    let pop = Rect::from_min_size(
        pos2(anchor.right() - width, anchor.bottom() + 4.0),
        vec2(width, height),
    );
    egui::Area::new(Id::new("canvasbar.c.uv.pop"))
        .order(egui::Order::Middle)
        .fixed_pos(pop.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(pop.size(), Sense::hover());
            let p = ui.painter().clone();
            w::rounded(&p, pop, t::MENU_BG, 6.0);
            w::outline(&p, pop, t::SEPARATOR, 1.0, 6.0);
            let body = pop.shrink(PAD + 2.0);
            // 色: 名前と色の四角
            let row = Rect::from_min_size(body.min, vec2(body.width(), 26.0));
            w::text(
                &p,
                Rect::from_min_max(row.min, pos2(row.right() - 70.0, row.bottom())),
                ctx.pick("色", "Color"),
                t::LABEL,
                Align::Left,
            );
            let [r, g, b] = UV_COLORS[color_i];
            let swatch =
                Rect::from_min_size(pos2(row.right() - 64.0, row.top() + 2.0), vec2(64.0, 22.0));
            let a = opacity / 100.0;
            let shown = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a];
            let tip = ctx.pick("UV ワイヤーフレームの色", "UV wireframe color");
            if w::color_swatch(ui, swatch, "canvasbar.c.uv.swatch", shown, tip, true).clicked() {
                ctx.values
                    .set_index("cb.uv.color", (color_i + 1) % UV_COLORS.len());
            }
            // 不透明度
            let slider = Rect::from_min_size(
                pos2(body.left(), row.bottom() + 6.0),
                vec2(body.width(), 24.0),
            );
            let spec = SliderSpec::new(
                ctx.pick("不透明度", "Opacity"),
                0.0,
                100.0,
                NumberFormat::int("%"),
            );
            let out = w::slider(ui, slider, "canvasbar.c.uv.opacity", opacity, &spec);
            if out.changed {
                ctx.values.set_float("cb.uv.opacity", out.value);
            }
        });
    pop
}

/// 欄の 1 行（選んでいる行は青、乗せると明るく）。
fn row(
    ui: &mut Ui,
    r: Rect,
    id: (&str, usize, usize),
    icon: Option<&str>,
    label: &str,
    selected: bool,
) -> egui::Response {
    let response = ui.interact(r, Id::new(id), Sense::click());
    let p = ui.painter();
    if selected {
        w::rounded(p, r, t::ACCENT_DIM, 4.0);
    } else if response.hovered() {
        w::rounded(p, r, t::CONTROL_HOVER, 4.0);
    }
    let color = if selected { Color32::WHITE } else { t::TEXT };
    let mut left = r.left() + 8.0;
    if let Some(icon) = icon {
        w::icon(
            p,
            Rect::from_min_size(pos2(left - 2.0, r.top()), vec2(20.0, r.height())),
            icon,
            if selected {
                Color32::WHITE
            } else {
                t::TEXT_DIM
            },
            16.0,
        );
        left += 22.0;
    }
    w::text(
        p,
        Rect::from_min_max(pos2(left, r.top()), pos2(r.right() - 4.0, r.bottom())),
        label,
        t::LABEL.with_color(color),
        Align::Left,
    );
    response
}

/// チャンネルのボタンの下に開く 3 つの列の欄（右端をボタンの右端にそろえる）。矩形を返す。
fn channels_popover(ui: &mut Ui, ctx: &mut SketchCtx, chip: Rect, st: c::State) -> Rect {
    let n_rows = c::CHANNELS
        .len()
        .max(c::SETS.len())
        .max(c::layer_sets(st.set).len());
    let width = COLUMNS.iter().sum::<f32>() + 2.0 * 2.0 + PAD * 2.0;
    let height = PAD * 2.0 + HEAD + n_rows as f32 * ROW;
    let pop = Rect::from_min_size(
        pos2(chip.right() - width, chip.bottom() + 4.0),
        vec2(width, height),
    );
    let mut picks: Vec<(usize, usize)> = Vec::new();
    egui::Area::new(Id::new("canvasbar.c.pop"))
        .order(egui::Order::Middle)
        .fixed_pos(pop.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(pop.size(), Sense::hover());
            let p = ui.painter().clone();
            w::rounded(&p, pop, t::MENU_BG, 6.0);
            w::outline(&p, pop, t::SEPARATOR, 1.0, 6.0);
            let mut x = pop.left() + PAD;
            let heads = [
                (c::ICON_SET, c::TIP_SET),
                (Some(c::ICON_LAYER_SET), c::TIP_LAYER_SET),
                (Some(c::ICON_PAINT), c::TIP_PAINT),
            ];
            for col in 0..3 {
                let area = Rect::from_min_size(
                    pos2(x, pop.top() + PAD),
                    vec2(COLUMNS[col], height - PAD * 2.0),
                );
                // 列の頭: アイコン（名前はツールチップ）。テクスチャセットの列は、合うアイコンが無いので文字。テクスチャセット・レイヤーセットの列は右に追加
                let head = Rect::from_min_size(area.min, vec2(area.width(), HEAD));
                let (icon, tip) = heads[col];
                let icon_rect = Rect::from_min_size(head.min + vec2(6.0, 0.0), vec2(22.0, HEAD));
                let icon_rect = match icon {
                    Some(icon) => {
                        w::icon(&p, icon_rect, icon, t::TEXT_DIM, 16.0);
                        icon_rect
                    }
                    None => {
                        let r = Rect::from_min_size(
                            head.min + vec2(8.0, 0.0),
                            vec2(head.width() - 40.0, HEAD),
                        );
                        w::text(&p, r, ctx.pick(tip.0, tip.1), t::LABEL_DIM, Align::Left);
                        r
                    }
                };
                ui.interact(
                    icon_rect,
                    Id::new(("canvasbar.c.head", col)),
                    Sense::hover(),
                )
                .on_hover_text(ctx.pick(tip.0, tip.1));
                if col < 2 {
                    let add = Rect::from_min_size(
                        pos2(head.right() - 26.0, head.top() + 2.0),
                        vec2(24.0, HEAD - 4.0),
                    );
                    let tip: Name = if col == 0 {
                        ("テクスチャセットを追加", "Add Texture Set")
                    } else {
                        ("レイヤーセットを追加", "Add Layer Set")
                    };
                    let _ = w::icon_button(
                        ui,
                        add,
                        ("canvasbar.c.add", col),
                        "add",
                        ctx.pick(tip.0, tip.1),
                        false,
                        true,
                        16.0,
                    );
                }
                let list_top = head.bottom();
                let line = |i: usize| {
                    Rect::from_min_size(
                        pos2(area.left(), list_top + i as f32 * ROW),
                        vec2(area.width(), ROW - 2.0),
                    )
                };
                match col {
                    0 => {
                        for (i, n) in c::SETS.iter().enumerate() {
                            let label = ctx.pick(n.0, n.1);
                            if row(
                                ui,
                                line(i),
                                ("canvasbar.c.set", i, 0),
                                None,
                                label,
                                i == st.set,
                            )
                            .clicked()
                            {
                                picks.push((0, i));
                            }
                        }
                    }
                    1 => {
                        for (i, n) in c::layer_sets(st.set).iter().enumerate() {
                            let label = ctx.pick(n.0, n.1);
                            let selected = i == st.layer_set;
                            if row(ui, line(i), ("canvasbar.c.ls", i, 0), None, label, selected)
                                .clicked()
                            {
                                picks.push((1, i));
                            }
                        }
                    }
                    _ => {
                        for (i, (n, icon)) in c::CHANNELS.iter().enumerate() {
                            let r = line(i);
                            // 右端の目（見せるチャンネル）の分を空けた行
                            let eye = Rect::from_min_size(
                                pos2(r.right() - 26.0, r.top()),
                                vec2(26.0, r.height()),
                            );
                            let main = Rect::from_min_max(r.min, pos2(eye.left(), r.bottom()));
                            let label = ctx.pick(n.0, n.1);
                            let name_row = row(
                                ui,
                                main,
                                ("canvasbar.c.ch", i, 0),
                                Some(icon),
                                label,
                                i == st.paint,
                            );
                            if name_row.clicked() {
                                picks.push((2, i));
                            }
                            // 見せるチャンネルの目: 選んでいる行は青い目、ほかは乗せたときだけ薄く
                            let showing = i == st.display;
                            let hit =
                                ui.interact(eye, Id::new(("canvasbar.c.eye", i)), Sense::click());
                            if showing || hit.hovered() || name_row.hovered() {
                                let color = if showing { t::ACCENT } else { t::TEXT_DISABLED };
                                w::icon(&p, eye, c::ICON_DISPLAY, color, 16.0);
                            }
                            let hit =
                                hit.on_hover_text(ctx.pick(c::TIP_DISPLAY.0, c::TIP_DISPLAY.1));
                            if hit.clicked() {
                                picks.push((3, i));
                            }
                        }
                    }
                }
                x += COLUMNS[col];
                if col < 2 {
                    w::vline(
                        &p,
                        x + 1.0,
                        pop.top() + PAD + 4.0,
                        pop.bottom() - PAD - 4.0,
                        t::SEPARATOR,
                    );
                    x += 2.0;
                }
            }
        });
    for (kind, i) in picks {
        match kind {
            0 => c::State::pick_set(ctx, i),
            1 => c::State::pick_layer_set(ctx, i),
            2 => c::State::pick_paint(ctx, i),
            _ => c::State::pick_display(ctx, i),
        }
    }
    pop
}
