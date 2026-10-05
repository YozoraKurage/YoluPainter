//! 層・マスク・グループの出力を、領域でまとめて読む（PSD の書き出しが、PSD に形の無い効果を画素にして書くときに読む）。
//!
//! 合成と同じ評価の道を通る（別の式を書かない）:
//! - 層の出力は、評価が要る層（有効なフィルター・グラデーション・投影）なら合成と同じブロックの評価（[`Document::layer_output_pixel`] と
//!   同じキャッシュ・予算・取消）、要らなければ保存した画素（塗りつぶしは値）。マスク・不透明度・合成の前の値で、層の有効の印は見ない。
//! - マスクの出力は、フィルターを通した隠す量（0〜255）。有効・反転・濃度の前の値（[`Document::mask_output_hide`] と同じ）。
//! - グループの出力は、グループの子の計画を透明から重ねた合成（グループ自身の不透明度・モード・マスクの前）。クリッピングされたグループを
//!   1 枚の画素にするときに読む。合成と同じ `composite_entries_into` と、同じ評価（[`Document::evaluate_entries`]）を通る。
//! - どれも文書を変えない（評価のキャッシュを埋めるだけ）。矩形は画布の中、行の並びは `order`（合成と同じ。`BottomUp` が文書の並び）。
//!   取消の旗が立てば `Cancelled` で戻り、出力は不定（呼び手が捨てる）。

use std::sync::atomic::AtomicBool;

use super::eval::{cancelled, SourceKey};
use super::*;
use crate::composite::{composite_entries_into, Stack};

/// 面から何を読むか。
#[derive(Clone, Copy)]
enum Take {
    /// straight RGBA8（1 画素 4 バイト）。
    Rgba,
    /// アルファだけ（マスクの隠す量。1 画素 1 バイト）。
    Alpha,
}

impl Take {
    fn bytes(self) -> usize {
        match self {
            Take::Rgba => 4,
            Take::Alpha => 1,
        }
    }
}

/// 面の矩形を `out`（矩形の幅 × 高さ × `take` の画素のバイト数）へ写す。無いタイルは 0（透明・隠さない）。
fn blit(
    surface: Option<&Surface>,
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
    take: Take,
) -> Result<(), CoreError> {
    out.fill(0);
    let Some(surface) = surface else {
        return Ok(());
    };
    if rect.is_empty() || surface.tile_count() == 0 {
        return Ok(());
    }
    let ts = tile_size;
    let bpp = take.bytes();
    let stride = rect.width as usize * bpp;
    let mut tile = vec![0u8; surface.tile_bytes()];
    for ty in rect.y / ts..=(rect.y + rect.height - 1) / ts {
        for tx in rect.x / ts..=(rect.x + rect.width - 1) / ts {
            if !surface.copy_tile(TileCoord::new(tx, ty), &mut tile)? {
                continue;
            }
            let (x0, x1) = (
                (tx * ts).max(rect.x),
                ((tx + 1) * ts).min(rect.x + rect.width),
            );
            let (y0, y1) = (
                (ty * ts).max(rect.y),
                ((ty + 1) * ts).min(rect.y + rect.height),
            );
            for y in y0..y1 {
                let row = (y - rect.y) as usize;
                let row = match order {
                    RowOrder::BottomUp => row,
                    RowOrder::TopDown => rect.height as usize - 1 - row,
                };
                let src = ((y % ts) * ts + x0 % ts) as usize * 4;
                let dst = row * stride + (x0 - rect.x) as usize * bpp;
                let count = (x1 - x0) as usize;
                match take {
                    Take::Rgba => {
                        out[dst..dst + count * 4].copy_from_slice(&tile[src..src + count * 4])
                    }
                    Take::Alpha => {
                        for i in 0..count {
                            out[dst + i] = tile[src + i * 4 + 3];
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

impl Document {
    fn check_output_rect(
        &self,
        rect: Rect,
        out_len: usize,
        bytes_per_pixel: usize,
    ) -> Result<(), CoreError> {
        self.check_rect(rect)?;
        if out_len != rect.width as usize * rect.height as usize * bytes_per_pixel {
            return Err(CoreError::InvalidArgument("出力の大きさが矩形と違う"));
        }
        Ok(())
    }

    /// 層の出力（straight RGBA8、行は下から上）。ラスターと塗りつぶしだけ。`layer_output_pixel` を矩形でまとめて読むのと同じ値。
    pub fn layer_output(
        &self,
        id: LayerId,
        channel: Channel,
        rect: Rect,
    ) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.layer_output_into(id, channel, rect, &mut out, RowOrder::BottomUp, None)?;
        Ok(out)
    }

    /// `layer_output` を呼び手の領域（width × height × 4）へ。行の並びと取消を選べる。
    pub fn layer_output_into(
        &self,
        id: LayerId,
        channel: Channel,
        rect: Rect,
        out: &mut [u8],
        order: RowOrder,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        self.require_channel(channel)?;
        self.check_output_rect(rect, out.len(), 4)?;
        let l = &self.layers[index];
        if !matches!(l.kind, LayerKind::Raster | LayerKind::Fill) {
            return Err(CoreError::Unsupported(
                "ラスターと塗りつぶし以外には、層の出力の画素が無い",
            ));
        }
        cancelled(cancel)?;
        if rect.is_empty() {
            return Ok(());
        }
        if l.has_evaluated_output(channel) {
            let range = self.range_of_rect(rect);
            let surface = self.output_surface(index, SourceKey::Channel(channel), range, cancel)?;
            return blit(Some(&surface), self.tile_size, rect, out, order, Take::Rgba);
        }
        match l.kind {
            LayerKind::Fill => {
                let value = l.fill_value(channel).unwrap_or(Rgba8::TRANSPARENT);
                for pixel in out.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&value.to_array());
                }
                Ok(())
            }
            _ => blit(
                l.surface(channel),
                self.tile_size,
                rect,
                out,
                order,
                Take::Rgba,
            ),
        }
    }

    /// マスクの出力（1 画素 1 バイトの隠す量 0〜255、行は下から上）。フィルターを通した値で、有効・反転・濃度の前。
    /// `mask_output_hide` を矩形でまとめて読むのと同じ値。
    pub fn mask_output(&self, id: LayerId, rect: Rect) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize];
        self.mask_output_into(id, rect, &mut out, RowOrder::BottomUp, None)?;
        Ok(out)
    }

    /// `mask_output` を呼び手の領域（width × height）へ。行の並びと取消を選べる。
    pub fn mask_output_into(
        &self,
        id: LayerId,
        rect: Rect,
        out: &mut [u8],
        order: RowOrder,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        self.check_output_rect(rect, out.len(), 1)?;
        let Some(mask) = &self.layers[index].mask else {
            return Err(CoreError::Unsupported("層にマスクが無い"));
        };
        cancelled(cancel)?;
        if rect.is_empty() {
            return Ok(());
        }
        if mask.has_active_filters() {
            let range = self.range_of_rect(rect);
            let surface = self.output_surface(index, SourceKey::Mask, range, cancel)?;
            return blit(
                Some(&surface),
                self.tile_size,
                rect,
                out,
                order,
                Take::Alpha,
            );
        }
        blit(
            Some(&mask.surface),
            self.tile_size,
            rect,
            out,
            order,
            Take::Alpha,
        )
    }

    /// グループの出力（straight RGBA8、行は下から上）: 子を透明から重ねた合成で、グループ自身の不透明度・モード・マスクの前。
    pub fn group_output(
        &self,
        id: LayerId,
        channel: Channel,
        rect: Rect,
    ) -> Result<Vec<u8>, CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.group_output_into(id, channel, rect, &mut out, RowOrder::BottomUp, None)?;
        Ok(out)
    }

    /// `group_output` を呼び手の領域（width × height × 4）へ。行の並びと取消を選べる。
    pub fn group_output_into(
        &self,
        id: LayerId,
        channel: Channel,
        rect: Rect,
        out: &mut [u8],
        order: RowOrder,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        let kind = self.channel_kind(channel)?;
        self.check_output_rect(rect, out.len(), 4)?;
        if self.layers[index].kind != LayerKind::Group {
            return Err(CoreError::Unsupported(
                "グループ以外には、グループの出力が無い",
            ));
        }
        cancelled(cancel)?;
        if rect.is_empty() {
            return Ok(());
        }
        let entries = Stack::new(&self.layers, channel, kind, None).plan_level(Some(id));
        let eval = self.evaluate_entries(&entries, channel, rect, cancel)?;
        let stack = Stack::new(&self.layers, channel, kind, Some(&eval));
        composite_entries_into(&stack, &entries, self.tile_size, rect, out, order);
        Ok(())
    }
}

impl Document {
    /// 調整の層の設定だけを替えた、履歴の無い読むだけの写し（`capture_snapshot` と同じ写しで、タイルは共有する）。書き出しが「刻みへ丸めたら
    /// 合成がどれだけ変わるか」を、丸めた設定と元の合成を比べて測るために使う。元は変えない。調整でない層・文書に無い層は断る。
    pub fn with_adjustments_replaced(
        &self,
        changes: &[(LayerId, AdjustmentSettings)],
    ) -> Result<Document, CoreError> {
        let mut copy = self.capture_snapshot()?;
        for (id, settings) in changes {
            settings.validate()?;
            let index = copy.index_of(*id)?;
            let layer = &mut copy.layers[index];
            if layer.adjustment.is_none() {
                return Err(CoreError::Unsupported("調整の層ではない"));
            }
            layer.adjustment = Some(settings.clone());
        }
        Ok(copy)
    }
}

impl Document {
    /// 層（グループも）が、そのチャンネルの合成の計画に出るか: 表示・不透明度・有効の印・中身があり、グループは子の計画が空でない。出ない層は合成が落とす
    /// ので、クリッピングの組にも入らない（下地のグループの通過を妨げない）。PSD の書き出しが、層を別の形（グループの画素化）で書くときに、
    /// 合成が落とす層を隠した層にして、重なりの意味を変えないために使う。
    pub fn contributes(&self, id: LayerId, channel: Channel) -> Result<bool, CoreError> {
        let index = self.index_of(id)?;
        let kind = self.channel_kind(channel)?;
        Ok(Stack::new(&self.layers, channel, kind, None)
            .make_entry(index)
            .is_some())
    }
}
