//! 文書の中の対象（レイヤー・チャンネル・効果・合成モード）を、命令の文字列から引く。名前が複数に当たるときは断る（IDで指す）。
//!
//! 相対の指し方（ID を持たない相手。記録したアクションが別の文書でも同じ相手を指せるように）:
//! - `$selected`: レイヤーの欄だけ。実行の時に選んでいるレイヤー（起動中のアプリの選んでいるレイヤー。画面なしの .ylp には選んでいたレイヤーが入っていないので断る）。
//!   まとめて当てる実行（アクション・CLI の batch）では、始めた時の 1 つに決める。
//! - `$created:<n>`: 同じ実行（アクション・CLI の batch）の中で n 番目（1 から）に作ったレイヤーか効果（`layer.add`・`effect.add` の順の通し番号）。
//!   1 つだけの命令では断る。レイヤーの欄に効果を、効果の欄にレイヤーを指すと断る。
//!
//! `$` で始まるほかの文字列は名前として引く（`$created:` で始まる物だけは、番号が読めなければ断る）。

use serde_json::json;
use yolu_core::effects::FilterId;
use yolu_core::{BlendMode, Channel, Document, LayerId, LayerKind};

use crate::command::Command;
use crate::error::{ErrorCode, Noun, OpError};
use crate::reply::Reply;

/// 実行の時に選んでいるレイヤー（レイヤーの欄だけ）。
pub const SELECTED: &str = "$selected";
/// 同じ実行の中で作ったレイヤー・効果の番号の前置き（`$created:1` が最初）。
pub const CREATED_PREFIX: &str = "$created:";

/// 相対の指し方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relative {
    Selected,
    /// 1 から。
    Created(usize),
}

/// 相対の指し方を読む（相対でなければ None。`$created:` で始まるのに番号が読めなければ誤り）。
pub fn parse_relative(text: &str) -> Result<Option<Relative>, OpError> {
    if text == SELECTED {
        return Ok(Some(Relative::Selected));
    }
    let Some(number) = text.strip_prefix(CREATED_PREFIX) else {
        return Ok(None);
    };
    number
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1 && number.bytes().all(|b| b.is_ascii_digit()))
        .map(|n| Some(Relative::Created(n)))
        .ok_or_else(|| {
            OpError::invalid_value(
                format!("「{text}」の番号は 1 からの数です"),
                format!("The number in \"{text}\" counts from 1"),
            )
        })
}

/// 命令の中の、相手を指す欄の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefSlot {
    /// `layer`・`above`・`parent`。
    Layer,
    /// `effect`。
    Effect,
}

/// 作った物の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreatedKind {
    Layer,
    Effect,
}

/// 同じ実行の中で作ったレイヤー・効果（作った順。`$created:<n>` が指す）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Created {
    items: Vec<(CreatedKind, String)>,
}

impl Created {
    /// 命令の返事から、作ったレイヤー・効果を覚える（`layer.add` のレイヤー・`effect.add` の効果）。
    pub fn note(&mut self, command: &Command, reply: &Reply) {
        let Reply::Edited(edited) = reply else {
            return;
        };
        let made = match command {
            Command::LayerAdd(_) => edited.layer.clone().map(|id| (CreatedKind::Layer, id)),
            Command::EffectAdd(_) => edited.effect.clone().map(|id| (CreatedKind::Effect, id)),
            _ => None,
        };
        self.items.extend(made);
    }
    /// 覚えた数。
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// n 番目（1 から）を、欄の種類に合わせて引く。
    fn get(&self, n: usize, slot: RefSlot, text: &str) -> Result<String, OpError> {
        let Some((kind, id)) = n.checked_sub(1).and_then(|i| self.items.get(i)) else {
            return Err(OpError::new(
                ErrorCode::NotFound,
                format!(
                    "「{text}」の物はありません（この実行で作ったのは {} 個です）",
                    self.items.len()
                ),
                format!(
                    "Nothing matches \"{text}\" (this run has created {} so far)",
                    self.items.len()
                ),
            )
            .with_data(json!({"name": text, "created": self.items.len()})));
        };
        match (slot, kind) {
            (RefSlot::Layer, CreatedKind::Layer) | (RefSlot::Effect, CreatedKind::Effect) => {
                Ok(id.clone())
            }
            (RefSlot::Layer, CreatedKind::Effect) => Err(OpError::invalid_value(
                format!("「{text}」は効果です（レイヤーの欄には使えません）"),
                format!("\"{text}\" is an effect, not a layer"),
            )),
            (RefSlot::Effect, CreatedKind::Layer) => Err(OpError::invalid_value(
                format!("「{text}」はレイヤーです（効果の欄には使えません）"),
                format!("\"{text}\" is a layer, not an effect"),
            )),
        }
    }
}

/// 欄の中身を書き換える関数（替えないなら None）。
pub type RefRewrite<'a> = dyn FnMut(RefSlot, &str) -> Result<Option<String>, OpError> + 'a;

/// 命令の中の、レイヤー・効果を指す欄を `f` に渡し、`f` が Some を返した欄を書き換えた命令を作る（変えなければ None）。
pub fn rewrite_refs(command: &Command, f: &mut RefRewrite<'_>) -> Result<Option<Command>, OpError> {
    let mut out = command.clone();
    let mut changed = false;
    let mut fix = |slot: RefSlot, text: &mut String| -> Result<(), OpError> {
        if let Some(new) = f(slot, text)? {
            *text = new;
            changed = true;
        }
        Ok(())
    };
    match &mut out {
        Command::LayerGet(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::LayerAdd(a) => {
            if let Some(above) = &mut a.above {
                fix(RefSlot::Layer, above)?;
            }
        }
        Command::LayerDelete(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::LayerMove(a) => {
            fix(RefSlot::Layer, &mut a.layer)?;
            if let Some(parent) = &mut a.parent {
                fix(RefSlot::Layer, parent)?;
            }
        }
        Command::LayerSet(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::MaskAdd(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::MaskDelete(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::MaskSet(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::EffectGet(a) => {
            fix(RefSlot::Layer, &mut a.layer)?;
            if let Some(effect) = &mut a.effect {
                fix(RefSlot::Effect, effect)?;
            }
        }
        Command::EffectAdd(a) => fix(RefSlot::Layer, &mut a.layer)?,
        Command::EffectSet(a) => {
            fix(RefSlot::Layer, &mut a.layer)?;
            fix(RefSlot::Effect, &mut a.effect)?;
        }
        Command::EffectDelete(a) => {
            fix(RefSlot::Layer, &mut a.layer)?;
            fix(RefSlot::Effect, &mut a.effect)?;
        }
        Command::DocInfo(_)
        | Command::DocOpen(_)
        | Command::SetInfo(_)
        | Command::EffectListKinds(_)
        | Command::HistoryInfo(_)
        | Command::Undo(_)
        | Command::Redo(_)
        | Command::Preview(_)
        | Command::ExportChannels(_)
        | Command::ExportTextures(_)
        | Command::ExportPsd(_)
        | Command::Save(_)
        | Command::SaveAs(_) => {}
        // 列の中の命令は、列を当てる所（`action::run`）が 1 つずつ引く
        Command::ActionRun(_) => {}
    }
    Ok(changed.then_some(out))
}

/// 命令が `$selected` を使うか。
pub fn uses_selected(command: &Command) -> bool {
    let mut found = false;
    let _ = rewrite_refs(command, &mut |_, text| {
        found |= text == SELECTED;
        Ok(None)
    });
    found
}

/// `$created:<n>` だけを、作った物の ID に替える（`$selected` は残す。起動中のアプリへ 1 つずつ送る CLI の batch が、送る前に使う）。
pub fn substitute_created(command: &Command, created: &Created) -> Result<Command, OpError> {
    let rewritten = rewrite_refs(command, &mut |slot, text| match parse_relative(text)? {
        Some(Relative::Created(n)) => created.get(n, slot, text).map(Some),
        _ => Ok(None),
    })?;
    Ok(rewritten.unwrap_or_else(|| command.clone()))
}

/// 相対の指し方を全部、ID に替える（替える物が無ければ None）。`created` が None なら 1 つだけの命令（`$created` は断る）。
/// `selected` は `$selected` が要るときだけ呼ぶ（選んでいるレイヤーの ID。無ければ理由つきの誤り）。
pub fn resolve_relative(
    command: &Command,
    created: Option<&Created>,
    selected: &mut dyn FnMut() -> Result<String, OpError>,
) -> Result<Option<Command>, OpError> {
    let mut chosen: Option<String> = None;
    rewrite_refs(command, &mut |slot, text| {
        match parse_relative(text)? {
        None => Ok(None),
        Some(Relative::Selected) => {
            if slot != RefSlot::Layer {
                return Err(OpError::invalid_value(
                    "$selected はレイヤーの欄だけで使えます",
                    "$selected can only be used for a layer",
                ));
            }
            if chosen.is_none() {
                chosen = Some(selected()?);
            }
            Ok(chosen.clone())
        }
        Some(Relative::Created(n)) => match created {
            Some(created) => created.get(n, slot, text).map(Some),
            None => Err(OpError::invalid_request(
                format!("「{text}」は、まとめて当てる実行（アクション・batch）の中だけで使えます"),
                format!("\"{text}\" can only be used inside a run of several commands (an action or a batch)"),
            )),
        },
    }
    })
}

/// 選んでいるレイヤーが無いときの誤り（`why` は、無い理由。日英）。
pub fn no_selection(why: Option<(&str, &str)>) -> OpError {
    let (ja, en) = match why {
        Some((ja, en)) => (
            format!("選んでいるレイヤーがありません（{ja}）"),
            format!("No layer is selected ({en})"),
        ),
        None => (
            "選んでいるレイヤーがありません".to_owned(),
            "No layer is selected".to_owned(),
        ),
    };
    OpError::new(ErrorCode::NotFound, ja, en).with_data(json!({"name": SELECTED}))
}

/// 32 桁の 16 進（レイヤー・効果の ID）から数へ。
pub fn parse_id(text: &str) -> Option<u128> {
    if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        u128::from_str_radix(text, 16).ok().filter(|v| *v != 0)
    } else {
        None
    }
}

/// レイヤーを ID か名前で引く。ID が先。名前が複数のレイヤーに当たれば `ambiguous`（候補の ID を `data` に）。
pub fn resolve_layer(doc: &Document, text: &str) -> Result<LayerId, OpError> {
    if let Some(id) = parse_id(text) {
        let id = LayerId(id);
        if doc.layer(id).is_some() {
            return Ok(id);
        }
    }
    let found: Vec<_> = doc.layers().iter().filter(|l| l.name() == text).collect();
    match found.as_slice() {
        [] => Err(OpError::not_found(Noun::Layer, text)),
        [one] => Ok(one.id()),
        many => Err(OpError::new(
            ErrorCode::Ambiguous,
            format!("レイヤー名「{text}」が {} 枚に当たります。ID で指してください", many.len()),
            format!("{} layers are named \"{text}\"; use the layer id", many.len()),
        )
        .with_data(json!({
            "candidates": many.iter().map(|l| json!({"id": l.id().to_string(), "kind": kind_word(l.kind())})).collect::<Vec<_>>()
        }))),
    }
}

/// テクスチャセットを ID か名前で引く（`sets` は（ID, 名前）の並び。省略は `current`（セットの ID）のセット、無ければ先頭）。ID が先。
/// 名前が複数に当たれば `ambiguous`（候補の ID と名前を `data` に）、無ければ `not_found`（あるセットの一覧を `data` に）。
/// 画面なしのホストと起動中のアプリのホストが、同じ指し方・同じ誤りになるように 1 か所に置く。
pub fn resolve_set(
    sets: &[(&str, &str)],
    set: Option<&str>,
    current: &str,
) -> Result<usize, OpError> {
    let Some(text) = set else {
        return sets
            .iter()
            .position(|(id, _)| *id == current)
            .or(if sets.is_empty() { None } else { Some(0) })
            .ok_or_else(|| OpError::not_found(Noun::Set, current));
    };
    if let Some(i) = sets.iter().position(|(id, _)| *id == text) {
        return Ok(i);
    }
    let named: Vec<usize> = sets
        .iter()
        .enumerate()
        .filter(|(_, (_, name))| *name == text)
        .map(|(i, _)| i)
        .collect();
    match named.as_slice() {
        [] => Err(OpError::not_found(Noun::Set, text)
            .with_data(json!({"sets": sets.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()}))),
        [one] => Ok(*one),
        many => Err(OpError::new(
            ErrorCode::Ambiguous,
            format!("セット名「{text}」が {} 個に当たります。ID で指してください", many.len()),
            format!("{} texture sets are named \"{text}\"; use the set id", many.len()),
        )
        .with_data(json!({"candidates": many.iter().map(|i| json!({"id": sets[*i].0, "name": sets[*i].1})).collect::<Vec<_>>()}))),
    }
}

pub fn kind_word(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Raster => "paint",
        LayerKind::Fill => "fill",
        LayerKind::Adjustment => "adjustment",
        LayerKind::Group => "group",
    }
}

/// チャンネルの名前（標準の名前か、文書のユーザーチャンネルの名前）。
pub fn channel_name(doc: &Document, channel: Channel) -> String {
    doc.channel_info(channel)
        .map(|i| i.name.clone())
        .or_else(|| channel.standard_name().map(str::to_owned))
        .unwrap_or_else(|| channel.index().to_string())
}

/// チャンネルを名前（大文字小文字を問わない）か番号で引く。文書にあるチャンネルだけ。
pub fn resolve_channel(doc: &Document, text: &str) -> Result<Channel, OpError> {
    let channels = doc.channels();
    if let Some(c) = channels
        .iter()
        .copied()
        .find(|c| channel_name(doc, *c) == text)
    {
        return Ok(c);
    }
    let lower = text.to_lowercase();
    let same: Vec<Channel> = channels
        .iter()
        .copied()
        .filter(|c| channel_name(doc, *c).to_lowercase() == lower)
        .collect();
    match same.as_slice() {
        [one] => return Ok(*one),
        [] => {}
        _ => {
            return Err(OpError::new(
                ErrorCode::Ambiguous,
                format!("チャンネル名「{text}」が複数に当たります。番号で指してください"),
                format!("Several channels are named \"{text}\"; use the channel number"),
            )
            .with_data(json!({
                "candidates": same.iter().map(|c| json!({"index": c.index(), "name": channel_name(doc, *c)})).collect::<Vec<_>>()
            })))
        }
    }
    if let Ok(index) = text.parse::<usize>() {
        if let Some(c) = Channel::from_index(index).filter(|c| doc.channel_info(*c).is_some()) {
            return Ok(c);
        }
    }
    Err(OpError::not_found(Noun::Channel, text).with_data(
        json!({"channels": channels.iter().map(|c| channel_name(doc, *c)).collect::<Vec<_>>()}),
    ))
}

/// 合成モードを名前から。`Multiply`・`multiply`・`color_dodge` のどれでもよい。
pub fn parse_blend(text: &str) -> Result<BlendMode, OpError> {
    let key = |s: &str| {
        s.chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .collect::<String>()
            .to_lowercase()
    };
    let want = key(text);
    BlendMode::LAYER_MODES
        .iter()
        .copied()
        .chain(std::iter::once(BlendMode::PassThrough))
        .find(|m| key(m.name()) == want)
        .ok_or_else(|| {
            let names: Vec<&str> = BlendMode::LAYER_MODES.iter().map(|m| m.name()).collect();
            OpError::invalid_value(
                format!("合成モード「{text}」は知りません"),
                format!("Unknown blend mode \"{text}\""),
            )
            .with_data(json!({"blend_modes": names, "group_only": "PassThrough"}))
        })
}

/// 効果の ID。
pub fn parse_filter_id(text: &str) -> Result<FilterId, OpError> {
    parse_id(text).map(FilterId).ok_or_else(|| {
        OpError::invalid_value(
            format!("効果の ID は 32 桁の 16 進です（{text}）"),
            format!("An effect id is 32 hex digits ({text})"),
        )
    })
}

/// レイヤーの名前の検査（1〜256 文字・制御文字なし。保存の上限に収める）。
pub fn check_layer_name(name: &str) -> Result<(), OpError> {
    let chars = name.chars().count();
    if chars == 0 || chars > 256 || name.chars().any(char::is_control) {
        return Err(OpError::invalid_value(
            "レイヤーの名前は 1〜256 文字で、制御文字を含められません",
            "A layer name has 1 to 256 characters and no control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        let mut doc = Document::new(8, 8).unwrap();
        doc.add_layer("a").unwrap();
        doc.add_layer("b").unwrap();
        doc.add_layer("b").unwrap();
        doc
    }

    #[test]
    fn layers_resolve_by_id_then_name_and_ambiguity_lists_candidates() {
        let doc = doc();
        let a = doc.layers()[0].id();
        assert_eq!(resolve_layer(&doc, &a.to_string()).unwrap(), a);
        assert_eq!(resolve_layer(&doc, "a").unwrap(), a);
        let e = resolve_layer(&doc, "b").unwrap_err();
        assert_eq!(e.code, ErrorCode::Ambiguous);
        assert_eq!(e.data.unwrap()["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(
            resolve_layer(&doc, "zzz").unwrap_err().code,
            ErrorCode::NotFound
        );
        // 32 桁の 16 進でも、その ID のレイヤーが無ければ名前として探す
        assert_eq!(
            resolve_layer(&doc, &"1".repeat(32)).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn channels_resolve_by_name_case_insensitive_and_number() {
        let doc = doc();
        assert_eq!(resolve_channel(&doc, "Color").unwrap(), Channel::Color);
        assert_eq!(
            resolve_channel(&doc, "roughness").unwrap(),
            Channel::Roughness
        );
        assert_eq!(resolve_channel(&doc, "4").unwrap(), Channel::Normal);
        let e = resolve_channel(&doc, "Gloss").unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(e.data.unwrap()["channels"].as_array().unwrap().len() >= 6);
        assert_eq!(
            resolve_channel(&doc, "40").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn blend_modes_accept_name_variants() {
        assert_eq!(parse_blend("Multiply").unwrap(), BlendMode::Multiply);
        assert_eq!(parse_blend("color_dodge").unwrap(), BlendMode::ColorDodge);
        assert_eq!(parse_blend("pass through").unwrap(), BlendMode::PassThrough);
        assert_eq!(
            parse_blend("nope").unwrap_err().code,
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn names_and_ids_are_checked() {
        assert!(check_layer_name("ok").is_ok());
        assert!(check_layer_name("").is_err());
        assert!(check_layer_name("a\nb").is_err());
        assert!(check_layer_name(&"x".repeat(257)).is_err());
        assert!(parse_id(&"a".repeat(32)).is_some());
        assert!(parse_id(&"0".repeat(32)).is_none());
        assert!(parse_id("xyz").is_none());
        assert!(parse_filter_id("xyz").is_err());
    }
}
