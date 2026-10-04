//! 選択範囲の変更: 拡張・縮小・境界・ぼかし（C# の SelectionModify。GIMP の Select メニューの考え方）。
//!
//! 拡張・縮小は濃淡のあるモルフォロジー（半径の円の中の最大・最小）で、柔らかい縁は柔らかいまま。作業は量のあるタイルの外接の
//! 矩形を半径だけ広げた窓に限り、行（ぼかしの縦は列）ごとに並列。どの行も独立に計算するので、結果はスレッドの数によらない。

use rayon::prelude::*;

use super::{ensure_working, SelectionMask, Window};
use crate::error::CoreError;
use crate::math::require_finite;

/// 拡張・縮小・境界・ぼかしの半径の上限（計算は半径に比例して増える）。
pub const MAX_MODIFY_RADIUS: u32 = 200;

fn check_radius(radius: u32) -> Result<(), CoreError> {
    if radius > MAX_MODIFY_RADIUS {
        Err(CoreError::InvalidArgument("半径（0〜200）"))
    } else {
        Ok(())
    }
}

/// 窓と周りの作業の大きさ（バイト）の見積もり: 窓の量・広げた窓・出力・ワーカーの列の溜まり。
fn morphology_bytes(w: Window, pad: i64, threads: usize) -> u128 {
    let (ww, wh) = (w.width as u128, w.height as u128);
    let (pw, ph) = (ww + 2 * pad as u128, wh + 2 * pad as u128);
    ww * wh * 2 + pw * ph + (pad as u128 + 1) * pw * threads as u128
}

/// 並列の 1 つの仕事が受け持つ計算の下限（画素 × 画素あたりの手間）。小さな窓でワーカーを何十本も起こす費用が計算そのものより
/// 大きくなる（32 スレッドで半径 200 の楕円のぼかしが 1 スレッドの 4 倍かかった）のを避け、行の数で仕事を束ねる。行ごとに独立に
/// 計算するので、束ね方で結果は変わらない。
const MIN_JOB_WORK: usize = 1 << 17;

/// 幅 width の行を、1 画素の手間 cost で束ねたとき、1 つの仕事に入れる行の数（1 以上）。
fn min_rows(width: usize, cost: usize) -> usize {
    (MIN_JOB_WORK / (width * cost).max(1)).max(1)
}

/// タイルの画素ごとに軽い計算（1 画素 1 回）をするとき、1 つの仕事に入れるタイルの数（1 以上）。
pub(super) fn min_tiles(tile_size: usize) -> usize {
    (MIN_JOB_WORK / (tile_size * tile_size).max(1)).max(1)
}

/// ぼかしの作業の大きさ（バイト）の見積もり: 窓の量・出力（窓 × 2）、広げた窓（バイト）、float の 2 枚と縦の箱ぼかしの列の束
/// （float 1 枚分）。
fn feather_bytes(w: Window, pad: i64) -> u128 {
    let cells = (w.width as u128 + 2 * pad as u128) * (w.height as u128 + 2 * pad as u128);
    w.width as u128 * w.height as u128 * 2 + cells + cells * 4 * 3
}

impl SelectionMask {
    /// どの画素も、半径の円の中の最大の量になる（GIMP の Grow）。画布の外は選ばれていないと数える。
    pub fn grow(&self, radius: u32, budget: u64) -> Result<SelectionMask, CoreError> {
        check_radius(radius)?;
        if radius == 0 || self.is_empty() {
            return Ok(self.clone());
        }
        let Some(w) = self.window(radius as i64) else {
            return Ok(self.clone());
        };
        ensure_working(morphology_bytes(w, radius as i64, threads()), budget)?;
        let padded = self.padded(w, radius as i64, false, false);
        let out = morphology(&padded, w, radius as i64, true);
        Ok(self.with_dense(&out, w))
    }

    /// どの画素も、半径の円の中の最小の量になる（GIMP の Shrink）。edge_lock なら選択範囲は画布の外へ続く（画布の縁からは縮まない）。
    pub fn shrink(
        &self,
        radius: u32,
        edge_lock: bool,
        budget: u64,
    ) -> Result<SelectionMask, CoreError> {
        check_radius(radius)?;
        if radius == 0 || self.is_empty() {
            return Ok(self.clone());
        }
        let Some(w) = self.window(0) else {
            return Ok(self.clone());
        };
        ensure_working(morphology_bytes(w, radius as i64, threads()), budget)?;
        let padded = self.padded(w, radius as i64, edge_lock, false);
        let out = morphology(&padded, w, radius as i64, false);
        Ok(self.with_dense(&out, w))
    }

    /// 縁の帯: 拡張(半径) − 縮小(半径 + 1)（GIMP の滑らかな境界。半径 1 は元から縮小(1) を引く、GIMP の特別な扱い）。半径 0 は空。
    pub fn border(
        &self,
        radius: u32,
        edge_lock: bool,
        budget: u64,
    ) -> Result<SelectionMask, CoreError> {
        check_radius(radius)?;
        if radius == 0 || self.is_empty() {
            return Ok(Self::blank(self.width(), self.height(), self.tile_size()));
        }
        let Some(w) = self.window(radius as i64) else {
            return Ok(self.clone());
        };
        let r = radius as i64;
        let inner = if radius == 1 { 1 } else { r + 1 };
        ensure_working(
            morphology_bytes(w, r, threads()) + morphology_bytes(w, inner, threads()),
            budget,
        )?;
        let mut grown = if radius == 1 {
            self.dense(w)
        } else {
            morphology(&self.padded(w, r, false, false), w, r, true)
        };
        let shrunk = morphology(&self.padded(w, inner, edge_lock, false), w, inner, false);
        for (g, s) in grown.iter_mut().zip(&shrunk) {
            *g = g.saturating_sub(*s);
        }
        Ok(self.with_dense(&grown, w))
    }

    /// 量のガウスぼかし（標準偏差は半径 / 3.5、GIMP の Feather の定数）。画布の外は選ばれていない、edge_lock なら縁の画素が
    /// 続く。小さい半径は正確な核、大きい半径は 3 回の箱ぼかし（ガウスの近似。C# と同じ式・同じ float の溜め方）。
    ///
    /// C# とのバイト一致を確かめたのは同じ libm（Linux の glibc）の上。正確な核（標準偏差 2 未満）は libm の `exp` を通るので、
    /// 別の libm では核が 1 ULP ずれ得る。箱ぼかしの分岐は `exp` を通らない。
    pub fn feather(
        &self,
        radius: f64,
        edge_lock: bool,
        budget: u64,
    ) -> Result<SelectionMask, CoreError> {
        require_finite(radius, "radius")?;
        if !(0.0..=MAX_MODIFY_RADIUS as f64).contains(&radius) {
            return Err(CoreError::InvalidArgument("半径（0〜200）"));
        }
        let sigma = radius / 3.5;
        if sigma < 0.05 || self.is_empty() {
            return Ok(self.clone());
        }
        let boxes = if sigma < 2.0 {
            None
        } else {
            Some(boxes_for_gauss(sigma, 3))
        };
        let kernel = if boxes.is_none() {
            Some(gaussian_kernel(sigma))
        } else {
            None
        };
        let reach = match (&boxes, &kernel) {
            (Some(b), _) => (b[0] + b[1] + b[2] - 3) / 2,
            (None, Some(k)) => (k.len() / 2) as i64,
            _ => unreachable!(),
        };
        let Some(w) = self.window(reach + 1) else {
            return Ok(self.clone());
        };
        let pad = reach;
        let (pw, ph) = (w.width + 2 * pad, w.height + 2 * pad);
        ensure_working(feather_bytes(w, pad), budget)?;
        let padded = self.padded(w, pad, edge_lock, true);
        let (pw, ph) = (pw as usize, ph as usize);
        let mut a: Vec<f32> = padded.iter().map(|&v| v as f32).collect();
        drop(padded);
        let mut b = vec![0f32; pw * ph];
        match (&kernel, &boxes) {
            (Some(k), _) => {
                convolve_rows(&a, &mut b, pw, k);
                convolve_columns(&b, &mut a, pw, ph, k);
            }
            (None, Some(sizes)) => {
                let mut blocks = Vec::new();
                for &size in sizes {
                    box_rows(&a, &mut b, pw, (size / 2) as usize);
                    box_columns(&b, &mut a, pw, ph, (size / 2) as usize, &mut blocks);
                }
            }
            _ => unreachable!(),
        }
        let (ww, wh) = (w.width as usize, w.height as usize);
        let pad = pad as usize;
        let mut result = vec![0u8; ww * wh];
        result
            .par_chunks_mut(ww)
            .enumerate()
            .with_min_len(min_rows(ww, 2))
            .for_each(|(y, row)| {
                let src = &a[(y + pad) * pw + pad..(y + pad) * pw + pad + ww];
                for (o, &v) in row.iter_mut().zip(src) {
                    // C# の (byte)Math.Max(0, Math.Min(255, Math.Floor(v + .5)))
                    let f = (v as f64 + 0.5).floor();
                    *o = if f >= 255.0 {
                        255
                    } else if f > 0.0 {
                        f as u8
                    } else {
                        0
                    };
                }
            });
        Ok(self.with_dense(&result, w))
    }

    /// 窓の周りに pad 画素を足したもの（C# の Padded）。画布の中で窓の外は選ばれていない（窓が量のある画素を全部含む）。
    /// 画布の外は、edge_lock なら 255（replicate なら一番近い縁の画素）、でなければ 0。
    fn padded(&self, w: Window, pad: i64, edge_lock: bool, replicate: bool) -> Vec<u8> {
        let dense = self.dense(w);
        let (pw, ph) = ((w.width + 2 * pad) as usize, (w.height + 2 * pad) as usize);
        let (width, height) = (self.width() as i64, self.height() as i64);
        let mut p = vec![0u8; pw * ph];
        let read = |cx: i64, cy: i64| -> u8 {
            let (dx, dy) = (cx - w.x, cy - w.y);
            if dx >= 0 && dy >= 0 && dx < w.width && dy < w.height {
                dense[(dy * w.width + dx) as usize]
            } else {
                0
            }
        };
        p.par_chunks_mut(pw)
            .enumerate()
            .with_min_len(min_rows(pw, 2))
            .for_each(|(py, row)| {
                let cy = w.y + py as i64 - pad;
                let out_y = cy < 0 || cy >= height;
                for (px, v) in row.iter_mut().enumerate() {
                    let cx = w.x + px as i64 - pad;
                    *v = if !out_y && cx >= 0 && cx < width {
                        read(cx, cy)
                    } else if !edge_lock {
                        0
                    } else if !replicate {
                        255
                    } else {
                        read(cx.clamp(0, width - 1), cy.clamp(0, height - 1))
                    };
                }
            });
        p
    }
}

fn threads() -> usize {
    rayon::current_num_threads().max(1)
}

/// GIMP の円（compute_border）: 横のずれ dx ごとの縦の半分の高さ。
fn circle(radius: i64) -> Vec<usize> {
    (0..=2 * radius)
        .map(|i| {
            let t = if i == radius {
                0.0
            } else {
                (i - radius).abs() as f64 - 0.5
            };
            (radius as f64 * radius as f64 - t * t)
                .sqrt()
                .round_ties_even() as usize
        })
        .collect()
}

/// 濃淡の膨張（最大）か収縮（最小）を GIMP の円で（C# の Morphology）: 出力の画素ごとに、横のずれ dx の列の ±circle[dx] 行の
/// 極値の、dx にわたる極値。入力は四方に radius だけ広げてある。行ごとに並列（行ごとの列の溜まりはワーカーが持つ）。
fn morphology(padded: &[u8], w: Window, radius: i64, max: bool) -> Vec<u8> {
    let (width, height) = (w.width as usize, w.height as usize);
    let r = radius as usize;
    let pw = width + 2 * r;
    let circle = circle(radius);
    let mut output = vec![0u8; width * height];
    output
        .par_chunks_mut(width)
        .enumerate()
        .with_min_len(min_rows(width, 2 * r + 1))
        .for_each_init(
            || vec![0u8; (r + 1) * pw],
            |columns, (y, out)| {
                let centre = (y + r) * pw;
                columns[..pw].copy_from_slice(&padded[centre..centre + pw]);
                for j in 1..=r {
                    let above = &padded[centre + j * pw..centre + j * pw + pw];
                    let below = &padded[centre - j * pw..centre - j * pw + pw];
                    let (prev, rest) = columns.split_at_mut(j * pw);
                    let previous = &prev[(j - 1) * pw..];
                    let row = &mut rest[..pw];
                    if max {
                        for x in 0..pw {
                            row[x] = previous[x].max(above[x].max(below[x]));
                        }
                    } else {
                        for x in 0..pw {
                            row[x] = previous[x].min(above[x].min(below[x]));
                        }
                    }
                }
                for (x, o) in out.iter_mut().enumerate() {
                    let mut v = if max { 0u8 } else { 255u8 };
                    for (i, &h) in circle.iter().enumerate() {
                        let c = columns[h * pw + x + i];
                        if if max { c > v } else { c < v } {
                            v = c;
                            if if max { v == 255 } else { v == 0 } {
                                break;
                            }
                        }
                    }
                    *o = v;
                }
            },
        );
    output
}

/// 正規化したガウスの核（C# の GaussianKernel と同じ演算の順）。`exp` は libm に依るので、別の libm では 1 ULP ずれ得る（一致を
/// 確かめたのは Linux の glibc の上）。
fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let reach = 1.max((3.0 * sigma).ceil() as i64);
    let mut k = vec![0.0; (2 * reach + 1) as usize];
    let mut sum = 0.0;
    for i in -reach..=reach {
        let v = (-(i * i) as f64 / (2.0 * sigma * sigma)).exp();
        k[(i + reach) as usize] = v;
        sum += v;
    }
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// 横の畳み込み（端は配列の端の値。行ごとに並列）。
fn convolve_rows(src: &[f32], dst: &mut [f32], w: usize, kernel: &[f64]) {
    let reach = (kernel.len() / 2) as i64;
    dst.par_chunks_mut(w)
        .enumerate()
        .with_min_len(min_rows(w, kernel.len()))
        .for_each(|(y, out)| {
            let line = &src[y * w..y * w + w];
            for (x, o) in out.iter_mut().enumerate() {
                let mut s = 0.0f64;
                for i in -reach..=reach {
                    let xx = (x as i64 + i).clamp(0, w as i64 - 1) as usize;
                    s += line[xx] as f64 * kernel[(i + reach) as usize];
                }
                *o = s as f32;
            }
        });
}

/// 縦の畳み込み（端は配列の端の値。行ごとに並列）。
fn convolve_columns(src: &[f32], dst: &mut [f32], w: usize, h: usize, kernel: &[f64]) {
    let reach = (kernel.len() / 2) as i64;
    dst.par_chunks_mut(w)
        .enumerate()
        .with_min_len(min_rows(w, kernel.len()))
        .for_each(|(y, out)| {
            for (x, o) in out.iter_mut().enumerate() {
                let mut s = 0.0f64;
                for i in -reach..=reach {
                    let yy = (y as i64 + i).clamp(0, h as i64 - 1) as usize;
                    s += src[yy * w + x] as f64 * kernel[(i + reach) as usize];
                }
                *o = s as f32;
            }
        });
}

/// 3 回の箱ぼかしでガウスに近づける箱の幅（W. Jarosz / P. Kovesi。C# の BoxesForGauss）。
fn boxes_for_gauss(sigma: f64, n: i64) -> Vec<i64> {
    let ideal = (12.0 * sigma * sigma / n as f64 + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal =
        (12.0 * sigma * sigma - (n * wl * wl) as f64 - (4 * n * wl) as f64 - (3 * n) as f64)
            / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round_ties_even() as i64;
    (0..n).map(|i| if i < m { wl } else { wu }).collect()
}

/// 1 本の線の、半幅 r の累積和の箱ぼかし（C# の Box の 1 本分。窓は配列の端で止める）。sum は double、配列は float で、
/// 足し引きは float の差を double に足す（C# の `sum += src[a] - src[b]`）。
#[inline]
fn box_line(get: impl Fn(usize) -> f32, mut put: impl FnMut(usize, f32), length: usize, r: usize) {
    let scale = 1.0 / (2 * r + 1) as f64;
    let mut sum = 0.0f64;
    let last = length as i64 - 1;
    for i in -(r as i64)..=r as i64 {
        sum += get(i.clamp(0, last) as usize) as f64;
    }
    for i in 0..length {
        put(i, (sum * scale) as f32);
        let add = (i + r + 1).min(length - 1);
        let remove = i.saturating_sub(r);
        sum += (get(add) - get(remove)) as f64;
    }
}

/// 横の箱ぼかし（行ごとに並列）。
fn box_rows(src: &[f32], dst: &mut [f32], w: usize, r: usize) {
    dst.par_chunks_mut(w)
        .enumerate()
        .with_min_len(min_rows(w, 3))
        .for_each(|(y, out)| {
            let line = &src[y * w..y * w + w];
            box_line(|i| line[i], |i, v| out[i] = v, w, r);
        });
}

/// 縦の箱ぼかし: 列の束ごとに並列に、束の中の列を行の順に同時に進め（各列の和は C# と同じ順に溜まる）、束の結果を後で
/// 行へ書き戻す（safe なまま列を分けて書くため。blocks は束の場所で、呼び手が使い回す）。
fn box_columns(
    src: &[f32],
    dst: &mut [f32],
    w: usize,
    h: usize,
    r: usize,
    blocks: &mut Vec<Vec<f32>>,
) {
    const BLOCK: usize = 64;
    let count = w.div_ceil(BLOCK);
    blocks.resize_with(count, Vec::new);
    let scale = 1.0 / (2 * r + 1) as f64;
    let last = h as i64 - 1;
    blocks.par_iter_mut().enumerate().for_each(|(k, block)| {
        let x0 = k * BLOCK;
        let bw = BLOCK.min(w - x0);
        block.clear();
        block.resize(bw * h, 0.0);
        let mut sums = [0.0f64; BLOCK];
        for i in -(r as i64)..=r as i64 {
            let row = i.clamp(0, last) as usize * w + x0;
            for c in 0..bw {
                sums[c] += src[row + c] as f64;
            }
        }
        for i in 0..h {
            let add = (i + r + 1).min(h - 1) * w + x0;
            let remove = i.saturating_sub(r) * w + x0;
            for c in 0..bw {
                block[i * bw + c] = (sums[c] * scale) as f32;
                sums[c] += (src[add + c] - src[remove + c]) as f64;
            }
        }
    });
    dst.par_chunks_mut(w)
        .enumerate()
        .with_min_len(min_rows(w, 1))
        .for_each(|(y, out)| {
            for (k, block) in blocks.iter().enumerate() {
                let x0 = k * BLOCK;
                let bw = BLOCK.min(w - x0);
                out[x0..x0 + bw].copy_from_slice(&block[y * bw..y * bw + bw]);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 既定の作業の予算（1 GiB）で、8192² の全面の選択範囲にどこまで入るか（selection の文書の言い分と同じ）。
    #[test]
    fn the_default_budget_fits_a_full_8192_canvas_up_to_these_radii() {
        let budget = crate::selection::DEFAULT_WORKING_BUDGET_BYTES as u128;
        let w = Window {
            x: 0,
            y: 0,
            width: 8192,
            height: 8192,
        };
        let feather = |radius: f64| {
            let sigma = radius / 3.5;
            let b = boxes_for_gauss(sigma, 3);
            feather_bytes(w, (b[0] + b[1] + b[2] - 3) / 2)
        };
        assert!(feather(100.0) <= budget, "{}", feather(100.0));
        assert!(feather(200.0) > budget, "{}", feather(200.0));
        // 拡張・縮小は 200 まで入る（スレッドが 64 本でも）
        assert!(morphology_bytes(w, 200, 64) <= budget);
    }

    #[test]
    fn gimps_circle_for_radius_three() {
        // GIMP の compute_border: dx=0 は 3、|dx|=1,2 は rint(√(9−0.25))=3・rint(√(9−2.25))=3、|dx|=3 は rint(√(9−6.25))=2
        assert_eq!(circle(3), vec![2, 3, 3, 3, 3, 3, 2]);
    }

    #[test]
    fn column_box_blur_matches_the_line_by_line_form() {
        let (w, h, r) = (150usize, 37usize, 5usize);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 256) as f32).collect();
        let mut dst = vec![0f32; w * h];
        let mut blocks = Vec::new();
        box_columns(&src, &mut dst, w, h, r, &mut blocks);
        for x in 0..w {
            let mut expected = vec![0f32; h];
            box_line(|i| src[i * w + x], |i, v| expected[i] = v, h, r);
            for y in 0..h {
                assert_eq!(dst[y * w + x].to_bits(), expected[y].to_bits(), "{x},{y}");
            }
        }
    }
}
