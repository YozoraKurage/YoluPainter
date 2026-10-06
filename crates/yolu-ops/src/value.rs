//! JSON の値の型: 効果の欄の値（数・真偽・文字列）、色の文字列、画像のバイト列（base64）。

use std::borrow::Cow;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use yolu_core::effects::catalog::ParamValue;
use yolu_core::Rgba8;

/// 効果の欄の値。数（整数も数）・真偽・文字列（選択肢・色）。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Text(String),
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            // 整数の値は整数で出す（4.0 ではなく 4）
            Value::Number(v) if v.fract() == 0.0 && v.abs() < 9.0e15 => s.serialize_i64(*v as i64),
            Value::Number(v) => s.serialize_f64(*v),
            Value::Bool(v) => s.serialize_bool(*v),
            Value::Text(v) => s.serialize_str(v),
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Number(n) => n
                .as_f64()
                .map(Value::Number)
                .ok_or_else(|| D::Error::custom("数が大きすぎます")),
            serde_json::Value::Bool(b) => Ok(Value::Bool(b)),
            serde_json::Value::String(t) => Ok(Value::Text(t)),
            other => Err(D::Error::custom(format!(
                "値は数・真偽・文字列のどれかです（{} は使えません）",
                match other {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Array(_) => "配列",
                    _ => "オブジェクト",
                }
            ))),
        }
    }
}

impl JsonSchema for Value {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Value")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A number, a boolean or a string (a choice).",
            "anyOf": [{"type": "number"}, {"type": "boolean"}, {"type": "string"}]
        })
    }
}

impl From<ParamValue> for Value {
    fn from(v: ParamValue) -> Self {
        match v {
            ParamValue::Number(n) => Value::Number(n),
            ParamValue::Bool(b) => Value::Bool(b),
            ParamValue::Choice(c) => Value::Text(c),
        }
    }
}
impl From<Value> for ParamValue {
    fn from(v: Value) -> Self {
        match v {
            Value::Number(n) => ParamValue::Number(n),
            Value::Bool(b) => ParamValue::Bool(b),
            Value::Text(t) => ParamValue::Choice(t),
        }
    }
}

/// `#rrggbb` か `#rrggbbaa`（`#` は省いてもよい。大文字小文字は問わない）から色へ。アルファを省くと不透明。
pub fn parse_color(text: &str) -> Option<Rgba8> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Rgba8::new(byte(0)?, byte(2)?, byte(4)?, if hex.len() == 8 { byte(6)? } else { 255 }))
}

/// 色を `#rrggbb`（不透明）か `#rrggbbaa` に。
pub fn format_color(c: Rgba8) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

/// base64（標準の綴り、`=` で埋める）の文字列として JSON に出るバイト列。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| ALPHABET.iter().position(|a| *a == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, quad) in bytes.chunks(4).enumerate() {
        let last = i == bytes.len() / 4 - 1;
        let pad = quad.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (k, c) in quad.iter().enumerate() {
            n <<= 6;
            if k < 4 - pad {
                n |= value(*c)?;
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64_encode(&self.0))
    }
}
impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        base64_decode(&text)
            .map(Bytes)
            .ok_or_else(|| serde::de::Error::custom("base64 ではありません"))
    }
}
impl JsonSchema for Bytes {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Base64Bytes")
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Binary data as a base64 string (standard alphabet, padded).",
            "type": "string",
            "contentEncoding": "base64"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_tail_length() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let text = base64_encode(&data);
            assert_eq!(text.len() % 4, 0);
            assert_eq!(base64_decode(&text).unwrap(), data, "{len}");
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert!(base64_decode("TQ=").is_none());
        assert!(base64_decode("T===").is_none());
        assert!(base64_decode("TQ==TQ==").is_none());
        assert!(base64_decode("T!==").is_none());
    }

    #[test]
    fn colors_parse_and_print() {
        assert_eq!(parse_color("#ff8000"), Some(Rgba8::new(255, 128, 0, 255)));
        assert_eq!(parse_color("FF800040"), Some(Rgba8::new(255, 128, 0, 64)));
        assert_eq!(format_color(Rgba8::new(255, 128, 0, 255)), "#ff8000");
        assert_eq!(format_color(Rgba8::new(1, 2, 3, 4)), "#01020304");
        for bad in ["", "#fff", "#12345", "#gg0000", "#1234567", "red"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn values_keep_integers_as_integers_and_refuse_other_json() {
        assert_eq!(serde_json::to_string(&Value::Number(4.0)).unwrap(), "4");
        assert_eq!(serde_json::to_string(&Value::Number(0.25)).unwrap(), "0.25");
        assert_eq!(serde_json::to_string(&Value::Bool(true)).unwrap(), "true");
        assert_eq!(serde_json::from_str::<Value>("\"x\"").unwrap(), Value::Text("x".into()));
        assert_eq!(serde_json::from_str::<Value>("3").unwrap(), Value::Number(3.0));
        for bad in ["null", "[1]", "{}"] {
            assert!(serde_json::from_str::<Value>(bad).is_err(), "{bad}");
        }
    }
}
