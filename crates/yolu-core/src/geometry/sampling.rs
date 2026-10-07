//! 面の上の参照の写像（C# の SurfaceGeometry.Sampling）: クローン・指先が読む場所を、UV アイランドをまたいで決める。
//!
//! - **展開の図**（[`SamplingChart`]）は、アンカーの三角形から、辺でつながった三角形を幅優先で平らに展開した局所の座標。UV の
//!   画像の上の近さは使わない（UV の鏡映・離れたアイランド・余白に左右されない）。非多様体の辺・別のレンダラー・別のスロットへは進まない。
//!   閉じた曲面の全体を 1 つに展開するものではなく、半径の内側だけ。重なる展開では、アンカーからの幅優先の経路を優先する。
//! - 図の上の点から画素の参照（最大 4 点の双線形）を作るとき、各点が三角形の UV の外なら、同じ展開の隣の三角形へ運んで読む
//!   （アイランドの余白の画素を読まない）。
//! - 三角形の数と図のバイト（1 三角形 128 バイト）には上限があり、超えたら部分の図を返さずに断る。参照の探索の回数（調べた三角形の数）にも
//!   上限がある。点から三角形を探すのは、図を作った後に一度だけ組む格子で候補を絞り、候補を図に入れた順に調べる（全部を順に調べたのと
//!   同じ三角形になる）。

use std::collections::{HashMap, VecDeque};

use glam::{Vec2, Vec3};

use super::build::position_key;
use super::dab::SurfacePixel;
use super::query::barycentric;
use super::unity::{
    cross, dot, finite, finite2, finite3, fmax, fmin, mix2, normalized, sqr_magnitude,
    v2_magnitude, v2max, v2min,
};
use super::{SurfaceGeometry, SurfaceHit};
use crate::brush::{BrushMappedPixel, BrushPixel, BrushSourceTap};

/// 展開の図の三角形の数の既定の上限（C# の maxTriangles）。
pub const SAMPLING_CHART_MAX_TRIANGLES: i32 = 2048;
/// 図の 1 三角形の名目のバイト（C# の NominalBytes）。
pub const SAMPLING_CHART_TRIANGLE_BYTES: i64 = 128;
/// 1 つの図で、点から三角形を探すときに調べる三角形の数の上限（格子で候補を絞るので、1 回に数個）。
const LOOKUP_BUDGET: i32 = 1 << 26;

/// 図を作れなかった・参照の探索を断った理由。どれもストロークを取り消す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingError {
    /// 当たりのスナップショットの世代が違う・三角形の番号が範囲外。
    SnapshotChanged,
    /// 当たりのレンダラー・スロットがスナップショットの三角形と合わない。
    BindingMismatch,
    /// 半径・位置・向きが範囲外か有限でない。
    InvalidArguments,
    /// 三角形の数かバイトの予算を超えた。
    ChartBudget,
    /// 参照を探す回数の予算を超えた。
    LookupBudget,
    /// ダブの画素が展開の図に入らない（図は、そのダブの画素を覆う広さで作る前提）。
    Unreachable,
}

impl std::fmt::Display for SamplingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SamplingError::SnapshotChanged => "モデルのスナップショットが変わりました",
            SamplingError::BindingMismatch => "当たった面がスナップショットと合いません",
            SamplingError::InvalidArguments => "参照の半径か位置が範囲外です",
            SamplingError::ChartBudget => {
                "参照を読む面の上限を超えたので、ストロークを取り消しました。ブラシを小さくしてください"
            }
            SamplingError::LookupBudget => {
                "参照を探す回数の上限を超えたので、ストロークを取り消しました。ブラシを小さくしてください"
            }
            SamplingError::Unreachable => "このダブの画素が参照の図に入りません",
        })
    }
}

impl std::error::Error for SamplingError {}

/// 展開した三角形 1 つ（図の上の 3 頂点）。
#[derive(Clone, Copy, Debug)]
struct Entry {
    triangle: u32,
    a: Vec2,
    b: Vec2,
    c: Vec2,
}

/// 辺の展開で作った局所の座標（アンカーが原点）。ダブ 1 つのあいだだけ持つ。
pub struct SamplingChart<'g> {
    owner: &'g SurfaceGeometry,
    index: HashMap<u32, usize>,
    ordered: Vec<Entry>,
    remaining_queries: i32,
    /// 点から三角形を探す格子（最初に探すときに組む）。
    grid: Option<ChartGrid>,
}

/// 図の三角形の格子: 升ごとに、箱が重なる三角形の番号（図に入れた順）。
struct ChartGrid {
    min: Vec2,
    cell: f32,
    columns: usize,
    rows: usize,
    offsets: Vec<u32>,
    items: Vec<u32>,
}

impl ChartGrid {
    fn new(entries: &[Entry]) -> ChartGrid {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        let mut boxes = Vec::with_capacity(entries.len());
        for e in entries {
            let lo = v2min(e.a, v2min(e.b, e.c));
            let hi = v2max(e.a, v2max(e.b, e.c));
            // 重みの許し（−1e-6）で外側の点も入るので、箱を少し広げる
            let pad = fmax(hi.x - lo.x, hi.y - lo.y) * 1e-5 + 1e-12;
            let (lo, hi) = (lo - Vec2::splat(pad), hi + Vec2::splat(pad));
            min = v2min(min, lo);
            max = v2max(max, hi);
            boxes.push((lo, hi));
        }
        let size = max - min;
        let area = fmax(size.x * size.y, 1e-30);
        // 1 升に三角形が 2 つほど入る大きさ（升の数は三角形の数の 4 倍まで）
        let mut cell = fmax((area * 2.0 / entries.len().max(1) as f32).sqrt(), 1e-12);
        let fits = |cell: f32| {
            let c = (size.x / cell).ceil().max(1.0) as f64;
            let r = (size.y / cell).ceil().max(1.0) as f64;
            c * r <= (entries.len() as f64 * 4.0).max(1.0)
        };
        while !fits(cell) {
            cell *= 1.5;
        }
        let columns = ((size.x / cell).ceil().max(1.0)) as usize;
        let rows = ((size.y / cell).ceil().max(1.0)) as usize;
        let at = |v: f32, lo: f32, n: usize| -> usize {
            let i = ((v - lo) / cell).floor();
            if i > 0.0 {
                (i as usize).min(n - 1)
            } else {
                0
            }
        };
        let mut counts = vec![0u32; columns * rows];
        let range = |(lo, hi): (Vec2, Vec2)| {
            (
                at(lo.x, min.x, columns),
                at(hi.x, min.x, columns),
                at(lo.y, min.y, rows),
                at(hi.y, min.y, rows),
            )
        };
        for b in &boxes {
            let (x0, x1, y0, y1) = range(*b);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    counts[y * columns + x] += 1;
                }
            }
        }
        let mut offsets = vec![0u32; columns * rows + 1];
        for i in 0..columns * rows {
            offsets[i + 1] = offsets[i] + counts[i];
        }
        let mut items = vec![0u32; offsets[columns * rows] as usize];
        let mut cursor: Vec<u32> = offsets[..columns * rows].to_vec();
        for (i, b) in boxes.iter().enumerate() {
            let (x0, x1, y0, y1) = range(*b);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let c = y * columns + x;
                    items[cursor[c] as usize] = i as u32;
                    cursor[c] += 1;
                }
            }
        }
        ChartGrid {
            min,
            cell,
            columns,
            rows,
            offsets,
            items,
        }
    }

    /// 点の升の候補（図に入れた順）。格子の外なら空。
    fn candidates(&self, p: Vec2) -> &[u32] {
        let x = ((p.x - self.min.x) / self.cell).floor();
        let y = ((p.y - self.min.y) / self.cell).floor();
        if !(x >= 0.0 && y >= 0.0) || x as usize >= self.columns || y as usize >= self.rows {
            return &[];
        }
        let c = y as usize * self.columns + x as usize;
        &self.items[self.offsets[c] as usize..self.offsets[c + 1] as usize]
    }
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Unity の `Vector3.ProjectOnPlane`。
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    let sqr = dot(normal, normal);
    if sqr < f32::MIN_POSITIVE {
        return v;
    }
    let d = dot(v, normal);
    Vec3::new(
        v.x - normal.x * d / sqr,
        v.y - normal.y * d / sqr,
        v.z - normal.z * d / sqr,
    )
}

/// 点 p の、三角形 (a, b, c) に対する重み。退化した三角形は None。
fn weights(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<Vec3> {
    let (u, v, q) = (b - a, c - a, p - a);
    let det = cross2(u, v);
    if det.abs() < 1e-20 {
        return None;
    }
    let y = cross2(q, v) / det;
    let z = cross2(u, q) / det;
    Some(Vec3::new(1.0 - y - z, y, z))
}

/// 整数に近ければ整数へ寄せる（C# の Snap。画素の中心の位置の丸め誤差で、隣の画素へ混ざらないように）。
fn snap(value: f64) -> f64 {
    let r = value.round_ties_even();
    if (value - r).abs() < 1e-4 {
        r
    } else {
        value
    }
}

impl<'g> SamplingChart<'g> {
    /// 図の三角形の数。
    pub fn triangle_count(&self) -> usize {
        self.ordered.len()
    }

    /// 予算に数える名目のバイト（C# の NominalBytes）。
    pub fn nominal_bytes(&self) -> i64 {
        self.ordered.len() as i64 * SAMPLING_CHART_TRIANGLE_BYTES
    }

    /// 当たりの位置の、図の上の座標。この図の三角形でなければ None。
    pub fn coordinates(&self, hit: &SurfaceHit) -> Option<Vec2> {
        if hit.revision != self.owner.revision {
            return None;
        }
        let e = self.ordered.get(*self.index.get(&hit.triangle)?)?;
        let t = &self.owner.triangles[hit.triangle as usize];
        if hit.renderer != t.renderer
            || hit.material_slot != t.material_slot
            || !finite3(hit.position)
        {
            return None;
        }
        let b = barycentric(hit.position, t);
        let point = mix2(e.a, e.b, e.c, b);
        finite2(point).then_some(point)
    }

    /// ダブの画素（その三角形の上のテクセルの位置）の、図の上の座標。
    pub fn pixel_coordinates(&self, pixel: &SurfacePixel) -> Option<Vec2> {
        let triangle = pixel.triangle?;
        let e = self.ordered.get(*self.index.get(&triangle)?)?;
        let b = barycentric(pixel.position, &self.owner.triangles[triangle as usize]);
        let point = mix2(e.a, e.b, e.c, b);
        finite2(point).then_some(point)
    }

    /// 図の上の点を含む三角形（図に入れた順でいちばん先のもの）と重み。
    fn locate(&mut self, point: Vec2) -> Result<Option<(Entry, Vec3)>, SamplingError> {
        if !finite2(point) {
            return Ok(None);
        }
        let grid = self
            .grid
            .get_or_insert_with(|| ChartGrid::new(&self.ordered));
        for &i in grid.candidates(point) {
            self.remaining_queries -= 1;
            if self.remaining_queries < 0 {
                return Err(SamplingError::LookupBudget);
            }
            let e = self.ordered[i as usize];
            if let Some(w) = weights(point, e.a, e.b, e.c) {
                if w.x >= -1e-6 && w.y >= -1e-6 && w.z >= -1e-6 {
                    return Ok(Some((e, w)));
                }
            }
        }
        Ok(None)
    }

    /// 図の上の点にいちばん近いテクセル（その点の三角形の UV の点を含む画素）。図に無い点・文書の外は None。
    pub fn nearest_texel(
        &mut self,
        point: Vec2,
        width: i32,
        height: i32,
    ) -> Result<Option<(i64, i64)>, SamplingError> {
        let Some((e, b)) = self.locate(point)? else {
            return Ok(None);
        };
        let t = self.owner.triangles[e.triangle as usize];
        let uv = mix2(t.uv_a, t.uv_b, t.uv_c, b);
        let x = (uv.x * width as f32).floor() as i64;
        let y = (uv.y * height as f32).floor() as i64;
        Ok((x >= 0 && y >= 0 && x < width as i64 && y < height as i64).then_some((x, y)))
    }

    /// 図の上の点 point を、ダブの画素 pixel の参照（UV の画素の双線形の 4 点）にする。図に無い点・どの参照も読めない画素は None。
    /// 各点が三角形の UV の外なら、同じ展開の隣の三角形へ運んで画素を決める（アイランドの余白は読まない）。
    pub fn try_sample(
        &mut self,
        point: Vec2,
        pixel: BrushPixel,
        width: i32,
        height: i32,
    ) -> Result<Option<BrushMappedPixel>, SamplingError> {
        if !finite2(point) || width <= 0 || height <= 0 {
            return Ok(None);
        }
        let Some((e, b)) = self.locate(point)? else {
            return Ok(None);
        };
        let t = self.owner.triangles[e.triangle as usize];
        let uv = mix2(t.uv_a, t.uv_b, t.uv_c, b);
        let x = snap(uv.x as f64 * width as f64 - 0.5);
        let y = snap(uv.y as f64 * height as f64 - 0.5);
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - ix as f64, y - iy as f64);
        let tap = |chart: &mut SamplingChart<'g>,
                   px: i64,
                   py: i64,
                   weight: f64|
         -> Result<BrushSourceTap, SamplingError> {
            if weight <= 0.0 {
                return Ok(BrushSourceTap::NONE);
            }
            // UV の補間点が三角形の外なら、同じ展開上の隣接面へ運ぶ。アイランドの余白を読み込まない。
            let tap_uv = Vec2::new(
                (px as f32 + 0.5) / width as f32,
                (py as f32 + 0.5) / height as f32,
            );
            let Some(wb) = weights(tap_uv, t.uv_a, t.uv_b, t.uv_c) else {
                return Ok(BrushSourceTap::NONE);
            };
            let q = mix2(e.a, e.b, e.c, wb);
            let Some((next, nb)) = chart.locate(q)? else {
                return Ok(BrushSourceTap::NONE);
            };
            let nt = chart.owner.triangles[next.triangle as usize];
            let nuv = mix2(nt.uv_a, nt.uv_b, nt.uv_c, nb);
            let nx = (nuv.x * width as f32).floor() as i32;
            let ny = (nuv.y * height as f32).floor() as i32;
            Ok(if nx < 0 || ny < 0 || nx >= width || ny >= height {
                BrushSourceTap::NONE
            } else {
                BrushSourceTap::new(nx as i64, ny as i64, weight)
            })
        };
        let a = tap(self, ix, iy, (1.0 - fx) * (1.0 - fy))?;
        let c = tap(self, ix, iy + 1, (1.0 - fx) * fy)?;
        let d = tap(self, ix + 1, iy + 1, fx * fy)?;
        let ab = tap(self, ix + 1, iy, fx * (1.0 - fy))?;
        if a.weight + ab.weight + c.weight + d.weight <= 0.0 {
            return Ok(None);
        }
        Ok(Some(BrushMappedPixel::new(pixel, a, ab, c, d)))
    }
}

impl SurfaceGeometry {
    /// アンカーから radius（モデルの単位）の内側の局所展開。三角形の数が max_triangles、バイトが max_bytes を超えるなら、部分の図は
    /// 返さずに断る。tangent は展開の横軸（0 なら世界の X から作る）で、2 つのアンカーを結ぶクローンは対応する横軸を渡す。
    pub fn build_sampling_chart(
        &self,
        anchor: &SurfaceHit,
        radius: f32,
        tangent: Vec3,
        max_triangles: i32,
        max_bytes: i64,
    ) -> Result<SamplingChart<'_>, SamplingError> {
        if anchor.revision != self.revision || anchor.triangle as usize >= self.triangles.len() {
            return Err(SamplingError::SnapshotChanged);
        }
        if !finite(radius) || radius <= 0.0 || !finite3(anchor.position) || !finite3(tangent) {
            return Err(SamplingError::InvalidArguments);
        }
        let seed = &self.triangles[anchor.triangle as usize];
        if anchor.renderer != seed.renderer || anchor.material_slot != seed.material_slot {
            return Err(SamplingError::BindingMismatch);
        }
        let n = seed.normal();
        let mut u = project_on_plane(
            if sqr_magnitude(tangent) > 0.0 {
                tangent
            } else {
                Vec3::X
            },
            n,
        );
        if sqr_magnitude(u) < 1e-12 {
            u = project_on_plane(Vec3::Y, n);
        }
        let u = normalized(u);
        let v = cross(n, u);
        let project = |p: Vec3| Vec2::new(dot(p - anchor.position, u), dot(p - anchor.position, v));
        let mut chart = SamplingChart {
            owner: self,
            index: HashMap::new(),
            ordered: Vec::new(),
            remaining_queries: LOOKUP_BUDGET,
            grid: None,
        };
        let add = |chart: &mut SamplingChart<'_>, entry: Entry| -> Result<(), SamplingError> {
            if chart.ordered.len() as i64 >= max_triangles as i64
                || (chart.ordered.len() as i64 + 1) * SAMPLING_CHART_TRIANGLE_BYTES > max_bytes
            {
                return Err(SamplingError::ChartBudget);
            }
            chart.index.insert(entry.triangle, chart.ordered.len());
            chart.ordered.push(entry);
            Ok(())
        };
        add(
            &mut chart,
            Entry {
                triangle: anchor.triangle,
                a: project(seed.a),
                b: project(seed.b),
                c: project(seed.c),
            },
        )?;
        let tolerance = self.seam_tolerance as f64;
        let mut queue: VecDeque<u32> = VecDeque::new();
        queue.push_back(anchor.triangle);
        while let Some(current) = queue.pop_front() {
            let t = &self.triangles[current as usize];
            let e = chart.ordered[chart.index[&current]];
            for &neighbor in self.neighbors(current as usize) {
                if chart.index.contains_key(&neighbor) {
                    continue;
                }
                let nt = &self.triangles[neighbor as usize];
                let old_world = [t.a, t.b, t.c];
                let old_flat = [e.a, e.b, e.c];
                let new_world = [nt.a, nt.b, nt.c];
                let (mut a, mut b, mut na, mut nb) = (-1i32, -1i32, -1i32, -1i32);
                // 位置の鍵（溶接の格子）が同じ頂点どうしが、共有する辺の端（最初の 1 組と、i・j とも別の次の 1 組）
                let old_keys = old_world.map(|p| position_key(p, tolerance));
                let new_keys = new_world.map(|p| position_key(p, tolerance));
                for (i, old_key) in old_keys.iter().enumerate() {
                    for (j, new_key) in new_keys.iter().enumerate() {
                        if old_key == new_key {
                            if a < 0 {
                                a = i as i32;
                                na = j as i32;
                            } else if i as i32 != a && j as i32 != na {
                                b = i as i32;
                                nb = j as i32;
                            }
                        }
                    }
                }
                if b < 0 {
                    continue;
                }
                let (a, b, na, nb) = (a as usize, b as usize, na as usize, nb as usize);
                let other = 3 - a - b;
                let next_other = 3 - na - nb;
                let edge = old_flat[b] - old_flat[a];
                let length = v2_magnitude(edge);
                if length <= 1e-12 {
                    continue;
                }
                let along = Vec2::new(edge.x / length, edge.y / length);
                let across = Vec2::new(-along.y, along.x);
                let da = sqr_magnitude(new_world[next_other] - new_world[na]);
                let db = sqr_magnitude(new_world[next_other] - new_world[nb]);
                let x = (da - db + length * length) / (2.0 * length);
                let h = fmax(0.0, da - x * x).sqrt();
                let side = if cross2(edge, old_flat[other] - old_flat[a]) >= 0.0 {
                    -1.0f32
                } else {
                    1.0f32
                };
                let mut flat = [Vec2::ZERO; 3];
                flat[na] = old_flat[a];
                flat[nb] = old_flat[b];
                flat[next_other] = old_flat[a] + along * x + across * h * side;
                let min = v2min(flat[0], v2min(flat[1], flat[2]));
                let max = v2max(flat[0], v2max(flat[1], flat[2]));
                let bx = fmax(min.x, fmin(0.0, max.x));
                let by = fmax(min.y, fmin(0.0, max.y));
                if bx * bx + by * by > radius * radius {
                    continue;
                }
                add(
                    &mut chart,
                    Entry {
                        triangle: neighbor,
                        a: flat[0],
                        b: flat[1],
                        c: flat[2],
                    },
                )?;
                queue.push_back(neighbor);
            }
        }
        Ok(chart)
    }
}
