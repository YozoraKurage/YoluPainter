//! パスのツールの欄: オプションバー（点の太さ・閉じる/開く・点を消す・ラスタライズ）と、左のドックのツールプロパティ（パスの節: 状態・点の操作・
//! 点の太さ・ブラシを使う・描き直す・ラスタライズ、ブラシの節: パスのブラシの値）。値の操作は
//! `Action::Path`（キー・試験と同じ道）。パスのブラシや点の太さのスライダーは、離したとき 1 回で描き直す（動かしている間は値だけ）。
//! 画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::properties::{choice_row, open_popup, section, toggle_row};
use crate::lang::Lang;
use crate::m2_menu::Popup;
use crate::pathtool::edit::{self, point_width};
use crate::pathtool::list::{display_name, ListOp, Toward};
use crate::pathtool::{path_brush, BrushEdit, PathAction, StyleEdit};
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};
use std::sync::Arc;
use yolu_core::geometry::world_radius;
use yolu_core::paths::{PathBrush, PathKind, Ribbon, RibbonMode};
use yolu_core::{ImageId, LayerPath};

/// スライダーの途中の値のキー。
const WIDTH: &str = "path.width";
const DIAMETER: &str = "path.diameter";
const HARDNESS: &str = "path.hardness";
const SPACING: &str = "path.spacing";
const OPACITY: &str = "path.opacity";
const FLOW: &str = "path.flow";
const RIBBON_SPACING: &str = "path.ribbon-spacing";
const TIP_ANGLE: &str = "path.tip-angle";
const DEPTH: &str = "path.depth";
const SMUDGE_STRENGTH: &str = "path.smudge-strength";

fn is_closed(path: &LayerPath) -> bool {
    edit::path_is_closed(path)
}

/// 動かしている間の値があればそれ、無ければ今の値。
fn shown(app: &AppState, key: &'static str, value: f32) -> f32 {
    app.path
        .pending
        .filter(|(k, _)| *k == key)
        .map_or(value, |(_, v)| v)
}

/// スライダー 1 つ。パスがあるとき（`deferred`）は離したとき、無いとき（次に作るパスのブラシ）はすぐ、新しい値を返す。
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    app: &mut AppState,
    at: Rect,
    key: &'static str,
    spec: SliderSpec,
    value: f32,
    deferred: bool,
) -> Option<f32> {
    let current = if deferred {
        shown(app, key, value)
    } else {
        value
    };
    let out = w::slider(ui, at, key, current, &spec);
    if !deferred {
        return out.changed.then_some(out.value);
    }
    if out.changed {
        app.path.pending = Some((key, out.value));
    }
    if out.released {
        return app
            .path
            .pending
            .take()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v);
    }
    if !out.active && !out.changed && app.path.pending.is_some_and(|(k, _)| k == key) {
        app.path.pending = None;
    }
    None
}

/// 点の太さのスライダーの当て方（選んでいる点の全部の太さ 0〜1。1 回の Undo）。
fn apply_width(app: &mut AppState, percent: f32) {
    let pressure = (percent / 100.0).clamp(0.0, 1.0) as f64;
    match app.path_selected_indices().as_slice() {
        [] => {}
        [index] => app.apply(Action::Path(PathAction::Point(edit::PointOp::Width {
            index: *index,
            pressure,
        }))),
        _ => app.apply(Action::Path(PathAction::Widths(pressure))),
    }
}

fn width_spec(lang: Lang, enabled: bool) -> SliderSpec<'static> {
    SliderSpec::new(lang.pick("太さ", "Width"), 0.0, 100.0, NumberFormat::int("%"))
        .tooltip(lang.pick(
            "選んでいる点の太さ（筆圧の代わり）。点ごとに変えると、パスの太さが滑らかに変わります。ブラシの「太さで直径・不透明度・流量を変える」が効いているとき、描く大きさに効きます",
            "Width at the selected point (stands in for pen pressure). Different widths along the path change its thickness smoothly, as far as the brush options for width are on",
        ))
        .enabled(enabled)
}

// ───────── オプションバー ─────────

/// オプションバーの中身（ツールのアイコンの右から）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let lang = app.lang;
    let free = app.can_edit();
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let mut x = x;
    let mut next = |width: f32| {
        let at = Rect::from_min_size(pos2(x + 4.0, y), vec2(width, h));
        x += width + 8.0;
        at
    };
    let selected = app.path_selected_index();
    let width_value = app
        .path_layer()
        .and_then(|(_, p)| selected.and_then(|i| point_width(p, i)))
        .map_or(100.0, |v| (v * 100.0) as f32);
    let at = next(150.0);
    let spec = width_spec(lang, free && selected.is_some());
    if let Some(v) = slider(ui, app, at, WIDTH, spec, width_value, true) {
        apply_width(app, v);
    }
    // 選んでいる点の接線: 角・取っ手（押すと切り替え）
    let tangent = app
        .path_layer()
        .and_then(|(_, p)| selected.and_then(|i| edit::point_tangent(p, i)));
    for (key, label, tip, on, action) in tangent_buttons(lang, tangent) {
        let bw = w::text_width(ui.painter(), label, t::LABEL) + 24.0;
        let at = next(bw);
        if w::button(
            ui,
            at,
            key,
            label,
            on,
            free && tangent.is_some(),
            Some(tip),
            None,
        )
        .clicked()
        {
            app.apply(action);
        }
    }
    let (has_path, closed, count) = match app.path_layer() {
        Some((_, p)) => (true, is_closed(p), p.point_count()),
        None => (false, false, 0),
    };
    // 塗りつぶしレイヤーのパスは画素にできない（パスの欄のボタンと同じ条件）
    let rasterizable = app
        .path_layer()
        .is_some_and(|(id, _)| app.path_can_rasterize(id));
    let (label, tip) = close_label_and_tip(lang, closed);
    let bw = w::text_width(ui.painter(), label, t::LABEL) + 24.0;
    let at = next(bw);
    if w::button(
        ui,
        at,
        "path.close",
        label,
        false,
        free && has_path && (closed || count >= 3),
        Some(tip),
        None,
    )
    .clicked()
    {
        app.apply(Action::Path(PathAction::Point(if closed {
            edit::PointOp::Open
        } else {
            edit::PointOp::Close
        })));
    }
    let label = delete_label(lang);
    let bw = w::text_width(ui.painter(), label, t::LABEL) + 24.0;
    let at = next(bw);
    if w::button(
        ui,
        at,
        "path.delete",
        label,
        false,
        free && count > 0,
        Some(&delete_tip(lang)),
        None,
    )
    .clicked()
    {
        app.apply(Action::Path(PathAction::DeleteSelected));
    }
    let label = lang.pick("ラスタライズ", "Rasterize");
    let bw = w::text_width(ui.painter(), label, t::LABEL) + 24.0;
    let at = next(bw);
    if w::button(
        ui,
        at,
        "path.rasterize",
        label,
        false,
        free && has_path && rasterizable,
        Some(rasterize_tip(lang)),
        None,
    )
    .clicked()
    {
        if let Some((id, _)) = app.path_layer() {
            app.apply(Action::Path(PathAction::Rasterize(id)));
        }
    }
}

/// 接線のボタン（キー・名前・ツールチップ・押されているか・押したときの操作）: 角と取っ手。
fn tangent_buttons(
    lang: Lang,
    tangent: Option<edit::TangentValue>,
) -> [(&'static str, &'static str, &'static str, bool, Action); 2] {
    let corner = tangent == Some(edit::TangentValue::Corner);
    let handles = matches!(tangent, Some(edit::TangentValue::Handles { .. }));
    [
        (
            "path.corner",
            lang.pick("角", "Corner"),
            corner_tip(lang),
            corner,
            Action::Path(PathAction::SetCorner(!corner)),
        ),
        (
            "path.handles",
            lang.pick("取っ手", "Handles"),
            handles_tip(lang),
            handles,
            Action::Path(PathAction::SetHandles(!handles)),
        ),
    ]
}

fn corner_tip(lang: Lang) -> &'static str {
    lang.pick(
        "選んでいる点で曲線を折ります（点をダブルクリックでも切り替え）",
        "Break the curve at the selected point (double-click the point to toggle)",
    )
}

fn handles_tip(lang: Lang) -> &'static str {
    lang.pick(
        "選んでいる点の曲がりを取っ手で決めます。取っ手をドラッグ（始めたあとに Alt を押すと片側だけ、Ctrl で両側を同じ割合で伸ばす）",
        "Shape the curve at the selected point with handles. Drag a handle (Alt pressed after you start: one side only; Ctrl: scale both)",
    )
}

/// 閉じる/開くの名前とツールチップ（今閉じているかで替わる）。
fn close_label_and_tip(lang: Lang, closed: bool) -> (&'static str, &'static str) {
    if closed {
        (
            lang.pick("開く", "Open"),
            lang.pick(
                "終わりの点（始めの点と同じ場所）を外して、輪を開きます",
                "Remove the end point (at the start point) to open the loop",
            ),
        )
    } else {
        (
            lang.pick("閉じる", "Close"),
            lang.pick(
                "終わりに始めの点を追加して、輪にします（3 点以上）",
                "Add the start point at the end to make a loop (3 points or more)",
            ),
        )
    }
}

fn delete_label(lang: Lang) -> &'static str {
    lang.pick("点を消す", "Delete Point")
}

fn delete_tip(lang: Lang) -> String {
    crate::shortcuts::named_with_keys(
        lang,
        lang.pick(
            "選んでいる点（無ければ最後の点）を消します",
            "Delete the selected point (the last one if none is selected)",
        ),
        &[crate::shortcuts::key_in(
            "path.delete_point",
            crate::mode::EditorMode::Paint,
        )],
    )
}

/// 3D のパスの「描き直す」: 描くのは休みの形（`path_surface_ctx` の `render`）で、今のポーズは使わない。
fn redraw_tip(lang: Lang) -> &'static str {
    lang.pick(
        "ポーズを付けない形（休みの形）のモデルの面に描き直します",
        "Redraw on the model's surface in its rest shape (the pose is not applied)",
    )
}

fn rasterize_tip(lang: Lang) -> &'static str {
    lang.pick(
        "今の画素を残してパスを外します。そのあとは普通に塗れます",
        "Keep the pixels and remove the path, so the layer can be painted on",
    )
}

// ───────── プロパティの欄 ─────────

/// 並べるボタン 1 つ。
struct Btn<'a> {
    id: &'static str,
    label: &'static str,
    tip: &'a str,
    enabled: bool,
    action: Action,
    /// 押されている（切り替えのボタンが入っている）か。
    on: bool,
}

/// ボタンを欄の幅に並べる。全部が 1 行に収まらないとき（狭い欄の日本語など）は、収まる最大の列数で折り返す（「…」で詰めない）。
fn button_grid(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, buttons: Vec<Btn<'_>>) {
    const GAP: f32 = 4.0;
    let needed = buttons
        .iter()
        .map(|b| w::text_width(ui.painter(), b.label, t::LABEL))
        .fold(0.0, f32::max)
        + 14.0;
    let width = rows.width();
    let cols = (1..=buttons.len())
        .rev()
        .find(|c| (width - GAP * (*c as f32 - 1.0)) / *c as f32 >= needed)
        .unwrap_or(1);
    for line in buttons.chunks(cols) {
        let row = rows.row(24.0, GAP);
        // 最後の行が短くても、ほかの行と同じ幅のボタン
        let cells = Rows::split(row, cols, GAP);
        for (cell, b) in cells.iter().zip(line) {
            if w::button(ui, *cell, b.id, b.label, b.on, b.enabled, Some(b.tip), None).clicked() {
                app.apply(b.action.clone());
            }
        }
    }
}

/// ツールプロパティ（パスのツール）: パスの節とパスのブラシの節。塗るチャンネルの組はプロパティの欄のマテリアル。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    path_section(ui, app, rows);
    brush_section(ui, app, rows);
}

// ───────── 塗りつぶしレイヤーのパス ─────────

/// 塗りつぶしレイヤーの欄のパスの行（`fill_props` が呼ぶ）: パスの一覧（あれば）と、パスのツールでこのレイヤーにパスを追加するボタン。
pub fn fill_layer_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    layer: yolu_core::LayerId,
) {
    let lang = app.lang;
    if app.selected_layer != Some(layer) {
        return;
    }
    let (open, _) = section(
        ui,
        app,
        rows,
        "fill-paths",
        lang.pick("パス", "Paths"),
        "conversion_path",
        None,
    );
    if !open {
        return;
    }
    if !app.path_entries().is_empty() {
        list_rows(ui, app, rows);
    }
    let r = rows.row(24.0, 4.0);
    if w::button(
        ui,
        r,
        "fill.add-path",
        lang.pick("パスを追加", "Add Path"),
        false,
        app.can_edit(),
        Some(lang.pick(
            "パスツールにして、この塗りつぶしレイヤーに新しいパスを始めます（パスは塗りつぶしと効果の上に重なります）",
            "Switch to the Path tool and start a new path on this fill layer (paths lie over the fill and its effects)",
        )),
        Some("add"),
    )
    .clicked()
    {
        app.apply(Action::SelectTool(crate::state::Tool::Path));
        app.apply(Action::Path(PathAction::SelectPath(None)));
        app.path.fresh = Some(layer);
    }
    rows.space(4.0);
}

// ───────── パスの一覧 ─────────

fn list_action(op: ListOp) -> Action {
    Action::Path(PathAction::List(op))
}

/// 一覧の行の右クリックのメニュー（パスの ID）。
pub fn context_entries(app: &AppState, id: u128) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = app.can_edit();
    let entries = app.path_entries();
    let Some(index) = entries.iter().position(|e| e.id() == id) else {
        return Vec::new();
    };
    let e = &entries[index];
    let pasteable = app.path.clipboard.is_some();
    vec![
        Entry::item(
            lang.pick("名前を変更", "Rename"),
            Action::Path(PathAction::BeginRename(id)),
        )
        .enabled(free),
        Entry::item(
            lang.pick("表示", "Visible"),
            list_action(ListOp::Visible(id, !e.visible)),
        )
        .checked(e.visible)
        .enabled(free),
        Entry::Separator,
        Entry::item(lang.pick("コピー", "Copy"), list_action(ListOp::Copy(id))),
        Entry::item(lang.pick("貼り付け", "Paste"), list_action(ListOp::Paste))
            .enabled(free && pasteable),
        Entry::item(
            lang.pick("複製", "Duplicate"),
            list_action(ListOp::Duplicate(id)),
        )
        .enabled(free),
        Entry::Separator,
        Entry::item(
            lang.pick("設定を貼り付け", "Paste Settings"),
            list_action(ListOp::PasteSettings(id)),
        )
        .enabled(free && pasteable),
        Entry::item(
            lang.pick("位置を貼り付け", "Paste Positions"),
            list_action(ListOp::PastePositions(id)),
        )
        .enabled(free && pasteable),
        Entry::Separator,
        Entry::item(
            lang.pick("上へ移動", "Move Up"),
            list_action(ListOp::Move(id, Toward::Up)),
        )
        .enabled(free && index + 1 < entries.len()),
        Entry::item(
            lang.pick("下へ移動", "Move Down"),
            list_action(ListOp::Move(id, Toward::Down)),
        )
        .enabled(free && index > 0),
        Entry::Separator,
        Entry::item(
            lang.pick("パスを削除", "Delete Path"),
            list_action(ListOp::Delete(id)),
        )
        .enabled(free),
    ]
}

/// 一覧の行（上が後に描くパス）と、その下の操作のアイコン。
fn list_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let Some(layer) = app.selected_layer else {
        return;
    };
    let entries = app.path_entries().to_vec();
    let active = app.path_layer().map(|(_, p)| p.id());
    let free = app.can_edit();
    for index in (0..entries.len()).rev() {
        let e = &entries[index];
        let id = e.id();
        let r = rows.row(t::ROW_HEIGHT, 1.0);
        let selected = active == Some(id);
        let name = display_name(lang, &entries, index);
        let response = ui.interact(r, ui.make_persistent_id(("path.row", id)), Sense::click());
        {
            let p = ui.painter();
            if selected {
                w::fill(p, r, t::ACCENT_SOFT);
                w::fill(
                    p,
                    Rect::from_min_size(r.min, vec2(3.0, r.height())),
                    t::ACCENT,
                );
            } else if response.hovered() {
                w::fill(p, r, t::CONTROL_HOVER);
            }
        }
        let eye = Rect::from_min_size(pos2(r.left() + 4.0, r.top()), vec2(20.0, r.height()));
        if w::icon_button(
            ui,
            eye,
            ("path.visible", id),
            if e.visible {
                "visibility"
            } else {
                "visibility_off"
            },
            lang.pick("表示", "Visible"),
            false,
            free,
            14.0,
        )
        .clicked()
        {
            app.apply(list_action(ListOp::Visible(id, !e.visible)));
        }
        let name_rect = Rect::from_min_max(pos2(eye.right() + 6.0, r.top()), r.max);
        let renaming = app
            .path
            .renaming
            .is_some_and(|a| a.layer == layer && a.path == id);
        if renaming {
            let first = !app.path.rename_started;
            app.path.rename_started = true;
            let out = w::text_field(
                ui,
                name_rect.shrink2(vec2(0.0, 1.0)),
                ("path.rename", id),
                &name,
                None,
                first,
            );
            if first || out.focused {
                app.path.note_typing(ui.ctx().cumulative_pass_nr());
            }
            if let Some(next) = out.committed {
                // 番号の名前のままなら名前を付けない（並べ替えで番号が変わっても付け直す）
                let next = if next.trim() == name && e.name.is_empty() {
                    String::new()
                } else {
                    next
                };
                app.apply(list_action(ListOp::Rename(id, next)));
            }
            if !first && !out.focused {
                app.path.renaming = None;
            }
        } else {
            let color = if !e.visible {
                t::TEXT_DIM
            } else if selected {
                Color32::WHITE
            } else {
                t::TEXT
            };
            let shown = w::fit(ui.painter(), &name, name_rect.width() - 2.0, t::LABEL);
            w::text(
                ui.painter(),
                name_rect,
                &shown,
                t::LABEL.with_color(color),
                w::Align::Left,
            );
        }
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &name)
        });
        if response.clicked() {
            app.apply(Action::Path(PathAction::SelectPath(Some(id))));
        }
        // 前の押しから間が無いと 3 回目の押しに数えられる（ブラシの一覧と同じく、それも名前の変更に）
        if (response.double_clicked() || response.triple_clicked()) && free {
            app.apply(Action::Path(PathAction::BeginRename(id)));
        }
        if response.secondary_clicked() {
            app.apply(Action::Path(PathAction::SelectPath(Some(id))));
            if let Some(at) = response.interact_pointer_pos() {
                let ctx = ui.ctx().clone();
                app.popup = Some(OpenPopup {
                    kind: PopupKind::M2(Popup::PathContext(id)),
                    state: PopupState::new(&ctx, context_anchor(at)),
                });
            }
        }
    }
    // 操作のアイコン: 新しいパス・複製・上へ・下へ・削除（選んだパスに）
    let r = rows.row(22.0, 4.0);
    let index = active.and_then(|id| entries.iter().position(|e| e.id() == id));
    let new_tip = new_path_tip(lang);
    let buttons: [(&str, &str, &str, bool, Option<Action>); 5] = [
        (
            "path.list.new",
            "add",
            &new_tip,
            free && active.is_some(),
            Some(Action::Path(PathAction::SelectPath(None))),
        ),
        (
            "path.list.duplicate",
            "copy_add",
            lang.pick("パスを複製", "Duplicate Path"),
            free && index.is_some(),
            active.map(|id| list_action(ListOp::Duplicate(id))),
        ),
        (
            "path.list.up",
            "expand_less",
            lang.pick("上へ移動", "Move Up"),
            free && index.is_some_and(|i| i + 1 < entries.len()),
            active.map(|id| list_action(ListOp::Move(id, Toward::Up))),
        ),
        (
            "path.list.down",
            "expand_more",
            lang.pick("下へ移動", "Move Down"),
            free && index.is_some_and(|i| i > 0),
            active.map(|id| list_action(ListOp::Move(id, Toward::Down))),
        ),
        (
            "path.list.delete",
            "delete",
            lang.pick("パスを削除", "Delete Path"),
            free && index.is_some(),
            active.map(|id| list_action(ListOp::Delete(id))),
        ),
    ];
    let mut x = r.left();
    for (key, icon, tip, enabled, action) in buttons {
        let b = Rect::from_min_size(pos2(x, r.top()), vec2(24.0, r.height()));
        x += 26.0;
        if w::icon_button(ui, b, key, icon, tip, false, enabled, 15.0).clicked() {
            if let Some(action) = action {
                app.apply(action);
            }
        }
    }
    rows.space(2.0);
}

fn path_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    // 選んでいるレイヤーにパスが無いあいだは、一覧と点の操作の欄は空なので出さない（次に作るパスのブラシは下の欄）
    if app.path_entries().is_empty() {
        return;
    }
    let (open, _) = section(
        ui,
        app,
        rows,
        "path",
        lang.pick("パス", "Path"),
        "conversion_path",
        None,
    );
    if !open {
        return;
    }
    list_rows(ui, app, rows);
    if app.path_layer().is_none() {
        rows.space(4.0);
        return;
    }
    let free = app.can_edit();
    // 別のモデルで描かれた 3D のパス（編集できない）
    let other_model = match (app.path_layer(), app.view3d.full_model()) {
        (Some((_, LayerPath::Surface(s))), Some(m)) => {
            *app.path_fingerprint(&m.geometry) != s.model_fingerprint
        }
        (Some((_, LayerPath::Surface(_))), None) => true,
        _ => false,
    };
    if other_model {
        let r = rows.row(t::ROW_HEIGHT, 2.0);
        let text = lang.pick("別のモデルで描かれています", "Drawn on another model");
        let shown = w::fit(ui.painter(), text, r.width(), t::LABEL_DIM);
        w::text(
            ui.painter(),
            r,
            &shown,
            t::LABEL_DIM.with_color(t::WARNING),
            w::Align::Left,
        );
    }
    let Some((id, path)) = app.path_layer().map(|(i, p)| (i, p.clone())) else {
        rows.space(4.0);
        return;
    };
    let editable = free && !other_model;
    let closed = is_closed(&path);
    let count = path.point_count();

    // 点の操作: 閉じる/開く・点を消す
    let (close_label, close_tip) = close_label_and_tip(lang, closed);
    let delete_tip = delete_tip(lang);
    button_grid(
        ui,
        app,
        rows,
        vec![
            Btn {
                id: "path.panel.close",
                label: close_label,
                tip: close_tip,
                enabled: editable && (closed || count >= 3),
                action: Action::Path(PathAction::Point(if closed {
                    edit::PointOp::Open
                } else {
                    edit::PointOp::Close
                })),
                on: false,
            },
            Btn {
                id: "path.panel.delete",
                label: delete_label(lang),
                tip: &delete_tip,
                enabled: editable && count > 0,
                action: Action::Path(PathAction::DeleteSelected),
                on: false,
            },
        ],
    );

    // 点の太さ
    let selected = app.path_selected_index();
    let value = selected
        .and_then(|i| point_width(&path, i))
        .map_or(100.0, |v| (v * 100.0) as f32);
    let at = rows.slider_row();
    if let Some(v) = slider(
        ui,
        app,
        at,
        "path.panel.width",
        width_spec(lang, editable && selected.is_some()),
        value,
        true,
    ) {
        apply_width(app, v);
    }

    // 選んでいる点の接線（角・取っ手）
    let tangent = selected.and_then(|i| edit::point_tangent(&path, i));
    let buttons = tangent_buttons(lang, tangent)
        .into_iter()
        .map(|(id, label, tip, on, action)| Btn {
            id: if id == "path.corner" {
                "path.panel.corner"
            } else {
                "path.panel.handles"
            },
            label,
            tip,
            enabled: editable && tangent.is_some(),
            action,
            on,
        })
        .collect();
    button_grid(ui, app, rows, buttons);

    // 対称・向きの反転（パス全体）
    let symmetric = path.style().symmetry != yolu_core::paths::PathSymmetry::None;
    button_grid(
        ui,
        app,
        rows,
        vec![
            Btn {
                id: "path.symmetry",
                label: lang.pick("対称", "Symmetry"),
                tip: lang.pick("対称", "Symmetry"),
                enabled: editable,
                action: Action::Path(PathAction::Symmetry(!symmetric)),
                on: symmetric,
            },
            Btn {
                id: "path.reverse",
                label: lang.pick("向きを反転", "Reverse"),
                tip: lang.pick(
                    "パスの向き（点の並び）を逆にします。筆先やリボンの並びの向きが変わります",
                    "Reverse the path direction (point order). Tips and ribbons follow the new direction",
                ),
                enabled: editable && count >= 2,
                action: Action::Path(PathAction::Reverse),
                on: false,
            },
        ],
    );

    // ブラシを使う・描き直す（3D）・ラスタライズ
    let mut buttons = vec![Btn {
        id: "path.use-brush",
        label: lang.pick("ブラシを使う", "Use Brush"),
        tip: lang.pick("ブラシを使う", "Use Brush"),
        enabled: editable,
        action: Action::Path(PathAction::UseBrush),
        on: false,
    }];
    if !path.is_canvas() {
        buttons.push(Btn {
            id: "path.redraw",
            label: lang.pick("描き直す", "Redraw"),
            tip: redraw_tip(lang),
            enabled: editable,
            action: Action::Path(PathAction::Redraw),
            on: false,
        });
    }
    // 塗りつぶしレイヤーのパスは画素にしない（塗りつぶしレイヤーは画素を持たない）
    buttons.push(Btn {
        id: "path.rasterize.panel",
        label: lang.pick("ラスタライズ", "Rasterize"),
        tip: rasterize_tip(lang),
        enabled: free && app.path_can_rasterize(id),
        action: Action::Path(PathAction::Rasterize(id)),
        on: false,
    });
    button_grid(ui, app, rows, buttons);
    rows.space(4.0);
}

/// プリセットの選択肢: 保存したプリセット（当てる）・今の設定を保存・名前の変更・削除。
pub fn preset_entries(app: &AppState) -> Vec<Entry<Action>> {
    use crate::pathtool::presets::PresetOp;
    let lang = app.lang;
    let free = app.can_edit();
    let list = app.path.presets.list();
    let mut v: Vec<Entry<Action>> = list
        .iter()
        .map(|p| {
            Entry::item(
                p.name.clone(),
                Action::Path(PathAction::Preset(PresetOp::Apply(p.id))),
            )
            .enabled(free)
        })
        .collect();
    if !v.is_empty() {
        v.push(Entry::Separator);
    }
    v.push(Entry::item(
        lang.pick(
            "今の設定をプリセットに保存",
            "Save Current Settings as Preset",
        ),
        Action::Path(PathAction::Preset(PresetOp::Save)),
    ));
    if !list.is_empty() {
        v.push(Entry::submenu(
            lang.pick("プリセットの名前を変更", "Rename Preset"),
            list.iter()
                .map(|p| {
                    Entry::item(
                        p.name.clone(),
                        Action::Path(PathAction::Preset(PresetOp::BeginRename(p.id))),
                    )
                })
                .collect(),
        ));
        v.push(Entry::submenu(
            lang.pick("プリセットを削除", "Delete Preset"),
            list.iter()
                .map(|p| {
                    Entry::item(
                        p.name.clone(),
                        Action::Path(PathAction::Preset(PresetOp::Delete(p.id))),
                    )
                })
                .collect(),
        ));
    }
    v
}

/// プリセットのボタン（押すと一覧）。名前を変えている間は、同じ所に名前の入力欄。
fn preset_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, v: &BrushView) {
    use crate::pathtool::presets::PresetOp;
    let lang = app.lang;
    let r = rows.row(24.0, 4.0);
    let renaming = app
        .path
        .preset_renaming
        .and_then(|id| Some((id, app.path.presets.get(id)?.name.clone())));
    if app.path.preset_renaming.is_some() && renaming.is_none() {
        app.path.preset_renaming = None;
    }
    if let Some((id, name)) = renaming {
        let first = !app.path.preset_rename_started;
        app.path.preset_rename_started = true;
        let out = w::text_field(
            ui,
            r,
            ("path.preset.rename", id),
            &name,
            Some(lang.pick("プリセットの名前", "Preset name")),
            first,
        );
        if first || out.focused {
            app.path.note_typing(ui.ctx().cumulative_pass_nr());
        }
        if let Some(next) = out.committed {
            app.apply(Action::Path(PathAction::Preset(PresetOp::Rename(id, next))));
        }
        if !first && !out.focused {
            app.path.preset_renaming = None;
        }
        return;
    }
    if w::button(
        ui,
        r,
        "path.presets",
        lang.pick("プリセット", "Presets"),
        false,
        v.editable || !app.path.presets.list().is_empty(),
        Some(lang.pick(
            "保存したパスの設定（種類・ブラシ・組・筆先・深さ・対称）を当てる・今の設定を保存する",
            "Apply saved path settings (type, brush, material, tip, depth, symmetry) or save the current ones",
        )),
        Some("library"),
    )
    .clicked()
    {
        let ctx = ui.ctx().clone();
        open_popup(app, &ctx, Popup::PathPresets, r, r.width());
    }
}

/// 種類の名前。
pub fn kind_name(lang: Lang, kind: PathKind) -> &'static str {
    match kind {
        PathKind::Stroke => lang.pick("ストローク", "Stroke"),
        PathKind::Ribbon(_) => lang.pick("リボン", "Ribbon"),
        PathKind::Fill => lang.pick("塗り", "Fill"),
        PathKind::Smudge { .. } => lang.pick("指先", "Smudge"),
        PathKind::Erase => lang.pick("消しゴム", "Erase"),
    }
}

fn ribbon_mode_name(lang: Lang, mode: RibbonMode) -> &'static str {
    match mode {
        RibbonMode::Tile => lang.pick("並べる", "Tile"),
        RibbonMode::Stretch => lang.pick("伸ばす", "Stretch"),
    }
}

/// アセットの画像（リソースの ID・名前）。
fn asset_images(app: &AppState) -> Vec<(ImageId, String)> {
    app.shelf
        .resources()
        .iter()
        .filter(|r| r.kind == "image")
        .filter_map(|r| Some((crate::fx::inputs::image_id(&r.id)?, r.name.clone())))
        .collect()
}

/// 種類の選択肢（今の値を引き継ぐ: リボンは今の画像か、無ければアセットの最初の画像）。
pub fn kind_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = brush_view(app).kind;
    let first_image = asset_images(app).first().map(|(id, _)| *id);
    let ribbon = match current {
        PathKind::Ribbon(r) => Some(r),
        _ => first_image.map(|image| Ribbon {
            image,
            mode: RibbonMode::Tile,
            spacing: 1.0,
        }),
    };
    let mut v = Vec::new();
    for kind in [
        Some(PathKind::Stroke),
        ribbon.map(PathKind::Ribbon),
        Some(PathKind::Fill),
        Some(match current {
            PathKind::Smudge { .. } => current,
            _ => PathKind::SMUDGE,
        }),
        Some(PathKind::Erase),
    ] {
        match kind {
            Some(k) => v.push(
                Entry::item(kind_name(lang, k), Action::Path(PathAction::Kind(k)))
                    .radio(std::mem::discriminant(&k) == std::mem::discriminant(&current)),
            ),
            // アセットに画像が無ければ、リボンは選べない（理由はツールチップ）
            None => v.push(
                Entry::item(
                    lang.pick("リボン", "Ribbon"),
                    Action::Path(PathAction::Kind(PathKind::Stroke)),
                )
                .enabled(false)
                .tooltip(lang.pick("アセットに画像がありません", "No image in the assets")),
            ),
        }
    }
    v
}

/// リボンの画像の選択肢（アセットの画像）。
pub fn ribbon_image_entries(app: &AppState) -> Vec<Entry<Action>> {
    let PathKind::Ribbon(r) = brush_view(app).kind else {
        return Vec::new();
    };
    asset_images(app)
        .into_iter()
        .map(|(image, name)| {
            Entry::item(
                name,
                Action::Path(PathAction::Kind(PathKind::Ribbon(Ribbon { image, ..r }))),
            )
            .radio(image == r.image)
        })
        .collect()
}

/// リボンの並べ方の選択肢。
pub fn ribbon_mode_entries(app: &AppState) -> Vec<Entry<Action>> {
    let PathKind::Ribbon(r) = brush_view(app).kind else {
        return Vec::new();
    };
    [RibbonMode::Tile, RibbonMode::Stretch]
        .into_iter()
        .map(|mode| {
            Entry::item(
                ribbon_mode_name(app.lang, mode),
                Action::Path(PathAction::Kind(PathKind::Ribbon(Ribbon { mode, ..r }))),
            )
            .radio(mode == r.mode)
        })
        .collect()
}

/// パスのブラシの縁のアンチエイリアスの箱（押されたら箱の矩形）。名前と値が 1 行に収まらない狭い欄では、名前を上の行に置く
/// （値を詰めない）。
fn anti_alias_row(
    ui: &mut Ui,
    rows: &mut Rows,
    lang: Lang,
    level: yolu_core::AntiAlias,
    editable: bool,
) -> Option<Rect> {
    let label = lang.pick("アンチエイリアス", "Anti-aliasing");
    let value = crate::m2::anti_alias_label(lang, level);
    let tip = lang.pick(
        "縁のギザギザをならす強さ",
        "How much the jagged edge is smoothed",
    );
    let p = ui.painter();
    let needed =
        w::text_width(p, label, t::LABEL) + 10.0 + w::text_width(p, value, t::LABEL) + 30.0;
    if rows.width() >= needed {
        return choice_row(
            ui,
            rows,
            "path.anti_alias",
            label,
            value,
            Some(tip),
            editable,
        );
    }
    let head = rows.row(16.0, 1.0);
    w::text(
        ui.painter(),
        head,
        label,
        t::LABEL.with_color(if editable { t::TEXT } else { t::TEXT_DISABLED }),
        w::Align::Left,
    );
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, b) = w::dropdown(
        ui,
        r,
        "path.anti_alias",
        None,
        value,
        Some(tip),
        editable,
        0.0,
    );
    let name = format!("{label}: {value}");
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, editable, &name));
    response.clicked().then_some(b)
}

/// パスのブラシの縁のアンチエイリアスの選び。
pub fn anti_alias_entries(app: &AppState) -> Vec<Entry<Action>> {
    let current = brush_view(app).anti_alias;
    yolu_core::AntiAlias::ALL
        .into_iter()
        .map(|level| {
            Entry::item(
                crate::m2::anti_alias_label(app.lang, level),
                Action::Path(PathAction::Brush(BrushEdit::AntiAlias(level))),
            )
            .radio(level == current)
        })
        .collect()
}

/// 種類の行（種類・リボンの画像と並べ方と間隔・指先の強さ）。
fn kind_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, v: &BrushView) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    if let Some(r) = choice_row(
        ui,
        rows,
        "path.kind",
        lang.pick("種類", "Type"),
        kind_name(lang, v.kind),
        Some(lang.pick(
            "パスに沿って描くもの（ストローク・画像のリボン・内側の塗り・指先・消しゴム）",
            "What the path draws (stroke, image ribbon, inside fill, smudge, erase)",
        )),
        v.editable,
    ) {
        open_popup(app, &ctx, Popup::PathKind, r, r.width());
    }
    match v.kind {
        PathKind::Ribbon(ribbon) => {
            let name = asset_images(app)
                .into_iter()
                .find(|(id, _)| *id == ribbon.image)
                .map(|(_, n)| n)
                .unwrap_or_else(|| lang.pick("見つからない画像", "Missing image").into());
            if let Some(r) = choice_row(
                ui,
                rows,
                "path.ribbon.image",
                lang.pick("画像", "Image"),
                &name,
                Some(lang.pick(
                    "パスに沿って並べるアセットの画像",
                    "The asset image laid along the path",
                )),
                v.editable,
            ) {
                open_popup(app, &ctx, Popup::PathRibbonImage, r, r.width());
            }
            if let Some(r) = choice_row(
                ui,
                rows,
                "path.ribbon.mode",
                lang.pick("並べ方", "Layout"),
                ribbon_mode_name(lang, ribbon.mode),
                Some(lang.pick(
                    "並べる: ダブごとに画像の全部。伸ばす: パス全体に 1 枚",
                    "Tile: the whole image per dab. Stretch: one image along the whole path",
                )),
                v.editable,
            ) {
                open_popup(app, &ctx, Popup::PathRibbonMode, r, r.width());
            }
            if ribbon.mode == RibbonMode::Tile {
                let at = rows.slider_row();
                let spec = SliderSpec::new(
                    lang.pick("並べる間隔", "Tile Spacing"),
                    10.0,
                    400.0,
                    NumberFormat::int("%"),
                )
                .tooltip(lang.pick(
                    "画像の中心の間隔（画像の長さに対する割合。100% で隙間なく並ぶ）",
                    "Distance between images (of an image's length; 100% makes them touch)",
                ))
                .enabled(v.editable);
                if let Some(x) = slider(
                    ui,
                    app,
                    at,
                    RIBBON_SPACING,
                    spec,
                    (ribbon.spacing * 100.0) as f32,
                    v.deferred,
                ) {
                    let spacing = (x as f64 / 100.0).clamp(0.1, 4.0);
                    app.apply(Action::Path(PathAction::Kind(PathKind::Ribbon(Ribbon {
                        spacing,
                        ..ribbon
                    }))));
                }
            }
        }
        PathKind::Smudge { strength } => {
            let at = rows.slider_row();
            let spec = SliderSpec::new(
                lang.pick("強さ", "Strength"),
                0.0,
                100.0,
                NumberFormat::int("%"),
            )
            .enabled(v.editable);
            if let Some(x) = slider(
                ui,
                app,
                at,
                SMUDGE_STRENGTH,
                spec,
                (strength * 100.0) as f32,
                v.deferred,
            ) {
                app.apply(Action::Path(PathAction::Kind(PathKind::Smudge {
                    strength: (x as f64 / 100.0).clamp(0.0, 1.0),
                })));
            }
        }
        _ => {}
    }
}

fn style_action(edit: StyleEdit) -> Action {
    Action::Path(PathAction::Style(edit))
}

/// 筆先の名前（丸・組み込みの名前・取り込んだ画像の名前）。
fn tip_name(lang: Lang, tip: Option<&yolu_core::BrushTip>) -> String {
    match tip {
        None => lang.pick("丸", "Round").into(),
        Some(t) => crate::m2::tip_display(lang, t),
    }
}

/// 筆先の選択肢: 丸・今のブラシの筆先（画像の筆先のとき）・組み込みの筆先。
pub fn tip_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = brush_view(app).style.tip;
    let same = |t: &Arc<yolu_core::BrushTip>| current.as_deref() == Some(&**t);
    let mut v = vec![
        Entry::item(tip_name(lang, None), style_action(StyleEdit::Tip(None)))
            .radio(current.is_none()),
    ];
    if let Some(t) = &app.m2.brush.tip.image {
        v.push(
            Entry::item(
                format!(
                    "{}（{}）",
                    lang.pick("今のブラシの筆先", "Current brush tip"),
                    crate::m2::tip_display(lang, t)
                ),
                style_action(StyleEdit::Tip(Some(t.clone()))),
            )
            .radio(same(t)),
        );
    }
    v.push(Entry::Separator);
    for id in yolu_core::brush::BUILTIN_TIPS {
        if let Some(t) = yolu_core::builtin_tip(id) {
            v.push(
                Entry::item(
                    crate::m2::tip_label(lang, id),
                    style_action(StyleEdit::Tip(Some(t.clone()))),
                )
                .radio(same(&t)),
            );
        }
    }
    v
}

/// 筆先の行（筆先・角度・パスに沿う）。
fn tip_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, v: &BrushView) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    if let Some(r) = choice_row(
        ui,
        rows,
        "path.tip",
        lang.pick("筆先", "Tip"),
        &tip_name(lang, v.style.tip.as_deref()),
        Some(lang.pick(
            "パスのダブの形（丸か画像の筆先）",
            "The shape of each dab (round or an image tip)",
        )),
        v.editable,
    ) {
        open_popup(app, &ctx, Popup::PathTip, r, r.width());
    }
    if v.style.tip.is_none() {
        return;
    }
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("角度", "Angle"),
        -180.0,
        180.0,
        NumberFormat::int("°"),
    )
    .tooltip(lang.pick(
        "筆先の向き（パスに沿うときはパスの進む向きから、そうでなければ 2D はキャンバス、3D はモデルの上から）",
        "Tip direction (from the path direction when following it; otherwise from the canvas in 2D and model up in 3D)",
    ))
    .enabled(v.editable);
    if let Some(a) = slider(
        ui,
        app,
        at,
        TIP_ANGLE,
        spec,
        v.style.angle as f32,
        v.deferred,
    ) {
        app.apply(style_action(StyleEdit::Angle(a as f64)));
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "path.follow",
        lang.pick("パスに沿う", "Follow Path"),
        v.style.follow,
        Some(lang.pick(
            "筆先をパスの進む向きに回します",
            "Turn the tip with the path direction",
        )),
        v.editable,
    ) {
        app.apply(style_action(StyleEdit::Follow(on)));
    }
}

/// 3D の投影の深さの行（自動・深さ）。
fn depth_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, v: &BrushView) {
    let lang = app.lang;
    let auto = v.style.depth.is_none();
    if let Some(on) = toggle_row(
        ui,
        rows,
        "path.depth-auto",
        lang.pick("投影の深さを自動にする", "Automatic projection depth"),
        auto,
        Some(lang.pick(
            "曲線から面を探す深さを、点の間の長さとブラシの大きさから決めます",
            "Find the surface as deep as the point spacing and brush size need",
        )),
        v.editable,
    ) {
        app.apply(style_action(StyleEdit::Depth(if on {
            None
        } else {
            Some(4.0)
        })));
    }
    if auto {
        return;
    }
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("投影の深さ", "Projection Depth"),
        5.0,
        1600.0,
        NumberFormat::int("%"),
    )
    .tooltip(lang.pick(
        "曲線から法線の向きに面を探す深さ（ブラシの半径に対する割合）。浅いと、面から離れた所は描きません",
        "How far along the normal the surface is searched (of the brush radius). Shallow depths skip parts away from the surface",
    ))
    .power(2.0)
    .enabled(v.editable);
    let value = (v.style.depth.unwrap_or(4.0) * 100.0) as f32;
    if let Some(d) = slider(ui, app, at, DEPTH, spec, value, v.deferred) {
        app.apply(style_action(StyleEdit::Depth(Some(
            (d as f64 / 100.0).clamp(0.05, 16.0),
        ))));
    }
}

/// ブラシの値の見え方（パスがあればそのブラシ、無ければ次に作るパスが取る今のブラシ）。
struct BrushView {
    /// 直径（画素。3D のパスはモデルが無いと求まらない）。
    diameter: Option<f32>,
    hardness: f32,
    spacing: f32,
    opacity: f32,
    flow: f32,
    pressure: [bool; 3],
    /// 種類（パスがあればそのパスの、無ければ次に作るパスの）。
    kind: PathKind,
    /// 筆先・角度・向き・深さ（パスがあればそのパスの、無ければ次に作るパスの）。
    style: yolu_core::paths::PathStyle,
    /// 3D のパスか（投影の深さの欄を出す）。
    surface: bool,
    /// パスのブラシか（スライダーは離したとき 1 回で描き直す）。
    deferred: bool,
    editable: bool,
    anti_alias: yolu_core::AntiAlias,
}

fn brush_view(app: &AppState) -> BrushView {
    let free = app.can_edit();
    match app.path_layer() {
        Some((_, path)) => {
            let PathBrush(b) = path_brush(path);
            let diameter = match path {
                LayerPath::Canvas(_) => Some((b.radius * 2.0) as f32),
                LayerPath::Surface(_) => app.view3d.full_model().map(|m| {
                    let unit = world_radius(m.rest_geometry(), 1.0, app.doc.width())
                        .max(f32::MIN_POSITIVE);
                    (b.radius as f32 / unit) * 2.0
                }),
            };
            // 別のモデルで描かれた 3D のパスは直せない
            let other = match (path, app.view3d.full_model()) {
                (LayerPath::Surface(s), Some(m)) => {
                    *app.path_fingerprint(&m.geometry) != s.model_fingerprint
                }
                (LayerPath::Surface(_), None) => true,
                _ => false,
            };
            BrushView {
                diameter,
                hardness: b.hardness as f32,
                spacing: b.spacing as f32,
                opacity: b.opacity as f32,
                flow: b.flow as f32,
                pressure: [b.pressure_size, b.pressure_opacity, b.pressure_flow],
                kind: path.style().kind,
                style: path.style().clone(),
                surface: !path.is_canvas(),
                deferred: true,
                editable: free && !other && diameter.is_some(),
                anti_alias: b.anti_alias,
            }
        }
        None => {
            let b = &app.brush;
            BrushView {
                diameter: Some(b.radius * 2.0),
                hardness: b.hardness,
                spacing: b.spacing,
                opacity: b.opacity,
                flow: b.flow,
                pressure: [b.pressure_size, b.pressure_opacity, b.pressure_flow],
                kind: app.path.next_style.kind,
                style: app.path.next_style.clone(),
                surface: false,
                deferred: false,
                editable: free,
                anti_alias: b.anti_alias,
            }
        }
    }
}

fn brush_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "path-brush",
        lang.pick("パスのブラシ", "Path Brush"),
        "paint_brush",
        None,
    );
    if !open {
        return;
    }
    let v = brush_view(app);
    preset_row(ui, app, rows, &v);
    kind_rows(ui, app, rows, &v);
    let stroke_like = matches!(
        v.kind,
        PathKind::Stroke | PathKind::Erase | PathKind::Smudge { .. }
    );
    if stroke_like {
        tip_rows(ui, app, rows, &v);
    }
    if v.surface && v.kind != PathKind::Fill {
        depth_rows(ui, app, rows, &v);
    }
    let edit_brush =
        |app: &mut AppState, e: BrushEdit| app.apply(Action::Path(PathAction::Brush(e)));
    // 塗りは直径を使わない（内側を塗る）。リボンの直径は幅
    if v.kind != PathKind::Fill {
        let at = rows.slider_row();
        let spec = SliderSpec::new(
            if matches!(v.kind, PathKind::Ribbon(_)) {
                lang.pick("幅", "Width")
            } else {
                lang.pick("直径", "Size")
            },
            1.0,
            256.0,
            NumberFormat::int(" px"),
        )
        .enabled(v.editable);
        if let Some(d) = slider(
            ui,
            app,
            at,
            DIAMETER,
            spec,
            v.diameter.unwrap_or(0.0),
            v.deferred,
        ) {
            edit_brush(app, BrushEdit::Diameter(d as f64));
        }
    }
    let percent = |ui: &mut Ui,
                   app: &mut AppState,
                   rows: &mut Rows,
                   key,
                   label,
                   value: f32,
                   min: f32,
                   tip: Option<&'static str>| {
        let mut spec =
            SliderSpec::new(label, min, 100.0, NumberFormat::int("%")).enabled(v.editable);
        if let Some(tip) = tip {
            spec = spec.tooltip(tip);
        }
        let at = rows.slider_row();
        slider(ui, app, at, key, spec, value * 100.0, v.deferred).map(|p| p as f64 / 100.0)
    };
    // 硬さは丸い筆先だけ（画像の筆先は縁を画像が決める）
    if stroke_like && v.style.tip.is_none() {
        if let Some(x) = percent(
            ui,
            app,
            rows,
            HARDNESS,
            lang.pick("硬さ", "Hardness"),
            v.hardness,
            0.0,
            None,
        ) {
            edit_brush(app, BrushEdit::Hardness(x));
        }
        // 縁のアンチエイリアス（丸い筆先の縁）
        let ctx = ui.ctx().clone();
        if let Some(r) = anti_alias_row(ui, rows, lang, v.anti_alias, v.editable) {
            open_popup(app, &ctx, Popup::PathAntiAlias, r, r.width());
        }
    }
    if stroke_like {
        if let Some(x) = percent(
            ui,
            app,
            rows,
            SPACING,
            lang.pick("間隔", "Spacing"),
            v.spacing,
            1.0,
            Some(lang.pick(
                "ダブの間隔（直径に対する割合）",
                "Distance between dabs (of the diameter)",
            )),
        ) {
            edit_brush(app, BrushEdit::Spacing(x));
        }
    }
    if let Some(x) = percent(
        ui,
        app,
        rows,
        OPACITY,
        lang.pick("不透明度", "Opacity"),
        v.opacity,
        0.0,
        None,
    ) {
        edit_brush(app, BrushEdit::Opacity(x));
    }
    if stroke_like {
        if let Some(x) = percent(
            ui,
            app,
            rows,
            FLOW,
            lang.pick("流量", "Flow"),
            v.flow,
            0.0,
            None,
        ) {
            edit_brush(app, BrushEdit::Flow(x));
        }
    }
    let toggles = [
        (
            "path.pressure-size",
            lang.pick("太さで直径を変える", "Width changes the size"),
            lang.pick(
                "点の太さを、描く直径に反映します",
                "The width of each point sets the drawn size",
            ),
            BrushEdit::PressureSize as fn(bool) -> BrushEdit,
        ),
        (
            "path.pressure-opacity",
            lang.pick("太さで不透明度を変える", "Width changes the opacity"),
            lang.pick(
                "点の太さを、不透明度に反映します",
                "The width of each point sets the opacity",
            ),
            BrushEdit::PressureOpacity as fn(bool) -> BrushEdit,
        ),
        (
            "path.pressure-flow",
            lang.pick("太さで流量を変える", "Width changes the flow"),
            lang.pick(
                "点の太さを、流量に反映します",
                "The width of each point sets the flow",
            ),
            BrushEdit::PressureFlow as fn(bool) -> BrushEdit,
        ),
    ];
    for (i, (id, label, tip, make)) in toggles.into_iter().enumerate() {
        // 塗りは筆圧を使わない。リボンは大きさと不透明度だけ
        let used = match v.kind {
            PathKind::Fill => false,
            PathKind::Ribbon(_) => i == 0,
            _ => true,
        };
        if !used {
            continue;
        }
        if let Some(on) = toggle_row(ui, rows, id, label, v.pressure[i], Some(tip), v.editable) {
            edit_brush(app, make(on));
        }
    }
    rows.space(4.0);
}

/// 「新しいパス」のボタンのツールチップ（キーは、ペイントのモードの今の割り当ての、パスを終える操作）。
pub fn new_path_tip(lang: Lang) -> String {
    crate::shortcuts::named_with_keys(
        lang,
        lang.pick("新しいパス", "New Path"),
        &[crate::shortcuts::key_in(
            "path.finish",
            crate::mode::EditorMode::Paint,
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3D のパスは休みの形で描くので、「描き直す」のツールチップは今のポーズに描くとは言わない。
    #[test]
    fn the_redraw_tip_says_the_pose_is_not_used() {
        let ja = redraw_tip(Lang::Ja);
        assert!(
            ja.contains("休みの形") && !ja.contains("今のポーズ"),
            "{ja}"
        );
        let en = redraw_tip(Lang::En);
        assert!(
            en.contains("rest shape") && !en.contains("current pose"),
            "{en}"
        );
    }
}
