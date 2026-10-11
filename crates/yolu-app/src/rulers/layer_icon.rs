//! レイヤーの一覧の定規のアイコン: 定規を持つレイヤー（グループも）の行の、サムネイル（とマスク）のすぐ右に出る。
//! - 押す: その行を選び、プロパティの「定規」の区分を開く。Shift＋押す: そのレイヤーの定規の表示を全部入り切り（1 つでも出ていれば全部隠し、全部隠れていれば全部出す）。
//! - 持って別の行へ落とす: そのレイヤー（グループ）へ定規を全部移す（1 回の取り消し）。
//! - 右クリック: 表示・表示の範囲・定規を削除。
//!
//! 全部の定規が隠れているレイヤーのアイコンは薄く描く。

use egui::{pos2, vec2, Color32, Pos2, Rect, Sense, Ui, WidgetInfo, WidgetType};
use yolu_core::{Layer, LayerId, RulerScope};

use super::{IconDrag, RulerAction};
use crate::lang::Lang;
use crate::m2::Row;
use crate::panels::effect_rows;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// アイコンの当たり（幅 × 高さ）。
pub const SIZE: (f32, f32) = (20.0, 22.0);
/// 行に置いたときに右へ進める幅。
pub const ADVANCE: f32 = 22.0;

/// アイコンの名前（ツールチップ・読み上げ）。
pub fn name(lang: Lang) -> &'static str {
    lang.pick("定規", "Ruler")
}

/// 定規を持つか・1 つでも表示しているか。定規が無ければ None。
pub fn look(layer: &Layer) -> Option<bool> {
    (!layer.rulers().is_empty()).then(|| layer.rulers().iter().any(|r| r.visible))
}

/// アイコンの当たり（行の中の左端 `x`）。
pub fn rect(row: Rect, x: f32) -> Rect {
    Rect::from_min_size(pos2(x, row.top() + 4.0), vec2(SIZE.0, SIZE.1))
}

/// アイコンの上の操作。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    None,
    /// 押した（修飾なし）: 行を選び、「定規」の区分を開く。
    Select,
    /// Shift＋押した: 表示を全部入り切り。
    ToggleAll,
    /// 右クリック（メニューを開く場所）。
    Menu(Pos2),
}

/// アイコンを描き、操作を受ける。`any_visible` は 1 つでも表示している定規があるか。持って動かし始めたときは、ここで `icon_drag` を立てる
/// （落とす先は `follow` が追う）。
pub fn show(
    ui: &mut Ui,
    app: &mut AppState,
    id: LayerId,
    at: Rect,
    any_visible: bool,
    enabled: bool,
) -> Event {
    let lang = app.lang;
    let response = ui.interact(
        at,
        ui.make_persistent_id(("layer.rulers", id.0)),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let dragging = app.rulers.icon_drag.is_some_and(|d| d.from == id);
    let color = if dragging || !any_visible {
        t::TEXT_DISABLED
    } else if response.hovered() {
        Color32::WHITE
    } else {
        t::TEXT_DIM
    };
    w::icon(ui.painter(), at, "tools/ruler", color, 16.0);
    let tip = name(lang);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, tip));
    if response.drag_started() && enabled {
        app.rulers.icon_drag = Some(IconDrag {
            from: id,
            over: None,
            can_drop: false,
        });
    }
    let shift = ui.input(|i| i.modifiers.shift);
    let event = if response.clicked() {
        if shift {
            Event::ToggleAll
        } else {
            Event::Select
        }
    } else if response.secondary_clicked() {
        Event::Menu(
            response
                .interact_pointer_pos()
                .unwrap_or_else(|| at.center()),
        )
    } else {
        Event::None
    };
    let _ = response.on_hover_text(tip);
    event
}

/// 持っているアイコンを追う: ポインタの下の行を落とす先にして枠を光らせ、アイコンをポインタに付ける。ボタンを離したら落とす
/// （同じ行・行の外は何もしない。上限を超える・読むだけのセットは、枠を光らせず、離すと移す操作が理由を知らせて断る）。見えない行（一覧を送って
/// 隠れた）でも離したら手放す。Esc・フォーカスの喪失では落とさずにやめる。一覧を描いたあとに呼ぶ。
pub fn follow(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &effect_rows::Layout,
    scroll: f32,
) {
    let Some(drag) = app.rulers.icon_drag else {
        return;
    };
    // Esc とウィンドウのフォーカスの喪失は、落とさずにやめる（定規のつまみのドラッグと同じ）。Esc は使い切って、キャンバスの Esc（選択の解除）に渡さない
    let escaped = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let focus_lost = ui.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(false)))
    });
    if escaped || focus_lost {
        app.rulers.icon_drag = None;
        return;
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        app.rulers.icon_drag = None;
        if let Some(to) = drag.over.filter(|to| *to != drag.from) {
            app.apply(Action::Ruler(RulerAction::MoveAll {
                from: drag.from,
                to,
            }));
        }
        return;
    }
    let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let over = list
        .contains(pointer)
        .then(|| layout.row_at(pointer.y - list.top() + scroll))
        .flatten()
        .and_then(|i| rows.get(i))
        .map(|r| r.id)
        .filter(|id| *id != drag.from);
    let can_drop = over.is_some_and(|to| can_drop(app, drag.from, to));
    app.rulers.icon_drag = Some(IconDrag {
        from: drag.from,
        over,
        can_drop,
    });
    // 落とせる行の枠（上限を超える・読むだけのセットの行は光らせない。そこで離すと、移す操作が理由を知らせて断る）
    if let Some(to) = over.filter(|_| can_drop) {
        if let Some(i) = rows.iter().position(|r| r.id == to) {
            let y = list.top() + layout.layer_y(i) - scroll;
            let painter = ui.painter_at(list);
            w::outline(
                &painter,
                Rect::from_min_size(
                    pos2(list.left() + 1.0, y + 1.0),
                    vec2(list.width() - 2.0, crate::panels::layers::ROW_HEIGHT - 2.0),
                ),
                t::ACCENT,
                2.0,
                3.0,
            );
        }
    }
    // ポインタに付くアイコン
    let ghost = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("rulers.icon-ghost"),
    ));
    w::icon(
        &ghost,
        Rect::from_center_size(pointer, vec2(SIZE.0, SIZE.1)),
        "tools/ruler",
        t::ACCENT,
        16.0,
    );
}

/// `from` の定規を全部 `to` へ移せるか（読むだけでなく、移し先が上限を超えない）。
fn can_drop(app: &AppState, from: LayerId, to: LayerId) -> bool {
    if !app.can_edit() {
        return false;
    }
    let count = |id: LayerId| app.doc.layer(id).map_or(0, |l| l.rulers().len());
    count(from) + count(to) <= yolu_core::MAX_RULERS_PER_LAYER
}

/// アイコンの右クリックのメニュー（表示・表示の範囲・定規を削除）。
pub fn menu_entries(app: &AppState, id: LayerId) -> Vec<Entry<Action>> {
    let l = app.lang;
    let Some(layer) = app.doc.layer(id) else {
        return Vec::new();
    };
    let rulers = layer.rulers();
    if rulers.is_empty() {
        return Vec::new();
    }
    let free = app.can_edit();
    let any_visible = rulers.iter().any(|r| r.visible);
    let scope_item = |label: &'static str, scope: RulerScope| {
        let on = rulers.iter().all(|r| r.visible && r.scope == scope);
        Entry::item(
            label,
            Action::Ruler(RulerAction::SetAllScope { owner: id, scope }),
        )
        .radio(on)
        .enabled(free)
    };
    vec![
        Entry::item(
            l.pick("表示", "Show"),
            Action::Ruler(RulerAction::SetAllVisible {
                owner: id,
                visible: !any_visible,
            }),
        )
        .checked(any_visible)
        .enabled(free),
        Entry::submenu(
            l.pick("表示の範囲", "Visible In"),
            vec![
                scope_item(l.pick("すべてのレイヤー", "All Layers"), RulerScope::All),
                scope_item(
                    l.pick("同じグループの中", "Within the Same Group"),
                    RulerScope::Group,
                ),
                scope_item(
                    l.pick("選んでいるときだけ", "Only While Selected"),
                    RulerScope::Selected,
                ),
            ],
        )
        .enabled(free),
        Entry::Separator,
        Entry::item(
            l.pick("定規を削除", "Delete Rulers"),
            Action::Ruler(RulerAction::Delete {
                owner: id,
                ids: rulers.iter().map(|r| r.id).collect(),
            }),
        )
        .enabled(free),
    ]
}
