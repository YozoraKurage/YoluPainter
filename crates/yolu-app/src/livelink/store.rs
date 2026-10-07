//! .ylp の根の `livelink.json`（Live Link の相手の文書）: 当てた頼みに今のポーズを入れ、マテリアルの値を除いた形で残し、開き直すと
//! Unity なしで同じモデルとポーズに戻す（`LiveLink::reopen` が、受けた頼みと同じ道で FBX を読み、セットに結ぶ。返事は書かない）。
//!
//! - 値（lilToon のマテリアルの値）は各セットの `look.json` の `received` にあり（設定で保存しないこともできる）、ここには書かない。
//!   lilToon の印の値（`_lilToonVersion`）だけは残す（開き直したとき、lilToon のスロットの絵をファイルから読むかを決める）。
//! - FBX と絵の道は Unity の頼みのまま（絶対の道）。ネットワークの道（`\\host\share`）は、開いたときに自動では読まない（細工した .ylp が、
//!   開くだけで外のホストへつながせないように。`newproject::reopen` と同じ決まり）。

use yolu_protocol::files::{read_request, Request, Values};

use crate::state::AppState;

/// 保存の材料。
#[derive(Clone, Debug, PartialEq)]
pub enum Stored {
    /// ファイルのエントリに触れない（モデルがまだ無い: 開き直している最中・開き直せなかった）。
    Keep,
    /// これにする（相手の文書）。
    Write(Vec<u8>),
    /// 外す（ほかのモデル（FBX・形ごと渡されたメッシュ）に替えた）。
    Remove,
}

/// 今の状態から保存の材料を取る。保存に残せなかった骨の名前（道で指せない骨）も返す（保存の知らせに出す）。
pub fn capture(state: &AppState) -> (Stored, Vec<String>) {
    if let Some(target) = state.link_target.as_ref().filter(|_| super::linked(state)) {
        if let Some(session) = state.view3d.pose.session.as_ref() {
            let mut request: Request = (*target.request).clone();
            let unsaved = target
                .layout
                .write_pose(&session.rig, session.pose(), &mut request);
            for m in &mut request.materials {
                m.values = without_values(&m.values);
            }
            if let Ok(bytes) = serde_json::to_vec_pretty(&request) {
                return (Stored::Write(bytes), unsaved);
            }
        }
        return (Stored::Keep, Vec::new());
    }
    let stored = match &state.model {
        Some(m) if !m.is_live_link() => Stored::Remove,
        _ => Stored::Keep,
    };
    (stored, Vec::new())
}

/// 保存に残す値（lilToon の印の値だけ）。
pub fn without_values(values: &Values) -> Values {
    let mark = crate::look::link::LILTOON_VERSION_PROPERTY;
    let mut out = Values::default();
    if let Some(&x) = values.floats.get(mark) {
        out.floats.insert(mark.to_owned(), x);
    }
    if let Some(&x) = values.ints.get(mark) {
        out.ints.insert(mark.to_owned(), x);
    }
    out
}

/// 材料を、プロジェクトの `livelink.json` へ書く（違うときだけ）。
pub fn write_into(project: yolu_io::Project, stored: &Stored) -> yolu_io::Result<yolu_io::Project> {
    let now = project.livelink().ok().flatten();
    match stored {
        Stored::Keep => Ok(project),
        Stored::Write(bytes) if now.as_deref() == Some(bytes.as_slice()) => Ok(project),
        Stored::Write(bytes) => project.with_livelink(Some(bytes)),
        Stored::Remove if project_has(&project) => project.with_livelink(None),
        Stored::Remove => Ok(project),
    }
}

fn project_has(project: &yolu_io::Project) -> bool {
    !matches!(project.livelink(), Ok(None))
}

/// 開いた .ylp の `livelink.json` を読む（頼みと同じ確かめ）。ネットワークの道を含む物は、自動では開き直さない（理由を返す）。
pub fn restore(bytes: &[u8]) -> Result<Request, String> {
    let request = read_request(bytes).map_err(|e| e.to_string())?;
    let network = request
        .models
        .iter()
        .map(|m| m.fbx.as_str())
        .chain(
            request
                .materials
                .iter()
                .flat_map(|m| m.textures.iter().filter_map(|t| t.path.as_deref())),
        )
        .any(crate::newproject::reopen::is_network_path);
    if network {
        return Err("network path".into());
    }
    Ok(request)
}
