//! 取り込みの試験用のブラシのファイル。試験の中で、公開された形式の並びどおりに一から組む（外のファイルは持ち込まない）。
#![allow(dead_code)]

/// ビッグエンディアンの書き出し器。
#[derive(Default, Clone)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> W {
        W(Vec::new())
    }
    pub fn i16(mut self, v: i32) -> W {
        self.0.extend_from_slice(&(v as i16).to_be_bytes());
        self
    }
    pub fn i32(mut self, v: i64) -> W {
        self.0.extend_from_slice(&(v as i32).to_be_bytes());
        self
    }
    pub fn u8(mut self, v: i32) -> W {
        self.0.push(v as u8);
        self
    }
    pub fn bytes(mut self, b: &[u8]) -> W {
        self.0.extend_from_slice(b);
        self
    }
    pub fn ascii(self, a: &str) -> W {
        self.bytes(a.as_bytes())
    }
    /// 長さ（NUL を含む UTF-16 の単位数）と UTF-16BE。
    pub fn unicode(mut self, t: &str) -> W {
        let units: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
        self = self.i32(units.len() as i64);
        for u in units {
            self.0.extend_from_slice(&u.to_be_bytes());
        }
        self
    }
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// GIMP の `.gbr`（版 2）。`pixels` は上の行から（ファイルの並び）。`bytes` は 1（灰色）か 4（RGBA）。
pub fn gbr(
    width: u32,
    height: u32,
    pixels: &[u8],
    bytes: u32,
    name: &str,
    spacing: u32,
) -> Vec<u8> {
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0);
    let header = 28 + name_bytes.len() as u32;
    let mut out = Vec::new();
    for v in [header, 2, width, height, bytes] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&0x4749_4D50u32.to_be_bytes());
    out.extend_from_slice(&spacing.to_be_bytes());
    out.extend_from_slice(&name_bytes);
    out.extend_from_slice(pixels);
    out
}

/// 3 × 2 の灰色の `.gbr`（上 = 255, 0, 0 / 下 = 0, 0, 128）。
pub fn gbr_gray(name: &str) -> Vec<u8> {
    gbr(3, 2, &[255, 0, 0, 0, 0, 128], 1, name, 50)
}

/// 色つきの `.gbr`（2 × 1）。アルファだけを筆先にするので「色つきの筆先」が表せなかった項目になる。
pub fn gbr_color(name: &str) -> Vec<u8> {
    gbr(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128], 4, name, 25)
}

/// Photoshop の `.abr` 版 1: 計算で描くブラシ 1 つと、画像のブラシ 1 つ（3 × 2）。
pub fn abr_v1() -> Vec<u8> {
    let computed = W::new()
        .i32(0)
        .i16(30)
        .i16(40)
        .i16(50)
        .i16(-30)
        .i16(80)
        .done();
    let sampled = W::new()
        .i32(0)
        .i16(25)
        .u8(1)
        .i16(0)
        .i16(0)
        .i16(2)
        .i16(3)
        .i32(0)
        .i32(0)
        .i32(2)
        .i32(3)
        .i16(8)
        .u8(0)
        .bytes(&[255, 0, 0, 0, 0, 128])
        .done();
    W::new()
        .i16(1)
        .i16(2)
        .i16(1)
        .i32(computed.len() as i64)
        .bytes(&computed)
        .i16(2)
        .i32(sampled.len() as i64)
        .bytes(&sampled)
        .done()
}

/// 模様 1 つ（Photoshop の Pattern 構造。灰色の 8 bit、無圧縮）。
fn pattern(name: &str, id: &str, w: i32, h: i32, plane: &[u8]) -> Vec<u8> {
    let body = W::new()
        .i32(1)
        .i32(1)
        .i16(h)
        .i16(w)
        .unicode(name)
        .u8(id.len() as i32)
        .ascii(id);
    let array = W::new()
        .i32(8)
        .i32(0)
        .i32(0)
        .i32(h as i64)
        .i32(w as i64)
        .i16(8)
        .u8(0)
        .bytes(plane)
        .done();
    let list = W::new()
        .i32(0)
        .i32(0)
        .i32(h as i64)
        .i32(w as i64)
        .i32(1)
        .i32(1)
        .i32(array.len() as i64)
        .bytes(&array)
        .i32(0)
        .i32(0)
        .done();
    body.i32(3).i32(list.len() as i64).bytes(&list).done()
}

/// `.pat`: 2 × 2 の灰色の模様をいくつか（名前の順）。
pub fn pat_file(names: &[&str]) -> Vec<u8> {
    let mut w = W::new().ascii("8BPT").i16(1).i32(names.len() as i64);
    for (i, name) in names.iter().enumerate() {
        let v = (i as u8 + 1) * 20;
        w = w.bytes(&pattern(
            name,
            &format!("p{i}"),
            2,
            2,
            &[v, v + 1, v + 2, v + 3],
        ));
    }
    w.done()
}
