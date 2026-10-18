//! 画面の側に置く物（テクスチャ・描いた形）の表。`UiState` と同じく画面の側の物で、状態の側の型は持たない。
//!
//! egui の `Context` の一時のデータに置き（Context ごとに 1 つ）、状態の側の印（[`UiSlot`]）ごとに 1 つの物を持つ。印が
//! 捨てられた物は、フレームの終わりの [`sweep`] が捨てる（テクスチャの解放の頼みは、前に状態の側でテクスチャを捨てて
//! いたのと同じフレームのうちに出る）。painter だけを受け取る描く関数（選択範囲とステンシルの重ね）からも届くよう、
//! `UiState` の欄ではなく Context に置く。

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use egui::{Context, Id, ImageData, TextureHandle, TextureId, TextureOptions};

use super::UiSlot;

struct Held {
    /// 持ち主の印（捨てられたら、この物も捨てる）。
    owner: Weak<()>,
    value: Box<dyn Any + Send + Sync>,
}

/// 印 → 物の表（写しは同じ表を指す）。
#[derive(Clone, Default)]
pub struct Store(Arc<Mutex<HashMap<usize, Held>>>);

/// 表を借りている間の口（借りている間に、同じ表をもう一度借りない）。
pub struct Lock<'a>(MutexGuard<'a, HashMap<usize, Held>>);

impl Store {
    /// この Context の表。
    pub fn of(ctx: &Context) -> Store {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Store>(Id::new("yolu.ui-store"))
                .clone()
        })
    }

    pub fn lock(&self) -> Lock<'_> {
        Lock(self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Lock<'_> {
    /// 印の物（置いていないか、型が違えば None）。
    pub fn get<T: Any>(&mut self, slot: &UiSlot) -> Option<&mut T> {
        self.0.get_mut(&slot.key())?.value.downcast_mut()
    }

    /// 印の物（置いていないか、型が違えば `T::default()` を置いてから）。
    pub fn get_or_default<T: Any + Default + Send + Sync>(&mut self, slot: &UiSlot) -> &mut T {
        let held = self.0.entry(slot.key()).or_insert_with(|| Held {
            owner: slot.watch(),
            value: Box::new(T::default()),
        });
        if !held.value.is::<T>() {
            held.value = Box::new(T::default());
        }
        held.value.downcast_mut().expect("型を確かめた")
    }

    /// 印の物を置く（前の物は捨てる）。
    pub fn put<T: Any + Send + Sync>(&mut self, slot: &UiSlot, value: T) {
        self.0.insert(
            slot.key(),
            Held {
                owner: slot.watch(),
                value: Box::new(value),
            },
        );
    }
}

/// 印のテクスチャ（置いてあれば）。
pub fn texture(ctx: &Context, slot: &UiSlot) -> Option<TextureHandle> {
    Store::of(ctx).lock().get::<TextureHandle>(slot).cloned()
}

/// 印のテクスチャの ID（置いてあれば）。
pub fn texture_id(ctx: &Context, slot: &UiSlot) -> Option<TextureId> {
    Store::of(ctx)
        .lock()
        .get::<TextureHandle>(slot)
        .map(|h| h.id())
}

/// 画素からテクスチャを作り、印の物として置く。
pub fn load_texture(
    ctx: &Context,
    slot: &UiSlot,
    name: impl Into<String>,
    image: impl Into<ImageData>,
    options: TextureOptions,
) -> TextureHandle {
    let handle = ctx.load_texture(name, image, options);
    Store::of(ctx).lock().put(slot, handle.clone());
    handle
}

/// 持ち主の印が捨てられた物を捨てる（フレームの終わりに 1 回。殻の `YoluApp::ui` が呼ぶ）。
pub fn sweep(ctx: &Context) {
    Store::of(ctx)
        .lock()
        .0
        .retain(|_, held| held.owner.strong_count() > 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, ColorImage};

    fn allocated(ctx: &Context) -> usize {
        ctx.tex_manager().read().num_allocated()
    }

    #[test]
    fn sweep_frees_only_the_textures_whose_slot_was_dropped() {
        let ctx = Context::default();
        let before = allocated(&ctx);
        let image = || ColorImage::new([2, 2], vec![Color32::WHITE; 4]);
        let kept = UiSlot::new();
        let dropped = UiSlot::new();
        let _ = load_texture(&ctx, &kept, "kept", image(), TextureOptions::LINEAR);
        let _ = load_texture(&ctx, &dropped, "dropped", image(), TextureOptions::LINEAR);
        assert_eq!(allocated(&ctx), before + 2);
        // 印が生きている間は、何度 sweep しても捨てない
        sweep(&ctx);
        sweep(&ctx);
        assert!(texture(&ctx, &kept).is_some() && texture(&ctx, &dropped).is_some());
        assert_eq!(allocated(&ctx), before + 2);
        // 印を捨てても、sweep までは残る
        drop(dropped);
        assert_eq!(Store::of(&ctx).lock().0.len(), 2);
        sweep(&ctx);
        assert_eq!(Store::of(&ctx).lock().0.len(), 1);
        assert_eq!(
            allocated(&ctx),
            before + 1,
            "捨てた印のテクスチャを解放する"
        );
        assert!(texture_id(&ctx, &kept).is_some());
        drop(kept);
        sweep(&ctx);
        assert_eq!(Store::of(&ctx).lock().0.len(), 0);
        assert_eq!(allocated(&ctx), before);
    }

    #[test]
    fn a_slot_holds_one_value_and_a_different_type_replaces_it() {
        let ctx = Context::default();
        let slot = UiSlot::new();
        let store = Store::of(&ctx);
        *store.lock().get_or_default::<u32>(&slot) = 7;
        assert_eq!(store.lock().get::<u32>(&slot).copied(), Some(7));
        assert!(
            store.lock().get::<String>(&slot).is_none(),
            "型が違えば None"
        );
        store.lock().get_or_default::<String>(&slot).push('a');
        assert!(
            store.lock().get::<u32>(&slot).is_none(),
            "型の違う物を置き直す"
        );
        assert_eq!(store.lock().0.len(), 1);
        // 写しの表も、同じ Context の表も、同じ物を指す
        assert_eq!(
            Store::of(&ctx).lock().get::<String>(&slot).cloned(),
            Some("a".to_string())
        );
    }
}
