//! 取り込みの窓の「CLIP STUDIO から」: 見つけた `.sut` の一覧（選ぶ印・筆先の見本・名前）、下の帯にフォルダを手で選ぶ・探し直す・
//! 全部選ぶ・取り込む。文字は名前・状態・短い理由だけで、説明はツールチップ（探したフォルダ・ファイル名・読めなかった理由）。
//! 窓は下の部品へ入力を渡さない（モーダル）。探す・中身を覗く仕事は別のスレッド（`brushes::clipstudio`）。

use egui::{
    pos2, vec2, Color32, Id, Key, Rect, Sense, TextureOptions, UiBuilder, Vec2, WidgetInfo,
    WidgetType,
};
use yolu_io::brushes::clipstudio::Missing;

use crate::brushes::clipstudio::{Listing, Row, RowState};
use crate::brushes::BrushAction;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 540.0;
const ROW_HEIGHT: f32 = 44.0;
const MAX_ROWS: usize = 7;
const FOOTER: f32 = 48.0;
const THUMB: f32 = 36.0;

/// 窓の名前（`windows::window_rect` で矩形を引く）。
pub const NAME: &str = "brush-clipstudio";

fn id() -> Id {
    Id::new(("yolu.window", NAME))
}

/// 一覧の 1 行の見出し（ブラシの名前。複数なら最初の名前と数、まだ読んでいなければファイル名）。
fn row_title(row: &Row) -> String {
    match &row.state {
        RowState::Ready(peek) => {
            let first = peek.names.first().cloned().unwrap_or_default();
            if peek.brushes > 1 {
                format!("{first} (+{})", peek.brushes - 1)
            } else {
                first
            }
        }
        _ => row
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// 筆先の見本の絵（塗る所の不透明度）。
fn thumbnail(ctx: &egui::Context, row: &mut Row) {
    let RowState::Ready(peek) = &row.state else {
        return;
    };
    if row.texture.is_some() {
        return;
    }
    let side = peek.preview.side as usize;
    let rgba: Vec<u8> = peek
        .preview
        .alpha
        .iter()
        .flat_map(|a| [t::TEXT.r(), t::TEXT.g(), t::TEXT.b(), *a])
        .collect();
    let image = egui::ColorImage::from_rgba_unmultiplied([side, side], &rgba);
    row.texture = Some(ctx.load_texture(
        format!("csp-tip-{}", row.path.display()),
        image,
        TextureOptions::LINEAR,
    ));
}

/// 状態の 1 行（短い名前・状態・理由だけ）と、注意の色にするか。
fn summary(app: &AppState) -> (String, bool, Option<String>) {
    let lang = app.lang;
    let csp = &app.brushes.csp;
    let Some(listing) = &csp.listing else {
        return (lang.pick("探しています", "Searching").into(), false, None);
    };
    let folders = listing
        .searched
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let tip = (!folders.is_empty()).then_some(folders);
    if let Some(missing) = listing.missing {
        let text = match missing {
            Missing::NoFolder => lang.pick(
                "サブツールのフォルダが見つかりません",
                "Sub tool folder not found",
            ),
            Missing::Unreadable => lang.pick("フォルダを開けません", "Cannot open the folder"),
            Missing::NoFiles => lang.pick("サブツールがありません", "No sub tools"),
        };
        return (text.into(), true, tip);
    }
    let n = listing.rows.len();
    let mut text = lang.pick(
        format!("サブツール {n} 個"),
        format!("{n} sub tool{}", if n == 1 { "" } else { "s" }),
    );
    if csp.is_busy() {
        text.push_str(lang.pick(" — 読み込み中", " — loading"));
    } else if listing.truncated {
        text.push_str(lang.pick(" — 一部のみ", " — partial"));
    }
    (text, false, tip)
}

/// 窓を描く（開いていなければ何もしない）。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.brushes.csp.open {
        return;
    }
    let lang = app.lang;
    let rows = app.brushes.csp.listing.as_ref().map_or(0, |l| l.rows.len());
    let visible = rows.clamp(1, MAX_ROWS);
    let height = window::HEADER_HEIGHT + 30.0 + visible as f32 * ROW_HEIGHT + 10.0 + FOOTER;
    // 見える行の見本の絵を作る（見えない行は作らない。一覧が長くても絵は 8 枚ほど）
    let first_row = (app.brushes.csp.scroll / ROW_HEIGHT).floor().max(0.0) as usize;
    if let Some(listing) = app.brushes.csp.listing.as_mut() {
        for row in listing.rows.iter_mut().skip(first_row).take(MAX_ROWS + 2) {
            thumbnail(ctx, row);
        }
    }
    let (summary_text, warning, summary_tip) = summary(app);
    let title = lang.pick("CLIP STUDIO から", "From CLIP STUDIO");
    let spec = Spec {
        title,
        icon: Some("folder_open"),
        size: vec2(WIDTH, height),
        modal: true,
        close_label: lang.pick("閉じる", "Close"),
    };
    let mut actions: Vec<BrushAction> = Vec::new();
    let mut offset: Vec2 = app.brushes.csp.offset;
    let mut scroll = app.brushes.csp.scroll;
    let window_id = id();
    let state = &*app;
    let closed = window::show(ctx, window_id, &spec, &mut offset, false, |ui, frame| {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            actions.push(BrushAction::ClipStudioClose);
        }
        let body = frame.body;
        let p = ui.painter().clone();
        let csp = &state.brushes.csp;
        // 状態の 1 行
        let line = Rect::from_min_size(
            pos2(body.left() + 14.0, body.top() + 4.0),
            vec2(body.width() - 28.0, 22.0),
        );
        let shown = w::fit(&p, &summary_text, line.width(), t::LABEL);
        w::text(
            &p,
            line,
            &shown,
            t::LABEL.with_color(if warning { t::WARNING } else { t::TEXT }),
            Align::Left,
        );
        if let Some(tip) = &summary_tip {
            let _ = ui
                .interact(line, window_id.with("summary"), Sense::hover())
                .on_hover_text(tip);
        }
        // 一覧
        let list = Rect::from_min_size(
            pos2(body.left(), body.top() + 30.0),
            vec2(body.width(), visible as f32 * ROW_HEIGHT),
        );
        if let Some(listing) = &csp.listing {
            draw_list(
                ui,
                list,
                listing,
                window_id,
                &mut scroll,
                &mut actions,
                state,
            );
        }
        // 下の帯
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        draw_footer(ui, footer, window_id, state, &mut actions);
    });
    app.brushes.csp.offset = offset;
    app.brushes.csp.scroll = scroll;
    for action in actions {
        app.apply(Action::Brush(action));
    }
    if closed {
        app.apply(Action::Brush(BrushAction::ClipStudioClose));
    }
}

fn draw_list(
    ui: &mut egui::Ui,
    list: Rect,
    listing: &Listing,
    window_id: Id,
    scroll: &mut f32,
    actions: &mut Vec<BrushAction>,
    app: &AppState,
) {
    let lang = app.lang;
    let content = listing.rows.len() as f32 * ROW_HEIGHT;
    let bar = Scroll::begin(ui, list, content, scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let cp = child.painter().clone();
    for (i, row) in listing.rows.iter().enumerate() {
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
        w::hline(&cp, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
        let thumb = Rect::from_min_size(
            pos2(r.left(), r.center().y - THUMB / 2.0),
            vec2(THUMB, THUMB),
        );
        w::fill(&cp, thumb, t::CONTROL_BG);
        if let Some(texture) = &row.texture {
            cp.image(
                texture.id(),
                thumb,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let name_left = thumb.right() + 10.0;
        let title = row_title(row);
        let hint = format!("{}\n{}", row.shown, describe_size(row.size));
        match &row.state {
            RowState::Failed(error) => {
                w::icon(
                    &cp,
                    Rect::from_min_size(pos2(name_left, r.center().y - 8.0), vec2(16.0, 16.0)),
                    "warning",
                    t::WARNING,
                    14.0,
                );
                let text_rect = Rect::from_min_max(
                    pos2(name_left + 23.0, r.top()),
                    pos2(r.right(), r.bottom()),
                );
                let shown = w::fit(&cp, &title, text_rect.width(), t::LABEL);
                w::text(
                    &cp,
                    text_rect,
                    &shown,
                    t::LABEL.with_color(t::WARNING),
                    Align::Left,
                );
                let reason = crate::brushes::import::describe_error(lang, error);
                let response = child.interact(r, window_id.with(("row", i)), Sense::hover());
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &title));
                let _ = response.on_hover_text(format!("{hint}\n{reason}"));
            }
            state => {
                let ready = matches!(state, RowState::Ready(_));
                let toggle = Rect::from_min_size(
                    pos2(name_left, r.center().y - t::ROW_HEIGHT / 2.0),
                    vec2(r.right() - name_left, t::ROW_HEIGHT),
                );
                let shown = w::fit(&cp, &title, toggle.width() - 30.0, t::LABEL);
                let next = w::toggle(
                    &mut child,
                    toggle,
                    window_id.with(("select", i)),
                    &shown,
                    row.selected,
                    Some(&hint),
                    ready,
                );
                if next != row.selected {
                    actions.push(BrushAction::ClipStudioToggle(i));
                }
            }
        }
    }
    bar.end(ui, window_id.with("list-scroll"), scroll);
}

fn describe_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

fn draw_footer(
    ui: &mut egui::Ui,
    footer: Rect,
    window_id: Id,
    app: &AppState,
    actions: &mut Vec<BrushAction>,
) {
    let lang = app.lang;
    let csp = &app.brushes.csp;
    let p = ui.painter().clone();
    let selected = csp.listing.as_ref().map_or(0, Listing::selected);
    let ready = csp.listing.as_ref().map_or(0, Listing::ready);
    let importing = app.brushes.import.is_busy();
    let button = |ui: &mut egui::Ui,
                  x: f32,
                  label: &str,
                  salt: &str,
                  primary: bool,
                  enabled: bool,
                  tip: &str|
     -> (bool, f32) {
        let width = w::text_width(&p, label, t::LABEL) + 28.0;
        let r = Rect::from_min_size(pos2(x, footer.top() + 10.0), vec2(width, 28.0));
        let clicked = w::button(
            ui,
            r,
            window_id.with(salt),
            label,
            primary,
            enabled,
            Some(tip),
            None,
        )
        .clicked();
        (clicked, width)
    };
    // 右から: 取り込む・全部選ぶ（外す）
    let import_label = if selected > 0 {
        lang.pick(
            format!("取り込む（{selected}）"),
            format!("Import ({selected})"),
        )
    } else {
        lang.pick("取り込む", "Import").to_owned()
    };
    let import_width = w::text_width(&p, &import_label, t::LABEL) + 28.0;
    let mut x = footer.right() - 14.0 - import_width;
    if button(
        ui,
        x,
        &import_label,
        "import",
        true,
        selected > 0 && !importing,
        lang.pick(
            "選んだサブツールを取り込む",
            "Import the selected sub tools",
        ),
    )
    .0
    {
        actions.push(BrushAction::ClipStudioImport);
    }
    let all = ready > 0 && selected == ready;
    let all_label = if all {
        lang.pick("選びを外す", "Clear")
    } else {
        lang.pick("すべて選ぶ", "Select All")
    };
    let all_width = w::text_width(&p, all_label, t::LABEL) + 28.0;
    x -= 8.0 + all_width;
    if button(
        ui,
        x,
        all_label,
        "all",
        false,
        ready > 0,
        lang.pick(
            "読めたサブツールを全部選ぶ・外す",
            "Select or clear every readable sub tool",
        ),
    )
    .0
    {
        actions.push(BrushAction::ClipStudioSelectAll(!all));
    }
    // 左から: フォルダ・探し直す
    let mut x = footer.left() + 14.0;
    let (picked, width) = button(
        ui,
        x,
        lang.pick("フォルダ…", "Folder…"),
        "folder",
        false,
        true,
        lang.pick(
            "CLIP STUDIO のサブツールのフォルダを選ぶ",
            "Choose the CLIP STUDIO sub tool folder",
        ),
    );
    if picked {
        actions.push(BrushAction::ClipStudioPickFolder);
    }
    x += width + 8.0;
    if button(
        ui,
        x,
        lang.pick("探し直す", "Rescan"),
        "rescan",
        false,
        true,
        lang.pick("今の場所を探し直す", "Search the same place again"),
    )
    .0
    {
        actions.push(BrushAction::ClipStudioRescan);
    }
}
