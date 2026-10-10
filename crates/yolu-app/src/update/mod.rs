//! 自動更新のつなぎ（画面側）。確かめと検証は `yolu-update`（署名・版・大きさ・SHA-256）、通信は `http`、OS ごとの口は `launch`。
//!
//! 守ること:
//! - 聞かずに通信しない。起動時の確かめは、初回の問い（`window`）で「はい」を選んだあとだけ。手で確かめる（ヘルプのメニュー）は、押したときだけ。
//! - 公開鍵は組み込んだ物だけを信じる（`YOLUPAINTER_UPDATE_PUBLIC_KEY`）。組み込んでいないビルドは、更新の項目もウィンドウも出さない。
//! - 試験版は、設定「試験版を使う」を入れたときだけ、stable の更新情報に加えて試験版の置き場（`BETA_UPDATER_URL`）を見て、新しい方を勧める。
//!   切のときは stable だけ（stable が載せない試験版は受けない）。版を下げる更新はしないので、試験版を入れていた人が設定を切っても、
//!   次の stable が今の版より新しくなるまで何も勧めない。どちらの置き場も同じ鍵・同じ検証（`yolu_update::UpdateClient::check_channels`）。
//!   確かめの途中で設定を切ったら、試験版の結果だけを捨てて stable の結果で答える。ダウンロード中・転送が済んだあとに切ったときも試験版は受けず、
//!   落とし済みの試験版は置き場のファイルごと消す。失敗の理由は stable の取得で決める（試験版の置き場が引けないことは混ぜない）。
//! - 新しい版が見つかっても、利用者が押すまでダウンロードしない。落としたファイルは、署名つきの更新情報の SHA-256・大きさで確かめた
//!   ものだけを置き、走らせる直前にもう一度確かめる。
//! - 描いている最中は入れない。保存していない変更があるときは、保存してから入れるか聞く。
//! - 同じ実行ファイルの別の起動（別のウィンドウ）が動いているときは入れない。インストーラーは動いている exe を書き換えられず、閉じたウィンドウだけが戻らないので、
//!   始める前に断る。落としたインストーラーは残し、別のウィンドウを閉じてからもう一度押せばすぐ入る。
//! - インストールした Windows は、インストーラーを無音で走らせてアプリを閉じ、インストーラーが終わったらアプリを起こし直す。
//!   それ以外（Linux・macOS・zip で展開した Windows）は、その版のリリースのページを開くだけ（自分で入れ替えない）。macOS は試作の配布物で、
//!   新しい版を知らせるだけ（落とさず、置き場も使わない）。
//!
//! 通信・ダウンロードは別のスレッドで、取消ができる（`Link`）。状態は `UpdateState`、操作は `UpdateAction`（メニュー・ウィンドウから）。

pub mod config;
pub mod http;
pub mod launch;
pub mod window;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use egui::Vec2;
use yolu_update::{
    merge_channels, release_page, sha256, Asset, AvailableUpdate, Error, Transport, UpdateClient,
    Version, BETA_UPDATER_URL, LINUX_ARCHIVE, MACOS_ARCHIVE, UPDATER_URL, WINDOWS_ARCHIVE,
    WINDOWS_INSTALLER,
};

use crate::jobs::{JobCard, JobSpec};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;
use crate::windows::CloseJob;
pub use config::Preference;
use http::{HttpTransport, Link};

/// 更新の入れ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// インストールした Windows: インストーラーを落として無音で走らせる。
    Installer,
    /// それ以外: その版のリリースのページを開く。
    Page,
}

/// 更新の操作（メニュー・ウィンドウから）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    /// 初回の問い「起動時に更新を確かめる」への答え。
    Answer(bool),
    /// 起動時に確かめる設定を替える（ヘルプのメニュー）。今は確かめない。
    SetCheckOnStartup(bool),
    /// 試験版も勧める設定を替える（ヘルプのメニュー）。今は確かめない。切にすると、見つけていた試験版の提案を消す。
    SetBeta(bool),
    /// 手で確かめる。
    Check,
    /// 見つかった版へ更新する（Installer はダウンロードを始める。落とし済みなら準備のウィンドウを出す。Page はリリースのページを開く）。
    Install,
    /// ダウンロードを取り消す。
    Cancel,
    /// 準備のウィンドウ: 更新して再起動する。`save` なら、先に保存する。
    Run { save: bool },
    /// 準備のウィンドウ: あとで。
    Later,
}

/// 通信を作る口（試験は、偽の通信に差し替える）。
type Factory = Arc<dyn Fn(Link) -> Box<dyn Transport + Send> + Send + Sync>;
/// インストーラーを走らせる口と、ページを開く口（試験は記録するだけの物に差し替える）。
type Launcher = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;
type Opener = Arc<dyn Fn(&str) -> io::Result<()> + Send + Sync>;
/// 同じ実行ファイルの別の起動があるか（試験は、実際のプロセスの一覧を見ない物に差し替える）。
type Peers = Arc<dyn Fn() -> bool + Send + Sync>;

/// 見つかった新しい版（署名つきの更新情報で確かめ済み）。
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: Version,
    update: AvailableUpdate,
}

/// ダウンロードして検証し、置き場へ書いたインストーラー。
#[derive(Clone, Debug)]
pub struct Ready {
    pub version: Version,
    path: PathBuf,
    asset: Asset,
}

/// 失敗の種類（画面の文言を言語ごとに作る）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// 通信できない（接続・時間切れ・HTTP の失敗・上限超え）。
    Network,
    /// 取れたが、検証を通らない（署名・形式・版・大きさ・SHA-256）。
    Verify,
    /// 署名つきの更新情報は正しいが、この環境向けの配布物が載っていない。
    NoAsset,
    /// インストーラーを置き場へ書けない。
    Disk,
    Canceled,
    /// 仕事のスレッドが結果を返さずに止まった。
    Stopped,
}

enum Kind {
    Check { manual: bool },
    Download { version: Version, total: u64 },
}

/// 確かめの結果。stable と試験版の置き場の結果を別々に持ち、どちらを数えるかは受け取るときの設定で決める
/// （確かめの途中で設定を切ったとき、試験版だけを捨てて stable の結果は生かす）。
struct Checked {
    stable: Result<Option<AvailableUpdate>, Failure>,
    /// 試験版の置き場が見つけた新しい版（見なかった・引けなかった・新しくなかったときは None）。
    beta: Option<AvailableUpdate>,
}

impl Checked {
    fn failed(failure: Failure) -> Checked {
        Checked {
            stable: Err(failure),
            beta: None,
        }
    }

    /// 勧める版。`beta` が偽なら、試験版の結果は使わない。
    fn settle(self, beta: bool) -> Result<Option<AvailableUpdate>, Failure> {
        merge_channels(self.stable, self.beta.filter(|_| beta))
    }
}

enum Outcome {
    Checked(Checked),
    Downloaded(Result<Ready, Failure>),
}

struct Job {
    kind: Kind,
    link: Link,
    rx: Receiver<Outcome>,
}

/// 仕事の札（進み具合と取消）に出す物。
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub version: Version,
    pub fraction: f32,
    pub canceling: bool,
}

/// 更新の確かめとダウンロード（ダウンロードだけが札と閉じる前の確かめに出る。起動時の確かめは利用者が待っていない）。更新のウィンドウは、
/// キーの割り当てを止める。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    card: Some(|app, lang| {
        let p = app.update.progress()?;
        Some(JobCard {
            text: format!(
                "{} — {}",
                lang.pick("更新をダウンロード中", "Downloading update"),
                p.version
            ),
            fraction: Some(p.fraction),
            cancel: Some(crate::state::Action::Update(UpdateAction::Cancel)),
            canceling: p.canceling,
        })
    }),
    close: Some(|app| {
        app.update
            .progress()
            .is_some()
            .then_some(CloseJob::UpdateDownload)
    }),
    cancel: Some(|app| app.apply(crate::state::Action::Update(UpdateAction::Cancel))),
    poll_while_stopping: Some(AppState::poll_update),
    modal: Some(|app| app.update.window_open()),
    ..JobSpec::new("update", |app| app.update.is_busy())
};

pub struct UpdateState {
    key: Option<[u8; 32]>,
    current: Version,
    /// 更新情報の中で自分の配布物を指す鍵（この OS で更新できなければ None）。
    target: Option<&'static str>,
    mode: Mode,
    preference: Preference,
    /// 設定「試験版を使う」。
    beta: bool,
    config: Option<PathBuf>,
    /// 初回の問いを出している。
    asking: bool,
    pub(crate) ask_offset: Vec2,
    transport: Factory,
    staging: Option<PathBuf>,
    launcher: Launcher,
    opener: Opener,
    other_instance: Peers,
    job: Option<Job>,
    offer: Option<Offer>,
    ready: Option<Ready>,
    /// 準備のウィンドウを出したい（描いている最中は、描き終わるまで待つ）。
    ready_wanted: bool,
    ready_open: bool,
    /// 別の起動が動いていて、更新を始められなかった（準備のウィンドウが理由を出す。押し直すか「あとで」で消える）。
    pub(crate) ready_blocked: bool,
    pub(crate) ready_offset: Vec2,
    /// 保存してから入れる、の保存の結果待ち。
    after_save: bool,
    /// 更新のために終わる（終了の確かめを聞き直さない）。
    quitting: bool,
}

/// この OS・この入れ方の更新の対象（更新情報の鍵）と入れ方。
fn platform_target(installed_copy: bool) -> (Option<&'static str>, Mode) {
    target_for(std::env::consts::OS, std::env::consts::ARCH, installed_copy)
}

/// `platform_target` の中身（OS・CPU の名前は `std::env::consts` と同じ。試験は、動かしていない OS の答えも確かめる）。
///
/// macOS は、Intel でも Apple Silicon でも同じ universal の配布物（試作の zip）の鍵を使い、入れ方は `Page` だけ。アプリは落として入れ替えず、
/// 新しい版を知らせて、その版のリリースのページを開く（署名は ad-hoc だけで、アプリが自分を入れ替えるのは安全でないため）。
fn target_for(os: &str, arch: &str, installed_copy: bool) -> (Option<&'static str>, Mode) {
    match (os, arch) {
        ("windows", "x86_64") => {
            if installed_copy {
                (Some(WINDOWS_INSTALLER), Mode::Installer)
            } else {
                (Some(WINDOWS_ARCHIVE), Mode::Page)
            }
        }
        ("linux", "x86_64") => (Some(LINUX_ARCHIVE), Mode::Page),
        ("macos", "aarch64" | "x86_64") => (Some(MACOS_ARCHIVE), Mode::Page),
        _ => (None, Mode::Page),
    }
}

impl UpdateState {
    /// 組み込んだ公開鍵・この実行ファイルの入れ方から作る。公開鍵が無いビルドでは `enabled()` が偽になる。
    pub fn detect() -> UpdateState {
        let installed = std::env::current_exe()
            .ok()
            .is_some_and(|exe| launch::is_installed_copy(&exe));
        let (target, mode) = platform_target(installed);
        UpdateState {
            key: yolu_update::embedded_public_key().ok(),
            current: Version::parse(env!("CARGO_PKG_VERSION")).expect("cargo の版は SemVer"),
            target,
            mode,
            preference: Preference::Unset,
            beta: false,
            config: None,
            asking: false,
            ask_offset: Vec2::ZERO,
            transport: Arc::new(|link| Box::new(HttpTransport::new(link))),
            staging: launch::staging_dir(),
            launcher: Arc::new(launch::run_installer),
            opener: Arc::new(launch::open_page),
            other_instance: Arc::new(launch::another_instance_running),
            job: None,
            offer: None,
            ready: None,
            ready_wanted: false,
            ready_open: false,
            ready_blocked: false,
            ready_offset: Vec2::ZERO,
            after_save: false,
            quitting: false,
        }
    }

    /// 更新の項目・ウィンドウを出すビルドか（公開鍵が組み込まれ、この OS で更新できる）。
    pub fn enabled(&self) -> bool {
        self.key.is_some() && self.target.is_some()
    }

    pub fn preference(&self) -> Preference {
        self.preference
    }

    /// 設定「試験版を使う」が入っているか。
    pub fn beta(&self) -> bool {
        self.beta
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 通信かダウンロードが走っている。
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }

    pub fn ready(&self) -> Option<&Ready> {
        self.ready.as_ref()
    }

    pub fn is_asking(&self) -> bool {
        self.asking
    }

    pub fn is_ready_open(&self) -> bool {
        self.ready_open
    }

    /// 別の起動が動いているので、更新を始められなかった（準備のウィンドウが理由を出している）。
    pub fn is_blocked(&self) -> bool {
        self.ready_blocked
    }

    /// 更新のウィンドウ（初回の問い・準備）が開いている（キーの割り当てを止める）。
    pub fn window_open(&self) -> bool {
        self.asking || self.ready_open
    }

    /// 更新のために終わろうとしている。
    pub fn is_quitting(&self) -> bool {
        self.quitting
    }

    /// ダウンロードの進み具合（札に出す）。
    pub fn progress(&self) -> Option<Progress> {
        let job = self.job.as_ref()?;
        let Kind::Download { version, total } = &job.kind else {
            return None;
        };
        let read = job.link.progress.load(Ordering::Relaxed);
        Some(Progress {
            version: version.clone(),
            fraction: if *total == 0 {
                0.0
            } else {
                (read as f32 / *total as f32).clamp(0.0, 1.0)
            },
            canceling: job.link.is_canceled(),
        })
    }

    /// 設定のファイル（`update.conf`）を結び付けて、前の選択を読む。読めない選択は、まだ聞いていない扱い（試験版は切）。
    pub fn attach_config(&mut self, path: PathBuf) {
        let stored = config::load(&path).unwrap_or_default();
        self.preference = stored.check;
        self.beta = stored.beta;
        self.config = Some(path);
    }

    /// 試験用の差し替え口。
    #[doc(hidden)]
    pub fn configure_for_test(
        &mut self,
        key: Option<[u8; 32]>,
        current: &str,
        target: Option<&'static str>,
        mode: Mode,
    ) {
        self.key = key;
        self.current = Version::parse(current).expect("試験の版");
        self.target = target;
        self.mode = mode;
    }

    #[doc(hidden)]
    pub fn set_transport_for_test(&mut self, factory: Factory) {
        self.transport = factory;
    }

    #[doc(hidden)]
    pub fn set_actions_for_test(&mut self, launcher: Launcher, opener: Opener, staging: PathBuf) {
        self.launcher = launcher;
        self.opener = opener;
        self.staging = Some(staging);
    }

    /// 試験用: 別の起動があるかの答えを差し替える。
    #[doc(hidden)]
    pub fn set_other_instance_for_test(&mut self, other_instance: Peers) {
        self.other_instance = other_instance;
    }

    /// ヘルプのメニューの「更新」の項目の名前。
    pub fn install_label(&self, lang: Lang) -> Option<String> {
        let offer = self.offer.as_ref()?;
        let version = &offer.version;
        Some(match (self.progress(), self.mode) {
            (Some(_), _) => lang.pick(
                format!("YoluPainter {version} をダウンロード中"),
                format!("Downloading YoluPainter {version}"),
            ),
            (None, Mode::Installer) => lang.pick(
                format!("YoluPainter {version} に更新"),
                format!("Update to YoluPainter {version}"),
            ),
            (None, Mode::Page) => lang.pick(
                format!("YoluPainter {version} のリリースを開く"),
                format!("Open the YoluPainter {version} release"),
            ),
        })
    }
}

/// 通信の失敗か、検証の失敗かを見分けるために、通信の失敗を覚える包み。
struct Recording {
    inner: Box<dyn Transport + Send>,
    failed: Arc<AtomicBool>,
}

impl Transport for Recording {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, Error> {
        let result = self.inner.get(url, max_bytes);
        // 試験版の置き場は、引けなくても stable の答えを妨げない任意の取得（まだ 1 つも出していない間はいつも 404）。
        // 失敗の理由は、返ってくる失敗（stable かダウンロード）の理由だけで決める。
        if result.is_err() && url != BETA_UPDATER_URL {
            self.failed.store(true, Ordering::Relaxed);
        }
        result
    }
}

struct Worker {
    factory: Factory,
    link: Link,
    key: [u8; 32],
    failed: Arc<AtomicBool>,
}

impl Worker {
    fn client(&self) -> Result<UpdateClient<Recording>, Failure> {
        let transport = Recording {
            inner: (self.factory)(self.link.clone()),
            failed: self.failed.clone(),
        };
        UpdateClient::with_public_key(transport, self.key).map_err(|_| Failure::Verify)
    }

    fn failure(&self, error: &Error) -> Failure {
        if self.link.is_canceled() {
            Failure::Canceled
        } else if self.failed.load(Ordering::Relaxed) {
            Failure::Network
        } else if error.is_missing_target() {
            Failure::NoAsset
        } else {
            Failure::Verify
        }
    }

    /// `beta` なら、stable に加えて試験版の置き場も見る（どちらを勧めるかは、受け取るときに決める）。
    fn check(&self, current: &Version, target: &str, beta: bool) -> Checked {
        let client = match self.client() {
            Ok(client) => client,
            Err(failure) => return Checked::failed(failure),
        };
        let beta_url = beta.then_some(BETA_UPDATER_URL);
        let (stable, beta) = client.check_each(UPDATER_URL, beta_url, current, target);
        Checked {
            stable: stable.map_err(|e| self.failure(&e)),
            beta,
        }
    }

    fn download(&self, update: AvailableUpdate, staging: Option<&Path>) -> Result<Ready, Failure> {
        let verified = self
            .client()?
            .download(update.approve_download())
            .map_err(|e| self.failure(&e))?;
        let dir = staging.ok_or(Failure::Disk)?;
        let path =
            stage(dir, &verified.asset().name, verified.bytes()).map_err(|_| Failure::Disk)?;
        Ok(Ready {
            version: verified.version().clone(),
            path,
            asset: verified.asset().clone(),
        })
    }
}

/// 検証済みの中身を置き場へ書く。前の版のインストーラーと書きかけは先に消し、一時ファイルに書いて最後に 1 回の rename で置く
/// （書きかけのファイルを、走らせられるインストーラーとして見せない）。
fn stage(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    use std::io::Write;
    // 名前は yolu-update が検証した配布物名だが、置く場所の外へ出ないことをここでも守る。
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe file name",
        ));
    }
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let old = entry.file_name().to_string_lossy().into_owned();
        // 書きかけは今の一時ファイル（`.{名前}.{pid}-{番号}.pending~`）と、前の版の `.part`
        let target = yolu_io::atomic::leftover_target(&old).unwrap_or(&old);
        if target.starts_with("yolupainter-")
            && (target.ends_with(".exe") || target.ends_with(".part"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let path = dir.join(name);
    yolu_io::atomic::replace_with(&path, &Default::default(), |file| file.write_all(bytes))?;
    Ok(path)
}

/// 置き場のファイル名（`yolupainter-<版>-<対象の鍵>.exe`、書きかけは一時ファイル `.{名前}.{pid}-{番号}.pending~` か、前の版の
/// 末尾の `.part`）から版を取る。
fn staged_version(file: &str) -> Option<Version> {
    let file = yolu_io::atomic::leftover_target(file).unwrap_or(file);
    let rest = file.strip_prefix("yolupainter-")?;
    let rest = rest.strip_suffix(".part").unwrap_or(rest);
    let rest = rest.strip_suffix(".exe")?;
    let version = rest.strip_suffix(&format!("-{WINDOWS_INSTALLER}"))?;
    Version::parse(version).ok()
}

/// 起動時の片付け: 走らせ終えた（または使われなかった）インストーラーと書きかけのうち、今の版以下のものを消す。
/// 新しい版のものは、並行して動いている別のアプリが落としている最中かもしれないので触らない（次のダウンロードが置き換える）。
/// 走っている最中のインストーラーは消せない（Windows）。そのときは残し、次の起動で消す。失敗は無視する。
fn clear_staged(dir: &Path, current: &Version) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if staged_version(&name).is_some_and(|version| version <= *current) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 置いたファイルが、署名つきの更新情報の大きさ・SHA-256 のままか（走らせる直前に確かめる）。
fn staged_file_is_intact(ready: &Ready) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(&ready.path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file
        .take(ready.asset.size + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return false;
    }
    bytes.len() as u64 == ready.asset.size && sha256(&bytes) == ready.asset.sha256
}

/// 別の起動が動いていて、更新を始められない理由（状態の知らせと準備のウィンドウが同じ文を出す）。
pub(crate) fn blocked_text(lang: Lang) -> &'static str {
    lang.pick(
        "ほかの YoluPainter が開いています",
        "Another YoluPainter is running",
    )
}

/// 失敗の知らせの種類（取り消しは済んだ知らせ、ほかは失敗）。
fn failure_kind(failure: &Failure) -> crate::notice::Kind {
    match failure {
        Failure::Canceled => crate::notice::Kind::Info,
        _ => crate::notice::Kind::Error,
    }
}

impl AppState {
    /// 起動時に 1 度（実際のウィンドウだけ。試験は呼ばない）: 初めてなら問いを出し、「確かめる」を選んでいれば確かめる。
    pub fn update_startup(&mut self) {
        if !self.update.enabled() {
            return;
        }
        if let Some(dir) = &self.update.staging {
            clear_staged(dir, &self.update.current);
        }
        match self.update.preference {
            Preference::Unset => self.update.asking = true,
            Preference::On => self.update_start_check(false),
            Preference::Off => {}
        }
    }

    pub(crate) fn update_apply(&mut self, action: UpdateAction) {
        if !self.update.enabled() {
            return;
        }
        match action {
            UpdateAction::Answer(on) => {
                self.update.asking = false;
                self.update_set_preference(on);
                if on {
                    self.update_start_check(false);
                }
            }
            UpdateAction::SetCheckOnStartup(on) => self.update_set_preference(on),
            UpdateAction::SetBeta(on) => self.update_set_beta(on),
            UpdateAction::Check => self.update_start_check(true),
            UpdateAction::Install => self.update_install(),
            UpdateAction::Cancel => {
                if let Some(job) = &self.update.job {
                    job.link.cancel.store(true, Ordering::Relaxed);
                }
            }
            UpdateAction::Run { save } => self.update_run(save),
            UpdateAction::Later => {
                self.update.ready_open = false;
                self.update.ready_wanted = false;
                self.update.ready_blocked = false;
            }
        }
    }

    fn update_set_preference(&mut self, on: bool) {
        self.update.preference = if on { Preference::On } else { Preference::Off };
        self.update_save_config();
    }

    /// 試験版の設定を替える。切にしたときは、見つけていた試験版の提案・落とし済みや落としている最中の試験版を残さない
    /// （切にしたら stable だけを勧める）。入にしても、その場では通信しない（次の確かめから効く）。
    fn update_set_beta(&mut self, on: bool) {
        self.update.beta = on;
        if !on {
            let pre = |version: &Version| !version.pre.is_empty();
            if self.update.offer.as_ref().is_some_and(|o| pre(&o.version)) {
                self.update.offer = None;
            }
            if let Some(ready) = self.update.ready.take_if(|r| pre(&r.version)) {
                // 置き場のインストーラーも消す（今の版より新しいファイルは、起動時の片付けでも消えない）。
                let _ = std::fs::remove_file(&ready.path);
                self.update.ready_wanted = false;
                self.update.ready_open = false;
                self.update.ready_blocked = false;
            }
            if let Some(job) = &self.update.job {
                if matches!(&job.kind, Kind::Download { version, .. } if pre(version)) {
                    job.link.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
        self.update_save_config();
    }

    /// 更新の設定（起動時の確かめ・試験版）を `update.conf` へ保存する。保存できなくても、この回の選択は効く。
    fn update_save_config(&mut self) {
        let stored = config::Stored {
            check: self.update.preference,
            beta: self.update.beta,
        };
        if let Some(path) = &self.update.config {
            if config::save(path, &stored).is_err() {
                self.fail(
                    Source::Update,
                    self.lang.pick(
                        "更新の設定を保存できません。",
                        "Cannot save the update setting.",
                    ),
                );
            }
        }
    }

    fn update_start_check(&mut self, manual: bool) {
        let lang = self.lang;
        if self.update.job.is_some() {
            if manual {
                self.refuse(
                    Source::Update,
                    lang.pick("更新を処理中です。", "An update job is running."),
                );
            }
            return;
        }
        let (Some(key), Some(target)) = (self.update.key, self.update.target) else {
            return;
        };
        let link = Link::default();
        let worker = Worker {
            factory: self.update.transport.clone(),
            link: link.clone(),
            key,
            failed: Arc::new(AtomicBool::new(false)),
        };
        let current = self.update.current.clone();
        let beta = self.update.beta;
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("yolu-update-check".into())
            .spawn(move || {
                let _ = tx.send(Outcome::Checked(worker.check(&current, target, beta)));
            });
        if let Err(e) = spawned {
            self.fail(
                Source::Update,
                crate::lang::update_start_failure(lang, "check", &e),
            );
            return;
        }
        if manual {
            self.info(
                Source::Update,
                lang.pick("更新を確かめています…", "Checking for updates…"),
            );
        }
        self.update.job = Some(Job {
            kind: Kind::Check { manual },
            link,
            rx,
        });
    }

    fn update_install(&mut self) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Update, crate::lang::refusals::during_stroke(lang));
            return;
        }
        let Some(offer) = self.update.offer.clone() else {
            return;
        };
        match self.update.mode {
            Mode::Page => {
                let url = release_page(&offer.version);
                match (self.update.opener)(&url) {
                    Ok(()) => self.info(
                        Source::Update,
                        lang.pick(
                            format!(
                                "YoluPainter {} のリリースのページを開きました。",
                                offer.version
                            ),
                            format!("Opened the YoluPainter {} release page.", offer.version),
                        ),
                    ),
                    Err(_) => self.fail(
                        Source::Update,
                        lang.pick(
                            format!("ページ{}を開けません。", lang.quote(&url)),
                            format!("Cannot open the page {}.", lang.quote(&url)),
                        ),
                    ),
                }
            }
            Mode::Installer => {
                // 落とし済みで、まだ変わっていなければ、落とし直さずに準備のウィンドウへ。
                if let Some(ready) = &self.update.ready {
                    if ready.version == offer.version && staged_file_is_intact(ready) {
                        self.update.ready_wanted = true;
                        return;
                    }
                }
                self.update.ready = None;
                self.update.ready_blocked = false;
                if self.update.job.is_some() {
                    return;
                }
                let (Some(key), Some(_)) = (self.update.key, self.update.target) else {
                    return;
                };
                let link = Link::default();
                let worker = Worker {
                    factory: self.update.transport.clone(),
                    link: link.clone(),
                    key,
                    failed: Arc::new(AtomicBool::new(false)),
                };
                let staging = self.update.staging.clone();
                let total = offer.update.asset().size;
                let version = offer.version.clone();
                let (tx, rx) = channel();
                let spawned = std::thread::Builder::new()
                    .name("yolu-update-download".into())
                    .spawn(move || {
                        let _ = tx.send(Outcome::Downloaded(
                            worker.download(offer.update, staging.as_deref()),
                        ));
                    });
                if let Err(e) = spawned {
                    self.fail(
                        Source::Update,
                        crate::lang::update_start_failure(lang, "download", &e),
                    );
                    return;
                }
                self.info(
                    Source::Update,
                    lang.pick(
                        format!("YoluPainter {version} をダウンロード中…"),
                        format!("Downloading YoluPainter {version}…"),
                    ),
                );
                self.update.job = Some(Job {
                    kind: Kind::Download { version, total },
                    link,
                    rx,
                });
            }
        }
    }

    /// 毎フレームの初め: 終わった通信・ダウンロードを受ける。準備のウィンドウは、描いている最中は開かない。
    pub fn poll_update(&mut self) {
        let lang = self.lang;
        if let Some(job) = &self.update.job {
            let outcome = match job.rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(match job.kind {
                    Kind::Check { .. } => Outcome::Checked(Checked::failed(Failure::Stopped)),
                    Kind::Download { .. } => Outcome::Downloaded(Err(Failure::Stopped)),
                }),
            };
            if let Some(outcome) = outcome {
                let job = self.update.job.take().expect("上で見た");
                let manual = matches!(job.kind, Kind::Check { manual: true });
                match outcome {
                    // 試験版も見る確かめの途中で設定を切ったときは、試験版の結果だけを捨てて stable の結果で答える（切にしたら stable だけ）。
                    Outcome::Checked(checked) => match checked.settle(self.update.beta) {
                        Ok(Some(update)) => {
                            let version = update.version().clone();
                            if manual {
                                self.info(
                                    Source::Update,
                                    lang.pick(
                                        format!("YoluPainter {version} があります。"),
                                        format!("YoluPainter {version} is available."),
                                    ),
                                );
                            }
                            self.update.offer = Some(Offer { version, update });
                        }
                        Ok(None) => {
                            if manual {
                                self.info(
                                    Source::Update,
                                    lang.pick(
                                        "YoluPainter は最新です。",
                                        "YoluPainter is up to date.",
                                    ),
                                );
                            }
                        }
                        // 起動時の確かめの失敗は、利用者が頼んだことではないので知らせない。
                        Err(failure) => {
                            if manual {
                                self.notify(
                                    failure_kind(&failure),
                                    Source::Update,
                                    crate::lang::update_failure(lang, "check", failure),
                                );
                            }
                        }
                    },
                    Outcome::Downloaded(Ok(ready)) => {
                        // 転送が済んだあとの確かめ・書き込みの間に、取り消された・試験版の設定が切になったものは受けない
                        // （置いたファイルも消す。取消は転送の途中でしか見ないので、ここで見る）。
                        if job.link.is_canceled()
                            || (!self.update.beta && !ready.version.pre.is_empty())
                        {
                            let _ = std::fs::remove_file(&ready.path);
                            self.info(
                                Source::Update,
                                crate::lang::update_failure(lang, "download", Failure::Canceled),
                            );
                        } else {
                            self.update.ready = Some(ready);
                            self.update.ready_wanted = true;
                        }
                    }
                    Outcome::Downloaded(Err(failure)) => {
                        self.notify(
                            failure_kind(&failure),
                            Source::Update,
                            crate::lang::update_failure(lang, "download", failure),
                        );
                    }
                }
            }
        }
        if self.update.ready_wanted && self.update.ready.is_some() && !self.is_stroking() {
            self.update.ready_wanted = false;
            self.update.ready_open = true;
        }
    }

    fn update_run(&mut self, save: bool) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Update, crate::lang::refusals::during_stroke(lang));
            return;
        }
        if self.update.ready.is_none() {
            return;
        }
        // 保存の途中は入れ替えない（保存の頼みは「変更あり」を下ろすので、いま入れ替えると、保存が失敗して変更が残っても、更新のために
        // 終わる流れは確認なしで閉じてしまう）。更新のウィンドウと落としたインストーラーは残し、保存が終わってからやり直せる
        if self.is_saving() {
            self.refuse(
                Source::Update,
                lang.with_reason(
                    lang.pick("更新できません", "Cannot update"),
                    crate::lang::refusals::saving(lang),
                ),
            );
            return;
        }
        // 保存する前に断る（更新が始まらないのに、保存のウィンドウを出さない）。落としたインストーラーは残す。
        if self.update_blocked_by_another_instance() {
            return;
        }
        if save && self.modified {
            // 保存の結果（成功の文・失敗の理由）は保存の側が message に書く。空にしておけば、保存先のウィンドウを取り消した
            // （message が空のまま）のと、保存が失敗した（理由が書かれている）のを、`update_finish_save` が見分けられる。
            self.clear_message();
            self.apply(crate::state::Action::SaveProject);
            self.update.after_save = true;
            self.update_finish_save();
        } else {
            self.update_launch();
        }
    }

    /// 保存の結果を受けて入れる（フレームの終わりにも呼ぶ。保存先を選ぶウィンドウが開いている間は待つ）。保存されていなければ入れない。
    /// 保存が失敗していれば、その理由（保存の側が書いた message）を残す。短い文を出すのは、保存先のウィンドウを取り消したときだけ。
    pub fn update_finish_save(&mut self) {
        // 保存の仕事が動いているあいだも待つ（裏のスレッドの保存が終わってから、その結果を見て入れる）
        if !self.update.after_save || self.dialog_request.is_some() || self.is_saving() {
            return;
        }
        self.update.after_save = false;
        if self.modified {
            if self.message.is_empty() {
                self.refuse(
                    Source::Update,
                    self.lang.pick(
                        "保存しなかったので、更新しません。",
                        "Not updating because the project was not saved.",
                    ),
                );
            }
            return;
        }
        self.update_launch();
    }

    /// 同じ実行ファイルの別の起動が動いていれば、理由を出して true（落としたインストーラーは残し、準備のウィンドウも開いたまま。
    /// 別のウィンドウを閉じてからもう一度押せばすぐ入る）。動いていなければ、理由の表示を消して false。
    fn update_blocked_by_another_instance(&mut self) -> bool {
        let blocked = (self.update.other_instance)();
        self.update.ready_blocked = blocked;
        if blocked {
            self.refuse(Source::Update, blocked_text(self.lang));
        }
        blocked
    }

    fn update_launch(&mut self) {
        let lang = self.lang;
        let Some(ready) = self.update.ready.clone() else {
            return;
        };
        // 入れ替えは、保存が終わって「変更あり」の印が確かなときだけ（ここへ来る道はどれも保存の途中を断るが、走らせる直前にも確かめる）
        if self.is_saving() {
            return;
        }
        // 保存のウィンドウを待つ間に別のウィンドウが開いたかもしれないので、走らせる直前にもう一度確かめる。
        if self.update_blocked_by_another_instance() {
            return;
        }
        if !staged_file_is_intact(&ready) {
            let _ = std::fs::remove_file(&ready.path);
            self.update.ready = None;
            self.update.ready_open = false;
            self.fail(
                Source::Update,
                lang.pick(
                    "ダウンロードしたファイルが変わっています。更新しません。",
                    "The downloaded file has changed. Not updating.",
                ),
            );
            return;
        }
        match (self.update.launcher)(&ready.path) {
            Ok(()) => {
                self.info(
                    Source::Update,
                    lang.pick(
                        format!("YoluPainter {} に更新します。", ready.version),
                        format!("Updating to YoluPainter {}.", ready.version),
                    ),
                );
                self.update.ready_open = false;
                self.update.quitting = true;
                self.quit = true;
            }
            Err(_) => {
                self.fail(
                    Source::Update,
                    lang.pick(
                        "インストーラーを起動できません。",
                        "Cannot start the installer.",
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/update-tests")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn staging_replaces_old_installers_and_never_leaves_a_partial_file() {
        let dir = scratch("stage");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("yolupainter-0.1.0-x86_64-pc-windows-msvc-setup.exe"),
            b"old",
        )
        .unwrap();
        std::fs::write(
            dir.join("yolupainter-0.1.0-x86_64-pc-windows-msvc-setup.exe.part"),
            b"half",
        )
        .unwrap();
        std::fs::write(dir.join("keep.txt"), b"mine").unwrap();
        let name = "yolupainter-0.2.0-x86_64-pc-windows-msvc-setup.exe";
        let path = stage(&dir, name, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["keep.txt", name]);
        // 同じ名前でも置き換えられる。置き場の外へ出る名前は断る。
        assert_eq!(
            std::fs::read(stage(&dir, name, b"newer").unwrap()).unwrap(),
            b"newer"
        );
        for bad in ["", "../x.exe", "a/b.exe", r"a\b.exe", ".hidden"] {
            assert!(stage(&dir, bad, b"x").is_err(), "{bad:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_clears_installers_that_are_not_newer_than_this_version() {
        let dir = scratch("clear");
        std::fs::create_dir_all(&dir).unwrap();
        let setup = |version: &str| format!("yolupainter-{version}-{WINDOWS_INSTALLER}.exe");
        let present = |names: &[&str]| {
            for name in names {
                std::fs::write(dir.join(name), b"x").unwrap();
            }
        };
        let left = || {
            let mut names: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let (older, same, part, newer, pre) = (
            setup("0.1.0"),
            setup("0.2.0"),
            format!("{}.part", setup("0.2.0")),
            setup("0.3.0"),
            setup("0.2.1-rc.1"),
        );
        // 利用者のファイル・版の読めない名前・別の種類（zip）は、今の版以下の名前に見えても触らない
        let others = [
            "keep.txt",
            "yolupainter-notes.exe",
            "yolupainter-0.1.0-x86_64-pc-windows-msvc.zip",
            "yolupainter-0.1.0-x86_64-unknown-linux-gnu.exe",
        ];
        present(&[&older, &same, &part, &newer, &pre]);
        present(&others);
        clear_staged(&dir, &Version::new(0, 2, 0));
        let mut expect: Vec<String> = [&newer, &pre].iter().map(|s| s.to_string()).collect();
        expect.extend(others.iter().map(|s| s.to_string()));
        expect.sort();
        assert_eq!(left(), expect);
        // プレリリースの今の版: 同じ版・それより前だけ消える（正式版の 0.2.1 は新しい）
        clear_staged(&dir, &Version::parse("0.2.1-rc.1").unwrap());
        assert!(!dir.join(&pre).exists() && dir.join(&newer).exists());
        // 置き場が無くても何も起きない
        clear_staged(&dir.join("missing"), &Version::new(9, 0, 0));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staged_names_give_their_version() {
        let name = |v: &str| format!("yolupainter-{v}-{WINDOWS_INSTALLER}.exe");
        assert_eq!(staged_version(&name("1.2.3")), Some(Version::new(1, 2, 3)));
        assert_eq!(
            staged_version(&format!("{}.part", name("1.2.3-rc.1"))),
            Some(Version::parse("1.2.3-rc.1").unwrap())
        );
        for bad in [
            "",
            "setup.exe",
            "yolupainter-1.2.3.exe",
            "yolupainter-x-x86_64-pc-windows-msvc-setup.exe",
            "yolupainter-1.2.3-x86_64-pc-windows-msvc-setup.zip",
            "yolupainter-1.2.3-x86_64-pc-windows-msvc.zip",
        ] {
            assert_eq!(staged_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_changed_staged_file_is_not_intact() {
        let dir = scratch("intact");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("setup.exe");
        std::fs::write(&path, b"installer").unwrap();
        let ready = |bytes: &[u8]| Ready {
            version: Version::new(1, 0, 0),
            path: path.clone(),
            asset: Asset {
                target: WINDOWS_INSTALLER.into(),
                name: "setup.exe".into(),
                url: String::new(),
                sha256: sha256(bytes),
                size: bytes.len() as u64,
            },
        };
        assert!(staged_file_is_intact(&ready(b"installer")));
        // 同じ長さで中身だけ違う・長さが違う・消えた
        assert!(!staged_file_is_intact(&ready(b"INSTALLER")));
        assert!(!staged_file_is_intact(&ready(b"installers")));
        std::fs::write(&path, b"installer+more").unwrap();
        assert!(!staged_file_is_intact(&ready(b"installer")));
        std::fs::remove_file(&path).unwrap();
        assert!(!staged_file_is_intact(&ready(b"installer")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_targets_follow_the_os_and_how_it_was_installed() {
        let (zip, mode) = platform_target(false);
        let (setup, setup_mode) = platform_target(true);
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!((zip, mode), (Some(WINDOWS_ARCHIVE), Mode::Page));
            assert_eq!(
                (setup, setup_mode),
                (Some(WINDOWS_INSTALLER), Mode::Installer)
            );
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            // Linux は、インストールの有無にかかわらず、ページを開くだけ。
            assert_eq!((zip, mode), (Some(LINUX_ARCHIVE), Mode::Page));
            assert_eq!((setup, setup_mode), (Some(LINUX_ARCHIVE), Mode::Page));
        } else if cfg!(target_os = "macos") {
            assert_eq!((zip, mode), (Some(MACOS_ARCHIVE), Mode::Page));
            assert_eq!((setup, setup_mode), (Some(MACOS_ARCHIVE), Mode::Page));
        }
    }

    #[test]
    fn macos_only_announces_the_new_version_and_never_installs_itself() {
        // Intel でも Apple Silicon でも、入れ方にかかわらず、同じ universal の配布物の鍵でページを開くだけ。
        for arch in ["aarch64", "x86_64"] {
            for installed in [false, true] {
                assert_eq!(
                    target_for("macos", arch, installed),
                    (Some(MACOS_ARCHIVE), Mode::Page),
                    "{arch} {installed}"
                );
            }
        }
        // 入れ替える（インストーラーを走らせる）のは、インストーラーで入れた Windows だけ。
        for os in ["windows", "linux", "macos", "freebsd"] {
            for arch in ["x86_64", "aarch64", "x86"] {
                for installed in [false, true] {
                    let (_, mode) = target_for(os, arch, installed);
                    assert_eq!(
                        mode == Mode::Installer,
                        (os, arch, installed) == ("windows", "x86_64", true),
                        "{os} {arch} {installed}"
                    );
                }
            }
        }
        // 対象の配布物を持たない OS・CPU は、更新の項目を出さない（今までどおり）。
        assert_eq!(target_for("macos", "powerpc", false).0, None);
        assert_eq!(target_for("windows", "aarch64", false).0, None);
    }

    #[test]
    fn failure_texts_name_what_failed_in_both_languages() {
        for lang in Lang::ALL {
            for failure in [
                Failure::Network,
                Failure::Verify,
                Failure::NoAsset,
                Failure::Disk,
                Failure::Stopped,
            ] {
                for what in ["check", "download"] {
                    let text = crate::lang::update_failure(lang, what, failure);
                    // 何が（なぜ）の 1 つの文（コロンでつながない）
                    assert!(
                        !text.contains(": ") && text.ends_with(lang.pick("）。", ").")),
                        "{text}"
                    );
                }
            }
        }
        assert!(
            crate::lang::update_failure(Lang::En, "check", Failure::Network)
                .starts_with("Cannot check for updates")
        );
        assert!(
            crate::lang::update_failure(Lang::Ja, "download", Failure::Verify)
                .starts_with("更新をダウンロードできません")
        );
        // 更新情報に対象の配布物が無いのは、検証の失敗とは別の理由で言う
        assert_eq!(
            crate::lang::update_failure(Lang::Ja, "check", Failure::NoAsset),
            "更新を確かめられません（この環境向けの配布物がありません）。"
        );
        assert_eq!(
            crate::lang::update_failure(Lang::En, "check", Failure::NoAsset),
            "Cannot check for updates (no download for this system)."
        );
        assert_eq!(
            crate::lang::update_failure(Lang::En, "download", Failure::Canceled),
            "Download canceled."
        );
    }
}
