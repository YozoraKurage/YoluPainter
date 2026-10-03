//! CPU の合成（C# の CpuCompositor の Plan・EvaluatePixel・BlendRect・ClipRect）。
//!
//! - 層は下から上へ、透明から始めて 1 層ずつ重ね、層ごとに RGBA8 へ丸める。
//! - クリッピング: 一番下でなくクリッピングの印のある層は、すぐ下の印の無い層（下地）の組に入る。下地の色に組の層を
//!   `clip_onto` で重ね（下地のアルファのまま）、それを下地の不透明度・モードで下へ重ねる。下地が見えない（非表示・不透明度 0・
//!   空）なら、組の層も描かない。
//! - 画素の値は自分の入力だけで決まるので、どのスレッドがどの行を受け持っても同じバイトになる。
//! - 行の核（`blend_span`・`clip_span`）は C# の BlendRect・ClipRect と同じ近道（上が透明・不透明で量 1・下が透明・下が不透明）
//!   だけを使い、どれも式そのもののバイトを出す。

use rayon::prelude::*;

use crate::blend::{blend, blend_rgb, clip_onto, separable_table, MIN_SHORTCUT_ALPHA};
use crate::document::Layer;
use crate::math::{to_byte, UNIT};
use crate::surface::{Surface, Tile};
use crate::types::{BlendMode, Channel, Rect, Rgba8, RowOrder, TileCoord};

/// 合成の 1 段: 印の無い層（下地）と、その組に入るクリッピングされた層（下から上）。
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub layer: usize,
    pub opacity: f64,
    pub mode: BlendMode,
    pub clips: Vec<ClipEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct ClipEntry {
    pub layer: usize,
    pub opacity: f64,
    pub mode: BlendMode,
}

/// その層がこのチャンネルで何かを出せるか（C# の Active: 表示・不透明度 > 0・チャンネルが有効・面がある）。
fn active(layer: &Layer, channel: Channel) -> bool {
    layer.visible
        && layer.opacity > 0.0
        && layer.is_channel_enabled(channel)
        && layer.surface(channel).is_some()
}

/// PassThrough は Normal として重ねる（グループの印。M1 の層には付かない）。
fn mode_of(mode: BlendMode) -> BlendMode {
    if mode == BlendMode::PassThrough {
        BlendMode::Normal
    } else {
        mode
    }
}

/// 層をクリッピングの組にまとめる（C# の PlanLevel、グループの無い一番上の段）。見えない下地はその組ごと落とす。
pub(crate) fn plan(layers: &[Layer], channel: Channel) -> Vec<Entry> {
    let mut plan = Vec::new();
    for k in 0..layers.len() {
        if k > 0 && layers[k].clipping {
            continue; // 下の下地の組に入る（下地が落ちたなら一緒に落ちる）
        }
        if !active(&layers[k], channel) {
            continue;
        }
        let mut entry = Entry {
            layer: k,
            opacity: layers[k].opacity,
            mode: mode_of(layers[k].blend_mode),
            clips: Vec::new(),
        };
        let mut m = k + 1;
        while m < layers.len() && layers[m].clipping {
            if active(&layers[m], channel) {
                entry.clips.push(ClipEntry {
                    layer: m,
                    opacity: layers[m].opacity,
                    mode: mode_of(layers[m].blend_mode),
                });
            }
            m += 1;
        }
        plan.push(entry);
    }
    plan
}

/// 画素 1 つの合成（C# の EvaluatePixel。行の核と照らし合わせる参照の式）。
pub(crate) fn composite_pixel(layers: &[Layer], channel: Channel, x: u32, y: u32) -> Rgba8 {
    let mut result = Rgba8::TRANSPARENT;
    for entry in plan(layers, channel) {
        let mut group = layers[entry.layer].pixel_or_transparent(channel, x, y);
        for clip in &entry.clips {
            group = clip_onto(
                group,
                layers[clip.layer].pixel_or_transparent(channel, x, y),
                clip.opacity,
                clip.mode,
            );
        }
        result = blend(result, group, entry.opacity, entry.mode);
    }
    result
}

/// 1 行ぶんの上の画素: 一様なタイルは 4 バイトを刻み 0 で読む。
#[derive(Clone, Copy)]
struct Source<'a> {
    bytes: &'a [u8],
    step: usize,
}

/// 下（res）に上（src）を量 amount・モード mode で重ねる（C# の BlendRect の 1 行。各画素は `blend` と同じバイト）。
#[inline]
fn blend_span(
    res: &mut [u8],
    src: Source<'_>,
    amount: f64,
    mode: BlendMode,
    table: Option<&[f64]>,
) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = res.len() / 4;
    let (sb, step) = (src.bytes, src.step);
    for i in 0..count {
        let r = i * 4;
        let s = i * step;
        let s_a = sb[s + 3];
        if s_a == 0 {
            continue; // 上が透明: 下のまま
        }
        if simple && s_a == 255 && amount == 1.0 {
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = 255;
            continue;
        }
        let sa = UNIT[s_a as usize] * amount;
        if sa <= 0.0 {
            continue;
        }
        let d_a = res[r + 3];
        if d_a == 0 && sa >= MIN_SHORTCUT_ALPHA {
            // 下が透明: 重みは 0・a_s・0 で色は (a_s·c)/a_s。積が正規化数なら c から 2 ulp 以内で、丸めると c のバイト
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = to_byte(sa);
            continue;
        }
        let (dr, dg, db) = (
            UNIT[res[r] as usize],
            UNIT[res[r + 1] as usize],
            UNIT[res[r + 2] as usize],
        );
        let (sr, sg, sbb) = (
            UNIT[sb[s] as usize],
            UNIT[sb[s + 1] as usize],
            UNIT[sb[s + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (sr, sg, sbb)
        } else if let Some(t) = table {
            (
                t[(res[r] as usize) << 8 | sb[s] as usize],
                t[(res[r + 1] as usize) << 8 | sb[s + 1] as usize],
                t[(res[r + 2] as usize) << 8 | sb[s + 2] as usize],
            )
        } else {
            blend_rgb(mode, dr, dg, db, sr, sg, sbb)
        };
        let (vr, vg, vb);
        if d_a == 255 {
            // 下が不透明: a = a_s + (1 − a_s) はちょうど 1、重みは 1 − a_s・0・a_s（0 の項と ÷1 は値を変えない）
            let t = 1.0 - sa;
            vr = t * dr + sa * br;
            vg = t * dg + sa * bg;
            vb = t * db + sa * bb;
            res[r + 3] = 255;
        } else {
            let da = UNIT[d_a as usize];
            let a = sa + da * (1.0 - sa);
            let wd = (1.0 - sa) * da;
            let ws = (1.0 - da) * sa;
            let wb = da * sa;
            vr = (wd * dr + ws * sr + wb * br) / a;
            vg = (wd * dg + ws * sg + wb * bg) / a;
            vb = (wd * db + ws * sbb + wb * bb) / a;
            res[r + 3] = to_byte(a);
        }
        res[r] = to_byte(vr);
        res[r + 1] = to_byte(vg);
        res[r + 2] = to_byte(vb);
    }
}

/// クリッピングの下地（g）へクリッピングされた層（c）を重ねる（C# の ClipRect の 1 行。各画素は `clip_onto` と同じバイト）。
#[inline]
fn clip_span(g: &mut [u8], c: Source<'_>, amount: f64, mode: BlendMode, table: Option<&[f64]>) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = g.len() / 4;
    let (cb, step) = (c.bytes, c.step);
    for i in 0..count {
        let r = i * 4;
        let s = i * step;
        let c_a = cb[s + 3];
        if c_a == 0 || g[r + 3] == 0 {
            continue; // 量 0、または描く下地が無い
        }
        let a = UNIT[c_a as usize] * amount;
        if a <= 0.0 {
            continue;
        }
        let (dr, dg, db) = (
            UNIT[g[r] as usize],
            UNIT[g[r + 1] as usize],
            UNIT[g[r + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        } else if let Some(t) = table {
            (
                t[(g[r] as usize) << 8 | cb[s] as usize],
                t[(g[r + 1] as usize) << 8 | cb[s + 1] as usize],
                t[(g[r + 2] as usize) << 8 | cb[s + 2] as usize],
            )
        } else {
            blend_rgb(
                mode,
                dr,
                dg,
                db,
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        };
        g[r] = to_byte(dr + (br - dr) * a);
        g[r + 1] = to_byte(dg + (bg - dg) * a);
        g[r + 2] = to_byte(db + (bb - db) * a);
    }
}

/// 計画の 1 段に解いた材料（面と表）。
struct Resolved<'a> {
    surface: &'a Surface,
    opacity: f64,
    mode: BlendMode,
    table: Option<&'static [f64]>,
    clips: Vec<ResolvedClip<'a>>,
}
struct ResolvedClip<'a> {
    surface: &'a Surface,
    opacity: f64,
    mode: BlendMode,
    table: Option<&'static [f64]>,
}

/// あるタイルの 1 行の読み元（無いタイルは None）。uniform は一様な色の 4 バイトを置く場所。
#[inline]
fn row_source<'a>(
    tile: Option<&'a Tile>,
    uniform: &'a mut [u8; 4],
    local_row: usize,
    local_x: usize,
    ts: usize,
) -> Option<Source<'a>> {
    match tile? {
        Tile::Uniform(c) => {
            *uniform = c.to_array();
            Some(Source {
                bytes: &uniform[..],
                step: 0,
            })
        }
        Tile::Data(d) => Some(Source {
            bytes: &d[(local_row * ts + local_x) * 4..],
            step: 4,
        }),
    }
}

/// 計画の 1 段の、あるタイルでの下地と組の層のタイル。
type EntryTiles<'a> = (Option<&'a Tile>, Vec<Option<&'a Tile>>);

/// これより仕事（画素 × 層）が少ない合成は、呼んだスレッドだけで行う（ワーカーを起こす方が高くつく。C# と同じ目安）。
const PARALLEL_MINIMUM_WORK: u64 = 1 << 16;

/// 矩形の合成を out（width × height × 4、行の並びは order）へ書く。out の元の中身は使わない。
pub(crate) fn composite_into(
    layers: &[Layer],
    tile_size: u32,
    channel: Channel,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
) {
    if rect.is_empty() {
        return;
    }
    let plan = plan(layers, channel);
    if plan.is_empty() {
        out.fill(0);
        return;
    }
    let resolved: Vec<Resolved<'_>> = plan
        .iter()
        .map(|e| Resolved {
            surface: layers[e.layer]
                .surface(channel)
                .expect("計画の層は面を持つ"),
            opacity: e.opacity,
            mode: e.mode,
            table: separable_table(e.mode),
            clips: e
                .clips
                .iter()
                .map(|c| ResolvedClip {
                    surface: layers[c.layer]
                        .surface(channel)
                        .expect("計画の層は面を持つ"),
                    opacity: c.opacity,
                    mode: c.mode,
                    table: separable_table(c.mode),
                })
                .collect(),
        })
        .collect();
    let layer_count: u64 = plan.iter().map(|e| 1 + e.clips.len() as u64).sum();
    let work = rect.width as u64 * rect.height as u64 * layer_count;
    let threads = if work < PARALLEL_MINIMUM_WORK {
        1
    } else {
        rayon::current_num_threads().max(1)
    };

    // 行の束: タイルの行をまたがない帯を、スレッドが余らない数に割る
    let ts = tile_size;
    let (ry0, ry1) = (rect.y, rect.y + rect.height);
    let tile_rows = (ry1 - 1) / ts - ry0 / ts + 1;
    let pieces_per_tile_row = if threads <= 1 {
        1
    } else {
        ((threads as u32 * 2).div_ceil(tile_rows)).clamp(1, ts.div_ceil(8))
    };
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for ty in ry0 / ts..=(ry1 - 1) / ts {
        let (y0, y1) = ((ty * ts).max(ry0), ((ty + 1) * ts).min(ry1));
        let h = y1 - y0;
        let step = h.div_ceil(pieces_per_tile_row).max(1);
        let mut y = y0;
        while y < y1 {
            let e = (y + step).min(y1);
            bands.push((y, e));
            y = e;
        }
    }
    // out を帯ごとの連続した行へ分ける（TopDown では上の帯が先）
    let row_bytes = rect.width as usize * 4;
    if order == RowOrder::TopDown {
        bands.reverse();
    }
    let mut chunks: Vec<((u32, u32), &mut [u8])> = Vec::with_capacity(bands.len());
    let mut rest = out;
    for &(y0, y1) in &bands {
        let (head, tail) = rest.split_at_mut((y1 - y0) as usize * row_bytes);
        chunks.push(((y0, y1), head));
        rest = tail;
    }
    let run = |((y0, y1), chunk): ((u32, u32), &mut [u8])| {
        composite_band(&resolved, ts, rect, y0, y1, chunk, order)
    };
    if threads <= 1 || chunks.len() <= 1 {
        chunks.into_iter().for_each(run);
    } else {
        chunks.into_par_iter().for_each(run);
    }
}

/// 1 つの帯（タイルの行の中の y0..y1）を合成する。chunk は帯の行（order の並び）。
fn composite_band(
    plan: &[Resolved<'_>],
    tile_size: u32,
    rect: Rect,
    y0: u32,
    y1: u32,
    chunk: &mut [u8],
    order: RowOrder,
) {
    chunk.fill(0); // 透明から（帯ごとに、受け持つスレッドが 0 で埋める）
    let ts = tile_size as usize;
    let ty = y0 / tile_size;
    let rows = (y1 - y0) as usize;
    let row_bytes = rect.width as usize * 4;
    let (rx0, rx1) = (rect.x, rect.x + rect.width);
    let mut group = vec![0u8; (ts.min(rect.width as usize)) * 4];
    for tx in rx0 / tile_size..=(rx1 - 1) / tile_size {
        let coord = TileCoord::new(tx, ty);
        let (x0, x1) = ((tx * tile_size).max(rx0), ((tx + 1) * tile_size).min(rx1));
        let count = (x1 - x0) as usize;
        let local_x = (x0 - tx * tile_size) as usize;
        let out_x = (x0 - rx0) as usize * 4;
        // このタイルに何かある層だけ
        let tiles: Vec<EntryTiles<'_>> = plan
            .iter()
            .map(|e| {
                (
                    e.surface.tile(coord),
                    e.clips.iter().map(|c| c.surface.tile(coord)).collect(),
                )
            })
            .collect();
        if tiles.iter().all(|(t, _)| t.is_none()) {
            continue; // 透明のまま
        }
        for row in 0..rows {
            let y = y0 as usize + row;
            let local_row = y - ty as usize * ts;
            let out_row = match order {
                RowOrder::BottomUp => row,
                RowOrder::TopDown => rows - 1 - row,
            };
            let res = &mut chunk[out_row * row_bytes + out_x..][..count * 4];
            for (e, (tile, clip_tiles)) in plan.iter().zip(&tiles) {
                let mut uniform = [0u8; 4];
                let Some(src) = row_source(*tile, &mut uniform, local_row, local_x, ts) else {
                    continue; // 下地が無い: 組の層も描かない
                };
                if e.clips.is_empty() {
                    blend_span(res, src, e.opacity, e.mode, e.table);
                    continue;
                }
                // クリッピングの組: 下地の色を作業の行へ写し、組の層を重ねてから下へ
                let g = &mut group[..count * 4];
                if src.step == 0 {
                    for p in g.chunks_exact_mut(4) {
                        p.copy_from_slice(&src.bytes[..4]);
                    }
                } else {
                    g.copy_from_slice(&src.bytes[..count * 4]);
                }
                for (c, ct) in e.clips.iter().zip(clip_tiles) {
                    let mut cu = [0u8; 4];
                    if let Some(cs) = row_source(*ct, &mut cu, local_row, local_x, ts) {
                        clip_span(g, cs, c.opacity, c.mode, c.table);
                    }
                }
                blend_span(
                    res,
                    Source { bytes: g, step: 4 },
                    e.opacity,
                    e.mode,
                    e.table,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::to_byte;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn byte(&mut self) -> u8 {
            let r = self.next();
            match r & 7 {
                0 => 0,
                1 => 255,
                _ => (r >> 8) as u8,
            }
        }
    }

    /// 行の核が画素ごとの式（blend・clip_onto）と同じバイトを出すか、近道の境（透明・不透明・量 1・極小の量）を含めて。
    #[test]
    fn spans_match_the_pixel_formulas() {
        let mut rng = Rng(7);
        let amounts = [
            1.0,
            0.7,
            0.5,
            1.0 / 255.0,
            1e-305,
            f64::from_bits(1),
            0.99999999,
        ];
        for mode in BlendMode::LAYER_MODES {
            let table = separable_table(mode);
            for &amount in &amounts {
                let n = 512;
                let below: Vec<u8> = (0..n * 4).map(|_| rng.byte()).collect();
                let over: Vec<u8> = (0..n * 4).map(|_| rng.byte()).collect();
                let mut res = below.clone();
                blend_span(
                    &mut res,
                    Source {
                        bytes: &over,
                        step: 4,
                    },
                    amount,
                    mode,
                    table,
                );
                let mut g = below.clone();
                clip_span(
                    &mut g,
                    Source {
                        bytes: &over,
                        step: 4,
                    },
                    amount,
                    mode,
                    table,
                );
                for i in 0..n {
                    let d = Rgba8::from_slice(&below[i * 4..]);
                    let s = Rgba8::from_slice(&over[i * 4..]);
                    assert_eq!(
                        Rgba8::from_slice(&res[i * 4..]),
                        blend(d, s, amount, mode),
                        "{mode:?} {amount} {d:?} {s:?}"
                    );
                    assert_eq!(
                        Rgba8::from_slice(&g[i * 4..]),
                        clip_onto(d, s, amount, mode),
                        "clip {mode:?} {amount}"
                    );
                }
            }
        }
        // 一様な読み元（刻み 0）
        let mut res = vec![10u8, 20, 30, 128, 0, 0, 0, 0];
        blend_span(
            &mut res,
            Source {
                bytes: &[200, 100, 50, 77],
                step: 0,
            },
            1.0,
            BlendMode::Screen,
            None,
        );
        assert_eq!(
            Rgba8::from_slice(&res[4..]),
            Rgba8::new(200, 100, 50, to_byte(77.0 / 255.0))
        );
    }
}
