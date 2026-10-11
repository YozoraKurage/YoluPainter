//! 自動更新のつなぎの試験。通信は偽の口（使い捨ての鍵で署名した更新情報を返す）で、インストーラーの起動・ページを開く口は記録するだけ。
//! 本物の鍵は使わない。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

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
    Version, BETA_UPDATER_URL, LINUX_ARCHIVE, MACOS_ARCHIVE, UPDATER_SCHEMA, UPDATER_URL,
    WINDOWS_ARCHIVE, WINDOWS_INSTALLER,
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
    /// 試験版の置き場の更新情報（`None` は、置き場が引けない）。
    beta_metadata: Mutex<Option<Vec<u8>>>,
    /// 試験版の置き場の取得を、これが下りるまで止める（確かめの途中で設定を替える）。
    hold_beta: AtomicBool,
    installer: Mutex<Vec<u8>>,
    calls: Mutex<Vec<String>>,
    /// インストーラーの取得を、これが下りるまで止める（途中の状態・取消を見る）。
    hold: AtomicBool,
    /// 止めている間に取消を見ない（転送は終わっていて、そのあとの確かめ・書き込みの間に取り消された形にする）。
    ignore_cancel: AtomicBool,
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
        if url == BETA_UPDATER_URL {
            while self.server.hold_beta.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            return self
                .server
                .beta_metadata
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| Error("置き場が無い".into()));
        }
        if url == UPDATER_URL {
            if self.server.fail_metadata.load(Ordering::Relaxed) {
                return Err(Error("ネットワークが無い".into()));
            }
            return Ok(self.server.metadata.lock().unwrap().clone());
        }
        while self.server.hold.load(Ordering::Relaxed) {
            if self.link.is_canceled() && !self.server.ignore_cancel.load(Ordering::Relaxed) {
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

/// その版の、署名つきの更新情報（zip・インストーラー・tar.gz・macOS の zip を載せる）。
fn signed_metadata(version: &str, installer: &[u8]) -> Vec<u8> {
    signed_metadata_without(version, installer, None)
}

/// `signed_metadata` から、`skip` の対象の配布物だけ載せない更新情報（署名は載せたものに対して正しい）。
fn signed_metadata_without(version: &str, installer: &[u8], skip: Option<&str>) -> Vec<u8> {
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
        assets: [
            (WINDOWS_ARCHIVE, &b"zip"[..]),
            (LINUX_ARCHIVE, &b"tar"[..]),
            (WINDOWS_INSTALLER, installer),
            (MACOS_ARCHIVE, &b"mac"[..]),
        ]
        .into_iter()
        .filter(|(target, _)| Some(*target) != skip)
        .map(|(target, bytes)| asset(target, bytes))
        .collect(),
    })
    .unwrap();
    let signature = Some(hex::encode(
        disposable_key().sign(payload.as_bytes()).to_bytes(),
    ));
    serde_json::to_vec(&Envelope { payload, signature }).unwrap()
}

pub(crate) struct Rig {
    server: Arc<Server>,
    staging: TempDir,
    launched: Arc<Mutex<Vec<PathBuf>>>,
    opened: Arc<Mutex<Vec<String>>>,
}

/// 状態を「公開鍵つきのビルド・今は 0.1.0・サーバーは `served`」にする。
pub(crate) fn rig(state: &mut AppState, served: &str, mode: Mode) -> Rig {
    let server = Arc::new(Server {
        metadata: Mutex::new(signed_metadata(served, INSTALLER)),
        beta_metadata: Mutex::new(None),
        hold_beta: AtomicBool::new(false),
        installer: Mutex::new(INSTALLER.to_vec()),
        calls: Mutex::new(Vec::new()),
        hold: AtomicBool::new(false),
        ignore_cancel: AtomicBool::new(false),
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
    // 実際のプロセスの一覧は見ない（別の起動の試験だけが、自分で答えを決める）
    state.update.set_other_instance_for_test(Arc::new(|| false));
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

/// 設定のウィンドウの「更新」の区分の、起動時の確かめの印（値は更新の部品が持つ）。
fn startup_check(state: &AppState) -> yolu_app::ui::menu::Check {
    if state.update.preference() == Preference::On {
        yolu_app::ui::menu::Check::Checked
    } else {
        yolu_app::ui::menu::Check::None
    }
}

/// 同じく、試験版を使うの印。
fn beta_check(state: &AppState) -> yolu_app::ui::menu::Check {
    if state.update.beta() {
        yolu_app::ui::menu::Check::Checked
    } else {
        yolu_app::ui::menu::Check::None
    }
}

// ───────── 公開鍵が無いビルド ─────────

#[test]
fn headless_a_build_without_a_public_key_shows_nothing_and_sends_nothing() {
    let mut state = AppState::new(64, 64);
    assert!(!state.update.enabled());
    // 更新の項目（更新・確認）は出ない。ログのフォルダ・「について」は公開鍵の有無によらず出る
    let updates = shell::menu_entries(&state, shell::HELP_MENU)
        .iter()
        .filter(|e| {
            matches!(
                e,
                yolu_app::ui::menu::Entry::Item {
                    action: Action::Update(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(updates, 0, "{:?}", help_labels(&state));
    assert_eq!(
        help_labels(&state),
        ["ログのフォルダを開く", "YoluPainter について"]
    );
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
fn headless_the_startup_setting_can_be_switched_later() {
    let dir = TempDir::new("config");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.update.attach_config(dir.0.join("update.conf"));
    assert_eq!(startup_check(&state), yolu_app::ui::menu::Check::None);
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    // 切り替えただけでは通信しない
    assert_eq!(rig.calls(), 0);
    assert_eq!(startup_check(&state), yolu_app::ui::menu::Check::Checked);
    assert_eq!(
        std::fs::read_to_string(dir.0.join("update.conf")).unwrap(),
        "check_on_startup=on\n"
    );
    apply(&mut state, UpdateAction::SetCheckOnStartup(false));
    assert_eq!(startup_check(&state), yolu_app::ui::menu::Check::None);
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
    // ログのフォルダは区切りの後・「について」の前
    assert!(help_labels(&state).ends_with(&[
        "ログのフォルダを開く".to_string(),
        "YoluPainter について".to_string()
    ]));
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
    assert_eq!(state.message, "更新を確かめられません（通信できません）。");
    assert!(state.update.offer().is_none());
    state.lang = Lang::En;
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates (connection failed)."
    );
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
        "Cannot check for updates (verification failed)."
    );
    assert!(state.update.offer().is_none());
}

#[test]
fn headless_a_failed_startup_check_stays_quiet() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    // 持っておく（式の中で作ると、すぐ消えて、あとで設定を書くときに同じ名前のフォルダが作り直されて残る）
    let config_dir = TempDir::new("quiet");
    state.update.attach_config(config_dir.0.join("update.conf"));
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    state.update_startup();
    settle(&mut state);
    assert_eq!(rig.calls(), 1);
    assert!(state.message.is_empty(), "{}", state.message);
}

// ───────── 試験版 ─────────

/// 試験版の置き場に、その版の更新情報を置く（`None` なら、置き場は引けない）。
fn serve_beta(rig: &Rig, version: Option<&str>) {
    *rig.server.beta_metadata.lock().unwrap() = version.map(|v| signed_metadata(v, INSTALLER));
}

/// stable の更新情報を、その版にする。
fn serve_stable(rig: &Rig, version: &str) {
    *rig.server.metadata.lock().unwrap() = signed_metadata(version, INSTALLER);
}

fn asked(rig: &Rig) -> Vec<String> {
    rig.server.calls.lock().unwrap().clone()
}

fn check_now(state: &mut AppState) {
    apply(state, UpdateAction::Check);
    settle(state);
}

fn offered(state: &AppState) -> Option<String> {
    state.update.offer().map(|o| o.version.to_string())
}

#[test]
fn headless_the_beta_setting_is_off_by_default_and_an_old_settings_file_reads_as_before() {
    let dir = TempDir::new("beta-default");
    // 旧い版が書いた 1 行だけのファイル
    std::fs::write(dir.0.join("update.conf"), "check_on_startup=on\n").unwrap();
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    state.update.attach_config(dir.0.join("update.conf"));
    assert_eq!(state.update.preference(), Preference::On);
    assert!(!state.update.beta());
    assert_eq!(beta_check(&state), yolu_app::ui::menu::Check::None);
    // 試験版が置いてあっても、切のうちは stable の置き場だけを見る
    state.update_startup();
    settle(&mut state);
    assert_eq!(asked(&rig), [UPDATER_URL]);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    // 設定が壊れていても、試験版は切（stable だけ）
    let broken = TempDir::new("beta-broken");
    std::fs::write(broken.0.join("update.conf"), "use_beta=maybe").unwrap();
    let mut next = AppState::new(64, 64);
    let rig2 = self::rig(&mut next, "0.2.0", Mode::Installer);
    serve_beta(&rig2, Some("0.3.0-rc.1"));
    next.update.attach_config(broken.0.join("update.conf"));
    assert!(!next.update.beta() && next.update.preference() == Preference::Unset);
    check_now(&mut next);
    assert_eq!(asked(&rig2), [UPDATER_URL]);
}

#[test]
fn headless_the_beta_setting_offers_the_newer_of_beta_and_stable_and_off_goes_back_to_stable() {
    let dir = TempDir::new("beta-switch");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    state.update.attach_config(dir.0.join("update.conf"));
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    // 入にしただけでは通信しない。いま出ている提案もそのまま
    apply(&mut state, UpdateAction::SetBeta(true));
    assert_eq!(rig.calls(), 1);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    assert_eq!(beta_check(&state), yolu_app::ui::menu::Check::Checked);
    // 入: stable と試験版の新しい方（試験版 0.3.0-rc.1）。両方の置き場を見る
    check_now(&mut state);
    assert_eq!(asked(&rig)[1..], [UPDATER_URL, BETA_UPDATER_URL]);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0-rc.1"));
    assert_eq!(state.message, "YoluPainter 0.3.0-rc.1 があります。");
    assert_eq!(help_labels(&state)[0], "YoluPainter 0.3.0-rc.1 に更新");
    // stable が試験版より新しくなれば、試験版を使う人にも stable が見える
    serve_stable(&rig, "0.3.0");
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0"));
    // 次の試験版が出れば、それが新しい
    serve_beta(&rig, Some("0.4.0-rc.1"));
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.4.0-rc.1"));
    // 切: 見つけていた試験版の提案はその場で消え、次の確かめは stable だけ
    let before = rig.calls();
    apply(&mut state, UpdateAction::SetBeta(false));
    assert!(state.update.offer().is_none());
    assert_eq!(beta_check(&state), yolu_app::ui::menu::Check::None);
    assert!(help_labels(&state)[0].starts_with("更新を確かめる"));
    assert_eq!(rig.calls(), before);
    check_now(&mut state);
    assert_eq!(asked(&rig)[before..], [UPDATER_URL]);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0"));
    // stable の提案は、設定を切っても消さない
    apply(&mut state, UpdateAction::SetBeta(true));
    apply(&mut state, UpdateAction::SetBeta(false));
    assert_eq!(offered(&state).as_deref(), Some("0.3.0"));
}

#[test]
fn headless_the_beta_setting_never_downgrades_and_stable_returns_when_it_is_newer() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    // 試験版 0.3.0-rc.1 を使っている
    state.update.configure_for_test(
        Some(public_key()),
        "0.3.0-rc.1",
        Some(WINDOWS_INSTALLER),
        Mode::Installer,
    );
    serve_beta(&rig, Some("0.3.0-rc.1"));
    for beta in [false, true] {
        apply(&mut state, UpdateAction::SetBeta(beta));
        // 今の版より古い stable（0.2.0）へは下げない。試験版も同じ版なので、何も勧めない
        check_now(&mut state);
        assert!(state.update.offer().is_none(), "beta={beta}");
        assert_eq!(state.message, "YoluPainter は最新です。");
    }
    // 次の stable が今の版より新しくなったら、設定が切でも入でも勧める（試験版から stable へ戻る）
    serve_stable(&rig, "0.3.0");
    for beta in [false, true] {
        apply(&mut state, UpdateAction::SetBeta(beta));
        check_now(&mut state);
        assert_eq!(offered(&state).as_deref(), Some("0.3.0"), "beta={beta}");
    }
}

#[test]
fn headless_the_beta_setting_is_saved_beside_the_startup_choice_and_read_back() {
    let dir = TempDir::new("beta-save");
    let file = dir.0.join("update.conf");
    let mut state = AppState::new(64, 64);
    let _rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.update.attach_config(file.clone());
    // まだ聞いていない人が試験版だけを入れても、起動時の問いは聞いていないまま
    apply(&mut state, UpdateAction::SetBeta(true));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "use_beta=on\n");
    assert_eq!(state.update.preference(), Preference::Unset);
    // 起動時の確かめの選択を足しても、試験版を落とさない（逆も同じ）
    apply(&mut state, UpdateAction::SetCheckOnStartup(true));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "check_on_startup=on\nuse_beta=on\n"
    );
    let mut next = AppState::new(64, 64);
    let _rig2 = self::rig(&mut next, "0.2.0", Mode::Installer);
    next.update.attach_config(file.clone());
    assert!(next.update.beta());
    assert_eq!(next.update.preference(), Preference::On);
    assert_eq!(beta_check(&next), yolu_app::ui::menu::Check::Checked);
    // 切にすると、旧い版と同じ 1 行になる
    apply(&mut state, UpdateAction::SetBeta(false));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "check_on_startup=on\n"
    );
    let mut again = AppState::new(64, 64);
    let _rig3 = self::rig(&mut again, "0.2.0", Mode::Installer);
    again.update.attach_config(file.clone());
    assert!(!again.update.beta() && again.update.preference() == Preference::On);
    // 何も設定していない状態へ戻ると、ファイルは残らない
    let mut fresh = AppState::new(64, 64);
    let _rig4 = self::rig(&mut fresh, "0.2.0", Mode::Installer);
    let other = dir.0.join("fresh").join("update.conf");
    fresh.update.attach_config(other.clone());
    apply(&mut fresh, UpdateAction::SetBeta(true));
    assert!(other.exists());
    apply(&mut fresh, UpdateAction::SetBeta(false));
    assert!(!other.exists());
    // 保存できなくても、この回の選択は効き、知らせる
    std::fs::remove_dir_all(&dir.0).unwrap();
    std::fs::write(&dir.0, b"a file where the folder should be").unwrap();
    apply(&mut state, UpdateAction::SetBeta(true));
    assert!(state.update.beta());
    assert!(
        state.message.contains("更新の設定を保存できません"),
        "{}",
        state.message
    );
    std::fs::remove_file(&dir.0).unwrap();
}

#[test]
fn headless_an_unusable_beta_place_never_stops_the_stable_update() {
    // 設定を入れた状態で、stable は 0.2.0・今は 0.1.0。試験版の置き場はそれぞれ別の状態にする
    let setup = |beta: Option<Vec<u8>>| {
        let mut state = AppState::new(64, 64);
        let rig = rig(&mut state, "0.2.0", Mode::Installer);
        *rig.server.beta_metadata.lock().unwrap() = beta;
        apply(&mut state, UpdateAction::SetBeta(true));
        (state, rig)
    };
    // まだ 1 つも試験版を出していない（置き場が引けない）
    let (mut state, _rig) = setup(None);
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    assert_eq!(state.message, "YoluPainter 0.2.0 があります。");
    // 別の鍵で署名された試験版は、新しくても受けない
    let v = Version::parse("0.9.0-rc.1").unwrap();
    let name = asset_name(&v, WINDOWS_INSTALLER).unwrap();
    let payload = serde_json::to_string(&Manifest {
        schema: UPDATER_SCHEMA,
        version: v.to_string(),
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
    let (mut state, _rig) = setup(Some(
        serde_json::to_vec(&Envelope { payload, signature }).unwrap(),
    ));
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    // stable が引けないとき: 新しい試験版があればそれを勧める
    let (mut state, rig) = setup(Some(signed_metadata("0.3.0-rc.1", INSTALLER)));
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0-rc.1"));
    // 試験版が新しくなければ、「最新」とは言わず失敗を伝える（引けなかった stable に新しい版があるかもしれない）
    let (mut state, rig) = setup(Some(signed_metadata("0.1.0-rc.1", INSTALLER)));
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    check_now(&mut state);
    assert_eq!(state.message, "更新を確かめられません（通信できません）。");
    assert!(state.update.offer().is_none());
}

#[test]
fn headless_turning_the_beta_setting_off_cancels_and_forgets_a_beta_download() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    apply(&mut state, UpdateAction::SetBeta(true));
    find_update(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0-rc.1"));
    // ダウンロード中に切る: 取り消し、何も残さない
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    assert!(state.update.is_busy());
    apply(&mut state, UpdateAction::SetBeta(false));
    assert!(state.update.progress().unwrap().canceling);
    settle(&mut state);
    assert_eq!(state.message, "ダウンロードを取り消しました。");
    assert!(rig.staging.files().is_empty());
    assert!(state.update.ready().is_none() && state.update.offer().is_none());
    // 落とし済み（あとで）の試験版も、切ると使わない
    rig.server.hold.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::SetBeta(true));
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert!(state.update.is_ready_open());
    apply(&mut state, UpdateAction::Later);
    assert!(state.update.ready().is_some());
    assert_eq!(rig.staging.files().len(), 1);
    apply(&mut state, UpdateAction::SetBeta(false));
    assert!(state.update.ready().is_none() && state.update.offer().is_none());
    assert!(!state.update.is_ready_open());
    // 置き場の試験版のインストーラーも消す（今の版より新しいファイルは、起動時の片付けでも消えない）
    assert!(rig.staging.files().is_empty(), "{:?}", rig.staging.files());
    // stable の落とし済みは、切っても残す（ファイルも）
    check_now(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    apply(&mut state, UpdateAction::Later);
    apply(&mut state, UpdateAction::SetBeta(true));
    apply(&mut state, UpdateAction::SetBeta(false));
    assert!(state.update.ready().is_some() && state.update.offer().is_some());
    assert_eq!(rig.staging.files().len(), 1);
}

#[test]
fn headless_a_cancel_after_the_transfer_ended_still_wins_and_leaves_no_file() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    apply(&mut state, UpdateAction::SetBeta(true));
    find_update(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.3.0-rc.1"));
    // 転送が済んだあと（確かめ・書き込みの間）に設定を切る: 取消は転送の途中でしか見ないが、試験版は受けない
    rig.server.ignore_cancel.store(true, Ordering::Relaxed);
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    assert!(state.update.is_busy());
    apply(&mut state, UpdateAction::SetBeta(false));
    rig.server.hold.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert!(state.update.ready().is_none() && !state.update.is_ready_open());
    assert_eq!(state.message, "ダウンロードを取り消しました。");
    assert!(rig.staging.files().is_empty(), "{:?}", rig.staging.files());
    // 利用者の取消も同じ（stable。準備のウィンドウを開かず、置いたファイルも消す）
    check_now(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    rig.server.hold.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    apply(&mut state, UpdateAction::Cancel);
    rig.server.hold.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert!(state.update.ready().is_none() && !state.update.is_ready_open());
    assert_eq!(state.message, "ダウンロードを取り消しました。");
    assert!(rig.staging.files().is_empty(), "{:?}", rig.staging.files());
    // 取り消さなければ、同じ道で準備のウィンドウが開く
    rig.server.ignore_cancel.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    assert!(state.update.is_ready_open());
    assert_eq!(rig.staging.files().len(), 1);
}

#[test]
fn headless_a_check_that_was_running_when_the_beta_setting_went_off_does_not_bring_in_a_beta() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.1.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    apply(&mut state, UpdateAction::SetBeta(true));
    rig.server.hold_beta.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Check);
    assert!(state.update.is_busy());
    apply(&mut state, UpdateAction::SetBeta(false));
    rig.server.hold_beta.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert!(state.update.offer().is_none());
    assert_eq!(state.message, "YoluPainter は最新です。");
}

#[test]
fn headless_a_check_cut_short_by_the_beta_setting_still_offers_a_newer_stable() {
    // 今は 0.1.0、stable は 0.2.0、試験版は 0.3.0-rc.1。確かめの途中で切にしても、stable の 0.2.0 は勧める（「最新」とは言わない）
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    serve_beta(&rig, Some("0.3.0-rc.1"));
    apply(&mut state, UpdateAction::SetBeta(true));
    rig.server.hold_beta.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Check);
    assert!(state.update.is_busy());
    apply(&mut state, UpdateAction::SetBeta(false));
    rig.server.hold_beta.store(false, Ordering::Relaxed);
    settle(&mut state);
    assert_eq!(offered(&state).as_deref(), Some("0.2.0"));
    assert_eq!(state.message, "YoluPainter 0.2.0 があります。");
    // stable が引けないときは、試験版を捨てたあとも「最新」とは言わず失敗を伝える
    let mut next = AppState::new(64, 64);
    let rig2 = self::rig(&mut next, "0.2.0", Mode::Installer);
    serve_beta(&rig2, Some("0.3.0-rc.1"));
    rig2.server.fail_metadata.store(true, Ordering::Relaxed);
    apply(&mut next, UpdateAction::SetBeta(true));
    rig2.server.hold_beta.store(true, Ordering::Relaxed);
    apply(&mut next, UpdateAction::Check);
    apply(&mut next, UpdateAction::SetBeta(false));
    rig2.server.hold_beta.store(false, Ordering::Relaxed);
    settle(&mut next);
    assert!(next.update.offer().is_none());
    assert_eq!(next.message, "更新を確かめられません（通信できません）。");
}

#[test]
fn headless_the_failure_reason_comes_from_stable_not_from_the_beta_place() {
    // 試験版の置き場が引けなくても（まだ 1 つも出していない間はいつもそう）、stable の失敗の本当の理由を言う
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    apply(&mut state, UpdateAction::SetBeta(true));
    serve_beta(&rig, None);
    // stable の更新情報が壊れている（署名・形式）: 検証を通らない
    *rig.server.metadata.lock().unwrap() = b"not an update file".to_vec();
    check_now(&mut state);
    assert_eq!(
        state.message,
        "更新を確かめられません（検証を通りません）。"
    );
    state.lang = Lang::En;
    check_now(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates (verification failed)."
    );
    // stable が引けない: 通信できない
    state.lang = Lang::Ja;
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    check_now(&mut state);
    assert_eq!(state.message, "更新を確かめられません（通信できません）。");
    assert!(state.update.offer().is_none());
}

/// 設定のウィンドウの「更新」の区分（公開鍵を組み込んだビルドだけ）に、起動時の確かめと試験版を使うの行があり、押すと更新の部品の値が変わって
/// 設定が書かれる（日英。試験版の行にはツールチップ）。ヘルプのメニューからは外してある。公開鍵の無いビルドには区分そのものが出ない。
#[test]
fn the_updates_category_has_the_startup_and_beta_rows_and_the_help_menu_does_not() {
    use egui_kittest::kittest::Queryable;
    use yolu_app::prefs::{Category, PrefsAction};
    for lang in Lang::ALL {
        let dir = TempDir::new("updates-category");
        let mut h = app(1280.0, 800.0, 64);
        let _rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
        h.state_mut()
            .state
            .update
            .attach_config(dir.0.join("update.conf"));
        h.state_mut().state.lang = lang;
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::OpenAt(Category::Updates)));
        h.run();
        let (startup, beta) = (
            lang.pick("起動時に更新を確かめる", "Check for Updates at Startup"),
            lang.pick("試験版を使う", "Use Beta Versions"),
        );
        let role = egui::accesskit::Role::CheckBox;
        let toggled = |h: &egui_kittest::Harness<'static, yolu_app::YoluApp>, label: &str| {
            use egui_kittest::kittest::NodeT;
            format!(
                "{:?}",
                h.get_by_role_and_label(role, label)
                    .accesskit_node()
                    .toggled()
            ) == "Some(True)"
        };
        // 行の順: 起動時の確かめの下に試験版
        let (a, b) = (
            h.get_by_role_and_label(role, startup).rect(),
            h.get_by_role_and_label(role, beta).rect(),
        );
        assert!(
            b.top() > a.bottom() - 1.0 && b.top() < a.bottom() + 30.0,
            "{lang:?}: {a:?} {b:?}"
        );
        assert!(
            !toggled(&h, startup) && !toggled(&h, beta),
            "{lang:?}: 既定は切"
        );
        h.get_by_role_and_label(role, startup).click();
        h.run();
        assert_eq!(h.state().state.update.preference(), Preference::On);
        assert!(toggled(&h, startup));
        h.get_by_role_and_label(role, beta).click();
        h.run();
        assert!(h.state().state.update.beta());
        assert!(toggled(&h, beta));
        let written = std::fs::read_to_string(dir.0.join("update.conf")).unwrap();
        assert!(written.contains("check_on_startup=on"), "{written}");
        assert!(written.contains("use_beta=on"), "{written}");
        // 試験版の行にはツールチップ（日英どちらでも、もう一方の言語の文字を含まない）
        let beta_rect = h.get_by_role_and_label(role, beta).rect();
        move_to(&h, beta_rect.center() + egui::vec2(0.0, 200.0));
        h.run();
        hover_and_wait(&mut h, beta_rect.center());
        let tip = lang.pick("正式版より前の試験版", "Also offers beta versions");
        assert!(
            h.query_by_label_contains(tip).is_some(),
            "{lang:?}: ツールチップが無い"
        );
        // ヘルプのメニューには、もう 2 つとも無い
        let help: Vec<String> = help_labels(&h.state().state);
        assert!(
            !help.iter().any(|l| l == startup || l == beta),
            "{lang:?}: {help:?}"
        );
        // 公開鍵の無いビルドには、区分そのものが出ない
        let mut plain = AppState::new(64, 64);
        plain.lang = lang;
        assert!(!Category::Updates.available(&plain));
        assert!(Category::Updates.items(&plain).is_empty());
        assert!(Category::Updates.available(&h.state().state));
    }
}

#[test]
fn the_status_band_marks_a_beta_version_and_only_a_beta_version() {
    use egui_kittest::kittest::Queryable;
    for (lang, mark) in [(Lang::Ja, "試験版"), (Lang::En, "Beta")] {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.state_mut().state.usage.build = Some("0.4.0 · a1b2c3d".into());
        h.run();
        assert!(
            h.query_by_label(mark).is_none(),
            "{lang:?}: 正式版に印は付かない"
        );
        h.state_mut().state.usage.build = Some("0.4.0-rc.1 · a1b2c3d".into());
        h.run();
        h.get_by_label(mark);
        h.get_by_label("0.4.0-rc.1 · a1b2c3d");
    }
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
    // 検証を通ったファイルが置き場にあり、準備のウィンドウが開く。まだ走らせない（アプリは閉じない）
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
        "更新をダウンロードできません（検証を通りません）。"
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
        "更新をダウンロードできません（通信できません）。"
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
fn headless_metadata_without_this_systems_download_says_so_and_a_broken_one_still_fails_verification(
) {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    // 署名は正しいが、この環境の対象（インストーラー）が載っていない: 検証の失敗ではなく、配布物が無いと言う
    *rig.server.metadata.lock().unwrap() =
        signed_metadata_without("0.2.0", INSTALLER, Some(WINDOWS_INSTALLER));
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert!(state.update.offer().is_none());
    assert_eq!(
        state.message,
        "更新を確かめられません（この環境向けの配布物がありません）。"
    );
    state.lang = Lang::En;
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates (no download for this system)."
    );
    // 署名の合わない（本文を書き換えた）更新情報は、今までどおり検証の失敗
    let mut envelope: Envelope =
        serde_json::from_slice(&signed_metadata("0.2.0", INSTALLER)).unwrap();
    envelope.payload = envelope.payload.replace("0.2.0", "0.9.0");
    *rig.server.metadata.lock().unwrap() = serde_json::to_vec(&envelope).unwrap();
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates (verification failed)."
    );
    // 通信の失敗は、通信できないまま
    rig.server.fail_metadata.store(true, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "Cannot check for updates (connection failed)."
    );
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
    // ダウンロード中に描き始めた: 終わっても、描き終わるまで準備のウィンドウを出さない
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
    // 落とし直さずに、準備のウィンドウがもう一度出る
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
    // まだ名前の無いプロジェクト: 保存先を選ぶウィンドウの頼みが出る。そのウィンドウをやめたら、更新しない
    apply(&mut state, UpdateAction::Run { save: true });
    assert_eq!(state.dialog_request, Some(DialogRequest::SaveAs));
    assert!(rig.launched().is_empty());
    state.update_finish_save();
    assert!(
        rig.launched().is_empty(),
        "ウィンドウがまだ開いている間は待つ"
    );
    state.dialog_request = None; // やめた
    state.update_finish_save();
    assert!(rig.launched().is_empty());
    assert!(!state.quit);
    assert_eq!(state.message, "保存しなかったので、更新しません。");
    assert!(
        state.update.is_ready_open(),
        "更新のウィンドウは残り、やり直せる"
    );
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
    // 保存して更新: その場で保存され、そのまま入れる。保存したことは中身で確かめる（更新時刻の比べは、時計の補正や
    // 時刻の細かさで前後しうるので使わない）
    let before = std::fs::read(dir.0.join("a.ylp")).unwrap();
    state.doc.add_layer("保存して更新").unwrap();
    state.modified = true;
    apply(&mut state, UpdateAction::Run { save: true });
    assert!(!state.modified);
    assert_ne!(
        std::fs::read(dir.0.join("a.ylp")).unwrap(),
        before,
        "保存し直した"
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
            state.message.contains("に保存できません（") && state.message.contains("a.ylp"),
            "{attempt}: {}",
            state.message
        );
        assert!(state.modified && !state.quit && rig.launched().is_empty());
        assert!(
            state.update.is_ready_open(),
            "更新のウィンドウは残り、やり直せる"
        );
        state.update_finish_save();
        assert!(
            state.message.contains("に保存できません（"),
            "{}",
            state.message
        );
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

/// 「保存して更新」は、裏のスレッドの保存が終わるまで入れず、終わって保存できていれば入れる。保存できなかったら、その理由を残して入れない。
#[test]
fn headless_save_and_update_waits_for_a_background_save() {
    for fails in [false, true] {
        let dir = TempDir::new("savebg");
        let mut state = AppState::new(64, 64);
        let rig = rig(&mut state, "0.2.0", Mode::Installer);
        state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
        assert!(!state.modified);
        find_update(&mut state);
        apply(&mut state, UpdateAction::Install);
        settle(&mut state);
        state.save.background = true;
        state.modified = true;
        let hold = state.save.hold_next();
        apply(&mut state, UpdateAction::Run { save: true });
        assert!(state.is_saving(), "{}", state.message);
        // 保存の間は、フレームの終わりごとに見ても入れない
        for _ in 0..3 {
            state.update_finish_save();
        }
        assert!(
            rig.launched().is_empty() && !state.quit,
            "保存の間は入れない"
        );
        if fails {
            // 保存先が外から消えて、保存できない
            std::fs::remove_dir_all(&dir.0).unwrap();
        }
        hold.release();
        state.wait_save();
        state.update_finish_save();
        if fails {
            assert!(
                state.message.contains("に保存できません（"),
                "{}",
                state.message
            );
            assert!(state.modified && !state.quit && rig.launched().is_empty());
            assert!(
                state.update.is_ready_open(),
                "更新のウィンドウは残り、やり直せる"
            );
        } else {
            assert!(!state.modified, "{}", state.message);
            assert_eq!(rig.launched().len(), 1);
            assert!(state.quit && state.update.is_quitting());
        }
    }
}

/// 保存の途中は、更新の入れ替え（「保存せずに更新」「更新して再起動」でも「保存して更新」でも）を断り、インストーラーを起動しない。保存を頼むと
/// 「変更あり」の印は下りて、保存が失敗するまで戻らない。その間に入れ替えを始めると、失敗して「変更あり」に戻っても、更新のために終わるので
/// 確認なしで閉じて変更を失う。
#[test]
fn headless_installing_is_refused_while_a_save_is_running() {
    for (save, english) in [(false, false), (true, false), (false, true)] {
        let dir = TempDir::new("savebusy");
        let mut state = AppState::new_in(64, 64, if english { Lang::En } else { Lang::Ja });
        let rig = rig(&mut state, "0.2.0", Mode::Installer);
        state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
        find_update(&mut state);
        apply(&mut state, UpdateAction::Install);
        settle(&mut state);
        state.save.background = true;
        state.modified = true;
        let hold = state.save.hold_next();
        state.apply(Action::SaveProject);
        assert!(state.is_saving() && !state.modified, "{}", state.message);
        assert!(
            state.shows_modified(),
            "保存の結果が出るまで、頼む前の印を見せる"
        );
        apply(&mut state, UpdateAction::Run { save });
        assert!(
            rig.launched().is_empty(),
            "保存の間はインストーラーを起動しない"
        );
        assert!(!state.quit && !state.update.is_quitting());
        assert!(
            state.update.is_ready_open(),
            "更新のウィンドウは残り、保存の後にやり直せる"
        );
        assert!(
            state.message.contains(if english {
                "A save is in progress"
            } else {
                "保存の途中です"
            }),
            "{}",
            state.message
        );
        // 保存の間は、フレームの終わりごとに見ても入れない
        state.update_finish_save();
        assert!(rig.launched().is_empty() && !state.quit);
        hold.release();
        state.wait_save();
        assert!(!state.modified, "{}", state.message);
        // 保存が終わってからなら入れる
        apply(&mut state, UpdateAction::Run { save: false });
        assert_eq!(rig.launched().len(), 1, "{}", state.message);
        assert!(state.quit && state.update.is_quitting());
    }
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

// ───────── 別のウィンドウが開いている ─────────

const BLOCKED_JA: &str = "ほかの YoluPainter が開いています";

#[test]
fn headless_another_running_window_stops_the_update_and_keeps_the_download() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    let other = Arc::new(AtomicBool::new(true));
    let seen = other.clone();
    state
        .update
        .set_other_instance_for_test(Arc::new(move || seen.load(Ordering::Relaxed)));
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    let name = "yolupainter-0.2.0-x86_64-pc-windows-msvc-setup.exe";
    let calls = rig.calls();
    // 断る: 走らせず、アプリは閉じず、落としたインストーラーも準備のウィンドウも残る（短い理由だけ。使い方の説明にしない）
    apply(&mut state, UpdateAction::Run { save: false });
    assert!(rig.launched().is_empty());
    assert!(!state.quit && !state.update.is_quitting());
    assert_eq!(state.message, BLOCKED_JA);
    assert!(state.update.is_ready_open() && state.update.is_blocked());
    assert_eq!(rig.staging.files(), [name]);
    // 閉じてからもう一度押せば、落とし直さずにすぐ入る
    other.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Run { save: false });
    assert_eq!(rig.launched(), [rig.staging.0.join(name)]);
    assert!(state.quit && state.update.is_quitting());
    assert!(!state.update.is_blocked());
    assert_eq!(rig.calls(), calls, "通信し直さない");
}

#[test]
fn headless_the_refusal_comes_before_saving_and_is_told_in_english_too() {
    let dir = TempDir::new("blocked-save");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    state.lang = Lang::En;
    state.update.set_other_instance_for_test(Arc::new(|| true));
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    state.modified = true;
    // 更新時刻を前にしておく（書き直されたら今の時刻になって食い違う。時刻の粒度に頼って待たない）
    let before = common::tmp::backdate(&dir.0.join("a.ylp"));
    // 更新が始まらないのに、保存はしない（保存先のウィンドウも出さない）
    apply(&mut state, UpdateAction::Run { save: true });
    assert!(state.modified, "保存していない変更は、そのまま");
    assert_eq!(
        std::fs::metadata(dir.0.join("a.ylp"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    assert!(state.dialog_request.is_none() && rig.launched().is_empty() && !state.quit);
    assert_eq!(state.message, "Another YoluPainter is running");
    assert!(state.update.is_ready_open());
    // 「あとで」で理由の表示も消える
    apply(&mut state, UpdateAction::Later);
    assert!(!state.update.is_blocked() && !state.update.is_ready_open());
}

#[test]
fn headless_a_window_opened_while_the_save_dialog_was_up_is_caught_before_launching() {
    let dir = TempDir::new("blocked-late");
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Installer);
    let other = Arc::new(AtomicBool::new(false));
    let seen = other.clone();
    state
        .update
        .set_other_instance_for_test(Arc::new(move || seen.load(Ordering::Relaxed)));
    find_update(&mut state);
    apply(&mut state, UpdateAction::Install);
    settle(&mut state);
    state.modified = true;
    // 名前の無いプロジェクト: 保存先を選ぶウィンドウが開いている間に、別のウィンドウが開く
    apply(&mut state, UpdateAction::Run { save: true });
    assert_eq!(state.dialog_request, Some(DialogRequest::SaveAs));
    other.store(true, Ordering::Relaxed);
    state.dialog_request = None;
    state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
    assert!(!state.modified);
    state.update_finish_save();
    // 保存は済んだが、走らせない。落としたインストーラーと準備のウィンドウは残る
    assert!(rig.launched().is_empty() && !state.quit);
    assert_eq!(state.message, BLOCKED_JA);
    assert!(state.update.is_ready_open() && state.update.is_blocked());
    assert_eq!(rig.staging.files().len(), 1);
    other.store(false, Ordering::Relaxed);
    apply(&mut state, UpdateAction::Run { save: false });
    assert_eq!(rig.launched().len(), 1);
    assert!(state.quit);
}

#[test]
fn the_ready_window_gives_the_reason_while_another_window_is_open() {
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    {
        let state = &mut h.state_mut().state;
        state.update.set_other_instance_for_test(Arc::new(|| true));
        find_update(state);
        apply(state, UpdateAction::Install);
        settle(state);
    }
    h.run();
    {
        use egui_kittest::kittest::Queryable;
        assert!(h.query_by_label(BLOCKED_JA).is_none(), "押すまでは出さない");
    }
    click_label(&mut h, "更新して再起動");
    h.run();
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label(BLOCKED_JA);
        h.get_by_label("更新して再起動");
    }
    assert!(rig.launched().is_empty() && !h.state().state.quit);
    // 別のウィンドウを閉じてからもう一度押すと入る
    h.state_mut()
        .state
        .update
        .set_other_instance_for_test(Arc::new(|| false));
    click_label(&mut h, "更新して再起動");
    h.run();
    assert_eq!(rig.launched().len(), 1);
    assert!(h.state().state.quit);
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

#[test]
fn headless_macos_announces_the_new_version_and_opens_the_page_without_downloading() {
    let mut state = AppState::new(64, 64);
    let rig = rig(&mut state, "0.2.0", Mode::Page);
    // mac のアプリが探す鍵（universal の配布物）で確かめる
    state
        .update
        .configure_for_test(Some(public_key()), "0.1.0", Some(MACOS_ARCHIVE), Mode::Page);
    find_update(&mut state);
    assert_eq!(state.update.mode(), Mode::Page);
    assert_eq!(help_labels(&state)[0], "YoluPainter 0.2.0 のリリースを開く");
    state.lang = Lang::En;
    assert_eq!(help_labels(&state)[0], "Open the YoluPainter 0.2.0 release");
    apply(&mut state, UpdateAction::Install);
    // 落とさず・走らせず・置き場も使わず、その版のページを開く（更新情報の 1 回の取得だけ）
    assert_eq!(
        *rig.opened.lock().unwrap(),
        [release_page(&Version::new(0, 2, 0))]
    );
    assert_eq!(rig.calls(), 1);
    assert!(rig.launched().is_empty() && rig.staging.files().is_empty());
    assert!(state.update.ready().is_none() && !state.update.is_ready_open() && !state.quit);
    assert_eq!(
        state.update.offer().map(|offer| offer.version.clone()),
        Some(Version::new(0, 2, 0))
    );
    // その版が mac の配布物を載せていなければ、検証の失敗ではなく「この環境向けの配布物がありません」
    *rig.server.metadata.lock().unwrap() =
        signed_metadata_without("0.3.0", INSTALLER, Some(MACOS_ARCHIVE));
    state.lang = Lang::Ja;
    apply(&mut state, UpdateAction::Check);
    settle(&mut state);
    assert_eq!(
        state.message,
        "更新を確かめられません（この環境向けの配布物がありません）。"
    );
}

// ───────── 画面（ウィンドウ・メニュー・印） ─────────

/// ウィンドウの中だけを撮って、正解の絵と比べる。
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

/// ウィンドウが出そろうまで数フレーム進める（保存の途中は描き直しを頼み続けるので、`run` は使えない）。
fn steps(h: &mut Harness<'_, YoluApp>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

/// 保存の途中は、保存を頼んだ後の「変更あり」の印が下りているが、更新のウィンドウは頼む前の印のまま（保存していない変更がある形）で見せる。
/// 保存が終わるまで、どのボタンもインストーラーを起動しない。
#[test]
fn the_ready_window_keeps_the_unsaved_form_while_a_save_runs_and_launches_nothing() {
    let dir = TempDir::new("readysaving");
    let mut h = app(1280.0, 800.0, 64);
    let rig = rig(&mut h.state_mut().state, "0.2.0", Mode::Installer);
    let hold;
    {
        let state = &mut h.state_mut().state;
        state.apply(Action::SaveProjectAs(dir.0.join("a.ylp")));
        find_update(state);
        apply(state, UpdateAction::Install);
        settle(state);
        state.save.background = true;
        state.modified = true;
        hold = state.save.hold_next();
        state.apply(Action::SaveProject);
        assert!(state.is_saving() && !state.modified, "{}", state.message);
    }
    steps(&mut h, 6);
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("保存せずに更新");
        h.get_by_label("保存して更新");
        assert!(h.query_by_label("更新して再起動").is_none());
    }
    click_label(&mut h, "保存せずに更新");
    steps(&mut h, 6);
    assert!(
        rig.launched().is_empty() && !h.state().state.quit,
        "{}",
        h.state().state.message
    );
    click_label(&mut h, "保存して更新");
    steps(&mut h, 6);
    assert!(
        rig.launched().is_empty() && !h.state().state.quit,
        "{}",
        h.state().state.message
    );
    hold.release();
    h.state_mut().state.wait_save();
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
    // 持っておく（式の中で作ると、すぐ消えて、あとで設定を書くときに同じ名前のフォルダが作り直されて残る）
    let config_dir = TempDir::new("menu");
    h.state_mut()
        .state
        .update
        .attach_config(config_dir.0.join("update.conf"));
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
