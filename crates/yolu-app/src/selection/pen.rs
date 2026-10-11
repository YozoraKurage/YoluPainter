//! 選択ペン・選択消し（CLIP STUDIO の「選択ペン」「選択消し」）。ブラシで塗るように選択範囲を足す・消すツールで、クイックマスクの
//! ブラシ・消しゴムも同じ仕組みで直す（`quick`）。
//!
//! ダブは今のブラシの直径・硬さ・不透明度（`AppState::brush`）の形で、画素ごとの量（0〜255）に描く。硬さまでは満量、そこから縁へ
//! なめらかに（core のブラシと同じ smoothstep）減る。1 ストローク分のダブは「ストロークの被覆」（タイルごとの量。重なりは大きい方）
//! に積み、離したときに今の選択範囲へ足す（選択ペン）か引く（選択消し）。被覆を文書の選択範囲と 1 回だけ組み合わせるので、
//! ダブの間隔や重なりに量が左右されず、1 ストロークが 1 回の Undo になる（`SelEdit::Shape`）。ストロークの途中は文書を変えない
//! （描いている間の見た目は被覆の重ね表示）ので、Esc・フォーカス喪失・ツールの切り替えで捨てても何も残らない。
//!
//! 被覆は文書と同じ大きさのタイルの量で、触れたタイルだけを持つ。メモリの予算（`PEN_BUDGET_BYTES`）を超えるストロークは、
//! 途中でも断って捨てる。
//!
//! 3D ビューのクイックマスク（`view3d::quick`）も同じ被覆を使う: 面のダブの覆い（文書の画素ごとの量）を [`PenStroke::raise`] で
//! 積む（ダブの形と間隔は core の `SurfaceCoverStroke` が 2D と同じ値で決める）。

use std::collections::{HashMap, HashSet};

use egui::{Modifiers, Pos2};

use super::{SelAction, SelEdit, ShapeDrag};
use crate::canvas::view::CanvasView;
use crate::engine::{CoreError, Document, SelectionCombine, SelectionMask, TileCoord};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{Action, AppState, BrushState, StrokeSource, Tool};

/// 1 ストロークが使ってよい作業の場所（被覆・見た目の 2 つの札・クイックマスクの合成結果のぶんも数えて、持つ量の 3 倍で見る）。
pub const PEN_BUDGET_BYTES: u64 = 256 << 20;

/// ダブの形（ストロークの始めに固める）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenParams {
    /// 半径（画素）。
    pub radius: f64,
    /// 0〜1。この割合までは満量で、縁へなめらかに減る。
    pub hardness: f64,
    /// 0〜1。ダブの最大の量。
    pub opacity: f64,
    /// 筆圧で半径を変える・量を変える。
    pub pressure_size: bool,
    pub pressure_opacity: bool,
}

impl PenParams {
    /// 3D ビューの面のダブの形（同じ値）。
    pub fn cover(&self) -> yolu_core::geometry::CoverParams {
        yolu_core::geometry::CoverParams {
            radius: self.radius,
            hardness: self.hardness,
            opacity: self.opacity,
            pressure_size: self.pressure_size,
            pressure_opacity: self.pressure_opacity,
        }
    }

    /// 今のブラシの設定から。
    pub fn from_brush(b: &BrushState) -> PenParams {
        PenParams {
            radius: b.radius as f64,
            hardness: (b.hardness as f64).clamp(0.0, 1.0),
            opacity: (b.opacity as f64).clamp(0.0, 1.0),
            pressure_size: b.pressure_size,
            pressure_opacity: b.pressure_opacity,
        }
    }
}

/// ストロークを続けられない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenError {
    /// 触れたタイルが作業の予算を超えた。
    TooLarge,
    /// 選択範囲の札を作れなかった（文書と大きさが合わないなど）。
    Core,
}

impl PenError {
    pub fn text(self, lang: Lang) -> &'static str {
        match self {
            PenError::TooLarge => lang.pick("ストロークが大きすぎます", "The stroke is too large"),
            PenError::Core => lang.pick("選択範囲を作れません", "Cannot build the selection"),
        }
    }
}

impl From<CoreError> for PenError {
    fn from(_: CoreError) -> Self {
        PenError::Core
    }
}

/// 中心から `dist`（半径に対する割合）の量（0〜1）。core のブラシの縁（`DualShape`）と同じ式。
pub fn dab_coverage(dist: f64, hardness: f64) -> f64 {
    if dist > 1.0 {
        return 0.0;
    }
    if dist > hardness {
        let t = (1.0 - dist) / (1.0 - hardness);
        return t * t * (3.0 - 2.0 * t);
    }
    1.0
}

/// 1 ストローク分の被覆（と、クイックマスクのときは今の選択範囲に重ねた見た目）。
pub struct PenStroke {
    pub erase: bool,
    params: PenParams,
    width: u32,
    height: u32,
    ts: u32,
    budget: u64,
    cover: HashMap<TileCoord, Vec<u8>>,
    /// まだ札に反映していないタイル。
    dirty: HashSet<TileCoord>,
    /// `sync` で札に反映したタイルのうち、2D の重ね表示がまだ読んでいないもの（重ね表示が作り直す範囲の手がかり。読むと空になる）。
    /// 2D のキャンバスと 3D ビューが同じフレームに `sync` を呼んでも、先に呼んだほうが反映したタイルを、重ね表示が受け取れる。
    synced: HashSet<TileCoord>,
    cover_mask: SelectionMask,
    /// クイックマスク: ストロークの始めの選択範囲と、それに被覆を重ねた見た目。
    base: Option<SelectionMask>,
    preview: Option<SelectionMask>,
    last: Option<(f64, f64, f64)>,
    /// 最後のダブからの距離。
    since: f64,
    dabs: u64,
}

impl PenStroke {
    /// 文書に向けて始める。`overlay_base` があれば、それに被覆を重ねた見た目も持つ（クイックマスク。何も選んでいないなら空の札を渡す）。
    pub fn new(
        doc: &Document,
        erase: bool,
        overlay_base: Option<SelectionMask>,
        params: PenParams,
        budget: u64,
    ) -> Result<PenStroke, PenError> {
        let cover_mask = SelectionMask::none(doc);
        let preview = overlay_base.clone();
        Ok(PenStroke {
            erase,
            params,
            width: doc.width(),
            height: doc.height(),
            ts: doc.tile_size(),
            budget,
            cover: HashMap::new(),
            dirty: HashSet::new(),
            synced: HashSet::new(),
            cover_mask,
            base: overlay_base,
            preview,
            last: None,
            since: 0.0,
            dabs: 0,
        })
    }

    /// 足すか引くか（組み合わせ方）。
    pub fn mode(&self) -> SelectionCombine {
        if self.erase {
            SelectionCombine::Subtract
        } else {
            SelectionCombine::Add
        }
    }

    /// 積んだダブの数。
    pub fn dabs(&self) -> u64 {
        self.dabs
    }

    /// 被覆のタイルの数。
    pub fn tile_count(&self) -> usize {
        self.cover.len()
    }

    /// 作業の予算に数える被覆のバイト（タイル 1 枚を、被覆・見た目の 2 つの札・合成の結果の 3 倍で見る。`tile_mut` と同じ数え方）。
    pub fn bytes(&self) -> u64 {
        self.cover.len() as u64 * (self.ts as u64 * self.ts as u64) * 3
    }

    /// 作業の予算（バイト）。
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// 外で求めたダブ 1 つの量（文書の画素と 0〜255。3D ビューの面のダブ）を被覆に積む（大きい方を残す）。文書の外の画素は飛ばす。
    pub fn raise(&mut self, pixels: &[yolu_core::geometry::CoverPixel]) -> Result<(), PenError> {
        self.dabs += 1;
        let (w, h, ts) = (self.width as i32, self.height as i32, self.ts as i32);
        for &(x, y, a) in pixels {
            if a == 0 || x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
            let i = ((y % ts) * ts + x % ts) as usize;
            let tile = self.tile_mut(coord)?;
            if a > tile[i] {
                tile[i] = a;
            }
        }
        Ok(())
    }

    /// ダブの半径と最大の量（筆圧を入れたもの）。
    fn dab_params(&self, pressure: f64) -> (f64, f64) {
        let p = pressure.clamp(0.0, 1.0);
        let radius = if self.params.pressure_size {
            self.params.radius * p.max(0.05)
        } else {
            self.params.radius
        };
        let opacity = if self.params.pressure_opacity {
            self.params.opacity * p
        } else {
            self.params.opacity
        };
        (radius.max(0.75), opacity)
    }

    /// 点（キャンバスの座標）まで、前の点から間隔を空けてダブを積む。最初の点には 1 つ置く。
    pub fn add(&mut self, x: f64, y: f64, pressure: f64) -> Result<(), PenError> {
        let Some((lx, ly, lp)) = self.last else {
            let (r, a) = self.dab_params(pressure);
            self.stamp(x, y, r, a)?;
            self.last = Some((x, y, pressure));
            self.since = 0.0;
            return Ok(());
        };
        let (dx, dy) = (x - lx, y - ly);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            self.last = Some((x, y, pressure));
            return Ok(());
        }
        // 間隔は直径の 5%（0.5 画素以上）。量は被覆の大きい方なので、間隔が量を左右しない
        let (r0, _) = self.dab_params(pressure);
        let step = (2.0 * r0 * 0.05).max(0.5);
        let mut t = step - self.since;
        let mut placed_at = None;
        while t <= len {
            let k = t / len;
            let p = lp + (pressure - lp) * k;
            let (r, a) = self.dab_params(p);
            self.stamp(lx + dx * k, ly + dy * k, r, a)?;
            placed_at = Some(t);
            t += step;
        }
        self.since = match placed_at {
            Some(at) => len - at,
            None => self.since + len,
        };
        self.last = Some((x, y, pressure));
        Ok(())
    }

    /// 被覆のタイル（無ければ作る。予算を超えるなら断る）。
    fn tile_mut(&mut self, coord: TileCoord) -> Result<&mut Vec<u8>, PenError> {
        if !self.cover.contains_key(&coord) {
            let bytes = (self.cover.len() as u64 + 1) * (self.ts as u64 * self.ts as u64) * 3;
            if bytes > self.budget {
                return Err(PenError::TooLarge);
            }
            self.cover
                .insert(coord, vec![0u8; (self.ts * self.ts) as usize]);
        }
        self.dirty.insert(coord);
        Ok(self.cover.get_mut(&coord).expect("上で入れた"))
    }

    /// ダブ 1 つ（中心 (x, y)・半径 `r`・最大の量 0〜1）を被覆に積む（大きい方を残す）。
    fn stamp(&mut self, x: f64, y: f64, r: f64, opacity: f64) -> Result<(), PenError> {
        self.dabs += 1;
        let (w, h, ts) = (self.width as i64, self.height as i64, self.ts as i64);
        let x0 = ((x - r).floor() as i64).max(0);
        let x1 = ((x + r).ceil() as i64).min(w);
        let y0 = ((y - r).floor() as i64).max(0);
        let y1 = ((y + r).ceil() as i64).min(h);
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        let hardness = self.params.hardness;
        for ty in (y0 / ts)..=((y1 - 1) / ts) {
            for tx in (x0 / ts)..=((x1 - 1) / ts) {
                let (px0, px1) = (x0.max(tx * ts), x1.min((tx + 1) * ts));
                let (py0, py1) = (y0.max(ty * ts), y1.min((ty + 1) * ts));
                let coord = TileCoord::new(tx as u32, ty as u32);
                // 量が 0 のダブ（筆圧 0 など）はタイルを作らない
                let mut touched = false;
                let mut local: Vec<(usize, u8)> = Vec::new();
                for py in py0..py1 {
                    let dy = py as f64 + 0.5 - y;
                    for px in px0..px1 {
                        let dx = px as f64 + 0.5 - x;
                        let dist = (dx * dx + dy * dy).sqrt() / r;
                        let c = dab_coverage(dist, hardness);
                        let a = (255.0 * c * opacity).round() as u8;
                        if a > 0 {
                            local.push((((py - ty * ts) * ts + (px - tx * ts)) as usize, a));
                            touched = true;
                        }
                    }
                }
                if !touched {
                    continue;
                }
                let tile = self.tile_mut(coord)?;
                for (i, a) in local {
                    if a > tile[i] {
                        tile[i] = a;
                    }
                }
            }
        }
        Ok(())
    }

    /// 触れたタイルを札に反映する（重ね表示の前に 1 フレームに 1 度）。
    pub fn sync(&mut self) -> Result<(), PenError> {
        if self.dirty.is_empty() {
            return Ok(());
        }
        let coords: Vec<TileCoord> = self.dirty.drain().collect();
        let tiles: Vec<(TileCoord, Vec<u8>)> =
            coords.iter().map(|c| (*c, self.cover[c].clone())).collect();
        self.cover_mask = self.cover_mask.with_tiles(tiles)?;
        if let (Some(base), Some(preview)) = (&self.base, &self.preview) {
            let n = (self.ts * self.ts) as usize;
            let (mut a, mut b) = (vec![0u8; n], vec![0u8; n]);
            let mut out = Vec::with_capacity(coords.len());
            for c in &coords {
                base.copy_tile(*c, &mut a)?;
                self.cover_mask.copy_tile(*c, &mut b)?;
                let combined: Vec<u8> = a
                    .iter()
                    .zip(&b)
                    .map(|(&a, &b)| combine_amount(a, b, self.erase))
                    .collect();
                out.push((*c, combined));
            }
            self.preview = Some(preview.with_tiles(out)?);
        }
        self.synced.extend(coords);
        Ok(())
    }

    /// 被覆の札（`sync` のあと）。
    pub fn cover(&self) -> &SelectionMask {
        &self.cover_mask
    }

    /// クイックマスクの見た目（始めの選択範囲に被覆を重ねたもの。`sync` のあと）。
    pub fn preview(&self) -> Option<&SelectionMask> {
        self.preview.as_ref()
    }

    /// 2D の重ね表示がまだ読んでいない、変わったタイルを取り出して空にする（何も無ければ None。重ね表示は、None なら中身を見比べる）。
    pub fn take_synced(&mut self) -> Option<Vec<TileCoord>> {
        (!self.synced.is_empty()).then(|| self.synced.drain().collect())
    }

    /// 終える: 被覆の札と組み合わせ方。
    pub fn finish(mut self) -> Result<(SelectionMask, SelectionCombine), PenError> {
        self.sync()?;
        let mode = self.mode();
        Ok((self.cover_mask, mode))
    }
}

/// core の `combine` と同じ式（足すは大きい方、引くは `a × (255 − b) / 255` を四捨五入）。
pub fn combine_amount(a: u8, b: u8, erase: bool) -> u8 {
    if erase {
        ((a as u32 * (255 - b as u32) + 127) / 255) as u8
    } else {
        a.max(b)
    }
}

// ───────── ツール（キャンバスの入力） ─────────

/// 動いているストローク 1 つ（選択ペンのツールか、クイックマスクのブラシ・消しゴム）。
pub struct ActivePen {
    pub stroke: PenStroke,
    pub source: StrokeSource,
    /// クイックマスクのブラシ・消しゴム（false なら選択ペンのツール）。
    pub quick: bool,
}

/// 選択ペンのツールで消すか（基本の切り替えと、押したときの修飾: 作成方法の割り当て（`combine_for`）で、足すに当たる修飾は選択ペン・
/// 引くに当たる修飾は選択消しに、押しているあいだだけ替える）。
pub fn erases(base_erase: bool, modifiers: Modifiers) -> bool {
    match super::combine_for(egui::PointerButton::Primary, &modifiers) {
        Some(SelectionCombine::Add) => false,
        Some(SelectionCombine::Subtract) => true,
        _ => base_erase,
    }
}

/// ストロークを始める（選択ペンのツール・クイックマスクで共通）。始められなければ理由をステータスバーへ。
pub fn begin(app: &mut AppState, source: StrokeSource, erase: bool, quick: bool) -> bool {
    let params = PenParams::from_brush(&app.brush);
    let overlay_base = if quick {
        Some(
            app.doc
                .selection()
                .cloned()
                .unwrap_or_else(|| SelectionMask::none(&app.doc)),
        )
    } else {
        None
    };
    match PenStroke::new(&app.doc, erase, overlay_base, params, app.sel.pen_budget) {
        Ok(stroke) => {
            app.sel.pen = Some(ActivePen {
                stroke,
                source,
                quick,
            });
            true
        }
        Err(e) => {
            app.fail(Source::Selection, e.text(app.lang));
            false
        }
    }
}

/// 点を足す。予算を超えたらストロークを捨てて理由を出す（捨てたら false）。
pub fn add_point(app: &mut AppState, view: &CanvasView, pos: Pos2, pressure: f32) -> bool {
    let (x, y) = view.to_canvas(pos);
    let Some(active) = app.sel.pen.as_mut() else {
        return false;
    };
    match active.stroke.add(x, y, pressure as f64) {
        Ok(()) => true,
        Err(e) => {
            let quick = active.quick;
            app.sel.pen = None;
            app.sel.drag = None;
            if quick {
                app.canvas.stroke = None;
            }
            app.fail(Source::Selection, e.text(app.lang));
            false
        }
    }
}

/// ストロークを終える。cancel でなければ文書の選択範囲に反映する（1 回の Undo）。動いていなければ何もしない。
pub fn finish(app: &mut AppState, cancel: bool) -> bool {
    let Some(active) = app.sel.pen.take() else {
        return false;
    };
    app.sel.drag = None;
    if cancel {
        app.info(
            Source::Selection,
            app.lang
                .pick("ストロークを取り消しました。", "Stroke cancelled."),
        );
        return true;
    }
    match active.stroke.finish() {
        Ok((cover, mode)) => {
            if !cover.is_empty() {
                app.apply(Action::Sel(SelAction::Edit(SelEdit::Shape {
                    mask: cover,
                    mode,
                })));
            }
        }
        Err(e) => app.fail(Source::Selection, e.text(app.lang)),
    }
    true
}

/// 毎フレームの前に、触れたタイルを札へ反映する（描いている間の重ね表示のため）。
pub fn sync(app: &mut AppState) {
    if let Some(active) = app.sel.pen.as_mut() {
        if let Err(e) = active.stroke.sync() {
            app.sel.pen = None;
            app.sel.drag = None;
            app.canvas.stroke = None;
            app.fail(Source::Selection, e.text(app.lang));
        }
    }
}

/// 選択ペンのツールを押した。
pub fn press(
    app: &mut AppState,
    view: &CanvasView,
    pos: Pos2,
    source: StrokeSource,
    modifiers: Modifiers,
) {
    if app.sel.pen.is_some() {
        return;
    }
    let eraser_end = matches!(source, StrokeSource::Pen(id)
        if app.sel.pen_note.is_some_and(|(n, _, e)| n == id && e));
    let erase = erases(app.sel.pen_erase || eraser_end, modifiers);
    if !begin(app, source, erase, false) {
        return;
    }
    let canvas = view.to_canvas(pos);
    // ペンが触れた・離したを押す・離すにするので、ドラッグの印（離しの振り分け・取りこぼしの救済）をほかの形と同じに持つ
    app.sel.drag = Some(ShapeDrag {
        tool: Tool::SelectPen,
        source,
        start_screen: pos,
        start: canvas,
        current: canvas,
        lasso: Vec::new(),
        moved: 0.0,
    });
    let pressure = app.sel.pen_pressure(source, app.canvas.touch_pressure);
    add_point(app, view, pos, pressure);
}

/// 選択ペンのツールのポインタが動いた。
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app
        .sel
        .pen
        .as_ref()
        .is_none_or(|a| a.source != source || a.quick)
    {
        return;
    }
    let pressure = app.sel.pen_pressure(source, app.canvas.touch_pressure);
    add_point(app, view, pos, pressure);
}
