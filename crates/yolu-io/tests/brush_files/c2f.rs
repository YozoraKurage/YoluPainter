//! CLIP STUDIO の素材の入れ物 C2F（`.layer`）の試験用ファイルの組み立て。本物の素材は持ち込まず、調べて分かった形
//! （docs/BRUSH_IMPORT.md の「素材の入れ物 C2F」）を試験の中で一から組む。
//!
//! 形: 先頭の印・塊（長さ u32 LE・種類・中身・CRC32 LE）の `HEAD`・`dATA`（読めない側）・`dATA`（1024 バイトのページ）・`TAIL`。
//! ページは SQLite の表の B 木（葉 0x0d・内部 0x05・オーバーフロー）で、表の定義（`sqlite_master` の行）を葉に置く。
//! 画像は `Offscreen.BlockData` の 256×256 のタイルごとの zlib。

use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;

pub const PAGE: usize = 1024;
/// 読めない側の塊が持つページの数（見た素材では 5）。
pub const HIDDEN_PAGES: usize = 5;

// ---------------- 値と行 ----------------

#[derive(Clone, Debug)]
pub enum V {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    /// 大きさだけを名乗る BLOB（中身は渡さない）。最後の列にだけ置ける。`Pager::cell_with` が、自分自身を指すオーバーフローのページで
    /// その大きさを名乗る（小さなファイルが大きな行を名乗る形）。
    Claimed(usize),
}

pub fn text(s: &str) -> V {
    V::Text(s.to_string())
}

pub fn varint(mut v: u64) -> Vec<u8> {
    if v >> 56 != 0 {
        // 9 バイト: 先頭 8 バイトは 7 ビットずつ、最後の 1 バイトは 8 ビット
        let mut out = vec![0u8; 9];
        out[8] = (v & 0xff) as u8;
        v >>= 8;
        for i in (0..8).rev() {
            out[i] = (v & 0x7f) as u8 | 0x80;
            v >>= 7;
        }
        return out;
    }
    let mut parts = vec![(v & 0x7f) as u8];
    v >>= 7;
    while v > 0 {
        parts.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    parts.reverse();
    parts
}

fn int_size(v: i64) -> (u64, usize) {
    match v {
        0 => (8, 0),
        1 => (9, 0),
        -128..=127 => (1, 1),
        -32768..=32767 => (2, 2),
        -8_388_608..=8_388_607 => (3, 3),
        -2_147_483_648..=2_147_483_647 => (4, 4),
        -140_737_488_355_328..=140_737_488_355_327 => (5, 6),
        _ => (6, 8),
    }
}

/// 行（レコード）。文字は `utf16` なら UTF-16LE。
pub fn record(values: &[V], utf16: bool) -> Vec<u8> {
    let (out, claimed) = record_parts(values, utf16);
    assert_eq!(claimed, 0, "名乗りだけの列がある行は Pager::table で置く");
    out
}

/// 行と、名乗りだけの BLOB（`V::Claimed`）の大きさ。返した行は、名乗りの分だけ宣言より短い。
pub fn record_parts(values: &[V], utf16: bool) -> (Vec<u8>, usize) {
    let mut types = Vec::new();
    let mut body = Vec::new();
    let mut claimed = 0;
    for (i, value) in values.iter().enumerate() {
        match value {
            V::Null => types.push(0),
            V::Int(v) => {
                let (t, n) = int_size(*v);
                types.push(t);
                body.extend_from_slice(&v.to_be_bytes()[8 - n..]);
            }
            V::Real(v) => {
                types.push(7);
                body.extend_from_slice(&v.to_be_bytes());
            }
            V::Text(s) => {
                let bytes: Vec<u8> = if utf16 {
                    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
                } else {
                    s.as_bytes().to_vec()
                };
                types.push(13 + 2 * bytes.len() as u64);
                body.extend_from_slice(&bytes);
            }
            V::Blob(b) => {
                types.push(12 + 2 * b.len() as u64);
                body.extend_from_slice(b);
            }
            V::Claimed(len) => {
                assert_eq!(i, values.len() - 1, "名乗りだけの列は最後に置く");
                types.push(12 + 2 * *len as u64);
                claimed = *len;
            }
        }
    }
    let types: Vec<Vec<u8>> = types.into_iter().map(varint).collect();
    let types_len: usize = types.iter().map(Vec::len).sum();
    // ヘッダーの長さは自分自身を含む
    let mut header_len = types_len + 1;
    if varint(header_len as u64).len() > 1 {
        header_len = types_len + varint(header_len as u64).len();
    }
    let mut out = varint(header_len as u64);
    for t in types {
        out.extend(t);
    }
    out.extend(body);
    (out, claimed)
}

// ---------------- ページ ----------------

/// 読める側のページの並び。ページ番号は `first` から（読めない側のページに続ける）。
pub struct Pager {
    first: u32,
    pub pages: Vec<Vec<u8>>,
}

impl Pager {
    pub fn new() -> Pager {
        Pager {
            first: HIDDEN_PAGES as u32 + 1,
            pages: Vec::new(),
        }
    }

    pub fn next_no(&self) -> u32 {
        self.first + self.pages.len() as u32
    }

    pub fn push(&mut self, mut page: Vec<u8>) -> u32 {
        assert!(page.len() <= PAGE, "ページに入らない: {}", page.len());
        page.resize(PAGE, 0);
        let no = self.next_no();
        self.pages.push(page);
        no
    }

    /// 葉のセル（行の中身が大きければオーバーフローのページを足す）。
    pub fn cell(&mut self, rowid: i64, record: &[u8]) -> Vec<u8> {
        self.cell_with(rowid, record, 0)
    }

    /// 葉のセル。`claimed` バイトぶん、行の中身が `record` より長いと名乗る（その部分は、自分自身を指す 1 ページのオーバーフローを
    /// 何度もたどって読ませる。ファイルは大きくならない）。
    pub fn cell_with(&mut self, rowid: i64, record: &[u8], claimed: usize) -> Vec<u8> {
        let total = record.len() + claimed;
        let max_local = PAGE - 35;
        let min_local = ((PAGE - 12) * 32 / 255) - 23;
        let mut out = varint(total as u64);
        out.extend(varint(rowid as u64));
        if total <= max_local {
            assert_eq!(claimed, 0, "名乗りは局所の部分に収まらない大きさで");
            out.extend_from_slice(record);
            return out;
        }
        let k = min_local + ((total - min_local) % (PAGE - 4));
        let local = if k <= max_local { k } else { min_local };
        if claimed > 0 {
            assert!(record.len() <= local, "行の先頭が局所の部分に入らない");
            out.extend_from_slice(record);
            out.resize(out.len() + (local - record.len()), 0);
            let looping = self.next_no();
            let mut page = looping.to_be_bytes().to_vec();
            page.resize(PAGE, 0xAB);
            self.push(page);
            out.extend_from_slice(&looping.to_be_bytes());
            return out;
        }
        out.extend_from_slice(&record[..local]);
        let mut rest = &record[local..];
        let first = self.next_no();
        let mut count = 0u32;
        while !rest.is_empty() {
            let take = rest.len().min(PAGE - 4);
            count += 1;
            let next = if rest.len() > take { first + count } else { 0 };
            let mut page = next.to_be_bytes().to_vec();
            page.extend_from_slice(&rest[..take]);
            self.push(page);
            rest = &rest[take..];
        }
        out.extend_from_slice(&first.to_be_bytes());
        out
    }

    /// 葉ページ（0x0d）。セルは後ろから詰める。
    pub fn leaf(&mut self, cells: &[Vec<u8>]) -> u32 {
        let mut page = vec![0u8; PAGE];
        page[0] = 0x0d;
        page[3..5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
        let mut end = PAGE;
        for (i, cell) in cells.iter().enumerate() {
            end -= cell.len();
            page[end..end + cell.len()].copy_from_slice(cell);
            page[8 + 2 * i..10 + 2 * i].copy_from_slice(&(end as u16).to_be_bytes());
        }
        assert!(8 + 2 * cells.len() <= end, "葉に入らない");
        page[5..7].copy_from_slice(&(end as u16).to_be_bytes());
        self.push(page)
    }

    /// 内部ページ（0x05）。子の番号の並びと、一番右の子。
    pub fn interior(&mut self, children: &[u32], right: u32) -> u32 {
        let mut page = vec![0u8; PAGE];
        page[0] = 0x05;
        page[3..5].copy_from_slice(&(children.len() as u16).to_be_bytes());
        page[8..12].copy_from_slice(&right.to_be_bytes());
        let mut end = PAGE;
        for (i, child) in children.iter().enumerate() {
            let mut cell = child.to_be_bytes().to_vec();
            cell.extend(varint(i as u64 + 1));
            end -= cell.len();
            page[end..end + cell.len()].copy_from_slice(&cell);
            page[12 + 2 * i..14 + 2 * i].copy_from_slice(&(end as u16).to_be_bytes());
        }
        page[5..7].copy_from_slice(&(end as u16).to_be_bytes());
        self.push(page)
    }

    /// 1 つの表を、1 行ずつの葉と（2 行以上なら）内部ページの根にして置く。根のページ番号を返す。
    pub fn table(&mut self, rows: &[(i64, Vec<V>)], utf16: bool) -> u32 {
        let leaves: Vec<u32> = rows
            .iter()
            .map(|(rowid, values)| {
                let (bytes, claimed) = record_parts(values, utf16);
                let cell = self.cell_with(*rowid, &bytes, claimed);
                self.leaf(&[cell])
            })
            .collect();
        match leaves.as_slice() {
            [only] => *only,
            [init @ .., last] => self.interior(init, *last),
            [] => self.leaf(&[]),
        }
    }
}

// ---------------- 塊と入れ物 ----------------

pub fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(body);
    out.extend_from_slice(&crc.finalize().to_le_bytes());
    out
}

pub const SIGNATURE: &[u8] = b"\x89C2F\r\n\x1a\n";

/// 読めない側の塊（先頭 2 バイト `01 00`・乱数に近い 5 ページ強）。
pub fn hidden_body() -> Vec<u8> {
    let mut out = vec![1u8, 0];
    let mut x = 0x9e37_79b9u32;
    for _ in 0..HIDDEN_PAGES * PAGE + 8 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.push((x >> 11) as u8);
    }
    out
}

/// 読める側の塊（先頭 2 バイト `00 00`・ページの並び）。
pub fn plain_body(pages: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0u8, 0];
    for page in pages {
        out.extend_from_slice(page);
    }
    out
}

pub fn container(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    for c in chunks {
        out.extend_from_slice(c);
    }
    out
}

pub fn file(hidden: &[u8], plain: &[u8]) -> Vec<u8> {
    container(&[
        chunk(b"HEAD", b""),
        chunk(b"dATA", hidden),
        chunk(b"dATA", plain),
        chunk(b"TAIL", b""),
    ])
}

// ---------------- 面（Offscreen） ----------------

fn be(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn named(name: &str) -> Vec<u8> {
    let mut out = be(&[name.encode_utf16().count() as u32]);
    out.extend(name.encode_utf16().flat_map(|u| u.to_be_bytes()));
    out
}

/// 面の記述 `Attribute`。`planes` は灰色 1・色 5、`extra` は `Parameter` の終わりの 0 の語（本物には 0 個と 2 個があった）。
pub fn attribute(width: u32, height: u32, planes: u32, extra: usize) -> Vec<u8> {
    attribute_with(width, height, planes, extra, 256)
}

/// `attribute` の、タイルの 1 辺（`Parameter` の 15・16 番目の語。256 でなければ読み手は断る）を選べるもの。
pub fn attribute_with(width: u32, height: u32, planes: u32, extra: usize, tile: u32) -> Vec<u8> {
    let tx = width.div_ceil(256);
    let ty = height.div_ceil(256);
    let mut param = named("Parameter");
    param.extend(be(&[
        width, height, tx, ty, 1, 1, 0, planes, 0, 0, 0, 1, 256, 0, tile, tile, 8, 8,
    ]));
    param.extend(be(&vec![0; extra]));
    let mut init = named("InitColor");
    init.extend(be(&[20, 0, 0, 0, 4]));
    let mut size = named("BlockSize");
    size.extend(be(&[12, tx * ty, 4]));
    size.extend(be(&vec![0x68; (tx * ty) as usize]));
    let mut out = be(&[16, param.len() as u32, init.len() as u32, size.len() as u32]);
    out.extend(param);
    out.extend(init);
    out.extend(size);
    out
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// タイル 1 つ分の塊。`tile` が None なら画素なし。
pub fn block(index: u32, tile: Option<&[u8]>) -> Vec<u8> {
    RawBlock::new(index, tile).build()
}

/// タイル 1 つ分の塊の、各欄を壊せる組み立て（`new` は正しい塊）。
#[derive(Clone)]
pub struct RawBlock {
    pub index: u32,
    /// タイルの大きさ（バイト。正しくは 65536）と、横・縦（正しくは 256）。
    pub bytes: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    /// 画素の有無の欄（正しくは 0 か 1）。
    pub has: u32,
    /// zlib の流れ（有れば長さの 2 つの欄と一緒に置く）。
    pub stream: Option<Vec<u8>>,
    /// 長さの欄（u32 BE）に足す値（正しくは 0。欄は zlib の長さ + 4）。
    pub stored_delta: i64,
    /// 塊の終わり（正しくは `BlockDataEndChunk`）。
    pub end: Vec<u8>,
}

impl RawBlock {
    pub fn new(index: u32, tile: Option<&[u8]>) -> RawBlock {
        RawBlock {
            index,
            bytes: 65536,
            tile_w: 256,
            tile_h: 256,
            has: tile.is_some() as u32,
            stream: tile.map(zlib),
            stored_delta: 0,
            end: named("BlockDataEndChunk"),
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut body = named("BlockDataBeginChunk");
        body.extend(be(&[
            self.index,
            self.bytes,
            self.tile_w,
            self.tile_h,
            self.has,
        ]));
        if let Some(z) = &self.stream {
            body.extend(be(&[(z.len() as i64 + 4 + self.stored_delta) as u32]));
            body.extend((z.len() as u32).to_le_bytes());
            body.extend(z);
        }
        body.extend(&self.end);
        let mut out = be(&[body.len() as u32 + 4]);
        out.extend(body);
        out
    }
}

/// 塊の並び（`blocks`）と、その後ろの `BlockStatus`（タイルごとの有無は `blocks.len()` 個ぶんすべて 1）。
pub fn assemble_blocks(blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for b in blocks {
        out.extend(b);
    }
    out.extend(named("BlockStatus"));
    out.extend(be(&[12, blocks.len() as u32, 4]));
    out.extend(be(&vec![1; blocks.len()]));
    out
}

/// `BlockData`: タイルの塊の並びと `BlockStatus`。`tiles` はタイルの順（行の順）。
pub fn block_data(tiles: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, t) in tiles.iter().enumerate() {
        out.extend(block(i as u32, t.as_deref()));
    }
    out.extend(named("BlockStatus"));
    out.extend(be(&[12, tiles.len() as u32, 4]));
    out.extend(be(&tiles
        .iter()
        .map(|t| t.is_some() as u32)
        .collect::<Vec<_>>()));
    out
}

/// 画像（行は上から。1 画素 1 バイト）を 256×256 のタイルに切る（はみ出す所は 0）。
pub fn tiles_of(width: u32, height: u32, values: &[u8]) -> Vec<Option<Vec<u8>>> {
    let (tx, ty) = (width.div_ceil(256), height.div_ceil(256));
    let mut out = Vec::new();
    for j in 0..ty {
        for i in 0..tx {
            let mut tile = vec![0u8; 256 * 256];
            for y in 0..256 {
                for x in 0..256 {
                    let (px, py) = (i * 256 + x, j * 256 + y);
                    if px < width && py < height {
                        tile[(y * 256 + x) as usize] = values[(py * width + px) as usize];
                    }
                }
            }
            out.push(Some(tile));
        }
    }
    out
}

// ---------------- 素材 ----------------

/// 試験の画素（なだらかな斜めの縞 + 細かい揺れ。圧縮はできるが一様ではない）。
pub fn sample_values(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| ((x * 3 + y * 5 + (x * y) % 17) % 251) as u8))
        .collect()
}

/// 組み立ての選択。
#[derive(Clone)]
pub struct Layer {
    pub width: u32,
    pub height: u32,
    /// 元の画像の画素（行は上から）。
    pub values: Vec<u8>,
    /// 表の文字を UTF-16LE にするか。
    pub utf16: bool,
    /// `Layer.ResizableOriginalMipmap`（0 なら描画用だけ）。
    pub original: bool,
    /// 描画用のミップマップの面にも画素を入れるか（元の面は別の値）。
    pub render_values: Option<Vec<u8>>,
    /// 非フォルダのレイヤーの数（1 が普通）。
    pub layers: usize,
    pub planes: u32,
    pub extra_words: usize,
    /// 元の面のタイルを欠かす（画素なし）番号。
    pub empty_tiles: Vec<usize>,
    /// 元の面もの描画用の面も、すべてのタイルが画素なしか。
    pub all_empty: bool,
    /// 元の面だけがすべて画素なしか。
    pub original_empty: bool,
    /// `Layer` 表の根の上に、子が 1 つだけの内部ページを積む段数（木を深くする。普通は 0）。
    pub extra_depth: usize,
    /// `Mipmap` 表に足す、大きさだけを名乗る行（BLOB の大きさ。どのブラシからも参照されない）。
    pub claims: Vec<usize>,
    /// `Layer` 表の定義の行を、根の違う別の表として、もう 1 つ足すか。
    pub second_layer_table: bool,
    /// `Layer` 表の定義の行を、同じ根のまま、もう 1 つ足すか（同じ行の写し）。
    pub repeated_layer_table: bool,
    /// 最後の表の定義の行だけ、文字の形（UTF-8 / UTF-16LE）を逆にするか。
    pub mixed_text: bool,
    /// 元の面の `BlockData` をこの中身にするか。
    pub blocks: Option<Vec<u8>>,
    /// `Parameter` に書くタイルの 1 辺（正しくは 256）。
    pub tile_param: u32,
}

impl Layer {
    pub fn new(width: u32, height: u32) -> Layer {
        Layer {
            width,
            height,
            values: sample_values(width, height),
            utf16: false,
            original: true,
            render_values: None,
            layers: 1,
            planes: 1,
            extra_words: 0,
            empty_tiles: Vec::new(),
            all_empty: false,
            original_empty: false,
            extra_depth: 0,
            claims: Vec::new(),
            second_layer_table: false,
            repeated_layer_table: false,
            mixed_text: false,
            blocks: None,
            tile_param: 256,
        }
    }

    /// 面の `BlockData`（`original` なら元の面。欠かすタイルは元の面だけ）。
    fn block_data(&self, values: &[u8], original: bool) -> Vec<u8> {
        if let (true, Some(blocks)) = (original, &self.blocks) {
            return blocks.clone();
        }
        let mut tiles = tiles_of(self.width, self.height, values);
        if self.all_empty || (original && self.original_empty) {
            tiles.iter_mut().for_each(|t| *t = None);
        }
        if original {
            for &i in &self.empty_tiles {
                tiles[i] = None;
            }
        }
        block_data(&tiles)
    }

    /// 読めない側の塊の後ろに続く、読める側のページの並びと、表の根を返す。
    pub fn pages(&self) -> Pager {
        let u = self.utf16;
        let mut pager = Pager::new();
        let attr = attribute_with(
            self.width,
            self.height,
            self.planes,
            self.extra_words,
            self.tile_param,
        );
        let color_attr = attribute(self.width, self.height, 5, self.extra_words);
        let empty = || {
            block_data(&vec![
                None;
                (self.width.div_ceil(256) * self.height.div_ceil(256))
                    as usize
            ])
        };
        let original_data = self.block_data(&self.values, true);
        let render_data = match &self.render_values {
            Some(v) => self.block_data(v, false),
            None => empty(),
        };
        // Offscreen: 3 = フォルダの絵（色・画素なし）、8 = 描画用、10 = 元の画像
        let off_rows = vec![
            (
                1,
                vec![
                    V::Null,
                    V::Int(3),
                    V::Int(0),
                    V::Int(2),
                    V::Blob(color_attr),
                    V::Blob(empty()),
                ],
            ),
            (
                2,
                vec![
                    V::Null,
                    V::Int(8),
                    V::Int(0),
                    V::Int(3),
                    V::Blob(attr.clone()),
                    V::Blob(render_data),
                ],
            ),
            (
                3,
                vec![
                    V::Null,
                    V::Int(10),
                    V::Int(0),
                    V::Int(3),
                    V::Blob(attr),
                    V::Blob(original_data),
                ],
            ),
        ];
        let offscreen = pager.table(&off_rows, u);
        let info_rows = vec![
            (
                1,
                vec![
                    V::Null,
                    V::Int(2),
                    V::Int(0),
                    V::Int(2),
                    V::Real(100.0),
                    V::Int(3),
                    V::Int(0),
                ],
            ),
            (
                2,
                vec![
                    V::Null,
                    V::Int(6),
                    V::Int(0),
                    V::Int(3),
                    V::Real(100.0),
                    V::Int(8),
                    V::Int(0),
                ],
            ),
            (
                3,
                vec![
                    V::Null,
                    V::Int(7),
                    V::Int(0),
                    V::Int(3),
                    V::Real(100.0),
                    V::Int(10),
                    V::Int(0),
                ],
            ),
        ];
        let info = pager.table(&info_rows, u);
        let mip_rows = vec![
            (
                1,
                vec![
                    V::Null,
                    V::Int(2),
                    V::Int(0),
                    V::Int(2),
                    V::Int(1),
                    V::Int(2),
                ],
            ),
            (
                2,
                vec![
                    V::Null,
                    V::Int(3),
                    V::Int(0),
                    V::Int(3),
                    V::Int(1),
                    V::Int(6),
                ],
            ),
            (
                3,
                vec![
                    V::Null,
                    V::Int(4),
                    V::Int(0),
                    V::Int(3),
                    V::Int(1),
                    V::Int(7),
                ],
            ),
        ];
        let mut mip_rows = mip_rows;
        for (i, len) in self.claims.iter().enumerate() {
            mip_rows.push((
                10 + i as i64,
                vec![
                    V::Null,
                    V::Int(1000 + i as i64),
                    V::Int(0),
                    V::Claimed(*len),
                ],
            ));
        }
        let mip = pager.table(&mip_rows, u);
        let mut layer_rows = vec![(
            1,
            vec![
                V::Null,
                V::Int(2),
                V::Int(0),
                text(""),
                V::Int(256),
                V::Int(1),
                V::Int(2),
                V::Int(0),
            ],
        )];
        for n in 0..self.layers {
            layer_rows.push((
                2 + n as i64,
                vec![
                    V::Null,
                    V::Int(3 + n as i64),
                    V::Int(0),
                    text("Layer name"),
                    V::Int(0),
                    V::Int(0),
                    V::Int(3),
                    V::Int(if self.original { 4 } else { 0 }),
                ],
            ));
        }
        let mut layer = pager.table(&layer_rows, u);
        for _ in 0..self.extra_depth {
            layer = pager.interior(&[], layer);
        }
        let layer_sql = "CREATE TABLE Layer(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER DEFAULT NULL, CanvasId INTEGER DEFAULT NULL, LayerName TEXT DEFAULT NULL, LayerType INTEGER DEFAULT NULL, LayerFolder INTEGER DEFAULT NULL, LayerRenderMipmap INTEGER DEFAULT NULL, ResizableOriginalMipmap INTEGER DEFAULT NULL)";
        let defs: [(&str, u32, &str); 4] = [
            ("Layer", layer, layer_sql),
            (
                "Mipmap",
                mip,
                "CREATE TABLE Mipmap(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER DEFAULT NULL, CanvasId INTEGER DEFAULT NULL, LayerId INTEGER DEFAULT NULL, MipmapCount INTEGER DEFAULT NULL, BaseMipmapInfo INTEGER DEFAULT NULL)",
            ),
            (
                "MipmapInfo",
                info,
                "CREATE TABLE MipmapInfo(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER DEFAULT NULL, CanvasId INTEGER DEFAULT NULL, LayerId INTEGER DEFAULT NULL, ThisScale REAL DEFAULT NULL, Offscreen INTEGER DEFAULT NULL, NextIndex INTEGER DEFAULT NULL)",
            ),
            (
                "Offscreen",
                offscreen,
                "CREATE TABLE Offscreen(_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, MainId INTEGER DEFAULT NULL, CanvasId INTEGER DEFAULT NULL, LayerId INTEGER DEFAULT NULL, Attribute BLOB DEFAULT NULL, BlockData BLOB DEFAULT NULL)",
            ),
        ];
        let mut defs = defs.to_vec();
        if self.repeated_layer_table {
            defs.push(("Layer", layer, layer_sql));
        }
        if self.second_layer_table {
            let other = pager.table(&layer_rows, u);
            defs.push(("Layer", other, layer_sql));
        }
        let last = defs.len() - 1;
        for (i, (name, root, sql)) in defs.iter().enumerate() {
            let row = vec![
                text("table"),
                text(name),
                text(name),
                V::Int(*root as i64),
                text(sql),
            ];
            // 最後の定義の行だけ文字の形を逆にする
            let row_utf16 = u != (self.mixed_text && i == last);
            let cell = pager.cell(3 + i as i64, &record(&row, row_utf16));
            pager.leaf(&[cell]);
        }
        pager
    }

    /// C2F のファイル全体。
    pub fn c2f(&self) -> Vec<u8> {
        file(&hidden_body(), &plain_body(&self.pager_pages()))
    }

    pub fn pager_pages(&self) -> Vec<Vec<u8>> {
        self.pages().pages
    }
}
