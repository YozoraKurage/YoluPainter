//! モデルの今のポーズを .ylp に残す（根の `pose.json`。`yolu_io::pose`）ための、ポーズ ⇄ 保存する形の変換と、開いたときに戻す手順。
//!
//! - 保存するのは休みの形からの差だけ（骨ごとの平行移動の差・回転の差・大きさの比と、BlendShape ごとの重み）で、骨は名前の道、BlendShape は
//!   メッシュの名前と BlendShape の名前の組で持つ。ポーズのプリセットと同じ表し方（`presets`）なので、名前の組が同じモデルへ戻せる。
//! - 戻すのは「休みの形 + 保存した差」: 合わない項目（名前の道に合う骨が無い・同じ名前が並ぶ・メッシュと BlendShape の名前の組が決まらない）は
//!   飛ばして理由を残し（ポーズの欄の知らせ）、1 つも合わなければポーズを変えない。戻したポーズは取り消しの段にも「変更あり」の印にもならない
//!   （開いた直後の状態）。
//! - 保存するのは、プロジェクトのモデル（FBX）のポーズだけ。Live Link で Unity から受けるポーズは頂点の位置（骨ではない）で、Live Link のモデルは
//!   ポーズのセッションを持たないので、保存しない。Live Link のモデルが出ている間（セッションが無い間）の保存は、ファイルのポーズに触れない
//!   （消さず、受けたポーズで上書きもしない）。保存済みのポーズは、そのプロジェクトのモデルを読み直したときに戻る。

use yolu_core::glam::{Quat, Vec3};
use yolu_core::skin::{Pose, Rig};
use yolu_io::pose::{StoredBone, StoredPose, StoredShape};

use super::presets::{self, Built, SkipReason, Skipped};
use super::presets::store::PoseEntry;
use crate::lang::Lang;
use crate::state::AppState;

/// 保存の材料になるか（名前・値が .ylp の決まりに合う）。
fn savable_name(name: &str) -> bool {
    yolu_io::pose::name_ok(name)
}

/// 今のポーズから保存する形を作る。決まりに合わない項目（名前の道が決まらない・名前が決まりに合わない・数が多すぎる）は含めず、
/// その骨・BlendShape の名前を返す（呼び手が利用者に知らせる）。
pub fn stored_from_pose(rig: &Rig, pose: &Pose) -> (StoredPose, Vec<String>) {
    let (entries, mut unsaved) = presets::entries_from_pose(rig, pose);
    let mut out = StoredPose::default();
    for e in entries {
        let ok = !e.path.is_empty()
            && e.path.len() <= yolu_io::pose::MAX_PATH_DEPTH
            && e.path.iter().all(|n| savable_name(n))
            && out.bones.len() < yolu_io::pose::MAX_BONES
            && [e.translation, e.scale]
                .iter()
                .all(|v| v.to_array().iter().all(|x| x.is_finite() && x.abs() <= yolu_io::pose::MAX_MAGNITUDE));
        if !ok {
            unsaved.push(e.path.last().cloned().unwrap_or_default());
            continue;
        }
        out.bones.push(StoredBone {
            path: e.path,
            translation: e.translation.to_array(),
            rotation: e.rotation.to_array(),
            scale: e.scale.to_array(),
        });
    }
    // BlendShape: メッシュの名前と BlendShape の名前の組が 1 つに決まるものだけ
    let rest = rig.rest_pose();
    let meshes = rig.meshes();
    for (m, mesh) in meshes.iter().enumerate() {
        for (k, shape) in mesh.blend_shapes.iter().enumerate() {
            let (Some(&w), Some(&base)) = (
                pose.blend_weights.get(m).and_then(|v| v.get(k)),
                rest.blend_weights.get(m).and_then(|v| v.get(k)),
            ) else {
                continue;
            };
            if w == base {
                continue;
            }
            let same = meshes
                .iter()
                .flat_map(|x| x.blend_shapes.iter().map(move |s| (x, s)))
                .filter(|(x, s)| x.mesh.name == mesh.mesh.name && s.name == shape.name)
                .count();
            let ok = same == 1
                && savable_name(&mesh.mesh.name)
                && savable_name(&shape.name)
                && w.is_finite()
                && w.abs() <= yolu_io::pose::MAX_WEIGHT
                && out.shapes.len() < yolu_io::pose::MAX_SHAPES;
            if ok {
                out.shapes.push(StoredShape {
                    mesh: mesh.mesh.name.clone(),
                    name: shape.name.clone(),
                    weight: w,
                });
            } else {
                unsaved.push(format!("{}/{}", mesh.mesh.name, shape.name));
            }
        }
    }
    (out, unsaved)
}

/// 保存した形を休みの形に重ねたポーズ。合わない項目は飛ばして理由を `skipped` へ（`label` は理由の頭に付く名前）。
pub fn pose_from_stored(rig: &Rig, stored: &StoredPose, label: &str) -> Built {
    let entries: Vec<PoseEntry> = stored
        .bones
        .iter()
        .map(|b| PoseEntry {
            path: b.path.clone(),
            translation: Vec3::from_array(b.translation),
            rotation: Quat::from_array(b.rotation).normalize(),
            scale: Vec3::from_array(b.scale),
        })
        .collect();
    let mut built = presets::build_pose(rig, &rig.rest_pose(), label, &entries, false);
    let meshes = rig.meshes();
    for s in &stored.shapes {
        let matches: Vec<(usize, usize)> = meshes
            .iter()
            .enumerate()
            .flat_map(|(m, mesh)| {
                mesh.blend_shapes
                    .iter()
                    .enumerate()
                    .filter(|(_, shape)| mesh.mesh.name == s.mesh && shape.name == s.name)
                    .map(move |(k, _)| (m, k))
            })
            .collect();
        let skip = |reason| Skipped {
            preset: label.to_owned(),
            path: format!("{}/{}", s.mesh, s.name),
            reason,
        };
        match matches.as_slice() {
            [(m, k)] => {
                built.pose.blend_weights[*m][*k] = s.weight;
                built.applied += 1;
            }
            [] => built.skipped.push(skip(SkipReason::ShapeNotFound)),
            _ => built.skipped.push(skip(SkipReason::ShapeAmbiguous)),
        }
    }
    built
}

/// プロジェクトを開いてモデルを読み終えたとき: ファイルのポーズを戻す。戻したか・飛ばした項目・読めなかった理由を、開いたときの知らせの
/// 文にして返す（何も無ければ None）。ファイルに無いとき・ポーズのセッションが無いときは何もしない。
pub fn restore_from_project(app: &mut AppState) -> Option<String> {
    let lang = app.lang;
    let stored = match app.project.as_ref()?.project().pose() {
        Ok(Some(p)) => p,
        Ok(None) => return None,
        Err(e) => {
            return Some(format!(
                "{}: {}",
                lang.pick("ポーズを読めません（ファイルには残っています）", "Cannot read the pose (kept in the file)"),
                lang.io_error(&e)
            ))
        }
    };
    let session = app.view3d.pose.session.as_ref()?;
    let label = lang.pick("ファイルのポーズ", "Pose in file");
    let built = pose_from_stored(&session.rig, &stored, label);
    let total = stored.bones.len() + stored.shapes.len();
    let skipped = built.skipped.len();
    let nothing_fits = built.applied == 0 && total > 0;
    let mut result = Ok(());
    if !nothing_fits {
        result = super::restore_pose(&mut app.view3d, built.pose);
    }
    if let Some(s) = app.view3d.pose.session.as_mut() {
        s.preset_notes = built.skipped;
    }
    Some(match result {
        Err(e) => format!("{}: {}", lang.pick("ポーズを戻せません", "Cannot restore the pose"), lang.view_error(&e)),
        Ok(()) if nothing_fits => lang.pick(
            "ファイルのポーズに合うボーンがありません。".to_owned(),
            "No bone fits the pose in the file.".to_owned(),
        ),
        Ok(()) if skipped > 0 => lang.pick(
            format!("ポーズを戻しました（合わない項目 {skipped} 件）。"),
            format!("Pose restored (unmatched items: {skipped})."),
        ),
        Ok(()) => lang.pick("ポーズを戻しました。".to_owned(), "Pose restored.".to_owned()),
    })
}

/// 保存の材料: プロジェクトのモデルのポーズの今。
#[derive(Clone, Debug, PartialEq)]
pub enum PoseCapture {
    /// ファイルのポーズに触れない（ポーズのセッションが無い: モデルを読んでいる最中・見つからない・Live Link のモデル）。
    Keep,
    /// ファイルのポーズをこれにする（休みの形のままなら None: エントリを消す）。
    Write(Option<StoredPose>),
}

/// 今の状態から保存の材料を取る。保存できなかった項目（骨・BlendShape の名前）も返す。
/// プロジェクトのモデルが無い（試しの人形・立方体・まだ読んでいない）ときは、ファイルのポーズも外す（どのモデルのポーズでもなくなる。
/// モデルの参照も同じ保存で外れる）。
pub fn capture(app: &AppState) -> (PoseCapture, Vec<String>) {
    if app.np.model_file.is_none() {
        return (PoseCapture::Write(None), Vec::new());
    }
    let Some(session) = app.view3d.pose.session.as_ref() else {
        return (PoseCapture::Keep, Vec::new());
    };
    let (stored, unsaved) = stored_from_pose(&session.rig, session.pose());
    (PoseCapture::Write((!stored.is_rest()).then_some(stored)), unsaved)
}

/// 保存できなかった項目の知らせ（無ければ空。先頭に空白を置いて、保存の知らせの文へ続ける）。
pub fn unsaved_note(lang: Lang, unsaved: &[String]) -> String {
    if unsaved.is_empty() {
        return String::new();
    }
    let names = unsaved.join(lang.pick("・", ", "));
    lang.pick(
        format!(" ポーズに保存できない項目: {names}。"),
        format!(" Not saved in the pose: {names}."),
    )
}

/// ポーズの材料を、プロジェクトの `pose.json` へ書く（違うときだけ）。読めない `pose.json` は、書く中身があるときだけ上書きして true を返す
/// （呼び手が利用者に知らせる）。消すだけのときは、読めないエントリを黙って消さずに残す。
pub fn write_into(
    project: yolu_io::Project,
    pose: &PoseCapture,
    lang: Lang,
) -> Result<(yolu_io::Project, bool), String> {
    let PoseCapture::Write(next) = pose else {
        return Ok((project, false));
    };
    let mut overwritten = false;
    match project.pose() {
        Ok(stored) if stored == *next => return Ok((project, false)),
        Err(_) if next.is_none() => return Ok((project, false)),
        Err(_) => overwritten = true,
        Ok(_) => {}
    }
    let written = project.with_pose(next.as_ref()).map_err(|e| {
        format!("{}: {}", lang.pick("ポーズを書けません", "Cannot write the pose"), lang.io_error(&e))
    })?;
    Ok((written, overwritten))
}
