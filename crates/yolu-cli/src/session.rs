//! MCP サーバーが持つ、開いた .ylp の組（`file` の引数で指された物）。
//!
//! 1 つの .ylp ごとに 1 つの `FileHost`（文書をメモリに開き、編集は取り消しの段になり、保存は yolu-io の安全な保存）を持ち、同じ道への
//! 次の呼び出しで使い回す。サーバーが終われば消える（保存していない編集は、保存の命令を出さなければファイルに入らない）。
//!
//! - 相対パスの起点は、その .ylp のあるフォルダ（`dir`・`path` の相対は、プロジェクトの隣）。MCP のクライアントがサーバーを起こす
//!   フォルダは決まっていないので、今のフォルダを起点にしない。`..` で出る道は yolu-ops が断る。
//! - 保存していない編集が無く、外でファイルが書き換わっていれば（アプリで保存した・ほかの道具で書いた）、次の呼び出しで開き直す
//!   （古い中身を読ませない）。編集があるときは開き直さず、保存が衝突として断る。編集を捨てて開き直すのは `doc_open`（`confirm: true`）。
//! - 組は、ホストが今開いているファイルに結び付く: 名前を付けて保存（`save_as`）や `doc_open` で別のファイルへ移ったら、組もその道に付け替わる
//!   （以後その文書は新しい道の `file` で指す。元の道の `file` は、元のファイルを開き直した別の組になる）。付け替え先を別の組がもう開いて
//!   いれば、保存していない編集の無いときだけ 1 つにまとめ、あるときは書く前に断る。相対パスの起点は、最初に開いたときの .ylp のフォルダのまま。
//! - 同時に開く数に上限（`MAX_OPEN`）。満杯なら、保存していない編集の無い物のうち一番使っていない物を閉じ、全部が編集中なら断る。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::json;
use yolu_ops::{execute, Command, ErrorCode, FileHost, OpError, OpHost, PathPolicy, Reply};

/// 同時に開いておく .ylp の数。
pub const MAX_OPEN: usize = 8;

/// ファイルの見分け（大きさと更新時刻）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let m = std::fs::metadata(path).ok()?;
    Some(Fingerprint {
        len: m.len(),
        modified: m.modified().ok(),
    })
}

struct Entry {
    path: PathBuf,
    host: FileHost,
    seen: Option<Fingerprint>,
    used: u64,
}

/// 開いた .ylp の組。
#[derive(Default)]
pub struct Sessions {
    entries: Vec<Entry>,
    clock: u64,
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `file`（絶対か、`base` からの相対）の .ylp のホストで命令を行う。まだ開いていなければ開く。
    pub fn run(&mut self, base: &Path, file: &str, command: &Command) -> Result<Reply, OpError> {
        let path = PathPolicy::new(base)
            .map_err(|e| io_error(&e))?
            .resolve(file)?;
        self.clock += 1;
        let clock = self.clock;
        let index = self.entry(&path, clock)?;
        // ホストが別のファイルへ移る命令: 移り先を別の組が編集中なら、書く（置き換える）前に断る
        if let Command::SaveAs(a) = command {
            self.refuse_when_edited_elsewhere(index, &a.path)?;
        }
        if let Command::DocOpen(a) = command {
            self.refuse_when_edited_elsewhere(index, &a.path)?;
        }
        let result = execute(&mut self.entries[index].host, command);
        if result.is_ok()
            && matches!(
                command,
                Command::Save(_) | Command::SaveAs(_) | Command::DocOpen(_)
            )
        {
            self.settle(index);
        }
        result
    }

    /// `path` の組の番号。まだ開いていなければ開き、編集が無く外で書き換わっていれば開き直す。
    fn entry(&mut self, path: &Path, clock: u64) -> Result<usize, OpError> {
        let index = match self.entries.iter().position(|e| same_file(&e.path, path)) {
            Some(i) => {
                let entry = &mut self.entries[i];
                // 編集が無く、外で書き換わったなら開き直す
                if !entry.host.has_unsaved_changes() && entry.seen != fingerprint(&entry.path) {
                    let reopened = open(&entry.path)?;
                    entry.host = reopened;
                    entry.seen = fingerprint(&entry.path);
                }
                i
            }
            None => {
                let host = open(path)?;
                self.make_room()?;
                self.entries.push(Entry {
                    seen: fingerprint(path),
                    path: path.to_path_buf(),
                    host,
                    used: clock,
                });
                self.entries.len() - 1
            }
        };
        self.entries[index].used = clock;
        Ok(index)
    }

    /// `index` の組のホストが `destination`（その組の道の決まりで解く）へ移るとき、同じファイルを別の組が保存していない編集つきで
    /// 開いていれば断る（その編集は、移った先で上書きされて、保存では取り返せなくなる）。
    fn refuse_when_edited_elsewhere(&self, index: usize, destination: &str) -> Result<(), OpError> {
        // 道が使えない形のときは、命令が同じ誤りで断るのでここでは何も言わない
        let Ok(target) = self.entries[index].host.policy().resolve(destination) else {
            return Ok(());
        };
        let held = self.entries.iter().enumerate().find(|(i, e)| {
            *i != index && e.host.has_unsaved_changes() && same_file(&e.path, &target)
        });
        match held {
            Some((_, other)) => Err(OpError::new(
                ErrorCode::Conflict,
                format!(
                    "「{}」は別の呼び出しで保存していない編集つきで開いています。先にそれを save するか、別の名前にしてください",
                    other.path.display()
                ),
                format!(
                    "\"{}\" is open with unsaved edits under another file argument; save it first or choose another name",
                    other.path.display()
                ),
            )
            .with_data(json!({"file": other.path.display().to_string()}))),
            None => Ok(()),
        }
    }

    /// 保存・名前を付けて保存・文書を開く命令のあとに、組を整える: ホストが今開いているファイルへ道を付け替え、見分けを取り直し
    /// （自分で書いたファイルを「外の書き換え」と見ない）、同じファイルを指す別の組は 1 つにまとめる。
    fn settle(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        if let Some(now) = entry.host.path() {
            if !same_file(now, &entry.path) {
                entry.path = now.to_path_buf();
            }
        }
        entry.seen = fingerprint(&entry.path);
        let path = entry.path.clone();
        let entries = std::mem::take(&mut self.entries);
        self.entries = entries
            .into_iter()
            .enumerate()
            .filter(|(i, e)| *i == index || !same_file(&e.path, &path))
            .map(|(_, e)| e)
            .collect();
    }

    fn make_room(&mut self) -> Result<(), OpError> {
        while self.entries.len() >= MAX_OPEN {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.host.has_unsaved_changes())
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.entries.remove(i);
                }
                None => {
                    return Err(OpError::new(
                        ErrorCode::Budget,
                        format!("開いている .ylp が {MAX_OPEN} 個とも保存していない編集を持っています。どれかを save してから別の .ylp を開いてください"),
                        format!("All {MAX_OPEN} open .ylp files have unsaved edits; save one of them before opening another"),
                    )
                    .with_data(json!({"open": self.entries.iter().map(|e| e.path.display().to_string()).collect::<Vec<_>>()})))
                }
            }
        }
        Ok(())
    }
}

fn open(path: &Path) -> Result<FileHost, OpError> {
    let base = path.parent().unwrap_or(Path::new("."));
    let mut host = FileHost::new(PathPolicy::new(base).map_err(|e| io_error(&e))?);
    host.open(path, true)?;
    Ok(host)
}

fn io_error(e: &std::io::Error) -> OpError {
    OpError::new(
        ErrorCode::Io,
        format!("パスを扱えません: {e}"),
        format!("Cannot handle the path: {e}"),
    )
}
