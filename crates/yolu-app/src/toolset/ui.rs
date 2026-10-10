//! ツールの並びの画面: 左のツールの列（押して替える・右クリックのメニュー・ドラッグで並べ替え）と、ツールの列・グループのタブ・ブラシの行を
//! またぐドラッグ（落とす先は描いた所が覚え、フレームの終わりに 1 回だけ当てる）、名前の入力欄とアイコンの格子（ツールの列の右に浮かせる）。
//! グループのタブとブラシの行は `panels::brushes`。画面には名前だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Color32, Id, Order, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::{GroupId, SlotId, ToolsetAction, ICONS};
use crate::brushes::{BrushAction, BrushKey, DropAt};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::ui::menu::context_anchor;
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 名前を変えている物。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renaming {
    Slot(SlotId),
    Group(GroupId),
}

/// ドラッグしている物。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dragged {
    Slot(SlotId),
    Group(GroupId),
    Brush(BrushKey),
}

/// ドラッグの落とす先（このフレームでポインタの下にある所）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// ツールの列の、このツールの前（None なら最後）。
    StripGap(Option<SlotId>),
    /// ツールの列のこのツールの上（グループ・ブラシを、そのツールへ）。
    Slot(SlotId),
    /// グループのタブの、このタブの前（None なら最後）。
    TabGap(SlotId, Option<GroupId>),
    /// このタブの上（ブラシを、そのグループへ）。
    Tab(GroupId),
    /// ブラシの一覧の中の位置。
    Row(GroupId, DropAt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub what: Dragged,
    pub target: Option<Target>,
}

/// ツールの並びの画面だけの状態（保存しない）。
#[derive(Default)]
pub struct ToolsetUi {
    pub renaming: Option<Renaming>,
    pub rename_started: bool,
    /// アイコンの格子を開いているツール。
    pub icon_picker: Option<SlotId>,
    pub drag: Option<Drag>,
    /// 右クリックのメニューの対象（ツールの列の空いた所なら None）。
    pub context_slot: Option<SlotId>,
    pub context_group: Option<GroupId>,
    /// 前のフレームのツールの列のボタンの矩形（名前の欄・アイコンの格子を、その右に出す）。
    pub slot_rects: Vec<(SlotId, Rect)>,
    /// ツールの列が入りきらないときのスクロール。
    pub strip_scroll: f32,
}

impl ToolsetUi {
    /// ドラッグの最中に、ポインタの下の落とす先を覚える（描いた所が呼ぶ。最後に呼んだ所が勝つ）。
    pub fn hover(&mut self, target: Target) {
        if let Some(drag) = &mut self.drag {
            drag.target = Some(target);
        }
    }

    pub fn dragging(&self) -> Option<Dragged> {
        self.drag.map(|d| d.what)
    }

    pub fn target(&self) -> Option<Target> {
        self.drag.and_then(|d| d.target)
    }

    pub fn start_drag(&mut self, what: Dragged) {
        self.drag = Some(Drag { what, target: None });
    }
}

/// フレームの終わり: ボタンを離したら、覚えた落とす先へ当てる（Esc ならやめる）。押したままなら、次のフレームの描く所が落とす先を
/// 覚え直す。
pub fn end_frame(ctx: &egui::Context, app: &mut AppState) {
    let Some(drag) = app.toolset.ui.drag else {
        return;
    };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.toolset.ui.drag = None;
        crate::ui::window::note_escape_taken(ctx);
        return;
    }
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(d) = &mut app.toolset.ui.drag {
            d.target = None;
        }
        ctx.request_repaint();
        return;
    }
    app.toolset.ui.drag = None;
    let copy = ctx.input(|i| i.modifiers.command);
    if let Some(action) = drop_action(app, drag, copy) {
        app.apply(action);
    }
}

/// 落とした物と先から、当てる操作。
pub fn drop_action(app: &AppState, drag: Drag, copy: bool) -> Option<Action> {
    let set = &app.toolset.set;
    let target = drag.target?;
    Some(match (drag.what, target) {
        (Dragged::Slot(slot), Target::StripGap(before)) => {
            Action::Tools(ToolsetAction::Move { slot, before })
        }
        (Dragged::Group(group), Target::TabGap(slot, before)) => {
            Action::Tools(ToolsetAction::MoveGroup {
                group,
                to: slot,
                before,
            })
        }
        (Dragged::Group(group), Target::Slot(slot)) => {
            if set.slot_of_group(group) == Some(slot) || !set.slot(slot)?.holds_brushes() {
                return None;
            }
            Action::Tools(ToolsetAction::MoveGroup {
                group,
                to: slot,
                before: None,
            })
        }
        (Dragged::Brush(key), Target::Row(group, at)) => {
            if !copy && set.group_of(key) == Some(group) {
                Action::Brush(BrushAction::Move { key, at })
            } else {
                Action::Brush(BrushAction::Place {
                    key,
                    group,
                    at,
                    copy,
                })
            }
        }
        (Dragged::Brush(key), Target::Tab(group)) => {
            if !copy && set.group_of(key) == Some(group) {
                return None;
            }
            Action::Brush(BrushAction::Place {
                key,
                group,
                at: DropAt::End,
                copy,
            })
        }
        (Dragged::Brush(key), Target::Slot(slot)) => {
            let group = set.shown_group(slot)?;
            if !copy && set.group_of(key) == Some(group) {
                return None;
            }
            Action::Brush(BrushAction::Place {
                key,
                group,
                at: DropAt::End,
                copy,
            })
        }
        _ => return None,
    })
}

/// ツールの列のボタン（押して替える・ドラッグで動かす・右クリックでメニュー）。
fn slot_button(
    ui: &mut Ui,
    r: Rect,
    slot: SlotId,
    icon: (&str, &str),
    tooltip: &str,
    selected: bool,
    returns: bool,
) -> egui::Response {
    let id = ui.make_persistent_id(("tool.slot", slot));
    let response = ui.interact(r, id, Sense::click_and_drag());
    let hover = response.hovered();
    let p = ui.painter();
    if selected {
        w::rounded(p, r, t::ACCENT_DIM, 4.0);
    } else if response.is_pointer_button_down_on() {
        w::rounded(p, r, t::CONTROL_ACTIVE, 4.0);
    } else if hover {
        w::rounded(p, r, t::CONTROL_HOVER, 4.0);
    }
    w::icon(
        p,
        r,
        if selected { icon.1 } else { icon.0 },
        if selected || hover {
            Color32::WHITE
        } else {
            t::TEXT
        },
        22.0,
    );
    // キーを離すと戻るツール（押している間だけのツールのキーの間）に、小さな点
    if returns {
        let at = pos2(r.right() - 5.0, r.bottom() - 5.0);
        p.circle_filled(at, 4.0, t::PANEL_BG);
        p.circle_filled(at, 2.75, t::ACCENT);
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, tooltip));
    response.on_hover_text(tooltip)
}

/// ツールの列の 1 つのボタンに出す物。
struct StripRow {
    slot: SlotId,
    name: String,
    icon: super::IconChoice,
    gap: bool,
    selected: bool,
    /// キーを離すと戻るツール（押している間だけのツールのキーの間）。
    returns: bool,
    key: String,
}

/// ツールの帯のツールの列（帯の下の端の 2 枚の色の分は、呼ぶ側が先に取る）。`bottom` は列の下の端。
pub fn strip(ui: &mut Ui, app: &mut AppState, r: Rect, bottom: f32) {
    let lang = app.lang;
    let set = &app.toolset.set;
    let return_slot = app.temp_tool_return_slot();
    let rows: Vec<StripRow> = set
        .slots()
        .iter()
        .map(|s| StripRow {
            slot: s.id,
            name: s.name_in(lang),
            icon: s.icon(),
            gap: s.gap,
            selected: set.active() == Some(s.id),
            returns: return_slot == Some(s.id),
            // キーは、そのキーで替わるツール（そのツールの列の最初の 1 つ）にだけ添える
            key: if set.first_of(s.tool) == Some(s.id) {
                crate::shortcuts::tool_key(s.tool)
            } else {
                String::new()
            },
        })
        .collect();
    let n = rows.len().max(1) as f32;
    let separators = rows.iter().filter(|r| r.gap).count() as f32;
    let area = Rect::from_min_max(pos2(r.left(), r.top()), pos2(r.right(), bottom));
    // ツールが増えても、ウィンドウの最小の高さ（帯が一番低くなる所）で最後のボタンが切れないよう、足りなければ間隔を詰める（ボタンの間は 2 点）。
    // それでも入りきらなければ、ホイールで送る
    let step = ((area.height() - 6.0 - 4.0 - separators * 9.0) / n).clamp(24.0, 34.0);
    let content = 6.0 + 4.0 + separators * 9.0 + n * step;
    let max_scroll = (content - area.height()).max(0.0);
    if ui.rect_contains_pointer(area) && max_scroll > 0.0 {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.toolset.ui.strip_scroll -= wheel;
    }
    app.toolset.ui.strip_scroll = app.toolset.ui.strip_scroll.clamp(0.0, max_scroll);
    let scroll = app.toolset.ui.strip_scroll;
    let outer = ui.clip_rect();
    ui.set_clip_rect(area.intersect(outer));
    let p = ui.painter().clone();
    let dragging = app.toolset.ui.dragging();
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut y = r.top() + 6.0 - scroll;
    let mut rects: Vec<(SlotId, Rect)> = Vec::with_capacity(rows.len());
    let mut gap_marks: Vec<(Option<SlotId>, f32)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let (slot, name, key) = (&row.slot, &row.name, row.key.as_str());
        if row.gap {
            w::strip_separator(
                &p,
                Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), 9.0)),
            );
            y += 9.0;
        }
        let at = Rect::from_min_size(pos2(r.left() + 5.0, y), vec2(r.width() - 10.0, step - 2.0));
        gap_marks.push((Some(*slot), at.top() - 1.0));
        rects.push((*slot, at));
        let tip = if key.is_empty() {
            name.clone()
        } else {
            lang.pick(format!("{name}（{key}）"), format!("{name} ({key})"))
        };
        let response = slot_button(
            ui,
            at,
            *slot,
            (row.icon.normal, row.icon.selected),
            &tip,
            row.selected,
            row.returns,
        );
        if response.clicked() {
            app.apply(Action::Tools(ToolsetAction::Select(*slot)));
        }
        if response.drag_started() {
            app.toolset.ui.start_drag(Dragged::Slot(*slot));
        }
        if response.secondary_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                app.toolset.ui.context_slot = Some(*slot);
                crate::panels::properties::open_popup(
                    app,
                    ui.ctx(),
                    Popup::ToolStripContext,
                    context_anchor(at),
                    0.0,
                );
            }
        }
        // ドラッグ: ツールは列の間へ、グループ・ブラシはブラシを持つツールの上へ
        if let (Some(what), Some(pos)) = (dragging, pointer) {
            let band = Rect::from_min_max(
                pos2(r.left(), at.top() - 1.0),
                pos2(r.right(), at.bottom() + 1.0),
            );
            if band.contains(pos) {
                match what {
                    Dragged::Slot(_) => {
                        let before = if pos.y < at.center().y {
                            Some(*slot)
                        } else {
                            rows.get(i + 1).map(|r| r.slot)
                        };
                        app.toolset.ui.hover(Target::StripGap(before));
                    }
                    Dragged::Group(_) | Dragged::Brush(_) => {
                        if app
                            .toolset
                            .set
                            .slot(*slot)
                            .is_some_and(|s| s.holds_brushes())
                        {
                            app.toolset.ui.hover(Target::Slot(*slot));
                        }
                    }
                }
            }
        }
        y += step;
    }
    gap_marks.push((None, y - 1.0));
    // ツールを列の最後より下へ落とす
    if let (Some(Dragged::Slot(_)), Some(pos)) = (dragging, pointer) {
        if area.contains(pos) && pos.y >= y - 2.0 {
            app.toolset.ui.hover(Target::StripGap(None));
        }
    }
    // 空いた所の右クリック（ツールを追加・最初の並びに戻す）
    let blank = Rect::from_min_max(pos2(r.left(), y.max(r.top())), area.max);
    if blank.height() > 0.0 {
        let response = ui.interact(
            blank,
            ui.make_persistent_id("tool.strip.blank"),
            Sense::click(),
        );
        if response.secondary_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                app.toolset.ui.context_slot = None;
                crate::panels::properties::open_popup(
                    app,
                    ui.ctx(),
                    Popup::ToolStripContext,
                    context_anchor(at),
                    0.0,
                );
            }
        }
    }
    // 落とす先の印（ツールの間の線・ツールの枠）
    match app.toolset.ui.target() {
        Some(Target::StripGap(before)) => {
            if let Some((_, y)) = gap_marks.iter().find(|(s, _)| *s == before) {
                w::fill(
                    &p,
                    Rect::from_min_size(pos2(r.left() + 4.0, y - 1.0), vec2(r.width() - 8.0, 2.0)),
                    t::ACCENT,
                );
            }
        }
        Some(Target::Slot(slot)) => {
            if let Some((_, at)) = rects.iter().find(|(s, _)| *s == slot) {
                w::outline(&p, *at, t::ACCENT, 1.5, 4.0);
            }
        }
        _ => {}
    }
    ui.set_clip_rect(outer);
    app.toolset.ui.slot_rects = rects;
    rename_overlay(ui.ctx(), app);
    icon_picker(ui.ctx(), app);
}

/// ツールの名前の入力欄（ツールの列のボタンの右に浮かせる）。
fn rename_overlay(ctx: &egui::Context, app: &mut AppState) {
    let Some(Renaming::Slot(slot)) = app.toolset.ui.renaming else {
        return;
    };
    let Some(button) = app
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == slot)
        .map(|(_, r)| *r)
    else {
        app.toolset.ui.renaming = None;
        return;
    };
    let current = app
        .toolset
        .set
        .slot(slot)
        .map(|s| s.name_in(app.lang))
        .unwrap_or_default();
    let field = Rect::from_min_size(
        pos2(button.right() + 8.0, button.center().y - 12.0),
        vec2(180.0, 24.0),
    );
    egui::Area::new(Id::new(("tool.rename", slot)))
        .order(Order::Foreground)
        .fixed_pos(field.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(field.size(), Sense::hover());
            let first = !app.toolset.ui.rename_started;
            app.toolset.ui.rename_started = true;
            let out = w::text_field(
                ui,
                field,
                ("tool.rename.field", slot),
                &current,
                None,
                first,
            );
            if let Some(next) = out.committed {
                app.apply(Action::Tools(ToolsetAction::Rename(slot, next)));
            }
            if !first && !out.focused {
                app.toolset.ui.renaming = None;
            }
        });
}

/// 格子の 1 マスの大きさと列の数。
const ICON_CELL: f32 = 30.0;
const ICON_COLUMNS: usize = 7;

/// アイコンの格子（ツールの列のボタンの右に浮かせる。押すと替えて閉じる。外を押す・Esc で閉じる）。
fn icon_picker(ctx: &egui::Context, app: &mut AppState) {
    let Some(slot) = app.toolset.ui.icon_picker else {
        return;
    };
    let Some(button) = app
        .toolset
        .ui
        .slot_rects
        .iter()
        .find(|(s, _)| *s == slot)
        .map(|(_, r)| *r)
    else {
        app.toolset.ui.icon_picker = None;
        return;
    };
    let current = app.toolset.set.slot(slot).map(|s| s.icon().key);
    let rows = ICONS.len().div_ceil(ICON_COLUMNS);
    let size = vec2(
        ICON_COLUMNS as f32 * ICON_CELL + 12.0,
        rows as f32 * ICON_CELL + 12.0,
    );
    let screen = ctx.content_rect();
    let mut frame = Rect::from_min_size(pos2(button.right() + 8.0, button.top()), size);
    if frame.bottom() > screen.bottom() - 8.0 {
        frame = frame.translate(vec2(0.0, screen.bottom() - 8.0 - frame.bottom()));
    }
    let lang = app.lang;
    crate::ui::window::note_open(ctx);
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !frame.contains(p) && !button.contains(p))
    {
        close = true;
    }
    egui::Area::new(Id::new("tool.icons"))
        .order(Order::Foreground)
        .fixed_pos(frame.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(frame.size(), Sense::click());
            let p = ui.painter().clone();
            w::rounded(&p, frame, t::PANEL_BG, 6.0);
            w::outline(&p, frame, t::SEPARATOR, 1.0, 6.0);
            for (i, choice) in ICONS.iter().enumerate() {
                let (row, column) = (i / ICON_COLUMNS, i % ICON_COLUMNS);
                let cell = Rect::from_min_size(
                    pos2(
                        frame.left() + 6.0 + column as f32 * ICON_CELL,
                        frame.top() + 6.0 + row as f32 * ICON_CELL,
                    ),
                    vec2(ICON_CELL - 2.0, ICON_CELL - 2.0),
                );
                let on = current == Some(choice.key);
                let response = ui.interact(
                    cell,
                    ui.make_persistent_id(("tool.icon", i)),
                    Sense::click(),
                );
                if on {
                    w::rounded(&p, cell, t::ACCENT_DIM, 4.0);
                } else if response.hovered() {
                    w::rounded(&p, cell, t::CONTROL_HOVER, 4.0);
                }
                w::icon(
                    &p,
                    cell,
                    choice.normal,
                    if on { Color32::WHITE } else { t::TEXT },
                    18.0,
                );
                let label = lang.pick(format!("アイコン {}", i + 1), format!("Icon {}", i + 1));
                response.widget_info(|| {
                    WidgetInfo::selected(WidgetType::SelectableLabel, true, on, &label)
                });
                if response.clicked() {
                    app.apply(Action::Tools(ToolsetAction::SetIcon(slot, choice.key)));
                }
            }
        });
    if close {
        app.toolset.ui.icon_picker = None;
        crate::ui::window::note_escape_taken(ctx);
    }
}

/// ツールの列の右クリックのメニュー（ツールの上なら、そのツールの操作も）。
pub fn strip_menu(app: &AppState) -> Vec<crate::ui::menu::Entry<Action>> {
    use crate::state::Tool;
    use crate::ui::menu::Entry;
    let lang = app.lang;
    let set = &app.toolset.set;
    let free = !app.is_stroking() && set.locked.is_none();
    let slot = app.toolset.ui.context_slot.and_then(|id| set.slot(id));
    let after = slot.map(|s| s.id);
    let mut add = vec![
        Entry::item(
            lang.pick("ブラシ", "Brush"),
            Action::Tools(ToolsetAction::Add {
                tool: Tool::Brush,
                after,
            }),
        )
        .enabled(free),
        Entry::item(
            lang.pick("消しゴム", "Eraser"),
            Action::Tools(ToolsetAction::Add {
                tool: Tool::Eraser,
                after,
            }),
        )
        .enabled(free),
    ];
    let removed = set.removed_tools();
    if !removed.is_empty() {
        add.push(Entry::Separator);
        for tool in removed {
            add.push(
                Entry::item(
                    tool.name_in(lang),
                    Action::Tools(ToolsetAction::Add { tool, after }),
                )
                .enabled(free),
            );
        }
    }
    let mut entries =
        vec![Entry::submenu(lang.pick("ツールを追加", "Add Tool"), add).enabled(free)];
    if let Some(slot) = slot {
        let last_of_kind =
            slot.holds_brushes() && set.slots().iter().filter(|s| s.tool == slot.tool).count() <= 1;
        let mut remove = Entry::item(
            lang.pick("このツールを削除", "Delete This Tool"),
            Action::Tools(ToolsetAction::Remove(slot.id)),
        )
        .enabled(free && !last_of_kind);
        if last_of_kind {
            remove = remove.tooltip(super::Refusal::LastOfKind(slot.tool).describe(lang));
        }
        entries.extend([
            Entry::item(
                lang.pick("名前を変更", "Rename"),
                Action::Tools(ToolsetAction::StartRename(slot.id)),
            )
            .enabled(free),
            Entry::item(
                lang.pick("アイコンを変更", "Change Icon"),
                Action::Tools(ToolsetAction::PickIcon(Some(slot.id))),
            )
            .enabled(free),
            Entry::item(
                lang.pick("前に区切り", "Separator Before"),
                Action::Tools(ToolsetAction::ToggleGap(slot.id)),
            )
            .checked(slot.gap)
            .enabled(free),
            Entry::Separator,
            remove,
        ]);
    }
    entries.push(Entry::Separator);
    entries.push(
        Entry::item(
            lang.pick("最初の並びに戻す…", "Reset Tool Layout…"),
            Action::Tools(ToolsetAction::ResetDialog),
        )
        .enabled(free),
    );
    entries
}

/// グループのタブの右クリックのメニュー。
pub fn group_menu(app: &AppState) -> Vec<crate::ui::menu::Entry<Action>> {
    use crate::ui::menu::Entry;
    let lang = app.lang;
    let set = &app.toolset.set;
    let free = !app.is_stroking() && set.locked.is_none();
    let Some((slot, group)) = app.toolset.ui.context_group.and_then(|g| set.group(g)) else {
        return Vec::new();
    };
    vec![
        Entry::item(
            lang.pick("グループを追加", "Add Group"),
            Action::Tools(ToolsetAction::AddGroup(slot.id)),
        )
        .enabled(free),
        Entry::item(
            lang.pick("名前を変更", "Rename"),
            Action::Tools(ToolsetAction::StartRenameGroup(group.id)),
        )
        .enabled(free),
        Entry::item(
            lang.pick("複製", "Duplicate"),
            Action::Tools(ToolsetAction::DuplicateGroup(group.id)),
        )
        .enabled(free),
        Entry::Separator,
        Entry::item(
            lang.pick("削除", "Delete"),
            Action::Tools(ToolsetAction::RemoveGroup(group.id)),
        )
        .enabled(free),
    ]
}

/// ブラシ `key` を引いているとき、落とせるグループ（ブラシを今いるグループのほか、満杯でないグループ。並びが読めないときは無い）。
/// グループのタブが、落とせることを光って見せる。
pub fn droppable_groups(app: &AppState, key: BrushKey) -> Vec<GroupId> {
    let set = &app.toolset.set;
    if set.locked.is_some() {
        return Vec::new();
    }
    let current = set.group_of(key);
    set.slots()
        .iter()
        .flat_map(|s| s.groups.iter())
        .filter(|g| Some(g.id) != current && g.has_room())
        .map(|g| g.id)
        .collect()
}

/// ブラシの行の右クリックのメニュー（対象は `brushes.ui.context`）。先頭はそのブラシの設定（ブラシの詳細のウィンドウ）。
/// 押せない項目は、その理由（描いている間・並びが読めない・取り込み中・グループが満杯）をツールチップに出す。
pub fn brush_menu(app: &AppState) -> Vec<crate::ui::menu::Entry<Action>> {
    use crate::ui::menu::Entry;
    let lang = app.lang;
    let set = &app.toolset.set;
    let Some(key) = app.brushes.ui.context else {
        return Vec::new();
    };
    // 断る理由（描いている間 → 並びが読めない → 取り込み中の順。取り込み中は並び・数・名前を変える項目だけ）
    let stroking = app
        .is_stroking()
        .then(|| crate::lang::refusals::during_stroke(lang).to_owned());
    let locked = set.lock_refusal().map(|r| r.describe(lang));
    let importing = app.brushes.import.is_busy().then(|| {
        lang.pick("ブラシを取り込み中です。", "Importing brushes.")
            .to_owned()
    });
    let reason = |layout: bool, changes: bool| -> Option<String> {
        stroking
            .clone()
            .or_else(|| if layout { locked.clone() } else { None })
            .or_else(|| if changes { importing.clone() } else { None })
    };
    // 項目の押せる条件と、断る理由（理由があれば押せず、理由をツールチップに）
    let gate = |entry: Entry<Action>, ok: bool, why: Option<String>| match why {
        Some(why) => entry.enabled(false).tooltip(why),
        None => entry.enabled(ok),
    };
    let user = key.is_user();
    let placed = set.contains(key);
    let layout = set.locked.is_none() && placed;
    let modified = app.brush_is_modified(key);
    let mut entries = vec![
        gate(
            Entry::item(
                lang.pick("ブラシの設定…", "Brush Settings…"),
                Action::Brush(BrushAction::OpenDetail(key)),
            ),
            true,
            reason(false, false),
        ),
        // 組み込みも、名前を変える・登録するとその場でファイルの写しになる
        gate(
            Entry::item(
                lang.pick("名前を変更", "Rename"),
                Action::Brush(BrushAction::StartRename(key)),
            ),
            user || layout,
            reason(!user, !user),
        ),
        gate(
            Entry::item(
                lang.pick("複製", "Duplicate"),
                Action::Brush(BrushAction::Duplicate(key)),
            ),
            set.locked.is_none(),
            reason(true, true),
        ),
    ];
    if let Some(menu) = move_menu(app, key, reason(true, true)) {
        entries.push(menu);
    }
    entries.extend([
        Entry::Separator,
        gate(
            Entry::item(
                lang.pick("この設定で登録", "Register These Settings"),
                Action::Brush(BrushAction::Register(key)),
            ),
            (user || layout) && modified,
            reason(!user, true),
        ),
        gate(
            Entry::item(
                lang.pick("元に戻す", "Revert"),
                Action::Brush(BrushAction::Revert(key)),
            ),
            modified,
            reason(false, false),
        ),
        Entry::Separator,
        gate(
            Entry::item(
                lang.pick("削除", "Delete"),
                Action::Brush(BrushAction::Delete(key)),
            ),
            layout,
            reason(true, true),
        ),
    ]);
    entries
}

/// 「グループへ移す ▸」: ブラシのあるツールのグループの一覧（今のグループは印だけで押せない。満杯のグループは押せない）。ブラシを持つ別のツールが
/// あれば、そのツールごとの入れ子のメニューに、そのグループ。並びに無いブラシ（置き場が無い）なら None。
fn move_menu(
    app: &AppState,
    key: BrushKey,
    why_not: Option<String>,
) -> Option<crate::ui::menu::Entry<Action>> {
    use crate::ui::menu::Entry;
    let lang = app.lang;
    let set = &app.toolset.set;
    let (own, current) = (set.slot_of(key)?, set.group_of(key)?);
    let groups = |slot: &super::ToolSlot| -> Vec<Entry<Action>> {
        slot.groups
            .iter()
            .map(|g| {
                let here = g.id == current;
                let item = Entry::item(
                    g.name_in(lang),
                    Action::Brush(BrushAction::Place {
                        key,
                        group: g.id,
                        at: DropAt::End,
                        copy: false,
                    }),
                )
                .radio(here);
                if here {
                    item.enabled(false)
                } else if !g.has_room() {
                    item.enabled(false)
                        .tooltip(super::Refusal::TooManyBrushes.describe(lang))
                } else {
                    item
                }
            })
            .collect()
    };
    let mut entries = set.slot(own).map(groups).unwrap_or_default();
    for slot in set
        .slots()
        .iter()
        .filter(|s| s.holds_brushes() && s.id != own)
    {
        if entries
            .last()
            .is_some_and(|e| !matches!(e, Entry::Separator))
        {
            entries.push(Entry::Separator);
        }
        entries.push(Entry::submenu(slot.name_in(lang), groups(slot)));
    }
    let menu = Entry::submenu(lang.pick("グループへ移す", "Move to Group"), entries);
    Some(match why_not {
        Some(why) => menu.enabled(false).tooltip(why),
        None => menu,
    })
}
