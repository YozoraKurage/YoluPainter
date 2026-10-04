//! 見た目の設定と .ylp（`sets/<ID>/look.json`）の受け渡し。読み込みは文書を作った直後に `restore_look`（Undo の段も版も増やさない）、
//! 保存は正本を書いたあとに `look.json` だけを置き換える（選択範囲と同じ流儀。`selection::io`）。正本を書き直さないセットでも、見た目の
//! 設定だけが変わっていれば書き換える。

use yolu_core::look::MaterialLook;
use yolu_io::Project;

use crate::engine::Document;
use crate::lang::Lang;
use crate::state::AppState;

/// 開いた .ylp のセットの見た目の設定を文書へ戻す。読めなければ（壊れた・新しい形式）標準のまま開いて理由を返す（エントリは
/// ファイルにバイト列のまま残り、見た目を変えない限り保存でも残る）。
pub fn restore_into(doc: &mut Document, project: &Project, set_id: &str, lang: Lang) -> Result<(), String> {
    restore_from(doc, project.look(set_id), lang)
}

/// `restore_into` の、読んだ結果（`Project::look`）から戻す形（プロジェクトを借りたまま文書を作れない呼び手のため）。
pub fn restore_from(doc: &mut Document, stored: yolu_io::Result<Option<MaterialLook>>, lang: Lang) -> Result<(), String> {
    let look = match stored {
        Ok(Some(look)) => look,
        Ok(None) => return Ok(()),
        Err(e) => {
            return Err(format!(
                "{}: {}",
                lang.pick("見た目の設定を読めません（ファイルには残っています）", "Cannot read the look settings (kept in the file)"),
                lang.io_error(&e)
            ))
        }
    };
    doc.restore_look(look).map_err(|e| {
        format!(
            "{}: {}",
            lang.pick("見た目の設定を戻せません", "Cannot restore the look settings"),
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
        let stored = project.look(id);
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

/// 保存の口: 描けるセット（読むだけのセットは元のバイト列のまま）の見た目の設定を書く。読めなかったエントリを上書きしたセットの
/// ID も返す（`write_into`）。
pub fn save_into(state: &AppState, project: Project) -> Result<(Project, Vec<String>), String> {
    let looks: Vec<(&str, &MaterialLook)> = state
        .sets
        .iter()
        .enumerate()
        .filter(|(_, set)| set.read_only.is_none())
        .map(|(i, set)| (set.id.as_str(), state.set_doc(i).look()))
        .collect();
    write_into(project, &looks, state.lang)
}
