//! 選んだボーンのインスペクター（Unity のインスペクターに当たる）: 名前、位置・回転（度。Unity と同じ並びのオイラー角）・大きさを
//! 数値の欄（ドラッグで増減・クリックで打つ）で直し、項目ごとの戻すボタンと、ボーン・ボーンと子を戻すボタンを並べる。
//! 値の変更は続けて変える操作（`edit::set_field`）、戻しは 1 回で 1 段（`edit::reset`）で、どちらもポーズの取り消しの並びの 1 段。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::glam::Vec3;

use crate::state::AppState;
use crate::ui::numfield::{number_field, NumSpec};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, Rows};
use crate::view3d::pose::edit::{self, Field, Part, Reset};
use crate::view3d::shape_gizmo::{AXIS_X, AXIS_Y, AXIS_Z};

const LABEL_W: f32 = 62.0;
const RESET_W: f32 = 22.0;

pub(super) fn show(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, bone: usize) {
    let lang = app.lang;
    let free = !app.is_stroking() && app.view3d.pose.drag.is_none();
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    if bone >= s.pose().locals.len() {
        return;
    }
    let local = s.pose().locals[bone];
    let degrees = edit::euler_degrees(s, bone);
    let name = s.rig.bones()[bone].name.clone();
    // 戻す余地（続けて変えている途中は戻さない。取り消しの段を割らない）
    let resettable = free && !s.is_editing();
    let can = |what| resettable && edit::can_reset(&s.rig, s.pose(), what);
    let (can_pos, can_rot, can_scale) = (
        can(Reset::Part(bone, Part::Position)),
        can(Reset::Part(bone, Part::Rotation)),
        can(Reset::Part(bone, Part::Scale)),
    );
    let (can_bone, can_tree) = (can(Reset::Bone(bone)), can(Reset::Subtree(bone)));

    rows.space(2.0);
    let at = rows.row(t::ROW_HEIGHT, 2.0);
    w::text(
        ui.painter(),
        at,
        &w::fit(ui.painter(), &name, at.width(), t::LABEL_BOLD),
        t::LABEL_BOLD,
        Align::Left,
    );

    let mut field = None;
    let mut reset = None;
    let vec3 = |v: Vec3| [f64::from(v.x), f64::from(v.y), f64::from(v.z)];
    let (next, clicked) = vec3_row(
        ui,
        rows,
        "pose.inspector.position",
        lang.pick("位置", "Position"),
        vec3(local.translation),
        &NumSpec::new(-1.0e6, 1.0e6, 0.001, 3),
        lang.pick("位置（親からの相対）", "Position (relative to the parent)"),
        lang.pick("位置を戻す", "Reset position"),
        free,
        can_pos,
    );
    if let Some(v) = next {
        field = Some(Field::Position(Vec3::new(
            v[0] as f32,
            v[1] as f32,
            v[2] as f32,
        )));
    }
    if clicked {
        reset = Some(Reset::Part(bone, Part::Position));
    }
    let (next, clicked) = vec3_row(
        ui,
        rows,
        "pose.inspector.rotation",
        lang.pick("回転", "Rotation"),
        degrees.map(f64::from),
        &NumSpec::new(-360.0, 360.0, 0.5, 2),
        lang.pick(
            "回転（度。Unity のインスペクターと同じ並び）",
            "Rotation (degrees, same order as the Unity Inspector)",
        ),
        lang.pick("回転を戻す", "Reset rotation"),
        free,
        can_rot,
    );
    if let Some(v) = next {
        field = Some(Field::Rotation([v[0] as f32, v[1] as f32, v[2] as f32]));
    }
    if clicked {
        reset = Some(Reset::Part(bone, Part::Rotation));
    }
    let (next, clicked) = vec3_row(
        ui,
        rows,
        "pose.inspector.scale",
        lang.pick("大きさ", "Scale"),
        vec3(local.scale),
        &NumSpec::new(0.001, 1000.0, 0.005, 3),
        lang.pick("大きさ（親からの相対）", "Scale (relative to the parent)"),
        lang.pick("大きさを戻す", "Reset scale"),
        free,
        can_scale,
    );
    if let Some(v) = next {
        field = Some(Field::Scale(Vec3::new(
            v[0] as f32,
            v[1] as f32,
            v[2] as f32,
        )));
    }
    if clicked {
        reset = Some(Reset::Part(bone, Part::Scale));
    }

    let row = rows.row(24.0, 4.0);
    let halves = Rows::split(row, 2, 4.0);
    if w::button(
        ui,
        halves[0],
        "pose.inspector.reset_bone",
        lang.pick("ボーンを戻す", "Reset Bone"),
        false,
        can_bone,
        Some(lang.pick(
            "このボーンの位置・回転・大きさをファイルのポーズへ戻す",
            "Return this bone's position, rotation and scale to the file's pose",
        )),
        None,
    )
    .clicked()
    {
        reset = Some(Reset::Bone(bone));
    }
    if w::button(
        ui,
        halves[1],
        "pose.inspector.reset_tree",
        lang.pick("ボーンと子を戻す", "Reset Subtree"),
        false,
        can_tree,
        Some(lang.pick(
            "このボーンと、その下のすべてのボーンをファイルのポーズへ戻す",
            "Return this bone and every bone under it to the file's pose",
        )),
        None,
    )
    .clicked()
    {
        reset = Some(Reset::Subtree(bone));
    }
    rows.space(2.0);

    if let Some(f) = field {
        edit::set_field(app, bone, f);
    }
    if let Some(what) = reset {
        edit::reset(app, what);
    }
}

/// 名前と X・Y・Z の 3 つの数値の欄と、戻すボタンの 1 行（軸の色の下線）。変えた欄があれば新しい 3 つの値、戻すボタンを押したか。
#[allow(clippy::too_many_arguments)]
fn vec3_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    values: [f64; 3],
    spec: &NumSpec,
    tooltip: &str,
    reset_tooltip: &str,
    enabled: bool,
    can_reset: bool,
) -> (Option<[f64; 3]>, bool) {
    let row = rows.row(t::ROW_HEIGHT, 3.0);
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL_W, row.height())),
        &w::fit(ui.painter(), label, LABEL_W - 4.0, t::LABEL),
        t::LABEL,
        Align::Left,
    );
    let button = Rect::from_min_size(
        pos2(row.right() - RESET_W, row.top()),
        vec2(RESET_W, row.height()),
    );
    let clicked = w::icon_button(
        ui,
        button,
        (id, "reset"),
        "restart_alt",
        reset_tooltip,
        false,
        can_reset,
        14.0,
    )
    .clicked();
    let cells = Rows::split(
        Rect::from_min_max(
            pos2(row.left() + LABEL_W, row.top()),
            pos2(button.left() - 2.0, row.bottom()),
        ),
        3,
        3.0,
    );
    let mut next = values;
    let mut changed = false;
    for (k, cell) in cells.iter().enumerate() {
        let out = number_field(
            ui,
            *cell,
            (id, k),
            "", // 軸の見分けは下線の色（X 赤・Y 緑・Z 青）。文字を置くと値が入りきらない
            values[k],
            spec,
            Some(&format!("{}: {tooltip}", ["X", "Y", "Z"][k])),
            Some([AXIS_X, AXIS_Y, AXIS_Z][k]),
            enabled,
        );
        if out.changed {
            next[k] = out.value;
            changed = true;
        }
    }
    (changed.then_some(next), clicked)
}
