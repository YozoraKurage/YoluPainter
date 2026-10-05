//! 命令（`Command`）と、その引数の型。JSON は `{"command": "layer.set", "args": {...}}`（命令の名前と引数）。
//!
//! - 引数の知らない欄は断る（`deny_unknown_fields`。綴りの間違いを黙って無視しない）。省ける欄は `Option` か既定値。
//! - セットは ID か名前で指す（`set`。省くと今のセット）。層は ID か名前（`layer`。名前が複数に当たれば断る）。チャンネルは名前（`Color` など。
//!   大文字小文字は問わない）かチャンネルの番号。
//! - 壊す操作（削除・上書き保存・PSD の書き戻し）は `confirm: true` が無ければ断る（[`crate::meta::Danger`]）。
//! - 欄の説明は英語（スキーマの `description` になり、MCP のクライアントの AI が読む）。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::value::Value;

/// 命令の版。引数の形を、古い命令が読めなくなる変え方をするときに上げる（欄を足すだけなら上げない）。
pub const COMMAND_VERSION: u32 = 1;

fn is_false(v: &bool) -> bool {
    !*v
}

/// 命令。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args")]
pub enum Command {
    #[serde(rename = "doc.info")]
    DocInfo(DocInfoArgs),
    #[serde(rename = "doc.open")]
    DocOpen(DocOpenArgs),
    #[serde(rename = "set.info")]
    SetInfo(SetInfoArgs),
    #[serde(rename = "layer.get")]
    LayerGet(LayerGetArgs),
    #[serde(rename = "layer.add")]
    LayerAdd(LayerAddArgs),
    #[serde(rename = "layer.delete")]
    LayerDelete(LayerDeleteArgs),
    #[serde(rename = "layer.move")]
    LayerMove(LayerMoveArgs),
    #[serde(rename = "layer.set")]
    LayerSet(LayerSetArgs),
    #[serde(rename = "mask.add")]
    MaskAdd(MaskAddArgs),
    #[serde(rename = "mask.delete")]
    MaskDelete(MaskDeleteArgs),
    #[serde(rename = "mask.set")]
    MaskSet(MaskSetArgs),
    #[serde(rename = "effect.get")]
    EffectGet(EffectGetArgs),
    #[serde(rename = "effect.add")]
    EffectAdd(EffectAddArgs),
    #[serde(rename = "effect.set")]
    EffectSet(EffectSetArgs),
    #[serde(rename = "effect.delete")]
    EffectDelete(EffectDeleteArgs),
    #[serde(rename = "effect.list_kinds")]
    EffectListKinds(EffectListKindsArgs),
    #[serde(rename = "history.info")]
    HistoryInfo(HistoryInfoArgs),
    #[serde(rename = "undo")]
    Undo(UndoArgs),
    #[serde(rename = "redo")]
    Redo(UndoArgs),
    #[serde(rename = "preview")]
    Preview(PreviewArgs),
    #[serde(rename = "export.channels")]
    ExportChannels(ExportChannelsArgs),
    #[serde(rename = "export.textures")]
    ExportTextures(ExportTexturesArgs),
    #[serde(rename = "export.psd")]
    ExportPsd(ExportPsdArgs),
    #[serde(rename = "save")]
    Save(SaveArgs),
    #[serde(rename = "save_as")]
    SaveAs(SaveAsArgs),
}

impl Command {
    /// 命令の名前（JSON の `command`）。
    pub fn name(&self) -> &'static str {
        match self {
            Command::DocInfo(_) => "doc.info",
            Command::DocOpen(_) => "doc.open",
            Command::SetInfo(_) => "set.info",
            Command::LayerGet(_) => "layer.get",
            Command::LayerAdd(_) => "layer.add",
            Command::LayerDelete(_) => "layer.delete",
            Command::LayerMove(_) => "layer.move",
            Command::LayerSet(_) => "layer.set",
            Command::MaskAdd(_) => "mask.add",
            Command::MaskDelete(_) => "mask.delete",
            Command::MaskSet(_) => "mask.set",
            Command::EffectGet(_) => "effect.get",
            Command::EffectAdd(_) => "effect.add",
            Command::EffectSet(_) => "effect.set",
            Command::EffectDelete(_) => "effect.delete",
            Command::EffectListKinds(_) => "effect.list_kinds",
            Command::HistoryInfo(_) => "history.info",
            Command::Undo(_) => "undo",
            Command::Redo(_) => "redo",
            Command::Preview(_) => "preview",
            Command::ExportChannels(_) => "export.channels",
            Command::ExportTextures(_) => "export.textures",
            Command::ExportPsd(_) => "export.psd",
            Command::Save(_) => "save",
            Command::SaveAs(_) => "save_as",
        }
    }

    /// 対象のセットの指定（セットを持たない命令は None）。
    pub fn set(&self) -> Option<&str> {
        match self {
            Command::SetInfo(a) => a.set.as_deref(),
            Command::LayerGet(a) => a.set.as_deref(),
            Command::LayerAdd(a) => a.set.as_deref(),
            Command::LayerDelete(a) => a.set.as_deref(),
            Command::LayerMove(a) => a.set.as_deref(),
            Command::LayerSet(a) => a.set.as_deref(),
            Command::MaskAdd(a) => a.set.as_deref(),
            Command::MaskDelete(a) => a.set.as_deref(),
            Command::MaskSet(a) => a.set.as_deref(),
            Command::EffectGet(a) => a.set.as_deref(),
            Command::EffectAdd(a) => a.set.as_deref(),
            Command::EffectSet(a) => a.set.as_deref(),
            Command::EffectDelete(a) => a.set.as_deref(),
            Command::HistoryInfo(a) => a.set.as_deref(),
            Command::Undo(a) | Command::Redo(a) => a.set.as_deref(),
            Command::Preview(a) => a.set.as_deref(),
            Command::ExportChannels(a) => a.set.as_deref(),
            Command::ExportTextures(a) => a.set.as_deref(),
            Command::ExportPsd(a) => a.set.as_deref(),
            Command::DocInfo(_)
            | Command::DocOpen(_)
            | Command::EffectListKinds(_)
            | Command::Save(_)
            | Command::SaveAs(_) => None,
        }
    }

    /// 引数の `confirm`（壊す操作の確認。欄の無い命令は None）。
    pub fn confirmed(&self) -> Option<bool> {
        match self {
            Command::DocOpen(a) => Some(a.confirm),
            Command::LayerDelete(a) => Some(a.confirm),
            Command::MaskDelete(a) => Some(a.confirm),
            Command::EffectDelete(a) => Some(a.confirm),
            Command::ExportChannels(a) => Some(a.confirm),
            Command::ExportTextures(a) => Some(a.confirm),
            Command::ExportPsd(a) => Some(a.confirm),
            Command::Save(a) => Some(a.confirm),
            Command::SaveAs(a) => Some(a.confirm),
            _ => None,
        }
    }
}

// ───────── 読む ─────────

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocInfoArgs {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocOpenArgs {
    /// Path of the .ylp file. Absolute, or relative to the working folder (a relative path may not leave it with "..").
    pub path: String,
    /// Required when the open document has unsaved changes (they are discarded).
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInfoArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerGetArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id (32 hex digits) or exact name.
    pub layer: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectGetArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    /// Effect id. Omit to list every effect of the layer (content stack and mask stack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectListKindsArgs {}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryInfoArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Channel name (e.g. Color, Normal) or number. Default Color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Longest side of the image in pixels (1..=2048). Default 512. The image is shrunk by a box average and never enlarged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edge: Option<u32>,
}

// ───────── 層 ─────────

/// Kind of a new layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NewLayerKind {
    /// A raster layer with an empty Color channel.
    Paint,
    /// A layer filling the canvas with one value per channel (give `fill`).
    Fill,
    /// An empty group.
    Group,
    /// An adjustment layer (give `adjustment`).
    Adjustment,
}

/// An effect kind with parameter values (see effect.list_kinds). Omitted parameters take their defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectSpec {
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerAddArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    pub kind: NewLayerKind,
    /// Layer name. Default depends on the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Insert just above this layer (same group). Omit for the top of the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<String>,
    /// For a fill layer: channel name -> color ("#rrggbb" or "#rrggbbaa"). Each listed channel is enabled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fill: BTreeMap<String, String>,
    /// For an adjustment layer: the adjustment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<EffectSpec>,
    /// For an adjustment layer: channels it applies to. Default: every channel the adjustment can be used on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerDeleteArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name. A group is deleted with its contents.
    pub layer: String,
    /// Must be true: deleting is destructive (it can be undone only inside this session).
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerMoveArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    /// Move into this group (id or name). Omit to stay in the current group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Move out to the top level. Not together with `parent`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub to_root: bool,
    /// Position among the siblings counted from the bottom (0 = bottom). Omit for the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

/// Lock flags to change. Omitted flags stay as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocksPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerSetArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    /// 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Blend mode name such as Normal, Multiply, Screen, Overlay (PassThrough only for groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    /// Clip to the layer below (same group).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipping: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locks: Option<LocksPatch>,
    /// Enable or disable channels: channel name -> bool. Pixels and values of a disabled channel are kept.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, bool>,
    /// Fill layer values: channel name -> color, or null to remove the value of that channel.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fill: BTreeMap<String, Option<String>>,
    /// Adjustment layer: new values. With the same kind, the listed values are changed and the others stay; with another kind it is rebuilt from defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<EffectSpec>,
}

// ───────── マスク ─────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskAddArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name. The new mask hides nothing.
    pub layer: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskDeleteArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    /// Must be true: deleting the mask is destructive.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskSetArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverted: Option<bool>,
    /// 0..=1 (0 hides nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
}

// ───────── 効果 ─────────

/// Which effect stack of a layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectTarget {
    /// The layer's pixels (per channel).
    #[default]
    Content,
    /// The layer's mask (one scalar shared by all channels).
    Mask,
}

impl EffectTarget {
    pub fn is_default(&self) -> bool {
        *self == EffectTarget::Content
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectAddArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    #[serde(default, skip_serializing_if = "EffectTarget::is_default")]
    pub target: EffectTarget,
    /// Effect kind (see effect.list_kinds), e.g. blur, levels, edge_wear.
    pub kind: String,
    /// Parameter values by name. Omitted parameters take their defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
    /// Content stack only: channels the effect applies to. Default: every channel that accepts it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// 0..=1. How much of the effect's result is used. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    /// Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Position in the stack (0 is applied first). Omit for the end (applied last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectSetArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    /// Effect id (see effect.get).
    pub effect: String,
    /// Change the kind. The effect is rebuilt from the defaults of the new kind plus `values`. Omit to keep the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Parameter values to change. The others keep their current values (with a new `kind`, they take defaults).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
    /// Content stack only: channels the effect applies to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Move to this position in the stack (0 is applied first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectDeleteArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Layer id or exact name.
    pub layer: String,
    /// Effect id.
    pub effect: String,
    /// Must be true: deleting is destructive.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

// ───────── 取り消し ─────────

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UndoArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// How many commands to undo (or redo). Default 1, at most 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
}

// ───────── 書き出し・保存 ─────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportChannelsArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Channels to write, by name or number. Default: every channel some layer uses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// Folder to write into (created if missing). Absolute, or relative to the working folder.
    pub dir: String,
    /// File name stem. Default: the project file name without extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Required to replace files that already exist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportTexturesArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Export template id: unity-standard (default), unity-hdrp or liltoon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Folder to write into (created if missing). Absolute, or relative to the working folder.
    pub dir: String,
    /// File name stem. Default: the project file name without extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Required to replace files that already exist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

/// How a PSD is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PsdMode {
    /// Keep the layers; bake what a PSD cannot express (a plan lists them in the reply).
    #[default]
    Bake,
    /// One flattened layer of the composite.
    Flat,
}

impl PsdMode {
    pub fn is_default(&self) -> bool {
        *self == PsdMode::Bake
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportPsdArgs {
    /// Texture set id or name. Omit for the current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// Path of the .psd file to write. Absolute, or relative to the working folder.
    pub path: String,
    /// Channel to write. Default Color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "PsdMode::is_default")]
    pub mode: PsdMode,
    /// Required to replace a file that already exists.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveArgs {
    /// Must be true: saving replaces the opened .ylp file (the previous version is kept in the backups folder next to it).
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveAsArgs {
    /// Path of the .ylp file to write (the name must end with .ylp). Absolute, or relative to the working folder.
    pub path: String,
    /// Required when the file already exists (it must be a valid .ylp; the previous version is kept in the backups folder).
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirm: bool,
}
