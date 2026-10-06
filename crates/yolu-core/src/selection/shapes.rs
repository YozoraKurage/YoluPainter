//! 選択範囲の形: 全選択・矩形・楕円・投げ縄と多角形・自動選択（C# の Selection.cs）。

use std::collections::HashMap;

use glam::DVec2;
use rayon::prelude::*;

use super::{ensure_working, SelectionMask};
use crate::document::Document;
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::math::require_finite;
use crate::types::{Channel, LayerKind, Rect, Rgba8, RowOrder, TileCoord};

/// 1 画素を 4 × 4 の点で見る（楕円・投げ縄の縁の滑らかさ）。
const SUPERSAMPLE: i64 = 4;
/// 投げ縄・多角形の点の数の上限（C# と同じ）。
pub const MAX_POLYGON_POINTS: usize = 100_000;

/// 4 × 4 の点のうち中にある数を 0〜255 の量へ（C# と同じ四捨五入）。
#[inline]
fn coverage(inside: i64) -> u8 {
    ((inside * 255 + SUPERSAMPLE * SUPERSAMPLE / 2) / (SUPERSAMPLE * SUPERSAMPLE)) as u8
}

/// 小数の座標を整数へ（C# の (int) は範囲の外で未定義なので、i64 へ飽和させる。画布で切るので結果は同じ）。
#[inline]
fn floor(v: f64) -> i64 {
    v.floor() as i64
}
#[inline]
fn ceil(v: f64) -> i64 {
    v.ceil() as i64
}

impl SelectionMask {
    /// 何も選ばない（文書の大きさ）。
    pub fn none(doc: &Document) -> SelectionMask {
        Self::blank(doc.width(), doc.height(), doc.tile_size())
    }

    /// 全部を選ぶ。
    pub fn all(doc: &Document) -> SelectionMask {
        let (w, h) = (doc.width() as i64, doc.height() as i64);
        Self::build(
            doc.width(),
            doc.height(),
            doc.tile_size(),
            (0, 0, w, h),
            |_, _| 255,
        )
    }

    /// 中心が [x0, x1) × [y0, y1) にある画素（画布の画素の座標、左下原点）。逆向きの角は入れ替える。
    pub fn rectangle(doc: &Document, x0: i64, y0: i64, x1: i64, y1: i64) -> SelectionMask {
        let (x0, x1) = if x1 < x0 { (x1, x0) } else { (x0, x1) };
        let (y0, y1) = if y1 < y0 { (y1, y0) } else { (y0, y1) };
        Self::build(
            doc.width(),
            doc.height(),
            doc.tile_size(),
            (x0, y0, x1, y1),
            |_, _| 255,
        )
    }

    /// 縁を滑らかにした楕円（画素ごとに 4 × 4 の点）。中心と半径は画素で、半径は絶対値を使い、0 なら何も選ばない。
    pub fn ellipse(
        doc: &Document,
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    ) -> Result<SelectionMask, CoreError> {
        require_finite(cx, "cx")?;
        require_finite(cy, "cy")?;
        require_finite(rx, "rx")?;
        require_finite(ry, "ry")?;
        let (rx, ry) = (rx.abs(), ry.abs());
        if rx <= 0.0 || ry <= 0.0 {
            return Ok(Self::none(doc));
        }
        let bounds = (
            floor(cx - rx),
            floor(cy - ry),
            ceil(cx + rx).saturating_add(1),
            ceil(cy + ry).saturating_add(1),
        );
        Ok(Self::build(
            doc.width(),
            doc.height(),
            doc.tile_size(),
            bounds,
            |x, y| {
                let mut inside = 0;
                for sy in 0..SUPERSAMPLE {
                    for sx in 0..SUPERSAMPLE {
                        let u = (x as f64 + (sx as f64 + 0.5) / SUPERSAMPLE as f64 - cx) / rx;
                        let v = (y as f64 + (sy as f64 + 0.5) / SUPERSAMPLE as f64 - cy) / ry;
                        if u * u + v * v <= 1.0 {
                            inside += 1;
                        }
                    }
                }
                coverage(inside)
            },
        ))
    }

    /// 縁を滑らかにした多角形（投げ縄）。偶奇の規則、画素ごとに 4 × 4 の点。点は画布の座標で、3 つ未満なら何も選ばない。
    ///
    /// C# は点ごとに全部の辺を数えるが、ここでは点の行ごとに、その高さをまたぐ辺の交点の x（C# と同じ式の double）を並べて
    /// おき、点より右にある交点の数の偶奇を二分探索で数える（同じ数なので結果は同じ。点の多い投げ縄でも速い）。
    pub fn polygon(doc: &Document, points: &[DVec2]) -> Result<SelectionMask, CoreError> {
        if points.len() < 3 {
            return Ok(Self::none(doc));
        }
        if points.len() > MAX_POLYGON_POINTS {
            return Err(CoreError::InvalidArgument(
                "多角形の点が多すぎる（100000 まで）",
            ));
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in points {
            require_finite(p.x, "x")?;
            require_finite(p.y, "y")?;
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        let bounds = (
            floor(min_x),
            floor(min_y),
            ceil(max_x).saturating_add(1),
            ceil(max_y).saturating_add(1),
        );
        let ts = doc.tile_size() as i64;
        Ok(Self::build_tiles(
            doc.width(),
            doc.height(),
            doc.tile_size(),
            bounds,
            |_, xs, ys, out| {
                let mut any = false;
                let mut crossings: Vec<f64> = Vec::new();
                for y in ys.0..ys.1 {
                    // この画素の行の 4 つの点の行ごとに、交点の x を並べる
                    let mut rows: [Vec<f64>; SUPERSAMPLE as usize] = Default::default();
                    for (sy, row) in rows.iter_mut().enumerate() {
                        let py = y as f64 + (sy as f64 + 0.5) / SUPERSAMPLE as f64;
                        crossings.clear();
                        let mut j = points.len() - 1;
                        for i in 0..points.len() {
                            let (a, b) = (points[i], points[j]);
                            if (a.y > py) != (b.y > py) {
                                let xc = (b.x - a.x) * (py - a.y) / (b.y - a.y) + a.x;
                                if !xc.is_nan() {
                                    crossings.push(xc); // px < NaN は偽なので、NaN は数えない
                                }
                            }
                            j = i;
                        }
                        crossings.sort_by(f64::total_cmp);
                        row.extend_from_slice(&crossings);
                    }
                    let line = ((y % ts) * ts) as usize;
                    for x in xs.0..xs.1 {
                        let mut count = 0;
                        for row in &rows {
                            for sx in 0..SUPERSAMPLE {
                                let px = x as f64 + (sx as f64 + 0.5) / SUPERSAMPLE as f64;
                                // px より右（px < 交点）の交点の数が奇数なら中
                                let right = row.len() - row.partition_point(|&c| c <= px);
                                if right % 2 == 1 {
                                    count += 1;
                                }
                            }
                        }
                        let a = coverage(count);
                        if a != 0 {
                            out[line + (x % ts) as usize] = a;
                            any = true;
                        }
                    }
                }
                any
            },
        ))
    }

    /// 自動選択（とバケツの範囲）: 種の画素の色から、各成分（RGBA）の差がどれも許し幅（0〜255）以下の画素。基準は層の
    /// そのチャンネルの画素（layer）か、チャンネルの合成（None）。contiguous なら種から 4 近傍でつながる所だけ、でなければ
    /// 画布全体の合う画素。縁は 0 か 255。作業の場所（選んだ印・読んだタイルの合うかの印・つながるときの待ちの連の列の最悪の長さ）は
    /// 確保の前に見積もり、budget を超えるなら断る（8192² で 300 MB ほど。列は画像の中身によらず最悪で見る）。
    #[allow(clippy::too_many_arguments)]
    pub fn magic_wand(
        doc: &Document,
        layer: Option<LayerId>,
        channel: Channel,
        seed_x: u32,
        seed_y: u32,
        tolerance: u8,
        contiguous: bool,
        budget: u64,
    ) -> Result<SelectionMask, CoreError> {
        doc.require_channel(channel)?;
        let source = match layer {
            Some(id) => Some(doc.layer(id).ok_or(CoreError::LayerNotFound)?),
            None => None,
        };
        let (w, h, ts) = (doc.width(), doc.height(), doc.tile_size());
        if seed_x >= w || seed_y >= h {
            return Err(CoreError::InvalidArgument("種がキャンバスの外"));
        }
        let reader = Reference {
            doc,
            layer: source,
            channel,
        };
        let seed = reader.pixel(seed_x, seed_y)?;
        let tol = tolerance as i32;
        let matches = move |c: Rgba8| {
            (c.r as i32 - seed.r as i32).abs() <= tol
                && (c.g as i32 - seed.g as i32).abs() <= tol
                && (c.b as i32 - seed.b as i32).abs() <= tol
                && (c.a as i32 - seed.a as i32).abs() <= tol
        };
        let tile_bytes = ts as u128 * ts as u128 * 4;
        if !contiguous {
            // 作業: 帯（合成のとき、幅 × タイル 1 行）とタイルごとの読み込み
            let band = if source.is_none() {
                w as u128 * ts as u128 * 4
            } else {
                0
            };
            ensure_working(
                band + tile_bytes * rayon::current_num_threads() as u128,
                budget,
            )?;
            // ディスクから読めないタイルがあれば、選択範囲を作らずに誤りを返す
            let failed = std::sync::OnceLock::new();
            let mask = match source {
                Some(l) => Self::build_tiles(
                    w,
                    h,
                    ts,
                    (0, 0, w as i64, h as i64),
                    |coord, xs, ys, out| {
                        let mut bytes = vec![0u8; tile_bytes as usize];
                        if let Err(e) = layer_tile(l, channel, w, h, ts, coord, &mut bytes) {
                            let _ = failed.set(e);
                            return false;
                        }
                        let t = ts as i64;
                        let (bx, by) = (coord.x as i64 * t, coord.y as i64 * t);
                        let mut any = false;
                        for y in ys.0..ys.1 {
                            for x in xs.0..xs.1 {
                                let i = ((y - by) * t + x - bx) as usize;
                                if matches(Rgba8::from_slice(&bytes[i * 4..i * 4 + 4])) {
                                    out[i] = 255;
                                    any = true;
                                }
                            }
                        }
                        any
                    },
                ),
                None => wand_composite_everywhere(doc, channel, &matches)?,
            };
            return match failed.into_inner() {
                Some(e) => Err(e),
                None => Ok(mask),
            };
        }
        // 走査線の塗りつぶし: 種から 4 近傍でつながる、条件に合う画素の集まり（どの順で辿っても同じ集まりになる）。
        // 作業は、選んだ印・タイルごとの「合うか」の印・待ちの連の列（最悪の長さで見積もる。下の `max_pending_runs`）
        let (wu, hu) = (w as usize, h as usize);
        ensure_working(contiguous_bytes(w, h, ts), budget)?;
        let filled = fill_contiguous(&reader, &matches, wu, hu, ts as usize, (seed_x, seed_y))?;
        let (selected, bounds) = (filled.selected, filled.bounds);
        Ok(Self::build(w, h, ts, bounds, |x, y| {
            if selected.get(y as usize * wu + x as usize) {
                255
            } else {
                0
            }
        }))
    }
}

/// 走査線の塗りつぶしの結果。
struct Filled {
    selected: Bits,
    /// 選ばれた画素の外接の箱（x0, y0, x1, y1。右と上は含まない）。
    bounds: (i64, i64, i64, i64),
    /// 待ちの連の数の最大（試験が見積もりの上限と比べる）。
    #[cfg_attr(not(test), allow(dead_code))]
    peak_pending: usize,
}

/// 待ちの連（印は付けたが、上下の行をまだ見ていない連）の数の上限。連は行の中の合う画素の最大の並びで、横に隣り合う 2 つの連の
/// 間には合わない画素が 1 つ以上あるので、1 行に (幅 + 1) / 2 本まで。
fn max_pending_runs(width: usize, height: usize) -> u128 {
    width.div_ceil(2) as u128 * height as u128
}

/// つながる自動選択の作業の場所の見積もり（バイト）: 選んだ印（1 画素 1 ビット）・タイルごとの合うかの印（全タイルを読む最悪）・
/// タイル 1 枚の読み込み・待ちの連の列の最悪。
fn contiguous_bytes(width: u32, height: u32, tile_size: u32) -> u128 {
    let tiles = width.div_ceil(tile_size) as u128 * height.div_ceil(tile_size) as u128;
    let ts = tile_size as u128;
    (width as u128 * height as u128).div_ceil(8)
        + tiles * (ts * ts).div_ceil(8)
        + ts * ts * 4
        + pending_bytes(width as usize, height as usize)
}

/// 待ちの連の列（連の左端の画素の番号を u32 で）の最悪のバイト数。倍々に確保するので 2 倍で見る。
fn pending_bytes(width: usize, height: usize) -> u128 {
    max_pending_runs(width, height) * std::mem::size_of::<u32>() as u128 * 2
}

/// 種から 4 近傍でつながる合う画素を選ぶ。連を見つけたらその場で全体（左右の端まで）に印を付けて待ちの列へ入れ、上下の行は
/// 取り出してから見る。印を付けた連は二度と見つからないので、どの連も 1 度しか入らず、列は max_pending_runs を超えない
/// （入れるのは連の左端の画素の番号だけで、取り出すときに印のついた並びの右端を数え直す）。画素の番号は、画布が 32768² までなので
/// u32 に収まる。
fn fill_contiguous<F: Fn(Rgba8) -> bool>(
    reader: &Reference,
    matches: &F,
    width: usize,
    height: usize,
    tile_size: usize,
    seed: (u32, u32),
) -> Result<Filled, CoreError> {
    let mut flood = Flood {
        lookup: MatchTiles {
            reader,
            matches,
            tiles: HashMap::new(),
            tile_size,
            scratch: vec![0u8; tile_size * tile_size * 4],
        },
        width,
        filled: Filled {
            selected: Bits::new(width * height),
            bounds: (i64::MAX, i64::MAX, i64::MIN, i64::MIN),
            peak_pending: 0,
        },
    };
    let mut pending: Vec<u32> = Vec::new();
    let (seed_x, seed_y) = (seed.0 as usize, seed.1 as usize);
    let left = flood.take_run(seed_x, seed_y)?;
    pending.push((seed_y * width + left) as u32);
    flood.filled.peak_pending = 1;
    while let Some(i) = pending.pop() {
        let (left, y) = (i as usize % width, i as usize / width);
        let mut right = left;
        while right < width - 1 && flood.filled.selected.get(y * width + right + 1) {
            right += 1;
        }
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= height {
                continue; // 0 の下は wrapping で大きくなる
            }
            for k in left..=right {
                if !flood.filled.selected.get(ny * width + k) && flood.lookup.at(k, ny)? {
                    let run_left = flood.take_run(k, ny)?;
                    pending.push((ny * width + run_left) as u32);
                    flood.filled.peak_pending = flood.filled.peak_pending.max(pending.len());
                }
            }
        }
    }
    Ok(flood.filled)
}

/// 塗りつぶしの作業中の状態（合うかの印と、選んだ印）。
struct Flood<'a, F> {
    lookup: MatchTiles<'a, F>,
    width: usize,
    filled: Filled,
}

impl<F: Fn(Rgba8) -> bool> Flood<'_, F> {
    /// (x, y)（合う画素で、まだ選ばれていない）を含む合う連の全体に印を付け、左端の x を返す。
    /// 選ばれた画素は連ごとに全部印が付いているので、隣の画素が選ばれていれば同じ連の中で、連の外へ延びることはない。
    fn take_run(&mut self, x: usize, y: usize) -> Result<usize, CoreError> {
        let (mut left, mut right) = (x, x);
        while left > 0 && self.lookup.at(left - 1, y)? {
            left -= 1;
        }
        while right < self.width - 1 && self.lookup.at(right + 1, y)? {
            right += 1;
        }
        for k in left..=right {
            self.filled.selected.set(y * self.width + k);
        }
        let b = &mut self.filled.bounds;
        *b = (
            b.0.min(left as i64),
            b.1.min(y as i64),
            b.2.max(right as i64 + 1),
            b.3.max(y as i64 + 1),
        );
        Ok(left)
    }
}

/// 自動選択の基準（層の画素か、チャンネルの合成）。
struct Reference<'a> {
    doc: &'a Document,
    layer: Option<&'a Layer>,
    channel: Channel,
}

impl Reference<'_> {
    /// タイル 1 枚の基準の画素（TileSize² × 4、画布の外の余白は 0）。
    fn tile(&self, coord: TileCoord, out: &mut [u8]) -> Result<(), CoreError> {
        let (w, h, ts) = (self.doc.width(), self.doc.height(), self.doc.tile_size());
        match self.layer {
            Some(l) => layer_tile(l, self.channel, w, h, ts, coord, out)?,
            None => {
                let rect = self.doc.tile_rect(coord).expect("キャンバスの中のタイル");
                let mut region = vec![0u8; rect.width as usize * rect.height as usize * 4];
                self.doc
                    .composite_into(self.channel, rect, &mut region, RowOrder::BottomUp)?;
                out.fill(0);
                let (tw, t) = (rect.width as usize * 4, ts as usize * 4);
                for r in 0..rect.height as usize {
                    out[r * t..r * t + tw].copy_from_slice(&region[r * tw..r * tw + tw]);
                }
            }
        }
        Ok(())
    }

    fn pixel(&self, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        let ts = self.doc.tile_size();
        let mut bytes = vec![0u8; ts as usize * ts as usize * 4];
        self.tile(TileCoord::new(x / ts, y / ts), &mut bytes)?;
        let i = (((y % ts) * ts + x % ts) * 4) as usize;
        Ok(Rgba8::from_slice(&bytes[i..i + 4]))
    }
}

/// 層そのものの画素（C# の PaintLayer.CopyTile）: ラスターは面、塗りつぶしは値（画布の中だけ）、調整・グループは無し。
fn layer_tile(
    layer: &Layer,
    channel: Channel,
    width: u32,
    height: u32,
    ts: u32,
    coord: TileCoord,
    out: &mut [u8],
) -> Result<(), CoreError> {
    out.fill(0);
    match layer.kind() {
        LayerKind::Raster => {
            if let Some(s) = layer.surface(channel) {
                s.copy_tile(coord, out)?;
            }
        }
        LayerKind::Fill => {
            let Some(v) = layer
                .fill_value(channel)
                .filter(|v| *v != Rgba8::TRANSPARENT)
            else {
                return Ok(());
            };
            let w = (width - coord.x * ts).min(ts) as usize;
            let h = (height - coord.y * ts).min(ts) as usize;
            let t = ts as usize;
            for y in 0..h {
                for p in out[y * t * 4..(y * t + w) * 4].chunks_exact_mut(4) {
                    p.copy_from_slice(&v.to_array());
                }
            }
        }
        LayerKind::Adjustment | LayerKind::Group => {}
    }
    Ok(())
}

/// 合成が基準の、つながりを見ない自動選択: タイル 1 行の帯ずつ合成し（合成は中で並列）、帯の中のタイルを並列に調べる。
fn wand_composite_everywhere<F>(
    doc: &Document,
    channel: Channel,
    matches: &F,
) -> Result<SelectionMask, CoreError>
where
    F: Fn(Rgba8) -> bool + Sync,
{
    let (w, h, ts) = (doc.width(), doc.height(), doc.tile_size());
    let mut map = HashMap::new();
    let n = ts as usize * ts as usize;
    for ty in 0..h.div_ceil(ts) {
        let y0 = ty * ts;
        let rows = ts.min(h - y0);
        let mut band = vec![0u8; w as usize * rows as usize * 4];
        doc.composite_into(
            channel,
            Rect::new(0, y0, w, rows),
            &mut band,
            RowOrder::BottomUp,
        )?;
        let tiles: Vec<(TileCoord, Option<super::Amounts>)> = (0..w.div_ceil(ts))
            .into_par_iter()
            .map(|tx| {
                let mut out = vec![0u8; n];
                let mut any = false;
                let x0 = (tx * ts) as usize;
                let x1 = ((tx + 1) * ts).min(w) as usize;
                for y in 0..rows as usize {
                    for x in x0..x1 {
                        let o = (y * w as usize + x) * 4;
                        if matches(Rgba8::from_slice(&band[o..o + 4])) {
                            out[y * ts as usize + x - x0] = 255;
                            any = true;
                        }
                    }
                }
                let coord = TileCoord::new(tx, ty);
                (
                    coord,
                    if any {
                        super::Amounts::from_bytes(out)
                    } else {
                        None
                    },
                )
            })
            .collect();
        map.extend(tiles.into_iter().filter_map(|(c, t)| t.map(|t| (c, t))));
    }
    Ok(SelectionMask::from_map(w, h, ts, map))
}

/// 画素ごとの印（ビットの並び）。
struct Bits(Vec<u64>);
impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    #[inline]
    fn get(&self, i: usize) -> bool {
        self.0[i >> 6] >> (i & 63) & 1 != 0
    }
    #[inline]
    fn set(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }
}

/// つながる自動選択の、タイルごとの「合うか」の印（初めて読むときに作る）。
struct MatchTiles<'a, F> {
    reader: &'a Reference<'a>,
    matches: &'a F,
    tiles: HashMap<TileCoord, Bits>,
    tile_size: usize,
    scratch: Vec<u8>,
}

impl<F: Fn(Rgba8) -> bool> MatchTiles<'_, F> {
    #[inline]
    fn at(&mut self, x: usize, y: usize) -> Result<bool, CoreError> {
        let ts = self.tile_size;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        let local = (y % ts) * ts + x % ts;
        if let Some(b) = self.tiles.get(&coord) {
            return Ok(b.get(local));
        }
        self.reader.tile(coord, &mut self.scratch)?;
        let mut bits = Bits::new(ts * ts);
        for (i, p) in self.scratch.chunks_exact(4).enumerate() {
            if (self.matches)(Rgba8::from_slice(p)) {
                bits.set(i);
            }
        }
        let r = bits.get(local);
        self.tiles.insert(coord, bits);
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rgba8 = Rgba8::new(255, 0, 0, 255);
    const B: Rgba8 = Rgba8::new(0, 0, 255, 255);

    /// 画素 (x, y) が A（でなければ B）の画像で、(0, 0) の種からつながる A を塗りつぶす。
    /// 返すのは、選ばれた画素の数・待ちの連の数の最大・総当たり（幅優先）の答えと同じか。
    fn fill_pattern(
        w: u32,
        h: u32,
        ts: u32,
        is_a: impl Fn(u32, u32) -> bool,
    ) -> (usize, usize, bool) {
        let mut doc = Document::with_tile_size(w, h, ts).unwrap();
        let id = doc.add_layer("a").unwrap();
        for y in 0..h {
            for x in 0..w {
                doc.set_pixel(id, x, y, if is_a(x, y) { A } else { B })
                    .unwrap();
            }
        }
        let layer = doc.layer(id).unwrap();
        let reader = Reference {
            doc: &doc,
            layer: Some(layer),
            channel: Channel::Color,
        };
        let matches = |c: Rgba8| c == A;
        let filled = fill_contiguous(
            &reader,
            &matches,
            w as usize,
            h as usize,
            ts as usize,
            (0, 0),
        )
        .unwrap();
        // 総当たり: 4 近傍の幅優先
        let (wu, hu) = (w as usize, h as usize);
        let mut seen = vec![false; wu * hu];
        let mut queue = std::collections::VecDeque::from([(0usize, 0usize)]);
        seen[0] = true;
        while let Some((x, y)) = queue.pop_front() {
            let near = [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ];
            for (nx, ny) in near {
                if nx < wu && ny < hu && !seen[ny * wu + nx] && is_a(nx as u32, ny as u32) {
                    seen[ny * wu + nx] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
        let same = (0..wu * hu).all(|i| filled.selected.get(i) == seen[i]);
        let count = (0..wu * hu).filter(|&i| filled.selected.get(i)).count();
        (count, filled.peak_pending, same)
    }

    /// 縞・櫛・格子・市松の入力でも、待ちの連の列は見積もりの上限（1 行に (幅 + 1) / 2 本）を超えず、答えは総当たりと同じ。
    /// 横線の行と櫛の行が並ぶ画像は、1 つの種から待ちの連が画素数の 1/4 以上まで溜まる形（96×80 で 1881〜2492 本。連ごとに印を付ける
    /// 前の、画素の番号を重ねて入れる方式では、横線と櫛の交互で 3714 本まで伸びた）。
    #[test]
    fn the_pending_runs_stay_under_the_estimate_on_stripes_combs_and_grids() {
        let (w, h) = (96u32, 80u32);
        let bound = max_pending_runs(w as usize, h as usize) as usize;
        type Pattern = Box<dyn Fn(u32, u32) -> bool>;
        let patterns: Vec<(&str, Pattern)> = vec![
            ("横線と櫛の交互", Box::new(|x, y| y % 2 == 0 || x % 2 == 0)),
            (
                "横線と縦の櫛が 3 行ごと",
                Box::new(|x, y| y % 3 == 0 || x % 2 == 0),
            ),
            ("縦縞と下の 1 行", Box::new(|x, y| y == 0 || x % 2 == 0)),
            ("横縞", Box::new(|_, y| y % 2 == 0)),
            ("格子", Box::new(|x, y| x % 3 == 0 || y % 3 == 0)),
            ("市松", Box::new(|x, y| (x + y) % 2 == 0)),
            ("全部", Box::new(|_, _| true)),
        ];
        let mut worst = 0;
        for (name, is_a) in &patterns {
            let (count, peak, same) = fill_pattern(w, h, 16, is_a);
            assert!(same, "{name}: 総当たりと違う");
            assert!(count >= 1, "{name}");
            assert!(
                peak <= bound,
                "{name}: 待ちの連 {peak} 本が上限 {bound} を超えた"
            );
            worst = worst.max(peak);
        }
        // 溜まる形は実際に溜まる（見積もりが空論でない）。上限の 1/4 以上に届く
        assert!(worst * 4 >= bound, "最大 {worst} 本、上限 {bound}");
    }

    /// 見積もりに待ちの連の列が入っている: 画素数が同じでも、列の分だけ作業の見積もりが増え、予算で断る。
    #[test]
    fn the_estimate_counts_the_pending_runs() {
        let mut doc = Document::with_tile_size(512, 512, 128).unwrap();
        let id = doc.add_layer("a").unwrap();
        doc.set_pixel(id, 0, 0, A).unwrap();
        let magic = |budget| {
            SelectionMask::magic_wand(&doc, Some(id), Channel::Color, 0, 0, 0, true, budget)
        };
        let stack = pending_bytes(512, 512) as u64;
        assert_eq!(stack, 256 * 512 * 4 * 2);
        // 印とタイルの分だけでは足りず、列の分を足すと入る
        let base = 512u64 * 512 / 8 + 16 * (128 * 128 / 8) + 128 * 128 * 4;
        assert_eq!(contiguous_bytes(512, 512, 128), (base + stack) as u128);
        assert!(matches!(
            magic(base + stack - 1),
            Err(CoreError::WorkingBudgetExceeded)
        ));
        assert!(magic(base + stack).is_ok());
        // 既定の予算（1 GiB）は 8192² を余裕で通し、16384² は通さない（選択範囲の mod の言い分と同じ）
        let budget = crate::selection::DEFAULT_WORKING_BUDGET_BYTES as u128;
        assert!(contiguous_bytes(8192, 8192, 128) <= budget / 3);
        assert!(contiguous_bytes(12288, 12288, 128) <= budget);
        assert!(contiguous_bytes(16384, 16384, 128) > budget);
    }
}
