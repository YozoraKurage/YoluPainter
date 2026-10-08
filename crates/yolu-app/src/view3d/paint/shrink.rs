//! 縮めと矩形: 予算と辺の上限から縮めの段を決める・上げる矩形をまとめて帯に切る。

use super::*;

impl Paint {
    /// この文書を `cap`（辺の上限）で縮めて持つとしたときのバイト数（使っているチャンネルのミップ込み。持つかどうかの計画に使う）。
    pub fn estimate_bytes(&self, doc: &Document, cap: u32) -> u64 {
        self.estimate(doc, cap).1
    }

    /// `estimate_bytes` の、縮めの段とバイト数（ユーザーチャンネルの配列は、この段から縮める）。
    pub fn estimate(&self, doc: &Document, cap: u32) -> (u32, u64) {
        let limit = self.side_limit().min(cap);
        let per_texel = self.bytes_per_texel_planned(doc);
        let shift = choose_shift([doc.width(), doc.height()], per_texel, limit, u64::MAX);
        let size = [
            doc.width().div_ceil(1 << shift).max(1),
            doc.height().div_ceil(1 << shift).max(1),
        ];
        (shift, mip_bytes(size, per_texel))
    }

    /// 次の同期のあとに、この文書の絵が取るバイト数（使っているチャンネルのミップ込み）。作り直すなら今の予算と上限で決まる大きさ、
    /// 作り直さないなら今の大きさ。今のセットが替わるとき、新しい絵を作る前に予算の残りを決めるのに使う。
    pub fn planned_bytes(&self, doc: &Document) -> u64 {
        let shift = self.planned_level(doc);
        let size = [
            doc.width().div_ceil(1 << shift).max(1),
            doc.height().div_ceil(1 << shift).max(1),
        ];
        mip_bytes(size, self.bytes_per_texel_planned(doc))
    }

    /// `planned_bytes` の縮めの段。
    pub fn planned_level(&self, doc: &Document) -> u32 {
        let (want, _) = self.shift_for(doc);
        match &self.set {
            Some(s) if !self.rebuild_needed(doc, want) => s.shift,
            _ => want,
        }
    }

    /// 文書を持つ縮めを決める 1 テクセルのバイト数: 使っているチャンネルの合計に、表示の写しを塗り広げる（重みの絵を持つ）なら重みの 1 バイトを足す。
    /// 重みの絵はセットごとに 1 つで、チャンネルの数によらない。使っているチャンネルが無ければミップを作らないので足さない。モデルの UV が無い
    /// （塗り広げない）セットも、計画では持つ側に数える（計画が実際より小さくならないように）。
    pub(super) fn bytes_per_texel_planned(&self, doc: &Document) -> u64 {
        let used = used_bytes_per_texel(doc);
        if used > 0 && self.pad_texels > 0 {
            used + 1
        } else {
            used
        }
    }

    /// 絵の一辺の上限（GPU の上限と `MAX_PAINT_SIZE` の小さい方。上限 `cap` は含めない）。
    pub(super) fn side_limit(&self) -> u32 {
        self.device
            .limits()
            .max_texture_dimension_2d
            .min(MAX_PAINT_SIZE)
    }

    /// 文書を持つ縮めの段（2 の shift 乗）と、それが予算で決まったか。使っているチャンネルのミップ込みの合計が予算に収まり、辺が GPU の
    /// 上限（と、あれば `cap`）に収まるまで上げる。`cap` で決まる縮めは予算で決まるのではないので、`by_budget` にしない。
    pub(super) fn shift_for(&self, doc: &Document) -> (u32, bool) {
        let limit = self.side_limit().min(self.cap.unwrap_or(u32::MAX));
        let per_texel = self.bytes_per_texel_planned(doc);
        let (w, h) = (doc.width(), doc.height());
        let shift = choose_shift([w, h], per_texel, limit, self.budget);
        let by_limit = choose_shift([w, h], 0, limit, u64::MAX);
        (shift, shift > by_limit)
    }
}

/// 文書が使っているチャンネルの 1 テクセルのバイト数の合計（重みの絵は含めない）。
pub(super) fn used_bytes_per_texel(doc: &Document) -> u64 {
    Slot::ALL
        .iter()
        .filter(|s| uses(doc, s.channel()))
        .map(|s| s.bytes_per_texel() as u64)
        .sum()
}

/// 大きさ（辺）のテクスチャ（全部の段）のバイト数。`bytes_per_texel` は 1 テクセルのバイト数（チャンネルを重ねるなら合計）。
pub(in crate::view3d) fn mip_bytes(size: [u32; 2], bytes_per_texel: u64) -> u64 {
    let levels = 32 - size[0].max(size[1]).leading_zeros();
    (0..levels)
        .map(|l| (size[0] >> l).max(1) as u64 * (size[1] >> l).max(1) as u64)
        .sum::<u64>()
        * bytes_per_texel
}

/// 大きさを 2 の shift 乗で縮める段数: 辺が `limit` に収まり、全部の段のバイト数（`bytes_per_texel` は重ねるチャンネルの合計）が
/// `budget` に収まる最小の段。1 × 1 まで縮めても収まらないなら、そこまで。
pub(in crate::view3d) fn choose_shift(
    size: [u32; 2],
    bytes_per_texel: u64,
    limit: u32,
    budget: u64,
) -> u32 {
    let mut shift = 0;
    while shift < 31 {
        let reduced = [
            size[0].div_ceil(1 << shift).max(1),
            size[1].div_ceil(1 << shift).max(1),
        ];
        let fits = reduced[0] <= limit
            && reduced[1] <= limit
            && mip_bytes(reduced, bytes_per_texel) <= budget;
        if fits || reduced == [1, 1] {
            break;
        }
        shift += 1;
    }
    shift
}

// ───────── 塗り広げ ─────────

/// 矩形を `by` 画素だけ外へ広げる（`bounds` で切る）。
pub(super) fn expand(rect: DocRect, by: u32, bounds: DocRect) -> DocRect {
    let x0 = rect.x.saturating_sub(by);
    let y0 = rect.y.saturating_sub(by);
    let x1 = (rect.x + rect.width + by).min(bounds.width);
    let y1 = (rect.y + rect.height + by).min(bounds.height);
    DocRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 矩形を縮めの境（2^shift）に合わせて外へ広げる（キャンバスの端で切る）。
pub(in crate::view3d) fn align(rect: DocRect, shift: u32, bounds: DocRect) -> DocRect {
    if shift == 0 {
        return rect;
    }
    let block = 1u32 << shift;
    let x0 = (rect.x >> shift) << shift;
    let y0 = (rect.y >> shift) << shift;
    let x1 = (rect.x + rect.width).div_ceil(block) * block;
    let y1 = (rect.y + rect.height).div_ceil(block) * block;
    DocRect::new(
        x0,
        y0,
        x1.min(bounds.width) - x0,
        y1.min(bounds.height) - y0,
    )
}

/// 全部を作るときに合成するタイル: キャンバス全体に効くレイヤー（塗りつぶし・調整）か、元の画素の無いタイルへ出力が広がる効果（そのチャンネルに当たる
/// フィルター・Generator、レイヤーのマスクのフィルター。ぼかしの広がり・画素の無いレイヤーの Generator）があれば全タイル、無ければどれかのレイヤーが
/// 面を持つタイル。全タイルを合成するのは 2D の表示と同じ見た目にするためで、透明のままのタイルも上げる（段 0 の作った直後の値と同じ）。
pub(super) fn full_rects(doc: &Document, slot: Slot, shift: u32) -> Vec<(DocRect, usize)> {
    let channel = slot.channel();
    let whole = doc.layers().iter().any(|l| {
        (matches!(l.kind(), LayerKind::Fill | LayerKind::Adjustment)
            && l.is_channel_enabled(channel))
            || l.has_active_filters(channel)
            || l.mask().is_some_and(|m| m.has_active_filters())
    });
    let mut coords: Vec<TileCoord> = if whole || slot == Slot::Normal && doc.derives_normal() {
        doc.canvas_tiles().collect()
    } else {
        let mut all: Vec<TileCoord> = doc
            .layers()
            .iter()
            .filter_map(|l| l.surface(channel))
            .flat_map(|s| s.tile_coords())
            .collect();
        if slot == Slot::Normal {
            all.extend(
                doc.layers()
                    .iter()
                    .filter_map(|l| l.surface(Channel::Height))
                    .flat_map(|s| s.tile_coords()),
            );
        }
        all
    };
    coords.sort();
    coords.dedup();
    rects_of(doc, slot, &coords, &[], shift)
}

/// 上げ直すタイル `coords`（そのチャンネルの since の後に変わったタイルなど）の矩形。Normal は、Height → Normal が有効なら Height の
/// 変わったタイルの外側 1 画素までを足す。
pub(super) fn changed_rects(
    doc: &Document,
    slot: Slot,
    coords: &[TileCoord],
    since: u64,
    shift: u32,
) -> Vec<(DocRect, usize)> {
    let height = if slot == Slot::Normal && doc.derives_normal() {
        doc.changed_tiles(Channel::Height, since)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    rects_of(doc, slot, coords, &height, shift)
}

/// 端が反対側の端を読む Height → Normal（Wrap）か。部分では足りないので、全部を 1 つの矩形で作る。
pub(super) fn keeps_whole(doc: &Document, slot: Slot) -> bool {
    slot == Slot::Normal
        && doc.derives_normal()
        && doc.normal_settings().edges() == HeightEdgeMode::Wrap
}

/// タイルの矩形（縮めの境に合わせる。Normal の Height 側は 1 画素外へ。同じ矩形は 1 つに）と、それが表すタイルの数。
pub(super) fn rects_of(
    doc: &Document,
    slot: Slot,
    coords: &[TileCoord],
    height_coords: &[TileCoord],
    shift: u32,
) -> Vec<(DocRect, usize)> {
    let bounds = doc.bounds();
    if keeps_whole(doc, slot) && (!coords.is_empty() || !height_coords.is_empty()) {
        return vec![(bounds, coords.len() + height_coords.len())];
    }
    let mut out: Vec<(DocRect, usize)> = Vec::new();
    for c in coords {
        if let Some(r) = doc.tile_rect(*c) {
            out.push((align(r, shift, bounds), 1));
        }
    }
    for c in height_coords {
        if let Some(r) = doc.tile_rect(*c) {
            out.push((align(expand(r, 1, bounds), shift, bounds), 1));
        }
    }
    out.sort_by_key(|(r, _)| (r.y, r.x, r.width, r.height));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// 1 回に合成して上げる帯の高さの上限（画素。縮めの境の倍数）。全面を作るときの作業メモリの上限（4096 幅で 16 MiB）。
pub(super) const MAX_BAND_ROWS: u32 = 1024;

pub(super) fn area(r: &DocRect) -> u64 {
    r.width as u64 * r.height as u64
}

pub(super) fn union(a: &DocRect, b: &DocRect) -> DocRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    DocRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 上げる矩形をまとめる: 隣り合って合わせても余計な面積がほとんど出ない矩形を 1 つにし（1 回の書き込みの手間が大きい API でも、
/// 1 回のストロークの隣り合うタイルは 1 回で上がる）、全面のように大きくなったものは帯に切る（`keep_whole` なら切らない）。
/// 各矩形のタイルの数は足し合わせる。
pub(in crate::view3d) fn plan_regions(
    mut items: Vec<(DocRect, usize)>,
    shift: u32,
    bounds: DocRect,
    keep_whole: bool,
) -> Vec<(DocRect, usize)> {
    // まず、ぴったり隣り合うもの（同じ行の横の並び → 同じ列の縦の並び）を線形に 1 つにする（全面を作るときの 1000 を超えるタイル）。
    items.sort_by_key(|(r, _)| (r.y, r.x, r.width, r.height));
    let mut rows: Vec<(DocRect, usize)> = Vec::with_capacity(items.len());
    for (r, n) in items {
        match rows.last_mut() {
            Some((p, pn)) if p.y == r.y && p.height == r.height && p.x + p.width == r.x => {
                p.width += r.width;
                *pn += n;
            }
            _ => rows.push((r, n)),
        }
    }
    rows.sort_by_key(|(r, _)| (r.x, r.width, r.y, r.height));
    let mut items: Vec<(DocRect, usize)> = Vec::with_capacity(rows.len());
    for (r, n) in rows {
        match items.last_mut() {
            Some((p, pn)) if p.x == r.x && p.width == r.width && p.y + p.height == r.y => {
                p.height += r.height;
                *pn += n;
            }
            _ => items.push((r, n)),
        }
    }
    // 残ったものが少なければ、合わせても面積が 1.25 倍までのものを 1 つにする（斜めに離れたタイルは別のまま）
    while items.len() <= 64 {
        let mut merged = false;
        'search: for i in 0..items.len() {
            for j in i + 1..items.len() {
                let u = union(&items[i].0, &items[j].0);
                if area(&u) * 4 <= (area(&items[i].0) + area(&items[j].0)) * 5 {
                    let tiles = items[i].1 + items[j].1;
                    items[i] = (u, tiles);
                    items.remove(j);
                    merged = true;
                    break 'search;
                }
            }
        }
        if !merged {
            break;
        }
    }
    if keep_whole {
        return items;
    }
    let rows = (MAX_BAND_ROWS >> shift).max(1) << shift;
    let mut out = Vec::with_capacity(items.len());
    for (rect, tiles) in items {
        if rect.height <= rows {
            out.push((rect, tiles));
            continue;
        }
        let bands = rect.height.div_ceil(rows);
        let mut y = rect.y;
        for k in 0..bands {
            let h = rows.min(rect.y + rect.height - y);
            // タイルの数は帯の面積に比例して配る（合計は変えない）
            let (k, n) = (k as usize, bands as usize);
            let share = tiles * (k + 1) / n - tiles * k / n;
            out.push((
                align(DocRect::new(rect.x, y, rect.width, h), shift, bounds),
                share,
            ));
            y += h;
        }
    }
    out
}

// ───────── 縮め ─────────
