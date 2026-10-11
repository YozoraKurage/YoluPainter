//! 選択範囲の下のボタンの帯（Photoshop のコンテキストタスクバー・CLIP STUDIO の選択範囲ランチャーと同じ）。選択範囲があるとき、その外接矩形の
//! すぐ下（画面の外へはみ出すなら上、どちらも入らなければ内側の下）に、アイコンだけのボタンを小さな帯にして浮かべる。ボタン: 選択を解除・
//! 反転・拡張・縮小・境界をぼかす（量を聞くウィンドウ）・塗りつぶし・消去・コピーして新しいレイヤー・マスクにする・選択範囲を覚える（名前を付けて残すウィンドウ）。文字のラベルは無く、名前とキーはツールチップ。
//! 表示の回転・拡大・パンに付いていく（外接矩形は画面の点へ写した 4 隅から求める）。描いている間・選択の形を作っている間・表示を動かして
//! いる間は隠す。左端の持ち手をドラッグするとずらせる（選択範囲を外すと初めの位置へ戻る）。「選択範囲」メニューで出さないこともできる。
//! 押した操作は `Action::Sel`（1 回の Undo）を通る。

use egui::{pos2, vec2, Color32, Id, Order, Pos2, Rect, Sense, Ui, Vec2};

use super::{saved::SavedOp, ModifyKind, SelAction, SelEdit, SelUiOp};
use crate::canvas::view::CanvasView;
use crate::engine::LayerKind;
use crate::lang::Lang;
use crate::pen::PressKind;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// ボタン 1 つの大きさと、ボタンの間隔・帯の余白・持ち手の幅・グループの区切りの幅。
const BUTTON: Vec2 = vec2(28.0, 26.0);
const GAP: f32 = 2.0;
const PAD: f32 = 4.0;
const GRIP: f32 = 12.0;
const SEPARATOR: f32 = 9.0;
/// 選択範囲の外接矩形と帯の間・表示域の縁と帯の間。
const BELOW: f32 = 10.0;
const MARGIN: f32 = 6.0;

/// 帯のボタン 1 つ。
struct Item {
    id: &'static str,
    icon: &'static str,
    /// 名前（とキー）。押せないときは理由を添える。
    tooltip: String,
    enabled: bool,
    action: Action,
}

/// 区切りでグループに分けたボタン（解除・反転・拡張・縮小・ぼかし｜塗りつぶし・消去｜コピー・マスク・覚える）。
fn groups(app: &AppState) -> [Vec<Item>; 3] {
    let lang = app.lang;
    let edit = |e: SelEdit| Action::Sel(SelAction::Edit(e));
    let ui = |op: SelUiOp| Action::Sel(SelAction::Ui(op));
    let free = app.can_edit();
    // 描けない・コピーできない・マスクを足せない理由（押せないボタンのツールチップに添える）
    let (paint_reason, copy_reason, mask_reason) = reasons(app);
    let named = |name: &str, reason: &Option<String>| match reason {
        Some(r) => {
            let (open, close) = lang.pick(("（", "）"), (" (", ")"));
            format!("{name}{open}{r}{close}")
        }
        None => name.to_owned(),
    };
    // 消去のキーは、今のモードで効くときだけ添える（編集のモードの Delete は選んだ物を消す）
    let erase_name = crate::shortcuts::named_with_keys(
        lang,
        lang.pick("選択範囲を消去", "Erase Selection"),
        &[crate::shortcuts::key_in("selection.erase", app.mode)],
    );
    [
        vec![
            Item {
                id: "deselect",
                icon: "deselect",
                tooltip: crate::shortcuts::named_with_keys(
                    lang,
                    lang.pick("選択を解除", "Deselect"),
                    &[
                        crate::shortcuts::key_in("selection.deselect", app.mode),
                        app.mode.paints().then(|| "Esc".to_owned()),
                    ],
                ),
                enabled: free,
                action: edit(SelEdit::Clear),
            },
            Item {
                id: "invert",
                icon: "invert_colors",
                tooltip: crate::shortcuts::named_with_keys(
                    lang,
                    lang.pick("選択範囲を反転", "Invert Selection"),
                    &[crate::shortcuts::key_in("selection.invert", app.mode)],
                ),
                enabled: free,
                action: edit(SelEdit::Invert),
            },
            Item {
                id: "grow",
                icon: "arrow_maximize",
                tooltip: lang.pick("選択範囲を拡張…", "Grow Selection…").into(),
                enabled: free,
                action: ui(SelUiOp::OpenAmount(ModifyKind::Grow)),
            },
            Item {
                id: "shrink",
                icon: "arrow_minimize",
                tooltip: lang.pick("選択範囲を縮小…", "Shrink Selection…").into(),
                enabled: free,
                action: ui(SelUiOp::OpenAmount(ModifyKind::Shrink)),
            },
            Item {
                id: "feather",
                icon: "blur_on",
                tooltip: lang.pick("境界をぼかす…", "Feather Selection…").into(),
                enabled: free,
                action: ui(SelUiOp::OpenAmount(ModifyKind::Feather)),
            },
        ],
        vec![
            Item {
                id: "fill",
                icon: "format_color_fill",
                tooltip: named(
                    lang.pick("描画色で塗りつぶす", "Fill with the Paint Color"),
                    &paint_reason,
                ),
                enabled: free && paint_reason.is_none(),
                action: edit(SelEdit::Fill),
            },
            Item {
                id: "erase",
                icon: "tools/eraser",
                tooltip: named(&erase_name, &paint_reason),
                enabled: free && paint_reason.is_none(),
                action: edit(SelEdit::Erase),
            },
        ],
        vec![
            Item {
                id: "copy",
                icon: "copy_add",
                tooltip: named(
                    &crate::shortcuts::named_with_keys(
                        lang,
                        lang.pick("コピーして新しいレイヤーに", "Copy to a New Layer"),
                        &[crate::shortcuts::key_in("selection.to_new_layer", app.mode)],
                    ),
                    &copy_reason,
                ),
                enabled: free && copy_reason.is_none(),
                action: edit(SelEdit::ToNewLayer),
            },
            Item {
                id: "mask",
                icon: "vignette",
                tooltip: named(
                    lang.pick(
                        "選択範囲をレイヤーマスクにする",
                        "Make the Selection a Layer Mask",
                    ),
                    &mask_reason,
                ),
                enabled: free && mask_reason.is_none(),
                action: edit(SelEdit::ToMask),
            },
            Item {
                id: "remember",
                icon: "save",
                tooltip: lang.pick("選択範囲を覚える…", "Remember Selection…").into(),
                enabled: free,
                action: Action::Sel(SelAction::Saved(SavedOp::OpenWindow)),
            },
        ],
    ]
}

/// 塗る・コピー・マスクにするが、今のレイヤーでできない理由（できるなら None）。
fn reasons(app: &AppState) -> (Option<String>, Option<String>, Option<String>) {
    let lang = app.lang;
    let Some(layer) = app.selected_layer.and_then(|id| app.doc.layer(id)) else {
        let none = lang
            .pick("レイヤーが選ばれていません", "No layer is selected")
            .to_owned();
        return (Some(none.clone()), Some(none.clone()), Some(none));
    };
    let paint = app.paint_blocker();
    let masked = app.m2.edit_mask && layer.mask().is_some();
    let copy =
        (!masked && !matches!(layer.kind(), LayerKind::Raster | LayerKind::Fill)).then(|| {
            lang.pick("画素を持たないレイヤーです", "This layer has no pixels")
                .to_owned()
        });
    (paint, copy, None)
}

/// 帯を出すか。描いている間・選択の形を作っている間・表示を動かしている間・量を聞くウィンドウが開いている間・ペンでキャンバスを押している間は隠す。
pub fn visible(app: &AppState) -> bool {
    app.prefs.settings.selection_bar
        && app.doc.selection().is_some()
        && app.sel.dialog.is_none()
        && app.read_only_reason().is_none()
        && !app.is_stroking()
        && app.sel.drag.is_none()
        && app.sel.polygon.is_empty()
        && app.region.drag.is_none()
        && app.canvas.rotating.is_none()
        && !app.canvas.panning
        && app.canvas.zooming.is_none()
        // ペンが帯のボタンを押しているとき（押しの行き先が「何もしない」）は、ボタンを押し終えるまで出しておく
        && !app
            .canvas
            .pen_press
            .is_some_and(|p| p.kind != PressKind::Ignored)
}

/// 帯の大きさ。
fn bar_size(groups: &[Vec<Item>; 3]) -> Vec2 {
    let buttons: usize = groups.iter().map(Vec::len).sum();
    let width = PAD
        + GRIP
        + buttons as f32 * (BUTTON.x + GAP)
        + (groups.len() - 1) as f32 * SEPARATOR
        + PAD
        - GAP;
    vec2(width, BUTTON.y + 2.0 * PAD)
}

/// 選択範囲の外接矩形（画面の点。表示の回転・反転は 4 隅を写して囲む）。
fn screen_bounds(app: &mut AppState, view: &CanvasView) -> Option<Rect> {
    let mask = app.doc.selection().cloned()?;
    let (x0, y0, x1, y1) = app.sel.bounds_of(&mask)?;
    let corners =
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| view.to_screen(x as f64, y as f64));
    Some(Rect::from_points(&corners))
}

/// 帯の初めの位置（左上）: 外接矩形の下に中央で。入らなければ上、どちらも入らなければ表示域の下の内側。表示域の中へ収める。
fn default_position(bounds: Rect, size: Vec2, area: Rect) -> Pos2 {
    let x = bounds.center().x - size.x / 2.0;
    let below = bounds.bottom() + BELOW;
    let above = bounds.top() - BELOW - size.y;
    let y = if below + size.y <= area.bottom() - MARGIN {
        below
    } else if above >= area.top() + MARGIN {
        above
    } else {
        area.bottom() - MARGIN - size.y
    };
    clamp_into(pos2(x, y), size, area)
}

/// 帯を表示域の中へ収める（表示域より帯が大きければ左上に合わせる）。
fn clamp_into(p: Pos2, size: Vec2, area: Rect) -> Pos2 {
    let max_x = (area.right() - MARGIN - size.x).max(area.left() + MARGIN);
    let max_y = (area.bottom() - MARGIN - size.y).max(area.top() + MARGIN);
    pos2(
        p.x.clamp(area.left() + MARGIN, max_x),
        p.y.clamp(area.top() + MARGIN, max_y),
    )
}

/// 帯を描いて、押されたボタンの操作を当てる（`area` は表示域）。
pub fn show(ui: &mut Ui, app: &mut AppState, view: &CanvasView, area: Rect) {
    if app.doc.selection().is_none() {
        app.sel.bar_offset = Vec2::ZERO;
        return;
    }
    if !visible(app) {
        return;
    }
    let Some(bounds) = screen_bounds(app, view) else {
        return;
    };
    // 選択範囲が表示域の外へ出ていれば出さない（縁に張り付いた帯が何の帯か分からなくなる）
    if !bounds.intersects(area) {
        return;
    }
    let groups = groups(app);
    let size = bar_size(&groups);
    let home = default_position(bounds, size, area);
    let at = clamp_into(home + app.sel.bar_offset, size, area);
    app.sel.bar_offset = at - home;
    let lang = app.lang;
    let mut clicked: Option<Action> = None;
    let mut dragged = Vec2::ZERO;
    let bar = Rect::from_min_size(at, size);
    egui::Area::new(Id::new("yolu.selection-bar"))
        .order(Order::Middle)
        .fixed_pos(at)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(size, Sense::hover());
            paint_frame(ui, bar);
            // 持ち手（ドラッグでずらす）
            let grip = Rect::from_min_size(
                pos2(bar.left() + PAD, bar.top() + PAD),
                vec2(GRIP, BUTTON.y),
            );
            let response = ui.interact(grip, Id::new("yolu.selection-bar.grip"), Sense::drag());
            w::icon(
                ui.painter(),
                grip,
                "grid_dots",
                if response.hovered() || response.dragged() {
                    Color32::WHITE
                } else {
                    t::TEXT_DIM
                },
                14.0,
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                });
            }
            if response.dragged() {
                dragged = response.drag_delta();
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, grip_name(lang))
            });
            let _ = response.on_hover_text(grip_name(lang));
            // ボタン
            let mut x = grip.right() + GAP;
            for (g, items) in groups.iter().enumerate() {
                if g > 0 {
                    let sx = x - GAP + SEPARATOR / 2.0;
                    w::vline(
                        ui.painter(),
                        sx.round(),
                        bar.top() + PAD + 3.0,
                        bar.bottom() - PAD - 3.0,
                        t::SEPARATOR,
                    );
                    x += SEPARATOR - GAP;
                }
                for item in items {
                    let r = Rect::from_min_size(pos2(x, bar.top() + PAD), BUTTON);
                    let response = w::icon_button(
                        ui,
                        r,
                        ("yolu.selection-bar", item.id),
                        item.icon,
                        &item.tooltip,
                        false,
                        item.enabled,
                        16.0,
                    );
                    if item.enabled && response.clicked() {
                        clicked = Some(item.action.clone());
                    }
                    x += BUTTON.x + GAP;
                }
            }
        });
    app.sel.bar_offset += dragged;
    if let Some(action) = clicked {
        app.apply(action);
    }
}

/// 持ち手の名前（ツールチップと読み上げ）。
pub fn grip_name(lang: Lang) -> &'static str {
    lang.pick("ボタンの帯を動かす", "Move the Button Bar")
}

/// 帯の地・縁・影。
fn paint_frame(ui: &Ui, bar: Rect) {
    let p = ui.painter();
    for (grow, alpha) in [(4.0, 14), (2.5, 22), (1.0, 34)] {
        w::rounded(
            p,
            bar.expand(grow).translate(vec2(0.0, 2.0)),
            Color32::from_black_alpha(alpha),
            9.0,
        );
    }
    w::rounded(p, bar, t::PANEL_HEADER, 7.0);
    w::outline(p, bar, t::BORDER, 1.0, 7.0);
}
