use crate::{
    smart::SmartResampling,
    surface::{Growth, Tile},
    CoreError, Rgba8, Surface, TileCoord,
};
fn axis(source: u32, target: u32, how: SmartResampling) -> Vec<Vec<(u32, f64)>> {
    let (s, t) = (source as i64, target as i64);
    (0..t)
        .map(|i| match how {
            SmartResampling::Nearest => vec![(((2 * i + 1) * s / (2 * t)) as u32, 1.0)],
            SmartResampling::Bilinear => {
                let num = (2 * i + 1) * s - t;
                let den = 2 * t;
                let j = num.div_euclid(den);
                let rem = num - j * den;
                let (a, b) = (j.max(0), (j + 1).min(s - 1));
                if rem == 0 || a == b {
                    vec![(if rem == 0 { j.clamp(0, s - 1) } else { a } as u32, 1.0)]
                } else {
                    let f = rem as f64 / den as f64;
                    vec![(a as u32, 1.0 - f), (b as u32, f)]
                }
            }
            SmartResampling::Area => {
                let (lo, hi) = (i * s, (i + 1) * s);
                (lo / t..=(hi - 1) / t)
                    .map(|j| {
                        (
                            j as u32,
                            ((hi.min((j + 1) * t) - lo.max(j * t)) as f64) / s as f64,
                        )
                    })
                    .collect()
            }
        })
        .collect()
}
fn pixel(
    source: &Surface,
    xs: &[(u32, f64)],
    ys: &[(u32, f64)],
    normal: bool,
) -> Result<Rgba8, CoreError> {
    let mut first = None;
    let mut same = true;
    let (mut a, mut r, mut g, mut b, mut zw, mut zr, mut zg, mut zb) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for &(y, wy) in ys {
        for &(x, wx) in xs {
            let w = wy * wx;
            if w <= 0.0 {
                continue;
            }
            let p = source.pixel(x, y)?;
            if let Some(f) = first {
                if f != p {
                    same = false;
                }
            } else {
                first = Some(p);
            }
            if p.a == 0 {
                zw += w;
                zr += w * p.r as f64;
                zg += w * p.g as f64;
                zb += w * p.b as f64;
                continue;
            }
            let k = w * p.a as f64;
            a += k;
            r += k * p.r as f64;
            g += k * p.g as f64;
            b += k * p.b as f64;
        }
    }
    if same {
        return Ok(first.unwrap_or(Rgba8::TRANSPARENT));
    }
    let byte = crate::math::to_byte;
    let alpha = byte(a / 255.0);
    Ok(if alpha == 0 {
        if zw > 0.0 {
            Rgba8::new(
                byte(zr / zw / 255.0),
                byte(zg / zw / 255.0),
                byte(zb / zw / 255.0),
                0,
            )
        } else {
            Rgba8::TRANSPARENT
        }
    } else if normal {
        crate::normal::encode(
            2.0 * r / a / 255.0 - 1.0,
            2.0 * g / a / 255.0 - 1.0,
            2.0 * b / a / 255.0 - 1.0,
            alpha,
        )
    } else {
        Rgba8::new(
            byte(r / a / 255.0),
            byte(g / a / 255.0),
            byte(b / a / 255.0),
            alpha,
        )
    })
}
/// 目標のタイルの並び（`tile` 画素ごと）それぞれが読む元のタイルの範囲（最小と最大の番号）。
fn spans(axis: &[Vec<(u32, f64)>], tile: u32, source_tile: u32) -> Vec<(u32, u32)> {
    axis.chunks(tile as usize)
        .map(|run| {
            let first = run.iter().map(|taps| taps[0].0).min().expect("1 画素以上");
            let last = run
                .iter()
                .map(|taps| taps[taps.len() - 1].0)
                .max()
                .expect("1 画素以上");
            (first / source_tile, last / source_tile)
        })
        .collect()
}
/// 目標のタイルが読む元のタイルの様子（C# の CanvasResampler.Resample と同じ分け方）。
enum Reads {
    /// 読む元のタイルが 1 つも無い（結果は全画素 0 で、タイルを持たない）。
    Nothing,
    /// 読む元のタイルが全部あり、全部が同じ一様な色（どの画素を読んでも同じ色なので、計算せずその色で埋める）。
    Uniform(Rgba8),
    Mixed,
}
fn reads(source: &Surface, xs: (u32, u32), ys: (u32, u32)) -> Reads {
    let (mut any, mut uniform, mut color) = (false, true, None);
    for y in ys.0..=ys.1 {
        for x in xs.0..=xs.1 {
            match source.tile(TileCoord { x, y }) {
                None => uniform = false,
                Some(tile) => {
                    any = true;
                    match tile {
                        Tile::Uniform(c) if color.is_none_or(|first| first == *c) => {
                            color = Some(*c);
                        }
                        _ => uniform = false,
                    }
                }
            }
        }
    }
    match (any, uniform, color) {
        (false, _, _) => Reads::Nothing,
        (true, true, Some(c)) => Reads::Uniform(c),
        _ => Reads::Mixed,
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn surface(
    source: &Surface,
    w: u32,
    h: u32,
    tile: u32,
    how: SmartResampling,
    normal: bool,
    budget: u64,
) -> Result<Surface, CoreError> {
    let mut out = Surface::new(w, h, tile);
    if source.tile_count() == 0 {
        return Ok(out);
    }
    let xs = axis(source.width(), w, how);
    let ys = axis(source.height(), h, how);
    let (x_reads, y_reads) = (
        spans(&xs, tile, source.tile_size()),
        spans(&ys, tile, source.tile_size()),
    );
    let mut bytes = vec![0; out.tile_bytes()];
    for ty in 0..out.tile_rows() {
        for tx in 0..out.tile_columns() {
            let uniform = match reads(source, x_reads[tx as usize], y_reads[ty as usize]) {
                Reads::Nothing => continue,
                Reads::Uniform(c) => Some(c),
                Reads::Mixed => None,
            };
            bytes.fill(0);
            for y in 0..tile.min(h - ty * tile) {
                for x in 0..tile.min(w - tx * tile) {
                    let p = match uniform {
                        Some(c) => c,
                        None => pixel(
                            source,
                            &xs[(tx * tile + x) as usize],
                            &ys[(ty * tile + y) as usize],
                            normal,
                        )?,
                    };
                    let i = ((y * tile + x) * 4) as usize;
                    bytes[i..i + 4].copy_from_slice(&[p.r, p.g, p.b, p.a]);
                }
            }
            out.import_tile(
                TileCoord { x: tx, y: ty },
                &bytes,
                Growth { budget, others: 0 },
            )?;
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 全ての目標の画素を計算する素朴な版（タイルの飛ばし・一様の埋めを使わない）。
    fn naive(
        source: &Surface,
        w: u32,
        h: u32,
        tile: u32,
        how: SmartResampling,
        normal: bool,
    ) -> Surface {
        let mut out = Surface::new(w, h, tile);
        let xs = axis(source.width(), w, how);
        let ys = axis(source.height(), h, how);
        let mut bytes = vec![0; out.tile_bytes()];
        for ty in 0..out.tile_rows() {
            for tx in 0..out.tile_columns() {
                bytes.fill(0);
                for y in 0..tile.min(h - ty * tile) {
                    for x in 0..tile.min(w - tx * tile) {
                        let p = pixel(
                            source,
                            &xs[(tx * tile + x) as usize],
                            &ys[(ty * tile + y) as usize],
                            normal,
                        )
                        .unwrap();
                        let i = ((y * tile + x) * 4) as usize;
                        bytes[i..i + 4].copy_from_slice(&[p.r, p.g, p.b, p.a]);
                    }
                }
                out.import_tile(TileCoord { x: tx, y: ty }, &bytes, Growth::UNLIMITED)
                    .unwrap();
            }
        }
        out
    }
    /// 一様・ばらばら・無いタイルが混ざり、端のタイルは余白が 0 の元の面。
    fn sparse(w: u32, h: u32, tile: u32, seed: u32) -> Surface {
        let mut s = Surface::new(w, h, tile);
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 16) as u8
        };
        for ty in 0..s.tile_rows() {
            for tx in 0..s.tile_columns() {
                let kind = next() % 4;
                if kind == 0 {
                    continue;
                }
                let mut bytes = vec![0u8; s.tile_bytes()];
                let uniform = [90, 100, 110, 255];
                for y in 0..tile {
                    for x in 0..tile {
                        let inside = tx * tile + x < w && ty * tile + y < h;
                        let i = ((y * tile + x) * 4) as usize;
                        if !inside {
                            continue;
                        }
                        let p = if kind == 1 || kind == 2 {
                            uniform
                        } else {
                            [next(), next(), next(), next()]
                        };
                        bytes[i..i + 4].copy_from_slice(&p);
                    }
                }
                s.import_tile(TileCoord { x: tx, y: ty }, &bytes, Growth::UNLIMITED)
                    .unwrap();
            }
        }
        s
    }
    #[test]
    fn skipping_and_uniform_fill_never_change_the_bytes() {
        for (sw, sh, st) in [(9, 7, 2), (16, 16, 4), (13, 5, 8)] {
            for seed in 0..4 {
                let source = sparse(sw, sh, st, seed);
                for (w, h, tile) in [(9, 7, 2), (5, 3, 4), (26, 20, 8), (40, 33, 16), (3, 11, 2)] {
                    for how in [
                        SmartResampling::Nearest,
                        SmartResampling::Bilinear,
                        SmartResampling::Area,
                    ] {
                        for normal in [false, true] {
                            let fast = surface(&source, w, h, tile, how, normal, u64::MAX).unwrap();
                            let slow = naive(&source, w, h, tile, how, normal);
                            let label =
                                format!("{sw}x{sh}/{st} -> {w}x{h}/{tile} {how:?} seed {seed}");
                            assert_eq!(fast.to_canvas_bytes(), slow.to_canvas_bytes(), "{label}");
                            assert_eq!(fast.allocated_bytes(), slow.allocated_bytes(), "{label}");
                            assert_eq!(fast.tile_coords(), slow.tile_coords(), "{label}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn target_tiles_that_read_only_missing_source_tiles_stay_empty() {
        // 元は左下のタイルだけが塗られている。右上へ向かう目標のタイルは何も読まないので、持たない
        let mut source = Surface::new(8, 8, 4);
        let painted: Vec<u8> = (0..16).flat_map(|_| [7, 8, 9, 255]).collect();
        source
            .import_tile(TileCoord { x: 0, y: 0 }, &painted, Growth::UNLIMITED)
            .unwrap();
        let out = surface(
            &source,
            16,
            16,
            4,
            SmartResampling::Nearest,
            false,
            u64::MAX,
        )
        .unwrap();
        assert!(out.tile_coords().iter().all(|c| c.x < 2 && c.y < 2));
        assert!(out.tile_count() > 0);
    }
}
