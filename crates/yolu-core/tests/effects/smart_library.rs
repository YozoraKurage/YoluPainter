//! 同梱のスマートマテリアル（`yolu_core::smart_library`）。置ける・大きさに依らない・1 回の Undo・レイヤーの上限で断る。
//! 見た目の良し悪しは試験で決めない（書き出して人が見る）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::smart_library::{self, SIZE};
use yolu_core::{
    smart::SmartPlacement, BlendMode, Channel, CoreError, Document, EffectInputs, Rect, Rgba8,
};

fn placed(doc: &mut Document, index: usize) -> yolu_core::LayerId {
    let e = smart_library::entries()[index];
    let m = e.build(e.name(false), false).unwrap();
    doc.place_smart_material(&m, &SmartPlacement::default())
        .unwrap()
        .layer_id
}
fn whole(doc: &Document, c: Channel) -> Vec<u8> {
    doc.composite_channel(c, Rect::new(0, 0, doc.width(), doc.height()))
        .unwrap()
}
fn distinct(px: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in px.chunks_exact(4) {
        seen.insert([p[0], p[1], p[2], p[3]]);
    }
    seen.len()
}

#[test]
fn every_material_places_as_a_named_group_in_one_undo_step() {
    for (i, e) in smart_library::entries().iter().enumerate() {
        let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
        let base = doc.add_layer("下").unwrap();
        doc.set_pixel(base, 3, 3, Rgba8::new(1, 2, 3, 255)).unwrap();
        let (count, layers) = (doc.undo_count(), doc.layers().len());
        let before = whole(&doc, Channel::Color);
        let id = placed(&mut doc, i);
        assert_eq!(doc.undo_count(), count + 1, "{}", e.id);
        assert!(doc.layer(id).unwrap().is_group(), "{}", e.id);
        assert_eq!(doc.layer(id).unwrap().name(), e.en);
        let material_layers = e.build(e.en, false).unwrap().layers().len();
        assert_eq!(doc.layers().len(), layers + material_layers + 1, "{}", e.id);
        assert_ne!(whole(&doc, Channel::Color), before, "{}", e.id);
        assert!(doc.undo().unwrap());
        assert_eq!(doc.layers().len(), layers);
        assert_eq!(whole(&doc, Channel::Color), before, "{}: Undo", e.id);
        assert!(doc.redo().unwrap());
    }
}

#[test]
fn materials_look_the_same_whatever_the_size_they_are_placed_into() {
    // 画素を持たない（値とマスクの Generator だけ）ので、置く先の大きさで中身が変わらない。UV では模様が画面いっぱいに同じ割合で並ぶ
    // ので、大きさの違う 2 つの文書の合成を同じ大きさへ縮めて比べると、近い（同じ模様の別の解像度）
    for index in [0, 6, 11] {
        let mut small = Document::with_tile_size(64, 64, 64).unwrap();
        let mut big = Document::with_tile_size(256, 256, 64).unwrap();
        placed(&mut small, index);
        placed(&mut big, index);
        let (a, b) = (whole(&small, Channel::Color), whole(&big, Channel::Color));
        assert!(distinct(&a) > 10 && distinct(&b) > 10, "{index}");
        // 256 を 64 へ（4×4 の平均）。形が合えば平均の差は小さい
        let mut total = 0f64;
        for y in 0..64usize {
            for x in 0..64usize {
                for c in 0..3usize {
                    let mut sum = 0u32;
                    for dy in 0..4usize {
                        for dx in 0..4usize {
                            sum += b[((y * 4 + dy) * 256 + x * 4 + dx) * 4 + c] as u32;
                        }
                    }
                    total += (sum as f64 / 16. - a[(y * 64 + x) * 4 + c] as f64).abs();
                }
            }
        }
        let mean = total / (64. * 64. * 3.);
        assert!(mean < 12., "素材 {index}: 平均の差 {mean}");
    }
}

#[test]
fn materials_set_every_pbr_channel_they_claim() {
    for (i, e) in smart_library::entries().iter().enumerate() {
        let mut doc = Document::with_tile_size(64, 64, 64).unwrap();
        placed(&mut doc, i);
        for c in [
            Channel::Color,
            Channel::Roughness,
            Channel::Metallic,
            Channel::Height,
        ] {
            let px = whole(&doc, c);
            // 土台のある素材は全面が埋まる。上に重ねる素材（泥はね）は、模様の所だけ
            let overlay = e.id == "mud-splatter";
            assert_eq!(
                px.chunks_exact(4).all(|p| p[3] == 255),
                !overlay,
                "{} {c:?}",
                e.id
            );
        }
        // 色と粗さには模様がある（土台だけの一様な絵ではない）
        assert!(distinct(&whole(&doc, Channel::Color)) > 8, "{}", e.id);
        assert!(distinct(&whole(&doc, Channel::Roughness)) > 4, "{}", e.id);
    }
}

#[test]
fn placing_beyond_the_layer_limit_is_refused_and_changes_nothing() {
    let mut doc = Document::with_tile_size(16, 16, 16).unwrap();
    while doc.layers().len() < 2046 {
        doc.add_fill_layer("埋め", &[(Channel::Color, Rgba8::new(1, 1, 1, 255))], None)
            .unwrap();
    }
    let e = smart_library::entries()[0];
    let m = e.build(e.en, false).unwrap();
    let (layers, undo) = (doc.layers().len(), doc.undo_count());
    let before = whole(&doc, Channel::Color);
    let err = doc
        .place_smart_material(&m, &SmartPlacement::default())
        .unwrap_err();
    assert!(matches!(err, CoreError::InvalidArgument(_)), "{err:?}");
    assert_eq!(doc.layers().len(), layers);
    assert_eq!(doc.undo_count(), undo);
    assert_eq!(whole(&doc, Channel::Color), before);
}

#[test]
fn placed_materials_never_wait_for_inputs() {
    // 同梱の素材はマップ（焼き）が要らない: 入力が無くても効かない効果にならず、読むだけにならない
    for (i, e) in smart_library::entries().iter().enumerate() {
        let mut doc = Document::with_tile_size(32, 32, 32).unwrap();
        placed(&mut doc, i);
        doc.set_effect_inputs(EffectInputs::new()).unwrap();
        assert!(doc.inactive_effect_list().is_empty(), "{}", e.id);
        assert!(doc.anchor_issues().is_empty(), "{}", e.id);
    }
}

#[test]
fn materials_are_not_pixel_data_and_survive_resizing_the_canvas() {
    for (i, e) in smart_library::entries().iter().enumerate() {
        let m = e.build(e.en, false).unwrap();
        assert_eq!((m.width(), m.height()), (SIZE, SIZE));
        assert_eq!(m.pixel_bytes(), 0, "{}", e.id);
        let mut doc = Document::with_tile_size(48, 40, 16).unwrap();
        let id = placed(&mut doc, i);
        // レイヤーは値と効果だけなので、ブレンドの上書きなど後から直せる
        doc.set_layer_blend_mode(id, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(id, 0.5, false).unwrap();
        assert!(doc.undo().unwrap() && doc.undo().unwrap());
    }
}

/// 塗装の剥げは、場の値をレベルで切って剥げの割合を決める。同梱の「塗装の剥げた金属」のレベルでは、正方形の文書で剥げ（地金の色）が出る
/// 画素の割合が約 3 割（測った値は 0.32。解像度に依らない）。傷のレイヤーを隠して、塗装の赤と地金の灰色のどちらかで数える。
/// 正方形でない文書は、模様のセルの数（長い辺に 1/scale 個、短い辺は同じ大きさになる個数）が変わるので割合も動く
/// （200×96 で測った値は 0.49。保証ではなく、どちらの色も出ることだけを見る）。
#[test]
fn the_chips_of_the_chipped_paint_metal_cover_about_three_tenths_of_a_square_surface() {
    let index = smart_library::entries()
        .iter()
        .position(|e| e.id == "chipped-paint-metal")
        .unwrap();
    let bare_ratio = |w: u32, h: u32| {
        let mut doc = Document::with_tile_size(w, h, 64).unwrap();
        placed(&mut doc, index);
        let scratches = doc
            .layers()
            .iter()
            .find(|l| l.name() == "Scratches")
            .map(|l| l.id())
            .unwrap();
        doc.set_layer_visible(scratches, false).unwrap();
        let px = whole(&doc, Channel::Color);
        // 塗装は赤（170, 36, 32）、地金は灰色（136, 138, 144）。緑で分ける
        let bare = px.chunks_exact(4).filter(|p| p[1] > 87).count();
        bare as f64 / (w * h) as f64
    };
    for size in [64u32, 128, 256, 512] {
        let ratio = bare_ratio(size, size);
        assert!(
            (0.25..=0.4).contains(&ratio),
            "{size} 四方: 剥げの割合 {ratio}"
        );
    }
    let ratio = bare_ratio(200, 96);
    assert!((0.05..=0.95).contains(&ratio), "200x96: 剥げの割合 {ratio}");
}

/// 見た目を確かめるための書き出し（`YOLU_LIBRARY_SHEET=<ディレクトリ>`。512 四方に置いた Color・Roughness・Height の生の RGBA）。
#[test]
fn dump_sheet() {
    let Ok(dir) = std::env::var("YOLU_LIBRARY_SHEET") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for (i, e) in smart_library::entries().iter().enumerate() {
        let mut doc = Document::with_tile_size(512, 512, 64).unwrap();
        placed(&mut doc, i);
        for (c, name) in [
            (Channel::Color, "color"),
            (Channel::Roughness, "roughness"),
            (Channel::Metallic, "metallic"),
            (Channel::Height, "height"),
        ] {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{}.{name}.rgba", e.id)),
                whole(&doc, c),
            )
            .unwrap();
        }
    }
}
