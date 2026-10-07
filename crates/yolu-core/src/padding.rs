//! 書き出しのパディング（UV の外への塗り広げ。Substance Painter の書き出しの「Dilation」）。
//!
//! UV の三角形が少しでも重なるテクセル（[`coverage`]）はそのまま残し、その外のテクセルを、外へ 1 テクセルずつ広げながら、すでに埋まった
//! 8 近傍の色で埋める（[`dilate`]）。Unity がミップマップで縮めたり、バイリニアで UV の境目の外を読んだりしても、背景（透明や黒）が
//! にじまないようにするため。正本は変えない。書き出す画像（straight RGBA8、行は下から上）を作り直すだけ。
//!
//! Unity 版の `TexturePadding`（`Runtime/Core/TexturePadding.cs`）と同じ結果を、並列で計算する。結果は並列の度合い（スレッド数・
//! 分け方）に依らない: 覆いは三角形ごとの重なりの OR で、塗り広げは段ごとに前の段までの結果だけを読む。
//!
//! 変わった所だけを塗り広げ直す口（[`Rings`]）: 段の地図（各テクセルを埋める段）を覆いから 1 度だけ作っておき、矩形 1 つを、その周りを
//! 段数だけ広げた入力から塗り広げる（[`Rings::dilate_region`]）。段 k のテクセルの色は、距離 k 以内のテクセルだけで決まるので、矩形の中は
//! 画像全体を [`dilate`] したときと同じ値になる（3D ビューの表示の写しが、描いたタイルの周りだけを塗り広げ直すのに使う）。

use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::export::ExportError;
use crate::glam::DVec2;
use crate::Rect;

/// 塗り広げの届く範囲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// この段数まで（0 は塗り広げない）。
    Texels(u32),
    /// 届くかぎり全部（Substance の「Infinite」）。
    Fill,
}

impl Reach {
    /// 設定の値（Unity 版の `PainterSettings.ExportPadding` と同じ: -1 は全部、0 以上は段数）から。-2 以下は断る。
    pub fn from_setting(value: i32) -> Result<Reach, ExportError> {
        match value {
            -1 => Ok(Reach::Fill),
            v if v >= 0 => Ok(Reach::Texels(v as u32)),
            _ => Err(ExportError::InvalidArgument("塗り広げの段数")),
        }
    }

    /// 設定の値へ（[`Reach::from_setting`] の逆。段数が `i32::MAX` を超えれば `i32::MAX`）。
    pub fn to_setting(self) -> i32 {
        match self {
            Reach::Fill => -1,
            Reach::Texels(t) => t.min(i32::MAX as u32) as i32,
        }
    }
}

/// 並列の分け方。どれも結果には効かない（試験が小さい値で境目を通すために [`coverage_tuned`]・[`dilate_tuned`] へ渡せる）。
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// 一度に並列へ回す三角形の数（メモリの上限）。
    pub triangle_batch: usize,
    /// 覆いを並列に塗る帯の高さ。
    pub band_rows: usize,
    /// 1 回に並列で色を計算する候補の数（一時の色の置き場の上限。4 バイト × この数）。
    pub color_chunk: usize,
    /// これより少ない候補は並列にしない（段が細い所の仕事の受け渡しを避ける）。
    pub parallel_min: usize,
}

impl Tuning {
    pub const DEFAULT: Tuning = Tuning {
        triangle_batch: 1 << 16,
        band_rows: 32,
        color_chunk: 1 << 18,
        parallel_min: 16384,
    };
}

/// 三角形の外接矩形（テクセルの範囲）と頂点。画像の外にはみ出した部分は切ってある。
struct Triangle {
    p: [f64; 6],
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

/// C# の `Math.Min`/`Math.Max`（NaN があれば NaN）。Rust の `f64::min` は NaN でない方を返す。
#[inline]
fn cs_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else if b < a {
        b
    } else if a == b {
        a
    } else {
        f64::NAN
    }
}
#[inline]
fn cs_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else if b > a {
        b
    } else if a == b {
        a
    } else {
        f64::NAN
    }
}

impl Triangle {
    /// 有限でない頂点を持つもの、画像に届かないものは None（飛ばす）。
    fn new(t: [DVec2; 3], w: usize, h: usize) -> Option<Triangle> {
        let p = [t[0].x, t[0].y, t[1].x, t[1].y, t[2].x, t[2].y];
        if p.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let min_x = cs_min(p[0], cs_min(p[2], p[4]));
        let max_x = cs_max(p[0], cs_max(p[2], p[4]));
        let min_y = cs_min(p[1], cs_min(p[3], p[5]));
        let max_y = cs_max(p[1], cs_max(p[3], p[5]));
        // 辺（整数の座標）の上の頂点は、隣のテクセルにも触れるので 1 つ手前から。±1e12 で止める（C# の int の範囲を超える座標でも、画像で切って扱う）
        let lo = |v: f64| -> i64 {
            (v.floor().clamp(-1.0e12, 1.0e12) as i64) - i64::from(v == v.floor())
        };
        let hi = |v: f64| -> i64 { v.floor().clamp(-1.0e12, 1.0e12) as i64 };
        let x0 = lo(min_x).max(0);
        let x1 = hi(max_x).min(w as i64 - 1);
        let y0 = lo(min_y).max(0);
        let y1 = hi(max_y).min(h as i64 - 1);
        if x0 > x1 || y0 > y1 {
            return None;
        }
        Some(Triangle {
            p,
            x0: x0 as usize,
            x1: x1 as usize,
            y0: y0 as usize,
            y1: y1 as usize,
        })
    }

    /// 3 辺の法線を軸にした分離の判定。
    fn axes(&self) -> [Axis; 3] {
        let [ax, ay, bx, by, cx, cy] = self.p;
        [
            Axis::new(ax, ay, bx, by, cx, cy),
            Axis::new(bx, by, cx, cy, ax, ay),
            Axis::new(cx, cy, ax, ay, bx, by),
        ]
    }
}

/// 辺 (a → b) の法線への射影: 三角形は [min, max] にあり、正方形の 4 隅の射影の範囲と重ならなければ分離している。
struct Axis {
    nx: f64,
    ny: f64,
    min: f64,
    max: f64,
    /// 正方形の隅（x, y）からの差の下限・上限（法線 (nx, ny) への射影の、0 と nx、0 と ny のそれぞれの小さい方・大きい方の和）。
    lo: (f64, f64),
    hi: (f64, f64),
    none: bool,
}

impl Axis {
    fn new(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> Axis {
        let nx = -(by - ay);
        let ny = bx - ax;
        let pa = nx * ax + ny * ay; // a と b は同じ値
        let pc = nx * cx + ny * cy;
        Axis {
            nx,
            ny,
            min: cs_min(pa, pc),
            max: cs_max(pa, pc),
            lo: (cs_min(0.0, nx), cs_min(0.0, ny)),
            hi: (cs_max(0.0, nx), cs_max(0.0, ny)),
            none: nx == 0.0 && ny == 0.0,
        }
    }

    #[inline]
    fn separates(&self, x: usize, y: usize) -> bool {
        if self.none {
            return false;
        }
        let p0 = self.nx * x as f64 + self.ny * y as f64;
        let lo = p0 + self.lo.0 + self.lo.1;
        let hi = p0 + self.hi.0 + self.hi.1;
        hi < self.min || lo > self.max
    }
}

/// UV の三角形（画素の座標、左下が原点。UV × 大きさ）が少しでも重なるテクセルの印（`[y * width + x]`）。テクセル (x, y) は正方形
/// [x, x+1] × [y, y+1]。重なりは分離軸（x・y と三角形の 3 辺の法線）で判定する（保守的: 角に触れるだけでも重なりとする）。
/// 塗った画素は UV の境目の上のテクセル（三角形が一部だけ覆う）にも入るので、それを書き出しで塗り広げの色に置き換えないため。
/// 画像の外にはみ出した部分は切る（UV の繰り返しは見ない。int の範囲を超える座標も切って扱う）。NaN や無限大を含む三角形は飛ばす。
pub fn coverage(
    width: u32,
    height: u32,
    triangles: impl IntoIterator<Item = [DVec2; 3]>,
) -> Result<Vec<bool>, ExportError> {
    coverage_tuned(width, height, triangles, Tuning::DEFAULT)
}

/// [`coverage`] の並列の分け方を選ぶもの（試験用。結果は同じ）。
#[doc(hidden)]
pub fn coverage_tuned(
    width: u32,
    height: u32,
    triangles: impl IntoIterator<Item = [DVec2; 3]>,
    tuning: Tuning,
) -> Result<Vec<bool>, ExportError> {
    if width == 0 || height == 0 {
        return Err(ExportError::InvalidArgument("大きさ"));
    }
    let (w, h) = (width as usize, height as usize);
    let mut covered = vec![false; w * h];
    let mut batch: Vec<Triangle> = Vec::new();
    for t in triangles {
        if let Some(tri) = Triangle::new(t, w, h) {
            batch.push(tri);
            if batch.len() >= tuning.triangle_batch.max(1) {
                mark(&mut covered, w, h, &batch, tuning.band_rows.max(1));
                batch.clear();
            }
        }
    }
    mark(&mut covered, w, h, &batch, tuning.band_rows.max(1));
    Ok(covered)
}

/// 三角形の束を、帯ごとに並列で覆いへ書く（各テクセルは自分の帯の仕事だけが書く）。
fn mark(covered: &mut [bool], w: usize, h: usize, batch: &[Triangle], band_rows: usize) {
    if batch.is_empty() {
        return;
    }
    let bands = h.div_ceil(band_rows);
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); bands];
    for (i, t) in batch.iter().enumerate() {
        for bucket in &mut buckets[t.y0 / band_rows..=t.y1 / band_rows] {
            bucket.push(i as u32);
        }
    }
    covered
        .par_chunks_mut(w * band_rows)
        .zip(buckets.par_iter())
        .enumerate()
        .for_each(|(band, (rows, list))| {
            let base = band * band_rows;
            let last = base + rows.len() / w - 1;
            for &i in list {
                let t = &batch[i as usize];
                let axes = t.axes();
                for y in t.y0.max(base)..=t.y1.min(last) {
                    let row = &mut rows[(y - base) * w..(y - base + 1) * w];
                    for (k, cell) in row[t.x0..=t.x1].iter_mut().enumerate() {
                        if *cell || axes.iter().any(|a| a.separates(t.x0 + k, y)) {
                            continue;
                        }
                        *cell = true;
                    }
                }
            }
        });
}

/// [`dilate`] の作業のバイト数: 出力の写し・段の印（i32）・段の候補の並び（どちらも 4 バイトの添え字で、合わせて画素数以下）で、
/// 1 画素あたり 12 バイト（Unity 版と同じ見積もり。色の計算の一時の置き場（1 MiB 余り）は含めない）。
pub fn working_bytes(width: u32, height: u32) -> u64 {
    12 * width as u64 * height as u64
}

/// `keep` の外のテクセルを、外へ 1 段ずつ埋めた新しい画像を返す（入力は変えない）。段 k で埋まるのは、段 k−1 までに埋まった 8 近傍を持つ
/// テクセルで、色はそれらの近傍の、アルファで重みを付けた色の平均（全部が透明なら色の平均）と、アルファの平均（どちらも四捨五入）。
/// 同じ段の中では前の段の結果だけを読むので、結果は処理の順にも並列の度合いにも依らない。`reach` 段で止める（[`Reach::Fill`] なら届くかぎり）。
/// 届かなかったテクセルは元のまま。`keep` が 1 つも無ければ何もしない（元の写しを返す）。作業のメモリ（[`working_bytes`]）が
/// `max_working_bytes` を超えるなら、確保の前に断る。
pub fn dilate(
    rgba: &[u8],
    width: u32,
    height: u32,
    keep: &[bool],
    reach: Reach,
    max_working_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    dilate_cancellable(rgba, width, height, keep, reach, max_working_bytes, None)
}

/// [`dilate`] に取り消しの旗を付けたもの。旗が立つと、次の段か色の計算の区切りで [`ExportError::Cancelled`] を返す（途中の画像は捨てる）。
pub fn dilate_cancellable(
    rgba: &[u8],
    width: u32,
    height: u32,
    keep: &[bool],
    reach: Reach,
    max_working_bytes: u64,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<u8>, ExportError> {
    dilate_tuned(
        rgba,
        width,
        height,
        keep,
        reach,
        max_working_bytes,
        cancel,
        Tuning::DEFAULT,
    )
}

/// [`dilate_cancellable`] の並列の分け方を選ぶもの（試験用。結果は同じ）。
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn dilate_tuned(
    rgba: &[u8],
    width: u32,
    height: u32,
    keep: &[bool],
    reach: Reach,
    max_working_bytes: u64,
    cancel: Option<&AtomicBool>,
    tuning: Tuning,
) -> Result<Vec<u8>, ExportError> {
    if width == 0 || height == 0 {
        return Err(ExportError::InvalidArgument("大きさ"));
    }
    let n = width as u64 * height as u64;
    if rgba.len() as u64 != n * 4 || keep.len() as u64 != n {
        return Err(ExportError::InvalidArgument(
            "画像と覆いは幅 × 高さ（画像は × 4 バイト）",
        ));
    }
    if n > u32::MAX as u64 {
        return Err(ExportError::InvalidArgument("画素数が多すぎる"));
    }
    let needed = working_bytes(width, height);
    if needed > max_working_bytes {
        return Err(ExportError::WorkingBudgetExceeded {
            needed,
            allowed: max_working_bytes,
        });
    }
    let mut output = rgba.to_vec();
    let limit = match reach {
        Reach::Texels(0) => return Ok(output),
        Reach::Texels(t) => t as i64,
        Reach::Fill => i64::MAX,
    };
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    let (w, h, n) = (width as usize, height as usize, n as usize);
    // 0: 埋まっていない、1: 残すテクセル、k+1: 段 k で埋めた、負: 今の段の候補の印
    let mut step = vec![0i32; n];
    for (s, &k) in step.iter_mut().zip(keep) {
        *s = i32::from(k);
    }
    let mut frontier: Vec<u32> = (0..n)
        .into_par_iter()
        .filter(|&i| {
            keep[i] && {
                let (around, count) = neighbors(i, w, h);
                around[..count].iter().any(|&j| step[j] == 0)
            }
        })
        .map(|i| i as u32)
        .collect();
    let mut candidates: Vec<u32> = Vec::new();
    let mut k: i64 = 1;
    while !frontier.is_empty() && k <= limit {
        if cancelled() {
            return Err(ExportError::Cancelled);
        }
        let ring = k as i32; // 段は画像の辺より多くならない
        let mark = -(ring + 1);
        candidates.clear();
        for &f in &frontier {
            let (around, count) = neighbors(f as usize, w, h);
            for &j in &around[..count] {
                if step[j] == 0 {
                    step[j] = mark;
                    candidates.push(j as u32);
                }
            }
        }
        // 色は前の段までの結果（step が正）だけから。候補の印は負なので、同じ段の書き込みは読まれない。
        for chunk in candidates.chunks(tuning.color_chunk.max(1)) {
            if cancelled() {
                return Err(ExportError::Cancelled);
            }
            let colors: Vec<u32> = if chunk.len() >= tuning.parallel_min {
                chunk
                    .par_iter()
                    .with_min_len(4096)
                    .map(|&c| average(c as usize, w, h, &step, &output))
                    .collect()
            } else {
                chunk
                    .iter()
                    .map(|&c| average(c as usize, w, h, &step, &output))
                    .collect()
            };
            for (&c, v) in chunk.iter().zip(colors) {
                output[c as usize * 4..c as usize * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        for &c in &candidates {
            step[c as usize] = ring + 1;
        }
        std::mem::swap(&mut frontier, &mut candidates);
        k += 1;
    }
    Ok(output)
}

/// [`Rings`] が持てる段数の上限（段を 1 テクセル 1 バイトで持つ）。
pub const MAX_RING_REACH: u32 = 254;

/// 段の地図: [`dilate`] が各テクセルを埋める段（`keep` から 8 近傍で 1 段ずつ広げた距離）を、`reach` 段まで持つ。
/// [`Rings::dilate_region`] が、変わった矩形の周りだけを塗り広げ直すのに使う（画像全体の段を毎回数え直さない）。
/// 1 テクセル 1 バイト（[`Rings::bytes`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rings {
    width: u32,
    height: u32,
    reach: u32,
    /// 0: 届かない（元のまま）、1: 残すテクセル、k + 1: 段 k で埋める。
    step: Vec<u8>,
}

impl Rings {
    /// 覆い（`keep`、`[y * width + x]`）から `reach` 段までの地図を作る（[`dilate`] の段と同じ数え方。`keep` が 1 つも無ければ全部 0）。
    /// `reach` は 0〜[`MAX_RING_REACH`]。
    pub fn new(width: u32, height: u32, keep: &[bool], reach: u32) -> Result<Rings, ExportError> {
        if width == 0 || height == 0 {
            return Err(ExportError::InvalidArgument("大きさ"));
        }
        let n = width as u64 * height as u64;
        if keep.len() as u64 != n {
            return Err(ExportError::InvalidArgument("覆いは幅 × 高さ"));
        }
        if n > u32::MAX as u64 {
            return Err(ExportError::InvalidArgument("画素数が多すぎる"));
        }
        if reach > MAX_RING_REACH {
            return Err(ExportError::InvalidArgument("塗り広げの段数"));
        }
        let (w, h, n) = (width as usize, height as usize, n as usize);
        let mut step: Vec<u8> = keep.iter().map(|&k| u8::from(k)).collect();
        let mut frontier: Vec<u32> = (0..n)
            .into_par_iter()
            .filter(|&i| {
                keep[i] && {
                    let (around, count) = neighbors(i, w, h);
                    around[..count].iter().any(|&j| step[j] == 0)
                }
            })
            .map(|i| i as u32)
            .collect();
        let mut next: Vec<u32> = Vec::new();
        for ring in 1..=reach {
            if frontier.is_empty() {
                break;
            }
            next.clear();
            for &f in &frontier {
                let (around, count) = neighbors(f as usize, w, h);
                for &j in &around[..count] {
                    if step[j] == 0 {
                        step[j] = (ring + 1) as u8;
                        next.push(j as u32);
                    }
                }
            }
            std::mem::swap(&mut frontier, &mut next);
        }
        Ok(Rings {
            width,
            height,
            reach,
            step,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// 段数（[`Reach::Texels`] の値）。
    pub fn reach(&self) -> u32 {
        self.reach
    }

    /// 持っているバイト数（1 テクセル 1 バイト）。
    pub fn bytes(&self) -> usize {
        self.step.len()
    }

    /// 矩形（画像の中に切る）の中に、覆いの中のテクセルと、塗り広げるテクセル（段 1〜`reach`）があるか。変わった矩形の塗り広げ直しを
    /// 要る所だけにするのに使う: 覆いの中のテクセルが無い矩形の変化は、ほかのテクセルの塗り広げの色に届かない。
    pub fn kinds_in(&self, rect: Rect) -> (bool, bool) {
        let x1 = (rect.x as u64 + rect.width as u64).min(self.width as u64) as usize;
        let y1 = (rect.y as u64 + rect.height as u64).min(self.height as u64) as usize;
        let (x0, y0) = (rect.x as usize, rect.y as usize);
        let (mut keep, mut filled) = (false, false);
        for y in y0..y1.max(y0) {
            let row = &self.step[y * self.width as usize..(y + 1) * self.width as usize];
            for &s in &row[x0.min(x1)..x1] {
                keep |= s == 1;
                filled |= s >= 2;
            }
            if keep && filled {
                break;
            }
        }
        (keep, filled)
    }

    /// テクセルを埋める段（0: 覆いの中、k: 段 k で埋める、None: `reach` 段では届かない）。
    pub fn ring(&self, x: u32, y: u32) -> Option<u32> {
        match self.step[(y * self.width + x) as usize] {
            0 => None,
            s => Some(s as u32 - 1),
        }
    }

    /// `pixels`（straight RGBA8、行は下から上）は画像の中の矩形 `outer` の中身。そのうち `inner` の中のテクセルを、画像全体を
    /// [`dilate`]（`Reach::Texels(self.reach())`）したときと同じ値に書き換える。`inner` を `reach` 段広げた矩形（画像の中に切ったもの）が
    /// `outer` に入っていること（入っていなければ断る）: 段 k のテクセルの色は距離 k 以内のテクセルだけで決まるので、その範囲の入力が
    /// あれば同じ値になる。`outer` の中の `inner` の外は途中の値（近傍が `outer` の外にあると違う値）で、使わない。
    pub fn dilate_region(
        &self,
        pixels: &mut [u8],
        outer: Rect,
        inner: Rect,
    ) -> Result<(), ExportError> {
        let inside = |r: &Rect, x0: u32, y0: u32, x1: u32, y1: u32| {
            r.x >= x0
                && r.y >= y0
                && r.x.checked_add(r.width).is_some_and(|e| e <= x1)
                && r.y.checked_add(r.height).is_some_and(|e| e <= y1)
        };
        if !inside(&outer, 0, 0, self.width, self.height) {
            return Err(ExportError::InvalidArgument("外の矩形は画像の中"));
        }
        if pixels.len() as u64 != outer.width as u64 * outer.height as u64 * 4 {
            return Err(ExportError::InvalidArgument(
                "画素は外の矩形の幅 × 高さ × 4 バイト",
            ));
        }
        if inner.is_empty() {
            return Ok(());
        }
        let need = Rect::new(
            inner.x.saturating_sub(self.reach),
            inner.y.saturating_sub(self.reach),
            0,
            0,
        );
        let need_x1 =
            (inner.x as u64 + inner.width as u64 + self.reach as u64).min(self.width as u64);
        let need_y1 =
            (inner.y as u64 + inner.height as u64 + self.reach as u64).min(self.height as u64);
        let need = Rect::new(
            need.x,
            need.y,
            (need_x1 - need.x as u64) as u32,
            (need_y1 - need.y as u64) as u32,
        );
        if !inside(
            &need,
            outer.x,
            outer.y,
            outer.x + outer.width,
            outer.y + outer.height,
        ) {
            return Err(ExportError::InvalidArgument(
                "外の矩形は、内の矩形を段数だけ広げた範囲を含む",
            ));
        }
        if self.reach == 0 {
            return Ok(());
        }
        // 外の矩形の中のテクセルを段ごとに並べる（段 1 から順に、前の段までの値だけを読んで埋める）
        let (ow, oh) = (outer.width as usize, outer.height as usize);
        let step_at = |lx: usize, ly: usize| -> u8 {
            self.step[(outer.y as usize + ly) * self.width as usize + outer.x as usize + lx]
        };
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); self.reach as usize];
        for ly in 0..oh {
            for lx in 0..ow {
                let s = step_at(lx, ly);
                if s >= 2 {
                    buckets[s as usize - 2].push((ly * ow + lx) as u32);
                }
            }
        }
        let color = |c: u32, s: u8, pixels: &[u8]| -> u32 {
            let (ly, lx) = ((c as usize) / ow, (c as usize) % ow);
            let mut around = [0usize; 8];
            let mut count = 0;
            for ny in ly.saturating_sub(1)..=(ly + 1).min(oh - 1) {
                for nx in lx.saturating_sub(1)..=(lx + 1).min(ow - 1) {
                    if (nx, ny) == (lx, ly) {
                        continue;
                    }
                    let t = step_at(nx, ny);
                    if t > 0 && t < s {
                        around[count] = ny * ow + nx;
                        count += 1;
                    }
                }
            }
            mix(around[..count].iter().map(|&j| &pixels[j * 4..j * 4 + 4]))
        };
        let mut colors: Vec<u32> = Vec::new();
        for (k, bucket) in buckets.iter().enumerate() {
            let s = (k + 2) as u8;
            colors.clear();
            if bucket.len() >= Tuning::DEFAULT.parallel_min {
                let read: &[u8] = pixels;
                bucket
                    .par_iter()
                    .with_min_len(4096)
                    .map(|&c| color(c, s, read))
                    .collect_into_vec(&mut colors);
            } else {
                colors.extend(bucket.iter().map(|&c| color(c, s, pixels)));
            }
            for (&c, v) in bucket.iter().zip(&colors) {
                let at = c as usize * 4;
                pixels[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        Ok(())
    }
}

/// 8 近傍（画像の中だけ）の添え字と、その数。内側のテクセルは境界の判定なしで引く。
#[inline(always)]
fn neighbors(i: usize, w: usize, h: usize) -> ([usize; 8], usize) {
    let y = i / w;
    let x = i - y * w;
    if x > 0 && y > 0 && x + 1 < w && y + 1 < h {
        return (
            [
                i - w - 1,
                i - w,
                i - w + 1,
                i - 1,
                i + 1,
                i + w - 1,
                i + w,
                i + w + 1,
            ],
            8,
        );
    }
    let mut out = [0usize; 8];
    let mut count = 0;
    for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
        for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
            let j = ny * w + nx;
            if j != i {
                out[count] = j;
                count += 1;
            }
        }
    }
    (out, count)
}

/// 埋まった（正の段の）8 近傍の色の平均。色はアルファで重みを付ける（透明な近傍の色に引っ張られない）。全部が透明なら色の平均。
/// R | G << 8 | B << 16 | A << 24。
fn average(c: usize, w: usize, h: usize, step: &[i32], image: &[u8]) -> u32 {
    let (around, around_count) = neighbors(c, w, h);
    mix(around[..around_count]
        .iter()
        .filter(|&&j| step[j] > 0)
        .map(|&j| &image[j * 4..j * 4 + 4]))
}

/// 近傍の画素（straight RGBA8）の平均（[`average`] の式。色はアルファで重みを付け、全部が透明なら色の平均。どちらも四捨五入）。
/// 近傍が無ければ 0。R | G << 8 | B << 16 | A << 24。
#[inline]
fn mix<'a>(around: impl Iterator<Item = &'a [u8]>) -> u32 {
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    let (mut rw, mut gw, mut bw) = (0u64, 0u64, 0u64);
    let mut count = 0u64;
    for px in around {
        let alpha = px[3] as u64;
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        a += alpha;
        count += 1;
        rw += px[0] as u64 * alpha;
        gw += px[1] as u64 * alpha;
        bw += px[2] as u64 * alpha;
    }
    if count == 0 {
        return 0; // 候補は埋まった近傍から来るので起きない
    }
    let (rr, gg, bb) = if a > 0 {
        (round(rw, a), round(gw, a), round(bw, a))
    } else {
        (round(r, count), round(g, count), round(b, count))
    };
    let aa = round(a, count);
    u32::from_le_bytes([rr as u8, gg as u8, bb as u8, aa as u8])
}

#[inline]
fn round(sum: u64, count: u64) -> u64 {
    (2 * sum + count) / (2 * count)
}
