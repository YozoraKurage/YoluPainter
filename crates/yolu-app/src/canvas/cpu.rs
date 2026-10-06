//! CPU の表示の道: 文書を PAGE 画素の頁（テクスチャ）に分けて見せる。GPU で合成しないとき（効果・調整の層・独立のグループがある、
//! 予算を超える、装置が無い）の道で、タイルごとに状態を持ち、core が「変わった」と言うタイルを、見えている所から・時間の枠の中で
//! 順に合成して頁へ上げる（egui の `set_partial`。egui-wgpu がその範囲だけ `write_texture` する）。
//!
//! - 状態（[`TileState`]）はタイルごと。変わったタイルは `Stale`（前の絵のまま見せ、直す順を待つ）、初めて見せるタイルは `Missing`
//!   （透明のまま）。合成して上げると `Fresh`。最後はどのタイルも正確な絵（core の正本の合成と同じバイト）になる。
//! - 上げる順（[`Viewport`] があるとき）: 見えているタイルの変更（描いた所）→ 見えているタイルの初めて → 見えない所（近い順）。
//!   見えている所は、時間の枠（[`FrameBudget`]）の中で、表示域の中心に近い順に。枠を超えたら残りは次のフレーム（`request_repaint`）。
//!   見えない所は、見えている所が済んでから、小さな枠で（止まっている間に少しずつ）。
//! - [`Viewport`] を渡さないとき（試験・書き出しの前の同期）は、枠なしで全部を上げてから返る。
//! - タイルは 1 枚ずつ `composite_into` を呼ばず、束（`Document::composite_tiles`）で合成する（1 枚ずつ呼ぶより、1 枚あたりの時間が短い）。
//!   画素の値は `composite_into` と同じバイト。乗算済みへの変換は表示のためだけ（保存の正本ではない）。
//! - 粗い絵（歩幅で拾った合成。`Document::composite_coarse_tiles`）を、正確な絵ができるまでの仮の絵に使う。1 枚の小さなテクスチャ（文書の 1/歩幅）
//!   を頁の下に敷き、`CoarseCurrent`・`CoarseOld` のタイルの所だけ描く（頁はそのタイルの所が透明）。使うのは 2 つの場面:
//!   開いた直後など、見えているタイルを枠の中で正確に直しきれないとき（粗い絵を先に出し、見える所から正確に置き換える）と、スライダーを
//!   ドラッグしている間（`Document::is_coalescing`）に効果の出力がまだ評価されていないタイル（粗く評価して見せ、離したら正確に直す）。
//!   粗い絵は表示だけの近似で、保存・書き出し・Live Link へ出す値には入らない。

use std::time::{Duration, Instant};

use egui::{epaint::Vertex, Color32, ColorImage, Mesh, Painter, Pos2, Shape, TextureHandle, TextureOptions};
use rayon::prelude::*;

use super::display::PAGE;
use super::view::CanvasView;
use crate::engine::{Channel, Document, Rect as DocRect, TileCoord};

/// 見えている範囲と拡大。タイルを上げる順と、粗い絵の歩幅を決める。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// 画面に見える範囲を含む文書の矩形（回しているときは外接の矩形）。
    pub visible: DocRect,
    /// 文書の画素 1 つの画面の大きさ（点）。
    pub pixel_size: f32,
}

/// 1 フレームに合成に使ってよい時間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBudget {
    /// 見えているタイルに（最低 1 枚は、枠を超えても進める）。
    pub visible: Duration,
    /// 見えていないタイルに（見えているタイルが済んだあと）。
    pub background: Duration,
}

impl Default for FrameBudget {
    fn default() -> Self {
        FrameBudget {
            visible: Duration::from_millis(10),
            background: Duration::from_millis(2),
        }
    }
}

/// タイルの表示の状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileState {
    /// まだ上げていない（透明）。
    Missing,
    /// 文書が変わった。前の絵のまま見せていて、直す順を待つ。
    Stale,
    /// 正確な絵（コアの合成と同じバイト）。
    Fresh,
    /// 粗い絵を見せている（頁のこのタイルの所は透明）。正確な絵が要る。
    CoarseCurrent,
    /// 粗い絵を見せているが、文書がその後に変わった（粗い絵も古い）。
    CoarseOld,
}

/// 1 回の同期の結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// 上げた（合成し直した）タイルの数（正確な絵だけ。粗い絵は数えない）。
    pub tiles: usize,
    /// 全部を作り直したか。
    pub rebuilt: bool,
    /// まだ正確でないタイルの数。
    pub pending: usize,
    /// 粗い絵を作ったタイルの数。
    pub coarse: usize,
}

struct Page {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// 初めてタイルを上げるときに作る（見えない頁の透明のテクスチャを、最初に作って上げない）。
    texture: Option<TextureHandle>,
}

/// 粗い絵のテクスチャ（文書全体の 1/歩幅。画素 (i, j) は文書の (i·歩幅, j·歩幅)）。
struct Coarse {
    stride: u32,
    width: u32,
    height: u32,
    texture: TextureHandle,
}

/// 1 回に合成するタイルの数の上限（時間の枠の確かめの間隔でもある）。
const CHUNK_MAX: usize = 64;
/// 見えていないタイルから 1 フレームで選ぶ数の上限（近い順に選ぶ。全部を並べ替えない）。
const BACKGROUND_PICK: usize = 256;
/// 粗い絵の歩幅の範囲（これより細かい絵は、正確な絵を直に作るほうが速い）。
const STRIDE_MIN: u32 = 4;
const STRIDE_MAX: u32 = 16;
/// 粗い絵のテクスチャの一辺の上限（表示のテクスチャの上限に収める）。
const COARSE_SIDE_MAX: u32 = 4096;

pub(super) struct CpuCanvas {
    doc_id: u128,
    channel: Option<Channel>,
    serial: u64,
    size: (u32, u32),
    tile_size: u32,
    cols: u32,
    rows: u32,
    nearest: bool,
    pages: Vec<Page>,
    state: Vec<TileState>,
    coarse: Option<Coarse>,
    /// 1 タイルを合成して上げるのにかかる秒（移動平均。時間の枠に入るタイルの数の見積り）。正確な絵と粗い絵で別々に。
    per_tile: f64,
    per_tile_coarse: f64,
}

pub(super) fn options(nearest: bool) -> TextureOptions {
    TextureOptions {
        magnification: if nearest {
            egui::TextureFilter::Nearest
        } else {
            egui::TextureFilter::Linear
        },
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    }
}

/// 粗い絵は、拡げて見せるので滑らかに。
fn coarse_options() -> TextureOptions {
    TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    }
}

/// straight RGBA8 の矩形を、乗算済みの egui の画像へ。
pub(super) fn to_image(width: u32, height: u32, straight: &[u8]) -> ColorImage {
    let pixels = straight
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    ColorImage::new([width as usize, height as usize], pixels)
}

impl Default for CpuCanvas {
    fn default() -> Self {
        CpuCanvas {
            doc_id: 0,
            channel: None,
            serial: 0,
            size: (0, 0),
            tile_size: 0,
            cols: 0,
            rows: 0,
            nearest: false,
            pages: Vec::new(),
            state: Vec::new(),
            coarse: None,
            per_tile: 0.0005,
            per_tile_coarse: 0.0001,
        }
    }
}

impl CpuCanvas {
    pub(super) fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// 前の文書の絵を全部捨てる（次の `sync` が全部を作り直す）。
    pub(super) fn clear(&mut self) {
        self.pages.clear();
        self.state.clear();
        self.coarse = None;
        self.doc_id = 0;
        self.channel = None;
        self.serial = 0;
    }

    /// 頁の数。
    pub(super) fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// 頁の文書の矩形（試験用）。
    pub(super) fn page_rect(&self, index: usize) -> Option<DocRect> {
        self.pages
            .get(index)
            .map(|p| DocRect::new(p.x, p.y, p.width, p.height))
    }

    /// 頁のテクスチャの番号（描くとき・試験用）。
    pub(super) fn page_texture(&self, index: usize) -> Option<egui::TextureId> {
        self.pages
            .get(index)
            .and_then(|p| p.texture.as_ref().map(|t| t.id()))
    }

    /// 粗い絵のテクスチャの番号と歩幅（試験用）。
    #[cfg(test)]
    pub(super) fn coarse_texture(&self) -> Option<(egui::TextureId, u32)> {
        self.coarse.as_ref().map(|c| (c.texture.id(), c.stride))
    }

    /// 今の画素の角を見せる補間か。
    pub(super) fn is_nearest(&self) -> bool {
        self.nearest
    }

    /// まだ正確でないタイルの数。
    pub(super) fn pending(&self) -> usize {
        self.state.iter().filter(|s| **s != TileState::Fresh).count()
    }

    /// 矩形（文書の画素）にかかるタイルのうち、まだ何も見せていない（`Missing`・`Stale` で、粗い絵も無い）ものの数。
    pub(super) fn unshown_in(&self, rect: &DocRect) -> usize {
        self.count_in(rect, |s| matches!(s, TileState::Missing | TileState::Stale))
    }

    /// 矩形（文書の画素）にかかるタイルのうち、まだ正確でないものの数。
    pub(super) fn pending_in(&self, rect: &DocRect) -> usize {
        self.count_in(rect, |s| s != TileState::Fresh)
    }

    fn count_in(&self, rect: &DocRect, pick: impl Fn(TileState) -> bool) -> usize {
        let ts = self.tile_size.max(1);
        let (tx0, ty0) = (rect.x / ts, rect.y / ts);
        let (tx1, ty1) = (
            (rect.x + rect.width).div_ceil(ts).min(self.cols),
            (rect.y + rect.height).div_ceil(ts).min(self.rows),
        );
        let mut n = 0;
        for ty in ty0..ty1 {
            for tx in tx0..tx1 {
                if pick(self.state[(ty * self.cols + tx) as usize]) {
                    n += 1;
                }
            }
        }
        n
    }

    /// タイルの状態（試験用）。
    pub(super) fn tile_state(&self, coord: TileCoord) -> Option<TileState> {
        (coord.x < self.cols && coord.y < self.rows)
            .then(|| self.state[(coord.y * self.cols + coord.x) as usize])
    }

    fn index(&self, coord: TileCoord) -> Option<usize> {
        (coord.x < self.cols && coord.y < self.rows)
            .then(|| (coord.y * self.cols + coord.x) as usize)
    }

    fn coord(&self, index: usize) -> TileCoord {
        TileCoord::new(index as u32 % self.cols, index as u32 / self.cols)
    }

    /// 頁を作り直し、どのタイルも `Missing` にする。
    fn reset(&mut self, doc: &Document, channel: Channel, nearest: bool) {
        let (w, h) = (doc.width(), doc.height());
        self.nearest = nearest;
        self.doc_id = doc.id();
        self.channel = Some(channel);
        self.size = (w, h);
        self.tile_size = doc.tile_size();
        self.cols = w.div_ceil(self.tile_size);
        self.rows = h.div_ceil(self.tile_size);
        self.state = vec![TileState::Missing; (self.cols * self.rows) as usize];
        self.coarse = None;
        self.pages.clear();
        let mut y = 0;
        while y < h {
            let mut x = 0;
            while x < w {
                let (pw, ph) = (PAGE.min(w - x), PAGE.min(h - y));
                self.pages.push(Page {
                    x,
                    y,
                    width: pw,
                    height: ph,
                    texture: None,
                });
                x += PAGE;
            }
            y += PAGE;
        }
    }

    /// 粗い絵の歩幅: 画面の 1 点に粗い画素が 1 つ以上入る大きさ（拡大の逆数以下の 2 のべき）。範囲は 4〜16 で、テクスチャの一辺が上限に
    /// 収まる大きさ以上、タイルの一辺の約数に切り下げる。粗い絵を使えない（タイルの一辺が 4 の倍数でない）ときは None。
    fn pick_stride(&self, pixel_size: f32) -> Option<u32> {
        let want = if pixel_size >= 1.0 {
            STRIDE_MIN
        } else {
            let n = (1.0 / pixel_size.max(1e-6)).floor().max(1.0) as u32;
            (1u32 << n.ilog2()).clamp(STRIDE_MIN, STRIDE_MAX)
        };
        let side = self.size.0.max(self.size.1).div_ceil(COARSE_SIDE_MAX).max(1);
        let mut s = want.max(side.next_power_of_two());
        while s >= STRIDE_MIN && !self.tile_size.is_multiple_of(s) {
            s /= 2;
        }
        (s >= STRIDE_MIN).then_some(s)
    }

    /// 文書の変わった所を頁へ上げる。`schedule` があれば見えている所から時間の枠の中で、無ければ全部を上げてから返る。
    pub(super) fn sync(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        channel: Channel,
        nearest: bool,
        schedule: Option<(&Viewport, &FrameBudget)>,
    ) -> Report {
        let changed = doc.changed_tiles(channel, self.serial);
        let serial = doc.change_serial();
        let rebuilt = self.size != (doc.width(), doc.height())
            || self.tile_size != doc.tile_size()
            || self.pages.is_empty()
            || self.nearest != nearest
            || self.doc_id != doc.id()
            || self.channel != Some(channel)
            || changed.is_none();
        if rebuilt {
            self.reset(doc, channel, nearest);
        } else {
            for coord in changed.unwrap_or_default() {
                if let Some(i) = self.index(coord) {
                    self.state[i] = match self.state[i] {
                        TileState::Fresh => TileState::Stale,
                        TileState::CoarseCurrent => TileState::CoarseOld,
                        other => other,
                    };
                }
            }
        }
        self.serial = serial;

        let started = Instant::now();
        let (mut uploaded, mut coarse_tiles) = (0usize, 0usize);
        // ドラッグの間、見えている所の粗い絵が済んだら、離すまですることが無い（再描画を続けて回さない）
        let mut waiting_for_release = false;
        match schedule {
            None => {
                let all: Vec<usize> = (0..self.state.len())
                    .filter(|&i| self.state[i] != TileState::Fresh)
                    .collect();
                let r = self.run(ctx, doc, channel, &all, None, started, None, false);
                uploaded += r.0;
            }
            Some((viewport, budget)) => {
                // 粗い絵の歩幅が変わったら粗い絵を捨てる（その絵を見せていたタイルは、直す順を待つ）
                let stride = self.pick_stride(viewport.pixel_size);
                if self.coarse.as_ref().is_some_and(|c| Some(c.stride) != stride) {
                    self.coarse = None;
                    for s in &mut self.state {
                        if matches!(*s, TileState::CoarseCurrent | TileState::CoarseOld) {
                            *s = TileState::Missing;
                        }
                    }
                }
                let interactive = doc.is_coalescing();
                let (mut visible, background) = self.order(viewport);
                // ドラッグの間は、見えている所だけ。粗い絵がもう新しいタイルは、離すまで触らない
                if interactive {
                    visible.retain(|&i| self.state[i] != TileState::CoarseCurrent);
                }
                // 見えているタイルを枠の中で正確に直しきれないなら、先に粗い絵を敷く
                if let Some(s) = stride.filter(|_| !interactive) {
                    let missing = visible
                        .iter()
                        .filter(|&&i| self.state[i] == TileState::Missing)
                        .count();
                    if missing as f64 * self.per_tile > budget.visible.as_secs_f64() * 1.5 {
                        coarse_tiles += self.underlay(ctx, doc, channel, &visible, s, budget);
                    }
                }
                let stride_now = stride.filter(|_| interactive);
                let (n, c) = self.run(
                    ctx,
                    doc,
                    channel,
                    &visible,
                    Some(budget.visible),
                    started,
                    stride_now,
                    interactive,
                );
                uploaded += n;
                coarse_tiles += c;
                let exact = visible.iter().all(|&i| self.state[i] == TileState::Fresh);
                // ドラッグの間は、粗い絵でよい（離したら正確に直す）
                waiting_for_release = interactive
                    && visible
                        .iter()
                        .all(|&i| matches!(self.state[i], TileState::Fresh | TileState::CoarseCurrent));
                if exact && !interactive {
                    let background_start = Instant::now();
                    uploaded += self
                        .run(
                            ctx,
                            doc,
                            channel,
                            &background,
                            Some(budget.background),
                            background_start,
                            None,
                            false,
                        )
                        .0;
                }
            }
        }
        // 粗い絵がいらなくなったら手放す
        if self.coarse.is_some()
            && !self
                .state
                .iter()
                .any(|s| matches!(s, TileState::CoarseCurrent | TileState::CoarseOld))
        {
            self.coarse = None;
        }
        let pending = self.pending();
        if pending > 0 && !waiting_for_release {
            ctx.request_repaint();
        }
        Report {
            tiles: uploaded,
            rebuilt,
            pending,
            coarse: coarse_tiles,
        }
    }

    /// 上げる順: 見えているタイル（変更が先、次に初めて。どちらも表示域の中心に近い順）と、見えていないタイル（表示域に近い順）。
    fn order(&self, viewport: &Viewport) -> (Vec<usize>, Vec<usize>) {
        let ts = self.tile_size.max(1);
        let v = viewport.visible;
        let (tx0, ty0) = (v.x / ts, v.y / ts);
        let (tx1, ty1) = (
            (v.x + v.width).div_ceil(ts).min(self.cols),
            (v.y + v.height).div_ceil(ts).min(self.rows),
        );
        let (cx, cy) = (
            (v.x as f64 + v.width as f64 * 0.5) / ts as f64 - 0.5,
            (v.y as f64 + v.height as f64 * 0.5) / ts as f64 - 0.5,
        );
        let mut visible: Vec<(u8, u64, usize)> = Vec::new();
        let mut background: Vec<(u64, usize)> = Vec::new();
        for (i, s) in self.state.iter().enumerate() {
            if *s == TileState::Fresh {
                continue;
            }
            let (tx, ty) = (i as u32 % self.cols, i as u32 / self.cols);
            let inside = tx >= tx0 && tx < tx1 && ty >= ty0 && ty < ty1;
            if inside {
                let d = ((tx as f64 - cx).powi(2) + (ty as f64 - cy).powi(2)) * 1024.0;
                // 描いた所（変わったタイル）が先。初めて見せるタイル・粗い絵のタイルは、そのあと
                let class = if matches!(*s, TileState::Stale | TileState::CoarseOld) { 0 } else { 1 };
                visible.push((class, d as u64, i));
            } else {
                // 表示域の矩形までの距離（タイルの単位）
                let dx = (tx0.saturating_sub(tx)).max(tx.saturating_sub(tx1.saturating_sub(1)));
                let dy = (ty0.saturating_sub(ty)).max(ty.saturating_sub(ty1.saturating_sub(1)));
                background.push(((dx as u64).pow(2) + (dy as u64).pow(2), i));
            }
        }
        visible.sort_unstable();
        if background.len() > BACKGROUND_PICK {
            background.select_nth_unstable(BACKGROUND_PICK);
            background.truncate(BACKGROUND_PICK);
        }
        background.sort_unstable();
        (
            visible.into_iter().map(|(_, _, i)| i).collect(),
            background.into_iter().map(|(_, i)| i).collect(),
        )
    }

    /// まだ何も見せていないタイル（`Missing`）に、粗い絵を敷く（見えているタイルが先。時間は枠の半分まで）。作ったタイルの数を返す。
    fn underlay(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        channel: Channel,
        visible: &[usize],
        stride: u32,
        budget: &FrameBudget,
    ) -> usize {
        let mut list: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|&i| self.state[i] == TileState::Missing)
            .collect();
        let seen: std::collections::HashSet<usize> = list.iter().copied().collect();
        list.extend(
            (0..self.state.len()).filter(|&i| self.state[i] == TileState::Missing && !seen.contains(&i)),
        );
        let started = Instant::now();
        let limit = budget.visible / 2;
        let mut made = 0usize;
        let mut pos = 0usize;
        while pos < list.len() {
            if made > 0 && started.elapsed() >= limit {
                break;
            }
            let n = if made == 0 {
                CHUNK_MAX
            } else {
                let room = limit.saturating_sub(started.elapsed()).as_secs_f64();
                ((room / self.per_tile_coarse.max(1e-6)) as usize).clamp(1, CHUNK_MAX)
            };
            let chunk = &list[pos..(pos + n).min(list.len())];
            pos += chunk.len();
            made += self.coarse_chunk(ctx, doc, channel, chunk, stride);
        }
        made
    }

    /// chunk のタイルを粗く合成して粗い絵のテクスチャへ上げ、`CoarseCurrent` にする。作った枚数を返す。
    fn coarse_chunk(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        channel: Channel,
        chunk: &[usize],
        stride: u32,
    ) -> usize {
        let coords: Vec<TileCoord> = chunk.iter().map(|&i| self.coord(i)).collect();
        let t = Instant::now();
        let Ok(tiles) = doc.composite_coarse_tiles(channel, &coords, stride) else {
            return 0;
        };
        for tile in &tiles {
            let i = self.index(tile.coord).expect("画布の中のタイル");
            // 頁に前の絵があるタイルは、頁のその所を透明にして、粗い絵が見えるようにする
            if matches!(self.state[i], TileState::Stale | TileState::Fresh) {
                let zeros = vec![0u8; (tile.rect.width * tile.rect.height * 4) as usize];
                for (page, pos, image) in self.pieces(tile.rect, &zeros) {
                    self.put(ctx, page, pos, image);
                }
            }
            self.coarse_put(ctx, tile);
            self.state[i] = TileState::CoarseCurrent;
        }
        if !tiles.is_empty() {
            let each = t.elapsed().as_secs_f64() / tiles.len() as f64;
            self.per_tile_coarse = self.per_tile_coarse * 0.7 + each * 0.3;
        }
        tiles.len()
    }

    /// 粗い絵のテクスチャへ 1 タイルぶんを上げる（テクスチャが無ければ、透明で作ってから）。
    fn coarse_put(&mut self, ctx: &egui::Context, tile: &crate::engine::CompositedTile) {
        let (stride, size) = (tile.stride, self.size);
        let coarse = self.coarse.get_or_insert_with(|| {
            let (w, h) = (size.0.div_ceil(stride), size.1.div_ceil(stride));
            let empty = ColorImage::filled([w as usize, h as usize], Color32::TRANSPARENT);
            Coarse {
                stride,
                width: w,
                height: h,
                texture: ctx.load_texture("canvas-coarse", empty, coarse_options()),
            }
        });
        let (w, h) = tile.size();
        coarse.texture.set_partial(
            [(tile.rect.x / stride) as usize, (tile.rect.y / stride) as usize],
            to_image(w, h, &tile.pixels),
            coarse_options(),
        );
    }

    /// list のタイルを順に合成して頁へ上げる。budget があれば、その時間を超えたところで止める（最初の 1 束は必ず進める）。
    /// `coarse_stride` があって、束の効果が評価されていない（ドラッグの途中）なら、その束は粗く合成する。
    /// （正確な絵を上げたタイルの数, 粗い絵を作ったタイルの数）を返す。
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        channel: Channel,
        list: &[usize],
        budget: Option<Duration>,
        started: Instant,
        coarse_stride: Option<u32>,
        interactive: bool,
    ) -> (usize, usize) {
        let (mut uploaded, mut coarse_made) = (0usize, 0usize);
        let mut pos = 0usize;
        let mut first = true;
        while pos < list.len() {
            let n = match budget {
                None => CHUNK_MAX,
                Some(b) => {
                    let elapsed = started.elapsed();
                    if elapsed >= b && !first {
                        break;
                    }
                    let room = b.saturating_sub(elapsed).as_secs_f64();
                    let each = if coarse_stride.is_some() {
                        self.per_tile_coarse
                    } else {
                        self.per_tile
                    };
                    ((room / each.max(1e-6)) as usize).clamp(1, CHUNK_MAX)
                }
            };
            first = false;
            let chunk = &list[pos..(pos + n).min(list.len())];
            pos += chunk.len();
            let coords: Vec<TileCoord> = chunk.iter().map(|&i| self.coord(i)).collect();
            // ドラッグの途中で、効果の出力をまだ評価していないタイルは、粗く合成して見せる（離したら正確に直す）
            if let Some(s) = coarse_stride {
                if interactive && doc.effects_pending(channel, &coords) {
                    coarse_made += self.coarse_chunk(ctx, doc, channel, chunk, s);
                    continue;
                }
            }
            let t = Instant::now();
            if let Ok(tiles) = doc.composite_tiles(channel, &coords) {
                // 乗算済みの画像への変換をタイルごとにワーカーへ分け、頁へ上げるのは 1 つのスレッドで
                let pieces: Vec<Vec<(usize, [usize; 2], ColorImage)>> = if tiles.len() < 4 {
                    tiles.iter().map(|t| self.pieces(t.rect, &t.pixels)).collect()
                } else {
                    let me = &*self;
                    tiles.par_iter().map(|t| me.pieces(t.rect, &t.pixels)).collect()
                };
                for (page, pos, image) in pieces.into_iter().flatten() {
                    self.put(ctx, page, pos, image);
                }
                uploaded += tiles.len();
            } // 合成できない（チャンネルが無くなったなど）タイルは、次の変更まで見直さない
            for &i in chunk {
                self.state[i] = TileState::Fresh;
            }
            let each = t.elapsed().as_secs_f64() / chunk.len() as f64;
            self.per_tile = self.per_tile * 0.7 + each * 0.3;
        }
        (uploaded, coarse_made)
    }

    /// 合成した矩形（straight RGBA8、行は下から上）を、重なる頁ごとの乗算済みの画像（頁の番号・頁の中の位置・画像）に切り分ける。
    fn pieces(&self, rect: DocRect, straight: &[u8]) -> Vec<(usize, [usize; 2], ColorImage)> {
        let mut out = Vec::with_capacity(1);
        for (i, page) in self.pages.iter().enumerate() {
            let x0 = rect.x.max(page.x);
            let y0 = rect.y.max(page.y);
            let x1 = (rect.x + rect.width).min(page.x + page.width);
            let y1 = (rect.y + rect.height).min(page.y + page.height);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let image = if (x0, y0, x1, y1) == (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height)
            {
                to_image(rect.width, rect.height, straight)
            } else {
                // 頁をまたぐタイル（頁の大きさがタイルの倍数でないとき）: 重なる行と列だけを切り出す
                let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
                let mut cut = Vec::with_capacity(w * h * 4);
                for y in y0..y1 {
                    let at = (((y - rect.y) * rect.width + (x0 - rect.x)) * 4) as usize;
                    cut.extend_from_slice(&straight[at..at + w * 4]);
                }
                to_image(w as u32, h as u32, &cut)
            };
            out.push((i, [(x0 - page.x) as usize, (y0 - page.y) as usize], image));
        }
        out
    }

    /// 画像を頁へ上げる（頁のテクスチャが無ければ、透明で作ってから）。
    fn put(&mut self, ctx: &egui::Context, page: usize, pos: [usize; 2], image: ColorImage) {
        let nearest = self.nearest;
        let p = &mut self.pages[page];
        let texture = p.texture.get_or_insert_with(|| {
            let empty =
                ColorImage::filled([p.width as usize, p.height as usize], Color32::TRANSPARENT);
            ctx.load_texture(
                format!("canvas-page-{}-{}", p.x, p.y),
                empty,
                options(nearest),
            )
        });
        texture.set_partial(pos, image, options(nearest));
    }

    /// 粗い絵（頁の下）と頁を描く（市松は呼び手が先に描く）。
    pub(super) fn paint(&self, painter: &Painter, view: &CanvasView) {
        if let Some(coarse) = &self.coarse {
            self.paint_coarse(painter, view, coarse);
        }
        let full = [
            Pos2::new(0.0, 0.0),
            Pos2::new(1.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(0.0, 1.0),
        ];
        for page in &self.pages {
            let Some(texture) = &page.texture else {
                continue;
            };
            let (x0, y0) = (page.x as f64, page.y as f64);
            let (x1, y1) = (x0 + page.width as f64, y0 + page.height as f64);
            // テクスチャの行 0 が文書の y0（下）なので、UV の v はそのまま y に比例する
            quad(
                painter,
                texture.id(),
                view,
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
                full,
            );
        }
    }

    /// 粗い絵を見せるタイルの所だけ、粗い絵のテクスチャの対応する範囲を描く（画面に入らないタイルは描かない）。
    fn paint_coarse(&self, painter: &Painter, view: &CanvasView, coarse: &Coarse) {
        let ts = self.tile_size as f64;
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        let (tw, th) = (
            coarse.width as f64 * coarse.stride as f64,
            coarse.height as f64 * coarse.stride as f64,
        );
        let screen = painter.clip_rect();
        let mut mesh = Mesh::with_texture(coarse.texture.id());
        for (i, s) in self.state.iter().enumerate() {
            if !matches!(s, TileState::CoarseCurrent | TileState::CoarseOld) {
                continue;
            }
            let c = self.coord(i);
            let (x0, y0) = (c.x as f64 * ts, c.y as f64 * ts);
            let (x1, y1) = ((x0 + ts).min(w), (y0 + ts).min(h));
            let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
            let pos = corners.map(|(x, y)| view.to_screen(x, y));
            let bounds = egui::Rect::from_points(&pos);
            if !bounds.intersects(screen) {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for (p, (x, y)) in pos.iter().zip(corners) {
                mesh.vertices.push(Vertex {
                    pos: *p,
                    uv: Pos2::new((x / tw) as f32, (y / th) as f32),
                    color: Color32::WHITE,
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }
}

/// 文書の矩形の 4 隅を写して、テクスチャの四角として描く。
pub(super) fn quad(
    painter: &Painter,
    texture: egui::TextureId,
    view: &CanvasView,
    corners: [(f64, f64); 4],
    uvs: [Pos2; 4],
) {
    let mut mesh = Mesh::with_texture(texture);
    for (c, uv) in corners.iter().zip(uvs) {
        mesh.vertices.push(Vertex {
            pos: view.to_screen(c.0, c.1),
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{BlendMode, Document, LayerId, Rgba8, RowOrder};
    use egui::TextureId;
    use std::collections::HashMap;

    /// egui へ上げた頁の内容を、差分（`TexturesDelta`）から組み立てて持つ（描画器の代わり）。
    #[derive(Default)]
    struct Mirror {
        images: HashMap<TextureId, ColorImage>,
    }

    impl Mirror {
        fn apply(&mut self, ctx: &egui::Context) {
            let mut delta = ctx.tex_manager().write().take_delta();
            for (id, deltas) in delta.set.iter() {
                for d in deltas {
                    let egui::ImageData::Color(patch) = &d.image;
                    match d.pos {
                        None => {
                            self.images.insert(*id, (**patch).clone());
                        }
                        Some([x, y]) => {
                            let target = self.images.get_mut(id).expect("先に全体を置く");
                            for row in 0..patch.size[1] {
                                let to = (y + row) * target.size[0] + x;
                                let from = row * patch.size[0];
                                target.pixels[to..to + patch.size[0]]
                                    .copy_from_slice(&patch.pixels[from..from + patch.size[0]]);
                            }
                        }
                    }
                }
            }
            delta.clear(); // 描画器に渡した（落とすとき、渡していない差分があると panic する）
        }
    }

    fn painted(w: u32, h: u32, tile: u32) -> (Document, LayerId) {
        let mut doc = Document::with_tile_size(w, h, tile).unwrap();
        let a = doc.add_layer("a").unwrap();
        let b = doc.add_layer("b").unwrap();
        doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(b, 0.6, false).unwrap();
        for y in (0..h).step_by(3) {
            for x in (0..w).step_by(2) {
                let v = (x * 7 + y * 13) as u8;
                doc.set_pixel(a, x, y, Rgba8::new(v, 255 - v, v / 2, 200)).unwrap();
                doc.set_pixel(b, x, y, Rgba8::new(255 - v, v, 90, (y % 255) as u8)).unwrap();
            }
        }
        doc.clear_history().unwrap();
        (doc, a)
    }

    /// 頁ごとに、いまの文書の合成（乗算済みへ変換したもの）と上げた内容が同じか。
    fn assert_shows_the_document(canvas: &CpuCanvas, mirror: &Mirror, doc: &Document) {
        for (i, page) in canvas.pages.iter().enumerate() {
            let rect = DocRect::new(page.x, page.y, page.width, page.height);
            let mut straight = vec![0u8; (rect.width * rect.height * 4) as usize];
            doc.composite_into(Channel::Color, rect, &mut straight, RowOrder::BottomUp)
                .unwrap();
            let want = to_image(rect.width, rect.height, &straight);
            let id = page.texture.as_ref().map(|t| t.id());
            let Some(got) = id.and_then(|id| mirror.images.get(&id)) else {
                // 上げたタイルが無い頁は、テクスチャが無い。文書の合成も全部透明のはず
                assert!(straight.iter().all(|b| *b == 0), "頁 {i} が上がっていない");
                continue;
            };
            assert_eq!(got.size, want.size, "頁 {i}");
            assert!(got.pixels == want.pixels, "頁 {i} の絵が合成と違う");
        }
    }

    fn viewport(x: u32, y: u32, w: u32, h: u32) -> Viewport {
        Viewport {
            visible: DocRect::new(x, y, w, h),
            pixel_size: 1.0,
        }
    }

    const ZERO: FrameBudget = FrameBudget {
        visible: Duration::ZERO,
        background: Duration::ZERO,
    };

    #[test]
    fn visible_tiles_come_first_and_everything_ends_exact() {
        let ctx = egui::Context::default();
        let (doc, _) = painted(256, 192, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        // 見えるのは中ほどの 2 × 1 タイル（x 64..192、y 64..128）。枠は 0 なので、1 フレームに 1 タイルずつ進む
        let vp = viewport(64, 64, 128, 64);
        let inside = |c: &TileCoord| (1..3).contains(&c.x) && c.y == 1;
        let mut frames = 0;
        let mut first_visible_done = None;
        for frame in 0..40 {
            let report = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &ZERO)));
            mirror.apply(&ctx);
            if report.tiles == 0 && report.pending == 0 {
                break;
            }
            // 枠が 0 でも 1 枚は必ず進む。見えているタイルが済んだフレームだけは、見えていない所へも 1 枚進む
            assert!((1..=2).contains(&report.tiles), "{}", report.tiles);
            frames += 1;
            let fresh: Vec<TileCoord> = (0..3)
                .flat_map(|y| (0..4).map(move |x| TileCoord::new(x, y)))
                .filter(|c| canvas.tile_state(*c) == Some(TileState::Fresh))
                .collect();
            let visible_done = fresh.iter().filter(|c| inside(c)).count() == 2;
            if visible_done && first_visible_done.is_none() {
                first_visible_done = Some(frame);
            }
            // 見えていないタイルが上がっているのは、見えているタイルが全部上がってから
            if fresh.iter().any(|c| !inside(c)) {
                assert!(visible_done, "frame {frame}: {fresh:?}");
            }
        }
        assert_eq!(first_visible_done, Some(1), "見えている 2 枚は 2 フレームで済む");
        assert_eq!(frames, 11, "11 フレーム（見えている所が済んだフレームは 2 枚）");
        assert_eq!(canvas.pending(), 0);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn a_changed_tile_is_redone_before_a_tile_that_was_never_shown() {
        let ctx = egui::Context::default();
        let (mut doc, a) = painted(256, 192, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let vp = viewport(0, 0, 256, 192); // 全部が見えている
        // 全部を上げる
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &ZERO))).pending > 0 {}
        mirror.apply(&ctx);
        assert_shows_the_document(&canvas, &mirror, &doc);
        // 作り直しで全部が上げ直しになる（補間を替える）。枠が 0 なので 1 枚だけ正確に上がり、残りは粗い絵で見える
        let r = canvas.sync(&ctx, &doc, Channel::Color, true, Some((&vp, &ZERO)));
        assert!(r.rebuilt && r.pending == 11);
        let edited = TileCoord::new(3, 2); // 表示域の中心から一番遠い隅
        assert_eq!(canvas.tile_state(edited), Some(TileState::CoarseCurrent));
        // 遠いタイルを描く: 粗い絵も古くなる。次のフレームは、まだ見せていない近いタイルより先に、これを直す
        doc.set_pixel(a, 250, 180, Rgba8::new(1, 2, 3, 255)).unwrap();
        let r = canvas.sync(&ctx, &doc, Channel::Color, true, Some((&vp, &ZERO)));
        assert_eq!(r.tiles, 1);
        assert_eq!(canvas.tile_state(edited), Some(TileState::Fresh));
        mirror.apply(&ctx);
        while canvas.sync(&ctx, &doc, Channel::Color, true, Some((&vp, &ZERO))).pending > 0 {}
        mirror.apply(&ctx);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn stale_tiles_keep_their_old_picture_until_they_are_redone() {
        let ctx = egui::Context::default();
        let (mut doc, a) = painted(128, 128, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let vp = viewport(0, 0, 128, 128);
        canvas.sync(&ctx, &doc, Channel::Color, false, None);
        mirror.apply(&ctx);
        doc.set_pixel(a, 5, 5, Rgba8::new(9, 9, 9, 255)).unwrap();
        doc.set_pixel(a, 100, 100, Rgba8::new(9, 9, 9, 255)).unwrap();
        // 枠が 0: 1 枚ずつ。まだ直していないタイルは Stale のまま、前の絵が残る
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &ZERO)));
        assert_eq!((r.tiles, r.pending), (1, 1));
        let stale: Vec<_> = (0..2)
            .flat_map(|y| (0..2).map(move |x| TileCoord::new(x, y)))
            .filter(|c| canvas.tile_state(*c) == Some(TileState::Stale))
            .collect();
        assert_eq!(stale.len(), 1);
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &ZERO)));
        assert_eq!((r.tiles, r.pending), (1, 0));
        mirror.apply(&ctx);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn without_a_viewport_everything_is_uploaded_before_returning() {
        let ctx = egui::Context::default();
        let (doc, _) = painted(300, 200, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, None);
        mirror.apply(&ctx);
        assert_eq!((r.tiles, r.pending, r.rebuilt), (5 * 4, 0, true));
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn tiles_that_straddle_pages_are_cut_per_page() {
        // タイルが頁の大きさの約数でない（頁は 2048 画素）: 2048 の手前のタイルが 2 つの頁にまたがる
        let ctx = egui::Context::default();
        let (doc, _) = painted(2100, 40, 100);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        canvas.sync(&ctx, &doc, Channel::Color, false, None);
        mirror.apply(&ctx);
        assert_eq!(canvas.page_count(), 2);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    // ───────── 粗い絵 ─────────

    fn coarse_expected(doc: &Document, size: (u32, u32), stride: u32, coords: &[TileCoord]) -> ColorImage {
        let (w, h) = (size.0.div_ceil(stride) as usize, size.1.div_ceil(stride) as usize);
        let mut image = ColorImage::filled([w, h], Color32::TRANSPARENT);
        for t in doc.composite_coarse_tiles(Channel::Color, coords, stride).unwrap() {
            let (tw, th) = t.size();
            let small = to_image(tw, th, &t.pixels);
            let (x0, y0) = ((t.rect.x / stride) as usize, (t.rect.y / stride) as usize);
            for r in 0..th as usize {
                for c in 0..tw as usize {
                    image.pixels[(y0 + r) * w + x0 + c] = small.pixels[r * tw as usize + c];
                }
            }
        }
        image
    }

    fn every_tile(canvas: &CpuCanvas) -> Vec<TileCoord> {
        (0..canvas.rows)
            .flat_map(|y| (0..canvas.cols).map(move |x| TileCoord::new(x, y)))
            .collect()
    }

    #[test]
    fn an_unfinished_open_shows_a_coarse_picture_and_ends_exact_and_without_it() {
        let ctx = egui::Context::default();
        let (doc, _) = painted(512, 512, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let vp = viewport(0, 0, 512, 512);
        // 枠が 0: 見えているタイルを正確に直しきれないので、先に粗い絵を敷く
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &ZERO)));
        mirror.apply(&ctx);
        assert_eq!(r.coarse, 64, "全部のタイルに粗い絵");
        let (id, stride) = canvas.coarse_texture().expect("粗い絵のテクスチャ");
        assert_eq!(stride, 4);
        let coords = every_tile(&canvas);
        let fresh: Vec<_> = coords
            .iter()
            .filter(|c| canvas.tile_state(**c) == Some(TileState::Fresh))
            .collect();
        assert_eq!(fresh.len(), 1, "正確なのは 1 枚だけ");
        let want = coarse_expected(&doc, (512, 512), 4, &coords);
        assert!(mirror.images[&id].pixels == want.pixels, "粗い絵が粗い合成と同じ");
        // 粗い絵のタイルは、頁の所が透明
        let page = &mirror.images[&canvas.page_texture(0).unwrap()];
        let coarse_tile = coords
            .iter()
            .find(|c| canvas.tile_state(**c) == Some(TileState::CoarseCurrent))
            .unwrap();
        let at = (coarse_tile.y * 64 * 512 + coarse_tile.x * 64) as usize;
        assert_eq!(page.pixels[at], Color32::TRANSPARENT);
        // 落ち着くと、全部が正確で、粗い絵は手放す
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &FrameBudget::default()))).pending > 0 {}
        mirror.apply(&ctx);
        assert!(canvas.coarse_texture().is_none());
        assert_eq!(canvas.pending(), 0);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn a_small_document_that_fits_the_frame_never_needs_the_coarse_picture() {
        let ctx = egui::Context::default();
        let (doc, _) = painted(128, 128, 64);
        let mut canvas = CpuCanvas::default();
        let vp = viewport(0, 0, 128, 128);
        let big = FrameBudget {
            visible: Duration::from_secs(5),
            background: Duration::from_secs(5),
        };
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &big)));
        assert_eq!((r.coarse, r.pending), (0, 0));
        assert!(canvas.coarse_texture().is_none());
    }

    fn with_blur(w: u32, h: u32, tile: u32) -> (Document, LayerId, yolu_core::FilterId, LayerId) {
        let (mut doc, a) = painted(w, h, tile);
        let other = doc.add_layer("other").unwrap();
        for y in (0..h).step_by(5) {
            for x in (0..w).step_by(4) {
                doc.set_pixel(other, x, y, Rgba8::new(200, 30, 60, 180)).unwrap();
            }
        }
        let fx = doc
            .add_filter(
                a,
                yolu_core::FilterTarget::Content,
                yolu_core::effects::FilterSpec::new(yolu_core::effects::EffectSettings::blur(6)),
            )
            .unwrap();
        doc.clear_history().unwrap();
        (doc, a, fx, other)
    }

    const PLENTY: FrameBudget = FrameBudget {
        visible: Duration::from_secs(30),
        background: Duration::from_secs(30),
    };

    #[test]
    fn dragging_an_effect_shows_coarse_tiles_without_evaluating_it_and_releasing_makes_them_exact() {
        let ctx = egui::Context::default();
        let (mut doc, a, fx, _) = with_blur(256, 192, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let vp = viewport(0, 0, 256, 192);
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY))).pending > 0 {}
        mirror.apply(&ctx);
        assert_shows_the_document(&canvas, &mirror, &doc);
        let evaluated = doc.effect_counters().blocks_evaluated;
        // 半径をドラッグで変える
        for radius in [9u32, 12, 15] {
            doc.set_filter_settings(a, fx, yolu_core::effects::EffectSettings::blur(radius), true)
                .unwrap();
            let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY)));
            mirror.apply(&ctx);
            assert!(r.coarse > 0 && r.tiles == 0, "ドラッグの間は粗い絵だけ: {r:?}");
            let coords = every_tile(&canvas);
            let coarse: Vec<TileCoord> = coords
                .iter()
                .copied()
                .filter(|c| canvas.tile_state(*c) == Some(TileState::CoarseCurrent))
                .collect();
            assert_eq!(coarse.len(), r.coarse);
            assert_eq!(
                doc.effect_counters().blocks_evaluated, evaluated,
                "ドラッグの間は効果を正確に評価しない（キャッシュにも入れない）"
            );
            // 粗い絵のテクスチャの内容と、頁の透明
            let (id, stride) = canvas.coarse_texture().unwrap();
            let want = coarse_expected(&doc, (256, 192), stride, &coarse);
            let got = &mirror.images[&id];
            for t in &coarse {
                for dy in 0..(64 / stride) as usize {
                    for dx in 0..(64 / stride) as usize {
                        let (x, y) = (t.x as usize * 64 / stride as usize + dx, t.y as usize * 64 / stride as usize + dy);
                        let i = y * got.size[0] + x;
                        assert!(got.pixels[i] == want.pixels[i], "粗い絵 {t:?} ({dx},{dy})");
                    }
                }
                let page = &mirror.images[&canvas.page_texture(0).unwrap()];
                assert_eq!(page.pixels[(t.y * 64 * 256 + t.x * 64) as usize], Color32::TRANSPARENT);
            }
            // 離すまで、同じ粗い絵に何もしない（再描画を回し続けない）
            assert!(canvas.state.iter().all(|s| *s == TileState::CoarseCurrent || *s == TileState::Fresh));
        }
        // 離す
        doc.end_coalescing();
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY))).pending > 0 {}
        mirror.apply(&ctx);
        assert!(canvas.coarse_texture().is_none());
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn a_drag_that_needs_no_new_effect_output_stays_exact() {
        let ctx = egui::Context::default();
        let (mut doc, _, _, other) = with_blur(256, 192, 64);
        let mut canvas = CpuCanvas::default();
        let mut mirror = Mirror::default();
        let vp = viewport(0, 0, 256, 192);
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY))).pending > 0 {}
        // 効果の無い層の不透明度のドラッグ: 効果の出力は評価済みなので、粗くせず正確に
        doc.set_layer_opacity(other, 0.3, true).unwrap();
        assert!(doc.is_coalescing());
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY)));
        mirror.apply(&ctx);
        assert_eq!((r.coarse, r.pending), (0, 0));
        assert!(r.tiles > 0);
        assert_shows_the_document(&canvas, &mirror, &doc);
    }

    #[test]
    fn tiles_outside_the_view_wait_until_the_drag_is_released() {
        let ctx = egui::Context::default();
        let (mut doc, a, fx, _) = with_blur(256, 192, 64);
        let mut canvas = CpuCanvas::default();
        let all = viewport(0, 0, 256, 192);
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&all, &PLENTY))).pending > 0 {}
        // 見えるのは左上の 1 タイルだけ
        let vp = viewport(0, 0, 64, 64);
        doc.set_filter_settings(a, fx, yolu_core::effects::EffectSettings::blur(11), true).unwrap();
        let r = canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY)));
        assert_eq!(r.coarse, 1, "見えているタイルだけ粗く");
        assert_eq!(canvas.tile_state(TileCoord::new(3, 2)), Some(TileState::Stale));
        doc.end_coalescing();
        while canvas.sync(&ctx, &doc, Channel::Color, false, Some((&vp, &PLENTY))).pending > 0 {}
        assert_eq!(canvas.pending(), 0);
    }
}
