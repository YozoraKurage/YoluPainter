//! 文書の選択範囲と、選択範囲の内側の塗りつぶし（C# の PaintDocument.Regions）。
//!
//! - 選択範囲は Undo の 1 段で置き換える（画素は変えない）。空の選択範囲は「選択なし」と同じにする（何も選ばれていない所に
//!   描くと、どのブラシも黙って何もしなくなるため。C# と同じ）。
//! - ストロークは始めたときの選択範囲の内側だけを、選ばれた量の割合で変える（[`crate::brush`] の画素の式の 1 か所）。
//! - 塗りつぶし（バケツ）は、渡した範囲と選択範囲の重なり（どちらかが無ければもう片方、どちらも無ければ全体）を、量の割合で
//!   塗る。どれも 1 回の Undo で、タイルの前後の状態を持つ。予算（ストロークの巻き戻しと画素）を超えたら何も変えずに断る。

use rayon::prelude::*;

use super::{Command, Document, Entry, Target, TileChange};
use crate::blend::blend;
use crate::error::CoreError;
use crate::layer::LayerId;
use crate::math::{require_finite, to_byte};
use crate::selection::{SelectionCombine, SelectionMask};
use crate::surface::Tile;
use crate::types::{BlendMode, Channel, LayerKind, Rgba8, TileCoord};

impl Document {
    /// 今の選択範囲（None は選択なし。編集はどこにでも効く）。
    pub fn selection(&self) -> Option<&SelectionMask> {
        self.selection.as_ref()
    }

    /// 選択範囲を置き換える（None か空の選択範囲は選択を外す）。1 回の Undo で、画素は変えない。今と同じ札なら何もしない
    /// （変更が「変わらない」ときに返す同じ札では段を積まない）。大きさ・タイルの大きさが文書と違えば断る。
    pub fn set_selection(&mut self, mask: Option<SelectionMask>) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.check_selection_size(mask.as_ref())?;
        let next = mask.filter(|m| !m.is_empty());
        match (&next, &self.selection) {
            (None, None) => return Ok(()),
            (Some(a), Some(b)) if a.same_as(b) => return Ok(()),
            _ => {}
        }
        // 履歴の重さは C# と同じ（64 + 新しい選択範囲の RGBA のタイルでの大きさ）
        let cost = 64 + next.as_ref().map_or(0, |m| m.history_bytes());
        let old = self.selection.clone();
        self.execute(Command::Selection { old, new: next }, cost)
    }

    /// 選択を外す（Undo の 1 段。選択が無ければ何もしない）。
    pub fn clear_selection(&mut self) -> Result<(), CoreError> {
        self.set_selection(None)
    }

    /// 新しい形を今の選択範囲と組み合わせて選択範囲にする（画面の選択のツール: Shift で足す・Ctrl で引く・両方で重ねる）。
    /// 置き換えか選択が無いときは形そのもの（引くなら選択なし）。1 回の Undo。
    pub fn combine_selection(
        &mut self,
        shape: &SelectionMask,
        mode: SelectionCombine,
    ) -> Result<(), CoreError> {
        let next = match &self.selection {
            Some(current) if mode != SelectionCombine::Replace => {
                Some(current.combine(shape, mode)?)
            }
            _ => (mode != SelectionCombine::Subtract).then(|| shape.clone()),
        };
        self.set_selection(next)
    }

    /// 文書と一緒に読んだ選択範囲を戻す（selection.bin）: Undo の段も版も増やさない（開いた直後の文書は保存済みのまま）。
    /// 履歴ができた後は断る。
    pub fn restore_selection(&mut self, mask: Option<SelectionMask>) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if !self.undo.is_empty() || !self.redo.is_empty() {
            return Err(CoreError::Unsupported(
                "選択範囲を戻せるのは読み込みの直後だけ",
            ));
        }
        self.check_selection_size(mask.as_ref())?;
        self.selection = mask.filter(|m| !m.is_empty());
        Ok(())
    }

    fn check_selection_size(&self, mask: Option<&SelectionMask>) -> Result<(), CoreError> {
        match mask {
            Some(m)
                if m.width() != self.width
                    || m.height() != self.height
                    || m.tile_size() != self.tile_size =>
            {
                Err(CoreError::InvalidArgument(
                    "選択範囲の大きさがキャンバスと違う",
                ))
            }
            _ => Ok(()),
        }
    }

    /// レイヤーのチャンネルを色で塗る（バケツ）: region（無ければ選択範囲、それも無ければ全体）の量 × 不透明度だけ Normal で重ねる。
    /// erase なら色のアルファ × 量だけアルファを減らす。ラスターレイヤーの有効なチャンネルだけ。画素が変わらなければ false
    /// （履歴に積まない）。
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        layer: LayerId,
        channel: Channel,
        color: Rgba8,
        opacity: f64,
        region: Option<&SelectionMask>,
        erase: bool,
    ) -> Result<bool, CoreError> {
        require_finite(opacity, "opacity")?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidArgument("opacity"));
        }
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(layer)?;
        if self.layers[index].kind != LayerKind::Raster {
            return Err(CoreError::Unsupported("塗りつぶしはラスターレイヤーだけ"));
        }
        if !self.layers[index].is_channel_enabled(channel) {
            return Err(CoreError::Unsupported("無効のチャンネルには塗れない"));
        }
        let keep_alpha = self.pixel_write_guard(layer, erase)?;
        // パスレイヤーの断りはロックのあと（C# の PaintableSurface は RefuseLockedPixels → RefusePathLayer の順）
        self.refuse_path_layer(index)?;
        let created = self.ensure_surface(index, channel);
        let r = self.edit_region(
            index,
            Target::Channel(channel),
            region,
            move |start, amount| fill_pixel(start, color, opacity, amount, erase, keep_alpha),
        );
        if !matches!(r, Ok(true)) {
            self.drop_created_surface(index, channel, created);
        }
        r
    }

    /// レイヤーのマスクを塗る（バケツ）: 隠す量を、amount × 範囲の量の割合だけ 255 へ（reveal なら 0 へ）寄せる。どの種類のレイヤーの
    /// マスクにも塗れる。
    pub fn fill_mask(
        &mut self,
        layer: LayerId,
        amount: f64,
        region: Option<&SelectionMask>,
        reveal: bool,
    ) -> Result<bool, CoreError> {
        require_finite(amount, "amount")?;
        if !(0.0..=1.0).contains(&amount) {
            return Err(CoreError::InvalidArgument("amount"));
        }
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        // マスクの有無がロックより先（C# の RequireMask のあとに RefuseLockedAttributes）
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("レイヤーにマスクが無い"));
        }
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        self.edit_region(index, Target::Mask, region, move |start, coverage| {
            mask_fill_pixel(start, amount, coverage, reveal)
        })
    }

    /// 編集の効く範囲: 渡した範囲と選択範囲の重なり、どちらか、どちらも無ければ None（全体）。
    pub(super) fn effective_region(
        &self,
        region: Option<&SelectionMask>,
    ) -> Result<Option<SelectionMask>, CoreError> {
        let Some(r) = region else {
            return Ok(self.selection.clone());
        };
        if r.width() != self.width || r.height() != self.height || r.tile_size() != self.tile_size {
            return Err(CoreError::InvalidArgument("範囲の大きさがキャンバスと違う"));
        }
        Ok(Some(match &self.selection {
            None => r.clone(),
            Some(s) => r.combine(s, SelectionCombine::Intersect)?,
        }))
    }

    /// 範囲の量のある画素へ pixel(描く前, 量 0〜1) を当て、1 回の Undo にする（C# の EditRegion）。新しいタイルはタイルごとに
    /// 並列に計算し、予算の確かめと書き込みはタイルの順にこのスレッドで行う（止まる所も、止まったときに戻すものも逐次と同じ）。
    /// 予算を超えたら書いたタイルを全部戻して断る。
    fn edit_region<F>(
        &mut self,
        index: usize,
        target: Target,
        region: Option<&SelectionMask>,
        pixel: F,
    ) -> Result<bool, CoreError>
    where
        F: Fn(Rgba8, f64) -> Rgba8 + Sync,
    {
        let effective = self.effective_region(region)?;
        self.edit_effective(index, target, effective, |_, _, start, amount| {
            pixel(start, amount)
        })
    }

    /// [`Document::edit_region`] の、範囲を決めたあとの部分: effective（None は全体）の量のある画素へ pixel(x, y, 描く前, 量) を当てる。
    /// 画像で置き換える入口（`replace_pixels`）は画素の位置が要るのでこちらを使う。
    pub(super) fn edit_effective<F>(
        &mut self,
        index: usize,
        target: Target,
        effective: Option<SelectionMask>,
        pixel: F,
    ) -> Result<bool, CoreError>
    where
        F: Fn(u32, u32, Rgba8, f64) -> Rgba8 + Sync,
    {
        let coords: Vec<TileCoord> = match &effective {
            Some(m) => m.tile_coords(),
            None => self.canvas_tiles().collect(),
        };
        let layer = self.layers[index].id;
        let changes =
            self.edit_region_tiles(index, target, effective.as_ref(), &coords, &mut 0, pixel)?;
        if changes.is_empty() {
            return Ok(false);
        }
        for c in &changes {
            self.mark_target_tile(index, target, c.coord);
        }
        let cost = 64
            + changes
                .iter()
                .map(|c| {
                    16 + c.before.as_ref().map_or(0, |t| t.byte_size())
                        + c.after.as_ref().map_or(0, |t| t.byte_size())
                })
                .sum::<u64>();
        self.revision += 1;
        self.push(Entry {
            kind: super::HistoryKind::Pixels,
            command: Command::Stroke {
                layer,
                target,
                changes,
            },
            cost,
        });
        Ok(true)
    }
    pub(super) fn edit_region_tiles<F>(
        &mut self,
        index: usize,
        target: Target,
        effective: Option<&SelectionMask>,
        coords: &[TileCoord],
        rollback: &mut u64,
        pixel: F,
    ) -> Result<Vec<TileChange>, CoreError>
    where
        F: Fn(u32, u32, Rgba8, f64) -> Rgba8 + Sync,
    {
        let growth = self.growth_for(index, target);
        let stroke_budget = self.stroke_budget;
        let (width, height, ts) = (self.width, self.height, self.tile_size);
        let surface = self.target_surface_mut(index, target).expect("編集の面");
        let n = ts as usize * ts as usize;
        let batch = (rayon::current_num_threads() * 4).max(1);
        let mut changes: Vec<TileChange> = Vec::new();
        let mut failure = None;
        'batches: for chunk in coords.chunks(batch) {
            // 選ばれていないタイルは None、ほかは（今と同じか, 書いた後のタイル）
            type Computed = Option<(bool, Option<Tile>)>;
            let computed: Vec<Result<Computed, CoreError>> = {
                let surface = &*surface;
                chunk
                    .par_iter()
                    .map(|&coord| {
                        let mut amounts = vec![255u8; n];
                        if let Some(m) = &effective {
                            if !m.copy_tile(coord, &mut amounts).expect("文書の中のタイル")
                            {
                                return Ok(None); // 選ばれていないタイル
                            }
                        }
                        let mut bytes = vec![0u8; n * 4];
                        surface.copy_tile(coord, &mut bytes)?;
                        let w = (width - coord.x * ts).min(ts) as usize;
                        let h = (height - coord.y * ts).min(ts) as usize;
                        let t = ts as usize;
                        for y in 0..h {
                            for x in 0..w {
                                let i = y * t + x;
                                if amounts[i] == 0 {
                                    continue;
                                }
                                let o = i * 4;
                                let start = Rgba8::from_slice(&bytes[o..o + 4]);
                                let next = pixel(
                                    coord.x * ts + x as u32,
                                    coord.y * ts + y as u32,
                                    start,
                                    amounts[i] as f64 / 255.0,
                                );
                                bytes[o..o + 4].copy_from_slice(&next.to_array());
                            }
                        }
                        let after = Tile::from_vec(bytes);
                        Ok(Some((
                            Tile::same(surface.tile(coord), after.as_ref()),
                            after,
                        )))
                    })
                    .collect()
            };
            for (&coord, result) in chunk.iter().zip(computed) {
                let result = match result {
                    Ok(r) => r,
                    Err(e) => {
                        failure = Some(e);
                        break 'batches;
                    }
                };
                let Some((same, after)) = result else {
                    continue; // 選ばれていないタイル
                };
                if same {
                    continue;
                }
                let before = surface.tile(coord).cloned();
                *rollback += 64 + before.as_ref().map_or(0, |t| t.byte_size());
                if *rollback > stroke_budget {
                    failure = Some(CoreError::StrokeBudgetExceeded);
                    break 'batches;
                }
                let delta = after.as_ref().map_or(0, |t| t.byte_size()) as i64
                    - before.as_ref().map_or(0, |t| t.byte_size()) as i64;
                if let Err(e) = growth.ensure(surface.allocated, delta) {
                    failure = Some(e);
                    break 'batches;
                }
                surface.restore(coord, after.as_ref());
                changes.push(TileChange {
                    coord,
                    before,
                    after,
                });
            }
        }
        if let Some(e) = failure {
            for c in changes.iter().rev() {
                surface.restore(c.coord, c.before.as_ref());
            }
            return Err(e);
        }
        Ok(changes)
    }
}

/// マスクの塗りの画素の式（C# の MaskFillRule）: 隠す量 A を、目標（隠すなら 255・見せるなら 0）へ (A + (目標 − A) × amount × coverage) / 255
/// を丸めた値にする。amount は塗る量、coverage は範囲・三角形・グラデーションの量（どちらも 0〜1）。マスクの塗りつぶし・
/// グラデーション・三角形の塗りが同じ式を通る（1 か所だけ直って食い違わないように）。
pub(super) fn mask_fill_pixel(start: Rgba8, amount: f64, coverage: f64, reveal: bool) -> Rgba8 {
    let target = if reveal { 0.0 } else { 255.0 };
    let hide = to_byte((start.a as f64 + (target - start.a as f64) * amount * coverage) / 255.0);
    if hide == 0 {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::new(0, 0, 0, hide)
    }
}

/// 塗りつぶしの画素の式（C# の FillRule）。amount は範囲の量（0〜1）。`keep_alpha` は書き込みの関門（`pixel_write_guard`）が返した
/// 透明部分のロックで、必須の引数（塗りつぶし・グラデーション・三角形の塗りがこの式を通るので、呼び出し側に値を決めさせる。渡し忘れは
/// コンパイルで落ちるが、false を書く・クロージャで引数を無視する入口は通る。関門を通すのは入口の責任）。守るときはアルファを変えず、
/// 消す指定は関門が断っているのでここへは来ない（debug_assert で確かめる）。
pub(super) fn fill_pixel(
    start: Rgba8,
    color: Rgba8,
    opacity: f64,
    amount: f64,
    erase: bool,
    keep_alpha: bool,
) -> Rgba8 {
    debug_assert!(
        !(erase && keep_alpha),
        "消す書き込みは透明部分のロックの関門が断っている"
    );
    if keep_alpha {
        return super::locks::paint_keeping_alpha(start, color, opacity * amount);
    }
    if !erase {
        return blend(start, color, opacity * amount, BlendMode::Normal);
    }
    let a = opacity * amount;
    let alpha = to_byte(start.a as f64 / 255.0 * (1.0 - a * color.a as f64 / 255.0));
    if alpha == 0 {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::new(start.r, start.g, start.b, alpha)
    }
}
