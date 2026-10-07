//! `tools.json`（設定のフォルダの直下）の読み書き。形:
//!
//! ```json
//! {
//!   "format": "yolupainter-tools",
//!   "version": 1,
//!   "known_tools": ["brush", "eraser", "fill", "…"],
//!   "known_brushes": ["standard", "hard-round", "…"],
//!   "user_brushes_through": 12,
//!   "tools": [
//!     { "tool": "brush", "groups": [
//!         { "group": "pen", "brushes": ["b:standard", "u:3"] },
//!         { "name": "インク", "brushes": ["u:7"] } ] },
//!     { "tool": "eraser", "groups": [ { "group": "eraser", "brushes": ["b:standard-eraser"] } ] },
//!     { "tool": "fill" },
//!     { "tool": "select-rectangle", "gap": true },
//!     { "tool": "brush", "name": "水彩", "icon": "paint-brush", "groups": [ { "name": "グループ", "brushes": [] } ] }
//!   ]
//! }
//! ```
//!
//! - `tool` はツールの表の id。`name` が無ければツールの表の名前（言語で替わる）。`icon` は `ICONS` の名前。`gap` はこのツールの前の区切り。
//! - `groups` はブラシ・消しゴムのツールだけ。`group`（組み込みのグループの id）があって `name` が無ければ、組み込みの名前（言語で替わる）。
//! - `known_tools`・`known_brushes` は書いたときに知っていた組み込みのツール・ブラシ。読んだとき、ここに無い組み込み（新しい版で足された物）は、
//!   ツールは列の最後へ、ブラシはその組み込みのグループの最後へ足す。利用者が外した物は known にあるので戻らない。
//! - `user_brushes_through` は書いたときにあった利用者のブラシのファイルの番号の最大。並びに無いファイルのうち、これより大きい番号の物
//!   （古い版が足した物・並びを書く前に落ちた物）は、ファイルの元のグループ（`group=`）へ足す。これ以下の物は、利用者が並びから外した物。
//!
//! 読めない所（JSON でない・形が違う・上限を超える・同じブラシや同じツールが 2 か所・グループを持てないツールにグループ・ブラシのツールか
//! 消しゴムのツールが無い）は、ファイル全体を読めないとする（呼ぶ側が初めの並びに戻し、元のファイルを退避して知らせる）。版が新しい
//! ファイルは触らない。知らないツールの id は、その項目だけ飛ばす（書き直すまでファイルは変えない）。知らない組み込みのブラシと、
//! フォルダにあるのに読めなかったブラシのファイル（`u:<番号>`）の札は、画面には出さずにグループへ覚え（`BrushGroup::unread`）、
//! 書き直すとき元の場所へ書き戻す（読めるようになったとき、元のグループにある）。フォルダに無いファイルの札は覚えない。
//! 知らないキーは読み飛ばす（形を足すときは版を上げる）。

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use super::{
    holds_brushes, icon, BrushGroup, ToolSet, ToolSlot, Unread, MAX_GROUPS, MAX_GROUP_BRUSHES,
    MAX_TOOLS,
};
use crate::brushes::{builtin, BrushKey, Group, MAX_NAME_CHARS};
use crate::lang::Lang;
use crate::state::Tool;

pub const FORMAT: &str = "yolupainter-tools";
pub const VERSION: u64 = 1;
/// ファイルの名前（設定のフォルダの直下）。
pub const FILE_NAME: &str = "tools.json";
/// ファイルの大きさの上限。
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// 利用者のブラシのファイルの番号の、起動のときの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserFile {
    /// 読めた（ブラシの一覧にある）。
    Loaded,
    /// フォルダにあるが読めなかった（壊れた・新しい版・同期の途中・数の上限で読まなかった）。
    Unloaded,
    /// フォルダに無い。
    Missing,
}

/// 読めなかった理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Broken {
    NotJson,
    /// 形が違う（どこが）。
    Shape(&'static str),
    /// 上限を超える。
    TooMany,
    /// 同じブラシ・同じツールが 2 か所。
    Duplicate,
    /// ブラシのツールか消しゴムのツールが無い。
    MissingTool,
    TooLarge,
}

impl Broken {
    pub fn describe(&self, lang: Lang) -> String {
        match self {
            Broken::NotJson => lang.pick("JSON ではありません", "not JSON").into(),
            Broken::Shape(at) => lang.pick(
                format!("{at} の形が違います"),
                format!("unexpected shape at {at}"),
            ),
            Broken::TooMany => lang.pick("数が上限を超えています", "over the limit").into(),
            Broken::Duplicate => lang
                .pick("同じ項目が 2 か所にあります", "an item appears twice")
                .into(),
            Broken::MissingTool => lang
                .pick(
                    "ブラシか消しゴムのツールがありません",
                    "no brush or eraser tool",
                )
                .into(),
            Broken::TooLarge => lang.pick("大きすぎます", "too large").into(),
        }
    }
}

/// 読んだ結果。
#[derive(Debug)]
pub enum Decoded {
    Ok(Box<Read>),
    /// 今の版より新しい（触らない）。
    Newer(u64),
}

/// 読めた並び。
#[derive(Debug)]
pub struct Read {
    pub set: ToolSet,
    pub known_tools: HashSet<String>,
    pub known_brushes: HashSet<String>,
    pub user_through: u32,
    /// 飛ばした項目の数（知らないツール・組み込みのブラシ・読めなかったブラシのファイル）。
    pub skipped: usize,
}

/// 書く（今の版が知っている組み込みのツールとブラシを known に、`user_through` を番号の最大に）。
pub fn encode(set: &ToolSet, user_through: u32) -> String {
    let tools: Vec<Value> = set
        .slots()
        .iter()
        .map(|slot| {
            let mut o = Map::new();
            o.insert("tool".into(), json!(slot.tool.id()));
            if let Some(name) = &slot.name {
                o.insert("name".into(), json!(name));
            }
            if let Some(icon) = slot.icon {
                o.insert("icon".into(), json!(icon));
            }
            if slot.gap {
                o.insert("gap".into(), json!(true));
            }
            if slot.holds_brushes() {
                let groups: Vec<Value> = slot
                    .groups
                    .iter()
                    .map(|g| {
                        let mut go = Map::new();
                        if let Some(b) = g.builtin {
                            go.insert("group".into(), json!(b.id()));
                        }
                        if let Some(name) = &g.name {
                            go.insert("name".into(), json!(name));
                        }
                        go.insert("brushes".into(), json!(g.tokens()));
                        Value::Object(go)
                    })
                    .collect();
                o.insert("groups".into(), Value::Array(groups));
            }
            Value::Object(o)
        })
        .collect();
    let known_tools: Vec<&str> = Tool::ALL.iter().map(|t| t.id()).collect();
    let known_brushes: Vec<&str> = builtin::all().iter().map(|b| b.id).collect();
    let root = json!({
        "format": FORMAT,
        "version": VERSION,
        "known_tools": known_tools,
        "known_brushes": known_brushes,
        "user_brushes_through": user_through,
        "tools": tools,
    });
    let mut text = serde_json::to_string_pretty(&root).unwrap_or_default();
    text.push('\n');
    text
}

fn tool_from_id(id: &str) -> Option<Tool> {
    Tool::ALL.into_iter().find(|t| t.id() == id)
}

fn name_field(o: &Map<String, Value>, at: &'static str) -> Result<Option<String>, Broken> {
    match o.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            if s.chars().count() > MAX_NAME_CHARS * 4 {
                return Err(Broken::TooMany);
            }
            Ok(crate::brushes::clean_name(s))
        }
        Some(_) => Err(Broken::Shape(at)),
    }
}

fn string_list(v: Option<&Value>, at: &'static str, max: usize) -> Result<Vec<String>, Broken> {
    match v {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            if items.len() > max {
                return Err(Broken::TooMany);
            }
            items
                .iter()
                .map(|i| i.as_str().map(str::to_owned).ok_or(Broken::Shape(at)))
                .collect()
        }
        Some(_) => Err(Broken::Shape(at)),
    }
}

/// 読む。`user_file` は利用者のブラシのファイルの番号の状態。読めなかったファイルの札と、今の版が知らない組み込みのブラシの札は、
/// 出さずにグループへ覚えておく（書くときに元の場所へ戻す）。フォルダに無いファイルの札は覚えない（書き直すと消える）。
pub fn decode(text: &str, user_file: &dyn Fn(u32) -> UserFile) -> Result<Decoded, Broken> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(Broken::TooLarge);
    }
    let root: Value = serde_json::from_str(text).map_err(|_| Broken::NotJson)?;
    let root = root.as_object().ok_or(Broken::Shape("root"))?;
    if root.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(Broken::Shape("format"));
    }
    let version = root
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(Broken::Shape("version"))?;
    if version > VERSION {
        return Ok(Decoded::Newer(version));
    }
    if version == 0 {
        return Err(Broken::Shape("version"));
    }
    let known_tools: HashSet<String> = string_list(root.get("known_tools"), "known_tools", 4096)?
        .into_iter()
        .collect();
    let known_brushes: HashSet<String> =
        string_list(root.get("known_brushes"), "known_brushes", 4096)?
            .into_iter()
            .collect();
    let user_through = root
        .get("user_brushes_through")
        .and_then(Value::as_u64)
        .ok_or(Broken::Shape("user_brushes_through"))?
        .min(u32::MAX as u64) as u32;
    let tools = root
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(Broken::Shape("tools"))?;
    if tools.len() > MAX_TOOLS {
        return Err(Broken::TooMany);
    }
    let mut set = ToolSet::empty();
    let mut skipped = 0;
    let mut seen_tools: HashSet<Tool> = HashSet::new();
    let mut seen_brushes: HashSet<String> = HashSet::new();
    let mut seen_keys: HashSet<BrushKey> = HashSet::new();
    for t in tools {
        let o = t.as_object().ok_or(Broken::Shape("tools[]"))?;
        let id = o
            .get("tool")
            .and_then(Value::as_str)
            .ok_or(Broken::Shape("tools[].tool"))?;
        let gap = match o.get("gap") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(Broken::Shape("tools[].gap")),
        };
        let name = name_field(o, "tools[].name")?;
        let icon_key = match o.get("icon") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => icon(s).map(|i| i.key),
            Some(_) => return Err(Broken::Shape("tools[].icon")),
        };
        let groups = o.get("groups");
        let Some(tool) = tool_from_id(id) else {
            // 知らないツール（新しい版のツール）は飛ばす
            skipped += 1;
            continue;
        };
        if !holds_brushes(tool) {
            if groups.is_some() {
                return Err(Broken::Shape("tools[].groups"));
            }
            if !seen_tools.insert(tool) {
                return Err(Broken::Duplicate);
            }
        }
        let mut slot: ToolSlot = set.new_slot(tool);
        slot.gap = gap;
        slot.name = name;
        slot.icon = icon_key;
        if holds_brushes(tool) {
            let empty = Vec::new();
            let groups = match groups {
                None => &empty,
                Some(Value::Array(g)) => g,
                Some(_) => return Err(Broken::Shape("tools[].groups")),
            };
            if groups.len() > MAX_GROUPS {
                return Err(Broken::TooMany);
            }
            for g in groups {
                let go = g.as_object().ok_or(Broken::Shape("groups[]"))?;
                let builtin_group = match go.get("group") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Group::from_id(s),
                    Some(_) => return Err(Broken::Shape("groups[].group")),
                };
                let name = name_field(go, "groups[].name")?;
                let tokens = string_list(go.get("brushes"), "groups[].brushes", MAX_GROUP_BRUSHES)?;
                let mut brushes = Vec::with_capacity(tokens.len());
                let mut unread = Vec::new();
                for token in tokens {
                    if !seen_brushes.insert(token.clone()) {
                        return Err(Broken::Duplicate);
                    }
                    let key = parse_token(&token);
                    if key.is_some_and(|key| !seen_keys.insert(key)) {
                        return Err(Broken::Duplicate);
                    }
                    let keep = match key {
                        Some(key @ BrushKey::Builtin(_)) => {
                            brushes.push(key);
                            continue;
                        }
                        Some(BrushKey::User(n)) => match user_file(n) {
                            UserFile::Loaded => {
                                brushes.push(BrushKey::User(n));
                                continue;
                            }
                            UserFile::Unloaded => true,
                            UserFile::Missing => false,
                        },
                        None if is_token_shape(&token) => true,
                        None => return Err(Broken::Shape("groups[].brushes[]")),
                    };
                    skipped += 1;
                    if keep {
                        unread.push(Unread {
                            at: brushes.len(),
                            token,
                        });
                    }
                }
                let mut group: BrushGroup = set.new_group(builtin_group, name, brushes);
                group.unread = unread;
                slot.groups.push(group);
            }
        }
        set.push_slot(slot);
    }
    if set.first_of(Tool::Brush).is_none() || set.first_of(Tool::Eraser).is_none() {
        return Err(Broken::MissingTool);
    }
    set.activate_first(Tool::Brush);
    Ok(Decoded::Ok(Box::new(Read {
        set,
        known_tools,
        known_brushes,
        user_through,
        skipped,
    })))
}

/// 札（`b:<id>`・`u:<番号>`）から読む（組み込みは今の版が知らなくても形が合えば、知らない組み込みとして None）。
fn parse_token(token: &str) -> Option<BrushKey> {
    BrushKey::parse_token(token)
}

fn is_token_shape(token: &str) -> bool {
    match (token.strip_prefix("b:"), token.strip_prefix("u:")) {
        (Some(id), _) => !id.is_empty(),
        (_, Some(n)) => n.parse::<u32>().is_ok(),
        _ => false,
    }
}

/// 読んだ並びに、新しい版で足された組み込みのツール（列の最後へ）とブラシ（その組み込みのグループの最後へ）と、並びを書いたあとに
/// できた利用者のブラシのファイル（`users` の (番号, 元のグループ) のうち `user_through` より大きく並びに無い物。元のグループの最後へ）を足す。
/// 足した数を返す。
pub fn add_missing(read: &mut Read, users: &[(u32, Group)]) -> usize {
    let set = &mut read.set;
    let mut added = 0;
    for tool in Tool::ALL {
        if read.known_tools.contains(tool.id()) || set.first_of(tool).is_some() {
            continue;
        }
        if set.slots().len() >= MAX_TOOLS {
            break;
        }
        let mut slot = set.new_slot(tool);
        if holds_brushes(tool) {
            let group = set.new_group(None, None, Vec::new());
            slot.groups.push(group);
        }
        set.push_slot(slot);
        added += 1;
    }
    let place = |set: &mut ToolSet, key: BrushKey, group: Group| {
        if set.contains(key) {
            return false;
        }
        // いっぱいなら同じ組み込みのグループを次に作る。グループも上限なら並びに置かない（ファイルだけ残る）
        let Some(target) = set.builtin_group_with_room(group) else {
            return false;
        };
        set.group_mut(target)
            .expect("見つけたグループ")
            .brushes
            .push(key);
        true
    };
    for b in builtin::all() {
        if read.known_brushes.contains(b.id) {
            continue;
        }
        if place(set, BrushKey::Builtin(b.id), b.group) {
            added += 1;
        }
    }
    let mut users: Vec<(u32, Group)> = users
        .iter()
        .copied()
        .filter(|(id, _)| *id > read.user_through)
        .collect();
    users.sort_by_key(|(id, _)| *id);
    for (id, group) in users {
        if place(set, BrushKey::User(id), group) {
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_users(_: u32) -> UserFile {
        UserFile::Loaded
    }

    fn initial() -> ToolSet {
        ToolSet::initial(
            builtin::all()
                .iter()
                .map(|b| (BrushKey::Builtin(b.id), b.group)),
        )
    }

    fn read(text: &str) -> Read {
        match decode(text, &all_users).unwrap() {
            Decoded::Ok(read) => *read,
            Decoded::Newer(v) => panic!("新しい版 {v}"),
        }
    }

    #[test]
    fn a_layout_is_written_and_read_back_the_same() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let fill = set.first_of(Tool::Fill).unwrap();
        let ink = set
            .add_slot(
                Tool::Brush,
                Some(fill),
                Some("インク \"1\"".into()),
                Some("線".into()),
            )
            .unwrap();
        set.set_icon(ink, Some("stylus")).unwrap();
        let group = set.slot(ink).unwrap().groups[0].id;
        set.insert_brush(BrushKey::User(4), group, None).unwrap();
        let pen = set.slot(brush).unwrap().groups[0].id;
        set.rename_group(pen, "線画").unwrap();
        set.toggle_gap(fill).unwrap();
        set.remove_slot(set.first_of(Tool::Liquify).unwrap())
            .unwrap();
        let text = encode(&set, 4);
        let back = read(&text);
        assert_eq!(back.set, set);
        assert_eq!(back.user_through, 4);
        assert_eq!(back.skipped, 0);
        assert!(
            back.known_tools.contains("liquify"),
            "外したツールも知っていた"
        );
        assert_eq!(encode(&back.set, 4), text, "書き直しても同じ文");
    }

    #[test]
    fn broken_files_are_reported_by_kind() {
        let good = encode(&initial(), 0);
        let cases: Vec<(String, Broken)> = vec![
            ("{".into(), Broken::NotJson),
            ("[]".into(), Broken::Shape("root")),
            (good.replace(FORMAT, "other"), Broken::Shape("format")),
            (
                good.replace("\"version\": 1", "\"version\": 0"),
                Broken::Shape("version"),
            ),
            (
                good.replace("\"tool\": \"fill\"", "\"tool\": 3"),
                Broken::Shape("tools[].tool"),
            ),
            (
                good.replace("\"tool\": \"fill\"", "\"tool\": \"fill\", \"groups\": []"),
                Broken::Shape("tools[].groups"),
            ),
            (
                good.replace("\"tool\": \"fill\"", "\"tool\": \"move\""),
                Broken::Duplicate,
            ),
            (
                good.replace("\"b:pencil\"", "\"b:pencil\", \"b:pencil\""),
                Broken::Duplicate,
            ),
            (
                good.replace("\"b:pencil\"", "\"pencil\""),
                Broken::Shape("groups[].brushes[]"),
            ),
            (
                good.replace("\"tool\": \"eraser\"", "\"tool\": \"no-such-tool\""),
                Broken::MissingTool,
            ),
            (
                good.replace(
                    "\"user_brushes_through\": 0",
                    "\"user_brushes_through\": -1",
                ),
                Broken::Shape("user_brushes_through"),
            ),
            (" ".repeat(MAX_FILE_BYTES as usize + 1), Broken::TooLarge),
        ];
        for (text, expected) in cases {
            let got = decode(&text, &all_users).map(|_| ()).unwrap_err();
            assert_eq!(got, expected, "{}", &text[..text.len().min(80)]);
            for lang in [Lang::Ja, Lang::En] {
                assert!(!got.describe(lang).is_empty());
            }
        }
    }

    #[test]
    fn limits_make_the_file_unreadable() {
        let mut set = initial();
        while set.slots().len() < MAX_TOOLS {
            set.add_slot(Tool::Brush, None, None, None).unwrap();
        }
        let text = encode(&set, 0);
        assert!(decode(&text, &all_users).is_ok(), "上限ちょうどは読める");
        let over = text.replacen(
            "\"tools\": [",
            "\"tools\": [\n    { \"tool\": \"brush\" },",
            1,
        );
        assert_eq!(decode(&over, &all_users).unwrap_err(), Broken::TooMany);
        let many: Vec<String> = (0..=MAX_GROUP_BRUSHES)
            .map(|i| format!("\"u:{}\"", i + 1))
            .collect();
        let over = encode(&initial(), 0).replacen("\"b:standard-eraser\"", &many.join(", "), 1);
        assert_eq!(decode(&over, &all_users).unwrap_err(), Broken::TooMany);
    }

    #[test]
    fn a_newer_file_is_left_alone() {
        let text = encode(&initial(), 0).replace("\"version\": 1", "\"version\": 2");
        assert!(matches!(decode(&text, &all_users), Ok(Decoded::Newer(2))));
    }

    #[test]
    fn unknown_items_are_skipped_and_unknown_keys_are_ignored() {
        let text = encode(&initial(), 9)
            .replace("\"b:pencil\"", "\"b:no-such-brush\"")
            .replace(
                "\"tool\": \"liquify\"",
                "\"tool\": \"from-the-future\", \"extra\": 1",
            )
            .replace("\"b:marker\"", "\"b:marker\", \"u:5\"");
        let read = match decode(&text, &|n| {
            if n == 5 {
                UserFile::Missing
            } else {
                UserFile::Loaded
            }
        })
        .unwrap()
        {
            Decoded::Ok(r) => *r,
            Decoded::Newer(_) => unreachable!(),
        };
        assert_eq!(
            read.skipped, 3,
            "知らない組み込み・知らないツール・フォルダに無いファイル"
        );
        assert!(!read.set.contains(BrushKey::Builtin("pencil")));
        assert!(read.set.first_of(Tool::Liquify).is_none());
    }

    #[test]
    fn new_built_ins_and_files_written_after_the_layout_are_added() {
        let mut set = initial();
        // 利用者が外した物（known にある）は戻らない
        set.remove_brush(BrushKey::Builtin("chalk")).unwrap();
        set.remove_slot(set.first_of(Tool::Ruler).unwrap()).unwrap();
        let text = encode(&set, 5)
            // この版が知らなかった組み込みのツールとブラシ
            .replace("\"liquify\",", "")
            .replace("\"ink-pen\",", "")
            .replace("\"b:ink-pen\",", "")
            .replace("{\n      \"tool\": \"liquify\"\n    },", "");
        let mut r = read(&text);
        assert!(r.set.first_of(Tool::Liquify).is_none());
        assert!(!r.set.contains(BrushKey::Builtin("ink-pen")));
        let added = add_missing(
            &mut r,
            &[(3, Group::Pen), (6, Group::Eraser), (7, Group::Imported)],
        );
        assert_eq!(
            added, 4,
            "ツール・組み込みのブラシ・あとからのファイル 2 つ"
        );
        let last = r.set.slots().last().unwrap();
        assert_eq!(last.tool, Tool::Liquify, "新しいツールは列の最後");
        let ink = r.set.find(BrushKey::Builtin("ink-pen")).unwrap();
        let group = &r.set.slots()[ink.slot].groups[ink.group];
        assert_eq!(group.builtin, Some(Group::Pen));
        assert_eq!(group.brushes.last(), Some(&BrushKey::Builtin("ink-pen")));
        assert!(
            !r.set.contains(BrushKey::Builtin("chalk")),
            "外した組み込みは戻らない"
        );
        assert!(
            r.set.first_of(Tool::Ruler).is_none(),
            "外したツールは戻らない"
        );
        assert!(
            !r.set.contains(BrushKey::User(3)),
            "書いたときにあった物は外した物"
        );
        assert_eq!(
            r.set.slot_of(BrushKey::User(6)),
            r.set.first_of(Tool::Eraser)
        );
        let imported = r.set.find(BrushKey::User(7)).unwrap();
        assert_eq!(
            r.set.slots()[imported.slot].groups[imported.group].builtin,
            Some(Group::Imported),
            "取り込みのグループを作る"
        );
    }

    /// 起動のときの状態: 5 は読めなかった・6 は無い・7 は読めた。
    fn state_5_unloaded_6_missing(n: u32) -> UserFile {
        match n {
            5 => UserFile::Unloaded,
            6 => UserFile::Missing,
            _ => UserFile::Loaded,
        }
    }

    fn decoded(text: &str, user_file: &dyn Fn(u32) -> UserFile) -> Read {
        match decode(text, user_file).unwrap() {
            Decoded::Ok(r) => *r,
            Decoded::Newer(_) => unreachable!(),
        }
    }

    fn marker_group(set: &ToolSet) -> &BrushGroup {
        let p = set.find(BrushKey::Builtin("marker")).expect("marker");
        &set.slots()[p.slot].groups[p.group]
    }

    #[test]
    fn tokens_that_cannot_be_shown_are_kept_in_place_and_written_back() {
        let original = encode(&initial(), 9);
        let text = original.replace(
            "\"b:marker\"",
            "\"b:marker\", \"u:5\", \"b:from-the-future\", \"u:6\", \"u:7\"",
        );
        let read = decoded(&text, &state_5_unloaded_6_missing);
        assert_eq!(read.skipped, 3, "読めなかった・知らない・フォルダに無い");
        let group = marker_group(&read.set);
        let at = group
            .brushes
            .iter()
            .position(|k| *k == BrushKey::Builtin("marker"))
            .unwrap()
            + 1;
        assert_eq!(
            group.unread,
            [
                Unread {
                    at,
                    token: "u:5".into()
                },
                Unread {
                    at,
                    token: "b:from-the-future".into()
                },
            ],
            "フォルダに無い 6 は覚えない"
        );
        assert_eq!(group.brushes[at], BrushKey::User(7));
        assert!(!read.set.contains(BrushKey::User(5)), "画面には出さない");
        // 書き戻すと、6 だけが消え、ほかは元の順のまま
        let written = encode(&read.set, 9);
        let tokens = [
            "\"b:marker\"",
            "\"u:5\"",
            "\"b:from-the-future\"",
            "\"u:7\"",
        ];
        assert!(
            written.contains(&tokens.join(",\n            ")),
            "{written}"
        );
        assert!(!written.contains("u:6"));
        // 読み直すと同じ中身。5 が読めるようになれば、元の場所に出る
        assert_eq!(decoded(&written, &state_5_unloaded_6_missing).set, read.set);
        let healed = decoded(&written, &|_| UserFile::Loaded);
        let group = marker_group(&healed.set);
        assert!(group.unread.iter().all(|u| u.token != "u:5"));
        assert_eq!(group.brushes[at], BrushKey::User(5));
        assert_eq!(group.brushes[at + 1], BrushKey::User(7));
    }

    #[test]
    fn unread_tokens_count_toward_the_group_limit_and_follow_their_group() {
        let mut group = initial().slots()[0].groups[0].clone();
        group.brushes = (1..=3).map(BrushKey::User).collect();
        group.unread = vec![
            Unread {
                at: 0,
                token: "u:90".into(),
            },
            Unread {
                at: 2,
                token: "b:x".into(),
            },
            Unread {
                at: 99,
                token: "u:91".into(),
            },
        ];
        assert_eq!(
            group.tokens(),
            ["u:90", "u:1", "u:2", "b:x", "u:3", "u:91"],
            "場所は出せる札の数で数え、出せる札が減って足りなければ末尾に寄る"
        );
        assert_eq!(group.file_len(), 6);
        group.brushes = (1..=(MAX_GROUP_BRUSHES - 3) as u32)
            .map(BrushKey::User)
            .collect();
        assert!(!group.has_room(), "書き戻す札も数える");
        group.brushes.pop();
        assert!(group.has_room());
    }

    #[test]
    fn a_group_full_with_unread_tokens_still_writes_and_reads_back() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let group = set
            .add_group(brush, None, "満杯".into(), Vec::new())
            .unwrap();
        let g = set.group_mut(group).unwrap();
        g.brushes = (1..MAX_GROUP_BRUSHES as u32).map(BrushKey::User).collect();
        g.unread = vec![Unread {
            at: 3,
            token: "u:2000".into(),
        }];
        let text = encode(&set, 2000);
        let state = |n: u32| {
            if n == 2000 {
                UserFile::Unloaded
            } else {
                UserFile::Loaded
            }
        };
        assert_eq!(decoded(&text, &state).set, set);
        assert!(set.insert_brush(BrushKey::User(5000), group, None).is_err());
    }
}
