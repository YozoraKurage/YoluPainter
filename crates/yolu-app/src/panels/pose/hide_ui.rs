//! 「面を隠す」の節: 次に足す項目の欄（子を含める・しきい値）と「選んだボーンの面を隠す」ボタン、隠している項目の一覧（外せる）、
//! 隠し方のプリセット（今の隠し方を名前を付けて保存・入れる/外す・消す。入れたプリセットは全部の和）。
//! 中身の計算と保存は `view3d::pose::hide`。ここは並べて、押されたことを渡すだけ。

use egui::{pos2, vec2, Rect, Ui};

use crate::state::AppState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};
use crate::view3d::pose::hide;

const ICON_W: f32 = 22.0;
const SAVE_W: f32 = 74.0;

pub(super) fn show(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    let state = s.hide.clone();
    let selected = s.selected;
    let entry_names: Vec<String> = state
        .entries
        .iter()
        .map(|e| s.rig.bones()[e.bone].name.clone())
        .collect();

    // 次に足す項目の欄
    let row = rows.row(t::ROW_HEIGHT, 2.0);
    let children = w::toggle(
        ui,
        row,
        "pose.hide.children",
        lang.pick("子を含める", "Include Children"),
        state.children,
        Some(lang.pick(
            "選んだボーンの子孫のボーンの影響も含めて隠す",
            "Also hide the surfaces influenced by the bones under the selected bone",
        )),
        free,
    );
    let threshold = super::super::properties::percent_row(
        ui,
        rows,
        "pose.hide.threshold",
        lang.pick("しきい値", "Threshold"),
        f64::from(state.threshold),
        (0.0, 1.0),
        Some(lang.pick(
            "面の 3 頂点が、そのボーンに掛けているウェイトの平均がこの値以上の面を隠す",
            "Hide surfaces whose three vertices average at least this weight on the bone",
        )),
        free,
    );
    if children != state.children || threshold.is_some() {
        hide::set_next(
            app,
            threshold.map_or(state.threshold, |v| v as f32),
            children,
        );
    }
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "pose.hide.add",
        lang.pick("選んだボーンの面を隠す", "Hide Selected Bone"),
        false,
        free && selected.is_some(),
        Some(lang.pick(
            "選んだボーンの影響のある面を隠す。同じボーンにもう一度使うと、しきい値と子の設定を入れ替える",
            "Hide the surfaces the selected bone influences. Using it again on the same bone replaces its threshold and children setting",
        )),
        Some("visibility_off"),
    )
    .clicked()
    {
        if let Some(bone) = selected {
            hide::hide_bone(app, bone);
        }
    }

    // 隠している項目
    let mut remove = None;
    for (i, e) in state.entries.iter().enumerate() {
        let row = rows.row(t::ROW_HEIGHT, 1.0);
        let p = ui.painter().clone();
        w::icon(
            &p,
            Rect::from_min_size(row.min, vec2(16.0, row.height())),
            "visibility_off",
            t::TEXT_DIM,
            13.0,
        );
        let button = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let status = format!(
            "{}%{}",
            (e.threshold * 100.0).round(),
            if e.children {
                lang.pick("・子", " +children")
            } else {
                ""
            }
        );
        let status_w = w::text_width(&p, &status, t::LABEL_DIM);
        w::text(
            &p,
            Rect::from_min_max(
                pos2(button.left() - status_w - 4.0, row.top()),
                pos2(button.left() - 2.0, row.bottom()),
            ),
            &status,
            t::LABEL_DIM,
            Align::Right,
        );
        let name_rect = Rect::from_min_max(
            pos2(row.left() + 20.0, row.top()),
            pos2(button.left() - status_w - 8.0, row.bottom()),
        );
        w::text(
            &p,
            name_rect,
            &w::fit(&p, &entry_names[i], name_rect.width(), t::LABEL),
            t::LABEL,
            Align::Left,
        );
        if w::icon_button(
            ui,
            button,
            ("pose.hide.remove", i),
            "close",
            lang.pick("この項目を外す", "Remove this entry"),
            false,
            free,
            12.0,
        )
        .clicked()
        {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        hide::remove_entry(app, i);
    }

    // プリセット
    rows.space(4.0);
    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let field = Rect::from_min_max(row.min, pos2(row.right() - SAVE_W - 4.0, row.bottom()));
    let default_name = lang.pick("隠し方", "Hide Set");
    let shown = if app.view3d.pose.hide_name.is_empty() {
        default_name.to_owned()
    } else {
        app.view3d.pose.hide_name.clone()
    };
    let out = w::text_field(
        ui,
        field,
        "pose.hide.name",
        &shown,
        Some(lang.pick("隠し方を保存する名前", "Name to save the hide set under")),
        false,
    );
    if let Some(name) = out.committed {
        app.view3d.pose.hide_name = name;
    }
    let save = Rect::from_min_max(pos2(row.right() - SAVE_W, row.top()), row.max);
    if w::button(
        ui,
        save,
        "pose.hide.save",
        lang.pick("保存", "Save"),
        false,
        free && !state.is_clear(),
        Some(lang.pick(
            "今の隠し方（隠している項目と、入れているプリセットの和）を、この名前で保存する。同じ名前があれば番号を付ける",
            "Save the current hide set (the entries plus the active presets) under this name. A name already in use gets a number",
        )),
        Some("save"),
    )
    .clicked()
    {
        let name = app.view3d.pose.hide_name.clone();
        if hide::save_preset(app, &name).is_some() {
            app.view3d.pose.hide_name.clear();
        }
    }

    let presets: Vec<(u32, String)> = app
        .view3d
        .pose
        .hide_presets
        .items()
        .iter()
        .map(|p| (p.id, p.name.clone()))
        .collect();
    let (mut toggle, mut delete) = (None, None);
    for (id, name) in &presets {
        let row = rows.row(t::ROW_HEIGHT, 1.0);
        let button = Rect::from_min_size(
            pos2(row.right() - ICON_W, row.top()),
            vec2(ICON_W, row.height()),
        );
        let label_rect = Rect::from_min_max(row.min, pos2(button.left() - 4.0, row.bottom()));
        let label = w::fit(ui.painter(), name, label_rect.width() - 24.0, t::LABEL);
        let on = state.presets.contains(id);
        if w::toggle(
            ui,
            label_rect,
            ("pose.hide.preset", *id),
            &label,
            on,
            Some(lang.pick(
                "この隠し方を入れる・外す。入れた隠し方は全部の和で隠す",
                "Turn this hide set on or off. Every active set hides together",
            )),
            free,
        ) != on
        {
            toggle = Some(*id);
        }
        if w::icon_button(
            ui,
            button,
            ("pose.hide.delete", *id),
            "delete",
            lang.pick("この隠し方を消す", "Delete this hide set"),
            false,
            free,
            13.0,
        )
        .clicked()
        {
            delete = Some(*id);
        }
    }
    if let Some(id) = toggle {
        hide::toggle_preset(app, id);
    }
    if let Some(id) = delete {
        hide::delete_preset(app, id);
    }

    // 対応させられず飛ばした項目と、読めなかったプリセットのファイル
    let mut notes: Vec<String> = state.skipped.iter().map(|k| k.describe(lang)).collect();
    notes.extend(
        app.view3d
            .pose
            .hide_presets
            .problems
            .iter()
            .map(|p| p.describe(lang)),
    );
    if !notes.is_empty() {
        let at = rows.row(t::ROW_HEIGHT, 2.0);
        super::warning_row(
            ui,
            at,
            "pose.hide.notes",
            notes.len(),
            &notes.join("\n"),
            lang,
        );
    }
    rows.space(2.0);
}
