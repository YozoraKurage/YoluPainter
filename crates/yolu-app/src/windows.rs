//! 浮いたウィンドウ（ベイクのウィンドウ・書き出しの確かめと結果・PSD の結果と確かめ）と、長い仕事の札（進み具合と取消）を毎フレーム描く。
//! ウィンドウの中身は `bake::window`・ここの一覧のウィンドウ。状態は `AppState` の `bake`・`export`・`psd`。

use egui::{pos2, vec2, Id, Key, Order, Rect, Sense, UiBuilder, Vec2};

use crate::bake;
use crate::export::{self, ExportAction};
use crate::lang::Lang;
use crate::psd::PsdAction;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
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

/// 一覧のウィンドウの下の帯のボタン。
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

/// 名前のウィンドウ（"bake"・"export"・"export-confirm"・"export-report"・"psd-confirm"・"psd-import"・"psd-report"・"merge-confirm"）の最後に描いた矩形（試験がウィンドウの中だけを撮る）。
pub fn window_rect(ctx: &egui::Context, name: &str) -> Option<Rect> {
    let id = if name == "bake" {
        Id::new("yolu.bake-window")
    } else {
        Id::new(("yolu.window", name))
    };
    window::last_rect(ctx, id)
}

/// 一覧のウィンドウの返事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Button(usize),
    /// 閉じるボタンか Esc。
    Closed,
}

const ROW_HEIGHT: f32 = 22.0;
const MAX_ROWS: usize = 12;
const FOOTER: f32 = 48.0;

/// 一覧のウィンドウを描く。押されたボタン・閉じたを返す。
pub fn show_list(
    ctx: &egui::Context,
    spec: &ListSpec,
    offset: &mut Vec2,
    scroll: &mut f32,
) -> Option<Reply> {
    show_list_with(ctx, spec, &[], offset, scroll)
}

/// `show_list` の、行ごとのツールチップ（`tips[i]` が `Some` の行に載せる。説明はここに置く）を渡せる形。
pub fn show_list_with(
    ctx: &egui::Context,
    spec: &ListSpec,
    tips: &[Option<String>],
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
    // ずらした量はウィンドウの id で覚える（呼ぶ側が毎回 0 から渡しても、ホイールとつまみが効く）。ウィンドウを出していなかった後は、渡された値から
    let scroll_key = id.with("scroll");
    let frame_now = ctx.cumulative_frame_nr();
    if let Some((value, at)) = ctx.data(|d| d.get_temp::<(f32, u64)>(scroll_key)) {
        if frame_now.saturating_sub(at) <= 1 {
            *scroll = value;
        }
    }
    let mut esc = false;
    let closed = window::show(ctx, id, &window_spec, offset, false, |ui, frame| {
        // 先に描いた部品（G/R/S など）が使った Esc では閉じない
        esc = ui.input(|i| i.key_pressed(Key::Escape))
            && !crate::ui::window::escape_taken(ui.ctx())
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
        let bar = Scroll::begin(ui, list, content, scroll);
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
                vec2(list.width() - 28.0 - bar.reserved(), ROW_HEIGHT),
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
            if let Some(Some(tip)) = tips.get(i) {
                child
                    .interact(r, id.with(("tip", i)), Sense::hover())
                    .on_hover_text(tip);
            }
        }
        bar.end(ui, id.with("list-scroll"), scroll);
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
    ctx.data_mut(|d| d.insert_temp(scroll_key, (*scroll, frame_now)));
    if reply.is_none() && (closed || esc) {
        reply = Some(Reply::Closed);
    }
    reply
}

/// 毎フレーム: ウィンドウと仕事の札を描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    // 別のスレッドの仕事が動いている間は描き直し続ける（進み具合・終わりを受ける）
    if crate::jobs::repaint_needed(app) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    bake::window::show(ctx, app);
    crate::panels::brush_detail::show(ctx, app);
    crate::panels::brush_catalog::show(ctx, app);
    crate::panels::brush_clipstudio::show(ctx, app);
    crate::newproject::window::show(ctx, app);
    export::window::show(ctx, app);
    export_confirm(ctx, app);
    export_report(ctx, app);
    psd_confirm(ctx, app);
    crate::psd_import::show_check(ctx, app);
    crate::psd_export::show_options(ctx, app);
    crate::psd_export::show_confirm(ctx, app);
    crate::distribute::window::show(ctx, app);
    crate::distribute::window::show_replace(ctx, app);
    psd_report(ctx, app);
    merge_confirm(ctx, app);
    crate::update::window::show(ctx, app);
    job_card(ctx, app);
    saving_before_close(ctx, app);
    crate::bake::overlap::poll_menu(ctx, app);
    app.release_idle_bake_input();
}

/// 確認のウィンドウや結果のウィンドウが開いている（キーの割り当てを止める。仕事の表 `jobs::JOBS` の `modal`）。
pub fn modal_open(app: &AppState) -> bool {
    crate::jobs::modal_open(app)
}

/// 終わる頼みを、保存が終わるまで待たせている（保存を捨てて閉じない）。
pub(crate) fn waiting_to_close(app: &AppState) -> bool {
    app.quit && app.is_saving()
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
    let Some(replace) = app.psd.confirm.clone() else {
        return;
    };
    let lang = app.lang;
    let summary = match replace.files.as_slice() {
        [only] => only
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        many => lang.pick(
            format!("{} 個のファイル", many.len()),
            format!("{} files", many.len()),
        ),
    };
    let spec = ListSpec {
        id: "psd-confirm",
        title: if replace.imported {
            lang.pick("取り込んだ PSD を置き換える", "Replace the Imported PSD")
        } else {
            lang.pick("置き換えるファイル", "Files to Replace")
        }
        .into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((summary, true)),
        rows: replace
            .files
            .iter()
            .map(|p| Row::text(p.display().to_string(), false))
            .collect(),
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
                    if replace.imported {
                        lang.pick(
                            "このプロジェクトを取り込んだ PSD です。書き出した PSD に置き換えます",
                            "This project was imported from this PSD. It is replaced by the exported PSD",
                        )
                    } else {
                        lang.pick(
                            "同じ名前のファイルを書き出した PSD に置き換えます",
                            "Replaces the files with the same names with the exported PSDs",
                        )
                    }
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

/// 見た目が丸めの許容差を超えて変わる結合の確かめ（変わるチャンネルの名前と画素数だけ。最大の差などの検査値は出さない）。「結合する」で許容差なしに同じ結合を行う。
fn merge_confirm(ctx: &egui::Context, app: &mut AppState) {
    let Some(confirm) = app.layer_ops.merge_confirm.clone() else {
        return;
    };
    let lang = app.lang;
    let rows: Vec<Row> = confirm
        .channels
        .iter()
        .map(|(channel, pixels)| Row {
            left: crate::m2::channel_name(lang, &app.doc, *channel),
            middle: String::new(),
            right: lang.pick(format!("{pixels} 画素"), format!("{pixels} px")),
            warning: false,
        })
        .collect();
    let spec = ListSpec {
        id: "merge-confirm",
        title: lang.pick("レイヤーの結合", "Merge Layers").into(),
        icon: "warning",
        modal: true,
        width: 420.0,
        summary: Some((
            lang.pick("見た目が変わります", "The look changes").into(),
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
                label: lang.pick("結合する", "Merge").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "見た目が変わっても結合します（取り消しでレイヤーが戻ります）",
                        "Merges even though the look changes (Undo brings the layers back)",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.layer_ops.confirm_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.layer_ops.confirm_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::M2(crate::m2::Edit::ConfirmMerge)),
        Some(_) => app.apply(Action::M2(crate::m2::Edit::CancelMerge)),
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
    let tips: Vec<Option<String>> = report.lines.iter().map(|l| l.tooltip.clone()).collect();
    let mut offset = app.psd.report_offset;
    let mut scroll = 0.0;
    let reply = show_list_with(ctx, &spec, &tips, &mut offset, &mut scroll);
    app.psd.report_offset = offset;
    if reply.is_some() {
        app.apply(Action::Psd(PsdAction::DismissReport));
    }
}

/// 長い仕事（ベイクのウィンドウを閉じているあいだのベイク・書き出し・PSD など。仕事の表 `jobs::JOBS` の `card`）の札。右下に出し、
/// 進み具合と取消を見せる。
fn job_card(ctx: &egui::Context, app: &mut AppState) {
    let lang: Lang = app.lang;
    let entries = crate::jobs::cards(app, lang);
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
            for (i, (id, e)) in entries.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(rect.left() + 10.0, rect.top() + 4.0 + i as f32 * row_h),
                    vec2(rect.width() - 20.0, row_h),
                );
                let button = Rect::from_min_size(
                    pos2(row.right() - 24.0, row.top() + 4.0),
                    vec2(24.0, 24.0),
                );
                // 取り消せない仕事（保存）は、取消のボタンの場所まで使う
                let right = if e.cancel.is_some() {
                    button.left() - 6.0
                } else {
                    row.right()
                };
                let text_rect = Rect::from_min_max(row.min, pos2(right, row.top() + 18.0));
                let shown = w::fit(&p, &e.text, text_rect.width(), t::LABEL_DIM);
                w::text(&p, text_rect, &shown, t::LABEL_DIM, Align::Left);
                let bar = Rect::from_min_max(
                    pos2(row.left(), row.top() + 22.0),
                    pos2(right, row.top() + 28.0),
                );
                // 割合が分かる仕事は左から埋め、終わりの分からない仕事は往復する帯
                w::progress_bar(&p, bar, e.fraction, ctx.input(|i| i.time));
                let Some(action) = &e.cancel else {
                    continue;
                };
                if w::icon_button(
                    ui,
                    button,
                    ("yolu.job.cancel", *id),
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
                    cancel = Some(action.clone());
                }
            }
        });
    if let Some(action) = cancel {
        app.apply(action);
    }
}

/// 終わる頼みを待たせているあいだの小さなウィンドウ（取り消しのボタンは無い）。保存が終わると消え、その結果の後の状態で終わる（保存していない
/// 変更が残っていれば、そのとき聞く）。下の部品へは入力を渡さない。
fn saving_before_close(ctx: &egui::Context, app: &AppState) {
    if !waiting_to_close(app) {
        return;
    }
    let Some(progress) = app.save_progress() else {
        return;
    };
    let lang = app.lang;
    let id = Id::new("yolu.window.saving-before-close");
    let screen = ctx.content_rect();
    egui::Area::new(id.with("blocker"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.interact(screen, id.with("blocker-hit"), Sense::click_and_drag());
            ui.painter()
                .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));
        });
    let rect = Rect::from_center_size(screen.center(), vec2(320.0, 84.0));
    // 最後に描いたウィンドウの矩形（試験が位置を知るために読む。`window::last_rect`）
    ctx.data_mut(|d| d.insert_temp(id.with("rect"), rect));
    window::note_open(ctx);
    egui::Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            w::rounded(&p, rect, t::PANEL_BG, 6.0);
            w::outline(&p, rect, t::SEPARATOR, 1.0, 6.0);
            let inner = rect.shrink2(vec2(16.0, 12.0));
            let title = lang.pick("保存しています", "Saving");
            let head = Rect::from_min_size(inner.min, vec2(inner.width(), 22.0));
            w::text(&p, head, title, t::HEADER, Align::Left);
            let name =
                Rect::from_min_size(pos2(inner.left(), head.bottom()), vec2(inner.width(), 18.0));
            let shown = w::fit(&p, &progress.file, name.width(), t::LABEL_DIM);
            w::text(&p, name, &shown, t::LABEL_DIM, Align::Left);
            let bar = Rect::from_min_size(
                pos2(inner.left(), name.bottom() + 6.0),
                vec2(inner.width(), 6.0),
            );
            w::rounded(&p, bar, t::CONTROL_BG, 3.0);
            w::rounded(
                &p,
                Rect::from_min_size(
                    bar.min,
                    vec2(
                        bar.width() * progress.fraction.clamp(0.0, 1.0),
                        bar.height(),
                    ),
                ),
                t::ACCENT,
                3.0,
            );
        });
}

/// 終わる頼みを待たせている小さなウィンドウの、最後に描いた矩形（試験が読む）。
pub fn saving_window_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, Id::new("yolu.window.saving-before-close"))
}

/// 試験用: 別のスレッドの仕事を、取消が来るまで始めずに止めておく（取消・終了前の取消が効いたことを、仕事の速さに頼らず
/// 確かめるため。止めた仕事は取消を見てすぐ止まる）。
#[doc(hidden)]
pub fn park_until_canceled(cancel: &std::sync::atomic::AtomicBool) {
    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// 閉じる前に知らせる仕事（走っていて、利用者が結果を待っている物）。閉じると取り消される。
/// 保存は含まない（保存は閉じる流れが終わるまで待つ。`YoluApp::close_flow`）。画面に出ない裏の仕事（サムネイル・一覧の読み込み・
/// 更新の確かめ）は、結果を待っていないので含まない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseJob {
    Bake,
    Export,
    PsdImport,
    PsdExport,
    Distribute,
    UpdateDownload,
    BrushImport,
    /// ライブラリのフォルダへの書き込み（ライブラリへ入れる・ファイルを足す）。
    LibraryWrite,
    /// レイヤーの素材（スマートマテリアル・マスク）の保存。
    ShelfSave,
    /// 個人のライブラリのファイルのプロジェクトへの取り込み。
    ShelfImport,
}

impl CloseJob {
    /// 確かめの文に出す名前（仕事の札・メニューの名前と同じ言い方）。
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            CloseJob::Bake => lang.pick("ベイク", "Bake"),
            CloseJob::Export => lang.pick("書き出し", "Export"),
            CloseJob::PsdImport => lang.pick("PSD の取り込み", "PSD import"),
            CloseJob::PsdExport => lang.pick("PSD の書き出し", "PSD export"),
            CloseJob::Distribute => lang.pick("配布用に保存", "Save for distribution"),
            CloseJob::UpdateDownload => lang.pick("更新のダウンロード", "Update download"),
            CloseJob::BrushImport => lang.pick("ブラシの取り込み", "Brush import"),
            CloseJob::LibraryWrite => lang.pick("ライブラリへの書き込み", "Writing to the library"),
            CloseJob::ShelfSave => lang.pick("素材の保存", "Saving a material"),
            CloseJob::ShelfImport => lang.pick("素材の取り込み", "Importing a material"),
        }
    }
}

/// 閉じると取り消される、走っている仕事（仕事の表 `jobs::JOBS` の順）。保存は含まない。更新のために終わるとき（更新のダウンロードは
/// 済んでいる）も、走っている物は挙げる（呼び手が、更新の流れでは聞かない）。
pub fn close_jobs(app: &AppState) -> Vec<CloseJob> {
    crate::jobs::close_jobs(app)
}

/// 閉じる前の確かめの文。保存していない変更（`modified`）と、閉じると取り消される仕事（`jobs`）を、1 つの問いにまとめる。
/// 変更も仕事も無ければ空（問わない）。
pub fn close_question(lang: Lang, modified: bool, jobs: &[CloseJob]) -> String {
    let names = jobs
        .iter()
        .map(|job| job.label(lang))
        .collect::<Vec<_>>()
        .join(lang.pick("・", ", "));
    match (modified, jobs.is_empty()) {
        (false, true) => String::new(),
        (true, true) => lang
            .pick(
                "保存していない変更があります。変更を捨てて終わりますか？",
                "There are unsaved changes. Discard them and quit?",
            )
            .to_owned(),
        (false, false) => lang.pick(
            format!("走っている仕事（{names}）は取り消されます。終わりますか？"),
            format!("Running jobs ({names}) will be cancelled. Quit?"),
        ),
        (true, false) => lang.pick(
            format!("保存していない変更があります。走っている仕事（{names}）も取り消されます。変更を捨てて終わりますか？"),
            format!("There are unsaved changes, and running jobs ({names}) will be cancelled. Discard and quit?"),
        ),
    }
}

/// 終わる前に、走っている仕事（ベイク・書き出し・PSD・配布用に保存・ブラシの取り込み・更新・ライブラリと素材の書き込み・FBX の読み込み。
/// 仕事の表 `jobs::JOBS` の `cancel`）を取り消して、止まるのを少し待つ（書きかけの一時ファイルを残さないため。取消は次の区切りで効くので、
/// 待つのは `wait` まで）。FBX の読み込みは読むだけの仕事（何も書かない）なので、閉じる前の確かめ（`close_jobs`）には入れない。ここで
/// 止めて、メモリを使い続けない。
pub fn stop_jobs(app: &mut AppState, wait: std::time::Duration) {
    crate::jobs::stop(app, wait);
}
