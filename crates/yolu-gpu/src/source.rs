//! 合成の入力の面（保存した画素と、評価した出力）のタイルを取り出す。
//! 評価した出力（有効なフィルター・Generator・塗りつぶしのグラデーションと投影・マスクのフィルター）は、文書の評価のキャッシュ
//! （`Document::layer_output_into`・`mask_output_into`。CPU の合成と同じ評価の道）から読む。効果の計算そのものは CPU で、
//! ここは出力をタイルとして GPU へ渡すだけ。
use super::{error, plan::Slot, GpuError};
use yolu_core::{Channel, Document, Rect, RowOrder, TileCoord};

/// まとめて評価する矩形の画素数の上限（これより広い束は 1 タイルずつ評価する。取り置きの大きさの上限）。
const HOLD_PIXELS: u64 = 4096 * 512;

/// 評価した出力の取り置き（矩形の画素。行は下から上の straight RGBA8。マスクは隠す量をアルファに置く）。
struct Held {
    rect: Rect,
    bytes: Vec<u8>,
}

pub(crate) struct Fetcher<'a> {
    doc: &'a Document,
    channel: Channel,
    ts: usize,
    /// 面ごとの取り置き（評価の出力を持つ面だけ）。
    held: Vec<Option<Held>>,
}

impl<'a> Fetcher<'a> {
    pub(crate) fn new(doc: &'a Document, channel: Channel, slots: usize) -> Self {
        Fetcher {
            doc,
            channel,
            ts: doc.tile_size() as usize,
            held: (0..slots).map(|_| None).collect(),
        }
    }

    /// 評価の出力を持つ面を、束のタイルを含む矩形ごと 1 回で評価して取り置く（タイルごとに評価を頼むと、ブロックを 1 つずつ
    /// 直列に評価する。まとめて頼むと、文書の評価がブロックを並べて評価する）。保存した面では何もしない。
    pub(crate) fn prefetch(
        &mut self,
        k: usize,
        slot: &Slot,
        coords: &[TileCoord],
    ) -> Result<(), GpuError> {
        if !slot.evaluated || coords.is_empty() {
            return Ok(());
        }
        let mut rect: Option<Rect> = None;
        for &c in coords {
            let t = self
                .doc
                .tile_rect(c)
                .ok_or_else(|| error("タイルがキャンバスの外"))?;
            rect = Some(match rect {
                None => t,
                Some(r) => {
                    let (x0, y0) = (r.x.min(t.x), r.y.min(t.y));
                    let (x1, y1) = (
                        (r.x + r.width).max(t.x + t.width),
                        (r.y + r.height).max(t.y + t.height),
                    );
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                }
            });
        }
        let rect = rect.expect("空でない");
        if u64::from(rect.width) * u64::from(rect.height) > HOLD_PIXELS {
            return Ok(()); // 広すぎる束は、タイルごとに評価する
        }
        self.held[k] = Some(Held {
            bytes: self.evaluate(slot, rect)?,
            rect,
        });
        Ok(())
    }

    /// 面 `k` の取り置きを手放す（束の次の面へ進むとき。評価の結果は文書の評価のキャッシュにも残るので、次の束で同じ範囲を頼んでも
    /// 評価し直さない。取り置きを全部の面で持ち続けると、効果のあるレイヤーの数に比例してメモリを使う）。
    pub(crate) fn release(&mut self, k: usize) {
        self.held[k] = None;
    }

    /// 面 `k` のタイルを `tile`（1 タイル分。行は `タイルの一辺 × 4` バイト。キャンバスの端の部分タイルは残りを 0）へ写す。
    /// そのタイルが無い（保存した面にタイルが無い・評価の出力が全部 0）なら false で、`tile` は 0 のまま。
    pub(crate) fn tile(
        &mut self,
        k: usize,
        slot: &Slot,
        coord: TileCoord,
        tile: &mut [u8],
    ) -> Result<bool, GpuError> {
        let layer = &self.doc.layers()[slot.layer];
        if !slot.evaluated {
            let surface = if slot.mask {
                layer.mask().map(|m| m.surface())
            } else {
                layer.surface(self.channel)
            };
            return match surface {
                // 無いタイルは 0 で埋めて false（`copy_tile` が埋める）
                Some(s) => Ok(s.copy_tile(coord, tile)?),
                None => {
                    tile.fill(0);
                    Ok(false)
                }
            };
        }
        tile.fill(0);
        let want = self
            .doc
            .tile_rect(coord)
            .ok_or_else(|| error("タイルがキャンバスの外"))?;
        let covered = self.held[k].as_ref().is_some_and(|h| {
            want.x >= h.rect.x
                && want.y >= h.rect.y
                && want.x + want.width <= h.rect.x + h.rect.width
                && want.y + want.height <= h.rect.y + h.rect.height
        });
        if !covered {
            self.held[k] = Some(Held {
                bytes: self.evaluate(slot, want)?,
                rect: want,
            });
        }
        let held = self.held[k].as_ref().expect("取り置いた");
        let mut any = false;
        for r in 0..want.height as usize {
            let from = ((want.y - held.rect.y) as usize + r) * held.rect.width as usize * 4
                + (want.x - held.rect.x) as usize * 4;
            let row = &held.bytes[from..from + want.width as usize * 4];
            any |= row.iter().any(|&b| b != 0);
            tile[r * self.ts * 4..r * self.ts * 4 + row.len()].copy_from_slice(row);
        }
        if !any {
            tile.fill(0);
        }
        Ok(any)
    }

    /// 評価の出力の矩形（straight RGBA8、行は下から上）。マスクは隠す量をアルファに置く。
    fn evaluate(&self, slot: &Slot, rect: Rect) -> Result<Vec<u8>, GpuError> {
        let layer = &self.doc.layers()[slot.layer];
        let pixels = rect.width as usize * rect.height as usize;
        if slot.mask {
            let mut hide = vec![0u8; pixels];
            self.doc
                .mask_output_into(layer.id(), rect, &mut hide, RowOrder::BottomUp, None)?;
            let mut rgba = vec![0u8; pixels * 4];
            for (px, h) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(hide) {
                px[3] = h;
            }
            Ok(rgba)
        } else {
            let mut rgba = vec![0u8; pixels * 4];
            self.doc.layer_output_into(
                layer.id(),
                self.channel,
                rect,
                &mut rgba,
                RowOrder::BottomUp,
                None,
            )?;
            Ok(rgba)
        }
    }
}
