//! マテリアルで塗る（Unity 版の「ブラシのマテリアル」）の欄: オン・オフ、塗るチャンネルの組（2 列のチップ）、組のチャンネルごとの値
//! （Color は描画色の見本、Emission は色、Roughness・Metallic・Height は 0〜1、Normal は傾き）。プロパティの「マテリアル」のタブと、
//! バケツ・ポリゴン塗りつぶしの欄の下に出る。値は画面の状態（`AppState::mat`）で、ブラシ・バケツ・ポリゴン塗りつぶしが使う。
//! 画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::properties::{section, slider_row, status_row};
use crate::engine::Channel;
use crate::lang::Lang;
use crate::m2::{channel_icon, channel_name};
use crate::matpaint::{MatAction, CHANNELS};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

const COLUMNS: usize = 2;
const CHIP_HEIGHT: f32 = 22.0;

fn two_decimals() -> NumberFormat<'static> {
    NumberFormat {
        decimals: 2,
        trim: false,
        suffix: "",
    }
}

/// 「マテリアル」のタブ。
pub fn material_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    material_section(ui, app, rows);
    crate::look::panel::look_section(ui, app, rows);
}

/// 「ブラシのマテリアル」の節（開閉は他の欄と同じく覚える）。
pub fn material_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "brush-material",
        lang.pick("ブラシのマテリアル", "Brush Material"),
        "layers",
        None,
    );
    if !open {
        return;
    }
    let free = !app.is_stroking();
    let on = app.mat.enabled;
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let next = w::toggle(
        ui,
        row,
        "mat.toggle",
        lang.pick("複数のチャンネルを一度に塗る", "Paint several channels at once"),
        on,
        Some(lang.pick(
            "オンなら、下でチェックしたチャンネルを、それぞれの値で 1 回のストロークで塗ります（2D と 3D、バケツ、ポリゴン塗りつぶし）。1 回の取り消しで全部が戻ります。オフなら、選んでいるチャンネルを描画色で塗ります",
            "On: one stroke (or fill) paints every channel checked below with its own value, in 2D and 3D. One undo takes all of them back. Off: paints the selected channel with the paint color",
        )),
        free,
    );
    if next != on {
        app.apply(Action::Mat(MatAction::Enabled(next)));
    }
    if !app.mat.enabled {
        rows.space(4.0);
        return;
    }
    chips(ui, app, rows, lang, free);
    if app.m2.edit_mask {
        status_row(
            ui,
            rows,
            lang.pick("マスクに塗っています", "Painting the layer mask"),
        );
    } else if app.m2.brush.effect.is_paint() {
        for channel in app.mat.included() {
            value_rows(ui, app, rows, channel, lang, free);
        }
    }
    rows.space(4.0);
}

/// 塗るチャンネルの組（2 列のチップ。押すと足す・外す）。
fn chips(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang, free: bool) {
    for line in CHANNELS.chunks(COLUMNS) {
        let row = rows.row(CHIP_HEIGHT, 4.0);
        let cells = Rows::split(row, COLUMNS, 4.0);
        for (cell, channel) in cells.iter().zip(line) {
            let included = app.mat.includes(*channel);
            let name = channel_name(lang, &app.doc, *channel);
            let tip = if included {
                lang.pick(
                    format!("{name}: ストロークで塗る（クリックで外す）"),
                    format!("{name}: painted by the stroke (click to leave it out)"),
                )
            } else {
                lang.pick(
                    format!("{name}: 塗らない（クリックで足す）"),
                    format!("{name}: not painted (click to paint it too)"),
                )
            };
            let current = *channel == app.m2.paint_channel;
            if chip(ui, *cell, *channel, &name, included, current, free, &tip) {
                app.apply(Action::Mat(MatAction::Channel(*channel, !included)));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn chip(
    ui: &mut Ui,
    r: Rect,
    channel: Channel,
    name: &str,
    included: bool,
    current: bool,
    enabled: bool,
    tooltip: &str,
) -> bool {
    let id = ui.make_persistent_id(("mat.chip", channel.index()));
    let response = ui.interact(r, id, Sense::click());
    let hover = enabled && response.hovered();
    let p = ui.painter();
    w::rounded(
        p,
        r,
        if included {
            t::ACCENT_DIM
        } else if hover {
            t::CONTROL_HOVER
        } else {
            t::CONTROL_BG
        },
        4.0,
    );
    let text_color = if included {
        egui::Color32::WHITE
    } else {
        t::TEXT
    };
    w::icon(
        p,
        Rect::from_min_size(pos2(r.left() + 3.0, r.top()), vec2(18.0, r.height())),
        if included {
            "check"
        } else {
            channel_icon(channel)
        },
        if included {
            egui::Color32::WHITE
        } else {
            t::TEXT_DIM
        },
        14.0,
    );
    let label_rect = Rect::from_min_max(
        pos2(r.left() + 21.0, r.top()),
        pos2(r.right() - 3.0, r.bottom()),
    );
    let shown = w::fit(p, name, label_rect.width(), t::LABEL_SMALL);
    w::text(
        p,
        label_rect,
        &shown,
        t::LABEL_SMALL.with_color(text_color),
        w::Align::Left,
    );
    if current {
        // 描くチャンネル（今見ているもの）の印
        w::rounded(
            p,
            Rect::from_min_size(
                pos2(r.left() + 4.0, r.bottom() - 3.0),
                vec2(r.width() - 8.0, 2.0),
            ),
            if included {
                egui::Color32::WHITE
            } else {
                t::ACCENT
            },
            1.0,
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, included, name));
    let response = response.on_hover_text(tooltip);
    enabled && response.clicked()
}

/// 組のチャンネルの値の行。
fn value_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    channel: Channel,
    lang: Lang,
    free: bool,
) {
    let name = channel_name(lang, &app.doc, channel);
    match channel {
        Channel::Color => {
            let row = rows.row(t::ROW_HEIGHT, 2.0);
            label_and_swatch(
                ui,
                row,
                &name,
                "mat.color",
                app.color.main,
                lang.pick(
                    "描画色（カラーのパネルと同じ）",
                    "The paint color (also in the Color panel)",
                ),
            );
        }
        Channel::Emission => {
            let row = rows.row(t::ROW_HEIGHT, 2.0);
            let e = app.mat.emission;
            let swatch = label_and_swatch(
                ui,
                row,
                &name,
                "mat.emission",
                [e[0], e[1], e[2], 1.0],
                lang.pick(
                    "ストロークが塗るエミッションの色（押すと描画色にする）",
                    "The emission color the stroke paints (click to take the paint color)",
                ),
            );
            if swatch && free {
                app.apply(Action::Mat(MatAction::EmissionFromPaint));
            }
            let hex_rect = Rect::from_min_max(
                pos2(row.left() + 84.0 + 40.0, row.top() + 1.0),
                pos2(row.right(), row.bottom() - 1.0),
            );
            if hex_rect.width() > 40.0 {
                let current = crate::state::to_hex([e[0], e[1], e[2], 1.0]);
                let out = w::text_field(
                    ui,
                    hex_rect,
                    "mat.emission.hex",
                    &current,
                    Some(lang.pick("16 進（#RRGGBB）", "Hex (#RRGGBB)")),
                    false,
                );
                if let Some(text) = out.committed {
                    if let Some(rgb) = crate::state::parse_hex(&text) {
                        app.apply(Action::Mat(MatAction::Emission(rgb)));
                    }
                }
            }
        }
        Channel::Normal => {
            let row = rows.row(t::ROW_HEIGHT, 2.0);
            w::text(
                ui.painter(),
                Rect::from_min_max(row.min, pos2(row.right() - 28.0, row.bottom())),
                &name,
                t::LABEL,
                w::Align::Left,
            );
            let reset = Rect::from_min_size(
                pos2(row.right() - 24.0, row.top()),
                vec2(24.0, row.height()),
            );
            if w::icon_button(
                ui,
                reset,
                "mat.normal.flat",
                "restart_alt",
                lang.pick(
                    "平らな法線（128, 128, 255）にする",
                    "Flat normal (128, 128, 255)",
                ),
                false,
                free,
                16.0,
            )
            .clicked()
            {
                app.apply(Action::Mat(MatAction::NormalFlat));
            }
            let [x, y] = app.mat.normal;
            let nx = slider_row(
                ui,
                rows,
                "mat.normal.x",
                lang.pick("傾き X", "Tilt X"),
                x,
                (-1.0, 1.0),
                two_decimals(),
                Some(lang.pick(
                    "法線の向き: +1 で右に傾く",
                    "The normal as a direction: +1 leans right",
                )),
                free,
            );
            let ny = slider_row(
                ui,
                rows,
                "mat.normal.y",
                lang.pick("傾き Y", "Tilt Y"),
                y,
                (-1.0, 1.0),
                two_decimals(),
                Some(lang.pick(
                    "+1 で上に傾く（OpenGL・Unity）",
                    "+1 leans up (OpenGL / Unity)",
                )),
                free,
            );
            if nx.is_some() || ny.is_some() {
                app.mat.set_normal(nx.unwrap_or(x), ny.unwrap_or(y));
            }
        }
        other => {
            let value = app.mat.scalar(other).unwrap_or(0.0);
            let tip = lang.pick(
                format!("ストロークが塗る{name}の値"),
                format!("The {name} value the stroke paints"),
            );
            let id = format!("mat.value.{}", other.index());
            if let Some(v) = slider_row(
                ui,
                rows,
                &id,
                &name,
                value,
                (0.0, 1.0),
                two_decimals(),
                Some(&tip),
                free,
            ) {
                app.mat.set_scalar(other, v);
            }
        }
    }
}

/// 名前と色の見本の行（見本が押されたか）。
fn label_and_swatch(
    ui: &mut Ui,
    row: Rect,
    name: &str,
    id: &str,
    color: [f32; 4],
    tooltip: &str,
) -> bool {
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(80.0, row.height())),
        name,
        t::LABEL,
        w::Align::Left,
    );
    let swatch = Rect::from_min_size(
        pos2(row.left() + 84.0, row.top() + 1.0),
        vec2(36.0, row.height() - 2.0),
    );
    w::color_swatch(ui, swatch, id, color, tooltip, true).clicked()
}
