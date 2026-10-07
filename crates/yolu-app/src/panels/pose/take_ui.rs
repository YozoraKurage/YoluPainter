//! 「テイク」の節: FBX の中のテイクを選ぶ箱・フレームのつまみ（値の箱を押すと数を打てる）・「ポーズにする」のボタン。テイクの無いモデルでは
//! 節ごと出さない（`content`）。求めるのと当てるのは `view3d::pose::takes`。ここは並べて、押されたことを渡すだけ。

use egui::Ui;

use crate::m2_menu::Popup;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};
use crate::view3d::pose::{takes, PoseAction};

pub(super) fn show(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    let free = !app.is_stroking();
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    let Some((source, take)) = s.takes.chosen() else {
        return;
    };
    let Some(range) = s.takes.chosen_take().map(|t| (t.first_frame, t.last_frame)) else {
        return;
    };
    // ギズモ・欄のドラッグの最中は、今のポーズが途中の値なので当てない
    let busy = app.view3d.pose.drag.is_some() || s.is_editing();
    let running = s.takes.is_running();
    let label = s.takes.label(source, take);
    let frame = s.takes.frame();

    let row = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, anchor) = w::dropdown(
        ui,
        row,
        "pose.take",
        None,
        &label,
        Some(lang.pick("FBX の中のテイク", "Take in the FBX")),
        free,
        0.0,
    );
    if response.clicked() {
        super::super::properties::open_popup(app, ui.ctx(), Popup::Take, anchor, anchor.width());
    }

    let at = rows.slider_row();
    let out = w::slider(
        ui,
        at,
        "pose.take.frame",
        frame as f32,
        &SliderSpec::new(
            lang.pick("フレーム", "Frame"),
            range.0 as f32,
            range.1 as f32,
            NumberFormat::int(""),
        )
        .tooltip(lang.pick("テイクの中の時刻（フレーム）", "Time in the take (frame)"))
        .enabled(free && range.1 > range.0),
    );
    if out.changed || out.released {
        takes::set_frame(app, out.value.round() as i64);
    }

    let row = rows.row(t::ROW_HEIGHT, 4.0);
    if w::button(
        ui,
        row,
        "pose.take.apply",
        lang.pick("ポーズにする", "Apply as Pose"),
        false,
        free && !busy && !running,
        Some(lang.pick(
            "このテイクのこのフレームのポーズにする（ポーズの取り消しに 1 段積む）",
            "Pose the model as in this frame of the take (one pose undo step)",
        )),
        None,
    )
    .clicked()
    {
        app.apply(Action::Pose(PoseAction::ApplyTake));
    }
}

/// テイクを選ぶポップアップの項目（選んでいるものに印）。
pub fn entries(app: &AppState) -> Vec<Entry<Action>> {
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return Vec::new();
    };
    let chosen = s.takes.chosen();
    s.takes
        .all()
        .into_iter()
        .map(|(source, take)| {
            Entry::item(
                s.takes.label(source, take),
                Action::Pose(PoseAction::ChooseTake(source, take)),
            )
            .radio(chosen == Some((source, take)))
        })
        .collect()
}
