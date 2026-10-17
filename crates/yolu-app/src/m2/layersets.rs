//! レイヤーセット（core の `document/layer_sets.rs`）の、状態を持たない部分: 操作の種類（[`LayerSetEdit`]）・名前・出力の読み・一番上のレイヤー。
//! 今のレイヤーセットと操作を当てる口は `sets.rs`（`AppState`）、メニューの項目は `m2_menu.rs`。

use crate::engine::{Channel, Document, LayerId, LayerSetId};
use crate::lang::Lang;

/// レイヤーセットの操作（文書を変える。どれも 1 回の Undo）。
#[derive(Clone, Debug, PartialEq)]
pub enum LayerSetEdit {
    /// 空のレイヤー 1 枚つきの新しいレイヤーセットを一覧の最後に足し、今のレイヤーセットにする。
    New,
    /// レイヤーセットを画素ごと写し、元のすぐ後に足して今のレイヤーセットにする。
    Duplicate(LayerSetId),
    Rename(LayerSetId, String),
    /// 一覧の位置（0 が最初）へ動かす。
    Move(LayerSetId, usize),
    /// 中のレイヤーごと消す。最後の 1 つは消せない。
    Remove(LayerSetId),
    /// チャンネルの出力の割り当て（None はそのチャンネルの既定値）。
    Output {
        channel: Channel,
        set: Option<LayerSetId>,
    },
    /// レイヤー（グループは中身ごと）を別のレイヤーセットの一番上へ移す。
    MoveLayers {
        ids: Vec<LayerId>,
        set: LayerSetId,
    },
}

/// 文書を開いた・新しく作ったときに選ぶレイヤー: 最初のレイヤーセットの一番上のレイヤー。
pub(crate) fn default_selection(doc: &Document) -> Option<LayerId> {
    let first = doc.layer_sets().first()?.id;
    top_layer_of(doc, first)
}

/// レイヤーセットの一番上のレイヤー（一番上の段の、並びの最後。グループは中身の上にある）。空なら None。
pub fn top_layer_of(doc: &Document, set: LayerSetId) -> Option<LayerId> {
    doc.layers()
        .iter()
        .rev()
        .find(|l| l.layer_set() == set)
        .map(|l| l.id())
}

/// 画面に出すレイヤーセットの名前（`index` は一覧の番号）。名前が空のレイヤーセット（新しい文書・前のバージョンの文書の最初のレイヤーセット）は、
/// 名前が空のものの最初なら「マテリアル」（並べ替えても同じ名前）、ほかは「レイヤーセット N」。
pub fn display_name(lang: Lang, doc: &Document, index: usize) -> String {
    let sets = doc.layer_sets();
    match sets.get(index) {
        Some(info) if !info.name.is_empty() => info.name.clone(),
        Some(_) if sets.iter().position(|s| s.name.is_empty()) == Some(index) => {
            lang.t("layersets-material").to_owned()
        }
        Some(_) => format!("{} {}", lang.t("layersets-layer-set"), index + 1),
        None => String::new(),
    }
}

/// レイヤーセットの名前（ID から。無ければ空）。
pub fn set_label(lang: Lang, doc: &Document, set: LayerSetId) -> String {
    doc.layer_sets()
        .iter()
        .position(|s| s.id == set)
        .map(|i| display_name(lang, doc, i))
        .unwrap_or_default()
}

/// レイヤーセットが最終の結果に出しているチャンネル（文書のチャンネルの並び）。
pub fn outputs(doc: &Document, set: LayerSetId) -> Vec<Channel> {
    doc.channels()
        .into_iter()
        .filter(|c| doc.channel_output(*c) == Ok(Some(set)))
        .collect()
}

/// 今のレイヤーセットで、描くチャンネルの最終の結果が出ていないか（レイヤーのパネルの頭と、テクスチャセットのパネルの行の印）。
pub fn is_off_output(doc: &Document, set: LayerSetId, channel: Channel) -> bool {
    doc.channel_output(channel) != Ok(Some(set))
}
