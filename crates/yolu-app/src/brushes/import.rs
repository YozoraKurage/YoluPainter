//! ブラシのファイルの取り込み（ABR・GBR・GIH・VBR・PNG・PAT・CLIP STUDIO の SUT）を、画面を止めずに行う。
//!
//! 読み込み（`yolu_io::brushes::import`）と保存（画像・ブラシのファイル）は別のスレッドで、1 回の取り込み（ファイルの並び）が 1 つの仕事。
//! ファイルごとに「読む → ブラシにして番号と名前を決める → 置く」を済ませ、置けたブラシだけを画面の側へ送る。画面の側は届いた分を
//! 一覧に足すだけなので、一覧に出ているブラシはいつもファイルがある（一覧とファイルが食い違わない）。途中で取り消しても、置いた分は
//! 残る。ブラシの番号は画面で足すブラシと同じ口（`IdSource`）から取る。
//! 取り込んだブラシは「取り込み」のグループの後ろに足す（名前が重なれば番号を付ける）。表せなかった設定は項目の一覧にして
//! ブラシに付ける（`Gap`）。ファイルの中のプリセットが取り込めなかった分・使えなかった模様は、数えて知らせる。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use yolu_io::brushes::{self, BrushImportError, Unrepresented};

use super::store::{BrushStore, StoreError};
use super::{
    clean_name, gaps, BrushAction, BrushKey, Entry, Group, IdSource, ImportMeta, UserBrush,
};
use crate::jobs::{JobCard, JobSpec, Polled, Worker};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState, DialogRequest, MAX_RADIUS};
use crate::windows::CloseJob;

/// 仕事の途中経過（進み具合の札が読む）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// 今読んでいるファイル。
    pub file: String,
    /// 何番目か（1 から）と全部の数。
    pub index: usize,
    pub total: usize,
    pub canceling: bool,
}

/// 取り込みの状態（走っている仕事は高々 1 つ）。
#[derive(Default)]
pub struct ImportState {
    job: Option<Job>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく（取消が効いたことを、仕事の速さに頼らず確かめるため）。
    #[doc(hidden)]
    pub park_next: bool,
    /// 試験用: N 個目のブラシを置いたあと、取消が来るまで止める（ファイルの途中・ファイルの間の取消で、置いた分が残ることを、
    /// 仕事の速さに頼らず確かめるため。次の仕事だけに効く）。
    #[doc(hidden)]
    pub park_after_brushes: Option<usize>,
}

struct Job {
    worker: Worker<Msg>,
    canceling: bool,
    file: String,
    index: usize,
    total: usize,
    report: Report,
}

/// 仕事全体の結果の積み上げ（終わったときの知らせの元）。
#[derive(Default)]
struct Report {
    files: usize,
    imported: usize,
    skipped: usize,
    capped: bool,
    save_error: Option<StoreError>,
    /// 読めなかったファイル（ファイル名と理由）。
    failed: Vec<(String, BrushImportError)>,
    /// 今回取り込んだ最初のブラシ（終わったら、これに替える）。
    first: Option<BrushKey>,
    last_file: String,
    /// 仕事が知らせずに止まった（スレッドの異常）。
    stopped: bool,
}

enum Msg {
    Reading { file: String, index: usize },
    Done(FileDone),
    Finished,
}

struct FileDone {
    file: String,
    result: Result<Loaded, BrushImportError>,
}

/// 1 つのファイルから置けたブラシ。
struct Loaded {
    brushes: Vec<UserBrush>,
    /// 取り込めなかったプリセット・使えなかった模様の数。
    skipped: usize,
    /// ブラシの数の上限で止めた。
    capped: bool,
    /// 保存できなくて止めた（ここまでに置けた分は `brushes` に入っている）。
    save_error: Option<StoreError>,
}

/// 仕事の入れ物（別のスレッドへ渡す）。
struct Work {
    paths: Vec<PathBuf>,
    store: Option<BrushStore>,
    ids: IdSource,
    /// 今ある利用者のブラシの名前（重ならない名前を付けるため）。
    names: HashSet<String>,
    /// あと何個足せるか。
    room: usize,
    park: bool,
    /// 試験用: 置いた数がこれになったら止める。
    park_after: Option<usize>,
    /// 今回置いた数。
    placed: usize,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 重ならない名前（上限の文字数に収める）。
fn unique_name(base: &str, taken: &mut HashSet<String>) -> String {
    let base = clean_name(base).unwrap_or_else(|| "Imported".into());
    let mut name = base.clone();
    let mut n = 1;
    while taken.contains(&name) {
        n += 1;
        let suffix = format!(" {n}");
        let room = super::MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
        name = format!("{}{suffix}", base.chars().take(room).collect::<String>());
    }
    taken.insert(name.clone());
    name
}

/// ファイルを読んでブラシにして置く（別のスレッド）。
fn load_file(
    path: &Path,
    work: &mut Work,
    cancel: &AtomicBool,
) -> Result<Loaded, BrushImportError> {
    let set = brushes::import(path)?;
    // ファイル全体の注記は、どのブラシにも当てはまる（設定の節を読めなかった・知らない節）ので全ブラシの項目に足す。
    // 使えなかった模様はどのブラシの項目でもないので、取り込めなかった数に入れる
    let file_gaps = gaps::fold(&set.notes);
    let skipped = set.skipped.len()
        + set
            .notes
            .iter()
            .map(|n| match n {
                Unrepresented::PatternSkipped { .. } => 1,
                // 多すぎて読まなかった .sut のブラシ
                Unrepresented::ClipStudio(brushes::SutNote::BrushesCapped(count)) => *count,
                _ => 0,
            })
            .sum::<usize>();
    let mut loaded = Loaded {
        brushes: Vec::new(),
        skipped,
        capped: false,
        save_error: None,
    };
    for imported in set.brushes {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if work.room == 0 {
            loaded.capped = true;
            break;
        }
        let store = work.store.as_ref();
        let Some(id) = work.ids.take(|id| store.is_some_and(|s| s.is_taken(id))) else {
            loaded.capped = true;
            break;
        };
        let mut brush = imported.brush;
        // 手ぶれ補正と入り抜きは描き手の設定でブラシに入らない。取り込んだブラシが持つ分は、ブラシの外に持たせる（選んでいる間だけ重ねる）
        let assist = super::carried_assist(Some(brush.assist));
        // 取り込んだ筆先の原寸（数百画素）がアプリの直径の上限を超えないように
        brush.base.radius = brush.base.radius.clamp(0.5, MAX_RADIUS as f64);
        let mut notes = gaps::fold(&imported.unrepresented);
        notes.extend(&file_gaps);
        let user = UserBrush {
            id,
            name: unique_name(&imported.name, &mut work.names),
            group: Group::Imported,
            brush,
            assist,
            import: Some(
                ImportMeta::new(
                    imported.source.label(),
                    matches!(imported.source, brushes::Source::PhotoshopPattern),
                    notes,
                )
                .with_mapped(imported.mapped),
            ),
        };
        if let Some(store) = store {
            if let Err(e) = store.save_brush(&user) {
                // 同じ理由で次も置けない。ここまでに置けた分は残す
                loaded.save_error = Some(e);
                break;
            }
        }
        work.room -= 1;
        work.placed += 1;
        loaded.brushes.push(user);
        if work.park_after == Some(work.placed) {
            crate::windows::park_until_canceled(cancel);
        }
    }
    Ok(loaded)
}

fn run(mut work: Work, cancel: &AtomicBool, tx: &Sender<Msg>) {
    if work.park {
        crate::windows::park_until_canceled(cancel);
    }
    let paths = std::mem::take(&mut work.paths);
    for (i, path) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let file = file_name(path);
        let _ = tx.send(Msg::Reading {
            file: file.clone(),
            index: i + 1,
        });
        let result = load_file(path, &mut work, cancel);
        let stop = result
            .as_ref()
            .is_ok_and(|l| l.capped || l.save_error.is_some());
        let _ = tx.send(Msg::Done(FileDone { file, result }));
        if stop {
            break;
        }
    }
    let _ = tx.send(Msg::Finished);
}

impl ImportState {
    /// 仕事が走っているか。
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn progress(&self) -> Option<Progress> {
        self.job.as_ref().map(|j| Progress {
            file: j.file.clone(),
            index: j.index,
            total: j.total,
            canceling: j.canceling,
        })
    }
}

/// ブラシの取り込み（札・閉じる前の確かめ・止める）。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(|app, lang| {
        let p = app.brushes.import.progress()?;
        Some(JobCard {
            text: format!(
                "{} — {}{}",
                lang.pick("ブラシを取り込み中", "Importing brushes"),
                p.file,
                if p.total > 1 {
                    format!(" ({}/{})", p.index, p.total)
                } else {
                    String::new()
                }
            ),
            fraction: None,
            cancel: Some(Action::Brush(super::BrushAction::ImportCancel)),
            canceling: p.canceling,
        })
    }),
    close: Some(|app| {
        app.brushes
            .import
            .is_busy()
            .then_some(CloseJob::BrushImport)
    }),
    cancel: Some(|app| app.apply(Action::Brush(super::BrushAction::ImportCancel))),
    poll_while_stopping: Some(AppState::poll_brush_import),
    ..JobSpec::new("brush-import", |app| app.brushes.import.is_busy())
};

impl AppState {
    /// 取り込む窓を開く頼み。
    pub(super) fn brush_import_dialog(&mut self) {
        self.dialog_request = Some(DialogRequest::ImportBrushes);
    }

    pub fn is_brush_importing(&self) -> bool {
        self.brushes.import.is_busy()
    }

    /// ファイルの取り込みを始める（読んで置くのは別のスレッド）。
    pub(super) fn brush_import_start(&mut self, paths: Vec<PathBuf>) {
        let lang = self.lang;
        if paths.is_empty() {
            return;
        }
        if self.brushes.import.is_busy() {
            self.brush_refuse_while_importing();
            return;
        }
        let user_count = self.brushes.lib.user_count();
        if user_count >= super::MAX_USER_BRUSHES {
            self.refuse(
                Source::Brush,
                lang.pick(
                    format!("ブラシは {} 個までです。", super::MAX_USER_BRUSHES),
                    format!("At most {} brushes.", super::MAX_USER_BRUSHES),
                ),
            );
            return;
        }
        let names: HashSet<String> = self
            .brushes
            .lib
            .entries()
            .iter()
            .filter(|e| e.key.is_user())
            .map(|e| e.name.clone())
            .collect();
        let park = std::mem::take(&mut self.brushes.import.park_next);
        let park_after = self.brushes.import.park_after_brushes.take();
        let work = Work {
            paths: paths.clone(),
            store: self.brushes.store.clone(),
            ids: self.brushes.lib.id_source(),
            names,
            room: super::MAX_USER_BRUSHES - user_count,
            park,
            park_after,
            placed: 0,
        };
        let spawned = Worker::spawn("yolu-brush-import", move |tx, cancel| {
            run(work, cancel.flag(), &tx)
        });
        let worker = match spawned {
            Ok(w) => w,
            Err(e) => {
                self.fail(
                    Source::Brush,
                    lang.with_reason(
                        lang.pick("ブラシを取り込めません", "Cannot import the brushes"),
                        lang.thread_error(&e),
                    ),
                );
                return;
            }
        };
        let first = paths.first().map(|p| file_name(p)).unwrap_or_default();
        self.info(
            Source::Brush,
            lang.pick(
                format!("ブラシを取り込み中: {first}"),
                format!("Importing brushes: {first}"),
            ),
        );
        self.brushes.import.job = Some(Job {
            worker,
            canceling: false,
            file: first,
            index: 1,
            total: paths.len(),
            report: Report::default(),
        });
    }

    /// 取り込みをやめる（置いた分は残る）。
    pub(super) fn brush_import_cancel(&mut self) {
        let lang = self.lang;
        if let Some(job) = &mut self.brushes.import.job {
            job.worker.cancel();
            job.canceling = true;
            self.info(
                Source::Brush,
                lang.pick("取り込みを取り消しています…", "Canceling the import…"),
            );
        }
    }

    /// 別のスレッドの取り込みから届いたものを受ける（フレームの初めに）。
    pub fn poll_brush_import(&mut self) {
        loop {
            let Some(job) = &mut self.brushes.import.job else {
                return;
            };
            let msg = match job.worker.poll() {
                Polled::Message(m) => m,
                Polled::Empty => return,
                // 仕事が知らせずに止まった（スレッドの異常）。届いた分で終えて、止まったことを知らせる
                Polled::Lost => {
                    job.report.stopped = true;
                    Msg::Finished
                }
            };
            match msg {
                Msg::Reading { file, index } => {
                    job.file = file;
                    job.index = index;
                }
                Msg::Done(done) => self.brush_import_file_done(done),
                Msg::Finished => {
                    self.brush_import_finish();
                    return;
                }
            }
        }
    }

    fn brush_import_file_done(&mut self, done: FileDone) {
        let Some(job) = &mut self.brushes.import.job else {
            return;
        };
        job.report.files += 1;
        job.report.last_file = done.file.clone();
        match done.result {
            Err(error) => job.report.failed.push((done.file, error)),
            Ok(loaded) => {
                job.report.skipped += loaded.skipped;
                job.report.capped |= loaded.capped;
                if loaded.save_error.is_some() {
                    job.report.save_error = loaded.save_error;
                }
                let lib = &mut self.brushes.lib;
                for user in loaded.brushes {
                    let key = BrushKey::User(user.id);
                    // 取り込んだブラシは「取り込み」のグループの後ろ（無ければ一覧の最後）
                    let at = lib
                        .entries
                        .iter()
                        .rposition(|e| e.group == Group::Imported)
                        .map_or(lib.entries.len(), |i| i + 1);
                    lib.entries.insert(
                        at,
                        Entry {
                            key,
                            name: user.name,
                            group: Group::Imported,
                            baseline: super::canonical(&user.brush),
                            edited: None,
                            import: user.import,
                            assist: user.assist,
                        },
                    );
                    job.report.imported += 1;
                    job.report.first.get_or_insert(key);
                }
            }
        }
    }

    /// 仕事の終わり: 並びを書き、最初のブラシに替え、結果を状態の帯へ。
    fn brush_import_finish(&mut self) {
        let Some(job) = self.brushes.import.job.take() else {
            return;
        };
        let lang = self.lang;
        let report = job.report;
        let mut order_error = None;
        if report.imported > 0 {
            // 並びを保存できなかった理由は、取り込みの知らせに添えて 1 回だけ知らせる
            if let Err(text) = self.brush_save_order() {
                order_error = Some(text);
            }
            if let Some(first) = report.first {
                if !self.is_stroking() {
                    self.brush_import_select(first);
                }
            }
        }
        let mut text = Self::brush_import_message(lang, &report, job.canceling);
        let mut kind = import_kind(&report);
        if let Some(order) = order_error.filter(|o| !o.is_empty()) {
            text.push_str(&lang.pick(format!("。{order}"), format!(". {order}")));
            kind = kind.worse(crate::notice::Kind::Warning);
        }
        self.notify(kind, Source::Brush, text);
    }

    /// 取り込みが終わったとき、取り込んだ最初のブラシに替える。道具は、描く道具（ブラシ・消しゴム）のときだけ従来どおりブラシに
    /// 合わせて替える。ほかの道具（選択・バケツなど）は、裏の仕事の終わりという利用者の操作でない出来事で奪わない
    /// （選択範囲の途中の形が消えるため）。そのときはブラシだけ替え、道具をブラシへ戻したときにそのブラシで描く。
    fn brush_import_select(&mut self, key: BrushKey) {
        if self.tool.paints() {
            self.brush_action(BrushAction::Select(key));
        } else if self.brushes.lib.entry(key).is_some() {
            self.brush_sync();
            self.brush_activate(key);
        }
    }

    fn brush_import_message(lang: Lang, r: &Report, canceled: bool) -> String {
        let reason = |e: &BrushImportError| crate::lang::brush_import_error(lang, e);
        // 1 つのファイルが読めなかっただけなら、その理由
        if r.imported == 0 && r.save_error.is_none() {
            if let Some((file, error)) = r.failed.first() {
                let file = lang.quote(file);
                if r.failed.len() == 1 {
                    return lang.with_reason(
                        lang.pick(
                            format!("{file}を取り込めません"),
                            format!("Cannot import {file}"),
                        ),
                        reason(error),
                    );
                }
                let unreadable = lang.with_reason(
                    lang.pick(format!("{file}は読めません"), format!("Cannot read {file}")),
                    reason(error),
                );
                return lang.pick(
                    format!(
                        "{} 個のファイルを取り込めません。{unreadable}",
                        r.failed.len()
                    ),
                    format!("Cannot import {} files. {unreadable}", r.failed.len()),
                );
            }
        }
        let n = r.imported;
        if r.stopped && n == 0 {
            return lang
                .pick("取り込みが止まりました。", "The import stopped.")
                .into();
        }
        let mut text = if r.files <= 1 {
            lang.pick(
                format!("ブラシを {n} 個取り込みました（{}）", r.last_file),
                format!(
                    "Imported {n} brush{} ({})",
                    if n == 1 { "" } else { "es" },
                    r.last_file
                ),
            )
        } else {
            lang.pick(
                format!("ブラシを {n} 個取り込みました（{} ファイル）", r.files),
                format!(
                    "Imported {n} brush{} ({} files)",
                    if n == 1 { "" } else { "es" },
                    r.files
                ),
            )
        };
        if canceled {
            text = if n == 0 {
                lang.pick("取り込みをやめました", "Import canceled").into()
            } else {
                lang.pick(
                    format!("取り込みをやめました（{n} 個は取り込み済み）"),
                    format!("Import canceled ({n} already imported)"),
                )
            };
        }
        if r.stopped {
            text.push_str(lang.pick("。取り込みが止まりました", ". The import stopped"));
        }
        if r.skipped > 0 {
            text.push_str(&lang.pick(
                format!("。{} 個は取り込めませんでした", r.skipped),
                format!(". {} could not be imported", r.skipped),
            ));
        }
        if let Some((file, error)) = r.failed.first() {
            let unreadable = lang.with_reason(
                lang.pick(
                    format!("{}は読めません", lang.quote(file)),
                    format!("Cannot read {}", lang.quote(file)),
                ),
                reason(error),
            );
            text.push_str(lang.pick("。", ". "));
            text.push_str(unreadable.trim_end_matches(['。', '.']));
        }
        if r.capped {
            text.push_str(&lang.pick(
                format!("。ブラシは {} 個までです", super::MAX_USER_BRUSHES),
                format!(". At most {} brushes", super::MAX_USER_BRUSHES),
            ));
        }
        if let Some(e) = &r.save_error {
            let unsaved = lang.with_reason(
                lang.pick("ブラシを保存できません", "Cannot save the brushes"),
                e.describe(lang),
            );
            text.push_str(lang.pick("。", ". "));
            text.push_str(unsaved.trim_end_matches(['。', '.']));
        }
        text
    }
}

/// 取り込みの知らせの種類: 1 つも入らなかった失敗（読めない・止まった・保存できない）は失敗、入ったが読めない・止まった・上限・
/// 保存できない物があれば注意、ほか（取り消しを含む）は済んだ知らせ。
fn import_kind(r: &Report) -> crate::notice::Kind {
    use crate::notice::Kind;
    let problems =
        r.stopped || r.skipped > 0 || !r.failed.is_empty() || r.capped || r.save_error.is_some();
    if r.imported == 0 && problems {
        Kind::Error
    } else if problems {
        Kind::Warning
    } else {
        Kind::Info
    }
}

/// 取り込みのファイルの種類か（ドロップされたファイルのうち、取り込みの対象にするもの。読めない種類 `.kpp` も理由を
/// 出すために通す）。
pub fn is_brush_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let e = e.to_ascii_lowercase();
        brushes::FileKind::EXTENSIONS.contains(&e.as_str()) || e == "kpp"
    })
}

/// 枠の外の PNG をブラシとして取り込まない種類（ドロップのとき、一覧の上に落とした PNG だけをブラシにする）。
pub fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_collide_get_a_number_and_stay_within_the_length_limit() {
        let mut taken: HashSet<String> = HashSet::new();
        assert_eq!(unique_name("Chalk", &mut taken), "Chalk");
        assert_eq!(unique_name("Chalk", &mut taken), "Chalk 2");
        assert_eq!(unique_name("Chalk", &mut taken), "Chalk 3");
        // 制御文字は空白になり、空の名前は仮の名前になる
        assert_eq!(unique_name("a\nb", &mut taken), "a b");
        assert_eq!(unique_name(" \n ", &mut taken), "Imported");
        // 上限いっぱいの名前に番号を付けても上限を超えない（切った名前が衝突しても番号で分かれる）
        let long = "あ".repeat(super::super::MAX_NAME_CHARS);
        let first = unique_name(&long, &mut taken);
        let second = unique_name(&long, &mut taken);
        let third = unique_name(&long, &mut taken);
        assert_eq!(first, long);
        assert!(second.ends_with(" 2") && third.ends_with(" 3"));
        for name in [&first, &second, &third] {
            assert!(
                name.chars().count() <= super::super::MAX_NAME_CHARS,
                "{name}"
            );
        }
        assert_eq!(taken.len(), 8);
    }

    #[test]
    fn only_brush_files_and_the_refused_kind_are_taken_from_a_drop() {
        for ok in [
            "a.abr", "a.GBR", "a.gih", "a.vbr", "a.pat", "a.png", "a.PNG", "a.kpp", "a.sut",
        ] {
            assert!(is_brush_file(Path::new(ok)), "{ok}");
        }
        for no in ["a.ylp", "a.psd", "a", "abr", "a.txt"] {
            assert!(!is_brush_file(Path::new(no)), "{no}");
        }
        assert!(is_png(Path::new("x.PnG")) && !is_png(Path::new("x.abr")));
    }
}
