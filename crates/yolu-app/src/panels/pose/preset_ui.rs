//! 「ポーズのプリセット」の節: 今のポーズを名前を付けて保存する欄と、保存したポーズの一覧（名前を押すと当てる・左右を反転して当てる・
//! 今のポーズで上書き・名前を変える・消す）。当てた結果の知らせ（飛ばしたボーン・読めなかったファイル）は印と件数で、中身はツールチップ。
//! 中身の計算と保存は `view3d::pose::presets`。ここは並べて、押されたことを渡すだけ。

use egui::{pos2, vec2, Rect, Sense, Ui};

use crate::state::AppState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};
use crate::view3d::pose::presets;

const ICON_W: f32 = 22.0;
const SAVE_W: f32 = 112.0;

/// 1 行の操作。
enum Op {
    Apply(u32, bool),
    Overwrite(u32),
    StartRename(u32),
    Delete(u32),
}

pub(super) fn show(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    // ギズモ・欄のドラッグの最中は、今のポーズが途中の値なので、保存も当てるのもしない
    let busy = app.view3d.pose.drag.is_some() || s.is_editing();
    let editable = free && !busy;
    let can_save = editable && presets::has_bone_changes(&s.rig, s.pose());

    // 保存する名前の欄と、保存のボタン
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let field = Rect::from_min_max(row.min, pos2(row.right() - SAVE_W - 4.0, row.bottom()));
    let default_name = presets::default_name(lang);
    let shown = if app.view3d.pose.preset_name.is_empty() {
        default_name.to_owned()
    } else {
        app.view3d.pose.preset_name.clone()
    };
    let out = w::text_field(
        ui,
        field,
        "pose.preset.name",
        &shown,
        Some(lang.pick("ポーズを保存する名前", "Name to save the pose under")),
        false,
    );
    if let Some(name) = out.committed {
        app.view3d.pose.preset_name = name;
    }
    let save = Rect::from_min_max(pos2(row.right() - SAVE_W, row.top()), row.max);
    if w::button(
        ui,
        save,
        "pose.preset.save",
        lang.pick("ポーズを保存", "Save Pose"),
        false,
        can_save,
        Some(lang.pick(
            "今のポーズ（休みの形と違うボーンの位置・回転・大きさ）をこの名前で保存する。同じ名前があれば番号を付ける",
            "Save the current pose (position, rotation and scale of the bones that differ from rest) under this name. A name already in use gets a number",
        )),
        Some("save"),
    )
    .clicked()
    {
        let name = app.view3d.pose.preset_name.clone();
        if presets::save_preset(app, &name).is_some() {
            app.view3d.pose.preset_name.clear();
        }
    }

    // 保存したポーズの一覧
    let list: Vec<(u32, String)> = app
        .view3d
        .pose
        .pose_presets
        .items()
        .iter()
        .map(|p| (p.id, p.name.clone()))
        .collect();
    let renaming = app.view3d.pose.preset_rename;
    let mut op = None;
    for (id, name) in &list {
        let row = rows.row(t::ROW_HEIGHT, 1.0);
        let mut x = row.right();
        let mut slot = || {
            x -= ICON_W;
            Rect::from_min_size(pos2(x, row.top()), vec2(ICON_W, row.height()))
        };
        let delete = slot();
        let rename = slot();
        let overwrite = slot();
        let mirror = slot();
        let label_rect = Rect::from_min_max(row.min, pos2(mirror.left() - 4.0, row.bottom()));
        if renaming == Some(*id) {
            let first = !app.view3d.pose.preset_rename_started;
            app.view3d.pose.preset_rename_started = true;
            let out = w::text_field(ui, label_rect, ("pose.preset.rename", *id), name, None, first);
            if let Some(next) = out.committed {
                presets::rename_preset(app, *id, &next);
            }
            if !first && !out.focused {
                app.view3d.pose.preset_rename = None;
            }
        } else {
            let response = ui.interact(
                label_rect,
                ui.make_persistent_id(("pose.preset.apply", *id)),
                if editable { Sense::click() } else { Sense::hover() },
            );
            if editable && response.hovered() {
                w::fill(ui.painter(), label_rect, t::CONTROL_HOVER);
            }
            let color = if editable { t::TEXT } else { t::TEXT_DISABLED };
            let shown = w::fit(ui.painter(), name, label_rect.width() - 8.0, t::LABEL);
            w::text(
                ui.painter(),
                Rect::from_min_max(
                    pos2(label_rect.left() + 4.0, label_rect.top()),
                    label_rect.max,
                ),
                &shown,
                t::LABEL.with_color(color),
                Align::Left,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, editable, name)
            });
            let response = response.on_hover_text(lang.pick(
                "このポーズを当てる（ポーズの取り消しに 1 段積む）",
                "Apply this pose (one pose undo step)",
            ));
            if response.clicked() {
                op = Some(Op::Apply(*id, false));
            }
        }
        if w::icon_button(
            ui,
            mirror,
            ("pose.preset.mirror", *id),
            "flip",
            lang.pick("左右を反転して当てる", "Apply Mirrored"),
            false,
            editable,
            14.0,
        )
        .clicked()
        {
            op = Some(Op::Apply(*id, true));
        }
        if w::icon_button(
            ui,
            overwrite,
            ("pose.preset.overwrite", *id),
            "save",
            lang.pick("今のポーズで上書き", "Overwrite with Current Pose"),
            false,
            // 保存と同じ条件（休みの形のままでは、保存したポーズを空の項目で置き換えて元に戻せなくなる）
            can_save,
            14.0,
        )
        .clicked()
        {
            op = Some(Op::Overwrite(*id));
        }
        if w::icon_button(
            ui,
            rename,
            ("pose.preset.edit", *id),
            "edit",
            lang.pick("名前を変える", "Rename"),
            renaming == Some(*id),
            free,
            14.0,
        )
        .clicked()
        {
            op = Some(Op::StartRename(*id));
        }
        if w::icon_button(
            ui,
            delete,
            ("pose.preset.delete", *id),
            "delete",
            lang.pick("このポーズを消す", "Delete this pose"),
            false,
            free,
            13.0,
        )
        .clicked()
        {
            op = Some(Op::Delete(*id));
        }
    }
    match op {
        Some(Op::Apply(id, mirror)) => {
            presets::apply_preset(app, id, mirror);
        }
        Some(Op::Overwrite(id)) => {
            presets::overwrite_preset(app, id);
        }
        Some(Op::StartRename(id)) => {
            app.view3d.pose.preset_rename = Some(id);
            app.view3d.pose.preset_rename_started = false;
        }
        Some(Op::Delete(id)) => {
            presets::delete_preset(app, id);
        }
        None => {}
    }

    // 飛ばしたボーンと、読めなかったプリセットのファイル
    let mut notes: Vec<String> = app
        .view3d
        .pose
        .session
        .as_ref()
        .map(|s| s.preset_notes.iter().map(|k| k.describe(lang)).collect())
        .unwrap_or_default();
    notes.extend(
        app.view3d
            .pose
            .pose_presets
            .problems
            .iter()
            .map(|p| p.describe(lang)),
    );
    if !notes.is_empty() {
        let at = rows.row(t::ROW_HEIGHT, 2.0);
        super::warning_row(
            ui,
            at,
            "pose.presets.notes",
            notes.len(),
            &notes.join("\n"),
            lang,
        );
    }
    rows.space(2.0);
}
