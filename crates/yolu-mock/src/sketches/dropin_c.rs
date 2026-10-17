//! ドロップの入れ先 案 C: ファイルの一覧のウィンドウ（1 つだけ落としたときも、同じ一覧で行が 1 つ）。ファイルごとの行に、入れ先を 1 つの選びで出す。
//! 入れ先の既定は何も選んでいない形（空の欄。読めないファイルの行と違って押せる）で、読めるファイルの入れ先が全部決まるまで「取り込む」は押せない。
//! 開いた一覧は、頭の「アセットに入れるだけ」の行（レイヤーにはせず、アセットの欄に入る。PNG 以外の画像も、取り込むときに PNG にしてアセットに入る。PSD の行では押せない）、
//! その下にテクスチャセットごとの見出し（選ぶと新しいレイヤーセットとして入る）とレイヤーセットの行（選ぶと、その一番上に入る）。
//! 上に全部の行へ当てる「すべて」の行。PSD の行は、今の取り込みの確かめの一覧（落とす・変わる・無視）を、行の下に開いて出す。

use egui::{pos2, vec2, Id, Rect, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};
use yolu_app::ui::window::HEADER_HEIGHT;

use super::dropin_common::{self as c, Dest, Pair};
use crate::page::SketchCtx;

struct File {
    name: &'static str,
    /// 形式と大きさ（日本語・英語で同じ）。
    info: &'static str,
    psd: bool,
    /// 読めないときの短い理由。
    refused: Option<Pair>,
    /// アセットに入れられるか（画像は PNG にしてアセットに入る。PSD は入れられない）。
    asset_ok: bool,
    /// 「全部決まった形」の入れ先。
    ready: Option<Dest>,
}

const FILES: [File; 6] = [
    File {
        name: "body.psd",
        info: "PSD",
        psd: true,
        refused: None,
        asset_ok: false,
        ready: Some(Dest::Set(0, 0)),
    },
    File {
        name: "skin_base.png",
        info: "PNG · 2048 × 2048",
        psd: false,
        refused: None,
        asset_ok: true,
        ready: Some(Dest::Set(0, 0)),
    },
    File {
        name: "skin_shadow.tga",
        info: "TGA · 2048 × 2048",
        psd: false,
        refused: None,
        asset_ok: true,
        // レイヤーにはせず、アセットに入れるだけ（PNG にして入る）
        ready: Some(Dest::Asset),
    },
    File {
        name: "cloth_base.webp",
        info: "WebP · 1024 × 1024",
        psd: false,
        refused: None,
        asset_ok: true,
        // テクスチャセットの行を選んだ形（新しいレイヤーセットとして入る）
        ready: Some(Dest::NewSet(1)),
    },
    File {
        name: "hair_mask.exr",
        info: "EXR · 2048 × 2048",
        psd: false,
        refused: None,
        asset_ok: true,
        ready: Some(Dest::Set(2, 1)),
    },
    File {
        name: "notes.txt",
        info: "",
        psd: false,
        refused: Some(("未対応の形式", "Unsupported format")),
        asset_ok: false,
        ready: None,
    },
];

/// 下書き 1 枚の違い。
struct Config {
    /// 並べるファイル（`FILES` の番号）。
    files: &'static [usize],
    /// 入れ先が全部決まった形か（false は何も選んでいない形）。
    ready: bool,
    /// PSD の行の下に、取り込みの確かめの一覧を開いているか。
    psd_open: bool,
    /// 入れ先の一覧を開いた形で撮るファイル（`FILES` の番号）。
    menu: Option<usize>,
    name: &'static str,
}

const ALL: &[usize] = &[0, 1, 2, 3, 4, 5];

/// 何も選んでいない形（「取り込む」は押せない）。
pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    list(
        ui,
        ctx,
        &Config {
            files: ALL,
            ready: false,
            psd_open: false,
            menu: None,
            name: "c",
        },
    );
}

/// 全部決まった形（「取り込む」が押せる）。
pub fn draw_ready(ui: &mut Ui, ctx: &mut SketchCtx) {
    list(
        ui,
        ctx,
        &Config {
            files: ALL,
            ready: true,
            psd_open: false,
            menu: None,
            name: "c-ready",
        },
    );
}

/// 入れ先の一覧を開いた形（画像の行。アセットの行が押せる）。
pub fn draw_menu(ui: &mut Ui, ctx: &mut SketchCtx) {
    list(
        ui,
        ctx,
        &Config {
            files: ALL,
            ready: true,
            psd_open: false,
            menu: Some(1),
            name: "c-menu",
        },
    );
}

/// 入れ先の一覧を開いた形（PSD の行。アセットの行は押せず、短い理由「PSD は不可」が出る）。
pub fn draw_menu_psd(ui: &mut Ui, ctx: &mut SketchCtx) {
    list(
        ui,
        ctx,
        &Config {
            files: ALL,
            ready: true,
            psd_open: false,
            menu: Some(0),
            name: "c-menu-psd",
        },
    );
}

/// PSD 1 つだけ落とした形（行が 1 つ。取り込みの確かめの一覧を行の下に開いている）。
pub fn draw_psd(ui: &mut Ui, ctx: &mut SketchCtx) {
    list(
        ui,
        ctx,
        &Config {
            files: &[0],
            ready: false,
            psd_open: true,
            menu: None,
            name: "c-psd",
        },
    );
}

const ROW_H: f32 = 44.0;
const NOTE_H: f32 = 22.0;
const DROP_W: f32 = 300.0;

fn file_key(i: usize) -> String {
    format!("c.{i}")
}

fn list(ui: &mut Ui, ctx: &mut SketchCtx, config: &Config) {
    ui.advance_cursor_after_rect(ui.max_rect());
    c::backdrop(ui, ctx);
    if !ctx.values.bool("c.init", false) {
        for (i, f) in FILES.iter().enumerate() {
            c::dest_set(ctx, &file_key(i), config.ready.then_some(f.ready).flatten());
        }
        c::dest_set(ctx, "c.all", None);
        ctx.values.set_bool("c.psd-open", config.psd_open);
        ctx.values.set_bool("c.init", true);
    }
    let notes = psd_rows(ctx);
    let psd_open = ctx.values.bool("c.psd-open", false);
    let note_block = notes.len() as f32 * NOTE_H + 10.0;
    let psd_rows_count = config.files.iter().filter(|&&i| FILES[i].psd).count();
    let mut height = HEADER_HEIGHT
        + 12.0
        + 18.0
        + 34.0
        + 8.0
        + config.files.len() as f32 * ROW_H
        + 10.0
        + c::FOOTER;
    if psd_open {
        height += psd_rows_count as f32 * note_block;
    }
    // 読めるファイルの入れ先が全部決まったら「取り込む」を押せる
    let all_set = config
        .files
        .iter()
        .filter(|&&i| FILES[i].refused.is_none())
        .all(|&i| c::dest_get(ctx, &file_key(i)).is_some());
    let title = ctx.pick("ファイルの取り込み", "Import Files");
    c::show_window(
        ui,
        ctx,
        config.name,
        title,
        "import",
        vec2(720.0, height),
        |ui, frame, ctx| {
            let body = frame.body;
            let p = ui.painter().clone();
            let left = body.left() + 30.0;
            let right = body.right() - 14.0;
            // 列: [開閉 16][サムネイル 36][名前・情報][入れ先][閉じる 24]
            let close_x = right - 24.0;
            let drop_x = close_x - 8.0 - DROP_W;
            let name_x = left + 36.0 + 10.0;

            // 列の見出しと「すべて」
            let y0 = body.top() + 12.0;
            w::text(
                &p,
                Rect::from_min_size(pos2(drop_x + 2.0, y0), vec2(DROP_W, 18.0)),
                ctx.pick("入れ先", "Destination"),
                t::LABEL_DIM,
                Align::Left,
            );
            let all_y = y0 + 18.0;
            w::text(
                &p,
                Rect::from_min_size(pos2(name_x, all_y), vec2(drop_x - name_x - 8.0, 30.0)),
                ctx.pick("すべて", "All"),
                t::LABEL_BOLD,
                Align::Left,
            );
            let all = Rect::from_min_size(pos2(drop_x, all_y + 2.0), vec2(DROP_W, 26.0));
            if let Some(dest) = c::dest_choice(ui, ctx, all, "c.all", None, 0.0, true, true, false)
            {
                for &k in config.files {
                    let f = &FILES[k];
                    // PSD の行には、アセットを当てない
                    if f.refused.is_none() && (dest != Dest::Asset || f.asset_ok) {
                        c::dest_set(ctx, &file_key(k), Some(dest));
                    }
                }
            }

            // ファイルの行
            let mut y = all_y + 34.0 + 8.0;
            w::hline(&p, body.left(), body.right(), y - 4.0, t::BORDER);
            for &i in config.files {
                let f = &FILES[i];
                let row = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), ROW_H));
                y += ROW_H;
                w::hline(&p, row.left(), row.right(), row.bottom() - 1.0, t::BORDER);
                if f.psd {
                    let open = ctx.values.bool("c.psd-open", false);
                    let fold = Rect::from_min_size(
                        pos2(body.left() + 6.0, row.top() + 9.0),
                        vec2(20.0, 26.0),
                    );
                    if w::icon_button(
                        ui,
                        fold,
                        ("c.fold", i),
                        if open { "expand_more" } else { "chevron_right" },
                        ctx.pick("取り込みの確かめ", "Import Check"),
                        false,
                        true,
                        15.0,
                    )
                    .clicked()
                    {
                        ctx.values.set_bool("c.psd-open", !open);
                    }
                }
                let thumb = Rect::from_min_size(pos2(left, row.top() + 4.0), vec2(36.0, 36.0));
                if f.refused.is_some() {
                    w::rounded(&p, thumb, t::CONTROL_BG, 3.0);
                    w::icon(&p, thumb, "warning", t::WARNING, 18.0);
                } else if f.psd {
                    w::rounded(&p, thumb, t::CONTROL_BG, 3.0);
                    w::outline(&p, thumb, t::BORDER, 1.0, 3.0);
                    w::icon(&p, thumb, "layers", t::TEXT, 20.0);
                } else {
                    c::image_thumb(&p, thumb);
                }
                let text_w = drop_x - name_x - 12.0;
                w::text(
                    &p,
                    Rect::from_min_size(pos2(name_x, row.top() + 3.0), vec2(text_w, 22.0)),
                    f.name,
                    t::LABEL.with_color(if f.refused.is_some() {
                        t::TEXT_DIM
                    } else {
                        t::TEXT
                    }),
                    Align::Left,
                );
                // 2 行目: 形式と大きさ。PSD は、取り込みの確かめの状態（落とす・変わる・無視）も続ける
                let psd_state = ctx.pick(
                    "落とす 1 · 変わる 1 · 無視 2",
                    "Dropped 1 · Changed 1 · Ignored 2",
                );
                let (info, info_style) = match (f.refused, f.psd) {
                    (Some(why), _) => (
                        ctx.pick(why.0, why.1).to_owned(),
                        t::LABEL_DIM.with_color(t::WARNING),
                    ),
                    (None, true) => (
                        format!("{} · {psd_state}", f.info),
                        t::LABEL_DIM.with_color(t::WARNING),
                    ),
                    (None, false) => (f.info.to_owned(), t::LABEL_DIM),
                };
                let info = w::fit(&p, &info, text_w, info_style);
                w::text(
                    &p,
                    Rect::from_min_size(pos2(name_x, row.top() + 21.0), vec2(text_w, 20.0)),
                    &info,
                    info_style,
                    Align::Left,
                );
                let dest_r = Rect::from_min_size(pos2(drop_x, row.top() + 9.0), vec2(DROP_W, 26.0));
                let _ = c::dest_choice(
                    ui,
                    ctx,
                    dest_r,
                    &file_key(i),
                    None,
                    0.0,
                    f.refused.is_none(),
                    f.asset_ok,
                    config.menu == Some(i),
                );
                let close = Rect::from_min_size(pos2(close_x, row.top() + 9.0), vec2(24.0, 26.0));
                let _ = w::icon_button(
                    ui,
                    close,
                    ("c.remove", i),
                    "close",
                    ctx.pick("一覧から外す", "Remove from List"),
                    false,
                    true,
                    14.0,
                );
                // PSD の取り込みの確かめの一覧（今の取り込みの確かめと同じ列）を、行の下に開く
                if f.psd && ctx.values.bool("c.psd-open", false) {
                    let area = Rect::from_min_size(
                        pos2(name_x, y + 2.0),
                        vec2(close_x - name_x, note_block - 4.0),
                    );
                    w::fill(
                        &p,
                        Rect::from_min_max(
                            pos2(body.left(), y),
                            pos2(body.right(), y + note_block - 1.0),
                        ),
                        t::CONTROL_BG,
                    );
                    psd_notes(&p, &notes, area);
                    w::hline(
                        &p,
                        body.left(),
                        body.right(),
                        y + note_block - 1.0,
                        t::BORDER,
                    );
                    y += note_block;
                }
            }
            let _ = c::footer(
                ui,
                Id::new(("dropin.c", config.name)),
                body,
                &[
                    (ctx.pick(c::CANCEL.0, c::CANCEL.1), false, true),
                    (ctx.pick(c::IMPORT.0, c::IMPORT.1), true, all_set),
                ],
            );
        },
    );
}

/// 取り込みの確かめの一覧（今の取り込みの確かめと同じ列: レイヤーの名前・機能・扱い）。
fn psd_notes(p: &egui::Painter, rows: &[PsdRow], area: Rect) {
    let right_w = rows
        .iter()
        .map(|r| w::text_width(p, &r.right, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    let mid_w = rows
        .iter()
        .map(|r| w::text_width(p, &r.middle, t::LABEL_DIM))
        .fold(0.0f32, f32::max);
    for (i, row) in rows.iter().enumerate() {
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
        let color = if row.warning { t::WARNING } else { t::TEXT };
        let shown = w::fit(p, &row.left, left.width(), t::LABEL);
        w::text(p, left, &shown, t::LABEL.with_color(color), Align::Left);
        w::text(p, middle, &row.middle, t::LABEL_DIM, Align::Left);
        w::text(p, right, &row.right, t::LABEL_DIM, Align::Right);
    }
}

struct PsdRow {
    left: String,
    middle: String,
    right: String,
    warning: bool,
}

fn psd_rows(ctx: &SketchCtx) -> Vec<PsdRow> {
    let row = |left: &str, middle: &str, right: &str, warning: bool| PsdRow {
        left: left.to_owned(),
        middle: middle.to_owned(),
        right: right.to_owned(),
        warning,
    };
    vec![
        row(
            ctx.pick("調整 1", "Adjustment 1"),
            ctx.pick("調整レイヤー: 露光量", "Adjustment layer: Exposure"),
            ctx.pick("落とす", "Dropped"),
            true,
        ),
        row(
            ctx.pick(
                "レイヤー 4、レイヤー 9 ほか 2 レイヤー",
                "Layer 4, Layer 9 and 2 more",
            ),
            ctx.pick("合成モード: ディゾルブ", "Blend mode: Dissolve"),
            ctx.pick("変わる", "Changed"),
            true,
        ),
        row(
            "",
            ctx.pick("画像リソース 3 件", "Image resources: 3"),
            ctx.pick("無視", "Ignored"),
            false,
        ),
        row(
            "",
            ctx.pick(
                "統合画像との差: 0.4% の画素・最大 3",
                "Differs from the merged image: 0.4% of pixels, max 3",
            ),
            ctx.pick("無視", "Ignored"),
            false,
        ),
    ]
}
