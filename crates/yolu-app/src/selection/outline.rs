//! 選択範囲の縁（点線が流れる表示の元）。選ばれた量が半分以上（128 以上）の画素と、そうでない画素の境を、画素の角の座標
//! （左下が原点、y は上向き）の軸に沿った線分にする。キャンバスの外は選ばれていないと数えるので、キャンバスの縁に接する所にも縁が出る。
//! 量のあるタイルだけを見る（無いタイルは全部 0）。タイルごとに作った線分は、つながるものを 1 本にまとめる（点線の流れが
//! タイルの境で切れない）。

use crate::engine::SelectionMask;

/// 縁の線分 1 本（画素の角の座標）。横線なら y が `fixed`、x が `from..to`。縦線なら x が `fixed`、y が `from..to`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub horizontal: bool,
    pub fixed: u32,
    pub from: u32,
    pub to: u32,
}

impl Run {
    /// 端の座標（x0, y0, x1, y1）。
    pub fn ends(&self) -> (f64, f64, f64, f64) {
        let (f, a, b) = (self.fixed as f64, self.from as f64, self.to as f64);
        if self.horizontal {
            (a, f, b, f)
        } else {
            (f, a, f, b)
        }
    }
}

/// 縁として数える量（半分以上。GIMP の見える縁と同じ）。
const EDGE_AMOUNT: u8 = 128;

/// つなげたあとの縁の線分の上限（これを超えるほど入り組んだ選択範囲は、超えた分を落とす。描くときの負荷を抑える）。
pub const MAX_RUNS: usize = 400_000;

/// タイルごとに作る、つなげる前の線分の上限。つなげる（全件の並べ替え）前に作る数と確保を抑えるので、画素数に比例して
/// 増えない（`Run` は 16 バイトなので 12.8 MB まで）。つながるのはタイルの境をまたぐ分だけなので、上限の倍あれば足りる。
const MAX_RAW_RUNS: usize = MAX_RUNS * 2;

/// 選択範囲の縁。線分が上限で打ち切られたかも返す（作る段階で上限に達したら、残りのタイルは見ない）。
pub fn outline(mask: &SelectionMask) -> (Vec<Run>, bool) {
    let (runs, capped) = raw_runs(mask, MAX_RAW_RUNS);
    let mut merged = merge(runs);
    let truncated = capped || merged.len() > MAX_RUNS;
    merged.truncate(MAX_RUNS);
    (merged, truncated)
}

/// タイルごとの線分（つなげる前）。作った数が `limit` に届いたら、そこでやめて `true` を返す
/// （行ごとに数えるので、上限を超えるのは 1 行ぶん、最大でタイルの幅の半分まで）。
fn raw_runs(mask: &SelectionMask, limit: usize) -> (Vec<Run>, bool) {
    let (w, h, ts) = (mask.width(), mask.height(), mask.tile_size());
    let n = ts as usize;
    let mut runs: Vec<Run> = Vec::new();
    let mut capped = false;
    let mut amounts = vec![0u8; n * n];
    let mut coords = mask.tile_coords();
    coords.sort_by_key(|c| (c.y, c.x));
    // キャンバスの画素が選ばれているか（キャンバスの外は選ばれていない）
    let on_at = |x: i64, y: i64| -> bool {
        x >= 0
            && y >= 0
            && x < w as i64
            && y < h as i64
            && mask.amount(x as u32, y as u32) >= EDGE_AMOUNT
    };
    'tiles: for c in coords {
        if !mask.copy_tile(c, &mut amounts).unwrap_or(false) {
            continue;
        }
        let (ox, oy) = (c.x * ts, c.y * ts);
        let tw = (w - ox).min(ts) as i64;
        let th = (h - oy).min(ts) as i64;
        // タイルの中の画素。外はほかのタイルの画素を見る
        let on = |lx: i64, ly: i64| -> bool {
            if lx >= 0 && ly >= 0 && lx < tw && ly < th {
                amounts[ly as usize * n + lx as usize] >= EDGE_AMOUNT
            } else {
                on_at(ox as i64 + lx, oy as i64 + ly)
            }
        };
        // 横の縁（下向きに開いた画素の下辺・上向きに開いた画素の上辺）
        for (dy, offset) in [(-1i64, 0u32), (1, 1)] {
            for ly in 0..th {
                if runs.len() >= limit {
                    capped = true;
                    break 'tiles;
                }
                let mut start: Option<i64> = None;
                for lx in 0..=tw {
                    let edge = lx < tw && on(lx, ly) && !on(lx, ly + dy);
                    match (edge, start) {
                        (true, None) => start = Some(lx),
                        (false, Some(s)) => {
                            runs.push(Run {
                                horizontal: true,
                                fixed: oy + ly as u32 + offset,
                                from: ox + s as u32,
                                to: ox + lx as u32,
                            });
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        // 縦の縁（左へ開いた画素の左辺・右へ開いた画素の右辺）
        for (dx, offset) in [(-1i64, 0u32), (1, 1)] {
            for lx in 0..tw {
                if runs.len() >= limit {
                    capped = true;
                    break 'tiles;
                }
                let mut start: Option<i64> = None;
                for ly in 0..=th {
                    let edge = ly < th && on(lx, ly) && !on(lx + dx, ly);
                    match (edge, start) {
                        (true, None) => start = Some(ly),
                        (false, Some(s)) => {
                            runs.push(Run {
                                horizontal: false,
                                fixed: ox + lx as u32 + offset,
                                from: oy + s as u32,
                                to: oy + ly as u32,
                            });
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    (runs, capped)
}

/// 同じ線の上でつながる（端が接する）線分を 1 本にする。
fn merge(mut runs: Vec<Run>) -> Vec<Run> {
    runs.sort_by_key(|r| (!r.horizontal, r.fixed, r.from, r.to));
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        match out.last_mut() {
            Some(last)
                if last.horizontal == r.horizontal
                    && last.fixed == r.fixed
                    && r.from <= last.to =>
            {
                last.to = last.to.max(r.to);
            }
            _ => out.push(r),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Document;

    fn doc(w: u32, h: u32) -> Document {
        Document::new(w, h).unwrap()
    }

    /// 縁の長さの合計（画素の辺の数）。
    fn length(runs: &[Run]) -> u32 {
        runs.iter().map(|r| r.to - r.from).sum()
    }

    #[test]
    fn a_rectangle_has_four_edges() {
        let d = doc(64, 48);
        let mask = SelectionMask::rectangle(&d, 10, 8, 30, 20);
        let (runs, truncated) = outline(&mask);
        assert!(!truncated);
        assert_eq!(runs.len(), 4, "{runs:?}");
        assert_eq!(length(&runs), 2 * (20 + 12));
        let find =
            |h: bool, fixed: u32| runs.iter().find(|r| r.horizontal == h && r.fixed == fixed);
        assert_eq!(find(true, 8).map(|r| (r.from, r.to)), Some((10, 30)));
        assert_eq!(find(true, 20).map(|r| (r.from, r.to)), Some((10, 30)));
        assert_eq!(find(false, 10).map(|r| (r.from, r.to)), Some((8, 20)));
        assert_eq!(find(false, 30).map(|r| (r.from, r.to)), Some((8, 20)));
    }

    #[test]
    fn edges_join_across_tile_borders_and_follow_the_canvas_edge() {
        // タイルは 128。選択が 3 枚のタイルにまたがっても、横の縁は 1 本につながる
        let d = doc(400, 300);
        let mask = SelectionMask::rectangle(&d, 100, 50, 300, 200);
        let (runs, _) = outline(&mask);
        assert_eq!(runs.len(), 4, "{runs:?}");
        // 全面の選択は、キャンバスの縁（4 辺）だけ
        let all = SelectionMask::all(&d);
        let (runs, _) = outline(&all);
        assert_eq!(runs.len(), 4, "{runs:?}");
        assert_eq!(length(&runs), 2 * (400 + 300));
    }

    #[test]
    fn a_hole_has_its_own_edge_and_half_selected_pixels_count_as_selected() {
        let d = doc(64, 64);
        let outer = SelectionMask::rectangle(&d, 8, 8, 56, 56);
        let hole = SelectionMask::rectangle(&d, 24, 24, 40, 40);
        let ring = outer
            .combine(&hole, crate::engine::SelectionCombine::Subtract)
            .unwrap();
        let (runs, _) = outline(&ring);
        assert_eq!(length(&runs), 2 * (48 + 48) + 2 * (16 + 16));
        // ぼかして半分以上の所までが縁の内側になる（縁は 1 本の輪のまま）
        let soft = outer.feather(6.0, false, 1 << 28).unwrap();
        let (soft_runs, _) = outline(&soft);
        assert!(!soft_runs.is_empty());
        assert!(soft_runs.iter().all(|r| r.from < r.to));
    }

    #[test]
    fn a_selection_with_too_many_edges_is_cut_at_the_limit_and_says_so() {
        // 1 画素おきの点（選ばれた画素がどれも孤立して、縁がつながらない）: 画素ごとに 4 本。768 × 768 で 59 万本
        let d = doc(768, 768);
        let ts = d.tile_size();
        let mut tiles = Vec::new();
        for ty in 0..768u32.div_ceil(ts) {
            for tx in 0..768u32.div_ceil(ts) {
                let mut a = vec![0u8; (ts * ts) as usize];
                for ly in 0..ts {
                    for lx in 0..ts {
                        if (tx * ts + lx).is_multiple_of(2) && (ty * ts + ly).is_multiple_of(2) {
                            a[(ly * ts + lx) as usize] = 255;
                        }
                    }
                }
                tiles.push((crate::engine::TileCoord::new(tx, ty), a));
            }
        }
        let mask = SelectionMask::from_amount_tiles(768, 768, ts, tiles).unwrap();
        let (runs, truncated) = outline(&mask);
        assert!(truncated);
        assert_eq!(runs.len(), MAX_RUNS);
        // 切り詰めても、残った線分は正しい形（端は画素の角）
        assert!(runs.iter().all(|r| r.to == r.from + 1));
    }

    #[test]
    fn raw_runs_stop_at_the_limit_before_merging_on_a_large_canvas() {
        // 8192 × 8192 のキャンバスに、1 画素おきの孤立した点のタイルを 200 枚。上限が無ければ 200 × 16384 = 328 万本（52 MB）を
        // 作ってからつなげる。作る段階で止まるので、確保は上限（80 万本）に 1 行ぶんを足した所までで頭打ちになる
        let d = doc(8192, 8192);
        let ts = d.tile_size();
        let checker: Vec<u8> = (0..ts * ts)
            .map(|i| {
                let (x, y) = (i % ts, i / ts);
                if x.is_multiple_of(2) && y.is_multiple_of(2) {
                    255
                } else {
                    0
                }
            })
            .collect();
        let tiles: Vec<_> = (0..200u32)
            .map(|i| {
                (
                    crate::engine::TileCoord::new(i % 64, i / 64),
                    checker.clone(),
                )
            })
            .collect();
        let mask = SelectionMask::from_amount_tiles(8192, 8192, ts, tiles).unwrap();
        let (raw, capped) = raw_runs(&mask, MAX_RAW_RUNS);
        assert!(capped);
        assert!(
            (MAX_RAW_RUNS..=MAX_RAW_RUNS + ts as usize).contains(&raw.len()),
            "{}",
            raw.len()
        );
        let (runs, truncated) = outline(&mask);
        assert!(truncated);
        assert_eq!(runs.len(), MAX_RUNS);
        assert!(runs.iter().all(|r| r.to == r.from + 1));
    }

    #[test]
    fn a_selection_just_under_the_limit_is_not_flagged() {
        // 上限に届かなければ、打ち切りの印は付かない（市松 3 タイルぶん = 約 5 万本）
        let d = doc(384, 128);
        let ts = d.tile_size();
        let tiles: Vec<_> = (0..3u32)
            .map(|tx| {
                let a: Vec<u8> = (0..ts * ts)
                    .map(|i| {
                        if (i % ts).is_multiple_of(2) && (i / ts).is_multiple_of(2) {
                            255
                        } else {
                            0
                        }
                    })
                    .collect();
                (crate::engine::TileCoord::new(tx, 0), a)
            })
            .collect();
        let mask = SelectionMask::from_amount_tiles(384, 128, ts, tiles).unwrap();
        let (runs, truncated) = outline(&mask);
        assert!(!truncated);
        assert_eq!(runs.len(), 3 * 4096 * 4);
    }

    #[test]
    fn an_empty_selection_has_no_edge() {
        let d = doc(32, 32);
        let (runs, truncated) = outline(&SelectionMask::none(&d));
        assert!(runs.is_empty() && !truncated);
    }
}
