//! 重なった UV のテクセル。数え方はベイクの行の割り当て（`Raster::row`）そのもので、1 テクセルに中心の 1 点（アンチエイリアス 1）。
//! UV のワイヤーフレームの重なりの表示と、塗りの知らせが使う。数えるのは渡した三角形だけで、ベイクの優先・焼かない島は見ない
//! （塗りでは、どの島も同じテクセルを使う）。座標は左下原点。
use super::{check, raster::Raster, Result};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// UV の行帯の参照に使ってよい大きさ（ベイクの予算の残りに当たるもの。超える UV は断る）。
const BAND_BUDGET: u64 = 256 << 20;

/// 重なった UV のテクセルの図。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UvOverlap {
    width: u32,
    height: u32,
    /// 行ごとの重なったテクセルの区間 `[始まり, 終わり)`（x の順、重ならない）。
    rows: Vec<Vec<[u32; 2]>>,
    texels: u64,
    /// 重なりに関わる三角形（持ち主と、その内側に重なった三角形。元の番号の昇順）。
    triangles: Vec<u32>,
}
impl UvOverlap {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// 重なったテクセルの数。
    pub fn texel_count(&self) -> u64 {
        self.texels
    }
    pub fn is_empty(&self) -> bool {
        self.texels == 0
    }
    /// 行 `y` の重なったテクセルの区間（`[始まり, 終わり)`、x の順）。
    pub fn row(&self, y: u32) -> &[[u32; 2]] {
        self.rows.get(y as usize).map_or(&[], Vec::as_slice)
    }
    /// 重なりに関わる三角形（元の番号の昇順）。
    pub fn triangles(&self) -> &[u32] {
        &self.triangles
    }
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let row = self.row(y);
        let i = row.partition_point(|r| r[1] <= x);
        row.get(i).is_some_and(|r| r[0] <= x)
    }
    /// 矩形（両端を含むテクセルの範囲。図の外は切る）に重なったテクセルがあるか。
    pub fn any_in(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> bool {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x0, x1) = (x0.max(0), x1.min(w - 1));
        let (y0, y1) = (y0.max(0), y1.min(h - 1));
        if x0 > x1 || y0 > y1 || self.texels == 0 {
            return false;
        }
        (y0..=y1).any(|y| {
            let row = &self.rows[y as usize];
            let i = row.partition_point(|r| (r[1] as i64) <= x0);
            row.get(i).is_some_and(|r| (r[0] as i64) <= x1)
        })
    }
}

/// 重なった UV のテクセルを数える。`uvs` は三角形ごとの 6 つ（a.x, a.y, b.x, b.y, c.x, c.y。三角形の番号の順）、`triangles` は数える
/// 三角形（元の番号）。UV の面積が 0 の三角形はベイクと同じく数えない。取り消されたら `None`。
pub fn uv_overlap(
    uvs: &[f32],
    triangles: &[usize],
    width: u32,
    height: u32,
    cancel: Option<&AtomicBool>,
) -> Result<Option<UvOverlap>> {
    check(
        (1..=8192).contains(&width) && (1..=8192).contains(&height),
        "UV の重なりの図の大きさが範囲外です",
    )?;
    check(
        uvs.len().is_multiple_of(6) && triangles.iter().all(|t| (t + 1) * 6 <= uvs.len()),
        "三角形の番号が UV の数を超えています",
    )?;
    let receivers: Vec<usize> = triangles
        .iter()
        .copied()
        .filter(|&t| {
            let u = &uvs[t * 6..t * 6 + 6];
            let area = ((u[2] - u[0]) * (u[5] - u[1]) - (u[4] - u[0]) * (u[3] - u[1])) as f64;
            u.iter().all(|v| v.is_finite()) && area.abs() * width as f64 * (height as f64) >= 1e-9
        })
        .collect();
    let raster = Raster::from_uvs(
        uvs,
        (width as i32, height as i32),
        1,
        &receivers,
        BAND_BUDGET,
    )?;
    let involved: Vec<AtomicBool> = (0..uvs.len() / 6).map(|_| AtomicBool::new(false)).collect();
    let canceled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    let rows: Option<Vec<Vec<[u32; 2]>>> = (0..height as usize)
        .into_par_iter()
        .map(|y| {
            if canceled() {
                return None;
            }
            let samples = raster.row_with(y, width as usize, 1, |a, b| {
                involved[a].store(true, Ordering::Relaxed);
                involved[b].store(true, Ordering::Relaxed);
            });
            let mut runs: Vec<[u32; 2]> = Vec::new();
            for x in 0..width {
                if !samples.get(x as usize).is_some_and(|s| s.overlap) {
                    continue;
                }
                match runs.last_mut() {
                    Some(last) if last[1] == x => last[1] = x + 1,
                    _ => runs.push([x, x + 1]),
                }
            }
            Some(runs)
        })
        .collect();
    let Some(rows) = rows else {
        return Ok(None);
    };
    if canceled() {
        return Ok(None);
    }
    let texels = rows.iter().flatten().map(|r| (r[1] - r[0]) as u64).sum();
    let triangles = involved
        .iter()
        .enumerate()
        .filter(|(_, b)| b.load(Ordering::Relaxed))
        .map(|(t, _)| t as u32)
        .collect();
    Ok(Some(UvOverlap {
        width,
        height,
        rows,
        texels,
        triangles,
    }))
}
