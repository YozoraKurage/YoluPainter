//! 返事（`Reply`）と、その中身の型。JSON は `{"reply": "layer", ...中身の欄}`。
//!
//! 読む命令は中身を返し、編集の命令は `Edited`（新しいレイヤー・効果の ID と、取り消しの状態）を返す。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::text::Text;
use crate::value::{Bytes, Value};

/// 返事。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Doc(DocInfo),
    Set(SetInfo),
    Layer(LayerInfo),
    Effects(EffectsInfo),
    Kinds(KindsInfo),
    History(HistoryInfo),
    Preview(PreviewInfo),
    Edited(Edited),
    Undone(Undone),
    Exported(Exported),
    Saved(Saved),
    Action(ActionDone),
}

impl Reply {
    /// 返事の種類の名前（JSON の `reply`）。
    pub fn name(&self) -> &'static str {
        match self {
            Reply::Doc(_) => "doc",
            Reply::Set(_) => "set",
            Reply::Layer(_) => "layer",
            Reply::Effects(_) => "effects",
            Reply::Kinds(_) => "kinds",
            Reply::History(_) => "history",
            Reply::Preview(_) => "preview",
            Reply::Edited(_) => "edited",
            Reply::Undone(_) => "undone",
            Reply::Exported(_) => "exported",
            Reply::Saved(_) => "saved",
            Reply::Action(_) => "action",
        }
    }
}

// ───────── 文書・セット ─────────

/// Whether a texture set can be edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetState {
    Editable,
    /// It holds content the editor cannot handle yet; it is kept byte for byte and cannot be edited here.
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetSummary {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub layer_count: u32,
    pub state: SetState,
    /// Why the set is read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    /// Edited since it was opened or saved.
    pub unsaved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WriterInfo {
    pub app: String,
    pub version: String,
    pub platform: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DocInfo {
    /// Path of the opened file.
    pub path: String,
    /// .ylp format number of the file.
    pub format: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_by: Option<WriterInfo>,
    pub sets: Vec<SetSummary>,
    /// Id of the current set (the one used when a command omits `set`).
    pub current_set: String,
    /// Any set has unsaved changes.
    pub unsaved: bool,
    /// Things to know about the file (migrated format, kept unknown entries ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelValueKind {
    Color,
    Scalar,
    Normal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelInfo {
    /// Channel number (0-5 are Color, Roughness, Metallic, Height, Normal, Emission).
    pub index: u8,
    pub name: String,
    pub kind: ChannelValueKind,
    pub color_space: ChannelSpace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerKindName {
    Paint,
    Fill,
    Adjustment,
    Group,
    /// A paint layer whose Color pixels are drawn from editable text.
    Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayerSummary {
    pub id: String,
    pub name: String,
    pub kind: LayerKindName,
    pub visible: bool,
    pub opacity: f64,
    pub blend_mode: String,
    pub clipping: bool,
    /// Id of the group it is in. Absent at the top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Nesting depth (0 at the top level).
    pub depth: u32,
    pub has_mask: bool,
    /// Number of effects in the content stack.
    pub effect_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub state: SetState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    pub channels: Vec<ChannelInfo>,
    /// Layers from the top of the stack to the bottom; a group comes before its contents. Empty for a read-only set.
    pub layers: Vec<LayerSummary>,
    /// Effects that currently pass their input through (missing map, model, anchor ...). They are kept in the file but are not in the composite.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inactive_effects: Vec<Text>,
    pub unsaved: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LockState {
    pub transparency: bool,
    pub pixels: bool,
    pub position: bool,
    pub all: bool,
}

/// What one channel of a layer holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayerChannel {
    pub channel: String,
    pub enabled: bool,
    /// Fill layers: the value ("#rrggbb" or "#rrggbbaa").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Blend mode used only in this channel (absent: the layer's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    /// Opacity used only in this channel (absent: the layer's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Fill layers: the point gradient of this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<crate::command::PointGradientSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaskInfo {
    pub enabled: bool,
    pub inverted: bool,
    pub density: f64,
    /// Effects of the mask stack.
    pub effects: Vec<EffectInfo>,
}

/// An effect kind with its parameter values (an adjustment layer's setting uses the same shape).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffectValues {
    pub kind: String,
    pub values: BTreeMap<String, Value>,
    /// Parts of the effect that values cannot show or change (they are kept as they are).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opaque: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffectInfo {
    pub id: String,
    pub kind: String,
    pub values: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opaque: Vec<String>,
    pub strength: f64,
    pub enabled: bool,
    /// Content stack: channels the effect applies to. Empty for a mask effect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// Which stack the effect is in.
    pub target: crate::command::EffectTarget,
    /// Position in its stack (0 is applied first).
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayerInfo {
    #[serde(flatten)]
    pub summary: LayerSummary,
    pub locks: LockState,
    pub channels: Vec<LayerChannel>,
    /// Adjustment layers: the adjustment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<EffectValues>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<MaskInfo>,
    /// Content stack, in the order they are applied.
    pub effects: Vec<EffectInfo>,
    /// Text layers: the text values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Box<TextInfo>>,
}

/// Values of a text layer (see `TextSpec` for their meaning).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextInfo {
    pub content: String,
    /// Bundled font name, when the layer uses a bundled font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font file path, when the layer uses a font file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_index: Option<u32>,
    /// Family name of the font file, when the layer uses a font file and the font names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// PostScript name of the font file, when the layer uses a font file and the font names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_postscript: Option<String>,
    pub size: f64,
    pub color: String,
    pub line_height: f64,
    pub letter_spacing: f64,
    pub align: crate::command::TextAlignName,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub wrap_width: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffectsInfo {
    pub layer: String,
    pub effects: Vec<EffectInfo>,
}

// ───────── 効果の種類 ─────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamKindName {
    Integer,
    Number,
    Boolean,
    Choice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParamInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ParamKindName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Choices of a `choice` parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    pub default: Value,
    pub description: Text,
}

/// Where an effect kind can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KindUse {
    /// A step of a layer's (or mask's) effect stack (effect.add).
    EffectStack,
    /// The setting of an adjustment layer (layer.add with kind adjustment).
    AdjustmentLayer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KindInfo {
    pub id: String,
    pub title: Text,
    pub description: Text,
    pub used_as: Vec<KindUse>,
    /// A generator (a step that makes values from the model's baked maps or from position/UV).
    pub generator: bool,
    /// Reads baked mesh maps or the model. A headless host has neither, so such a generator is saved but passes its input through (it is not in the composite, the preview or exports). Procedural noise and grunge do not need them: without a position map they are evaluated in UV space.
    pub needs_baked_maps: bool,
    /// Only the standalone application has it. A document that uses it is saved in a newer document version, which the Unity version up to 0.4.x cannot open (it refuses the file with a reason; nothing is lost).
    pub rust_only: bool,
    /// Can be created from values. Kinds that are not addable can still be read, and their strength, enabled flag and channels can be changed.
    pub addable: bool,
    pub params: Vec<ParamInfo>,
    /// Parts that values cannot create or change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opaque: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KindsInfo {
    pub kinds: Vec<KindInfo>,
}

// ───────── 取り消し・見本 ─────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryInfo {
    pub undo_count: u32,
    pub redo_count: u32,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewInfo {
    pub set: String,
    pub channel: String,
    pub width: u32,
    pub height: u32,
    /// Size of the texture set (the image is smaller when it was shrunk).
    pub source_width: u32,
    pub source_height: u32,
    /// PNG, top row first.
    pub png: Bytes,
    /// Effects that pass their input through, so the image does not show them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inactive_effects: Vec<Text>,
}

/// Result of an editing command: ids it created and the undo state afterwards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edited {
    pub set: String,
    /// The layer that was added or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// The effect that was added or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// The command changed nothing (the values were already as asked).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unchanged: bool,
    pub undo_count: u32,
    pub can_undo: bool,
    /// Things to know about the edit (a text layer redrawn with a different font than the one it remembered ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Undone {
    pub set: String,
    /// How many commands were undone (or redone). Fewer than asked when the history ran out.
    pub steps: u32,
    pub can_undo: bool,
    pub can_redo: bool,
}

// ───────── 書き出し・保存 ─────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportedFile {
    pub path: String,
    pub width: u32,
    pub height: u32,
    /// An existing file was replaced.
    pub replaced: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Exported {
    pub files: Vec<ExportedFile>,
    /// Images of the template that were not written because nothing in the set feeds them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// Things to know (baked or rounded layers of a PSD, inactive effects ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Saved {
    pub path: String,
    /// False when there was nothing to write (no set was edited).
    pub written: bool,
    /// .ylp format of the file now.
    pub format: i32,
    /// The format the file had before it was moved to the current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgraded_from: Option<i32>,
    /// Ids of the sets that were rewritten. The others are copied byte for byte.
    pub sets_written: Vec<String>,
    /// The previous version of a replaced file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
}

// ───────── アクション ─────────

/// What one command of an action did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionStep {
    /// The layer the command added or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// The effect the command added or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// The command changed nothing (the values were already as asked).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unchanged: bool,
    /// Things to know about this command's edit (the same notes as the command's own reply).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
}

/// Every command of the action was applied, as one undo step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionDone {
    /// The texture set that was changed.
    pub set: String,
    /// One entry per command, in the same order.
    pub steps: Vec<ActionStep>,
    /// Nothing changed (no undo step was added).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unchanged: bool,
    pub undo_count: u32,
    pub can_undo: bool,
}
