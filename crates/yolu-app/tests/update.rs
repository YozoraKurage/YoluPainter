//! 自動更新のつなぎの試験。通信は偽の口（使い捨ての鍵で署名した更新情報を返す）で、インストーラーの起動・ページを開く口は記録するだけ。
//! 本物の鍵は使わない。`headless_` で始まる試験は画面を描かず、Wine でも回る。
mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use ed25519_dalek::{Signer, SigningKey};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::shell;
use yolu_app::state::{Action, AppState, DialogRequest, StrokeSource};
use yolu_app::update::http::Link;
use yolu_app::update::{Mode, Preference, UpdateAction};
use yolu_app::YoluApp;
use yolu_update::{
    asset_name, asset_url, release_page, sha256, Asset, Envelope, Error, Manifest, Transport,
    Version, LINUX_ARCHIVE, UPDATER_SCHEMA, UPDATER_URL, WINDOWS_ARCHIVE, WINDOWS_INSTALLER,
};

const SEED: [u8; 32] = [42; 32];
const INSTALLER: &[u8] = b"pretend this is the new installer";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-update-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 偽の配布元。更新情報とインストーラーを返し、取った URL を記録する。
struct Server {
    metadata: Mutex<Vec<u8>>,
    installer: Mutex<Vec<u8>>,
    calls: Mutex<Vec<String>>,
    /// インストーラーの取得を、これが下りるまで止める（途中の状態・取消を見る）。
    hold: AtomicBool,
    fail_metadata: AtomicBool,
    fail_download: AtomicBool,
}

struct Fake {
    server: Arc<Server>,
    link: Link,
}

impl Transport for Fake {
    fn get(&self, url: &str, _max_bytes: usize) -> Result<Vec<u8>, Error> {
        self.server.calls.lock().unwrap().push(url.to_owned());
        if url == UPDATER_URL {
            if self.server.fail_metadata.load(Ordering::Relaxed) {
                return Err(Error("ネットワークが無い".into()));
            }
            return Ok(self.server.metadata.lock().unwrap().clone());
        }
        while self.server.hold.load(Ordering::Relaxed) {
            if self.link.is_canceled() {
                return Err(Error("取り消し".into()));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.server.fail_download.load(Ordering::Relaxed) {
            return Err(Error("切れた".into()));
        }
        let bytes = self.server.installer.lock().unwrap().clone();
        self.link
            .progress
            .store(bytes.len() as u64 / 2, Ordering::Relaxed);
        Ok(bytes)
    }
}

fn disposable_key() -> SigningKey {
    SigningKey::from_bytes(&SEED)
}

fn public_key() -> [u8; 32] {
    disposable_key().verifying_key().to_bytes()
}

/// その版の、署名つきの更新情報（zip・インストーラー・tar.gz を載せる）。
fn signed_metadata(version: &str, installer: &[u8]) -> Vec<u8> {
    let v = Version::parse(version).unwrap();
    let asset = |target: &str, bytes: &[u8]| {
        let name = asset_name(&v, target).unwrap();
        Asset {
            target: target.into(),
            url: asset_url(&v, &name),
            name,
            sha256: sha256(bytes),
            size: bytes.len() as u64,
        }
    };
    let payload = serde_json::to_string(&Manifest {
        schema: UPDATER_SCHEMA,
        version: version.into(),
        assets: vec![
            asset(WINDOWS_ARCHIVE, b"zip"),
            asset(LINUX_ARCHIVE, b"tar"),
            asset(WINDOWS_INSTALLER, installer),
        ],
    })
    .unwrap();
    let signature = Some(hex::encode(
        disposable_key().sign(payload.as_bytes()).to_bytes(),
    ));
    serde_json::to_vec(&Envelope { payload, signature }).unwrap()
}

struct Rig {
    server: Arc<Server>,
    staging: TempDir,
    launched: Arc<Mutex<Vec<PathBuf>>>,
    opened: Arc<Mutex<Vec<String>>>,
}

/// 状態を「公開鍵つきのビルド・今は 0.1.0・サーバーは `served`」にする。
fn rig(state: &mut AppState, served: &str, mode: Mode) -> Rig {
    let server = Arc::new(Server {
        metadata: Mutex::new(signed_metadata(served, INSTALLER)),
        installer: Mutex::new(INSTALLER.to_vec()),
        calls: Mutex::new(Vec::new()),
        hold: AtomicBool::new(false),
        fail_metadata: AtomicBool::new(false),
        fail_download: AtomicBool::new(false),
    });
    let staging = TempDir::new("staging");
    let launched = Arc::new(Mutex::new(Vec::new()));
    let opened = Arc::new(Mutex::new(Vec::new()));
    let target = match mode {
        Mode::Installer => WINDOWS_INSTALLER,
        Mode::Page => LINUX_ARCHIVE,
    };
    state
        .update
        .configure_for_test(Some(public_key()), "0.1.0", Some(target), mode);
    let shared = server.clone();
    state.update.set_transport_for_test(Arc::new(move |link| {
        Box::new(Fake {
            server: shared.clone(),
            link,
        })
    }));
    let (l, o) = (launched.clone(), opened.clone());
    state.update.set_actions_for_test(
        Arc::new(move |path| {
            l.lock().unwrap().push(path.to_owned());
            Ok(())
        }),
        Arc::new(move |url| {
            o.lock().unwrap().push(url.to_owned());
            Ok(())
        }),
        staging.0.clone(),
    );
    Rig {
        server,
        staging,
        launched,
        opened,
    }
}

impl Rig {
    fn calls(&self) -> usize {
        self.server.calls.lock().unwrap().len()
    }
    fn launched(&self) -> Vec<PathBuf> {
        self.launched.lock().unwrap().clone()
    }
}

/// 通信・ダウンロードが終わるまで、フレームの初めの受け取りを回す。
fn settle(state: &mut AppState) {
    let start = Instant::now();
    while state.update.is_busy() {
        state.poll_update();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "更新の仕事が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    state.poll_update();
}

fn apply(state: &mut AppState, action: UpdateAction) {
    state.apply(Action::Update(action));
}

/// 新しい版を見つけるまで（手で確かめる）。
fn find_update(state: &mut AppState) {
    apply(state, UpdateAction::Check);
    settle(state);
    assert!(state.update.offer().is_some(), "{}", state.message);
}

fn help_labels(state: &AppState) -> Vec<String> {
    shell::menu_entries(state, shell::HELP_MENU)
        .iter()
        .filter_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect()
}

fn help_item(state: &AppState, label: &str) -> (bool, yolu_app::ui::menu::Check) {
    shell::menu_entries(state, shell::HELP_MENU)
        .into_iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item {
                label: l,
                enabled,
                check,
                ..
            } if l == label => Some((enabled, check)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{label} が無い: {:?}", help_labels(state)))
}

// ───────── 公開鍵が無いビルド ─────────

#[test]
fn headless_a_build_without_a_public_key_shows_nothing_and_sends_nothing() {
    let mut state = AppState::new(64, 64);
    assert!(!state.update.enabled());
    assert_eq!(help_labels(&state), ["YoluPainter について"]);
    state.update_startup();
    assert!(!state.update.is_asking() && !state.update.window_open());
    apply(&mut state, UpdateAction::Check);
    assert!(!state.update.is_busy());
    assert!(state.update.offer().is_none());
}

// ───────── 置き場の片付け ─────────

#[test]
fn headless_startup_clears_old_installers_but_not_a_newer_one() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    let setup = |version: &str| format!("yolupainter-{version}-{WINDOWS_INSTALLER}.exe");
    for name in [
        setup("0.1.0"),
        format!("{}.part", setup("0.1.0")),
        setup("0.2.0"),
        "mine.txt".to_owned(),
    ] {
        std::fs::write(rig.staging.0.join(name), b"x").unwrap();
    }
    // 今は 0.1.0。同じ版・古い版（走らせ終えたもの）は消え、新しい版（別のアプリが落としている最中かもしれない）と
    // 利用者のファイルは残る
    state.update_startup();
    assert_eq!(rig.staging.files(), ["mine.txt", setup("0.2.0").as_str()]);
}

// ───────── 聞かずに通信しない ─────────

#[test]
fn headless_nothing_is_sent_until_the_user_has_answered() {
    let dir = TempDir::new("config");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.update.attach_config(dir.0.join("update.conf"));
    // 初めての起動: 問いが出て、答えるまで通信しない（待っても、何も送られない）
    state.update_startup();
    assert!(state.update.is_asking());
    assert_eq!(state.update.preference(), Preference::Unset);
    for _ in 0..20 {
        state.poll_update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(rig.calls(), 0);
    assert!(!state.update.is_busy());
    // 「いいえ」: 通信せず、選択を保存する
    apply(&mut state, UpdateAction::Answer(false));
    assert!(!state.update.is_asking());
    assert_eq!(rig.calls(), 0);
    assert_eq!(
        std::fs::read_to_string(dir.0.join("update.conf")).unwrap(),
        "check_on_startup=off\n"
    );
    // 次の起動も、確かめない・問わない
    let mut next = AppState::new(64, 64);
    let rig2 = self::rig(&mut next, "0.2.0", Mode::Installer);
    next.update.attach_config(dir.0.join("update.conf"));
    next.update_startup();
    assert!(!next.update.is_asking() && !next.update.is_busy());
    assert_eq!(rig2.calls(), 0);
}

#[test]
fn headless_yes_saves_the_choice_and_checks_at_every_later_startup() {
    let dir = TempDir::new("config");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.update.attach_config(dir.0.join("update.conf"));
    state.update_startup();
    apply(&mut state, UpdateAction::Answer(true));
    settle(&mut state);
    assert_eq!(rig.calls(), 1);
    assert_eq!(rig.server.calls.lock().unwrap()[0], UPDATER_URL);
    assert_eq!(state.update.offer().unwrap().version.to_string(), "0.2.0");
    // 起動時の確かめは、見つけても何も言わない（印と項目だけ。手で確かめたときだけ知らせる）
    assert!(state.message.is_empty(), "{}", state.message);
    let mut next = AppState::new(64, 64);
    let rig2 = self::rig(&mut next, "0.2.0", Mode::Installer);
    next.update.attach_config(dir.0.join("update.conf"));
    assert_eq!(next.update.preference(), Preference::On);
    next.update_startup();
    settle(&mut next);
    assert_eq!(rig2.calls(), 1);
    assert!(next.update.offer().is_some());
}

#[test]
fn headless_the_startup_setting_can_be_switched_from_the_help_menu_later() {
    let dir = TempDir::new("config");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.update.attach_config(dir.0.join("update.conf"));
    assert_eq!(
        help_item(&state, "起動時に更新を確かめる").1,
        yolu_app::ui::menu::Check::None
    );
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    // 切り替えただけでは通信しない
    assert_eq!(rig.calls(), 0);
    assert_eq!(
        help_item(&state, "起動時に更新を確かめる").1,
        yolu_app::ui::menu::Check::Checked
    );
    assert_eq!(
        std::fs::read_to_string(dir.0.join("update.conf")).unwrap(),
        "check_on_startup=on\n"
    );
    apply(&mut state, UpdateAction::SetCheckOnStartup(false));
    assert_eq!(
        help_item(&state, "起動時に更新を確かめる").1,
        yolu_app::ui::menu::Check::None
    );
    assert_eq!(
        std::fs::read_to_string(dir.0.join("update.conf")).unwrap(),
        "check_on_startup=off\n"
    );
    // 保存できなくても、この回の選択は効き、知らせる
    std::fs::remove_dir_all(&dir.0).unwrap();
    std::fs::write(&dir.0, b"a file where the folder should be").unwrap();
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    assert_eq!(state.update.preference(), Preference::On);
    assert!(
        state.message.contains("更新の設定を保存できません"),
        "{}",
        state.message
    );
    std::fs::remove_file(&dir.0).unwrap();
}

#[test]
fn headless_a_manual_check_needs_no_earlier_choice_and_tells_the_result() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.1.0", Mode::Installer);
    // 今と同じ版: 最新
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(rig.calls(), 1);
    assert_eq!(state.message, "YoluPainter は最新です。");
    assert!(state.update.offer().is_none());
    // 新しい版: あると知らせ、メニューに項目が出る
    *rig.server.metadata.lock().unwrap() = signed_metadata("0.2.0", INSTALLER);
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(state.message, "YoluPainter 0.2.0 があります。");
    assert_eq!(help_labels(&state)[0], "YoluPainter 0.2.0 に更新");
    // 英語
    state.lang = Lang::En;
    assert_eq!(help_labels(&state)[0], "Update to YoluPainter 0.2.0");
    assert_eq!(help_labels(&state)[1], "Check for Updates…");
}

#[test]
fn headless_failures_are_told_by_kind_in_both_languages() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    // 通信できない
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(state.message, "更新を確かめられません: 通信できません");
    assert!(state.update.offer().is_none());
    state.lang = Lang::En;
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(state.message, "Cannot check for updates: connection failed");
    // 取れたが、署名が合わない（別の鍵で署名した更新情報）
    rig.server.fail_metadata.store(false, Ordering::Relaxed);
    let v = Version::parse("0.2.0").unwrap();
    let name = asset_name(&v, WINDOWS_INSTALLER).unwrap();
    let payload = serde_json::to_string(&Manifest {
        schema: UPDATER_SCHEMA,
        version: "0.2.0".into(),
        assets: vec![Asset {
            target: WINDOWS_INSTALLER.into(),
            url: asset_url(&v, &name),
            name,
            sha256: sha256(INSTALLER),
            size: INSTALLER.len() as u64,
        }],
    })
    .unwrap();
    let other = SigningKey::from_bytes(&[43; 32]);
    let signature = Some(hex::encode(other.sign(payload.as_bytes()).to_bytes()));
    *rig.server.metadata.lock().unwrap() =
        serde_json::to_vec(&Envelope { payload, signature }).unwrap();
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates: verification failed"
    );
    assert!(state.update.offer().is_none());
}

#[test]
fn headless_a_failed_startup_check_stays_quiet() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    state
        .update
        .attach_config(TempDir::new("quiet").0.join("update.conf"));
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    state.update_startup();
    settle(&mut state);
    assert_eq!(rig.calls(), 1);
    assert!(state.message.is_empty(), "{}", state.message);
}

// ───────── ダウンロードと検証、インストーラーの起動 ─────────

#[test]
fn headless_update_waits_for_the_user_then_downloads_verifies_and_runs_the_installer() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    // 見つけただけではダウンロードしない
    assert_eq!(rig.calls(), 1);
    assert!(state.update.ready().is_none());
    // 押す → ダウンロード（途中の進み具合と、メニューの名前）
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    assert!(state.update.is_busy());
    assert_eq!(help_labels(&state)[0], "YoluPainter 0.2.0 をダウンロード中");
    assert!(!matches!(
        help_item(&state, "YoluPainter 0.2.0 をダウンロード中"),
        (true, _)
    ));
    let progress = state.update.progress().unwrap();
    assert_eq!(progress.version.to_string(), "0.2.0");
    assert!(!progress.canceling);
    rig.server.hold.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert_eq!(rig.calls(), 2);
    assert_eq!(
        rig.server.calls.lock().unwrap()[1],
        asset_url(
            &Version::new(0, 2, 0),
            "yolupainter-0.2.0-x86_64-pc-windows-msvc-setup.exe"
        )
    );
    // 検証を通ったファイルが置き場にあり、準備の窓が開く。まだ走らせない（アプリは閉じない）
    let name = "yolupainter-0.2.0-x86_64-pc-windows-msvc-setup.exe";
    assert_eq!(rig.staging.files(), [name]);
    assert_eq!(std::fs::read(rig.staging.0.join(name)).unwrap(), INSTALLER);
    assert!(state.update.is_ready_open());
    assert!(rig.launched().is_empty());
    assert!(!state.quit);
    // 保存していない変更が無ければ、そのまま更新して再起動する
    apply(&mut state, UpdateAction::Run { save: false });
    assert_eq!(rig.launched(), [rig.staging.0.join(name)]);
    assert!(state.quit && state.update.is_quitting());
    assert!(!state.update.is_ready_open());
}

#[test]
fn headless_a_corrupt_or_cut_download_is_rejected_and_leaves_nothing_to_run() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    // 中身が違う（SHA-256 が合わない）
    *rig.server.installer.lock().unwrap() = b"pretend this is the NEW installer".to_vec();
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert_eq!(
        state.message,
        "更新をダウンロードできません: 検証を通りません"
    );
    assert!(rig.staging.files().is_empty());
    assert!(state.update.ready().is_none() && !state.update.is_ready_open());
    // 途中で切れた
    *rig.server.installer.lock().unwrap() = INSTALLER.to_vec();
    rig.server.fail_download.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert_eq!(
        state.message,
        "更新をダウンロードできません: 通信できません"
    );
    assert!(rig.staging.files().is_empty());
    assert!(rig.launched().is_empty() && !state.quit);
    // 落とし直せる
    rig.server.fail_download.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert!(state.update.is_ready_open());
}

#[test]
fn headless_a_file_changed_after_download_is_not_run() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    let path = rig.staging.0.join(rig.staging.files().remove(0));
    // 置き場のファイルが差し替えられた（長さは同じ）
    std::fs::write(&path, b"pretend this is the evil installer!").unwrap();
    apply(&mut state, UpdateAction::Run { save: false });
    assert!(rig.launched().is_empty());
    assert!(!state.quit);
    assert_eq!(
        state.message,
        "ダウンロードしたファイルが変わっています。更新しません。"
    );
    assert!(state.update.ready().is_none());
    assert!(!path.exists());
}

#[test]
fn headless_canceling_stops_the_download_and_removes_nothing_it_did_not_write() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    assert!(state.update.is_busy());
    apply(&mut state, UpdateAction::Cancel);
    assert!(state.update.progress().unwrap().canceling);
    settle(&mut state);
    assert_eq!(state.message, "ダウンロードを取り消しました。");
    assert!(rig.staging.files().is_empty());
    assert!(state.update.ready().is_none());
    // 取り消したあとも、もう一度押せる
    rig.server.hold.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert!(state.update.is_ready_open());
}

#[test]
fn headless_closing_the_app_cancels_a_running_download() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    yolu_app::windows::stop_jobs(&mut state, Duration::from_secs(5));
    assert!(!state.update.is_busy());
    assert!(rig.staging.files().is_empty());
}

#[test]
fn headless_installing_is_refused_while_drawing_and_the_window_waits_for_the_stroke() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    state.canvas.stroke = Some(StrokeSource::Mouse);
    apply(&mut state, UpdateAction::Install);
    assert!(!state.update.is_busy());
    assert_eq!(state.message, "描いている間はできません。");
    assert!(!matches!(
        help_item(&state, "YoluPainter 0.2.0 に更新"),
        (true, _)
    ));
    // ダウンロード中に描き始めた: 終わっても、描き終わるまで準備の窓を出さない
    state.canvas.stroke = None;
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    state.canvas.stroke = Some(StrokeSource::Mouse);
    rig.server.hold.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert!(state.update.ready().is_some());
    assert!(!state.update.is_ready_open());
    // 描いている間は走らせない
    apply(&mut state, UpdateAction::Run { save: false });
    assert!(rig.launched().is_empty());
    state.canvas.stroke = None;
    state.poll_update();
    assert!(state.update.is_ready_open());
}

#[test]
fn headless_later_keeps_the_download_and_pressing_update_again_reuses_it() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    apply(&mut state, UpdateAction::Later);
    assert!(!state.update.is_ready_open());
    assert_eq!(rig.staging.files().len(), 1);
    let calls = rig.calls();
    apply(&mut state, UpdateAction::Install);
    state.poll_update();
    // 落とし直さずに、準備の窓がもう一度出る
    assert_eq!(rig.calls(), calls);
    assert!(state.update.is_ready_open());
}

// ───────── 保存していない変更 ─────────

#[test]
fn headless_unsaved_changes_are_saved_first_and_an_unsaved_project_stops_the_update() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    state.modified = true;
    // まだ名前の無いプロジェクト: 保存先を選ぶ窓の頼みが出る。その窓をやめたら、更新しない
    apply(&mut state, UpdateAction::Run { save: true });
    assert_eq!(state.dialog_request, Some(DialogRequest::SaveAs));
    assert!(rig.launched().is_empty());
    state.update_finish_save();
    assert!(rig.launched().is_empty(), "窓がまだ開いている間は待つ");
    state.dialog_request = None; // やめた
    state.update_finish_save();
    assert!(rig.launched().is_empty());
    assert!(!state.quit);
    assert_eq!(state.message, "保存しなかったので、更新しません。");
    assert!(state.update.is_ready_open(), "更新の窓は残り、やり直せる");
    // 保存先を選んで保存した: そのあとで更新する
    let dir = TempDir::new("save");
    apply(&mut state, UpdateAction::Run { save: true });
    state.dialog_request = None;
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    assert!(!state.modified);
    state.update_finish_save();
    assert_eq!(rig.launched().len(), 1);
    assert!(state.quit);
}

#[test]
fn headless_saving_a_named_project_then_updating_and_updating_without_saving() {
    let dir = TempDir::new("save");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    assert!(!state.modified);
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    // 保存して更新: その場で保存され、そのまま入れる
    state.modified = true;
    let before = std::fs::metadata(dir.0.join("a.ylp"))
        .unwrap()
        .modified()
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    apply(&mut state, UpdateAction::Run { save: true });
    assert!(!state.modified);
    assert!(
        std::fs::metadata(dir.0.join("a.ylp"))
            .unwrap()
            .modified()
            .unwrap()
            >= before
    );
    assert_eq!(rig.launched().len(), 1);
    assert!(state.quit && state.update.is_quitting());
}

#[test]
fn headless_update_without_saving_leaves_the_project_file_alone() {
    let dir = TempDir::new("nosave");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    let saved = std::fs::read(dir.0.join("a.ylp")).unwrap();
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    state.modified = true;
    apply(&mut state, UpdateAction::Run { save: false });
    assert_eq!(rig.launched().len(), 1);
    assert!(
        state.quit && state.modified,
        "保存していない変更は、捨てると選んだとおり残ったまま終わる"
    );
    assert_eq!(std::fs::read(dir.0.join("a.ylp")).unwrap(), saved);
}

#[test]
fn headless_a_failed_save_keeps_its_reason_and_stops_the_update() {
    let dir = TempDir::new("savefail");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    assert!(!state.modified);
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    // 保存先が外から消えて、保存できない（元の場所へは上書きしない）
    std::fs::remove_dir_all(&dir.0).unwrap();
    state.modified = true;
    // 2 回続けても（前と同じ文の失敗でも）、理由が残る。保存できなかったので入れない
    for attempt in 0..2 {
        apply(&mut state, UpdateAction::Run { save: true });
        assert!(
            state.message.starts_with("保存できません: ") && state.message.contains("a.ylp"),
            "{attempt}: {}",
            state.message
        );
        assert!(state.modified && !state.quit && rig.launched().is_empty());
        assert!(state.update.is_ready_open(), "更新の窓は残り、やり直せる");
        state.update_finish_save();
        assert!(state.message.starts_with("保存できません: "), "{}", state.message);
    }
    // 別の場所へ保存し直せば、そのあとで保存して入れる
    let elsewhere = TempDir::new("savefail-elsewhere");
    state.apply(Action::SaveProjectAs(elsewhere.0.join("b.ylp")));
    assert!(!state.modified, "{}", state.message);
    state.modified = true;
    apply(&mut state, UpdateAction::Run { save: true });
    assert!(!state.modified, "{}", state.message);
    assert_eq!(rig.launched().len(), 1);
    assert!(state.quit);
}

#[test]
fn headless_a_failed_launch_keeps_the_app_open() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    let staging = rig.staging.0.clone();
    state.update.set_actions_for_test(
        Arc::new(|_| Err(std::io::Error::other("blocked"))),
        Arc::new(|_| Ok(())),
        staging,
    );
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    apply(&mut state, UpdateAction::Run { save: false });
    assert!(!state.quit && !state.update.is_quitting());
    assert_eq!(state.message, "インストーラーを起動できません。");
}

// ───────── ページを開くだけの環境 ─────────

#[test]
fn headless_where_it_cannot_replace_itself_it_only_opens_the_release_page() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Page);
    find_update(&mut state);
    assert_eq!(help_labels(&state)[0], "YoluPainter 0.2.0 のリリースを開く");
    apply(&mut state, UpdateAction::Install);
    // ダウンロードも起動もせず、その版のページを開く
    assert_eq!(
        *rig.opened.lock().unwrap(),
        [release_page(&Version::new(0, 2, 0))]
    );
    assert_eq!(rig.calls(), 1);
    assert!(rig.launched().is_empty() && rig.staging.files().is_empty());
    assert!(!state.quit && !state.update.is_ready_open());
    assert!(
        state.message.contains("リリースのページを開きました"),
        "{}",
        state.message
    );
}

// ───────── 画面（窓・メニュー・印） ─────────

/// 窓の中だけを撮って、正解の絵と比べる。
fn shot(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

fn click_label(h: &mut Harness<'_, YoluApp>, label: &str) {
    let at = rect_of(h, label, |_| true).center();
    click(h, at);
}

#[test]
fn the_first_question_is_a_modal_window_with_two_answers_and_no_explanation() {
    let dir = TempDir::new("ask");
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    h.state_mut()
        .state
        .update
        .attach_config(dir.0.join("update.conf"));
    h.state_mut().state.update_startup();
    h.run();
    shot(&mut h, "update-ask", "update_ask");
    // 問いと選択肢だけ（説明の文は置かない）。問いの文は描くだけの文字なので、絵の比較（update_ask）で確かめる
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("いいえ");
        h.get_by_label("はい");
    }
    // 下のキャンバスなどへ入力を渡さない: 問いの間は、キーの割り当てが止まる
    assert!(yolu_app::windows::modal_open(&h.state().state));
    assert_eq!(rig.calls(), 0);
    click_label(&mut h, "はい");
    h.run();
    assert!(!h.state().state.update.is_asking());
    assert_eq!(h.state().state.update.preference(), Preference::On);
    assert_eq!(
        std::fs::read_to_string(dir.0.join("update.conf")).unwrap(),
        "check_on_startup=on\n"
    );
    let state = &mut h.state_mut().state;
    settle(state);
    assert_eq!(rig.calls(), 1);
}

#[test]
fn closing_or_escaping_the_question_means_no_and_never_connects() {
    let dir = TempDir::new("ask-esc");
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    h.state_mut()
        .state
        .update
        .attach_config(dir.0.join("update.conf"));
    h.state_mut().state.update_startup();
    h.run();
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.update.is_asking());
    assert_eq!(h.state().state.update.preference(), Preference::Off);
    assert_eq!(rig.calls(), 0);
}

#[test]
fn the_question_is_in_english_too() {
    let mut h = app(1280.0, 800.0, 64);
    let _rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    h.state_mut().state.lang = Lang::En;
    h.state_mut().state.update_startup();
    h.run();
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("No");
        h.get_by_label("Yes");
    }
    shot(&mut h, "update-ask", "update_ask_english");
}

#[test]
fn the_help_title_carries_a_mark_while_a_new_version_waits_and_the_item_runs_the_update() {
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    h.run();
    // 印の無いヘルプ
    let bar = menu_title(&h, "ヘルプ");
    let plain = {
        let image = h.render().expect("描画");
        image::imageops::crop_imm(&image, bar.left() as u32, 0, bar.width() as u32, 24).to_image()
    };
    find_update(&mut h.state_mut().state);
    h.run();
    let marked = {
        let image = h.render().expect("描画");
        image::imageops::crop_imm(&image, bar.left() as u32, 0, bar.width() as u32, 24).to_image()
    };
    assert_ne!(
        plain.as_raw(),
        marked.as_raw(),
        "新しい版があるとき、ヘルプの見出しに印が付く"
    );
    // メニューから更新する
    click(&mut h, bar.center());
    click_label_in_popup(&mut h, "YoluPainter 0.2.0 に更新");
    {
        let state = &mut h.state_mut().state;
        settle(state);
    }
    h.run();
    assert!(h.state().state.update.is_ready_open());
    assert!(rig.launched().is_empty());
}

fn click_label_in_popup(h: &mut Harness<'_, YoluApp>, label: &str) {
    let at = popup_item(h, label).center();
    click(h, at);
}

#[test]
fn the_ready_window_asks_to_save_first_when_there_are_unsaved_changes() {
    let dir = TempDir::new("ready");
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    {
        let state = &mut h.state_mut().state;
        state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
        find_update(state);
        apply(state, UpdateAction::Install);
        settle(state);
        state.modified = true;
    }
    h.run();
    shot(&mut h, "update-ready", "update_ready_unsaved");
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("あとで");
        h.get_by_label("保存せずに更新");
        h.get_by_label("保存して更新");
    }
    assert!(rig.launched().is_empty());
    click_label(&mut h, "保存して更新");
    h.run();
    assert!(!h.state().state.modified, "先に保存する");
    assert_eq!(rig.launched().len(), 1);
    assert!(h.state().state.quit);
}

#[test]
fn the_ready_window_is_short_when_nothing_is_unsaved_and_later_closes_it() {
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    {
        let state = &mut h.state_mut().state;
        find_update(state);
        apply(state, UpdateAction::Install);
        settle(state);
    }
    h.run();
    shot(&mut h, "update-ready", "update_ready");
    {
        use egui_kittest::kittest::Queryable;
        assert!(h.query_by_label("保存して更新").is_none());
        h.get_by_label("更新して再起動");
    }
    click_label(&mut h, "あとで");
    h.run();
    assert!(!h.state().state.update.is_ready_open());
    assert!(rig.launched().is_empty());
    // ヘルプのメニューにはまだ更新の項目が残る
    assert_eq!(help_labels(&h.state().state)[0], "YoluPainter 0.2.0 に更新");
}

#[test]
fn the_download_shows_a_job_card_with_a_cancel_button() {
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    {
        let state = &mut h.state_mut().state;
        find_update(state);
        rig.server.hold.store(true, Ordering::Relaxed);
        apply(state, UpdateAction::Install);
    }
    h.run();
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("取消: 更新をダウンロード中");
    }
    click_label(&mut h, "取消: 更新をダウンロード中");
    let state = &mut h.state_mut().state;
    settle(state);
    assert_eq!(state.message, "ダウンロードを取り消しました。");
}

/// 開いているメニューの中だけ（四隅と影を除く内側）を撮って、正解の絵と比べる。外側には下の画面が写り、
/// ほかのパネルの変更で壊れるので撮らない。見出しの印は `the_help_title_carries_a_mark…` が見る。
fn shot_menu(h: &mut Harness<'_, YoluApp>, name: &str) {
    h.event(egui::Event::PointerGone);
    h.step();
    let popup = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect
        .shrink(6.0);
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        popup.left().floor() as u32,
        popup.top().floor() as u32,
        popup.width().ceil() as u32,
        popup.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn the_help_menu_lists_the_update_items_with_the_new_version_first() {
    let mut h = app(1280.0, 800.0, 64);
    let _rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    h.state_mut()
        .state
        .update
        .attach_config(TempDir::new("menu").0.join("update.conf"));
    find_update(&mut h.state_mut().state);
    h.run();
    let at = menu_title(&h, "ヘルプ").center();
    click(&mut h, at);
    shot_menu(&mut h, "menu_help_update");
    // 新しい版が無いビルドと同じく、言語を替えれば英語になる
    h.state_mut().state.lang = Lang::En;
    h.run();
    let at = menu_title(&h, "Help").center();
    click(&mut h, at);
    click(&mut h, at);
    shot_menu(&mut h, "menu_help_update_english");
}
