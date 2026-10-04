//! Photoshop の ActionDescriptor（ABR の `desc` 節）。Photoshop File Formats Specification の「Descriptor structure」から書いた。
//!
//! 知らない値の型は長さが分からないので、そこで読み止めて断る（推測しない）。深さ・個数・全体の値の数に上限があり、
//! 宣言された個数で先に確保しない（読めた分だけ増える）。有限でない数は断る（設定の計算に NaN を入れない）。

use super::error::{Counted, Fault, Result};
use super::printable;
use super::reader::{utf16be, Reader};

const MAX_DEPTH: usize = 32;
const MAX_ITEMS: u32 = 100_000;
/// 1 つの記述子全体の値（入れ子を含む）の数の上限。大きな ABR の全プリセットが十分入り、悪意のある宣言が数 GB を確保しない大きさ。
const MAX_VALUES: usize = 1_000_000;

#[derive(Debug)]
pub(crate) enum Value {
    Object(Object),
    List(Vec<Value>),
    Number(f64),
    Bool(bool),
    Text(String),
    Enum {
        value: String,
    },
    /// 型・参照・生のデータ（読み飛ばすだけで解釈しない）。
    Other,
}

#[derive(Debug, Default)]
pub(crate) struct Object {
    pub class_id: String,
    /// ファイルの並び（キーが重なっても、取るのは最初）。
    pub items: Vec<(String, Value)>,
}

impl Object {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn object(&self, key: &str) -> Option<&Object> {
        match self.get(key) {
            Some(Value::Object(o)) => Some(o),
            _ => None,
        }
    }
    pub fn list(&self, key: &str) -> Option<&[Value]> {
        match self.get(key) {
            Some(Value::List(l)) => Some(l),
            _ => None,
        }
    }
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.get(key) {
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Text(t)) => Some(t),
            _ => None,
        }
    }
    /// 列挙値（`enum` の 2 つ目の ID）。
    pub fn enum_value(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Enum { value }) => Some(value),
            _ => None,
        }
    }
}

/// 読み手の位置にある記述子（名前・クラス ID・項目）を読む。
pub(crate) fn read_descriptor(r: &mut Reader) -> Result<Object> {
    let mut values = 0usize;
    object(r, 0, &mut values)
}

fn object(r: &mut Reader, depth: usize, values: &mut usize) -> Result<Object> {
    if depth > MAX_DEPTH {
        return Err(Fault::DescriptorDepth);
    }
    unicode_string(r)?;
    let class_id = key(r)?;
    let count = r.u32()?;
    if count > MAX_ITEMS {
        return Err(Fault::DescriptorItems);
    }
    let mut o = Object {
        class_id,
        items: Vec::new(),
    };
    for _ in 0..count {
        let k = key(r)?;
        let kind = r.ascii(4)?;
        let v = value(r, &kind, depth, values)?;
        o.items.push((k, v));
    }
    Ok(o)
}

fn value(r: &mut Reader, kind: &str, depth: usize, values: &mut usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(Fault::DescriptorDepth);
    }
    *values += 1;
    if *values > MAX_VALUES {
        return Err(Fault::DescriptorItems);
    }
    Ok(match kind {
        "Objc" | "GlbO" => Value::Object(object(r, depth + 1, values)?),
        "VlLs" => {
            let n = r.u32()?;
            if n > MAX_ITEMS {
                return Err(Fault::DescriptorItems);
            }
            let mut list = Vec::new();
            for _ in 0..n {
                let k = r.ascii(4)?;
                list.push(value(r, &k, depth + 1, values)?);
            }
            Value::List(list)
        }
        "UntF" => {
            r.skip(4)?; // 単位
            Value::Number(finite(r.f64()?)?)
        }
        "doub" => Value::Number(finite(r.f64()?)?),
        "long" => Value::Number(r.i32()? as f64),
        "comp" => Value::Number(r.i64()? as f64),
        "bool" => Value::Bool(r.u8()? != 0),
        "TEXT" => Value::Text(unicode_string(r)?),
        "enum" => {
            key(r)?;
            Value::Enum { value: key(r)? }
        }
        "type" | "GlbC" => {
            unicode_string(r)?;
            key(r)?;
            Value::Other
        }
        "alis" | "tdta" => {
            let n = r.counted(Counted::DataLength)?;
            r.skip(n)?;
            Value::Other
        }
        "obj " => {
            reference(r)?;
            Value::Other
        }
        _ => {
            return Err(Fault::DescriptorType {
                kind: printable(kind, 32),
                offset: r.position().saturating_sub(4),
            })
        }
    })
}

fn finite(v: f64) -> Result<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Fault::DescriptorNotFinite)
    }
}

fn reference(r: &mut Reader) -> Result<()> {
    let n = r.u32()?;
    if n > MAX_ITEMS {
        return Err(Fault::DescriptorItems);
    }
    for _ in 0..n {
        let form = r.ascii(4)?;
        match form.as_str() {
            "prop" => {
                unicode_string(r)?;
                key(r)?;
                key(r)?;
            }
            "Clss" => {
                unicode_string(r)?;
                key(r)?;
            }
            "Enmr" => {
                unicode_string(r)?;
                key(r)?;
                key(r)?;
                key(r)?;
            }
            "rele" => {
                unicode_string(r)?;
                key(r)?;
                r.i32()?;
            }
            "Idnt" | "indx" => {
                r.i32()?;
            }
            "name" => {
                unicode_string(r)?;
                key(r)?;
                unicode_string(r)?;
            }
            _ => return Err(Fault::DescriptorReference(printable(&form, 32))),
        }
    }
    Ok(())
}

/// キー: 長さ 0 なら 4 文字、そうでなければその長さの文字列。
pub(crate) fn key(r: &mut Reader) -> Result<String> {
    let length = r.u32()?;
    if length == 0 {
        r.ascii(4)
    } else {
        let n = r.count(length, 1, Counted::KeyLength)?;
        r.ascii(n)
    }
}

/// 長さ（UTF-16 の単位数）つきの文字列。
pub(crate) fn unicode_string(r: &mut Reader) -> Result<String> {
    let value = r.u32()?;
    let units = r.count(value, 2, Counted::StringLength)?;
    Ok(utf16be(r.bytes(units * 2)?))
}
