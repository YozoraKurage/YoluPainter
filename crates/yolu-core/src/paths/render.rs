use super::*;
use crate::geometry::SurfaceBrushBudget;
use crate::{Document, LayerId, Stroke, Surface};
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
struct Part {
    doc: Document,
    stroke: Stroke,
    layer: LayerId,
    channel: Channel,
}
pub(super) struct Painter<'a, 'b> {
    parts: Vec<Part>,
    options: &'a Options<'b>,
}
impl<'a, 'b> Painter<'a, 'b> {
    pub fn new(
        paints: Vec<ChannelPaint>,
        brush: BrushSettings,
        options: &'a Options<'b>,
    ) -> Result<Self, Error> {
        options.check()?;
        let mut parts = Vec::new();
        for p in paints {
            let mut doc =
                Document::with_tile_size(options.width, options.height, options.tile_size)?;
            doc.set_source_budget_bytes(options.source_budget_bytes)?;
            doc.set_stroke_budget_bytes(options.stroke_budget_bytes)?;
            doc.set_undo_budget_bytes(0)?;
            doc.set_minimum_undo_steps(0)?;
            let layer = doc.add_layer("path")?;
            doc.set_channel_enabled(layer, p.channel, true)?;
            let stroke = doc.begin_stroke_in(
                layer,
                p.channel,
                &BrushSettings {
                    color: p.color,
                    ..brush
                },
            )?;
            parts.push(Part {
                doc,
                stroke,
                layer,
                channel: p.channel,
            });
        }
        Ok(Self { parts, options })
    }
    fn apply(
        &mut self,
        mut f: impl FnMut(&mut Stroke, &mut Document) -> Result<(), CoreError>,
    ) -> Result<(), Error> {
        self.options.check()?;
        for p in &mut self.parts {
            f(&mut p.stroke, &mut p.doc)?;
        }
        if self
            .parts
            .iter()
            .map(|p| p.doc.allocated_bytes())
            .sum::<u64>()
            > self.options.source_budget_bytes
        {
            return Err(Error::Core(CoreError::SourceBudgetExceeded));
        }
        if self
            .parts
            .iter()
            .map(|p| p.doc.active_stroke_stats().unwrap().rollback_bytes)
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
    pub fn finish(self, samples: usize, dabs: usize, gaps: usize) -> Result<Rendered, Error> {
        self.options.check()?;
        let mut channels = Vec::new();
        for mut p in self.parts {
            p.doc.end_stroke(p.stroke)?;
            channels.push((
                p.channel,
                p.doc
                    .layer(p.layer)
                    .unwrap()
                    .surface(p.channel)
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
    let mut painter = Painter::new(
        paints(path.channel, path.brush, &path.material)?,
        path.brush.0,
        options,
    )?;
    let ps = &path.points;
    let mut samples = 0;
    if let Some(p) = ps.first() {
        painter.point(p.x, p.y, p.pressure)?;
    }
    let step = (path.brush.0.radius * 2.0 * path.brush.0.spacing).max(0.01) / 4.0;
    for s in 0..ps.len().saturating_sub(1) {
        let p = [
            ps[s.saturating_sub(1)],
            ps[s],
            ps[s + 1],
            ps[(s + 2).min(ps.len() - 1)],
        ];
        let chord = ((p[2].x - p[1].x).powi(2) + (p[2].y - p[1].y).powi(2)).sqrt();
        let count = (chord / step).ceil().max(1.0);
        if count > MAX_CANVAS_SAMPLES as f64 || samples + count as usize > MAX_CANVAS_SAMPLES {
            return Err(Error::TooManySamples);
        }
        samples += count as usize;
        for k in 1..=count as usize {
            let t = k as f64 / count;
            let (x, y) = curve(p, t);
            painter.point(x, y, p[1].pressure + (p[2].pressure - p[1].pressure) * t)?;
        }
    }
    painter.finish(samples + usize::from(!ps.is_empty()), 0, 0)
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
        let mut painter = Painter::new(
            vec![ChannelPaint {
                channel: Channel::Color,
                color: b.color,
            }],
            b,
            &o,
        )
        .unwrap();
        painter.point(24.0, 24.0, 1.0).unwrap();
        assert!(painter.parts[0].doc.allocated_bytes() > 0);
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(painter.finish(1, 0, 0).unwrap_err(), Error::Canceled);
    }
}
