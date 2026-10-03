//! レイヤーのパネル（Unity 版の LayersPanel の配置）: 上に合成モードと不透明度、行（目・サムネイル・名前。ダブルクリックで名前を
//! 変える・右クリックのメニュー・ドラッグで並べ替え）、下に操作の帯（新規・上へ・下へ・削除）。行は上が一番上のレイヤー。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Rect, Sense, TextureHandle, TextureOptions, Ui, WidgetInfo,
    WidgetType,
};

use crate::engine::{layer_has_pixels, layer_pixel, Document, LayerId};
use crate::state::{blend_name, Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

pub const ROW_HEIGHT: f32 = 30.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// パネルの上の端から一覧の上の端まで（合成モードと不透明度の行）。
pub const LIST_TOP: f32 = 6.0 + 24.0 + 6.0;

/// レイヤーのサムネイル（文書の版が変わったら作り直す。core は層ごとの版を持たないので文書の版で見る。描いている間は毎フレーム）。
#[derive(Default)]
pub struct Thumbnails {
    map: HashMap<LayerId, (u64, TextureHandle)>,
    /// 作り直した回数（試験用）。
    pub rebuilt: usize,
}

impl Thumbnails {
    fn get(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        id: LayerId,
        max_px: u32,
    ) -> Option<&TextureHandle> {
        let layer = doc.layer(id)?;
        let revision = doc.revision();
        let fresh = self
            .map
            .get(&id)
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
            if layer_has_pixels(layer) {
                for ty in 0..th {
                    // 下から上の行を、画像の上から下へ並べ替える
                    let sy = (((ty as f32 + 0.5) * dh as f32 / th as f32) as u32).min(dh - 1);
                    for tx in 0..tw {
                        let sx = (((tx as f32 + 0.5) * dw as f32 / tw as f32) as u32).min(dw - 1);
                        let p = layer_pixel(layer, sx, sy);
                        pixels[((th - 1 - ty) * tw + tx) as usize] =
                            Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
                    }
                }
            }
            let image = ColorImage::new([tw as usize, th as usize], pixels);
            match self.map.get_mut(&id) {
                Some((r, handle)) if handle.size() == [tw as usize, th as usize] => {
                    handle.set(image, TextureOptions::LINEAR);
                    *r = revision;
                }
                _ => {
                    let handle =
                        ctx.load_texture(format!("thumb-{}", id.0), image, TextureOptions::LINEAR);
                    self.map.insert(id, (revision, handle));
                }
            }
            self.rebuilt += 1;
        }
        self.map.get(&id).map(|(_, h)| h)
    }

    fn retain(&mut self, doc: &Document) {
        self.map.retain(|id, _| doc.layer(*id).is_some());
    }
}

pub fn show(ui: &mut Ui, app: &mut AppState, thumbs: &mut Thumbnails) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let enabled = !app.is_stroking();
    let selected = app
        .selected_layer
        .and_then(|id| app.doc.layer(id).map(|l| (id, l.blend_mode(), l.opacity())));

    // 上: 合成モードと不透明度（Photoshop の配置）
    let top = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + 6.0),
        vec2(r.width() - 2.0 * t::PADDING, 24.0),
    );
    let left_w = ((top.width() - 6.0) * 0.42).floor();
    let blend_rect = Rect::from_min_size(top.min, vec2(left_w, top.height()));
    let opacity_rect = Rect::from_min_max(pos2(top.left() + left_w + 6.0, top.top()), top.max);
    if let Some((id, blend, opacity)) = selected {
        let (response, b) = w::dropdown(
            ui,
            blend_rect,
            "layers.blend",
            None,
            blend_name(blend),
            Some("合成モード"),
            enabled,
            0.0,
        );
        if response.clicked() {
            app.popup = Some(OpenPopup {
                kind: PopupKind::BlendMode(id),
                state: PopupState::new(&ctx, b).with_min_width(b.width()),
            });
        }
        let spec = SliderSpec::new("不透明度", 0.0, 100.0, NumberFormat::int("%"))
            .enabled(enabled)
            .tooltip("レイヤーの不透明度");
        let o = w::slider(
            ui,
            opacity_rect,
            "layers.opacity",
            (opacity * 100.0) as f32,
            &spec,
        );
        if o.changed {
            let _ = app
                .doc
                .set_layer_opacity(id, (o.value as f64 / 100.0).clamp(0.0, 1.0), true);
            app.modified = true;
        }
        if o.released {
            app.doc.end_coalescing();
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
    let ids: Vec<LayerId> = app.doc.layers().iter().rev().map(|l| l.id()).collect(); // 上から
    let n = ids.len();
    let content = n as f32 * ROW_HEIGHT;
    let max_scroll = (content - list.height()).max(0.0);
    if ui.rect_contains_pointer(list) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        app.layer_scroll -= wheel;
    }
    app.layer_scroll = app.layer_scroll.clamp(0.0, max_scroll);
    let row_width = list.width() - if max_scroll > 0.0 { 10.0 } else { 0.0 };
    for (row_index, id) in ids.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + row_index as f32 * ROW_HEIGHT - app.layer_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        layer_row(ui, app, thumbs, &ctx, list, row, *id, n, row_index);
    }
    // ドラッグの落とす先の線
    if let Some((_, gap)) = app.layer_drag {
        let y = list.top() + gap as f32 * ROW_HEIGHT - app.layer_scroll;
        ui.painter_at(list).rect_filled(
            Rect::from_min_size(pos2(list.left() + 4.0, y - 1.0), vec2(row_width - 8.0, 2.0)),
            0.0,
            t::ACCENT,
        );
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

    // 下: 操作
    let bar = Rect::from_min_size(
        pos2(r.left(), list.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    if w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "layers.add",
        "add",
        "新規レイヤー",
        false,
        enabled,
        18.0,
    )
    .clicked()
    {
        app.apply(Action::NewLayer);
    }
    let has = selected.is_some() && enabled;
    let x = bar.right() - 4.0 - 27.0 * 3.0;
    if w::icon_button(
        ui,
        button(x),
        "layers.up",
        "expand_less",
        "レイヤーを上へ",
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
        "レイヤーを下へ",
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
        "レイヤーを削除",
        false,
        has && n > 1,
        17.0,
    )
    .clicked()
    {
        app.apply(Action::DeleteLayer);
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
    id: LayerId,
    n: usize,
    row_index: usize,
) {
    let Some(layer) = app.doc.layer(id) else {
        return;
    };
    let (name, visible) = (layer.name().to_owned(), layer.visible());
    let selected = app.selected_layer == Some(id);
    let enabled = !app.is_stroking();
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

    // 名前の矩形（目とサムネイルの右）
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let thumb = Rect::from_min_size(
        pos2(eye.right() + 4.0, row.top() + 4.0),
        vec2(row.height() - 8.0, row.height() - 8.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(thumb.right() + 6.0, row.top() + 4.0),
        pos2(row.right() - 26.0, row.bottom() - 4.0),
    );

    // 選ぶ・ダブルクリックで名前・右クリックのメニュー・ドラッグで並べ替え
    if response.clicked() || response.drag_started() {
        app.selected_layer = Some(id);
        if app.renaming != Some(id) {
            app.renaming = None;
        }
    }
    if response.double_clicked()
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
            app.popup = Some(OpenPopup {
                kind: PopupKind::LayerContext(id),
                state: PopupState::new(ctx, context_anchor(at)),
            });
        }
    }
    if response.dragged() {
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
            let gap = ((p.y - list.top() + app.layer_scroll) / ROW_HEIGHT)
                .round()
                .clamp(0.0, n as f32) as usize;
            app.layer_drag = Some((id, gap));
        }
    }
    if response.drag_stopped() {
        if let Some((dragged, gap)) = app.layer_drag.take() {
            drop_layer(app, dragged, gap);
        }
    }
    let _ = row_index;

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
            "非表示にする"
        } else {
            "表示する"
        },
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleVisible(id));
    }
    // サムネイル（市松の上に、文書の縦横比で）
    let painter = ui.painter_at(list);
    w::checker(&painter, thumb, 3.0);
    let max_px = (thumb.width() * ctx.pixels_per_point()).round().max(8.0) as u32;
    if let Some(handle) = thumbs.get(ctx, &app.doc, id, max_px) {
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

    // 名前（ダブルクリックで変える）
    if app.renaming == Some(id) {
        let first = !app.rename_started;
        app.rename_started = true;
        let out = w::text_field(ui, name_rect, ("layer.rename", id.0), &name, None, first);
        if let Some(next) = out.committed {
            let next = next.trim().to_owned();
            if !next.is_empty() {
                if let Err(e) = app.doc.set_layer_name(id, &next) {
                    app.message = e.to_string();
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
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &name)
    });
}

/// ドラッグで落とした所へ移す（gap は上から数えた行の隙間。gap の行のすぐ上に置く。n なら一番下）。
pub fn drop_layer(app: &mut AppState, dragged: LayerId, gap: usize) {
    let order: Vec<LayerId> = app.doc.layers().iter().map(|l| l.id()).collect(); // 下から
    let n = order.len();
    let target = if gap >= n {
        0
    } else {
        let below = order[n - 1 - gap]; // 線のすぐ下の行のレイヤー。その上に置く
        if below == dragged {
            return;
        }
        let rest: Vec<LayerId> = order.iter().copied().filter(|l| *l != dragged).collect();
        rest.iter()
            .position(|l| *l == below)
            .map(|i| i + 1)
            .unwrap_or(0)
    };
    if app.doc.layer_index(dragged) != Some(target) {
        let _ = app.doc.move_layer(dragged, target);
        app.modified = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_places_above_the_row_under_the_line() {
        let mut app = AppState::new(32, 32);
        app.apply(Action::NewLayer);
        app.apply(Action::NewLayer); // 下から 1, 2, 3
        let ids: Vec<LayerId> = app.doc.layers().iter().map(|l| l.id()).collect();
        // 一番上（3）を、上から 2 行目（2）と 3 行目（1）の間へ
        drop_layer(&mut app, ids[2], 2);
        let names: Vec<&str> = app.doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["レイヤー 1", "レイヤー 3", "レイヤー 2"]);
        // 一番下へ
        drop_layer(&mut app, ids[1], 3);
        assert_eq!(app.doc.layers()[0].id(), ids[1]);
    }
}
