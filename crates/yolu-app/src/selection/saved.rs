//! 名前を付けて文書に残した選択範囲。今のテクスチャセット
//! （文書）ごとに、名前と選択範囲の札を持ち、呼び出すときは新規・追加・削除・共通のどれかで今の選択範囲と組み合わせる（1 回の Undo）。
//!
//! 持ち主は core の `Document`（`save_selection`・`rename_saved_selection`・`delete_saved_selection`。どれも 1 回の Undo で、キャンバスの大きさを
//! 変える操作は残した選択範囲も作り直す）。.ylp には使う文書だけを形式 8 で保存する（`sets/<ID>/selections.json`。`io`）。ここは画面と、
//! 全体の予算（`SAVED_BUDGET_BYTES`。プロジェクト全体の合計）の確かめだけ。

use egui::{pos2, vec2, Id, Key, Rect, Vec2};

use super::{combine_name, SelAction, SelEdit};
use crate::engine::{SelectionCombine, SelectionMask};
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// 1 つの文書に残せる数（core の上限）と、残した選択範囲のバイト数の合計（プロジェクト全体）の上限。
pub use yolu_core::{SavedSelection, MAX_SAVED_SELECTIONS as MAX_SAVED};
pub const SAVED_BUDGET_BYTES: u64 = 256 << 20;

/// ウィンドウ（開いていれば）の状態。
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
    /// 名前を変えている行（並びの番号）と、その欄に初めのフォーカスを渡したか。
    pub rename: Option<usize>,
    pub rename_started: bool,
}

/// 覚えた選択範囲の操作（画面と覚えた一覧だけ。文書は変えない）。
#[derive(Clone, Debug, PartialEq)]
pub enum SavedOp {
    OpenWindow,
    CloseWindow,
    /// 今の選択範囲を名前を付けて残す（同じ名前があれば入れ替える。1 回の Undo）。
    Save(String),
    /// 消す（1 回の Undo）。
    Delete(usize),
    /// 名前を変える（1 回の Undo。ほかの選択範囲と同じ名前は断る）。
    Rename {
        index: usize,
        name: String,
    },
}

impl SavedOp {
    /// 文書（残した選択範囲）を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        matches!(self, Self::Save(_) | Self::Delete(_) | Self::Rename { .. })
    }
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
    /// 今の文書に残した選択範囲。
    pub fn saved_selections(&self) -> &[SavedSelection] {
        self.doc.saved_selections()
    }

    /// 残した選択範囲を呼び出すための札（今の文書と大きさが合うときだけ。大きさを変える操作は残した選択範囲も作り直すので、
    /// 合わないのは読み込みの不整合だけ）。
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
                    "覚えたときとキャンバスの大きさが違います。",
                    "The canvas size differs from when it was saved.",
                )
                .into());
        }
        Ok(m.clone())
    }

    /// 残した選択範囲の操作。
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
                if self.refuse_while_stroking() {
                    return;
                }
                let Some(mask) = self.doc.selection().cloned() else {
                    self.refuse(
                        Source::Selection,
                        lang.pick("選択範囲がありません。", "No selection."),
                    );
                    return;
                };
                let name = name.trim().to_owned();
                let name = if name.is_empty() {
                    self.default_saved_name()
                } else {
                    name
                };
                let existing = self.saved_selections().iter().position(|s| s.name == name);
                if existing.is_none() && self.saved_selections().len() >= MAX_SAVED {
                    self.refuse(
                        Source::Selection,
                        lang.pick("覚えられる数の上限です。", "Too many saved selections."),
                    );
                    return;
                }
                if self.saved_bytes_without(existing) + mask.allocated_bytes()
                    > self.sel.saved_budget
                {
                    self.refuse(
                        Source::Selection,
                        lang.pick(
                            "覚えた選択範囲が大きすぎます。",
                            "The saved selections are too large.",
                        ),
                    );
                    return;
                }
                let revision = self.doc.revision();
                match self.doc.save_selection(&name) {
                    Ok(_) => {
                        self.info(
                            Source::Selection,
                            format!(
                                "{}: {name}",
                                lang.pick("選択範囲を覚えました", "Remembered")
                            ),
                        );
                        self.modified |= self.doc.revision() != revision;
                    }
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Selection,
                        self.lang.core_error(&e),
                    ),
                }
                let next = self.default_saved_name();
                if let Some(win) = self.sel.saved_window.as_mut() {
                    win.name = next;
                }
            }
            SavedOp::Delete(index) => {
                if self.refuse_while_stroking() {
                    return;
                }
                let Some(gone) = self.saved_selections().get(index).map(|s| s.name.clone()) else {
                    return;
                };
                match self.doc.delete_saved_selection(index) {
                    Ok(()) => {
                        self.info(
                            Source::Selection,
                            format!("{}: {gone}", lang.pick("削除", "Removed")),
                        );
                        self.modified = true;
                        if let Some(win) = self.sel.saved_window.as_mut() {
                            // 消した行より後ろの名前の変更は、番号が 1 つずれる
                            match win.rename {
                                Some(r) if r == index => win.rename = None,
                                Some(r) if r > index => win.rename = Some(r - 1),
                                _ => {}
                            }
                        }
                    }
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Selection,
                        self.lang.core_error(&e),
                    ),
                }
            }
            SavedOp::Rename { index, name } => {
                if self.refuse_while_stroking() {
                    return;
                }
                let Some(current) = self.saved_selections().get(index).map(|s| s.name.clone())
                else {
                    return;
                };
                let name = name.trim().to_owned();
                if name == current {
                    return;
                }
                match self.doc.rename_saved_selection(index, &name) {
                    Ok(()) => {
                        self.info(
                            Source::Selection,
                            format!("{}: {name}", lang.pick("名前を変えました", "Renamed")),
                        );
                        self.modified = true;
                    }
                    Err(e) => self.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Selection,
                        self.lang.core_error(&e),
                    ),
                }
            }
        }
    }

    /// 描いている間はできない（知らせて true）。
    fn refuse_while_stroking(&mut self) -> bool {
        if self.is_stroking() {
            self.refuse(
                Source::Selection,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return true;
        }
        false
    }

    /// 残した選択範囲のバイト数の合計（プロジェクト全体）から、今のセットの `skip` 番目（入れ替える選択範囲）を除いたもの。
    fn saved_bytes_without(&self, skip: Option<usize>) -> u64 {
        let current = self.sets.current().uid;
        (0..self.sets.len())
            .flat_map(|i| {
                let uid = self.sets.get(i).map(|s| s.uid);
                let skip = (uid == Some(current)).then_some(skip).flatten();
                self.set_doc(i)
                    .saved_selections()
                    .iter()
                    .enumerate()
                    .filter(move |(n, _)| Some(*n) != skip)
                    .map(|(_, s)| s.mask.allocated_bytes())
                    .collect::<Vec<_>>()
            })
            .sum()
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

// ───────── ウィンドウ ─────────

const WIDTH: f32 = 420.0;
const ROW: f32 = 28.0;
const VISIBLE_ROWS: usize = 8;

fn window_id() -> Id {
    Id::new("yolu.sel-saved")
}

/// 最後に描いたウィンドウの矩形（開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// ウィンドウの名前（見出し・メニュー）。
pub fn window_title(lang: crate::lang::Lang) -> &'static str {
    lang.pick("覚えた選択範囲", "Remembered Selections")
}

/// 開いていればウィンドウを描き、押されたものを `Action` として当てる。
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
    // 名前を変えている間の Esc は名前の欄のもの（欄がやめて元の表示へ戻る。ウィンドウは閉じない）
    let esc_pressed =
        keys_free && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let esc = esc_pressed && win.rename.is_none();
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
    let mut rename_op: Option<(usize, String)> = None;
    let mut start_rename: Option<usize> = None;
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
                    "今の選択範囲を、この名前で覚える",
                    "Remember the current selection under this name",
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
        let bar = Scroll::begin(ui, list_rect, content_h, &mut win.scroll);
        let width = width - bar.reserved();
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
            let buttons = (COMBINES.len() + 2) as f32 * 28.0 + 4.0;
            let label = Rect::from_min_size(row.min, vec2(width - buttons - 6.0, ROW));
            let p = ui.painter().clone();
            if win.rename == Some(i) {
                // 名前を変えている行: 名前の欄（Enter か外を押して決める。Esc でやめる）
                let first = !win.rename_started;
                win.rename_started = true;
                let out = w::text_field(
                    ui,
                    label.shrink2(vec2(0.0, 2.0)),
                    (id, "rename", i),
                    name,
                    None,
                    first,
                );
                if let Some(next) = out.committed {
                    rename_op = Some((i, next));
                }
                if !first && !out.focused {
                    win.rename = None;
                }
            } else {
                let shown = w::fit(&p, name, label.width() - 4.0, t::LABEL);
                w::text(
                    &p,
                    label,
                    &shown,
                    t::LABEL.with_color(if *fits { t::TEXT } else { t::TEXT_DISABLED }),
                    Align::Left,
                );
            }
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
            let tip = format!("{}: {name}", lang.pick("名前を変える", "Rename"));
            if w::icon_button(
                ui,
                at,
                (id, "rename-button", i),
                "edit",
                &tip,
                win.rename == Some(i),
                can_edit,
                16.0,
            )
            .clicked()
            {
                start_rename = Some(i);
            }
            x += 28.0;
            let at = Rect::from_min_size(pos2(x, row.top()), vec2(28.0, ROW));
            let tip = format!("{} {name}", lang.pick("消す:", "Remove:"));
            if w::icon_button(
                ui,
                at,
                (id, "delete", i),
                "delete",
                &tip,
                false,
                can_edit,
                16.0,
            )
            .clicked()
            {
                actions.push(Action::Sel(SelAction::Saved(SavedOp::Delete(i))));
            }
        }
        bar.end(ui, (id, "scroll"), &mut win.scroll);
    });
    win.offset = offset;
    if let Some(i) = start_rename {
        win.rename = Some(i);
        win.rename_started = false;
    }
    if let Some((index, name)) = rename_op {
        actions.push(Action::Sel(SelAction::Saved(SavedOp::Rename {
            index,
            name,
        })));
    }
    // 動かしたウィンドウの状態を戻す（操作で閉じた・ウィンドウを替えたあとは上書きしない）
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
