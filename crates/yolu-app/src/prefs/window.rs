//! 設定のウィンドウの枠: 左に区分、上に項目の名前で探す欄、右にその区分の項目（区分ごとにスクロール）。
//!
//! - 左の区分を押すと右の欄がその区分に替わる（探す欄は空にする）。前に開いた区分は `PrefsState::category` が覚える。
//! - 探す欄に文字があるときは、右の欄に、全部の区分から名前の合う項目だけを区分ごとにまとめて並べ、左では、合う項目のある区分を光らせる
//!   （合わない区分は薄くする）。区分の名前に合うときは、その区分の項目を全部出す。
//! - 「ショートカット」の区分は、左の区分の下にショートカットの区分（どこでも・視点…）が並び、右の欄は表（`shortcuts::editor`）。
//!   「ペン」の区分は、ペンの入力と筆圧の調整（`pen::window`）。
//! - Esc: 探す欄に文字があれば空にし、なければウィンドウを閉じる。ほかの物が使う Esc では閉じない: 割り当てを待っている間・ポップアップ
//!   （そのフレームの初めに Esc で閉じたものも）・確認のウィンドウ・色のウィンドウ・スライダーや曲線を掴んでいる間・探す欄以外の入力欄に
//!   打っている間（egui は Esc を受けたフレームの初めに欄のフォーカスを外すので、前のフレームの終わりのフォーカスで見る）。
//! - 左の区分の列は、低い画面で収まらないとき共通のスクロールで送る。

use egui::{pos2, vec2, Id, Key, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::sections::{self, draw_item, heading, Category, Item, Out, Request, Slot, View};
use super::PrefsAction;
use crate::shortcuts::editor;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// ウィンドウの大きさ（1280 × 800 の画面に収まる。小さい画面では画面に合わせて縮み、右の欄をスクロールで送る）。
const SIZE: Vec2 = Vec2::new(940.0, 700.0);
const SIDEBAR: f32 = 210.0;
const SIDE_ROW: f32 = 29.0;
/// 右の欄のうち、上の探す欄が占める高さ（本体の上端から）。
pub const PANE_TOP: f32 = 52.0;

pub(super) fn id() -> Id {
    Id::new("yolu.prefs")
}

fn search_id() -> Id {
    Id::new("yolu.prefs.search")
}

/// 前のフレームの終わりのフォーカスを覚える場所（フレームの番号と、フォーカスのある部品）。
fn focus_key() -> Id {
    id().with("focus")
}

/// 前のフレームの終わりに、探す欄以外の部品（入力欄）にフォーカスがあったか。egui は Esc を受けたフレームの初めに入力欄のフォーカスを外すので、
/// このフレームの `focused()` では、Esc が入力をやめるのに使われたことが分からない（キャンバスの `typed_last` と同じ見方）。
fn typed_last(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Option<Id>)>(focus_key()))
        .is_some_and(|(at, focused)| frame <= at + 1 && focused.is_some_and(|f| f != search_id()))
}

/// 最後に描いたウィンドウの矩形（画面の点。開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, id())
}

/// 最後に並べた右の欄の中身の高さ（欄の上端から。画面の点。ショートカットの区分では None）。試験が、中身が欄に収まるかを見る。
pub fn drawn_content_height(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|d| d.get_temp(id().with("content")))
}

/// 右の欄に項目を並べているか（ショートカットの表は項目でなく表）。探す欄の文字・区分から決まる。
fn searching(app: &AppState) -> bool {
    !app.prefs.search.trim().is_empty()
}

/// 筆圧の調整（描く枠）が右の欄に出るか。
fn pressure_in_view(app: &AppState) -> bool {
    if !app.prefs.open {
        return false;
    }
    if searching(app) {
        sections::search(app, &app.prefs.search)
            .iter()
            .any(|(_, items)| items.contains(&Item::PressureAdjust))
    } else {
        app.prefs.category == Category::Pen
    }
}

impl AppState {
    /// 右の欄に出る物が変わったときの後始末（毎フレーム、描く前と描いたあとに呼ぶ）: ウィンドウを閉じたら、ぶつかるキーを保存していないことを
    /// 知らせる（どの区分を見ていても）。ショートカットの区分を離れた・ウィンドウを閉じたら割り当て待ちとパイの項目選びをやめ、筆圧の調整が出始めたら
    /// 今の調整を「元に戻す」の行き先に覚え、消えたら描いた線を捨てる。
    pub(crate) fn prefs_sync_views(&mut self) {
        let open = self.prefs.open;
        if self.prefs.was_open && !open {
            editor::warn_unsaved(self);
        }
        self.prefs.was_open = open;
        let shortcuts = self.prefs.shows(Category::Shortcuts);
        if self.prefs.shortcuts_shown && !shortcuts {
            editor::leave(self);
        }
        self.prefs.shortcuts_shown = shortcuts;
        let pressure = pressure_in_view(self);
        if pressure && !self.pressure.entered() {
            self.pressure_enter();
        } else if !pressure && self.pressure.entered() {
            self.pressure_leave();
        }
    }
}

/// 開いていればウィンドウを描き、選んだ値を `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    app.prefs_sync_views();
    if !app.prefs.open {
        app.prefs.dragging = false;
        app.prefs.gpu_memory_before_drag = None;
        return;
    }
    let lang = app.lang;
    let spec = Spec {
        title: lang.pick("設定", "Settings"),
        icon: Some("tune"),
        size: SIZE,
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let window_id = id();
    let finished = editor::begin_frame(ctx, app, app.prefs.shows(Category::Shortcuts));
    let mut offset = app.prefs.offset;
    let mut out = Out::default();
    let mut esc = false;
    let closed = window::show(ctx, window_id, &spec, &mut offset, false, |ui, frame| {
        esc = take_escape(ui, app, frame.rect, finished);
        body(ui, app, frame.body, &mut out);
    });
    app.prefs.offset = offset;
    app.prefs.dragging = out.dragging || out.gpu_dragging;
    for request in out.requests {
        match request {
            Request::Open(which, anchor) => {
                app.popup = Some(OpenPopup {
                    kind: PopupKind::M2(crate::m2_menu::Popup::Pref(which)),
                    state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
                });
            }
            Request::Do(action) => app.apply(Action::Prefs(action)),
            Request::Apply(action) => app.apply(*action),
        }
    }
    if closed || esc {
        app.apply(Action::Prefs(PrefsAction::Close));
    }
    // このフレームの終わりのフォーカス（次のフレームの Esc が、入力欄の入力をやめるのに使われたかを見る）
    let focused = ctx.memory(|m| m.focused());
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(focus_key(), (frame, focused)));
    app.prefs_sync_views();
}

/// この Esc でウィンドウを閉じるか（探す欄に文字があるときは、その文字を消すだけ）。ほかの物が使う Esc では閉じない。
fn take_escape(ui: &Ui, app: &mut AppState, window: Rect, finished: bool) -> bool {
    let ctx = ui.ctx();
    if finished
        || app.shortcuts.capturing()
        || !ui.input(|i| i.key_pressed(Key::Escape))
        || window::escape_taken(ctx)
        || !ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| window.contains(p))
    {
        return false;
    }
    // ポップアップ（このフレームの初めに Esc で閉じたものも。`popups` がこの描き方より先に閉じる）・確認のウィンドウ・色のウィンドウ・
    // スライダーや曲線を掴んでいる間の Esc は、それぞれが使う
    if app.popup.is_some()
        || app.ui.popup_was_open
        || crate::windows::modal_open(app)
        || crate::panels::color_window::is_open(ctx)
        || ui.input(|i| i.pointer.primary_down())
    {
        return false;
    }
    if searching(app) {
        app.prefs.search.clear();
        app.prefs.scroll = 0.0;
        window::note_escape_taken(ctx);
        return false;
    }
    // ほかの入力欄（ポート番号・表の探す欄）に打っている間の Esc は、その欄が使う
    let typing_now = ctx
        .memory(|m| m.focused())
        .is_some_and(|f| f != search_id());
    !(typing_now || typed_last(ctx))
}

fn body(ui: &mut Ui, app: &mut AppState, body: Rect, out: &mut Out) {
    let p = ui.painter().clone();
    let side = Rect::from_min_size(body.min, vec2(SIDEBAR, body.height()));
    w::fill(&p, side, t::PANEL_HEADER);
    let main = Rect::from_min_max(pos2(side.right(), body.top()), body.max);
    let found = sections::search(app, &app.prefs.search);
    sidebar(ui, app, side, &found);
    search_field(
        ui,
        app,
        Rect::from_min_size(
            pos2(main.left() + 16.0, main.top() + 12.0),
            vec2(main.width() - 32.0, 28.0),
        ),
    );
    // 左の区分を押したフレームは、探す欄が空になり、区分が替わっている
    let pane = Rect::from_min_max(pos2(main.left(), main.top() + PANE_TOP), main.max);
    if searching(app) {
        let found = sections::search(app, &app.prefs.search);
        let groups: Vec<(Option<Category>, Vec<Item>)> = found
            .into_iter()
            .map(|(category, items)| (Some(category), items))
            .collect();
        page(ui, app, pane, &groups, 255, out);
    } else if app.prefs.category == Category::Shortcuts {
        editor::content(ui, app, pane);
        ui.ctx().data_mut(|d| d.remove::<f32>(id().with("content")));
    } else {
        let items = app.prefs.category.items(app);
        let groups = [(None, items)];
        page(ui, app, pane, &groups, app.prefs.category as u8, out);
    }
}

/// 左の区分（ショートカットを見ているときは、その下にショートカットの区分）。探しているときは、合う項目のある区分を光らせる。
fn sidebar(ui: &mut Ui, app: &mut AppState, side: Rect, found: &[(Category, Vec<Item>)]) {
    let lang = app.lang;
    let searching = searching(app);
    // 低い画面で下の区分がウィンドウからはみ出ても、ウィンドウの外へは描かない（絵筆は、切り取りを入れたあとの物を使う）
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(side.intersect(outer_clip));
    let p = ui.painter().clone();
    let categories: Vec<Category> = Category::ALL
        .into_iter()
        .filter(|c| c.available(app))
        .collect();
    // 列の高さ（ショートカットを開いているときは、その下の区分も）。収まらなければ、共通のスクロールで送る
    let row_count = categories.len()
        + if app.prefs.category == Category::Shortcuts && !searching {
            editor::Section::ALL.len()
        } else {
            0
        };
    let mut scroll = app.prefs.side_scroll;
    let bar = Scroll::begin(ui, side, 20.0 + row_count as f32 * SIDE_ROW, &mut scroll);
    let row_width = side.width() - bar.reserved();
    let mut y = side.top() + 10.0 - scroll;
    for category in categories {
        let r = Rect::from_min_size(pos2(side.left(), y), vec2(row_width, SIDE_ROW));
        y += SIDE_ROW;
        let current = app.prefs.category == category && !searching;
        // ショートカットの区分を開いているときは、下の区分の 1 つが選ばれた印になる（「キーで探す」で絞っている間は、ここが選ばれた印）
        let expanded = category == Category::Shortcuts && current;
        let selected = current && (!expanded || app.shortcuts.key_filter.is_some());
        let lit = searching && found.iter().any(|(c, _)| *c == category);
        let response = ui.interact(
            r,
            Id::new(("yolu.prefs.category", category as u8)),
            Sense::click(),
        );
        if selected || lit {
            w::fill(&p, r, t::ACCENT_SOFT);
        } else if response.hovered() {
            w::fill(&p, r, t::CONTROL_HOVER);
        }
        if lit {
            w::fill(
                &p,
                Rect::from_min_size(r.min, vec2(3.0, r.height())),
                t::ACCENT,
            );
        }
        let color = if selected || lit || expanded {
            t::TEXT
        } else if searching {
            t::TEXT_DISABLED
        } else {
            t::TEXT_DIM
        };
        let label = category.label(lang);
        let shown = w::fit(&p, label, r.width() - 28.0, t::LABEL);
        w::text(
            &p,
            r.shrink2(vec2(14.0, 0.0)),
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected || lit, label)
        });
        if response.clicked() {
            app.apply(Action::Prefs(PrefsAction::Choose(category)));
        }
        if expanded {
            for section in editor::Section::ALL {
                let r = Rect::from_min_size(pos2(side.left(), y), vec2(row_width, SIDE_ROW));
                y += SIDE_ROW;
                let on = app.shortcuts.section == section && app.shortcuts.key_filter.is_none();
                let response = ui.interact(
                    r,
                    Id::new(("yolu.prefs.shortcuts_section", section as u8)),
                    Sense::click(),
                );
                if on {
                    w::fill(&p, r, t::ACCENT_SOFT);
                } else if response.hovered() {
                    w::fill(&p, r, t::CONTROL_HOVER);
                }
                let label = section.label(lang);
                let shown = w::fit(&p, label, r.width() - 42.0, t::LABEL);
                w::text(
                    &p,
                    r.shrink2(vec2(28.0, 0.0)),
                    &shown,
                    t::LABEL.with_color(if on { t::TEXT } else { t::TEXT_DIM }),
                    Align::Left,
                );
                response.widget_info(|| {
                    WidgetInfo::selected(WidgetType::SelectableLabel, true, on, label)
                });
                if response.clicked() {
                    editor::select_section(app, section);
                }
            }
        }
    }
    ui.set_clip_rect(outer_clip);
    bar.end(ui, id().with("side-scroll"), &mut scroll);
    app.prefs.side_scroll = scroll;
}

/// 上の探す欄（項目の名前で探す。右端の印で空にする）。
fn search_field(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    {
        let p = ui.painter();
        w::rounded(p, r, t::CONTROL_BG, 3.0);
        w::outline(p, r, t::BORDER, 1.0, 3.0);
        w::icon(
            p,
            Rect::from_min_size(r.min + vec2(6.0, 0.0), vec2(18.0, r.height())),
            "search",
            t::TEXT_DIM,
            16.0,
        );
    }
    let has_text = !app.prefs.search.is_empty();
    let clear = Rect::from_min_size(
        pos2(r.right() - 26.0, r.top() + 3.0),
        vec2(22.0, r.height() - 6.0),
    );
    let inner = Rect::from_min_max(
        r.min + vec2(30.0, 1.0),
        pos2(
            if has_text {
                clear.left() - 2.0
            } else {
                r.right() - 6.0
            },
            r.bottom() - 1.0,
        ),
    );
    let mut text = app.prefs.search.clone();
    let response = ui.put(
        inner,
        egui::TextEdit::singleline(&mut text)
            .id(search_id())
            .frame(egui::Frame::NONE)
            .font(t::LABEL.font())
            .text_color(t::TEXT)
            .hint_text(lang.pick("設定を探す", "Search Settings"))
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center),
    );
    if response.changed() {
        app.prefs.search = text;
        app.prefs.scroll = 0.0;
    }
    if has_text
        && w::icon_button(
            ui,
            clear,
            id().with("search-clear"),
            "close",
            lang.pick("検索を消す", "Clear Search"),
            false,
            true,
            13.0,
        )
        .clicked()
    {
        app.prefs.search.clear();
        app.prefs.scroll = 0.0;
    }
}

/// 項目の行を、右の欄に並べる（`groups` の見出しは、探した結果のとき（`Some`）だけ出す）。`key` は、欄ごとに中身の高さを覚える鍵。
fn page(
    ui: &mut Ui,
    app: &mut AppState,
    pane: Rect,
    groups: &[(Option<Category>, Vec<Item>)],
    key: u8,
    out: &mut Out,
) {
    let ctx = ui.ctx().clone();
    // 区分を押したフレームは、押した所で 0 に戻してある（ここで読む）
    let mut scroll = app.prefs.scroll;
    let lang = app.lang;
    let v = View::of(app);
    let content_id = id().with("content");
    // 中身の高さは前のフレームのもの（初めは 0 で、つまみは次のフレームから）
    let previous = ctx
        .data(|d| d.get_temp::<f32>(content_id.with(key)))
        .unwrap_or(0.0);
    let bar = Scroll::begin(ui, pane, previous, &mut scroll);
    // 行（名前の列と値の列）は、上の探す欄と同じ左右の端（欄の左右 16）まで。つまみは右の余白（16）の中に出るので、行から引かない
    let area = Rect::from_min_max(
        pos2(pane.left() + 8.0, pane.top() - scroll),
        pos2(pane.right() - 8.0, pane.bottom()),
    );
    let outer_clip = ui.clip_rect();
    let visible = pane.intersect(outer_clip);
    ui.set_clip_rect(visible);
    let mut rows = w::Rows::new(area, 4.0);
    for (gi, (category, items)) in groups.iter().enumerate() {
        if let Some(category) = category {
            heading(ui, &mut rows, gi > 0, category.label(lang));
            if items.is_empty() {
                // 項目の無い区分（ショートカットの表）は、区分の名前の行から開く
                let label = category.label(lang);
                if w::button(
                    ui,
                    rows.row(t::ROW_HEIGHT, super::GAP),
                    ("prefs", format!("open-{}", *category as u8)),
                    label,
                    false,
                    true,
                    None,
                    None,
                )
                .clicked()
                {
                    out.requests
                        .push(Request::Do(PrefsAction::Choose(*category)));
                }
            }
        }
        for (ii, item) in items.iter().enumerate() {
            let slot = Slot {
                first: ii == 0,
                visible,
            };
            draw_item(ui, &mut rows, app, &v, *item, &slot, out);
        }
    }
    rows.space(8.0);
    let used = rows.used();
    ctx.data_mut(|d| {
        d.insert_temp(content_id.with(key), used);
        d.insert_temp(content_id, used);
    });
    // 中身の高さが前のフレームと違えば（区分を移した・詳しくを開け閉めした）、つまみの出し入れを次のフレームで合わせる
    if (used - previous).abs() > 0.5 {
        ctx.request_repaint();
    }
    ui.set_clip_rect(outer_clip);
    bar.end(ui, id().with("scroll"), &mut scroll);
    app.prefs.scroll = scroll;
}
