//! 塗りつぶしの画像の入力: 棚（.ylp の resources）の画像を、文書の効果の入力（`EffectInputs` の画像）へ渡す。
//!
//! 文書が持つのは画像の ID だけで、画素は文書の外（棚）にある。`set_fill_image` は入力に無い画像を断るので、画像を差す前と、
//! 画像を読む層がある文書（開いた .ylp・Undo で戻した層）には、層が読む画像だけを棚から読み出して入力へ足す。棚の画像を全部は読まない
//! （大きな画像が何枚もある棚で、開くだけで全部を展開しない）。展開した画像は中身の鍵（`content`）ごとに覚える。マップ・モデルの
//! ルートなど画像以外の入力は、今ある入力をそのまま残す（それを渡す側が決める）。

use std::collections::HashMap;

use yolu_core::{EffectInputs, ImageId, ImageInput};
use yolu_io::Resource;

use crate::state::AppState;

/// 棚の画像の ID（ハイフン付きの GUID）から核の画像の ID（128 bit）。
pub fn image_id(resource_id: &str) -> Option<ImageId> {
    let hex: String = resource_id.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
        .filter(|v| *v != 0)
        .map(ImageId)
}

/// 画像の ID に対応する棚の画像の ID（小文字のハイフン付き GUID。棚に無ければ `shelf_resource` で見つからない）。
pub fn resource_id(id: ImageId) -> String {
    let h = format!("{:032x}", id.0);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// 覚える画素の量の予算（バイト。展開した画像の画素。文書の入力が同じ画素を持っている間は、その分も数える）。
const CACHE_BUDGET: u64 = 256 * 1024 * 1024;
/// 覚える画像の数の上限（壊れた画像の理由のように画素の無い覚えも含む）。
const CACHE_ENTRIES: usize = 512;

struct Entry {
    result: Result<ImageInput, String>,
    /// 画素のバイト数（壊れた画像は 0）。
    bytes: u64,
    /// 最後に引いた順（大きいほど新しい）。
    used: u64,
}

/// 展開した画像の覚え（中身と色空間の鍵 → 展開の結果）。壊れた画像の理由も覚えて、毎フレーム読み直さない。覚える画素は予算で絞り、
/// 使われていない古いものから捨てる（今入れたものは、予算を超える大きさでも 1 つは残す）。
pub struct ImageCache {
    decoded: HashMap<String, Entry>,
    tick: u64,
    bytes: u64,
    budget: u64,
    decodes: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        ImageCache {
            decoded: HashMap::new(),
            tick: 0,
            bytes: 0,
            budget: CACHE_BUDGET,
            decodes: 0,
        }
    }
}

impl ImageCache {
    /// 覚える画素の予算を替える（試験用。実際は既定のまま）。
    pub fn with_budget(budget: u64) -> ImageCache {
        ImageCache {
            budget,
            ..ImageCache::default()
        }
    }

    /// 棚の画像 1 枚を効果の入力にする。覚えていればそれを返し、`png`（PNG の中身。棚から借りる）は呼ばない。覚えが無いときだけ
    /// `png` を呼んで展開する（壊れていた理由も覚える）。中身が借りられないときは覚えずに断る。
    pub fn get<'a>(
        &mut self,
        resource: &Resource,
        png: impl FnOnce() -> Option<&'a [u8]>,
    ) -> Result<ImageInput, String> {
        // 色空間は索引だけで画素の鍵が変わらないので、鍵に色空間を含める（替えたら読み直す）
        let key = format!("{}:{}", resource.content, space_of(resource).0);
        self.tick += 1;
        if let Some(entry) = self.decoded.get_mut(&key) {
            entry.used = self.tick;
            return entry.result.clone();
        }
        let png = png().ok_or_else(|| "content".to_owned())?;
        self.decodes += 1;
        let result = decode(resource, png);
        let bytes = result.as_ref().map_or(0, |i| i.pixels.len() as u64);
        self.bytes += bytes;
        self.decoded.insert(
            key.clone(),
            Entry {
                result: result.clone(),
                bytes,
                used: self.tick,
            },
        );
        self.trim(&key);
        result
    }

    /// 予算（と数）に収まるまで、`keep` 以外の使われていない古い覚えを捨てる。
    fn trim(&mut self, keep: &str) {
        while (self.bytes > self.budget || self.decoded.len() > CACHE_ENTRIES)
            && self.decoded.len() > 1
        {
            let Some(oldest) = self
                .decoded
                .iter()
                .filter(|(k, _)| k.as_str() != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(entry) = self.decoded.remove(&oldest) {
                self.bytes -= entry.bytes;
            }
        }
    }

    /// 覚えている画像の数と画素のバイト数。
    pub fn held(&self) -> (usize, u64) {
        (self.decoded.len(), self.bytes)
    }

    /// これまでに PNG を展開した回数（覚えが効いているかを試験が見る）。
    pub fn decode_count(&self) -> u64 {
        self.decodes
    }
}

/// 棚の画像の色空間（索引の文字と core の型）。
pub fn space_of(resource: &Resource) -> (&'static str, yolu_core::ImageColorSpace) {
    match resource.metadata.get("colorSpace").and_then(|v| v.as_str()) {
        Some("srgb") => ("srgb", yolu_core::ImageColorSpace::Srgb),
        Some("linear") => ("linear", yolu_core::ImageColorSpace::Linear),
        _ => ("unspecified", yolu_core::ImageColorSpace::Unspecified),
    }
}

/// PNG（上の行が先）を、文書と同じ向き（下の行が先）の straight RGBA8 にして効果の入力にする。
pub fn decode(resource: &Resource, png: &[u8]) -> Result<ImageInput, String> {
    let (w, h) = (
        resource.metadata["width"].as_u64().unwrap_or(0) as u32,
        resource.metadata["height"].as_u64().unwrap_or(0) as u32,
    );
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    if (img.width(), img.height()) != (w, h) {
        return Err("size".into());
    }
    let mut pixels = Vec::with_capacity(img.as_raw().len());
    for row in img.as_raw().chunks_exact(w as usize * 4).rev() {
        pixels.extend_from_slice(row);
    }
    ImageInput::new(w, h, pixels, space_of(resource).1).map_err(|e| e.to_string())
}

impl AppState {
    /// 棚の画像のうち、文書の層が読むもの（と `extra`）を、全部のテクスチャセットの文書の入力へ足す。入力の画像がすでに同じ中身なら
    /// 何もしない。足せない（棚に無い・壊れている）画像は足さず、文書は値を見せる（ID は残る）。
    pub fn fillfx_sync_images(&mut self, extra: Option<ImageId>) {
        let sets = self.sets.len();
        for index in 0..sets {
            let wanted = {
                let doc = self.set_doc(index);
                let mut ids: Vec<ImageId> = doc
                    .layers()
                    .iter()
                    .flat_map(|l| l.fill_images().map(|(_, id)| id))
                    .collect();
                if index == self.sets.current_index() {
                    ids.extend(extra);
                }
                ids.sort_by_key(|i| i.0);
                ids.dedup();
                ids
            };
            if wanted.is_empty() {
                continue;
            }
            let mut inputs: Option<EffectInputs> = None;
            for id in wanted {
                let rid = resource_id(id);
                // 毎フレームの確かめは借用だけで済ませる。入力が同じ中身なら何もしない。足りないときも、覚えを先に引き、
                // PNG の中身は覚えが無いときにだけ棚から借りる（壊れた画像も覚えるので、毎フレームの作業は覚えの検索だけ）
                let Some(resource) = self.shelf.get(&rid) else {
                    continue;
                };
                let has = self
                    .set_doc(index)
                    .effect_inputs()
                    .image(id)
                    .is_some_and(|i| {
                        i.hash == resource.content && i.color_space == space_of(resource).1
                    });
                if has {
                    continue;
                }
                let shelf = self.shelf.shelf();
                if let Ok(image) = self
                    .fillfx
                    .images
                    .get(resource, || shelf.content_bytes(&rid))
                {
                    let base = inputs
                        .take()
                        .unwrap_or_else(|| self.set_doc(index).effect_inputs().clone());
                    inputs = Some(base.with_image(id, image));
                }
            }
            if let Some(inputs) = inputs {
                let _ = self.set_doc_mut(index).set_effect_inputs(inputs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(content: &str, width: u32, height: u32, space: &str) -> Resource {
        Resource {
            id: format!("id-{content}"),
            kind: "image".into(),
            name: content.into(),
            content: content.into(),
            entry: format!("resources/{content}.png"),
            metadata: serde_json::json!({"width": width, "height": height, "colorSpace": space}),
        }
    }

    /// 2 × 2 の PNG（画素 16 バイト）。
    fn png(seed: u8) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_raw(2, 2, [[seed, 1, 2, 255]; 4].concat())
            .unwrap()
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_broken_image_is_remembered_and_its_bytes_are_asked_for_only_once() {
        let mut cache = ImageCache::default();
        let broken = resource("broken", 4, 4, "srgb");
        let asked = std::cell::Cell::new(0);
        let bytes = png(1); // 2 × 2 の PNG だが、索引は 4 × 4（寸法が索引と違う）
        for _ in 0..50 {
            let got = cache.get(&broken, || {
                asked.set(asked.get() + 1);
                Some(&bytes)
            });
            assert_eq!(got.err().as_deref(), Some("size"));
        }
        assert_eq!(asked.get(), 1, "壊れた画像は、2 度目からは中身を借りない");
        assert_eq!(cache.decode_count(), 1);
        assert_eq!(cache.held(), (1, 0));
        // 色空間を替えれば別の鍵（読み直す）
        let linear = resource("broken", 4, 4, "linear");
        let _ = cache.get(&linear, || Some(&bytes));
        assert_eq!(cache.decode_count(), 2);
        // 中身を借りられないときは、覚えずに断る
        let none = cache.get(&resource("none", 2, 2, "srgb"), || None);
        assert!(none.is_err());
        assert_eq!(cache.decode_count(), 2);
        assert_eq!(cache.held().0, 2);
    }

    #[test]
    fn the_cache_drops_the_least_recently_used_images_to_stay_inside_its_budget() {
        let (a, b, c) = (
            resource("a", 2, 2, "srgb"),
            resource("b", 2, 2, "srgb"),
            resource("c", 2, 2, "srgb"),
        );
        let (pa, pb, pc) = (png(10), png(20), png(30));
        // 画素 16 バイトの画像を 2 枚まで（32 ≤ 40 < 48）
        let mut cache = ImageCache::with_budget(40);
        assert!(cache.get(&a, || Some(&pa)).is_ok());
        assert!(cache.get(&b, || Some(&pb)).is_ok());
        assert_eq!(cache.held(), (2, 32));
        // a を引き直して新しくし、c を入れる: いちばん使われていない b が落ちる
        assert!(cache.get(&a, || Some(&pa)).is_ok());
        assert!(cache.get(&c, || Some(&pc)).is_ok());
        assert_eq!(cache.held(), (2, 32));
        assert_eq!(cache.decode_count(), 3);
        assert!(cache.get(&a, || Some(&pa)).is_ok());
        assert!(cache.get(&c, || Some(&pc)).is_ok());
        assert_eq!(cache.decode_count(), 3, "a と c は覚えている");
        assert!(cache.get(&b, || Some(&pb)).is_ok());
        assert_eq!(cache.decode_count(), 4, "b は落としたので読み直す");
        // 予算より大きい 1 枚でも、今入れたものは残す（ほかは落とす）
        let mut tiny = ImageCache::with_budget(1);
        assert!(tiny.get(&a, || Some(&pa)).is_ok());
        assert_eq!(tiny.held(), (1, 16));
        assert!(tiny.get(&b, || Some(&pb)).is_ok());
        assert_eq!(tiny.held(), (1, 16));
        assert!(tiny.get(&b, || Some(&pb)).is_ok());
        assert_eq!(tiny.decode_count(), 2, "残した 1 枚は覚えている");
    }

    #[test]
    fn resource_ids_round_trip() {
        let id = ImageId(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        let s = resource_id(id);
        assert_eq!(s, "01234567-89ab-cdef-0011-223344556677");
        assert_eq!(image_id(&s), Some(id));
        assert_eq!(
            image_id("00000000-0000-0000-0000-000000000000"),
            None,
            "0 は使わない"
        );
        assert_eq!(image_id("not-a-guid"), None);
        assert_eq!(
            image_id(&s.replace('-', "")),
            Some(id),
            "ハイフンが無くても読める"
        );
    }
}
