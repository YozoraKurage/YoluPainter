//! 画像のミップマップをテクスチャセット（文書）のあいだで共有する: 同じ中身の画像は 1 つのミップマップを使い、どの文書も持たなくなれば
//! 作り直す。予算はそれぞれの文書で数える。
use yolu_core::{Channel, Document, EffectInputs, Rect, Rgba8};

const W: u32 = 64;
const H: u32 = 32;
const GRAY: Rgba8 = Rgba8::new(90, 90, 90, 255);

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, W, H))
        .unwrap()
}

#[test]
fn two_texture_sets_reading_the_same_image_share_one_mip_chain() {
    use std::sync::Arc;
    use yolu_core::{ChannelKind, ImageColorSpace, ImageId, ImageInput};
    // ほかの試験と中身が重ならない画像（並んで走る試験の文書と共有しない）
    let pixels: Vec<u8> = (0..37 * 23 * 4)
        .map(|i| ((i * 7919 + 0x5eed) % 251) as u8)
        .collect();
    let set = |id: u128| {
        let mut doc = Document::with_tile_size(W, H, 16).unwrap();
        let fill = doc
            .add_fill_layer("塗り", &[(Channel::Color, GRAY)], None)
            .unwrap();
        let image = ImageId(id);
        doc.set_effect_inputs(EffectInputs::new().with_image(
            image,
            ImageInput::new(37, 23, pixels.clone(), ImageColorSpace::Unspecified).unwrap(),
        ))
        .unwrap();
        doc.set_fill_image(fill, Channel::Color, Some(image))
            .unwrap();
        (doc, image)
    };
    // ID は違っても中身が同じ画像（セットごとに読み込んだ写し）
    let (a, ia) = set(0xa11ce);
    let (b, ib) = set(0xb0b);
    let ca = whole(&a);
    let cb = whole(&b);
    assert_eq!(ca, cb);
    let (built_a, built_b) = (a.effect_counters(), b.effect_counters());
    assert_eq!(built_a.mip_chains_built, 1);
    assert_eq!(built_b.mip_chains_built, 0, "2 つ目のセットは作らない");
    assert_eq!(built_b.mip_chains_shared, 1);
    let chain_a = a.cached_mip_chain(ia, ChannelKind::Color).unwrap();
    let chain_b = b.cached_mip_chain(ib, ChannelKind::Color).unwrap();
    assert!(Arc::ptr_eq(&chain_a, &chain_b), "同じミップマップ");
    // 覚えの予算はそれぞれの文書で数える（持ち主が 1 つ減っても、もう 1 つの文書の評価は変わらない）
    assert_eq!(
        built_b.image_cache_bytes, built_a.image_cache_bytes,
        "それぞれの予算に入る"
    );
    drop((chain_a, chain_b));
    drop(a);
    b.release_effect_cache();
    assert_eq!(whole(&b), cb);
    assert_eq!(
        b.effect_counters().mip_chains_built,
        1,
        "持ち主が居なくなれば作り直す"
    );
}
