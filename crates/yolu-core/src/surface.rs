//! 疎なタイルの straight RGBA8 の面（C# の SparseTileSurface・TileStorage）。
//!
//! - 原点は左下。タイルの中も行優先で、一番下の行が先。
//! - 無いタイルは全画素 0（透明・RGB も 0）。一様なタイルは 4 バイトだけ持つ。ほかは TileSize² × 4 バイト。
//! - 画布の外にはみ出すタイルの余白は必ず 0。
//! - 写し（Undo・ストロークの巻き戻し）は `Arc` を共有し、最初の書き込みでだけ複製する（copy-on-write）。
//! - アルファ 0 の画素の RGB も、明示して消すまでそのまま保つ。

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::CoreError;
use crate::tile_cache::TileCell;
use crate::types::{Rgba8, TileCoord};

/// 1 枚のタイルの中身。「無い」は `Option<Tile>` の `None` で表す。
#[derive(Clone, Debug)]
pub(crate) enum Tile {
    /// 全画素が同じ色（4 バイト）。
    Uniform(Rgba8),
    /// 全画素（TileSize² × 4 バイト）の持ち主。写しと共有し、中身は変えない（書くときは新しい cell にする）。中身はディスクへ
    /// 逃がされていることがある（読む口が読み戻す）。
    Data(Arc<TileCell>),
}

/// 読んだタイルの中身（全画素の並びはメモリに置いたまま持つ。持っている間はディスクへ逃がさない）。
#[derive(Clone, Debug)]
pub(crate) enum Pixels {
    Uniform(Rgba8),
    Data(Arc<Vec<u8>>),
}

impl Pixels {
    /// index はタイルの中のバイトの位置（画素の番号 × 4）。
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Rgba8 {
        match self {
            Pixels::Uniform(c) => *c,
            Pixels::Data(d) => Rgba8::from_slice(&d[index..index + 4]),
        }
    }

    /// 全画素の並び（一様なら広げる）。
    pub(crate) fn copy_to(&self, destination: &mut [u8]) {
        match self {
            Pixels::Data(d) => destination.copy_from_slice(d),
            Pixels::Uniform(c) => fill(destination, *c),
        }
    }
}

impl Tile {
    /// 全画素の並びのタイル（新しい cell）。
    #[inline]
    pub(crate) fn data(bytes: Vec<u8>) -> Tile {
        Tile::Data(TileCell::new(bytes))
    }

    /// 全画素の並びのタイルで、ディスクへ逃がさないもの（効果の評価のキャッシュの中身。キャッシュの予算で持ち、作り直せる）。
    #[inline]
    pub(crate) fn kept(bytes: Vec<u8>) -> Tile {
        Tile::Data(TileCell::kept(bytes))
    }

    /// 予算に数える大きさ（C# の TileStorage.ByteSize）。ディスクへ逃がしていても同じ。
    #[inline]
    pub(crate) fn byte_size(&self) -> u64 {
        match self {
            Tile::Uniform(_) => 4,
            Tile::Data(d) => d.len() as u64,
        }
    }

    /// 中身を読む（ディスクへ逃がしていれば読み戻す）。
    #[inline]
    pub(crate) fn read(&self) -> Result<Pixels, CoreError> {
        match self {
            Tile::Uniform(c) => Ok(Pixels::Uniform(*c)),
            Tile::Data(d) => Ok(Pixels::Data(d.bytes()?)),
        }
    }

    /// index はタイルの中のバイトの位置（画素の番号 × 4）。
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Result<Rgba8, CoreError> {
        match self {
            Tile::Uniform(c) => Ok(*c),
            Tile::Data(d) => d.with(|d| Rgba8::from_slice(&d[index..index + 4])),
        }
    }

    /// この画素の並びのタイル: 全部 0 なら無し、全画素が同じなら一様、ほかは写し（C# の TileStorage.FromBytes）。
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Tile> {
        if uniformity(bytes) {
            let c = Rgba8::from_slice(&bytes[0..4]);
            if c == Rgba8::TRANSPARENT {
                None
            } else {
                Some(Tile::Uniform(c))
            }
        } else {
            Some(Tile::data(bytes.to_vec()))
        }
    }

    /// `from_bytes` の、並びを受け取る形（写さない）。
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Option<Tile> {
        if uniformity(&bytes) {
            let c = Rgba8::from_slice(&bytes[0..4]);
            if c == Rgba8::TRANSPARENT {
                None
            } else {
                Some(Tile::Uniform(c))
            }
        } else {
            Some(Tile::data(bytes))
        }
    }

    /// 詰め直し: 全画素が同じなら一様に、全部透明（0）なら無しに（C# の TileStorage.Compact）。読めないタイルはそのまま。
    pub(crate) fn compact(&self) -> Option<Tile> {
        match self {
            Tile::Uniform(c) => {
                if *c == Rgba8::TRANSPARENT {
                    None
                } else {
                    Some(self.clone())
                }
            }
            Tile::Data(cell) => {
                let Ok(Some(first)) =
                    cell.with(|d| uniformity(d).then(|| Rgba8::from_slice(&d[0..4])))
                else {
                    return Some(self.clone());
                };
                if first == Rgba8::TRANSPARENT {
                    None
                } else {
                    Some(Tile::Uniform(first))
                }
            }
        }
    }

    /// 全画素の並び（一様なら広げる）。
    pub(crate) fn copy_to(&self, destination: &mut [u8]) -> Result<(), CoreError> {
        match self {
            Tile::Data(d) => d.with(|d| destination.copy_from_slice(d)),
            Tile::Uniform(c) => {
                fill(destination, *c);
                Ok(())
            }
        }
    }

    /// 2 つの状態が同じ画素か（C# の TileStorage.Same）。片方だけが一様なら、もう片方の全画素がその色か。
    /// 読めないタイルは違うとみなす（同じと決めて変化を落とさない）。
    pub(crate) fn same(a: Option<&Tile>, b: Option<&Tile>) -> bool {
        match (a, b) {
            (None, None) => true,
            (None, Some(_)) | (Some(_), None) => false,
            (Some(Tile::Uniform(x)), Some(Tile::Uniform(y))) => x == y,
            (Some(Tile::Data(x)), Some(Tile::Data(y))) => {
                TileCell::ptr_eq(x, y)
                    || match (x.bytes(), y.bytes()) {
                        (Ok(x), Ok(y)) => x[..] == y[..],
                        _ => false,
                    }
            }
            (Some(Tile::Uniform(u)), Some(Tile::Data(d)))
            | (Some(Tile::Data(d)), Some(Tile::Uniform(u))) => d
                .with(|d| d.chunks_exact(4).all(|p| p == u.to_array()))
                .unwrap_or(false),
        }
    }

    /// ディスクから読めなかったことのある中身か。
    pub(crate) fn is_lost(&self) -> bool {
        matches!(self, Tile::Data(d) if d.is_lost())
    }
}

/// 全画素が最初の画素と同じか。
fn uniformity(bytes: &[u8]) -> bool {
    let first = [bytes[0], bytes[1], bytes[2], bytes[3]];
    bytes.chunks_exact(4).all(|p| p == first)
}

pub(crate) fn fill(bytes: &mut [u8], c: Rgba8) {
    if c == Rgba8::TRANSPARENT {
        bytes.fill(0);
        return;
    }
    let a = c.to_array();
    for p in bytes.chunks_exact_mut(4) {
        p.copy_from_slice(&a);
    }
}

/// 面が大きくなるときの予算の確かめ（文書の画素の予算 − ほかの面の分）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Growth {
    /// 文書の SourceBudgetBytes。
    pub budget: u64,
    /// この面以外が今持っているバイト数。
    pub others: u64,
}

impl Growth {
    #[cfg(test)]
    pub(crate) const UNLIMITED: Growth = Growth {
        budget: u64::MAX,
        others: 0,
    };
    /// C# の PaintDocument.EnsureSourceGrowth: additional > 0 かつ additional > 予算 − 全体 なら断る。
    #[inline]
    pub(crate) fn ensure(&self, own: u64, additional: i64) -> Result<(), CoreError> {
        if additional > 0 {
            let room = self.budget as i128 - (self.others as i128 + own as i128);
            if additional as i128 > room {
                return Err(CoreError::SourceBudgetExceeded);
            }
        }
        Ok(())
    }
}

/// 疎なタイルの面。単独のスレッドから書き換える（合成のワーカーは読むだけ）。
#[derive(Clone, Debug)]
pub struct Surface {
    width: u32,
    height: u32,
    tile_size: u32,
    pub(crate) tiles: HashMap<TileCoord, Tile>,
    /// タイルの byte_size の合計。タイルを足す・外す・入れ替える・広げるたびに増減する。
    pub(crate) allocated: u64,
}

impl Surface {
    pub(crate) fn new(width: u32, height: u32, tile_size: u32) -> Surface {
        Surface {
            width,
            height,
            tile_size,
            tiles: HashMap::new(),
            allocated: 0,
        }
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }
    /// 1 タイルのバイト数（TileSize² × 4）。
    #[inline]
    pub fn tile_bytes(&self) -> usize {
        (self.tile_size as usize) * (self.tile_size as usize) * 4
    }
    /// 中身のあるタイルの数。
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn has_tile(&self, coord: TileCoord) -> bool {
        self.tiles.contains_key(&coord)
    }
    /// 持っている画素のバイト数（一様なタイルは 4）。
    pub fn allocated_bytes(&self) -> u64 {
        self.allocated
    }
    /// 中身のあるタイルの座標（Y、次に X の順）。
    pub fn tile_coords(&self) -> Vec<TileCoord> {
        let mut v: Vec<TileCoord> = self.tiles.keys().copied().collect();
        v.sort();
        v
    }
    /// 横・縦のタイルの数。
    pub fn tile_columns(&self) -> u32 {
        self.width.div_ceil(self.tile_size)
    }
    pub fn tile_rows(&self) -> u32 {
        self.height.div_ceil(self.tile_size)
    }

    /// 画素（画布の外は Err）。無いタイルは透明。
    pub fn pixel(&self, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素がキャンバスの外"));
        }
        let ts = self.tile_size;
        match self.tiles.get(&TileCoord::new(x / ts, y / ts)) {
            None => Ok(Rgba8::TRANSPARENT),
            Some(t) => t.get((((y % ts) * ts + x % ts) * 4) as usize),
        }
    }

    /// 1 行の画素（x から out.len() / 4 画素、straight RGBA8）。無いタイルと画布の外は 0。
    pub fn read_row(&self, x: u32, y: u32, out: &mut [u8]) -> Result<(), CoreError> {
        let n = out.len() / 4;
        let ts = self.tile_size;
        let mut i = 0usize;
        while i < n {
            let px = x + i as u32;
            let run = ((ts - px % ts) as usize).min(n - i);
            let dst = &mut out[i * 4..(i + run) * 4];
            match self.tiles.get(&TileCoord::new(px / ts, y / ts)) {
                None => dst.fill(0),
                Some(Tile::Uniform(c)) => fill(dst, *c),
                Some(Tile::Data(d)) => {
                    let at = (((y % ts) * ts + px % ts) * 4) as usize;
                    d.with(|d| dst.copy_from_slice(&d[at..at + run * 4]))?;
                }
            }
            i += run;
        }
        Ok(())
    }

    /// 1 タイルを呼び手の領域（TileSize² × 4 バイト、行優先・下の行から、余白 0）へ写す。無いタイルは 0 を書いて false。
    pub fn copy_tile(&self, coord: TileCoord, destination: &mut [u8]) -> Result<bool, CoreError> {
        self.check_coord(coord)?;
        if destination.len() != self.tile_bytes() {
            return Err(CoreError::InvalidArgument("タイルのバイト数が違う"));
        }
        match self.tiles.get(&coord) {
            None => {
                destination.fill(0);
                Ok(false)
            }
            Some(t) => {
                t.copy_to(destination)?;
                Ok(true)
            }
        }
    }

    /// 画布の大きさの straight RGBA8（行は下から）。試験向け（ディスクから読めない中身があれば止まる）。
    pub fn to_canvas_bytes(&self) -> Vec<u8> {
        self.canvas_bytes()
            .expect("ディスクのキャッシュからタイルを読めない")
    }

    /// 画布の大きさの straight RGBA8（行は下から）。ディスクから読めない中身があれば誤り。
    pub fn canvas_bytes(&self) -> Result<Vec<u8>, CoreError> {
        let (w, h, ts) = (
            self.width as usize,
            self.height as usize,
            self.tile_size as usize,
        );
        let mut out = vec![0u8; w * h * 4];
        let mut buf = vec![0u8; self.tile_bytes()];
        for (coord, tile) in &self.tiles {
            tile.copy_to(&mut buf)?;
            let (x0, y0) = (coord.x as usize * ts, coord.y as usize * ts);
            let cw = ts.min(w - x0);
            for row in 0..ts.min(h - y0) {
                let src = &buf[row * ts * 4..][..cw * 4];
                out[((y0 + row) * w + x0) * 4..][..cw * 4].copy_from_slice(src);
            }
        }
        Ok(out)
    }

    #[inline]
    pub(crate) fn tile(&self, coord: TileCoord) -> Option<&Tile> {
        self.tiles.get(&coord)
    }

    /// 1 タイルの中身を読む（無いタイルは None。ディスクへ逃がしていれば読み戻す）。
    #[inline]
    pub(crate) fn read(&self, coord: TileCoord) -> Result<Option<Pixels>, CoreError> {
        self.tiles.get(&coord).map(Tile::read).transpose()
    }

    /// ディスクから読めなかった中身を持っているか（読もうとして失敗したタイルだけ。読んでいないタイルは数えない）。
    pub fn has_unreadable_tiles(&self) -> bool {
        self.tiles.values().any(Tile::is_lost)
    }

    /// 持っている全画素のタイルの中身を今すぐディスクへ逃がす（読んでいる最中の中身は飛ばす）。逃がした数。試験と計測の口。
    #[doc(hidden)]
    pub fn evict_tiles_now(&self) -> usize {
        self.tiles
            .values()
            .filter(|t| matches!(t, Tile::Data(d) if d.evict()))
            .count()
    }

    /// 全画素のタイルのうち、ディスクにだけある（メモリに無い）数。試験と計測の口。
    #[doc(hidden)]
    pub fn evicted_tile_count(&self) -> usize {
        self.tiles
            .values()
            .filter(|t| matches!(t, Tile::Data(d) if d.residency() == (false, true)))
            .count()
    }

    /// 試験: 持っている全画素のタイルを、これから先ディスクから読めないことにする（キャッシュが壊れた・外れたときの試験の口）。
    #[doc(hidden)]
    pub fn fail_tile_reads_for_test(&self) {
        for t in self.tiles.values() {
            if let Tile::Data(d) = t {
                d.fail_reads_for_test();
            }
        }
    }

    pub(crate) fn check_coord(&self, coord: TileCoord) -> Result<(), CoreError> {
        if (coord.x as u64) * (self.tile_size as u64) >= self.width as u64
            || (coord.y as u64) * (self.tile_size as u64) >= self.height as u64
        {
            Err(CoreError::InvalidArgument("タイルの座標がキャンバスの外"))
        } else {
            Ok(())
        }
    }

    /// タイルを置き換える（写しを置く。`None` で外す）。Undo・取消・読み込みが使う。
    pub(crate) fn restore(&mut self, coord: TileCoord, snapshot: Option<&Tile>) {
        if let Some(old) = self.tiles.remove(&coord) {
            self.allocated -= old.byte_size();
        }
        if let Some(t) = snapshot {
            self.allocated += t.byte_size();
            self.tiles.insert(coord, t.clone());
        }
    }

    /// タイルを詰め直す（一様・透明になっていれば小さく）。
    pub(crate) fn compact(&mut self, coord: TileCoord) {
        if let Some(tile) = self.tiles.get(&coord) {
            let compact = tile.compact();
            let old = tile.byte_size();
            self.allocated -= old;
            match compact {
                None => {
                    self.tiles.remove(&coord);
                }
                Some(t) => {
                    self.allocated += t.byte_size();
                    self.tiles.insert(coord, t);
                }
            }
        }
    }

    /// 置き換えたときの大きさの差（予算の確かめ用）。
    pub(crate) fn growth_to(&self, coord: TileCoord, next: Option<&Tile>) -> i64 {
        next.map_or(0, |t| t.byte_size() as i64)
            - self.tiles.get(&coord).map_or(0, |t| t.byte_size() as i64)
    }

    /// 1 タイルを丸ごと読み込む（C# の ImportTile）。余白が 0 でなければ断る。同じ中身なら false。
    pub(crate) fn import_tile(
        &mut self,
        coord: TileCoord,
        bytes: &[u8],
        growth: Growth,
    ) -> Result<bool, CoreError> {
        let next = self.prepare_import(coord, bytes)?;
        self.commit_import(coord, next, growth)
    }

    /// `import_tile` の、確かめて詰める段（面を変えない。ワーカーで並べられる）。
    pub(crate) fn prepare_import(
        &self,
        coord: TileCoord,
        bytes: &[u8],
    ) -> Result<Option<Tile>, CoreError> {
        self.check_import(coord, bytes)?;
        Ok(Tile::from_bytes(bytes))
    }

    /// 読み込むタイルの座標・バイト数・キャンバスの外の余白（0 でなければ断る）を確かめる。
    fn check_import(&self, coord: TileCoord, bytes: &[u8]) -> Result<(), CoreError> {
        self.check_coord(coord)?;
        if bytes.len() != self.tile_bytes() {
            return Err(CoreError::InvalidArgument("タイルのバイト数が違う"));
        }
        let ts = self.tile_size as usize;
        let (sx, sy) = (coord.x as usize * ts, coord.y as usize * ts);
        let (w, h) = (self.width as usize, self.height as usize);
        // キャンバスの内側だけのタイルに余白は無い
        if sx + ts <= w && sy + ts <= h {
            return Ok(());
        }
        for y in 0..ts {
            for x in 0..ts {
                if sx + x < w && sy + y < h {
                    continue;
                }
                let p = (y * ts + x) * 4;
                if bytes[p..p + 4] != [0, 0, 0, 0] {
                    return Err(CoreError::InvalidArgument(
                        "キャンバスの外の余白は 0 でなければならない",
                    ));
                }
            }
        }
        Ok(())
    }

    /// `import_tile` の、予算を確かめて置く段（確かめて詰めたタイル）。同じ中身なら false。
    pub(crate) fn commit_import(
        &mut self,
        coord: TileCoord,
        next: Option<Tile>,
        growth: Growth,
    ) -> Result<bool, CoreError> {
        growth.ensure(self.allocated, self.growth_to(coord, next.as_ref()))?;
        if Tile::same(self.tiles.get(&coord), next.as_ref()) {
            return Ok(false);
        }
        self.restore(coord, next.as_ref());
        Ok(true)
    }

    /// 1 画素を直接書く（C# の SetPixelInternal + Compact）。変わったら true。
    pub(crate) fn set_pixel(
        &mut self,
        x: u32,
        y: u32,
        color: Rgba8,
        growth: Growth,
    ) -> Result<bool, CoreError> {
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素がキャンバスの外"));
        }
        let ts = self.tile_size;
        let coord = TileCoord::new(x / ts, y / ts);
        let index = (((y % ts) * ts + x % ts) * 4) as usize;
        let full = self.tile_bytes();
        let changed = write_pixel(
            &mut self.tiles,
            &mut self.allocated,
            coord,
            index,
            color,
            full,
            growth,
        )?;
        if changed {
            self.compact(coord);
        }
        Ok(changed)
    }
}

/// 面を、最近読んだタイルの中身を持ったまま 1 画素ずつ読む（読み手ごとに 1 つ作り、共有しない）。画素ごとにタイルを引き直さず、
/// 双線形の 4 点がタイルの境をまたいでも、持っている数枚の間で引き直しを繰り返さない（引くたびに、ほかのワーカーと同じタイルの
/// 参照の数を書き合う）。ディスクから読めないタイルは誤りを覚えて透明として進める（画素ごとの式を誤りの分岐で重くしない）。
/// 読み終えたら `finish` で誤りを確かめ、誤りなら読んだ結果を使わない。
pub(crate) struct PixelReader<'a> {
    surface: &'a Surface,
    /// 最近読んだタイル（座標と中身。無いタイルは None）。
    recent: [(Option<TileCoord>, Option<Pixels>); 4],
    /// 次に入れ替える位置。
    next: usize,
    failed: Option<CoreError>,
}

impl<'a> PixelReader<'a> {
    pub(crate) fn new(surface: &'a Surface) -> Self {
        PixelReader {
            surface,
            recent: Default::default(),
            next: 0,
            failed: None,
        }
    }
    pub(crate) fn surface(&self) -> &'a Surface {
        self.surface
    }
    /// 画素（画布の外は透明）。
    #[inline]
    pub(crate) fn pixel(&mut self, x: i64, y: i64) -> Rgba8 {
        let s = self.surface;
        if x < 0 || y < 0 || x >= s.width as i64 || y >= s.height as i64 {
            return Rgba8::TRANSPARENT;
        }
        let ts = s.tile_size as i64;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        let slot = match self.recent.iter().position(|(c, _)| *c == Some(coord)) {
            Some(i) => i,
            None => {
                let i = self.next;
                self.next = (i + 1) % self.recent.len();
                let tile = match s.read(coord) {
                    Ok(t) => t,
                    Err(e) => {
                        self.failed.get_or_insert(e);
                        None
                    }
                };
                self.recent[i] = (Some(coord), tile);
                i
            }
        };
        match &self.recent[slot].1 {
            None => Rgba8::TRANSPARENT,
            Some(t) => t.get((((y % ts) * ts + x % ts) * 4) as usize),
        }
    }
    /// 読んだ画素が全部読めたか。
    pub(crate) fn finish(self) -> Result<(), CoreError> {
        self.failed.map_or(Ok(()), Err)
    }
}

/// 面ごとの `PixelReader`（画素ごとの式が、ワーカーごとに 1 つ持つ。どの面を読むかは呼ぶ所で決める）。
#[derive(Default)]
pub(crate) struct Readers<'a> {
    list: Vec<PixelReader<'a>>,
}

impl<'a> Readers<'a> {
    /// 面の画素（画布の外は透明）。ディスクから読めない画素は透明で、`finish` が誤りを返す。
    #[inline]
    pub(crate) fn pixel(&mut self, surface: &'a Surface, x: u32, y: u32) -> Rgba8 {
        let i = match self
            .list
            .iter()
            .position(|r| std::ptr::eq(r.surface, surface))
        {
            Some(i) => i,
            None => {
                self.list.push(PixelReader::new(surface));
                self.list.len() - 1
            }
        };
        self.list[i].pixel(x as i64, y as i64)
    }
    /// 読んだ画素が全部読めたか。
    pub(crate) fn finish(self) -> Result<(), CoreError> {
        self.list.into_iter().try_for_each(PixelReader::finish)
    }
}

/// 1 画素を書く（C# の WritePixelQuiet と同じ予算の確かめを同じ順で）。無いタイルへ透明を書くのは何もしない。
pub(crate) fn write_pixel(
    tiles: &mut HashMap<TileCoord, Tile>,
    allocated: &mut u64,
    coord: TileCoord,
    index: usize,
    color: Rgba8,
    full: usize,
    growth: Growth,
) -> Result<bool, CoreError> {
    let tile = match tiles.get_mut(&coord) {
        Some(t) => t,
        None => {
            if color == Rgba8::TRANSPARENT {
                return Ok(false);
            }
            growth.ensure(*allocated, full as i64)?;
            *allocated += 4;
            tiles
                .entry(coord)
                .or_insert(Tile::Uniform(Rgba8::TRANSPARENT))
        }
    };
    write_into(tile, allocated, index, color, full, growth)
}

/// あるタイルへ 1 画素を書く。一様なタイルは広げ、写しと共有しているタイルは複製してから書く（中身は新しい cell になる）。
#[inline]
pub(crate) fn write_into(
    tile: &mut Tile,
    allocated: &mut u64,
    index: usize,
    color: Rgba8,
    full: usize,
    growth: Growth,
) -> Result<bool, CoreError> {
    if tile.get(index)? == color {
        return Ok(false);
    }
    let mut data = match tile {
        Tile::Uniform(u) => {
            growth.ensure(*allocated, full as i64 - 4)?;
            let mut data = vec![0u8; full];
            fill(&mut data, *u);
            *allocated += full as u64 - 4;
            data
        }
        Tile::Data(cell) => {
            // 中身を取り出す（ほかが持っていれば複製）。直前に読めたので、まず失敗しない
            let cell = cell.clone();
            let bytes = cell.bytes()?;
            *tile = Tile::Uniform(Rgba8::TRANSPARENT);
            TileCell::detach(cell, bytes)
        }
    };
    data[index..index + 4].copy_from_slice(&color.to_array());
    *tile = Tile::data(data);
    Ok(true)
}

/// ストロークが 1 枚のタイルを処理する間の、そのタイルの状態（面から取り出して持ち、終わったら戻す）。
/// 写しと共有しているタイルは、初めて書くときに 1 回だけ自分のものにする（画素ごとに持ち主を確かめない）。
/// 予算の確かめは `write_pixel` と同じ順。
pub(crate) enum LiveTile {
    Absent,
    Uniform(Rgba8),
    /// 面のタイルの持ち主と、読んだ中身（持っている間はディスクへ逃がさない）。
    Shared(Arc<TileCell>, Arc<Vec<u8>>),
    Owned(Vec<u8>),
}

impl LiveTile {
    /// 面からタイルを取り出す（中身を先に読むので、読めなければ面はそのまま）。
    pub(crate) fn take(surface: &mut Surface, coord: TileCoord) -> Result<LiveTile, CoreError> {
        let live = match surface.tiles.get(&coord) {
            None => LiveTile::Absent,
            Some(Tile::Uniform(c)) => LiveTile::Uniform(*c),
            Some(Tile::Data(d)) => LiveTile::Shared(d.clone(), d.bytes()?),
        };
        surface.tiles.remove(&coord);
        Ok(live)
    }
    /// 取り出したタイルを面へ戻す。
    pub(crate) fn put_back(self, surface: &mut Surface, coord: TileCoord) {
        if let Some(t) = self.into_tile() {
            surface.tiles.insert(coord, t);
        }
    }
    pub(crate) fn into_tile(self) -> Option<Tile> {
        match self {
            LiveTile::Absent => None,
            LiveTile::Uniform(c) => Some(Tile::Uniform(c)),
            LiveTile::Shared(cell, _) => Some(Tile::Data(cell)),
            LiveTile::Owned(v) => Some(Tile::data(v)),
        }
    }
    /// 今の状態の写し（巻き戻し用。全画素のタイルは持ち主を共有する）と、その読んだ中身。
    pub(crate) fn snapshot(&mut self) -> (Option<Tile>, Option<Pixels>) {
        if let LiveTile::Owned(v) = self {
            // 書いた後に写しを取ることは無い（写しはそのタイルの最初の画素の前）が、来ても共有の形へ戻して正しく写す
            let (cell, bytes) = TileCell::new_pinned(std::mem::take(v));
            *self = LiveTile::Shared(cell, bytes);
        }
        match self {
            LiveTile::Absent => (None, None),
            LiveTile::Uniform(c) => (Some(Tile::Uniform(*c)), Some(Pixels::Uniform(*c))),
            LiveTile::Shared(cell, bytes) => (
                Some(Tile::Data(cell.clone())),
                Some(Pixels::Data(bytes.clone())),
            ),
            LiveTile::Owned(_) => unreachable!(),
        }
    }
    pub(crate) fn byte_size(&self) -> u64 {
        match self {
            LiveTile::Absent => 0,
            LiveTile::Uniform(_) => 4,
            LiveTile::Shared(_, d) => d.len() as u64,
            LiveTile::Owned(v) => v.len() as u64,
        }
    }
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Rgba8 {
        match self {
            LiveTile::Absent => Rgba8::TRANSPARENT,
            LiveTile::Uniform(c) => *c,
            LiveTile::Shared(_, d) => Rgba8::from_slice(&d[index..index + 4]),
            LiveTile::Owned(v) => Rgba8::from_slice(&v[index..index + 4]),
        }
    }
    /// 1 画素を書く（C# の WritePixelQuiet）。同じ値なら何もせず false。
    #[inline]
    pub(crate) fn write(
        &mut self,
        index: usize,
        color: Rgba8,
        allocated: &mut u64,
        full: usize,
        growth: Growth,
    ) -> Result<bool, CoreError> {
        if let LiveTile::Owned(v) = self {
            if v[index..index + 4] == color.to_array() {
                return Ok(false);
            }
            v[index..index + 4].copy_from_slice(&color.to_array());
            return Ok(true);
        }
        if self.get(index) == color {
            return Ok(false); // 無いタイルへ透明を書くのもここ
        }
        if let LiveTile::Absent = self {
            growth.ensure(*allocated, full as i64)?;
            *allocated += 4;
            *self = LiveTile::Uniform(Rgba8::TRANSPARENT);
        }
        match std::mem::replace(self, LiveTile::Absent) {
            LiveTile::Uniform(u) => {
                if let Err(e) = growth.ensure(*allocated, full as i64 - 4) {
                    *self = LiveTile::Uniform(u);
                    return Err(e);
                }
                let mut data = vec![0u8; full];
                fill(&mut data, u);
                *allocated += full as u64 - 4;
                *self = LiveTile::Owned(data);
            }
            LiveTile::Shared(cell, bytes) => {
                *self = LiveTile::Owned(TileCell::detach(cell, bytes));
            }
            _ => unreachable!(),
        }
        match self {
            LiveTile::Owned(v) => v[index..index + 4].copy_from_slice(&color.to_array()),
            _ => unreachable!(),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_import_is_four_bytes_and_detached() {
        // C# CoreTests: 8×8・タイル 4 の面へ全画素 (17,0,0,255) のタイルを読み込むと一様（4 バイト）、呼び手の領域を後で変えても影響しない
        let mut s = Surface::new(8, 8, 4);
        let mut bytes = vec![0u8; 64];
        for p in bytes.chunks_exact_mut(4) {
            p.copy_from_slice(&[17, 0, 0, 255]);
        }
        s.import_tile(TileCoord::new(1, 0), &bytes, Growth::UNLIMITED)
            .unwrap();
        assert_eq!(s.allocated_bytes(), 4);
        bytes[0] = 88;
        assert_eq!(s.pixel(4, 0).unwrap().r, 17);
        let mut out = vec![0u8; 64];
        assert!(s.copy_tile(TileCoord::new(1, 0), &mut out).unwrap());
        out[0] = 99;
        assert_eq!(s.pixel(4, 0).unwrap().r, 17);
    }

    #[test]
    fn edge_padding_must_be_zero() {
        // C# CoreTests: 5×5・タイル 4 の (1,1) で画布の外の画素に値があると断り、何も持たない
        let mut s = Surface::new(5, 5, 4);
        let mut bytes = vec![0u8; 64];
        bytes[7] = 255; // 画素 (1,0) のアルファ = 画布の (5,4)、外
        assert!(s
            .import_tile(TileCoord::new(1, 1), &bytes, Growth::UNLIMITED)
            .is_err());
        assert_eq!(s.tile_count(), 0);
    }

    #[test]
    fn transparent_rgb_is_kept() {
        // 無いタイル = 全バイト 0。アルファ 0 でも RGB のあるタイルは残る
        let mut s = Surface::new(16, 16, 4);
        assert!(s
            .set_pixel(1, 1, Rgba8::new(32, 64, 128, 0), Growth::UNLIMITED)
            .unwrap());
        assert_eq!(s.tile_count(), 1);
        assert_eq!(s.pixel(1, 1).unwrap(), Rgba8::new(32, 64, 128, 0));
    }

    #[test]
    fn coordinates_are_sorted_and_empty_tiles_removed() {
        // C# CoreTests: SetPixel(12,8)・(0,0) の後、(8,4) を足し (0,0) を透明にすると、そのタイルは消える
        let mut s = Surface::new(16, 16, 4);
        let red = Rgba8::new(255, 0, 0, 255);
        s.set_pixel(12, 8, red, Growth::UNLIMITED).unwrap();
        s.set_pixel(0, 0, red, Growth::UNLIMITED).unwrap();
        let snapshot = s.tile_coords();
        s.set_pixel(8, 4, red, Growth::UNLIMITED).unwrap();
        s.set_pixel(0, 0, Rgba8::TRANSPARENT, Growth::UNLIMITED)
            .unwrap();
        assert_eq!(snapshot, vec![TileCoord::new(0, 0), TileCoord::new(3, 2)]);
        assert_eq!(
            s.tile_coords(),
            vec![TileCoord::new(2, 1), TileCoord::new(3, 2)]
        );
    }

    #[test]
    fn copy_tile_pads_with_zero_and_rejects_bad_buffers() {
        // C# ChangeTrackingTests の CopyTile
        let mut s = Surface::new(20, 20, 8);
        s.set_pixel(17, 18, Rgba8::new(10, 20, 30, 40), Growth::UNLIMITED)
            .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                s.set_pixel(x, y, Rgba8::new(7, 7, 7, 7), Growth::UNLIMITED)
                    .unwrap();
            }
        }
        assert_eq!(s.allocated_bytes(), 4 + 256); // (0,0) は一様に詰め直されている
        let mut buf = vec![0u8; 256];
        assert!(s.copy_tile(TileCoord::new(2, 2), &mut buf).unwrap());
        let p = ((18 - 16) * 8 + (17 - 16)) * 4;
        assert_eq!(&buf[p..p + 4], &[10, 20, 30, 40]);
        assert!(buf[(4 * 8 * 4)..].iter().all(|&b| b == 0)); // 画布の外（行 20〜23）は 0
        assert!(s.copy_tile(TileCoord::new(0, 0), &mut buf).unwrap());
        assert!(buf.iter().all(|&b| b == 7));
        buf.fill(0xAB);
        assert!(!s.copy_tile(TileCoord::new(1, 1), &mut buf).unwrap());
        assert!(buf.iter().all(|&b| b == 0));
        assert!(s.copy_tile(TileCoord::new(0, 0), &mut [0u8; 4]).is_err());
        assert!(s.copy_tile(TileCoord::new(3, 0), &mut buf).is_err());
    }

    #[test]
    fn budget_refuses_growth() {
        let mut s = Surface::new(16, 16, 4);
        let g = Growth {
            budget: 64,
            others: 0,
        };
        s.set_pixel(0, 0, Rgba8::new(1, 2, 3, 4), g).unwrap();
        assert_eq!(s.allocated_bytes(), 64);
        assert_eq!(
            s.set_pixel(8, 8, Rgba8::new(1, 2, 3, 4), g),
            Err(CoreError::SourceBudgetExceeded)
        );
    }
}
