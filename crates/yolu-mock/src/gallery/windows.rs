//! ウィンドウ（浮いたウィンドウ）。

use egui::{pos2, vec2, Rect, Ui, Vec2};
use yolu_app::ui::widgets as w;
use yolu_app::ui::window::{self, Spec};

use super::{remember, store, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("ウィンドウ", "Windows"),
        entries: vec![Entry::new(
            "window::show",
            (
                "見出しの帯つきの浮いたウィンドウ",
                "Floating window with a header",
            ),
            200.0,
            floating_window,
        )],
    }
}

fn floating_window(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let key = "gallery.window.open";
    let mut open = ctx.still || ctx.values.bool(key, false);
    if !ctx.still {
        let button = Rect::from_min_size(
            pos2(sample.left() + 10.0, sample.top() + 10.0),
            vec2(150.0, 24.0),
        );
        if w::button(
            ui,
            button,
            "open",
            ctx.pick("ウィンドウを開く", "Open the window"),
            false,
            true,
            None,
            None,
        )
        .clicked()
        {
            open = true;
        }
    }
    if !open {
        return;
    }
    let id = ui.id().with("window");
    let screen = ui.ctx().content_rect();
    // 撮る間は見本の枠の真ん中に置く。ウィンドウでは画面の真ん中から動かせる
    let mut offset: Vec2 = if ctx.still {
        sample.center() - screen.center()
    } else {
        remember(ui, "offset", || Vec2::ZERO)
    };
    let lang = ctx.lang;
    let spec = Spec {
        title: lang.pick("ブラシの設定", "Brush settings"),
        icon: Some("paint_brush"),
        size: vec2(260.0, 150.0),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let closed = window::show(ui.ctx(), id, &spec, &mut offset, false, |ui, frame| {
        let mut rows = w::Rows::new(frame.body, 10.0);
        rows.space(10.0);
        let r = rows.row(22.0, 4.0);
        let on = ctx.values.bool("gallery.window.toggle", true);
        let next = w::toggle(
            ui,
            r,
            "toggle",
            lang.pick("筆圧でサイズを変える", "Pressure changes size"),
            on,
            None,
            true,
        );
        ctx.values.set_bool("gallery.window.toggle", next);
        rows.space(10.0);
        let line = rows.row(24.0, 0.0);
        let cells = w::Rows::split(line, 2, 6.0);
        let _ = w::button(
            ui,
            cells[0],
            "cancel",
            lang.pick("キャンセル", "Cancel"),
            false,
            true,
            None,
            None,
        );
        let _ = w::button(
            ui,
            cells[1],
            "ok",
            lang.pick("OK", "OK"),
            true,
            true,
            None,
            None,
        );
    });
    if !ctx.still {
        store(ui, "offset", offset);
        if closed {
            ctx.values.set_bool(key, false);
        }
    }
}
