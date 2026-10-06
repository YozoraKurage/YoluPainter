//! 文書の中の対象（層・チャンネル・効果・合成モード）を、命令の文字列から引く。名前が複数に当たるときは断る（IDで指す）。

use serde_json::json;
use yolu_core::effects::FilterId;
use yolu_core::{BlendMode, Channel, Document, LayerId, LayerKind};

use crate::error::{ErrorCode, Noun, OpError};

/// 32 桁の 16 進（層・効果の ID）から数へ。
pub fn parse_id(text: &str) -> Option<u128> {
    if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        u128::from_str_radix(text, 16).ok().filter(|v| *v != 0)
    } else {
        None
    }
}

/// 層を ID か名前で引く。ID が先。名前が複数の層に当たれば `ambiguous`（候補の ID を `data` に）。
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

/// 層の名前の検査（1〜256 文字・制御文字なし。保存の上限に収める）。
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
        // 32 桁の 16 進でも、その ID の層が無ければ名前として探す
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
