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

#[test]
fn undo_and_redo_of_a_fill_layer_path_keep_the_surface_consistent() {
    // 値の無いチャンネル（色）へのパス: 付けると面ができ、Undo で面ごと戻り、Redo でまた付く
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Roughness, RED)], None)
        .unwrap();
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    for _ in 0..3 {
        doc.undo().unwrap();
        assert!(!doc.layer(fill).unwrap().has_paths());
        assert_eq!(px(&doc, 30, 32).a, 0);
        assert!(
            doc.layer(fill).unwrap().surface(Channel::Color).is_none(),
            "塗りつぶしレイヤーは、パスを外したら画素の面を持たない"
        );
        doc.redo().unwrap();
        assert_eq!(px(&doc, 30, 32), BLUE);
    }
}

#[test]
fn undo_of_a_fill_layer_path_on_a_filled_channel_drops_the_empty_surface() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, RED)], None)
        .unwrap();
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    doc.undo().unwrap();
    assert!(doc.layer(fill).unwrap().surface(Channel::Color).is_none());
    assert_eq!(px(&doc, 30, 32), RED);
    doc.redo().unwrap();
    assert_eq!(px(&doc, 30, 32), BLUE);
    assert!(doc.layer(fill).unwrap().is_channel_enabled(Channel::Color));
}

#[test]
fn a_refused_redo_of_a_fill_layer_path_leaves_no_empty_surface() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, RED)], None)
        .unwrap();
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    doc.undo().unwrap();
    // 画素の予算がもう余っていないので、Redo は断られる
    doc.set_source_budget_bytes(doc.allocated_bytes()).unwrap();
    assert!(doc.redo().is_err());
    assert!(!doc.layer(fill).unwrap().has_paths());
    assert!(
        doc.layer(fill).unwrap().surface(Channel::Color).is_none(),
        "断られた Redo が空の面を残さない"
    );
    assert_eq!(px(&doc, 30, 32), RED);
}

#[test]
fn a_refused_undo_of_removing_a_fill_layer_path_leaves_no_empty_surface() {
    let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, RED)], None)
        .unwrap();
    doc.set_canvas_paths(fill, vec![line(32.0, BLUE)]).unwrap();
    doc.set_paths(fill, Vec::new(), Vec::new()).unwrap();
    assert!(doc.layer(fill).unwrap().surface(Channel::Color).is_none());
    // パスを外した Undo は、消えた画素を戻すので予算が要る。余っていなければ断られ、空の面は残らない
    doc.set_source_budget_bytes(doc.allocated_bytes()).unwrap();
    assert!(doc.undo().is_err());
    assert!(!doc.layer(fill).unwrap().has_paths());
    assert!(doc.layer(fill).unwrap().surface(Channel::Color).is_none());
}

/// クリッピングの下地・クリッピングされる側のどちらの塗りつぶしレイヤーでも、パスの付け外しと Undo・Redo のあとの合成が、
/// 最初から計算し直した文書と同じ（面の作成・破棄でクリッピングの下地のキャッシュが古いまま残らない）。
#[test]
fn clipped_fill_layer_paths_composite_the_same_after_undo_and_redo() {
    // fill_clipped: 塗りつぶしがグループの下地にクリッピングされる / そうでなければ、塗りつぶしが下地でラスターがクリッピングされる
    for fill_clipped in [true, false] {
        let build = |with_path: bool| -> (Document, yolu_core::LayerId) {
            let mut doc = Document::with_tile_size(64, 64, 16).unwrap();
            let fill;
            if fill_clipped {
                let base = doc.add_layer("下地").unwrap();
                doc.set_canvas_paths(base, vec![line(32.0, RED)]).unwrap();
                let group = doc.group_layers(&[base], "グループ").unwrap();
                fill = doc
                    .add_fill_layer("塗り", &[(Channel::Roughness, RED)], Some(group))
                    .unwrap();
                doc.set_layer_clipping(fill, true).unwrap();
            } else {
                fill = doc
                    .add_fill_layer("塗り", &[(Channel::Roughness, RED)], None)
                    .unwrap();
                let top = doc.add_layer("上").unwrap();
                doc.set_canvas_paths(top, vec![line(34.0, BLUE)]).unwrap();
                doc.set_layer_clipping(top, true).unwrap();
            }
            if with_path {
                doc.set_canvas_paths(fill, vec![line(34.0, Rgba8::new(0, 200, 0, 255))])
                    .unwrap();
            }
            (doc, fill)
        };
        let sample = |d: &Document| -> Vec<Rgba8> {
            let mut v = Vec::new();
            for c in [Channel::Color, Channel::Roughness] {
                for y in [10, 31, 33, 35, 50] {
                    v.push(d.composite_pixel(c, 30, y).unwrap());
                }
            }
            v
        };
        let (without, _) = build(false);
        let expect_without = sample(&without);
        let (with, _) = build(true);
        let expect_with = sample(&with);
        assert_ne!(expect_with, expect_without, "{fill_clipped}");
        // 付ける前の合成を計算させて（キャッシュを温めて）から、付け外しを繰り返す
        let (mut doc, fill) = build(false);
        assert_eq!(sample(&doc), expect_without);
        for _ in 0..2 {
            doc.set_canvas_paths(fill, vec![line(34.0, Rgba8::new(0, 200, 0, 255))])
                .unwrap();
            assert_eq!(sample(&doc), expect_with, "{fill_clipped}: 付けた");
            doc.undo().unwrap();
            assert_eq!(sample(&doc), expect_without, "{fill_clipped}: Undo");
            doc.redo().unwrap();
            assert_eq!(sample(&doc), expect_with, "{fill_clipped}: Redo");
            doc.set_paths(fill, Vec::new(), Vec::new()).unwrap();
            assert_eq!(sample(&doc), expect_without, "{fill_clipped}: 外した");
            doc.undo().unwrap();
            assert_eq!(sample(&doc), expect_with, "{fill_clipped}: 外しの Undo");
            doc.undo().unwrap();
            assert_eq!(sample(&doc), expect_without);
        }
    }
}
