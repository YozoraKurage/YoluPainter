//! レイヤーの行の下の効果の行（Substance Painter の効果の行。Unity 版の `EffectRows`）: レイヤーの行の下に、対象の側のスタックだけを字下げした
//! 子の行で並べる。選んだレイヤーでマスクが対象ならマスクの Anchor（そこまでの結果）とマスクの効果の段、それ以外（選んでいないレイヤーも）はレイヤーの
//! Anchor と画素の効果の段（どちらも上が後に掛かる）。行は目（有効の切り替え）・アイコン・
//! 名前と主な値で、押すとその段を選び、プロパティの欄にその段の設定が出る。マウスの乗った行と選んだ行に上へ・下へ・消すのボタン、
//! 右クリックで同じ操作。効いていない Generator には印（理由はツールチップ）。
//!
//! 一覧の行の高さがレイヤーと効果で違うので、行の位置・落とす先は `Layout` で数える（レイヤーの並べ替えのドラッグとスマートマテリアルの
//! ドロップも、レイヤーの行の番号に直してから `m2::drop_target_at` へ渡す）。

use egui::{pos2, vec2, Rect, Sense, Ui, WidgetInfo, WidgetType};
use yolu_core::{
    AnchorId, AnchorPlacement, Document, EffectSettings, FilterId, FilterTarget, LayerId,
};

use crate::fx::{names, FxOp, Selected};
use crate::m2::Row;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 効果の行の高さ。
pub const EFFECT_ROW_HEIGHT: f32 = 22.0;
/// レイヤーの字下げ 1 段（`layers::INDENT` と同じ）。
const INDENT: f32 = 14.0;

/// レイヤーの行の下の子の行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    Anchor {
        layer: LayerId,
        placement: AnchorPlacement,
        id: AnchorId,
    },
    Effect {
        layer: LayerId,
        target: FilterTarget,
        id: FilterId,
        /// スタックの中の位置（0 が最初に当たる）と段の数。
        index: usize,
        count: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// レイヤーの行（`rows` の番号）。
    Layer(usize),
    Child(Child),
}

/// 一覧の 1 行（一覧の中の上の端と高さ）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    pub y: f32,
    pub height: f32,
    /// レイヤーの字下げの段（子の行はそのレイヤーと同じ）。
    pub depth: usize,
    /// そのレイヤーの子の行の最後（つなぎの線をここで止める）。
    pub last_child: bool,
}

/// 一覧の行の並び。
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub entries: Vec<Entry>,
    pub height: f32,
    layer_height: f32,
    layer_tops: Vec<f32>,
}

/// レイヤーの行の下に並べる子の行（上から）。`mask_target` はマスクが対象のレイヤー（`crate::fx::mask_target`）で、そのレイヤーの下にはマスクの Anchor と
/// マスクの効果の段だけ、ほかのレイヤーの下にはレイヤーの Anchor と画素の効果の段だけを出す。
fn children_of(doc: &Document, layer: LayerId, mask_target: Option<LayerId>) -> Vec<Child> {
    let Some(l) = doc.layer(layer) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    let mut stack = |target: FilterTarget, anchor: Option<&yolu_core::Anchor>, placement| {
        if let Some(a) = anchor {
            v.push(Child::Anchor {
                layer,
                placement,
                id: a.id(),
            });
        }
        let filters = doc.filters_of(layer, target).unwrap_or(&[]);
        let count = filters.len();
        for (index, e) in filters.iter().enumerate().rev() {
            v.push(Child::Effect {
                layer,
                target,
                id: e.id(),
                index,
                count,
            });
        }
    };
    match l.mask().filter(|_| mask_target == Some(layer)) {
        Some(mask) => stack(FilterTarget::Mask, mask.anchor(), AnchorPlacement::Mask),
        None => stack(FilterTarget::Content, l.anchor(), AnchorPlacement::Layer),
    }
    v
}

/// 一覧の行を数える。`rows` はレイヤーの行（上から。閉じたグループの中身は含まない）、`layer_height` はレイヤーの行の高さ、`mask_target` は
/// マスクが対象のレイヤー（`crate::fx::mask_target`）。対象で子の行の数が変わるので、一覧を描く・ドラッグの落とす先を数える・棚の素材を
/// 落とす先を数えるのは、同じフレームの同じ `Layout` で行う。
pub fn layout(
    doc: &Document,
    rows: &[Row],
    layer_height: f32,
    mask_target: Option<LayerId>,
) -> Layout {
    let mut entries = Vec::new();
    let mut layer_tops = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for (i, row) in rows.iter().enumerate() {
        layer_tops.push(y);
        entries.push(Entry {
            kind: Kind::Layer(i),
            y,
            height: layer_height,
            depth: row.depth,
            last_child: false,
        });
        y += layer_height;
        let children = children_of(doc, row.id, mask_target);
        let n = children.len();
        for (k, child) in children.into_iter().enumerate() {
            entries.push(Entry {
                kind: Kind::Child(child),
                y,
                height: EFFECT_ROW_HEIGHT,
                depth: row.depth,
                last_child: k + 1 == n,
            });
            y += EFFECT_ROW_HEIGHT;
        }
    }
    Layout {
        entries,
        height: y,
        layer_height,
        layer_tops,
    }
}

impl Layout {
    /// レイヤーの行（`rows` の番号）の上の端。
    pub fn layer_y(&self, row: usize) -> f32 {
        self.layer_tops.get(row).copied().unwrap_or(self.height)
    }

    /// `gap` 番目のレイヤーの行のすぐ上の線の高さ（レイヤーの数なら一覧の下の端）。
    pub fn gap_y(&self, gap: usize) -> f32 {
        if gap >= self.layer_tops.len() {
            self.height
        } else {
            self.layer_tops[gap]
        }
    }

    /// 一覧の中の高さ `y` にあるレイヤーの行（レイヤーの行か、そのレイヤーの効果の行。どこにも当たらなければ None）。
    pub fn row_at(&self, y: f32) -> Option<usize> {
        if y < 0.0 || y >= self.height {
            return None;
        }
        Some(
            self.layer_tops
                .partition_point(|top| *top <= y)
                .saturating_sub(1),
        )
    }

    /// 一覧の中の高さ `y` を、レイヤーの行の単位（0 が一番上のレイヤーの行の上の端。1 行 = 1）に直す。効果の行の上は、そのレイヤーの下の隙間（次のレイヤーの上の端）。
    pub fn position_at(&self, y: f32) -> f32 {
        let n = self.layer_tops.len();
        if n == 0 || y < 0.0 {
            return if y < 0.0 { -1.0 } else { 0.0 };
        }
        if y >= self.height {
            return n as f32;
        }
        // y を含むレイヤーの行（その行の top ≤ y < 次の行の top）
        let i = self
            .layer_tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1);
        let within = y - self.layer_tops[i];
        if within < self.layer_height {
            i as f32 + within / self.layer_height
        } else {
            (i + 1) as f32
        }
    }
}

/// Anchor を読んでいる段（レイヤーと、どちらのスタックか）。
pub fn anchor_readers(doc: &Document, anchor: AnchorId) -> Vec<(LayerId, FilterTarget)> {
    let mut v = Vec::new();
    for layer in doc.layers() {
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            for e in doc.filters_of(layer.id(), target).unwrap_or(&[]) {
                if let EffectSettings::Generator(g) = e.settings() {
                    if g.anchor.id == anchor.0 && e.settings().reads_anchor() {
                        v.push((layer.id(), target));
                    }
                }
            }
        }
    }
    v
}

fn open_popup(app: &mut AppState, ctx: &egui::Context, popup: Popup, anchor: Rect) {
    app.popup = Some(OpenPopup {
        kind: PopupKind::M2(popup),
        state: PopupState::new(ctx, anchor),
    });
}

/// 子の行を描く。
pub fn child_row(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    row: Rect,
    entry: &Entry,
    child: Child,
) {
    match child {
        Child::Effect {
            layer,
            target,
            id,
            index,
            count,
        } => effect_row(ui, app, list, row, entry, layer, target, id, index, count),
        Child::Anchor { placement, id, .. } => anchor_row(ui, app, list, row, entry, placement, id),
    }
}

/// つなぎの線（レイヤーの名前の下から、最後の子の行で止める）。
fn guide(painter: &egui::Painter, row: Rect, x: f32, last: bool) {
    w::vline(
        painter,
        x - 6.0,
        row.top(),
        row.bottom() - if last { row.height() / 2.0 } else { 0.0 },
        t::SEPARATOR,
    );
    w::hline(
        painter,
        x - 6.0,
        x - 1.0,
        row.center().y.round(),
        t::SEPARATOR,
    );
}

#[allow(clippy::too_many_arguments)]
fn effect_row(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    row: Rect,
    entry: &Entry,
    layer: LayerId,
    target: FilterTarget,
    id: FilterId,
    index: usize,
    count: usize,
) {
    let lang = app.lang;
    let Some((_, effect, _)) = app.doc.find_filter(id) else {
        return;
    };
    let (enabled_stage, active) = (effect.enabled(), effect.is_active());
    let is_generator = effect.settings().is_generator();
    let icon = names::effect_icon(effect.settings());
    let label = names::effect_label(lang, effect, target, app.m2.paint_channel, |c| {
        crate::m2::channel_name(lang, &app.doc, c)
    });
    let reason = if is_generator && active {
        app.doc
            .generator_inactive(layer, id)
            .ok()
            .flatten()
            .map(|r| lang.inactive_reason(&r))
            .or_else(|| {
                let reason = app.doc.generator_fallback(layer, id).ok().flatten()?;
                let EffectSettings::Generator(g) = effect.settings() else {
                    return None;
                };
                Some(lang.fallback_effect(&yolu_core::FallbackEffect {
                    layer,
                    layer_name: app.doc.layer(layer)?.name().to_owned(),
                    mask: target == FilterTarget::Mask,
                    kind: g.kind,
                    reason: yolu_core::InactiveReason::Generator(reason),
                }))
            })
    } else {
        None
    };
    let selected = app.fx.selected == Some(Selected::Filter { layer, id });
    let can_edit = app.can_edit();
    let hit = row.intersect(list);
    let response = ui.interact(hit, ui.make_persistent_id(("fx.row", id.0)), Sense::click());
    let hover = response.hovered();
    let painter = ui.painter_at(list);
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if hover {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left() + 28.0,
        row.right(),
        row.bottom() - 1.0,
        t::SEPARATOR,
    );
    // 目（有効の切り替え。レイヤーの目と同じ列）
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 2.0),
        vec2(24.0, row.height() - 4.0),
    );
    let eye_tip = if enabled_stage {
        lang.pick("フィルターを無効にする", "Turn the filter off")
    } else {
        lang.pick("フィルターを有効にする", "Turn the filter on")
    };
    if w::icon_button(
        ui,
        eye,
        ("fx.eye", id.0),
        if enabled_stage {
            "visibility"
        } else {
            "visibility_off"
        },
        eye_tip,
        false,
        can_edit,
        14.0,
    )
    .clicked()
    {
        app.apply(Action::Fx(FxOp::SetEnabled {
            layer,
            id,
            enabled: !enabled_stage,
        }));
    }
    // レイヤーの名前の位置から一段下げ、つなぎの線とアイコン
    let painter = ui.painter_at(list);
    let mut x = eye.right() + 4.0 + INDENT * entry.depth as f32 + 10.0;
    guide(&painter, row, x, entry.last_child);
    if target == FilterTarget::Mask {
        w::icon(
            &painter,
            Rect::from_min_size(pos2(x, row.top()), vec2(14.0, row.height())),
            "vignette",
            t::TEXT_DIM,
            12.0,
        );
        x += 15.0;
    }
    let color = if !enabled_stage {
        t::TEXT_DISABLED
    } else if selected {
        egui::Color32::WHITE
    } else {
        t::TEXT
    };
    w::icon(
        &painter,
        Rect::from_min_size(pos2(x, row.top()), vec2(16.0, row.height())),
        icon,
        color,
        13.0,
    );
    x += 19.0;
    // 上へ・下へ・消す（マウスの乗った行と選んだ行）
    let buttons = selected || hover;
    let right = row.right() - 4.0 - if buttons { 60.0 } else { 0.0 };
    // 効いていない印
    let mark = reason
        .is_some()
        .then(|| Rect::from_min_size(pos2(right - 18.0, row.top()), vec2(16.0, row.height())));
    let text_right = mark.map_or(right, |m| m.left() - 2.0);
    let text_rect = Rect::from_min_max(pos2(x, row.top()), pos2(text_right.max(x), row.bottom()));
    let shown = w::fit(&painter, &label, text_rect.width(), t::LABEL);
    w::text(
        &painter,
        text_rect,
        &shown,
        t::LABEL.with_color(if !enabled_stage { t::TEXT_DIM } else { color }),
        w::Align::Left,
    );
    // 印・省略した名前の上では、その場所のツールチップだけを出す（行のツールチップは重なるので出さない）
    let mut over_part = false;
    if let Some(mark) = mark {
        w::icon(&painter, mark, "warning", t::WARNING, 13.0);
        let tip = reason.clone().unwrap_or_default();
        let on_mark = ui
            .interact(
                mark,
                ui.make_persistent_id(("fx.mark", id.0)),
                Sense::hover(),
            )
            .on_hover_text(tip);
        over_part |= on_mark.hovered();
    }
    if shown != label {
        let tip_rect = text_rect.intersect(list);
        let name_tip = match &reason {
            Some(why) => format!("{label}\n{why}"),
            None => label.clone(),
        };
        let on_name = ui
            .interact(
                tip_rect,
                ui.make_persistent_id(("fx.name", id.0)),
                Sense::hover(),
            )
            .on_hover_text(name_tip);
        over_part |= on_name.hovered();
    }
    if buttons {
        let at = |dx: f32| {
            Rect::from_min_size(
                pos2(right + dx, row.top() + 1.0),
                vec2(20.0, row.height() - 2.0),
            )
        };
        if w::icon_button(
            ui,
            at(0.0),
            ("fx.up", id.0),
            "expand_less",
            lang.pick("上へ（後から掛かる）", "Move up (applied later)"),
            false,
            can_edit && index + 1 < count,
            14.0,
        )
        .clicked()
        {
            app.apply(Action::Fx(FxOp::Move {
                layer,
                id,
                index: index + 1,
            }));
        }
        if w::icon_button(
            ui,
            at(20.0),
            ("fx.down", id.0),
            "expand_more",
            lang.pick("下へ（先に掛かる）", "Move down (applied earlier)"),
            false,
            can_edit && index > 0,
            14.0,
        )
        .clicked()
        {
            app.apply(Action::Fx(FxOp::Move {
                layer,
                id,
                index: index - 1,
            }));
        }
        if w::icon_button(
            ui,
            at(40.0),
            ("fx.remove", id.0),
            "close",
            lang.pick("フィルターを削除", "Remove the filter"),
            false,
            can_edit,
            13.0,
        )
        .clicked()
        {
            app.apply(Action::Fx(FxOp::Remove { layer, id }));
        }
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &label));
    // 効いていない理由は、行のどこに乗せても出す（印だけでは小さい）。印・省略した名前の上ではそれぞれの場所のものだけ
    let response = match &reason {
        Some(why) if !over_part => response.on_hover_text(format!("{label}\n{why}")),
        _ => response,
    };
    if response.clicked() {
        app.apply(Action::Fx(FxOp::SelectFilter { layer, id }));
    }
    if response.secondary_clicked() {
        app.apply(Action::Fx(FxOp::SelectFilter { layer, id }));
        if let Some(at) = response.interact_pointer_pos() {
            let ctx = ui.ctx().clone();
            open_popup(app, &ctx, Popup::EffectContext, context_anchor(at));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn anchor_row(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    row: Rect,
    entry: &Entry,
    placement: AnchorPlacement,
    id: AnchorId,
) {
    let lang = app.lang;
    let Some(info) = app.doc.find_anchor(id) else {
        return;
    };
    let name = names::anchor_label(lang, info.anchor, placement);
    let readers = anchor_readers(&app.doc, id).len();
    let selected = app.fx.selected == Some(Selected::Anchor { id });
    let can_edit = app.can_edit();
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("fx.anchor", id.0)),
        Sense::click(),
    );
    let hover = response.hovered();
    let painter = ui.painter_at(list);
    if selected {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if hover {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left() + 28.0,
        row.right(),
        row.bottom() - 1.0,
        t::SEPARATOR,
    );
    // Anchor には有効・無効が無いので目は出さない（目の列の右から）
    let mut x = row.left() + 4.0 + 24.0 + 4.0 + INDENT * entry.depth as f32 + 10.0;
    guide(&painter, row, x, entry.last_child);
    if placement == AnchorPlacement::Mask {
        w::icon(
            &painter,
            Rect::from_min_size(pos2(x, row.top()), vec2(14.0, row.height())),
            "vignette",
            t::TEXT_DIM,
            12.0,
        );
        x += 15.0;
    }
    w::icon(
        &painter,
        Rect::from_min_size(pos2(x, row.top()), vec2(16.0, row.height())),
        "anchor",
        t::ACCENT,
        13.0,
    );
    x += 19.0;
    let buttons = selected || hover;
    let right = row.right() - 4.0 - if buttons { 20.0 } else { 0.0 };
    let count = lang.pick(format!("{readers} 段が読む"), format!("read by {readers}"));
    let count_w =
        (w::text_width(&painter, &count, t::LABEL_DIM) + 6.0).min((right - x - 40.0).max(0.0));
    let name_rect = Rect::from_min_max(
        pos2(x, row.top()),
        pos2((right - count_w).max(x), row.bottom()),
    );
    let shown = w::fit(&painter, &name, name_rect.width() - 2.0, t::LABEL);
    w::text(
        &painter,
        name_rect,
        &shown,
        t::LABEL.with_color(if selected {
            egui::Color32::WHITE
        } else {
            t::TEXT
        }),
        w::Align::Left,
    );
    if count_w > 20.0 {
        let count_rect = Rect::from_min_max(
            pos2(name_rect.right(), row.top()),
            pos2(right, row.bottom()),
        );
        let shown = w::fit(&painter, &count, count_w, t::LABEL_DIM);
        w::text(&painter, count_rect, &shown, t::LABEL_DIM, w::Align::Left);
    }
    if shown != name {
        ui.interact(
            name_rect.intersect(list),
            ui.make_persistent_id(("fx.anchor.name", id.0)),
            Sense::hover(),
        )
        .on_hover_text(name.clone());
    }
    if buttons
        && w::icon_button(
            ui,
            Rect::from_min_size(pos2(right, row.top() + 1.0), vec2(20.0, row.height() - 2.0)),
            ("fx.anchor.remove", id.0),
            "close",
            lang.pick("アンカーを外す", "Remove the anchor"),
            false,
            can_edit,
            13.0,
        )
        .clicked()
    {
        app.apply(Action::Fx(FxOp::RemoveAnchor(id)));
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &name));
    if response.clicked() {
        app.apply(Action::Fx(FxOp::SelectAnchor(id)));
    }
    if response.secondary_clicked() {
        app.apply(Action::Fx(FxOp::SelectAnchor(id)));
        if let Some(at) = response.interact_pointer_pos() {
            let ctx = ui.ctx().clone();
            open_popup(app, &ctx, Popup::EffectContext, context_anchor(at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::FilterKind;

    fn rows(app: &AppState) -> Vec<Row> {
        crate::m2::visible_rows(&app.doc, &app.m2.collapsed)
    }

    /// レイヤーの効果（アンカー・ぼかし・反転）とマスクの効果（アンカー・ぼかし）を両方持つレイヤー。
    fn both_stacks(app: &mut AppState) -> (LayerId, AnchorId, AnchorId) {
        let layer = app.selected_layer.unwrap();
        app.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
        let layer_anchor = app
            .doc
            .add_anchor(layer, AnchorPlacement::Layer, Some("A"), None)
            .unwrap();
        let mask_anchor = app
            .doc
            .add_anchor(layer, AnchorPlacement::Mask, Some("M"), None)
            .unwrap();
        for kind in [FilterKind::Blur, FilterKind::Invert] {
            app.apply(Action::Fx(FxOp::AddFilter {
                target: FilterTarget::Content,
                kind,
            }));
        }
        app.apply(Action::M2Ui(crate::m2::UiOp::EditMask(true)));
        app.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::Blur,
        }));
        (layer, layer_anchor, mask_anchor)
    }

    #[test]
    fn children_follow_the_stack_top_first_and_only_the_target_side_shows() {
        let mut app = AppState::new(32, 32);
        let (layer, layer_anchor, mask_anchor) = both_stacks(&mut app);
        let rows = rows(&app);
        // レイヤーの画素が対象（マスクは対象でない）: レイヤーのアンカーと画素の段だけ。上（後に掛かる）から反転（index 1）、ぼかし（index 0）
        let l = layout(&app.doc, &rows, 30.0, None);
        let kinds: Vec<Kind> = l.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds.len(), 1 + 1 + 2);
        assert!(matches!(
            kinds[1],
            Kind::Child(Child::Anchor { id, placement: AnchorPlacement::Layer, .. }) if id == layer_anchor
        ));
        assert!(matches!(
            kinds[2],
            Kind::Child(Child::Effect {
                index: 1,
                count: 2,
                target: FilterTarget::Content,
                ..
            })
        ));
        assert!(matches!(
            kinds[3],
            Kind::Child(Child::Effect {
                index: 0,
                count: 2,
                target: FilterTarget::Content,
                ..
            })
        ));
        assert!(l.entries[3].last_child && !l.entries[2].last_child);
        assert_eq!(l.height, 30.0 + 3.0 * EFFECT_ROW_HEIGHT);
        // マスクが対象: マスクのアンカーとマスクの段だけ
        let l = layout(&app.doc, &rows, 30.0, Some(layer));
        let kinds: Vec<Kind> = l.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds.len(), 1 + 1 + 1);
        assert!(matches!(
            kinds[1],
            Kind::Child(Child::Anchor { id, placement: AnchorPlacement::Mask, .. }) if id == mask_anchor
        ));
        assert!(matches!(
            kinds[2],
            Kind::Child(Child::Effect {
                target: FilterTarget::Mask,
                index: 0,
                count: 1,
                ..
            })
        ));
        assert!(l.entries[2].last_child && !l.entries[1].last_child);
        assert_eq!(l.height, 30.0 + 2.0 * EFFECT_ROW_HEIGHT);
    }

    #[test]
    fn the_mask_target_changes_only_that_layers_children() {
        let mut app = AppState::new(32, 32);
        let (layer, _, _) = both_stacks(&mut app);
        // 上のレイヤー（選んでいない。マスクの効果も持つ）の下には、マスクが対象のレイヤーがあってもレイヤーの効果を出す
        app.apply(Action::NewLayer);
        let upper = app.selected_layer.unwrap();
        assert_ne!(upper, layer);
        app.apply(Action::M2(crate::m2::Edit::AddMask(upper)));
        app.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::Invert,
        }));
        app.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        }));
        let rows = rows(&app);
        let upper_row = rows.iter().position(|r| r.id == upper).unwrap();
        let layer_row = rows.iter().position(|r| r.id == layer).unwrap();
        let children_of_row = |l: &Layout, row: usize| -> Vec<Child> {
            l.entries
                .iter()
                .filter_map(|e| match e.kind {
                    Kind::Child(c) if l.row_at(e.y) == Some(row) => Some(c),
                    _ => None,
                })
                .collect()
        };
        for mask in [Some(layer), Some(upper), None] {
            let l = layout(&app.doc, &rows, 30.0, mask);
            for (row, id) in [(upper_row, upper), (layer_row, layer)] {
                let targets: Vec<FilterTarget> = children_of_row(&l, row)
                    .into_iter()
                    .filter_map(|c| match c {
                        Child::Effect { target, .. } => Some(target),
                        Child::Anchor { .. } => None,
                    })
                    .collect();
                let want = if mask == Some(id) {
                    FilterTarget::Mask
                } else {
                    FilterTarget::Content
                };
                assert!(!targets.is_empty());
                assert!(
                    targets.iter().all(|t| *t == want),
                    "{mask:?} {id:?}: {targets:?}"
                );
            }
        }
    }

    #[test]
    fn a_mask_target_without_a_mask_shows_the_layer_effects() {
        let mut app = AppState::new(32, 32);
        app.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        }));
        let layer = app.selected_layer.unwrap();
        let rows = rows(&app);
        let l = layout(&app.doc, &rows, 30.0, Some(layer));
        assert_eq!(
            l.entries.len(),
            2,
            "マスクの無いレイヤーは、対象を渡されてもレイヤーの効果"
        );
        assert!(matches!(
            l.entries[1].kind,
            Kind::Child(Child::Effect {
                target: FilterTarget::Content,
                ..
            })
        ));
    }

    #[test]
    fn positions_map_back_to_layer_rows() {
        let mut app = AppState::new(32, 32);
        app.apply(Action::NewLayer);
        let bottom = app.doc.layers()[0].id();
        app.apply(Action::Fx(FxOp::AddAnchor {
            layer: bottom,
            placement: AnchorPlacement::Layer,
        }));
        let rows = rows(&app);
        assert_eq!(rows.len(), 2);
        // 上の行: 新しいレイヤー（効果なし）、下の行: 下のレイヤー（アンカーの行が続く）
        let l = layout(&app.doc, &rows, 30.0, None);
        assert_eq!(l.layer_y(0), 0.0);
        assert_eq!(l.layer_y(1), 30.0);
        assert_eq!(l.gap_y(2), 30.0 + 30.0 + EFFECT_ROW_HEIGHT);
        assert_eq!(l.position_at(15.0), 0.5);
        assert_eq!(l.position_at(45.0), 1.5);
        // アンカーの行の上は、そのレイヤーの下の隙間
        assert_eq!(l.position_at(62.0), 2.0);
        assert_eq!(l.position_at(-3.0), -1.0);
        assert_eq!(l.position_at(10_000.0), 2.0);
    }
}
