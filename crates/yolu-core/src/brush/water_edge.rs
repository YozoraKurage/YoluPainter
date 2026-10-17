//! 水彩境界（拡張: C# に無い。取り込んだブラシの水彩境界を表す形）: ストロークの覆いの縁から内側の帯を、
//! 濃く（透明度影響）・暗く（明度影響）する。
//!
//! - 縁の強さ e: 覆いのある画素の中心から、覆いが 0 の画素（ストロークの外）の中心までの最短の距離 d（ユークリッド。キャンバスの外は
//!   外と数えない）から e = clamp((幅 + 1 − d) / 幅, 0, 1)。覆いのある一番外の画素（d = 1）で 1、幅の分だけ内へ入ると 0。
//! - 帯の形 s = e^(3/4)（平方根だけで求める。OS の libm によらない）。不透明度は A' = min(1, A + 3 × 透明度影響 × s)、色の RGB は
//!   × (1 − 0.75 × 明度影響 × s)（アルファは変えない。straight の RGB を明度の向きに暗くする）。係数と形は、比べの線（ペンの縁の外の
//!   画素のアルファの上がり方と、帯の奥行きごとの暗さ）に当てはめた値。
//! - 画素は、ストロークの前の色・覆い・画素ごとの色（ダブごとの色・色の混ぜ・ステンシルの色）から、ダブの式（`apply_at`）と同じ
//!   合成で計算し直す。e = 0 の画素は、ダブの式が書いた値と同じバイト。選択範囲の量・透明部分のロックもダブの式と同じに掛ける。
//! - ドラッグ後に処理が切: 描いている間に縁が見える。ストロークの入力 1 つ（点・直線の終わり・面のダブ）の終わりごとに、そこまでに
//!   画素を変えたかもしれない箱（タイルごと）を幅だけ広げた範囲を計算し直す。e は覆いだけで決まるので、計算し直す区切り方によらず、
//!   最後の画素は同じ。下地を今の面から読む色の混ぜ（重ねる・伸ばす）は、縁の印の立つ画素を縁の無い値で読む
//!   （[`StrokeState::ground_without_edge`]。縁が次の打点の荷に入ると、区切りごとに暗さが積み重なるため）。
//! - ドラッグ後に処理が入: 描いている間は縁なし。確定のときに 1 回だけ、ストロークが触ったタイル全部を計算し直し、ぼかし幅があれば
//!   e を箱のぼかし（半径 = ぼかし幅、端の数は割合の重み）で縦横に均す（外の画素は e = 1 として。縁の一番外を薄めすぎず、内への落ち方が
//!   なだらかになる）。Undo はストロークと一緒の 1 段。
//! - 3D の面のストロークは、幅とぼかし幅を画面の点として、中心のテクセルの画面の大きさ（`TexelMetric`）が分かった最初の面のダブの
//!   値でテクセルへ直す（ストロークの間は固定。それより前のダブの縁は、決まったときに塗ったタイル全部を計算し直して付け直す）。縁はテクスチャの上で求めるので、UV アイランドの切れ目にも縁ができる。
//! - 効くのは、色のチャンネルを色で塗るブラシだけ（消しゴム・効果のブラシ・データのチャンネル・マスクでは効かない）。
//!
//! 計算は、タイルごとに、範囲を届く距離だけ広げた箱の覆いを読み、行の向きの距離から、上下の届く距離の内の行（遠くまで届くときは
//! 列ごとの放物線の下側の包絡）で 2 乗の距離を出す（どれも整数の値なので、道とスレッドによらず同じ）。範囲が大きいときは、タイルごとの値をワーカーで求め、書くのは呼んだ
//! スレッドがタイルの順に行う（予算の確かめの順も同じ）。

use std::collections::HashMap;

use rayon::prelude::*;

use super::rows;
use super::{worth_parallel, BrushStencil, Selected, StencilMode, StrokeState, StrokeTile};
use crate::error::CoreError;
use crate::math::require_finite;
use crate::selection::SelectionMask;
use crate::surface::{LiveTile, Surface};
use crate::types::{ChannelKind, Rgba8, TileCoord};
use crate::Reason;

/// 幅の下限・上限（画素。3D は画面の点）。
pub const MIN_WATER_EDGE_WIDTH: f64 = 0.1;
pub const MAX_WATER_EDGE_WIDTH: f64 = 20.0;
/// ぼかし幅の上限（画素。3D は画面の点）。
pub const MAX_WATER_EDGE_BLUR: f64 = 20.0;
/// 3D の面でテクセルへ直した幅・ぼかし幅の上限（テクセル。読む箱の大きさを抑える）。
const MAX_TEXEL_LENGTH: f64 = 64.0;
/// 透明度影響 1 のときの、帯の一番外での不透明度の足し。
const OPACITY_GAIN: f32 = 3.0;
/// 明度影響 1 のときの、帯の一番外での暗さ（RGB に 1 − これを掛ける）。
const DARKNESS_GAIN: f32 = 0.75;
/// 読む箱の 1 セルの時間の見積もり（ナノ秒。覆いの読み・距離の 2 回の走査・帯の画素の式と書き込みの平均）。半径 30 と 100 の硬い丸の
/// 直線・幅 3 と 20・ぼかしなしで、1 スレッドの縁の時間を読む箱のセルの数で割ると 8〜11（ぼかし幅 5 では 21〜24）。ワーカーで求めるかの換算に使う。
pub(super) const EDGE_CELL_NANOS: i64 = 9;

/// 水彩境界の設定（[`super::Brush::water_edge`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterEdge {
    /// 帯の幅（画素。3D の面のストロークは画面の点）。0.1〜20。
    pub width: f64,
    /// 透明度影響（帯の不透明度の上げ）0〜1。
    pub opacity: f64,
    /// 明度影響（帯の暗さ）0〜1。
    pub darkness: f64,
    /// ドラッグ後に処理: 描いている間は縁を付けず、ストロークを確定するときに 1 回だけ付ける。
    pub after_stroke: bool,
    /// ぼかし幅（画素。3D は画面の点）0〜20。ドラッグ後に処理のときだけ効く。
    pub blur: f64,
}

/// 既定（水彩境界を入れたブラシの初めの値。取り込んだファイルに列が無いときも）: 幅 4 は、届く距離 5 で上下の行を読む速い形
/// （届く距離 8 まで）に収まり、半径 10〜30 の線で帯が見える幅。透明度影響と明度影響は両方 0.3 にして、入れたときに
/// どちらの効きも見えるようにする（係数を掛けて、帯の一番外で不透明度 +0.9・RGB × 0.775。不透明度の低い線でも縁が見え、
/// 色が黒へ寄りすぎない）。
impl Default for WaterEdge {
    fn default() -> Self {
        WaterEdge {
            width: 4.0,
            opacity: 0.3,
            darkness: 0.3,
            after_stroke: false,
            blur: 0.0,
        }
    }
}

impl WaterEdge {
    /// 範囲を確かめる（有限・幅 0.1〜20・透明度影響と明度影響 0〜1・ぼかし幅 0〜20）。
    pub fn validate(&self) -> Result<(), CoreError> {
        for v in [self.width, self.opacity, self.darkness, self.blur] {
            require_finite(v, Reason::WaterEdge)?;
        }
        if !(MIN_WATER_EDGE_WIDTH..=MAX_WATER_EDGE_WIDTH).contains(&self.width)
            || !(0.0..=1.0).contains(&self.opacity)
            || !(0.0..=1.0).contains(&self.darkness)
            || !(0.0..=MAX_WATER_EDGE_BLUR).contains(&self.blur)
        {
            return Err(CoreError::InvalidArgument(Reason::WaterEdgeRange));
        }
        Ok(())
    }
}

/// 進行中のストロークの水彩境界（効くストロークだけが持つ）。
pub(crate) struct EdgeRun {
    edge: WaterEdge,
    /// 幅・ぼかし幅を描く先の画素へ直す係数（2D は無し = 1。3D は最初の面のダブで決める）。
    scale: Option<f64>,
    /// 描いている間の縁: 前に計算し直してから、画素を変えたかもしれない箱（タイルごと。x0, x1, y0, y1。両端を含む）。
    dirty: HashMap<TileCoord, [i64; 4]>,
    /// 縁を付けた値を持つかもしれない画素（タイルごとのビットの並び）。立っていない画素で e = 0 なら、ダブの式が書いた値のままなので
    /// 計算し直さない（ダブが上書きしても立てたままにする。立っていれば計算し直すだけなので、多めに立っていてもよい）。
    edged: HashMap<TileCoord, Vec<u64>>,
    /// 直前に印を付けたタイル（画素ごとの印で、同じタイルの表を引き直さない）。
    last: Option<(TileCoord, [i64; 4])>,
}

impl EdgeRun {
    pub(super) fn new(edge: WaterEdge) -> EdgeRun {
        EdgeRun {
            edge,
            scale: None,
            dirty: HashMap::new(),
            edged: HashMap::new(),
            last: None,
        }
    }

    /// 描く先の画素での幅とぼかし幅（ぼかしはドラッグ後に処理のときだけ）。
    fn lengths(&self) -> (f64, f64) {
        let k = self.scale.unwrap_or(1.0);
        let width = (self.edge.width * k).clamp(MIN_WATER_EDGE_WIDTH, MAX_TEXEL_LENGTH);
        let blur = if self.edge.after_stroke {
            (self.edge.blur * k).clamp(0.0, MAX_TEXEL_LENGTH)
        } else {
            0.0
        };
        (width, blur)
    }

    fn mark(&mut self, coord: TileCoord, b: [i64; 4]) {
        if let Some((c, r)) = &mut self.last {
            if *c == coord {
                union(r, b);
                return;
            }
        }
        self.settle_last();
        self.last = Some((coord, b));
    }

    fn settle_last(&mut self) {
        if let Some((c, r)) = self.last.take() {
            self.dirty.entry(c).and_modify(|d| union(d, r)).or_insert(r);
        }
    }
}

fn union(into: &mut [i64; 4], b: [i64; 4]) {
    into[0] = into[0].min(b[0]);
    into[1] = into[1].max(b[1]);
    into[2] = into[2].min(b[2]);
    into[3] = into[3].max(b[3]);
}

/// 縁が届く距離（幅 + 1 の切り上げ。これより遠い外の画素は e を変えない）。
fn reach(width: f64) -> i64 {
    (width + 1.0).ceil() as i64
}

impl StrokeState {
    /// 描いている間の縁の印: 箱（キャンバスの中、両端を含む）の画素を、このストロークが変えたかもしれない。
    pub(super) fn edge_mark_box(&mut self, xs: (i64, i64), ys: (i64, i64)) {
        let ts = self.tile_size;
        let Some(run) = self.water_edge.as_mut().filter(|r| !r.edge.after_stroke) else {
            return;
        };
        if xs.0 > xs.1 || ys.0 > ys.1 {
            return;
        }
        for ty in ys.0 / ts..=ys.1 / ts {
            for tx in xs.0 / ts..=xs.1 / ts {
                let b = [
                    xs.0.max(tx * ts),
                    xs.1.min(tx * ts + ts - 1),
                    ys.0.max(ty * ts),
                    ys.1.min(ty * ts + ts - 1),
                ];
                run.mark(TileCoord::new(tx as u32, ty as u32), b);
            }
        }
    }

    /// 描いている間の縁の印: 画素 1 つ（キャンバスの外は無視）。
    pub(super) fn edge_mark_pixel(&mut self, x: i64, y: i64) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let ts = self.tile_size;
        if let Some(run) = self.water_edge.as_mut().filter(|r| !r.edge.after_stroke) {
            run.mark(
                TileCoord::new((x / ts) as u32, (y / ts) as u32),
                [x, x, y, y],
            );
        }
    }

    /// 描いている間に縁を付けるストロークか（ドラッグ後に処理が切）。
    pub(super) fn edge_live(&self) -> bool {
        self.water_edge
            .as_ref()
            .is_some_and(|r| !r.edge.after_stroke)
    }

    /// 3D の面のストローク: 幅・ぼかし幅（画面の点）を描く先のテクセルへ直す係数（画面の点 1 つのテクセルの数）。最初の 1 回だけ
    /// 覚える（ストロークの間は固定。計算し直す区切り方によらず同じ縁にする）。係数が決まる前のダブ（中心のテクセルの画面の大きさが
    /// 分からなかったもの）が描いている間の縁を係数 1 で付けていたら、塗ったタイル全部を次の計算し直しに積む（古い幅の縁を残さない）。
    pub(crate) fn set_water_edge_scale(&mut self, texels_per_point: f64) {
        let ts = self.tile_size;
        let (w, h) = (self.width, self.height);
        let Some(run) = self.water_edge.as_mut() else {
            return;
        };
        if run.scale.is_some() || !texels_per_point.is_finite() || texels_per_point <= 0.0 {
            return;
        }
        run.scale = Some(texels_per_point);
        if run.edge.after_stroke {
            return;
        }
        run.settle_last();
        for c in self.tiles.keys() {
            let (x0, y0) = (c.x as i64 * ts, c.y as i64 * ts);
            let b = [x0, (x0 + ts - 1).min(w - 1), y0, (y0 + ts - 1).min(h - 1)];
            run.dirty.entry(*c).and_modify(|d| union(d, b)).or_insert(b);
        }
    }

    /// 描いている間の縁: 印の付いた範囲を幅だけ広げて計算し直す（ドラッグ後に処理のストロークでは何もしない）。
    pub(crate) fn flush_water_edge(
        &mut self,
        surface: &mut Surface,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let Some(run) = self.water_edge.as_mut() else {
            return Ok(false);
        };
        run.settle_last();
        if run.edge.after_stroke || run.dirty.is_empty() {
            return Ok(false);
        }
        let dirty = std::mem::take(&mut run.dirty);
        let r = reach(run.lengths().0);
        let ts = self.tile_size;
        let mut targets: HashMap<TileCoord, [i64; 4]> = HashMap::new();
        for b in dirty.into_values() {
            let x0 = (b[0] - r).max(0);
            let x1 = (b[1] + r).min(self.width - 1);
            let y0 = (b[2] - r).max(0);
            let y1 = (b[3] + r).min(self.height - 1);
            for ty in y0 / ts..=y1 / ts {
                for tx in x0 / ts..=x1 / ts {
                    let part = [
                        x0.max(tx * ts),
                        x1.min(tx * ts + ts - 1),
                        y0.max(ty * ts),
                        y1.min(ty * ts + ts - 1),
                    ];
                    targets
                        .entry(TileCoord::new(tx as u32, ty as u32))
                        .and_modify(|d| union(d, part))
                        .or_insert(part);
                }
            }
        }
        self.edge_pass(surface, targets, changed)
    }

    /// ストロークの確定: ドラッグ後に処理なら、触ったタイル全部の縁を 1 回だけ付ける（ぼかし幅つき）。描いている間に付けるなら、
    /// 残った印の範囲を計算し直す。
    pub(crate) fn finish_water_edge(
        &mut self,
        surface: &mut Surface,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        match self.water_edge.as_ref() {
            None => Ok(false),
            Some(run) if !run.edge.after_stroke => self.flush_water_edge(surface, changed),
            Some(_) => {
                let ts = self.tile_size;
                let targets = self
                    .tiles
                    .keys()
                    .map(|&c| {
                        let (x0, y0) = (c.x as i64 * ts, c.y as i64 * ts);
                        (
                            c,
                            [
                                x0,
                                (x0 + ts - 1).min(self.width - 1),
                                y0,
                                (y0 + ts - 1).min(self.height - 1),
                            ],
                        )
                    })
                    .collect();
                self.edge_pass(surface, targets, changed)
            }
        }
    }

    /// 範囲（タイルごと）の画素を、縁を付けて計算し直す。
    fn edge_pass(
        &mut self,
        surface: &mut Surface,
        targets: HashMap<TileCoord, [i64; 4]>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let _profile = super::profile::scope(super::profile::Stage::WaterEdge);
        // 覆いのある画素はストロークのタイルにしかない
        let mut list: Vec<(TileCoord, [i64; 4])> = targets
            .into_iter()
            .filter(|(c, _)| self.tiles.contains_key(c))
            .collect();
        if list.is_empty() {
            return Ok(false);
        }
        list.sort_by_key(|(c, _)| (c.y, c.x));
        let budgets = self.budgets;
        let (scratch, cells) = {
            let ctx = self.edge_ctx();
            let scratch: Vec<u64> = list.iter().map(|(_, r)| ctx.scratch_bytes(r)).collect();
            let cells: i64 = list.iter().map(|(_, r)| ctx.window_cells(r)).sum();
            (scratch, cells)
        };
        let total: u64 = scratch.iter().sum();
        let held = self.rollback_bytes.saturating_add(self.held_bytes());
        let parallel = list.len() > 1
            && rayon::current_num_threads() > 1
            && worth_parallel(cells, EDGE_CELL_NANOS)
            && held.saturating_add(total) <= budgets.stroke;
        let mut any = false;
        if parallel {
            let values: Vec<Vec<(u32, Rgba8, bool)>> = {
                let ctx = self.edge_ctx();
                list.par_iter()
                    .map(|(coord, rect)| ctx.values(*coord, rect))
                    .collect()
            };
            for ((coord, _), v) in list.iter().zip(&values) {
                any |= self.write_edge(surface, *coord, v, changed)?;
            }
        } else {
            for ((coord, rect), bytes) in list.iter().zip(&scratch) {
                let held = self.rollback_bytes.saturating_add(self.held_bytes());
                if held.saturating_add(*bytes) > budgets.stroke {
                    return Err(CoreError::StrokeBudgetExceeded);
                }
                let v = self.edge_ctx().values(*coord, rect);
                any |= self.write_edge(surface, *coord, &v, changed)?;
            }
        }
        Ok(any)
    }

    /// 下地を今の面から読む色の混ぜ（伸ばす・重ねる）の読み: タイル `coord` の番号 `local0` から並ぶ画素 `out`（面から読んだ値）の
    /// うち、縁を付けた印の立つ画素を、縁の無い値（s = 0 で計算し直した値。水彩境界の無いブラシがそこに書く値と同じバイト）へ差し替える。
    /// 描いている間の縁が次の打点の下地に入ると、入力の区切りごとに縁の暗さが荷へ積み重なり、線が区切り方で変わるため。
    pub(super) fn ground_without_edge(&self, coord: TileCoord, local0: usize, out: &mut [Rgba8]) {
        let Some(bits) = self.water_edge.as_ref().and_then(|r| r.edged.get(&coord)) else {
            return;
        };
        let Some(st) = self.tiles.get(&coord) else {
            return;
        };
        let mut ctx: Option<(EdgeCtx<'_>, Selected<'_>)> = None;
        for (i, o) in out.iter_mut().enumerate() {
            let local = local0 + i;
            if bits[local / 64] >> (local % 64) & 1 == 0 {
                continue;
            }
            let wash = st.wash[local];
            if wash <= 0.0 {
                continue;
            }
            let (ctx, selected) = ctx.get_or_insert_with(|| {
                let c = self.edge_ctx();
                let s = Selected::of(c.selection, coord);
                (c, s)
            });
            if let Some(c) = ctx.recompute(st, selected, local, wash, 0.0) {
                *o = c;
            }
        }
    }

    /// 縁を求めるのに読む物（ストロークとその水彩境界から）。
    fn edge_ctx(&self) -> EdgeCtx<'_> {
        let run = self.water_edge.as_ref().expect("水彩境界のストローク");
        let (width, blur) = run.lengths();
        EdgeCtx {
            tiles: &self.tiles,
            tile_size: self.tile_size,
            canvas: (self.width, self.height),
            width: width as f32,
            reach: reach(width),
            blur,
            opacity: run.edge.opacity as f32 * OPACITY_GAIN,
            darkness: run.edge.darkness as f32 * DARKNESS_GAIN,
            keep_alpha: self.keep_alpha,
            stroke_color: self.stroke_color,
            stencil: self
                .stencil_kind
                .zip(self.brush.stencil.as_deref())
                .filter(|(_, s)| s.mode() == StencilMode::Color),
            selection: self.selection.as_ref(),
            edged: &run.edged,
        }
    }

    /// 求めた画素をタイルへ書き（同じ値は書かない）、縁を付けた画素の印を直す。印のビットの並びを初めて持つタイルは、その大きさを
    /// ストロークの予算に数える。画素が変わったら true。
    fn write_edge(
        &mut self,
        surface: &mut Surface,
        coord: TileCoord,
        values: &[(u32, Rgba8, bool)],
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        if values.is_empty() {
            return Ok(false);
        }
        let ts = surface.tile_size() as usize;
        let words = (ts * ts).div_ceil(64);
        let needs_bits = values.iter().any(|v| v.2)
            && !self
                .water_edge
                .as_ref()
                .is_some_and(|r| r.edged.contains_key(&coord));
        if needs_bits {
            let next = self.rollback_bytes + words as u64 * 8;
            self.ensure_budget(next)?;
            self.rollback_bytes = next;
        }
        let growth = self.budgets.growth;
        let mut live = LiveTile::take(surface, coord)?;
        let mut any = false;
        let mut result = Ok(());
        for &(local, c, _) in values {
            match live.write(
                local as usize * 4,
                c,
                &mut surface.allocated,
                ts * ts * 4,
                growth,
            ) {
                Ok(w) => any |= w,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        live.put_back(surface, coord);
        result?;
        let run = self.water_edge.as_mut().expect("水彩境界のストローク");
        if needs_bits || run.edged.contains_key(&coord) {
            let bits = run.edged.entry(coord).or_insert_with(|| vec![0; words]);
            for &(local, _, edged) in values {
                let (w, b) = (local as usize / 64, local % 64);
                if edged {
                    bits[w] |= 1 << b;
                } else {
                    bits[w] &= !(1 << b);
                }
            }
        }
        if any {
            changed.push(coord);
        }
        Ok(any)
    }
}

/// 縁を求めるのに読む物（ワーカーからも読む）。
struct EdgeCtx<'a> {
    tiles: &'a HashMap<TileCoord, StrokeTile>,
    tile_size: i64,
    canvas: (i64, i64),
    /// 描く先の画素での幅と、縁が届く距離（切り上げ）・ぼかし幅。
    width: f32,
    reach: i64,
    blur: f64,
    /// 帯の一番外での不透明度の足し・暗さ（設定 × 係数）。
    opacity: f32,
    darkness: f32,
    keep_alpha: bool,
    stroke_color: Rgba8,
    /// ステンシルの色を受けるチャンネルと、色のモードのステンシル。
    stencil: Option<(ChannelKind, &'a BrushStencil)>,
    selection: Option<&'a SelectionMask>,
    edged: &'a HashMap<TileCoord, Vec<u64>>,
}

/// 範囲と、e を求める箱・覆いを読む箱（どれも x0, x1, y0, y1。両端を含む、キャンバスの中）。
struct Frames {
    e: [i64; 4],
    window: [i64; 4],
}

impl EdgeCtx<'_> {
    fn frames(&self, rect: &[i64; 4]) -> Frames {
        let (w, h) = self.canvas;
        let grow = |r: &[i64; 4], by: i64| {
            [
                (r[0] - by).max(0),
                (r[1] + by).min(w - 1),
                (r[2] - by).max(0),
                (r[3] + by).min(h - 1),
            ]
        };
        let e = grow(rect, self.blur.ceil() as i64);
        Frames {
            window: grow(&e, self.reach),
            e,
        }
    }

    fn window_cells(&self, rect: &[i64; 4]) -> i64 {
        let w = self.frames(rect).window;
        (w[1] - w[0] + 1) * (w[3] - w[2] + 1)
    }

    /// 1 つのタイルの計算の一時の領域（覆い f32・行の距離 f64・列の包絡・e f32）。
    fn scratch_bytes(&self, rect: &[i64; 4]) -> u64 {
        let f = self.frames(rect);
        let cells = self.window_cells(rect) as u64;
        let e = ((f.e[1] - f.e[0] + 1) * (f.e[3] - f.e[2] + 1)) as u64;
        let ts = self.tile_size as u64;
        cells * 12 + e * 4 + ts * ts * 8 + 256
    }

    /// 読む箱の覆い（ストロークの覆い。タイルの無い所は 0）を行ごとに読む。
    fn read_window(&self, win: &[i64; 4], out: &mut [f32]) {
        let ts = self.tile_size;
        let ww = (win[1] - win[0] + 1) as usize;
        for (row, py) in (win[2]..=win[3]).enumerate() {
            let ty = py / ts;
            let line = &mut out[row * ww..(row + 1) * ww];
            let mut px = win[0];
            while px <= win[1] {
                let tx = px / ts;
                let end = win[1].min(tx * ts + ts - 1);
                let len = (end - px + 1) as usize;
                let at = (px - win[0]) as usize;
                if let Some(t) = self.tiles.get(&TileCoord::new(tx as u32, ty as u32)) {
                    let start = ((py - ty * ts) * ts + (px - tx * ts)) as usize;
                    line[at..at + len].copy_from_slice(&t.wash[start..start + len]);
                }
                px = end + 1;
            }
        }
    }

    /// 1 つのタイルの範囲の、覆いのある画素の新しい値（タイルの中の番号と色）。
    fn values(&self, coord: TileCoord, rect: &[i64; 4]) -> Vec<(u32, Rgba8, bool)> {
        let f = self.frames(rect);
        let win = f.window;
        let ww = (win[1] - win[0] + 1) as usize;
        let wh = (win[3] - win[2] + 1) as usize;
        let mut cover = vec![0.0f32; ww * wh];
        self.read_window(&win, &mut cover);
        let dist = squared_distances(&cover, ww, wh, self.reach);
        // e を求める箱（ぼかしの無いときは範囲そのもの）
        let (ex0, ey0) = (f.e[0], f.e[2]);
        let ew = (f.e[1] - ex0 + 1) as usize;
        let eh = (f.e[3] - ey0 + 1) as usize;
        let mut e = vec![0.0f32; ew * eh];
        let w = self.width;
        for row in 0..eh {
            let wy = (ey0 + row as i64 - win[2]) as usize;
            for col in 0..ew {
                let wx = (ex0 + col as i64 - win[0]) as usize;
                let k = wy * ww + wx;
                e[row * ew + col] = if cover[k] <= 0.0 {
                    1.0 // 外（ぼかしで縁の一番外を薄めない）
                } else {
                    ((w + 1.0 - (dist[k] as f32).sqrt()) / w).clamp(0.0, 1.0)
                };
            }
        }
        if self.blur > 0.0 {
            blur_box(&mut e, ew, eh, self.blur);
        }
        let ts = self.tile_size;
        let (ox, oy) = (coord.x as i64 * ts, coord.y as i64 * ts);
        let st = &self.tiles[&coord];
        let selected = Selected::of(self.selection, coord);
        let bits = self.edged.get(&coord);
        let mut out = Vec::new();
        for py in rect[2]..=rect[3] {
            for px in rect[0]..=rect[1] {
                let k = (py - win[2]) as usize * ww + (px - win[0]) as usize;
                let wash = cover[k];
                if wash <= 0.0 {
                    continue;
                }
                let local = ((py - oy) * ts + (px - ox)) as usize;
                let s = shape(e[(py - ey0) as usize * ew + (px - ex0) as usize]);
                let was = bits.is_some_and(|b| b[local / 64] >> (local % 64) & 1 == 1);
                if s <= 0.0 && !was {
                    continue; // ダブの式が書いた値のまま
                }
                if let Some(c) = self.recompute(st, &selected, local, wash, s) {
                    out.push((local as u32, c, s > 0.0));
                }
            }
        }
        out
    }

    /// 覆いのある画素 1 つを、帯の形 s で計算し直した値。選択の外・透明部分のロックで守る透明な画素は、ダブの式も書かないので None。
    fn recompute(
        &self,
        st: &StrokeTile,
        selected: &Selected<'_>,
        local: usize,
        wash: f32,
        s: f32,
    ) -> Option<Rgba8> {
        let sel = selected.amount(local);
        if sel <= 0.0 {
            return None;
        }
        let start = st
            .before_px
            .as_ref()
            .map_or(Rgba8::TRANSPARENT, |t| t.get(local * 4));
        if self.keep_alpha && start.a == 0 {
            return None;
        }
        Some(self.pixel(st, local, start, wash, sel, s))
    }

    /// 1 画素: ダブの式（`apply_at` の塗り）と同じ合成に、帯の不透明度と暗さを入れる（s = 0 ならダブの式の値そのもの）。
    fn pixel(
        &self,
        st: &StrokeTile,
        local: usize,
        start: Rgba8,
        wash: f32,
        sel: f32,
        s: f32,
    ) -> Rgba8 {
        let ts = self.tile_size as usize;
        let mut color = self.stroke_color;
        if let Some(pc) = st.paint.as_ref() {
            let plane = ts * ts;
            let v: [u8; 4] = std::array::from_fn(|c| rows::byte32(pc[c * plane + local]));
            color = Rgba8::new(v[0], v[1], v[2], v[3]);
        }
        if let Some((kind, stencil)) = self.stencil {
            let known = st
                .stencil_amount
                .as_ref()
                .is_some_and(|a| !a[local].is_nan());
            if let (true, Some(c)) = (known, st.stencil_color.as_ref()) {
                color = stencil.paint_for(kind, c[local], self.stroke_color.a);
            }
        }
        let mut amount = rows::min32(1.0, wash);
        if s > 0.0 {
            amount = rows::min32(1.0, amount + self.opacity * s);
            let k = 1.0 - self.darkness * s;
            if k < 1.0 {
                let dark = |c: u8| (f32::from(c) * k + 0.5) as u8;
                color = Rgba8::new(dark(color.r), dark(color.g), dark(color.b), color.a);
            }
        }
        if self.keep_alpha {
            rows::keeping32(start, color, amount * sel)
        } else {
            let next = rows::normal32(start, color, amount);
            if sel < 1.0 {
                rows::fade32(start, next, sel)
            } else {
                next
            }
        }
    }
}

/// 帯の形 s = e^(3/4)（平方根だけ）。
#[inline]
fn shape(e: f32) -> f32 {
    if e <= 0.0 {
        return 0.0;
    }
    let r = e.sqrt();
    (r * r.sqrt()).min(1.0)
}

/// 読む箱の各セルから、覆いが 0 のセル（外）までの 2 乗の距離（(reach + 1)² 以上の距離は (reach + 1)² に詰める。そこでは e = 0）。
/// 行の向きの距離を 2 回の走査で求め、上下 reach 行の内で (行の差)² + (その行の距離)² のいちばん小さい値を取る（reach 行より離れた
/// 行は (reach + 1)² 以上なので、詰めた値と同じ）。どれも整数の値。
fn squared_distances(cover: &[f32], ww: usize, wh: usize, reach: i64) -> Vec<u32> {
    let cap = (reach + 1) as u32;
    let cap2 = cap * cap;
    // 行の向き: 同じ行の外までの距離の 2 乗（無ければ cap²）
    let mut g = vec![cap2; ww * wh];
    for row in 0..wh {
        let line = &cover[row * ww..(row + 1) * ww];
        let out = &mut g[row * ww..(row + 1) * ww];
        let mut last: Option<usize> = None;
        for x in 0..ww {
            if line[x] <= 0.0 {
                last = Some(x);
            }
            if let Some(l) = last {
                out[x] = ((x - l) as u32).min(cap);
            } else {
                out[x] = cap;
            }
        }
        let mut next: Option<usize> = None;
        for x in (0..ww).rev() {
            if line[x] <= 0.0 {
                next = Some(x);
            }
            if let Some(n) = next {
                out[x] = out[x].min(((n - x) as u32).min(cap));
            }
            out[x] *= out[x];
        }
    }
    // 列の向き。遠くまで届くときは、列ごとに放物線の下側の包絡（Felzenszwalb と Huttenlocher の距離変換。セルの数に比例）で
    if reach > ROW_SCAN_REACH {
        let mut f = vec![0.0f64; wh];
        let mut out = vec![0.0f64; wh];
        let mut v = vec![0usize; wh];
        let mut z = vec![0.0f64; wh + 1];
        let mut d = vec![cap2; ww * wh];
        for col in 0..ww {
            for row in 0..wh {
                f[row] = f64::from(g[row * ww + col]);
            }
            lower_envelope(&f, &mut out, &mut v, &mut z);
            for row in 0..wh {
                // 値は整数の和（2 乗の距離 + 2 乗の距離）なので、f64 で丸めなく求まる
                d[row * ww + col] = (out[row] as u32).min(cap2);
            }
        }
        return d;
    }
    // 近いときは、上下 reach 行の内のいちばん小さい値（行ごとに続けて読むので速い）
    let mut d = vec![cap2; ww * wh];
    let r = reach.max(0) as usize;
    for row in 0..wh {
        let out = &mut d[row * ww..(row + 1) * ww];
        for other in row.saturating_sub(r)..=(row + r).min(wh - 1) {
            let dy = row.abs_diff(other) as u32;
            let dy2 = dy * dy;
            let line = &g[other * ww..(other + 1) * ww];
            for (o, &v) in out.iter_mut().zip(line) {
                let c = v + dy2;
                if c < *o {
                    *o = c;
                }
            }
        }
    }
    d
}

/// 列の向きの距離を、上下の行を読む形で求める届く距離の上限（これより遠くまで届くときは、下側の包絡）。
const ROW_SCAN_REACH: i64 = 8;

/// 1 次元の 2 乗の距離変換: d[q] = min_p ((q − p)² + f[p])。
fn lower_envelope(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let at = |q: usize| f[q] + (q * q) as f64;
    for q in 1..n {
        let mut s = (at(q) - at(v[k])) / (2.0 * (q as f64 - v[k] as f64));
        while s <= z[k] {
            k -= 1;
            s = (at(q) - at(v[k])) / (2.0 * (q as f64 - v[k] as f64));
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - v[k] as f64;
        *out = dq * dq + f[v[k]];
    }
}

/// 箱のぼかし（半径 radius。端の 1 つは小数の割合の重み）を横・縦の順に。箱の外（キャンバスの外）は数えずに割る。
fn blur_box(e: &mut [f32], w: usize, h: usize, radius: f64) {
    let whole = radius.floor() as usize;
    let part = (radius - radius.floor()) as f32;
    let mut weights = vec![1.0f32; whole + 1];
    if part > 0.0 {
        weights.push(part);
    }
    let reach = weights.len() - 1;
    let mut tmp = vec![0.0f32; w.max(h)];
    let mut pass = |e: &mut [f32], n: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        for line in 0..lines {
            for i in 0..n {
                let (mut sum, mut total) = (0.0f32, 0.0f32);
                let lo = i.saturating_sub(reach);
                let hi = (i + reach).min(n - 1);
                for j in lo..=hi {
                    let wgt = weights[i.abs_diff(j)];
                    sum += e[at(line, j)] * wgt;
                    total += wgt;
                }
                tmp[i] = sum / total;
            }
            for i in 0..n {
                e[at(line, i)] = tmp[i];
            }
        }
    };
    pass(e, w, h, &|line, i| line * w + i);
    pass(e, h, w, &|line, i| i * w + line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::simd::{forced, Level};
    use crate::{Brush, BrushSettings, Document, PressureSwitches};
    use glam::DVec2;

    /// 道を固定して、水彩境界のブラシの線を描く（行の核の道・画素ごとの式の両方を通るブラシ）。
    fn stroke_on(level: Level, b: &Brush) -> Vec<u8> {
        forced::with_level(level, || {
            let mut d = Document::with_tile_size(120, 80, 32).unwrap();
            let l = d.add_layer("L").unwrap();
            let mut s = d.begin_brush_stroke(l, b).unwrap();
            for (x, y) in [(12.0, 20.0), (60.0, 55.5), (108.25, 30.0)] {
                s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
            }
            d.end_stroke(s).unwrap();
            d.composite(d.bounds()).unwrap()
        })
    }

    #[test]
    fn every_level_paints_the_same_edge() {
        let levels = forced::supported();
        for (hardness, tip_colors, after_stroke) in
            [(1.0, false, false), (0.3, false, true), (0.6, true, false)]
        {
            let mut b = Brush::simple(
                BrushSettings {
                    radius: 10.0,
                    hardness,
                    spacing: 0.1,
                    opacity: 0.8,
                    color: crate::Rgba8::new(40, 100, 200, 255),
                    ..BrushSettings::default()
                },
                PressureSwitches::NONE,
            );
            if tip_colors {
                b.color.hue = 0.2;
                b.color.per_tip = true;
            }
            b.water_edge = Some(WaterEdge {
                width: 5.0,
                opacity: 0.3,
                darkness: 0.7,
                after_stroke,
                blur: if after_stroke { 1.5 } else { 0.0 },
            });
            let reference = stroke_on(Level::Scalar, &b);
            assert!(reference.chunks_exact(4).any(|p| p[3] > 0));
            for &level in &levels {
                assert_eq!(stroke_on(level, &b), reference, "{level:?}");
            }
        }
    }

    fn brute(cover: &[f32], ww: usize, wh: usize, cap2: u32) -> Vec<u32> {
        let mut out = vec![cap2; ww * wh];
        for y in 0..wh {
            for x in 0..ww {
                for qy in 0..wh {
                    for qx in 0..ww {
                        if cover[qy * ww + qx] <= 0.0 {
                            let dx = x.abs_diff(qx) as u32;
                            let dy = y.abs_diff(qy) as u32;
                            let d = dx * dx + dy * dy;
                            if d < out[y * ww + x] {
                                out[y * ww + x] = d;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_scale_found_after_the_first_dabs_redoes_their_edge() {
        // 3D で中心のテクセルの画面の大きさが分からないダブが先に来ると、縁は係数 1 で付く。後で係数が決まったら、塗った所全部を
        // その係数で計算し直す（係数 2 は、幅 2 倍のブラシを初めから描いたのと同じバイト）
        let brush = |width: f64| {
            let mut b = Brush::simple(
                BrushSettings {
                    radius: 8.0,
                    hardness: 0.8,
                    spacing: 0.1,
                    color: crate::Rgba8::new(40, 100, 200, 255),
                    ..BrushSettings::default()
                },
                PressureSwitches::NONE,
            );
            b.water_edge = Some(WaterEdge {
                width,
                opacity: 0.3,
                darkness: 0.7,
                after_stroke: false,
                blur: 0.0,
            });
            b
        };
        let draw = |b: &Brush, scale_after_first: Option<f64>| {
            let mut d = Document::with_tile_size(120, 80, 32).unwrap();
            let l = d.add_layer("L").unwrap();
            let mut s = d.begin_brush_stroke(l, b).unwrap();
            for (i, (x, y)) in [(12.0, 20.0), (40.0, 30.0), (60.0, 55.5), (108.25, 30.0)]
                .into_iter()
                .enumerate()
            {
                s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
                if i == 1 {
                    if let Some(k) = scale_after_first {
                        s.set_water_edge_scale(&mut d, k).unwrap();
                    }
                }
            }
            d.end_stroke(s).unwrap();
            d.composite(d.bounds()).unwrap()
        };
        let wide = draw(&brush(6.0), None);
        assert_ne!(draw(&brush(3.0), None), wide);
        assert_eq!(draw(&brush(3.0), Some(2.0)), wide);
    }

    #[test]
    fn distances_match_a_brute_force_search() {
        let (ww, wh) = (23usize, 17usize);
        let mut cover = vec![1.0f32; ww * wh];
        // 外の画素を散らす（決まった並び）
        for (i, c) in cover.iter_mut().enumerate() {
            if (i * 37 + i / 5) % 29 == 0 {
                *c = 0.0;
            }
        }
        // 上下の行を読む形（8 まで）と、下側の包絡（9 から）の両方
        for reach in [1, 3, 6, 8, 9, 13, 40] {
            let cap2 = ((reach + 1) * (reach + 1)) as u32;
            assert_eq!(
                squared_distances(&cover, ww, wh, reach),
                brute(&cover, ww, wh, cap2),
                "{reach}"
            );
        }
        // 外の無い箱は、どこも届く距離の外
        let full = vec![0.5f32; 9];
        assert!(squared_distances(&full, 3, 3, 2).iter().all(|&d| d == 9));
    }

    #[test]
    fn the_band_shape_is_monotone_and_ends_at_one() {
        assert_eq!(shape(0.0), 0.0);
        assert_eq!(shape(1.0), 1.0);
        let mut last = 0.0;
        for i in 1..=100 {
            let s = shape(i as f32 / 100.0);
            assert!(s > last);
            last = s;
        }
    }

    #[test]
    fn the_box_blur_keeps_a_flat_field_and_spreads_a_step() {
        let (w, h) = (9usize, 5usize);
        let mut flat = vec![0.7f32; w * h];
        blur_box(&mut flat, w, h, 2.5);
        assert!(flat.iter().all(|&v| (v - 0.7).abs() < 1e-6));
        let mut step: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 4 { 1.0 } else { 0.0 })
            .collect();
        blur_box(&mut step, w, h, 1.0);
        assert!(step[3] < 1.0 && step[4] > 0.0, "{step:?}");
    }

    #[test]
    fn ranges_are_checked() {
        assert!(WaterEdge::default().validate().is_ok());
        for bad in [
            WaterEdge {
                width: 0.05,
                ..WaterEdge::default()
            },
            WaterEdge {
                width: 20.5,
                ..WaterEdge::default()
            },
            WaterEdge {
                opacity: 1.5,
                ..WaterEdge::default()
            },
            WaterEdge {
                darkness: -0.1,
                ..WaterEdge::default()
            },
            WaterEdge {
                blur: 21.0,
                ..WaterEdge::default()
            },
        ] {
            assert_eq!(
                bad.validate(),
                Err(CoreError::InvalidArgument(Reason::WaterEdgeRange))
            );
        }
        assert_eq!(
            WaterEdge {
                width: f64::NAN,
                ..WaterEdge::default()
            }
            .validate(),
            Err(CoreError::InvalidArgument(Reason::WaterEdge))
        );
    }

    #[test]
    fn a_post_corrected_stroke_gets_its_edge_from_the_redrawn_line_only() {
        // 後補正の描き直しの後の縁は、整えた入力を初めから描いた線（後補正なし）と同じ画素。揺れた元の線の縁の印は残らない
        // （描いている間に付ける縁も、ドラッグ後に処理の縁とぼかし幅も）
        let wobbly: Vec<crate::BrushSample> = (0..=112)
            .map(|i| {
                let y = 48.0 + if (i / 2) % 2 == 0 { 0.6 } else { -0.6 };
                crate::BrushSample::new(
                    24.0 + f64::from(i),
                    y,
                    1.0,
                    f64::from(i) / 200.0,
                    DVec2::ZERO,
                )
                .unwrap()
            })
            .collect();
        let draw = |b: &Brush, samples: &[crate::BrushSample]| {
            let mut d = Document::with_tile_size(160, 96, 32).unwrap();
            let l = d.add_layer("L").unwrap();
            let mut s = d.begin_brush_stroke(l, b).unwrap();
            for p in samples {
                s.add_sample(&mut d, *p).unwrap();
            }
            d.end_stroke(s).unwrap();
            d.composite(d.bounds()).unwrap()
        };
        for (what, edge) in [
            (
                "描いている間",
                WaterEdge {
                    width: 4.0,
                    opacity: 0.3,
                    darkness: 0.6,
                    after_stroke: false,
                    blur: 0.0,
                },
            ),
            (
                "ドラッグ後",
                WaterEdge {
                    width: 4.0,
                    opacity: 0.3,
                    darkness: 0.6,
                    after_stroke: true,
                    blur: 2.0,
                },
            ),
        ] {
            let mut plain = Brush::simple(
                BrushSettings {
                    radius: 12.0,
                    hardness: 0.7,
                    spacing: 0.1,
                    color: crate::Rgba8::new(48, 112, 192, 255),
                    ..BrushSettings::default()
                },
                PressureSwitches::NONE,
            );
            plain.water_edge = Some(edge);
            let mut corrected = plain.clone();
            corrected.assist.post = crate::PostCorrection {
                on: true,
                strength: 10.0, // 許し 1 画素 > 揺れ 0.6
                by_speed: false,
                by_view_scale: false,
                ..crate::PostCorrection::default()
            };
            // 整えた入力（描き直しが与える列）
            let points: Vec<crate::brush::PostPoint> = wobbly
                .iter()
                .map(|s| crate::brush::PostPoint {
                    x: s.x,
                    y: s.y,
                    pressure: s.pressure,
                    time: s.time,
                    tilt: s.tilt,
                    rotation: s.rotation,
                })
                .collect();
            let plan = crate::brush::correction_plan(
                &points,
                &[],
                corrected.assist.post.tolerance(false),
                None,
            )
            .expect("揺れは許しより小さいので整える");
            let fixed: Vec<crate::BrushSample> = plan
                .into_iter()
                .flat_map(|(points, _)| points)
                .map(|p| crate::BrushSample::new(p.x, p.y, p.pressure, p.time, p.tilt).unwrap())
                .collect();
            let raw = draw(&plain, &wobbly);
            let redrawn = draw(&corrected, &wobbly);
            let from_scratch = draw(&plain, &fixed);
            assert_ne!(redrawn, raw, "{what}: 描き直した");
            let differing = redrawn
                .iter()
                .zip(&from_scratch)
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                differing, 0,
                "{what}: 整えた入力を初めから描いた線と違うバイト"
            );
        }
    }
}
