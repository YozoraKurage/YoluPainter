//! 画面の側に置いた物（テクスチャ・描いた形）の持ち主の印。egui を知らない。
//!
//! 状態の側の型は、画面へ上げた物の代わりにこの印を持つ（画素の元と、作り直すかを決める鍵は状態の側のまま）。画面の側
//! （`ui_store`）は印ごとに 1 つの物を持ち、印が捨てられた物をフレームの終わりに捨てる。印を捨てる・替える所が、前に
//! テクスチャを捨てていた所になる。

use std::fmt;
use std::sync::{Arc, Weak};

/// 画面の側に置いた物の持ち主の印（写せない。捨てると、画面の側の物もフレームの終わりに捨てられる）。
#[derive(Default)]
pub struct UiSlot(Arc<()>);

impl UiSlot {
    pub fn new() -> UiSlot {
        UiSlot::default()
    }

    /// 画面の側の表の鍵（印が生きている間と、画面の側が弱い参照を持っている間は、ほかの印と重ならない）。
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    /// 印が捨てられたかを見る弱い参照（画面の側が持つ）。
    pub(crate) fn watch(&self) -> Weak<()> {
        Arc::downgrade(&self.0)
    }
}

impl fmt::Debug for UiSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UiSlot")
    }
}
