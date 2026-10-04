//! 書き置きの材料。主のスレッドでは、描いている最中ではない区切りで、変わったセットの文書の写し
//! （`Document::capture_snapshot`。タイルは共有。画素はコピーしない）と、名前・マテリアルなどの小さな値だけを取る。
//! 正本への詰め直し・合成・書き込みは別のスレッド（`writer`）で、`build` がする。
//!
//! 書き置きの中身は、保存（`project::save`）が書く `.ylp` と同じ形（同じ `Project`）で、合成の PNG とメッシュマップだけが
//! 違う。合成の PNG は派生物で時間がかかり、メッシュマップは焼き直せる派生物なので、書き置きには入れない（開いた時の
//! ファイルにあったものは、バイト列のまま残る）。変わっていないセット・読むだけのセットの正本も、開いた時のバイト列のまま。

use std::sync::Arc;

use yolu_core::SelectionMask;
use yolu_io::{NativeDocument, Project, SetSpec};

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
}

/// 1 セット分の材料。
pub(crate) struct SetCapture {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 文書の写し。読むだけのセットと、開いた・保存した時から変わっていないセットは取らない（開いた時のバイト列のまま）。
    pub snapshot: Option<Document>,
}

/// 書き置き 1 回分の材料（別のスレッドへ渡す）。
pub(crate) struct Capture {
    pub sets: Vec<SetCapture>,
    pub current: String,
    /// 開いた・保存した時の中身（`ProjectFile` と共有する。複製しない）。無ければ新しいプロジェクト。
    pub base: Option<Arc<Project>>,
    /// アセットの棚（変えていて読めるときだけ。変えていなければ開いたファイルのバイト列のまま）。
    pub shelf: Option<yolu_io::shelf::Shelf>,
    pub lang: Lang,
    /// 一覧に出す名前（ファイルのあるプロジェクトの名前。無ければ空）と元の .ylp のパス（無ければ空）。
    pub title: String,
    pub project_path: String,
    pub fingerprint: Fingerprint,
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
                (s.id.clone(), doc.id(), doc.revision(), s.name.clone(), s.material.clone())
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
    }
}

/// 取れない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 描いている最中。
    Stroke,
    /// 取り込み（PSD）の途中。
    Import,
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
        let unchanged = in_base && set.saved == Some((doc.id(), doc.revision()));
        let read_only = set.read_only.is_some();
        let snapshot = if read_only || unchanged {
            None
        } else {
            Some(doc.capture_snapshot().map_err(|_| Refusal::Stroke)?)
        };
        sets.push(SetCapture {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            snapshot,
        });
    }
    let project_path = match state.project.as_ref().filter(|p| p.is_file()) {
        Some(p) => p.path().display().to_string(),
        None => recovered_from.unwrap_or_default().to_owned(),
    };
    // 一覧に出す名前は、ファイルのあるプロジェクトのものだけ（名前の無いプロジェクトは空にして、一覧が今の言語で
    // 「名称未設定」と出す。書いた時の言語の既定の名前を残さない）
    let title = if project_path.is_empty() { String::new() } else { state.project_name.clone() };
    let shelf = (state.shelf.changed && state.shelf.unavailable.is_none())
        .then(|| state.shelf.shelf().clone());
    Ok(Capture {
        sets,
        current: state.sets.current().id.clone(),
        base,
        shelf,
        lang: state.lang,
        title,
        project_path,
        fingerprint: fingerprint(state),
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
        let document = match &set.snapshot {
            Some(doc) => Some(NativeDocument::from_core(doc)?),
            None => None,
        };
        if document.is_none() && !in_base {
            return Err(RecoveryError::Project(yolu_io::Error::InvalidData(format!(
                "セット「{}」の元の正本がありません",
                set.name
            ))));
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
    let project = match &capture.base {
        Some(base) if base.info().format < 7 => base
            .upgraded(writer.clone())?
            .with_sets(writer, &specs, &capture.current)?,
        Some(base) => base.with_sets(writer, &specs, &capture.current)?,
        None => Project::create(writer, &specs, &capture.current)?,
    };
    // 選択範囲（selection.bin）は正本と別のエントリ。取った写しのものを、違うセットだけ書き換える
    let selections: Vec<(&str, Option<&SelectionMask>)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.selection())))
        .collect();
    let project = crate::selection::io::write_into(project, &selections, capture.lang)
        .map_err(RecoveryError::Text)?;
    // 見た目の設定（look.json）も、取った写しのものを、違うセットだけ書き換える
    let looks: Vec<(&str, &yolu_core::look::MaterialLook)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.look())))
        .collect();
    // 読めなかったエントリを上書きしたかは、復旧の写しでは知らせない（開いた .ylp には手を付けない。保存のときに知らせる）
    let (project, _) = crate::look::io::write_into(project, &looks, capture.lang)
        .map_err(RecoveryError::Text)?;
    // アセットの棚（保存と同じく、変えたときだけ resources を書き直す）
    match &capture.shelf {
        Some(shelf) => Ok(project.with_shelf(shelf, crate::project::writer())?),
        None => Ok(project),
    }
}
