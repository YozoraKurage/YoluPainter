//! 文書の命令。`yolu_core::Document` の上の関数として 1 か所にあり、画面なしのホストも起動中のアプリのホストも同じ関数を通す。
//!
//! - 編集は 1 つの命令が `Document::batch` 1 回（取り消しの 1 段）。途中で断ると、積んだ段を戻して文書を元のままにする。
//!   名前の解決・値の検査は文書を変える前に済ませる（断る理由を、何も変えないうちに返す）。
//! - 読む命令は文書を変えない。

use std::collections::BTreeMap;

use serde_json::json;
use yolu_core::effects::catalog::ParamValue;
use yolu_core::effects::{
    EffectSettings, FilterEffect, FilterId, FilterSpec, FilterTarget, InactiveEffect,
    InactiveReason, InactiveTarget,
};
use yolu_core::{
    AdjustmentSettings, Channel, ChannelKind, CoreError, Document, Layer, LayerId, LayerKind,
    LayerLocks, Rgba8,
};

use crate::command::*;
use crate::error::{ErrorCode, Noun, OpError};
use crate::refs::{
    channel_name, check_layer_name, parse_blend, parse_filter_id, resolve_channel, resolve_layer,
};
use crate::reply::*;
use crate::text::Text;
use crate::text_layer::{self, patched, text_info, FontData};
use crate::value::{format_color, parse_color, Value};
use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace};
use yolu_core::text::TextSettings;

/// 命令が当たるセットの、文書の外の事実。
#[derive(Clone, Copy, Debug)]
pub struct SetFacts<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// 開いた・保存したあとに編集した。
    pub unsaved: bool,
}

/// 命令が文書を変えるか（変えるなら `write`、読むだけなら `read` に渡す）。
pub fn is_write(command: &Command) -> bool {
    matches!(
        command,
        Command::LayerAdd(_)
            | Command::LayerDelete(_)
            | Command::LayerMove(_)
            | Command::LayerSet(_)
            | Command::MaskAdd(_)
            | Command::MaskDelete(_)
            | Command::MaskSet(_)
            | Command::EffectAdd(_)
            | Command::EffectSet(_)
            | Command::EffectDelete(_)
            | Command::Undo(_)
            | Command::Redo(_)
    )
}

/// 読む命令を文書に当てる（`set.info`・`layer.get`・`effect.get`・`history.info`）。
pub fn read(facts: SetFacts<'_>, doc: &Document, command: &Command) -> Result<Reply, OpError> {
    match command {
        Command::SetInfo(_) => Ok(Reply::Set(set_info(facts, doc))),
        Command::LayerGet(a) => {
            let id = resolve_layer(doc, &a.layer)?;
            Ok(Reply::Layer(layer_info(doc, layer_of(doc, id)?)))
        }
        Command::EffectGet(a) => effect_get(doc, a),
        Command::HistoryInfo(_) => Ok(Reply::History(history_info(doc))),
        other => Err(OpError::new(
            ErrorCode::Internal,
            format!("{} はプロジェクトを読む命令ではありません", other.name()),
            format!("{} is not a document read command", other.name()),
        )),
    }
}

/// 文書を変える命令を当てる。
pub fn write(facts: SetFacts<'_>, doc: &mut Document, command: &Command) -> Result<Reply, OpError> {
    write_with_font(facts, doc, command, None)
}

/// 文書を変える命令を当てる。テキストレイヤーを描く命令には、前に探したフォント（[`crate::text_layer::font_for`]）を渡す。
pub fn write_with_font(
    facts: SetFacts<'_>,
    doc: &mut Document,
    command: &Command,
    font: Option<&FontData>,
) -> Result<Reply, OpError> {
    let before = doc.undo_count();
    match command {
        Command::LayerAdd(a) => layer_add(facts, doc, a, font, before),
        Command::LayerDelete(a) => {
            let id = resolve_layer(doc, &a.layer)?;
            batch(doc, |d| d.remove_layer(id))?;
            Ok(edited(facts, doc, None, None, before))
        }
        Command::LayerMove(a) => layer_move(facts, doc, a, before),
        Command::LayerSet(a) => layer_set(facts, doc, a, font, before),
        Command::MaskAdd(a) => {
            let id = resolve_layer(doc, &a.layer)?;
            batch(doc, |d| d.add_layer_mask(id))?;
            Ok(edited(facts, doc, Some(id), None, before))
        }
        Command::MaskDelete(a) => {
            let id = resolve_layer(doc, &a.layer)?;
            batch(doc, |d| d.remove_layer_mask(id))?;
            Ok(edited(facts, doc, Some(id), None, before))
        }
        Command::MaskSet(a) => mask_set(facts, doc, a, before),
        Command::EffectAdd(a) => effect_add(facts, doc, a, before),
        Command::EffectSet(a) => effect_set(facts, doc, a, before),
        Command::EffectDelete(a) => {
            let layer = resolve_layer(doc, &a.layer)?;
            let (id, _, _) = find_effect(doc, layer, &a.effect)?;
            batch(doc, |d| d.remove_filter(layer, id))?;
            Ok(edited(facts, doc, Some(layer), None, before))
        }
        Command::Undo(a) => step_history(facts, doc, a, true),
        Command::Redo(a) => step_history(facts, doc, a, false),
        other => Err(OpError::new(
            ErrorCode::Internal,
            format!("{} はプロジェクトを変える命令ではありません", other.name()),
            format!("{} is not a document write command", other.name()),
        )),
    }
}

/// 1 つの命令を取り消しの 1 段にする（core の `batch`。断ると文書は元のまま）。もうまとめの中（アクションの実行が、命令の列の全部を
/// 1 つのまとめで当てている）なら、そのまとめに積む（まとめは入れ子にできない。途中で断った命令の段は、外のまとめが全部と一緒に戻す）。
fn batch<T>(
    doc: &mut Document,
    edits: impl FnOnce(&mut Document) -> Result<T, CoreError>,
) -> Result<T, OpError> {
    if doc.is_batching() {
        return edits(doc).map_err(|e| OpError::from_core(&e));
    }
    doc.batch(edits).map_err(|e| OpError::from_core(&e))
}

fn layer_of(doc: &Document, id: LayerId) -> Result<&Layer, OpError> {
    doc.layer(id)
        .ok_or_else(|| OpError::not_found(Noun::Layer, &id.to_string()))
}

fn edited(
    facts: SetFacts<'_>,
    doc: &Document,
    layer: Option<LayerId>,
    effect: Option<FilterId>,
    undo_before: usize,
) -> Reply {
    Reply::Edited(Edited {
        set: facts.id.to_owned(),
        layer: layer.map(|l| l.to_string()),
        effect: effect.map(|e| e.to_string()),
        unchanged: doc.undo_count() == undo_before,
        undo_count: doc.undo_count() as u32,
        can_undo: doc.can_undo(),
        notes: Vec::new(),
    })
}

/// 編集の返事に知らせを添える（編集の返事でなければそのまま）。
fn with_note(mut reply: Reply, note: Text) -> Reply {
    if let Reply::Edited(e) = &mut reply {
        e.notes.push(note);
    }
    reply
}

// ───────── 読む ─────────

pub fn kind_name(kind: LayerKind) -> LayerKindName {
    match kind {
        LayerKind::Raster => LayerKindName::Paint,
        LayerKind::Fill => LayerKindName::Fill,
        LayerKind::Adjustment => LayerKindName::Adjustment,
        LayerKind::Group => LayerKindName::Group,
    }
}

pub fn layer_summary(doc: &Document, layer: &Layer) -> LayerSummary {
    LayerSummary {
        id: layer.id().to_string(),
        name: layer.name().to_owned(),
        kind: if layer.text().is_some() {
            LayerKindName::Text
        } else {
            kind_name(layer.kind())
        },
        visible: layer.visible(),
        opacity: layer.opacity(),
        blend_mode: layer.blend_mode().name().to_owned(),
        clipping: layer.clipping(),
        parent: layer.parent().map(|p| p.to_string()),
        depth: doc.depth_of(layer.id()).unwrap_or(0) as u32,
        has_mask: layer.mask().is_some(),
        effect_count: layer.filters().len() as u32,
    }
}

pub fn channel_infos(doc: &Document) -> Vec<ChannelInfo> {
    doc.channels()
        .into_iter()
        .filter_map(|c| {
            let info = doc.channel_info(c)?;
            Some(ChannelInfo {
                index: c.index() as u8,
                name: info.name.clone(),
                kind: match info.kind {
                    ChannelKind::Color => ChannelValueKind::Color,
                    ChannelKind::Scalar => ChannelValueKind::Scalar,
                    ChannelKind::Normal => ChannelValueKind::Normal,
                },
                color_space: match info.color_space {
                    yolu_core::ColorSpace::Srgb => ChannelSpace::Srgb,
                    yolu_core::ColorSpace::Linear => ChannelSpace::Linear,
                },
            })
        })
        .collect()
}

pub fn set_info(facts: SetFacts<'_>, doc: &Document) -> SetInfo {
    SetInfo {
        id: facts.id.to_owned(),
        name: facts.name.to_owned(),
        width: doc.width(),
        height: doc.height(),
        state: SetState::Editable,
        reason: None,
        channels: channel_infos(doc),
        // 平らな並びは下から上でグループの中身がグループの前に続くので、逆にすると上から下で、グループが中身の前に来る
        layers: doc
            .layers()
            .iter()
            .rev()
            .map(|l| layer_summary(doc, l))
            .collect(),
        inactive_effects: inactive_texts(doc),
        unsaved: facts.unsaved,
    }
}

pub fn history_info(doc: &Document) -> HistoryInfo {
    HistoryInfo {
        undo_count: doc.undo_count() as u32,
        redo_count: doc.redo_count() as u32,
        can_undo: doc.can_undo(),
        can_redo: doc.can_redo(),
    }
}

fn values_of(settings: &EffectSettings) -> BTreeMap<String, Value> {
    settings
        .catalog_values()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.into()))
        .collect()
}

fn target_of(t: FilterTarget) -> EffectTarget {
    match t {
        FilterTarget::Content => EffectTarget::Content,
        FilterTarget::Mask => EffectTarget::Mask,
    }
}

fn core_target(t: EffectTarget) -> FilterTarget {
    match t {
        EffectTarget::Content => FilterTarget::Content,
        EffectTarget::Mask => FilterTarget::Mask,
    }
}

pub fn effect_info(
    doc: &Document,
    fe: &FilterEffect,
    target: FilterTarget,
    index: usize,
) -> EffectInfo {
    EffectInfo {
        id: fe.id().to_string(),
        kind: fe.settings().kind_id().to_owned(),
        values: values_of(fe.settings()),
        opaque: fe
            .settings()
            .opaque_parts()
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        strength: fe.strength(),
        enabled: fe.enabled(),
        channels: fe
            .channels()
            .iter()
            .map(|c| channel_name(doc, *c))
            .collect(),
        target: target_of(target),
        index: index as u32,
    }
}

fn effects_of(doc: &Document, layer: &Layer) -> Vec<EffectInfo> {
    layer
        .filters()
        .iter()
        .enumerate()
        .map(|(i, fe)| effect_info(doc, fe, FilterTarget::Content, i))
        .collect()
}

fn adjustment_values(a: &AdjustmentSettings) -> EffectValues {
    EffectValues {
        kind: a.kind_id().to_owned(),
        values: a
            .catalog_values()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into()))
            .collect(),
        opaque: a.opaque_parts().iter().map(|s| (*s).to_owned()).collect(),
    }
}

pub fn layer_info(doc: &Document, layer: &Layer) -> LayerInfo {
    let locks = layer.locks();
    LayerInfo {
        summary: layer_summary(doc, layer),
        locks: LockState {
            transparency: locks.contains(LayerLocks::TRANSPARENCY),
            pixels: locks.contains(LayerLocks::PIXELS),
            position: locks.contains(LayerLocks::POSITION),
            all: locks.contains(LayerLocks::ALL),
        },
        channels: doc
            .channels()
            .into_iter()
            .map(|c| {
                let blend = layer.channel_blend(c);
                LayerChannel {
                    channel: channel_name(doc, c),
                    enabled: layer.is_channel_enabled(c),
                    fill: layer.fill_value(c).map(format_color),
                    blend_mode: blend.mode.map(|m| m.name().to_owned()),
                    opacity: blend.opacity,
                    points: layer.fill_points(c).map(points_spec),
                }
            })
            .collect(),
        adjustment: layer.adjustment().map(adjustment_values),
        mask: layer.mask().map(|m| MaskInfo {
            enabled: m.enabled(),
            inverted: m.inverted(),
            density: m.density(),
            effects: m
                .filters()
                .iter()
                .enumerate()
                .map(|(i, fe)| effect_info(doc, fe, FilterTarget::Mask, i))
                .collect(),
        }),
        effects: effects_of(doc, layer),
        text: layer.text().map(|t| Box::new(text_info(t))),
    }
}

fn effect_get(doc: &Document, args: &EffectGetArgs) -> Result<Reply, OpError> {
    let layer_id = resolve_layer(doc, &args.layer)?;
    let layer = layer_of(doc, layer_id)?;
    let effects = match &args.effect {
        None => {
            let mut all = effects_of(doc, layer);
            if let Some(mask) = layer.mask() {
                all.extend(
                    mask.filters()
                        .iter()
                        .enumerate()
                        .map(|(i, fe)| effect_info(doc, fe, FilterTarget::Mask, i)),
                );
            }
            all
        }
        Some(text) => {
            let (id, target, index) = find_effect(doc, layer_id, text)?;
            let (_, fe, _) = doc
                .find_filter(id)
                .ok_or_else(|| OpError::not_found(Noun::Effect, text))?;
            vec![effect_info(doc, fe, target, index)]
        }
    };
    Ok(Reply::Effects(EffectsInfo {
        layer: layer_id.to_string(),
        effects,
    }))
}

/// 効果の ID を、そのレイヤーの中から探す（別のレイヤーの効果は「無い」）。ID・スタック・位置を返す。
fn find_effect(
    doc: &Document,
    layer: LayerId,
    text: &str,
) -> Result<(FilterId, FilterTarget, usize), OpError> {
    let id = parse_filter_id(text)?;
    match doc.find_filter(id) {
        Some((owner, _, target)) if owner == layer => {
            let stack = doc
                .filters_of(layer, target)
                .map_err(|e| OpError::from_core(&e))?;
            let index = stack.iter().position(|e| e.id() == id).unwrap_or(0);
            Ok((id, target, index))
        }
        _ => Err(OpError::not_found(Noun::Effect, text)),
    }
}

// ───────── 効いていない効果 ─────────

/// 入力のまま通している効果の知らせ（日英）。
pub fn inactive_texts(doc: &Document) -> Vec<Text> {
    doc.inactive_effect_list()
        .iter()
        .map(inactive_text)
        .collect()
}

fn inactive_text(e: &InactiveEffect) -> Text {
    use yolu_core::generator::Inactive as I;
    let what = match e.target {
        InactiveTarget::Generator { mask, kind } => {
            let name = match kind {
                yolu_core::generator::Kind::EdgeWear => "edge wear",
                yolu_core::generator::Kind::Dirt => "dirt",
                yolu_core::generator::Kind::PositionGradient => "position gradient",
                yolu_core::generator::Kind::Thickness => "thickness",
                yolu_core::generator::Kind::Direction => "direction",
                yolu_core::generator::Kind::ShapeGradient => "shape gradient",
                yolu_core::generator::Kind::IdColor => "ID color",
                yolu_core::generator::Kind::Anchor => "anchor",
                yolu_core::generator::Kind::Noise => "noise",
                yolu_core::generator::Kind::Grunge => "grunge",
                yolu_core::generator::Kind::Image => "image",
                yolu_core::generator::Kind::Pattern => "pattern",
                yolu_core::generator::Kind::Light => "light",
                yolu_core::generator::Kind::MaskBuilder => "mask builder",
                yolu_core::generator::Kind::UvIslandVariation => "UV island variation",
            };
            if mask {
                format!("{name} generator (mask)")
            } else {
                format!("{name} generator")
            }
        }
        InactiveTarget::FillGradient(c) => format!("gradient ({c:?})"),
        InactiveTarget::Decal => "decal".to_owned(),
        InactiveTarget::FillImage(c) => format!("image ({c:?})"),
        InactiveTarget::FillPoints(c) => format!("point gradient ({c:?})"),
    };
    let why = match &e.reason {
        InactiveReason::Generator(I::MissingMap(k)) => format!("no {k:?} map is available"),
        InactiveReason::Generator(I::StaleMap(k)) => {
            format!("the {k:?} map was baked under other conditions")
        }
        InactiveReason::Generator(I::UnverifiedMap(k)) => {
            format!("the {k:?} map cannot be verified")
        }
        InactiveReason::Generator(I::MapSize(k)) => {
            format!("the {k:?} map has another size than the texture set")
        }
        InactiveReason::Generator(I::PinMismatch(k)) => {
            format!("the {k:?} map differs from the pinned bake")
        }
        InactiveReason::Generator(I::MissingFrame) => {
            "the model root position is unknown".to_owned()
        }
        InactiveReason::Generator(I::EmptyBounds) => "the position bounds are empty".to_owned(),
        InactiveReason::Generator(I::NoIdColors) => "no ID colors are chosen".to_owned(),
        InactiveReason::Generator(I::Anchor(_)) => "the anchor is not usable".to_owned(),
        InactiveReason::Generator(I::NoImage) => "no image is chosen".to_owned(),
        InactiveReason::Generator(I::MissingImage) => {
            "the image is not in the project or cannot be read".to_owned()
        }
        InactiveReason::Generator(I::NoModel) => "no model is loaded".to_owned(),
        InactiveReason::Generator(I::IslandMap) => {
            "the UV island map does not fit in the working memory budget".to_owned()
        }
        InactiveReason::Rejected(_) => "the settings cannot be used".to_owned(),
    };
    Text::new(
        e.to_string(),
        format!(
            "Layer \"{}\": the {what} passes its input through ({why})",
            e.layer_name
        ),
    )
}

// ───────── レイヤー ─────────

fn color_of(text: &str) -> Result<Rgba8, OpError> {
    parse_color(text).ok_or_else(|| {
        OpError::invalid_value(
            format!("色は #rrggbb か #rrggbbaa です（{text}）"),
            format!("A color is #rrggbb or #rrggbbaa ({text})"),
        )
    })
}

fn points_spec(g: &PointGradient) -> PointGradientSpec {
    PointGradientSpec {
        space: match g.space {
            PointSpace::Model => PointSpaceName::Model,
            PointSpace::Uv => PointSpaceName::Uv,
        },
        spread: Some(g.spread),
        points: g
            .points
            .iter()
            .map(|p| PointSpec {
                position: match g.space {
                    PointSpace::Model => p.position.to_vec(),
                    PointSpace::Uv => p.position[..2].to_vec(),
                },
                color: format_color(p.color),
            })
            .collect(),
    }
}

fn points_of(spec: &PointGradientSpec) -> Result<PointGradient, OpError> {
    let space = match spec.space {
        PointSpaceName::Model => PointSpace::Model,
        PointSpaceName::Uv => PointSpace::Uv,
    };
    let mut points = Vec::with_capacity(spec.points.len());
    for p in &spec.points {
        let position = match (space, p.position.as_slice()) {
            (PointSpace::Model, [x, y, z]) => [*x, *y, *z],
            (PointSpace::Uv, [u, v]) => [*u, *v, 0.0],
            _ => {
                return Err(OpError::invalid_value(
                    "点の位置は、モデルの空間なら [x, y, z]、UV の空間なら [u, v] です",
                    "A point's position is [x, y, z] in model space and [u, v] in UV space",
                ))
            }
        };
        points.push(GradientPoint {
            position,
            color: color_of(&p.color)?,
        });
    }
    let g = PointGradient {
        space,
        spread: spec
            .spread
            .unwrap_or(yolu_core::fill_points::DEFAULT_SPREAD),
        points,
    };
    g.validate().map_err(|why| {
        OpError::invalid_value(
            format!("点のグラデーションが使えません（{why}）"),
            "The point gradient is out of range (1 to 64 points, finite positions within ±1e6, spread 0..=1)",
        )
    })?;
    Ok(g)
}

fn adjustment_of(spec: &EffectSpec) -> Result<AdjustmentSettings, OpError> {
    let values: BTreeMap<String, ParamValue> = spec
        .values
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().into()))
        .collect();
    Ok(AdjustmentSettings::from_catalog(&spec.kind, &values)?)
}

fn layer_add(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &LayerAddArgs,
    font: Option<&FontData>,
    before: usize,
) -> Result<Reply, OpError> {
    let above = args
        .above
        .as_deref()
        .map(|t| resolve_layer(doc, t))
        .transpose()?;
    if let Some(name) = &args.name {
        check_layer_name(name)?;
    }
    let misplaced = |what: &str| {
        OpError::invalid_request(
            format!("{what} はこの種類のレイヤーには使えません"),
            format!("{what} cannot be used with this kind of layer"),
        )
    };
    if args.kind != NewLayerKind::Text && args.text.is_some() {
        return Err(misplaced("text"));
    }
    let id = match args.kind {
        NewLayerKind::Paint | NewLayerKind::Group => {
            if !args.fill.is_empty() {
                return Err(misplaced("fill"));
            }
            if args.adjustment.is_some() {
                return Err(misplaced("adjustment"));
            }
            if !args.channels.is_empty() {
                return Err(misplaced("channels"));
            }
            if args.kind == NewLayerKind::Paint {
                let name = args.name.as_deref().unwrap_or("Layer");
                batch(doc, |d| d.add_layer_above(name, above))?
            } else {
                let name = args.name.as_deref().unwrap_or("Group");
                batch(doc, |d| d.add_group(name, above))?
            }
        }
        NewLayerKind::Fill => {
            if args.adjustment.is_some() {
                return Err(misplaced("adjustment"));
            }
            if args.channels.is_empty() && args.fill.is_empty() {
                return Err(OpError::invalid_request(
                    "塗りつぶしのレイヤーには fill（チャンネル → 色）が要ります",
                    "A fill layer needs `fill` (channel -> color)",
                ));
            }
            if !args.channels.is_empty() {
                return Err(misplaced("channels"));
            }
            let mut values = Vec::new();
            for (channel, color) in &args.fill {
                values.push((resolve_channel(doc, channel)?, color_of(color)?));
            }
            let name = args.name.as_deref().unwrap_or("Fill");
            batch(doc, |d| d.add_fill_layer(name, &values, above))?
        }
        NewLayerKind::Adjustment => {
            if !args.fill.is_empty() {
                return Err(misplaced("fill"));
            }
            let spec = args.adjustment.as_ref().ok_or_else(|| {
                OpError::invalid_request(
                    "調整のレイヤーには adjustment（種類と値）が要ります",
                    "An adjustment layer needs `adjustment` (kind and values)",
                )
            })?;
            let settings = adjustment_of(spec)?;
            let channels: Vec<Channel> = args
                .channels
                .iter()
                .map(|c| resolve_channel(doc, c))
                .collect::<Result<_, _>>()?;
            let name = args.name.as_deref().unwrap_or("Adjustment");
            let selected = (!channels.is_empty()).then_some(channels.as_slice());
            batch(doc, |d| {
                d.add_adjustment_layer(name, settings, selected, above)
            })?
        }
        NewLayerKind::Text => {
            if !args.fill.is_empty() {
                return Err(misplaced("fill"));
            }
            if args.adjustment.is_some() {
                return Err(misplaced("adjustment"));
            }
            if !args.channels.is_empty() {
                return Err(misplaced("channels"));
            }
            let spec = args
                .text
                .as_ref()
                .filter(|t| t.content.is_some())
                .ok_or_else(|| {
                    OpError::invalid_request(
                        "テキストレイヤーには text.content（文）が要ります",
                        "A text layer needs `text.content`",
                    )
                })?;
            let font = font.ok_or_else(|| {
                OpError::new(
                    ErrorCode::Internal,
                    "テキストレイヤーのフォントを探していません",
                    "The font of the text layer was not looked up",
                )
            })?;
            // 既定の基準の点はキャンバスの左上
            let base = TextSettings::new("", font.font.clone(), 0.0, f64::from(doc.height()));
            let settings = patched(&base, spec, font)?;
            let name = args.name.as_deref().unwrap_or("Text");
            batch(doc, |d| {
                d.add_text_layer(name, settings, &font.bytes, above, false)
            })?
        }
    };
    Ok(edited(facts, doc, Some(id), None, before))
}

fn layer_move(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &LayerMoveArgs,
    before: usize,
) -> Result<Reply, OpError> {
    let id = resolve_layer(doc, &args.layer)?;
    if args.to_root && args.parent.is_some() {
        return Err(OpError::invalid_request(
            "parent と to_root は一緒に使えません",
            "`parent` and `to_root` cannot be used together",
        ));
    }
    let current = layer_of(doc, id)?.parent();
    let parent = if args.to_root {
        None
    } else if let Some(text) = &args.parent {
        Some(resolve_layer(doc, text)?)
    } else {
        current
    };
    let siblings = doc
        .layers()
        .iter()
        .filter(|l| l.parent() == parent && l.id() != id)
        .count();
    let index = args.index.unwrap_or(siblings);
    if index > siblings {
        return Err(OpError::invalid_value(
            format!("index は 0〜{siblings}（下から数える）です"),
            format!("index must be 0 to {siblings} (counted from the bottom)"),
        ));
    }
    batch(doc, |d| d.move_layer_to(id, parent, index))?;
    Ok(edited(facts, doc, Some(id), None, before))
}

fn layer_set(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &LayerSetArgs,
    font: Option<&FontData>,
    before: usize,
) -> Result<Reply, OpError> {
    let id = resolve_layer(doc, &args.layer)?;
    let text = match &args.text {
        None => None,
        Some(spec) => {
            let Some(current) = layer_of(doc, id)?.text() else {
                return Err(OpError::new(
                    ErrorCode::Unsupported,
                    "テキストレイヤーだけが text を持ちます",
                    "Only text layers have `text`",
                ));
            };
            let font = font.ok_or_else(|| {
                OpError::new(
                    ErrorCode::Internal,
                    "テキストレイヤーのフォントを探していません",
                    "The font of the text layer was not looked up",
                )
            })?;
            Some((patched(current, spec, font)?, font))
        }
    };
    if let Some(name) = &args.name {
        check_layer_name(name)?;
    }
    let blend = args.blend_mode.as_deref().map(parse_blend).transpose()?;
    let mut channels = Vec::new();
    for (name, on) in &args.channels {
        channels.push((resolve_channel(doc, name)?, *on));
    }
    let mut fills = Vec::new();
    for (name, color) in &args.fill {
        fills.push((
            resolve_channel(doc, name)?,
            color.as_deref().map(color_of).transpose()?,
        ));
    }
    let mut point_gradients = Vec::new();
    for (name, spec) in &args.points {
        point_gradients.push((
            resolve_channel(doc, name)?,
            spec.as_ref().map(points_of).transpose()?,
        ));
    }
    let adjustment = match &args.adjustment {
        None => None,
        Some(spec) => {
            let layer = layer_of(doc, id)?;
            let Some(current) = layer.adjustment() else {
                return Err(OpError::new(
                    ErrorCode::Unsupported,
                    "調整のレイヤーだけが adjustment を持ちます",
                    "Only adjustment layers have an adjustment",
                ));
            };
            let values: BTreeMap<String, ParamValue> = spec
                .values
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect();
            let new = if current.kind_id() == spec.kind {
                current.with_catalog_values(&values)?
            } else {
                AdjustmentSettings::from_catalog(&spec.kind, &values)?
            };
            // 有効なチャンネル（`channels` の変更のあと）のどれかに使えない調整は、どのチャンネルかを言って断る
            let refused: Vec<String> = doc
                .channels()
                .into_iter()
                .filter(|c| {
                    let patched = channels.iter().find(|(ch, _)| ch == c).map(|(_, on)| *on);
                    patched.unwrap_or_else(|| layer.is_channel_enabled(*c))
                })
                .filter(|c| {
                    doc.channel_info(*c)
                        .is_some_and(|i| !new.applies_to(i.kind))
                })
                .map(|c| channel_name(doc, c))
                .collect();
            if !refused.is_empty() {
                return Err(OpError::new(
                    ErrorCode::Unsupported,
                    format!(
                        "この調整は、有効なチャンネル（{}）には使えません。channels で無効にしてください",
                        refused.join("・")
                    ),
                    format!(
                        "This adjustment cannot be used on the enabled channel(s) {}; disable them with `channels`",
                        refused.join(", ")
                    ),
                )
                .with_data(json!({"channels": refused})));
            }
            Some(new)
        }
    };
    let locks = match &args.locks {
        None => None,
        Some(patch) => {
            let mut bits = layer_of(doc, id)?.locks().bits();
            for (flag, change) in [
                (LayerLocks::TRANSPARENCY, patch.transparency),
                (LayerLocks::PIXELS, patch.pixels),
                (LayerLocks::POSITION, patch.position),
                (LayerLocks::ALL, patch.all),
            ] {
                match change {
                    Some(true) => bits |= flag.bits(),
                    Some(false) => bits &= !flag.bits(),
                    None => {}
                }
            }
            Some(LayerLocks::from_bits(bits).map_err(|e| OpError::from_core(&e))?)
        }
    };
    batch(doc, |d| {
        if let Some(name) = &args.name {
            d.set_layer_name(id, name)?;
        }
        if let Some(v) = args.visible {
            d.set_layer_visible(id, v)?;
        }
        if let Some(v) = args.opacity {
            d.set_layer_opacity(id, v, false)?;
        }
        if let Some(mode) = blend {
            d.set_layer_blend_mode(id, mode)?;
        }
        if let Some(v) = args.clipping {
            d.set_layer_clipping(id, v)?;
        }
        if let Some(locks) = locks {
            d.set_layer_locks(id, locks)?;
        }
        // 調整レイヤーは、「新しい調整が有効なチャンネルに使えること」と「有効にするチャンネルに今の調整が使えること」を核が段ごとに見る。
        // 無効にする → 調整を替える → 有効にする、の順なら、どの途中の状態も両方を満たす（事前の検査が、最後の状態を通している）
        for (channel, _) in channels.iter().filter(|(_, on)| !*on) {
            d.set_channel_enabled(id, *channel, false)?;
        }
        if let Some(settings) = adjustment {
            d.set_adjustment(id, settings, false)?;
        }
        for (channel, _) in channels.iter().filter(|(_, on)| *on) {
            d.set_channel_enabled(id, *channel, true)?;
        }
        for (channel, value) in &fills {
            d.set_fill_value(id, *channel, *value, false)?;
        }
        for (channel, g) in &point_gradients {
            d.set_fill_points(id, *channel, g.clone(), false)?;
        }
        if let Some((settings, font)) = &text {
            d.set_text(id, settings.clone(), &font.bytes, false)?;
        }
        Ok(())
    })?;
    let reply = edited(facts, doc, Some(id), None, before);
    // フォントを指定しなかったのに、見つけたのが覚えたフォントと中身の違うもの: 黙って入れ替えず、描き直したことを返事に残す
    Ok(match &text {
        Some((settings, font)) if font.different => with_note(
            reply,
            Text::new(
                format!(
                    "フォントが違います（{}。見つけたフォントで描き直しました）",
                    text_layer::font_name(&settings.font)
                ),
                format!(
                    "The font differs ({}; the text was redrawn with the font that was found)",
                    text_layer::font_name(&settings.font)
                ),
            ),
        ),
        _ => reply,
    })
}

fn mask_set(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &MaskSetArgs,
    before: usize,
) -> Result<Reply, OpError> {
    let id = resolve_layer(doc, &args.layer)?;
    if layer_of(doc, id)?.mask().is_none() {
        return Err(OpError::new(
            ErrorCode::Unsupported,
            "このレイヤーにはマスクがありません",
            "This layer has no mask",
        ));
    }
    batch(doc, |d| {
        if let Some(v) = args.enabled {
            d.set_layer_mask_enabled(id, v)?;
        }
        if let Some(v) = args.inverted {
            d.set_layer_mask_inverted(id, v)?;
        }
        if let Some(v) = args.density {
            d.set_layer_mask_density(id, v, false)?;
        }
        Ok(())
    })?;
    Ok(edited(facts, doc, Some(id), None, before))
}

// ───────── 効果 ─────────

fn param_map(values: &BTreeMap<String, Value>) -> BTreeMap<String, ParamValue> {
    values
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().into()))
        .collect()
}

fn channels_of(doc: &Document, names: &[String]) -> Result<Vec<Channel>, OpError> {
    names.iter().map(|n| resolve_channel(doc, n)).collect()
}

fn effect_add(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &EffectAddArgs,
    before: usize,
) -> Result<Reply, OpError> {
    let layer = resolve_layer(doc, &args.layer)?;
    let target = core_target(args.target);
    let settings = EffectSettings::from_catalog(&args.kind, &param_map(&args.values))?;
    let channels = channels_of(doc, &args.channels)?;
    if target == FilterTarget::Mask && !channels.is_empty() {
        return Err(OpError::invalid_request(
            "マスクの効果はチャンネルを持ちません（マスクは全チャンネルで共有）",
            "A mask effect has no channels (the mask is shared by all channels)",
        ));
    }
    let mut spec = FilterSpec::new(settings);
    if !channels.is_empty() {
        spec = spec.channels(&channels);
    }
    spec.index = args.index;
    if let Some(v) = args.strength {
        spec.strength = v;
    }
    if let Some(v) = args.enabled {
        spec.enabled = v;
    }
    let id = batch(doc, |d| d.add_filter(layer, target, spec))?;
    Ok(edited(facts, doc, Some(layer), Some(id), before))
}

fn effect_set(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &EffectSetArgs,
    before: usize,
) -> Result<Reply, OpError> {
    let layer = resolve_layer(doc, &args.layer)?;
    let (id, target, _) = find_effect(doc, layer, &args.effect)?;
    let current = doc
        .find_filter(id)
        .map(|(_, fe, _)| fe.settings().clone())
        .ok_or_else(|| OpError::not_found(Noun::Effect, &args.effect))?;
    let values = param_map(&args.values);
    let settings = match &args.kind {
        Some(kind) if kind != current.kind_id() => {
            Some(EffectSettings::from_catalog(kind, &values)?)
        }
        _ if !values.is_empty() => Some(current.with_catalog_values(&values)?),
        _ => None,
    }
    .filter(|s| *s != current);
    let channels = channels_of(doc, &args.channels)?;
    if target == FilterTarget::Mask && !channels.is_empty() {
        return Err(OpError::invalid_request(
            "マスクの効果はチャンネルを持ちません",
            "A mask effect has no channels",
        ));
    }
    // 新しい設定が、これから適用するチャンネルに使えるかを、文書を変える前に見る（核の断りは理由が短いので、どのチャンネルかをここで言う）
    let old_channels: Vec<Channel> = doc
        .find_filter(id)
        .map(|(_, fe, _)| fe.channels().to_vec())
        .unwrap_or_default();
    if let (Some(new), FilterTarget::Content) = (&settings, target) {
        let after = if channels.is_empty() {
            &old_channels
        } else {
            &channels
        };
        let refused: Vec<String> = after
            .iter()
            .filter(|c| doc.filter_refusal(layer, target, new, **c).is_err())
            .map(|c| channel_name(doc, *c))
            .collect();
        if !refused.is_empty() {
            return Err(OpError::invalid_value(
                format!(
                    "この設定は、適用するチャンネル（{}）には使えません。channels で適用するチャンネルを選び直してください",
                    refused.join("・")
                ),
                format!(
                    "These settings cannot be applied to the channel(s) {}; choose the channels with `channels`",
                    refused.join(", ")
                ),
            )
            .with_data(json!({"channels": refused})));
        }
    }
    // 設定と適用するチャンネルを同時に変えるとき、新しい設定が今のチャンネルを受け付けるなら設定が先、そうでなければチャンネルが先
    // （途中の状態も、設定とチャンネルが食い違わない順にする）
    let settings_first = match (&settings, channels.is_empty()) {
        (Some(new), false) => old_channels
            .iter()
            .all(|c| doc.filter_refusal(layer, target, new, *c).is_ok()),
        _ => true,
    };
    batch(doc, |d| {
        if settings_first {
            if let Some(s) = &settings {
                d.set_filter_settings(layer, id, s.clone(), false)?;
            }
        }
        if !channels.is_empty() {
            d.set_filter_channels(layer, id, &channels)?;
        }
        if !settings_first {
            if let Some(s) = &settings {
                d.set_filter_settings(layer, id, s.clone(), false)?;
            }
        }
        if let Some(v) = args.strength {
            d.set_filter_strength(layer, id, v, false)?;
        }
        if let Some(v) = args.enabled {
            d.set_filter_enabled(layer, id, v)?;
        }
        if let Some(index) = args.index {
            d.move_filter(layer, id, index)?;
        }
        Ok(())
    })?;
    Ok(edited(facts, doc, Some(layer), Some(id), before))
}

// ───────── 取り消し ─────────

fn step_history(
    facts: SetFacts<'_>,
    doc: &mut Document,
    args: &UndoArgs,
    undo: bool,
) -> Result<Reply, OpError> {
    let steps = args.steps.unwrap_or(1);
    if !(1..=100).contains(&steps) {
        return Err(OpError::invalid_value(
            "steps は 1〜100 です",
            "steps must be between 1 and 100",
        )
        .with_data(json!({"min": 1, "max": 100})));
    }
    let mut done = 0;
    for _ in 0..steps {
        let moved =
            if undo { doc.undo() } else { doc.redo() }.map_err(|e| OpError::from_core(&e))?;
        if !moved {
            break;
        }
        done += 1;
    }
    Ok(Reply::Undone(Undone {
        set: facts.id.to_owned(),
        steps: done,
        can_undo: doc.can_undo(),
        can_redo: doc.can_redo(),
    }))
}

// ───────── 効果の種類の一覧 ─────────

/// `effect.list_kinds` の返事。表（core）の種類・欄に、文（`describe`）を付ける。
pub fn kinds_info() -> KindsInfo {
    use yolu_core::effects::catalog::{kinds, ParamType};
    KindsInfo {
        kinds: kinds()
            .iter()
            .map(|k| {
                let (title, description) = crate::describe::kind_text(k.id)
                    .unwrap_or_else(|| (Text::new(k.id, k.id), Text::new("", "")));
                let mut used_as = Vec::new();
                if k.stack {
                    used_as.push(KindUse::EffectStack);
                }
                if k.adjustment {
                    used_as.push(KindUse::AdjustmentLayer);
                }
                KindInfo {
                    id: k.id.to_owned(),
                    title,
                    description,
                    used_as,
                    generator: k.generator,
                    needs_baked_maps: k.needs_maps,
                    rust_only: k.rust_only,
                    addable: k.addable,
                    params: k
                        .params
                        .iter()
                        .map(|p| {
                            let (kind, min, max, options) = match &p.ty {
                                ParamType::Integer { min, max } => (
                                    ParamKindName::Integer,
                                    Some(*min as f64),
                                    Some(*max as f64),
                                    vec![],
                                ),
                                ParamType::Number { min, max } => {
                                    (ParamKindName::Number, Some(*min), Some(*max), vec![])
                                }
                                ParamType::Bool => (ParamKindName::Boolean, None, None, vec![]),
                                ParamType::Choice { options } => (
                                    ParamKindName::Choice,
                                    None,
                                    None,
                                    options.iter().map(|o| (*o).to_owned()).collect(),
                                ),
                            };
                            ParamInfo {
                                name: p.name.to_owned(),
                                kind,
                                min,
                                max,
                                options,
                                default: p.default.clone().into(),
                                description: crate::describe::param_text(k.id, p.name)
                                    .unwrap_or_else(|| Text::new("", "")),
                            }
                        })
                        .collect(),
                    opaque: k.opaque.iter().map(|s| (*s).to_owned()).collect(),
                }
            })
            .collect(),
    }
}
