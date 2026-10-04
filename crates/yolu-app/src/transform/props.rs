//! 移動・変形の道具の設定: オプションバー（反転・90° 回転・補間）と、プロパティの欄（同じボタン・数値の変形・補間・選んだ層のロック）。
//! 操作は `Action::M2(Edit::Transform)` を通る（キー・メニュー・試験と同じ道。1 回の Undo）。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};

use super::{resampling_name, Numeric};
use crate::layerops::Xform;
use crate::m2::Edit;
use crate::m2_menu::Popup;
use crate::panels::layer_props;
use crate::panels::properties::{choice_row, group_label, open_popup, section, slider_row};
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

/// 4 つの変形ボタン（反転 2・90° 回転 2）: (アイコン, 名前, 変形)。
fn buttons(app: &AppState) -> [(&'static str, &'static str, Xform); 4] {
    let l = app.lang;
    [
        (
            "flip",
            l.pick("左右反転", "Flip Horizontal"),
            Xform::Flip { horizontal: true },
        ),
        (
            "flip_vertical",
            l.pick("上下反転", "Flip Vertical"),
            Xform::Flip { horizontal: false },
        ),
        (
            "rotate_90_degrees_ccw",
            l.pick("反時計回りに 90° 回転", "Rotate 90° Counter-clockwise"),
            Xform::Rotate90 { clockwise: false },
        ),
        (
            "rotate_90_degrees_cw",
            l.pick("時計回りに 90° 回転", "Rotate 90° Clockwise"),
            Xform::Rotate90 { clockwise: true },
        ),
    ]
}

/// 変形できる状態か（描いていない・動かす層がある）。
fn usable(app: &AppState) -> bool {
    !app.is_stroking() && app.can_edit() && !app.transform_targets().is_empty()
}

fn open_resampling(app: &mut AppState, ctx: &egui::Context, anchor: Rect) {
    app.popup = Some(OpenPopup {
        kind: PopupKind::M2(Popup::Resampling),
        state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
    });
}

/// オプションバー（`x` は左端）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let enabled = usable(app);
    for (icon, tip, xform) in buttons(app) {
        let at = Rect::from_min_size(pos2(x, y), vec2(28.0, h));
        if w::icon_button(
            ui,
            at,
            ("options.transform", icon),
            icon,
            tip,
            false,
            enabled,
            20.0,
        )
        .clicked()
        {
            app.apply(Action::M2(Edit::Transform(xform)));
        }
        x += 32.0;
    }
    x += 4.0;
    w::vline(
        ui.painter(),
        x,
        r.top() + 6.0,
        r.bottom() - 6.0,
        t::SEPARATOR,
    );
    x += 8.0;
    let lang = app.lang;
    let width = 190.0;
    if x + width <= r.right() - 8.0 {
        let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
        let (response, b) = w::dropdown(
            ui,
            at,
            "options.transform.resampling",
            Some(lang.pick("補間", "Resampling")),
            resampling_name(lang, app.transform.resampling),
            Some(lang.pick(
                "バイリニアはなめらか、ニアレストネイバーは画素の硬さを保つ。整数画素の移動・90° 回転・反転はどちらでも画素をそのまま写す",
                "Bilinear smooths, Nearest keeps hard pixels. Whole-pixel moves, 90° turns and flips copy pixels exactly either way",
            )),
            true,
            0.0,
        );
        if response.clicked() {
            let ctx = ui.ctx().clone();
            open_resampling(app, &ctx, b);
        }
    }
}

/// プロパティの欄（移動・変形の道具を選んでいるとき）。
pub fn body(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    let enabled = usable(app);
    let (open, _) = section(
        ui,
        app,
        rows,
        "transform",
        lang.pick("移動・変形", "Move / Transform"),
        "arrow_move",
        None,
    );
    if open {
        let row = rows.row(24.0, 4.0);
        for (i, (icon, tip, xform)) in buttons(app).into_iter().enumerate() {
            let at = Rect::from_min_size(
                pos2(row.left() + 30.0 * i as f32, row.top()),
                vec2(28.0, row.height()),
            );
            if w::icon_button(
                ui,
                at,
                ("props.transform", icon),
                icon,
                tip,
                false,
                enabled,
                18.0,
            )
            .clicked()
            {
                app.apply(Action::M2(Edit::Transform(xform)));
            }
        }
        group_label(ui, rows, lang.pick("数値", "Numeric"));
        let n = app.transform.numeric;
        let (w_px, h_px) = (app.doc.width() as f32, app.doc.height() as f32);
        let px = |suffix: &'static str| NumberFormat {
            decimals: 1,
            trim: true,
            suffix,
        };
        if let Some(v) = slider_row(
            ui,
            rows,
            "transform.dx",
            "X",
            n.dx as f32,
            (-w_px, w_px),
            px(" px"),
            Some(lang.pick("横にずらす量（+ は右）", "Horizontal offset (+ is right)")),
            enabled,
        ) {
            app.transform.numeric.dx = v as f64;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "transform.dy",
            "Y",
            n.dy as f32,
            (-h_px, h_px),
            px(" px"),
            Some(lang.pick("縦にずらす量（+ は上）", "Vertical offset (+ is up)")),
            enabled,
        ) {
            app.transform.numeric.dy = v as f64;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "transform.sx",
            lang.pick("幅", "Width"),
            n.scale_x as f32,
            (-400.0, 400.0),
            px("%"),
            Some(lang.pick(
                "横の拡大率（負は左右反転）",
                "Horizontal scale (a negative value flips)",
            )),
            enabled,
        ) {
            app.transform.numeric.scale_x = v as f64;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "transform.sy",
            lang.pick("高さ", "Height"),
            n.scale_y as f32,
            (-400.0, 400.0),
            px("%"),
            Some(lang.pick(
                "縦の拡大率（負は上下反転）",
                "Vertical scale (a negative value flips)",
            )),
            enabled,
        ) {
            app.transform.numeric.scale_y = v as f64;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "transform.angle",
            lang.pick("角度", "Angle"),
            n.degrees as f32,
            (-180.0, 180.0),
            px("°"),
            Some(lang.pick(
                "反時計回り。動かすものの中心を軸にする",
                "Counter-clockwise, about the centre of what moves",
            )),
            enabled,
        ) {
            app.transform.numeric.degrees = v as f64;
        }
        if let Some(b) = choice_row(
            ui,
            rows,
            "transform.resampling",
            lang.pick("補間", "Resampling"),
            resampling_name(lang, app.transform.resampling),
            Some(lang.pick(
                "バイリニアはなめらか、ニアレストネイバーは画素の硬さを保つ",
                "Bilinear smooths, Nearest keeps hard pixels",
            )),
            true,
        ) {
            open_popup(app, ctx, Popup::Resampling, b, b.width());
        }
        let row = rows.row(26.0, 4.0);
        let half = (row.width() - 6.0) / 2.0;
        let apply = Rect::from_min_size(row.min, vec2(half, row.height()));
        let reset = Rect::from_min_size(
            pos2(row.left() + half + 6.0, row.top()),
            vec2(half, row.height()),
        );
        let changed = app.transform.numeric != Numeric::default();
        if w::button(
            ui,
            apply,
            "transform.apply",
            lang.pick("適用", "Apply"),
            true,
            enabled && changed,
            Some(lang.pick(
                "この数値で変形する（1 回の取り消し）",
                "Transform by these numbers (one undo step)",
            )),
            None,
        )
        .clicked()
        {
            let n = app.transform.numeric;
            app.apply(Action::M2(Edit::Transform(Xform::Numeric {
                dx: n.dx,
                dy: n.dy,
                degrees: n.degrees,
                sx: n.scale_x / 100.0,
                sy: n.scale_y / 100.0,
            })));
            app.transform.numeric = Numeric::default();
        }
        if w::button(
            ui,
            reset,
            "transform.reset",
            lang.pick("戻す", "Reset"),
            false,
            changed,
            Some(lang.pick("数値を初めに戻す", "Back to no change")),
            None,
        )
        .clicked()
        {
            app.transform.numeric = Numeric::default();
        }
    }
    layer_props::lock_section(ui, app, rows);
}
