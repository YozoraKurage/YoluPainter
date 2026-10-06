//! 文書の中のノイズ・グランジの段: 位置のマップで評価し、使えないときは入力のまま通さず UV に落として理由を言い、
//! ブロックの大きさで結果が変わらず、1 回の Undo で戻り、設定を直すとその分だけ評価し直す。
#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::attach_support;
use attach_support::*;
use yolu_core::generator::{
    Blend, GrungePreset, Inactive, Kind, MapKind, ProceduralSpace, Settings,
};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterId, FilterSpec, FilterTarget,
    InactiveReason, LayerId,
};

fn grunge(preset: GrungePreset) -> Settings {
    let mut g = Settings::grunge(preset);
    g.blend = Blend::Replace;
    g
}
fn add(doc: &mut Document, layer: LayerId, target: FilterTarget, g: Settings) -> FilterId {
    let mut spec = FilterSpec::new(EffectSettings::generator(g));
    if target == FilterTarget::Content {
        spec = spec.channels(&[Channel::Color]);
    }
    doc.add_filter(layer, target, spec).unwrap()
}

#[test]
fn stages_change_the_output_and_undo_in_one_step() {
    for kind in [Settings::new(Kind::Noise), grunge(GrungePreset::Cracks)] {
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            let (mut doc, l) = world();
            let top = l[2];
            if target == FilterTarget::Mask {
                doc.add_layer_mask(top).unwrap();
            }
            let before = whole(&doc, Channel::Color);
            let count = doc.undo_count();
            let mut g = kind.clone();
            g.blend = Blend::Replace;
            let id = add(&mut doc, top, target, g);
            assert_eq!(doc.undo_count(), count + 1);
            let after = whole(&doc, Channel::Color);
            assert_ne!(before, after, "{:?} {target:?}", kind.kind);
            // 設定を直す（スライダーのドラッグ）は続けて 1 回の Undo
            let count = doc.undo_count();
            for seed in 1..4 {
                let mut g = kind.clone();
                g.blend = Blend::Replace;
                g.procedural.seed = seed;
                doc.set_filter_settings(top, id, EffectSettings::generator(g), true)
                    .unwrap();
            }
            assert_eq!(doc.undo_count(), count + 1);
            assert_ne!(whole(&doc, Channel::Color), after);
            assert!(doc.undo().unwrap());
            assert_eq!(whole(&doc, Channel::Color), after);
            assert!(doc.undo().unwrap());
            assert_eq!(whole(&doc, Channel::Color), before);
            assert!(doc.redo().unwrap() && doc.redo().unwrap());
            assert_ne!(whole(&doc, Channel::Color), after);
        }
    }
}

#[test]
fn the_block_size_does_not_change_the_composite() {
    for g in [
        Settings::new(Kind::Noise),
        grunge(GrungePreset::Rust),
        grunge(GrungePreset::Weave),
        grunge(GrungePreset::Scratches),
    ] {
        let (mut doc, l) = world();
        let mut g = g;
        g.blend = Blend::Replace;
        add(&mut doc, l[2], FilterTarget::Content, g.clone());
        doc.add_layer_mask(l[0]).unwrap();
        add(&mut doc, l[0], FilterTarget::Mask, g);
        doc.set_filter_block_pixels(16).unwrap();
        let reference = whole(&doc, Channel::Color);
        for block in [8, 24, 40, 256] {
            doc.set_filter_block_pixels(block).unwrap();
            assert_eq!(whole(&doc, Channel::Color), reference, "ブロック {block}");
        }
    }
}

#[test]
fn a_missing_position_map_is_a_notice_not_an_inactive_effect() {
    let (mut doc, l) = world();
    let top = l[2];
    let id = add(
        &mut doc,
        top,
        FilterTarget::Content,
        Settings::new(Kind::Noise),
    );
    doc.add_layer_mask(l[0]).unwrap();
    let mask_id = add(
        &mut doc,
        l[0],
        FilterTarget::Mask,
        grunge(GrungePreset::Dust),
    );
    let uv_id = {
        let mut g = Settings::new(Kind::Noise);
        g.procedural.space = ProceduralSpace::Uv;
        add(&mut doc, l[1], FilterTarget::Content, g)
    };
    // マップが使える間は理由が無い
    assert_eq!(doc.generator_fallback(top, id).unwrap(), None);
    assert_eq!(doc.generator_inactive(top, id).unwrap(), None);
    assert!(doc.fallback_effect_list().is_empty());
    let with_maps = whole(&doc, Channel::Color);
    // 外すと、入力のまま通さず UV に落ちる（理由が出て、編集は止まらない）
    doc.set_effect_inputs(EffectInputs::new()).unwrap();
    assert_eq!(
        doc.generator_fallback(top, id).unwrap(),
        Some(Inactive::MissingMap(MapKind::Position))
    );
    assert_eq!(doc.generator_inactive(top, id).unwrap(), None);
    assert_eq!(doc.generator_fallback(l[1], uv_id).unwrap(), None);
    assert!(
        doc.inactive_effects().is_empty(),
        "{:?}",
        doc.inactive_effects()
    );
    let list = doc.fallback_effect_list();
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(list
        .iter()
        .any(|f| f.layer == top && !f.mask && f.kind == Kind::Noise));
    assert!(list
        .iter()
        .any(|f| f.layer == l[0] && f.mask && f.kind == Kind::Grunge));
    assert_eq!(
        list[0].reason,
        InactiveReason::Generator(Inactive::MissingMap(MapKind::Position))
    );
    let text = list[0].to_string();
    assert!(text.contains("UV") && text.contains("Position"), "{text}");
    let fallen = whole(&doc, Channel::Color);
    assert_ne!(fallen, with_maps);
    // UV に落ちた値は、最初から UV を選んだ設定と同じ（段の入力は同じなので、合成は同じ）
    let mut explicit = Settings::new(Kind::Noise);
    explicit.procedural.space = ProceduralSpace::Uv;
    doc.set_filter_settings(top, id, EffectSettings::generator(explicit), false)
        .unwrap();
    assert_eq!(whole(&doc, Channel::Color), fallen);
    // マップを渡し直すと位置の評価に戻る
    doc.set_effect_inputs(inputs(0)).unwrap();
    assert!(doc.fallback_effect_list().iter().all(|f| f.layer != top));
    // 無効な段は一覧に入れない
    doc.set_effect_inputs(EffectInputs::new()).unwrap();
    doc.set_filter_enabled(l[0], mask_id, false).unwrap();
    assert!(doc.fallback_effect_list().is_empty());
}

#[test]
fn documents_with_the_same_inputs_render_the_same_bytes() {
    // 同じ設定・位置のマップなら、別の文書でも同じバイト（保存・読み直しで変わらない土台）
    let (mut a, la) = world();
    let (mut b, lb) = world();
    let g = grunge(GrungePreset::Pebbles);
    add(&mut a, la[3], FilterTarget::Content, g.clone());
    add(&mut b, lb[3], FilterTarget::Content, g);
    assert_eq!(whole(&a, Channel::Color), whole(&b, Channel::Color));
    let px = a
        .composite_channel(
            Channel::Color,
            yolu_core::Rect::new(0, 0, a.width(), a.height()),
        )
        .unwrap();
    assert!(px.chunks_exact(4).any(|p| p != &px[..4]));
}
