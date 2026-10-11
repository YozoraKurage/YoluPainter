//! ポーズの数値の編集と戻し（ボーンのインスペクター・BlendShape の戻し）。
//!
//! - どの変更も、ポーズの取り消しの並び（画素の取り消しとは別）の 1 段。数値の欄のドラッグは、押している間は続けて変える操作
//!   （`begin_edit`・`edit`）で、離したら 1 段にまとめる。打ち込んだ値・戻しは 1 回で 1 段。変わらなければ積まない。
//! - 戻し先は、骨ごとのファイルにあったときの変換（`Bone::rest`）と、BlendShape の初めの重み（`Rig::rest_pose`）。
//! - 回転の表示は Unity のインスペクターと同じ並びのオイラー角（度。Z・X・Y の順に当てる）。数値で入れた角は、同じ回転を別の角の
//!   組へ読み替えて見せない（`EulerHint`。入れた回転のままなら入れた角を見せ、ギズモや取り消しで回転が替わったら回転から求め直す）。
//! - 描いている最中は変えない（ポーズの決まり。理由を知らせる）。

use yolu_core::glam::{EulerRot, Quat, Vec3};
use yolu_core::skin::{BoneTransform, Pose, Rig};

use super::{begin_edit, edit, end_edit, set_pose, PoseSession};
use crate::state::AppState;

/// 骨の変換の 1 項目。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Position,
    Rotation,
    Scale,
}

/// 戻す範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    /// 骨 1 つの 1 項目（位置だけ・回転だけ・大きさだけ）。
    Part(usize, Part),
    /// 骨 1 つ（位置・回転・大きさ）。
    Bone(usize),
    /// 骨とその子孫の全部。
    Subtree(usize),
    /// 全部の骨（BlendShape はそのまま）。
    AllBones,
    /// BlendShape 1 つ（メッシュの番号・メッシュの中の番号）。
    Shape(usize, usize),
    /// 全部の BlendShape（骨はそのまま）。
    AllShapes,
}

/// 入れたオイラー角（度、X・Y・Z の欄の値）と、そこから作った回転。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EulerHint {
    pub bone: usize,
    pub rotation: Quat,
    pub degrees: [f32; 3],
}

/// 回転 → Unity のインスペクターと同じ並びのオイラー角（度。−0 は 0）。
pub fn rotation_to_degrees(q: Quat) -> [f32; 3] {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    [x, y, z].map(|r| {
        let d = r.to_degrees();
        if d == 0.0 {
            0.0
        } else {
            d
        }
    })
}

/// オイラー角（度）→ 回転（Unity の Quaternion.Euler と同じ順）。
pub fn rotation_from_degrees(d: [f32; 3]) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        d[1].to_radians(),
        d[0].to_radians(),
        d[2].to_radians(),
    )
    .normalize()
}

/// 欄に見せる骨のオイラー角（入れた回転のままなら入れた角、でなければ回転から）。
pub fn euler_degrees(s: &PoseSession, bone: usize) -> [f32; 3] {
    let q = s.pose().locals[bone].rotation;
    match s.euler_hint {
        Some(h) if h.bone == bone && h.rotation == q => h.degrees,
        _ => rotation_to_degrees(q),
    }
}

/// 戻したあとのポーズ。
pub fn apply_reset(rig: &Rig, pose: &Pose, what: Reset) -> Pose {
    let rest = rig.rest_pose();
    let mut next = pose.clone();
    let mut part = |bone: usize, which: Option<Part>| {
        let (Some(local), Some(base)) = (next.locals.get_mut(bone), rest.locals.get(bone)) else {
            return;
        };
        match which {
            Some(Part::Position) => local.translation = base.translation,
            Some(Part::Rotation) => local.rotation = base.rotation,
            Some(Part::Scale) => local.scale = base.scale,
            None => *local = *base,
        }
    };
    match what {
        Reset::Part(bone, which) => part(bone, Some(which)),
        Reset::Bone(bone) => part(bone, None),
        Reset::Subtree(bone) => {
            for (i, on) in rig.subtree(bone).into_iter().enumerate() {
                if on {
                    part(i, None);
                }
            }
        }
        Reset::AllBones => next.locals.clone_from(&rest.locals),
        Reset::Shape(m, k) => {
            if let (Some(w), Some(base)) = (
                next.blend_weights.get_mut(m).and_then(|v| v.get_mut(k)),
                rest.blend_weights.get(m).and_then(|v| v.get(k)),
            ) {
                *w = *base;
            }
        }
        Reset::AllShapes => next.blend_weights.clone_from(&rest.blend_weights),
    }
    next
}

/// 戻す余地があるか（今のポーズが戻し先と違うか。ボタンを押せるかの判断）。
pub fn can_reset(rig: &Rig, pose: &Pose, what: Reset) -> bool {
    apply_reset(rig, pose, what) != *pose
}

/// 戻す（取り消しの並びに 1 段。変わらなければ積まない）。描いている最中と、続けて変える操作の途中は断る。
pub fn reset(app: &mut AppState, what: Reset) {
    if app.is_stroking() {
        app.refuse(
            crate::notice::Source::Pose,
            app.lang.view_error(&super::ViewError::Stroking),
        );
        return;
    }
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    if s.is_editing() {
        return;
    }
    let next = apply_reset(&s.rig, s.pose(), what);
    if let Err(e) = set_pose(&mut app.view3d, next) {
        app.notify(
            e.notice_kind(),
            crate::notice::Source::Pose,
            app.lang.view_error(&e),
        );
    }
}

/// インスペクターの欄の値の変更。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Position(Vec3),
    /// X・Y・Z の度。
    Rotation([f32; 3]),
    Scale(Vec3),
}

/// 骨の変換の 1 項目を入れる（続けて変える操作の途中。欄のドラッグの間は同じ操作で、離したら `finish_live_edit` が 1 段にする）。
/// 描いている最中は断って理由を知らせる。
pub fn set_field(app: &mut AppState, bone: usize, field: Field) {
    if app.is_stroking() {
        app.refuse(
            crate::notice::Source::Pose,
            app.lang.view_error(&super::ViewError::Stroking),
        );
        return;
    }
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    if bone >= s.pose().locals.len() {
        return;
    }
    let mut next = s.pose().clone();
    let hint = {
        let local: &mut BoneTransform = &mut next.locals[bone];
        match field {
            Field::Position(v) => {
                local.translation = v;
                None
            }
            Field::Rotation(d) => {
                local.rotation = rotation_from_degrees(d);
                Some(EulerHint {
                    bone,
                    rotation: local.rotation,
                    degrees: d,
                })
            }
            Field::Scale(v) => {
                local.scale = v;
                None
            }
        }
    };
    if next == *s.pose() {
        return;
    }
    live_edit(app, next);
    if let (Some(hint), Some(s)) = (hint, app.view3d.pose.session.as_mut()) {
        if s.pose().locals[bone].rotation == hint.rotation {
            s.euler_hint = Some(hint);
        }
    }
}

fn live_edit(app: &mut AppState, next: Pose) {
    let started = app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| s.is_editing());
    let result = if started {
        Ok(())
    } else {
        begin_edit(&mut app.view3d)
    }
    .and_then(|()| edit(&mut app.view3d, next));
    if let Err(e) = result {
        app.notify(
            e.notice_kind(),
            crate::notice::Source::Pose,
            app.lang.view_error(&e),
        );
    }
}

/// 続けて変える操作を、ポインタのボタンが離れていれば確定する（欄のドラッグを離した・打ち込んだ値）。ギズモのドラッグは自分で終える。
pub fn finish_live_edit(app: &mut AppState, pointer_down: bool) {
    // G/R/S の途中は、決める・やめるときに自分で終える
    if pointer_down || app.view3d.pose.drag.is_some() || crate::objects::transforming(app) {
        return;
    }
    if app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| s.is_editing())
    {
        end_edit(&mut app.view3d, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Action, AppState};
    use crate::view3d::pose::PoseAction;

    fn app() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(app.view3d.pose.session.is_some(), "{}", app.message);
        app
    }

    fn bone(app: &AppState, name: &str) -> usize {
        let s = app.view3d.pose.session.as_ref().unwrap();
        s.rig.bones().iter().position(|b| b.name == name).unwrap()
    }

    fn local(app: &AppState, b: usize) -> BoneTransform {
        app.view3d.pose.session.as_ref().unwrap().pose().locals[b]
    }

    fn undo_len(app: &AppState) -> usize {
        app.view3d.pose.session.as_ref().unwrap().undo_len()
    }

    fn rest(app: &AppState, b: usize) -> BoneTransform {
        app.view3d.pose.session.as_ref().unwrap().rig.bones()[b].rest
    }

    #[test]
    fn euler_degrees_round_trip_in_the_unity_order() {
        for d in [
            [0.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [0.0, 45.0, 0.0],
            [0.0, 0.0, -60.0],
            [20.0, 35.0, -50.0],
            [-80.0, 170.0, 10.0],
        ] {
            let back = rotation_to_degrees(rotation_from_degrees(d));
            for k in 0..3 {
                assert!((back[k] - d[k]).abs() < 1e-3, "{d:?} → {back:?}");
            }
        }
        // Unity の Quaternion.Euler(0, 90, 0) は Y まわり 90°
        let q = rotation_from_degrees([0.0, 90.0, 0.0]);
        assert!(
            (q * Vec3::Z - Vec3::X).length() < 1e-5,
            "Y の 90° は +Z を +X へ"
        );
        // −0 は出さない
        assert!(rotation_to_degrees(Quat::IDENTITY)
            .iter()
            .all(|d| d.is_sign_positive()));
    }

    #[test]
    fn a_numeric_field_is_one_undo_step_and_unity_style_angles_stay_as_typed() {
        let mut app = app();
        let upper = bone(&app, "右上腕");
        // ドラッグ: 続けて変えて、離したら 1 段
        for z in [10.0, 20.0, 35.0] {
            set_field(&mut app, upper, Field::Rotation([0.0, 0.0, z]));
        }
        assert_eq!(undo_len(&app), 0, "離すまでは積まない");
        finish_live_edit(&mut app, true);
        assert_eq!(undo_len(&app), 0, "押している間は終えない");
        finish_live_edit(&mut app, false);
        assert_eq!(undo_len(&app), 1);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(euler_degrees(s, upper), [0.0, 0.0, 35.0]);
        // 別の表し方になる角（X が 90° を超える）を入れても、入れた角のまま見える
        set_field(&mut app, upper, Field::Rotation([120.0, 10.0, 20.0]));
        finish_live_edit(&mut app, false);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(euler_degrees(s, upper), [120.0, 10.0, 20.0]);
        assert_eq!(undo_len(&app), 2);
        // 取り消すと、回転から求めた角（前の入力のヒントは合わないので使わない）
        assert!(crate::view3d::pose::undo(&mut app.view3d).unwrap());
        let s = app.view3d.pose.session.as_ref().unwrap();
        let shown = euler_degrees(s, upper);
        assert!(
            (shown[2] - 35.0).abs() < 1e-3 && shown[0].abs() < 1e-3,
            "{shown:?}"
        );
        // 位置・大きさも 1 段（値が同じなら積まない）
        set_field(&mut app, upper, Field::Position(Vec3::new(0.1, 0.2, 0.3)));
        finish_live_edit(&mut app, false);
        assert_eq!(local(&app, upper).translation, Vec3::new(0.1, 0.2, 0.3));
        let n = undo_len(&app);
        set_field(&mut app, upper, Field::Position(Vec3::new(0.1, 0.2, 0.3)));
        finish_live_edit(&mut app, false);
        assert_eq!(undo_len(&app), n, "同じ値は積まない");
        set_field(&mut app, upper, Field::Scale(Vec3::new(1.5, 1.0, 1.0)));
        finish_live_edit(&mut app, false);
        assert_eq!(local(&app, upper).scale, Vec3::new(1.5, 1.0, 1.0));
        assert_eq!(undo_len(&app), n + 1);
    }

    #[test]
    fn resets_cover_a_part_a_bone_its_subtree_all_bones_and_the_shapes() {
        let mut app = app();
        let (upper, lower, hand) = (
            bone(&app, "右上腕"),
            bone(&app, "右前腕"),
            bone(&app, "右手"),
        );
        let (spine, left) = (bone(&app, "背骨"), bone(&app, "左上腕"));
        // 全部を少しずつ動かす
        let mut p = app.view3d.pose.session.as_ref().unwrap().pose().clone();
        for b in [upper, lower, hand, spine, left] {
            p.locals[b].translation += Vec3::new(0.01, 0.02, 0.03);
            p.locals[b].rotation = Quat::from_rotation_z(0.4);
            p.locals[b].scale = Vec3::new(1.2, 1.0, 1.0);
        }
        p.blend_weights[0][0] = 70.0;
        p.blend_weights[5][0] = 30.0;
        crate::view3d::pose::set_pose(&mut app.view3d, p).unwrap();
        let steps = undo_len(&app);

        // 項目ごと
        reset(&mut app, Reset::Part(upper, Part::Position));
        assert_eq!(
            local(&app, upper).translation,
            rest(&app, upper).translation
        );
        assert_ne!(local(&app, upper).rotation, rest(&app, upper).rotation);
        assert_ne!(local(&app, upper).scale, rest(&app, upper).scale);
        reset(&mut app, Reset::Part(upper, Part::Rotation));
        assert_eq!(local(&app, upper).rotation, rest(&app, upper).rotation);
        assert_ne!(local(&app, upper).scale, rest(&app, upper).scale);
        reset(&mut app, Reset::Part(upper, Part::Scale));
        assert_eq!(local(&app, upper), rest(&app, upper), "3 項目で骨が戻る");
        assert_eq!(undo_len(&app), steps + 3, "どの戻しも 1 段");
        // 骨と子
        reset(&mut app, Reset::Bone(lower));
        assert_eq!(local(&app, lower), rest(&app, lower));
        assert_ne!(local(&app, hand), rest(&app, hand), "子はそのまま");
        reset(&mut app, Reset::Subtree(lower));
        assert_eq!(local(&app, hand), rest(&app, hand), "子まで戻る");
        assert_ne!(local(&app, spine), rest(&app, spine), "ほかの枝は動かない");
        // 戻す余地が無ければ積まない
        let n = undo_len(&app);
        reset(&mut app, Reset::Bone(upper));
        assert_eq!(undo_len(&app), n);
        // BlendShape 1 つ
        reset(&mut app, Reset::Shape(0, 0));
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(
            s.pose().blend_weights[0][0],
            s.rig.rest_pose().blend_weights[0][0]
        );
        assert_eq!(s.pose().blend_weights[5][0], 30.0);
        // 全部の BlendShape は骨に触れない
        reset(&mut app, Reset::AllShapes);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.pose().blend_weights, s.rig.rest_pose().blend_weights);
        assert_ne!(local(&app, spine), rest(&app, spine));
        // 全部の骨は BlendShape に触れない
        let mut p = s.pose().clone();
        p.blend_weights[0][0] = 40.0;
        crate::view3d::pose::set_pose(&mut app.view3d, p).unwrap();
        reset(&mut app, Reset::AllBones);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.pose().locals, s.rig.rest_pose().locals);
        assert_eq!(s.pose().blend_weights[0][0], 40.0);
        // 戻しも取り消せる
        assert!(crate::view3d::pose::undo(&mut app.view3d).unwrap());
        assert_ne!(local(&app, spine), rest(&app, spine));
    }

    #[test]
    fn can_reset_follows_what_differs() {
        let mut app = app();
        let upper = bone(&app, "右上腕");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(!can_reset(&s.rig, s.pose(), Reset::Bone(upper)));
        assert!(!can_reset(&s.rig, s.pose(), Reset::AllBones));
        set_field(&mut app, upper, Field::Position(Vec3::new(0.0, 0.5, 0.0)));
        finish_live_edit(&mut app, false);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(can_reset(
            &s.rig,
            s.pose(),
            Reset::Part(upper, Part::Position)
        ));
        assert!(!can_reset(
            &s.rig,
            s.pose(),
            Reset::Part(upper, Part::Rotation)
        ));
        assert!(can_reset(
            &s.rig,
            s.pose(),
            Reset::Subtree(bone(&app, "右肩"))
        ));
        assert!(!can_reset(&s.rig, s.pose(), Reset::AllShapes));
    }

    #[test]
    fn edits_and_resets_are_refused_while_stroking_and_while_a_drag_is_open() {
        let mut app = app();
        let upper = bone(&app, "右上腕");
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.view3d.input.stroke = Some(crate::state::StrokeSource::Mouse);
        set_field(&mut app, upper, Field::Position(Vec3::X));
        assert_eq!(app.message, super::super::ViewError::Stroking.to_string());
        assert_eq!(local(&app, upper), rest(&app, upper));
        app.message.clear();
        reset(&mut app, Reset::AllBones);
        assert_eq!(app.message, super::super::ViewError::Stroking.to_string());
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
        app.view3d.stroke_ended();
        // 続けて変えている途中は、戻しで段を割らない
        set_field(&mut app, upper, Field::Position(Vec3::X));
        assert!(app.view3d.pose.session.as_ref().unwrap().is_editing());
        reset(&mut app, Reset::AllBones);
        assert_eq!(
            local(&app, upper).translation,
            Vec3::X,
            "途中の操作は戻さない"
        );
        finish_live_edit(&mut app, false);
        reset(&mut app, Reset::AllBones);
        assert_eq!(local(&app, upper), rest(&app, upper));
    }
}
