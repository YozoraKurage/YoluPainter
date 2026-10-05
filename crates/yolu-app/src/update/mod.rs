//! 自動更新のつなぎ（画面側）。確かめと検証は `yolu-update`（署名・版・大きさ・SHA-256）、通信は `http`、OS ごとの口は `launch`。
//!
//! 守ること:
//! - 聞かずに通信しない。起動時の確かめは、初回の問い（`window`）で「はい」を選んだあとだけ。手で確かめる（ヘルプのメニュー）は、押したときだけ。
//! - 公開鍵は組み込んだ物だけを信じる（`YOLUPAINTER_UPDATE_PUBLIC_KEY`）。組み込んでいないビルドは、更新の項目も窓も出さない。
//! - 新しい版が見つかっても、利用者が押すまでダウンロードしない。落としたファイルは、署名つきの更新情報の SHA-256・大きさで確かめた
//!   ものだけを置き、走らせる直前にもう一度確かめる。
//! - 描いている最中は入れない。保存していない変更があるときは、保存してから入れるか聞く。
//! - インストールした Windows は、インストーラーを無音で走らせてアプリを閉じ、インストーラーが終わったらアプリを起こし直す。
//!   それ以外（Linux・zip で展開した Windows）は、その版のリリースのページを開くだけ（自分で入れ替えない）。
//!
//! 通信・ダウンロードは別のスレッドで、取消ができる（`Link`）。状態は `UpdateState`、操作は `UpdateAction`（メニュー・窓から）。

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
    release_page, sha256, Asset, AvailableUpdate, Error, Transport, UpdateClient, Version,
    LINUX_ARCHIVE, UPDATER_URL, WINDOWS_ARCHIVE, WINDOWS_INSTALLER,
};

use crate::lang::Lang;
use crate::state::AppState;
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

/// 更新の操作（メニュー・窓から）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    /// 初回の問い「起動時に更新を確かめる」への答え。
    Answer(bool),
    /// 起動時に確かめる設定を替える（ヘルプのメニュー）。今は確かめない。
    SetCheckOnStartup(bool),
    /// 手で確かめる。
    Check,
    /// 見つかった版へ更新する（Installer はダウンロードを始める。落とし済みなら準備の窓を出す。Page はリリースのページを開く）。
    Install,
    /// ダウンロードを取り消す。
    Cancel,
    /// 準備の窓: 更新して再起動する。`save` なら、先に保存する。
    Run { save: bool },
    /// 準備の窓: あとで。
    Later,
}

/// 通信を作る口（試験は、偽の通信に差し替える）。
type Factory = Arc<dyn Fn(Link) -> Box<dyn Transport + Send> + Send + Sync>;
/// インストーラーを走らせる口と、ページを開く口（試験は記録するだけの物に差し替える）。
type Launcher = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;
type Opener = Arc<dyn Fn(&str) -> io::Result<()> + Send + Sync>;

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

enum Outcome {
    Checked(Result<Option<AvailableUpdate>, Failure>),
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

pub struct UpdateState {
    key: Option<[u8; 32]>,
    current: Version,
    /// 更新情報の中で自分の配布物を指す鍵（この OS で更新できなければ None）。
    target: Option<&'static str>,
    mode: Mode,
    preference: Preference,
    config: Option<PathBuf>,
    /// 初回の問いを出している。
    asking: bool,
    pub(crate) ask_offset: Vec2,
    transport: Factory,
    staging: Option<PathBuf>,
    launcher: Launcher,
    opener: Opener,
    job: Option<Job>,
    offer: Option<Offer>,
    ready: Option<Ready>,
    /// 準備の窓を出したい（描いている最中は、描き終わるまで待つ）。
    ready_wanted: bool,
    ready_open: bool,
    pub(crate) ready_offset: Vec2,
    /// 保存してから入れる、の保存の結果待ち。
    after_save: bool,
    /// 更新のために終わる（終了の確かめを聞き直さない）。
    quitting: bool,
}

/// この OS・この入れ方の更新の対象（更新情報の鍵）と入れ方。
fn platform_target(installed_copy: bool) -> (Option<&'static str>, Mode) {
    if cfg!(all(windows, target_arch = "x86_64")) {
        if installed_copy {
            (Some(WINDOWS_INSTALLER), Mode::Installer)
        } else {
            (Some(WINDOWS_ARCHIVE), Mode::Page)
        }
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (Some(LINUX_ARCHIVE), Mode::Page)
    } else {
        (None, Mode::Page)
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
            config: None,
            asking: false,
            ask_offset: Vec2::ZERO,
            transport: Arc::new(|link| Box::new(HttpTransport::new(link))),
            staging: launch::staging_dir(),
            launcher: Arc::new(launch::run_installer),
            opener: Arc::new(launch::open_page),
            job: None,
            offer: None,
            ready: None,
            ready_wanted: false,
            ready_open: false,
            ready_offset: Vec2::ZERO,
            after_save: false,
            quitting: false,
        }
    }

    /// 更新の項目・窓を出すビルドか（公開鍵が組み込まれ、この OS で更新できる）。
    pub fn enabled(&self) -> bool {
        self.key.is_some() && self.target.is_some()
    }

    pub fn preference(&self) -> Preference {
        self.preference
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

    /// 更新の窓（初回の問い・準備）が開いている（キーの割り当てを止める）。
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

    /// 設定のファイル（`update.conf`）を結び付けて、前の選択を読む。読めない選択は、まだ聞いていない扱い。
    pub fn attach_config(&mut self, path: PathBuf) {
        self.preference = config::load(&path).unwrap_or(Preference::Unset);
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

    /// 試験用: 見つかった版を直接入れる（通信を通さずに窓・メニューを見る）。
    #[doc(hidden)]
    pub fn set_offer_for_test(&mut self, update: AvailableUpdate) {
        self.offer = Some(Offer {
            version: update.version().clone(),
            update,
        });
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
        if result.is_err() {
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

    fn failure(&self) -> Failure {
        if self.link.is_canceled() {
            Failure::Canceled
        } else if self.failed.load(Ordering::Relaxed) {
            Failure::Network
        } else {
            Failure::Verify
        }
    }

    fn check(
        &self,
        current: &Version,
        target: &str,
        allow_prerelease: bool,
    ) -> Result<Option<AvailableUpdate>, Failure> {
        self.client()?
            .check(UPDATER_URL, current, target, allow_prerelease)
            .map_err(|_| self.failure())
    }

    fn download(&self, update: AvailableUpdate, staging: Option<&Path>) -> Result<Ready, Failure> {
        let verified = self
            .client()?
            .download(update.approve_download())
            .map_err(|_| self.failure())?;
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
        if old.starts_with("yolupainter-") && (old.ends_with(".exe") || old.ends_with(".part")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let path = dir.join(name);
    let part = dir.join(format!("{name}.part"));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&part, &path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written.map(|()| path)
}

/// 置き場のファイル名（`yolupainter-<版>-<対象の鍵>.exe`、書きかけは末尾に `.part`）から版を取る。
fn staged_version(file: &str) -> Option<Version> {
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

fn failure_text(lang: Lang, what: &'static str, failure: Failure) -> String {
    // what: "check" か "download"
    let head = match what {
        "check" => lang.pick("更新を確かめられません", "Cannot check for updates"),
        _ => lang.pick("更新をダウンロードできません", "Cannot download the update"),
    };
    let reason = match failure {
        Failure::Network => lang.pick("通信できません", "connection failed"),
        Failure::Verify => lang.pick("検証を通りません", "verification failed"),
        Failure::Disk => lang.pick("ファイルを保存できません", "cannot save the file"),
        Failure::Stopped => lang.pick("処理が止まりました", "the job stopped"),
        Failure::Canceled => {
            return lang
                .pick("ダウンロードを取り消しました。", "Download canceled.")
                .into()
        }
    };
    format!("{head}: {reason}")
}

impl AppState {
    /// 起動時に 1 度（実際の窓だけ。試験は呼ばない）: 初めてなら問いを出し、「確かめる」を選んでいれば確かめる。
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
            }
        }
    }

    fn update_set_preference(&mut self, on: bool) {
        self.update.preference = if on { Preference::On } else { Preference::Off };
        if let Some(path) = &self.update.config {
            if config::save(path, on).is_err() {
                self.message = self
                    .lang
                    .pick(
                        "更新の設定を保存できません。",
                        "Cannot save the update setting.",
                    )
                    .into();
            }
        }
    }

    fn update_start_check(&mut self, manual: bool) {
        let lang = self.lang;
        if self.update.job.is_some() {
            if manual {
                self.message = lang
                    .pick("更新を処理中です。", "An update job is running.")
                    .into();
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
        let allow_prerelease = !current.pre.is_empty();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("yolu-update-check".into())
            .spawn(move || {
                let _ = tx.send(Outcome::Checked(worker.check(
                    &current,
                    target,
                    allow_prerelease,
                )));
            });
        if let Err(e) = spawned {
            self.message = e.to_string();
            return;
        }
        if manual {
            self.message = lang
                .pick("更新を確かめています…", "Checking for updates…")
                .into();
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
            self.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        let Some(offer) = self.update.offer.clone() else {
            return;
        };
        match self.update.mode {
            Mode::Page => {
                let url = release_page(&offer.version);
                self.message = match (self.update.opener)(&url) {
                    Ok(()) => lang.pick(
                        format!(
                            "YoluPainter {} のリリースのページを開きました。",
                            offer.version
                        ),
                        format!("Opened the YoluPainter {} release page.", offer.version),
                    ),
                    Err(_) => lang.pick(
                        format!("ページを開けません: {url}"),
                        format!("Cannot open the page: {url}"),
                    ),
                };
            }
            Mode::Installer => {
                // 落とし済みで、まだ変わっていなければ、落とし直さずに準備の窓へ。
                if let Some(ready) = &self.update.ready {
                    if ready.version == offer.version && staged_file_is_intact(ready) {
                        self.update.ready_wanted = true;
                        return;
                    }
                }
                self.update.ready = None;
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
                    self.message = e.to_string();
                    return;
                }
                self.message = lang.pick(
                    format!("YoluPainter {version} をダウンロード中…"),
                    format!("Downloading YoluPainter {version}…"),
                );
                self.update.job = Some(Job {
                    kind: Kind::Download { version, total },
                    link,
                    rx,
                });
            }
        }
    }

    /// 毎フレームの初め: 終わった通信・ダウンロードを受ける。準備の窓は、描いている最中は開かない。
    pub fn poll_update(&mut self) {
        let lang = self.lang;
        if let Some(job) = &self.update.job {
            let outcome = match job.rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(match job.kind {
                    Kind::Check { .. } => Outcome::Checked(Err(Failure::Stopped)),
                    Kind::Download { .. } => Outcome::Downloaded(Err(Failure::Stopped)),
                }),
            };
            if let Some(outcome) = outcome {
                let job = self.update.job.take().expect("上で見た");
                let manual = matches!(job.kind, Kind::Check { manual: true });
                match outcome {
                    Outcome::Checked(Ok(Some(update))) => {
                        let version = update.version().clone();
                        if manual {
                            self.message = lang.pick(
                                format!("YoluPainter {version} があります。"),
                                format!("YoluPainter {version} is available."),
                            );
                        }
                        self.update.offer = Some(Offer { version, update });
                    }
                    Outcome::Checked(Ok(None)) => {
                        if manual {
                            self.message = lang
                                .pick("YoluPainter は最新です。", "YoluPainter is up to date.")
                                .into();
                        }
                    }
                    // 起動時の確かめの失敗は、利用者が頼んだことではないので知らせない。
                    Outcome::Checked(Err(failure)) => {
                        if manual {
                            self.message = failure_text(lang, "check", failure);
                        }
                    }
                    Outcome::Downloaded(Ok(ready)) => {
                        self.update.ready = Some(ready);
                        self.update.ready_wanted = true;
                    }
                    Outcome::Downloaded(Err(failure)) => {
                        self.message = failure_text(lang, "download", failure);
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
            self.message = lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        if self.update.ready.is_none() {
            return;
        }
        if save && self.modified {
            // 保存の結果（成功の文・失敗の理由）は保存の側が message に書く。空にしておけば、保存先の窓を取り消した
            // （message が空のまま）のと、保存が失敗した（理由が書かれている）のを、`update_finish_save` が見分けられる。
            self.clear_message();
            self.apply(crate::state::Action::SaveProject);
            self.update.after_save = true;
            self.update_finish_save();
        } else {
            self.update_launch();
        }
    }

    /// 保存の結果を受けて入れる（フレームの終わりにも呼ぶ。保存先を選ぶ窓が開いている間は待つ）。保存されていなければ入れない。
    /// 保存が失敗していれば、その理由（保存の側が書いた message）を残す。短い文を出すのは、保存先の窓を取り消したときだけ。
    pub fn update_finish_save(&mut self) {
        if !self.update.after_save || self.dialog_request.is_some() {
            return;
        }
        self.update.after_save = false;
        if self.modified {
            if self.message.is_empty() {
                self.message = self
                    .lang
                    .pick(
                        "保存しなかったので、更新しません。",
                        "Not updating: the project was not saved.",
                    )
                    .into();
            }
            return;
        }
        self.update_launch();
    }

    fn update_launch(&mut self) {
        let lang = self.lang;
        let Some(ready) = self.update.ready.clone() else {
            return;
        };
        if !staged_file_is_intact(&ready) {
            let _ = std::fs::remove_file(&ready.path);
            self.update.ready = None;
            self.update.ready_open = false;
            self.message = lang
                .pick(
                    "ダウンロードしたファイルが変わっています。更新しません。",
                    "The downloaded file has changed. Not updating.",
                )
                .into();
            return;
        }
        match (self.update.launcher)(&ready.path) {
            Ok(()) => {
                self.message = lang.pick(
                    format!("YoluPainter {} に更新します。", ready.version),
                    format!("Updating to YoluPainter {}.", ready.version),
                );
                self.update.ready_open = false;
                self.update.quitting = true;
                self.quit = true;
            }
            Err(_) => {
                self.message = lang
                    .pick(
                        "インストーラーを起動できません。",
                        "Cannot start the installer.",
                    )
                    .into();
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
        }
    }

    #[test]
    fn failure_texts_name_what_failed_in_both_languages() {
        for lang in Lang::ALL {
            for failure in [
                Failure::Network,
                Failure::Verify,
                Failure::Disk,
                Failure::Stopped,
            ] {
                for what in ["check", "download"] {
                    let text = failure_text(lang, what, failure);
                    assert!(text.contains(": "), "{text}");
                }
            }
        }
        assert!(failure_text(Lang::En, "check", Failure::Network)
            .starts_with("Cannot check for updates"));
        assert!(failure_text(Lang::Ja, "download", Failure::Verify)
            .starts_with("更新をダウンロードできません"));
        assert_eq!(
            failure_text(Lang::En, "download", Failure::Canceled),
            "Download canceled."
        );
    }
}
