//! 塗りのパス（種類 `Fill`）: パスの曲線を多角形にし、その内側（巻き数が 0 でない所）を、ブラシの色（組があれば組の値）で塗る。
//! 開いたパスは終わりから始めへ閉じて塗る。覆いは画素ごとに 4 × 4 の副標本の数（16 段）。
//!
//! - 2D: キャンバスの曲線をそのまま多角形にする（0.5 画素ほどの間隔の標本）。
//! - 3D: 点が全部 1 つの UV の島にあるときだけ。曲線を面へ投影し（描くストロークと同じレイ）、当たった所の UV を多角形にする。
//!   投影が島の外に当たれば断り（[`Error::FillIslands`]）、当たらない標本は欠落に数えて飛ばす。副標本は、島の三角形の UV に入る
//!   ものだけ数える（UV の上で島の内側に入れ子になったほかの島や、島の外の余白は塗らない）。
//!
//! 塗るのはブラシのストロークの 1 画素ずつの塗り（不透明度が天井。流量は 1 にする。硬さ・筆圧は使わない）。

use std::collections::HashMap;

use glam::{DVec2, Vec2};

use super::render::{canvas_curve, Painter};
use super::*;
use crate::geometry::{region, uv_barycentric, SurfaceGeometry, SurfaceRegionKind};

/// 副標本の 1 辺の数。
const SUB: i64 = 4;
/// 多角形と副標本の行の交わりの数の上限（極端な多角形で覚えが膨らまないように）。
const MAX_CROSSINGS: usize = 64_000_000;
/// 多角形の点・交わり・副標本の行の覚え 1 つの見積もり（バイト）。
const POINT_BYTES: u64 = std::mem::size_of::<DVec2>() as u64;
const CROSSING_BYTES: u64 = std::mem::size_of::<(f64, i32)>() as u64;
const ROW_BYTES: u64 = std::mem::size_of::<Vec<(f64, i32)>>() as u64;
/// 3D の島の升 1 つ（鍵 (i64, i64) と三角形の番号の並び）の見積もり（バイト）。
const GRID_CELL_BYTES: u64 =
    (std::mem::size_of::<((i64, i64), Vec<u32>)>() + std::mem::size_of::<u64>()) as u64;

/// 塗りの作業の覚え（多角形・副標本の行ごとの交わり・3D の曲線と島の升）の見積もり。ストロークの予算（`stroke_budget_bytes`）に数え、
/// 超えるなら [`CoreError::StrokeBudgetExceeded`] で断る（リボンの集めた画素と同じ。実際の確保でなく、1 つあたりの大きさの見積もり）。
pub(super) struct Memory {
    budget: u64,
    used: u64,
}

impl Memory {
    pub(super) fn new(budget: u64) -> Memory {
        Memory { budget, used: 0 }
    }
    fn add(&mut self, bytes: u64) -> Result<(), Error> {
        self.used = self.used.saturating_add(bytes);
        if self.used > self.budget {
            Err(Error::Core(CoreError::StrokeBudgetExceeded))
        } else {
            Ok(())
        }
    }
}

/// 塗りのブラシ（流量 1・消しゴムなし。不透明度と色はブラシのまま）。
fn fill_brush(b: BrushSettings) -> crate::Brush {
    crate::Brush::from(BrushSettings {
        flow: 1.0,
        erase: false,
        ..b
    })
}

/// 多角形（画素の座標）の内側の覆いを、画素の行を下から、行の中を左から `emit(x, y, 覆い)` へ渡す。`inside(x, y)` は副標本の点を
/// 数えるか。画布の外は渡さない。渡した画素の数を返す。`memory` に、副標本の行ごとの交わりの覚えを数える（多角形の点は呼び手が
/// 数えてある）。取消は、画素を渡す `emit` が確かめる。
pub(super) fn rasterize(
    poly: &[DVec2],
    width: u32,
    height: u32,
    inside: &dyn Fn(f64, f64) -> bool,
    memory: &mut Memory,
    emit: &mut dyn FnMut(i32, i32, f64) -> Result<(), Error>,
) -> Result<usize, Error> {
    if poly.len() < 3 {
        return Ok(0);
    }
    let (mut lo, mut hi) = (poly[0], poly[0]);
    for p in poly {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let x0 = (lo.x.floor() as i64).max(0);
    let x1 = (hi.x.ceil() as i64).min(width as i64) - 1;
    let y0 = (lo.y.floor() as i64).max(0);
    let y1 = (hi.y.ceil() as i64).min(height as i64) - 1;
    if x1 < x0 || y1 < y0 {
        return Ok(0);
    }
    // 副標本の行ごとの交わり（x と向き）
    let rows = ((y1 - y0 + 1) * SUB) as usize;
    memory.add(rows as u64 * ROW_BYTES)?;
    let mut crossings: Vec<Vec<(f64, i32)>> = vec![Vec::new(); rows];
    let mut total = 0usize;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if a.y == b.y {
            continue;
        }
        let dir = if b.y > a.y { 1 } else { -1 };
        let (low, high) = if a.y < b.y { (a.y, b.y) } else { (b.y, a.y) };
        // 標本の行 k の y は y0 + (k + 0.5) / SUB。low ≤ y < high の行
        let first = ((low - y0 as f64) * SUB as f64 - 0.5).ceil().max(0.0) as i64;
        let last = (((high - y0 as f64) * SUB as f64 - 0.5).ceil() as i64 - 1).min(rows as i64 - 1);
        // この辺が加える交わりの覚えを、加える前に数える（予算を超えるなら、確保する前に断る）
        if last >= first {
            memory.add((last - first + 1) as u64 * CROSSING_BYTES)?;
        }
        for k in first..=last {
            let y = y0 as f64 + (k as f64 + 0.5) / SUB as f64;
            let x = a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y);
            crossings[k as usize].push((x, dir));
            total += 1;
            if total > MAX_CROSSINGS {
                return Err(Error::TooManySamples);
            }
        }
    }
    let span = (x1 - x0 + 1) as usize;
    let mut counts = vec![0u8; span];
    let mut emitted = 0;
    for py in y0..=y1 {
        counts.iter_mut().for_each(|c| *c = 0);
        for sub in 0..SUB {
            let k = ((py - y0) * SUB + sub) as usize;
            let row = &mut crossings[k];
            row.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.cmp(&q.1)));
            let y = py as f64 + (sub as f64 + 0.5) / SUB as f64;
            let mut wind = 0;
            for w in 0..row.len() {
                wind += row[w].1;
                if wind == 0 || w + 1 >= row.len() {
                    continue;
                }
                // [row[w].x, row[w + 1].x) の副標本の列
                let from = (row[w].0 * SUB as f64 - 0.5).ceil() as i64;
                let to = (row[w + 1].0 * SUB as f64 - 0.5).ceil() as i64 - 1;
                for s in from.max(x0 * SUB)..=to.min((x1 + 1) * SUB - 1) {
                    let x = (s as f64 + 0.5) / SUB as f64;
                    if inside(x, y) {
                        counts[(s / SUB - x0) as usize] += 1;
                    }
                }
            }
        }
        for (i, c) in counts.iter().enumerate() {
            if *c > 0 {
                emit(
                    (x0 + i as i64) as i32,
                    py as i32,
                    *c as f64 / (SUB * SUB) as f64,
                )?;
                emitted += 1;
            }
        }
    }
    Ok(emitted)
}

/// 2D の塗りのパス。曲線の標本の数を返す。
pub(super) fn draw_canvas(
    painter: &mut Painter<'_, '_>,
    path: &CanvasPath,
) -> Result<usize, Error> {
    painter.begin(
        paints(path.channel, path.brush, &path.material)?,
        &fill_brush(path.brush.0),
    )?;
    let mut memory = Memory::new(painter.options().stroke_budget_bytes);
    let mut poly = Vec::new();
    let samples = canvas_curve(path, 0.5, &mut |x, y, _| {
        memory.add(POINT_BYTES)?;
        poly.push(DVec2::new(x, y));
        Ok(())
    })?;
    let (w, h) = painter.size();
    rasterize(&poly, w, h, &|_, _| true, &mut memory, &mut |x, y, c| {
        painter.pixel(x, y, c, 1.0)
    })?;
    painter.end()?;
    Ok(samples)
}

/// 3D の塗りのパス。(曲線の標本の数, 0, 面に投影できなかった標本の数)。
pub(super) fn draw_surface(
    painter: &mut Painter<'_, '_>,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    o: &Options<'_>,
) -> Result<(usize, usize, usize), Error> {
    painter.begin(
        paints(path.channel, path.brush, &path.material)?,
        &fill_brush(BrushSettings {
            radius: 1.0,
            ..path.brush.0
        }),
    )?;
    let ps = &path.points;
    if ps.len() < 3 {
        painter.end()?;
        return Ok((ps.len(), 0, 0));
    }
    let island = region(g, ps[0].triangle, SurfaceRegionKind::UvIsland);
    if ps
        .iter()
        .any(|p| island.binary_search(&p.triangle).is_err())
    {
        return Err(Error::FillIslands);
    }
    // 曲線をテクセルの半分ほどの間隔で辿り、面へ投影した UV を多角形にする
    let spacing = crate::geometry::world_radius(g, 0.5, o.width).max(1e-6);
    let curve = super::surface::surface_curve(path, g, spacing, o)?;
    let mut memory = Memory::new(o.stroke_budget_bytes);
    memory.add(curve.len() as u64 * std::mem::size_of::<super::surface::CurvePoint>() as u64)?;
    let (wf, hf) = (o.width as f64, o.height as f64);
    let mut poly = Vec::new();
    let mut gaps = 0;
    for q in &curve {
        o.check()?;
        let Some(hit) = super::surface::project(g, q) else {
            gaps += 1;
            continue;
        };
        if island.binary_search(&hit.triangle).is_err() {
            return Err(Error::FillIslands);
        }
        memory.add(POINT_BYTES)?;
        poly.push(DVec2::new(hit.uv.x as f64 * wf, hit.uv.y as f64 * hf));
    }
    // 島の三角形を UV の升（16 画素）に分けて、副標本が島の UV に入るかを引く
    const CELL: f64 = 16.0;
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for &t in &island {
        let tri = &g.triangles()[t as usize];
        let uv = [tri.uv_a, tri.uv_b, tri.uv_c];
        let (mut lo, mut hi) = (uv[0], uv[0]);
        for p in uv {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let (cx0, cx1) = (
            (lo.x as f64 * wf / CELL).floor() as i64,
            (hi.x as f64 * wf / CELL).floor() as i64,
        );
        let (cy0, cy1) = (
            (lo.y as f64 * hf / CELL).floor() as i64,
            (hi.y as f64 * hf / CELL).floor() as i64,
        );
        if (cx1 - cx0 + 1) * (cy1 - cy0 + 1) > 1 << 20 {
            continue;
        }
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let cell = grid.entry((cx, cy)).or_default();
                // 升の覚え: 三角形の番号 1 つ、新しい升なら鍵と空き（見積もり）
                memory.add(
                    std::mem::size_of::<u32>() as u64
                        + if cell.is_empty() { GRID_CELL_BYTES } else { 0 },
                )?;
                cell.push(t);
            }
        }
    }
    let inside = |x: f64, y: f64| {
        let cell = ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
        let uv = Vec2::new((x / wf) as f32, (y / hf) as f32);
        grid.get(&cell).is_some_and(|ts| {
            ts.iter()
                .any(|&t| uv_barycentric(uv, &g.triangles()[t as usize]).is_some())
        })
    };
    let (w, h) = painter.size();
    rasterize(&poly, w, h, &inside, &mut memory, &mut |x, y, c| {
        painter.pixel(x, y, c, 1.0)
    })?;
    painter.end()?;
    Ok((curve.len(), 0, gaps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(poly: &[DVec2], w: u32, h: u32) -> Vec<(i32, i32, f64)> {
        let mut out = Vec::new();
        rasterize(
            poly,
            w,
            h,
            &|_, _| true,
            &mut Memory::new(u64::MAX),
            &mut |x, y, c| {
                out.push((x, y, c));
                Ok(())
            },
        )
        .unwrap();
        out
    }

    #[test]
    fn a_square_covers_whole_and_half_pixels() {
        // 2.0〜6.5 の正方形: 2〜5 の画素は全部、6 の画素は半分
        let poly = [
            DVec2::new(2.0, 2.0),
            DVec2::new(6.5, 2.0),
            DVec2::new(6.5, 6.5),
            DVec2::new(2.0, 6.5),
        ];
        let px = run(&poly, 10, 10);
        let at = |x: i32, y: i32| px.iter().find(|p| p.0 == x && p.1 == y).map(|p| p.2);
        assert_eq!(at(3, 3), Some(1.0));
        assert_eq!(at(6, 3), Some(0.5));
        assert_eq!(at(6, 6), Some(0.25));
        assert_eq!(at(1, 3), None);
        assert_eq!(at(7, 3), None);
        // 向きを逆にしても同じ（巻き数が 0 でない所）
        let mut reversed = poly;
        reversed.reverse();
        assert_eq!(run(&reversed, 10, 10), px);
    }

    #[test]
    fn the_work_memory_of_a_fill_counts_against_the_budget_to_the_byte() {
        let poly = [
            DVec2::new(2.0, 2.0),
            DVec2::new(6.5, 2.0),
            DVec2::new(6.5, 6.5),
            DVec2::new(2.0, 6.5),
        ];
        let used = |budget: u64| {
            let mut memory = Memory::new(budget);
            let result = rasterize(&poly, 10, 10, &|_, _| true, &mut memory, &mut |_, _, _| {
                Ok(())
            });
            (result, memory.used)
        };
        // 行 5 画素 × 4 = 20 本、縦の 2 辺がそれぞれ 18 行をまたぐので交わりは 36 個
        let (full, needed) = used(u64::MAX);
        assert!(full.is_ok());
        assert_eq!(needed, 20 * ROW_BYTES + 36 * CROSSING_BYTES);
        assert!(used(needed).0.is_ok(), "ちょうどなら通る");
        assert_eq!(
            used(needed - 1).0,
            Err(Error::Core(CoreError::StrokeBudgetExceeded)),
            "1 バイト足りなければ断る"
        );
        // 行の覚えだけで足りないときも、交わりを集める前に断る
        assert_eq!(
            used(20 * ROW_BYTES - 1).0,
            Err(Error::Core(CoreError::StrokeBudgetExceeded))
        );
    }

    #[test]
    fn a_self_overlapping_loop_fills_by_nonzero_winding_and_clips_to_the_canvas() {
        // 同じ向きに 2 周する正方形も 1 回だけ塗る（覆いは 1 まで）
        let sq = [
            DVec2::new(-3.0, -3.0),
            DVec2::new(4.0, -3.0),
            DVec2::new(4.0, 4.0),
            DVec2::new(-3.0, 4.0),
        ];
        let twice: Vec<DVec2> = sq.iter().chain(sq.iter()).copied().collect();
        let px = run(&twice, 8, 8);
        assert!(px.iter().all(|p| p.2 <= 1.0 && p.0 >= 0 && p.1 >= 0));
        assert_eq!(px.len(), 16);
    }
}
