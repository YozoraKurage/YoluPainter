//! ポーズのプリセット: 今のポーズ（骨ごとの位置・回転・大きさ）に名前を付けて、個人の設定のフォルダに残し、あとで当てる。
//! 保存の形と置き場は `store`、左右の名前の決まりは `mirror`。ここは、ポーズ ⇄ 項目の変換と、メニューや欄からの操作。
//!
//! - 項目は骨ごとの「休みの形（`Bone::rest`）からの差」。休みの形と違う骨だけを持つ: 平行移動は差（足し算）、回転は骨のローカルの差
//!   （休みの回転の逆 × 今の回転。当てるときは 休みの回転 × 差）、大きさは比。骨は名前の道で持つので、骨の名前の組が同じモデルへ
//!   当てられる。BlendShape の重みは持たない（当てても今のまま）。
//! - 当てるのは「休みの形 + 項目」: 項目に無い骨は休みの形へ戻る（保存したポーズそのもの）。合わない骨（名前の道に合う骨が無い・同じ
//!   名前が並ぶ）は飛ばして理由を残し（欄の知らせ）、1 つも合わないときはポーズを変えない。当てるのはポーズの取り消しの 1 段。描いている
//!   最中と、続けて変える操作の途中は当てない。
//! - 左右を反転して当てる: 名前の決まり（`mirror`）で対になる骨へ、差の x を反転して（平行移動は x の符号、回転は y・z の符号）当てる。
//!   対にならない骨はそのまま当てる。左右の休みの形が鏡像でない対（ローカルの軸の向きが鏡像でないリグ）は、取り違えないよう理由つきで
//!   飛ばす。親の鎖が左右対称なリグが前提（親の骨の休みの形までは確かめない）。

pub mod mirror;
pub mod store;

use yolu_core::glam::{Quat, Vec3};
use yolu_core::skin::{BonePathError, BoneTransform, Pose, Rig};

use self::store::{PoseEntry, StoreError};
use crate::lang::Lang;
use crate::state::AppState;
use crate::view3d::model::ViewError;

/// 名前を付けないで保存したときの名前。
pub fn default_name(lang: Lang) -> &'static str {
    lang.pick("ポーズ", "Pose")
}

/// 項目を骨に対応させられなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 骨の名前の道に合う骨が無い。
    NotFound,
    /// 同じ名前の骨が同じ親の下に複数ある。
    Ambiguous,
    /// 左右反転の相手の骨が無い。
    MirrorNotFound,
    /// 左右反転の相手の名前の骨が複数ある。
    MirrorAmbiguous,
    /// 左右の骨の休みの形が鏡像でない。
    MirrorAsymmetric,
    /// 同じ骨へ当てる項目が 2 つ以上ある（先のものを当てた）。
    Overlap,
}

/// 飛ばした項目。
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub preset: String,
    pub path: String,
    pub reason: SkipReason,
}

impl Skipped {
    pub fn describe(&self, lang: Lang) -> String {
        let reason = match self.reason {
            SkipReason::NotFound => lang.pick("ボーンがありません", "Bone not found"),
            SkipReason::Ambiguous => lang.pick(
                "同じ名前のボーンが複数あります",
                "Several bones share the name",
            ),
            SkipReason::MirrorNotFound => {
                lang.pick("対になるボーンがありません", "No mirrored bone")
            }
            SkipReason::MirrorAmbiguous => lang.pick(
                "対になる名前のボーンが複数あります",
                "Several bones share the mirrored name",
            ),
            SkipReason::MirrorAsymmetric => lang.pick(
                "左右の休みの形が鏡像ではありません",
                "Rest poses are not mirror images",
            ),
            SkipReason::Overlap => lang.pick(
                "同じボーンへの項目が重なっています",
                "Two entries target the same bone",
            ),
        };
        format!("{}: {} ({reason})", self.preset, self.path)
    }
}

/// 休みの形からの差を、骨の変換の形で。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Delta {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

/// 大きさの比（休みの大きさが 0 の軸は、比を表せないので 1）。
fn ratio(value: f32, base: f32) -> f32 {
    if base.abs() > 1.0e-8 {
        value / base
    } else {
        1.0
    }
}

/// 休みの形から今の変換への差。
fn delta_of(rest: &BoneTransform, local: &BoneTransform) -> Delta {
    let mut rotation = (rest.rotation.inverse() * local.rotation).normalize();
    if rotation.w < 0.0 {
        // q と -q は同じ回転（ファイルの値を 1 通りに）
        rotation = -rotation;
    }
    Delta {
        translation: local.translation - rest.translation,
        rotation,
        scale: Vec3::new(
            ratio(local.scale.x, rest.scale.x),
            ratio(local.scale.y, rest.scale.y),
            ratio(local.scale.z, rest.scale.z),
        ),
    }
}

/// 休みの形に差を重ねた変換。
fn compose(rest: &BoneTransform, d: &Delta) -> BoneTransform {
    BoneTransform {
        translation: rest.translation + d.translation,
        rotation: (rest.rotation * d.rotation).normalize(),
        scale: rest.scale * d.scale,
    }
}

/// 左右（x の符号）を反転した差。
fn mirrored(d: &Delta) -> Delta {
    Delta {
        translation: Vec3::new(-d.translation.x, d.translation.y, d.translation.z),
        rotation: Quat::from_xyzw(d.rotation.x, -d.rotation.y, -d.rotation.z, d.rotation.w),
        scale: d.scale,
    }
}

/// 左右の骨の休みの形が鏡像か（`source` を左右反転したものが `target` に近いか）。
fn rest_is_mirror(source: &BoneTransform, target: &BoneTransform) -> bool {
    let m = mirrored(&Delta {
        translation: source.translation,
        rotation: source.rotation,
        scale: source.scale,
    });
    let tolerance = 1.0e-3 * (1.0 + target.translation.length().max(source.translation.length()));
    (m.translation - target.translation).length() <= tolerance
        && m.rotation.dot(target.rotation).abs() >= 0.9999
        && (m.scale - target.scale).abs().max_element() <= 1.0e-3
}

/// 項目の変換（`Delta`）。
fn entry_delta(e: &PoseEntry) -> Delta {
    Delta {
        translation: e.translation,
        rotation: e.rotation,
        scale: e.scale,
    }
}

/// 休みの形と違う骨があるか（BlendShape は見ない）。
pub fn has_bone_changes(rig: &Rig, pose: &Pose) -> bool {
    pose.locals
        .iter()
        .zip(rig.bones())
        .any(|(local, bone)| *local != bone.rest)
}

/// 今のポーズの、休みの形と違う骨の項目（骨の並びの順）。名前の道が骨に決まらない骨（同じ親の下に同じ名前がある）は項目にできず、
/// その名前を返す。
pub fn entries_from_pose(rig: &Rig, pose: &Pose) -> (Vec<PoseEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut unsaved = Vec::new();
    for (i, (local, bone)) in pose.locals.iter().zip(rig.bones()).enumerate() {
        if *local == bone.rest {
            continue;
        }
        let path = rig.bone_path(i);
        if rig.resolve_bone_path(&path) != Ok(i) {
            unsaved.push(bone.name.clone());
            continue;
        }
        let d = delta_of(&bone.rest, local);
        entries.push(PoseEntry {
            path,
            translation: d.translation,
            rotation: d.rotation,
            scale: d.scale,
        });
    }
    (entries, unsaved)
}

/// 項目を当てたポーズ。
#[derive(Debug)]
pub struct Built {
    pub pose: Pose,
    /// 骨へ当てた項目の数。
    pub applied: usize,
    pub skipped: Vec<Skipped>,
}

/// 休みの形に項目を重ねたポーズ（BlendShape の重みは `current` のまま）。合わない項目は飛ばして理由を `skipped` へ。
pub fn build_pose(
    rig: &Rig,
    current: &Pose,
    preset: &str,
    entries: &[PoseEntry],
    mirror: bool,
) -> Built {
    let mut pose = rig.rest_pose();
    pose.blend_weights = current.blend_weights.clone();
    let mut taken = vec![false; rig.bones().len()];
    let mut applied = 0;
    let mut skipped = Vec::new();
    for e in entries {
        let target_path = if mirror {
            mirror::mirror_path(&e.path)
        } else {
            None
        };
        let flipped = target_path.is_some();
        let path = target_path.as_deref().unwrap_or(&e.path);
        let skip = |reason| Skipped {
            preset: preset.to_owned(),
            path: e.path_text(),
            reason,
        };
        let bone = match rig.resolve_bone_path(path) {
            Ok(b) => b,
            Err(err) => {
                let ambiguous = matches!(err, BonePathError::Ambiguous { .. });
                skipped.push(skip(match (flipped, ambiguous) {
                    (false, false) => SkipReason::NotFound,
                    (false, true) => SkipReason::Ambiguous,
                    (true, false) => SkipReason::MirrorNotFound,
                    (true, true) => SkipReason::MirrorAmbiguous,
                }));
                continue;
            }
        };
        let mut d = entry_delta(e);
        if flipped {
            // 左右の休みの形が鏡像でなければ、反転した差は別の向きの回転になる（元の骨が今のモデルにあるときだけ確かめられる）
            if let Ok(source) = rig.resolve_bone_path(&e.path) {
                if !rest_is_mirror(&rig.bones()[source].rest, &rig.bones()[bone].rest) {
                    skipped.push(skip(SkipReason::MirrorAsymmetric));
                    continue;
                }
            }
            d = mirrored(&d);
        }
        if taken[bone] {
            skipped.push(skip(SkipReason::Overlap));
            continue;
        }
        taken[bone] = true;
        pose.locals[bone] = compose(&rig.bones()[bone].rest, &d);
        applied += 1;
    }
    Built {
        pose,
        applied,
        skipped,
    }
}

fn save_error(lang: Lang, e: &StoreError) -> String {
    format!(
        "{}: {}",
        lang.pick("ポーズを保存できません", "Cannot save the pose"),
        e.describe(lang)
    )
}

/// ポーズを変えている最中（ギズモのドラッグ・欄のドラッグ）か。
fn is_editing(app: &AppState) -> bool {
    app.view3d.pose.drag.is_some()
        || app
            .view3d
            .pose
            .session
            .as_ref()
            .is_some_and(|s| s.is_editing())
}

/// 今のポーズを名前を付けてプリセットに保存する（名前が空なら既定の名前。同じ名前があれば番号を付ける）。名前の道が決まらない骨は
/// 保存できない（知らせる）。保存できたらその番号。
pub fn save_preset(app: &mut AppState, name: &str) -> Option<u32> {
    let lang = app.lang;
    let s = app.view3d.pose.session.as_ref()?;
    if is_editing(app) {
        return None;
    }
    let (entries, unsaved) = entries_from_pose(&s.rig, s.pose());
    let name = if name.trim().is_empty() {
        default_name(lang)
    } else {
        name
    };
    match app.view3d.pose.pose_presets.add(name, entries) {
        Ok(id) => {
            if !unsaved.is_empty() {
                let names = unsaved.join(lang.pick("・", ", "));
                app.message = lang.pick(
                    format!("同じ名前のボーンがあり、保存できないボーン: {names}"),
                    format!("Not saved (same-named bones): {names}"),
                );
            }
            Some(id)
        }
        Err(e) => {
            app.message = save_error(lang, &e);
            None
        }
    }
}

/// プリセットを今のポーズで上書きする（名前はそのまま）。上書きできたら true。
pub fn overwrite_preset(app: &mut AppState, id: u32) -> bool {
    let lang = app.lang;
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return false;
    };
    if is_editing(app) || app.view3d.pose.pose_presets.get(id).is_none() {
        return false;
    }
    let (entries, unsaved) = entries_from_pose(&s.rig, s.pose());
    match app.view3d.pose.pose_presets.replace(id, entries) {
        Ok(()) => {
            let name = app
                .view3d
                .pose
                .pose_presets
                .get(id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            app.message = lang.pick(
                format!("ポーズ {name} を今のポーズで上書きしました"),
                format!("Overwrote pose {name} with the current pose"),
            );
            if !unsaved.is_empty() {
                let names = unsaved.join(lang.pick("・", ", "));
                app.message += &lang.pick(
                    format!("（保存できないボーン: {names}）"),
                    format!(" (not saved: {names})"),
                );
            }
            true
        }
        Err(e) => {
            app.message = save_error(lang, &e);
            false
        }
    }
}

/// プリセットの名前を変える（同じ名前があれば番号を付ける）。変えられたら true。
pub fn rename_preset(app: &mut AppState, id: u32, name: &str) -> bool {
    let lang = app.lang;
    match app.view3d.pose.pose_presets.rename(id, name) {
        Ok(_) => true,
        Err(e) => {
            app.message = format!(
                "{}: {}",
                lang.pick("名前を変えられません", "Cannot rename the pose"),
                e.describe(lang)
            );
            false
        }
    }
}

/// プリセットを消す（ファイルも）。消せたら true。
pub fn delete_preset(app: &mut AppState, id: u32) -> bool {
    let lang = app.lang;
    match app.view3d.pose.pose_presets.remove(id) {
        Ok(()) => {
            if app.view3d.pose.preset_rename == Some(id) {
                app.view3d.pose.preset_rename = None;
            }
            true
        }
        Err(e) => {
            app.message = format!(
                "{}: {}",
                lang.pick("ポーズを消せません", "Cannot delete the pose"),
                e.describe(lang)
            );
            false
        }
    }
}

/// プリセットをポーズに当てる（`mirror` なら左右を反転して）。ポーズの取り消しの 1 段。描いている最中・変えている最中は当てない。
/// 1 つも合わない項目だけのプリセットは、ポーズを変えず理由だけ残す。当てられたら true。
pub fn apply_preset(app: &mut AppState, id: u32, mirror: bool) -> bool {
    let lang = app.lang;
    if app.is_stroking() {
        app.message = lang.view_error(&ViewError::Stroking);
        return false;
    }
    if is_editing(app) {
        return false;
    }
    let Some(preset) = app.view3d.pose.pose_presets.get(id).cloned() else {
        return false;
    };
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return false;
    };
    let built = build_pose(&s.rig, s.pose(), &preset.name, &preset.entries, mirror);
    let skipped = built.skipped.len();
    let nothing_fits = built.applied == 0 && !preset.entries.is_empty();
    let name = &preset.name;
    let tail = {
        let mut t = String::new();
        if mirror {
            t += lang.pick("（左右反転）", " (mirrored)");
        }
        if skipped > 0 {
            t += &lang.pick(
                format!("（飛ばしたボーン {skipped} 件）"),
                format!(" (skipped bones: {skipped})"),
            );
        }
        t
    };
    let mut result = Ok(());
    if !nothing_fits {
        result = super::set_pose(&mut app.view3d, built.pose);
    }
    if let Some(s) = app.view3d.pose.session.as_mut() {
        s.preset_notes = built.skipped;
    }
    match result {
        Ok(()) if nothing_fits => {
            app.message = lang.pick(
                format!("ポーズ {name} に合うボーンがありません{tail}"),
                format!("No bone fits pose {name}{tail}"),
            );
            false
        }
        Ok(()) => {
            app.message = lang.pick(
                format!("ポーズ {name} を当てました{tail}"),
                format!("Applied pose {name}{tail}"),
            );
            true
        }
        Err(e) => {
            app.message = lang.view_error(&e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Action, AppState};
    use crate::view3d::pose::{self, PoseAction};

    fn app() -> AppState {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(app.view3d.pose.session.is_some(), "{}", app.message);
        app
    }

    fn session(app: &AppState) -> &pose::PoseSession {
        app.view3d.pose.session.as_ref().unwrap()
    }

    fn bone(app: &AppState, name: &str) -> usize {
        session(app)
            .rig
            .bones()
            .iter()
            .position(|b| b.name == name)
            .unwrap()
    }

    /// 骨を回して・動かして・大きさを変えたポーズにする（取り消しに 1 段）。
    fn pose_bones(app: &mut AppState) {
        let mut p = session(app).pose().clone();
        let arm = bone(app, "右上腕");
        p.locals[arm].rotation = Quat::from_euler(yolu_core::glam::EulerRot::XYZ, 0.3, -0.5, 1.1);
        p.locals[arm].translation += Vec3::new(0.01, -0.02, 0.03);
        p.locals[arm].scale = Vec3::new(1.0, 1.25, 0.8);
        let head = bone(app, "頭");
        p.locals[head].rotation = Quat::from_rotation_y(0.4);
        pose::set_pose(&mut app.view3d, p).unwrap();
    }

    fn close(a: &BoneTransform, b: &BoneTransform) -> bool {
        (a.translation - b.translation).length() < 1.0e-5
            && a.rotation.dot(b.rotation).abs() > 0.999_999
            && (a.scale - b.scale).abs().max_element() < 1.0e-5
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yolu-pose-presets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn only_bones_that_differ_from_rest_are_saved_as_differences() {
        let mut app = app();
        let s = session(&app);
        assert!(!has_bone_changes(&s.rig, s.pose()));
        assert!(entries_from_pose(&s.rig, s.pose()).0.is_empty());
        pose_bones(&mut app);
        let s = session(&app);
        assert!(has_bone_changes(&s.rig, s.pose()));
        let (entries, unsaved) = entries_from_pose(&s.rig, s.pose());
        assert!(unsaved.is_empty());
        assert_eq!(entries.len(), 2, "動かした 2 本だけ");
        let arm = entries.iter().find(|e| e.path.last().unwrap() == "右上腕").unwrap();
        assert_eq!(arm.path, ["腰", "背骨", "胸", "右肩", "右上腕"]);
        assert!((arm.translation - Vec3::new(0.01, -0.02, 0.03)).length() < 1.0e-6, "平行移動は差");
        assert_eq!(arm.scale, Vec3::new(1.0, 1.25, 0.8), "大きさは比（休みは 1）");
        assert!(arm.rotation.w >= 0.0);
        // BlendShape だけの変更は、骨の項目にならない
        assert!(!has_bone_changes(&s.rig, &Pose { locals: s.rig.rest_pose().locals, ..s.pose().clone() }));
    }

    #[test]
    fn a_preset_applied_to_the_same_model_restores_the_pose_in_one_undo_step() {
        let mut app = app();
        pose_bones(&mut app);
        let saved = session(&app).pose().clone();
        let id = save_preset(&mut app, "構え").unwrap();
        // 休みの形へ戻してから当てる
        pose::reset(&mut app.view3d).unwrap();
        assert!(!session(&app).is_posed());
        let undo_before = session(&app).undo_len();
        assert!(apply_preset(&mut app, id, false), "{}", app.message);
        assert_eq!(session(&app).undo_len(), undo_before + 1, "取り消しの 1 段");
        for (i, (a, b)) in session(&app).pose().locals.iter().zip(&saved.locals).enumerate() {
            assert!(close(a, b), "骨 {i}: {a:?} {b:?}");
        }
        assert!(app.message.contains("構え") && !app.message.contains("飛ばした"), "{}", app.message);
        assert!(session(&app).preset_notes.is_empty());
        // 取り消すと当てる前（休みの形）
        assert!(pose::undo(&mut app.view3d).unwrap());
        assert!(!session(&app).is_posed());
        // 別のポーズから当てると、項目に無い骨は休みの形へ戻る（保存したポーズそのもの）
        let mut other = session(&app).pose().clone();
        let leg = bone(&app, "左足");
        other.locals[leg].rotation = Quat::from_rotation_x(0.5);
        pose::set_pose(&mut app.view3d, other).unwrap();
        assert!(apply_preset(&mut app, id, false));
        assert_eq!(session(&app).pose().locals[leg], session(&app).rig.bones()[leg].rest);
        assert!(close(&session(&app).pose().locals[bone(&app, "頭")], &saved.locals[bone(&app, "頭")]));
    }

    #[test]
    fn applying_keeps_the_current_blend_shape_weights() {
        let mut app = app();
        pose_bones(&mut app);
        let id = save_preset(&mut app, "構え").unwrap();
        pose::reset(&mut app.view3d).unwrap();
        let s = session(&app);
        let weights = s.pose().blend_weights.clone();
        // 試しの人形には BlendShape が無いので、重みの並びが同じまま残ることだけを確かめる
        assert!(apply_preset(&mut app, id, false));
        assert_eq!(session(&app).pose().blend_weights, weights);
    }

    #[test]
    fn a_preset_survives_a_restart_through_the_settings_folder() {
        let dir = temp_dir("restart");
        let mut app = app();
        app.view3d.pose.pose_presets.attach(dir.clone());
        pose_bones(&mut app);
        let saved = session(&app).pose().clone();
        let id = save_preset(&mut app, "構え").unwrap();
        assert!(dir.join(format!("pose-{id}.ylpose")).exists());
        // 別のアプリが同じフォルダを読んで、同じプリセットを当てる
        let mut again = self::tests::app();
        again.view3d.pose.pose_presets.attach(dir.clone());
        assert!(again.view3d.pose.pose_presets.problems.is_empty());
        assert!(apply_preset(&mut again, id, false), "{}", again.message);
        for (a, b) in session(&again).pose().locals.iter().zip(&saved.locals) {
            assert!(close(a, b));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_overwrite_and_delete_keep_the_other_presets() {
        let dir = temp_dir("manage");
        let mut app = app();
        app.view3d.pose.pose_presets.attach(dir.clone());
        pose_bones(&mut app);
        let a = save_preset(&mut app, "構え").unwrap();
        let b = save_preset(&mut app, "").unwrap();
        assert_eq!(app.view3d.pose.pose_presets.get(b).unwrap().name, "ポーズ", "名前が空なら既定の名前");
        // 名前を変える
        assert!(rename_preset(&mut app, a, "走り"));
        assert_eq!(app.view3d.pose.pose_presets.get(a).unwrap().name, "走り");
        assert!(!rename_preset(&mut app, a, "  "));
        assert!(app.message.contains("名前を変えられません"), "{}", app.message);
        // 今のポーズ（休みの形）で上書き: 項目が 0 になり、名前はそのまま（データとしては許す。ポーズの欄は休みの形では
        // 上書きのボタンを押せなくする）
        pose::reset(&mut app.view3d).unwrap();
        assert!(overwrite_preset(&mut app, a));
        assert!(app.message.contains("走り"), "{}", app.message);
        let p = app.view3d.pose.pose_presets.get(a).unwrap();
        assert_eq!((p.name.as_str(), p.entries.len()), ("走り", 0));
        assert_eq!(app.view3d.pose.pose_presets.get(b).unwrap().entries.len(), 2, "ほかは変わらない");
        // 休みの形のプリセットを当てると、ポーズが休みの形へ
        pose_bones(&mut app);
        assert!(apply_preset(&mut app, a, false));
        assert!(!session(&app).is_posed());
        // 消す
        assert!(delete_preset(&mut app, a));
        assert!(app.view3d.pose.pose_presets.get(a).is_none());
        assert!(!dir.join(format!("pose-{a}.ylpose")).exists());
        assert!(dir.join(format!("pose-{b}.ylpose")).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bones_that_do_not_fit_are_skipped_with_reasons_and_the_rest_apply() {
        let mut app = app();
        let id = app
            .view3d
            .pose
            .pose_presets
            .add(
                "合わない",
                vec![
                    PoseEntry {
                        path: ["腰", "背骨", "胸", "首", "頭"].map(String::from).to_vec(),
                        translation: Vec3::ZERO,
                        rotation: Quat::from_rotation_y(0.5),
                        scale: Vec3::ONE,
                    },
                    PoseEntry {
                        path: ["別の根", "頭"].map(String::from).to_vec(),
                        translation: Vec3::ZERO,
                        rotation: Quat::from_rotation_y(0.5),
                        scale: Vec3::ONE,
                    },
                    PoseEntry {
                        path: ["腰", "しっぽ"].map(String::from).to_vec(),
                        translation: Vec3::ZERO,
                        rotation: Quat::from_rotation_y(0.5),
                        scale: Vec3::ONE,
                    },
                ],
            )
            .unwrap();
        assert!(apply_preset(&mut app, id, false));
        assert!(session(&app).is_posed(), "合う骨は当たる");
        let notes: Vec<_> = session(&app).preset_notes.iter().map(|k| (k.path.clone(), k.reason)).collect();
        assert_eq!(
            notes,
            [
                ("別の根/頭".to_string(), SkipReason::NotFound),
                ("腰/しっぽ".to_string(), SkipReason::NotFound),
            ]
        );
        assert!(app.message.contains("飛ばしたボーン 2 件"), "{}", app.message);
        for k in &session(&app).preset_notes {
            assert!(k.describe(Lang::Ja).ends_with("(ボーンがありません)"), "{}", k.describe(Lang::Ja));
            assert!(k.describe(Lang::En).ends_with("(Bone not found)"), "{}", k.describe(Lang::En));
        }
        // 1 つも合わないプリセットは、ポーズを変えない（休みの形へ戻さない）
        let before = session(&app).pose().clone();
        let none = app
            .view3d
            .pose
            .pose_presets
            .add(
                "全部合わない",
                vec![PoseEntry {
                    path: vec!["別の根".to_string()],
                    translation: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                }],
            )
            .unwrap();
        let undo = session(&app).undo_len();
        assert!(!apply_preset(&mut app, none, false));
        assert_eq!(session(&app).pose(), &before);
        assert_eq!(session(&app).undo_len(), undo);
        assert!(app.message.contains("合うボーンがありません"), "{}", app.message);
        assert_eq!(session(&app).preset_notes.len(), 1, "理由は残す");
    }

    #[test]
    fn same_named_bones_are_not_saved_and_applying_names_them_as_ambiguous() {
        use yolu_core::skin::{Bone, Rig, RigBudget};
        let bone = |name: &str, parent: Option<u32>| Bone {
            name: name.into(),
            parent,
            rest: BoneTransform::IDENTITY,
        };
        // 同じ親の下に同じ名前の骨が 2 つ
        let rig = Rig::new(
            "重なり",
            vec![bone("根", None), bone("同じ", Some(0)), bone("同じ", Some(0)), bone("別", Some(0))],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &RigBudget::default(),
        )
        .unwrap();
        let mut pose = rig.rest_pose();
        for i in 1..4 {
            pose.locals[i].rotation = Quat::from_rotation_z(0.3);
        }
        let (entries, unsaved) = entries_from_pose(&rig, &pose);
        assert_eq!(entries.len(), 1, "決まる骨だけ: {entries:?}");
        assert_eq!(unsaved, ["同じ", "同じ"]);
        let ambiguous = PoseEntry {
            path: vec!["根".to_string(), "同じ".to_string()],
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_z(0.3),
            scale: Vec3::ONE,
        };
        let built = build_pose(&rig, &pose, "x", &[ambiguous], false);
        assert_eq!(built.applied, 0);
        assert_eq!(built.skipped[0].reason, SkipReason::Ambiguous);
    }

    #[test]
    fn nothing_is_applied_while_stroking_or_editing() {
        let mut app = app();
        pose_bones(&mut app);
        let id = save_preset(&mut app, "構え").unwrap();
        pose::reset(&mut app.view3d).unwrap();
        // 続けて変えている最中
        pose::begin_edit(&mut app.view3d).unwrap();
        assert!(!apply_preset(&mut app, id, false));
        assert!(!session(&app).is_posed());
        assert!(save_preset(&mut app, "途中").is_none(), "途中のポーズは保存しない");
        pose::end_edit(&mut app.view3d, false);
        // 描いている最中
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.view3d.input.stroke = Some(crate::state::StrokeSource::Mouse);
        let undo = session(&app).undo_len();
        assert!(!apply_preset(&mut app, id, false));
        assert_eq!(app.message, ViewError::Stroking.to_string());
        assert!(!session(&app).is_posed());
        assert_eq!(session(&app).undo_len(), undo);
        assert!(!apply_preset(&mut app, id, true));
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
        app.view3d.stroke_ended();
        assert!(apply_preset(&mut app, id, false));
    }

    #[test]
    fn a_mirrored_preset_moves_the_paired_bones_and_leaves_the_others_as_they_are() {
        let mut app = app();
        // 右腕を曲げ、頭を傾けたポーズ
        let mut p = session(&app).pose().clone();
        let r_arm = bone(&app, "右上腕");
        let l_arm = bone(&app, "左上腕");
        let head = bone(&app, "頭");
        let rest_arm = session(&app).rig.bones()[r_arm].rest;
        p.locals[r_arm].rotation = Quat::from_euler(yolu_core::glam::EulerRot::XYZ, 0.2, 0.4, -0.9);
        p.locals[r_arm].translation = rest_arm.translation + Vec3::new(0.02, 0.01, -0.03);
        p.locals[head].rotation = Quat::from_rotation_z(0.3);
        pose::set_pose(&mut app.view3d, p.clone()).unwrap();
        let id = save_preset(&mut app, "右を曲げる").unwrap();
        pose::reset(&mut app.view3d).unwrap();
        assert!(apply_preset(&mut app, id, true), "{}", app.message);
        assert!(app.message.contains("左右反転"), "{}", app.message);
        let s = session(&app);
        // 左腕: 差の y・z が逆の符号、x の平行移動が逆
        let rest_l = s.rig.bones()[l_arm].rest;
        let got = s.pose().locals[l_arm];
        let d = rest_l.rotation.inverse() * got.rotation;
        let src = rest_arm.rotation.inverse() * p.locals[r_arm].rotation;
        assert!(d.dot(Quat::from_xyzw(src.x, -src.y, -src.z, src.w)).abs() > 0.999_999, "{d:?} {src:?}");
        assert!(((got.translation - rest_l.translation) - Vec3::new(-0.02, 0.01, -0.03)).length() < 1.0e-6);
        // 右腕は休みの形のまま
        assert_eq!(s.pose().locals[r_arm], s.rig.bones()[r_arm].rest);
        // 対にならない骨（頭）は、そのまま
        assert!(close(&s.pose().locals[head], &p.locals[head]));
        // 反転を 2 回（元のモデルへ）で、元のポーズになる
        let again = save_preset(&mut app, "左を曲げる").unwrap();
        pose::reset(&mut app.view3d).unwrap();
        assert!(apply_preset(&mut app, again, true));
        for (a, b) in session(&app).pose().locals.iter().zip(&p.locals) {
            assert!(close(a, b), "{a:?} {b:?}");
        }
        assert!(session(&app).preset_notes.is_empty());
    }

    #[test]
    fn a_pair_whose_rest_poses_are_not_mirror_images_is_skipped_with_a_reason() {
        use yolu_core::skin::{Bone, Rig, RigBudget};
        let rest = |x: f32, roll: f32| BoneTransform {
            translation: Vec3::new(x, 1.0, 0.0),
            rotation: Quat::from_rotation_x(roll),
            scale: Vec3::ONE,
        };
        let bone = |name: &str, parent: Option<u32>, rest: BoneTransform| Bone {
            name: name.into(),
            parent,
            rest,
        };
        let build = |left_roll: f32| {
            Rig::new(
                "非対称",
                vec![
                    bone("Root", None, BoneTransform::IDENTITY),
                    bone("Arm_L", Some(0), rest(-0.2, left_roll)),
                    bone("Arm_R", Some(0), rest(0.2, 0.0)),
                    bone("Solo_L", Some(0), rest(-0.1, 0.0)),
                ],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                &RigBudget::default(),
            )
            .unwrap()
        };
        let entry = |path: &[&str]| PoseEntry {
            path: path.iter().map(|s| s.to_string()).collect(),
            translation: Vec3::new(0.01, 0.0, 0.0),
            rotation: Quat::from_rotation_z(0.5),
            scale: Vec3::ONE,
        };
        // 鏡像の休みの形: 当たる
        let rig = build(0.0);
        let built = build_pose(&rig, &rig.rest_pose(), "p", &[entry(&["Root", "Arm_R"])], true);
        assert_eq!((built.applied, built.skipped.len()), (1, 0), "{:?}", built.skipped);
        assert!(built.pose.locals[1].rotation.dot(Quat::from_rotation_z(-0.5)).abs() > 0.999_999);
        // 左だけ休みの回転が違う（鏡像でない）: 飛ばして理由
        let rig = build(0.6);
        let built = build_pose(&rig, &rig.rest_pose(), "p", &[entry(&["Root", "Arm_R"])], true);
        assert_eq!((built.applied, built.skipped.len()), (0, 1));
        assert_eq!(built.skipped[0].reason, SkipReason::MirrorAsymmetric);
        // 反転しないなら休みの形は見ない
        let built = build_pose(&rig, &rig.rest_pose(), "p", &[entry(&["Root", "Arm_R"])], false);
        assert_eq!(built.applied, 1);
        // 相手の骨が無い: 理由
        let built = build_pose(&rig, &rig.rest_pose(), "p", &[entry(&["Root", "Solo_L"])], true);
        assert_eq!(built.skipped[0].reason, SkipReason::MirrorNotFound);
        // 左右の両方を持つプリセット: 入れ替わる（重ならない）
        let rig = build(0.0);
        let built = build_pose(
            &rig,
            &rig.rest_pose(),
            "p",
            &[entry(&["Root", "Arm_L"]), entry(&["Root", "Arm_R"])],
            true,
        );
        assert_eq!((built.applied, built.skipped.len()), (2, 0));
        for r in [SkipReason::MirrorNotFound, SkipReason::MirrorAmbiguous, SkipReason::MirrorAsymmetric, SkipReason::Overlap] {
            let k = Skipped { preset: "p".into(), path: "a/b".into(), reason: r };
            assert!(!k.describe(Lang::Ja).is_empty() && k.describe(Lang::En).is_ascii());
        }
    }
}
