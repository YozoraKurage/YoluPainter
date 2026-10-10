//! 選択範囲の、オプションバーとツールプロパティの部品。
//! - 選択のツールのオプションバー: 作成方法（新規・追加・削除・共通。選択ペンは選択ペン・選択消し）、選択ペンの直径、自動選択の許容値
//! - 左のドックのツールプロパティ: 選択のツールの作成方法の帯（新規・追加・削除に、共通を出し入れする「⋯」。選択ペンは選択ペンと選択消し）、
//!   ツールごとの設定（自動選択の許容値・隣接・全レイヤー、形のツールのアンチエイリアス・縦横比・中心から・角の丸め、選択ペンの直径・硬さ・不透明度）。
//! - ツールプロパティに無い操作の置き場: すべてを選択・クイックマスクは「選択範囲」メニューとキー、選択を解除・選択範囲を反転はメニュー・キー・
//!   選択範囲の下のバー、拡張・縮小・境界をぼかすはメニューとバー、境界線・境界をくっきりはメニューだけ
//!
//! 値は画面の状態を直に、文書を変えるものは `Action::Sel` を通す（1 回の Undo）。画面には名前と値だけを出し、ツールチップは名前とキー（押せないときは短い理由）。

use egui::{pos2, vec2, Rect, Ui};

use super::{combine_tooltip, SelAction};
use crate::engine::SelectionCombine;
use crate::lang::Lang;
use crate::panels::properties::{slider_row, toggle_row};
use crate::state::{Action, AppState, Tool};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

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
    let effective = super::combine_of(app.sel.combine, egui::PointerButton::Primary, held);
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
            &combine_tooltip(l, mode),
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

/// 帯の中の選択ペン・選択消しの名前（英語は帯に入る短さ）。
fn pen_name(l: Lang, erase: bool) -> &'static str {
    if erase {
        l.pick("選択消し", "Eraser")
    } else {
        l.pick("選択ペン", "Pen")
    }
}

/// 選択ペン・選択消しのツールチップ（名前と、割り当ての表の修飾。修飾を読む `pen::erases` と同じ行）。
fn pen_tooltip(l: Lang, erase: bool) -> String {
    use crate::keymap::Operation;
    if erase {
        super::name_with_modifier(
            l,
            l.pick("選択消し", "Selection Eraser"),
            Some(Operation::SelectionSubtract),
        )
    } else {
        super::name_with_modifier(
            l,
            l.pick("選択ペン", "Selection Pen"),
            Some(Operation::SelectionAdd),
        )
    }
}

/// 選択ペン・選択消しのアイコン。
fn pen_icon(erase: bool) -> &'static str {
    if erase {
        "tools/eraser"
    } else {
        "edit"
    }
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
    let items = [false, true].map(|erase| (erase, pen_icon(erase), pen_tooltip(l, erase)));
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
            &tip,
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
/// 選択ペンは直径、自動選択は許容値。ツールごとのほかの設定はツールプロパティ。
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
            &SliderSpec::new(l.pick("直径", "Size"), 1.0, 256.0, NumberFormat::int(" px"))
                .tooltip(l.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])")),
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
}

/// 幅いっぱいの帯の 1 つ: 名前・アイコン・点いているか（青）・枠だけか（修飾キーで替わっている間の、選んでいる側）・ツールチップ。
struct Segment<'a> {
    label: &'a str,
    icon: &'a str,
    lit: bool,
    outlined: bool,
    tooltip: String,
}

/// 帯の右端の「⋯」（名前なしの細いボタン）: 点いているか・ツールチップ。
struct MoreButton<'a> {
    /// 試験・読み上げの名前（ツールチップに理由が付いても変わらない）。
    label: &'a str,
    lit: bool,
    /// 押せるか。押せないときも点きは変えず、押し込めない見た目にする。
    enabled: bool,
    tooltip: String,
}

/// 「⋯」の幅。
const MORE_WIDTH: f32 = 28.0;

/// 帯のボタン 1 つの幅: 行の幅から「⋯」の分（`reserved`）とボタンの間（`gap`）を引いて等分する（狭くても負にしない）。
fn segment_width(row_width: f32, reserved: f32, gap: f32, count: usize) -> f32 {
    ((row_width - reserved - gap * count.saturating_sub(1) as f32) / count.max(1) as f32).max(0.0)
}

/// 全部のボタンで「アイコン＋名前」が入るか（名前の幅 + アイコンと間の 22）。1 つでも入らなければ、全部アイコンだけにする。
fn names_fit(label_widths: &[f32], each: f32) -> bool {
    label_widths.iter().all(|width| width + 22.0 <= each)
}

/// 帯の 1 行（高さ 24）を幅いっぱいに、同じ幅のボタンで分ける（右端に「⋯」があれば先にその幅を除く）。アイコン＋名前が入らない幅では、
/// 全部のボタンを名前なしのアイコンだけにする（折り返さず、名前を「…」で詰めない）。押されたボタンの番号と、「⋯」が押されたかを返す。
fn segmented_row(
    ui: &mut Ui,
    rows: &mut Rows,
    salt: &str,
    items: &[Segment],
    more: Option<MoreButton>,
) -> (Option<usize>, bool) {
    const GAP: f32 = 4.0;
    let row = rows.row(24.0, 4.0);
    let reserved = if more.is_some() {
        MORE_WIDTH + GAP
    } else {
        0.0
    };
    let each = segment_width(row.width(), reserved, GAP, items.len());
    let painter = ui.painter().clone();
    let widths: Vec<f32> = items
        .iter()
        .map(|b| w::text_width(&painter, b.label, t::LABEL))
        .collect();
    let named = names_fit(&widths, each);
    let mut clicked = None;
    let mut x = row.left();
    for (i, b) in items.iter().enumerate() {
        let at = Rect::from_min_size(pos2(x, row.top()), vec2(each, row.height()));
        if w::button_shown(
            ui,
            at,
            (salt, i),
            b.label,
            b.lit,
            true,
            Some(&b.tooltip),
            Some(b.icon),
            named,
            Some(b.lit),
        )
        .clicked()
        {
            clicked = Some(i);
        }
        if b.outlined && !b.lit {
            w::outline(ui.painter(), at.shrink(1.0), t::ACCENT, 1.0, 4.0);
        }
        x += each + GAP;
    }
    let mut more_clicked = false;
    if let Some(m) = more {
        let at = Rect::from_min_size(
            pos2(row.right() - MORE_WIDTH, row.top()),
            vec2(MORE_WIDTH, row.height()),
        );
        more_clicked = w::button_shown(
            ui,
            at,
            (salt, "more"),
            m.label,
            m.lit && m.enabled,
            m.enabled,
            Some(&m.tooltip),
            Some("more_horizontal"),
            false,
            Some(m.lit),
        )
        .clicked();
        if m.lit && !m.enabled {
            // 点いたまま押せない: 沈んだ青の地に、薄いアイコン
            let p = ui.painter();
            w::rounded(p, at, t::ACCENT_DIM, 4.0);
            w::icon(p, at, "more_horizontal", t::TEXT.gamma_multiply(0.55), 16.0);
        }
    }
    (clicked, more_clicked)
}

/// 作成方法の 1 行（新規・追加・削除に、切り替えの「⋯」。開くと共通も出る。選んでいるのが点き、バーの作成方法と同じ値）。選んでいるのが
/// 「共通」のあいだは、畳む設定でも 4 つ出す。Shift+Ctrl を押している間に畳んでいると、効いている共通の代わりに「⋯」が点く。
pub fn creation_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let l = app.lang;
    let held = ui.input(|i| i.modifiers);
    let effective = super::combine_of(app.sel.combine, egui::PointerButton::Primary, held);
    let all =
        app.prefs.settings.selection_all_modes || app.sel.combine == SelectionCombine::Intersect;
    let modes = if all {
        &CREATION_MODES[..]
    } else {
        &CREATION_MODES[..3]
    };
    let items: Vec<Segment> = modes
        .iter()
        .map(|&mode| Segment {
            label: super::combine_name(l, mode),
            icon: super::saved::creation_icon(mode),
            lit: effective == mode,
            outlined: app.sel.combine == mode,
            tooltip: combine_tooltip(l, mode),
        })
        .collect();
    // 共通を選んでいるあいだは、畳む設定でも 4 つ出る（畳めない）ので、押せない
    let pinned = app.sel.combine == SelectionCombine::Intersect;
    let label = l.pick("すべての作成方法", "All modes");
    let more = MoreButton {
        label,
        lit: all || effective == SelectionCombine::Intersect,
        enabled: !pinned,
        tooltip: if pinned {
            l.pick(
                "すべての作成方法（共通を選んでいる間）",
                "All modes (while Intersect is chosen)",
            )
            .to_owned()
        } else {
            label.to_owned()
        },
    };
    let (clicked, more_clicked) = segmented_row(ui, rows, "props.select.mode", &items, Some(more));
    if let Some(i) = clicked {
        app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::Combine(
            modes[i],
        ))));
    }
    if more_clicked {
        let on = !app.prefs.settings.selection_all_modes;
        app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::AllModes(on))));
    }
}

/// 選択ペン・選択消しの 1 行（バーと同じ値）。
fn pen_row(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let l = app.lang;
    let held = ui.input(|i| i.modifiers);
    let erasing = super::pen::erases(app.sel.pen_erase, held);
    let items: Vec<Segment> = [false, true]
        .into_iter()
        .map(|erase| Segment {
            label: pen_name(l, erase),
            icon: pen_icon(erase),
            lit: erasing == erase,
            outlined: app.sel.pen_erase == erase,
            tooltip: pen_tooltip(l, erase),
        })
        .collect();
    if let (Some(i), _) = segmented_row(ui, rows, "props.select.pen", &items, None) {
        app.apply(Action::Sel(SelAction::Ui(super::SelUiOp::PenErase(i == 1))));
    }
}

/// ツールプロパティの中身（選択のツールのもの。ID の色で選択は範囲のツールの欄 `region_props`）。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    if app.tool == Tool::SelectPen {
        pen_row(ui, app, rows);
    } else {
        creation_row(ui, app, rows);
    }
    tool_settings(ui, app, rows);
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
            None,
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
            None,
            true,
        ) {
            app.sel.all_layers = v;
        }
        if let Some(v) = crate::panels::properties::snap_symmetry_row(
            ui,
            rows,
            lang,
            "props.wand.snap-symmetry",
            app.sel.snap_symmetry,
            app.paints_only_in_3d()
                .then(|| lang.pick("2D だけ", "2D only")),
        ) {
            app.sel.snap_symmetry = v;
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
            Some(lang.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])")),
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
            Some(shared),
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
        (!aa_applies).then(|| lang.pick("角が丸いときに効く", "Applies to rounded corners")),
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
            Some(&crate::shortcuts::named_with_keys(
                lang,
                lang.pick("縦横比を固定", "Fixed ratio"),
                &[Some(super::shape::RATIO_KEY.to_owned())],
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
            Some(&crate::shortcuts::named_with_keys(
                lang,
                lang.pick("中心から", "From center"),
                &[Some(super::shape::CENTER_KEY.to_owned())],
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
            None,
            true,
        ) {
            app.sel.corner_radius = v.round().clamp(0.0, 512.0) as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyconfig::Combo;
    use crate::keymap::{Operation, GESTURES};
    use crate::selection::pen::erases;
    use egui::{Modifiers, PointerButton};

    #[test]
    fn segment_width_splits_the_row_evenly_and_never_goes_negative() {
        // 3 つ + 「⋯」（28 + 4）: (252 - 32 - 8) / 3
        assert!((segment_width(252.0, 32.0, 4.0, 3) - 212.0 / 3.0).abs() < 1e-4);
        // 2 つで「⋯」なし
        assert_eq!(segment_width(100.0, 0.0, 4.0, 2), 48.0);
        // 幅が足りなくても負にしない
        assert_eq!(segment_width(20.0, 32.0, 4.0, 4), 0.0);
        assert_eq!(segment_width(0.0, 0.0, 4.0, 3), 0.0);
        // ボタンが 0 個でも壊れない
        assert_eq!(segment_width(100.0, 0.0, 4.0, 0), 100.0);
    }

    #[test]
    fn names_fit_only_when_every_name_and_the_icon_fit_in_the_button() {
        // 名前の幅 + 22（アイコンと間）が、ボタンの幅に入るときだけ
        assert!(names_fit(&[26.0, 26.0, 26.0], 70.0));
        assert!(names_fit(&[48.0], 70.0), "ちょうど入る");
        assert!(
            !names_fit(&[26.0, 49.0], 70.0),
            "1 つでも入らなければ全部外す"
        );
        // 狭い・幅が 0
        assert!(!names_fit(&[26.0], 40.0));
        assert!(!names_fit(&[1.0], 0.0));
        // 名前が無ければ（空の並び）出す
        assert!(names_fit(&[], 0.0));
    }

    fn index_of(op: Operation) -> u8 {
        GESTURES
            .iter()
            .find(|g| g.scope == "selection" && g.operation == op && g.starts)
            .unwrap()
            .index
    }

    #[test]
    fn the_creation_and_pen_tooltips_follow_the_selection_modifier_assignments() {
        let mut app = AppState::new(8, 8);
        let add = index_of(Operation::SelectionAdd);
        let subtract = index_of(Operation::SelectionSubtract);
        // 既定
        for (lang, new, add_tip, sub_tip, isect_tip, pen, eraser) in [
            (
                Lang::Ja,
                "新規",
                "追加（Shift）",
                "削除（Ctrl）",
                "共通（Shift+Ctrl）",
                "選択ペン（Shift）",
                "選択消し（Ctrl）",
            ),
            (
                Lang::En,
                "New",
                "Add (Shift)",
                "Subtract (Ctrl)",
                "Intersect (Shift+Ctrl)",
                "Selection Pen (Shift)",
                "Selection Eraser (Ctrl)",
            ),
        ] {
            assert_eq!(combine_tooltip(lang, SelectionCombine::Replace), new);
            assert_eq!(combine_tooltip(lang, SelectionCombine::Add), add_tip);
            assert_eq!(combine_tooltip(lang, SelectionCombine::Subtract), sub_tip);
            assert_eq!(
                combine_tooltip(lang, SelectionCombine::Intersect),
                isect_tip
            );
            assert_eq!(pen_tooltip(lang, false), pen);
            assert_eq!(pen_tooltip(lang, true), eraser);
        }
        // 「追加」を Alt + 左に替えると、追加・選択ペンのツールチップと、選択ペンに替える修飾が替わる
        let alt_left = Combo {
            button: PointerButton::Primary,
            alt: true,
            shift: false,
            ctrl: false,
        };
        app.keys.set_combo(add, Some(alt_left));
        assert_eq!(
            combine_tooltip(Lang::En, SelectionCombine::Add),
            "Add (Alt)"
        );
        assert_eq!(
            combine_tooltip(Lang::Ja, SelectionCombine::Add),
            "追加（Alt）"
        );
        assert_eq!(pen_tooltip(Lang::En, false), "Selection Pen (Alt)");
        let alt = Modifiers {
            alt: true,
            ..Modifiers::NONE
        };
        assert!(!erases(true, alt), "Alt で選択ペンに替わる");
        assert!(erases(true, Modifiers::NONE) && !erases(false, Modifiers::SHIFT));
        // 「引く」の行を外すと、修飾は無くなり、ツールチップは名前だけ
        app.keys.set_combo(subtract, None);
        assert_eq!(
            combine_tooltip(Lang::En, SelectionCombine::Subtract),
            "Subtract"
        );
        assert_eq!(pen_tooltip(Lang::Ja, true), "選択消し");
        assert!(
            !erases(false, Modifiers::COMMAND),
            "外した修飾では替わらない"
        );
        // 戻せば既定
        app.keys.reset_combo(add);
        app.keys.reset_combo(subtract);
        assert_eq!(
            combine_tooltip(Lang::En, SelectionCombine::Add),
            "Add (Shift)"
        );
        assert!(erases(false, Modifiers::COMMAND));
    }
}
