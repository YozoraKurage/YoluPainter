//! レイヤーのパネル（Unity 版の LayersPanel の配置）: 上に描くチャンネルの合成モードと不透明度（左端の切り替えで、そのチャンネルだけの
//! 値にする）、行（目・グループの開閉・サムネイル・マスク・名前。ダブルクリックで名前を変える・右クリックのメニュー・ドラッグで
//! 並べ替えとグループへの出し入れ）、下に操作の帯（新規・塗りつぶし・調整・グループ・マスク・上へ・下へ・削除）。行は上が一番上のレイヤーで、
//! グループの中身は字下げして続く（閉じたグループの中身は出さない）。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Rect, Sense, TextureHandle, TextureOptions, Ui, WidgetInfo,
    WidgetType,
};

use crate::engine::{Channel, Document, LayerId, LayerKind};
use crate::m2::{self, AdjustmentKind, DropTarget, Edit, LayerDrag, Row, UiOp};
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

pub const ROW_HEIGHT: f32 = 30.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// パネルの上の端から一覧の上の端まで（合成モードと不透明度の行）。
pub const LIST_TOP: f32 = 6.0 + 24.0 + 6.0;
/// 1 段の字下げ。
const INDENT: f32 = 14.0;

/// サムネイルの絵の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbSource {
    /// そのチャンネルの画素。
    Channel(Channel),
    /// マスク（見せる量を灰色で）。
    Mask,
}

/// レイヤーのサムネイル（文書の版が変わったら作り直す。core は層ごとの版を持たないので文書の版で見る。描いている間は毎フレーム）。
#[derive(Default)]
pub struct Thumbnails {
    map: HashMap<(LayerId, ThumbSource), (u64, TextureHandle)>,
    /// 作り直した回数（試験用）。
    pub rebuilt: usize,
}

/// 層の 1 画素（straight RGBA8）。マスクは見せる量の灰色。
fn thumb_pixel(doc: &Document, id: LayerId, source: ThumbSource, x: u32, y: u32) -> [u8; 4] {
    let Some(layer) = doc.layer(id) else {
        return [0; 4];
    };
    match source {
        ThumbSource::Channel(c) => layer
            .surface(c)
            .and_then(|s| s.pixel(x, y).ok())
            .map(|p| p.to_array())
            .unwrap_or([0; 4]),
        ThumbSource::Mask => layer
            .mask()
            .and_then(|m| m.factor_at(x, y).ok())
            .map(|f| {
                let g = (f.clamp(0.0, 1.0) * 255.0).round() as u8;
                [g, g, g, 255]
            })
            .unwrap_or([255; 4]),
    }
}

fn thumb_has_pixels(doc: &Document, id: LayerId, source: ThumbSource) -> bool {
    let Some(layer) = doc.layer(id) else {
        return false;
    };
    match source {
        ThumbSource::Channel(c) => layer.surface(c).is_some_and(|s| s.tile_count() > 0),
        ThumbSource::Mask => layer.mask().is_some(),
    }
}

impl Thumbnails {
    fn get(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        id: LayerId,
        max_px: u32,
        source: ThumbSource,
    ) -> Option<&TextureHandle> {
        doc.layer(id)?;
        let revision = doc.revision();
        let key = (id, source);
        let fresh = self
            .map
            .get(&key)
            .is_some_and(|(r, h)| *r == revision && h.size()[0].max(h.size()[1]) as u32 == max_px);
        if !fresh {
            let (dw, dh) = (doc.width(), doc.height());
            let (tw, th) = if dw >= dh {
                (
                    max_px,
                    ((max_px as f32 * dh as f32 / dw as f32).round() as u32).max(1),
                )
            } else {
                (
                    ((max_px as f32 * dw as f32 / dh as f32).round() as u32).max(1),
                    max_px,
                )
            };
            let mut pixels = vec![Color32::TRANSPARENT; (tw * th) as usize];
            if thumb_has_pixels(doc, id, source) {
                for ty in 0..th {
                    // 下から上の行を、画像の上から下へ並べ替える
                    let sy = (((ty as f32 + 0.5) * dh as f32 / th as f32) as u32).min(dh - 1);
                    for tx in 0..tw {
                        let sx = (((tx as f32 + 0.5) * dw as f32 / tw as f32) as u32).min(dw - 1);
                        let p = thumb_pixel(doc, id, source, sx, sy);
                        pixels[((th - 1 - ty) * tw + tx) as usize] =
                            Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
                    }
                }
            } else if source == ThumbSource::Mask {
                pixels.fill(Color32::WHITE); // 何も隠さないマスクは白
            }
            let image = ColorImage::new([tw as usize, th as usize], pixels);
            match self.map.get_mut(&key) {
                Some((r, handle)) if handle.size() == [tw as usize, th as usize] => {
                    handle.set(image, TextureOptions::LINEAR);
                    *r = revision;
                }
                _ => {
                    let name = match source {
                        ThumbSource::Channel(c) => format!("thumb-{}-{}", id.0, c.index()),
                        ThumbSource::Mask => format!("thumb-{}-mask", id.0),
                    };
                    let handle = ctx.load_texture(name, image, TextureOptions::LINEAR);
                    self.map.insert(key, (revision, handle));
                }
            }
            self.rebuilt += 1;
        }
        self.map.get(&key).map(|(_, h)| h)
    }

    fn retain(&mut self, doc: &Document) {
        self.map.retain(|(id, source), _| {
            doc.layer(*id)
                .is_some_and(|l| *source != ThumbSource::Mask || l.mask().is_some())
        });
    }
}

fn open_popup(app: &mut AppState, ctx: &egui::Context, kind: PopupKind, anchor: Rect, min: f32) {
    app.popup = Some(OpenPopup {
        kind,
        state: PopupState::new(ctx, anchor).with_min_width(min),
    });
}

pub fn show(ui: &mut Ui, app: &mut AppState, thumbs: &mut Thumbnails) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let enabled = app.can_edit();
    let lang = app.lang;
    let channel = app.m2.paint_channel;
    let selected = app.selected_layer.and_then(|id| {
        app.doc.layer(id).map(|l| {
            (
                id,
                l.blend_mode_in(channel),
                l.opacity_in(channel),
                !l.channel_blend(channel).is_empty(),
            )
        })
    });

    // 上: 描くチャンネルの合成モードと不透明度（Photoshop の配置）。左端の切り替えで、そのチャンネルだけの値にする
    let top = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + 6.0),
        vec2(r.width() - 2.0 * t::PADDING, 24.0),
    );
    let own_rect = Rect::from_min_size(top.min, vec2(18.0, top.height()));
    let rest = Rect::from_min_max(pos2(top.left() + 20.0, top.top()), top.max);
    let left_w = ((rest.width() - 6.0) * 0.42).floor();
    let blend_rect = Rect::from_min_size(rest.min, vec2(left_w, rest.height()));
    let opacity_rect = Rect::from_min_max(pos2(rest.left() + left_w + 6.0, rest.top()), rest.max);
    if let Some((id, blend, opacity, own)) = selected {
        let name = m2::channel_name(lang, &app.doc, channel);
        let own_tip = if own {
            lang.pick(
                format!("{name} だけの合成モードと不透明度（押すと層の値に戻す）"),
                format!("{name} only: its own blend mode and opacity (click to follow the layer)"),
            )
        } else {
            lang.pick(
                format!("層の合成モードと不透明度（押すと {name} 専用にする）"),
                format!("The layer's blend mode and opacity (click to give {name} its own)"),
            )
        };
        if w::icon_button(
            ui,
            own_rect,
            "layers.own",
            m2::channel_icon(channel),
            &own_tip,
            own,
            enabled,
            15.0,
        )
        .clicked()
        {
            app.apply(Action::M2(Edit::OwnBlend {
                id,
                channel,
                own: !own,
            }));
        }
        let (response, b) = w::dropdown(
            ui,
            blend_rect,
            "layers.blend",
            None,
            m2::blend_label(lang, blend),
            Some(lang.pick("合成モード", "Blend mode")),
            enabled,
            0.0,
        );
        if response.clicked() {
            open_popup(app, &ctx, PopupKind::BlendMode(id), b, b.width());
        }
        let spec = SliderSpec::new(
            lang.pick("不透明度", "Opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        )
        .enabled(enabled)
        .tooltip(lang.pick("レイヤーの不透明度", "Layer opacity"));
        let o = w::slider(
            ui,
            opacity_rect,
            "layers.opacity",
            (opacity * 100.0) as f32,
            &spec,
        );
        if o.changed {
            app.apply(Action::M2(Edit::Opacity {
                id,
                channel: own.then_some(channel),
                value: (o.value as f64 / 100.0).clamp(0.0, 1.0),
            }));
        }
        if o.released {
            app.m2_end_drag();
        }
    }

    // 一覧
    thumbs.retain(&app.doc);
    let list = Rect::from_min_max(
        pos2(r.left(), r.top() + LIST_TOP),
        pos2(
            r.right(),
            (r.bottom() - TOOLBAR_HEIGHT).max(r.top() + LIST_TOP),
        ),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let rows = m2::visible_rows(&app.doc, &app.m2.collapsed);
    let n = rows.len();
    let content = n as f32 * ROW_HEIGHT;
    let max_scroll = (content - list.height()).max(0.0);
    if ui.rect_contains_pointer(list) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.layer_scroll -= wheel;
    }
    app.layer_scroll = app.layer_scroll.clamp(0.0, max_scroll);
    let row_width = list.width() - if max_scroll > 0.0 { 10.0 } else { 0.0 };

    // 空白の右クリック（行の下）
    let blank = Rect::from_min_max(
        pos2(
            list.left(),
            (list.top() + content - app.layer_scroll).max(list.top()),
        ),
        list.max,
    );
    if blank.height() > 0.0 {
        let response = ui.interact(blank, ui.make_persistent_id("layers.blank"), Sense::click());
        if response.secondary_clicked() && enabled {
            if let Some(at) = response.interact_pointer_pos() {
                open_popup(
                    app,
                    &ctx,
                    PopupKind::M2(Popup::LayerBlank),
                    context_anchor(at),
                    0.0,
                );
            }
        }
    }

    for (row_index, row) in rows.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + row_index as f32 * ROW_HEIGHT - app.layer_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if rect.bottom() < list.top() || rect.top() > list.bottom() {
            continue;
        }
        layer_row(ui, app, thumbs, &ctx, list, rect, *row, &rows);
    }
    follow_drag(ui, app, list, &rows);
    crate::panels::assets::layer_list_drop(ui, app, list, &rows);
    // ドラッグの落とす先（線か、グループの枠）
    if let Some(LayerDrag {
        target: Some(target),
        ..
    }) = app.layer_drag
    {
        let painter = ui.painter_at(list);
        match target {
            DropTarget::Gap(gap) => {
                let y = list.top() + gap as f32 * ROW_HEIGHT - app.layer_scroll;
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(list.left() + 4.0, y - 1.0),
                        vec2(row_width - 8.0, 2.0),
                    ),
                    0.0,
                    t::ACCENT,
                );
            }
            DropTarget::Into(group) => {
                if let Some(i) = rows.iter().position(|r| r.id == group) {
                    let y = list.top() + i as f32 * ROW_HEIGHT - app.layer_scroll;
                    w::outline(
                        &painter,
                        Rect::from_min_size(
                            pos2(list.left() + 1.0, y + 1.0),
                            vec2(row_width - 2.0, ROW_HEIGHT - 2.0),
                        ),
                        t::ACCENT,
                        2.0,
                        3.0,
                    );
                }
            }
        }
    }
    if max_scroll > 0.0 {
        let bar_h = list.height() * list.height() / content;
        let bar_y = list.top() + (list.height() - bar_h) * app.layer_scroll / max_scroll;
        w::rounded(
            ui.painter(),
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }

    toolbar(ui, app, &ctx, r, list, enabled);
}

/// 下の操作の帯。
fn toolbar(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    r: Rect,
    list: Rect,
    enabled: bool,
) {
    let lang = app.lang;
    let bar = Rect::from_min_size(
        pos2(r.left(), list.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    let selected = app.selected_layer.and_then(|id| app.doc.layer(id));
    let has = selected.is_some() && enabled;
    let has_mask = selected.is_some_and(|l| l.mask().is_some());
    let editing = app.m2.edit_mask && has_mask;
    let can_delete = app
        .selected_layer
        .is_some_and(|id| app.doc.layers().len() > m2::subtree_len(&app.doc, id));
    let mut x = bar.left() + 4.0;
    let mut next = || {
        let b = button(x);
        x += 27.0;
        b
    };
    if w::icon_button(
        ui,
        next(),
        "layers.add",
        "add",
        lang.pick("新規レイヤー", "New Layer"),
        false,
        enabled,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::NewLayer);
    }
    if w::icon_button(
        ui,
        next(),
        "layers.fill",
        "format_color_fill",
        lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        false,
        enabled,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::M2(Edit::NewFill));
    }
    let b = next();
    let response = w::icon_button(
        ui,
        b,
        "layers.adjustment",
        "tune",
        lang.pick("新規調整レイヤー", "New Adjustment Layer"),
        false,
        enabled,
        17.0,
    );
    if response.clicked() {
        open_popup(app, ctx, PopupKind::M2(Popup::NewAdjustment), b, 0.0);
    }
    if w::icon_button(
        ui,
        next(),
        "layers.group",
        "folder",
        lang.pick("レイヤーをグループ化", "Group Layers"),
        false,
        has,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::M2(Edit::GroupSelected));
    }
    let mask_tip = if has_mask {
        lang.pick("レイヤーマスクを編集", "Edit Layer Mask")
    } else {
        lang.pick("レイヤーマスクを追加", "Add Layer Mask")
    };
    if w::icon_button(
        ui,
        next(),
        "layers.mask",
        "vignette",
        mask_tip,
        editing,
        has,
        17.0,
    )
    .clicked()
    {
        if let Some(id) = app.selected_layer {
            if has_mask {
                app.apply(Action::M2Ui(UiOp::EditMask(!editing)));
            } else {
                app.apply(Action::M2(Edit::AddMask(id)));
            }
        }
    }
    let x = bar.right() - 4.0 - 27.0 * 3.0;
    if w::icon_button(
        ui,
        button(x),
        "layers.up",
        "expand_less",
        lang.pick("レイヤーを上へ", "Move Layer Up"),
        false,
        has,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::LayerUp);
    }
    if w::icon_button(
        ui,
        button(x + 27.0),
        "layers.down",
        "expand_more",
        lang.pick("レイヤーを下へ", "Move Layer Down"),
        false,
        has,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::LayerDown);
    }
    if w::icon_button(
        ui,
        button(x + 54.0),
        "layers.delete",
        "delete",
        lang.pick("レイヤーを削除", "Delete Layer"),
        false,
        has && can_delete,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::DeleteLayer);
    }
}

/// 層の種類の見た目（サムネイルの位置）。ラスターは絵、塗りつぶしは色（無ければアイコン）、調整・グループはアイコン。
#[allow(clippy::too_many_arguments)]
fn kind_thumb(
    ui: &mut Ui,
    app: &AppState,
    thumbs: &mut Thumbnails,
    ctx: &egui::Context,
    list: Rect,
    thumb: Rect,
    id: LayerId,
    expanded: bool,
) {
    let Some(layer) = app.doc.layer(id) else {
        return;
    };
    let painter = ui.painter_at(list);
    let channel = app.m2.display_channel;
    match layer.kind() {
        LayerKind::Raster => {
            w::checker(&painter, thumb, 3.0);
            let max_px = (thumb.width() * ctx.pixels_per_point()).round().max(8.0) as u32;
            if let Some(handle) =
                thumbs.get(ctx, &app.doc, id, max_px, ThumbSource::Channel(channel))
            {
                let [tw, th] = handle.size();
                let k = (thumb.width() / tw as f32).min(thumb.height() / th as f32);
                let at = Rect::from_center_size(thumb.center(), vec2(tw as f32 * k, th as f32 * k));
                painter.image(
                    handle.id(),
                    at,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            w::outline(&painter, thumb, t::BORDER, 1.0, 0.0);
        }
        LayerKind::Fill => {
            let value = layer
                .fill_value(channel)
                .or_else(|| layer.fill_value(app.m2.paint_channel))
                .or_else(|| layer.fill_values().next().map(|(_, v)| v));
            match value {
                Some(v) => {
                    w::checker(&painter, thumb, 3.0);
                    w::rounded(
                        &painter,
                        thumb,
                        Color32::from_rgba_unmultiplied(v.r, v.g, v.b, v.a),
                        3.0,
                    );
                    w::outline(&painter, thumb, t::BORDER, 1.0, 3.0);
                }
                None => {
                    w::rounded(&painter, thumb, t::PANEL_HEADER, 3.0);
                    w::icon(&painter, thumb, "format_color_fill", t::TEXT_DIM, 15.0);
                }
            }
        }
        LayerKind::Adjustment => {
            w::rounded(&painter, thumb, t::PANEL_HEADER, 3.0);
            let icon = layer
                .adjustment()
                .map(|a| AdjustmentKind::of(a).icon())
                .unwrap_or("tune");
            w::icon(&painter, thumb, icon, t::TEXT_DIM, 15.0);
        }
        LayerKind::Group => {
            w::rounded(&painter, thumb, t::PANEL_HEADER, 3.0);
            w::icon(
                &painter,
                thumb,
                if expanded { "folder_open" } else { "folder" },
                t::TEXT_DIM,
                15.0,
            );
        }
    }
}

/// ポインタの位置から、いまの落とす先を決める。
fn update_drag(ui: &Ui, app: &mut AppState, list: Rect, rows: &[Row], id: LayerId) {
    if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
        let position = (p.y - list.top() + app.layer_scroll) / ROW_HEIGHT;
        let target = m2::drop_target_at(&app.doc, rows, id, position);
        app.layer_drag = Some(LayerDrag { id, target });
    }
}

/// ドラッグを終える（落とす先があれば落とす）。
fn drop_drag(app: &mut AppState, rows: &[Row]) {
    if let Some(LayerDrag {
        id: dragged,
        target: Some(target),
    }) = app.layer_drag.take()
    {
        if let Some(edit) = m2::drop_edit(&app.doc, rows, dragged, target) {
            app.apply(Action::M2(edit));
        }
    }
    app.layer_drag = None;
}

/// ドラッグ中にホイールで一覧を送って、ドラッグしている行が見えなくなったとき。見えない行は描かない（応答が来ない）ので、
/// 行の側の「動いた・離した」が届かず、落とす先の線が残り続けて落とす操作も起きない。ここで、ボタンを押しているあいだは
/// 落とす先をポインタに追わせ、離したら行の側と同じく落として手放す。
fn follow_drag(ui: &Ui, app: &mut AppState, list: Rect, rows: &[Row]) {
    let Some(LayerDrag { id, .. }) = app.layer_drag else {
        return;
    };
    if ui.input(|i| i.pointer.primary_down()) {
        update_drag(ui, app, list, rows, id);
    } else {
        drop_drag(app, rows);
    }
}

#[allow(clippy::too_many_arguments)]
fn layer_row(
    ui: &mut Ui,
    app: &mut AppState,
    thumbs: &mut Thumbnails,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    info: Row,
    rows: &[Row],
) {
    let id = info.id;
    let Some(layer) = app.doc.layer(id) else {
        return;
    };
    let (name, visible, kind) = (layer.name().to_owned(), layer.visible(), layer.kind());
    let has_mask = layer.mask().is_some();
    let channel = app.m2.paint_channel;
    let no_pixels = kind == LayerKind::Raster && !layer.is_channel_enabled(channel);
    let clipped = app
        .doc
        .layer_index(id)
        .is_some_and(|i| app.doc.is_effectively_clipped(i));
    let selected = app.selected_layer == Some(id);
    let editing_mask = selected && app.m2.edit_mask && has_mask;
    let enabled = app.can_edit();
    let lang = app.lang;
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("layer.row", id.0)),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
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

    // 目・字下げ・開閉（グループ）か下地へのクリッピングの印・サムネイル・マスク・名前
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let mut x = eye.right() + 4.0 + INDENT * info.depth as f32;
    let fold = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(INDENT, row.height() - 6.0));
    if info.is_group || clipped {
        x += INDENT;
    }
    let thumb = Rect::from_min_size(
        pos2(x, row.top() + 4.0),
        vec2(row.height() - 8.0, row.height() - 8.0),
    );
    x = thumb.right() + 4.0;
    let mask_box = Rect::from_min_size(
        pos2(x, row.top() + 4.0),
        vec2(row.height() - 8.0, row.height() - 8.0),
    );
    if has_mask {
        x = mask_box.right() + 4.0;
    }
    let name_rect = Rect::from_min_max(
        pos2(x + 2.0, row.top() + 4.0),
        pos2(row.right() - 26.0, row.bottom() - 4.0),
    );

    // 選ぶ・ダブルクリックで名前・右クリックのメニュー・ドラッグで並べ替え
    if response.clicked() || response.drag_started() {
        if app.selected_layer != Some(id) {
            app.set_edit_mask(false);
        }
        app.selected_layer = Some(id);
        if app.renaming != Some(id) {
            app.renaming = None;
        }
    }
    if (response.double_clicked() || response.triple_clicked())
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.renaming = Some(id);
        app.rename_started = false;
    }
    if response.secondary_clicked() {
        app.selected_layer = Some(id);
        if let Some(at) = response.interact_pointer_pos() {
            open_popup(
                app,
                ctx,
                PopupKind::LayerContext(id),
                context_anchor(at),
                0.0,
            );
        }
    }
    if response.dragged() {
        update_drag(ui, app, list, rows, id);
    }
    if response.drag_stopped() {
        drop_drag(app, rows);
    }

    // 目
    if w::icon_button(
        ui,
        eye,
        ("layer.eye", id.0),
        if visible {
            "visibility"
        } else {
            "visibility_off"
        },
        if visible {
            lang.pick("非表示にする", "Hide")
        } else {
            lang.pick("表示する", "Show")
        },
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleVisible(id));
    }
    // グループの開閉・クリッピングの印
    let collapsed = app.m2.collapsed.contains(&id);
    if info.is_group {
        if w::icon_button(
            ui,
            fold,
            ("layer.fold", id.0),
            if collapsed {
                "chevron_right"
            } else {
                "expand_more"
            },
            if collapsed {
                lang.pick("グループを開く", "Expand Group")
            } else {
                lang.pick("グループを閉じる", "Collapse Group")
            },
            false,
            true,
            14.0,
        )
        .clicked()
        {
            app.apply(Action::M2Ui(UiOp::ToggleCollapsed(id)));
        }
    } else if clipped {
        w::icon(&painter, fold, "keyboard_arrow_down", t::TEXT_DIM, 14.0);
    }
    kind_thumb(ui, app, thumbs, ctx, list, thumb, id, !collapsed);
    // マスク（押すとマスクに描く）
    if has_mask {
        let maskr = ui.interact(
            mask_box,
            ui.make_persistent_id(("layer.mask", id.0)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let painter = ui.painter_at(list);
        w::checker(&painter, mask_box, 3.0);
        let max_px = (mask_box.width() * ctx.pixels_per_point()).round().max(8.0) as u32;
        if let Some(handle) = thumbs.get(ctx, &app.doc, id, max_px, ThumbSource::Mask) {
            let [tw, th] = handle.size();
            let k = (mask_box.width() / tw as f32).min(mask_box.height() / th as f32);
            let at = Rect::from_center_size(mask_box.center(), vec2(tw as f32 * k, th as f32 * k));
            painter.image(
                handle.id(),
                at,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        w::outline(&painter, mask_box, t::BORDER, 1.0, 0.0);
        if editing_mask {
            w::outline(&painter, mask_box.expand(2.0), t::ACCENT, 2.0, 2.0);
        }
        let tip = lang.pick(
            "レイヤーマスク（押すとマスクに描く）",
            "Layer mask (click to paint on it)",
        );
        maskr.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, editing_mask, tip));
        if maskr.clicked() {
            app.selected_layer = Some(id);
            app.apply(Action::M2Ui(UiOp::EditMask(!editing_mask)));
        }
        if maskr.secondary_clicked() {
            app.selected_layer = Some(id);
            if let Some(at) = maskr.interact_pointer_pos() {
                open_popup(
                    app,
                    ctx,
                    PopupKind::LayerContext(id),
                    context_anchor(at),
                    0.0,
                );
            }
        }
        let _ = maskr.on_hover_text(tip);
    }

    // 名前（ダブルクリックで変える）
    let painter = ui.painter_at(list);
    if app.renaming == Some(id) {
        let first = !app.rename_started;
        app.rename_started = true;
        let out = w::text_field(ui, name_rect, ("layer.rename", id.0), &name, None, first);
        if let Some(next) = out.committed {
            let next = next.trim().to_owned();
            if !next.is_empty() {
                if let Err(e) = app.doc.set_layer_name(id, &next) {
                    app.message = app.lang.core_error(&e);
                }
                app.modified = true;
            }
        }
        if !first && !out.focused {
            app.renaming = None;
        }
    } else {
        let color = if !visible {
            t::TEXT_DIM
        } else if selected {
            Color32::WHITE
        } else {
            t::TEXT
        };
        let shown = w::fit(&painter, &name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
    }
    // 右端の印: 描くチャンネルを使っていないラスターの層
    if no_pixels {
        let mark = Rect::from_min_size(
            pos2(row.right() - 24.0, row.top()),
            vec2(20.0, row.height()),
        );
        w::icon(&painter, mark, "link_off", t::TEXT_DISABLED, 13.0);
        ui.interact(
            mark,
            ui.make_persistent_id(("layer.nochannel", id.0)),
            Sense::hover(),
        )
        .on_hover_text(lang.pick(
            "このレイヤーは描くチャンネルを使っていません",
            "This layer does not use the paint channel",
        ));
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &name)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_follow_the_channel_and_forget_dead_layers() {
        let mut app = AppState::new(32, 32);
        let id = app.selected_layer.unwrap();
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::default();
        assert!(thumbs
            .get(&ctx, &app.doc, id, 16, ThumbSource::Channel(Channel::Color))
            .is_some());
        let built = thumbs.rebuilt;
        assert!(thumbs
            .get(&ctx, &app.doc, id, 16, ThumbSource::Channel(Channel::Color))
            .is_some());
        assert_eq!(thumbs.rebuilt, built, "文書が変わらなければ作り直さない");
        thumbs
            .get(
                &ctx,
                &app.doc,
                id,
                16,
                ThumbSource::Channel(Channel::Roughness),
            )
            .unwrap();
        assert_eq!(thumbs.rebuilt, built + 1, "チャンネルごとに作る");
        app.apply(Action::M2(Edit::AddMask(id)));
        thumbs
            .get(&ctx, &app.doc, id, 16, ThumbSource::Mask)
            .unwrap();
        app.apply(Action::M2(Edit::RemoveMask(id)));
        thumbs.retain(&app.doc);
        assert!(thumbs.map.keys().all(|(_, s)| *s != ThumbSource::Mask));
    }
}
