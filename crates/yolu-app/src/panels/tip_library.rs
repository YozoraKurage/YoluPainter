//! 詳細の窓の「形状」の筆先の選びに、同梱の Krita の筆先（76 個）の格子を足す部品。名前で絞れ、見本は別のスレッドで作る
//! （`brushes::krita`）。今の筆先の見本（取り込んだ画像・Krita の筆先も出せる）もここ。

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use egui::{
    pos2, vec2, Color32, Id, Rect, Sense, TextureHandle, TextureId, TextureOptions, Ui, WidgetInfo,
    WidgetType,
};
use yolu_core::{BrushTip, TipShape};

use super::properties::group_label;
use crate::brushes::krita;
use crate::lang::Lang;
use crate::m2::{BrushOp, UiOp};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Rows};

/// 見本の 1 マスと間（組み込みの筆先の格子と同じ）。
pub const CELL: f32 = 40.0;
pub const CELL_GAP: f32 = 4.0;

/// 今の筆先の種類。
pub enum Current {
    Round,
    Builtin(&'static str),
    /// 同梱の Krita の筆先（同梱の並びの添字）。
    Krita(usize),
    /// 取り込んだ画像（ホースなら 1 枚目）。
    Image(Arc<BrushTip>),
}

pub fn current(tip: &TipShape) -> Current {
    if tip.image.is_none() && tip.images.is_empty() {
        return Current::Round;
    }
    if tip.images.is_empty() {
        if let Some(image) = &tip.image {
            // 名前だけ同じで中身が違う（取り込んだ）画像は、組み込みとして扱わない
            let builtin = yolu_core::brush::BUILTIN_TIPS
                .into_iter()
                .find(|id| yolu_core::builtin_tip(id).is_some_and(|b| *b == **image));
            if let Some(id) = builtin {
                return Current::Builtin(id);
            }
        }
    }
    if let Some(index) = krita::current_index(tip) {
        return Current::Krita(index);
    }
    match tip.image.clone().or_else(|| tip.images.first().cloned()) {
        Some(image) => Current::Image(image),
        None => Current::Round,
    }
}

/// 取り込んだ画像の見本の絵（画像ごとに 1 回だけ作る。画像が無くなったものは捨てる）。
#[derive(Clone, Default)]
struct ImageThumbs(HashMap<usize, (Weak<BrushTip>, TextureHandle)>);

const MAX_IMAGE_THUMBS: usize = 32;

pub fn image_texture(ctx: &egui::Context, tip: &Arc<BrushTip>) -> TextureId {
    let cache_id = Id::new("yolu.image-thumbs");
    let mut cache: ImageThumbs = ctx.data(|d| d.get_temp(cache_id)).unwrap_or_default();
    let at = Arc::as_ptr(tip) as usize;
    if let Some((alive, handle)) = cache.0.get(&at) {
        if alive.strong_count() > 0 {
            return handle.id();
        }
    }
    if cache.0.len() >= MAX_IMAGE_THUMBS {
        cache.0.retain(|_, (alive, _)| alive.strong_count() > 0);
    }
    if cache.0.len() >= MAX_IMAGE_THUMBS {
        cache.0.clear();
    }
    let handle = ctx.load_texture(
        format!("tip-image-{at:x}"),
        krita::thumbnail(Some(tip)),
        TextureOptions::LINEAR,
    );
    let id = handle.id();
    cache.0.insert(at, (Arc::downgrade(tip), handle));
    ctx.data_mut(|d| d.insert_temp(cache_id, cache));
    id
}

/// 筆先の見本の 1 マス（白地に見本。選んでいれば枠）。押されたら true。
fn cell(
    ui: &mut Ui,
    r: Rect,
    index: usize,
    texture: Option<TextureId>,
    selected: bool,
    name: &str,
) -> bool {
    let response = ui.interact(
        r,
        ui.make_persistent_id(("krita.tip", index)),
        Sense::click(),
    );
    let p = ui.painter();
    w::rounded(p, r, Color32::WHITE, 3.0);
    if let Some(texture) = texture {
        p.image(
            texture,
            r.shrink(2.0),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if selected {
        w::outline(p, r.expand(2.0), t::ACCENT, 2.0, 4.0);
    } else if response.hovered() {
        w::outline(p, r.expand(1.0), t::ACCENT_DIM, 1.0, 4.0);
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, name));
    let clicked = response.clicked();
    let _ = response.on_hover_text(name);
    clicked
}

/// 名前の検索の欄（打つたびに絞る）。
fn search(ui: &mut Ui, rows: &mut Rows, app: &mut AppState, lang: Lang) {
    let r = rows.row(24.0, 6.0);
    super::assets::search_field(
        ui,
        r,
        "krita.search",
        &mut app.brushes.krita.search,
        lang.pick("名前で探す", "Search by name"),
    );
}

/// Krita の筆先の格子（検索の欄と、見本の格子）。読み込みが終わるまでは、薄い空のマスを並べる。
pub fn krita_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    app.brushes.krita.poll(ctx);
    rows.space(5.0);
    group_label(ui, rows, "Krita");
    let ready = app.brushes.krita.is_ready();
    if ready {
        search(ui, rows, app, lang);
    }
    let columns = (((rows.width() + CELL_GAP) / (CELL + CELL_GAP)).floor() as usize).max(1);
    if !ready {
        // 読み込み中: 空のマス（見本ができたら入れ替わる）
        for _ in 0..3 {
            let line = rows.row(CELL, CELL_GAP);
            for k in 0..columns {
                let r = Rect::from_min_size(
                    pos2(line.left() + k as f32 * (CELL + CELL_GAP), line.top()),
                    vec2(CELL, CELL),
                );
                w::rounded(ui.painter(), r, t::CONTROL_BG, 3.0);
            }
        }
        return;
    }
    let selected = krita::current_index(&app.m2.brush.tip);
    let indices = app.brushes.krita.matches();
    for chunk in indices.chunks(columns) {
        let line = rows.row(CELL, CELL_GAP);
        for (k, index) in chunk.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(line.left() + k as f32 * (CELL + CELL_GAP), line.top()),
                vec2(CELL, CELL),
            );
            let name = app
                .brushes
                .krita
                .name(*index)
                .unwrap_or_default()
                .to_owned();
            let texture = app.brushes.krita.texture(ctx, *index);
            if cell(ui, r, *index, texture, selected == Some(*index), &name) {
                app.apply(Action::M2Ui(UiOp::Brush(BrushOp::KritaTip(*index))));
            }
        }
    }
}
