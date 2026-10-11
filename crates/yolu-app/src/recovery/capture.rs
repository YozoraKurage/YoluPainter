//! 書き置きの材料。主のスレッドでは、描いている最中ではない区切りで、変わったセットの文書の写し
//! （`Document::capture_snapshot`。タイルは共有。画素はコピーしない）と、名前・マテリアルなどの小さな値だけを取る。
//! 正本への詰め直し・合成・書き込みは別のスレッド（`writer`）で、`build` がする。
//!
//! 書き置きの中身は、保存（`project::save`）が書く `.ylp` と同じ形（同じ `Project`）で、合成の PNG とメッシュマップだけが
//! 違う。合成の PNG は派生物で時間がかかり、メッシュマップは焼き直せる派生物なので、書き置きには入れない（開いた時の
//! ファイルにあったものは、バイト列のまま残る）。変わっていないセット・読むだけのセットの正本も、開いた時のバイト列のまま。
//! 読むだけで、保存したプロジェクトにも無いセット（保存したことの無いセットが読めなくなった）は、そのセットだけ書き置きに入れない。

use std::sync::Arc;

use yolu_core::SelectionMask;
use yolu_io::{DocumentSource, Project, SetSpec};

use crate::engine::Document;
use crate::lang::Lang;
use crate::sets::MaterialRef;
use crate::state::AppState;

use super::RecoveryError;

/// 変わったか見分ける札。2 つが等しければ、書き置きの中身は同じ。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    sets: Vec<(String, u128, u64, String, MaterialRef)>,
    current: String,
    title: String,
    /// アセットの棚の素材（ID と名前）。
    shelf: Vec<(String, String)>,
    /// プロジェクトのモデルのポーズを変えた回数（ポーズだけの変更も書き置きになる。モデルが無ければ None）。
    pose: Option<u64>,
}

/// 1 セット分の材料。
pub(crate) struct SetCapture {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 文書の写し（タイルは共有）。読むだけのセットと、開いた・保存した時から変わっていないセットは取らない（開いた時のバイト列のまま）。
    pub snapshot: Option<Arc<Document>>,
}

/// 書き置き 1 回分の材料（別のスレッドへ渡す）。
pub(crate) struct Capture {
    pub sets: Vec<SetCapture>,
    pub current: String,
    /// 開いた・保存した時の中身（`ProjectFile` と共有する。複製しない）。無ければ新しいプロジェクト。
    pub base: Option<Arc<Project>>,
    /// 開いたあとに文書を別の物に替えたセット（PSD を「今のセットへ」取り込み直した）。書き置きにも古い PSD の原本を持ち越さない
    /// （復旧で開くと、書き置きが次の保存の元になるので、ここで除かないと保存で古い原本が戻る）。
    pub replaced: Vec<String>,
    /// アセットの棚（変えていて読めるときだけ。変えていなければ開いたファイルのバイト列のまま）。
    pub shelf: Option<yolu_io::shelf::Shelf>,
    /// プロジェクトのモデルのポーズ（ファイルの `pose.json` へ書く材料。保存と同じ形）。
    pub pose: crate::view3d::pose::stored::PoseCapture,
    pub lang: Lang,
    /// 一覧に出す名前（ファイルのあるプロジェクトの名前。無ければ空）と元の .ylp のパス（無ければ空）。
    pub title: String,
    pub project_path: String,
    pub fingerprint: Fingerprint,
    /// 書く形の閾値（取ったスレッドのもの。書き込みのスレッドも同じ形で書く。本物は既定の値で、試験だけが小さくする）。
    pub thresholds: yolu_io::Thresholds,
}

/// いまの状態の札（文書の版が変わらない変更 = セットの名前・マテリアル・並び・今のセットを含む）。
pub(crate) fn fingerprint(state: &AppState) -> Fingerprint {
    Fingerprint {
        sets: state
            .sets
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let doc = state.set_doc(i);
                (
                    s.id.clone(),
                    doc.id(),
                    doc.revision(),
                    s.name.clone(),
                    s.material.clone(),
                )
            })
            .collect(),
        current: state.sets.current().id.clone(),
        title: state.project_name.clone(),
        shelf: state
            .shelf
            .resources()
            .iter()
            .map(|r| (r.id.clone(), r.name.clone()))
            .collect(),
        pose: state
            .np
            .model_file
            .as_ref()
            .and_then(|_| state.view3d.pose.session.as_ref().map(|s| s.edits)),
    }
}

/// 取れない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 描いている最中。
    Stroke,
    /// 取り込み（PSD）の途中。
    Import,
    /// 書けるセットが 1 つも無い（どのセットも、保存したことが無く読めない）。
    NothingToWrite,
}

/// 書き置きの材料を取る。ストロークの最中・取り込みの途中は取らない。
pub(crate) fn capture(state: &AppState, recovered_from: Option<&str>) -> Result<Capture, Refusal> {
    if state.is_stroking() {
        return Err(Refusal::Stroke);
    }
    if state.psd.is_busy() {
        return Err(Refusal::Import);
    }
    let base = state.project.as_ref().map(|p| p.project_shared());
    let mut sets = Vec::with_capacity(state.sets.len());
    for (i, set) in state.sets.iter().enumerate() {
        let doc = state.set_doc(i);
        let in_base = base
            .as_ref()
            .is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        let read_only = set.read_only.is_some();
        if read_only && !in_base {
            // 読むだけで、保存したプロジェクトにも無いセットは、書く元の中身が無い。このセットだけ書き置きに入れず、ほかのセットは書く
            continue;
        }
        let unchanged = in_base && set.saved == Some((doc.id(), doc.revision()));
        let snapshot = if read_only || unchanged {
            None
        } else {
            Some(Arc::new(
                doc.capture_snapshot().map_err(|_| Refusal::Stroke)?,
            ))
        };
        sets.push(SetCapture {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            snapshot,
        });
    }
    // 今のセットを除いたときは、並びで最初の残るセットを今のセットにする（保存と同じ）。残るセットが無ければ書き置きを作らない
    let current = state.sets.current().id.clone();
    let current = sets
        .iter()
        .find(|s| s.id == current)
        .or(sets.first())
        .map(|s| s.id.clone())
        .ok_or(Refusal::NothingToWrite)?;
    let replaced = crate::project::replaced_sets(
        base.as_deref(),
        // 読むだけのセットの文書は見せるだけの写しなので数えない（保存と同じ）
        state
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| set.read_only.is_none())
            .map(|(i, set)| (set.id.as_str(), state.set_doc(i).id())),
    );
    let project_path = match state.project.as_ref().filter(|p| p.is_file()) {
        Some(p) => p.path().display().to_string(),
        None => recovered_from.unwrap_or_default().to_owned(),
    };
    // 一覧に出す名前は、ファイルのあるプロジェクトのものだけ（名前の無いプロジェクトは空にして、一覧が今の言語で
    // 「名称未設定」と出す。書いた時の言語の既定の名前を残さない）
    let title = if project_path.is_empty() {
        String::new()
    } else {
        state.project_name.clone()
    };
    let shelf = (state.shelf.changed && state.shelf.unavailable.is_none())
        .then(|| state.shelf.shelf().clone());
    Ok(Capture {
        sets,
        current,
        base,
        replaced,
        shelf,
        pose: crate::view3d::pose::stored::capture(state).0,
        lang: state.lang,
        title,
        project_path,
        fingerprint: fingerprint(state),
        thresholds: yolu_io::Thresholds::current(),
    })
}

/// 材料から、保存が書くのと同じ形の `Project` を作る（別のスレッドで動かす）。
pub(crate) fn build(capture: &Capture) -> Result<Project, RecoveryError> {
    let mut specs = Vec::with_capacity(capture.sets.len());
    for set in &capture.sets {
        let in_base = capture
            .base
            .as_ref()
            .is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        // 正本は全体をメモリに組まない: 写しを渡し、置き場へ書くときにレイヤーごとに流して作る（変わらないレイヤーの部分は前の世代と共有）
        let document = match &set.snapshot {
            Some(doc) => Some(DocumentSource::from_core(doc.clone())?),
            None => None,
        };
        if document.is_none() && !in_base {
            return Err(RecoveryError::Project(yolu_io::Error::InvalidData(
                format!("セット「{}」の元の中身がありません", set.name),
            )));
        }
        specs.push(SetSpec {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            document,
            composites: Vec::new(),
        });
    }
    let writer = crate::project::writer();
    // 大きさを変えた文書の、古い大きさの今の選択範囲は外す（新しい文書と合わず、書き直しの検証が断る。今の選択範囲はあとで書く）
    let sizes: Vec<(&str, (u32, u32, u32))> = capture
        .sets
        .iter()
        .filter_map(|s| {
            s.snapshot
                .as_ref()
                .map(|d| (s.id.as_str(), (d.width(), d.height(), d.tile_size())))
        })
        .collect();
    let resized = capture
        .base
        .as_ref()
        .map(|base| crate::selection::io::resized_sets(base, &sizes))
        .unwrap_or_default();
    let project = match &capture.base {
        Some(base) => {
            let fitted = crate::selection::io::without_stale(base, &sizes, capture.lang)
                .map_err(RecoveryError::Text)?;
            if fitted.info().format < 7 {
                fitted
                    .upgraded(writer.clone())?
                    .with_sets(writer, &specs, &capture.current)?
            } else {
                fitted.with_sets(writer, &specs, &capture.current)?
            }
        }
        None => Project::create(writer, &specs, &capture.current)?,
    };
    // 文書を替えたセットの古い PSD の原本は、保存（`project::save`）と同じく持ち越さない
    let mut project = project;
    for id in &capture.replaced {
        project = project.without_imported_original(id)?;
    }
    // 選択範囲（selection.bin）は正本と別のエントリ。取った写しのものを、違うセットだけ書き換える
    let selections: Vec<(&str, Option<&SelectionMask>)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.selection())))
        .collect();
    let project = crate::selection::io::write_into(project, &selections, capture.lang)
        .map_err(RecoveryError::Text)?;
    // 名前を付けて残した選択範囲も、取った写しのものを、違うセットだけ書き換える（読めなかった項目を置き換えたかは、保存のときに知らせる）
    let saved: Vec<(&str, &[yolu_core::SavedSelection])> = capture
        .sets
        .iter()
        .filter_map(|s| {
            s.snapshot
                .as_ref()
                .map(|d| (s.id.as_str(), d.saved_selections()))
        })
        .collect();
    let (project, _) =
        crate::selection::io::write_saved_into(project, &saved, &resized, capture.lang)
            .map_err(RecoveryError::Text)?;
    // 見た目の設定（look.json）も、取った写しのものを、違うセットだけ書き換える
    let looks: Vec<(&str, &yolu_core::look::MaterialLook)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.look())))
        .collect();
    // 読めなかったエントリを上書きしたかは、復旧の写しでは知らせない（開いた .ylp には手を付けない。保存のときに知らせる）
    let (project, _) =
        crate::look::io::write_into(project, &looks, capture.lang).map_err(RecoveryError::Text)?;
    // Unity から受けた値（復旧は、開いていた時の見た目に戻すために、保存の設定によらず書く）
    let received: Vec<(&str, Option<&yolu_core::look::ReceivedLook>)> = capture
        .sets
        .iter()
        .filter_map(|s| {
            s.snapshot
                .as_ref()
                .map(|d| (s.id.as_str(), d.received_look()))
        })
        .collect();
    let project = crate::look::io::write_received_into(project, &received, capture.lang)
        .map_err(RecoveryError::Text)?;
    // モデルのポーズ（pose.json。保存と同じく、違うときだけ書く。読めなかったエントリを上書きしたかは、保存のときに知らせる）
    let (project, _) =
        crate::view3d::pose::stored::write_into(project, &capture.pose, capture.lang)
            .map_err(RecoveryError::Text)?;
    // アセットの棚（保存と同じく、変えたときだけ resources を書き直す）
    match &capture.shelf {
        Some(shelf) => Ok(project.with_shelf(shelf, crate::project::writer())?),
        None => Ok(project),
    }
}
