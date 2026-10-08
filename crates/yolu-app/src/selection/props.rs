//! 選択範囲と対称の、オプションバーとツールプロパティの部品。
//! - 選択のツールのオプションバー: 作成方法（新規・追加・削除・共通。選択ペンは選択ペン・選択消し）、選択ペンの直径、自動選択の許容値
//! - ブラシ・消しゴムのオプションバーの右端: 対称の切り替えとモードの選び（▾）
//! - 左のドックのツールプロパティ: 選択のツールの作成方法・すべて・解除・反転・クイックマスク、ツールごとの設定（自動選択の許容値・隣接・全レイヤー、
//!   形のツールのアンチエイリアス・縦横比・中心から・角の丸め、選択ペンの直径・硬さ・不透明度）、選択範囲を変更。対称の欄は、ブラシの詳細のウィンドウの
//!   「対称」のカテゴリ（`symmetry_fields`）
//!
//! 値は画面の状態を直に、文書を変えるものは `Action::Sel` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::symmetry::{axis_name, mode_name, mode_tooltip, AXES_3D, MODES};
use super::{combine_tooltip, ModifyKind, SelAction, SelEdit, SymOp};
use crate::engine::{BrushEffect, SelectionCombine, SymmetryMode, MAX_MODIFY_RADIUS};
use crate::lang::Lang;
use crate::panels::properties::{group_label, slider_row, status_row, toggle_row};
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

/// 作成方法（新規・追加・削除・共通）の並び。
pub const CREATION_MODES: [SelectionCombine; 4] = [
    SelectionCombine::Replace,
    SelectionCombine::Add,
    SelectionCombine::Subtract,
    SelectionCombine::Intersect,
];

/// 作成方法のアイコンの組（CLIP STUDIO の「作成方法」）。選んでいる作成方法が点き、Shift・Ctrl・Shift+Ctrl を押しているあいだは、
/// 実際に効く作成方法が一時的に点く（選んでいるものは枠だけになる）。置いた幅だけ進めた x を返す。
fn creation_group(
    ui: &mut Ui,
    app: &mut AppState,
    y: f32,
    h: f32,
    x: f32,
    held: egui::Modifiers,
) -> f32 {
    let l = app.lang;
    let effective = super::combine_of(app.sel.combine, held);
    let width = CREATION_MODES.len() as f32 * 28.0 + 4.0;
    let group = Rect::from_min_size(pos2(x, y), vec2(width, h));
    w::rounded(ui.painter(), group, t::CONTROL_BG, 5.0);
    let mut bx = x + 2.0;
    for mode in CREATION_MODES {
        let at = Rect::from_min_size(pos2(bx, y), vec2(28.0, h));
        let lit = effective == mode;
        if w::icon_button(
            ui,
            at,
            ("options.select.mode", mode),
            super::saved::creation_icon(mode),
            combine_tooltip(l, mode),
            lit,
            true,
            18.0,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::Combine(mode))));
        }
        // 修飾キーで一時的に替わっているあいだ、選んでいる作成方法は枠だけで残す
        if app.sel.combine == mode && !lit {
            w::outline(ui.painter(), at.shrink(1.0), t::ACCENT, 1.0, 4.0);
        }
        bx += 28.0;
    }
    x + width
}

/// 選択ペン・選択消しの切り替え（選択ペンのツールのオプションバー）。Shift は選択ペン・Ctrl は選択消しに、押しているあいだ替える。
fn pen_group(
    ui: &mut Ui,
    app: &mut AppState,
    y: f32,
    h: f32,
    x: f32,
    held: egui::Modifiers,
) -> f32 {
    let l = app.lang;
    let erasing = super::pen::erases(app.sel.pen_erase, held);
    let items = [
        (
            false,
            "edit",
            l.pick(
                "選択ペン: 選択範囲に追加（Shift）",
                "Selection Pen: add to the selection (Shift)",
            ),
        ),
        (
            true,
            "tools/eraser",
            l.pick(
                "選択消し: 選択範囲から消す（Ctrl）",
                "Selection Eraser: remove from the selection (Ctrl)",
            ),
        ),
    ];
    let width = items.len() as f32 * 28.0 + 4.0;
    let group = Rect::from_min_size(pos2(x, y), vec2(width, h));
    w::rounded(ui.painter(), group, t::CONTROL_BG, 5.0);
    let mut bx = x + 2.0;
    for (erase, icon, tip) in items {
        let at = Rect::from_min_size(pos2(bx, y), vec2(28.0, h));
        let lit = erasing == erase;
        if w::icon_button(
            ui,
            at,
            ("options.select.pen", erase),
            icon,
            tip,
            lit,
            true,
            18.0,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::PenErase(erase))));
        }
        if app.sel.pen_erase == erase && !lit {
            w::outline(ui.painter(), at.shrink(1.0), t::ACCENT, 1.0, 4.0);
        }
        bx += 28.0;
    }
    x + width
}

/// 選択のツールのオプションバーの中身。`x` は次の部品を置く左端（ツールのアイコンと区切りの右）。作成方法（選択ペンは選択ペンと選択消し）に、
/// 選択ペンは直径、自動選択は許容値。すべて・解除・反転・クイックマスクと、ツールごとのほかの設定はツールプロパティ。
pub fn select_options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let mut x = x + 4.0;
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let l = app.lang;
    let p = ui.painter().clone();
    let held = ui.input(|i| i.modifiers);
    x = if app.tool == Tool::SelectPen {
        pen_group(ui, app, y, h, x, held)
    } else {
        creation_group(ui, app, y, h, x, held)
    };
    // ウィンドウが狭いときは、入りきらない部品を出さない
    let fits = |x: f32, width: f32| x + width <= r.right() - 8.0;
    if app.tool == Tool::SelectPen {
        x += 8.0;
        w::vline(&p, x, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
        x += 8.0;
        if !fits(x, 150.0) {
            return;
        }
        let at = Rect::from_min_size(pos2(x, y), vec2(150.0, h));
        let b = &mut app.brush;
        let out = w::slider(
            ui,
            at,
            "options.sel-pen.size",
            b.radius * 2.0,
            &SliderSpec::new(l.pick("直径", "Size"), 1.0, 256.0, NumberFormat::int(" px")).tooltip(
                l.pick(
                    "ブラシの直径（[ と ]）。ブラシと共通",
                    "Brush diameter ([ and ]), shared with the brush",
                ),
            ),
        );
        if out.changed {
            b.radius = (out.value / 2.0).max(0.5);
        }
    }
    if app.tool == Tool::Wand {
        x += 8.0;
        w::vline(&p, x, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
        x += 8.0;
        if !fits(x, 170.0) {
            return;
        }
        let at = Rect::from_min_size(pos2(x, y), vec2(170.0, h));
        let out = w::slider(
            ui,
            at,
            "options.wand.tolerance",
            app.sel.tolerance as f32,
            &wand_tolerance_spec(l),
        );
        if out.changed {
            app.sel.tolerance = out.value.round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn wand_tolerance_spec(l: Lang) -> SliderSpec<'static> {
    SliderSpec::new(
        l.pick("許容値", "Tolerance"),
        0.0,
        255.0,
        NumberFormat::int(""),
    )
    .tooltip(l.pick(
        "種の色から、各成分（RGBA）の差がこの値以下の画素を選ぶ",
        "Selects pixels whose every RGBA component is within this distance of the clicked color",
    ))
}

/// ブラシ・消しゴムのオプションバーの右端に、対称の切り替えとモードの選び（▾）。左の部品が使える右端（`left`）より狭ければ何も出さない。
pub fn symmetry_options(ui: &mut Ui, app: &mut AppState, r: Rect, left: f32) {
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let l = app.lang;
    let on = app.sel.symmetry.enabled();
    let p = ui.painter().clone();
    let mode_text = if on {
        Some(mode_name(l, app.sel.symmetry.mode))
    } else {
        None
    };
    let text_w = mode_text.map_or(0.0, |t| w::text_width(&p, t, t::LABEL) + 8.0);
    // モード名が入らないほど狭い（ウィンドウの最小の幅で、英語の長い名前）ときは、名前を落としてトグルと ▾ を残す
    let fixed = 28.0 + 20.0 + 14.0;
    let (mode_text, text_w) = if r.right() - 8.0 - (fixed + text_w) >= left {
        (mode_text, text_w)
    } else {
        (None, 0.0)
    };
    let total = fixed + text_w;
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

/// 作成方法の 1 行（新規・追加・削除・共通のアイコン。選んでいるのが点く。バーの作成方法と同じ値）。
pub fn creation_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let l = app.lang;
    let row = rows.row(24.0, 4.0);
    let held = ui.input(|i| i.modifiers);
    let effective = super::combine_of(app.sel.combine, held);
    for (i, mode) in CREATION_MODES.into_iter().enumerate() {
        let at = Rect::from_min_size(
            pos2(row.left() + 30.0 * i as f32, row.top()),
            vec2(28.0, row.height()),
        );
        let lit = effective == mode;
        if w::icon_button(
            ui,
            at,
            ("props.select.mode", mode),
            super::saved::creation_icon(mode),
            combine_tooltip(l, mode),
            lit,
            true,
            18.0,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::Combine(mode))));
        }
        if app.sel.combine == mode && !lit {
            w::outline(ui.painter(), at.shrink(1.0), t::ACCENT, 1.0, 4.0);
        }
    }
}

/// 選択ペン・選択消しの 1 行（バーと同じ値）。
fn pen_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let l = app.lang;
    let row = rows.row(24.0, 4.0);
    let held = ui.input(|i| i.modifiers);
    let erasing = super::pen::erases(app.sel.pen_erase, held);
    let items = [
        (
            false,
            "edit",
            l.pick(
                "選択ペン: 選択範囲に追加（Shift）",
                "Selection Pen: add to the selection (Shift)",
            ),
        ),
        (
            true,
            "tools/eraser",
            l.pick(
                "選択消し: 選択範囲から消す（Ctrl）",
                "Selection Eraser: remove from the selection (Ctrl)",
            ),
        ),
    ];
    for (i, (erase, icon, tip)) in items.into_iter().enumerate() {
        let at = Rect::from_min_size(
            pos2(row.left() + 30.0 * i as f32, row.top()),
            vec2(28.0, row.height()),
        );
        let lit = erasing == erase;
        if w::icon_button(
            ui,
            at,
            ("props.select.pen", erase),
            icon,
            tip,
            lit,
            true,
            18.0,
        )
        .clicked()
        {
            app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::PenErase(erase))));
        }
        if app.sel.pen_erase == erase && !lit {
            w::outline(ui.painter(), at.shrink(1.0), t::ACCENT, 1.0, 4.0);
        }
    }
}

/// すべて・解除・反転・クイックマスクの 1 行。
fn operations_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let l = app.lang;
    let row = rows.row(24.0, 4.0);
    let free = !app.is_stroking() && app.read_only_reason().is_none();
    let any = app.doc.selection().is_some();
    let buttons: [(&str, &str, SelEdit, bool); 3] = [
        (
            "select_all",
            l.pick("すべてを選択", "Select All"),
            SelEdit::All,
            free,
        ),
        (
            "deselect",
            l.pick("選択を解除", "Deselect"),
            SelEdit::Clear,
            free && any,
        ),
        (
            "invert_colors",
            l.pick("選択範囲を反転", "Invert Selection"),
            SelEdit::Invert,
            free && any,
        ),
    ];
    let mut x = row.left();
    for (icon, name, edit, enabled) in buttons {
        let at = Rect::from_min_size(pos2(x, row.top()), vec2(28.0, row.height()));
        // キーは割り当ての表から（文字を直に書かない）
        let action = Action::Sel(SelAction::Edit(edit));
        let tip = crate::shortcuts::tip_with_key(l, name, &action);
        if w::icon_button(
            ui,
            at,
            ("props.select.op", icon),
            icon,
            &tip,
            false,
            enabled,
            18.0,
        )
        .clicked()
        {
            app.apply(action);
        }
        x += 30.0;
    }
    // クイックマスク（入っているあいだ点く）
    let at = Rect::from_min_size(pos2(x, row.top()), vec2(28.0, row.height()));
    if w::icon_button(
        ui,
        at,
        "props.select.quick-mask",
        "quick_mask",
        &crate::shortcuts::tip_with_key(
            l,
            l.pick("クイックマスク", "Quick Mask"),
            &Action::Sel(SelAction::Ui(super::SelUiOp::QuickMask(None))),
        ),
        app.sel.quick,
        !app.is_stroking(),
        18.0,
    )
    .clicked()
    {
        app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::QuickMask(None))));
    }
}

/// ツールプロパティの中身（選択のツールのもの。ID の色で選択は範囲のツールの欄 `region_props`）。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    if app.tool == Tool::SelectPen {
        pen_row(ui, app, rows);
    } else {
        creation_row(ui, app, rows);
    }
    operations_row(ui, app, rows);
    tool_settings(ui, app, rows);
    modify_selection(ui, app, rows);
}

/// 選択範囲を変更（拡張・縮小・境界線・ぼかしなどの半径と実行）。
fn modify_selection(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("選択範囲を変更", "Modify Selection"));
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
                "選択範囲がキャンバスの外へ続くものとして扱う（縮小・境界線・ぼかしがキャンバスの端から離れない）",
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
            tooltip: if any {
                kind.tooltip(lang)
            } else {
                no_selection
            },
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

/// ツールごとの設定（自動選択: 許容値・隣接・全レイヤー。形のツール: アンチエイリアス・縦横比・中心から・角の丸め。選択ペン: 直径・硬さ・不透明度）。
fn tool_settings(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let tool = app.tool;
    if tool == Tool::Wand {
        let at = rows.slider_row();
        let out = w::slider(
            ui,
            at,
            "props.wand.tolerance",
            app.sel.tolerance as f32,
            &wand_tolerance_spec(lang),
        );
        if out.changed {
            app.sel.tolerance = out.value.round().clamp(0.0, 255.0) as u8;
        }
        if let Some(v) = toggle_row(
            ui,
            rows,
            "props.wand.contiguous",
            lang.pick("隣接", "Contiguous"),
            app.sel.contiguous,
            Some(lang.pick(
                "種からつながる所だけを選ぶ（切ると、キャンバス全体の合う画素）",
                "Only pixels connected to the click (off: every matching pixel)",
            )),
            true,
        ) {
            app.sel.contiguous = v;
        }
        if let Some(v) = toggle_row(
            ui,
            rows,
            "props.wand.all-layers",
            lang.pick("全レイヤーを対象", "Sample All Layers"),
            app.sel.all_layers,
            Some(lang.pick(
                "選んだレイヤーでなく、チャンネルの合成から選ぶ",
                "Use the composite instead of the selected layer",
            )),
            true,
        ) {
            app.sel.all_layers = v;
        }
        return;
    }
    if tool == Tool::SelectPen {
        let shared = lang.pick("ブラシと共通", "Shared with the brush");
        if let Some(v) = slider_row(
            ui,
            rows,
            "sel.pen.size",
            lang.pick("直径", "Size"),
            app.brush.radius * 2.0,
            (1.0, 256.0),
            NumberFormat::int(" px"),
            Some(shared),
            true,
        ) {
            app.brush.radius = (v / 2.0).max(0.5);
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "sel.pen.hardness",
            lang.pick("硬さ", "Hardness"),
            app.brush.hardness * 100.0,
            (0.0, 100.0),
            NumberFormat::int("%"),
            Some(shared),
            true,
        ) {
            app.brush.hardness = v / 100.0;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "sel.pen.opacity",
            lang.pick("不透明度", "Opacity"),
            app.brush.opacity * 100.0,
            (0.0, 100.0),
            NumberFormat::int("%"),
            Some(lang.pick(
                "足す量の上限（ブラシと共通）",
                "Largest amount a stroke adds (shared with the brush)",
            )),
            true,
        ) {
            app.brush.opacity = v / 100.0;
        }
        return;
    }
    if !matches!(
        tool,
        Tool::SelectRect | Tool::SelectEllipse | Tool::Lasso | Tool::Polygon
    ) {
        return;
    }
    let rect = tool == Tool::SelectRect;
    let shaped = matches!(tool, Tool::SelectRect | Tool::SelectEllipse);
    // 長方形は角を丸めたときだけ縁が滑らかになる
    let aa_applies = !rect || app.sel.corner_radius > 0;
    if let Some(v) = toggle_row(
        ui,
        rows,
        "sel.antialias",
        lang.pick("アンチエイリアス", "Anti-alias"),
        app.sel.antialias,
        Some(if aa_applies {
            lang.pick(
                "縁を滑らかにする（切ると、縁の量は 0 か 255 だけ）",
                "Smooth the edge (off: the edge is all or nothing)",
            )
        } else {
            lang.pick("角が丸いときに効く", "Applies to rounded corners")
        }),
        aa_applies,
    ) {
        app.sel.antialias = v;
    }
    if shaped {
        if let Some(v) = toggle_row(
            ui,
            rows,
            "sel.fixed-ratio",
            lang.pick("縦横比を固定", "Fixed ratio"),
            app.sel.fixed_ratio,
            Some(lang.pick(
                "常に正方形・正円にする（切っていても、押し始めたあとに Shift を押すあいだは固定）",
                "Always a square or circle (with it off, Shift pressed after starting holds the ratio)",
            )),
            true,
        ) {
            app.sel.fixed_ratio = v;
        }
        if let Some(v) = toggle_row(
            ui,
            rows,
            "sel.from-center",
            lang.pick("中心から", "From center"),
            app.sel.from_center,
            Some(lang.pick(
                "押した点を中心に広げる（切っていても、ドラッグを始めたあとに Alt を押すあいだは中心から）",
                "Grow around the pressed point (with it off, pressing Alt after you start dragging does the same)",
            )),
            true,
        ) {
            app.sel.from_center = v;
        }
    }
    if rect {
        if let Some(v) = slider_row(
            ui,
            rows,
            "sel.corner-radius",
            lang.pick("角の丸め", "Corner radius"),
            app.sel.corner_radius as f32,
            (0.0, 512.0),
            NumberFormat::int(" px"),
            Some(lang.pick(
                "長方形の角の半径（短い辺の半分まで）",
                "Radius of the rectangle's corners (up to half the short side)",
            )),
            true,
        ) {
            app.sel.corner_radius = v.round().clamp(0.0, 512.0) as u32;
        }
    }
}

/// ブラシの詳細のウィンドウの「対称」の欄（見出しと既定に戻すはウィンドウが出す）。2D のキャンバスの対称と、3D の面の対称（3D のビューを出しているとき）。
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
        "軸の通る点（キャンバスの座標）",
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
