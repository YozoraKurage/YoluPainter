//! レイヤーマスクの丸ごとのコピー・別のレイヤーへの移動・置き換えと、黒のマスクの追加、マスクへの画像の貼り付け。
//!
//! - マスクを丸ごと写す物は [`MaskCopy`]（面・有効・反転・濃度・マスクのフィルターのスタック。`Anchor` は写さない）。貼るたびに、
//!   マスクのフィルターには文書の中で重ならない新しい ID を付ける。
//! - 貼る先にマスクがあれば置き換える。置き換えられる側のマスクの `Anchor` は、貼る先に残す（指している Generator が切れない）。
//! - 移動は `Anchor` ごと動かす（`AnchorId` が同じまま行き先のマスクにあり、指している Generator が切れない）。行き先にマスクが
//!   あれば置き換え、そのマスクにあった `Anchor` は一緒に無くなる（マスクを外すのと同じ）。
//! - どれも 1 回の Undo。ロックは、書き換わるレイヤー（行き先と、移動の元）の「すべて」で断る（コピー元のロックは見ない）。

use super::{Command, Document, Entry, HistoryKind, Target};
use crate::error::CoreError;
use crate::layer::{LayerId, RasterMask};
use crate::selection::SelectionMask;
use crate::surface::{Surface, Tile};
use crate::types::Rgba8;
use crate::{PixelClipboard, Reason};

/// コピーしたマスクの丸ごと（`Document::copy_mask`）。作ったあとは変わらない。
#[derive(Clone, Debug)]
pub struct MaskCopy {
    mask: RasterMask,
}

impl MaskCopy {
    /// 写したときのキャンバスの大きさ。貼れるのは同じ大きさの文書だけ。
    pub fn size(&self) -> (u32, u32) {
        (self.mask.surface.width(), self.mask.surface.height())
    }
    pub fn enabled(&self) -> bool {
        self.mask.enabled
    }
    pub fn inverted(&self) -> bool {
        self.mask.inverted
    }
    pub fn density(&self) -> f64 {
        self.mask.density
    }
    /// マスクのフィルターの段の数。
    pub fn filter_count(&self) -> usize {
        self.mask.filters.len()
    }
    /// 面が持っている画素のバイト数（一様なタイルは 4）。
    pub fn allocated_bytes(&self) -> u64 {
        self.mask.surface.allocated_bytes()
    }
}

/// `Document::paste_into_mask` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskPasteResult {
    /// 文書が写したときと違う大きさなので、元の位置ではなく中央へ置いた。
    pub centered: bool,
    /// キャンバスの外へはみ出して切り捨てた、値のある画素の数。
    pub clipped_pixels: u64,
    /// マスクの画素が 1 つでも変わったか（変わらなければ履歴の段は積まない）。
    pub changed: bool,
}

/// 画像の 1 画素を「見える量」（0〜255。白 = 見える）にする: Rec.709 の重みの明るさ。灰色（R = G = B）はその値のまま。
fn visible_of(r: u8, g: u8, b: u8) -> u32 {
    (r as u32 * 2126 + g as u32 * 7152 + b as u32 * 722 + 5000) / 10000
}

impl Document {
    /// 黒（全体を隠す）のラスターマスクをレイヤーに足す。どの種類のレイヤーにも 1 つまで。足すのと全体を隠すのを 1 回の Undo で行い、
    /// 面は一様なタイルだけで持つ（画素のメモリを増やさない）。選択範囲には関わらず全体を隠す。
    pub fn add_black_layer_mask(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        // マスクの有無がロックより先（`add_layer_mask` と同じ）
        if self.layers[index].mask.is_some() {
            return Err(CoreError::Unsupported(Reason::LayerAlreadyHasMask));
        }
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        let mut surface = Surface::new(self.width, self.height, self.tile_size);
        let hidden = Tile::Uniform(Rgba8::new(0, 0, 0, 255));
        for coord in self.canvas_tiles() {
            surface.restore(coord, Some(&hidden));
        }
        let cost = 64 + surface.allocated_bytes();
        self.execute_as(
            HistoryKind::AddMask,
            Command::SwapSmartMask {
                id,
                mask: Some(Box::new(RasterMask::new(surface))),
            },
            cost,
        )
    }

    /// レイヤーのマスクを丸ごと写す（文書は変えない。`Anchor` は写さない）。
    pub fn copy_mask(&self, id: LayerId) -> Result<MaskCopy, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        let mut mask = Self::mask_of(&self.layers[index])?.clone();
        mask.anchor = None;
        Ok(MaskCopy { mask })
    }

    /// コピーしたマスクを `targets` の全部のレイヤーへ貼る（マスクがあれば置き換える）。1 回の Undo。1 つでも断る（ロック・
    /// キャンバスの大きさ違い・レイヤーが無い）と何も変えない。同じレイヤーを重ねて指しても 1 回だけ貼る。
    pub fn set_masks(&mut self, targets: &[LayerId], copy: &MaskCopy) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let surface = &copy.mask.surface;
        if (surface.width(), surface.height(), surface.tile_size())
            != (self.width, self.height, self.tile_size)
        {
            return Err(CoreError::Unsupported(Reason::MaskSizeDiffersFromCanvas));
        }
        let mut ids: Vec<LayerId> = Vec::with_capacity(targets.len());
        for &id in targets {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for &id in &ids {
            self.index_of(id)?;
            self.refuse_lock(id, super::LayerLocks::ALL)?;
        }
        let mut steps = Vec::with_capacity(ids.len());
        for &id in &ids {
            let index = self.index_of(id)?;
            let mut mask = copy.mask.clone();
            // 置き換えられる側の Anchor は貼る先に残す
            mask.anchor = self.layers[index]
                .mask
                .as_ref()
                .and_then(|m| m.anchor.clone());
            for filter in &mut mask.filters {
                filter.id = self.new_filter_id();
            }
            let cost = 64 + mask.surface.allocated_bytes();
            steps.push(Entry {
                kind: HistoryKind::PasteMask,
                command: Command::SwapSmartMask {
                    id,
                    mask: Some(Box::new(mask)),
                },
                cost,
            });
        }
        self.execute_group_as(HistoryKind::PasteMask, steps)
    }

    /// マスクを `from` から `to` へ動かす（`to` にマスクがあれば置き換える）。`Anchor` も一緒に動く。1 回の Undo。同じレイヤーへは
    /// 何もしない。`from` にマスクが無い・どちらかがロックされているなら、何も変えずに断る。
    pub fn move_mask(&mut self, from: LayerId, to: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let source = self.index_of(from)?;
        // マスクの有無がロックより先（`remove_layer_mask` と同じ）
        let mask = Self::mask_of(&self.layers[source])?.clone();
        self.index_of(to)?;
        if from == to {
            return Ok(());
        }
        self.refuse_lock(from, super::LayerLocks::ALL)?;
        self.refuse_lock(to, super::LayerLocks::ALL)?;
        let cost = 64 + mask.surface.allocated_bytes();
        // 先に元から外してから行き先へ置く（同じ AnchorId が同時に 2 つのマスクにならない順）
        let steps = vec![
            Entry {
                kind: HistoryKind::RemoveMask,
                command: Command::RemoveMask {
                    id: from,
                    mask: None,
                },
                cost,
            },
            Entry {
                kind: HistoryKind::PasteMask,
                command: Command::SwapSmartMask {
                    id: to,
                    mask: Some(Box::new(mask)),
                },
                cost,
            },
        ];
        self.execute_group_as(HistoryKind::MoveMask, steps)
    }

    /// クリップボードの画像を、レイヤーのマスクに貼る（白 = 見える・黒 = 隠す。色のある画像は明るさで）。置く位置は
    /// [`Document::paste_as_layer`] と同じ（同じ大きさの文書なら元の位置、違えば中央。はみ出す分は切って数える）。写しの透明な画素は
    /// マスクをそのまま残し、半透明の画素はマスクの今の値と混ぜる。選択範囲には関わらない。1 回の Undo。
    pub fn paste_into_mask(
        &mut self,
        id: LayerId,
        clip: &PixelClipboard,
    ) -> Result<MaskPasteResult, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        Self::mask_of(&self.layers[index])?;
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        let (w, h) = (self.width as i64, self.height as i64);
        let (cw, ch) = (clip.width() as i64, clip.height() as i64);
        let (ox, oy, centered) = self.paste_origin(clip);
        let (x0, y0) = (ox.max(0), oy.max(0));
        let (x1, y1) = ((ox + cw).min(w), (oy + ch).min(h));
        let src = clip.pixels();
        let mut clipped = 0u64;
        if x0 != ox || y0 != oy || x1 != ox + cw || y1 != oy + ch {
            for y in 0..ch {
                for x in 0..cw {
                    let (px, py) = (ox + x, oy + y);
                    if px >= x0 && px < x1 && py >= y0 && py < y1 {
                        continue;
                    }
                    let o = ((y * cw + x) * 4) as usize;
                    if src[o + 3] != 0 {
                        clipped += 1;
                    }
                }
            }
        }
        let mut changed = false;
        if x0 < x1 && y0 < y1 {
            let region = SelectionMask::rectangle(self, x0, y0, x1, y1);
            changed =
                self.edit_effective(index, Target::Mask, Some(region), |x, y, start, _amount| {
                    let o = (((y as i64 - oy) * cw + (x as i64 - ox)) * 4) as usize;
                    let alpha = src[o + 3] as u32;
                    if alpha == 0 {
                        return start;
                    }
                    let hide = 255 - visible_of(src[o], src[o + 1], src[o + 2]);
                    let hide = if alpha == 255 {
                        hide
                    } else {
                        (start.a as u32 * (255 - alpha) + hide * alpha + 127) / 255
                    };
                    if hide == 0 {
                        Rgba8::TRANSPARENT
                    } else {
                        Rgba8::new(0, 0, 0, hide as u8)
                    }
                })?;
            if changed {
                if let Some(entry) = self.undo.last_mut() {
                    entry.kind = HistoryKind::PasteMask;
                }
            }
        }
        Ok(MaskPasteResult {
            centered,
            clipped_pixels: clipped,
            changed,
        })
    }

    /// 段を当てて、指定の種類で積む（`execute` は段から種類を決める）。
    fn execute_as(
        &mut self,
        kind: HistoryKind,
        mut command: Command,
        cost: u64,
    ) -> Result<(), CoreError> {
        self.apply(&mut command)?;
        self.revision += 1;
        self.push(Entry {
            command,
            cost,
            kind,
        });
        Ok(())
    }

    /// 複数の段を 1 回の Undo にして、指定の種類で積む（費用は段の合計に 32。途中で断ったら済んだ分を戻して何も積まない）。
    fn execute_group_as(&mut self, kind: HistoryKind, steps: Vec<Entry>) -> Result<(), CoreError> {
        let cost = 32 + steps.iter().map(|e| e.cost).sum::<u64>();
        self.execute_as(kind, Command::Compound(steps), cost)
    }
}
