//! レイヤーセットの画面の下書きが使う、ダミーの値と、3 つのパネル（テクスチャセット・レイヤー・チャンネル）の描き方。
//! 部品は `yolu_app::ui` の物だけ。出力の割り当ては、チャンネルのパネルの行に入れる。
//!
//! 幅が広いとき（1000 など）は 3 つのパネルを横に並べ、狭いとき（300〜420）は 1 列に縦に積む。

use egui::{pos2, vec2, Id, Rect, Sense, Ui};
use yolu_app::ui::menu::{self, context_anchor, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

use super::layers_common::{self as common, DummyRow, Kind};
use crate::page::SketchCtx;
use egui::Color32;

// ───────── ダミーの値 ─────────

/// レイヤーセット（日本語・英語）。テクスチャセット「Body」の中の 3 つ。
pub const LAYER_SETS: [(&str, &str); 3] = [
    ("マテリアル", "Material"),
    ("影のマスク", "Shadow mask"),
    ("下書き", "Draft"),
];
/// 出力の割り当ての「なし」（そのチャンネルは既定値）。
pub const NONE: usize = 3;

/// チャンネル: アイコン・日本語・英語・形式。最後の 1 つはユーザーチャンネル。
pub const CHANNELS: [(&str, &str, &str, &str); 7] = [
    ("palette", "カラー", "Color", "sRGB8"),
    ("blur_on", "ラフネス", "Roughness", "L8"),
    ("contrast", "メタリック", "Metallic", "L8"),
    ("3d_rotation", "ノーマル", "Normal", "RGB8"),
    ("texture", "ハイト", "Height", "L8"),
    ("light_mode", "エミッション", "Emission", "sRGB8"),
    ("layers", "影の強度", "Shadow", "L8"),
];
/// 出力の割り当ての初めの値（チャンネル → レイヤーセットの番号）。
const DEFAULT_ROUTE: [usize; 7] = [0, 0, 0, 0, 0, 0, 1];

const fn layer(
    name: (&'static str, &'static str),
    kind: Kind,
    mask: bool,
    blend: (&'static str, &'static str),
    opacity: f32,
) -> DummyRow {
    DummyRow {
        name,
        kind,
        depth: 0,
        mask,
        locked: false,
        blend,
        opacity,
    }
}

const NORMAL: (&str, &str) = ("通常", "Normal");
const MULTIPLY: (&str, &str) = ("乗算", "Multiply");

/// レイヤーセットごとのレイヤー（上から）。
const LAYERS_MATERIAL: &[DummyRow] = &[
    layer(
        ("髪", "Hair"),
        Kind::Raster(Color32::from_rgb(200, 90, 130)),
        false,
        NORMAL,
        100.0,
    ),
    layer(
        ("服", "Clothes"),
        Kind::Raster(Color32::from_rgb(70, 120, 200)),
        true,
        NORMAL,
        100.0,
    ),
    layer(
        ("肌", "Skin"),
        Kind::Raster(Color32::from_rgb(222, 150, 80)),
        false,
        NORMAL,
        100.0,
    ),
    layer(
        ("ベース", "Base"),
        Kind::Fill(Color32::from_rgb(230, 230, 230)),
        false,
        NORMAL,
        100.0,
    ),
];
const LAYERS_SHADOW: &[DummyRow] = &[
    layer(
        ("影（やわらかめ）", "Shadow (soft)"),
        Kind::Raster(Color32::from_rgb(150, 150, 160)),
        false,
        MULTIPLY,
        80.0,
    ),
    layer(
        ("影のベース", "Shadow base"),
        Kind::Fill(Color32::from_rgb(110, 110, 120)),
        false,
        NORMAL,
        100.0,
    ),
];
const LAYERS_DRAFT: &[DummyRow] = &[
    layer(
        ("試し描き 2", "Trial 2"),
        Kind::Raster(Color32::from_rgb(96, 170, 120)),
        false,
        NORMAL,
        100.0,
    ),
    layer(
        ("試し描き 1", "Trial 1"),
        Kind::Raster(Color32::from_rgb(160, 160, 170)),
        false,
        NORMAL,
        100.0,
    ),
];

fn layers_of(set: usize) -> &'static [DummyRow] {
    match set {
        0 => LAYERS_MATERIAL,
        1 => LAYERS_SHADOW,
        _ => LAYERS_DRAFT,
    }
}

// ───────── 見せる状態 ─────────

/// 撮る間に開いた形で見せる物。
#[derive(Clone, Copy, PartialEq)]
pub enum Show {
    /// レイヤーセットの行の右クリックのメニュー。
    Menu,
    /// 出力の割り当てのドロップダウン（影の強度の行）。
    Pick,
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Sets,
    Layers,
    Channels,
}

// ───────── 状態（触ると動く値） ─────────

/// 今のレイヤーセット（初めは下書き。出力に出ていないチャンネルを描いている形を見せる）。
fn current_set(ctx: &SketchCtx) -> usize {
    ctx.values.index("lsets.set", 2)
}
fn paint_channel(ctx: &SketchCtx) -> usize {
    ctx.values.index("lsets.paint", 0)
}
fn route(ctx: &SketchCtx, channel: usize) -> usize {
    ctx.values
        .index(&format!("lsets.route.{channel}"), DEFAULT_ROUTE[channel])
}
fn outputs(ctx: &SketchCtx, set: usize) -> Vec<usize> {
    (0..CHANNELS.len())
        .filter(|c| route(ctx, *c) == set)
        .collect()
}
fn set_name(ctx: &SketchCtx, set: usize) -> &'static str {
    match LAYER_SETS.get(set) {
        Some((ja, en)) => ctx.pick(ja, en),
        None => ctx.pick("なし", "None"),
    }
}
fn channel_name(ctx: &SketchCtx, channel: usize) -> &'static str {
    ctx.pick(CHANNELS[channel].1, CHANNELS[channel].2)
}

// ───────── ポップアップ ─────────

/// ポップアップのメニューを開く・描く。`open_at` があれば、そこへ開く。撮る間は `force` の所へ最初のフレームで開き、開いたままにする。
/// 選ばれた行動を返す。
fn popup<A: Clone>(
    ui: &mut Ui,
    ctx: &SketchCtx,
    key: &str,
    open_at: Option<Rect>,
    force: Option<Rect>,
    entries: &[MenuEntry<A>],
    min_width: f32,
) -> Option<A> {
    let id = Id::new(("lsets.popup", key));
    let seen_id = id.with("seen");
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    let first = !ui.data(|d| d.get_temp::<bool>(seen_id).unwrap_or(false));
    let anchor = open_at.or(if ctx.still && first { force } else { None });
    if let Some(anchor) = anchor {
        state = Some(PopupState::new(ui.ctx(), anchor).with_min_width(min_width));
        ui.data_mut(|d| d.insert_temp(seen_id, true));
    }
    let mut chosen = None;
    if let Some(mut open) = state {
        let keep = open.anchor;
        match menu::show(ui.ctx(), id, &mut open, entries, &[keep]) {
            PopupOutcome::Open => {
                ui.data_mut(|d| d.insert_temp(id, open));
            }
            PopupOutcome::Chosen(action) => {
                chosen = Some(action);
                ui.data_mut(|d| d.remove::<PopupState>(id));
            }
            _ => {
                if ctx.still {
                    ui.data_mut(|d| d.insert_temp(id, open));
                } else {
                    ui.data_mut(|d| d.remove::<PopupState>(id));
                }
            }
        }
    }
    chosen
}

/// レイヤーセットの選び（出力の割り当てのドロップダウンと同じ）。
fn set_choices(ctx: &SketchCtx, current: usize, with_none: bool) -> Vec<MenuEntry<usize>> {
    let mut entries: Vec<MenuEntry<usize>> = LAYER_SETS
        .iter()
        .enumerate()
        .map(|(i, (ja, en))| MenuEntry::item(ctx.pick(ja, en), i).radio(current == i))
        .collect();
    if with_none {
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::item(ctx.pick("なし", "None"), NONE).radio(current == NONE));
    }
    entries
}

// ───────── 全体の並び ─────────

const MARGIN: f32 = 8.0;
const GAP: f32 = 8.0;
/// パネルの頭の帯の高さ。
const TAB_HEIGHT: f32 = 28.0;
const SET_ROW: f32 = 28.0;
const SET_TOOLBAR: f32 = 28.0;
const CHANNEL_ROW: f32 = 28.0;
const CHANNEL_TOOLBAR: f32 = 30.0;
/// チャンネルの行の右端の、形式の名前の欄の幅。
const FORMAT_WIDTH: f32 = 42.0;
const LAYER_TOP: f32 = 4.0 + 22.0 + 4.0 + 20.0 + 4.0 + 28.0;

pub fn scene(ui: &mut Ui, ctx: &mut SketchCtx, show: Show) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    w::fill(ui.painter(), r, t::WINDOW_BG);

    let wide = r.width() >= 700.0;
    let panels = [Panel::Sets, Panel::Layers, Panel::Channels];
    let area = r.shrink(MARGIN);
    let rects: Vec<Rect> = if wide {
        let n = panels.len() as f32;
        let width = (area.width() - GAP * (n - 1.0)) / n;
        (0..panels.len())
            .map(|i| {
                Rect::from_min_size(
                    pos2(area.left() + i as f32 * (width + GAP), area.top()),
                    vec2(width, area.height()),
                )
            })
            .collect()
    } else {
        let mut y = area.top();
        let last = panels.len() - 1;
        panels
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let h = if i == last {
                    (area.bottom() - y).max(0.0)
                } else {
                    needed_height(ctx, *p)
                };
                let rect = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h));
                y += h + GAP;
                rect
            })
            .collect()
    };

    for (p, rect) in panels.iter().zip(&rects) {
        let title = match p {
            Panel::Sets => ctx.pick("テクスチャセット", "Texture sets"),
            Panel::Layers => ctx.pick("レイヤー", "Layers"),
            Panel::Channels => ctx.pick("チャンネル", "Channels"),
        };
        let body = dock_panel(ui, *rect, title);
        match p {
            Panel::Sets => sets_panel(ui, ctx, body, show),
            Panel::Layers => layers_panel(ui, ctx, body),
            Panel::Channels => channels_panel(ui, ctx, body, show),
        }
    }
}

/// 狭いとき（縦に積むとき）の、パネルに要る高さ。
fn needed_height(ctx: &SketchCtx, panel: Panel) -> f32 {
    match panel {
        Panel::Sets => {
            let rows = (LAYER_SETS.len() + 2) as f32 * SET_ROW;
            TAB_HEIGHT + rows + SET_TOOLBAR + 6.0
        }
        Panel::Layers => {
            TAB_HEIGHT
                + LAYER_TOP
                + layers_of(current_set(ctx)).len().max(4) as f32 * common::ROW_HEIGHT
                + 6.0
        }
        Panel::Channels => TAB_HEIGHT + CHANNELS.len() as f32 * CHANNEL_ROW + CHANNEL_TOOLBAR + 6.0,
    }
}

/// ドックのパネルの枠（頭の帯に名前の 1 つのタブ）。中身の矩形を返す。
fn dock_panel(ui: &mut Ui, r: Rect, title: &str) -> Rect {
    let p = ui.painter().clone();
    w::rounded(&p, r, t::PANEL_BG, 4.0);
    let band = Rect::from_min_size(r.min, vec2(r.width(), TAB_HEIGHT));
    w::fill(&p, band, t::PANEL_HEADER);
    w::hline(
        &p,
        band.left(),
        band.right(),
        band.bottom() - 1.0,
        t::BORDER,
    );
    let width = w::text_width(&p, title, t::HEADER) + 24.0;
    let tab = Rect::from_min_size(
        pos2(band.left() + 4.0, band.top() + 2.0),
        vec2(width, band.height() - 2.0),
    );
    w::rounded(&p, tab, t::PANEL_BG, 3.0);
    w::fill(
        &p,
        Rect::from_min_size(
            pos2(tab.left() + 4.0, tab.bottom() - 2.0),
            vec2(tab.width() - 8.0, 2.0),
        ),
        t::ACCENT,
    );
    w::text(
        &p,
        tab,
        title,
        t::HEADER.with_color(Color32::WHITE),
        Align::Center,
    );
    Rect::from_min_max(pos2(r.left(), band.bottom()), r.max)
}

// ───────── テクスチャセットのパネル ─────────

#[derive(Clone, Copy, PartialEq)]
enum SetRow {
    /// テクスチャセット（名前・解像度）。
    Texture(&'static str, &'static str),
    /// 今のテクスチャセットの、レイヤーセットの行。`last` は字下げの線の終わり。
    LayerSet(usize),
}

fn sets_panel(ui: &mut Ui, ctx: &mut SketchCtx, body: Rect, show: Show) {
    let toolbar = Rect::from_min_max(pos2(body.left(), body.bottom() - SET_TOOLBAR), body.max);
    sets_toolbar(ui, ctx, toolbar);

    let rows: Vec<SetRow> = std::iter::once(SetRow::Texture("Body", "2048"))
        .chain((0..LAYER_SETS.len()).map(SetRow::LayerSet))
        .chain(std::iter::once(SetRow::Texture("Cloth", "1024")))
        .collect();
    let list_height = rows.len() as f32 * SET_ROW;
    let list = Rect::from_min_size(body.min, vec2(body.width(), list_height));
    w::fill(ui.painter(), list, t::CONTROL_BG);

    let mut menu_anchor = None;
    let mut open_at = None;
    let current = current_set(ctx);
    for (i, kind) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * SET_ROW),
            vec2(list.width(), SET_ROW),
        );
        let out = set_row(ui, ctx, list, row, *kind, current);
        if let SetRow::LayerSet(s) = kind {
            if out.clicked {
                ctx.values.set_index("lsets.set", *s);
                if let Some(first) = outputs(ctx, *s).first() {
                    ctx.values.set_index("lsets.paint", *first);
                }
            }
            if *s == 2 {
                menu_anchor = Some(context_anchor(pos2(row.left() + 70.0, row.bottom() - 5.0)));
            }
            if let Some(at) = out.context {
                open_at = Some(context_anchor(at));
            }
        }
    }

    // レイヤーセットの行の右クリックのメニュー
    let entries: Vec<MenuEntry<usize>> = vec![
        MenuEntry::item(ctx.pick("新しいレイヤーセット", "New layer set"), 0),
        MenuEntry::item(ctx.pick("複製", "Duplicate"), 1),
        MenuEntry::item(ctx.pick("名前を変更", "Rename"), 2),
        MenuEntry::Separator,
        MenuEntry::item(ctx.pick("削除", "Delete"), 3),
    ];
    let force = if show == Show::Menu {
        menu_anchor
    } else {
        None
    };
    let _ = popup(ui, ctx, "set-context", open_at, force, &entries, 0.0);
}

struct RowOut {
    clicked: bool,
    context: Option<egui::Pos2>,
}

fn set_row(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    list: Rect,
    row: Rect,
    kind: SetRow,
    current: usize,
) -> RowOut {
    let id = ui.make_persistent_id(("lsets.row", row.top() as i32));
    let response = ui.interact(row.intersect(list), id, Sense::click());
    let selected = matches!(kind, SetRow::LayerSet(s) if s == current);
    let painter = ui.painter_at(list);
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if response.hovered() {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );

    match kind {
        SetRow::Texture(name, resolution) => {
            let eye = Rect::from_min_size(
                pos2(row.left() + 4.0, row.top() + 3.0),
                vec2(24.0, row.height() - 6.0),
            );
            let _ = w::icon_button(
                ui,
                eye,
                ("lsets.eye", name),
                "visibility",
                ctx.pick("3D ビューで非表示にする", "Hide in the 3D view"),
                false,
                true,
                16.0,
            );
            let painter = ui.painter_at(list);
            let res_w = w::text_width(&painter, resolution, t::LABEL_DIM) + 4.0;
            let res_rect = Rect::from_min_max(
                pos2(row.right() - 8.0 - res_w, row.top()),
                pos2(row.right() - 8.0, row.bottom()),
            );
            w::text(&painter, res_rect, resolution, t::LABEL_DIM, Align::Right);
            let name_rect = Rect::from_min_max(
                pos2(eye.right() + 6.0, row.top() + 4.0),
                pos2(res_rect.left() - 4.0, row.bottom() - 4.0),
            );
            w::text(&painter, name_rect, name, t::LABEL, Align::Left);
        }
        SetRow::LayerSet(s) => {
            // 字下げの線（└）
            let line_x = row.left() + 16.0;
            let last = s + 1 == LAYER_SETS.len();
            let bottom = if last { row.center().y } else { row.bottom() };
            w::vline(&painter, line_x, row.top(), bottom, t::SEPARATOR);
            w::hline(
                &painter,
                line_x,
                line_x + 10.0,
                row.center().y,
                t::SEPARATOR,
            );

            let outs = outputs(ctx, s);
            let right = row.right() - 8.0;
            let mut name_right = right;
            if outs.is_empty() {
                // 結果に出していないレイヤーセット（下書き）: 名前を薄くして、切れた鎖
                let mark = Rect::from_min_size(
                    pos2(right - 18.0, row.top() + 5.0),
                    vec2(18.0, row.height() - 10.0),
                );
                w::icon(&painter, mark, "link_off", t::TEXT_DIM, 15.0);
                name_right = mark.left() - 4.0;
            }
            let name_rect = Rect::from_min_max(
                pos2(line_x + 16.0, row.top() + 4.0),
                pos2(name_right, row.bottom() - 4.0),
            );
            let color = if selected {
                Color32::WHITE
            } else if outs.is_empty() {
                t::TEXT_DIM
            } else {
                t::TEXT
            };
            let name = set_name(ctx, s);
            let shown = w::fit(&painter, name, name_rect.width(), t::LABEL);
            w::text(
                &painter,
                name_rect,
                &shown,
                t::LABEL.with_color(color),
                Align::Left,
            );
        }
    }
    RowOut {
        clicked: response.clicked(),
        context: if response.secondary_clicked() {
            response.interact_pointer_pos()
        } else {
            None
        },
    }
}

/// 下の帯（テクスチャセットの追加・削除・ベイク・プロジェクトの設定）。
fn sets_toolbar(ui: &mut Ui, ctx: &SketchCtx, bar: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, bar, t::PANEL_HEADER);
    w::hline(&p, bar.left(), bar.right(), bar.top(), t::BORDER);
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(26.0, bar.height() - 4.0));
    let items = [
        (
            bar.left() + 4.0,
            "add",
            "テクスチャセットを追加",
            "Add texture set",
        ),
        (
            bar.left() + 34.0,
            "delete",
            "テクスチャセットを削除",
            "Remove texture set",
        ),
        (
            bar.right() - 60.0,
            "local_fire_department",
            "メッシュマップをベイク",
            "Bake mesh maps",
        ),
        (
            bar.right() - 30.0,
            "tune",
            "プロジェクトの設定",
            "Project configuration",
        ),
    ];
    for (k, (x, icon, ja, en)) in items.iter().enumerate() {
        let _ = w::icon_button(
            ui,
            button(*x),
            ("lsets.set-bar", k),
            icon,
            ctx.pick(ja, en),
            false,
            true,
            16.0,
        );
    }
}

/// 出力の割り当てのドロップダウンの選び。選んだらその場で割り当てを替える。
fn route_popup(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    channel: usize,
    open_at: Option<Rect>,
    force: Option<Rect>,
) {
    let current = route(ctx, channel);
    let entries = set_choices(ctx, current, true);
    let width = open_at.or(force).map_or(0.0, |r| r.width());
    if let Some(next) = popup(
        ui,
        ctx,
        &format!("route.{channel}"),
        open_at,
        force,
        &entries,
        width,
    ) {
        ctx.values
            .set_index(&format!("lsets.route.{channel}"), next);
    }
}

// ───────── レイヤーのパネル ─────────

fn layers_panel(ui: &mut Ui, ctx: &mut SketchCtx, body: Rect) {
    let current = current_set(ctx);
    let paint = paint_channel(ctx);
    let mut y = body.top() + 4.0;

    // 頭: レイヤーセットのドロップダウン（2 つ以上あるときだけ）。今の描くチャンネルが出力に出ていなければ、横に印
    if LAYER_SETS.len() >= 2 {
        let missing = route(ctx, paint) != current;
        let mark_w = if missing { 26.0 } else { 0.0 };
        let dd = Rect::from_min_size(
            pos2(body.left() + t::PADDING, y),
            vec2(body.width() - 2.0 * t::PADDING - mark_w, 22.0),
        );
        let (response, b) = w::dropdown(
            ui,
            dd,
            "lsets.layerset",
            None,
            set_name(ctx, current),
            Some(ctx.pick("レイヤーセット", "Layer set")),
            true,
            0.0,
        );
        if missing {
            let mark = Rect::from_min_size(pos2(dd.right() + 4.0, y), vec2(22.0, 22.0));
            w::icon(ui.painter(), mark, "link_off", t::WARNING, 17.0);
            let tip = ctx.pick("出力なし", "Not in output");
            let hit = ui.interact(mark, Id::new("lsets.mark"), Sense::hover());
            let _ = hit.on_hover_text(tip);
        }
        let entries = set_choices(ctx, current, false);
        if let Some(next) = popup(
            ui,
            ctx,
            "layerset",
            response.clicked().then_some(b),
            None,
            &entries,
            b.width(),
        ) {
            ctx.values.set_index("lsets.set", next);
        }
        y = dd.bottom() + 4.0;
    }

    // 合成方法と不透明度
    let top = Rect::from_min_size(
        pos2(body.left() + t::PADDING, y),
        vec2(body.width() - 2.0 * t::PADDING, 20.0),
    );
    let stacked_width = (top.width() - 6.0) * 0.42;
    let left_w = stacked_width.floor();
    let blend = Rect::from_min_size(top.min, vec2(left_w, top.height()));
    let opacity = Rect::from_min_max(pos2(top.left() + left_w + 6.0, top.top()), top.max);
    let rows = layers_of(current);
    let selected = ctx.values.index("layers.selected", 0).min(rows.len() - 1);
    let _ = w::dropdown(
        ui,
        blend,
        "lsets.blend",
        None,
        ctx.pick(rows[selected].blend.0, rows[selected].blend.1),
        None,
        true,
        0.0,
    );
    let value = ctx
        .values
        .float("layers.top-opacity", rows[selected].opacity);
    let spec = SliderSpec::new(
        ctx.pick("不透明度", "Opacity"),
        0.0,
        100.0,
        NumberFormat::int("%"),
    );
    let out = w::slider(ui, opacity, "lsets.opacity", value, &spec);
    if out.changed {
        ctx.values.set_float("layers.top-opacity", out.value);
    }
    y = top.bottom() + 4.0;

    // 操作の帯
    let bar = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), 28.0));
    common::toolbar(ui, ctx, bar);
    y = bar.bottom();

    // 行
    let list = Rect::from_min_max(pos2(body.left(), y), body.max);
    w::fill(ui.painter(), list, t::CONTROL_BG);
    for (i, d) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * common::ROW_HEIGHT),
            vec2(list.width(), common::ROW_HEIGHT),
        );
        if row.top() >= list.bottom() {
            break;
        }
        common::row(ui, ctx, list, row, i, d, 26.0);
    }
}

// ───────── チャンネルのパネル ─────────

fn channels_panel(ui: &mut Ui, ctx: &mut SketchCtx, body: Rect, show: Show) {
    let toolbar = Rect::from_min_max(pos2(body.left(), body.bottom() - CHANNEL_TOOLBAR), body.max);
    let p = ui.painter().clone();
    w::fill(&p, toolbar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, toolbar.top() + 3.0), vec2(26.0, 24.0));
    let _ = w::icon_button(
        ui,
        button(toolbar.left() + 4.0),
        "lsets.channels.add",
        "add",
        ctx.pick("チャンネルを追加", "Add channel"),
        false,
        true,
        18.0,
    );
    let _ = w::icon_button(
        ui,
        button(toolbar.right() - 30.0),
        "lsets.channels.delete",
        "delete",
        ctx.pick("チャンネルを削除", "Delete channel"),
        false,
        true,
        17.0,
    );

    let list = Rect::from_min_size(
        body.min,
        vec2(body.width(), CHANNELS.len() as f32 * CHANNEL_ROW),
    );
    w::fill(&p, list, t::CONTROL_BG);
    let painting = paint_channel(ctx);
    let many = LAYER_SETS.len() >= 2;
    for (c, channel) in CHANNELS.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + c as f32 * CHANNEL_ROW),
            vec2(list.width(), CHANNEL_ROW),
        );
        let on = painting == c;
        let response = ui.interact(
            row,
            ui.make_persistent_id(("lsets.channel.row", c)),
            Sense::click(),
        );
        if response.clicked() {
            ctx.values.set_index("lsets.paint", c);
        }
        if on {
            w::fill(&p, row, t::ACCENT_SOFT);
            w::fill(
                &p,
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                t::ACCENT,
            );
        } else if response.hovered() {
            w::fill(&p, row, t::CONTROL_HOVER);
        }
        w::hline(&p, row.left(), row.right(), row.bottom() - 1.0, t::BORDER);

        let paint_btn = Rect::from_min_size(
            pos2(row.left() + 4.0, row.top() + 2.0),
            vec2(24.0, row.height() - 4.0),
        );
        let eye = Rect::from_min_size(
            pos2(paint_btn.right() + 2.0, row.top() + 2.0),
            vec2(24.0, row.height() - 4.0),
        );
        let icon_rect =
            Rect::from_min_size(pos2(eye.right() + 4.0, row.top()), vec2(18.0, row.height()));
        let name = channel_name(ctx, c);
        if w::icon_button(
            ui,
            paint_btn,
            ("lsets.channel.paint", c),
            "paint_brush",
            name,
            on,
            true,
            15.0,
        )
        .clicked()
        {
            ctx.values.set_index("lsets.paint", c);
        }
        let _ = w::icon_button(
            ui,
            eye,
            ("lsets.channel.eye", c),
            if c == 0 {
                "visibility"
            } else {
                "visibility_off"
            },
            name,
            c == 0,
            true,
            16.0,
        );
        w::icon(
            &p,
            icon_rect,
            channel.0,
            if on { Color32::WHITE } else { t::TEXT_DIM },
            15.0,
        );

        let format = channel.3;
        // 幅が足りない（340 未満）ときは、形式の名前を出さず、出力元のレイヤーセットの欄に譲る
        let show_format = !(many && row.width() < 340.0);
        let format_w = if show_format { FORMAT_WIDTH } else { 0.0 };
        let format_rect = Rect::from_min_max(
            pos2(row.right() - 8.0 - format_w, row.top()),
            pos2(row.right() - 8.0, row.bottom()),
        );
        let name_w = 76.0;
        let name_rect = Rect::from_min_size(
            pos2(icon_rect.right() + 4.0, row.top() + 3.0),
            vec2(name_w, row.height() - 6.0),
        );
        let shown = w::fit(&p, name, name_rect.width(), t::LABEL);
        w::text(
            &p,
            name_rect,
            &shown,
            t::LABEL.with_color(if on { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );

        let right = if show_format {
            format_rect.left() - 4.0
        } else {
            row.right() - 8.0
        };
        if many {
            // 行の右に出力元のレイヤーセット（形式の名前の左）
            let dd = Rect::from_min_max(
                pos2(name_rect.right() + 2.0, row.top() + 3.0),
                pos2(right, row.bottom() - 3.0),
            );
            let value = route(ctx, c);
            let (response, b) = w::dropdown(
                ui,
                dd,
                ("lsets.chan.dd", c),
                None,
                set_name(ctx, value),
                Some(ctx.pick("出力元のレイヤーセット", "Output layer set")),
                true,
                0.0,
            );
            route_popup(
                ui,
                ctx,
                c,
                response.clicked().then_some(b),
                (show == Show::Pick && c == 6).then_some(b),
            );
        }
        if show_format {
            w::text(&p, format_rect, format, t::LABEL_DIM, Align::Right);
        }
    }
}
