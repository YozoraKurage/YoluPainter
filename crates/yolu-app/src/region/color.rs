//! 色から範囲を求める。元の文書は変えず、取消・予算拒否ではマスクも公開しない。
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use yolu_core::{
    Channel, CoreError, Document, LayerId, Rect, Rgba8, RowOrder, SelectionMask, TileCoord,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reference {
    #[default]
    Editing,
    Visible,
    Marked,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Distance {
    #[default]
    Rgb,
    Perceptual,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub reference: Reference,
    pub distance: Distance,
    pub gap: u8,
    pub margin: i16,
    pub leftovers: bool,
    pub max_area: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            reference: Reference::Editing,
            distance: Distance::Rgb,
            gap: 0,
            margin: 0,
            leftovers: false,
            max_area: 1024,
        }
    }
}

pub fn check(cancel: &AtomicBool) -> Result<(), CoreError> {
    if cancel.load(Ordering::Relaxed) {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
thread_local! { static COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// 赤の平均に応じて重みを変える整数の知覚近似。アルファ差は独立した上限。
fn similar(a: Rgba8, b: Rgba8, tolerance: u8, distance: Distance) -> bool {
    #[cfg(test)]
    COMPARISONS.with(|n| n.set(n.get() + 1));
    if a.a.abs_diff(b.a) > tolerance {
        return false;
    }
    let d = [
        a.r.abs_diff(b.r) as u64,
        a.g.abs_diff(b.g) as u64,
        a.b.abs_diff(b.b) as u64,
    ];
    match distance {
        Distance::Rgb => d.into_iter().max().unwrap() <= tolerance as u64,
        Distance::Perceptual => {
            let red = (a.r as u64 + b.r as u64) / 2;
            (512 + red) * d[0] * d[0] + 1024 * d[1] * d[1] + (767 - red) * d[2] * d[2]
                <= 2303 * (tolerance as u64).pow(2)
        }
    }
}

pub struct Request {
    pub layer: LayerId,
    pub channel: Channel,
    pub tolerance: u8,
    pub contiguous: bool,
    pub options: Options,
    pub marked: HashSet<LayerId>,
    /// 塗り残しなら通った点の列、でなければ種の点（対称定規の写しで 2 つ以上。和を 1 つの範囲にする）。
    pub points: Vec<(f64, f64)>,
    pub radius: f64,
    pub budget: u64,
    pub edit_mask: bool,
}

/// 参照印のあるグループは子孫を含み、親グループの可視性・マスク・不透明度は保つ。
fn reference_document(doc: &Document, marked: &HashSet<LayerId>) -> Result<Document, CoreError> {
    let mut copy = doc.capture_snapshot()?;
    let mut included = HashSet::new();
    let mut seen = HashSet::new();
    // 印とクリッピング下地は子孫も評価する。祖先は器だけを残す。
    let mut pending: Vec<_> = marked.iter().map(|&id| (id, true)).collect();
    while let Some((id, descendants)) = pending.pop() {
        if !seen.insert((id, descendants)) {
            continue;
        }
        let Some(layer) = doc.layer(id) else {
            continue;
        };
        included.insert(id);
        if descendants && layer.is_group() {
            pending.extend(
                doc.layers()
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
            let at = doc.layer_index(id).ok_or(CoreError::LayerNotFound)?;
            let mut base = None;
            for below in doc.layers()[..at]
                .iter()
                .rev()
                .filter(|l| l.parent() == layer.parent())
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
    for l in doc.layers() {
        if !included.contains(&l.id()) && l.visible() {
            copy.set_layer_visible(l.id(), false)?;
        }
    }
    Ok(copy)
}

fn read(doc: &Document, r: &Request, cancel: &AtomicBool) -> Result<Vec<Rgba8>, CoreError> {
    let (w, h) = (doc.width(), doc.height());
    let mut pixels = vec![Rgba8::TRANSPARENT; w as usize * h as usize];
    if r.options.reference == Reference::Editing {
        let layer = doc.layer(r.layer).ok_or(CoreError::LayerNotFound)?;
        for y in 0..h {
            check(cancel)?;
            for x in 0..w {
                pixels[(y * w + x) as usize] = layer.pixel(r.channel, x, y)?;
            }
        }
    } else {
        let copy;
        let source = if r.options.reference == Reference::Marked {
            copy = reference_document(doc, &r.marked)?;
            &copy
        } else {
            doc
        };
        let band_height = h.min(32);
        let mut band = vec![0; w as usize * band_height as usize * 4];
        for y in (0..h).step_by(band_height as usize) {
            check(cancel)?;
            let rows = band_height.min(h - y);
            let bytes = &mut band[..(w * rows * 4) as usize];
            source.composite_into_cancellable(
                r.channel,
                Rect::new(0, y, w, rows),
                bytes,
                RowOrder::BottomUp,
                Some(cancel),
            )?;
            for i in 0..(w * rows) as usize {
                pixels[y as usize * w as usize + i] = Rgba8::from_slice(&bytes[i * 4..]);
            }
        }
    }
    Ok(pixels)
}

/// 線の間にある幅 gap 以下の水平・垂直の切れ目を仮の壁にする。元画素は変更しない。
fn close_gaps(
    open: &mut [u8],
    w: usize,
    h: usize,
    gap: usize,
    cancel: &AtomicBool,
) -> Result<(), CoreError> {
    if gap == 0 {
        return Ok(());
    }
    let original = open.to_vec();
    for vertical in [false, true] {
        let (lines, length, stride) = if vertical { (w, h, w) } else { (h, w, 1) };
        for line in 0..lines {
            check(cancel)?;
            let base = if vertical { line } else { line * w };
            let mut previous = None;
            for p in 0..length {
                if original[base + p * stride] != 0 {
                    continue;
                }
                if let Some(last) = previous {
                    if p - last - 1 <= gap {
                        for k in last + 1..p {
                            open[base + k * stride] = 0;
                        }
                    }
                }
                previous = Some(p);
            }
        }
    }
    Ok(())
}

/// 色の判定と隙間閉じを探索した画素の近傍だけで行う。
/// 閉じる切れ目は `close_gaps` と同じ、両端に壁がある水平・垂直の連。
struct RegionPixels<'a> {
    pixels: &'a [Rgba8],
    width: usize,
    height: usize,
    seed: Rgba8,
    tolerance: u8,
    distance: Distance,
    gap: usize,
}
impl RegionPixels<'_> {
    fn matches(&self, i: usize) -> bool {
        similar(self.pixels[i], self.seed, self.tolerance, self.distance)
    }

    fn wall_distance(&self, x: usize, y: usize, dx: isize, dy: isize) -> Option<usize> {
        for step in 1..=self.gap {
            let nx = x as isize + dx * step as isize;
            let ny = y as isize + dy * step as isize;
            if nx < 0 || ny < 0 || nx >= self.width as isize || ny >= self.height as isize {
                break;
            }
            if !self.matches(ny as usize * self.width + nx as usize) {
                return Some(step);
            }
        }
        None
    }

    fn open(&self, i: usize) -> bool {
        if !self.matches(i) {
            return false;
        }
        if self.gap == 0 {
            return true;
        }
        let (x, y) = (i % self.width, i / self.width);
        for (dx, dy) in [(1, 0), (0, 1)] {
            if let (Some(a), Some(b)) = (
                self.wall_distance(x, y, dx, dy),
                self.wall_distance(x, y, -dx, -dy),
            ) {
                if a + b <= self.gap + 1 {
                    return false;
                }
            }
        }
        true
    }
}

fn mask(doc: &Document, data: &[u8]) -> Result<SelectionMask, CoreError> {
    let (w, h, ts) = (doc.width(), doc.height(), doc.tile_size());
    let tiles = (0..h.div_ceil(ts))
        .flat_map(|ty| (0..w.div_ceil(ts)).map(move |tx| (tx, ty)))
        .filter_map(|(tx, ty)| {
            let mut tile = vec![0; (ts * ts) as usize];
            let mut any = false;
            for y in 0..ts.min(h - ty * ts) {
                for x in 0..ts.min(w - tx * ts) {
                    let a = data[((ty * ts + y) * w + tx * ts + x) as usize];
                    tile[(y * ts + x) as usize] = a;
                    any |= a != 0;
                }
            }
            any.then_some((TileCoord::new(tx, ty), tile))
        });
    SelectionMask::from_amount_tiles(w, h, ts, tiles)
}

fn touched(x: usize, y: usize, r: &Request) -> bool {
    let p = (x as f64 + 0.5, y as f64 + 0.5);
    r.points.iter().enumerate().any(|(i, &b)| {
        // `NaN` の点は線の区切り（3D の、面の外・継ぎ目をまたぐ所）。区切りの前後は線でつながない
        if !b.0.is_finite() {
            return false;
        }
        let a = match i.checked_sub(1).map(|j| r.points[j]) {
            Some(before) if before.0.is_finite() => before,
            _ => b,
        };
        let d = (b.0 - a.0, b.1 - a.1);
        let len = d.0 * d.0 + d.1 * d.1;
        let t = if len == 0.0 {
            0.0
        } else {
            (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / len).clamp(0.0, 1.0)
        };
        (p.0 - a.0 - t * d.0).powi(2) + (p.1 - a.1 - t * d.1).powi(2) <= r.radius.powi(2)
    })
}

pub fn compute(
    doc: &Document,
    r: &Request,
    cancel: &AtomicBool,
) -> Result<SelectionMask, CoreError> {
    check(cancel)?;
    let (w, h) = (doc.width() as usize, doc.height() as usize);
    let n = w * h;
    // 画素、探索列、印、出力とモルフォロジーの一時領域。タイルの端の余白も見積もる。
    let padded = doc.width().div_ceil(doc.tile_size()) as u64
        * doc.height().div_ceil(doc.tile_size()) as u64
        * (doc.tile_size() as u64).pow(2);
    let reserved = n as u64 * 32 + padded * 4 + r.points.len() as u64 * 16;
    if reserved > r.budget {
        return Err(CoreError::WorkingBudgetExceeded);
    }
    let remaining = r.budget - reserved;
    let pixels = read(doc, r, cancel)?;
    let &(sx, sy) = r
        .points
        .first()
        .ok_or(CoreError::InvalidArgument("塗る点がない"))?;
    if !sx.is_finite()
        || !sy.is_finite()
        || sx < 0.0
        || sy < 0.0
        || sx >= w as f64
        || sy >= h as f64
    {
        return Err(CoreError::InvalidArgument("種がキャンバスの外"));
    }
    let mut output = vec![0; n];
    let mut visited = vec![false; n];
    // 色が異なる次の領域では再判定するが、配列の全消去はしない。
    // 領域数は最大で画素数（文書上限の 32768² は u32 に収まる）。
    let mut examined = vec![0u32; n];
    let mut generation = 0u32;
    let mut queue = Vec::with_capacity(n);
    // 塗り残しでなければ、点の列が種（対称定規の写し）。最初の点がキャンバスの外なら上で断る。ほかの点は、外のもの（呼び出し側が捨てるので
    // 通常は無い）を読み飛ばし、同じ画素は 1 回にする。種ごとに自分の色で範囲を求める
    let seeds: Box<dyn Iterator<Item = usize>> = if r.options.leftovers {
        Box::new(0..n)
    } else {
        let mut list: Vec<usize> = Vec::new();
        for &(x, y) in &r.points {
            if x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && y >= 0.0
                && x < w as f64
                && y < h as f64
            {
                let seed = y as usize * w + x as usize;
                if !list.contains(&seed) {
                    list.push(seed);
                }
            }
        }
        Box::new(list.into_iter())
    };
    for seed in seeds {
        if seed % w == 0 {
            check(cancel)?;
        }
        // 塗り残しは、先の領域に入った画素を種にしない。そうでなければ、種ごとに自分の色で範囲を求める（先の種の領域の中の種も飛ばさない）
        if r.options.leftovers && visited[seed] {
            continue;
        }
        if r.options.leftovers
            && (!touched(seed % w, seed / w, r)
                || !(pixels[seed].a < 255
                    || similar(
                        pixels[seed],
                        Rgba8::new(255, 255, 255, 255),
                        r.tolerance,
                        r.options.distance,
                    )))
        {
            continue;
        }
        let lookup = RegionPixels {
            pixels: &pixels,
            width: w,
            height: h,
            seed: pixels[seed],
            tolerance: r.tolerance,
            distance: r.options.distance,
            gap: r.options.gap as usize,
        };
        if !r.contiguous && !r.options.leftovers {
            let mut open = vec![0; n];
            for y in 0..h {
                check(cancel)?;
                for x in 0..w {
                    let i = y * w + x;
                    open[i] = u8::from(lookup.matches(i));
                }
            }
            close_gaps(&mut open, w, h, lookup.gap, cancel)?;
            if open[seed] != 0 {
                for i in 0..n {
                    output[i] |= open[i] * 255;
                }
            }
            continue;
        }
        if !lookup.open(seed) {
            visited[seed] = true;
            continue;
        }
        generation += 1;
        examined[seed] = generation;
        queue.clear();
        queue.push(seed);
        let mut edge = false;
        let mut head = 0;
        while head < queue.len() {
            if head % 4096 == 0 {
                check(cancel)?;
            }
            let i = queue[head];
            head += 1;
            visited[i] = true;
            let (x, y) = (i % w, i / w);
            edge |= x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            for next in [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then_some(i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then_some(i + w),
            ]
            .into_iter()
            .flatten()
            {
                if examined[next] != generation {
                    examined[next] = generation;
                    if lookup.open(next) {
                        queue.push(next);
                    }
                }
            }
        }
        if !r.options.leftovers || (!edge && queue.len() <= r.options.max_area as usize) {
            for &i in &queue {
                output[i] = 255;
            }
        }
    }
    check(cancel)?;
    let mut result = mask(doc, &output)?;
    if r.options.margin > 0 {
        result = result.grow(r.options.margin as u32, remaining)?;
    }
    if r.options.margin < 0 {
        result = result.shrink(r.options.margin.unsigned_abs() as u32, false, remaining)?;
    }
    if r.options.leftovers && !r.edit_mask {
        // 塗り済みの画素は変えない。白い不透明の紙も参照元としては使える。
        let layer = doc.layer(r.layer).ok_or(CoreError::LayerNotFound)?;
        for y in 0..h {
            check(cancel)?;
            for x in 0..w {
                let pixel = layer.pixel(r.channel, x as u32, y as u32)?;
                let unfilled = pixel.a < 255
                    || similar(
                        pixel,
                        Rgba8::new(255, 255, 255, 255),
                        r.tolerance,
                        r.options.distance,
                    );
                output[y * w + x] = if unfilled {
                    result.amount(x as u32, y as u32)
                } else {
                    0
                };
            }
        }
        result = mask(doc, &output)?;
    }
    check(cancel)?;
    Ok(result)
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn many_enclosures_do_not_repeat_full_canvas_color_classification() {
        let mut doc = Document::new(97, 97).unwrap();
        let layer = doc.add_layer("囲み").unwrap();
        for y in 0..97 {
            for x in 0..97 {
                let c = if x % 4 == 0 || y % 4 == 0 {
                    Rgba8::new(0, 0, 0, 255)
                } else {
                    Rgba8::new(255, 255, 255, 255)
                };
                doc.set_pixel(layer, x, y, c).unwrap();
            }
        }
        let r = Request {
            layer,
            channel: Channel::Color,
            tolerance: 0,
            contiguous: true,
            options: Options {
                leftovers: true,
                gap: 1,
                ..Default::default()
            },
            marked: HashSet::new(),
            points: vec![(48.5, 48.5)],
            radius: 100.0,
            budget: 1 << 30,
            edit_mask: false,
        };
        COMPARISONS.with(|n| n.set(0));
        let result = compute(&doc, &r, &AtomicBool::new(false)).unwrap();
        let comparisons = COMPARISONS.with(|n| n.get());
        eprintln!("97×97、576 領域、隙間幅 1: 色比較 {comparisons} 回");
        assert_eq!(
            result.to_canvas_bytes().iter().filter(|&&a| a > 0).count(),
            24 * 24 * 9
        );
        assert!(
            comparisons <= 64 * 97 * 97,
            "576 個の囲みで色比較 {comparisons} 回: 全キャンバスの反復をしない"
        );
    }
    #[test]
    fn local_gap_classification_matches_dense_color_and_alpha_boundaries() {
        let mut random = 17u32;
        let palette = [
            Rgba8::TRANSPARENT,
            Rgba8::new(0, 0, 0, 255),
            Rgba8::new(255, 255, 255, 255),
            Rgba8::new(245, 235, 225, 255),
            Rgba8::new(0, 0, 0, 32),
        ];
        for _ in 0..16 {
            let pixels: Vec<_> = (0..56)
                .map(|_| {
                    random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                    palette[random as usize % palette.len()]
                })
                .collect();
            for seed in palette {
                for distance in [Distance::Rgb, Distance::Perceptual] {
                    for tolerance in [0, 32] {
                        for gap in 0..=4 {
                            let lookup = RegionPixels {
                                pixels: &pixels,
                                width: 8,
                                height: 7,
                                seed,
                                tolerance,
                                distance,
                                gap,
                            };
                            let mut dense: Vec<_> = pixels
                                .iter()
                                .map(|&p| u8::from(similar(p, seed, tolerance, distance)))
                                .collect();
                            close_gaps(&mut dense, 8, 7, gap, &AtomicBool::new(false)).unwrap();
                            let local: Vec<_> = (0..56).map(|i| u8::from(lookup.open(i))).collect();
                            assert_eq!(
                                local, dense,
                                "種 {seed:?}、色差 {distance:?}、許容 {tolerance}、幅 {gap}"
                            );
                        }
                    }
                }
            }
        }
    }
}
