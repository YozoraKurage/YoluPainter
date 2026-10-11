//! 文書の解像度変更。C# ResampleAxis / CanvasResampler と同じ整数比の重み。
//!
//! 行き先の面はタイルごとに作る。C# の CanvasResampler と同じく、読む元のタイルが 1 枚も無い行き先は飛ばし、読む元が全部同じ
//! 一様なタイルなら計算せずその色で埋める（どちらも画素ごとに計算した結果と同じバイトで、疎なレイヤーの費用が内容に比例する）。
use super::operations::Dirty;
use super::{Document, Target};
use crate::effects::LayerPath;
use crate::math::to_byte;
use crate::paths::{render_list, CanvasPath, CanvasPoint, Options, PathSymmetry};
use crate::rulers::{Ruler, RulerPlace, MAX_CANVAS_COORD, MIN_CANVAS_SEPARATION};
use crate::surface::{PixelReader, Tile};
use crate::text::TextSettings;
use crate::{
    Channel, ChannelKind, CoreError, LayerId, NormalSettings, Rgba8, Surface, SymmetryMode,
    TileCoord,
};
use glam::DVec2;
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
    /// モデルの上のパスで描かれたレイヤー。UV に結び付いたパスは残り、パスのチャンネルの画素は画素として写した（resize_image は補間、
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

/// 行き先のある範囲が読む元の範囲（両端を含む。元のキャンバスの中だけ）。
struct Span {
    lo: u32,
    hi: u32,
    /// 範囲のどの画素も元のキャンバスの中を読む（外を読む画素があると、一様な元でも外は透明なので埋められない）。
    inside: bool,
}

/// 行き先の画素が元のどこから来るか。
trait Mapping: Sync {
    /// 行き先の [lo, hi]（両端を含む）が読む元の範囲。全部が元の外なら None。
    fn span_x(&self, lo: u32, hi: u32) -> Option<Span>;
    fn span_y(&self, lo: u32, hi: u32) -> Option<Span>;
    fn pixel(&self, source: &mut PixelReader<'_>, normal: bool, x: u32, y: u32) -> Rgba8;
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
    fn pixel(&self, source: &mut PixelReader<'_>, normal: bool, x: u32, y: u32) -> Rgba8 {
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
    fn pixel(&self, source: &mut PixelReader<'_>, _normal: bool, x: u32, y: u32) -> Rgba8 {
        source.pixel(
            x as i64 - self.offset.0 as i64,
            y as i64 - self.offset.1 as i64,
        )
    }
}

fn resampled_pixel(
    source: &mut PixelReader<'_>,
    xs: &[(u32, f64)],
    ys: &[(u32, f64)],
    normal: bool,
) -> Rgba8 {
    if xs.len() == 1 && ys.len() == 1 {
        return source.pixel(xs[0].0 as i64, ys[0].0 as i64);
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
            let p = source.pixel(x as i64, y as i64);
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

/// A だけの RGBA の面（[`super::transform::selection_surface`] の逆）から選択範囲を作る。作ったばかりの面でも、裏の書き手が上限を
/// 超えた分をディスクへ逃がしていることがあるので、読めないタイルは誤りで返す（選ばれていないことにしない）。
pub(super) fn selection_from_surface(surface: &Surface) -> Result<crate::SelectionMask, CoreError> {
    let n = (surface.tile_size() * surface.tile_size()) as usize;
    let mut tiles = Vec::new();
    for coord in surface.tile_coords() {
        let Some(tile) = surface.read(coord)? else {
            continue;
        };
        let amounts: Vec<u8> = (0..n).map(|i| tile.get(i * 4).a).collect();
        if amounts.iter().any(|&a| a != 0) {
            tiles.push((coord, amounts));
        }
    }
    crate::SelectionMask::from_amount_tiles(
        surface.width(),
        surface.height(),
        surface.tile_size(),
        tiles,
    )
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
                let mut reader = PixelReader::new(source);
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
                reader.finish()?;
                Ok((coord, Tile::from_vec(bytes)))
            })
            .collect::<Result<_, CoreError>>()?;
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
                "大きさの変更の準備のあとにプロジェクトが変わった",
            ));
        }
        self.commit_resized(copy, &mut report)?;
        Ok(report)
    }
    /// 画素を再補間せずキャンバスだけを変更する。offset は元の左下を置く先。外へ出た画素は切り落とし、Undo で戻す。履歴の予算は
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
    /// 選択範囲を新しい大きさへ作り直す（A だけの RGBA の面としてレイヤーと同じ道で）。
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
        selection_from_surface(&resized)
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
            let redrawn: Vec<Channel> = match self.layers[i].path() {
                Some(p) if p.is_canvas() => crate::paths::list_channels(&self.layers[i].paths),
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
            // 選択範囲は A だけの RGBA の面としてレイヤーと同じ道を通る（飛ばす・埋める・取消）。画素の予算には数えない
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
        // 画素の外の設定を大きさに合わせる（resize_image では C# の Resampled がレイヤーごとにすること）: パス・フィルターの半径。Anchor・塗りつぶしの画像と
        // 投影・グラデーション・Generator は UV・モデルの空間・画素ごとの式で決まり、大きさによらないのでそのまま写る（レイヤーごと複製済み）
        for i in 0..self.layers.len() {
            match self.layers[i].path() {
                Some(LayerPath::Canvas(_)) => {
                    if cancelled() {
                        return Err(CoreError::Cancelled);
                    }
                    let fitted: Vec<crate::paths::LayerPathEntry> = self.layers[i]
                        .paths
                        .iter()
                        .map(|e| {
                            let LayerPath::Canvas(path) = &e.path else {
                                unreachable!("一覧はどれも同じ側")
                            };
                            crate::paths::LayerPathEntry {
                                path: LayerPath::Canvas(fit.canvas_path(
                                    path,
                                    &self.layers[i].name,
                                    &mut report.notes,
                                )),
                                ..e.clone()
                            }
                        })
                        .collect();
                    let rendered = render_list(
                        &fitted,
                        None,
                        &Options {
                            width,
                            height,
                            tile_size: self.tile_size,
                            source_budget_bytes: self.source_budget,
                            stroke_budget_bytes: self.stroke_budget,
                            images: self.effects.inputs.images.clone(),
                            ..Options::default()
                        },
                    )
                    .map_err(crate::effects::paths_error)?;
                    let layer = &mut copy.layers[i];
                    // パスのチャンネルは写していないので、古い大きさの面のまま残らないよう、空の面へ置き換えてから描いた結果を入れる
                    for c in crate::paths::list_channels(&fitted) {
                        layer.put_surface(c, Some(Surface::new(width, height, self.tile_size)));
                    }
                    for (c, surface) in rendered.channels {
                        budget.used += surface.allocated_bytes();
                        if budget.limit.is_some_and(|limit| budget.used > limit) {
                            return Err(CoreError::SourceBudgetExceeded);
                        }
                        layer.put_surface(c, Some(surface));
                    }
                    layer.paths = fitted;
                }
                Some(LayerPath::Surface(_)) => report.surface_path_layers.push(self.layers[i].id),
                None => {}
            }
            // テキストレイヤー: 画素は上で写した（ずらし・補間）。値を同じだけ動かし、次に文を直したときに同じ所・大きさで描き直す
            if let Some(text) = &self.layers[i].text {
                copy.layers[i].text = Some(fit.text(text, &self.layers[i].name, &mut report.notes));
            }
            // 定規: 2D の定規の点は画素と同じ空間なので同じだけ動かす（3D の定規はモデルの空間なので動かさない）
            if self.layers[i]
                .rulers
                .iter()
                .any(|r| matches!(r.place, RulerPlace::Canvas { .. }))
            {
                copy.layers[i].rulers = fit.rulers(
                    &self.layers[i].rulers,
                    &self.layers[i].name,
                    &mut report.notes,
                );
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

/// 2D の定規の 2 点（動かしたあとの値）を、確かめ（[`Ruler::validate`]）に通る位置へ直す。通るものは変えない。範囲（±1e7）の外の点は中へ寄せ、
/// それでも 2 点が 0.01 画素より近ければ、`a` を動かさず `b` を最小の間隔（少し余裕を見て）離す。離す向きは元の向き、その逆、範囲の中へ向かう
/// 斜めの順に試し、範囲の中の最初のものを取る（斜めはどの角でも範囲の中に収まる）。返すのは 2 点と、直したか。
fn fit_canvas_points(ma: DVec2, mb: DVec2) -> (DVec2, DVec2, bool) {
    let limit = DVec2::splat(MAX_CANVAS_COORD);
    let inside = |p: DVec2| p.is_finite() && p.abs().max_element() <= MAX_CANVAS_COORD;
    let fits =
        |a: DVec2, b: DVec2| inside(a) && inside(b) && a.distance(b) >= MIN_CANVAS_SEPARATION;
    if fits(ma, mb) {
        return (ma, mb, false);
    }
    // 有限でない値は来ない（倍率と座標は有限）が、来ても確かめに通る位置を返す
    let pull = |p: DVec2| {
        if p.is_finite() {
            p.clamp(-limit, limit)
        } else {
            DVec2::ZERO
        }
    };
    let (na, nb) = (pull(ma), pull(mb));
    if fits(na, nb) {
        return (na, nb, true);
    }
    let step = MIN_CANVAS_SEPARATION * 1.01;
    let along = (mb - ma).try_normalize().unwrap_or(DVec2::X);
    let toward_inside = DVec2::new(
        if na.x >= 0.0 { -1.0 } else { 1.0 },
        if na.y >= 0.0 { -1.0 } else { 1.0 },
    ) * std::f64::consts::FRAC_1_SQRT_2;
    for dir in [along, -along, toward_inside] {
        let moved = na + dir * step;
        if fits(na, moved) {
            return (na, moved, true);
        }
    }
    // ここへは来ない（範囲の中へ向かう斜めは、範囲の中の点からなら必ず収まる）。来ても確かめに通る位置を返す
    (DVec2::ZERO, DVec2::X * step, true)
}

/// 大きさの変更が、画素の外の設定（効果の半径・2D のパス）をどう動かすか。
#[derive(Clone, Copy)]
enum Fit {
    /// 拡大・縮小（resize_image）: 半径・パスの点と太さを倍率に合わせる。`scale` は縦横の倍率の幾何平均（C# の Resampled）。
    Scale { sx: f64, sy: f64, scale: f64 },
    /// キャンバスだけを動かす（resize_canvas）: 倍率は 1。パスの点を画素と同じだけずらす。C# に対応する操作が無い、Rust 独自の決め
    /// （C# と照らしていない。試験は seam_ops の resizing_the_canvas_moves_2d_path_points_and_redraws）。
    Shift((i32, i32)),
}
impl Fit {
    /// 大きさに合わせた 2D のパス（ID・チャンネル・組はそのまま）。点とキャンバスの対称の中心は縦横の倍率かずらしで、ブラシの半径は縦横の
    /// 倍率の幾何平均で動かす（最大 4096 画素）。範囲（±1000000 画素）を出る点は中へ寄せ、変えたことを `notes` に書く。
    fn canvas_path(self, path: &CanvasPath, owner: &str, notes: &mut Vec<String>) -> CanvasPath {
        const LIMIT: f64 = 1e6;
        let mut next = path.clone();
        let mut clamped = false;
        // 取っ手は点からの向きなので、拡大・縮小では縦横の倍率を掛け、ずらしでは変えない
        let (hx, hy) = match self {
            Fit::Scale { sx, sy, .. } => (sx, sy),
            Fit::Shift(_) => (1.0, 1.0),
        };
        let mut place = |p: &CanvasPoint, x: f64, y: f64| {
            let (cx, cy) = (x.clamp(-LIMIT, LIMIT), y.clamp(-LIMIT, LIMIT));
            clamped |= cx != x || cy != y;
            let handle = |h: glam::DVec2| {
                glam::DVec2::new(
                    (h.x * hx).clamp(-crate::paths::MAX_HANDLE, crate::paths::MAX_HANDLE),
                    (h.y * hy).clamp(-crate::paths::MAX_HANDLE, crate::paths::MAX_HANDLE),
                )
            };
            CanvasPoint {
                x: cx,
                y: cy,
                pressure: p.pressure,
                tangent: match p.tangent {
                    crate::paths::Tangent::Handles { incoming, outgoing } => {
                        crate::paths::Tangent::Handles {
                            incoming: handle(incoming),
                            outgoing: handle(outgoing),
                        }
                    }
                    other => other,
                },
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
        // キャンバスの対称の中心はキャンバスの画素の座標なので、点と同じに動かす（動かさないと、映した側が古い中心で映されて違う所に描かれる）
        if let PathSymmetry::Canvas(symmetry) = &mut next.style.symmetry {
            let (x, y) = match self {
                Fit::Scale { sx, sy, .. } => (symmetry.center.x * sx, symmetry.center.y * sy),
                Fit::Shift((dx, dy)) => {
                    (symmetry.center.x + dx as f64, symmetry.center.y + dy as f64)
                }
            };
            let (cx, cy) = (x.clamp(-LIMIT, LIMIT), y.clamp(-LIMIT, LIMIT));
            clamped |= cx != x || cy != y;
            symmetry.center = glam::DVec2::new(cx, cy);
            // 線対称の最初の軸の向きは、縦横の倍率が違えば変わる（向きのベクトルを倍率で写す）
            if let (SymmetryMode::Lines, Fit::Scale { sx, sy, .. }) = (symmetry.mode, self) {
                if sx != sy {
                    let t = symmetry.angle.to_radians();
                    symmetry.angle = crate::rulers::direction_degrees(glam::DVec2::new(
                        sx * t.cos(),
                        sy * t.sin(),
                    ));
                }
            }
        }
        if clamped {
            notes.push(format!(
                "「{owner}」のパス: キャンバスから遠く外れた点を ±1000000 画素の内へ寄せた"
            ));
        }
        next
    }

    /// 大きさに合わせた定規（2D の定規だけ。3D の定規は変えない）。点を縦横の倍率かずらしで動かす。範囲（±1e7）の外へ出た点や、近づきすぎた
    /// 2 点は、確かめに通る位置へ直し（[`fit_canvas_points`]）、直した定規の数をレイヤーごとに 1 つの知らせにまとめる。確かめに通らない値は残さない。
    fn rulers(self, list: &[Ruler], owner: &str, notes: &mut Vec<String>) -> Vec<Ruler> {
        let map = |p: DVec2| match self {
            Fit::Scale { sx, sy, .. } => DVec2::new(p.x * sx, p.y * sy),
            Fit::Shift((dx, dy)) => DVec2::new(p.x + dx as f64, p.y + dy as f64),
        };
        let mut adjusted = 0usize;
        let out = list
            .iter()
            .map(|r| {
                let RulerPlace::Canvas { a, b } = r.place else {
                    return r.clone();
                };
                let (na, nb, changed) = fit_canvas_points(map(a), map(b));
                let fitted = Ruler {
                    place: RulerPlace::Canvas { a: na, b: nb },
                    ..r.clone()
                };
                // 最後の守り: 確かめに通らなければ、動かす前の定規を残す（通らない値を文書に入れない）
                if fitted.validate().is_err() {
                    adjusted += 1;
                    return r.clone();
                }
                adjusted += usize::from(changed);
                fitted
            })
            .collect();
        if adjusted > 0 {
            notes.push(format!(
                "「{owner}」の定規 {adjusted} 個: 範囲の外へ出た点を寄せた、または近づきすぎた 2 点の間隔を保った"
            ));
        }
        out
    }

    /// 大きさに合わせたテキストの値。ずらしは基準の点だけを動かす（画素も同じだけずれるので、描き直しても同じ所）。拡大・縮小は
    /// 基準の点を縦横の倍率で、大きさを幾何平均で、折り返しの幅を横の倍率で動かす。このとき画素は補間で写しただけなので、
    /// `notes` に書く（文を直すと新しい大きさで描き直す。縦横の倍率が違えば、描き直した形は写した画素と同じにならない）。
    fn text(self, text: &TextSettings, owner: &str, notes: &mut Vec<String>) -> TextSettings {
        let limit = crate::text::MAX_COORDINATE;
        let mut next = text.clone();
        match self {
            Fit::Scale { sx, sy, scale } => {
                next.x = (text.x * sx).clamp(-limit, limit);
                next.y = (text.y * sy).clamp(-limit, limit);
                next.size = (text.size * scale).clamp(crate::text::MIN_SIZE, crate::text::MAX_SIZE);
                next.wrap_width = (text.wrap_width * sx).clamp(0.0, limit);
                notes.push(format!(
                    "「{owner}」のテキストは画素を拡大・縮小しました（文を直すと新しいサイズで描き直す）"
                ));
            }
            Fit::Shift((dx, dy)) => {
                next.x = (text.x + dx as f64).clamp(-limit, limit);
                next.y = (text.y + dy as f64).clamp(-limit, limit);
            }
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
        let mut reader = PixelReader::new(source);
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

    /// 動かした 2 点がどこにあっても（範囲の外・角・同じ点・近すぎる）、直した 2 点は定規の確かめに通る。通る入力は変えない。
    #[test]
    fn fitted_ruler_points_always_pass_the_ruler_check() {
        use crate::rulers::{RulerId, RulerKind};
        let check = |ma: DVec2, mb: DVec2| {
            let (a, b, changed) = fit_canvas_points(ma, mb);
            let r = Ruler::canvas(RulerId(1), RulerKind::Line, a, b);
            assert!(r.validate().is_ok(), "{ma:?} {mb:?} -> {a:?} {b:?}");
            let passes = Ruler::canvas(RulerId(1), RulerKind::Line, ma, mb)
                .validate()
                .is_ok();
            assert_eq!(changed, !passes, "{ma:?} {mb:?}");
            if passes {
                assert_eq!((a, b), (ma, mb), "通る入力は変えない");
            }
        };
        let values = [
            0.0,
            1.0,
            -12.3456,
            9.0e6,
            9.5e6,
            9_999_999.995,
            1e7,
            -1e7,
            1.8e7,
            -1.9e7,
            3e7,
        ];
        for ax in values {
            for ay in values {
                for bx in values {
                    for by in values {
                        check(DVec2::new(ax, ay), DVec2::new(bx, by));
                    }
                }
            }
        }
        // 角のまわりの細かい点（向きも距離も変えながら）
        let mut state = 7u64;
        for _ in 0..20_000 {
            let mut coord = || {
                let sign = if next(&mut state).is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                let near = (next(&mut state) % 1_000_000) as f64 / 1e4;
                sign * (1e7 + near - 50.0)
            };
            let (ax, ay, bx, by) = (coord(), coord(), coord(), coord());
            check(DVec2::new(ax, ay), DVec2::new(bx, by));
        }
    }

    /// 斜めに外へ出る定規（拡大で角の外へ）を、サイズ変更で動かしたあとも、レイヤーの定規の一覧が確かめに通る。知らせはレイヤーごとに 1 つ。
    #[test]
    fn a_ruler_pushed_past_a_corner_by_a_resize_stays_valid_with_one_note_per_layer() {
        use crate::rulers::{RulerId, RulerKind};
        let rulers: Vec<Ruler> = [
            ((9e6, 9e6), (9.5e6, 8e6)),
            ((9e6, -9e6), (8e6, -9.5e6)),
            ((-9e6, 9e6), (-9.5e6, 9.9e6)),
            ((-9.9e6, -9.9e6), (-9.9e6, -9.9e6 + 0.5)),
            ((1.0, 1.0), (50.0, 30.0)),
        ]
        .into_iter()
        .enumerate()
        .map(|(k, (a, b))| {
            Ruler::canvas(
                RulerId(k as u128 + 1),
                RulerKind::Line,
                DVec2::new(a.0, a.1),
                DVec2::new(b.0, b.1),
            )
        })
        .collect();
        for fit in [
            Fit::Scale {
                sx: 2.0,
                sy: 2.0,
                scale: 2.0,
            },
            Fit::Scale {
                sx: 3.0,
                sy: 0.5,
                scale: 1.2,
            },
            Fit::Shift((2_000_000, -3_000_000)),
        ] {
            let mut notes = Vec::new();
            let out = fit.rulers(&rulers, "レイヤー", &mut notes);
            assert_eq!(out.len(), rulers.len());
            for (r, before) in out.iter().zip(&rulers) {
                assert!(r.validate().is_ok(), "{r:?}");
                // 守りで動かす前の定規へ戻したのではなく、点を写して直してある
                assert_ne!(r.place, before.place, "{r:?}");
            }
            assert_eq!(notes.len(), 1, "レイヤーごとに 1 つ: {notes:?}");
            // 範囲の中に残る定規は、点だけが写る
            assert_eq!(out[4].id, rulers[4].id);
        }
        // 直す物が無ければ、知らせは出ない
        let mut notes = Vec::new();
        let _ = Fit::Shift((1, 1)).rulers(&rulers[4..], "レイヤー", &mut notes);
        assert!(notes.is_empty());
    }

    /// 範囲の外の点は、その側の端（±1e7）へ寄る（負の側は −1e7）。範囲の中の点と、もう一方の点は動かさない。
    #[test]
    fn a_point_past_the_range_is_pulled_to_the_nearest_edge_on_its_own_side() {
        let edge = MAX_CANVAS_COORD;
        let b = DVec2::new(5.0, -7.0);
        for (outside, pulled) in [
            (DVec2::new(-2.0 * edge, 3.0), DVec2::new(-edge, 3.0)),
            (DVec2::new(4.0, -3.0 * edge), DVec2::new(4.0, -edge)),
            (DVec2::new(2.0 * edge, 3.0), DVec2::new(edge, 3.0)),
            (DVec2::new(-2.0 * edge, 3.0 * edge), DVec2::new(-edge, edge)),
        ] {
            assert_eq!(fit_canvas_points(outside, b), (pulled, b, true));
            assert_eq!(fit_canvas_points(b, outside), (b, pulled, true));
        }
    }

    /// 近づきすぎた 2 点は、`a` を動かさず、`b` を元の向き（`a`→`b`）に最小の間隔（少し余裕を見て）まで離す。
    #[test]
    fn two_points_too_close_are_separated_along_their_own_direction() {
        let step = MIN_CANVAS_SEPARATION * 1.01;
        let a = DVec2::new(5.0, 3.0);
        for dir in [DVec2::Y, -DVec2::Y, DVec2::X, -DVec2::X] {
            let (na, nb, changed) = fit_canvas_points(a, a + dir * 0.001);
            assert_eq!(na, a);
            assert!(changed);
            assert!((nb - (a + dir * step)).length() < 1e-12, "{dir:?}: {nb:?}");
        }
    }

    /// 元の向きへ離すと範囲の外へ出る（`a` が端にある）ときは、その逆の向きへ離す。
    #[test]
    fn two_points_too_close_at_the_edge_are_separated_the_other_way_back_into_the_range() {
        let edge = MAX_CANVAS_COORD;
        let step = MIN_CANVAS_SEPARATION * 1.01;
        for (a, outward) in [
            (DVec2::new(edge, 5.0), DVec2::X),
            (DVec2::new(-edge, 5.0), -DVec2::X),
            (DVec2::new(5.0, edge), DVec2::Y),
            (DVec2::new(5.0, -edge), -DVec2::Y),
        ] {
            let (na, nb, changed) = fit_canvas_points(a, a + outward * 0.001);
            assert!(changed);
            assert_eq!(na, a);
            assert!(
                (nb - (a - outward * step)).length() < 1e-9,
                "外へ向かう向きの逆へ離す（{outward:?}）: {nb:?}"
            );
        }
    }

    /// 2 点とも範囲の外で、同じ角へ寄ってしまい、元の向きもその逆も範囲の外へ出るときは、範囲の内へ向かう斜めに離す（`a` は角のまま）。
    #[test]
    fn two_points_pulled_onto_the_same_corner_are_separated_diagonally_into_the_range() {
        let edge = MAX_CANVAS_COORD;
        let step = MIN_CANVAS_SEPARATION * 1.01;
        let inward = std::f64::consts::FRAC_1_SQRT_2 * step;
        for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let corner = DVec2::new(sx * edge, sy * edge);
            // a から b への向きは、x では外へ、y では内へ（その逆は x で内、y で外）。どちらの向きでも角の外へ出る
            let a = corner * 2.0;
            let b = a + DVec2::new(sx * 0.5 * edge, -sy * edge);
            let (na, nb, changed) = fit_canvas_points(a, b);
            assert!(changed);
            assert_eq!(na, corner, "a は角のまま（{sx} {sy}）");
            assert!(
                (nb - (corner - DVec2::new(sx, sy) * inward)).length() < 1e-9,
                "範囲の内へ向かう斜めに離す（{sx} {sy}）: {nb:?}"
            );
        }
    }

    /// 確かめに通らない定規（ID が 0 など）は、動かす前のまま残り、直した数に数えられて知らせの文に入る。
    #[test]
    fn a_ruler_that_cannot_pass_the_check_stays_as_it_was_and_is_counted_in_the_note() {
        use crate::rulers::{RulerId, RulerKind};
        let broken = Ruler::canvas(RulerId(0), RulerKind::Line, DVec2::ZERO, DVec2::X * 10.0);
        let fine = Ruler::canvas(RulerId(2), RulerKind::Line, DVec2::ZERO, DVec2::X * 10.0);
        let mut notes = Vec::new();
        let out = Fit::Shift((1, 1)).rulers(&[broken.clone(), fine], "レイヤー", &mut notes);
        assert_eq!(out[0], broken, "動かす前のまま");
        assert_eq!(
            out[1].place,
            RulerPlace::Canvas {
                a: DVec2::ONE,
                b: DVec2::new(11.0, 1.0)
            }
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("定規 1 個"), "{notes:?}");

        // 範囲の外へ出た定規 2 つ（寄せて直す）と通らない定規 1 つ: 3 個
        let past = |id: u128| {
            Ruler::canvas(
                RulerId(id),
                RulerKind::Line,
                DVec2::new(9e6, 9e6),
                DVec2::new(9.5e6, 8e6),
            )
        };
        let mut notes = Vec::new();
        let _ =
            Fit::Shift((2_000_000, 0)).rulers(&[past(3), broken, past(4)], "レイヤー", &mut notes);
        assert!(notes[0].contains("定規 3 個"), "{notes:?}");
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
