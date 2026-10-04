//! 選択範囲と対称の、オプションバーとプロパティの欄の部品。
//! - 選択の道具のオプションバー: 組み合わせ方（置き換え・足す・引く・重ねる）、すべて・解除・反転、自動選択の許容値・隣接・全レイヤー
//! - ブラシ・消しゴムのオプションバーの右端: 対称の切り替えとモードの選び（▾）
//! - プロパティの欄: 選択の道具では「選択範囲を変更」、描く道具では「対称」
//!
//! 値は画面の状態を直に、文書を変えるものは `Action::Sel` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::symmetry::{mode_name, mode_tooltip, MODES};
use super::{combine_name, combine_tooltip, ModifyKind, SelAction, SelEdit, SymOp};
use crate::engine::{BrushEffect, SelectionCombine, SymmetryMode, MAX_MODIFY_RADIUS};
use crate::lang::Lang;
use crate::panels::properties::{section, slider_row, status_row, toggle_row};
use crate::state::{Action, AppState, OpenPopup, PopupKind, Tool};
use crate::ui::menu::PopupState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

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
    if let Some(v) = slider_row(
        ui,
        rows,
        "sel.radius",
        lang.pick("半径", "Radius"),
        app.sel.radius as f32,
        (0.0, MAX_MODIFY_RADIUS as f32),
        NumberFormat::int(" px"),
        Some(lang.pick(
            "拡張・縮小・境界線・ぼかしの半径",
            "Radius for Grow, Shrink, Border and Feather",
        )),
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
        Some(lang.pick(
            "選択範囲が画布の外へ続くものとして扱う（縮小・境界線・ぼかしが画布の端から離れない）",
            "Treat the selection as continuing past the canvas edge (Shrink, Border and Feather do not pull away from it)",
        )),
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
            tooltip: kind.tooltip(lang),
        })
        .collect();
    if let Some(i) = flow_buttons(ui, rows, "sel.modify", &items) {
        app.apply(Action::Sel(SelAction::Edit(SelEdit::Modify {
            kind: ModifyKind::ALL[i],
            radius: app.sel.radius,
            edge_lock: app.sel.edge_lock,
        })));
    }
    if !any {
        status_row(ui, rows, lang.pick("選択範囲なし", "No selection"));
    }
}

/// プロパティの欄の「対称」（描く道具のタブの末尾）。
pub fn symmetry_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    rows.indent = 0.0;
    let (open, reset) = section(
        ui,
        app,
        rows,
        "brush-symmetry",
        lang.pick("対称", "Symmetry"),
        "flip",
        Some(lang.pick("対称を既定に戻す", "Reset symmetry")),
    );
    if reset {
        app.sel.symmetry = super::SymmetryState::default();
    }
    if !open {
        return;
    }
    let free = !app.is_stroking();
    let current = app.sel.symmetry.mode;
    // モードのボタン
    let items: Vec<FlowButton> = MODES
        .iter()
        .map(|mode| FlowButton {
            label: mode_name(lang, *mode),
            primary: current == *mode,
            enabled: free,
            tooltip: mode_tooltip(lang, *mode),
        })
        .collect();
    if let Some(i) = flow_buttons(ui, rows, "symmetry.mode", &items) {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::Mode(MODES[i]))));
    }
    if current == SymmetryMode::None {
        return;
    }
    // 3D の面のストロークは対称を見ない（core に 3D の対称は無い）
    if app.view3d.paintable_on_screen() {
        status_row(ui, rows, lang.pick("3D では効きません", "No effect in 3D"));
    }
    let (width, height) = (app.doc.width() as f64, app.doc.height() as f64);
    let (cx, cy) = app.sel.symmetry.center;
    let centered = NumberFormat {
        decimals: 1,
        trim: true,
        suffix: " px",
    };
    let tip = lang.pick(
        "軸の通る点（画布の座標）",
        "Where the axes cross (canvas pixels)",
    );
    let nx = slider_row(
        ui,
        rows,
        "symmetry.cx",
        lang.pick("中心 X", "Center X"),
        (cx * width) as f32,
        (0.0, width as f32),
        centered,
        Some(tip),
        free,
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
        free,
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
            Some(lang.pick("中心のまわりの写しの数", "Copies around the center")),
            free,
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
        free && app.sel.symmetry.center != (0.5, 0.5),
        None,
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
        None,
        true,
    ) {
        app.apply(Action::Sel(SelAction::Symmetry(SymOp::ShowAxes(v))));
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
