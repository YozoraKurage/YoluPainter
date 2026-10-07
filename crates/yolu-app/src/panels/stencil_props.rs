//! プロパティの欄の描くツールの「ステンシル」のタブ（Unity 版の Stencil の欄と同じ並び）: 画像（サムネイルと名前。押すと画像を読む・
//! 読んだ画像・外す。PNG のファイルを落としても読める）、読み方（自動・量（マスク）・色）、反転と繰り返し、重ねの不透明度、置き場
//! （大きさ・角度）と「置き場を戻す」。値の操作は `Action::Stencil`（文書は変えない）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Sense, Ui};
use yolu_core::{StencilMode, StencilTiling};

use super::properties::{open_popup, percent_row, section, slider_row};
use crate::lang::Lang;
use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::stencil::{StencilOp, MIN_SIZE, SLIDER_MAX_SIZE};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

/// 左の見出し（「画像」「読み方」）の幅。
const LABEL_WIDTH: f32 = 72.0;
const IMAGE_ROW: f32 = 24.0;

/// 読み方の名前（自動は、この画像で決めた読み方を括弧に短く添える）。
pub fn mode_name(lang: Lang, mode: StencilMode, resolved: Option<StencilMode>) -> String {
    match mode {
        StencilMode::Mask => lang.pick("量（マスク）", "Amount (mask)").to_owned(),
        StencilMode::Color => lang.pick("色", "Color").to_owned(),
        StencilMode::Auto => {
            let auto = lang.pick("自動", "Auto");
            match resolved {
                Some(StencilMode::Mask) => format!("{auto} ({})", lang.pick("量", "Amount")),
                Some(StencilMode::Color) => format!("{auto} ({})", lang.pick("色", "Color")),
                _ => auto.to_owned(),
            }
        }
    }
}

pub fn tiling_name(lang: Lang, tiling: StencilTiling) -> &'static str {
    match tiling {
        StencilTiling::None => lang.pick("繰り返さない", "No tiling"),
        StencilTiling::Horizontal => lang.pick("横に繰り返す", "Horizontal"),
        StencilTiling::Vertical => lang.pick("縦に繰り返す", "Vertical"),
        StencilTiling::Both => lang.pick("縦横に繰り返す", "Both"),
    }
}

/// 画像の箱を押したときのポップアップ: 画像を読む・読んだ画像・外す。
pub fn image_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let st = &app.stencil;
    let current = st.image.as_ref().and_then(|i| i.path.as_deref());
    let mut v = vec![Entry::item(
        lang.pick("画像を読む…", "Load an Image…"),
        Action::Stencil(StencilOp::Pick),
    )
    .enabled(free)];
    if !st.recent.is_empty() {
        v.push(Entry::Separator);
        for (i, r) in st.recent.iter().enumerate() {
            v.push(
                Entry::item(r.name.clone(), Action::Stencil(StencilOp::Recent(i)))
                    .radio(current == Some(r.path.as_path()))
                    .enabled(free),
            );
        }
    }
    if st.image.is_some() {
        v.push(Entry::Separator);
        v.push(
            Entry::item(
                lang.pick("なし（ステンシルを使わない）", "None (no stencil)"),
                Action::Stencil(StencilOp::Clear),
            )
            .enabled(free),
        );
    }
    v
}

pub fn mode_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let resolved = app.stencil.resolved_mode();
    [StencilMode::Auto, StencilMode::Mask, StencilMode::Color]
        .into_iter()
        .map(|m| {
            Entry::item(
                mode_name(lang, m, resolved),
                Action::Stencil(StencilOp::Mode(m)),
            )
            .radio(app.stencil.mode == m)
        })
        .collect()
}

pub fn tiling_entries(app: &AppState) -> Vec<Entry<Action>> {
    [
        StencilTiling::None,
        StencilTiling::Horizontal,
        StencilTiling::Vertical,
        StencilTiling::Both,
    ]
    .into_iter()
    .map(|m| {
        Entry::item(
            tiling_name(app.lang, m),
            Action::Stencil(StencilOp::Tiling(m)),
        )
        .radio(app.stencil.tiling == m)
    })
    .collect()
}

/// 落とした PNG（このフレームに落とされ、ポインタが箱の上にあるもの）。
fn dropped_png(ui: &Ui, over: bool) -> Option<std::path::PathBuf> {
    if !over {
        return None;
    }
    ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
    })
}

/// 画像の箱（サムネイル・名前・▾）。押したら true。
fn image_box(ui: &mut Ui, app: &mut AppState, box_rect: Rect, enabled: bool) -> bool {
    let lang = app.lang;
    let id = ui.make_persistent_id("stencil.image");
    let shown = w::look(ui.ctx(), id, enabled);
    let response = ui.interact(
        box_rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let pointer_over = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| box_rect.contains(p));
    let dragging_file = enabled && ui.input(|i| !i.raw.hovered_files.is_empty()) && pointer_over;
    let hover = shown.live && response.hovered();
    let ctx = ui.ctx().clone();
    let texture = app.stencil.thumb_texture(&ctx);
    let name = app
        .stencil
        .image
        .as_ref()
        .map(|i| i.name.clone())
        .unwrap_or_else(|| lang.pick("なし", "None").to_owned());
    let has_image = app.stencil.image.is_some();
    let p = ui.painter();
    w::rounded(
        p,
        box_rect,
        if dragging_file {
            t::ACCENT_DIM
        } else if hover {
            t::CONTROL_HOVER
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    w::outline(
        p,
        box_rect,
        if dragging_file || hover {
            t::ACCENT_DIM
        } else {
            t::BORDER
        },
        1.0,
        3.0,
    );
    let thumb = Rect::from_min_size(
        pos2(box_rect.left() + 3.0, box_rect.top() + 3.0),
        vec2(box_rect.height() - 6.0, box_rect.height() - 6.0),
    );
    w::checker(p, thumb, 3.0);
    match (texture, app.stencil.image.as_ref()) {
        (Some(id), Some(image)) => {
            // 縦横比を保って収める
            let aspect = image.width as f32 / image.height as f32;
            let fit = if aspect >= 1.0 {
                vec2(thumb.width(), thumb.width() / aspect)
            } else {
                vec2(thumb.height() * aspect, thumb.height())
            };
            p.image(
                id,
                Rect::from_center_size(thumb.center(), fit),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        _ => w::icon(p, thumb, "square", t::TEXT_DIM, 13.0),
    }
    w::outline(p, thumb, t::BORDER, 1.0, 0.0);
    let label = Rect::from_min_max(
        pos2(thumb.right() + 6.0, box_rect.top()),
        pos2(box_rect.right() - 22.0, box_rect.bottom()),
    );
    // 画像が無いときは名前（「なし」）を書かない（空の欄の状態は印。名前は読み上げとツールチップ）。窓が最小のときも詰まらない
    if has_image {
        let shown_name = w::fit(p, &name, label.width(), t::LABEL);
        w::text(
            p,
            label,
            &shown_name,
            t::LABEL.with_color(if shown.enabled {
                t::TEXT
            } else {
                t::TEXT_DISABLED
            }),
            w::Align::Left,
        );
    }
    w::icon(
        p,
        Rect::from_min_size(
            pos2(box_rect.right() - 22.0, box_rect.top()),
            vec2(20.0, box_rect.height()),
        ),
        "arrow_drop_down",
        t::TEXT_DIM,
        16.0,
    );
    let accessible = format!("{}: {name}", lang.pick("画像", "Image"));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, &accessible)
    });
    let tip = match app.stencil.image.as_ref() {
        Some(i) => format!(
            "{} ({} × {})\n{}",
            i.name,
            i.width,
            i.height,
            lang.pick(
                "ブラシが通して塗る画像。2D キャンバスと 3D ビューに重ねる。押すと画像を読む・読んだ画像から選ぶ。PNG をここへ落としてもよい。",
                "The image the brush paints through, laid over the 2D canvas and the 3D view. Click to load an image or pick a recent one; a PNG can be dropped here."
            )
        ),
        None => lang
            .pick(
                "ブラシが通して塗る画像。2D キャンバスと 3D ビューに重ねる。押すと画像を読む。PNG をここへ落としてもよい。",
                "The image the brush paints through, laid over the 2D canvas and the 3D view. Click to load an image; a PNG can be dropped here.",
            )
            .to_owned(),
    };
    let clicked = response.clicked();
    let _ = response.on_hover_text(tip);
    if enabled {
        if let Some(path) = dropped_png(ui, pointer_over) {
            app.apply(Action::Stencil(StencilOp::Load(path)));
        }
    }
    clicked
}

/// ステンシルのタブ。
pub fn stencil_tab(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "stencil",
        lang.pick("ステンシル", "Stencil"),
        "square",
        None,
    );
    if !open {
        return;
    }
    let enabled = !app.is_stroking();
    // 画像
    let row = rows.row(IMAGE_ROW, 4.0);
    let has_image = app.stencil.image.is_some();
    let clear_w = if has_image { 28.0 } else { 0.0 };
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL_WIDTH, row.height())),
        lang.pick("画像", "Image"),
        t::LABEL.with_color(w::label_color(ui, "stencil.image.label", enabled)),
        w::Align::Left,
    );
    let box_rect = Rect::from_min_max(
        pos2(row.left() + LABEL_WIDTH, row.top()),
        pos2(row.right() - clear_w, row.bottom()),
    );
    if image_box(ui, app, box_rect, enabled) {
        open_popup(app, ctx, Popup::StencilImage, box_rect, box_rect.width());
    }
    if has_image
        && w::icon_button(
            ui,
            Rect::from_min_size(
                pos2(row.right() - 24.0, row.top()),
                vec2(24.0, row.height()),
            ),
            "stencil.clear",
            "close",
            lang.pick("ステンシルを外す", "Stop using the stencil"),
            false,
            enabled,
            15.0,
        )
        .clicked()
    {
        app.apply(Action::Stencil(StencilOp::Clear));
    }
    if !has_image {
        rows.space(4.0);
        return;
    }
    // 読み方
    let resolved = app.stencil.resolved_mode();
    let mode = mode_name(lang, app.stencil.mode, resolved);
    let mode_row = rows.row(t::ROW_HEIGHT, 4.0);
    let (mode_response, b) = w::dropdown(
        ui,
        mode_row,
        "stencil.mode",
        Some(lang.pick("読み方", "Reads as")),
        &mode,
        Some(lang.pick(
            "量: 画像の明るさが塗りを通す量（白は塗り、黒と透明は止める）。色: ブラシは画像の色で塗り、そのアルファが量。自動: 灰色の画像なら量、そうでなければ色。\n色のモードで、色を受けるのは描くチャンネル（マスクには量だけ）。",
            "Amount: the image's brightness is how much paint gets through (white paints, black and transparent hold back). Color: the brush paints the image's colors, its alpha is the amount. Automatic: amount for a grey image, color otherwise.\nIn color mode the colors go to the paint channel (a mask takes only the amount).",
        )),
        enabled,
        LABEL_WIDTH,
    );
    if mode_response.clicked() {
        open_popup(app, ctx, Popup::StencilMode, b, b.width().max(150.0));
    }
    // 反転と繰り返し
    let cols = Rows::split(rows.row(t::ROW_HEIGHT, 4.0), 2, 6.0);
    let invert = app.stencil.invert;
    let masked = resolved == Some(StencilMode::Mask);
    let next = w::toggle(
        ui,
        cols[0],
        "stencil.invert",
        lang.pick("反転", "Invert"),
        invert,
        Some(lang.pick(
            "黒が塗りを通し、白が止める（透明はやはり止める）。量のときだけ効く。",
            "Black lets the paint through and white holds it back (transparent still holds back). Applies to amount only.",
        )),
        enabled && masked,
    );
    if next != invert {
        app.apply(Action::Stencil(StencilOp::Invert(next)));
    }
    let (response, b) = w::dropdown(
        ui,
        cols[1],
        "stencil.tiling",
        None,
        tiling_name(lang, app.stencil.tiling),
        Some(lang.pick(
            "画像を端の先へ、表示域いっぱいに繰り返す。繰り返さないときは、画像の外には塗らない。",
            "Repeat the image beyond its edges across the view. Without tiling nothing is painted outside the image.",
        )),
        enabled,
        0.0,
    );
    if response.clicked() {
        open_popup(app, ctx, Popup::StencilTiling, b, b.width());
    }
    // 重ねの不透明度・置き場
    if let Some(v) = percent_row(
        ui,
        rows,
        "stencil.opacity",
        lang.pick("重ねの不透明度", "Overlay opacity"),
        app.stencil.opacity as f64,
        (0.0, 1.0),
        Some(lang.pick(
            "ステンシルを表示域にどれだけ濃く重ねるか（表示だけ）。",
            "How strongly the stencil is shown over the views (display only).",
        )),
        enabled,
    ) {
        app.apply(Action::Stencil(StencilOp::Opacity(v as f32)));
    }
    if let Some(v) = percent_row(
        ui,
        rows,
        "stencil.size",
        lang.pick("大きさ", "Size"),
        app.stencil.size as f64,
        (MIN_SIZE as f64, SLIDER_MAX_SIZE as f64),
        Some(lang.pick(
            "画像の高さ（表示域の高さに対する割合）。",
            "The image's height as a share of the view's height.",
        )),
        enabled,
    ) {
        app.apply(Action::Stencil(StencilOp::Size(v as f32)));
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "stencil.angle",
        lang.pick("角度", "Angle"),
        app.stencil.angle,
        (-180.0, 180.0),
        NumberFormat::int("°"),
        Some(lang.pick(
            "画面の上の回転（時計回り）。",
            "Rotation on the screen (clockwise).",
        )),
        enabled,
    ) {
        app.apply(Action::Stencil(StencilOp::Angle(v)));
    }
    if w::button(
        ui,
        rows.row(24.0, 4.0),
        "stencil.reset",
        lang.pick("置き場を戻す", "Reset placement"),
        false,
        enabled,
        Some(lang.pick(
            "表示域の中央・初めの大きさ・回転なしに戻す。",
            "Back to the middle of the view, the first size and no rotation.",
        )),
        None,
    )
    .clicked()
    {
        app.apply(Action::Stencil(StencilOp::ResetPlacement));
    }
    rows.space(4.0);
}
