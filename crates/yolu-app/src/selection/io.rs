//! 選択範囲と .ylp（`selection.bin`）の受け渡し。読み込みは文書を作った直後に `restore_selection`（Undo の段も版も増やさない）、
//! 保存は正本を書いたあとに `selection.bin` だけを置き換える（`Project::with_selection`）。選択範囲は文書と別のエントリなので、
//! 描いていないセットの正本はバイト列のまま残したまま、選択範囲だけが変わっていれば書き換える。
//!
//! 名前を付けて残した選択範囲（`sets/<ID>/selections.json` と中身。形式 8）も同じ流儀: 読み込みは `restore_saved_into`（読めない項目は
//! 飛ばして理由を返し、ファイルには残る）、保存は `write_saved_into`（残した選択範囲を変えたセットだけ書き換える。使う文書だけが形式 8）。

use yolu_io::saved_selections::{
    SavedSelection as StoredSaved, SavedSelections, SkipReason, Skipped,
};
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
        lang.with_reason(
            lang.pick("選択範囲を読めません", "Cannot read the selection"),
            lang.io_error(&e),
        )
    })?;
    doc.restore_selection(Some(mask)).map_err(|e| {
        lang.with_reason(
            lang.pick("選択範囲を戻せません", "Cannot restore the selection"),
            lang.core_error(&e),
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
            lang.with_reason(
                lang.pick("選択範囲の形にできません", "Cannot convert the selection"),
                lang.io_error(&e),
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
            lang.with_reason(
                lang.pick("選択範囲を書けません", "Cannot write the selection"),
                lang.io_error(&e),
            )
        })?;
    }
    Ok(project)
}

/// 文書の大きさを変えるセット（`sizes` はセットの ID と、新しい文書の幅・高さ・タイルの大きさ）の、古い大きさの今の選択範囲
/// （`selection.bin`）を外したプロジェクト。新しい文書とは合わず、正本を書き直すときの検証が断るため。今の選択範囲は、このあと
/// `write_into` が文書のものを書く。大きさが合うセットと選択範囲の無いセットには触れない。
pub fn without_stale(
    project: &Project,
    sizes: &[(&str, (u32, u32, u32))],
    lang: Lang,
) -> Result<Project, String> {
    let mut project = project.clone();
    for (id, (w, h, ts)) in sizes {
        let stale = project
            .sets()
            .iter()
            .find(|s| s.id == *id)
            .and_then(|s| s.selection.as_ref())
            .is_some_and(|s| {
                (s.width(), s.height(), s.tile_size()) != (*w as i32, *h as i32, *ts as i32)
            });
        if stale {
            project = project.with_selection(id, None).map_err(|e| {
                lang.with_reason(
                    lang.pick(
                        "古い選択範囲を外せません",
                        "Cannot drop the outdated selection",
                    ),
                    lang.io_error(&e),
                )
            })?;
        }
    }
    Ok(project)
}

/// 飛ばした項目の理由（短い文）。
fn skip_text(lang: Lang, s: &Skipped) -> String {
    let reason = match &s.reason {
        SkipReason::Index(_) => lang.pick("索引を読めません", "the index cannot be read"),
        SkipReason::Item(_) => lang.pick("項目の形が正しくありません", "malformed item"),
        SkipReason::MissingContent => lang.pick("中身がありません", "content is missing"),
        SkipReason::UnreadableContent(_) => lang.pick("中身を読めません", "content cannot be read"),
        SkipReason::WrongSize => lang.pick(
            "キャンバスと大きさが違います",
            "size differs from the canvas",
        ),
        SkipReason::DuplicateName => lang.pick("名前が重なっています", "duplicate name"),
        SkipReason::TooMany => lang.pick("数の上限を超えています", "over the limit"),
    };
    match &s.name {
        Some(name) => format!("{name} ({reason})"),
        None => reason.to_owned(),
    }
}

/// 開いた .ylp のセットの、名前を付けて残した選択範囲を文書へ戻す（`Project::saved_selections` の結果）。読めた項目は戻し、飛ばした
/// 項目は理由を返す（エントリはファイルにバイト列のまま残り、残した選択範囲を変えて保存するまで保たれる）。
pub fn restore_saved_into(
    doc: &mut Document,
    read: SavedSelections,
    lang: Lang,
) -> Result<(), String> {
    let mut skipped = read.skipped;
    let mut list = Vec::with_capacity(read.items.len());
    for item in read.items {
        match item.selection.to_core() {
            Ok(mask) => list.push(yolu_core::SavedSelection {
                name: item.name,
                mask,
            }),
            Err(_) => skipped.push(Skipped {
                index: 0,
                name: Some(item.name),
                reason: SkipReason::UnreadableContent(String::new()),
            }),
        }
    }
    let restored = doc.restore_saved_selections(list).map_err(|e| {
        lang.with_reason(
            lang.pick(
                "覚えた選択範囲を戻せません",
                "Cannot restore the remembered selections",
            ),
            lang.core_error(&e),
        )
    });
    // 文書へ戻せなかった並びは何も戻さない。読めなかった項目は ファイルに残っていることと一緒に言う
    restored?;
    if skipped.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = skipped.iter().map(|s| skip_text(lang, s)).collect();
    Err(lang.kept_in_file(lang.with_reason(
        lang.pick(
            "読めない覚えた選択範囲があります",
            "Some remembered selections cannot be read",
        ),
        list.join(lang.pick("、", ", ")),
    )))
}

/// 文書の大きさを変えるセットの ID（`sizes` はセットの ID と、新しい文書の幅・高さ・タイルの大きさ）。プロジェクトの同じ ID のセットの
/// 今の大きさと違うもの。新しいセットや大きさが同じセットは入らない。`write_saved_into` が、古い大きさの残した選択範囲のエントリを外す
/// セットを知るために、正本を書き直す前のプロジェクトで求める。
pub fn resized_sets(base: &Project, sizes: &[(&str, (u32, u32, u32))]) -> Vec<String> {
    sizes
        .iter()
        .filter(|(id, (w, h, ts))| {
            base.sets().iter().find(|s| s.id == *id).is_some_and(|s| {
                (
                    s.document.width(),
                    s.document.height(),
                    s.document.tile_size(),
                ) != (*w as i32, *h as i32, *ts as i32)
            })
        })
        .map(|(id, _)| (*id).to_owned())
        .collect()
}

/// セットごとの、名前を付けて残した選択範囲を、プロジェクトのものと違うセットだけ置き換える（`lists` はセットの ID と文書の今の並び、
/// `resized` は文書の大きさを変えたセットの ID。`resized_sets`）。並びが開いたときに読めた並びと同じなら、読めなかった項目（壊れた
/// エントリ）にも触れずにバイト列のまま残す。ただし大きさを変えたセットの、古い大きさのエントリ（`WrongSize`）は新しい文書と合わない
/// ので、並びが空のままでも外す（文書の大きさを変えない限り、大きさの合わない項目はファイルに残す）。変えたセットの、読めなかった
/// 項目（大きさを変えたあとの古いものは数えない）を置き換えたときは、そのセットの ID を返す（呼び手が利用者に知らせる）。
pub fn write_saved_into(
    mut project: Project,
    lists: &[(&str, &[yolu_core::SavedSelection])],
    resized: &[String],
    lang: Lang,
) -> Result<(Project, Vec<String>), String> {
    let fail = |what: &str, why: String| lang.with_reason(what, why);
    let mut overwritten = Vec::new();
    for (id, list) in lists {
        let stored = project.saved_selections(id).map_err(|e| {
            fail(
                lang.pick(
                    "覚えた選択範囲を読めません",
                    "Cannot read the remembered selections",
                ),
                lang.io_error(&e),
            )
        })?;
        let mut next = Vec::with_capacity(list.len());
        for s in *list {
            let selection = Selection::from_core(&s.mask).map_err(|e| {
                fail(
                    lang.pick(
                        "覚えた選択範囲の形にできません",
                        "Cannot convert the remembered selections",
                    ),
                    lang.io_error(&e),
                )
            })?;
            next.push(StoredSaved {
                name: s.name.clone(),
                selection,
            });
        }
        let outdated = resized.iter().any(|r| r == id)
            && stored
                .skipped
                .iter()
                .any(|s| matches!(s.reason, SkipReason::WrongSize));
        if next == stored.items && !outdated {
            continue;
        }
        if stored
            .skipped
            .iter()
            .any(|s| !matches!(s.reason, SkipReason::WrongSize))
        {
            overwritten.push((*id).to_owned());
        }
        project = project.with_saved_selections(id, &next).map_err(|e| {
            fail(
                lang.pick(
                    "覚えた選択範囲を書けません",
                    "Cannot write the remembered selections",
                ),
                lang.io_error(&e),
            )
        })?;
    }
    Ok((project, overwritten))
}
