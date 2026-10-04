//! ボーンの影響で面を隠す。隠すのは 3D ビューの見え方と当たりだけ（描く・範囲の道具・スポイト・ブラシのカーソルは、隠した面を見ない）。
//! 2D のキャンバス・保存・書き出し・ポーズは変わらない。`.ylp` にも入れない。
//!
//! - 隠す面: 骨（子を含められる）の影響の重み（面の 3 頂点が、その骨の組に掛けているウェイトの平均。core の
//!   `Rig::triangle_bone_weights`）がしきい値以上の面（0 の面は隠さない）。項目ごとにしきい値を持ち、項目どうしは和。
//! - 隠し方は 2 つの組み合わせ: 手で足した項目（`HideState::entries`。骨の番号で持つ）と、入れているプリセット（`HideState::presets`。
//!   名前つきで個人の設定のフォルダに残る。骨は名前の道で持つので、同じ骨の名前の組のモデルへ持ち越せる。合わない項目は、理由を
//!   残して飛ばす）。プリセットは複数を同時に入れられ、全部の和で隠す。
//! - 全部の面が隠れてしまう隠し方は断る（3D ビューに何も残さない状態にしない）。描いている最中は変えない。
//! - 見せる形への入れ方は `View3dState::set_face_mask`（隠したマテリアルと同じ道。三角形の番号はマテリアルを隠す前の形のもの）。

pub mod store;

use std::sync::Arc;

use yolu_core::skin::{BonePathError, Rig};

use self::store::{PresetEntry, Presets, StoreError};
use crate::lang::Lang;
use crate::state::AppState;
use crate::view3d::model::ViewError;

/// 新しい項目のしきい値の既定（面の 3 頂点のウェイトの平均が半分以上なら隠す）。
pub const DEFAULT_THRESHOLD: f32 = 0.5;

/// 隠す面の印（受けたままの形の三角形の通し番号ごと）。
#[derive(Debug, PartialEq)]
pub struct FaceMask {
    hidden: Vec<bool>,
    count: usize,
}

impl FaceMask {
    pub fn new(hidden: Vec<bool>) -> FaceMask {
        let count = hidden.iter().filter(|h| **h).count();
        FaceMask { hidden, count }
    }
    /// 三角形の数。
    pub fn len(&self) -> usize {
        self.hidden.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }
    /// 隠す三角形の数。
    pub fn hidden_count(&self) -> usize {
        self.count
    }
    pub fn is_hidden(&self, triangle: usize) -> bool {
        self.hidden.get(triangle).copied().unwrap_or(false)
    }
}

/// 隠し方の項目 1 つ（骨とその子の影響）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HideEntry {
    pub bone: usize,
    /// 0〜1。
    pub threshold: f32,
    /// 子孫の骨の影響も含めるか。
    pub children: bool,
}

/// プリセットの項目を今のモデルへ対応させられなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 骨の名前の道に合う骨が無い。
    NotFound,
    /// 同じ名前の骨が同じ親の下に複数ある。
    Ambiguous,
}

/// 飛ばしたプリセットの項目。
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
        };
        format!("{}: {} ({reason})", self.preset, self.path)
    }
}

/// セッションの隠し方。
#[derive(Clone, Debug, PartialEq)]
pub struct HideState {
    /// 手で足した項目。
    pub entries: Vec<HideEntry>,
    /// 入れているプリセット（番号）。
    pub presets: Vec<u32>,
    /// 次に足す項目のしきい値と、子を含めるか（欄の値）。
    pub threshold: f32,
    pub children: bool,
    /// 入れているプリセットの項目のうち、今のモデルへ対応させられず飛ばしたもの。
    pub skipped: Vec<Skipped>,
}

impl Default for HideState {
    fn default() -> Self {
        HideState {
            entries: Vec::new(),
            presets: Vec::new(),
            threshold: DEFAULT_THRESHOLD,
            children: true,
            skipped: Vec::new(),
        }
    }
}

impl HideState {
    /// 何も隠していないか。
    pub fn is_clear(&self) -> bool {
        self.entries.is_empty() && self.presets.is_empty()
    }
}

/// 今の隠し方の項目の全部（手で足した項目と、入れているプリセットの項目を、骨の名前の道で今のモデルへ対応させたもの）と、
/// 対応させられず飛ばした項目。番号が無くなったプリセットは読み飛ばす。
pub fn effective(
    rig: &Rig,
    state: &HideState,
    presets: &Presets,
) -> (Vec<HideEntry>, Vec<Skipped>) {
    let mut entries: Vec<HideEntry> = state
        .entries
        .iter()
        .filter(|e| e.bone < rig.bones().len())
        .copied()
        .collect();
    let mut skipped = Vec::new();
    for id in &state.presets {
        let Some(preset) = presets.get(*id) else {
            continue;
        };
        for e in &preset.entries {
            match rig.resolve_bone_path(&e.path) {
                Ok(bone) => {
                    let entry = HideEntry {
                        bone,
                        threshold: e.threshold,
                        children: e.children,
                    };
                    if !entries.contains(&entry) {
                        entries.push(entry);
                    }
                }
                Err(err) => skipped.push(Skipped {
                    preset: preset.name.clone(),
                    path: e.path_text(),
                    reason: match err {
                        BonePathError::Ambiguous { .. } => SkipReason::Ambiguous,
                        _ => SkipReason::NotFound,
                    },
                }),
            }
        }
    }
    (entries, skipped)
}

/// 項目の和で隠す面（項目が無ければ None）。
pub fn mask_for(rig: &Rig, entries: &[HideEntry]) -> Option<FaceMask> {
    if entries.is_empty() {
        return None;
    }
    let mut hidden = vec![false; rig.triangle_count()];
    for e in entries {
        let selected = if e.children {
            rig.subtree(e.bone)
        } else {
            let mut one = vec![false; rig.bones().len()];
            if let Some(slot) = one.get_mut(e.bone) {
                *slot = true;
            }
            one
        };
        for (h, w) in hidden.iter_mut().zip(rig.triangle_bone_weights(&selected)) {
            if w > 0.0 && w >= e.threshold {
                *h = true;
            }
        }
    }
    Some(FaceMask::new(hidden))
}

enum Refresh {
    Done,
    /// 全部の面が隠れる。
    AllHidden,
}

/// 今の隠し方から面の印を組み直して、3D ビューへ入れる（全部の面が隠れるなら入れずに断る）。
fn refresh(app: &mut AppState) -> Refresh {
    let Some(s) = app.view3d.pose.session.as_ref() else {
        app.view3d.set_face_mask(None);
        return Refresh::Done;
    };
    let (entries, skipped) = effective(&s.rig, &s.hide, &app.view3d.pose.hide_presets);
    let mask = mask_for(&s.rig, &entries);
    if let Some(m) = &mask {
        if !m.is_empty() && m.hidden_count() >= m.len() {
            return Refresh::AllHidden;
        }
    }
    if let Some(s) = app.view3d.pose.session.as_mut() {
        s.hide.skipped = skipped;
    }
    app.view3d.set_face_mask(mask.map(Arc::new));
    Refresh::Done
}

/// 隠し方を変えて面の印を組み直す。描いている最中・全部の面が隠れる変え方は断って、元の隠し方へ戻す（理由を知らせる）。変えられたら true。
fn change(app: &mut AppState, f: impl FnOnce(&mut HideState)) -> bool {
    let lang = app.lang;
    if app.is_stroking() {
        app.message = lang.view_error(&ViewError::Stroking);
        return false;
    }
    let Some(s) = app.view3d.pose.session.as_mut() else {
        return false;
    };
    let before = s.hide.clone();
    f(&mut s.hide);
    match refresh(app) {
        Refresh::Done => true,
        Refresh::AllHidden => {
            if let Some(s) = app.view3d.pose.session.as_mut() {
                s.hide = before;
            }
            app.message = lang
                .pick(
                    "すべての面が隠れるため隠せません",
                    "Cannot hide every surface",
                )
                .into();
            false
        }
    }
}

/// 隠す項目を足す（同じ骨の項目があれば、しきい値と子を含めるかを入れ替える）。足した項目が単独で隠す面を 1 つも持たないときだけ知らせる
/// （すでに隠している同じ項目をもう一度足しても、面は隠れているので知らせない）。
pub fn hide_bone(app: &mut AppState, bone: usize) {
    let lang = app.lang;
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    if bone >= s.rig.bones().len() {
        return;
    }
    let entry = HideEntry {
        bone,
        threshold: s.hide.threshold,
        children: s.hide.children,
    };
    let changed = change(app, |h| {
        match h.entries.iter_mut().find(|e| e.bone == bone) {
            Some(e) => *e = entry,
            None => h.entries.push(entry),
        }
    });
    if !changed {
        return;
    }
    let alone_hides_nothing = app.view3d.pose.session.as_ref().is_some_and(|s| {
        mask_for(&s.rig, std::slice::from_ref(&entry)).is_none_or(|m| m.hidden_count() == 0)
    });
    if alone_hides_nothing {
        app.message = lang
            .pick("隠れる面がありません", "No surfaces to hide")
            .into();
    }
}

/// 今隠している面の数（見せる形に入れた印）。
pub fn hidden_total(app: &AppState) -> usize {
    app.view3d.face_mask().map_or(0, |m| m.hidden_count())
}

/// 手で足した項目を外す。
pub fn remove_entry(app: &mut AppState, index: usize) {
    change(app, |h| {
        if index < h.entries.len() {
            h.entries.remove(index);
        }
    });
}

/// すべて表示する（手で足した項目も、入れているプリセットも外す）。
pub fn show_all(app: &mut AppState) {
    change(app, |h| {
        h.entries.clear();
        h.presets.clear();
    });
}

/// プリセットを入れる・外す（入れているプリセットは全部の和）。
pub fn toggle_preset(app: &mut AppState, id: u32) {
    if app.view3d.pose.hide_presets.get(id).is_none() {
        return;
    }
    change(app, |h| match h.presets.iter().position(|p| *p == id) {
        Some(at) => {
            h.presets.remove(at);
        }
        None => h.presets.push(id),
    });
}

/// 次に足す項目の欄の値。
pub fn set_next(app: &mut AppState, threshold: f32, children: bool) {
    if let Some(s) = app.view3d.pose.session.as_mut() {
        s.hide.threshold = threshold.clamp(0.0, 1.0);
        s.hide.children = children;
    }
}

/// 今の隠し方（手で足した項目と入れているプリセットの和）を、名前を付けてプリセットに保存する。骨の名前の道が決まらない骨は
/// 保存できない（知らせる）。保存できたらその番号。
pub fn save_preset(app: &mut AppState, name: &str) -> Option<u32> {
    let lang = app.lang;
    let s = app.view3d.pose.session.as_ref()?;
    let (entries, _) = effective(&s.rig, &s.hide, &app.view3d.pose.hide_presets);
    let mut saved: Vec<PresetEntry> = Vec::new();
    let mut unsaved: Vec<String> = Vec::new();
    for e in &entries {
        let path = s.rig.bone_path(e.bone);
        if s.rig.resolve_bone_path(&path) != Ok(e.bone) {
            unsaved.push(s.rig.bones()[e.bone].name.clone());
            continue;
        }
        let entry = PresetEntry {
            path,
            threshold: e.threshold,
            children: e.children,
        };
        if !saved.contains(&entry) {
            saved.push(entry);
        }
    }
    let fallback = lang.pick("隠し方", "Hide Set");
    let name = if name.trim().is_empty() {
        fallback
    } else {
        name
    };
    match app.view3d.pose.hide_presets.add(name, saved) {
        Ok(id) => {
            if !unsaved.is_empty() {
                let names = unsaved.join(lang.pick("・", ", "));
                app.message = lang.pick(
                    format!("同じ名前のボーンがあり、保存できない項目: {names}"),
                    format!("Not saved (same-named bones): {names}"),
                );
            }
            Some(id)
        }
        Err(e) => {
            app.message = describe_save_error(lang, &e);
            None
        }
    }
}

fn describe_save_error(lang: Lang, e: &StoreError) -> String {
    format!(
        "{}: {}",
        lang.pick("隠し方を保存できません", "Cannot save the hide set"),
        e.describe(lang)
    )
}

/// プリセットを消す（入れていたら外す）。描いている最中は断り、ファイルにも一覧にも触らない。入れていたプリセットは、外せると
/// 決まってからファイルを消す（外せなければ、ファイルも一覧もそのまま）。
pub fn delete_preset(app: &mut AppState, id: u32) {
    let lang = app.lang;
    if app.is_stroking() {
        app.message = lang.view_error(&ViewError::Stroking);
        return;
    }
    let used = app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| s.hide.presets.contains(&id));
    if used && !change(app, |h| h.presets.retain(|p| *p != id)) {
        return;
    }
    if let Err(e) = app.view3d.pose.hide_presets.remove(id) {
        app.message = format!(
            "{}: {}",
            lang.pick("隠し方を消せません", "Cannot delete the hide set"),
            e.describe(lang)
        );
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

    fn full(app: &AppState) -> usize {
        app.view3d.full_model().unwrap().triangle_count()
    }

    fn shown(app: &AppState) -> usize {
        app.view3d.model.as_ref().unwrap().triangle_count()
    }

    #[test]
    fn hiding_a_rigid_bone_removes_exactly_its_faces_from_the_shown_model() {
        let mut app = app();
        let total = full(&app);
        let head = bone(&app, "頭");
        let sphere = {
            let s = app.view3d.pose.session.as_ref().unwrap();
            s.rig.meshes()[5].mesh.triangle_count()
        };
        hide_bone(&mut app, head);
        assert_eq!(shown(&app), total - sphere, "頭の球だけが消える");
        assert_eq!(full(&app), total, "受けたままの形は変わらない");
        assert_eq!(hidden_total(&app), sphere);
        // 当たり: 頭のあった所へ向けた視線は、頭の面に当たらない
        let model = app.view3d.model.as_ref().unwrap();
        assert!(model
            .meshes
            .iter()
            .all(|m| m.name != "頭" || m.submeshes.is_empty()));
        // 見せる形の番号 → 受けたままの形の番号
        let last = (shown(&app) - 1) as u32;
        let mapped = app.view3d.full_triangle(last).unwrap();
        assert!((mapped as usize) < total && !app.view3d.is_face_hidden(mapped));
        let first_hidden = (0..total as u32)
            .find(|i| app.view3d.is_face_hidden(*i))
            .unwrap();
        assert!(first_hidden as usize >= total - sphere);
        assert_eq!(app.view3d.full_triangle(shown(&app) as u32), None);
        // すべて表示する
        show_all(&mut app);
        assert_eq!(shown(&app), total);
        assert!(std::sync::Arc::ptr_eq(
            app.view3d.model.as_ref().unwrap(),
            app.view3d.full_model().unwrap()
        ));
        assert_eq!(hidden_total(&app), 0);
    }

    #[test]
    fn the_threshold_and_children_decide_which_faces_go() {
        let mut app = app();
        let total = full(&app);
        let hand = bone(&app, "右手");
        let lower = bone(&app, "右前腕");
        let count = |app: &AppState| total - shown(app);
        // 手の骨だけ、しきい値 0.5
        set_next(&mut app, 0.5, false);
        hide_bone(&mut app, hand);
        let half = count(&app);
        assert!(half > 0, "手の先の面は隠れる");
        // しきい値を下げて同じ骨へ足し直すと、入れ替わって隠れる面が増える（項目は 1 つのまま）
        set_next(&mut app, 0.05, false);
        hide_bone(&mut app, hand);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.hide.entries.len(), 1);
        assert!(count(&app) > half, "しきい値が低いほど広く隠れる");
        // しきい値を 1 にすると、ウェイトが 100% の面だけ
        set_next(&mut app, 1.0, false);
        hide_bone(&mut app, hand);
        assert!(count(&app) <= half);
        // 子を含める: 前腕とその子（手）の木
        show_all(&mut app);
        set_next(&mut app, 0.5, false);
        hide_bone(&mut app, lower);
        let alone = count(&app);
        set_next(&mut app, 0.5, true);
        hide_bone(&mut app, lower);
        assert!(count(&app) > alone, "子を含めると手の分が増える");
        // 項目を外すと戻る
        remove_entry(&mut app, 0);
        assert_eq!(count(&app), 0);
    }

    #[test]
    fn a_bone_that_hides_nothing_is_reported_and_hiding_everything_is_refused() {
        let mut app = app();
        let total = full(&app);
        // 根の骨を、子を含めてしきい値 0.0（> 0 の面は全部）にすると全部の面が隠れる → 断る
        let root = bone(&app, "腰");
        set_next(&mut app, 0.0, true);
        hide_bone(&mut app, root);
        assert_eq!(shown(&app), total, "全部は隠さない");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.hide.entries.is_empty(), "断った項目は残らない");
        assert!(app.message.contains("すべての面"), "{}", app.message);
        // 影響する頂点が無い骨（手首より先の指などは無い）: 根の「首」は筒の継ぎ目で混ざるが、足の骨を 100% にすれば足の先だけ
        // 何も隠れない項目: どのメッシュにも掛からない骨は、無いものとして知らせる
        let mut app2 = AppState::new(64, 64);
        app2.apply(Action::Pose(PoseAction::LoadFigure));
        let neck = bone(&app2, "首");
        set_next(&mut app2, 1.0, false);
        hide_bone(&mut app2, neck);
        assert_eq!(hidden_total(&app2), 0);
        assert!(
            app2.message.contains("隠れる面がありません"),
            "{}",
            app2.message
        );
        let s = app2.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.hide.entries.len(), 1, "項目は残る（しきい値を直せる）");
        // 英語
        app2.lang = Lang::En;
        let before = app2.message.clone();
        set_next(&mut app2, 1.0, false);
        hide_bone(&mut app2, neck);
        assert_ne!(app2.message, before);
        assert!(app2.message.is_ascii(), "{}", app2.message);
    }

    #[test]
    fn hiding_follows_the_pose_and_keeps_the_snapshot_cheap_to_update() {
        let mut app = app();
        let head = bone(&app, "頭");
        hide_bone(&mut app, head);
        let before = app.view3d.model.clone().unwrap();
        // ポーズを付けると、見せる形は組み直され（新しい世代）、隠した面はそのまま
        let upper = bone(&app, "右上腕");
        let mut p = app.view3d.pose.session.as_ref().unwrap().pose().clone();
        p.locals[upper].rotation = yolu_core::glam::Quat::from_rotation_z(-1.0);
        crate::view3d::pose::set_pose(&mut app.view3d, p).unwrap();
        let after = app.view3d.model.clone().unwrap();
        assert!(after.revision() > before.revision());
        assert_eq!(after.triangle_count(), before.triangle_count());
        assert!(
            after
                .geometry
                .triangles()
                .iter()
                .zip(before.geometry.triangles())
                .any(|(a, b)| a.a != b.a),
            "位置は新しいポーズ"
        );
        // 隠していても、ブラシの大きさの基準は受けたままの形のもの
        assert_eq!(
            after.geometry.brush_scale(),
            app.view3d.full_model().unwrap().geometry.brush_scale()
        );
        // 隣り合わせは使い回す（組み直しの溶接は走らない）
        assert_eq!(after.geometry.neighbors(10), before.geometry.neighbors(10));
    }

    /// 描き始める（ドキュメントのストロークと 3D ビューの入力の両方を立てる）。
    fn begin_stroke(app: &mut AppState) {
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.view3d.input.stroke = Some(crate::state::StrokeSource::Mouse);
    }

    /// 描き終える（取り消して、3D ビューへ終わりを知らせる）。
    fn end_stroke(app: &mut AppState) {
        let stroke = app.stroke.take().unwrap();
        app.doc.cancel_stroke(stroke);
        app.view3d.stroke_ended();
    }

    #[test]
    fn hiding_changes_are_refused_while_stroking_and_work_after_it_ends() {
        let mut app = app();
        let total = full(&app);
        let head = bone(&app, "頭");
        begin_stroke(&mut app);
        hide_bone(&mut app, head);
        assert_eq!(app.message, ViewError::Stroking.to_string());
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.hide.entries.is_empty());
        assert_eq!(shown(&app), total);
        end_stroke(&mut app);
        hide_bone(&mut app, head);
        assert!(shown(&app) < total);
    }

    #[test]
    fn deleting_a_preset_while_stroking_leaves_the_file_the_list_and_the_mask_alone() {
        let dir = std::env::temp_dir().join(format!("yolu-hide-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = app();
        app.view3d.pose.hide_presets.attach(dir.clone());
        let total = full(&app);
        let head = bone(&app, "頭");
        hide_bone(&mut app, head);
        let id = save_preset(&mut app, "頭").unwrap();
        show_all(&mut app);
        toggle_preset(&mut app, id);
        let hidden = total - shown(&app);
        assert!(hidden > 0);
        let file = dir.join(format!("hide-{id}.ylhide"));
        assert!(file.exists());
        // 描いている最中: 断って、ファイルも一覧も入れている印も面の印もそのまま
        app.message.clear();
        begin_stroke(&mut app);
        delete_preset(&mut app, id);
        assert_eq!(app.message, ViewError::Stroking.to_string());
        assert!(file.exists(), "ファイルは消えない");
        assert!(app.view3d.pose.hide_presets.get(id).is_some());
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.hide.presets, [id], "入れている印も残る");
        assert_eq!(hidden_total(&app), hidden, "面の印も変わらない");
        end_stroke(&mut app);
        // 終わったら消せる（ファイルも、入れている印も、面の印も）
        delete_preset(&mut app, id);
        assert!(!file.exists());
        assert!(app.view3d.pose.hide_presets.get(id).is_none());
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.hide.presets.is_empty());
        assert_eq!(shown(&app), total);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_mask_changed_while_stroking_is_applied_when_the_stroke_ends() {
        let mut app = app();
        let total = full(&app);
        let (head, hand) = (bone(&app, "頭"), bone(&app, "右手"));
        hide_bone(&mut app, head);
        let head_mask = app.view3d.face_mask().cloned().unwrap();
        let after_head = shown(&app);
        assert!(after_head < total);
        // 描いている最中に印を外す（モデルの入れ替え・セッション終了が描いている最中に来た場合）: 見せる形は変わらない
        begin_stroke(&mut app);
        app.view3d.set_face_mask(None);
        assert!(app.view3d.face_mask().is_none());
        assert_eq!(
            shown(&app),
            after_head,
            "描き終わるまで見せる形は組み直さない"
        );
        assert!(
            app.view3d.is_face_hidden(
                (0..total as u32)
                    .find(|i| head_mask.is_hidden(*i as usize))
                    .unwrap()
            ),
            "当たりの側も、見せる形のまま"
        );
        end_stroke(&mut app);
        assert_eq!(shown(&app), total, "終わったら印なしで組み直す");
        assert!(!app.view3d.is_face_hidden(0));
        // 別の印へ替えるのも同じ（描いている最中は前の印のまま、終わると当たる）
        hide_bone(&mut app, head);
        assert_eq!(shown(&app), after_head);
        let hand_mask = {
            let s = app.view3d.pose.session.as_ref().unwrap();
            mask_for(
                &s.rig,
                &[HideEntry {
                    bone: hand,
                    threshold: 0.5,
                    children: true,
                }],
            )
            .map(Arc::new)
            .unwrap()
        };
        let hand_count = hand_mask.hidden_count();
        assert!(hand_count > 0);
        begin_stroke(&mut app);
        app.view3d.set_face_mask(Some(hand_mask));
        assert_eq!(shown(&app), after_head);
        end_stroke(&mut app);
        assert_eq!(shown(&app), total - hand_count);
    }

    #[test]
    fn pressing_the_same_hide_again_does_not_claim_that_nothing_hides() {
        let mut app = app();
        let hand = bone(&app, "右手");
        app.message.clear();
        set_next(&mut app, 0.5, true);
        hide_bone(&mut app, hand);
        let hidden = hidden_total(&app);
        assert!(hidden > 0);
        assert!(app.message.is_empty(), "{}", app.message);
        // 同じ項目をもう一度: 面は隠れたまま、知らせは出ない
        hide_bone(&mut app, hand);
        assert_eq!(hidden_total(&app), hidden);
        assert!(app.message.is_empty(), "{}", app.message);
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.hide.entries.len(), 1);
        // すでに隠れている面の内側へ別の骨を足して、隠す面の数が増えなくても、その骨の面はあるので知らせない
        show_all(&mut app);
        let lower = bone(&app, "右前腕");
        hide_bone(&mut app, lower);
        let with_hand = hidden_total(&app);
        assert!(with_hand >= hidden);
        hide_bone(&mut app, hand);
        assert_eq!(hidden_total(&app), with_hand, "手の分はもう隠れている");
        assert!(app.message.is_empty(), "{}", app.message);
        // 何も隠れない項目は知らせる（項目単体で面を持たない）
        let neck = bone(&app, "首");
        set_next(&mut app, 1.0, false);
        hide_bone(&mut app, neck);
        assert!(
            app.message.contains("隠れる面がありません"),
            "{}",
            app.message
        );
    }

    fn entry(path: &[&str], threshold: f32, children: bool) -> PresetEntry {
        PresetEntry {
            path: path.iter().map(|s| s.to_string()).collect(),
            threshold,
            children,
        }
    }

    #[test]
    fn presets_save_load_and_combine_as_a_union() {
        let mut app = app();
        let total = full(&app);
        let (head, hand) = (bone(&app, "頭"), bone(&app, "右手"));
        // 頭を隠して保存
        hide_bone(&mut app, head);
        let id_head = save_preset(&mut app, "頭").unwrap();
        show_all(&mut app);
        assert_eq!(shown(&app), total);
        // 手を隠して保存
        set_next(&mut app, 0.5, true);
        hide_bone(&mut app, hand);
        let hand_only = total - shown(&app);
        let id_hand = save_preset(&mut app, "右手").unwrap();
        show_all(&mut app);
        // プリセットを入れる: 頭だけ
        toggle_preset(&mut app, id_head);
        let head_only = total - shown(&app);
        assert!(head_only > 0);
        // 2 つ入れると和
        toggle_preset(&mut app, id_hand);
        assert_eq!(
            total - shown(&app),
            head_only + hand_only,
            "重ならない 2 つの和"
        );
        // 手で足した項目とも組み合わさる
        let foot = bone(&app, "左足");
        hide_bone(&mut app, foot);
        assert!(total - shown(&app) > head_only + hand_only);
        // 保存すると、入れているプリセットと手の項目の和が 1 つのプリセットになる
        let id_all = save_preset(&mut app, "全部").unwrap();
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(
            app.view3d
                .pose
                .hide_presets
                .get(id_all)
                .unwrap()
                .entries
                .len(),
            3
        );
        let _ = s;
        // 外すと元へ
        show_all(&mut app);
        toggle_preset(&mut app, id_all);
        let combined = total - shown(&app);
        toggle_preset(&mut app, id_all);
        assert_eq!(shown(&app), total);
        assert!(combined > head_only + hand_only);
        // 消したプリセットは外れる
        toggle_preset(&mut app, id_head);
        delete_preset(&mut app, id_head);
        assert!(app.view3d.pose.hide_presets.get(id_head).is_none());
        assert_eq!(
            shown(&app),
            total,
            "入れていたプリセットを消すと隠すのをやめる"
        );
    }

    #[test]
    fn a_preset_carries_to_another_model_by_bone_name_path_and_skips_the_ones_that_do_not_fit() {
        let mut app = app();
        let total = full(&app);
        // 名前の道が合うもの・合わないもの（上の骨が違う・無い名前）が混ざったプリセット
        let id = app
            .view3d
            .pose
            .hide_presets
            .add(
                "混ぜ",
                vec![
                    entry(&["腰", "背骨", "胸", "首", "頭"], 0.5, true),
                    entry(&["腰", "背骨", "右手"], 0.5, true),
                    entry(&["別の根", "頭"], 0.5, true),
                ],
            )
            .unwrap();
        toggle_preset(&mut app, id);
        assert!(shown(&app) < total, "合う項目は効く");
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert_eq!(s.hide.skipped.len(), 2, "合わない 2 項目は理由つきで飛ばす");
        assert!(s
            .hide
            .skipped
            .iter()
            .all(|k| k.reason == SkipReason::NotFound && k.preset == "混ぜ"));
        assert_eq!(s.hide.skipped[0].path, "腰/背骨/右手");
        assert!(s.hide.skipped[0]
            .describe(Lang::En)
            .contains("Bone not found"));
        assert!(s.hide.skipped[0]
            .describe(Lang::Ja)
            .contains("ボーンがありません"));
        // 別のモデル（もう一度読み直した同じ人形）でも使える。隠し方は新しいセッションに引き継がれない
        app.apply(Action::Pose(PoseAction::LoadFigure));
        let s = app.view3d.pose.session.as_ref().unwrap();
        assert!(s.hide.is_clear());
        assert_eq!(shown(&app), total, "前のモデルの隠す面は残らない");
        assert!(app.view3d.face_mask().is_none());
        toggle_preset(&mut app, id);
        assert!(shown(&app) < total);
    }

    #[test]
    fn the_mask_does_not_outlive_its_model() {
        let mut app = app();
        let head = bone(&app, "頭");
        hide_bone(&mut app, head);
        assert!(app.view3d.face_mask().is_some());
        // 別のモデル（試しの立方体）に替わると、セッションが終わって印も外れる
        app.apply(Action::LoadDemoModel);
        crate::view3d::pose::poll(&mut app.view3d);
        assert!(app.view3d.pose.session.is_none());
        assert!(app.view3d.face_mask().is_none());
        assert_eq!(
            shown(&app),
            app.view3d.full_model().unwrap().triangle_count()
        );
        // 三角形の数が違う印は、モデルに当てない
        app.view3d
            .set_face_mask(Some(Arc::new(FaceMask::new(vec![true; 3]))));
        assert_eq!(
            shown(&app),
            app.view3d.full_model().unwrap().triangle_count()
        );
    }

    #[test]
    fn hiding_works_together_with_a_hidden_material() {
        let mut app = app();
        let total = full(&app);
        // マテリアル 1（顔・頭の球）を隠すと同じ面が消えるが、番号の対応は崩れない
        app.view3d.set_hidden(vec![1]);
        let head = bone(&app, "頭");
        let sphere = {
            let s = app.view3d.pose.session.as_ref().unwrap();
            s.rig.meshes()[5].mesh.triangle_count()
        };
        assert_eq!(shown(&app), total - sphere);
        // 右手の面も隠すと、さらに減る。両方の隠しを通した番号の対応が、受けたままの形の番号を指す
        set_next(&mut app, 0.5, true);
        let hand = bone(&app, "右手");
        hide_bone(&mut app, hand);
        let both = shown(&app);
        assert!(both < total - sphere);
        for shown_index in (0..both as u32).step_by(37) {
            let f = app.view3d.full_triangle(shown_index).unwrap();
            assert!(!app.view3d.is_face_hidden(f));
            let tri = &app.view3d.full_model().unwrap().geometry.triangles()[f as usize];
            assert_ne!(tri.material, 1, "隠したマテリアルの面ではない");
            let shown_tri =
                &app.view3d.model.as_ref().unwrap().geometry.triangles()[shown_index as usize];
            assert_eq!(shown_tri.uv_a, tri.uv_a);
        }
        // マテリアルを戻しても、面の隠しは残る
        app.view3d.set_hidden(Vec::new());
        assert!(shown(&app) < total);
        assert!(shown(&app) > both);
        let _ = head;
    }

    /// 計測（cargo test -p yolu-app --lib measure_hiding -- --ignored --nocapture。dev プロファイル（opt-level 1・依存は 3）の値で、
    /// release の値ではない。ほかの cargo が同時に走ると揺れる）。7 万三角形の試しの人形で、隠す項目を足す
    /// 時間（面の重み・メッシュの組み直し・幾何の組み立て）と、隠したままポーズを 1 回変える時間（箱の当て直し）を、隠さないときと比べる。
    #[test]
    #[ignore]
    fn measure_hiding_on_the_avatar_figure() {
        use std::time::Instant;
        use yolu_core::skin::{demo_figure, FigureDetail};
        let mut app = AppState::new(64, 64);
        crate::view3d::pose::load_rig(&mut app.view3d, demo_figure(FigureDetail::AVATAR)).unwrap();
        let upper = bone(&app, "右上腕");
        let hand = bone(&app, "右手");
        let pose_once = |app: &mut AppState, k: usize| {
            let mut p = app.view3d.pose.session.as_ref().unwrap().pose().clone();
            p.locals[upper].rotation =
                yolu_core::glam::Quat::from_rotation_z(-0.2 - 0.05 * k as f32);
            let clock = Instant::now();
            crate::view3d::pose::set_pose(&mut app.view3d, p).unwrap();
            clock.elapsed().as_secs_f64() * 1000.0
        };
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        };
        let plain: Vec<f64> = (0..7).map(|k| pose_once(&mut app, k)).collect();
        set_next(&mut app, 0.5, true);
        let clock = Instant::now();
        hide_bone(&mut app, hand);
        let add = clock.elapsed().as_secs_f64() * 1000.0;
        let hidden = hidden_total(&app);
        let posed: Vec<f64> = (7..14).map(|k| pose_once(&mut app, k)).collect();
        println!(
            "人形 {} 三角形（隠す {}）: 隠す項目を足す {:.1} ms、ポーズを 1 回変える: 隠さない {:.1} ms・隠したまま {:.1} ms（中央値）",
            full(&app),
            hidden,
            add,
            median(plain),
            median(posed)
        );
    }
}
