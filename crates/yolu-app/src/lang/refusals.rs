//! 断りの文（今の状態で受けられない操作の理由）。同じ断りは、どの操作でも同じ文にする（呼ぶ側に文字のまま書かない）。
//! 知らせるときは `AppState::refuse`（種類は Refusal）。ツールチップ・押せない理由に出すときも同じ関数を通す。

use super::Lang;
use crate::pathtool::edit::Refusal;
use yolu_core::CoreError;

/// 描いている間（ストローク・移動と変形のドラッグ・範囲のツールやパスの途中）は受けない。
pub fn during_stroke(lang: Lang) -> &'static str {
    crate::crash::problem(lang.pick("描いている間はできません。", "Not while drawing."))
}

/// 描いている間・表示を回すドラッグの間は、表示を回さない。
pub fn during_stroke_or_drag(lang: Lang) -> &'static str {
    crate::crash::problem(lang.pick(
        "描いている間・ドラッグの間は回せません。",
        "Cannot rotate during a stroke or drag.",
    ))
}

/// 読むだけのテクスチャセットは変えない（`reason` は読むだけになった理由）。
pub fn read_only_set(lang: Lang, reason: &str) -> String {
    lang.with_reason(
        lang.pick(
            "このテクスチャセットは読むだけです",
            "This texture set is read-only",
        ),
        reason,
    )
}

/// テクスチャセットの数が上限に当たったときの理由（ウィンドウの下の帯・ツールチップ）。
pub fn set_limit(lang: Lang) -> String {
    lang.pick(
        format!(
            "1 つのプロジェクトのテクスチャセットは {} までです",
            crate::newproject::MAX_SETS
        ),
        format!(
            "A project has at most {} texture sets",
            crate::newproject::MAX_SETS
        ),
    )
}

/// テクスチャセットの大きさが選べる値でない（新規・構成のウィンドウの下の帯）。
pub fn set_size(lang: Lang) -> String {
    lang.pick(
        format!(
            "セットの大きさは {} のどれかです",
            crate::newproject::RESOLUTIONS
                .map(|r| r.to_string())
                .join("・")
        ),
        format!(
            "A texture set's size is one of {}",
            crate::newproject::RESOLUTIONS
                .map(|r| r.to_string())
                .join(", ")
        ),
    )
}

/// 保存を断る・待たせる理由（保存の途中に、ほかの保存・開く・新規・配布用に保存・更新の入れ替えが来たとき）。
pub fn saving(lang: Lang) -> &'static str {
    lang.pick("保存の途中です", "A save is in progress")
}

/// 棚の個数が上限（`MAX_RESOURCES`）に達しているときの短い理由。
pub fn shelf_full(lang: Lang) -> &'static str {
    lang.pick("アセットがいっぱいです", "Assets are full")
}

/// 点の操作を断る理由の文。
/// パスの点の編集の断り（点の数の上限・閉じたパス・種類の違いなど）。
pub fn path_edit(lang: Lang, refusal: Refusal) -> String {
    match refusal {
        Refusal::TooMany => lang
            .pick(
                "パスの点は 4096 個までです",
                "A path has at most 4096 points",
            )
            .into(),
        Refusal::NeedThree => lang
            .pick(
                "閉じるには 3 点以上が要ります",
                "Closing needs at least 3 points",
            )
            .into(),
        Refusal::AlreadyClosed => lang.pick("もう閉じています", "Already closed").into(),
        Refusal::NotClosed => lang.pick("閉じていません", "Not closed").into(),
        Refusal::NoPoint => lang.pick("その点がありません", "No such point").into(),
        Refusal::OtherKind => lang
            .pick(
                "このレイヤーのパスは 2D と 3D が違います",
                "This layer's path is of the other kind (2D or 3D)",
            )
            .into(),
        Refusal::Invalid(why) => lang.core_error(&CoreError::InvalidArgument(why)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_read_in_both_languages() {
        for lang in Lang::ALL {
            assert!(!during_stroke(lang).is_empty());
        }
        assert_eq!(during_stroke(Lang::Ja), "描いている間はできません。");
        assert_eq!(during_stroke(Lang::En), "Not while drawing.");
        assert_eq!(
            read_only_set(Lang::Ja, "理由"),
            "このテクスチャセットは読むだけです（理由）。"
        );
        assert_eq!(
            read_only_set(Lang::En, "Waiting for inputs"),
            "This texture set is read-only (Waiting for inputs)."
        );
    }
}
