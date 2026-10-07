//! 0.5.0 の近傍のフィルター（スロープぼかし・方向のぼかし・ゆがみ・モルフォロジー・エッジ検出・ハイパス・メディアン・グロー）。
//!
//! - スカラーの 1 本の道（SIMD にしない）なので、`YOLU_SIMD` の道によらず同じバイト。近傍の足し込みは f64（双線形・平均）か整数
//!   （箱ぼかし・度数）。
//! - 入力は矩形 cur の straight RGBA8、出力は矩形 next（cur = grow(next, halo)）。画像の外は端の画素を繰り返す（読む位置を画像の中へ寄せる）。
//! - どのブロックの分け方でも同じ画素は同じ値: 読む位置は halo の中に寄せ、双線形の 2 つ目の画素が矩形の外に出るのは重みが 0 のときだけ。
//! - 混ぜる・平均する段（スロープぼかし・方向のぼかし・ゆがみ）は乗算済みで読み、straight へ戻す。結果の A が 0 になる画素と、A を変えない段
//!   （モルフォロジー・エッジ検出・ハイパス・グロー）の透明な画素は、入力の RGB のまま。
//! - 強さは `pixels::mix`（A が変わらなければ RGB の線形の混ぜ）。

use super::pixels::{gaussian_q, mix, Check};
use super::rows::{blur_pixel, premultiply_at};
use super::{area, grow, zeros, Error, MorphologyMode, Rect, Settings, SlopeMode, Stage};
use crate::generator::{sin_cos_deg, value2_gradient};
use crate::math::{clamp01, to_byte, UNIT};

/// メディアンの度数（4 チャンネル × 256 × u32）。
const HISTOGRAM_BYTES: u64 = 4 * 256 * 4;
/// 値のノイズの勾配の、格子の単位での最大（fade の微分の最大 1.875 × 角の値の差の最大 1）。ゆがみのずれを −1〜1 にそろえる。
const MAX_SLOPE: f64 = 1.875;
/// 方向のぼかしの取る数の上限。
const MAX_LINE_SAMPLES: u32 = 512;

/// 1 ブロックの作業バイトの見積り（`block_working_bytes` の段ごとの最大）。`input`・`output` は段の入力・出力の画素の数、
/// `input_width` は入力の幅、`row_sums` はぼかしの縦の行の累積のバイト。
pub(super) fn working_bytes(
    settings: &Settings,
    input: u64,
    output: u64,
    input_width: u64,
    row_sums: u64,
) -> u64 {
    match settings {
        // 入力・出力と、乗算済みの u16 × 4
        Settings::SlopeBlur { .. } | Settings::DirectionalBlur { .. } | Settings::Warp { .. } => {
            input * 12 + output * 4
        }
        // 入力・出力と、行ごとの最大・最小の表（3 チャンネル × 段の数）
        Settings::Morphology { .. } => {
            input * (4 + 3 * u64::from(levels(input_width as usize))) + output * 4
        }
        // ぼかし（`gaussian_q`）と同じ作業に、ぼかした画素（グローは明るい所の画素）を 1 枚
        Settings::EdgeDetect { .. } | Settings::HighPass { .. } | Settings::Glow { .. } => {
            input * 32 + output * 4 + row_sums
        }
        // 入力・出力と、チャンネルごとの度数
        _ => input * 4 + output * 4 + HISTOGRAM_BYTES,
    }
}

/// 最大・最小の表の段の数（幅 `width` の区間を 2 つの段の値で引ける数: floor(log2(width)) + 1）。
fn levels(width: usize) -> u32 {
    usize::BITS - width.max(1).leading_zeros()
}

/// 近傍の段 1 つ（強さの混ぜを含む）。
#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate(
    buf: &[u8],
    cur: Rect,
    next: Rect,
    w: u32,
    h: u32,
    s: &Stage,
    _ty: super::ValueType,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let t = s.strength;
    let image = Image { buf, r: cur, w, h };
    match s.settings {
        Settings::SlopeBlur {
            intensity,
            samples,
            mode,
            scale,
            seed,
        } => slope_blur(
            &image,
            next,
            Slope {
                intensity,
                samples,
                mode,
                scale,
                seed: seed as u32,
            },
            t,
            check,
        ),
        Settings::DirectionalBlur { angle, distance } => {
            directional_blur(&image, next, angle, distance, t, check)
        }
        Settings::Warp {
            intensity,
            scale,
            seed,
        } => warp(&image, next, intensity, scale, seed as u32, t, check),
        Settings::Morphology { mode, radius } => morphology(&image, next, mode, radius, t, check),
        Settings::EdgeDetect { width, threshold } => {
            edge_detect(&image, next, width, threshold, t, check)
        }
        Settings::HighPass { radius } => high_pass(&image, next, radius, t, check),
        Settings::Median { radius } => median(&image, next, radius, t, check),
        Settings::Glow {
            threshold,
            radius,
            intensity,
        } => glow(&image, next, threshold, radius, intensity, t, check),
        _ => unreachable!("近傍の段だけ"),
    }
}

/// 段の入力（矩形 r の straight RGBA8）と画像の大きさ。
struct Image<'a> {
    buf: &'a [u8],
    r: Rect,
    w: u32,
    h: u32,
}

impl Image<'_> {
    fn index(&self, x: u32, y: u32) -> usize {
        (y - self.r.y) as usize * self.r.width as usize + (x - self.r.x) as usize
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.index(x, y) * 4;
        self.buf[i..i + 4].try_into().unwrap()
    }
    /// 乗算済み（`rows::premultiply_at`: R・G・B は C × A、A は A × 255）の写し。
    fn premultiplied(&self) -> Result<Vec<u16>, Error> {
        let mut q = zeros::<u16>(self.buf.len())?;
        premultiply_at(crate::math::simd::level(), self.buf, &mut q);
        Ok(q)
    }
    /// 読む位置 p（画素 x を中心に ±reach の中）を、その範囲と画像の中へ寄せる。
    fn place(&self, p: f64, center: u32, reach: f64, size: u32) -> f64 {
        let c = f64::from(center);
        p.clamp(c - reach, c + reach)
            .clamp(0.0, f64::from(size - 1))
    }
}

/// 乗算済みの u16 × 4 を双線形で読む。読む位置は呼び手が halo の中と画像の中へ寄せてある。2 つ目の画素が矩形の外に出るのは、
/// 位置がちょうど端の整数で重みが 0 のときだけなので、矩形の端へ寄せても値は変わらない。
struct Premultiplied<'a> {
    q: &'a [u16],
    r: Rect,
}

impl Premultiplied<'_> {
    fn sample(&self, px: f64, py: f64) -> [f64; 4] {
        let (fx, fy) = (px.floor(), py.floor());
        let (tx, ty) = (px - fx, py - fy);
        let clamp_x = |x: f64| {
            (x.max(f64::from(self.r.x)) as u32).min(self.r.x + self.r.width - 1) - self.r.x
        };
        let clamp_y = |y: f64| {
            (y.max(f64::from(self.r.y)) as u32).min(self.r.y + self.r.height - 1) - self.r.y
        };
        let (xa, xb) = (clamp_x(fx), clamp_x(fx + 1.0));
        let (ya, yb) = (clamp_y(fy), clamp_y(fy + 1.0));
        let stride = self.r.width as usize;
        let at = |x: u32, y: u32| (y as usize * stride + x as usize) * 4;
        let (i00, i10, i01, i11) = (at(xa, ya), at(xb, ya), at(xa, yb), at(xb, yb));
        let mut out = [0.0; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let top = f64::from(self.q[i00 + c]) * (1.0 - tx) + f64::from(self.q[i10 + c]) * tx;
            let bottom = f64::from(self.q[i01 + c]) * (1.0 - tx) + f64::from(self.q[i11 + c]) * tx;
            *o = top * (1.0 - ty) + bottom * ty;
        }
        out
    }
}

/// 乗算済みの値（R・G・B は C × A、A は A × 255 の 0〜65025）を straight の RGBA8 へ。A が 0 に丸まれば入力の RGB のまま。
fn unpremultiply(p: [f64; 4], input: [u8; 4]) -> [u8; 4] {
    let a = to_byte(p[3] / 65025.0);
    if a == 0 {
        return [input[0], input[1], input[2], 0];
    }
    [
        to_byte(p[0] / p[3]),
        to_byte(p[1] / p[3]),
        to_byte(p[2] / p[3]),
        a,
    ]
}

/// 出力の矩形を行ごとに埋める（行の頭で取り消しを確かめる）。`f` は画素の結果（強さの混ぜの前）。
fn each_pixel(
    image: &Image<'_>,
    next: Rect,
    t: f64,
    check: Check<'_>,
    mut f: impl FnMut(u32, u32, [u8; 4]) -> [u8; 4],
) -> Result<Vec<u8>, Error> {
    let mut out = zeros::<u8>(area(next) * 4)?;
    for (row, line) in out.chunks_exact_mut(next.width as usize * 4).enumerate() {
        check()?;
        let y = next.y + row as u32;
        for (col, o) in line.chunks_exact_mut(4).enumerate() {
            let x = next.x + col as u32;
            let input = image.pixel(x, y);
            let result = f(x, y, input);
            o.copy_from_slice(&mix(input, result, t, false));
        }
    }
    Ok(out)
}

// ───────── スロープぼかし・方向のぼかし・ゆがみ ─────────

struct Slope {
    intensity: f64,
    samples: u32,
    mode: SlopeMode,
    scale: f64,
    seed: u32,
}

/// 画素 (x, y) の中心での内蔵の値ノイズの勾配（格子の単位）。
fn slope_at(x: u32, y: u32, scale: f64, seed: u32) -> [f64; 2] {
    value2_gradient(
        (f64::from(x) + 0.5) / scale,
        (f64::from(y) + 0.5) / scale,
        seed,
    )
    .1
}

fn slope_blur(
    image: &Image<'_>,
    next: Rect,
    s: Slope,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let q = image.premultiplied()?;
    let src = Premultiplied { q: &q, r: image.r };
    let reach = s.intensity;
    each_pixel(image, next, t, check, |x, y, input| {
        let g = slope_at(x, y, s.scale, s.seed);
        let len = (g[0] * g[0] + g[1] * g[1]).sqrt();
        if len < 1e-12 {
            return input; // 平らな所は向きが無い
        }
        let (dx, dy) = (g[0] / len, g[1] / len);
        let at = |k: u32| {
            let d = s.intensity * f64::from(k) / f64::from(s.samples);
            (
                image.place(f64::from(x) + dx * d, x, reach, image.w),
                image.place(f64::from(y) + dy * d, y, reach, image.h),
            )
        };
        match s.mode {
            SlopeMode::Blur => {
                // 自分と、勾配の向きの samples 個の平均（乗算済み）
                let i = image.index(x, y) * 4;
                let mut sum = [0.0; 4];
                for (c, v) in sum.iter_mut().enumerate() {
                    *v = f64::from(q[i + c]);
                }
                for k in 1..=s.samples {
                    let (px, py) = at(k);
                    let p = src.sample(px, py);
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                }
                let n = f64::from(s.samples + 1);
                unpremultiply(sum.map(|v| v / n), input)
            }
            SlopeMode::Min | SlopeMode::Max => {
                // チャンネルごとの最小・最大（RGB は A が正の所だけ。A は全部）
                let max = s.mode == SlopeMode::Max;
                let pick = |a: f64, b: f64| if max { a.max(b) } else { a.min(b) };
                let mut alpha = UNIT[input[3] as usize];
                let mut rgb: Option<[f64; 3]> = (input[3] > 0).then(|| {
                    [
                        UNIT[input[0] as usize],
                        UNIT[input[1] as usize],
                        UNIT[input[2] as usize],
                    ]
                });
                for k in 1..=s.samples {
                    let (px, py) = at(k);
                    let p = src.sample(px, py);
                    alpha = pick(alpha, p[3] / 65025.0);
                    if p[3] > 0.0 {
                        let v = [p[0] / p[3], p[1] / p[3], p[2] / p[3]];
                        rgb = Some(match rgb {
                            None => v,
                            Some(r) => [pick(r[0], v[0]), pick(r[1], v[1]), pick(r[2], v[2])],
                        });
                    }
                }
                let a = to_byte(alpha);
                match rgb {
                    Some(v) if a > 0 => [to_byte(v[0]), to_byte(v[1]), to_byte(v[2]), a],
                    _ => [input[0], input[1], input[2], a],
                }
            }
        }
    })
}

fn directional_blur(
    image: &Image<'_>,
    next: Rect,
    angle: f64,
    distance: f64,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let q = image.premultiplied()?;
    let src = Premultiplied { q: &q, r: image.r };
    let (sin, cos) = sin_cos_deg(angle);
    // 両側へ distance、取る数は長さに比例（両端と中心を含む奇数。上限 512）
    let n = (2 * (distance.ceil() as u32) + 1).min(MAX_LINE_SAMPLES);
    each_pixel(image, next, t, check, |x, y, input| {
        let mut sum = [0.0; 4];
        for k in 0..n {
            let d = -distance + 2.0 * distance * f64::from(k) / f64::from(n - 1);
            let px = image.place(f64::from(x) + cos * d, x, distance, image.w);
            let py = image.place(f64::from(y) + sin * d, y, distance, image.h);
            let p = src.sample(px, py);
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        let n = f64::from(n);
        unpremultiply(sum.map(|v| v / n), input)
    })
}

fn warp(
    image: &Image<'_>,
    next: Rect,
    intensity: f64,
    scale: f64,
    seed: u32,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let q = image.premultiplied()?;
    let src = Premultiplied { q: &q, r: image.r };
    each_pixel(image, next, t, check, |x, y, input| {
        let g = slope_at(x, y, scale, seed);
        // 勾配を −1〜1 にそろえて intensity 画素ずらす（長さは intensity まで）
        let (mut ox, mut oy) = (g[0] / MAX_SLOPE * intensity, g[1] / MAX_SLOPE * intensity);
        let len = (ox * ox + oy * oy).sqrt();
        if len > intensity {
            ox *= intensity / len;
            oy *= intensity / len;
        }
        let px = image.place(f64::from(x) + ox, x, intensity, image.w);
        let py = image.place(f64::from(y) + oy, y, intensity, image.h);
        unpremultiply(src.sample(px, py), input)
    })
}

// ───────── モルフォロジー ─────────

fn morphology(
    image: &Image<'_>,
    next: Rect,
    mode: MorphologyMode,
    radius: u32,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let dilate = mode == MorphologyMode::Dilate;
    // 透明な画素はウィンドウに数えない（最大なら 0、最小なら 255 として引く）
    let neutral = if dilate { 0u8 } else { 255 };
    let pick = |a: u8, b: u8| if dilate { a.max(b) } else { a.min(b) };
    let (cw, ch) = (image.r.width as usize, image.r.height as usize);
    let count = levels(cw) as usize;
    // 段 k の値は、行の x から 2^k 画素の最大・最小（RGB の 3 つ）
    let plane = cw * ch * 3;
    let mut table = zeros::<u8>(plane * count)?;
    for (i, p) in image.buf.chunks_exact(4).enumerate() {
        for c in 0..3 {
            table[i * 3 + c] = if p[3] == 0 { neutral } else { p[c] };
        }
    }
    for k in 1..count {
        let half = 1usize << (k - 1);
        let (done, rest) = table.split_at_mut(plane * k);
        let prev = &done[plane * (k - 1)..];
        let cur = &mut rest[..plane];
        for y in 0..ch {
            check()?;
            for x in 0..cw.saturating_sub((1 << k) - 1) {
                let a = (y * cw + x) * 3;
                let b = (y * cw + x + half) * 3;
                for c in 0..3 {
                    cur[a + c] = pick(prev[a + c], prev[b + c]);
                }
            }
        }
    }
    // 丸いウィンドウの行ごとの半幅（整数の平方根）
    let r = i64::from(radius);
    let widths: Vec<i64> = (0..=r)
        .map(|dy| {
            let mut x = ((r * r - dy * dy) as f64).sqrt() as i64;
            while x * x > r * r - dy * dy {
                x -= 1;
            }
            while (x + 1) * (x + 1) <= r * r - dy * dy {
                x += 1;
            }
            x
        })
        .collect();
    let (w, h) = (i64::from(image.w), i64::from(image.h));
    each_pixel(image, next, t, check, |x, y, input| {
        if input[3] == 0 {
            return input;
        }
        let mut best = [neutral; 3];
        for dy in -r..=r {
            // 画像の外の行は、端の行の（より広い）ウィンドウに含まれる
            let yy = i64::from(y) + dy;
            if !(0..h).contains(&yy) {
                continue;
            }
            let half = widths[dy.unsigned_abs() as usize];
            let lo = (i64::from(x) - half).max(0) as u32 - image.r.x;
            let hi = (i64::from(x) + half).min(w - 1) as u32 - image.r.x;
            let row = (yy as u32 - image.r.y) as usize;
            let span = (hi - lo + 1) as usize;
            let k = (usize::BITS - 1 - span.leading_zeros()) as usize;
            let a = (row * cw + lo as usize) * 3 + plane * k;
            let b = (row * cw + hi as usize + 1 - (1 << k)) * 3 + plane * k;
            for c in 0..3 {
                best[c] = pick(best[c], pick(table[a + c], table[b + c]));
            }
        }
        [best[0], best[1], best[2], input[3]]
    })
}

// ───────── ぼかしを使う段（エッジ検出・ハイパス・グロー） ─────────

fn edge_detect(
    image: &Image<'_>,
    next: Rect,
    width: u32,
    threshold: f64,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let (w, h) = (image.w, image.h);
    // width でぼかし、そのまわり 1 画素で Sobel
    let mid = grow(next, 1, w, h);
    let q = gaussian_q(image.premultiplied()?, image.r, mid, width, w, h, check)?;
    let mut blurred = zeros::<u8>(area(mid) * 4)?;
    for (i, b) in blurred.chunks_exact_mut(4).enumerate() {
        let (x, y) = (
            mid.x + (i % mid.width as usize) as u32,
            mid.y + (i / mid.width as usize) as u32,
        );
        b.copy_from_slice(&blur_pixel(image.pixel(x, y), &q[i * 4..], 1.0));
    }
    let blurred = Image {
        buf: &blurred,
        r: mid,
        w,
        h,
    };
    each_pixel(image, next, t, check, |x, y, input| {
        if input[3] == 0 {
            return input;
        }
        let at = |dx: i64, dy: i64| {
            let xx = (i64::from(x) + dx).clamp(0, i64::from(w) - 1) as u32;
            let yy = (i64::from(y) + dy).clamp(0, i64::from(h) - 1) as u32;
            blurred.pixel(xx, yy)
        };
        let (p00, p10, p20) = (at(-1, -1), at(0, -1), at(1, -1));
        let (p01, p21) = (at(-1, 0), at(1, 0));
        let (p02, p12, p22) = (at(-1, 1), at(0, 1), at(1, 1));
        let mut out = input;
        for c in 0..3 {
            let v = |p: [u8; 4]| i32::from(p[c]);
            let gx = (v(p20) + 2 * v(p21) + v(p22)) - (v(p00) + 2 * v(p01) + v(p02));
            let gy = (v(p02) + 2 * v(p12) + v(p22)) - (v(p00) + 2 * v(p10) + v(p20));
            // 段差 1 の縁で 1（Sobel の重みの和 4 × 255）
            let m = clamp01(f64::from(gx * gx + gy * gy).sqrt() / 1020.0);
            out[c] = if m <= threshold { 0 } else { to_byte(m) };
        }
        out
    })
}

fn high_pass(
    image: &Image<'_>,
    next: Rect,
    radius: u32,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let q = gaussian_q(
        image.premultiplied()?,
        image.r,
        next,
        radius,
        image.w,
        image.h,
        check,
    )?;
    let stride = next.width as usize;
    each_pixel(image, next, t, check, |x, y, input| {
        if input[3] == 0 {
            return input;
        }
        let i = ((y - next.y) as usize * stride + (x - next.x) as usize) * 4;
        let b = blur_pixel(input, &q[i..], 1.0);
        let mut out = input;
        for c in 0..3 {
            out[c] = to_byte(clamp01(0.5 + UNIT[input[c] as usize] - UNIT[b[c] as usize]));
        }
        out
    })
}

fn glow(
    image: &Image<'_>,
    next: Rect,
    threshold: f64,
    radius: u32,
    intensity: f64,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    // しきい値を超えた分（A は元のまま）をぼかし、乗算済みの値（覆いの分だけ弱まる）を足す
    let mut bright = zeros::<u8>(image.buf.len())?;
    for (b, p) in bright.chunks_exact_mut(4).zip(image.buf.chunks_exact(4)) {
        for c in 0..3 {
            b[c] = to_byte((UNIT[p[c] as usize] - threshold).max(0.0));
        }
        b[3] = p[3];
    }
    let bright = Image {
        buf: &bright,
        r: image.r,
        w: image.w,
        h: image.h,
    };
    let q = gaussian_q(
        bright.premultiplied()?,
        image.r,
        next,
        radius,
        image.w,
        image.h,
        check,
    )?;
    let stride = next.width as usize;
    each_pixel(image, next, t, check, |x, y, input| {
        if input[3] == 0 {
            return input;
        }
        let i = ((y - next.y) as usize * stride + (x - next.x) as usize) * 4;
        let mut out = input;
        for c in 0..3 {
            let g = f64::from(q[i + c]) / 65025.0;
            out[c] = to_byte(UNIT[input[c] as usize] + g * intensity);
        }
        out
    })
}

// ───────── メディアン ─────────

/// チャンネルごとの度数（RGB は A が正の画素だけ、A は全部）。
struct Histogram {
    rgb: [[u32; 256]; 3],
    alpha: [u32; 256],
    opaque: u32,
    count: u32,
}

impl Histogram {
    fn add(&mut self, p: [u8; 4], n: i32) {
        let step = |v: &mut u32| *v = v.wrapping_add_signed(n);
        step(&mut self.alpha[p[3] as usize]);
        step(&mut self.count);
        if p[3] > 0 {
            for (bins, v) in self.rgb.iter_mut().zip(p) {
                step(&mut bins[v as usize]);
            }
            step(&mut self.opaque);
        }
    }
    /// 小さい方から (n − 1) / 2 番目の値（下の中央値）。
    fn median(bins: &[u32; 256], n: u32) -> u8 {
        let k = (n - 1) / 2;
        let mut seen = 0;
        for (v, b) in bins.iter().enumerate() {
            seen += b;
            if seen > k {
                return v as u8;
            }
        }
        255
    }
}

fn median(
    image: &Image<'_>,
    next: Rect,
    radius: u32,
    t: f64,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let r = i64::from(radius);
    let (w, h) = (i64::from(image.w), i64::from(image.h));
    let clamp = |v: i64, n: i64| v.clamp(0, n - 1) as u32;
    let mut out = zeros::<u8>(area(next) * 4)?;
    let mut hist = Box::new(Histogram {
        rgb: [[0; 256]; 3],
        alpha: [0; 256],
        opaque: 0,
        count: 0,
    });
    for (row, line) in out.chunks_exact_mut(next.width as usize * 4).enumerate() {
        check()?;
        let y = i64::from(next.y) + row as i64;
        let column = |hist: &mut Histogram, x: i64, n: i32| {
            for dy in -r..=r {
                hist.add(image.pixel(clamp(x, w), clamp(y + dy, h)), n);
            }
        };
        *hist = Histogram {
            rgb: [[0; 256]; 3],
            alpha: [0; 256],
            opaque: 0,
            count: 0,
        };
        let x0 = i64::from(next.x);
        for dx in -r..=r {
            column(&mut hist, x0 + dx, 1);
        }
        for (col, o) in line.chunks_exact_mut(4).enumerate() {
            let x = x0 + col as i64;
            if col > 0 {
                column(&mut hist, x - 1 - r, -1);
                column(&mut hist, x + r, 1);
            }
            let input = image.pixel(x as u32, y as u32);
            let a = Histogram::median(&hist.alpha, hist.count);
            let result = if a == 0 || hist.opaque == 0 {
                [input[0], input[1], input[2], a]
            } else {
                [
                    Histogram::median(&hist.rgb[0], hist.opaque),
                    Histogram::median(&hist.rgb[1], hist.opaque),
                    Histogram::median(&hist.rgb[2], hist.opaque),
                    a,
                ]
            };
            o.copy_from_slice(&mix(input, result, t, false));
        }
    }
    Ok(out)
}
