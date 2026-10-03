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
use crate::types::{Rgba8, TileCoord};

/// 1 枚のタイルの中身。「無い」は `Option<Tile>` の `None` で表す。
#[derive(Clone, Debug)]
pub(crate) enum Tile {
    /// 全画素が同じ色（4 バイト）。
    Uniform(Rgba8),
    /// 全画素（TileSize² × 4 バイト）。写しと共有している間は書き込みで複製する。
    Data(Arc<Vec<u8>>),
}

impl Tile {
    /// 予算に数える大きさ（C# の TileStorage.ByteSize）。
    #[inline]
    pub(crate) fn byte_size(&self) -> u64 {
        match self {
            Tile::Uniform(_) => 4,
            Tile::Data(d) => d.len() as u64,
        }
    }

    /// index はタイルの中のバイトの位置（画素の番号 × 4）。
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Rgba8 {
        match self {
            Tile::Uniform(c) => *c,
            Tile::Data(d) => Rgba8::from_slice(&d[index..index + 4]),
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
            Some(Tile::Data(Arc::new(bytes.to_vec())))
        }
    }

    /// 詰め直し: 全画素が同じなら一様に、全部透明（0）なら無しに（C# の TileStorage.Compact）。
    pub(crate) fn compact(&self) -> Option<Tile> {
        match self {
            Tile::Uniform(c) => {
                if *c == Rgba8::TRANSPARENT {
                    None
                } else {
                    Some(self.clone())
                }
            }
            Tile::Data(d) => {
                if !uniformity(d) {
                    return Some(self.clone());
                }
                let first = Rgba8::from_slice(&d[0..4]);
                if first == Rgba8::TRANSPARENT {
                    None
                } else {
                    Some(Tile::Uniform(first))
                }
            }
        }
    }

    /// 全画素の並び（一様なら広げる）。
    pub(crate) fn copy_to(&self, destination: &mut [u8]) {
        match self {
            Tile::Data(d) => destination.copy_from_slice(d),
            Tile::Uniform(c) => fill(destination, *c),
        }
    }

    /// 2 つの状態が同じ画素か（C# の TileStorage.Same）。片方だけが一様なら、もう片方の全画素がその色か。
    pub(crate) fn same(a: Option<&Tile>, b: Option<&Tile>) -> bool {
        match (a, b) {
            (None, None) => true,
            (None, Some(_)) | (Some(_), None) => false,
            (Some(Tile::Uniform(x)), Some(Tile::Uniform(y))) => x == y,
            (Some(Tile::Data(x)), Some(Tile::Data(y))) => Arc::ptr_eq(x, y) || x[..] == y[..],
            (Some(Tile::Uniform(u)), Some(Tile::Data(d)))
            | (Some(Tile::Data(d)), Some(Tile::Uniform(u))) => {
                d.chunks_exact(4).all(|p| p == u.to_array())
            }
        }
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
            return Err(CoreError::InvalidArgument("画素が画布の外"));
        }
        let ts = self.tile_size;
        Ok(match self.tiles.get(&TileCoord::new(x / ts, y / ts)) {
            None => Rgba8::TRANSPARENT,
            Some(t) => t.get((((y % ts) * ts + x % ts) * 4) as usize),
        })
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
                t.copy_to(destination);
                Ok(true)
            }
        }
    }

    /// 画布の大きさの straight RGBA8（行は下から）。試験・書き出し向け。
    pub fn to_canvas_bytes(&self) -> Vec<u8> {
        let (w, h, ts) = (
            self.width as usize,
            self.height as usize,
            self.tile_size as usize,
        );
        let mut out = vec![0u8; w * h * 4];
        let mut buf = vec![0u8; self.tile_bytes()];
        for (coord, tile) in &self.tiles {
            tile.copy_to(&mut buf);
            let (x0, y0) = (coord.x as usize * ts, coord.y as usize * ts);
            let cw = ts.min(w - x0);
            for row in 0..ts.min(h - y0) {
                let src = &buf[row * ts * 4..][..cw * 4];
                out[((y0 + row) * w + x0) * 4..][..cw * 4].copy_from_slice(src);
            }
        }
        out
    }

    #[inline]
    pub(crate) fn tile(&self, coord: TileCoord) -> Option<&Tile> {
        self.tiles.get(&coord)
    }

    pub(crate) fn check_coord(&self, coord: TileCoord) -> Result<(), CoreError> {
        if (coord.x as u64) * (self.tile_size as u64) >= self.width as u64
            || (coord.y as u64) * (self.tile_size as u64) >= self.height as u64
        {
            Err(CoreError::InvalidArgument("タイルの座標が画布の外"))
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
        self.check_coord(coord)?;
        if bytes.len() != self.tile_bytes() {
            return Err(CoreError::InvalidArgument("タイルのバイト数が違う"));
        }
        let ts = self.tile_size as usize;
        let (sx, sy) = (coord.x as usize * ts, coord.y as usize * ts);
        for y in 0..ts {
            for x in 0..ts {
                if sx + x < self.width as usize && sy + y < self.height as usize {
                    continue;
                }
                let p = (y * ts + x) * 4;
                if bytes[p..p + 4] != [0, 0, 0, 0] {
                    return Err(CoreError::InvalidArgument(
                        "画布の外の余白は 0 でなければならない",
                    ));
                }
            }
        }
        let next = Tile::from_bytes(bytes);
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
            return Err(CoreError::InvalidArgument("画素が画布の外"));
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

/// あるタイルへ 1 画素を書く。一様なタイルは広げ、写しと共有しているタイルは複製してから書く。
#[inline]
pub(crate) fn write_into(
    tile: &mut Tile,
    allocated: &mut u64,
    index: usize,
    color: Rgba8,
    full: usize,
    growth: Growth,
) -> Result<bool, CoreError> {
    if tile.get(index) == color {
        return Ok(false);
    }
    if let Tile::Uniform(u) = *tile {
        growth.ensure(*allocated, full as i64 - 4)?;
        let mut data = vec![0u8; full];
        fill(&mut data, u);
        *tile = Tile::Data(Arc::new(data));
        *allocated += full as u64 - 4;
    }
    match tile {
        Tile::Data(d) => {
            let d = Arc::make_mut(d);
            d[index..index + 4].copy_from_slice(&color.to_array());
        }
        Tile::Uniform(_) => unreachable!(),
    }
    Ok(true)
}

/// ストロークが 1 枚のタイルを処理する間の、そのタイルの状態（面から取り出して持ち、終わったら戻す）。
/// 写しと共有しているタイルは、初めて書くときに 1 回だけ自分のものにする（画素ごとの `Arc::make_mut` の原子操作を避ける）。
/// 予算の確かめは `write_pixel` と同じ順。
pub(crate) enum LiveTile {
    Absent,
    Uniform(Rgba8),
    Shared(Arc<Vec<u8>>),
    Owned(Vec<u8>),
}

impl LiveTile {
    pub(crate) fn from(tile: Option<Tile>) -> LiveTile {
        match tile {
            None => LiveTile::Absent,
            Some(Tile::Uniform(c)) => LiveTile::Uniform(c),
            Some(Tile::Data(d)) => LiveTile::Shared(d),
        }
    }
    pub(crate) fn into_tile(self) -> Option<Tile> {
        match self {
            LiveTile::Absent => None,
            LiveTile::Uniform(c) => Some(Tile::Uniform(c)),
            LiveTile::Shared(d) => Some(Tile::Data(d)),
            LiveTile::Owned(v) => Some(Tile::Data(Arc::new(v))),
        }
    }
    /// 今の状態の写し（巻き戻し用。全画素のタイルは Arc を共有する）。
    pub(crate) fn snapshot(&mut self) -> Option<Tile> {
        if let LiveTile::Owned(v) = self {
            // 書いた後に写しを取ることは無い（写しはそのタイルの最初の画素の前）が、来ても共有の形へ戻して正しく写す
            *self = LiveTile::Shared(Arc::new(std::mem::take(v)));
        }
        match self {
            LiveTile::Absent => None,
            LiveTile::Uniform(c) => Some(Tile::Uniform(*c)),
            LiveTile::Shared(d) => Some(Tile::Data(d.clone())),
            LiveTile::Owned(_) => unreachable!(),
        }
    }
    pub(crate) fn byte_size(&self) -> u64 {
        match self {
            LiveTile::Absent => 0,
            LiveTile::Uniform(_) => 4,
            LiveTile::Shared(d) => d.len() as u64,
            LiveTile::Owned(v) => v.len() as u64,
        }
    }
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Rgba8 {
        match self {
            LiveTile::Absent => Rgba8::TRANSPARENT,
            LiveTile::Uniform(c) => *c,
            LiveTile::Shared(d) => Rgba8::from_slice(&d[index..index + 4]),
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
            LiveTile::Shared(d) => {
                *self = LiveTile::Owned(Arc::try_unwrap(d).unwrap_or_else(|d| (*d).clone()));
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
