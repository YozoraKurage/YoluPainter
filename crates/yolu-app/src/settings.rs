//! 利用者ごとのアプリ設定。文書・試験の AppState とは独立して読み書きする。
//!
//! 設定のファイルは `キー=値` を 1 行ずつ。**既定の値は書かない**（言語は常に書く）ので、何も変えていない間は今までと同じ中身で、
//! 知らないキーは読み飛ばす（新しい版が足した項目で壊れない）。正しくない値は、その項目だけを既定へ戻して理由（`Problem`）を返し、
//! ほかの項目は生かす。読んだだけではファイルに触らず、設定を変えて書き直すときに置き換える。
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::engine::DEFAULT_SOURCE_BUDGET_BYTES;
use crate::lang::Lang;
use crate::pen::adjust::{PressureAdjust, MIN_SPAN};

/// 設定のファイルの場所（設定のフォルダが分からなければ None）。
pub fn path() -> Option<PathBuf> {
    config_path(std::env::consts::OS, |key| std::env::var_os(key).map(PathBuf::from))
}

fn config_base(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let absolute = |key| env(key).filter(|p| p.is_absolute());
    match os {
        "windows" => absolute("APPDATA"),
        "macos" => absolute("HOME").map(|p| p.join("Library/Application Support")),
        _ => absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|p| p.join(".config"))),
    }
}

fn config_path(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    Some(config_base(os, env)?.join("YoluPainter").join("settings.conf"))
}

/// 棚の場所の既定（設定のフォルダの下の Library）。設定のフォルダが分からなければ None。
pub fn default_library_folder() -> Option<PathBuf> {
    config_base(std::env::consts::OS, |key| std::env::var_os(key).map(PathBuf::from))
        .map(|base| base.join("YoluPainter").join("Library"))
}

// ───────── 値の種類 ─────────

/// メモリの予算の指定: 自動（このマシンのメモリから決める）か MiB。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    Auto,
    Mib(u32),
}

/// 予算の種類（文書ごと）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BudgetKind {
    /// 取り消し履歴。
    Undo,
    /// 全レイヤーの画素の合計。
    Source,
    /// 1 回の操作（ストローク・塗りつぶし）の巻き戻し用。
    Stroke,
}

impl BudgetKind {
    pub const ALL: [BudgetKind; 3] = [BudgetKind::Undo, BudgetKind::Source, BudgetKind::Stroke];

    /// 設定のファイルのキー。
    pub fn key(self) -> &'static str {
        match self {
            BudgetKind::Undo => "undo_budget_mib",
            BudgetKind::Source => "source_budget_mib",
            BudgetKind::Stroke => "stroke_budget_mib",
        }
    }

    /// 指定できる MiB の範囲（Unity 版と同じ）。取り消し履歴の 0 は、最小の段数だけを残す。
    pub fn range(self) -> (u32, u32) {
        match self {
            BudgetKind::Undo => (0, 16384),
            BudgetKind::Source => (16, 32768),
            BudgetKind::Stroke => (8, 8192),
        }
    }

    /// 自動のときの MiB: 物理メモリの一定の割合を、小さい機械でも作業できる下限と、ほかのアプリ・モデルの分を残す上限で挟む
    /// （16 GB で 取り消し 1024・画素 2048・1 回の操作 512。Unity 版と同じ式）。
    pub fn automatic_mib(self, ram_mib: u64) -> u32 {
        let (div, lo, hi) = match self {
            BudgetKind::Undo => (16, 256, 2048),
            BudgetKind::Source => (8, 256, 8192),
            BudgetKind::Stroke => (32, 64, 1024),
        };
        (ram_mib / div).clamp(lo, hi) as u32
    }

    /// 予算の選択肢（自動のほかに並べる MiB）。
    pub fn choices(self) -> &'static [u32] {
        match self {
            BudgetKind::Undo => &[0, 256, 512, 1024, 2048, 4096, 8192],
            BudgetKind::Source => &[512, 1024, 2048, 4096, 8192, 16384],
            BudgetKind::Stroke => &[64, 128, 256, 512, 1024, 2048],
        }
    }
}

/// 表示の合成をどこで行うか（保存・書き出しの合成は、どれでも CPU が正本）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compositing {
    /// 使えるなら GPU、ソフトウェアの描画では CPU。
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl Compositing {
    pub const ALL: [Compositing; 3] = [Compositing::Auto, Compositing::Gpu, Compositing::Cpu];

    fn key(self) -> &'static str {
        match self {
            Compositing::Auto => "auto",
            Compositing::Gpu => "gpu",
            Compositing::Cpu => "cpu",
        }
    }
}

/// 書き出しの余白に選べる値（テクセル。-1 は届くかぎり全部、0 は塗り広げない）。
pub const EXPORT_PADDINGS: [i32; 8] = [0, 2, 4, 8, 16, 32, 64, -1];
/// 書き出しの余白の既定。
pub const DEFAULT_EXPORT_PADDING: i32 = -1;
/// CPU のスレッドの数の上限（論理プロセッサの数より多くてもよい。多すぎると遅くなるだけ）。
pub const MAX_CPU_THREADS: u32 = 1024;
/// 取り消し履歴の予算を超えても残す直近の段数の上限と既定。
pub const MAX_MIN_UNDO_STEPS: u32 = 100;
pub const DEFAULT_MIN_UNDO_STEPS: u32 = 5;

/// 利用者ごとの設定。
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub lang: Lang,
    /// 書き出しで UV の外へ色を塗り広げるテクセルの数（-1 は届くかぎり全部）。
    pub export_padding: i32,
    pub undo_budget: Budget,
    pub source_budget: Budget,
    pub stroke_budget: Budget,
    /// 取り消し履歴の予算を超えても残す直近の段数。
    pub min_undo_steps: u32,
    /// CPU の処理に使うスレッドの数（None は自動 = 論理プロセッサの数。起動のときに決まる）。
    pub cpu_threads: Option<u32>,
    pub compositing: Compositing,
    /// 棚の場所（None は既定。`default_library_folder`）。
    pub library_folder: Option<PathBuf>,
    /// 上書き保存で置き換えた前の版（退避）をいくつ残すか。
    pub backups: BackupKeep,
    /// 選択範囲の下のボタンの帯を出すか（「選択範囲」メニューで切り替える。設定の窓には無い）。
    pub selection_bar: bool,
    /// 全体の筆圧の調整（端末ごと。ペンの筆圧を、ブラシへ渡す前に下限・上限と曲線で直す。「表示 → 筆圧の調整…」の窓）。
    pub pressure: PressureAdjust,
    pub navigation: crate::view3d::navigation::Preferences,
    pub uv_wireframe: bool,
    pub uv_wireframe_color: [u8; 4],
    /// 起動時に Live Link を待ち受けるか（--livelink はこの設定より優先）。
    pub livelink_on_startup: bool,
    /// カラーの欄を色相の円と中の四角で出すか（切ると四角と色相の帯）。
    pub color_wheel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::default(),
            export_padding: DEFAULT_EXPORT_PADDING,
            undo_budget: Budget::Auto,
            source_budget: Budget::Auto,
            stroke_budget: Budget::Auto,
            min_undo_steps: DEFAULT_MIN_UNDO_STEPS,
            cpu_threads: None,
            compositing: Compositing::Auto,
            library_folder: None,
            backups: BackupKeep::All,
            selection_bar: true,
            pressure: PressureAdjust::default(),
            navigation: crate::view3d::navigation::Preferences::default(),
            uv_wireframe: true,
            uv_wireframe_color: crate::uv_wireframe::DEFAULT_COLOR,
            livelink_on_startup: true,
            color_wheel: true,
        }
    }
}

impl Settings {
    pub fn budget(&self, kind: BudgetKind) -> Budget {
        match kind {
            BudgetKind::Undo => self.undo_budget,
            BudgetKind::Source => self.source_budget,
            BudgetKind::Stroke => self.stroke_budget,
        }
    }

    pub fn set_budget(&mut self, kind: BudgetKind, budget: Budget) {
        match kind {
            BudgetKind::Undo => self.undo_budget = budget,
            BudgetKind::Source => self.source_budget = budget,
            BudgetKind::Stroke => self.stroke_budget = budget,
        }
    }

    /// 文書に入れる予算（バイト）。自動は `ram_mib` から。
    pub fn budgets(&self, ram_mib: u64) -> Budgets {
        let bytes = |kind: BudgetKind| {
            let mib = match self.budget(kind) {
                Budget::Auto => kind.automatic_mib(ram_mib),
                Budget::Mib(n) => n,
            };
            mib as u64 * 1024 * 1024
        };
        Budgets {
            undo: bytes(BudgetKind::Undo),
            source: bytes(BudgetKind::Source),
            stroke: bytes(BudgetKind::Stroke),
            min_undo_steps: self.min_undo_steps as usize,
        }
    }

    /// 読み込み（.ylp を開く・書き出しが写した文書を戻す）で 1 つの文書に許す層の画素のバイト数: 設定の予算と core の既定の大きい方。
    /// 設定を上げれば大きな文書も読める。設定を下げても、読めていた文書は読める（今の画素が予算を超えるときは `sync_budgets` が
    /// 予算をその量まで広げて知らせる）。
    pub fn load_source_bytes(&self, ram_mib: u64) -> u64 {
        self.budgets(ram_mib).source.max(DEFAULT_SOURCE_BUDGET_BYTES)
    }

    /// 棚の場所（設定になければ既定。設定のフォルダも分からなければ None）。
    pub fn library_folder(&self) -> Option<PathBuf> {
        self.library_folder.clone().or_else(default_library_folder)
    }
}

/// 文書に入れる予算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub undo: u64,
    pub source: u64,
    pub stroke: u64,
    pub min_undo_steps: usize,
}

/// このマシンの物理メモリ（MiB。分からなければ 8 GiB と仮定。最低 1 GiB）。
pub fn system_memory_mib() -> u64 {
    static RAM: OnceLock<u64> = OnceLock::new();
    *RAM.get_or_init(|| detect_memory_mib().unwrap_or(8192).max(1024))
}

#[cfg(target_os = "linux")]
fn detect_memory_mib() -> Option<u64> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// `/proc/meminfo` の MemTotal（kB）を MiB に。
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_meminfo(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib / 1024)
}

#[cfg(windows)]
fn detect_memory_mib() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: dwLength を設定した MEMORYSTATUSEX を渡す（Win32 の呼び方どおり）。
    unsafe { GlobalMemoryStatusEx(&mut status).ok()? };
    Some(status.ullTotalPhys / (1024 * 1024))
}

#[cfg(target_os = "macos")]
fn detect_memory_mib() -> Option<u64> {
    let out = std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok()?;
    let bytes: u64 = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn detect_memory_mib() -> Option<u64> {
    None
}

// ───────── 読み込みで見つけた問題 ─────────

/// 読んだときに既定へ戻したもの（画面は種類から短い理由を作る）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// ファイルを読めない（読み込みの失敗・大きすぎる・`キー=値` の形ではない行）。設定は全部既定。
    Unreadable,
    /// 知らない言語（書いてあった値）。
    Language(String),
    /// 値が正しくない項目（設定のファイルのキーと、書いてあった値）。その項目だけ既定。
    Invalid { key: &'static str, value: String },
    /// 退避を残す数が `all` でも 0〜上限の数でもない（書いてあった値）。すべて残す。
    Backups(String),
}

impl Problem {
    /// 状態の帯の短い文。
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Self::Unreadable => lang.pick("設定を読めません。", "Cannot read the settings.").into(),
            Self::Language(_) => lang.pick("言語の設定を読めません。", "Cannot read the language setting.").into(),
            Self::Invalid { key, value } => {
                let shown: String = value.chars().take(12).collect();
                let name = setting_name(lang, key);
                lang.pick(
                    format!("{name}の設定が正しくありません（{shown}）。既定に戻します。"),
                    format!("Invalid {name} setting ({shown}); using the default."),
                )
            }
            Self::Backups(value) => {
                let shown: String = value.chars().take(12).collect();
                lang.pick(
                    format!("退避を残す数の設定が正しくありません（{shown}）。すべて残します。"),
                    format!("Invalid Backups to Keep setting ({shown}); keeping all."),
                )
            }
        }
    }
}

/// 設定のキーの、画面での名前。
pub fn setting_name(lang: Lang, key: &str) -> &'static str {
    match key {
        "language" => lang.pick("言語", "Language"),
        "view3d_orbit" => lang.pick("回転の中心", "Orbit center"),
        "view3d_zoom" => lang.pick("ズームの中心", "Zoom center"),
        "export_padding" => lang.pick("書き出しの余白", "Export padding"),
        "undo_budget_mib" => lang.pick("取り消し履歴", "Undo history"),
        "source_budget_mib" => lang.pick("レイヤーの画素", "Layer pixels"),
        "stroke_budget_mib" => lang.pick("1 回の操作", "One operation"),
        "min_undo_steps" => lang.pick("最小の取り消し段数", "Minimum undo steps"),
        "cpu_threads" => lang.pick("CPU のスレッド", "CPU threads"),
        "compositing" => lang.pick("表示の合成", "Display compositing"),
        "library_folder" => lang.pick("棚の場所", "Library folder"),
        "backups" => lang.pick("退避を残す数", "Backups to Keep"),
        "uv_wireframe_color" => lang.pick("UV ワイヤーフレームの色", "UV wireframe color"),
        "pressure_low" => lang.pick("筆圧の下限", "Pen pressure low"),
        "pressure_high" => lang.pick("筆圧の上限", "Pen pressure high"),
        "pressure_curve" => lang.pick("筆圧の曲線", "Pen pressure curve"),
        _ => lang.pick("設定", "Setting"),
    }
}

// ───────── 読む・書く ─────────

/// 設定のファイルを読む。無ければ既定。値が正しくない項目は既定へ戻して `Problem` を返す（ファイルは、設定を変えて書き直すまで触らない）。
pub fn load(path: &Path) -> (Settings, Vec<Problem>) {
    match read(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), Vec::new()),
        Err(_) => (Settings::default(), vec![Problem::Unreadable]),
    }
}

fn read(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "settings too large"));
    }
    Ok(text)
}

const MAX_FILE_BYTES: u64 = 4096;

fn parse(text: &str) -> (Settings, Vec<Problem>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    // 筆圧の調整は 3 つの項目が組で意味を持つので、読み終えてからまとめて作る
    let (mut low, mut high, mut curve) = (None, None, None);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return (Settings::default(), vec![Problem::Unreadable]);
        };
        let value = value.trim();
        let mut invalid = |key: &'static str| problems.push(Problem::Invalid { key, value: value.to_owned() });
        match key.trim() {
            "language" => match value {
                "ja" => settings.lang = Lang::Ja,
                "en" => settings.lang = Lang::En,
                _ => problems.push(Problem::Language(value.to_owned())),
            },
            "export_padding" => match parse_padding(value) {
                Some(v) => settings.export_padding = v,
                None => invalid("export_padding"),
            },
            "min_undo_steps" => match value.parse::<u32>().ok().filter(|n| *n <= MAX_MIN_UNDO_STEPS) {
                Some(v) => settings.min_undo_steps = v,
                None => invalid("min_undo_steps"),
            },
            "cpu_threads" => match parse_threads(value) {
                Some(v) => settings.cpu_threads = v,
                None => invalid("cpu_threads"),
            },
            "compositing" => match Compositing::ALL.into_iter().find(|c| c.key() == value) {
                Some(c) => settings.compositing = c,
                None => invalid("compositing"),
            },
            "library_folder" => match parse_folder(value) {
                Some(v) => settings.library_folder = v,
                None => invalid("library_folder"),
            },
            "backups" => match parse_backups(value) {
                Some(keep) => settings.backups = keep,
                None => problems.push(Problem::Backups(value.to_owned())),
            },
            // 切ったときだけ書く行。読めない値は出す（既定）のまま、理由は出さない
            "selection_bar" => settings.selection_bar = value != "off",
            "pressure_low" => match value.parse::<f32>().ok().filter(|v| (0.0..=1.0 - MIN_SPAN).contains(v)) {
                Some(v) => low = Some(v),
                None => invalid("pressure_low"),
            },
            "pressure_high" => match value.parse::<f32>().ok().filter(|v| (MIN_SPAN..=1.0).contains(v)) {
                Some(v) => high = Some(v),
                None => invalid("pressure_high"),
            },
            "pressure_curve" => match crate::brushes::store::parse_curve(value)
                .filter(|points| PressureAdjust::new(0.0, 1.0, points.clone()).is_ok())
            {
                Some(points) => curve = Some(points),
                None => invalid("pressure_curve"),
            },
            "view3d_orbit" | "view3d_zoom" => settings.navigation.parse(key.trim(), value, &mut problems),
            "uv_wireframe" => settings.uv_wireframe = value != "off",
            "uv_wireframe_color" => match crate::uv_wireframe::parse_color(value) { Some(c) => settings.uv_wireframe_color = c, None => invalid("uv_wireframe_color") },
            "livelink_on_startup" => settings.livelink_on_startup = value != "off",
            "color_wheel" => settings.color_wheel = value != "off",
            other => {
                if let Some(kind) = BudgetKind::ALL.into_iter().find(|k| k.key() == other) {
                    match parse_budget(kind, value) {
                        Some(b) => settings.set_budget(kind, b),
                        None => invalid(kind.key()),
                    }
                }
                // 知らないキーは読み飛ばす
            }
        }
    }
    let (low, high) = (low.unwrap_or(0.0), high.unwrap_or(1.0));
    match PressureAdjust::new(low, high, curve.unwrap_or_default()) {
        Ok(adjust) => settings.pressure = adjust,
        // 下限と上限が近すぎる: 組として使えないので、筆圧の調整は全部既定に戻す
        Err(_) => problems.push(Problem::Invalid {
            key: "pressure_high",
            value: format!("{high}"),
        }),
    }
    (settings, problems)
}

fn parse_padding(value: &str) -> Option<i32> {
    if value == "fill" {
        return Some(-1);
    }
    let n: i32 = value.parse().ok()?;
    (n >= 0 && EXPORT_PADDINGS.contains(&n)).then_some(n)
}

fn parse_budget(kind: BudgetKind, value: &str) -> Option<Budget> {
    if value == "auto" {
        return Some(Budget::Auto);
    }
    let n: u32 = value.parse().ok()?;
    let (lo, hi) = kind.range();
    (lo..=hi).contains(&n).then_some(Budget::Mib(n))
}

/// `auto` か 1〜上限の数（Some(None) が自動）。
fn parse_threads(value: &str) -> Option<Option<u32>> {
    if value == "auto" {
        return Some(None);
    }
    let n: u32 = value.parse().ok()?;
    (1..=MAX_CPU_THREADS).contains(&n).then_some(Some(n))
}

/// `all`（すべて残す）か、0〜上限の数。
fn parse_backups(value: &str) -> Option<BackupKeep> {
    if value == "all" {
        return Some(BackupKeep::All);
    }
    let n: u32 = value.parse().ok()?;
    (n <= MAX_BACKUPS_TO_KEEP).then_some(BackupKeep::Count(n))
}

/// 空は既定（Some(None)）、絶対パスはそのまま。相対パスは、どこからの相対か決まらないので断る。
fn parse_folder(value: &str) -> Option<Option<PathBuf>> {
    if value.is_empty() {
        return Some(None);
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(Some(path))
}

/// 書く内容。言語は常に、ほかは既定でないものだけ（何も変えていない間は今までと同じ中身）。
fn render(settings: &Settings) -> String {
    let mut text = format!("language={}\n", settings.lang.pick("ja", "en"));
    let default = Settings::default();
    if settings.export_padding != default.export_padding {
        let value = if settings.export_padding < 0 {
            "fill".to_owned()
        } else {
            settings.export_padding.to_string()
        };
        text += &format!("export_padding={value}\n");
    }
    for kind in BudgetKind::ALL {
        if let Budget::Mib(n) = settings.budget(kind) {
            let (lo, hi) = kind.range();
            text += &format!("{}={}\n", kind.key(), n.clamp(lo, hi));
        }
    }
    if settings.min_undo_steps != default.min_undo_steps {
        text += &format!("min_undo_steps={}\n", settings.min_undo_steps.min(MAX_MIN_UNDO_STEPS));
    }
    if let Some(n) = settings.cpu_threads {
        text += &format!("cpu_threads={}\n", n.clamp(1, MAX_CPU_THREADS));
    }
    if settings.compositing != default.compositing {
        text += &format!("compositing={}\n", settings.compositing.key());
    }
    if let BackupKeep::Count(n) = settings.backups {
        text += &format!("backups={}\n", n.min(MAX_BACKUPS_TO_KEEP));
    }
    if !settings.selection_bar {
        text += "selection_bar=off\n";
    }
    let pressure = &settings.pressure;
    if pressure.low() != 0.0 {
        text += &format!("pressure_low={}\n", pressure.low());
    }
    if pressure.high() != 1.0 {
        text += &format!("pressure_high={}\n", pressure.high());
    }
    if !pressure.curve().is_empty() {
        text += &format!("pressure_curve={}\n", crate::brushes::store::curve_text(pressure.curve()));
    }
    settings.navigation.write(&mut text);
    crate::uv_wireframe::save_settings(&mut text, settings);
    if !settings.livelink_on_startup {
        text += "livelink_on_startup=off\n";
    }
    if !settings.color_wheel {
        text += "color_wheel=off\n";
    }
    // 改行を含むパスは書かない（読めなくなる）
    if let Some(folder) = settings.library_folder.as_ref().filter(|p| p.is_absolute()) {
        let shown = folder.to_string_lossy();
        if !shown.contains('\n') {
            text += &format!("library_folder={shown}\n");
        }
    }
    text
}

pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let text = render(settings);
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "settings too large"));
    }
    let pending = path.with_extension(format!("{}.pending", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&pending)?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&pending, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

/// 起動のときに、設定のファイルの CPU のスレッドの数を rayon の全体のスレッドプールに入れる（最初の rayon の利用より前に 1 回。
/// 読めない設定・既に使われた後は何もしない）。自動なら rayon の既定（論理プロセッサの数）のまま。
pub fn apply_thread_setting() {
    let Some(path) = path() else { return };
    let (settings, _) = load(&path);
    if let Some(n) = settings.cpu_threads {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(n as usize).build_global();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-tests").join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn with_lang(lang: Lang) -> Settings {
        Settings { lang, ..Settings::default() }
    }

    fn custom(dir: &Path) -> Settings {
        Settings {
            lang: Lang::En,
            export_padding: 8,
            undo_budget: Budget::Mib(512),
            source_budget: Budget::Mib(4096),
            stroke_budget: Budget::Auto,
            min_undo_steps: 12,
            cpu_threads: Some(4),
            compositing: Compositing::Cpu,
            library_folder: Some(dir.join("shelf")),
            backups: BackupKeep::Count(7),
            selection_bar: true,
            pressure: PressureAdjust::new(
                0.125,
                0.875,
                vec![
                    yolu_core::generator::CurvePoint { x: 0.0, y: 0.0 },
                    yolu_core::generator::CurvePoint { x: 0.4, y: 0.6 },
                    yolu_core::generator::CurvePoint { x: 1.0, y: 1.0 },
                ],
            )
            .unwrap(),
            navigation: crate::view3d::navigation::Preferences::default(),
            uv_wireframe: true,
            uv_wireframe_color: crate::uv_wireframe::DEFAULT_COLOR,
            livelink_on_startup: true,
            color_wheel: true,
        }
    }

    #[test]
    fn live_link_startup_defaults_on_and_survives_restart_when_disabled() {
        let dir = temp_dir("livelink");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.livelink_on_startup);
        let off = Settings { livelink_on_startup: false, ..Settings::default() };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path).unwrap().contains("livelink_on_startup=off"));
        save(&path, &Settings::default()).unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![]));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("livelink_on_startup"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_color_wheel_defaults_on_and_survives_restart_when_switched_off() {
        let dir = temp_dir("colorwheel");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.color_wheel);
        let off = Settings { color_wheel: false, ..Settings::default() };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path).unwrap().contains("color_wheel=off"));
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("color_wheel"));
        // 切り替えは AppState の設定に出て、読んだ設定は AppState へ入る
        let mut state = crate::state::AppState::new(8, 8);
        assert!(state.color.wheel && state.settings().color_wheel);
        state.apply(crate::state::Action::ToggleColorWheel);
        assert!(!state.settings().color_wheel);
        let mut again = crate::state::AppState::new(8, 8);
        again.load_settings(state.settings());
        assert!(!again.color.wheel);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_directories_follow_each_os_and_reject_relative_xdg() {
        let base = std::env::current_dir().unwrap();
        let home = base.join("example");
        let env = |key: &str| match key {
            "HOME" => Some(home.clone()),
            "APPDATA" => Some(base.join("roaming")),
            "XDG_CONFIG_HOME" => Some(PathBuf::from("relative")),
            _ => None,
        };
        assert_eq!(config_path("linux", env).unwrap(), home.join(".config/YoluPainter/settings.conf"));
        assert_eq!(config_path("macos", env).unwrap(), home.join("Library/Application Support/YoluPainter/settings.conf"));
        assert_eq!(config_path("windows", env).unwrap(), base.join("roaming/YoluPainter/settings.conf"));
        assert!(config_path("linux", |_| None).is_none());
        assert_eq!(config_path("linux", |_| Some(base.join("config"))).unwrap(), base.join("config/YoluPainter/settings.conf"));
    }

    #[test]
    fn language_survives_restart_and_failed_replace_preserves_settings() {
        let dir = temp_dir("language");
        let path = dir.join("settings.conf");
        assert_eq!(load(&path), (Settings::default(), vec![]));
        assert_eq!(Settings::default().lang, Lang::Ja);
        for lang in [Lang::En, Lang::Ja, Lang::En] {
            save(&path, &with_lang(lang)).unwrap();
            assert_eq!(load(&path), (with_lang(lang), vec![]));
        }
        let pending = path.with_extension(format!("{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        assert!(save(&path, &with_lang(Lang::Ja)).is_err());
        assert_eq!(load(&path).0.lang, Lang::En);
        std::fs::write(&path, "language=unknown").unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![Problem::Language("unknown".into())]));
        std::fs::write(&path, vec![b'a'; 4097]).unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![Problem::Unreadable]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_setting_survives_a_restart_and_the_default_ones_are_not_written() {
        let dir = temp_dir("all");
        let path = dir.join("settings.conf");
        // 既定は言語の 1 行だけ（今までのファイルと同じ中身）
        save(&path, &with_lang(Lang::En)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        let all = custom(&dir);
        save(&path, &all).unwrap();
        assert_eq!(load(&path), (all.clone(), vec![]));
        let written = std::fs::read_to_string(&path).unwrap();
        for line in [
            "language=en",
            "export_padding=8",
            "undo_budget_mib=512",
            "source_budget_mib=4096",
            "min_undo_steps=12",
            "cpu_threads=4",
            "compositing=cpu",
            "backups=7",
            "pressure_low=0.125",
            "pressure_high=0.875",
            "pressure_curve=0:0,0.4:0.6,1:1",
        ] {
            assert!(written.lines().any(|l| l == line), "{line}\n{written}");
        }
        assert!(!written.contains("stroke_budget_mib"), "自動は書かない");
        assert!(written.contains("library_folder="), "{written}");
        // 選び直して既定に戻すと、行が消える
        let mut back = all.clone();
        back.export_padding = DEFAULT_EXPORT_PADDING;
        back.undo_budget = Budget::Auto;
        back.source_budget = Budget::Auto;
        back.min_undo_steps = DEFAULT_MIN_UNDO_STEPS;
        back.cpu_threads = None;
        back.compositing = Compositing::Auto;
        back.library_folder = None;
        back.backups = BackupKeep::All;
        back.pressure = PressureAdjust::default();
        save(&path, &back).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        // 範囲の端の値
        let mut edge = Settings {
            export_padding: 0,
            undo_budget: Budget::Mib(0),
            source_budget: Budget::Mib(32768),
            stroke_budget: Budget::Mib(8),
            cpu_threads: Some(MAX_CPU_THREADS),
            min_undo_steps: 0,
            ..Settings::default()
        };
        save(&path, &edge).unwrap();
        assert_eq!(load(&path), (edge.clone(), vec![]));
        // 届くかぎり全部（-1）は既定なので書かず、fill と書いても読める
        edge.export_padding = -1;
        assert!(!render(&edge).contains("export_padding"));
        assert_eq!(parse("export_padding=fill\n"), (Settings::default(), vec![]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_values_fall_back_to_the_default_one_by_one_with_a_reason() {
        let (read, problems) = parse("language=en\nundo_budget_mib=lots\ncompositing=cpu\n");
        assert_eq!(read.lang, Lang::En);
        assert_eq!(read.compositing, Compositing::Cpu, "正しい項目は生かす");
        assert_eq!(read.undo_budget, Budget::Auto);
        assert_eq!(problems, [Problem::Invalid { key: "undo_budget_mib", value: "lots".into() }]);
        // 範囲の外・負・小数・空・大文字・16 進・桁あふれ
        let cases: [(&'static str, &[&str]); 10] = [
            ("undo_budget_mib", &["-1", "16385", "1.5", "", "AUTO", "0x10", "99999999999999999999"]),
            ("source_budget_mib", &["15", "32769", "0"]),
            ("stroke_budget_mib", &["7", "8193"]),
            ("min_undo_steps", &["101", "-1", "auto", ""]),
            ("cpu_threads", &["0", "1025", "-2", "many"]),
            ("export_padding", &["1", "3", "65", "-1", "-5", "Fill", ""]),
            ("compositing", &["both", "GPU", ""]),
            ("pressure_low", &["-0.1", "0.95", "x", "", "NaN"]),
            ("pressure_high", &["0.05", "1.5", "x", ""]),
            ("pressure_curve", &["0:0", "0:0,1:2", "0:0,0.5:0.5", "0:0;1:1", "a:b", "", "0:0,0.001:0.5,1:1"]),
        ];
        for (key, values) in cases {
            for bad in values {
                let (read, problems) = parse(&format!("language=ja\n{key}={bad}\n"));
                assert_eq!(read, Settings::default(), "{key}={bad:?}");
                assert_eq!(problems, [Problem::Invalid { key, value: (*bad).into() }], "{key}={bad:?}");
            }
        }
        // 棚の場所: 相対パスは断る（どこからの相対か決まらない）、空は既定
        for bad in ["shelf", "./shelf", "../shelf"] {
            let (read, problems) = parse(&format!("library_folder={bad}\n"));
            assert_eq!(read.library_folder, None);
            assert_eq!(problems, [Problem::Invalid { key: "library_folder", value: bad.into() }]);
        }
        assert_eq!(parse("library_folder=\n"), (Settings::default(), vec![]));
        let absolute = std::env::current_dir().unwrap().join("shelf");
        assert_eq!(parse(&format!("library_folder={}\n", absolute.display())).0.library_folder, Some(absolute));
        // 複数の壊れた値は、全部の理由。書いていない項目は既定で理由を出さない
        let (read, problems) = parse("language=fr\ncpu_threads=0\nmin_undo_steps=-3\n");
        assert_eq!(read, Settings::default());
        assert_eq!(
            problems,
            [
                Problem::Language("fr".into()),
                Problem::Invalid { key: "cpu_threads", value: "0".into() },
                Problem::Invalid { key: "min_undo_steps", value: "-3".into() },
            ]
        );
        // 空白・空行・知らないキー（読み飛ばす。新しい版が足した項目で壊れない）・後ろの行が勝つ
        let (read, problems) = parse("\n  language = en \n future=1\n cpu_threads = 2 \ncpu_threads=3\n");
        assert_eq!((read.lang, read.cpu_threads), (Lang::En, Some(3)));
        assert!(problems.is_empty());
        // `キー=値` ではない行は、読めないファイル（全部既定）
        assert_eq!(parse("language=en\njunk\n"), (Settings::default(), vec![Problem::Unreadable]));
        // 筆圧の下限と上限は組: 1 つずつは範囲内でも、近すぎれば調整は全部既定に戻して理由を出す。ほかの項目は生かす
        let (read, problems) = parse("cpu_threads=2\npressure_low=0.5\npressure_high=0.55\npressure_curve=0:0,0.5:0.8,1:1\n");
        assert_eq!(read.pressure, PressureAdjust::default());
        assert_eq!(read.cpu_threads, Some(2));
        assert_eq!(problems, [Problem::Invalid { key: "pressure_high", value: "0.55".into() }]);
        // 片方だけ書いてあっても読める
        let (read, problems) = parse("pressure_high=0.8\n");
        assert_eq!((read.pressure.low(), read.pressure.high()), (0.0, 0.8));
        assert!(problems.is_empty());
    }

    #[test]
    fn a_pressure_curve_made_with_the_editor_is_restored_as_written() {
        // 編集の部品が作る曲線の半端な値は、調整が f32 に丸めて持つので、保存して読み戻しても同じ値になる
        use crate::ui::curve::ops;
        use yolu_core::curve::Curve;
        let dir = temp_dir("editor-curve");
        let path = dir.join("settings.conf");
        let (c, k) = ops::add_point(&Curve::identity(), 0.373_737_373_7, 0.616_161_616_1).unwrap();
        let c = ops::move_point(&c, k, 0.412_345_678_91, 0.777_777_777_7).unwrap();
        let pressure = PressureAdjust::new(0.1, 0.9, vec![]).unwrap().with_curve_shape(c).unwrap();
        let settings = Settings { pressure, ..Settings::default() };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), (settings.clone(), vec![]));
        // 直線へ戻すと、曲線の行は消える
        let straight = Settings {
            pressure: settings.pressure.with_curve_shape(Curve::identity()).unwrap(),
            ..Settings::default()
        };
        save(&path, &straight).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("pressure_curve"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backups_to_keep_is_written_only_when_it_is_not_the_default_and_restored() {
        let dir = temp_dir("backups");
        let path = dir.join("settings.conf");
        let with = |lang, backups| Settings { lang, backups, ..Settings::default() };
        // 既定（すべて残す）は書かない。今までのファイルと同じ中身
        save(&path, &with(Lang::En, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        for keep in [BackupKeep::Count(0), BackupKeep::Count(1), BackupKeep::Count(37), BackupKeep::Count(MAX_BACKUPS_TO_KEEP)] {
            save(&path, &with(Lang::Ja, keep)).unwrap();
            assert_eq!(load(&path), (with(Lang::Ja, keep), vec![]), "{keep:?}");
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\nbackups=1000\n");
        // 選び直して「すべて」に戻すと、行は消える
        save(&path, &with(Lang::Ja, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
        // 上限を超えて渡されても、書くのは上限
        save(&path, &with(Lang::Ja, BackupKeep::Count(5000))).unwrap();
        assert_eq!(load(&path).0.backups, BackupKeep::Count(MAX_BACKUPS_TO_KEEP));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_selection_bar_is_written_only_when_it_is_off_and_restored() {
        let dir = temp_dir("selection-bar");
        let path = dir.join("settings.conf");
        save(&path, &with_lang(Lang::Ja)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n", "出す（既定）は書かない");
        let off = Settings { selection_bar: false, backups: BackupKeep::Count(3), ..with_lang(Lang::En) };
        save(&path, &off).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\nbackups=3\nselection_bar=off\n");
        assert_eq!(load(&path), (off, vec![]));
        // 知らない値は既定（出す）。理由は出さない
        assert_eq!(parse("selection_bar=maybe\n"), (Settings::default(), vec![]));
        assert_eq!(parse("selection_bar=on\n"), (Settings::default(), vec![]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_backups_value_falls_back_to_keeping_all_with_a_reason_and_the_rest_is_kept() {
        let (read, problems) = parse("language=en\nbackups=many\ncompositing=cpu\n");
        assert_eq!((read.lang, read.backups, read.compositing), (Lang::En, BackupKeep::All, Compositing::Cpu));
        assert_eq!(problems, [Problem::Backups("many".into())]);
        // 範囲の外・負・小数・空・大文字・16 進・桁あふれ・上限の次
        for bad in ["-1", "1001", "5000", "2.5", "", "ALL", "0x10", "99999999999999999999"] {
            let (read, problems) = parse(&format!("backups={bad}\n"));
            assert_eq!(read.backups, BackupKeep::All, "{bad:?}");
            assert_eq!(problems, [Problem::Backups(bad.into())], "{bad:?}");
        }
        assert_eq!(parse("backups=7\n").0.backups, BackupKeep::Count(7));
        assert_eq!(parse("backups=all\n"), (Settings::default(), vec![]));
        assert_eq!(parse("backups = 12 \nbackups=13\n").0.backups, BackupKeep::Count(13), "後ろの行が勝つ");
    }

    #[test]
    fn problems_have_short_reasons_in_both_languages_without_the_other_one() {
        let problems = [
            Problem::Unreadable,
            Problem::Language("x".into()),
            Problem::Invalid { key: "undo_budget_mib", value: "5000".into() },
            Problem::Invalid { key: "library_folder", value: "rel".into() },
            Problem::Backups("5000".into()),
        ];
        for lang in Lang::ALL {
            let texts: Vec<String> = problems.iter().map(|p| p.text(lang)).collect();
            for (i, a) in texts.iter().enumerate() {
                assert_eq!(a.is_ascii(), lang == Lang::En, "{lang:?} {a}");
                assert!(texts.iter().skip(i + 1).all(|b| a != b));
            }
            assert!(texts[2].contains("5000") && texts[4].contains("5000"));
        }
        assert_eq!(problems[4].text(Lang::Ja), "退避を残す数の設定が正しくありません（5000）。すべて残します。");
        assert_eq!(problems[4].text(Lang::En), "Invalid Backups to Keep setting (5000); keeping all.");
        assert_eq!(problems[2].text(Lang::Ja), "取り消し履歴の設定が正しくありません（5000）。既定に戻します。");
        assert_eq!(problems[2].text(Lang::En), "Invalid Undo history setting (5000); using the default.");
        // 長い値は切って、帯を溢れさせない
        let long = Problem::Invalid { key: "cpu_threads", value: "9".repeat(500) }.text(Lang::En);
        assert!(long.len() < 100, "{long}");
        let long = Problem::Backups("9".repeat(500)).text(Lang::En);
        assert!(long.len() < 100, "{long}");
        // どのキーにも名前がある
        for key in ["language", "export_padding", "min_undo_steps", "cpu_threads", "compositing", "library_folder", "backups"]
            .into_iter()
            .chain(BudgetKind::ALL.iter().map(|k| k.key()))
        {
            for lang in Lang::ALL {
                assert_ne!(setting_name(lang, key), setting_name(lang, "unknown"), "{key}");
            }
        }
    }

    #[test]
    fn automatic_budgets_follow_the_memory_like_unity_does() {
        // 16 GB: 取り消し 1024・画素 2048・1 回の操作 512
        assert_eq!(BudgetKind::Undo.automatic_mib(16384), 1024);
        assert_eq!(BudgetKind::Source.automatic_mib(16384), 2048);
        assert_eq!(BudgetKind::Stroke.automatic_mib(16384), 512);
        // 小さい機械の下限と、大きい機械の上限
        assert_eq!(BudgetKind::Undo.automatic_mib(1024), 256);
        assert_eq!(BudgetKind::Source.automatic_mib(1024), 256);
        assert_eq!(BudgetKind::Stroke.automatic_mib(1024), 64);
        assert_eq!(BudgetKind::Undo.automatic_mib(1 << 20), 2048);
        assert_eq!(BudgetKind::Source.automatic_mib(1 << 20), 8192);
        assert_eq!(BudgetKind::Stroke.automatic_mib(1 << 20), 1024);
        // 設定からバイトへ: 自動はメモリから、数はそのまま
        let s = Settings {
            source_budget: Budget::Mib(100),
            min_undo_steps: 7,
            ..Settings::default()
        };
        assert_eq!(
            s.budgets(16384),
            Budgets { undo: 1024 << 20, source: 100 << 20, stroke: 512 << 20, min_undo_steps: 7 }
        );
        // 選択肢は範囲の中で、昇順
        for kind in BudgetKind::ALL {
            let (lo, hi) = kind.range();
            let choices = kind.choices();
            assert!(choices.windows(2).all(|w| w[0] < w[1]), "{kind:?}");
            assert!(choices.iter().all(|c| (lo..=hi).contains(c)), "{kind:?}");
        }
    }

    #[test]
    fn the_memory_is_read_from_meminfo_and_has_a_floor() {
        assert_eq!(parse_meminfo("MemTotal:       16384000 kB\nMemFree: 1 kB\n"), Some(16000));
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: many kB\n"), None);
        assert!(system_memory_mib() >= 1024);
    }

    #[test]
    fn the_library_folder_falls_back_to_the_default_next_to_the_settings() {
        let mut s = Settings::default();
        assert_eq!(s.library_folder(), default_library_folder());
        if let Some(default) = default_library_folder() {
            assert!(default.ends_with("YoluPainter/Library") || default.ends_with("YoluPainter\\Library"), "{default:?}");
        }
        let chosen = std::env::current_dir().unwrap().join("mine");
        s.library_folder = Some(chosen.clone());
        assert_eq!(s.library_folder(), Some(chosen));
    }
}
