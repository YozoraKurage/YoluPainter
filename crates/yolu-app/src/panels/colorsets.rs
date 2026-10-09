//! カラーセット・履歴・中間色の専用タブ。
use crate::dialog::places::Place;
use crate::notice::Source;
use crate::{
    colorsets::{self, Edit, Error, Palette, Swatch},
    state::AppState,
    ui::{theme as t, widgets as w},
};
use egui::{vec2, Color32, Rect, Response, Sense, Ui, WidgetInfo, WidgetType};

fn report(app: &mut AppState, result: Result<(), Error>) {
    if let Err(e) = result {
        app.fail(Source::Color, e.message(app.lang));
    }
}
fn swatch(ui: &mut Ui, color: &Swatch, size: f32, selected: bool) -> Response {
    let (r, response) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(r, 0.0, Color32::WHITE);
    let half = r.size() * 0.5;
    p.rect_filled(
        Rect::from_min_size(r.min, half),
        0.0,
        Color32::from_gray(160),
    );
    p.rect_filled(
        Rect::from_min_size(r.center(), half),
        0.0,
        Color32::from_gray(160),
    );
    let [red, green, blue, alpha] = color.rgba.map(w::to_byte);
    p.rect_filled(
        r,
        0.0,
        Color32::from_rgba_unmultiplied(red, green, blue, alpha),
    );
    w::outline(
        p,
        r,
        if selected || response.hovered() {
            t::ACCENT
        } else {
            t::BORDER
        },
        if selected { 2.0 } else { 1.0 },
        0.0,
    );
    let label = color.label();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
    response.on_hover_text(label)
}
fn pick(ui: &mut Ui, app: &mut AppState, response: &Response, color: &Swatch) {
    if response.clicked() {
        if ui.input(|i| i.modifiers.alt) {
            app.color.sub = color.rgba;
        } else {
            app.color.set_main(color.rgba);
        }
    }
    response.context_menu(|ui| {
        if ui
            .button(app.lang.pick("メインの色", "Foreground color"))
            .clicked()
        {
            app.color.set_main(color.rgba);
            ui.close();
        }
        if ui
            .button(app.lang.pick("サブの色", "Background color"))
            .clicked()
        {
            app.color.sub = color.rgba;
            ui.close();
        }
    });
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let lang = app.lang;
    egui::ScrollArea::vertical()
        .id_salt("colorsets.scroll")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut active = app.colorsets.active_index();
                egui::ComboBox::from_id_salt("colorsets.set")
                    .selected_text(&app.colorsets.palette().name)
                    .width((ui.available_width() - 60.0).max(32.0))
                    .show_ui(ui, |ui| {
                        for (i, e) in app.colorsets.entries().iter().enumerate() {
                            ui.selectable_value(&mut active, i, &e.palette.name);
                        }
                    })
                    .response
                    .on_hover_text(lang.pick("カラーセット", "Color Sets"));
                if active != app.colorsets.active_index() {
                    app.colorsets.switch(active);
                }
                ui.menu_button(lang.pick("編集", "Edit"), |ui| {
                    if ui.button(lang.pick("新規セット", "New Set")).clicked() {
                        let r = app.colorsets.add_set(Palette {
                            name: lang.pick("新規セット", "New Set").into(),
                            colors: Vec::new(),
                        });
                        report(app, r);
                        ui.close();
                    }
                    if ui
                        .button(lang.pick("セットを複製", "Duplicate Set"))
                        .clicked()
                    {
                        let r = app.colorsets.duplicate(lang);
                        report(app, r);
                        ui.close();
                    }
                    if ui.button(lang.pick("名前を変更", "Rename")).clicked() {
                        app.colorsets.rename = Some(app.colorsets.palette().name.clone());
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            app.colorsets.entries().len() > 1,
                            egui::Button::new(lang.pick("セットを削除", "Delete Set")),
                        )
                        .clicked()
                    {
                        let r = app.colorsets.remove_set();
                        report(app, r);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(lang.pick("読み込み", "Import")).clicked() {
                        if let Some(path) = crate::dialog::file(app, Place::ColorSet)
                            .add_filter("GPL / ACO", &["gpl", "aco"])
                            .pick_file()
                        {
                            app.note_file_chosen(Place::ColorSet, &path);
                            let r = app.colorsets.import(&path);
                            report(app, r);
                        }
                        ui.close();
                    }
                    if ui
                        .button(lang.pick("書き出し", "Export"))
                        .on_hover_text(lang.pick(
                            "GIMP パレット（RGB・8 ビット）",
                            "GIMP palette (RGB, 8-bit)",
                        ))
                        .clicked()
                    {
                        // 透明色を含む場合は保存先のウィンドウを開く前に理由を返す。
                        match colorsets::format::write_gpl(app.colorsets.palette()) {
                            Err(e) => report(app, Err(e)),
                            Ok(_) => {
                                if let Some(path) = crate::dialog::file(app, Place::ColorSet)
                                    .add_filter("GIMP", &["gpl"])
                                    .set_file_name("palette.gpl")
                                    .save_file()
                                {
                                    app.note_file_chosen(Place::ColorSet, &path);
                                    let r = app.colorsets.export(&path);
                                    report(app, r);
                                }
                            }
                        }
                        ui.close();
                    }
                });
            });
            if let Some(mut name) = app.colorsets.rename.take() {
                let mut finish = false;
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width((ui.available_width() - 62.0).max(24.0))
                            .char_limit(colorsets::MAX_NAME_BYTES),
                    );
                    response
                        .clone()
                        .on_hover_text(lang.pick("セット名", "Set name"));
                    if ui.button(lang.pick("確定", "Apply")).clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        let result = app.colorsets.edit(Edit::Rename(name.clone()));
                        finish = result.is_ok();
                        report(app, result);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        finish = true;
                        // この Esc は名前の変更をやめるのに使った（キャンバスが同じ Esc で選択範囲を解除しない）
                        crate::ui::window::note_escape_taken(ui.ctx());
                    }
                });
                if !finish {
                    app.colorsets.rename = Some(name);
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(lang.pick("色を追加", "Add Color")).clicked() {
                    let r = app.colorsets.edit(Edit::Add(app.color.main));
                    report(app, r);
                }
                let selected = app.colorsets.selected;
                if ui
                    .add_enabled(
                        selected.is_some(),
                        egui::Button::new(lang.pick("置き換え", "Replace")),
                    )
                    .clicked()
                {
                    let r = app
                        .colorsets
                        .edit(Edit::Replace(selected.unwrap(), app.color.main));
                    report(app, r);
                }
                if ui
                    .add_enabled(
                        selected.is_some(),
                        egui::Button::new(lang.pick("削除", "Delete")),
                    )
                    .clicked()
                {
                    let r = app.colorsets.edit(Edit::Remove(selected.unwrap()));
                    report(app, r);
                }
            });
            let colors = app.colorsets.palette().colors.clone();
            let cols = colorsets::columns(ui.available_width());
            let size = 24.0_f32.min(ui.available_width().max(1.0));
            let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                app.colorsets.dragging = None;
            }
            let mut drop_at = None;
            egui::Grid::new("colorsets.grid")
                .min_col_width(0.0)
                .min_row_height(0.0)
                .spacing(vec2(4.0, 4.0))
                .show(ui, |ui| {
                    for (i, c) in colors.iter().enumerate() {
                        let response = swatch(ui, c, size, app.colorsets.selected == Some(i));
                        if response.clicked() || response.secondary_clicked() {
                            app.colorsets.selected = Some(i);
                        }
                        pick(ui, app, &response, c);
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            app.colorsets.dragging = Some(i);
                        }
                        if released && response.contains_pointer() {
                            drop_at = Some(i);
                        }
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            if released {
                if let (Some(from), Some(to)) = (app.colorsets.dragging.take(), drop_at) {
                    let r = app.colorsets.edit(Edit::Move(from, to));
                    report(app, r);
                }
            }
            egui::CollapsingHeader::new(lang.pick("履歴", "History"))
                .default_open(true)
                .show(ui, |ui| {
                    let cols = colorsets::columns(ui.available_width());
                    egui::Grid::new("colorsets.history")
                        .min_col_width(0.0)
                        .min_row_height(0.0)
                        .spacing(vec2(4.0, 4.0))
                        .show(ui, |ui| {
                            for (i, c) in app.color.recent.clone().into_iter().enumerate() {
                                let c = Swatch::new(c);
                                let response = swatch(ui, &c, size, false);
                                pick(ui, app, &response, &c);
                                if (i + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            egui::CollapsingHeader::new(lang.pick("中間色", "Intermediate Colors"))
                .default_open(true)
                .show(ui, |ui| {
                    let side = ((ui.available_width() - 12.0) / 7.0).clamp(1.0, 24.0);
                    let corners = app.colorsets.corners;
                    egui::Grid::new("colorsets.intermediate")
                        .min_col_width(0.0)
                        .min_row_height(0.0)
                        .spacing(vec2(2.0, 2.0))
                        .show(ui, |ui| {
                            for y in 0..7 {
                                for x in 0..7 {
                                    let corner = match (x, y) {
                                        (0, 0) => Some(0),
                                        (6, 0) => Some(1),
                                        (0, 6) => Some(2),
                                        (6, 6) => Some(3),
                                        _ => None,
                                    };
                                    let name = match corner {
                                        Some(0) => lang.pick("左上", "Top left"),
                                        Some(1) => lang.pick("右上", "Top right"),
                                        Some(2) => lang.pick("左下", "Bottom left"),
                                        Some(3) => lang.pick("右下", "Bottom right"),
                                        _ => "",
                                    };
                                    let c = Swatch {
                                        name: name.into(),
                                        rgba: colorsets::intermediate(
                                            corners,
                                            x as f32 / 6.0,
                                            y as f32 / 6.0,
                                        ),
                                    };
                                    let response = swatch(ui, &c, side, corner.is_some());
                                    if let Some(i) = corner {
                                        if response.clicked() {
                                            app.colorsets.corners[i] = app.color.main;
                                        }
                                    } else {
                                        pick(ui, app, &response, &c);
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
        });
}
