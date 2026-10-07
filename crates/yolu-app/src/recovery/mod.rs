//! 落ちても失わない: 復旧用の世代の書き置きと、起動時の復旧。
//!
//! 変更があってから一定の時間（`RecoverySettings::interval_seconds`。連続して描いているときは前の書き置きから）か、終わった
//! ストロークの数で、描いていない区切りに文書全体の写し（`Document::capture_snapshot`。タイルは共有）を取り、別のスレッド
//! （`writer`）が `.ylp` と同じ形に詰めて世代として書く（`yolu_io::GenerationStore`。`current` を最後に置換）。主のスレッド
//! が使うのは写しを取る時間だけで、書き置きの失敗は状態の帯に短い理由を出すだけで描くのを止めない。保存した `.ylp` と
//! 同じ（変更なし）ときは書かない。
//!
//! 1 回の起動が 1 つのプール（`pool`）を持つ。落ちると印（`session.lock`）が残るので、次の起動は復旧の窓（`window`）で世代の
//! 一覧から開く・捨てるを選ばせる。開いたものは「名称未設定（復旧）」で、元の `.ylp` には書かない。正しく閉じると印を消し、
//! 世代は閉じたプールの合計で設定の数だけ残す。
//!
//! ディスクの使いすぎを防ぐ歯止めが 2 つある。1 つは使う量の上限（`quota`。利用者が選ぶ。超えたぶんは古い世代から消し、この実行の
//! 最新と落ちた実行ごとの最新は残す）、もう 1 つは書く前の空きの守り（`space`。書くと空きが残す量を割るなら、書かずに理由を出す。
//! 描くのは止めない）。使っている量は復旧の窓に出す。
//!
//! 試験では `RecoveryState::enable` に一時フォルダを渡して使う（何もしなければ復旧は動かず、ディスクに触れない）。

mod capture;
mod pool;
mod quota;
mod settings;
mod space;
mod text;
pub mod window;
mod writer;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yolu_io::{Fault, StoreError};

pub use capture::Fingerprint;
pub use pool::{Kind as PoolKind, Row};
pub use quota::{usage, Limits, Trimmed, Usage, CRASHED_KEEP_DAYS};
pub use settings::{
    DiskBudget, IoReason, Problem, RecoverySettings, DISK_GIB_RANGE, INTERVAL_RANGE, KEEP_RANGE,
    MAX_STROKES,
};
pub use space::{reserve as space_reserve, system_probe, DiskSpace, SpaceProbe};
pub use text::recovered_name;
pub(crate) use writer::Waiter;

use crate::jobs::JobSpec;
use crate::state::{Action, AppState};

/// 復旧の失敗。画面は種類から短い理由を作る（`Lang::recovery_error`）。
#[derive(Debug)]
pub enum RecoveryError {
    Store(StoreError),
    Project(yolu_io::Error),
    Io(std::io::Error),
    /// 材料を正本にする途中の理由（言語つきの文）。
    Text(String),
    /// 置き場の根の直下のプールではない。
    NotPool,
    /// 書き手のスレッドが内部の失敗で止まった（画面は止めず、次の頼みでやり直す）。
    Panicked,
}
impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::Project(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::Text(t) => f.write_str(t),
            Self::NotPool => f.write_str("復旧の置き場の中の世代ではありません"),
            Self::Panicked => f.write_str("書き込みのスレッドが内部の失敗で止まりました"),
        }
    }
}
impl std::error::Error for RecoveryError {}
impl From<StoreError> for RecoveryError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<yolu_io::Error> for RecoveryError {
    fn from(e: yolu_io::Error) -> Self {
        Self::Project(e)
    }
}
impl From<std::io::Error> for RecoveryError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// 復旧の窓・メニューからの操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// 窓を開く（一覧を読み直す）。
    OpenWindow,
    CloseWindow,
    /// 一覧の行を選ぶ。
    Select(usize),
    /// 選んだ世代を開く（保存していない変更があれば、捨ててよいか聞いてから）。
    Open,
    /// 選んだ世代を捨てる（確かめの窓を出す）。
    Discard,
    ConfirmDiscard,
    CancelDiscard,
    /// 書き置きの間隔（秒）・残す世代の数を替える（設定のファイルへ書く）。
    SetInterval(u32),
    SetKeep(u32),
    /// 使うディスクの量を替える（設定のファイルへ書き、超えていれば古い世代から消す）。
    SetDisk(DiskBudget),
    /// 窓の「詳しく」を開く・閉じる（窓の中だけの状態。設定には書かない）。
    DiskDetails(bool),
}

/// 世代を開く頼み（今の変更を捨ててよいか確かめたあとで開く）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub pool: PathBuf,
    pub id: String,
}

/// この実行の復旧（置き場・書き手・変わったかの見張り）。
struct Active {
    root: PathBuf,
    session: pool::Session,
    writer: writer::Writer,
    /// 最後に確定した書き置きの札。
    recorded: Option<Fingerprint>,
    /// 最後に頼んだ書き置きの札（実行中か待ち。失敗したら None に戻す）。
    submitted: Option<Fingerprint>,
    /// 前のフレームの札・ストロークの最中だったか・変更ありだったか（区切りの検出）。
    last_seen: Option<Fingerprint>,
    /// 今のストロークを描き始める前の札。
    before_stroke: Option<Fingerprint>,
    was_stroking: bool,
    was_modified: bool,
    /// 保存していない変更が初めて見つかった時刻と、最後に頼んだ時刻。
    dirty_since: Option<Instant>,
    last_attempt: Option<Instant>,
    strokes_since: u32,
    /// 保存・開く・新規のたびに増える（前の世代の結果を保存済みの印に使わない）。
    epoch: u64,
    /// 時間を待たずに書く（フォーカスを失った・終わる前）。
    force: bool,
}

/// 復旧の窓の確かめ（キーの割り当てを止める）。
pub(crate) const JOB: JobSpec = JobSpec {
    modal: Some(|app| {
        app.recovery
            .window
            .as_ref()
            .is_some_and(|w| w.confirm.is_some())
    }),
    ..JobSpec::new("recovery", crate::jobs::never)
};

/// アプリの状態の中の、復旧の状態。
#[derive(Default)]
pub struct RecoveryState {
    settings: RecoverySettings,
    active: Option<Active>,
    pub window: Option<window::WindowState>,
    open_request: Option<OpenRequest>,
    /// 復旧から開いた文書の、元の .ylp の名前（以降の書き置きの一覧に出す）。
    recovered_from: Option<String>,
    /// 書き置きが失敗している（次に成功したら知らせる）。
    failed: bool,
    /// 書き置きが確定した数と、最後の書き込みにかかった時間（試験・診断用）。
    checkpoints: u64,
    last_millis: f64,
    /// 書き置きのあとの上限の整理で、これまでに消した世代の数と、最後の整理の結果（試験・診断用）。
    trimmed_total: usize,
    last_trimmed: Trimmed,
    /// 設定のファイル（変えたら書く）。
    settings_path: Option<PathBuf>,
    /// 設定のファイルを読めなかった。利用者が選んだ世代の数が分からないので、選び直すまで世代を整理せず、設定のファイルに
    /// 書かない（既定の数で、選んだ世代を消さない・選んだ設定を既定で上書きしない）。
    settings_unreadable: bool,
    /// 読めなかった設定のまま、利用者が窓で世代の数を選んだ（この実行のあいだ、その数で整理する）。
    keep_chosen: bool,
    /// 読めなかった設定のまま、利用者が窓で使う量を選んだ（この実行のあいだ、その量で整理する）。読めないあいだは、選んだ量が
    /// 分からないので、ディスクの上限では消さない（空きの守りは、設定に関わらず働く）。
    disk_chosen: bool,
    fault: Option<Fault>,
    /// 試験用: 書き込みの予算（1 エントリ・合計、バイト数）。
    budget: Option<(u64, u64)>,
    /// 試験用: 空きを偽る口（`None` は OS に聞く）と、上限を直に指定する値。
    probe: Option<SpaceProbe>,
    cap_override: Option<u64>,
}

impl RecoveryState {
    pub fn settings(&self) -> &RecoverySettings {
        &self.settings
    }
    pub fn is_enabled(&self) -> bool {
        self.active.is_some()
    }
    /// 置き場の根。
    pub fn root(&self) -> Option<&Path> {
        self.active.as_ref().map(|a| a.root.as_path())
    }
    /// この実行のプール。
    pub fn session_dir(&self) -> Option<&Path> {
        self.active.as_ref().map(|a| a.session.dir())
    }
    /// 書き置きが確定した数。
    pub fn checkpoints(&self) -> u64 {
        self.checkpoints
    }
    /// 書き置きのあとの、ディスクの上限の整理で消した世代の数（この実行の合計）。
    pub fn trimmed_generations(&self) -> usize {
        self.trimmed_total
    }
    /// 最後の書き置きのあとの、上限の整理の結果。
    pub fn last_trimmed(&self) -> Trimmed {
        self.last_trimmed
    }
    /// 最後の書き置きの、別のスレッドでの所要時間（ミリ秒。主のスレッドは止めない）。
    pub fn last_write_millis(&self) -> f64 {
        self.last_millis
    }
    /// 書き込みが動いていない。
    pub fn is_idle(&self) -> bool {
        self.active.as_ref().is_none_or(|a| a.writer.is_idle())
    }
    /// 保存していない作業の世代がある印を立てているか（落ちたとき、次の起動が知らせる）。
    pub fn is_marked_dirty(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.session.is_dirty())
    }
    /// 世代を整理する数。設定のファイルを読めず、数も選んでいないときは None（整理しない）。
    fn keep(&self) -> Option<usize> {
        (!self.settings_unreadable || self.keep_chosen)
            .then_some(self.settings.generations_to_keep as usize)
    }
    /// 空きの確かめと上限のもと。
    fn limits(&self) -> Limits {
        Limits {
            budget: self.settings.disk,
            probe: self.probe.clone().unwrap_or_else(system_probe),
            cap_override: self.cap_override,
        }
    }
    /// ディスクの上限で整理するときのもと。設定のファイルを読めず、量を選んでいないときは None（整理しない）。
    fn quota_limits(&self) -> Option<Limits> {
        (!self.settings_unreadable || self.disk_chosen).then(|| self.limits())
    }
    /// 試験用: 空きを偽る（`None` で OS に聞く）。書く前の守りと、自動の上限と、窓の表示が使う。
    pub fn set_space_probe(&mut self, probe: Option<SpaceProbe>) {
        self.probe = probe;
    }
    /// 試験用: 上限のバイト数を直に指定する（`None` で選びに従う）。
    pub fn set_disk_cap(&mut self, cap: Option<u64>) {
        self.cap_override = cap;
    }
    /// いま復旧が使っている量（置き場が動いていなければ None）。
    pub fn usage(&self) -> Option<Usage> {
        let a = self.active.as_ref()?;
        Some(quota::usage(&a.root, Some(a.session.dir())))
    }
    /// いまの上限（バイト）。置き場が動いていなければ None。
    pub fn disk_cap(&self) -> Option<u64> {
        let a = self.active.as_ref()?;
        let used = quota::usage(&a.root, Some(a.session.dir())).total();
        Some(self.limits().cap(&a.root, used))
    }
    /// 試験用: 書き込みの予算を小さくする（1 エントリの上限・合計の上限、バイト数）。
    pub fn set_budget(&mut self, budget: Option<(u64, u64)>) {
        self.budget = budget;
        if let Some(a) = self.active.as_mut() {
            a.writer.set_budget(budget);
        }
    }
    /// 試験用の障害の注入（`snapshot`・`file:…`・`verified`・`generation-renamed`・`before-pointer`・`after-pointer`）。
    pub fn set_fault(&mut self, fault: Option<Fault>) {
        self.fault = fault.clone();
        if let Some(a) = self.active.as_mut() {
            a.writer.set_fault(fault);
        }
    }
    /// 設定のファイル（画面で間隔・世代の数を替えたとき書く先）。`start_default` が決めるが、試験は直に渡せる。
    pub fn set_settings_path(&mut self, path: Option<PathBuf>) {
        self.settings_path = path;
    }
    pub fn take_open_request(&mut self) -> Option<OpenRequest> {
        self.open_request.take()
    }

    /// 復旧を始める（置き場の根・設定）。前の実行が落ちていて保存していない作業の世代が残っていれば、復旧の窓を開いた
    /// 状態にする。始められなければ Err（復旧は動かない）。返すのは、前の実行の後片付けで気づいたこと。
    pub fn enable(
        &mut self,
        root: PathBuf,
        settings: RecoverySettings,
    ) -> Result<Vec<Problem>, RecoveryError> {
        self.settings_unreadable = false;
        self.keep_chosen = false;
        self.disk_chosen = false;
        self.enable_with(root, settings)
    }

    fn enable_with(
        &mut self,
        root: PathBuf,
        settings: RecoverySettings,
    ) -> Result<Vec<Problem>, RecoveryError> {
        self.settings = settings;
        let started = pool::start(&root, self.keep(), self.quota_limits().as_ref())?;
        let mut problems = Vec::new();
        if let Some(e) = &started.skipped {
            problems.push(Problem::PreviousRun(IoReason::of(e)));
        }
        let mut writer = writer::Writer::new();
        writer.set_fault(self.fault.clone());
        writer.set_budget(self.budget);
        self.active = Some(Active {
            root,
            session: started.session,
            writer,
            recorded: None,
            submitted: None,
            last_seen: None,
            before_stroke: None,
            was_stroking: false,
            was_modified: false,
            dirty_since: None,
            last_attempt: None,
            strokes_since: 0,
            epoch: 0,
            force: false,
        });
        if !started.crashed.is_empty() {
            self.window = Some(window::WindowState::default());
            self.refresh_window();
        }
        Ok(problems)
    }

    /// 起動の設定（設定のファイルと置き場）から始める。設定のフォルダが決まらない・始められないときは復旧を使わず、理由を返す。
    pub fn start_default(&mut self) -> Result<Vec<Problem>, RecoveryError> {
        self.start_from(RecoverySettings::path())
    }

    /// `start_default` の、設定のファイル（`recovery.conf`）の場所を渡せる形。置き場の既定は、そのファイルと同じフォルダの
    /// `recovery`。ファイルを読めないときは、既定の間隔で動くが、世代は整理せず（選んだ数が分からない）、ファイルにも書かない
    /// （選んだ置き場・数を既定で上書きしない）。理由は `Problem::Unreadable` で返す。
    pub fn start_from(&mut self, conf: Option<PathBuf>) -> Result<Vec<Problem>, RecoveryError> {
        self.settings_path = conf.clone();
        self.settings_unreadable = false;
        self.keep_chosen = false;
        self.disk_chosen = false;
        let mut problems = Vec::new();
        let settings = match conf.as_deref().map(RecoverySettings::load) {
            Some(Ok((settings, found))) => {
                problems = found;
                settings
            }
            Some(Err(e)) => {
                self.settings_unreadable = true;
                self.settings_path = None;
                problems.push(Problem::Unreadable(IoReason::of(&e)));
                RecoverySettings::default()
            }
            None => RecoverySettings::default(),
        };
        let root = settings.root_beside(conf.as_deref()).ok_or_else(|| {
            RecoveryError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "settings folder unknown",
            ))
        })?;
        problems.extend(self.enable_with(root, settings)?);
        Ok(problems)
    }

    /// 一覧を読み直す（窓が開いていれば）。選んでいた世代が残っていればそのまま、無ければ新しい読める世代を選ぶ。
    pub(crate) fn refresh_window(&mut self) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let rows = pool::list(&active.root, Some(active.session.dir()));
        let usage = quota::usage(&active.root, Some(active.session.dir()));
        let limits = self.limits();
        let cap = limits.cap(&active.root, usage.total());
        let free = limits.space(&active.root).map(|d| d.available);
        if let Some(w) = self.window.as_mut() {
            w.set_rows(rows);
            w.set_usage(usage, cap, free);
        }
    }

    /// 次にフレームを起こすまでの時間（変更があって書くのを待っているとき・書き込みが動いているとき）。
    fn wakeup(&self, now: Instant) -> Option<Duration> {
        let a = self.active.as_ref()?;
        if !a.writer.is_idle() {
            return Some(Duration::from_millis(100));
        }
        let since = a.dirty_since?;
        let base = a.last_attempt.map_or(since, |l| l.max(since));
        let due = base + Duration::from_secs(self.settings.interval_seconds as u64);
        Some(
            due.saturating_duration_since(now)
                .max(Duration::from_millis(50)),
        )
    }
}

impl AppState {
    /// 1 フレームの見張り（書き込みの結果を受ける・変わったか見る・書く頃なら材料を取って頼む）。返すのは次に起こしたい時間。
    pub fn recovery_tick(&mut self) -> Option<Duration> {
        self.recovery_tick_at(Instant::now())
    }

    /// `recovery_tick` の、時刻を渡せる形（試験用）。
    pub fn recovery_tick_at(&mut self, now: Instant) -> Option<Duration> {
        self.recovery.active.as_ref()?;
        self.recovery_poll();
        // 保存の間は見張りを止める。保存は開いた .ylp を置き換える（書き置きの読みと重ねない）うえ、「保存していない変更」の印
        // （`modified`）は保存の結果が出るまで確かでない（保存の頼みが下ろして、保存の間の編集だけを立て直す）
        if self.is_saving() {
            return self.recovery.wakeup(now);
        }
        let modified = self.modified;
        let stroking = self.is_stroking();
        let fingerprint = capture::fingerprint(self);
        {
            let a = self.recovery.active.as_mut().expect("上で確かめた");
            // 保存・開く・新規で変更なしに戻った: 保存した .ylp と同じなので、書かない。落ちても知らせない印へ
            if a.was_modified && !modified {
                self.recovery.recovered_from = None;
                a.epoch += 1;
                a.recorded = None;
                a.submitted = None;
                a.dirty_since = None;
                a.strokes_since = 0;
                a.force = false;
                let _ = a.session.mark(false);
            }
            a.was_modified = modified;
            // 描き始めたときの札を覚え、描き終えて文書が変わっていたら、終わったストロークを 1 つ数える
            if !a.was_stroking && stroking {
                a.before_stroke = a.last_seen.clone();
            }
            if a.was_stroking && !stroking && a.before_stroke.as_ref() != Some(&fingerprint) {
                a.strokes_since += 1;
            }
            a.was_stroking = stroking;
            a.last_seen = Some(fingerprint.clone());
        }
        let interval = Duration::from_secs(self.recovery.settings.interval_seconds as u64);
        let strokes = self.recovery.settings.strokes_between;
        let a = self.recovery.active.as_mut().expect("上で確かめた");
        // 書くものが無いときは、「時間を待たずに書く」の頼み（フォーカスを失った）も下ろす。残すと、戻ってきて最初に描いた
        // ストロークの終わりで、間隔を待たずに書いてしまう
        if !stroking && !modified {
            a.dirty_since = None;
            a.force = false;
            return self.recovery.wakeup(now);
        }
        // 書き置きに入っていない変更。描いている最中でも、変わった時刻は覚える（取るのは描き終えてから）
        if a.recorded.as_ref() == Some(&fingerprint) || a.submitted.as_ref() == Some(&fingerprint) {
            a.force = false;
            return self.recovery.wakeup(now);
        }
        let since = *a.dirty_since.get_or_insert(now);
        if stroking {
            return self.recovery.wakeup(now);
        }
        let base = a.last_attempt.map_or(since, |l| l.max(since));
        let due = now.saturating_duration_since(base) >= interval
            || (strokes > 0 && a.strokes_since >= strokes)
            || a.force;
        if due {
            self.recovery_submit(now);
        }
        self.recovery.wakeup(now)
    }

    /// 材料を取って書き手へ頼む（取れない区切り — ストロークの最中・取り込みの途中 — なら、何もせず次のフレームで）。
    fn recovery_submit(&mut self, now: Instant) {
        let recovered_from = self.recovery.recovered_from.clone();
        let captured = match capture::capture(self, recovered_from.as_deref()) {
            Ok(captured) => captured,
            Err(capture::Refusal::NothingToWrite) => {
                // 書けるセットが無い（どのセットも保存したことが無く読めない）。書き置きは作らず、同じ札のうちは頼み直さない
                // （保存のときに、入れなかったセットを知らせる）
                let fingerprint = capture::fingerprint(self);
                let a = self.recovery.active.as_mut().expect("呼ぶ前に確かめた");
                a.submitted = Some(fingerprint);
                a.last_attempt = Some(now);
                a.dirty_since = None;
                a.strokes_since = 0;
                a.force = false;
                return;
            }
            // 描いている最中・取り込みの途中は、次のフレームで取り直す
            Err(_) => return,
        };
        let keep = self.recovery.keep();
        let limits = self.recovery.limits();
        let enforce = self.recovery.quota_limits().is_some();
        let a = self.recovery.active.as_mut().expect("呼ぶ前に確かめた");
        a.submitted = Some(captured.fingerprint.clone());
        a.last_attempt = Some(now);
        a.dirty_since = None;
        a.strokes_since = 0;
        a.force = false;
        a.writer.submit(writer::Request {
            capture: captured,
            root: a.session.dir().to_path_buf(),
            keep,
            epoch: a.epoch,
            home: a.root.clone(),
            limits,
            enforce,
        });
    }

    /// 書き込みの結果を受ける。確定したら札を覚え、保存していない作業の世代があると印に書く。失敗したら理由を状態の帯に出す
    /// （描くのは止めない。次の頼みでやり直す）。失敗のあとに成功したら、戻ったことを知らせる。
    fn recovery_poll(&mut self) {
        let mut landed = false;
        while let Some(a) = self.recovery.active.as_mut() {
            let Some(outcome) = a.writer.take() else {
                break;
            };
            match outcome.result {
                Ok(_) => {
                    if outcome.epoch == a.epoch {
                        a.recorded = Some(outcome.fingerprint);
                        if self.modified {
                            let _ = a.session.mark(true);
                        }
                    }
                    self.recovery.checkpoints += 1;
                    self.recovery.last_millis = outcome.millis;
                    self.recovery.trimmed_total += outcome.trimmed.generations;
                    self.recovery.last_trimmed = outcome.trimmed;
                    landed = true;
                    if self.recovery.failed {
                        self.recovery.failed = false;
                        self.message = self.lang.recovery_working_again().into();
                    }
                }
                Err(error) => {
                    a.submitted = None;
                    self.recovery.failed = true;
                    self.message = self.lang.recovery_failed(&error);
                }
            }
        }
        // 窓が開いていれば、増えた世代を一覧に足す
        if landed {
            self.recovery.refresh_window();
        }
    }

    /// フォーカスを失ったなど: 時間を待たずに、次の見張りで書く。
    pub fn recovery_request_flush(&mut self) {
        if let Some(a) = self.recovery.active.as_mut() {
            a.force = true;
        }
    }

    /// 動いている書き込みの終わりを、ほかのスレッドから待てる口（復旧が動いていなければ None）。保存が、開いた .ylp を置き換える前に、
    /// 書き置きの読みが終わるのを待つために使う。
    pub(crate) fn recovery_waiter(&self) -> Option<writer::Waiter> {
        self.recovery.active.as_ref().map(|a| a.writer.waiter())
    }

    /// 動いている書き込みが終わるまで待って、結果を受ける（試験。描画の途中では使わない）。期限が無いので、終わる前には
    /// `recovery_wait_within` を使う。
    pub fn recovery_wait(&mut self) {
        if let Some(a) = self.recovery.active.as_ref() {
            a.writer.wait();
        }
        self.recovery_poll();
    }

    /// `recovery_wait` の、待つのを `wait` までにする形（終わる前の。遅いディスク・止まったネットワークドライブで終了が固まらないように）。
    /// 終わっていれば true。間に合わなかった書き込みは走ったままで、確定するまで前の世代が残る（置換は最後の 1 回）。
    pub fn recovery_wait_within(&mut self, wait: Duration) -> bool {
        let done = self
            .recovery
            .active
            .as_ref()
            .is_none_or(|a| a.writer.wait_until(Some(Instant::now() + wait)));
        self.recovery_poll();
        done
    }

    /// いまの状態を（変わっていれば）書いて、書き込みが終わるまで待つ。返すのは、いまの状態が書き置きに入っているか
    /// （保存した .ylp と同じ・書き置きが同じならtrue。描いている最中・書き込みの失敗は false）。
    pub fn recovery_flush(&mut self) -> bool {
        self.recovery_flush_until(None)
    }

    /// `recovery_flush` の、待つのを `wait` までにする形（GPU の装置を失ったなど、急いで書き置きを取るとき）。
    pub fn recovery_flush_within(&mut self, wait: Duration) -> bool {
        self.recovery_flush_until(Some(Instant::now() + wait))
    }

    /// `recovery_flush` の、待つ期限を付けられる形（終わるときは、遅いディスクで固まらないように期限を付ける）。
    fn recovery_flush_until(&mut self, deadline: Option<Instant>) -> bool {
        if self.recovery.active.is_none() {
            return true;
        }
        self.recovery_request_flush();
        for _ in 0..2 {
            self.recovery_tick_at(Instant::now());
            if let Some(a) = self.recovery.active.as_ref() {
                if !a.writer.wait_until(deadline) {
                    return false;
                }
            }
            self.recovery_poll();
            if self.recovery_is_current() {
                return true;
            }
            self.recovery_request_flush();
        }
        self.recovery_is_current()
    }

    fn recovery_is_current(&self) -> bool {
        let Some(a) = self.recovery.active.as_ref() else {
            return true;
        };
        !self.modified || a.recorded.as_ref() == Some(&capture::fingerprint(self))
    }

    /// 正しく閉じる: 変更があれば最後の世代を書いて待ち、印を消す（世代は設定の数だけ残す）。
    pub fn recovery_shutdown(&mut self) {
        if self.recovery.active.is_none() {
            return;
        }
        // 最後の世代を書く（遅いディスクで終了が固まらないよう、待つのは 10 秒まで。間に合わなければ、確定しなかった作りかけは
        // 残らず、前の世代が残る）
        let deadline = Instant::now() + Duration::from_secs(10);
        self.recovery_flush_until(Some(deadline));
        let keep = self.recovery.keep();
        let limits = self.recovery.quota_limits();
        if let Some(a) = self.recovery.active.take() {
            if a.writer.wait_until(Some(deadline)) {
                a.session.close_clean(&a.root, keep, limits.as_ref());
            }
            // 間に合わなかったときは、書き込みが終わる前に印を消さない（落ちた体のまま。次の起動が世代を見つける）
        }
        self.recovery.window = None;
    }

    /// 復旧の操作を当てる。
    pub fn recovery_apply(&mut self, action: RecoveryAction) {
        use RecoveryAction as A;
        if self.recovery.active.is_none() {
            return;
        }
        match action {
            A::OpenWindow => {
                self.recovery.window.get_or_insert_with(Default::default);
                self.recovery.refresh_window();
            }
            A::CloseWindow => self.recovery.window = None,
            A::Select(i) => {
                if let Some(w) = self.recovery.window.as_mut() {
                    w.select(i);
                }
            }
            A::Open => {
                let Some(row) = self
                    .recovery
                    .window
                    .as_ref()
                    .and_then(|w| w.selected_row())
                    .cloned()
                else {
                    return;
                };
                if row.problem.is_some() {
                    return;
                }
                if self.is_stroking() {
                    self.message = self
                        .lang
                        .pick("描いている間は開きません。", "Cannot open during a stroke.")
                        .into();
                    return;
                }
                let request = OpenRequest {
                    pool: row.pool,
                    id: row.id,
                };
                if self.modified {
                    // 今の変更を捨ててよいかは、窓を持つ側（YoluApp）が聞いてから `recovery_open` する
                    self.recovery.open_request = Some(request);
                } else {
                    self.recovery_open(request);
                }
            }
            A::Discard => {
                if let Some(w) = self.recovery.window.as_mut() {
                    w.confirm = w.selected_row().cloned();
                }
            }
            A::CancelDiscard => {
                if let Some(w) = self.recovery.window.as_mut() {
                    w.confirm = None;
                }
            }
            A::ConfirmDiscard => self.recovery_discard_confirmed(),
            A::SetInterval(seconds) => {
                let (lo, hi) = INTERVAL_RANGE;
                self.recovery.settings.interval_seconds = seconds.clamp(lo, hi);
                self.recovery_save_settings();
            }
            A::SetKeep(count) => {
                let (lo, hi) = KEEP_RANGE;
                self.recovery.settings.generations_to_keep = count.clamp(lo, hi);
                // 読めなかった設定でも、利用者が選んだ数は分かった（この実行のあいだ、その数で整理する）
                self.recovery.keep_chosen = true;
                self.recovery_save_settings();
            }
            A::SetDisk(budget) => {
                self.recovery.settings.disk = match budget {
                    DiskBudget::Gib(n) => {
                        DiskBudget::Gib(n.clamp(DISK_GIB_RANGE.0, DISK_GIB_RANGE.1))
                    }
                    other => other,
                };
                // 読めなかった設定でも、利用者が選んだ量は分かった（この実行のあいだ、その量で整理する）
                self.recovery.disk_chosen = true;
                self.recovery_save_settings();
                self.recovery_enforce_now();
            }
            A::DiskDetails(open) => {
                if let Some(w) = self.recovery.window.as_mut() {
                    w.details = open;
                }
            }
        }
    }

    /// 使う量を替えたとき: 書き込みが終わるのを待って（置き場を変えない）、上限を超えていれば古い世代から消す。
    fn recovery_enforce_now(&mut self) {
        if let Some(a) = self.recovery.active.as_ref() {
            a.writer.wait();
        }
        self.recovery_poll();
        if let (Some(a), Some(limits)) =
            (self.recovery.active.as_ref(), self.recovery.quota_limits())
        {
            quota::enforce(&a.root, &limits, Some(a.session.dir()), pool::now_ms());
        }
        self.recovery.refresh_window();
    }

    fn recovery_save_settings(&mut self) {
        let saved = match self.recovery.settings_path.clone() {
            Some(path) => self.recovery.settings.save(&path).is_ok(),
            // 読めなかった設定のファイルは、選んだ置き場・数を既定で上書きしないために書かない
            None => !self.recovery.settings_unreadable,
        };
        if !saved {
            self.message = self
                .lang
                .pick(
                    "復旧の設定を保存できません。",
                    "Cannot save the recovery settings.",
                )
                .into();
        }
    }

    /// 世代を開く（今の変更を捨ててよいと確かめたあと）。開けなければ何も変えずに理由を出す。
    pub fn recovery_open(&mut self, request: OpenRequest) {
        let Some(a) = self.recovery.active.as_ref() else {
            return;
        };
        if self.is_stroking() {
            return;
        }
        if self.is_saving() {
            self.message = format!(
                "{}: {}",
                self.lang.pick("開けません", "Cannot open"),
                crate::project::busy_reason(self.lang)
            );
            return;
        }
        let limits = yolu_io::Limits::from_layer_pixels(self.load_source_bytes());
        match pool::load(&a.root, &request.pool, &request.id, limits, a.session.dir()) {
            Ok((project, info)) => {
                crate::project::open_recovered(self, project);
                self.recovery.recovered_from = Some(info.project_path)
                    .filter(|p| !p.is_empty())
                    .or_else(|| Some(info.title).filter(|t| !t.is_empty()));
                self.recovery.window = None;
            }
            Err(error) => {
                self.message = self.lang.recovery_cannot_open(&error);
                if let Some(w) = self.recovery.window.as_mut() {
                    w.error = Some(self.message.clone());
                }
            }
        }
    }

    fn recovery_discard_confirmed(&mut self) {
        let Some(row) = self.recovery.window.as_mut().and_then(|w| w.confirm.take()) else {
            return;
        };
        let Some(a) = self.recovery.active.as_ref() else {
            return;
        };
        let own = a.session.dir().to_path_buf();
        if row.own {
            // 書き込みが動いているあいだは置き場を変えない
            a.writer.wait();
            self.recovery_poll();
        }
        let result = {
            let a = self.recovery.active.as_ref().expect("上で確かめた");
            pool::discard(&a.root, &row.pool, &row.id, Some(&own))
        };
        match result {
            Ok(()) if row.own => {
                // この実行の置き場の世代を捨てた: 次の確定が期待する札を読み直し、書き置きは入っていないものとして書き直す
                let a = self.recovery.active.as_mut().expect("上で確かめた");
                a.writer.resync(&a.session.store());
                a.recorded = None;
                a.submitted = None;
            }
            Ok(()) => {}
            Err(error) => {
                self.message = self.lang.recovery_error(&error);
                if let Some(w) = self.recovery.window.as_mut() {
                    w.error = Some(self.message.clone());
                }
            }
        }
        self.recovery.refresh_window();
    }
}

impl From<RecoveryAction> for Action {
    fn from(a: RecoveryAction) -> Self {
        Action::Recovery(a)
    }
}
