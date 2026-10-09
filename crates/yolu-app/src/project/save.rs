//! .ylp の保存。画面のスレッドは、描いていないことの確認・セットごとの文書の写し・保存の頼みの組み立てと、結果を受けるところまで。重い所
//! （正本を作る・合成の PNG・ZIP を書く・確かめる・置き換える・退避）は裏のスレッドで動かす。保存の間も描ける・見られる。
//!
//! - **保存は写しから**: 頼みの時点の文書の写し（`capture_snapshot`。タイルは共有）から作る。保存の間に描いた分は保存に入らず、
//!   終わった後も「変更あり」のまま（保存済みの印は、写しを取った時点の文書の ID と版）。
//! - **保存の間は**、同じファイルへの次の保存・開く・新規・配布用に保存・更新の入れ替えは断る（理由を出す）。復旧の書き置きの新しい頼みも
//!   出さない（書き置きは開いた .ylp のハンドルから読むので、置き換えと重ねない。すでに動いている書き込みは、置換の前に裏のスレッドが待つ）。
//! - **終わったら** 画面のスレッドで結果を受けて、`ProjectFile`・セットの保存済みの印・メッセージを更新する。失敗したら何も変えず、
//!   保存の前の「変更あり」を戻して理由を出す（ファイルは yolu-io の安全な保存なので前の中身のまま）。
//! - **閉じる**ときは、保存が終わるのを待ってから（`YoluApp`）。
//! - 試験の状態（`AppState::new`）は、保存の頼みの中で同じ仕事を呼び手のスレッドで終えて結果を受ける（`background` が false）。実際のウィンドウ
//!   （`YoluApp::new`）だけ裏のスレッドで動かす。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use yolu_core::mesh_maps::BakedMeshMap;
use yolu_io::{BackupKeep, Project, SaveStage, SaveTarget};

use super::capture::{
    build, capture, left_out_note, BuildError, BuildProgress, Capture, THREAD_STACK,
};
use super::{backup_text, reopen_note, same_file, ProjectFile};
use crate::jobs::{JobCard, JobSpec};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

/// 保存の段の数（合成・組み立て、数える、書く、確かめる、置き換える）。
const STAGES: usize = 1 + SaveStage::COUNT;
/// 各段が全体に占める割合（合計 1。大きな文書で測った時間の見当。段の内側の進みは、合成だけがセットの数で細かくなる）。
const WEIGHTS: [f32; STAGES] = [0.15, 0.2, 0.4, 0.2, 0.05];

/// 裏の保存の状態。
#[derive(Default)]
pub struct SaveState {
    /// 保存を裏のスレッドで動かすか（実際のウィンドウは true）。false なら、保存の頼みの中で、同じ仕事を呼び手のスレッドで終えて結果を受ける。
    pub background: bool,
    job: Option<Job>,
    /// 試験用: 次の保存の仕事を、手が離されるまで始めずに止めておく。
    hold: Option<Arc<AtomicBool>>,
    /// 外からの操作が頼んだ保存の結果（`save_for_ops`。`take_outcome` が 1 度だけ渡す）。
    outcome: Option<SaveOutcome>,
}

/// 外からの操作（`save_for_ops`）が頼んだ保存の結果。画面の保存（メニュー・Ctrl+S）の結果は残さない。
#[derive(Debug)]
pub struct SaveOutcome {
    /// 頼んだ保存先。
    pub path: PathBuf,
    /// 成功なら書いたセットの ID と退避の場所、失敗なら理由の文（画面の言語）。
    pub result: Result<SavedFacts, String>,
}

/// 保存した事実（返事の組み立てに使う）。
#[derive(Debug)]
pub struct SavedFacts {
    /// 文書を書き直したセットの ID（書き直さなかったセットは開いたときのバイト列のまま残る）。
    pub sets_written: Vec<String>,
    /// 読めないため保存に入れなかったセットの名前（保存したことが無いセット。無ければ空）。
    pub left_out: Vec<String>,
    /// 置き換えで残した前の版（上書きでなければ無い）。
    pub backup: Option<PathBuf>,
}

/// 試験用: 止めておいた保存の仕事の手。`release` で動き出す。
#[doc(hidden)]
#[derive(Clone)]
pub struct SaveHold(Arc<AtomicBool>);
impl SaveHold {
    pub fn release(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// 保存の進み具合（仕事の札・閉じるのを待つウィンドウ）。
#[derive(Clone, Debug, PartialEq)]
pub struct SaveProgress {
    pub file: String,
    /// 0..1。
    pub fraction: f32,
}

/// 裏のスレッドと画面が見る進み。
#[derive(Default)]
struct Shared {
    stage: AtomicUsize,
    build: BuildProgress,
}

struct Job {
    path: PathBuf,
    file: String,
    rx: Receiver<Finished>,
    shared: Arc<Shared>,
    /// 開いているファイルと同じ場所への保存か（そうなら、保存後の印を開いているファイルの印にする）。
    reuse: bool,
    /// 外からの操作が頼んだ保存か（終わったら `SaveState::outcome` に結果を残す）。
    report: bool,
    /// 頼む前の「変更あり」・棚の「変更あり」（失敗したら戻す。保存の間の編集は、そのまま残る）。
    was_modified: bool,
    shelf_was_changed: bool,
    /// 写しを取った時点の、書き直したセットの（ID、文書の ID、版）。成功したら保存済みの印にする。
    written: Vec<(String, u128, u64)>,
    /// 書いたメッシュマップ（セットの ID ごと）。成功したら保存済みにする。
    maps: Vec<(String, Vec<Arc<BakedMeshMap>>)>,
    /// 読めないため保存に入れなかったセットの名前。
    left_out: Vec<String>,
}

/// 裏の仕事の結果。
struct Finished {
    result: Result<Done, String>,
    /// 保存で更新された保存先の印（置換の後に失敗しても、新しい版は確定していて印も新しい）。
    target: Option<SaveTarget>,
}
struct Done {
    /// 書いたファイルを指すプロジェクト（次の保存・書き置きの元）。
    project: Arc<Project>,
    text: String,
    /// 保存できたが気をつけること（読めなかった物の上書き・保存しなかったポーズの項目・効いていない効果・開き直しの予算・
    /// 消せなかった古い退避）を文に添えたか。
    caveat: bool,
    /// 置き換えで残した前の版。
    backup: Option<PathBuf>,
}

/// 裏のスレッドへ渡す頼み。
struct Request {
    capture: Capture,
    path: PathBuf,
    reuse: Option<SaveTarget>,
    keep: BackupKeep,
    recovery: Option<crate::recovery::Waiter>,
    reopen_limits: yolu_io::Limits,
    thresholds: yolu_io::Thresholds,
    hold: Option<Arc<AtomicBool>>,
    shared: Arc<Shared>,
}

impl SaveState {
    /// 進み具合（保存していなければ None）。
    pub fn progress(&self) -> Option<SaveProgress> {
        let job = self.job.as_ref()?;
        let stage = job.shared.stage.load(Ordering::Relaxed).min(STAGES - 1);
        let done: f32 = WEIGHTS[..stage].iter().sum();
        // 合成の段だけ、セットの数で細かくする
        let inside = if stage == 0 {
            let total = job.shared.build.sets_total.load(Ordering::Relaxed);
            let finished = job.shared.build.sets_done.load(Ordering::Relaxed);
            if total > 0 {
                finished as f32 / total as f32
            } else {
                0.0
            }
        } else {
            0.0
        };
        Some(SaveProgress {
            file: job.file.clone(),
            fraction: (done + WEIGHTS[stage] * inside).clamp(0.0, 1.0),
        })
    }

    /// 外からの操作が頼んだ保存の結果を受け取る（終わっていなければ None。受け取ると空になる）。
    pub fn take_outcome(&mut self) -> Option<SaveOutcome> {
        self.outcome.take()
    }

    /// 試験用: 次の保存の仕事を、手が離されるまで始めずに止めておく。
    #[doc(hidden)]
    pub fn hold_next(&mut self) -> SaveHold {
        let flag = Arc::new(AtomicBool::new(false));
        self.hold = Some(flag.clone());
        SaveHold(flag)
    }
}

/// 保存（札。取り消せない）。終わる頼みを保存が終わるまで待たせている間は、キーの割り当てを止める。保存は閉じる前の確かめにも、
/// 止める仕事にも入れない（閉じる流れが終わるまで待つ。`YoluApp::close_flow`）。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(|app, lang| {
        let p = app.save_progress()?;
        Some(JobCard {
            text: format!("{} — {}", lang.pick("保存しています", "Saving"), p.file),
            fraction: Some(p.fraction),
            // 保存は途中で止めない（検証した一時ファイルからの 1 回の置換で確定させる）
            cancel: None,
            canceling: false,
        })
    }),
    modal: Some(crate::windows::waiting_to_close),
    ..JobSpec::new("save", AppState::is_saving)
};

impl AppState {
    /// 保存の仕事が動いているか（裏のスレッド。終わりは `poll_save` が受ける）。
    pub fn is_saving(&self) -> bool {
        self.save.job.is_some()
    }

    /// 「変更あり」の印。保存の間は、保存の結果が出るまで、頼む前の印のまま見せる（保存の頼みは `modified` を下ろして、その後の編集を
    /// 見分けるので、見せる側はこちらを読む）。
    pub fn shows_modified(&self) -> bool {
        self.modified || self.save.job.as_ref().is_some_and(|j| j.was_modified)
    }

    /// 保存の進み具合（仕事の札・閉じるのを待つウィンドウ）。
    pub fn save_progress(&self) -> Option<SaveProgress> {
        self.save.progress()
    }

    /// 終わった保存の仕事を受ける（フレームの初めに）。
    pub fn poll_save(&mut self) {
        let Some(job) = &self.save.job else {
            return;
        };
        let finished = match job.rx.try_recv() {
            Ok(f) => f,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => stopped(self.lang),
        };
        let job = self.save.job.take().expect("上で見た");
        // 操作の外で結果を書くので、操作と同じく新しい知らせとして出す
        let prior = self.message_begin();
        finish(self, job, finished);
        self.message_end(prior);
    }

    /// 保存の仕事が終わるまで待って受ける（試験・裏のスレッドを使わない後始末。待ちの上限は 120 秒）。画面のスレッドを止めるので、
    /// 描画の途中では使わない。
    #[doc(hidden)]
    pub fn wait_save(&mut self) {
        crate::jobs::wait_until_idle(
            self,
            "保存の処理が終わらない（ハング検出上限）",
            |s| s.save.job.is_some(),
            Self::poll_save,
        );
    }
}

/// 結果を返さずに裏のスレッドが止まった。
fn stopped(lang: Lang) -> Finished {
    Finished {
        result: Err(lang.pick("処理が止まりました", "The save stopped").into()),
        target: None,
    }
}

/// 保存の頼み（ファイルのメニュー・Ctrl+S・別名で保存・保存して更新）。断る・失敗するときは、何も変えずに理由を出す。
pub fn save_from(state: &mut AppState, path: &Path) {
    if let Err(e) = start(state, path, false) {
        let lang = state.lang;
        state.refuse(Source::Save, lang.with_reason(cannot_save(lang, path), e));
    }
}

/// 外からの操作の保存の頼み: `save_from` と同じ道（裏のスレッド・書き直しの印・退避）で、結果は `SaveState::take_outcome` が渡す（メッセージだけに
/// しない）。始められなければ理由の文（何も変えない）。試験の状態（`background` が false）は、頼みの中で終えるので、戻ってすぐ結果が取れる。
pub fn save_for_ops(state: &mut AppState, path: &Path) -> Result<(), String> {
    state.save.outcome = None;
    start(state, path, true)
}

/// 頼みを組み立てて、仕事を始める。始められなければ理由。`report` は外からの操作の頼み（結果を `SaveState::outcome` に残す）。
fn start(state: &mut AppState, path: &Path, report: bool) -> Result<(), String> {
    let lang = state.lang;
    if state.is_stroking() {
        return Err(crate::lang::refusals::during_stroke(lang).into());
    }
    if state.save.job.is_some() {
        return Err(crate::lang::refusals::saving(lang).into());
    }
    // 配布用に保存の写し（準備した写し・書いている途中）は、保存前のプロジェクトが開いている .ylp のハンドルから読む。普通は、保存で
    // そのファイルを置き換えても、ハンドルは置き換える前のファイルを読み続ける。しかし、置換の規則が POSIX でないファイルシステム
    // （FAT・exFAT・一部のネットワーク）は、開いているファイルを置き換えられないので、保存が置換のためにそのハンドルを手放し、写しの読みは
    // 断られる。その間は保存しない
    if state.distribute.is_busy() || state.distribute.is_open() {
        return Err(lang
            .pick(
                "配布用に保存の途中は保存しません",
                "Cannot save while saving for distribution",
            )
            .into());
    }
    let capture = capture(state, path.to_path_buf())?;
    // 開いている .ylp と同じファイルへの保存なら、その保存先の印を引き継ぐ（複製を渡し、結果の印で置き換える）
    let reuse = state
        .project
        .as_ref()
        .filter(|p| p.is_file() && same_file(&p.path, path))
        .and_then(|p| p.target.clone());
    let same_file_as_open = reuse.is_some();
    let shared = Arc::new(Shared::default());
    let written: Vec<(String, u128, u64)> = capture
        .sets
        .iter()
        .filter(|s| s.rewrite)
        .map(|s| (s.id.clone(), s.mark.0, s.mark.1))
        .collect();
    let maps: Vec<(String, Vec<Arc<BakedMeshMap>>)> = capture
        .maps
        .iter()
        .map(|(id, _, maps)| (id.clone(), maps.clone()))
        .collect();
    let left_out = capture.left_out.clone();
    let request = Request {
        capture,
        path: path.to_path_buf(),
        reuse,
        keep: state.prefs.settings.backups,
        // 書き置きの書き込みも、開いた .ylp のハンドルから読む（上と同じ事情）。そのファイルを置き換える保存は、今の書き込みと待っている
        // 頼みが終わるのを待ってから置き換える。新しい頼みは、保存の間は出さない
        recovery: if same_file_as_open {
            state.recovery_waiter()
        } else {
            None
        },
        reopen_limits: yolu_io::Limits::from_layer_pixels(state.load_source_bytes()),
        thresholds: yolu_io::Thresholds::current(),
        hold: state.save.hold.take(),
        shared: shared.clone(),
    };
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let was_modified = state.modified;
    let shelf_was_changed = state.shelf.changed;
    let job = |rx| Job {
        path: path.to_path_buf(),
        file: file.clone(),
        rx,
        shared: shared.clone(),
        reuse: same_file_as_open,
        report,
        was_modified,
        shelf_was_changed,
        written,
        maps,
        left_out,
    };
    if state.save.background {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("yolu-save".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                let _ = tx.send(run(request));
            })
            .map_err(|e| e.to_string())?;
        // 保存の間の編集を見分けるために下ろす（失敗したら戻す）。復旧の見張りは、保存の間は印を読まない
        state.modified = false;
        state.shelf.changed = false;
        state.save.job = Some(job(rx));
    } else {
        // 試験の状態: 呼び手のスレッドで終えて、すぐ受ける（止めておく頼みは、止めても手を離す側がいないので使わない）
        let (tx, rx) = channel();
        let _ = tx.send(run(Request {
            hold: None,
            ..request
        }));
        state.modified = false;
        state.shelf.changed = false;
        let job = job(rx);
        let finished = job.rx.try_recv().unwrap_or_else(|_| stopped(lang));
        finish(state, job, finished);
    }
    Ok(())
}

/// 裏の仕事の本体（正本・合成の PNG を作る → 保存先を決める → 書く・確かめる・置き換える）。`AppState` には触らない。
fn run(request: Request) -> Finished {
    if let Some(hold) = &request.hold {
        while !hold.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    // 閾値はスレッドごとの値（試験が小さくして、小さな文書で大きな形の道を通す）。頼んだスレッドの値を、この仕事でも使う
    request.thresholds.scoped(|| work(request))
}

fn work(request: Request) -> Finished {
    let Request {
        capture,
        path,
        reuse,
        keep,
        recovery,
        reopen_limits,
        shared,
        ..
    } = request;
    let lang = capture.lang;
    let fail = |text: String| Finished {
        result: Err(text),
        target: None,
    };
    shared.stage.store(0, Ordering::Relaxed);
    let built = match build(&capture, None, &shared.build) {
        Ok(b) => b,
        Err(BuildError::Message(text)) => return fail(text),
        Err(BuildError::Canceled) => return fail(lang.pick("取り消しました", "Canceled").into()),
    };
    let project = Arc::new(built.project);
    let overwrite = path.exists();
    let (mut target, reused) = match reuse {
        Some(t) => (t, true),
        None => {
            // 別の場所: あれば .ylp として読めるものだけを上書きする（読めないファイルを黙って潰さない）。中身は確かめるだけなので、
            // 予算では断らない
            let opened = if overwrite {
                SaveTarget::open_within(&path, &yolu_io::Limits::unbounded())
                    .map(|(_, t)| t)
                    .map_err(|e| {
                        lang.with_reason(
                            lang.pick(
                                "上書きする先を .ylp として読めません",
                                "Invalid overwrite target",
                            ),
                            lang.io_error(&e),
                        )
                    })
            } else {
                SaveTarget::create(&path).map_err(|e| lang.io_error(&e))
            };
            match opened {
                Ok(t) => (t, false),
                Err(text) => return fail(text),
            }
        }
    };
    if let Some(waiter) = &recovery {
        waiter.wait();
    }
    let mut report = match target.save_with_progress(&project, keep, &mut |stage| {
        shared.stage.store(1 + stage.index(), Ordering::Relaxed);
    }) {
        Ok(r) => r,
        // 置換の後に失敗しても、新しい版は確定していて印も新しい（開いているファイルの保存なら、その印を持ち帰る）
        Err(e) => {
            return Finished {
                result: Err(lang.io_error(&e)),
                target: reused.then_some(target),
            };
        }
    };
    // 次の保存・書き置きの元は、書いたファイルを指すプロジェクト（変わらないエントリはそのファイルから写す。保存に使った core の文書の
    // 写しは手放す）
    let saved = report.project.take().map(Arc::new).unwrap_or(project);
    // 書いた .ylp を、今の「レイヤーのメモリ」の予算で開き直せるか（読み手の上限は予算から決まり、一様なタイルの多い文書は core の画素が
    // 小さいまま正本だけが大きくなる。保存は止めず、開き直すのに予算が要ることをここで言う）
    let reopen = reopen_note(lang, &saved, &reopen_limits);
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut text = lang.pick(format!("保存しました: {file}。"), format!("Saved: {file}."));
    if !capture.left_out.is_empty() {
        text += " ";
        text += &left_out_note(lang, &capture.left_out);
    }
    let map_total: usize = capture.maps.iter().map(|(_, _, m)| m.len()).sum();
    if map_total > 0 {
        text += &lang.pick(
            format!(" メッシュマップ {map_total} 枚を書きました。"),
            format!(" Wrote {map_total} mesh map(s)."),
        );
    }
    // 開くときに読めなかった見た目の設定（新しい形式など）を、変えた見た目で上書きしたセット
    if !built.looks_overwritten.is_empty() {
        let names: Vec<&str> = built
            .looks_overwritten
            .iter()
            .filter_map(|id| {
                capture
                    .sets
                    .iter()
                    .find(|s| s.id == *id)
                    .map(|s| s.name.as_str())
            })
            .collect();
        text += " ";
        text += &lang.with_reason(
            lang.pick(
                "読めなかった見た目の設定を上書きしました",
                "Overwrote unreadable look settings",
            ),
            names.join(lang.pick("、", ", ")),
        );
    }
    // 開くときに読めなかった、名前を付けて残した選択範囲の項目を、変えた並びで置き換えたセット
    if !built.saved_overwritten.is_empty() {
        let names: Vec<&str> = built
            .saved_overwritten
            .iter()
            .filter_map(|id| {
                capture
                    .sets
                    .iter()
                    .find(|s| s.id == *id)
                    .map(|s| s.name.as_str())
            })
            .collect();
        text += " ";
        text += &lang.with_reason(
            lang.pick(
                "読めなかった覚えた選択範囲の項目を置き換えました",
                "Replaced unreadable remembered selections",
            ),
            names.join(lang.pick("、", ", ")),
        );
    }
    if built.pose_overwritten {
        text += &lang.pick(
            " 読めなかったポーズを、今のポーズで置き換えました。".to_owned(),
            " Replaced the unreadable pose with the current pose.".to_owned(),
        );
    }
    let unsaved = crate::view3d::pose::stored::unsaved_note(lang, &capture.pose_unsaved);
    let caveat = !capture.left_out.is_empty()
        || !built.looks_overwritten.is_empty()
        || !built.saved_overwritten.is_empty()
        || built.pose_overwritten
        || !unsaved.is_empty()
        || !built.inactive_effects.is_empty()
        || reopen.is_some()
        || !report.prune_failures.is_empty();
    text += &unsaved;
    text += &built.inactive_effects;
    if let Some(note) = reopen {
        text += &note;
    }
    text += &backup_text(lang, &path, &report);
    Finished {
        result: Ok(Done {
            project: saved,
            text,
            caveat,
            backup: report.backup.clone(),
        }),
        target: Some(target),
    }
}

/// 結果を受ける（画面のスレッド）。成功なら開いているファイルとセットの保存済みの印を更新し、失敗なら何も変えずに理由を出す。
fn finish(state: &mut AppState, job: Job, finished: Finished) {
    let lang = state.lang;
    match finished.result {
        Ok(done) => {
            let Some(target) = finished.target else {
                return fail(
                    state,
                    &job,
                    lang.pick("処理が止まりました", "The save stopped").into(),
                    None,
                );
            };
            match state.project.as_mut().filter(|_| job.reuse) {
                Some(file) => {
                    file.target = Some(target);
                    file.original = done.project;
                    file.path = job.path.clone();
                }
                None => {
                    state.project = Some(ProjectFile {
                        path: job.path.clone(),
                        target: Some(target),
                        original: done.project,
                    });
                }
            }
            // 保存済みの印は、写しを取った時点の文書（保存の間に描いた分は「変更あり」のまま）
            for (id, doc_id, revision) in &job.written {
                if let Some(set) = state
                    .sets
                    .iter()
                    .position(|s| s.id == *id)
                    .and_then(|i| state.sets.get_mut(i))
                {
                    set.saved = Some((*doc_id, *revision));
                }
            }
            for (id, maps) in &job.maps {
                if let Some(set) = state
                    .sets
                    .iter()
                    .position(|s| s.id == *id)
                    .and_then(|i| state.sets.get_mut(i))
                {
                    set.mesh_maps.mark_saved(maps);
                }
            }
            state.save_folder = None;
            state.project_name = job
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| lang.pick("名称未設定", "Untitled").into());
            state.rewritten_sets = job.written.len();
            // 入れなかったセットは、画面にあってファイルに無い。「保存していない変更」のままにして、閉じる・開き直す・捨てるときに聞く
            // （次の保存でも同じ。知らせは保存の時点の 1 回だけで、あとの知らせに上書きされうるので、注意としてログにも残す）
            if !job.left_out.is_empty() {
                state.modified = true;
            }
            if done.caveat {
                state.warn(Source::Save, done.text);
            } else {
                state.info(Source::Save, done.text);
            }
            if job.report {
                state.save.outcome = Some(SaveOutcome {
                    path: job.path.clone(),
                    result: Ok(SavedFacts {
                        sets_written: job.written.iter().map(|(id, _, _)| id.clone()).collect(),
                        left_out: job.left_out.clone(),
                        backup: done.backup,
                    }),
                });
            }
        }
        Err(text) => fail(state, &job, text, finished.target),
    }
}

/// 保存が済まなかった: 開いているファイルの保存先の印は更新があれば持ち帰り、保存の前の「変更あり」を戻して、理由を出す。
fn fail(state: &mut AppState, job: &Job, text: String, target: Option<SaveTarget>) {
    if let (Some(file), Some(target), true) = (state.project.as_mut(), target, job.reuse) {
        file.target = Some(target);
    }
    state.modified |= job.was_modified;
    state.shelf.changed |= job.shelf_was_changed;
    if job.report {
        state.save.outcome = Some(SaveOutcome {
            path: job.path.clone(),
            result: Err(text.clone()),
        });
    }
    let lang = state.lang;
    state.fail(
        Source::Save,
        lang.with_reason(cannot_save(lang, &job.path), text),
    );
}

/// 「「ファイル」に保存できません」（理由は `Lang::with_reason` で添える）。
fn cannot_save(lang: Lang, path: &Path) -> String {
    let file = lang.quote(&path.display().to_string());
    lang.pick(
        format!("{file}に保存できません"),
        format!("Cannot save to {file}"),
    )
}
