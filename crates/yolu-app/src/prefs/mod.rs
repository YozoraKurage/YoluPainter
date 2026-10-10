//! 設定のウィンドウ（編集 → 設定…、Ctrl+,）。左に区分（一般・ペン・ショートカット・表示・メモリ・処理・3D ビュー・ファイル・Live Link と外からの操作・更新）、
//! 上に項目の名前で探す欄、右にその区分の項目。区分ごとに右の欄をスクロールし、前に開いた区分を覚える。値は `AppState::prefs` に入り、設定のファイル
//! （`settings`）へは次のフレームで書かれる。ウィンドウは浮いたウィンドウの骨組み（`ui::window`）で、見出しをドラッグして動かせる。選択肢はポップアップ
//! （`m2_menu::Popup::Pref`）。
//!
//! - 区分と項目の表は `sections`（`Category`・`Item`）、ウィンドウの枠（左の区分・探す欄・右の欄・Esc）は `window`。
//! - **ショートカット**の区分は、キーとマウスの割り当て・パイメニューの表（`shortcuts::editor`）を右の欄に出す。ショートカットの区分の中の探す欄
//!   （名前で探す・キーで探す）は、表の中だけを絞る（上の探す欄は設定の項目を探す）。
//! - **ペン**の区分は、macOS の「タブレットの筆圧（試し）」（`PrefsState::tablet_row`）と、Windows の「ペンの入力」（`PrefsState::pen_input_row`）に、
//!   筆圧の調整（`pen::window`）を並べる。タブレットの筆圧は、Wacom・XP-Pen などのドライバーが標準のイベントで送る筆圧・傾き・消しゴムの端を読むか
//!   （`pen::mac_tablet`。既定は入）。ペンの入力は、ペンを Windows Ink と WinTab のどちらで読むか（`pen::win_tab`。既定は Windows Ink。WinTab が使えない
//!   PC では Windows Ink のまま）。切り替えは次のフレームでペンの受け口の札に届く（`YoluApp::frame_body`）。
//! - **メモリの予算**（取り消し履歴・レイヤーのメモリ）は**プロジェクト全体**の上限で、全テクスチャセットの合計が設定を超えない
//!   （`sync_budgets`）。描けるのは今のセットだけなので、今のセットの文書に「設定 − ほかのセットが使っている量（画素・履歴）」を入れ、
//!   セットを切り替える・開くときに入れ直す。自動は物理メモリから（`settings::BudgetKind`）。描いている間は入れず、終わったフレームで
//!   入れる。1 回の操作・最小の取り消し段数はセットごとの値のまま（1 回の操作は同時に 1 つしか走らない）。今の画素がすでに予算を
//!   超えているときは画素を捨てず、予算をその量まで広げて知らせる（それ以上は足せない）。
//! - **CPU のスレッド**は rayon の全体のスレッドプールで、起動のときに決まる（`settings::apply_thread_setting`）。変えた値は次の起動から
//!   効くので、行に「再起動で反映」と出す。
//! - **ディスクキャッシュ**（既定は入。「メモリ」の区分）は、メモリの上限（レイヤーのメモリと取り消し履歴の予算の和）を超えた分のタイルの中身を、置き場所の
//!   フォルダのキャッシュのファイルへ逃がす（`yolu_core::tile_cache`。どのセットの・レイヤーの・取り消しの写しのタイルかは区別せず、使っていない
//!   ものから）。入のときは、今のセットの画素の予算を「メモリの上限＋ディスクの上限 − ほかのセットの画素」にする。ディスクの上限の自動は
//!   64 GiB と、置き場所の空き（そのフォルダで初めて測ったとき）の半分の小さい方。ディスクから読めないタイルが出たら、それを持つセットを
//!   読むだけにする（保存は開いたときの中身のまま。保存したことの無いセットは元の中身が無いので、そのセットだけ保存と復旧用の書き置きに
//!   入れない。`check_tile_cache`）。
//! - **垂直同期**（既定は切 = 待たない。「表示」の区分）は、ウィンドウの面の同期（`view3d::render::wgpu_configuration`）で、ウィンドウを作るときに決まる。変えた値は
//!   次の起動から効くので、行に「再起動で反映」と出す（`PrefsState::vsync_at_start`）。切の間は、アプリがフレームの間隔に下限をかける（`pacing`）。
//! - **表示の合成**は 2D のキャンバスの表示の方針（`YoluApp::apply_compositing` がキャンバスの表示に入れる。自動は環境変数
//!   `YOLUPAINTER_CANVAS` か自動）。保存・書き出し・3D ビューの値の合成は、どれでも CPU が正本。
//! - **更新**の区分（起動時に更新を確かめる・試験版を使う）は、更新の部品（`update`）が持つ値で、公開鍵を組み込んだビルドだけに出る。

mod sections;
mod window;

pub use sections::{search, Category, Item};
pub use window::{drawn_content_height, last_rect, show, PANE_TOP};

use std::path::PathBuf;

use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::gpu_memory::{self, GpuMemory};
use crate::lang::Lang;
use crate::m2::UiOp;
use crate::notice::Source;
use crate::settings::{
    system_memory_mib, Budget, BudgetKind, Compositing, DiskLimit, PenApi, Settings,
    EXPORT_PADDINGS, MAX_CPU_THREADS, MAX_MIN_UNDO_STEPS,
};
use crate::state::{Action, AppState, DialogRequest};
use crate::ui::menu::Entry;
use crate::view3d::navigation::{OrbitCenter, ZoomCenter};

/// 行と行の間（UV ワイヤーフレームの行も、これで置く）。
pub(crate) const GAP: f32 = 4.0;
/// 「すべて残す」を切ったときにスライダーが戻る数（まだ数を選んでいないとき）。
const DEFAULT_BACKUP_COUNT: u32 = 10;

/// 設定の 1 つの値の選び。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pref {
    /// 書き出しのパディング（テクセル。-1 は無限に広げる）。
    ExportPadding(i32),
    LiveLinkOnStartup(bool),
    /// Unity から受けたマテリアルの値を .ylp に保存するか。
    LiveLinkKeepValues(bool),
    /// 外からの操作（MCP のクライアント・コマンドラインなど）を受けるか。入れている間だけ待ち受ける（`mcp_server`）。
    ExternalOps(bool),
    /// 外からの操作を待つ番号（範囲の外は断る）。
    ExternalOpsPort(u16),
    /// macOS のタブレットの筆圧・傾き・消しゴムの端を読むか（「ペン」の節。macOS だけに出る）。
    TabletPressure(bool),
    /// ベイクで GPU の RT コア（ray query）を使うか（「表示」の節。切ると compute）。
    BakeRayQuery(bool),
    /// Windows のペンを Windows Ink と WinTab のどちらで読むか（「ペン」の節。Windows だけに出る）。
    PenInput(PenApi),
    Budget(BudgetKind, Budget),
    MinUndoSteps(u32),
    /// None は自動。
    CpuThreads(Option<u32>),
    Compositing(Compositing),
    /// 画面の更新を垂直同期まで待たせるか（ウィンドウの面の同期。変えた値は次の起動から効く）。
    Vsync(bool),
    /// GPU のメモリ（自動・低・標準・高、詳しくで指定した合計）。
    GpuMemory(GpuMemory),
    /// None は既定。
    LibraryFolder(Option<PathBuf>),
    /// 3D ビューの回転の中心（画面の中心・面の位置・モデルの中心・テクスチャセットの中心）。3D ビューの表示の設定の「視点」と同じ値。
    OrbitCenter(OrbitCenter),
    /// 3D ビューのズームの中心。
    ZoomCenter(ZoomCenter),
    /// 3D ビューを軸の向きで正投影にする（軸の視点・スナップ回転で軸に入ったとき）。
    AxisOrthographic(bool),
    /// ディスクキャッシュの入切。
    DiskCache(bool),
    /// ディスクキャッシュの上限。
    DiskCacheLimit(DiskLimit),
    /// ディスクキャッシュの置き場所（None は OS の一時フォルダ）。
    DiskCacheFolder(Option<PathBuf>),
}

/// 選択肢のポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefChoice {
    Language,
    ExportPadding,
    Budget(BudgetKind),
    CpuThreads,
    Compositing,
    /// Windows のペンの入力（Windows Ink・WinTab）。
    PenInput,
    GpuMemory,
    OrbitCenter,
    ZoomCenter,
    DiskCacheLimit,
}

/// 設定のウィンドウの操作（`Action::Prefs`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefsAction {
    /// 前に開いた区分で開く。
    Open,
    Close,
    Set(Pref),
    /// 退避を残す数を選ぶ（上限を超える数は上限にする）。
    SetBackups(BackupKeep),
    /// 棚の場所のフォルダを選ぶウィンドウを頼む。
    ChooseLibraryFolder,
    /// GPU のメモリの「詳しく」を開く・閉じる（ウィンドウの中だけの状態。設定には書かない）。
    GpuDetails(bool),
    /// ディスクキャッシュの置き場所のフォルダを選ぶウィンドウを頼む。
    ChooseCacheFolder,
    /// ディスクキャッシュの「詳しく」（上限・置き場所）を開く・閉じる（ウィンドウの中だけの状態。設定には書かない）。
    CacheDetails(bool),
    /// 設定のウィンドウを、その区分で開く（開いていれば区分を移す）。
    OpenAt(Category),
    /// 左で区分を選ぶ（探す欄は空にする）。
    Choose(Category),
}

/// 設定のウィンドウの状態と、いま選んでいる設定。
#[derive(Debug)]
pub struct PrefsState {
    pub open: bool,
    /// 開いている区分（閉じても覚えていて、次に開いたときも同じ区分。設定のファイルには書かない）。
    pub category: Category,
    /// 上の探す欄に打った文字（空でなければ、全部の区分から名前の合う項目だけを右に並べる）。
    pub search: String,
    /// 退避の数のスライダーをドラッグしている最中。設定のファイルへは、離すまで退避の数を書かない（ドラッグの間じゅうフレームごとに
    /// 同期付きの書き込みをしない。Esc で戻した値は、書いてある値と同じなので書かない）。
    pub dragging: bool,
    /// 「すべて残す」のあいだスライダーに見せる数（最後に選んだ数）。
    remembered_backups: u32,
    /// 今の設定（言語は `AppState::lang` が持つので、ここの `lang` は使わない。`AppState::settings` が重ねる）。
    pub settings: Settings,
    /// 起動のときに rayon に入れたスレッドの設定（今の設定と違えば「再起動で反映」）。
    pub threads_at_start: Option<u32>,
    /// 起動のときにウィンドウの面へ入れた「垂直同期」の設定（今の設定と違えば「再起動で反映」）。
    pub vsync_at_start: bool,
    /// このマシンの物理メモリ（MiB。自動の予算の元。試験は差し替える）。
    pub ram_mib: u64,
    /// このマシンの論理プロセッサの数（スレッドの選択肢と自動の表示。試験は差し替える）。
    pub cores: u32,
    /// 予算を文書へ入れるか（設定を読んだ・予算を選んだあと。試験の AppState と作っただけの AppState は core の既定のまま）。
    managed: bool,
    /// 画素がすでに予算を超えていると知らせた文書（通し番号）と、そのとき入れた予算（同じ状況で知らせ直さない）。
    over: Option<(u128, u64)>,
    /// アダプターから分かった GPU のメモリ（自動・低・標準・高の元。`YoluApp::with_render_state` が入れる。試験は差し替える）。
    pub gpu: gpu_memory::Adapter,
    /// GPU のメモリの「詳しく」を開いているか（ウィンドウの中だけの状態）。
    pub gpu_details: bool,
    /// ディスクキャッシュの「詳しく」を開いているか（ウィンドウの中だけの状態）。
    pub cache_details: bool,
    /// 「ペン」の区分に、タブレットの筆圧の切り替えを出すか。macOS だけ（試験は差し替える）。
    pub tablet_row: bool,
    /// 「ペン」の区分に、ペンの入力（Windows Ink・WinTab）の選びを出すか。Windows だけ（試験は差し替える）。
    pub pen_input_row: bool,
    /// 合計のスライダーを押し始めたときの「GPU のメモリ」の選び（自動・段・指定）。Esc で止めたとき、押し始めの量ではなく
    /// この選びへ戻す（量へ戻すと、自動・段が「指定」に置き換わる）。押していないあいだは None。
    gpu_memory_before_drag: Option<GpuMemory>,
    /// 右の欄のずらした量（中身が欄に収まらないとき、共通のスクロールで送る。区分を移すと 0 に戻す）。
    scroll: f32,
    /// ウィンドウの見出しをドラッグして動かした量（画面の真ん中からの）。
    offset: egui::Vec2,
    /// 前のフレームに、右の欄がショートカットの表だったか（離れたとき・閉じたときに、割り当て待ちとパイの項目選びをやめる）。
    shortcuts_shown: bool,
    /// 前のフレームに開いていたか（閉じたフレームに、ぶつかるキーを保存していないことを知らせる）。
    was_open: bool,
    /// 左の区分の列のずらした量（低い画面で区分が列に収まらないとき、共通のスクロールで送る）。
    side_scroll: f32,
    /// ディスクキャッシュの置き場所ごとの、そこで初めて測った空き（バイト。分からなければ None）。自動のディスクの上限の元。試験は値を入れる。
    pub cache_free: Vec<(PathBuf, Option<u64>)>,
    /// 前のフレームまでに見た、ディスクから読めなかった回数（`tile_cache::read_failures`。増えたら読めないタイルを持つセットを探す）。
    cache_failures: u64,
}

impl Default for PrefsState {
    fn default() -> Self {
        Self {
            open: false,
            category: Category::default(),
            search: String::new(),
            dragging: false,
            remembered_backups: DEFAULT_BACKUP_COUNT,
            settings: Settings::default(),
            threads_at_start: None,
            vsync_at_start: false,
            ram_mib: system_memory_mib(),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
            managed: false,
            over: None,
            gpu: gpu_memory::Adapter::default(),
            gpu_details: false,
            cache_details: false,
            tablet_row: cfg!(target_os = "macos"),
            pen_input_row: cfg!(windows),
            gpu_memory_before_drag: None,
            scroll: 0.0,
            offset: egui::Vec2::ZERO,
            shortcuts_shown: false,
            was_open: false,
            side_scroll: 0.0,
            cache_free: Vec::new(),
            cache_failures: yolu_core::tile_cache::read_failures(),
        }
    }
}

impl PrefsState {
    /// 設定のウィンドウが開いていて、その区分を見ているか（探す欄に文字があるときは、区分でなく探した結果を見ている）。
    pub fn shows(&self, category: Category) -> bool {
        self.open && self.category == category && self.search.trim().is_empty()
    }

    /// 退避のスライダーに出す数。
    fn shown_backups(&self) -> u32 {
        match self.settings.backups {
            BackupKeep::Count(n) => n,
            BackupKeep::All => self.remembered_backups,
        }
    }
}

impl AppState {
    /// 今の設定（言語は画面の言語）。
    pub fn settings(&self) -> Settings {
        Settings {
            lang: self.lang,
            color_wheel: self.color.wheel,
            view3d_post: self.view3d.display.post,
            view3d_paint: self.view3d.projection,
            ..self.prefs.settings.clone()
        }
    }

    /// 起動のとき、読んだ設定を入れる（言語は作るときに決めてある）。予算は次の `sync_budgets` で文書へ。
    pub fn load_settings(&mut self, settings: Settings) {
        self.export.padding = settings.export_padding;
        self.prefs.threads_at_start = settings.cpu_threads;
        self.prefs.vsync_at_start = settings.vsync;
        self.color.wheel = settings.color_wheel;
        self.view3d.display.post = settings.view3d_post;
        self.view3d.projection = settings.view3d_paint;
        // 「すべて残す」を外したときに戻る数も、保存してあった数にする
        if let BackupKeep::Count(n) = settings.backups {
            self.prefs.remembered_backups = n;
        }
        self.bake.follow_ray_query(settings.bake_ray_query);
        self.prefs.settings = settings;
        self.prefs.managed = true;
    }

    pub fn prefs_apply(&mut self, action: PrefsAction) {
        let lang = self.lang;
        match action {
            PrefsAction::Open => self.prefs.open = true,
            PrefsAction::OpenAt(category) => {
                self.prefs.open = true;
                self.prefs_choose(category);
            }
            PrefsAction::Choose(category) => self.prefs_choose(category),
            PrefsAction::Close => {
                self.prefs.open = false;
                // 次に開くときは、探す欄は空で、前に開いた区分から（探していた間も、区分そのものは変わらない）
                self.prefs.search.clear();
                self.prefs.dragging = false;
                self.prefs.gpu_memory_before_drag = None;
            }
            PrefsAction::SetBackups(keep) => {
                let keep = match keep {
                    BackupKeep::Count(n) => BackupKeep::Count(n.min(MAX_BACKUPS_TO_KEEP)),
                    all => all,
                };
                if let BackupKeep::Count(n) = keep {
                    self.prefs.remembered_backups = n;
                }
                self.prefs.settings.backups = keep;
            }
            PrefsAction::ChooseLibraryFolder => {
                self.dialog_request = Some(DialogRequest::PrefsLibraryFolder)
            }
            PrefsAction::GpuDetails(open) => self.prefs.gpu_details = open,
            PrefsAction::CacheDetails(open) => self.prefs.cache_details = open,
            PrefsAction::ChooseCacheFolder => {
                self.dialog_request = Some(DialogRequest::PrefsCacheFolder)
            }
            PrefsAction::Set(pref) => match pref {
                Pref::LiveLinkOnStartup(v) => self.prefs.settings.livelink_on_startup = v,
                Pref::LiveLinkKeepValues(v) => self.prefs.settings.livelink_keep_values = v,
                Pref::ExternalOps(v) => self.prefs.settings.external_ops = v,
                Pref::TabletPressure(v) => self.prefs.settings.tablet_pressure = v,
                Pref::BakeRayQuery(v) => {
                    self.prefs.settings.bake_ray_query = v;
                    self.bake.follow_ray_query(v);
                }
                Pref::PenInput(api) => self.prefs.settings.pen_input = api,
                Pref::ExternalOpsPort(port) => {
                    if yolu_mcp::valid_port(port) {
                        self.prefs.settings.external_ops_port = port;
                    } else {
                        self.refuse(
                            Source::Settings,
                            lang.pick(
                                "ポート番号は 1024〜65535 です。",
                                "The port is a number from 1024 to 65535.",
                            ),
                        );
                    }
                }
                Pref::ExportPadding(v) => {
                    if EXPORT_PADDINGS.contains(&v) {
                        self.prefs.settings.export_padding = v;
                        self.export.padding = v;
                    }
                }
                Pref::Budget(kind, budget) => {
                    let budget = match budget {
                        Budget::Mib(n) => {
                            let (lo, hi) = kind.range();
                            Budget::Mib(n.clamp(lo, hi))
                        }
                        auto => auto,
                    };
                    self.prefs.settings.set_budget(kind, budget);
                    self.prefs.managed = true;
                    self.sync_budgets();
                }
                Pref::MinUndoSteps(n) => {
                    self.prefs.settings.min_undo_steps = n.min(MAX_MIN_UNDO_STEPS);
                    self.prefs.managed = true;
                    self.sync_budgets();
                }
                Pref::CpuThreads(n) => {
                    self.prefs.settings.cpu_threads = n.map(|n| n.clamp(1, MAX_CPU_THREADS));
                }
                Pref::Compositing(c) => self.prefs.settings.compositing = c,
                Pref::Vsync(on) => self.prefs.settings.vsync = on,
                Pref::GpuMemory(choice) => {
                    self.prefs.settings.gpu_memory = match choice {
                        GpuMemory::Mib(n) => GpuMemory::Mib(
                            n.clamp(gpu_memory::MIN_TOTAL_MIB, gpu_memory::MAX_TOTAL_MIB),
                        ),
                        level => level,
                    };
                }
                Pref::OrbitCenter(center) => self.prefs.settings.navigation.orbit = center,
                Pref::ZoomCenter(center) => self.prefs.settings.navigation.zoom = center,
                Pref::AxisOrthographic(on) => self.prefs.settings.navigation.axis_ortho = on,
                Pref::LibraryFolder(folder) => match folder {
                    Some(path) if !path.is_absolute() => {
                        self.refuse(
                            Source::Settings,
                            lang.pick(
                                "ライブラリの場所は絶対パスで指定します。",
                                "The library folder must be an absolute path.",
                            ),
                        );
                    }
                    folder => self.prefs.settings.library_folder = folder,
                },
                Pref::DiskCache(on) => {
                    self.prefs.settings.disk_cache = on;
                    self.prefs.managed = true;
                    self.sync_budgets();
                }
                Pref::DiskCacheLimit(limit) => {
                    let (lo, hi) = DiskLimit::RANGE;
                    self.prefs.settings.disk_cache_limit = match limit {
                        DiskLimit::Gib(n) => DiskLimit::Gib(n.clamp(lo, hi)),
                        auto => auto,
                    };
                    self.prefs.managed = true;
                    self.sync_budgets();
                }
                Pref::DiskCacheFolder(folder) => match folder {
                    Some(path) if !path.is_absolute() => {
                        self.refuse(
                            Source::Settings,
                            lang.pick(
                                "キャッシュの場所は絶対パスで指定します。",
                                "The cache folder must be an absolute path.",
                            ),
                        );
                    }
                    folder => {
                        self.prefs.settings.disk_cache_folder = folder;
                        self.prefs.managed = true;
                        self.sync_budgets();
                    }
                },
            },
        }
    }

    /// 区分を選ぶ（探す欄は空にして、右の欄は先頭から）。
    fn prefs_choose(&mut self, category: Category) {
        // このビルドに無い区分（公開鍵の無いビルドの「更新」）は開かず、「一般」にする
        let category = if category.available(self) {
            category
        } else {
            Category::default()
        };
        if self.prefs.category != category || !self.prefs.search.is_empty() {
            self.prefs.scroll = 0.0;
        }
        self.prefs.category = category;
        self.prefs.search.clear();
    }

    /// タイルの中身を逃がす係に入れる設定（置き場所の空きは、そのフォルダで初めて測った値を使い回す）。
    fn cache_settings(&mut self) -> yolu_core::tile_cache::CacheSettings {
        let folder = self.prefs.settings.disk_cache_folder();
        if !self.prefs.cache_free.iter().any(|(f, _)| *f == folder) {
            let free = crate::recovery::system_probe()(&folder).map(|d| d.available);
            self.prefs.cache_free.push((folder, free));
        }
        self.prefs
            .settings
            .cache_settings(self.prefs.ram_mib, self.cache_free_now())
    }

    /// ディスクから読めなかったタイルが出たら（`tile_cache::read_failures` が増えたら）、読めないタイルを持つセットを読むだけにして
    /// 知らせる。読むだけのセットは描けず、保存は開いたときの中身のまま書く（欠けた中身を書かない）。保存したことの無いセットは
    /// 元の中身が無いので、そのセットだけ保存と復旧用の書き置きに入らない（ほかのセットは書ける）。毎フレーム呼べる。
    pub fn check_tile_cache(&mut self) {
        let failures = yolu_core::tile_cache::read_failures();
        if failures == self.prefs.cache_failures {
            return;
        }
        self.prefs.cache_failures = failures;
        let lang = self.lang;
        let reason = lang.pick(
            "ディスクのキャッシュから読めないタイルがあります",
            "Some tiles cannot be read back from the disk cache",
        );
        // 保存したプロジェクトの中にあるセットは、開いたときの中身のまま書ける。無いセット（新しいプロジェクト・前の保存の後に
        // 追加したセット）は元の中身が無いので、そのセットだけ保存と復旧用の書き置きに入らない（保存のたびに知らせる）
        let base = self.project.as_ref().map(|p| p.project_shared());
        let (mut names, mut unsaved) = (Vec::new(), Vec::new());
        for i in 0..self.sets.len() {
            if self.sets.get(i).is_some_and(|s| s.read_only.is_some())
                || !self.set_doc(i).has_unreadable_tiles()
            {
                continue;
            }
            if let Some(set) = self.sets.get_mut(i) {
                set.read_only = Some(reason.into());
                set.waiting_inputs = false;
                names.push(set.name.clone());
                if !base
                    .as_ref()
                    .is_some_and(|b| b.sets().iter().any(|s| s.id == set.id))
                {
                    unsaved.push(set.name.clone());
                }
            }
        }
        if names.is_empty() {
            return;
        }
        // どのセットか・保存できないものを、短い理由として添える
        let ja = |v: &[String]| v.iter().map(|n| format!("「{n}」")).collect::<String>();
        let en = |v: &[String]| {
            let quoted: Vec<String> = v.iter().map(|n| format!("\"{n}\"")).collect();
            quoted.join(", ")
        };
        let (mut ja_why, mut en_why): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        if names.len() > unsaved.len() {
            ja_why.push("最後に保存した後の編集は保存できません".to_owned());
            en_why.push("Edits since the last save cannot be saved".to_owned());
        }
        if !unsaved.is_empty() {
            ja_why.push(format!(
                "{}は保存したことが無いため、保存に入りません",
                ja(&unsaved)
            ));
            en_why.push(format!(
                "{} has never been saved, so it will be left out of the save",
                en(&unsaved)
            ));
        }
        // 「何を（なぜ）」の 1 文のあとに、保存で失うものを文で続ける
        let mut text = lang.with_reason(
            lang.pick(
                format!("テクスチャセット{}を読むだけにしました", ja(&names)),
                format!("Texture set {} is now read-only", en(&names)),
            ),
            reason,
        );
        for why in lang.pick(ja_why, en_why) {
            text += lang.pick("", " ");
            text += &why;
            text += lang.pick("。", ".");
        }
        self.fail(Source::TextureSet, text);
    }

    /// 選んだ予算を今のセットの文書に入れる（全体の予算から、ほかのセットが使っている量を引く）。入れる値が今と同じなら何もしない
    /// ので、毎フレーム呼べる。描いている間は入れず、終わった後のフレームで入れる。今の画素が（ほかのセットの分を引いた）予算を
    /// 超えていれば、画素は捨てずに予算をその量まで広げ、同じ状況では 1 度だけ知らせる。
    pub fn sync_budgets(&mut self) {
        if !self.prefs.managed || self.is_stroking() {
            return;
        }
        let wanted = self.prefs.settings.budgets(self.prefs.ram_mib);
        // ディスクキャッシュが入なら、画素の予算はメモリの上限＋ディスクの上限（書けなくなった後は、そのとき使っていた量まで）
        let cache = self.cache_settings();
        yolu_core::tile_cache::configure(&cache);
        let total_source = if cache.enabled {
            cache
                .memory_limit
                .saturating_add(usable_disk(cache.disk_limit))
        } else {
            wanted.source
        };
        let current = self.sets.current_index();
        let (mut other_source, mut other_history) = (0u64, 0u64);
        for i in (0..self.sets.len()).filter(|&i| i != current) {
            let doc = self.set_doc(i);
            other_source = other_source.saturating_add(doc.allocated_bytes());
            other_history = other_history.saturating_add(doc.history_bytes());
        }
        let undo = wanted.undo.saturating_sub(other_history);
        let source = total_source.saturating_sub(other_source);
        let own = self.doc.allocated_bytes();
        let over = source < own;
        let id = self.doc.id();
        // 4 つとも core が断らない値（画素の予算は今の画素以上）。同じ値は入れ直さない（履歴の整理を毎フレーム走らせない）
        let source = source.max(own);
        let doc = &mut self.doc;
        if doc.minimum_undo_steps() != wanted.min_undo_steps {
            let _ = doc.set_minimum_undo_steps(wanted.min_undo_steps);
        }
        if doc.undo_budget_bytes() != undo {
            let _ = doc.set_undo_budget_bytes(undo);
        }
        if doc.stroke_budget_bytes() != wanted.stroke {
            let _ = doc.set_stroke_budget_bytes(wanted.stroke);
        }
        if doc.source_budget_bytes() != source {
            let _ = doc.set_source_budget_bytes(source);
        }
        if !over {
            self.prefs.over = None;
        } else if self.prefs.over != Some((id, source)) {
            self.prefs.over = Some((id, source));
            let lang = self.lang;
            self.refuse(Source::Settings, lang
                .pick(
                    "レイヤーのメモリがすでに予算を超えているので、予算を上げるまで追加できません。",
                    "The layer memory is already over the budget; nothing can be added until it is raised.",
                )
                );
        }
    }

    /// 読み込み（.ylp を開く・書き出しが写した文書を戻す）で 1 つの文書に許すレイヤーの画素のバイト数。
    pub fn load_source_bytes(&self) -> u64 {
        let plain = self.prefs.settings.load_source_bytes(self.prefs.ram_mib);
        // ディスクキャッシュを係に入れている間は、メモリの上限＋ディスクの上限まで（`sync_budgets` と同じ）
        if !(self.prefs.managed && self.prefs.settings.disk_cache) {
            return plain;
        }
        let cache = self
            .prefs
            .settings
            .cache_settings(self.prefs.ram_mib, self.cache_free_now());
        plain.max(
            cache
                .memory_limit
                .saturating_add(usable_disk(cache.disk_limit)),
        )
    }

    /// GPU のメモリの設定とアダプターから配った 3 つの予算（3D の絵・キャンバスの合成・棚のサムネイル。`YoluApp` が変わったときに入れる）。
    pub fn gpu_budgets(&self) -> gpu_memory::Budgets {
        gpu_memory::budgets(self.prefs.settings.gpu_memory, &self.prefs.gpu)
    }

    /// 今の GPU のメモリの合計（MiB。「詳しく」のスライダーに見せる。選んだ段・自動もこの数になる）。
    pub fn gpu_total_mib(&self) -> u32 {
        (gpu_memory::total_bytes(self.prefs.settings.gpu_memory, &self.prefs.gpu) / gpu_memory::MIB)
            as u32
    }
}

// ───────── 名前 ─────────

pub(crate) fn padding_name(lang: Lang, texels: i32) -> String {
    match texels {
        0 => lang.pick("なし", "No padding").into(),
        n if n < 0 => lang.pick("無限に広げる", "Dilation infinite").into(),
        n => lang.pick(format!("{n} px 広げる"), format!("Dilation {n} px")),
    }
}

fn budget_name(lang: Lang, kind: BudgetKind, budget: Budget, ram_mib: u64) -> String {
    match budget {
        Budget::Auto => {
            let mib = kind.automatic_mib(ram_mib);
            lang.pick(format!("自動（{mib} MiB）"), format!("Auto ({mib} MiB)"))
        }
        Budget::Mib(n) => format!("{n} MiB"),
    }
}

/// ディスクに置ける量: 上限（書けなくなった後は、そのとき置いていた量まで）。
fn usable_disk(limit: u64) -> u64 {
    let status = yolu_core::tile_cache::status();
    if status.write_failed {
        status.disk_bytes.min(limit)
    } else {
        limit
    }
}

/// ディスクキャッシュの上限の名前（自動は今の置き場所での量を添える）。
fn disk_limit_name(lang: Lang, limit: DiskLimit, free: Option<u64>) -> String {
    match limit {
        DiskLimit::Auto => {
            let gib = DiskLimit::Auto.bytes(free) >> 30;
            lang.pick(format!("自動（{gib} GiB）"), format!("Auto ({gib} GiB)"))
        }
        DiskLimit::Gib(n) => format!("{n} GiB"),
    }
}

impl AppState {
    /// WinTab が使えず Windows Ink で読んでいることを知らせる（理由は `pen::Unavailable`。設定の選びは WinTab のままで、使える PC では WinTab で読む）。
    pub fn wintab_unavailable(&mut self, why: crate::pen::Unavailable) {
        let text = why.text(self.lang);
        self.warn(Source::Settings, text);
    }

    /// 今の置き場所で測った空き（まだ測っていなければ None）。
    fn cache_free_now(&self) -> Option<u64> {
        let folder = self.prefs.settings.disk_cache_folder();
        self.prefs
            .cache_free
            .iter()
            .find(|(measured, _)| *measured == folder)
            .and_then(|(_, free)| *free)
    }
}

fn threads_name(lang: Lang, threads: Option<u32>, cores: u32) -> String {
    match threads {
        None => lang.pick(format!("自動（{cores}）"), format!("Automatic ({cores})")),
        Some(1) => lang
            .pick("1（並列にしない）", "1 (no parallel work)")
            .into(),
        Some(n) => n.to_string(),
    }
}

/// ペンの入力の名前（どちらも製品の名前なので日英で同じ）。
fn pen_api_name(api: PenApi) -> &'static str {
    match api {
        PenApi::Ink => "Windows Ink",
        PenApi::WinTab => "WinTab",
    }
}

fn compositing_name(lang: Lang, c: Compositing) -> &'static str {
    match c {
        Compositing::Auto => lang.pick("自動", "Automatic"),
        Compositing::Gpu => "GPU",
        Compositing::Cpu => "CPU",
    }
}

/// スレッドの選択肢（自動・1・2 の冪で論理プロセッサの数より小さいもの・論理プロセッサの数）。
fn thread_choices(cores: u32) -> Vec<Option<u32>> {
    let mut v = vec![None, Some(1)];
    let mut n = 2;
    while n < cores {
        v.push(Some(n));
        n *= 2;
    }
    if cores > 1 {
        v.push(Some(cores));
    }
    v
}

/// ポップアップの項目。選んでいる値は印。今の値が選択肢に無ければ（ファイルに書いた中途半端な数）末尾に足す。
pub fn entries(app: &AppState, choice: PrefChoice) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let s = &app.prefs.settings;
    let set = |pref: Pref| Action::Prefs(PrefsAction::Set(pref));
    match choice {
        PrefChoice::Language => Lang::ALL
            .into_iter()
            .map(|l| Entry::item(l.name(), Action::M2Ui(UiOp::Language(l))).radio(app.lang == l))
            .collect(),
        PrefChoice::ExportPadding => EXPORT_PADDINGS
            .into_iter()
            .map(|p| {
                Entry::item(padding_name(lang, p), set(Pref::ExportPadding(p)))
                    .radio(s.export_padding == p)
            })
            .collect(),
        PrefChoice::Budget(kind) => {
            let ram = app.prefs.ram_mib;
            let mut values = vec![Budget::Auto];
            values.extend(kind.choices().iter().map(|n| Budget::Mib(*n)));
            if !values.contains(&s.budget(kind)) {
                values.push(s.budget(kind));
            }
            values
                .into_iter()
                .map(|b| {
                    Entry::item(budget_name(lang, kind, b, ram), set(Pref::Budget(kind, b)))
                        .radio(s.budget(kind) == b)
                })
                .collect()
        }
        PrefChoice::CpuThreads => {
            let mut values = thread_choices(app.prefs.cores);
            if !values.contains(&s.cpu_threads) {
                values.push(s.cpu_threads);
            }
            values
                .into_iter()
                .map(|n| {
                    Entry::item(
                        threads_name(lang, n, app.prefs.cores),
                        set(Pref::CpuThreads(n)),
                    )
                    .radio(s.cpu_threads == n)
                })
                .collect()
        }
        PrefChoice::Compositing => Compositing::ALL
            .into_iter()
            .map(|c| {
                Entry::item(compositing_name(lang, c), set(Pref::Compositing(c)))
                    .radio(s.compositing == c)
            })
            .collect(),
        PrefChoice::PenInput => PenApi::ALL
            .into_iter()
            .map(|a| Entry::item(pen_api_name(a), set(Pref::PenInput(a))).radio(s.pen_input == a))
            .collect(),
        PrefChoice::OrbitCenter => OrbitCenter::ALL
            .into_iter()
            .map(|c| {
                Entry::item(c.label(lang), set(Pref::OrbitCenter(c))).radio(s.navigation.orbit == c)
            })
            .collect(),
        PrefChoice::ZoomCenter => ZoomCenter::ALL
            .into_iter()
            .map(|c| {
                Entry::item(c.label(lang), set(Pref::ZoomCenter(c))).radio(s.navigation.zoom == c)
            })
            .collect(),
        PrefChoice::DiskCacheLimit => {
            let free = app.cache_free_now();
            let mut values = vec![DiskLimit::Auto];
            values.extend(DiskLimit::CHOICES.iter().map(|n| DiskLimit::Gib(*n)));
            if !values.contains(&s.disk_cache_limit) {
                values.push(s.disk_cache_limit);
            }
            values
                .into_iter()
                .map(|l| {
                    Entry::item(disk_limit_name(lang, l, free), set(Pref::DiskCacheLimit(l)))
                        .radio(s.disk_cache_limit == l)
                })
                .collect()
        }
        PrefChoice::GpuMemory => {
            let mut entries: Vec<Entry<Action>> = GpuMemory::LEVELS
                .into_iter()
                .map(|g| {
                    Entry::item(g.name(lang), set(Pref::GpuMemory(g))).radio(s.gpu_memory == g)
                })
                .collect();
            // 詳しくで量を指定しているときは、その印を末尾に（数は出さない。選び直すと段に戻る）
            if matches!(s.gpu_memory, GpuMemory::Mib(_)) {
                entries.push(
                    Entry::item(s.gpu_memory.name(lang), set(Pref::GpuMemory(s.gpu_memory)))
                        .radio(true),
                );
            }
            entries
        }
    }
}
