//! チャンネルの面の作成・有効の印・取り消しの後始末（`ensure_surface` と、履歴の段が持つ「作った」の印）。
//! 面を作った操作を取り消すと、作る前と同じ（面の有無・有効の印・合成）に戻り、やり直しで面が戻ること。
//!
//! 保存（.ylp）が読むのは、レイヤーの面を持つチャンネルとそのタイルと有効の印なので、ここでは「面を持つチャンネル・有効なチャンネル・
//! タイルの数・合成」の組が操作の前と同じかどうかで保存のバイトの代わりにする。

use super::*;
use crate::glam::DVec2;
use crate::material::ChannelPaint;

/// 面と有効の印と合成の見え方（保存が読む物）。
#[derive(Debug, PartialEq)]
struct Look {
    enabled: Vec<Channel>,
    surfaces: Vec<(Channel, usize)>,
    /// 合成のバイトの FNV-1a（失敗の表示を短くする）。
    composite: u64,
}

fn look(d: &Document, id: LayerId) -> Look {
    let layer = d.layer(id).unwrap();
    Look {
        enabled: layer.enabled_channels(),
        surfaces: layer
            .surface_channels()
            .into_iter()
            .map(|c| (c, layer.surface(c).unwrap().tile_count()))
            .collect(),
        composite: d
            .composite(d.bounds())
            .unwrap()
            .iter()
            .fold(0xcbf29ce484222325, |h, &b| {
                (h ^ b as u64).wrapping_mul(0x100000001b3)
            }),
    }
}

/// 色のチャンネルが有効なのに面が無いラスターレイヤー（面を作る入口の、有効のまま面を作る形）。
fn enabled_without_surface() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(48, 32, 16).unwrap();
    let id = d.add_layer("bare").unwrap();
    let i = d.index_of(id).unwrap();
    d.layers[i].put_surface(Channel::Color, None);
    assert!(d.layer(id).unwrap().is_channel_enabled(Channel::Color));
    assert!(d.layer(id).unwrap().surface(Channel::Color).is_none());
    d.clear_history().unwrap();
    (d, id)
}

/// 色は面あり、粗さは無効で面も無いラスターレイヤー（無効のチャンネルを有効にして面を作る形）。
fn disabled_roughness() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(48, 32, 16).unwrap();
    let id = d.add_layer("layer").unwrap();
    let layer = d.layer(id).unwrap();
    assert!(!layer.is_channel_enabled(Channel::Roughness));
    assert!(layer.surface(Channel::Roughness).is_none());
    d.clear_history().unwrap();
    (d, id)
}

fn dab_brush() -> BrushSettings {
    BrushSettings {
        radius: 4.0,
        hardness: 1.0,
        ..BrushSettings::default()
    }
}

fn material() -> [ChannelPaint; 2] {
    [
        ChannelPaint::new(Channel::Color, Rgba8::new(200, 30, 20, 255)),
        ChannelPaint::new(Channel::Roughness, Rgba8::new(90, 90, 90, 255)),
    ]
}

/// 操作して、取り消して（作る前と同じ）、やり直して（操作の直後と同じ）、もう一度取り消す。
fn undo_redo_roundtrip(d: &mut Document, id: LayerId, before: &Look) -> Look {
    let after = look(d, id);
    assert_ne!(&after, before, "操作で何も変わっていない");
    assert!(d.undo().unwrap());
    assert_eq!(&look(d, id), before, "取り消しで作る前に戻らない");
    assert!(d.redo().unwrap());
    assert_eq!(look(d, id), after, "やり直しで操作の直後に戻らない");
    assert!(d.undo().unwrap());
    assert_eq!(
        &look(d, id),
        before,
        "やり直しの後の取り消しで作る前に戻らない"
    );
    after
}

fn paint_stroke(d: &mut Document, id: LayerId) -> Stroke {
    let mut s = d
        .begin_material_stroke(id, &material(), &dab_brush())
        .unwrap();
    s.add_point(d, 20.0, 12.0, 1.0, DVec2::ZERO).unwrap();
    s
}

// ---- 無効のチャンネルを有効にして面を作る（今も取り消しで外れる形） ----

#[test]
fn a_material_fill_on_a_disabled_channel_undoes_to_before_and_redoes() {
    let (mut d, id) = disabled_roughness();
    let before = look(&d, id);
    assert!(d.fill_material(id, &material(), 1.0, None, false).unwrap());
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_material_stroke_on_a_disabled_channel_undoes_to_before_and_redoes() {
    let (mut d, id) = disabled_roughness();
    let before = look(&d, id);
    let s = paint_stroke(&mut d, id);
    d.end_stroke(s).unwrap();
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_cancelled_material_stroke_on_a_disabled_channel_leaves_nothing() {
    let (mut d, id) = disabled_roughness();
    let before = look(&d, id);
    let s = paint_stroke(&mut d, id);
    d.cancel_stroke(s);
    assert_eq!(look(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn enabling_a_channel_undoes_to_before_and_redoes() {
    let (mut d, id) = disabled_roughness();
    let before = look(&d, id);
    d.set_channel_enabled(id, Channel::Roughness, true).unwrap();
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_material_fill_that_changes_nothing_leaves_nothing_behind() {
    let (mut d, id) = disabled_roughness();
    let before = look(&d, id);
    // 消しゴムで空のレイヤーを消しても何も変わらない: 有効にした粗さも外れる
    assert!(!d.fill_material(id, &material(), 1.0, None, true).unwrap());
    assert_eq!(look(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}

// ---- 有効なのに面が無いチャンネルへ面を作る（段が「作った」を持つ形） ----

#[test]
fn a_fill_that_made_the_surface_undoes_to_before_and_redoes() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let color = Rgba8::new(10, 200, 30, 255);
    assert!(d.fill(id, Channel::Color, color, 1.0, None, false).unwrap());
    let after = undo_redo_roundtrip(&mut d, id, &before);
    assert!(after
        .surfaces
        .iter()
        .any(|&(c, n)| c == Channel::Color && n > 0));
}

#[test]
fn replacing_pixels_that_made_the_surface_undoes_to_before_and_redoes() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let rgba: Vec<u8> = (0..48 * 32).flat_map(|_| [90u8, 40, 200, 255]).collect();
    assert!(d.replace_pixels(id, Channel::Color, &rgba, false).unwrap());
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_material_fill_that_made_the_surface_undoes_to_before_and_redoes() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    assert!(d.fill_material(id, &material(), 1.0, None, false).unwrap());
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_material_stroke_that_made_the_surface_undoes_to_before_and_redoes() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let s = paint_stroke(&mut d, id);
    d.end_stroke(s).unwrap();
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_cancelled_material_stroke_that_made_the_surface_leaves_nothing() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let s = paint_stroke(&mut d, id);
    d.cancel_stroke(s);
    assert_eq!(look(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn a_fill_and_a_material_fill_that_change_nothing_leave_nothing_behind() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let color = Rgba8::new(10, 200, 30, 255);
    assert!(!d.fill(id, Channel::Color, color, 1.0, None, true).unwrap());
    assert!(!d.fill_material(id, &material(), 1.0, None, true).unwrap());
    assert_eq!(look(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}

fn triangle() -> [crate::material_triangles::PixelTriangle; 1] {
    [[
        DVec2::new(0.0, 0.0),
        DVec2::new(30.0, 0.0),
        DVec2::new(0.0, 20.0),
    ]]
}

#[test]
fn a_triangle_fill_that_made_the_surface_undoes_to_before_and_redoes() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let mut f = d
        .begin_material_triangle_fill(id, &material(), 1.0, false)
        .unwrap();
    f.add(&mut d, &triangle()).unwrap();
    f.commit(&mut d).unwrap();
    undo_redo_roundtrip(&mut d, id, &before);
}

#[test]
fn a_cancelled_triangle_fill_that_made_the_surface_leaves_nothing() {
    let (mut d, id) = enabled_without_surface();
    let before = look(&d, id);
    let mut f = d
        .begin_material_triangle_fill(id, &material(), 1.0, false)
        .unwrap();
    f.add(&mut d, &triangle()).unwrap();
    f.cancel(&mut d);
    assert_eq!(look(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}
