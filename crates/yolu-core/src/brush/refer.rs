//! はみ出し防止（参照レイヤーの線をはみ出さない）: ストロークの覆いに、参照レイヤーの重なりから求めた範囲のマスクを掛ける。
//!
//! - **参照の重なり**: ストロークの最初のダブの前に、参照の印のあるレイヤー（グループは子孫も）だけを見せた文書の写しを凍結する
//!   （[`Document::reference_view`] と同じ合成。写しはタイルを共有するので、凍結に画素の複製は要らない）。ダブが読む所のタイルだけを
//!   必要になったときに合成して覚え、覚えたバイトはストロークの予算に数える（上限を超えそうなら覚えたものを捨てて読み直す。写しは
//!   変わらないので、読み直しても同じ画素）。描いている間に参照の印やレイヤーが変わっても、そのストロークは始めの写しのまま。
//! - **狙いの色**: ストロークの最初のダブの中心の画素の、参照の重なりの色。
//! - **ダブごとの範囲**: ダブの外接の箱（キャンバスの中）の中で、ダブの中心の画素から、狙いの色に近い（[`crate::near_color::similar`]、
//!   色の誤差）画素を 4 近傍でたどった所だけを塗る。中心の画素が狙いの色に近くなければ、そのダブは何も塗らない。線をまたいで描くと、
//!   線の手前のダブは手前だけ、向こうのダブは向こうだけを塗り、線の上の画素は塗らない。
//! - **領域拡縮**: ダブの範囲を広げる（丸く・四角く・最も濃いピクセルまで）か、負の量で縮める。広げた範囲もダブの形の内側だけを塗る。
//! - 3D の面のダブは、面の UV が指す画素（キャンバスの画素）で同じことをする。ダブの画素を 4 近傍でつながる塊に分け、中心の画素を含む
//!   塊は中心から、ほかの塊（UV の継ぎ目の向こう・3D の対称の写し）は覆いの一番大きい画素からたどる。
//!
//! 範囲は 0 か 1（覆いを 0 にするか、そのまま）なので、範囲の内側の画素は、はみ出し防止を使わないときと同じ式・同じバイトで塗る。
//! 箱の中の参照が全部狙いの色に近いダブは、範囲を作らずに描く。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::error::CoreError;
use crate::layer::LayerSetId;
use crate::math::require_finite;
use crate::near_color::{similar, ColorDistance};
use crate::types::{Channel, LayerKind, Rect, Rgba8, RowOrder, TileCoord};
use crate::{Document, Reason};

/// 領域拡縮の量の上限（画素。負の量は縮める）。
pub const MAX_REFERENCE_EXPAND: f64 = 100.0;

/// 色の誤差の上限（%）。
pub const MAX_REFERENCE_MARGIN: f64 = 100.0;

/// 領域拡縮の方法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExpandMode {
    /// 四角く（縦横の距離の大きい方で量まで）。
    Square,
    /// 丸く（量を半径にした円で）。
    Round,
    /// 最も濃いピクセルまで: 丸く広げた中で、参照の濃さ（不透明度 × 暗さ）が下がらない向きにだけ広げる（線の中の一番濃い所で止まる）。
    /// 負の量（縮める）は丸くと同じ。
    #[default]
    Darkest,
}

/// はみ出し防止の設定（[`super::Brush::reference_lines`]）。ストロークを始めたときに写して固定する。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReferenceLines {
    /// 参照レイヤーの線をはみ出さない。
    pub enabled: bool,
    /// 色の誤差（0〜100 %）。狙いの色との、各成分とアルファの差の上限（255 の割合）。
    pub margin: f64,
    /// 領域拡縮を使うか。
    pub expand: bool,
    /// 領域拡縮の量（画素、−100〜100。四捨五入した整数の画素で効く）。
    pub amount: f64,
    pub mode: ExpandMode,
}

impl Default for ReferenceLines {
    /// 初めの値。色の誤差 12.5 % は、バケツの「近い色」の初めの許し幅（32 / 255）と同じ許し幅（[`ReferenceLines::tolerance`] で 32）にして、
    /// 値を変えないときにバケツと同じ範囲を「近い」と見る。量 2 画素は、線の縁のにじみ（アンチエイリアスの 1 画素）の外へ届く最小の整数。
    /// 方法は、線の中の一番濃い所で止まり、線の向こうの開いた所へ漏れない最も濃いピクセルまで。
    fn default() -> Self {
        ReferenceLines {
            enabled: false,
            margin: 12.5,
            expand: false,
            amount: 2.0,
            mode: ExpandMode::Darkest,
        }
    }
}

impl ReferenceLines {
    pub fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.margin, Reason::ReferenceMarginRange)?;
        require_finite(self.amount, Reason::ReferenceExpandRange)?;
        if !(0.0..=MAX_REFERENCE_MARGIN).contains(&self.margin) {
            return Err(CoreError::InvalidArgument(Reason::ReferenceMarginRange));
        }
        if !(-MAX_REFERENCE_EXPAND..=MAX_REFERENCE_EXPAND).contains(&self.amount) {
            return Err(CoreError::InvalidArgument(Reason::ReferenceExpandRange));
        }
        Ok(())
    }

    /// 色の誤差を、近い色の比べの許し幅（0〜255）へ。
    pub fn tolerance(&self) -> u8 {
        (self.margin / MAX_REFERENCE_MARGIN * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    }

    /// 領域拡縮の画素（負は縮める。使わなければ 0）。
    fn radius(&self) -> i64 {
        if self.expand {
            self.amount.round() as i64
        } else {
            0
        }
    }
}

/// 凍結した参照の重なり（ストロークのチャンネルの全部が共有する）。
pub(crate) struct ReferSource {
    view: Document,
    set: LayerSetId,
    channel: Channel,
    tile_size: i64,
    width: i64,
    height: i64,
    /// 見えるレイヤーが画素を持ち得るタイル（無い所は透明）。
    present: HashSet<TileCoord>,
    cache: Mutex<ReferCache>,
}

#[derive(Default)]
struct ReferCache {
    tiles: HashMap<TileCoord, Arc<Vec<Rgba8>>>,
    bytes: u64,
}

/// 覚えておく合成のタイルの上限（バイト）。超えるときは覚えたものを捨てる。
const CACHE_LIMIT: u64 = 32 << 20;

impl ReferSource {
    pub(crate) fn new(view: Document, set: LayerSetId, channel: Channel) -> ReferSource {
        let ts = view.tile_size() as i64;
        let mut present = HashSet::new();
        let all: Vec<TileCoord> = view.canvas_tiles().collect();
        for layer in view.layers() {
            if layer.set != set || !layer.visible || !layer.is_channel_enabled(channel) {
                continue;
            }
            match layer.kind {
                // 内容のフィルター・ジェネレーターは元の面のタイルの外へ画素を出しうる（ぼかしの広がり・全面の生成）ので、
                // その Raster は塗りつぶし・調整と同じく全部のタイルを読む（バケツの全面の合成と同じ画素にする）
                LayerKind::Raster if layer.has_evaluated_output(channel) => {
                    present.extend(all.iter().copied());
                }
                LayerKind::Raster => {
                    if let Some(s) = layer.surface(channel) {
                        present.extend(s.tile_coords());
                    }
                }
                LayerKind::Group => {}
                _ => present.extend(all.iter().copied()),
            }
        }
        ReferSource {
            tile_size: ts,
            width: view.width() as i64,
            height: view.height() as i64,
            view,
            set,
            channel,
            present,
            cache: Mutex::new(ReferCache::default()),
        }
    }

    /// 覚えている合成のバイト。
    pub(crate) fn bytes(&self) -> u64 {
        self.cache.lock().map_or(0, |c| c.bytes)
    }

    /// タイル 1 枚の合成（無い所は None = 透明）。`room` は覚えてよいバイトの残り。
    fn tile(&self, coord: TileCoord, room: u64) -> Result<Option<Arc<Vec<Rgba8>>>, CoreError> {
        if !self.present.contains(&coord) {
            return Ok(None);
        }
        let mut cache = self.cache.lock().map_err(|_| CoreError::Cancelled)?;
        if let Some(t) = cache.tiles.get(&coord) {
            return Ok(Some(t.clone()));
        }
        let ts = self.tile_size;
        let (x, y) = (coord.x as i64 * ts, coord.y as i64 * ts);
        let rect = Rect::new(
            x as u32,
            y as u32,
            ts.min(self.width - x) as u32,
            ts.min(self.height - y) as u32,
        );
        let bytes = 64 + (ts * ts * 4) as u64;
        if bytes > room {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        if cache.bytes + bytes > room.min(CACHE_LIMIT) {
            cache.tiles.clear();
            cache.bytes = 0;
        }
        let mut raw = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.view.composite_set_into_cancellable(
            self.set,
            self.channel,
            rect,
            &mut raw,
            RowOrder::BottomUp,
            None,
        )?;
        // タイルの大きさの格子に置き直す（キャンバスの端のタイルは、外を透明に）
        let mut tile = vec![Rgba8::TRANSPARENT; (ts * ts) as usize];
        for row in 0..rect.height as usize {
            for col in 0..rect.width as usize {
                let i = (row * rect.width as usize + col) * 4;
                tile[row * ts as usize + col] = Rgba8::from_slice(&raw[i..i + 4]);
            }
        }
        let tile = Arc::new(tile);
        cache.tiles.insert(coord, tile.clone());
        cache.bytes += bytes;
        Ok(Some(tile))
    }

    /// 画素 (x, y) の参照の色（キャンバスの中）。
    fn pixel(&self, x: i64, y: i64, room: u64) -> Result<Rgba8, CoreError> {
        let ts = self.tile_size;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        Ok(match self.tile(coord, room)? {
            None => Rgba8::TRANSPARENT,
            Some(t) => t[((y % ts) * ts + x % ts) as usize],
        })
    }

    /// 箱 [x0, x0 + w) × [y0, y0 + h)（キャンバスの中）にかかるタイルが、どれも画素を持ち得ないか（全部透明）。
    fn blank(&self, (x0, y0, w, h): (i64, i64, usize, usize)) -> bool {
        let ts = self.tile_size;
        let (x1, y1) = (x0 + w as i64 - 1, y0 + h as i64 - 1);
        (y0 / ts..=y1 / ts).all(|ty| {
            (x0 / ts..=x1 / ts)
                .all(|tx| !self.present.contains(&TileCoord::new(tx as u32, ty as u32)))
        })
    }

    /// 箱 [x0, x0 + w) × [y0, y0 + h)（キャンバスの中）の参照の色を out へ（行は下から）。
    fn read_box(
        &self,
        (x0, y0, w, h): (i64, i64, usize, usize),
        out: &mut Vec<Rgba8>,
        room: u64,
    ) -> Result<(), CoreError> {
        out.clear();
        out.resize(w * h, Rgba8::TRANSPARENT);
        let ts = self.tile_size;
        let (x1, y1) = (x0 + w as i64 - 1, y0 + h as i64 - 1);
        for ty in y0 / ts..=y1 / ts {
            for tx in x0 / ts..=x1 / ts {
                let Some(tile) = self.tile(TileCoord::new(tx as u32, ty as u32), room)? else {
                    continue;
                };
                let xs = (x0.max(tx * ts), x1.min(tx * ts + ts - 1));
                let ys = (y0.max(ty * ts), y1.min(ty * ts + ts - 1));
                for y in ys.0..=ys.1 {
                    let src = ((y - ty * ts) * ts + xs.0 - tx * ts) as usize;
                    let dst = (y - y0) as usize * w + (xs.0 - x0) as usize;
                    let len = (xs.1 - xs.0 + 1) as usize;
                    out[dst..dst + len].copy_from_slice(&tile[src..src + len]);
                }
            }
        }
        Ok(())
    }
}

/// 1 つのダブの範囲: 外接の箱と、箱の画素ごとの 0・1。
pub(crate) struct DabRegion {
    x0: i64,
    y0: i64,
    w: usize,
    h: usize,
    open: Vec<u8>,
}

impl DabRegion {
    /// 行 py の、x から始まる cov の [lo, hi) のうち、範囲の外の画素の覆いを 0 にする。
    #[inline]
    pub(crate) fn mask_row(&self, py: i64, x: i64, cov: &mut [f32], lo: usize, hi: usize) {
        let row = py - self.y0;
        if row < 0 || row >= self.h as i64 {
            cov[lo..hi].fill(0.0);
            return;
        }
        let base = row as usize * self.w;
        for (i, c) in cov.iter_mut().enumerate().take(hi).skip(lo) {
            let col = x + i as i64 - self.x0;
            if col < 0 || col >= self.w as i64 || self.open[base + col as usize] == 0 {
                *c = 0.0;
            }
        }
    }

    #[inline]
    pub(crate) fn contains(&self, x: i64, y: i64) -> bool {
        let (col, row) = (x - self.x0, y - self.y0);
        col >= 0
            && row >= 0
            && (col as usize) < self.w
            && (row as usize) < self.h
            && self.open[row as usize * self.w + col as usize] != 0
    }
}

/// ダブの範囲の答え。
pub(crate) enum DabArea {
    /// 箱の全部（範囲を掛けずに描く）。
    All,
    /// 何も塗らない。
    Nothing,
    Mask(DabRegion),
}

/// 1 つのチャンネルのストロークの、はみ出し防止の進み。
pub(crate) struct ReferRun {
    source: Arc<ReferSource>,
    lines: ReferenceLines,
    tolerance: u8,
    target: Option<Rgba8>,
    work: Work,
}

/// 範囲を求める作業の置き場（ダブの間で使い回す）。
#[derive(Default)]
struct Work {
    pixels: Vec<Rgba8>,
    open: Vec<u8>,
    queue: Vec<u32>,
    vertical: Vec<u32>,
}

impl Work {
    fn bytes(&self) -> u64 {
        (self.pixels.capacity() * 4
            + self.open.capacity()
            + self.queue.capacity() * 4
            + self.vertical.capacity() * 4) as u64
    }
}

/// 箱 1 画素あたりの作業のバイト（画素・開き・列・縦の距離・答え、と拡縮の一時の 2 枚）。
const WORK_BYTES_PER_PIXEL: u64 = 4 + 1 + 4 + 4 + 1 + 2;

impl ReferRun {
    pub(crate) fn new(source: Arc<ReferSource>, lines: ReferenceLines) -> ReferRun {
        ReferRun {
            source,
            tolerance: lines.tolerance(),
            lines,
            target: None,
            work: Work::default(),
        }
    }

    /// 予算に数えるバイト（覚えた合成と作業の置き場）。
    pub(crate) fn bytes(&self) -> u64 {
        self.source.bytes() + self.work.bytes()
    }

    /// 狙いの色が決まっていなければ、(x, y) の画素（キャンバスへ寄せる）の参照の色にする。
    pub(crate) fn ensure_target(&mut self, x: f64, y: f64, room: u64) -> Result<(), CoreError> {
        if self.target.is_some() {
            return Ok(());
        }
        let (px, py) = self.clamp_to_canvas(x, y);
        self.target = Some(self.source.pixel(px, py, room)?);
        Ok(())
    }

    fn clamp_to_canvas(&self, x: f64, y: f64) -> (i64, i64) {
        let s = &self.source;
        (
            (x.floor() as i64).clamp(0, s.width - 1),
            (y.floor() as i64).clamp(0, s.height - 1),
        )
    }

    /// `work.pixels` の画素ごとに、狙いの色に近いか（`work.open`）。全部近ければ true。
    fn classify(&self, work: &mut Work, target: Rgba8) -> bool {
        work.open.clear();
        let mut all = true;
        for &p in &work.pixels {
            let o = p == target || similar(p, target, self.tolerance, ColorDistance::Rgb);
            all &= o;
            work.open.push(o as u8);
        }
        all
    }

    /// 縮めるときに広げて読む箱（キャンバスの中）。
    fn padded(&self, xr: (i64, i64), yr: (i64, i64)) -> ((i64, i64), (i64, i64)) {
        let pad = (-self.lines.radius()).max(0);
        let s = &self.source;
        (
            ((xr.0 - pad).max(0), (xr.1 + pad).min(s.width - 1)),
            ((yr.0 - pad).max(0), (yr.1 + pad).min(s.height - 1)),
        )
    }

    fn ensure_work(&self, n: usize, room: u64) -> Result<(), CoreError> {
        if n as u64 * WORK_BYTES_PER_PIXEL > room {
            Err(CoreError::StrokeBudgetExceeded)
        } else {
            Ok(())
        }
    }

    /// 2D のダブ（外接の箱 xr × yr はキャンバスの中、中心 (x, y)）の範囲。
    pub(crate) fn area(
        &mut self,
        xr: (i64, i64),
        yr: (i64, i64),
        (x, y): (f64, f64),
        room: u64,
    ) -> Result<DabArea, CoreError> {
        // 縮めるときは、箱の外のそばの線も見る（箱を縮める量だけ広げて読み、たどるのはダブの箱の中だけ）
        let (bx, by) = self.padded(xr, yr);
        let (w, h) = ((bx.1 - bx.0 + 1) as usize, (by.1 - by.0 + 1) as usize);
        let n = w * h;
        self.ensure_work(n, room)?;
        self.ensure_target(x, y, room)?;
        let target = self.target.expect("決めた");
        // 箱の参照が全部透明（画素を持ち得るタイルが無い）で、狙いの色が透明に近ければ、範囲は箱の全部
        if self.source.blank((bx.0, by.0, w, h))
            && similar(
                Rgba8::TRANSPARENT,
                target,
                self.tolerance,
                ColorDistance::Rgb,
            )
        {
            return Ok(DabArea::All);
        }
        let mut work = std::mem::take(&mut self.work);
        let result = (|| {
            self.source
                .read_box((bx.0, by.0, w, h), &mut work.pixels, room)?;
            let all = self.classify(&mut work, target);
            if all {
                return Ok(DabArea::All);
            }
            let inner = (bx, by) != (xr, yr);
            let allowed: Option<Vec<u8>> = inner.then(|| {
                (0..n)
                    .map(|i| {
                        let (px, py) = (bx.0 + (i % w) as i64, by.0 + (i / w) as i64);
                        (px >= xr.0 && px <= xr.1 && py >= yr.0 && py <= yr.1) as u8
                    })
                    .collect()
            });
            let sx = ((x.floor() as i64).clamp(xr.0, xr.1) - bx.0) as usize;
            let sy = ((y.floor() as i64).clamp(yr.0, yr.1) - by.0) as usize;
            let seed = sy * w + sx;
            let region = self.region(&mut work, w, h, &[seed], allowed.as_deref());
            Ok(finish(crop(&region, (bx.0, by.0, w), xr, yr), xr, yr))
        })();
        self.work = work;
        result
    }

    /// 3D の面のダブ: キャンバスの中の画素 `cells`（座標と覆い。同じ画素が重なっていてもよい）のうち、塗る画素の印。`center` は
    /// ダブの中心（キャンバスの画素の座標。無ければ、どの塊も覆いの一番大きい画素からたどり、狙いの色もその画素で決める）。
    pub(crate) fn keep_listed(
        &mut self,
        cells: &[(i64, i64, f64)],
        center: Option<(f64, f64)>,
        room: u64,
    ) -> Result<Vec<bool>, CoreError> {
        let mut keep = vec![false; cells.len()];
        if cells.is_empty() {
            return Ok(keep);
        }
        let strongest = (0..cells.len())
            .reduce(|best, m| if cells[m].2 > cells[best].2 { m } else { best })
            .expect("空でない");
        let (cx, cy) = center.unwrap_or((
            cells[strongest].0 as f64 + 0.5,
            cells[strongest].1 as f64 + 0.5,
        ));
        self.ensure_target(cx, cy, room)?;
        let target = self.target.expect("決めた");
        // 画素から番号（同じ画素の重なりは最初の番号を代表に）
        let mut index: HashMap<(i64, i64), usize> = HashMap::with_capacity(cells.len());
        for (i, &(x, y, _)) in cells.iter().enumerate() {
            index.entry((x, y)).or_insert(i);
        }
        let center_px = center.map(|(x, y)| (x.floor() as i64, y.floor() as i64));
        let mut group = vec![usize::MAX; cells.len()];
        let mut groups = 0usize;
        let mut members: Vec<usize> = Vec::new();
        let mut work = std::mem::take(&mut self.work);
        let result = (|| {
            for start in 0..cells.len() {
                let key = (cells[start].0, cells[start].1);
                if index[&key] != start || group[start] != usize::MAX {
                    continue;
                }
                // 4 近傍でつながる塊
                members.clear();
                group[start] = groups;
                members.push(start);
                let mut head = 0;
                while head < members.len() {
                    let (x, y, _) = cells[members[head]];
                    head += 1;
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        if let Some(&j) = index.get(&(x + dx, y + dy)) {
                            if group[j] == usize::MAX {
                                group[j] = groups;
                                members.push(j);
                            }
                        }
                    }
                }
                groups += 1;
                let center_member = center_px
                    .and_then(|c| index.get(&c))
                    .copied()
                    .filter(|&j| group[j] == groups - 1);
                self.keep_group(
                    &mut work,
                    cells,
                    &members,
                    center_member,
                    target,
                    room,
                    &mut keep,
                )?;
            }
            Ok(())
        })();
        self.work = work;
        result?;
        // 重なった画素は代表と同じ
        for (i, &(x, y, _)) in cells.iter().enumerate() {
            keep[i] = keep[index[&(x, y)]];
        }
        Ok(keep)
    }

    /// 3D の面のダブの、4 近傍でつながる塊 1 つ（`members` は `cells` の番号）の範囲を求めて `keep` に印を付ける。`center_member` は
    /// ダブの中心の画素がこの塊にあるときのその番号（その画素から、無ければ覆いの一番大きい画素からたどる）。
    #[allow(clippy::too_many_arguments)]
    fn keep_group(
        &self,
        work: &mut Work,
        cells: &[(i64, i64, f64)],
        members: &[usize],
        center_member: Option<usize>,
        target: Rgba8,
        room: u64,
        keep: &mut [bool],
    ) -> Result<(), CoreError> {
        let (mut x0, mut x1, mut y0, mut y1) = (i64::MAX, i64::MIN, i64::MAX, i64::MIN);
        for &m in members {
            let (x, y, _) = cells[m];
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        let ((x0, x1), (y0, y1)) = self.padded((x0, x1), (y0, y1));
        let (w, h) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
        self.ensure_work(w * h * 2, room)?;
        self.source
            .read_box((x0, y0, w, h), &mut work.pixels, room)?;
        let mut allowed = vec![0u8; w * h];
        for &m in members {
            let (x, y, _) = cells[m];
            allowed[(y - y0) as usize * w + (x - x0) as usize] = 1;
        }
        self.classify(work, target);
        let shrinks = self.lines.radius() < 0;
        let all = (0..w * h).all(|i| work.open[i] != 0 || (allowed[i] == 0 && !shrinks));
        let region = if all {
            None
        } else {
            // 種: 中心の画素を含む塊は中心、ほかは覆いの一番大きい画素（同じなら番号の小さい方）
            let seed_member = center_member.unwrap_or_else(|| {
                let mut best = members[0];
                for &m in members {
                    if cells[m].2 > cells[best].2 || (cells[m].2 == cells[best].2 && m < best) {
                        best = m;
                    }
                }
                best
            });
            let seed =
                (cells[seed_member].1 - y0) as usize * w + (cells[seed_member].0 - x0) as usize;
            Some(DabRegion {
                x0,
                y0,
                w,
                h,
                open: self.region(work, w, h, &[seed], Some(&allowed)),
            })
        };
        for &m in members {
            if region
                .as_ref()
                .is_none_or(|r| r.contains(cells[m].0, cells[m].1))
            {
                keep[m] = true;
            }
        }
        Ok(())
    }

    /// 開いた画素（`work.open`）を、種から 4 近傍でたどった範囲に、領域拡縮を当てる（箱 w × h。`allowed` があれば、たどるのはその画素
    /// だけ: 面のダブの画素か、縮めるときに広げた箱の中のダブの箱）。種が開いていなければ空。
    fn region(
        &self,
        work: &mut Work,
        w: usize,
        h: usize,
        seeds: &[usize],
        allowed: Option<&[u8]>,
    ) -> Vec<u8> {
        let n = w * h;
        let mut region = vec![0u8; n];
        let may = |j: usize| allowed.is_none_or(|a| a[j] != 0);
        work.queue.clear();
        for &s in seeds {
            if work.open[s] != 0 && may(s) && region[s] == 0 {
                region[s] = 1;
                work.queue.push(s as u32);
            }
        }
        let mut head = 0;
        while head < work.queue.len() {
            let i = work.queue[head] as usize;
            head += 1;
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if work.open[j] != 0 && may(j) && region[j] == 0 {
                    region[j] = 1;
                    work.queue.push(j as u32);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if work.queue.is_empty() {
            return region;
        }
        let radius = self.lines.radius();
        if radius == 0 {
            return region;
        }
        // 箱の中の 2 点は w + h より離れない（丸くの対角も入る）ので、それ以上は量を丸めても同じ範囲
        let r = (radius.unsigned_abs() as usize).min(w + h);
        let half: Vec<usize> = match self.lines.mode {
            ExpandMode::Square => vec![r; 2 * r + 1],
            ExpandMode::Round | ExpandMode::Darkest => {
                crate::selection::disk_half_heights(r as i64)
            }
        };
        if radius > 0 {
            let grown = grow(&region, w, h, &half, &mut work.vertical);
            if self.lines.mode == ExpandMode::Darkest {
                darkest(&region, &grown, w, h, &work.pixels, &mut work.queue)
            } else {
                grown
            }
        } else {
            // 縮める: 範囲の外（たどれる画素のうち範囲に入らなかった所と、箱の中の線）を広げた所を除く。たどれない所の開いた画素は
            // 範囲の続きと見る（ダブの縁からは縮まない）
            let outside: Vec<u8> = (0..n)
                .map(|i| (region[i] == 0 && (may(i) || work.open[i] == 0)) as u8)
                .collect();
            let grown = grow(&outside, w, h, &half, &mut work.vertical);
            (0..n).map(|i| region[i] & (1 - grown[i])).collect()
        }
    }
}

/// 広げた箱（左下 (x0, y0)、幅 w）の範囲を、ダブの箱 xr × yr へ切り出す。
fn crop(region: &[u8], (x0, y0, w): (i64, i64, usize), xr: (i64, i64), yr: (i64, i64)) -> Vec<u8> {
    let mut out = Vec::with_capacity(((xr.1 - xr.0 + 1) * (yr.1 - yr.0 + 1)) as usize);
    for y in yr.0..=yr.1 {
        let row = (y - y0) as usize * w;
        out.extend_from_slice(&region[row + (xr.0 - x0) as usize..=row + (xr.1 - x0) as usize]);
    }
    out
}

/// ダブの箱の範囲の 0・1 から答えへ（全部 1 なら範囲なし、全部 0 なら塗らない）。
fn finish(region: Vec<u8>, xr: (i64, i64), yr: (i64, i64)) -> DabArea {
    if region.iter().all(|&v| v != 0) {
        DabArea::All
    } else if region.iter().all(|&v| v == 0) {
        DabArea::Nothing
    } else {
        DabArea::Mask(DabRegion {
            x0: xr.0,
            y0: yr.0,
            w: (xr.1 - xr.0 + 1) as usize,
            h: (yr.1 - yr.0 + 1) as usize,
            open: region,
        })
    }
}

/// 2 値の膨張: 横のずれ i − r の列の、縦に ±half[i] の中に 1 があれば 1（箱の外は 0）。
fn grow(mask: &[u8], w: usize, h: usize, half: &[usize], vertical: &mut Vec<u32>) -> Vec<u8> {
    let r = (half.len() / 2) as i64;
    // 列ごとの、上下で一番近い 1 までの縦の距離
    vertical.clear();
    vertical.resize(w * h, u32::MAX);
    for x in 0..w {
        let mut last = None;
        for y in 0..h {
            let i = y * w + x;
            if mask[i] != 0 {
                last = Some(y);
            }
            if let Some(l) = last {
                vertical[i] = (y - l) as u32;
            }
        }
        let mut last = None;
        for y in (0..h).rev() {
            let i = y * w + x;
            if mask[i] != 0 {
                last = Some(y);
            }
            if let Some(l) = last {
                vertical[i] = vertical[i].min((l - y) as u32);
            }
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let lo = (x as i64 - r).max(0);
            let hi = (x as i64 + r).min(w as i64 - 1);
            for xx in lo..=hi {
                if vertical[row + xx as usize] as u64 <= half[(xx - x as i64 + r) as usize] as u64 {
                    out[row + x] = 1;
                    break;
                }
            }
        }
    }
    out
}

/// 参照の濃さ（不透明度 × 暗さ。整数の輝度で）。
#[inline]
fn density(p: Rgba8) -> u32 {
    let luma = (299 * p.r as u32 + 587 * p.g as u32 + 114 * p.b as u32 + 500) / 1000;
    p.a as u32 * (255 - luma)
}

/// 最も濃いピクセルまで: 範囲から、丸く広げた所（`grown`）の中を、濃さが下がらない隣へだけ 4 近傍でたどる。
fn darkest(
    region: &[u8],
    grown: &[u8],
    w: usize,
    h: usize,
    pixels: &[Rgba8],
    queue: &mut Vec<u32>,
) -> Vec<u8> {
    let mut out = region.to_vec();
    queue.clear();
    queue.extend((0..w * h).filter(|&i| region[i] != 0).map(|i| i as u32));
    let mut head = 0;
    while head < queue.len() {
        let i = queue[head] as usize;
        head += 1;
        let d = density(pixels[i]);
        let (x, y) = (i % w, i / w);
        for (ok, j) in [
            (x > 0, i.wrapping_sub(1)),
            (x + 1 < w, i + 1),
            (y > 0, i.wrapping_sub(w)),
            (y + 1 < h, i + w),
        ] {
            if ok && out[j] == 0 && grown[j] != 0 && density(pixels[j]) >= d {
                out[j] = 1;
                queue.push(j as u32);
            }
        }
    }
    out
}

impl Document {
    /// 参照レイヤーの重なりを読むための写し（バケツの「近い色」の「参照: 参照レイヤー」と、ブラシのはみ出し防止が使う）: 印のある
    /// レイヤー（グループは子孫も、クリッピングしたレイヤーは下地も）だけを見せ、祖先のグループは器（見え方・マスク・不透明度）だけを
    /// 残す。画素は写さない（タイルを共有する）。描いている最中は断る。
    pub fn reference_view(&self, marked: &HashSet<crate::LayerId>) -> Result<Document, CoreError> {
        let copy = self.capture_snapshot()?;
        self.hide_unreferenced(copy, marked)
    }

    /// [`Document::reference_view`] の、描いている最中の形（ストロークの最初のダブの前だけに使う。画素はまだ変わっていない）。
    pub(crate) fn reference_view_now(
        &self,
        marked: &HashSet<crate::LayerId>,
    ) -> Result<Document, CoreError> {
        let copy = self.snapshot_unchecked()?;
        self.hide_unreferenced(copy, marked)
    }

    fn hide_unreferenced(
        &self,
        mut copy: Document,
        marked: &HashSet<crate::LayerId>,
    ) -> Result<Document, CoreError> {
        let mut included = HashSet::new();
        let mut seen = HashSet::new();
        // 印とクリッピング下地は子孫も評価する。祖先は器だけを残す。
        let mut pending: Vec<_> = marked.iter().map(|&id| (id, true)).collect();
        while let Some((id, descendants)) = pending.pop() {
            if !seen.insert((id, descendants)) {
                continue;
            }
            let Some(layer) = self.layer(id) else {
                continue;
            };
            included.insert(id);
            if descendants && layer.is_group() {
                pending.extend(
                    self.layers()
                        .iter()
                        .filter(|l| l.parent() == Some(id))
                        .map(|l| (l.id(), true)),
                );
            }
            if let Some(parent) = layer.parent() {
                pending.push((parent, false));
            }
            if layer.clipping() {
                // 合成器と同じく、同じ親の下へ辿って最初の非クリッピングレイヤー
                // （全てクリッピングなら先頭の兄弟）を下地にする。
                let at = self.layer_index(id).ok_or(CoreError::LayerNotFound)?;
                let mut base = None;
                for below in self.layers()[..at]
                    .iter()
                    .rev()
                    .filter(|l| l.parent() == layer.parent() && l.layer_set() == layer.layer_set())
                {
                    base = Some(below.id());
                    if !below.clipping() {
                        break;
                    }
                }
                if let Some(base) = base {
                    pending.push((base, true));
                }
            }
        }
        for l in self.layers() {
            if !included.contains(&l.id()) && l.visible() {
                copy.set_layer_visible(l.id(), false)?;
            }
        }
        Ok(copy)
    }
}

#[cfg(test)]
mod tests {
    //! 道（スカラー・SSE4.1・AVX2・NEON と画素ごとの式）とスレッドの数・ワーカーの経路によらず、はみ出し防止のストロークが同じバイトになること。
    use super::*;
    use crate::brush::rows::PER_PIXEL;
    use crate::math::simd::{forced, Level};
    use crate::{
        builtin_tip, Brush, BrushSettings, CanvasSymmetry, DualBrush, LayerId, PaperTexture,
        PressureSwitches, SymmetryMode,
    };
    use glam::DVec2;
    use std::sync::atomic::Ordering;

    fn scene() -> (Document, LayerId, LayerId) {
        let mut d = Document::with_tile_size(160, 96, 32).unwrap();
        let r = d.add_layer("参照").unwrap();
        for y in 0..96 {
            for (x, a) in [(70, 128), (71, 255), (72, 255), (73, 128)] {
                d.set_pixel(r, x, y, Rgba8::new(10, 10, 10, a)).unwrap();
            }
            if y % 3 == 0 {
                d.set_pixel(r, 110 + (y % 7), y, Rgba8::new(0, 0, 0, 255))
                    .unwrap();
            }
        }
        let p = d.add_layer("塗る").unwrap();
        d.clear_history().unwrap();
        (d, r, p)
    }

    fn brushes() -> Vec<Brush> {
        let base = Brush::simple(
            BrushSettings {
                radius: 18.0,
                hardness: 0.6,
                spacing: 0.1,
                opacity: 0.9,
                flow: 0.7,
                color: Rgba8::new(200, 40, 30, 255),
                ..BrushSettings::default()
            },
            PressureSwitches::NONE,
        );
        let lines = |amount: f64, mode: ExpandMode, expand: bool| ReferenceLines {
            enabled: true,
            expand,
            amount,
            mode,
            ..ReferenceLines::default()
        };
        let mut out = Vec::new();
        for l in [
            lines(0.0, ExpandMode::Round, false),
            lines(4.0, ExpandMode::Round, true),
            lines(4.0, ExpandMode::Square, true),
            lines(6.0, ExpandMode::Darkest, true),
            lines(-3.0, ExpandMode::Round, true),
        ] {
            let mut b = base.clone();
            b.reference_lines = l;
            out.push(b.clone());
            // デュアル・紙の質感・ダブごとの色（画素ごとの色の道）・対称
            let mut d = b.clone();
            d.dual = Some(DualBrush {
                radius: 6.0,
                ..DualBrush::default()
            });
            d.texture = Some(PaperTexture::new(builtin_tip("grain").unwrap(), 0.5));
            out.push(d);
            let mut c = b.clone();
            c.color.hue = 0.2;
            c.color.per_tip = true;
            c.seed = 3;
            out.push(c);
            let mut s = b.clone();
            s.symmetry =
                CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(80.0, 48.0), 2).unwrap();
            out.push(s);
        }
        out
    }

    fn run(b: &Brush) -> Vec<u8> {
        let (mut d, r, p) = scene();
        let marked: HashSet<LayerId> = [r].into_iter().collect();
        let mut s = d.begin_brush_stroke(p, b).unwrap();
        assert!(s
            .use_reference_layers(&mut d, &marked, Channel::Color)
            .unwrap());
        for (x, y) in [(20.0, 30.0), (90.0, 50.0), (150.0, 70.0), (60.0, 80.0)] {
            s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
        }
        d.end_stroke(s).unwrap();
        d.layer(p)
            .unwrap()
            .surface(Channel::Color)
            .map(|s| {
                let mut out = Vec::new();
                for y in 0..96 {
                    for x in 0..160 {
                        out.extend_from_slice(&s.pixel(x, y).unwrap().to_array());
                    }
                }
                out
            })
            .unwrap_or_default()
    }

    fn per_pixel<R>(f: impl FnOnce() -> R) -> R {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                PER_PIXEL.store(false, Ordering::Relaxed);
            }
        }
        PER_PIXEL.store(true, Ordering::Relaxed);
        let _reset = Reset;
        f()
    }

    #[test]
    fn every_level_thread_count_and_worker_path_paints_the_same_bytes() {
        for (i, b) in brushes().iter().enumerate() {
            let reference = forced::with_level(Level::Scalar, || per_pixel(|| run(b)));
            assert!(reference.chunks_exact(4).any(|p| p[3] != 0), "{i}: 描いた");
            for level in forced::supported() {
                assert_eq!(
                    forced::with_level(level, || run(b)),
                    reference,
                    "{i}: {level:?}"
                );
            }
            let old = crate::brush::set_parallel_dab_pixels(1);
            for threads in [1, 3] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                assert_eq!(
                    pool.install(|| run(b)),
                    reference,
                    "{i}: {threads} スレッド"
                );
            }
            crate::brush::set_parallel_dab_pixels(old);
        }
    }
}
