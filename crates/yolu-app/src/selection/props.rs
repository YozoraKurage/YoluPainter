//! 選択範囲と対称の、オプションバーとツールプロパティの部品。
//! - 選択のツールのオプションバー: 作成方法（新規・追加・削除・共通。選択ペンは選択ペン・選択消し）、選択ペンの直径、自動選択の許容値
//! - ブラシ・消しゴムのオプションバーの右端: 対称の切り替えとモードの選び（▾）
//! - 左のドックのツールプロパティ: 選択のツールの作成方法の帯（新規・追加・削除に、共通を出し入れする「⋯」。選択ペンは選択ペンと選択消し）、
//!   ツールごとの設定（自動選択の許容値・隣接・全レイヤー、形のツールのアンチエイリアス・縦横比・中心から・角の丸め、選択ペンの直径・硬さ・不透明度）。
//!   対称の欄は、ブラシの詳細のウィンドウの「対称」のカテゴリ（`symmetry_fields`）。
//! - ツールプロパティに無い操作の置き場: すべてを選択・クイックマスクは「選択範囲」メニューとキー、選択を解除・選択範囲を反転はメニュー・キー・
//!   選択範囲の下のバー、拡張・縮小・境界をぼかすはメニューとバー、境界線・境界をくっきりはメニューだけ
//!
//! 値は画面の状態を直に、文書を変えるものは `Action::Sel` を通す（1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::symmetry::{axis_name, mode_name, mode_tooltip, AXES_3D, MODES};
use super::{combine_tooltip, SelAction, SymOp};
use crate::engine::{BrushEffect, SelectionCombine, SymmetryMode};
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

/// ブラシの詳細のウィンドウの「対称」の欄（見出しと既定に戻すはウィンドウが出す）。2D の対称（UV の平面）と、3D の対称（モデルの空間。
/// モデルがあるとき）。どちらも 2D のキャンバスと 3D ビューの両方のストロークに効く（両方入っていれば、3D の写しの後に 2D の写し）。
/// 指先・クローンは対称と組めないので、効かない欄を無効にする。
pub fn symmetry_fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, lang: Lang) {
    rows.indent = 0.0;
    let free = !app.is_stroking();
    // モデルがあれば、3D の対称の設定も出す（2D のキャンバスだけを出していても、2D のストロークに効く）
    let both = app.view3d.model.is_some();
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
    // 効かない欄は注記の行を置かず、無効（灰色）にして理由をツールチップに出す。2D の対称は 3D ビューのストロークにも効く（UV の平面で
    // 写す）。指先・クローンは対称を使えない
    let reason: Option<&str> = if matches!(
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
