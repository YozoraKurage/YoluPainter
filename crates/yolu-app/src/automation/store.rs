//! アクションの置き場: 設定のフォルダの `actions/<名前>.json`（1 つのアクションが 1 つのファイル。中身は `yolu_ops::action` の形で、
//! `yolupainter-cli run-action` にそのまま渡せる）と、一覧の並びの `actions/order.json`（`{"format": 1, "files": [...]}`）。.ylp には入れない。
//!
//! - 書くのは一時ファイルからの置換（`crate::userfiles::write_text`。読み戻して確かめる）。
//! - 読めないファイル（形の違う・版の新しい・大きすぎる・入れられない命令がある）は読まずに知らせ、消さない。その名前のファイルは新しく作らない
//!   （上書きしない）。上限（[`MAX_ACTIONS`]）を超えた分も読まずに知らせる。
//! - ファイルの名前はアクションの名前から作る（Windows で使えない文字は `_`、予約の名前は前に `_`、ほかのファイルと重なれば ` 2`…）。
//! - 並びのファイルが無い・読めなければ、名前の順。並びに無いファイルは後ろへ名前の順で足す。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use yolu_ops::action::{self, ActionFile, MAX_FILE_BYTES};
use yolu_ops::{Command, OpError};

use crate::lang::Lang;
use crate::userfiles::{self, FileError};

/// アクションの数の上限。
pub const MAX_ACTIONS: usize = 256;
/// 並びのファイル。
pub const ORDER_FILE: &str = "order.json";
const ORDER_LIMIT: u64 = 1 << 20;

/// 置いてあるアクション 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub name: String,
    pub commands: Vec<Command>,
    /// フォルダの中のファイルの名前。
    pub file: String,
}

/// 読めなかったファイル。
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: String,
    pub reason: ProblemReason,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProblemReason {
    /// 形が違う・版が新しい・入れられない命令（理由は日英）。
    Invalid(yolu_ops::Text),
    TooLarge,
    Unreadable,
    /// 上限を超えた。
    TooMany,
}

impl Problem {
    /// 知らせの文（「アクションのファイル「a.json」を読めません（理由）」）。
    pub fn message(&self, lang: Lang) -> String {
        let reason = match &self.reason {
            ProblemReason::Invalid(text) => text.pick(super::ops_lang(lang)).to_owned(),
            ProblemReason::TooLarge => lang
                .pick("ファイルが大きすぎます", "the file is too large")
                .to_owned(),
            ProblemReason::Unreadable => lang
                .pick("ファイルを読めません", "the file cannot be read")
                .to_owned(),
            ProblemReason::TooMany => lang.pick(
                format!("アクションは {MAX_ACTIONS} 個までです"),
                format!("at most {MAX_ACTIONS} actions"),
            ),
        };
        lang.with_reason(
            lang.pick(
                format!("アクションのファイル{}を読めません", lang.quote(&self.file)),
                format!("Cannot read the action file {}", lang.quote(&self.file)),
            ),
            reason,
        )
    }
}

/// 置き場の操作の失敗。
#[derive(Debug)]
pub enum StoreError {
    /// 設定のフォルダが無い（設定のファイルを使わない起動・試験）。
    NoFolder,
    TooMany,
    /// 同じ名前のアクションがある。
    NameTaken,
    /// 名前の形が違う。
    BadName(OpError),
    TooLarge,
    Io,
}

impl StoreError {
    /// 理由の文（`Lang::with_reason` の「なぜ」）。
    pub fn reason(&self, lang: Lang) -> String {
        match self {
            StoreError::NoFolder => lang
                .pick("設定のフォルダがありません", "there is no settings folder")
                .to_owned(),
            StoreError::TooMany => lang.pick(
                format!("アクションは {MAX_ACTIONS} 個までです"),
                format!("at most {MAX_ACTIONS} actions"),
            ),
            StoreError::NameTaken => lang
                .pick(
                    "同じ名前のアクションがあります",
                    "an action with the same name exists",
                )
                .to_owned(),
            StoreError::BadName(e) => e.message.pick(super::ops_lang(lang)).to_owned(),
            StoreError::TooLarge => lang
                .pick("ファイルが大きすぎます", "the file would be too large")
                .to_owned(),
            StoreError::Io => lang
                .pick("ファイルを書けません", "the file cannot be written")
                .to_owned(),
        }
    }
}

impl From<FileError> for StoreError {
    fn from(e: FileError) -> Self {
        match e {
            FileError::TooLarge => StoreError::TooLarge,
            _ => StoreError::Io,
        }
    }
}

/// アクションの置き場。
#[derive(Debug, Default)]
pub struct ActionStore {
    dir: Option<PathBuf>,
    items: Vec<Stored>,
    /// 読まなかったファイル（新しいファイルにその名前を使わない）。
    skipped: Vec<String>,
    /// 直前の操作で並びのファイルを書けなかった（アクションのファイルは書けている。[`ActionStore::take_order_failure`]）。
    order_unsaved: bool,
}

/// ファイルの名前に使えない文字を `_` にし、Windows の予約の名前を避けた、名前の元（拡張子なし）。
pub fn file_stem_for(name: &str) -> String {
    let mut stem: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    while stem.ends_with(['.', ' ']) {
        stem.pop();
    }
    let stem = stem.trim_start().to_owned();
    let stem = if stem.is_empty() {
        "action".to_owned()
    } else {
        stem
    };
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{stem}")
    } else {
        stem
    }
}

impl ActionStore {
    /// フォルダのアクションを読む（無いフォルダは空）。読めなかったファイルを返す。
    pub fn attach(&mut self, dir: PathBuf) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut found: Vec<Stored> = Vec::new();
        self.items.clear();
        self.skipped.clear();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut files: Vec<String> = entries
                .flatten()
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                .filter(|n| n.to_ascii_lowercase().ends_with(".json") && n != ORDER_FILE)
                .collect();
            files.sort();
            for file in files {
                match read_action(&dir.join(&file)) {
                    Ok(action) => found.push(Stored {
                        name: action.name,
                        commands: action.commands,
                        file,
                    }),
                    Err(reason) => {
                        self.skipped.push(file.clone());
                        problems.push(Problem { file, reason });
                    }
                }
            }
        }
        // 並び: 並びのファイルの順、無い物は後ろへ名前の順
        let order = read_order(&dir.join(ORDER_FILE));
        found.sort_by(|a, b| {
            let rank = |s: &Stored| {
                order
                    .iter()
                    .position(|f| *f == s.file)
                    .unwrap_or(usize::MAX)
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.file.cmp(&b.file))
        });
        for extra in found.drain(MAX_ACTIONS.min(found.len())..) {
            self.skipped.push(extra.file.clone());
            problems.push(Problem {
                file: extra.file,
                reason: ProblemReason::TooMany,
            });
        }
        self.items = found;
        self.dir = Some(dir);
        problems
    }

    pub fn items(&self) -> &[Stored] {
        &self.items
    }

    pub fn get(&self, index: usize) -> Option<&Stored> {
        self.items.get(index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// フォルダ（無ければ None）。
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// 名前が、ほかのアクション（`except` を除く）と重なるか。
    pub fn name_taken(&self, name: &str, except: Option<usize>) -> bool {
        self.items
            .iter()
            .enumerate()
            .any(|(i, s)| Some(i) != except && s.name == name)
    }

    /// 新しいアクションの名前（「アクション 1」から、重ならない番号）。
    pub fn new_name(&self, lang: Lang) -> String {
        let base = lang.pick("アクション", "Action");
        (1..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !self.name_taken(name, None))
            .expect("名前は尽きない")
    }

    /// 使っていないファイルの名前（`except` の物は使ってよい）。
    fn file_for(&self, dir: &Path, name: &str, except: Option<&str>) -> String {
        let stem = file_stem_for(name);
        let taken = |file: &str| {
            let lower = file.to_lowercase();
            let mine = except.is_some_and(|e| e.to_lowercase() == lower);
            !mine
                && (lower == ORDER_FILE
                    || self.items.iter().any(|s| s.file.to_lowercase() == lower)
                    || self.skipped.iter().any(|f| f.to_lowercase() == lower)
                    || dir.join(file).exists())
        };
        let first = format!("{stem}.json");
        if !taken(&first) {
            return first;
        }
        (2..)
            .map(|n| format!("{stem} {n}.json"))
            .find(|f| !taken(f))
            .expect("名前は尽きない")
    }

    /// アクションを足す（一覧の最後）。足した番号を返す。
    pub fn add(&mut self, name: &str, commands: Vec<Command>) -> Result<usize, StoreError> {
        let dir = self.dir.clone().ok_or(StoreError::NoFolder)?;
        action::check_name(name).map_err(StoreError::BadName)?;
        if self.items.len() >= MAX_ACTIONS {
            return Err(StoreError::TooMany);
        }
        if self.name_taken(name, None) {
            return Err(StoreError::NameTaken);
        }
        let file = self.file_for(&dir, name, None);
        write_action(&dir.join(&file), &ActionFile::new(name, commands.clone()))?;
        self.items.push(Stored {
            name: name.to_owned(),
            commands,
            file,
        });
        // アクションのファイルは書けているので、並びを書けなくても足したことにする（断ると、同じ記録をもう 1 つ保存してしまう）
        self.save_order();
        Ok(self.items.len() - 1)
    }

    /// 名前を変える（ファイルの名前も変え、前のファイルを消す）。
    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), StoreError> {
        let dir = self.dir.clone().ok_or(StoreError::NoFolder)?;
        action::check_name(name).map_err(StoreError::BadName)?;
        let Some(item) = self.items.get(index) else {
            return Ok(());
        };
        if item.name == name {
            return Ok(());
        }
        if self.name_taken(name, Some(index)) {
            return Err(StoreError::NameTaken);
        }
        let old_file = item.file.clone();
        let file = self.file_for(&dir, name, Some(&old_file));
        // 大文字小文字だけが違う名前は、ファイルの名前を替えない（大文字小文字を区別するファイルシステムでは別のファイルになり、
        // 前のファイルが残ってもう 1 つのアクションとして読まれる。区別しないところでは同じファイルで、名前の字形が残るとは限らない）
        let file = if file.to_lowercase() == old_file.to_lowercase() {
            old_file.clone()
        } else {
            file
        };
        write_action(
            &dir.join(&file),
            &ActionFile::new(name, item.commands.clone()),
        )?;
        let item = &mut self.items[index];
        item.name = name.to_owned();
        item.file = file.clone();
        let removed = if file == old_file {
            Ok(())
        } else {
            userfiles::remove(&dir.join(&old_file))
        };
        self.save_order();
        removed.map_err(|_| StoreError::Io)
    }

    /// 消す（ファイルも消す）。
    pub fn remove(&mut self, index: usize) -> Result<(), StoreError> {
        let dir = self.dir.clone().ok_or(StoreError::NoFolder)?;
        let Some(item) = self.items.get(index) else {
            return Ok(());
        };
        userfiles::remove(&dir.join(&item.file)).map_err(|_| StoreError::Io)?;
        self.items.remove(index);
        self.save_order();
        Ok(())
    }

    /// 並べ替え（`from` を `to` の位置へ）。
    pub fn move_item(&mut self, from: usize, to: usize) -> Result<(), StoreError> {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return Ok(());
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.write_order()
    }

    /// 直前の操作で並びのファイルを書けなかったか（読んだら下ろす）。並びは一覧の順を覚えるだけなので、書けなくてもアクションのファイルは
    /// 有効で、次に起動したときに並びが前のままになる。呼ぶ側が知らせる。
    pub fn take_order_failure(&mut self) -> bool {
        std::mem::take(&mut self.order_unsaved)
    }

    /// 足す・名前の変更・消すの後に並びを書く。書けなくても操作は済んでいる（失敗は [`ActionStore::take_order_failure`] で渡す）。
    /// 並べ替えは、並びを書くことが操作そのものなので、これを使わず断る。
    fn save_order(&mut self) {
        self.order_unsaved = self.write_order().is_err();
    }

    fn write_order(&self) -> Result<(), StoreError> {
        let dir = self.dir.as_ref().ok_or(StoreError::NoFolder)?;
        let files: Vec<&str> = self.items.iter().map(|s| s.file.as_str()).collect();
        let mut text = serde_json::to_string_pretty(&json!({"format": 1, "files": files}))
            .expect("並びは JSON にできる");
        text.push('\n');
        let expected = text.clone();
        userfiles::write_text(&dir.join(ORDER_FILE), &text, ORDER_LIMIT, |read| {
            read == expected
        })?;
        Ok(())
    }
}

/// アクションのファイルを読む（大きさを先に確かめる）。
fn read_action(path: &Path) -> Result<ActionFile, ProblemReason> {
    let text = userfiles::read_checked(path, MAX_FILE_BYTES).map_err(|e| match e {
        FileError::TooLarge => ProblemReason::TooLarge,
        _ => ProblemReason::Unreadable,
    })?;
    action::parse_action(&text).map_err(|e| ProblemReason::Invalid(e.message))
}

fn write_action(path: &Path, file: &ActionFile) -> Result<(), StoreError> {
    let text = file.to_text();
    let expected = text.clone();
    userfiles::write_text(path, &text, MAX_FILE_BYTES, |read| read == expected)?;
    Ok(())
}

/// 並びのファイル（読めなければ空）。
fn read_order(path: &Path) -> Vec<String> {
    let Ok(text) = userfiles::read_checked(path, ORDER_LIMIT) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    if value["format"] != 1 {
        return Vec::new();
    }
    value["files"]
        .as_array()
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yolu-app-actions-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn commands() -> Vec<Command> {
        vec![yolu_ops::parse_command(
            &json!({"command": "mask.add", "args": {"layer": "$selected"}}),
        )
        .unwrap()]
    }

    #[test]
    fn file_names_avoid_characters_and_names_windows_refuses() {
        assert_eq!(file_stem_for("Wash: blue/1?"), "Wash_ blue_1_");
        assert_eq!(file_stem_for("con"), "_con");
        assert_eq!(file_stem_for("COM3.x"), "_COM3.x");
        assert_eq!(file_stem_for("Combo"), "Combo");
        assert_eq!(file_stem_for("end. "), "end");
        assert_eq!(file_stem_for("..."), "action");
        assert_eq!(file_stem_for("汚し"), "汚し");
    }

    #[test]
    fn actions_are_added_renamed_moved_and_removed_with_their_files_and_order() {
        let d = dir("crud");
        let mut store = ActionStore::default();
        assert!(store.attach(d.clone()).is_empty());
        assert_eq!(store.add("B", commands()).unwrap(), 0);
        assert_eq!(store.add("A", commands()).unwrap(), 1);
        assert!(matches!(
            store.add("A", commands()),
            Err(StoreError::NameTaken)
        ));
        assert!(matches!(
            store.add("", commands()),
            Err(StoreError::BadName(_))
        ));
        // 名前が「order」でも並びのファイルと重ならない
        store.add("order", commands()).unwrap();
        assert_eq!(store.get(2).unwrap().file, "order 2.json");
        store.move_item(1, 0).unwrap();
        store.rename(0, "A2").unwrap();
        assert!(d.join("A2.json").is_file() && !d.join("A.json").exists());
        let mut again = ActionStore::default();
        assert!(again.attach(d.clone()).is_empty());
        let names: Vec<&str> = again.items().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["A2", "B", "order"], "並びは並びのファイルの順");
        again.remove(1).unwrap();
        assert!(!d.join("B.json").exists());
        let mut third = ActionStore::default();
        third.attach(d.clone());
        assert_eq!(third.len(), 2);
        // CLI に渡せる形
        let text = std::fs::read_to_string(d.join("A2.json")).unwrap();
        let file = action::parse_action(&text).unwrap();
        assert_eq!(file.name, "A2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn broken_files_are_reported_kept_and_never_overwritten() {
        let d = dir("broken");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Broken.json"), "{ not json").unwrap();
        std::fs::write(
            d.join("Newer.json"),
            json!({"format": 9, "name": "Newer", "commands": []}).to_string(),
        )
        .unwrap();
        std::fs::write(
            d.join("Save.json"),
            json!({"format": 1, "name": "Save", "commands": [{"command": "save", "args": {"confirm": true}}]}).to_string(),
        )
        .unwrap();
        std::fs::write(d.join("notes.txt"), "not an action").unwrap();
        let mut store = ActionStore::default();
        let problems = store.attach(d.clone());
        assert_eq!(problems.len(), 3);
        assert!(store.is_empty());
        for p in &problems {
            for lang in Lang::ALL {
                let text = p.message(lang);
                assert!(text.contains(&p.file), "{text}");
            }
        }
        // 読めないファイルの名前は使わない（上書きしない）
        store.add("Broken", commands()).unwrap();
        assert_eq!(store.get(0).unwrap().file, "Broken 2.json");
        assert_eq!(
            std::fs::read_to_string(d.join("Broken.json")).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn more_than_the_limit_are_left_unread_and_adding_is_refused() {
        let d = dir("limit");
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..MAX_ACTIONS + 2 {
            let file = ActionFile::new(format!("A{i:03}"), commands());
            std::fs::write(d.join(format!("A{i:03}.json")), file.to_text()).unwrap();
        }
        let mut store = ActionStore::default();
        let problems = store.attach(d.clone());
        assert_eq!(store.len(), MAX_ACTIONS);
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| p.reason == ProblemReason::TooMany));
        assert!(matches!(
            store.add("More", commands()),
            Err(StoreError::TooMany)
        ));
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            MAX_ACTIONS + 2,
            "消さない"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// フォルダの中の、アクションのファイルの名前（並びのファイルを除く。順）。
    fn action_files(d: &Path) -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n.ends_with(".json") && n != ORDER_FILE)
            .collect();
        files.sort();
        files
    }

    #[test]
    fn renaming_only_the_case_keeps_one_file_and_one_action() {
        let d = dir("case");
        let mut store = ActionStore::default();
        store.attach(d.clone());
        store.add("A", commands()).unwrap();
        store.add("Other", commands()).unwrap();
        store.rename(0, "a").unwrap();
        assert_eq!(store.get(0).unwrap().name, "a");
        // 大文字小文字を区別するファイルシステムでも、前のファイルが別のアクションとして残らない
        assert_eq!(action_files(&d).len(), 2, "{:?}", action_files(&d));
        let mut again = ActionStore::default();
        assert!(again.attach(d.clone()).is_empty());
        let names: Vec<&str> = again.items().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "Other"], "並びも変わらない");
        // 大文字小文字のほかも変わる名前は、これまでどおりファイルの名前も変わる
        again.rename(0, "B").unwrap();
        assert!(again.get(0).unwrap().file.eq_ignore_ascii_case("B.json"));
        assert_eq!(action_files(&d).len(), 2, "{:?}", action_files(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_order_write_does_not_undo_what_was_saved() {
        let d = dir("order");
        // 並びのファイルの場所をフォルダにして、書けなくする
        std::fs::create_dir_all(d.join(ORDER_FILE)).unwrap();
        let mut store = ActionStore::default();
        assert!(store.attach(d.clone()).is_empty());
        assert!(!store.take_order_failure());
        store.add("A", commands()).unwrap();
        assert_eq!(store.len(), 1);
        assert!(d.join("A.json").is_file());
        assert!(store.take_order_failure(), "知らせるために覚えている");
        assert!(!store.take_order_failure(), "読んだら下ろす");
        store.add("B", commands()).unwrap();
        store.rename(1, "C").unwrap();
        assert!(store.take_order_failure());
        store.remove(0).unwrap();
        assert!(store.take_order_failure());
        assert_eq!(store.len(), 1);
        assert_eq!(action_files(&d), ["C.json"]);
        // 並べ替えは、並びを書くことが操作そのものなので断る
        store.add("D", commands()).unwrap();
        assert!(matches!(store.move_item(1, 0), Err(StoreError::Io)));
        let mut again = ActionStore::default();
        assert!(again.attach(d.clone()).is_empty());
        assert_eq!(again.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn without_a_settings_folder_nothing_is_written() {
        let mut store = ActionStore::default();
        assert!(matches!(
            store.add("A", commands()),
            Err(StoreError::NoFolder)
        ));
    }
}
