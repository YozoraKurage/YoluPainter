//! 名前を付けて覚えた選択範囲（CLIP STUDIO の「選択範囲を保存」・Photoshop のアルファチャンネルに当たる）。今のテクスチャセット
//! （文書）ごとに、名前と選択範囲の札を覚え、呼び出すときは新規・追加・削除・共通のどれかで今の選択範囲と組み合わせる（1 回の Undo）。
//!
//! 覚えておくのはセッションの中だけで、.ylp には入れない（形式の版を上げる決まりが先に要る）。文書の大きさやタイルの大きさが
//! 変わると、覚えた選択範囲は呼び出せない（断って理由を出す）。覚えた札の合計はプロジェクト全体で `SAVED_BUDGET_BYTES` までで、
//! セットを消す・別のプロジェクトを開くと、そのセットの札は捨てる（`sel_prune_saved`）。

use egui::{pos2, vec2, Id, Key, Rect, Vec2};

use super::{combine_name, SelAction, SelEdit};
use crate::engine::{SelectionCombine, SelectionMask};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// 1 つの文書に覚えられる数と、覚えた札のバイト数の合計（プロジェクト全体）の上限。
pub const MAX_SAVED: usize = 32;
pub const SAVED_BUDGET_BYTES: u64 = 256 << 20;

/// 覚えた選択範囲。
#[derive(Clone, Debug)]
pub struct SavedSelection {
    pub name: String,
    pub mask: SelectionMask,
}

/// 窓（開いていれば）の状態。
#[derive(Clone, Debug, Default)]
pub struct SavedWindow {
    /// 名前の入力欄。
    pub name: String,
    /// 見出しで動かした量。
    pub offset: Vec2,
    /// 一覧のスクロール（先頭の行からの画面の点）。
    pub scroll: f32,
    /// 開いた直後に入力欄へフォーカスを置く。
    pub focus: bool,
}

/// 覚えた選択範囲の操作（画面と覚えた一覧だけ。文書は変えない）。
#[derive(Clone, Debug, PartialEq)]
pub enum SavedOp {
    OpenWindow,
    CloseWindow,
    /// 今の選択範囲を名前を付けて覚える（同じ名前があれば入れ替える）。
    Save(String),
    Delete(usize),
}

const COMBINES: [SelectionCombine; 4] = [
    SelectionCombine::Replace,
    SelectionCombine::Add,
    SelectionCombine::Subtract,
    SelectionCombine::Intersect,
];

fn combine_icon(mode: SelectionCombine) -> &'static str {
    match mode {
        SelectionCombine::Replace => "color_square",
        SelectionCombine::Add => "shape_union",
        SelectionCombine::Subtract => "shape_subtract",
        SelectionCombine::Intersect => "shape_intersect",
    }
}

/// 作成方法のボタンのアイコン（オプションバーの作成方法の組と同じ）。
pub fn creation_icon(mode: SelectionCombine) -> &'static str {
    combine_icon(mode)
}

impl AppState {
    /// 今の文書に覚えた選択範囲。
    pub fn saved_selections(&self) -> &[SavedSelection] {
        self.sel
            .saved
            .get(&self.sets.current().uid)
            .map_or(&[][..], Vec::as_slice)
    }

    /// 覚えた選択範囲を呼び出すための札（今の文書と大きさが合うときだけ）。
    pub(super) fn saved_mask(&self, index: usize) -> Result<SelectionMask, String> {
        let lang = self.lang;
        let Some(saved) = self.saved_selections().get(index) else {
            return Err(lang
                .pick("覚えた選択範囲がありません。", "No such saved selection.")
                .into());
        };
        let m = &saved.mask;
        if m.width() != self.doc.width()
            || m.height() != self.doc.height()
            || m.tile_size() != self.doc.tile_size()
        {
            return Err(lang
                .pick(
                    "覚えたときと文書の大きさが違います。",
                    "The document size differs from when it was saved.",
                )
                .into());
        }
        Ok(m.clone())
    }

    /// 覚えた選択範囲の操作。
    pub fn sel_saved(&mut self, op: SavedOp) {
        let lang = self.lang;
        match op {
            SavedOp::OpenWindow => {
                let name = self.default_saved_name();
                self.sel.saved_window = Some(SavedWindow {
                    name,
                    focus: false,
                    ..SavedWindow::default()
                });
            }
            SavedOp::CloseWindow => self.sel.saved_window = None,
            SavedOp::Save(name) => {
                if self.is_stroking() {
                    self.message = lang
                        .pick("描いている間はできません。", "Not while drawing.")
                        .into();
                    return;
                }
                let Some(mask) = self.doc.selection().cloned() else {
                    self.message = lang.pick("選択範囲がありません。", "No selection.").into();
                    return;
                };
                let name = name.trim().to_owned();
                let name = if name.is_empty() {
                    self.default_saved_name()
                } else {
                    name
                };
                let uid = self.sets.current().uid;
                let existing = self.saved_selections().iter().position(|s| s.name == name);
                if existing.is_none() && self.saved_selections().len() >= MAX_SAVED {
                    self.message = lang
                        .pick("覚えられる数の上限です。", "Too many saved selections.")
                        .into();
                    return;
                }
                if self.saved_bytes_without(uid, existing) + mask.allocated_bytes()
                    > self.sel.saved_budget
                {
                    self.message = lang
                        .pick(
                            "覚えた選択範囲が大きすぎます。",
                            "The saved selections are too large.",
                        )
                        .into();
                    return;
                }
                let saved = SavedSelection {
                    name: name.clone(),
                    mask,
                };
                let list = self.sel.saved.entry(uid).or_default();
                match existing {
                    Some(i) => list[i] = saved,
                    None => list.push(saved),
                }
                self.message = format!(
                    "{}: {name}",
                    lang.pick("選択範囲を覚えました", "Remembered")
                );
                let next = self.default_saved_name();
                if let Some(win) = self.sel.saved_window.as_mut() {
                    win.name = next;
                }
            }
            SavedOp::Delete(index) => {
                let uid = self.sets.current().uid;
                if let Some(list) = self.sel.saved.get_mut(&uid) {
                    if index < list.len() {
                        let gone = list.remove(index);
                        self.message = format!("{}: {}", lang.pick("削除", "Removed"), gone.name);
                    }
                    if list.is_empty() {
                        self.sel.saved.remove(&uid);
                    }
                }
            }
        }
    }

    /// 覚えた札のバイト数の合計（プロジェクト全体）から、セット `uid` の `skip` 番目（入れ替える札）を除いたもの。
    fn saved_bytes_without(&self, uid: u32, skip: Option<usize>) -> u64 {
        self.sel
            .saved
            .iter()
            .flat_map(|(u, list)| list.iter().enumerate().map(move |(i, s)| (*u, i, s)))
            .filter(|(u, i, _)| !(*u == uid && Some(*i) == skip))
            .map(|(_, _, s)| s.mask.allocated_bytes())
            .sum()
    }

    /// 今のプロジェクトに無いセットの覚えた選択範囲を捨てる（セットを消した・別のプロジェクトを開いた。セットの uid は開き直すたびに
    /// 新しくなるので、残しても呼び出せず、メモリだけが残る）。
    pub fn sel_prune_saved(&mut self) {
        let sets = &self.sets;
        self.sel
            .saved
            .retain(|uid, _| sets.index_of(*uid).is_some());
    }

    /// まだ使っていない既定の名前（選択範囲 1・2・…）。
    fn default_saved_name(&self) -> String {
        let used = self.saved_selections();
        (1..)
            .map(|n| format!("{} {n}", self.lang.pick("選択範囲", "Selection")))
            .find(|name| !used.iter().any(|s| &s.name == name))
            .expect("無限の列")
    }
}

// ───────── 窓 ─────────

const WIDTH: f32 = 380.0;
const ROW: f32 = 28.0;
const VISIBLE_ROWS: usize = 8;

fn window_id() -> Id {
    Id::new("yolu.sel-saved")
}

/// 最後に描いた窓の矩形（開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// 窓の名前（見出し・メニュー）。
pub fn window_title(lang: crate::lang::Lang) -> &'static str {
    lang.pick("覚えた選択範囲", "Remembered Selections")
}

/// 開いていれば窓を描き、押されたものを `Action` として当てる。
pub fn show_window(ctx: &egui::Context, app: &mut AppState) {
    let Some(mut win) = app.sel.saved_window.clone() else {
        return;
    };
    let lang = app.lang;
    let any_selection = app.doc.selection().is_some();
    let can_edit = app.can_edit();
    let rows = app.saved_selections().len();
    let shown = rows.clamp(1, VISIBLE_ROWS);
    let list_h = if rows == 0 { 0.0 } else { shown as f32 * ROW };
    let height = window::HEADER_HEIGHT + 14.0 + 28.0 + 10.0 + list_h + 14.0;
    let keys_free = !ctx.egui_wants_keyboard_input();
    let esc = keys_free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let spec = Spec {
        title: window_title(lang),
        icon: Some("save"),
        size: vec2(WIDTH, height),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let id = window_id();
    let mut offset = win.offset;
    let mut actions: Vec<Action> = Vec::new();
    let list: Vec<(String, bool)> = app
        .saved_selections()
        .iter()
        .map(|s| {
            let fits = s.mask.width() == app.doc.width()
                && s.mask.height() == app.doc.height()
                && s.mask.tile_size() == app.doc.tile_size();
            (s.name.clone(), fits)
        })
        .collect();
    let closed = window::show(ctx, id, &spec, &mut offset, esc, |ui, frame| {
        let left = frame.body.left() + 14.0;
        let width = frame.body.width() - 28.0;
        let top = frame.body.top() + 14.0;
        // 名前の入力欄と「保存」
        let save_w = 84.0;
        let field = Rect::from_min_size(pos2(left, top), vec2(width - save_w - 8.0, 28.0));
        let out = w::text_field(
            ui,
            field,
            id.with("name"),
            &win.name,
            Some(lang.pick("選択範囲の名前", "Selection name")),
            win.focus,
        );
        win.focus = false;
        if let Some(name) = out.committed {
            win.name = name;
        }
        let save_rect = Rect::from_min_size(pos2(field.right() + 8.0, top), vec2(save_w, 28.0));
        if w::button(
            ui,
            save_rect,
            id.with("save"),
            lang.pick("覚える", "Remember"),
            true,
            any_selection && !app.is_stroking(),
            Some(if any_selection {
                lang.pick(
                    "今の選択範囲を、この名前で覚える（このセッションの間だけ。プロジェクトには保存されません）",
                    "Remember the current selection under this name (this session only; it is not saved in the project)",
                )
            } else {
                lang.pick("選択範囲なし", "No selection")
            }),
            None,
        )
        .clicked()
        {
            actions.push(Action::Sel(SelAction::Saved(SavedOp::Save(
                win.name.clone(),
            ))));
        }
        // 一覧（手で動かすスクロール。矩形の外は描かず、押せない）
        let list_rect = Rect::from_min_size(pos2(left, top + 28.0 + 10.0), vec2(width, list_h));
        if list.is_empty() {
            return;
        }
        let content_h = list.len() as f32 * ROW;
        let max_scroll = (content_h - list_h).max(0.0);
        if ui.rect_contains_pointer(list_rect) {
            win.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        win.scroll = win.scroll.clamp(0.0, max_scroll);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        child.set_clip_rect(list_rect.intersect(ui.clip_rect()));
        let ui = &mut child;
        for (i, (name, fits)) in list.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(
                    list_rect.left(),
                    list_rect.top() + i as f32 * ROW - win.scroll,
                ),
                vec2(width, ROW),
            );
            if row.bottom() < list_rect.top() || row.top() > list_rect.bottom() {
                continue;
            }
            let buttons = (COMBINES.len() + 1) as f32 * 28.0 + 4.0;
            let label = Rect::from_min_size(row.min, vec2(width - buttons - 6.0, ROW));
            let p = ui.painter().clone();
            let shown = w::fit(&p, name, label.width() - 4.0, t::LABEL);
            w::text(
                &p,
                label,
                &shown,
                t::LABEL.with_color(if *fits { t::TEXT } else { t::TEXT_DISABLED }),
                Align::Left,
            );
            let mut x = row.right() - buttons;
            for mode in COMBINES {
                let at = Rect::from_min_size(pos2(x, row.top()), vec2(28.0, ROW));
                let tip = format!("{}: {name}", combine_name(lang, mode));
                if w::icon_button(
                    ui,
                    at,
                    (id, "recall", i, mode),
                    combine_icon(mode),
                    &tip,
                    false,
                    *fits && can_edit,
                    16.0,
                )
                .clicked()
                {
                    actions.push(Action::Sel(SelAction::Edit(SelEdit::Recall {
                        index: i,
                        mode,
                    })));
                }
                x += 28.0;
            }
            x += 4.0;
            let at = Rect::from_min_size(pos2(x, row.top()), vec2(28.0, ROW));
            let tip = format!("{} {name}", lang.pick("消す:", "Remove:"));
            if w::icon_button(ui, at, (id, "delete", i), "delete", &tip, false, true, 16.0)
                .clicked()
            {
                actions.push(Action::Sel(SelAction::Saved(SavedOp::Delete(i))));
            }
        }
    });
    win.offset = offset;
    // 動かした窓の状態を戻す（操作で閉じた・窓を替えたあとは上書きしない）
    if app.sel.saved_window.is_some() {
        app.sel.saved_window = Some(win);
    }
    for a in actions {
        app.apply(a);
    }
    if closed {
        app.apply(Action::Sel(SelAction::Saved(SavedOp::CloseWindow)));
    }
}
