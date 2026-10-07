//! オートアクション（操作の記録と再生）。画面でした操作のうち命令にできる物を、CLI・MCP と同じ命令（`yolu_ops::Command`）の列として
//! 記録し（[`record`]）、設定のフォルダの `actions/` に 1 つずつ JSON で置き（[`store`]）、今の文書へ **1 回の取り消し**で再生する
//! （`yolu_ops::action::run` を起動中のアプリのホスト `AppHost` に当てる）。描く操作（ストローク）は記録しない。
//!
//! - 再生は今のテクスチャセットの文書へ。描いている最中・保存の途中は断る（Refusal）。途中の命令が断ったら、そこまでを戻して、
//!   何番目の命令が・なぜかを知らせる（Error。ログに残る）。記録中は再生しない。
//! - 記録を止めると、命令が 1 つ以上あれば「アクション N」の名前で保存する（名前はウィンドウで変えられる）。

pub mod record;
pub mod store;

use crate::lang::Lang;
use crate::notice::{Kind, Source};
use crate::state::AppState;

/// アクションのウィンドウの操作。
#[derive(Clone, Debug, PartialEq)]
pub enum AutomationOp {
    /// 記録を始める。
    StartRecording,
    /// 記録を止め、命令が 1 つ以上あれば新しいアクションとして保存する。
    StopRecording,
    /// 一覧の n 番目を、今の文書に 1 回の取り消しで当てる。
    Play(usize),
    /// 行を選ぶ（None で外す）。
    Select(Option<usize>),
    /// 名前の入力を始める。
    StartRename(usize),
    Rename(usize, String),
    CancelRename,
    Delete(usize),
    /// 並べ替え（`from` の行を `to` の位置へ）。
    Move {
        from: usize,
        to: usize,
    },
}

impl AutomationOp {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        matches!(self, AutomationOp::Play(_))
    }
}

/// アクションの状態（置き場・記録・ウィンドウの選び）。アプリの状態で、.ylp には入れない。
#[derive(Debug, Default)]
pub struct Automation {
    pub store: store::ActionStore,
    /// 記録中なら、その記録。
    pub recorder: Option<record::Recorder>,
    /// 選んでいる行。
    pub selected: Option<usize>,
    /// 名前を変えている行と、入力欄がフォーカスを取った後か。
    pub renaming: Option<usize>,
    pub rename_started: bool,
    /// 並べ替えでつかんでいる行。
    pub dragging: Option<usize>,
}

impl Automation {
    /// 記録中か。
    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }
    /// 記録しなかった、文書を変えた操作があったか（記録のウィンドウの印）。
    pub fn skipped(&self) -> bool {
        self.recorder.as_ref().is_some_and(|r| r.skipped)
    }
}

/// 画面の言語を、命令の文の言語へ。
pub fn ops_lang(lang: Lang) -> yolu_ops::Lang {
    match lang {
        Lang::Ja => yolu_ops::Lang::Ja,
        Lang::En => yolu_ops::Lang::En,
    }
}

/// 設定のフォルダの `actions/` を読む。読めなかったファイルがあれば、知らせの文（「 / 」でつなぐ）。
pub fn attach(app: &mut AppState, dir: std::path::PathBuf) -> Option<String> {
    let problems = app.automation.store.attach(dir);
    (!problems.is_empty()).then(|| {
        problems
            .iter()
            .map(|p| p.message(app.lang))
            .collect::<Vec<_>>()
            .join(" / ")
    })
}

impl AppState {
    /// アクションのウィンドウの操作を当てる。
    pub fn automation_apply(&mut self, op: AutomationOp) {
        let lang = self.lang;
        match op {
            AutomationOp::StartRecording => {
                if self.automation.recorder.is_none() {
                    self.automation.recorder = Some(record::Recorder::start(self));
                }
            }
            AutomationOp::StopRecording => self.stop_recording(),
            AutomationOp::Play(index) => self.play_action(index),
            AutomationOp::Select(index) => {
                self.automation.selected = index.filter(|i| *i < self.automation.store.len());
            }
            AutomationOp::StartRename(index) => {
                if index < self.automation.store.len() {
                    self.automation.selected = Some(index);
                    self.automation.renaming = Some(index);
                    self.automation.rename_started = false;
                }
            }
            AutomationOp::CancelRename => self.automation.renaming = None,
            AutomationOp::Rename(index, name) => {
                self.automation.renaming = None;
                let name = name.trim();
                match self.automation.store.rename(index, name) {
                    Ok(()) => self.note_unsaved_order(false),
                    Err(e) => {
                        let text = lang.with_reason(
                            lang.pick(
                                "アクションの名前を変えられません",
                                "Cannot rename the action",
                            ),
                            e.reason(lang),
                        );
                        self.fail(Source::Action, text);
                    }
                }
            }
            AutomationOp::Delete(index) => {
                let Some(name) = self.automation.store.get(index).map(|s| s.name.clone()) else {
                    return;
                };
                match self.automation.store.remove(index) {
                    Ok(()) => {
                        let len = self.automation.store.len();
                        self.automation.selected = match self.automation.selected {
                            Some(s) if s > index => Some(s - 1),
                            Some(s) if s == index => (len > 0).then(|| index.min(len - 1)),
                            other => other,
                        };
                        self.automation.renaming = None;
                        self.info(
                            Source::Action,
                            lang.pick(
                                format!("アクション{}を削除しました。", lang.quote(&name)),
                                format!("Deleted action {}.", lang.quote(&name)),
                            ),
                        );
                        self.note_unsaved_order(true);
                    }
                    Err(e) => {
                        let text = lang.with_reason(
                            lang.pick(
                                format!("アクション{}を削除できません", lang.quote(&name)),
                                format!("Cannot delete action {}", lang.quote(&name)),
                            ),
                            e.reason(lang),
                        );
                        self.fail(Source::Action, text);
                    }
                }
            }
            AutomationOp::Move { from, to } => {
                let selected = self.automation.selected;
                match self.automation.store.move_item(from, to) {
                    Ok(()) => {
                        if selected == Some(from) {
                            self.automation.selected = Some(to);
                        }
                    }
                    Err(e) => {
                        let text = lang.with_reason(
                            lang.pick(
                                "アクションを並べ替えられません",
                                "Cannot reorder the actions",
                            ),
                            e.reason(lang),
                        );
                        self.fail(Source::Action, text);
                    }
                }
            }
        }
    }

    /// 並びのファイルを書けなかったときだけ知らせる（足す・名前の変更・消すは、アクションのファイルの側が済んでいるので成功のまま）。
    /// `after_info`: 済んだ知らせを出した直後なら、その知らせに但し書きとして添える。
    fn note_unsaved_order(&mut self, after_info: bool) {
        if !self.automation.store.take_order_failure() {
            return;
        }
        let lang = self.lang;
        let text = lang.with_reason(
            lang.pick(
                "アクションの並びを保存できません",
                "Cannot save the order of the actions",
            ),
            store::StoreError::Io.reason(lang),
        );
        if after_info {
            self.amend(Kind::Warning, Source::Action, " ", &text);
        } else {
            self.warn(Source::Action, text);
        }
    }

    fn stop_recording(&mut self) {
        let lang = self.lang;
        let Some(recorder) = self.automation.recorder.take() else {
            return;
        };
        let commands = recorder.commands();
        if commands.is_empty() {
            self.info(
                Source::Action,
                lang.pick("記録した操作はありません。", "Nothing was recorded."),
            );
            return;
        }
        let name = self.automation.store.new_name(lang);
        match self.automation.store.add(&name, commands) {
            Ok(index) => {
                self.automation.selected = Some(index);
                self.info(
                    Source::Action,
                    lang.pick(
                        format!("アクション{}を保存しました。", lang.quote(&name)),
                        format!("Saved action {}.", lang.quote(&name)),
                    ),
                );
                self.note_unsaved_order(true);
            }
            Err(e) => {
                // 保存できなければ記録を続ける（記録した操作を捨てない）
                self.automation.recorder = Some(recorder);
                let text = lang.with_reason(
                    lang.pick("アクションを保存できません", "Cannot save the action"),
                    e.reason(lang),
                );
                self.fail(Source::Action, text);
            }
        }
    }

    fn play_action(&mut self, index: usize) {
        let lang = self.lang;
        let Some(item) = self.automation.store.get(index) else {
            return;
        };
        let (name, commands) = (item.name.clone(), item.commands.clone());
        let what = lang.pick(
            format!("アクション{}を再生できません", lang.quote(&name)),
            format!("Cannot play action {}", lang.quote(&name)),
        );
        if self.automation.recorder.is_some() {
            let text = lang.with_reason(&what, lang.pick("記録中です", "Recording is in progress"));
            self.refuse(Source::Action, text);
            return;
        }
        if self.is_stroking() {
            self.refuse(Source::Action, crate::lang::refusals::during_stroke(lang));
            return;
        }
        if self.is_saving() {
            let text = lang.with_reason(&what, crate::lang::refusals::saving(lang));
            self.refuse(Source::Action, text);
            return;
        }
        self.automation.selected = Some(index);
        let result = {
            let mut host = crate::ops_host::AppHost::new(self);
            yolu_ops::action::run(&mut host, &commands)
        };
        match result {
            Ok(_) => self.info(
                Source::Action,
                lang.pick(
                    format!("アクション{}を再生しました。", lang.quote(&name)),
                    format!("Played action {}.", lang.quote(&name)),
                ),
            ),
            Err(e) => {
                let message = e
                    .message
                    .pick(ops_lang(lang))
                    .trim_end_matches(['。', '.'])
                    .to_owned();
                let at = e
                    .data
                    .as_ref()
                    .and_then(|d| d.get("index"))
                    .and_then(serde_json::Value::as_u64);
                let command = e
                    .data
                    .as_ref()
                    .and_then(|d| d.get("command"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let reason = match at {
                    Some(i) => lang.pick(
                        format!("{} 番目の {command} — {message}", i + 1),
                        format!("command {} {command} — {message}", i + 1),
                    ),
                    None => message,
                };
                let text = lang.with_reason(&what, reason);
                match e.code {
                    yolu_ops::ErrorCode::Busy | yolu_ops::ErrorCode::ReadOnly => {
                        self.refuse(Source::Action, text)
                    }
                    _ => self.fail(Source::Action, text),
                }
            }
        }
    }
}
