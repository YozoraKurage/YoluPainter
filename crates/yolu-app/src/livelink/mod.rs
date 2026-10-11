//! Live Link のスタンドアロンの側（ファイルの受け渡し）。Unity のエディタの YoluPainter が、相手（シーンのオブジェクト）の FBX の道・
//! 骨の値・マテリアルの値と絵の道を頼みの JSON に書いて受け渡しのフォルダの `inbox/` に置き、ここが拾って開く。書き出したら、書いた
//! PNG を返事の JSON にして `outbox/` に置く（形とフォルダは `yolu_protocol::files`）。
//!
//! - 受け付け: 設定「Unity の Live Link を受け付ける」（既定は入）か、`--livelink` で起動したとき。受け付けている間、起きている印
//!   （`presence.json`）を [`PRESENCE_EVERY`] ごとに書き直し、`inbox/` を [`INBOX_EVERY`] ごとに見る（画面のフレームの頭で。ファイルは小さい
//!   ので画面のスレッドで読む）。FBX と絵の読みは裏の仕事（`load`）。
//! - 頼みの当て方: 同じ相手（`target.key`）の文書を開いていれば送り直し（ポーズ・BlendShape・マテリアルの値・表示の入切を当てる。FBX の
//!   道か guid・使うメッシュ・マテリアルの付け方が変わったときだけ組み直し、FBX は道と guid が同じなら読み直さない）。違う相手なら、
//!   保存していない変更があれば「開く」と同じ確かめを経て、新しいプロジェクトに開く。描いている最中・保存の途中・ほかの読み込みの間に
//!   来た頼みは、終わってから当てる（断らずに待つ。待つ間は `claimed/` に置いたまま）。裏の仕事の間に利用者が文書を替えた（新規・開く）ときと、
//!   受け付けをやめたときは、結果を入れずに取り消して `declined` を返す。
//! - テクスチャセットはマテリアルの `key`（Unity のマテリアルのアセット）ごとに 1 つ。元の絵は新しく作ったセットと何も触っていない最初の
//!   セットの一番下のレイヤー（`originals`。Color の流し込み先の絵が PSD なら、PSD のレイヤーのまま）。送り直しで元の絵のファイルが変わって
//!   いれば、入れた直後のままのセットには入れ直し、触ったセットは変えずに知らせる（`SetOriginal`）。lilToon の値は受けた見た目（`look::link`）。
//! - 返事: 開いた・送り直しを当てた `opened`（合わなかった物は `problems`）、受けなかった `refused`、書き出した `exported`。
//! - .ylp: 当てた頼み（今のポーズを入れ、マテリアルの値を除いたもの）を根の `livelink.json` に残し、開き直すと Unity なしで同じモデルと
//!   ポーズになる（`store`）。

pub mod images;
pub mod layout;
pub mod load;
pub mod originals;
pub mod store;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yolu_protocol::files::{
    default_folder, Claimed, ExportedFile, Folder, Presence, Problem, Reason, Refusal, Reply,
    ReplyKind, Request, INBOX_EVERY, KEY_NONE, PRESENCE_EVERY,
};

use yolu_core::skin::{Pose, Rig};

use crate::lang::Lang;
use crate::model::SceneModel;
use crate::state::AppState;
use crate::view3d::model::ViewError;
use crate::view3d::pose::{self, RigJob};
use layout::Layout;
use load::{Inputs, LoadedFbx, Opened, Original, OriginalSource, SlotPicture, Watched};

pub use originals::{OriginalMark, OriginalMarks};

/// 名乗り（外からの操作の挨拶も使う）。
pub const AGENT: &str = concat!("YoluPainter ", env!("CARGO_PKG_VERSION"));
/// 返事・起きている印に書く版。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// フォルダを使えなかったとき、次に試すまでの間。
const RETRY: Duration = Duration::from_secs(5);
/// `claimed/` に残った頼みを片付けるまでの古さ（落ちたプロセスが残した物）。
const STALE_CLAIM: Duration = Duration::from_secs(24 * 3600);

/// 受け付けの様子。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LinkStatus {
    #[default]
    Off,
    Accepting,
    /// 受け渡しのフォルダを使えない（理由）。
    Failed(String),
}

/// 知らせの重さ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// 入口の印の様子（切っている・受け付け中・相手の文書・合わなかった物がある・フォルダを使えない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIndicator {
    Off,
    Waiting,
    Linked,
    Problems,
    Failed,
}

/// 画面が読む Live Link の様子の写し。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkView {
    pub status: LinkStatus,
    pub notice: Option<(NoticeLevel, String)>,
    /// 今の文書の相手の名前（Live Link の相手の文書なら）。
    pub target: Option<String>,
    /// 頼みを読んでいる・当てるのを待っている。
    pub working: bool,
    /// 最後に当てた頼みで合わなかった物（Unity が送れなかった物を含む）。
    pub problems: Vec<Problem>,
}

impl LinkView {
    /// 受け付けているか。
    pub fn is_on(&self) -> bool {
        self.status == LinkStatus::Accepting
    }

    pub fn indicator(&self) -> LinkIndicator {
        match &self.status {
            LinkStatus::Off => LinkIndicator::Off,
            LinkStatus::Failed(_) => LinkIndicator::Failed,
            LinkStatus::Accepting if self.target.is_none() => LinkIndicator::Waiting,
            LinkStatus::Accepting if !self.problems.is_empty() => LinkIndicator::Problems,
            LinkStatus::Accepting => LinkIndicator::Linked,
        }
    }

    /// ウィンドウの先頭に出す状態の名前（名前だけ）。
    pub fn state_label(&self, lang: Lang) -> &'static str {
        match &self.status {
            LinkStatus::Off => lang.pick("受け付けていない", "Not accepting"),
            LinkStatus::Accepting if self.working => lang.pick("読み込み中", "Loading"),
            LinkStatus::Accepting => lang.pick("受付中", "Accepting"),
            LinkStatus::Failed(_) => lang.pick("フォルダを使えません", "Folder unavailable"),
        }
    }

    /// ウィンドウの先頭とツールチップの 1 行目（「Live Link: 受付中」）。
    pub fn heading(&self, lang: Lang) -> String {
        format!("Live Link: {}", self.state_label(lang))
    }

    /// 開いている Unity のオブジェクトの行（「Unity: 名前」。相手の文書でなければ None）。
    pub fn target_line(&self) -> Option<String> {
        self.target.as_ref().map(|t| format!("Unity: {t}"))
    }

    /// 入口のアイコンのツールチップ: 状態・開いている Unity のオブジェクト・理由（フォルダを使えない・合わなかった物）。
    pub fn tooltip(&self, lang: Lang) -> String {
        let mut lines = vec![self.heading(lang)];
        lines.extend(self.target_line());
        if let LinkStatus::Failed(why) = &self.status {
            lines.push(why.clone());
        }
        for p in self.problems.iter().take(8) {
            lines.push(format!("{}: {}", p.path, reason_text(lang, &p.reason)));
        }
        if self.problems.len() > 8 {
            lines.push(format!("… {}", self.problems.len() - 8));
        }
        lines.join("\n")
    }
}

/// 理由の言葉の、人に見せる短い文（知らない言葉はそのまま）。
pub fn reason_text(lang: Lang, reason: &str) -> String {
    let Some(r) = Reason::parse(reason) else {
        return reason.to_owned();
    };
    match r {
        Reason::MeshNotFromFbx => {
            lang.pick("FBX のメッシュではありません", "Not a mesh from an FBX")
        }
        Reason::BoneNotFound => lang.pick("FBX の中に見つかりません", "Not found in the FBX"),
        Reason::AmbiguousBone => lang.pick(
            "同じ名前のボーンがあり、決まりません",
            "Ambiguous: bones with the same name",
        ),
        Reason::UnsupportedImport => lang.pick(
            "取り込みの設定（軸の変換の焼き込み）に合わせられません",
            "Unsupported import setting (Bake Axis Conversion)",
        ),
        Reason::FbxUnreadable => lang.pick("FBX を読めません", "Cannot read the FBX"),
        Reason::TextureUnreadable => lang.pick("テクスチャを読めません", "Cannot read the texture"),
        Reason::TooLarge => lang.pick("大きすぎます", "Too large"),
        Reason::FormatUnknown => lang.pick("形式が違います", "Unknown format"),
        Reason::Busy => lang.pick("今は受け付けられません", "Not now"),
        Reason::Declined => lang.pick("開くのをやめました", "Opening was cancelled"),
    }
    .to_owned()
}

/// 始める・やめるの頼み（メニューから。`YoluApp` が当てる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRequest {
    Start,
    Stop,
}

/// 今の文書の相手（`AppState::link_target`。保存・送り直し・書き出しが読む）。
#[derive(Clone, Debug)]
pub struct LinkTarget {
    /// 当てた頼み（マテリアルの値を含む。保存するときは今のポーズを入れ、値を除く）。
    pub request: Arc<Request>,
    pub layout: Arc<Layout>,
    /// ポーズのセッションの `Rig` の同一性（`SceneModel::rig_id`）。
    pub rig: usize,
    /// 利用者が書き出しのウィンドウで選んだ置き場（無ければ頼みの `export_dir`）。
    pub export_dir: Option<PathBuf>,
}

impl LinkTarget {
    /// 書き出しの置き場の既定。
    pub fn export_folder(&self) -> Option<PathBuf> {
        self.export_dir.clone().or_else(|| {
            let dir = self.request.target.export_dir.trim();
            (!dir.is_empty()).then(|| PathBuf::from(dir))
        })
    }
}

impl AppState {
    /// 書き出しのウィンドウの置き場の既定（Live Link の相手の文書なら、利用者が選んだ置き場か頼みの `export_dir`）。
    pub fn link_export_dir(&self) -> Option<PathBuf> {
        self.link_target.as_ref()?.export_folder()
    }

    /// 書き出しのウィンドウで置き場を選んだ（Live Link の相手の文書なら、次からの既定にする）。
    pub fn note_export_dir(&mut self, dir: &std::path::Path) {
        if let Some(t) = self.link_target.as_mut() {
            t.export_dir = Some(dir.to_path_buf());
        }
    }
}

/// テクスチャセットに入れた元の絵の覚え（セットの uid ごと。セッションの中だけで、保存しない）。送り直しで元の絵のファイルが変わったとき、
/// 入れた直後のままのセットには入れ直し、触ったセットは変えずに知らせる。
#[derive(Clone, Debug)]
struct SetOriginal {
    /// 入れた直後の文書（ID と版）。今のセットの文書が同じなら、何も触っていない。触ったセットと、.ylp から開き直したセット（入れた直後を
    /// 知らない）は None。
    untouched: Option<(u128, u64)>,
    /// 最後に見た元の絵のファイルの身元（変わったと知らせたら、新しい身元にする: 同じ変化を送り直しのたびに知らせない）。
    source: OriginalSource,
    /// 入れ直してもセットの大きさを変えない（新規プロジェクトのウィンドウで解像度を選んだ最初のセット）。
    keep_size: bool,
}

/// 元の絵を入れたセット（セットの uid・マテリアルの番号・大きさを変えないか）。
type Installed = (u32, usize, bool);

/// 拾った頼み 1 つ。
struct Taken {
    /// 受け渡しのフォルダの頼み（.ylp から開き直すものは None。返事を書かない）。
    claimed: Option<Claimed>,
    request: Result<Arc<Request>, Refusal>,
}

/// 裏の仕事。`epoch` は始めた時のプロジェクト（`AppState::project_epoch`）。替わっていたら、結果は入れずに捨てる。
enum Running {
    /// Rig を組み直す（FBX を読む・並べる・絵を読む）。
    Rig {
        job: RigJob<Opened>,
        /// 何も使えるレンダラーが無かったときの結果（返事の理由）。
        empty: Arc<Mutex<Option<Opened>>>,
        taken: Taken,
        epoch: u64,
    },
    /// 絵だけを読む（送り直しで Rig はそのまま）。
    Plain {
        rx: Receiver<Result<Opened, ViewError>>,
        cancel: Arc<AtomicBool>,
        taken: Taken,
        epoch: u64,
    },
}

impl Running {
    fn epoch(&self) -> u64 {
        match self {
            Running::Rig { epoch, .. } | Running::Plain { epoch, .. } => *epoch,
        }
    }

    fn taken(&self) -> &Taken {
        match self {
            Running::Rig { taken, .. } | Running::Plain { taken, .. } => taken,
        }
    }

    /// 仕事を止めて、頼みを返す。
    fn cancel(self) -> Taken {
        match self {
            Running::Rig { job, taken, .. } => {
                job.cancel();
                taken
            }
            Running::Plain { cancel, taken, .. } => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                taken
            }
        }
    }
}

/// Live Link（`YoluApp` が 1 つ持つ）。
pub struct LiveLink {
    /// 受け付けを設定に合わせて始めてよい（実際のウィンドウの起動・メニュー・試験がフォルダを決めたとき）。ウィンドウを作るだけの試験は、利用者の
    /// フォルダに触らない。
    armed: bool,
    /// 受け渡しのフォルダ（試験が差し替える。None は OS の既定）。
    root: Option<PathBuf>,
    /// `--livelink` で起動した（設定が切れていても受ける。メニューで切ると下ろす）。
    forced: bool,
    folder: Option<Folder>,
    /// ウィンドウを起こす口（実際のウィンドウ。試験は無し）。あれば、受け付けている間の見張り（`Watcher`）を立てる。
    waker: Option<egui::Context>,
    /// 起きている印を書き直し、`inbox/` に頼みが来たらウィンドウを起こす裏のスレッド（ウィンドウが描き直しを頼まない間も、印は古くならない）。
    watcher: Option<Watcher>,
    status: LinkStatus,
    retry_at: Option<Instant>,
    notice: Option<(NoticeLevel, String)>,
    next_presence: Option<Instant>,
    next_inbox: Option<Instant>,
    queue: VecDeque<Taken>,
    /// 保存していない変更を捨ててよいかを聞いている頼み。
    asking: Option<Taken>,
    /// 聞いた答え（`answer_discard`）。
    answer: Option<bool>,
    job: Option<Running>,
    /// 読んだ FBX（同じ道と guid なら読み直さない）。
    cache: Vec<Arc<LoadedFbx>>,
    /// 読んだスロットの絵（同じファイルなら読み直さない）。
    slot_cache: Vec<Arc<SlotPicture>>,
    /// セットに入れた元の絵の覚え（セットの uid ごと）。
    set_originals: BTreeMap<u32, SetOriginal>,
    /// 頼みの id ごとの、次の返事の番号。
    counters: BTreeMap<String, u32>,
    problems: Vec<Problem>,
}

impl Default for LiveLink {
    fn default() -> Self {
        LiveLink::new()
    }
}

impl Drop for LiveLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 受け付けている間の見張りのスレッド: [`PRESENCE_EVERY`] ごとに起きている印を書き直し、[`INBOX_EVERY`] ごとに `inbox/` を見て、頼みが
/// あればウィンドウを起こす（拾うのは画面のスレッド）。落とすと止まる。
struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    fn start(folder: Folder, ctx: egui::Context) -> Watcher {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("yolu-livelink-watch".into())
            .spawn(move || {
                let mut next_presence = Instant::now();
                while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                    let now = Instant::now();
                    if now >= next_presence {
                        let _ = folder.write_presence(&Presence::now(VERSION));
                        next_presence = now + PRESENCE_EVERY;
                    }
                    if folder.waiting().is_ok_and(|w| !w.is_empty()) {
                        ctx.request_repaint();
                    }
                    std::thread::sleep(INBOX_EVERY);
                }
            })
            .ok();
        Watcher { stop, thread }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl LiveLink {
    /// 見張りを止め、起きている印を消す（落とすときと同じ。デストラクターを走らせずにプロセスを終えるとき、先にこれを呼ぶ）。
    pub fn shutdown(&mut self) {
        self.watcher = None;
        if let Some(f) = &self.folder {
            let _ = f.remove_presence();
        }
    }

    pub fn new() -> LiveLink {
        LiveLink {
            armed: false,
            root: None,
            forced: false,
            folder: None,
            waker: None,
            watcher: None,
            status: LinkStatus::Off,
            retry_at: None,
            notice: None,
            next_presence: None,
            next_inbox: None,
            queue: VecDeque::new(),
            asking: None,
            answer: None,
            job: None,
            cache: Vec::new(),
            slot_cache: Vec::new(),
            set_originals: BTreeMap::new(),
            counters: BTreeMap::new(),
            problems: Vec::new(),
        }
    }

    /// 受け渡しのフォルダを替える（試験。受け付けていないときだけ）。
    pub fn set_folder(&mut self, root: PathBuf) -> Result<(), String> {
        if self.folder.is_some() {
            return Err("受け付けている間はフォルダを替えません。".into());
        }
        self.root = Some(root);
        self.armed = true;
        Ok(())
    }

    /// 受け付けを設定に合わせて始めてよいことにする（実際のウィンドウの起動）。`ctx` は、頼みが来たときにウィンドウを起こす口。
    pub fn arm(&mut self, ctx: &egui::Context) {
        self.armed = true;
        self.waker = Some(ctx.clone());
    }

    /// `--livelink` で起動した（設定によらず受ける）。
    pub fn force(&mut self) {
        self.forced = true;
        self.armed = true;
    }

    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// 受け渡しのフォルダ（受け付けていなければ None）。
    pub fn folder(&self) -> Option<&Folder> {
        self.folder.as_ref()
    }

    /// 次に回したい時までの間（裏の仕事・待っている頼みの間は短く。見張りのスレッドが無ければ `inbox/` を見る間隔）。
    pub fn next_wake(&self) -> Option<Duration> {
        if self.job.is_some() || !self.queue.is_empty() {
            Some(Duration::from_millis(50))
        } else if self.retry_at.is_some() {
            Some(RETRY)
        } else if self.folder.is_some() && self.watcher.is_none() {
            Some(INBOX_EVERY)
        } else {
            None
        }
    }

    /// 頼みを読んでいる・当てるのを待っているか（試験・診断用）。
    pub fn is_working(&self) -> bool {
        self.job.is_some() || !self.queue.is_empty() || self.asking.is_some()
    }

    /// 画面に写す様子。
    pub fn view(&self, state: &AppState) -> LinkView {
        LinkView {
            status: self.status.clone(),
            notice: self.notice.clone(),
            target: state
                .link_target
                .as_ref()
                .filter(|_| linked(state))
                .map(|t| t.request.target.name.clone()),
            working: self.is_working(),
            problems: self.problems.clone(),
        }
    }

    fn notify(&mut self, level: NoticeLevel, text: String, state: &mut AppState) {
        let kind = match level {
            NoticeLevel::Info => crate::notice::Kind::Info,
            NoticeLevel::Warning => crate::notice::Kind::Warning,
            NoticeLevel::Error => crate::notice::Kind::Error,
        };
        state.notify(kind, crate::notice::Source::LiveLink, text.clone());
        self.notice = Some((level, text));
    }

    /// 始める・やめるの頼みを当てる（メニュー。設定「Unity の Live Link を受け付ける」を入れる・切る）。
    pub fn request(&mut self, request: LinkRequest, state: &mut AppState) {
        self.armed = true;
        match request {
            LinkRequest::Start => state.prefs.settings.livelink_on_startup = true,
            LinkRequest::Stop => {
                state.prefs.settings.livelink_on_startup = false;
                self.forced = false;
            }
        }
        self.sync(state, Instant::now());
    }

    /// 保存していない変更を捨ててよいかを聞きたい頼みがあるか（`YoluApp` がウィンドウで聞いて `answer_discard` を呼ぶ）。
    pub fn wants_discard(&self) -> bool {
        self.asking.is_some() && self.answer.is_none()
    }

    /// 聞いた答え（捨ててよければ true）。
    pub fn answer_discard(&mut self, discard: bool) {
        if self.asking.is_some() {
            self.answer = Some(discard);
        }
    }

    /// 受け付けの入切を設定に合わせる。
    fn sync(&mut self, state: &mut AppState, now: Instant) {
        let want = self.armed && (state.prefs.settings.livelink_on_startup || self.forced);
        if !want {
            if self.folder.is_some() {
                self.decline_pending(state);
            }
            self.watcher = None;
            if let Some(f) = self.folder.take() {
                let _ = f.remove_presence();
            }
            if self.status != LinkStatus::Off {
                self.status = LinkStatus::Off;
                self.retry_at = None;
            }
            return;
        }
        if self.folder.is_some() || self.retry_at.is_some_and(|t| now < t) {
            return;
        }
        let root = self.root.clone().or_else(default_folder);
        let opened = match root {
            Some(root) => Folder::open(root).map_err(|e| e.to_string()),
            None => Err(state
                .lang
                .pick("フォルダの場所が決まりません", "The folder is not known")
                .to_owned()),
        };
        match opened {
            Ok(f) => {
                let _ = f.sweep_claimed(STALE_CLAIM);
                // 起きている印は始めてすぐ書く（見張りのスレッドの最初の回を待たない）
                let _ = f.write_presence(&Presence::now(VERSION));
                self.next_presence = Some(now + PRESENCE_EVERY);
                self.watcher = self.waker.clone().map(|ctx| Watcher::start(f.clone(), ctx));
                self.folder = Some(f);
                self.status = LinkStatus::Accepting;
                self.retry_at = None;
                self.next_inbox = None;
            }
            Err(e) => {
                let text = state.lang.pick(
                    format!("Live Link: 受け渡しのフォルダを使えません（{e}）"),
                    format!("Live Link: Cannot use the exchange folder ({e})"),
                );
                if self.status != LinkStatus::Failed(text.clone()) {
                    self.notify(NoticeLevel::Error, text.clone(), state);
                }
                self.status = LinkStatus::Failed(text);
                self.retry_at = Some(now + RETRY);
            }
        }
    }

    /// 毎フレームの頭で: 受け付けを合わせ、起きている印を書き、頼みを拾い、裏の仕事の終わりを受け、待っている頼みを当てる。
    pub fn poll(&mut self, state: &mut AppState) {
        let now = Instant::now();
        if let Some(request) = state.link_reopen.take() {
            self.reopen(request);
        }
        self.sync(state, now);
        self.drop_stale_job(state);
        // 相手の文書でなくなった（別のモデル・新しいプロジェクト・開いた）なら、相手を忘れる
        if state.link_target.is_some() && !linked(state) && self.job.is_none() {
            state.link_target = None;
            self.problems.clear();
        }
        if let Some(folder) = &self.folder {
            // 見張りのスレッドが無い（試験）ときは、起きている印をここで書き直す
            if self.watcher.is_none() && self.next_presence.is_none_or(|t| now >= t) {
                let _ = folder.write_presence(&Presence::now(VERSION));
                self.next_presence = Some(now + PRESENCE_EVERY);
            }
            if self.next_inbox.is_none_or(|t| now >= t) {
                self.next_inbox = Some(now + INBOX_EVERY);
                if let Ok(waiting) = folder.waiting() {
                    for path in waiting {
                        if let Ok(Some(claimed)) = folder.claim(&path) {
                            let request = claimed.read().map(Arc::new);
                            self.queue.push_back(Taken {
                                claimed: Some(claimed),
                                request,
                            });
                        }
                    }
                }
            }
        }
        self.poll_job(state);
        self.next(state);
    }

    /// プロジェクトが替わったあとも走っている裏の仕事は、結果を入れずに止めて断る（新しい文書へ Unity のモデルを流し込まない）。
    fn drop_stale_job(&mut self, state: &mut AppState) {
        if self
            .job
            .as_ref()
            .is_none_or(|j| j.epoch() == state.project_epoch)
        {
            return;
        }
        if let Some(running) = self.job.take() {
            let taken = running.cancel();
            self.decline_replaced(&taken, state);
        }
    }

    /// 文書が替わったので当てなかった頼みに、`declined` を返す。
    fn decline_replaced(&mut self, taken: &Taken, state: &mut AppState) {
        let text = state.lang.pick(
            "Live Link: プロジェクトが替わったので、開きませんでした。",
            "Live Link: Not opened (the project was replaced).",
        );
        self.decline(taken, text, state);
    }

    /// 当てなかった頼みに `declined` を返し、claimed から消す（`text` は画面の知らせ）。
    fn decline(&mut self, taken: &Taken, text: &str, state: &mut AppState) {
        let name = taken
            .request
            .as_ref()
            .map(|r| r.target.name.clone())
            .unwrap_or_default();
        self.reply(
            taken,
            ReplyKind::Refused,
            vec![Problem::new(name, Reason::Declined)],
        );
        self.finish(taken);
        self.notify(NoticeLevel::Info, text.to_owned(), state);
    }

    /// 受け付けをやめるとき: Unity の頼みで、まだ当てていない物と走っている裏の仕事は、取り消して断る（Unity に返事を待たせない。
    /// 返事を書くフォルダを手放す前に呼ぶ）。.ylp の開き直しは Unity なしの操作なので、続ける。
    fn decline_pending(&mut self, state: &mut AppState) {
        let (keep, waiting): (VecDeque<Taken>, VecDeque<Taken>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|t| t.claimed.is_none());
        self.queue = keep;
        let mut declined: Vec<Taken> = waiting.into_iter().collect();
        declined.extend(self.asking.take());
        self.answer = None;
        if self
            .job
            .as_ref()
            .is_some_and(|j| j.taken().claimed.is_some())
        {
            declined.extend(self.job.take().map(Running::cancel));
        }
        let text = state.lang.pick(
            "Live Link: 受け付けをやめたので、開きませんでした。",
            "Live Link: Not opened (no longer accepting).",
        );
        for taken in &declined {
            self.decline(taken, text, state);
        }
    }

    /// .ylp を開いたとき: 残っていた頼み（`livelink.json`）を、Unity なしで開き直す（返事は書かない）。
    pub fn reopen(&mut self, request: Request) {
        self.queue.push_front(Taken {
            claimed: None,
            request: Ok(Arc::new(request)),
        });
    }

    /// 次の頼みを当てる（当てられる間だけ）。
    fn next(&mut self, state: &mut AppState) {
        if self.job.is_some() {
            return;
        }
        let taken = match self.asking.take() {
            Some(t) => t,
            None => match self.queue.pop_front() {
                Some(t) => t,
                None => return,
            },
        };
        let request = match &taken.request {
            Err(refusal) => {
                let refusal = refusal.clone();
                let why = reason_text(state.lang, refusal.reason.as_str());
                let text = state.lang.pick(
                    format!("Live Link: 開きませんでした（{why}）。"),
                    format!("Live Link: Not opened ({why})."),
                );
                self.reply(
                    &taken,
                    ReplyKind::Refused,
                    vec![Problem::new("", refusal.reason)],
                );
                self.finish(&taken);
                self.notify(NoticeLevel::Warning, text, state);
                return;
            }
            Ok(r) => r.clone(),
        };
        // 描いている最中・保存の途中・ほかの読み込みの間は、終わってから
        if busy(state) {
            self.queue.push_front(taken);
            return;
        }
        let same = same_target(state, &request);
        if !same && taken.claimed.is_some() {
            if state.shows_modified() {
                match self.answer.take() {
                    None => {
                        self.asking = Some(taken);
                        return;
                    }
                    Some(false) => {
                        self.reply(
                            &taken,
                            ReplyKind::Refused,
                            vec![Problem::new(request.target.name.clone(), Reason::Declined)],
                        );
                        self.finish(&taken);
                        let text = state.lang.pick(
                            "Live Link: 保存していない変更があるので、開きませんでした。",
                            "Live Link: Not opened (unsaved changes).",
                        );
                        self.notify(NoticeLevel::Info, text.into(), state);
                        return;
                    }
                    Some(true) => {}
                }
            }
            self.answer = None;
            if !state.is_pristine() {
                crate::project::new_into(state);
            }
        }
        self.start(taken, request, same, state);
    }

    /// 裏の仕事を始める。
    fn start(&mut self, taken: Taken, request: Arc<Request>, same: bool, state: &mut AppState) {
        let target = state.link_target.clone().filter(|_| same);
        let keep_rig = target
            .as_ref()
            .is_some_and(|t| rig_unchanged(&t.request, &request));
        // 元の絵を読むマテリアル: 送り直しは、まだセットの無いマテリアルだけ（新しく作るセット）。.ylp から開き直すときは、絵はセットの中に
        // 残っているので、元の絵は入れ直さない（読んでも使わず、読めない絵が合わない物として出るだけ）
        let reopening = taken.claimed.is_none();
        // セットのあるマテリアルは、元の絵のファイルが変わったかを見る（送り直しは覚えと比べ、開き直しは今の身元を覚えるだけ）
        let mut originals: Vec<usize> = Vec::new();
        let mut watched: Vec<Watched> = Vec::new();
        for (i, m) in request.materials.iter().enumerate() {
            if reopening {
                watched.push(Watched {
                    material: i,
                    known: None,
                    untouched: false,
                });
                continue;
            }
            let key = crate::sets::material_from_link(&load::material_key(&m.key, &m.name));
            let set = same
                .then(|| {
                    state
                        .sets
                        .iter()
                        .position(|s| s.bound.is_some() && s.material == key)
                })
                .flatten();
            let Some(index) = set else {
                originals.push(i);
                continue;
            };
            let record = state
                .sets
                .get(index)
                .and_then(|s| self.set_originals.get(&s.uid));
            let doc = state.set_doc(index);
            watched.push(Watched {
                material: i,
                known: record.map(|r| r.source.clone()),
                untouched: record.is_some_and(|r| r.untouched == Some((doc.id(), doc.revision()))),
            });
        }
        let inputs = Inputs {
            request,
            cache: self.cache.clone(),
            slot_cache: self.slot_cache.clone(),
            originals,
            watched,
            keep_rig,
            psd_budget: state.load_source_bytes(),
        };
        let epoch = state.project_epoch;
        if keep_rig {
            let (tx, rx) = channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            std::thread::spawn(move || {
                let _ = tx.send(load::run(inputs, &flag).map(|(_, _, opened)| opened));
            });
            self.job = Some(Running::Plain {
                rx,
                cancel,
                taken,
                epoch,
            });
        } else {
            let empty = Arc::new(Mutex::new(None));
            let slot = empty.clone();
            let job = pose::prepare_rig_with(&mut state.view3d, move |cancel| {
                let (rig, warnings, mut opened) = load::run(inputs, cancel)?;
                match rig {
                    Some(rig) => Ok((rig, warnings, std::mem::take(&mut opened.takes), opened)),
                    None => {
                        *slot.lock().expect("ロックを持ったまま落ちない") = Some(opened);
                        Err(ViewError::NoTriangles)
                    }
                }
            });
            self.job = Some(Running::Rig {
                job,
                empty,
                taken,
                epoch,
            });
        }
    }

    /// 裏の仕事の終わりを受ける。
    fn poll_job(&mut self, state: &mut AppState) {
        let Some(running) = self.job.take() else {
            return;
        };
        // 描いている最中は入れない（終わってから）
        if state.is_stroking() {
            self.job = Some(running);
            return;
        }
        match running {
            Running::Rig {
                job,
                empty,
                taken,
                epoch,
            } => match job.poll() {
                None => {
                    self.job = Some(Running::Rig {
                        job,
                        empty,
                        taken,
                        epoch,
                    })
                }
                Some(Ok((prepared, opened))) => {
                    self.install(taken, opened, Some(prepared), state);
                }
                Some(Err(e)) => {
                    let opened = empty.lock().ok().and_then(|mut s| s.take());
                    self.failed(taken, opened, e, state);
                }
            },
            Running::Plain {
                rx,
                cancel,
                taken,
                epoch,
            } => match rx.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.job = Some(Running::Plain {
                        rx,
                        cancel,
                        taken,
                        epoch,
                    })
                }
                Ok(Ok(opened)) => self.install(taken, opened, None, state),
                Ok(Err(e)) => self.failed(taken, None, e, state),
                Err(TryRecvError::Disconnected) => {
                    self.failed(taken, None, ViewError::LoadStopped, state)
                }
            },
        }
    }

    /// 頼みを当てられなかった（使えるレンダラーが無い・組めない）。
    fn failed(&mut self, taken: Taken, opened: Option<Opened>, e: ViewError, state: &mut AppState) {
        let mut problems = opened.map(|o| o.problems).unwrap_or_default();
        if problems.is_empty() {
            problems.push(Problem::new("", Reason::FbxUnreadable));
        }
        let lang = state.lang;
        let name = taken
            .request
            .as_ref()
            .map(|r| r.target.name.clone())
            .unwrap_or_default();
        let why = match e {
            ViewError::NoTriangles => problems
                .first()
                .map(|p| reason_text(lang, &p.reason))
                .unwrap_or_default(),
            e => lang.view_error(&e),
        };
        self.reply(&taken, ReplyKind::Refused, problems.clone());
        self.finish(&taken);
        self.problems = problems;
        let text = lang.pick(
            format!("Live Link: 「{name}」を開けません（{why}）。"),
            format!("Live Link: Cannot open “{name}” ({why})."),
        );
        self.notify(NoticeLevel::Warning, text, state);
    }

    /// 終わった裏の仕事の結果を入れる。
    fn install(
        &mut self,
        taken: Taken,
        mut opened: Opened,
        prepared: Option<pose::PreparedModel>,
        state: &mut AppState,
    ) {
        let lang = state.lang;
        let request = opened.request.clone();
        for f in &opened.fbx {
            if !self.cache.iter().any(|c| Arc::ptr_eq(c, f)) {
                self.cache.push(f.clone());
            }
        }
        // 使わなくなった FBX は手放す（今の頼みが使う物だけ）
        self.cache
            .retain(|c| opened.fbx.iter().any(|f| Arc::ptr_eq(c, f)));
        self.slot_cache.clone_from(&opened.slot_cache);
        let reopening = taken.claimed.is_none();
        // 今の文書が、この頼みと同じ相手のままか。裏の仕事の間に別のモデルを開かれていれば、相手の文書ではない
        let previous = state.link_target.clone().filter(|_| linked(state));
        let resend = previous
            .as_ref()
            .is_some_and(|p| p.request.target.key == request.target.key);
        // 同じ相手の組み直し（入切・メッシュ・付け方の変更）も送り直し。初めて入れる（前の相手が無い・違う相手）ときだけ、ポーズを取り消しの段にしない
        let first_install = !resend;
        // 組み直した Rig は新しいセッション（取り消しの段が空・休みのポーズ）になるので、前のポーズを先に控えておく
        let rebuilt = prepared.is_some();
        let carried = (rebuilt && resend)
            .then(|| state.view3d.pose.session.as_ref().map(|s| s.pose().clone()))
            .flatten();
        let (rig, layout, fresh, pristine_first) = match prepared {
            Some(prepared) => {
                let pristine_first = (first_install && !reopening && state.is_pristine())
                    .then(|| state.sets.current().uid);
                if first_install && !reopening {
                    // 前のモデルのファイルの参照を引きずらない（保存すると view.json・pose.json が、前の FBX と Unity の Rig のポーズを指してしまう）
                    state.np.model_file = None;
                }
                pose::install_prepared(&mut state.view3d, prepared);
                let rig = state
                    .view3d
                    .pose
                    .session
                    .as_ref()
                    .expect("入れたセッション")
                    .rig
                    .clone();
                state.model = Some(SceneModel::from_live_link(
                    &rig,
                    &request.target.key,
                    &request.target.name,
                    opened.materials.clone(),
                ));
                let report = if reopening {
                    state.bind_model_only()
                } else {
                    state.bind_model()
                };
                if state.sets.current().bound.is_none() {
                    if let Some(i) = state.sets.iter().position(|s| s.bound.is_some()) {
                        let _ = state.switch_set(i);
                    }
                }
                let mut fresh = report.created_sets.clone();
                fresh.extend(pristine_first);
                (rig, Arc::new(opened.layout.clone()), fresh, pristine_first)
            }
            None => {
                // 組み直さずに絵だけを読んだ仕事は、同じ相手の文書にだけ入れる（別のモデルに替わっていたら断る）
                let session_rig = state.view3d.pose.session.as_ref().map(|s| s.rig.clone());
                let (Some(target), Some(rig)) = (previous.clone().filter(|_| resend), session_rig)
                else {
                    self.decline_replaced(&taken, state);
                    return;
                };
                if let Some(m) = state.model.as_mut() {
                    m.materials = opened.materials.clone();
                }
                let report = state.bind_model();
                (
                    rig,
                    target.layout.clone(),
                    report.created_sets.clone(),
                    None,
                )
            }
        };
        // 元の絵（新しく作ったセットと、何も触っていない最初のセット）
        let mut notes: Vec<String> = Vec::new();
        let fresh_uids: Vec<u32> = fresh.clone();
        let mut installed: Vec<Installed> = Vec::new();
        for uid in fresh {
            let Some(index) = state.sets.index_of(uid) else {
                continue;
            };
            let Some(m) = state.sets.get(index).and_then(|s| s.bound) else {
                continue;
            };
            let m = m as usize;
            let refit = pristine_first == Some(uid) && !state.resolution_chosen;
            let keep_size = pristine_first == Some(uid) && state.resolution_chosen;
            if let Some(original) = opened.originals.remove(&m) {
                put_original(
                    state, index, &request, m, original, &opened, refit, &mut notes,
                );
                installed.push((uid, m, keep_size));
            }
        }
        // 送り直し: 元の絵のファイルが変わったセット。入れた直後のままなら入れ直し、触ったセットは変えずに知らせる
        let mut changed: Vec<String> = Vec::new();
        for index in 0..state.sets.len() {
            let Some((uid, m, name)) = state
                .sets
                .get(index)
                .and_then(|s| s.bound.map(|m| (s.uid, m as usize, s.name.clone())))
            else {
                continue;
            };
            if fresh_uids.contains(&uid) {
                continue;
            }
            let Some(source) = opened.sources.get(&m).cloned() else {
                continue;
            };
            let doc = state.set_doc(index);
            let now = (doc.id(), doc.revision());
            let Some(record) = self.set_originals.get_mut(&uid) else {
                // 覚えの無いセット（.ylp から開き直した・前からあった）は、今の身元を覚えるだけ
                self.set_originals.insert(
                    uid,
                    SetOriginal {
                        untouched: None,
                        source,
                        keep_size: false,
                    },
                );
                continue;
            };
            if record.untouched != Some(now) {
                record.untouched = None;
            }
            if record.source == source {
                continue;
            }
            let keep_size = record.keep_size;
            let original = opened
                .originals
                .remove(&m)
                .filter(|_| record.untouched.is_some());
            if let Some(Original::Unreadable { path, why }) = &original {
                // 読めなかった（壊れている・書き込みの途中など）: 今のセットを残し、見た身元も前のままにして、次の送り直しでもう一度読む
                // （読めない間に身元だけ覚えると、時刻と大きさが同じまま読めるようになっても入れ直さない）
                notes.push(originals::unreadable_kept_note(lang, &name, path, why));
                continue;
            }
            match original {
                Some(original) => {
                    if let Err(e) = originals::reset_untouched(state, index, lang) {
                        notes.push(format!("{name}: {e}"));
                        continue;
                    }
                    record.source = source.clone();
                    put_original(
                        state, index, &request, m, original, &opened, !keep_size, &mut notes,
                    );
                    installed.push((uid, m, keep_size));
                    // 保存したプロジェクトの中身が変わった
                    state.modified = true;
                }
                None => {
                    record.source = source.clone();
                    changed.extend(
                        source
                            .path
                            .as_deref()
                            .map(|path| originals::changed_note(lang, &name, path)),
                    );
                }
            }
        }
        // 受けた見た目（lilToon の値と、スロットの絵）
        for index in 0..state.sets.len() {
            let Some(m) = state.sets.get(index).and_then(|s| s.bound) else {
                continue;
            };
            let Some(material) = request.materials.get(m as usize) else {
                continue;
            };
            let images: BTreeMap<String, Arc<yolu_core::look::ReceivedImage>> = opened
                .slots
                .iter()
                .filter(|((mi, _), r)| *mi == m as usize && r.is_ok())
                .map(|((_, slot), r)| (slot.clone(), r.clone().expect("上で見た")))
                .collect();
            let missing: BTreeMap<String, yolu_core::look::MissingImage> = opened
                .slots
                .iter()
                .filter_map(|((mi, slot), r)| match r {
                    Err(why) if *mi == m as usize => Some((slot.clone(), *why)),
                    _ => None,
                })
                .collect();
            let received =
                if reopening && material.values == store::without_values(&material.values) {
                    // .ylp から開き直した: 値は look.json に残っている。絵だけを入れる
                    state.set_doc(index).received_look().cloned().map(|mut r| {
                        for (slot, image) in &images {
                            r.missing.remove(slot);
                            r.images.insert(slot.clone(), image.clone());
                        }
                        for (slot, why) in &missing {
                            if !r.images.contains_key(slot) {
                                r.missing.insert(slot.clone(), *why);
                            }
                        }
                        r
                    })
                } else {
                    crate::look::link::received_look(material, &images, &missing)
                };
            if let Err(e) = state.set_doc_mut(index).set_received_look(received) {
                notes.push(lang.core_error(&e));
            }
        }
        // ポーズ（初めて入れたモデルは取り消しの段にしない。送り直しは 1 つの段で、手で動かした分も上書きする）
        let (pose, mut problems) = layout.pose(&rig, &request);
        let posed = if first_install {
            pose::restore_pose(&mut state.view3d, pose)
        } else {
            if let Some(before) = carried.and_then(|b| carried_pose(&rig, b)) {
                // 組み直した Rig に前のポーズを戻してから（段にしない）、送り直しを 1 つの段にする。骨の数も変わって戻せなければ、休みから
                let _ = pose::restore_pose(&mut state.view3d, before);
            }
            pose::set_pose(&mut state.view3d, pose)
        };
        if let Err(e) = posed {
            notes.push(lang.view_error(&e));
        }
        // 送り直しで変わったポーズと、組み直した Rig の入切・メッシュ・付け方は、保存する変更（相手の文書は .ylp の `livelink.json` に残る）
        if !first_install {
            pose::sync_modified(state);
            if rebuilt {
                state.modified = true;
            }
        }
        // 入れた元の絵を覚える（入れた直後の文書の ID と版。受けた見た目とポーズは文書の版を進めない）。無くなったセットの覚えは捨てる
        for (uid, m, keep_size) in installed {
            let Some(index) = state.sets.index_of(uid) else {
                continue;
            };
            let doc = state.set_doc(index);
            self.set_originals.insert(
                uid,
                SetOriginal {
                    untouched: Some((doc.id(), doc.revision())),
                    source: opened.sources.get(&m).cloned().unwrap_or_default(),
                    keep_size,
                },
            );
        }
        self.set_originals
            .retain(|uid, _| state.sets.index_of(*uid).is_some());
        problems.splice(0..0, opened.problems.iter().cloned());
        problems.dedup();
        state.link_target = Some(LinkTarget {
            request: request.clone(),
            layout,
            rig: SceneModel::rig_id(&rig),
            export_dir: previous
                .filter(|p| p.request.target.key == request.target.key)
                .and_then(|p| p.export_dir),
        });
        if first_install && !reopening {
            state.view3d.pose.focus = true;
        }
        if reopening {
            // .ylp を開き直したモデル（ポーズも当てた後）で、焼いたマップを照合し直す
            state.expect_reopen_check();
        }
        self.reply(&taken, ReplyKind::Opened, problems.clone());
        self.finish(&taken);
        let mut shown = problems.clone();
        shown.extend(request.refused.iter().cloned());
        self.problems = shown;
        let name = &request.target.name;
        let mut text = if reopening {
            lang.pick(
                format!("Live Link: 「{name}」のモデルを開き直しました。"),
                format!("Live Link: Reopened the model of “{name}”."),
            )
        } else if first_install {
            lang.pick(
                format!("Live Link: 「{name}」を開きました。"),
                format!("Live Link: Opened “{name}”."),
            )
        } else {
            lang.pick(
                format!("Live Link: 「{name}」を送り直しで更新しました。"),
                format!("Live Link: Updated “{name}”."),
            )
        };
        if !self.problems.is_empty() {
            text += &lang.pick(
                format!(" 合わない物 {} 件。", self.problems.len()),
                format!(" {} item(s) did not fit.", self.problems.len()),
            );
        }
        for n in notes.iter().chain(&changed) {
            let n = n.trim_end_matches(['。', '.']);
            text += &lang.pick(format!(" {n}。"), format!(" {n}."));
        }
        let level = if notes.is_empty() && self.problems.is_empty() {
            NoticeLevel::Info
        } else {
            NoticeLevel::Warning
        };
        self.notify(level, text, state);
    }

    /// 返事を書く（.ylp から開き直したものには書かない）。
    fn reply(&mut self, taken: &Taken, kind: ReplyKind, problems: Vec<Problem>) {
        let (Some(folder), Some(claimed)) = (&self.folder, &taken.claimed) else {
            return;
        };
        let Some(id) = claimed.reply_id(taken.request.as_ref().ok().map(|r| r.as_ref())) else {
            return;
        };
        let mut reply = Reply::new(&id, kind, VERSION);
        reply.problems = problems;
        let n = self.counters.entry(id).or_insert(0);
        if folder.write_reply(&reply, *n).is_ok() {
            *n += 1;
        }
    }

    fn finish(&mut self, taken: &Taken) {
        if let Some(c) = &taken.claimed {
            let _ = c.finish();
        }
    }

    /// 書き出しが終わった（Live Link の相手の文書なら、書いた PNG のうちマテリアルのプロパティに当たる物を `exported` の返事にする）。
    /// `files` は（書いたファイル・セットの uid・lilToon のプロパティ）。
    pub fn exported(
        &mut self,
        state: &mut AppState,
        files: &[(yolu_io::export::WrittenImage, u32, Option<String>)],
    ) {
        let Some(target) = state.link_target.clone().filter(|_| linked(state)) else {
            return;
        };
        let Some(folder) = &self.folder else {
            return;
        };
        let mut reply = Reply::new(&target.request.id, ReplyKind::Exported, VERSION);
        for (image, uid, property) in files {
            let Some(property) = property else { continue };
            let Some(m) = state.sets.by_uid(*uid).and_then(|s| s.bound) else {
                continue;
            };
            let Some(material) = target
                .request
                .materials
                .get(m as usize)
                .filter(|m| m.key != KEY_NONE)
            else {
                continue;
            };
            reply.files.push(ExportedFile {
                material: material.key.clone(),
                property: property.clone(),
                path: image.path.to_string_lossy().replace('\\', "/"),
                srgb: image.srgb,
                normal_map: image.normal_map,
            });
        }
        if reply.files.is_empty() {
            return;
        }
        let n = self.counters.entry(reply.request.clone()).or_insert(0);
        if folder.write_reply(&reply, *n).is_ok() {
            *n += 1;
        }
    }
}

/// 読んだ元の絵 1 つを、セット `index`（マテリアル `m` に付いた、新しく作った・何も触っていない・入れ直すために空に戻したセット）に入れる。
/// 落とした物・入れられなかった理由は `notes` に。`refit` は元の絵の大きさで文書を作り直してよいか。
#[allow(clippy::too_many_arguments)]
fn put_original(
    state: &mut AppState,
    index: usize,
    request: &Request,
    m: usize,
    original: Original,
    opened: &Opened,
    refit: bool,
    notes: &mut Vec<String>,
) {
    let lang = state.lang;
    let converted = request
        .materials
        .get(m)
        .and_then(|mat| {
            mat.textures.iter().find(|t| {
                crate::look::link::SHOWN
                    .iter()
                    .any(|(_, p)| *p == t.property)
            })
        })
        .is_some_and(|t| !t.srgb && t.path.is_some());
    let name = state
        .sets
        .get(index)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let result = match original {
        Original::Picture(p) => {
            if let Some((path, why)) = opened.flattened.get(&m) {
                notes.push(originals::flattened_note(lang, &name, path, why));
            }
            originals::install(state, index, &p, converted, refit, lang)
        }
        Original::Layers(layers) => {
            let note = originals::layers_note(lang, &name, &layers);
            let path = layers.path.clone();
            let installed = originals::install_layers(state, index, *layers, lang);
            if installed.is_ok() {
                // 同じファイルへ PSD を書き出すときは、置き換える前に確かめる（PSD の取り込みと同じ）
                state.psd.remember_imported(path);
                notes.extend(note);
            }
            installed
        }
        Original::White => originals::install_white(state, index, lang),
        Original::Unreadable { path, why } => {
            notes.push(format!("{path}: {}", why.text(lang)));
            originals::install_white(state, index, lang)
        }
    };
    if let Err(e) = result {
        notes.push(format!("{name}: {e}"));
    }
}

/// 頼みを当てずに待つ間か（描いている最中・保存の途中・ほかのモデルの読み込みの間）。
fn busy(state: &AppState) -> bool {
    state.is_stroking()
        || state.is_saving()
        || state.np.reopening.is_some()
        || state.view3d.pose.is_loading()
}

/// 今の文書が、記録の相手の文書のままか（モデルの記録が Live Link の相手で、ポーズのセッションが同じ Rig）。
fn linked(state: &AppState) -> bool {
    let Some(target) = &state.link_target else {
        return false;
    };
    let record = state
        .model
        .as_ref()
        .and_then(|m| m.link_key().map(|k| (k, m.source.rig())));
    let session = state
        .view3d
        .pose
        .session
        .as_ref()
        .map(|s| SceneModel::rig_id(&s.rig));
    matches!(record, Some((key, Some(rig))) if key == target.request.target.key && rig == target.rig)
        && session == Some(target.rig)
}

/// 頼みが今の文書と同じ相手か。
fn same_target(state: &AppState, request: &Request) -> bool {
    linked(state)
        && state
            .link_target
            .as_ref()
            .is_some_and(|t| t.request.target.key == request.target.key)
}

/// 前の Rig のポーズを、組み直した Rig に戻せる形にする。そのまま当てはまればそのまま。メッシュ・BlendShape の数が変わっていれば、骨の値
/// （骨の数が同じとき）だけ引き継ぎ、重みは休み。骨の数も違えば戻せない。
fn carried_pose(rig: &Rig, before: Pose) -> Option<Pose> {
    if rig.check_pose(&before).is_ok() {
        return Some(before);
    }
    let mut pose = rig.rest_pose();
    if pose.locals.len() != before.locals.len() {
        return None;
    }
    pose.locals = before.locals;
    Some(pose)
}

/// 送り直しで Rig を組み直さなくてよいか（FBX・取り込みの設定・使うメッシュ・マテリアルの付け方・入切が前と同じ）。
pub fn rig_unchanged(before: &Request, after: &Request) -> bool {
    let models = |r: &Request| {
        r.models
            .iter()
            .map(|m| {
                (
                    m.id,
                    m.fbx.clone(),
                    m.guid.clone(),
                    serde_json::to_string(&m.import).ok(),
                )
            })
            .collect::<Vec<_>>()
    };
    let renderers = |r: &Request| {
        r.renderers
            .iter()
            .map(|x| (x.model, x.node.clone(), x.enabled, x.materials.clone()))
            .collect::<Vec<_>>()
    };
    let keys = |r: &Request| {
        r.materials
            .iter()
            .map(|m| m.key.clone())
            .collect::<Vec<_>>()
    };
    models(before) == models(after)
        && renderers(before) == renderers(after)
        && keys(before) == keys(after)
}
