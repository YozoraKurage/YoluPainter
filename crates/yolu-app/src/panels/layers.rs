//! レイヤーのパネル（Unity 版の LayersPanel の配置）: 上に描くチャンネルの合成モードと不透明度（左端の切り替えで、そのチャンネルだけの
//! 値にする）、そのすぐ下に操作の帯（左端にクリッピング、右に新規・塗りつぶし・グループ・調整・削除）、その下に行（目・グループの開閉・
//! サムネイル・マスク・名前。ダブルクリックで名前を変える・右クリックのメニュー・ドラッグで並べ替えとグループへの出し入れ）。
//! 行は上が一番上のレイヤーで、グループの中身は字下げして続く（閉じたグループの中身は出さない）。クリッピングしているレイヤーは
//! 字下げせず、サムネイルの左の細い縦の帯で示す。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui,
    WidgetInfo, WidgetType,
};

use yolu_core::FilterTarget;

use crate::engine::{Channel, Document, LayerId, LayerKind};
use crate::layerops::lock_names;
use crate::m2::{self, AdjustmentKind, DropTarget, Edit, LayerDrag, MaskDrag, Row, UiOp};
use crate::m2_menu::Popup;
use crate::panels::effect_rows;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

pub const ROW_HEIGHT: f32 = 30.0;
/// 操作の帯の高さ。
pub const TOOLBAR_HEIGHT: f32 = 26.0;
/// 合成モードと不透明度の行の高さ・上の余白・次の帯との間。
const TOP_HEIGHT: f32 = 20.0;
const TOP_MARGIN: f32 = 4.0;
const TOP_GAP: f32 = 4.0;
/// パネルの上の端から一覧の上の端まで（合成モードと不透明度の行、操作の帯）。パネルが狭くて 1 行に収まらないときは、合成モードと
/// 不透明度を 2 行に積む（`STACKED_LIST_TOP`）。
pub const LIST_TOP: f32 = TOP_MARGIN + TOP_HEIGHT + TOP_GAP + TOOLBAR_HEIGHT;
/// 合成モードと不透明度を 2 行に積んだときの、パネルの上の端から一覧の上の端まで。
pub const STACKED_LIST_TOP: f32 = LIST_TOP + TOP_HEIGHT + TOP_GAP;
/// クリッピングしている行の縦の帯の幅（サムネイルの左、選んだ行の囲いから 1 px 離す）。
const CLIPPING_MARK_WIDTH: f32 = 2.5;
/// 1 段の字下げ。
const INDENT: f32 = 14.0;
/// 行の右端の印（描くチャンネルを使っていないレイヤー・パスレイヤー）の幅。
const MARK_WIDTH: f32 = 20.0;

/// 右から `slot` 番目（0 が右端）の印の左端が、行の右端から内へどれだけか。
fn mark_inset(slot: usize) -> f32 {
    24.0 + MARK_WIDTH * slot as f32
}

/// マスクのサムネイルの印のツールチップの 1 行（マスクの効果の数。レイヤーが対象のあいだ、それらの行は一覧に出ない）。
pub fn mask_effects_tip(lang: crate::lang::Lang, count: usize) -> String {
    match (count, lang) {
        (1, crate::lang::Lang::En) => "1 effect on the mask".to_owned(),
        (n, crate::lang::Lang::En) => format!("{n} effects on the mask"),
        (n, crate::lang::Lang::Ja) => format!("マスクに効果 {n} 件"),
    }
}

/// 名前欄の右端が、行の右端から内へどれだけか（印が `marks` 個）。印が無いときと 1 つのときは同じ（1 つぶんの場所を空けておく）で、
/// 2 つ並ぶときだけ 2 つ目のぶん狭める。
fn name_right_inset(marks: usize) -> f32 {
    26.0 + MARK_WIDTH * marks.saturating_sub(1) as f32
}

/// サムネイルの絵の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbSource {
    /// そのチャンネルの画素。
    Channel(Channel),
    /// マスク（見せる量を灰色で）。
    Mask,
}

/// レイヤーのサムネイル（文書の版が変わったら作り直す。core はレイヤーごとの版を持たないので文書の版で見る。描いている間は毎フレーム）。
#[derive(Default)]
pub struct Thumbnails {
    map: HashMap<(LayerId, ThumbSource), (u64, TextureHandle)>,
    /// 作り直した回数（試験用）。
    pub rebuilt: usize,
}

/// レイヤーの 1 画素（straight RGBA8）。マスクは見せる量の灰色。
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

/// 描くチャンネルだけの合成モードと不透明度のときの印: 合成モードの箱の上の縁に、チャンネルの名前を小さく（縁の線を切って重ねる）。
fn own_legend(ui: &mut Ui, blend: Rect, name: &str, tip: &str) {
    let p = ui.painter();
    let shown = w::fit(p, name, blend.width() - 16.0, t::LABEL_SMALL);
    let width = w::text_width(p, &shown, t::LABEL_SMALL);
    let at = Rect::from_min_size(
        pos2(blend.left() + 6.0, blend.top() - 6.0),
        vec2(width + 4.0, 11.0),
    );
    w::fill(p, at, t::PANEL_BG);
    w::text(
        p,
        at.translate(vec2(2.0, 0.0)),
        &shown,
        t::LABEL_SMALL.with_color(t::ACCENT),
        Align::Left,
    );
    let response = ui.interact(at, ui.make_persistent_id("layers.own"), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    response.on_hover_text(tip);
}

/// レイヤーセットのドロップダウンの高さ。
const SET_HEAD_HEIGHT: f32 = 22.0;
/// 出力なしの印の幅（ドロップダウンの右）。
const SET_MARK_WIDTH: f32 = 22.0;

/// 頭のレイヤーセットのドロップダウン（レイヤーセットが 1 つだけなら出さず、0 を返す）。返すのは、頭が下へ押す高さ（ドロップダウンと次の帯の間を含む）。
fn layer_set_head(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    r: Rect,
    enabled: bool,
) -> f32 {
    let doc = &app.session.doc;
    if doc.layer_sets().len() < 2 {
        return 0.0;
    }
    let lang = app.lang;
    let set = app.layer_set();
    let missing = crate::m2::layersets::is_off_output(doc, set, app.m2.paint_channel);
    let mark = if missing { SET_MARK_WIDTH + 4.0 } else { 0.0 };
    let name = crate::m2::layersets::set_label(lang, doc, set);
    let at = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + TOP_MARGIN),
        vec2(r.width() - 2.0 * t::PADDING - mark, SET_HEAD_HEIGHT),
    );
    let (response, b) = w::dropdown(
        ui,
        at,
        "layers.layerset",
        None,
        &name,
        Some(lang.t("layersets-layer-set")),
        enabled,
        0.0,
    );
    if response.clicked() {
        open_popup(
            app,
            ctx,
            PopupKind::M2(crate::m2_menu::Popup::LayerSet),
            b,
            b.width(),
        );
    }
    if missing {
        let icon = Rect::from_min_size(
            pos2(at.right() + 4.0, at.top()),
            vec2(SET_MARK_WIDTH, SET_HEAD_HEIGHT),
        );
        w::icon(ui.painter(), icon, "link_off", t::WARNING, 17.0);
        let tip = lang.t("layersets-no-output-tip");
        let hit = ui.interact(
            icon,
            ui.make_persistent_id("layers.layerset.off"),
            Sense::hover(),
        );
        hit.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, tip));
        hit.on_hover_text(tip);
    }
    SET_HEAD_HEIGHT + TOP_GAP
}

fn open_popup(app: &mut AppState, ctx: &egui::Context, kind: PopupKind, anchor: Rect, min: f32) {
    app.ui.popup = Some(OpenPopup {
        kind,
        state: PopupState::new(ctx, anchor).with_min_width(min),
    });
}

/// 一覧の空白（行の下）の右クリックで、レイヤーのメニューを開く。
fn blank_menu(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    content: f32,
    enabled: bool,
) {
    let blank = Rect::from_min_max(
        pos2(
            list.left(),
            (list.top() + content - app.ui.layer_scroll).max(list.top()),
        ),
        list.max,
    );
    if blank.height() <= 0.0 {
        return;
    }
    let response = ui.interact(blank, ui.make_persistent_id("layers.blank"), Sense::click());
    if response.secondary_clicked() && enabled {
        if let Some(at) = response.interact_pointer_pos() {
            open_popup(
                app,
                ctx,
                PopupKind::M2(Popup::LayerBlank),
                context_anchor(at),
                0.0,
            );
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut AppState, thumbs: &mut Thumbnails) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let enabled = app.can_edit();
    let lang = app.lang;
    let channel = app.m2.paint_channel;
    let selected = app.session.selected_layer.and_then(|id| {
        app.session.doc.layer(id).map(|l| {
            (
                id,
                l.blend_mode_in(channel),
                l.opacity_in(channel),
                !l.channel_blend(channel).is_empty(),
            )
        })
    });

    // 頭: レイヤーセットのドロップダウン（2 つ以上あるときだけ）。今の描くチャンネルが、このレイヤーセットから最終の結果に出ていないときは、
    // 右に印だけ（理由はツールチップ）
    let head = layer_set_head(ui, app, &ctx, r, enabled);

    // 上: 描くチャンネルの合成モードと不透明度（Photoshop の配置）。そのチャンネルだけの値にするのは、合成モードのメニューの頭の項目
    let rest = Rect::from_min_size(
        pos2(r.left() + t::PADDING, r.top() + TOP_MARGIN + head),
        vec2(r.width() - 2.0 * t::PADDING, TOP_HEIGHT),
    );
    let left_w = ((rest.width() - 6.0) * 0.42).floor();
    let mut blend_rect = Rect::from_min_size(rest.min, vec2(left_w, rest.height()));
    let mut opacity_rect =
        Rect::from_min_max(pos2(rest.left() + left_w + 6.0, rest.top()), rest.max);
    // 1 行の不透明度のスライダーは、名前と値（幅広の数字）と左右の余白が要る。収まらないほど狭いときは、名前を詰める代わりに
    // 合成モードの下の行へ積み、名前を詰めずに出す
    let opacity_label = lang.t("panels-opacity");
    let opacity_need = {
        let p = ui.painter();
        w::text_width(p, opacity_label, t::LABEL) + w::text_width(p, "100%", t::VALUE) + 8.0 + 14.0
    };
    let stacked = opacity_rect.width() < opacity_need;
    if stacked {
        blend_rect = Rect::from_min_size(rest.min, rest.size());
        opacity_rect = blend_rect.translate(vec2(0.0, TOP_HEIGHT + TOP_GAP));
    }
    let list_top = head + if stacked { STACKED_LIST_TOP } else { LIST_TOP };
    if let Some((id, blend, opacity, own)) = selected {
        let name = m2::channel_name(lang, &app.session.doc, channel);
        let (blend_tip, opacity_tip) = if own {
            (
                lang.f("panels-blend-mode-only-tip", &[("name", &name)]),
                lang.f("panels-opacity-only-tip", &[("name", &name)]),
            )
        } else {
            (
                lang.t("panels-blend-mode-tip").to_owned(),
                lang.t("panels-layer-opacity-tip").to_owned(),
            )
        };
        let (response, b) = w::dropdown(
            ui,
            blend_rect,
            "layers.blend",
            None,
            m2::blend_label(lang, blend),
            Some(&blend_tip),
            enabled,
            0.0,
        );
        if own {
            own_legend(ui, b, &name, &blend_tip);
        }
        if response.clicked() {
            open_popup(app, &ctx, PopupKind::BlendMode(id), b, b.width());
        }
        let spec = SliderSpec::new(opacity_label, 0.0, 100.0, NumberFormat::int("%"))
            .enabled(enabled)
            .absolute()
            .tooltip(&opacity_tip);
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
    thumbs.retain(&app.session.doc);
    let list = Rect::from_min_max(
        pos2(r.left(), r.top() + list_top),
        r.max.max(pos2(r.right(), r.top() + list_top)),
    );
    w::fill(ui.painter(), list, t::CONTROL_BG);
    let rows = m2::visible_rows_in(&app.session.doc, &app.m2.collapsed, app.layer_set());
    // レイヤーの行の下に効果の行（高さが違う。マスクが対象のレイヤーはマスクの効果、それ以外はレイヤーの効果）が続くので、行の位置は配置を数えて決める。
    // ドラッグの落とす先の数え方も、このフレームで描いた配置と同じ物を使う（途中で対象が替わっても、見えている行とずれない）
    let layout = effect_rows::layout(
        &app.session.doc,
        &rows,
        ROW_HEIGHT,
        crate::fx::mask_target(app),
    );
    let content = layout.height;
    let bar = Scroll::begin(ui, list, content, &mut app.ui.layer_scroll);
    let row_width = list.width() - bar.reserved();

    blank_menu(ui, app, &ctx, list, content, enabled);

    let chosen = app.selected_layers();
    // 送って一覧の上へはみ出た行の部品（目・開閉・サムネイル・印）が、一覧の外（上の帯など）で当たりを持たないよう、
    // 行を描くあいだだけ clip rect を一覧に絞る（egui は部品の当たりを clip rect で切る）
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(outer_clip.intersect(list));
    for entry in &layout.entries {
        let rect = Rect::from_min_size(
            pos2(list.left(), list.top() + entry.y - app.ui.layer_scroll),
            vec2(row_width, entry.height),
        );
        if rect.bottom() < list.top() || rect.top() > list.bottom() {
            continue;
        }
        match entry.kind {
            effect_rows::Kind::Layer(row_index) => layer_row(
                ui,
                app,
                thumbs,
                &ctx,
                list,
                rect,
                rows[row_index],
                &rows,
                &layout,
                &chosen,
            ),
            effect_rows::Kind::Child(child) => {
                effect_rows::child_row(ui, app, list, rect, entry, child)
            }
        }
    }
    ui.set_clip_rect(outer_clip);
    follow_drag(ui, app, list, &rows, &layout);
    follow_mask_drag(ui, app, list, &rows, &layout);
    crate::rulers::layer_icon::follow(ui, app, list, &rows, &layout, app.ui.layer_scroll);
    crate::panels::assets::layer_list_drop(ui, app, list, &rows, &layout);
    draw_drag_marks(ui, app, thumbs, list, row_width, &rows, &layout);
    bar.end(ui, "layers.scroll", &mut app.ui.layer_scroll);

    let bar = Rect::from_min_size(
        pos2(r.left(), r.top() + list_top - TOOLBAR_HEIGHT),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    toolbar(ui, app, &ctx, bar, enabled);
}

/// 操作の帯のボタンの数（左端にクリッピング 1 つ・右に 5 つ）。
const TOOLBAR_BUTTONS: usize = 6;

/// クリッピングの切り替えの名前（ツールチップと、試験・読み上げの名前）。
pub fn clipping_name(lang: crate::lang::Lang) -> &'static str {
    lang.t("panels-clip-to-the-layer-below")
}

/// クリッピングのボタンのツールチップ（押せないときは名前に理由を添える。試験・読み上げの名前も同じ）。
pub fn clipping_tooltip(lang: crate::lang::Lang, reason: Option<&str>) -> String {
    match reason {
        Some(reason) => lang.f(
            "panels-name-with-reason",
            &[("name", &clipping_name(lang)), ("reason", &reason)],
        ),
        None => clipping_name(lang).to_owned(),
    }
}

/// 選んでいるレイヤーのクリッピング: (入っているか, 押せない理由)。入れるには同じグループの中にすぐ下のレイヤーが要る（一番下は
/// 何にもクリッピングできない）。すでに入っているレイヤーは、下が無くても外せる。レイヤーが選ばれていなければ (切, None)。
pub fn clipping_state(app: &AppState) -> (bool, Option<&'static str>) {
    let lang = app.lang;
    let Some(layer) = app
        .session
        .selected_layer
        .and_then(|id| app.session.doc.layer(id))
    else {
        return (false, None);
    };
    if layer.clipping() {
        return (true, None);
    }
    let Some(index) = app.session.doc.layer_index(layer.id()) else {
        return (false, None);
    };
    let parent = layer.parent();
    let set = layer.layer_set();
    if app.session.doc.layers()[..index]
        .iter()
        .any(|l| l.parent() == parent && l.layer_set() == set)
    {
        (false, None)
    } else if parent.is_some() {
        (false, Some(lang.t("panels-bottom-layer-of-its-group")))
    } else {
        (false, Some(lang.t("panels-bottom-layer")))
    }
}

/// 操作の帯。左端にクリッピング、右へ向かって新規・塗りつぶし・グループ・調整・削除（いちばん右が削除）。
fn toolbar(ui: &mut Ui, app: &mut AppState, ctx: &egui::Context, bar: Rect, enabled: bool) {
    let lang = app.lang;
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    // 狭いパネルでは、ボタンが重ならないよう間隔を詰める
    let pitch = ((bar.width() - 8.0) / TOOLBAR_BUTTONS as f32).min(27.0);
    let button = |x: f32| {
        Rect::from_min_size(
            pos2(x, bar.top() + 1.0),
            vec2(pitch - 1.0, bar.height() - 2.0),
        )
    };
    // 狭いパネルでは、ボタンの幅に合わせてアイコンも小さくする（隣のアイコンに触れない）
    let icon = |base: f32| base.min((pitch - 2.0).max(10.0));
    let has = app
        .session
        .selected_layer
        .and_then(|id| app.session.doc.layer(id))
        .is_some()
        && enabled;
    let removed: usize = app
        .session
        .doc
        .topmost_of(&app.selected_layers())
        .unwrap_or_default()
        .iter()
        .map(|id| m2::subtree_len(&app.session.doc, *id))
        .sum();
    let can_delete =
        app.session.selected_layer.is_some() && app.session.doc.layers().len() > removed;

    // クリッピング（すぐ下のレイヤーの中にだけ描く。押している状態が分かる切り替え）
    let (clipping, clip_reason) = clipping_state(app);
    let clip_tip = clipping_tooltip(lang, clip_reason);
    if w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "layers.clipping",
        "clip_below",
        &clip_tip,
        clipping,
        has && clip_reason.is_none(),
        icon(18.0),
    )
    .clicked()
    {
        if let Some(id) = app.session.selected_layer {
            app.apply(Action::M2(Edit::Clipping(id, !clipping)));
        }
    }

    // 右端に詰めて、左から新規・塗りつぶし・グループ・調整・削除
    let right = bar.right() - 4.0 - pitch * (TOOLBAR_BUTTONS - 1) as f32;
    let mut x = right;
    let mut next = || {
        let b = button(x);
        x += pitch;
        b
    };
    if w::icon_button(
        ui,
        next(),
        "layers.add",
        "add",
        lang.t("panels-new-layer-tip"),
        false,
        enabled,
        icon(18.0),
    )
    .clicked()
    {
        app.apply(Action::NewLayer);
    }
    let b = next();
    if w::icon_button(
        ui,
        b,
        "layers.fill",
        "format_color_fill",
        lang.t("panels-new-fill-layer-tip"),
        false,
        enabled,
        icon(18.0),
    )
    .clicked()
    {
        open_popup(app, ctx, PopupKind::M2(Popup::NewFill), b, 0.0);
    }
    if w::icon_button(
        ui,
        next(),
        "layers.group",
        "folder",
        lang.t("panels-group-layers-tip"),
        false,
        has,
        icon(18.0),
    )
    .clicked()
    {
        app.apply(Action::M2(Edit::GroupSelected));
    }
    let b = next();
    if w::icon_button(
        ui,
        b,
        "layers.adjustment",
        "texture",
        lang.t("panels-new-adjustment-layer-tip"),
        false,
        enabled,
        icon(18.0),
    )
    .clicked()
    {
        open_popup(app, ctx, PopupKind::M2(Popup::NewAdjustment), b, 0.0);
    }
    if w::icon_button(
        ui,
        next(),
        "layers.delete",
        "delete",
        lang.t("panels-delete-layer-tip"),
        false,
        has && can_delete,
        icon(18.0),
    )
    .clicked()
    {
        app.apply(Action::DeleteLayer);
    }
}

/// 調整レイヤーの行のサムネイルの絵: 調整の種類ごとの絵。種類が分からないときは、調整レイヤー全般の絵。
pub fn adjustment_icon(settings: Option<&crate::engine::AdjustmentSettings>) -> &'static str {
    settings.map_or_else(
        || m2::layer_kind_icon(LayerKind::Adjustment),
        |a| AdjustmentKind::of(a).icon(),
    )
}

/// レイヤーの種類の見た目（サムネイルの位置）。ラスターは絵、塗りつぶしは色（無ければアイコン）、調整・グループはアイコン。
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
    let Some(layer) = app.session.doc.layer(id) else {
        return;
    };
    let painter = ui.painter_at(list);
    let channel = app.m2.display_channel;
    match layer.kind() {
        LayerKind::Raster => {
            w::checker(&painter, thumb, 3.0);
            let max_px = (thumb.width() * ctx.pixels_per_point()).round().max(8.0) as u32;
            if let Some(handle) = thumbs.get(
                ctx,
                &app.session.doc,
                id,
                max_px,
                ThumbSource::Channel(channel),
            ) {
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
            w::icon(
                &painter,
                thumb,
                adjustment_icon(layer.adjustment()),
                t::TEXT_DIM,
                15.0,
            );
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

/// ドラッグで運ぶレイヤー: つかんだレイヤーが複数選択の中にあれば選んだレイヤーの全部、なければそれだけ。
fn dragged_layers(app: &AppState, id: LayerId) -> Vec<LayerId> {
    let chosen = app.selected_layers();
    if chosen.len() > 1 && chosen.contains(&id) {
        chosen
    } else {
        vec![id]
    }
}

/// ポインタの位置から、いまの落とす先を決める。
fn update_drag(
    ui: &Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &effect_rows::Layout,
    id: LayerId,
) {
    if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
        let position = layout.position_at(p.y - list.top() + app.ui.layer_scroll);
        let target =
            m2::drop_target_for(&app.session.doc, rows, &dragged_layers(app, id), position);
        app.ui.layer_drag = Some(LayerDrag { id, target });
    }
}

/// ドラッグを終える（落とす先があれば落とす。複数選んでいれば選んだレイヤーをまとめて、1 回の Undo）。
fn drop_drag(app: &mut AppState, rows: &[Row]) {
    if let Some(LayerDrag {
        id: dragged,
        target: Some(target),
    }) = app.ui.layer_drag.take()
    {
        let ids = dragged_layers(app, dragged);
        if let Some(edit) = m2::drop_edit_for(&app.session.doc, rows, &ids, target) {
            app.apply(Action::M2(edit));
        }
    }
    app.ui.layer_drag = None;
}

/// ドラッグ中にホイールで一覧を送って、ドラッグしている行が見えなくなったとき。見えない行は描かない（応答が来ない）ので、
/// 行の側の「動いた・離した」が届かず、落とす先の線が残り続けて落とす操作も起きない。ここで、ボタンを押しているあいだは
/// 落とす先をポインタに追わせ、離したら行の側と同じく落として手放す。
fn follow_drag(
    ui: &Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &effect_rows::Layout,
) {
    let Some(LayerDrag { id, .. }) = app.ui.layer_drag else {
        return;
    };
    if ui.input(|i| i.pointer.primary_down()) {
        update_drag(ui, app, list, rows, layout, id);
    } else {
        drop_drag(app, rows);
    }
}

/// ドラッグの印: 並べ替えの落とす先（線か、グループの枠）と、マスクの落とす先・つかんだマスク。
fn draw_drag_marks(
    ui: &Ui,
    app: &AppState,
    thumbs: &mut Thumbnails,
    list: Rect,
    row_width: f32,
    rows: &[Row],
    layout: &effect_rows::Layout,
) {
    if let Some(LayerDrag {
        target: Some(target),
        ..
    }) = app.ui.layer_drag
    {
        let painter = ui.painter_at(list);
        match target {
            DropTarget::Gap(gap) => {
                let y = list.top() + layout.gap_y(gap) - app.ui.layer_scroll;
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
                    let y = list.top() + layout.layer_y(i) - app.ui.layer_scroll;
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
    draw_mask_drag(ui, app, thumbs, list, row_width, rows, layout);
}

/// マスクのサムネイルの当たり（押すとマスクが対象）。つかんだらマスクのドラッグを始める（動かす・落とすのは `follow_mask_drag`。
/// 並べ替えの `layer_drag` とは別）。
fn mask_thumb(
    ui: &Ui,
    app: &mut AppState,
    mask_box: Rect,
    enabled: bool,
    id: LayerId,
) -> egui::Response {
    let maskr = ui.interact(
        mask_box,
        ui.make_persistent_id(("layer.mask", id.0)),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    if maskr.drag_started_by(egui::PointerButton::Primary) {
        app.ui.mask_drag = Some(MaskDrag {
            from: id,
            target: None,
        });
    }
    maskr
}

/// マスクのサムネイルのドラッグの落とす先: ポインタの下の行のレイヤー（一覧の外・マスクを持っている行自身は無し）。
fn update_mask_drag(
    ui: &Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &effect_rows::Layout,
    from: LayerId,
) {
    let target = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| list.contains(*p))
        .and_then(|p| layout.row_at(p.y - list.top() + app.ui.layer_scroll))
        .and_then(|i| rows.get(i))
        .map(|r| r.id)
        .filter(|id| *id != from);
    app.ui.mask_drag = Some(MaskDrag { from, target });
}

/// マスクのドラッグを終える: 落とす先があれば、離したときに Alt を押していればコピー、そうでなければ移動（どちらも 1 回の Undo。
/// ロックなどの断りは操作の側が理由を言う）。
fn drop_mask_drag(ui: &Ui, app: &mut AppState) {
    if let Some(MaskDrag {
        from,
        target: Some(to),
    }) = app.ui.mask_drag.take()
    {
        let edit = if click_modifiers(ui).alt {
            Edit::CopyMaskTo { from, to }
        } else {
            Edit::MoveMask { from, to }
        };
        app.apply(Action::M2(edit));
    }
    app.ui.mask_drag = None;
}

/// マスクのドラッグ中にホイールで一覧を送って、つかんだ行が見えなくなったときの面倒を見る（`follow_drag` と同じ）。
fn follow_mask_drag(
    ui: &Ui,
    app: &mut AppState,
    list: Rect,
    rows: &[Row],
    layout: &effect_rows::Layout,
) {
    let Some(MaskDrag { from, .. }) = app.ui.mask_drag else {
        return;
    };
    if ui.input(|i| i.pointer.primary_down()) {
        update_mask_drag(ui, app, list, rows, layout, from);
    } else {
        drop_mask_drag(ui, app);
    }
}

/// マスクのドラッグの印: 落とせる行を行全体の薄い塗りと細い枠で光らせ（並べ替えの線・グループの太い枠とは別）、つかんだマスクのサムネイルを
/// ポインタに添える（Alt を押していればコピーの＋つき）。ロックされた行は光らせない（落としても理由を言って断る）。
fn draw_mask_drag(
    ui: &Ui,
    app: &AppState,
    thumbs: &mut Thumbnails,
    list: Rect,
    row_width: f32,
    rows: &[Row],
    layout: &effect_rows::Layout,
) {
    let Some(MaskDrag { from, target }) = app.ui.mask_drag else {
        return;
    };
    let ctx = ui.ctx();
    let copy = ui.input(|i| i.modifiers.alt);
    ctx.set_cursor_icon(if copy {
        egui::CursorIcon::Copy
    } else {
        egui::CursorIcon::Grabbing
    });
    if let Some(i) = target.and_then(|to| rows.iter().position(|r| r.id == to)) {
        let locked = rows
            .get(i)
            .and_then(|r| app.session.doc.effective_locks(r.id).ok())
            .is_some_and(|l| l.contains(yolu_core::LayerLocks::ALL));
        if !locked {
            let y = list.top() + layout.layer_y(i) - app.ui.layer_scroll;
            let glow = Rect::from_min_size(
                pos2(list.left() + 1.0, y + 1.0),
                vec2(row_width - 2.0, ROW_HEIGHT - 2.0),
            );
            let painter = ui.painter_at(list);
            w::fill(&painter, glow, t::ACCENT.gamma_multiply(0.25));
            w::outline(&painter, glow, t::ACCENT, 1.0, 2.0);
        }
    }
    let Some(at) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let size = ROW_HEIGHT - 8.0;
    let ghost = Rect::from_min_size(at + vec2(10.0, 10.0), vec2(size, size));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("layers.mask_drag"),
    ));
    let max_px = (size * ctx.pixels_per_point()).round().max(8.0) as u32;
    if let Some(handle) = thumbs.get(ctx, &app.session.doc, from, max_px, ThumbSource::Mask) {
        painter.image(
            handle.id(),
            ghost,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::from_white_alpha(210),
        );
    }
    w::outline(&painter, ghost, t::ACCENT, 1.0, 0.0);
    if copy {
        let badge = Rect::from_center_size(ghost.right_bottom(), vec2(12.0, 12.0));
        painter.circle_filled(badge.center(), 7.0, t::ACCENT);
        w::icon(&painter, badge, "add", Color32::WHITE, 10.0);
    }
}

/// 行を押して離したときの修飾キー（離したイベントの修飾。無ければ今の修飾）。
fn click_modifiers(ui: &Ui) -> egui::Modifiers {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(i.modifiers)
    })
}

/// 行を押したとき（修飾キー無しならそのレイヤーだけを選ぶ。別のレイヤーなら画素が対象、同じレイヤーなら今の対象のまま。Ctrl・Cmd で足し引き、
/// Shift で範囲、Ctrl + Shift で範囲を足す）。選んでいた効果の欄はレイヤーの欄へ戻る。
fn click_row(ui: &Ui, app: &mut AppState, id: LayerId, rows: &[Row]) {
    app.fx.selected = None;
    let modifiers = click_modifiers(ui);
    if modifiers.command || modifiers.shift {
        let ids: Vec<LayerId> = rows.iter().map(|r| r.id).collect();
        if modifiers.shift {
            app.select_layer_range(id, modifiers.command, &ids);
        } else {
            app.toggle_layer_selected(id);
        }
    } else {
        app.select_single_layer(id);
    }
    if app.ui.renaming != Some(id) {
        app.ui.renaming = None;
    }
}

/// レイヤーかマスクのサムネイルを押したとき: 修飾キー無しなら、そのレイヤーを選んで（今選んでいるレイヤーならそのまま）`mask` の側を対象にする
/// （切り替えではなく、もう一度押しても同じ側）。修飾キーつきは行を押したのと同じ（選び方だけを変える）。
fn click_thumb(ui: &Ui, app: &mut AppState, id: LayerId, rows: &[Row], mask: bool) {
    let modifiers = click_modifiers(ui);
    if modifiers.command || modifiers.shift {
        click_row(ui, app, id, rows);
        return;
    }
    if app.session.selected_layer != Some(id) {
        app.select_single_layer(id);
    }
    app.fx.selected = None;
    if app.ui.renaming != Some(id) {
        app.ui.renaming = None;
    }
    if app.m2.edit_mask != mask {
        app.apply(Action::M2Ui(UiOp::EditMask(mask)));
    }
}

/// レイヤーの右クリックのメニュー（行とレイヤーのサムネイルで同じ）。押したレイヤーを選んでから開く。
fn open_layer_menu(
    app: &mut AppState,
    ctx: &egui::Context,
    id: LayerId,
    in_selection: bool,
    at: Option<Pos2>,
) {
    select_for_menu(app, id, in_selection);
    if let Some(at) = at {
        open_popup(
            app,
            ctx,
            PopupKind::LayerContext(id),
            context_anchor(at),
            0.0,
        );
    }
}

/// 右クリックで選ぶ: 選んだレイヤーの中の行なら選択をそのまま（押した行を描く先にする）、外の行ならその 1 つだけ。
fn select_for_menu(app: &mut AppState, id: LayerId, in_selection: bool) {
    if in_selection && app.has_multiple_layers_selected() {
        let chosen = app.selected_layers();
        app.select_layers(chosen, id);
    } else {
        app.select_single_layer(id);
    }
}

/// クリッピングしている行の縦の帯。`column` は行の開閉の欄（グループでない行ではサムネイルの左と同じ位置）で、帯はその左に置く。
/// グループの行はサムネイルが開閉の矢印の右にあるので、サムネイルを基準にすると帯が矢印に重なる。
pub fn clipping_mark_rect(row: Rect, column: Rect) -> Rect {
    Rect::from_min_size(
        pos2(column.left() - 3.0 - CLIPPING_MARK_WIDTH, row.top() + 3.0),
        vec2(CLIPPING_MARK_WIDTH, row.height() - 7.0),
    )
}

/// 行の右端のロックの印。自分のロックなら押すと外す（選んだレイヤーの中の行なら選んだ全部の自分のロックを外す）。
fn lock_mark(
    ui: &mut Ui,
    app: &mut AppState,
    row: Rect,
    id: LayerId,
    effective: yolu_core::LayerLocks,
    chosen: &[LayerId],
) {
    use yolu_core::LayerLocks;
    let lang = app.lang;
    let own = app
        .session
        .doc
        .layer(id)
        .map(|l| l.locks())
        .unwrap_or(LayerLocks::NONE);
    let full = effective.contains(LayerLocks::ALL);
    let mark = Rect::from_min_size(
        pos2(row.right() - 24.0, row.top()),
        vec2(20.0, row.height()),
    );
    let names = lock_names(lang, effective).join(lang.t("panels-text"));
    let mut tip = if full {
        lang.t("panels-locked-all-tip").to_owned()
    } else {
        format!("{}: {names}", lang.t("panels-locked-tip"))
    };
    let enabled = app.can_edit();
    let own_locked = own != LayerLocks::NONE;
    if !own_locked {
        tip.push_str(lang.t("panels-from-its-group"));
    }
    let response = ui.interact(
        mark,
        ui.make_persistent_id(("layer.lock", id.0)),
        if own_locked && enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let painter = ui.painter_at(row);
    w::icon(
        &painter,
        mark,
        if full { "lock_filled" } else { "lock" },
        if own_locked {
            t::TEXT_DIM
        } else {
            t::TEXT_DISABLED
        },
        13.0,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled && own_locked, &tip));
    let clicked = response.clicked();
    let _ = response.on_hover_text(&tip);
    if clicked {
        let ids = if chosen.len() > 1 && chosen.contains(&id) {
            chosen.to_vec()
        } else {
            vec![id]
        };
        app.apply(Action::M2(Edit::Lock {
            ids,
            flag: LayerLocks::from_bits(15).expect("全部のロック"),
            on: false,
        }));
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
    layout: &effect_rows::Layout,
    chosen: &[LayerId],
) {
    let id = info.id;
    let Some(layer) = app.session.doc.layer(id) else {
        return;
    };
    let (name, visible, kind) = (layer.name().to_owned(), layer.visible(), layer.kind());
    let has_mask = layer.mask().is_some();
    let ruler_look = crate::rulers::layer_icon::look(layer);
    let has_path = layer.path().is_some();
    let has_text = layer.text().is_some();
    let channel = app.m2.paint_channel;
    let no_pixels = kind == LayerKind::Raster && !layer.is_channel_enabled(channel);
    let clipped = app
        .session
        .doc
        .layer_index(id)
        .is_some_and(|i| app.session.doc.is_effectively_clipped(i));
    let selected = app.session.selected_layer == Some(id);
    let in_selection = selected || (chosen.len() > 1 && chosen.contains(&id));
    let editing_mask = crate::fx::mask_target(app) == Some(id);
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
    if in_selection {
        w::fill(&painter, row, t::ACCENT_SOFT);
        if selected {
            w::fill(
                &painter,
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                t::ACCENT,
            );
        }
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

    // 目・字下げ・開閉（グループ）・サムネイル・マスク・名前（下地へのクリッピングは字下げせず、サムネイルの左の帯で示す）
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let mut x = eye.right() + 4.0 + INDENT * info.depth as f32;
    let fold = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(INDENT, row.height() - 6.0));
    if info.is_group {
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
    // 定規のアイコン（サムネイルとマスクのすぐ右。定規を持つレイヤーだけ場所を取る）
    let ruler_rect = ruler_look.map(|_| {
        let at = crate::rulers::layer_icon::rect(row, x);
        x += crate::rulers::layer_icon::ADVANCE;
        at
    });
    let effective_locks = app.session.doc.effective_locks(id).unwrap_or_default();
    let locked = effective_locks != yolu_core::LayerLocks::NONE;
    // Live Link で入れた「元の絵」が、GPU を通した・圧縮から読んだ・直した・拡大縮小した絵のとき、理由を出す印
    let marks = &app.services.link_originals;
    let original_note = marks
        .get(app.session.doc.id(), id)
        .filter(|m| m.is_noted())
        .map(|m| m.tooltip(lang, (app.session.doc.width(), app.session.doc.height())));
    let name_rect = Rect::from_min_max(
        pos2(x + 2.0, row.top() + 4.0),
        pos2(
            row.right()
                - name_right_inset(
                    usize::from(locked)
                        + usize::from(has_path)
                        + usize::from(no_pixels)
                        + usize::from(original_note.is_some()),
                ),
            row.bottom() - 4.0,
        ),
    );

    // 選ぶ（Ctrl・Cmd で足し引き、Shift で範囲、Ctrl + Shift で範囲を足す。選んだ行を押してそのままドラッグすれば選んだ全部を運ぶ）・
    // ダブルクリックで名前・右クリックのメニュー・ドラッグで並べ替え
    if response.clicked() {
        click_row(ui, app, id, rows);
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        if !in_selection {
            app.select_single_layer(id);
        }
        app.fx.selected = None; // レイヤーを選ぶと、選んでいた効果の欄はレイヤーの欄へ戻る
        if app.ui.renaming != Some(id) {
            app.ui.renaming = None;
        }
    }
    if (response.double_clicked() || response.triple_clicked())
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.ui.renaming = Some(id);
        app.ui.rename_started = false;
    }
    if response.secondary_clicked() {
        open_layer_menu(app, ctx, id, in_selection, response.interact_pointer_pos());
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        update_drag(ui, app, list, rows, layout, id);
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
            lang.t("panels-hide-tip")
        } else {
            lang.t("panels-show-tip")
        },
        false,
        enabled,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleVisible(id));
    }
    // グループの開閉
    let collapsed = app.m2.collapsed.contains(&id);
    if info.is_group
        && w::icon_button(
            ui,
            fold,
            ("layer.fold", id.0),
            if collapsed {
                "chevron_right"
            } else {
                "expand_more"
            },
            if collapsed {
                lang.t("panels-expand-group-tip")
            } else {
                lang.t("panels-collapse-group-tip")
            },
            false,
            true,
            14.0,
        )
        .clicked()
    {
        app.apply(Action::M2Ui(UiOp::ToggleCollapsed(id)));
    }
    if clipped {
        // 選んだ行の囲い（サムネイルから 2 px 外）の外から 1 px 離す。グループの行も同じ深さの行と帯の x が揃う
        w::rounded(
            &painter,
            clipping_mark_rect(row, fold),
            t::CLIPPING_MARK,
            1.0,
        );
    }
    kind_thumb(ui, app, thumbs, ctx, list, thumb, id, !collapsed);
    // レイヤーのサムネイル（押すとレイヤーの画素が対象。選んだレイヤーで画素が対象なら、マスクの有無によらずマスクと同じ青い枠）
    let pixels_target = selected && !editing_mask;
    if pixels_target {
        w::outline(&painter, thumb.expand(2.0), t::ACCENT, 2.0, 2.0);
    }
    let thumbr = ui.interact(
        thumb,
        ui.make_persistent_id(("layer.thumb", id.0)),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let thumb_label = lang.t("panels-layer-pixels");
    thumbr.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, enabled, pixels_target, thumb_label)
    });
    if thumbr.clicked() {
        click_thumb(ui, app, id, rows, false);
    }
    // レイヤーのサムネイルは行の上で click を取るので、右クリックも行と同じメニューにつなぐ
    if thumbr.secondary_clicked() {
        open_layer_menu(app, ctx, id, in_selection, thumbr.interact_pointer_pos());
    }
    if has_mask {
        let _ = thumbr.on_hover_text(lang.t("panels-layer-pixels-click-to-target-tip"));
    }
    // マスク（押すとマスクが対象。右クリックでマスクのメニュー）
    if has_mask {
        let maskr = mask_thumb(ui, app, mask_box, enabled, id);
        let painter = ui.painter_at(list);
        w::checker(&painter, mask_box, 3.0);
        let max_px = (mask_box.width() * ctx.pixels_per_point()).round().max(8.0) as u32;
        if let Some(handle) = thumbs.get(ctx, &app.session.doc, id, max_px, ThumbSource::Mask) {
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
        // マスクが対象でないあいだはマスクの効果の行が一覧に出ないので、効果の付いたマスクのサムネイルの角に小さな効果の印
        // （数は画面に出さず、ツールチップに）
        let hidden_effects = if editing_mask {
            0
        } else {
            app.session
                .doc
                .filters_of(id, FilterTarget::Mask)
                .map_or(0, <[_]>::len)
        };
        if hidden_effects > 0 {
            let at = mask_box.right_bottom() - vec2(2.0, 2.0);
            painter.circle_filled(at, 6.0, t::PANEL_HEADER);
            painter.circle_stroke(at, 6.0, egui::Stroke::new(1.0, t::BORDER));
            w::icon(
                &painter,
                Rect::from_center_size(at, vec2(10.0, 10.0)),
                "auto_awesome",
                t::TEXT,
                9.0,
            );
            // 押す操作とツールチップはマスクのサムネイルが受ける（印は読み上げの名前を持つだけ。ツールチップが 2 つ重ならない）
            let count = mask_effects_tip(lang, hidden_effects);
            let mark = ui.interact(
                Rect::from_center_size(at, vec2(12.0, 12.0)),
                ui.make_persistent_id(("layer.mask.fx", id.0)),
                Sense::hover(),
            );
            mark.widget_info(|| WidgetInfo::labeled(WidgetType::Label, enabled, &count));
        }
        let tip = lang.t("panels-layer-mask-click-to-target-tip");
        let hover = match hidden_effects {
            0 => tip.to_owned(),
            n => format!("{tip}\n{}", mask_effects_tip(lang, n)),
        };
        maskr.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, editing_mask, tip));
        if maskr.clicked() {
            click_thumb(ui, app, id, rows, true);
        }
        if maskr.secondary_clicked() {
            select_for_menu(app, id, in_selection);
            if let Some(at) = maskr.interact_pointer_pos() {
                open_popup(
                    app,
                    ctx,
                    PopupKind::M2(Popup::MaskContext(id)),
                    context_anchor(at),
                    0.0,
                );
            }
        }
        let _ = maskr.on_hover_text(hover);
    }

    // 定規のアイコン（押すと行を選んでプロパティの「定規」を開く。Shift＋押すと表示を全部入り切り。右クリックでメニュー）
    if let (Some(any_visible), Some(at)) = (ruler_look, ruler_rect) {
        use crate::rulers::layer_icon::{self, Event};
        match layer_icon::show(ui, app, id, at, any_visible, enabled) {
            Event::Select => {
                click_row(ui, app, id, rows);
                app.ui.sections.insert("rulers", true);
            }
            Event::ToggleAll => {
                app.apply(Action::Ruler(crate::rulers::RulerAction::SetAllVisible {
                    owner: id,
                    visible: !any_visible,
                }))
            }
            Event::Menu(at) => {
                select_for_menu(app, id, in_selection);
                open_popup(app, ctx, PopupKind::RulerLayer(id), context_anchor(at), 0.0);
            }
            Event::None => {}
        }
    }

    // 名前（ダブルクリックで変える）
    let painter = ui.painter_at(list);
    if app.ui.renaming == Some(id) {
        let first = !app.ui.rename_started;
        app.ui.rename_started = true;
        let out = w::text_field(ui, name_rect, ("layer.rename", id.0), &name, None, first);
        if let Some(next) = out.committed {
            let next = next.trim().to_owned();
            if !next.is_empty() {
                app.apply(Action::RenameLayer(id, next));
            }
        }
        if !first && !out.focused {
            app.ui.renaming = None;
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
    // 右端の印: ロック（すべては塗った錠、ほかの自分のロックは線の錠、グループから効いているだけなら薄い線の錠。自分のロックは押すと外す）と、
    // 描くチャンネルを使っていないラスターレイヤー
    if locked {
        lock_mark(ui, app, row, id, effective_locks, chosen);
    }
    if no_pixels {
        let mark = Rect::from_min_size(
            pos2(row.right() - mark_inset(usize::from(locked)), row.top()),
            vec2(MARK_WIDTH, row.height()),
        );
        w::icon(&painter, mark, "link_off", t::TEXT_DISABLED, 13.0);
        ui.interact(
            mark,
            ui.make_persistent_id(("layer.nochannel", id.0)),
            Sense::hover(),
        )
        .on_hover_text(lang.t("panels-this-layer-does-not-use-tip"));
    }
    // 右端の印: パスで描かれたレイヤー（手では描けない。ラスタライズで普通のレイヤーになる）
    if has_path {
        let at = row.right() - mark_inset(usize::from(locked) + usize::from(no_pixels));
        let mark = Rect::from_min_size(pos2(at, row.top()), vec2(MARK_WIDTH, row.height()));
        w::icon(&painter, mark, "conversion_path", t::TEXT_DIM, 13.0);
        ui.interact(
            mark,
            ui.make_persistent_id(("layer.path", id.0)),
            Sense::hover(),
        )
        .on_hover_text(lang.t("panels-drawn-by-a-path-it-tip"));
    }
    // 右端の印: テキストレイヤー（手では描けない。ラスタライズで普通のレイヤーになる）
    if has_text {
        let at = row.right() - mark_inset(usize::from(locked) + usize::from(no_pixels));
        let mark = Rect::from_min_size(pos2(at, row.top()), vec2(MARK_WIDTH, row.height()));
        w::icon(&painter, mark, "tools/text", t::TEXT_DIM, 13.0);
        ui.interact(
            mark,
            ui.make_persistent_id(("layer.text", id.0)),
            Sense::hover(),
        )
        .on_hover_text(lang.t("panels-text-layer-the-text-stays-tip"));
    }
    // 右端の印: Live Link で入れた元の絵の読み方（理由はツールチップ）
    if let Some(note) = original_note {
        let at = row.right()
            - mark_inset(usize::from(locked) + usize::from(no_pixels) + usize::from(has_path));
        let mark = Rect::from_min_size(pos2(at, row.top()), vec2(MARK_WIDTH, row.height()));
        w::icon(&painter, mark, "info", t::TEXT_DIM, 13.0);
        ui.interact(
            mark,
            ui.make_persistent_id(("layer.original", id.0)),
            Sense::hover(),
        )
        .on_hover_text(note);
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &name)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_field_stops_next_to_the_marks_that_are_drawn() {
        for marks in 1..=2 {
            // いちばん左の印の左端のすぐ外（2 点の隙間）まで名前を出す。それ以上は空けない
            let leftmost = mark_inset(marks - 1);
            assert_eq!(name_right_inset(marks), leftmost + 2.0, "{marks} 個");
        }
        // 印が無いときは、1 つぶんの場所のまま（行の右端の余白）
        assert_eq!(name_right_inset(0), name_right_inset(1));
    }

    /// 行の欄の並び（`layer_row` と同じ式）: (開閉の欄, サムネイル)。
    fn columns(row: Rect, depth: usize, group: bool) -> (Rect, Rect) {
        let x = row.left() + 4.0 + 24.0 + 4.0 + INDENT * depth as f32;
        let fold = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(INDENT, row.height() - 6.0));
        let thumb_x = if group { x + INDENT } else { x };
        let thumb = Rect::from_min_size(
            pos2(thumb_x, row.top() + 4.0),
            vec2(row.height() - 8.0, row.height() - 8.0),
        );
        (fold, thumb)
    }

    #[test]
    fn the_clipping_mark_sits_left_of_the_thumbnail_and_clear_of_the_selection_outline() {
        let row = Rect::from_min_size(pos2(0.0, 100.0), vec2(300.0, ROW_HEIGHT));
        let (fold, thumb) = columns(row, 0, false);
        let mark = clipping_mark_rect(row, fold);
        // 選んだ行の囲い（サムネイルを 2 点広げ、内側に 2 点の線）の外から 1 点離れる
        let outline = thumb.expand(2.0);
        assert!(mark.right() + 1.0 <= outline.left(), "{mark:?} {outline:?}");
        assert!(!mark.intersects(outline));
        assert_eq!(mark.width(), CLIPPING_MARK_WIDTH);
        assert!(
            mark.top() >= row.top() && mark.bottom() <= row.bottom(),
            "{mark:?}"
        );
        // 字下げの深さに合わせて動く（サムネイルのすぐ左。同じ距離）
        let (fold2, _) = columns(row, 1, false);
        let moved = clipping_mark_rect(row, fold2);
        assert_eq!(moved.right() - mark.right(), INDENT);
    }

    #[test]
    fn the_clipping_mark_of_a_group_row_clears_the_fold_arrow_and_lines_up_with_a_layer_row() {
        let row = Rect::from_min_size(pos2(0.0, 100.0), vec2(300.0, ROW_HEIGHT));
        for depth in 0..3 {
            let (layer_fold, layer_thumb) = columns(row, depth, false);
            let (group_fold, group_thumb) = columns(row, depth, true);
            let layer_mark = clipping_mark_rect(row, layer_fold);
            let group_mark = clipping_mark_rect(row, group_fold);
            assert_eq!(
                group_mark, layer_mark,
                "深さ {depth}: 同じ深さの行と帯が揃う"
            );
            assert!(
                !group_mark.intersects(group_fold),
                "{group_mark:?} {group_fold:?}"
            );
            assert!(
                group_mark.right() + 1.0 <= group_thumb.expand(2.0).left()
                    && group_mark.right() + 1.0 <= layer_thumb.expand(2.0).left(),
                "{group_mark:?}"
            );
        }
    }

    #[test]
    fn an_adjustment_row_shows_its_kinds_icon_and_falls_back_to_the_adjustment_layer_icon() {
        assert_eq!(adjustment_icon(None), "texture");
        for kind in AdjustmentKind::ALL {
            assert_eq!(adjustment_icon(Some(&kind.settings())), kind.icon());
        }
        assert_eq!(m2::layer_kind_icon(LayerKind::Adjustment), "texture");
    }

    #[test]
    fn thumbnails_follow_the_channel_and_forget_dead_layers() {
        let mut app = AppState::new(32, 32);
        let id = app.session.selected_layer.unwrap();
        let ctx = egui::Context::default();
        let mut thumbs = Thumbnails::default();
        assert!(thumbs
            .get(
                &ctx,
                &app.session.doc,
                id,
                16,
                ThumbSource::Channel(Channel::Color)
            )
            .is_some());
        let built = thumbs.rebuilt;
        assert!(thumbs
            .get(
                &ctx,
                &app.session.doc,
                id,
                16,
                ThumbSource::Channel(Channel::Color)
            )
            .is_some());
        assert_eq!(thumbs.rebuilt, built, "文書が変わらなければ作り直さない");
        thumbs
            .get(
                &ctx,
                &app.session.doc,
                id,
                16,
                ThumbSource::Channel(Channel::Roughness),
            )
            .unwrap();
        assert_eq!(thumbs.rebuilt, built + 1, "チャンネルごとに作る");
        app.apply(Action::M2(Edit::AddMask(id)));
        thumbs
            .get(&ctx, &app.session.doc, id, 16, ThumbSource::Mask)
            .unwrap();
        app.apply(Action::M2(Edit::RemoveMask(id)));
        thumbs.retain(&app.session.doc);
        assert!(thumbs.map.keys().all(|(_, s)| *s != ThumbSource::Mask));
    }
}
