//! 層自身と祖先のロック。番号は C# と保存形式の版 12 に合わせる。
use super::operations::Dirty;
use super::{Document, Property};
use crate::{CoreError, LayerId, Rgba8};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayerLocks(u8);
impl LayerLocks {
    pub const NONE: Self = Self(0);
    pub const TRANSPARENCY: Self = Self(1);
    pub const PIXELS: Self = Self(2);
    pub const POSITION: Self = Self(4);
    pub const ALL: Self = Self(8);
    pub fn from_bits(bits: u8) -> Result<Self, CoreError> {
        if bits & !15 != 0 {
            return Err(CoreError::InvalidArgument("未知のロック"));
        }
        Ok(Self(bits))
    }
    pub const fn bits(self) -> u8 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}
impl std::ops::BitOr for LayerLocks {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl Document {
    pub fn effective_locks(&self, id: LayerId) -> Result<LayerLocks, CoreError> {
        let mut at = Some(id);
        let mut bits = 0;
        while let Some(id) = at {
            let l = self.layer(id).ok_or(CoreError::LayerNotFound)?;
            bits |= l.locks.0;
            at = l.parent;
        }
        if bits & 8 != 0 {
            bits |= 7;
        }
        Ok(LayerLocks(bits))
    }
    pub fn set_layer_locks(&mut self, id: LayerId, locks: LayerLocks) -> Result<(), CoreError> {
        self.set_property(id, Property::Locks(locks), None)
    }
    /// 読み込みの最後に使う。ロックは検証済みの型で受け、履歴を消す。
    pub fn set_locks_for_load(&mut self, id: LayerId, locks: LayerLocks) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let i = self.index_of(id)?;
        self.layers[i].locks = locks;
        self.external_mutation();
        Ok(())
    }
    pub fn change_layer_locks(
        &mut self,
        ids: &[LayerId],
        flags: LayerLocks,
        on: bool,
    ) -> Result<(), CoreError> {
        let count = ids.iter().collect::<std::collections::HashSet<_>>().len() as u64;
        // ロックは合成を変えないので、変化の記録には何も印を付けない（単独の set_layer_locks と同じ）
        self.batch_layers_cost(Some(64 + 16 * count), |d| {
            for &id in ids {
                let old = d.layer(id).ok_or(CoreError::LayerNotFound)?.locks;
                d.set_layer_locks(
                    id,
                    LayerLocks(if on {
                        old.0 | flags.0
                    } else {
                        old.0 & !flags.0
                    }),
                )?;
            }
            Ok(((), Dirty::NOTHING))
        })
    }
    pub fn ensure_pixels_editable(&self, id: LayerId, erase: bool) -> Result<(), CoreError> {
        self.refuse_lock(id, LayerLocks::ALL)?;
        self.refuse_lock(id, LayerLocks::PIXELS)?;
        if erase {
            self.refuse_lock(id, LayerLocks::TRANSPARENCY)?;
        }
        Ok(())
    }
    /// 画素を書く入口が、書く前に必ず通る関門。ロックで断る（画像・すべて、消すなら透明部分も）か、書くなら透明部分のロックで
    /// アルファを守るか（`true`）を返す。検査と守り方を別々に扱うと、新しい書き込み口が黙ってロックを迂回するので、
    /// 返した値は `StrokeState::new` の `keep_alpha` や塗りつぶしの式へそのまま渡す（どちらも必須の引数で、渡し忘れはコンパイルで落ちる。
    /// ただし false を書く入口は通るので、足し忘れを見つけるのは入口の表 `tests/docops.rs` の `write_entries` の網羅だけ）。マスクへの
    /// 書き込みは透明部分のロックの対象外で、すべてのロックだけで断る（`refuse_lock`）。
    pub(crate) fn pixel_write_guard(&self, id: LayerId, erase: bool) -> Result<bool, CoreError> {
        self.ensure_pixels_editable(id, erase)?;
        Ok(self.effective_locks(id)?.contains(LayerLocks::TRANSPARENCY))
    }
    pub(super) fn refuse_lock(&self, id: LayerId, flag: LayerLocks) -> Result<(), CoreError> {
        self.refuse_lock_from(id, id, flag)
    }
    /// 画素の中身を評価で決め直す編集（画像・グラデーション・パスの付け外し）の関門。決め直すと画素もアルファも変わるので、画像・
    /// すべてのロックに加えて、透明部分のロックでも断る（C# の RefuseLockedPixels → RefuseLockedTransparency の順。名指しもその順）。
    pub(super) fn ensure_pixels_rewritable(&self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_pixels_editable(id, false)?;
        self.refuse_lock(id, LayerLocks::TRANSPARENCY)
    }
    /// [`Document::ensure_pixels_rewritable`] の、まだ文書に無い層（これから親 `parent` の中へ入る。自分のロックは無い）の分。
    /// 断るときの名指しは `layer`（入る層の ID）。親のグループのロックだけが効く。
    pub(super) fn ensure_new_layer_rewritable(
        &self,
        layer: LayerId,
        parent: Option<LayerId>,
    ) -> Result<(), CoreError> {
        let Some(parent) = parent else {
            return Ok(());
        };
        self.refuse_lock_from(layer, parent, LayerLocks::ALL)?;
        self.refuse_lock_from(layer, parent, LayerLocks::PIXELS)?;
        self.refuse_lock_from(layer, parent, LayerLocks::TRANSPARENCY)
    }
    /// `start`（層自身かその祖先）から見て `flag` が掛かっていれば断る。名指しは `named`、持ち主は `start` から祖先へ探した最初の層。
    fn refuse_lock_from(
        &self,
        named: LayerId,
        start: LayerId,
        flag: LayerLocks,
    ) -> Result<(), CoreError> {
        if !self.effective_locks(start)?.contains(flag) {
            return Ok(());
        }
        let mut holder = start;
        loop {
            let l = self.layer(holder).ok_or(CoreError::LayerNotFound)?;
            if l.locks.0 & (flag.0 | 8) != 0 {
                return Err(CoreError::LayerLocked {
                    layer: named,
                    holder,
                    lock: if l.locks.contains(LayerLocks::ALL) {
                        LayerLocks::ALL
                    } else {
                        flag
                    },
                });
            }
            holder = l.parent.ok_or(CoreError::LayerNotFound)?;
        }
    }
}
pub(crate) fn paint_keeping_alpha(start: Rgba8, paint: Rgba8, amount: f64) -> Rgba8 {
    if start.a == 0 {
        return start;
    }
    let a = (amount * paint.a as f64 / 255.0).min(1.0);
    if a <= 0.0 {
        return start;
    }
    let mix = |s: u8, p: u8| {
        crate::math::to_byte(
            crate::math::UNIT[s as usize]
                + (crate::math::UNIT[p as usize] - crate::math::UNIT[s as usize]) * a,
        )
    };
    Rgba8::new(
        mix(start.r, paint.r),
        mix(start.g, paint.g),
        mix(start.b, paint.b),
        start.a,
    )
}
