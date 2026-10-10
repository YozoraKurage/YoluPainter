//! 命令の一覧（名前・説明・読むだけか壊すか・引数と返事のスキーマ）。CLI の説明と MCP のツールはここから作る。
//!
//! スキーマは型（`schemars`）から作る。手で書いた表は持たない（型と食い違わない）。名前は `Command::name` と同じ綴り。

use std::sync::OnceLock;

use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::*;
use crate::error::OpError;
use crate::reply::*;
use crate::text::Text;

/// 壊す操作の印。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Danger {
    /// 壊さない。
    Safe,
    /// いつも壊す（削除・上書き保存）。`confirm: true` が無ければ断る。
    Always,
    /// ファイルを置き換える・開いている文書の変更を捨てるときだけ壊す。そうなるときに `confirm: true` が無ければ断る。
    WhenReplacing,
    /// 列の中の命令による（`action.run`）。壊す命令は、その命令に `confirm: true` が無ければ、列の全部を当てる前に断る。
    PerCommand,
}

/// 命令 1 つの説明。
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub name: &'static str,
    pub title: Text,
    pub description: Text,
    /// 何も変えない（文書にもファイルにも）。
    pub read_only: bool,
    pub danger: Danger,
    /// 同じ引数でもう一度呼んでも、結果が変わらない。
    pub idempotent: bool,
    /// 返事の種類（`Reply::name`）。
    pub reply: &'static str,
    args: fn() -> Value,
    payload: fn() -> Value,
}

impl CommandSpec {
    /// 引数のスキーマ（JSON Schema 2020-12。MCP の inputSchema）。
    pub fn args_schema(&self) -> Value {
        (self.args)()
    }
    /// 返事の中身のスキーマ（MCP の outputSchema）。
    pub fn reply_schema(&self) -> Value {
        (self.payload)()
    }
    /// 破壊的か（MCP の destructiveHint）。
    pub fn destructive(&self) -> bool {
        self.danger != Danger::Safe
    }
    /// ツールの名前。AI の API は、ツールの名前に使える文字を英数字・`_`・`-` に絞ることがあるので、命令の名前の `.` を `_` にする
    /// （命令の名前と 1 対 1。[`command_spec_by_tool`] で戻せる）。
    pub fn tool_name(&self) -> String {
        self.name.replace('.', "_")
    }
}

/// ツールの名前（[`CommandSpec::tool_name`]）から命令を引く。
pub fn command_spec_by_tool(tool: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.tool_name() == tool)
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("スキーマは JSON にできる")
}

#[allow(clippy::too_many_arguments)]
fn spec<A: JsonSchema, R: JsonSchema>(
    name: &'static str,
    reply: &'static str,
    read_only: bool,
    danger: Danger,
    idempotent: bool,
    ja: (&str, &str),
    en: (&str, &str),
) -> CommandSpec {
    CommandSpec {
        name,
        title: Text::new(ja.0, en.0),
        description: Text::new(ja.1, en.1),
        read_only,
        danger,
        idempotent,
        reply,
        args: schema_of::<A>,
        payload: schema_of::<R>,
    }
}

/// 命令の一覧（`Command` の変種と同じ並び）。
pub fn commands() -> &'static [CommandSpec] {
    static SPECS: OnceLock<Vec<CommandSpec>> = OnceLock::new();
    SPECS.get_or_init(build)
}

/// 名前から。
pub fn command_spec(name: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.name == name)
}

fn build() -> Vec<CommandSpec> {
    use Danger::*;
    vec![
        spec::<DocInfoArgs, DocInfo>(
            "doc.info", "doc", true, Safe, true,
            ("プロジェクトの情報", "開いている .ylp の情報（ファイル・形式・テクスチャセットと今のセット）を返す。"),
            ("Document info", "Describe the opened .ylp: file, format, texture sets and the current set."),
        ),
        spec::<DocOpenArgs, DocInfo>(
            "doc.open", "doc", false, WhenReplacing, false,
            ("プロジェクトを開く", "指定した .ylp を開く。開いていたプロジェクトに保存していない変更があれば、confirm: true が要る（変更は捨てる）。ディスクのファイルには触らない。"),
            ("Open a document", "Open a .ylp file. The document that was open is closed; with unsaved changes this needs confirm: true (they are discarded). The file on disk is not touched."),
        ),
        spec::<SetInfoArgs, SetInfo>(
            "set.info", "set", true, Safe, true,
            ("セットの情報", "テクスチャセットの大きさ・チャンネル・レイヤーの並び（上から下）を返す。"),
            ("Texture set info", "Describe a texture set: size, channels and the layer stack from top to bottom."),
        ),
        spec::<LayerGetArgs, LayerInfo>(
            "layer.get", "layer", true, Safe, true,
            ("レイヤーを読む", "1 枚のレイヤーの全部（属性・チャンネルごとの状態・マスク・効果）を返す。"),
            ("Read a layer", "Everything about one layer: attributes, per-channel state, mask and effects."),
        ),
        spec::<LayerAddArgs, Edited>(
            "layer.add", "edited", false, Safe, false,
            ("レイヤーを追加", "ペイント・塗りつぶし・グループ・調整・テキストレイヤーを追加する。1 回の取り消しで戻る。"),
            ("Add a layer", "Add a paint, fill, group, adjustment or text layer. One undo step."),
        ),
        spec::<LayerDeleteArgs, Edited>(
            "layer.delete", "edited", false, Always, true,
            ("レイヤーを消す", "レイヤー（グループは中身ごと）を消す。confirm: true が要る。取り消しはこのセッションの中だけ。"),
            ("Delete a layer", "Delete a layer (a group with its contents). Needs confirm: true. Undo works only inside this session."),
        ),
        spec::<LayerMoveArgs, Edited>(
            "layer.move", "edited", false, Safe, true,
            ("レイヤーを動かす", "レイヤーを別の位置・別のグループへ動かす。1 回の取り消しで戻る。"),
            ("Move a layer", "Move a layer to another position or group. One undo step."),
        ),
        spec::<LayerSetArgs, Edited>(
            "layer.set", "edited", false, Safe, true,
            ("レイヤーを変える", "名前・表示・不透明度・合成モード・クリッピング・ロック・チャンネルの有効・塗りつぶしの値・調整の値・文字の値を変える。まとめて 1 回の取り消しで戻る。"),
            ("Change a layer", "Change name, visibility, opacity, blend mode, clipping, locks, enabled channels, fill values, adjustment values or text values. All of it is one undo step."),
        ),
        spec::<MaskAddArgs, Edited>(
            "mask.add", "edited", false, Safe, false,
            ("マスクを追加", "何も隠さないラスターマスクを追加する。1 回の取り消しで戻る。"),
            ("Add a mask", "Add a raster mask that hides nothing. One undo step."),
        ),
        spec::<MaskDeleteArgs, Edited>(
            "mask.delete", "edited", false, Always, true,
            ("マスクを消す", "レイヤーのマスクを外す。confirm: true が要る。"),
            ("Delete a mask", "Remove the layer's mask. Needs confirm: true."),
        ),
        spec::<MaskSetArgs, Edited>(
            "mask.set", "edited", false, Safe, true,
            ("マスクを変える", "マスクの有効・反転・濃度を変える。まとめて 1 回の取り消しで戻る。"),
            ("Change a mask", "Change the mask's enabled flag, inversion and density. One undo step."),
        ),
        spec::<EffectGetArgs, EffectsInfo>(
            "effect.get", "effects", true, Safe, true,
            ("効果を読む", "レイヤーの効果（内容とマスクのスタック）を返す。effect を指すと 1 つだけ。"),
            ("Read effects", "The effects of a layer (content and mask stacks), or just one when `effect` is given."),
        ),
        spec::<EffectAddArgs, Edited>(
            "effect.add", "edited", false, Safe, false,
            ("効果を追加", "フィルター・Generator を種類の名前と値で追加する（種類は effect.list_kinds）。1 回の取り消しで戻る。"),
            ("Add an effect", "Add a filter or generator by kind and parameter values (see effect.list_kinds). One undo step."),
        ),
        spec::<EffectSetArgs, Edited>(
            "effect.set", "edited", false, Safe, true,
            ("効果を変える", "効果の値・強さ・有効・チャンネル・位置を変える。まとめて 1 回の取り消しで戻る。"),
            ("Change an effect", "Change an effect's values, strength, enabled flag, channels or position. All of it is one undo step."),
        ),
        spec::<EffectDeleteArgs, Edited>(
            "effect.delete", "edited", false, Always, true,
            ("効果を消す", "効果を外す。confirm: true が要る。"),
            ("Delete an effect", "Remove an effect from its stack. Needs confirm: true."),
        ),
        spec::<EffectListKindsArgs, KindsInfo>(
            "effect.list_kinds", "kinds", true, Safe, true,
            ("効果の種類の一覧", "効果の種類と、欄の名前・型・範囲・既定を返す。"),
            ("List effect kinds", "Effect kinds with the name, type, range and default of every parameter."),
        ),
        spec::<HistoryInfoArgs, HistoryInfo>(
            "history.info", "history", true, Safe, true,
            ("取り消しの状態", "取り消せる数・やり直せる数を返す（このセッションの中だけ）。"),
            ("History info", "How many commands can be undone or redone (this session only)."),
        ),
        spec::<UndoArgs, Undone>(
            "undo", "undone", false, Safe, false,
            ("取り消す", "直前の命令（steps 個）を取り消す。"),
            ("Undo", "Undo the last command (or `steps` commands)."),
        ),
        spec::<UndoArgs, Undone>(
            "redo", "undone", false, Safe, false,
            ("やり直す", "取り消した命令（steps 個）をやり直す。"),
            ("Redo", "Redo the undone command (or `steps` commands)."),
        ),
        spec::<PreviewArgs, PreviewInfo>(
            "preview", "preview", true, Safe, true,
            ("見本の画像", "チャンネルの合成の PNG（長い辺が max_edge に収まるよう箱の平均で縮める）を返す。"),
            ("Preview image", "A PNG of a channel's composite (shrunk by a box average so the longest side fits max_edge)."),
        ),
        spec::<ExportChannelsArgs, Exported>(
            "export.channels", "exported", false, WhenReplacing, true,
            ("チャンネルの書き出し", "チャンネルの合成を PNG にしてフォルダへ書く。既にあるファイルを置き換えるときは confirm: true が要る。"),
            ("Export channels", "Write channel composites as PNG files into a folder. Replacing existing files needs confirm: true."),
        ),
        spec::<ExportTexturesArgs, Exported>(
            "export.textures", "exported", false, WhenReplacing, true,
            ("テクスチャの書き出し", "テンプレート（Unity Standard・HDRP・lilToon）の画像を PNG にしてフォルダへ書く。既にあるファイルを置き換えるときは confirm: true が要る。"),
            ("Export textures", "Write the images of an export template (Unity Standard, HDRP, lilToon) as PNG files. Replacing existing files needs confirm: true."),
        ),
        spec::<ExportPsdArgs, Exported>(
            "export.psd", "exported", false, WhenReplacing, true,
            ("PSD の書き出し", "1 つのチャンネルを PSD に書く。既にあるファイルを置き換えるときは confirm: true が要る。"),
            ("Export PSD", "Write one channel as a PSD. Replacing an existing file needs confirm: true."),
        ),
        spec::<SaveArgs, Saved>(
            "save", "saved", false, Always, true,
            ("保存", "開いている .ylp に上書きで保存する（前の版は隣の退避のフォルダに残る）。confirm: true が要る。"),
            ("Save", "Save over the opened .ylp (the previous version is kept in the backups folder next to it). Needs confirm: true."),
        ),
        spec::<SaveAsArgs, Saved>(
            "save_as", "saved", false, WhenReplacing, true,
            ("名前を付けて保存", "別の .ylp として保存する。既にあるファイルへは confirm: true が要る。"),
            ("Save as", "Save as another .ylp. Replacing an existing file needs confirm: true."),
        ),
        spec::<ActionRunArgs, ActionDone>(
            "action.run", "action", false, PerCommand, false,
            ("アクションを実行", "レイヤー・マスク・効果を変える命令の列を、1 つのテクスチャセットへ取り消しの 1 段で当てる。途中の命令が断れば全部を戻し、何番目か（data.index、0 から）を返す。中の壊す命令には、それぞれ confirm: true が要る。"),
            ("Run an action", "Apply a list of commands that change layers, masks and effects to one texture set as one undo step. If a command fails, everything is rolled back and the error tells which one (data.index, from 0). Destructive commands in the list need their own confirm: true."),
        ),
    ]
}

/// `Command` 全体の JSON Schema。
pub fn command_schema() -> Value {
    schema_of::<Command>()
}
/// `Reply` 全体の JSON Schema。
pub fn reply_schema() -> Value {
    schema_of::<Reply>()
}
/// `OpError` の JSON Schema。
pub fn error_schema() -> Value {
    schema_of::<OpError>()
}

impl Reply {
    /// 返事の中身だけの JSON（`reply` の印を除く。`CommandSpec::reply_schema` の形）。
    pub fn payload(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("返事は JSON にできる");
        if let Value::Object(map) = &mut value {
            map.remove("reply");
        }
        value
    }
}
