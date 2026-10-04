//! SoCo の ActionDescriptor。知らない型は長さを推測せず拒否する。
use super::binary::Reader;
use crate::{check, check_budget, Error, Result};
enum Value {
    Object(Vec<u8>, Vec<(Vec<u8>, Value)>),
    Number(f64),
    Bool(bool),
    Other,
}
/// 平らな記述子の 1 つの値（数か真偽）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Scalar {
    Number(f64),
    Bool(bool),
}
fn count(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    check_budget(n <= 100000, "記述子の項目数上限超過")?;
    Ok(n)
}
fn name(r: &mut Reader) -> Result<()> {
    let n = r.u32()? as usize;
    let bytes = n
        .checked_mul(2)
        .ok_or_else(|| Error::InvalidData("記述子名長のオーバーフロー".into()))?;
    r.take(bytes)?;
    Ok(())
}
fn key(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.u32()? as usize;
    Ok(r.take(if n == 0 { 4 } else { n })?.to_vec())
}
fn object(r: &mut Reader, depth: usize) -> Result<Value> {
    check_budget(depth <= 32, "記述子の深さ上限超過")?;
    name(r)?;
    let class = key(r)?;
    let n = count(r)?;
    let mut items = Vec::new();
    for _ in 0..n {
        let k = key(r)?;
        let t = r.key()?;
        items.push((k, value(r, t, depth)?))
    }
    Ok(Value::Object(class, items))
}
fn value(r: &mut Reader, t: [u8; 4], depth: usize) -> Result<Value> {
    check_budget(depth <= 32, "記述子の深さ上限超過")?;
    match &t {
        b"Objc" | b"GlbO" => return object(r, depth + 1),
        b"doub" => {
            return Ok(Value::Number(f64::from_be_bytes(
                r.take(8)?.try_into().unwrap(),
            )))
        }
        b"UntF" => {
            r.take(4)?;
            return Ok(Value::Number(f64::from_be_bytes(
                r.take(8)?.try_into().unwrap(),
            )));
        }
        b"long" => return Ok(Value::Number(f64::from(r.i32()?))),
        b"comp" => {
            return Ok(Value::Number(
                i64::from_be_bytes(r.take(8)?.try_into().unwrap()) as f64,
            ))
        }
        b"VlLs" => {
            let n = count(r)?;
            for _ in 0..n {
                let t = r.key()?;
                value(r, t, depth + 1)?;
            }
        }
        b"bool" => return Ok(Value::Bool(r.u8()? != 0)),
        b"TEXT" => name(r)?,
        b"enum" => {
            key(r)?;
            key(r)?;
        }
        b"type" | b"GlbC" => {
            name(r)?;
            key(r)?;
        }
        b"alis" | b"tdta" => {
            let n = r.u32()? as usize;
            r.take(n)?;
        }
        b"obj " => {
            let n = count(r)?;
            for _ in 0..n {
                match &r.key()? {
                    b"prop" => {
                        name(r)?;
                        key(r)?;
                        key(r)?;
                    }
                    b"Clss" => {
                        name(r)?;
                        key(r)?;
                    }
                    b"Enmr" => {
                        name(r)?;
                        key(r)?;
                        key(r)?;
                        key(r)?;
                    }
                    b"rele" => {
                        name(r)?;
                        key(r)?;
                        r.i32()?;
                    }
                    b"Idnt" | b"indx" => {
                        r.i32()?;
                    }
                    b"name" => {
                        name(r)?;
                        key(r)?;
                        name(r)?;
                    }
                    _ => return r.fail("未知の記述子参照の型"),
                }
            }
        }
        _ => return r.fail("未知の記述子の型"),
    }
    Ok(Value::Other)
}
pub(super) fn color(r: &mut Reader) -> Result<[f64; 3]> {
    let root = object(r, 0)?;
    r.zeros(r.remaining())?;
    let Value::Object(_, root) = root else {
        unreachable!()
    };
    check(
        root.len() == 1 && root[0].0 == b"Clr ",
        "SoCo のClr オブジェクトがありません",
    )?;
    let Value::Object(class, items) = &root[0].1 else {
        return r.fail("SoCo のClr がオブジェクトではありません");
    };
    check(class == b"RGBC", "SoCo はRGB記述子のみ対応します")?;
    let mut rgb = [0.0; 3];
    for (i, k) in [b"Rd  ", b"Grn ", b"Bl  "].iter().enumerate() {
        let Some((_, Value::Number(v))) = items.iter().find(|(key, _)| key == *k) else {
            return r.fail("SoCo のRGB成分が不足しています");
        };
        rgb[i] = *v
    }
    Ok(rgb)
}
/// 平らな記述子の項目の並び（キーと値）。
pub(super) type FlatItems = Vec<(Vec<u8>, Scalar)>;
/// 平らな ActionDescriptor（項目が数と真偽だけ）。項目の並びを返す。入れ子・文字列・列挙などを含むものは対応しないので None。
/// 末尾の余白（0）以外が残っていれば壊れたデータとして断る。
pub(super) fn flat(r: &mut Reader) -> Result<Option<FlatItems>> {
    let root = object(r, 0)?;
    r.zeros(r.remaining())?;
    let Value::Object(_, items) = root else {
        unreachable!()
    };
    Ok(items
        .into_iter()
        .map(|(k, v)| match v {
            Value::Number(n) => Some((k, Scalar::Number(n))),
            Value::Bool(b) => Some((k, Scalar::Bool(b))),
            _ => None,
        })
        .collect())
}
