//! 効果のブラシ（C# の BrushStroke.Effects）の読み元: ダブの前に、読む範囲の画素を凍結した枠。書いている間に面を読み返さないので、
//! ワーカーやタイルの順によらない。ぼかしは枠の積分画像（プリマルチプライドの和）で箱の平均を、指先とクローンは双線形で読む。

use crate::types::Rgba8;

/// ダブ 1 つの読み元（画布の画素 [x, x + width) × [y, y + height)）。
pub(crate) struct EffectFrame {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    pub(crate) pixels: Vec<Rgba8>,
    integral: Vec<i64>,
}

impl EffectFrame {
    pub(crate) fn new(x: i64, y: i64, width: i64, height: i64) -> EffectFrame {
        EffectFrame {
            x,
            y,
            width,
            height,
            pixels: vec![Rgba8::TRANSPARENT; (width * height) as usize],
            integral: Vec::new(),
        }
    }

    /// 前のダブの枠の領域を使い回して、透明の枠にする。
    pub(crate) fn reset(mut self, x: i64, y: i64, width: i64, height: i64) -> EffectFrame {
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels
            .resize((width * height) as usize, Rgba8::TRANSPARENT);
        self
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn set(&mut self, px: i64, py: i64, c: Rgba8) {
        self.pixels[((py - self.y) * self.width + px - self.x) as usize] = c;
    }

    /// 枠の行 py の、画布の x 座標 from から始まる区間（書き込み用）。
    #[inline]
    pub(crate) fn row_mut(&mut self, py: i64, from: i64, len: usize) -> &mut [Rgba8] {
        let start = ((py - self.y) * self.width + from - self.x) as usize;
        &mut self.pixels[start..start + len]
    }

    /// 画布の画素 (px, py) から右へ n 画素（枠の中に全部あるとき）のバイト（RGBA の並び）を out の先頭へ写す。無ければ false。
    #[inline]
    pub(crate) fn run_bytes(&self, px: i64, py: i64, n: usize, out: &mut [u8]) -> bool {
        let (cx, cy) = (px - self.x, py - self.y);
        if cx < 0 || cy < 0 || cy >= self.height || cx + n as i64 > self.width {
            return false;
        }
        let start = (cy * self.width + cx) as usize;
        for (k, p) in self.pixels[start..start + n].iter().enumerate() {
            out[k * 4..k * 4 + 4].copy_from_slice(&p.to_array());
        }
        true
    }

    /// 積分画像の、画布の箱 [x0, x1) × [y0, y1)（枠の中。`blur` と同じ式）の 4 チャンネルの和。
    #[inline]
    pub(crate) fn box_sums(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> [i64; 4] {
        let stride = (self.width + 1) * 4;
        let tl = ((y0 - self.y) * stride + (x0 - self.x) * 4) as usize;
        let tr = ((y0 - self.y) * stride + (x1 - self.x) * 4) as usize;
        let bl = ((y1 - self.y) * stride + (x0 - self.x) * 4) as usize;
        let br = ((y1 - self.y) * stride + (x1 - self.x) * 4) as usize;
        let s = &self.integral;
        let sum = |c: usize| s[br + c] - s[bl + c] - s[tr + c] + s[tl + c];
        [sum(0), sum(1), sum(2), sum(3)]
    }

    /// 積分画像が、画布の箱 [x0, x1) × [y0, y1) を全部含むか。
    #[inline]
    pub(crate) fn integral_covers(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> bool {
        !self.integral.is_empty()
            && x0 >= self.x
            && y0 >= self.y
            && x1 <= self.x + self.width
            && y1 <= self.y + self.height
    }

    /// 画素 (x, y) の色（枠の中。色の混ぜが下地を 1 画素ずつ読む）。
    #[inline]
    pub(crate) fn pixel(&self, px: i64, py: i64) -> Rgba8 {
        self.pixels[((py - self.y) * self.width + px - self.x) as usize]
    }

    /// 積分画像（行ごとの累積の和を上へ足す。R·A・G·A・B·A・A の 4 つ）。
    pub(crate) fn build_integral(&mut self) {
        let stride = ((self.width + 1) * 4) as usize;
        let width = self.width as usize;
        let mut integral = std::mem::take(&mut self.integral);
        // 使い回しの領域: 書かない 0 行目と 0 列目だけを 0 にする（ほかは下で全部書く）
        integral.resize(stride * (self.height + 1) as usize, 0);
        integral[..stride].fill(0);
        for y in 1..=self.height as usize {
            // 1 つ上の行（読む）と今の行（書く）を分けて持つ（範囲の確かめを画素ごとに挟まない）
            let (above, rest) = integral.split_at_mut(y * stride);
            let up = &above[(y - 1) * stride..];
            let cur = &mut rest[..stride];
            cur[..4].fill(0);
            let (mut r, mut g, mut b, mut a) = (0i64, 0i64, 0i64, 0i64);
            let pixels = &self.pixels[(y - 1) * width..y * width];
            for ((p, up4), out4) in pixels
                .iter()
                .zip(up[4..].chunks_exact(4))
                .zip(cur[4..].chunks_exact_mut(4))
            {
                let pa = p.a as i64;
                r += p.r as i64 * pa;
                g += p.g as i64 * pa;
                b += p.b as i64 * pa;
                a += pa;
                out4[0] = up4[0] + r;
                out4[1] = up4[1] + g;
                out4[2] = up4[2] + b;
                out4[3] = up4[3] + a;
            }
        }
        self.integral = integral;
    }

    /// (x, y) を中心に半径 radius の箱の平均（画布 w × h の中だけ。プリマルチプライドの平均、アルファは箱の画素の平均）。
    pub(crate) fn blur(&self, x: i64, y: i64, radius: i64, w: i64, h: i64) -> Rgba8 {
        let x0 = (x - radius).max(0);
        let y0 = (y - radius).max(0);
        let x1 = (x + radius).min(w - 1) + 1;
        let y1 = (y + radius).min(h - 1) + 1;
        let stride = (self.width + 1) * 4;
        let tl = ((y0 - self.y) * stride + (x0 - self.x) * 4) as usize;
        let tr = ((y0 - self.y) * stride + (x1 - self.x) * 4) as usize;
        let bl = ((y1 - self.y) * stride + (x0 - self.x) * 4) as usize;
        let br = ((y1 - self.y) * stride + (x1 - self.x) * 4) as usize;
        let s = &self.integral;
        let sum = |c: usize| s[br + c] - s[bl + c] - s[tr + c] + s[tl + c];
        let a = sum(3);
        if a == 0 {
            return Rgba8::TRANSPARENT;
        }
        let af = a as f64;
        Rgba8::new(
            byte255(sum(0) as f64 / af),
            byte255(sum(1) as f64 / af),
            byte255(sum(2) as f64 / af),
            byte255(af / ((x1 - x0) * (y1 - y0)) as f64),
        )
    }

    /// 画素の座標 (x, y)（整数で画素そのもの）の双線形（プリマルチプライド）。画布の端の外は端の画素を延ばす。
    pub(crate) fn sample(&self, x: f64, y: f64, w: i64, h: i64) -> Rgba8 {
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let fx = x - ix as f64;
        let fy = y - iy as f64;
        let at = |px: i64, py: i64| {
            self.pixels[((py.min(h - 1) - self.y) * self.width + px.min(w - 1) - self.x) as usize]
        };
        if fx == 0.0 && fy == 0.0 {
            return at(ix, iy);
        }
        let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
        let mut add = |p: Rgba8, weight: f64| {
            let v = p.a as f64 * weight;
            r += p.r as f64 * v;
            g += p.g as f64 * v;
            b += p.b as f64 * v;
            a += v;
        };
        add(at(ix, iy), (1.0 - fx) * (1.0 - fy));
        add(at(ix + 1, iy), fx * (1.0 - fy));
        add(at(ix, iy + 1), (1.0 - fx) * fy);
        add(at(ix + 1, iy + 1), fx * fy);
        if a <= 0.0 {
            Rgba8::TRANSPARENT
        } else {
            Rgba8::new(byte255(r / a), byte255(g / a), byte255(b / a), byte255(a))
        }
    }

    /// 枠が持つバイト（予算に数える分は呼び手が C# と同じ式で見積もる）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pixels.len()
    }
}

/// 指先・ぼかしの混ぜ方（C# の MixEffect の、透明部分のロックの無い経路）: アルファで重みを付けた補間。
#[inline]
pub(crate) fn mix_effect(start: Rgba8, sample: Rgba8, amount: f64) -> Rgba8 {
    let a = start.a as f64 * (1.0 - amount);
    let b = sample.a as f64 * amount;
    let alpha = a + b;
    if alpha <= 0.0 {
        return Rgba8::new(start.r, start.g, start.b, 0);
    }
    Rgba8::new(
        byte255((start.r as f64 * a + sample.r as f64 * b) / alpha),
        byte255((start.g as f64 * a + sample.g as f64 * b) / alpha),
        byte255((start.b as f64 * a + sample.b as f64 * b) / alpha),
        byte255(alpha),
    )
}

/// 0〜255 の値を四捨五入して収める（C# の Byte255: Math.Max(0, Math.Min(255, Math.Floor(v + .5)))）。
#[inline]
pub(crate) fn byte255(v: f64) -> u8 {
    let f = (v + 0.5).floor();
    let m = if 255.0 < f { 255.0 } else { f };
    let c = if 0.0 > m { 0.0 } else { m };
    c as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_averages_premultiplied_and_ignores_transparent_colour() {
        // C# BrushEffectTests.BlurLeavesZeroAlphaRgbAndIgnoresItsColorInTheAverage の式の部分
        let mut f = EffectFrame::new(0, 0, 3, 1);
        f.set(0, 0, Rgba8::new(200, 0, 0, 255));
        f.set(1, 0, Rgba8::new(0, 255, 0, 0)); // 透明の RGB は平均に入らない
        f.set(2, 0, Rgba8::new(0, 0, 100, 255));
        f.build_integral();
        let c = f.blur(1, 0, 1, 3, 1);
        assert_eq!(c, Rgba8::new(100, 0, 50, 170));
        assert_eq!(f.len(), 3);
    }

    #[test]
    fn sample_is_bilinear_and_exact_on_pixels() {
        let mut f = EffectFrame::new(2, 2, 2, 1);
        f.set(2, 2, Rgba8::new(0, 0, 0, 255));
        f.set(3, 2, Rgba8::new(255, 255, 255, 255));
        assert_eq!(f.sample(2.0, 2.0, 4, 3), Rgba8::new(0, 0, 0, 255));
        assert_eq!(f.sample(2.5, 2.0, 4, 3).r, 128);
        assert_eq!(f.sample(3.5, 2.0, 4, 3), Rgba8::new(255, 255, 255, 255)); // 画布の端の外は端を延ばす
    }

    #[test]
    fn byte255_rounds_and_clamps() {
        assert_eq!(byte255(-3.0), 0);
        assert_eq!(byte255(254.5), 255);
        assert_eq!(byte255(300.0), 255);
        assert_eq!(byte255(127.49), 127);
    }

    #[test]
    fn mix_effect_keeps_rgb_when_both_are_transparent() {
        assert_eq!(
            mix_effect(Rgba8::new(9, 8, 7, 0), Rgba8::new(1, 2, 3, 0), 0.5),
            Rgba8::new(9, 8, 7, 0)
        );
    }
}
