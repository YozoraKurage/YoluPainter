//! アセットのパネル（Substance のシェルフ）: 置き場（このプロジェクトの棚・個人のライブラリ）を切り替えて、素材（画像・ブラシ・マテリアル・
//! スマートマテリアル・スマートマスク）をサムネイルの格子で並べる。上に置き場の切り替え、種類の絞り込み（すべて・5 種類のアイコン）と
//! ファイルの読み込み、名前の検索、選んだレイヤーの保存（レイヤー・マスク。棚のとき）、下に選んだ素材の状態（置けないときは短い理由）と操作
//! （置く・書き出す・ライブラリへ入れる・プロジェクトで使う・消す）。格子の素材はダブルクリック・右クリック・ドラッグでレイヤーの
//! パネルへ置く（スマートマテリアルは落とした行の間・グループの中、スマートマスクは落とした行のレイヤーのマスク）。
//! サムネイルと項目の情報は別のスレッドで作り（見えている項目だけ頼み、できた分から出す）、描いている間も画面を止めない。
//! 画面には名前と状態と短い理由だけを出し、説明はツールチップに置く。

use std::path::PathBuf;

use egui::{
    pos2, vec2, Color32, DragAndDrop, Id, Rect, Sense, TextureHandle, Ui, WidgetInfo, WidgetType,
};

use crate::brushes::{sample::SampleSpec, BrushAction, BrushKey};
use crate::dialog::places::Place;
use crate::engine::Document;
use crate::lang::Lang;
use crate::library::{self, Source};
use crate::m2::{DropTarget, Row};
use crate::notice::Source as NoticeSource;
use crate::panels::layers::ROW_HEIGHT;
use crate::shelf::{self, ItemKind, PlaceTarget, ShelfDrag, ShelfOp};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, Entry, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const CELL_W: f32 = 88.0;
pub const CELL_H: f32 = 92.0;
pub const GAP: f32 = 6.0;
pub const THUMB_BOX: f32 = 64.0;
/// 縦のスクロールバーに空ける幅（共通のスクロールの部品の、掴める幅）。
pub const BAR_W: f32 = crate::ui::scroll::BAR_WIDTH;
/// 下の名前と操作の帯の高さ。
pub const FOOTER_H: f32 = 62.0;
/// 見えている行の上下に余分に頼む行の数（スクロールの先読み）。
const AHEAD_ROWS: usize = 1;

/// 格子に並べる 1 つ。
struct Card {
    id: String,
    name: String,
    kind: ItemKind,
    detail: String,
    /// 置けない理由（短い文）。
    block: Option<String>,
    /// 素材の中身のせいで置けない（印を付ける）。
    warn: bool,
    /// 同梱の素材（棚に入っていない。消せない・書き出せない）。
    builtin: bool,
    thumb: Option<TextureHandle>,
    /// 個人のライブラリの項目（ファイル・利用者のブラシ）。棚の項目ではない。
    library: bool,
    /// ライブラリのファイルが、いまのプロジェクトの棚にも（同じ中身で）ある。
    in_project: bool,
    /// 利用者のブラシ（絵は見本のストローク。設定は描くときに取り出す）。
    brush: Option<BrushKey>,
    /// ツールチップに添える置き場所（ライブラリの相対パス）。
    place: Option<String>,
}

fn pending_label(app: &AppState) -> Option<(&'static str, String)> {
    let lang = app.lang;
    if let Some(name) = app.shelf.saving_name() {
        return Some((app.shelf.saving_label(lang), name.to_owned()));
    }
    app.library
        .writing_name()
        .map(|name| (lang.pick("書き込み中", "Writing"), name.to_owned()))
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    app.shelf.use_language(lang);
    app.library.set_context(&ctx);
    let source = app.library.source;
    // 項目の情報とサムネイルは別のスレッドで作る。できた分を受け取り、見えている項目の分は格子を描くときに頼む
    match source {
        Source::Project => app.shelf.begin_inspection_frame(&ctx),
        Source::Library => {
            let folder = app.library_root();
            app.library.prepare(folder);
            app.library.begin_frame();
            app.library.poll();
        }
    }
    // できたときは、別のスレッドが描き直しを頼む（こちらから見に来続けない。受け取りは `frame`）

    let mut rows = w::Rows::new(r, 6.0);
    // 置き場の切り替え（左）と、ライブラリの読み直し・フォルダを開く（右）
    let row = rows.row(24.0, 4.0);
    let tools = if source == Source::Library {
        2.0 * 28.0
    } else {
        0.0
    };
    let width = ((row.width() - tools - 4.0 - 4.0) / 2.0).clamp(40.0, 110.0);
    for (i, s) in Source::ALL.into_iter().enumerate() {
        let b = Rect::from_min_size(
            pos2(row.left() + i as f32 * (width + 4.0), row.top()),
            vec2(width, row.height()),
        );
        if w::button(
            ui,
            b,
            ("shelf.source", i),
            s.name(lang),
            s == source,
            true,
            Some(s.tooltip(lang)),
            None,
        )
        .clicked()
            && s != source
        {
            app.library.source = s;
            app.shelf.scroll = 0.0;
            if s == Source::Library {
                app.library.refresh();
            }
        }
    }
    if source == Source::Library {
        let refresh = Rect::from_min_size(
            pos2(row.right() - 54.0, row.top()),
            vec2(26.0, row.height()),
        );
        if w::icon_button(
            ui,
            refresh,
            "shelf.library.refresh",
            "sync",
            lang.pick("一覧を読み直す", "Reload the list"),
            false,
            true,
            16.0,
        )
        .clicked()
        {
            app.apply(Action::Shelf(ShelfOp::LibraryRefresh));
        }
        let reveal = Rect::from_min_size(
            pos2(row.right() - 26.0, row.top()),
            vec2(26.0, row.height()),
        );
        if w::icon_button(
            ui,
            reveal,
            "shelf.library.reveal",
            "folder_open",
            lang.pick("ライブラリのフォルダを開く", "Open the library folder"),
            false,
            app.library_root().is_some(),
            16.0,
        )
        .clicked()
        {
            app.apply(Action::Shelf(ShelfOp::LibraryReveal));
        }
    }
    // 種類の絞り込み（左）と、ファイルの読み込み（右）
    let row = rows.row(26.0, 4.0);
    let mut x = row.left();
    let mut filters: Vec<(Option<ItemKind>, &str, &str)> =
        vec![(None, "grid_dots", lang.pick("すべて", "All"))];
    for kind in ItemKind::ALL {
        filters.push((Some(kind), kind.icon(), kind.name(lang)));
    }
    for (kind, icon, name) in filters {
        let b = Rect::from_min_size(pos2(x, row.top()), vec2(26.0, row.height()));
        x += 28.0;
        let id = ("shelf.filter", kind.map_or(0, |k| k as u8 + 1));
        if w::icon_button(ui, b, id, icon, name, app.shelf.filter == kind, true, 16.0).clicked() {
            app.shelf.filter = kind;
            app.shelf.scroll = 0.0;
        }
    }
    let import = Rect::from_min_size(
        pos2(row.right() - 26.0, row.top()),
        vec2(26.0, row.height()),
    );
    match source {
        Source::Project => {
            if w::icon_button(
                ui,
                import,
                "shelf.import",
                "import",
                lang.pick(".ylsmart を読み込む…", "Import .ylsmart…"),
                false,
                app.shelf.unavailable.is_none(),
                16.0,
            )
            .clicked()
            {
                app.apply(Action::Shelf(ShelfOp::ImportDialog));
            }
        }
        Source::Library => {
            if w::icon_button(
                ui,
                import,
                "shelf.library.add",
                "add",
                lang.pick(
                    "ライブラリへファイルを追加…（PNG・.ylsmart）",
                    "Add files to the library… (PNG, .ylsmart)",
                ),
                false,
                app.library_root().is_some(),
                16.0,
            )
            .clicked()
            {
                app.apply(Action::Shelf(ShelfOp::LibraryAddDialog));
            }
        }
    }
    // 名前の検索
    let row = rows.row(24.0, 6.0);
    if search_field(
        ui,
        row,
        "shelf.search",
        &mut app.shelf.search,
        lang.pick("検索", "Search"),
    ) {
        app.shelf.scroll = 0.0;
    }
    // 別のスレッドで走っている仕事（レイヤーの保存・ライブラリのファイルの取り込み・ライブラリへの書き込み）の名前と「やめる」。
    // 走っていないときは、棚では選んだレイヤーの保存
    let pending = pending_label(app);
    if pending.is_some() || source == Source::Project {
        let row = rows.row(24.0, 6.0);
        if let Some((label, name)) = pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            let cancel = Rect::from_min_size(
                pos2(row.right() - 72.0, row.top()),
                vec2(72.0, row.height()),
            );
            let text_rect = Rect::from_min_max(row.min, pos2(cancel.left() - 6.0, row.bottom()));
            let p = ui.painter().clone();
            let text = format!("{label}: {name}");
            let shown = w::fit(&p, &text, text_rect.width(), t::LABEL_DIM);
            w::text(&p, text_rect, &shown, t::LABEL_DIM, Align::Left);
            if w::button(
                ui,
                cancel,
                "shelf.save.cancel",
                lang.pick("やめる", "Cancel"),
                false,
                true,
                None,
                None,
            )
            .clicked()
            {
                app.apply(Action::Shelf(ShelfOp::CancelSave));
            }
        } else {
            save_buttons(ui, app, row);
        }
    }

    let top = rows.y();
    let grid = Rect::from_min_max(
        pos2(r.left() + t::PADDING, top),
        pos2(r.right() - t::PADDING, (r.bottom() - FOOTER_H).max(top)),
    );
    app.library.grid_rect = (source == Source::Library).then_some(grid);
    cards(ui, app, &ctx, grid);
    footer(
        ui,
        app,
        Rect::from_min_max(
            pos2(r.left() + t::PADDING, grid.bottom() + 6.0),
            pos2(
                r.right() - t::PADDING,
                (r.bottom() - 8.0).max(grid.bottom() + 6.0),
            ),
        ),
    );
    ghost(&ctx);
}

/// 選んだレイヤーの保存のボタン（レイヤー・マスク）。
fn save_buttons(ui: &mut Ui, app: &mut AppState, row: Rect) {
    let lang = app.lang;
    let halves = w::Rows::split(row, 2, 6.0);
    let layer = app
        .selected_layer
        .and_then(|id| app.doc.layer(id).map(|l| (id, l.mask().is_some())));
    let can_save = layer.is_some() && app.can_edit() && app.shelf.unavailable.is_none();
    if w::button(
        ui,
        halves[0],
        "shelf.save.material",
        lang.pick("レイヤーを保存", "Save Layer"),
        false,
        can_save,
        Some(lang.pick(
            "選んでいるレイヤー（グループなら中身ごと）をスマートマテリアルとしてアセットに入れる",
            "Put the selected layer (with its contents, for a group) into the project's assets as a smart material",
        )),
        None,
    )
    .clicked()
    {
        if let Some((id, _)) = layer {
            app.apply(Action::Shelf(ShelfOp::SaveMaterial(id)));
        }
    }
    if w::button(
        ui,
        halves[1],
        "shelf.save.mask",
        lang.pick("マスクを保存", "Save Mask"),
        false,
        can_save && layer.is_some_and(|(_, has_mask)| has_mask),
        Some(lang.pick(
            "選んでいるレイヤーのマスクをスマートマスクとしてアセットに入れる",
            "Put the selected layer's mask into the project's assets as a smart mask",
        )),
        None,
    )
    .clicked()
    {
        if let Some((id, _)) = layer {
            app.apply(Action::Shelf(ShelfOp::SaveMask(id)));
        }
    }
}

/// 名前の検索の欄（打つたびに絞る）。変えたら true。`id_salt` は欄ごとに変える（同じ画面に並んでも入力が混ざらない）。
pub(crate) fn search_field(
    ui: &mut Ui,
    r: Rect,
    id_salt: &'static str,
    text: &mut String,
    hint: &str,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let had_focus = ui.memory(|m| m.has_focus(id));
    let hover = ui.rect_contains_pointer(r);
    {
        let p = ui.painter();
        w::rounded(p, r, t::CONTROL_BG, 3.0);
        w::outline(
            p,
            r,
            if had_focus {
                t::ACCENT
            } else if hover {
                t::ACCENT_DIM
            } else {
                t::BORDER
            },
            1.0,
            3.0,
        );
        w::icon(
            p,
            Rect::from_min_size(pos2(r.left() + 3.0, r.top()), vec2(20.0, r.height())),
            "search",
            t::TEXT_DIM,
            14.0,
        );
    }
    let clear_w = if text.is_empty() { 0.0 } else { 22.0 };
    let inner = Rect::from_min_max(
        pos2(r.left() + 26.0, r.top() + 1.0),
        pos2(r.right() - 6.0 - clear_w, r.bottom() - 1.0),
    );
    let before = text.clone();
    ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(t::LABEL.font())
            .text_color(t::TEXT)
            .hint_text(
                egui::RichText::new(hint)
                    .color(t::TEXT_DIM)
                    .font(t::LABEL.font()),
            )
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center),
    );
    if !text.is_empty() {
        let b = Rect::from_min_size(pos2(r.right() - 22.0, r.top()), vec2(20.0, r.height()));
        if w::icon_button(ui, b, (id_salt, "clear"), "close", "×", false, true, 12.0).clicked() {
            text.clear();
        }
    }
    *text != before
}

/// 格子の寸法。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridMetrics {
    pub columns: usize,
    /// 1 枚の幅（列の幅をバーの分を除いて均等に割る）。
    pub cell_w: f32,
    /// 全体の高さ（実際の列数での行数から）。
    pub content: f32,
    pub max_scroll: f32,
}

/// 幅 `width`・高さ `height` の格子に `count` 枚を並べる寸法。スクロールバーが出ると幅が狭まって列数が減り、行数が増えて
/// 全体の高さも変わるので、バー無しの寸法で高さに収まるかを見てバーの有無を決め、あるときはバー分を引いた幅で列数・行数・
/// 全体の高さ・最大のスクロールを求め直す（バー無しの行数を持ち越すと、列数が減る幅で最後の段へ届かない）。
pub fn grid_metrics(width: f32, height: f32, count: usize) -> GridMetrics {
    let at = |usable: f32| {
        let columns = (((usable - GAP) / (CELL_W + GAP)).floor() as usize).max(1);
        let content = GAP + count.div_ceil(columns) as f32 * (CELL_H + GAP);
        (usable, columns, content)
    };
    let mut laid = at(width);
    if laid.2 > height {
        laid = at(width - BAR_W);
    }
    let (usable, columns, content) = laid;
    GridMetrics {
        columns,
        cell_w: (usable - GAP * (columns as f32 + 1.0)) / columns as f32,
        content,
        max_scroll: (content - height).max(0.0),
    }
}

/// 棚（プロジェクト）の項目の格子の 1 つ分（棚の並びのまま。組み込みの素材も）。
fn project_cards(app: &mut AppState, ctx: &egui::Context) -> Vec<Card> {
    let lang = app.lang;
    let ids: Vec<String> = app.shelf.visible().iter().map(|r| r.id.clone()).collect();
    let mut list: Vec<Card> = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(res) = app.shelf.get(&id) else {
            continue;
        };
        let Some(kind) = ItemKind::of(&res.kind) else {
            continue;
        };
        let name = res.name.clone();
        let detail = app.shelf.detail(lang, res);
        let block = app.shelf.block_of(&id).map(|b| b.reason(lang));
        let warn = app.shelf.warns(&id);
        let thumb = app.shelf.texture(ctx, &id);
        let builtin = shelf::is_builtin(&id);
        list.push(Card {
            id,
            name,
            kind,
            detail,
            block,
            warn,
            builtin,
            thumb,
            library: false,
            in_project: false,
            brush: None,
            place: None,
        });
    }
    list
}

/// 個人のライブラリの項目（フォルダのファイルと、利用者のブラシ）の格子の 1 つ分。
fn library_cards(app: &mut AppState, ctx: &egui::Context) -> Vec<Card> {
    let lang = app.lang;
    let filter = app.shelf.filter;
    let search = app.shelf.search.clone();
    let items = app.library.visible(filter, &search);
    let project = app.library.project_index(app.shelf.shelf());
    let mut list: Vec<Card> = Vec::with_capacity(items.len());
    for item in items {
        let info = app.library.info(&item.rel);
        let block = info.and_then(|i| i.inspected.block.clone());
        let detail = info
            .map(|i| {
                shelf::describe(
                    lang,
                    item.kind,
                    Some(&i.inspected),
                    (i.inspected.width, i.inspected.height),
                    false,
                )
            })
            .unwrap_or_default();
        let content = info.map_or("", |i| i.content.as_str());
        let in_project = app
            .library
            .entry(&item.rel)
            .is_some_and(|e| project.contains(e.kind, &item.rel, content));
        let thumb = app.library.texture(ctx, &item.rel);
        list.push(Card {
            warn: block
                .as_ref()
                .is_some_and(|b| !matches!(b, shelf::Block::Kind(_))),
            block: block.map(|b| b.reason(lang)),
            id: item.id,
            name: item.name,
            kind: item.kind,
            detail,
            builtin: false,
            thumb,
            library: true,
            in_project,
            brush: None,
            place: Some(item.rel),
        });
    }
    // 利用者のブラシ（今の brushes/ をそのまま。ライブラリのフォルダへは写さない）
    if filter.is_none_or(|f| f == ItemKind::Brush) {
        let needle = search.trim().to_lowercase();
        for entry in app.brushes.lib.entries() {
            if !entry.key.is_user()
                || !(needle.is_empty() || entry.name.to_lowercase().contains(&needle))
            {
                continue;
            }
            list.push(Card {
                id: format!("{}{}", library::BRUSH_PREFIX, entry.key.token()),
                name: entry.name.clone(),
                kind: ItemKind::Brush,
                detail: entry.group.name(lang).to_owned(),
                block: None,
                warn: false,
                builtin: false,
                thumb: None,
                library: true,
                in_project: false,
                brush: Some(entry.key),
                place: None,
            });
        }
    }
    list
}

/// 見えている行（上下に `AHEAD_ROWS` 行の余裕）の項目。
fn wanted(list: &[Card], columns: usize, scroll: f32, height: f32) -> &[Card] {
    let step = CELL_H + GAP;
    let first = (((scroll - GAP) / step).floor().max(0.0) as usize).saturating_sub(AHEAD_ROWS);
    let last = ((scroll + height) / step).floor().max(0.0) as usize + AHEAD_ROWS;
    let from = (first * columns).min(list.len());
    let to = ((last + 1) * columns).min(list.len());
    &list[from..to]
}

fn cards(ui: &mut Ui, app: &mut AppState, ctx: &egui::Context, grid: Rect) {
    let lang = app.lang;
    w::rounded(ui.painter(), grid, t::MENU_BG, 4.0);
    let source = app.library.source;
    // 並べる素材（棚の並びのまま。ライブラリはフォルダの並び、ブラシが最後）
    let list = match source {
        Source::Project => project_cards(app, ctx),
        Source::Library => library_cards(app, ctx),
    };
    if list.is_empty() {
        // 空の棚・一致なしは、空の状態の文字（「なし」「一致なし」）を置かず、空のまま。読めないときだけ、その状態を出す
        let unreadable = match source {
            Source::Project => app
                .shelf
                .unavailable
                .clone()
                .map(|r| (lang.pick("アセットを読めません", "Assets unreadable"), r)),
            Source::Library => app
                .library
                .problem()
                .cloned()
                .map(|r| (lang.pick("ライブラリを読めません", "Library unreadable"), r)),
        };
        if let Some((text, reason)) = unreadable {
            let at = Rect::from_min_size(
                pos2(grid.left() + 8.0, grid.top() + 10.0),
                vec2(grid.width() - 16.0, 18.0),
            );
            w::text(
                ui.painter(),
                at,
                text,
                t::LABEL_DIM.with_color(t::WARNING),
                Align::Left,
            );
            ui.interact(at, ui.id().with("shelf.unreadable"), Sense::hover())
                .on_hover_text(reason.reason(lang));
        }
        return;
    }
    let GridMetrics {
        columns,
        cell_w,
        content,
        ..
    } = grid_metrics(grid.width(), grid.height(), list.len());
    let bar = Scroll::begin(ui, grid, content, &mut app.shelf.scroll);
    // 見えている項目（と先読みの行）だけ、情報とサムネイルを別のスレッドへ頼む
    let near = wanted(&list, columns, app.shelf.scroll, grid.height());
    match source {
        Source::Project => {
            let ids: Vec<String> = near.iter().map(|c| c.id.clone()).collect();
            let cache = app.library.cache().cloned();
            app.shelf.request_inspections(&ids, cache.as_ref());
        }
        Source::Library => {
            let rels: Vec<String> = near
                .iter()
                .filter(|c| c.brush.is_none())
                .filter_map(|c| library::rel_of(&c.id).map(str::to_owned))
                .collect();
            app.library.request_probes(&rels);
        }
    }
    let painter = ui.painter_at(grid);
    for (i, card) in list.iter().enumerate() {
        let (cx, cy) = (i % columns, i / columns);
        let cell = Rect::from_min_size(
            pos2(
                grid.left() + GAP + cx as f32 * (cell_w + GAP),
                grid.top() + GAP + cy as f32 * (CELL_H + GAP) - app.shelf.scroll,
            ),
            vec2(cell_w, CELL_H),
        );
        if cell.bottom() < grid.top() || cell.top() > grid.bottom() {
            continue;
        }
        card_cell(ui, app, &painter, grid, cell, card);
    }
    bar.end(ui, "assets.scroll", &mut app.shelf.scroll);
}

/// カードが選ばれているか（棚の項目は棚の選び、ライブラリの項目はライブラリの選び）。
fn is_selected(app: &AppState, card: &Card) -> bool {
    let selected = if card.library {
        app.library.selected.as_deref()
    } else {
        app.shelf.selected.as_deref()
    };
    selected == Some(card.id.as_str())
}

fn select(app: &mut AppState, card: &Card) {
    if card.library {
        app.library.selected = Some(card.id.clone());
    } else {
        app.shelf.selected = Some(card.id.clone());
    }
}

/// カードの主の操作（ダブルクリック）。
fn primary_action(card: &Card) -> Option<Action> {
    if let Some(key) = card.brush {
        return Some(Action::Brush(BrushAction::Select(key)));
    }
    if card.library && card.kind == ItemKind::Brush {
        // ブラシのファイルは、まだ置けない種類。プロジェクトの棚へ入れる
        let rel = library::rel_of(&card.id)?;
        return Some(Action::Shelf(ShelfOp::UseFromLibrary(rel.to_owned())));
    }
    Some(Action::Shelf(ShelfOp::Place {
        id: card.id.clone(),
        target: PlaceTarget::Selected,
    }))
}

fn card_cell(
    ui: &mut Ui,
    app: &mut AppState,
    painter: &egui::Painter,
    grid: Rect,
    cell: Rect,
    card: &Card,
) {
    let selected = is_selected(app, card);
    let hit = cell.intersect(grid);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("shelf.card", &card.id)),
        Sense::click_and_drag(),
    );
    if selected {
        w::rounded(painter, cell, t::ACCENT_DIM, 4.0);
    } else if response.hovered() {
        w::rounded(painter, cell, t::CONTROL_HOVER, 4.0);
    }
    let thumb = Rect::from_min_size(
        pos2(cell.center().x - THUMB_BOX / 2.0, cell.top() + 4.0),
        vec2(THUMB_BOX, THUMB_BOX),
    );
    if let Some(key) = card.brush {
        // 利用者のブラシは、白い紙の上の見本のストローク（見本は別のスレッドで描く）
        let spec = SampleSpec {
            width: 96,
            height: 64,
            eraser: false,
        };
        if let Some(brush) = app.brushes.lib.entry(key).map(|e| e.baseline.clone()) {
            let old = ui.clip_rect();
            ui.set_clip_rect(old.intersect(grid));
            let place = egui::Id::new(("brush.sample.asset", key));
            super::brushes::paint_sample(ui, app, thumb, &brush, spec, place);
            ui.set_clip_rect(old);
        }
    } else {
        w::checker(painter, thumb, 6.0);
        match &card.thumb {
            Some(handle) => {
                let [tw, th] = handle.size();
                let k = (THUMB_BOX / tw as f32).min(THUMB_BOX / th as f32);
                let at = Rect::from_center_size(thumb.center(), vec2(tw as f32 * k, th as f32 * k));
                painter.image(
                    handle.id(),
                    at,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                w::rounded(painter, thumb, t::PANEL_HEADER, 3.0);
                w::icon(painter, thumb, card.kind.icon(), t::TEXT_DIM, 24.0);
            }
        }
    }
    w::outline(painter, thumb, t::BORDER, 1.0, 0.0);
    // 置けないしるし（右上。素材の中身で置けないものだけ。種類で置けないものは下の名前の帯に理由を出す）と、
    // プロジェクトにもあるしるし（ライブラリのファイル）
    if card.warn || card.in_project {
        let badge = Rect::from_min_size(
            pos2(thumb.right() - 16.0, thumb.top() + 2.0),
            vec2(14.0, 14.0),
        );
        w::rounded(painter, badge, t::PANEL_BG, 7.0);
        if card.warn {
            w::icon(painter, badge, "warning", t::WARNING, 11.0);
        } else {
            w::icon(painter, badge, "check", t::ACCENT, 11.0);
        }
    }
    if card.builtin {
        // 組み込みの印（左上。消せない・書き出せない元）
        let badge = Rect::from_min_size(
            pos2(thumb.left() + 2.0, thumb.top() + 2.0),
            vec2(14.0, 14.0),
        );
        w::rounded(painter, badge, t::PANEL_BG, 7.0);
        w::icon(painter, badge, "lock", t::TEXT_DIM, 11.0);
    }
    if card.kind != ItemKind::Image {
        let mark = Rect::from_min_size(
            pos2(thumb.left() + 2.0, thumb.bottom() - 16.0),
            vec2(14.0, 14.0),
        );
        w::rounded(painter, mark, t::PANEL_BG, 3.0);
        w::icon(painter, mark, card.kind.icon(), t::ACCENT, 11.0);
    }
    let label = Rect::from_min_size(
        pos2(cell.left() + 3.0, thumb.bottom() + 2.0),
        vec2(cell.width() - 6.0, CELL_H - THUMB_BOX - 8.0),
    );
    let shown = w::fit(painter, &card.name, label.width(), t::LABEL_SMALL);
    w::text(
        painter,
        label,
        &shown,
        t::LABEL_SMALL.with_color(if selected { Color32::WHITE } else { t::TEXT }),
        Align::Center,
    );
    if response.clicked() || response.drag_started() {
        select(app, card);
    }
    if response.double_clicked() {
        if let Some(action) = primary_action(card) {
            app.apply(action);
        }
    }
    if response.secondary_clicked() {
        select(app, card);
        if let Some(at) = response.interact_pointer_pos() {
            app.popup = Some(OpenPopup {
                kind: PopupKind::Shelf,
                state: PopupState::new(ui.ctx(), context_anchor(at)),
            });
        }
    }
    // 利用者のブラシは、レイヤーへ置けないので、引かない
    if response.drag_started() && card.brush.is_none() {
        response.dnd_set_drag_payload(ShelfDrag {
            id: card.id.clone(),
            kind: card.kind,
            name: card.name.clone(),
        });
    }
    let lang = app.lang;
    let mut tip = card.name.clone();
    if !card.detail.is_empty() {
        tip += &format!("\n{}", card.detail);
    }
    if let Some(place) = &card.place {
        tip += &format!("\n{place}");
    }
    if let Some(reason) = &card.block {
        tip += &format!("\n{reason}");
    }
    if card.builtin {
        tip += &format!("\n{}", lang.pick("組み込み", "Built-in"));
    }
    if card.in_project {
        tip += &format!(
            "\n{}",
            lang.pick("プロジェクトにあります", "Already in the project")
        );
    }
    let name = card.name.clone();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &name));
    let _ = response.on_hover_text(tip);
}

fn footer(ui: &mut Ui, app: &mut AppState, r: Rect) {
    match app.library.source {
        Source::Project => project_footer(ui, app, r),
        Source::Library => library_footer(ui, app, r),
    }
}

fn project_footer(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let info = Rect::from_min_size(r.min, vec2(r.width(), 18.0));
    let buttons = Rect::from_min_size(pos2(r.left(), info.bottom() + 4.0), vec2(r.width(), 26.0));
    let selected = app.shelf.selected_resource().and_then(|res| {
        ItemKind::of(&res.kind).map(|k| {
            (
                res.id.clone(),
                res.name.clone(),
                k,
                app.shelf.detail(lang, res),
            )
        })
    });
    let block = selected
        .as_ref()
        .and_then(|(id, ..)| app.shelf.block_of(id))
        .cloned();
    // 名前と状態（置けないときは短い理由。説明はツールチップ）
    if let Some((_, name, kind, detail)) = &selected {
        let p = ui.painter().clone();
        // 名前は選んだ格子の素材に出ている（繰り返さない）。置けないときだけ、その短い理由を帯に出す。名前と状態はツールチップ
        let full = match &block {
            Some(b) => format!("{name} · {}", b.reason(lang)),
            None if detail.is_empty() => name.clone(),
            None => format!("{name} · {detail}"),
        };
        if let Some(b) = &block {
            let reason = b.reason(lang);
            let shown = w::fit(&p, &reason, info.width(), t::LABEL_DIM);
            w::text(
                &p,
                info,
                &shown,
                t::LABEL_DIM.with_color(t::WARNING),
                Align::Left,
            );
        }
        let tip = format!("{} · {full}", kind.singular(lang));
        ui.interact(info, ui.id().with("shelf.info"), Sense::hover())
            .on_hover_text(tip);
    }
    let kind = selected.as_ref().map(|(_, _, k, _)| *k);
    let id = selected.as_ref().map(|(id, ..)| id.clone());
    let free = !app.is_stroking() && app.can_edit();
    let mask = kind == Some(ItemKind::SmartMask);
    let place_enabled = id.is_some() && block.is_none() && free;
    let place_tip = match (&block, mask) {
        (Some(b), _) => b.reason(lang),
        (None, true) => lang
            .pick(
                "選んでいるレイヤーのマスクをこのマスクに入れ替える（1 回の取り消し）",
                "Replace the selected layer's mask with this one (one undo step)",
            )
            .to_owned(),
        (None, false) => lang
            .pick(
                "選んでいるレイヤーの上に新しいレイヤーとして置く（1 回の取り消し）",
                "Put it above the selected layer as new layers (one undo step)",
            )
            .to_owned(),
    };
    let icons = 3.0 * 28.0;
    let main = Rect::from_min_size(
        buttons.min,
        vec2(buttons.width() - icons - 2.0, buttons.height()),
    );
    let label = if mask {
        lang.pick("マスクに適用", "Apply to Mask")
    } else {
        lang.pick("置く", "Place")
    };
    if w::button(
        ui,
        main,
        "shelf.place",
        label,
        true,
        place_enabled,
        Some(&place_tip),
        None,
    )
    .clicked()
    {
        if let Some(id) = &id {
            app.apply(Action::Shelf(ShelfOp::Place {
                id: id.clone(),
                target: PlaceTarget::Selected,
            }));
        }
    }
    let export = Rect::from_min_size(
        pos2(main.right() + 4.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        export,
        "shelf.export",
        "save",
        lang.pick("書き出す…（.ylsmart）", "Export… (.ylsmart)"),
        false,
        id.as_deref().is_some_and(|i| !shelf::is_builtin(i))
            && kind.is_some_and(ItemKind::is_smart),
        16.0,
    )
    .clicked()
    {
        if let Some(id) = &id {
            app.apply(Action::Shelf(ShelfOp::ExportDialog(id.clone())));
        }
    }
    let to_library = Rect::from_min_size(
        pos2(export.right() + 2.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        to_library,
        "shelf.to-library",
        "library",
        lang.pick("ライブラリへ入れる", "Put into the library"),
        false,
        id.as_deref().is_some_and(|i| !shelf::is_builtin(i))
            && !app.is_stroking()
            && !matches!(block, Some(shelf::Block::Unreadable(_))),
        16.0,
    )
    .clicked()
    {
        if let Some(id) = &id {
            app.apply(Action::Shelf(ShelfOp::ToLibrary(id.clone())));
        }
    }
    let remove = Rect::from_min_size(
        pos2(to_library.right() + 2.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        remove,
        "shelf.remove",
        "delete",
        lang.pick(
            "アセットから消す（置いたレイヤーはそのまま）",
            "Remove from the project's assets (placed layers stay)",
        ),
        false,
        id.as_deref().is_some_and(|i| !shelf::is_builtin(i))
            && !app.is_stroking()
            && app.shelf.unavailable.is_none(),
        16.0,
    )
    .clicked()
    {
        if let Some(id) = &id {
            app.apply(Action::Shelf(ShelfOp::AskRemove(id.clone())));
        }
    }
}

/// ライブラリの下の帯: 選んだ項目の状態（置けない理由。選んでいなければ一覧の状態）と操作（置く・使う・プロジェクトで使う・フォルダを開く・消す）。
fn library_footer(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let info = Rect::from_min_size(r.min, vec2(r.width(), 18.0));
    let buttons = Rect::from_min_size(pos2(r.left(), info.bottom() + 4.0), vec2(r.width(), 26.0));
    let selected = app.library.selected.clone();
    let rel = selected
        .as_deref()
        .and_then(library::rel_of)
        .filter(|rel| app.library.entry(rel).is_some())
        .map(str::to_owned);
    let brush_key = selected
        .as_deref()
        .and_then(|id| id.strip_prefix(library::BRUSH_PREFIX))
        .and_then(BrushKey::parse_token);
    let kind = match (&rel, brush_key) {
        (Some(rel), _) => app.library.entry(rel).map(|e| app.library.kind_of(e)),
        (None, Some(_)) => Some(ItemKind::Brush),
        _ => None,
    };
    let block = rel
        .as_deref()
        .and_then(|rel| app.library.info(rel))
        .and_then(|i| i.inspected.block.clone());
    // 状態: 置けない理由。何も選んでいなければ、一覧の状態（読み飛ばした数・途中までの一覧）
    let p = ui.painter().clone();
    let (text, tip) = match (&block, kind) {
        (Some(b), _) => (Some(b.reason(lang)), None),
        (None, Some(_)) => (None, None),
        (None, None) => list_status(app),
    };
    if let Some(text) = &text {
        let shown = w::fit(&p, text, info.width(), t::LABEL_DIM);
        w::text(
            &p,
            info,
            &shown,
            t::LABEL_DIM.with_color(t::WARNING),
            Align::Left,
        );
        ui.interact(info, ui.id().with("shelf.library.info"), Sense::hover())
            .on_hover_text(tip.unwrap_or_else(|| text.clone()));
    }
    let free = !app.is_stroking();
    let mask = kind == Some(ItemKind::SmartMask);
    let file_kind = rel.is_some().then_some(kind).flatten();
    // 主の操作: 置く（画像・スマート素材・マテリアル）・使う（ブラシのファイルは棚へ入れる。利用者のブラシは今のブラシにする）
    let uses = kind == Some(ItemKind::Brush);
    let (label, tip, enabled, action) = match (&rel, brush_key, kind) {
        (_, Some(key), _) if rel.is_none() => (
            lang.pick("使う", "Use"),
            lang.pick(
                "このブラシを今のブラシにする",
                "Make this the current brush",
            )
            .to_owned(),
            free,
            Some(Action::Brush(BrushAction::Select(key))),
        ),
        (Some(rel), _, _) if uses => (
            lang.pick("使う", "Use"),
            lang.pick(
                "写しをプロジェクトのアセットへ入れる（まだ置けない種類）",
                "Copy it into the project's assets (cannot be placed yet)",
            )
            .to_owned(),
            free && app.shelf.unavailable.is_none()
                && !matches!(block, Some(shelf::Block::Unreadable(_))),
            Some(Action::Shelf(ShelfOp::UseFromLibrary(rel.clone()))),
        ),
        (Some(rel), _, _) => (
            if mask {
                lang.pick("マスクに適用", "Apply to Mask")
            } else {
                lang.pick("置く", "Place")
            },
            match (&block, mask) {
                (Some(b), _) => b.reason(lang),
                (None, true) => lang
                    .pick(
                        "選んでいるレイヤーのマスクをこのマスクに入れ替える（1 回の取り消し）",
                        "Replace the selected layer's mask with this one (one undo step)",
                    )
                    .to_owned(),
                (None, false) => lang
                    .pick(
                        "選んでいるレイヤーの上に新しいレイヤーとして置く（1 回の取り消し）",
                        "Put it above the selected layer as new layers (one undo step)",
                    )
                    .to_owned(),
            },
            free && block.is_none() && app.can_edit() && app.library.info(rel).is_some(),
            Some(Action::Shelf(ShelfOp::Place {
                id: library::library_id(rel),
                target: PlaceTarget::Selected,
            })),
        ),
        _ => (lang.pick("置く", "Place"), String::new(), false, None),
    };
    let icons = 3.0 * 28.0;
    let main = Rect::from_min_size(
        buttons.min,
        vec2(buttons.width() - icons - 2.0, buttons.height()),
    );
    if w::button(
        ui,
        main,
        "shelf.place",
        label,
        true,
        enabled,
        (!tip.is_empty()).then_some(tip.as_str()),
        None,
    )
    .clicked()
    {
        if let Some(action) = action {
            app.apply(action);
        }
    }
    let usable = rel.is_some()
        && free
        && app.shelf.unavailable.is_none()
        && !matches!(block, Some(shelf::Block::Unreadable(_)));
    let use_icon = Rect::from_min_size(
        pos2(main.right() + 4.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        use_icon,
        "shelf.library.use",
        "import",
        lang.pick(
            "プロジェクトで使う（写しを .ylp に入れる）",
            "Use in this project (a copy goes into the .ylp)",
        ),
        false,
        usable,
        16.0,
    )
    .clicked()
    {
        if let Some(rel) = &rel {
            app.apply(Action::Shelf(ShelfOp::UseFromLibrary(rel.clone())));
        }
    }
    let reveal = Rect::from_min_size(
        pos2(use_icon.right() + 2.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        reveal,
        "shelf.library.open",
        "folder_open",
        lang.pick("ライブラリのフォルダを開く", "Open the library folder"),
        false,
        app.library_root().is_some(),
        16.0,
    )
    .clicked()
    {
        app.apply(Action::Shelf(ShelfOp::LibraryReveal));
    }
    let remove = Rect::from_min_size(
        pos2(reveal.right() + 2.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        remove,
        "shelf.library.remove",
        "delete",
        lang.pick(
            "ライブラリから消す（プロジェクトの写しと置いたレイヤーはそのまま）",
            "Remove from the library (project copies and placed layers stay)",
        ),
        false,
        file_kind.is_some() && free,
        16.0,
    )
    .clicked()
    {
        if let Some(rel) = &rel {
            app.apply(Action::Shelf(ShelfOp::LibraryAskRemove(rel.clone())));
        }
    }
}

/// 何も選んでいないときの、ライブラリの一覧の状態（読み飛ばしたファイル・途中までの一覧）。短い文と、ツールチップの全文。
fn list_status(app: &AppState) -> (Option<String>, Option<String>) {
    let lang = app.lang;
    let library = &app.library;
    if library.truncated() {
        return (
            Some(match lang {
                Lang::Ja => format!("先頭の {} 件だけ", yolu_io::library::MAX_ENTRIES),
                Lang::En => format!("First {} only", yolu_io::library::MAX_ENTRIES),
            }),
            None,
        );
    }
    let skipped = library.skipped();
    if skipped.is_empty() {
        return (None, None);
    }
    let text = match lang {
        Lang::Ja => format!("{} 件を読み飛ばしました", skipped.len()),
        Lang::En => format!("Skipped {}", skipped.len()),
    };
    let mut tip = text.clone();
    for s in skipped.iter().take(8) {
        let why = match &s.reason {
            yolu_io::library::SkipReason::Link => lang.pick("リンク", "Link").to_owned(),
            yolu_io::library::SkipReason::Name => {
                lang.pick("名前が使えません", "Name not allowed").to_owned()
            }
            yolu_io::library::SkipReason::Deep => lang.pick("深すぎます", "Too deep").to_owned(),
            yolu_io::library::SkipReason::Unreadable(_) => {
                lang.pick("読めません", "Unreadable").to_owned()
            }
        };
        tip += &format!("\n{}: {why}", s.rel);
    }
    (Some(text), Some(tip))
}

/// 引いている素材の影（ポインタの近くに名前）。
fn ghost(ctx: &egui::Context) {
    let Some(drag) = DragAndDrop::payload::<ShelfDrag>(ctx) else {
        return;
    };
    let Some(at) = ctx.pointer_latest_pos() else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("shelf.ghost"),
    ));
    let width = w::text_width(&painter, &drag.name, t::LABEL).min(180.0) + 34.0;
    let r = Rect::from_min_size(at + vec2(12.0, 10.0), vec2(width, 22.0));
    w::rounded(&painter, r, t::PANEL_HEADER, 4.0);
    w::outline(&painter, r, t::ACCENT, 1.0, 4.0);
    w::icon(
        &painter,
        Rect::from_min_size(r.min, vec2(22.0, r.height())),
        drag.kind.icon(),
        t::ACCENT,
        13.0,
    );
    let shown = w::fit(&painter, &drag.name, width - 30.0, t::LABEL);
    w::text(
        &painter,
        Rect::from_min_max(pos2(r.left() + 22.0, r.top()), r.max),
        &shown,
        t::LABEL,
        Align::Left,
    );
}

// ───────── レイヤーの一覧への落とし先 ─────────

/// 一覧の中の高さ（行の上端からの距離を行の高さで割ったもの）から、落とす先（線かグループの枠）を決める。
pub fn gap_or_group(rows: &[Row], position: f32) -> DropTarget {
    let n = rows.len();
    let index = position.floor().max(0.0) as usize;
    if index >= n {
        return DropTarget::Gap(n);
    }
    let within = position - index as f32;
    if rows[index].is_group && within > 0.3 && within < 0.7 {
        DropTarget::Into(rows[index].id)
    } else if within < 0.5 {
        DropTarget::Gap(index)
    } else {
        DropTarget::Gap(index + 1)
    }
}

/// 落とす先の置き場所（親のグループと、その子の中の位置）。線は、その下の行のレイヤーのすぐ上（同じグループの中）。一番下の線は最上位の
/// 一番下。グループの枠はその中の一番上。
pub fn placement_for(doc: &Document, rows: &[Row], target: DropTarget) -> PlaceTarget {
    match target {
        DropTarget::Into(group) => PlaceTarget::At {
            parent: Some(group),
            position: None,
        },
        DropTarget::Gap(k) => match rows.get(k).and_then(|row| doc.layer(row.id)) {
            None => PlaceTarget::At {
                parent: None,
                position: Some(0),
            },
            Some(layer) => {
                let parent = layer.parent();
                let siblings = doc.children_of(parent).unwrap_or_default();
                PlaceTarget::At {
                    parent,
                    position: siblings
                        .iter()
                        .position(|c| *c == layer.id())
                        .map(|i| i + 1),
                }
            }
        },
    }
}

/// レイヤーの一覧の上で棚の素材を引いているとき、落とす先の印を描き、離したら置く（スマートマテリアル・マテリアル・画像は行の間・
/// グループの中、スマートマスクは行のレイヤーのマスク、それ以外は断る理由をステータスバーへ）。
pub fn layer_list_drop(
    ui: &Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &crate::panels::effect_rows::Layout,
) {
    let ctx = ui.ctx();
    let Some(drag) = DragAndDrop::payload::<ShelfDrag>(ctx) else {
        return;
    };
    let Some(p) = ctx.pointer_latest_pos().filter(|p| list.contains(*p)) else {
        return;
    };
    let released = ui.input(|i| i.pointer.any_released());
    // 一覧の行の高さはレイヤーと効果で違うので、行の数え方は一覧を描いた効果の行の配置から（レイヤーの行の単位に直す）
    let at = p.y - list.top() + app.ui.layer_scroll;
    let position = layout.position_at(at);
    let painter = ui.painter_at(list);
    let frame = |y: f32| {
        Rect::from_min_size(
            pos2(list.left() + 1.0, y + 1.0),
            vec2(list.width() - 2.0, ROW_HEIGHT - 2.0),
        )
    };
    let place = match drag.kind {
        ItemKind::SmartMask => {
            let index = layout.row_at(at).unwrap_or(rows.len());
            rows.get(index).map(|row| {
                let y = list.top() + layout.layer_y(index) - app.ui.layer_scroll;
                w::outline(&painter, frame(y), t::ACCENT, 2.0, 3.0);
                PlaceTarget::Mask(row.id)
            })
        }
        // 画像は 1 枚のペイントのレイヤー、マテリアルは塗りつぶしレイヤーとして、スマートマテリアルと同じ所へ置く
        ItemKind::SmartMaterial | ItemKind::Material | ItemKind::Image => {
            let target = gap_or_group(rows, position);
            match target {
                DropTarget::Gap(gap) => {
                    let y = list.top() + layout.gap_y(gap) - app.ui.layer_scroll;
                    painter.rect_filled(
                        Rect::from_min_size(
                            pos2(list.left() + 4.0, y - 1.0),
                            vec2(list.width() - 8.0, 2.0),
                        ),
                        0.0,
                        t::ACCENT,
                    );
                }
                DropTarget::Into(group) => {
                    if let Some(i) = rows.iter().position(|r| r.id == group) {
                        let y = list.top() + layout.layer_y(i) - app.ui.layer_scroll;
                        w::outline(&painter, frame(y), t::ACCENT, 2.0, 3.0);
                    }
                }
            }
            Some(placement_for(&app.doc, rows, target))
        }
        // 置けない種類は印を出さず、離したときに理由を出す
        _ => Some(PlaceTarget::Selected),
    };
    if released {
        if let Some(target) = place {
            app.apply(Action::Shelf(ShelfOp::Place {
                id: drag.id.clone(),
                target,
            }));
        }
        DragAndDrop::clear_payload(ctx);
    }
}

// ───────── メニュー・ファイルのウィンドウ ─────────

/// 棚の素材の右クリックのメニュー（選んでいる素材）。
pub fn menu_entries(app: &AppState) -> Vec<Entry<Action>> {
    if app.library.source == Source::Library {
        return library_menu_entries(app);
    }
    let lang = app.lang;
    let Some(res) = app.shelf.selected_resource() else {
        return Vec::new();
    };
    let kind = ItemKind::of(&res.kind);
    let id = res.id.clone();
    let free = !app.is_stroking();
    let blocked = app.shelf.block_of(&id).is_some();
    let builtin = shelf::is_builtin(&id);
    let label = if kind == Some(ItemKind::SmartMask) {
        lang.pick("マスクに適用", "Apply to Mask")
    } else {
        lang.pick("置く", "Place")
    };
    vec![
        Entry::item(
            label,
            Action::Shelf(ShelfOp::Place {
                id: id.clone(),
                target: PlaceTarget::Selected,
            }),
        )
        .enabled(free && !blocked && app.can_edit()),
        Entry::Separator,
        Entry::item(
            lang.pick("書き出す…", "Export…"),
            Action::Shelf(ShelfOp::ExportDialog(id.clone())),
        )
        .enabled(kind.is_some_and(ItemKind::is_smart) && !builtin),
        Entry::item(
            lang.pick("ライブラリへ入れる", "Put into the library"),
            Action::Shelf(ShelfOp::ToLibrary(id.clone())),
        )
        .enabled(
            free && !builtin
                && !matches!(app.shelf.block_of(&id), Some(shelf::Block::Unreadable(_))),
        ),
        Entry::item(
            lang.pick("アセットから消す…", "Remove from the project's assets…"),
            Action::Shelf(ShelfOp::AskRemove(id)),
        )
        .enabled(free && app.shelf.unavailable.is_none() && !builtin),
    ]
}

/// ライブラリの項目の右クリックのメニュー（選んでいる項目）。
fn library_menu_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(id) = app.library.selected.clone() else {
        return Vec::new();
    };
    let free = !app.is_stroking();
    if let Some(key) = id
        .strip_prefix(library::BRUSH_PREFIX)
        .and_then(BrushKey::parse_token)
    {
        return vec![Entry::item(
            lang.pick("使う", "Use"),
            Action::Brush(BrushAction::Select(key)),
        )
        .enabled(free)];
    }
    let Some(rel) = library::rel_of(&id).map(str::to_owned) else {
        return Vec::new();
    };
    let Some(entry) = app.library.entry(&rel) else {
        return Vec::new();
    };
    let kind = app.library.kind_of(entry);
    let block = app
        .library
        .info(&rel)
        .and_then(|i| i.inspected.block.clone());
    let unreadable = matches!(block, Some(shelf::Block::Unreadable(_)));
    let uses = kind == ItemKind::Brush;
    let mut items = Vec::new();
    if uses {
        items.push(
            Entry::item(
                lang.pick("使う", "Use"),
                Action::Shelf(ShelfOp::UseFromLibrary(rel.clone())),
            )
            .enabled(free && !unreadable && app.shelf.unavailable.is_none()),
        );
    } else {
        items.push(
            Entry::item(
                if kind == ItemKind::SmartMask {
                    lang.pick("マスクに適用", "Apply to Mask")
                } else {
                    lang.pick("置く", "Place")
                },
                Action::Shelf(ShelfOp::Place {
                    id: id.clone(),
                    target: PlaceTarget::Selected,
                }),
            )
            .enabled(free && block.is_none() && app.can_edit() && app.library.info(&rel).is_some()),
        );
        items.push(
            Entry::item(
                lang.pick("プロジェクトで使う", "Use in this project"),
                Action::Shelf(ShelfOp::UseFromLibrary(rel.clone())),
            )
            .enabled(free && !unreadable && app.shelf.unavailable.is_none()),
        );
    }
    items.push(Entry::Separator);
    items.push(Entry::item(
        lang.pick("ライブラリのフォルダを開く", "Open the library folder"),
        Action::Shelf(ShelfOp::LibraryReveal),
    ));
    items.push(
        Entry::item(
            lang.pick("ライブラリから消す…", "Remove from the library…"),
            Action::Shelf(ShelfOp::LibraryAskRemove(rel)),
        )
        .enabled(free),
    );
    items
}

/// 毎フレーム: 別のスレッドの書き出し・取り込み・ライブラリへの書き込みが終わっていれば結果を入れ、ウィンドウに落としたファイルを入れる
/// （ライブラリの格子の上に落とした PNG と .ylsmart はライブラリへ、そうでない .ylsmart は棚へ。PNG は格子の上だけ
/// （筆先・ステンシルの画像の箱へ落とした PNG は、そちらが取る）。.ylp は `YoluApp` が開く）。
pub fn frame(ctx: &egui::Context, state: &mut AppState) {
    state.shelf.context = Some(ctx.clone());
    // 棚の項目の絵（アセットの欄・塗りつぶしの画像の箱が頼む）は、欄が見えていなくても受け取る
    state.shelf.attach_repaint(ctx);
    state.shelf.poll_inspections(shelf::INSPECTIONS_PER_FRAME);
    state.shelf_poll();
    state.library_poll();
    // 格子の範囲はこのフレームで描いたときだけ入る（前のフレームの分を取り、今のフレームの描画が入れ直す）
    let grid = state.library.grid_rect.take();
    import_dropped(ctx, state, grid);
}

pub(crate) fn import_dropped(ctx: &egui::Context, state: &mut AppState, grid: Option<Rect>) {
    let over_grid = state.library.source == Source::Library
        && grid
            .zip(ctx.input(|i| i.pointer.latest_pos()))
            .is_some_and(|(r, p)| r.contains(p));
    let dropped: Vec<PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| {
                p.extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case("ylsmart")
                        || (over_grid && e.eq_ignore_ascii_case("png"))
                })
            })
            .collect()
    });
    if dropped.is_empty() {
        return;
    }
    if over_grid {
        state.apply(Action::Shelf(ShelfOp::LibraryAddFiles(dropped)));
    } else {
        state.apply(Action::Shelf(ShelfOp::ImportFiles(dropped)));
    }
}

/// ファイルの名前に使えない文字を置き換える。
fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_matches('.').to_owned();
    if s.is_empty() {
        "smart".into()
    } else {
        s
    }
}

/// 棚のファイルのウィンドウ・確認のウィンドウ（ウィンドウを開かない試験では呼ばれない。頼みは `state.dialog_request` に残る）。
pub fn run_dialog(state: &mut AppState, request: DialogRequest) {
    let lang = state.lang;
    match request {
        DialogRequest::ShelfImport => {
            if let Some(paths) = crate::dialog::file(state, Place::Assets)
                .set_title(lang.pick(
                    "スマート素材をアセットへ読み込む",
                    "Import smart assets into the project",
                ))
                .add_filter("YoluPainter Smart", &["ylsmart"])
                .pick_files()
            {
                if let Some(first) = paths.first() {
                    state.note_file_chosen(Place::Assets, first);
                }
                state.apply(Action::Shelf(ShelfOp::ImportFiles(paths)));
            }
        }
        DialogRequest::ShelfExport => {
            let Some(id) = state.shelf.export_id.take() else {
                return;
            };
            let Some(name) = state.shelf.get(&id).map(|r| r.name.clone()) else {
                return;
            };
            if let Some(path) = crate::dialog::file(state, Place::Assets)
                .set_title(lang.pick("スマート素材を書き出す", "Export the smart asset"))
                .add_filter("YoluPainter Smart", &["ylsmart"])
                .set_file_name(format!("{}.ylsmart", file_stem(&name)))
                .save_file()
            {
                state.note_file_chosen(Place::Assets, &path);
                state.apply(Action::Shelf(ShelfOp::ExportFile { id, path }));
            }
        }
        DialogRequest::LibraryAdd => {
            if let Some(paths) = crate::dialog::file(state, Place::Assets)
                .set_title(lang.pick(
                    "ライブラリへ追加するファイル",
                    "Files to add to the library",
                ))
                .add_filter("PNG / YoluPainter Smart", &["png", "ylsmart"])
                .pick_files()
            {
                if let Some(first) = paths.first() {
                    state.note_file_chosen(Place::Assets, first);
                }
                state.apply(Action::Shelf(ShelfOp::LibraryAddFiles(paths)));
            }
        }
        DialogRequest::LibraryRemove => {
            let Some(rel) = state.library.pending_remove.take() else {
                return;
            };
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_owned();
            let yes = crate::dialog::message()
                .set_title("YoluPainter")
                .set_description(match lang {
                    Lang::Ja => format!(
                        "「{name}」をライブラリのフォルダから消しますか？（プロジェクトの中の写しと置いたレイヤーは残ります）"
                    ),
                    Lang::En => format!(
                        "Remove \"{name}\" from the library folder? (Copies inside projects and placed layers stay.)"
                    ),
                })
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Warning)
                .show()
                == rfd::MessageDialogResult::Yes;
            if yes {
                state.apply(Action::Shelf(ShelfOp::LibraryRemove(rel)));
            }
        }
        DialogRequest::LibraryReveal => {
            if let Some(root) = state.library_root() {
                if let Err(e) = library::ops::open_folder(&root) {
                    state.fail(
                        NoticeSource::Library,
                        lang.with_reason(
                            lang.pick("フォルダを開けません", "Cannot open the folder"),
                            lang.file_error(&e),
                        ),
                    );
                }
            }
        }
        DialogRequest::ShelfRemove => {
            let Some(id) = state.shelf.pending_remove.take() else {
                return;
            };
            let Some(name) = state.shelf.get(&id).map(|r| r.name.clone()) else {
                return;
            };
            let yes = crate::dialog::message()
                .set_title("YoluPainter")
                .set_description(match lang {
                    Lang::Ja => format!("「{name}」をアセットから消しますか？"),
                    Lang::En => format!("Remove \"{name}\" from the project's assets?"),
                })
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Warning)
                .show()
                == rfd::MessageDialogResult::Yes;
            if yes {
                state.apply(Action::Shelf(ShelfOp::Remove(id)));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LayerId;

    fn row(id: u128, is_group: bool) -> Row {
        Row {
            id: LayerId(id),
            depth: 0,
            is_group,
        }
    }

    #[test]
    fn gaps_and_group_frames_follow_the_row_thresholds() {
        let rows = [row(3, false), row(2, true), row(1, false)];
        assert_eq!(gap_or_group(&rows, 0.1), DropTarget::Gap(0));
        assert_eq!(gap_or_group(&rows, 0.6), DropTarget::Gap(1));
        assert_eq!(gap_or_group(&rows, 1.5), DropTarget::Into(LayerId(2)));
        assert_eq!(gap_or_group(&rows, 1.9), DropTarget::Gap(2));
        assert_eq!(gap_or_group(&rows, 2.9), DropTarget::Gap(3));
        assert_eq!(gap_or_group(&rows, 9.0), DropTarget::Gap(3));
    }

    #[test]
    fn file_names_lose_what_a_path_cannot_hold() {
        assert_eq!(file_stem("a/b:c"), "a_b_c");
        assert_eq!(file_stem("  "), "smart");
        assert_eq!(file_stem("..."), "smart");
        assert_eq!(file_stem("木目"), "木目");
    }
}
