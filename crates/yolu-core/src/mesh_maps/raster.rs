use super::{check, MeshBakeInput, MeshBakeSettings, Result};
pub(crate) struct Raster {
    pub triangles: Vec<RasterTriangle>,
    pub bands: Vec<Vec<usize>>,
}
pub(crate) struct RasterTriangle {
    pub original: usize,
    pub affine: [f64; 6],
    pub x0: i32,
    pub x1: i32,
    pub y0: i32,
    pub y1: i32,
}
#[derive(Clone, Copy)]
pub(crate) struct Sample {
    pub triangle: usize,
    pub u: f64,
    pub v: f64,
    pub overlap: bool,
}
pub(crate) struct RowSamples {
    owner: Vec<i32>,
    u: Vec<f64>,
    v: Vec<f64>,
    interior: Vec<u8>,
    overlap: Vec<u8>,
}
impl RowSamples {
    fn new(n: usize) -> Self {
        Self {
            owner: vec![-1; n],
            u: vec![0.; n],
            v: vec![0.; n],
            interior: vec![0; n],
            overlap: vec![0; n],
        }
    }
    pub fn get(&self, i: usize) -> Option<Sample> {
        (self.owner[i] >= 0).then(|| Sample {
            triangle: self.owner[i] as usize,
            u: self.u[i],
            v: self.v[i],
            overlap: self.overlap[i] != 0,
        })
    }
}
impl Raster {
    pub fn new(
        input: &MeshBakeInput,
        s: &MeshBakeSettings,
        receivers: &[usize],
        remaining: u64,
    ) -> Result<Self> {
        Self::from_uvs(
            &input.uvs,
            (s.width, s.height),
            s.antialiasing,
            receivers,
            remaining,
        )
    }

    /// 三角形ごとの UV（三角形の番号の順に a.x, a.y, b.x, b.y, c.x, c.y の 6 つ）から。ベイクの割り当てと同じ式で、UV の島の図
    /// （`geometry::UvTopology`）もこれを使う（重なりの見つけ方を 1 つにする）。
    pub fn from_uvs(
        uvs: &[f32],
        (width, height): (i32, i32),
        n: i32,
        receivers: &[usize],
        remaining: u64,
    ) -> Result<Self> {
        let first = 0.5 / n as f64;
        let last = 1. - first;
        let mut triangles = vec![];
        let mut items = 0usize;
        for &t in receivers {
            let uv = &uvs[t * 6..];
            let ax = uv[0] as f64 * width as f64;
            let ay = uv[1] as f64 * height as f64;
            let bx = uv[2] as f64 * width as f64;
            let by = uv[3] as f64 * height as f64;
            let cx = uv[4] as f64 * width as f64;
            let cy = uv[5] as f64 * height as f64;
            let m0 = bx - ax;
            let m1 = cx - ax;
            let m2 = by - ay;
            let m3 = cy - ay;
            let inv = 1. / (m0 * m3 - m1 * m2);
            let x0 = ((ax.min(bx.min(cx)) - last - 1e-9).ceil() as i32).max(0);
            let x1 = ((ax.max(bx.max(cx)) - first + 1e-9).floor() as i32).min(width - 1);
            let mut y0 = ((ay.min(by.min(cy)) - last - 1e-9).ceil() as i32).max(0);
            let mut y1 = ((ay.max(by.max(cy)) - first + 1e-9).floor() as i32).min(height - 1);
            if x1 < x0 || y1 < y0 {
                y0 = 1;
                y1 = 0;
            } else {
                items += (y1 / 8 - y0 / 8 + 1) as usize;
            }
            triangles.push(RasterTriangle {
                original: t,
                affine: [ax, ay, m3 * inv, -m1 * inv, -m2 * inv, m0 * inv],
                x0,
                x1,
                y0,
                y1,
            });
        }
        check(
            items <= i32::MAX as usize / 2
                && items.saturating_sub(2 * receivers.len()) as u64 * 4 <= remaining,
            "UVの行帯参照がベイク予算を超えます",
        )?;
        let mut bands = vec![vec![]; (height as usize).div_ceil(8)];
        for (r, t) in triangles.iter().enumerate() {
            if t.y1 >= t.y0 {
                for band in t.y0 / 8..=t.y1 / 8 {
                    bands[band as usize].push(r);
                }
            }
        }
        Ok(Self { triangles, bands })
    }
    pub fn row(&self, y: usize, width: usize, n: usize) -> RowSamples {
        let mut samples = RowSamples::new(width * n * n);
        for j in 0..n {
            let py = y as f64 + (j as f64 + 0.5) / n as f64;
            for &r in &self.bands[y / 8] {
                let t = &self.triangles[r];
                if (y as i32) < t.y0 || (y as i32) > t.y1 {
                    continue;
                }
                let [u0, v0, m00, m01, m10, m11] = t.affine;
                let dy = py - v0;
                let c1 = m01 * dy;
                let c2 = m11 * dy;
                for x in t.x0..=t.x1 {
                    for i in 0..n {
                        let dx = x as f64 + (i as f64 + 0.5) / n as f64 - u0;
                        let u = m00 * dx + c1;
                        let v = m10 * dx + c2;
                        let w = 1. - u - v;
                        let min = w.min(u.min(v));
                        if min < -1e-9 {
                            continue;
                        }
                        let interior = min > 1e-6;
                        let idx = (j * width + x as usize) * n + i;
                        if samples.owner[idx] < 0 {
                            samples.owner[idx] = t.original as i32;
                            samples.u[idx] = u;
                            samples.v[idx] = v;
                            samples.interior[idx] = u8::from(interior);
                        } else if interior && samples.interior[idx] != 0 {
                            samples.overlap[idx] = 1;
                        }
                    }
                }
            }
        }
        samples
    }
}
