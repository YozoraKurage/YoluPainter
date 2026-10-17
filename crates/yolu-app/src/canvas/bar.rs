//! 2D キャンバスの右上に重ねる部品（3D ビューには出さない）。同じ形の小さな半透明のボタンが 2 つ並ぶ。
//!
//! - 左は「目・UV・▽」（中身は `uv_wireframe::bar`）: 目と UV の文字を押すと UV の線の表示・非表示がすぐ切り替わり、▽ を押すと UV の線の色と
//!   不透明度のメニューが開く。重なった UV の色は出さない。
//! - 右は、描くチャンネルのアイコンと名前・▽: 押すと下に 3 つの列（テクスチャセット・今のテクスチャセットのレイヤーセット・チャンネル）の欄が開く。
//!   各列の見出しの右の ＋ で、テクスチャセット・レイヤーセットを追加する。チャンネルの行は、押すと描くチャンネル（青い行）、右の目で 2D のキャンバスに見せる
//!   チャンネル。欄の中身（行の名前・印・押したときの操作）は状態の側が作り（`AppState::canvas_bar_columns`）、ここは並べて描くだけ。
//!
//! 描いている間は押せない（開いていれば閉じる）。欄が開いている間は、キャンバスへの押し・ホイールを渡さない（`windows::pointer_blocked`）。

use egui::{pos2, vec2, Color32, Id, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::uv_wireframe::bar as uv;

const MARGIN: f32 = 8.0;
const GAP: f32 = 4.0;
pub const BUTTON_HEIGHT: f32 = 26.0;
const ROW: f32 = 26.0;
const HEAD: f32 = 28.0;
pub(crate) const PAD: f32 = 6.0;
const COLUMNS: [f32; 3] = [142.0, 150.0, 196.0];
/// 部品の下の端（キャンバスの上の端から。その下に状態のアイコンの列を置く）。
pub const BOTTOM: f32 = MARGIN + BUTTON_HEIGHT + 4.0;

/// 開いている欄と、各列のスクロール（保存しない）。
#[derive(Debug, Default)]
pub struct BarState {
    /// UV の線の色と不透明度の欄を開いている。
    pub uv_menu: bool,
    /// 3 列の欄を開いている。
    pub open: bool,
    /// 不透明度のスライダーをドラッグしている間（設定のファイルへは、離すまで書かない）。
    pub dragging: bool,
    scroll: [f32; 3],
}

impl BarState {
    pub fn is_open(&self) -> bool {
        self.uv_menu || self.open
    }

    fn close(&mut self) {
        self.uv_menu = false;
        self.open = false;
    }

    /// 欄を開く・閉じる（もう片方の欄は閉じる）。
    fn toggle_uv_menu(&mut self) {
        let open = !self.uv_menu;
        self.close();
        self.uv_menu = open;
    }

    fn toggle_open(&mut self) {
        let open = !self.open;
        self.close();
        self.open = open;
    }
}

type Chip = crate::state::canvas_bar::Chip;
type Row = crate::state::canvas_bar::Row<Action>;
type Column = crate::state::canvas_bar::Column<Action>;

/// ボタンの地（閉じているとき。キャンバスが透ける）。
pub(crate) fn button_bg(open: bool, hovered: bool, pressed: bool, dimmed: bool) -> Color32 {
    if open {
        t::ACCENT_DIM
    } else if pressed {
        Color32::from_black_alpha(220)
    } else if hovered {
        Color32::from_black_alpha(190)
    } else if dimmed {
        Color32::from_black_alpha(200)
    } else {
        Color32::from_black_alpha(120)
    }
}

/// 右上の 2 つのボタンの位置（右端がチャンネルのボタン、その左が UV のボタン）。
struct Placement {
    chip: Rect,
    /// UV のボタンの、表示・非表示の範囲と ▽ の範囲、その両方。
    toggle: Rect,
    arrow: Rect,
    uv: Rect,
    /// 2 つのボタンを含む範囲。
    bar: Rect,
}

fn place(ui: &Ui, view: Rect, chip_name: &str) -> Placement {
    let chip_w = (w::text_width(ui.painter(), chip_name, t::LABEL) + 6.0 + 20.0 + 4.0 + 18.0 + 6.0)
        .max(84.0);
    let chip = Rect::from_min_size(
        pos2(view.right() - MARGIN - chip_w, view.top() + MARGIN),
        vec2(chip_w, BUTTON_HEIGHT),
    );
    let toggle_w = uv::toggle_width(ui);
    let uv = Rect::from_min_size(
        pos2(
            chip.left() - GAP - toggle_w - uv::ARROW_WIDTH,
            view.top() + MARGIN,
        ),
        vec2(toggle_w + uv::ARROW_WIDTH, BUTTON_HEIGHT),
    );
    let toggle = Rect::from_min_size(uv.min, vec2(toggle_w, BUTTON_HEIGHT));
    let arrow = Rect::from_min_max(pos2(toggle.right(), uv.top()), uv.max);
    Placement {
        chip,
        toggle,
        arrow,
        uv,
        bar: uv.union(chip),
    }
}

/// 押された物。
#[derive(Default)]
struct Clicks {
    uv_toggle: bool,
    uv_menu: bool,
    chip: bool,
}

/// 部品の全体（右上の 2 つのボタンと、開いている欄）。`view` はキャンバスの表示域、`free` は描いていないこと。
pub fn show(ui: &mut Ui, app: &mut AppState, view: Rect, free: bool) {
    let ctx = ui.ctx().clone();
    if !free {
        app.ui.canvas_bar.close();
    }
    app.ui.canvas_bar.dragging = false;
    let chip = app.canvas_bar_chip();
    let place = place(ui, view, &chip.name);
    let clicks = buttons(&ctx, app, &place, &chip, free);
    let mut actions: Vec<Action> = Vec::new();
    if clicks.uv_toggle {
        actions.push(Action::ToggleUvWireframe);
    }
    if clicks.uv_menu {
        app.ui.canvas_bar.toggle_uv_menu();
    }
    if clicks.chip {
        app.ui.canvas_bar.toggle_open();
    }

    // 開いている欄
    let mut popover = Rect::NOTHING;
    if app.ui.canvas_bar.uv_menu {
        popover = uv::popover(&ctx, app, place.uv);
    }
    if app.ui.canvas_bar.open {
        popover = channels_popover(&ctx, app, view, place.chip, free, &mut actions);
    }
    close_when_outside(&ctx, app, popover, place.bar);
    for action in actions {
        app.apply(action);
    }
}

/// 欄の外を押したら閉じる（欄・2 つのボタン・欄から開いた色のウィンドウの上は除く）。Esc でも閉じる。
fn close_when_outside(ctx: &egui::Context, app: &mut AppState, popover: Rect, bar: Rect) {
    if !app.ui.canvas_bar.is_open() {
        return;
    }
    let color_window = uv::color_window_rect(ctx);
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|q| {
                !popover.contains(q)
                    && !bar.contains(q)
                    && !color_window.is_some_and(|r| r.contains(q))
            })
    });
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.canvas_bar.close();
    }
}

/// 右上の 2 つのボタンを描いて、押されたかを返す。
fn buttons(
    ctx: &egui::Context,
    app: &AppState,
    place: &Placement,
    chip: &Chip,
    free: bool,
) -> Clicks {
    let mut clicks = Clicks::default();
    egui::Area::new(Id::new("canvasbar.buttons"))
        .order(egui::Order::Middle)
        .fixed_pos(place.bar.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(place.bar.size(), Sense::hover());
            (clicks.uv_toggle, clicks.uv_menu) =
                uv::button(ui, app, place.toggle, place.arrow, free);
            clicks.chip = chip_button(ui, app, place.chip, chip, free);
        });
    clicks
}

/// チャンネルのボタン（UV のボタンと同じ高さ・同じ地）。押したか。
fn chip_button(ui: &mut Ui, app: &AppState, rect: Rect, chip: &Chip, free: bool) -> bool {
    let id = Id::new("canvasbar.chip");
    let look = w::look(ui.ctx(), id, free);
    let response = ui.interact(rect, id, if free { Sense::click() } else { Sense::hover() });
    let open = app.ui.canvas_bar.open;
    let p = ui.painter().clone();
    w::rounded(
        &p,
        rect,
        button_bg(
            open,
            look.live && response.hovered(),
            look.live && response.is_pointer_button_down_on(),
            false,
        ),
        6.0,
    );
    let fg = if open || (look.live && response.hovered()) {
        Color32::WHITE
    } else {
        t::TEXT
    };
    w::icon(
        &p,
        Rect::from_min_size(rect.min + vec2(6.0, 0.0), vec2(20.0, BUTTON_HEIGHT)),
        chip.icon,
        fg,
        16.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), rect.max),
        &chip.name,
        t::LABEL.with_color(fg),
        Align::Left,
    );
    w::icon(
        &p,
        Rect::from_min_size(
            pos2(rect.right() - 22.0, rect.top()),
            vec2(18.0, BUTTON_HEIGHT),
        ),
        "arrow_drop_down",
        fg,
        18.0,
    );
    let tip = app.lang.t("canvasbar-paint-channel-tip");
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, free, tip));
    let response = response.on_hover_text(tip);
    free && response.clicked()
}

/// 欄の 1 行（選んでいる行は青、乗せると明るく）。
fn row(
    ui: &mut Ui,
    clip: Rect,
    r: Rect,
    id: (&str, u128, usize),
    icon: Option<&str>,
    label: &str,
    selected: bool,
) -> egui::Response {
    let response = ui.interact(r.intersect(clip), Id::new(id), Sense::click());
    let p = ui.painter_at(clip);
    if selected {
        w::rounded(&p, r, t::ACCENT_DIM, 4.0);
    } else if response.hovered() {
        w::rounded(&p, r, t::CONTROL_HOVER, 4.0);
    }
    let color = if selected { Color32::WHITE } else { t::TEXT };
    let mut left = r.left() + 8.0;
    if let Some(icon) = icon {
        w::icon(
            &p,
            Rect::from_min_size(pos2(left - 2.0, r.top()), vec2(20.0, r.height())),
            icon,
            if selected {
                Color32::WHITE
            } else {
                t::TEXT_DIM
            },
            16.0,
        );
        left += 22.0;
    }
    let text = Rect::from_min_max(pos2(left, r.top()), pos2(r.right() - 4.0, r.bottom()));
    let shown = w::fit(&p, label, text.width(), t::LABEL);
    w::text(&p, text, &shown, t::LABEL.with_color(color), Align::Left);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    response
}

/// 3 つの列の欄の大きさと位置。
struct Frame {
    pop: Rect,
    widths: [f32; 3],
}

/// 欄の大きさ（右端をボタンの右端にそろえる）。列の幅は、キャンバスが狭いときだけ縮める。行の数は 3 つの列の多いほうで、画面に収まらなければ列ごとにスクロール。
fn frame(view: Rect, chip: Rect, columns: &[Column; 3]) -> Frame {
    let available = (view.width() - 2.0 * MARGIN - 2.0 * 2.0 - 2.0 * PAD).max(1.0);
    let scale = (available / COLUMNS.iter().sum::<f32>()).clamp(0.55, 1.0);
    let widths = COLUMNS.map(|c| (c * scale).floor());
    let width = widths.iter().sum::<f32>() + 2.0 * 2.0 + PAD * 2.0;
    let wanted = columns
        .iter()
        .map(|c| c.rows.len())
        .max()
        .unwrap_or(0)
        .max(1);
    let room = (view.bottom() - chip.bottom() - 4.0 - MARGIN - 2.0 * PAD - HEAD).max(ROW);
    let rows = wanted.min((room / ROW).floor().max(1.0) as usize);
    let height = PAD * 2.0 + HEAD + rows as f32 * ROW;
    let left = (chip.right() - width).max(view.left() + MARGIN);
    Frame {
        pop: Rect::from_min_size(pos2(left, chip.bottom() + 4.0), vec2(width, height)),
        widths,
    }
}

/// チャンネルのボタンの下に開く 3 つの列の欄。矩形を返す。
fn channels_popover(
    ctx: &egui::Context,
    app: &mut AppState,
    view: Rect,
    chip: Rect,
    free: bool,
    actions: &mut Vec<Action>,
) -> Rect {
    let columns = app.canvas_bar_columns(free);
    let Frame { pop, widths } = frame(view, chip, &columns);
    egui::Area::new(Id::new("canvasbar.pop"))
        .order(egui::Order::Middle)
        .fixed_pos(pop.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(pop.size(), Sense::hover());
            let p = ui.painter().clone();
            w::rounded(&p, pop, t::MENU_BG, 6.0);
            w::outline(&p, pop, t::SEPARATOR, 1.0, 6.0);
            let mut x = pop.left() + PAD;
            for (col, column) in columns.iter().enumerate() {
                let area = Rect::from_min_size(
                    pos2(x, pop.top() + PAD),
                    vec2(widths[col], pop.height() - PAD * 2.0),
                );
                let list = column_head(ui, area, col, column, actions);
                let scroll = &mut app.ui.canvas_bar.scroll[col];
                column_rows(ui, scroll, (col, area, list), column, free, actions);
                x += widths[col];
                if col < 2 {
                    w::vline(
                        &p,
                        x + 1.0,
                        pop.top() + PAD + 4.0,
                        pop.bottom() - PAD - 4.0,
                        t::SEPARATOR,
                    );
                    x += 2.0;
                }
            }
        });
    pop
}

/// 列の頭: アイコン（名前はツールチップ）。テクスチャセットの列は、合うアイコンが無いので文字。テクスチャセット・レイヤーセットの列は右に追加。
/// 行を並べる範囲を返す。
fn column_head(
    ui: &mut Ui,
    area: Rect,
    col: usize,
    column: &Column,
    actions: &mut Vec<Action>,
) -> Rect {
    let p = ui.painter().clone();
    let head = Rect::from_min_size(area.min, vec2(area.width(), HEAD));
    let icon_rect = match column.icon {
        Some(icon) => {
            let r = Rect::from_min_size(head.min + vec2(6.0, 0.0), vec2(22.0, HEAD));
            w::icon(&p, r, icon, t::TEXT_DIM, 16.0);
            r
        }
        None => {
            let r = Rect::from_min_size(
                head.min + vec2(8.0, 0.0),
                vec2((head.width() - 40.0).max(0.0), HEAD),
            );
            let shown = w::fit(&p, column.head, r.width(), t::LABEL_DIM);
            w::text(&p, r, &shown, t::LABEL_DIM, Align::Left);
            r
        }
    };
    let hover = ui.interact(icon_rect, Id::new(("canvasbar.head", col)), Sense::hover());
    hover.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, column.head));
    hover.on_hover_text(column.head);
    if let Some(add) = &column.add {
        let at = Rect::from_min_size(
            pos2(head.right() - 26.0, head.top() + 2.0),
            vec2(24.0, HEAD - 4.0),
        );
        if w::icon_button(
            ui,
            at,
            ("canvasbar.add", col),
            "add",
            add.tip,
            false,
            add.enabled,
            16.0,
        )
        .clicked()
        {
            actions.push(add.action.clone());
        }
    }
    Rect::from_min_max(pos2(area.left(), head.bottom()), area.max)
}

/// 列の中身（行が列に収まらなければスクロール）。`place` は（列の番号, 列の範囲, 行を並べる範囲）。
fn column_rows(
    ui: &mut Ui,
    scroll_state: &mut f32,
    place: (usize, Rect, Rect),
    column: &Column,
    free: bool,
    actions: &mut Vec<Action>,
) {
    let (col, area, list) = place;
    let scroll = Scroll::begin(ui, list, column.rows.len() as f32 * ROW, scroll_state);
    let offset = *scroll_state;
    let row_width = area.width() - scroll.reserved();
    for (i, model) in column.rows.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(area.left(), list.top() + i as f32 * ROW - offset),
            vec2(row_width, ROW - 2.0),
        );
        if r.bottom() < list.top() || r.top() > list.bottom() {
            continue;
        }
        draw_row(ui, list, r, model, free, actions);
    }
    scroll.end_menu(ui, ("canvasbar.scroll", col), scroll_state);
}

/// 欄の 1 行と、あれば右端の目。名前の行を押すと操作を当てる（描いていない間だけ）。目は描いている間も押せる。
fn draw_row(ui: &mut Ui, list: Rect, r: Rect, model: &Row, free: bool, actions: &mut Vec<Action>) {
    // 右端の目（見せるチャンネル）の分を空けた行
    let eye_rect = Rect::from_min_size(pos2(r.right() - 26.0, r.top()), vec2(26.0, r.height()));
    let main = match model.eye {
        Some(_) => Rect::from_min_max(r.min, pos2(eye_rect.left(), r.bottom())),
        None => r,
    };
    let name_row = row(
        ui,
        list,
        main,
        model.key,
        model.icon,
        &model.label,
        model.selected,
    );
    if name_row.clicked() && free {
        actions.push(model.action.clone());
    }
    let Some(eye) = &model.eye else {
        return;
    };
    // 見せるチャンネルの目: 選んでいる行は青い目、ほかは乗せたときだけ薄く
    let hit = ui.interact(
        eye_rect.intersect(list),
        Id::new(("canvasbar.eye", eye.key)),
        Sense::click(),
    );
    if eye.showing || hit.hovered() || name_row.hovered() {
        let color = if eye.showing {
            t::ACCENT
        } else {
            t::TEXT_DISABLED
        };
        w::icon(&ui.painter_at(list), eye_rect, "visibility", color, 16.0);
    }
    hit.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, eye.showing, &eye.tip));
    if hit.on_hover_text(&eye.tip).clicked() {
        actions.push(eye.action.clone());
    }
}
