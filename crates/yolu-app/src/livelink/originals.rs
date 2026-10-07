//! 元の絵（相手のマテリアルの Color の流し込み先の絵のファイル）を、テクスチャセットの一番下のレイヤー「元の絵」として入れる。
//!
//! 決まり:
//! - 入れるのは、頼みを当てたときに**新しく作ったセット**と、**何も触っていない最初のセット**（開いた・保存したファイルが無く、描いていない・
//!   変えていない最初のプロジェクトの、最初のセット）だけ。描いたセット・利用者が開いたプロジェクトのセットには入れない（描いたものを
//!   黙って変えない）。
//! - 何も触っていない最初のセットは、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限。`sets::fit_side`）で作り直した文書へ入れる。
//!   作り直した文書に入らないとき（予算）は、今の文書へ縮めて入れる。新規プロジェクトのウィンドウで解像度を選んで作ったプロジェクト
//!   （`AppState::resolution_chosen`）の最初のセットは作り直さない。
//! - 絵の無いマテリアル（テクスチャの無いスロット・Unity の中にしかない絵）と、読めない絵は、不透明な白（Unity が絵の無いスロットを描く
//!   既定の白）の「元の絵」を入れる。読めない絵は理由を知らせる。
//! - 絵の大きさがセットと違うときは、セットの大きさへ拡大縮小する（`images::resample`）。リニアの絵は sRGB の画素へ直して読んである。
//! - 一番下に足したレイヤーは初期化で、Undo の履歴には入れない（セットを作った直後の状態の一部）。
//! - レイヤーの欄の印: 直した・拡大縮小した絵のとき、理由をツールチップに出す（保存した .ylp には入らない）。
//! - Color の流し込み先の絵が PSD なら（`load` がレイヤーのまま読めたとき）、平らな「元の絵」の代わりに、PSD の取り込みと同じ写しの文書を
//!   そのセットの文書にする（`install_layers`。条件は上と同じ: 新しく作ったセットと何も触っていない最初のセットだけ）。PSD のレイヤーが下に並び、
//!   セットにあった空のレイヤー（使うチャンネル・Normal の設定）はその上に残る。大きさは PSD のキャンバスのまま（レイヤー・マスクの画素を拡大縮小
//!   すると、取り込んだレイヤーが PSD のレイヤーと違う画素になるので、新規プロジェクトのウィンドウで選んだ解像度にも合わせない）。取り込みで落とす・変わる物は、
//!   PSD の取り込みの確認のウィンドウと同じ名前で知らせに出す（`layers_note`）。
//! - 送り直しで元の絵のファイル（道・更新時刻と大きさ・色の扱い）が変わったとき、元の絵を入れた直後のまま（文書の ID と版が同じ）の
//!   セットは、空のセットの文書に戻してから（`reset_untouched`）同じ決まりで入れ直す。触ったセットは変えずに知らせる（`changed_note`）。
//!   入れた直後かどうかはセッションの中だけで覚える（`super::LiveLink`）。

use super::images::{resample, Picture, PictureError};
use super::load::PsdLayers;
use crate::engine::{Channel, Document, LayerId, PixelClipboard};
use crate::lang::Lang;
use crate::sets::fit_side;
use crate::state::AppState;
use yolu_io::psd::{CopyRefusal, ImportAction};

/// レイヤーの欄に出す印 1 つ（入れた「元の絵」のレイヤーごと。セッションの中だけで、保存しない）。
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

/// 入れた「元の絵」のレイヤーの印の一覧（`AppState` が持つ）。
#[derive(Debug, Default)]
pub struct OriginalMarks(Vec<OriginalMark>);

impl OriginalMarks {
    /// 印の数の上限（古いものから捨てる）。
    const MAX: usize = 256;

    /// 文書 `doc` のレイヤー `layer` の印（無ければ None）。
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

/// 文書の大きさの画素を、一番下のレイヤー「元の絵」として入れる（履歴には入れない: 作った直後の初期化）。入れたレイヤーを返す。
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

/// `like`（何も触っていないセットの文書）と同じ空のセットの文書を、大きさ `size` で作る: 一番上のレイヤーの名前・使うチャンネル・Normal の設定・
/// タイルの大きさ・予算を受け継いだ、空のレイヤー 1 つの文書（履歴は空）。
fn blank_like(like: &Document, size: (u32, u32), lang: Lang) -> Result<Document, String> {
    let core = |e: crate::engine::CoreError| lang.core_error(&e);
    let mut doc = crate::newproject::new_set_document(
        size.0,
        size.1,
        like.tile_size(),
        lang,
        &crate::newproject::used_channels(like),
        like.normal_settings(),
    )?;
    if let (Some(top), Some(first)) = (like.layers().last(), doc.layers().first()) {
        let first = first.id();
        if doc.layers()[0].name() != top.name() {
            doc.set_layer_name(first, top.name()).map_err(core)?;
            doc.clear_history().map_err(core)?;
        }
    }
    // 予算は今の文書と同じ
    doc.set_minimum_undo_steps(like.minimum_undo_steps())
        .map_err(core)?;
    doc.set_undo_budget_bytes(like.undo_budget_bytes())
        .map_err(core)?;
    doc.set_stroke_budget_bytes(like.stroke_budget_bytes())
        .map_err(core)?;
    doc.set_source_budget_bytes(like.source_budget_bytes())
        .map_err(core)?;
    Ok(doc)
}

/// `like`（何も触っていないセットの文書）の代わりに、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限）で作り直した文書へ
/// 元の絵を入れたもの。大きさが同じなら作り直さず、作り直した文書に入らない（予算）ときも None（今の文書へ縮めて入れる）。
fn rebuilt_with(like: &Document, picture: &Picture, lang: Lang) -> Option<(Document, LayerId)> {
    let side = fit_side(picture.width.max(picture.height));
    if (side, side) == (like.width(), like.height()) {
        return None;
    }
    let mut doc = blank_like(like, (side, side), lang).ok()?;
    let layer = add_bottom_layer(&mut doc, fit(picture, (side, side)), lang).ok()?;
    Some((doc, layer))
}

/// 元の絵を入れた直後のまま（何も触っていない）セット `index` の文書を、元の絵を入れる前の空のセットの文書（同じ大きさ）に戻す。
/// 送り直しで元の絵のファイルが変わったとき、入れ直す前に（呼ぶ側が、入れた直後のままかを確かめる）。
pub fn reset_untouched(state: &mut AppState, index: usize, lang: Lang) -> Result<(), String> {
    let like = state.set_doc(index);
    let doc = blank_like(like, (like.width(), like.height()), lang)?;
    state.swap_untouched_set_document(index, doc);
    Ok(())
}

/// Color の流し込み先の PSD をレイヤーのまま、セット `index` の文書にする（呼ぶ側が、新しく作ったセットか何も触っていない最初のセットかを
/// 確かめる）。PSD のレイヤーの上に、今のセットの文書の一番上のレイヤーの名前で空のレイヤーを置き、今の文書が使うチャンネルと Normal の設定を受け継ぐ。
/// 履歴には入れない（セットを作った直後の状態の一部）。入れられなければ文書は変えない。
pub fn install_layers(
    state: &mut AppState,
    index: usize,
    layers: PsdLayers,
    lang: Lang,
) -> Result<(), String> {
    let core = |e: crate::engine::CoreError| lang.core_error(&e);
    let like = state.set_doc(index);
    let channels = crate::newproject::used_channels(like);
    let normal = like.normal_settings();
    let name = like
        .layers()
        .last()
        .map(|l| l.name().to_owned())
        .unwrap_or_else(|| format!("{} 1", lang.pick("レイヤー", "Layer")));
    let mut doc = layers.doc;
    let top = doc.add_layer(&name).map_err(core)?;
    for channel in &channels {
        doc.set_channel_enabled(top, *channel, true).map_err(core)?;
    }
    if doc.normal_settings() != normal {
        doc.set_normal_settings(normal, false).map_err(core)?;
    }
    doc.clear_history().map_err(core)?;
    crate::look::apply_new_set_look(&mut doc);
    state.swap_untouched_set_document(index, doc);
    Ok(())
}

/// レイヤーのまま入れた PSD の、取り込みで落とす・変わる物の知らせ（無視だけなら None）。名前は PSD の取り込みの確認のウィンドウと同じ。
pub fn layers_note(lang: Lang, set: &str, layers: &PsdLayers) -> Option<String> {
    /// 扱いごとに並べる名前の数（残りは数だけ）。
    const SHOWN: usize = 3;
    let sorted = crate::psd_import::sorted(&layers.notes);
    let names = |action: ImportAction| -> Option<String> {
        let all: Vec<String> = sorted
            .iter()
            .filter(|n| n.action == action)
            .map(|n| crate::psd_import::feature_text(lang, n))
            .collect();
        if all.is_empty() {
            return None;
        }
        let mut text = all
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join(lang.pick("、", ", "));
        if all.len() > SHOWN {
            let rest = all.len() - SHOWN;
            text += &lang.pick(format!(" ほか {rest} 件"), format!(" and {rest} more"));
        }
        Some(text)
    };
    let parts: Vec<String> = [
        (ImportAction::Dropped, "落とす物", "dropped"),
        (ImportAction::Changed, "変わる物", "changed"),
    ]
    .into_iter()
    .filter_map(|(action, ja, en)| {
        names(action).map(|list| lang.pick(format!("{ja}（{list}）"), format!("{en} ({list})")))
    })
    .collect();
    if parts.is_empty() {
        return None;
    }
    let file = lang.quote(&file_name(&layers.path));
    Some(lang.pick(
        format!("{set}: {file}の取り込みで{}", parts.join("・")),
        format!("{set}: importing {file}, {}", parts.join(", ")),
    ))
}

/// レイヤーのまま取り込めず、平らにして入れた元の絵の PSD の知らせ。
pub fn flattened_note(lang: Lang, set: &str, path: &str, why: &CopyRefusal) -> String {
    let file = lang.quote(&file_name(std::path::Path::new(path)));
    lang.with_reason(
        lang.pick(
            format!("{set}: {file}をレイヤーのまま入れられないので、平らにして入れました"),
            format!("{set}: {file} was flattened instead of kept as layers"),
        ),
        crate::lang::psd_copy_refusal(lang, why),
    )
}

/// 触ったセットの元の絵のファイルが変わった（セットは変えない）知らせ。
pub fn changed_note(lang: Lang, set: &str, path: &str) -> String {
    let file = lang.quote(&file_name(std::path::Path::new(path)));
    lang.pick(
        format!("{set}: 元の絵{file}が変わりました"),
        format!("{set}: the original {file} has changed"),
    )
}

/// 送り直しで変わった元の絵のファイルを読めなかったので、入れ直さずにセットをそのままにした知らせ。
pub fn unreadable_kept_note(lang: Lang, set: &str, path: &str, why: &PictureError) -> String {
    let file = lang.quote(&file_name(std::path::Path::new(path)));
    lang.with_reason(
        lang.pick(
            format!("{set}: 元の絵{file}を読めないので、セットはそのままです"),
            format!("{set}: the original {file} cannot be read, so the set is kept"),
        ),
        why.text(lang),
    )
}

/// 道のファイル名（無ければ道のまま）。
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 絵の無いマテリアルのセットに、不透明な白の「元の絵」を入れる（Unity は絵の無いスロットを既定の白で描く。レイヤーの欄の印は付けない）。
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

    fn has_japanese(text: &str) -> bool {
        text.chars()
            .any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
    }

    #[test]
    fn what_a_psd_original_drops_or_changes_is_named_and_ignored_things_are_not() {
        use yolu_io::psd::{ImportFeature, ImportNote};
        let note = |feature, action| ImportNote {
            feature,
            action,
            layers: vec!["下".into()],
            count: 1,
            details: vec![],
        };
        let layers = |notes| PsdLayers {
            path: "/work/Body.psd".into(),
            doc: crate::engine::Document::new(4, 4).unwrap(),
            notes,
        };
        let only_ignored = layers(vec![note(ImportFeature::GlobalMask, ImportAction::Ignored)]);
        assert_eq!(layers_note(Lang::Ja, "Skin", &only_ignored), None);
        let mut notes = vec![
            note(ImportFeature::LayerEffects, ImportAction::Changed),
            note(ImportFeature::TextLayer, ImportAction::Dropped),
            note(ImportFeature::GlobalMask, ImportAction::Ignored),
        ];
        let ja = layers_note(Lang::Ja, "Skin", &layers(notes.clone())).unwrap();
        assert_eq!(
            ja,
            "Skin: 「Body.psd」の取り込みで落とす物（テキストのデータ）・変わる物（レイヤー効果）"
        );
        let en = layers_note(Lang::En, "Skin", &layers(notes.clone())).unwrap();
        assert!(!has_japanese(&en), "{en}");
        assert!(en.contains("dropped (Text data)") && en.contains("changed (Layer effects)"));
        // 多いときは 3 つまで名前、残りは数
        for f in [
            ImportFeature::VectorMask,
            ImportFeature::BlendIf,
            ImportFeature::Knockout,
        ] {
            notes.push(note(f, ImportAction::Changed));
        }
        let ja = layers_note(Lang::Ja, "Skin", &layers(notes)).unwrap();
        assert!(ja.ends_with("ほか 1 件）"), "{ja}");
        // レイヤーのまま入れられなかった理由
        let why = CopyRefusal::CanvasTooLarge {
            width: 9000,
            height: 9000,
        };
        let ja = flattened_note(Lang::Ja, "Skin", "/work/Body.psd", &why);
        assert!(
            ja.starts_with("Skin: 「Body.psd」") && ja.contains("9000×9000"),
            "{ja}"
        );
        let en = flattened_note(Lang::En, "Skin", "/work/Body.psd", &why);
        assert!(!has_japanese(&en) && en.contains("flattened"), "{en}");
        // 触ったセットの元の絵が変わった
        assert_eq!(
            changed_note(Lang::Ja, "Skin", "/work/Body.psd"),
            "Skin: 元の絵「Body.psd」が変わりました"
        );
        let en = changed_note(Lang::En, "Skin", "/work/Body.psd");
        assert!(!has_japanese(&en) && en.contains("\"Body.psd\""), "{en}");
    }
}
