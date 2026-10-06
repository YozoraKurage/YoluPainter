//! 命令を実行するホストの口（`OpHost`）と、命令を振り分ける `execute`。
//!
//! 文書の命令は `doc_ops`（`Document` の上の関数）に、プロジェクトの命令（セットの一覧・文書の読み書き・書き出し・保存・見本）はこのトレイトに向けて
//! 書く。ホストは 2 つ: 画面なしの [`crate::FileHost`]（.ylp を開いて操作し、保存は yolu-io の安全な保存）と、起動中のアプリの中のホスト
//! （開いている文書・今のセット・取り消しの道・裏の保存へ。アプリの側で書く）。どちらも同じ命令・同じ返事・同じ誤りを返す。

use std::path::{Path, PathBuf};

use yolu_core::Document;

use crate::command::*;
use crate::doc_ops::{self, SetFacts};
use crate::error::{ErrorCode, OpError};
use crate::meta::{command_spec, Danger};
use crate::path::PathPolicy;
use crate::reply::*;
use crate::text::Text;

/// 読むために渡すセット。
pub struct SetView<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// 編集できるセットの文書。読むだけのセット（core で扱えない中身がある）は None。
    pub doc: Option<&'a Document>,
    /// 読むだけの理由。
    pub read_only: Option<&'a Text>,
    /// 読むだけのセットの大きさ・層の数（文書を読めなくても、正本の骨組みから分かる）。
    pub size: (u32, u32),
    pub layer_count: u32,
    /// 開いた・保存したあとに編集した。
    pub unsaved: bool,
    /// 書き出しのファイル名の既定（開いている .ylp の名前から拡張子を除いたもの）。
    pub stem: &'a str,
    /// プロジェクトのセットの数（書き出しのファイル名にセット名を入れるか）。
    pub set_count: usize,
    /// 層の画素に許すバイト数（PSD の書き出しの予算）。
    pub source_budget: u64,
}

impl SetView<'_> {
    pub fn facts(&self) -> SetFacts<'_> {
        SetFacts {
            id: self.id,
            name: self.name,
            unsaved: self.unsaved,
        }
    }
    /// 編集できるセットの文書。読むだけのセットは理由つきで断る。
    pub fn editable_doc(&self) -> Result<&Document, OpError> {
        self.doc
            .ok_or_else(|| read_only_error(self.name, self.read_only))
    }
}

/// 読むだけのセットを断る誤り。
pub fn read_only_error(name: &str, reason: Option<&Text>) -> OpError {
    let (why_ja, why_en) = reason.map_or((String::new(), String::new()), |t| {
        (format!("（{}）", t.ja), format!(" ({})", t.en))
    });
    OpError::new(
        ErrorCode::ReadOnly,
        format!("テクスチャセット「{name}」は読むだけです{why_ja}"),
        format!("Texture set \"{name}\" is read-only{why_en}"),
    )
}

/// 保存の頼み。
#[derive(Clone, Debug)]
pub enum SaveJob {
    /// 開いている .ylp へ上書き。
    InPlace { confirm: bool },
    /// 別の .ylp へ（`path` は道の決まりで解決済み）。
    As { path: PathBuf, confirm: bool },
}

/// 書き出しの頼み。
#[derive(Clone, Copy, Debug)]
pub enum ExportJob<'a> {
    Channels(&'a ExportChannelsArgs),
    Textures(&'a ExportTexturesArgs),
    Psd(&'a ExportPsdArgs),
}

impl ExportJob<'_> {
    pub fn set(&self) -> Option<&str> {
        match self {
            ExportJob::Channels(a) => a.set.as_deref(),
            ExportJob::Textures(a) => a.set.as_deref(),
            ExportJob::Psd(a) => a.set.as_deref(),
        }
    }
}

/// 命令を実行するホスト。
pub trait OpHost {
    /// ファイルの道の決まり。
    fn policy(&self) -> &PathPolicy;
    /// 開いている文書の情報。無ければ `no_document`。
    fn doc_info(&mut self) -> Result<DocInfo, OpError>;
    /// 文書を開く（`doc.open`）。開いていた文書に保存していない変更があれば、`confirm` が無ければ断る。ディスクのファイルには触らない。
    /// 起動中のアプリのホストは、開いている文書が相手なので、断るか今の文書を返す。
    fn open(&mut self, path: &Path, confirm: bool) -> Result<DocInfo, OpError>;
    /// セット（省略は今のセット）の文書を読む。
    fn read_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(&SetView<'_>) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError>;
    /// セットの文書を書き換える（読むだけのセットは断る。書き換えたら「保存していない」にする）。
    fn write_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(SetFacts<'_>, &mut Document) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError>;
    /// 保存する。
    fn save(&mut self, job: &SaveJob) -> Result<Reply, OpError>;
    /// 書き出す。既定は、セットの文書を読んで `crate::export` の関数を通す。
    fn export(&mut self, job: &ExportJob<'_>) -> Result<Reply, OpError> {
        let policy = self.policy().clone();
        self.read_set(job.set(), &mut |view| {
            crate::export::run(view, &policy, job)
        })
    }
    /// 見本の画像。既定は、セットの文書を読んで `crate::preview` を通す。
    fn preview(&mut self, args: &PreviewArgs) -> Result<Reply, OpError> {
        self.read_set(args.set.as_deref(), &mut |view| {
            crate::preview::render(view, args)
        })
    }
}

/// 壊す操作に `confirm: true` があるか（`Danger::Always` の命令。置き換えるときだけ壊す命令は、置き換える所で確かめる）。
fn check_confirm(command: &Command) -> Result<(), OpError> {
    let Some(spec) = command_spec(command.name()) else {
        return Ok(());
    };
    if spec.danger == Danger::Always && command.confirmed() != Some(true) {
        return Err(OpError::confirm_required(
            format!(
                "{} は壊す操作です。confirm: true を付けてください",
                command.name()
            ),
            format!("{} is destructive; pass confirm: true", command.name()),
            Some(serde_json::json!({"command": command.name()})),
        ));
    }
    Ok(())
}

/// 命令を実行する。壊す操作の確認 → 振り分け。断った命令は何も変えない。
///
/// 途中の panic は `internal` の誤りにして返す（呼び手のプロセス・つながりを落とさない）。文書の編集は `Document::batch` が積んだ段を戻してから
/// panic を返すので、文書は編集の前のまま。
pub fn execute(host: &mut dyn OpHost, command: &Command) -> Result<Reply, OpError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(host, command)))
        .unwrap_or_else(|payload| {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(OpError::new(
                ErrorCode::Internal,
                format!("{} の途中で想定していない失敗が起きました", command.name()),
                format!(
                    "{} stopped because of an unexpected failure",
                    command.name()
                ),
            )
            .with_data(serde_json::json!({"command": command.name(), "detail": detail})))
        })
}

fn dispatch(host: &mut dyn OpHost, command: &Command) -> Result<Reply, OpError> {
    check_confirm(command)?;
    match command {
        Command::DocInfo(_) => host.doc_info().map(Reply::Doc),
        Command::DocOpen(a) => {
            let path = host.policy().resolve(&a.path)?;
            host.open(&path, a.confirm).map(Reply::Doc)
        }
        Command::EffectListKinds(_) => Ok(Reply::Kinds(doc_ops::kinds_info())),
        Command::Save(a) => host.save(&SaveJob::InPlace { confirm: a.confirm }),
        Command::SaveAs(a) => {
            let path = host.policy().resolve(&a.path)?;
            host.save(&SaveJob::As {
                path,
                confirm: a.confirm,
            })
        }
        Command::Preview(a) => host.preview(a),
        Command::ExportChannels(a) => host.export(&ExportJob::Channels(a)),
        Command::ExportTextures(a) => host.export(&ExportJob::Textures(a)),
        Command::ExportPsd(a) => host.export(&ExportJob::Psd(a)),
        c if doc_ops::is_write(c) => {
            host.write_set(c.set(), &mut |facts, doc| doc_ops::write(facts, doc, c))
        }
        c => host.read_set(c.set(), &mut |view| match view.doc {
            Some(doc) => doc_ops::read(view.facts(), doc, c),
            None => read_only_read(view, c),
        }),
    }
}

/// 読むだけのセットに読む命令を当てる: `set.info` は大きさと理由だけを返し、ほかは理由つきで断る。
fn read_only_read(view: &SetView<'_>, command: &Command) -> Result<Reply, OpError> {
    match command {
        Command::SetInfo(_) => Ok(Reply::Set(SetInfo {
            id: view.id.to_owned(),
            name: view.name.to_owned(),
            width: view.size.0,
            height: view.size.1,
            state: SetState::ReadOnly,
            reason: view.read_only.cloned(),
            channels: Vec::new(),
            layers: Vec::new(),
            inactive_effects: Vec::new(),
            unsaved: view.unsaved,
        })),
        _ => Err(read_only_error(view.name, view.read_only)),
    }
}
