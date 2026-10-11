//! 利用者ごとのアプリ設定。文書・試験の AppState とは独立して読み書きする。
//!
//! 設定のファイルは `キー=値` を 1 行ずつ。**既定の値は書かない**（言語は常に書く）ので、何も変えていない間は今までと同じ中身で、
//! 知らないキーは読み飛ばす（新しい版が足した項目で壊れない）。正しくない値は、その項目だけを既定へ戻して理由（`Problem`）を返し、
//! ほかの項目は生かす。読んだだけではファイルに触らず、設定を変えて書き直すときに置き換える。
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::engine::DEFAULT_SOURCE_BUDGET_BYTES;
use crate::gpu_memory::GpuMemory;
use crate::lang::Lang;
use crate::pen::adjust::{PressureAdjust, MIN_SPAN};

/// 設定のファイルの場所（設定のフォルダが分からなければ None）。
pub fn path() -> Option<PathBuf> {
    config_path(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })
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
    Some(
        config_base(os, env)?
            .join("YoluPainter")
            .join("settings.conf"),
    )
}

/// 棚の場所の既定（設定のフォルダの下の Library）。設定のフォルダが分からなければ None。
pub fn default_library_folder() -> Option<PathBuf> {
    config_base(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })
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
    /// 全レイヤーの画素が使うメモリの合計（画面の名前は「レイヤーのメモリ」）。
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

    /// 指定できる MiB の範囲。取り消し履歴の 0 は、最小の段数だけを残す。上限は自動の最大（`automatic_mib` の上限）以上。
    /// 古い版（上限 32768）が書いた値はすべてこの範囲に入るので、そのまま読める。
    pub fn range(self) -> (u32, u32) {
        match self {
            BudgetKind::Undo => (0, 16384),
            BudgetKind::Source => (16, 65536),
            BudgetKind::Stroke => (8, 8192),
        }
    }

    /// 自動のときの MiB: 物理メモリの一定の割合（レイヤーのメモリ 1/2・取り消し 1/8・1 回の操作 1/16）を、小さい機械でも作業できる
    /// 下限と、メモリを積んだ機械でも頭打ちにする上限で挟む（16 GB で 取り消し 2048・レイヤーのメモリ 8192・1 回の操作 1024。
    /// 64 GB で 8192・32768・4096。上限は 16384・65536・4096）。
    ///
    /// 予算は使えるメモリの天井で、予約ではない: 文書が実際に使うときに取り、使っていない分は先に確保しない（予算の値で配列を作る所は
    /// 無い）。大きな値を既定にしても、小さな文書を開いているアプリが使うメモリは変わらない。3 つの割合の合計（11/16）は 3 つが同時に天井まで
    /// 使われる前提ではなく、超える操作を断る・古い履歴を捨てる境目の位置を決める。
    pub fn automatic_mib(self, ram_mib: u64) -> u32 {
        let (div, lo, hi) = match self {
            BudgetKind::Undo => (8, 256, 16384),
            BudgetKind::Source => (2, 256, 65536),
            BudgetKind::Stroke => (16, 64, 4096),
        };
        (ram_mib / div).clamp(lo, hi) as u32
    }

    /// 予算の選択肢（自動のほかに並べる MiB）。
    pub fn choices(self) -> &'static [u32] {
        match self {
            BudgetKind::Undo => &[0, 256, 512, 1024, 2048, 4096, 8192, 16384],
            BudgetKind::Source => &[512, 1024, 2048, 4096, 8192, 16384, 32768, 65536],
            BudgetKind::Stroke => &[64, 128, 256, 512, 1024, 2048, 4096],
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

/// Windows でペンの筆圧・傾きなどを読む方式（設定「ペンの入力」。Windows だけで効くが、設定のファイルには同じ書き方で残す）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PenApi {
    /// Windows Ink（WM_POINTER）。
    #[default]
    Ink,
    /// WinTab（`Wintab32.dll`。Wacom などのドライバーが出す）。使えない機械では Windows Ink に戻る。
    WinTab,
}

impl PenApi {
    pub const ALL: [PenApi; 2] = [PenApi::Ink, PenApi::WinTab];

    pub fn key(self) -> &'static str {
        match self {
            PenApi::Ink => "ink",
            PenApi::WinTab => "wintab",
        }
    }
}

/// ディスクキャッシュに使う量の上限の指定: 自動（64 GiB と、置き場所の起動したときの空きの半分の小さい方）か GiB。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskLimit {
    Auto,
    Gib(u32),
}

impl DiskLimit {
    /// 選択肢（自動のほかに並べる GiB）。
    pub const CHOICES: [u32; 8] = [4, 8, 16, 32, 64, 128, 256, 512];
    /// 指定できる GiB の範囲。
    pub const RANGE: (u32, u32) = (1, 4096);
    /// 自動の上限（GiB）。
    pub const AUTO_MAX_GIB: u32 = 64;

    /// バイト。自動は 64 GiB と、空き（分かれば）の半分の小さい方。
    pub fn bytes(self, free: Option<u64>) -> u64 {
        const GIB: u64 = 1 << 30;
        match self {
            DiskLimit::Auto => {
                let most = DiskLimit::AUTO_MAX_GIB as u64 * GIB;
                free.map_or(most, |f| (f / 2).min(most))
            }
            DiskLimit::Gib(n) => n as u64 * GIB,
        }
    }
}

/// 書き出しのパディングに選べる値（テクセル。-1 は無限に広げる、0 は塗り広げない）。
pub const EXPORT_PADDINGS: [i32; 8] = [0, 2, 4, 8, 16, 32, 64, -1];
/// 書き出しのパディングの既定。
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
    /// 書き出しで UV の外へ色を塗り広げるテクセルの数（-1 は無限に広げる）。
    pub export_padding: i32,
    /// 書き出しのウィンドウで最後に選んだ出力テンプレート（次に開いたときの選び。文書ごとではなく設定に覚える）。設定のウィンドウの区分には出さない（書き出しのウィンドウが持つ）。
    pub export_form: crate::export::ExportForm,
    pub undo_budget: Budget,
    pub source_budget: Budget,
    pub stroke_budget: Budget,
    /// 取り消し履歴の予算を超えても残す直近の段数。
    pub min_undo_steps: u32,
    /// CPU の処理に使うスレッドの数（None は自動 = 論理プロセッサの数。起動のときに決まる）。
    pub cpu_threads: Option<u32>,
    pub compositing: Compositing,
    /// 画面の更新を、モニターの垂直同期まで待たせるか（既定は待たない。待たないと画面の上下のずれ〔テアリング〕が出うる代わりに、
    /// ペンの入力から線が画面に出るまでの遅れが短い）。ウィンドウの面を作るときに決まるので、変えた値は次の起動から効く
    /// （`view3d::render::wgpu_configuration`）。
    pub vsync: bool,
    /// 棚の場所（None は既定。`default_library_folder`）。
    pub library_folder: Option<PathBuf>,
    /// 上書き保存で置き換えた前の版（退避）をいくつ残すか。
    pub backups: BackupKeep,
    /// 選択範囲の下のボタンの帯を出すか（「選択範囲」メニューで切り替える。設定のウィンドウには無い）。
    pub selection_bar: bool,
    /// 選択のツールのツールプロパティで、作成方法を 4 つ（新規・追加・削除・共通）出すか（切は共通を畳む。設定のウィンドウには無い。
    /// 「共通」を選んでいるあいだは、この設定によらず 4 つ出す）。
    pub selection_all_modes: bool,
    /// 全体の筆圧の調整（端末ごと。ペンの筆圧を、ブラシへ渡す前に下限・上限と曲線で直す。設定の「ペン」の筆圧の調整）。
    pub pressure: PressureAdjust,
    /// macOS のタブレット（Wacom・XP-Pen などのドライバー）の筆圧・傾き・消しゴムの端を NSEvent から読むか（試し。既定は入。`pen::mac_tablet`）。切ると、ペンはマウスと同じに描く。
    /// macOS 以外では使わないが、設定のファイルには同じ書き方で残す（OS をまたいで設定のフォルダを共有しても消さない）。
    pub tablet_pressure: bool,
    /// Windows でペンを Windows Ink と WinTab のどちらで読むか（`pen::win_tab`。既定は Windows Ink）。Windows 以外では使わないが、
    /// 設定のファイルには同じ書き方で残す（OS をまたいで設定のフォルダを共有しても消さない）。
    pub pen_input: PenApi,
    pub navigation: crate::view3d::navigation::Preferences,
    /// 3D の絵の仕上げ（アンチエイリアス・ブルーム。「3D ビューの設定 → 画質」）。
    pub view3d_post: crate::view3d::display::PostFx,
    /// 3D の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・継ぎ目のにじみ。ブラシの詳細の「ストローク」の 3D の組）。
    pub view3d_paint: yolu_core::geometry::ProjectionSettings,
    pub uv_wireframe: bool,
    pub uv_wireframe_color: [u8; 4],
    /// 重なった UV のテクセルとアイランドの縁を 2D のキャンバスに出すか（表示のメニュー）と、その色。
    pub uv_overlap: bool,
    pub uv_overlap_color: [u8; 4],
    /// Unity からの Live Link の頼みを受けるか（設定のファイルのキーは前の版と同じ `livelink_on_startup`。--livelink で起動すると、
    /// この設定によらず受ける）。
    pub livelink_on_startup: bool,
    /// Live Link で Unity から受けたマテリアルの値を .ylp に保存するか（`look.json` の `received`。既定は保存する）。
    pub livelink_keep_values: bool,
    /// 外からの操作（MCP のクライアント・コマンドラインなど、同じ PC のプログラムからの命令）を受けるか。既定は切。入っている間だけ
    /// `http://127.0.0.1:<external_ops_port>/mcp` で待つ（`mcp_server`）。
    pub external_ops: bool,
    /// 外からの操作を待つ番号（1024〜65535。既定は `yolu_mcp::DEFAULT_PORT`）。
    pub external_ops_port: u16,
    /// カラーの欄を色相の円と中の四角で出すか（切ると四角と色相の帯）。
    pub color_wheel: bool,
    /// GPU のメモリ（3D の絵・キャンバスの GPU の合成・棚のサムネイルへ配る合計。配り方は `gpu_memory`）。
    pub gpu_memory: GpuMemory,
    /// ベイクで GPU の RT コア（ray query）を使うか（既定は入。使えない GPU や、自己照合に通らないときは compute に戻る。切ると常に compute。
    /// ドライバーが固まる PC で切る。環境変数 `YOLUPAINTER_BAKE_RAY_QUERY=0` でも既定が切になる）。画面の描画の方式は変えない。
    pub bake_ray_query: bool,
    /// メモリの予算（レイヤーのメモリ＋取り消し履歴）を超えた分のタイルの中身を、ディスクへ逃がすか（`yolu_core::tile_cache`）。既定は入。
    pub disk_cache: bool,
    /// ディスクキャッシュのファイルを置くフォルダ（None は OS の一時フォルダ）。
    pub disk_cache_folder: Option<PathBuf>,
    /// ディスクキャッシュに使う量の上限。
    pub disk_cache_limit: DiskLimit,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::default(),
            export_padding: DEFAULT_EXPORT_PADDING,
            export_form: crate::export::ExportForm::default(),
            undo_budget: Budget::Auto,
            source_budget: Budget::Auto,
            stroke_budget: Budget::Auto,
            min_undo_steps: DEFAULT_MIN_UNDO_STEPS,
            cpu_threads: None,
            compositing: Compositing::Auto,
            vsync: false,
            library_folder: None,
            backups: BackupKeep::All,
            selection_bar: true,
            selection_all_modes: false,
            pressure: PressureAdjust::default(),
            tablet_pressure: true,
            pen_input: PenApi::default(),
            navigation: crate::view3d::navigation::Preferences::default(),
            view3d_post: crate::view3d::display::PostFx::default(),
            view3d_paint: yolu_core::geometry::ProjectionSettings::default(),
            uv_wireframe: true,
            uv_wireframe_color: crate::uv_wireframe::DEFAULT_COLOR,
            uv_overlap: true,
            uv_overlap_color: crate::uv_wireframe::DEFAULT_OVERLAP_COLOR,
            livelink_on_startup: true,
            livelink_keep_values: true,
            external_ops: false,
            external_ops_port: yolu_mcp::DEFAULT_PORT,
            color_wheel: true,
            gpu_memory: GpuMemory::Auto,
            bake_ray_query: true,
            disk_cache: true,
            disk_cache_folder: None,
            disk_cache_limit: DiskLimit::Auto,
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

    /// 読み込み（.ylp を開く・書き出しが写した文書を戻す）で 1 つの文書に許すレイヤーの画素のバイト数: 設定の予算と core の既定の大きい方。
    /// 設定を上げれば大きな文書も読める。設定を下げても、読めていた文書は読める（今の画素が予算を超えるときは `sync_budgets` が
    /// 予算をその量まで広げて知らせる）。
    pub fn load_source_bytes(&self, ram_mib: u64) -> u64 {
        self.budgets(ram_mib)
            .source
            .max(DEFAULT_SOURCE_BUDGET_BYTES)
    }

    /// 棚の場所（設定になければ既定。設定のフォルダも分からなければ None）。
    pub fn library_folder(&self) -> Option<PathBuf> {
        self.library_folder.clone().or_else(default_library_folder)
    }

    /// ディスクキャッシュの置き場所（設定になければ OS の一時フォルダ）。
    pub fn disk_cache_folder(&self) -> PathBuf {
        self.disk_cache_folder
            .clone()
            .unwrap_or_else(std::env::temp_dir)
    }

    /// タイルの中身を逃がす係に入れる設定。メモリの上限は、レイヤーのメモリと取り消し履歴の予算の和。`free` は置き場所の、起動した
    /// ときの空き（自動のディスクの上限の元。分からなければ None）。
    pub fn cache_settings(
        &self,
        ram_mib: u64,
        free: Option<u64>,
    ) -> yolu_core::tile_cache::CacheSettings {
        let budgets = self.budgets(ram_mib);
        yolu_core::tile_cache::CacheSettings {
            enabled: self.disk_cache,
            folder: Some(self.disk_cache_folder()),
            memory_limit: budgets.source.saturating_add(budgets.undo),
            disk_limit: self.disk_cache_limit.bytes(free),
        }
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
    let out = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
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
            Self::Unreadable => lang
                .pick("設定を読めません。", "Cannot read the settings.")
                .into(),
            Self::Language(_) => lang
                .pick(
                    "言語の設定を読めません。",
                    "Cannot read the language setting.",
                )
                .into(),
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
        "view3d_axis_ortho" => lang.pick("軸の向きで正投影", "Orthographic on axis views"),
        "view3d_antialias" => lang.pick("アンチエイリアス", "Anti-aliasing"),
        "view3d_bloom" => lang.pick("ブルーム", "Bloom"),
        "view3d_bloom_strength" => lang.pick("ブルームの強さ", "Bloom strength"),
        "view3d_bloom_threshold" => lang.pick("ブルームのしきい値", "Bloom threshold"),
        "view3d_paint_hidden" => lang.pick("隠れた所も塗る", "Paint hidden areas"),
        "view3d_paint_backfaces" => lang.pick("裏の面も塗る", "Paint back faces"),
        "view3d_paint_falloff" => lang.pick("面の向きで弱める", "Fade by angle"),
        "view3d_paint_falloff_start" => lang.pick("弱め始め", "Fade start"),
        "view3d_paint_falloff_end" => lang.pick("塗らない角度", "Fade end"),
        "view3d_paint_seam_bleed" => lang.pick("継ぎ目のにじみ", "Seam bleed"),
        "export_padding" => lang.pick("書き出しのパディング", "Export padding"),
        "export_form" => lang.pick("出力テンプレート", "Output template"),
        "undo_budget_mib" => lang.pick("取り消し履歴", "Undo history"),
        "source_budget_mib" => lang.pick("レイヤーのメモリ", "Layer memory"),
        "stroke_budget_mib" => lang.pick("1 回の操作", "One operation"),
        "min_undo_steps" => lang.pick("最小の取り消し段数", "Minimum undo steps"),
        "cpu_threads" => lang.pick("CPU のスレッド", "CPU threads"),
        "compositing" => lang.pick("表示の合成", "Display compositing"),
        "vsync" => lang.pick("垂直同期", "VSync"),
        "library_folder" => lang.pick("ライブラリの場所", "Library folder"),
        "backups" => lang.pick("退避を残す数", "Backups to Keep"),
        "gpu_memory" => lang.pick("GPU のメモリ", "GPU memory"),
        "bake_ray_query" => lang.pick("ベイクで RT コアを使う", "Use RT cores for baking"),
        "external_ops" => lang.pick("外からの操作を受ける", "Accept external commands"),
        "external_ops_port" => lang.pick("ポート番号", "Port"),
        "uv_wireframe_color" => lang.pick("UV ワイヤーフレームの色", "UV wireframe color"),
        "uv_overlap_color" => lang.pick("重なった UV の色", "Overlapping UV color"),
        "pressure_low" => lang.pick("筆圧の下限", "Pen pressure low"),
        "pressure_high" => lang.pick("筆圧の上限", "Pen pressure high"),
        "pressure_curve" => lang.pick("筆圧の曲線", "Pen pressure curve"),
        "tablet_pressure" => {
            lang.pick("タブレットの筆圧（試し）", "Tablet pressure (experimental)")
        }
        "pen_input" => lang.pick("ペンの入力", "Pen input"),
        "disk_cache" => lang.pick("ディスクキャッシュ", "Disk cache"),
        "disk_cache_folder" => lang.pick("キャッシュの場所", "Cache folder"),
        "disk_cache_limit_gib" => lang.pick("キャッシュの上限", "Cache limit"),
        _ => lang.pick("設定", "Setting"),
    }
}

// ───────── 読む・書く ─────────

/// 設定のファイルを読む。無ければ既定。値が正しくない項目は既定へ戻して `Problem` を返す（ファイルは、設定を変えて書き直すまで触らない）。
pub fn load(path: &Path) -> (Settings, Vec<Problem>) {
    let (settings, problems, _) = load_marked(path);
    (settings, problems)
}

/// アプリの起動で設定を読む。`load` と同じで、設定に言語が読めなかったとき（ファイルが無い・読めない・`language` の行が無い・値が正しくない）だけ、
/// 言語を `system`（OS の言語。`lang::system_lang`）にする。`language=ja|en` が読めたなら、いつもそれが先。
pub fn load_for_startup(path: &Path, system: Lang) -> (Settings, Vec<Problem>) {
    let (mut settings, problems, has_language) = load_marked(path);
    if !has_language {
        settings.lang = system;
    }
    (settings, problems)
}

/// `load` に、言語をファイルから読めたかを添えたもの。
fn load_marked(path: &Path) -> (Settings, Vec<Problem>, bool) {
    match read(path) {
        Ok(text) => parse_marked(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), Vec::new(), false),
        Err(_) => (Settings::default(), vec![Problem::Unreadable], false),
    }
}

fn read(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings too large",
        ));
    }
    Ok(text)
}

const MAX_FILE_BYTES: u64 = 4096;

#[cfg(test)]
fn parse(text: &str) -> (Settings, Vec<Problem>) {
    let (settings, problems, _) = parse_marked(text);
    (settings, problems)
}

/// `parse` に、言語（`language=ja|en`）を読めたかを添えたもの。読めなかった設定の言語は既定のまま（呼ぶ側が決める）。
fn parse_marked(text: &str) -> (Settings, Vec<Problem>, bool) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    let mut has_language = false;
    // 筆圧の調整は 3 つの項目が組で意味を持つので、読み終えてからまとめて作る
    let (mut low, mut high, mut curve) = (None, None, None);
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            return (Settings::default(), vec![Problem::Unreadable], false);
        };
        let value = value.trim();
        let mut invalid = |key: &'static str| {
            problems.push(Problem::Invalid {
                key,
                value: value.to_owned(),
            })
        };
        match key.trim() {
            "language" => match value {
                "ja" | "en" => {
                    settings.lang = if value == "ja" { Lang::Ja } else { Lang::En };
                    has_language = true;
                }
                _ => problems.push(Problem::Language(value.to_owned())),
            },
            "export_padding" => match parse_padding(value) {
                Some(v) => settings.export_padding = v,
                None => invalid("export_padding"),
            },
            "export_form" => match crate::export::ExportForm::from_key(value) {
                Some(form) => settings.export_form = form,
                None => invalid("export_form"),
            },
            "min_undo_steps" => match value
                .parse::<u32>()
                .ok()
                .filter(|n| *n <= MAX_MIN_UNDO_STEPS)
            {
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
            // 入れたときだけ書く行（既定は待たない）。読めない値は待たないまま
            "vsync" => settings.vsync = value == "on",
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
            // 入れたときだけ書く行（既定は切）。読めない値は切のまま
            "selection_all_modes" => settings.selection_all_modes = value == "on",
            "pressure_low" => match value
                .parse::<f32>()
                .ok()
                .filter(|v| (0.0..=1.0 - MIN_SPAN).contains(v))
            {
                Some(v) => low = Some(v),
                None => invalid("pressure_low"),
            },
            "pressure_high" => match value
                .parse::<f32>()
                .ok()
                .filter(|v| (MIN_SPAN..=1.0).contains(v))
            {
                Some(v) => high = Some(v),
                None => invalid("pressure_high"),
            },
            "pressure_curve" => match crate::brushes::store::parse_curve(value)
                .filter(|points| PressureAdjust::new(0.0, 1.0, points.clone()).is_ok())
            {
                Some(points) => curve = Some(points),
                None => invalid("pressure_curve"),
            },
            "view3d_orbit" | "view3d_zoom" | "view3d_axis_ortho" => {
                settings.navigation.parse(key.trim(), value, &mut problems)
            }
            "view3d_antialias" => match value
                .parse::<u32>()
                .ok()
                .filter(|n| crate::view3d::display::SAMPLE_CHOICES.contains(n))
            {
                Some(n) => settings.view3d_post.antialias = n,
                None => invalid("view3d_antialias"),
            },
            "view3d_bloom" => match value {
                "on" => settings.view3d_post.bloom = true,
                "off" => settings.view3d_post.bloom = false,
                _ => invalid("view3d_bloom"),
            },
            "view3d_bloom_strength" => match value
                .parse::<f32>()
                .ok()
                .filter(|v| (0.0..=crate::view3d::display::BLOOM_STRENGTH_MAX).contains(v))
            {
                Some(v) => settings.view3d_post.bloom_strength = v,
                None => invalid("view3d_bloom_strength"),
            },
            "view3d_bloom_threshold" => match value
                .parse::<f32>()
                .ok()
                .filter(|v| (0.0..=crate::view3d::display::BLOOM_THRESHOLD_MAX).contains(v))
            {
                Some(v) => settings.view3d_post.bloom_threshold = v,
                None => invalid("view3d_bloom_threshold"),
            },
            "view3d_paint_hidden"
            | "view3d_paint_backfaces"
            | "view3d_paint_falloff"
            | "view3d_paint_falloff_start"
            | "view3d_paint_falloff_end"
            | "view3d_paint_seam_bleed" => {
                if let Some(key) = parse_paint(&mut settings.view3d_paint, key.trim(), value) {
                    invalid(key);
                }
            }
            "uv_wireframe" => settings.uv_wireframe = value != "off",
            "uv_wireframe_color" => match crate::uv_wireframe::parse_color(value) {
                Some(c) => settings.uv_wireframe_color = c,
                None => invalid("uv_wireframe_color"),
            },
            "uv_overlap" => settings.uv_overlap = value != "off",
            "uv_overlap_color" => match crate::uv_wireframe::parse_color(value) {
                Some(c) => settings.uv_overlap_color = c,
                None => invalid("uv_overlap_color"),
            },
            "livelink_on_startup" => settings.livelink_on_startup = value != "off",
            "livelink_keep_values" => settings.livelink_keep_values = value != "off",
            // 入れたときだけ書く行（既定は切）。読めない値は切のまま
            "external_ops" => settings.external_ops = value == "on",
            "external_ops_port" => match value.parse::<u16>() {
                Ok(port) if yolu_mcp::valid_port(port) => settings.external_ops_port = port,
                _ => invalid("external_ops_port"),
            },
            "color_wheel" => settings.color_wheel = value != "off",
            // 切ったときだけ書く行（既定は入）。読めない値は入のまま
            "tablet_pressure" => settings.tablet_pressure = value != "off",
            "pen_input" => match PenApi::ALL.into_iter().find(|a| a.key() == value) {
                Some(a) => settings.pen_input = a,
                None => invalid("pen_input"),
            },
            "gpu_memory" => match GpuMemory::parse(value) {
                Some(v) => settings.gpu_memory = v,
                None => invalid("gpu_memory"),
            },
            // 切ったときだけ書く行（既定は入）。環境変数と同じく 0・false・off・no が切で、ほかの値は入のまま
            "bake_ray_query" => settings.bake_ray_query = !yolu_gpu::is_off_value(value),
            // 切ったときだけ書く行（既定は入）。読めない値は入のまま
            "disk_cache" => settings.disk_cache = value != "off",
            "disk_cache_folder" => match parse_folder(value) {
                Some(v) => settings.disk_cache_folder = v,
                None => invalid("disk_cache_folder"),
            },
            "disk_cache_limit_gib" => match parse_disk_limit(value) {
                Some(v) => settings.disk_cache_limit = v,
                None => invalid("disk_cache_limit_gib"),
            },
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
    // 弱め始めと塗らない角度は組で意味を持つ（塗らない角度は弱め始め以上）
    settings.view3d_paint = settings.view3d_paint.sanitized();
    let (low, high) = (low.unwrap_or(0.0), high.unwrap_or(1.0));
    match PressureAdjust::new(low, high, curve.unwrap_or_default()) {
        Ok(adjust) => settings.pressure = adjust,
        // 下限と上限が近すぎる: 組として使えないので、筆圧の調整は全部既定に戻す
        Err(_) => problems.push(Problem::Invalid {
            key: "pressure_high",
            value: format!("{high}"),
        }),
    }
    (settings, problems, has_language)
}

/// 3D の塗りの切り替えの 1 項目を読む（読めなければ、その項目は既定のまま、正しくない項目のキーを返す）。
fn parse_paint(
    paint: &mut yolu_core::geometry::ProjectionSettings,
    key: &str,
    value: &str,
) -> Option<&'static str> {
    let switch = |value: &str| match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    let angle = |value: &str| {
        value
            .parse::<f32>()
            .ok()
            .filter(|v| (0.0..=90.0).contains(v))
    };
    match key {
        "view3d_paint_hidden" => match switch(value) {
            Some(v) => paint.paint_hidden = v,
            None => return Some("view3d_paint_hidden"),
        },
        "view3d_paint_backfaces" => match switch(value) {
            Some(v) => paint.paint_backfaces = v,
            None => return Some("view3d_paint_backfaces"),
        },
        "view3d_paint_falloff" => match switch(value) {
            Some(v) => paint.angle_falloff = v,
            None => return Some("view3d_paint_falloff"),
        },
        "view3d_paint_falloff_start" => match angle(value) {
            Some(v) => paint.angle_start = v,
            None => return Some("view3d_paint_falloff_start"),
        },
        "view3d_paint_falloff_end" => match angle(value) {
            Some(v) => paint.angle_end = v,
            None => return Some("view3d_paint_falloff_end"),
        },
        _ => match value
            .parse::<u32>()
            .ok()
            .filter(|n| *n <= yolu_core::geometry::MAX_SEAM_BLEED)
        {
            Some(n) => paint.seam_bleed = n,
            None => return Some("view3d_paint_seam_bleed"),
        },
    }
    None
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

/// `auto` か、範囲の中の GiB。
fn parse_disk_limit(value: &str) -> Option<DiskLimit> {
    if value == "auto" {
        return Some(DiskLimit::Auto);
    }
    let n: u32 = value.parse().ok()?;
    let (lo, hi) = DiskLimit::RANGE;
    (lo..=hi).contains(&n).then_some(DiskLimit::Gib(n))
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
    if settings.export_form != default.export_form {
        text += &format!("export_form={}\n", settings.export_form.key());
    }
    for kind in BudgetKind::ALL {
        if let Budget::Mib(n) = settings.budget(kind) {
            let (lo, hi) = kind.range();
            text += &format!("{}={}\n", kind.key(), n.clamp(lo, hi));
        }
    }
    if settings.min_undo_steps != default.min_undo_steps {
        text += &format!(
            "min_undo_steps={}\n",
            settings.min_undo_steps.min(MAX_MIN_UNDO_STEPS)
        );
    }
    if let Some(n) = settings.cpu_threads {
        text += &format!("cpu_threads={}\n", n.clamp(1, MAX_CPU_THREADS));
    }
    if settings.compositing != default.compositing {
        text += &format!("compositing={}\n", settings.compositing.key());
    }
    if settings.vsync {
        text += "vsync=on\n";
    }
    if let BackupKeep::Count(n) = settings.backups {
        text += &format!("backups={}\n", n.min(MAX_BACKUPS_TO_KEEP));
    }
    if !settings.selection_bar {
        text += "selection_bar=off\n";
    }
    if settings.selection_all_modes {
        text += "selection_all_modes=on\n";
    }
    let pressure = &settings.pressure;
    if pressure.low() != 0.0 {
        text += &format!("pressure_low={}\n", pressure.low());
    }
    if pressure.high() != 1.0 {
        text += &format!("pressure_high={}\n", pressure.high());
    }
    if !pressure.curve().is_empty() {
        text += &format!(
            "pressure_curve={}\n",
            crate::brushes::store::curve_text(pressure.curve())
        );
    }
    if !settings.tablet_pressure {
        text += "tablet_pressure=off\n";
    }
    if settings.pen_input != default.pen_input {
        text += &format!("pen_input={}\n", settings.pen_input.key());
    }
    if !settings.bake_ray_query {
        text += "bake_ray_query=off\n";
    }
    settings.navigation.write(&mut text);
    write_post(&mut text, &settings.view3d_post);
    write_paint(&mut text, &settings.view3d_paint);
    crate::uv_wireframe::save_settings(&mut text, settings);
    if !settings.livelink_on_startup {
        text += "livelink_on_startup=off\n";
    }
    if !settings.livelink_keep_values {
        text += "livelink_keep_values=off\n";
    }
    if settings.external_ops {
        text += "external_ops=on\n";
    }
    if settings.external_ops_port != default.external_ops_port {
        text += &format!("external_ops_port={}\n", settings.external_ops_port);
    }
    if !settings.color_wheel {
        text += "color_wheel=off\n";
    }
    if settings.gpu_memory != default.gpu_memory {
        text += &format!("gpu_memory={}\n", settings.gpu_memory.key());
    }
    // 改行を含むパスは書かない（読めなくなる）
    if let Some(folder) = settings.library_folder.as_ref().filter(|p| p.is_absolute()) {
        let shown = folder.to_string_lossy();
        if !shown.contains('\n') {
            text += &format!("library_folder={shown}\n");
        }
    }
    if !settings.disk_cache {
        text += "disk_cache=off\n";
    }
    if let DiskLimit::Gib(n) = settings.disk_cache_limit {
        let (lo, hi) = DiskLimit::RANGE;
        text += &format!("disk_cache_limit_gib={}\n", n.clamp(lo, hi));
    }
    if let Some(folder) = settings
        .disk_cache_folder
        .as_ref()
        .filter(|p| p.is_absolute())
    {
        let shown = folder.to_string_lossy();
        if !shown.contains('\n') {
            text += &format!("disk_cache_folder={shown}\n");
        }
    }
    text
}

/// 3D の絵の仕上げ。既定のものは書かない。範囲の外の値は、書くときに範囲へ収める。
fn write_post(text: &mut String, post: &crate::view3d::display::PostFx) {
    use crate::view3d::display::{PostFx, BLOOM_STRENGTH_MAX, BLOOM_THRESHOLD_MAX, SAMPLE_CHOICES};
    let default = PostFx::default();
    if post.antialias != default.antialias && SAMPLE_CHOICES.contains(&post.antialias) {
        *text += &format!("view3d_antialias={}\n", post.antialias);
    }
    if post.bloom != default.bloom {
        *text += if post.bloom {
            "view3d_bloom=on\n"
        } else {
            "view3d_bloom=off\n"
        };
    }
    let value = |v: f32, max: f32| v.is_finite().then(|| v.clamp(0.0, max));
    if let Some(v) =
        value(post.bloom_strength, BLOOM_STRENGTH_MAX).filter(|v| *v != default.bloom_strength)
    {
        *text += &format!("view3d_bloom_strength={v}\n");
    }
    if let Some(v) =
        value(post.bloom_threshold, BLOOM_THRESHOLD_MAX).filter(|v| *v != default.bloom_threshold)
    {
        *text += &format!("view3d_bloom_threshold={v}\n");
    }
}

/// 3D の塗りの切り替え。既定のものは書かない。範囲の外の値は、書くときに範囲へ収める。
fn write_paint(text: &mut String, paint: &yolu_core::geometry::ProjectionSettings) {
    let default = yolu_core::geometry::ProjectionSettings::default();
    let paint = paint.sanitized();
    let switch = |v: bool| if v { "on" } else { "off" };
    if paint.paint_hidden != default.paint_hidden {
        *text += &format!("view3d_paint_hidden={}\n", switch(paint.paint_hidden));
    }
    if paint.paint_backfaces != default.paint_backfaces {
        *text += &format!("view3d_paint_backfaces={}\n", switch(paint.paint_backfaces));
    }
    if paint.angle_falloff != default.angle_falloff {
        *text += &format!("view3d_paint_falloff={}\n", switch(paint.angle_falloff));
    }
    if paint.angle_start != default.angle_start {
        *text += &format!("view3d_paint_falloff_start={}\n", paint.angle_start);
    }
    if paint.angle_end != default.angle_end {
        *text += &format!("view3d_paint_falloff_end={}\n", paint.angle_end);
    }
    if paint.seam_bleed != default.seam_bleed {
        *text += &format!("view3d_paint_seam_bleed={}\n", paint.seam_bleed);
    }
}

pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "settings directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let text = render(settings);
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "settings too large",
        ));
    }
    yolu_io::atomic::replace_bytes(path, text.as_bytes())
}

/// 起動のときに、設定のファイルの CPU のスレッドの数を rayon の全体のスレッドプールに入れる（最初の rayon の利用より前に 1 回。
/// 読めない設定・既に使われた後は何もしない）。自動なら rayon の既定（論理プロセッサの数）のまま。
pub fn apply_thread_setting() {
    let Some(path) = path() else { return };
    let (settings, _) = load(&path);
    if let Some(n) = settings.cpu_threads {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n as usize)
            .build_global();
    }
}

/// 起動のときに、設定のファイルの「垂直同期」を返す（ウィンドウの面を作る前に 1 回。読めない設定・項目が無いときは待たない）。
/// `apply_thread_setting` と同じく、アプリが後で読む設定と同じファイルを見る。
pub fn startup_vsync() -> bool {
    path().is_some_and(|path| load(&path).0.vsync)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/settings-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn with_lang(lang: Lang) -> Settings {
        Settings {
            lang,
            ..Settings::default()
        }
    }

    fn custom(dir: &Path) -> Settings {
        Settings {
            lang: Lang::En,
            export_padding: 8,
            export_form: crate::export::ExportForm::LilToon,
            undo_budget: Budget::Mib(512),
            source_budget: Budget::Mib(4096),
            stroke_budget: Budget::Auto,
            min_undo_steps: 12,
            cpu_threads: Some(4),
            compositing: Compositing::Cpu,
            vsync: true,
            library_folder: Some(dir.join("shelf")),
            backups: BackupKeep::Count(7),
            selection_bar: true,
            selection_all_modes: true,
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
            view3d_post: crate::view3d::display::PostFx {
                antialias: 8,
                bloom: true,
                bloom_strength: 1.25,
                bloom_threshold: 1.5,
            },
            view3d_paint: yolu_core::geometry::ProjectionSettings {
                paint_hidden: true,
                paint_backfaces: true,
                angle_falloff: false,
                angle_start: 60.0,
                angle_end: 75.0,
                seam_bleed: 4,
            },
            uv_wireframe: true,
            uv_wireframe_color: crate::uv_wireframe::DEFAULT_COLOR,
            uv_overlap: false,
            uv_overlap_color: [10, 20, 30, 40],
            livelink_on_startup: true,
            livelink_keep_values: false,
            external_ops: true,
            external_ops_port: 23456,
            color_wheel: true,
            tablet_pressure: false,
            pen_input: PenApi::WinTab,
            gpu_memory: GpuMemory::Mib(1536),
            bake_ray_query: false,
            disk_cache: false,
            disk_cache_folder: Some(dir.join("cache")),
            disk_cache_limit: DiskLimit::Gib(16),
        }
    }

    #[test]
    fn keeping_the_unity_values_defaults_on_and_survives_restart_when_disabled() {
        let dir = temp_dir("livelinkvalues");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.livelink_keep_values);
        let off = Settings {
            livelink_keep_values: false,
            ..Settings::default()
        };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("livelink_keep_values=off"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_tablet_pressure_defaults_on_and_survives_restart_when_switched_off() {
        let dir = temp_dir("tabletpressure");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.tablet_pressure, "ファイルが無ければ入");
        let off = Settings {
            tablet_pressure: false,
            ..Settings::default()
        };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("tablet_pressure=off"));
        // 入は書かない（行が無い設定ファイルと同じ）
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("tablet_pressure"));
        // この項目を知らない古い設定は入として読む。読めない値も入
        assert!(parse("language=ja\n").0.tablet_pressure);
        let (settings, problems) = parse("tablet_pressure=maybe\nlanguage=en\n");
        assert!(
            settings.tablet_pressure && problems.is_empty(),
            "{problems:?}"
        );
        // 画面の名前は日英とも「試し」と分かる
        assert!(setting_name(Lang::Ja, "tablet_pressure").contains("試し"));
        assert!(setting_name(Lang::En, "tablet_pressure").contains("experimental"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_rt_cores_for_baking_default_on_and_survive_a_restart_when_switched_off() {
        let dir = temp_dir("bakeraytracing");
        let path = dir.join("settings.conf");
        assert!(Settings::default().bake_ray_query);
        assert!(load(&path).0.bake_ray_query, "ファイルが無ければ入");
        let off = Settings {
            bake_ray_query: false,
            ..Settings::default()
        };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("bake_ray_query=off"));
        // 入は書かない（行が無い設定ファイルと同じ）
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("bake_ray_query"));
        // この項目を知らない古い設定は入として読む。読めない値も入
        assert!(parse("language=ja\nbackups=3\n").0.bake_ray_query);
        let (settings, problems) = parse("bake_ray_query=maybe\nlanguage=en\n");
        assert!(
            settings.bake_ray_query && problems.is_empty(),
            "{problems:?}"
        );
        for off in ["off", "0", "false", "no", " OFF "] {
            assert!(
                !parse(&format!("bake_ray_query={off}\n")).0.bake_ray_query,
                "{off:?}"
            );
        }
        for on in ["on", "1", "true", "yes"] {
            assert!(
                parse(&format!("bake_ray_query={on}\n")).0.bake_ray_query,
                "{on:?}"
            );
        }
        assert!(setting_name(Lang::Ja, "bake_ray_query").contains("RT コア"));
        assert!(setting_name(Lang::En, "bake_ray_query").contains("RT cores"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_export_form_defaults_to_the_channel_png_and_a_bad_value_resets_only_it() {
        use crate::export::ExportForm;
        let dir = temp_dir("exportform");
        let path = dir.join("settings.conf");
        // 項目が無ければ「PNG（今のチャンネル）」。既定は書かない
        assert_eq!(parse("language=ja\n").0.export_form, ExportForm::ChannelPng);
        save(&path, &with_lang(Lang::En)).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("export_form"));
        // 選んだ形は次の起動で戻る（テンプレートの形はテンプレートの ID）
        for form in ExportForm::ALL {
            save(
                &path,
                &Settings {
                    export_form: form,
                    ..with_lang(Lang::En)
                },
            )
            .unwrap();
            assert_eq!(load(&path).0.export_form, form, "{form:?}");
        }
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .any(|l| l == "export_form=liltoon"));
        // 読めない値は、その項目だけ既定に戻して名前つきで知らせる
        let (settings, problems) = parse("language=en\nexport_form=psd\nexport_padding=8\n");
        assert_eq!(settings.export_form, ExportForm::ChannelPng);
        assert_eq!(settings.export_padding, 8, "ほかの項目は生きる");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(matches!(
            &problems[0],
            Problem::Invalid {
                key: "export_form",
                ..
            }
        ));
        assert_eq!(setting_name(Lang::Ja, "export_form"), "出力テンプレート");
        assert_eq!(setting_name(Lang::En, "export_form"), "Output template");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_pen_input_defaults_to_windows_ink_and_keeps_wintab_across_a_restart() {
        let dir = temp_dir("peninput");
        let path = dir.join("settings.conf");
        assert_eq!(
            load(&path).0.pen_input,
            PenApi::Ink,
            "ファイルが無ければ Windows Ink"
        );
        let wintab = Settings {
            pen_input: PenApi::WinTab,
            ..Settings::default()
        };
        save(&path, &wintab).unwrap();
        assert_eq!(load(&path), (wintab, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .any(|l| l == "pen_input=wintab"));
        // Windows Ink は書かない（行が無い設定ファイルと同じ）
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("pen_input"));
        // この項目を知らない古い設定は Windows Ink として読む。書き方は 2 つだけ
        assert_eq!(parse("language=ja\n").0.pen_input, PenApi::Ink);
        assert_eq!(parse("pen_input=ink\n").0.pen_input, PenApi::Ink);
        assert_eq!(parse("pen_input=wintab\n").0.pen_input, PenApi::WinTab);
        // 知らない値は既定へ戻して、正しくない項目として知らせる（大文字は別の値）
        for bad in ["pen_input=tablet", "pen_input=WinTab", "pen_input="] {
            let (settings, problems) = parse(&format!("{bad}\nlanguage=en\n"));
            assert_eq!(settings.pen_input, PenApi::Ink, "{bad}");
            assert_eq!(settings.lang, Lang::En, "ほかの行は読む: {bad}");
            assert!(
                problems
                    .iter()
                    .any(|p| format!("{p:?}").contains("pen_input")),
                "{bad}: {problems:?}"
            );
        }
        // 画面の名前は日英で出る
        assert_eq!(setting_name(Lang::Ja, "pen_input"), "ペンの入力");
        assert_eq!(setting_name(Lang::En, "pen_input"), "Pen input");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn accepting_external_commands_defaults_off_and_survives_restart_only_when_on() {
        let dir = temp_dir("externalops");
        let path = dir.join("settings.conf");
        assert!(!load(&path).0.external_ops, "ファイルが無ければ切");
        let on = Settings {
            external_ops: true,
            ..Settings::default()
        };
        save(&path, &on).unwrap();
        assert_eq!(load(&path), (on, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("external_ops=on"));
        // 切は書かない（行が無い設定ファイルと同じ）
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("external_ops"));
        // この項目を知らない古い設定は切として読む。読めない値も切
        assert!(
            !parse("language=ja\nlivelink_keep_values=off\n")
                .0
                .external_ops
        );
        let (settings, problems) = parse("external_ops=maybe\nlanguage=en\n");
        assert!(
            !settings.external_ops && problems.is_empty(),
            "{problems:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn vsync_defaults_off_and_survives_restart_only_when_on() {
        let dir = temp_dir("vsync");
        let path = dir.join("settings.conf");
        assert!(!Settings::default().vsync, "既定は垂直同期を待たない");
        assert!(!load(&path).0.vsync, "ファイルが無ければ待たない");
        let on = Settings {
            vsync: true,
            ..Settings::default()
        };
        save(&path, &on).unwrap();
        assert_eq!(load(&path), (on, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .any(|l| l == "vsync=on"));
        // 待たないは書かない（行が無い設定ファイルと同じ中身）
        save(&path, &with_lang(Lang::En)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        // この項目を知らない前の版の設定は、待たないとして読む。ほかの項目は前のまま生きる
        let (old, problems) = parse("language=ja\ncpu_threads=4\nexternal_ops=on\n");
        assert!(!old.vsync && problems.is_empty(), "{problems:?}");
        assert_eq!((old.cpu_threads, old.external_ops), (Some(4), true));
        // 読めない値・大文字・空は、待たないまま（知らせる問題にはしない）
        for bad in ["maybe", "ON", "", "1", "true", "off"] {
            let (read, problems) = parse(&format!("language=ja\nvsync={bad}\n"));
            assert!(!read.vsync, "vsync={bad:?}");
            assert!(problems.is_empty(), "vsync={bad:?}: {problems:?}");
        }
        // 画面の名前は日英で出る
        assert_eq!(setting_name(Lang::Ja, "vsync"), "垂直同期");
        assert_eq!(setting_name(Lang::En, "vsync"), "VSync");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_external_commands_port_defaults_to_the_standard_one_and_is_kept_only_when_changed() {
        assert_eq!(
            Settings::default().external_ops_port,
            yolu_mcp::DEFAULT_PORT
        );
        let (settings, problems) = parse("external_ops_port=23456\n");
        assert_eq!((settings.external_ops_port, problems), (23456, vec![]));
        assert!(!render(&Settings::default()).contains("external_ops_port"));
        // 1024 より下・番号でない値は既定へ戻し、その項目を知らせる
        for bad in ["80", "0", "70000", "port"] {
            let (settings, problems) = parse(&format!("external_ops_port={bad}\n"));
            assert_eq!(settings.external_ops_port, yolu_mcp::DEFAULT_PORT, "{bad}");
            assert_eq!(problems.len(), 1, "{bad}: {problems:?}");
        }
    }

    #[test]
    fn live_link_startup_defaults_on_and_survives_restart_when_disabled() {
        let dir = temp_dir("livelink");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.livelink_on_startup);
        let off = Settings {
            livelink_on_startup: false,
            ..Settings::default()
        };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("livelink_on_startup=off"));
        save(&path, &Settings::default()).unwrap();
        assert_eq!(load(&path), (Settings::default(), vec![]));
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("livelink_on_startup"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_color_wheel_defaults_on_and_survives_restart_when_switched_off() {
        let dir = temp_dir("colorwheel");
        let path = dir.join("settings.conf");
        assert!(load(&path).0.color_wheel);
        let off = Settings {
            color_wheel: false,
            ..Settings::default()
        };
        save(&path, &off).unwrap();
        assert_eq!(load(&path), (off, vec![]));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("color_wheel=off"));
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("color_wheel"));
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
        assert_eq!(
            config_path("linux", env).unwrap(),
            home.join(".config/YoluPainter/settings.conf")
        );
        assert_eq!(
            config_path("macos", env).unwrap(),
            home.join("Library/Application Support/YoluPainter/settings.conf")
        );
        assert_eq!(
            config_path("windows", env).unwrap(),
            base.join("roaming/YoluPainter/settings.conf")
        );
        assert!(config_path("linux", |_| None).is_none());
        assert_eq!(
            config_path("linux", |_| Some(base.join("config"))).unwrap(),
            base.join("config/YoluPainter/settings.conf")
        );
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
        // 置き換える前に失敗しても、前のファイルのまま（一時ファイルも残らない）
        assert!(yolu_io::atomic::failing(|| save(&path, &with_lang(Lang::Ja))).is_err());
        assert_eq!(load(&path).0.lang, Lang::En);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::write(&path, "language=unknown").unwrap();
        assert_eq!(
            load(&path),
            (
                Settings::default(),
                vec![Problem::Language("unknown".into())]
            )
        );
        std::fs::write(&path, vec![b'a'; 4097]).unwrap();
        assert_eq!(
            load(&path),
            (Settings::default(), vec![Problem::Unreadable])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 起動で設定を読むとき、言語が読めなかった（ファイルが無い・読めない・行が無い・値が正しくない）ときだけ OS の言語になる。
    /// 言語が書いてあれば OS の言語によらずそれ。ほかの項目は `load` と同じ。
    #[test]
    fn startup_uses_the_system_language_only_when_the_file_has_none() {
        let dir = temp_dir("startup-language");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.conf");
        let startup = |text: Option<&str>, system| {
            match text {
                Some(text) => std::fs::write(&path, text).unwrap(),
                None => drop(std::fs::remove_file(&path)),
            }
            load_for_startup(&path, system)
        };
        for system in Lang::ALL {
            // 初めての起動（ファイルが無い）
            assert_eq!(startup(None, system), (with_lang(system), vec![]));
            // ファイルはあるが言語の行が無い（ほかの項目は読む）
            let (settings, problems) = startup(Some("export_padding=8\n"), system);
            assert_eq!((settings.lang, settings.export_padding), (system, 8));
            assert_eq!(problems, vec![]);
            // 言語が書いてあれば、OS の言語によらずそれ
            for saved in Lang::ALL {
                let text = format!("language={}\nexport_padding=8\n", saved.pick("ja", "en"));
                let (settings, problems) = startup(Some(&text), system);
                assert_eq!((settings.lang, settings.export_padding), (saved, 8));
                assert_eq!(problems, vec![]);
            }
            // 言語の値が正しくない: 知らせは残し、言語は OS の言語
            assert_eq!(
                startup(Some("language=unknown\n"), system),
                (with_lang(system), vec![Problem::Language("unknown".into())])
            );
            assert_eq!(
                startup(Some("language=\n"), system),
                (with_lang(system), vec![Problem::Language(String::new())])
            );
            // 読めないファイル（`キー=値` でない行・大きすぎる）: 設定は全部既定で、言語は OS の言語
            assert_eq!(
                startup(Some("garbage\n"), system),
                (with_lang(system), vec![Problem::Unreadable])
            );
            // 言語の行より後ろに壊れた行があっても、読めなかったファイルの言語は使わない
            assert_eq!(
                startup(Some("language=ja\ngarbage\n"), system),
                (with_lang(system), vec![Problem::Unreadable])
            );
            std::fs::write(&path, vec![b'a'; 4097]).unwrap();
            assert_eq!(
                load_for_startup(&path, system),
                (with_lang(system), vec![Problem::Unreadable])
            );
        }
        // 読み込みの口（`load`）は今までどおり、言語が無ければ既定の日本語
        assert_eq!(load(&dir.join("none.conf")).0.lang, Lang::Ja);
        std::fs::write(&path, "export_padding=8\n").unwrap();
        assert_eq!(load(&path).0.lang, Lang::Ja);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 言語が無いまま始めた設定は、設定を変えて書くまでファイルを作らず、書いた後は書いた言語が先になる。
    #[test]
    fn the_language_a_first_start_chose_is_kept_once_written() {
        let dir = temp_dir("startup-written");
        let path = dir.join("settings.conf");
        let (first, _) = load_for_startup(&path, Lang::En);
        assert_eq!(first.lang, Lang::En);
        assert!(!path.exists(), "読むだけではファイルを作らない");
        save(&path, &first).unwrap();
        for system in Lang::ALL {
            assert_eq!(load_for_startup(&path, system).0.lang, Lang::En);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_3d_finish_defaults_to_four_samples_and_no_bloom_and_is_written_only_when_changed() {
        use crate::view3d::display::PostFx;
        let dir = temp_dir("finish");
        let path = dir.join("settings.conf");
        let default = PostFx::default();
        assert_eq!((default.antialias, default.bloom), (4, false));
        assert_eq!(load(&path).0.view3d_post, default);
        // 既定は書かない
        save(&path, &with_lang(Lang::Ja)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
        // 1 つずつ変えると、その行だけが書かれて読み直せる
        let cases = [
            (
                PostFx {
                    antialias: 1,
                    ..default
                },
                "view3d_antialias=1",
            ),
            (
                PostFx {
                    antialias: 2,
                    ..default
                },
                "view3d_antialias=2",
            ),
            (
                PostFx {
                    antialias: 8,
                    ..default
                },
                "view3d_antialias=8",
            ),
            (
                PostFx {
                    bloom: true,
                    ..default
                },
                "view3d_bloom=on",
            ),
            (
                PostFx {
                    bloom_strength: 0.0,
                    ..default
                },
                "view3d_bloom_strength=0",
            ),
            (
                PostFx {
                    bloom_strength: 2.0,
                    ..default
                },
                "view3d_bloom_strength=2",
            ),
            (
                PostFx {
                    bloom_threshold: 0.0,
                    ..default
                },
                "view3d_bloom_threshold=0",
            ),
            (
                PostFx {
                    bloom_threshold: 3.5,
                    ..default
                },
                "view3d_bloom_threshold=3.5",
            ),
        ];
        for (post, line) in cases {
            let settings = Settings {
                view3d_post: post,
                ..with_lang(Lang::Ja)
            };
            save(&path, &settings).unwrap();
            let written = std::fs::read_to_string(&path).unwrap();
            assert_eq!(written, format!("language=ja\n{line}\n"), "{post:?}");
            assert_eq!(load(&path), (settings, vec![]), "{post:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_settings_file_from_before_the_3d_finish_reads_unchanged() {
        use crate::view3d::display::PostFx;
        // 仕上げの項目が無い、前の版が書いたファイル（ほかの項目は全部そのまま読める。仕上げは既定）
        let old = "language=en\nexport_padding=8\nundo_budget_mib=512\ncompositing=cpu\nview3d_orbit=model\nview3d_zoom=pointer\nuv_wireframe=off\n";
        let (settings, problems) = parse(old);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(settings.lang, Lang::En);
        assert_eq!(settings.export_padding, 8);
        assert_eq!(settings.undo_budget, Budget::Mib(512));
        assert_eq!(settings.compositing, Compositing::Cpu);
        assert!(!settings.uv_wireframe);
        assert_eq!(settings.view3d_post, PostFx::default());
        // 読んで書き直しても、前の項目の値は同じ（仕上げは既定のままなので行は増えない）
        let written = render(&settings);
        assert!(
            !written.contains("view3d_antialias") && !written.contains("view3d_bloom"),
            "{written}"
        );
        assert_eq!(parse(&written), (settings, vec![]));
    }

    #[test]
    fn a_bad_3d_finish_value_resets_only_that_item_and_names_it() {
        use crate::view3d::display::PostFx;
        let text = "language=en\nview3d_antialias=3\nview3d_bloom=maybe\nview3d_bloom_strength=9\nview3d_bloom_threshold=nan\nexport_padding=8\n";
        let (settings, problems) = parse(text);
        assert_eq!(settings.view3d_post, PostFx::default());
        assert_eq!(settings.export_padding, 8, "ほかの項目は生きる");
        let keys: Vec<&str> = problems
            .iter()
            .filter_map(|p| match p {
                Problem::Invalid { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            [
                "view3d_antialias",
                "view3d_bloom",
                "view3d_bloom_strength",
                "view3d_bloom_threshold"
            ]
        );
        for lang in Lang::ALL {
            for key in keys.iter().copied() {
                let name = setting_name(lang, key);
                assert_ne!(
                    name,
                    setting_name(lang, "unknown_key"),
                    "{key} に名前がある"
                );
            }
        }
        // 範囲の外の値は書かない（読めなくなる）。範囲へ収めて書く
        let wild = Settings {
            view3d_post: PostFx {
                bloom_strength: 99.0,
                bloom_threshold: f32::NAN,
                ..PostFx::default()
            },
            ..Settings::default()
        };
        let written = render(&wild);
        assert!(
            written.contains("view3d_bloom_strength=2\n")
                && !written.contains("view3d_bloom_threshold"),
            "{written}"
        );
    }

    #[test]
    fn the_3d_paint_switches_survive_a_restart_and_are_written_only_when_changed() {
        use yolu_core::geometry::ProjectionSettings;
        let dir = temp_dir("paint3d");
        let path = dir.join("settings.conf");
        let default = ProjectionSettings::default();
        assert_eq!(load(&path).0.view3d_paint, default);
        save(&path, &with_lang(Lang::Ja)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
        // 1 つずつ変えると、その行だけが書かれて読み直せる（範囲の端も）
        let cases = [
            (
                ProjectionSettings {
                    paint_hidden: true,
                    ..default
                },
                "view3d_paint_hidden=on",
            ),
            (
                ProjectionSettings {
                    paint_backfaces: true,
                    ..default
                },
                "view3d_paint_backfaces=on",
            ),
            (
                ProjectionSettings {
                    angle_falloff: false,
                    ..default
                },
                "view3d_paint_falloff=off",
            ),
            (
                ProjectionSettings {
                    angle_start: 0.0,
                    ..default
                },
                "view3d_paint_falloff_start=0",
            ),
            (
                ProjectionSettings {
                    angle_end: 90.0,
                    ..default
                },
                "view3d_paint_falloff_end=90",
            ),
            (
                ProjectionSettings {
                    seam_bleed: 0,
                    ..default
                },
                "view3d_paint_seam_bleed=0",
            ),
            (
                ProjectionSettings {
                    seam_bleed: yolu_core::geometry::MAX_SEAM_BLEED,
                    ..default
                },
                "view3d_paint_seam_bleed=16",
            ),
        ];
        for (paint, line) in cases {
            let settings = Settings {
                view3d_paint: paint,
                ..with_lang(Lang::Ja)
            };
            save(&path, &settings).unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                format!("language=ja\n{line}\n"),
                "{paint:?}"
            );
            assert_eq!(load(&path), (settings, vec![]), "{paint:?}");
        }
        // 画面の切り替えは AppState の設定に出て、読んだ設定は AppState へ入る（起動し直しても同じ）
        let mut state = crate::state::AppState::new(8, 8);
        assert_eq!(state.settings().view3d_paint, default);
        let changed = ProjectionSettings {
            paint_hidden: true,
            angle_start: 70.0,
            angle_end: 88.0,
            seam_bleed: 5,
            ..default
        };
        state.view3d.projection = changed;
        save(&path, &state.settings()).unwrap();
        let mut again = crate::state::AppState::new(8, 8);
        again.load_settings(load(&path).0);
        assert_eq!(again.view3d.projection, changed);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_settings_file_from_before_the_3d_paint_switches_reads_them_as_defaults() {
        let old = "language=en\nexport_padding=8\nview3d_antialias=8\nuv_wireframe=off\n";
        let (settings, problems) = parse(old);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            settings.view3d_paint,
            yolu_core::geometry::ProjectionSettings::default()
        );
        assert_eq!(
            (settings.export_padding, settings.view3d_post.antialias),
            (8, 8)
        );
        assert!(!render(&settings).contains("view3d_paint"));
    }

    #[test]
    fn a_bad_3d_paint_value_resets_only_that_item_and_names_it() {
        use yolu_core::geometry::ProjectionSettings;
        let text = "language=en\nview3d_paint_hidden=yes\nview3d_paint_backfaces=on\nview3d_paint_falloff=1\nview3d_paint_falloff_start=91\nview3d_paint_falloff_end=nan\nview3d_paint_seam_bleed=17\nexport_padding=8\n";
        let (settings, problems) = parse(text);
        assert_eq!(
            settings.view3d_paint,
            ProjectionSettings {
                paint_backfaces: true,
                ..ProjectionSettings::default()
            },
            "正しい項目は生かす"
        );
        assert_eq!(settings.export_padding, 8);
        let keys: Vec<&str> = problems
            .iter()
            .filter_map(|p| match p {
                Problem::Invalid { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            [
                "view3d_paint_hidden",
                "view3d_paint_falloff",
                "view3d_paint_falloff_start",
                "view3d_paint_falloff_end",
                "view3d_paint_seam_bleed"
            ]
        );
        for lang in Lang::ALL {
            for key in keys.iter().copied() {
                assert_ne!(
                    setting_name(lang, key),
                    setting_name(lang, "unknown_key"),
                    "{key} に名前がある"
                );
            }
        }
        // 塗らない角度が弱め始めより小さいファイルは、弱め始めに揃えて読む
        let (settings, problems) =
            parse("view3d_paint_falloff_start=70\nview3d_paint_falloff_end=50\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            (
                settings.view3d_paint.angle_start,
                settings.view3d_paint.angle_end
            ),
            (70.0, 70.0)
        );
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
            "export_form=liltoon",
            "undo_budget_mib=512",
            "source_budget_mib=4096",
            "min_undo_steps=12",
            "cpu_threads=4",
            "compositing=cpu",
            "vsync=on",
            "backups=7",
            "gpu_memory=1536",
            "pressure_low=0.125",
            "pressure_high=0.875",
            "pressure_curve=0:0,0.4:0.6,1:1",
            "view3d_antialias=8",
            "view3d_bloom=on",
            "view3d_bloom_strength=1.25",
            "view3d_bloom_threshold=1.5",
            "view3d_paint_hidden=on",
            "view3d_paint_backfaces=on",
            "view3d_paint_falloff=off",
            "view3d_paint_falloff_start=60",
            "view3d_paint_falloff_end=75",
            "view3d_paint_seam_bleed=4",
            "uv_overlap=off",
            "uv_overlap_color=10,20,30,40",
            "external_ops=on",
            "tablet_pressure=off",
            "pen_input=wintab",
            "bake_ray_query=off",
            "disk_cache=off",
            "disk_cache_limit_gib=16",
            "external_ops_port=23456",
        ] {
            assert!(written.lines().any(|l| l == line), "{line}\n{written}");
        }
        assert!(!written.contains("stroke_budget_mib"), "自動は書かない");
        assert!(written.contains("library_folder="), "{written}");
        // 選び直して既定に戻すと、行が消える
        let mut back = all.clone();
        back.export_padding = DEFAULT_EXPORT_PADDING;
        back.export_form = crate::export::ExportForm::default();
        back.undo_budget = Budget::Auto;
        back.source_budget = Budget::Auto;
        back.min_undo_steps = DEFAULT_MIN_UNDO_STEPS;
        back.cpu_threads = None;
        back.compositing = Compositing::Auto;
        back.vsync = false;
        back.library_folder = None;
        back.backups = BackupKeep::All;
        back.gpu_memory = GpuMemory::Auto;
        back.pressure = PressureAdjust::default();
        back.livelink_keep_values = true;
        back.tablet_pressure = true;
        back.pen_input = PenApi::Ink;
        back.bake_ray_query = true;
        back.external_ops = false;
        back.external_ops_port = yolu_mcp::DEFAULT_PORT;
        back.view3d_post = crate::view3d::display::PostFx::default();
        back.view3d_paint = yolu_core::geometry::ProjectionSettings::default();
        back.disk_cache = true;
        back.disk_cache_folder = None;
        back.disk_cache_limit = DiskLimit::Auto;
        back.uv_overlap = true;
        back.uv_overlap_color = crate::uv_wireframe::DEFAULT_OVERLAP_COLOR;
        back.selection_all_modes = false;
        save(&path, &back).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        // 範囲の端の値
        let mut edge = Settings {
            export_padding: 0,
            undo_budget: Budget::Mib(0),
            source_budget: Budget::Mib(65536),
            stroke_budget: Budget::Mib(8),
            cpu_threads: Some(MAX_CPU_THREADS),
            min_undo_steps: 0,
            ..Settings::default()
        };
        save(&path, &edge).unwrap();
        assert_eq!(load(&path), (edge.clone(), vec![]));
        // 無限に広げる（-1）は既定なので書かず、fill と書いても読める
        edge.export_padding = -1;
        assert!(!render(&edge).contains("export_padding"));
        assert_eq!(
            parse("export_padding=fill\n"),
            (Settings::default(), vec![])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_values_fall_back_to_the_default_one_by_one_with_a_reason() {
        let (read, problems) = parse("language=en\nundo_budget_mib=lots\ncompositing=cpu\n");
        assert_eq!(read.lang, Lang::En);
        assert_eq!(read.compositing, Compositing::Cpu, "正しい項目は生かす");
        assert_eq!(read.undo_budget, Budget::Auto);
        assert_eq!(
            problems,
            [Problem::Invalid {
                key: "undo_budget_mib",
                value: "lots".into()
            }]
        );
        // 範囲の外・負・小数・空・大文字・16 進・桁あふれ
        let cases: [(&'static str, &[&str]); 10] = [
            (
                "undo_budget_mib",
                &[
                    "-1",
                    "16385",
                    "1.5",
                    "",
                    "AUTO",
                    "0x10",
                    "99999999999999999999",
                ],
            ),
            ("source_budget_mib", &["15", "65537", "0"]),
            ("stroke_budget_mib", &["7", "8193"]),
            ("min_undo_steps", &["101", "-1", "auto", ""]),
            ("cpu_threads", &["0", "1025", "-2", "many"]),
            ("export_padding", &["1", "3", "65", "-1", "-5", "Fill", ""]),
            ("compositing", &["both", "GPU", ""]),
            ("pressure_low", &["-0.1", "0.95", "x", "", "NaN"]),
            ("pressure_high", &["0.05", "1.5", "x", ""]),
            (
                "pressure_curve",
                &[
                    "0:0",
                    "0:0,1:2",
                    "0:0,0.5:0.5",
                    "0:0;1:1",
                    "a:b",
                    "",
                    "0:0,0.001:0.5,1:1",
                ],
            ),
        ];
        for (key, values) in cases {
            for bad in values {
                let (read, problems) = parse(&format!("language=ja\n{key}={bad}\n"));
                assert_eq!(read, Settings::default(), "{key}={bad:?}");
                assert_eq!(
                    problems,
                    [Problem::Invalid {
                        key,
                        value: (*bad).into()
                    }],
                    "{key}={bad:?}"
                );
            }
        }
        // 棚の場所: 相対パスは断る（どこからの相対か決まらない）、空は既定
        for bad in ["shelf", "./shelf", "../shelf"] {
            let (read, problems) = parse(&format!("library_folder={bad}\n"));
            assert_eq!(read.library_folder, None);
            assert_eq!(
                problems,
                [Problem::Invalid {
                    key: "library_folder",
                    value: bad.into()
                }]
            );
        }
        assert_eq!(parse("library_folder=\n"), (Settings::default(), vec![]));
        let absolute = std::env::current_dir().unwrap().join("shelf");
        assert_eq!(
            parse(&format!("library_folder={}\n", absolute.display()))
                .0
                .library_folder,
            Some(absolute)
        );
        // 複数の壊れた値は、全部の理由。書いていない項目は既定で理由を出さない
        let (read, problems) = parse("language=fr\ncpu_threads=0\nmin_undo_steps=-3\n");
        assert_eq!(read, Settings::default());
        assert_eq!(
            problems,
            [
                Problem::Language("fr".into()),
                Problem::Invalid {
                    key: "cpu_threads",
                    value: "0".into()
                },
                Problem::Invalid {
                    key: "min_undo_steps",
                    value: "-3".into()
                },
            ]
        );
        // 空白・空行・知らないキー（読み飛ばす。新しい版が足した項目で壊れない）・後ろの行が勝つ
        let (read, problems) =
            parse("\n  language = en \n future=1\n cpu_threads = 2 \ncpu_threads=3\n");
        assert_eq!((read.lang, read.cpu_threads), (Lang::En, Some(3)));
        assert!(problems.is_empty());
        // `キー=値` ではない行は、読めないファイル（全部既定）
        assert_eq!(
            parse("language=en\njunk\n"),
            (Settings::default(), vec![Problem::Unreadable])
        );
        // 筆圧の下限と上限は組: 1 つずつは範囲内でも、近すぎれば調整は全部既定に戻して理由を出す。ほかの項目は生かす
        let (read, problems) = parse(
            "cpu_threads=2\npressure_low=0.5\npressure_high=0.55\npressure_curve=0:0,0.5:0.8,1:1\n",
        );
        assert_eq!(read.pressure, PressureAdjust::default());
        assert_eq!(read.cpu_threads, Some(2));
        assert_eq!(
            problems,
            [Problem::Invalid {
                key: "pressure_high",
                value: "0.55".into()
            }]
        );
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
        let pressure = PressureAdjust::new(0.1, 0.9, vec![])
            .unwrap()
            .with_curve_shape(c)
            .unwrap();
        let settings = Settings {
            pressure,
            ..Settings::default()
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), (settings.clone(), vec![]));
        // 直線へ戻すと、曲線の行は消える
        let straight = Settings {
            pressure: settings
                .pressure
                .with_curve_shape(Curve::identity())
                .unwrap(),
            ..Settings::default()
        };
        save(&path, &straight).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("pressure_curve"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backups_to_keep_is_written_only_when_it_is_not_the_default_and_restored() {
        let dir = temp_dir("backups");
        let path = dir.join("settings.conf");
        let with = |lang, backups| Settings {
            lang,
            backups,
            ..Settings::default()
        };
        // 既定（すべて残す）は書かない。今までのファイルと同じ中身
        save(&path, &with(Lang::En, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        for keep in [
            BackupKeep::Count(0),
            BackupKeep::Count(1),
            BackupKeep::Count(37),
            BackupKeep::Count(MAX_BACKUPS_TO_KEEP),
        ] {
            save(&path, &with(Lang::Ja, keep)).unwrap();
            assert_eq!(load(&path), (with(Lang::Ja, keep), vec![]), "{keep:?}");
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=ja\nbackups=1000\n"
        );
        // 選び直して「すべて」に戻すと、行は消える
        save(&path, &with(Lang::Ja, BackupKeep::All)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
        // 上限を超えて渡されても、書くのは上限
        save(&path, &with(Lang::Ja, BackupKeep::Count(5000))).unwrap();
        assert_eq!(
            load(&path).0.backups,
            BackupKeep::Count(MAX_BACKUPS_TO_KEEP)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_selection_bar_is_written_only_when_it_is_off_and_restored() {
        let dir = temp_dir("selection-bar");
        let path = dir.join("settings.conf");
        save(&path, &with_lang(Lang::Ja)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=ja\n",
            "出す（既定）は書かない"
        );
        let off = Settings {
            selection_bar: false,
            backups: BackupKeep::Count(3),
            ..with_lang(Lang::En)
        };
        save(&path, &off).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=en\nbackups=3\nselection_bar=off\n"
        );
        assert_eq!(load(&path), (off, vec![]));
        // 知らない値は既定（出す）。理由は出さない
        assert_eq!(
            parse("selection_bar=maybe\n"),
            (Settings::default(), vec![])
        );
        assert_eq!(parse("selection_bar=on\n"), (Settings::default(), vec![]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_selection_modes_are_written_only_when_all_four_are_shown_and_restored() {
        let dir = temp_dir("selection-modes");
        let path = dir.join("settings.conf");
        assert!(!Settings::default().selection_all_modes, "既定は畳む");
        save(&path, &with_lang(Lang::Ja)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=ja\n",
            "畳む（既定）は書かない"
        );
        let all = Settings {
            selection_all_modes: true,
            ..with_lang(Lang::En)
        };
        save(&path, &all).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=en\nselection_all_modes=on\n"
        );
        assert_eq!(load(&path), (all, vec![]));
        // 読めない値は既定（畳む）。理由は出さない
        assert_eq!(
            parse("selection_all_modes=maybe\n"),
            (Settings::default(), vec![])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_backups_value_falls_back_to_keeping_all_with_a_reason_and_the_rest_is_kept() {
        let (read, problems) = parse("language=en\nbackups=many\ncompositing=cpu\n");
        assert_eq!(
            (read.lang, read.backups, read.compositing),
            (Lang::En, BackupKeep::All, Compositing::Cpu)
        );
        assert_eq!(problems, [Problem::Backups("many".into())]);
        // 範囲の外・負・小数・空・大文字・16 進・桁あふれ・上限の次
        for bad in [
            "-1",
            "1001",
            "5000",
            "2.5",
            "",
            "ALL",
            "0x10",
            "99999999999999999999",
        ] {
            let (read, problems) = parse(&format!("backups={bad}\n"));
            assert_eq!(read.backups, BackupKeep::All, "{bad:?}");
            assert_eq!(problems, [Problem::Backups(bad.into())], "{bad:?}");
        }
        assert_eq!(parse("backups=7\n").0.backups, BackupKeep::Count(7));
        assert_eq!(parse("backups=all\n"), (Settings::default(), vec![]));
        assert_eq!(
            parse("backups = 12 \nbackups=13\n").0.backups,
            BackupKeep::Count(13),
            "後ろの行が勝つ"
        );
    }

    #[test]
    fn problems_have_short_reasons_in_both_languages_without_the_other_one() {
        let problems = [
            Problem::Unreadable,
            Problem::Language("x".into()),
            Problem::Invalid {
                key: "undo_budget_mib",
                value: "5000".into(),
            },
            Problem::Invalid {
                key: "library_folder",
                value: "rel".into(),
            },
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
        assert_eq!(
            problems[4].text(Lang::Ja),
            "退避を残す数の設定が正しくありません（5000）。すべて残します。"
        );
        assert_eq!(
            problems[4].text(Lang::En),
            "Invalid Backups to Keep setting (5000); keeping all."
        );
        assert_eq!(
            problems[2].text(Lang::Ja),
            "取り消し履歴の設定が正しくありません（5000）。既定に戻します。"
        );
        assert_eq!(
            problems[2].text(Lang::En),
            "Invalid Undo history setting (5000); using the default."
        );
        // 長い値は切って、帯を溢れさせない
        let long = Problem::Invalid {
            key: "cpu_threads",
            value: "9".repeat(500),
        }
        .text(Lang::En);
        assert!(long.len() < 100, "{long}");
        let long = Problem::Backups("9".repeat(500)).text(Lang::En);
        assert!(long.len() < 100, "{long}");
        // どのキーにも名前がある
        for key in [
            "language",
            "export_padding",
            "min_undo_steps",
            "cpu_threads",
            "compositing",
            "library_folder",
            "backups",
        ]
        .into_iter()
        .chain(BudgetKind::ALL.iter().map(|k| k.key()))
        {
            for lang in Lang::ALL {
                assert_ne!(
                    setting_name(lang, key),
                    setting_name(lang, "unknown"),
                    "{key}"
                );
            }
        }
    }

    #[test]
    fn automatic_budgets_follow_the_memory_in_the_share_each_one_needs() {
        // (物理メモリ, 取り消し, レイヤーのメモリ, 1 回の操作) の MiB: 割合は 1/8・1/2・1/16。下限は小さい機械でも作業できる量、上限は大きい機械の値
        for (ram, undo, source, stroke) in [
            (512, 256, 256, 64),
            (1024, 256, 512, 64),
            (8192, 1024, 4096, 512),
            (16384, 2048, 8192, 1024),
            (32768, 4096, 16384, 2048),
            (65536, 8192, 32768, 4096),
            (131072, 16384, 65536, 4096),
            (1 << 20, 16384, 65536, 4096),
        ] {
            assert_eq!(
                BudgetKind::Undo.automatic_mib(ram),
                undo,
                "取り消し @ {ram}"
            );
            assert_eq!(
                BudgetKind::Source.automatic_mib(ram),
                source,
                "レイヤーのメモリ @ {ram}"
            );
            assert_eq!(
                BudgetKind::Stroke.automatic_mib(ram),
                stroke,
                "1 回の操作 @ {ram}"
            );
        }
        // 設定からバイトへ: 自動はメモリから、数はそのまま
        let s = Settings {
            source_budget: Budget::Mib(100),
            min_undo_steps: 7,
            ..Settings::default()
        };
        assert_eq!(
            s.budgets(16384),
            Budgets {
                undo: 2048 << 20,
                source: 100 << 20,
                stroke: 1024 << 20,
                min_undo_steps: 7
            }
        );
        // 自動の最大は範囲の中で、選択肢にもある（自動で出る値を、そのまま選び直せる）
        for kind in BudgetKind::ALL {
            let (lo, hi) = kind.range();
            let top = kind.automatic_mib(1 << 24);
            assert!(
                (lo..=hi).contains(&top),
                "{kind:?}: 自動の最大 {top} は範囲 {lo}..={hi} の中"
            );
            assert!(kind.choices().contains(&top), "{kind:?}: 選択肢に {top}");
        }
        assert_eq!(BudgetKind::Source.range().1, 65536);
        assert!(BudgetKind::Source
            .choices()
            .ends_with(&[16384, 32768, 65536]));
        assert!(BudgetKind::Undo.choices().ends_with(&[8192, 16384]));
        assert!(BudgetKind::Stroke.choices().ends_with(&[2048, 4096]));
        // 選択肢は範囲の中で、昇順
        for kind in BudgetKind::ALL {
            let (lo, hi) = kind.range();
            let choices = kind.choices();
            assert!(choices.windows(2).all(|w| w[0] < w[1]), "{kind:?}");
            assert!(choices.iter().all(|c| (lo..=hi).contains(c)), "{kind:?}");
        }
    }

    #[test]
    fn budget_values_written_before_the_raise_still_read_and_the_new_top_is_accepted() {
        // 古い設定ファイルの MiB の指定は、そのまま読める（自動へ直さない）
        let (read, problems) =
            parse("source_budget_mib=8192\nundo_budget_mib=2048\nstroke_budget_mib=512\n");
        assert_eq!(problems, vec![]);
        assert_eq!(
            (read.source_budget, read.undo_budget, read.stroke_budget),
            (Budget::Mib(8192), Budget::Mib(2048), Budget::Mib(512))
        );
        // 新しい上限まで書けて、1 つ上は断る
        let (read, problems) =
            parse("source_budget_mib=65536\nundo_budget_mib=16384\nstroke_budget_mib=4096\n");
        assert_eq!(problems, vec![]);
        assert_eq!(read.source_budget, Budget::Mib(65536));
        let (read, problems) = parse("source_budget_mib=65537\n");
        assert_eq!(read.source_budget, Budget::Auto);
        assert_eq!(
            problems,
            [Problem::Invalid {
                key: "source_budget_mib",
                value: "65537".into()
            }]
        );
        // 大きい予算も、設定から文書へ渡すバイトで桁あふれしない
        let big = Settings {
            source_budget: Budget::Mib(65536),
            ..Settings::default()
        };
        assert_eq!(big.budgets(1 << 20).source, 65536u64 << 20);
        assert_eq!(big.load_source_bytes(1 << 20), 65536u64 << 20);
    }

    #[test]
    fn the_memory_is_read_from_meminfo_and_has_a_floor() {
        assert_eq!(
            parse_meminfo("MemTotal:       16384000 kB\nMemFree: 1 kB\n"),
            Some(16000)
        );
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: many kB\n"), None);
        assert!(system_memory_mib() >= 1024);
    }

    #[test]
    fn the_library_folder_falls_back_to_the_default_next_to_the_settings() {
        let mut s = Settings::default();
        assert_eq!(s.library_folder(), default_library_folder());
        if let Some(default) = default_library_folder() {
            assert!(
                default.ends_with("YoluPainter/Library")
                    || default.ends_with("YoluPainter\\Library"),
                "{default:?}"
            );
        }
        let chosen = std::env::current_dir().unwrap().join("mine");
        s.library_folder = Some(chosen.clone());
        assert_eq!(s.library_folder(), Some(chosen));
    }

    #[test]
    fn the_gpu_memory_survives_a_restart_and_the_default_is_not_written() {
        let dir = temp_dir("gpu-memory");
        let path = dir.join("settings.conf");
        assert_eq!(
            load(&path).0.gpu_memory,
            GpuMemory::Auto,
            "ファイルが無ければ自動"
        );
        for choice in [
            GpuMemory::Low,
            GpuMemory::Standard,
            GpuMemory::High,
            GpuMemory::Mib(256),
            GpuMemory::Mib(1536),
            GpuMemory::Mib(crate::gpu_memory::MAX_TOTAL_MIB),
        ] {
            let settings = Settings {
                gpu_memory: choice,
                ..Settings::default()
            };
            save(&path, &settings).unwrap();
            assert_eq!(load(&path), (settings, vec![]), "{choice:?}");
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .any(|l| l == format!("gpu_memory={}", choice.key())));
        }
        // 自動へ戻すと行が消え、今までと同じ中身になる
        save(&path, &Settings::default()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("gpu_memory"));
        assert_eq!(load(&path), (Settings::default(), vec![]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_settings_file_from_before_the_gpu_memory_reads_as_automatic_without_a_reason() {
        let (read, problems) = parse("language=en\nundo_budget_mib=512\ncompositing=cpu\n");
        assert_eq!((read.gpu_memory, problems), (GpuMemory::Auto, vec![]));
        assert_eq!(
            read.undo_budget,
            Budget::Mib(512),
            "ほかの項目は今までどおり"
        );
        // 値の無い行・空の値は壊れた値の扱い
        let (read, problems) = parse("gpu_memory=\n");
        assert_eq!(read.gpu_memory, GpuMemory::Auto);
        assert_eq!(
            problems,
            vec![Problem::Invalid {
                key: "gpu_memory",
                value: String::new()
            }]
        );
    }

    #[test]
    fn a_broken_gpu_memory_falls_back_to_automatic_with_a_reason_and_keeps_the_rest() {
        for bad in [
            "medium",
            "AUTO",
            "255",
            "32769",
            "-1",
            "1e3",
            "12.5",
            "99999999999999999999",
            "1024MiB",
        ] {
            let (read, problems) = parse(&format!("language=en\ngpu_memory={bad}\nbackups=3\n"));
            assert_eq!(read.gpu_memory, GpuMemory::Auto, "{bad}");
            assert_eq!(
                problems,
                vec![Problem::Invalid {
                    key: "gpu_memory",
                    value: bad.to_owned()
                }],
                "{bad}"
            );
            assert_eq!(
                (read.lang, read.backups),
                (Lang::En, BackupKeep::Count(3)),
                "ほかの項目は生かす: {bad}"
            );
            // 理由は短い文で、設定の名前を言う
            let ja = problems[0].text(Lang::Ja);
            let en = problems[0].text(Lang::En);
            assert!(
                ja.contains("GPU のメモリ") && en.contains("GPU memory"),
                "{ja} / {en}"
            );
        }
        // 読み直して書き直すと、壊れた行は消える（直前まで読めた値は残る）
        let dir = temp_dir("gpu-memory-broken");
        let path = dir.join("settings.conf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "language=en\ngpu_memory=lots\nbackups=3\n").unwrap();
        let (read, problems) = load(&path);
        assert_eq!(problems.len(), 1);
        save(&path, &read).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=en\nbackups=3\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_disk_cache_settings_survive_a_restart_and_broken_values_fall_back_one_by_one() {
        let dir = temp_dir("disk-cache");
        let path = dir.join("settings.conf");
        let folder = dir.join("cache");
        let chosen = Settings {
            lang: Lang::En,
            disk_cache: false,
            disk_cache_folder: Some(folder.clone()),
            disk_cache_limit: DiskLimit::Gib(16),
            ..Settings::default()
        };
        save(&path, &chosen).unwrap();
        assert_eq!(load(&path), (chosen.clone(), vec![]));
        let written = std::fs::read_to_string(&path).unwrap();
        for line in ["disk_cache=off", "disk_cache_limit_gib=16"] {
            assert!(written.lines().any(|l| l == line), "{line}\n{written}");
        }
        assert!(written.contains("disk_cache_folder="), "{written}");
        // 既定（入・一時フォルダ・自動）は書かない
        save(&path, &with_lang(Lang::En)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        // 正しくない値は、その項目だけ既定へ
        for (line, key) in [
            ("disk_cache_limit_gib=0", "disk_cache_limit_gib"),
            ("disk_cache_limit_gib=5000", "disk_cache_limit_gib"),
            ("disk_cache_limit_gib=lots", "disk_cache_limit_gib"),
            ("disk_cache_folder=relative/cache", "disk_cache_folder"),
        ] {
            let (read, problems) = parse(&format!("language=en\n{line}\nbackups=3\n"));
            assert_eq!(read.disk_cache_limit, DiskLimit::Auto, "{line}");
            assert_eq!(read.disk_cache_folder, None, "{line}");
            assert_eq!(read.backups, BackupKeep::Count(3), "{line}");
            assert!(
                matches!(&problems[..], [Problem::Invalid { key: k, .. }] if *k == key),
                "{line}: {problems:?}"
            );
        }
        assert!(
            parse("disk_cache=nonsense\n").0.disk_cache,
            "読めない値は入のまま"
        );
        for lang in Lang::ALL {
            for key in ["disk_cache", "disk_cache_folder", "disk_cache_limit_gib"] {
                assert_ne!(
                    setting_name(lang, key),
                    setting_name(lang, "unknown"),
                    "{key}"
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_cache_limits_follow_the_budgets_and_the_free_space() {
        const GIB: u64 = 1 << 30;
        let s = Settings {
            undo_budget: Budget::Mib(512),
            source_budget: Budget::Mib(4096),
            ..Settings::default()
        };
        let c = s.cache_settings(16384, Some(100 * GIB));
        assert!(c.enabled);
        assert_eq!(
            c.memory_limit,
            (512 + 4096) * 1024 * 1024,
            "レイヤーのメモリと取り消し履歴の和"
        );
        assert_eq!(c.disk_limit, 50 * GIB, "自動は空きの半分");
        assert_eq!(
            s.cache_settings(16384, Some(1000 * GIB)).disk_limit,
            64 * GIB,
            "自動は 64 GiB まで"
        );
        assert_eq!(s.cache_settings(16384, None).disk_limit, 64 * GIB);
        assert_eq!(c.folder, Some(std::env::temp_dir()));
        let chosen = Settings {
            disk_cache: false,
            disk_cache_limit: DiskLimit::Gib(8),
            disk_cache_folder: Some(PathBuf::from("/cache")),
            ..s
        };
        let c = chosen.cache_settings(16384, Some(GIB));
        assert!(!c.enabled);
        assert_eq!(c.disk_limit, 8 * GIB, "指定は空きによらない");
        assert_eq!(c.folder, Some(PathBuf::from("/cache")));
    }
}
