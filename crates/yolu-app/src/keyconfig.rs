//! 利用者のキー・マウスの組み合わせ・パイメニューの設定。設定のウィンドウの「ショートカット」の区分（`shortcuts::editor`）が変え、既定（`keymap::bindings`・
//! `keymap::GESTURES`・`pie::builtin`）から変えた所だけを持つ。設定のフォルダの `keymap.json` に、変えた所だけを書く（設定の `キー=値` の
//! ファイルとは別）。
//!
//! - キーは操作と効く範囲（`GroupKey`）ごとの入力の並びで持つ。既定の行の入力をこの並びで置き換え（行の場面は既定の行のまま）、空なら外す。
//!   既定の行が無い操作に入れた入力は、場面なしの行になる。
//! - マウスの組み合わせは、既定の表の番号ごとに、組み合わせ（ボタンと修飾。押しながらのキーは既定のまま）か、外したか。
//! - 同じ範囲・同じ場面で同じ入力が別の操作を指す「ぶつかり」が残っている間は、ファイルに書かない（効く割り当ては変えたとおり。判定の順で先の
//!   行が取る）。
//! - 読めない・壊れた `keymap.json` は既定で始め、ファイルは消さない（次に書くとき、別の名前へ移してから書く）。知らない操作・範囲は飛ばして数える。
//! - 効かせるのは `keymap::install`（このスレッドの表）。アプリはフレームの初めにも入れる。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Key, Modifiers, PointerButton};
use serde_json::{json, Map, Value};

use crate::commands;
use crate::keymap::{self, Gesture, KeyBinding, Keymap, Operation, Scope, Trigger, When, GESTURES};
use crate::lang::Lang;
use crate::notice::Source;
use crate::pie::{self, PieItem, PieMenu, PieName};
use crate::state::AppState;
use crate::toolkeys::ToolKeyMode;
use crate::ui::pie::SLOTS;

/// 設定のフォルダの中のファイルの名前。
pub const FILE_NAME: &str = "keymap.json";
/// 読めなかったファイルを、書き直す前に移す名前。
const BROKEN_NAME: &str = "keymap.broken.json";
const FORMAT: &str = "yolupainter-keymap";
const VERSION: u64 = 1;

/// キーの行のまとまり（操作の ID と、効く範囲）。
pub type GroupKey = (&'static str, Scope);

/// マウスの組み合わせ（ボタンと、押しの始めに押している修飾。押しながらのキーは既定のまま）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub button: PointerButton,
    pub alt: bool,
    pub shift: bool,
    pub ctrl: bool,
}

impl Combo {
    pub fn of(g: &Gesture) -> Combo {
        Combo {
            button: g.button,
            alt: g.alt,
            shift: g.shift,
            ctrl: g.ctrl,
        }
    }

    /// 押したボタンと修飾から（Ctrl は Mac の Command も）。
    pub fn pressed(button: PointerButton, m: &Modifiers) -> Combo {
        Combo {
            button,
            alt: m.alt,
            shift: m.shift,
            ctrl: m.ctrl || m.command,
        }
    }

    /// この行に入れられる形。始めたあとに効く修飾の行（ステンシルの回転の刻み）は修飾だけを持つので、ボタンを行のボタンにし、修飾が
    /// 無ければ入れられない（None。押しても効かない行になる）。
    pub fn fit(self, g: &Gesture) -> Option<Combo> {
        if g.starts || g.click {
            return Some(self);
        }
        (self.alt || self.shift || self.ctrl).then_some(Combo {
            button: g.button,
            ..self
        })
    }

    fn apply(self, g: &Gesture) -> Gesture {
        Gesture {
            button: self.button,
            alt: self.alt,
            shift: self.shift,
            ctrl: self.ctrl,
            ..*g
        }
    }
}

/// マウスの組み合わせのぶつかり。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComboConflict {
    /// ほかの組み合わせ（既定の表の番号）と同じ。
    With(u8),
    /// 修飾なしの左ボタン（描く押し）と同じ。
    Painting,
}

/// 利用者の設定（既定から変えた所）と、それから作った今の割り当て。
#[derive(Debug)]
pub struct KeyConfig {
    keys: BTreeMap<GroupKey, Vec<Trigger>>,
    mouse: BTreeMap<u8, Option<Combo>>,
    /// ツールのキーの動き方（押すと切り替えから変えた操作だけ）。
    tool_modes: BTreeMap<&'static str, ToolKeyMode>,
    map: Arc<Keymap>,
    path: Option<PathBuf>,
    /// 起動のときに読めなかったファイルを、そのまま残している（次に書くとき、別の名前へ移してから書く）。
    broken: bool,
    /// ぶつかりが残っていて、ファイルに書いていない変更がある。
    pub unsaved: bool,
}

impl Default for KeyConfig {
    fn default() -> Self {
        KeyConfig {
            keys: BTreeMap::new(),
            mouse: BTreeMap::new(),
            tool_modes: BTreeMap::new(),
            map: keymap::default_map(),
            path: None,
            broken: false,
            unsaved: false,
        }
    }
}

/// 読んだ設定（キー・マウス・パイと、飛ばした知らない物の数）。
#[derive(Debug, Default)]
pub struct Parsed {
    pub keys: BTreeMap<GroupKey, Vec<Trigger>>,
    pub mouse: BTreeMap<u8, Option<Combo>>,
    pub tool_modes: BTreeMap<&'static str, ToolKeyMode>,
    pub pies: Vec<PieMenu>,
    pub skipped: usize,
}

/// 既定の行の、まとまりの並び（表の順。最初に出た所）。
pub fn default_groups() -> Vec<GroupKey> {
    let mut out: Vec<GroupKey> = Vec::new();
    for b in keymap::default_map().rows() {
        let key = (b.command, b.scope);
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// まとまりの既定の入力（表の順）。
pub fn default_triggers(key: GroupKey) -> Vec<Trigger> {
    keymap::default_map()
        .rows()
        .iter()
        .filter(|b| (b.command, b.scope) == key)
        .map(|b| b.trigger)
        .collect()
}

/// 操作の ID を、プロセスの間ずっと使える文字にする（知っている操作は表の文字、利用者のパイは `keymap::intern`）。知らなければ None。
pub fn command_id(id: &str, pies: &[PieMenu]) -> Option<&'static str> {
    if let Some(c) = commands::find(id) {
        return Some(c.id);
    }
    let pie = keymap::pie_of(id)?;
    pies.iter().any(|m| m.id == pie).then(|| keymap::intern(id))
}

impl KeyConfig {
    /// 今の割り当て。
    pub fn map(&self) -> Arc<Keymap> {
        self.map.clone()
    }

    /// 覚えている設定のファイル。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// まとまりの今の入力。
    pub fn triggers(&self, key: GroupKey) -> Vec<Trigger> {
        self.keys
            .get(&key)
            .cloned()
            .unwrap_or_else(|| default_triggers(key))
    }

    /// まとまりを既定から変えたか（ツールのキーは、動き方を変えたときも）。
    pub fn is_changed(&self, key: GroupKey) -> bool {
        self.keys.contains_key(&key) || self.tool_modes.contains_key(key.0)
    }

    /// ツールを選ぶ操作のキーの動き方（変えていなければ、押すと切り替え）。
    pub fn tool_mode(&self, command: &str) -> ToolKeyMode {
        self.tool_modes
            .get(command)
            .copied()
            .unwrap_or(ToolKeyMode::Tap)
    }

    /// ツールのキーの動き方を替える（押すと切り替えなら、変えた所から外す）。ツールを選ぶ操作でなければ何もしない。
    pub fn set_tool_mode(&mut self, command: &str, mode: ToolKeyMode) {
        let Some(command) =
            commands::find(command).filter(|c| commands::selected_tool(c.id).is_some())
        else {
            return;
        };
        if mode == ToolKeyMode::Tap {
            self.tool_modes.remove(command.id);
        } else {
            self.tool_modes.insert(command.id, mode);
        }
        self.rebuild();
    }

    /// 変えたまとまり（既定の行が無い操作に入れた物も）。
    pub fn changed_groups(&self) -> impl Iterator<Item = GroupKey> + '_ {
        self.keys.keys().copied()
    }

    /// まとまりの入力を置き換える（既定と同じなら、変えた所から外す）。
    pub fn set(&mut self, key: GroupKey, triggers: Vec<Trigger>) {
        if triggers == default_triggers(key) {
            self.keys.remove(&key);
        } else {
            self.keys.insert(key, triggers);
        }
        self.rebuild();
    }

    /// まとまりを既定に戻す（ツールのキーは、動き方も押すと切り替えに）。
    pub fn reset(&mut self, key: GroupKey) {
        self.keys.remove(&key);
        self.tool_modes.remove(key.0);
        self.rebuild();
    }

    /// 全部を既定に戻す（キーとマウスと、ツールのキーの動き方）。
    pub fn reset_all(&mut self) {
        self.keys.clear();
        self.mouse.clear();
        self.tool_modes.clear();
        self.rebuild();
    }

    /// 利用者のパイを消したとき、そのパイを開くキーも外す。
    pub fn forget_command(&mut self, command: &str) {
        self.keys.retain(|(c, _), _| *c != command);
        self.tool_modes.remove(command);
        self.rebuild();
    }

    /// マウスの組み合わせ（既定の表の番号。外していれば None）。
    pub fn combo(&self, index: u8) -> Option<Combo> {
        match self.mouse.get(&index) {
            Some(c) => *c,
            None => GESTURES.get(index as usize).map(Combo::of),
        }
    }

    pub fn combo_changed(&self, index: u8) -> bool {
        self.mouse.contains_key(&index)
    }

    /// マウスの組み合わせを置き換える（None で外す。既定と同じなら、変えた所から外す）。
    pub fn set_combo(&mut self, index: u8, combo: Option<Combo>) {
        let default = GESTURES.get(index as usize).map(Combo::of);
        if combo == default {
            self.mouse.remove(&index);
        } else {
            self.mouse.insert(index, combo);
        }
        self.rebuild();
    }

    pub fn reset_combo(&mut self, index: u8) {
        self.mouse.remove(&index);
        self.rebuild();
    }

    /// 既定から変えた所が無いか。
    pub fn is_default(&self) -> bool {
        self.keys.is_empty() && self.mouse.is_empty() && self.tool_modes.is_empty()
    }

    /// 割り当てを作り直して、このスレッドで効かせる。
    fn rebuild(&mut self) {
        let defaults = keymap::bindings();
        let mut rows: Vec<KeyBinding> = Vec::new();
        let mut done: Vec<GroupKey> = Vec::new();
        for row in &defaults {
            let key = (row.command, row.scope);
            let Some(triggers) = self.keys.get(&key) else {
                rows.push(*row);
                continue;
            };
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            let templates: Vec<&KeyBinding> = defaults
                .iter()
                .filter(|b| (b.command, b.scope) == key)
                .collect();
            for (i, trigger) in triggers.iter().enumerate() {
                let base = templates.get(i).copied().unwrap_or(templates[0]);
                rows.push(KeyBinding {
                    trigger: *trigger,
                    ..*base
                });
            }
        }
        // 既定の行が無い操作（パイを開く操作など）の入力
        for (key, triggers) in &self.keys {
            if done.contains(key) {
                continue;
            }
            for trigger in triggers {
                rows.push(KeyBinding {
                    trigger: *trigger,
                    command: key.0,
                    scope: key.1,
                    when: When::Always,
                });
            }
        }
        let gestures: Vec<Gesture> = GESTURES
            .iter()
            .filter_map(|g| match self.mouse.get(&g.index) {
                None => Some(*g),
                Some(Some(c)) => Some(c.apply(g)),
                Some(None) => None,
            })
            .collect();
        let modes = self.tool_modes.iter().map(|(id, m)| (*id, *m)).collect();
        self.map = Arc::new(Keymap::with_gestures(rows, gestures).with_tool_modes(modes));
        keymap::install(self.map.clone());
    }

    /// まとまりの `index` 番目の入力が、ほかの操作とぶつかるか（同じ範囲・同じ場面で同じ入力）。ぶつかる相手のまとまり。
    pub fn conflict(&self, key: GroupKey, index: usize) -> Option<GroupKey> {
        let rows: Vec<&KeyBinding> = self
            .map
            .rows()
            .iter()
            .filter(|b| (b.command, b.scope) == key)
            .collect();
        let row = rows.get(index)?;
        conflict_of(&self.map, row).map(|o| (o.command, o.scope))
    }

    /// 同じ範囲・同じ場面で同じ入力に先に当たる、ほかの範囲の行（モードの段がどこでもの段より先に取るなど）。ぶつかりではない。
    pub fn shadowed_by(&self, key: GroupKey, index: usize) -> Option<GroupKey> {
        let rows: Vec<&KeyBinding> = self
            .map
            .rows()
            .iter()
            .filter(|b| (b.command, b.scope) == key)
            .collect();
        let row = rows.get(index)?;
        if row.scope != Scope::Everywhere {
            return None;
        }
        self.map
            .rows()
            .iter()
            .find(|o| {
                o.command != row.command
                    && o.scope.is_mode()
                    && o.when == When::Always
                    && keymap::same_input(&o.trigger, &row.trigger)
            })
            .map(|o| (o.command, o.scope))
    }

    /// モードの段（ペイント・編集・ポーズ）の行が、どこでもの段の同じ入力の行より先に取るなら、その取られる行（ぶつかりではない）。
    pub fn takes_before(&self, key: GroupKey, index: usize) -> Option<GroupKey> {
        if !key.1.is_mode() {
            return None;
        }
        let rows: Vec<&KeyBinding> = self
            .map
            .rows()
            .iter()
            .filter(|b| (b.command, b.scope) == key)
            .collect();
        let row = rows.get(index)?;
        if row.when != When::Always {
            return None;
        }
        self.map
            .rows()
            .iter()
            .find(|o| {
                o.command != row.command
                    && o.scope == Scope::Everywhere
                    && keymap::same_input(&o.trigger, &row.trigger)
            })
            .map(|o| (o.command, o.scope))
    }

    /// マウスの組み合わせが、ほかの組み合わせとぶつかるか（同じ所で効くまとまり・ボタン・押しながらのキー・修飾・始め方が同じ。2D の
    /// canvas と選択のツールの selection は同時に効くので同じ所）。2D の押しは、視点の行（パン・回転）→ 選択の行 → スポイトの順に、
    /// 書いた修飾を含む押しで引くので、先に引く行の組み合わせに修飾を足しただけの、後の行の組み合わせもぶつかる（後の行が効かない）。
    /// 2D・3D の視点の行と離しの行に修飾なしの左ボタンを当てると、描く押しとぶつかる（選択の行は、選択のツールの押しの組み合わせ方を
    /// 決めるだけなので、ぶつからない）。
    pub fn combo_conflict(&self, index: u8) -> Option<ComboConflict> {
        let gestures = self.map.gestures();
        let g = gestures.iter().find(|g| g.index == index)?;
        if matches!(g.scope, "canvas" | "view3d")
            && (g.starts || g.click)
            && g.button == PointerButton::Primary
            && g.held.is_none()
            && !(g.alt || g.shift || g.ctrl)
        {
            return Some(ComboConflict::Painting);
        }
        gestures
            .iter()
            .find(|o| o.index != index && (same_press(o, g) || hides(o, g) || hides(g, o)))
            .map(|o| ComboConflict::With(o.index))
    }

    /// ぶつかりが残っているか。
    pub fn has_conflicts(&self) -> bool {
        self.map
            .rows()
            .iter()
            .any(|r| conflict_of(&self.map, r).is_some())
            || self
                .map
                .gestures()
                .iter()
                .any(|g| self.combo_conflict(g.index).is_some())
    }

    /// 既定から変えた所の JSON（キー・マウス・パイ）。
    pub fn to_json(&self, pies: &[PieMenu]) -> Value {
        let keys: Vec<Value> = self
            .keys
            .iter()
            .map(|((command, scope), triggers)| {
                json!({
                    "command": command,
                    "scope": scope.key(),
                    "keys": triggers.iter().map(trigger_text).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mouse: Vec<Value> = self
            .mouse
            .iter()
            .filter_map(|(index, combo)| {
                let g = GESTURES.get(*index as usize)?;
                let nth = GESTURES[..*index as usize]
                    .iter()
                    .filter(|o| (o.scope, o.operation, o.click) == (g.scope, g.operation, g.click))
                    .count();
                Some(json!({
                    "scope": g.scope,
                    "operation": operation_key(g.operation),
                    "click": g.click,
                    "index": nth,
                    "combo": combo.map(combo_text),
                }))
            })
            .collect();
        let builtin = pie::builtin();
        let pies: Vec<Value> = pies
            .iter()
            .filter(|m| {
                builtin
                    .iter()
                    .find(|b| b.id == m.id)
                    .is_none_or(|b| b.slots != m.slots)
            })
            .map(|m| {
                let mut o = Map::new();
                o.insert("id".into(), Value::String(m.id.clone()));
                if let PieName::User(name) = &m.name {
                    o.insert("name".into(), Value::String(name.clone()));
                }
                o.insert(
                    "slots".into(),
                    Value::Array(m.slots.iter().map(slot_json).collect()),
                );
                Value::Object(o)
            })
            .collect();
        let mut out = json!({
            "format": FORMAT,
            "version": VERSION,
            "keys": keys,
            "mouse": mouse,
            "pies": pies,
        });
        // ツールのキーの動き方は、押すと切り替えから変えた物があるときだけ（無ければ書かない）
        if !self.tool_modes.is_empty() {
            let modes: Map<String, Value> = self
                .tool_modes
                .iter()
                .map(|(id, mode)| ((*id).to_owned(), Value::String(mode.key().to_owned())))
                .collect();
            out["tool_keys"] = Value::Object(modes);
        }
        out
    }

    /// 読んだ設定を入れる（今の変更は捨て、既定に読んだ所を重ねる）。パイは呼ぶ側が入れる。
    pub fn apply(&mut self, parsed: &Parsed) {
        self.keys = parsed.keys.clone();
        self.mouse = parsed.mouse.clone();
        self.tool_modes = parsed.tool_modes.clone();
        self.rebuild();
    }

    /// 設定のファイルを覚えて読む（起動のとき）。読めなければ既定のまま（ファイルは消さない）。知らせる文（読めない・飛ばした物）を返す。
    pub fn attach(&mut self, path: PathBuf, pies: &mut Vec<PieMenu>, lang: Lang) -> Option<String> {
        self.path = Some(path.clone());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                self.broken = true;
                return Some(broken_message(lang, &e.to_string()));
            }
        };
        match parse(&text, lang) {
            Ok(parsed) => {
                self.apply(&parsed);
                *pies = merged_pies(&parsed.pies);
                (parsed.skipped > 0).then(|| skipped_message(lang, parsed.skipped))
            }
            Err(why) => {
                self.broken = true;
                Some(broken_message(lang, &why))
            }
        }
    }

    /// 設定のファイルへ書く（覚えたファイルが無ければ何もしない）。ぶつかりが残っていれば書かずに Ok(false)。書いたら Ok(true)。
    pub fn save(&mut self, pies: &[PieMenu]) -> std::io::Result<bool> {
        if self.has_conflicts() {
            self.unsaved = true;
            return Ok(false);
        }
        self.unsaved = false;
        let Some(path) = self.path.clone() else {
            return Ok(false);
        };
        let pies_default = pies == pie::builtin().as_slice();
        if self.is_default() && pies_default && !path.exists() {
            return Ok(false);
        }
        if self.broken {
            // 読めなかったファイルは消さずに、別の名前へ移す
            if path.exists() {
                std::fs::rename(&path, free_name(&path))?;
            }
            self.broken = false;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json(pies)).unwrap_or_default();
        yolu_io::atomic::replace_bytes(&path, text.as_bytes())?;
        Ok(true)
    }
}

/// ぶつかる行（同じ範囲・同じ場面・同じ入力の、別の操作の行）。
fn conflict_of<'a>(map: &'a Keymap, row: &KeyBinding) -> Option<&'a KeyBinding> {
    map.rows().iter().find(|o| {
        o.command != row.command
            && o.scope == row.scope
            && o.when == row.when
            && keymap::same_input(&o.trigger, &row.trigger)
    })
}

/// 読めなかったファイルを移す先（`keymap.broken.json`。あれば番号を付ける）。
fn free_name(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let first = dir.join(BROKEN_NAME);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("keymap.broken-{n}.json")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn broken_message(lang: Lang, why: &str) -> String {
    lang.pick(
        format!("キーの設定（{FILE_NAME}）を読めないので、既定のキーで始めました（{why}）。ファイルは消さずに残してあります。"),
        format!("Cannot read the key settings ({FILE_NAME}), so the default keys are used ({why}). The file is kept."),
    )
}

/// 飛ばした知らない物の数の知らせ。
/// 押しが効く所（2D の canvas と選択のツールの selection は同じ所）。
fn place(scope: &'static str) -> &'static str {
    if scope == "selection" {
        "canvas"
    } else {
        scope
    }
}

/// 同じ所・同じボタン・同じ押しながらのキー・同じ修飾・同じ始め方。
fn same_press(a: &Gesture, b: &Gesture) -> bool {
    place(a.scope) == place(b.scope)
        && a.held == b.held
        && a.button == b.button
        && (a.alt, a.shift, a.ctrl) == (b.alt, b.shift, b.ctrl)
        && (a.starts, a.click) == (b.starts, b.click)
}

/// 2D の押しで行を引く順（視点の行 → 選択の行 → スポイト。`canvas::nav::start_of`）。2D の始める行でなければ None。
fn canvas_rank(g: &Gesture) -> Option<u8> {
    if !g.starts {
        return None;
    }
    match (g.scope, g.operation) {
        ("canvas", Operation::Pan | Operation::Rotate) => Some(0),
        ("selection", _) => Some(1),
        ("canvas", _) => Some(2),
        _ => None,
    }
}

/// `a` が、`b` の組み合わせの押しで先に引かれて、`b` を隠すか（2D の押しで、`a` を先に引き、`a` は書いた修飾を含む押しに当たる）。
fn hides(a: &Gesture, b: &Gesture) -> bool {
    match (canvas_rank(a), canvas_rank(b)) {
        (Some(ra), Some(rb)) => {
            ra < rb
                && a.held == b.held
                && a.button == b.button
                && (!a.alt || b.alt)
                && (!a.shift || b.shift)
                && (!a.ctrl || b.ctrl)
        }
        _ => false,
    }
}

pub fn skipped_message(lang: Lang, skipped: usize) -> String {
    lang.pick(
        format!("キーの設定の、読めない・知らない項目（操作・入力・組み合わせ・パイの項目）{skipped} 個を飛ばしました。"),
        format!("Skipped {skipped} unreadable or unknown item(s) (actions, keys, combinations or pie items) in the key settings."),
    )
}

// ───────── 文字にする・読む ─────────

/// 入力の文字（ファイルの書き方。`Ctrl` は Command の割り当て（Windows・Linux の Ctrl、Mac の Command）、`Control` は Control そのもの。
/// 文字の入力は `text:` を付ける）。
pub fn trigger_text(t: &Trigger) -> String {
    match t {
        Trigger::Text(text) => format!("text:{text}"),
        Trigger::Key { modifiers, key } => {
            let mut s = String::new();
            if modifiers.command {
                s.push_str("Ctrl+");
            } else if modifiers.ctrl {
                s.push_str("Control+");
            }
            if modifiers.alt {
                s.push_str("Alt+");
            }
            if modifiers.shift {
                s.push_str("Shift+");
            }
            s.push_str(key.name());
            s
        }
    }
}

/// `trigger_text` の文字を読む（読めなければ None）。
pub fn parse_trigger(text: &str) -> Option<Trigger> {
    if let Some(t) = text.strip_prefix("text:") {
        return (!t.is_empty()).then(|| Trigger::Text(keymap::intern(t)));
    }
    let mut modifiers = Modifiers::NONE;
    let mut rest = text;
    loop {
        if let Some(r) = rest.strip_prefix("Ctrl+") {
            modifiers.command = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Control+") {
            modifiers.ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Alt+") {
            modifiers.alt = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Shift+") {
            modifiers.shift = true;
            rest = r;
        } else {
            break;
        }
    }
    Key::from_name(rest).map(|key| Trigger::Key { modifiers, key })
}

/// 押したキーの割り当ての入力（Ctrl は Command の割り当て。Mac の Control だけは Control）。
pub fn trigger_of(key: Key, m: &Modifiers) -> Trigger {
    let modifiers = Modifiers {
        alt: m.alt,
        shift: m.shift,
        command: m.command || m.mac_cmd,
        ctrl: !(m.command || m.mac_cmd) && m.ctrl,
        mac_cmd: false,
    };
    Trigger::Key { modifiers, key }
}

pub fn combo_text(c: Combo) -> String {
    let mut s = String::new();
    if c.ctrl {
        s.push_str("Ctrl+");
    }
    if c.alt {
        s.push_str("Alt+");
    }
    if c.shift {
        s.push_str("Shift+");
    }
    s.push_str(match c.button {
        PointerButton::Primary => "Left",
        PointerButton::Middle => "Middle",
        PointerButton::Secondary => "Right",
        PointerButton::Extra1 => "Back",
        PointerButton::Extra2 => "Forward",
    });
    s
}

pub fn parse_combo(text: &str) -> Option<Combo> {
    let (mut alt, mut shift, mut ctrl) = (false, false, false);
    let mut rest = text;
    loop {
        if let Some(r) = rest.strip_prefix("Ctrl+") {
            ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Alt+") {
            alt = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Shift+") {
            shift = true;
            rest = r;
        } else {
            break;
        }
    }
    let button = match rest {
        "Left" => PointerButton::Primary,
        "Middle" => PointerButton::Middle,
        "Right" => PointerButton::Secondary,
        "Back" => PointerButton::Extra1,
        "Forward" => PointerButton::Extra2,
        _ => return None,
    };
    Some(Combo {
        button,
        alt,
        shift,
        ctrl,
    })
}

fn operation_key(op: keymap::Operation) -> &'static str {
    use keymap::Operation::*;
    match op {
        Orbit => "orbit",
        Pan => "pan",
        Zoom => "zoom",
        Rotate => "rotate",
        Pick => "pick",
        SnapOrbit => "snap_orbit",
        CloneSource => "clone_source",
        SelectionAdd => "selection_add",
        SelectionSubtract => "selection_subtract",
        SelectionIntersect => "selection_intersect",
        MoveStencil => "move_stencil",
        RotateStencil => "rotate_stencil",
        ScaleStencil => "scale_stencil",
        SnapStencilRotation => "snap_stencil_rotation",
    }
}

fn slot_json(slot: &Option<PieItem>) -> Value {
    match slot {
        None => Value::Null,
        Some(PieItem::Command(id)) => json!({ "command": id }),
        Some(PieItem::Pie(id)) => json!({ "pie": id }),
        Some(PieItem::Ops(text)) => json!({ "ops": text }),
        Some(PieItem::Recorded(name)) => json!({ "recorded": name }),
    }
}

/// パイの項目を読む（知らない操作は None で、`skipped` を数える）。
fn slot_of(v: &Value, skipped: &mut usize) -> Option<PieItem> {
    let o = v.as_object()?;
    let text = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
    if let Some(id) = text("command") {
        if commands::find(&id).is_some() {
            return Some(PieItem::Command(id));
        }
        *skipped += 1;
        return None;
    }
    if let Some(id) = text("pie") {
        return Some(PieItem::Pie(id));
    }
    if let Some(ops) = text("ops") {
        return Some(PieItem::Ops(ops));
    }
    if let Some(name) = text("recorded") {
        return Some(PieItem::Recorded(name));
    }
    *skipped += 1;
    None
}

/// `keymap.json` の中身を読む。形が違えば理由。知らない操作・範囲・組み合わせは飛ばして数える。
pub fn parse(text: &str, lang: Lang) -> Result<Parsed, String> {
    let bad = |why: &str| lang.pick(format!("形が違います: {why}"), format!("bad format: {why}"));
    let v: Value = serde_json::from_str(text).map_err(|e| bad(&e.to_string()))?;
    let o = v.as_object().ok_or_else(|| bad("not an object"))?;
    if o.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(bad("format"));
    }
    let version = o.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version == 0 || version > VERSION {
        return Err(lang.pick(
            format!("この版では読めない版です（{version}）"),
            format!("unsupported version ({version})"),
        ));
    }
    let mut parsed = Parsed::default();
    // パイ（キーの `pie.<ID>` を知るため、先に読む）
    for p in o
        .get("pies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = p.get("id").and_then(Value::as_str) else {
            parsed.skipped += 1;
            continue;
        };
        let slots_in: Vec<Value> = p
            .get("slots")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut slots: [Option<PieItem>; SLOTS] = Default::default();
        for (i, s) in slots_in.iter().take(SLOTS).enumerate() {
            slots[i] = if s.is_null() {
                None
            } else {
                slot_of(s, &mut parsed.skipped)
            };
        }
        let builtin = pie::builtin().into_iter().find(|b| b.id == id);
        let menu = match builtin {
            Some(b) => PieMenu { slots, ..b },
            None => {
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_owned();
                pie::user_menu(id, name, slots)
            }
        };
        if parsed.pies.iter().any(|m| m.id == menu.id) {
            parsed.skipped += 1;
            continue;
        }
        parsed.pies.push(menu);
    }
    let known_pies = merged_pies(&parsed.pies);
    let mut seen_keys: Vec<GroupKey> = Vec::new();
    for k in o
        .get("keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let command = k
            .get("command")
            .and_then(Value::as_str)
            .and_then(|id| command_id(id, &known_pies));
        let scope = k
            .get("scope")
            .and_then(Value::as_str)
            .and_then(Scope::from_key);
        let (Some(command), Some(scope)) = (command, scope) else {
            parsed.skipped += 1;
            continue;
        };
        if commands::find(command).is_some_and(|c| c.locked) {
            parsed.skipped += 1;
            continue;
        }
        // 同じ行の 2 つ目からは飛ばす
        if seen_keys.contains(&(command, scope)) {
            parsed.skipped += 1;
            continue;
        }
        seen_keys.push((command, scope));
        let Some(texts) = k.get("keys").and_then(Value::as_array) else {
            parsed.skipped += 1;
            continue;
        };
        let mut triggers = Vec::new();
        let mut unread = 0;
        for t in texts {
            match t.as_str().and_then(parse_trigger) {
                Some(t) => triggers.push(t),
                None => unread += 1,
            }
        }
        // 読めない入力だけの行は行ごと飛ばして既定のまま（外すのは空の並び `[]` だけ）
        if !texts.is_empty() && triggers.is_empty() {
            parsed.skipped += 1;
            continue;
        }
        parsed.skipped += unread;
        if triggers != default_triggers((command, scope)) {
            parsed.keys.insert((command, scope), triggers);
        }
    }
    let mut seen_mouse: Vec<u8> = Vec::new();
    for m in o
        .get("mouse")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let scope = m.get("scope").and_then(Value::as_str);
        let operation = m.get("operation").and_then(Value::as_str);
        let click = m.get("click").and_then(Value::as_bool).unwrap_or(false);
        let nth = m.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let found = GESTURES
            .iter()
            .filter(|g| {
                Some(g.scope) == scope
                    && Some(operation_key(g.operation)) == operation
                    && g.click == click
            })
            .nth(nth);
        let Some(g) = found else {
            parsed.skipped += 1;
            continue;
        };
        if seen_mouse.contains(&g.index) {
            parsed.skipped += 1;
            continue;
        }
        seen_mouse.push(g.index);
        let combo = match m.get("combo") {
            None | Some(Value::Null) => None,
            Some(c) => match c.as_str().and_then(parse_combo).and_then(|c| c.fit(g)) {
                Some(c) => Some(c),
                None => {
                    parsed.skipped += 1;
                    continue;
                }
            },
        };
        if combo != Some(Combo::of(g)) {
            parsed.mouse.insert(g.index, combo);
        }
    }
    // ツールのキーの動き方（ツールを選ぶ操作の ID → 動き方）。知らない操作・知らない動き方は飛ばして数える。押すと切り替え（既定）は変えた所に入れない
    match o.get("tool_keys") {
        None | Some(Value::Null) => {}
        Some(Value::Object(modes)) => {
            for (id, mode) in modes {
                let command =
                    commands::find(id).filter(|c| commands::selected_tool(c.id).is_some());
                let mode = mode.as_str().and_then(ToolKeyMode::from_key);
                match (command, mode) {
                    (Some(_), Some(ToolKeyMode::Tap)) => {}
                    (Some(c), Some(mode)) => {
                        parsed.tool_modes.insert(c.id, mode);
                    }
                    _ => parsed.skipped += 1,
                }
            }
        }
        Some(_) => parsed.skipped += 1,
    }
    Ok(parsed)
}

/// 最初からあるパイに、読んだパイを重ねた並び（最初からあるパイは中身を置き換え、利用者のパイは後ろに並べる）。
pub fn merged_pies(read: &[PieMenu]) -> Vec<PieMenu> {
    let mut out = pie::builtin();
    for m in read {
        match out.iter_mut().find(|b| b.id == m.id) {
            Some(b) => b.slots = m.slots.clone(),
            None => out.push(m.clone()),
        }
    }
    out
}

impl AppState {
    /// キーの設定を変えたあと: 効かせて、ぶつかりが無ければファイルへ書く。
    pub fn keys_changed(&mut self) {
        keymap::install(self.keys.map());
        if let Err(e) = self.keys.save(&self.pie.menus) {
            let lang = self.lang;
            self.fail(
                Source::Settings,
                lang.pick(
                    format!("キーの設定を保存できません（{e}）"),
                    format!("Cannot save the key settings ({e})"),
                ),
            );
        }
    }

    /// 全部を既定に戻す（キー・マウス・パイ）。
    pub fn keys_reset_all(&mut self) {
        self.keys.reset_all();
        self.pie.menus = pie::builtin();
        self.keys_changed();
    }

    /// 今の設定（既定から変えた所）をファイルへ書き出す。
    pub fn keys_export(&mut self, path: &Path) {
        let lang = self.lang;
        let text =
            serde_json::to_string_pretty(&self.keys.to_json(&self.pie.menus)).unwrap_or_default();
        match yolu_io::atomic::replace_bytes(path, text.as_bytes()) {
            Ok(()) => self.info(
                Source::Settings,
                lang.pick("キーの設定を書き出しました。", "Key settings exported."),
            ),
            Err(e) => self.fail(
                Source::Settings,
                lang.pick(
                    format!("キーの設定を書き出せません（{e}）"),
                    format!("Cannot export the key settings ({e})"),
                ),
            ),
        }
    }

    /// ファイルの設定を読み込む（今の変更は捨て、既定に読んだ所を重ねる）。知らない物は飛ばして数を知らせる。
    pub fn keys_import(&mut self, path: &Path) {
        let lang = self.lang;
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| parse(&text, lang));
        match parsed {
            Ok(parsed) => {
                self.pie.menus = merged_pies(&parsed.pies);
                self.keys.apply(&parsed);
                self.keys_changed();
                let mut text = lang
                    .pick("キーの設定を読み込みました。", "Key settings imported.")
                    .to_owned();
                if parsed.skipped > 0 {
                    text.push(' ');
                    text.push_str(&skipped_message(lang, parsed.skipped));
                    self.warn(Source::Settings, text);
                } else {
                    self.info(Source::Settings, text);
                }
            }
            Err(why) => self.fail(
                Source::Settings,
                lang.pick(
                    format!("キーの設定を読み込めません（{why}）"),
                    format!("Cannot import the key settings ({why})"),
                ),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
