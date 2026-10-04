//! 選択範囲と対称の、オプションバーとプロパティの欄の部品。
//! - 選択の道具のオプションバー: 組み合わせ方（置き換え・足す・引く・重ねる）、すべて・解除・反転、自動選択の許容値・隣接・全レイヤー
//! - ブラシ・消しゴムのオプションバーの右端: 対称の切り替えとモードの選び（▾）
//! - プロパティの欄: 選択の道具では「選択範囲を変更」。対称の欄は、ブラシの詳細の窓の「対称」のカテゴリ（`symmetry_fields`）
//!
//! 値は画面の状態を直に、文書を変えるものは `Action::Sel` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::symmetry::{axis_name, mode_name, mode_tooltip, AXES_3D, MODES};
use super::{combine_name, combine_tooltip, ModifyKind, SelAction, SelEdit, SymOp};
use crate::engine::{BrushEffect, SelectionCombine, SymmetryMode, MAX_MODIFY_RADIUS};
use crate::lang::Lang;
use crate::panels::properties::{group_label, section, slider_row, status_row, toggle_row};
use crate::state::{Action, AppState, OpenPopup, PopupKind, Tool};
use crate::ui::menu::PopupState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};
use yolu_core::geometry::SymmetryAxis;

/// ボタンの列の 1 つ: 名前・選んでいる（青）か・押せるか・ツールチップ。
struct FlowButton<'a> {
    label: &'a str,
    primary: bool,
    enabled: bool,
    tooltip: &'a str,
}

/// ボタンを名前の幅に合わせて左から並べ、収まらなければ次の行へ回す（名前を切らない）。各行の余りは行のボタンに均等に配る。
/// 押されたボタンの番号を返す。
fn flow_buttons(ui: &mut Ui, rows: &mut Rows, salt: &str, items: &[FlowButton]) -> Option<usize> {
    const GAP: f32 = 4.0;
    let first = rows.row(24.0, 4.0);
    let painter = ui.painter().clone();
    let widths: Vec<f32> = items
        .iter()
        .map(|b| w::text_width(&painter, b.label, t::LABEL) + 22.0)
        .collect();
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0.0;
    for (i, width) in widths.iter().enumerate() {
        let line = lines.last_mut().expect("最初の行がある");
        let needed = if line.is_empty() {
            *width
        } else {
            used + GAP + *width
        };
        if needed > first.width() && !line.is_empty() {
            lines.push(vec![i]);
            used = *width;
        } else {
            line.push(i);
            used = needed;
        }
    }
    let mut clicked = None;
    for (n, line) in lines.iter().enumerate() {
        let row = if n == 0 { first } else { rows.row(24.0, 4.0) };
        let total: f32 =
            line.iter().map(|&i| widths[i]).sum::<f32>() + GAP * (line.len() - 1) as f32;
        let extra = ((row.width() - total) / line.len() as f32).max(0.0);
        let mut x = row.left();
        for &i in line {
            let width = widths[i] + extra;
            let at = Rect::from_min_size(pos2(x, row.top()), vec2(width, row.height()));
            let b = &items[i];
            if w::button(
                ui,
                at,
                (salt, i),
                b.label,
                b.primary,
                b.enabled,
                Some(b.tooltip),
                None,
            )
            .clicked()
            {
                clicked = Some(i);
            }
            x += width + GAP;
        }
    }
    clicked
}

/// 選択の道具のオプションバーの中身。`x` は次の部品を置く左端（道具のアイコンと区切りの右）。
pub fn select_options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let l = app.lang;
    let p = ui.painter().clone();
    for mode in [
        SelectionCombine::Replace,
        SelectionCombine::Add,
        SelectionCombine::Subtract,
        SelectionCombine::Intersect,
    ] {
        let name = combine_name(l, mode);
        let width = w::text_width(&p, name, t::LABEL) + 22.0;
        let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
        if w::button(
            ui,
            at,
            ("options.select.mode", mode),
            name,
            app.sel.combine == mode,
            true,
            Some(combine_tooltip(l, mode)),
            None,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::Combine(mode))));
        }
        x += width + 4.0;
    }
    x += 4.0;
    w::vline(&p, x, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
    x += 8.0;
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let any = app.doc.selection().is_some();
    let buttons: [(&str, &str, SelEdit, bool); 3] = [
        (
            "select_all",
            l.pick("すべてを選択（Ctrl+A）", "Select All (Ctrl+A)"),
            SelEdit::All,
            free,
        ),
        (
            "deselect",
            l.pick("選択を解除（Ctrl+D）", "Deselect (Ctrl+D)"),
            SelEdit::Clear,
            free && any,
        ),
        (
            "invert_colors",
            l.pick(
                "選択範囲を反転（Ctrl+Shift+I）",
                "Invert Selection (Ctrl+Shift+I)",
            ),
            SelEdit::Invert,
            free && any,
        ),
    ];
    for (icon, tip, edit, enabled) in buttons {
        let at = Rect::from_min_size(pos2(x, y), vec2(28.0, h));
        if w::icon_button(
            ui,
            at,
            ("options.select.op", icon),
            icon,
            tip,
            false,
            enabled,
            20.0,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Edit(edit)));
        }
        x += 32.0;
    }
    if app.tool == Tool::Wand {
        x += 4.0;
        w::vline(&p, x, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
        x += 8.0;
        // 窓が狭いときは、入りきらない部品を出さない
        let fits = |x: f32, width: f32| x + width <= r.right() - 8.0;
        if !fits(x, 170.0) {
            return;
        }
        let at = Rect::from_min_size(pos2(x, y), vec2(170.0, h));
        let out = w::slider(
            ui,
            at,
            "options.wand.tolerance",
            app.sel.tolerance as f32,
            &SliderSpec::new(
                l.pick("許容値", "Tolerance"),
                0.0,
                255.0,
                NumberFormat::int(""),
            )
            .tooltip(l.pick(
                "種の色から、各成分（RGBA）の差がこの値以下の画素を選ぶ",
                "Selects pixels whose every RGBA component is within this distance of the clicked color",
            )),
        );
        if out.changed {
            app.sel.tolerance = out.value.round().clamp(0.0, 255.0) as u8;
        }
        x += 170.0 + 10.0;
        if !fits(x, 90.0) {
            return;
        }
        let at = Rect::from_min_size(pos2(x, y), vec2(90.0, h));
        app.sel.contiguous = w::toggle(
            ui,
            at,
            "options.wand.contiguous",
            l.pick("隣接", "Contiguous"),
            app.sel.contiguous,
            Some(l.pick(
                "種からつながる所だけを選ぶ（切ると、キャンバス全体の合う画素）",
                "Only pixels connected to the click (off: every matching pixel)",
            )),
            true,
        );
        x += 90.0 + 10.0;
        if !fits(x, 140.0) {
            return;
        }
        let at = Rect::from_min_size(pos2(x, y), vec2(140.0, h));
        app.sel.all_layers = w::toggle(
            ui,
            at,
            "options.wand.all-layers",
            l.pick("全レイヤーを対象", "Sample All Layers"),
            app.sel.all_layers,
            Some(l.pick(
                "選んだレイヤーでなく、チャンネルの合成から選ぶ",
                "Use the composite instead of the selected layer",
            )),
            true,
        );
    }
}

/// ブラシ・消しゴムのオプションバーの右端に、対称の切り替えとモードの選び（▾）。左の部品が使える右端（`left`）より狭ければ何も出さない。
pub fn symmetry_options(ui: &mut Ui, app: &mut AppState, r: Rect, left: f32) {
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let l = app.lang;
    let on = app.sel.symmetry.enabled();
    let mode_text = if on {
        Some(mode_name(l, app.sel.symmetry.mode))
    } else {
        None
    };
    let p = ui.painter().clone();
    let text_w = mode_text.map_or(0.0, |t| w::text_width(&p, t, t::LABEL) + 8.0);
    let total = 28.0 + 20.0 + text_w + 14.0;
    let start = r.right() - 8.0 - total;
    if start < left {
        return;
    }
    w::vline(
        &p,
        start - 6.0,
        r.top() + 6.0,
        r.bottom() - 6.0,
        t::SEPARATOR,
    );
    let mut x = start;
    if let Some(text) = mode_text {
        w::text(
            &p,
            Rect::from_min_size(pos2(x, r.top()), vec2(text_w, r.height())),
            text,
            t::LABEL.with_color(t::ACCENT),
            w::Align::Left,
        );
        x += text_w;
    }
    let toggle = Rect::from_min_size(pos2(x, y), vec2(28.0, h));
    if w::icon_button(
        ui,
        toggle,
        "options.symmetry",
        "flip",
        l.pick("対称のオン・オフ（2D）", "Symmetry On/Off (2D)"),
        on,
        true,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::Toggle)));
    }
    x += 30.0;
    let more = Rect::from_min_size(pos2(x, y), vec2(18.0, h));
    if w::icon_button(
        ui,
        more,
        "options.symmetry.more",
        "arrow_drop_down",
        l.pick("対称のモード", "Symmetry mode"),
        false,
        true,
        16.0,
    )
    .clicked()
    {
        let anchor = Rect::from_min_size(
            pos2(more.left(), more.bottom() + 4.0),
            vec2(more.width(), 0.0),
        );
        app.popup = Some(OpenPopup {
            kind: PopupKind::Symmetry,
            state: PopupState::new(ui.ctx(), anchor),
        });
    }
}

/// プロパティの欄の選択の道具の中身（選択範囲を変更）。
pub fn selection_body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "selection-modify",
        lang.pick("選択範囲を変更", "Modify Selection"),
        "select_all",
        None,
    );
    if !open {
        return;
    }
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let any = app.doc.selection().is_some();
    // 選択範囲が無いあいだは欄を無効にして、理由をツールチップに出す（注記の行は置かない）
    let no_selection = lang.pick("選択範囲なし", "No selection");
    if let Some(v) = slider_row(
        ui,
        rows,
        "sel.radius",
        lang.pick("半径", "Radius"),
        app.sel.radius as f32,
        (0.0, MAX_MODIFY_RADIUS as f32),
        NumberFormat::int(" px"),
        Some(if any {
            lang.pick(
                "拡張・縮小・境界線・ぼかしの半径",
                "Radius for Grow, Shrink, Border and Feather",
            )
        } else {
            no_selection
        }),
        any,
    ) {
        app.sel.radius = v.round().clamp(0.0, MAX_MODIFY_RADIUS as f32) as u32;
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "sel.edge-lock",
        lang.pick("端を固定", "Edge lock"),
        app.sel.edge_lock,
        Some(if any {
            lang.pick(
                "選択範囲が画布の外へ続くものとして扱う（縮小・境界線・ぼかしが画布の端から離れない）",
                "Treat the selection as continuing past the canvas edge (Shrink, Border and Feather do not pull away from it)",
            )
        } else {
            no_selection
        }),
        any,
    ) {
        app.sel.edge_lock = v;
    }
    let items: Vec<FlowButton> = ModifyKind::ALL
        .iter()
        .map(|kind| FlowButton {
            label: kind.name(lang),
            primary: false,
            enabled: any && free,
            tooltip: if any { kind.tooltip(lang) } else { no_selection },
        })
        .collect();
    if let Some(i) = flow_buttons(ui, rows, "sel.modify", &items) {
        app.apply(Action::Sel(SelAction::Edit(SelEdit::Modify {
            kind: ModifyKind::ALL[i],
            radius: app.sel.radius,
            edge_lock: app.sel.edge_lock,
        })));
    }
}

/// ブラシの詳細の窓の「対称」の欄（見出しと既定に戻すは窓が出す）。2D のキャンバスの対称と、3D の面の対称（3D のビューを出しているとき）。
/// 指先・クローンは対称と組めないので、対称のモードは「なし」のほかを無効にする。
pub fn symmetry_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    rows.indent = 0.0;
    let free = !app.is_stroking();
    // 2D のキャンバスと 3D のビューを並べて見ているときは、どちらの設定も出す（別々に効く）
    let both = app.view3d.paintable_on_screen();
    if both {
        group_label(ui, rows, "2D");
    }
    symmetry_2d(ui, app, rows, lang, free);
    if both {
        group_label(ui, rows, "3D");
        symmetry_3d(ui, app, rows, lang, free);
    }
}

fn symmetry_2d(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang, free: bool) {
    let current = app.sel.symmetry.mode;
    // モードのボタン
    let items: Vec<FlowButton> = MODES
        .iter()
        .map(|mode| {
            // モードは指先・クローンのあいだも替えられる（ペイントに戻したときに効く設定を用意できる）。効かない欄は下で無効にする
            FlowButton {
                label: mode_name(lang, *mode),
                primary: current == *mode,
                enabled: free,
                tooltip: mode_tooltip(lang, *mode),
            }
        })
        .collect();
    if let Some(i) = flow_buttons(ui, rows, "symmetry.mode", &items) {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::Mode(MODES[i]))));
    }
    if current == SymmetryMode::None {
        return;
    }
    // 効かない欄は注記の行を置かず、無効（灰色）にして理由をツールチップに出す。
    // 3D の面のストロークは対称を見ない（core に 3D の対称は無い）ので、描ける先が 3D だけのあいだだけ無効にする（ドックを分けて
    // キャンバスも出ているあいだは、2D に描く対称の設定を残す）。指先・クローンも対称を見ない
    let reason: Option<&str> = if app.paints_only_in_3d() {
        Some(lang.pick("3D では効きません", "No effect in 3D"))
    } else if matches!(
        app.m2.brush.effect,
        BrushEffect::Smudge { .. } | BrushEffect::Clone { .. }
    ) {
        Some(lang.pick("指先・クローンでは使えません", "Not with smudge or clone"))
    } else {
        None
    };
    let live = free && reason.is_none();
    let (width, height) = (app.doc.width() as f64, app.doc.height() as f64);
    let (cx, cy) = app.sel.symmetry.center;
    let centered = NumberFormat {
        decimals: 1,
        trim: true,
        suffix: " px",
    };
    let tip = reason.unwrap_or(lang.pick(
        "軸の通る点（画布の座標）",
        "Where the axes cross (canvas pixels)",
    ));
    let nx = slider_row(
        ui,
        rows,
        "symmetry.cx",
        lang.pick("中心 X", "Center X"),
        (cx * width) as f32,
        (0.0, width as f32),
        centered,
        Some(tip),
        live,
    );
    let ny = slider_row(
        ui,
        rows,
        "symmetry.cy",
        lang.pick("中心 Y", "Center Y"),
        (cy * height) as f32,
        (0.0, height as f32),
        centered,
        Some(tip),
        live,
    );
    if nx.is_some() || ny.is_some() {
        let x = nx.map_or(cx, |v| v as f64 / width);
        let y = ny.map_or(cy, |v| v as f64 / height);
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::Center(x, y))));
    }
    if current == SymmetryMode::Radial {
        if let Some(v) = slider_row(
            ui,
            rows,
            "symmetry.count",
            lang.pick("写しの数", "Copies"),
            app.sel.symmetry.count as f32,
            (2.0, 16.0),
            NumberFormat::int(""),
            Some(reason.unwrap_or(lang.pick("中心のまわりの写しの数", "Copies around the center"))),
            live,
        ) {
            app.apply(Action::Sel(SelAction::Symmetry(SymOp::Count(
                v.round() as u32
            ))));
        }
    }
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "symmetry.center-canvas",
        lang.pick("キャンバスの中心", "Canvas center"),
        false,
        live && app.sel.symmetry.center != (0.5, 0.5),
        reason,
        None,
    )
    .clicked()
    {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::CenterCanvas)));
    }
    if let Some(v) = toggle_row(
        ui,
        rows,
        "symmetry.axes",
        lang.pick("軸を表示", "Show axes"),
        app.sel.symmetry.show_axes,
        reason,
        reason.is_none(),
    ) {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::ShowAxes(v))));
    }
}

/// 軸のボタンのツールチップ（ミラーは面の向き、放射状は回す軸）。
fn axis_tooltip(lang: Lang, axis: SymmetryAxis, radial: bool) -> String {
    let name = axis_name(axis);
    if radial {
        lang.pick(
            format!("モデルの {name} 軸のまわりに回して写す"),
            format!("Rotate copies around the model's {name} axis"),
        )
    } else {
        lang.pick(
            format!("面はモデルの {name} 軸に直交する"),
            format!("The plane is perpendicular to the model's {name} axis"),
        )
    }
}

/// 軸の選びのボタンの列（選んでいる軸を返す）。
fn axis_buttons(
    ui: &mut Ui,
    rows: &mut Rows,
    lang: Lang,
    salt: &str,
    current: SymmetryAxis,
    radial: bool,
    enabled: bool,
) -> Option<SymmetryAxis> {
    let tips: Vec<String> = AXES_3D
        .iter()
        .map(|a| axis_tooltip(lang, *a, radial))
        .collect();
    let items: Vec<FlowButton> = AXES_3D
        .iter()
        .zip(&tips)
        .map(|(a, tip)| FlowButton {
            label: axis_name(*a),
            primary: current == *a,
            enabled,
            tooltip: tip,
        })
        .collect();
    flow_buttons(ui, rows, salt, &items).map(|i| AXES_3D[i])
}

fn symmetry_3d(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang, free: bool) {
    let s = app.sel.symmetry.surface.clone();
    let send = |app: &mut AppState, op: SymOp| app.apply(Action::Sel(SelAction::Symmetry(op)));
    // ミラー
    if let Some(v) = toggle_row(
        ui,
        rows,
        "symmetry3d.mirror",
        lang.pick("ミラー", "Mirror"),
        s.mirror,
        Some(lang.pick(
            "モデルの軸に直交する面で左右に写す（今のテクスチャセットの面だけ）",
            "Reflect dabs across a plane perpendicular to a model axis, within the active texture set",
        )),
        free,
    ) {
        send(app, SymOp::Mirror3d(v));
    }
    if s.mirror {
        if let Some(axis) = axis_buttons(ui, rows, lang, "symmetry3d.axis", s.axis, false, free) {
            send(app, SymOp::Axis3d(axis));
        }
        // ずれの範囲はモデルの大きさに合わせる（±1000 のままではスライダーが使えない）
        let reach = app
            .view3d
            .model
            .as_ref()
            .map_or(1.0, |m| m.geometry.bounds().size().length().max(1e-3))
            .max(s.offset.abs());
        if let Some(v) = slider_row(
            ui,
            rows,
            "symmetry3d.offset",
            lang.pick("中心", "Center"),
            s.offset,
            (-reach, reach),
            NumberFormat {
                decimals: 3,
                trim: true,
                suffix: "",
            },
            Some(lang.pick(
                "モデルの原点から軸の向きの距離（モデルの単位）",
                "Distance from the model origin along the axis (model units)",
            )),
            free,
        ) {
            send(app, SymOp::Offset3d(v));
        }
        let has_model = app.view3d.model.is_some();
        let items = [
            FlowButton {
                label: lang.pick("原点", "Origin"),
                primary: false,
                enabled: free && s.offset != 0.0,
                tooltip: lang.pick(
                    "面をモデルの原点に置く",
                    "Put the plane through the model origin",
                ),
            },
            FlowButton {
                label: lang.pick("境界の中心", "Bounds center"),
                primary: false,
                enabled: free && has_model,
                tooltip: lang.pick(
                    "面をモデルの境界の中央に置く",
                    "Put the plane through the middle of the model's bounds",
                ),
            },
        ];
        match flow_buttons(ui, rows, "symmetry3d.center", &items) {
            Some(0) => send(app, SymOp::OffsetOrigin3d),
            Some(_) => send(app, SymOp::OffsetBounds3d),
            None => {}
        }
    }
    // 放射状
    if let Some(v) = toggle_row(
        ui,
        rows,
        "symmetry3d.radial",
        lang.pick("放射状", "Radial"),
        s.radial,
        Some(lang.pick(
            "モデルの軸のまわりに回して写す（ミラーと一緒に使える）",
            "Rotate copies around a model axis (can be combined with the mirror)",
        )),
        free,
    ) {
        send(app, SymOp::Radial3d(v));
    }
    if s.radial {
        if let Some(axis) = axis_buttons(
            ui,
            rows,
            lang,
            "symmetry3d.radial-axis",
            s.radial_axis,
            true,
            free,
        ) {
            send(app, SymOp::RadialAxis3d(axis));
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "symmetry3d.count",
            lang.pick("写しの数", "Copies"),
            s.radial_count as f32,
            (2.0, 16.0),
            NumberFormat::int(""),
            Some(lang.pick("軸のまわりの写しの数", "Copies around the axis")),
            free,
        ) {
            send(app, SymOp::RadialCount3d(v.round() as u32));
        }
    }
    if s.enabled() {
        if let Some(v) = toggle_row(
            ui,
            rows,
            "symmetry3d.visibility",
            lang.pick("見えない面にも", "Ignore visibility"),
            s.ignore_visibility,
            Some(lang.pick(
                "写しは、カメラから見えない面・裏の面にも塗る。元のダブは見える面だけ",
                "Copies also paint back-facing and hidden surfaces. The original dab paints visible surfaces only",
            )),
            free,
        ) {
            send(app, SymOp::IgnoreVisibility3d(v));
        }
        if let Some(v) = toggle_row(
            ui,
            rows,
            "symmetry3d.plane",
            lang.pick("面を表示", "Show plane"),
            s.show_plane,
            Some(lang.pick(
                "3D ビューに対称の面と軸を出す",
                "Show the symmetry plane and axis in the 3D view",
            )),
            true,
        ) {
            send(app, SymOp::ShowPlane3d(v));
        }
        if matches!(
            app.m2.brush.effect,
            BrushEffect::Smudge { .. } | BrushEffect::Clone { .. }
        ) {
            status_row(
                ui,
                rows,
                lang.pick("指先・クローンでは使えません", "Not with smudge or clone"),
            );
        }
    }
}
