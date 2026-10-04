//! 設定の窓（表示 → 設定…）: 言語・書き出しの余白・メモリの予算（取り消し履歴・レイヤーの画素・1 回の操作・最小の取り消し段数）・
//! CPU のスレッド・表示の合成・棚の場所・退避を残す数。値は `AppState::prefs` に入り、設定のファイル（`settings`）へは次のフレームで書かれる。
//! 窓は浮いた窓の骨組み（`ui::window`）で、見出しをドラッグして動かせる。選択肢はポップアップ（`m2_menu::Popup::Pref`）。
//!
//! - **メモリの予算**（取り消し履歴・レイヤーの画素）は**プロジェクト全体**の上限で、全テクスチャセットの合計が設定を超えない
//!   （`sync_budgets`）。描けるのは今のセットだけなので、今のセットの文書に「設定 − ほかのセットが使っている量（画素・履歴）」を入れ、
//!   セットを切り替える・開くときに入れ直す。自動は物理メモリから（`settings::BudgetKind`）。描いている間は入れず、終わったフレームで
//!   入れる。1 回の操作・最小の取り消し段数はセットごとの値のまま（1 回の操作は同時に 1 つしか走らない）。今の画素がすでに予算を
//!   超えているときは画素を捨てず、予算をその量まで広げて知らせる（それ以上は足せない）。
//! - **CPU のスレッド**は rayon の全体のスレッドプールで、起動のときに決まる（`settings::apply_thread_setting`）。変えた値は次の起動から
//!   効くので、窓に「再起動で反映」と出す。
//! - **表示の合成**は 2D のキャンバスの表示の方針（`YoluApp::apply_compositing` がキャンバスの表示に入れる。自動は環境変数
//!   `YOLUPAINTER_CANVAS` か自動）。保存・書き出し・3D ビューの値の合成は、どれでも CPU が正本。

use std::path::PathBuf;

use egui::{pos2, vec2, Id, Rect, Vec2};
use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::lang::Lang;
use crate::m2::UiOp;
use crate::settings::{
    system_memory_mib, Budget, BudgetKind, Compositing, Settings, EXPORT_PADDINGS, MAX_CPU_THREADS,
    MAX_MIN_UNDO_STEPS,
};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 400.0;
/// ラベルの幅（値の箱はその右）。
const LABEL_WIDTH: f32 = 150.0;
const GAP: f32 = 4.0;
const SEPARATOR: f32 = 12.0;
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
    Budget(BudgetKind, Budget),
    MinUndoSteps(u32),
    /// None は自動。
    CpuThreads(Option<u32>),
    Compositing(Compositing),
    /// None は既定。
    LibraryFolder(Option<PathBuf>),
}

/// 選択肢のポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefChoice {
    Language,
    ExportPadding,
    Budget(BudgetKind),
    CpuThreads,
    Compositing,
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
            ..self.prefs.settings.clone()
        }
    }

    /// 起動のとき、読んだ設定を入れる（言語は作るときに決めてある）。予算は次の `sync_budgets` で文書へ。
    pub fn load_settings(&mut self, settings: Settings) {
        self.export.padding = settings.export_padding;
        self.prefs.threads_at_start = settings.cpu_threads;
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
            PrefsAction::ChooseLibraryFolder => self.dialog_request = Some(DialogRequest::PrefsLibraryFolder),
            PrefsAction::Set(pref) => match pref {
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
                Pref::LibraryFolder(folder) => match folder {
                    Some(path) if !path.is_absolute() => {
                        self.message = lang
                            .pick("棚の場所は絶対パスで指定します。", "The library folder must be an absolute path.")
                            .into();
                    }
                    folder => self.prefs.settings.library_folder = folder,
                },
            },
        }
    }

    /// 選んだ予算を今のセットの文書に入れる（全体の予算から、ほかのセットが使っている量を引く）。入れる値が今と同じなら何もしない
    /// ので、毎フレーム呼べる。描いている間は入れず、終わった後のフレームで入れる。今の画素が（ほかのセットの分を引いた）予算を
    /// 超えていれば、画素は捨てずに予算をその量まで広げ、同じ状況では 1 度だけ知らせる。
    pub fn sync_budgets(&mut self) {
        if !self.prefs.managed || self.is_stroking() {
            return;
        }
        let wanted = self.prefs.settings.budgets(self.prefs.ram_mib);
        let current = self.sets.current_index();
        let (mut other_source, mut other_history) = (0u64, 0u64);
        for i in (0..self.sets.len()).filter(|&i| i != current) {
            let doc = self.set_doc(i);
            other_source = other_source.saturating_add(doc.allocated_bytes());
            other_history = other_history.saturating_add(doc.history_bytes());
        }
        let undo = wanted.undo.saturating_sub(other_history);
        let source = wanted.source.saturating_sub(other_source);
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
            self.message = lang
                .pick(
                    "レイヤーの画素がすでに予算を超えているので、予算を上げるまで足せません。",
                    "The layer pixels are already over the budget; nothing can be added until it is raised.",
                )
                .into();
        }
    }

    /// 読み込み（.ylp を開く・書き出しが写した文書を戻す）で 1 つの文書に許す層の画素のバイト数。
    pub fn load_source_bytes(&self) -> u64 {
        self.prefs.settings.load_source_bytes(self.prefs.ram_mib)
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

fn threads_name(lang: Lang, threads: Option<u32>, cores: u32) -> String {
    match threads {
        None => lang.pick(format!("自動（{cores}）"), format!("Automatic ({cores})")),
        Some(1) => lang.pick("1（並列にしない）", "1 (no parallel work)").into(),
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
            .map(|p| Entry::item(padding_name(lang, p), set(Pref::ExportPadding(p))).radio(s.export_padding == p))
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
                    Entry::item(budget_name(lang, kind, b, ram), set(Pref::Budget(kind, b))).radio(s.budget(kind) == b)
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
                    Entry::item(threads_name(lang, n, app.prefs.cores), set(Pref::CpuThreads(n))).radio(s.cpu_threads == n)
                })
                .collect()
        }
        PrefChoice::Compositing => Compositing::ALL
            .into_iter()
            .map(|c| Entry::item(compositing_name(lang, c), set(Pref::Compositing(c))).radio(s.compositing == c))
            .collect(),
    }
}

// ───────── 窓 ─────────

/// 窓が窓の中に出す 1 行の要求（描いたあとで当てる）。
enum Request {
    Open(PrefChoice, Rect),
    Do(PrefsAction),
}

/// 窓の高さ（見出し・行・区切り）。
fn window_height() -> f32 {
    let dropdown = t::ROW_HEIGHT + GAP;
    window::HEADER_HEIGHT
        + 8.0
        + dropdown * 2.0 // 言語・書き出しの余白
        + SEPARATOR
        + dropdown * 3.0 // 予算
        + t::SLIDER_ROW_HEIGHT
        + GAP // 最小の取り消し段数
        + SEPARATOR
        + dropdown * 2.0 // スレッド・合成
        + SEPARATOR
        + dropdown * 2.0 // 棚の場所（パスとボタン）
        + SEPARATOR
        + t::SLIDER_ROW_HEIGHT
        + GAP // 退避を残す数
        + dropdown // すべて残す
        + dropdown // UV ワイヤーフレーム
        + 8.0
}

/// 開いていれば窓を描き、選んだ値を `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.prefs.open {
        app.prefs.dragging = false;
        return;
    }
    let lang = app.lang;
    let spec = Spec {
        title: lang.pick("設定", "Settings"),
        icon: Some("tune"),
        size: vec2(WIDTH, window_height()),
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
    let keep_all = s.backups == BackupKeep::All;
    let backup_count = app.prefs.shown_backups();
    let mut dragging = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        let mut rows = w::Rows::new(frame.body, 8.0);
        let choice = |ui: &mut egui::Ui, rows: &mut w::Rows, key: &str, label: &str, value: &str, tip: &str, which: PrefChoice| {
            let r = rows.row(t::ROW_HEIGHT, GAP);
            let (response, b) = w::dropdown(ui, r, ("prefs", key), Some(label), value, Some(tip), enabled, LABEL_WIDTH);
            response.clicked().then_some(Request::Open(which, b))
        };
        let separator = |ui: &mut egui::Ui, rows: &mut w::Rows| {
            let r = rows.row(SEPARATOR, 0.0);
            w::hline(ui.painter(), r.left(), r.right(), r.center().y, t::SEPARATOR);
        };

        requests.extend(choice(
            ui,
            &mut rows,
            "language",
            lang.pick("言語", "Language"),
            lang.name(),
            lang.pick("画面の言語", "The language of the screen"),
            PrefChoice::Language,
        ));
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
        separator(ui, &mut rows);
        let budget_tips = [
            lang.pick(
                "取り消しの履歴に使うメモリの、全テクスチャセットの合計。超えた古い段から捨てます（最小の段数は残します）。0 は最小の段数だけ残します",
                "Memory for the undo history, in total over all texture sets. The oldest steps beyond it are dropped (the minimum steps are kept). 0 keeps only those",
            ),
            lang.pick(
                "全テクスチャセットの全レイヤーの画素の合計。超える操作は何も変えずに断ります",
                "Total pixel data of all layers in all texture sets. An edit that would exceed it is refused without changing anything",
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
        separator(ui, &mut rows);
        let mut threads = threads_name(lang, s.cpu_threads, cores);
        if restart {
            threads += &format!("{}{}", lang.pick(" ・ ", " · "), lang.pick("再起動で反映", "applies after restart"));
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
        separator(ui, &mut rows);
        // 棚の場所: ラベルとパス、その下にボタン
        let row = rows.row(t::ROW_HEIGHT, GAP);
        let p = ui.painter().clone();
        w::text(
            &p,
            Rect::from_min_size(row.min, vec2(LABEL_WIDTH, row.height())),
            lang.pick("棚の場所", "Library folder"),
            t::LABEL,
            Align::Left,
        );
        let shown = library
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let path_rect = Rect::from_min_max(pos2(row.left() + LABEL_WIDTH, row.top()), row.max);
        w::rounded(&p, path_rect, t::CONTROL_BG, 3.0);
        w::outline(&p, path_rect, t::BORDER, 1.0, 3.0);
        let fitted = w::fit(&p, &shown, path_rect.width() - 14.0, t::LABEL_DIM);
        w::text(
            &p,
            Rect::from_min_max(pos2(path_rect.left() + 7.0, path_rect.top()), path_rect.max),
            &fitted,
            t::LABEL_DIM,
            Align::Left,
        );
        ui.interact(path_rect, id.with("library-path"), egui::Sense::hover()).on_hover_text(&shown);
        let row = rows.row(t::ROW_HEIGHT, GAP);
        let (choose, default) = (lang.pick("選ぶ…", "Choose…"), lang.pick("既定に戻す", "Default"));
        let widths = [choose, default].map(|s| w::text_width(&p, s, t::LABEL) + 24.0);
        let right = row.right();
        let default_rect = Rect::from_min_size(pos2(right - widths[1], row.top()), vec2(widths[1], row.height()));
        let choose_rect = Rect::from_min_size(
            pos2(default_rect.left() - GAP - widths[0], row.top()),
            vec2(widths[0], row.height()),
        );
        if w::button(
            ui,
            choose_rect,
            ("prefs", "library-choose"),
            choose,
            false,
            enabled,
            Some(lang.pick("棚の場所のフォルダを選ぶ", "Choose the library folder")),
            None,
        )
        .clicked()
        {
            requests.push(Request::Do(PrefsAction::ChooseLibraryFolder));
        }
        if w::button(
            ui,
            default_rect,
            ("prefs", "library-default"),
            default,
            false,
            enabled && s.library_folder.is_some(),
            Some(lang.pick("既定の場所に戻す", "Back to the default folder")),
            None,
        )
        .clicked()
        {
            requests.push(Request::Do(PrefsAction::Set(Pref::LibraryFolder(None))));
        }
        separator(ui, &mut rows);
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
        dragging = out.active;
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
        dragging |= crate::uv_wireframe::settings_row(ui, &mut rows, app);
        if next != keep_all {
            let keep = if next { BackupKeep::All } else { BackupKeep::Count(backup_count) };
            requests.push(Request::Do(PrefsAction::SetBackups(keep)));
        }
    });
    app.prefs.offset = offset;
    app.prefs.dragging = dragging;
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
