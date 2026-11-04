//! 評価のキャッシュの予算と捨て方: 予算より大きな範囲を同じ順で何度も読んでも一部は当たり、予算がいくつでも（0 でも）結果は同じバイト。
use yolu_core::{
    Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, Rect, Rgba8,
};

const SIDE: u32 = 512;
const TILE: u32 = 32;
/// ブロックの一辺（64² の画素で、文書は 8×8 = 64 ブロック。1 ブロックの出力は 16 KiB）。
const BLOCK: u32 = 64;

/// 画像を UV で塗るレイヤーが 2 枚の文書（どのタイルも一様でない画素なので、ブロックの出力がそのまま予算の量になる）。
fn document() -> Document {
    let mut doc = Document::with_tile_size(SIDE, SIDE, TILE).unwrap();
    doc.set_filter_block_pixels(BLOCK).unwrap();
    let pixels: Vec<u8> = (0..61 * 47 * 4)
        .map(|i| ((i * 7919 + 0xcafe) % 253) as u8)
        .collect();
    let image = ImageId(0xcac4e);
    doc.set_effect_inputs(EffectInputs::new().with_image(
        image,
        ImageInput::new(61, 47, pixels, ImageColorSpace::Unspecified).unwrap(),
    ))
    .unwrap();
    for name in ["下", "上"] {
        let fill = doc
            .add_fill_layer(name, &[(Channel::Color, Rgba8::new(90, 90, 90, 200))], None)
            .unwrap();
        doc.set_fill_image(fill, Channel::Color, Some(image))
            .unwrap();
    }
    doc
}

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, SIDE, SIDE))
        .unwrap()
}

fn evaluated(doc: &Document) -> u64 {
    doc.effect_counters().blocks_evaluated
}

/// 全面の評価の出力の量（2 枚 × 64 ブロック × 16 KiB）。
const ALL: u64 = 2 * 64 * (BLOCK as u64 * BLOCK as u64 * 4);

#[test]
fn reading_twice_the_budget_in_the_same_order_hits_on_the_second_pass() {
    let doc = {
        let mut d = document();
        d.set_filter_cache_budget_bytes(ALL / 2);
        d
    };
    let first = whole(&doc);
    let pass1 = evaluated(&doc);
    assert_eq!(pass1, 128, "1 回目は全部を評価する");
    assert!(doc.effect_counters().cache_bytes <= ALL / 2);
    let second = whole(&doc);
    let pass2 = evaluated(&doc) - pass1;
    assert_eq!(first, second);
    assert!(
        pass2 < 128,
        "予算の 2 倍を同じ順で読んでも、2 回目に一部は当たる（評価し直したブロック {pass2}/128）"
    );
    let third = whole(&doc);
    let pass3 = evaluated(&doc) - pass1 - pass2;
    assert_eq!(first, third);
    assert!(
        pass3 <= pass2 + 8,
        "続けて読んでも当たりは大きく減らない: {pass2} → {pass3}"
    );
}

#[test]
fn the_result_does_not_depend_on_the_budget() {
    let reference = {
        let mut d = document();
        d.set_filter_cache_budget_bytes(ALL * 2);
        let r = whole(&d);
        assert_eq!(whole(&d), r);
        assert_eq!(evaluated(&d), 128, "収まれば 2 回目は全部当たる");
        r
    };
    for budget in [0, 1, ALL / 64, ALL / 8, ALL / 2 + 12345] {
        let mut d = document();
        d.set_filter_cache_budget_bytes(budget);
        for pass in 0..3 {
            assert_eq!(whole(&d), reference, "予算 {budget} の {pass} 回目");
            assert!(d.effect_counters().cache_bytes <= budget);
        }
        if budget < BLOCK as u64 * BLOCK as u64 * 4 {
            assert_eq!(
                d.effect_counters().cache_bytes,
                0,
                "1 ブロックも入らない予算は持たない"
            );
        }
    }
}

#[test]
fn lowering_the_budget_trims_at_once() {
    let mut doc = document();
    doc.set_filter_cache_budget_bytes(ALL);
    let r = whole(&doc);
    assert_eq!(doc.effect_counters().cache_bytes, ALL);
    doc.set_filter_cache_budget_bytes(ALL / 4);
    assert!(doc.effect_counters().cache_bytes <= ALL / 4);
    assert_eq!(whole(&doc), r);
}

/// 画像で塗った下のレイヤーの Anchor を、上のレイヤーの Anchor の段が読む文書（Anchor のタイルとブロックが両方とも埋まる）。
fn anchored_document() -> Document {
    use yolu_core::generator::{self, anchor::ReadMode, Settings};
    use yolu_core::{AnchorPlacement, EffectSettings, FilterSpec, FilterTarget};
    let mut doc = document();
    let layers: Vec<_> = doc.layers().iter().map(|l| l.id()).collect();
    let (base, top) = (layers[0], layers[1]);
    let anchor = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
    doc.set_generator_anchor(
        top,
        reader,
        Some(anchor),
        Channel::Color,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc
}

#[test]
fn the_result_with_anchor_tiles_does_not_depend_on_either_budget() {
    // 画像（61×47）のミップマップが入る小さい量
    const MIP: u64 = 64 * 1024;
    let reference = {
        let mut d = anchored_document();
        d.set_filter_cache_budget_bytes(ALL * 4);
        d.set_fill_image_cache_budget_bytes(1 << 30);
        let r = whole(&d);
        assert_eq!(whole(&d), r);
        assert!(
            d.effect_counters().anchor_tiles_composited > 0,
            "Anchor のタイルを合成している"
        );
        r
    };
    for budget in [0, 1, 4096, ALL / 64, ALL / 8, ALL / 2 + 12345] {
        // 画像のミップマップの予算は、画像 1 枚のミップマップが入る小さい量（これより小さいと塗りが使えなくなる）と十分
        for image_budget in [MIP, 1 << 30] {
            let mut d = anchored_document();
            d.set_filter_cache_budget_bytes(budget);
            d.set_fill_image_cache_budget_bytes(image_budget);
            for pass in 0..3 {
                assert_eq!(
                    whole(&d),
                    reference,
                    "予算 {budget}・画像 {image_budget} の {pass} 回目"
                );
                assert!(d.effect_counters().cache_bytes <= budget);
                assert!(d.effect_counters().image_cache_bytes <= image_budget);
            }
        }
    }
}

#[test]
fn blocks_and_anchor_tiles_together_stay_within_one_budget() {
    // 全部が入るだけの予算で両方を埋めて、半分・1/4 に下げる
    let mut d = anchored_document();
    d.set_filter_cache_budget_bytes(ALL * 4);
    let reference = whole(&d);
    let full = d.effect_counters().cache_bytes;
    assert!(full > 0);
    assert!(
        d.effect_counters().anchor_tiles_composited > 0 && evaluated(&d) > 0,
        "ブロックも Anchor のタイルも持つ"
    );
    for budget in [full / 2, full / 4, full / 16] {
        d.set_filter_cache_budget_bytes(budget);
        assert!(
            d.effect_counters().cache_bytes <= budget,
            "下げたらすぐ合計が予算の中（{} > {budget}）",
            d.effect_counters().cache_bytes
        );
        for _ in 0..2 {
            assert_eq!(whole(&d), reference);
            assert!(d.effect_counters().cache_bytes <= budget);
        }
    }
}

#[test]
fn copies_keep_at_most_the_default_effect_cache_budget() {
    const MIB: u64 = 1 << 20;
    let mut d = document();
    d.set_filter_cache_budget_bytes(4096 * MIB);
    assert_eq!(
        d.capture_snapshot().unwrap().filter_cache_budget_bytes(),
        256 * MIB,
        "大きな予算の文書の写しは 256 MiB まで"
    );
    d.set_filter_cache_budget_bytes(64 * MIB);
    assert_eq!(
        d.capture_snapshot().unwrap().filter_cache_budget_bytes(),
        64 * MIB,
        "小さい予算はそのまま引き継ぐ"
    );
}
