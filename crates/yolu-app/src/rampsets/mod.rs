//! グラデーションセット（CLIP STUDIO の「グラデーションセット」）: ランプの見本の一覧を、組（グループ）ごとに持つ。組み込みの組（`builtin`）と、
//! 利用者の組（設定のフォルダに保存。`store`）があり、グラデーションマップ・塗りつぶしのグラデーションのランプの欄が同じ一覧を使う。
//! セットは分岐点の並び（色・不透明度・中点・混合率曲線）と値のカーブで、混色モード・輝度の補正は含めない（当てるときは今の設定を残す）。
//! 状態は画面のもので、.ylp には入れない。起動のとき読めなかったファイルがあるあいだは、そのファイルを守るため、足す・名前を変える・消すを断る。

pub mod builtin;
pub mod store;

use std::path::PathBuf;

use yolu_core::generator::{LuminanceCorrection, MixMode, Ramp};

pub use store::{RampSetStore, StoreError, MAX_USER};

use crate::brushes::clean_name;
use crate::lang::Lang;

/// 利用者の 1 つ（名前とランプ）。
#[derive(Clone, Debug, PartialEq)]
pub struct UserRamp {
    pub name: String,
    pub ramp: Ramp,
}

/// 追加・名前の変更・削除の失敗。
#[derive(Debug)]
pub enum SetsError {
    /// 数の上限（`MAX_USER`）。
    TooMany,
    /// 名前が空。
    Name,
    /// 範囲外の番号（組み込みの組は変えられない）。
    Index,
    /// 起動のとき読めなかったファイルが設定のフォルダに残っている（上書きしないので、足す・名前を変える・消すはしない）。理由は日本語・英語の順。
    Unreadable(String, String),
    Store(StoreError),
}

impl SetsError {
    pub fn describe(&self, lang: Lang) -> String {
        match self {
            SetsError::TooMany => lang.pick(
                format!("自分のグラデーションは {MAX_USER} 個までです"),
                format!("At most {MAX_USER} gradients of your own"),
            ),
            SetsError::Name => lang.pick("名前が空です".to_owned(), "The name is empty".to_owned()),
            SetsError::Index => lang.pick(
                "組み込みのものは変えられません".to_owned(),
                "Built-in gradients cannot be changed".to_owned(),
            ),
            SetsError::Unreadable(ja, en) => lang.pick(
                format!("ファイルが読めないままなので、グラデーションを保存できません（{ja}）。"),
                format!("Cannot save the gradients because the file is still unreadable ({en})."),
            ),
            SetsError::Store(e) => lang.with_reason(
                lang.pick(
                    "グラデーションを保存できません",
                    "Cannot save the gradients",
                ),
                e.describe(lang),
            ),
        }
    }
}

/// 一覧の 1 つ分（見せる名前とランプ）。
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub ramp: Ramp,
}

/// グラデーションセットの状態（画面のもの）。
#[derive(Debug, Default)]
pub struct RampSets {
    /// 見せている組（0〜組み込みの数 − 1 が組み込み、最後が利用者の組）。
    pub group: usize,
    /// 一覧で選んでいる 1 つ（見せている組の中の番号）。
    pub selected: Option<usize>,
    user: Vec<UserRamp>,
    store: Option<RampSetStore>,
    /// 起動のとき読めなかった理由（読めないファイルは触らず、保存もしない）。
    problem: Option<StoreError>,
}

impl RampSets {
    /// 組の数（組み込み + 利用者）。
    pub fn group_count() -> usize {
        builtin::groups([0.0; 4], [0.0; 4]).len() + 1
    }

    /// 利用者の組の番号。
    pub fn user_group() -> usize {
        Self::group_count() - 1
    }

    /// 設定のフォルダのセットを読む（起動のとき 1 回）。読めなければ空で始め、そのファイルへは保存しない（触らない）。
    pub fn attach(&mut self, dir: PathBuf) {
        let store = RampSetStore::new(dir);
        match store.load() {
            Ok(user) => {
                self.user = user;
                self.problem = None;
            }
            Err(e) => {
                self.user.clear();
                self.problem = Some(e);
            }
        }
        self.store = Some(store);
    }

    /// 起動のとき読めなかった理由。
    pub fn problem(&self) -> Option<&StoreError> {
        self.problem.as_ref()
    }

    /// 利用者のセット。
    pub fn user(&self) -> &[UserRamp] {
        &self.user
    }

    /// 組の名前（組み込みは日本語・英語、利用者の組は「自分のグラデーション」）。
    pub fn group_name(lang: Lang, group: usize) -> String {
        let builtins = builtin::groups([0.0; 4], [0.0; 4]);
        match builtins.get(group) {
            Some(g) => lang.pick(g.ja, g.en).to_owned(),
            None => lang.pick("自分", "Mine").to_owned(),
        }
    }

    /// 見せている組の一覧。`main`・`sub` は「基本」の見本のためのメインとサブの色。
    pub fn entries(&self, lang: Lang, main: [f32; 4], sub: [f32; 4]) -> Vec<Entry> {
        let builtins = builtin::groups(main, sub);
        match builtins.into_iter().nth(self.group) {
            Some(g) => g
                .items
                .into_iter()
                .map(|b| Entry {
                    name: lang.pick(b.ja, b.en).to_owned(),
                    ramp: b.ramp,
                })
                .collect(),
            None => self
                .user
                .iter()
                .map(|u| Entry {
                    name: u.name.clone(),
                    ramp: u.ramp.clone(),
                })
                .collect(),
        }
    }

    /// 見せている組が利用者の組か。
    pub fn showing_user(&self) -> bool {
        self.group == Self::user_group()
    }

    fn persist(&mut self) -> Result<(), SetsError> {
        // 読めなかったファイルは上書きしない。保存できないまま一覧にだけ残すと、再起動で黙って消えるので、足す・名前を変える・消すを断って理由を出す
        if let Some(e) = &self.problem {
            return Err(SetsError::Unreadable(
                e.describe(Lang::Ja),
                e.describe(Lang::En),
            ));
        }
        match &self.store {
            Some(store) => store.save(&self.user).map_err(SetsError::Store),
            None => Ok(()),
        }
    }

    /// 今のランプを利用者の組へ足す（名前が空なら「グラデーション N」）。足した番号を返す。保存に失敗したら（読めなかったファイルがあるときも）足さない。
    /// セットには混色モード・輝度の補正を入れない（外して足す）。
    pub fn add(&mut self, name: &str, ramp: &Ramp, lang: Lang) -> Result<usize, SetsError> {
        if self.user.len() >= MAX_USER {
            return Err(SetsError::TooMany);
        }
        let name = clean_name(name).unwrap_or_else(|| {
            let n = self.user.len() + 1;
            lang.pick(format!("グラデーション {n}"), format!("Gradient {n}"))
        });
        // 分岐点の並び・混合率曲線・値のカーブは入れ、混色モードと輝度の補正は外す（セットの外の設定）
        let ramp = ramp.with_mixing(MixMode::Standard, LuminanceCorrection::default());
        self.user.push(UserRamp { name, ramp });
        if let Err(e) = self.persist() {
            self.user.pop();
            return Err(e);
        }
        Ok(self.user.len() - 1)
    }

    /// 利用者のセットの名前を変える。
    pub fn rename(&mut self, index: usize, name: &str) -> Result<(), SetsError> {
        let name = clean_name(name).ok_or(SetsError::Name)?;
        let slot = self.user.get_mut(index).ok_or(SetsError::Index)?;
        let old = std::mem::replace(&mut slot.name, name);
        if let Err(e) = self.persist() {
            self.user[index].name = old;
            return Err(e);
        }
        Ok(())
    }

    /// 利用者のセットを消す。
    pub fn remove(&mut self, index: usize) -> Result<(), SetsError> {
        if index >= self.user.len() {
            return Err(SetsError::Index);
        }
        let old = self.user.remove(index);
        if let Err(e) = self.persist() {
            self.user.insert(index, old);
            return Err(e);
        }
        Ok(())
    }

    /// 組を替える（選びは外す）。
    pub fn show_group(&mut self, group: usize) {
        self.group = group.min(Self::user_group());
        self.selected = None;
    }
}

#[cfg(test)]
mod tests;
