//! 3D ビューの右から出る「ハイド」の引き出し（ハイドのモードの間だけ、ビューごとに 1 つ。開閉はビューごとで保存しない。既定は開いた形）。
//! 閉じた形はビューの右端の縦の真ん中につまみだけ（「‹」）、開いた形は欄が右端から出て、つまみは欄の左の縁に付く（「›」）。欄の上端は右上の
//! アイコンの列の下、下端はビューの下から少し空け、あふれたらスクロール。中身は、選んだ面を隠す・すべて表示のボタンと、「隠している物」
//! 「ボーンで隠す」「プリセット」「ハイドの設定」の節（ポーズの欄の「面を隠す」と同じ状態を見る）。
//!
//! 欄とつまみは別のレイヤー（`egui::Area` の `Order::Middle`）に置き、矩形の全部を押しの受け手にするので、引き出しの下でビューの押し
//! （描く・回す・選ぶ）は始まらない。

use egui::{pos2, vec2, Color32, Order, Rect, Sense, Shape, Stroke, Ui};

use crate::panels::properties::{percent_row, section_default};
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};
use crate::view3d::hide::HideAction;
use crate::view3d::pose::hide as bone_hide;

const DRAWER_WIDTH: f32 = 280.0;
const HANDLE_WIDTH: f32 = 14.0;
const HANDLE_HEIGHT: f32 = 48.0;
const ICON_W: f32 = 22.0;
const SAVE_W: f32 = 74.0;

/// 引き出しの欄とつまみの矩形（`top` は右上のアイコンの列の下の端）。
pub struct Layout {
    pub panel: Option<Rect>,
    pub handle: Rect,
}

pub fn layout(view: Rect, top: f32, open: bool) -> Layout {
    let width = DRAWER_WIDTH.min((view.width() * 0.6).max(HANDLE_WIDTH * 4.0));
    let top = top.max(view.top());
    let bottom = (view.bottom() - 12.0).max(top + HANDLE_HEIGHT);
    let panel = Rect::from_min_max(pos2(view.right() - width, top), pos2(view.right(), bottom));
    let cy = view
        .center()
        .y
        .clamp(top + HANDLE_HEIGHT / 2.0, bottom - HANDLE_HEIGHT / 2.0);
    let edge = if open { panel.left() } else { view.right() };
    Layout {
        panel: open.then_some(panel),
        handle: Rect::from_min_size(
            pos2(edge - HANDLE_WIDTH, cy - HANDLE_HEIGHT / 2.0),
            vec2(HANDLE_WIDTH, HANDLE_HEIGHT),
        ),
    }
}

/// つまみの上の「‹」「›」（左向きの矢印のアイコンが無いので、線で描く）。
fn chevron(p: &egui::Painter, r: Rect, left: bool, color: Color32) {
    let c = r.center();
    let d = if left { -1.0 } else { 1.0 };
    let pts = vec![
        c + vec2(-2.5 * d, -5.0),
        c + vec2(2.5 * d, 0.0),
        c + vec2(-2.5 * d, 5.0),
    ];
    p.add(Shape::line(pts, Stroke::new(1.5, color)));
}

/// 引き出しを描く（`top` は右上のアイコンの列の下の端。`live` は今のビュー。`free` は描いていない間で、押せるかを決める）。
pub fn show(ui: &mut Ui, app: &mut AppState, view: Rect, top: f32, live: bool, free: bool) {
    if app.mode != crate::mode::EditorMode::Hide || app.view3d.full_model().is_none() {
        return;
    }
    let lang = app.lang;
    let id = app.view3d.active_view();
    let open = app.view3d.view.hide.drawer_open;
    let l = layout(view, top, open);
    let full = l.panel.map_or(l.handle, |p| p.union(l.handle));
    let mut toggle = false;
    egui::Area::new(egui::Id::new(("yolu.view3d.hide.drawer", id.0)))
        .order(Order::Middle)
        .fixed_pos(full.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(full.size(), Sense::hover());
            // 欄とつまみの全部を取る（部品の間の押しも、下のビューに落とさない）
            let _ = ui.interact(
                l.handle,
                ui.id().with("drawer.block.handle"),
                Sense::click_and_drag(),
            );
            if let Some(panel) = l.panel {
                let _ = ui.interact(panel, ui.id().with("drawer.block"), Sense::click_and_drag());
            }
            let p = ui.painter().clone();
            if let Some(panel) = l.panel {
                // ビューとの境の薄い影
                for k in 0..8 {
                    let x = panel.left() - 1.0 - k as f32;
                    let alpha = (46.0 - k as f32 * 5.5).max(0.0) as u8;
                    w::fill(
                        &p,
                        Rect::from_min_max(pos2(x, panel.top()), pos2(x + 1.0, panel.bottom())),
                        Color32::from_black_alpha(alpha),
                    );
                }
                w::fill(&p, panel, t::PANEL_BG);
                w::vline(&p, panel.left(), panel.top(), panel.bottom(), t::BORDER);
                let title =
                    Rect::from_min_size(panel.min, vec2(panel.width(), t::PANEL_HEADER_HEIGHT));
                w::panel_title(&p, title, lang.t("hide-hide"), Some("visibility_off"));
                let body = Rect::from_min_max(pos2(panel.left(), title.bottom()), panel.max);
                if live {
                    body_ui(ui, app, body, free);
                } else {
                    // 今のビューでないビューの引き出しは見せるだけ（ポインタが乗れば今のビューになる）
                    w::enabled_scope(ui, ("hide.drawer.idle", id.0), false, |ui| {
                        body_ui(ui, app, body, free)
                    });
                }
            }
            let handle = l.handle;
            let response = ui.interact(
                handle,
                ui.id().with(("drawer.handle", id.0)),
                Sense::click(),
            );
            let bg = if response.hovered() {
                t::CONTROL_HOVER
            } else {
                t::PANEL_HEADER
            };
            let radius = egui::CornerRadius {
                nw: 6,
                sw: 6,
                ne: 0,
                se: 0,
            };
            p.rect_filled(handle, radius, bg);
            p.rect_stroke(
                handle,
                radius,
                Stroke::new(1.0, t::BORDER),
                egui::StrokeKind::Inside,
            );
            chevron(
                &p,
                handle,
                !open,
                if response.hovered() {
                    Color32::WHITE
                } else {
                    t::TEXT
                },
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    true,
                    open,
                    lang.t("hide-hide"),
                )
            });
            if response.on_hover_text(lang.t("hide-hide")).clicked() {
                toggle = true;
            }
        });
    if toggle && live {
        app.apply(Action::Hide(HideAction::ToggleDrawer(id)));
    }
}

/// 欄の中身（あふれたらスクロール）。
fn body_ui(ui: &mut Ui, app: &mut AppState, body: Rect, free: bool) {
    let content = app.view3d.view.hide.drawer_content;
    let mut offset = app.view3d.view.hide.drawer_scroll;
    let scroller = Scroll::new(body, content, &mut offset);
    if ui.rect_contains_pointer(body) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        offset = (offset - wheel).clamp(0.0, scroller.max);
    }
    let area = Rect::from_min_max(
        pos2(body.left(), body.top() - offset),
        pos2(body.right() - scroller.reserved(), body.bottom()),
    );
    let outer = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer));
    let mut rows = Rows::new(area, 6.0);
    contents(ui, app, &mut rows, body.height() >= 480.0, free);
    rows.indent = 0.0;
    rows.space(6.0);
    app.view3d.view.hide.drawer_content = rows.used();
    ui.set_clip_rect(outer);
    scroller.end(
        ui,
        ("hide.drawer.scroll", app.view3d.active_view().0),
        &mut offset,
    );
    app.view3d.view.hide.drawer_scroll = offset;
}

/// 欄の中身の並び（上のボタン・隠している物・ボーンの節・設定）。
fn contents(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, tall: bool, free: bool) {
    let lang = app.lang;
    top_buttons(ui, app, rows, free);
    // 隠している物
    let (open, _) = section_default(
        ui,
        app,
        rows,
        "hide.list",
        lang.t("hide-hidden"),
        "visibility_off",
        None,
        true,
        false,
    );
    if open {
        entry_rows(ui, app, rows, free);
    }
    rows.indent = 0.0;

    if app.view3d.pose.session.is_some() {
        bone_sections(ui, app, rows, free, tall);
    }
    settings_section(ui, app, rows);
}

/// 上の 2 つのボタン（選んだ面を隠す・すべて表示）。
fn top_buttons(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let selecting = !app.view3d.view.hide.selected.is_empty();
    let mut action = None;
    let row = rows.row(24.0, 4.0);
    let tip = crate::view3d::hide::strip::named(
        app,
        lang.t("hide-hide-selected-faces"),
        "hide.hide_selected",
    );
    if w::button(
        ui,
        row,
        "hide.drawer.hide",
        &tip,
        false,
        free && selecting,
        None,
        Some("visibility_off"),
    )
    .clicked()
    {
        action = Some(HideAction::HideSelected);
    }
    let row = rows.row(24.0, 4.0);
    let tip = crate::view3d::hide::strip::named(app, lang.t("hide-show-all"), "hide.show_all");
    if w::button(
        ui,
        row,
        "hide.drawer.show",
        &tip,
        false,
        free,
        None,
        Some("visibility"),
    )
    .clicked()
    {
        action = Some(HideAction::ShowAll);
    }
    if let Some(a) = action.take() {
        app.apply(Action::Hide(a));
    }
    rows.space(2.0);
}

/// ボーンで隠す節とプリセットの節（ボーンのあるモデルだけ）。
fn bone_sections(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool, tall: bool) {
    let lang = app.lang;
    let (open, _) = section_default(
        ui,
        app,
        rows,
        "hide.bone",
        lang.t("hide-hide-by-bone"),
        "accessibility",
        None,
        true,
        false,
    );
    if open {
        bone_rows(ui, app, rows, free);
    }
    rows.indent = 0.0;

    let (open, _) = section_default(
        ui,
        app,
        rows,
        "hide.presets",
        lang.t("hide-presets"),
        "save",
        None,
        tall,
        false,
    );
    if open {
        preset_rows(ui, app, rows, free);
    }
    rows.indent = 0.0;
}

/// ハイドの設定の節（輪郭を出す・選択を残す）。
fn settings_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = section_default(
        ui,
        app,
        rows,
        "hide.settings",
        lang.t("hide-settings"),
        "tune",
        None,
        false,
        false,
    );
    if open {
        let row = rows.row(t::ROW_HEIGHT, 2.0);
        let outline = app.view3d.hiding.outline;
        if w::toggle(
            ui,
            row,
            "hide.drawer.outline",
            lang.t("hide-show-outline"),
            outline,
            None,
            true,
        ) != outline
        {
            app.apply(Action::Hide(HideAction::SetOutline(!outline)));
        }
        let row = rows.row(t::ROW_HEIGHT, 2.0);
        let keep = app.view3d.hiding.keep_selection;
        if w::toggle(
            ui,
            row,
            "hide.drawer.keep",
            lang.t("hide-keep-selection"),
            keep,
            None,
            true,
        ) != keep
        {
            app.apply(Action::Hide(HideAction::SetKeepSelection(!keep)));
        }
    }
    rows.indent = 0.0;
}

/// 隠している物 1 行ずつ（アイコン・名前・状態・外す ×）。
fn entry_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let list = crate::view3d::hide::entries(app);
    let p = ui.painter().clone();
    let mut remove = None;
    for (i, e) in list.iter().enumerate() {
        let row = rows.row(t::ROW_HEIGHT, 1.0);
        w::icon(
            &p,
            Rect::from_min_size(row.min, vec2(16.0, row.height())),
            e.icon,
            t::TEXT_DIM,
            13.0,
        );
        let button = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let status_w = if e.status.is_empty() {
            0.0
        } else {
            w::text_width(&p, &e.status, t::LABEL_DIM)
        };
        w::text(
            &p,
            Rect::from_min_max(
                pos2(button.left() - status_w - 4.0, row.top()),
                pos2(button.left() - 2.0, row.bottom()),
            ),
            &e.status,
            t::LABEL_DIM,
            Align::Right,
        );
        let name_rect = Rect::from_min_max(
            pos2(row.left() + 20.0, row.top()),
            pos2(button.left() - status_w - 8.0, row.bottom()),
        );
        w::text(
            &p,
            name_rect,
            &w::fit(&p, &e.name, name_rect.width(), t::LABEL),
            t::LABEL,
            Align::Left,
        );
        if w::icon_button(
            ui,
            button,
            ("hide.drawer.remove", i),
            "close",
            lang.t("panels-remove-this-entry-tip"),
            false,
            free,
            12.0,
        )
        .clicked()
        {
            remove = Some(e.remove);
        }
    }
    if let Some(r) = remove {
        crate::view3d::hide::remove(app, r);
    }
}

/// ボーンで隠す: 選んだボーンの名前の行・子を含める・しきい値・選んだボーンの面を隠す。
fn bone_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    let state = app.view3d.view.hide.bones.clone();
    let selected = app.view3d.pose.session.as_ref().and_then(|s| s.selected);
    let name = selected.and_then(|b| {
        app.view3d
            .pose
            .session
            .as_ref()
            .and_then(|s| s.rig.bones().get(b))
            .map(|b| b.name.clone())
    });
    if let Some(name) = name {
        let row = rows.row(t::ROW_HEIGHT, 2.0);
        let p = ui.painter().clone();
        w::icon(
            &p,
            Rect::from_min_size(row.min, vec2(16.0, row.height())),
            "accessibility",
            t::TEXT_DIM,
            13.0,
        );
        let at = Rect::from_min_max(pos2(row.left() + 20.0, row.top()), row.max);
        w::text(
            &p,
            at,
            &w::fit(&p, &name, at.width(), t::LABEL),
            t::LABEL,
            Align::Left,
        );
    }
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let children = w::toggle(
        ui,
        row,
        "hide.drawer.children",
        lang.t("panels-include-children"),
        state.children,
        None,
        free,
    );
    let threshold = percent_row(
        ui,
        rows,
        "hide.drawer.threshold",
        lang.t("panels-threshold"),
        f64::from(state.threshold),
        (0.0, 1.0),
        None,
        free,
    );
    if children != state.children || threshold.is_some() {
        bone_hide::set_next(
            app,
            threshold.map_or(state.threshold, |v| v as f32),
            children,
        );
    }
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "hide.drawer.bone",
        lang.t("panels-hide-selected-bone"),
        false,
        free && selected.is_some(),
        None,
        Some("visibility_off"),
    )
    .clicked()
    {
        app.apply(Action::Hide(HideAction::HideBone));
    }
}

/// プリセット: 今の隠し方を名前を付けて保存、入れる・外す、消す。
fn preset_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, free: bool) {
    let lang = app.lang;
    rows.space(4.0);
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let field = Rect::from_min_max(row.min, pos2(row.right() - SAVE_W - 4.0, row.bottom()));
    let shown = if app.view3d.pose.hide_name.is_empty() {
        lang.t("panels-hide-set").to_owned()
    } else {
        app.view3d.pose.hide_name.clone()
    };
    let out = w::text_field(ui, field, "hide.drawer.name", &shown, None, false);
    if let Some(name) = out.committed {
        app.view3d.pose.hide_name = name;
    }
    let save = Rect::from_min_max(pos2(row.right() - SAVE_W, row.top()), row.max);
    let has_bones = !app.view3d.view.hide.bones.is_clear();
    if w::button(
        ui,
        save,
        "hide.drawer.save",
        lang.t("panels-save"),
        false,
        free && has_bones,
        None,
        Some("save"),
    )
    .clicked()
    {
        let name = app.view3d.pose.hide_name.clone();
        if bone_hide::save_preset(app, &name).is_some() {
            app.view3d.pose.hide_name.clear();
        }
    }
    let presets: Vec<(u32, String)> = app
        .view3d
        .pose
        .hide_presets
        .items()
        .iter()
        .map(|p| (p.id, p.name.clone()))
        .collect();
    let on: Vec<u32> = app.view3d.view.hide.bones.presets.clone();
    let (mut toggle, mut delete) = (None, None);
    for (id, name) in &presets {
        let row = rows.row(t::ROW_HEIGHT, 1.0);
        let button = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let label_rect = Rect::from_min_max(row.min, pos2(button.left() - 4.0, row.bottom()));
        let label = w::fit(ui.painter(), name, label_rect.width() - 24.0, t::LABEL);
        let is_on = on.contains(id);
        if w::toggle(
            ui,
            label_rect,
            ("hide.drawer.preset", *id),
            &label,
            is_on,
            None,
            free,
        ) != is_on
        {
            toggle = Some(*id);
        }
        if w::icon_button(
            ui,
            button,
            ("hide.drawer.delete", *id),
            "delete",
            lang.t("panels-delete-this-hide-set-tip"),
            false,
            free,
            13.0,
        )
        .clicked()
        {
            delete = Some(*id);
        }
    }
    if let Some(id) = toggle {
        bone_hide::toggle_preset(app, id);
    }
    if let Some(id) = delete {
        bone_hide::delete_preset(app, id);
    }
}
