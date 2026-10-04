use crate::{Error, Result};
#[derive(Clone)]
pub(super) struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub end: usize,
    /// 予約領域・余白がゼロでなくても断らない（写しとしての取り込み。原本を保つ読みは厳密）。
    pub lenient: bool,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len(),
            lenient: false,
        }
    }
    pub fn lenient(data: &'a [u8]) -> Self {
        Self {
            lenient: true,
            ..Self::new(data)
        }
    }
    pub fn remaining(&self) -> usize {
        self.end - self.pos
    }
    pub fn fail<T>(&self, why: &str) -> Result<T> {
        Err(Error::InvalidData(format!("{}: {why}", self.pos)))
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return self.fail("PSD の区間が途切れています");
        }
        let start = self.pos;
        self.pos += n;
        Ok(&self.data[start..self.pos])
    }
    pub fn slice(&mut self, n: usize) -> Result<Self> {
        let pos = self.pos;
        self.take(n)?;
        Ok(Self {
            data: self.data,
            pos,
            end: pos + n,
            lenient: self.lenient,
        })
    }
    pub fn section(&mut self) -> Result<Self> {
        let n = self.u32()? as usize;
        if n > i32::MAX as usize {
            return self.fail("区間長が上限を超えています");
        }
        self.slice(n)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn key(&mut self) -> Result<[u8; 4]> {
        Ok(self.take(4)?.try_into().unwrap())
    }
    pub fn zeros(&mut self, n: usize) -> Result<()> {
        let lenient = self.lenient;
        if self.take(n)?.iter().any(|b| *b != 0) && !lenient {
            return self.fail("予約領域・余白がゼロではありません");
        }
        Ok(())
    }
    pub fn utf16(&mut self, n: usize) -> Result<String> {
        let b = self.take(
            n.checked_mul(2)
                .ok_or_else(|| Error::InvalidData("UTF-16 長のオーバーフロー".into()))?,
        )?;
        let units = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        if self.lenient {
            // 写しとしての取り込み: 対にならない代用対は置き換え文字にして読む
            return Ok(String::from_utf16_lossy(&units));
        }
        String::from_utf16(&units)
            .map_err(|_| Error::InvalidData(format!("{}: 不正なUTF-16", self.pos)))
    }
}
pub(super) trait Emit {
    fn u16(&mut self, v: u16);
    fn u32(&mut self, v: u32);
    fn section(&mut self, b: &[u8]);
    fn tag(&mut self, key: &[u8; 4], b: &[u8]);
}
impl Emit for Vec<u8> {
    fn u16(&mut self, v: u16) {
        self.extend(v.to_be_bytes())
    }
    fn u32(&mut self, v: u32) {
        self.extend(v.to_be_bytes())
    }
    fn section(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.extend(b)
    }
    fn tag(&mut self, key: &[u8; 4], b: &[u8]) {
        self.extend(b"8BIM");
        self.extend(key);
        self.section(b);
        if !b.len().is_multiple_of(2) {
            self.push(0)
        }
    }
}
