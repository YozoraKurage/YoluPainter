//! 配布用に保存のウィンドウ（写しに含める物にチェックを入れる）と、置き換えの確認のウィンドウ。アセットは種類ごとのたためる区分に 1 つずつ並べ
//! （区分の行のチェックは中の全部の一括。一部だけなら横線）、アセット以外の除ける物は種類ごとに 1 行。ウィンドウには名前・大きさ・状態の印だけを書き、
//! 説明はツールチップに置く。0 件の種類は出さない。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Id, Key, Rect, Sense, TextureHandle, WidgetInfo, WidgetType};
use yolu_io::shelf::ResourceKind;
use yolu_io::{Removal, ShelfItem};

use super::DistributeAction;
use crate::lang::Lang;
use crate::shelf::ItemKind;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};
use crate::windows::{show_list, Button, ListSpec, Reply, Row};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const WINDOW: &str = "distribute";
pub const CONFIRM: &str = "distribute-replace";

const WIDTH: f32 = 480.0;
/// 一覧の高さの上限（ウィンドウは 570 まで。見出し・上下の余白・下の帯を引いた分）。
const MAX_LIST: f32 = 570.0 - window::HEADER_HEIGHT - 8.0 - 10.0 - FOOTER;
const GROUP_HEIGHT: f32 = 30.0;
const ASSET_HEIGHT: f32 = 32.0;
const OTHER_HEIGHT: f32 = 30.0;
/// 区分の下の隙間。
const GROUP_GAP: f32 = 2.0;
/// アセットの区分とアセット以外の行の間（余白・区切り線・余白）。
const SEPARATOR_HEIGHT: f32 = 14.0;
const FOOTER: f32 = 48.0;
const SIDE: f32 = 16.0;
const SIZE_WIDTH: f32 = 58.0;
/// アセットの種類を並べる順（0 件の種類は出さない）。
const KINDS: [ResourceKind; 5] = [
    ResourceKind::Image,
    ResourceKind::SmartMaterial,
    ResourceKind::SmartMask,
    ResourceKind::Material,
    ResourceKind::Brush,
];

/// 種類の名前。
pub fn label(lang: Lang, removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => lang.t("distribute-original-psds"),
        Removal::SourcePaths => lang.t("distribute-asset-source-paths"),
        Removal::ModelReference => lang.t("distribute-model-reference"),
        Removal::MeshMaps => lang.t("distribute-mesh-maps"),
        Removal::UnityValues => lang.t("distribute-unity-material-values"),
        Removal::SavedSelections => lang.t("distribute-remembered-selections"),
        Removal::StaleEntries => lang.t("distribute-old-thumbnail-and-state"),
        Removal::UnknownEntries => lang.t("distribute-unknown-entries"),
    }
}

/// 種類のツールチップ（なぜ除くか。説明はここに置く）。
pub fn tooltip(lang: Lang, removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => lang.t("distribute-the-original-bytes-of-imported-tip"),
        Removal::SourcePaths => lang.t("distribute-where-the-assets-were-imported-tip"),
        Removal::ModelReference => lang.t("distribute-the-location-of-the-model-tip"),
        Removal::MeshMaps => lang.t("distribute-maps-baked-from-the-model-tip"),
        Removal::UnityValues => lang.t("distribute-values-received-from-unity-materials-tip"),
        Removal::SavedSelections => lang.t("distribute-selections-saved-under-a-name-tip"),
        Removal::StaleEntries => lang.t("distribute-thumbnail-brush-settings-and-similar-tip"),
        Removal::UnknownEntries => lang.t("distribute-entries-this-app-does-not-tip"),
    }
}

/// アセット以外の行のアイコン。
fn icon_of(removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => "import",
        Removal::SourcePaths => "folder_open",
        Removal::ModelReference => "view_in_ar",
        Removal::MeshMaps => "uv_wireframe",
        Removal::UnityValues => "sync",
        Removal::SavedSelections => "select_all",
        Removal::StaleEntries => "document_copy",
        Removal::UnknownEntries => "info",
    }
}

/// 「保存…」のツールチップ。
fn save_tooltip(lang: Lang) -> &'static str {
    lang.t("distribute-choose-where-to-save-a-tip")
}

/// 大きさの短い文字（100 KB 未満は「12 KB」、以上は「4.8 MB」、1 GB 以上は「1.2 GB」）。
pub fn size_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < 100.0 * KB {
        format!("{} KB", ((b / KB).ceil() as u64).max(1))
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// 区分のチェックの状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tri {
    None,
    Some,
    All,
}

/// ウィンドウの 1 行。
enum Item {
    /// アセットの種類の区分（チェックは中の全部の一括）。
    Group {
        kind: ResourceKind,
        state: Tri,
        open: bool,
        /// 含める物の大きさの合計。
        bytes: u64,
    },
    /// アセット 1 つ。
    Asset { item: ShelfItem, included: bool },
    /// アセットの区分とアセット以外の行の間の区切り。
    Separator,
    /// アセット以外の除ける物の種類。
    Other {
        removal: Removal,
        included: bool,
        bytes: u64,
    },
}

impl Item {
    fn height(&self) -> f32 {
        match self {
            Item::Group { .. } => GROUP_HEIGHT,
            Item::Asset { .. } => ASSET_HEIGHT,
            Item::Separator => SEPARATOR_HEIGHT,
            Item::Other { .. } => OTHER_HEIGHT,
        }
    }
    /// 区分の下の隙間は、区分（と、開いていればそのアセット）の最後の行の高さに足す。
    fn gap_after(&self, next: Option<&Item>) -> f32 {
        let ends_group = matches!(self, Item::Group { .. } | Item::Asset { .. })
            && !matches!(next, Some(Item::Asset { .. }));
        if ends_group && next.is_some() {
            GROUP_GAP
        } else {
            0.0
        }
    }
}

/// 一覧の行と、それぞれの上の端（一覧の中の高さ）、中身の全体の高さ。
fn layout(items: &[Item]) -> (Vec<f32>, f32) {
    let mut tops = Vec::with_capacity(items.len());
    let mut y = 0.0;
    for (i, item) in items.iter().enumerate() {
        tops.push(y);
        y += item.height() + item.gap_after(items.get(i + 1));
    }
    (tops, y)
}

/// ウィンドウの行を作る（開いている区分のアセットも並べる）。
fn build_items(app: &AppState) -> Vec<Item> {
    let Some(window) = app.distribute.window() else {
        return Vec::new();
    };
    let inventory = window.inventory();
    let mut items = Vec::new();
    for kind in KINDS {
        let members: Vec<&ShelfItem> = inventory.shelf.iter().filter(|i| i.kind == kind).collect();
        if members.is_empty() {
            continue;
        }
        let on = members
            .iter()
            .filter(|i| window.includes_resource(&i.id))
            .count();
        let state = if on == 0 {
            Tri::None
        } else if on == members.len() {
            Tri::All
        } else {
            Tri::Some
        };
        let bytes = members
            .iter()
            .filter(|i| window.includes_resource(&i.id))
            .map(|i| i.bytes)
            .sum();
        let open = !app.distribute.folded.contains(&kind);
        items.push(Item::Group {
            kind,
            state,
            open,
            bytes,
        });
        if open {
            for item in members {
                items.push(Item::Asset {
                    included: window.includes_resource(&item.id),
                    item: item.clone(),
                });
            }
        }
    }
    if !inventory.shelf.is_empty() && !inventory.found.is_empty() {
        items.push(Item::Separator);
    }
    for found in &inventory.found {
        items.push(Item::Other {
            removal: found.removal,
            included: window.includes(found.removal),
            bytes: found.bytes,
        });
    }
    items
}

// ───────── ウィンドウ ─────────

/// 毎フレーム、開いているウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.distribute.window_visible() {
        return;
    }
    let lang = app.lang;
    let items = build_items(app);
    let (tops, content) = layout(&items);
    let list_height = content.min(MAX_LIST);
    let height = window::HEADER_HEIGHT + 8.0 + list_height + 10.0 + FOOTER;
    // 見えているアセット（と前後 1 画面分）のサムネイルを別のスレッドで作らせ、できているものを取る
    let scroll_now = app
        .distribute
        .window_scroll
        .clamp(0.0, (content - list_height).max(0.0));
    let wanted = wanted_thumbnails(&items, &tops, scroll_now, list_height);
    let cache = app.library.cache().cloned();
    app.shelf.request_inspections(&wanted, cache.as_ref());
    let thumbs: HashMap<String, TextureHandle> = wanted
        .iter()
        .filter_map(|id| Some((id.clone(), app.shelf.texture(ctx, id)?)))
        .collect();
    let spec = Spec {
        title: lang.t("distribute-save-for-distribution"),
        icon: Some("save"),
        size: vec2(WIDTH, height),
        modal: true,
        close_label: lang.t("distribute-close-window"),
    };
    let id = Id::new(("yolu.window", WINDOW));
    let mut offset = app.ui.distribute_window_offset;
    let mut scroll = app.distribute.window_scroll;
    let mut actions: Vec<DistributeAction> = Vec::new();
    let mut folds: Vec<ResourceKind> = Vec::new();
    let mut esc = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        esc = ui.input(|i| i.key_pressed(Key::Escape));
        let body = frame.body;
        let list = Rect::from_min_size(
            pos2(body.left(), body.top() + 8.0),
            vec2(body.width(), list_height),
        );
        let bar = Scroll::begin(ui, list, content, &mut scroll);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        for (i, item) in items.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(list.left() + 14.0, list.top() + tops[i] - scroll),
                vec2(list.width() - 28.0 - bar.reserved(), item.height()),
            );
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            draw_item(&mut child, lang, r, item, &thumbs, &mut actions, &mut folds);
        }
        bar.end(ui, id.with("list-scroll"), &mut scroll);
        draw_footer(ui, lang, id, body, &mut actions);
    });
    app.ui.distribute_window_offset = offset;
    app.distribute.window_scroll = scroll;
    for kind in folds {
        match app.distribute.folded.iter().position(|k| *k == kind) {
            Some(at) => {
                app.distribute.folded.remove(at);
            }
            None => app.distribute.folded.push(kind),
        }
    }
    if (closed || esc) && actions.is_empty() {
        actions.push(DistributeAction::CancelWindow);
    }
    for action in actions {
        app.apply(Action::Distribute(action));
    }
}

/// 見えているアセット（と前後 1 画面分）の ID。サムネイルを作らせる対象。
fn wanted_thumbnails(
    items: &[Item],
    tops: &[f32],
    scroll_now: f32,
    list_height: f32,
) -> Vec<String> {
    items
        .iter()
        .zip(tops)
        .filter_map(|(item, top)| match item {
            Item::Asset { item, .. }
                if *top + ASSET_HEIGHT >= scroll_now - list_height
                    && *top <= scroll_now + 2.0 * list_height =>
            {
                Some(item.id.clone())
            }
            _ => None,
        })
        .collect()
}

/// 一覧の 1 行を描き、押されたら操作（`actions`）か区分の開閉（`folds`）に積む。
fn draw_item(
    ui: &mut egui::Ui,
    lang: Lang,
    r: Rect,
    item: &Item,
    thumbs: &HashMap<String, TextureHandle>,
    actions: &mut Vec<DistributeAction>,
    folds: &mut Vec<ResourceKind>,
) {
    match item {
        Item::Group {
            kind,
            state,
            open,
            bytes,
        } => draw_group(ui, lang, r, *kind, *state, *open, *bytes, actions, folds),
        Item::Asset { item, included } => {
            if row_box(
                ui,
                r,
                ("asset", item.id.as_str()),
                if *included { Tri::All } else { Tri::None },
                &item.name,
                lang.t("distribute-include"),
                26.0,
            ) {
                actions.push(DistributeAction::ToggleResource(item.id.clone()));
            }
            let p = ui.painter().clone();
            asset_content(ui, &p, lang, r, item, *included, thumbs.get(&item.id));
        }
        Item::Separator => {
            let p = ui.painter().clone();
            w::hline(&p, r.left(), r.right(), r.center().y, t::SEPARATOR);
        }
        Item::Other {
            removal,
            included,
            bytes,
        } => draw_other(ui, lang, r, *removal, *included, *bytes, actions),
    }
}

/// 区分の行（一括のチェック・開閉の矢印・名前・含める物の大きさ）。
#[allow(clippy::too_many_arguments)]
fn draw_group(
    ui: &mut egui::Ui,
    lang: Lang,
    r: Rect,
    kind: ResourceKind,
    state: Tri,
    open: bool,
    bytes: u64,
    actions: &mut Vec<DistributeAction>,
    folds: &mut Vec<ResourceKind>,
) {
    let name = group_name(lang, kind);
    // 行全体（一括のチェック）を先に置き、折りたたみの矢印をその上に重ねる
    if row_box(
        ui,
        r,
        ("group", kind as u8),
        state,
        name,
        lang.t("distribute-include"),
        0.0,
    ) {
        actions.push(DistributeAction::SetKind(kind, state != Tri::All));
    }
    let chevron = Rect::from_min_size(r.min, vec2(22.0, r.height()));
    let (icon, tip) = if open {
        ("expand_more", lang.t("distribute-collapse"))
    } else {
        ("chevron_right", lang.t("distribute-expand"))
    };
    if w::icon_button(
        ui,
        chevron,
        ("fold", kind as u8),
        icon,
        tip,
        false,
        true,
        16.0,
    )
    .clicked()
    {
        folds.push(kind);
    }
    let p = ui.painter().clone();
    let label_x = r.left() + 28.0 + SIDE + 8.0;
    w::text(
        &p,
        Rect::from_min_max(pos2(label_x, r.top()), r.max),
        name,
        t::LABEL_BOLD,
        Align::Left,
    );
    size_label(&p, r, 4.0, &size_text(bytes));
}

/// アセット以外の行（チェック・アイコン・名前・大きさ）。
fn draw_other(
    ui: &mut egui::Ui,
    lang: Lang,
    r: Rect,
    removal: Removal,
    included: bool,
    bytes: u64,
    actions: &mut Vec<DistributeAction>,
) {
    let name = label(lang, removal);
    let toggled = row_box(
        ui,
        r,
        ("removal", removal as u8),
        if included { Tri::All } else { Tri::None },
        name,
        tooltip(lang, removal),
        0.0,
    );
    if toggled {
        actions.push(DistributeAction::Toggle(removal));
    }
    let p = ui.painter().clone();
    let box_right = r.left() + 28.0 + SIDE + 2.0;
    let side = (r.height() - 8.0).min(26.0);
    let thumb = Rect::from_min_size(
        pos2(box_right + 10.0, r.center().y - side / 2.0),
        vec2(side, side),
    );
    w::rounded(&p, thumb, t::PANEL_HEADER, 3.0);
    w::icon(&p, thumb, icon_of(removal), t::TEXT_DIM, side - 10.0);
    let right = size_label(&p, r, 4.0, &size_text(bytes));
    let name_rect = Rect::from_min_max(pos2(thumb.right() + 8.0, r.top()), pos2(right, r.bottom()));
    let shown = w::fit(&p, name, name_rect.width(), t::LABEL);
    w::text(
        &p,
        name_rect,
        &shown,
        t::LABEL.with_color(if included { t::TEXT } else { t::TEXT_DIM }),
        Align::Left,
    );
}

/// 下の帯（やめる・保存…）。
fn draw_footer(
    ui: &mut egui::Ui,
    lang: Lang,
    id: Id,
    body: Rect,
    actions: &mut Vec<DistributeAction>,
) {
    // 下の帯
    let p = ui.painter().clone();
    let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
    w::fill(&p, footer, t::PANEL_HEADER);
    w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
    let mut x = footer.right() - 14.0;
    let buttons = [
        (
            lang.t("distribute-cancel"),
            false,
            None,
            DistributeAction::CancelWindow,
        ),
        (
            lang.t("distribute-save"),
            true,
            Some(save_tooltip(lang)),
            DistributeAction::ChooseFile,
        ),
    ];
    for (i, (label, primary, tip, action)) in buttons.into_iter().enumerate().rev() {
        let bw = w::text_width(&p, label, t::LABEL) + 32.0;
        let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
        x = r.left() - 8.0;
        if w::button(
            ui,
            r,
            id.with(("button", i)),
            label,
            primary,
            true,
            tip,
            None,
        )
        .clicked()
        {
            actions.push(action);
        }
    }
}

/// アセットの種類の区分の名前（アセットのタブの種類の名前と同じ）。
fn group_name(lang: Lang, kind: ResourceKind) -> &'static str {
    ItemKind::of(kind.as_str()).map_or("", |k| k.name(lang))
}

/// 大きさの文字を行の右端に置き、その左の端（ほかの物を置ける所の右の端）を返す。
fn size_label(p: &egui::Painter, row: Rect, pad: f32, text: &str) -> f32 {
    let right = row.right() - pad;
    w::text(
        p,
        Rect::from_min_size(
            pos2(right - SIZE_WIDTH, row.top()),
            vec2(SIZE_WIDTH, row.height()),
        ),
        text,
        t::LABEL_DIM,
        Align::Right,
    );
    right - SIZE_WIDTH - 8.0
}

/// 行の左のチェックの箱（全部・一部・なし）と、行全体の押せる範囲。箱の左は行の左から 28 + `indent`。押されたら true。
/// `name` は読み上げ・テストのための名前、`tip` はツールチップ。
fn row_box(
    ui: &mut egui::Ui,
    row: Rect,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    state: Tri,
    name: &str,
    tip: &str,
    indent: f32,
) -> bool {
    let id = ui.make_persistent_id(id_salt);
    let live = w::look(ui.ctx(), id, true).live;
    let response = ui.interact(row, id, Sense::click());
    let boxr = Rect::from_min_size(
        pos2(row.left() + 28.0 + indent, row.center().y - SIDE / 2.0),
        vec2(SIDE, SIDE),
    );
    let p = ui.painter().clone();
    if state == Tri::None {
        w::rounded(&p, boxr, t::CONTROL_BG, 3.0);
        w::outline(
            &p,
            boxr,
            if live && response.hovered() {
                t::ACCENT_DIM
            } else {
                t::SEPARATOR
            },
            1.0,
            3.0,
        );
    } else {
        w::rounded(&p, boxr, t::ACCENT, 3.0);
        if state == Tri::All {
            w::icon(&p, boxr, "check", Color32::WHITE, 14.0);
        } else {
            w::fill(
                &p,
                Rect::from_center_size(boxr.center(), vec2(boxr.width() - 7.0, 2.0)),
                Color32::WHITE,
            );
        }
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, state == Tri::All, name));
    if !tip.is_empty() {
        let _ = response.clone().on_hover_text(tip);
    }
    response.clicked()
}

/// 「未使用」の小さな印（名前と同じ行の右側）。幅を返す。
fn unused_chip(p: &egui::Painter, right: f32, row: Rect, label: &str) -> f32 {
    let width = w::text_width(p, label, t::LABEL_SMALL) + 12.0;
    let r = Rect::from_min_size(pos2(right - width, row.center().y - 9.0), vec2(width, 18.0));
    w::rounded(p, r, t::CONTROL_BG, 9.0);
    w::outline(p, r, t::SEPARATOR, 1.0, 9.0);
    w::text(
        p,
        r,
        label,
        t::LABEL_SMALL.with_color(t::TEXT_DIM),
        Align::Center,
    );
    width
}

/// アセット 1 行の中身（サムネイル・名前・「未使用」の印・使っているのに外した印・大きさ）。外した行は名前を暗くする。
fn asset_content(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    lang: Lang,
    row: Rect,
    item: &ShelfItem,
    included: bool,
    texture: Option<&TextureHandle>,
) {
    let box_right = row.left() + 28.0 + 26.0 + SIDE;
    let side = (row.height() - 8.0).min(26.0);
    let thumb = Rect::from_min_size(
        pos2(box_right + 10.0, row.center().y - side / 2.0),
        vec2(side, side),
    );
    thumbnail(p, thumb, item.kind, texture);
    let mut right = size_label(p, row, 4.0, &size_text(item.bytes));
    if !item.used {
        right -= unused_chip(p, right, row, lang.t("distribute-unused")) + 6.0;
    } else if !included {
        // 使っているアセットを外した（見た目が欠ける）印
        let mark = Rect::from_min_size(pos2(right - 18.0, row.top()), vec2(18.0, row.height()));
        w::icon(p, mark, "warning", t::WARNING, 15.0);
        let id = ui.make_persistent_id(("in-use", item.id.as_str()));
        let _ = ui
            .interact(mark, id, Sense::hover())
            .on_hover_text(lang.t("distribute-asset-in-use"));
        right -= 24.0;
    }
    let name_rect = Rect::from_min_max(
        pos2(thumb.right() + 8.0, row.top()),
        pos2(right, row.bottom()),
    );
    let shown = w::fit(p, &item.name, name_rect.width(), t::LABEL);
    w::text(
        p,
        name_rect,
        &shown,
        t::LABEL.with_color(if included { t::TEXT } else { t::TEXT_DIM }),
        Align::Left,
    );
}

/// アセットのサムネイル（作れていれば画像、まだか画像の無い種類は種類のアイコン）。
fn thumbnail(p: &egui::Painter, r: Rect, kind: ResourceKind, texture: Option<&TextureHandle>) {
    match texture {
        Some(texture) => {
            w::checker(p, r, 4.0);
            let size = texture.size_vec2();
            let scale = (r.width() / size.x.max(1.0)).min(r.height() / size.y.max(1.0));
            let fitted = Rect::from_center_size(r.center(), size * scale);
            p.image(
                texture.id(),
                fitted,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            w::outline(p, r, t::BORDER, 1.0, 0.0);
        }
        None => {
            w::rounded(p, r, t::PANEL_HEADER, 3.0);
            let icon = ItemKind::of(kind.as_str()).map_or("texture", |k| k.icon());
            w::icon(p, r, icon, t::TEXT_DIM, r.height() - 10.0);
            w::outline(p, r, t::BORDER, 1.0, 3.0);
        }
    }
}

// ───────── 置き換えの確かめ ─────────

/// 毎フレーム、置き換えの確認のウィンドウを描き、押された操作を当てる（既にあるファイルを選んだとき）。
pub fn show_replace(ctx: &egui::Context, app: &mut AppState) {
    let Some(path) = app.distribute.replace.clone() else {
        return;
    };
    let lang = app.lang;
    let spec = ListSpec {
        id: CONFIRM,
        title: lang.t("distribute-file-to-replace").into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            true,
        )),
        rows: vec![Row::text(path.display().to_string(), false)],
        buttons: vec![
            Button {
                label: lang.t("distribute-cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.t("distribute-replace").into(),
                primary: true,
                tooltip: Some(lang.t("distribute-replaces-the-file-with-the-tip").into()),
            },
        ],
        close_label: lang.t("distribute-close-window").into(),
    };
    let mut offset = app.ui.distribute_replace_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.ui.distribute_replace_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Distribute(DistributeAction::ConfirmReplace)),
        Some(_) => app.apply(Action::Distribute(DistributeAction::CancelReplace)),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ツールチップは、読み手を実際にある版（0.3.x のスタンドアロン版・0.4.x までの Unity 版）で言い、どの種類にも日英がある。
    /// 「覚えた選択範囲」は形式 7 に戻ること、「モデルの参照」は Live Link で開いたモデルの記録も除くことを言う。
    #[test]
    fn the_tooltips_name_the_readers_and_what_each_kind_takes_out() {
        for lang in [Lang::Ja, Lang::En] {
            for removal in Removal::ALL {
                assert!(!tooltip(lang, removal).is_empty(), "{removal:?}");
                assert!(!label(lang, removal).is_empty(), "{removal:?}");
            }
        }
        let saved = tooltip(Lang::Ja, Removal::SavedSelections);
        assert!(saved.contains("0.4.x") && saved.contains(" 7 "), "{saved}");
        let saved = tooltip(Lang::En, Removal::SavedSelections);
        assert!(
            saved.contains("0.4.x") && saved.contains("format 7"),
            "{saved}"
        );
        assert!(tooltip(Lang::Ja, Removal::SourcePaths).contains("フォント"));
        assert!(tooltip(Lang::En, Removal::SourcePaths).contains("font"));
        assert!(tooltip(Lang::Ja, Removal::ModelReference).contains("Live Link"));
        assert!(tooltip(Lang::En, Removal::ModelReference).contains("Live Link"));
    }
}
