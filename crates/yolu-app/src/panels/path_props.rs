//! パスの道具の欄: オプションバー（点の太さ・閉じる/開く・点を消す・ラスタライズ）と、左のドックのツールプロパティ（パスの節: 状態・点の操作・
//! 点の太さ・ブラシを使う・描き直す・ラスタライズ、ブラシの節: パスのブラシの値）。値の操作は
//! `Action::Path`（キー・試験と同じ道）。パスのブラシや点の太さのスライダーは、離したとき 1 回で描き直す（動かしている間は値だけ）。
//! 画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::properties::{section, status_row, toggle_row};
use crate::lang::Lang;
use crate::m2::channel_name;
use crate::pathtool::edit::{self, point_width};
use crate::pathtool::{path_brush, BrushEdit, PathAction};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};
use yolu_core::geometry::world_radius;
use yolu_core::paths::PathBrush;
use yolu_core::LayerPath;

/// スライダーの途中の値のキー。
const WIDTH: &str = "path.width";
const DIAMETER: &str = "path.diameter";
const HARDNESS: &str = "path.hardness";
const SPACING: &str = "path.spacing";
const OPACITY: &str = "path.opacity";
const FLOW: &str = "path.flow";

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

/// 点の太さのスライダーの当て方（選んでいる点の太さ 0〜1）。
fn apply_width(app: &mut AppState, percent: f32) {
    if let Some(index) = app.path_selected_index() {
        app.apply(Action::Path(PathAction::Point(edit::PointOp::Width {
            index,
            pressure: (percent / 100.0).clamp(0.0, 1.0) as f64,
        })));
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
    let (has_path, closed, count) = match app.path_layer() {
        Some((_, p)) => (true, is_closed(p), p.point_count()),
        None => (false, false, 0),
    };
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
        Some(delete_tip(lang)),
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
        free && has_path,
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

fn delete_tip(lang: Lang) -> &'static str {
    lang.pick(
        "選んでいる点（無ければ最後の点）を消します（Delete）",
        "Delete the selected point (the last one if none is selected) (Delete)",
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
struct Btn {
    id: &'static str,
    label: &'static str,
    tip: &'static str,
    enabled: bool,
    action: Action,
}

/// ボタンを欄の幅に並べる。全部が 1 行に収まらないとき（狭い欄の日本語など）は、収まる最大の列数で折り返す（「…」で詰めない）。
fn button_grid(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, buttons: Vec<Btn>) {
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
            if w::button(
                ui,
                *cell,
                b.id,
                b.label,
                false,
                b.enabled,
                Some(b.tip),
                None,
            )
            .clicked()
            {
                app.apply(b.action.clone());
            }
        }
    }
}

/// ツールプロパティ（パスの道具）: パスの節とパスのブラシの節。塗るチャンネルの組はプロパティの欄のマテリアル。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    path_section(ui, app, rows);
    brush_section(ui, app, rows);
}

/// パスの状態の短い文（2D・3D、見える点の数、描くチャンネル）。パスが無ければ None。
pub fn status_text(app: &AppState) -> Option<String> {
    let lang = app.lang;
    let (_, path) = app.path_layer()?;
    let kind = if path.is_canvas() { "2D" } else { "3D" };
    let channels = path
        .channels()
        .iter()
        .map(|c| channel_name(lang, &app.doc, *c))
        .collect::<Vec<_>>()
        .join(", ");
    // 閉じたパスは終わりに始めの点の複製を持つので、見える点の数はそのぶん少ない
    let n = path.point_count() - usize::from(is_closed(path));
    Some(lang.pick(
        format!("{kind} · {n} 点 · {channels}"),
        format!(
            "{kind} · {n} {} · {channels}",
            if n == 1 { "point" } else { "points" }
        ),
    ))
}

fn path_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    // 選んでいる層にパスが無いあいだは、点の操作の欄は空なので出さない（次に作るパスのブラシは下の欄）
    if app.path_layer().is_none() {
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
    let free = app.can_edit();
    if let Some(text) = status_text(app) {
        status_row(ui, rows, &text);
    }
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
            },
            Btn {
                id: "path.panel.delete",
                label: delete_label(lang),
                tip: delete_tip(lang),
                enabled: editable && count > 0,
                action: Action::Path(PathAction::DeleteSelected),
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

    // ブラシを使う・描き直す（3D）・ラスタライズ
    let mut buttons = vec![Btn {
        id: "path.use-brush",
        label: lang.pick("ブラシを使う", "Use Brush"),
        tip: lang.pick(
            "今のブラシと、マテリアルで塗るチャンネルの組で、パスを描き直します",
            "Redraw the path with the current brush and the channels of Brush Material",
        ),
        enabled: editable,
        action: Action::Path(PathAction::UseBrush),
    }];
    if !path.is_canvas() {
        buttons.push(Btn {
            id: "path.redraw",
            label: lang.pick("描き直す", "Redraw"),
            tip: lang.pick(
                "今のポーズのモデルの面に描き直します",
                "Redraw on the model in its current pose",
            ),
            enabled: editable,
            action: Action::Path(PathAction::Redraw),
        });
    }
    buttons.push(Btn {
        id: "path.rasterize.panel",
        label: lang.pick("ラスタライズ", "Rasterize"),
        tip: rasterize_tip(lang),
        enabled: free,
        action: Action::Path(PathAction::Rasterize(id)),
    });
    button_grid(ui, app, rows, buttons);
    rows.space(4.0);
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
    /// パスのブラシか（スライダーは離したとき 1 回で描き直す）。
    deferred: bool,
    editable: bool,
}

fn brush_view(app: &AppState) -> BrushView {
    let free = app.can_edit();
    match app.path_layer() {
        Some((_, path)) => {
            let PathBrush(b) = path_brush(path);
            let diameter = match path {
                LayerPath::Canvas(_) => Some((b.radius * 2.0) as f32),
                LayerPath::Surface(_) => app.view3d.full_model().map(|m| {
                    let unit =
                        world_radius(&m.geometry, 1.0, app.doc.width()).max(f32::MIN_POSITIVE);
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
                deferred: true,
                editable: free && !other && diameter.is_some(),
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
                deferred: false,
                editable: free,
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
    let edit_brush =
        |app: &mut AppState, e: BrushEdit| app.apply(Action::Path(PathAction::Brush(e)));
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("直径", "Size"),
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
        if let Some(on) = toggle_row(ui, rows, id, label, v.pressure[i], Some(tip), v.editable) {
            edit_brush(app, make(on));
        }
    }
    rows.space(4.0);
}
