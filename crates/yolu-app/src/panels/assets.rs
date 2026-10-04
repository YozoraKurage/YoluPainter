//! アセットのパネル（Substance のシェルフ）: 棚の素材（画像・ブラシ・マテリアル・スマートマテリアル・スマートマスク）をサムネイルの
//! 格子で並べる。上に種類の絞り込み（すべて・5 種類のアイコン）と .ylsmart の読み込み、名前の検索、選んだ層の保存（層・マスク）、
//! 下に選んだ素材の名前と状態（置けないときは短い理由）と操作（置く・書き出す・消す）。格子の素材はダブルクリック・右クリック・
//! ドラッグでレイヤーのパネルへ置く（スマートマテリアルは落とした行の間・グループの中、スマートマスクは落とした行の層のマスク）。
//! 画面には名前と状態と短い理由だけを出し、説明はツールチップに置く。

use std::path::PathBuf;

use egui::{
    pos2, vec2, Color32, DragAndDrop, Id, Rect, Sense, TextureHandle, Ui, WidgetInfo, WidgetType,
};

use crate::engine::Document;
use crate::lang::Lang;
use crate::m2::{DropTarget, Row};
use crate::panels::layers::ROW_HEIGHT;
use crate::shelf::{self, ItemKind, PlaceTarget, ShelfDrag, ShelfOp};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const CELL_W: f32 = 76.0;
pub const CELL_H: f32 = 92.0;
pub const GAP: f32 = 6.0;
pub const THUMB_BOX: f32 = 64.0;
/// 縦のスクロールバーに空ける幅。
pub const BAR_W: f32 = 10.0;
/// 下の名前と操作の帯の高さ。
pub const FOOTER_H: f32 = 62.0;

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
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    app.shelf.use_language(lang);
    // 見える項目のサムネイルと説明は 1 フレームに数個ずつ作る（大きな素材が一度に並んでも固まらない）
    if app.shelf.inspect_pending(2) {
        ctx.request_repaint();
    }

    let mut rows = w::Rows::new(r, 6.0);
    // 種類の絞り込み（左）と、.ylsmart の読み込み（右）
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
    // 選んだ層の保存（別のスレッドで書き出している間は、名前と「やめる」）
    let row = rows.row(24.0, 6.0);
    if let Some(name) = app.shelf.saving_name().map(str::to_owned) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let cancel = Rect::from_min_size(
            pos2(row.right() - 72.0, row.top()),
            vec2(72.0, row.height()),
        );
        let label = Rect::from_min_max(row.min, pos2(cancel.left() - 6.0, row.bottom()));
        let p = ui.painter().clone();
        let text = format!("{}: {name}", lang.pick("保存中", "Saving"));
        let shown = w::fit(&p, &text, label.width(), t::LABEL_DIM);
        w::text(&p, label, &shown, t::LABEL_DIM, Align::Left);
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

    let top = rows.y();
    let grid = Rect::from_min_max(
        pos2(r.left() + t::PADDING, top),
        pos2(r.right() - t::PADDING, (r.bottom() - FOOTER_H).max(top)),
    );
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

/// 選んだ層の保存のボタン（層・マスク）。
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
        lang.pick("層を保存", "Save Layer"),
        false,
        can_save,
        Some(lang.pick(
            "選んでいるレイヤー（グループなら中身ごと）をスマートマテリアルとして棚に入れる",
            "Put the selected layer (with its contents, for a group) on the shelf as a smart material",
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
            "選んでいるレイヤーのマスクをスマートマスクとして棚に入れる",
            "Put the selected layer's mask on the shelf as a smart mask",
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

fn cards(ui: &mut Ui, app: &mut AppState, ctx: &egui::Context, grid: Rect) {
    let lang = app.lang;
    w::rounded(ui.painter(), grid, t::MENU_BG, 4.0);
    // 並べる素材（棚の並びのまま）
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
        });
    }
    if list.is_empty() {
        // 空の棚・一致なしは、空の状態の文字（「なし」「一致なし」）を置かず、空のまま。読めないときだけ、その状態を出す
        if let Some(reason) = app.shelf.unavailable.clone() {
            let at = Rect::from_min_size(
                pos2(grid.left() + 8.0, grid.top() + 10.0),
                vec2(grid.width() - 16.0, 18.0),
            );
            w::text(
                ui.painter(),
                at,
                lang.pick("棚を読めません", "Shelf unreadable"),
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
        max_scroll,
    } = grid_metrics(grid.width(), grid.height(), list.len());
    if ui.rect_contains_pointer(grid) {
        app.shelf.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    app.shelf.scroll = app.shelf.scroll.clamp(0.0, max_scroll);
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
    if max_scroll > 0.0 {
        let bar_h = grid.height() * grid.height() / content;
        let bar_y = grid.top() + (grid.height() - bar_h) * app.shelf.scroll / max_scroll;
        w::rounded(
            &painter,
            Rect::from_min_size(pos2(grid.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
}

fn card_cell(
    ui: &mut Ui,
    app: &mut AppState,
    painter: &egui::Painter,
    grid: Rect,
    cell: Rect,
    card: &Card,
) {
    let selected = app.shelf.selected.as_deref() == Some(card.id.as_str());
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
    w::outline(painter, thumb, t::BORDER, 1.0, 0.0);
    // 置けないしるし（右上。素材の中身で置けないものだけ。種類で置けないものは下の名前の帯に理由を出す）と種類の印（左下）
    if card.warn {
        let badge = Rect::from_min_size(
            pos2(thumb.right() - 16.0, thumb.top() + 2.0),
            vec2(14.0, 14.0),
        );
        w::rounded(painter, badge, t::PANEL_BG, 7.0);
        w::icon(painter, badge, "warning", t::WARNING, 11.0);
    }
    if card.builtin {
        // 組み込みの印（左上。消せない・書き出せない元）
        let badge = Rect::from_min_size(pos2(thumb.left() + 2.0, thumb.top() + 2.0), vec2(14.0, 14.0));
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
        app.shelf.selected = Some(card.id.clone());
    }
    if response.double_clicked() {
        app.apply(Action::Shelf(ShelfOp::Place {
            id: card.id.clone(),
            target: PlaceTarget::Selected,
        }));
    }
    if response.secondary_clicked() {
        app.shelf.selected = Some(card.id.clone());
        if let Some(at) = response.interact_pointer_pos() {
            app.popup = Some(OpenPopup {
                kind: PopupKind::Shelf,
                state: PopupState::new(ui.ctx(), context_anchor(at)),
            });
        }
    }
    if response.drag_started() {
        response.dnd_set_drag_payload(ShelfDrag {
            id: card.id.clone(),
            kind: card.kind,
            name: card.name.clone(),
        });
    }
    let mut tip = card.name.clone();
    if !card.detail.is_empty() {
        tip += &format!("\n{}", card.detail);
    }
    if let Some(reason) = &card.block {
        tip += &format!("\n{reason}");
    }
    if card.builtin {
        tip += &format!("\n{}", app.lang.pick("組み込み", "Built-in"));
    }
    let name = card.name.clone();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &name));
    let _ = response.on_hover_text(tip);
}

fn footer(ui: &mut Ui, app: &mut AppState, r: Rect) {
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
    let icons = 2.0 * 28.0;
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
        id.as_deref().is_some_and(|i| !shelf::is_builtin(i)) && kind.is_some_and(ItemKind::is_smart),
        16.0,
    )
    .clicked()
    {
        if let Some(id) = &id {
            app.apply(Action::Shelf(ShelfOp::ExportDialog(id.clone())));
        }
    }
    let remove = Rect::from_min_size(
        pos2(export.right() + 2.0, buttons.top()),
        vec2(26.0, buttons.height()),
    );
    if w::icon_button(
        ui,
        remove,
        "shelf.remove",
        "delete",
        lang.pick(
            "棚から消す（置いた層はそのまま）",
            "Remove from the shelf (placed layers stay)",
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

/// 落とす先の置き場所（親のグループと、その子の中の位置）。線は、その下の行の層のすぐ上（同じグループの中）。一番下の線は最上位の
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

/// レイヤーの一覧の上で棚の素材を引いているとき、落とす先の印を描き、離したら置く（スマートマテリアルは行の間・グループの中、
/// スマートマスクは行の層のマスク、それ以外は断る理由をステータスバーへ）。
pub fn layer_list_drop(ui: &Ui, app: &mut AppState, list: Rect, rows: &[Row]) {
    let ctx = ui.ctx();
    let Some(drag) = DragAndDrop::payload::<ShelfDrag>(ctx) else {
        return;
    };
    let Some(p) = ctx.pointer_latest_pos().filter(|p| list.contains(*p)) else {
        return;
    };
    let released = ui.input(|i| i.pointer.any_released());
    // 一覧の行の高さは層と効果で違うので、行の数え方は効果の行の配置から（層の行の単位に直す）
    let layout = crate::panels::effect_rows::layout(&app.doc, rows, ROW_HEIGHT);
    let at = p.y - list.top() + app.layer_scroll;
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
                let y = list.top() + layout.layer_y(index) - app.layer_scroll;
                w::outline(&painter, frame(y), t::ACCENT, 2.0, 3.0);
                PlaceTarget::Mask(row.id)
            })
        }
        // 画像は 1 枚のペイントの層として、スマートマテリアルと同じ所へ置く
        ItemKind::SmartMaterial | ItemKind::Image => {
            let target = gap_or_group(rows, position);
            match target {
                DropTarget::Gap(gap) => {
                    let y = list.top() + layout.gap_y(gap) - app.layer_scroll;
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
                        let y = list.top() + layout.layer_y(i) - app.layer_scroll;
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

// ───────── メニュー・ファイルの窓 ─────────

/// 棚の素材の右クリックのメニュー（選んでいる素材）。
pub fn menu_entries(app: &AppState) -> Vec<Entry<Action>> {
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
            lang.pick("棚から消す…", "Remove from the shelf…"),
            Action::Shelf(ShelfOp::AskRemove(id)),
        )
        .enabled(free && app.shelf.unavailable.is_none() && !builtin),
    ]
}

/// 毎フレーム: 別のスレッドの書き出しが終わっていれば棚へ入れ、窓に落とした .ylsmart を棚へ入れる（.ylp は `YoluApp` が開く）。
pub fn frame(ctx: &egui::Context, state: &mut AppState) {
    state.shelf.context = Some(ctx.clone());
    state.shelf_poll();
    import_dropped(ctx, state);
}

fn import_dropped(ctx: &egui::Context, state: &mut AppState) {
    let dropped: Vec<PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ylsmart"))
            })
            .collect()
    });
    if !dropped.is_empty() {
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

/// 棚のファイルの窓・確かめの窓（窓を開かない試験では呼ばれない。頼みは `state.dialog_request` に残る）。
pub fn run_dialog(state: &mut AppState, request: DialogRequest) {
    let lang = state.lang;
    match request {
        DialogRequest::ShelfImport => {
            if let Some(paths) = rfd::FileDialog::new()
                .set_title(lang.pick(
                    "スマート素材を棚へ読み込む",
                    "Import smart assets to the shelf",
                ))
                .add_filter("YoluPainter Smart", &["ylsmart"])
                .pick_files()
            {
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
            if let Some(path) = rfd::FileDialog::new()
                .set_title(lang.pick("スマート素材を書き出す", "Export the smart asset"))
                .add_filter("YoluPainter Smart", &["ylsmart"])
                .set_file_name(format!("{}.ylsmart", file_stem(&name)))
                .save_file()
            {
                state.apply(Action::Shelf(ShelfOp::ExportFile { id, path }));
            }
        }
        DialogRequest::ShelfRemove => {
            let Some(id) = state.shelf.pending_remove.take() else {
                return;
            };
            let Some(name) = state.shelf.get(&id).map(|r| r.name.clone()) else {
                return;
            };
            let yes = rfd::MessageDialog::new()
                .set_title("YoluPainter")
                .set_description(match lang {
                    Lang::Ja => format!("「{name}」を棚から消しますか？"),
                    Lang::En => format!("Remove \"{name}\" from the shelf?"),
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
