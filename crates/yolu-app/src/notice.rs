//! 知らせ（直前の操作の結果・断り・注意・失敗）。どの機能も `AppState::notify`（短い形は `info`・`refuse`・`warn`・`fail`）で知らせ、
//! `AppState::message` は ここでだけ書く（試験と `shell::status_text` は今までどおり `message` を読む）。
//!
//! - 種類（`Kind`）はトーストの出し方を決める（`toast`。文の中身では決めない）。注意と失敗は、起動してからの分をログの窓（`panels::log`）に
//!   残し、診断の記録（`crash` の `session-*.log`）へも 1 行ずつ書く（伏せ字の決まりは `crash` のまま。名前の付かない理由の部分だけ）。
//! - 出どころ（`Source`）は機能の単位。ログの窓に名前を出し、記録には言語によらない名前（`key`）を書く。
//! - 断りの文は `lang::refusals`、エラーを文にする関数は `lang::errors` に置く。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lang::Lang;
use crate::state::AppState;

/// 知らせの種類。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 済んだ知らせ。
    #[default]
    Info,
    /// 今の状態で受けられない操作の断り（描いている間・読むだけ・選んでいない…）。
    Refusal,
    /// 済んだが気をつけること・一部できなかったこと。
    Warning,
    /// 失敗（読めない・書けない・壊れている）。
    Error,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Info, Kind::Refusal, Kind::Warning, Kind::Error];

    /// ログの窓と診断の記録に残す種類か（注意と失敗）。
    pub fn is_logged(self) -> bool {
        matches!(self, Kind::Warning | Kind::Error)
    }

    /// core の誤りを知らせるときの種類: 今の状態で受けない物（ロック・描いている間・まとめた編集の中・結合や焼き込みの断り・
    /// クリップボードの断り）は断り、取り消しは済んだ知らせ、ほか（値・予算・無いレイヤー）は失敗。
    pub fn of_core(error: &yolu_core::CoreError) -> Kind {
        use yolu_core::CoreError::*;
        match error {
            Cancelled => Kind::Info,
            LayerLocked { .. }
            | StrokeActive
            | BatchActive
            | MergeRefused(_)
            | MergeAppearance(_)
            | InactiveEffect { .. }
            | Clipboard(_) => Kind::Refusal,
            InvalidArgument(_)
            | Unsupported(_)
            | LayerNotFound
            | ChannelNotFound
            | NoActiveStroke
            | SourceBudgetExceeded
            | StrokeBudgetExceeded
            | WorkingBudgetExceeded
            | TileUnreadable => Kind::Error,
        }
    }

    /// 2 つの知らせを 1 つの文にまとめたときの種類（重いほう: 失敗 > 注意 > 断り > 済んだ知らせ）。
    pub fn worse(self, other: Kind) -> Kind {
        let rank = |k: Kind| match k {
            Kind::Info => 0,
            Kind::Refusal => 1,
            Kind::Warning => 2,
            Kind::Error => 3,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }

    /// 画面の名前（ログの窓の印のツールチップ・写した文）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Kind::Info => lang.pick("情報", "Info"),
            Kind::Refusal => lang.pick("断り", "Refused"),
            Kind::Warning => lang.pick("注意", "Warning"),
            Kind::Error => lang.pick("エラー", "Error"),
        }
    }

    /// 診断の記録に書く名前（言語によらない）。
    pub fn key(self) -> &'static str {
        match self {
            Kind::Info => "Info",
            Kind::Refusal => "Refused",
            Kind::Warning => "Warning",
            Kind::Error => "Error",
        }
    }
}

/// 知らせの出どころ（機能の単位）。足すときは末尾に。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// 取り消し・やり直し・ヒストリー・アプリの全体の操作。
    Edit,
    Save,
    Open,
    /// 新規プロジェクト・プロジェクトの構成。
    Project,
    TextureSet,
    Layer,
    Channel,
    /// 効果の層（フィルター・ジェネレーター）。
    Effect,
    /// 塗りつぶしの層（画像と投影・デカール・グラデーションデカール）。
    FillLayer,
    /// 塗りつぶし・ポリゴン塗りつぶしの道具。
    Fill,
    Material,
    /// ブラシ・消しゴム・サブツール。
    Brush,
    Canvas,
    Selection,
    Path,
    Gradient,
    /// 移動・変形。
    Transform,
    Eyedropper,
    Stencil,
    /// 図形と定規。
    Ruler,
    /// カラー・カラーセット。
    Color,
    /// アセットの棚。
    Assets,
    Library,
    Clipboard,
    Psd,
    /// テンプレートの画像・チャンネルの書き出し。
    Export,
    /// 配布用に保存。
    Distribute,
    Bake,
    View3d,
    Pose,
    Settings,
    Update,
    Recovery,
    LiveLink,
    /// 外からの操作（CLI・MCP のクライアント）。
    Ops,
    /// 表示（GPU の装置・描き直し）。
    Display,
    /// アクション（操作の記録と再生）。
    Action,
}

impl Source {
    pub const ALL: [Source; 37] = [
        Source::Edit,
        Source::Save,
        Source::Open,
        Source::Project,
        Source::TextureSet,
        Source::Layer,
        Source::Channel,
        Source::Effect,
        Source::FillLayer,
        Source::Fill,
        Source::Material,
        Source::Brush,
        Source::Canvas,
        Source::Selection,
        Source::Path,
        Source::Gradient,
        Source::Transform,
        Source::Eyedropper,
        Source::Stencil,
        Source::Ruler,
        Source::Color,
        Source::Assets,
        Source::Library,
        Source::Clipboard,
        Source::Psd,
        Source::Export,
        Source::Distribute,
        Source::Bake,
        Source::View3d,
        Source::Pose,
        Source::Settings,
        Source::Update,
        Source::Recovery,
        Source::LiveLink,
        Source::Ops,
        Source::Display,
        Source::Action,
    ];

    /// 画面の名前（ログの窓の列）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Source::Edit => lang.pick("編集", "Edit"),
            Source::Save => lang.pick("保存", "Save"),
            Source::Open => lang.pick("開く", "Open"),
            Source::Project => lang.pick("プロジェクト", "Project"),
            Source::TextureSet => lang.pick("テクスチャセット", "Texture Set"),
            Source::Layer => lang.pick("レイヤー", "Layer"),
            Source::Channel => lang.pick("チャンネル", "Channel"),
            Source::Effect => lang.pick("効果", "Effects"),
            Source::FillLayer => lang.pick("塗りつぶしレイヤー", "Fill Layer"),
            Source::Fill => lang.pick("塗りつぶし", "Fill"),
            Source::Material => lang.pick("マテリアル", "Material"),
            Source::Brush => lang.pick("ブラシ", "Brush"),
            Source::Canvas => lang.pick("キャンバス", "Canvas"),
            Source::Selection => lang.pick("選択範囲", "Selection"),
            Source::Path => lang.pick("パス", "Path"),
            Source::Gradient => lang.pick("グラデーション", "Gradient"),
            Source::Transform => lang.pick("移動・変形", "Transform"),
            Source::Eyedropper => lang.pick("スポイト", "Eyedropper"),
            Source::Stencil => lang.pick("ステンシル", "Stencil"),
            Source::Ruler => lang.pick("図形と定規", "Shapes and Rulers"),
            Source::Color => lang.pick("カラー", "Color"),
            Source::Assets => lang.pick("アセット", "Assets"),
            Source::Library => lang.pick("ライブラリ", "Library"),
            Source::Clipboard => lang.pick("クリップボード", "Clipboard"),
            Source::Psd => "PSD",
            Source::Export => lang.pick("書き出し", "Export"),
            Source::Distribute => lang.pick("配布用に保存", "Save for Distribution"),
            Source::Bake => lang.pick("ベイク", "Bake"),
            Source::View3d => lang.pick("3D ビュー", "3D View"),
            Source::Pose => lang.pick("ポーズ", "Pose"),
            Source::Settings => lang.pick("設定", "Settings"),
            Source::Update => lang.pick("更新", "Updates"),
            Source::Recovery => lang.pick("復旧", "Recovery"),
            Source::LiveLink => "Live Link",
            Source::Ops => lang.pick("外からの操作", "External Commands"),
            Source::Display => lang.pick("表示", "Display"),
            Source::Action => lang.pick("アクション", "Actions"),
        }
    }

    /// 診断の記録に書く名前（言語によらない。変えない）。
    pub fn key(self) -> &'static str {
        match self {
            Source::Edit => "edit",
            Source::Save => "save",
            Source::Open => "open",
            Source::Project => "project",
            Source::TextureSet => "texture_set",
            Source::Layer => "layer",
            Source::Channel => "channel",
            Source::Effect => "effect",
            Source::FillLayer => "fill_layer",
            Source::Fill => "fill",
            Source::Material => "material",
            Source::Brush => "brush",
            Source::Canvas => "canvas",
            Source::Selection => "selection",
            Source::Path => "path",
            Source::Gradient => "gradient",
            Source::Transform => "transform",
            Source::Eyedropper => "eyedropper",
            Source::Stencil => "stencil",
            Source::Ruler => "ruler",
            Source::Color => "color",
            Source::Assets => "assets",
            Source::Library => "library",
            Source::Clipboard => "clipboard",
            Source::Psd => "psd",
            Source::Export => "export",
            Source::Distribute => "distribute",
            Source::Bake => "bake",
            Source::View3d => "view3d",
            Source::Pose => "pose",
            Source::Settings => "settings",
            Source::Update => "update",
            Source::Recovery => "recovery",
            Source::LiveLink => "livelink",
            Source::Ops => "ops",
            Source::Display => "display",
            Source::Action => "action",
        }
    }
}

/// 1 つの知らせ。
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub kind: Kind,
    pub source: Source,
    pub text: String,
    pub at: SystemTime,
}

/// ログの窓が持つ行の上限（起動の間。超えたら古い物から捨てる）。
pub const LOG_LIMIT: usize = 1000;

/// ログの 1 行。同じ知らせ（種類・出どころ・文が同じ）が続いたら 1 行にまとめ、回数を数える（時刻は最後の分）。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// 行の番号（起動の間で一意。古い行を捨てても、選んだ行を見失わない）。
    pub id: u64,
    pub notice: Notice,
    pub count: u32,
}

/// 起動してからの注意と失敗の知らせ（ログの窓が読む。ファイルは読まない）。
#[derive(Clone, Debug, Default)]
pub struct NoticeLog {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl NoticeLog {
    /// 知らせを入れる（注意と失敗だけ。ほかは入れずに false）。直前の行と同じ知らせなら回数を増やす。
    pub fn push(&mut self, notice: Notice) -> bool {
        if !notice.kind.is_logged() {
            return false;
        }
        if let Some(last) = self.entries.back_mut() {
            if last.notice.kind == notice.kind
                && last.notice.source == notice.source
                && last.notice.text == notice.text
            {
                last.count = last.count.saturating_add(1);
                last.notice.at = notice.at;
                return true;
            }
        }
        if self.entries.len() == LOG_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            id: self.next_id,
            notice,
            count: 1,
        });
        self.next_id += 1;
        true
    }

    /// 古い順の行。
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 最後の行がこの知らせなら取り下げる（まとめた行なら回数を 1 つ減らす）。`AppState::amend` が使う。
    fn retract(&mut self, notice: &Notice) {
        let Some(last) = self.entries.back_mut() else {
            return;
        };
        if last.notice.kind != notice.kind
            || last.notice.source != notice.source
            || last.notice.text != notice.text
        {
            return;
        }
        if last.count > 1 {
            last.count -= 1;
        } else {
            self.entries.pop_back();
        }
    }

    /// 画面の一覧だけを消す（診断の記録のファイルは消さない）。
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl AppState {
    /// 知らせる: `message` に文を入れ（トーストと試験が読む）、種類を `last_notice` に残す。注意と失敗はログの窓と診断の記録へも（断りは、印を付けた部分だけ診断の記録へ）。
    /// 空の文は知らせではない（`message` を空にする）。
    pub fn notify(&mut self, kind: Kind, source: Source, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            self.message.clear();
            self.last_notice = None;
            return;
        }
        let notice = Notice {
            kind,
            source,
            text,
            at: SystemTime::now(),
        };
        self.message.clone_from(&notice.text);
        if kind.is_logged() {
            crate::crash::notice(kind.key(), source.key(), &notice.text);
            self.notice_log.push(notice.clone());
        } else {
            // 断りと済んだ知らせはログの窓に入れない。診断の記録へは、失敗・断りの文として印を付けた部分だけ（前と同じ）。
            crate::crash::message(&notice.text);
        }
        self.last_notice = Some(notice);
    }

    /// 済んだ知らせ。
    pub fn info(&mut self, source: Source, text: impl Into<String>) {
        self.notify(Kind::Info, source, text);
    }

    /// 今の状態で受けられない操作の断り。
    pub fn refuse(&mut self, source: Source, text: impl Into<String>) {
        self.notify(Kind::Refusal, source, text);
    }

    /// 済んだが気をつけること・一部できなかったこと。
    pub fn warn(&mut self, source: Source, text: impl Into<String>) {
        self.notify(Kind::Warning, source, text);
    }

    /// 失敗。
    pub fn fail(&mut self, source: Source, text: impl Into<String>) {
        self.notify(Kind::Error, source, text);
    }

    /// 今の `message` を書いた知らせ（`notify` が書いた文でなければ None）。
    pub fn current_notice(&self) -> Option<&Notice> {
        self.last_notice
            .as_ref()
            .filter(|n| !self.message.is_empty() && n.text == self.message)
    }

    /// 今の `message` の種類（`notify` が書いた文でなければ Info）。
    pub fn message_kind(&self) -> Kind {
        self.current_notice().map_or(Kind::Info, |n| n.kind)
    }

    /// `message` を書く操作の入口。前の文を預かって `message` を空にする（出口で、書かれたかを前と同じ文でも見分けて、知らせにする）。
    /// 操作の中では、前の操作の文は見えない。
    pub fn message_begin(&mut self) -> String {
        let kind = self.message_kind();
        self.toast.begin(&mut self.message, kind)
    }

    /// `message_begin` の出口。書かれていれば新しい知らせとして出し、書かれていなければ前の文を戻す。
    pub fn message_end(&mut self, prior: String) {
        // 種類が要るのは書かれたとき（書かれていなければ前の文に戻すだけで、知らせは出し直さない）
        let kind = self.message_kind();
        self.toast.end(&mut self.message, prior, kind);
    }

    /// `message` を明示して空にする（操作の中では、空のまま終わっても前の文を戻さない。保存の結果が書かれたかを見分ける所が使う）。
    pub fn clear_message(&mut self) {
        self.toast.clear(&mut self.message);
        self.last_notice = None;
    }

    /// 今の知らせに但し書き `extra` を `joint` でつないで知らせ直す（同じ結果の知らせを 2 つにしない）。種類は今の知らせと `kind` の
    /// 重いほう、出どころは今の知らせのもの（今の知らせが無ければ `kind`・`source` で `extra` だけを知らせる）。今の知らせをログに
    /// 入れていたら、その行は添えた後の文に置き換える（診断の記録のファイルは書き直さない）。
    pub fn amend(&mut self, kind: Kind, source: Source, joint: &str, extra: &str) {
        let Some(current) = self.current_notice().cloned() else {
            self.notify(kind, source, extra);
            return;
        };
        if current.kind.is_logged() {
            self.notice_log.retract(&current);
        }
        let text = format!("{}{joint}{extra}", current.text);
        self.notify(current.kind.worse(kind), current.source, text);
    }

    /// `f` の間に書かれた知らせを捨て、前の知らせ（文と種類）に戻す（起動時の知らせを、待ち受けを始めた文で上書きしない、など）。
    pub fn keep_notice<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let message = self.message.clone();
        let notice = self.last_notice.clone();
        let result = f(self);
        self.message = message;
        self.last_notice = notice;
        result
    }

    /// 試験の口: `message` を、知らせを通さずに書いた形にする（直に書かれた文を種類なしで扱うことを確かめる）。
    #[doc(hidden)]
    pub fn set_message_for_test(&mut self, text: impl Into<String>) {
        self.message = text.into();
    }

    /// トーストへ、ここまでに書かれた `message` を渡す（フレームの中で、`apply` の外の書き込みもすぐ出す）。
    pub(crate) fn flush_message(&mut self) {
        let kind = self.message_kind();
        self.toast.flush(&self.message, kind);
    }

    /// 診断の記録へ、知らせを通さずに書かれた `message`（外からの直の書き込み）だけを渡す。`notify` の文は `notify` が書いた。
    pub(crate) fn record_message(&self) {
        if self.current_notice().is_none() {
            crate::crash::message(&self.message);
        }
    }
}

/// 地方時と UTC の差（秒）。まだ測っていなければ `UNMEASURED`。
static UTC_OFFSET: AtomicI64 = AtomicI64::new(UNMEASURED);
const UNMEASURED: i64 = i64::MIN;

/// 地方時と UTC の差（秒。起動の間で一度だけ測る。夏時間の切り替わりは追わない）。
fn utc_offset() -> i64 {
    let known = UTC_OFFSET.load(Ordering::Relaxed);
    if known != UNMEASURED {
        return known;
    }
    let measured = measure_utc_offset();
    UTC_OFFSET.store(measured, Ordering::Relaxed);
    measured
}

/// 試験の口: 地方時の差を決める（画面の絵を、走らせる机の時間帯によらず同じにする）。
#[doc(hidden)]
pub fn set_utc_offset_for_test(seconds: i64) {
    UTC_OFFSET.store(seconds, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn measure_utc_offset() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: tm は書き込み先として渡すだけ。localtime_r はスレッド安全な形。
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
    if ok {
        tm.tm_gmtoff
    } else {
        0
    }
}

#[cfg(windows)]
fn measure_utc_offset() -> i64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
    // 1601 年からの 100 ナノ秒
    let ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64 / 100)
        + 116_444_736_000_000_000;
    let utc = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local = FILETIME::default();
    // SAFETY: 読む値と書く先を渡すだけ。
    if unsafe { FileTimeToLocalFileTime(&utc, &mut local) }.is_err() {
        return 0;
    }
    let local = (u64::from(local.dwHighDateTime) << 32) | u64::from(local.dwLowDateTime);
    (local as i64 - ticks as i64) / 10_000_000
}

#[cfg(not(any(target_os = "linux", windows)))]
fn measure_utc_offset() -> i64 {
    0
}

/// 地方時の時:分:秒（ログの窓の列・写した文）。
pub fn clock_text(at: SystemTime) -> String {
    clock_at(at, utc_offset())
}

/// UTC との差 `offset` 秒の時計の時:分:秒。
fn clock_at(at: SystemTime, offset: i64) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
        + offset;
    let day = seconds.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn notice(kind: Kind, text: &str, at: u64) -> Notice {
        Notice {
            kind,
            source: Source::Save,
            text: text.into(),
            at: UNIX_EPOCH + Duration::from_secs(at),
        }
    }

    #[test]
    fn the_log_keeps_warnings_and_errors_and_folds_repeats() {
        let mut log = NoticeLog::default();
        assert!(!log.push(notice(Kind::Info, "保存しました。", 1)));
        assert!(!log.push(notice(Kind::Refusal, "描いている間はできません。", 2)));
        assert!(log.is_empty());
        assert!(log.push(notice(Kind::Error, "書けません", 3)));
        assert!(log.push(notice(Kind::Error, "書けません", 4)));
        assert!(log.push(notice(Kind::Error, "書けません", 5)));
        assert_eq!(log.len(), 1);
        let first = log.entries().next().unwrap();
        assert_eq!(first.count, 3);
        assert_eq!(first.notice.at, UNIX_EPOCH + Duration::from_secs(5));
        // 種類が違えば別の行、間に別の知らせを挟めばまた 1 から
        log.push(notice(Kind::Warning, "書けません", 6));
        log.push(notice(Kind::Error, "書けません", 7));
        assert_eq!(
            log.entries().map(|e| e.count).collect::<Vec<_>>(),
            [3, 1, 1]
        );
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn the_log_drops_the_oldest_rows_past_the_limit() {
        let mut log = NoticeLog::default();
        for i in 0..LOG_LIMIT + 5 {
            log.push(notice(Kind::Warning, &format!("注意 {i}"), i as u64));
        }
        assert_eq!(log.len(), LOG_LIMIT);
        assert_eq!(log.entries().next().unwrap().notice.text, "注意 5");
        assert_eq!(
            log.entries().last().unwrap().notice.text,
            format!("注意 {}", LOG_LIMIT + 4)
        );
        // 行の番号は捨てても振り直さない
        assert_eq!(log.entries().next().unwrap().id, 5);
    }

    #[test]
    fn sources_and_kinds_have_both_names_and_distinct_keys() {
        let mut keys = std::collections::HashSet::new();
        for source in Source::ALL {
            assert!(keys.insert(source.key()), "{}", source.key());
            for lang in Lang::ALL {
                assert!(!source.name(lang).is_empty());
            }
        }
        assert_eq!(keys.len(), Source::ALL.len());
        for kind in Kind::ALL {
            assert_ne!(kind.name(Lang::Ja), kind.name(Lang::En));
        }
    }

    #[test]
    fn notify_writes_the_message_and_keeps_warnings_and_errors_in_the_log() {
        let mut app = AppState::new_in(8, 8, Lang::Ja);
        app.info(Source::Save, "保存しました。");
        assert_eq!(app.message, "保存しました。");
        assert_eq!(app.message_kind(), Kind::Info);
        app.refuse(Source::Edit, "描いている間はできません。");
        assert_eq!(app.message_kind(), Kind::Refusal);
        assert!(
            app.notice_log.is_empty(),
            "済んだ知らせと断りはログに入れない"
        );
        app.warn(
            Source::Bake,
            "UV の面積が 0 の三角形があり、その面は焼けません。",
        );
        app.fail(Source::Open, "「a.ylp」を開けません（壊れています）。");
        let rows: Vec<(Kind, Source)> = app
            .notice_log
            .entries()
            .map(|e| (e.notice.kind, e.notice.source))
            .collect();
        assert_eq!(
            rows,
            [(Kind::Warning, Source::Bake), (Kind::Error, Source::Open)]
        );
        assert_eq!(app.message, "「a.ylp」を開けません（壊れています）。");
        // 知らせを通さずに書かれた文（まだ移していない所）は、種類なし（済んだ知らせの出し方）
        app.set_message_for_test("直に書いた文");
        assert_eq!(app.message_kind(), Kind::Info);
        assert!(app.current_notice().is_none());
        // 空の文は知らせではない
        app.notify(Kind::Error, Source::Save, "");
        assert!(app.message.is_empty() && app.last_notice.is_none());
        assert_eq!(app.notice_log.len(), 2);
    }

    #[test]
    fn amend_adds_a_caveat_to_the_same_notice_and_keeps_one_log_row() {
        let mut app = AppState::new_in(8, 8, Lang::Ja);
        app.info(Source::Library, "置きました: a");
        app.amend(
            Kind::Warning,
            Source::Library,
            " · ",
            "16 bit を 8 bit にしました",
        );
        assert_eq!(app.message, "置きました: a · 16 bit を 8 bit にしました");
        assert_eq!(app.message_kind(), Kind::Warning, "重いほうの種類");
        assert_eq!(app.notice_log.len(), 1);
        // ログに入っていた注意に添えたら、その行を置き換える（2 行にしない）
        app.amend(Kind::Warning, Source::Library, " ", "もう 1 つ");
        assert_eq!(app.notice_log.len(), 1);
        assert_eq!(
            app.notice_log.entries().next().unwrap().notice.text,
            "置きました: a · 16 bit を 8 bit にしました もう 1 つ"
        );
        // 今の知らせが無ければ、添える文だけを知らせる
        app.clear_message();
        app.amend(Kind::Warning, Source::Recovery, " ", "復旧を使えません");
        assert_eq!(app.message, "復旧を使えません");
        assert_eq!(app.current_notice().unwrap().source, Source::Recovery);
    }

    #[test]
    fn keep_notice_puts_back_the_previous_notice() {
        let mut app = AppState::new_in(8, 8, Lang::Ja);
        app.warn(Source::Settings, "設定を読めません");
        app.keep_notice(|app| app.info(Source::LiveLink, "待っています"));
        assert_eq!(app.message, "設定を読めません");
        assert_eq!(app.message_kind(), Kind::Warning);
    }

    #[test]
    fn the_toast_takes_the_kind_of_the_notice_written_in_the_operation() {
        let mut app = AppState::new_in(8, 8, Lang::Ja);
        let prior = app.message_begin();
        app.fail(Source::Save, "保存できません");
        app.message_end(prior);
        app.toast.update(0.0);
        assert_eq!(app.toast.kind(), Kind::Error);
        assert_eq!(app.toast.lifetime(), crate::toast::LONG_SECONDS);
        let prior = app.message_begin();
        app.info(Source::Save, "保存しました。");
        app.message_end(prior);
        app.toast.update(1.0);
        assert_eq!(app.toast.kind(), Kind::Info);
        assert_eq!(app.toast.lifetime(), crate::toast::INFO_SECONDS);
        // 何も書かない操作は、前の文と種類を戻す
        let prior = app.message_begin();
        app.message_end(prior);
        assert_eq!(app.message, "保存しました。");
        assert_eq!(app.message_kind(), Kind::Info);
    }

    #[test]
    fn the_clock_is_hours_minutes_and_seconds() {
        let at = |s: u64| UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(clock_at(at(3661), 9 * 3600), "10:01:01");
        assert_eq!(clock_at(at(86_399), 0), "23:59:59");
        assert_eq!(clock_at(at(3600), -2 * 3600), "23:00:00");
    }
}
