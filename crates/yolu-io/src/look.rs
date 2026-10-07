//! テクスチャセットの見た目の設定（`sets/<ID>/look.json`）: 3D ビューの描き方（標準の PBR か lilToon の再現か）と、lilToon のときの
//! マテリアルの値（[`yolu_core::look::MaterialLook`]）。
//!
//! 正本（`document.utpaint`）の画素とは別のエントリで、正本の版も .ylp の形式も変えない。Unity 版 0.2.0 の読み手は、知らないエントリとして
//! 開くときに一覧で知らせ（「保存すると残らない」）、Unity 版で保存するとこのエントリは落ちる（画素・層・チャンネルは失わない。
//! 失うのは見た目の設定だけで、標準の描き方に戻る）。
//!
//! 形（UTF-8 の JSON のオブジェクト、64 KiB の中ではなく 1 MiB まで。プロパティの数が多いため）:
//!
//! ```json
//! {
//!   "format": 1,
//!   "kind": "lilToon",
//!   "shader": "Hidden/lilToonCutoutOutline",
//!   "properties": {
//!     "_ShadowBorder": { "float": 0.5 },
//!     "_UseShadow": { "int": 1 },
//!     "_ShadowColor": { "color": [0.82, 0.76, 0.85, 1.0] },
//!     "_MainTex_ST": { "vector": [1.0, 1.0, 0.0, 0.0] }
//!   },
//!   "textures": {
//!     "_MainTex": { "channel": 0 },
//!     "_ShadowStrengthMask": { "packed": [ { "channel": 7, "component": 0 }, { "channel": 7, "component": 0 }, "zero", "one" ] },
//!     "_MatCapTex": { "image": "<32 桁の 16 進>" }
//!   },
//!   "keywords": [],
//!   "kindChosen": true,
//!   "received": {
//!     "kind": "lilToon",
//!     "shader": "Hidden/lilToonTransparent",
//!     "shaderGuid": "<32 桁の 16 進>",
//!     "shaderVersion": "2.3.4",
//!     "renderQueue": 3000,
//!     "source": "lilToon 2.3.4",
//!     "properties": { "_ShadowBorder": { "float": 0.3 } },
//!     "textures": { "_MainTex": { "channel": 0 } },
//!     "keywords": [],
//!     "missing": { "_MatCapTex": "notAFile", "_ShadowColorTex": "overBudget" }
//!   }
//! }
//! ```
//!
//! - 根の本体（`kind`〜`keywords`・`shaderGuid`・`shaderVersion`・`renderQueue`）は利用者の設定。`kindChosen` は利用者が欄で描き方を
//!   選んだか（無ければ偽）。`shaderGuid`・`shaderVersion`（Unity のシェーダーの身元）と `renderQueue`（描画の順）は無ければ不明・既定。
//! - `received` は Live Link で Unity のマテリアルから受けた値（`yolu_core::look::ReceivedLook`。本体と同じ形と、出どころの文 `source`、
//!   絵の無いスロットの理由 `missing`）。描くときは受けた値の上に利用者の設定を重ねる。受けた絵の画素は書かない（絵はファイルから
//!   読み直す）。絵のあったスロットは `missing` に書かない。理由は `overBudget`・`unreadable`・`notAFile`（Unity の中にしかない絵）。
//!   0.4 までの書き手の `pending` は `unreadable` として読む。
//!   利用者の設定と受けた見た目は別々に書き換える（[`write`] は `received` を前のエントリのまま、[`write_received`] は本体を前のまま）。
//! - `format` は 1。2 以上は読まずに断る（エントリはバイト列のまま残る）。
//! - `kind` は `standard`・`lilToon`。知らない値は断る。
//! - `channel` は文書のチャンネルの番号（標準 0〜5・ユーザーチャンネル 6〜63。正本の版 22 のチャンネルの一覧の番号）。文書に無い番号は
//!   読み手が割り当てなしとして扱う（ここでは断らない）。
//! - 色はガンマの空間（Unity のマテリアルの値と同じ）。
//! - 知らないキーは読み飛ばし、書き直すときは前のエントリの知らないキーを残す（同じ形式の中で足したキーを、古い書き手が落とさない）。
//!   前のエントリが同じ形式（`format` が 1）でなければ（新しい形式・壊れている）、何も残さずに書く（新しい形式のキーと形式 1 の
//!   本体が混ざったエントリを作らない。読めないエントリを上書きするかどうかは呼び手が決めて、利用者に知らせる）。

use serde_json::{Map, Value};
use yolu_core::look::{
    LookKind, LookValue, MaterialLook, MissingImage, PlaneSource, ReceivedLook, TextureSource,
};
use yolu_core::{Channel, ImageId};

use crate::{check, check_budget, Error, Result};

/// エントリの名前（セットの下）。
pub const ENTRY: &str = "look.json";
/// 形式の版。
pub const FORMAT: i64 = 1;
/// エントリの大きさの上限。
pub const MAX_BYTES: usize = 1 << 20;

/// 読む（形の違い・新しい版・知らない種類・範囲の外は断る）。利用者の設定だけ（`received` は [`read_received`]）。
pub fn read(bytes: &[u8]) -> Result<MaterialLook> {
    let obj = root_object(bytes)?;
    let mut look = read_body(&obj, "look.json")?;
    look.kind_chosen = match obj.get("kindChosen") {
        None | Some(Value::Null) => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| invalid("look.json の kindChosen が真偽ではありません"))?,
    };
    look.validate()
        .map_err(|e| invalid(format!("look.json の値が範囲外です: {e}")))?;
    Ok(look)
}

/// 受けた見た目（`received`。Live Link で Unity のマテリアルから受けた値）を読む。無ければ None。絵の画素は保存しないので、
/// 絵のあったスロットは絵も理由も無いまま戻る（絵は Live Link の相手の絵のファイルから読み直す）。
pub fn read_received(bytes: &[u8]) -> Result<Option<ReceivedLook>> {
    let obj = root_object(bytes)?;
    let Some(received) = obj.get("received").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let r = received
        .as_object()
        .ok_or_else(|| invalid("look.json の received がオブジェクトではありません"))?;
    let look = read_body(r, "look.json の received")?;
    let source = match r.get("source") {
        None | Some(Value::Null) => String::new(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| invalid("look.json の received.source が文字列ではありません"))?
            .to_owned(),
    };
    let mut missing = std::collections::BTreeMap::new();
    if let Some(m) = r.get("missing") {
        let m = m
            .as_object()
            .ok_or_else(|| invalid("look.json の received.missing がオブジェクトではありません"))?;
        check_budget(
            m.len() <= yolu_core::look::MAX_TEXTURES,
            "look.json の received.missing の数が上限を超えています",
        )?;
        for (slot, why) in m {
            let why = why
                .as_str()
                .and_then(MissingImage::from_key)
                .ok_or_else(|| {
                    invalid(format!(
                        "look.json の received.missing の {slot} が違います"
                    ))
                })?;
            missing.insert(slot.clone(), why);
        }
    }
    let received = ReceivedLook {
        look,
        source,
        images: Default::default(),
        missing,
    };
    received
        .validate()
        .map_err(|e| invalid(format!("look.json の received が範囲外です: {e}")))?;
    Ok(Some(received))
}

/// 中身のオブジェクト（予算・JSON・形式の版を確かめる）。
fn root_object(bytes: &[u8]) -> Result<Map<String, Value>> {
    check_budget(bytes.len() <= MAX_BYTES, "look.json のバイト予算超過です")?;
    let root: Value = serde_json::from_slice(bytes)?;
    let Value::Object(obj) = root else {
        return Err(invalid("look.json がオブジェクトではありません"));
    };
    let format = obj
        .get("format")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("look.json の format が整数ではありません"))?;
    check(format >= 1, "look.json の format が範囲外です")?;
    if format > FORMAT {
        return Err(Error::InvalidData(format!(
            "look.json の形式 {format} はこの版では読めません（{FORMAT} まで）"
        )));
    }
    Ok(obj)
}

/// 見た目の本体（kind・shader・properties・textures・keywords）を読む。`at` は誤りの文に出す場所。
fn read_body(obj: &Map<String, Value>, at: &str) -> Result<MaterialLook> {
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{at} の kind が文字列ではありません")))?;
    let kind = LookKind::from_key(kind)
        .ok_or_else(|| invalid(format!("{at} の知らない kind です: {kind}")))?;
    let shader = match obj.get("shader") {
        None | Some(Value::Null) => String::new(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| invalid(format!("{at} の shader が文字列ではありません")))?
            .to_owned(),
    };
    let text = |key: &str| -> Result<String> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(v) => Ok(v
                .as_str()
                .ok_or_else(|| invalid(format!("{at} の {key} が文字列ではありません")))?
                .to_owned()),
        }
    };
    let render_queue = match obj.get("renderQueue") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_i64()
                .and_then(|q| i32::try_from(q).ok())
                .ok_or_else(|| invalid(format!("{at} の renderQueue が整数ではありません")))?,
        ),
    };
    let mut look = MaterialLook {
        kind,
        shader,
        shader_guid: text("shaderGuid")?,
        shader_version: text("shaderVersion")?,
        render_queue,
        ..MaterialLook::default()
    };
    if let Some(props) = obj.get("properties") {
        let props = props
            .as_object()
            .ok_or_else(|| invalid(format!("{at} の properties がオブジェクトではありません")))?;
        check_budget(
            props.len() <= yolu_core::look::MAX_PROPERTIES,
            "look.json のプロパティの数が上限を超えています",
        )?;
        for (name, v) in props {
            look.properties.insert(name.clone(), value(name, v)?);
        }
    }
    if let Some(textures) = obj.get("textures") {
        let textures = textures
            .as_object()
            .ok_or_else(|| invalid(format!("{at} の textures がオブジェクトではありません")))?;
        check_budget(
            textures.len() <= yolu_core::look::MAX_TEXTURES,
            "look.json のテクスチャの数が上限を超えています",
        )?;
        for (name, v) in textures {
            look.textures.insert(name.clone(), texture(name, v)?);
        }
    }
    if let Some(keywords) = obj.get("keywords") {
        let keywords = keywords
            .as_array()
            .ok_or_else(|| invalid(format!("{at} の keywords が配列ではありません")))?;
        check_budget(
            keywords.len() <= yolu_core::look::MAX_KEYWORDS,
            "look.json のキーワードの数が上限を超えています",
        )?;
        for k in keywords {
            look.keywords.push(
                k.as_str()
                    .ok_or_else(|| invalid(format!("{at} のキーワードが文字列ではありません")))?
                    .to_owned(),
            );
        }
    }
    look.validate()
        .map_err(|e| invalid(format!("{at} の値が範囲外です: {e}")))?;
    Ok(look)
}

/// 前のエントリの、同じ形式のオブジェクト（知らないキーを残す元）。ほかの形式・壊れたものは空。
fn previous_object(previous: Option<&[u8]>) -> Map<String, Value> {
    previous
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
        .and_then(|v| match v {
            Value::Object(m) if m.get("format").and_then(Value::as_i64) == Some(FORMAT) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

/// 見た目の本体を書く（`kind`・`shader`・`properties`・`textures`・`keywords`）。
fn write_body(root: &mut Map<String, Value>, look: &MaterialLook) {
    root.insert("kind".into(), Value::from(look.kind.key()));
    if look.shader.is_empty() {
        root.remove("shader");
    } else {
        root.insert("shader".into(), Value::from(look.shader.as_str()));
    }
    let props: Map<String, Value> = look
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), value_json(v)))
        .collect();
    root.insert("properties".into(), Value::Object(props));
    let textures: Map<String, Value> = look
        .textures
        .iter()
        .map(|(k, t)| (k.clone(), texture_json(t)))
        .collect();
    root.insert("textures".into(), Value::Object(textures));
    root.insert(
        "keywords".into(),
        Value::Array(
            look.keywords
                .iter()
                .map(|k| Value::from(k.as_str()))
                .collect(),
        ),
    );
    for (key, text) in [
        ("shaderGuid", &look.shader_guid),
        ("shaderVersion", &look.shader_version),
    ] {
        if text.is_empty() {
            root.remove(key);
        } else {
            root.insert(key.into(), Value::from(text.as_str()));
        }
    }
    match look.render_queue {
        Some(q) => {
            root.insert("renderQueue".into(), Value::from(q));
        }
        None => {
            root.remove("renderQueue");
        }
    }
}

fn finish(root: Map<String, Value>) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))?;
    check_budget(bytes.len() <= MAX_BYTES, "look.json のバイト予算超過です")?;
    Ok(bytes)
}

/// 書く（利用者の設定）。`previous` は同じエントリの前のバイト列（同じ形式のオブジェクトなら、知らないキーと受けた見た目
/// （`received`）を残す。ほかは残さない）。
pub fn write(look: &MaterialLook, previous: Option<&[u8]>) -> Result<Vec<u8>> {
    look.validate()
        .map_err(|e| invalid(format!("見た目の設定が範囲外です: {e}")))?;
    let mut root = previous_object(previous);
    root.insert("format".into(), Value::from(FORMAT));
    write_body(&mut root, look);
    if look.kind_chosen {
        root.insert("kindChosen".into(), Value::from(true));
    } else {
        root.remove("kindChosen");
    }
    finish(root)
}

/// 受けた見た目（`received`）だけを置き換えて書く（None は外す）。利用者の設定と知らないキーは前のエントリのまま（前のエントリが無ければ、
/// 利用者の設定は既定（標準）で書く。読めない前のエントリには書かずに断る）。絵の画素は書かず、絵のあったスロットは `missing` の `pending` にする。
/// 利用者の設定が既定で受けた見た目も無いなら None（エントリを消す）。
pub fn write_received(
    received: Option<&ReceivedLook>,
    previous: Option<&[u8]>,
) -> Result<Option<Vec<u8>>> {
    // 読めない前のエントリ（新しい形式・壊れた）の上には書かない（利用者の設定を黙って消さない）
    if let Some(b) = previous {
        root_object(b)?;
    }
    let mut root = previous_object(previous);
    // 読めない本体は既定と見なさない（受けた見た目を外しても、エントリごと消さない）
    let user_default = root.is_empty()
        || read(&serde_json::to_vec(&Value::Object(root.clone()))?)
            .map(|l| l.is_default())
            .unwrap_or(false);
    if root.is_empty() {
        root.insert("format".into(), Value::from(FORMAT));
        write_body(&mut root, &MaterialLook::default());
    }
    match received {
        None => {
            root.remove("received");
            if user_default && root.keys().all(|k| BODY_KEYS.contains(&k.as_str())) {
                return Ok(None);
            }
        }
        Some(r) => {
            r.validate()
                .map_err(|e| invalid(format!("受けた見た目が範囲外です: {e}")))?;
            let mut body = Map::new();
            write_body(&mut body, &r.look);
            if !r.source.is_empty() {
                body.insert("source".into(), Value::from(r.source.as_str()));
            }
            // 絵のあったスロットは書かない（絵はファイルから読み直す）
            let missing: Map<String, Value> = r
                .missing
                .iter()
                .filter(|(k, _)| !r.images.contains_key(*k))
                .map(|(k, why)| (k.clone(), Value::from(why.key())))
                .collect();
            if !missing.is_empty() {
                body.insert("missing".into(), Value::Object(missing));
            }
            root.insert("received".into(), Value::Object(body));
        }
    }
    finish(root).map(Some)
}

/// 利用者の設定の本体のキー（これだけのエントリは、既定なら消してよい）。
const BODY_KEYS: [&str; 10] = [
    "format",
    "kind",
    "shader",
    "properties",
    "textures",
    "keywords",
    "kindChosen",
    "shaderGuid",
    "shaderVersion",
    "renderQueue",
];

/// 前のエントリに受けた見た目（`received`）があるか（読めなくても、キーがあれば true）。
pub fn has_received(previous: &[u8]) -> bool {
    serde_json::from_slice::<Value>(previous)
        .ok()
        .and_then(|v| v.get("received").map(|r| !r.is_null()))
        .unwrap_or(false)
}

fn invalid(why: impl Into<String>) -> Error {
    Error::InvalidData(why.into())
}

fn number4(name: &str, v: &Value) -> Result<[f32; 4]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 4)
        .ok_or_else(|| invalid(format!("look.json の {name} が 4 つの数ではありません")))?;
    let mut out = [0.0f32; 4];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x
            .as_f64()
            .ok_or_else(|| invalid(format!("look.json の {name} が数ではありません")))?
            as f32;
    }
    Ok(out)
}

fn value(name: &str, v: &Value) -> Result<LookValue> {
    let o = v
        .as_object()
        .filter(|o| o.len() == 1)
        .ok_or_else(|| invalid(format!("look.json のプロパティ {name} の形が違います")))?;
    let (ty, x) = o.iter().next().expect("1 つ");
    Ok(match ty.as_str() {
        "float" => LookValue::Float(
            x.as_f64()
                .ok_or_else(|| invalid(format!("look.json の {name} が数ではありません")))?
                as f32,
        ),
        "int" => LookValue::Int(
            x.as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or_else(|| invalid(format!("look.json の {name} が整数ではありません")))?,
        ),
        "color" => LookValue::Color(number4(name, x)?),
        "vector" => LookValue::Vector(number4(name, x)?),
        other => {
            return Err(invalid(format!(
                "look.json のプロパティ {name} の知らない型です: {other}"
            )))
        }
    })
}

fn value_json(v: &LookValue) -> Value {
    let four = |a: &[f32; 4]| Value::Array(a.iter().map(|x| Value::from(*x as f64)).collect());
    match v {
        LookValue::Float(x) => serde_json::json!({ "float": *x as f64 }),
        LookValue::Int(x) => serde_json::json!({ "int": x }),
        LookValue::Color(a) => serde_json::json!({ "color": four(a) }),
        LookValue::Vector(a) => serde_json::json!({ "vector": four(a) }),
    }
}

fn channel(name: &str, v: &Value) -> Result<Channel> {
    v.as_u64()
        .and_then(|i| Channel::from_index(i as usize).filter(|_| i < 64))
        .ok_or_else(|| {
            invalid(format!(
                "look.json の {name} のチャンネルの番号が範囲外です"
            ))
        })
}

fn plane(name: &str, v: &Value) -> Result<PlaneSource> {
    match v {
        Value::String(s) if s == "zero" => Ok(PlaneSource::Zero),
        Value::String(s) if s == "one" => Ok(PlaneSource::One),
        Value::Object(o) => {
            let c = channel(name, o.get("channel").unwrap_or(&Value::Null))?;
            let component = o
                .get("component")
                .and_then(Value::as_u64)
                .filter(|c| *c <= 3)
                .ok_or_else(|| invalid(format!("look.json の {name} の成分が範囲外です")))?;
            Ok(PlaneSource::Channel {
                channel: c,
                component: component as u8,
            })
        }
        _ => Err(invalid(format!("look.json の {name} の成分の形が違います"))),
    }
}

fn texture(name: &str, v: &Value) -> Result<TextureSource> {
    let o = v
        .as_object()
        .ok_or_else(|| invalid(format!("look.json のテクスチャ {name} の形が違います")))?;
    if let Some(c) = o.get("channel") {
        return Ok(TextureSource::Channel(channel(name, c)?));
    }
    if let Some(p) = o.get("packed") {
        let a = p.as_array().filter(|a| a.len() == 4).ok_or_else(|| {
            invalid(format!(
                "look.json の {name} の packed が 4 つではありません"
            ))
        })?;
        let mut planes = [PlaneSource::Zero; 4];
        for (out, x) in planes.iter_mut().zip(a) {
            *out = plane(name, x)?;
        }
        return Ok(TextureSource::Packed(planes));
    }
    if let Some(i) = o.get("image") {
        let s = i
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| invalid(format!("look.json の {name} の画像の ID が違います")))?;
        let id = u128::from_str_radix(s, 16)
            .ok()
            .filter(|id| *id != 0)
            .ok_or_else(|| invalid(format!("look.json の {name} の画像の ID が違います")))?;
        return Ok(TextureSource::Image(ImageId(id)));
    }
    Err(invalid(format!(
        "look.json のテクスチャ {name} の知らない入力です"
    )))
}

fn texture_json(t: &TextureSource) -> Value {
    let plane = |p: &PlaneSource| match p {
        PlaneSource::Zero => Value::from("zero"),
        PlaneSource::One => Value::from("one"),
        PlaneSource::Channel { channel, component } => {
            serde_json::json!({ "channel": channel.index(), "component": component })
        }
    };
    match t {
        TextureSource::Channel(c) => serde_json::json!({ "channel": c.index() }),
        TextureSource::Packed(planes) => {
            serde_json::json!({ "packed": planes.iter().map(plane).collect::<Vec<_>>() })
        }
        TextureSource::Image(id) => serde_json::json!({ "image": format!("{:032x}", id.0) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MaterialLook {
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            shader: "Hidden/lilToonCutoutOutline".into(),
            ..MaterialLook::default()
        };
        look.properties
            .insert("_ShadowBorder".into(), LookValue::Float(0.375));
        look.properties
            .insert("_UseShadow".into(), LookValue::Int(1));
        look.properties.insert(
            "_ShadowColor".into(),
            LookValue::Color([0.82, 0.76, 0.85, 1.0]),
        );
        look.properties.insert(
            "_MainTex_ST".into(),
            LookValue::Vector([1.0, 2.0, 0.0, 0.5]),
        );
        look.properties
            .insert("_SomeFutureProperty".into(), LookValue::Float(-3.5));
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        look.textures.insert(
            "_ShadowStrengthMask".into(),
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: Channel::from_index(7).unwrap(),
                    component: 0,
                },
                PlaneSource::Channel {
                    channel: Channel::from_index(8).unwrap(),
                    component: 3,
                },
                PlaneSource::Zero,
                PlaneSource::One,
            ]),
        );
        look.textures.insert(
            "_MatCapTex".into(),
            TextureSource::Image(ImageId(0x0123_4567_89ab_cdef_0011_2233_4455_6677)),
        );
        look.keywords = vec!["_EMISSION".into()];
        look
    }

    #[test]
    fn round_trip_keeps_every_value() {
        let look = sample();
        let bytes = write(&look, None).unwrap();
        assert_eq!(read(&bytes).unwrap(), look);
        // 同じ設定は同じバイト列（名前の順）
        assert_eq!(write(&look, None).unwrap(), bytes);
        let standard = MaterialLook::default();
        assert_eq!(read(&write(&standard, None).unwrap()).unwrap(), standard);
    }

    #[test]
    fn the_kind_choice_round_trips_and_a_rewrite_keeps_the_received_values() {
        let mut look = sample();
        look.kind_chosen = true;
        let bytes = write(&look, None).unwrap();
        assert_eq!(read(&bytes).unwrap(), look);
        let mut received = ReceivedLook {
            look: sample(),
            ..ReceivedLook::default()
        };
        received.look.kind_chosen = false;
        let with = write_received(Some(&received), Some(&bytes))
            .unwrap()
            .unwrap();
        assert_eq!(read(&with).unwrap(), look, "利用者の設定はそのまま");
        // 利用者の設定を書き直しても受けた見た目は残る
        let mut changed = look.clone();
        changed.kind_chosen = false;
        let rewritten = write(&changed, Some(&with)).unwrap();
        assert_eq!(read(&rewritten).unwrap(), changed);
        assert_eq!(read_received(&rewritten).unwrap(), Some(received));
        assert!(has_received(&rewritten));
        assert!(!has_received(&bytes));
        // 知らないキーがあれば、受けた見た目を外してもエントリは残る
        let mut v: Value = serde_json::from_slice(
            &write_received(Some(&ReceivedLook::default()), None)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        v["futureKey"] = Value::from(1);
        let kept = write_received(None, Some(&serde_json::to_vec(&v).unwrap())).unwrap();
        assert!(kept.is_some());
        let plain = write_received(Some(&ReceivedLook::default()), None)
            .unwrap()
            .unwrap();
        assert_eq!(write_received(None, Some(&plain)).unwrap(), None);
        // 本体が読めない（形式は同じ）エントリは、受けた見た目を外しても消さない
        let mut broken: Value = serde_json::from_slice(&plain).unwrap();
        broken["properties"]["_A"] = serde_json::json!({"float": "x"});
        let kept = write_received(None, Some(&serde_json::to_vec(&broken).unwrap())).unwrap();
        assert!(kept.is_some());
    }

    #[test]
    fn unknown_keys_survive_a_rewrite() {
        let mut v: Value = serde_json::from_slice(&write(&sample(), None).unwrap()).unwrap();
        v["futureKey"] = serde_json::json!({"x": [1, 2]});
        let previous = serde_json::to_vec(&v).unwrap();
        let mut changed = sample();
        changed.kind = LookKind::Standard;
        let rewritten: Value =
            serde_json::from_slice(&write(&changed, Some(&previous)).unwrap()).unwrap();
        assert_eq!(rewritten["futureKey"], serde_json::json!({"x": [1, 2]}));
        assert_eq!(rewritten["kind"], "standard");
        assert_eq!(
            read(&serde_json::to_vec(&rewritten).unwrap()).unwrap(),
            changed
        );
    }

    #[test]
    fn malformed_and_newer_entries_are_refused() {
        let good: Value = serde_json::from_slice(&write(&sample(), None).unwrap()).unwrap();
        let refuse = |edit: &dyn Fn(&mut Value)| {
            let mut v = good.clone();
            edit(&mut v);
            read(&serde_json::to_vec(&v).unwrap()).is_err()
        };
        assert!(refuse(&|v| v["format"] = Value::from(2)));
        assert!(refuse(&|v| v["format"] = Value::from(0)));
        assert!(refuse(&|v| v["format"] = Value::from("1")));
        assert!(refuse(&|v| v["kind"] = Value::from("lilToonFur")));
        assert!(refuse(&|v| v["shader"] = Value::from(3)));
        assert!(refuse(
            &|v| v["properties"]["_A"] = serde_json::json!({"float": "x"})
        ));
        assert!(refuse(
            &|v| v["properties"]["_A"] = serde_json::json!({"color": [1, 2, 3]})
        ));
        assert!(refuse(
            &|v| v["properties"]["_A"] = serde_json::json!({"texture": 1})
        ));
        assert!(refuse(
            &|v| v["properties"]["_A"] = serde_json::json!({"int": 5_000_000_000i64})
        ));
        assert!(refuse(
            &|v| v["properties"]["_A"] = serde_json::json!({"float": 1, "int": 1})
        ));
        assert!(refuse(
            &|v| v["properties"][""] = serde_json::json!({"float": 1})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"channel": 64})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"channel": -1})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"packed": ["zero"]})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"packed": [{"channel": 0, "component": 4}, "one", "one", "one"]})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"image": "00"})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] =
                serde_json::json!({"image": "00000000000000000000000000000000"})
        ));
        assert!(refuse(
            &|v| v["textures"]["_T"] = serde_json::json!({"lut": 1})
        ));
        assert!(refuse(&|v| v["keywords"] = serde_json::json!(["A", "A"])));
        assert!(read(b"[]").is_err());
        assert!(read(b"{").is_err());
        assert!(read(&vec![b' '; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn json_numbers_are_not_rounded_through_text() {
        // f32 → f64 の JSON → f32 で同じ値に戻る（色の値・境界の値を保存で変えない）
        let mut look = MaterialLook::default();
        for (i, x) in [0.1f32, 1.0 / 3.0, 0.82, 1e-7, 123456.79, -0.0]
            .iter()
            .enumerate()
        {
            look.properties
                .insert(format!("_V{i}"), LookValue::Float(*x));
        }
        let back = read(&write(&look, None).unwrap()).unwrap();
        for (k, v) in &look.properties {
            assert_eq!(
                back.properties[k].as_f32().to_bits(),
                v.as_f32().to_bits(),
                "{k}"
            );
        }
    }
}
