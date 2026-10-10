//! プロパティのレイヤーの欄の「定規」の区分: 選んでいるレイヤー（グループ）に付いた定規の一覧（行ごとに表示・スナップする特殊定規の印・名前・空間・削除）と、
//! 選んだ行の欄（表示の範囲、種類ごとの値）。定規が無いレイヤーでは区分ごと出さない。値の変更は 1 回の取り消し（スライダーのドラッグはまとめる）。

use egui::{pos2, vec2, Rect, Sense, Ui, WidgetInfo, WidgetType};
use yolu_core::glam::{DVec2, DVec3};
use yolu_core::{LayerId, Ruler, RulerKind, RulerPlace, RulerScope};

use super::active::canvas_points;
use super::tool::{fit_count, kind_name};
use super::RulerAction;
use crate::lang::Lang;
use crate::panels::properties::{
    choice_buttons, group_label, section, slider_row, toggle_row, ChoiceButton,
};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

/// 行の名前（種類と主な値。「対称 6」「パース 2 点」）。
pub fn title(lang: Lang, r: &Ruler) -> String {
    match r.kind {
        RulerKind::Perspective => {
            let points = if r.two_points {
                lang.pick("2 点", "2 Points")
            } else {
                lang.pick("1 点", "1 Point")
            };
            format!("{} {points}", kind_name(lang, r.kind))
        }
        RulerKind::Symmetry => format!("{} {}", kind_name(lang, r.kind), r.lines),
        _ => kind_name(lang, r.kind).to_owned(),
    }
}

/// 向き `a`→`b` の角度（度、+X から反時計回り）。
pub fn angle_of(a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    d.y.atan2(d.x).to_degrees()
}

/// `a` から角度 `degrees` の向きに、`b` と同じ距離の点（距離が短すぎれば 1 画素）。
pub fn with_angle(a: DVec2, b: DVec2, degrees: f64) -> DVec2 {
    let length = a.distance(b).max(1.0);
    let (s, c) = degrees.to_radians().sin_cos();
    a + DVec2::new(c, s) * length
}

fn px(places: u8) -> NumberFormat<'static> {
    NumberFormat {
        decimals: places,
        trim: true,
        suffix: " px",
    }
}

/// 欄を触れるか。触れないとき（描いている最中・読むだけのセット）。読むだけのセットのときは、理由をツールチップに出す（ほかの欄と同じ）。
#[derive(Clone, Copy)]
struct Gate<'a> {
    enabled: bool,
    /// 読むだけのセットの理由の文（読むだけでなければ None）。
    reason: Option<&'a str>,
}

impl<'a> Gate<'a> {
    /// 触れない理由があれば、その文を。無ければ（触れるとき・描いている最中だけのとき）、いつものツールチップ。
    fn tip(self, normal: Option<&'a str>) -> Option<&'a str> {
        if self.enabled {
            normal
        } else {
            self.reason.or(normal)
        }
    }

    /// `tip` の、いつものツールチップがある欄用。
    fn label(self, normal: &'a str) -> &'a str {
        self.tip(Some(normal)).unwrap_or(normal)
    }

    /// 触れて、さらにこの条件も満たすか。
    fn on(self, extra: bool) -> bool {
        self.enabled && extra
    }
}

/// 値を替えた定規を、1 回の取り消し（`coalesce` ならドラッグ・スライダーをまとめる）で当てる。
fn commit(app: &mut AppState, owner: LayerId, ruler: Ruler, coalesce: bool) {
    app.apply(Action::Ruler(RulerAction::Replace {
        owner,
        ruler,
        coalesce,
    }));
}

/// 2D の定規の 2 点を替えた写し。
fn with_points(r: &Ruler, a: DVec2, b: DVec2) -> Ruler {
    let mut next = r.clone();
    next.place = RulerPlace::Canvas { a, b };
    next
}

/// 3D の定規の 3 つのベクトルを替えた写し。
fn with_model(r: &Ruler, a: DVec3, b: DVec3, up: DVec3) -> Ruler {
    let mut next = r.clone();
    next.place = RulerPlace::Model { a, b, up };
    next
}

/// 「定規」の区分。
pub fn section_for_layer(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let Some(owner) = app.selected_layer else {
        return;
    };
    let list: Vec<Ruler> = match app.doc.layer(owner) {
        Some(l) if !l.rulers().is_empty() => l.rulers().to_vec(),
        _ => return,
    };
    let (open, _) = section(
        ui,
        app,
        rows,
        "rulers",
        lang.pick("定規", "Ruler"),
        "tools/ruler",
        None,
    );
    if !open {
        return;
    }
    let reason = app
        .read_only_reason()
        .map(|r| crate::lang::refusals::read_only_set(lang, r));
    let gate = Gate {
        enabled: app.can_edit(),
        reason: reason.as_deref(),
    };
    let effective = {
        let special = app.doc.snapping_special_rulers(Some(owner));
        [special.canvas, special.model]
            .into_iter()
            .flatten()
            .filter(|_| app.rulers.snap_special)
            .map(|r| r.ruler.id)
            .collect::<Vec<_>>()
    };
    for r in &list {
        let selected = app.rulers.selected == Some((owner, r.id));
        let on = effective.contains(&r.id);
        ruler_row(ui, app, rows, owner, r, on, selected, gate);
    }
    rows.space(4.0);
    let Some(current) = app.selected_ruler().map(|(_, r)| r.clone()) else {
        return;
    };
    scope_buttons(ui, app, rows, owner, &current, gate);
    match current.place {
        RulerPlace::Canvas { .. } => canvas_fields(ui, app, rows, owner, &current, gate),
        RulerPlace::Model { .. } => model_fields(ui, app, rows, owner, &current, gate),
    }
}

#[allow(clippy::too_many_arguments)]
fn ruler_row(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    effective: bool,
    selected: bool,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    if selected {
        w::fill(ui.painter(), row, t::ACCENT_SOFT);
    }
    let body = ui.interact(
        row,
        ui.make_persistent_id(("ruler.row", r.id.0)),
        Sense::click(),
    );
    let title = title(lang, r);
    body.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &title));
    if body.clicked() {
        app.apply(Action::Ruler(RulerAction::Select(Some((owner, r.id)))));
    }
    let eye = Rect::from_min_size(row.min, vec2(22.0, row.height()));
    let mark = Rect::from_min_size(pos2(eye.right() + 2.0, row.top()), vec2(20.0, row.height()));
    let trash = Rect::from_min_size(
        pos2(row.right() - 22.0, row.top()),
        vec2(22.0, row.height()),
    );
    let space = Rect::from_min_size(
        pos2(trash.left() - 26.0, row.top()),
        vec2(24.0, row.height()),
    );
    let name = Rect::from_min_max(
        pos2(mark.right() + 4.0, row.top()),
        pos2(space.left() - 2.0, row.bottom()),
    );
    if w::icon_button(
        ui,
        eye,
        ("ruler.eye", r.id.0),
        if r.visible {
            "visibility"
        } else {
            "visibility_off"
        },
        gate.label(if r.visible {
            lang.pick("非表示にする", "Hide")
        } else {
            lang.pick("表示する", "Show")
        }),
        false,
        gate.enabled,
        16.0,
    )
    .clicked()
    {
        let mut next = r.clone();
        next.visible = !next.visible;
        commit(app, owner, next, false);
    }
    if r.is_special() {
        snap_mark(ui, app, owner, r, mark, effective, gate);
    }
    let shown = w::fit(ui.painter(), &title, name.width(), t::LABEL);
    let color = if r.visible { t::TEXT } else { t::TEXT_DIM };
    w::text(
        ui.painter(),
        name,
        &shown,
        t::LABEL.with_color(color),
        w::Align::Left,
    );
    let label = match r.space() {
        yolu_core::RulerSpace::Canvas => "2D",
        yolu_core::RulerSpace::Model => "3D",
    };
    w::text(
        ui.painter(),
        space,
        label,
        t::LABEL.with_color(t::TEXT_DIM),
        w::Align::Right,
    );
    if w::icon_button(
        ui,
        trash,
        ("ruler.delete", r.id.0),
        "delete",
        gate.label(lang.pick("定規を削除", "Delete Ruler")),
        false,
        gate.enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::Ruler(RulerAction::Delete {
            owner,
            ids: vec![r.id],
        }));
    }
}

/// スナップする特殊定規の印（点。印があって効いているのは強調色、印があって効いていないのは薄い色、印が無いのは輪郭だけ）。押すと印を付ける
/// （付いていれば外す）。
fn snap_mark(
    ui: &mut Ui,
    app: &mut AppState,
    owner: LayerId,
    r: &Ruler,
    at: Rect,
    effective: bool,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let response = ui.interact(
        at,
        ui.make_persistent_id(("ruler.snap", r.id.0)),
        if gate.enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let center = at.center();
    let p = ui.painter();
    if r.snap {
        let fill = if effective { t::ACCENT } else { t::ACCENT_DIM };
        p.circle_filled(center, 4.5, fill);
    } else {
        let tone = if response.hovered() {
            t::TEXT
        } else {
            t::TEXT_DIM
        };
        p.circle_stroke(center, 4.5, egui::Stroke::new(1.2, tone));
    }
    let tip = lang.pick("スナップする特殊定規", "Snapping Special Ruler");
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, gate.enabled, r.snap, tip));
    let clicked = response.clicked();
    let _ = response.on_hover_text(gate.label(tip));
    if clicked {
        if r.snap {
            let mut next = r.clone();
            next.snap = false;
            commit(app, owner, next, false);
        } else {
            app.apply(Action::Ruler(RulerAction::MarkSnap { owner, id: r.id }));
        }
    }
}

/// 表示の範囲（3 つのボタン。光っているのが今の範囲で、光っているものをもう一度押すと全部切る＝隠す。切った状態で押すとその範囲で出す）。
fn scope_buttons(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("表示の範囲", "Visible In"));
    let scopes = [RulerScope::All, RulerScope::Group, RulerScope::Selected];
    let labels = [
        lang.pick("すべて", "All"),
        lang.pick("グループ", "Group"),
        lang.pick("選択時", "Selected"),
    ];
    let tips = [
        lang.pick("すべてのレイヤー", "All Layers"),
        lang.pick("同じグループの中", "Within the Same Group"),
        lang.pick("選んでいるときだけ", "Only While Selected"),
    ];
    let ids = [
        "ruler.scope.all",
        "ruler.scope.group",
        "ruler.scope.selected",
    ];
    let items: Vec<ChoiceButton> = (0..3)
        .map(|i| ChoiceButton {
            id: ids[i],
            label: labels[i],
            selected: r.visible && r.scope == scopes[i],
            enabled: gate.enabled,
            tooltip: Some(gate.label(tips[i])),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let mut next = r.clone();
        if r.visible && r.scope == scopes[i] {
            next.visible = false;
        } else {
            next.visible = true;
            next.scope = scopes[i];
        }
        commit(app, owner, next, false);
    }
}

/// X・Y の欄の範囲 `((x の最小, 最大), (y の最小, 最大))`。普通の点はキャンバスの中。
fn canvas_ranges(size: (u32, u32)) -> ((f32, f32), (f32, f32)) {
    ((0.0, size.0 as f32), (0.0, size.1 as f32))
}

/// パースの消失点は、キャンバスの外に置くことが多いので、キャンバスの大きさだけ外まで（−大きさ〜2×大きさ）。
fn vanishing_ranges(size: (u32, u32)) -> ((f32, f32), (f32, f32)) {
    let (w, h) = (size.0 as f32, size.1 as f32);
    ((-w, 2.0 * w), (-h, 2.0 * h))
}

/// X・Y の 2 つのスライダー（文書の画素）。替わった値を返す。
fn xy_sliders(
    ui: &mut Ui,
    rows: &mut Rows,
    salt: &str,
    labels: (&str, &str),
    value: DVec2,
    ranges: ((f32, f32), (f32, f32)),
    gate: Gate<'_>,
) -> Option<DVec2> {
    let x = slider_row(
        ui,
        rows,
        &format!("{salt}.x"),
        labels.0,
        value.x as f32,
        ranges.0,
        px(1),
        gate.tip(None),
        gate.enabled,
    );
    let y = slider_row(
        ui,
        rows,
        &format!("{salt}.y"),
        labels.1,
        value.y as f32,
        ranges.1,
        px(1),
        gate.tip(None),
        gate.enabled,
    );
    (x.is_some() || y.is_some())
        .then(|| DVec2::new(x.map_or(value.x, f64::from), y.map_or(value.y, f64::from)))
}

fn canvas_fields(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let Some((a, b)) = canvas_points(r) else {
        return;
    };
    let size = (app.doc.width(), app.doc.height());
    let salt = format!("ruler.{}", r.id);
    match r.kind {
        RulerKind::Line => {
            if let Some(v) = xy_sliders(
                ui,
                rows,
                &format!("{salt}.a"),
                (
                    lang.pick("始点 X", "Start X"),
                    lang.pick("始点 Y", "Start Y"),
                ),
                a,
                canvas_ranges(size),
                gate,
            ) {
                commit(app, owner, with_points(r, v, b), true);
            }
            if let Some(v) = xy_sliders(
                ui,
                rows,
                &format!("{salt}.b"),
                (lang.pick("終点 X", "End X"), lang.pick("終点 Y", "End Y")),
                b,
                canvas_ranges(size),
                gate,
            ) {
                commit(app, owner, with_points(r, a, v), true);
            }
        }
        RulerKind::Parallel => {
            angle_slider(ui, app, rows, owner, r, (a, b), gate);
        }
        RulerKind::Concentric => {
            if let Some(v) = xy_sliders(
                ui,
                rows,
                &format!("{salt}.a"),
                (
                    lang.pick("中心 X", "Center X"),
                    lang.pick("中心 Y", "Center Y"),
                ),
                a,
                canvas_ranges(size),
                gate,
            ) {
                commit(app, owner, with_points(r, v, b + (v - a)), true);
            }
        }
        RulerKind::Perspective => {
            perspective_points(ui, app, rows, owner, r, (a, b), size, gate);
        }
        RulerKind::Symmetry => {
            symmetry_count(ui, app, rows, owner, r, gate);
            angle_slider(ui, app, rows, owner, r, (a, b), gate);
            if let Some(v) = xy_sliders(
                ui,
                rows,
                &format!("{salt}.a"),
                (
                    lang.pick("中心 X", "Center X"),
                    lang.pick("中心 Y", "Center Y"),
                ),
                a,
                canvas_ranges(size),
                gate,
            ) {
                commit(app, owner, with_points(r, v, b + (v - a)), true);
            }
            let center = DVec2::new(size.0 as f64 / 2.0, size.1 as f64 / 2.0);
            let row = rows.row(24.0, 4.0);
            if w::button(
                ui,
                row,
                "ruler.center_canvas",
                lang.pick("キャンバスの中心", "Canvas Center"),
                false,
                gate.on(a != center),
                gate.tip(None),
                None,
            )
            .clicked()
            {
                commit(app, owner, with_points(r, center, b + (center - a)), false);
            }
        }
    }
}

fn angle_slider(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    (a, b): (DVec2, DVec2),
    gate: Gate<'_>,
) {
    let lang = app.lang;
    if let Some(v) = slider_row(
        ui,
        rows,
        &format!("ruler.{}.angle", r.id),
        lang.pick("角度", "Angle"),
        angle_of(a, b) as f32,
        (-180.0, 180.0),
        NumberFormat {
            decimals: 1,
            trim: true,
            suffix: "°",
        },
        gate.tip(None),
        gate.enabled,
    ) {
        commit(
            app,
            owner,
            with_points(r, a, with_angle(a, b, v as f64)),
            true,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn perspective_points(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    (a, b): (DVec2, DVec2),
    size: (u32, u32),
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let items = [
        ChoiceButton {
            id: "ruler.one",
            label: lang.pick("1 点", "1 Point"),
            selected: !r.two_points,
            enabled: gate.enabled,
            tooltip: gate.tip(None),
        },
        ChoiceButton {
            id: "ruler.two",
            label: lang.pick("2 点", "2 Points"),
            selected: r.two_points,
            enabled: gate.enabled,
            tooltip: gate.tip(None),
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let mut next = r.clone();
        next.two_points = i == 1;
        commit(app, owner, next, false);
    }
    let salt = format!("ruler.{}", r.id);
    if let Some(v) = xy_sliders(
        ui,
        rows,
        &format!("{salt}.a"),
        (
            lang.pick("消失点 1 X", "Vanishing Point 1 X"),
            lang.pick("消失点 1 Y", "Vanishing Point 1 Y"),
        ),
        a,
        vanishing_ranges(size),
        gate,
    ) {
        // 1 点のときは 2 つ目の点（使わないが間隔を保つ）も一緒に動かす
        let b = if r.two_points { b } else { b + (v - a) };
        commit(app, owner, with_points(r, v, b), true);
    }
    if r.two_points {
        if let Some(v) = xy_sliders(
            ui,
            rows,
            &format!("{salt}.b"),
            (
                lang.pick("消失点 2 X", "Vanishing Point 2 X"),
                lang.pick("消失点 2 Y", "Vanishing Point 2 Y"),
            ),
            b,
            vanishing_ranges(size),
            gate,
        ) {
            commit(app, owner, with_points(r, a, v), true);
        }
    }
}

/// 対称定規の線の本数と線対称。
fn symmetry_count(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    if let Some(v) = slider_row(
        ui,
        rows,
        &format!("ruler.{}.lines", r.id),
        lang.pick("線の本数", "Lines"),
        r.lines as f32,
        (2.0, 16.0),
        NumberFormat::int(""),
        gate.tip(None),
        gate.enabled,
    ) {
        let mut next = r.clone();
        next.lines = fit_count(v, r.line_symmetry);
        commit(app, owner, next, true);
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        &format!("ruler.{}.line_symmetry", r.id),
        lang.pick("線対称", "Line Symmetry"),
        r.line_symmetry,
        gate.tip(None),
        gate.enabled,
    ) {
        let mut next = r.clone();
        next.line_symmetry = on;
        next.lines = super::fit_lines(next.lines, on);
        commit(app, owner, next, false);
    }
}

fn model_fields(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let RulerPlace::Model { a, b, up } = r.place else {
        return;
    };
    match r.kind {
        RulerKind::Perspective => {
            let items = [
                ChoiceButton {
                    id: "ruler.one",
                    label: lang.pick("1 点", "1 Point"),
                    selected: !r.two_points,
                    enabled: gate.enabled,
                    tooltip: gate.tip(None),
                },
                ChoiceButton {
                    id: "ruler.two",
                    label: lang.pick("2 点", "2 Points"),
                    selected: r.two_points,
                    enabled: gate.enabled,
                    tooltip: gate.tip(None),
                },
            ];
            if let Some(i) = choice_buttons(ui, rows, &items) {
                let mut next = r.clone();
                next.two_points = i == 1;
                commit(app, owner, next, false);
            }
        }
        RulerKind::Symmetry => {
            symmetry_count(ui, app, rows, owner, r, gate);
            axis_buttons(ui, app, rows, owner, r, (a, b, up), gate);
            center_buttons(ui, app, rows, owner, r, (a, b, up), gate);
            if let Some(on) = toggle_row(
                ui,
                rows,
                &format!("ruler.{}.see_through", r.id),
                lang.pick("見えない面にも写す", "Paint Hidden Surfaces"),
                r.see_through,
                gate.tip(None),
                gate.enabled,
            ) {
                let mut next = r.clone();
                next.see_through = on;
                commit(app, owner, next, false);
            }
        }
        // 位置は編集モードで動かす（欄に数を並べない）
        RulerKind::Line | RulerKind::Parallel | RulerKind::Concentric => {}
    }
}

const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

/// 軸 [X][Y][Z]: 回転の軸（up）をモデルの軸へ、最初の線をその次の軸へ。
fn axis_buttons(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    (a, b, up): (DVec3, DVec3, DVec3),
    gate: Gate<'_>,
) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("軸", "Axis"));
    let items: Vec<ChoiceButton> = ["X", "Y", "Z"]
        .iter()
        .enumerate()
        .map(|(i, name)| ChoiceButton {
            id: ["ruler.axis.x", "ruler.axis.y", "ruler.axis.z"][i],
            label: name,
            selected: up.normalize_or_zero().dot(AXES[i]).abs() > 0.999,
            enabled: gate.enabled,
            tooltip: gate.tip(None),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let length = a.distance(b).max(1e-3);
        let axis = AXES[i];
        let first = AXES[(i + 1) % 3];
        commit(
            app,
            owner,
            with_model(r, a, a + first * length, axis),
            false,
        );
    }
}

/// 中心 [原点][境界の中心]: 定規の中心（a）をモデルの原点・境界の中心へ（向きはそのまま）。
fn center_buttons(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    owner: LayerId,
    r: &Ruler,
    (a, b, up): (DVec3, DVec3, DVec3),
    gate: Gate<'_>,
) {
    let lang = app.lang;
    let bounds = app
        .view3d
        .model
        .as_ref()
        .map(|m| m.geometry.bounds().center.as_dvec3());
    group_label(ui, rows, lang.pick("中心", "Center"));
    let items = [
        ChoiceButton {
            id: "ruler.center.origin",
            label: lang.pick("原点", "Origin"),
            selected: false,
            enabled: gate.on(a != DVec3::ZERO),
            tooltip: gate.tip(None),
        },
        ChoiceButton {
            id: "ruler.center.bounds",
            label: lang.pick("境界の中心", "Bounds Center"),
            selected: false,
            enabled: gate.on(bounds.is_some_and(|c| c != a)),
            tooltip: gate.tip(None),
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let to = if i == 0 { Some(DVec3::ZERO) } else { bounds };
        if let Some(to) = to {
            commit(app, owner, with_model(r, to, b + (to - a), up), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::RulerId;

    #[test]
    fn titles_name_the_kind_and_the_main_value() {
        let mut r = Ruler::canvas(RulerId(1), RulerKind::Symmetry, DVec2::ZERO, DVec2::X);
        r.lines = 6;
        assert_eq!(title(Lang::Ja, &r), "対称 6");
        assert_eq!(title(Lang::En, &r), "Symmetry 6");
        let mut p = Ruler::canvas(RulerId(2), RulerKind::Perspective, DVec2::ZERO, DVec2::X);
        assert_eq!(title(Lang::Ja, &p), "パース 1 点");
        p.two_points = true;
        assert_eq!(title(Lang::En, &p), "Perspective 2 Points");
        let l = Ruler::canvas(RulerId(3), RulerKind::Line, DVec2::ZERO, DVec2::X);
        assert_eq!(title(Lang::Ja, &l), "直線定規");
    }

    #[test]
    fn only_vanishing_points_may_sit_outside_the_canvas() {
        assert_eq!(canvas_ranges((100, 50)), ((0.0, 100.0), (0.0, 50.0)));
        assert_eq!(
            vanishing_ranges((100, 50)),
            ((-100.0, 200.0), (-50.0, 100.0)),
            "−大きさ〜2×大きさ"
        );
    }

    #[test]
    fn setting_the_angle_keeps_the_distance() {
        let a = DVec2::new(10.0, 10.0);
        let b = DVec2::new(10.0, 14.0);
        let turned = with_angle(a, b, 0.0);
        assert!((turned - DVec2::new(14.0, 10.0)).length() < 1e-12);
        assert!((angle_of(a, turned)).abs() < 1e-12);
        let again = with_angle(a, b, angle_of(a, b));
        assert!((again - b).length() < 1e-9);
        // 2 点が同じ所でも、向きを決められる
        assert!((with_angle(a, a, 90.0) - DVec2::new(10.0, 11.0)).length() < 1e-9);
    }
}
