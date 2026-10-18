//! 設定の区分と項目の表（左の区分・上の探す欄の対象）と、項目ごとの行（右の欄に並べる）。
//!
//! 項目（`Item`）は、名前（今の言語の文字。探す欄が当てる）と、属する区分を持つ。行の描き方は `draw_item` の 1 か所で、区分の表示でも、
//! 探した結果の並びでも同じ行を使う（表示のたびに行を二重に持たない）。値の変更は `Request` に集めて、描き終えたあとで当てる。

use std::path::PathBuf;

use egui::{pos2, vec2, Id, Rect, Ui};
use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use super::{
    budget_name, compositing_name, disk_limit_name, padding_name, pen_api_name, threads_name, Pref,
    PrefChoice, PrefsAction,
};
use crate::gpu_memory::{self, GpuMemory};
use crate::lang::Lang;
use crate::settings::{BudgetKind, Settings, MAX_MIN_UNDO_STEPS};
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::update::UpdateAction;

/// ラベルの幅（値の箱はその右）。
const LABEL_WIDTH: f32 = 150.0;
const GAP: f32 = super::GAP;
/// 節の見出し（探した結果の区分の名前・「ペン」の筆圧の調整の見出し）の行の高さ。
const HEADING: f32 = 24.0;

/// 左の区分。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Category {
    /// 言語。
    #[default]
    General,
    /// ペンの入力・タブレットの筆圧・筆圧の調整。
    Pen,
    /// キー・マウス・パイメニューの割り当ての表（`shortcuts::editor`）。
    Shortcuts,
    /// 画面の出し方（表示の合成・垂直同期・GPU のメモリ）と、ベイクで RT コアを使うか。
    Display,
    /// メモリの予算（取り消し履歴・レイヤーのメモリ・1 回の操作・最小の取り消し段数）と、あふれた分を逃がすディスクキャッシュ。
    Memory,
    /// CPU の使い方（スレッド）。
    Processing,
    /// 3D ビューの視点の中心・UV ワイヤーフレーム。
    View3d,
    /// ファイル（書き出しのパディング・ライブラリの場所・退避を残す数）。
    Files,
    /// Live Link と外からの操作。
    LiveLink,
    /// 更新の確かめ方（公開鍵を組み込んだビルドだけ）。
    Updates,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::General,
        Category::Pen,
        Category::Shortcuts,
        Category::Display,
        Category::Memory,
        Category::Processing,
        Category::View3d,
        Category::Files,
        Category::LiveLink,
        Category::Updates,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Category::General => lang.t("prefs-general"),
            Category::Pen => lang.t("prefs-pen"),
            Category::Shortcuts => lang.t("prefs-keyboard-shortcuts"),
            Category::Display => lang.t("prefs-display"),
            Category::Memory => lang.t("prefs-memory"),
            Category::Processing => lang.t("prefs-processing"),
            Category::View3d => lang.t("prefs-3d-view"),
            Category::Files => lang.t("prefs-files"),
            Category::LiveLink => lang.t("prefs-live-link-commands"),
            Category::Updates => lang.t("prefs-updates"),
        }
    }

    /// この区分を左に出すか（更新は、公開鍵を組み込んだビルドだけ）。
    pub fn available(self, app: &AppState) -> bool {
        self != Category::Updates || app.services.update.enabled()
    }

    /// この区分に並べる項目（表示順。この PC・このビルドで出すものだけ）。ショートカットは表なので項目は無い。
    pub fn items(self, app: &AppState) -> Vec<Item> {
        let mut items = match self {
            Category::General => vec![Item::Language],
            Category::Pen => {
                let mut v = Vec::new();
                if app.prefs.pen_input_row {
                    v.push(Item::PenInput);
                }
                if app.prefs.tablet_row {
                    v.push(Item::TabletPressure);
                }
                v.push(Item::PressureAdjust);
                v
            }
            Category::Shortcuts => Vec::new(),
            Category::Display => vec![
                Item::Compositing,
                Item::Vsync,
                Item::GpuMemory,
                Item::BakeRayQuery,
            ],
            Category::Memory => {
                let mut v: Vec<Item> = BudgetKind::ALL.into_iter().map(Item::Budget).collect();
                v.push(Item::MinUndoSteps);
                v.push(Item::DiskCache);
                v
            }
            Category::Processing => vec![Item::CpuThreads],
            Category::View3d => vec![
                Item::OrbitCenter,
                Item::ZoomCenter,
                Item::AxisOrtho,
                Item::UvWireframe,
            ],
            Category::Files => vec![Item::ExportPadding, Item::Library, Item::Backups],
            Category::LiveLink => vec![Item::LiveLinkAccept, Item::LiveLinkKeep, Item::ExternalOps],
            Category::Updates => vec![Item::UpdateStartup, Item::UpdateBeta],
        };
        if !self.available(app) {
            items.clear();
        }
        items
    }
}

/// 設定の項目 1 つ（探す欄が名前で当てる単位）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Language,
    PenInput,
    TabletPressure,
    PressureAdjust,
    Compositing,
    Vsync,
    GpuMemory,
    BakeRayQuery,
    Budget(BudgetKind),
    MinUndoSteps,
    CpuThreads,
    OrbitCenter,
    ZoomCenter,
    AxisOrtho,
    UvWireframe,
    ExportPadding,
    Library,
    Backups,
    DiskCache,
    LiveLinkAccept,
    LiveLinkKeep,
    ExternalOps,
    UpdateStartup,
    UpdateBeta,
}

impl Item {
    /// 項目の全部（探す欄・区分の表・試験が全部を数える）。足したら `rank` の `match` も直す（コンパイルが教える）。
    pub const ALL: [Item; 26] = [
        Item::Language,
        Item::PenInput,
        Item::TabletPressure,
        Item::PressureAdjust,
        Item::Compositing,
        Item::Vsync,
        Item::GpuMemory,
        Item::Budget(BudgetKind::Undo),
        Item::Budget(BudgetKind::Source),
        Item::Budget(BudgetKind::Stroke),
        Item::MinUndoSteps,
        Item::DiskCache,
        Item::CpuThreads,
        Item::OrbitCenter,
        Item::ZoomCenter,
        Item::AxisOrtho,
        Item::UvWireframe,
        Item::ExportPadding,
        Item::Library,
        Item::Backups,
        Item::LiveLinkAccept,
        Item::LiveLinkKeep,
        Item::ExternalOps,
        Item::UpdateStartup,
        Item::UpdateBeta,
        Item::BakeRayQuery,
    ];

    /// `ALL` の中の番号。項目を足して `ALL` に入れ忘れたときに、試験が見つける（`match` が全部の項目を数えるので、足すとコンパイルが通らない）。
    pub fn rank(self) -> usize {
        match self {
            Item::Language => 0,
            Item::PenInput => 1,
            Item::TabletPressure => 2,
            Item::PressureAdjust => 3,
            Item::Compositing => 4,
            Item::Vsync => 5,
            Item::GpuMemory => 6,
            Item::Budget(BudgetKind::Undo) => 7,
            Item::Budget(BudgetKind::Source) => 8,
            Item::Budget(BudgetKind::Stroke) => 9,
            Item::MinUndoSteps => 10,
            Item::DiskCache => 11,
            Item::CpuThreads => 12,
            Item::OrbitCenter => 13,
            Item::ZoomCenter => 14,
            Item::AxisOrtho => 15,
            Item::UvWireframe => 16,
            Item::ExportPadding => 17,
            Item::Library => 18,
            Item::Backups => 19,
            Item::LiveLinkAccept => 20,
            Item::LiveLinkKeep => 21,
            Item::ExternalOps => 22,
            Item::UpdateStartup => 23,
            Item::UpdateBeta => 24,
            Item::BakeRayQuery => 25,
        }
    }

    /// 属する区分。
    pub fn category(self) -> Category {
        match self {
            Item::Language => Category::General,
            Item::PenInput | Item::TabletPressure | Item::PressureAdjust => Category::Pen,
            Item::Compositing | Item::Vsync | Item::GpuMemory | Item::BakeRayQuery => {
                Category::Display
            }
            Item::Budget(_) | Item::MinUndoSteps | Item::DiskCache => Category::Memory,
            Item::CpuThreads => Category::Processing,
            Item::OrbitCenter | Item::ZoomCenter | Item::AxisOrtho | Item::UvWireframe => {
                Category::View3d
            }
            Item::ExportPadding | Item::Library | Item::Backups => Category::Files,
            Item::LiveLinkAccept | Item::LiveLinkKeep | Item::ExternalOps => Category::LiveLink,
            Item::UpdateStartup | Item::UpdateBeta => Category::Updates,
        }
    }

    /// 行に出る名前（今の言語の文字。探す欄が当てる）。1 つの項目が行を重ねるもの（ポート番号・キャッシュの上限と場所・合計・下限と上限・すべて残す）は、
    /// 下の行の名前も全部。
    pub fn names(self, lang: Lang) -> Vec<&'static str> {
        use crate::settings::setting_name as name;
        match self {
            Item::Language => vec![lang.t("prefs-language")],
            Item::PenInput => vec![name(lang, "pen_input")],
            Item::TabletPressure => vec![name(lang, "tablet_pressure")],
            Item::PressureAdjust => vec![
                lang.t("prefs-pen-pressure"),
                lang.t("prefs-low"),
                lang.t("prefs-high"),
            ],
            Item::Compositing => vec![name(lang, "compositing")],
            Item::Vsync => vec![name(lang, "vsync")],
            Item::GpuMemory => vec![name(lang, "gpu_memory"), lang.t("prefs-total")],
            Item::BakeRayQuery => vec![name(lang, "bake_ray_query")],
            Item::Budget(kind) => vec![name(lang, kind.key())],
            Item::MinUndoSteps => vec![name(lang, "min_undo_steps")],
            Item::CpuThreads => vec![name(lang, "cpu_threads")],
            Item::OrbitCenter => vec![name(lang, "view3d_orbit")],
            Item::ZoomCenter => vec![name(lang, "view3d_zoom")],
            Item::AxisOrtho => vec![name(lang, "view3d_axis_ortho")],
            Item::UvWireframe => vec![lang.t("prefs-uv-wireframe")],
            Item::ExportPadding => vec![name(lang, "export_padding")],
            Item::Library => vec![name(lang, "library_folder")],
            Item::Backups => vec![lang.t("prefs-backups-to-keep"), lang.t("prefs-keep-all")],
            Item::DiskCache => vec![
                name(lang, "disk_cache"),
                name(lang, "disk_cache_limit_gib"),
                name(lang, "disk_cache_folder"),
            ],
            Item::LiveLinkAccept => vec![lang.t("prefs-accept-live-link-from-unity")],
            Item::LiveLinkKeep => vec![lang.t("prefs-save-material-values-received-from")],
            Item::ExternalOps => vec![name(lang, "external_ops"), name(lang, "external_ops_port")],
            Item::UpdateStartup => {
                vec![lang.t("prefs-check-for-updates-at-startup")]
            }
            Item::UpdateBeta => vec![lang.t("prefs-use-beta-versions")],
        }
    }
}

/// 探す欄の文字に合う区分と、その項目（区分の順・区分の中の順）。項目の名前に合うか、区分の名前に合う（その区分の項目は全部）。
/// ショートカットの区分は表なので項目を持たず、区分の名前に合うときだけ入る（項目は空）。
pub fn search(app: &AppState, query: &str) -> Vec<(Category, Vec<Item>)> {
    let lang = app.lang;
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let matches = |text: &str| text.to_lowercase().contains(&query);
    let mut found = Vec::new();
    for category in Category::ALL.into_iter().filter(|c| c.available(app)) {
        let items = category.items(app);
        if matches(category.label(lang)) {
            found.push((category, items));
            continue;
        }
        let hits: Vec<Item> = items
            .into_iter()
            .filter(|item| item.names(lang).into_iter().any(matches))
            .collect();
        if !hits.is_empty() {
            found.push((category, hits));
        }
    }
    found
}

/// 描き終えたあとで当てる、行の要求。
pub(super) enum Request {
    /// 選択肢のポップアップを、この矩形の下に開く。
    Open(PrefChoice, Rect),
    Do(PrefsAction),
    /// 設定のウィンドウの外の操作（更新の確かめ方）。
    Apply(Box<Action>),
}

/// 1 フレームの間に変わらない値（行を描く前に 1 回だけ読む）。
pub(super) struct View {
    lang: Lang,
    s: Settings,
    ram: u64,
    cores: u32,
    restart: bool,
    vsync_restart: bool,
    library: Option<PathBuf>,
    cache_free: Option<u64>,
    keep_all: bool,
    backup_count: u32,
    gpu_details: bool,
    gpu_total: u32,
    cache_details: bool,
    update_startup: bool,
    update_beta: bool,
}

impl View {
    pub(super) fn of(app: &AppState) -> View {
        let s = app.prefs.settings.clone();
        View {
            lang: app.lang,
            ram: app.prefs.ram_mib,
            cores: app.prefs.cores,
            restart: s.cpu_threads != app.prefs.threads_at_start,
            vsync_restart: s.vsync != app.prefs.vsync_at_start,
            library: s.library_folder(),
            cache_free: app.cache_free_now(),
            keep_all: s.backups == BackupKeep::All,
            backup_count: app.prefs.shown_backups(),
            gpu_details: app.prefs.gpu_details,
            gpu_total: app.gpu_total_mib(),
            cache_details: app.prefs.cache_details,
            update_startup: app.services.update.preference() == crate::update::Preference::On,
            update_beta: app.services.update.beta(),
            s,
        }
    }
}

/// 描いている間に集める結果。
#[derive(Default)]
pub(super) struct Out {
    pub requests: Vec<Request>,
    /// 退避の数・UV の色のスライダーをドラッグしている。
    pub dragging: bool,
    /// GPU のメモリの合計のスライダーをドラッグしている。
    pub gpu_dragging: bool,
}

fn window_id() -> Id {
    super::window::id()
}

/// 行の左に名前、右に値の箱（選ぶとポップアップ）。
fn choice_in(
    ui: &mut Ui,
    r: Rect,
    key: &str,
    label: &str,
    value: &str,
    tip: &str,
    which: PrefChoice,
) -> Option<Request> {
    let (response, b) = w::dropdown(
        ui,
        r,
        ("prefs", key),
        Some(label),
        value,
        Some(tip),
        true,
        LABEL_WIDTH,
    );
    response.clicked().then_some(Request::Open(which, b))
}

fn choice(
    ui: &mut Ui,
    rows: &mut w::Rows,
    key: &str,
    label: &str,
    value: &str,
    tip: &str,
    which: PrefChoice,
) -> Option<Request> {
    choice_in(
        ui,
        rows.row(t::ROW_HEIGHT, GAP),
        key,
        label,
        value,
        tip,
        which,
    )
}

/// 節の見出し（探した結果の区分の名前・筆圧の調整）。`line` は上に細い線を引く。
pub(super) fn heading(ui: &mut Ui, rows: &mut w::Rows, line: bool, title: &str) {
    let r = rows.row(HEADING, 0.0);
    if line {
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
}

/// 行を、左の欄と、右の「詳しく」の開け閉めに分ける。
fn details_split(ui: &Ui, r: Rect, lang: Lang) -> (Rect, Rect) {
    let width = 16.0 + w::text_width(ui.painter(), lang.t("prefs-details"), t::LABEL_BOLD) + 12.0;
    let details = Rect::from_min_max(pos2(r.right() - width, r.top()), r.max);
    let left = Rect::from_min_max(r.min, pos2(details.left() - 8.0, r.bottom()));
    (left, details)
}

/// フォルダの 2 行（ラベルとパス、その下に「選ぶ…」「既定に戻す」）。押したボタン（選ぶ・既定に戻す）を返す。
#[allow(clippy::too_many_arguments)]
fn folder_rows(
    ui: &mut Ui,
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
    let (choose, default) = (lang.t("prefs-choose"), lang.t("prefs-default"));
    let widths = [choose, default].map(|s| w::text_width(&p, s, t::LABEL) + 24.0);
    let right = row.right();
    let default_rect = Rect::from_min_size(
        pos2(right - widths[1], row.top()),
        vec2(widths[1], row.height()),
    );
    let choose_rect = Rect::from_min_size(
        pos2(default_rect.left() - 4.0 - widths[0], row.top()),
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

/// 項目の置かれる所。
pub(super) struct Slot {
    /// 右の欄（探した結果なら、区分のまとまり）の先頭の項目か（先頭の見出しの上には線を引かない）。
    pub first: bool,
    /// 右の欄のうち画面に出ている範囲（筆圧の調整の描く枠が、隠れた所を数えない）。
    pub visible: Rect,
}

/// 項目 1 つの行を `rows` に並べる。
pub(super) fn draw_item(
    ui: &mut Ui,
    rows: &mut w::Rows,
    app: &mut AppState,
    v: &View,
    item: Item,
    slot: &Slot,
    out: &mut Out,
) {
    let lang = v.lang;
    let s = &v.s;
    let id = window_id();
    match item {
        Item::Language => {
            out.requests.extend(choice(
                ui,
                rows,
                "language",
                lang.t("prefs-language"),
                lang.name(),
                lang.t("prefs-the-language-of-the-screen-tip"),
                PrefChoice::Language,
            ));
        }
        Item::PenInput => {
            out.requests.extend(choice(
                ui,
                rows,
                "pen-input",
                crate::settings::setting_name(lang, "pen_input"),
                pen_api_name(s.pen_input),
                lang.t("prefs-how-pen-pressure-tilt-rotation-tip"),
                PrefChoice::PenInput,
            ));
        }
        Item::TabletPressure => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("tablet-pressure"),
                crate::settings::setting_name(lang, "tablet_pressure"),
                s.tablet_pressure,
                Some(lang.t("prefs-reads-the-pressure-tilt-eraser-tip")),
                true,
            );
            if next != s.tablet_pressure {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::TabletPressure(next))));
            }
        }
        Item::BakeRayQuery => {
            // 環境変数で切っているあいだは、設定を入れても効かない。押せなくして理由を出す
            let allowed = yolu_gpu::ray_query_env_allows();
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("bake-ray-query"),
                crate::settings::setting_name(lang, "bake_ray_query"),
                s.bake_ray_query && allowed,
                (!allowed).then(|| lang.t("prefs-turned-off-by-an-environment-tip")),
                allowed,
            );
            if allowed && next != s.bake_ray_query {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::BakeRayQuery(next))));
            }
        }
        Item::PressureAdjust => {
            // 上に行があるときだけ、見出しの上に線を引く
            heading(ui, rows, !slot.first, lang.t("prefs-pen-pressure"));
            crate::pen::window::rows(ui, rows, app, slot.visible);
        }
        Item::Compositing => {
            out.requests.extend(choice(
                ui,
                rows,
                "compositing",
                lang.t("prefs-display-compositing"),
                compositing_name(lang, s.compositing),
                lang.t("prefs-where-the-layers-shown-on-tip"),
                PrefChoice::Compositing,
            ));
        }
        Item::Vsync => {
            // 垂直同期（ウィンドウの面を作るときに決まる。起動のときの値と違えば「再起動で反映」）
            let mut label = crate::settings::setting_name(lang, "vsync").to_owned();
            if v.vsync_restart {
                label += &format!(
                    "{}{}",
                    lang.t("prefs-text-3"),
                    lang.t("prefs-applies-after-restart")
                );
            }
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("vsync"),
                &label,
                s.vsync,
                Some(lang.t("prefs-waiting-for-the-monitor-s-tip")),
                true,
            );
            if next != s.vsync {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::Vsync(next))));
            }
        }
        Item::GpuMemory => {
            // GPU のメモリと、同じ行の右に「詳しく」
            let (gpu_rect, details_rect) = details_split(ui, rows.row(t::ROW_HEIGHT, GAP), lang);
            out.requests.extend(choice_in(
                ui,
                gpu_rect,
                "gpu-memory",
                crate::settings::setting_name(lang, "gpu_memory"),
                s.gpu_memory.name(lang),
                lang.t("prefs-how-much-gpu-memory-the-tip"),
                PrefChoice::GpuMemory,
            ));
            let open = w::subsection_header(
                ui,
                details_rect,
                ("prefs", "gpu-details"),
                lang.t("prefs-details"),
                v.gpu_details,
            );
            if open != v.gpu_details {
                out.requests
                    .push(Request::Do(PrefsAction::GpuDetails(open)));
            }
            if v.gpu_details {
                let slider = w::slider(
                    ui,
                    rows.row(t::SLIDER_ROW_HEIGHT, GAP),
                    ("prefs", "gpu-total"),
                    v.gpu_total as f32,
                    &SliderSpec::new(
                        lang.t("prefs-total"),
                        gpu_memory::MIN_TOTAL_MIB as f32,
                        gpu_memory::MAX_TOTAL_MIB as f32,
                        NumberFormat::int(" MiB"),
                    )
                    .tooltip(lang.t("prefs-the-total-gpu-memory-split-tip")),
                );
                out.gpu_dragging = slider.active;
                // 押し始めの選びを覚える（この枠の `s` は、押した枠の変更を入れる前の設定）
                if slider.active && app.prefs.gpu_memory_before_drag.is_none() {
                    app.prefs.gpu_memory_before_drag = Some(s.gpu_memory);
                }
                if slider.changed {
                    if !slider.active && !slider.released {
                        // Esc でドラッグを止めた（スライダーは押し始めの量を返す）。量ではなく、押す前の選びへ戻す
                        if let Some(before) = app.prefs.gpu_memory_before_drag {
                            out.requests
                                .push(Request::Do(PrefsAction::Set(Pref::GpuMemory(before))));
                        }
                    } else {
                        let step = gpu_memory::TOTAL_STEP_MIB as f32;
                        let mib = ((slider.value / step).round() * step) as u32;
                        out.requests
                            .push(Request::Do(PrefsAction::Set(Pref::GpuMemory(
                                GpuMemory::Mib(mib),
                            ))));
                    }
                }
                if !slider.active {
                    app.prefs.gpu_memory_before_drag = None;
                }
            }
        }
        Item::Budget(kind) => {
            let tip = match kind {
                BudgetKind::Undo => lang.t("prefs-memory-for-the-undo-history-tip"),
                BudgetKind::Source => lang.t("prefs-the-most-memory-all-layers-tip"),
                BudgetKind::Stroke => lang.t("prefs-undo-data-one-stroke-or-tip"),
            };
            out.requests.extend(choice(
                ui,
                rows,
                kind.key(),
                crate::settings::setting_name(lang, kind.key()),
                &budget_name(lang, kind, s.budget(kind), v.ram),
                tip,
                PrefChoice::Budget(kind),
            ));
        }
        Item::MinUndoSteps => {
            let slider = w::slider(
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
                .tooltip(lang.t("prefs-the-newest-steps-kept-even-tip")),
            );
            if slider.changed {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::MinUndoSteps(
                        slider.value.round().clamp(0.0, MAX_MIN_UNDO_STEPS as f32) as u32,
                    ))));
            }
        }
        Item::CpuThreads => {
            let mut threads = threads_name(lang, s.cpu_threads, v.cores);
            if v.restart {
                threads += &format!(
                    "{}{}",
                    lang.t("prefs-text-3"),
                    lang.t("prefs-applies-after-restart")
                );
            }
            out.requests.extend(choice(
                ui,
                rows,
                "threads",
                lang.t("prefs-cpu-threads"),
                &threads,
                lang.t("prefs-the-most-threads-the-cpu-tip"),
                PrefChoice::CpuThreads,
            ));
        }
        Item::OrbitCenter => {
            out.requests.extend(choice(
                ui,
                rows,
                "orbit-center",
                lang.t("prefs-orbit-center"),
                s.navigation.orbit.label(lang),
                lang.t("prefs-the-pivot-when-orbiting-the-tip"),
                PrefChoice::OrbitCenter,
            ));
        }
        Item::ZoomCenter => {
            out.requests.extend(choice(
                ui,
                rows,
                "zoom-center",
                lang.t("prefs-zoom-center"),
                s.navigation.zoom.label(lang),
                lang.t("prefs-the-center-when-zooming-the-tip"),
                PrefChoice::ZoomCenter,
            ));
        }
        Item::AxisOrtho => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("axis-ortho"),
                crate::settings::setting_name(lang, "view3d_axis_ortho"),
                s.navigation.axis_ortho,
                Some(lang.t("prefs-switches-to-orthographic-when-you-tip")),
                true,
            );
            if next != s.navigation.axis_ortho {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::AxisOrthographic(next))));
            }
        }
        Item::UvWireframe => {
            out.dragging |= crate::uv_wireframe::settings_row(ui, rows, app);
        }
        Item::ExportPadding => {
            out.requests.extend(choice(
                ui,
                rows,
                "padding",
                lang.t("prefs-export-padding"),
                &padding_name(lang, s.export_padding),
                lang.t("prefs-how-far-the-colors-at-tip"),
                PrefChoice::ExportPadding,
            ));
        }
        Item::Library => {
            // ライブラリの場所: ラベルとパス、その下にボタン
            let shown = v
                .library
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let (choose, reset) = folder_rows(
                ui,
                rows,
                lang,
                "library",
                lang.t("prefs-library-folder"),
                &shown,
                lang.t("prefs-choose-the-library-folder-tip"),
                lang.t("prefs-back-to-the-default-folder-tip"),
                s.library_folder.is_some(),
                true,
            );
            if choose {
                out.requests
                    .push(Request::Do(PrefsAction::ChooseLibraryFolder));
            }
            if reset {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::LibraryFolder(None))));
            }
        }
        Item::Backups => {
            // 退避を残す数: スライダー（すべて残す間は動かせない）と「すべて残す」
            let slider = w::slider(
                ui,
                rows.row(t::SLIDER_ROW_HEIGHT, GAP),
                ("prefs", "backups"),
                v.backup_count as f32,
                &SliderSpec::new(
                    lang.t("prefs-backups-to-keep"),
                    0.0,
                    MAX_BACKUPS_TO_KEEP as f32,
                    NumberFormat::int(""),
                )
                .tooltip(lang.t("prefs-how-many-replaced-versions-to-tip"))
                .enabled(!v.keep_all),
            );
            out.dragging |= slider.active;
            if slider.changed {
                let n = slider.value.round().clamp(0.0, MAX_BACKUPS_TO_KEEP as f32) as u32;
                out.requests
                    .push(Request::Do(PrefsAction::SetBackups(BackupKeep::Count(n))));
            }
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("backups-all"),
                lang.t("prefs-keep-all"),
                v.keep_all,
                Some(lang.t("prefs-never-delete-backups-tip")),
                true,
            );
            if next != v.keep_all {
                let keep = if next {
                    BackupKeep::All
                } else {
                    BackupKeep::Count(v.backup_count)
                };
                out.requests
                    .push(Request::Do(PrefsAction::SetBackups(keep)));
            }
        }
        Item::DiskCache => {
            // ディスクキャッシュの入切と、同じ行の右に「詳しく」
            let (toggle_rect, details_rect) = details_split(ui, rows.row(t::ROW_HEIGHT, GAP), lang);
            let next = w::toggle(
                ui,
                toggle_rect,
                id.with("disk-cache"),
                crate::settings::setting_name(lang, "disk_cache"),
                s.disk_cache,
                Some(lang.t("prefs-moves-the-tiles-beyond-the-tip")),
                true,
            );
            if next != s.disk_cache {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::DiskCache(next))));
            }
            let open = w::subsection_header(
                ui,
                details_rect,
                ("prefs", "cache-details"),
                lang.t("prefs-details"),
                v.cache_details,
            );
            if open != v.cache_details {
                out.requests
                    .push(Request::Do(PrefsAction::CacheDetails(open)));
            }
            if v.cache_details {
                let r = rows.row(t::ROW_HEIGHT, GAP);
                let (response, b) = w::dropdown(
                    ui,
                    r,
                    ("prefs", "disk-cache-limit"),
                    Some(crate::settings::setting_name(lang, "disk_cache_limit_gib")),
                    &disk_limit_name(lang, s.disk_cache_limit, v.cache_free),
                    Some(lang.t("prefs-the-most-disk-space-the-tip")),
                    s.disk_cache,
                    LABEL_WIDTH,
                );
                if response.clicked() {
                    out.requests
                        .push(Request::Open(PrefChoice::DiskCacheLimit, b));
                }
                let shown = s.disk_cache_folder().display().to_string();
                let (choose, reset) = folder_rows(
                    ui,
                    rows,
                    lang,
                    "cache",
                    crate::settings::setting_name(lang, "disk_cache_folder"),
                    &shown,
                    lang.t("prefs-choose-the-folder-for-the-tip"),
                    lang.t("prefs-back-to-the-system-temporary-tip"),
                    s.disk_cache_folder.is_some(),
                    s.disk_cache,
                );
                if choose {
                    out.requests
                        .push(Request::Do(PrefsAction::ChooseCacheFolder));
                }
                if reset {
                    out.requests
                        .push(Request::Do(PrefsAction::Set(Pref::DiskCacheFolder(None))));
                }
            }
        }
        Item::LiveLinkAccept => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("livelink-on-startup"),
                lang.t("prefs-accept-live-link-from-unity"),
                s.livelink_on_startup,
                Some(lang.t("prefs-opens-the-model-and-materials-tip")),
                true,
            );
            if next != s.livelink_on_startup {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::LiveLinkOnStartup(next))));
            }
        }
        Item::LiveLinkKeep => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("livelink-keep-values"),
                lang.t("prefs-save-material-values-received-from"),
                s.livelink_keep_values,
                Some(lang.t("prefs-stores-the-liltoon-values-received-tip")),
                true,
            );
            if next != s.livelink_keep_values {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::LiveLinkKeepValues(
                        next,
                    ))));
            }
        }
        Item::ExternalOps => {
            let ops_url = yolu_mcp::endpoint(s.external_ops_port);
            let ops_tip = lang.f(
                "prefs-accepts-commands-from-ai-assistants-tip",
                &[("ops_url", &ops_url)],
            );
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("external-ops"),
                crate::settings::setting_name(lang, "external_ops"),
                s.external_ops,
                Some(&ops_tip),
                true,
            );
            if next != s.external_ops {
                out.requests
                    .push(Request::Do(PrefsAction::Set(Pref::ExternalOps(next))));
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
                    Some(lang.t("prefs-the-port-external-commands-are-tip")),
                    false,
                );
                if let Some(text) = typed.committed {
                    // 番号でない・範囲の外は 0 として渡し、断る理由を出す
                    let port = text.trim().parse::<u16>().unwrap_or(0);
                    out.requests
                        .push(Request::Do(PrefsAction::Set(Pref::ExternalOpsPort(port))));
                }
            }
        }
        Item::UpdateStartup => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("update-startup"),
                lang.t("prefs-check-for-updates-at-startup"),
                v.update_startup,
                None,
                true,
            );
            if next != v.update_startup {
                out.requests.push(Request::Apply(Box::new(Action::Update(
                    UpdateAction::SetCheckOnStartup(next),
                ))));
            }
        }
        Item::UpdateBeta => {
            let next = w::toggle(
                ui,
                rows.row(t::ROW_HEIGHT, GAP),
                id.with("update-beta"),
                lang.t("prefs-use-beta-versions"),
                v.update_beta,
                Some(lang.t("prefs-also-offers-beta-versions-as-tip")),
                true,
            );
            if next != v.update_beta {
                out.requests.push(Request::Apply(Box::new(Action::Update(
                    UpdateAction::SetBeta(next),
                ))));
            }
        }
    }
}
