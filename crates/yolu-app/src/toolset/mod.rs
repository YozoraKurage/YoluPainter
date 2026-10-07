//! ツールの並び: 左のツールの列（ツールの順・区切り・名前・アイコン）と、ブラシ・消しゴムのツールが持つブラシのグループ（グループの順・名前・
//! 中のブラシの順）。保存は設定のフォルダの `tools.json`（`file`）で、.ylp には入れない。並びの操作は文書ではない（取り消しの段に入れない）。
//!
//! ツールの列の 1 つ（`ToolSlot`）は、ツールの表のツール（`Tool`）を 1 つ指す。ブラシと消しゴムのツールは何個でも置け、それぞれがブラシの
//! グループ（`BrushGroup`）の並びを持つ（好きなブラシの組を 1 つのツールにできる）。消すかどうかはツールが決める（消しゴムのツールの中の
//! ブラシは消す）。ほかのツールは列に 1 つまで（列から外しても、キーとメニューからは使える）。ブラシのツール・消しゴムのツールは、それぞれ
//! 1 つは列に残す（キーの B・E の行き先と、ブラシの一覧を出す先が無くならないように）。
//!
//! グループの中のブラシは、ブラシの一覧（`brushes`）の印（`BrushKey`）で指す。組み込みは変えるまで `b:<id>` の参照のまま、ほかは利用者の
//! ブラシのファイル（`u:<番号>`）。同じ印は並び全体で 1 か所だけ（2 つ目を置くときは、ファイルの写しを作る。`brushes` の側）。
//! 並びから外したブラシのファイルは消さない（「＋」の窓の自分のブラシから戻せる）。
//!
//! 画面の状態（今のツール・ツールごとに出しているグループと最後のブラシ・ドラッグ・名前の入力）は保存しない。

pub mod catalog;
pub mod file;
pub mod ops;
pub mod ui;

pub use ops::{LoadProblem, ToolsetAction, ToolsetState};

use std::collections::HashMap;

use crate::brushes::{clean_name, BrushKey, Group};
use crate::lang::Lang;
use crate::state::Tool;

/// ツールの列のツールの数の上限。
pub const MAX_TOOLS: usize = 64;
/// 1 つのツールが持つグループの数の上限。
pub const MAX_GROUPS: usize = 64;
/// 1 つのグループのブラシの数の上限。
pub const MAX_GROUP_BRUSHES: usize = 512;

/// ツールの列の 1 つの印（起動のたびに振る。ファイルには書かない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(u32);

/// ブラシのグループの印（起動のたびに振る。ファイルには書かない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupId(u32);

/// ツールの列のボタンに選べるアイコン（`key` はファイルに書く名前。`normal`・`selected` は描くアイコン。アイコンは今の許諾つきの組から）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconChoice {
    pub key: &'static str,
    pub normal: &'static str,
    pub selected: &'static str,
}

const fn tool_icon(key: &'static str, normal: &'static str, selected: &'static str) -> IconChoice {
    IconChoice {
        key,
        normal,
        selected,
    }
}

/// 選べるアイコン（ツールの表のアイコンと、ブラシのツールに合う形）。
pub static ICONS: [IconChoice; 28] = [
    tool_icon("brush", "tools/brush", "tools/brush_selected"),
    tool_icon("eraser", "tools/eraser", "tools/eraser_selected"),
    tool_icon("fill", "tools/fill", "tools/fill_selected"),
    tool_icon("gradient", "tools/gradient", "tools/gradient_selected"),
    tool_icon("shape", "tools/shape", "tools/shape_selected"),
    tool_icon("ruler", "tools/ruler", "tools/ruler_selected"),
    tool_icon(
        "polygon-fill",
        "tools/polygon-fill",
        "tools/polygon-fill_selected",
    ),
    tool_icon(
        "eyedropper",
        "tools/eyedropper",
        "tools/eyedropper_selected",
    ),
    tool_icon(
        "select-rectangle",
        "tools/select-rectangle",
        "tools/select-rectangle_selected",
    ),
    tool_icon(
        "select-ellipse",
        "tools/select-ellipse",
        "tools/select-ellipse_selected",
    ),
    tool_icon("lasso", "tools/lasso", "tools/lasso_selected"),
    tool_icon(
        "select-polygon",
        "tools/select-polygon",
        "tools/select-polygon_selected",
    ),
    tool_icon(
        "magic-wand",
        "tools/magic-wand",
        "tools/magic-wand_selected",
    ),
    tool_icon("id-select", "tools/id-select", "tools/id-select_selected"),
    tool_icon(
        "select-pen",
        "tools/select-pen",
        "tools/select-pen_selected",
    ),
    tool_icon("move", "tools/move", "tools/move_selected"),
    tool_icon("liquify", "tools/liquify", "tools/liquify_selected"),
    tool_icon("path", "tools/path", "tools/path_selected"),
    tool_icon("stylus", "stylus", "stylus"),
    tool_icon("paint-brush", "paint_brush", "paint_brush"),
    tool_icon("ink-stroke", "ink_stroke", "ink_stroke"),
    tool_icon("blur", "blur_on", "blur_on"),
    tool_icon("scatter", "data_scatter", "data_scatter"),
    tool_icon("palette", "palette", "palette"),
    tool_icon("texture", "texture", "texture"),
    tool_icon("sparkle", "auto_awesome", "auto_awesome"),
    tool_icon("drop", "opacity", "opacity"),
    tool_icon("target", "target", "target"),
];

/// 名前のアイコン（知らない名前は None）。
pub fn icon(key: &str) -> Option<&'static IconChoice> {
    ICONS.iter().find(|i| i.key == key)
}

/// ツールの表のアイコン（ツールの id と同じ名前の選べるアイコン）。
pub fn tool_default_icon(tool: Tool) -> IconChoice {
    icon(tool.id()).copied().unwrap_or(IconChoice {
        key: "",
        normal: "tools/brush",
        selected: "tools/brush_selected",
    })
}

/// ブラシのグループ（ブラシ・消しゴムのツールの一覧の上のタブ 1 つ）。
#[derive(Clone, Debug, PartialEq)]
pub struct BrushGroup {
    pub id: GroupId,
    /// 組み込みのグループから作った物（名前を変えていなければ、その名前を言語で出す。アイコンも）。
    pub builtin: Option<Group>,
    /// 利用者が付けた名前（None なら組み込みの名前）。
    pub name: Option<String>,
    pub brushes: Vec<BrushKey>,
    /// 読んだときに出せなかった札（読めなかった利用者のブラシのファイル・今の版が知らない組み込みのブラシ）。画面には出さず、
    /// 書くときは元の場所のまま書き戻す（読めるようになったとき、元のグループの元の場所にあるように）。
    pub unread: Vec<Unread>,
}

/// グループの札のうち、読んだときに出せなかった物 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    /// 読んだとき、この札の前にあった出せる札の数（書くときの場所。そのあとの編集で出せる札が減ったら、末尾に寄る）。
    pub at: usize,
    /// ファイルの札（`b:<id>`・`u:<番号>`）。
    pub token: String,
}

impl BrushGroup {
    /// ファイルに書く札の数（出せる物と、出せなかった物）。
    pub fn file_len(&self) -> usize {
        self.brushes.len() + self.unread.len()
    }

    /// あと 1 つ札を置けるか（上限は、書き戻す札も数える）。
    pub fn has_room(&self) -> bool {
        self.file_len() < MAX_GROUP_BRUSHES
    }

    /// ファイルに書く札を、出せなかった物を元の場所へ戻した並びで。
    pub fn tokens(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.file_len());
        let mut unread = self.unread.iter().peekable();
        for (i, key) in self.brushes.iter().enumerate() {
            while let Some(u) = unread.next_if(|u| u.at <= i) {
                out.push(u.token.clone());
            }
            out.push(key.token());
        }
        out.extend(unread.map(|u| u.token.clone()));
        out
    }

    pub fn name_in(&self, lang: Lang) -> String {
        match (&self.name, self.builtin) {
            (Some(name), _) => name.clone(),
            (None, Some(group)) => group.name(lang).to_owned(),
            (None, None) => lang.pick("グループ", "Group").to_owned(),
        }
    }

    /// タブに書く短い名前（利用者の名前はそのまま。組み込みの名前は短い形）。
    pub fn short_in(&self, lang: Lang) -> String {
        match (&self.name, self.builtin) {
            (None, Some(group)) => group.short(lang).to_owned(),
            _ => self.name_in(lang),
        }
    }
}

/// ツールの列の 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSlot {
    pub id: SlotId,
    /// 振る舞い（描く・消す・選ぶ…）を決めるツールの表のツール。
    pub tool: Tool,
    /// 利用者が付けた名前（None ならツールの表の名前）。
    pub name: Option<String>,
    /// 利用者が選んだアイコン（`ICONS` の名前。None ならツールのアイコン）。
    pub icon: Option<&'static str>,
    /// このツールの前に区切りを入れる。
    pub gap: bool,
    /// ブラシのグループ（ブラシ・消しゴムのツールだけ）。
    pub groups: Vec<BrushGroup>,
}

impl ToolSlot {
    /// ブラシのグループを持つツールか（ブラシと消しゴム）。
    pub fn holds_brushes(&self) -> bool {
        holds_brushes(self.tool)
    }

    pub fn name_in(&self, lang: Lang) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.tool.name_in(lang).to_owned())
    }

    /// 描くアイコン。
    pub fn icon(&self) -> IconChoice {
        self.icon
            .and_then(icon)
            .copied()
            .unwrap_or_else(|| tool_default_icon(self.tool))
    }

    pub fn group(&self, id: GroupId) -> Option<&BrushGroup> {
        self.groups.iter().find(|g| g.id == id)
    }
}

/// ブラシのグループを持つツールか（ブラシと消しゴム。描くツール）。
pub fn holds_brushes(tool: Tool) -> bool {
    tool.paints()
}

/// 並びの中のブラシの場所（ツール・グループ・グループの中の番号。添字）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub slot: usize,
    pub group: usize,
    pub index: usize,
}

/// 並びを変えられない理由（`tools.json` に触らない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// 今の版より新しい（触らずに初めの並びで動いている）。
    Newer(u64),
    /// 読めなかった・読めないファイルを退避できなかった（上書きしない）。
    Unreadable,
}

/// 並びを変える操作を断る理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `tools.json` が今の版より新しい（触らずに初めの並びで動いている）。
    Newer(u64),
    /// `tools.json` を読めなかった（上書きしない）。
    Unreadable,
    TooManyTools,
    TooManyGroups,
    TooManyBrushes,
    /// ブラシ・消しゴムのツールの最後の 1 つ。
    LastOfKind(Tool),
    /// ブラシのグループを持てないツール。
    NoGroups,
    /// 同じツールは列に 1 つまで（ブラシ・消しゴムのツールは別）。
    AlreadyPlaced(Tool),
    /// 指す物が無い。
    Missing,
}

/// ツールの並びの全体。
#[derive(Clone, Debug)]
pub struct ToolSet {
    slots: Vec<ToolSlot>,
    next: u32,
    /// 今のツールの列の 1 つ（列から外したツールをキーで使っている間は None）。
    active: Option<SlotId>,
    /// ツールごとに出しているグループ。
    shown: HashMap<SlotId, GroupId>,
    /// ツールごとに最後に使ったブラシ。
    last: HashMap<SlotId, BrushKey>,
    /// 並びを変えられない理由（新しい版の・読めなかった `tools.json`）。
    pub locked: Option<Lock>,
}

impl PartialEq for ToolSet {
    /// 保存する中身（ツール・名前・アイコン・区切り・グループ・ブラシ）が同じか（印の番号と画面の状態は見ない）。
    fn eq(&self, other: &ToolSet) -> bool {
        self.slots.len() == other.slots.len()
            && self.slots.iter().zip(&other.slots).all(|(a, b)| {
                a.tool == b.tool
                    && a.name == b.name
                    && a.icon == b.icon
                    && a.gap == b.gap
                    && a.groups.len() == b.groups.len()
                    && a.groups.iter().zip(&b.groups).all(|(x, y)| {
                        x.builtin == y.builtin
                            && x.name == y.name
                            && x.brushes == y.brushes
                            && x.unread == y.unread
                    })
            })
    }
}

impl Default for ToolSet {
    /// 組み込みのブラシだけの最初の並び（設定のフォルダを読む前・試験）。
    fn default() -> Self {
        ToolSet::initial(
            crate::brushes::builtin::all()
                .iter()
                .map(|b| (BrushKey::Builtin(b.id), b.group)),
        )
    }
}

/// 初めの並びで、ブラシのツールが持つ組み込みのグループ（取り込みは、取り込んだブラシがあるときだけ最後に）。
const BRUSH_GROUPS: [Group; 5] = [
    Group::Pen,
    Group::Brush,
    Group::Airbrush,
    Group::Effect,
    Group::Special,
];

impl ToolSet {
    fn empty() -> ToolSet {
        ToolSet {
            slots: Vec::new(),
            next: 1,
            active: None,
            shown: HashMap::new(),
            last: HashMap::new(),
            locked: None,
        }
    }

    fn take_id(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }

    pub(crate) fn new_slot(&mut self, tool: Tool) -> ToolSlot {
        ToolSlot {
            id: SlotId(self.take_id()),
            tool,
            name: None,
            icon: None,
            gap: false,
            groups: Vec::new(),
        }
    }

    pub(crate) fn new_group(
        &mut self,
        builtin: Option<Group>,
        name: Option<String>,
        brushes: Vec<BrushKey>,
    ) -> BrushGroup {
        BrushGroup {
            id: GroupId(self.take_id()),
            builtin,
            name,
            brushes,
            unread: Vec::new(),
        }
    }

    /// 初めの並び（ツールの表の並びと区切り。ブラシのツールにペン・筆・エアブラシ・効果・特殊（取り込んだブラシがあれば取り込み）、
    /// 消しゴムのツールに消しゴムのグループ）に、ブラシを元のグループ（組み込みはそのグループ、利用者のブラシはファイルの `group=`）の
    /// 順に入れる。
    pub fn initial(brushes: impl IntoIterator<Item = (BrushKey, Group)>) -> ToolSet {
        let brushes: Vec<(BrushKey, Group)> = brushes.into_iter().collect();
        let mut set = ToolSet::empty();
        let imported = brushes.iter().any(|(_, g)| *g == Group::Imported);
        for tool in Tool::ALL {
            let mut slot = set.new_slot(tool);
            slot.gap = tool.def().starts_group;
            let groups: Vec<Group> = match tool {
                Tool::Brush => BRUSH_GROUPS
                    .iter()
                    .copied()
                    .chain(imported.then_some(Group::Imported))
                    .collect(),
                Tool::Eraser => vec![Group::Eraser],
                _ => Vec::new(),
            };
            for g in groups {
                let group = set.new_group(Some(g), None, Vec::new());
                slot.groups.push(group);
            }
            set.slots.push(slot);
        }
        for (key, group) in brushes {
            if set.find(key).is_some() {
                continue;
            }
            // グループがいっぱいなら同じ組み込みのグループを次に作って続きを入れる。グループも上限なら並びに置かない（ファイルだけ残る）
            if let Some(target) = set.builtin_group_with_room(group) {
                set.group_mut(target)
                    .expect("見つけたグループ")
                    .brushes
                    .push(key);
            }
        }
        set.active = set.first_of(Tool::Brush);
        set
    }

    /// 組み込みのグループ `group` の最初の物。無ければ、そのブラシが消すなら最初の消しゴムのツールの先頭のグループ、ほかは最初のブラシの
    /// ツールの先頭のグループ（取り込みは、最初のブラシのツールの最後に作る）。
    pub(crate) fn builtin_group_or_first(&mut self, group: Group) -> Option<GroupId> {
        if let Some(found) = self
            .slots
            .iter()
            .flat_map(|s| &s.groups)
            .find(|g| g.builtin == Some(group))
        {
            return Some(found.id);
        }
        let tool = if group.is_eraser() {
            Tool::Eraser
        } else {
            Tool::Brush
        };
        let slot = self.slots.iter().position(|s| s.tool == tool)?;
        if group == Group::Imported || self.slots[slot].groups.is_empty() {
            if self.slots[slot].groups.len() >= MAX_GROUPS {
                return self.slots[slot].groups.last().map(|g| g.id);
            }
            let new = self.new_group(Some(group), None, Vec::new());
            let id = new.id;
            self.slots[slot].groups.push(new);
            return Some(id);
        }
        self.slots[slot].groups.first().map(|g| g.id)
    }

    /// 組み込みのグループ `group` のブラシを置く先。同じ組み込みのグループで空きのある最初の物。どれもいっぱいなら、同じ組み込みの
    /// 最後のグループの次に、同じ組み込みのグループを作る。同じ組み込みのグループが無いときは `builtin_group_or_first` と同じ代わりの
    /// ツール（取り込みは最後に作る）で、そこがいっぱいなら空きのあるグループ、無ければ最後に作る。ツールのグループが上限なら None
    /// （呼ぶ側は並びに置かず、ファイルだけ残す）。
    pub(crate) fn builtin_group_with_room(&mut self, group: Group) -> Option<GroupId> {
        if let Some(found) = self
            .slots
            .iter()
            .flat_map(|s| &s.groups)
            .find(|g| g.builtin == Some(group) && g.has_room())
        {
            return Some(found.id);
        }
        let last_same = self
            .slots
            .iter()
            .enumerate()
            .flat_map(|(si, s)| {
                s.groups
                    .iter()
                    .enumerate()
                    .filter(move |(_, g)| g.builtin == Some(group))
                    .map(move |(gi, _)| (si, gi))
            })
            .last();
        if let Some((si, gi)) = last_same {
            return self.insert_builtin_group(si, gi + 1, group);
        }
        let tool = if group.is_eraser() {
            Tool::Eraser
        } else {
            Tool::Brush
        };
        let si = self.slots.iter().position(|s| s.tool == tool)?;
        let end = self.slots[si].groups.len();
        if group == Group::Imported || end == 0 {
            return self.insert_builtin_group(si, end, group);
        }
        match self.slots[si].groups.iter().find(|g| g.has_room()) {
            Some(g) => Some(g.id),
            None => self.insert_builtin_group(si, end, group),
        }
    }

    /// ツール（添字 `si`）の `at` 番目に、組み込みのグループ `group` から作ったグループを足す（ツールのグループが上限なら None）。
    fn insert_builtin_group(&mut self, si: usize, at: usize, group: Group) -> Option<GroupId> {
        if self.slots[si].groups.len() >= MAX_GROUPS {
            return None;
        }
        let new = self.new_group(Some(group), None, Vec::new());
        let id = new.id;
        self.slots[si].groups.insert(at, new);
        Some(id)
    }

    pub fn slots(&self) -> &[ToolSlot] {
        &self.slots
    }

    pub fn slot(&self, id: SlotId) -> Option<&ToolSlot> {
        self.slots.iter().find(|s| s.id == id)
    }

    pub(crate) fn slot_mut(&mut self, id: SlotId) -> Option<&mut ToolSlot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    pub fn slot_index(&self, id: SlotId) -> Option<usize> {
        self.slots.iter().position(|s| s.id == id)
    }

    /// グループと、それを持つツール。
    pub fn group(&self, id: GroupId) -> Option<(&ToolSlot, &BrushGroup)> {
        self.slots.iter().find_map(|s| s.group(id).map(|g| (s, g)))
    }

    pub(crate) fn group_mut(&mut self, id: GroupId) -> Option<&mut BrushGroup> {
        self.slots
            .iter_mut()
            .flat_map(|s| s.groups.iter_mut())
            .find(|g| g.id == id)
    }

    /// グループを持つツールの印。
    pub fn slot_of_group(&self, id: GroupId) -> Option<SlotId> {
        self.group(id).map(|(s, _)| s.id)
    }

    /// ブラシの場所。
    pub fn find(&self, key: BrushKey) -> Option<Place> {
        for (si, slot) in self.slots.iter().enumerate() {
            for (gi, group) in slot.groups.iter().enumerate() {
                if let Some(i) = group.brushes.iter().position(|k| *k == key) {
                    return Some(Place {
                        slot: si,
                        group: gi,
                        index: i,
                    });
                }
            }
        }
        None
    }

    pub fn contains(&self, key: BrushKey) -> bool {
        self.find(key).is_some()
    }

    /// ブラシのあるツール。
    pub fn slot_of(&self, key: BrushKey) -> Option<SlotId> {
        self.find(key).map(|p| self.slots[p.slot].id)
    }

    /// ブラシのあるグループ。
    pub fn group_of(&self, key: BrushKey) -> Option<GroupId> {
        self.find(key)
            .map(|p| self.slots[p.slot].groups[p.group].id)
    }

    /// 並びの全部のブラシ（列の順・グループの順）。
    pub fn brushes(&self) -> Vec<BrushKey> {
        self.slots
            .iter()
            .flat_map(|s| s.groups.iter().flat_map(|g| g.brushes.iter().copied()))
            .collect()
    }

    /// ツール `tool` の最初の 1 つ。
    pub fn first_of(&self, tool: Tool) -> Option<SlotId> {
        self.slots.iter().find(|s| s.tool == tool).map(|s| s.id)
    }

    /// ツール `tool` に替えるときの行き先（今のツールがもうそのツールなら今のまま、ほかは列の最初の 1 つ。列に無ければ None）。
    pub fn slot_for_tool(&self, tool: Tool) -> Option<SlotId> {
        self.active
            .filter(|id| self.slot(*id).is_some_and(|s| s.tool == tool))
            .or_else(|| self.first_of(tool))
    }

    pub fn active(&self) -> Option<SlotId> {
        self.active
    }

    pub fn active_slot(&self) -> Option<&ToolSlot> {
        self.active.and_then(|id| self.slot(id))
    }

    pub(crate) fn set_active(&mut self, slot: Option<SlotId>) {
        self.active = slot.filter(|id| self.slot(*id).is_some());
    }

    /// ツールで出しているグループ（覚えていなければ先頭）。
    pub fn shown_group(&self, slot: SlotId) -> Option<GroupId> {
        let slot = self.slot(slot)?;
        self.shown
            .get(&slot.id)
            .copied()
            .filter(|g| slot.group(*g).is_some())
            .or_else(|| slot.groups.first().map(|g| g.id))
    }

    pub(crate) fn show_group(&mut self, group: GroupId) {
        if let Some(slot) = self.slot_of_group(group) {
            self.shown.insert(slot, group);
        }
    }

    /// ツールで最後に使ったブラシ（まだそのツールの中にある物だけ）。
    pub fn last(&self, slot: SlotId) -> Option<BrushKey> {
        self.last
            .get(&slot)
            .copied()
            .filter(|k| self.slot_of(*k) == Some(slot))
    }

    /// `key` を、そのツールの最後のブラシ・出すグループにする。
    pub(crate) fn note_used(&mut self, key: BrushKey) {
        if let Some(p) = self.find(key) {
            let slot = self.slots[p.slot].id;
            let group = self.slots[p.slot].groups[p.group].id;
            self.last.insert(slot, key);
            self.shown.insert(slot, group);
        }
    }

    /// ツールに替えたときに選ぶブラシ（最後のブラシ、なければ出しているグループの先頭、なければツールの最初のブラシ）。
    pub fn pick_for(&self, slot: SlotId) -> Option<BrushKey> {
        if let Some(key) = self.last(slot) {
            return Some(key);
        }
        let s = self.slot(slot)?;
        self.shown_group(slot)
            .and_then(|g| s.group(g))
            .and_then(|g| g.brushes.first().copied())
            .or_else(|| s.groups.iter().find_map(|g| g.brushes.first().copied()))
    }

    /// 列に無い（外した）組み込みのツール（ブラシ・消しゴムのツールは何個でも足せるので含めない）。
    pub fn removed_tools(&self) -> Vec<Tool> {
        Tool::ALL
            .into_iter()
            .filter(|t| !holds_brushes(*t) && self.first_of(*t).is_none())
            .collect()
    }

    /// 並びを変える操作を断る理由（変えられるなら None）。
    pub fn lock_refusal(&self) -> Option<Refusal> {
        self.locked.map(|lock| match lock {
            Lock::Newer(version) => Refusal::Newer(version),
            Lock::Unreadable => Refusal::Unreadable,
        })
    }

    fn check_unlocked(&self) -> Result<(), Refusal> {
        match self.lock_refusal() {
            Some(r) => Err(r),
            None => Ok(()),
        }
    }

    /// ツールを `before` の前（None なら最後）に足す。ブラシ・消しゴムのツールは名前 `name` と空のグループ `group_name` を 1 つ持って始まる。
    pub(crate) fn add_slot(
        &mut self,
        tool: Tool,
        before: Option<SlotId>,
        name: Option<String>,
        group_name: Option<String>,
    ) -> Result<SlotId, Refusal> {
        self.check_unlocked()?;
        if self.slots.len() >= MAX_TOOLS {
            return Err(Refusal::TooManyTools);
        }
        if !holds_brushes(tool) && self.first_of(tool).is_some() {
            return Err(Refusal::AlreadyPlaced(tool));
        }
        let mut slot = self.new_slot(tool);
        slot.name = name;
        if holds_brushes(tool) {
            let group = self.new_group(None, group_name, Vec::new());
            slot.groups.push(group);
        }
        let id = slot.id;
        let at = before
            .and_then(|b| self.slot_index(b))
            .unwrap_or(self.slots.len());
        self.slots.insert(at, slot);
        Ok(id)
    }

    /// ツールを列から外す（中のブラシは並びから外れるだけ。返すのは外れたブラシ）。
    pub(crate) fn remove_slot(&mut self, id: SlotId) -> Result<Vec<BrushKey>, Refusal> {
        self.check_unlocked()?;
        let at = self.slot_index(id).ok_or(Refusal::Missing)?;
        let tool = self.slots[at].tool;
        if holds_brushes(tool) && self.slots.iter().filter(|s| s.tool == tool).count() <= 1 {
            return Err(Refusal::LastOfKind(tool));
        }
        let slot = self.slots.remove(at);
        // 外したツールの前の区切りは、次のツールが受け継ぐ（区切りの並びを崩さない）
        if slot.gap {
            if let Some(next) = self.slots.get_mut(at) {
                next.gap = true;
            }
        }
        self.shown.remove(&id);
        self.last.remove(&id);
        if self.active == Some(id) {
            self.active = None;
        }
        Ok(slot.groups.into_iter().flat_map(|g| g.brushes).collect())
    }

    /// ツールを `before` の前（None なら最後）へ動かす。動いたら true。
    pub(crate) fn move_slot(
        &mut self,
        id: SlotId,
        before: Option<SlotId>,
    ) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        if before == Some(id) {
            return Ok(false);
        }
        let from = self.slot_index(id).ok_or(Refusal::Missing)?;
        let order: Vec<SlotId> = self.slots.iter().map(|s| s.id).collect();
        let slot = self.slots.remove(from);
        let to = before
            .and_then(|b| self.slot_index(b))
            .unwrap_or(self.slots.len());
        self.slots.insert(to, slot);
        Ok(self.slots.iter().map(|s| s.id).ne(order))
    }

    /// ツールの名前（None・空ならツールの表の名前へ戻す）。変わったら true。
    pub(crate) fn rename_slot(&mut self, id: SlotId, name: Option<&str>) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        let slot = self.slot_mut(id).ok_or(Refusal::Missing)?;
        let builtin = slot.tool.name_in(Lang::Ja);
        let builtin_en = slot.tool.name_in(Lang::En);
        // ツールの表の名前と同じ名前は、名前を持たない（言語を替えたら、その言語の名前になる）
        let name = name
            .and_then(clean_name)
            .filter(|n| n != builtin && n != builtin_en);
        if slot.name == name {
            return Ok(false);
        }
        slot.name = name;
        Ok(true)
    }

    /// ツールのアイコン（None ならツールのアイコン）。変わったら true。
    pub(crate) fn set_icon(&mut self, id: SlotId, key: Option<&str>) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        let slot = self.slot_mut(id).ok_or(Refusal::Missing)?;
        let choice = key.and_then(icon).map(|i| i.key);
        let default = tool_default_icon(slot.tool).key;
        let choice = choice.filter(|k| *k != default);
        if slot.icon == choice {
            return Ok(false);
        }
        slot.icon = choice;
        Ok(true)
    }

    /// ツールの前の区切りを入れる・外す。
    pub(crate) fn toggle_gap(&mut self, id: SlotId) -> Result<(), Refusal> {
        self.check_unlocked()?;
        let slot = self.slot_mut(id).ok_or(Refusal::Missing)?;
        slot.gap = !slot.gap;
        Ok(())
    }

    /// グループをツールの `before` の前（None なら最後）に足す。
    pub(crate) fn add_group(
        &mut self,
        slot: SlotId,
        before: Option<GroupId>,
        name: String,
        brushes: Vec<BrushKey>,
    ) -> Result<GroupId, Refusal> {
        self.check_unlocked()?;
        let s = self.slot(slot).ok_or(Refusal::Missing)?;
        if !s.holds_brushes() {
            return Err(Refusal::NoGroups);
        }
        if s.groups.len() >= MAX_GROUPS {
            return Err(Refusal::TooManyGroups);
        }
        if brushes.len() > MAX_GROUP_BRUSHES {
            return Err(Refusal::TooManyBrushes);
        }
        let group = self.new_group(None, clean_name(&name), brushes);
        let id = group.id;
        let s = self.slot_mut(slot).expect("確かめたツール");
        let at = before
            .and_then(|b| s.groups.iter().position(|g| g.id == b))
            .unwrap_or(s.groups.len());
        s.groups.insert(at, group);
        Ok(id)
    }

    /// グループを外す（中のブラシは並びから外れるだけ。返すのは外れたブラシ）。
    pub(crate) fn remove_group(&mut self, id: GroupId) -> Result<Vec<BrushKey>, Refusal> {
        self.check_unlocked()?;
        let slot = self.slot_of_group(id).ok_or(Refusal::Missing)?;
        let s = self.slot_mut(slot).expect("グループを持つツール");
        let at = s.groups.iter().position(|g| g.id == id).expect("グループ");
        let group = s.groups.remove(at);
        if self.shown.get(&slot) == Some(&id) {
            // 隣のグループを出す
            let next = self
                .slot(slot)
                .and_then(|s| s.groups.get(at).or(s.groups.last()))
                .map(|g| g.id);
            match next {
                Some(next) => {
                    self.shown.insert(slot, next);
                }
                None => {
                    self.shown.remove(&slot);
                }
            }
        }
        Ok(group.brushes)
    }

    /// グループの名前（空なら変えない）。変わったら true。
    pub(crate) fn rename_group(&mut self, id: GroupId, name: &str) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        let group = self.group_mut(id).ok_or(Refusal::Missing)?;
        let Some(name) = clean_name(name) else {
            return Ok(false);
        };
        // 組み込みのグループの名前と同じなら、名前を持たない（言語で替わる）
        let same_builtin = group
            .builtin
            .is_some_and(|g| g.name(Lang::Ja) == name || g.name(Lang::En) == name);
        let next = (!same_builtin).then_some(name);
        if group.name == next {
            return Ok(false);
        }
        group.name = next;
        Ok(true)
    }

    /// グループをツール `to` の `before` の前（None なら最後）へ動かす（別のツールへも）。動いたら true。
    pub(crate) fn move_group(
        &mut self,
        id: GroupId,
        to: SlotId,
        before: Option<GroupId>,
    ) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        if before == Some(id) {
            return Ok(false);
        }
        let from = self.slot_of_group(id).ok_or(Refusal::Missing)?;
        let target = self.slot(to).ok_or(Refusal::Missing)?;
        if !target.holds_brushes() {
            return Err(Refusal::NoGroups);
        }
        if from != to && target.groups.len() >= MAX_GROUPS {
            return Err(Refusal::TooManyGroups);
        }
        let before_order: Vec<(SlotId, Vec<GroupId>)> = self.group_order();
        let s = self.slot_mut(from).expect("グループを持つツール");
        let at = s.groups.iter().position(|g| g.id == id).expect("グループ");
        let group = s.groups.remove(at);
        if from != to && self.shown.get(&from) == Some(&id) {
            self.shown.remove(&from);
        }
        let t = self.slot_mut(to).expect("確かめたツール");
        let at = before
            .and_then(|b| t.groups.iter().position(|g| g.id == b))
            .unwrap_or(t.groups.len());
        t.groups.insert(at, group);
        Ok(self.group_order() != before_order)
    }

    fn group_order(&self) -> Vec<(SlotId, Vec<GroupId>)> {
        self.slots
            .iter()
            .map(|s| (s.id, s.groups.iter().map(|g| g.id).collect()))
            .collect()
    }

    /// ブラシを並びから外す（外したら true）。
    pub(crate) fn remove_brush(&mut self, key: BrushKey) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        let Some(p) = self.find(key) else {
            return Ok(false);
        };
        self.slots[p.slot].groups[p.group].brushes.remove(p.index);
        Ok(true)
    }

    /// ブラシ `key`（並びに無い物）をグループ `group` の `before` の前（None なら最後）に置く。
    pub(crate) fn insert_brush(
        &mut self,
        key: BrushKey,
        group: GroupId,
        before: Option<BrushKey>,
    ) -> Result<(), Refusal> {
        self.check_unlocked()?;
        if self.contains(key) {
            return Err(Refusal::Missing);
        }
        let g = self.group_mut(group).ok_or(Refusal::Missing)?;
        if !g.has_room() {
            return Err(Refusal::TooManyBrushes);
        }
        let at = before
            .and_then(|b| g.brushes.iter().position(|k| *k == b))
            .unwrap_or(g.brushes.len());
        g.brushes.insert(at, key);
        Ok(())
    }

    /// ブラシ `key` を、グループ `group` の `before` の前（None なら最後）へ動かす（別のグループ・別のツールへも）。動いたら true。
    pub(crate) fn move_brush(
        &mut self,
        key: BrushKey,
        group: GroupId,
        before: Option<BrushKey>,
    ) -> Result<bool, Refusal> {
        self.check_unlocked()?;
        if before == Some(key) {
            return Ok(false);
        }
        let from = self.find(key).ok_or(Refusal::Missing)?;
        let from_group = self.slots[from.slot].groups[from.group].id;
        let target = self.group(group).ok_or(Refusal::Missing)?.1;
        if from_group != group && !target.has_room() {
            return Err(Refusal::TooManyBrushes);
        }
        let before_order = self.brushes();
        self.slots[from.slot].groups[from.group]
            .brushes
            .remove(from.index);
        let g = self.group_mut(group).expect("確かめたグループ");
        let at = before
            .and_then(|b| g.brushes.iter().position(|k| *k == b))
            .unwrap_or(g.brushes.len());
        g.brushes.insert(at, key);
        Ok(self.brushes() != before_order)
    }

    /// 並びの中の `old` を同じ場所で `new` に替える（組み込みの参照を、ファイルの写しに替えるとき）。
    pub(crate) fn replace_brush(&mut self, old: BrushKey, new: BrushKey) -> bool {
        let Some(p) = self.find(old) else {
            return false;
        };
        self.slots[p.slot].groups[p.group].brushes[p.index] = new;
        for key in self.last.values_mut() {
            if *key == old {
                *key = new;
            }
        }
        true
    }

    /// 並び全体を `other` に替える（最初の並びに戻す。今のツールは同じツールの最初の 1 つへ）。
    pub(crate) fn replace_with(&mut self, other: ToolSet) {
        let active_tool = self.active_slot().map(|s| s.tool);
        let locked = self.locked;
        *self = other;
        self.locked = locked;
        self.active = active_tool.and_then(|t| self.first_of(t));
    }

    /// ツールの最後に、取り込みのグループを作る（グループがいっぱいなら None）。
    pub(crate) fn add_imported_group(&mut self, slot: SlotId) -> Option<GroupId> {
        if self.locked.is_some() {
            return None;
        }
        let s = self.slot(slot)?;
        if !s.holds_brushes() || s.groups.len() >= MAX_GROUPS {
            return None;
        }
        let group = self.new_group(Some(Group::Imported), None, Vec::new());
        let id = group.id;
        self.slot_mut(slot)?.groups.push(group);
        Some(id)
    }

    /// 読んだ並びの後始末: 足りない物を足す（`file` の読み込みが呼ぶ）。
    pub(crate) fn push_slot(&mut self, slot: ToolSlot) {
        self.slots.push(slot);
    }

    /// 今のツールを、ブラシのツールの最初の 1 つにする（読み込みのあと）。
    pub(crate) fn activate_first(&mut self, tool: Tool) {
        self.active = self.first_of(tool);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brushes::builtin;

    fn initial() -> ToolSet {
        ToolSet::initial(
            builtin::all()
                .iter()
                .map(|b| (BrushKey::Builtin(b.id), b.group)),
        )
    }

    #[test]
    fn the_initial_layout_follows_the_tool_table_and_the_built_in_groups() {
        let set = initial();
        let tools: Vec<Tool> = set.slots().iter().map(|s| s.tool).collect();
        assert_eq!(tools, Tool::ALL);
        for slot in set.slots() {
            assert_eq!(slot.gap, slot.tool.def().starts_group, "{:?}", slot.tool);
            assert_eq!(slot.groups.is_empty(), !slot.holds_brushes());
        }
        let brush = &set.slots()[0];
        let names: Vec<Option<Group>> = brush.groups.iter().map(|g| g.builtin).collect();
        assert_eq!(
            names,
            BRUSH_GROUPS.iter().copied().map(Some).collect::<Vec<_>>(),
            "取り込みのグループは、取り込んだブラシが無ければ作らない"
        );
        let eraser = &set.slots()[1];
        assert_eq!(eraser.groups.len(), 1);
        assert_eq!(
            eraser.groups[0].brushes[0],
            BrushKey::Builtin(builtin::STANDARD_ERASER)
        );
        // 組み込みは全部、そのグループに 1 つずつ
        for b in builtin::all() {
            let p = set.find(BrushKey::Builtin(b.id)).expect(b.id);
            assert_eq!(set.slots()[p.slot].groups[p.group].builtin, Some(b.group));
        }
        assert_eq!(set.brushes().len(), builtin::all().len());
        assert_eq!(set.active(), set.first_of(Tool::Brush));
    }

    #[test]
    fn imported_brushes_get_their_group_at_the_end_of_the_brush_tool() {
        let set = ToolSet::initial([
            (BrushKey::User(3), Group::Imported),
            (BrushKey::User(1), Group::Pen),
            (BrushKey::User(2), Group::Eraser),
        ]);
        let brush = &set.slots()[0];
        let last = brush.groups.last().unwrap();
        assert_eq!(last.builtin, Some(Group::Imported));
        assert_eq!(last.brushes, [BrushKey::User(3)]);
        assert_eq!(brush.groups[0].brushes, [BrushKey::User(1)]);
        assert_eq!(set.slots()[1].groups[0].brushes, [BrushKey::User(2)]);
    }

    #[test]
    fn a_full_group_continues_in_a_new_group_of_the_same_built_in_kind() {
        let count = MAX_GROUP_BRUSHES * 2 + 3;
        let users = (1..=count as u32).map(|n| {
            let group = if n % 2 == 0 {
                Group::Pen
            } else {
                Group::Imported
            };
            (BrushKey::User(n), group)
        });
        let built_in = builtin::all()
            .iter()
            .map(|b| (BrushKey::Builtin(b.id), b.group));
        let set = ToolSet::initial(built_in.chain(users));
        let brush = &set.slots()[0];
        let sizes = |kind: Group| -> Vec<usize> {
            brush
                .groups
                .iter()
                .filter(|g| g.builtin == Some(kind))
                .map(|g| g.file_len())
                .collect()
        };
        let pens = builtin::all()
            .iter()
            .filter(|b| b.group == Group::Pen)
            .count();
        assert_eq!(
            sizes(Group::Pen),
            [MAX_GROUP_BRUSHES, pens + count / 2 - MAX_GROUP_BRUSHES],
            "組み込みのブラシのあとに利用者のブラシが続き、いっぱいになったら次のグループ"
        );
        assert_eq!(
            sizes(Group::Imported),
            [MAX_GROUP_BRUSHES, count / 2 + 1 - MAX_GROUP_BRUSHES]
        );
        let order: Vec<Option<Group>> = brush.groups.iter().map(|g| g.builtin).collect();
        assert_eq!(
            &order[..2],
            [Some(Group::Pen), Some(Group::Pen)],
            "続きは同じ組み込みのグループの次"
        );
        assert_eq!(set.brushes().len(), builtin::all().len() + count);
    }

    #[test]
    fn brushes_that_find_no_room_in_the_groups_are_left_out_of_the_layout() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let si = set.slot_index(brush).unwrap();
        let mut n = 0u32;
        while set.slots()[si].groups.len() < MAX_GROUPS {
            let mut group = set.new_group(Some(Group::Imported), None, Vec::new());
            group.brushes = (0..MAX_GROUP_BRUSHES)
                .map(|_| {
                    n += 1;
                    BrushKey::User(n)
                })
                .collect();
            set.slots[si].groups.push(group);
        }
        let before = set.clone();
        assert_eq!(
            set.builtin_group_with_room(Group::Imported),
            None,
            "グループが上限なら並びに置かない（ファイルだけ残る）"
        );
        assert_eq!(set, before, "上限のときはグループを足さない");
        // ブラシのツールがいっぱいでも、消しゴムのツールのグループは別
        assert!(set.builtin_group_with_room(Group::Eraser).is_some());
    }

    #[test]
    fn the_target_group_falls_back_to_the_first_group_with_room_when_the_built_in_group_is_gone() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let airbrush = set
            .slot(brush)
            .unwrap()
            .groups
            .iter()
            .find(|g| g.builtin == Some(Group::Airbrush))
            .unwrap()
            .id;
        set.remove_group(airbrush).unwrap();
        let first = set.slot(brush).unwrap().groups[0].id;
        assert_eq!(set.builtin_group_with_room(Group::Airbrush), Some(first));
        set.group_mut(first).unwrap().brushes =
            (1..=MAX_GROUP_BRUSHES as u32).map(BrushKey::User).collect();
        let second = set.slot(brush).unwrap().groups[1].id;
        assert_eq!(
            set.builtin_group_with_room(Group::Airbrush),
            Some(second),
            "最初のグループがいっぱいなら、空きのある次のグループ"
        );
    }

    #[test]
    fn tools_are_added_moved_renamed_and_removed_but_the_last_brush_tool_stays() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let fill = set.first_of(Tool::Fill).unwrap();
        // 組み込みのツールは列に 1 つまで
        assert_eq!(
            set.add_slot(Tool::Fill, None, None, None),
            Err(Refusal::AlreadyPlaced(Tool::Fill))
        );
        let ink = set
            .add_slot(
                Tool::Brush,
                Some(fill),
                Some("インク".into()),
                Some("グループ".into()),
            )
            .unwrap();
        assert_eq!(set.slot_index(ink), Some(2));
        let s = set.slot(ink).unwrap();
        assert_eq!(s.groups.len(), 1);
        assert!(s.groups[0].brushes.is_empty());
        assert_eq!(s.name_in(Lang::En), "インク");
        // 並べ替え
        assert!(set.move_slot(ink, Some(brush)).unwrap());
        assert_eq!(set.slot_index(ink), Some(0));
        assert!(!set.move_slot(ink, Some(brush)).unwrap(), "同じ場所");
        // 名前: ツールの表の名前と同じなら名前を持たない
        assert!(set.rename_slot(brush, Some("  線画  ")).unwrap());
        assert_eq!(set.slot(brush).unwrap().name.as_deref(), Some("線画"));
        assert!(set.rename_slot(brush, Some("Brush")).unwrap());
        assert_eq!(set.slot(brush).unwrap().name, None);
        assert_eq!(set.slot(brush).unwrap().name_in(Lang::Ja), "ブラシ");
        // アイコン
        assert!(set.set_icon(ink, Some("stylus")).unwrap());
        assert_eq!(set.slot(ink).unwrap().icon().normal, "stylus");
        assert!(
            set.set_icon(ink, Some("brush")).unwrap(),
            "ツールのアイコンに戻す"
        );
        assert_eq!(set.slot(ink).unwrap().icon, None);
        assert!(!set.set_icon(ink, Some("no-such-icon")).unwrap());
        // 外す。ブラシのツールの最後の 1 つは外せない
        let moved = set.remove_slot(brush).unwrap();
        assert_eq!(
            moved.len(),
            builtin::all().len() - 3,
            "消しゴムの 3 つは残る"
        );
        assert_eq!(set.remove_slot(ink), Err(Refusal::LastOfKind(Tool::Brush)));
        let eraser = set.first_of(Tool::Eraser).unwrap();
        assert_eq!(
            set.remove_slot(eraser),
            Err(Refusal::LastOfKind(Tool::Eraser))
        );
        // 外した組み込みのツールは、足し直せる
        set.remove_slot(fill).unwrap();
        assert_eq!(set.removed_tools(), [Tool::Fill]);
        set.add_slot(Tool::Fill, None, None, None).unwrap();
        assert!(set.removed_tools().is_empty());
    }

    #[test]
    fn removing_a_tool_keeps_its_separator_on_the_next_tool() {
        let mut set = initial();
        let rect = set.first_of(Tool::SelectRect).unwrap();
        assert!(set.slot(rect).unwrap().gap);
        let at = set.slot_index(rect).unwrap();
        set.remove_slot(rect).unwrap();
        assert!(set.slots()[at].gap);
        set.toggle_gap(set.slots()[at].id).unwrap();
        assert!(!set.slots()[at].gap);
    }

    #[test]
    fn groups_are_added_renamed_moved_between_tools_and_removed() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let eraser = set.first_of(Tool::Eraser).unwrap();
        let pen = set.slot(brush).unwrap().groups[0].id;
        let ink = set
            .add_group(brush, Some(pen), "インク".into(), Vec::new())
            .unwrap();
        assert_eq!(set.slot(brush).unwrap().groups[0].id, ink);
        // 名前: 組み込みの名前と同じなら名前を持たない
        assert!(set.rename_group(pen, "Pen").is_ok_and(|c| !c));
        assert!(set.rename_group(pen, "線画").unwrap());
        assert_eq!(set.group(pen).unwrap().1.name_in(Lang::En), "線画");
        assert!(set.rename_group(pen, "ペン").unwrap());
        assert_eq!(set.group(pen).unwrap().1.name_in(Lang::En), "Pen");
        assert!(!set.rename_group(pen, "  ").unwrap(), "空の名前は変えない");
        // 別のツールへ動かす（中のブラシも一緒に。消すかどうかはツールが決める）
        let first_pen = set.group(pen).unwrap().1.brushes[0];
        assert!(set.move_group(pen, eraser, None).unwrap());
        assert_eq!(set.slot_of_group(pen), Some(eraser));
        assert_eq!(set.slot_of(first_pen), Some(eraser));
        // グループを持てないツールへは動かせない
        let fill = set.first_of(Tool::Fill).unwrap();
        assert_eq!(set.move_group(pen, fill, None), Err(Refusal::NoGroups));
        assert_eq!(
            set.add_group(fill, None, "x".into(), Vec::new()),
            Err(Refusal::NoGroups)
        );
        // 外すと、中のブラシは並びから外れる
        set.show_group(pen);
        let out = set.remove_group(pen).unwrap();
        assert!(out.contains(&first_pen));
        assert!(!set.contains(first_pen));
        assert_ne!(set.shown_group(eraser), Some(pen));
    }

    #[test]
    fn brushes_move_within_and_between_groups_and_stay_unique() {
        let mut set = initial();
        let brush = set.first_of(Tool::Brush).unwrap();
        let groups: Vec<GroupId> = set
            .slot(brush)
            .unwrap()
            .groups
            .iter()
            .map(|g| g.id)
            .collect();
        let pencil = BrushKey::Builtin("pencil");
        let standard = BrushKey::Builtin(builtin::STANDARD);
        assert!(set.move_brush(pencil, groups[0], Some(standard)).unwrap());
        assert_eq!(set.group(groups[0]).unwrap().1.brushes[0], pencil);
        assert!(set.move_brush(pencil, groups[1], None).unwrap());
        assert_eq!(set.group_of(pencil), Some(groups[1]));
        assert_eq!(
            set.group(groups[1]).unwrap().1.brushes.last(),
            Some(&pencil)
        );
        assert!(!set.move_brush(pencil, groups[1], Some(pencil)).unwrap());
        // 並びにある物は 2 つ目を置けない（写しは `brushes` がファイルで作る）
        assert!(set.insert_brush(pencil, groups[0], None).is_err());
        assert!(set.remove_brush(pencil).unwrap());
        assert!(!set.remove_brush(pencil).unwrap());
        set.insert_brush(pencil, groups[0], Some(standard)).unwrap();
        assert_eq!(set.group(groups[0]).unwrap().1.brushes[0], pencil);
        // 参照を写しに替える（同じ場所）
        set.note_used(pencil);
        assert!(set.replace_brush(pencil, BrushKey::User(9)));
        assert_eq!(
            set.group(groups[0]).unwrap().1.brushes[0],
            BrushKey::User(9)
        );
        assert_eq!(set.last(brush), Some(BrushKey::User(9)));
    }

    #[test]
    fn limits_are_kept() {
        let mut set = initial();
        while set.slots().len() < MAX_TOOLS {
            set.add_slot(Tool::Brush, None, None, None).unwrap();
        }
        assert_eq!(
            set.add_slot(Tool::Eraser, None, None, None),
            Err(Refusal::TooManyTools)
        );
        let brush = set.first_of(Tool::Brush).unwrap();
        while set.slot(brush).unwrap().groups.len() < MAX_GROUPS {
            set.add_group(brush, None, "g".into(), Vec::new()).unwrap();
        }
        assert_eq!(
            set.add_group(brush, None, "g".into(), Vec::new()),
            Err(Refusal::TooManyGroups)
        );
        let eraser = set.first_of(Tool::Eraser).unwrap();
        let eraser_group = set.slot(eraser).unwrap().groups[0].id;
        assert_eq!(
            set.move_group(eraser_group, brush, None),
            Err(Refusal::TooManyGroups)
        );
        let full: Vec<BrushKey> = (1..=MAX_GROUP_BRUSHES as u32).map(BrushKey::User).collect();
        let g = set.add_group(eraser, None, "full".into(), full).unwrap();
        assert_eq!(
            set.insert_brush(BrushKey::User(9999), g, None),
            Err(Refusal::TooManyBrushes)
        );
        assert_eq!(
            set.move_brush(BrushKey::Builtin(builtin::STANDARD_ERASER), g, None),
            Err(Refusal::TooManyBrushes)
        );
    }

    #[test]
    fn a_locked_layout_refuses_every_change() {
        let mut set = initial();
        set.locked = Some(Lock::Newer(2));
        let brush = set.first_of(Tool::Brush).unwrap();
        let group = set.slot(brush).unwrap().groups[0].id;
        assert_eq!(
            set.add_slot(Tool::Brush, None, None, None),
            Err(Refusal::Newer(2))
        );
        assert_eq!(set.rename_slot(brush, Some("x")), Err(Refusal::Newer(2)));
        assert_eq!(set.toggle_gap(brush), Err(Refusal::Newer(2)));
        assert_eq!(set.rename_group(group, "x"), Err(Refusal::Newer(2)));
        assert_eq!(
            set.remove_brush(BrushKey::Builtin("pencil")),
            Err(Refusal::Newer(2))
        );
        assert_eq!(initial(), set, "何も変わらない");
    }

    #[test]
    fn switching_to_a_tool_keeps_the_current_one_of_the_same_kind() {
        let mut set = initial();
        let first = set.first_of(Tool::Brush).unwrap();
        let second = set.add_slot(Tool::Brush, None, None, None).unwrap();
        assert_eq!(set.slot_for_tool(Tool::Brush), Some(first));
        set.set_active(Some(second));
        assert_eq!(set.slot_for_tool(Tool::Brush), Some(second));
        assert_eq!(set.slot_for_tool(Tool::Fill), set.first_of(Tool::Fill));
        // ツールに替えたときのブラシ: 最後のブラシ、なければ出しているグループの先頭
        assert_eq!(
            set.pick_for(first),
            Some(BrushKey::Builtin(builtin::STANDARD))
        );
        assert_eq!(set.pick_for(second), None, "空のツール");
        let pencil = BrushKey::Builtin("pencil");
        set.note_used(pencil);
        assert_eq!(set.pick_for(first), Some(pencil));
        let group = set.group_of(pencil).unwrap();
        assert_eq!(set.shown_group(first), Some(group));
        let target = set.slot(second).unwrap().groups[0].id;
        set.move_brush(pencil, target, None).unwrap();
        assert_eq!(set.pick_for(second), Some(pencil));
        assert_ne!(
            set.pick_for(first),
            Some(pencil),
            "動いたブラシは前のツールの最後ではない"
        );
    }
}
