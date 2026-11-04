//! ポインターレイヤーのプロパティ（塗りつぶしの欄と同じ並びの 1 つの大見出し）: 指す先のドロップダウン（このテクスチャセットのレイヤー・
//! グループ、このセットのレイヤーセット、ほかのテクスチャセットのレイヤーセット。選ぶと循環・深すぎる・自分自身の物は押せず、理由は
//! ツールチップ）と、有効なチャンネルごとの読む元のチャンネル。読めない理由は警告の印と短い文。値は `Action::Fx` を通る（1 回の Undo）。

use egui::{pos2, vec2, Rect, Sense, Ui};
use yolu_core::{Channel, InactiveReason, InactiveTarget, LayerId, PointerIssue, PointerTarget};

use super::properties::{group_label, section};
use crate::fx::pointers::PointerOp;
use crate::fx::FxOp;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Rows};

const LABEL_W: f32 = 86.0;

/// ポインターレイヤーの欄。
pub fn section_for(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, id: LayerId, enabled: bool) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "pointer",
        lang.t("panels-pointer"),
        crate::m2::layer_kind_icon(yolu_core::LayerKind::Pointer),
        None,
    );
    if !open {
        return;
    }
    let Some(target) = app
        .session
        .doc
        .layer(id)
        .and_then(|l| l.pointer())
        .map(|p| p.target())
    else {
        return;
    };
    let index = app.session.sets.current_index();
    let problems: Vec<(Option<Channel>, PointerIssue)> = app
        .inactive_effects_of(index)
        .into_iter()
        .filter(|e| e.layer == id)
        .filter_map(|e| match (e.target, e.reason) {
            (InactiveTarget::Pointer(c), InactiveReason::Pointer(i)) => Some((c, i)),
            _ => None,
        })
        .collect();
    let ctx = ui.ctx().clone();
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let value = app.pointer_target_label(target);
    let (response, anchor) = w::dropdown(
        ui,
        r,
        ("pointer.target", id.0),
        Some(lang.t("panels-pointer-target")),
        &value,
        None,
        enabled,
        LABEL_W,
    );
    if response.clicked() {
        open_popup(app, &ctx, Popup::PointerTarget(id), anchor);
    }
    if let Some((_, issue)) = problems.iter().find(|(c, _)| c.is_none()) {
        warn_row(
            ui,
            rows,
            &lang.inactive_reason(&InactiveReason::Pointer(*issue)),
        );
    }
    source_rows(ui, app, rows, id, target, &problems, enabled);
}

/// 有効なチャンネルごとの読む元の行。
fn source_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    target: PointerTarget,
    problems: &[(Option<Channel>, PointerIssue)],
    enabled: bool,
) {
    let lang = app.lang;
    let Some(layer) = app.session.doc.layer(id) else {
        return;
    };
    let settings = layer.pointer().cloned();
    let channels = layer.enabled_channels();
    if channels.is_empty() {
        return;
    }
    let sources = app.pointer_source_channels(target);
    group_label(ui, rows, lang.t("panels-pointer-sources"));
    let ctx = ui.ctx().clone();
    for c in channels {
        let name = crate::m2::channel_name(lang, &app.session.doc, c);
        let source = settings.as_ref().map_or(c, |p| p.source(c));
        let value = sources
            .iter()
            .find(|(s, _)| *s == source)
            .map_or_else(|| format!("#{}", source.index()), |(_, n)| n.clone());
        let r = rows.row(t::ROW_HEIGHT, 4.0);
        let (response, anchor) = w::dropdown(
            ui,
            r,
            ("pointer.source", id.0, c.index()),
            Some(&name),
            &value,
            None,
            enabled,
            LABEL_W,
        );
        if response.clicked() {
            open_popup(app, &ctx, Popup::PointerSource(id, c), anchor);
        }
        if let Some((_, issue)) = problems.iter().find(|(p, _)| *p == Some(c)) {
            warn_row(
                ui,
                rows,
                &lang.inactive_reason(&InactiveReason::Pointer(*issue)),
            );
        }
    }
}

fn open_popup(app: &mut AppState, ctx: &egui::Context, popup: Popup, anchor: Rect) {
    app.ui.popup = Some(OpenPopup {
        kind: PopupKind::M2(popup),
        state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
    });
}

/// 読めない理由の行（警告の印と短い理由。入りきらなければ切り、全文はツールチップ）。
fn warn_row(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(t::ROW_HEIGHT, 2.0);
    let p = ui.painter();
    w::icon(
        p,
        Rect::from_min_size(r.min, vec2(18.0, r.height())),
        "warning",
        t::WARNING,
        14.0,
    );
    let area = Rect::from_min_max(pos2(r.left() + 20.0, r.top()), r.max);
    let shown = w::fit(p, text, area.width(), t::LABEL_DIM);
    w::text(
        p,
        area,
        &shown,
        t::LABEL_DIM.with_color(t::WARNING),
        w::Align::Left,
    );
    ui.interact(r, ui.id().with(("pointer.warn", text)), Sense::hover())
        .on_hover_text(text);
}

/// ポインターレイヤーのポップアップの項目（指す先・読む元）。
pub fn entries(app: &AppState, popup: Popup) -> Vec<Entry<Action>> {
    match popup {
        Popup::PointerTarget(layer) => target_entries(app, layer),
        Popup::PointerSource(layer, channel) => source_entries(app, layer, channel),
        _ => Vec::new(),
    }
}

/// 指す先の一覧（節ごとの見出し。レイヤー・グループは入れ子を字下げ。押せない候補は理由をツールチップに）。
fn target_entries(app: &AppState, layer: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = app.can_edit();
    let current = app
        .session
        .doc
        .layer(layer)
        .and_then(|l| l.pointer())
        .map(|p| p.target());
    let mut v = Vec::new();
    for section in app.pointer_candidates(layer) {
        if section.items.is_empty() {
            continue;
        }
        if !v.is_empty() {
            v.push(Entry::Separator);
        }
        v.push(Entry::Heading(section.heading));
        for c in section.items {
            let label = format!("{}{}", "    ".repeat(c.depth), c.label);
            let mut e = Entry::item(
                label,
                Action::Fx(FxOp::Pointer(PointerOp::Target {
                    layer,
                    target: c.target,
                })),
            )
            .radio(current == Some(c.target))
            .enabled(free && c.refused.is_none());
            if let Some(why) = c.refused {
                e = e.tooltip(lang.inactive_reason(&InactiveReason::Pointer(why)));
            }
            v.push(e);
        }
    }
    v
}

/// 読む元のチャンネルの一覧（指す先の文書のチャンネル）。
fn source_entries(app: &AppState, layer: LayerId, channel: Channel) -> Vec<Entry<Action>> {
    let free = app.can_edit();
    let Some(settings) = app.session.doc.layer(layer).and_then(|l| l.pointer()) else {
        return Vec::new();
    };
    let current = settings.source(channel);
    app.pointer_source_channels(settings.target())
        .into_iter()
        .map(|(c, name)| {
            Entry::item(
                name,
                Action::Fx(FxOp::Pointer(PointerOp::Source {
                    layer,
                    channel,
                    source: (c != channel).then_some(c),
                })),
            )
            .radio(current == c)
            .enabled(free)
        })
        .collect()
}
