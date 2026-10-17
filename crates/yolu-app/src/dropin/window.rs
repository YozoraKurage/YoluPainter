//! ファイルの取り込みのウィンドウ: 列の見出し「入れ先」・「すべて」の入れ先・ファイルごとの行（サムネイル・名前・形式と大きさ・入れ先・外す）、
//! PSD の行の下に開く取り込みの確かめ（落とす・変わる・無視）、下の帯の「やめる」「取り込む」。入れ先の一覧は 1 つ: 頭に「アセットに入れるだけ」、
//! 続けてテクスチャセットごとの見出し（新しいレイヤーセット）と、その下に字下げのレイヤーセット。入れられない行は押せず、短い理由を右に出す。

use egui::{pos2, vec2, Color32, Id, Key, Order, Rect, Sense, UiBuilder, WidgetInfo, WidgetType};

use super::{Content, Dest, DropOp, MenuFor, Row, WINDOW};
use crate::lang::Lang;
use crate::state::{ui_store, Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec, HEADER_HEIGHT};

const WIDTH: f32 = 720.0;
const ROW_H: f32 = 44.0;
const NOTE_H: f32 = 22.0;
const DROP_W: f32 = 300.0;
const FOOTER: f32 = 48.0;
/// 行の一覧の高さの上限（越えたらずらす）。
const MAX_LIST: f32 = 8.0 * ROW_H;
const POPUP_ROW: f32 = 28.0;
const POPUP_PAD: f32 = 6.0;

/// 行の高さ（PSD の確かめを開いていれば、その分も）。
fn row_height(row: &Row) -> f32 {
    match &row.content {
        Content::Psd(psd) if row.open => ROW_H + note_block(psd.notes().len()),
        _ => ROW_H,
    }
}

fn note_block(notes: usize) -> f32 {
    notes as f32 * NOTE_H + 10.0
}

/// 入れ先の一覧の 1 行。
struct Choice {
    dest: Dest,
    /// 字下げの段（0 は見出し）。
    depth: usize,
    label: String,
    /// 押せない短い理由（押せるなら None。理由を出さずに押せない行は空の文字）。
    blocked: Option<String>,
}

/// 箱に出す入れ先の名前: 「肌 › 影のマスク」「肌 › ＋新しいレイヤーセット」「アセット」。
fn dest_text(app: &AppState, dest: Dest) -> String {
    let lang = app.lang;
    match dest {
        Dest::Asset => lang.t("dropin-assets").into(),
        Dest::NewSet(uid) => format!(
            "{} › {}",
            set_name(app, uid),
            lang.t("dropin-new-layer-set-mark")
        ),
        Dest::Set(uid, set) => {
            let name = app
                .session
                .sets
                .index_of(uid)
                .map(|i| crate::m2::layersets::set_label(lang, app.set_doc(i), set))
                .unwrap_or_default();
            format!("{} › {name}", set_name(app, uid))
        }
    }
}

fn set_name(app: &AppState, uid: u32) -> String {
    app.session
        .sets
        .by_uid(uid)
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

/// 一覧の行（`for_row` は行の箱、None は「すべて」の箱）。
fn choices(app: &AppState, for_row: Option<u64>) -> Vec<Choice> {
    let lang = app.lang;
    let readable: Vec<u64> = app
        .dropin
        .rows
        .iter()
        .filter(|r| r.is_readable())
        .map(|r| r.id)
        .collect();
    // 行の箱は、その行の理由。「すべて」の箱は、どれか 1 つの行に入れられれば押せる
    let blocked = |dest: Dest| -> Option<String> {
        match for_row {
            Some(id) => app.dropin_problem(id, dest),
            None => {
                let reasons: Vec<Option<String>> = readable
                    .iter()
                    .map(|id| app.dropin_problem(*id, dest))
                    .collect();
                if reasons.iter().any(Option::is_none) {
                    None
                } else {
                    Some(reasons.into_iter().flatten().next().unwrap_or_default())
                }
            }
        }
    };
    let mut out = vec![Choice {
        dest: Dest::Asset,
        depth: 0,
        label: lang.t("dropin-assets-only").into(),
        blocked: blocked(Dest::Asset),
    }];
    for (i, set) in app.session.sets.iter().enumerate() {
        let head = blocked(Dest::NewSet(set.uid));
        let doc = app.set_doc(i);
        let head_blocked = head.is_some();
        out.push(Choice {
            dest: Dest::NewSet(set.uid),
            depth: 0,
            label: set.name.clone(),
            blocked: head,
        });
        for (k, info) in doc.layer_sets().iter().enumerate() {
            let dest = Dest::Set(set.uid, info.id);
            out.push(Choice {
                dest,
                depth: 1,
                label: crate::m2::layersets::display_name(lang, doc, k),
                // 見出しで理由を出したテクスチャセットの下は、理由を重ねない
                blocked: if head_blocked {
                    Some(String::new())
                } else {
                    blocked(dest)
                },
            });
        }
    }
    out
}

/// 1 フレームの描きで集める物（押された操作・入れ先の一覧を開く箱）と、描くときの共通の値。
struct Pass {
    ops: Vec<DropOp>,
    /// 入れ先の一覧を開いている箱の矩形。
    anchor: Option<(MenuFor, Rect)>,
    menu: Option<MenuFor>,
    id: Id,
    lang: Lang,
}

/// 列の x: [開閉 20][サムネイル 36][名前・情報][入れ先][外す 24]。
#[derive(Clone, Copy)]
struct Cols {
    left: f32,
    name_x: f32,
    drop_x: f32,
    close_x: f32,
}

/// 読み終わった画像のサムネイルを GPU へ上げる（1 度だけ）。
fn upload_thumbs(ctx: &egui::Context, app: &mut AppState) {
    for row in &mut app.dropin.rows {
        if let Content::Image(img) = &mut row.content {
            if let Some(picture) = img.thumb.take() {
                let mut image = egui::ColorImage::from_rgba_unmultiplied(
                    [picture.width as usize, picture.height as usize],
                    &picture.rgba,
                );
                image.source_size = vec2(picture.width as f32, picture.height as f32);
                let _ = ui_store::load_texture(
                    ctx,
                    &img.slot,
                    "dropin-thumb",
                    image,
                    egui::TextureOptions::LINEAR,
                );
            }
        }
    }
}

/// 毎フレーム、開いているファイルの取り込みのウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.dropin.is_open() {
        return;
    }
    let lang = app.lang;
    upload_thumbs(ctx, app);
    let content_h: f32 = app.dropin.rows.iter().map(row_height).sum();
    let list_h = content_h.min(MAX_LIST);
    let height = HEADER_HEIGHT + 12.0 + 18.0 + 34.0 + 8.0 + list_h + 10.0 + FOOTER;
    let spec = Spec {
        title: lang.t("dropin-import-files"),
        icon: Some("import"),
        size: vec2(WIDTH, height),
        modal: true,
        close_label: lang.t("dropin-close-window"),
    };
    let mut pass = Pass {
        ops: Vec::new(),
        anchor: None,
        menu: app.dropin.menu,
        id: Id::new(("yolu.window", WINDOW)),
        lang,
    };
    let mut offset = app.dropin.offset;
    let mut scroll = app.dropin.scroll;
    let ready = app.dropin.ready();
    let esc = pass.menu.is_none()
        && ctx.input(|i| i.key_pressed(Key::Escape))
        && !crate::ui::window::escape_taken(ctx);
    let app_ref: &AppState = app;
    let closed = window::show(ctx, pass.id, &spec, &mut offset, false, |ui, frame| {
        let body = frame.body;
        let left = body.left() + 30.0;
        let close_x = body.right() - 14.0 - 24.0;
        let cols = Cols {
            left,
            name_x: left + 36.0 + 10.0,
            drop_x: close_x - 8.0 - DROP_W,
            close_x,
        };
        let all_y = head(ui, app_ref, &mut pass, body, cols);
        let top = all_y + 34.0 + 8.0;
        let list = Rect::from_min_size(pos2(body.left(), top), vec2(body.width(), list_h));
        file_rows(
            ui,
            ctx,
            app_ref,
            &mut pass,
            (body, cols),
            (list, content_h),
            &mut scroll,
        );
        footer(ui, &mut pass, body, ready);
    });
    app.dropin.offset = offset;
    app.dropin.scroll = scroll;
    open_popup(ctx, app, &mut pass);
    if (closed || esc) && pass.ops.is_empty() {
        pass.ops.push(DropOp::Cancel);
    }
    for op in pass.ops {
        app.apply(Action::DropIn(op));
    }
}

/// 列の見出しと「すべて」。「すべて」の行の y を返す。
fn head(ui: &mut egui::Ui, app: &AppState, pass: &mut Pass, body: Rect, cols: Cols) -> f32 {
    let lang = pass.lang;
    let p = ui.painter().clone();
    let y0 = body.top() + 12.0;
    w::text(
        &p,
        Rect::from_min_size(pos2(cols.drop_x + 2.0, y0), vec2(DROP_W, 18.0)),
        lang.t("dropin-destination"),
        t::LABEL_DIM,
        Align::Left,
    );
    let all_y = y0 + 18.0;
    w::text(
        &p,
        Rect::from_min_size(
            pos2(cols.name_x, all_y),
            vec2(cols.drop_x - cols.name_x - 8.0, 30.0),
        ),
        lang.t("dropin-all"),
        t::LABEL_BOLD,
        Align::Left,
    );
    let all_rect = Rect::from_min_size(pos2(cols.drop_x, all_y + 2.0), vec2(DROP_W, 26.0));
    let shown = app
        .dropin
        .all
        .map(|d| dest_text(app, d))
        .unwrap_or_default();
    let (response, b) = w::dropdown(
        ui,
        all_rect,
        "dropin.all",
        None,
        &shown,
        None,
        app.dropin.rows.iter().any(Row::is_readable),
        0.0,
    );
    if response.clicked() {
        pass.ops.push(DropOp::Menu(Some(MenuFor::All)));
    }
    if pass.menu == Some(MenuFor::All) {
        pass.anchor = Some((MenuFor::All, b));
    }
    all_y
}

/// ファイルの行の一覧（ずらせる）。
fn file_rows(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &AppState,
    pass: &mut Pass,
    (body, cols): (Rect, Cols),
    (list, content_h): (Rect, f32),
    scroll: &mut f32,
) {
    let p = ui.painter().clone();
    w::hline(&p, body.left(), body.right(), list.top() - 4.0, t::BORDER);
    let bar = Scroll::begin(ui, list, content_h, scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let cp = child.painter().clone();
    let cols = Cols {
        close_x: cols.close_x - bar.reserved(),
        drop_x: cols.drop_x - bar.reserved(),
        ..cols
    };
    let mut y = list.top() - *scroll;
    for row in &app.dropin.rows {
        let r = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), ROW_H));
        let full = row_height(row);
        y += full;
        if r.top() + full < list.top() || r.top() > list.bottom() {
            continue;
        }
        file_row(ctx, app, pass, (&mut child, &cp), (row, r), (body, cols));
    }
    bar.end(ui, pass.id.with("list-scroll"), scroll);
}

/// ファイルの行 1 つ（開閉・サムネイル・名前・形式と大きさ・入れ先の箱・外す・PSD の取り込みの確かめ）。
fn file_row(
    ctx: &egui::Context,
    app: &AppState,
    pass: &mut Pass,
    (child, cp): (&mut egui::Ui, &egui::Painter),
    (row, r): (&Row, Rect),
    (body, cols): (Rect, Cols),
) {
    let lang = pass.lang;
    w::hline(cp, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    if let Content::Psd(_) = &row.content {
        let fold = Rect::from_min_size(pos2(body.left() + 6.0, r.top() + 9.0), vec2(20.0, 26.0));
        if w::icon_button(
            child,
            fold,
            ("dropin.fold", row.id),
            if row.open {
                "expand_more"
            } else {
                "chevron_right"
            },
            lang.t("dropin-import-check-tip"),
            false,
            true,
            15.0,
        )
        .clicked()
        {
            pass.ops.push(DropOp::ToggleCheck(row.id));
        }
    }
    let thumb = Rect::from_min_size(pos2(cols.left, r.top() + 4.0), vec2(36.0, 36.0));
    draw_thumb(ctx, cp, thumb, row);
    let text_w = cols.drop_x - cols.name_x - 12.0;
    let dim = row.is_refused();
    let name = w::fit(cp, &row.name, text_w, t::LABEL);
    w::text(
        cp,
        Rect::from_min_size(pos2(cols.name_x, r.top() + 3.0), vec2(text_w, 22.0)),
        &name,
        t::LABEL.with_color(if dim { t::TEXT_DIM } else { t::TEXT }),
        Align::Left,
    );
    let (info, style) = info_line(lang, row);
    let shown_info = w::fit(cp, &info, text_w, style);
    let info_rect = Rect::from_min_size(pos2(cols.name_x, r.top() + 21.0), vec2(text_w, 20.0));
    w::text(cp, info_rect, &shown_info, style, Align::Left);
    if shown_info != info {
        child
            .interact(info_rect, pass.id.with(("info", row.id)), Sense::hover())
            .on_hover_text(&info);
    }
    let dest_rect = Rect::from_min_size(pos2(cols.drop_x, r.top() + 9.0), vec2(DROP_W, 26.0));
    let value = match (row.is_readable(), row.dest) {
        (true, Some(d)) => dest_text(app, d),
        _ => String::new(),
    };
    let (response, b) = w::dropdown(
        child,
        dest_rect,
        ("dropin.dest", row.id),
        None,
        &value,
        None,
        row.is_readable(),
        0.0,
    );
    if response.clicked() {
        pass.ops.push(DropOp::Menu(Some(MenuFor::Row(row.id))));
    }
    if pass.menu == Some(MenuFor::Row(row.id)) {
        pass.anchor = Some((MenuFor::Row(row.id), b));
    }
    let remove = Rect::from_min_size(pos2(cols.close_x, r.top() + 9.0), vec2(24.0, 26.0));
    if w::icon_button(
        child,
        remove,
        ("dropin.remove", row.id),
        "close",
        lang.t("dropin-remove-from-list-tip"),
        false,
        true,
        14.0,
    )
    .clicked()
    {
        pass.ops.push(DropOp::Remove(row.id));
    }
    // PSD の取り込みの確かめ（今の取り込みの確かめと同じ列）を、行の下に開く
    if let (Content::Psd(psd), true) = (&row.content, row.open) {
        let block = note_block(psd.notes().len());
        let band = Rect::from_min_max(
            pos2(body.left(), r.bottom()),
            pos2(body.right(), r.bottom() + block - 1.0),
        );
        w::fill(cp, band, t::CONTROL_BG);
        let area = Rect::from_min_size(
            pos2(cols.name_x, r.bottom() + 4.0),
            vec2(cols.close_x + 24.0 - cols.name_x, block - 8.0),
        );
        notes(
            child,
            lang,
            psd.notes(),
            area,
            pass.id.with(("notes", row.id)),
        );
        w::hline(cp, body.left(), body.right(), band.bottom(), t::BORDER);
    }
}

/// 下の帯の「やめる」「取り込む」。
fn footer(ui: &mut egui::Ui, pass: &mut Pass, body: Rect, ready: bool) {
    let lang = pass.lang;
    let p = ui.painter().clone();
    let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
    w::fill(&p, footer, t::PANEL_HEADER);
    w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
    let mut x = footer.right() - 14.0;
    let buttons = [
        (lang.t("dropin-cancel"), false, true, DropOp::Cancel),
        (lang.t("dropin-import"), true, ready, DropOp::Import),
    ];
    for (i, (label, primary, enabled, op)) in buttons.into_iter().enumerate().rev() {
        let bw = w::text_width(&p, label, t::LABEL) + 32.0;
        let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
        x = r.left() - 8.0;
        if w::button(
            ui,
            r,
            pass.id.with(("button", i)),
            label,
            primary,
            enabled,
            None,
            None,
        )
        .clicked()
        {
            pass.ops.push(op);
        }
    }
}

/// 開いている箱の入れ先の一覧を描き、選んだ・閉じた操作を集める。
fn open_popup(ctx: &egui::Context, app: &AppState, pass: &mut Pass) {
    let Some((which, b)) = pass.anchor else {
        return;
    };
    let for_row = match which {
        MenuFor::All => None,
        MenuFor::Row(id) => Some(id),
    };
    let current = match which {
        MenuFor::All => app.dropin.all,
        MenuFor::Row(id) => app.dropin.row(id).and_then(|r| r.dest),
    };
    let rows = choices(app, for_row);
    match popup(ctx, pass.id.with(("popup", for_row)), b, &rows, current) {
        Popup::Open => {}
        Popup::Close => pass.ops.push(DropOp::Menu(None)),
        Popup::Chosen(dest) => pass.ops.push(match which {
            MenuFor::All => DropOp::SetAll(dest),
            MenuFor::Row(id) => DropOp::SetDest(id, dest),
        }),
    }
}

/// 2 行目: 形式と大きさ。PSD は取り込みの確かめの件数を続ける。読めない行は形式と理由。
fn info_line(lang: Lang, row: &Row) -> (String, crate::ui::theme::TextStyle) {
    match &row.content {
        Content::Reading => (super::format_name(&row.path), t::LABEL_DIM),
        Content::Image(img) => (
            format!("{} · {} × {}", img.format, img.width, img.height),
            t::LABEL_DIM,
        ),
        Content::Psd(psd) => {
            let (wd, ht) = psd.size();
            let state = crate::psd_import::summary(lang, psd.notes());
            if state.is_empty() {
                (format!("PSD · {wd} × {ht}"), t::LABEL_DIM)
            } else {
                (
                    format!("PSD · {wd} × {ht} · {state}"),
                    t::LABEL_DIM.with_color(t::WARNING),
                )
            }
        }
        Content::Refused { format, reason } => (
            if format.is_empty() {
                reason.clone()
            } else {
                format!("{format} · {reason}")
            },
            t::LABEL_DIM.with_color(t::WARNING),
        ),
    }
}

/// サムネイル: 画像は縮めた画像、PSD はレイヤーの印、読めない行は注意の印。
fn draw_thumb(ctx: &egui::Context, p: &egui::Painter, r: Rect, row: &Row) {
    match &row.content {
        Content::Image(img) => {
            w::checker(p, r, 4.0);
            let texture = ui_store::texture(ctx, &img.slot);
            if let Some(texture) = texture {
                let [tw, th] = texture.size();
                let scale = (r.width() / tw as f32).min(r.height() / th as f32);
                let fitted = Rect::from_center_size(r.center(), vec2(tw as f32, th as f32) * scale);
                p.image(
                    texture.id(),
                    fitted,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            w::outline(p, r, t::BORDER, 1.0, 0.0);
        }
        Content::Psd(_) => {
            w::rounded(p, r, t::CONTROL_BG, 3.0);
            w::outline(p, r, t::BORDER, 1.0, 3.0);
            w::icon(p, r, "layers", t::TEXT, 20.0);
        }
        Content::Reading => {
            w::rounded(p, r, t::CONTROL_BG, 3.0);
            w::outline(p, r, t::BORDER, 1.0, 3.0);
        }
        Content::Refused { .. } => {
            w::rounded(p, r, t::CONTROL_BG, 3.0);
            w::icon(p, r, "warning", t::WARNING, 18.0);
        }
    }
}

/// 取り込みの確かめの一覧（今の取り込みの確かめと同じ列: レイヤーの名前・機能・扱い。説明はツールチップ）。
fn notes(ui: &mut egui::Ui, lang: Lang, notes: &[yolu_io::psd::ImportNote], area: Rect, id: Id) {
    let p = ui.painter().clone();
    let sorted = crate::psd_import::sorted(notes);
    let rows: Vec<(String, String, &'static str, bool, Option<&'static str>)> = sorted
        .iter()
        .map(|n| {
            (
                crate::psd_import::layers_text(lang, n),
                crate::psd_import::feature_text(lang, n),
                crate::psd_import::action_text(lang, n.action),
                n.action != yolu_io::psd::ImportAction::Ignored,
                crate::psd_import::feature_tooltip(lang, n),
            )
        })
        .collect();
    let right_w = rows
        .iter()
        .map(|r| w::text_width(&p, r.2, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    let mid_w = rows
        .iter()
        .map(|r| w::text_width(&p, &r.1, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    for (i, (left_text, middle_text, right_text, warning, tip)) in rows.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(area.left(), area.top() + i as f32 * NOTE_H),
            vec2(area.width(), NOTE_H),
        );
        let right = Rect::from_min_size(
            pos2(r.right() - right_w, r.top()),
            vec2(right_w, r.height()),
        );
        let middle = Rect::from_min_size(
            pos2(right.left() - 12.0 - mid_w, r.top()),
            vec2(mid_w, r.height()),
        );
        let left = Rect::from_min_max(r.min, pos2(middle.left() - 12.0, r.bottom()));
        let color = if *warning { t::WARNING } else { t::TEXT };
        let shown = w::fit(&p, left_text, left.width().max(0.0), t::LABEL);
        w::text(&p, left, &shown, t::LABEL.with_color(color), Align::Left);
        w::text(&p, middle, middle_text, t::LABEL_DIM, Align::Left);
        w::text(&p, right, right_text, t::LABEL_DIM, Align::Right);
        if let Some(tip) = tip {
            ui.interact(r, id.with(i), Sense::hover())
                .on_hover_text(*tip);
        }
    }
}

enum Popup {
    Open,
    Chosen(Dest),
    Close,
}

/// 入れ先の一覧（箱の下に。入らなければ上）。外を押す・Esc で閉じる。
fn popup(
    ctx: &egui::Context,
    id: Id,
    anchor: Rect,
    rows: &[Choice],
    current: Option<Dest>,
) -> Popup {
    let screen = ctx.content_rect();
    let size = vec2(
        anchor.width().max(280.0),
        rows.len() as f32 * POPUP_ROW + 2.0 * POPUP_PAD,
    );
    let below = anchor.bottom() + 2.0;
    let y = if below + size.y <= screen.bottom() - 8.0 {
        below
    } else {
        (anchor.top() - 2.0 - size.y).max(screen.top() + 8.0)
    };
    let x = anchor.left().min(screen.right() - 8.0 - size.x).max(8.0);
    let rect = Rect::from_min_size(pos2(x, y), size);
    let mut result = Popup::Open;
    // 外を押したら閉じるだけで、下の部品へは渡さない
    egui::Area::new(id.with("blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            if ui
                .interact(screen, id.with("hit"), Sense::click_and_drag())
                .clicked()
            {
                result = Popup::Close;
            }
        });
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        result = Popup::Close;
    }
    egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            // 一覧の範囲を持つ（下の部品へ押しを渡さない。範囲はテストがスクリーンショットに含める）
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let p = ui.painter().clone();
            for i in (1..=5).rev() {
                let f = i as f32;
                p.rect_filled(
                    rect.expand(f).translate(vec2(0.0, 2.0)),
                    6.0 + f,
                    Color32::from_black_alpha(20),
                );
            }
            w::rounded(&p, rect, t::MENU_BG, 6.0);
            w::outline(&p, rect, t::SEPARATOR, 1.0, 6.0);
            for (i, choice) in rows.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(
                        rect.left() + 4.0,
                        rect.top() + POPUP_PAD + i as f32 * POPUP_ROW,
                    ),
                    vec2(rect.width() - 8.0, POPUP_ROW),
                );
                if choice.depth == 0 && i > 0 {
                    w::hline(
                        &p,
                        rect.left() + 8.0,
                        rect.right() - 8.0,
                        row.top() - 0.5,
                        t::BORDER,
                    );
                }
                if popup_row(ui, &p, id.with(("row", i)), row, choice, current) {
                    result = Popup::Chosen(choice.dest);
                }
            }
        });
    result
}

/// 入れ先の一覧の 1 行を描く。押せる行を押したら true。
fn popup_row(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    id: Id,
    row: Rect,
    choice: &Choice,
    current: Option<Dest>,
) -> bool {
    let usable = choice.blocked.is_none();
    let response = ui.interact(
        row,
        id,
        if usable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, usable, &choice.label));
    if current == Some(choice.dest) {
        w::rounded(p, row, t::ACCENT_SOFT, 4.0);
        w::fill(
            p,
            Rect::from_min_size(row.min + vec2(0.0, 4.0), vec2(3.0, row.height() - 8.0)),
            t::ACCENT,
        );
    } else if usable && response.hovered() {
        w::rounded(p, row, t::CONTROL_HOVER, 4.0);
    }
    let color = if usable { t::TEXT } else { t::TEXT_DISABLED };
    let reason_w = choice
        .blocked
        .as_deref()
        .map_or(0.0, |s| w::text_width(p, s, t::LABEL_DIM) + 16.0);
    match choice.dest {
        Dest::Asset => popup_asset_row(p, row, choice, color, reason_w),
        Dest::NewSet(_) => popup_head_row(p, row, choice, usable, color, reason_w),
        Dest::Set(..) => popup_layer_set_row(p, row, choice, usable, color, reason_w),
    }
    if let Some(reason) = choice.blocked.as_deref().filter(|s| !s.is_empty()) {
        w::text(
            p,
            Rect::from_min_max(row.min, pos2(row.right() - 12.0, row.bottom())),
            reason,
            t::LABEL_DIM.with_color(t::TEXT_DISABLED),
            Align::Right,
        );
    }
    usable && response.clicked()
}

/// 「アセットに入れるだけ」の行。
fn popup_asset_row(p: &egui::Painter, row: Rect, choice: &Choice, color: Color32, reason_w: f32) {
    w::icon(
        p,
        Rect::from_min_size(pos2(row.left() + 12.0, row.top()), vec2(18.0, row.height())),
        "library",
        color,
        15.0,
    );
    w::text(
        p,
        Rect::from_min_max(
            pos2(row.left() + 36.0, row.top()),
            pos2(row.right() - reason_w, row.bottom()),
        ),
        &choice.label,
        t::LABEL.with_color(color),
        Align::Left,
    );
}

/// テクスチャセットの見出しの行（選ぶと新しいレイヤーセット）。
fn popup_head_row(
    p: &egui::Painter,
    row: Rect,
    choice: &Choice,
    usable: bool,
    color: Color32,
    reason_w: f32,
) {
    let label_r = Rect::from_min_max(
        pos2(row.left() + 12.0, row.top()),
        pos2(row.right() - 28.0 - reason_w, row.bottom()),
    );
    let shown = w::fit(p, &choice.label, label_r.width().max(0.0), t::LABEL_BOLD);
    w::text(
        p,
        label_r,
        &shown,
        t::LABEL_BOLD.with_color(color),
        Align::Left,
    );
    if usable {
        w::icon(
            p,
            Rect::from_min_size(
                pos2(row.right() - 28.0, row.top()),
                vec2(20.0, row.height()),
            ),
            "add",
            t::TEXT_DIM,
            16.0,
        );
    }
}

/// レイヤーセットの行（字下げ。選ぶとそのレイヤーセットの一番上）。
fn popup_layer_set_row(
    p: &egui::Painter,
    row: Rect,
    choice: &Choice,
    usable: bool,
    color: Color32,
    reason_w: f32,
) {
    let x = row.left() + 12.0 + 18.0 * choice.depth as f32;
    w::icon(
        p,
        Rect::from_min_size(pos2(x, row.top()), vec2(18.0, row.height())),
        "layers",
        if usable {
            t::TEXT_DIM
        } else {
            t::TEXT_DISABLED
        },
        15.0,
    );
    let label_r = Rect::from_min_max(
        pos2(x + 24.0, row.top()),
        pos2(row.right() - reason_w, row.bottom()),
    );
    let shown = w::fit(p, &choice.label, label_r.width().max(0.0), t::LABEL);
    w::text(p, label_r, &shown, t::LABEL.with_color(color), Align::Left);
}
