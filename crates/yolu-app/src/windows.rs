//! 浮いた窓（ベイクの窓・書き出しの確かめと結果・PSD の結果と確かめ）と、長い仕事の札（進み具合と取消）を毎フレーム描く。
//! 窓の中身は `bake::window`・ここの一覧の窓。状態は `AppState` の `bake`・`export`・`psd`。

use egui::{pos2, vec2, Id, Key, Order, Rect, Sense, UiBuilder, Vec2};

use crate::bake::{self, BakeAction};
use crate::export::{self, ExportAction};
use crate::lang::Lang;
use crate::psd::PsdAction;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// 一覧の 1 行。
pub struct Row {
    pub left: String,
    pub middle: String,
    pub right: String,
    pub warning: bool,
}

impl Row {
    pub fn text(left: impl Into<String>, warning: bool) -> Row {
        Row {
            left: left.into(),
            middle: String::new(),
            right: String::new(),
            warning,
        }
    }
}

/// 一覧の窓の下の帯のボタン。
pub struct Button {
    pub label: String,
    pub primary: bool,
    pub tooltip: Option<String>,
}

pub struct ListSpec {
    pub id: &'static str,
    pub title: String,
    pub icon: &'static str,
    pub modal: bool,
    pub width: f32,
    /// 見出しの下の 1 行（名前・状態・短い理由）と、注意の色か。
    pub summary: Option<(String, bool)>,
    pub rows: Vec<Row>,
    pub buttons: Vec<Button>,
    pub close_label: String,
}

/// 名前の窓（"bake"・"export-confirm"・"export-report"・"psd-confirm"・"psd-report"）の最後に描いた矩形（試験が窓の中だけを撮る）。
pub fn window_rect(ctx: &egui::Context, name: &str) -> Option<Rect> {
    let id = if name == "bake" {
        Id::new("yolu.bake-window")
    } else {
        Id::new(("yolu.window", name))
    };
    window::last_rect(ctx, id)
}

/// 一覧の窓の返事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Button(usize),
    /// 閉じるボタンか Esc。
    Closed,
}

const ROW_HEIGHT: f32 = 22.0;
const MAX_ROWS: usize = 12;
const FOOTER: f32 = 48.0;

/// 一覧の窓を描く。押されたボタン・閉じたを返す。
pub fn show_list(
    ctx: &egui::Context,
    spec: &ListSpec,
    offset: &mut Vec2,
    scroll: &mut f32,
) -> Option<Reply> {
    let visible = spec.rows.len().min(MAX_ROWS);
    let summary_h = if spec.summary.is_some() { 30.0 } else { 6.0 };
    let height = window::HEADER_HEIGHT + summary_h + visible as f32 * ROW_HEIGHT + 10.0 + FOOTER;
    let window_spec = Spec {
        title: &spec.title,
        icon: Some(spec.icon),
        size: vec2(spec.width, height),
        modal: spec.modal,
        close_label: &spec.close_label,
    };
    let mut reply = None;
    let id = Id::new(("yolu.window", spec.id));
    let mut esc = false;
    let closed = window::show(ctx, id, &window_spec, offset, false, |ui, frame| {
        esc = ui.input(|i| i.key_pressed(Key::Escape))
            && (spec.modal
                || ui
                    .input(|i| i.pointer.hover_pos())
                    .is_some_and(|p| frame.rect.contains(p)));
        let body = frame.body;
        let p = ui.painter().clone();
        let mut y = body.top() + 4.0;
        if let Some((text, warning)) = &spec.summary {
            let r =
                Rect::from_min_size(pos2(body.left() + 14.0, y), vec2(body.width() - 28.0, 22.0));
            let shown = w::fit(&p, text, r.width(), t::LABEL);
            w::text(
                &p,
                r,
                &shown,
                t::LABEL.with_color(if *warning { t::WARNING } else { t::TEXT }),
                Align::Left,
            );
            if shown != *text {
                ui.interact(r, id.with("summary"), Sense::hover())
                    .on_hover_text(text);
            }
            y += 26.0;
        } else {
            y += 2.0;
        }
        let list = Rect::from_min_size(
            pos2(body.left(), y),
            vec2(body.width(), visible as f32 * ROW_HEIGHT),
        );
        let content = spec.rows.len() as f32 * ROW_HEIGHT;
        let max_scroll = (content - list.height()).max(0.0);
        if ui.rect_contains_pointer(list) {
            *scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        *scroll = scroll.clamp(0.0, max_scroll);
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        let cp = child.painter().clone();
        let right_w = spec
            .rows
            .iter()
            .map(|r| w::text_width(&cp, &r.right, t::LABEL_DIM))
            .fold(0.0f32, f32::max);
        let mid_w = spec
            .rows
            .iter()
            .map(|r| w::text_width(&cp, &r.middle, t::LABEL_DIM))
            .fold(0.0f32, f32::max);
        for (i, row) in spec.rows.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(
                    list.left() + 14.0,
                    list.top() + i as f32 * ROW_HEIGHT - *scroll,
                ),
                vec2(
                    list.width() - 28.0 - if max_scroll > 0.0 { 8.0 } else { 0.0 },
                    ROW_HEIGHT,
                ),
            );
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            let right = Rect::from_min_size(
                pos2(r.right() - right_w, r.top()),
                vec2(right_w, r.height()),
            );
            let middle_right = if right_w > 0.0 {
                right.left() - 12.0
            } else {
                r.right()
            };
            let middle =
                Rect::from_min_size(pos2(middle_right - mid_w, r.top()), vec2(mid_w, r.height()));
            let left_right = if mid_w > 0.0 {
                middle.left() - 12.0
            } else {
                middle_right
            };
            let left = Rect::from_min_max(r.min, pos2(left_right, r.bottom()));
            let color = if row.warning { t::WARNING } else { t::TEXT };
            let shown = w::fit(&cp, &row.left, left.width(), t::LABEL);
            w::text(&cp, left, &shown, t::LABEL.with_color(color), Align::Left);
            if shown != row.left {
                child
                    .interact(left, id.with(("row", i)), Sense::hover())
                    .on_hover_text(&row.left);
            }
            w::text(&cp, middle, &row.middle, t::LABEL_DIM, Align::Left);
            w::text(&cp, right, &row.right, t::LABEL_DIM, Align::Right);
        }
        if max_scroll > 0.0 {
            let bar_h = (list.height() * list.height() / content).max(16.0);
            let bar_y = list.top() + (list.height() - bar_h) * *scroll / max_scroll;
            w::rounded(
                &cp,
                Rect::from_min_size(pos2(list.right() - 8.0, bar_y), vec2(4.0, bar_h)),
                t::CONTROL_ACTIVE,
                2.0,
            );
        }
        // 下の帯
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let mut x = footer.right() - 14.0;
        for (i, b) in spec.buttons.iter().enumerate().rev() {
            let bw = w::text_width(&p, &b.label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            if w::button(
                ui,
                r,
                id.with(("button", i)),
                &b.label,
                b.primary,
                true,
                b.tooltip.as_deref(),
                None,
            )
            .clicked()
            {
                reply = Some(Reply::Button(i));
            }
        }
    });
    if reply.is_none() && (closed || esc) {
        reply = Some(Reply::Closed);
    }
    reply
}

/// 毎フレーム: 窓と仕事の札を描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    // 別のスレッドの仕事が動いている間は描き直し続ける（進み具合・終わりを受ける）
    if app.bake.is_baking()
        || app.bake.is_checking()
        || app.bake.is_probing_gpu()
        || app.export.is_exporting()
        || app.psd.is_busy()
        || app.update.is_busy()
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    bake::window::show(ctx, app);
    crate::panels::brush_detail::show(ctx, app);
    export_confirm(ctx, app);
    export_report(ctx, app);
    psd_confirm(ctx, app);
    psd_report(ctx, app);
    crate::update::window::show(ctx, app);
    job_card(ctx, app);
    app.release_idle_bake_input();
}

/// 確かめの窓や結果の窓が開いている（キーの割り当てを止める）。
pub fn modal_open(app: &AppState) -> bool {
    app.export.confirm.is_some() || app.psd.confirm.is_some() || app.update.window_open()
}

fn export_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(confirm) = app.export.confirm.clone() else {
        return;
    };
    let lang = app.lang;
    let mut rows: Vec<Row> = confirm
        .existing
        .iter()
        .take(8)
        .map(|n| Row::text(n.clone(), false))
        .collect();
    if confirm.existing.len() > 8 {
        rows.push(Row::text(
            lang.pick(
                format!("ほか {} 件", confirm.existing.len() - 8),
                format!("and {} more", confirm.existing.len() - 8),
            ),
            false,
        ));
    }
    let spec = ListSpec {
        id: "export-confirm",
        title: lang.pick("置き換えるファイル", "Files to Replace").into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((
            lang.pick(
                format!(
                    "書く {} 枚のうち、もうあるファイル {} 個",
                    confirm.total,
                    confirm.existing.len()
                ),
                format!(
                    "{} of {} images already exist",
                    confirm.existing.len(),
                    confirm.total
                ),
            ),
            true,
        )),
        rows,
        buttons: vec![
            Button {
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.pick("置き換える", "Replace").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "同じ名前のファイルを新しい画像に置き換えます",
                        "Replaces the files with the same names",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.export.confirm_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.export.confirm_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Export(ExportAction::ConfirmReplace)),
        Some(_) => app.apply(Action::Export(ExportAction::CancelConfirm)),
        None => {}
    }
}

fn export_report(ctx: &egui::Context, app: &mut AppState) {
    let Some(report) = app.export.report.clone() else {
        return;
    };
    let lang = app.lang;
    let mut rows: Vec<Row> = report
        .images
        .iter()
        .map(|i| Row {
            left: i.file_name.clone(),
            middle: format!(
                "{}×{}  {}",
                i.width,
                i.height,
                if i.normal_map {
                    lang.pick("法線", "Normal")
                } else if i.srgb {
                    "sRGB"
                } else {
                    lang.pick("リニア", "Linear")
                }
            ),
            right: if i.replaced {
                lang.pick("置き換え", "Replaced").into()
            } else {
                lang.pick("新規", "New").into()
            },
            warning: false,
        })
        .collect();
    for n in &report.notes {
        rows.push(Row::text(export::note_text(lang, n), true));
    }
    let spec = ListSpec {
        id: "export-report",
        title: lang.pick("書き出した画像", "Exported Images").into(),
        icon: "folder_open",
        modal: false,
        width: 560.0,
        summary: Some((
            format!("{} → {}", report.template, report.dir.display()),
            false,
        )),
        rows,
        buttons: vec![Button {
            label: lang.pick("閉じる", "Close").into(),
            primary: false,
            tooltip: None,
        }],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.export.report_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.export.report_offset = offset;
    if reply.is_some() {
        app.apply(Action::Export(ExportAction::DismissReport));
    }
}

fn psd_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(path) = app.psd.confirm.clone() else {
        return;
    };
    let lang = app.lang;
    let spec = ListSpec {
        id: "psd-confirm",
        title: lang.pick("取り込んだ PSD を置き換える", "Replace the Imported PSD").into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            true,
        )),
        rows: vec![Row::text(path.display().to_string(), false)],
        buttons: vec![
            Button {
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.pick("置き換える", "Replace").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "この文書を取り込んだ PSD です。書き出した PSD に置き換えます",
                        "This document was imported from this PSD. It is replaced by the exported PSD",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.psd.confirm_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.psd.confirm_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Psd(PsdAction::ConfirmReplace)),
        Some(_) => app.apply(Action::Psd(PsdAction::CancelConfirm)),
        None => {}
    }
}

fn psd_report(ctx: &egui::Context, app: &mut AppState) {
    let Some(report) = app.psd.report.clone() else {
        return;
    };
    let lang = app.lang;
    let spec = ListSpec {
        id: "psd-report",
        title: if report.importing {
            lang.pick("PSD の読み込み", "PSD Import")
        } else {
            lang.pick("PSD の書き出し", "PSD Export")
        }
        .into(),
        icon: if report.ok { "info" } else { "warning" },
        modal: false,
        width: 640.0,
        summary: Some((format!("{} · {}", report.file, report.summary), !report.ok)),
        rows: report
            .lines
            .iter()
            .map(|l| Row::text(l.text.clone(), l.warning))
            .collect(),
        buttons: vec![Button {
            label: lang.pick("閉じる", "Close").into(),
            primary: false,
            tooltip: None,
        }],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.psd.report_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.psd.report_offset = offset;
    if reply.is_some() {
        app.apply(Action::Psd(PsdAction::DismissReport));
    }
}

/// 長い仕事（ベイクの窓を閉じているあいだのベイク・書き出し・PSD）の札。右下に出し、進み具合と取消を見せる。
fn job_card(ctx: &egui::Context, app: &mut AppState) {
    struct Entry {
        id: &'static str,
        text: String,
        fraction: Option<f32>,
        cancel: Action,
        canceling: bool,
    }
    let lang: Lang = app.lang;
    let mut entries: Vec<Entry> = Vec::new();
    if app.bake.window.is_none() {
        if let Some(p) = app.bake.progress() {
            let set = if p.total > 1 {
                format!(
                    "{} {}/{}: {} · ",
                    lang.pick("セット", "Set"),
                    p.index,
                    p.total,
                    p.set
                )
            } else {
                String::new()
            };
            entries.push(Entry {
                id: "bake",
                text: format!(
                    "{} — {set}{}… {}%",
                    lang.pick("メッシュマップをベイク", "Baking mesh maps"),
                    if p.canceling {
                        lang.pick("取り消し中", "Canceling").to_owned()
                    } else {
                        bake::phase_label(lang, &p.phase)
                    },
                    (p.fraction * 100.0) as i32
                ),
                fraction: Some(p.fraction as f32),
                cancel: Action::Bake(BakeAction::Cancel),
                canceling: p.canceling,
            });
        }
    }
    if let Some(p) = app.export.progress() {
        entries.push(Entry {
            id: "export",
            text: format!(
                "{} — {} {}/{}",
                lang.pick("書き出し", "Exporting"),
                p.template,
                p.index,
                p.total
            ),
            fraction: Some((p.index.saturating_sub(1)) as f32 / p.total.max(1) as f32),
            cancel: Action::Export(ExportAction::Cancel),
            canceling: p.canceling,
        });
    }
    if let Some(p) = app.psd.progress() {
        entries.push(Entry {
            id: "psd",
            text: format!(
                "{} — {}",
                if p.importing {
                    lang.pick("PSD を読み込み中", "Reading PSD")
                } else {
                    lang.pick("PSD を書き出し中", "Writing PSD")
                },
                p.file
            ),
            fraction: None,
            cancel: Action::Psd(PsdAction::Cancel),
            canceling: p.canceling,
        });
    }
    if let Some(p) = app.update.progress() {
        entries.push(Entry {
            id: "update",
            text: format!(
                "{} — {}",
                lang.pick("更新をダウンロード中", "Downloading update"),
                p.version
            ),
            fraction: Some(p.fraction),
            cancel: Action::Update(crate::update::UpdateAction::Cancel),
            canceling: p.canceling,
        });
    }
    if entries.is_empty() {
        return;
    }
    let screen = ctx.content_rect();
    let row_h = 38.0;
    let size = vec2(340.0, entries.len() as f32 * row_h + 8.0);
    let rect = Rect::from_min_size(
        pos2(
            screen.right() - size.x - 12.0,
            screen.bottom() - t::STATUS_BAR_HEIGHT - size.y - 10.0,
        ),
        size,
    );
    let mut cancel: Option<Action> = None;
    egui::Area::new(Id::new("yolu.jobs"))
        .order(Order::Middle)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            w::rounded(&p, rect, t::PANEL_BG, 6.0);
            w::outline(&p, rect, t::SEPARATOR, 1.0, 6.0);
            for (i, e) in entries.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(rect.left() + 10.0, rect.top() + 4.0 + i as f32 * row_h),
                    vec2(rect.width() - 20.0, row_h),
                );
                let button = Rect::from_min_size(
                    pos2(row.right() - 24.0, row.top() + 4.0),
                    vec2(24.0, 24.0),
                );
                let text_rect =
                    Rect::from_min_max(row.min, pos2(button.left() - 6.0, row.top() + 18.0));
                let shown = w::fit(&p, &e.text, text_rect.width(), t::LABEL_DIM);
                w::text(&p, text_rect, &shown, t::LABEL_DIM, Align::Left);
                let bar = Rect::from_min_max(
                    pos2(row.left(), row.top() + 22.0),
                    pos2(button.left() - 6.0, row.top() + 28.0),
                );
                w::rounded(&p, bar, t::CONTROL_BG, 3.0);
                match e.fraction {
                    Some(f) => w::rounded(
                        &p,
                        Rect::from_min_size(
                            bar.min,
                            vec2(bar.width() * f.clamp(0.0, 1.0), bar.height()),
                        ),
                        t::ACCENT,
                        3.0,
                    ),
                    None => {
                        // 終わりの分からない仕事: 往復する帯
                        let phase = (ctx.input(|i| i.time) * 1.2).fract() as f32;
                        let wdt = bar.width() * 0.25;
                        let x =
                            bar.left() + (bar.width() - wdt) * (1.0 - (phase * 2.0 - 1.0).abs());
                        w::rounded(
                            &p,
                            Rect::from_min_size(pos2(x, bar.top()), vec2(wdt, bar.height())),
                            t::ACCENT,
                            3.0,
                        );
                    }
                }
                if w::icon_button(
                    ui,
                    button,
                    ("yolu.job.cancel", e.id),
                    "close",
                    &format!(
                        "{}: {}",
                        lang.pick("取消", "Cancel"),
                        e.text.split(" — ").next().unwrap_or_default()
                    ),
                    false,
                    !e.canceling,
                    15.0,
                )
                .clicked()
                {
                    cancel = Some(e.cancel.clone());
                }
            }
        });
    if let Some(action) = cancel {
        app.apply(action);
    }
}

/// 試験用: 別のスレッドの仕事を、取消が来るまで始めずに止めておく（取消・終了前の取消が効いたことを、仕事の速さに頼らず
/// 確かめるため。止めた仕事は取消を見てすぐ止まる）。
#[doc(hidden)]
pub fn park_until_canceled(cancel: &std::sync::atomic::AtomicBool) {
    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// 終わる前に、走っている仕事（ベイク・書き出し・PSD）を取り消して、止まるのを少し待つ（書きかけの一時ファイルを残さないため。
/// 取消は次の区切りで効くので、待つのは `wait` まで）。
pub fn stop_jobs(app: &mut AppState, wait: std::time::Duration) {
    app.apply(Action::Bake(BakeAction::Cancel));
    app.apply(Action::Export(ExportAction::Cancel));
    app.apply(Action::Psd(PsdAction::Cancel));
    app.apply(Action::Update(crate::update::UpdateAction::Cancel));
    let start = std::time::Instant::now();
    while (app.bake.is_baking()
        || app.export.is_exporting()
        || app.psd.is_busy()
        || app.update.is_busy())
        && start.elapsed() < wait
    {
        app.poll_bake();
        app.poll_export();
        app.poll_psd();
        app.poll_update();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
