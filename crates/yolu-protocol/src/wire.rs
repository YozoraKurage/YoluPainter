//! 中身の読み書きの基礎（リトルエンディアンの数・UTF-8 の文字列・数の並び）。読みは必ず長さを確かめ、上限を超える数は読む前に断る
//! （壊れた・悪意のある長さで大きな領域を取らない）。

use std::fmt;

/// 中身を読めなかった理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// 中身が途中で終わっている。
    Truncated,
    /// 数が上限を超える（何の数か）。
    TooLarge(&'static str),
    /// 値が決まりに合わない（何の値か）。
    Invalid(&'static str),
    /// 文字列が UTF-8 でない。
    Utf8,
    /// この版の知らない種類の命令。
    UnknownKind(u16),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "中身が途中で終わっています"),
            DecodeError::TooLarge(what) => write!(f, "{what} が上限を超えます"),
            DecodeError::Invalid(what) => write!(f, "{what} が決まりに合いません"),
            DecodeError::Utf8 => write!(f, "文字列が UTF-8 ではありません"),
            DecodeError::UnknownKind(kind) => write!(f, "知らない命令です（種類 0x{kind:04x}）"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// 中身を書く。
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    pub fn with_capacity(bytes: usize) -> Self {
        Writer {
            buf: Vec::with_capacity(bytes),
        }
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    /// 長さ（u32）と UTF-8 のバイト。
    pub fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
    }
    pub fn raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
    /// 数（u32）と、組ごとに N 個の f32。
    pub fn f32_groups<const N: usize>(&mut self, v: &[[f32; N]]) {
        self.u32(v.len() as u32);
        self.buf.reserve(v.len() * N * 4);
        for group in v {
            for x in group {
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
    /// 数（u32）と u32 の並び。
    pub fn u32_slice(&mut self, v: &[u32]) {
        self.u32(v.len() as u32);
        self.buf.reserve(v.len() * 4);
        for x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

/// 中身を読む。
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    /// まだ読んでいないバイトの数（後ろに足された欄は、古い読み手が読み飛ばす）。
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid("真偽の値")),
        }
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// 数（u32）を読み、上限と残りのバイト（1 つあたり item_bytes）で確かめる。
    pub fn count(
        &mut self,
        max: usize,
        item_bytes: usize,
        what: &'static str,
    ) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(DecodeError::TooLarge(what));
        }
        if item_bytes > 0 && n.saturating_mul(item_bytes) > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }
    /// 長さ（u32、max_bytes まで）と UTF-8 の文字列。
    pub fn str(&mut self, max_bytes: usize, what: &'static str) -> Result<String, DecodeError> {
        let n = self.count(max_bytes, 1, what)?;
        let bytes = self.take(n)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::Utf8)
    }
    /// 数（u32、max まで）と、組ごとに N 個の有限の f32。
    pub fn f32_groups<const N: usize>(
        &mut self,
        max: usize,
        what: &'static str,
    ) -> Result<Vec<[f32; N]>, DecodeError> {
        let n = self.count(max, N * 4, what)?;
        let bytes = self.take(n * N * 4)?;
        let mut out = Vec::with_capacity(n);
        for chunk in bytes.chunks_exact(N * 4) {
            let mut group = [0f32; N];
            for (i, x) in group.iter_mut().enumerate() {
                *x = f32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
                if !x.is_finite() {
                    return Err(DecodeError::Invalid(what));
                }
            }
            out.push(group);
        }
        Ok(out)
    }
    /// 数（u32、max まで）と u32 の並び。
    pub fn u32_vec(&mut self, max: usize, what: &'static str) -> Result<Vec<u32>, DecodeError> {
        let n = self.count(max, 4, what)?;
        let bytes = self.take(n * 4)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings_round_trip() {
        let mut w = Writer::new();
        w.u8(7);
        w.bool(true);
        w.u16(0xBEEF);
        w.u32(0xDEAD_BEEF);
        w.u64(u64::MAX - 1);
        w.i64(-42);
        w.str("マテリアル");
        w.f32_groups(&[[1.0f32, -2.5, 3.25]]);
        w.u32_slice(&[1, 2, 3]);
        let bytes = w.into_inner();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8().unwrap(), 7);
        assert!(r.bool().unwrap());
        assert_eq!(r.u16().unwrap(), 0xBEEF);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.u64().unwrap(), u64::MAX - 1);
        assert_eq!(r.i64().unwrap(), -42);
        assert_eq!(r.str(64, "名前").unwrap(), "マテリアル");
        assert_eq!(
            r.f32_groups::<3>(4, "位置").unwrap(),
            vec![[1.0, -2.5, 3.25]]
        );
        assert_eq!(r.u32_vec(4, "添字").unwrap(), vec![1, 2, 3]);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn refuses_lengths_beyond_the_data_or_the_limit() {
        let mut w = Writer::new();
        w.u32(1_000_000); // 100 万個と言うが中身が無い
        let bytes = w.into_inner();
        assert_eq!(
            Reader::new(&bytes).u32_vec(usize::MAX, "添字"),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            Reader::new(&bytes).u32_vec(10, "添字"),
            Err(DecodeError::TooLarge("添字"))
        );
        let mut w = Writer::new();
        w.f32_groups(&[[f32::NAN, 0.0]]);
        assert_eq!(
            Reader::new(&w.into_inner()).f32_groups::<2>(4, "UV"),
            Err(DecodeError::Invalid("UV"))
        );
        let mut w = Writer::new();
        w.u32(2);
        w.raw(&[0xff, 0xfe]);
        assert_eq!(
            Reader::new(&w.into_inner()).str(16, "名前"),
            Err(DecodeError::Utf8)
        );
    }
}
