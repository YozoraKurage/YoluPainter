//! ボタンと切り替え（アイコンのボタン・文字のボタン・チェック・ツールの帯のボタン）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::widgets as w;

use super::{rows, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("ボタンと切り替え", "Buttons and toggles"),
        entries: vec![
            Entry::new(
                "widgets::icon_button",
                ("アイコンだけのボタン", "Icon-only button"),
                44.0,
                icon_buttons,
            ),
            Entry::new(
                "widgets::button",
                (
                    "文字のボタン（通常・青・押せない）",
                    "Text button (normal, primary, disabled)",
                ),
                70.0,
                text_buttons,
            ),
            Entry::new(
                "widgets::button_shown",
                (
                    "アイコンだけにした文字のボタン",
                    "Text button drawn as an icon",
                ),
                44.0,
                icon_only_buttons,
            ),
            Entry::new(
                "widgets::toggle・toggle_height",
                (
                    "チェック（長い名前は折り返す）",
                    "Checkbox (long labels wrap)",
                ),
                122.0,
                toggles,
            ),
            Entry::new(
                "widgets::tool_button",
                ("ツールの帯のボタン", "Tool strip button"),
                52.0,
                tool_buttons,
            ),
        ],
    }
}

fn icon_buttons(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let selected = ctx.values.bool("gallery.icon-button.selected", true);
    let at = |i: f32| {
        Rect::from_min_size(
            pos2(sample.left() + 10.0 + i * 34.0, sample.top() + 10.0),
            vec2(24.0, 24.0),
        )
    };
    let _ = w::icon_button(
        ui,
        at(0.0),
        "normal",
        "add",
        ctx.pick("追加", "Add"),
        false,
        true,
        18.0,
    );
    if w::icon_button(
        ui,
        at(1.0),
        "selected",
        "vignette",
        ctx.pick("マスク", "Mask"),
        selected,
        true,
        17.0,
    )
    .clicked()
    {
        ctx.values
            .set_bool("gallery.icon-button.selected", !selected);
    }
    let _ = w::icon_button(
        ui,
        at(2.0),
        "disabled",
        "delete",
        ctx.pick("削除", "Delete"),
        false,
        false,
        17.0,
    );
    let _ = w::icon_button(
        ui,
        at(3.0),
        "eye",
        "visibility",
        ctx.pick("表示", "Show"),
        false,
        true,
        16.0,
    );
}

fn text_buttons(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(8.0);
    let line = rows.row(24.0, 8.0);
    let cells = w::Rows::split(line, 3, 6.0);
    let _ = w::button(
        ui,
        cells[0],
        "cancel",
        ctx.pick("キャンセル", "Cancel"),
        false,
        true,
        None,
        None,
    );
    let _ = w::button(
        ui,
        cells[1],
        "apply",
        ctx.pick("適用", "Apply"),
        true,
        true,
        None,
        None,
    );
    let _ = w::button(
        ui,
        cells[2],
        "off",
        ctx.pick("削除", "Delete"),
        false,
        false,
        None,
        None,
    );
    let line = rows.row(24.0, 0.0);
    let cells = w::Rows::split(line, 2, 6.0);
    let _ = w::button(
        ui,
        cells[0],
        "add",
        ctx.pick("追加", "Add"),
        false,
        true,
        None,
        Some("add"),
    );
    let _ = w::button(
        ui,
        cells[1],
        "import",
        ctx.pick("取り込み", "Import"),
        true,
        true,
        None,
        Some("import"),
    );
}

fn icon_only_buttons(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let at = |i: f32| {
        Rect::from_min_size(
            pos2(sample.left() + 10.0 + i * 36.0, sample.top() + 10.0),
            vec2(30.0, 24.0),
        )
    };
    for (i, (icon, ja, en)) in [
        ("add", "追加", "Add"),
        ("folder", "フォルダー", "Folder"),
        ("save", "保存", "Save"),
    ]
    .iter()
    .enumerate()
    {
        let _ = w::button_shown(
            ui,
            at(i as f32),
            ("icon-only", i),
            ctx.pick(ja, en),
            false,
            true,
            Some(ctx.pick(ja, en)),
            Some(icon),
            false,
            None,
        );
    }
}

fn toggles(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(8.0);
    let items: [(&str, &str, &str, bool, bool); 3] = [
        ("on", "クリッピング", "Clipping", true, true),
        ("off", "ロック", "Lock", false, true),
        ("disabled", "押せない", "Disabled", true, false),
    ];
    for (key, ja, en, default, enabled) in items {
        let name = format!("gallery.toggle.{key}");
        let value = ctx.values.bool(&name, default);
        let r = rows.row(22.0, 2.0);
        let next = w::toggle(ui, r, key, ctx.pick(ja, en), value, None, enabled);
        ctx.values.set_bool(&name, next);
    }
    let long = ctx.pick(
        "筆圧に合わせてブラシの大きさと濃さを変える",
        "Change brush size and opacity with pen pressure",
    );
    let height = w::toggle_height(ui.painter(), rows.width(), long);
    let r = rows.row(height, 0.0);
    let value = ctx.values.bool("gallery.toggle.long", false);
    let next = w::toggle(ui, r, "long", long, value, None, true);
    ctx.values.set_bool("gallery.toggle.long", next);
}

fn tool_buttons(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let selected = ctx.values.index("gallery.tool", 0);
    let tools = [
        ("brush", "ブラシ", "Brush"),
        ("eraser", "消しゴム", "Eraser"),
        ("fill", "塗りつぶし", "Fill"),
        ("gradient", "グラデーション", "Gradient"),
        ("eyedropper", "スポイト", "Eyedropper"),
    ];
    for (i, (id, ja, en)) in tools.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(sample.left() + 10.0 + i as f32 * 44.0, sample.top() + 8.0),
            vec2(36.0, 36.0),
        );
        if w::tool_button(ui, r, id, ctx.pick(ja, en), selected == i).clicked() {
            ctx.values.set_index("gallery.tool", i);
        }
    }
}
