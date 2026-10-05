//! チャンネルのパネル（Substance Painter のテクスチャセットのチャンネルの一覧）: 文書のチャンネル（標準の 6 つとユーザーチャンネル）を
//! 行にし、筆のボタンで描くチャンネル、目で 2D のキャンバスに出すチャンネルを選ぶ。ユーザーチャンネルは名前（ダブルクリック）・
//! 種類（右端の名前を押す）を変えられ、下の帯で足す・消す。どれも文書の操作（1 回の Undo）で、画面だけの選択は `UiOp`。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::properties::{group_label, open_popup, section, slider_row, toggle_row};
use crate::engine::{Channel, ChannelInfo, NormalSettings};
use crate::m2::{self, direction_name, Edit, UiOp};
use crate::m2_menu::{edges_name, Popup};
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows};

pub const ROW_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// 右端の種類の名前の幅。
const KIND_WIDTH: f32 = 66.0;

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
    // 一覧と、その下の Normal の設定は 1 つのスクロールで動く（下の帯の足す・消すは動かない）
    let body = Rect::from_min_max(
        r.min,
        pos2(r.right(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
    );
    let channels = app.doc.channels();
    let list_height = channels.len() as f32 * ROW_HEIGHT;
    let bar = Scroll::begin(ui, body, app.m2.channels_content, &mut app.m2.channel_scroll);
    let scroll = app.m2.channel_scroll;
    w::fill(&ui.painter_at(body), body, t::PANEL_BG);
    let list = Rect::from_min_size(
        pos2(body.left(), body.top() - scroll),
        vec2(body.width(), list_height),
    );
    w::fill(&ui.painter_at(body), list, t::CONTROL_BG);
    // つまみは行に重ねて出す（浮かぶつまみ。カラーセットの欄と同じ。行の幅を狭めると、名前が切れる）
    let row_width = list.width();
    for (i, channel) in channels.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW_HEIGHT),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < body.top() || row.top() > body.bottom() {
            continue;
        }
        channel_row(ui, app, &ctx, body, row, *channel);
    }
    // 一覧の下: Normal の設定
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer_clip));
    let area = Rect::from_min_max(
        pos2(body.left(), list.bottom()),
        pos2(
            body.right() - bar.reserved(),
            body.bottom().max(list.bottom() + 1.0),
        ),
    );
    let mut rows = Rows::new(area, 0.0);
    normal_section(ui, app, &mut rows, &ctx);
    rows.indent = 0.0;
    rows.space(8.0);
    app.m2.channels_content = list_height + rows.used();
    ui.set_clip_rect(outer_clip);
    // スライダーのドラッグを離したら、まとめていた変更を 1 回の Undo にする
    if !ui.input(|i| i.pointer.primary_down()) {
        app.m2_end_drag();
    }
    bar.end(ui, "channels.scroll", &mut app.m2.channel_scroll);

    // 下: 足す・消す
    let bar = Rect::from_min_size(
        pos2(r.left(), body.bottom()),
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

/// Normal の設定の節（文書の Normal の出力: Height から作る・強さ・端・ファイルの Y の向き）。値の変更は文書の操作（1 回の Undo。
/// 強さのスライダーのドラッグは離したとき 1 回にまとまる）。強さと端は Height から作るときだけ効く。
fn normal_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    // 初めは閉じておく（チャンネルの一覧を押しのけない。開いた・閉じたは覚える）
    app.sections.entry("normal").or_insert(false);
    let (open, _) = section(
        ui,
        app,
        rows,
        "normal",
        lang.pick("ノーマル", "Normal"),
        "3d_rotation",
        None,
    );
    if !open {
        return;
    }
    let settings = app.doc.normal_settings();
    let enabled = app.can_edit();
    let derive = settings.derive_from_height();
    let edit = |app: &mut AppState, settings: NormalSettings, coalesce: bool| {
        app.apply(Action::M2(Edit::NormalSettings { settings, coalesce }));
    };
    if let Some(next) = toggle_row(
        ui,
        rows,
        "normal.derive",
        lang.pick("ハイト → ノーマル", "Height → Normal"),
        derive,
        Some(lang.pick(
            "ノーマルの出力（プレビュー・.ylp のテクスチャ・書き出し）で、描いたノーマルのレイヤーの下にハイトのチャンネルから作った法線を足します。ハイトから毎回作り直し、レイヤーには描きません",
            "Adds the normal derived from the Height channel under the painted Normal layers in the Normal output (preview, .ylp texture, exports). It is regenerated from Height, never painted into a layer",
        )),
        enabled,
    ) {
        edit(app, settings.with_derive(next), false);
    }
    if let Some(value) = slider_row(
        ui,
        rows,
        "normal.strength",
        lang.pick("強さ", "Strength"),
        settings.strength() as f32,
        (
            -(NormalSettings::MAX_STRENGTH as f32),
            NormalSettings::MAX_STRENGTH as f32,
        ),
        NumberFormat {
            decimals: 2,
            trim: true,
            suffix: "",
        },
        Some(lang.pick(
            "ハイトの全範囲（0 → 1）で何テクセル分盛り上がるか。マイナスにすると凸が凹になります",
            "Texels of rise for the full height range (0 → 1). Negative turns bumps into dents",
        )),
        enabled && derive,
    ) {
        if let Ok(next) = settings.with_strength(value as f64) {
            edit(app, next, true);
        }
    }
    // 狭い欄でも値を切らないよう、名前を上に置いて箱は幅いっぱいに
    let dropdown = |ui: &mut Ui, rows: &mut Rows, id: &str, value: &str, tip: &str, enabled: bool| {
        let r = rows.row(t::ROW_HEIGHT, 4.0);
        let (response, at) = w::dropdown(ui, r, id, None, value, Some(tip), enabled, 0.0);
        response.clicked().then_some(at)
    };
    group_label(ui, rows, lang.pick("端", "Edges"));
    if let Some(at) = dropdown(
        ui,
        rows,
        "normal.edges",
        edges_name(lang, settings.edges()),
        lang.pick(
            "クランプ: キャンバスの端の傾きは端のテクセルで求めます。ラップ: 反対側の端を読みます（タイルするテクスチャ）",
            "Clamp: the slope at the canvas edge uses the edge texel. Wrap: it reads the opposite edge (tiling textures)",
        ),
        enabled && derive,
    ) {
        open_popup(app, ctx, Popup::NormalEdges, at, at.width());
    }
    group_label(ui, rows, lang.pick("ファイルの Y", "File Y"));
    if let Some(at) = dropdown(
        ui,
        rows,
        "normal.direction",
        direction_name(settings.file_direction()),
        lang.pick(
            "画像の書き出し・PNG・PSD で書くノーマルの画像の緑の向き。Unity は OpenGL（Y+）で、.ylp のテクスチャとプレビューは常に OpenGL です",
            "Green direction of Normal images written by the exports. Unity uses OpenGL (Y+); the .ylp texture and the preview always do",
        ),
        enabled,
    ) {
        open_popup(app, ctx, Popup::NormalDirection, at, at.width());
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
    // このチャンネルを読む lilToon のスロット（見た目が lilToon のとき）: チャンネルの印の右上に小さな点、スロットの名前はツールチップ
    let reading = crate::look::slots_reading(&app.doc, channel);

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

    if !reading.is_empty() {
        let painter = ui.painter_at(list);
        let dot = pos2(icon_rect.right() - 1.0, row.top() + 7.0);
        painter.circle_filled(dot, 3.0, t::ACCENT);
        let names: Vec<&str> = reading.iter().map(|s| s.label(lang)).collect();
        let tip = format!("lilToon: {}", names.join(lang.pick("、", ", ")));
        let hit = icon_rect.intersect(list);
        let response = ui.interact(hit, ui.make_persistent_id(("channel.look", channel.index())), Sense::hover());
        let label = tip.clone();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
        response.on_hover_text(tip);
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
