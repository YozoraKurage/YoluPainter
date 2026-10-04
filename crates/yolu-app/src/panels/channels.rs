//! チャンネルのパネル（Substance Painter のテクスチャセットのチャンネルの一覧）: 文書のチャンネル（標準の 6 つとユーザーチャンネル）を
//! 行にし、筆のボタンで描くチャンネル、目で 2D のキャンバスに出すチャンネルを選ぶ。ユーザーチャンネルは名前（ダブルクリック）・
//! 種類（右端の名前を押す）を変えられ、下の帯で足す・消す。どれも文書の操作（1 回の Undo）で、画面だけの選択は `UiOp`。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::engine::{Channel, ChannelInfo};
use crate::m2::{self, Edit, UiOp};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const ROW_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// 右端の種類の名前の幅。
const KIND_WIDTH: f32 = 74.0;

fn open(app: &mut AppState, ctx: &egui::Context, kind: PopupKind, anchor: Rect) {
    app.popup = Some(OpenPopup {
        kind,
        state: PopupState::new(ctx, anchor),
    });
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    let enabled = app.can_edit();
    let list = Rect::from_min_max(
        r.min,
        pos2(r.right(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let channels = app.doc.channels();
    let content = channels.len() as f32 * ROW_HEIGHT;
    let max_scroll = (content - list.height()).max(0.0);
    if ui.rect_contains_pointer(list) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.m2.channel_scroll -= wheel;
    }
    app.m2.channel_scroll = app.m2.channel_scroll.clamp(0.0, max_scroll);
    let row_width = list.width() - if max_scroll > 0.0 { 10.0 } else { 0.0 };
    for (i, channel) in channels.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + i as f32 * ROW_HEIGHT - app.m2.channel_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        channel_row(ui, app, &ctx, list, row, *channel);
    }
    if max_scroll > 0.0 {
        let bar_h = list.height() * list.height() / content;
        let bar_y = list.top() + (list.height() - bar_h) * app.m2.channel_scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }

    // 下: 足す・消す
    let bar = Rect::from_min_size(
        pos2(r.left(), list.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    let add = button(bar.left() + 4.0);
    if w::icon_button(
        ui,
        add,
        "channels.add",
        "add",
        lang.pick("チャンネルを追加", "Add Channel"),
        false,
        enabled,
        18.0,
    )
    .clicked()
    {
        open(app, &ctx, PopupKind::M2(Popup::NewChannel), add);
    }
    let paint = app.m2.paint_channel;
    if w::icon_button(
        ui,
        button(bar.right() - 4.0 - 26.0),
        "channels.delete",
        "delete",
        lang.pick("チャンネルを削除", "Delete Channel"),
        false,
        enabled && !paint.is_standard(),
        17.0,
    )
    .clicked()
    {
        app.apply(Action::M2(Edit::RemoveChannel(paint)));
    }
}

fn channel_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    channel: Channel,
) {
    let lang = app.lang;
    let name = m2::channel_name(lang, &app.doc, channel);
    let Some(info) = app.doc.channel_info(channel).cloned() else {
        return;
    };
    let painting = app.m2.paint_channel == channel;
    let showing = app.m2.display_channel == channel;
    let user = !channel.is_standard();
    let enabled = app.can_edit();
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("channel.row", channel.index())),
        Sense::click(),
    );
    let painter = ui.painter_at(list);
    if painting {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if response.hovered() {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );

    let paint_btn = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 2.0),
        vec2(24.0, row.height() - 4.0),
    );
    let eye = Rect::from_min_size(
        pos2(paint_btn.right() + 2.0, row.top() + 2.0),
        vec2(24.0, row.height() - 4.0),
    );
    let icon_rect =
        Rect::from_min_size(pos2(eye.right() + 4.0, row.top()), vec2(18.0, row.height()));
    let kind_rect = Rect::from_min_max(
        pos2(row.right() - KIND_WIDTH - 4.0, row.top() + 4.0),
        pos2(row.right() - 4.0, row.bottom() - 4.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(icon_rect.right() + 4.0, row.top() + 3.0),
        pos2(kind_rect.left() - 4.0, row.bottom() - 3.0),
    );

    if response.clicked() {
        app.apply(Action::M2Ui(UiOp::PaintChannel(channel)));
        if app.m2.renaming_channel != Some(channel) {
            app.m2.renaming_channel = None;
        }
    }
    // 直前に別の所を押していると、egui は 2 度目を 3 回押しとして数えることがある（位置は直前の 1 回としか比べない）ので、どちらでも
    if (response.double_clicked() || response.triple_clicked())
        && user
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.apply(Action::M2Ui(UiOp::RenameChannel(channel)));
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            open(
                app,
                ctx,
                PopupKind::M2(Popup::ChannelContext(channel)),
                context_anchor(at),
            );
        }
    }

    if w::icon_button(
        ui,
        paint_btn,
        ("channel.paint", channel.index()),
        "paint_brush",
        &lang.pick(
            format!("{name} を描くチャンネルにする"),
            format!("Paint {name}"),
        ),
        painting,
        enabled,
        15.0,
    )
    .clicked()
    {
        app.apply(Action::M2Ui(UiOp::PaintChannel(channel)));
    }
    if w::icon_button(
        ui,
        eye,
        ("channel.eye", channel.index()),
        if showing {
            "visibility"
        } else {
            "visibility_off"
        },
        &lang.pick(
            format!("{name} をキャンバスに出す"),
            format!("Show {name} in the canvas"),
        ),
        showing,
        true,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::M2Ui(UiOp::DisplayChannel(channel)));
    }
    let painter = ui.painter_at(list);
    w::icon(
        &painter,
        icon_rect,
        m2::channel_icon(channel),
        if painting {
            Color32::WHITE
        } else {
            t::TEXT_DIM
        },
        15.0,
    );

    // 名前（ユーザーチャンネルはダブルクリックで変える）
    if app.m2.renaming_channel == Some(channel) {
        let first = !app.m2.rename_channel_started;
        app.m2.rename_channel_started = true;
        let out = w::text_field(
            ui,
            name_rect,
            ("channel.rename", channel.index()),
            &info.name,
            None,
            first,
        );
        if let Some(next) = out.committed {
            let next = next.trim().to_owned();
            if !next.is_empty() && next != info.name {
                app.apply(Action::M2(Edit::SetChannel {
                    channel,
                    info: ChannelInfo {
                        name: next,
                        ..info.clone()
                    },
                }));
            }
        }
        if !first && !out.focused {
            app.m2.renaming_channel = None;
        }
    } else {
        let shown = w::fit(&painter, &name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(if painting { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );
    }

    // 種類（ユーザーチャンネルは押すと替えられる）
    let kind_name = m2::kind_label(lang, info.kind);
    if user {
        let (response, b) = w::dropdown(
            ui,
            kind_rect,
            ("channel.kind", channel.index()),
            None,
            kind_name,
            Some(lang.pick("チャンネルの種類", "Channel type")),
            enabled,
            0.0,
        );
        if response.clicked() {
            open(app, ctx, PopupKind::M2(Popup::ChannelKind(channel)), b);
        }
    } else {
        w::text(&painter, kind_rect, kind_name, t::LABEL_DIM, Align::Right);
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, painting, &name)
    });
}
