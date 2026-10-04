//! 範囲を確かめるビッグエンディアンの読み手と PackBits、展開の予算。
//!
//! 信頼できないファイルを読むので、尽きたら必ず `Fault::Truncated` で止め（パニックしない）、宣言された個数・長さは残りの
//! バイト数に入ると確かめてから使う（巨大な宣言で先に確保しない）。

use super::error::{Counted, Fault, PackBitsFault, Result};

/// 1 回の取り込みで展開する筆先・模様の画素の合計の上限（バイト）。PackBits は 1 バイトの繰り返し（2 バイト）で 128 画素へ
/// 展開できるので、ファイルの大きさの上限だけでは、小さなファイルが巨大な画像へ展開される形（圧縮爆弾）を断てない。
pub const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

/// 展開した画素の予算。使う分を先に引き、足りなければ `Fault::Budget`。
#[derive(Debug)]
pub(crate) struct Budget {
    remaining: u64,
}

impl Budget {
    pub fn new(limit: u64) -> Budget {
        Budget { remaining: limit }
    }
    pub fn take(&mut self, bytes: u64) -> Result<()> {
        if bytes > self.remaining {
            return Err(Fault::Budget);
        }
        self.remaining -= bytes;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }
    pub fn at(data: &'a [u8], pos: usize) -> Reader<'a> {
        Reader { data, pos }
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    /// 位置を動かす（後ろへ飛んでもよい。次の読みが範囲を確かめる）。
    pub fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn need(&self, count: usize) -> Result<()> {
        if self.pos > self.data.len() || count > self.remaining() {
            return Err(Fault::Truncated { offset: self.pos });
        }
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8> {
        self.need(1)?;
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn i64(&mut self) -> Result<i64> {
        let b = self.bytes(8)?;
        Ok(i64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.i64()? as u64))
    }
    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        self.need(count)?;
        let slice = &self.data[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }
    pub fn skip(&mut self, count: usize) -> Result<()> {
        self.need(count)?;
        self.pos += count;
        Ok(())
    }
    /// 4 文字などの印（ASCII 以外は `?`）。
    pub fn ascii(&mut self, count: usize) -> Result<String> {
        Ok(ascii(self.bytes(count)?))
    }
    /// 宣言された長さ（要素数）が、残りのバイト数に入るか確かめて、使える数にする。
    pub fn count(&self, value: u32, element_size: usize, what: Counted) -> Result<usize> {
        if value as u64 * element_size as u64 > self.remaining() as u64 {
            return Err(Fault::BadCount {
                what,
                value: value as u64,
                offset: self.pos,
            });
        }
        Ok(value as usize)
    }
    /// 宣言された長さ（u32）を読み、残りに入る数として返す。
    pub fn counted(&mut self, what: Counted) -> Result<usize> {
        let value = self.u32()?;
        self.count(value, 1, what)
    }
}

pub(crate) fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { '?' })
        .collect()
}

/// UTF-16BE の文字列（末尾の NUL は除く。壊れたサロゲートは U+FFFD）。
pub(crate) fn utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_string()
}

/// PackBits の 1 行（ちょうど `width` バイトに展開する）。
pub(crate) fn decode_packbits_row(source: &[u8], width: usize) -> Result<Vec<u8>> {
    let mut row = Vec::with_capacity(width);
    let mut s = 0usize;
    while row.len() < width {
        if s >= source.len() {
            return Err(Fault::PackBits(PackBitsFault::EndedEarly));
        }
        let n = source[s] as i8;
        s += 1;
        if n >= 0 {
            let count = n as usize + 1;
            if s + count > source.len() || row.len() + count > width {
                return Err(Fault::PackBits(PackBitsFault::Overflow));
            }
            row.extend_from_slice(&source[s..s + count]);
            s += count;
        } else if n != -128 {
            let count = 1 + (-(n as i16)) as usize;
            if s >= source.len() || row.len() + count > width {
                return Err(Fault::PackBits(PackBitsFault::Overflow));
            }
            let v = source[s];
            s += 1;
            row.resize(row.len() + count, v);
        }
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stop_at_the_end_without_panicking() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16().unwrap(), 0x0102);
        assert!(matches!(r.u16(), Err(Fault::Truncated { offset: 2 })));
        r.set_position(100);
        assert!(r.u8().is_err());
        assert_eq!(r.remaining(), 0);
        assert!(
            r.bytes(0).is_err(),
            "終わりを越えた位置からは 0 バイトの読みも断る"
        );
    }

    #[test]
    fn declared_counts_must_fit_the_rest() {
        let r = Reader::new(&[0; 10]);
        assert_eq!(r.count(10, 1, Counted::PixelData).unwrap(), 10);
        assert!(r.count(11, 1, Counted::PixelData).is_err());
        assert!(r.count(6, 2, Counted::PixelData).is_err());
        assert!(
            r.count(u32::MAX, 4, Counted::PixelData).is_err(),
            "32 bit の積があふれない"
        );
    }

    #[test]
    fn packbits_matches_the_rows_of_the_published_description() {
        assert_eq!(
            decode_packbits_row(&[0x00, 255, 0xFF, 0], 3).unwrap(),
            vec![255, 0, 0]
        );
        assert_eq!(
            decode_packbits_row(&[0xFF, 0, 0x00, 128], 3).unwrap(),
            vec![0, 0, 128]
        );
        assert_eq!(
            decode_packbits_row(&[0x80, 0x00, 7], 1).unwrap(),
            vec![7],
            "-128 は何もしない"
        );
        assert!(decode_packbits_row(&[0x00, 1], 2).is_err(), "途中で終わる");
        assert!(decode_packbits_row(&[0x02, 1, 2], 2).is_err(), "幅を超える");
        assert!(
            decode_packbits_row(&[0xFE, 9], 2).is_err(),
            "繰り返しが幅を超える（3 回）"
        );
        assert!(
            decode_packbits_row(&[0xFF], 2).is_err(),
            "繰り返しの値が無い"
        );
    }

    #[test]
    fn the_budget_refuses_what_it_cannot_cover() {
        let mut b = Budget::new(10);
        b.take(6).unwrap();
        assert_eq!(b.take(5), Err(Fault::Budget));
        b.take(4).unwrap();
    }

    #[test]
    fn utf16_drops_trailing_nuls_and_replaces_broken_surrogates() {
        assert_eq!(utf16be(&[0x00, 0x41, 0x00, 0x00]), "A");
        assert_eq!(utf16be(&[0xD8, 0x00, 0x00, 0x41]), "\u{FFFD}A");
    }
}
