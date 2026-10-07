//! ツールの並びの操作（`Action::Tools`）と、`tools.json` の読み込み・保存・最初の並びに戻す。ブラシを並べる・写す操作は
//! `brushes`（`BrushAction`）の側。並びの操作は文書ではない（取り消しの段に入れない）。描いている間・ブラシを取り込んでいる間は断る。

use std::path::{Path, PathBuf};

use super::file::{self, Broken, Decoded};
use super::{GroupId, Refusal, SlotId, ToolSet};
use crate::brushes::{builtin, BrushKey, Group};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{AppState, Tool};
use crate::userfiles::{self, FileError};

/// ツールの並びの操作。
#[derive(Clone, Debug, PartialEq)]
pub enum ToolsetAction {
    /// ツールの列の 1 つに替える。
    Select(SlotId),
    /// ツールを `after` の後ろ（None なら最後）に足す。ブラシ・消しゴムは新しいツール（空のグループ 1 つで始まる）、ほかは外した組み込みのツールを戻す。
    Add {
        tool: Tool,
        after: Option<SlotId>,
    },
    StartRename(SlotId),
    /// 名前を変える（空ならツールの表の名前へ戻す）。
    Rename(SlotId, String),
    /// アイコンを選ぶ格子を開く・閉じる。
    PickIcon(Option<SlotId>),
    SetIcon(SlotId, &'static str),
    /// ツールの前の区切りを入れる・外す。
    ToggleGap(SlotId),
    Remove(SlotId),
    Move {
        slot: SlotId,
        before: Option<SlotId>,
    },
    /// グループをツールの最後に足す。
    AddGroup(SlotId),
    ShowGroup(GroupId),
    StartRenameGroup(GroupId),
    RenameGroup(GroupId, String),
    /// グループの写し（中のブラシも写す）をすぐ後ろに作る。
    DuplicateGroup(GroupId),
    RemoveGroup(GroupId),
    MoveGroup {
        group: GroupId,
        to: SlotId,
        before: Option<GroupId>,
    },
    /// 最初の並びに戻す前に確かめる（ウィンドウの頼み）。
    ResetDialog,
    /// 最初の並びに戻す（利用者のブラシのファイルは消さず、元のグループへ並べ直す）。
    Reset,
}

/// 起動のとき読めなかった理由（知らせる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadProblem {
    /// 読めない所があった（初めの並びに戻し、元のファイルを `moved_to` へ退避した。退避できなければ None で、並びを変える操作を断る）。
    Broken {
        reason: Broken,
        moved_to: Option<String>,
    },
    /// 読み込めなかった（OS の誤り。並びを変える操作を断る）。
    Unreadable(String),
    /// 今の版より新しい（触らずに初めの並びで動く）。
    Newer(u64),
}

/// ツールの並びの状態の全部。
#[derive(Default)]
pub struct ToolsetState {
    pub set: ToolSet,
    /// `tools.json` の場所（無ければ保存しない）。
    pub path: Option<PathBuf>,
    pub problem: Option<LoadProblem>,
    pub ui: super::ui::ToolsetUi,
    pub catalog: super::catalog::Catalog,
}

impl Refusal {
    pub fn describe(&self, lang: Lang) -> String {
        match self {
            Refusal::Newer(_) => lang
                .pick(
                    "ツールの並びのファイルが新しい版のものです",
                    "the tool layout file is from a newer version",
                )
                .into(),
            Refusal::Unreadable => lang
                .pick(
                    "ツールの並びのファイルを読めませんでした",
                    "the tool layout file could not be read",
                )
                .into(),
            Refusal::TooManyTools => lang.pick(
                format!("ツールは {} 個までです", super::MAX_TOOLS),
                format!("at most {} tools", super::MAX_TOOLS),
            ),
            Refusal::TooManyGroups => lang.pick(
                format!("グループはツールごとに {} 個までです", super::MAX_GROUPS),
                format!("at most {} groups per tool", super::MAX_GROUPS),
            ),
            Refusal::TooManyBrushes => lang.pick(
                format!(
                    "1 つのグループのブラシは {} 個までです",
                    super::MAX_GROUP_BRUSHES
                ),
                format!("at most {} brushes per group", super::MAX_GROUP_BRUSHES),
            ),
            Refusal::LastOfKind(tool) => lang.pick(
                format!("{}ツールは 1 つ残します", tool.name_in(lang)),
                format!("one {} tool stays", tool.name_in(lang).to_lowercase()),
            ),
            Refusal::NoGroups => lang
                .pick(
                    "このツールはブラシのグループを持てません",
                    "this tool has no brush groups",
                )
                .into(),
            Refusal::AlreadyPlaced(tool) => lang.pick(
                format!("{}はもうツールバーにあります", tool.name_in(lang)),
                format!("{} is already in the toolbar", tool.name_in(lang)),
            ),
            Refusal::Missing => String::new(),
        }
    }
}

impl LoadProblem {
    pub fn describe(&self, lang: Lang) -> String {
        let file = lang.quote(file::FILE_NAME);
        match self {
            LoadProblem::Broken { reason, moved_to } => {
                let what = lang.pick(
                    format!("ツールの並びのファイル{file}を読めないので、最初の並びにしました"),
                    format!("Cannot read the tool layout file {file}; using the initial layout"),
                );
                let reason = reason.describe(lang);
                let mut text = lang.with_reason(what, reason);
                match moved_to {
                    Some(name) => text.push_str(&lang.pick(
                        format!("元のファイルは{}に残しました。", lang.quote(name)),
                        format!(" The original was kept as {}.", lang.quote(name)),
                    )),
                    None => text.push_str(lang.pick(
                        "元のファイルを残せないので、並びは保存しません。",
                        " The original could not be kept, so the layout is not saved.",
                    )),
                }
                text
            }
            LoadProblem::Unreadable(reason) => lang.with_reason(
                lang.pick(
                    format!("ツールの並びのファイル{file}を読めません"),
                    format!("Cannot read the tool layout file {file}"),
                ),
                reason,
            ),
            LoadProblem::Newer(_) => lang.with_reason(
                lang.pick(
                    format!("ツールの並びのファイル{file}は新しい版のものなので、変えずに最初の並びで使います"),
                    format!("The tool layout file {file} is from a newer version; it is left as is and the initial layout is used"),
                ),
                "",
            ),
        }
    }
}

/// 読めないファイルを、同じフォルダの `tools.broken.json`（あれば `tools.broken-2.json` …）へ名前を変えて退避する。退避した名前。
fn move_aside(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    for n in 1..1000 {
        let name = if n == 1 {
            "tools.broken.json".to_owned()
        } else {
            format!("tools.broken-{n}.json")
        };
        let target = dir.join(&name);
        if std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        return std::fs::rename(path, &target).ok().map(|_| name);
    }
    None
}

impl AppState {
    /// 組み込み（組み込みの並び）と利用者のブラシ（番号の順）を元のグループへ置いた、最初の並び。
    pub(crate) fn toolset_initial(&self) -> ToolSet {
        let builtins = builtin::all()
            .iter()
            .map(|b| (BrushKey::Builtin(b.id), b.group));
        let mut users: Vec<(u32, Group)> = self
            .brushes
            .lib
            .entries()
            .iter()
            .filter_map(|e| match e.key {
                BrushKey::User(id) => Some((id, e.group)),
                BrushKey::Builtin(_) => None,
            })
            .collect();
        users.sort_by_key(|(id, _)| *id);
        ToolSet::initial(builtins.chain(users.into_iter().map(|(id, g)| (BrushKey::User(id), g))))
    }

    /// 設定のフォルダの `tools.json` を読む（起動のとき 1 回。利用者のブラシを読んだ後）。無ければ、今までのブラシの並び（`order.conf` と
    /// ファイルの `group=`）から今と同じ並びを作る。読めなければ初めの並びにして、理由を残す（`toolset_problem_message`）。
    pub fn attach_toolset(&mut self, path: PathBuf) {
        let users: Vec<(u32, Group)> = self
            .brushes
            .lib
            .entries()
            .iter()
            .filter_map(|e| match e.key {
                BrushKey::User(id) => Some((id, e.group)),
                BrushKey::Builtin(_) => None,
            })
            .collect();
        // 今までの並び（一覧の並びは order.conf のとおり）
        let migrated =
            ToolSet::initial(self.brushes.lib.entries().iter().map(|e| (e.key, e.group)));
        let (set, problem) = match userfiles::read_checked(&path, file::MAX_FILE_BYTES) {
            Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => (migrated, None),
            Err(FileError::Io(e)) => {
                let mut set = migrated;
                set.locked = Some(super::Lock::Unreadable);
                (set, Some(LoadProblem::Unreadable(self.lang.file_error(&e))))
            }
            Err(FileError::TooLarge) => self.toolset_broken(&path, Broken::TooLarge, migrated),
            Err(FileError::NotText | FileError::Mismatch) => {
                self.toolset_broken(&path, Broken::NotJson, migrated)
            }
            Ok(text) => match file::decode(&text, &|id| self.toolset_user_file(id)) {
                Ok(Decoded::Ok(mut read)) => {
                    file::add_missing(&mut read, &users);
                    (read.set, None)
                }
                Ok(Decoded::Newer(version)) => {
                    let mut set = migrated;
                    set.locked = Some(super::Lock::Newer(version));
                    (set, Some(LoadProblem::Newer(version)))
                }
                Err(reason) => self.toolset_broken(&path, reason, migrated),
            },
        };
        self.toolset.set = set;
        self.toolset.path = Some(path);
        self.toolset.problem = problem;
        // 今のブラシ・ツールを、読んだ並びに合わせる
        self.toolset_follow_current();
    }

    fn toolset_broken(
        &self,
        path: &Path,
        reason: Broken,
        mut set: ToolSet,
    ) -> (ToolSet, Option<LoadProblem>) {
        let moved_to = move_aside(path);
        if moved_to.is_none() {
            set.locked = Some(super::Lock::Unreadable);
        }
        (set, Some(LoadProblem::Broken { reason, moved_to }))
    }

    /// 並びを読み直した・戻したあと: 今のツールを同じツールの列の 1 つにし、今のブラシのツールの覚えを付け直す。
    pub(crate) fn toolset_follow_current(&mut self) {
        let tool = self.tool;
        let slot = self
            .toolset
            .set
            .slot_of(self.brushes.lib.current())
            .filter(|s| self.toolset.set.slot(*s).is_some_and(|s| s.tool == tool))
            .or_else(|| self.toolset.set.slot_for_tool(tool));
        self.toolset.set.set_active(slot);
        let current = self.brushes.lib.current();
        self.toolset.set.note_used(current);
    }

    /// 起動のとき読めなかったツールの並びの知らせ（無ければ None）。
    pub fn toolset_problem_message(&self) -> Option<String> {
        self.toolset.problem.as_ref().map(|p| p.describe(self.lang))
    }

    /// 利用者のブラシのファイルの番号の、起動のときの状態（読めた・フォルダにあるが読めなかった・無い）。
    fn toolset_user_file(&self, id: u32) -> file::UserFile {
        if self.brushes.lib.entry(BrushKey::User(id)).is_some() {
            file::UserFile::Loaded
        } else if self.brushes.unloaded.contains(&id) {
            file::UserFile::Unloaded
        } else {
            file::UserFile::Missing
        }
    }

    /// 並びのファイルに書く利用者のブラシの番号の最大（起動のときにあったファイルと、一覧の利用者のブラシの番号）。
    fn toolset_user_through(&self) -> u32 {
        let listed = self
            .brushes
            .lib
            .entries()
            .iter()
            .filter_map(|e| match e.key {
                BrushKey::User(id) => Some(id),
                BrushKey::Builtin(_) => None,
            })
            .max()
            .unwrap_or(0);
        listed.max(self.brushes.loaded_through)
    }

    /// 並びを書く（知らせない）。書けなければ理由の文。
    pub(crate) fn toolset_save(&mut self) -> Result<(), String> {
        let Some(path) = self.toolset.path.clone() else {
            return Ok(());
        };
        if self.toolset.set.locked.is_some() {
            return Ok(());
        }
        let text = file::encode(&self.toolset.set, self.toolset_user_through());
        let expect = self.toolset.set.clone();
        let verify = |read: &str| {
            matches!(
                file::decode(read, &|id| self.toolset_user_file(id)),
                Ok(Decoded::Ok(r)) if r.set == expect
            )
        };
        userfiles::write_text(&path, &text, file::MAX_FILE_BYTES, verify).map_err(|e| {
            let reason = match e {
                FileError::Io(e) => self.lang.file_error(&e),
                FileError::TooLarge => self.lang.pick("大きすぎます", "too large").into(),
                FileError::NotText | FileError::Mismatch => self
                    .lang
                    .pick(
                        "書いた中身を確かめられません",
                        "the written file did not match",
                    )
                    .into(),
            };
            self.lang.with_reason(
                self.lang.pick(
                    "ツールの並びを保存できません",
                    "Cannot save the tool layout",
                ),
                reason,
            )
        })
    }

    /// 並びを書く（失敗は知らせる）。書けた（書く必要が無かった）なら true。
    pub(crate) fn toolset_persist(&mut self) -> bool {
        match self.toolset_save() {
            Ok(()) => true,
            Err(text) => {
                self.fail(Source::Brush, text);
                false
            }
        }
    }

    /// 並びを変える操作を断る理由を知らせる。
    pub(crate) fn toolset_refuse(&mut self, refusal: Refusal) {
        if refusal == Refusal::Missing {
            return;
        }
        let lang = self.lang;
        let text = lang.with_reason(
            lang.pick(
                "ツールの並びを変えられません",
                "Cannot change the tool layout",
            ),
            refusal.describe(lang),
        );
        self.refuse(Source::Brush, text);
    }

    /// 並びを変える前に、描いている間・取り込み中を断る。断ったなら true。
    pub(crate) fn toolset_blocked(&mut self) -> bool {
        if self.is_stroking() {
            self.refuse(
                Source::Brush,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return true;
        }
        self.brush_refuse_while_importing()
    }

    /// 結果を当てる: 変わったなら書く、断られたなら知らせる。
    fn toolset_done(&mut self, result: Result<bool, Refusal>) -> bool {
        match result {
            Ok(true) => {
                self.toolset_persist();
                true
            }
            Ok(false) => false,
            Err(r) => {
                self.toolset_refuse(r);
                false
            }
        }
    }

    /// 組み込みのグループ `group` から作ったグループ（列の最初の物）のブラシ（並びの順）。
    pub fn brush_entries_in(&self, group: Group) -> Vec<&crate::brushes::Entry> {
        let set = &self.toolset.set;
        set.slots()
            .iter()
            .flat_map(|s| s.groups.iter())
            .find(|g| g.builtin == Some(group))
            .map(|g| {
                g.brushes
                    .iter()
                    .filter_map(|k| self.brushes.lib.entry(*k))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 今のツール（ブラシを持たないツールなら今のブラシのツール）で出しているグループの、元の組み込みのグループ（利用者が作ったグループなら None）。
    pub fn shown_brush_group(&self) -> Option<Group> {
        let set = &self.toolset.set;
        let slot = set
            .active_slot()
            .filter(|s| s.holds_brushes())
            .map(|s| s.id)
            .or_else(|| set.slot_of(self.brushes.lib.current()))?;
        let group = set.shown_group(slot)?;
        set.group(group).and_then(|(_, g)| g.builtin)
    }

    /// 組み込みのグループ `group` から作ったグループ（列の最初の物）を、そのツールで出す。
    pub fn show_brush_group(&mut self, group: Group) {
        let found = self
            .toolset
            .set
            .slots()
            .iter()
            .flat_map(|s| s.groups.iter())
            .find(|g| g.builtin == Some(group))
            .map(|g| g.id);
        if let Some(id) = found {
            self.toolset.set.show_group(id);
            self.brushes.ui.list_scroll = 0.0;
        }
    }

    /// ツールの列の 1 つに替える（`switch_tool` と同じ道。ブラシ・消しゴムのツールは、そのツールの最後のブラシへ）。
    pub fn select_slot(&mut self, slot: SlotId) -> bool {
        let Some(tool) = self.toolset.set.slot(slot).map(|s| s.tool) else {
            return false;
        };
        self.switch_to(tool, Some(slot), false)
    }

    pub fn toolset_action(&mut self, action: ToolsetAction) {
        let lang = self.lang;
        match action {
            ToolsetAction::Select(slot) => {
                self.select_slot(slot);
            }
            ToolsetAction::Add { tool, after } => {
                if self.toolset_blocked() {
                    return;
                }
                let set = &self.toolset.set;
                let before = after
                    .and_then(|a| set.slot_index(a))
                    .and_then(|i| set.slots().get(i + 1))
                    .map(|s| s.id);
                let (name, group) = if super::holds_brushes(tool) {
                    let base = tool.name_in(lang);
                    let taken = |n: &str| set.slots().iter().any(|s| s.name_in(lang) == n);
                    (
                        Some(userfiles::unused_name(base, taken)),
                        Some(lang.pick("グループ", "Group").to_owned()),
                    )
                } else {
                    (None, None)
                };
                match self.toolset.set.add_slot(tool, before, name, group) {
                    Ok(slot) => {
                        self.toolset_persist();
                        self.select_slot(slot);
                    }
                    Err(r) => self.toolset_refuse(r),
                }
            }
            ToolsetAction::StartRename(slot) => {
                if self.toolset.set.slot(slot).is_some() {
                    self.toolset.ui.renaming = Some(super::ui::Renaming::Slot(slot));
                    self.toolset.ui.rename_started = false;
                }
            }
            ToolsetAction::Rename(slot, name) => {
                if self.toolset.ui.renaming == Some(super::ui::Renaming::Slot(slot)) {
                    self.toolset.ui.renaming = None;
                }
                if self.toolset_blocked() {
                    return;
                }
                let name = name.trim();
                let result = self
                    .toolset
                    .set
                    .rename_slot(slot, (!name.is_empty()).then_some(name));
                self.toolset_done(result);
            }
            ToolsetAction::PickIcon(slot) => self.toolset.ui.icon_picker = slot,
            ToolsetAction::SetIcon(slot, key) => {
                self.toolset.ui.icon_picker = None;
                if self.toolset_blocked() {
                    return;
                }
                let result = self.toolset.set.set_icon(slot, Some(key));
                self.toolset_done(result);
            }
            ToolsetAction::ToggleGap(slot) => {
                if self.toolset_blocked() {
                    return;
                }
                let result = self.toolset.set.toggle_gap(slot).map(|_| true);
                self.toolset_done(result);
            }
            ToolsetAction::Remove(slot) => {
                if self.toolset_blocked() {
                    return;
                }
                let was_active = self.toolset.set.active() == Some(slot);
                let Some(tool) = self.toolset.set.slot(slot).map(|s| s.tool) else {
                    return;
                };
                let name = self
                    .toolset
                    .set
                    .slot(slot)
                    .map(|s| s.name_in(lang))
                    .unwrap_or_default();
                match self.toolset.set.remove_slot(slot) {
                    Ok(_) => {
                        // 今のツールを外したら、同じ種類のツール（無ければブラシ）へ
                        if was_active {
                            let next = self
                                .toolset
                                .set
                                .first_of(tool)
                                .or_else(|| self.toolset.set.first_of(Tool::Brush));
                            if let Some(next) = next {
                                self.select_slot(next);
                            }
                        }
                        self.info(
                            Source::Brush,
                            lang.pick(
                                format!("ツールを削除しました: {name}"),
                                format!("Tool deleted: {name}"),
                            ),
                        );
                        self.toolset_persist();
                    }
                    Err(r) => self.toolset_refuse(r),
                }
            }
            ToolsetAction::Move { slot, before } => {
                if self.toolset_blocked() {
                    return;
                }
                let result = self.toolset.set.move_slot(slot, before);
                self.toolset_done(result);
            }
            ToolsetAction::AddGroup(slot) => {
                if self.toolset_blocked() {
                    return;
                }
                let base = lang.pick("グループ", "Group");
                let name = match self.toolset.set.slot(slot) {
                    Some(s) => userfiles::unused_name(base, |n| {
                        s.groups.iter().any(|g| g.name_in(lang) == n)
                    }),
                    None => return,
                };
                match self.toolset.set.add_group(slot, None, name, Vec::new()) {
                    Ok(group) => {
                        self.toolset.set.show_group(group);
                        self.toolset_persist();
                        self.toolset.ui.renaming = Some(super::ui::Renaming::Group(group));
                        self.toolset.ui.rename_started = false;
                    }
                    Err(r) => self.toolset_refuse(r),
                }
            }
            ToolsetAction::ShowGroup(group) => {
                self.toolset.set.show_group(group);
                self.brushes.ui.list_scroll = 0.0;
            }
            ToolsetAction::StartRenameGroup(group) => {
                if self.toolset.set.group(group).is_some() {
                    self.toolset.ui.renaming = Some(super::ui::Renaming::Group(group));
                    self.toolset.ui.rename_started = false;
                }
            }
            ToolsetAction::RenameGroup(group, name) => {
                if self.toolset.ui.renaming == Some(super::ui::Renaming::Group(group)) {
                    self.toolset.ui.renaming = None;
                }
                if self.toolset_blocked() {
                    return;
                }
                let result = self.toolset.set.rename_group(group, &name);
                self.toolset_done(result);
            }
            ToolsetAction::DuplicateGroup(group) => self.brush_duplicate_group(group),
            ToolsetAction::RemoveGroup(group) => {
                if self.toolset_blocked() {
                    return;
                }
                let Some(name) = self.toolset.set.group(group).map(|(_, g)| g.name_in(lang)) else {
                    return;
                };
                match self.toolset.set.remove_group(group) {
                    Ok(_) => {
                        self.brush_after_unplaced();
                        self.info(
                            Source::Brush,
                            lang.pick(
                                format!("グループを削除しました: {name}"),
                                format!("Group deleted: {name}"),
                            ),
                        );
                        self.toolset_persist();
                    }
                    Err(r) => self.toolset_refuse(r),
                }
            }
            ToolsetAction::MoveGroup { group, to, before } => {
                if self.toolset_blocked() {
                    return;
                }
                let result = self.toolset.set.move_group(group, to, before);
                if self.toolset_done(result) {
                    self.toolset.set.show_group(group);
                    self.brush_after_unplaced();
                }
            }
            ToolsetAction::ResetDialog => {
                if self.toolset_blocked() {
                    return;
                }
                if let Some(r) = self.toolset.set.lock_refusal() {
                    return self.toolset_refuse(r);
                }
                self.dialog_request = Some(crate::state::DialogRequest::ToolsetReset);
            }
            ToolsetAction::Reset => {
                if self.toolset_blocked() {
                    return;
                }
                if let Some(r) = self.toolset.set.lock_refusal() {
                    return self.toolset_refuse(r);
                }
                let initial = self.toolset_initial();
                self.toolset.set.replace_with(initial);
                self.toolset.ui = Default::default();
                self.toolset_follow_current();
                self.brush_after_unplaced();
                self.info(
                    Source::Brush,
                    lang.pick(
                        "ツールの並びを最初の並びに戻しました。",
                        "The tool layout was reset.",
                    ),
                );
                self.toolset_persist();
            }
        }
    }
}
