//! テクスチャセットのパネル（Substance Painter の Texture Set List の並び）: 行は目・名前・状態のアイコン・解像度。押すと今のセットを
//! 替え、ダブルクリックで名前を変え、右クリックでメニュー。下の 1 行に今のセットのマテリアルと Unity での見え方。
//!
//! 状態のアイコン: 鍵 = 読むだけ（core で扱えない中身がある）、切れた鎖 = 今のモデルに無いマテリアル（鍵は残してある）、
//! 同期 = Unity に見せている、注意 = Unity 側に Color の流し込み先が無い（描いても Unity には見えない）。画面には名前と短い状態だけを
//! 出し、説明はツールチップに置く。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const ROW_HEIGHT: f32 = 28.0;
/// 下の説明の行の高さ。
pub const FOOTER_HEIGHT: f32 = 22.0;

/// セットの見え方: アイコンと色、画面に出す短い状態、ツールチップの説明。
#[derive(Clone, Debug, PartialEq)]
pub struct SetLook {
    pub icon: &'static str,
    pub color: Color32,
    pub label: &'static str,
    pub tooltip: String,
}

fn look(
    icon: &'static str,
    color: Color32,
    label: &'static str,
    tooltip: String,
) -> Option<SetLook> {
    Some(SetLook {
        icon,
        color,
        label,
        tooltip,
    })
}

/// セットの見え方（無ければ普通）。
pub fn set_state(app: &AppState, index: usize) -> Option<SetLook> {
    let set = app.sets.get(index)?;
    if let Some(reason) = &set.read_only {
        return look(
            "lock",
            t::WARNING,
            "読むだけ",
            format!("読むだけ: {reason}"),
        );
    }
    let model = app.model.as_ref()?;
    let Some(material) = set.bound else {
        return look(
            "link_off",
            t::TEXT_DIM,
            "モデルに無い",
            "今のモデルに無いマテリアル。鍵は残してあり、そのマテリアルのあるモデルでまた付く"
                .into(),
        );
    };
    // 流し込み先は Live Link のモデルだけの話（FBX・試しの人形は Unity に出さないので、無くても警告しない）
    let routed = !model.is_link()
        || model.materials.get(material as usize).is_some_and(|m| {
            m.routes
                .iter()
                .any(|r| r.channel == yolu_protocol::channel::COLOR)
        });
    if !set.visible {
        return look(
            "visibility_off",
            t::TEXT_DIM,
            "隠している",
            "3D ビューと Unity に見せていない".into(),
        );
    }
    if !routed {
        return look(
            "warning",
            t::WARNING,
            "流し込み先なし",
            "Unity 側にこのマテリアルの Color の流し込み先が無い（Unity には見えない）".into(),
        );
    }
    if app.link.published.contains(&set.uid) {
        return look(
            "sync",
            t::ACCENT,
            "Unity に表示中",
            "Unity に見せている".into(),
        );
    }
    None
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let list = Rect::from_min_max(
        r.min,
        pos2(r.right(), (r.bottom() - FOOTER_HEIGHT).max(r.top())),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let n = app.sets.len();
    let content = n as f32 * ROW_HEIGHT;
    let max_scroll = (content - list.height()).max(0.0);
    if ui.rect_contains_pointer(list) {
        app.set_scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    app.set_scroll = app.set_scroll.clamp(0.0, max_scroll);
    let row_width = list.width() - if max_scroll > 0.0 { 10.0 } else { 0.0 };
    for index in 0..n {
        let row = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + index as f32 * ROW_HEIGHT - app.set_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        set_row(ui, app, &ctx, list, row, index);
    }
    if max_scroll > 0.0 {
        let bar_h = list.height() * list.height() / content;
        let bar_y = list.top() + (list.height() - bar_h) * app.set_scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }

    // 下: 今のセットのマテリアルと見え方
    let footer = Rect::from_min_max(pos2(r.left(), list.bottom()), r.max);
    let p = ui.painter();
    w::fill(p, footer, t::PANEL_HEADER);
    let current = app.sets.current_index();
    let set = app.sets.current();
    let material = crate::sets::describe_material(&set.material);
    let (text, tip) = match set_state(app, current) {
        Some(l) => (
            format!("{material} · {}", l.label),
            format!("{material}\n{}", l.tooltip),
        ),
        None => (material.clone(), material),
    };
    let inner = footer.shrink2(vec2(t::PADDING, 0.0));
    let shown = w::fit(p, &text, inner.width(), t::LABEL_SMALL);
    w::text(p, inner, &shown, t::LABEL_SMALL, Align::Left);
    ui.interact(footer, ui.id().with("sets.footer"), Sense::hover())
        .on_hover_text(tip);
}

fn set_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    index: usize,
) {
    let Some(set) = app.sets.get(index) else {
        return;
    };
    let (uid, name, visible) = (set.uid, set.name.clone(), set.visible);
    let selected = index == app.sets.current_index();
    let free = !app.is_stroking();
    let doc = app.set_doc(index);
    let resolution = if doc.width() == doc.height() {
        doc.width().to_string()
    } else {
        format!("{}×{}", doc.width(), doc.height())
    };
    let state = set_state(app, index);
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("set.row", uid)),
        if free { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter_at(list);
    if selected {
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
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let res_w = w::text_width(&painter, &resolution, t::LABEL_DIM) + 4.0;
    let res_rect = Rect::from_min_max(
        pos2(row.right() - 8.0 - res_w, row.top()),
        pos2(row.right() - 8.0, row.bottom()),
    );
    let icon_rect = Rect::from_min_size(
        pos2(res_rect.left() - 22.0, row.top() + 5.0),
        vec2(18.0, row.height() - 10.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(eye.right() + 6.0, row.top() + 4.0),
        pos2(icon_rect.left() - 4.0, row.bottom() - 4.0),
    );

    if response.clicked() {
        app.apply(Action::SelectSet(uid));
        if app.renaming_set != Some(uid) {
            app.renaming_set = None;
        }
    }
    if response.double_clicked()
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.apply(Action::StartRenameSet(uid));
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            app.popup = Some(OpenPopup {
                kind: PopupKind::SetContext(uid),
                state: PopupState::new(ctx, context_anchor(at)),
            });
        }
    }

    if w::icon_button(
        ui,
        eye,
        ("set.eye", uid),
        if visible {
            "visibility"
        } else {
            "visibility_off"
        },
        if visible {
            "隠す（3D ビューと Unity に見せない）"
        } else {
            "見せる"
        },
        false,
        true,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleSetVisible(uid));
    }

    let painter = ui.painter_at(list);
    if let Some(l) = &state {
        w::icon(&painter, icon_rect, l.icon, l.color, 15.0);
        ui.interact(
            icon_rect.intersect(list),
            ui.make_persistent_id(("set.state", uid)),
            Sense::hover(),
        )
        .on_hover_text(l.tooltip.as_str());
    }
    w::text(&painter, res_rect, &resolution, t::LABEL_DIM, Align::Right);

    if app.renaming_set == Some(uid) {
        let first = !app.rename_set_started;
        app.rename_set_started = true;
        let out = w::text_field(ui, name_rect, ("set.rename", uid), &name, None, first);
        if let Some(next) = out.committed {
            if let Err(e) = app.rename_set(uid, &next) {
                app.message = e;
            }
        }
        if !first && !out.focused {
            app.renaming_set = None;
        }
    } else {
        let color = if selected {
            Color32::WHITE
        } else if !visible {
            t::TEXT_DIM
        } else {
            t::TEXT
        };
        let shown = w::fit(&painter, &name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, free, selected, &name));
}
