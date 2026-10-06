//! 見た目の設定と .ylp（`sets/<ID>/look.json`）の受け渡し。読み込みは文書を作った直後に `restore_look`（Undo の段も版も増やさない）、
//! 保存は正本を書いたあとに `look.json` だけを置き換える（選択範囲と同じ流儀。`selection::io`）。正本を書き直さないセットでも、見た目の
//! 設定だけが変わっていれば書き換える。
//!
//! Live Link で Unity から受けた値（受けた見た目）は同じ `look.json` の `received` に、設定（`livelink_keep_values`、既定は保存する）が
//! 入のときだけ書く（切っていれば外す）。受けた絵の画素は書かない。開くと受けた見た目として戻り（Undo の段も版も増やさない）、
//! つないで新しい値が来れば置き換わる。

use yolu_core::look::{MaterialLook, MissingImage, ReceivedLook};
use yolu_io::Project;

use crate::engine::Document;
use crate::lang::Lang;

/// 開いた .ylp のセットの見た目の設定を文書へ戻す。読めなければ（壊れた・新しい形式）標準のまま開いて理由を返す（エントリは
/// ファイルにバイト列のまま残り、見た目を変えない限り保存でも残る）。
pub fn restore_into(
    doc: &mut Document,
    project: &Project,
    set_id: &str,
    lang: Lang,
) -> Result<(), String> {
    let mine = restore_from(doc, project.look(set_id), lang);
    let received = restore_received_from(doc, project.received_look(set_id), lang);
    match (mine, received) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
        (Err(a), Err(b)) => Err(format!("{a}。{b}")),
    }
}

/// 開いた .ylp のセットの、Unity から受けた値（`received`）を文書へ戻す。読めなければ戻さずに理由を返す（エントリは残る）。
pub fn restore_received_from(
    doc: &mut Document,
    stored: yolu_io::Result<Option<ReceivedLook>>,
    lang: Lang,
) -> Result<(), String> {
    let received = match stored {
        Ok(r) => r,
        Err(e) => {
            return Err(format!(
                "{}: {}",
                lang.pick(
                    "Unity から受けた値を読めません（ファイルには残っています）",
                    "Cannot read the values received from Unity (kept in the file)"
                ),
                lang.io_error(&e)
            ))
        }
    };
    if received.is_none() {
        return Ok(());
    }
    doc.set_received_look(received).map(|_| ()).map_err(|e| {
        format!(
            "{}: {}",
            lang.pick(
                "Unity から受けた値を戻せません",
                "Cannot restore the values received from Unity"
            ),
            lang.core_error(&e)
        )
    })
}

/// 保存する形の受けた見た目（絵の画素は書かず、絵のあったスロットは「届いていない」）。読み直したものと比べる。
fn as_stored(received: &ReceivedLook) -> ReceivedLook {
    let mut out = received.clone();
    for slot in std::mem::take(&mut out.images).into_keys() {
        out.missing.insert(slot, MissingImage::Pending);
    }
    out
}

/// セットの受けた見た目（`received`）を、プロジェクトのものと違うセットだけ置き換える（`received` はセットの ID と、書く受けた見た目。
/// None は外す）。読めないエントリは触らない（受けた値は書かずに、ファイルのバイト列のまま残す）。
pub fn write_received_into(
    mut project: Project,
    received: &[(&str, Option<&ReceivedLook>)],
    lang: Lang,
) -> Result<Project, String> {
    for (id, r) in received {
        let next = r.map(as_stored);
        match project.received_look(id) {
            Ok(stored) if stored == next => continue,
            // 読めないエントリ（新しい形式・壊れた）は残す（受けた値は書かない。開いたときに理由を言ってある）
            Err(_) => continue,
            Ok(_) => {}
        }
        project = project.with_received_look(id, next.as_ref()).map_err(|e| {
            format!(
                "{}: {}",
                lang.pick(
                    "Unity から受けた値を書けません",
                    "Cannot write the values received from Unity"
                ),
                lang.io_error(&e)
            )
        })?;
    }
    Ok(project)
}

/// `restore_into` の、読んだ結果（`Project::look`）から戻す形（プロジェクトを借りたまま文書を作れない呼び手のため）。
pub fn restore_from(
    doc: &mut Document,
    stored: yolu_io::Result<Option<MaterialLook>>,
    lang: Lang,
) -> Result<(), String> {
    let look = match stored {
        Ok(Some(look)) => look,
        Ok(None) => return Ok(()),
        Err(e) => {
            return Err(format!(
                "{}: {}",
                lang.pick(
                    "見た目の設定を読めません（ファイルには残っています）",
                    "Cannot read the look settings (kept in the file)"
                ),
                lang.io_error(&e)
            ))
        }
    };
    doc.restore_look(look).map_err(|e| {
        format!(
            "{}: {}",
            lang.pick(
                "見た目の設定を戻せません",
                "Cannot restore the look settings"
            ),
            lang.core_error(&e)
        )
    })
}

/// 文書の見た目の設定と、プロジェクトの同じセットの `look.json` が違うセットだけ、`look.json` を置き換える（`looks` はセットの ID と
/// 今の設定）。既定（標準・値なし）はエントリを消す。読めないエントリは、見た目が既定のままなら残す（読めなかったものを黙って消さない）。
/// 見た目を変えたセットの読めないエントリは今の設定で上書きし、そのセットの ID を返す（呼び手が利用者に知らせる）。
pub fn write_into(
    mut project: Project,
    looks: &[(&str, &MaterialLook)],
    lang: Lang,
) -> Result<(Project, Vec<String>), String> {
    let mut overwritten = Vec::new();
    for (id, look) in looks {
        // 受けた見た目だけのエントリの本体は既定（無いのと同じ）
        let stored = project.look(id).map(|l| l.filter(|l| !l.is_default()));
        let next = (!look.is_default()).then_some(*look);
        match stored {
            Ok(stored) if stored.as_ref() == next => continue,
            Err(_) if next.is_none() => continue,
            Err(_) => overwritten.push((*id).to_owned()),
            _ => {}
        }
        project = project.with_look(id, next).map_err(|e| {
            format!(
                "{}: {}",
                lang.pick("見た目の設定を書けません", "Cannot write the look settings"),
                lang.io_error(&e)
            )
        })?;
    }
    Ok((project, overwritten))
}
