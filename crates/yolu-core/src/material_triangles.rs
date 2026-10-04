//! 三角形の和集合。4×4 サンプルを OR し、共有辺を一方だけへ割り当てる。
use crate::{glam::DVec2, CoreError, Document, SelectionMask, TileCoord};
use rayon::prelude::*;
use std::collections::BTreeMap;
pub type PixelTriangle = [DVec2; 3];
#[derive(Clone)]
struct Prepared {
    p: PixelTriangle,
    bounds: [u32; 4],
}
fn prepare(ts: &[PixelTriangle], w: u32, h: u32) -> Result<Vec<Prepared>, CoreError> {
    if ts.iter().flatten().any(|p| !p.is_finite()) {
        return Err(CoreError::InvalidArgument("三角形の座標"));
    }
    let mut out = Vec::new();
    for &t in ts {
        let [a, mut b, mut c] = t;
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        // 面積がほぼ 0 の三角形は何も覆わない。座標が大きすぎて面積が溢れる（有限でなくなる）三角形も、辺の式が定まらないので
        // 飛ばす（C# も int へ収まらない座標を飛ばす。画布を含む巨大な三角形は、面積が溢れない大きさまでは画布に切って塗る）
        if !area.is_finite() || area.abs() < 1e-12 {
            continue;
        }
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
        }
        let x0 = a.x.min(b.x.min(c.x)).floor().max(0.0).min(w as f64) as u32;
        let y0 = a.y.min(b.y.min(c.y)).floor().max(0.0).min(h as f64) as u32;
        let x1 = (a.x.max(b.x.max(c.x)).ceil() + 1.0).max(0.0).min(w as f64) as u32;
        let y1 = (a.y.max(b.y.max(c.y)).ceil() + 1.0).max(0.0).min(h as f64) as u32;
        if x0 < x1 && y0 < y1 {
            out.push(Prepared {
                p: [a, b, c],
                bounds: [x0, y0, x1, y1],
            });
        }
    }
    Ok(out)
}
fn inside(a: DVec2, b: DVec2, x: f64, y: f64) -> bool {
    let e = (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
    if e != 0.0 {
        e > 0.0
    } else {
        b.y > a.y || (b.y == a.y && b.x < a.x)
    }
}
pub(crate) fn coverage(bits: u16) -> u8 {
    ((bits.count_ones() * 255 + 8) / 16) as u8
}
/// 各タイルの 16 ビットのサンプル。None は画布内の全画素を覆うタイル。
pub(crate) type Samples = BTreeMap<TileCoord, Option<Vec<u16>>>;
/// 並列で一度に作るタイルの数（作業の場所を、全タイル分でなくこの数だけにする。結果は数によらない）。
const SAMPLE_BATCH: usize = 64;
/// 三角形を samples へ足し、サンプルが増えたタイルを（タイルの順に）返す。タイルごとの 16 ビットのサンプルを OR する計算は
/// タイルどうしで独立なので、まとまりごとにワーカーで並列に求め、予算の確かめと表への登録はタイルの順にこのスレッドで行う
/// （止まる所も、止まったときに登録済みのものも逐次と同じ。画素の結果は並列の度合いによらない）。
pub(crate) fn add_samples(
    samples: &mut Samples,
    ts: &[PixelTriangle],
    w: u32,
    h: u32,
    size: u32,
    budget: u64,
    scratch: &mut u64,
) -> Result<Vec<TileCoord>, CoreError> {
    let prepared = prepare(ts, w, h)?;
    let mut bins: BTreeMap<TileCoord, Vec<&Prepared>> = BTreeMap::new();
    for p in &prepared {
        for y in p.bounds[1] / size..=(p.bounds[3] - 1) / size {
            for x in p.bounds[0] / size..=(p.bounds[2] - 1) / size {
                bins.entry(TileCoord::new(x, y)).or_default().push(p);
            }
        }
    }
    // 全部覆ったタイルは変わらない
    let bins: Vec<(TileCoord, Vec<&Prepared>)> = bins
        .into_iter()
        .filter(|(coord, _)| !matches!(samples.get(coord), Some(None)))
        .collect();
    let mut changed = Vec::new();
    for batch in bins.chunks(SAMPLE_BATCH) {
        let computed: Vec<Option<(bool, Vec<u16>)>> = {
            let samples = &*samples;
            batch
                .par_iter()
                .map(|(coord, triangles)| {
                    let mut bits = samples
                        .get(coord)
                        .and_then(|b| b.clone())
                        .unwrap_or_else(|| vec![0; (size * size) as usize]);
                    if !rasterize(&mut bits, *coord, triangles, size) {
                        return None;
                    }
                    let full = (0..(h - coord.y * size).min(size)).all(|y| {
                        (0..(w - coord.x * size).min(size))
                            .all(|x| bits[(y * size + x) as usize] == u16::MAX)
                    });
                    Some((full, bits))
                })
                .collect()
        };
        for ((coord, _), result) in batch.iter().zip(computed) {
            let Some((full, bits)) = result else {
                continue;
            };
            if !full && !samples.contains_key(coord) {
                *scratch += 16 + 2 * (size as u64) * (size as u64);
                if *scratch > budget {
                    return Err(CoreError::StrokeBudgetExceeded);
                }
            }
            samples.insert(*coord, if full { None } else { Some(bits) });
            changed.push(*coord);
        }
    }
    Ok(changed)
}
/// タイルの画素のサンプルへ、そのタイルに掛かる三角形のビットを OR する。増えたビットがあれば true。
fn rasterize(bits: &mut [u16], coord: TileCoord, triangles: &[&Prepared], size: u32) -> bool {
    let mut grew = false;
    for p in triangles {
        let [a, b, c] = p.p;
        for y in p.bounds[1].max(coord.y * size)..p.bounds[3].min((coord.y + 1) * size) {
            for x in p.bounds[0].max(coord.x * size)..p.bounds[2].min((coord.x + 1) * size) {
                let i = ((y % size) * size + x % size) as usize;
                if bits[i] == u16::MAX {
                    continue;
                }
                let old = bits[i];
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f64 + (sx as f64 + 0.5) / 4.0;
                        let py = y as f64 + (sy as f64 + 0.5) / 4.0;
                        if inside(a, b, px, py) && inside(b, c, px, py) && inside(c, a, px, py) {
                            bits[i] |= 1 << (sy * 4 + sx);
                        }
                    }
                }
                grew |= old != bits[i];
            }
        }
    }
    grew
}
impl SelectionMask {
    pub fn from_triangles(doc: &Document, triangles: &[PixelTriangle]) -> Result<Self, CoreError> {
        let mut samples = Samples::new();
        add_samples(
            &mut samples,
            triangles,
            doc.width(),
            doc.height(),
            doc.tile_size(),
            u64::MAX,
            &mut 0,
        )?;
        let size = doc.tile_size();
        Self::from_amount_tiles(
            doc.width(),
            doc.height(),
            size,
            samples.into_iter().map(|(coord, bits)| {
                let mut a = vec![0; (size * size) as usize];
                for y in 0..(doc.height() - coord.y * size).min(size) {
                    for x in 0..(doc.width() - coord.x * size).min(size) {
                        let i = (y * size + x) as usize;
                        a[i] = bits.as_ref().map_or(255, |b| coverage(b[i]));
                    }
                }
                (coord, a)
            }),
        )
    }
    /// geometry の隣接計算で範囲を辿り、UV を画素座標へ変換する。
    pub fn from_surface_region(
        doc: &Document,
        geometry: &crate::geometry::SurfaceGeometry,
        start: u32,
        kind: crate::geometry::SurfaceRegionKind,
    ) -> Result<Self, CoreError> {
        let triangles = surface_triangles(doc, geometry, start, kind)?;
        Self::from_triangles(doc, &triangles)
    }
}
pub fn surface_triangles(
    doc: &Document,
    geometry: &crate::geometry::SurfaceGeometry,
    start: u32,
    kind: crate::geometry::SurfaceRegionKind,
) -> Result<Vec<PixelTriangle>, CoreError> {
    if start as usize >= geometry.triangles().len() {
        return Err(CoreError::InvalidArgument("三角形番号"));
    }
    Ok(crate::geometry::region(geometry, start, kind)
        .iter()
        .map(|i| {
            let t = &geometry.triangles()[*i as usize];
            [t.uv_a, t.uv_b, t.uv_c].map(|p| {
                DVec2::new(
                    p.x as f64 * doc.width() as f64,
                    p.y as f64 * doc.height() as f64,
                )
            })
        })
        .collect())
}
