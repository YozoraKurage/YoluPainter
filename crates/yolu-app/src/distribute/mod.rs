//! 配布用に保存（ファイルのメニュー）: 今のプロジェクトの写しを、作った人が気づかないまま残る物（取り込んだ PSD の原本・使っていない棚の素材・
//! 出どころのパス・モデルの参照・メッシュマップ・Unity の値・古い状態・知らないエントリ）を除いて別のファイルに書く。除く物は書く前の窓
//! （`window`）に種類ごとに並べ、種類ごとに外せる（既定は全部除く）。何を除くかの判断と写しの組み立ては `yolu_io::Project::for_distribution`
//! （ここは仕事の運びと窓の状態だけ）。
//!
//! - **開いているものは変えない**: 文書・プロジェクト・未保存の印・Undo・開いているファイルのどれも変えない。始めるときに、描いていない区切りで
//!   変わったセットの文書の写し（`Document::capture_snapshot`。タイルは共有）と小さな値だけを取り、写しの組み立て（正本・合成の PNG・選択範囲・
//!   見た目・メッシュマップ・棚。保存と同じ並び）と書き込みは別のスレッドで行う。描いている最中は始めない・書かない。
//! - **流れ**: 準備（別のスレッドで、今の状態の完全な写しの `Project` を組む）→ 窓（除く種類の切り替え。当たる物が無ければ窓を出さない）→
//!   保存先を選ぶ → 既にあるファイルなら置き換えを確かめる → 書き込み（別のスレッドで、種類を除いた写しを組み、検証した一時ファイルから 1 回の
//!   置き換えで書く。`SaveTarget`）。取消・失敗は何も書かない（保存先は元のまま）。
//! - **開いている作業用の .ylp へは書かない**: 同じファイルは選べない。配布用の写しの隣に `-backups~` は作らない（退避は作業用の .ylp の
//!   仕事。置き換えるときは確かめてから、前の写しは残さない）。
//! - 写しは今と同じ形式（形式 7・正本の版も同じ）。スタンドアロン版で開ける。Unity 版 0.2.0 が開けるのは正本の版 21 のセットだけを含む写しで、
//!   版 22 以上のセットを含む写しは開けない（`yolu_io::distribution` の文書）。Unity 版の読み手のコードを読んで確かめたもので、Unity では開いていない。

pub mod window;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Vec2;
use yolu_core::mesh_maps::BakedMeshMap;
use yolu_core::Document;
use yolu_io::{
    composite_pngs, shelf::Shelf, BackupKeep, DocumentSource, Inventory, Project, Removal,
    SaveTarget, SetSpec,
};

use crate::lang::Lang;
use crate::newproject::relative_model_path;
use crate::sets::MaterialRef;
use crate::state::{Action, AppState, DialogRequest};

/// 準備・書き込みのスレッドのスタック（正本への詰め直しと合成の再帰に足りる大きさ）。
const THREAD_STACK: usize = 8 * 1024 * 1024;

/// 配布用に保存の操作（`Action::Distribute`）。
#[derive(Clone, Debug, PartialEq)]
pub enum DistributeAction {
    /// ファイルのメニュー「配布用に保存…」: 準備を始める。
    Start,
    /// 窓で、種類ごとに除く・残すを切り替える。
    Toggle(Removal),
    /// 窓の「やめる」。
    CancelWindow,
    /// 窓の「保存…」: 保存先を選ぶ窓を頼む。
    ChooseFile,
    /// 保存先が決まった（既にあるファイルなら置き換えを確かめる）。
    Save(PathBuf),
    /// 置き換える確かめの「置き換える」。
    ConfirmReplace,
    /// 置き換える確かめの「やめる」。
    CancelReplace,
    /// 仕事の取消。
    CancelJob,
}

/// 準備の結果: 今の状態の完全な写し（除く前）と、モデルのファイル。
pub struct Prepared {
    project: Arc<Project>,
    model: Option<PathBuf>,
}

/// 書く前の窓の状態。
pub struct Window {
    prepared: Arc<Prepared>,
    selected: Vec<Removal>,
    inventory: Inventory,
    /// 窓を出しているか（書いている間・除く物が無いとき・保存先を選んでいる間の、窓なしの流れでは出さない）。
    visible: bool,
}

impl Window {
    fn new(prepared: Prepared) -> Self {
        let selected = Removal::ALL.to_vec();
        let inventory = prepared.project.distribution_inventory(&selected);
        Self {
            prepared: Arc::new(prepared),
            selected,
            inventory,
            visible: false,
        }
    }
    /// 除く種類の選び（既定は全部）。
    pub fn selected(&self) -> &[Removal] {
        &self.selected
    }
    /// 今の選びでの目録（当たる物が無い種類は含まない）。
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }
    /// 準備した写し（除く前）。
    pub fn project(&self) -> &Project {
        &self.prepared.project
    }
}

enum Output {
    Prepared(Box<Prepared>),
    Written(PathBuf),
}

/// 別のスレッドの仕事が失敗した理由（文は画面の言語で作る）。
#[derive(Debug)]
enum Failure {
    Canceled,
    /// 仕事のスレッドが結果を返さずに止まった。
    Stopped,
    Message(String),
}

enum Kind {
    Prepare,
    Write,
}

struct Job {
    kind: Kind,
    file: String,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Output, Failure>>,
}

/// 進み具合（仕事の札）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub file: String,
    pub writing: bool,
    pub canceling: bool,
}

/// 配布用に保存の状態。
#[derive(Default)]
pub struct DistributeState {
    /// 準備が済んだ写しと窓の選び（書き込みが済むか、やめるまで持つ）。
    window: Option<Window>,
    pub window_offset: Vec2,
    pub window_scroll: f32,
    /// 置き換えを確かめている保存先。
    pub replace: Option<PathBuf>,
    pub replace_offset: Vec2,
    job: Option<Job>,
    /// 試験用: 次の準備を、取消が来るまで始めずに止めておく。
    #[doc(hidden)]
    pub park_next: bool,
    /// 試験用: 次の書き込みを、取消が来るまで始めずに止めておく。
    #[doc(hidden)]
    pub park_write: bool,
}

impl DistributeState {
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }
    /// 準備した写しがあるか（窓を出している・保存先を選んでいる・置き換えを確かめている間）。
    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }
    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref()
    }
    /// 窓を描くか。
    pub fn window_visible(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.visible)
            && self.job.is_none()
            && self.replace.is_none()
    }
    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        Some(Progress {
            file: job.file.clone(),
            writing: matches!(job.kind, Kind::Write),
            canceling: job.cancel.load(Ordering::Relaxed),
        })
    }
}

/// 保存先を選ぶ窓に出す初めのファイル名（今の名前に短い接尾辞を付けて、開いている作業用のファイルと区別する）。
pub fn default_name(state: &AppState) -> String {
    format!("{}-dist.ylp", state.project_name)
}

/// 保存先を選ぶ窓（`DialogRequest::DistributeSave`）を出す。選ばなければ何もしない（窓は開いたまま）。
pub fn run_dialog(state: &mut AppState) {
    let lang = state.lang;
    let mut dialog = rfd::FileDialog::new()
        .set_title(lang.pick("配布用に保存", "Save for Distribution"))
        .add_filter(
            lang.pick("YoluPainter プロジェクト", "YoluPainter Project"),
            &["ylp"],
        )
        .set_file_name(default_name(state));
    if let Some(dir) = state
        .project
        .as_ref()
        .filter(|p| p.is_file())
        .and_then(|p| p.path().parent())
        .filter(|d| d.is_dir())
    {
        dialog = dialog.set_directory(dir);
    }
    match dialog.save_file() {
        Some(path) => state.apply(Action::Distribute(DistributeAction::Save(path))),
        // 窓なしの流れ（除く物が無かった）で選ばなかったら、準備した写しを閉じる（窓があるときは窓に戻る）
        None => state.settle_distribute_window(),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 名前が .ylp で終わっていなければ .ylp を足す（窓の種類で付かない環境がある）。
fn with_extension(path: PathBuf) -> PathBuf {
    let ends = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ylp"));
    if ends {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".ylp");
        PathBuf::from(name)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl AppState {
    pub fn distribute_apply(&mut self, action: DistributeAction) {
        let lang = self.lang;
        let stroking = self.is_stroking();
        let refuse = |s: &mut AppState| {
            s.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into()
        };
        match action {
            DistributeAction::Start => {
                if stroking {
                    return refuse(self);
                }
                self.start_distribute_prepare();
            }
            DistributeAction::Toggle(removal) => {
                let Some(window) = self.distribute.window.as_mut() else {
                    return;
                };
                match window.selected.iter().position(|r| *r == removal) {
                    Some(at) => {
                        window.selected.remove(at);
                    }
                    None => {
                        window.selected.push(removal);
                        window.selected.sort();
                    }
                }
                // 使っているかは、除いた後に残る見た目の設定で決まる（目録と写しが食い違わない）
                window.inventory = window
                    .prepared
                    .project
                    .distribution_inventory(&window.selected);
            }
            DistributeAction::CancelWindow => {
                if self.distribute.job.is_none() {
                    self.distribute.window = None;
                    self.distribute.replace = None;
                }
            }
            DistributeAction::ChooseFile => {
                if stroking {
                    return refuse(self);
                }
                if self.distribute.window.is_some() && self.distribute.job.is_none() {
                    self.dialog_request = Some(DialogRequest::DistributeSave);
                }
            }
            DistributeAction::Save(path) => {
                if stroking {
                    refuse(self);
                    // 窓なしの流れ（除く物が無かった）で、断ったまま準備した写しを抱えて動かなくならないように
                    return self.settle_distribute_window();
                }
                self.distribute_save(with_extension(path));
            }
            DistributeAction::ConfirmReplace => {
                if let Some(path) = self.distribute.replace.take() {
                    if stroking {
                        refuse(self);
                        return self.settle_distribute_window();
                    }
                    self.start_distribute_write(path, true);
                }
            }
            DistributeAction::CancelReplace => {
                if self.distribute.replace.take().is_some() {
                    self.message = lang
                        .pick(
                            "配布用の保存をやめました。",
                            "Save for distribution canceled.",
                        )
                        .into();
                    self.settle_distribute_window();
                }
            }
            DistributeAction::CancelJob => {
                if let Some(job) = &self.distribute.job {
                    job.cancel.store(true, Ordering::Relaxed);
                    self.message = lang
                        .pick(
                            "配布用の保存を取り消しています…",
                            "Canceling the save for distribution…",
                        )
                        .into();
                }
            }
        }
    }

    /// 書き込みが済まなかったとき（取消・失敗・置き換えをやめた）の窓: 除く物の窓があれば出し直し、窓なしの流れ（除く物が無かった）なら閉じる。
    fn settle_distribute_window(&mut self) {
        if self.distribute.job.is_some() || self.distribute.replace.is_some() {
            return;
        }
        if self
            .distribute
            .window
            .as_ref()
            .is_some_and(|w| w.inventory.is_empty())
        {
            self.distribute.window = None;
        } else if let Some(window) = self.distribute.window.as_mut() {
            window.visible = true;
        }
    }

    /// 準備を始める: 材料を主のスレッドで取り（文書は変えない）、写しの組み立ては別のスレッドで。
    fn start_distribute_prepare(&mut self) {
        let lang = self.lang;
        if self.distribute.job.is_some() || self.distribute.window.is_some() {
            return;
        }
        if self.psd.is_busy() || self.psd.import_check.is_some() {
            self.message = lang
                .pick("PSD を処理中です。", "A PSD job is running.")
                .into();
            return;
        }
        let capture = match capture(self) {
            Ok(c) => c,
            Err(text) => {
                self.message = format!(
                    "{}: {text}",
                    lang.pick("配布用に保存できません", "Cannot save for distribution")
                );
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let flag = cancel.clone();
        let park = std::mem::take(&mut self.distribute.park_next);
        let spawned = std::thread::Builder::new()
            .name("yolu-distribute-prepare".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(build(&capture, &flag).map(|p| Output::Prepared(Box::new(p))));
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            return;
        }
        self.message = lang
            .pick(
                "配布用の写しを準備中…",
                "Preparing the copy for distribution…",
            )
            .into();
        self.distribute.job = Some(Job {
            kind: Kind::Prepare,
            file: self.project_name.clone(),
            cancel,
            rx,
        });
    }

    /// 保存先が決まった: 開いているファイルなら断り、既にあるファイルなら置き換えを確かめ、無ければ書き始める。
    fn distribute_save(&mut self, path: PathBuf) {
        let lang = self.lang;
        if self.distribute.window.is_none() || self.distribute.job.is_some() {
            return;
        }
        if self
            .project
            .as_ref()
            .is_some_and(|p| p.is_file() && same_file(p.path(), &path))
        {
            self.message = lang
                .pick(
                    "開いているファイルには書けません",
                    "Cannot write over the open file",
                )
                .into();
            // 窓なしの流れなら閉じ、窓があれば除く物の窓に戻る（選び直せる）
            self.settle_distribute_window();
            return;
        }
        if std::fs::symlink_metadata(&path).is_ok() {
            self.distribute.replace = Some(path);
            return;
        }
        self.start_distribute_write(path, false);
    }

    /// 書き込みを始める: 窓の選びのまま、写しの組み立てと書き込みを別のスレッドで。
    fn start_distribute_write(&mut self, path: PathBuf, replacing: bool) {
        let lang = self.lang;
        let Some(window) = self.distribute.window.as_mut() else {
            return;
        };
        window.visible = false;
        let prepared = window.prepared.clone();
        let remove = window.selected.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let flag = cancel.clone();
        let park = std::mem::take(&mut self.distribute.park_write);
        let target = path.clone();
        let spawned = std::thread::Builder::new()
            .name("yolu-distribute-write".into())
            .stack_size(THREAD_STACK)
            .spawn(move || {
                if park {
                    crate::windows::park_until_canceled(&flag);
                }
                let _ = tx.send(
                    write(&prepared, &remove, &target, replacing, lang, &flag).map(Output::Written),
                );
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            // 出さないと決めた窓を戻す（窓なしの流れなら閉じる）。仕事が無いまま、窓も出ない状態にしない
            self.settle_distribute_window();
            return;
        }
        let file = file_name(&path);
        self.message = lang.pick(
            format!("配布用に保存中: {file}"),
            format!("Saving for distribution: {file}"),
        );
        self.distribute.job = Some(Job {
            kind: Kind::Write,
            file,
            cancel,
            rx,
        });
    }

    /// 終わった仕事を受ける（フレームの初めに）。
    pub fn poll_distribute(&mut self) {
        let lang = self.lang;
        let Some(job) = &self.distribute.job else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(Failure::Stopped),
        };
        let job = self.distribute.job.take().expect("上で見た");
        match (result, job.kind) {
            (Ok(Output::Prepared(prepared)), Kind::Prepare) => {
                let mut window = Window::new(*prepared);
                // 除く物が無ければ窓を出さず、すぐ保存先を選ぶ
                if window.inventory.is_empty() {
                    self.dialog_request = Some(DialogRequest::DistributeSave);
                } else {
                    window.visible = true;
                    self.distribute.window_scroll = 0.0;
                }
                self.distribute.window = Some(window);
                self.message.clear();
            }
            (Ok(Output::Written(path)), Kind::Write) => {
                self.distribute.window = None;
                self.message = lang.pick(
                    format!("配布用に保存しました: {}", path.display()),
                    format!("Saved for distribution: {}", path.display()),
                );
            }
            (Err(failure), kind) => {
                let writing = matches!(kind, Kind::Write);
                // 書けなかった（取消も）ときは、窓を戻して選び直せるようにする
                if writing {
                    self.settle_distribute_window();
                }
                self.message = match failure {
                    Failure::Canceled => lang
                        .pick(
                            "配布用の保存を取り消しました。",
                            "Save for distribution canceled.",
                        )
                        .into(),
                    Failure::Stopped => lang
                        .pick(
                            "配布用に保存できません: 処理が止まりました",
                            "Cannot save for distribution: the job stopped",
                        )
                        .into(),
                    Failure::Message(text) => format!(
                        "{}: {text}",
                        lang.pick("配布用に保存できません", "Cannot save for distribution")
                    ),
                };
            }
            _ => {}
        }
    }

    /// 試験用: 配布用に保存の仕事が終わるまで待って受ける（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_distribute(&mut self) {
        let start = Instant::now();
        while self.distribute.job.is_some() {
            self.poll_distribute();
            if self.distribute.job.is_none() {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 120,
                "配布用に保存の処理が終わらない（ハング検出上限）"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

// ───────── 材料と組み立て ─────────

/// 1 セット分の材料。
struct SetCapture {
    id: String,
    name: String,
    material: MaterialRef,
    /// 文書の写し（読むだけのセットは無い。選択範囲・見た目・Unity の値はここから）。
    snapshot: Option<Arc<Document>>,
    /// 正本・合成の PNG を作り直すか（開いた時のファイルに無い、または開いた・保存した時から変わった）。
    rewrite: bool,
}

/// 準備の材料（別のスレッドへ渡す。保存 `project::save` と同じ並びで、今の状態の完全な写しを組むもの）。
struct Capture {
    sets: Vec<SetCapture>,
    current: String,
    base: Option<Arc<Project>>,
    /// 開いたあとに文書を別の物に替えたセット（古い PSD の原本を持ち越さない）。
    replaced: Vec<String>,
    /// アセットの棚（変えていて読めるときだけ。変えていなければ開いたファイルのバイト列のまま）。
    shelf: Option<Shelf>,
    /// まだ .ylp に書いていないメッシュマップ（セットの ID ごと）。
    maps: Vec<(String, Vec<Arc<BakedMeshMap>>)>,
    model: Option<PathBuf>,
    /// 開いているファイルの場所（モデルの相対のパスの基準。無ければ空）。
    open_path: PathBuf,
    /// Unity から受けた値を写しに書くか（設定。切っていれば外す）。
    keep_received: bool,
    lang: Lang,
}

/// 材料を取る（文書は変えない。写しはタイルを共有する）。取れなければ理由。
fn capture(state: &AppState) -> Result<Capture, String> {
    let lang = state.lang;
    if state.is_stroking() {
        return Err(lang
            .pick("描いている間は保存しません", "Cannot save during a stroke")
            .into());
    }
    let base = state.project.as_ref().map(|p| p.project_shared());
    let mut sets = Vec::with_capacity(state.sets.len());
    let mut maps = Vec::new();
    for (i, set) in state.sets.iter().enumerate() {
        let doc = state.set_doc(i);
        let in_base = base
            .as_ref()
            .is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        let read_only = set.read_only.is_some();
        if read_only && !in_base {
            return Err(lang.pick(
                format!("読むだけのセット「{}」の元の文書がありません", set.name),
                format!("Original document missing for read-only set “{}”", set.name),
            ));
        }
        let unchanged = set.saved == Some((doc.id(), doc.revision()));
        let rewrite = !(in_base && (read_only || unchanged));
        let snapshot = if read_only {
            None
        } else {
            Some(Arc::new(doc.capture_snapshot().map_err(|e| lang.core_error(&e))?))
        };
        let unsaved = set.mesh_maps.unsaved();
        if !unsaved.is_empty() {
            maps.push((set.id.clone(), unsaved));
        }
        sets.push(SetCapture {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            snapshot,
            rewrite,
        });
    }
    let replaced = crate::project::replaced_sets(
        base.as_deref(),
        state
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| set.read_only.is_none())
            .map(|(i, set)| (set.id.as_str(), state.set_doc(i).id())),
    );
    let open_path = state
        .project
        .as_ref()
        .filter(|p| p.is_file())
        .map(|p| p.path().to_path_buf())
        .unwrap_or_default();
    Ok(Capture {
        sets,
        current: state.sets.current().id.clone(),
        base,
        replaced,
        shelf: (state.shelf.changed && state.shelf.unavailable.is_none())
            .then(|| state.shelf.shelf().clone()),
        maps,
        model: state.np.model_file.clone(),
        open_path,
        keep_received: state.prefs.settings.livelink_keep_values,
        lang,
    })
}

fn io_failure(lang: Lang, e: &yolu_io::Error) -> Failure {
    Failure::Message(lang.io_error(e))
}

/// 材料から、今の状態の完全な写しの `Project`（保存が書くのと同じ形。除く前）を組む（別のスレッドで動かす）。
fn build(capture: &Capture, cancel: &AtomicBool) -> Result<Prepared, Failure> {
    let lang = capture.lang;
    let io = |e: yolu_io::Error| io_failure(lang, &e);
    let mut specs = Vec::with_capacity(capture.sets.len());
    for set in &capture.sets {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Canceled);
        }
        let (document, composites) = if set.rewrite {
            let doc = set
                .snapshot
                .as_ref()
                .expect("読むだけのセットは作り直さない");
            let native = DocumentSource::from_core(doc.clone()).map_err(|e| {
                Failure::Message(format!(
                    "{}: {}",
                    lang.pick(
                        format!("セット「{}」の文書を作れません", set.name),
                        format!("Cannot convert texture set “{}” to a document", set.name)
                    ),
                    lang.io_error(&e)
                ))
            })?;
            let pngs = composite_pngs(doc).map_err(|e| {
                Failure::Message(format!(
                    "{}: {}",
                    lang.pick(
                        format!("セット「{}」の合成の PNG を作れません", set.name),
                        format!(
                            "Cannot build the composite PNG of texture set “{}”",
                            set.name
                        )
                    ),
                    lang.io_error(&e)
                ))
            })?;
            (Some(native), pngs)
        } else {
            (None, Vec::new())
        };
        specs.push(SetSpec {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            document,
            composites,
        });
    }
    let writer = crate::project::writer();
    let mut project = match &capture.base {
        Some(base) => {
            let upgraded;
            let base: &Project = if base.info().format < 7 {
                upgraded = base.upgraded(writer.clone()).map_err(io)?;
                &upgraded
            } else {
                base
            };
            // 開いたあとに消したセットは、写しにも入れない
            let dropped: Vec<&str> = base
                .sets()
                .iter()
                .map(|s| s.id.as_str())
                .filter(|id| !capture.sets.iter().any(|s| s.id == *id))
                .collect();
            base.with_sets_dropping(writer.clone(), &specs, &capture.current, &dropped)
                .map_err(io)?
        }
        None => Project::create(writer.clone(), &specs, &capture.current).map_err(io)?,
    };
    for id in &capture.replaced {
        project = project.without_imported_original(id).map_err(io)?;
    }
    let text = |reason: String| Failure::Message(reason);
    let selections: Vec<(&str, Option<&yolu_core::SelectionMask>)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.selection())))
        .collect();
    project = crate::selection::io::write_into(project, &selections, lang).map_err(text)?;
    let looks: Vec<(&str, &yolu_core::look::MaterialLook)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.look())))
        .collect();
    project = crate::look::io::write_into(project, &looks, lang)
        .map_err(text)?
        .0;
    let received: Vec<(&str, Option<&yolu_core::look::ReceivedLook>)> = capture
        .sets
        .iter()
        .filter_map(|s| {
            s.snapshot.as_ref().map(|d| {
                (
                    s.id.as_str(),
                    d.received_look().filter(|_| capture.keep_received),
                )
            })
        })
        .collect();
    project = crate::look::io::write_received_into(project, &received, lang).map_err(text)?;
    for (id, maps) in &capture.maps {
        for map in maps {
            project = project.with_mesh_map(id, map).map_err(io)?;
        }
    }
    if let Some(shelf) = &capture.shelf {
        project = project.with_shelf(shelf, writer).map_err(io)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Canceled);
    }
    // モデルのファイルの参照: 保存と同じく、開いているファイルからの相対のパス（書くときに、選んだ保存先に合わせて付け直す）
    let anchor = if capture.open_path.as_os_str().is_empty() {
        PathBuf::from("untitled.ylp")
    } else {
        capture.open_path.clone()
    };
    let wanted = capture
        .model
        .as_ref()
        .map(|m| relative_model_path(m, &anchor));
    if project.view_model().ok().flatten() != wanted {
        project = project.with_view_model(wanted.as_deref()).map_err(io)?;
    }
    Ok(Prepared {
        project: Arc::new(project),
        model: capture.model.clone(),
    })
}

/// 窓の選びで種類を除いた写しを、`dest` へ書く（別のスレッドで動かす）。モデルの参照を残すなら、保存先からの相対に付け直す。
/// 書くのは検証した一時ファイルから 1 回の置き換え（退避は作らない）。置き換えるのは、確かめを終えた .ylp として読める既存のファイルだけ。
fn write(
    prepared: &Prepared,
    remove: &[Removal],
    dest: &Path,
    replacing: bool,
    lang: Lang,
    cancel: &AtomicBool,
) -> Result<PathBuf, Failure> {
    let io = |e: yolu_io::Error| io_failure(lang, &e);
    let repointed;
    let source: &Project = match (&prepared.model, remove.contains(&Removal::ModelReference)) {
        (Some(model), false) => {
            repointed = prepared
                .project
                .with_view_model(Some(&relative_model_path(model, dest)))
                .map_err(io)?;
            &repointed
        }
        _ => &prepared.project,
    };
    let copy = source
        .for_distribution(crate::project::writer(), remove)
        .map_err(io)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Canceled);
    }
    let mut target = if replacing {
        // 置き換える先は .ylp として読めることだけを確かめる（予算では断らない）
        SaveTarget::open_within(dest, &yolu_io::Limits::unbounded())
            .map_err(|e| {
                Failure::Message(format!(
                    "{}: {}",
                    lang.pick(
                        "置き換える先を .ylp として読めません",
                        "The file to replace is not a readable .ylp"
                    ),
                    lang.io_error(&e)
                ))
            })?
            .1
    } else {
        SaveTarget::create(dest).map_err(io)?
    };
    // 配布用の写しの隣に -backups~ を作らない（Count(0)）
    target.save_with(&copy, BackupKeep::Count(0)).map_err(io)?;
    Ok(dest.to_path_buf())
}
