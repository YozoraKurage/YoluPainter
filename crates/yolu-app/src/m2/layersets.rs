//! レイヤーセット（core の `document/layer_sets.rs`）の、状態を持たない部分: 操作の種類（[`LayerSetEdit`]）・名前・出力の読み・一番上のレイヤー。
//! 今のレイヤーセットと操作を当てる口は `sets.rs`（`AppState`）、メニューの項目は `m2_menu.rs`。

use crate::engine::{Channel, ChannelInfo, CoreError, Document, LayerId, LayerSetId};
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
    /// レイヤーセットの扱うチャンネルの組に、チャンネルを 1 つ入れる・外す（入っていれば外す）。外したチャンネルの出力の割り当てがこの
    /// レイヤーセットなら、割り当ても外れる（最終の結果はそのチャンネルの既定値）。
    ToggleChannel {
        set: LayerSetId,
        channel: Channel,
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

/// 新しいユーザーチャンネルを、そのチャンネルの名前のレイヤーセット（組はそのチャンネルだけ・出力の割り当てもそこ・そのチャンネルを有効にした
/// 空のレイヤー 1 枚つき）と一緒に足す。取り消しを 1 回にまとめるのは呼ぶ側（`doc.batch` の中で呼ぶ）。レイヤーセットが上限
/// （`yolu_core::MAX_LAYER_SETS`）なら、レイヤーセットを作らずに `Document::add_channel` で足し（番号の出力の割り当て先の組に入る）、
/// レイヤーセットは None を返す（知らせは呼ぶ側）。
pub fn add_channel_in_own_set(
    d: &mut Document,
    info: ChannelInfo,
    lang: Lang,
) -> Result<(Channel, Option<LayerSetId>), CoreError> {
    if d.layer_sets().len() >= yolu_core::MAX_LAYER_SETS {
        return Ok((d.add_channel(info)?, None));
    }
    let set_name = unique_layer_set_name_in(lang, d, &info.name);
    let layer_name = format!("{} 1", lang.t("state-layer"));
    let set = d.add_layer_set_with(&set_name, 0)?;
    let channel = d.add_channel_in_set(info, set)?;
    d.add_layer_in_set_on(&layer_name, set, None, channel)?;
    Ok((channel, Some(set)))
}

/// 描くチャンネルを替えるときの、今のレイヤーセットの扱い（[`set_for_paint_channel`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintSet {
    /// 今のレイヤーセットの組にある（移らない）。
    Here,
    /// このレイヤーセットへ移る。
    Move(LayerSetId),
    /// どのレイヤーセットの組にも無い。
    Nowhere,
}

/// 描くチャンネルを `channel` へ替えるとき、今のレイヤーセット `current` の組に無ければ移る先のレイヤーセット: `channel` の出力の割り当て先、
/// 無ければ組に `channel` を持つ最初のレイヤーセット。
pub fn set_for_paint_channel(doc: &Document, current: LayerSetId, channel: Channel) -> PaintSet {
    let has = |set: LayerSetId| {
        doc.layer_set_channels(set)
            .is_ok_and(|bits| bits & channel.bit() != 0)
    };
    if has(current) {
        return PaintSet::Here;
    }
    doc.channel_output(channel)
        .ok()
        .flatten()
        .filter(|s| has(*s))
        .or_else(|| doc.layer_sets().iter().map(|s| s.id).find(|s| has(*s)))
        .map_or(PaintSet::Nowhere, PaintSet::Move)
}

/// 文書の中のほかのレイヤーセットと名前が重ならない（大文字小文字を区別しない）ように、要れば「 2」「 3」… を付けた名前。
/// `AppState` の外（`doc.batch` の中・取り込み）でも使う。
pub(crate) fn unique_layer_set_name_in(lang: Lang, doc: &Document, base: &str) -> String {
    let names: Vec<String> = (0..doc.layer_sets().len())
        .map(|i| display_name(lang, doc, i))
        .collect();
    unique_name(base, names.iter().map(String::as_str))
}

/// 名前が大文字小文字を区別せずにほかと重ならないように、要れば「 2」「 3」… を付ける（.ylp の決まり。書き出すファイルの名前に使う）。
pub fn unique_name<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    // yolu-io の検証と同じく大文字にそろえて比べる
    let free = |n: &str| !taken.clone().any(|t| t.to_uppercase() == n.to_uppercase());
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| free(n))
        .expect("どこかで空く")
}
