//! 設定の窓（編集 → 設定…、Ctrl+,）: 言語・書き出しの余白・メモリの予算（取り消し履歴・レイヤーのメモリ・1 回の操作・最小の取り消し段数）・
//! CPU のスレッド・表示の合成・棚の場所・退避を残す数。値は `AppState::prefs` に入り、設定のファイル（`settings`）へは次のフレームで書かれる。
//! 窓は浮いた窓の骨組み（`ui::window`）で、見出しをドラッグして動かせる。選択肢はポップアップ（`m2_menu::Popup::Pref`）。
//!
//! - **メモリの予算**（取り消し履歴・レイヤーのメモリ）は**プロジェクト全体**の上限で、全テクスチャセットの合計が設定を超えない
//!   （`sync_budgets`）。描けるのは今のセットだけなので、今のセットの文書に「設定 − ほかのセットが使っている量（画素・履歴）」を入れ、
//!   セットを切り替える・開くときに入れ直す。自動は物理メモリから（`settings::BudgetKind`）。描いている間は入れず、終わったフレームで
//!   入れる。1 回の操作・最小の取り消し段数はセットごとの値のまま（1 回の操作は同時に 1 つしか走らない）。今の画素がすでに予算を
//!   超えているときは画素を捨てず、予算をその量まで広げて知らせる（それ以上は足せない）。
//! - **CPU のスレッド**は rayon の全体のスレッドプールで、起動のときに決まる（`settings::apply_thread_setting`）。変えた値は次の起動から
//!   効くので、窓に「再起動で反映」と出す。
//! - **ディスクキャッシュ**（既定は入）は、メモリの上限（レイヤーのメモリと取り消し履歴の予算の和）を超えた分のタイルの中身を、置き場所の
//!   フォルダのキャッシュのファイルへ逃がす（`yolu_core::tile_cache`。どのセットの・層の・取り消しの写しのタイルかは区別せず、使っていない
//!   ものから）。入のときは、今のセットの画素の予算を「メモリの上限＋ディスクの上限 − ほかのセットの画素」にする。ディスクの上限の自動は
//!   64 GiB と、置き場所の空き（そのフォルダで初めて測ったとき）の半分の小さい方。ディスクから読めないタイルが出たら、それを持つセットを
//!   読むだけにする（保存は開いたときの中身のまま。保存したことの無いセットは元の中身が無いので、そのセットだけ保存と復旧用の書き置きに
//!   入れない。`check_tile_cache`）。
//! - **表示の合成**は 2D のキャンバスの表示の方針（`YoluApp::apply_compositing` がキャンバスの表示に入れる。自動は環境変数
//!   `YOLUPAINTER_CANVAS` か自動）。保存・書き出し・3D ビューの値の合成は、どれでも CPU が正本。

use std::path::PathBuf;

use egui::{pos2, vec2, Id, Rect, Vec2};
use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::gpu_memory::{self, GpuMemory};
use crate::lang::Lang;
use crate::m2::UiOp;
use crate::notice::Source;
use crate::settings::{
    system_memory_mib, Budget, BudgetKind, Compositing, DiskLimit, Settings, EXPORT_PADDINGS,
    MAX_CPU_THREADS, MAX_MIN_UNDO_STEPS,
};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::ui::window::{self, Spec};
use crate::view3d::navigation::{OrbitCenter, ZoomCenter};

const WIDTH: f32 = 400.0;
/// ラベルの幅（値の箱はその右）。
const LABEL_WIDTH: f32 = 150.0;
const GAP: f32 = 4.0;
/// 「すべて残す」を切ったときにスライダーが戻る数（まだ数を選んでいないとき）。
const DEFAULT_BACKUP_COUNT: u32 = 10;

fn window_id() -> Id {
    Id::new("yolu.prefs")
}

/// 最後に描いた窓の矩形（画面の点。開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// 設定の 1 つの値の選び。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pref {
    /// 書き出しの余白（テクセル。-1 は届くかぎり全部）。
    ExportPadding(i32),
    LiveLinkOnStartup(bool),
    /// Unity から受けたマテリアルの値を .ylp に保存するか。
    LiveLinkKeepValues(bool),
    /// 外からの操作（MCP のクライアント・コマンドラインなど）を受けるか。入れている間だけ待ち受ける（`mcp_server`）。
    ExternalOps(bool),
    /// 外からの操作を待つ番号（範囲の外は断る）。
    ExternalOpsPort(u16),
    Budget(BudgetKind, Budget),
    MinUndoSteps(u32),
    /// None は自動。
    CpuThreads(Option<u32>),
    Compositing(Compositing),
    /// GPU のメモリ（自動・低・標準・高、詳しくで指定した合計）。
    GpuMemory(GpuMemory),
    /// None は既定。
    LibraryFolder(Option<PathBuf>),
    /// 3D ビューの回転の中心（画面の中心・面の位置・モデルの中心・テクスチャセットの中心）。3D ビューの表示の設定の「視点」と同じ値。
    OrbitCenter(OrbitCenter),
    /// 3D ビューのズームの中心。
    ZoomCenter(ZoomCenter),
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
    GpuMemory,
    OrbitCenter,
    ZoomCenter,
    DiskCacheLimit,
}

/// 設定の窓の操作（`Action::Prefs`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefsAction {
    Open,
    Close,
    Set(Pref),
    /// 退避を残す数を選ぶ（上限を超える数は上限にする）。
    SetBackups(BackupKeep),
    /// 棚の場所のフォルダを選ぶ窓を頼む。
    ChooseLibraryFolder,
    /// GPU のメモリの「詳しく」を開く・閉じる（窓の中だけの状態。設定には書かない）。
    GpuDetails(bool),
    /// ディスクキャッシュの置き場所のフォルダを選ぶ窓を頼む。
    ChooseCacheFolder,
    /// ディスクキャッシュの「詳しく」（上限・置き場所）を開く・閉じる（窓の中だけの状態。設定には書かない）。
    CacheDetails(bool),
}

/// 設定の窓の状態と、いま選んでいる設定。
#[derive(Debug)]
pub struct PrefsState {
    pub open: bool,
    /// 退避の数のスライダーをドラッグしている最中。設定のファイルへは、離すまで退避の数を書かない（ドラッグの間じゅうフレームごとに
    /// 同期付きの書き込みをしない。Esc で戻した値は、書いてある値と同じなので書かない）。
    pub dragging: bool,
    offset: Vec2,
    /// 「すべて残す」のあいだスライダーに見せる数（最後に選んだ数）。
    remembered_backups: u32,
    /// 今の設定（言語は `AppState::lang` が持つので、ここの `lang` は使わない。`AppState::settings` が重ねる）。
    pub settings: Settings,
    /// 起動のときに rayon に入れたスレッドの設定（今の設定と違えば「再起動で反映」）。
    pub threads_at_start: Option<u32>,
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
    /// GPU のメモリの「詳しく」を開いているか（窓の中だけの状態）。
    pub gpu_details: bool,
    /// ディスクキャッシュの「詳しく」を開いているか（窓の中だけの状態）。
    pub cache_details: bool,
    /// 合計のスライダーを押し始めたときの「GPU のメモリ」の選び（自動・段・指定）。Esc で止めたとき、押し始めの量ではなく
    /// この選びへ戻す（量へ戻すと、自動・段が「指定」に置き換わる）。押していないあいだは None。
    gpu_memory_before_drag: Option<GpuMemory>,
    /// 窓の中のずらした量（小さい画面で中身が窓に収まらないとき、共通のスクロールで送る）。
    scroll: f32,
    /// ディスクキャッシュの置き場所ごとの、そこで初めて測った空き（バイト。分からなければ None）。自動のディスクの上限の元。試験は値を入れる。
    pub cache_free: Vec<(PathBuf, Option<u64>)>,
    /// 前のフレームまでに見た、ディスクから読めなかった回数（`tile_cache::read_failures`。増えたら読めないタイルを持つセットを探す）。
    cache_failures: u64,
}

impl Default for PrefsState {
    fn default() -> Self {
        Self {
            open: false,
            dragging: false,
            offset: Vec2::ZERO,
            remembered_backups: DEFAULT_BACKUP_COUNT,
            settings: Settings::default(),
            threads_at_start: None,
            ram_mib: system_memory_mib(),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
            managed: false,
            over: None,
            gpu: gpu_memory::Adapter::default(),
            gpu_details: false,
            cache_details: false,
            gpu_memory_before_drag: None,
            scroll: 0.0,
            cache_free: Vec::new(),
            cache_failures: yolu_core::tile_cache::read_failures(),
        }
    }
}

impl PrefsState {
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
        self.color.wheel = settings.color_wheel;
        self.view3d.display.post = settings.view3d_post;
        self.view3d.projection = settings.view3d_paint;
        // 「すべて残す」を外したときに戻る数も、保存してあった数にする
        if let BackupKeep::Count(n) = settings.backups {
            self.prefs.remembered_backups = n;
        }
        self.prefs.settings = settings;
        self.prefs.managed = true;
    }

    pub fn prefs_apply(&mut self, action: PrefsAction) {
        let lang = self.lang;
        match action {
            PrefsAction::Open => self.prefs.open = true,
            PrefsAction::Close => {
                self.prefs.open = false;
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

    /// 読み込み（.ylp を開く・書き出しが写した文書を戻す）で 1 つの文書に許す層の画素のバイト数。
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

fn padding_name(lang: Lang, texels: i32) -> String {
    match texels {
        0 => lang.pick("なし", "Off").into(),
        n if n < 0 => lang.pick("届くかぎり", "Fill (all the way)").into(),
        n => lang.pick(format!("{n} テクセル"), format!("{n} texels")),
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
        PrefChoice::OrbitCenter => OrbitCenter::ALL
            .into_iter()
            .map(|c| {
                Entry::item(c.label(lang), set(Pref::OrbitCenter(c)))
                    .radio(s.navigation.orbit == c)
                    .tooltip(c.tip(lang))
            })
            .collect(),
        PrefChoice::ZoomCenter => ZoomCenter::ALL
            .into_iter()
            .map(|c| {
                Entry::item(c.label(lang), set(Pref::ZoomCenter(c)))
                    .radio(s.navigation.zoom == c)
                    .tooltip(c.tip(lang))
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

// ───────── 窓 ─────────

/// 窓が窓の中に出す 1 行の要求（描いたあとで当てる）。
enum Request {
    Open(PrefChoice, Rect),
    Do(PrefsAction),
}

/// 節の見出しの行の高さ（最初の節以外は、上に細い線）。
const HEADING: f32 = 24.0;

/// 窓の中身（見出しの帯の下）の高さの見積もり。描く行の数と合わせる（試験が、実際に並べた高さと同じであることを確かめる）。
/// 画面に収まらなければ、窓は画面の高さにして、中身は共通のスクロールで送る。`external_ops` は「外からの操作を受ける」が入っているか
/// （入っている間だけ、その下にポート番号の行を出す）。
fn content_height(gpu_details: bool, cache_details: bool, external_ops: bool) -> f32 {
    let dropdown = t::ROW_HEIGHT + GAP;
    let slider = t::SLIDER_ROW_HEIGHT + GAP;
    8.0 + HEADING * 5.0 // 節の見出し: 一般・メモリ・処理・3D ビュー・ファイル
        + dropdown * 5.0 // 一般: 言語・Live Link の起動・受けた値の保存・外からの操作・書き出しの余白
        + if external_ops { dropdown } else { 0.0 } // 外からの操作のポート番号
        + dropdown * 3.0 + slider // メモリ: 予算 3 つ・最小の取り消し段数
        + dropdown * 2.0 // メモリ: ディスクキャッシュ・詳しく
        + if cache_details { dropdown * 3.0 } else { 0.0 } // キャッシュの上限・置き場所（パスとボタン）
        + dropdown * 3.0 + dropdown // 処理: スレッド・合成・GPU のメモリ・詳しく
        + if gpu_details { slider } else { 0.0 } // GPU のメモリの合計
        + dropdown * 3.0 // 3D ビュー: 回転の中心・ズームの中心・UV ワイヤーフレーム
        + dropdown * 2.0 // ファイル: ライブラリの場所（パスとボタン）
        + slider + dropdown // 退避を残す数・すべて残す
        + 8.0
}

fn window_height(gpu_details: bool, cache_details: bool, external_ops: bool) -> f32 {
    window::HEADER_HEIGHT + content_height(gpu_details, cache_details, external_ops)
}

/// 最後に描いた中身の高さ（見出しの帯の下。画面の点。開いていなければ None）。試験が、見積もりと実際の並びの食い違いを見つける。
pub fn drawn_content_height(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|d| d.get_temp(window_id().with("content")))
}

/// フォルダの 2 行（ラベルとパス、その下に「選ぶ…」「既定に戻す」）。押したボタン（選ぶ・既定に戻す）を返す。
#[allow(clippy::too_many_arguments)]
fn folder_rows(
    ui: &mut egui::Ui,
    rows: &mut w::Rows,
    lang: Lang,
    key: &str,
    label: &str,
    shown: &str,
    choose_tip: &str,
    default_tip: &str,
    can_default: bool,
    enabled: bool,
) -> (bool, bool) {
    let row = rows.row(t::ROW_HEIGHT, GAP);
    let p = ui.painter().clone();
    w::text(
        &p,
        Rect::from_min_size(row.min, vec2(LABEL_WIDTH, row.height())),
        label,
        t::LABEL,
        Align::Left,
    );
    let path_rect = Rect::from_min_max(pos2(row.left() + LABEL_WIDTH, row.top()), row.max);
    w::rounded(&p, path_rect, t::CONTROL_BG, 3.0);
    w::outline(&p, path_rect, t::BORDER, 1.0, 3.0);
    let fitted = w::fit(&p, shown, path_rect.width() - 14.0, t::LABEL_DIM);
    w::text(
        &p,
        Rect::from_min_max(pos2(path_rect.left() + 7.0, path_rect.top()), path_rect.max),
        &fitted,
        t::LABEL_DIM,
        Align::Left,
    );
    ui.interact(
        path_rect,
        window_id().with((key, "path")),
        egui::Sense::hover(),
    )
    .on_hover_text(shown);
    let row = rows.row(t::ROW_HEIGHT, GAP);
    let (choose, default) = (
        lang.pick("選ぶ…", "Choose…"),
        lang.pick("既定に戻す", "Default"),
    );
    let widths = [choose, default].map(|s| w::text_width(&p, s, t::LABEL) + 24.0);
    let right = row.right();
    let default_rect = Rect::from_min_size(
        pos2(right - widths[1], row.top()),
        vec2(widths[1], row.height()),
    );
    let choose_rect = Rect::from_min_size(
        pos2(default_rect.left() - GAP - widths[0], row.top()),
        vec2(widths[0], row.height()),
    );
    let chose = w::button(
        ui,
        choose_rect,
        ("prefs", format!("{key}-choose")),
        choose,
        false,
        enabled,
        Some(choose_tip),
        None,
    )
    .clicked();
    let reset = w::button(
        ui,
        default_rect,
        ("prefs", format!("{key}-default")),
        default,
        false,
        enabled && can_default,
        Some(default_tip),
        None,
    )
    .clicked();
    (chose, reset)
}

/// 開いていれば窓を描き、選んだ値を `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.prefs.open {
        app.prefs.dragging = false;
        app.prefs.gpu_memory_before_drag = None;
        return;
    }
    let lang = app.lang;
    let spec = Spec {
        title: lang.pick("設定", "Settings"),
        icon: Some("tune"),
        size: vec2(
            WIDTH,
            window_height(
                app.prefs.gpu_details,
                app.prefs.cache_details,
                app.prefs.settings.external_ops,
            ),
        ),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let id = window_id();
    let mut offset = app.prefs.offset;
    let mut requests: Vec<Request> = Vec::new();
    let enabled = true;
    let s = app.prefs.settings.clone();
    let ram = app.prefs.ram_mib;
    let cores = app.prefs.cores;
    let restart = s.cpu_threads != app.prefs.threads_at_start;
    let library = s.library_folder();
    let cache_free = app.cache_free_now();
    let keep_all = s.backups == BackupKeep::All;
    let backup_count = app.prefs.shown_backups();
    let (gpu_details, gpu_total) = (app.prefs.gpu_details, app.gpu_total_mib());
    let cache_details = app.prefs.cache_details;
    let mut dragging = false;
    let mut gpu_dragging = false;
    let mut scroll = app.prefs.scroll;
    let content_id = id.with("content");
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        // 画面が低くて収まらないときは、中身を共通のスクロールで送る（中身の高さは前のフレームのもの。初めは見積もり）
        let body = frame.body;
        let previous = ctx
            .data(|d| d.get_temp::<f32>(content_id))
            .unwrap_or_else(|| content_height(gpu_details, cache_details, s.external_ops));
        let bar = Scroll::begin(ui, body, previous, &mut scroll);
        let area = Rect::from_min_max(
            pos2(body.left(), body.top() - scroll),
            pos2(body.right() - bar.reserved(), body.bottom()),
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(body.intersect(outer_clip));
        let mut rows = w::Rows::new(area, 8.0);
        let choice = |ui: &mut egui::Ui,
                      rows: &mut w::Rows,
                      key: &str,
                      label: &str,
                      value: &str,
                      tip: &str,
                      which: PrefChoice| {
            let r = rows.row(t::ROW_HEIGHT, GAP);
            let (response, b) = w::dropdown(
                ui,
                r,
                ("prefs", key),
                Some(label),
                value,
                Some(tip),
                enabled,
                LABEL_WIDTH,
            );
            response.clicked().then_some(Request::Open(which, b))
        };
        // 節の見出し（2 つ目からは上に細い線）
        let section = |ui: &mut egui::Ui, rows: &mut w::Rows, first: bool, title: &str| {
            let r = rows.row(HEADING, 0.0);
            if !first {
                w::hline(
                    ui.painter(),
                    r.left(),
                    r.right(),
                    r.top() + 3.0,
                    t::SEPARATOR,
                );
            }
            w::text(
                ui.painter(),
                Rect::from_min_max(pos2(r.left(), r.top() + 6.0), r.max),
                title,
                t::LABEL_BOLD.with_color(t::TEXT_DIM),
                Align::Left,
            );
        };

        section(ui, &mut rows, true, lang.pick("一般", "General"));
        requests.extend(choice(
            ui,
            &mut rows,
            "language",
            lang.pick("言語", "Language"),
            lang.name(),
            lang.pick("画面の言語", "The language of the screen"),
            PrefChoice::Language,
        ));
        let next = w::toggle(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            id.with("livelink-on-startup"),
            lang.pick("Unity の Live Link を受け付ける", "Accept Live Link from Unity"),
            s.livelink_on_startup,
            Some(lang.pick(
                "Unity のエディタの「YoluPainter で開く」で送ったモデルとマテリアルを開く。--livelink を付けて起動すると、この設定によらず受け付ける",
                "Opens the model and materials sent with Open in YoluPainter in the Unity Editor. Launching with --livelink always accepts them",
            )),
            enabled,
        );
        if next != s.livelink_on_startup {
            requests.push(Request::Do(PrefsAction::Set(Pref::LiveLinkOnStartup(next))));
        }
        let next = w::toggle(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            id.with("livelink-keep-values"),
            lang.pick("Unity から受けたマテリアルの値を保存する", "Save material values received from Unity"),
            s.livelink_keep_values,
            Some(lang.pick(
                "Live Link で受けた lilToon の値を .ylp に入れる（テクスチャの画素は入れず、開き直すとファイルから読む）。切ると、次に保存するときに外す",
                "Stores the lilToon values received through Live Link in the .ylp (not the pixels of textures; they are read from their files on reopening). When off, they are removed on the next save",
            )),
            enabled,
        );
        if next != s.livelink_keep_values {
            requests.push(Request::Do(PrefsAction::Set(Pref::LiveLinkKeepValues(
                next,
            ))));
        }
        let ops_url = yolu_mcp::endpoint(s.external_ops_port);
        let ops_tip = lang.pick(
            format!("AI のアシスタント（MCP）やコマンドラインからの操作を {ops_url} で受ける。入れている間だけ待ち、切るとつながりも閉じる。合言葉は無いので、この PC のほかのアカウントのプログラムもつなげる"),
            format!("Accepts commands from AI assistants (MCP) and the command line at {ops_url}. It listens only while on; turning it off also closes the connections. There is no password, so programs of other accounts on this PC can connect too"),
        );
        let next = w::toggle(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            id.with("external-ops"),
            crate::settings::setting_name(lang, "external_ops"),
            s.external_ops,
            Some(&ops_tip),
            enabled,
        );
        if next != s.external_ops {
            requests.push(Request::Do(PrefsAction::Set(Pref::ExternalOps(next))));
        }
        // 外からの操作を待つポート番号（入れている間だけ）: 名前と打つ欄（Enter か外を押して決める。Esc でやめる）
        if s.external_ops {
            let row = rows.row(t::ROW_HEIGHT, GAP);
            w::text(
                ui.painter(),
                Rect::from_min_size(row.min, vec2(LABEL_WIDTH, row.height())),
                crate::settings::setting_name(lang, "external_ops_port"),
                t::LABEL,
                Align::Left,
            );
            let field = Rect::from_min_max(pos2(row.left() + LABEL_WIDTH, row.top()), row.max);
            let typed = w::text_field(
                ui,
                field,
                ("prefs", "external-ops-port"),
                &s.external_ops_port.to_string(),
                Some(lang.pick(
                    "外からの操作を待つ番号（1024〜65535）。変えたら、つなぐ側の設定の番号も同じにする",
                    "The port external commands are accepted on (1024 to 65535). When you change it, use the same port in the programs that connect",
                )),
                false,
            );
            if let Some(text) = typed.committed {
                // 番号でない・範囲の外は 0 として渡し、断る理由を出す
                let port = text.trim().parse::<u16>().unwrap_or(0);
                requests.push(Request::Do(PrefsAction::Set(Pref::ExternalOpsPort(port))));
            }
        }
        requests.extend(choice(
            ui,
            &mut rows,
            "padding",
            lang.pick("書き出しの余白", "Export padding"),
            &padding_name(lang, s.export_padding),
            lang.pick(
                "書き出しで、UV が触れないテクセルへ UV の縁の色を塗り広げる量。ミップマップや補間で縁に別の色が混ざるのを防ぎます",
                "How far the colors at the UV edges are spread into texels no UV touches when exporting, so mipmaps and filtering do not pull in other colors",
            ),
            PrefChoice::ExportPadding,
        ));
        section(ui, &mut rows, false, lang.pick("メモリ", "Memory"));
        let budget_tips = [
            lang.pick(
                "取り消しの履歴に使うメモリの、全テクスチャセットの合計。超えた古い段から捨てます（最小の段数は残します）。0 は最小の段数だけ残します",
                "Memory for the undo history, in total over all texture sets. The oldest steps beyond it are dropped (the minimum steps are kept). 0 keeps only those",
            ),
            lang.pick(
                "全テクスチャセットの全レイヤーが画素に使うメモリの合計の上限。使う分だけ取り、先には確保しません。PSD の読み込み・書き出しや .ylp を開くときの大きさの上限もこれで決まります。超える操作は何も変えずに断ります",
                "The most memory all layers in all texture sets may use for pixels, in total. Only what is used is taken, nothing is reserved up front. It also sets the size limit when importing or exporting a PSD or opening a .ylp. An edit that would exceed it is refused without changing anything",
            ),
            lang.pick(
                "ストロークや塗りつぶし 1 回が巻き戻し用に持てるメモリ。書き出しの作業にも使います。超える操作は何も変えずに止めます",
                "Undo data one stroke or fill may keep; also the working memory of exports. A bigger one is stopped without changing anything",
            ),
        ];
        for (kind, tip) in BudgetKind::ALL.into_iter().zip(budget_tips) {
            requests.extend(choice(
                ui,
                &mut rows,
                kind.key(),
                crate::settings::setting_name(lang, kind.key()),
                &budget_name(lang, kind, s.budget(kind), ram),
                tip,
                PrefChoice::Budget(kind),
            ));
        }
        let out = w::slider(
            ui,
            rows.row(t::SLIDER_ROW_HEIGHT, GAP),
            ("prefs", "min-undo-steps"),
            s.min_undo_steps as f32,
            &SliderSpec::new(
                crate::settings::setting_name(lang, "min_undo_steps"),
                0.0,
                MAX_MIN_UNDO_STEPS as f32,
                NumberFormat::int(""),
            )
            .tooltip(lang.pick(
                "取り消しの予算を超えても残す直近の段数。大きな塗りつぶしや変形も取り消せるようにします。0 は予算を厳密にします",
                "The newest steps kept even beyond the undo budget, so a large fill can still be undone. 0 makes the budget strict",
            )),
        );
        if out.changed {
            requests.push(Request::Do(PrefsAction::Set(Pref::MinUndoSteps(
                out.value.round().clamp(0.0, MAX_MIN_UNDO_STEPS as f32) as u32,
            ))));
        }
        let next = w::toggle(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            id.with("disk-cache"),
            crate::settings::setting_name(lang, "disk_cache"),
            s.disk_cache,
            Some(lang.pick(
                "レイヤーのメモリと取り消し履歴の予算を超えた分のタイルを、使っていないものからディスクへ移して続けます。切ると、予算を超える操作は断ります",
                "Moves the tiles beyond the layer memory and undo history budgets to the disk, least recently used first, so work can go on. When off, edits beyond the budgets are refused",
            )),
            enabled,
        );
        if next != s.disk_cache {
            requests.push(Request::Do(PrefsAction::Set(Pref::DiskCache(next))));
        }
        let open = w::subsection_header(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            ("prefs", "cache-details"),
            lang.pick("詳しく", "Details"),
            cache_details,
        );
        if open != cache_details {
            requests.push(Request::Do(PrefsAction::CacheDetails(open)));
        }
        if cache_details {
            let r = rows.row(t::ROW_HEIGHT, GAP);
            let (response, b) = w::dropdown(
                ui,
                r,
                ("prefs", "disk-cache-limit"),
                Some(crate::settings::setting_name(lang, "disk_cache_limit_gib")),
                &disk_limit_name(lang, s.disk_cache_limit, cache_free),
                Some(lang.pick(
                    "ディスクキャッシュに使う量の上限。自動は 64 GiB と、置き場所の空きの半分の小さい方。満杯なら、超える操作は断ります",
                    "The most disk space the cache may use. Automatic is 64 GiB or half the free space of the folder, whichever is smaller. When it is full, edits beyond it are refused",
                )),
                enabled && s.disk_cache,
                LABEL_WIDTH,
            );
            if response.clicked() {
                requests.push(Request::Open(PrefChoice::DiskCacheLimit, b));
            }
            let shown = s.disk_cache_folder().display().to_string();
            let (choose, reset) = folder_rows(
                ui,
                &mut rows,
                lang,
                "cache",
                crate::settings::setting_name(lang, "disk_cache_folder"),
                &shown,
                lang.pick(
                    "キャッシュのファイルを置くフォルダを選ぶ。速いドライブ（SSD）ほど、移したタイルを戻すのが速い",
                    "Choose the folder for the cache file. A faster drive (SSD) brings moved tiles back faster",
                ),
                lang.pick("OS の一時フォルダに戻す", "Back to the system temporary folder"),
                s.disk_cache_folder.is_some(),
                enabled && s.disk_cache,
            );
            if choose {
                requests.push(Request::Do(PrefsAction::ChooseCacheFolder));
            }
            if reset {
                requests.push(Request::Do(PrefsAction::Set(Pref::DiskCacheFolder(None))));
            }
        }
        section(ui, &mut rows, false, lang.pick("処理", "Processing"));
        let mut threads = threads_name(lang, s.cpu_threads, cores);
        if restart {
            threads += &format!(
                "{}{}",
                lang.pick(" ・ ", " · "),
                lang.pick("再起動で反映", "applies after restart")
            );
        }
        requests.extend(choice(
            ui,
            &mut rows,
            "threads",
            lang.pick("CPU のスレッド", "CPU threads"),
            &threads,
            lang.pick(
                "CPU の処理（合成・ブラシ・塗りつぶし・選択範囲など）が同時に使う数。どの値でも結果の画素は同じで、少ないと大きなキャンバスで遅くなる代わりにほかのアプリに余裕が残ります。次の起動から効きます",
                "The most threads the CPU work (compositing, brushes, fills, selections) uses at once. Any value gives the same pixels; fewer are slower on large canvases but leave cores to other programs. Takes effect from the next start",
            ),
            PrefChoice::CpuThreads,
        ));
        requests.extend(choice(
            ui,
            &mut rows,
            "compositing",
            lang.pick("表示の合成", "Display compositing"),
            compositing_name(lang, s.compositing),
            lang.pick(
                "2D のキャンバスに出すレイヤーの合成をどこで行うか。保存・書き出し・3D ビューの値の合成は、どれでも CPU です",
                "Where the layers shown on the 2D canvas are composited. Saving, exporting and the 3D view always composite on the CPU",
            ),
            PrefChoice::Compositing,
        ));
        requests.extend(choice(
            ui,
            &mut rows,
            "gpu-memory",
            crate::settings::setting_name(lang, "gpu_memory"),
            s.gpu_memory.name(lang),
            lang.pick(
                "3D ビュー・キャンバスの GPU の合成・アセットのサムネイルが使ってよい GPU のメモリの量。足りないと、3D ビューはほかのテクスチャセットの絵を減らし、今のセットの絵を小さくして見せます（テクスチャと書き出しは変わりません）。自動は GPU のメモリの量が分かるときだけ、それに合わせます（少なければ低に、多ければ標準の量を増やします）。分からないときは標準です",
                "How much GPU memory the 3D view, the canvas compositing and the asset previews may use. When it runs short, the 3D view drops the other sets' pictures and shows the current one smaller (the texture and exports are unchanged). Automatic follows the GPU's memory only when it is known (Low when there is little, a larger Standard when there is plenty); otherwise it is Standard",
            ),
            PrefChoice::GpuMemory,
        ));
        let open = w::subsection_header(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            ("prefs", "gpu-details"),
            lang.pick("詳しく", "Details"),
            gpu_details,
        );
        if open != gpu_details {
            requests.push(Request::Do(PrefsAction::GpuDetails(open)));
        }
        if gpu_details {
            let out = w::slider(
                ui,
                rows.row(t::SLIDER_ROW_HEIGHT, GAP),
                ("prefs", "gpu-total"),
                gpu_total as f32,
                &SliderSpec::new(
                    lang.pick("合計", "Total"),
                    gpu_memory::MIN_TOTAL_MIB as f32,
                    gpu_memory::MAX_TOTAL_MIB as f32,
                    NumberFormat::int(" MiB"),
                )
                .tooltip(lang.pick(
                    "GPU のメモリの合計。3D の絵・キャンバスの合成・アセットのサムネイルへ 4 : 4 : 1 に配ります。動かすと、段の選びを置き換えた量の指定になります",
                    "The total GPU memory, split 4 : 4 : 1 between the 3D pictures, the canvas compositing and the asset previews. Moving it replaces the level with a custom amount",
                )),
            );
            gpu_dragging = out.active;
            // 押し始めの選びを覚える（この枠の `s` は、押した枠の変更を入れる前の設定）
            if out.active && app.prefs.gpu_memory_before_drag.is_none() {
                app.prefs.gpu_memory_before_drag = Some(s.gpu_memory);
            }
            if out.changed {
                if !out.active && !out.released {
                    // Esc でドラッグを止めた（スライダーは押し始めの量を返す）。量ではなく、押す前の選びへ戻す
                    if let Some(before) = app.prefs.gpu_memory_before_drag {
                        requests.push(Request::Do(PrefsAction::Set(Pref::GpuMemory(before))));
                    }
                } else {
                    let step = gpu_memory::TOTAL_STEP_MIB as f32;
                    let mib = ((out.value / step).round() * step) as u32;
                    requests.push(Request::Do(PrefsAction::Set(Pref::GpuMemory(
                        GpuMemory::Mib(mib),
                    ))));
                }
            }
            if !out.active {
                app.prefs.gpu_memory_before_drag = None;
            }
        }
        section(ui, &mut rows, false, lang.pick("3D ビュー", "3D View"));
        requests.extend(choice(
            ui,
            &mut rows,
            "orbit-center",
            lang.pick("回転の中心", "Orbit center"),
            s.navigation.orbit.label(lang),
            lang.pick(
                "3D ビューを回すときの中心。3D ビューの表示の設定の「視点」と同じ値です",
                "The pivot when orbiting the 3D view. The same value as Navigation in the 3D view's display settings",
            ),
            PrefChoice::OrbitCenter,
        ));
        requests.extend(choice(
            ui,
            &mut rows,
            "zoom-center",
            lang.pick("ズームの中心", "Zoom center"),
            s.navigation.zoom.label(lang),
            lang.pick(
                "3D ビューをズームするときの中心。3D ビューの表示の設定の「視点」と同じ値です",
                "The center when zooming the 3D view. The same value as Navigation in the 3D view's display settings",
            ),
            PrefChoice::ZoomCenter,
        ));
        dragging |= crate::uv_wireframe::settings_row(ui, &mut rows, app);
        section(ui, &mut rows, false, lang.pick("ファイル", "Files"));
        // 棚の場所: ラベルとパス、その下にボタン
        let shown = library
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let (choose, reset) = folder_rows(
            ui,
            &mut rows,
            lang,
            "library",
            lang.pick("ライブラリの場所", "Library folder"),
            &shown,
            lang.pick(
                "ライブラリの場所のフォルダを選ぶ",
                "Choose the library folder",
            ),
            lang.pick("既定の場所に戻す", "Back to the default folder"),
            s.library_folder.is_some(),
            enabled,
        );
        if choose {
            requests.push(Request::Do(PrefsAction::ChooseLibraryFolder));
        }
        if reset {
            requests.push(Request::Do(PrefsAction::Set(Pref::LibraryFolder(None))));
        }
        // 退避を残す数: スライダー（すべて残す間は動かせない）と「すべて残す」
        let out = w::slider(
            ui,
            rows.row(t::SLIDER_ROW_HEIGHT, GAP),
            ("prefs", "backups"),
            backup_count as f32,
            &SliderSpec::new(
                lang.pick("退避を残す数", "Backups to keep"),
                0.0,
                MAX_BACKUPS_TO_KEEP as f32,
                NumberFormat::int(""),
            )
            .tooltip(lang.pick(
                "上書き保存で置き換えた前の版を、新しい順にいくつ残すか。超えた古い版は消します。0 は残しません",
                "How many replaced versions to keep per file, newest first. Older ones beyond this are deleted. 0 keeps none",
            ))
            .enabled(!keep_all),
        );
        dragging |= out.active;
        if out.changed {
            let n = out.value.round().clamp(0.0, MAX_BACKUPS_TO_KEEP as f32) as u32;
            requests.push(Request::Do(PrefsAction::SetBackups(BackupKeep::Count(n))));
        }
        let next = w::toggle(
            ui,
            rows.row(t::ROW_HEIGHT, GAP),
            id.with("backups-all"),
            lang.pick("すべて残す", "Keep all"),
            keep_all,
            Some(lang.pick("退避を消さない", "Never delete backups")),
            enabled,
        );
        if next != keep_all {
            let keep = if next {
                BackupKeep::All
            } else {
                BackupKeep::Count(backup_count)
            };
            requests.push(Request::Do(PrefsAction::SetBackups(keep)));
        }
        rows.space(8.0);
        ctx.data_mut(|d| d.insert_temp(content_id, rows.used()));
        ui.set_clip_rect(outer_clip);
        bar.end(ui, id.with("scroll"), &mut scroll);
    });
    app.prefs.scroll = scroll;
    app.prefs.offset = offset;
    app.prefs.dragging = dragging || gpu_dragging;
    for request in requests {
        match request {
            Request::Open(which, anchor) => {
                app.popup = Some(OpenPopup {
                    kind: PopupKind::M2(crate::m2_menu::Popup::Pref(which)),
                    state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
                });
            }
            Request::Do(action) => app.apply(Action::Prefs(action)),
        }
    }
    if closed {
        app.apply(Action::Prefs(PrefsAction::Close));
    }
}
