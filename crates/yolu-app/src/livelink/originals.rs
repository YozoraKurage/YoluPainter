//! 元の絵（相手のマテリアルの Color の流し込み先の絵のファイル）を、テクスチャセットの一番下のレイヤー「元の絵」として入れる。
//!
//! 決まり:
//! - 入れるのは、頼みを当てたときに**新しく作ったセット**と、**何も触っていない最初のセット**（開いた・保存したファイルが無く、描いていない・
//!   変えていない最初のプロジェクトの、最初のセット）だけ。描いたセット・利用者が開いたプロジェクトのセットには入れない（描いたものを
//!   黙って変えない）。
//! - 何も触っていない最初のセットは、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限。`sets::fit_side`）で作り直した文書へ入れる。
//!   作り直した文書に入らないとき（予算）は、今の文書へ縮めて入れる。新規プロジェクトの窓で解像度を選んで作ったプロジェクト
//!   （`AppState::resolution_chosen`）の最初のセットは作り直さない。
//! - 絵の無いマテリアル（テクスチャの無いスロット・Unity の中にしかない絵）と、読めない絵は、不透明な白（Unity が絵の無いスロットを描く
//!   既定の白）の「元の絵」を入れる。読めない絵は理由を知らせる。
//! - 絵の大きさがセットと違うときは、セットの大きさへ拡大縮小する（`images::resample`）。リニアの絵は sRGB の画素へ直して読んである。
//! - 一番下に足した層は初期化で、Undo の履歴には入れない（セットを作った直後の状態の一部）。
//! - 層の欄の印: 直した・拡大縮小した絵のとき、理由をツールチップに出す（保存した .ylp には入らない）。

use super::images::{resample, Picture};
use crate::engine::{Channel, Document, LayerId, PixelClipboard};
use crate::lang::Lang;
use crate::sets::fit_side;
use crate::state::AppState;

/// 層の欄に出す印 1 つ（入れた「元の絵」の層ごと。セッションの中だけで、保存しない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalMark {
    pub doc: u128,
    pub layer: LayerId,
    /// リニアの絵を sRGB の画素へ直した。
    pub converted: bool,
    /// セットの大きさへ拡大縮小した（元の大きさ）。
    pub resized_from: Option<(u32, u32)>,
}

impl OriginalMark {
    /// 印を出すか（原本のそのままの値だけなら出さない）。
    pub fn is_noted(&self) -> bool {
        self.converted || self.resized_from.is_some()
    }

    /// ツールチップ（理由を 1 行ずつ）。
    pub fn tooltip(&self, lang: Lang, set: (u32, u32)) -> String {
        let mut lines = Vec::new();
        if self.converted {
            lines.push(lang.pick(
                "リニアのテクスチャを sRGB の画素へ直しています".to_owned(),
                "Converted from a linear texture to sRGB pixels".to_owned(),
            ));
        }
        if let Some((w, h)) = self.resized_from {
            lines.push(lang.pick(
                format!(
                    "{w}×{h} をセットの大きさ {}×{} に拡大縮小しています",
                    set.0, set.1
                ),
                format!("Scaled from {w}×{h} to the set size {}×{}", set.0, set.1),
            ));
        }
        lines.join("\n")
    }
}

/// 入れた「元の絵」の層の印の一覧（`AppState` が持つ）。
#[derive(Debug, Default)]
pub struct OriginalMarks(Vec<OriginalMark>);

impl OriginalMarks {
    /// 印の数の上限（古いものから捨てる）。
    const MAX: usize = 256;

    /// 文書 `doc` の層 `layer` の印（無ければ None）。
    pub fn get(&self, doc: u128, layer: LayerId) -> Option<&OriginalMark> {
        self.0.iter().find(|m| m.doc == doc && m.layer == layer)
    }

    pub(crate) fn push(&mut self, mark: OriginalMark) {
        if self.0.len() >= Self::MAX {
            self.0.remove(0);
        }
        self.0.push(mark);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 文書の大きさの画素を、一番下のレイヤー「元の絵」として入れる（履歴には入れない: 作った直後の初期化）。入れた層を返す。
/// 入らなければ文書は変えない。
fn add_bottom_layer(doc: &mut Document, pixels: Vec<u8>, lang: Lang) -> Result<LayerId, String> {
    let clip = PixelClipboard::from_image(doc.width(), doc.height(), pixels, Channel::Color)
        .map_err(|e| lang.core_error(&e))?;
    let name = lang.pick("元の絵", "Original");
    let pasted = doc
        .paste_as_layer(&clip, Channel::Color, Some(name), None)
        .map_err(|e| lang.core_error(&e))?;
    if let Err(e) = doc.move_layer(pasted.layer, 0) {
        let _ = doc.undo();
        return Err(lang.core_error(&e));
    }
    let _ = doc.clear_history();
    Ok(pasted.layer)
}

/// 絵を `size` の大きさにする（同じなら写すだけ）。
fn fit(picture: &Picture, size: (u32, u32)) -> Vec<u8> {
    if (picture.width, picture.height) == size {
        picture.rgba.clone()
    } else {
        resample(
            &picture.rgba,
            [picture.width, picture.height],
            [size.0, size.1],
        )
    }
}

/// 元の絵を、セットの文書の一番下のレイヤーとして入れる（履歴には入れない）。`refit` は何も触っていない最初のセットで、元の絵の大きさで
/// 作り直した文書へ入れる（入らなければ今の文書へ、セットの大きさに縮めて入れる）。`converted` はリニアから直した絵。
pub fn install(
    state: &mut AppState,
    index: usize,
    picture: &Picture,
    converted: bool,
    refit: bool,
    lang: Lang,
) -> Result<(), String> {
    let source = (picture.width, picture.height);
    let mark = |doc: &Document, layer: LayerId| OriginalMark {
        doc: doc.id(),
        layer,
        converted,
        resized_from: (source != (doc.width(), doc.height())).then_some(source),
    };
    if refit {
        if let Some((doc, layer)) = rebuilt_with(state.set_doc(index), picture, lang) {
            let mark = mark(&doc, layer);
            state.swap_untouched_set_document(index, doc);
            state.link_originals.push(mark);
            return Ok(());
        }
    }
    let doc = state.set_doc_mut(index);
    let size = (doc.width(), doc.height());
    let layer = add_bottom_layer(doc, fit(picture, size), lang)?;
    let mark = mark(state.set_doc(index), layer);
    state.link_originals.push(mark);
    Ok(())
}

/// `like`（何も触っていない最初のセットの文書）の代わりに、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限）で作り直した文書へ
/// 元の絵を入れたもの。大きさが同じなら作り直さず、作り直した文書に入らない（予算）ときも None（今の文書へ縮めて入れる）。
fn rebuilt_with(like: &Document, picture: &Picture, lang: Lang) -> Option<(Document, LayerId)> {
    let side = fit_side(picture.width.max(picture.height));
    if (side, side) == (like.width(), like.height()) {
        return None;
    }
    let mut doc = crate::newproject::new_set_document(
        side,
        side,
        like.tile_size(),
        lang,
        &crate::newproject::used_channels(like),
        like.normal_settings(),
    )
    .ok()?;
    // 予算は今の文書と同じ（入らなければ今の文書へ戻る）
    doc.set_minimum_undo_steps(like.minimum_undo_steps()).ok()?;
    doc.set_undo_budget_bytes(like.undo_budget_bytes()).ok()?;
    doc.set_stroke_budget_bytes(like.stroke_budget_bytes())
        .ok()?;
    doc.set_source_budget_bytes(like.source_budget_bytes())
        .ok()?;
    let layer = add_bottom_layer(&mut doc, fit(picture, (side, side)), lang).ok()?;
    Some((doc, layer))
}

/// 絵の無いマテリアルのセットに、不透明な白の「元の絵」を入れる（Unity は絵の無いスロットを既定の白で描く。層の欄の印は付けない）。
pub fn install_white(state: &mut AppState, index: usize, lang: Lang) -> Result<(), String> {
    let doc = state.set_doc_mut(index);
    let pixels = vec![255u8; doc.width() as usize * doc.height() as usize * 4];
    add_bottom_layer(doc, pixels, lang).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_shown_only_for_a_value_that_is_not_the_original_files() {
        let mark = |converted, resized_from| OriginalMark {
            doc: 1,
            layer: LayerId(1),
            converted,
            resized_from,
        };
        assert!(!mark(false, None).is_noted());
        assert!(mark(true, None).is_noted());
        assert!(mark(false, Some((4096, 2048))).is_noted());
        let tip = mark(true, Some((4096, 2048))).tooltip(Lang::En, (2048, 2048));
        assert_eq!(tip.lines().count(), 2);
        assert!(tip.contains("4096×2048") && tip.contains("2048×2048"));
    }
}
