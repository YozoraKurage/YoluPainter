//! 選択範囲と .ylp（`selection.bin`）の受け渡し。読み込みは文書を作った直後に `restore_selection`（Undo の段も版も増やさない）、
//! 保存は正本を書いたあとに `selection.bin` だけを置き換える（`Project::with_selection`）。選択範囲は文書と別のエントリなので、
//! 描いていないセットの正本はバイト列のまま残したまま、選択範囲だけが変わっていれば書き換える。

use yolu_io::{Project, Selection};

use crate::engine::Document;
use crate::lang::Lang;

/// 開いた .ylp の選択範囲を文書へ戻す。読めなければ（大きさが文書と違うなど）選択なしのままにして理由を返す（黙って捨てない）。
pub fn restore_into(
    doc: &mut Document,
    selection: Option<&Selection>,
    lang: Lang,
) -> Result<(), String> {
    let Some(selection) = selection else {
        return Ok(());
    };
    let mask = selection.to_core().map_err(|e| {
        format!(
            "{}: {e}",
            lang.pick("選択範囲を読めません", "Cannot read the selection")
        )
    })?;
    doc.restore_selection(Some(mask)).map_err(|e| {
        format!(
            "{}: {e}",
            lang.pick("選択範囲を戻せません", "Cannot restore the selection")
        )
    })
}

/// 文書の選択範囲と、プロジェクトの同じセットの `selection.bin` が違うセットだけ、`selection.bin` を置き換える（`selections` は
/// セットの ID と今の選択範囲）。同じなら元のバイト列のまま。
pub fn write_into(
    mut project: Project,
    selections: &[(&str, Option<&crate::engine::SelectionMask>)],
    lang: Lang,
) -> Result<Project, String> {
    for (id, mask) in selections {
        let next = mask.map(Selection::from_core).transpose().map_err(|e| {
            format!(
                "{}: {e}",
                lang.pick(
                    "選択範囲を正本にできません",
                    "Cannot turn the selection into the document"
                )
            )
        })?;
        let stored = project
            .sets()
            .iter()
            .find(|s| s.id == *id)
            .and_then(|s| s.selection.as_ref());
        if next.as_ref() == stored {
            continue;
        }
        project = project.with_selection(id, next.as_ref()).map_err(|e| {
            format!(
                "{}: {e}",
                lang.pick("選択範囲を書けません", "Cannot write the selection")
            )
        })?;
    }
    Ok(project)
}
