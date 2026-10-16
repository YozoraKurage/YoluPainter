//! 断りの文（今の状態で受けられない操作の理由）。同じ断りは、どの操作でも同じ文にする（呼ぶ側に文字のまま書かない）。
//! 知らせるときは `AppState::refuse`（種類は Refusal）。ツールチップ・押せない理由に出すときも同じ関数を通す。

use super::Lang;
use crate::pathtool::edit::Refusal;
use yolu_core::CoreError;

/// 描いている間（ストローク・移動と変形のドラッグ・範囲のツールやパスの途中）は受けない。
pub fn during_stroke(lang: Lang) -> &'static str {
    crate::crash::problem(lang.t("lang-not-while-drawing"))
}

/// 描いている間・表示を回すドラッグの間は、表示を回さない。
pub fn during_stroke_or_drag(lang: Lang) -> &'static str {
    crate::crash::problem(lang.t("lang-cannot-rotate-during-a-stroke"))
}

/// 読むだけのテクスチャセットは変えない（`reason` は読むだけになった理由）。
pub fn read_only_set(lang: Lang, reason: &str) -> String {
    lang.with_reason(lang.t("lang-this-texture-set-is-read"), reason)
}

/// テクスチャセットの数が上限に当たったときの理由（ウィンドウの下の帯・ツールチップ）。
pub fn set_limit(lang: Lang) -> String {
    lang.f(
        "lang-a-project-has-at-most",
        &[("MAX_SETS", &crate::newproject::MAX_SETS)],
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

/// 最近開いたプロジェクトのファイルが無い（開かずに断る理由）。
pub fn file_missing(lang: Lang) -> &'static str {
    lang.t("lang-the-file-does-not-exist")
}

/// 保存を断る・待たせる理由（保存の途中に、ほかの保存・開く・新規・配布用に保存・更新の入れ替えが来たとき）。
pub fn saving(lang: Lang) -> &'static str {
    lang.t("lang-a-save-is-in-progress")
}

/// 棚の個数が上限（`MAX_RESOURCES`）に達しているときの短い理由。
pub fn shelf_full(lang: Lang) -> &'static str {
    lang.t("lang-assets-are-full")
}

/// 点の操作を断る理由の文。
/// パスの点の編集の断り（点の数の上限・閉じたパス・種類の違いなど）。
pub fn path_edit(lang: Lang, refusal: Refusal) -> String {
    match refusal {
        Refusal::TooMany => lang.t("lang-a-path-has-at-most").into(),
        Refusal::NeedThree => lang.t("lang-closing-needs-at-least-3").into(),
        Refusal::AlreadyClosed => lang.t("lang-already-closed").into(),
        Refusal::NotClosed => lang.t("lang-not-closed").into(),
        Refusal::NoPoint => lang.t("lang-no-such-point").into(),
        Refusal::OtherKind => lang.t("lang-this-layer-s-path-is").into(),
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
