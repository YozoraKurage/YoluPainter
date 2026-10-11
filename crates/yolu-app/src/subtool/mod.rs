//! サブツール: ツールごとの設定の組のプリセットの一覧（クリスタのサブツールに当たる）。ブラシと消しゴムの一覧は `brushes`（.ylbrush）、
//! 選択のツールは同じ並びのツールそのもの（`tools::SubTools::Tools`）で、ここはバケツ・ポリゴン塗りつぶし・グラデーション・図形・定規・スポイト・
//! 移動・ゆがみの設定の組を扱う。
//!
//! 設定の欄は宣言の表（`fields`: 名前・種類・既定・読む関数・書く関数）で、プリセットの値はその欄の並びの `Vec<Value>`。組み込みは
//! 「既定から変える欄」だけを持ち、利用者のプリセットは設定のフォルダにツールごと 1 ファイル（`store`）。一覧の 1 つ（`Entry`）は
//! 基準の設定（`baseline`）と、変えたままの設定（`edited`）を持つ（ブラシの一覧と同じ流儀）。今の設定は、これまでどおり各ツールの状態
//! （`AppState::region` など）が持ち、ツールを離れるとき今の設定を一覧の側へ書き戻し（`subtool_leave`）、入るとき今のサブツールの設定を
//! 今の設定へ写す（`subtool_enter`）。サブツールを替える操作は、押した行の設定を今の設定へ写す。
//! サブツールの設定は文書ではない（Undo に入れない）。ストロークの最中は、サブツールを替える・足す・消す操作を断る。

pub mod fields;
pub mod store;

use std::path::PathBuf;

use self::store::{Problem, SubToolStore, MAX_USER_PRESETS};
use crate::brushes::clean_name;
use crate::lang::Lang;
use crate::state::{AppState, Tool};

/// 欄の値。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Float(f32),
    /// 選び（名前で持つ）。
    Choice(&'static str),
}

/// 欄の種類（保存した値の検証にも使う）。
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool,
    /// 下限と上限（含む）。
    Int(i32, i32),
    Float(f32, f32),
    Choice(&'static [&'static str]),
}

/// 設定の 1 欄の宣言。
pub struct Field {
    /// 保存する名前。
    pub key: &'static str,
    pub kind: Kind,
    pub default: Value,
    /// 今の設定から読む。
    pub get: fn(&AppState) -> Value,
    /// 今の設定へ書く（範囲に収め、関連する状態も整える）。
    pub set: fn(&mut AppState, &Value),
}

/// プリセットの値（欄の並び）。
pub type Values = Vec<Value>;

/// 組み込みのプリセット（名前と、既定から変える欄）。
pub struct Builtin {
    pub id: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
    pub with: &'static [(&'static str, Value)],
}

/// 一覧の 1 つを指す印。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Builtin(&'static str),
    User(u32),
}

impl Key {
    pub fn is_user(self) -> bool {
        matches!(self, Key::User(_))
    }
}

/// 保存から読んだ利用者のプリセット 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct UserPreset {
    pub id: u32,
    pub name: String,
    pub values: Values,
}

/// 一覧の 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: Key,
    /// 利用者のプリセットの名前（組み込みは言語ごとの名前なので空）。
    pub name: String,
    /// 基準の設定（組み込みは出荷時、利用者のプリセットは登録した設定）。
    pub baseline: Values,
    /// 基準と違う設定のまま使っている間の設定（基準と同じなら None）。
    pub edited: Option<Values>,
}

impl Entry {
    /// 今使う設定（変えていればそれ、なければ基準）。
    pub fn effective(&self) -> &Values {
        self.edited.as_ref().unwrap_or(&self.baseline)
    }
}

/// ツールの欄の既定の値。
pub fn defaults(tool: Tool) -> Values {
    fields::fields(tool)
        .iter()
        .map(|f| f.default.clone())
        .collect()
}

/// 組み込みのプリセットの値（既定に、変える欄を重ねる）。
fn builtin_values(tool: Tool, builtin: &Builtin) -> Values {
    let table = fields::fields(tool);
    let mut values = defaults(tool);
    for (key, value) in builtin.with {
        if let Some(at) = table.iter().position(|f| f.key == *key) {
            values[at] = value.clone();
        }
    }
    values
}

/// ツールごとのプリセットの一覧。
pub struct PresetList {
    tool: Tool,
    entries: Vec<Entry>,
    current: Key,
    /// 次に付ける利用者のプリセットの番号。
    next_id: u32,
}

impl PresetList {
    /// 組み込みと、読んだ利用者のプリセット（番号の順）。
    pub fn new(tool: Tool, users: Vec<UserPreset>) -> PresetList {
        let mut entries: Vec<Entry> = fields::builtins(tool)
            .iter()
            .map(|b| Entry {
                key: Key::Builtin(b.id),
                name: String::new(),
                baseline: builtin_values(tool, b),
                edited: None,
            })
            .collect();
        let mut users = users;
        users.sort_by_key(|u| u.id);
        let next_id = users
            .iter()
            .map(|u| u.id)
            .max()
            .map_or(1, |m| m.saturating_add(1));
        for u in users {
            entries.push(Entry {
                key: Key::User(u.id),
                name: u.name,
                baseline: u.values,
                edited: None,
            });
        }
        // 初めの選びは、既定の設定と同じ組み込み（無ければ先頭）
        let start = defaults(tool);
        let current = entries
            .iter()
            .find(|e| e.baseline == start)
            .or_else(|| entries.first())
            .map_or(Key::User(0), |e| e.key);
        PresetList {
            tool,
            entries,
            current,
            next_id,
        }
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, key: Key) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    fn entry_mut(&mut self, key: Key) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.key == key)
    }

    /// 今のサブツール。
    pub fn current(&self) -> Key {
        self.current
    }

    /// 名前（組み込みは言語ごと）。
    pub fn name_in(&self, key: Key, lang: Lang) -> String {
        match key {
            Key::Builtin(id) => fields::builtins(self.tool)
                .iter()
                .find(|b| b.id == id)
                .map(|b| lang.pick(b.ja, b.en).to_owned())
                .unwrap_or_default(),
            Key::User(_) => self.entry(key).map(|e| e.name.clone()).unwrap_or_default(),
        }
    }

    /// 今のサブツールの設定が基準と違うか（`live` は今の設定）。ほかのサブツールは覚えている変更で見る。
    pub fn is_modified(&self, key: Key, live: &Values) -> bool {
        crate::userfiles::is_modified(
            self.entry(key).map(|e| (&e.baseline, e.edited.is_some())),
            key == self.current,
            live,
        )
    }

    /// 保存する利用者のプリセット（基準の設定。番号の順）。
    pub fn users(&self) -> Vec<UserPreset> {
        self.entries
            .iter()
            .filter_map(|e| match e.key {
                Key::User(id) => Some(UserPreset {
                    id,
                    name: e.name.clone(),
                    values: e.baseline.clone(),
                }),
                Key::Builtin(_) => None,
            })
            .collect()
    }

    pub fn user_count(&self) -> usize {
        self.entries.iter().filter(|e| e.key.is_user()).count()
    }

    fn unused_name(&self, base: &str) -> String {
        crate::userfiles::unused_name(base, |name| {
            self.entries
                .iter()
                .any(|e| e.key.is_user() && e.name == name)
        })
    }
}

/// 一覧の画面だけの状態。
#[derive(Default)]
pub struct SubToolUi {
    /// 一覧のスクロールと、前のフレームの中身の高さ。
    pub list_scroll: f32,
    pub list_content: f32,
    /// 名前を変えているサブツールと、入力欄がフォーカスを取った後か。
    pub renaming: Option<(Tool, Key)>,
    pub rename_started: bool,
    /// 右クリックのメニューの対象。
    pub context: Option<(Tool, Key)>,
    /// 次に一覧を描くとき、今のサブツールの行が見えるところまでスクロールする。
    pub reveal: bool,
    /// 前のフレームに描いたパネルの右端（ブラシの詳細のウィンドウを、その右に置く）。描いていなければ 0。
    pub panel_right: f32,
    /// ツールプロパティのパネルの、前のフレームの中身の高さ（ツールごと。ツールを替えた最初のフレームから、そのツールの高さで組み立てる）。
    pub props_content: [f32; Tool::ALL.len()],
    /// ツールプロパティのパネルのスクロール（ツールごと）。
    pub props_scroll: [f32; Tool::ALL.len()],
    /// ブラシサイズのパネルのスクロール。
    pub sizes_scroll: f32,
}

/// サブツールの状態の全部。
pub struct SubToolState {
    pub lists: Vec<PresetList>,
    pub store: Option<SubToolStore>,
    /// 起動のとき読めなかったファイル。
    pub problems: Vec<Problem>,
    pub ui: SubToolUi,
}

impl Default for SubToolState {
    fn default() -> Self {
        SubToolState {
            lists: fields::TOOLS
                .iter()
                .map(|t| PresetList::new(*t, Vec::new()))
                .collect(),
            store: None,
            problems: Vec::new(),
            ui: SubToolUi::default(),
        }
    }
}

/// 一覧の操作（ボタン・右クリックから）。
#[derive(Clone, Debug, PartialEq)]
pub enum SubToolAction {
    /// このサブツールに替える（設定が今の設定になる）。
    Select(Tool, Key),
    /// 今の設定を新しいサブツールとして足す。
    Add(Tool),
    /// このサブツール（今のサブツールなら今の設定）を、すぐ後ろへ複製する。
    Duplicate(Tool, Key),
    Delete(Tool, Key),
    StartRename(Tool, Key),
    Rename(Tool, Key, String),
    /// 基準の設定へ戻す。
    Revert(Tool, Key),
    /// 今の設定を、そのサブツールの基準として登録する（利用者のものだけ）。
    Register(Tool, Key),
}

impl AppState {
    /// ツールのプリセットの一覧（プリセットのツールでなければ None）。
    pub fn subtool_list(&self, tool: Tool) -> Option<&PresetList> {
        self.subtools.lists.iter().find(|l| l.tool == tool)
    }

    fn subtool_list_mut(&mut self, tool: Tool) -> Option<&mut PresetList> {
        self.subtools.lists.iter_mut().find(|l| l.tool == tool)
    }

    /// 今の設定（ツールの欄の値）。
    pub fn subtool_capture(&self, tool: Tool) -> Values {
        fields::fields(tool).iter().map(|f| (f.get)(self)).collect()
    }

    /// 値を今の設定へ写す（欄の並びの順に書く）。
    pub fn subtool_apply(&mut self, tool: Tool, values: &Values) {
        for (field, value) in fields::fields(tool).iter().zip(values) {
            (field.set)(self, value);
        }
    }

    /// 今のサブツールの設定が基準と違うか。
    pub fn subtool_is_modified(&self, tool: Tool, key: Key) -> bool {
        self.subtool_list(tool)
            .is_some_and(|l| l.is_modified(key, &self.subtool_capture(tool)))
    }

    /// 今の設定が、今のサブツールの基準でも変えたままの設定でもなく、別のサブツールの基準とちょうど同じなら、そのサブツールを今のものにする
    /// （オプションバー・ツールプロパティ・キーで設定を替えて、別のサブツールと同じになったとき、一覧の印がついてくる）。替えたら true。
    /// 前のサブツールの「変えたままの設定」は、使い始めたときのまま残す。
    pub fn subtool_follow(&mut self, tool: Tool) -> bool {
        let live = self.subtool_capture(tool);
        let Some(list) = self.subtool_list_mut(tool) else {
            return false;
        };
        let current = list.current;
        // 今のサブツールの基準と同じ、または変えたままの設定と同じなら、印は動かさない（変えたままの設定が別のサブツールの基準と同じでも、
        // 押した行から印が逃げない）
        if list
            .entry(current)
            .is_some_and(|e| e.baseline == live || *e.effective() == live)
        {
            return false;
        }
        match list
            .entries
            .iter()
            .find(|e| e.baseline == live && e.key != current)
            .map(|e| e.key)
        {
            Some(key) => {
                list.current = key;
                true
            }
            None => false,
        }
    }

    /// 今の設定を、今のサブツールの「変えたままの設定」として一覧へ書き戻す（基準と同じなら変更なし）。
    pub fn subtool_sync(&mut self, tool: Tool) {
        if self.subtool_follow(tool) {
            return;
        }
        let live = self.subtool_capture(tool);
        if let Some(list) = self.subtool_list_mut(tool) {
            let current = list.current;
            if let Some(entry) = list.entry_mut(current) {
                entry.edited = (live != entry.baseline).then_some(live);
            }
        }
    }

    /// ツールを離れる（今の設定を覚える）。
    pub(crate) fn subtool_leave(&mut self, tool: Tool) {
        self.subtool_sync(tool);
    }

    /// ツールに入る（そのツールの今のサブツールの設定を今の設定にする）。
    pub(crate) fn subtool_enter(&mut self, tool: Tool) {
        let Some(values) = self
            .subtool_list(tool)
            .and_then(|l| l.entry(l.current))
            .map(|e| e.effective().clone())
        else {
            return;
        };
        self.subtool_apply(tool, &values);
        // 一覧はツールごとにスクロールの位置を持たないので、入ったツールの今の行が見えるところまで送る
        self.subtools.ui.reveal = true;
    }

    fn subtool_notice(&mut self, kind: crate::notice::Kind, ja: String, en: String) {
        let text = self.lang.pick(ja, en);
        self.notify(kind, crate::notice::Source::Brush, text);
    }

    fn subtool_refuse(&mut self) {
        self.refuse(
            crate::notice::Source::Brush,
            crate::lang::refusals::during_stroke(self.lang),
        );
    }

    /// ツールのファイルを読めなかったとき、書くと読めなかったファイル（新しい版かもしれない）を壊すので、増やす・変える操作を断る。断ったなら true。
    fn subtool_refuse_blocked(&mut self, tool: Tool) -> bool {
        let lang = self.lang;
        let Some(problem) = self
            .subtools
            .problems
            .iter()
            .find(|p| p.file.strip_suffix(".ylsubtool") == Some(tool.id()))
        else {
            return false;
        };
        let file = lang.quote(&problem.file);
        self.fail(
            crate::notice::Source::Brush,
            lang.with_reason(
                lang.pick(
                    format!("サブツールのファイル{file}を読めないため、保存できません"),
                    format!("Cannot save sub tools because the file {file} could not be read"),
                ),
                problem.describe(lang),
            ),
        );
        true
    }

    /// 利用者のプリセットのファイルを書く（失敗は知らせ、メモリの一覧は保つ）。書けた（書く先が無い）なら true。
    fn subtool_persist(&mut self, tool: Tool) -> bool {
        let Some(users) = self.subtool_list(tool).map(PresetList::users) else {
            return true;
        };
        let Some(store) = self.subtools.store.clone() else {
            return true;
        };
        match store.save(tool, &users) {
            Ok(()) => true,
            Err(e) => {
                let reason = e.describe(self.lang);
                self.subtool_notice(
                    crate::notice::Kind::Error,
                    Lang::Ja.with_reason("サブツールを保存できません", &reason),
                    Lang::En.with_reason("Cannot save the sub tools", &reason),
                );
                false
            }
        }
    }

    /// 設定のフォルダのサブツールを読んで一覧に入れる（起動のとき 1 回）。読めなかったファイルは読み飛ばし、理由を残す。
    pub fn attach_subtool_store(&mut self, dir: PathBuf) {
        let report = store::load_all(&dir);
        for (tool, users) in report.presets {
            if let Some(slot) = self.subtool_list_mut(tool) {
                *slot = PresetList::new(tool, users);
            }
        }
        self.subtools.store = Some(SubToolStore::new(dir));
        self.subtools.problems = report.problems;
        // 今のツールの一覧が読み直されたので、今の設定を新しい一覧の今のサブツールに合わせる
        let tool = self.tool;
        self.subtool_enter(tool);
    }

    /// 起動のとき読めなかったサブツールのファイルの知らせ（無ければ None）。
    pub fn subtool_problem_message(&self) -> Option<String> {
        let problems = &self.subtools.problems;
        let first = problems.first()?;
        let lang = self.lang;
        let file = lang.quote(&first.file);
        let one = lang.with_reason(
            lang.pick(
                format!("サブツールのファイル{file}を読めません"),
                format!("Cannot read the sub tool file {file}"),
            ),
            first.describe(lang),
        );
        Some(if problems.len() == 1 {
            one
        } else {
            lang.pick(
                format!(
                    "サブツールのファイルを {} 件読めません。{one}",
                    problems.len()
                ),
                format!("Cannot read {} sub tool files. {one}", problems.len()),
            )
        })
    }

    /// 新しい利用者のプリセットを足して、それに替える。`from` が None なら今の設定を、Some なら、そのサブツール（今のものなら今の設定）の複製を。
    fn subtool_create(&mut self, tool: Tool, from: Option<Key>) {
        if self.is_stroking() {
            return self.subtool_refuse();
        }
        if self.subtool_refuse_blocked(tool) {
            return;
        }
        let lang = self.lang;
        let live = self.subtool_capture(tool);
        let Some(list) = self.subtool_list(tool) else {
            return;
        };
        if list.user_count() >= MAX_USER_PRESETS {
            return self.subtool_notice(
                crate::notice::Kind::Refusal,
                format!("サブツールは {MAX_USER_PRESETS} 個までです。"),
                format!("At most {MAX_USER_PRESETS} sub tools."),
            );
        }
        let source_key = from.unwrap_or(list.current);
        let Some(source) = list.entry(source_key) else {
            return;
        };
        let values = if source_key == list.current {
            live
        } else {
            source.effective().clone()
        };
        let source_name = list.name_in(source_key, lang);
        let base = match from {
            None => lang.pick("プリセット", "Preset").to_owned(),
            Some(_) => lang.pick(
                format!("{source_name} のコピー"),
                format!("{source_name} copy"),
            ),
        };
        let base = clean_name(&base).unwrap_or_else(|| lang.pick("プリセット", "Preset").into());
        self.subtool_sync(tool);
        let Some(list) = self.subtool_list_mut(tool) else {
            return;
        };
        let name = list.unused_name(&base);
        let id = list.next_id;
        list.next_id = list.next_id.saturating_add(1);
        let key = Key::User(id);
        let at = match from {
            Some(k) if k.is_user() => list
                .entries
                .iter()
                .position(|e| e.key == k)
                .map_or(list.entries.len(), |i| i + 1),
            _ => list.entries.len(),
        };
        list.entries.insert(
            at,
            Entry {
                key,
                name: name.clone(),
                baseline: values.clone(),
                edited: None,
            },
        );
        list.current = key;
        self.subtool_apply(tool, &values);
        self.subtools.ui.reveal = true;
        // 知らせを先に（保存できなければ、その理由が知らせを上書きする）
        self.subtool_notice(
            crate::notice::Kind::Info,
            format!("サブツールを追加しました: {name}"),
            format!("Sub tool added: {name}"),
        );
        self.subtool_persist(tool);
    }

    /// 一覧の操作を当てる。
    pub fn subtool_action(&mut self, action: SubToolAction) {
        match action {
            SubToolAction::Select(tool, key) => {
                if self.is_stroking() {
                    return self.subtool_refuse();
                }
                if self.subtool_list(tool).and_then(|l| l.entry(key)).is_none() {
                    return;
                }
                // 今の設定を先に一覧へ書き戻してから、押した行の設定を読む（今の行を押し直したとき、変えたばかりの設定を基準で上書きしない）
                self.subtool_sync(tool);
                let Some(values) = self
                    .subtool_list(tool)
                    .and_then(|l| l.entry(key))
                    .map(|e| e.effective().clone())
                else {
                    return;
                };
                if let Some(list) = self.subtool_list_mut(tool) {
                    list.current = key;
                }
                self.subtool_apply(tool, &values);
                self.subtools.ui.reveal = true;
            }
            SubToolAction::Add(tool) => self.subtool_create(tool, None),
            SubToolAction::Duplicate(tool, key) => self.subtool_create(tool, Some(key)),
            SubToolAction::Delete(tool, key) => {
                if self.is_stroking() {
                    return self.subtool_refuse();
                }
                let lang = self.lang;
                let Some(name) = self
                    .subtool_list(tool)
                    .filter(|l| l.entry(key).is_some())
                    .map(|l| l.name_in(key, lang))
                else {
                    return;
                };
                if !key.is_user() {
                    return self.subtool_notice(
                        crate::notice::Kind::Refusal,
                        "組み込みのサブツールは消せません。".into(),
                        "Built-in sub tools cannot be deleted.".into(),
                    );
                }
                if self.subtool_refuse_blocked(tool) {
                    return;
                }
                self.subtool_sync(tool);
                let Some(list) = self.subtool_list_mut(tool) else {
                    return;
                };
                let at = list.entries.iter().position(|e| e.key == key).unwrap_or(0);
                list.entries.remove(at);
                // 今のサブツールなら、後ろの隣（なければ前の隣）へ移る。組み込みは消えないので、先頭はいつもある
                let mut values = None;
                if list.current == key {
                    let next = list
                        .entries
                        .get(at)
                        .or_else(|| at.checked_sub(1).and_then(|i| list.entries.get(i)))
                        .or_else(|| list.entries.first())
                        .map(|e| e.key);
                    if let Some(next) = next {
                        list.current = next;
                        values = list.entry(next).map(|e| e.effective().clone());
                    }
                }
                if self.subtools.ui.renaming == Some((tool, key)) {
                    self.subtools.ui.renaming = None;
                }
                if let Some(values) = values {
                    self.subtool_apply(tool, &values);
                }
                self.subtools.ui.reveal = true;
                self.subtool_notice(
                    crate::notice::Kind::Info,
                    format!("サブツールを削除しました: {name}"),
                    format!("Sub tool deleted: {name}"),
                );
                self.subtool_persist(tool);
            }
            SubToolAction::StartRename(tool, key) => {
                if self.subtool_list(tool).and_then(|l| l.entry(key)).is_none() {
                    return;
                }
                if !key.is_user() {
                    return self.subtool_notice(
                        crate::notice::Kind::Refusal,
                        "組み込みのサブツールは名前を変えられません。".into(),
                        "Built-in sub tools cannot be renamed.".into(),
                    );
                }
                self.subtools.ui.renaming = Some((tool, key));
                self.subtools.ui.rename_started = false;
            }
            SubToolAction::Rename(tool, key, name) => {
                if self.subtools.ui.renaming == Some((tool, key)) {
                    self.subtools.ui.renaming = None;
                }
                let Some(name) = clean_name(&name) else {
                    return;
                };
                if !key.is_user() || self.subtool_refuse_blocked(tool) {
                    return;
                }
                let changed = match self.subtool_list_mut(tool).and_then(|l| l.entry_mut(key)) {
                    Some(entry) if entry.name != name => {
                        entry.name = name;
                        true
                    }
                    _ => false,
                };
                if changed {
                    self.subtool_persist(tool);
                }
            }
            SubToolAction::Revert(tool, key) => {
                if self.is_stroking() {
                    return self.subtool_refuse();
                }
                let is_current = self.subtool_list(tool).is_some_and(|l| l.current == key);
                let Some(entry) = self.subtool_list_mut(tool).and_then(|l| l.entry_mut(key)) else {
                    return;
                };
                entry.edited = None;
                let baseline = entry.baseline.clone();
                if is_current {
                    self.subtool_apply(tool, &baseline);
                }
            }
            SubToolAction::Register(tool, key) => {
                if !key.is_user() || self.subtool_refuse_blocked(tool) {
                    return;
                }
                if self.subtool_list(tool).is_some_and(|l| l.current == key) {
                    self.subtool_sync(tool);
                }
                let registered = match self.subtool_list_mut(tool).and_then(|l| l.entry_mut(key)) {
                    Some(entry) => match entry.edited.take() {
                        Some(edited) => {
                            entry.baseline = edited;
                            true
                        }
                        None => false,
                    },
                    None => false,
                };
                if registered {
                    self.subtool_persist(tool);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gradient::End;
    use yolu_core::geometry::SurfaceRegionKind;
    use yolu_core::material::GradientShape;

    #[test]
    fn every_tool_starts_on_the_builtin_that_equals_its_defaults_and_is_not_modified() {
        let app = AppState::new(32, 32);
        for tool in fields::TOOLS {
            let list = app.subtool_list(tool).unwrap();
            assert_eq!(
                app.subtool_capture(tool),
                defaults(tool),
                "{tool:?}: 既定の状態"
            );
            let current = list.entry(list.current()).unwrap();
            assert_eq!(current.baseline, defaults(tool), "{tool:?}");
            assert!(!app.subtool_is_modified(tool, list.current()), "{tool:?}");
            assert!(!list.entries().is_empty());
            // どの組み込みも、欄の並びと同じ数の値を持つ
            for e in list.entries() {
                assert_eq!(e.baseline.len(), fields::fields(tool).len(), "{tool:?}");
            }
        }
    }

    #[test]
    fn getting_what_was_set_gives_the_same_values_for_every_builtin() {
        for tool in fields::TOOLS {
            let mut app = AppState::new(32, 32);
            let entries: Vec<Values> = app
                .subtool_list(tool)
                .unwrap()
                .entries()
                .iter()
                .map(|e| e.baseline.clone())
                .collect();
            for values in entries {
                app.subtool_apply(tool, &values);
                assert_eq!(app.subtool_capture(tool), values, "{tool:?}");
            }
        }
    }

    #[test]
    fn a_line_cannot_be_filled_and_the_figure_is_set_first() {
        let mut app = AppState::new(32, 32);
        let mut values = defaults(Tool::Shape);
        values[1] = Value::Bool(true); // fill
        app.subtool_apply(Tool::Shape, &values);
        assert!(!app.drafting.fill, "直線は塗れない");
        values[0] = Value::Choice("rectangle");
        app.subtool_apply(Tool::Shape, &values);
        assert!(app.drafting.fill);
    }

    #[test]
    fn selecting_a_row_sets_the_live_values_and_keeps_edits_of_the_row_left_behind() {
        let mut app = AppState::new(32, 32);
        let list = app.subtool_list(Tool::Fill).unwrap();
        let triangle = Key::Builtin("triangle");
        assert!(list.entry(triangle).is_some());
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        // 初めは UV アイランド
        assert_eq!(
            app.subtool_list(Tool::Fill).unwrap().current(),
            Key::Builtin("uv-island")
        );
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Fill,
            triangle,
        )));
        assert_eq!(app.region.kind, SurfaceRegionKind::Triangle);
        assert!(!app.region.by_color);
        // 三角形の行で許容を変える → 三角形が変更あり。別の行へ移っても覚えている
        app.region.tolerance = 77;
        assert!(app.subtool_is_modified(Tool::Fill, triangle));
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Fill,
            Key::Builtin("similar-colors"),
        )));
        assert!(app.region.by_color);
        assert_eq!(app.region.tolerance, 32, "近い色の行は自分の設定");
        assert!(
            app.subtool_is_modified(Tool::Fill, triangle),
            "離れた行の変更は残る"
        );
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Fill,
            triangle,
        )));
        assert_eq!(app.region.tolerance, 77, "戻ると変えたままの設定");
        // 元に戻す
        app.apply(crate::state::Action::SubTool(SubToolAction::Revert(
            Tool::Fill,
            triangle,
        )));
        assert_eq!(app.region.tolerance, 32);
        assert!(!app.subtool_is_modified(Tool::Fill, triangle));
    }

    #[test]
    fn selecting_the_current_row_again_keeps_the_live_settings() {
        use crate::state::Action;
        let mut app = AppState::new(32, 32);
        app.apply(Action::SelectTool(Tool::Fill));
        let similar = Key::Builtin("similar-colors");
        let select = |app: &mut AppState, key| {
            app.apply(Action::SubTool(SubToolAction::Select(Tool::Fill, key)));
        };
        select(&mut app, similar);
        // パネルで許容を変えた（一覧へはまだ書き戻していない）直後に、同じ行を押す
        app.region.tolerance = 90;
        select(&mut app, similar);
        assert_eq!(app.region.tolerance, 90, "押し直しで基準へ戻らない");
        assert!(app.subtool_is_modified(Tool::Fill, similar));
        // 利用者のサブツールも同じ
        app.apply(Action::SubTool(SubToolAction::Add(Tool::Fill)));
        let user = app.subtool_list(Tool::Fill).unwrap().current();
        assert!(user.is_user());
        app.region.tolerance = 12;
        select(&mut app, user);
        assert_eq!(app.region.tolerance, 12);
        assert!(app.subtool_is_modified(Tool::Fill, user));
        // 元に戻すのは専用の操作だけ
        app.apply(Action::SubTool(SubToolAction::Revert(Tool::Fill, user)));
        assert_eq!(
            app.region.tolerance, 90,
            "登録した設定（追加したときの設定）"
        );
        assert!(!app.subtool_is_modified(Tool::Fill, user));
    }

    #[test]
    fn the_mark_stays_on_the_row_pressed_even_when_its_changed_settings_equal_another_rows_original(
    ) {
        use crate::state::Action;
        let mut app = AppState::new(32, 32);
        app.apply(Action::SelectTool(Tool::Fill));
        let similar = Key::Builtin("similar-colors");
        let select = |app: &mut AppState, key| {
            app.apply(Action::SubTool(SubToolAction::Select(Tool::Fill, key)));
            app.subtool_follow(Tool::Fill); // パネルが毎フレームすること
        };
        select(&mut app, similar);
        app.region.tolerance = 90;
        // 変えたままの設定から足した利用者のサブツールの基準は、その設定と同じ
        app.apply(Action::SubTool(SubToolAction::Add(Tool::Fill)));
        let user = app.subtool_list(Tool::Fill).unwrap().current();
        select(&mut app, Key::Builtin("triangle"));
        // 「近い色」を押すと、変えたままの設定（許容 90）が今の設定になり、印は「近い色」にある（利用者のサブツールへ逃げない）
        select(&mut app, similar);
        assert_eq!(app.region.tolerance, 90);
        assert_eq!(app.subtool_list(Tool::Fill).unwrap().current(), similar);
        // 利用者のサブツールを押せば、そこへ
        select(&mut app, user);
        assert_eq!(app.subtool_list(Tool::Fill).unwrap().current(), user);
    }

    #[test]
    fn choosing_a_brush_while_another_tool_is_active_keeps_that_tools_changed_settings() {
        use crate::state::Action;
        let mut app = AppState::new(32, 32);
        app.apply(Action::SelectTool(Tool::Fill));
        app.region.tolerance = 90;
        // ブラシの選び（詳細のウィンドウの組み込みの一覧など）は、ツールをブラシ・消しゴムへ替える。ツールの切り替えの口を通らない道でも設定は残る
        app.apply(Action::M2Ui(crate::m2::UiOp::Preset(0)));
        assert!(app.tool.paints(), "{:?}", app.tool);
        app.apply(Action::SelectTool(Tool::Fill));
        assert_eq!(app.region.tolerance, 90, "戻ると変えたままの設定");
        assert!(
            app.subtool_is_modified(Tool::Fill, app.subtool_list(Tool::Fill).unwrap().current())
        );
    }

    #[test]
    fn switching_tools_keeps_each_tools_own_selection() {
        let mut app = AppState::new(32, 32);
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Fill,
            Key::Builtin("material"),
        )));
        app.apply(crate::state::Action::SelectTool(Tool::PolygonFill));
        assert_eq!(
            app.region.kind,
            SurfaceRegionKind::UvIsland,
            "ポリゴン塗りつぶしは自分の一覧の選び（既定は UV アイランド）"
        );
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::PolygonFill,
            Key::Builtin("triangle"),
        )));
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        assert_eq!(
            app.region.kind,
            SurfaceRegionKind::Material,
            "バケツへ戻ると、バケツの選び"
        );
        app.apply(crate::state::Action::SelectTool(Tool::PolygonFill));
        assert_eq!(app.region.kind, SurfaceRegionKind::Triangle);
        // グラデーションは形と終点
        app.apply(crate::state::Action::SelectTool(Tool::Gradient));
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Gradient,
            Key::Builtin("radial-sub"),
        )));
        assert_eq!(app.gradient.shape, GradientShape::Radial);
        assert_eq!(app.gradient.end, End::Sub);
    }

    #[test]
    fn user_presets_are_added_renamed_registered_and_deleted_and_every_step_is_saved() {
        let dir = std::env::temp_dir().join(format!("yolu-subtool-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = AppState::new(32, 32);
        app.attach_subtool_store(dir.clone());
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        app.region.tolerance = 60;
        app.apply(crate::state::Action::SubTool(SubToolAction::Add(
            Tool::Fill,
        )));
        let list = app.subtool_list(Tool::Fill).unwrap();
        let key = list.current();
        assert!(key.is_user());
        assert_eq!(list.name_in(key, Lang::Ja), "プリセット");
        assert_eq!(list.entry(key).unwrap().baseline[2], Value::Int(60));
        assert!(!app.subtool_is_modified(Tool::Fill, key));
        let file = dir.join("fill.ylsubtool");
        assert!(file.exists(), "足したらすぐ書く");
        // 名前を変える
        app.apply(crate::state::Action::SubTool(SubToolAction::Rename(
            Tool::Fill,
            key,
            "太い許容".into(),
        )));
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("preset.1.name=太い許容"));
        // 設定を変えると変更あり。登録すると基準が替わる
        app.region.tolerance = 90;
        assert!(app.subtool_is_modified(Tool::Fill, key));
        app.apply(crate::state::Action::SubTool(SubToolAction::Register(
            Tool::Fill,
            key,
        )));
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("preset.1.tolerance=90"));
        assert!(!app.subtool_is_modified(Tool::Fill, key));
        // 組み込みは名前を変えられず消せない
        let builtin = Key::Builtin("triangle");
        app.apply(crate::state::Action::SubTool(SubToolAction::Delete(
            Tool::Fill,
            builtin,
        )));
        assert!(app
            .subtool_list(Tool::Fill)
            .unwrap()
            .entry(builtin)
            .is_some());
        app.apply(crate::state::Action::SubTool(SubToolAction::StartRename(
            Tool::Fill,
            builtin,
        )));
        assert!(app.subtools.ui.renaming.is_none());
        // 複製は後ろへ、複製に替わる
        app.apply(crate::state::Action::SubTool(SubToolAction::Duplicate(
            Tool::Fill,
            key,
        )));
        let list = app.subtool_list(Tool::Fill).unwrap();
        assert_eq!(list.user_count(), 2);
        let copy = list.current();
        assert_eq!(list.name_in(copy, Lang::Ja), "太い許容 のコピー");
        // 読み直すと同じ
        let mut again = AppState::new(32, 32);
        again.attach_subtool_store(dir.clone());
        let names: Vec<String> = again
            .subtool_list(Tool::Fill)
            .unwrap()
            .users()
            .iter()
            .map(|u| u.name.clone())
            .collect();
        assert_eq!(names, ["太い許容", "太い許容 のコピー"]);
        // 消すと今のサブツールは隣へ、ファイルも更新される
        app.apply(crate::state::Action::SubTool(SubToolAction::Delete(
            Tool::Fill,
            copy,
        )));
        assert_eq!(app.subtool_list(Tool::Fill).unwrap().current(), key);
        app.apply(crate::state::Action::SubTool(SubToolAction::Delete(
            Tool::Fill,
            key,
        )));
        assert!(
            !file.exists(),
            "利用者のプリセットが無くなればファイルも消える"
        );
        assert_eq!(app.subtool_list(Tool::Fill).unwrap().user_count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_largest_number_of_user_presets_is_added_and_saved_and_one_more_is_refused() {
        let dir = std::env::temp_dir().join(format!("yolu-subtool-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = AppState::new(32, 32);
        app.attach_subtool_store(dir.clone());
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        // いちばん欄の多いバケツで、足す（今の設定の追加と複製を交ぜる）
        for n in 0..MAX_USER_PRESETS {
            let action = if n % 2 == 0 {
                SubToolAction::Add(Tool::Fill)
            } else {
                let current = app.subtool_list(Tool::Fill).unwrap().current();
                SubToolAction::Duplicate(Tool::Fill, current)
            };
            app.apply(crate::state::Action::SubTool(action));
            assert_eq!(
                app.subtool_list(Tool::Fill).unwrap().user_count(),
                n + 1,
                "{n} 個目: {}",
                app.message
            );
            assert!(
                !app.message.contains("保存できません"),
                "{n} 個目: {}",
                app.message
            );
        }
        let file = dir.join("fill.ylsubtool");
        let size = std::fs::metadata(&file).unwrap().len();
        assert!(size <= store::MAX_FILE_BYTES, "{size}");
        // もう 1 つは足さない（一覧もファイルも変えず、理由を知らせる）
        let before = std::fs::read_to_string(&file).unwrap();
        let current = app.subtool_list(Tool::Fill).unwrap().current();
        for action in [
            SubToolAction::Add(Tool::Fill),
            SubToolAction::Duplicate(Tool::Fill, current),
        ] {
            app.message.clear();
            app.apply(crate::state::Action::SubTool(action));
            assert_eq!(
                app.subtool_list(Tool::Fill).unwrap().user_count(),
                MAX_USER_PRESETS
            );
            assert!(
                app.message.contains(&MAX_USER_PRESETS.to_string()),
                "{}",
                app.message
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
        }
        // 全部が読み戻せる（起動のとき、1 つも落ちない）
        let mut restarted = AppState::new(32, 32);
        restarted.attach_subtool_store(dir.clone());
        assert!(restarted.subtool_problem_message().is_none());
        assert_eq!(
            restarted.subtool_list(Tool::Fill).unwrap().user_count(),
            MAX_USER_PRESETS
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("yolu-subtool-blocked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gradient.ylsubtool");
        std::fs::write(&file, "yolupainter-subtools 9\ntool=gradient\n").unwrap();
        let mut app = AppState::new(32, 32);
        app.attach_subtool_store(dir.clone());
        assert!(app
            .subtool_problem_message()
            .unwrap()
            .contains("gradient.ylsubtool"));
        app.apply(crate::state::Action::SelectTool(Tool::Gradient));
        app.apply(crate::state::Action::SubTool(SubToolAction::Add(
            Tool::Gradient,
        )));
        assert_eq!(
            app.subtool_list(Tool::Gradient).unwrap().user_count(),
            0,
            "足せない"
        );
        assert!(
            app.message.contains("gradient.ylsubtool"),
            "{}",
            app.message
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "yolupainter-subtools 9\ntool=gradient\n",
            "触らない"
        );
        // ほかのツールは使える
        app.apply(crate::state::Action::SelectTool(Tool::Shape));
        app.apply(crate::state::Action::SubTool(SubToolAction::Add(
            Tool::Shape,
        )));
        assert_eq!(app.subtool_list(Tool::Shape).unwrap().user_count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_changes_while_drawing() {
        let mut app = AppState::new(32, 32);
        app.apply(crate::state::Action::SelectTool(Tool::Fill));
        app.drafting.drag = Some(crate::drafting::Drag {
            source: crate::state::StrokeSource::Mouse,
            start: yolu_core::glam::DVec2::ZERO,
            current: yolu_core::glam::DVec2::ZERO,
            shift: false,
            alt: false,
        });
        assert!(app.is_stroking());
        app.apply(crate::state::Action::SubTool(SubToolAction::Select(
            Tool::Fill,
            Key::Builtin("material"),
        )));
        assert_eq!(app.region.kind, SurfaceRegionKind::UvIsland);
        app.apply(crate::state::Action::SubTool(SubToolAction::Add(
            Tool::Fill,
        )));
        assert_eq!(app.subtool_list(Tool::Fill).unwrap().user_count(), 0);
    }
}
