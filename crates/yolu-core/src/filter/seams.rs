//! 近傍の段が UV の継ぎ目をまたいで読む（`Options::seams`）。
//!
//! 近傍の段（`halo` > 0）の前に、段の入力の写しの、島の外の帯のテクセルを、帯の写し（`geometry::SeamBand`）が指す相手の島の画素で埋める
//! （双線形。前乗算して混ぜ、接空間の法線は継ぎ目の向きで XY を回す）。ふつうの 2D の近傍の段をかけた後、島の外のテクセルは段の入力のまま
//! 戻す（書き出しのパディングとは別）。点の段はそのまま。
//!
//! 相手の島の画素がブロックの入力の外にあるときは、その点を含む小さな矩形を、その段の前までの段で評価して読む（前の段にも近傍の段があれば、
//! そこでも同じく継ぎ目をまたぐ）。どのブロックの分け方でも、同じ画素は同じ値になる。

use super::*;
use crate::geometry::seam_band::BandEntry;
use crate::geometry::SeamBand;
use crate::math::UNIT;

/// 相手の島の画素を読む小さな矩形の、まとめる升の一辺（前の段の半径の和がこれより大きければ、その和）。
const CELL: u32 = 32;

/// 帯のテクセル 1 つが、帯を埋めている間（`fill_band`・`remote_values`）に同時に持つ作業バイトの上限。同時に生きているのは、
/// - 帯の項目 `items`（位置と読む所）
/// - ブロックの外の読む点（1 テクセルあたり最大 [`POINTS`] 点）ごとの、点の鍵 `keys`（集めるときに容量が最大 2 倍まで伸びるので 2 倍で数える）・
///   升で並べた `order`・読んだ値 `out`
///
/// で、読み終えた後の `fills`・`saved`（`items`・`keys`・値と同時に生きる）を足した姿は、これより小さい（`fill_stays_under_the_peak` が確かめる）。
const TEXEL_BYTES: u64 = (std::mem::size_of::<(usize, BandEntry)>()
    + POINTS
        * (2 * std::mem::size_of::<u64>()
            + std::mem::size_of::<(u64, u32)>()
            + std::mem::size_of::<[u8; 4]>())) as u64;
/// 帯のテクセル 1 つが読む点の最大の数（双線形の 4 点）。
const POINTS: usize = 4;
/// 読み終えた後に生きている、帯のテクセル 1 つあたりのバイト数（`items`・`keys`・`fills`・`saved`・`out`）。`TEXEL_BYTES` を超えない。
#[cfg(test)]
const AFTER_BYTES: u64 = (std::mem::size_of::<(usize, BandEntry)>()
    + POINTS * (2 * std::mem::size_of::<u64>() + std::mem::size_of::<[u8; 4]>())
    + std::mem::size_of::<[u8; 4]>()
    + std::mem::size_of::<(usize, [u8; 4])>()) as u64;

/// 継ぎ目をまたぐときに、1 ブロックの評価が足す作業バイトの見積り（`block_working_bytes` に足す）: 近傍の段ごとに、
/// - 帯を埋めるテクセルの確保（1 テクセルあたり `TEXEL_BYTES`）× テクセルの数。テクセルの数は、ブロックの入力の画素数（相手の島を読む小さな
///   矩形の画素数が大きければそれ）と帯の写しの全テクセルの数 `band_texels` の小さい方。帯の写しが分からない（文書が評価の前に見積もる）ときは
///   `u64::MAX` を渡す（入力の画素ぜんぶが帯のテクセルになる最悪）。
/// - 相手の島を読む小さな矩形の評価（升の一辺 + 前の段の半径の 2 倍の正方形、画素あたり 32 バイト）。
///
/// 前の段にも近傍の段があれば入れ子になるので、段の分を足す。
pub fn seam_working_bytes(
    stack: &[Stage],
    block: u32,
    width: u32,
    height: u32,
    band_texels: u64,
) -> u64 {
    let active: Vec<&Stage> = stack.iter().filter(|s| s.active()).collect();
    let total: u32 = active.iter().map(|s| s.settings.halo()).sum();
    let input = u64::from(width.min(block.saturating_add(2 * total)))
        * u64::from(height.min(block.saturating_add(2 * total)));
    // 相手の島を読む小さな矩形（段ごと。画像の中）
    let mut nested = Vec::new();
    let mut prefix = 0u32;
    for s in &active {
        let h = s.settings.halo();
        if h > 0 {
            let side = CELL.max(prefix).saturating_add(2 * prefix);
            nested.push(u64::from(side.min(width)) * u64::from(side.min(height)));
        }
        prefix = prefix.saturating_add(h);
    }
    let rect = nested.iter().copied().fold(input, u64::max);
    let texels = rect.min(band_texels);
    nested.iter().fold(0u64, |extra, &px| {
        extra
            .saturating_add(texels.saturating_mul(TEXEL_BYTES))
            .saturating_add(px.saturating_mul(32))
    })
}

fn contains(r: Rect, x: u32, y: u32) -> bool {
    x >= r.x && y >= r.y && x - r.x < r.width && y - r.y < r.height
}

fn key(x: u32, y: u32) -> u64 {
    (u64::from(y) << 32) | u64::from(x)
}

impl Engine<'_> {
    /// 近傍の段 k（入力 buf は cur の画素、出力は next の画素）。継ぎ目の帯の写しがあれば、帯を埋めてからかけ、島の外を戻す。
    pub(super) fn neighborhood_stage(
        &self,
        k: usize,
        s: &Stage,
        mut buf: Vec<u8>,
        cur: Rect,
        next: Rect,
    ) -> Result<Vec<u8>, Error> {
        let check = || self.options.check();
        let Some(band) = self.options.seams else {
            return pixels::neighborhood(
                &buf,
                cur,
                next,
                self.width,
                self.height,
                s,
                self.value_type,
                &check,
            );
        };
        let saved = self.fill_band(band, k, &mut buf, cur)?;
        let mut out = pixels::neighborhood(
            &buf,
            cur,
            next,
            self.width,
            self.height,
            s,
            self.value_type,
            &check,
        )?;
        restore_outside(band, &mut out, next, &buf, cur, &saved);
        Ok(out)
    }

    /// buf（cur の画素。段 k の入力）の帯のテクセルを埋める。埋めたテクセルの位置（buf のバイトの位置）と元の値を返す。
    fn fill_band(
        &self,
        band: &SeamBand,
        k: usize,
        buf: &mut [u8],
        cur: Rect,
    ) -> Result<Vec<(usize, [u8; 4])>, Error> {
        let cw = cur.width as usize;
        let mut items: Vec<(usize, BandEntry)> = Vec::new();
        for y in cur.y..cur.y + cur.height {
            for e in band.row_entries(y, cur.x, cur.x + cur.width) {
                let i = ((y - cur.y) as usize * cw + (u32::from(e.x) - cur.x) as usize) * 4;
                items.push((i, *e));
            }
        }
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.options.check()?;
        let ty = self.value_type;
        let local = |x: u32, y: u32| -> [u8; 4] {
            let o = ((y - cur.y) as usize * cw + (x - cur.x) as usize) * 4;
            buf[o..o + 4].try_into().unwrap()
        };
        let fills: Vec<[u8; 4]> = if k == 0 {
            // 最初の段の入力は読み元そのもの: ブロックの外の点は読み元から直に読む（マスクは不透明の灰色にして）
            let at = |x: u32, y: u32| -> [u8; 4] {
                if contains(cur, x, y) {
                    local(x, y)
                } else {
                    let p = self.source.pixel(x, y);
                    if ty == ValueType::Mask {
                        [p[3], p[3], p[3], 255]
                    } else {
                        p
                    }
                }
            };
            items
                .iter()
                .map(|(_, e)| blend(e, at, ty, band.frame(e.frame)))
                .collect()
        } else {
            let mut keys: Vec<u64> = items
                .iter()
                .flat_map(|(_, e)| e.points())
                .filter(|&(px, py, _)| !contains(cur, px, py))
                .map(|(px, py, _)| key(px, py))
                .collect();
            keys.sort_unstable();
            keys.dedup();
            let values = self.remote_values(k, &keys)?;
            let at = |x: u32, y: u32| -> [u8; 4] {
                if contains(cur, x, y) {
                    local(x, y)
                } else {
                    let i = keys
                        .binary_search(&key(x, y))
                        .expect("ブロックの外の点は先に読んである");
                    values[i]
                }
            };
            items
                .iter()
                .map(|(_, e)| blend(e, at, ty, band.frame(e.frame)))
                .collect()
        };
        let mut saved = Vec::with_capacity(items.len());
        for ((i, _), v) in items.iter().zip(fills) {
            let i = *i;
            saved.push((i, buf[i..i + 4].try_into().unwrap()));
            buf[i..i + 4].copy_from_slice(&v);
        }
        Ok(saved)
    }

    /// 段 k の入力の、点 keys（並べて重ねたもの）の値。近い点を升ごとの小さな矩形にまとめて評価する。
    fn remote_values(&self, k: usize, keys: &[u64]) -> Result<Vec<[u8; 4]>, Error> {
        let mut out = vec![[0u8; 4]; keys.len()];
        if keys.is_empty() {
            return Ok(out);
        }
        let prefix: u32 = self.chain[..k].iter().map(|s| s.settings.halo()).sum();
        let cell = CELL.max(prefix);
        let split = |k: u64| (k as u32, (k >> 32) as u32);
        let mut order: Vec<(u64, u32)> = keys
            .iter()
            .enumerate()
            .map(|(i, &k)| {
                let (x, y) = split(k);
                (key(x / cell, y / cell), i as u32)
            })
            .collect();
        order.sort_unstable();
        for group in order.chunk_by(|a, b| a.0 == b.0) {
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
            for &(_, i) in group {
                let (x, y) = split(keys[i as usize]);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            let r = Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
            let data = self.rect(k, r)?;
            for &(_, i) in group {
                let (x, y) = split(keys[i as usize]);
                let o = ((y - y0) as usize * r.width as usize + (x - x0) as usize) * 4;
                out[i as usize] = data[o..o + 4].try_into().unwrap();
            }
        }
        Ok(out)
    }
}

/// 帯のテクセル 1 つの値: 使う点を重みで混ぜる（色・スカラー・マスクは前乗算で。どの点も透明なら、重みのいちばん大きい点の RGB で透明）。
/// 接空間の法線は、アルファを掛けた単位ベクトルを混ぜ、継ぎ目の向き（frame）で XY を回して正規化する。
fn blend(
    e: &BandEntry,
    at: impl Fn(u32, u32) -> [u8; 4],
    ty: ValueType,
    frame: [f64; 4],
) -> [u8; 4] {
    let normal = ty == ValueType::TangentNormal;
    let mut total = 0u64;
    let mut alpha = 0u64;
    let mut color = [0u64; 3];
    let mut vector = [0f64; 3];
    let mut heaviest = (0u32, [0u8; 4]);
    for (x, y, w) in e.points() {
        let p = at(x, y);
        let wa = u64::from(w) * u64::from(p[3]);
        total += u64::from(w);
        alpha += wa;
        if normal {
            for c in 0..3 {
                vector[c] += wa as f64 * (UNIT[p[c] as usize] * 2.0 - 1.0);
            }
        } else {
            for c in 0..3 {
                color[c] += wa * u64::from(p[c]);
            }
        }
        if w > heaviest.0 {
            heaviest = (w, p);
        }
    }
    if total == 0 {
        return heaviest.1;
    }
    let a = ((alpha + total / 2) / total) as u8;
    if alpha == 0 {
        let mut p = heaviest.1;
        p[3] = 0;
        return p;
    }
    if normal {
        let [r0, r1, r2, r3] = frame;
        let (x, y) = (vector[0], vector[1]);
        return pixels::encode([r0 * x + r1 * y, r2 * x + r3 * y, vector[2]], a);
    }
    let c = |v: u64| ((v + alpha / 2) / alpha).min(255) as u8;
    [c(color[0]), c(color[1]), c(color[2]), a]
}

/// 近傍の段の出力 out（next の画素）のうち、島の外のテクセルを段の入力に戻す。入力は、埋める前の値（saved の位置は埋めた後の buf では
/// 上書きされているので、saved の値）。
fn restore_outside(
    band: &SeamBand,
    out: &mut [u8],
    next: Rect,
    buf: &[u8],
    cur: Rect,
    saved: &[(usize, [u8; 4])],
) {
    let islands = band.islands();
    let (nw, cw) = (next.width as usize, cur.width as usize);
    let end = next.x + next.width;
    for y in next.y..next.y + next.height {
        let o_row = (y - next.y) as usize * nw;
        let b_row = (y - cur.y) as usize * cw;
        let mut copy = |from: u32, to: u32| {
            let o = (o_row + (from - next.x) as usize) * 4;
            let b = (b_row + (from - cur.x) as usize) * 4;
            let n = (to - from) as usize * 4;
            out[o..o + n].copy_from_slice(&buf[b..b + n]);
        };
        let runs = islands.row(y);
        let first = runs.partition_point(|r| r.end <= next.x);
        let mut x = next.x;
        for r in &runs[first..] {
            if r.start >= end {
                break;
            }
            let start = r.start.max(next.x);
            if start > x {
                copy(x, start);
            }
            x = x.max(r.end.min(end));
        }
        if x < end {
            copy(x, end);
        }
    }
    for &(i, v) in saved {
        let p = i / 4;
        let (x, y) = (cur.x + (p % cw) as u32, cur.y + (p / cw) as u32);
        if contains(next, x, y) {
            let o = ((y - next.y) as usize * nw + (x - next.x) as usize) * 4;
            out[o..o + 4].copy_from_slice(&v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blur(radius: u32) -> Stage {
        Stage::new(Settings::GaussianBlur { radius })
    }

    #[test]
    fn fill_stays_under_the_peak() {
        // 読み終えた後の姿（fills・saved を足したもの）は、`remote_values` の間の姿を超えない
        const { assert!(AFTER_BYTES <= TEXEL_BYTES) };
        // 見積りの単位は型の大きさから: 項目 24 + 4 点 × (鍵 16 + 並び 16 + 値 4)
        assert_eq!(TEXEL_BYTES, 24 + 4 * (16 + 16 + 4));
    }

    #[test]
    fn the_estimate_follows_the_band_texels_and_the_stack() {
        let (block, w, h) = (256, 4096, 4096);
        let one = [blur(8)];
        // 帯のテクセルが分からなければ、入力の画素ぜんぶが帯のテクセルになる最悪
        let worst = seam_working_bytes(&one, block, w, h, u64::MAX);
        let input = u64::from(block + 16) * u64::from(block + 16);
        assert!(worst >= input * TEXEL_BYTES, "{worst}");
        // 実際の数が小さければ、その数で見積もる（入力より多くは数えない）
        let few = seam_working_bytes(&one, block, w, h, 1000);
        assert!(few < worst);
        assert_eq!(
            seam_working_bytes(&one, block, w, h, input * 10),
            worst,
            "入力の画素より多い数は頭打ち"
        );
        // 点の段・無効な段・強さ 0 の段は足さない
        assert_eq!(
            seam_working_bytes(&[Stage::new(Settings::Invert)], block, w, h, u64::MAX),
            0
        );
        // 近傍の段が増えれば、段の分が増える
        let two = [blur(8), blur(8)];
        assert!(seam_working_bytes(&two, block, w, h, u64::MAX) > worst);
        // 画像より大きな矩形は数えない（小さな画像）
        let tiny = seam_working_bytes(&one, block, 32, 32, u64::MAX);
        assert!(tiny <= 32 * 32 * (TEXEL_BYTES + 32), "{tiny}");
    }
}
