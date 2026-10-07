//! リボンのパス（種類 `Ribbon`）: アセットの画像を、パスの向きに回した長方形のダブとして並べる。
//!
//! - ダブの幅（パスに直交する向き）はブラシの直径（筆圧で大きさを変えるなら筆圧を掛ける）。画像の横（x）がパスに沿う向き、縦（y）が
//!   幅の向き（パスの進む向きの左が上）。
//! - `Tile`: 1 つのダブに画像の全部。ダブの長さは幅 × 画像の横縦の比、中心の間隔は長さ × 間隔。パスの始めから並べる。
//! - `Stretch`: パス全体に 1 枚を伸ばしたのに近い形。パスを幅の 1/4 ほどの長さの n 個のダブに分け、ダブ i に画像の [i/n, (i+1)/n] の区間。
//! - ダブの向きは、ダブの両端の間の曲線の向き（2D はそのまま、3D はダブの中心の面の接平面へ落としたもの）。3D のダブの画素は、面の
//!   ダブ（中心から長方形の対角の半分の球）の画素のうち、中心の接平面の座標が長方形に入るもの。
//! - 1 本のリボンの中で重なる画素は、覆い（画像のアルファ）の大きい方。全部のダブを集めてから、色の種類のチャンネル（Color・
//!   Emission）は画像の色で、ほかのチャンネルは組の値（無ければブラシの色）で、覆い × 不透明度を「通常」で重ねる。
//! - 画像は双線形で読む（乗算済みで混ぜる）。リニアの色空間の画像は sRGB に直して色のチャンネルに使う。

use std::collections::BTreeMap;

use glam::{DVec2, Vec3};

use super::render::{canvas_curve, Painter};
use super::surface::{project, surface_curve, CurvePoint};
use super::*;
use crate::effects::ImageInput;
use crate::geometry::unity::{cross, dot, magnitude, normalized};
use crate::geometry::SurfaceGeometry;
use crate::Rgba8;

/// 伸ばすときの 1 つのダブの長さ（幅に対する割合）。
const STRETCH_PIECE: f64 = 0.25;
/// 集めた画素 1 つの見積もりのバイト（ストロークの予算に数える）。
const PIXEL_BYTES: u64 = 32;
/// 1 本のリボンのダブの数の上限。
const MAX_DABS: usize = 1_000_000;

/// 画像の読み手。
struct Sampler<'a> {
    image: &'a ImageInput,
    linear: bool,
}

impl Sampler<'_> {
    fn new(image: &ImageInput) -> Sampler<'_> {
        Sampler {
            image,
            linear: image.color_space == crate::brush::ImageColorSpace::Linear,
        }
    }
    /// (u, v)（0〜1、左下原点）の色（straight）と覆い（0〜1）。双線形（乗算済みで混ぜる）。
    fn sample(&self, u: f64, v: f64) -> (Rgba8, f64) {
        let (w, h) = (self.image.width as i64, self.image.height as i64);
        let x = (u.clamp(0.0, 1.0) * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
        let y = (v.clamp(0.0, 1.0) * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (x.floor() as i64, y.floor() as i64);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let px = |xx: i64, yy: i64| {
            let i = ((yy * w + xx) * 4) as usize;
            let p = &self.image.pixels[i..i + 4];
            let a = p[3] as f64 / 255.0;
            [p[0] as f64 * a, p[1] as f64 * a, p[2] as f64 * a, a]
        };
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bottom - top) * fy;
        }
        let alpha = out[3];
        if alpha <= 0.0 {
            return (Rgba8::new(0, 0, 0, 0), 0.0);
        }
        let channel = |v: f64| {
            let byte = (v / alpha).round().clamp(0.0, 255.0) as u8;
            if self.linear {
                crate::brush::linear_to_srgb(byte)
            } else {
                byte
            }
        };
        (
            Rgba8::new(
                channel(out[0]),
                channel(out[1]),
                channel(out[2]),
                (alpha * 255.0).round() as u8,
            ),
            alpha,
        )
    }
}

/// 1 本のリボンが集めた画素（覆いの大きい方を残す）。
struct Gathered {
    pixels: BTreeMap<(i32, i32), (f64, Rgba8)>,
    budget: u64,
}

impl Gathered {
    fn put(&mut self, x: i32, y: i32, alpha: f64, color: Rgba8) -> Result<(), Error> {
        if alpha <= 0.0 {
            return Ok(());
        }
        match self.pixels.get_mut(&(y, x)) {
            Some(old) if old.0 >= alpha => {}
            Some(old) => *old = (alpha, color),
            None => {
                if (self.pixels.len() as u64 + 1) * PIXEL_BYTES > self.budget {
                    return Err(Error::Core(CoreError::StrokeBudgetExceeded));
                }
                self.pixels.insert((y, x), (alpha, color));
            }
        }
        Ok(())
    }
    /// 不透明度を掛けて、作業面へ重ねる。
    fn finish(
        self,
        painter: &mut Painter<'_, '_>,
        paints: &[ChannelPaint],
        opacity: f64,
    ) -> Result<(), Error> {
        let list: Vec<(i32, i32, Rgba8, f64)> = self
            .pixels
            .into_iter()
            .map(|((y, x), (a, c))| (x, y, c, a * opacity))
            .collect();
        painter.blend(paints, &list)
    }
}

/// ダブの並び（パスの始めからの長さの中心・長さ・画像の区間）。
fn layout(
    total: f64,
    width: f64,
    image: &ImageInput,
    r: Ribbon,
) -> Result<Vec<(f64, f64, f64, f64)>, Error> {
    let mut out = Vec::new();
    if total <= 0.0 || width <= 0.0 {
        return Ok(out);
    }
    match r.mode {
        RibbonMode::Tile => {
            let length = width * image.width as f64 / image.height as f64;
            let step = length * r.spacing;
            let mut center = length * 0.5;
            while center - length * 0.5 < total {
                if out.len() >= MAX_DABS {
                    return Err(Error::TooManySamples);
                }
                out.push((center, length, 0.0, 1.0));
                center += step;
            }
        }
        RibbonMode::Stretch => {
            let n = (total / (width * STRETCH_PIECE)).ceil().max(1.0);
            if n > MAX_DABS as f64 {
                return Err(Error::TooManySamples);
            }
            let n = n as usize;
            let length = total / n as f64;
            for i in 0..n {
                out.push((
                    (i as f64 + 0.5) * length,
                    length,
                    i as f64 / n as f64,
                    (i + 1) as f64 / n as f64,
                ));
            }
        }
    }
    Ok(out)
}

/// 折れ線の、始めからの長さ `s` の所（0〜全長に収める）の点の番号と割合。
fn locate(lengths: &[f64], s: f64) -> (usize, f64) {
    let s = s.clamp(0.0, *lengths.last().unwrap_or(&0.0));
    let i = lengths
        .partition_point(|l| *l <= s)
        .clamp(1, lengths.len().max(2) - 1);
    let (a, b) = (lengths[i - 1], lengths[i]);
    (i, if b > a { (s - a) / (b - a) } else { 0.0 })
}

fn image<'a>(o: &'a Options<'_>, r: Ribbon) -> Result<&'a ImageInput, Error> {
    o.images.get(&r.image).ok_or(Error::MissingImage)
}

/// 2D のリボン。曲線の標本の数を返す。
pub(super) fn draw_canvas(
    painter: &mut Painter<'_, '_>,
    path: &CanvasPath,
    r: Ribbon,
) -> Result<usize, Error> {
    let paints = paints(path.channel, path.brush, &path.material)?;
    let img = image(painter.options(), r)?.clone();
    let sampler = Sampler::new(&img);
    let b = path.brush.0;
    let mut line: Vec<(DVec2, f64)> = Vec::new();
    let samples = canvas_curve(path, 0.5, &mut |x, y, p| {
        line.push((DVec2::new(x, y), p));
        Ok(())
    })?;
    let mut lengths = Vec::with_capacity(line.len());
    let mut total = 0.0;
    for (i, (p, _)) in line.iter().enumerate() {
        if i > 0 {
            total += p.distance(line[i - 1].0);
        }
        lengths.push(total);
    }
    let width = b.radius * 2.0;
    let mut gathered = Gathered {
        pixels: BTreeMap::new(),
        budget: painter.options().stroke_budget_bytes,
    };
    // パスの外（始めの前・終わりの後）は端の向きにまっすぐ伸ばす（最後のダブが端で切れるように、中心は外にも置ける）
    let at = |s: f64| -> (DVec2, f64) {
        if line.len() == 1 {
            return line[0];
        }
        let n = line.len();
        if s > total {
            let dir = (line[n - 1].0 - line[n - 2].0).normalize_or_zero();
            return (line[n - 1].0 + dir * (s - total), line[n - 1].1);
        }
        if s < 0.0 {
            let dir = (line[1].0 - line[0].0).normalize_or_zero();
            return (line[0].0 + dir * s, line[0].1);
        }
        let (i, f) = locate(&lengths, s);
        let (a, b) = (line[i - 1], line[i]);
        (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
    };
    let (w, h) = painter.size();
    if line.len() >= 2 {
        for (center, length, u0, u1) in layout(total, width, &img, r)? {
            painter.options().check()?;
            let (c, pressure) = at(center);
            let dir =
                (at(center + length * 0.5).0 - at(center - length * 0.5).0).normalize_or_zero();
            if dir == DVec2::ZERO {
                continue;
            }
            let nrm = DVec2::new(-dir.y, dir.x);
            let scale = if b.pressure_size {
                pressure.max(0.001)
            } else {
                1.0
            };
            let (hl, hw) = (length * 0.5, width * scale * 0.5);
            let corners = [
                c + dir * hl + nrm * hw,
                c + dir * hl - nrm * hw,
                c - dir * hl + nrm * hw,
                c - dir * hl - nrm * hw,
            ];
            let (mut lo, mut hi) = (corners[0], corners[0]);
            for k in corners {
                lo = lo.min(k);
                hi = hi.max(k);
            }
            let x0 = (lo.x.floor() as i64 - 1).max(0);
            let x1 = (hi.x.ceil() as i64 + 1).min(w as i64 - 1);
            let y0 = (lo.y.floor() as i64 - 1).max(0);
            let y1 = (hi.y.ceil() as i64 + 1).min(h as i64 - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = DVec2::new(x as f64 + 0.5, y as f64 + 0.5) - c;
                    let (along, across) = (d.dot(dir), d.dot(nrm));
                    // 長さの向きは隣のダブと半画素ずつ重ねる（隙間を出さない）。パスの端の外は切る。幅の縁は 1 画素で落とす
                    if along.abs() > hl + 0.5
                        || center + along > total + 0.5
                        || center + along < -0.5
                    {
                        continue;
                    }
                    let edge = (hw + 0.5 - across.abs()).clamp(0.0, 1.0);
                    if edge <= 0.0 {
                        continue;
                    }
                    let u = u0 + (along / length + 0.5).clamp(0.0, 1.0) * (u1 - u0);
                    let v = across / (hw * 2.0) + 0.5;
                    let (color, alpha) = sampler.sample(u, v);
                    gathered.put(x as i32, y as i32, alpha * edge, color)?;
                }
            }
        }
    }
    let opacity = b.opacity;
    gathered.finish(painter, &paints, opacity)?;
    Ok(samples)
}

/// 3D のリボン。(曲線の標本の数, ダブの数, 面に投影できなかったダブの数)。
pub(super) fn draw_surface(
    painter: &mut Painter<'_, '_>,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    o: &Options<'_>,
    r: Ribbon,
) -> Result<(usize, usize, usize), Error> {
    let paints = paints(path.channel, path.brush, &path.material)?;
    let img = image(o, r)?;
    let sampler = Sampler::new(img);
    let b = path.brush.0;
    let width = b.radius * 2.0;
    let curve = surface_curve(path, g, (width * 0.1) as f32, o)?;
    let mut lengths = Vec::with_capacity(curve.len());
    let mut total = 0.0;
    for (i, q) in curve.iter().enumerate() {
        if i > 0 {
            total += magnitude(q.position - curve[i - 1].position) as f64;
        }
        lengths.push(total);
    }
    let at = |s: f64| -> CurvePoint {
        if curve.len() == 1 {
            let q = &curve[0];
            return CurvePoint { ..*q };
        }
        let n = curve.len();
        if s > total || s < 0.0 {
            let (a, c, from, beyond) = if s > total {
                (&curve[n - 2], &curve[n - 1], &curve[n - 1], s - total)
            } else {
                (&curve[1], &curve[0], &curve[0], -s)
            };
            let dir = c.position - a.position;
            let dir = if magnitude(dir) > 1e-12 {
                normalized(dir)
            } else {
                Vec3::ZERO
            };
            return CurvePoint {
                position: from.position + dir * beyond as f32,
                ..*from
            };
        }
        let (i, f) = locate(&lengths, s);
        let (a, c) = (&curve[i - 1], &curve[i]);
        let f32f = f as f32;
        CurvePoint {
            position: a.position + (c.position - a.position) * f32f,
            normal: normalized(a.normal + (c.normal - a.normal) * f32f),
            pressure: a.pressure + (c.pressure - a.pressure) * f,
            reach: a.reach.max(c.reach),
        }
    };
    let mut gathered = Gathered {
        pixels: BTreeMap::new(),
        budget: o.stroke_budget_bytes,
    };
    let (mut dabs, mut gaps) = (0, 0);
    if curve.len() >= 2 {
        for (center, length, u0, u1) in layout(total, width, img, r)? {
            o.check()?;
            let q = at(center);
            let Some(hit) = project(g, &q) else {
                gaps += 1;
                continue;
            };
            let n = hit.normal;
            let chord = at(center + length * 0.5).position - at(center - length * 0.5).position;
            let along_plane = chord - n * dot(chord, n);
            if magnitude(along_plane) < 1e-12 {
                gaps += 1;
                continue;
            }
            let dir = normalized(along_plane);
            let side = cross(n, dir);
            let scale = if b.pressure_size {
                q.pressure.max(0.001)
            } else {
                1.0
            };
            let (hl, hw) = ((length * 0.5) as f32, (width * scale * 0.5) as f32);
            let radius = (hl * hl + hw * hw).sqrt();
            let result = g.build_surface_dabs(
                &hit,
                radius,
                o.width as i32,
                o.height as i32,
                hit.position + n * (radius * 4.0).max(1e-5),
                1.0,
                &o.surface_budget,
                None,
                false,
            );
            if let Some(e) = result.refusal {
                return Err(Error::Dab(e));
            }
            for p in result.pixels {
                let d: Vec3 = p.position - hit.position;
                let (a, s) = (dot(d, dir), dot(d, side));
                // パスの端の外は切る
                let along = center + a as f64;
                if a.abs() > hl || s.abs() > hw || along > total || along < 0.0 {
                    continue;
                }
                let u = u0 + (a as f64 / length + 0.5).clamp(0.0, 1.0) * (u1 - u0);
                let v = s as f64 / (hw as f64 * 2.0) + 0.5;
                let (color, alpha) = sampler.sample(u, v);
                gathered.put(p.x, p.y, alpha, color)?;
            }
            dabs += 1;
        }
    }
    gathered.finish(painter, &paints, b.opacity)?;
    Ok((curve.len(), dabs, gaps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_follow_the_image_ratio_and_stretch_splits_the_whole_image() {
        let img = ImageInput::new(
            4,
            2,
            vec![255; 4 * 2 * 4],
            crate::brush::ImageColorSpace::Srgb,
        )
        .unwrap();
        let r = Ribbon {
            image: crate::ImageId(1),
            mode: RibbonMode::Tile,
            spacing: 1.0,
        };
        // 幅 10・比 2 の画像: 長さ 20 のダブを 20 ずつ、全長 45 なら 3 つ
        let t = layout(45.0, 10.0, &img, r).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].0, t[0].1), (10.0, 20.0));
        assert_eq!(t[2].0, 50.0);
        let s = layout(
            45.0,
            10.0,
            &img,
            Ribbon {
                mode: RibbonMode::Stretch,
                ..r
            },
        )
        .unwrap();
        // 幅の 1/4 = 2.5 → 18 個、区間は [i/18, (i+1)/18]
        assert_eq!(s.len(), 18);
        assert_eq!((s[0].2, s[17].3), (0.0, 1.0));
        assert!((s[5].3 - s[6].2).abs() < 1e-12);
    }

    #[test]
    fn the_sampler_reads_straight_colors_with_premultiplied_mixing() {
        // 左が不透明な赤、右が透明な緑: 真ん中は赤のまま、覆いは半分
        let img = ImageInput::new(
            2,
            1,
            vec![255, 0, 0, 255, 0, 255, 0, 0],
            crate::brush::ImageColorSpace::Srgb,
        )
        .unwrap();
        let s = Sampler::new(&img);
        let (c, a) = s.sample(0.5, 0.5);
        assert_eq!((c.r, c.g, c.b), (255, 0, 0));
        assert!((a - 0.5).abs() < 1e-9);
        assert_eq!(s.sample(1.0, 0.5).1, 0.0);
    }
}
