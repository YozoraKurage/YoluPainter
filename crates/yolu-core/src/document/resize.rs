//! 文書の解像度変更。C# ResampleAxis / CanvasResampler と同じ整数比の重み。
//!
//! 行き先の面はタイルごとに作る。C# の CanvasResampler と同じく、読む元のタイルが 1 枚も無い行き先は飛ばし、読む元が全部同じ
//! 一様なタイルなら計算せずその色で埋める（どちらも画素ごとに計算した結果と同じバイトで、疎な層の費用が内容に比例する）。
use super::operations::Dirty;
use super::{Document, Target};
use crate::effects::LayerPath;
use crate::math::to_byte;
use crate::paths::{render_canvas, CanvasPath, CanvasPoint, Options};
use crate::surface::Tile;
use crate::{Channel, ChannelKind, CoreError, LayerId, NormalSettings, Rgba8, Surface, TileCoord};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasResampling {
    Nearest,
    Bilinear,
    Area,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResizeReport {
    pub notes: Vec<String>,
    /// モデルの上のパスで描かれた層。UV に結び付いたパスは残り、パスのチャンネルの画素は画素として写した（resize_image は補間、
    /// resize_canvas はずらし）だけなので、呼び手がモデルでパスから描き直すか、写した画素のままにして知らせる（C# の
    /// `ResampledDocument.SurfacePathLayers`）。
    pub surface_path_layers: Vec<LayerId>,
    /// この変更の履歴の費用（前後の格納量）が履歴の予算を超えた。Undo はこの 1 段だけ残し、次の編集の整理で落ち得る。
    pub history_over_budget: bool,
    /// 縮小で何も選んでいない所だけが残り、外した覚えた選択範囲（名前を付けて残したもの）の数。`notes` にも文で書く（日本語のみ）。
    /// 取り消せば元の大きさのものへ戻る。呼び手が履歴を消すときは、戻せなくなる前にこの数を利用者へ知らせる。
    pub dropped_saved_selections: usize,
}
/// [`Document::prepare_resize_image`] が作る、大きさを変えた文書の写し（まだ文書に入っていない）。
pub struct PreparedResize {
    copy: Document,
    report: ResizeReport,
    id: u128,
    revision: u64,
}
impl PreparedResize {
    /// 入れたときに返す報告（準備の段階で分かっている分）。
    pub fn report(&self) -> &ResizeReport {
        &self.report
    }
}
struct Axis(Vec<Vec<(u32, f64)>>);
impl Axis {
    fn new(source: u32, target: u32, method: CanvasResampling) -> Self {
        let (s, t) = (source as i64, target as i64);
        Self(
            (0..t)
                .map(|i| match method {
                    CanvasResampling::Nearest => vec![(((2 * i + 1) * s / (2 * t)) as u32, 1.)],
                    CanvasResampling::Bilinear => {
                        let num = (2 * i + 1) * s - t;
                        let den = 2 * t;
                        let j = num.div_euclid(den);
                        let rem = num - j * den;
                        let a = j.max(0);
                        let b = (j + 1).min(s - 1);
                        if rem == 0 || a == b {
                            vec![(if rem == 0 { j.clamp(0, s - 1) } else { a } as u32, 1.)]
                        } else {
                            let f = rem as f64 / den as f64;
                            vec![(a as u32, 1. - f), (b as u32, f)]
                        }
                    }
                    CanvasResampling::Area => {
                        let lo = i * s;
                        let hi = (i + 1) * s;
                        (lo / t..=(hi - 1) / t)
                            .map(|j| {
                                (
                                    j as u32,
                                    ((hi.min((j + 1) * t) - lo.max(j * t)) as f64) / s as f64,
                                )
                            })
                            .collect()
                    }
                })
                .collect(),
        )
    }
}
impl Axis {
    /// 行き先の [lo, hi]（両端を含む）が読む元の [最初, 最後]（両端を含む）。元の並びは行き先の並びと同じ向きに進む。
    fn span(&self, lo: u32, hi: u32) -> Span {
        Span {
            lo: self.0[lo as usize][0].0,
            hi: self.0[hi as usize].last().expect("1 つ以上").0,
            inside: true,
        }
    }
}

/// 行き先のある範囲が読む元の範囲（両端を含む。元の画布の中だけ）。
struct Span {
    lo: u32,
    hi: u32,
    /// 範囲のどの画素も元の画布の中を読む（外を読む画素があると、一様な元でも外は透明なので埋められない）。
    inside: bool,
}

/// 元の面を、直前に読んだタイルを覚えて読む（C# の TileReader）。読み手ごとに 1 つ作り、共有しない。
struct Reader<'a> {
    surface: &'a Surface,
    tile_size: u32,
    at: (u32, u32),
    tile: Option<&'a Tile>,
}
impl<'a> Reader<'a> {
    fn new(surface: &'a Surface) -> Self {
        Self {
            surface,
            tile_size: surface.tile_size(),
            at: (u32::MAX, u32::MAX),
            tile: None,
        }
    }
    /// 画布の中の画素。無いタイルは透明。
    #[inline]
    fn get(&mut self, x: u32, y: u32) -> Rgba8 {
        debug_assert!(x < self.surface.width() && y < self.surface.height());
        let ts = self.tile_size;
        let at = (x / ts, y / ts);
        if at != self.at {
            self.at = at;
            self.tile = self.surface.tile(TileCoord::new(at.0, at.1));
        }
        self.tile.map_or(Rgba8::TRANSPARENT, |t| {
            t.get((((y % ts) * ts + x % ts) * 4) as usize)
        })
    }
    /// 画布の外は透明。
    #[inline]
    fn get_or_transparent(&mut self, x: i64, y: i64) -> Rgba8 {
        if x < 0 || y < 0 || x >= self.surface.width() as i64 || y >= self.surface.height() as i64 {
            Rgba8::TRANSPARENT
        } else {
            self.get(x as u32, y as u32)
        }
    }
}

/// 行き先の画素が元のどこから来るか。
trait Mapping: Sync {
    /// 行き先の [lo, hi]（両端を含む）が読む元の範囲。全部が元の外なら None。
    fn span_x(&self, lo: u32, hi: u32) -> Option<Span>;
    fn span_y(&self, lo: u32, hi: u32) -> Option<Span>;
    fn pixel(&self, source: &mut Reader<'_>, normal: bool, x: u32, y: u32) -> Rgba8;
}
/// 整数比の重みで拡大・縮小する（resize_image）。
struct Resampled {
    xs: Axis,
    ys: Axis,
}
impl Mapping for Resampled {
    fn span_x(&self, lo: u32, hi: u32) -> Option<Span> {
        Some(self.xs.span(lo, hi))
    }
    fn span_y(&self, lo: u32, hi: u32) -> Option<Span> {
        Some(self.ys.span(lo, hi))
    }
    fn pixel(&self, source: &mut Reader<'_>, normal: bool, x: u32, y: u32) -> Rgba8 {
        resampled_pixel(
            source,
            &self.xs.0[x as usize],
            &self.ys.0[y as usize],
            normal,
        )
    }
}
/// 補間せずに整数の位置へずらす（resize_canvas）。offset は元の左下を置く先。
struct Shifted {
    offset: (i32, i32),
    source: (u32, u32),
}
impl Shifted {
    fn span(lo: u32, hi: u32, offset: i32, size: u32) -> Option<Span> {
        let (a, b) = (lo as i64 - offset as i64, hi as i64 - offset as i64);
        let (first, last) = (a.max(0), b.min(size as i64 - 1));
        (first <= last).then_some(Span {
            lo: first as u32,
            hi: last as u32,
            inside: first == a && last == b,
        })
    }
}
impl Mapping for Shifted {
    fn span_x(&self, lo: u32, hi: u32) -> Option<Span> {
        Self::span(lo, hi, self.offset.0, self.source.0)
    }
    fn span_y(&self, lo: u32, hi: u32) -> Option<Span> {
        Self::span(lo, hi, self.offset.1, self.source.1)
    }
    fn pixel(&self, source: &mut Reader<'_>, _normal: bool, x: u32, y: u32) -> Rgba8 {
        source.get_or_transparent(
            x as i64 - self.offset.0 as i64,
            y as i64 - self.offset.1 as i64,
        )
    }
}

fn resampled_pixel(
    source: &mut Reader<'_>,
    xs: &[(u32, f64)],
    ys: &[(u32, f64)],
    normal: bool,
) -> Rgba8 {
    if xs.len() == 1 && ys.len() == 1 {
        return source.get(xs[0].0, ys[0].0);
    }
    let (mut a, mut r, mut g, mut b, mut zw, mut zr, mut zg, mut zb) =
        (0., 0., 0., 0., 0., 0., 0., 0.);
    let mut first = None;
    let mut same = true;
    for &(y, wy) in ys {
        if wy <= 0. {
            continue;
        }
        for &(x, wx) in xs {
            let w = wy * wx;
            if w <= 0. {
                continue;
            }
            let p = source.get(x, y);
            if let Some(f) = first {
                if p != f {
                    same = false;
                }
            } else {
                first = Some(p);
            }
            if p.a == 0 {
                zw += w;
                zr += w * p.r as f64;
                zg += w * p.g as f64;
                zb += w * p.b as f64;
                continue;
            }
            let k = w * p.a as f64;
            a += k;
            r += k * p.r as f64;
            g += k * p.g as f64;
            b += k * p.b as f64;
        }
    }
    if same {
        return first.unwrap_or(Rgba8::TRANSPARENT);
    }
    let alpha = to_byte(a / 255.);
    if alpha == 0 {
        return if zw > 0. {
            Rgba8::new(
                to_byte(zr / zw / 255.),
                to_byte(zg / zw / 255.),
                to_byte(zb / zw / 255.),
                0,
            )
        } else {
            Rgba8::TRANSPARENT
        };
    }
    if normal {
        crate::normal::encode(
            2. * r / a / 255. - 1.,
            2. * g / a / 255. - 1.,
            2. * b / a / 255. - 1.,
            alpha,
        )
    } else {
        Rgba8::new(
            to_byte(r / a / 255.),
            to_byte(g / a / 255.),
            to_byte(b / a / 255.),
            alpha,
        )
    }
}

/// 新しい面の格納量の合計と上限（None は数えるだけ）。
struct Budget {
    used: u64,
    limit: Option<u64>,
}

/// 元の面から、寸法の違う行き先の面を作る。行き先のタイルごとに、読む元のタイルが 1 枚も無ければ飛ばし、全部が同じ一様な
/// 色なら計算せずその色で埋める。残りはタイルのまとまりごとに並列で計算し、まとまりごとに取消を確かめる。
fn resample_surface(
    source: &Surface,
    width: u32,
    height: u32,
    map: &dyn Mapping,
    normal: bool,
    cancelled: &mut dyn FnMut() -> bool,
    budget: &mut Budget,
) -> Result<Surface, CoreError> {
    if cancelled() {
        return Err(CoreError::Cancelled);
    }
    let ts = source.tile_size();
    let mut out = Surface::new(width, height, ts);
    if source.tile_count() == 0 {
        return Ok(out);
    }
    let mut work: Vec<(TileCoord, Option<Rgba8>)> = Vec::new();
    for ty in 0..height.div_ceil(ts) {
        for tx in 0..width.div_ceil(ts) {
            let (x0, y0) = (tx * ts, ty * ts);
            let (x1, y1) = (width.min(x0 + ts) - 1, height.min(y0 + ts) - 1);
            let (Some(sx), Some(sy)) = (map.span_x(x0, x1), map.span_y(y0, y1)) else {
                continue;
            };
            let (mut any, mut all_uniform, mut color) = (false, sx.inside && sy.inside, None);
            'scan: for cy in sy.lo / ts..=sy.hi / ts {
                for cx in sx.lo / ts..=sx.hi / ts {
                    match source.tile(TileCoord::new(cx, cy)) {
                        None => all_uniform = false,
                        Some(tile) => {
                            any = true;
                            match tile {
                                Tile::Uniform(c) if all_uniform => match color {
                                    None => color = Some(*c),
                                    Some(first) if first != *c => all_uniform = false,
                                    Some(_) => {}
                                },
                                _ => all_uniform = false,
                            }
                        }
                    }
                    if any && !all_uniform {
                        break 'scan;
                    }
                }
            }
            if any {
                work.push((TileCoord::new(tx, ty), color.filter(|_| all_uniform)));
            }
        }
    }
    for batch in work.chunks(rayon::current_num_threads().clamp(1, 64) * 4) {
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        let tiles: Vec<_> = batch
            .par_iter()
            .map(|&(coord, uniform)| {
                let (x0, y0) = (coord.x * ts, coord.y * ts);
                let (tw, th) = (ts.min(width - x0), ts.min(height - y0));
                let mut bytes = vec![0; source.tile_bytes()];
                let mut reader = Reader::new(source);
                for y in 0..th {
                    for x in 0..tw {
                        let p = match uniform {
                            Some(c) => c,
                            None => map.pixel(&mut reader, normal, x0 + x, y0 + y),
                        };
                        let at = ((y * ts + x) * 4) as usize;
                        bytes[at..at + 4].copy_from_slice(&p.to_array());
                    }
                }
                (coord, Tile::from_bytes(&bytes))
            })
            .collect();
        for (coord, tile) in tiles {
            budget.used += tile.as_ref().map_or(0, Tile::byte_size);
            if budget.limit.is_some_and(|limit| budget.used > limit) {
                return Err(CoreError::SourceBudgetExceeded);
            }
            out.restore(coord, tile.as_ref());
        }
    }
    Ok(out)
}

impl Document {
    pub const MAX_NATIVE_SIDE: u32 = 8192;
    /// 解像度を変更する。ロックに関係なく全チャンネル・マスクを処理し、1 回の Undo で寸法と設定も戻す。履歴の費用は前後の格納量で、
    /// 履歴の予算を超えてもこの 1 段は残す（古い段を落とし、`ResizeReport::history_over_budget` で知らせる。断りはしない）。
    pub fn resize_image(
        &mut self,
        width: u32,
        height: u32,
        method: CanvasResampling,
    ) -> Result<ResizeReport, CoreError> {
        self.resize_image_cancellable(width, height, method, &mut || false)
    }
    pub fn resize_image_cancellable(
        &mut self,
        width: u32,
        height: u32,
        method: CanvasResampling,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<ResizeReport, CoreError> {
        match self.prepare_resize_image(width, height, method, cancelled)? {
            Some(prepared) => self.commit_prepared_resize(prepared),
            None => Ok(ResizeReport::default()),
        }
    }
    /// [`Document::resize_image`] の失敗しうる前半だけ（結果の写しを作る。文書も履歴も変えない）。大きさが今と同じなら `None`。
    /// 複数の文書を「全部変えるか、1 つも変えない」で変えるとき、先に全部を準備してから [`Document::commit_prepared_resize`] で入れる
    /// （途中の文書が予算で断られても、先の文書の履歴を積んで落とすことがない）。
    pub fn prepare_resize_image(
        &self,
        width: u32,
        height: u32,
        method: CanvasResampling,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<PreparedResize>, CoreError> {
        self.ensure_no_stroke()?;
        Self::check_resize(width, height)?;
        if width == self.width && height == self.height {
            return Ok(None);
        }
        let map = Resampled {
            xs: Axis::new(self.width, width, method),
            ys: Axis::new(self.height, height, method),
        };
        let (sx, sy) = (
            width as f64 / self.width as f64,
            height as f64 / self.height as f64,
        );
        let scale = (sx * sy).sqrt();
        let (mut copy, mut report) =
            self.resize_surfaces(width, height, &map, Fit::Scale { sx, sy, scale }, cancelled)?;
        let wanted = self.normal_settings.strength() * scale;
        let strength = wanted.clamp(-NormalSettings::MAX_STRENGTH, NormalSettings::MAX_STRENGTH);
        copy.normal_settings = self.normal_settings.with_strength(strength)?;
        if self.selection.is_some() && copy.selection.is_none() {
            report.notes.push("縮小で選択範囲が消えた".into());
        }
        if strength != wanted {
            report
                .notes
                .push("Height → Normal の強さを上限に制限した".into());
        }
        Ok(Some(PreparedResize {
            copy,
            report,
            id: self.id,
            revision: self.revision,
        }))
    }
    /// [`Document::prepare_resize_image`] の結果を 1 回の Undo の段として入れる（`resize_image` の後半）。準備のあとに文書が変わって
    /// いた・別の文書のものなら、何も変えずに断る。履歴の予算は `resize_image` と同じ（超えてもこの 1 段は残す）。
    pub fn commit_prepared_resize(
        &mut self,
        prepared: PreparedResize,
    ) -> Result<ResizeReport, CoreError> {
        self.ensure_no_stroke()?;
        let PreparedResize {
            copy,
            mut report,
            id,
            revision,
        } = prepared;
        if id != self.id || revision != self.revision {
            return Err(CoreError::InvalidArgument(
                "大きさの変更の準備のあとに文書が変わった",
            ));
        }
        self.commit_resized(copy, &mut report)?;
        Ok(report)
    }
    /// 画素を再補間せず画布だけを変更する。offset は元の左下を置く先。外へ出た画素は切り落とし、Undo で戻す。履歴の予算は
    /// [`Document::resize_image`] と同じ（超えてもこの 1 段は残す）。
    pub fn resize_canvas(
        &mut self,
        width: u32,
        height: u32,
        offset: (i32, i32),
    ) -> Result<ResizeReport, CoreError> {
        self.ensure_no_stroke()?;
        Self::check_resize(width, height)?;
        if width == self.width && height == self.height && offset == (0, 0) {
            return Ok(ResizeReport::default());
        }
        let map = Shifted {
            offset,
            source: (self.width, self.height),
        };
        let (copy, mut report) =
            self.resize_surfaces(width, height, &map, Fit::Shift(offset), &mut || false)?;
        self.commit_resized(copy, &mut report)?;
        Ok(report)
    }
    /// 前後の格納量を履歴の費用にして 1 段で交換する。予算を超えても残し、そうなったことを報告に書く。
    fn commit_resized(
        &mut self,
        copy: Document,
        report: &mut ResizeReport,
    ) -> Result<(), CoreError> {
        let cost = 128
            + self.allocated_bytes()
            + copy.allocated_bytes()
            + self.selection.as_ref().map_or(0, |s| s.history_bytes())
            + copy.selection.as_ref().map_or(0, |s| s.history_bytes())
            + self
                .saved_selections
                .iter()
                .map(|s| s.mask.history_bytes())
                .sum::<u64>()
            + copy
                .saved_selections
                .iter()
                .map(|s| s.mask.history_bytes())
                .sum::<u64>();
        self.commit_copy_kept(copy, cost, Dirty::All)?;
        report.history_over_budget = cost > self.undo_budget;
        if report.history_over_budget {
            report.notes.push("履歴の予算を超えた".into());
        }
        Ok(())
    }
    /// 選択範囲を新しい大きさへ作り直す（A だけの RGBA の面として層と同じ道で）。
    fn resample_mask(
        &self,
        selection: &crate::SelectionMask,
        width: u32,
        height: u32,
        map: &dyn Mapping,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<crate::SelectionMask, CoreError> {
        let source = super::transform::selection_surface(selection);
        let resized = resample_surface(
            &source,
            width,
            height,
            map,
            false,
            cancelled,
            &mut Budget {
                used: 0,
                limit: None,
            },
        )?;
        let n = (self.tile_size * self.tile_size) as usize;
        let tiles = resized.tile_coords().into_iter().filter_map(|coord| {
            let tile = resized.tile(coord)?;
            let amounts: Vec<u8> = (0..n).map(|i| tile.get(i * 4).a).collect();
            amounts.iter().any(|&a| a != 0).then_some((coord, amounts))
        });
        crate::SelectionMask::from_amount_tiles(width, height, self.tile_size, tiles)
    }
    fn check_resize(width: u32, height: u32) -> Result<(), CoreError> {
        if width == 0
            || height == 0
            || width > Self::MAX_NATIVE_SIDE
            || height > Self::MAX_NATIVE_SIDE
        {
            return Err(CoreError::InvalidArgument("画像の辺は 1〜8192"));
        }
        Ok(())
    }
    fn resize_surfaces(
        &self,
        width: u32,
        height: u32,
        map: &dyn Mapping,
        fit: Fit,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(Document, ResizeReport), CoreError> {
        let mut copy = self.edit_copy()?;
        copy.width = width;
        copy.height = height;
        let mut report = ResizeReport::default();
        let mut budget = Budget {
            used: 0,
            limit: Some(self.source_budget),
        };
        for i in 0..self.layers.len() {
            // 2D のパスで描かれたチャンネルは写さない（下で、大きさに合わせたパスから描き直す）。resize_image（Fit::Scale）は C# の
            // Resampled と同じ。resize_canvas（Fit::Shift: 点をずらして描き直す）に当たる C# の操作は無く、Rust 独自の決め
            let redrawn: Vec<Channel> = match &self.layers[i].path {
                Some(p) if p.is_canvas() => p.channels(),
                _ => Vec::new(),
            };
            let mut surfaces: Vec<_> = self.layers[i]
                .surface_channels()
                .into_iter()
                .filter(|c| !redrawn.contains(c))
                .map(Target::Channel)
                .collect();
            if self.layers[i].mask.is_some() {
                surfaces.push(Target::Mask);
            }
            for target in surfaces {
                let source = self.target_surface(i, target).expect("面");
                let normal = matches!(
                    target,
                    Target::Channel(c) if self.channel_kind(c).ok() == Some(ChannelKind::Normal)
                );
                *copy.target_surface_mut(i, target).expect("面") =
                    resample_surface(source, width, height, map, normal, cancelled, &mut budget)?;
            }
        }
        if let Some(selection) = &self.selection {
            // 選択範囲は A だけの RGBA の面として層と同じ道を通る（飛ばす・埋める・取消）。画素の予算には数えない
            let mask = self.resample_mask(selection, width, height, map, cancelled)?;
            copy.selection = (!mask.is_empty()).then_some(mask);
        }
        // 名前を付けて残した選択範囲も、今の選択範囲と同じ道で新しい大きさへ作り直す（大きさが文書と合わないものを残さない）。
        // 縮小で何も選んでいない所だけが残るものは外し、報告に書く（取り消せば元の大きさのものへ戻る）
        if !self.saved_selections.is_empty() {
            let mut kept = Vec::with_capacity(self.saved_selections.len());
            let mut dropped = 0usize;
            for saved in self.saved_selections.iter() {
                let mask = self.resample_mask(&saved.mask, width, height, map, cancelled)?;
                if mask.is_empty() {
                    dropped += 1;
                } else {
                    kept.push(super::SavedSelection {
                        name: saved.name.clone(),
                        mask,
                    });
                }
            }
            if dropped > 0 {
                report.dropped_saved_selections = dropped;
                report
                    .notes
                    .push(format!("縮小で残した選択範囲 {dropped} 件が消えた"));
            }
            copy.saved_selections = std::sync::Arc::new(kept);
        }
        // 画素の外の設定を大きさに合わせる（resize_image では C# の Resampled が層ごとにすること）: パス・フィルターの半径。Anchor・塗りつぶしの画像と
        // 投影・グラデーション・Generator は UV・モデルの空間・画素ごとの式で決まり、大きさによらないのでそのまま写る（層ごと複製済み）
        for i in 0..self.layers.len() {
            match &self.layers[i].path {
                Some(LayerPath::Canvas(path)) => {
                    if cancelled() {
                        return Err(CoreError::Cancelled);
                    }
                    let fitted = fit.canvas_path(path, &self.layers[i].name, &mut report.notes);
                    let rendered = render_canvas(
                        &fitted,
                        &Options {
                            width,
                            height,
                            tile_size: self.tile_size,
                            source_budget_bytes: self.source_budget,
                            stroke_budget_bytes: self.stroke_budget,
                            ..Options::default()
                        },
                    )
                    .map_err(crate::effects::paths_error)?;
                    let layer = &mut copy.layers[i];
                    // パスのチャンネルは写していないので、古い大きさの面のまま残らないよう、空の面へ置き換えてから描いた結果を入れる
                    for c in LayerPath::Canvas(fitted.clone()).channels() {
                        layer.put_surface(c, Some(Surface::new(width, height, self.tile_size)));
                    }
                    for (c, surface) in rendered.channels {
                        budget.used += surface.allocated_bytes();
                        if budget.limit.is_some_and(|limit| budget.used > limit) {
                            return Err(CoreError::SourceBudgetExceeded);
                        }
                        layer.put_surface(c, Some(surface));
                    }
                    layer.path = Some(LayerPath::Canvas(fitted));
                }
                Some(LayerPath::Surface(_)) => report.surface_path_layers.push(self.layers[i].id),
                None => {}
            }
        }
        if let Fit::Scale { scale, .. } = fit {
            report
                .notes
                .extend(Document::scale_effect_radii(&mut copy.layers, scale));
        }
        // 新しい大きさでも段の到達半径・作業メモリが上限に収まるか（収まらなければ、何も変えずに断る）
        copy.check_layer_stacks(&copy.layers)?;
        if cancelled() {
            return Err(CoreError::Cancelled);
        }
        Ok((copy, report))
    }
}

/// 大きさの変更が、画素の外の設定（効果の半径・2D のパス）をどう動かすか。
#[derive(Clone, Copy)]
enum Fit {
    /// 拡大・縮小（resize_image）: 半径・パスの点と太さを倍率に合わせる。`scale` は縦横の倍率の幾何平均（C# の Resampled）。
    Scale { sx: f64, sy: f64, scale: f64 },
    /// 画布だけを動かす（resize_canvas）: 倍率は 1。パスの点を画素と同じだけずらす。C# に対応する操作が無い、Rust 独自の決め
    /// （C# と照らしていない。試験は seam_ops の resizing_the_canvas_moves_2d_path_points_and_redraws）。
    Shift((i32, i32)),
}
impl Fit {
    /// 大きさに合わせた 2D のパス（ID・チャンネル・組はそのまま）。点は縦横の倍率かずらしで、ブラシの半径は縦横の倍率の幾何平均で
    /// 動かす（最大 4096 画素）。範囲（±1000000 画素）を出る点は中へ寄せ、変えたことを `notes` に書く。
    fn canvas_path(self, path: &CanvasPath, owner: &str, notes: &mut Vec<String>) -> CanvasPath {
        const LIMIT: f64 = 1e6;
        let mut next = path.clone();
        let mut clamped = false;
        let mut place = |p: &CanvasPoint, x: f64, y: f64| {
            let (cx, cy) = (x.clamp(-LIMIT, LIMIT), y.clamp(-LIMIT, LIMIT));
            clamped |= cx != x || cy != y;
            CanvasPoint {
                x: cx,
                y: cy,
                pressure: p.pressure,
            }
        };
        match self {
            Fit::Scale { sx, sy, scale } => {
                let wanted = path.brush.0.radius * scale;
                next.brush.0.radius = wanted.min(4096.0);
                if next.brush.0.radius != wanted {
                    notes.push(format!(
                        "「{owner}」のパス: ブラシの半径 {:.1} → 4096 画素（最大。見た目を保つには {wanted:.1} 画素）",
                        path.brush.0.radius
                    ));
                }
                next.points = path
                    .points
                    .iter()
                    .map(|p| place(p, p.x * sx, p.y * sy))
                    .collect();
            }
            Fit::Shift((dx, dy)) => {
                next.points = path
                    .points
                    .iter()
                    .map(|p| place(p, p.x + dx as f64, p.y + dy as f64))
                    .collect();
            }
        }
        if clamped {
            notes.push(format!(
                "「{owner}」のパス: キャンバスから遠く外れた点を ±1000000 画素の内へ寄せた"
            ));
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    /// 無い・一様（2 色のどちらか。隣と同じ色になりやすい）・画素ありのタイルが混ざった面。端のタイルは余白を 0 にする。
    fn mixed_surface(width: u32, height: u32, ts: u32, seed: u64) -> Surface {
        let mut state = seed;
        let mut surface = Surface::new(width, height, ts);
        let colors = [Rgba8::new(200, 40, 90, 255), Rgba8::new(10, 220, 30, 128)];
        for ty in 0..height.div_ceil(ts) {
            for tx in 0..width.div_ceil(ts) {
                let kind = next(&mut state) % 4;
                if kind == 0 {
                    continue;
                }
                let (w, h) = (ts.min(width - tx * ts), ts.min(height - ty * ts));
                let color = colors[(next(&mut state) % 2) as usize];
                let mut bytes = vec![0u8; (ts * ts * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let p = if kind == 3 {
                            let n = next(&mut state);
                            Rgba8::new(
                                n as u8,
                                (n >> 8) as u8,
                                (n >> 16) as u8,
                                (n >> 24) as u8 % 3 * 100,
                            )
                        } else {
                            color
                        };
                        let at = ((y * ts + x) * 4) as usize;
                        bytes[at..at + 4].copy_from_slice(&p.to_array());
                    }
                }
                surface.restore(TileCoord::new(tx, ty), Tile::from_bytes(&bytes).as_ref());
            }
        }
        surface
    }

    /// 画素ごとに計算した答え（飛ばす・埋めるの省略が無い）。
    fn naive(
        source: &Surface,
        width: u32,
        height: u32,
        map: &dyn Mapping,
        normal: bool,
    ) -> Vec<u8> {
        let mut reader = Reader::new(source);
        let mut out = Vec::new();
        for y in 0..height {
            for x in 0..width {
                out.extend_from_slice(&map.pixel(&mut reader, normal, x, y).to_array());
            }
        }
        out
    }

    fn fast(source: &Surface, width: u32, height: u32, map: &dyn Mapping, normal: bool) -> Vec<u8> {
        let mut budget = Budget {
            used: 0,
            limit: None,
        };
        resample_surface(
            source,
            width,
            height,
            map,
            normal,
            &mut || false,
            &mut budget,
        )
        .unwrap()
        .to_canvas_bytes()
    }

    #[test]
    fn skipping_and_filling_tiles_equal_computing_every_pixel() {
        let sizes = [
            (17, 13),
            (40, 31),
            (8, 6),
            (5, 3),
            (1, 1),
            (23, 9),
            (64, 64),
        ];
        for (case, &(sw, sh)) in sizes.iter().enumerate() {
            for ts in [1, 2, 4, 8] {
                let source = mixed_surface(sw, sh, ts, 1 + case as u64 * 31 + ts as u64);
                for &(tw, th) in &sizes {
                    for method in [
                        CanvasResampling::Nearest,
                        CanvasResampling::Bilinear,
                        CanvasResampling::Area,
                    ] {
                        let map = Resampled {
                            xs: Axis::new(sw, tw, method),
                            ys: Axis::new(sh, th, method),
                        };
                        for normal in [false, true] {
                            assert!(
                                fast(&source, tw, th, &map, normal)
                                    == naive(&source, tw, th, &map, normal),
                                "{sw}×{sh} → {tw}×{th} タイル {ts} {method:?} normal={normal}"
                            );
                        }
                    }
                }
                for &(tw, th) in &sizes {
                    for offset in [(0, 0), (3, -2), (-5, 4), (30, 20), (-40, -40), (1, 1)] {
                        let map = Shifted {
                            offset,
                            source: (sw, sh),
                        };
                        assert!(
                            fast(&source, tw, th, &map, false)
                                == naive(&source, tw, th, &map, false),
                            "{sw}×{sh} → {tw}×{th} タイル {ts} offset={offset:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_sparse_surface_renders_only_tiles_that_read_something() {
        // 1 タイルだけ描いた 8192² を半分へ: 取消の確認はタイルのまとまりごとなので、確認の回数が計算したまとまりの数になる
        let mut source = Surface::new(8192, 8192, 128);
        source.restore(
            TileCoord::new(0, 0),
            Tile::from_bytes(&[9u8; 128 * 128 * 4]).as_ref(),
        );
        let map = Resampled {
            xs: Axis::new(8192, 4096, CanvasResampling::Area),
            ys: Axis::new(8192, 4096, CanvasResampling::Area),
        };
        let mut checks = 0;
        let mut budget = Budget {
            used: 0,
            limit: None,
        };
        let out = resample_surface(
            &source,
            4096,
            4096,
            &map,
            false,
            &mut || {
                checks += 1;
                false
            },
            &mut budget,
        )
        .unwrap();
        // 面の初めの 1 回と、計算した 1 まとまりの 1 回。画素ごとに計算すると 4096 タイル ÷ (並列度 × 4) 回になる
        assert_eq!(checks, 2);
        assert_eq!(out.tile_count(), 1);
        assert_eq!(out.pixel(0, 0).unwrap(), Rgba8::from_slice(&[9; 4]));
    }
}
