//! 選択範囲（C# の SelectionMask・SelectionModify）。
//!
//! - 画素ごとに選ばれた量（0〜255）を、文書と同じ大きさの疎なタイルで持つ（無いタイルは選ばれていない）。ブラシ・塗りつぶし・
//!   消しゴムは、この量の割合でだけ画素を変える（[`crate::Document::set_selection`]）。
//! - [`SelectionMask`] は不変の札（中身は `Arc` で共有。写しは安い）。作る・組み合わせる・変える操作はどれも新しい札を返し、元を
//!   変えない。変わらないときは同じ札を返す（C# が `this` を返すのと同じ。文書は同じ札を置いても Undo の段を積まない）。
//! - 量は 1 画素 1 バイト（全画素が同じ量のタイルは 1 値）で持つ。C# は RGBA のタイルのアルファに持つので 4 倍の大きさになるが、
//!   履歴の重さ（[`SelectionMask::history_bytes`]）は C# と同じ数え方にして、同じ予算で同じ段が残るようにする。
//! - 式・丸め・作るタイルの並びは C# と同じで、出力はバイト一致を確かめている（tests/golden の select・modify の事例）。
//!   一致を確かめたのは同じ libm（Linux の glibc）の上だけで、別の libm（Windows など）の上では測っていない。libm の `exp` を
//!   通るのは、ぼかしの小さい半径（標準偏差 2 未満、半径 7 未満）の正確な核だけで、別の libm では 1 ULP ずれ得て、画素の量が
//!   1 違う可能性がある。形・組み合わせ・拡張・縮小・境界・鋭く・自動選択と、ぼかしの箱ぼかしの分岐は、加減乗除・平方根・
//!   floor/ceil・丸めだけを通る。
//! - 大きな作業の場所（変更のウィンドウ・自動選択の印と待ちの連の列）は、確保の前に見積もって作業の予算（[`DEFAULT_WORKING_BUDGET_BYTES`] など、
//!   呼び手が渡す）で断る（C# には無い）。
//!
//! ```
//! use yolu_core::{Document, SelectionCombine, SelectionMask};
//! let mut doc = Document::new(256, 256).unwrap();
//! let budget = yolu_core::selection::DEFAULT_WORKING_BUDGET_BYTES;
//! let ellipse = SelectionMask::ellipse(&doc, 128.0, 128.0, 60.0, 40.0).unwrap();
//! let hole = SelectionMask::rectangle(&doc, 120, 120, 136, 136);
//! let ring = ellipse.combine(&hole, SelectionCombine::Subtract).unwrap();
//! doc.set_selection(Some(ring.feather(4.0, false, budget).unwrap())).unwrap(); // 1 回の Undo
//! assert!(doc.selection().unwrap().amount(128, 128) == 0);
//! doc.undo().unwrap();
//! assert!(doc.selection().is_none());
//! ```

mod modify;
mod shapes;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rayon::prelude::*;

use crate::error::CoreError;
use crate::types::TileCoord;

pub use modify::MAX_MODIFY_RADIUS;
pub use shapes::MAX_POLYGON_POINTS;

/// 選択範囲の変更・自動選択の作業の場所の既定の予算（1 GiB）。8192² の全面の選択範囲で、拡張・縮小・境界は半径 200、ぼかしは
/// 半径 100 まで入る（ぼかし 200 は 1.08 GB 要るので超える。小さいキャンバスや狭い選択範囲なら 200 まで入る）。つながる自動選択は、待ちの連の列を
/// 最悪の長さで見るので、12288² までは入り、16384² は予算を上げる必要がある。
pub const DEFAULT_WORKING_BUDGET_BYTES: u64 = 1 << 30;

/// 新しい形と今の選択範囲の組み合わせ方（C# の SelectionCombine）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum SelectionCombine {
    /// 新しい形で置き換える。
    #[default]
    Replace,
    /// 大きい方の量（Shift）。
    Add,
    /// 新しい形の量だけ減らす（Ctrl）: a × (255 − b) / 255 を四捨五入。
    Subtract,
    /// 小さい方の量（Shift + Ctrl）。
    Intersect,
}

/// 1 枚のタイルの量。全部 0 のタイルは持たない（無いタイル）。
#[derive(Clone, Debug)]
pub(crate) enum Amounts {
    /// 全画素（キャンバスの外の余白も）が同じ量（0 でない）。
    Uniform(u8),
    /// TileSize² バイト（行優先、下の行から。キャンバスの外の余白は 0）。
    Data(Arc<Vec<u8>>),
}

impl Amounts {
    /// タイルの中の画素の番号（行 × TileSize + 列）の量。
    #[inline(always)]
    pub(crate) fn get(&self, index: usize) -> u8 {
        match self {
            Amounts::Uniform(v) => *v,
            Amounts::Data(d) => d[index],
        }
    }

    /// 持っているバイト数。
    fn byte_size(&self) -> u64 {
        match self {
            Amounts::Uniform(_) => 1,
            Amounts::Data(d) => d.len() as u64,
        }
    }

    /// C# の RGBA のタイルでの大きさ（一様は 4、ほかは TileSize² × 4）。
    fn csharp_bytes(&self) -> u64 {
        match self {
            Amounts::Uniform(_) => 4,
            Amounts::Data(d) => d.len() as u64 * 4,
        }
    }

    /// 量の並びのタイル（全部 0 なら無し、全部同じなら一様）。C# の TileStorage.FromBytes と同じ詰め方（余白も比べる）。
    fn from_bytes(bytes: Vec<u8>) -> Option<Amounts> {
        let first = bytes[0];
        if bytes.iter().all(|&a| a == first) {
            (first != 0).then_some(Amounts::Uniform(first))
        } else {
            Some(Amounts::Data(Arc::new(bytes)))
        }
    }

    fn copy_to(&self, out: &mut [u8]) {
        match self {
            Amounts::Uniform(v) => out.fill(*v),
            Amounts::Data(d) => out.copy_from_slice(d),
        }
    }
}

struct MaskData {
    width: u32,
    height: u32,
    tile_size: u32,
    tiles: HashMap<TileCoord, Amounts>,
}

/// 選択範囲（不変。写しは同じ中身を指す）。大きさとタイルの大きさは作った文書と同じ。
#[derive(Clone)]
pub struct SelectionMask(Arc<MaskData>);

impl std::fmt::Debug for SelectionMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SelectionMask({}×{}, タイル {}, {} 枚)",
            self.0.width,
            self.0.height,
            self.0.tile_size,
            self.0.tiles.len()
        )
    }
}

/// 量が同じか（札が違っても中身を比べる）。
impl PartialEq for SelectionMask {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        let (a, b) = (&self.0, &other.0);
        if a.width != b.width || a.height != b.height || a.tile_size != b.tile_size {
            return false;
        }
        let n = (a.tile_size * a.tile_size) as usize;
        let (mut x, mut y) = (vec![0u8; n], vec![0u8; n]);
        let coords: HashSet<TileCoord> = a.tiles.keys().chain(b.tiles.keys()).copied().collect();
        coords.into_iter().all(|c| {
            self.copy_amounts(c, &mut x);
            other.copy_amounts(c, &mut y);
            x == y
        })
    }
}

impl SelectionMask {
    // ───────── 作る（形は shapes.rs、変更は modify.rs） ─────────

    /// 何も選んでいない選択範囲（大きさ 1〜32768、タイル 1〜1024）。
    pub fn empty(width: u32, height: u32, tile_size: u32) -> Result<SelectionMask, CoreError> {
        if width == 0 || width > 32768 || height == 0 || height > 32768 {
            return Err(CoreError::InvalidArgument("選択範囲の大きさ"));
        }
        if tile_size == 0 || tile_size > 1024 {
            return Err(CoreError::InvalidArgument("選択範囲のタイルの大きさ"));
        }
        Ok(Self::blank(width, height, tile_size))
    }

    pub(crate) fn blank(width: u32, height: u32, tile_size: u32) -> SelectionMask {
        Self::from_map(width, height, tile_size, HashMap::new())
    }

    fn from_map(
        width: u32,
        height: u32,
        tile_size: u32,
        tiles: HashMap<TileCoord, Amounts>,
    ) -> SelectionMask {
        SelectionMask(Arc::new(MaskData {
            width,
            height,
            tile_size,
            tiles,
        }))
    }

    /// 保存したタイルの量（TileSize² バイト、行優先・下の行から）から作る（selection.bin の読み込み）。どのタイルもキャンバスの中に
    /// あり、重ならず、全部 0 でなく、キャンバスの外の余白が 0 でなければ断る（C# の SelectionBinary.Read が断るものと同じ）。
    pub fn from_amount_tiles(
        width: u32,
        height: u32,
        tile_size: u32,
        tiles: impl IntoIterator<Item = (TileCoord, Vec<u8>)>,
    ) -> Result<SelectionMask, CoreError> {
        let empty = Self::empty(width, height, tile_size)?;
        let mut map = HashMap::new();
        for (coord, amounts) in tiles {
            if !Self::check_tile(width, height, tile_size, coord, &amounts)? {
                return Err(CoreError::InvalidArgument("選択範囲のタイルが空"));
            }
            let tile = Amounts::from_bytes(amounts).expect("空でない");
            if map.insert(coord, tile).is_some() {
                return Err(CoreError::InvalidArgument("選択範囲のタイルが重なっている"));
            }
        }
        if map.is_empty() {
            return Ok(empty);
        }
        Ok(Self::from_map(width, height, tile_size, map))
    }

    /// タイル 1 枚の量の検査（キャンバスの中・長さ・キャンバスの外の余白が 0）。量のある画素が 1 つでもあれば true。
    fn check_tile(
        width: u32,
        height: u32,
        tile_size: u32,
        coord: TileCoord,
        amounts: &[u8],
    ) -> Result<bool, CoreError> {
        let ts = tile_size as usize;
        let (cols, rows) = (width.div_ceil(tile_size), height.div_ceil(tile_size));
        if coord.x >= cols || coord.y >= rows {
            return Err(CoreError::InvalidArgument(
                "選択範囲のタイルがキャンバスの外",
            ));
        }
        if amounts.len() != ts * ts {
            return Err(CoreError::InvalidArgument("選択範囲のタイルの長さ"));
        }
        let w = (width - coord.x * tile_size).min(tile_size) as usize;
        let h = (height - coord.y * tile_size).min(tile_size) as usize;
        let mut any = false;
        for (i, &a) in amounts.iter().enumerate() {
            if a == 0 {
                continue;
            }
            if i % ts >= w || i / ts >= h {
                return Err(CoreError::InvalidArgument(
                    "選択範囲のタイルの余白が 0 でない",
                ));
            }
            any = true;
        }
        Ok(any)
    }

    /// 一部のタイルの量だけを置き換えた選択範囲（ペンで描くストロークの途中の見え方の更新に使う。置き換えないタイルは同じ中身を
    /// 共有するので、置き換えた枚数に比例する）。タイルの検査は [`SelectionMask::from_amount_tiles`] と同じ（キャンバスの中・TileSize²
    /// バイト・キャンバスの外の余白が 0。同じタイルを 2 度渡せば後のものが残る）。全部 0 のタイルは「無いタイル」にする。
    pub fn with_tiles(
        &self,
        tiles: impl IntoIterator<Item = (TileCoord, Vec<u8>)>,
    ) -> Result<SelectionMask, CoreError> {
        let d = &self.0;
        let mut map = d.tiles.clone();
        for (coord, amounts) in tiles {
            if Self::check_tile(d.width, d.height, d.tile_size, coord, &amounts)? {
                map.insert(coord, Amounts::from_bytes(amounts).expect("空でない"));
            } else {
                map.remove(&coord);
            }
        }
        Ok(Self::from_map(d.width, d.height, d.tile_size, map))
    }

    // ───────── 読む ─────────

    pub fn width(&self) -> u32 {
        self.0.width
    }
    pub fn height(&self) -> u32 {
        self.0.height
    }
    pub fn tile_size(&self) -> u32 {
        self.0.tile_size
    }
    /// 何も選んでいないか。
    pub fn is_empty(&self) -> bool {
        self.0.tiles.is_empty()
    }
    /// 同じ札か（中身ではなく、同じものを指すか。C# の ReferenceEquals）。
    pub fn same_as(&self, other: &SelectionMask) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// 持っている量のバイト数（1 画素 1 バイト、一様なタイルは 1）。
    pub fn allocated_bytes(&self) -> u64 {
        self.0.tiles.values().map(Amounts::byte_size).sum()
    }
    /// 履歴の重さに数える大きさ（C# の SelectionMask.AllocatedBytes と同じ: RGBA のタイルでの大きさ）。
    pub fn history_bytes(&self) -> u64 {
        self.0.tiles.values().map(Amounts::csharp_bytes).sum()
    }
    /// 画素 (x, y) の量（0〜255）。キャンバスの外は 0。
    pub fn amount(&self, x: u32, y: u32) -> u8 {
        let d = &self.0;
        if x >= d.width || y >= d.height {
            return 0;
        }
        let ts = d.tile_size;
        let coord = TileCoord::new(x / ts, y / ts);
        d.tiles
            .get(&coord)
            .map_or(0, |t| t.get(((y % ts) * ts + x % ts) as usize))
    }
    /// 量のあるタイルの座標（Y、次に X の順）。
    pub fn tile_coords(&self) -> Vec<TileCoord> {
        let mut v: Vec<TileCoord> = self.0.tiles.keys().copied().collect();
        v.sort();
        v
    }
    /// タイルの全画素の量を out（TileSize² バイト）へ。タイルが無ければ 0 で埋めて false。
    pub fn copy_tile(&self, coord: TileCoord, out: &mut [u8]) -> Result<bool, CoreError> {
        let ts = self.0.tile_size;
        if out.len() != (ts * ts) as usize {
            return Err(CoreError::InvalidArgument("タイルの長さ"));
        }
        if coord.x >= self.0.width.div_ceil(ts) || coord.y >= self.0.height.div_ceil(ts) {
            return Err(CoreError::InvalidArgument("タイルがキャンバスの外"));
        }
        Ok(self.copy_amounts(coord, out))
    }
    fn copy_amounts(&self, coord: TileCoord, out: &mut [u8]) -> bool {
        match self.0.tiles.get(&coord) {
            Some(t) => {
                t.copy_to(out);
                true
            }
            None => {
                out.fill(0);
                false
            }
        }
    }
    /// キャンバス全体の量（幅 × 高さ バイト、下の行から）。表示の覆いや試験に。
    pub fn to_canvas_bytes(&self) -> Vec<u8> {
        let d = &self.0;
        let (w, ts) = (d.width as usize, d.tile_size as usize);
        let mut out = vec![0u8; w * d.height as usize];
        for (coord, t) in &d.tiles {
            let (x0, y0) = (coord.x as usize * ts, coord.y as usize * ts);
            let cw = ts.min(w - x0);
            let ch = ts.min(d.height as usize - y0);
            for y in 0..ch {
                let row = &mut out[(y0 + y) * w + x0..(y0 + y) * w + x0 + cw];
                match t {
                    Amounts::Uniform(v) => row.fill(*v),
                    Amounts::Data(a) => row.copy_from_slice(&a[y * ts..y * ts + cw]),
                }
            }
        }
        out
    }
    /// 量のあるタイルを全部含む画素の矩形（x0, y0, x1, y1 は半開区間。キャンバスで切る）。何も無ければ None。
    pub fn tile_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        self.window(0).map(|w| {
            (
                w.x as u32,
                w.y as u32,
                (w.x + w.width) as u32,
                (w.y + w.height) as u32,
            )
        })
    }

    pub(crate) fn tile(&self, coord: TileCoord) -> Option<&Amounts> {
        self.0.tiles.get(&coord)
    }

    fn same_size(&self, other: &SelectionMask) -> bool {
        self.0.width == other.0.width
            && self.0.height == other.0.height
            && self.0.tile_size == other.0.tile_size
    }

    // ───────── 組み合わせ・反転・鋭く ─────────

    /// 反転（キャンバスの中の量を 255 − 量に。全タイルを作る）。
    pub fn invert(&self) -> SelectionMask {
        self.map_tiles(None, true, |a, _| 255 - a)
    }

    /// other（同じ大きさ）と組み合わせる: Add は大きい方、Subtract は other の分を減らす、Intersect は小さい方、Replace は other。
    pub fn combine(
        &self,
        other: &SelectionMask,
        mode: SelectionCombine,
    ) -> Result<SelectionMask, CoreError> {
        if !self.same_size(other) {
            return Err(CoreError::InvalidArgument("選択範囲の大きさが違う"));
        }
        Ok(match mode {
            SelectionCombine::Replace => other.clone(),
            SelectionCombine::Add => self.map_tiles(Some(other), false, |a, b| a.max(b)),
            SelectionCombine::Subtract => self.map_tiles(Some(other), false, |a, b| {
                ((a as u32 * (255 - b as u32) + 127) / 255) as u8
            }),
            SelectionCombine::Intersect => self.map_tiles(Some(other), false, |a, b| a.min(b)),
        })
    }

    /// 半分以上の量を全部に、ほかを 0 に（GIMP の Sharpen）。
    pub fn sharpen(&self) -> SelectionMask {
        self.map_tiles(None, false, |a, _| if a >= 128 { 255 } else { 0 })
    }

    /// 2 つの選択範囲のタイルから、画素ごとの式で作る（C# の FromTiles。無いタイルは 0 と読み、キャンバスの外の余白は 0 のまま）。
    /// every_tile ならキャンバスの全タイル、でなければどちらかにあるタイルだけ。タイルごとに並列。
    fn map_tiles<F>(&self, other: Option<&SelectionMask>, every_tile: bool, f: F) -> SelectionMask
    where
        F: Fn(u8, u8) -> u8 + Sync,
    {
        let d = &self.0;
        let (w, h, ts) = (d.width, d.height, d.tile_size);
        let coords: Vec<TileCoord> = if every_tile {
            (0..h.div_ceil(ts))
                .flat_map(|y| (0..w.div_ceil(ts)).map(move |x| TileCoord::new(x, y)))
                .collect()
        } else {
            let mut set: HashSet<TileCoord> = d.tiles.keys().copied().collect();
            if let Some(o) = other {
                set.extend(o.0.tiles.keys().copied());
            }
            set.into_iter().collect()
        };
        let n = (ts * ts) as usize;
        let tiles: Vec<(TileCoord, Option<Amounts>)> = coords
            .into_par_iter()
            .with_min_len(modify::min_tiles(ts as usize))
            .map(|coord| {
                let mut ta = vec![0u8; n];
                let mut tb = vec![0u8; n];
                self.copy_amounts(coord, &mut ta);
                if let Some(o) = other {
                    o.copy_amounts(coord, &mut tb);
                }
                let mut out = vec![0u8; n];
                let cw = (w - coord.x * ts).min(ts) as usize;
                let ch = (h - coord.y * ts).min(ts) as usize;
                let ts = ts as usize;
                for y in 0..ch {
                    for x in 0..cw {
                        let i = y * ts + x;
                        out[i] = f(ta[i], tb[i]);
                    }
                }
                (coord, Amounts::from_bytes(out))
            })
            .collect();
        Self::collect(w, h, ts, tiles)
    }

    fn collect(
        width: u32,
        height: u32,
        tile_size: u32,
        tiles: Vec<(TileCoord, Option<Amounts>)>,
    ) -> SelectionMask {
        let map: HashMap<TileCoord, Amounts> = tiles
            .into_iter()
            .filter_map(|(c, t)| t.map(|t| (c, t)))
            .collect();
        Self::from_map(width, height, tile_size, map)
    }

    /// [x0, x1) × [y0, y1)（キャンバスで切る）の画素の量を amount(x, y) で決めて作る（C# の Build）。タイルごとに並列なので、amount は
    /// 純粋な関数（共有の状態を持たない）であること。
    pub(crate) fn build<F>(
        width: u32,
        height: u32,
        tile_size: u32,
        bounds: (i64, i64, i64, i64),
        amount: F,
    ) -> SelectionMask
    where
        F: Fn(i64, i64) -> u8 + Sync,
    {
        Self::build_tiles(width, height, tile_size, bounds, |_, xs, ys, out| {
            let ts = tile_size as i64;
            let mut any = false;
            for y in ys.0..ys.1 {
                let row = ((y % ts) * ts) as usize;
                for x in xs.0..xs.1 {
                    let a = amount(x, y);
                    if a != 0 {
                        out[row + (x % ts) as usize] = a;
                        any = true;
                    }
                }
            }
            any
        })
    }

    /// build のタイルごとの形: tile(coord, (x0, x1), (y0, y1), out) がタイルの中の範囲の量を out（TileSize²、0 で始まる）へ書き、
    /// 何か書いたら true を返す。
    pub(crate) fn build_tiles<F>(
        width: u32,
        height: u32,
        tile_size: u32,
        bounds: (i64, i64, i64, i64),
        tile: F,
    ) -> SelectionMask
    where
        F: Fn(TileCoord, (i64, i64), (i64, i64), &mut [u8]) -> bool + Sync,
    {
        let (w, h, ts) = (width as i64, height as i64, tile_size as i64);
        let x0 = bounds.0.max(0);
        let y0 = bounds.1.max(0);
        let x1 = bounds.2.min(w);
        let y1 = bounds.3.min(h);
        if x0 >= x1 || y0 >= y1 {
            return Self::blank(width, height, tile_size);
        }
        let mut coords = Vec::new();
        for ty in y0 / ts..=(y1 - 1) / ts {
            for tx in x0 / ts..=(x1 - 1) / ts {
                coords.push(TileCoord::new(tx as u32, ty as u32));
            }
        }
        let n = (ts * ts) as usize;
        let tiles: Vec<(TileCoord, Option<Amounts>)> = coords
            .into_par_iter()
            .map(|c| {
                let (tx, ty) = (c.x as i64, c.y as i64);
                let xs = (x0.max(tx * ts), x1.min((tx + 1) * ts));
                let ys = (y0.max(ty * ts), y1.min((ty + 1) * ts));
                let mut out = vec![0u8; n];
                let any = tile(c, xs, ys, &mut out);
                (c, if any { Amounts::from_bytes(out) } else { None })
            })
            .collect();
        Self::collect(width, height, tile_size, tiles)
    }

    /// 量のあるタイルを全部含む矩形を margin 広げてキャンバスで切ったもの（C# の Window）。何も無ければ None。
    pub(crate) fn window(&self, margin: i64) -> Option<Window> {
        let d = &self.0;
        let (w, h, t) = (d.width as i64, d.height as i64, d.tile_size as i64);
        let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        for c in d.tiles.keys() {
            let (cx, cy) = (c.x as i64, c.y as i64);
            x0 = x0.min(cx * t);
            y0 = y0.min(cy * t);
            x1 = x1.max(w.min((cx + 1) * t));
            y1 = y1.max(h.min((cy + 1) * t));
        }
        if x0 == i64::MAX {
            return None;
        }
        let x0 = (x0 - margin).max(0);
        let y0 = (y0 - margin).max(0);
        let x1 = (x1 + margin).min(w);
        let y1 = (y1 + margin).min(h);
        Some(Window {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }

    /// ウィンドウの中の量（ウィンドウの一番下の行から、行優先）。
    pub(crate) fn dense(&self, w: Window) -> Vec<u8> {
        let d = &self.0;
        let t = d.tile_size as i64;
        let mut dense = vec![0u8; (w.width * w.height) as usize];
        for (c, tile) in &d.tiles {
            let (cx, cy) = (c.x as i64, c.y as i64);
            let tx0 = w.x.max(cx * t);
            let tx1 = (w.x + w.width).min((cx + 1) * t);
            let ty0 = w.y.max(cy * t);
            let ty1 = (w.y + w.height).min((cy + 1) * t);
            if tx0 >= tx1 || ty0 >= ty1 {
                continue;
            }
            for y in ty0..ty1 {
                let out = ((y - w.y) * w.width + tx0 - w.x) as usize;
                let len = (tx1 - tx0) as usize;
                match tile {
                    Amounts::Uniform(v) => dense[out..out + len].fill(*v),
                    Amounts::Data(a) => {
                        let src = ((y - cy * t) * t + tx0 - cx * t) as usize;
                        dense[out..out + len].copy_from_slice(&a[src..src + len]);
                    }
                }
            }
        }
        dense
    }

    /// ウィンドウの量から同じ大きさの新しい選択範囲を作る（C# の FromDense）。
    pub(crate) fn with_dense(&self, dense: &[u8], w: Window) -> SelectionMask {
        let d = &self.0;
        let t = d.tile_size as i64;
        let mut coords = Vec::new();
        for ty in w.y / t..=(w.y + w.height - 1) / t {
            for tx in w.x / t..=(w.x + w.width - 1) / t {
                coords.push((tx, ty));
            }
        }
        let n = (t * t) as usize;
        let tiles: Vec<(TileCoord, Option<Amounts>)> = coords
            .into_par_iter()
            .with_min_len(modify::min_tiles(t as usize))
            .map(|(tx, ty)| {
                let mut out = vec![0u8; n];
                let x0 = w.x.max(tx * t);
                let x1 = (w.x + w.width).min((tx + 1) * t);
                let y0 = w.y.max(ty * t);
                let y1 = (w.y + w.height).min((ty + 1) * t);
                let mut any = false;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let a = dense[((y - w.y) * w.width + x - w.x) as usize];
                        if a != 0 {
                            out[((y - ty * t) * t + x - tx * t) as usize] = a;
                            any = true;
                        }
                    }
                }
                let coord = TileCoord::new(tx as u32, ty as u32);
                (coord, if any { Amounts::from_bytes(out) } else { None })
            })
            .collect();
        Self::collect(d.width, d.height, d.tile_size, tiles)
    }
}

/// キャンバスの中の画素のウィンドウ（左下の画素と大きさ）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// 作業の場所の見積もりを予算と比べる。
pub(crate) fn ensure_working(bytes: u128, budget: u64) -> Result<(), CoreError> {
    if bytes > budget as u128 {
        Err(CoreError::WorkingBudgetExceeded)
    } else {
        Ok(())
    }
}
