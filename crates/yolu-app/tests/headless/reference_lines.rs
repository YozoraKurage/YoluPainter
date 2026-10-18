//! ブラシのはみ出し防止（参照レイヤーの線をはみ出さない）: バケツと同じ参照レイヤーの印の重なりを、ストロークを始めるときに渡すこと。
use crate::common::drive::Drive;
use yolu_app::matpaint::MatAction;
use yolu_app::region::RegionAction;
use yolu_app::sets::Keep;
use yolu_app::state::{Action, AppState};
use yolu_core::glam::DVec2;
use yolu_core::{BrushPixel, Channel, Document, LayerId, Rgba8};

/// 縦の線（x 30〜31）の参照のレイヤーと、その上の描くレイヤー（選んだ状態）。
fn scene() -> (AppState, LayerId, LayerId) {
    let mut s = AppState::new(64, 32);
    let lines = s.doc_mut().add_layer("線").unwrap();
    for y in 0..32 {
        for x in [30, 31] {
            s.doc_mut()
                .set_pixel(lines, x, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    let paint = s.doc_mut().add_layer("塗る").unwrap();
    s.set_selected_layer(Some(paint));
    s.m2_mut().brush.reference_lines.enabled = true;
    s.brush_mut().radius = 6.0;
    (s, lines, paint)
}

fn stroke_across(s: &mut AppState, layer: LayerId) {
    let mut stroke = s.begin_paint_stroke(layer, false).unwrap();
    for x in [10.0, 54.0] {
        stroke
            .add_point(s.doc_mut(), x, 16.5, 1.0, DVec2::ZERO)
            .unwrap();
    }
    s.doc_mut().end_stroke(stroke).unwrap();
}

fn alpha(s: &AppState, layer: LayerId, x: u32) -> u8 {
    s.doc()
        .layer(layer)
        .unwrap()
        .pixel(Channel::Color, x, 16)
        .unwrap()
        .a
}

#[test]
fn marked_reference_layers_keep_the_brush_off_their_lines() {
    let (mut s, lines, paint) = scene();
    s.act(Action::Region(RegionAction::ReferenceLayer(lines)));
    stroke_across(&mut s, paint);
    assert_eq!(alpha(&s, paint, 30), 0, "線の上は塗らない");
    assert_eq!(alpha(&s, paint, 31), 0);
    assert!(alpha(&s, paint, 20) > 0, "線の手前");
    assert!(alpha(&s, paint, 40) > 0, "線の向こう");
    // 1 回の Undo で戻る
    s.doc_mut().undo().unwrap();
    assert_eq!(alpha(&s, paint, 20), 0);
}

#[test]
fn without_marks_the_brush_crosses_the_lines() {
    let (mut s, _, paint) = scene();
    stroke_across(&mut s, paint);
    assert!(
        alpha(&s, paint, 30) > 0,
        "印が無ければ、はみ出し防止は効かない"
    );
}

/// 3D の面のストロークは、ダブの画素の列を `apply_dab` で渡す（始め方は 2D と同じ `begin_paint_stroke`）。
#[test]
fn a_surface_dab_stroke_keeps_off_the_marked_lines_too() {
    let (mut s, lines, paint) = scene();
    s.act(Action::Region(RegionAction::ReferenceLayer(lines)));
    let mut stroke = s.begin_paint_stroke(paint, false).unwrap();
    let pixels: Vec<BrushPixel> = (20..42)
        .map(|x| BrushPixel {
            x,
            y: 16,
            coverage: 1.0,
        })
        .collect();
    stroke
        .apply_dab(s.doc_mut(), &pixels, DVec2::new(25.5, 16.5), 1.0)
        .unwrap();
    s.doc_mut().end_stroke(stroke).unwrap();
    assert_eq!(alpha(&s, paint, 30), 0, "線の上は塗らない");
    assert_eq!(alpha(&s, paint, 31), 0);
    assert!(alpha(&s, paint, 22) > 0, "線の手前");
    assert_eq!(
        alpha(&s, paint, 38),
        0,
        "線をまたいでつながる塊は中心からたどるので、線の向こうへは届かない"
    );
}

fn mask_value(s: &AppState, layer: LayerId, x: u32) -> Rgba8 {
    s.doc()
        .layer(layer)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .pixel(x, 16)
        .unwrap()
}

#[test]
fn a_mask_stroke_keeps_off_the_marked_lines() {
    let (mut s, lines, paint) = scene();
    s.doc_mut().add_layer_mask(paint).unwrap();
    s.set_edit_mask(true);
    s.color_mut().main = [0.0, 0.0, 0.0, 1.0];
    let before = [20, 30, 31, 40].map(|x| mask_value(&s, paint, x));
    s.act(Action::Region(RegionAction::ReferenceLayer(lines)));
    stroke_across(&mut s, paint);
    assert_ne!(mask_value(&s, paint, 20), before[0], "線の手前");
    assert_eq!(mask_value(&s, paint, 30), before[1], "線の上は塗らない");
    assert_eq!(mask_value(&s, paint, 31), before[2]);
    assert_ne!(mask_value(&s, paint, 40), before[3], "線の向こう");
}

#[test]
fn a_material_stroke_keeps_every_channel_off_the_marked_lines() {
    let (mut s, lines, paint) = scene();
    s.mat_apply(MatAction::Enabled(true));
    s.mat_apply(MatAction::Channel(Channel::Roughness, true));
    s.act(Action::Region(RegionAction::ReferenceLayer(lines)));
    stroke_across(&mut s, paint);
    for channel in [Channel::Color, Channel::Roughness] {
        let at = |x: u32| {
            s.doc()
                .layer(paint)
                .unwrap()
                .pixel(channel, x, 16)
                .unwrap()
                .a
        };
        assert_eq!(at(30), 0, "{channel:?} の線の上は塗らない");
        assert_eq!(at(31), 0, "{channel:?}");
        assert!(at(20) > 0, "{channel:?} の線の手前");
        assert!(at(40) > 0, "{channel:?} の線の向こう");
    }
}

#[test]
fn marks_of_another_document_do_not_apply() {
    let (mut s, lines, _) = scene();
    s.act(Action::Region(RegionAction::ReferenceLayer(lines)));
    let previous = s.doc().id();
    // 別の文書（ほかのテクスチャセット）に替える
    let (other, other_lines, other_paint) = {
        let (mut s2, l, p) = scene();
        let doc = std::mem::replace(s2.doc_mut(), Document::new(1, 1).unwrap());
        (doc, l, p)
    };
    s.install_document(other, Keep::reset());
    s.set_selected_layer(Some(other_paint));
    s.m2_mut().brush.reference_lines.enabled = true;
    s.brush_mut().radius = 6.0;
    assert_ne!(s.doc().id(), previous);
    // 前の文書の鍵が付いた印は、今の文書に同じ番号のレイヤーがあっても効かない
    s.region_mut().references.insert((previous, other_lines));
    stroke_across(&mut s, other_paint);
    assert!(alpha(&s, other_paint, 30) > 0, "前の文書の印は効かない");
    // 今の文書の鍵なら効く
    s.doc_mut().undo().unwrap();
    s.act(Action::Region(RegionAction::ReferenceLayer(other_lines)));
    stroke_across(&mut s, other_paint);
    assert_eq!(alpha(&s, other_paint, 30), 0, "今の文書の印は効く");
}

#[test]
fn marks_in_another_layer_set_do_not_apply() {
    let (mut s, _, paint) = scene();
    let set = s.doc_mut().add_layer_set("別").unwrap();
    let other = s.doc_mut().add_layer_in_set("別の線", set, None).unwrap();
    for y in 0..32 {
        for x in [30, 31] {
            s.doc_mut()
                .set_pixel(other, x, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    s.act(Action::Region(RegionAction::ReferenceLayer(other)));
    stroke_across(&mut s, paint);
    assert!(
        alpha(&s, paint, 30) > 0,
        "ほかのレイヤーセットの印は、描くレイヤーのセットの重なりに入らない"
    );
}
