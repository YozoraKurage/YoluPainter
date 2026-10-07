//! 塗りつぶしレイヤーのパス: 合成は塗りつぶし → そのレイヤーの効果のスタック → パスの画素の順（パスは効果の上に重なり、効果を受けない）。
//! パスの付け外しは 1 回の Undo。塗りつぶしの値の無いチャンネルのパスも出る。画素にする（ラスタライズ）は断る。
use yolu_core::effects::{EffectSettings, FilterSpec};
use yolu_core::paths::{CanvasPath, CanvasPoint, LayerPathEntry, PathBrush};
use yolu_core::{BrushSettings, Channel, Document, FilterTarget, LayerPath, Rgba8};

const RED: Rgba8 = Rgba8::new(200, 30, 20, 255);
const BLUE: Rgba8 = Rgba8::new(10, 40, 230, 255);

fn line(y: f64, color: Rgba8) -> LayerPathEntry {
    LayerPathEntry::new(LayerPath::Canvas(CanvasPath {
        id: (y * 10.0) as u128 + 1,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 3.0,
            hardness: 1.0,
            color,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(4.0, y, 1.0).unwrap(),
            CanvasPoint::new(60.0, y, 1.0).unwrap(),
        ],
        material: None,
        style: Default::default(),
    }))
}

fn px(doc: &Document, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(Channel::Color, x, y).unwrap()
}

#[test]
fn a_fill_layer_path_lies_over_the_fill_and_its_effects() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, RED)], None)
        .unwrap();
    // 効果: 色の反転（塗りつぶしは反転し、パスは反転しない）
    doc.add_filter(
        fill,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::Filter(yolu_core::filter::Settings::Invert))
            .channels(&[Channel::Color]),
    )
    .unwrap();
    let inverted = px(&doc, 30, 10);
    assert_eq!((inverted.r, inverted.g, inverted.b), (55, 225, 235));
    let undo = doc.undo_count();
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    assert_eq!(doc.undo_count(), undo + 1);
    assert_eq!(px(&doc, 30, 32), BLUE, "パスは効果の上");
    assert_eq!(
        px(&doc, 30, 10),
        inverted,
        "パスの外は塗りつぶしと効果のまま"
    );
    // パスを動かす（描き直す）と、前の画素は残らない
    doc.set_canvas_paths(fill, vec![line(48.0, BLUE)]).unwrap();
    assert_eq!(px(&doc, 30, 32), inverted);
    assert_eq!(px(&doc, 30, 48), BLUE);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(px(&doc, 30, 32), inverted, "Undo でパスが外れる");
    assert!(!doc.layer(fill).unwrap().has_paths());
}

#[test]
fn a_fill_layer_path_shows_on_a_channel_without_a_fill_value_and_is_not_rasterized() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    // 色の値の無い塗りつぶしレイヤー（Roughness だけ）
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Roughness, RED)], None)
        .unwrap();
    assert_eq!(px(&doc, 30, 32).a, 0);
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    assert_eq!(px(&doc, 30, 32), BLUE);
    assert_eq!(px(&doc, 30, 10).a, 0);
    assert!(
        doc.rasterize(fill).is_err(),
        "塗りつぶしレイヤーのパスは画素にしない"
    );
    assert!(doc.layer(fill).unwrap().has_paths());
}
