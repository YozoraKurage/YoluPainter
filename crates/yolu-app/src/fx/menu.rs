//! 効果のメニューの項目（メニューバーの「フィルター」・レイヤーのパネルの効果のボタン・プロパティの欄のドロップダウン・マスクの右クリック）。
//! 断られる項目は黙って隠さず、押せない項目にして理由をツールチップに置く（チャンネルの型・レイヤーの種類・マスクの有無・読める Anchor が無い。
//! ラベルは名前だけで、理由は続けない）。フィルターとジェネレーターは入り口を分ける（「フィルターを追加」と「ジェネレーターを追加」の
//! 別々のボタン・入れ子。どちらも同じスタックに積む）。

use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::{self, anchor::ReadMode, Blend, Kind, NoiseSpace, Shape};
use yolu_core::{AnchorId, AnchorPlacement, Channel, EffectSettings, FilterTarget, LayerId};

use super::names::{self, FilterKind};
use super::FxOp;
use crate::lang::Lang;
use crate::m2::Edit;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;

/// 効果を足す先: マスクに描いているあいだはマスク、そうでなければレイヤーの画素。
pub fn target(app: &AppState) -> FilterTarget {
    match app.selected_layer {
        Some(id) => target_for(app, id),
        None => FilterTarget::Content,
    }
}

/// `layer` に効果を足す先（そのレイヤーを選んでマスクに描いているあいだはマスク、そうでなければレイヤーの画素）。
pub fn target_for(app: &AppState, layer: LayerId) -> FilterTarget {
    if super::mask_target(app) == Some(layer) {
        FilterTarget::Mask
    } else {
        FilterTarget::Content
    }
}

/// 押せない効果の項目（ラベルは名前だけ。理由はツールチップ）。
fn refused(label: &str, why: impl Into<String>) -> Entry<Action> {
    Entry::item(label, Action::Fx(FxOp::Deselect))
        .enabled(false)
        .tooltip(why)
}

/// 「フィルターを追加」（ボタン・入れ子のメニューの名前）。
pub fn add_filter_label(lang: Lang) -> &'static str {
    lang.pick("フィルターを追加", "Add Filter")
}

/// 「ジェネレーターを追加」（ボタン・入れ子のメニューの名前）。
pub fn add_generator_label(lang: Lang) -> &'static str {
    lang.pick("ジェネレーターを追加", "Add Generator")
}

/// フィルターを足す項目（平らな並び。足す先は `target`）。断られる項目は、押せない項目にして理由をツールチップに置く。
pub fn filter_entries(app: &AppState, layer: LayerId, target: FilterTarget) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let channel = app.m2.paint_channel;
    let mut v = Vec::new();
    for kind in FilterKind::ALL {
        let label = kind.name(lang);
        match app
            .doc
            .filter_refusal(layer, target, &kind.settings(), channel)
        {
            Ok(()) => v.push(
                Entry::item(label, Action::Fx(FxOp::AddFilter { target, kind })).enabled(free),
            ),
            Err(why) => v.push(refused(label, lang.core_error(&why))),
        }
    }
    v
}

/// Generator を足す項目（平らな並び。足す先は `target`）。
pub fn generator_entries(
    app: &AppState,
    layer: LayerId,
    target: FilterTarget,
) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let channel = app.m2.paint_channel;
    let mut v = Vec::new();
    for kind in names::GENERATOR_KINDS {
        let label = names::generator_name(lang, kind);
        let settings = EffectSettings::generator(generator::Settings::new(kind));
        let refusal = app
            .doc
            .filter_refusal(layer, target, &settings, channel)
            .err();
        let no_anchor = kind == Kind::Anchor
            && refusal.is_none()
            && app
                .doc
                .anchors_readable_from(layer)
                .map_or(true, |a| a.is_empty());
        match (refusal, no_anchor) {
            (None, false) => v.push(
                Entry::item(label, Action::Fx(FxOp::AddGenerator { target, kind })).enabled(free),
            ),
            (Some(why), _) => v.push(refused(label, lang.core_error(&why))),
            (None, true) => v.push(refused(
                label,
                lang.pick(
                    "このレイヤーより下にアンカーが無い",
                    "no anchor below this layer",
                ),
            )),
        }
    }
    v
}

/// 選んでいるレイヤーの、足せるレイヤー（無ければ None）。
fn selected_layer(app: &AppState) -> Option<LayerId> {
    app.selected_layer.filter(|id| app.doc.layer(*id).is_some())
}

/// 「フィルターを追加」のボタンのポップアップ（フィルターだけ。足す先は `target`）。選んでいるレイヤーが無ければ空。
pub fn add_filter_entries(app: &AppState, target: FilterTarget) -> Vec<Entry<Action>> {
    selected_layer(app).map_or_else(Vec::new, |layer| filter_entries(app, layer, target))
}

/// 「ジェネレーターを追加」のボタンのポップアップ（ジェネレーターだけ。足す先は `target`）。選んでいるレイヤーが無ければ空。
pub fn add_generator_entries(app: &AppState, target: FilterTarget) -> Vec<Entry<Action>> {
    selected_layer(app).map_or_else(Vec::new, |layer| generator_entries(app, layer, target))
}

/// レイヤーのメニューの効果の項目: 「フィルターを追加 ▸」「ジェネレーターを追加 ▸」、アンカーを置く・外す。足す先は `target_for`。
pub fn layer_entries(app: &AppState, layer: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let target = target_for(app, layer);
    let mut v = vec![
        Entry::submenu(add_filter_label(lang), filter_entries(app, layer, target)),
        Entry::submenu(
            add_generator_label(lang),
            generator_entries(app, layer, target),
        ),
    ];
    v.extend(anchor_entries_for(app, layer));
    v
}

/// マスクのサムネイルの右クリック: 「フィルターを追加 ▸」「ジェネレーターを追加 ▸」（足す先はマスク）・反転・有効・マスクを削除
/// （プロパティのマスクの欄と同じ操作）。マスクが無ければ空。
pub fn mask_entries(app: &AppState, layer: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some(mask) = app.doc.layer(layer).and_then(|l| l.mask()) else {
        return Vec::new();
    };
    let (enabled, inverted) = (mask.enabled(), mask.inverted());
    vec![
        Entry::submenu(
            add_filter_label(lang),
            filter_entries(app, layer, FilterTarget::Mask),
        ),
        Entry::submenu(
            add_generator_label(lang),
            generator_entries(app, layer, FilterTarget::Mask),
        ),
        Entry::Separator,
        Entry::item(
            lang.pick("反転", "Invert"),
            Action::M2(Edit::MaskInverted(layer, !inverted)),
        )
        .checked(inverted)
        .enabled(free),
        Entry::item(
            lang.pick("有効", "Enabled"),
            Action::M2(Edit::MaskEnabled(layer, !enabled)),
        )
        .checked(enabled)
        .enabled(free),
        Entry::Separator,
        Entry::item(
            lang.pick("マスクを削除", "Delete Mask"),
            Action::M2(Edit::RemoveMask(layer)),
        )
        .enabled(free),
    ]
}

/// アンカーを置く・外す項目（選んでいるレイヤーとそのマスク）。
pub fn anchor_entries(app: &AppState) -> Vec<Entry<Action>> {
    match app.selected_layer {
        Some(id) => anchor_entries_for(app, id),
        None => Vec::new(),
    }
}

/// アンカーを置く・外す項目（`id` のレイヤーとそのマスク）。
pub fn anchor_entries_for(app: &AppState, id: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some(layer) = app.doc.layer(id) else {
        return Vec::new();
    };
    let mut v = Vec::new();
    match layer.anchor() {
        None => v.push(
            Entry::item(
                lang.pick("アンカーを置く", "Add Anchor"),
                Action::Fx(FxOp::AddAnchor {
                    layer: id,
                    placement: AnchorPlacement::Layer,
                }),
            )
            .enabled(free),
        ),
        Some(a) => v.push(
            Entry::item(
                lang.pick("アンカーを外す", "Remove Anchor"),
                Action::Fx(FxOp::RemoveAnchor(a.id())),
            )
            .enabled(free),
        ),
    }
    if let Some(mask) = layer.mask() {
        match mask.anchor() {
            None => v.push(
                Entry::item(
                    lang.pick("マスクにアンカーを置く", "Add Anchor to Mask"),
                    Action::Fx(FxOp::AddAnchor {
                        layer: id,
                        placement: AnchorPlacement::Mask,
                    }),
                )
                .enabled(free),
            ),
            Some(a) => v.push(
                Entry::item(
                    lang.pick("マスクのアンカーを外す", "Remove Mask Anchor"),
                    Action::Fx(FxOp::RemoveAnchor(a.id())),
                )
                .enabled(free),
            ),
        }
    }
    v
}

/// 効果の行の右クリック（選んでいる段: 有効の切り替え・上へ・下へ・消す、選んでいるアンカー: 外す）。
pub fn context_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    if let Some((layer, effect, target)) = app.fx.filter(&app.doc) {
        let id = effect.id();
        let stack = app.doc.filters_of(layer, target).unwrap_or(&[]);
        let index = stack.iter().position(|e| e.id() == id).unwrap_or(0);
        let count = stack.len();
        let on = effect.enabled();
        vec![
            Entry::item(
                if on {
                    lang.pick("フィルターを無効にする", "Turn the filter off")
                } else {
                    lang.pick("フィルターを有効にする", "Turn the filter on")
                },
                Action::Fx(FxOp::SetEnabled {
                    layer,
                    id,
                    enabled: !on,
                }),
            )
            .enabled(free),
            Entry::item(
                lang.pick("上へ（後から掛かる）", "Move up (applied later)"),
                Action::Fx(FxOp::Move {
                    layer,
                    id,
                    index: index + 1,
                }),
            )
            .enabled(free && index + 1 < count),
            Entry::item(
                lang.pick("下へ（先に掛かる）", "Move down (applied earlier)"),
                Action::Fx(FxOp::Move {
                    layer,
                    id,
                    index: index.wrapping_sub(1),
                }),
            )
            .enabled(free && index > 0),
            Entry::item(
                lang.pick("フィルターを削除", "Remove the filter"),
                Action::Fx(FxOp::Remove { layer, id }),
            )
            .enabled(free),
        ]
    } else if let Some(info) = app.fx.anchor(&app.doc) {
        vec![Entry::item(
            lang.pick("アンカーを外す", "Remove Anchor"),
            Action::Fx(FxOp::RemoveAnchor(info.anchor.id())),
        )
        .enabled(free)]
    } else {
        Vec::new()
    }
}

/// メニューバーの「フィルター」の中身（フィルターとアンカー。ジェネレーターは「レイヤー」のメニューの「ジェネレーターを追加 ▸」）。
pub fn menu_entries(app: &AppState) -> Vec<Entry<Action>> {
    let mut v = add_filter_entries(app, target(app));
    let anchors = anchor_entries(app);
    if !v.is_empty() && !anchors.is_empty() {
        v.push(Entry::Separator);
    }
    v.extend(anchors);
    v
}

/// プロパティの欄のドロップダウンの選択肢（開いている種類）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxChoice {
    Blend,
    Axis,
    Direction,
    NoiseSpace,
    ProceduralSpace,
    NoiseBasis,
    CellOutput,
    FractalMode,
    Shape,
    Anchor,
    AnchorChannel,
    AnchorRead,
    /// 画像の段の画像（アセットの画像の一覧・ファイルから取り込む・外す）。
    Image,
    /// 画像の段の投影の種類（デカールは無い）。
    ImageMode,
    /// 画像の段の投影の外側。
    ImageWrap,
    /// 画像の段がマスク・スカラーで値にする成分。
    ImageComponent,
}

/// 選んでいる Generator の設定を 1 つ変えて渡す操作。
fn set_generator(layer: LayerId, id: yolu_core::FilterId, g: generator::Settings) -> Action {
    Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings: EffectSettings::generator(g),
        coalesce: false,
    })
}

pub fn choice_entries(app: &AppState, choice: FxChoice) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some((layer, effect, _)) = app.fx.filter(&app.doc) else {
        return Vec::new();
    };
    let id = effect.id();
    let Some(g) = effect.settings().generator_settings().cloned() else {
        return Vec::new();
    };
    match choice {
        FxChoice::Blend => names::BLENDS
            .iter()
            .map(|b: &Blend| {
                let mut next = g.clone();
                next.blend = *b;
                Entry::item(names::blend_name(lang, *b), set_generator(layer, id, next))
                    .radio(g.blend == *b)
                    .enabled(free)
            })
            .collect(),
        FxChoice::Axis => (0..3usize)
            .map(|a| {
                let mut next = g.clone();
                next.axis = a;
                Entry::item(names::axis_name(a), set_generator(layer, id, next))
                    .radio(g.axis == a)
                    .enabled(free)
            })
            .collect(),
        FxChoice::Direction => {
            let mut options: Vec<[f64; 3]> = names::DIRECTIONS.to_vec();
            if !options.contains(&g.direction) {
                options.push(g.direction);
            }
            options
                .into_iter()
                .map(|d| {
                    let mut next = g.clone();
                    next.direction = d;
                    Entry::item(
                        names::direction_name(lang, d),
                        set_generator(layer, id, next),
                    )
                    .radio(g.direction == d)
                    .enabled(free)
                })
                .collect()
        }
        FxChoice::ProceduralSpace => names::PROCEDURAL_SPACES
            .iter()
            .map(|value| {
                let mut next = g.clone();
                next.procedural.space = *value;
                Entry::item(
                    names::procedural_space_name(lang, *value),
                    set_generator(layer, id, next),
                )
                .radio(g.procedural.space == *value)
                .enabled(free)
            })
            .collect(),
        FxChoice::NoiseBasis => names::NOISE_BASES
            .iter()
            .map(|value| {
                let mut next = g.clone();
                next.procedural.basis = *value;
                // セルの出力は Worley 専用（core が断る）ので、ほかの基底へ替えるときは既定へ戻す
                if *value != generator::NoiseBasis::Worley {
                    next.procedural.cell_output = generator::CellOutput::F1;
                }
                Entry::item(
                    names::noise_basis_name(lang, *value),
                    set_generator(layer, id, next),
                )
                .radio(g.procedural.basis == *value)
                .enabled(free)
            })
            .collect(),
        FxChoice::CellOutput => names::CELL_OUTPUTS
            .iter()
            .map(|value| {
                let mut next = g.clone();
                next.procedural.cell_output = *value;
                Entry::item(
                    names::cell_output_name(*value),
                    set_generator(layer, id, next),
                )
                .radio(g.procedural.cell_output == *value)
                .enabled(free)
            })
            .collect(),
        FxChoice::FractalMode => names::FRACTAL_MODES
            .iter()
            .map(|value| {
                let mut next = g.clone();
                next.procedural.fractal = *value;
                Entry::item(
                    names::fractal_mode_name(*value),
                    set_generator(layer, id, next),
                )
                .radio(g.procedural.fractal == *value)
                .enabled(free)
            })
            .collect(),
        FxChoice::NoiseSpace => [NoiseSpace::Model, NoiseSpace::Uv]
            .iter()
            .map(|s| {
                let mut next = g.clone();
                next.noise_space = *s;
                Entry::item(
                    names::noise_space_name(lang, *s),
                    set_generator(layer, id, next),
                )
                .radio(g.noise_space == *s)
                .enabled(free)
            })
            .collect(),
        FxChoice::Shape => names::SHAPES
            .iter()
            .map(|s: &Shape| {
                let mut next = g.clone();
                next.volume.shape = *s;
                Entry::item(names::shape_name(lang, *s), set_generator(layer, id, next))
                    .radio(g.volume.shape == *s)
                    .enabled(free)
            })
            .collect(),
        FxChoice::Anchor => {
            let mut choices: Vec<Option<AnchorId>> = vec![None];
            if let Ok(readable) = app.doc.anchors_readable_from(layer) {
                choices.extend(readable.iter().map(|a| Some(a.anchor.id())));
            }
            let current = (g.anchor.id != 0).then_some(AnchorId(g.anchor.id));
            if current.is_some() && !choices.contains(&current) {
                choices.push(current);
            }
            choices
                .into_iter()
                .map(|c| {
                    Entry::item(
                        anchor_choice_name(app, c),
                        Action::Fx(FxOp::SetAnchorRef {
                            layer,
                            filter: id,
                            anchor: c,
                            channel: g.anchor.channel,
                            read: g.anchor.read,
                        }),
                    )
                    .radio(current == c)
                    .enabled(free)
                })
                .collect()
        }
        FxChoice::AnchorChannel => names::ANCHOR_CHANNELS
            .iter()
            .map(|c: &Channel| {
                Entry::item(
                    crate::m2::channel_name(lang, &app.doc, *c),
                    Action::Fx(FxOp::SetAnchorRef {
                        layer,
                        filter: id,
                        anchor: (g.anchor.id != 0).then_some(AnchorId(g.anchor.id)),
                        channel: *c,
                        read: g.anchor.read,
                    }),
                )
                .radio(g.anchor.channel == *c)
                .enabled(free)
            })
            .collect(),
        FxChoice::AnchorRead => [ReadMode::Value, ReadMode::Coverage]
            .iter()
            .map(|r| {
                Entry::item(
                    names::read_mode_name(lang, *r),
                    Action::Fx(FxOp::SetAnchorRef {
                        layer,
                        filter: id,
                        anchor: (g.anchor.id != 0).then_some(AnchorId(g.anchor.id)),
                        channel: g.anchor.channel,
                        read: *r,
                    }),
                )
                .radio(g.anchor.read == *r)
                .enabled(free)
            })
            .collect(),
        FxChoice::Image => {
            let current = (g.image.image != 0).then_some(yolu_core::ImageId(g.image.image));
            crate::panels::fill_props::image_list(app, current, |image| {
                Action::Fx(FxOp::SetImage { layer, id, image })
            })
        }
        FxChoice::ImageMode => names::IMAGE_PROJECTIONS
            .iter()
            .map(|mode| {
                let mut next = g.clone();
                let p = &mut next.image.projection;
                // UV から型の上の投影へ替えたとき、置き場が初めのままならモデルの外形に合わせる（塗りつぶしレイヤーと同じ）
                if p.mode == ProjectionMode::Uv
                    && *mode != ProjectionMode::Uv
                    && p.placement == yolu_core::fill_image::Placement::default()
                {
                    if let Some(placement) = app.fitted_placement_for(*mode, None) {
                        p.placement = placement;
                    }
                }
                p.mode = *mode;
                Entry::item(
                    crate::panels::fill_props::projection_name(lang, *mode),
                    set_generator(layer, id, next),
                )
                .radio(g.image.projection.mode == *mode)
                .enabled(free)
            })
            .collect(),
        FxChoice::ImageWrap => names::IMAGE_WRAPS
            .iter()
            .map(|wrap| {
                let mut next = g.clone();
                next.image.projection.wrap = *wrap;
                Entry::item(
                    crate::panels::fill_props::wrap_name(lang, *wrap),
                    set_generator(layer, id, next),
                )
                .radio(g.image.projection.wrap == *wrap)
                .enabled(free)
            })
            .collect(),
        FxChoice::ImageComponent => generator::ImageComponent::ALL
            .iter()
            .map(|c| {
                let mut next = g.clone();
                next.image.component = *c;
                Entry::item(
                    names::image_component_name(lang, *c),
                    set_generator(layer, id, next),
                )
                .radio(g.image.component == *c)
                .enabled(free)
            })
            .collect(),
    }
}

/// Anchor の選択肢の名前（選んでいない・無くなった・名前）。
pub fn anchor_choice_name(app: &AppState, id: Option<AnchorId>) -> String {
    let lang = app.lang;
    match id {
        None => lang.pick("なし", "None").to_owned(),
        Some(id) => match app.doc.find_anchor(id) {
            None => lang.pick("（無いアンカー）", "(missing anchor)").to_owned(),
            Some(info) => names::anchor_label(lang, info.anchor, info.placement),
        },
    }
}
