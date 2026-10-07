use super::*;
use crate::geometry::SurfaceBrushBudget;
use crate::{Brush, Document, LayerId, Stroke, Surface};
use glam::DVec2;
use std::sync::atomic::{AtomicBool, Ordering};

/// 予算は組全体。文書から独立しているため、失敗時には作りかけを返さない。
pub struct Options<'a> {
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub source_budget_bytes: u64,
    pub stroke_budget_bytes: u64,
    pub surface_budget: SurfaceBrushBudget,
    pub cancel: Option<&'a AtomicBool>,
    /// リボンの画像（アセットの画像。画素は共有）。リボンのパスが無ければ空でよい。
    pub images: std::collections::HashMap<crate::ImageId, crate::effects::ImageInput>,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            width: 256,
            height: 256,
            tile_size: 128,
            source_budget_bytes: 256 << 20,
            stroke_budget_bytes: 64 << 20,
            surface_budget: Default::default(),
            cancel: None,
            images: Default::default(),
        }
    }
}
impl Options<'_> {
    #[cfg_attr(test, track_caller)]
    pub(super) fn check(&self) -> Result<(), Error> {
        #[cfg(test)]
        super::cancellation_tests::checkpoint(self.cancel, std::panic::Location::caller());
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Canceled)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug)]
pub struct Rendered {
    pub channels: Vec<(Channel, Surface)>,
    pub samples: usize,
    pub dabs: usize,
    pub gaps: usize,
}
/// チャンネル 1 つの作業面（一覧のパスが順に描く。文書は試しの外へ出さない）。
struct Canvas {
    doc: Document,
    layer: LayerId,
    channel: Channel,
}
/// 一覧のパスを描く作業面の組（チャンネルごと）と、描いているパスのストローク（そのパスの組のチャンネルごと）。
pub(super) struct Painter<'a, 'b> {
    canvases: Vec<Canvas>,
    active: Vec<(usize, Stroke)>,
    options: &'a Options<'b>,
}
impl<'a, 'b> Painter<'a, 'b> {
    pub fn new(options: &'a Options<'b>) -> Result<Self, Error> {
        options.check()?;
        Ok(Self {
            canvases: Vec::new(),
            active: Vec::new(),
            options,
        })
    }
    /// チャンネルの作業面（無ければ作る）の番号。
    fn canvas(&mut self, channel: Channel) -> Result<usize, Error> {
        if let Some(i) = self.canvases.iter().position(|c| c.channel == channel) {
            return Ok(i);
        }
        let o = self.options;
        let mut doc = Document::with_tile_size(o.width, o.height, o.tile_size)?;
        doc.set_source_budget_bytes(o.source_budget_bytes)?;
        doc.set_stroke_budget_bytes(o.stroke_budget_bytes)?;
        doc.set_undo_budget_bytes(0)?;
        doc.set_minimum_undo_steps(0)?;
        let layer = doc.add_layer("path")?;
        doc.set_channel_enabled(layer, channel, true)?;
        self.canvases.push(Canvas {
            doc,
            layer,
            channel,
        });
        Ok(self.canvases.len() - 1)
    }
    /// 画布の大きさ。
    pub fn size(&self) -> (u32, u32) {
        (self.options.width, self.options.height)
    }
    /// 予算・取消・画像の設定。
    pub fn options(&self) -> &Options<'b> {
        self.options
    }
    /// 描かないチャンネルにも作業面を作る（結果は空の面）。
    pub fn ensure_channels(&mut self, channels: &[Channel]) -> Result<(), Error> {
        for c in channels {
            self.canvas(*c)?;
        }
        Ok(())
    }
    /// 1 本のパスのストロークを、組のチャンネルごとに始める（色は組の色）。
    pub fn begin(&mut self, paints: Vec<ChannelPaint>, brush: &Brush) -> Result<(), Error> {
        debug_assert!(self.active.is_empty(), "前のパスを終えてから");
        for p in paints {
            let i = self.canvas(p.channel)?;
            let c = &mut self.canvases[i];
            let stroke = c.doc.begin_brush_stroke_in(
                c.layer,
                p.channel,
                &Brush {
                    base: BrushSettings {
                        color: p.color,
                        ..brush.base
                    },
                    ..brush.clone()
                },
            )?;
            self.active.push((i, stroke));
        }
        Ok(())
    }
    fn apply(
        &mut self,
        mut f: impl FnMut(&mut Stroke, &mut Document) -> Result<(), CoreError>,
    ) -> Result<(), Error> {
        self.options.check()?;
        for (i, s) in &mut self.active {
            f(s, &mut self.canvases[*i].doc)?;
        }
        if self
            .canvases
            .iter()
            .map(|c| c.doc.allocated_bytes())
            .sum::<u64>()
            > self.options.source_budget_bytes
        {
            return Err(Error::Core(CoreError::SourceBudgetExceeded));
        }
        if self
            .active
            .iter()
            .map(|(i, _)| {
                self.canvases[*i]
                    .doc
                    .active_stroke_stats()
                    .unwrap()
                    .rollback_bytes
            })
            .sum::<u64>()
            > self.options.stroke_budget_bytes
        {
            return Err(Error::Core(CoreError::StrokeBudgetExceeded));
        }
        Ok(())
    }
    pub fn point(&mut self, x: f64, y: f64, pressure: f64) -> Result<(), Error> {
        self.apply(|s, d| s.add_point(d, x, y, pressure, DVec2::ZERO))
    }
    pub fn pixel(&mut self, x: i32, y: i32, coverage: f64, pressure: f64) -> Result<(), Error> {
        self.apply(|s, d| {
            s.apply_pixel(d, x as i64, y as i64, coverage, pressure)
                .map(|_| ())
        })
    }
    /// 面のダブを丸ごと塗る（3D の指先: 読み元を凍結してから、中心 `center`（画布の画素）の動きで引きずる）。
    pub fn dab(
        &mut self,
        pixels: &[crate::BrushPixel],
        center: DVec2,
        pressure: f64,
    ) -> Result<(), Error> {
        self.apply(|s, d| s.apply_dab(d, pixels, center, pressure).map(|_| ()))
    }
    /// 指先の前の位置を忘れる（3D で UV の継ぎ目をまたぐとき）。
    pub fn reset_direction(&mut self) -> Result<(), Error> {
        self.apply(|s, d| s.reset_effect_direction(d))
    }
    /// 画素ごとの色の画素を、チャンネルの作業面へ「通常」で重ねる（リボン）。`color` はその画素の色（色のチャンネルに使う）、
    /// `alpha` は覆い（0〜1）。色の種類のチャンネルは画素の色、ほかのチャンネルは組の値を、覆い × 組の値のアルファで重ねる。
    pub fn blend(
        &mut self,
        paints: &[ChannelPaint],
        pixels: &[(i32, i32, crate::Rgba8, f64)],
    ) -> Result<(), Error> {
        for p in paints {
            self.options.check()?;
            let i = self.canvas(p.channel)?;
            let color_kind = crate::ChannelInfo::standard(p.channel).map(|c| c.kind)
                == Some(crate::ChannelKind::Color);
            let c = &mut self.canvases[i];
            for &(x, y, color, alpha) in pixels {
                if x < 0
                    || y < 0
                    || x as u32 >= self.options.width
                    || y as u32 >= self.options.height
                {
                    continue;
                }
                let (src, a) = if color_kind {
                    (color, alpha)
                } else {
                    (p.color, alpha * p.color.a as f64 / 255.0)
                };
                c.doc
                    .paths_blend_pixel(c.layer, c.channel, x as u32, y as u32, src, a)?;
            }
            if c.doc.allocated_bytes() > self.options.source_budget_bytes {
                return Err(Error::Core(CoreError::SourceBudgetExceeded));
            }
        }
        Ok(())
    }
    /// 描いているパスのストロークを確定する。
    pub fn end(&mut self) -> Result<(), Error> {
        for (i, s) in std::mem::take(&mut self.active) {
            self.canvases[i].doc.end_stroke(s)?;
        }
        Ok(())
    }
    pub fn finish(mut self, samples: usize, dabs: usize, gaps: usize) -> Result<Rendered, Error> {
        self.options.check()?;
        self.end()?;
        let mut channels = Vec::new();
        for c in self.canvases {
            channels.push((
                c.channel,
                c.doc
                    .layer(c.layer)
                    .unwrap()
                    .surface(c.channel)
                    .cloned()
                    .unwrap_or_else(|| {
                        Surface::new(
                            self.options.width,
                            self.options.height,
                            self.options.tile_size,
                        )
                    }),
            ));
        }
        self.options.check()?;
        Ok(Rendered {
            channels,
            samples,
            dabs,
            gaps,
        })
    }
}
pub fn render_canvas(path: &CanvasPath, options: &Options<'_>) -> Result<Rendered, Error> {
    path.validate()?;
    options.check()?;
    let mut painter = Painter::new(options)?;
    let samples = draw_canvas(&mut painter, path)?;
    painter.finish(samples, 0, 0)
}
/// 種類に合わせたストロークのブラシ（消しゴムは `erase`、指先は指先の効果）と、筆先の画像・角度・向き。
pub(super) fn kind_brush(base: BrushSettings, style: &PathStyle) -> Brush {
    let mut brush = Brush::from(base);
    brush.tip.image = style.tip.clone();
    brush.tip.angle = style.angle;
    brush.tip.follow_direction = style.follow;
    match style.kind {
        PathKind::Erase => brush.base.erase = true,
        PathKind::Smudge { strength } => {
            brush.base.erase = false;
            brush.effect = crate::BrushEffect::Smudge { strength };
        }
        PathKind::Stroke | PathKind::Fill | PathKind::Ribbon(_) => {}
    }
    brush
}

/// 2D のパスを作業面へ描く（ストロークを始めて終える）。対称なら、映したパスも続けて描く。サンプルの数を返す。
pub(super) fn draw_canvas(
    painter: &mut Painter<'_, '_>,
    path: &CanvasPath,
) -> Result<usize, Error> {
    let mut samples = draw_canvas_one(painter, path)?;
    if let PathSymmetry::Canvas(sym) = path.style.symmetry {
        let transforms = sym
            .transforms()
            .map_err(|_| Error::Invalid("パスの対称の種類・中心・数が範囲外です"))?;
        for t in transforms.iter().skip(1) {
            if let Some(copy) = mirrored_canvas(path, t) {
                samples += draw_canvas_one(painter, &copy)?;
            }
        }
    }
    Ok(samples)
}

/// 対称の写し 1 つのパス（点と取っ手を写す。範囲の外へ出る写しは None）。
fn mirrored_canvas(path: &CanvasPath, t: &crate::SymmetryTransform) -> Option<CanvasPath> {
    let mut points = Vec::with_capacity(path.points.len());
    for p in &path.points {
        let (x, y) = t.map(p.x, p.y);
        let handle = |h: DVec2| {
            let (hx, hy) = t.map(p.x + h.x, p.y + h.y);
            DVec2::new(hx - x, hy - y)
        };
        let tangent = match p.tangent {
            Tangent::Handles { incoming, outgoing } => Tangent::Handles {
                incoming: handle(incoming),
                outgoing: handle(outgoing),
            },
            other => other,
        };
        let q = CanvasPoint::new(x, y, p.pressure)
            .ok()?
            .with_tangent(tangent);
        q.validate().ok()?;
        points.push(q);
    }
    Some(CanvasPath {
        points,
        style: PathStyle {
            symmetry: PathSymmetry::None,
            ..path.style.clone()
        },
        ..path.clone()
    })
}

fn draw_canvas_one(painter: &mut Painter<'_, '_>, path: &CanvasPath) -> Result<usize, Error> {
    match path.style.kind {
        PathKind::Fill => return super::fill::draw_canvas(painter, path),
        PathKind::Ribbon(r) => return super::ribbon::draw_canvas(painter, path, r),
        PathKind::Stroke | PathKind::Smudge { .. } | PathKind::Erase => {}
    }
    painter.begin(
        paints(path.channel, path.brush, &path.material)?,
        &kind_brush(path.brush.0, &path.style),
    )?;
    let step = (path.brush.0.radius * 2.0 * path.brush.0.spacing).max(0.01) / 4.0;
    let samples = canvas_curve(path, step, &mut |x, y, pressure| {
        painter.point(x, y, pressure)
    })?;
    painter.end()?;
    Ok(samples)
}

/// 2D のパスの曲線の標本を順に渡す（最初の点、続けて区間ごとに `step` ほどの間隔の標本。x・y・筆圧）。標本の数を返す。
pub(super) fn canvas_curve(
    path: &CanvasPath,
    step: f64,
    emit: &mut dyn FnMut(f64, f64, f64) -> Result<(), Error>,
) -> Result<usize, Error> {
    let ps = &path.points;
    let mut samples = 0;
    if let Some(p) = ps.first() {
        emit(p.x, p.y, p.pressure)?;
    }
    for s in 0..ps.len().saturating_sub(1) {
        let p = [
            ps[s.saturating_sub(1)],
            ps[s],
            ps[s + 1],
            ps[(s + 2).min(ps.len() - 1)],
        ];
        // 両端が滑らかなら今までの Catmull–Rom（同じバイト）、角・取っ手があればベジェ（標本の数は制御多角形の長さで）
        let bezier = super::bezier::canvas_controls(p);
        let length = match bezier {
            None => ((p[2].x - p[1].x).powi(2) + (p[2].y - p[1].y).powi(2)).sqrt(),
            Some(c) => super::bezier::length2(c),
        };
        let count = (length / step).ceil().max(1.0);
        if count > MAX_CANVAS_SAMPLES as f64 || samples + count as usize > MAX_CANVAS_SAMPLES {
            return Err(Error::TooManySamples);
        }
        samples += count as usize;
        for k in 1..=count as usize {
            let t = k as f64 / count;
            let (x, y) = match bezier {
                None => curve(p, t),
                Some(c) => {
                    let q = super::bezier::eval2(c, t);
                    (q.x, q.y)
                }
            };
            emit(x, y, p[1].pressure + (p[2].pressure - p[1].pressure) * t)?;
        }
    }
    Ok(samples + usize::from(!ps.is_empty()))
}
#[cfg(test)]
pub(super) fn curve_for_test(p: [CanvasPoint; 4], t: f64) -> (f64, f64) {
    curve(p, t)
}

fn curve(p: [CanvasPoint; 4], t: f64) -> (f64, f64) {
    let knot = |a: CanvasPoint, b: CanvasPoint| {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2))
            .sqrt()
            .sqrt()
            .max(1e-6)
    };
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let u = t1 + (t2 - t1) * t;
    let f = |v: [f64; 4]| {
        let l =
            |a: f64, b: f64, ta: f64, tb: f64| (tb - u) / (tb - ta) * a + (u - ta) / (tb - ta) * b;
        let a1 = l(v[0], v[1], 0.0, t1);
        let a2 = l(v[1], v[2], t1, t2);
        let a3 = l(v[2], v[3], t2, t3);
        l(l(a1, a2, 0.0, t2), l(a2, a3, t1, t3), t1, t2)
    };
    (f(p.map(|p| p.x)), f(p.map(|p| p.y)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_after_a_painted_sample_discards_the_result() {
        let cancel = AtomicBool::new(false);
        let o = Options {
            width: 64,
            height: 64,
            tile_size: 16,
            cancel: Some(&cancel),
            ..Options::default()
        };
        let b = BrushSettings::default();
        let mut painter = Painter::new(&o).unwrap();
        painter
            .begin(
                vec![ChannelPaint {
                    channel: Channel::Color,
                    color: b.color,
                }],
                &Brush::from(b),
            )
            .unwrap();
        painter.point(24.0, 24.0, 1.0).unwrap();
        assert!(painter.canvases[0].doc.allocated_bytes() > 0);
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(painter.finish(1, 0, 0).unwrap_err(), Error::Canceled);
    }
}
