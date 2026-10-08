//! 帯の写し（[`SeamBand`]）: UV アイランドの外の帯（継ぎ目の縁から帯の幅まで）のテクセルごとに、3D で縁の向こう側のアイランドのどのテクセルを
//! （双線形の重みで）読めばよいか。レイヤーのフィルターの近傍の段が、継ぎ目をまたいで読むのに使う（`filter::Options::seams`）。
//!
//! 作り方:
//! - 継ぎ目の縁ごとに、縁の三角形 T の平面に、3D で縁を共有する相手の三角形 N と、N から UV でつながる（継ぎ目を越えない）三角形を
//!   幅優先で平らに展開する（辺の長さを 3D のまま保つ）。展開した三角形を T の UV の写像で T の UV の画素の空間へ置き、帯のテクセルの中心が
//!   入る展開の三角形の重心座標から、その三角形の本当の UV（相手のアイランドの中）を読む所にする。アイランドどうしの拡大率・向き・鏡映の違いは、この写像が
//!   持つ（補間で読む）。展開は相手のアイランドの中だけ（もう 1 つ継ぎ目を越えた先は読まない）で、1 つの縁あたり [`MAX_CHART_TRIANGLES`] まで。
//! - 帯のテクセルは、どの三角形の中心も覆わない（アイランドの図でアイランドの外の）テクセルのうち、縁の線分から帯の幅の内にあるもの。いくつかの縁が
//!   同じテクセルを取り合うときは、縁の線分にいちばん近い縁のもの。同じ近さ（重なった UV では 1 テクセル以内）で読む所が 1 テクセルより
//!   違えば、そのテクセルは埋めない（重なった UV の両側が別の所を読みたい所。今のまま）。
//! - 読む点は、相手のアイランドの中のテクセル（アイランドの図で相手のアイランドか、重なりの印のもの）だけ。双線形の 4 点のうち外れた点は使わず、重みを
//!   残りで割り直す。どれも使えなければ埋めない。
//! - 1 つの縁のなかで展開が重なったときは、幅優先で先に置いた三角形のもの。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use glam::{DMat2, DVec2};
use rayon::prelude::*;

use super::build::FastMap;
use super::uv_topology::{IslandMap, Parts, SeamEdge, UvTopology, UvTopologyError};
use crate::TileCoord;

/// 1 つの継ぎ目の縁で展開する三角形の上限。
pub const MAX_CHART_TRIANGLES: usize = 4096;
/// 帯を埋める並列の単位（行の数）。
const STRIP: u32 = 64;

/// 継ぎ目の縁 1 つの展開（展開できなければ None）と、相手のアイランドの UV の向きの回転、展開が上限で止まったか。
type Built = (Option<Chart>, [f64; 4], bool);

/// 行の帯 1 つ（`STRIP` 行）の帯のテクセル。行 `local` のテクセルは `entries[rows[local]..rows[local + 1]]`（x の順）。
#[derive(Debug, PartialEq)]
struct StripEntries {
    rows: Vec<u32>,
    entries: Vec<BandEntry>,
}

/// 帯のテクセル 1 つ: 読む所（双線形の 4 点のうち使う点の印と重み）と、継ぎ目の縁（法線を回す向き）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BandEntry {
    /// 行の中の x。
    pub x: u16,
    /// 双線形の左下の点に 1 を足したもの（点は −1 にもなりうる）。
    pub sx: u16,
    pub sy: u16,
    /// 右・上の点への重み（1/256）。
    pub fx: u8,
    pub fy: u8,
    /// 使う点の印（ビット 0 左下・1 右下・2 左上・3 右上）。
    pub taps: u8,
    /// 継ぎ目の縁の番号（`SeamBand::frame`）。
    pub frame: u32,
}

impl BandEntry {
    /// 使う点（テクセルの座標と重み。重みの合計は使う点の分で、割り直す前）。
    pub(crate) fn points(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        let (fx, fy) = (u32::from(self.fx), u32::from(self.fy));
        let (x0, y0) = (u32::from(self.sx), u32::from(self.sy));
        [
            (0u32, 0u32, (256 - fx) * (256 - fy)),
            (1, 0, fx * (256 - fy)),
            (0, 1, (256 - fx) * fy),
            (1, 1, fx * fy),
        ]
        .into_iter()
        .enumerate()
        .filter(|(bit, _)| self.taps & (1 << bit) != 0)
        // sx・sy は点に 1 を足したもの。使う点は 0 以上なので、引いても負にならない
        .map(move |(_, (dx, dy, w))| (x0 + dx - 1, y0 + dy - 1, w))
    }
}

/// 帯の写しを作ったときの数（計測・確かめ用）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeamBandStats {
    /// 継ぎ目の縁の数。
    pub seam_edges: usize,
    /// 展開した三角形の数（全部の縁の合計）。
    pub chart_triangles: usize,
    /// 埋めるテクセルの数。
    pub texels: usize,
    /// 縁どうしが違う所を読もうとして埋めなかったテクセルの数（重なった UV など）。
    pub conflicts: usize,
    /// 展開が上限に達した縁の数。
    pub capped_edges: usize,
    /// 作るのにかかった時間（ミリ秒）。全体と、そのうち展開・帯を埋める所。
    pub build_ms: f64,
    pub chart_ms: f64,
    pub fill_ms: f64,
}

/// 解像度 1 つ・帯の幅 1 つの帯の写し。作った後は読むだけ（`Send + Sync`）。
pub struct SeamBand {
    width: u32,
    height: u32,
    band: u32,
    islands: Arc<IslandMap>,
    /// 行の帯（`STRIP` 行ずつ）ごとの帯のテクセル。1 本にまとめない（作る間、まとめる写しの分の確保を要らなくする）。
    strips: Vec<StripEntries>,
    /// 帯のテクセルの数（全部）。
    texels: usize,
    /// 継ぎ目の縁ごとの、相手のアイランドの UV の向き → こちらの UV の向きの回転（鏡映を含む。行優先の 2×2）。接空間の法線の XY を回す。
    frames: Vec<[f64; 4]>,
    stats: SeamBandStats,
    /// タイルの大きさごとの、タイルの読み書きの対応。
    deps: Mutex<Vec<Arc<TileDeps>>>,
}

impl std::fmt::Debug for SeamBand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeamBand")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("band", &self.band)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl SeamBand {
    /// 帯のテクセル・縁の向き・アイランドの図が同じか（作るのにかかった時間・タイルの対応の覚えは見ない）。
    pub(crate) fn same_content(&self, other: &SeamBand) -> bool {
        (self.width, self.height, self.band, self.texels)
            == (other.width, other.height, other.band, other.texels)
            && self.strips == other.strips
            && self.frames == other.frames
            && self.islands == other.islands
    }
}

impl SeamBand {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// 帯の幅（テクセル）。
    pub fn band(&self) -> u32 {
        self.band
    }
    /// アイランドの図（アイランドの外のテクセルは、近傍の段の後で段の入力のまま戻す）。
    pub fn islands(&self) -> &Arc<IslandMap> {
        &self.islands
    }
    pub fn stats(&self) -> SeamBandStats {
        self.stats
    }
    /// 埋めるテクセルの数。
    pub fn texel_count(&self) -> usize {
        self.texels
    }
    /// 持っているバイト数（名目。アイランドの図は別）。
    pub fn bytes(&self) -> u64 {
        (self
            .strips
            .iter()
            .map(|s| s.rows.len() * 4 + s.entries.len() * std::mem::size_of::<BandEntry>())
            .sum::<usize>()
            + self.frames.len() * 32) as u64
    }
    /// 帯のテクセル (x, y) が読む点（テクセルの座標と、合計 1 の重み）。帯でなければ None。
    pub fn taps(&self, x: u32, y: u32) -> Option<Vec<(u32, u32, f64)>> {
        if y >= self.height {
            return None;
        }
        let e = self.row_entries(y, x, x + 1).first()?;
        let total: u32 = e.points().map(|p| p.2).sum();
        Some(
            e.points()
                .map(|(px, py, w)| (px, py, f64::from(w) / f64::from(total)))
                .collect(),
        )
    }
    /// 行 y の、x が x0..x1 の帯のテクセル。
    pub(crate) fn row_entries(&self, y: u32, x0: u32, x1: u32) -> &[BandEntry] {
        let strip = &self.strips[(y / STRIP) as usize];
        let local = (y % STRIP) as usize;
        let row = &strip.entries[strip.rows[local] as usize..strip.rows[local + 1] as usize];
        let a = row.partition_point(|e| u32::from(e.x) < x0);
        let b = row.partition_point(|e| u32::from(e.x) < x1);
        &row[a..b]
    }
    /// 継ぎ目の縁の向きの回転（行優先の 2×2）。
    pub(crate) fn frame(&self, index: u32) -> [f64; 4] {
        self.frames
            .get(index as usize)
            .copied()
            .unwrap_or([1.0, 0.0, 0.0, 1.0])
    }

    /// タイルの大きさ tile_size での、タイルの読み書きの対応（覚えていればそれ）。
    pub(crate) fn deps(&self, tile_size: u32) -> Arc<TileDeps> {
        let mut deps = self.deps.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = deps.iter().find(|d| d.tile_size == tile_size) {
            return d.clone();
        }
        let d = Arc::new(TileDeps::new(self, tile_size));
        deps.push(d.clone());
        d
    }
}

/// タイルの読み書きの対応: 帯のテクセルのあるタイル → その読む点のあるタイル（と逆）。
pub(crate) struct TileDeps {
    tile_size: u32,
    cols: u32,
    rows: u32,
    sources_at: Vec<u32>,
    sources: Vec<u32>,
    dependents_at: Vec<u32>,
    dependents: Vec<u32>,
}

impl TileDeps {
    fn new(band: &SeamBand, tile_size: u32) -> TileDeps {
        let cols = band.width.div_ceil(tile_size);
        let rows = band.height.div_ceil(tile_size);
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for y in 0..band.height {
            let ty = y / tile_size;
            for e in band.row_entries(y, 0, band.width) {
                let from = ty * cols + u32::from(e.x) / tile_size;
                for (px, py, _) in e.points() {
                    let to = (py / tile_size) * cols + px / tile_size;
                    if pairs.last() != Some(&(from, to)) {
                        pairs.push((from, to));
                    }
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let csr = |pairs: &[(u32, u32)]| {
            let mut at = vec![0u32; (cols * rows) as usize + 1];
            for &(a, _) in pairs {
                at[a as usize + 1] += 1;
            }
            for i in 0..(cols * rows) as usize {
                at[i + 1] += at[i];
            }
            (at, pairs.iter().map(|p| p.1).collect::<Vec<u32>>())
        };
        let (sources_at, sources) = csr(&pairs);
        let mut back: Vec<(u32, u32)> = pairs.iter().map(|&(a, b)| (b, a)).collect();
        back.sort_unstable();
        let (dependents_at, dependents) = csr(&back);
        TileDeps {
            tile_size,
            cols,
            rows,
            sources_at,
            sources,
            dependents_at,
            dependents,
        }
    }
    fn coord(&self, i: u32) -> TileCoord {
        TileCoord::new(i % self.cols, i / self.cols)
    }
    fn index(&self, c: TileCoord) -> Option<usize> {
        (c.x < self.cols && c.y < self.rows).then(|| (c.y * self.cols + c.x) as usize)
    }
    /// タイル c の帯のテクセルが読む点のあるタイル。
    pub(crate) fn sources(&self, c: TileCoord) -> impl Iterator<Item = TileCoord> + '_ {
        let range = self.index(c).map_or(0..0, |i| {
            self.sources_at[i] as usize..self.sources_at[i + 1] as usize
        });
        self.sources[range].iter().map(|&i| self.coord(i))
    }
    /// タイル c の画素を読む帯のテクセルのあるタイル。
    pub(crate) fn dependents(&self, c: TileCoord) -> impl Iterator<Item = TileCoord> + '_ {
        let range = self.index(c).map_or(0..0, |i| {
            self.dependents_at[i] as usize..self.dependents_at[i + 1] as usize
        });
        self.dependents[range].iter().map(|&i| self.coord(i))
    }
}

/// 展開した三角形 1 つ: こちらの UV の画素の空間での位置（v）と、本当の UV の画素の位置（w）。
struct ChartTri {
    v: [DVec2; 3],
    w: [DVec2; 3],
}

/// 展開した三角形を塗るときの係数（三角形ごとにその場で作る。覚えておく展開を小さく保つ）。
struct Raster {
    v: [DVec2; 3],
    e1: DVec2,
    e2: DVec2,
    det: f64,
    lo: DVec2,
    hi: DVec2,
}

impl ChartTri {
    fn new(v: [DVec2; 3], w: [DVec2; 3]) -> Option<ChartTri> {
        let t = ChartTri { v, w };
        let det = t.raster().det;
        (det.is_finite() && det.abs() > 1e-12 && w.iter().all(|p| p.is_finite())).then_some(t)
    }
    fn raster(&self) -> Raster {
        let v = self.v;
        let (e1, e2) = (v[1] - v[0], v[2] - v[0]);
        Raster {
            v,
            e1,
            e2,
            det: e1.x * e2.y - e1.y * e2.x,
            lo: v[0].min(v[1]).min(v[2]),
            hi: v[0].max(v[1]).max(v[2]),
        }
    }
}

impl Raster {
    /// 高さ y の横の線と三角形の重なる x の範囲（無ければ None）。
    fn span(&self, y: f64) -> Option<(f64, f64)> {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..3 {
            let (p, q) = (self.v[i], self.v[(i + 1) % 3]);
            if (p.y - y) * (q.y - y) > 0.0 {
                continue;
            }
            if p.y == q.y {
                lo = lo.min(p.x.min(q.x));
                hi = hi.max(p.x.max(q.x));
            } else {
                let x = p.x + (q.x - p.x) * ((y - p.y) / (q.y - p.y));
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        (lo <= hi).then_some((lo, hi))
    }
    /// 点 p の重心座標（B・C の重み。A は 1 − 2 つ）。
    fn bary(&self, p: DVec2) -> (f64, f64) {
        let d = p - self.v[0];
        (
            (d.x * self.e2.y - d.y * self.e2.x) / self.det,
            (self.e1.x * d.y - self.e1.y * d.x) / self.det,
        )
    }
}

/// 継ぎ目の縁 1 つの展開。
struct Chart {
    /// 縁の受け持ちの所。
    region: Region,
    /// 相手のアイランドの番号。
    island: u32,
    /// 縁の内側のテクセルが重なった UV。
    overlapped: bool,
    tris: Vec<ChartTri>,
    /// 帯がかかる行（含む）。
    y0: u32,
    y1: u32,
}

fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// 継ぎ目の縁 1 つの受け持ちの所（こちらの UV の画素）: 縁の外の、縁から帯の幅までの所のうち、縁に垂直な帯と、両端の角の外の扇形の、
/// この縁の帯に接する半分。アイランドの縁が一筋にたどれない端では、端の外の扇形を全部受け持つ。受け持ちの境目は半テクセル重ね、重なった所は
/// 近い縁・同じ近さの決まりで決める。
struct Region {
    /// 縁の線分。
    a: DVec2,
    b: DVec2,
    /// 縁の向き（a → b の単位）と長さ。
    dir: DVec2,
    len: f64,
    /// アイランドの縁をたどった前の縁・後の縁の向き（単位。たどれなければ None）。
    prev: Option<DVec2>,
    next: Option<DVec2>,
    band: f64,
}

impl Region {
    /// 点 p が受け持ちから margin（画素）の内にあるか（margin 0 で受け持ちそのもの）。
    fn claims(&self, p: DVec2, margin: f64) -> bool {
        if segment_distance(p, self.a, self.b) > self.band + margin {
            return false;
        }
        let along = (p - self.a).dot(self.dir);
        if along >= -0.5 - margin && along <= self.len + 0.5 + margin {
            return true;
        }
        let (end, ahead) = if along > self.len {
            (self.b, self.next)
        } else {
            (self.a, self.prev.map(|d| -d))
        };
        let Some(ahead) = ahead else {
            return true;
        };
        // 隣の縁の帯の中なら隣の縁のもの。角の外の扇形は、二等分する線でこちらの帯に接する半分（隣の縁の線より、この縁の線から
        // 遠い所）だけ
        let q = p - end;
        q.dot(ahead) <= margin && cross2(self.dir, q).abs() + margin >= cross2(ahead, q).abs()
    }
}

/// 線分 a–b からの距離が da・db の点のうち、a→b の side の側（cross の符号）のもの。
fn place(a: DVec2, b: DVec2, da: f64, db: f64, side: f64) -> Option<DVec2> {
    let d = b - a;
    let l = d.length();
    if l.is_nan() || l <= 0.0 || side == 0.0 {
        return None;
    }
    let x = (da * da - db * db + l * l) / (2.0 * l);
    let y = (da * da - x * x).max(0.0).sqrt();
    let dir = d / l;
    let perp = DVec2::new(-dir.y, dir.x);
    let p = a + dir * x + perp * (y * side.signum());
    p.is_finite().then_some(p)
}

fn segment_distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let e = b - a;
    let t = ((p - a).dot(e) / e.length_squared()).clamp(0.0, 1.0);
    (p - (a + e * t)).length()
}

/// 相手の UV の向き → こちらの UV の向き（ヤコビアン J）の、回転・鏡映の部分（極分解。三角関数を使わない）。
fn polar(j: DMat2) -> [f64; 4] {
    let (a, b, c, d) = (j.x_axis.x, j.y_axis.x, j.x_axis.y, j.y_axis.y);
    let rotation = |p: f64, q: f64| -> Option<[f64; 4]> {
        let l = (p * p + q * q).sqrt();
        (l > 1e-12 && l.is_finite()).then(|| [p / l, -q / l, q / l, p / l])
    };
    let reflection = |p: f64, q: f64| -> Option<[f64; 4]> {
        let l = (p * p + q * q).sqrt();
        (l > 1e-12 && l.is_finite()).then(|| [p / l, q / l, q / l, -p / l])
    };
    let r = if a * d - b * c >= 0.0 {
        rotation(a + d, c - b)
    } else {
        reflection(a - d, c + b)
    };
    r.unwrap_or([1.0, 0.0, 0.0, 1.0])
}

impl Chart {
    #[allow(clippy::too_many_arguments)]
    fn new(
        topology: &UvTopology,
        parts: &Parts,
        islands: &IslandMap,
        s: &SeamEdge,
        width: u32,
        height: u32,
        band: u32,
    ) -> (Option<Chart>, [f64; 4], bool) {
        let identity = [1.0, 0.0, 0.0, 1.0];
        let g = topology.geometry();
        let tris = g.triangles();
        let scale = DVec2::new(width as f64, height as f64);
        let t = &tris[s.triangle as usize];
        let p = [t.a.as_dvec3(), t.b.as_dvec3(), t.c.as_dvec3()];
        let u = [
            t.uv_a.as_dvec2() * scale,
            t.uv_b.as_dvec2() * scale,
            t.uv_c.as_dvec2() * scale,
        ];
        let (ia, ib) = (s.edge as usize, (s.edge as usize + 1) % 3);
        let ic = 3 - ia - ib;
        // T の平面の 2D の座標（辺の長さを保つ）と、そこから T の UV の画素への写像
        let e1 = p[1] - p[0];
        let e2 = p[2] - p[0];
        let normal = e1.cross(e2);
        let len1 = e1.length();
        if !(normal.length() > 1e-20 && len1 > 0.0 && normal.is_finite()) {
            return (None, identity, false);
        }
        let xh = e1 / len1;
        let yh = normal.normalize().cross(xh);
        let q = [
            DVec2::ZERO,
            DVec2::new(len1, 0.0),
            DVec2::new(e2.dot(xh), e2.dot(yh)),
        ];
        let qm = DMat2::from_cols(q[1] - q[0], q[2] - q[0]);
        let um = DMat2::from_cols(u[1] - u[0], u[2] - u[0]);
        if qm.determinant().abs() <= 1e-30 || um.determinant().abs() <= 1e-12 {
            return (None, identity, false);
        }
        let m = um * qm.inverse();
        let to_uv = |x: DVec2| u[0] + m * (x - q[0]);
        let (a, b) = (u[ia], u[ib]);
        if (b - a).length_squared() <= 1e-18 {
            return (None, identity, false);
        }
        let len = (b - a).length();
        let dir = (b - a) / len;
        let inward = {
            let n = DVec2::new(-dir.y, dir.x);
            if n.dot(u[ic] - a) >= 0.0 {
                n
            } else {
                -n
            }
        };
        // 受け持ちの箱: 縁の外側の帯と、両端の外の扇形（たどれない端は丸ごと）
        let reach = band as f64;
        let out = -inward;
        let along =
            |v: Option<glam::Vec2>| v.map(|v| v.as_dvec2() * scale).filter(|v| v.is_finite());
        let next = along(s.next)
            .map(|c| c - b)
            .filter(|d| d.length() > 1e-12)
            .map(|d| d.normalize());
        let prev = along(s.prev)
            .map(|z| a - z)
            .filter(|d| d.length() > 1e-12)
            .map(|d| d.normalize());
        let region = Region {
            a,
            b,
            dir,
            len,
            prev,
            next,
            band: reach,
        };
        let mut corners = vec![a, b, a + out * reach, b + out * reach];
        for (end, ahead) in [(b, dir), (a, -dir)] {
            let diagonal = (ahead + out).normalize();
            corners.extend([end + ahead * reach, end + diagonal * reach]);
        }
        let lo = corners
            .iter()
            .fold(DVec2::splat(f64::INFINITY), |m, c| m.min(*c))
            - DVec2::splat(2.0);
        let hi = corners
            .iter()
            .fold(DVec2::splat(f64::NEG_INFINITY), |m, c| m.max(*c))
            + DVec2::splat(2.0);
        if hi.x < 0.0 || hi.y < 0.0 || lo.x > width as f64 || lo.y > height as f64 {
            return (None, identity, false);
        }
        // 縁の内側のテクセルが重なった UV か（縁に沿って、0.75 テクセル内側を見る）
        let steps = (len.ceil() as usize).clamp(1, 64);
        let overlapped = (0..steps).any(|k| {
            let at = a + (b - a) * ((k as f64 + 0.5) / steps as f64) + inward * 0.75;
            at.x >= 0.0
                && at.y >= 0.0
                && islands.overlapped(at.x.floor() as u32, at.y.floor() as u32)
        });
        // 相手の三角形を T の平面へ、縁の向こう側に置く
        let side_t = cross2(q[ib] - q[ia], q[ic] - q[ia]);
        let n = &tris[s.partner as usize];
        let pn = [n.a.as_dvec3(), n.b.as_dvec3(), n.c.as_dvec3()];
        let (na, nb) = (s.partner_a as usize, s.partner_b as usize);
        let nc = 3 - na - nb;
        let mut qn = [DVec2::ZERO; 3];
        qn[na] = q[ia];
        qn[nb] = q[ib];
        let Some(c) = place(
            q[ia],
            q[ib],
            (pn[nc] - pn[na]).length(),
            (pn[nc] - pn[nb]).length(),
            -side_t,
        ) else {
            return (None, identity, overlapped);
        };
        qn[nc] = c;
        let mut visited: FastMap<u32, ()> = FastMap::default();
        visited.insert(s.triangle, ());
        visited.insert(s.partner, ());
        let mut queue = std::collections::VecDeque::from([(s.partner, qn)]);
        let mut out: Vec<ChartTri> = Vec::new();
        let mut capped = false;
        let mut frame = identity;
        while let Some((k, qk)) = queue.pop_front() {
            if out.len() >= MAX_CHART_TRIANGLES {
                capped = true;
                break;
            }
            let kt = &tris[k as usize];
            let v = [to_uv(qk[0]), to_uv(qk[1]), to_uv(qk[2])];
            let w = [
                kt.uv_a.as_dvec2() * scale,
                kt.uv_b.as_dvec2() * scale,
                kt.uv_c.as_dvec2() * scale,
            ];
            let Some(tri) = ChartTri::new(v, w) else {
                continue;
            };
            // 受け持ちから離れた三角形は置かず、先へも広げない（頂点か重心が、辺の長さの分広げた受け持ちに入るか）
            let size = (v[0] - v[1])
                .length()
                .max((v[1] - v[2]).length())
                .max((v[2] - v[0]).length());
            let centroid = (v[0] + v[1] + v[2]) / 3.0;
            let r = tri.raster();
            let near = r.hi.x >= lo.x
                && r.lo.x <= hi.x
                && r.hi.y >= lo.y
                && r.lo.y <= hi.y
                && v.iter()
                    .chain([&centroid])
                    .any(|p| region.claims(*p, size + 1.0));
            if !near && k != s.partner {
                continue;
            }
            if out.is_empty() {
                // 相手のアイランドの UV の向き → こちらの UV の向き（相手の三角形 1 つで決める）
                let vm = DMat2::from_cols(v[1] - v[0], v[2] - v[0]);
                let wm = DMat2::from_cols(w[1] - w[0], w[2] - w[0]);
                if wm.determinant().abs() > 1e-12 {
                    frame = polar(vm * wm.inverse());
                }
            }
            out.push(tri);
            // UV でつながる隣（相手のアイランドの中だけ）へ広げる。継ぎ目（その先は別のアイランド）・相手の無い辺では止まる
            for (j, link) in parts.links[k as usize].iter().enumerate() {
                if link.neighbor == u32::MAX || !link.continuous {
                    continue;
                }
                let nbr = link.neighbor;
                if visited.contains_key(&nbr) || parts.island_of[nbr as usize] == 0 {
                    continue;
                }
                let (x, y) = (j, (j + 1) % 3);
                let z = 3 - x - y;
                let (ox, oy) = (link.a as usize, link.b as usize);
                let oz = 3 - ox - oy;
                let o = &tris[nbr as usize];
                let po = [o.a.as_dvec3(), o.b.as_dvec3(), o.c.as_dvec3()];
                let side = cross2(qk[y] - qk[x], qk[z] - qk[x]);
                let Some(pz) = place(
                    qk[x],
                    qk[y],
                    (po[oz] - po[ox]).length(),
                    (po[oz] - po[oy]).length(),
                    -side,
                ) else {
                    continue;
                };
                let mut qo = [DVec2::ZERO; 3];
                qo[ox] = qk[x];
                qo[oy] = qk[y];
                qo[oz] = pz;
                visited.insert(nbr, ());
                queue.push_back((nbr, qo));
            }
        }
        if out.is_empty() {
            return (None, identity, overlapped);
        }
        let y0 = (lo.y - 0.5).floor().max(0.0) as u32;
        let y1 = ((hi.y - 0.5).ceil().max(0.0) as u32).min(height - 1);
        out.shrink_to_fit();
        let chart = Chart {
            region,
            island: parts.island_of[s.partner as usize],
            overlapped,
            tris: out,
            y0,
            y1,
        };
        (Some(chart), frame, capped)
    }
}

/// アイランドの中のテクセル（ビット）。
struct Inside {
    stride: usize,
    words: Vec<u64>,
}

impl Inside {
    fn new(map: &IslandMap) -> Inside {
        let stride = (map.width() as usize).div_ceil(64);
        let mut words = vec![0u64; stride * map.height() as usize];
        for y in 0..map.height() {
            let row = &mut words[y as usize * stride..(y as usize + 1) * stride];
            for r in map.row(y) {
                for x in r.start..r.end {
                    row[x as usize / 64] |= 1 << (x % 64);
                }
            }
        }
        Inside { stride, words }
    }
    fn get(&self, x: u32, y: u32) -> bool {
        self.words[y as usize * self.stride + x as usize / 64] >> (x % 64) & 1 == 1
    }
}

/// 帯の写しを作る。作る間の確保（アイランドの図は作る間も生きている）が `budget` バイトに収まらなければ断る。アイランドの図のほかに生きているのは、
/// - アイランドの中のビット（`Inside`）と、継ぎ目の縁ごとに展開した三角形（`charts`）— 行の帯を埋め終えるまで
/// - 行の帯ごとの作業（`fill_strip` の 4 つの表と、帯のテクセルを集める入れ物の伸び）。同時に動かす数は、残りの半分に収まる数で頭打ちにする
/// - 帯のテクセル（行の帯ごとの結果をそのまま持つ。残りから作業の分を引いた量まで）
///
/// 展開した三角形と帯のテクセルは、作りながらバイト数を数え、超えた時点で途中で断る。
///
/// `parallel` なら縁ごとの展開と行の帯ごとの埋めを rayon で並べ、そうでなければ今のスレッドで順に回す（結果・断る理由は同じ。確保の見積りと
/// 同時に動かす数の頭打ちも同じ式で、逐次のときは実際に同時に動く数が少ないだけ）。
pub(crate) fn build(
    topology: &UvTopology,
    islands: Arc<IslandMap>,
    band: u32,
    budget: u64,
    parallel: bool,
) -> Result<SeamBand, UvTopologyError> {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let over = || UvTopologyError::Budget { budget };
    let clock = Instant::now();
    let parts = topology.parts();
    let (width, height) = (islands.width(), islands.height());
    let strips = height.div_ceil(STRIP) as usize;
    let avail = budget.saturating_sub(islands.bytes());
    let inside_bytes = u64::from(width).div_ceil(64) * 8 * u64::from(height);
    let lists_bytes = strips as u64 * std::mem::size_of::<Vec<u32>>() as u64;
    let fixed = inside_bytes + lists_bytes;
    if fixed > avail {
        return Err(over());
    }
    let inside = Inside::new(&islands);
    let t0 = Instant::now();
    // 展開した三角形: 残りの半分まで
    let chart_limit = (avail - fixed) / 2;
    let chart_held = AtomicU64::new(0);
    let item_bytes = std::mem::size_of::<Built>() as u64;
    let tri_bytes = std::mem::size_of::<ChartTri>() as u64;
    let chart_of = |s: &SeamEdge| -> Result<Built, UvTopologyError> {
        let made = Chart::new(topology, parts, &islands, s, width, height, band);
        let bytes = item_bytes
            + made
                .0
                .as_ref()
                .map_or(0, |c| c.tris.len() as u64 * tri_bytes);
        if chart_held.fetch_add(bytes, Relaxed) + bytes > chart_limit {
            Err(over())
        } else {
            Ok(made)
        }
    };
    let built: Result<Vec<Built>, UvTopologyError> = if parallel {
        parts.seams.par_iter().map(chart_of).collect()
    } else {
        parts.seams.iter().map(chart_of).collect()
    };
    let built = built?;
    let mut charts = Vec::with_capacity(built.len());
    let mut frames = Vec::with_capacity(built.len());
    let mut capped_edges = 0;
    for (c, f, capped) in built {
        charts.push(c);
        frames.push(f);
        capped_edges += usize::from(capped);
    }
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); strips];
    for (i, c) in charts.iter().enumerate() {
        if let Some(c) = c {
            for list in &mut lists[(c.y0 / STRIP) as usize..=(c.y1 / STRIP) as usize] {
                list.push(i as u32);
            }
        }
    }
    let list_bytes: u64 = lists.iter().map(|l| l.capacity() as u64 * 4).sum();
    // 行の帯ごとの作業と帯のテクセルで使える残り
    let rest = (avail - fixed).saturating_sub(chart_held.load(Relaxed) + list_bytes);
    let scratch = u64::from(STRIP.min(height)) * u64::from(width) * STRIP_BYTES_PER_TEXEL;
    if scratch > rest / 2 {
        return Err(over());
    }
    let threads = rayon::current_num_threads().max(1);
    let workers = ((rest / 2 / scratch) as usize).clamp(1, threads);
    let entry_limit = rest - workers as u64 * scratch;
    let entry_held = AtomicU64::new(0);
    let t1 = Instant::now();
    let order: Vec<usize> = (0..strips).collect();
    // 同時に動かせる数が足りるなら、全部を 1 度に並べる（結果は同じ。作業の分だけ数を絞るときだけ、その数ずつ）
    let group = if workers >= threads { strips } else { workers };
    let mut done: Vec<(Vec<u32>, Vec<BandEntry>, usize)> = Vec::with_capacity(strips);
    let strip_of = |&s: &usize| -> Result<(Vec<u32>, Vec<BandEntry>, usize), UvTopologyError> {
        let made = fill_strip(s as u32, &lists[s], &charts, &inside, &islands, band);
        let bytes =
            made.1.len() as u64 * std::mem::size_of::<BandEntry>() as u64 + made.0.len() as u64 * 4;
        if entry_held.fetch_add(bytes, Relaxed) + bytes > entry_limit {
            Err(over())
        } else {
            Ok(made)
        }
    };
    for chunk in order.chunks(group.max(1)) {
        let part: Result<Vec<_>, UvTopologyError> = if parallel {
            chunk.par_iter().map(strip_of).collect()
        } else {
            chunk.iter().map(strip_of).collect()
        };
        done.extend(part?);
    }
    let fill_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let chart_ms = (t1 - t0).as_secs_f64() * 1000.0;
    let chart_triangles = charts.iter().flatten().map(|c| c.tris.len()).sum();
    let mut conflicts = 0;
    let mut texels = 0;
    let strips: Vec<StripEntries> = done
        .into_iter()
        .map(|(counts, entries, c)| {
            conflicts += c;
            texels += entries.len();
            let mut rows = Vec::with_capacity(counts.len() + 1);
            rows.push(0u32);
            rows.extend(counts);
            StripEntries { rows, entries }
        })
        .collect();
    let stats = SeamBandStats {
        seam_edges: parts.seams.len(),
        chart_triangles,
        texels,
        conflicts,
        capped_edges,
        build_ms: clock.elapsed().as_secs_f64() * 1000.0,
        chart_ms,
        fill_ms,
    };
    Ok(SeamBand {
        width,
        height,
        band,
        islands,
        strips,
        texels,
        frames,
        stats,
        deps: Mutex::new(Vec::new()),
    })
}

/// 行の帯の作業の、テクセル 1 つあたりのバイト数: `fill_strip` の `best_d` 8・`best_q` 16・`best_e` 4・`conflict` 1 と、帯のテクセルを
/// 集める入れ物が伸びるとき（最大 2 倍）の分 2 × 16。
const STRIP_BYTES_PER_TEXEL: u64 = (std::mem::size_of::<f64>()
    + std::mem::size_of::<DVec2>()
    + std::mem::size_of::<u32>()
    + std::mem::size_of::<bool>()
    + 2 * std::mem::size_of::<BandEntry>()) as u64;

/// 行の帯 1 つ（STRIP 行）を埋める。返すのは、帯の各行の終わりの位置（帯の始まりからの数）・帯のテクセル・埋めなかったテクセルの数。
fn fill_strip(
    s: u32,
    list: &[u32],
    charts: &[Option<Chart>],
    inside: &Inside,
    islands: &IslandMap,
    band: u32,
) -> (Vec<u32>, Vec<BandEntry>, usize) {
    let (width, height) = (islands.width(), islands.height());
    let y0 = s * STRIP;
    let y1 = (y0 + STRIP).min(height);
    let w = width as usize;
    let n = (y1 - y0) as usize * w;
    let mut best_d = vec![f64::INFINITY; n];
    let mut best_q = vec![DVec2::ZERO; n];
    let mut best_e = vec![u32::MAX; n];
    let mut conflict = vec![false; n];
    let reach = band as f64;
    for &ci in list {
        let Some(c) = &charts[ci as usize] else {
            continue;
        };
        for tri in &c.tris {
            // 中心 (x + 0.5, y + 0.5) が箱にかかる行（重心座標の許しの分、1 つ広く）と、行ごとに三角形にかかる x（少し広く。
            // 入るかは重心座標で決める）
            let r = tri.raster();
            let fy0 = (r.lo.y - 0.5).floor().max(y0 as f64);
            let fy1 = (r.hi.y - 0.5).ceil().min((y1 - 1) as f64);
            if fy0 > fy1 {
                continue;
            }
            for y in fy0 as u32..=fy1 as u32 {
                let row_y = (y as f64 + 0.5).clamp(r.lo.y, r.hi.y);
                let Some((sx0, sx1)) = r.span(row_y) else {
                    continue;
                };
                // 行の中心が三角形の上下の外でも許しで入りうるので、1 画素広げる
                let fx0 = (sx0 - 1.5).floor().max(0.0);
                let fx1 = (sx1 + 0.5).ceil().min((width - 1) as f64);
                if fx0 > fx1 {
                    continue;
                }
                for x in fx0 as u32..=fx1 as u32 {
                    if inside.get(x, y) {
                        continue;
                    }
                    let p = DVec2::new(x as f64 + 0.5, y as f64 + 0.5);
                    if !c.region.claims(p, 0.0) {
                        continue;
                    }
                    let (bb, bc) = r.bary(p);
                    let ba = 1.0 - bb - bc;
                    if ba.min(bb).min(bc) < -1e-9 {
                        continue;
                    }
                    let d = segment_distance(p, c.region.a, c.region.b);
                    if d > reach {
                        continue;
                    }
                    let q = tri.w[0] * ba + tri.w[1] * bb + tri.w[2] * bc;
                    let i = (y - y0) as usize * w + x as usize;
                    let held = best_e[i];
                    if held == ci {
                        continue; // 同じ縁の展開の重なり: 先に置いた三角形のもの
                    }
                    if held == u32::MAX {
                        best_d[i] = d;
                        best_q[i] = q;
                        best_e[i] = ci;
                        continue;
                    }
                    let other = charts[held as usize].as_ref().expect("持っている縁");
                    let tol = if c.overlapped || other.overlapped {
                        1.0
                    } else {
                        1e-4
                    };
                    if d < best_d[i] - tol {
                        best_d[i] = d;
                        best_q[i] = q;
                        best_e[i] = ci;
                        conflict[i] = false;
                    } else if d <= best_d[i] + tol {
                        if (q - best_q[i]).length() > 1.0 {
                            conflict[i] = true;
                        }
                        if d < best_d[i] {
                            best_d[i] = d;
                            best_q[i] = q;
                            best_e[i] = ci;
                        }
                    }
                }
            }
        }
    }
    let mut counts = Vec::with_capacity((y1 - y0) as usize);
    let mut out = Vec::new();
    let mut conflicts = 0;
    for y in y0..y1 {
        let row = (y - y0) as usize * w;
        for x in 0..width {
            let i = row + x as usize;
            let e = best_e[i];
            if e == u32::MAX {
                continue;
            }
            if conflict[i] {
                conflicts += 1;
                continue;
            }
            let c = charts[e as usize].as_ref().expect("持っている縁");
            if let Some(entry) = entry_for(x, best_q[i], c.island, e, islands) {
                out.push(entry);
            }
        }
        counts.push(out.len() as u32);
    }
    // 集める間に伸びた余りを返す（持つのは使う分だけ）
    out.shrink_to_fit();
    (counts, out, conflicts)
}

/// 帯のテクセル x の、相手のアイランドの点 q（UV の画素）を読む印。使える点が無ければ None。
fn entry_for(x: u32, q: DVec2, island: u32, frame: u32, islands: &IslandMap) -> Option<BandEntry> {
    let (width, height) = (islands.width() as i64, islands.height() as i64);
    let (gx, gy) = (q.x - 0.5, q.y - 0.5);
    if !(gx.is_finite() && gy.is_finite()) {
        return None;
    }
    let (fx0, fy0) = (gx.floor(), gy.floor());
    if fx0 < -1.0 || fy0 < -1.0 || fx0 > width as f64 || fy0 > height as f64 {
        return None;
    }
    let (mut x0, mut y0) = (fx0 as i64, fy0 as i64);
    let (mut fx, mut fy) = (
        ((gx - fx0) * 256.0).round() as u32,
        ((gy - fy0) * 256.0).round() as u32,
    );
    if fx >= 256 {
        x0 += 1;
        fx = 0;
    }
    if fy >= 256 {
        y0 += 1;
        fy = 0;
    }
    let mut taps = 0u8;
    for (bit, (dx, dy, weight)) in [
        (0i64, 0i64, (256 - fx) * (256 - fy)),
        (1, 0, fx * (256 - fy)),
        (0, 1, (256 - fx) * fy),
        (1, 1, fx * fy),
    ]
    .into_iter()
    .enumerate()
    {
        let (tx, ty) = (x0 + dx, y0 + dy);
        if weight == 0 || tx < 0 || ty < 0 || tx >= width || ty >= height {
            continue;
        }
        let (tx, ty) = (tx as u32, ty as u32);
        if islands.island(tx, ty) == island || islands.overlapped(tx, ty) {
            taps |= 1 << bit;
        }
    }
    (taps != 0).then(|| BandEntry {
        x: x as u16,
        sx: (x0 + 1) as u16,
        sy: (y0 + 1) as u16,
        fx: fx as u8,
        fy: fy as u8,
        taps,
        frame,
    })
}

/// 近傍のフィルターの半径の最大から、帯の幅（8 以上の 2 の累乗、512 まで。0 は継ぎ目をまたがない）。半径を少しずつ変えても帯の写しを
/// 作り直さないよう、段の幅に丸める（広い帯の写しは、狭い半径でもそのまま使える）。
pub fn seam_band_width(max_halo: u32) -> u32 {
    if max_halo == 0 {
        0
    } else {
        max_halo
            .next_power_of_two()
            .clamp(8, super::uv_topology::MAX_SEAM_BAND)
    }
}
